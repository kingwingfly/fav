use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr as _,
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result, anyhow};
use futures::StreamExt as _;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

pub use crate::downloader::build_safe_filename;
use crate::{
    api::{BiliApiError, BiliClient},
    context::AppContext,
    db::{Db, PullFilter},
    downloader::{
        PageTarget, StreamDownloadParams, download_and_mux_page, fetch_page_playurl,
        preview_extension, resolve_recorded_path,
    },
    entity::{media, media_page},
    error::{AppError, AppResult},
    id_parse::parse_media_ident,
    reconcile::{ReconcileReport, apply_reconciliation, reconcile_completed_medias},
    response::Page,
    state::{
        AccountState, AudioFormat, MediaPageState, MediaState, MediaType, SetState, UpState,
        VideoClarity, VideoCodec,
    },
    table::write_stdout_line,
};

#[derive(Debug, Clone, Default)]
pub struct PullOptions {
    pub targets: Vec<String>,
    pub set_ids: Vec<i64>,
    pub up_ids: Vec<i64>,
    pub limit: Option<usize>,
    pub jobs: Option<usize>,
    pub output_dir: Option<PathBuf>,
    pub limit_rate: Option<u64>,
    pub retry_failed: bool,
    pub force: bool,
    pub dry_run: bool,
    pub ignore_errors: bool,
    pub sound_only: bool,
    pub audio_format: Option<AudioFormat>,
    pub clarity: Option<VideoClarity>,
    pub codec: Option<VideoCodec>,
    pub recheck: bool,
}

enum MediaOutcome {
    Downloaded,
    Skipped,
    DeferredToNextAccount,
    Failed(String),
    AccountExpired(String),
    Cancelled,
}

