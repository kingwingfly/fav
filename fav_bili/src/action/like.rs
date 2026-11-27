use std::collections::HashSet;

use anyhow::{Context as _, anyhow};
use futures::StreamExt as _;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    api::BiliApiError,
    context::AppContext,
    cookies::extract_csrf,
    error::{AppError, AppResult},
    id_parse::parse_media_ident,
    state::{AccountState, SetState},
    table::write_stdout_line,
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
    // Eagerly validate explicitly specified --account IDs before reading stdin or making requests.
    let explicit_accounts = if !opts.account_ids.is_empty() {
        let mut list = Vec::with_capacity(opts.account_ids.len());
        let mut seen = HashSet::new();
        for &id in &opts.account_ids {
            if !seen.insert(id) {
                continue;
            }
            let acct = ctx.db.get_account(id).await.map_err(AppError::usage)?;
            if acct.state != AccountState::Active.as_str() {
                return Err(AppError::usage(anyhow!(
                    "Account<{id} {}> is {} (not Active). Run `fav activate account {id}` first.",
                    acct.name,
                    acct.state
                )));
            }
            list.push(acct);
        }
        Some(list)
    } else {
        None
    };

    let target_aids = collect_target_aids(ctx, &opts).await?;
    if target_aids.is_empty() {
        return Err(AppError::usage(anyhow!(
            "No target medias specified. Pass BV/av IDs, --set <ID>, --all-active, or pipe IDs via stdin (`-`)."
        )));
    }

    let accounts = match explicit_accounts {
        Some(list) => list,
        None => ctx.db.active_accounts().await?,
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
            if !write_stdout_line(&aid.to_string()).map_err(AppError::runtime)? {
                break;
            }
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
        let account_cancel = CancellationToken::new();
        let mut tasks = futures::stream::iter(target_aids.iter().copied())
            .map(|aid| {
                let client = client.clone();
                let csrf = csrf.clone();
                let cancel = account_cancel.clone();
                async move {
                    if cancel.is_cancelled() {
                        return (aid, None);
                    }
                    let res = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return (aid, None),
                        res = client.like_media(aid, like_flag, &csrf) => res,
                    };
                    (aid, Some(res))
                }
            })
            .buffer_unordered(api_concurrency);

        let mut account_expired = false;
        while let Some((aid, maybe_res)) = tasks.next().await {
            let Some(res) = maybe_res else {
                continue;
            };
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
                    {
                        account_cancel.cancel();
                        if !account_expired {
                            account_expired = true;
                            auth_expired_accounts += 1;
                            let _ = ctx
                                .db
                                .set_account_state(acct.account_id, AccountState::Expired)
                                .await;
                            warn!(
                                "Account {} ({}) session expired; cancelling remaining {verb} target(s) for this account.",
                                acct.name, acct.account_id
                            );
                        }
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
        for aid in parse_stdin_targets(&buf)? {
            if seen.insert(aid) {
                aids.push(aid);
            }
        }
    }

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
                "Set<{set_id} {}> does not belong to any active account. Run `fav activate account` first.",
                set_row.name
            )));
        }
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

