use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _},
    process::{Child, Command},
};
use tracing::debug;

use crate::state::AudioFormat;

const MAX_STDERR_BYTES: usize = 8 * 1024;
const MAX_PROBE_STDOUT_BYTES: usize = 4 * 1024;
const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

static PART_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MuxRequest {
    pub video: Option<PathBuf>,
    pub audio: Option<PathBuf>,
    pub output: PathBuf,
    pub audio_format: Option<AudioFormat>,
    pub source_audio_codec: Option<String>,
    pub metadata_comment: Option<String>,
}

pub trait Muxer: Send + Sync {
    fn mux<'a>(&'a self, req: MuxRequest) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    fn probe<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

    fn read_comment_tag<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>>;
}

/// RAII guard that removes a staging `.part` file unless explicitly disarmed after atomic rename.
struct PartFileGuard {
    path: PathBuf,
    armed: bool,
}

impl PartFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PartFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// RAII guard that kills and best-effort reaps a spawned child process on error, cancellation, or
/// panic unwind. Always declare this AFTER `PartFileGuard` so the child exits (releasing file
/// handles on Windows) before `PartFileGuard` deletes the `.part` file.
struct ChildKillGuard {
    child: Option<Child>,
}

impl ChildKillGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let child = self
            .child
            .as_mut()
            .expect("ChildKillGuard::wait called more than once");
        let status = child.wait().await?;
        self.child = None;
        Ok(status)
    }
}