pub async fn pull(ctx: &AppContext, opts: PullOptions) -> AppResult<()> {
    let target_entries = resolve_target_entries(&opts.targets)?;
    validate_pull_scope(ctx, &opts, &target_entries).await?;
    let target_aids: Vec<i64> = target_entries.iter().map(|(_, aid)| *aid).collect();

    let output_dir = ctx.effective_output_dir(opts.output_dir.as_deref());
    let sound_only = ctx.effective_sound_only(opts.sound_only);
    let audio_format = ctx.effective_audio_format(opts.audio_format);
    let clarity = ctx.effective_clarity(opts.clarity);
    let codec = ctx.effective_codec(opts.codec);
    let limit_rate = ctx.effective_limit_rate(opts.limit_rate);

    let should_recheck = should_run_recheck(ctx, &opts, &target_aids).await?;
    let recheck_scope = if should_recheck {
        compute_recheck_scope(ctx, &opts, &target_aids).await?
    } else {
        None
    };

    if opts.dry_run {
        let recheck_report = if should_recheck {
            Some(
                reconcile_completed_medias(
                    &ctx.db,
                    ctx.muxer.as_ref(),
                    &output_dir,
                    true,
                    recheck_scope.as_ref(),
                )
                .await?,
            )
        } else {
            None
        };
        return run_pull_dry_run(
            ctx,
            &opts,
            &target_aids,
            &output_dir,
            sound_only,
            audio_format,
            recheck_report.as_ref(),
        )
        .await;
    }

    // Fail fast if `ffmpeg` is missing or broken before downloading a single byte.
    let ffmpeg_version = ctx
        .muxer
        .probe()
        .await
        .context("ffmpeg preflight check failed; cannot mux downloads")?;
    debug!(ffmpeg = %ffmpeg_version, "ffmpeg preflight passed");

    let accounts = ctx.db.active_accounts().await?;
    if accounts.is_empty() {
        return Err(AppError::usage(anyhow!(
            "No active accounts. Run `fav auth login` or `fav activate account` first."
        )));
    }

    tokio::fs::create_dir_all(&output_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to create output directory '{}'",
                output_dir.display()
            )
        })?;
    let tmp_dir = ctx.effective_tmp_dir();
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .with_context(|| format!("Failed to create temp directory '{}'", tmp_dir.display()))?;

    if should_recheck {
        let report = reconcile_completed_medias(
            &ctx.db,
            ctx.muxer.as_ref(),
            &output_dir,
            true,
            recheck_scope.as_ref(),
        )
        .await?;
        let applied = apply_reconciliation(&ctx.db, &report).await?;
        if applied.relinked_pages > 0 {
            info!(
                "Recheck relinked {} relocated/renamed file(s) in DB.",
                applied.relinked_pages
            );
        }
        if applied.reset_medias > 0 {
            info!(
                "Recheck queued {} media(s) ({} missing page(s), {} missing legacy media(s)) for re-download.",
                applied.reset_medias,
                report.missing_pages.len(),
                report.missing_legacy_ids.len(),
            );
        } else {
            info!(
                "Recheck verified {} active completed item(s) on disk; 0 missing.",
                report.total_verified()
            );
        }
    }

    let jobs = ctx.effective_jobs(opts.jobs);
    let stream_params = Arc::new(StreamDownloadParams {
        sound_only,
        audio_format,
        clarity,
        codec,
        limit_rate,
        try_look: false,
    });
    let global_cancel = CancellationToken::new();

    let mut downloaded_total = 0usize;
    let mut failed_total = 0usize;
    let mut skipped_total = 0usize;
    let mut attempted_accounts = 0usize;
    let mut expired_accounts = 0usize;
    let mut expired_unfinished_ids: HashSet<i64> = HashSet::new();
    let mut handled_ids: HashSet<i64> = HashSet::new();
    let mut interrupted = false;

    'accounts: for (idx, account) in accounts.iter().enumerate() {
        if global_cancel.is_cancelled() {
            interrupted = true;
            break 'accounts;
        }

        let remaining_limit = opts
            .limit
            .map(|limit| limit.saturating_sub(downloaded_total + failed_total + skipped_total));
        if remaining_limit == Some(0) {
            break 'accounts;
        }

        // Re-query candidates scoped to this active account so Account A never attempts
        // Account B's exclusive videos, and already-handled items from earlier accounts are
        // not re-attempted (including during `--force` or `--retry-failed` runs).
        let query_limit = remaining_limit.map(|rem| rem.saturating_add(handled_ids.len()));
        let mut candidates: Vec<media::Model> = query_pull_candidates(
            ctx,
            &opts,
            &target_aids,
            Some(account.account_id),
            query_limit,
        )
        .await?
        .into_iter()
        .filter(|m| !handled_ids.contains(&m.id))
        .collect();
        if let Some(rem) = remaining_limit {
            candidates.truncate(rem);
        }
        if candidates.is_empty() {
            continue 'accounts;
        }
        attempted_accounts += 1;
        let account_candidate_ids: Vec<i64> = candidates.iter().map(|m| m.id).collect();

        info!(
            "Pulling {} media(s) with account<{}> (jobs={jobs})",
            candidates.len(),
            account.name
        );

        let client = ctx.client_for_account(&account.cookies)?;
        match client.ensure_anti_bot_cookies().await {
            Ok(true) => {
                let updated = client.export_cookies();
                let _ = ctx
                    .db
                    .update_account_cookies(account.account_id, updated, None)
                    .await;
            }
            Ok(false) => {}
            Err(e) => {
                debug!(
                    "Could not bootstrap anti-bot cookies for {}: {e:#}",
                    account.name
                );
            }
        }

        let account_cancel = global_cancel.child_token();
        let output_dir_arc = Arc::new(output_dir.clone());
        let tmp_dir_arc = Arc::new(tmp_dir.clone());
        let in_flight = Arc::new(Mutex::new(HashSet::new()));
        let remaining_account_ids: Arc<Vec<i64>> =
            Arc::new(accounts[idx + 1..].iter().map(|a| a.account_id).collect());

        let mut tasks = futures::stream::iter(candidates)
            .map(|media_row| {
                let client = client.clone();
                let token = account_cancel.clone();
                let output_dir = output_dir_arc.clone();
                let tmp_dir = tmp_dir_arc.clone();
                let params = stream_params.clone();
                let in_flight = in_flight.clone();
                let remaining_accounts = remaining_account_ids.clone();
                let force = opts.force;
                async move {
                    if token.is_cancelled() {
                        return (media_row.id, MediaOutcome::Cancelled);
                    }
                    let outcome = download_media(
                        ctx,
                        &client,
                        &media_row,
                        &output_dir,
                        &tmp_dir,
                        &params,
                        force,
                        token,
                        &in_flight,
                        &remaining_accounts,
                    )
                    .await;
                    (media_row.id, outcome)
                }
            })
            .buffer_unordered(jobs);

        let mut account_expired = false;

        loop {
            tokio::select! {
                maybe_res = tasks.next() => {
                    let Some((media_id, outcome)) = maybe_res else {
                        break;
                    };
                    match outcome {
                        MediaOutcome::Downloaded => {
                            downloaded_total += 1;
                            handled_ids.insert(media_id);
                        }
                        MediaOutcome::Skipped => {
                            skipped_total += 1;
                            handled_ids.insert(media_id);
                        }
                        MediaOutcome::DeferredToNextAccount => {}
                        MediaOutcome::Failed(msg) => {
                            failed_total += 1;
                            handled_ids.insert(media_id);
                            error!("{msg}");
                        }
                        MediaOutcome::AccountExpired(msg) => {
                            if !account_expired {
                                account_expired = true;
                                expired_accounts += 1;
                                account_cancel.cancel();
                                let _ = ctx
                                    .db
                                    .set_account_state(account.account_id, AccountState::Expired)
                                    .await;
                                error!(
                                    "Account<{}> session expired ({msg}); marked Expired and switching accounts.",
                                    account.name
                                );
                            }
                        }
                        MediaOutcome::Cancelled => {}
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    warn!("Received Ctrl-C — cancelling active downloads and cleaning up...");
                    global_cancel.cancel();
                    interrupted = true;
                    // Drop the unordered stream immediately so in-flight muxer futures unwind and
                    // their RAII guards kill child `ffmpeg` processes and remove `.part` outputs.
                    drop(tasks);
                    match rollback_inflight_downloading(&ctx.db, &in_flight).await {
                        Ok(reset) if reset > 0 => {
                            info!("Reset {reset} interrupted in-flight media(s) back to Pending.");
                        }
                        Ok(_) => {}
                        Err(e) => {
                            warn!("Failed rolling back interrupted in-flight media(s): {e:#}");
                        }
                    }
                    break 'accounts;
                }
            }
        }

        if account_expired {
            for id in account_candidate_ids {
                if !handled_ids.contains(&id) {
                    expired_unfinished_ids.insert(id);
                }
            }
        }
    }

    let expired_with_unfinished = expired_unfinished_ids
        .iter()
        .any(|id| !handled_ids.contains(id));

    if downloaded_total == 0
        && failed_total == 0
        && skipped_total == 0
        && expired_accounts == 0
        && !interrupted
        && target_aids.is_empty()
        && opts.set_ids.is_empty()
        && opts.up_ids.is_empty()
        && let Ok(summary) = ctx.db.status_summary(0).await
        && let Ok(active_pending) = ctx.db.count_active_pending_medias().await
    {
        let inactive_pending = summary.medias.pending.saturating_sub(active_pending);
        if inactive_pending > 0 {
            info!(
                "No active pending medias to pull ({inactive_pending} pending media(s) belong to inactive sets/UPs)."
            );
        }
    }

    info!(
        "Pull finished: Downloaded {downloaded_total}, Failed {failed_total}, Skipped {skipped_total}."
    );

    if interrupted {
        return Err(AppError::runtime(anyhow!(
            "Pull interrupted by Ctrl-C (Downloaded {downloaded_total}, Failed {failed_total}, Skipped {skipped_total})."
        )));
    }

    if failed_total > 0 && !opts.ignore_errors {
        if attempted_accounts > 0
            && expired_accounts == attempted_accounts
            && downloaded_total == 0
            && skipped_total == 0
        {
            return Err(AppError::auth(anyhow!(
                "All active accounts with pull candidates expired during pull."
            )));
        }
        return Err(AppError::partial(anyhow!(
            "Pull finished with {failed_total} failure(s) (Downloaded {downloaded_total}, Skipped {skipped_total})."
        )));
    }

    if attempted_accounts > 0
        && expired_accounts == attempted_accounts
        && downloaded_total == 0
        && skipped_total == 0
    {
        return Err(AppError::auth(anyhow!(
            "All active accounts with pull candidates expired during pull."
        )));
    }

    if expired_accounts > 0 && expired_with_unfinished && !opts.ignore_errors {
        return Err(AppError::partial(anyhow!(
            "Pull finished with {expired_accounts} expired account(s) leaving unfinished item(s) (Downloaded {downloaded_total}, Skipped {skipped_total})."
        )));
    }

    Ok(())
}

