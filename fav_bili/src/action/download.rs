use std::{
    collections::{BTreeSet, HashMap, HashSet},
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
            ephemeral_staging: true,
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
            if cancel.is_cancelled() {
                return TargetOutcome::Cancelled;
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
                if cancel.is_cancelled() {
                    return TargetOutcome::Cancelled;
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

    parse_and_dedup_targets(&raw_tokens, opts.all_pages, opts.pages.is_some())
}

fn parse_and_dedup_targets(
    raw_tokens: &[String],
    all_pages: bool,
    has_explicit_pages: bool,
) -> AppResult<Vec<ResolvedTarget>> {
    let mut parsed_targets = Vec::with_capacity(raw_tokens.len());
    for token in raw_tokens {
        let parsed = parse_media_target(token).map_err(AppError::usage)?;
        let aid = parsed
            .ident
            .to_aid()
            .ok_or_else(|| AppError::usage(anyhow!("Cannot convert '{token}' to numeric AID")))?;
        let bv_id = parsed.ident.to_bvid().unwrap_or_else(|| format!("av{aid}"));
        parsed_targets.push(ResolvedTarget {
            aid,
            bv_id,
            url_page: parsed.page,
            raw: token.clone(),
        });
    }

    let mut targets = Vec::with_capacity(parsed_targets.len());
    if all_pages || has_explicit_pages {
        // `--all-pages` and `--page` override URL `?p=N`, so deduplicate strictly by `aid`.
        let mut seen_aids: HashSet<i64> = HashSet::new();
        for target in parsed_targets {
            if seen_aids.insert(target.aid) {
                targets.push(target);
            }
        }
        return Ok(targets);
    }

    // Without `--all-pages`/`--page`, a bare target (`url_page: None`) downloads all pages of
    // that `aid`. Collapse any overlapping `?p=N` entries for the same `aid` (in any order) to
    // the single `url_page: None` target so no page is ever downloaded concurrently twice.
    let mut full_target_by_aid: HashMap<i64, ResolvedTarget> = HashMap::new();
    for target in &parsed_targets {
        if target.url_page.is_none() {
            full_target_by_aid
                .entry(target.aid)
                .or_insert_with(|| target.clone());
        }
    }

    let mut emitted_full_aids: HashSet<i64> = HashSet::new();
    let mut seen_pages: HashSet<(i64, Option<i64>)> = HashSet::new();
    for target in parsed_targets {
        if let Some(full_target) = full_target_by_aid.remove(&target.aid) {
            emitted_full_aids.insert(target.aid);
            targets.push(full_target);
        } else if emitted_full_aids.contains(&target.aid) {
            continue;
        } else if seen_pages.insert((target.aid, target.url_page)) {
            targets.push(target);
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
    use std::time::Duration;

    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::{TcpListener, TcpStream},
        sync::watch,
    };
    use url::Url;

    use super::*;
    use crate::{
        entity::account, error::ExitKind, muxer::test_support::MockMuxer, state::AccountState,
    };

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

    fn test_client_config(addr: std::net::SocketAddr) -> BiliClientConfig {
        let url = Url::parse(&format!("http://{addr}")).unwrap();
        BiliClientConfig {
            api_base_url: url.clone(),
            passport_base_url: url,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
            api_request_timeout: Duration::from_secs(15),
            max_retries: 0,
            base_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
            min_request_interval: Duration::ZERO,
            wbi_ttl: Duration::from_secs(3600),
        }
    }

    async fn read_http_req(stream: &mut TcpStream) -> Option<(String, String, String)> {
        let mut buf = Vec::with_capacity(2048);
        let mut tmp = [0u8; 2048];
        let header_end = loop {
            let n = stream.read(&mut tmp).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]);
        let mut lines = head.lines();
        let target = lines.next()?.split_whitespace().nth(1)?;
        let (path, query) = match target.split_once('?') {
            Some((p, q)) => (p.to_owned(), q.to_owned()),
            None => (target.to_owned(), String::new()),
        };
        let mut cookie = String::new();
        for line in lines {
            if let Some((k, v)) = line.split_once(':')
                && k.trim().eq_ignore_ascii_case("cookie")
            {
                cookie = v.trim().to_owned();
            }
        }
        Some((path, query, cookie))
    }

    fn query_i64(query: &str, key: &str) -> Option<i64> {
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == key).then(|| v.parse().ok()).flatten()
        })
    }

    async fn write_json_resp(stream: &mut TcpStream, body: &Value) {
        let bytes = serde_json::to_vec(body).unwrap();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        let _ = stream.write_all(head.as_bytes()).await;
        let _ = stream.write_all(&bytes).await;
        let _ = stream.flush().await;
    }

    async fn write_bytes_resp(stream: &mut TcpStream, bytes: &[u8]) {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        let _ = stream.write_all(head.as_bytes()).await;
        let _ = stream.write_all(bytes).await;
        let _ = stream.flush().await;
    }

    fn common_bootstrap_resp(path: &str) -> Option<Value> {
        match path {
            "/x/web-interface/nav" => Some(json!({
                "code": 0,
                "message": "0",
                "data": {
                    "isLogin": true,
                    "mid": 1001,
                    "uname": "Tester",
                    "wbi_img": {
                        "img_url": "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
                        "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png",
                    }
                }
            })),
            "/x/web-frontend/getbuvid" | "/x/frontend/finger/spi" => Some(json!({
                "code": 0,
                "message": "0",
                "data": { "buvid": "MOCK-BUVID" }
            })),
            "/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket" => Some(json!({
                "code": 0,
                "message": "0",
                "data": {
                    "ticket": "mock_ticket",
                    "created_at": 4_102_444_800u64,
                    "ttl": 259200u64,
                }
            })),
            _ => None,
        }
    }

    fn view_ok_json(aid: i64, cid: i64) -> Value {
        json!({
            "code": 0,
            "message": "0",
            "data": {
                "aid": aid,
                "bvid": format!("BVmock{aid}"),
                "title": format!("Title {aid}"),
                "owner": { "mid": 1, "name": "UP" },
                "pages": [{ "cid": cid, "page": 1, "part": "P1" }]
            }
        })
    }

    fn playurl_dash_json(origin: &str, aid: i64, cid: i64) -> Value {
        json!({
            "code": 0,
            "message": "0",
            "data": {
                "quality": 80,
                "dash": {
                    "video": [{
                        "id": 80,
                        "baseUrl": format!("{origin}/cdn/v_{aid}_{cid}.m4s"),
                        "backupUrl": [],
                        "mimeType": "video/mp4",
                        "codecs": "avc1.64001f",
                        "bandwidth": 1000,
                        "codecid": 7
                    }],
                    "audio": [{
                        "id": 30280,
                        "baseUrl": format!("{origin}/cdn/a_{aid}_{cid}.m4s"),
                        "backupUrl": [],
                        "mimeType": "audio/mp4",
                        "codecs": "mp4a.40.2",
                        "bandwidth": 500,
                        "codecid": 0
                    }]
                },
                "durl": null
            }
        })
    }

    #[test]
    fn target_dedup_preserves_distinct_url_pages_without_bare_overlap() {
        let raw = vec![
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=1".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=3".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=1".to_owned(),
        ];
        let parsed = parse_and_dedup_targets(&raw, false, false).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].url_page, Some(1));
        assert_eq!(parsed[1].url_page, Some(3));
    }

    #[test]
    fn target_dedup_collapses_overlapping_bare_and_url_pages_in_any_order() {
        // Bare target AFTER `?p=N` entries collapses to single `url_page: None`.
        let bare_after = vec![
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=1".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=3".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=1".to_owned(),
            "av455017605".to_owned(),
            "BV1Q541167Qg".to_owned(),
        ];
        let parsed = parse_and_dedup_targets(&bare_after, false, false).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].aid, 455_017_605);
        assert_eq!(parsed[0].url_page, None);
        assert_eq!(parsed[0].raw, "av455017605");

        // Bare target BEFORE `?p=N` entries collapses to single `url_page: None`.
        let bare_before = vec![
            "BV1Q541167Qg".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=2".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=1".to_owned(),
        ];
        let parsed = parse_and_dedup_targets(&bare_before, false, false).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].aid, 455_017_605);
        assert_eq!(parsed[0].url_page, None);
        assert_eq!(parsed[0].raw, "BV1Q541167Qg");

        // Interleaved across multiple AIDs: overlapping AID collapses while distinct pages on
        // another AID remain separate.
        let mixed = vec![
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=1".to_owned(),
            "https://www.bilibili.com/video/av170001/?p=2".to_owned(),
            "https://www.bilibili.com/video/av170001/?p=4".to_owned(),
            "av455017605".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=3".to_owned(),
        ];
        let parsed = parse_and_dedup_targets(&mixed, false, false).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].aid, 455_017_605);
        assert_eq!(parsed[0].url_page, None);
        assert_eq!(parsed[1].aid, 170_001);
        assert_eq!(parsed[1].url_page, Some(2));
        assert_eq!(parsed[2].aid, 170_001);
        assert_eq!(parsed[2].url_page, Some(4));
    }

    #[test]
    fn target_dedup_deduplicates_strictly_by_aid_when_all_pages_or_explicit_pages() {
        let raw = vec![
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=1".to_owned(),
            "https://www.bilibili.com/video/BV1Q541167Qg/?p=3".to_owned(),
            "av455017605".to_owned(),
            "https://www.bilibili.com/video/av170001/?p=2".to_owned(),
            "https://www.bilibili.com/video/av170001/?p=4".to_owned(),
        ];

        for (all_pages, has_explicit_pages) in [(true, false), (false, true)] {
            let parsed = parse_and_dedup_targets(&raw, all_pages, has_explicit_pages).unwrap();
            assert_eq!(parsed.len(), 2);
            assert_eq!(parsed[0].aid, 455_017_605);
            assert_eq!(parsed[0].raw, raw[0]);
            assert_eq!(parsed[1].aid, 170_001);
            assert_eq!(parsed[1].raw, raw[3]);
        }
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

    #[tokio::test]
    async fn download_with_env_cleans_up_ephemeral_parts_and_fav_tmp_on_mux_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let origin = format!("http://{addr}");
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let origin = origin.clone();
                tokio::spawn(async move {
                    let Some((path, query, _)) = read_http_req(&mut stream).await else {
                        return;
                    };
                    if let Some(resp) = common_bootstrap_resp(&path) {
                        write_json_resp(&mut stream, &resp).await;
                    } else if path == "/x/web-interface/wbi/view" {
                        let aid = query_i64(&query, "aid").unwrap_or(0);
                        write_json_resp(&mut stream, &view_ok_json(aid, 501)).await;
                    } else if path == "/x/player/wbi/playurl" {
                        let avid = query_i64(&query, "avid").unwrap_or(0);
                        let cid = query_i64(&query, "cid").unwrap_or(0);
                        write_json_resp(&mut stream, &playurl_dash_json(&origin, avid, cid)).await;
                    } else if path.starts_with("/cdn/") {
                        write_bytes_resp(&mut stream, b"mock-stream-bytes").await;
                    }
                });
            }
        });

        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        let muxer = MockMuxer::failing("simulated mux failure");

        let err = download_with_env(
            None,
            Config::default(),
            Arc::new(muxer.clone()),
            test_client_config(addr),
            RuntimeOptions {
                no_progress: true,
                non_interactive: true,
                ..Default::default()
            },
            DownloadOptions {
                targets: vec!["av455017605".to_owned()],
                output_dir: Some(out.clone()),
                anonymous: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::PartialFailure);

        let reqs = muxer.requests();
        assert_eq!(reqs.len(), 1);
        let pid_tag = format!("_{}_", std::process::id());
        let v_part = reqs[0].video.as_ref().unwrap();
        let a_part = reqs[0].audio.as_ref().unwrap();
        assert!(
            v_part
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(&pid_tag)
        );
        assert!(
            a_part
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(&pid_tag)
        );
        assert!(
            !v_part.exists(),
            "ephemeral video .part must be removed on mux failure"
        );
        assert!(
            !a_part.exists(),
            "ephemeral audio .part must be removed on mux failure"
        );
        assert!(
            !out.join(".fav-tmp").exists(),
            ".fav-tmp/ must be removed after ephemeral .part cleanup on mux failure"
        );

        server.abort();
    }

    #[tokio::test]
    async fn download_with_env_cleans_up_ephemeral_parts_and_fav_tmp_on_cancellation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let origin = format!("http://{addr}");
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let origin = origin.clone();
                tokio::spawn(async move {
                    let Some((path, query, _)) = read_http_req(&mut stream).await else {
                        return;
                    };
                    if let Some(resp) = common_bootstrap_resp(&path) {
                        write_json_resp(&mut stream, &resp).await;
                    } else if path == "/x/web-interface/wbi/view" {
                        let aid = query_i64(&query, "aid").unwrap_or(0);
                        write_json_resp(&mut stream, &view_ok_json(aid, 501)).await;
                    } else if path == "/x/player/wbi/playurl" {
                        let avid = query_i64(&query, "avid").unwrap_or(0);
                        let cid = query_i64(&query, "cid").unwrap_or(0);
                        write_json_resp(&mut stream, &playurl_dash_json(&origin, avid, cid)).await;
                    } else if path.starts_with("/cdn/") {
                        // Send partial body and hold open so staging `.part` files exist on disk.
                        let head = "HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(head.as_bytes()).await;
                        let _ = stream.write_all(&[b'x'; 128]).await;
                        let _ = stream.flush().await;
                        std::future::pending::<()>().await;
                    }
                });
            }
        });

        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        let tmp_dir = out.join(".fav-tmp");

        let mut dl_fut = Box::pin(download_with_env(
            None,
            Config::default(),
            Arc::new(MockMuxer::succeeding()),
            test_client_config(addr),
            RuntimeOptions {
                no_progress: true,
                non_interactive: true,
                ..Default::default()
            },
            DownloadOptions {
                targets: vec!["av455017605".to_owned()],
                output_dir: Some(out.clone()),
                anonymous: true,
                ..Default::default()
            },
        ));

        let mut saw_part = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            tokio::select! {
                res = &mut dl_fut => panic!("download exited unexpectedly before cancel: {res:?}"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {
                    if let Ok(rd) = std::fs::read_dir(&tmp_dir)
                        && rd.filter_map(Result::ok).any(|e| {
                            e.path().extension().and_then(|s| s.to_str()) == Some("part")
                        })
                    {
                        saw_part = true;
                        break;
                    }
                }
            }
        }
        assert!(
            saw_part,
            "expected in-flight staging .part file inside .fav-tmp/"
        );

        // Dropping the future simulates Ctrl-C cancellation & unwind of RAII guards.
        drop(dl_fut);

        assert!(
            !tmp_dir.exists(),
            ".fav-tmp/ and all ephemeral .part files must be removed upon cancellation"
        );

        server.abort();
    }

    #[tokio::test]
    async fn cancelled_media_info_and_playurl_errors_yield_cancelled_and_retry_next_session() {
        // Part 1: Direct unit verification of `download_single_target` cancellation precedence.
        let (pu_err_reached_tx, mut pu_err_reached_rx) = watch::channel(false);
        let (pu_err_release_tx, pu_err_release_rx) = watch::channel(false);
        let (pu_auth_reached_tx, mut pu_auth_reached_rx) = watch::channel(false);
        let (pu_auth_release_tx, pu_auth_release_rx) = watch::channel(false);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let unit_server =
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let pu_err_reached_tx = pu_err_reached_tx.clone();
                    let mut pu_err_release_rx = pu_err_release_rx.clone();
                    let pu_auth_reached_tx = pu_auth_reached_tx.clone();
                    let mut pu_auth_release_rx = pu_auth_release_rx.clone();
                    tokio::spawn(async move {
                        let Some((path, query, _)) = read_http_req(&mut stream).await else {
                            return;
                        };
                        if let Some(resp) = common_bootstrap_resp(&path) {
                            write_json_resp(&mut stream, &resp).await;
                        } else if path == "/x/web-interface/wbi/view" {
                            match query_i64(&query, "aid").unwrap_or(0) {
                                1 => {
                                    write_json_resp(
                                        &mut stream,
                                        &json!({"code": -400, "message": "bad req", "data": null}),
                                    )
                                    .await
                                }
                                2 => {
                                    write_json_resp(
                                        &mut stream,
                                        &json!({"code": -101, "message": "expired", "data": null}),
                                    )
                                    .await
                                }
                                3 => write_json_resp(&mut stream, &view_ok_json(3, 301)).await,
                                4 => write_json_resp(&mut stream, &view_ok_json(4, 401)).await,
                                _ => write_json_resp(
                                    &mut stream,
                                    &json!({"code": -404, "message": "not found", "data": null}),
                                )
                                .await,
                            }
                        } else if path == "/x/player/wbi/playurl" {
                            match query_i64(&query, "avid").unwrap_or(0) {
                                3 => {
                                    let _ = pu_err_reached_tx.send(true);
                                    let _ = pu_err_release_rx.wait_for(|&v| v).await;
                                    write_json_resp(
                                    &mut stream,
                                    &json!({"code": -400, "message": "playurl err", "data": null}),
                                )
                                .await;
                                }
                                4 => {
                                    let _ = pu_auth_reached_tx.send(true);
                                    let _ = pu_auth_release_rx.wait_for(|&v| v).await;
                                    write_json_resp(
                                        &mut stream,
                                        &json!({"code": -101, "message": "expired", "data": null}),
                                    )
                                    .await;
                                }
                                _ => write_json_resp(
                                    &mut stream,
                                    &json!({"code": -404, "message": "no playurl", "data": null}),
                                )
                                .await,
                            }
                        }
                    });
                }
            });

        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("unit_out");
        let stage = tmp.path().join("unit_tmp");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::create_dir_all(&stage).unwrap();

        let client = BiliClient::with_config("SESSDATA=s1", test_client_config(addr)).unwrap();
        let muxer = MockMuxer::succeeding();
        let params = StreamDownloadParams {
            ephemeral_staging: true,
            ..Default::default()
        };

        let mk_target = |aid: i64| ResolvedTarget {
            aid,
            bv_id: format!("av{aid}"),
            url_page: None,
            raw: format!("av{aid}"),
        };

        // Non-cancelled media_info error -> Failed
        let outcome = download_single_target(
            &client,
            &muxer,
            &mk_target(1),
            &out,
            &stage,
            &params,
            None,
            false,
            false,
            true,
            None,
            CancellationToken::new(),
        )
        .await;
        assert!(matches!(outcome, TargetOutcome::Failed(_)));

        // Cancelled media_info non-auth error -> Cancelled
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let outcome = download_single_target(
            &client,
            &muxer,
            &mk_target(1),
            &out,
            &stage,
            &params,
            None,
            false,
            false,
            true,
            None,
            cancelled.clone(),
        )
        .await;
        assert!(matches!(outcome, TargetOutcome::Cancelled));

        // Auth-expired media_info error takes precedence over cancellation when authenticated
        let outcome = download_single_target(
            &client,
            &muxer,
            &mk_target(2),
            &out,
            &stage,
            &params,
            None,
            false,
            false,
            true,
            None,
            cancelled,
        )
        .await;
        assert!(matches!(outcome, TargetOutcome::AccountExpired(_)));

        // In-flight cancelled fetch_page_playurl non-auth error -> Cancelled
        let playurl_cancel = CancellationToken::new();
        let playurl_cancel_c = playurl_cancel.clone();
        let cancel_task = tokio::spawn(async move {
            let _ = pu_err_reached_rx.wait_for(|&v| v).await;
            playurl_cancel_c.cancel();
            let _ = pu_err_release_tx.send(true);
        });
        let outcome = download_single_target(
            &client,
            &muxer,
            &mk_target(3),
            &out,
            &stage,
            &params,
            None,
            false,
            false,
            true,
            None,
            playurl_cancel,
        )
        .await;
        cancel_task.await.unwrap();
        assert!(matches!(outcome, TargetOutcome::Cancelled));

        // In-flight cancelled fetch_page_playurl auth-expired error -> AccountExpired
        let playurl_auth_cancel = CancellationToken::new();
        let playurl_auth_cancel_c = playurl_auth_cancel.clone();
        let cancel_auth_task = tokio::spawn(async move {
            let _ = pu_auth_reached_rx.wait_for(|&v| v).await;
            playurl_auth_cancel_c.cancel();
            let _ = pu_auth_release_tx.send(true);
        });
        let outcome = download_single_target(
            &client,
            &muxer,
            &mk_target(4),
            &out,
            &stage,
            &params,
            None,
            false,
            false,
            true,
            None,
            playurl_auth_cancel,
        )
        .await;
        cancel_auth_task.await.unwrap();
        assert!(matches!(outcome, TargetOutcome::AccountExpired(_)));

        unit_server.abort();

        // Part 2: End-to-end session failover in `download_with_env` retries cancelled
        // `media_info` (av102) and `fetch_page_playurl` (av103) siblings on session 2.
        let (t2_in_view_tx, t2_in_view_rx) = watch::channel(false);
        let (t3_in_playurl_tx, t3_in_playurl_rx) = watch::channel(false);
        let (t1_expired_tx, t1_expired_rx) = watch::channel(false);

        let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr2 = listener2.local_addr().unwrap();
        let origin2 = format!("http://{addr2}");
        let failover_server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener2.accept().await else {
                    break;
                };
                let origin = origin2.clone();
                let t2_in_view_tx = t2_in_view_tx.clone();
                let mut t2_in_view_rx = t2_in_view_rx.clone();
                let t3_in_playurl_tx = t3_in_playurl_tx.clone();
                let mut t3_in_playurl_rx = t3_in_playurl_rx.clone();
                let t1_expired_tx = t1_expired_tx.clone();
                let mut t1_expired_rx = t1_expired_rx.clone();
                tokio::spawn(async move {
                    let Some((path, query, cookie)) = read_http_req(&mut stream).await else {
                        return;
                    };
                    if let Some(resp) = common_bootstrap_resp(&path) {
                        write_json_resp(&mut stream, &resp).await;
                        return;
                    }
                    let is_expired_session = cookie.contains("SESSDATA=sess_expired");
                    if path == "/x/web-interface/wbi/view" {
                        let aid = query_i64(&query, "aid").unwrap_or(0);
                        if is_expired_session {
                            match aid {
                                101 => {
                                    let _ = tokio::time::timeout(Duration::from_secs(5), async {
                                        let _ = t2_in_view_rx.wait_for(|&v| v).await;
                                        let _ = t3_in_playurl_rx.wait_for(|&v| v).await;
                                    })
                                    .await;
                                    write_json_resp(
                                        &mut stream,
                                        &json!({"code": -101, "message": "account expired", "data": null}),
                                    )
                                    .await;
                                    let _ = t1_expired_tx.send(true);
                                }
                                102 => {
                                    let _ = t2_in_view_tx.send(true);
                                    let _ = tokio::time::timeout(
                                        Duration::from_secs(5),
                                        t1_expired_rx.wait_for(|&v| v),
                                    )
                                    .await;
                                    tokio::time::sleep(Duration::from_millis(50)).await;
                                    write_json_resp(
                                        &mut stream,
                                        &json!({"code": -400, "message": "aborted sibling view", "data": null}),
                                    )
                                    .await;
                                }
                                103 => {
                                    write_json_resp(&mut stream, &view_ok_json(103, 1030)).await;
                                }
                                _ => unreachable!(),
                            }
                        } else {
                            write_json_resp(&mut stream, &view_ok_json(aid, aid * 10)).await;
                        }
                    } else if path == "/x/player/wbi/playurl" {
                        let avid = query_i64(&query, "avid").unwrap_or(0);
                        let cid = query_i64(&query, "cid").unwrap_or(0);
                        if is_expired_session && avid == 103 {
                            let _ = t3_in_playurl_tx.send(true);
                            let _ = tokio::time::timeout(
                                Duration::from_secs(5),
                                t1_expired_rx.wait_for(|&v| v),
                            )
                            .await;
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            write_json_resp(
                                &mut stream,
                                &json!({"code": -400, "message": "aborted sibling playurl", "data": null}),
                            )
                            .await;
                        } else {
                            write_json_resp(&mut stream, &playurl_dash_json(&origin, avid, cid))
                                .await;
                        }
                    } else if path.starts_with("/cdn/") {
                        write_bytes_resp(&mut stream, b"mock-bytes").await;
                    }
                });
            }
        });

        let db = Db::open_in_memory().await.unwrap();
        db.upsert_account(account::Model {
            account_id: 1,
            name: "AliceExpired".to_owned(),
            cookies: "SESSDATA=sess_expired; bili_jct=csrf1".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 2,
            name: "BobValid".to_owned(),
            cookies: "SESSDATA=sess_valid; bili_jct=csrf2".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();

        let failover_out = tmp.path().join("failover_out");
        let failover_muxer = MockMuxer::succeeding();
        download_with_env(
            Some(&db),
            Config::default(),
            Arc::new(failover_muxer.clone()),
            test_client_config(addr2),
            RuntimeOptions {
                no_progress: true,
                non_interactive: true,
                ..Default::default()
            },
            DownloadOptions {
                targets: vec!["av101".to_owned(), "av102".to_owned(), "av103".to_owned()],
                output_dir: Some(failover_out.clone()),
                jobs: Some(3),
                ..Default::default()
            },
        )
        .await
        .expect("all targets including cancelled siblings must succeed on session 2");

        let mux_reqs = failover_muxer.requests();
        assert_eq!(mux_reqs.len(), 3);
        let outputs: BTreeSet<String> = mux_reqs
            .iter()
            .map(|r| r.output.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(outputs.iter().any(|n| n.starts_with("101-")));
        assert!(outputs.iter().any(|n| n.starts_with("102-")));
        assert!(outputs.iter().any(|n| n.starts_with("103-")));
        assert!(!failover_out.join(".fav-tmp").exists());

        failover_server.abort();
    }
}