impl Drop for ChildKillGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            // Best-effort synchronous reap so file handles are closed before PartFileGuard runs.
            for _ in 0..20 {
                match child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct FfmpegMuxer {
    ffmpeg_path: PathBuf,
    probe_timeout: Duration,
}

impl Default for FfmpegMuxer {
    fn default() -> Self {
        Self::new(None)
    }
}

impl FfmpegMuxer {
    pub fn new(ffmpeg_path: Option<PathBuf>) -> Self {
        Self {
            ffmpeg_path: ffmpeg_path.unwrap_or_else(|| PathBuf::from("ffmpeg")),
            probe_timeout: DEFAULT_PROBE_TIMEOUT,
        }
    }

    pub fn with_probe_timeout(mut self, timeout: Duration) -> Self {
        self.probe_timeout = timeout;
        self
    }

    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg_path
    }

    fn ffprobe_path(&self) -> PathBuf {
        let exe_name = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        if let Some(parent) = self.ffmpeg_path.parent()
            && !parent.as_os_str().is_empty()
        {
            let sibling = parent.join(exe_name);
            if sibling.exists() {
                return sibling;
            }
        }
        PathBuf::from(exe_name)
    }

    async fn run_mux(&self, req: MuxRequest) -> Result<()> {
        if req.video.is_none() && req.audio.is_none() {
            return Err(anyhow!(
                "MuxRequest must contain at least one video or audio input"
            ));
        }

        if let Some(parent) = req.output.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!(
                    "Failed to create output directory '{}'",
                    parent.to_string_lossy()
                )
            })?;
        }

        let part_path = make_part_path(&req.output);
        // Declared first so it drops AFTER `child_guard` kills/reaps ffmpeg.
        let mut part_guard = PartFileGuard::new(part_path.clone());

        let mut cmd = Command::new(&self.ffmpeg_path);
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostats", "-y"]);
        if let Some(video) = &req.video {
            cmd.arg("-i").arg(video);
        }
        if let Some(audio) = &req.audio {
            cmd.arg("-i").arg(audio);
        }

        if let Some(audio_fmt) = req.audio_format {
            cmd.arg("-vn");
            match audio_fmt {
                AudioFormat::Copy => {
                    cmd.args(["-c:a", "copy"]);
                    if uses_faststart(&req.output) {
                        cmd.args(["-movflags", "+faststart"]);
                    }
                    if let Some(fmt) = ffmpeg_format_for_output(&req.output) {
                        cmd.args(["-f", fmt]);
                    }
                }
                AudioFormat::M4a => {
                    if is_aac_codec(req.source_audio_codec.as_deref()) {
                        cmd.args(["-c:a", "copy"]);
                    } else {
                        cmd.args(["-c:a", "aac", "-b:a", "192k"]);
                    }
                    cmd.args(["-movflags", "+faststart", "-f", "mp4"]);
                }
                AudioFormat::Mp3 => {
                    cmd.args(["-c:a", "libmp3lame", "-q:a", "2", "-f", "mp3"]);
                }
                AudioFormat::Flac => {
                    if is_flac_codec(req.source_audio_codec.as_deref()) {
                        cmd.args(["-c:a", "copy"]);
                    } else {
                        cmd.args(["-c:a", "flac"]);
                    }
                    cmd.args(["-f", "flac"]);
                }
                AudioFormat::Opus => {
                    cmd.args(["-c:a", "libopus", "-b:a", "160k", "-f", "opus"]);
                }
            }
        } else {
            cmd.args(["-c", "copy"]);
            if uses_faststart(&req.output) {
                cmd.args(["-movflags", "+faststart"]);
            }
            if let Some(fmt) = ffmpeg_format_for_output(&req.output) {
                cmd.args(["-f", fmt]);
            }
        }

        if let Some(comment) = &req.metadata_comment
            && !comment.is_empty()
        {
            cmd.args(["-metadata", &format!("comment={comment}")]);
        }

        cmd.arg(&part_path);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!(
                    "ffmpeg not found on PATH (tried '{}'): install ffmpeg or set `ffmpeg_path`",
                    self.ffmpeg_path.display()
                )
            } else {
                anyhow!(
                    "Failed to spawn ffmpeg '{}': {e}",
                    self.ffmpeg_path.display()
                )
            }
        })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let mut child_guard = ChildKillGuard::new(child);

        let stdout_fut = async move {
            match stdout {
                Some(r) => drain_bounded(r, 0).await,
                None => Ok(Vec::new()),
            }
        };
        let stderr_fut = async move {
            match stderr {
                Some(r) => drain_bounded(r, MAX_STDERR_BYTES).await,
                None => Ok(Vec::new()),
            }
        };

        let (status, _, stderr_bytes) =
            tokio::try_join!(child_guard.wait(), stdout_fut, stderr_fut)
                .context("Error while running ffmpeg subprocess")?;

        let stderr_text = String::from_utf8_lossy(&stderr_bytes);
        if !stderr_text.trim().is_empty() {
            debug!(
                output = %req.output.display(),
                stderr = %stderr_text.trim(),
                "ffmpeg stderr output"
            );
        }

        if !status.success() {
            let code_desc = status
                .code()
                .map(|c| format!("code {c}"))
                .unwrap_or_else(|| "signal".to_string());
            let excerpt = stderr_excerpt(&stderr_text);
            return Err(anyhow!(
                "ffmpeg mux failed ({code_desc}) for '{}': {excerpt}",
                req.output.display()
            ));
        }

        tokio::fs::rename(&part_path, &req.output)
            .await
            .with_context(|| {
                format!(
                    "Failed to atomically move muxed output '{}' -> '{}'",
                    part_path.display(),
                    req.output.display()
                )
            })?;
        part_guard.disarm();
        Ok(())
    }

    async fn run_probe(&self) -> Result<String> {
        let mut cmd = Command::new(&self.ffmpeg_path);
        cmd.args(["-hide_banner", "-version"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!(
                    "ffmpeg not found on PATH (tried '{}'): install ffmpeg or set `ffmpeg_path`",
                    self.ffmpeg_path.display()
                )
            } else {
                anyhow!(
                    "Failed to spawn ffmpeg '{}': {e}",
                    self.ffmpeg_path.display()
                )
            }
        })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let mut child_guard = ChildKillGuard::new(child);

        let probe_fut = async {
            let stdout_fut = async move {
                match stdout {
                    Some(r) => drain_bounded(r, MAX_PROBE_STDOUT_BYTES).await,
                    None => Ok(Vec::new()),
                }
            };
            let stderr_fut = async move {
                match stderr {
                    Some(r) => drain_bounded(r, MAX_STDERR_BYTES).await,
                    None => Ok(Vec::new()),
                }
            };
            tokio::try_join!(child_guard.wait(), stdout_fut, stderr_fut)
        };

        let (status, stdout_bytes, stderr_bytes) =
            tokio::time::timeout(self.probe_timeout, probe_fut)
                .await
                .map_err(|_| {
                    anyhow!(
                        "Timed out waiting for '{}' -version after {:?}",
                        self.ffmpeg_path.display(),
                        self.probe_timeout
                    )
                })?
                .context("Failed while probing ffmpeg version")?;

        let stdout_text = String::from_utf8_lossy(&stdout_bytes);
        let stderr_text = String::from_utf8_lossy(&stderr_bytes);
        if !status.success() {
            return Err(anyhow!(
                "ffmpeg -version exited with {}: {}",
                status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string()),
                stderr_excerpt(&stderr_text)
            ));
        }

        let first_line = stdout_text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("ffmpeg (unknown version)");
        Ok(first_line.to_string())
    }

    async fn run_read_comment_tag(&self, path: &Path) -> Result<Option<String>> {
        let ffprobe = self.ffprobe_path();
        run_read_comment_tag_with_bin(&ffprobe, path, self.probe_timeout).await
    }
}

