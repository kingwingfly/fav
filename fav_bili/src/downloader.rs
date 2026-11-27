use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context as _, Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sanitize_filename::{Options as SanitizeOptions, sanitize_with_options};
use tokio_util::sync::CancellationToken;

use crate::{
    api::{BiliClient, DownloadOptions, DownloadProgress},
    muxer::{MuxRequest, Muxer},
    payload::DashPayload,
    response::{DashData, select_best_audio, select_video_stream},
    state::{AudioFormat, VideoClarity, VideoCodec},
    table::head,
};

const BAR_TEMPLATE: &str = "{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})";
const SPINNER_TEMPLATE: &str =
    "{msg} {spinner:.green} [{elapsed_precise}] {bytes} ({bytes_per_sec})";
pub const MAX_FILENAME_BYTES: usize = 240;

/// Stream-selection and transfer parameters shared by `fav pull` and `fav download`.
#[derive(Debug, Clone)]
pub struct StreamDownloadParams {
    pub sound_only: bool,
    pub audio_format: AudioFormat,
    pub clarity: VideoClarity,
    pub codec: Option<VideoCodec>,
    pub limit_rate: Option<u64>,
    pub try_look: bool,
}

impl Default for StreamDownloadParams {
    fn default() -> Self {
        Self {
            sound_only: false,
            audio_format: AudioFormat::Copy,
            clarity: VideoClarity::Best,
            codec: None,
            limit_rate: None,
            try_look: false,
        }
    }
}

/// Metadata identifying a single page to download and mux.
#[derive(Debug, Clone)]
pub struct PageTarget<'a> {
    pub aid: i64,
    pub bv_id: &'a str,
    pub title: &'a str,
    pub cid: i64,
    pub page: i64,
    pub part: &'a str,
    pub only_one_page: bool,
}

impl PageTarget<'_> {
    pub fn progress_label(&self) -> String {
        if self.only_one_page {
            self.title.to_string()
        } else if self.part.trim().is_empty() {
            format!("{} (P{})", self.title, self.page)
        } else {
            format!("{} (P{}: {})", self.title, self.page, self.part)
        }
    }
}

/// Result metadata for a successfully downloaded & muxed page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedPageInfo {
    pub file_name: String,
    pub full_path: PathBuf,
    pub file_size: u64,
    pub file_inode: Option<u64>,
}

/// Format the embedded container metadata comment tag used for cross-disk rename reconciliation.
pub fn format_comment_tag(aid: i64, cid: i64) -> String {
    format!("fav:aid={aid};cid={cid}")
}

/// Parse a `fav:aid=<aid>;cid=<cid>` container comment tag.
pub fn parse_comment_tag(raw: &str) -> Option<(i64, i64)> {
    for line in raw.lines() {
        let trimmed = line.trim();
        let Some(pos) = trimmed.find("fav:") else {
            continue;
        };
        let body = trimmed[pos + 4..]
            .split_whitespace()
            .next()
            .unwrap_or_default();
        let mut aid: Option<i64> = None;
        let mut cid: Option<i64> = None;
        for part in body.split(';') {
            if let Some((k, v)) = part.split_once('=') {
                match k.trim() {
                    "aid" => aid = v.trim().parse::<i64>().ok().filter(|&n| n > 0),
                    "cid" => cid = v.trim().parse::<i64>().ok().filter(|&n| n > 0),
                    _ => {}
                }
            }
        }
        if let (Some(a), Some(c)) = (aid, cid) {
            return Some((a, c));
        }
    }
    None
}

/// Extract `(file_size, file_inode)` from an on-disk path (`inode` is `None` on non-Unix).
pub fn file_size_and_inode(path: &Path) -> (Option<i64>, Option<i64>) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (None, None);
    };
    let (size, inode) = metadata_size_and_inode(&meta);
    (
        i64::try_from(size).ok(),
        inode.and_then(|ino| i64::try_from(ino).ok()),
    )
}

