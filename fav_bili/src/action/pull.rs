use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr as _,
    sync::Arc,
};

use anyhow::{Context as _, Result, anyhow};
use futures::StreamExt as _;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

pub use crate::downloader::build_safe_filename;
use crate::{
    api::{BiliApiError, BiliClient},
    context::AppContext,
    db::PullFilter,
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
    let mut expired_accounts = 0usize;
    let mut attempted_failed_ids: HashSet<i64> = HashSet::new();
    let mut interrupted = false;

    'accounts: for account in &accounts {
        if global_cancel.is_cancelled() {
            interrupted = true;
            break 'accounts;
        }

        // Re-query candidates on every account iteration so already-completed/expired items
        // from earlier accounts are not retried.
        let candidates = query_pull_candidates(ctx, &opts, &target_aids).await?;
        let candidates: Vec<media::Model> = candidates
            .into_iter()
            .filter(|m| !attempted_failed_ids.contains(&m.id))
            .collect();
        if candidates.is_empty() {
            break 'accounts;
        }

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

        let mut tasks = futures::stream::iter(candidates)
            .map(|media_row| {
                let client = client.clone();
                let token = account_cancel.clone();
                let output_dir = output_dir_arc.clone();
                let tmp_dir = tmp_dir_arc.clone();
                let params = stream_params.clone();
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
                        MediaOutcome::Downloaded => downloaded_total += 1,
                        MediaOutcome::Skipped => skipped_total += 1,
                        MediaOutcome::Failed(msg) => {
                            failed_total += 1;
                            attempted_failed_ids.insert(media_id);
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
                    break 'accounts;
                }
            }
        }
    }

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
        if expired_accounts == accounts.len() && downloaded_total == 0 {
            return Err(AppError::auth(anyhow!(
                "All active accounts expired during pull."
            )));
        }
        return Err(AppError::partial(anyhow!(
            "Pull finished with {failed_total} failure(s) (Downloaded {downloaded_total}, Skipped {skipped_total})."
        )));
    }

    if expired_accounts == accounts.len() && downloaded_total == 0 && skipped_total == 0 {
        return Err(AppError::auth(anyhow!(
            "All active accounts expired during pull."
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
    if !target_aids.is_empty() {
        return Ok(Some(target_aids.iter().copied().collect()));
    }
    if opts.set_ids.is_empty() && opts.up_ids.is_empty() {
        return Ok(None);
    }
    let scope_opts = PullOptions {
        set_ids: opts.set_ids.clone(),
        up_ids: opts.up_ids.clone(),
        force: true,
        limit: None,
        ..Default::default()
    };
    let rows = query_pull_candidates(ctx, &scope_opts, &[]).await?;
    let ids: HashSet<i64> = rows
        .into_iter()
        .filter(|m| m.state == MediaState::Completed.as_str())
        .map(|m| m.id)
        .collect();
    Ok(Some(ids))
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
            return handle_media_api_error(ctx, media_row, e).await;
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
                let _ = ctx
                    .db
                    .mark_page_failed(media_row.id, cid, page as i32, part.clone())
                    .await;
                return handle_media_api_error(ctx, media_row, e).await;
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
) -> MediaOutcome {
    if let Some(api_err) = err.downcast_ref::<BiliApiError>() {
        if api_err.is_auth_expired() {
            // Restore media to Pending — account auth expiry is not a media failure.
            let _ = ctx
                .db
                .set_media_state(media_row.id, MediaState::Pending)
                .await;
            return MediaOutcome::AccountExpired(api_err.to_string());
        }
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
) -> Result<Vec<media::Model>> {
    let limit_u64 = opts.limit.map(|l| l as u64);

    if !target_aids.is_empty() || (opts.set_ids.len() <= 1 && opts.up_ids.len() <= 1) {
        return ctx
            .db
            .pull_candidates(&PullFilter {
                ids: target_aids.to_vec(),
                set_id: opts.set_ids.first().copied(),
                up_id: opts.up_ids.first().copied(),
                retry_failed: opts.retry_failed,
                force: opts.force,
                limit: limit_u64,
            })
            .await;
    }

    // Multiple `--set` and/or `--up` filters: union candidates and deduplicate by media ID.
    let mut seen = HashSet::new();
    let mut combined = Vec::new();
    for &set_id in &opts.set_ids {
        let rows = ctx
            .db
            .pull_candidates(&PullFilter {
                set_id: Some(set_id),
                retry_failed: opts.retry_failed,
                force: opts.force,
                ..Default::default()
            })
            .await?;
        for row in rows {
            if seen.insert(row.id) {
                combined.push(row);
            }
        }
    }
    for &up_id in &opts.up_ids {
        let rows = ctx
            .db
            .pull_candidates(&PullFilter {
                up_id: Some(up_id),
                retry_failed: opts.retry_failed,
                force: opts.force,
                ..Default::default()
            })
            .await?;
        for row in rows {
            if seen.insert(row.id) {
                combined.push(row);
            }
        }
    }
    combined.sort_by_key(|m| m.id);
    if let Some(limit) = opts.limit {
        combined.truncate(limit);
    }
    Ok(combined)
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
    let mut candidates = query_pull_candidates(ctx, opts, target_aids).await?;
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
}
