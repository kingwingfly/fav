use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, anyhow};
use console::Term;
use futures::StreamExt as _;
use indicatif::{MultiProgress, ProgressDrawTarget};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::{
    api::{BiliApiError, BiliClient, BiliClientConfig},
    config::{Config, Workspace},
    context::{DEFAULT_JOBS, RuntimeOptions},
    db::Db,
    downloader::{
        PageTarget, StreamDownloadParams, build_safe_filename, download_and_mux_page,
        fetch_page_playurl, preview_extension,
    },
    error::{AppError, AppResult},
    id_parse::{parse_media_target, parse_page_spec},
    muxer::{FfmpegMuxer, Muxer},
    response::{Page, select_best_audio},
    state::{AudioFormat, VideoClarity, VideoCodec},
    table::write_stdout_line,
};

#[derive(Debug, Clone, Default)]
pub struct DownloadOptions {
    pub targets: Vec<String>,
    pub read_stdin: bool,
    pub pages: Option<String>,
    pub all_pages: bool,
    pub output_dir: Option<PathBuf>,
    pub jobs: Option<usize>,
    pub limit_rate: Option<u64>,
    pub sound_only: bool,
    pub audio_format: Option<AudioFormat>,
    pub clarity: Option<VideoClarity>,
    pub codec: Option<VideoCodec>,
    pub force: bool,
    pub anonymous: bool,
    pub dry_run: bool,
    pub ignore_errors: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTarget {
    aid: i64,
    bv_id: String,
    url_page: Option<i64>,
    raw: String,
}

enum TargetOutcome {
    Downloaded,
    Skipped,
    Failed(String),
    AccountExpired(String),
    Cancelled,
}

/// RAII guard that removes an owned `.fav-tmp/` directory on drop if and only if it is empty.
struct EmptyDirGuard {
    path: Option<PathBuf>,
}

impl EmptyDirGuard {
    fn new(path: PathBuf, owned: bool) -> Self {
        Self {
            path: owned.then_some(path),
        }
    }
}

impl Drop for EmptyDirGuard {
    fn drop(&mut self) {
        if let Some(ref p) = self.path {
            let _ = std::fs::remove_dir(p);
        }
    }
}

/// Entry point for untracked `fav download` / `fav dl`: zero DB writes and zero `.fav/fav.lock`.
pub async fn download(runtime: RuntimeOptions, opts: DownloadOptions) -> AppResult<()> {
    let workspace = match Workspace::discover_existing(runtime.explicit_dir.as_deref()) {
        Ok(ws) => Some(ws),
        Err(e) => {
            if runtime.explicit_dir.is_some() {
                warn!(
                    "No workspace found at `-C/--dir` ({e:#}); proceeding without workspace cookies."
                );
            }
            None
        }
    };

    let config = Config::load(workspace.as_ref())?;
    let db = if opts.anonymous {
        None
    } else if let Some(ref ws) = workspace {
        match Db::open_existing(&ws.db_path(), true).await {
            Ok(db) => Some(db),
            Err(e) => {
                warn!(
                    "Could not open workspace database read-only ({e:#}); falling back to anonymous download."
                );
                None
            }
        }
    } else {
        None
    };

    let muxer: Arc<dyn Muxer> = Arc::new(FfmpegMuxer::new(config.ffmpeg_path.clone()));
    download_with_env(
        db.as_ref(),
        config,
        muxer,
        BiliClientConfig::default(),
        runtime,
        opts,
    )
    .await
}

/// Injectable untracked download engine (zero DB writes, zero workspace lock) for CLI & integration tests.
pub async fn download_with_env(
    db: Option<&Db>,
    config: Config,
    muxer: Arc<dyn Muxer>,
    api_config: BiliClientConfig,
    mut runtime: RuntimeOptions,
    opts: DownloadOptions,
) -> AppResult<()> {
    // Eagerly validate config keys so malformed TOML fails fast with ExitKind::Usage.
    let cfg_rate = config.parsed_limit_rate()?;
    let cfg_clarity = config.parsed_clarity()?;
    let cfg_codec = config.parsed_codec()?;
    let cfg_audio_fmt = config.parsed_audio_format()?;
    if config.no_progress == Some(true) {
        runtime.no_progress = true;
    }

    if opts.all_pages && opts.pages.is_some() {
        return Err(AppError::usage(anyhow!(
            "Cannot combine `--all-pages` with `--page <SPEC>`."
        )));
    }
    let explicit_pages: Option<BTreeSet<i64>> = opts
        .pages
        .as_deref()
        .map(parse_page_spec)
        .transpose()
        .map_err(AppError::usage)?;

    let targets = collect_targets(&opts).await?;
    if targets.is_empty() {
        return Err(AppError::usage(anyhow!(
            "No download targets specified. Pass BVID(s), avID(s), numeric AID(s), Bilibili video URL(s), or '-' / --stdin."
        )));
    }

    let output_dir = opts
        .output_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let (tmp_dir, owns_tmp_dir) = resolve_download_tmp_dir(&output_dir, config.tmp_dir.as_deref());

    let jobs = opts.jobs.or(config.jobs).unwrap_or(DEFAULT_JOBS).max(1);
    let limit_rate = opts.limit_rate.or(cfg_rate);
    let sound_only = opts.sound_only || config.sound_only.unwrap_or(false);
    let audio_format = opts.audio_format.or(cfg_audio_fmt).unwrap_or_default();
    let clarity = opts.clarity.or(cfg_clarity).unwrap_or_default();
    let codec = opts.codec.or(cfg_codec);

    // Resolve read-only auth sessions (NEVER writes DB, NEVER touches inactive accounts).
    let mut sessions: Vec<(Option<String>, BiliClient, bool)> = Vec::new();
    if !opts.anonymous
        && let Some(db) = db
    {
        let accounts = db.active_accounts().await?;
        for acct in accounts {
            let client = BiliClient::with_config(&acct.cookies, api_config.clone())?;
            sessions.push((Some(acct.name), client, false));
        }
    }
    if sessions.is_empty() {
        let anon = BiliClient::with_config("", api_config.clone())?;
        if opts.anonymous {
            info!("Downloading anonymously (--anonymous; try_look=1).");
        } else {
            warn!(
                "No active workspace account found; downloading anonymously (try_look=1, quality may be limited)."
            );
        }
        sessions.push((None, anon, true));
    }

    if opts.dry_run {
        let auth_label = match sessions[0].0.as_deref() {
            Some(name) => format!("account<{name}>"),
            None => "anonymous(try_look=1)".to_owned(),
        };
        let ext = preview_extension(sound_only, audio_format);
        eprintln!(
            "[dry-run] Output directory: {} ({} target(s), auth={auth_label}, ext=.{ext})",
            output_dir.display(),
            targets.len()
        );
        for t in &targets {
            let page_desc = if opts.all_pages {
                "all".to_owned()
            } else if let Some(ref spec) = opts.pages {
                format!("page={spec}")
            } else if let Some(p) = t.url_page {
                format!("p={p}")
            } else {
                "all".to_owned()
            };
            if !write_stdout_line(&format!(
                "{}\t{}\t{}\t{}",
                t.aid,
                t.bv_id,
                page_desc,
                output_dir.display()
            ))
            .map_err(AppError::runtime)?
            {
                return Ok(());
            }
        }
        return Ok(());
    }

    // Fail fast if ffmpeg is missing or broken before downloading a single byte.
    let ffmpeg_version = muxer
        .probe()
        .await
        .context("ffmpeg preflight check failed; cannot mux downloads")?;
    debug!(ffmpeg = %ffmpeg_version, "ffmpeg preflight passed");

    tokio::fs::create_dir_all(&output_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to create output directory '{}'",
                output_dir.display()
            )
        })?;
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .with_context(|| format!("Failed to create temp directory '{}'", tmp_dir.display()))?;
    // Remove `.fav-tmp/` automatically when empty on exit, cancellation, or unwind (Issue #153).
    let _tmp_cleanup = EmptyDirGuard::new(tmp_dir.clone(), owns_tmp_dir);

    let multi_progress = build_multi_progress(&runtime);
    let global_cancel = CancellationToken::new();

    let mut remaining: Vec<ResolvedTarget> = targets;
    let mut downloaded_targets = 0usize;
    let mut skipped_targets = 0usize;
    let mut failed_targets = 0usize;
    let mut expired_sessions = 0usize;
    let mut interrupted = false;

    'sessions: for (acct_name, client, try_look) in &sessions {
        if global_cancel.is_cancelled() {
            interrupted = true;
            break 'sessions;
        }
        if remaining.is_empty() {
            break 'sessions;
        }

        // Bootstrap anti-bot cookies in memory only (NEVER writes back to DB).
        if let Err(e) = client.ensure_anti_bot_cookies().await {
            debug!("Could not bootstrap anti-bot cookies for download: {e:#}");
        }

        let session_label = acct_name
            .as_deref()
            .map(|n| format!("account<{n}>"))
            .unwrap_or_else(|| "anonymous".to_owned());
        info!(
            "Downloading {} target(s) with {session_label} (jobs={jobs})",
            remaining.len()
        );

        let stream_params = Arc::new(StreamDownloadParams {
            sound_only,
            audio_format,
            clarity,
            codec,
            limit_rate,
            try_look: *try_look,
        });
        let session_cancel = global_cancel.child_token();
        let output_dir_arc = Arc::new(output_dir.clone());
        let tmp_dir_arc = Arc::new(tmp_dir.clone());
        let explicit_pages_arc = Arc::new(explicit_pages.clone());
        let mp_arc = multi_progress.clone();
        let all_pages = opts.all_pages;
        let force = opts.force;
        let is_authenticated = acct_name.is_some();

        let batch = std::mem::take(&mut remaining);
        let mut retry_next_session = Vec::new();
        let mut session_expired = false;

        let mut tasks = futures::stream::iter(batch)
            .map(|target| {
                let client = client.clone();
                let muxer = muxer.clone();
                let token = session_cancel.clone();
                let output_dir = output_dir_arc.clone();
                let tmp_dir = tmp_dir_arc.clone();
                let params = stream_params.clone();
                let explicit_pages = explicit_pages_arc.clone();
                let mp = mp_arc.clone();
                async move {
                    if token.is_cancelled() {
                        return (target, TargetOutcome::Cancelled);
                    }
                    let outcome = download_single_target(
                        &client,
                        muxer.as_ref(),
                        &target,
                        &output_dir,
                        &tmp_dir,
                        &params,
                        explicit_pages.as_ref().as_ref(),
                        all_pages,
                        force,
                        is_authenticated,
                        mp.as_ref(),
                        token,
                    )
                    .await;
                    (target, outcome)
                }
            })
            .buffer_unordered(jobs);

        loop {
            tokio::select! {
                maybe_res = tasks.next() => {
                    let Some((target, outcome)) = maybe_res else {
                        break;
                    };
                    match outcome {
                        TargetOutcome::Downloaded => downloaded_targets += 1,
                        TargetOutcome::Skipped => skipped_targets += 1,
                        TargetOutcome::Failed(msg) => {
                            failed_targets += 1;
                            error!("{msg}");
                        }
                        TargetOutcome::AccountExpired(msg) => {
                            retry_next_session.push(target);
                            if !session_expired {
                                session_expired = true;
                                expired_sessions += 1;
                                session_cancel.cancel();
                                warn!(
                                    "{session_label} session expired ({msg}); trying next account without mutating DB."
                                );
                            }
                        }
                        TargetOutcome::Cancelled => {
                            if session_expired && !global_cancel.is_cancelled() {
                                retry_next_session.push(target);
                            }
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    warn!("Received Ctrl-C — cancelling active downloads and cleaning up...");
                    global_cancel.cancel();
                    interrupted = true;
                    drop(tasks);
                    break 'sessions;
                }
            }
        }

        remaining = retry_next_session;
    }

    info!(
        "Download finished: Downloaded {downloaded_targets}, Failed {failed_targets}, Skipped {skipped_targets}."
    );

    if interrupted {
        return Err(AppError::runtime(anyhow!(
            "Download interrupted by Ctrl-C (Downloaded {downloaded_targets}, Failed {failed_targets}, Skipped {skipped_targets})."
        )));
    }

    if !remaining.is_empty() && !opts.ignore_errors {
        if expired_sessions == sessions.len() && downloaded_targets == 0 {
            return Err(AppError::auth(anyhow!(
                "All active accounts expired during download. Run `fav auth check` or pass `--anonymous`."
            )));
        }
        return Err(AppError::partial(anyhow!(
            "Download finished with {} unfinished target(s).",
            remaining.len()
        )));
    }

    if failed_targets > 0 && !opts.ignore_errors {
        return Err(AppError::partial(anyhow!(
            "Download finished with {failed_targets} failure(s) (Downloaded {downloaded_targets}, Skipped {skipped_targets})."
        )));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_single_target(
    client: &BiliClient,
    muxer: &dyn Muxer,
    target: &ResolvedTarget,
    output_dir: &Path,
    tmp_dir: &Path,
    params: &StreamDownloadParams,
    explicit_pages: Option<&BTreeSet<i64>>,
    all_pages: bool,
    force: bool,
    is_authenticated: bool,
    multi_progress: Option<&MultiProgress>,
    cancel: CancellationToken,
) -> TargetOutcome {
    let info = match client.media_info(target.aid).await {
        Ok(info) => info,
        Err(e) => {
            if is_authenticated
                && let Some(api_err) = e.downcast_ref::<BiliApiError>()
                && api_err.is_auth_expired()
            {
                return TargetOutcome::AccountExpired(api_err.to_string());
            }
            return TargetOutcome::Failed(format!(
                "Failed fetching metadata for '{}' (av{}): {e:#}",
                target.raw, target.aid
            ));
        }
    };

    if info.pages.is_empty() {
        return TargetOutcome::Failed(format!(
            "Target '{}' (av{}) returned 0 playable pages.",
            target.raw, target.aid
        ));
    }

    let selected_pages = match select_pages_for_target(
        &info.pages,
        all_pages,
        explicit_pages,
        target.url_page,
        &target.raw,
    ) {
        Ok(pages) => pages,
        Err(e) => return TargetOutcome::Failed(format!("{e:#}")),
    };

    let only_one_page = info.pages.len() == 1;
    let bv_id = if !info.bvid.trim().is_empty() {
        info.bvid.as_str()
    } else {
        target.bv_id.as_str()
    };
    let media_title = if !info.title.trim().is_empty() {
        info.title.as_str()
    } else {
        bv_id
    };

    let mut downloaded_pages = 0usize;

    for Page {
        cid, page, part, ..
    } in selected_pages
    {
        if cancel.is_cancelled() {
            return TargetOutcome::Cancelled;
        }

        // Fast pre-check before playurl when output extension is statically known.
        if !force && let Some(static_ext) = static_output_extension(params) {
            let file_name = build_safe_filename(
                target.aid,
                media_title,
                page,
                &part,
                only_one_page,
                static_ext,
            );
            let candidate = output_dir.join(&file_name);
            if tokio::fs::metadata(&candidate)
                .await
                .is_ok_and(|m| m.is_file() && m.len() > 0)
            {
                info!(
                    "Skipping existing file '{}' (pass --force to overwrite).",
                    candidate.display()
                );
                continue;
            }
        }

        let dash_data = match fetch_page_playurl(client, target.aid, cid, params).await {
            Ok(d) => d,
            Err(e) => {
                if is_authenticated
                    && let Some(api_err) = e.downcast_ref::<BiliApiError>()
                    && api_err.is_auth_expired()
                {
                    return TargetOutcome::AccountExpired(api_err.to_string());
                }
                return TargetOutcome::Failed(format!(
                    "Failed fetching playurl for '{}' page {page}: {e:#}",
                    target.raw
                ));
            }
        };

        // Post-playurl skip check for `sound_only` + `AudioFormat::Copy` (where ext depends on stream).
        if !force && params.sound_only && params.audio_format == AudioFormat::Copy {
            let resolved_ext = dash_data
                .dash
                .as_ref()
                .and_then(select_best_audio)
                .map(|a| a.preferred_audio_extension())
                .unwrap_or("m4a");
            let file_name = build_safe_filename(
                target.aid,
                media_title,
                page,
                &part,
                only_one_page,
                resolved_ext,
            );
            let candidate = output_dir.join(&file_name);
            if tokio::fs::metadata(&candidate)
                .await
                .is_ok_and(|m| m.is_file() && m.len() > 0)
            {
                info!(
                    "Skipping existing file '{}' (pass --force to overwrite).",
                    candidate.display()
                );
                continue;
            }
        }

        let page_target = PageTarget {
            aid: target.aid,
            bv_id,
            title: media_title,
            cid,
            page,
            part: &part,
            only_one_page,
        };

        match download_and_mux_page(
            client,
            muxer,
            &page_target,
            dash_data,
            output_dir,
            tmp_dir,
            params,
            multi_progress,
            cancel.clone(),
        )
        .await
        {
            Ok(info) => {
                downloaded_pages += 1;
                info!("Saved {}", info.full_path.display());
            }
            Err(e) => {
                if cancel.is_cancelled() {
                    return TargetOutcome::Cancelled;
                }
                return TargetOutcome::Failed(format!(
                    "Failed downloading '{}' (av{}) page {page}: {e:#}",
                    target.raw, target.aid
                ));
            }
        }
    }

    if downloaded_pages > 0 {
        TargetOutcome::Downloaded
    } else {
        TargetOutcome::Skipped
    }
}

fn static_output_extension(params: &StreamDownloadParams) -> Option<&'static str> {
    if params.sound_only {
        params.audio_format.extension()
    } else {
        Some("mp4")
    }
}

fn resolve_download_tmp_dir(output_dir: &Path, cfg_tmp_dir: Option<&Path>) -> (PathBuf, bool) {
    match cfg_tmp_dir {
        Some(p) if p.is_absolute() => (p.to_path_buf(), false),
        Some(p) => (output_dir.join(p), false),
        None => (output_dir.join(".fav-tmp"), true),
    }
}

fn select_pages_for_target(
    available: &[Page],
    all_pages: bool,
    explicit_pages: Option<&BTreeSet<i64>>,
    url_page: Option<i64>,
    raw_target: &str,
) -> Result<Vec<Page>> {
    if all_pages {
        return Ok(available.to_vec());
    }

    let available_nums: BTreeSet<i64> = available.iter().map(|p| p.page).collect();
    let max_page = available_nums.iter().copied().max().unwrap_or(0);

    if let Some(spec) = explicit_pages {
        let missing: Vec<i64> = spec
            .iter()
            .copied()
            .filter(|p| !available_nums.contains(p))
            .collect();
        if !missing.is_empty() {
            return Err(anyhow!(
                "Requested page(s) {:?} out of range for '{}' (available pages: 1..={max_page}).",
                missing,
                raw_target
            ));
        }
        return Ok(available
            .iter()
            .filter(|p| spec.contains(&p.page))
            .cloned()
            .collect());
    }

    if let Some(p_num) = url_page {
        if !available_nums.contains(&p_num) {
            return Err(anyhow!(
                "Requested page ?p={p_num} out of range for '{}' (available pages: 1..={max_page}).",
                raw_target
            ));
        }
        return Ok(available
            .iter()
            .filter(|p| p.page == p_num)
            .cloned()
            .collect());
    }

    Ok(available.to_vec())
}

async fn collect_targets(opts: &DownloadOptions) -> AppResult<Vec<ResolvedTarget>> {
    let mut raw_tokens = Vec::new();
    let mut wants_stdin = opts.read_stdin;

    for raw in &opts.targets {
        let trimmed = raw.trim();
        if trimmed == "-" {
            wants_stdin = true;
            continue;
        }
        if !trimmed.is_empty() {
            raw_tokens.push(trimmed.to_owned());
        }
    }

    if wants_stdin {
        let buf = tokio::task::spawn_blocking(|| {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
            Ok::<_, std::io::Error>(s)
        })
        .await
        .context("Failed joining stdin reader task")?
        .context("Failed reading download targets from stdin")?;
        for line in buf.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            for token in trimmed.split_whitespace() {
                if token.starts_with('#') {
                    break;
                }
                raw_tokens.push(token.to_owned());
            }
        }
    }

    parse_and_dedup_targets(&raw_tokens)
}

