use std::{
    collections::{HashMap, HashSet},
    str::FromStr as _,
};

use anyhow::anyhow;
use futures::StreamExt as _;
use indicatif::{ProgressBar, ProgressStyle};
use tracing::{debug, error, info, warn};

use crate::{
    api::{BiliApiError, BiliClient},
    context::AppContext,
    entity::{media, media_set, media_up, set, up},
    error::{AppError, AppResult},
    response::{Media as RemoteMedia, Up},
    state::{AccountState, MediaState, SetKind, SetState, UpState},
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
    if opts.purge_completed && !opts.prune {
        return Err(AppError::usage(anyhow!(
            "--purge-completed requires --prune (or `prune = true` in config.toml)."
        )));
    }
    let accounts = ctx.db.active_accounts().await?;
    if accounts.is_empty() {
        return Err(AppError::usage(anyhow!(
            "No active accounts. Run `fav auth login` or `fav activate account` first."
        )));
    }

    // Eagerly validate explicit --set / --up targets before any network requests.
    validate_explicit_targets(ctx, &opts).await?;

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

    // Phase 1: Sync account favorite folders (created + collected/seasons) and followed UPs.
    let sync_catalog = opts.set_ids.is_empty() && opts.up_ids.is_empty();
    if sync_catalog {
        for (account_id, account_name, client) in &clients {
            if expired_accounts.contains(account_id) {
                continue;
            }
            if do_sets {
                sync_account_set_catalog(
                    ctx,
                    *account_id,
                    account_name,
                    client,
                    &mut expired_accounts,
                    &mut summary,
                )
                .await?;
                if expired_accounts.contains(account_id) {
                    continue;
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
                                                sync_complete: false,
                                                resume_page: None,
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

    let full_scan = opts.full;

    // Phase 2: Fetch medias in active favorite sets (created, collected folders, and seasons).
    if do_sets {
        let mut fetched_sets: HashSet<i64> = HashSet::new();
        for (account_id, account_name, client) in &clients {
            if expired_accounts.contains(account_id) {
                continue;
            }
            let account_set_ids = ctx.db.get_set_ids_of_account(*account_id).await?;
            let candidate_set_ids: Vec<i64> = if !opts.set_ids.is_empty() {
                let owned: HashSet<i64> = account_set_ids.into_iter().collect();
                opts.set_ids
                    .iter()
                    .copied()
                    .filter(|id| owned.contains(id))
                    .collect()
            } else {
                account_set_ids
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
                if set_row.state != SetState::Active.as_str() {
                    continue;
                }

                let mut existing_ids: HashSet<i64> = ctx
                    .db
                    .get_media_ids_of_set(set_id)
                    .await?
                    .into_iter()
                    .collect();

                if sync_catalog && set_row.count == 0 && !full_scan {
                    if !existing_ids.is_empty() {
                        let removed = ctx.db.unlink_stale_media_sets(set_id, &[]).await?;
                        if removed > 0 {
                            info!(
                                "Unlinked {removed} removed media(s) from empty set<{}>",
                                set_row.name
                            );
                        }
                    }
                    if !set_row.sync_complete || set_row.resume_page.is_some() {
                        ctx.db.set_set_sync_progress(set_id, true, None).await?;
                    }
                    fetched_sets.insert(set_id);
                    continue;
                }

                let set_kind = SetKind::from_str(&set_row.kind).unwrap_or_default();
                let season_owner_mid = if set_kind == SetKind::Season {
                    match set_row.owner_mid.filter(|&m| m > 0) {
                        Some(mid) => Some(mid),
                        None => {
                            let msg = format!(
                                "Season set<{set_id} {}> is missing owner_mid; re-run `fav fetch` to refresh set catalog",
                                set_row.name
                            );
                            error!("{msg}");
                            summary.errors.push(msg);
                            continue;
                        }
                    }
                } else {
                    None
                };

                info!(
                    "Fetching medias in {} set<{}> (account<{account_name}>)",
                    set_kind, set_row.name
                );

                let mut had_page_error = false;
                if full_scan {
                    let mut completed_full_scan = false;
                    let mut seen_in_set: Vec<i64> = Vec::new();
                    for pn in 1i64..=1000 {
                        match fetch_set_page(client, set_id, set_kind, season_owner_mid, pn).await {
                            Ok((items, reached_end)) => {
                                for m in &items {
                                    seen_in_set.push(m.id);
                                }
                                persist_set_page_batch(
                                    ctx,
                                    set_id,
                                    season_owner_mid,
                                    items,
                                    &mut summary,
                                )
                                .await?;
                                if reached_end {
                                    completed_full_scan = true;
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
                    if !had_page_error && completed_full_scan {
                        let removed = ctx.db.unlink_stale_media_sets(set_id, &seen_in_set).await?;
                        if removed > 0 {
                            info!(
                                "Unlinked {removed} unfavorited media(s) from set<{}>",
                                set_row.name
                            );
                        }
                        ctx.db.set_set_sync_progress(set_id, true, None).await?;
                    }
                } else if set_row.sync_complete {
                    for pn in 1i64..=1000 {
                        match fetch_set_page(client, set_id, set_kind, season_owner_mid, pn).await {
                            Ok((items, reached_end)) => {
                                let mut unseen = Vec::new();
                                let mut hit_existing = false;
                                for m in items {
                                    if existing_ids.contains(&m.id) {
                                        hit_existing = true;
                                        break;
                                    }
                                    existing_ids.insert(m.id);
                                    unseen.push(m);
                                }
                                persist_set_page_batch(
                                    ctx,
                                    set_id,
                                    season_owner_mid,
                                    unseen,
                                    &mut summary,
                                )
                                .await?;
                                if hit_existing {
                                    debug!(
                                        set_id,
                                        pn,
                                        "Incremental set fetch reached already-fetched media; stopping early"
                                    );
                                    break;
                                }
                                if reached_end {
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
                } else {
                    let start_pn = set_row.resume_page.map(i64::from).unwrap_or(1).max(1);
                    let mut finished_during_head = false;

                    if start_pn > 1 {
                        for head_pn in 1..start_pn {
                            match fetch_set_page(
                                client,
                                set_id,
                                set_kind,
                                season_owner_mid,
                                head_pn,
                            )
                            .await
                            {
                                Ok((items, reached_end)) => {
                                    let mut unseen = Vec::new();
                                    let mut hit_existing = false;
                                    for m in items {
                                        if existing_ids.contains(&m.id) {
                                            hit_existing = true;
                                        } else {
                                            existing_ids.insert(m.id);
                                            unseen.push(m);
                                        }
                                    }
                                    persist_set_page_batch(
                                        ctx,
                                        set_id,
                                        season_owner_mid,
                                        unseen,
                                        &mut summary,
                                    )
                                    .await?;
                                    if reached_end {
                                        ctx.db.set_set_sync_progress(set_id, true, None).await?;
                                        finished_during_head = true;
                                        break;
                                    }
                                    if hit_existing {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    had_page_error = true;
                                    record_api_error(
                                        ctx,
                                        *account_id,
                                        &format!("fetch set<{}> page {head_pn}", set_row.name),
                                        &e,
                                        &mut expired_accounts,
                                        &mut summary,
                                    )
                                    .await;
                                    break;
                                }
                            }
                        }
                    }

                    if !had_page_error && !finished_during_head {
                        for pn in start_pn..=1000 {
                            match fetch_set_page(client, set_id, set_kind, season_owner_mid, pn)
                                .await
                            {
                                Ok((items, reached_end)) => {
                                    for m in &items {
                                        existing_ids.insert(m.id);
                                    }
                                    persist_set_page_batch(
                                        ctx,
                                        set_id,
                                        season_owner_mid,
                                        items,
                                        &mut summary,
                                    )
                                    .await?;
                                    if reached_end {
                                        ctx.db.set_set_sync_progress(set_id, true, None).await?;
                                        break;
                                    } else {
                                        let next_page = i32::try_from(pn + 1).ok();
                                        ctx.db
                                            .set_set_sync_progress(set_id, false, next_page)
                                            .await?;
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

    // Phase 3: Fetch published videos of active UPs.
    if do_ups {
        let mut fetched_ups: HashSet<i64> = HashSet::new();
        for (account_id, account_name, client) in &clients {
            if expired_accounts.contains(account_id) {
                continue;
            }
            let account_up_ids = ctx.db.get_up_ids_of_account(*account_id).await?;
            let candidate_up_ids: Vec<i64> = if !opts.up_ids.is_empty() {
                let followed: HashSet<i64> = account_up_ids.into_iter().collect();
                opts.up_ids
                    .iter()
                    .copied()
                    .filter(|id| followed.contains(id))
                    .collect()
            } else {
                account_up_ids
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
                if up_row.state != UpState::Active.as_str() {
                    continue;
                }

                let mut existing_ids: HashSet<i64> = ctx
                    .db
                    .get_up_feed_media_ids(up_id)
                    .await?
                    .into_iter()
                    .collect();

                info!(
                    "Fetching published videos of up<{}> (account<{account_name}>)",
                    up_row.name
                );

                let mut had_page_error = false;
                if full_scan {
                    let mut completed_full_scan = false;
                    for pn in 1i64..=1000 {
                        match fetch_up_page(client, up_id, pn).await {
                            Ok((items, reached_end)) => {
                                persist_up_page_batch(ctx, up_id, items, &mut summary).await?;
                                if reached_end {
                                    completed_full_scan = true;
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
                    if !had_page_error && completed_full_scan {
                        ctx.db.set_up_sync_progress(up_id, true, None).await?;
                    }
                } else if up_row.sync_complete {
                    for pn in 1i64..=1000 {
                        match fetch_up_page(client, up_id, pn).await {
                            Ok((items, reached_end)) => {
                                let mut unseen = Vec::new();
                                let mut hit_existing = false;
                                for m in items {
                                    if existing_ids.contains(&m.id) {
                                        hit_existing = true;
                                        break;
                                    }
                                    existing_ids.insert(m.id);
                                    unseen.push(m);
                                }
                                persist_up_page_batch(ctx, up_id, unseen, &mut summary).await?;
                                if hit_existing {
                                    debug!(
                                        up_id,
                                        pn,
                                        "Incremental UP fetch reached already-fetched media; stopping early"
                                    );
                                    break;
                                }
                                if reached_end {
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
                } else {
                    let start_pn = up_row.resume_page.map(i64::from).unwrap_or(1).max(1);
                    let mut finished_during_head = false;

                    if start_pn > 1 {
                        for head_pn in 1..start_pn {
                            match fetch_up_page(client, up_id, head_pn).await {
                                Ok((items, reached_end)) => {
                                    let mut unseen = Vec::new();
                                    let mut hit_existing = false;
                                    for m in items {
                                        if existing_ids.contains(&m.id) {
                                            hit_existing = true;
                                        } else {
                                            existing_ids.insert(m.id);
                                            unseen.push(m);
                                        }
                                    }
                                    persist_up_page_batch(ctx, up_id, unseen, &mut summary).await?;
                                    if reached_end {
                                        ctx.db.set_up_sync_progress(up_id, true, None).await?;
                                        finished_during_head = true;
                                        break;
                                    }
                                    if hit_existing {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    had_page_error = true;
                                    record_api_error(
                                        ctx,
                                        *account_id,
                                        &format!("fetch up<{}> page {head_pn}", up_row.name),
                                        &e,
                                        &mut expired_accounts,
                                        &mut summary,
                                    )
                                    .await;
                                    break;
                                }
                            }
                        }
                    }

                    if !had_page_error && !finished_during_head {
                        for pn in start_pn..=1000 {
                            match fetch_up_page(client, up_id, pn).await {
                                Ok((items, reached_end)) => {
                                    for m in &items {
                                        existing_ids.insert(m.id);
                                    }
                                    persist_up_page_batch(ctx, up_id, items, &mut summary).await?;
                                    if reached_end {
                                        ctx.db.set_up_sync_progress(up_id, true, None).await?;
                                        break;
                                    } else {
                                        let next_page = i32::try_from(pn + 1).ok();
                                        ctx.db
                                            .set_up_sync_progress(up_id, false, next_page)
                                            .await?;
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
                    }
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
    // Unless `--full` / `--refresh-status` is passed, skip terminal/errored states to avoid wasted `/view` calls.
    let missing_up_medias: Vec<media::Model> = ctx
        .db
        .all_active_medias_without_up()
        .await?
        .into_iter()
        .filter(|m| {
            if opts.full {
                true
            } else {
                m.state == MediaState::Pending.as_str()
                    || m.state == MediaState::Downloading.as_str()
                    || m.state == MediaState::Completed.as_str()
            }
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
                        from_up_feed: false,
                    });
                    if let Some(staff) = info.staff {
                        for member in staff {
                            ups.insert(member.mid, member.clone());
                            links.push(media_up::Model {
                                id: m.id,
                                up_id: member.mid,
                                from_up_feed: false,
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
                    sync_complete: false,
                    resume_page: None,
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

    if let Some(hint) = build_fetch_hint(ctx).await {
        ctx.hint(&hint);
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

async fn build_fetch_hint(ctx: &AppContext) -> Option<String> {
    let status = ctx.db.status_summary(0).await.ok()?;
    if status.sets.active == 0 && status.ups.active == 0 {
        return Some(
            "Activate sets or UPs with `fav activate set` / `fav activate up`, then run `fav fetch`."
                .to_owned(),
        );
    }
    let active_pending = ctx.db.count_active_pending_medias().await.unwrap_or(0);
    if active_pending > 0 {
        Some(format!(
            "Run `fav pull` to download {active_pending} active pending media(s) (or `fav status` to inspect)."
        ))
    } else if status.medias.pending > 0 {
        Some(format!(
            "{} pending media(s) belong to inactive sets/UPs; activate them with `fav activate set` / `fav activate up` before pulling.",
            status.medias.pending
        ))
    } else {
        None
    }
}

async fn validate_explicit_targets(ctx: &AppContext, opts: &FetchOptions) -> AppResult<()> {
    for &set_id in &opts.set_ids {
        let set_row = ctx.db.get_set(set_id).await.map_err(|_| {
            AppError::usage(anyhow!(
                "Unknown set<{set_id}>. Run `fav fetch` to discover sets, then `fav activate set {set_id}`."
            ))
        })?;
        if set_row.state != SetState::Active.as_str() {
            return Err(AppError::usage(anyhow!(
                "Set<{set_id} {}> is {} (not Active). Run `fav activate set {set_id}` first.",
                set_row.name,
                set_row.state
            )));
        }
        if !ctx.db.is_set_owned_by_active_account(set_id).await? {
            return Err(AppError::usage(anyhow!(
                "Set<{set_id} {}> does not belong to any active account. Run `fav activate account` first.",
                set_row.name
            )));
        }
    }

    for &up_id in &opts.up_ids {
        let up_row = ctx.db.get_up(up_id).await.map_err(|_| {
            AppError::usage(anyhow!(
                "Unknown up<{up_id}>. Run `fav fetch` to discover followed UPs, then `fav activate up {up_id}`."
            ))
        })?;
        if up_row.state != UpState::Active.as_str() {
            return Err(AppError::usage(anyhow!(
                "UP<{up_id} {}> is {} (not Active). Run `fav activate up {up_id}` first.",
                up_row.name,
                up_row.state
            )));
        }
        if !ctx.db.is_up_followed_by_active_account(up_id).await? {
            return Err(AppError::usage(anyhow!(
                "UP<{up_id} {}> is not followed by any active account. Run `fav activate account` first.",
                up_row.name
            )));
        }
    }

    Ok(())
}

async fn sync_account_set_catalog(
    ctx: &AppContext,
    account_id: i64,
    account_name: &str,
    client: &BiliClient,
    expired_accounts: &mut HashSet<i64>,
    summary: &mut FetchSummary,
) -> AppResult<()> {
    info!("Fetching favorite sets (created + collected/seasons) for account<{account_name}>");
    let mut had_catalog_error = false;
    let mut combined: Vec<set::Model> = Vec::new();
    let mut seen_ids: HashSet<i64> = HashSet::new();
    let mut invalidated_set_ids: Vec<i64> = Vec::new();
    let mut discovered_ups: HashMap<i64, up::Model> = HashMap::new();

    match client.list_created_sets(account_id).await {
        Ok(list) => {
            for s in list {
                if seen_ids.insert(s.id) {
                    combined.push(set::Model {
                        set_id: s.id,
                        name: s.title,
                        count: s.media_count,
                        state: SetState::Inactive.to_string(),
                        kind: SetKind::Created.to_string(),
                        owner_mid: Some(account_id),
                        sync_complete: false,
                        resume_page: None,
                    });
                }
            }
        }
        Err(e) => {
            had_catalog_error = true;
            record_api_error(
                ctx,
                account_id,
                &format!("fetch created sets for {account_name}"),
                &e,
                expired_accounts,
                summary,
            )
            .await;
            if expired_accounts.contains(&account_id) {
                return Ok(());
            }
        }
    }

    let mut seen_collected: i64 = 0;
    for pn in 1i64..=1000 {
        match client.list_collected_sets(account_id, pn, 20).await {
            Ok(data) => {
                if data.list.is_empty() {
                    break;
                }
                let page_len = data.list.len() as i64;
                seen_collected += page_len;
                for item in data.list {
                    let invalidated = item.is_invalidated();
                    let kind = item.set_kind();
                    let owner_mid = item.owner_mid();
                    if let Some(upper) = item.upper
                        && upper.mid > 0
                        && !upper.name.trim().is_empty()
                    {
                        discovered_ups.insert(
                            upper.mid,
                            up::Model {
                                up_id: upper.mid,
                                name: upper.name,
                                state: UpState::Inactive.to_string(),
                                sync_complete: false,
                                resume_page: None,
                            },
                        );
                    }
                    // Created sets take precedence if an ID ever collides.
                    if seen_ids.insert(item.id) {
                        if invalidated {
                            invalidated_set_ids.push(item.id);
                        }
                        combined.push(set::Model {
                            set_id: item.id,
                            name: item.title,
                            count: item.media_count,
                            state: if invalidated {
                                SetState::Unreachable.to_string()
                            } else {
                                SetState::Inactive.to_string()
                            },
                            kind: kind.to_string(),
                            owner_mid,
                            sync_complete: false,
                            resume_page: None,
                        });
                    }
                }
                if data.has_more == Some(false) {
                    break;
                }
                if data.has_more.is_none() && (seen_collected >= data.count || page_len < 20) {
                    break;
                }
            }
            Err(e) => {
                had_catalog_error = true;
                record_api_error(
                    ctx,
                    account_id,
                    &format!("fetch collected sets page {pn} for {account_name}"),
                    &e,
                    expired_accounts,
                    summary,
                )
                .await;
                break;
            }
        }
    }

    if !discovered_ups.is_empty() {
        ctx.db.upsert_ups(discovered_ups.into_values()).await?;
    }

    if had_catalog_error {
        warn!(
            "Skipping stale set unlinking for account<{account_name}> because one or more set catalog requests failed."
        );
    }

    summary.sets_synced += combined.len();
    let unlinked = ctx
        .db
        .sync_account_sets(account_id, combined, !had_catalog_error)
        .await?;
    for set_id in unlinked {
        warn!("Unlinked account<{account_name}> and set<{set_id}>");
    }

    // For already-known sets that became invalidated remotely, mark Unreachable without deleting Completed medias.
    for inv_set_id in invalidated_set_ids {
        if let Err(e) = ctx
            .db
            .set_set_state(inv_set_id, SetState::Unreachable)
            .await
        {
            warn!("Failed marking invalidated set<{inv_set_id}> Unreachable: {e:#}");
        }
    }

    Ok(())
}

async fn fetch_set_page(
    client: &BiliClient,
    set_id: i64,
    kind: SetKind,
    season_owner_mid: Option<i64>,
    pn: i64,
) -> anyhow::Result<(Vec<RemoteMedia>, bool)> {
    match kind {
        SetKind::Created | SetKind::Collected => {
            let data = client.set_medias(set_id, pn, 20).await?;
            let reached_end = data.medias.is_empty() || !data.has_more;
            Ok((data.medias, reached_end))
        }
        SetKind::Season => {
            let mid = season_owner_mid
                .filter(|&m| m > 0)
                .ok_or_else(|| anyhow!("Season set<{set_id}> is missing owner_mid"))?;
            let data = client.season_archives(mid, set_id, pn, 30, true).await?;
            let page_len = data.archives.len() as i64;
            let total = data.page.as_ref().map(|p| p.total).unwrap_or(0);
            let reached_end =
                data.archives.is_empty() || (total > 0 && pn * 30 >= total) || page_len < 30;
            Ok((data.archives, reached_end))
        }
    }
}

async fn persist_set_page_batch(
    ctx: &AppContext,
    set_id: i64,
    season_owner_mid: Option<i64>,
    items: Vec<RemoteMedia>,
    summary: &mut FetchSummary,
) -> AppResult<()> {
    if items.is_empty() {
        return Ok(());
    }

    let mut media_models = Vec::with_capacity(items.len());
    let mut media_set_links = Vec::with_capacity(items.len());
    let mut discovered_ups: HashMap<i64, up::Model> = HashMap::new();
    let mut media_up_links: Vec<media_up::Model> = Vec::new();
    let mut invalidated_ids: Vec<i64> = Vec::new();

    for m in items {
        let invalidated = m.is_invalidated();
        if invalidated {
            invalidated_ids.push(m.id);
        }
        let mut linked_up_mid = None;
        if let Some(upper) = m.upper
            && upper.mid > 0
        {
            linked_up_mid = Some(upper.mid);
            discovered_ups.insert(
                upper.mid,
                up::Model {
                    up_id: upper.mid,
                    name: upper.name,
                    state: UpState::Inactive.to_string(),
                    sync_complete: false,
                    resume_page: None,
                },
            );
            media_up_links.push(media_up::Model {
                id: m.id,
                up_id: upper.mid,
                from_up_feed: false,
            });
        }
        if let Some(owner_mid) = season_owner_mid
            && owner_mid > 0
            && linked_up_mid != Some(owner_mid)
        {
            media_up_links.push(media_up::Model {
                id: m.id,
                up_id: owner_mid,
                from_up_feed: false,
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

    summary.medias_upserted += media_models.len();
    ctx.db.upsert_medias(media_models).await?;
    ctx.db.upsert_media_sets(media_set_links).await?;

    if let Some(owner_mid) = season_owner_mid
        && owner_mid > 0
        && !discovered_ups.contains_key(&owner_mid)
        && ctx.db.get_up(owner_mid).await.is_err()
    {
        discovered_ups.insert(
            owner_mid,
            up::Model {
                up_id: owner_mid,
                name: format!("up_{owner_mid}"),
                state: UpState::Inactive.to_string(),
                sync_complete: false,
                resume_page: None,
            },
        );
    }

    if !discovered_ups.is_empty() {
        ctx.db.upsert_ups(discovered_ups.into_values()).await?;
    }
    if !media_up_links.is_empty() {
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

    Ok(())
}

async fn fetch_up_page(
    client: &BiliClient,
    up_id: i64,
    pn: i64,
) -> anyhow::Result<(Vec<RemoteMedia>, bool)> {
    let data = client.up_medias(up_id, pn, 30).await?;
    let vlist = data.list.vlist;
    let page_len = vlist.len() as i64;
    let total_count = data.page.as_ref().map(|p| p.count).unwrap_or(0);
    let reached_end =
        vlist.is_empty() || (total_count > 0 && pn * 30 >= total_count) || page_len < 30;
    Ok((vlist, reached_end))
}

async fn persist_up_page_batch(
    ctx: &AppContext,
    up_id: i64,
    items: Vec<RemoteMedia>,
    summary: &mut FetchSummary,
) -> AppResult<()> {
    if items.is_empty() {
        return Ok(());
    }

    let mut media_models = Vec::with_capacity(items.len());
    let mut media_up_links = Vec::with_capacity(items.len());
    let mut invalidated_ids = Vec::new();

    for m in items {
        let invalidated = m.is_invalidated();
        if invalidated {
            invalidated_ids.push(m.id);
        }
        media_up_links.push(media_up::Model {
            id: m.id,
            up_id,
            from_up_feed: true,
        });
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

    summary.medias_upserted += media_models.len();
    ctx.db.upsert_medias(media_models).await?;
    ctx.db.upsert_up_feed_media_ups(media_up_links).await?;

    for inv_id in invalidated_ids {
        if let Ok(existing) = ctx.db.get_media(inv_id).await
            && existing.state == MediaState::Pending.as_str()
        {
            let _ = ctx.db.set_media_state(inv_id, MediaState::Expired).await;
        }
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
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };
    use url::Url;

    use super::*;
    use crate::{
        api::BiliClientConfig,
        config::{Config, Workspace},
        db::Db,
        entity::{account, media, media_set, set, set_account, up, up_account},
        error::ExitKind,
        muxer::test_support::MockMuxer,
        state::{AccountState, MediaState, MediaType, SetState},
    };

    async fn spawn_http<F>(handler: F) -> Url
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
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
                    let body = handler(path);
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        Url::parse(&format!("http://{addr}")).unwrap()
    }

    fn test_api_cfg(base: Url) -> BiliClientConfig {
        BiliClientConfig {
            api_base_url: base.clone(),
            passport_base_url: base,
            max_retries: 0,
            base_backoff: Duration::ZERO,
            min_request_interval: Duration::ZERO,
            ..Default::default()
        }
    }

    fn test_ctx_with_cfg(base: Url, cfg: Config, db: Db) -> (tempfile::TempDir, AppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let ctx = AppContext::for_test(
            ws,
            cfg,
            db,
            Arc::new(MockMuxer::succeeding()),
            test_api_cfg(base),
        );
        (tmp, ctx)
    }

    fn query_param<'a>(path: &'a str, key: &str) -> Option<&'a str> {
        let q = path.split_once('?')?.1;
        q.split('&').find_map(|pair| {
            pair.split_once('=')
                .filter(|(k, _)| *k == key)
                .map(|(_, v)| v)
        })
    }

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
        let base = spawn_http(|path| {
            if path.starts_with("/x/v3/fav/folder/created/list-all") {
                r#"{"code":0,"data":{"list":[]}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/folder/collected/list") {
                r#"{"code":0,"data":{"count":0,"list":[],"has_more":false}}"#.to_owned()
            } else if path.starts_with("/x/relation/stat") {
                r#"{"code":0,"data":{"following":0}}"#.to_owned()
            } else if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket") {
                r#"{"code":0,"data":{"ticket":"t","created_at":4000000000,"ttl":86400}}"#.to_owned()
            } else if path.starts_with("/x/web-frontend/getbuvid") {
                r#"{"code":0,"data":{"buvid":"B1"}}"#.to_owned()
            } else {
                r#"{"code":0,"data":{}}"#.to_owned()
            }
        })
        .await;

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
            kind: SetKind::Created.to_string(),
            owner_mid: None,
            sync_complete: false,
            resume_page: None,
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

        let cfg = Config {
            prune: Some(true),
            ..Default::default()
        };
        let (_tmp, ctx) = test_ctx_with_cfg(base, cfg, db);

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

    #[tokio::test]
    async fn fetch_eagerly_rejects_inactive_or_unowned_set_and_up_targets_before_network() {
        let hits = Arc::new(Mutex::new(Vec::<String>::new()));
        let hits_c = hits.clone();
        let base = spawn_http(move |path| {
            hits_c.lock().unwrap().push(path.to_owned());
            r#"{"code":0,"data":{}}"#.to_owned()
        })
        .await;

        let db = Db::open_in_memory().await.unwrap();
        db.upsert_account(account::Model {
            account_id: 1,
            name: "ActiveAcct".to_owned(),
            cookies: "SESSDATA=ok; bili_jct=csrf".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 2,
            name: "InactiveAcct".to_owned(),
            cookies: "SESSDATA=ok2; bili_jct=csrf2".to_owned(),
            state: AccountState::Inactive.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_sets([
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
                name: "ActiveSetInactiveAcct".to_owned(),
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
        db.upsert_ups([
            up::Model {
                up_id: 100,
                name: "InactiveUp".to_owned(),
                state: UpState::Inactive.to_string(),
                sync_complete: false,
                resume_page: None,
            },
            up::Model {
                up_id: 200,
                name: "ActiveUpInactiveAcct".to_owned(),
                state: UpState::Active.to_string(),
                sync_complete: false,
                resume_page: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_up_accounts([
            up_account::Model {
                up_id: 100,
                account_id: 1,
            },
            up_account::Model {
                up_id: 200,
                account_id: 2,
            },
        ])
        .await
        .unwrap();

        let (_tmp, ctx) = test_ctx_with_cfg(base, Config::default(), db);

        for opts in [
            FetchOptions {
                set_ids: vec![999],
                ..Default::default()
            },
            FetchOptions {
                set_ids: vec![10],
                ..Default::default()
            },
            FetchOptions {
                set_ids: vec![20],
                ..Default::default()
            },
            FetchOptions {
                up_ids: vec![999],
                ..Default::default()
            },
            FetchOptions {
                up_ids: vec![100],
                ..Default::default()
            },
            FetchOptions {
                up_ids: vec![200],
                ..Default::default()
            },
        ] {
            let err = fetch(&ctx, opts).await.unwrap_err();
            assert_eq!(err.kind(), ExitKind::Usage);
        }

        assert!(
            hits.lock().unwrap().is_empty(),
            "eager validation must fail BEFORE any HTTP calls"
        );
    }

    #[tokio::test]
    async fn incremental_cursor_stops_at_first_known_item_and_prune_does_not_force_full_scan() {
        let set_pages_requested = Arc::new(Mutex::new(Vec::<i64>::new()));
        let pages_c = set_pages_requested.clone();

        let base = spawn_http(move |path| {
            if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket") {
                r#"{"code":0,"data":{"ticket":"t","created_at":4000000000,"ttl":86400}}"#.to_owned()
            } else if path.starts_with("/x/web-frontend/getbuvid") {
                r#"{"code":0,"data":{"buvid":"B1"}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/folder/created/list-all") {
                r#"{"code":0,"data":{"list":[{"id":10,"title":"Fav","media_count":60}]}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/folder/collected/list") {
                r#"{"code":0,"data":{"count":0,"list":[],"has_more":false}}"#.to_owned()
            } else if path.starts_with("/x/relation/stat") {
                r#"{"code":0,"data":{"following":0}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/resource/list") {
                let pn: i64 = query_param(path, "pn").unwrap_or("1").parse().unwrap();
                pages_c.lock().unwrap().push(pn);
                match pn {
                    1 => r#"{"code":0,"data":{"has_more":true,"medias":[
                        {"id":301,"bvid":"BV1new00000301","title":"Brand New","type":2,"attr":0,"upper":{"mid":7,"name":"U7"}},
                        {"id":201,"bvid":"BV1old00000201","title":"Already Known","type":2,"attr":0,"upper":{"mid":7,"name":"U7"}}
                    ]}}"#
                        .to_owned(),
                    2 => r#"{"code":0,"data":{"has_more":true,"medias":[
                        {"id":102,"bvid":"BV1old00000102","title":"Page2","type":2,"attr":9,"upper":{"mid":7,"name":"U7"}}
                    ]}}"#
                        .to_owned(),
                    _ => r#"{"code":0,"data":{"has_more":false,"medias":[
                        {"id":103,"bvid":"BV1old00000103","title":"Page3","type":2,"attr":0,"upper":{"mid":7,"name":"U7"}}
                    ]}}"#
                        .to_owned(),
                }
            } else {
                r#"{"code":0,"data":{}}"#.to_owned()
            }
        })
        .await;

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
        db.upsert_sets([set::Model {
            set_id: 10,
            name: "Fav".to_owned(),
            count: 60,
            state: SetState::Active.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: true,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_set_accounts([set_account::Model {
            set_id: 10,
            account_id: 1,
        }])
        .await
        .unwrap();
        db.upsert_medias([
            media::Model {
                id: 201,
                bv_id: "BV1old00000201".to_owned(),
                title: "Already Known".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 102,
                bv_id: "BV1old00000102".to_owned(),
                title: "Page2".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 999,
                bv_id: "BV1unfav000999".to_owned(),
                title: "Unfavorited".to_owned(),
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
                id: 201,
                set_id: 10,
            },
            media_set::Model {
                id: 102,
                set_id: 10,
            },
            media_set::Model {
                id: 999,
                set_id: 10,
            },
        ])
        .await
        .unwrap();
        db.upsert_ups([up::Model {
            up_id: 7,
            name: "U7".to_owned(),
            state: UpState::Inactive.to_string(),
            sync_complete: false,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_media_ups([
            media_up::Model {
                id: 201,
                up_id: 7,
                from_up_feed: false,
            },
            media_up::Model {
                id: 102,
                up_id: 7,
                from_up_feed: false,
            },
            media_up::Model {
                id: 999,
                up_id: 7,
                from_up_feed: false,
            },
        ])
        .await
        .unwrap();

        let (_tmp, ctx) = test_ctx_with_cfg(base, Config::default(), db);

        // Incremental fetch WITH --prune must STILL stop after page 1!
        fetch(
            &ctx,
            FetchOptions {
                sets_only: true,
                prune: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            *set_pages_requested.lock().unwrap(),
            vec![1],
            "incremental fetch (even with --prune) must stop after page 1 on first known media"
        );
        assert!(
            ctx.db.get_media(301).await.is_ok(),
            "new item 301 must be upserted"
        );
        assert_eq!(
            ctx.db.get_media(102).await.unwrap().state,
            MediaState::Pending.as_str(),
            "page 2 was not scanned during incremental run"
        );

        // Explicit --full scans all 3 pages, marks invalidated Pending item 102 Expired, and unlinks 999!
        set_pages_requested.lock().unwrap().clear();
        fetch(
            &ctx,
            FetchOptions {
                sets_only: true,
                full: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(*set_pages_requested.lock().unwrap(), vec![1, 2, 3]);
        assert_eq!(
            ctx.db.get_media(102).await.unwrap().state,
            MediaState::Expired.as_str()
        );
        assert!(ctx.db.get_media(103).await.is_ok());
        let set_ids = ctx.db.get_media_ids_of_set(10).await.unwrap();
        assert!(
            !set_ids.contains(&999),
            "stale media 999 must be unlinked on --full"
        );
    }

    #[tokio::test]
    async fn resumable_initial_backfill_persists_cursor_and_resumes_without_rescanning() {
        let fail_page_2 = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let fail_c = fail_page_2.clone();
        let requested = Arc::new(Mutex::new(Vec::<i64>::new()));
        let req_c = requested.clone();

        let base = spawn_http(move |path| {
            if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket") {
                r#"{"code":0,"data":{"ticket":"t","created_at":4000000000,"ttl":86400}}"#.to_owned()
            } else if path.starts_with("/x/web-frontend/getbuvid") {
                r#"{"code":0,"data":{"buvid":"B1"}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/resource/list") {
                let pn: i64 = query_param(path, "pn").unwrap_or("1").parse().unwrap();
                req_c.lock().unwrap().push(pn);
                match pn {
                    1 => r#"{"code":0,"data":{"has_more":true,"medias":[
                        {"id":10,"bvid":"BV1p1000000010","title":"P1","type":2,"attr":0,"upper":{"mid":7,"name":"U7"}}
                    ]}}"#
                        .to_owned(),
                    2 => {
                        if fail_c.load(std::sync::atomic::Ordering::SeqCst) {
                            r#"{"code":-400,"message":"boom","data":null}"#.to_owned()
                        } else {
                            r#"{"code":0,"data":{"has_more":true,"medias":[
                                {"id":20,"bvid":"BV1p2000000020","title":"P2","type":2,"attr":0,"upper":{"mid":7,"name":"U7"}}
                            ]}}"#
                                .to_owned()
                        }
                    }
                    _ => r#"{"code":0,"data":{"has_more":false,"medias":[
                        {"id":30,"bvid":"BV1p3000000030","title":"P3","type":2,"attr":0,"upper":{"mid":7,"name":"U7"}}
                    ]}}"#
                        .to_owned(),
                }
            } else {
                r#"{"code":0,"data":{}}"#.to_owned()
            }
        })
        .await;

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
        db.upsert_sets([set::Model {
            set_id: 10,
            name: "BackfillSet".to_owned(),
            count: 3,
            state: SetState::Active.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: false,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_set_accounts([set_account::Model {
            set_id: 10,
            account_id: 1,
        }])
        .await
        .unwrap();

        let (_tmp, ctx) = test_ctx_with_cfg(base, Config::default(), db);

        // Run 1: page 1 succeeds, page 2 fails -> resume_page saved as Some(2), sync_complete=false!
        let err = fetch(
            &ctx,
            FetchOptions {
                set_ids: vec![10],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::PartialFailure);
        let row1 = ctx.db.get_set(10).await.unwrap();
        assert!(!row1.sync_complete);
        assert_eq!(row1.resume_page, Some(2));
        assert!(ctx.db.get_media(10).await.is_ok());

        // Run 2: page 2 now succeeds -> head check on page 1 (hits media 10), then resumes at page 2 and 3!
        requested.lock().unwrap().clear();
        fail_page_2.store(false, std::sync::atomic::Ordering::SeqCst);
        fetch(
            &ctx,
            FetchOptions {
                set_ids: vec![10],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(*requested.lock().unwrap(), vec![1, 2, 3]);
        let row2 = ctx.db.get_set(10).await.unwrap();
        assert!(row2.sync_complete);
        assert_eq!(row2.resume_page, None);
        assert!(ctx.db.get_media(20).await.is_ok());
        assert!(ctx.db.get_media(30).await.is_ok());
    }

    #[tokio::test]
    async fn collected_catalog_and_season_archives_sync_and_phase4_skips_errored_states() {
        let view_hits = Arc::new(Mutex::new(Vec::<i64>::new()));
        let view_c = view_hits.clone();

        let base = spawn_http(move |path| {
            if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket") {
                r#"{"code":0,"data":{"ticket":"t","created_at":4000000000,"ttl":86400}}"#.to_owned()
            } else if path.starts_with("/x/web-frontend/getbuvid") {
                r#"{"code":0,"data":{"buvid":"B1"}}"#.to_owned()
            } else if path.starts_with("/x/web-interface/nav") {
                r#"{"code":0,"data":{"isLogin":true,"mid":1,"uname":"Alice","wbi_img":{"img_url":"https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png","sub_url":"https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png"}}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/folder/created/list-all") {
                r#"{"code":0,"data":{"list":[{"id":1,"title":"Created","media_count":1}]}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/folder/collected/list") {
                r#"{"code":0,"data":{"count":3,"has_more":false,"list":[
                    {"id":2,"mid":200,"title":"CollectedFolder","media_count":1,"type":11,"attr":22,"state":0,"upper":{"mid":200,"name":"FolderOwner"}},
                    {"id":3,"mid":300,"title":"CollectedSeason","media_count":1,"type":21,"attr":0,"state":0,"upper":{"mid":300,"name":"SeasonOwner"}},
                    {"id":4,"mid":400,"title":"DeadSeason","media_count":0,"type":21,"attr":0,"state":1}
                ]}}"#.to_owned()
            } else if path.starts_with("/x/relation/stat") {
                r#"{"code":0,"data":{"following":0}}"#.to_owned()
            } else if path.starts_with("/x/polymer/web-space/seasons_archives_list") {
                r#"{"code":0,"data":{"archives":[{"aid":700,"bvid":"BV1season00700","title":"Season Ep1"}],"page":{"page_num":1,"page_size":30,"total":1}}}"#.to_owned()
            } else if path.starts_with("/x/web-interface/wbi/view") {
                let aid: i64 = query_param(path, "aid").unwrap_or("0").parse().unwrap();
                view_c.lock().unwrap().push(aid);
                format!(r#"{{"code":0,"data":{{"aid":{aid},"bvid":"BV1view{aid:07}","title":"V","owner":{{"mid":88,"name":"Up88"}},"pages":[]}}}}"#)
            } else {
                r#"{"code":0,"data":{}}"#.to_owned()
            }
        })
        .await;

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

        let (_tmp, ctx) = test_ctx_with_cfg(base, Config::default(), db);

        // Catalog sync discovers Created (Inactive), Collected (Inactive), Season (Inactive), DeadSeason (Unreachable).
        fetch(
            &ctx,
            FetchOptions {
                sets_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let s1 = ctx.db.get_set(1).await.unwrap();
        assert_eq!(s1.kind, SetKind::Created.as_str());
        assert_eq!(s1.state, SetState::Inactive.as_str());

        let s2 = ctx.db.get_set(2).await.unwrap();
        assert_eq!(s2.kind, SetKind::Collected.as_str());
        assert_eq!(s2.state, SetState::Inactive.as_str());
        assert_eq!(s2.owner_mid, Some(200));

        let s3 = ctx.db.get_set(3).await.unwrap();
        assert_eq!(s3.kind, SetKind::Season.as_str());
        assert_eq!(s3.state, SetState::Inactive.as_str());
        assert_eq!(s3.owner_mid, Some(300));

        let s4 = ctx.db.get_set(4).await.unwrap();
        assert_eq!(s4.kind, SetKind::Season.as_str());
        assert_eq!(s4.state, SetState::Unreachable.as_str());

        // Season media 700 must NOT have been fetched while set 3 is Inactive.
        assert!(ctx.db.get_media(700).await.is_err());

        // Activate season 3 and fetch it -> populates media 700 linked to media_set(3) AND media_up(300)!
        ctx.db.activate_set(3).await.unwrap();

        // Also seed unlinked medias in various states inside active set 3 to verify Phase 4 filtering.
        ctx.db
            .upsert_medias([
                media::Model {
                    id: 701,
                    bv_id: "BV1pend0000701".to_owned(),
                    title: "PendingNoUp".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Pending.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
                media::Model {
                    id: 702,
                    bv_id: "BV1fail0000702".to_owned(),
                    title: "FailedNoUp".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Failed.to_string(),
                    retry_count: 1,
                    last_error: None,
                    updated_at: None,
                },
                media::Model {
                    id: 703,
                    bv_id: "BV1perm0000703".to_owned(),
                    title: "PermDeniedNoUp".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::PermissionDenied.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
                media::Model {
                    id: 704,
                    bv_id: "BV1exp00000704".to_owned(),
                    title: "ExpiredNoUp".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Expired.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
                media::Model {
                    id: 705,
                    bv_id: "BV1skip0000705".to_owned(),
                    title: "SkippedNoUp".to_owned(),
                    r#type: MediaType::Video.to_string(),
                    state: MediaState::Skipped.to_string(),
                    retry_count: 0,
                    last_error: None,
                    updated_at: None,
                },
            ])
            .await
            .unwrap();
        ctx.db
            .upsert_media_sets([
                media_set::Model { id: 701, set_id: 3 },
                media_set::Model { id: 702, set_id: 3 },
                media_set::Model { id: 703, set_id: 3 },
                media_set::Model { id: 704, set_id: 3 },
                media_set::Model { id: 705, set_id: 3 },
            ])
            .await
            .unwrap();

        fetch(
            &ctx,
            FetchOptions {
                set_ids: vec![3],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(ctx.db.get_media(700).await.is_ok());
        assert_eq!(ctx.db.get_up_ids_of_media(700).await.unwrap(), vec![300]);
        assert!(ctx.db.get_set(3).await.unwrap().sync_complete);

        // Phase 4 must ONLY have queried /view for Pending media 701 (NOT Failed/PermissionDenied/Expired/Skipped, and NOT 700 which already got media_up=300)!
        assert_eq!(*view_hits.lock().unwrap(), vec![701]);
    }

    #[tokio::test]
    async fn collected_catalog_partial_failure_preserves_links_and_invalidated_preserves_completed()
    {
        let base = spawn_http(move |path| {
            if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket") {
                r#"{"code":0,"data":{"ticket":"t","created_at":4000000000,"ttl":86400}}"#.to_owned()
            } else if path.starts_with("/x/web-frontend/getbuvid") {
                r#"{"code":0,"data":{"buvid":"B1"}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/folder/created/list-all") {
                r#"{"code":0,"data":{"list":[{"id":1,"title":"Created","media_count":0}]}}"#
                    .to_owned()
            } else if path.starts_with("/x/v3/fav/folder/collected/list") {
                let pn: i64 = query_param(path, "pn").unwrap_or("1").parse().unwrap();
                if pn == 1 {
                    r#"{"code":0,"data":{"count":40,"has_more":true,"list":[
                        {"id":2,"mid":200,"title":"该合集已失效","media_count":0,"type":21,"state":0}
                    ]}}"#
                    .to_owned()
                } else {
                    r#"{"code":-400,"message":"page 2 boom","data":null}"#.to_owned()
                }
            } else if path.starts_with("/x/relation/stat") {
                r#"{"code":0,"data":{"following":0}}"#.to_owned()
            } else {
                r#"{"code":0,"data":{}}"#.to_owned()
            }
        })
        .await;

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
        // Existing Active season 2 with a Completed media, plus set 3 living on failing page 2.
        db.upsert_sets([
            set::Model {
                set_id: 2,
                name: "WasActiveSeason".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Season.to_string(),
                owner_mid: Some(200),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 3,
                name: "Page2Set".to_owned(),
                count: 1,
                state: SetState::Inactive.to_string(),
                kind: SetKind::Collected.to_string(),
                owner_mid: Some(300),
                sync_complete: false,
                resume_page: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_set_accounts([
            set_account::Model {
                set_id: 2,
                account_id: 1,
            },
            set_account::Model {
                set_id: 3,
                account_id: 1,
            },
        ])
        .await
        .unwrap();
        db.upsert_medias([media::Model {
            id: 555,
            bv_id: "BV1done0000555".to_owned(),
            title: "Saved Episode".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Completed.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        }])
        .await
        .unwrap();
        db.upsert_media_sets([media_set::Model { id: 555, set_id: 2 }])
            .await
            .unwrap();

        let (_tmp, ctx) = test_ctx_with_cfg(base, Config::default(), db);
        let err = fetch(
            &ctx,
            FetchOptions {
                sets_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::PartialFailure);

        // Set 3 link MUST survive because collected page 2 errored (`unlink_stale = false`).
        let acct_sets = ctx.db.get_set_ids_of_account(1).await.unwrap();
        assert!(acct_sets.contains(&3));

        // Previously Active set 2 became invalidated remotely -> transitions to Unreachable, Completed media 555 intact!
        assert_eq!(
            ctx.db.get_set(2).await.unwrap().state,
            SetState::Unreachable.as_str()
        );
        assert_eq!(
            ctx.db.get_media(555).await.unwrap().state,
            MediaState::Completed.as_str()
        );
    }

    #[tokio::test]
    async fn up_incremental_cursor_and_resumable_backfill_work_end_to_end() {
        let fail_page_2 = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let fail_c = fail_page_2.clone();
        let requested = Arc::new(Mutex::new(Vec::<i64>::new()));
        let req_c = requested.clone();
        let include_new_top = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let new_top_c = include_new_top.clone();

        let base = spawn_http(move |path| {
            if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket") {
                r#"{"code":0,"data":{"ticket":"t","created_at":4000000000,"ttl":86400}}"#.to_owned()
            } else if path.starts_with("/x/web-frontend/getbuvid") {
                r#"{"code":0,"data":{"buvid":"B1"}}"#.to_owned()
            } else if path.starts_with("/x/web-interface/nav") {
                r#"{"code":0,"data":{"isLogin":true,"mid":1,"uname":"Alice","wbi_img":{"img_url":"https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png","sub_url":"https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png"}}}"#.to_owned()
            } else if path.starts_with("/x/space/wbi/arc/search") {
                let pn: i64 = query_param(path, "pn").unwrap_or("1").parse().unwrap();
                req_c.lock().unwrap().push(pn);
                match pn {
                    1 => {
                        let mut vlist = Vec::new();
                        if new_top_c.load(std::sync::atomic::Ordering::SeqCst) {
                            vlist.push(
                                r#"{"aid":999,"bvid":"BV1upnew000999","title":"NewTop"}"#
                                    .to_owned(),
                            );
                        }
                        for i in 1..=30 {
                            if vlist.len() == 30 {
                                break;
                            }
                            vlist.push(format!(
                                r#"{{"aid":{},"bvid":"BV1upp1{:07}","title":"P1_{i}"}}"#,
                                100 + i,
                                i
                            ));
                        }
                        format!(
                            r#"{{"code":0,"data":{{"list":{{"vlist":[{}]}},"page":{{"pn":1,"ps":30,"count":45}}}}}}"#,
                            vlist.join(",")
                        )
                    }
                    _ => {
                        if fail_c.load(std::sync::atomic::Ordering::SeqCst) {
                            r#"{"code":-400,"message":"up page 2 boom","data":null}"#.to_owned()
                        } else {
                            let vlist: Vec<String> = (1..=15)
                                .map(|i| {
                                    format!(
                                        r#"{{"aid":{},"bvid":"BV1upp2{:07}","title":"P2_{i}"}}"#,
                                        200 + i,
                                        i
                                    )
                                })
                                .collect();
                            format!(
                                r#"{{"code":0,"data":{{"list":{{"vlist":[{}]}},"page":{{"pn":2,"ps":30,"count":45}}}}}}"#,
                                vlist.join(",")
                            )
                        }
                    }
                }
            } else {
                r#"{"code":0,"data":{}}"#.to_owned()
            }
        })
        .await;

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
        db.upsert_ups([up::Model {
            up_id: 77,
            name: "Up77".to_owned(),
            state: UpState::Active.to_string(),
            sync_complete: false,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_up_accounts([up_account::Model {
            up_id: 77,
            account_id: 1,
        }])
        .await
        .unwrap();

        let (_tmp, ctx) = test_ctx_with_cfg(base, Config::default(), db);

        // Run 1: page 1 succeeds (30 items), page 2 fails -> resume_page=Some(2), sync_complete=false!
        let err = fetch(
            &ctx,
            FetchOptions {
                up_ids: vec![77],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::PartialFailure);
        let u1 = ctx.db.get_up(77).await.unwrap();
        assert!(!u1.sync_complete);
        assert_eq!(u1.resume_page, Some(2));

        // Run 2: page 2 succeeds -> head check on page 1 + resume page 2 -> sync_complete=true!
        requested.lock().unwrap().clear();
        fail_page_2.store(false, std::sync::atomic::Ordering::SeqCst);
        fetch(
            &ctx,
            FetchOptions {
                up_ids: vec![77],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(*requested.lock().unwrap(), vec![1, 2]);
        let u2 = ctx.db.get_up(77).await.unwrap();
        assert!(u2.sync_complete);
        assert_eq!(u2.resume_page, None);
        assert!(ctx.db.get_media(215).await.is_ok());

        // Run 3: incremental fetch with 1 new video on page 1 -> stops after 1 page (NEVER requests page 2)!
        requested.lock().unwrap().clear();
        include_new_top.store(true, std::sync::atomic::Ordering::SeqCst);
        fetch(
            &ctx,
            FetchOptions {
                up_ids: vec![77],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(*requested.lock().unwrap(), vec![1]);
        assert!(ctx.db.get_media(999).await.is_ok());
    }

    #[tokio::test]
    async fn fetch_hint_respects_active_pending_scope() {
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
        db.upsert_sets([
            set::Model {
                set_id: 10,
                name: "ActiveSet".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 20,
                name: "InactiveSet".to_owned(),
                count: 1,
                state: SetState::Inactive.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
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
                account_id: 1,
            },
        ])
        .await
        .unwrap();
        db.upsert_medias([
            media::Model {
                id: 1,
                bv_id: "BV17x411w7KC".to_owned(),
                title: "ActiveCompleted".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 2,
                bv_id: "BV1Q541167Qg".to_owned(),
                title: "InactivePending".to_owned(),
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
            media_set::Model { id: 1, set_id: 10 },
            media_set::Model { id: 2, set_id: 20 },
        ])
        .await
        .unwrap();

        let (_tmp, ctx) = test_ctx_with_cfg(
            Url::parse("http://127.0.0.1:1").unwrap(),
            Config::default(),
            db,
        );

        // Only inactive media is Pending -> must NOT tell user to run `fav pull`!
        let inactive_hint = build_fetch_hint(&ctx)
            .await
            .expect("expected inactive hint");
        assert!(
            !inactive_hint.contains("fav pull"),
            "inactive-only pending medias must not suggest fav pull: {inactive_hint}"
        );
        assert!(inactive_hint.contains("1 pending media(s) belong to inactive sets/UPs"));

        // Activate set 20 -> now 1 active pending media -> suggests `fav pull`!
        ctx.db.activate_set(20).await.unwrap();
        let active_hint = build_fetch_hint(&ctx).await.expect("expected active hint");
        assert!(
            active_hint.contains("Run `fav pull` to download 1 active pending media(s)"),
            "unexpected active hint: {active_hint}"
        );
    }

    #[tokio::test]
    async fn up_incremental_cursor_ignores_phase2_favorite_set_media_up_links() {
        let set_pages_requested = Arc::new(Mutex::new(Vec::<i64>::new()));
        let set_pages_c = set_pages_requested.clone();
        let up_pages_requested = Arc::new(Mutex::new(Vec::<i64>::new()));
        let up_pages_c = up_pages_requested.clone();

        let base = spawn_http(move |path| {
            if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket") {
                r#"{"code":0,"data":{"ticket":"t","created_at":4000000000,"ttl":86400}}"#.to_owned()
            } else if path.starts_with("/x/web-frontend/getbuvid") {
                r#"{"code":0,"data":{"buvid":"B1"}}"#.to_owned()
            } else if path.starts_with("/x/web-interface/nav") {
                r#"{"code":0,"data":{"isLogin":true,"mid":1,"uname":"Alice","wbi_img":{"img_url":"https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png","sub_url":"https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png"}}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/folder/created/list-all") {
                r#"{"code":0,"data":{"list":[{"id":10,"title":"Fav","media_count":1}]}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/folder/collected/list") {
                r#"{"code":0,"data":{"count":0,"list":[],"has_more":false}}"#.to_owned()
            } else if path.starts_with("/x/relation/stat") {
                r#"{"code":0,"data":{"following":1}}"#.to_owned()
            } else if path.starts_with("/x/relation/followings") {
                r#"{"code":0,"data":{"list":[{"mid":77,"uname":"Up77"}],"total":1}}"#.to_owned()
            } else if path.starts_with("/x/v3/fav/resource/list") {
                let pn: i64 = query_param(path, "pn").unwrap_or("1").parse().unwrap();
                set_pages_c.lock().unwrap().push(pn);
                // User favorited V5 (aid=105) from Up77 into Active set 10.
                r#"{"code":0,"data":{"has_more":false,"medias":[
                    {"id":105,"bvid":"BV1up000000105","title":"V5","type":2,"attr":0,"upper":{"mid":77,"name":"Up77"}}
                ]}}"#
                    .to_owned()
            } else if path.starts_with("/x/space/wbi/arc/search") {
                let pn: i64 = query_param(path, "pn").unwrap_or("1").parse().unwrap();
                up_pages_c.lock().unwrap().push(pn);
                match pn {
                    1 => {
                        // Full 30-item page (count=60 so reached_end=false):
                        // [V5(105), V4(104), V3(103), V2(102), V1(101), older 50..74]
                        let mut vlist = vec![
                            r#"{"aid":105,"bvid":"BV1up000000105","title":"V5"}"#.to_owned(),
                            r#"{"aid":104,"bvid":"BV1up000000104","title":"V4"}"#.to_owned(),
                            r#"{"aid":103,"bvid":"BV1up000000103","title":"V3"}"#.to_owned(),
                            r#"{"aid":102,"bvid":"BV1up000000102","title":"V2"}"#.to_owned(),
                            r#"{"aid":101,"bvid":"BV1up000000101","title":"V1"}"#.to_owned(),
                        ];
                        for aid in 50..75 {
                            vlist.push(format!(
                                r#"{{"aid":{aid},"bvid":"BV1old{aid:07}","title":"Older_{aid}"}}"#
                            ));
                        }
                        format!(
                            r#"{{"code":0,"data":{{"list":{{"vlist":[{}]}},"page":{{"pn":1,"ps":30,"count":60}}}}}}"#,
                            vlist.join(",")
                        )
                    }
                    _ => r#"{"code":0,"data":{"list":{"vlist":[{"aid":1,"bvid":"BV1p2000000001","title":"Page2"}]},"page":{"pn":2,"ps":30,"count":60}}}"#.to_owned(),
                }
            } else {
                r#"{"code":0,"data":{}}"#.to_owned()
            }
        })
        .await;

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
        db.upsert_sets([set::Model {
            set_id: 10,
            name: "Fav".to_owned(),
            count: 1,
            state: SetState::Active.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: true,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_set_accounts([set_account::Model {
            set_id: 10,
            account_id: 1,
        }])
        .await
        .unwrap();
        db.upsert_ups([up::Model {
            up_id: 77,
            name: "Up77".to_owned(),
            state: UpState::Active.to_string(),
            sync_complete: true,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_up_accounts([up_account::Model {
            up_id: 77,
            account_id: 1,
        }])
        .await
        .unwrap();

        // Active UP 77 already has V1 (101) and V2 (102) synced from its UP feed.
        db.upsert_medias([
            media::Model {
                id: 101,
                bv_id: "BV1up000000101".to_owned(),
                title: "V1".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 102,
                bv_id: "BV1up000000102".to_owned(),
                title: "V2".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_up_feed_media_ups([
            media_up::Model {
                id: 101,
                up_id: 77,
                from_up_feed: true,
            },
            media_up::Model {
                id: 102,
                up_id: 77,
                from_up_feed: true,
            },
        ])
        .await
        .unwrap();

        let (_tmp, ctx) = test_ctx_with_cfg(base, Config::default(), db);

        // Full `fav fetch`: Phase 2 discovers V5 via Active set 10 (from_up_feed = false),
        // then Phase 3 scans Up77's feed and must NOT stop at V5!
        fetch(&ctx, FetchOptions::default()).await.unwrap();

        assert_eq!(*set_pages_requested.lock().unwrap(), vec![1]);
        assert_eq!(
            *up_pages_requested.lock().unwrap(),
            vec![1],
            "Phase 3 must stop at V2 on page 1 and never fetch page 2"
        );
        assert_eq!(ctx.db.get_media_ids_of_set(10).await.unwrap(), vec![105]);

        // V5, V4, and V3 must all be fetched; older filler item 50 after V2 must NOT be fetched.
        assert!(ctx.db.get_media(105).await.is_ok(), "V5 must exist");
        assert!(ctx.db.get_media(104).await.is_ok(), "V4 must be fetched");
        assert!(ctx.db.get_media(103).await.is_ok(), "V3 must be fetched");
        assert!(
            ctx.db.get_media(50).await.is_err(),
            "Phase 3 must stop at V2 and skip older items after V2"
        );

        // V5 must be upgraded from false -> true, and V4/V3 inserted with from_up_feed = true.
        let mut feed_ids = ctx.db.get_up_feed_media_ids(77).await.unwrap();
        feed_ids.sort_unstable();
        assert_eq!(feed_ids, vec![101, 102, 103, 104, 105]);

        // Subsequent incremental run immediately stops at V5 on page 1.
        set_pages_requested.lock().unwrap().clear();
        up_pages_requested.lock().unwrap().clear();
        fetch(&ctx, FetchOptions::default()).await.unwrap();
        assert_eq!(*set_pages_requested.lock().unwrap(), vec![1]);
        assert_eq!(*up_pages_requested.lock().unwrap(), vec![1]);
        assert!(ctx.db.get_media(50).await.is_err());
    }

    #[tokio::test]
    async fn purge_completed_requires_effective_prune() {
        let hits = Arc::new(Mutex::new(Vec::<String>::new()));
        let hits_c = hits.clone();
        let base = spawn_http(move |path| {
            hits_c.lock().unwrap().push(path.to_owned());
            r#"{"code":0,"data":{}}"#.to_owned()
        })
        .await;

        let make_db_with_account = || async {
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
            db
        };

        // 1. Neither `--prune` nor `config.prune = true` -> rejected with Usage before any HTTP calls.
        let (_tmp1, ctx_default) = test_ctx_with_cfg(
            base.clone(),
            Config::default(),
            make_db_with_account().await,
        );
        let err = fetch(
            &ctx_default,
            FetchOptions {
                purge_completed: true,
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
        assert!(err.to_string().contains("--prune"));
        assert!(err.to_string().contains("prune = true"));

        // 2. Explicit `config.prune = Some(false)` without `--prune` -> rejected with Usage.
        let (_tmp2, ctx_false) = test_ctx_with_cfg(
            base.clone(),
            Config {
                prune: Some(false),
                ..Default::default()
            },
            make_db_with_account().await,
        );
        let err = fetch(
            &ctx_false,
            FetchOptions {
                purge_completed: true,
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
        assert!(hits.lock().unwrap().is_empty());

        // 3. CLI `--prune` + `--purge-completed` -> accepted.
        fetch(
            &ctx_default,
            FetchOptions {
                prune: true,
                purge_completed: true,
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // 4. `config.prune = Some(true)` + `--purge-completed` (CLI `prune = false`) -> accepted.
        let (_tmp3, ctx_cfg_prune) = test_ctx_with_cfg(
            base,
            Config {
                prune: Some(true),
                ..Default::default()
            },
            make_db_with_account().await,
        );
        fetch(
            &ctx_cfg_prune,
            FetchOptions {
                purge_completed: true,
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
}
