use std::collections::{HashMap, HashSet};

use anyhow::anyhow;
use futures::StreamExt as _;
use indicatif::{ProgressBar, ProgressStyle};
use tracing::{debug, error, info, warn};

use crate::{
    api::{BiliApiError, BiliClient},
    context::AppContext,
    db::MediaFilter,
    entity::{media, media_set, media_up, set, up},
    error::{AppError, AppResult},
    response::Up,
    state::{AccountState, MediaState, SetState, UpState},
};

const FETCH_BAR_TEMPLATE: &str = "{msg} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len}";

#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    pub sets_only: bool,
    pub ups_only: bool,
    pub set_ids: Vec<i64>,
    pub up_ids: Vec<i64>,
    pub full: bool,
    pub prune: bool,
    pub purge_completed: bool,
    pub dry_run: bool,
    pub ignore_errors: bool,
}

#[derive(Default)]
struct FetchSummary {
    sets_synced: usize,
    ups_synced: usize,
    medias_upserted: usize,
    metadata_backfilled: usize,
    errors: Vec<String>,
    auth_expired_accounts: usize,
}

pub async fn fetch(ctx: &AppContext, mut opts: FetchOptions) -> AppResult<()> {
    opts.prune = effective_prune(&opts, &ctx.config);
    let accounts = ctx.db.active_accounts().await?;
    if accounts.is_empty() {
        return Err(AppError::usage(anyhow!(
            "No active accounts. Run `fav auth login` or `fav activate account` first."
        )));
    }

    let (do_sets, do_ups) = resolve_scope(&opts);

    if opts.dry_run {
        return run_fetch_dry_run(ctx, &opts, accounts.len(), do_sets, do_ups).await;
    }

    let api_concurrency = ctx.effective_api_concurrency();
    let mut summary = FetchSummary::default();
    let mut clients: Vec<(i64, String, BiliClient)> = Vec::with_capacity(accounts.len());
    let mut expired_accounts: HashSet<i64> = HashSet::new();

    // Build isolated per-account clients and ensure anti-bot cookies once per run.
    for acct in &accounts {
        let client = ctx.client_for_account(&acct.cookies)?;
        match client.ensure_anti_bot_cookies().await {
            Ok(true) => {
                let updated = client.export_cookies();
                if let Err(e) = ctx
                    .db
                    .update_account_cookies(acct.account_id, updated, None)
                    .await
                {
                    warn!(
                        "Failed persisting refreshed anti-bot cookies for {}: {e:#}",
                        acct.name
                    );
                }
            }
            Ok(false) => {}
            Err(e) => {
                debug!(
                    "Could not bootstrap anti-bot cookies for {}: {e:#}",
                    acct.name
                );
            }
        }
        clients.push((acct.account_id, acct.name.clone(), client));
    }

    // Phase 1: Sync account favorite folders and followed UPs (catalog discovery).
    let sync_catalog = opts.set_ids.is_empty() && opts.up_ids.is_empty();
    if sync_catalog {
        for (account_id, account_name, client) in &clients {
            if expired_accounts.contains(account_id) {
                continue;
            }
            if do_sets {
                info!("Fetching favorite sets for account<{account_name}>");
                match client.list_created_sets(*account_id).await {
                    Ok(list) => {
                        let models: Vec<set::Model> = list
                            .iter()
                            .map(|s| set::Model {
                                set_id: s.id,
                                name: s.title.clone(),
                                count: s.media_count,
                                state: SetState::Inactive.to_string(),
                            })
                            .collect();
                        summary.sets_synced += models.len();
                        let unlinked = ctx.db.sync_account_sets(*account_id, models).await?;
                        for set_id in unlinked {
                            warn!("Unlinked account<{account_name}> and set<{set_id}>");
                        }
                    }
                    Err(e) => {
                        record_api_error(
                            ctx,
                            *account_id,
                            &format!("fetch sets for {account_name}"),
                            &e,
                            &mut expired_accounts,
                            &mut summary,
                        )
                        .await;
                        if expired_accounts.contains(account_id) {
                            continue;
                        }
                    }
                }
            }

            if do_ups {
                info!("Fetching followed UPs for account<{account_name}>");
                match client.following_count(*account_id).await {
                    Ok(0) => {
                        // Explicitly unlink all stale UPs when account now follows 0 UPs.
                        let unlinked = ctx.db.sync_account_ups(*account_id, vec![], true).await?;
                        for up_id in unlinked {
                            warn!("Unlinked account<{account_name}> and up<{up_id}>");
                        }
                    }
                    Ok(following) => {
                        let total_pages = (following - 1) / 50 + 1;
                        let mut tasks = futures::stream::iter(1..=total_pages)
                            .map(|pn| {
                                let client = client.clone();
                                let account_id = *account_id;
                                async move {
                                    client
                                        .following_ups(account_id, pn, 50)
                                        .await
                                        .map(|d| (pn, d.list))
                                }
                            })
                            .buffer_unordered(api_concurrency);

                        let mut had_page_error = false;
                        let mut ups_by_id: HashMap<i64, up::Model> = HashMap::new();

                        while let Some(res) = tasks.next().await {
                            match res {
                                Ok((_, list)) => {
                                    for u in list {
                                        ups_by_id.insert(
                                            u.mid,
                                            up::Model {
                                                up_id: u.mid,
                                                name: u.name,
                                                state: UpState::Inactive.to_string(),
                                            },
                                        );
                                    }
                                }
                                Err(e) => {
                                    had_page_error = true;
                                    record_api_error(
                                        ctx,
                                        *account_id,
                                        &format!("fetch following UPs for {account_name}"),
                                        &e,
                                        &mut expired_accounts,
                                        &mut summary,
                                    )
                                    .await;
                                }
                            }
                        }

                        if had_page_error {
                            warn!(
                                "Skipping stale UP unlinking for account<{account_name}> because one or more pagination pages failed."
                            );
                        }
                        let up_models: Vec<up::Model> = ups_by_id.into_values().collect();
                        summary.ups_synced += up_models.len();
                        let unlinked = ctx
                            .db
                            .sync_account_ups(*account_id, up_models, !had_page_error)
                            .await?;
                        for up_id in unlinked {
                            warn!("Unlinked account<{account_name}> and up<{up_id}>");
                        }
                    }
                    Err(e) => {
                        record_api_error(
                            ctx,
                            *account_id,
                            &format!("fetch following count for {account_name}"),
                            &e,
                            &mut expired_accounts,
                            &mut summary,
                        )
                        .await;
                    }
                }
            }
        }
    }

    // Phase 2: Fetch medias in active (or explicitly targeted) favorite sets.
    if do_sets {
        let mut fetched_sets: HashSet<i64> = HashSet::new();
        for (account_id, account_name, client) in &clients {
            if expired_accounts.contains(account_id) {
                continue;
            }
            let candidate_set_ids = if !opts.set_ids.is_empty() {
                opts.set_ids.clone()
            } else {
                ctx.db.get_set_ids_of_account(*account_id).await?
            };

            for set_id in candidate_set_ids {
                if fetched_sets.contains(&set_id) {
                    continue;
                }
                let set_row = match ctx.db.get_set(set_id).await {
                    Ok(s) => s,
                    Err(e) => {
                        summary.errors.push(format!("{e:#}"));
                        error!("{e:#}");
                        continue;
                    }
                };
                if opts.set_ids.is_empty() && set_row.state != SetState::Active.as_str() {
                    continue;
                }

                let existing_ids: HashSet<i64> = ctx
                    .db
                    .get_media_ids_of_set(set_id)
                    .await?
                    .into_iter()
                    .collect();

                if set_row.count == 0 && !opts.full {
                    if !existing_ids.is_empty() {
                        let removed = ctx.db.unlink_stale_media_sets(set_id, &[]).await?;
                        if removed > 0 {
                            info!(
                                "Unlinked {removed} removed media(s) from empty set<{}>",
                                set_row.name
                            );
                        }
                    }
                    fetched_sets.insert(set_id);
                    continue;
                }

                info!(
                    "Fetching medias in set<{}> (account<{account_name}>)",
                    set_row.name
                );
                let full_scan = opts.full || opts.prune || existing_ids.is_empty();
                let mut had_page_error = false;
                let mut completed_full_scan = false;
                let mut seen_in_set: Vec<i64> = Vec::new();
                let mut media_models: Vec<media::Model> = Vec::new();
                let mut media_set_links: Vec<media_set::Model> = Vec::new();
                let mut discovered_ups: HashMap<i64, up::Model> = HashMap::new();
                let mut media_up_links: Vec<media_up::Model> = Vec::new();
                let mut invalidated_ids: Vec<i64> = Vec::new();

                for pn in 1i64..=1000 {
                    match client.set_medias(set_id, pn, 20).await {
                        Ok(data) => {
                            if data.medias.is_empty() {
                                completed_full_scan = true;
                                break;
                            }
                            let all_already_known =
                                data.medias.iter().all(|m| existing_ids.contains(&m.id));
                            for m in data.medias {
                                let invalidated = m.is_invalidated();
                                if invalidated {
                                    invalidated_ids.push(m.id);
                                }
                                seen_in_set.push(m.id);
                                if let Some(upper) = m.upper
                                    && upper.mid > 0
                                {
                                    discovered_ups.insert(
                                        upper.mid,
                                        up::Model {
                                            up_id: upper.mid,
                                            name: upper.name,
                                            state: UpState::Inactive.to_string(),
                                        },
                                    );
                                    media_up_links.push(media_up::Model {
                                        id: m.id,
                                        up_id: upper.mid,
                                    });
                                }
                                media_set_links.push(media_set::Model { id: m.id, set_id });
                                media_models.push(media::Model {
                                    id: m.id,
                                    bv_id: m.bv_id,
                                    title: m.title,
                                    r#type: m.r#type.to_string(),
                                    state: if invalidated {
                                        MediaState::Expired.to_string()
                                    } else {
                                        MediaState::Pending.to_string()
                                    },
                                    retry_count: 0,
                                    last_error: None,
                                    updated_at: None,
                                });
                            }

                            if !data.has_more {
                                completed_full_scan = true;
                                break;
                            }
                            if !full_scan && all_already_known {
                                debug!(
                                    set_id,
                                    pn,
                                    "Incremental set fetch reached already-known page; stopping early"
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            had_page_error = true;
                            record_api_error(
                                ctx,
                                *account_id,
                                &format!("fetch set<{}> page {pn}", set_row.name),
                                &e,
                                &mut expired_accounts,
                                &mut summary,
                            )
                            .await;
                            break;
                        }
                    }
                }

                if !media_models.is_empty() {
                    summary.medias_upserted += media_models.len();
                    ctx.db.upsert_medias(media_models).await?;
                    ctx.db.upsert_media_sets(media_set_links).await?;
                }
                if !discovered_ups.is_empty() {
                    ctx.db.upsert_ups(discovered_ups.into_values()).await?;
                    ctx.db.upsert_media_ups(media_up_links).await?;
                }
                for inv_id in invalidated_ids {
                    // Only mark Pending items Expired; preserve already-Completed local backups.
                    if let Ok(existing) = ctx.db.get_media(inv_id).await
                        && existing.state == MediaState::Pending.as_str()
                    {
                        let _ = ctx.db.set_media_state(inv_id, MediaState::Expired).await;
                    }
                }

                if !had_page_error && completed_full_scan {
                    let removed = ctx.db.unlink_stale_media_sets(set_id, &seen_in_set).await?;
                    if removed > 0 {
                        info!(
                            "Unlinked {removed} unfavorited media(s) from set<{}>",
                            set_row.name
                        );
                    }
                }

                if !had_page_error {
                    fetched_sets.insert(set_id);
                }
                if expired_accounts.contains(account_id) {
                    break;
                }
            }
        }
    }

    // Phase 3: Fetch published videos of active (or explicitly targeted) UPs.
    if do_ups {
        let mut fetched_ups: HashSet<i64> = HashSet::new();
        for (account_id, account_name, client) in &clients {
            if expired_accounts.contains(account_id) {
                continue;
            }
            let candidate_up_ids = if !opts.up_ids.is_empty() {
                opts.up_ids.clone()
            } else {
                ctx.db.get_up_ids_of_account(*account_id).await?
            };

            for up_id in candidate_up_ids {
                if fetched_ups.contains(&up_id) {
                    continue;
                }
                let up_row = match ctx.db.get_up(up_id).await {
                    Ok(u) => u,
                    Err(e) => {
                        summary.errors.push(format!("{e:#}"));
                        error!("{e:#}");
                        continue;
                    }
                };
                if opts.up_ids.is_empty() && up_row.state != UpState::Active.as_str() {
                    continue;
                }

                let existing_ids: HashSet<i64> = ctx
                    .db
                    .query_medias(&MediaFilter {
                        up_id: Some(up_id),
                        ..Default::default()
                    })
                    .await?
                    .into_iter()
                    .map(|m| m.id)
                    .collect();
                let full_scan = opts.full || existing_ids.is_empty();

                info!(
                    "Fetching published videos of up<{}> (account<{account_name}>)",
                    up_row.name
                );

                let mut had_page_error = false;
                let mut media_models: Vec<media::Model> = Vec::new();
                let mut media_up_links: Vec<media_up::Model> = Vec::new();

                for pn in 1i64..=1000 {
                    match client.up_medias(up_id, pn, 30).await {
                        Ok(data) => {
                            let vlist = data.list.vlist;
                            if vlist.is_empty() {
                                break;
                            }
                            let page_len = vlist.len() as i64;
                            let all_already_known =
                                vlist.iter().all(|m| existing_ids.contains(&m.id));
                            for m in vlist {
                                media_up_links.push(media_up::Model { id: m.id, up_id });
                                media_models.push(media::Model {
                                    id: m.id,
                                    bv_id: m.bv_id,
                                    title: m.title,
                                    r#type: m.r#type.to_string(),
                                    state: MediaState::Pending.to_string(),
                                    retry_count: 0,
                                    last_error: None,
                                    updated_at: None,
                                });
                            }

                            let total_count = data.page.map(|p| p.count).unwrap_or(0);
                            if (total_count > 0 && pn * 30 >= total_count) || page_len < 30 {
                                break;
                            }
                            if !full_scan && all_already_known {
                                debug!(
                                    up_id,
                                    pn,
                                    "Incremental UP fetch reached already-known page; stopping early"
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            had_page_error = true;
                            record_api_error(
                                ctx,
                                *account_id,
                                &format!("fetch up<{}> page {pn}", up_row.name),
                                &e,
                                &mut expired_accounts,
                                &mut summary,
                            )
                            .await;
                            break;
                        }
                    }
                }

                if !media_models.is_empty() {
                    summary.medias_upserted += media_models.len();
                    ctx.db.upsert_medias(media_models).await?;
                    ctx.db.upsert_media_ups(media_up_links).await?;
                }
                if !had_page_error {
                    fetched_ups.insert(up_id);
                }
                if expired_accounts.contains(account_id) {
                    break;
                }
            }
        }
    }

    // Phase 4: Backfill UP metadata ONLY for active medias still missing any `media_up` link.
    let missing_up_medias: Vec<media::Model> = ctx
        .db
        .all_active_medias_without_up()
        .await?
        .into_iter()
        .filter(|m| {
            m.state != MediaState::Expired.as_str() && m.state != MediaState::Skipped.as_str()
        })
        .collect();

    if !missing_up_medias.is_empty()
        && let Some((account_id, account_name, client)) = clients
            .iter()
            .find(|(id, _, _)| !expired_accounts.contains(id))
    {
        info!(
            "Backfilling UP metadata for {} unlinked active media(s) with account<{account_name}>",
            missing_up_medias.len()
        );
        let pb = if ctx.progress_enabled()
            && let Some(mp) = &ctx.multi_progress
        {
            let pb = mp.add(ProgressBar::new(missing_up_medias.len() as u64));
            pb.set_message("Fetching media metadata");
            if let Ok(style) = ProgressStyle::with_template(FETCH_BAR_TEMPLATE) {
                pb.set_style(style.progress_chars("#>-"));
            }
            Some(pb)
        } else {
            None
        };

        let mut tasks = futures::stream::iter(missing_up_medias)
            .map(|m| {
                let client = client.clone();
                async move {
                    let res = client.media_info(m.id).await;
                    (m, res)
                }
            })
            .buffer_unordered(api_concurrency);

        let mut ups: HashMap<i64, Up> = HashMap::new();
        let mut links: Vec<media_up::Model> = Vec::new();

        while let Some((m, res)) = tasks.next().await {
            if let Some(ref pb) = pb {
                pb.inc(1);
            }
            match res {
                Ok(info) => {
                    summary.metadata_backfilled += 1;
                    ups.insert(info.owner.mid, info.owner.clone());
                    links.push(media_up::Model {
                        id: m.id,
                        up_id: info.owner.mid,
                    });
                    if let Some(staff) = info.staff {
                        for member in staff {
                            ups.insert(member.mid, member.clone());
                            links.push(media_up::Model {
                                id: m.id,
                                up_id: member.mid,
                            });
                        }
                    }
                }
                Err(e) => {
                    if let Some(api_err) = e.downcast_ref::<BiliApiError>() {
                        if api_err.is_media_expired() {
                            if m.state == MediaState::Pending.as_str() {
                                let _ = ctx.db.set_media_state(m.id, MediaState::Expired).await;
                            }
                            warn!("Media<{} {}> is expired/deleted: {api_err}", m.id, m.title);
                            continue;
                        } else if api_err.is_permission_denied() {
                            if m.state == MediaState::Pending.as_str() {
                                let _ = ctx
                                    .db
                                    .set_media_state(m.id, MediaState::PermissionDenied)
                                    .await;
                            }
                            warn!("Media<{} {}> permission denied: {api_err}", m.id, m.title);
                            continue;
                        }
                    }
                    record_api_error(
                        ctx,
                        *account_id,
                        &format!("fetch metadata for media<{} {}>", m.id, m.title),
                        &e,
                        &mut expired_accounts,
                        &mut summary,
                    )
                    .await;
                }
            }
        }

        if let Some(pb) = pb {
            pb.finish_and_clear();
        }

        if !ups.is_empty() {
            ctx.db
                .upsert_ups(ups.into_values().map(|u| up::Model {
                    up_id: u.mid,
                    name: u.name,
                    state: UpState::Inactive.to_string(),
                }))
                .await?;
        }
        if !links.is_empty() {
            ctx.db.upsert_media_ups(links).await?;
        }
    }

    // Phase 5: Optional prune in strict dependency order (`sets -> medias -> ups`).
    if opts.prune {
        let report = ctx.db.prune_all(opts.purge_completed, false).await?;
        info!(
            "Pruned {} set(s), {} media(s), {} up(s).",
            report.sets_pruned, report.medias_pruned, report.ups_pruned
        );
    }

    info!(
        "Finished fetch: {} set(s), {} up(s), {} media(s) synced, {} backfilled, {} error(s).",
        summary.sets_synced,
        summary.ups_synced,
        summary.medias_upserted,
        summary.metadata_backfilled,
        summary.errors.len()
    );

    if let Ok(status) = ctx.db.status_summary(0).await {
        if status.sets.active == 0 && status.ups.active == 0 {
            ctx.hint("Activate sets or UPs with `fav activate set` / `fav activate up`, then run `fav fetch`.");
        } else if status.medias.pending > 0 {
            ctx.hint(&format!(
                "Run `fav pull` to download {} pending media(s) (or `fav status` to inspect).",
                status.medias.pending
            ));
        }
    }

    if !summary.errors.is_empty() && !opts.ignore_errors {
        if summary.auth_expired_accounts == accounts.len() {
            return Err(AppError::auth(anyhow!(
                "All active accounts failed authentication during fetch."
            )));
        }
        return Err(AppError::partial(anyhow!(
            "Fetch completed with {} error(s).",
            summary.errors.len()
        )));
    }
    Ok(())
}

fn effective_prune(opts: &FetchOptions, config: &crate::config::Config) -> bool {
    opts.prune || config.prune.unwrap_or(false)
}

fn resolve_scope(opts: &FetchOptions) -> (bool, bool) {
    if opts.sets_only && !opts.ups_only {
        return (true, false);
    }
    if opts.ups_only && !opts.sets_only {
        return (false, true);
    }
    if !opts.set_ids.is_empty() && opts.up_ids.is_empty() {
        return (true, false);
    }
    if !opts.up_ids.is_empty() && opts.set_ids.is_empty() {
        return (false, true);
    }
    (true, true)
}

async fn run_fetch_dry_run(
    ctx: &AppContext,
    opts: &FetchOptions,
    account_count: usize,
    do_sets: bool,
    do_ups: bool,
) -> AppResult<()> {
    let status = ctx.db.status_summary(0).await?;
    eprintln!(
        "[dry-run] Would fetch with {account_count} active account(s) (sets={do_sets}, ups={do_ups}, full={}).",
        opts.full
    );
    eprintln!(
        "[dry-run] Active targets currently tracked: {} set(s), {} up(s).",
        status.sets.active, status.ups.active
    );
    if opts.prune {
        let report = ctx.db.prune_all(opts.purge_completed, true).await?;
        eprintln!(
            "[dry-run] Prune would remove {} set(s), {} media(s), {} up(s) (purge_completed={}).",
            report.sets_pruned, report.medias_pruned, report.ups_pruned, opts.purge_completed
        );
    }
    Ok(())
}

async fn record_api_error(
    ctx: &AppContext,
    account_id: i64,
    context_label: &str,
    err: &anyhow::Error,
    expired_accounts: &mut HashSet<i64>,
    summary: &mut FetchSummary,
) {
    if err
        .downcast_ref::<BiliApiError>()
        .is_some_and(BiliApiError::is_auth_expired)
    {
        if expired_accounts.insert(account_id) {
            summary.auth_expired_accounts += 1;
            let _ = ctx
                .db
                .set_account_state(account_id, AccountState::Expired)
                .await;
            error!("Account<{account_id}> cookies expired (-101); marked Expired.");
        }
    } else {
        error!("Failed to {context_label}: {err:#}");
    }
    summary.errors.push(format!("{context_label}: {err:#}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn effective_prune_respects_cli_flag_and_config_default() {
        let default_opts = FetchOptions::default();
        let default_cfg = Config::default();
        assert!(!effective_prune(&default_opts, &default_cfg));

        let cfg_prune_true = Config {
            prune: Some(true),
            ..Default::default()
        };
        assert!(effective_prune(&default_opts, &cfg_prune_true));

        let cfg_prune_false = Config {
            prune: Some(false),
            ..Default::default()
        };
        assert!(!effective_prune(&default_opts, &cfg_prune_false));

        let cli_prune = FetchOptions {
            prune: true,
            ..Default::default()
        };
        assert!(effective_prune(&cli_prune, &cfg_prune_false));
    }

    #[tokio::test]
    async fn config_prune_true_triggers_pruning_when_opts_prune_is_false() {
        use std::{sync::Arc, time::Duration};
        use tokio::{
            io::{AsyncReadExt as _, AsyncWriteExt as _},
            net::TcpListener,
        };
        use url::Url;

        use crate::{
            api::BiliClientConfig,
            config::Workspace,
            db::Db,
            entity::{account, media, media_set, set},
            muxer::test_support::MockMuxer,
            state::{AccountState, MediaState, MediaType, SetState},
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = match stream.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let path = req
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("/");
                    let body = if path.starts_with("/x/v3/fav/folder/created/list-all") {
                        r#"{"code":0,"data":{"list":[]}}"#
                    } else if path.starts_with("/x/relation/stat") {
                        r#"{"code":0,"data":{"following":0}}"#
                    } else if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket")
                    {
                        r#"{"code":0,"data":{"ticket":"t","created_at":4000000000,"ttl":86400}}"#
                    } else if path.starts_with("/x/web-frontend/getbuvid") {
                        r#"{"code":0,"data":{"buvid":"B1"}}"#
                    } else {
                        r#"{"code":0,"data":{}}"#
                    };
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(account::Model {
            account_id: 1,
            name: "Alice".to_owned(),
            cookies: "SESSDATA=ok; bili_jct=csrf".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        // Seed an orphan inactive set + pending media that should be pruned when config.prune = true.
        db.upsert_sets([set::Model {
            set_id: 99,
            name: "Orphan".to_owned(),
            count: 1,
            state: SetState::Inactive.to_string(),
        }])
        .await
        .unwrap();
        db.upsert_medias([media::Model {
            id: 500,
            bv_id: "BV1prune000500".to_owned(),
            title: "Orphan Pending".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        }])
        .await
        .unwrap();
        db.upsert_media_sets([media_set::Model {
            id: 500,
            set_id: 99,
        }])
        .await
        .unwrap();

        let base = Url::parse(&format!("http://{addr}")).unwrap();
        let api_cfg = BiliClientConfig {
            api_base_url: base.clone(),
            passport_base_url: base,
            max_retries: 0,
            base_backoff: Duration::ZERO,
            min_request_interval: Duration::ZERO,
            ..Default::default()
        };
        let cfg = Config {
            prune: Some(true),
            ..Default::default()
        };
        let ctx = AppContext::for_test(ws, cfg, db, Arc::new(MockMuxer::succeeding()), api_cfg);

        // `opts.prune` is explicitly `false`, yet `config.prune = Some(true)` must trigger pruning!
        fetch(&ctx, FetchOptions::default()).await.unwrap();

        assert!(
            ctx.db.get_set(99).await.is_err(),
            "orphan set 99 should have been pruned via config.prune = true"
        );
        assert!(
            ctx.db.get_media(500).await.is_err(),
            "inactive pending media 500 should have been pruned via config.prune = true"
        );
    }
}