fn parse_and_dedup_targets(raw_tokens: &[String]) -> AppResult<Vec<ResolvedTarget>> {
    let mut targets = Vec::with_capacity(raw_tokens.len());
    let mut seen: HashSet<(i64, Option<i64>)> = HashSet::new();
    for token in raw_tokens {
        let parsed = parse_media_target(token).map_err(AppError::usage)?;
        let aid = parsed
            .ident
            .to_aid()
            .ok_or_else(|| AppError::usage(anyhow!("Cannot convert '{token}' to numeric AID")))?;
        let bv_id = parsed.ident.to_bvid().unwrap_or_else(|| format!("av{aid}"));
        if seen.insert((aid, parsed.page)) {
            targets.push(ResolvedTarget {
                aid,
                bv_id,
                url_page: parsed.page,
                raw: token.clone(),
            });
        }
    }
    Ok(targets)
}

fn build_multi_progress(runtime: &RuntimeOptions) -> Option<MultiProgress> {
    if !runtime.quiet && !runtime.no_progress && Term::stderr().is_term() {
        Some(MultiProgress::with_draw_target(ProgressDrawTarget::stderr()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_pages(n: i64) -> Vec<Page> {
        (1..=n)
            .map(|p| Page {
                cid: 1000 + p,
                page: p,
                part: format!("Part {p}"),
                duration: 60,
            })
            .collect()
    }

    #[test]
    fn target_dedup_preserves_distinct_url_pages_and_dedups_duplicates() {
        let raw = vec![
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=1".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=3".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=1".to_owned(),
            "av455017605".to_owned(),
            "BV1Q541167Qg".to_owned(),
        ];
        let parsed = parse_and_dedup_targets(&raw).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].url_page, Some(1));
        assert_eq!(parsed[1].url_page, Some(3));
        assert_eq!(parsed[2].url_page, None);
    }

    #[test]
    fn page_selection_precedence_and_out_of_range_errors() {
        let pages = mk_pages(4);

        // --all-pages overrides URL ?p=2
        let sel = select_pages_for_target(&pages, true, None, Some(2), "t").unwrap();
        assert_eq!(sel.len(), 4);

        // --page overrides URL ?p=2
        let spec = parse_page_spec("1,3-4").unwrap();
        let sel = select_pages_for_target(&pages, false, Some(&spec), Some(2), "t").unwrap();
        assert_eq!(
            sel.iter().map(|p| p.page).collect::<Vec<_>>(),
            vec![1, 3, 4]
        );

        // URL ?p=2 alone selects only page 2
        let sel = select_pages_for_target(&pages, false, None, Some(2), "t").unwrap();
        assert_eq!(sel.iter().map(|p| p.page).collect::<Vec<_>>(), vec![2]);

        // Neither -> all pages
        let sel = select_pages_for_target(&pages, false, None, None, "t").unwrap();
        assert_eq!(sel.len(), 4);

        // Out-of-range --page errors cleanly
        let bad_spec = parse_page_spec("1,5").unwrap();
        let err = select_pages_for_target(&pages, false, Some(&bad_spec), None, "t").unwrap_err();
        assert!(err.to_string().contains("out of range"));

        // Out-of-range URL ?p=9 errors cleanly
        let err = select_pages_for_target(&pages, false, None, Some(9), "t").unwrap_err();
        assert!(err.to_string().contains("?p=9"));
    }

    #[test]
    fn download_tmp_dir_uses_output_fav_tmp_and_cleans_up_only_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        let (dir, owned) = resolve_download_tmp_dir(&out, None);
        assert_eq!(dir, out.join(".fav-tmp"));
        assert!(owned);

        // Empty owned dir is removed on guard drop
        std::fs::create_dir_all(&dir).unwrap();
        {
            let _guard = EmptyDirGuard::new(dir.clone(), owned);
        }
        assert!(!dir.exists());

        // Non-empty owned dir (e.g. resumable `.part` file) is preserved on guard drop
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1_2_v_80.part"), b"partial").unwrap();
        {
            let _guard = EmptyDirGuard::new(dir.clone(), owned);
        }
        assert!(dir.is_dir());

        // Explicit config.tmp_dir is NOT removed
        let custom = tmp.path().join("custom_stage");
        let (dir2, owned2) = resolve_download_tmp_dir(&out, Some(&custom));
        assert_eq!(dir2, custom);
        assert!(!owned2);
    }

    #[tokio::test]
    async fn download_with_env_validates_conflicts_and_dry_run_touches_no_db() {
        use crate::{error::ExitKind, muxer::test_support::MockMuxer};

        let db = Db::open_in_memory().await.unwrap();
        let muxer: Arc<dyn Muxer> = Arc::new(MockMuxer::succeeding());

        // Combining --all-pages and --page fails with ExitKind::Usage
        let err = download_with_env(
            Some(&db),
            Config::default(),
            muxer.clone(),
            BiliClientConfig::default(),
            RuntimeOptions {
                no_progress: true,
                non_interactive: true,
                ..Default::default()
            },
            DownloadOptions {
                targets: vec!["BV1Q541167Qg".to_owned()],
                pages: Some("1-2".to_owned()),
                all_pages: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);

        // Dry run succeeds cleanly without writing anything to DB
        let tmp = tempfile::tempdir().unwrap();
        download_with_env(
            Some(&db),
            Config::default(),
            muxer,
            BiliClientConfig::default(),
            RuntimeOptions {
                no_progress: true,
                non_interactive: true,
                ..Default::default()
            },
            DownloadOptions {
                targets: vec!["https://www.bilibili.com/video/BV1Q541167Qg/?p=2".to_owned()],
                output_dir: Some(tmp.path().join("out")),
                sound_only: true,
                audio_format: Some(AudioFormat::Opus),
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(db.all_medias().await.unwrap().is_empty());
        assert!(!tmp.path().join("out").exists());
    }
}