pub fn metadata_size_and_inode(meta: &std::fs::Metadata) -> (u64, Option<u64>) {
    let size = meta.len();
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt as _;
        Some(meta.ino())
    };
    #[cfg(not(unix))]
    let inode = None;
    (size, inode)
}

/// Resolve a recorded `local_path` (relative to `output_dir` or absolute).
pub fn resolve_recorded_path(output_dir: &Path, recorded: &str) -> PathBuf {
    let p = Path::new(recorded);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        output_dir.join(p)
    }
}

/// Predict the dry-run output filename extension given audio extraction settings.
pub fn preview_extension(sound_only: bool, audio_format: AudioFormat) -> &'static str {
    if sound_only {
        audio_format.extension().unwrap_or("m4a")
    } else {
        "mp4"
    }
}

/// Fetch playurl streams for a page using the configured clarity `qn` and `try_look` flag.
pub async fn fetch_page_playurl(
    client: &BiliClient,
    aid: i64,
    cid: i64,
    params: &StreamDownloadParams,
) -> Result<DashData> {
    let payload = DashPayload::new(aid, cid)
        .with_qn(params.clarity.request_qn())
        .with_try_look(params.try_look);
    client.dash_playurl_with(&payload).await
}

/// Download stream(s) for a single page into `tmp_dir` (never `/tmp`), mux atomically into
/// `output_dir`, embed `fav:aid=...;cid=...` metadata, clean up `.part` files, and return file info.
#[allow(clippy::too_many_arguments)]
pub async fn download_and_mux_page(
    client: &BiliClient,
    muxer: &dyn Muxer,
    target: &PageTarget<'_>,
    dash_data: DashData,
    output_dir: &Path,
    tmp_dir: &Path,
    params: &StreamDownloadParams,
    multi_progress: Option<&MultiProgress>,
    cancel: CancellationToken,
) -> Result<DownloadedPageInfo> {
    let referer = format!("https://www.bilibili.com/video/{}/", target.bv_id);
    let dl_opts = DownloadOptions {
        limit_rate: params.limit_rate,
        referer: Some(referer),
        cancel: Some(cancel),
    };
    let label = target.progress_label();
    let comment_tag = Some(format_comment_tag(target.aid, target.cid));

    if let Some(dash) = dash_data.dash {
        if params.sound_only {
            let best_audio = select_best_audio(&dash).cloned().ok_or_else(|| {
                anyhow!("DASH response contained no audio stream for --sound-only download")
            })?;
            let ext = params
                .audio_format
                .extension()
                .unwrap_or_else(|| best_audio.preferred_audio_extension());
            let a_part = tmp_dir.join(format!(
                "{}_{}_a_{}.part",
                target.aid, target.cid, best_audio.id
            ));

            let pb = make_progress_bar(multi_progress, &label);
            let pb_c = pb.clone();
            let res = client
                .download_to_part(&best_audio.all_urls(), &a_part, &dl_opts, move |p| {
                    update_single_pb(&pb_c, p);
                })
                .await;
            finish_pb(pb);
            res?;

            let file_name = build_safe_filename(
                target.aid,
                target.title,
                target.page,
                target.part,
                target.only_one_page,
                ext,
            );
            let output_path = output_dir.join(&file_name);

            muxer
                .mux(MuxRequest {
                    video: None,
                    audio: Some(a_part.clone()),
                    output: output_path.clone(),
                    audio_format: Some(params.audio_format),
                    source_audio_codec: Some(best_audio.codecs.clone()),
                    metadata_comment: comment_tag,
                })
                .await?;

            let _ = tokio::fs::remove_file(&a_part).await;
            return finalize_page_info(file_name, output_path).await;
        }

        let best_video = select_video_stream(
            &dash.video,
            params.clarity,
            params.codec.map(|c| c.codecid()),
        )
        .cloned();
        let best_audio = select_best_audio(&dash).cloned();

        if best_video.is_none() && best_audio.is_none() {
            return Err(anyhow!(
                "DASH response contained neither video nor audio streams"
            ));
        }

        let ext = if best_video.is_some() {
            "mp4"
        } else {
            best_audio
                .as_ref()
                .map(|a| a.preferred_audio_extension())
                .unwrap_or("m4a")
        };

        let v_part = best_video
            .as_ref()
            .map(|v| tmp_dir.join(format!("{}_{}_v_{}.part", target.aid, target.cid, v.id)));
        let a_part = best_audio
            .as_ref()
            .map(|a| tmp_dir.join(format!("{}_{}_a_{}.part", target.aid, target.cid, a.id)));

        let pb = make_progress_bar(multi_progress, &label);

        // Download video and audio streams cleanly; NEVER drop audio if `best_audio` is present.
        match (
            best_video.as_ref(),
            v_part.as_ref(),
            best_audio.as_ref(),
            a_part.as_ref(),
        ) {
            (Some(v), Some(vp), Some(a), Some(ap)) => {
                let v_urls = v.all_urls();
                let a_urls = a.all_urls();
                let pb_v = pb.clone();
                let pb_a = pb.clone();
                let v_total = Arc::new(AtomicU64::new(0));
                let a_total = Arc::new(AtomicU64::new(0));
                let v_done = Arc::new(AtomicU64::new(0));
                let a_done = Arc::new(AtomicU64::new(0));

                let vt1 = v_total.clone();
                let at1 = a_total.clone();
                let vd1 = v_done.clone();
                let ad1 = a_done.clone();
                let v_fut = client.download_to_part(&v_urls, vp, &dl_opts, move |p| {
                    vd1.store(p.downloaded, Ordering::Relaxed);
                    if let Some(t) = p.total {
                        vt1.store(t, Ordering::Relaxed);
                    }
                    update_combined_pb(&pb_v, &vt1, &at1, &vd1, &ad1);
                });

                let vt2 = v_total.clone();
                let at2 = a_total.clone();
                let vd2 = v_done.clone();
                let ad2 = a_done.clone();
                let a_fut = client.download_to_part(&a_urls, ap, &dl_opts, move |p| {
                    ad2.store(p.downloaded, Ordering::Relaxed);
                    if let Some(t) = p.total {
                        at2.store(t, Ordering::Relaxed);
                    }
                    update_combined_pb(&pb_a, &vt2, &at2, &vd2, &ad2);
                });

                let res = tokio::try_join!(v_fut, a_fut);
                finish_pb(pb);
                res?;
            }
            (Some(v), Some(vp), None, None) => {
                let pb_c = pb.clone();
                let res = client
                    .download_to_part(&v.all_urls(), vp, &dl_opts, move |p| {
                        update_single_pb(&pb_c, p);
                    })
                    .await;
                finish_pb(pb);
                res?;
            }
            (None, None, Some(a), Some(ap)) => {
                let pb_c = pb.clone();
                let res = client
                    .download_to_part(&a.all_urls(), ap, &dl_opts, move |p| {
                        update_single_pb(&pb_c, p);
                    })
                    .await;
                finish_pb(pb);
                res?;
            }
            _ => unreachable!("stream and part path options are constructed together"),
        }

        let file_name = build_safe_filename(
            target.aid,
            target.title,
            target.page,
            target.part,
            target.only_one_page,
            ext,
        );
        let output_path = output_dir.join(&file_name);

        muxer
            .mux(MuxRequest {
                video: v_part.clone(),
                audio: a_part.clone(),
                output: output_path.clone(),
                audio_format: None,
                source_audio_codec: best_audio.as_ref().map(|a| a.codecs.clone()),
                metadata_comment: comment_tag,
            })
            .await?;

        // Clean up staging stream `.part` files only after atomic mux succeeds.
        if let Some(vp) = v_part {
            let _ = tokio::fs::remove_file(vp).await;
        }
        if let Some(ap) = a_part {
            let _ = tokio::fs::remove_file(ap).await;
        }

        return finalize_page_info(file_name, output_path).await;
    }

    // Legacy non-DASH `durl` fallback.
    if let Some(durl) = dash_data.durl.first() {
        let part_path = tmp_dir.join(format!("{}_{}_durl.part", target.aid, target.cid));
        let pb = make_progress_bar(multi_progress, &label);
        let pb_c = pb.clone();
        let res = client
            .download_to_part(&durl.all_urls(), &part_path, &dl_opts, move |p| {
                update_single_pb(&pb_c, p);
            })
            .await;
        finish_pb(pb);
        res?;

        let (ext, audio_format) = if params.sound_only {
            (
                params.audio_format.extension().unwrap_or("m4a"),
                Some(params.audio_format),
            )
        } else {
            ("mp4", None)
        };

        let file_name = build_safe_filename(
            target.aid,
            target.title,
            target.page,
            target.part,
            target.only_one_page,
            ext,
        );
        let output_path = output_dir.join(&file_name);
        muxer
            .mux(MuxRequest {
                video: Some(part_path.clone()),
                audio: None,
                output: output_path.clone(),
                audio_format,
                source_audio_codec: None,
                metadata_comment: comment_tag,
            })
            .await?;
        let _ = tokio::fs::remove_file(part_path).await;
        return finalize_page_info(file_name, output_path).await;
    }

    Err(anyhow!(
        "Playurl response contained neither DASH nor durl streams"
    ))
}

