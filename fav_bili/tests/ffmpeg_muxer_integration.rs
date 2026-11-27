mod common;

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use fav_bili::{
    config::Workspace,
    muxer::{FfmpegMuxer, MuxRequest, Muxer as _},
    state::AudioFormat,
};

fn list_part_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .expect("read dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(".part"))
        })
        .collect()
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    // `kill -0` returns 0 if process exists and is not yet reaped.
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn create_slow_ffmpeg_wrapper(dir: &Path, pid_file: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    // Wrapper records its PID, creates the requested output `.part` path, and execs a real
    // long-running `ffmpeg` command so the PID belongs directly to the live `ffmpeg` child.
    let script_path = dir.join("slow_ffmpeg.sh");
    let script = format!(
        r#"#!/bin/sh
echo "$$" > "{pid_file}"
out=""
for arg in "$@"; do
    out="$arg"
done
touch "$out"
exec ffmpeg -hide_banner -loglevel error -y -f lavfi -re -i testsrc=duration=30:size=64x64:rate=5 -f null -
"#,
        pid_file = pid_file.display(),
    );
    std::fs::write(&script_path, script).expect("write wrapper script");
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod wrapper script");
    script_path
}

#[tokio::test]
async fn real_ffmpeg_mux_video_and_audio_produces_valid_dual_stream_mp4_without_part_files() {
    let tmp = tempfile::tempdir().unwrap();
    let (video, audio) = common::generate_tiny_av_streams(tmp.path());
    let output_dir = tmp.path().join("out");
    std::fs::create_dir_all(&output_dir).unwrap();
    let output = output_dir.join("combined.mp4");

    let muxer = FfmpegMuxer::new(None);
    let version = muxer.probe().await.expect("ffmpeg probe should succeed");
    assert!(
        version.to_ascii_lowercase().contains("ffmpeg"),
        "unexpected probe version: {version}"
    );

    muxer
        .mux(MuxRequest {
            video: Some(video),
            audio: Some(audio),
            output: output.clone(),
            ..Default::default()
        })
        .await
        .expect("real ffmpeg mux should succeed");

    assert!(output.is_file(), "muxed output file missing");
    assert!(std::fs::metadata(&output).unwrap().len() > 0);
    assert!(
        list_part_files(&output_dir).is_empty(),
        "stale .part files left after successful mux"
    );

    // Verify via ffprobe that BOTH video and audio streams exist in the final container.
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(&output)
        .output()
        .expect("run ffprobe on muxed mp4");
    assert!(probe.status.success(), "ffprobe failed on muxed mp4");
    let streams = String::from_utf8_lossy(&probe.stdout);
    assert!(
        streams.lines().any(|l| l.trim() == "video"),
        "missing video stream in ffprobe output: {streams}"
    );
    assert!(
        streams.lines().any(|l| l.trim() == "audio"),
        "missing audio stream in ffprobe output: {streams}"
    );
}