async fn validate_pull_scope(
    ctx: &AppContext,
    opts: &PullOptions,
    target_entries: &[(String, i64)],
) -> AppResult<()> {
    for &set_id in &opts.set_ids {
        let set_row = ctx.db.get_set(set_id).await.map_err(AppError::usage)?;
        if set_row.state != SetState::Active.as_str() {
            return Err(AppError::usage(anyhow!(
                "Set<{set_id} {}> is {} (not Active). Run `fav activate set {set_id}` first.",
                set_row.name,
                set_row.state
            )));
        }
        if !ctx.db.is_set_owned_by_active_account(set_id).await? {
            return Err(AppError::usage(anyhow!(
                "Set<{set_id} {}> is not owned by any Active account. Run `fav activate account` first.",
                set_row.name
            )));
        }
    }

    for &up_id in &opts.up_ids {
        let up_row = ctx.db.get_up(up_id).await.map_err(AppError::usage)?;
        if up_row.state != UpState::Active.as_str() {
            return Err(AppError::usage(anyhow!(
                "UP<{up_id} {}> is {} (not Active). Run `fav activate up {up_id}` first.",
                up_row.name,
                up_row.state
            )));
        }
        if !ctx.db.is_up_followed_by_active_account(up_id).await? {
            return Err(AppError::usage(anyhow!(
                "UP<{up_id} {}> is not followed by any Active account. Run `fav activate account` first.",
                up_row.name
            )));
        }
    }

    for (raw, aid) in target_entries {
        let media_row = ctx.db.get_media(*aid).await.map_err(|_| {
            AppError::usage(anyhow!(
                "Target '{raw}' (av{aid}) is not tracked in this workspace. Run `fav activate ... && fav fetch`, or use `fav download {raw}` for one-off downloads."
            ))
        })?;
        if !ctx.db.is_media_active(*aid).await? {
            return Err(AppError::usage(anyhow!(
                "Target '{}' (av{aid}) does not belong to any Active set or Active UP under an Active account. Run `fav activate ... && fav fetch`, or use `fav download {raw}` for one-off downloads.",
                media_row.bv_id
            )));
        }
    }

    Ok(())
}