async fn run_read_comment_tag_with_bin(
    ffprobe_bin: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<Option<String>> {
    let mut cmd = Command::new(ffprobe_bin);
    cmd.args([
        "-v",
        "error",
        "-show_entries",
        "format_tags=comment:stream_tags=comment",
        "-of",
        "default=nw=1:nk=1",
    ])
    .arg(path)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            debug!(
                ffprobe = %ffprobe_bin.display(),
                path = %path.display(),
                error = %e,
                "ffprobe unavailable for comment tag inspection; skipping"
            );
            return Ok(None);
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut child_guard = ChildKillGuard::new(child);

    let probe_fut = async {
        let stdout_fut = async move {
            match stdout {
                Some(r) => drain_bounded(r, MAX_PROBE_STDOUT_BYTES).await,
                None => Ok(Vec::new()),
            }
        };
        let stderr_fut = async move {
            match stderr {
                Some(r) => drain_bounded(r, MAX_STDERR_BYTES).await,
                None => Ok(Vec::new()),
            }
        };
        tokio::try_join!(child_guard.wait(), stdout_fut, stderr_fut)
    };

    let Ok(Ok((status, stdout_bytes, _stderr_bytes))) =
        tokio::time::timeout(timeout, probe_fut).await
    else {
        debug!(
            path = %path.display(),
            "ffprobe comment tag read timed out or failed; skipping"
        );
        return Ok(None);
    };

    if !status.success() {
        return Ok(None);
    }

    let stdout_text = String::from_utf8_lossy(&stdout_bytes);
    let mut first_non_empty: Option<&str> = None;
    for line in stdout_text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.contains("fav:") {
            return Ok(Some(line.to_string()));
        }
        if first_non_empty.is_none() {
            first_non_empty = Some(line);
        }
    }
    Ok(first_non_empty.map(str::to_string))
}

impl Muxer for FfmpegMuxer {
    fn mux<'a>(&'a self, req: MuxRequest) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(self.run_mux(req))
    }

    fn probe<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(self.run_probe())
    }

    fn read_comment_tag<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>> {
        Box::pin(self.run_read_comment_tag(path))
    }
}

fn is_aac_codec(codec: Option<&str>) -> bool {
    codec.is_some_and(|c| {
        let lc = c.to_ascii_lowercase();
        lc.contains("mp4a") || lc.contains("aac")
    })
}

fn is_flac_codec(codec: Option<&str>) -> bool {
    codec.is_some_and(|c| c.to_ascii_lowercase().contains("flac"))
}

fn make_part_path(output: &Path) -> PathBuf {
    let parent = match output.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let pid = std::process::id();
    let seq = PART_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(".fav-mux-{pid}-{seq}.part"))
}

fn uses_faststart(output: &Path) -> bool {
    output
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "mp4" | "m4a"))
}

fn ffmpeg_format_for_output(output: &Path) -> Option<&'static str> {
    let ext = output.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "m4a" | "mov" => Some("mp4"),
        "mkv" => Some("matroska"),
        "webm" => Some("webm"),
        "flac" => Some("flac"),
        "mp3" => Some("mp3"),
        "ogg" => Some("ogg"),
        "opus" => Some("opus"),
        "aac" => Some("adts"),
        _ => None,
    }
}

async fn drain_bounded<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> std::io::Result<Vec<u8>> {
    let mut collected = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        if collected.len() < limit {
            let remaining = limit - collected.len();
            let take = n.min(remaining);
            collected.extend_from_slice(&chunk[..take]);
        }
    }
    Ok(collected)
}