/// Convenience wrapper fetching playurl and downloading/muxing a single page.
#[allow(clippy::too_many_arguments)]
pub async fn download_media_page(
    client: &BiliClient,
    muxer: &dyn Muxer,
    target: &PageTarget<'_>,
    output_dir: &Path,
    tmp_dir: &Path,
    params: &StreamDownloadParams,
    multi_progress: Option<&MultiProgress>,
    cancel: CancellationToken,
) -> Result<DownloadedPageInfo> {
    let dash_data = fetch_page_playurl(client, target.aid, target.cid, params).await?;
    download_and_mux_page(
        client,
        muxer,
        target,
        dash_data,
        output_dir,
        tmp_dir,
        params,
        multi_progress,
        cancel,
    )
    .await
}

async fn finalize_page_info(file_name: String, full_path: PathBuf) -> Result<DownloadedPageInfo> {
    let meta = tokio::fs::metadata(&full_path).await.with_context(|| {
        format!(
            "Muxed output '{}' missing after completion",
            full_path.display()
        )
    })?;
    let (file_size, file_inode) = metadata_size_and_inode(&meta);
    Ok(DownloadedPageInfo {
        file_name,
        full_path,
        file_size,
        file_inode,
    })
}

/// Build a cross-OS-safe filename strictly `<= 240` UTF-8 bytes that NEVER truncates away the
/// media `id`, multi-page `({page})` index, or container `.{ext}` extension.
pub fn build_safe_filename(
    media_id: i64,
    title: &str,
    page: i64,
    part: &str,
    only_one_page: bool,
    ext: &str,
) -> String {
    let sanitize_opts = SanitizeOptions {
        windows: true,
        truncate: false,
        replacement: "_",
    };

    let clean_ext = sanitize_with_options(ext, sanitize_opts.clone())
        .trim_matches(|c| c == '.' || c == ' ')
        .to_string();
    let clean_ext = if clean_ext.is_empty() {
        "mp4".to_string()
    } else {
        clean_ext
    };

    let clean_title = sanitize_component(title, &sanitize_opts);
    let clean_part = sanitize_component(part, &sanitize_opts);

    let prefix = format!("{media_id}-");
    let suffix = format!(".{clean_ext}");

    if only_one_page {
        let fixed_len = prefix.len() + suffix.len();
        let budget = MAX_FILENAME_BYTES.saturating_sub(fixed_len);
        let trunc_title = truncate_utf8_bytes(&clean_title, budget);
        let stem = if trunc_title.is_empty() {
            format!("{media_id}")
        } else {
            format!("{prefix}{trunc_title}")
        };
        let stem = stem.trim_end_matches(['.', ' ']);
        return format!("{stem}{suffix}");
    }

    let page_marker = format!("({page})");
    if clean_part.is_empty() {
        let fixed_len = prefix.len() + page_marker.len() + suffix.len();
        let budget = MAX_FILENAME_BYTES.saturating_sub(fixed_len);
        let trunc_title = truncate_utf8_bytes(&clean_title, budget);
        let stem = format!("{prefix}{trunc_title}{page_marker}");
        let stem = stem.trim_end_matches(['.', ' ']);
        return format!("{stem}{suffix}");
    }

    // Multi-page with part title: preserve `{id}-`, `({page})-`, and `.{ext}`, splitting remaining
    // UTF-8 byte budget proportionally between `title` and `part`.
    let fixed_len = prefix.len() + page_marker.len() + 1 + suffix.len(); // +1 for '-'
    let budget = MAX_FILENAME_BYTES.saturating_sub(fixed_len);

    let (title_budget, part_budget) = if clean_title.len() + clean_part.len() <= budget {
        (clean_title.len(), clean_part.len())
    } else {
        let half = budget / 2;
        if clean_title.len() <= half {
            (clean_title.len(), budget.saturating_sub(clean_title.len()))
        } else if clean_part.len() <= half {
            (budget.saturating_sub(clean_part.len()), clean_part.len())
        } else {
            (half, budget.saturating_sub(half))
        }
    };

    let trunc_title = truncate_utf8_bytes(&clean_title, title_budget);
    let trunc_part = truncate_utf8_bytes(&clean_part, part_budget);
    let stem = if trunc_part.is_empty() {
        format!("{prefix}{trunc_title}{page_marker}")
    } else {
        format!("{prefix}{trunc_title}{page_marker}-{trunc_part}")
    };
    let stem = stem.trim_end_matches(['.', ' ']);
    format!("{stem}{suffix}")
}

