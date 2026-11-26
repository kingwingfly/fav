use std::collections::HashSet;

use anyhow::{Context as _, anyhow};
use futures::StreamExt as _;
use tracing::{error, info, warn};

use crate::{
    api::BiliApiError,
    context::AppContext,
    cookies::extract_csrf,
    error::{AppError, AppResult},
    id_parse::parse_media_ident,
    state::AccountState,
};

#[derive(Debug, Clone, Default)]
pub struct LikeOptions {
    pub targets: Vec<String>,
    pub set_ids: Vec<i64>,
    pub all_active: bool,
    pub unlike: bool,
    pub account_ids: Vec<i64>,
    pub dry_run: bool,
    pub read_stdin: bool,
    pub ignore_errors: bool,
}

pub async fn like(ctx: &AppContext, opts: LikeOptions) -> AppResult<()> {
    let target_aids = collect_target_aids(ctx, &opts).await?;
    if target_aids.is_empty() {
        return Err(AppError::usage(anyhow!(
            "No target medias specified. Pass BV/av IDs, --set <ID>, --all-active, or pipe IDs via stdin (`-`)."
        )));
    }

    let accounts = if !opts.account_ids.is_empty() {
        let mut list = Vec::with_capacity(opts.account_ids.len());
        for &id in &opts.account_ids {
            let acct = ctx.db.get_account(id).await.map_err(AppError::usage)?;
            list.push(acct);
        }
        list
    } else {
        ctx.db.active_accounts().await?
    };

    if accounts.is_empty() {
        return Err(AppError::usage(anyhow!(
            "No active accounts available. Run `fav auth login` or `fav activate account` first."
        )));
    }

    let verb = if opts.unlike { "unlike" } else { "like" };
    if opts.dry_run {
        eprintln!(
            "[dry-run] Would {verb} {} media(s) across {} account(s):",
            target_aids.len(),
            accounts.len()
        );
        for aid in &target_aids {
            println!("{aid}");
        }
        return Ok(());
    }

    let api_concurrency = ctx.effective_api_concurrency();
    let mut succeeded = 0usize;
    let mut failures = Vec::new();
    let mut auth_expired_accounts = 0usize;

    for acct in &accounts {
        let client = ctx.client_for_account(&acct.cookies)?;
        let cookie_map = client.export_cookie_map();
        let Some(csrf) = extract_csrf(&cookie_map) else {
            let msg = format!(
                "Account {} ({}) is missing `bili_jct` cookie required to {verb}; re-import full browser cookies via `fav auth usecookies --stdin`.",
                acct.name, acct.account_id
            );
            error!("{msg}");
            failures.push(msg);
            continue;
        };

        match client.ensure_anti_bot_cookies().await {
            Ok(true) => {
                let updated = client.export_cookies();
                ctx.db
                    .update_account_cookies(acct.account_id, updated, None)
                    .await?;
            }
            Ok(false) => {}
            Err(e) => {
                warn!(
                    "Failed refreshing anti-bot cookies for {} ({}): {e:#}",
                    acct.name, acct.account_id
                );
            }
        }

        let like_flag = !opts.unlike;
        let mut tasks = futures::stream::iter(target_aids.iter().copied())
            .map(|aid| {
                let client = client.clone();
                let csrf = csrf.clone();
                async move { (aid, client.like_media(aid, like_flag, &csrf).await) }
            })
            .buffer_unordered(api_concurrency);

        let mut account_expired = false;
        while let Some((aid, res)) = tasks.next().await {
            match res {
                Ok(()) => {
                    succeeded += 1;
                    info!(
                        "{} media<{aid}> with account<{}>",
                        if opts.unlike { "Unliked" } else { "Liked" },
                        acct.name
                    );
                }
                Err(e) => {
                    if e.downcast_ref::<BiliApiError>()
                        .is_some_and(BiliApiError::is_auth_expired)
                        && !account_expired
                    {
                        account_expired = true;
                        auth_expired_accounts += 1;
                        let _ = ctx
                            .db
                            .set_account_state(acct.account_id, AccountState::Expired)
                            .await;
                    }
                    let msg = format!(
                        "Failed to {verb} media<{aid}> with account<{}>: {e:#}",
                        acct.name
                    );
                    error!("{msg}");
                    failures.push(msg);
                }
            }
        }
    }

    info!(
        "Finished {verb}: {succeeded} succeeded, {} failed.",
        failures.len()
    );

    if !failures.is_empty() && !opts.ignore_errors {
        if auth_expired_accounts == accounts.len() && succeeded == 0 {
            return Err(AppError::auth(anyhow!(
                "All selected accounts failed authentication during {verb}."
            )));
        }
        return Err(AppError::partial(anyhow!(
            "{} operation(s) failed during {verb} ({succeeded} succeeded).",
            failures.len()
        )));
    }
    Ok(())
}

async fn collect_target_aids(ctx: &AppContext, opts: &LikeOptions) -> AppResult<Vec<i64>> {
    let mut aids = Vec::new();
    let mut seen = HashSet::new();
    let mut wants_stdin = opts.read_stdin;

    for raw in &opts.targets {
        let trimmed = raw.trim();
        if trimmed == "-" {
            wants_stdin = true;
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let ident = parse_media_ident(trimmed).map_err(AppError::usage)?;
        let aid = ident
            .to_aid()
            .ok_or_else(|| AppError::usage(anyhow!("Cannot convert '{trimmed}' to numeric AID")))?;
        if seen.insert(aid) {
            aids.push(aid);
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
        .context("Failed reading target IDs from stdin")?;
        for token in buf.split_whitespace() {
            if token.starts_with('#') {
                continue;
            }
            let ident = parse_media_ident(token).map_err(AppError::usage)?;
            let aid = ident.to_aid().ok_or_else(|| {
                AppError::usage(anyhow!("Cannot convert '{token}' to numeric AID"))
            })?;
            if seen.insert(aid) {
                aids.push(aid);
            }
        }
    }

    for &set_id in &opts.set_ids {
        // Validate set exists so users get a clear error on typoed set IDs.
        let _ = ctx.db.get_set(set_id).await.map_err(AppError::usage)?;
        let ids = ctx.db.get_media_ids_of_set(set_id).await?;
        for aid in ids {
            if seen.insert(aid) {
                aids.push(aid);
            }
        }
    }

    if opts.all_active {
        let medias = ctx.db.all_active_medias().await?;
        for m in medias {
            if seen.insert(m.id) {
                aids.push(m.id);
            }
        }
    }

    Ok(aids)
}
