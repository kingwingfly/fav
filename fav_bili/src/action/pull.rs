use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr as _,
    sync::Arc,
};

use anyhow::{Context as _, Result, anyhow};
use futures::StreamExt as _;
use indicatif::{ProgressBar, ProgressStyle};
use sanitize_filename::{Options as SanitizeOptions, sanitize_with_options};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::{
    api::{BiliApiError, BiliClient, DownloadOptions, DownloadProgress},
    context::AppContext,
    db::PullFilter,
    entity::{media, media_page, media_up, up},
    error::{AppError, AppResult},
    id_parse::parse_media_idents,
    muxer::MuxRequest,
    response::{Page, select_best_audio, select_best_video},
    state::{AccountState, MediaPageState, MediaState, MediaType, UpState},
    table::{head, write_stdout_line},
};

const BAR_TEMPLATE: &str = "{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})";
const SPINNER_TEMPLATE: &str =
    "{msg} {spinner:.green} [{elapsed_precise}] {bytes} ({bytes_per_sec})";
const MAX_FILENAME_BYTES: usize = 240;

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
}

enum MediaOutcome {
    Downloaded,
    Skipped,
    Failed(String),
    AccountExpired(String),
    Cancelled,
}

pub async fn pull(ctx: &AppContext, opts: PullOptions) -> AppResult<()> {
    let target_aids = resolve_target_aids(&opts.targets)?;
    let output_dir = ctx.effective_output_dir(opts.output_dir.as_deref());

    if opts.dry_run {
        return run_pull_dry_run(ctx, &opts, &target_aids, &output_dir).await;
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
    let tmp_dir = ctx.workspace.tmp_dir();
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .with_context(|| format!("Failed to create temp directory '{}'", tmp_dir.display()))?;

    // If the user explicitly targeted BVID/AIDs not yet in DB, fetch metadata on demand.
    if !target_aids.is_empty() {
        let first_client = ctx.client_for_account(&accounts[0].cookies)?;
        ensure_targets_in_db(ctx, &first_client, &target_aids).await?;
    }

    let jobs = ctx.effective_jobs(opts.jobs);
    let limit_rate = ctx.effective_limit_rate(opts.limit_rate);
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
                        limit_rate,
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

#[allow(clippy::too_many_arguments)]
async fn download_media(
    ctx: &AppContext,
    client: &BiliClient,
    media_row: &media::Model,
    output_dir: &Path,
    tmp_dir: &Path,
    limit_rate: Option<u64>,
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

        let dash_data = match client.dash_playurl(media_row.id, cid).await {
            Ok(d) => d,
            Err(e) => {
                let _ = ctx
                    .db
                    .mark_page_failed(media_row.id, cid, page as i32, part.clone())
                    .await;
                return handle_media_api_error(ctx, media_row, e).await;
            }
        };

        let label = if only1p {
            media_title.to_string()
        } else if part.trim().is_empty() {
            format!("{media_title} (P{page})")
        } else {
            format!("{media_title} (P{page}: {part})")
        };

        let page_res = download_and_mux_page(
            ctx,
            client,
            media_row,
            media_title,
            cid,
            page,
            &part,
            only1p,
            &label,
            dash_data,
            output_dir,
            tmp_dir,
            limit_rate,
            cancel.clone(),
        )
        .await;

        match page_res {
            Ok(saved_name) => {
                if let Err(e) = ctx
                    .db
                    .mark_page_completed(media_row.id, cid, page as i32, part, saved_name)
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

#[allow(clippy::too_many_arguments)]
async fn download_and_mux_page(
    ctx: &AppContext,
    client: &BiliClient,
    media_row: &media::Model,
    media_title: &str,
    cid: i64,
    page: i64,
    part: &str,
    only1p: bool,
    label: &str,
    dash_data: crate::response::DashData,
    output_dir: &Path,
    tmp_dir: &Path,
    limit_rate: Option<u64>,
    cancel: CancellationToken,
) -> Result<String> {
    let referer = format!("https://www.bilibili.com/video/{}/", media_row.bv_id);
    let dl_opts = DownloadOptions {
        limit_rate,
        referer: Some(referer),
        cancel: Some(cancel),
    };

    if let Some(dash) = dash_data.dash {
        let best_video = select_best_video(&dash.video, None, None).cloned();
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
            .map(|v| tmp_dir.join(format!("{}_{}_v_{}.part", media_row.id, cid, v.id)));
        let a_part = best_audio
            .as_ref()
            .map(|a| tmp_dir.join(format!("{}_{}_a_{}.part", media_row.id, cid, a.id)));

        let pb = make_progress_bar(ctx, label);

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
                let v_total = Arc::new(std::sync::atomic::AtomicU64::new(0));
                let a_total = Arc::new(std::sync::atomic::AtomicU64::new(0));
                let v_done = Arc::new(std::sync::atomic::AtomicU64::new(0));
                let a_done = Arc::new(std::sync::atomic::AtomicU64::new(0));

                let vt1 = v_total.clone();
                let at1 = a_total.clone();
                let vd1 = v_done.clone();
                let ad1 = a_done.clone();
                let v_fut = client.download_to_part(&v_urls, vp, &dl_opts, move |p| {
                    vd1.store(p.downloaded, std::sync::atomic::Ordering::Relaxed);
                    if let Some(t) = p.total {
                        vt1.store(t, std::sync::atomic::Ordering::Relaxed);
                    }
                    update_combined_pb(&pb_v, &vt1, &at1, &vd1, &ad1);
                });

                let vt2 = v_total.clone();
                let at2 = a_total.clone();
                let vd2 = v_done.clone();
                let ad2 = a_done.clone();
                let a_fut = client.download_to_part(&a_urls, ap, &dl_opts, move |p| {
                    ad2.store(p.downloaded, std::sync::atomic::Ordering::Relaxed);
                    if let Some(t) = p.total {
                        at2.store(t, std::sync::atomic::Ordering::Relaxed);
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

        let file_name = build_safe_filename(media_row.id, media_title, page, part, only1p, ext);
        let output_path = output_dir.join(&file_name);

        ctx.muxer
            .mux(MuxRequest {
                video: v_part.clone(),
                audio: a_part.clone(),
                output: output_path,
            })
            .await?;

        // Clean up staging stream `.part` files only after atomic mux succeeds.
        if let Some(vp) = v_part {
            let _ = tokio::fs::remove_file(vp).await;
        }
        if let Some(ap) = a_part {
            let _ = tokio::fs::remove_file(ap).await;
        }

        return Ok(file_name);
    }

    // Legacy non-DASH `durl` fallback.
    if let Some(durl) = dash_data.durl.first() {
        let part_path = tmp_dir.join(format!("{}_{}_durl.part", media_row.id, cid));
        let pb = make_progress_bar(ctx, label);
        let pb_c = pb.clone();
        let res = client
            .download_to_part(&durl.all_urls(), &part_path, &dl_opts, move |p| {
                update_single_pb(&pb_c, p);
            })
            .await;
        finish_pb(pb);
        res?;

        let file_name = build_safe_filename(media_row.id, media_title, page, part, only1p, "mp4");
        let output_path = output_dir.join(&file_name);
        ctx.muxer
            .mux(MuxRequest {
                video: Some(part_path.clone()),
                audio: None,
                output: output_path,
            })
            .await?;
        let _ = tokio::fs::remove_file(part_path).await;
        return Ok(file_name);
    }

    Err(anyhow!(
        "Playurl response contained neither DASH nor durl streams"
    ))
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

async fn ensure_targets_in_db(
    ctx: &AppContext,
    client: &BiliClient,
    target_aids: &[i64],
) -> AppResult<()> {
    for &aid in target_aids {
        if ctx.db.get_media(aid).await.is_ok() {
            continue;
        }
        info!("Fetching metadata on demand for targeted media<{aid}>");
        let info = client
            .media_info(aid)
            .await
            .with_context(|| format!("Failed to fetch metadata for targeted media<{aid}>"))?;
        let bv_id = if info.bvid.trim().is_empty() {
            format!("av{aid}")
        } else {
            info.bvid.clone()
        };
        let title = if info.title.trim().is_empty() {
            bv_id.clone()
        } else {
            info.title.clone()
        };
        ctx.db
            .upsert_medias([media::Model {
                id: aid,
                bv_id,
                title,
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            }])
            .await?;
        if info.owner.mid > 0 {
            ctx.db
                .upsert_ups([up::Model {
                    up_id: info.owner.mid,
                    name: info.owner.name.clone(),
                    state: UpState::Inactive.to_string(),
                }])
                .await?;
            ctx.db
                .upsert_media_ups([media_up::Model {
                    id: aid,
                    up_id: info.owner.mid,
                }])
                .await?;
        }
    }
    Ok(())
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
) -> AppResult<()> {
    let candidates = query_pull_candidates(ctx, opts, target_aids).await?;
    let mut known_ids: HashSet<i64> = HashSet::new();
    eprintln!(
        "[dry-run] Output directory: {} ({} tracked candidate(s))",
        output_dir.display(),
        candidates.len()
    );
    for m in &candidates {
        known_ids.insert(m.id);
        let preview = build_safe_filename(m.id, &m.title, 1, "", true, "mp4");
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
    for &aid in target_aids {
        if !known_ids.contains(&aid)
            && !write_stdout_line(&format!(
                "{aid}\t(unfetched)\tPending\t{}",
                output_dir.display()
            ))
            .map_err(AppError::runtime)?
        {
            return Ok(());
        }
    }
    Ok(())
}

fn resolve_target_aids(targets: &[String]) -> AppResult<Vec<i64>> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let idents = parse_media_idents(targets).map_err(AppError::usage)?;
    let mut aids = Vec::with_capacity(idents.len());
    let mut seen = HashSet::new();
    for ident in idents {
        let aid = ident.to_aid().ok_or_else(|| {
            AppError::usage(anyhow!("Cannot convert identifier '{ident}' to AID"))
        })?;
        if seen.insert(aid) {
            aids.push(aid);
        }
    }
    Ok(aids)
}

fn resolve_recorded_path(output_dir: &Path, recorded: &str) -> PathBuf {
    let p = Path::new(recorded);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        output_dir.join(p)
    }
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

fn make_progress_bar(ctx: &AppContext, label: &str) -> Option<ProgressBar> {
    if !ctx.progress_enabled() {
        return None;
    }
    let mp = ctx.multi_progress.as_ref()?;
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_message(head(label, 24));
    if let Ok(style) = ProgressStyle::with_template(SPINNER_TEMPLATE) {
        pb.set_style(style);
    }
    Some(pb)
}

fn update_single_pb(pb: &Option<ProgressBar>, p: DownloadProgress) {
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

fn update_combined_pb(
    pb: &Option<ProgressBar>,
    v_total: &std::sync::atomic::AtomicU64,
    a_total: &std::sync::atomic::AtomicU64,
    v_done: &std::sync::atomic::AtomicU64,
    a_done: &std::sync::atomic::AtomicU64,
) {
    let Some(pb) = pb else {
        return;
    };
    let vt = v_total.load(std::sync::atomic::Ordering::Relaxed);
    let at = a_total.load(std::sync::atomic::Ordering::Relaxed);
    if vt > 0 && at > 0 {
        let total = vt + at;
        if pb.length() != Some(total) {
            pb.set_length(total);
            if let Ok(style) = ProgressStyle::with_template(BAR_TEMPLATE) {
                pb.set_style(style.progress_chars("#>-"));
            }
        }
    }
    let pos = v_done.load(std::sync::atomic::Ordering::Relaxed)
        + a_done.load(std::sync::atomic::Ordering::Relaxed);
    pb.set_position(pos);
}

fn finish_pb(pb: Option<ProgressBar>) {
    if let Some(pb) = pb {
        pb.finish_and_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