fn sanitize_component(raw: &str, opts: &SanitizeOptions<'_>) -> String {
    let cleaned = sanitize_with_options(raw.trim(), opts.clone());
    cleaned.trim().trim_end_matches(['.', ' ']).to_string()
}

fn truncate_utf8_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim_end_matches(['.', ' '])
}

pub fn make_progress_bar(
    multi_progress: Option<&MultiProgress>,
    label: &str,
) -> Option<ProgressBar> {
    let mp = multi_progress?;
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_message(head(label, 24));
    if let Ok(style) = ProgressStyle::with_template(SPINNER_TEMPLATE) {
        pb.set_style(style);
    }
    Some(pb)
}

pub fn update_single_pb(pb: &Option<ProgressBar>, p: DownloadProgress) {
    let Some(pb) = pb else {
        return;
    };
    if let Some(total) = p.total
        && pb.length() != Some(total)
    {
        pb.set_length(total);
        if let Ok(style) = ProgressStyle::with_template(BAR_TEMPLATE) {
            pb.set_style(style.progress_chars("#>-"));
        }
    }
    pb.set_position(p.downloaded);
}

pub fn update_combined_pb(
    pb: &Option<ProgressBar>,
    v_total: &AtomicU64,
    a_total: &AtomicU64,
    v_done: &AtomicU64,
    a_done: &AtomicU64,
) {
    let Some(pb) = pb else {
        return;
    };
    let vt = v_total.load(Ordering::Relaxed);
    let at = a_total.load(Ordering::Relaxed);
    if vt > 0 && at > 0 {
        let total = vt + at;
        if pb.length() != Some(total) {
            pb.set_length(total);
            if let Ok(style) = ProgressStyle::with_template(BAR_TEMPLATE) {
                pb.set_style(style.progress_chars("#>-"));
            }
        }
    }
    let pos = v_done.load(Ordering::Relaxed) + a_done.load(Ordering::Relaxed);
    pb.set_position(pos);
}