async fn should_run_recheck(
    ctx: &AppContext,
    opts: &PullOptions,
    target_aids: &[i64],
) -> Result<bool> {
    if ctx.effective_recheck(opts.recheck) {
        return Ok(true);
    }
    if target_aids.is_empty() {
        return Ok(false);
    }
    for &aid in target_aids {
        if ctx
            .db
            .get_media(aid)
            .await
            .is_ok_and(|m| m.state == MediaState::Completed.as_str())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn compute_recheck_scope(
    ctx: &AppContext,
    opts: &PullOptions,
    target_aids: &[i64],
) -> Result<Option<HashSet<i64>>> {
    if target_aids.is_empty() && opts.set_ids.is_empty() && opts.up_ids.is_empty() {
        return Ok(None);
    }
    let scope_opts = PullOptions {
        set_ids: opts.set_ids.clone(),
        up_ids: opts.up_ids.clone(),
        force: true,
        limit: None,
        ..Default::default()
    };
    let rows = query_pull_candidates(ctx, &scope_opts, target_aids, None, None).await?;
    let ids: HashSet<i64> = rows
        .into_iter()
        .filter(|m| m.state == MediaState::Completed.as_str())
        .map(|m| m.id)
        .collect();
    Ok(Some(ids))
}

async fn rollback_inflight_downloading(db: &Db, in_flight: &Mutex<HashSet<i64>>) -> Result<u64> {
    let ids: Vec<i64> = {
        let mut guard = in_flight.lock().unwrap_or_else(|e| e.into_inner());
        guard.drain().collect()
    };
    db.reset_downloading_medias(&ids).await
}

#[allow(clippy::too_many_arguments)]
async fn download_media(
    ctx: &AppContext,
    client: &BiliClient,
    media_row: &media::Model,
    output_dir: &Path,
    tmp_dir: &Path,
    stream_params: &StreamDownloadParams,
    force: bool,
    cancel: CancellationToken,
    in_flight: &Mutex<HashSet<i64>>,
    remaining_account_ids: &[i64],
) -> MediaOutcome {
    match MediaType::from_str(&media_row.r#type) {
        Ok(MediaType::Video) => {}
        Ok(other) => {
            warn!(
                "Skipping unsupported media type '{other}' for media<{} {}>.",
                media_row.id, media_row.title
            );
            return MediaOutcome::Skipped;
        }
        Err(_) => {
            warn!(
                "Skipping unknown media type '{}' for media<{} {}>.",
                media_row.r#type, media_row.id, media_row.title
            );
            return MediaOutcome::Skipped;
        }
    }

    {
        let mut guard = in_flight.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(media_row.id);
    }
    let outcome = download_video_media(
        ctx,
        client,
        media_row,
        output_dir,
        tmp_dir,
        stream_params,
        force,
        cancel,
        remaining_account_ids,
    )
    .await;
    {
        let mut guard = in_flight.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(&media_row.id);
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn download_video_media(
    ctx: &AppContext,
    client: &BiliClient,
    media_row: &media::Model,
    output_dir: &Path,
    tmp_dir: &Path,
    stream_params: &StreamDownloadParams,
    force: bool,
    cancel: CancellationToken,
    remaining_account_ids: &[i64],
) -> MediaOutcome {
    if let Err(e) = ctx
        .db
        .set_media_state(media_row.id, MediaState::Downloading)
        .await
    {
        return MediaOutcome::Failed(format!(
            "Failed marking media<{}> Downloading: {e:#}",
            media_row.id
        ));
    }

    let info = match client.media_info(media_row.id).await {
        Ok(info) => info,
        Err(e) => {
            return handle_media_api_error(ctx, media_row, e, &cancel, remaining_account_ids).await;
        }
    };

    if info.pages.is_empty() {
        let msg = format!(
            "Media<{} {}> returned 0 playable pages.",
            media_row.id, media_row.title
        );
        let _ = ctx.db.record_media_failure(media_row.id, &msg).await;
        return MediaOutcome::Failed(msg);
    }

    let existing_pages: HashMap<i64, media_page::Model> = ctx
        .db
        .pages_for_media(media_row.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p.cid, p))
        .collect();

    let only1p = info.pages.len() == 1;
    let media_title = if !info.title.trim().is_empty() {
        info.title.as_str()
    } else {
        media_row.title.as_str()
    };

    let mp = if ctx.progress_enabled() {
        ctx.multi_progress.as_ref()
    } else {
        None
    };

    for Page {
        cid, page, part, ..
    } in info.pages
    {
        if cancel.is_cancelled() {
            let _ = ctx
                .db
                .set_media_state(media_row.id, MediaState::Pending)
                .await;
            return MediaOutcome::Cancelled;
        }

        // Skip already-completed pages whose local output file still exists and is non-empty.
        if !force
            && let Some(prev) = existing_pages.get(&cid)
            && prev.state == MediaPageState::Completed.as_str()
            && let Some(ref rel_or_abs) = prev.local_path
        {
            let candidate_path = resolve_recorded_path(output_dir, rel_or_abs);
            if tokio::fs::metadata(&candidate_path)
                .await
                .is_ok_and(|m| m.len() > 0)
            {
                debug!(
                    aid = media_row.id,
                    cid,
                    path = %candidate_path.display(),
                    "Skipping already-completed page present on disk"
                );
                continue;
            }
        }

        let dash_data = match fetch_page_playurl(client, media_row.id, cid, stream_params).await {
            Ok(d) => d,
            Err(e) => {
                let outcome =
                    handle_media_api_error(ctx, media_row, e, &cancel, remaining_account_ids).await;
                if matches!(outcome, MediaOutcome::Failed(_)) {
                    let _ = ctx
                        .db
                        .mark_page_failed(media_row.id, cid, page as i32, part.clone())
                        .await;
                }
                return outcome;
            }
        };

        let target = PageTarget {
            aid: media_row.id,
            bv_id: &media_row.bv_id,
            title: media_title,
            cid,
            page,
            part: &part,
            only_one_page: only1p,
        };

        let page_res = download_and_mux_page(
            client,
            ctx.muxer.as_ref(),
            &target,
            dash_data,
            output_dir,
            tmp_dir,
            stream_params,
            mp,
            cancel.clone(),
        )
        .await;

        match page_res {
            Ok(page_info) => {
                let file_size = i64::try_from(page_info.file_size).ok();
                let file_inode = page_info.file_inode.and_then(|ino| i64::try_from(ino).ok());
                if let Err(e) = ctx
                    .db
                    .mark_page_completed(
                        media_row.id,
                        cid,
                        page as i32,
                        part,
                        page_info.file_name,
                        file_size,
                        file_inode,
                    )
                    .await
                {
                    warn!(
                        "Downloaded page {page} of media<{}> but failed updating media_page: {e:#}",
                        media_row.id
                    );
                }
            }
            Err(e) => {
                if cancel.is_cancelled() {
                    let _ = ctx
                        .db
                        .set_media_state(media_row.id, MediaState::Pending)
                        .await;
                    return MediaOutcome::Cancelled;
                }
                let err_msg = format!(
                    "Failed downloading media<{} {}> page {page}: {e:#}",
                    media_row.id, media_title
                );
                let _ = ctx
                    .db
                    .mark_page_failed(media_row.id, cid, page as i32, part)
                    .await;
                let _ = ctx.db.record_media_failure(media_row.id, &err_msg).await;
                return MediaOutcome::Failed(err_msg);
            }
        }
    }

    if let Err(e) = ctx
        .db
        .set_media_state(media_row.id, MediaState::Completed)
        .await
    {
        return MediaOutcome::Failed(format!(
            "Downloaded media<{}> but failed marking Completed: {e:#}",
            media_row.id
        ));
    }

    info!("Downloaded [{}] {}", media_row.bv_id, media_title);
    MediaOutcome::Downloaded
}

async fn handle_media_api_error(
    ctx: &AppContext,
    media_row: &media::Model,
    err: anyhow::Error,
    cancel: &CancellationToken,
    remaining_account_ids: &[i64],
) -> MediaOutcome {
    if let Some(api_err) = err.downcast_ref::<BiliApiError>()
        && api_err.is_auth_expired()
    {
        // Restore media to Pending — account auth expiry is not a media failure.
        let _ = ctx
            .db
            .set_media_state(media_row.id, MediaState::Pending)
            .await;
        return MediaOutcome::AccountExpired(api_err.to_string());
    }
    if cancel.is_cancelled() {
        let _ = ctx
            .db
            .set_media_state(media_row.id, MediaState::Pending)
            .await;
        return MediaOutcome::Cancelled;
    }
    if let Some(api_err) = err.downcast_ref::<BiliApiError>() {
        if api_err.is_media_expired() {
            let _ = ctx
                .db
                .set_media_state(media_row.id, MediaState::Expired)
                .await;
            return MediaOutcome::Failed(format!(
                "Media<{} {}> marked Expired: {api_err}",
                media_row.id, media_row.title
            ));
        }
        if api_err.is_permission_denied() {
            if ctx
                .db
                .is_media_active_for_any_account(media_row.id, remaining_account_ids)
                .await
                .unwrap_or(false)
            {
                let _ = ctx
                    .db
                    .set_media_state(media_row.id, MediaState::Pending)
                    .await;
                warn!(
                    "Media<{} {}> returned permission denied ({api_err}); deferring to another active account.",
                    media_row.id, media_row.title
                );
                return MediaOutcome::DeferredToNextAccount;
            }
            let _ = ctx
                .db
                .set_media_state(media_row.id, MediaState::PermissionDenied)
                .await;
            return MediaOutcome::Failed(format!(
                "Media<{} {}> marked PermissionDenied: {api_err}",
                media_row.id, media_row.title
            ));
        }
    }

    // Transient errors (-412, -352, -799, 5xx, network) MUST NOT mark media Expired!
    let msg = format!(
        "Transient/download error for media<{} {}>: {err:#}",
        media_row.id, media_row.title
    );
    let _ = ctx.db.record_media_failure(media_row.id, &msg).await;
    MediaOutcome::Failed(msg)
}

async fn query_pull_candidates(
    ctx: &AppContext,
    opts: &PullOptions,
    target_aids: &[i64],
    account_id: Option<i64>,
    limit: Option<usize>,
) -> Result<Vec<media::Model>> {
    ctx.db
        .pull_candidates(&PullFilter {
            ids: target_aids.to_vec(),
            set_ids: opts.set_ids.clone(),
            up_ids: opts.up_ids.clone(),
            account_id,
            retry_failed: opts.retry_failed,
            force: opts.force,
            limit: limit.map(|l| l as u64),
        })
        .await
}

async fn run_pull_dry_run(
    ctx: &AppContext,
    opts: &PullOptions,
    target_aids: &[i64],
    output_dir: &Path,
    sound_only: bool,
    audio_format: AudioFormat,
    recheck_report: Option<&ReconcileReport>,
) -> AppResult<()> {
    let mut candidates = query_pull_candidates(ctx, opts, target_aids, None, opts.limit).await?;
    let mut existing_ids: BTreeSet<i64> = candidates.iter().map(|m| m.id).collect();

    if let Some(rep) = recheck_report {
        eprintln!(
            "[dry-run] Recheck: {} relocated file(s) (would relink), {} missing page(s) and {} missing legacy media(s) across {} media(s) (would reset & re-download)",
            rep.relocated.len(),
            rep.missing_pages.len(),
            rep.missing_legacy_ids.len(),
            rep.affected_missing_media_ids().len(),
        );
        for missing_id in rep.affected_missing_media_ids() {
            if existing_ids.insert(missing_id)
                && let Ok(row) = ctx.db.get_media(missing_id).await
            {
                candidates.push(row);
            }
        }
        candidates.sort_by_key(|m| m.id);
        if let Some(limit) = opts.limit {
            candidates.truncate(limit);
        }
    }

    let ext = preview_extension(sound_only, audio_format);
    eprintln!(
        "[dry-run] Output directory: {} ({} tracked candidate(s))",
        output_dir.display(),
        candidates.len()
    );
    for m in &candidates {
        let preview = build_safe_filename(m.id, &m.title, 1, "", true, ext);
        if !write_stdout_line(&format!(
            "{}\t{}\t{}\t{}",
            m.id,
            m.bv_id,
            m.state,
            output_dir.join(preview).display()
        ))
        .map_err(AppError::runtime)?
        {
            return Ok(());
        }
    }
    Ok(())
}

fn resolve_target_entries(targets: &[String]) -> AppResult<Vec<(String, i64)>> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::with_capacity(targets.len());
    let mut seen = HashSet::new();
    for raw in targets {
        let ident = parse_media_ident(raw).map_err(AppError::usage)?;
        let aid = ident.to_aid().ok_or_else(|| {
            AppError::usage(anyhow!("Cannot convert identifier '{ident}' to AID"))
        })?;
        if seen.insert(aid) {
            entries.push((raw.clone(), aid));
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::BiliClientConfig,
        config::{Config, Workspace},
        db::Db,
        entity::{account, media_set, set, set_account, up, up_account},
        error::ExitKind,
        muxer::test_support::MockMuxer,
        state::SetKind,
    };

    async fn make_ctx() -> (tempfile::TempDir, AppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();
        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            BiliClientConfig::default(),
        );
        (tmp, ctx)
    }

    #[tokio::test]
    async fn pull_strictly_rejects_untracked_and_inactive_targets_sets_and_ups() {
        let (_tmp, ctx) = make_ctx().await;

        ctx.db
            .upsert_account(account::Model {
                account_id: 1,
                name: "Alice".to_owned(),
                cookies: "SESSDATA=s".to_owned(),
                state: AccountState::Active.to_string(),
                refresh_token: None,
            })
            .await
            .unwrap();
        ctx.db
            .upsert_account(account::Model {
                account_id: 2,
                name: "Bob".to_owned(),
                cookies: "SESSDATA=b".to_owned(),
                state: AccountState::Inactive.to_string(),
                refresh_token: None,
            })
            .await
            .unwrap();
        ctx.db
            .upsert_sets([
                set::Model {
                    set_id: 10,
                    name: "InactiveSet".to_owned(),
                    count: 1,
                    state: SetState::Inactive.to_string(),
                    kind: SetKind::Created.to_string(),
                    owner_mid: Some(1),
                    sync_complete: false,
                    resume_page: None,
                },
                set::Model {
                    set_id: 20,
                    name: "BobActiveSet".to_owned(),
                    count: 1,
                    state: SetState::Active.to_string(),
                    kind: SetKind::Created.to_string(),
                    owner_mid: Some(2),
                    sync_complete: false,
                    resume_page: None,
                },
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_set_accounts([
                set_account::Model {
                    set_id: 10,
                    account_id: 1,
                },
                set_account::Model {
                    set_id: 20,
                    account_id: 2,
                },
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_ups([up::Model {
                up_id: 30,
                name: "InactiveUp".to_owned(),
                state: UpState::Inactive.to_string(),
                sync_complete: false,
                resume_page: None,
            }])
            .await
            .unwrap();
        ctx.db
            .upsert_up_accounts([up_account::Model {
                up_id: 30,
                account_id: 1,
            }])
            .await
            .unwrap();
        ctx.db
            .upsert_medias([media::Model {
                id: 170_001,
                bv_id: "BV17x411w7KC".to_owned(),
                title: "In Inactive Set".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            }])
            .await
            .unwrap();
        ctx.db
            .upsert_media_sets([media_set::Model {
                id: 170_001,
                set_id: 10,
            }])
            .await
            .unwrap();

        // Untracked target rejected with Usage + `fav download` hint, 0 rows inserted!
        let err = pull(
            &ctx,
            PullOptions {
                targets: vec!["BV1Q541167Qg".to_owned()],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
        assert!(err.to_string().contains("fav download"));
        assert!(ctx.db.get_media(455_017_605).await.is_err());

        // Tracked but inactive target rejected with Usage + `fav download` hint
        let err = pull(
            &ctx,
            PullOptions {
                targets: vec!["BV17x411w7KC".to_owned()],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
        assert!(err.to_string().contains("fav download"));

        // Inactive --set rejected
        let err = pull(
            &ctx,
            PullOptions {
                set_ids: vec![10],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);

        // Active --set owned only by Inactive account rejected
        let err = pull(
            &ctx,
            PullOptions {
                set_ids: vec![20],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);

        // Inactive --up rejected
        let err = pull(
            &ctx,
            PullOptions {
                up_ids: vec![30],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
    }

    #[tokio::test]
    async fn pull_recheck_dry_run_previews_without_mutating_db() {
        let (tmp, ctx) = make_ctx().await;
        let out_dir = tmp.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();

        ctx.db
            .upsert_account(account::Model {
                account_id: 1,
                name: "Alice".to_owned(),
                cookies: "SESSDATA=s".to_owned(),
                state: AccountState::Active.to_string(),
                refresh_token: None,
            })
            .await
            .unwrap();
        ctx.db
            .upsert_sets([set::Model {
                set_id: 10,
                name: "ActiveSet".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            }])
            .await
            .unwrap();
        ctx.db
            .upsert_set_accounts([set_account::Model {
                set_id: 10,
                account_id: 1,
            }])
            .await
            .unwrap();
        ctx.db
            .upsert_medias([media::Model {
                id: 170_001,
                bv_id: "BV17x411w7KC".to_owned(),
                title: "Deleted On Disk".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            }])
            .await
            .unwrap();
        ctx.db
            .upsert_media_sets([media_set::Model {
                id: 170_001,
                set_id: 10,
            }])
            .await
            .unwrap();
        ctx.db
            .mark_page_completed(
                170_001,
                279_786,
                1,
                "P1".to_owned(),
                "170001-Deleted On Disk.mp4".to_owned(),
                Some(123),
                None,
            )
            .await
            .unwrap();

        // Dry run with --recheck + --sound-only must NOT mutate Completed state in DB.
        pull(
            &ctx,
            PullOptions {
                recheck: true,
                dry_run: true,
                sound_only: true,
                audio_format: Some(AudioFormat::Mp3),
                output_dir: Some(out_dir),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            ctx.db.get_media(170_001).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            ctx.db.pages_for_media(170_001).await.unwrap()[0].state,
            MediaPageState::Completed.as_str()
        );
    }

    #[tokio::test]
    async fn rollback_inflight_downloading_resets_only_downloading_rows() {
        let (_tmp, ctx) = make_ctx().await;

        ctx.db
            .upsert_medias([
                media::Model {
                    id: 1,
                    bv_id: "BV1dl000000001".to_owned(),
                    title: "InFlightDownloading".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Downloading.to_string(),
                    retry_count: 0,
                    last_error: Some("stale".to_owned()),
                    updated_at: None,
                },
                media::Model {
                    id: 2,
                    bv_id: "BV1dl000000002".to_owned(),
                    title: "AlreadyCompleted".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Completed.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
                media::Model {
                    id: 3,
                    bv_id: "BV1dl000000003".to_owned(),
                    title: "AlreadyFailed".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Failed.to_string(),
                    retry_count: 2,
                    last_error: Some("network timeout".to_owned()),
                    updated_at: None,
                },
                media::Model {
                    id: 4,
                    bv_id: "BV1dl000000004".to_owned(),
                    title: "AlreadyExpired".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Expired.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
                media::Model {
                    id: 5,
                    bv_id: "BV1dl000000005".to_owned(),
                    title: "NotInFlightDownloading".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Downloading.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
            ])
            .await
            .unwrap();

        let in_flight = Mutex::new(HashSet::from([1, 2, 3, 4]));
        let reset = rollback_inflight_downloading(&ctx.db, &in_flight)
            .await
            .unwrap();
        assert_eq!(reset, 1);
        assert!(in_flight.lock().unwrap().is_empty());

        let m1 = ctx.db.get_media(1).await.unwrap();
        assert_eq!(m1.state, MediaState::Pending.as_str());
        assert_eq!(m1.last_error, None);

        let m2 = ctx.db.get_media(2).await.unwrap();
        assert_eq!(m2.state, MediaState::Completed.as_str());

        let m3 = ctx.db.get_media(3).await.unwrap();
        assert_eq!(m3.state, MediaState::Failed.as_str());
        assert_eq!(m3.retry_count, 2);
        assert_eq!(m3.last_error.as_deref(), Some("network timeout"));

        let m4 = ctx.db.get_media(4).await.unwrap();
        assert_eq!(m4.state, MediaState::Expired.as_str());

        let m5 = ctx.db.get_media(5).await.unwrap();
        assert_eq!(m5.state, MediaState::Downloading.as_str());
    }

    #[tokio::test]
    async fn compute_recheck_scope_intersects_targets_sets_and_ups() {
        let (_tmp, ctx) = make_ctx().await;

        ctx.db
            .upsert_account(account::Model {
                account_id: 1,
                name: "Alice".to_owned(),
                cookies: "SESSDATA=s".to_owned(),
                state: AccountState::Active.to_string(),
                refresh_token: None,
            })
            .await
            .unwrap();
        ctx.db
            .upsert_sets([
                set::Model {
                    set_id: 10,
                    name: "Set10".to_owned(),
                    count: 2,
                    state: SetState::Active.to_string(),
                    kind: SetKind::Created.to_string(),
                    owner_mid: Some(1),
                    sync_complete: true,
                    resume_page: None,
                },
                set::Model {
                    set_id: 20,
                    name: "Set20".to_owned(),
                    count: 1,
                    state: SetState::Active.to_string(),
                    kind: SetKind::Created.to_string(),
                    owner_mid: Some(1),
                    sync_complete: true,
                    resume_page: None,
                },
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_set_accounts([
                set_account::Model {
                    set_id: 10,
                    account_id: 1,
                },
                set_account::Model {
                    set_id: 20,
                    account_id: 1,
                },
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_ups([
                up::Model {
                    up_id: 100,
                    name: "Up100".to_owned(),
                    state: UpState::Active.to_string(),
                    sync_complete: true,
                    resume_page: None,
                },
                up::Model {
                    up_id: 200,
                    name: "Up200".to_owned(),
                    state: UpState::Active.to_string(),
                    sync_complete: true,
                    resume_page: None,
                },
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_up_accounts([
                up_account::Model {
                    up_id: 100,
                    account_id: 1,
                },
                up_account::Model {
                    up_id: 200,
                    account_id: 1,
                },
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_medias([
                media::Model {
                    id: 1,
                    bv_id: "BV1rc000000001".to_owned(),
                    title: "S10_U100".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Completed.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
                media::Model {
                    id: 2,
                    bv_id: "BV1rc000000002".to_owned(),
                    title: "S10_U200".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Completed.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
                media::Model {
                    id: 3,
                    bv_id: "BV1rc000000003".to_owned(),
                    title: "S20_U100".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Completed.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_media_sets([
                media_set::Model { id: 1, set_id: 10 },
                media_set::Model { id: 2, set_id: 10 },
                media_set::Model { id: 3, set_id: 20 },
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_media_ups([
                crate::entity::media_up::Model { id: 1, up_id: 100 },
                crate::entity::media_up::Model { id: 2, up_id: 200 },
                crate::entity::media_up::Model { id: 3, up_id: 100 },
            ])
            .await
            .unwrap();

        assert_eq!(
            compute_recheck_scope(&ctx, &PullOptions::default(), &[])
                .await
                .unwrap(),
            None
        );

        let scope = compute_recheck_scope(
            &ctx,
            &PullOptions {
                set_ids: vec![10, 20],
                up_ids: vec![100],
                ..Default::default()
            },
            &[1, 2],
        )
        .await
        .unwrap();
        assert_eq!(scope, Some(HashSet::from([1])));
    }

    async fn start_pull_mock_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        use tokio::{
            io::{AsyncReadExt as _, AsyncWriteExt as _},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let origin = format!("http://{addr}");

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let origin = origin.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let first_line = req.lines().next().unwrap_or_default();
                    let target = first_line.split_whitespace().nth(1).unwrap_or("/");
                    let (path, query) = target.split_once('?').unwrap_or((target, ""));
                    let cookie = req
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.eq_ignore_ascii_case("cookie").then(|| v.trim())
                        })
                        .unwrap_or_default();

                    let (content_type, body): (&str, Vec<u8>) = if path == "/x/frontend/finger/spi"
                    {
                        (
                            "application/json",
                            r#"{"code":0,"message":"0","data":{"b_3":"BUVID3","b_4":"BUVID4"}}"#
                                .as_bytes()
                                .to_vec(),
                        )
                    } else if path == "/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket" {
                        (
                            "application/json",
                            r#"{"code":0,"message":"0","data":{"ticket":"mock_ticket","created_at":1,"ttl":3600}}"#
                                .as_bytes()
                                .to_vec(),
                        )
                    } else if path == "/x/web-interface/nav" {
                        (
                            "application/json",
                            r#"{"code":0,"message":"0","data":{"isLogin":true,"uname":"Mock","mid":1,"wbi_img":{"img_url":"https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png","sub_url":"https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png"}}}"#
                                .as_bytes()
                                .to_vec(),
                        )
                    } else if path == "/x/web-interface/wbi/view" {
                        if cookie.contains("SESSDATA=sess_expired") {
                            (
                                "application/json",
                                r#"{"code":-101,"message":"not logged in","data":null}"#
                                    .as_bytes()
                                    .to_vec(),
                            )
                        } else {
                            let aid = query
                                .split('&')
                                .find_map(|kv| kv.strip_prefix("aid="))
                                .and_then(|v| v.parse::<i64>().ok())
                                .unwrap_or(1);
                            let cid = aid * 10;
                            let json = format!(
                                r#"{{"code":0,"message":"0","data":{{"aid":{aid},"bvid":"BV1ok000000{aid}","title":"Video{aid}","owner":{{"mid":99,"name":"Up"}},"pages":[{{"cid":{cid},"page":1,"part":"P1"}}]}}}}"#
                            );
                            ("application/json", json.into_bytes())
                        }
                    } else if path == "/x/player/wbi/playurl" {
                        let json = format!(
                            r#"{{"code":0,"message":"0","data":{{"dash":{{"video":[{{"id":80,"baseUrl":"{origin}/cdn/v.mp4","backupUrl":[],"mimeType":"video/mp4","codecs":"avc1.640032","bandwidth":1000000,"codecid":7}}],"audio":[{{"id":30280,"baseUrl":"{origin}/cdn/a.m4a","backupUrl":[],"mimeType":"audio/mp4","codecs":"mp4a.40.2","bandwidth":128000,"codecid":0}}]}}}}}}"#
                        );
                        ("application/json", json.into_bytes())
                    } else if path.starts_with("/cdn/") {
                        ("application/octet-stream", b"mock-stream-bytes".to_vec())
                    } else {
                        (
                            "application/json",
                            r#"{"code":-404,"message":"not found","data":null}"#
                                .as_bytes()
                                .to_vec(),
                        )
                    };

                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });

        (addr, handle)
    }

    fn mock_client_config(addr: std::net::SocketAddr) -> BiliClientConfig {
        let base = url::Url::parse(&format!("http://{addr}")).unwrap();
        BiliClientConfig {
            api_base_url: base.clone(),
            passport_base_url: base,
            max_retries: 0,
            base_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(5),
            min_request_interval: std::time::Duration::ZERO,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn pull_returns_auth_when_only_account_with_candidates_expires() {
        let (addr, server_handle) = start_pull_mock_server().await;
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        // Alice (Active, 0 pull candidates) and Bob (Active, 1 pull candidate, returns -101).
        db.upsert_account(account::Model {
            account_id: 1,
            name: "Alice".to_owned(),
            cookies: "SESSDATA=sess_valid; bili_jct=csrf_a".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 2,
            name: "Bob".to_owned(),
            cookies: "SESSDATA=sess_expired; bili_jct=csrf_b".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_sets([
            set::Model {
                set_id: 10,
                name: "AliceEmptySet".to_owned(),
                count: 0,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 20,
                name: "BobSet".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(2),
                sync_complete: true,
                resume_page: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_set_accounts([
            set_account::Model {
                set_id: 10,
                account_id: 1,
            },
            set_account::Model {
                set_id: 20,
                account_id: 2,
            },
        ])
        .await
        .unwrap();
        db.upsert_medias([media::Model {
            id: 501,
            bv_id: "BV1ex000000501".to_owned(),
            title: "BobOnly".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        }])
        .await
        .unwrap();
        db.upsert_media_sets([media_set::Model {
            id: 501,
            set_id: 20,
        }])
        .await
        .unwrap();

        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            mock_client_config(addr),
        );

        let err = pull(&ctx, PullOptions::default()).await.unwrap_err();
        assert_eq!(err.kind(), ExitKind::Auth);
        assert_eq!(
            ctx.db.get_account(1).await.unwrap().state,
            AccountState::Active.as_str()
        );
        assert_eq!(
            ctx.db.get_account(2).await.unwrap().state,
            AccountState::Expired.as_str()
        );
        assert_eq!(
            ctx.db.get_media(501).await.unwrap().state,
            MediaState::Pending.as_str()
        );

        server_handle.abort();
    }

    #[tokio::test]
    async fn pull_returns_partial_when_expired_account_leaves_unfinished_items() {
        let (addr, server_handle) = start_pull_mock_server().await;
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        // Alice (Active, exclusive media 601, returns -101) and Bob (Active, exclusive media 602, succeeds).
        db.upsert_account(account::Model {
            account_id: 1,
            name: "Alice".to_owned(),
            cookies: "SESSDATA=sess_expired; bili_jct=csrf_a".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 2,
            name: "Bob".to_owned(),
            cookies: "SESSDATA=sess_valid; bili_jct=csrf_b".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_sets([
            set::Model {
                set_id: 10,
                name: "AliceSet".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 20,
                name: "BobSet".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(2),
                sync_complete: true,
                resume_page: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_set_accounts([
            set_account::Model {
                set_id: 10,
                account_id: 1,
            },
            set_account::Model {
                set_id: 20,
                account_id: 2,
            },
        ])
        .await
        .unwrap();
        db.upsert_medias([
            media::Model {
                id: 601,
                bv_id: "BV1ex000000601".to_owned(),
                title: "AliceExclusive".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 602,
                bv_id: "BV1ok000000602".to_owned(),
                title: "BobExclusive".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model {
                id: 601,
                set_id: 10,
            },
            media_set::Model {
                id: 602,
                set_id: 20,
            },
        ])
        .await
        .unwrap();

        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            mock_client_config(addr),
        );

        let err = pull(
            &ctx,
            PullOptions {
                output_dir: Some(out_dir),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::PartialFailure);
        assert_eq!(
            ctx.db.get_account(1).await.unwrap().state,
            AccountState::Expired.as_str()
        );
        assert_eq!(
            ctx.db.get_media(601).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        assert_eq!(
            ctx.db.get_media(602).await.unwrap().state,
            MediaState::Completed.as_str()
        );

        server_handle.abort();
    }

    #[tokio::test]
    async fn handle_media_api_error_cancels_cleanly_after_auth_check() {
        let (_tmp, ctx) = make_ctx().await;
        let row = media::Model {
            id: 701,
            bv_id: "BV1cn000000701".to_owned(),
            title: "CancelledSibling".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Downloading.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        };
        ctx.db.upsert_medias([row.clone()]).await.unwrap();

        let cancelled = CancellationToken::new();
        cancelled.cancel();

        // Non-auth API error while cancelled must restore Pending and return Cancelled.
        let outcome = handle_media_api_error(
            &ctx,
            &row,
            anyhow::Error::new(BiliApiError::new(-404, Some("aborted".into()), "view")),
            &cancelled,
            &[],
        )
        .await;
        assert!(matches!(outcome, MediaOutcome::Cancelled));
        assert_eq!(
            ctx.db.get_media(701).await.unwrap().state,
            MediaState::Pending.as_str()
        );

        // Auth-expired (-101) takes precedence over cancellation so account expiry is still recorded.
        ctx.db
            .set_media_state(701, MediaState::Downloading)
            .await
            .unwrap();
        let outcome = handle_media_api_error(
            &ctx,
            &row,
            anyhow::Error::new(BiliApiError::new(-101, Some("expired".into()), "view")),
            &cancelled,
            &[],
        )
        .await;
        assert!(matches!(outcome, MediaOutcome::AccountExpired(_)));
        assert_eq!(
            ctx.db.get_media(701).await.unwrap().state,
            MediaState::Pending.as_str()
        );
    }
}