fn stderr_excerpt(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return "(no stderr output)".to_string();
    }
    const MAX_EXCERPT: usize = 512;
    if trimmed.len() <= MAX_EXCERPT {
        trimmed.to_string()
    } else {
        let mut end = MAX_EXCERPT;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &trimmed[..end])
    }
}

pub mod test_support {
    use std::{
        collections::HashMap,
        future::Future,
        path::{Path, PathBuf},
        pin::Pin,
        sync::{Arc, Mutex},
    };

    use anyhow::{Result, anyhow};

    use super::{MuxRequest, Muxer};

    const MOCK_STUB_PREFIX: &str = "mock-mp4\n";

    #[derive(Clone)]
    pub struct MockMuxer {
        requests: Arc<Mutex<Vec<MuxRequest>>>,
        fail_message: Option<String>,
        probe_result: Arc<Mutex<Result<String, String>>>,
        comment_tags: Arc<Mutex<HashMap<PathBuf, String>>>,
        write_stub_output: bool,
    }

    impl Default for MockMuxer {
        fn default() -> Self {
            Self::succeeding()
        }
    }

    impl MockMuxer {
        pub fn succeeding() -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                fail_message: None,
                probe_result: Arc::new(Mutex::new(Ok("ffmpeg version mock-1.0".to_string()))),
                comment_tags: Arc::new(Mutex::new(HashMap::new())),
                write_stub_output: true,
            }
        }

        pub fn failing(msg: impl Into<String>) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                fail_message: Some(msg.into()),
                probe_result: Arc::new(Mutex::new(Ok("ffmpeg version mock-1.0".to_string()))),
                comment_tags: Arc::new(Mutex::new(HashMap::new())),
                write_stub_output: false,
            }
        }

        pub fn with_probe_error(self, err: impl Into<String>) -> Self {
            *self.probe_result.lock().unwrap() = Err(err.into());
            self
        }

        pub fn with_comment_tag(self, path: impl Into<PathBuf>, tag: impl Into<String>) -> Self {
            self.set_comment_tag(path, tag);
            self
        }

        pub fn set_comment_tag(&self, path: impl Into<PathBuf>, tag: impl Into<String>) {
            self.comment_tags
                .lock()
                .unwrap()
                .insert(path.into(), tag.into());
        }

        pub fn requests(&self) -> Vec<MuxRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Muxer for MockMuxer {
        fn mux<'a>(
            &'a self,
            req: MuxRequest,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            let requests = self.requests.clone();
            let fail = self.fail_message.clone();
            let write_stub = self.write_stub_output;
            let comment_tags = self.comment_tags.clone();
            Box::pin(async move {
                requests.lock().unwrap().push(req.clone());
                if let Some(msg) = fail {
                    return Err(anyhow!(msg));
                }
                if let Some(ref comment) = req.metadata_comment {
                    comment_tags
                        .lock()
                        .unwrap()
                        .insert(req.output.clone(), comment.clone());
                }
                if write_stub {
                    if let Some(parent) = req.output.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    let payload = match &req.metadata_comment {
                        Some(comment) => format!("{MOCK_STUB_PREFIX}{comment}\n").into_bytes(),
                        None => b"mock-mp4".to_vec(),
                    };
                    tokio::fs::write(&req.output, payload).await?;
                }
                Ok(())
            })
        }

        fn probe<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
            let probe_result = self.probe_result.clone();
            Box::pin(async move {
                match &*probe_result.lock().unwrap() {
                    Ok(v) => Ok(v.clone()),
                    Err(e) => Err(anyhow!("{e}")),
                }
            })
        }

        fn read_comment_tag<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>> {
            let comment_tags = self.comment_tags.clone();
            let requests = self.requests.clone();
            Box::pin(async move {
                if let Some(tag) = comment_tags.lock().unwrap().get(path).cloned() {
                    return Ok(Some(tag));
                }
                if let Ok(canon) = std::fs::canonicalize(path) {
                    let guard = comment_tags.lock().unwrap();
                    for (k, v) in guard.iter() {
                        if std::fs::canonicalize(k).ok().as_deref() == Some(canon.as_path()) {
                            return Ok(Some(v.clone()));
                        }
                    }
                }
                if let Some(tag) = requests
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|r| r.output == path)
                    .and_then(|r| r.metadata_comment.clone())
                {
                    return Ok(Some(tag));
                }
                // Also support reading mock stub payload across file copies/renames.
                if let Ok(bytes) = tokio::fs::read(path).await
                    && let Ok(text) = std::str::from_utf8(&bytes)
                    && let Some(rest) = text.strip_prefix(MOCK_STUB_PREFIX)
                {
                    let line = rest.lines().next().map(str::trim).unwrap_or_default();
                    if !line.is_empty() {
                        return Ok(Some(line.to_string()));
                    }
                }
                Ok(None)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn list_part_files(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("part"))
            .collect()
    }

    async fn has_ffmpeg() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok()
    }

    async fn generate_synthetic_streams(dir: &Path) -> Option<(PathBuf, PathBuf, PathBuf)> {
        let vid = dir.join("v.mp4");
        let aud = dir.join("a.m4a");
        let durl = dir.join("durl.mp4");

        let v_ok = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:d=0.2",
                "-c:v",
                "mpeg4",
            ])
            .arg(&vid)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .ok()?
            .success();
        if !v_ok {
            return None;
        }

        let a_ok = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=mono",
                "-t",
                "0.2",
                "-c:a",
                "aac",
            ])
            .arg(&aud)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .ok()?
            .success();
        if !a_ok {
            return None;
        }

        let d_ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&vid)
            .arg("-i")
            .arg(&aud)
            .args(["-c", "copy"])
            .arg(&durl)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .ok()?
            .success();
        if !d_ok {
            return None;
        }

        Some((vid, aud, durl))
    }

    async fn ffprobe_stream_types(path: &Path) -> Vec<String> {
        let out = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "default=nw=1:nk=1",
            ])
            .arg(path)
            .stdin(Stdio::null())
            .output()
            .await
            .expect("ffprobe should run");
        assert!(out.status.success(), "ffprobe failed on {}", path.display());
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    #[tokio::test]
    async fn probe_succeeds_or_reports_missing_binary() {
        let missing = FfmpegMuxer::new(Some(PathBuf::from("nonexistent-ffmpeg-binary-xyz-123")));
        let err = missing.probe().await.unwrap_err();
        assert!(
            err.to_string().contains("ffmpeg not found on PATH"),
            "unexpected error: {err}"
        );

        // Missing ffprobe returns Ok(None) gracefully without crashing.
        let dir = TempDir::new().unwrap();
        let dummy = dir.path().join("x.mp4");
        std::fs::write(&dummy, b"abc").unwrap();
        let tag = run_read_comment_tag_with_bin(
            Path::new("nonexistent-ffprobe-binary-xyz-123"),
            &dummy,
            DEFAULT_PROBE_TIMEOUT,
        )
        .await
        .unwrap();
        assert_eq!(tag, None);

        // If system ffmpeg is installed, verify probe captures a clean single-line version.
        if has_ffmpeg().await {
            let muxer = FfmpegMuxer::new(None);
            let ver = muxer.probe().await.expect("ffmpeg probe should succeed");
            assert!(ver.contains("ffmpeg"), "unexpected version line: {ver}");
            assert!(!ver.contains('\n'));
        }
    }

    #[tokio::test]
    async fn ffmpeg_failure_cleans_up_part_and_returns_error() {
        if !has_ffmpeg().await {
            return;
        }

        let dir = TempDir::new().unwrap();
        let bad_video = dir.path().join("corrupt.m4s");
        std::fs::write(&bad_video, b"not a valid media stream").unwrap();
        let output = dir.path().join("out.mp4");

        let muxer = FfmpegMuxer::new(None);
        let err = muxer
            .mux(MuxRequest {
                video: Some(bad_video),
                audio: None,
                output: output.clone(),
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("ffmpeg mux failed"),
            "expected ffmpeg failure error, got: {err}"
        );
        assert!(!output.exists(), "final output must not exist on failure");
        assert!(
            list_part_files(dir.path()).is_empty(),
            "all .part files must be removed on failure"
        );
    }

    #[tokio::test]
    async fn ffmpeg_mux_valid_video_and_audio_succeeds_atomically_with_comment_tag() {
        if !has_ffmpeg().await {
            return;
        }

        let dir = TempDir::new().unwrap();
        let Some((vid, aud, _)) = generate_synthetic_streams(dir.path()).await else {
            return;
        };
        let out = dir.path().join("muxed.mp4");
        let comment = "fav:aid=170001;cid=279786";

        let muxer = FfmpegMuxer::new(None);
        muxer
            .mux(MuxRequest {
                video: Some(vid),
                audio: Some(aud),
                output: out.clone(),
                metadata_comment: Some(comment.to_string()),
                ..Default::default()
            })
            .await
            .expect("mux should succeed");

        assert!(out.exists(), "muxed output should exist");
        assert!(std::fs::metadata(&out).unwrap().len() > 0);
        assert!(
            list_part_files(dir.path()).is_empty(),
            "no leftover .part files after success"
        );

        let read_back = muxer.read_comment_tag(&out).await.unwrap();
        assert_eq!(read_back.as_deref(), Some(comment));
    }

    #[tokio::test]
    async fn ffmpeg_audio_only_formats_durl_strip_and_comment_roundtrip() {
        if !has_ffmpeg().await {
            return;
        }

        let dir = TempDir::new().unwrap();
        let Some((_vid, aud, durl)) = generate_synthetic_streams(dir.path()).await else {
            return;
        };
        let muxer = FfmpegMuxer::new(None);

        let cases: &[(&str, AudioFormat, Option<&str>, &str)] = &[
            (
                "dash_copy.m4a",
                AudioFormat::Copy,
                Some("mp4a.40.2"),
                "fav:aid=10;cid=101",
            ),
            (
                "dash_m4a.m4a",
                AudioFormat::M4a,
                Some("mp4a.40.2"),
                "fav:aid=10;cid=102",
            ),
            (
                "dash_mp3.mp3",
                AudioFormat::Mp3,
                Some("mp4a.40.2"),
                "fav:aid=10;cid=103",
            ),
            (
                "dash_flac.flac",
                AudioFormat::Flac,
                Some("mp4a.40.2"),
                "fav:aid=10;cid=104",
            ),
            (
                "dash_opus.opus",
                AudioFormat::Opus,
                Some("mp4a.40.2"),
                "fav:aid=10;cid=105",
            ),
        ];

        for (file_name, fmt, src_codec, tag) in cases {
            let out = dir.path().join(file_name);
            muxer
                .mux(MuxRequest {
                    video: None,
                    audio: Some(aud.clone()),
                    output: out.clone(),
                    audio_format: Some(*fmt),
                    source_audio_codec: src_codec.map(str::to_string),
                    metadata_comment: Some((*tag).to_string()),
                })
                .await
                .unwrap_or_else(|e| panic!("audio mux failed for {file_name}: {e:#}"));

            assert!(out.is_file(), "missing output {file_name}");
            assert!(std::fs::metadata(&out).unwrap().len() > 0);
            let stream_types = ffprobe_stream_types(&out).await;
            assert!(
                !stream_types.iter().any(|s| s == "video"),
                "{file_name} must have 0 video streams, got: {stream_types:?}"
            );
            assert!(
                stream_types.iter().any(|s| s == "audio"),
                "{file_name} must have an audio stream, got: {stream_types:?}"
            );
            let read_back = muxer.read_comment_tag(&out).await.unwrap();
            assert_eq!(
                read_back.as_deref(),
                Some(*tag),
                "metadata comment roundtrip failed for {file_name}"
            );
        }

        // Legacy durl audio extraction: video+audio input with audio_format=Some(M4a) strips video (-vn)
        let durl_audio_out = dir.path().join("durl_audio.m4a");
        let durl_tag = "fav:aid=99;cid=991";
        muxer
            .mux(MuxRequest {
                video: Some(durl),
                audio: None,
                output: durl_audio_out.clone(),
                audio_format: Some(AudioFormat::M4a),
                source_audio_codec: None,
                metadata_comment: Some(durl_tag.to_string()),
            })
            .await
            .expect("durl audio extraction should succeed");

        let durl_streams = ffprobe_stream_types(&durl_audio_out).await;
        assert!(
            !durl_streams.iter().any(|s| s == "video"),
            "durl audio extraction must produce 0 video streams, got: {durl_streams:?}"
        );
        assert!(
            durl_streams.iter().any(|s| s == "audio"),
            "durl audio extraction must contain an audio stream, got: {durl_streams:?}"
        );
        assert_eq!(
            muxer
                .read_comment_tag(&durl_audio_out)
                .await
                .unwrap()
                .as_deref(),
            Some(durl_tag)
        );
        assert!(list_part_files(dir.path()).is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn future_cancellation_kills_child_and_deletes_part_file() {
        let dir = TempDir::new().unwrap();
        let pid_file = dir.path().join("child.pid");
        let fake_ffmpeg = dir.path().join("fake-ffmpeg.sh");
        // Script records its PID, writes to the final arg (the .part file), prints noise to
        // stdout/stderr (testing isolation), then sleeps long enough to be cancelled.
        // Written via a short-lived `/bin/sh` child so no write FD is ever opened in the test
        // process (preventing Linux ETXTBSY races with concurrent `fork`/`exec` tests).
        let script = format!(
            "#!/bin/sh\necho $$ > \"{}\"\nfor last; do true; done\necho partial-bytes > \"$last\"\necho stdout-noise\necho stderr-noise >&2\nexec sleep 30\n",
            pid_file.display()
        );
        let write_status = Command::new("/bin/sh")
            .args([
                "-c",
                "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"",
                "--",
            ])
            .arg(&script)
            .arg(&fake_ffmpeg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(write_status.success());

        let dummy_in = dir.path().join("in.m4s");
        std::fs::write(&dummy_in, b"data").unwrap();
        let output = dir.path().join("cancelled.mp4");

        let muxer = FfmpegMuxer::new(Some(fake_ffmpeg));
        let mut fut = muxer.mux(MuxRequest {
            video: Some(dummy_in),
            audio: None,
            output: output.clone(),
            ..Default::default()
        });

        // Poll the mux future concurrently while waiting until the child creates the .part file.
        let wait_ready = async {
            for _ in 0..100 {
                if pid_file.exists() && !list_part_files(dir.path()).is_empty() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("fake ffmpeg did not start or create .part file in time");
        };

        tokio::select! {
            res = &mut fut => panic!("fake ffmpeg should not finish early: {res:?}"),
            _ = wait_ready => {}
        }

        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(!list_part_files(dir.path()).is_empty());

        // Drop the in-flight future (simulating Ctrl-C / task cancellation).
        drop(fut);

        // Give OS a brief moment if needed, then confirm .part is gone and PID is dead.
        assert!(
            list_part_files(dir.path()).is_empty(),
            ".part file must be deleted when mux future is cancelled"
        );
        assert!(!output.exists(), "final output must not exist");

        // Verify process is no longer alive (kill -0 returns non-zero when dead).
        let mut dead = false;
        for _ in 0..20 {
            let alive = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !alive {
                dead = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(dead, "child process {pid} should be killed on future drop");
    }

    #[test]
    fn part_file_guard_cleans_up_on_panic_unwind() {
        let dir = TempDir::new().unwrap();
        let part = dir.path().join("unwound.part");
        std::fs::write(&part, b"partial").unwrap();

        let part_clone = part.clone();
        let result = std::panic::catch_unwind(move || {
            let _guard = PartFileGuard::new(part_clone);
            panic!("simulated panic during mux");
        });
        assert!(result.is_err());
        assert!(!part.exists(), ".part file must be removed on panic unwind");
    }

    #[tokio::test]
    async fn mock_muxer_records_requests_and_reads_comment_tags() {
        let dir = TempDir::new().unwrap();
        let out = dir.path().join("mock.mp4");
        let mock = test_support::MockMuxer::succeeding();
        mock.mux(MuxRequest {
            video: Some(dir.path().join("v.m4s")),
            audio: Some(dir.path().join("a.m4s")),
            output: out.clone(),
            metadata_comment: Some("fav:aid=7;cid=77".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(mock.requests().len(), 1);
        assert!(out.exists());
        assert!(mock.probe().await.unwrap().contains("mock"));
        assert_eq!(
            mock.read_comment_tag(&out).await.unwrap().as_deref(),
            Some("fav:aid=7;cid=77")
        );

        // Copied file still resolves stub tag in MockMuxer for cross-disk rename simulation
        let copied = dir.path().join("copied_unrelated.mp4");
        std::fs::copy(&out, &copied).unwrap();
        assert_eq!(
            mock.read_comment_tag(&copied).await.unwrap().as_deref(),
            Some("fav:aid=7;cid=77")
        );
    }
}