pub fn finish_pb(pb: Option<ProgressBar>) {
    if let Some(pb) = pb {
        pb.finish_and_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_tag_format_and_parse_roundtrip() {
        let tag = format_comment_tag(170_001, 279_786);
        assert_eq!(tag, "fav:aid=170001;cid=279786");
        assert_eq!(parse_comment_tag(&tag), Some((170_001, 279_786)));
        assert_eq!(
            parse_comment_tag("user note\nfav:aid=42;cid=99 extra"),
            Some((42, 99))
        );
        assert_eq!(parse_comment_tag("fav:cid=99;aid=42"), Some((42, 99)));
        assert_eq!(parse_comment_tag(""), None);
        assert_eq!(parse_comment_tag("unrelated comment"), None);
        assert_eq!(parse_comment_tag("fav:aid=0;cid=99"), None);
        assert_eq!(parse_comment_tag("fav:aid=abc;cid=99"), None);
        assert_eq!(parse_comment_tag("fav:aid=42"), None);
    }

    #[test]
    fn file_size_and_inode_reads_existing_and_handles_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("sample.mp4");
        std::fs::write(&file, b"hello world").unwrap();

        let (size, inode) = file_size_and_inode(&file);
        assert_eq!(size, Some(11));
        #[cfg(unix)]
        assert!(inode.is_some_and(|i| i > 0));
        #[cfg(not(unix))]
        assert_eq!(inode, None);

        assert_eq!(
            file_size_and_inode(&tmp.path().join("missing.mp4")),
            (None, None)
        );
    }

    #[test]
    fn long_cjk_multipage_filenames_stay_under_240_bytes_and_never_collide() {
        let long_title = "这是一个非常非常长的中文视频标题用来测试文件名字节长度截断以及多P视频分集编号绝对不会被截断导致互相覆盖的情况".repeat(3);
        let long_part = "分集名称也非常长包含各种说明文字和课程章节信息".repeat(4);

        let p1 = build_safe_filename(123456789, &long_title, 1, &long_part, false, "mp4");
        let p2 = build_safe_filename(123456789, &long_title, 2, &long_part, false, "mp4");

        assert!(
            p1.len() <= MAX_FILENAME_BYTES,
            "p1 byte len {} > {MAX_FILENAME_BYTES}: {p1}",
            p1.len()
        );
        assert!(
            p2.len() <= MAX_FILENAME_BYTES,
            "p2 byte len {} > {MAX_FILENAME_BYTES}: {p2}",
            p2.len()
        );
        assert!(p1.starts_with("123456789-"));
        assert!(p1.contains("(1)"));
        assert!(p2.contains("(2)"));
        assert!(p1.ends_with(".mp4"));
        assert!(p2.ends_with(".mp4"));
        assert_ne!(
            p1, p2,
            "different pages must never collide after truncation"
        );
    }

    #[test]
    fn windows_unsafe_chars_and_trailing_dots_are_sanitized_on_all_platforms() {
        let name = build_safe_filename(42, "A/B:C*D?\"<E>|... ", 1, "", true, "m4a");
        assert!(!name.contains('/'));
        assert!(!name.contains(':'));
        assert!(!name.contains('*'));
        assert!(!name.contains('?'));
        assert!(!name.contains('<'));
        assert!(!name.contains('>'));
        assert!(!name.contains('|'));
        assert!(name.ends_with(".m4a"));
        assert!(!name.ends_with("..m4a"));
        assert!(name.len() <= MAX_FILENAME_BYTES);
    }
}