#[tokio::test]
async fn real_ffmpeg_corrupt_input_returns_stderr_error_and_cleans_up_part() {
    let tmp = tempfile::tempdir().unwrap();
    let bad_video = tmp.path().join("corrupt.mp4");
    let bad_audio = tmp.path().join("corrupt.m4a");
    std::fs::write(&bad_video, b"not a valid video file at all").unwrap();
    std::fs::write(&bad_audio, b"not a valid audio file at all").unwrap();

    let output_dir = tmp.path().join("out");
    std::fs::create_dir_all(&output_dir).unwrap();
    let output = output_dir.join("failed.mp4");

    let muxer = FfmpegMuxer::new(None);
    let err = muxer
        .mux(MuxRequest {
            video: Some(bad_video),
            audio: Some(bad_audio),
            output: output.clone(),
            ..Default::default()
        })
        .await
        .expect_err("corrupt inputs must fail muxing");

    let msg = format!("{err:#}");
    assert!(
        msg.to_ascii_lowercase().contains("ffmpeg"),
        "error should include ffmpeg diagnostic context, got: {msg}"
    );
    assert!(
        !output.exists(),
        "final output file must not exist after failed mux"
    );
    assert!(
        list_part_files(&output_dir).is_empty(),
        "no .part file may remain after failed mux"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn real_ffmpeg_cancellation_kills_child_process_and_removes_part() {
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join("ffmpeg.pid");
    let wrapper = create_slow_ffmpeg_wrapper(tmp.path(), &pid_file);
    let dummy_v = tmp.path().join("in.mp4");
    std::fs::write(&dummy_v, b"dummy").unwrap();
    let output_dir = tmp.path().join("out");
    std::fs::create_dir_all(&output_dir).unwrap();
    let output = output_dir.join("cancelled.mp4");

    let muxer = FfmpegMuxer::new(Some(wrapper));
    let mut fut = muxer.mux(MuxRequest {
        video: Some(dummy_v),
        audio: None,
        output: output.clone(),
        ..Default::default()
    });

    // Poll until the wrapper has started `exec ffmpeg` and written its PID + .part file.
    let pid: u32 = tokio::select! {
        res = &mut fut => panic!("slow ffmpeg wrapper exited unexpectedly early: {res:?}"),
        pid = async {
            for _ in 0..100 {
                if let Ok(content) = std::fs::read_to_string(&pid_file)
                    && let Ok(parsed) = content.trim().parse::<u32>()
                    && !list_part_files(&output_dir).is_empty()
                {
                    return parsed;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("timed out waiting for slow ffmpeg wrapper to start");
        } => pid,
    };

    assert!(
        is_process_alive(pid),
        "expected ffmpeg child {pid} to be alive before drop"
    );

    // Dropping the in-flight future must immediately kill the child and delete the .part file.
    drop(fut);

    // Give kernel a brief moment to complete process teardown if still finalizing.
    for _ in 0..20 {
        if !is_process_alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(
        !is_process_alive(pid),
        "ffmpeg child process {pid} leaked after future cancellation"
    );
    assert!(!output.exists(), "output file must not exist after cancel");
    assert!(
        list_part_files(&output_dir).is_empty(),
        "stale .part file remained after cancellation"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn panic_unwind_kills_child_cleans_part_and_releases_workspace_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join("panic_ffmpeg.pid");
    let wrapper = create_slow_ffmpeg_wrapper(tmp.path(), &pid_file);
    let dummy_v = tmp.path().join("in.mp4");
    std::fs::write(&dummy_v, b"dummy").unwrap();
    let output_dir = tmp.path().join("out");
    std::fs::create_dir_all(&output_dir).unwrap();
    let output = output_dir.join("panicked.mp4");

    let ws_root = tmp.path().join("ws");
    std::fs::create_dir_all(&ws_root).unwrap();
    let ws = Workspace::new(ws_root.clone());

    let pid_file_clone = pid_file.clone();
    let output_dir_clone = output_dir.clone();
    let handle = tokio::spawn(async move {
        let _lock = ws.acquire_lock().expect("acquire workspace lock");
        let muxer = FfmpegMuxer::new(Some(wrapper));
        let mut fut = muxer.mux(MuxRequest {
            video: Some(dummy_v),
            audio: None,
            output,
            ..Default::default()
        });

        tokio::select! {
            res = &mut fut => panic!("wrapper exited early: {res:?}"),
            _ = async {
                for _ in 0..100 {
                    if std::fs::read_to_string(&pid_file_clone)
                        .ok()
                        .and_then(|s| s.trim().parse::<u32>().ok())
                        .is_some()
                        && !list_part_files(&output_dir_clone).is_empty()
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            } => {}
        }

        panic!("simulated panic while holding ffmpeg child and WorkspaceLock");
    });

    let join_err = handle.await.expect_err("task should panic");
    assert!(join_err.is_panic(), "expected panic unwind (not abort)");

    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("pid file written")
        .trim()
        .parse()
        .expect("valid pid");
    for _ in 0..20 {
        if !is_process_alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(
        !is_process_alive(pid),
        "ffmpeg child {pid} leaked after panic unwind"
    );
    assert!(
        list_part_files(&output_dir).is_empty(),
        "stale .part file remained after panic unwind"
    );

    // WorkspaceLock must have been released during stack unwind.
    let ws2 = Workspace::new(ws_root);
    let _relock = ws2
        .acquire_lock()
        .expect("WorkspaceLock should be released after panic unwind");
}

fn ffprobe_stream_entries(path: &Path, entry: &str) -> Vec<String> {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            entry,
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert!(
        probe.status.success(),
        "ffprobe failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&probe.stderr)
    );
    String::from_utf8_lossy(&probe.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[tokio::test]
async fn real_ffmpeg_audio_only_formats_durl_vn_and_comment_tag_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let (video, audio) = common::generate_tiny_av_streams(tmp.path());
    let durl = common::generate_tiny_durl_mp4(tmp.path(), &video, &audio);
    let output_dir = tmp.path().join("out");
    std::fs::create_dir_all(&output_dir).unwrap();

    let muxer = FfmpegMuxer::new(None);

    // 1. Standard dual-stream MP4 with metadata_comment -> read_comment_tag roundtrip.
    let dual_out = output_dir.join("dual_tagged.mp4");
    let dual_tag = "fav:aid=170001;cid=279786";
    muxer
        .mux(MuxRequest {
            video: Some(video),
            audio: Some(audio.clone()),
            output: dual_out.clone(),
            metadata_comment: Some(dual_tag.to_owned()),
            ..Default::default()
        })
        .await
        .expect("dual mux with metadata_comment should succeed");
    assert_eq!(
        muxer.read_comment_tag(&dual_out).await.unwrap().as_deref(),
        Some(dual_tag)
    );

    // 2. DASH audio-only across all 5 AudioFormat variants: copy, m4a, mp3, flac, opus.
    let cases: &[(&str, AudioFormat, Option<&str>, &str, &str)] = &[
        (
            "audio_copy.m4a",
            AudioFormat::Copy,
            Some("mp4a.40.2"),
            "aac",
            "fav:aid=201;cid=301",
        ),
        (
            "audio_m4a_copy.m4a",
            AudioFormat::M4a,
            Some("mp4a.40.2"),
            "aac",
            "fav:aid=202;cid=302",
        ),
        (
            "audio_m4a_transcode.m4a",
            AudioFormat::M4a,
            None,
            "aac",
            "fav:aid=203;cid=303",
        ),
        (
            "audio_mp3.mp3",
            AudioFormat::Mp3,
            Some("mp4a.40.2"),
            "mp3",
            "fav:aid=204;cid=304",
        ),
        (
            "audio_flac.flac",
            AudioFormat::Flac,
            Some("mp4a.40.2"),
            "flac",
            "fav:aid=205;cid=305",
        ),
        (
            "audio_opus.opus",
            AudioFormat::Opus,
            Some("mp4a.40.2"),
            "opus",
            "fav:aid=206;cid=306",
        ),
    ];

    for (name, fmt, src_codec, expected_codec, tag) in cases {
        let out = output_dir.join(name);
        muxer
            .mux(MuxRequest {
                video: None,
                audio: Some(audio.clone()),
                output: out.clone(),
                audio_format: Some(*fmt),
                source_audio_codec: src_codec.map(str::to_owned),
                metadata_comment: Some((*tag).to_owned()),
            })
            .await
            .unwrap_or_else(|e| panic!("mux failed for {name}: {e:#}"));

        assert!(out.is_file(), "missing {name}");
        assert!(std::fs::metadata(&out).unwrap().len() > 0);
        let types = ffprobe_stream_entries(&out, "stream=codec_type");
        assert_eq!(types, vec!["audio"], "expected audio-only for {name}");
        let codecs = ffprobe_stream_entries(&out, "stream=codec_name");
        assert_eq!(
            codecs,
            vec![(*expected_codec).to_owned()],
            "unexpected codec for {name}"
        );
        assert_eq!(
            muxer.read_comment_tag(&out).await.unwrap().as_deref(),
            Some(*tag),
            "comment tag roundtrip failed for {name}"
        );
    }

    // 3. Legacy `durl` dual-stream MP4 with `-vn` audio extraction (copy + mp3).
    for (name, fmt, expected_codec, tag) in [
        (
            "durl_copy.m4a",
            AudioFormat::Copy,
            "aac",
            "fav:aid=301;cid=401",
        ),
        (
            "durl_mp3.mp3",
            AudioFormat::Mp3,
            "mp3",
            "fav:aid=302;cid=402",
        ),
    ] {
        let out = output_dir.join(name);
        muxer
            .mux(MuxRequest {
                video: Some(durl.clone()),
                audio: None,
                output: out.clone(),
                audio_format: Some(fmt),
                source_audio_codec: None,
                metadata_comment: Some(tag.to_owned()),
            })
            .await
            .unwrap_or_else(|e| panic!("durl -vn mux failed for {name}: {e:#}"));

        let types = ffprobe_stream_entries(&out, "stream=codec_type");
        assert_eq!(
            types,
            vec!["audio"],
            "durl -vn extraction must strip video stream for {name}"
        );
        let codecs = ffprobe_stream_entries(&out, "stream=codec_name");
        assert_eq!(codecs, vec![expected_codec.to_owned()]);
        assert_eq!(
            muxer.read_comment_tag(&out).await.unwrap().as_deref(),
            Some(tag)
        );
    }

    assert!(
        list_part_files(&output_dir).is_empty(),
        "no stale .part files may remain"
    );
}