fn parse_stdin_targets(buf: &str) -> AppResult<Vec<i64>> {
    let mut aids = Vec::new();
    let mut seen = HashSet::new();
    for line in buf.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        for token in trimmed.split_whitespace() {
            if token.starts_with('#') {
                break;
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
    Ok(aids)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
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
        entity::{account, media, media_set, set, set_account},
        error::ExitKind,
        muxer::test_support::MockMuxer,
        state::{MediaState, MediaType, SetKind},
    };

    async fn seeded_context() -> (tempfile::TempDir, AppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(account::Model {
            account_id: 1,
            name: "ActiveUser".to_owned(),
            cookies: "SESSDATA=ok; bili_jct=csrf".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 2,
            name: "InactiveUser".to_owned(),
            cookies: "SESSDATA=ok2; bili_jct=csrf2".to_owned(),
            state: AccountState::Inactive.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 3,
            name: "ExpiredUser".to_owned(),
            cookies: "SESSDATA=ok3; bili_jct=csrf3".to_owned(),
            state: AccountState::Expired.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();

        db.upsert_sets([
            set::Model {
                set_id: 10,
                name: "ActiveOwned".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 11,
                name: "InactiveOwned".to_owned(),
                count: 1,
                state: SetState::Inactive.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(1),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 12,
                name: "ActiveByInactiveAccount".to_owned(),
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
                set_id: 11,
                account_id: 1,
            },
            set_account::Model {
                set_id: 12,
                account_id: 2,
            },
        ])
        .await
        .unwrap();

        db.upsert_medias([media::Model {
            id: 100,
            bv_id: "BV17x411w7KC".to_owned(),
            title: "Video".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        }])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model {
                id: 100,
                set_id: 10,
            },
            media_set::Model {
                id: 100,
                set_id: 11,
            },
            media_set::Model {
                id: 100,
                set_id: 12,
            },
        ])
        .await
        .unwrap();

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
    async fn like_rejects_inactive_and_expired_accounts_and_inactive_or_unowned_sets() {
        let (_tmp, ctx) = seeded_context().await;

        let err_inactive_acct = like(
            &ctx,
            LikeOptions {
                targets: vec!["av100".to_owned()],
                account_ids: vec![2],
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_inactive_acct.kind(), ExitKind::Usage);

        let err_expired_acct = like(
            &ctx,
            LikeOptions {
                targets: vec!["av100".to_owned()],
                account_ids: vec![3],
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_expired_acct.kind(), ExitKind::Usage);

        let err_inactive_set = like(
            &ctx,
            LikeOptions {
                set_ids: vec![11],
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_inactive_set.kind(), ExitKind::Usage);

        let err_unowned_set = like(
            &ctx,
            LikeOptions {
                set_ids: vec![12],
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_unowned_set.kind(), ExitKind::Usage);

        like(
            &ctx,
            LikeOptions {
                set_ids: vec![10],
                account_ids: vec![1],
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .expect("active set owned by active account should succeed in dry-run");
    }

    #[test]
    fn parse_stdin_targets_handles_comments_blank_lines_and_dedup() {
        let input = "\
# full-line comment with multiple words
   # indented full-line comment

av100 # inline comment with multiple words
100 200 av200 # trailing inline comment
   \t   
BV17x411w7KC # classic test video (aid 170001)
av170001
";
        let aids = parse_stdin_targets(input).unwrap();
        assert_eq!(aids, vec![100, 200, 170001]);

        let err = parse_stdin_targets("av100\nnot-a-valid-id\n").unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
    }

    async fn spawn_like_mock_server<F>(handler: F) -> Url
    where
        F: Fn(&str, &str) -> String + Send + Sync + 'static,
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
                    let body = handler(path, &req);
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

    #[tokio::test]
    async fn like_cancels_remaining_targets_and_marks_expired_on_auth_error() {
        let acct1_calls = Arc::new(AtomicUsize::new(0));
        let acct2_calls = Arc::new(AtomicUsize::new(0));
        let c1 = acct1_calls.clone();
        let c2 = acct2_calls.clone();

        let base = spawn_like_mock_server(move |path, req| {
            if path.starts_with("/x/web-interface/archive/like") {
                if req.contains("SESSDATA=expired_tok") {
                    c1.fetch_add(1, Ordering::SeqCst);
                    return r#"{"code":-101,"message":"账号未登录"}"#.to_owned();
                }
                if req.contains("SESSDATA=valid_tok") {
                    c2.fetch_add(1, Ordering::SeqCst);
                    return r#"{"code":0,"message":"0"}"#.to_owned();
                }
            }
            r#"{"code":0,"message":"0","data":{}}"#.to_owned()
        })
        .await;

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().to_path_buf());
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(account::Model {
            account_id: 1,
            name: "ExpiringUser".to_owned(),
            cookies: "SESSDATA=expired_tok; bili_jct=csrf1; buvid3=b1; bili_ticket=t1; bili_ticket_expires=9999999999".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 2,
            name: "HealthyUser".to_owned(),
            cookies: "SESSDATA=valid_tok; bili_jct=csrf2; buvid3=b2; bili_ticket=t2; bili_ticket_expires=9999999999".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();

        let cfg = Config {
            api_concurrency: Some(1),
            ..Default::default()
        };
        let api_cfg = BiliClientConfig {
            api_base_url: base.clone(),
            passport_base_url: base,
            max_retries: 0,
            base_backoff: Duration::ZERO,
            min_request_interval: Duration::ZERO,
            ..Default::default()
        };
        let ctx = AppContext::for_test(ws, cfg, db, Arc::new(MockMuxer::succeeding()), api_cfg);

        // 1. Single expiring account: -101 stops after 1st target, marks Expired, returns Auth.
        let err = like(
            &ctx,
            LikeOptions {
                targets: vec![
                    "av101".into(),
                    "av102".into(),
                    "av103".into(),
                    "av104".into(),
                    "av105".into(),
                ],
                account_ids: vec![1],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Auth);
        assert_eq!(
            acct1_calls.load(Ordering::SeqCst),
            1,
            "expected remaining targets for expired account to be cancelled"
        );
        assert_eq!(
            ctx.db.get_account(1).await.unwrap().state,
            AccountState::Expired.as_str()
        );

        // 2. Re-activate account 1 and run across both accounts:
        //    Account 1 stops after 1 failure (cancelled targets not counted as extra failures),
        //    while Account 2 completes all 3 targets.
        ctx.db
            .set_account_state(1, AccountState::Active)
            .await
            .unwrap();
        acct1_calls.store(0, Ordering::SeqCst);

        let err2 = like(
            &ctx,
            LikeOptions {
                targets: vec!["av201".into(), "av202".into(), "av203".into()],
                account_ids: vec![1, 2],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err2.kind(), ExitKind::PartialFailure);
        assert!(
            err2.to_string().contains("1 operation(s) failed"),
            "cancelled targets must not be counted as extra failures, got: {err2}"
        );
        assert_eq!(acct1_calls.load(Ordering::SeqCst), 1);
        assert_eq!(acct2_calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            ctx.db.get_account(1).await.unwrap().state,
            AccountState::Expired.as_str()
        );
        assert_eq!(
            ctx.db.get_account(2).await.unwrap().state,
            AccountState::Active.as_str()
        );
    }
}
