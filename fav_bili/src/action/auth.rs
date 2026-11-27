use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use anyhow::{Context as _, Result, anyhow};
use console::Term;
use qrcode::{QrCode, render::unicode};
use serde::Serialize;
use tracing::{error, info, warn};

use crate::{
    api::{BiliApiError, BiliClient},
    context::AppContext,
    cookies::{
        extract_csrf, format_cookie_header, parse_cookie_header, parse_netscape_cookies,
        validate_sessdata,
    },
    entity::account,
    error::{AppError, AppResult},
    payload::{ConfirmRefreshPayload, CookieRefreshPayload},
    state::AccountState,
    table::{
        OutputFormat, TableOptions, format_state_badge, render_ids, render_json, render_jsonl,
        render_table, render_tsv, write_stdout_line,
    },
};

use super::fetch::{FetchOptions, fetch};

#[derive(Debug, Clone, Default)]
pub struct LoginOptions {
    pub invert_qr: bool,
    pub show_url: bool,
    pub fetch: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UseCookiesOptions {
    pub cookies: Vec<String>,
    pub stdin: bool,
    pub file: Option<PathBuf>,
    pub fetch: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LogoutOptions {
    pub account_ids: Vec<i64>,
    pub all: bool,
    pub local_only: bool,
    pub force: bool,
    pub yes: bool,
}

#[derive(Debug, Clone)]
pub struct CheckOptions {
    pub account_ids: Vec<i64>,
    pub all: bool,
    pub format: OutputFormat,
    pub wide: bool,
    pub refresh: bool,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            account_ids: Vec::new(),
            all: false,
            format: OutputFormat::Table,
            wide: false,
            refresh: false,
        }
    }
}

#[derive(Debug, Serialize)]
struct CheckRow {
    account_id: i64,
    name: String,
    state: String,
    valid: bool,
    has_csrf: bool,
    has_refresh_token: bool,
    refreshed: bool,
    message: String,
}

pub async fn login(ctx: &AppContext, opts: LoginOptions) -> AppResult<()> {
    let client = ctx.anonymous_client()?;
    let qr = client
        .qr_generate()
        .await
        .context("Failed to generate Bilibili login QR code")?;

    let code = QrCode::new(qr.url.as_ref()).context("Failed to render login QR code")?;
    let (dark, light) = if opts.invert_qr {
        (unicode::Dense1x2::Dark, unicode::Dense1x2::Light)
    } else {
        (unicode::Dense1x2::Light, unicode::Dense1x2::Dark)
    };
    let image = code
        .render::<unicode::Dense1x2>()
        .dark_color(dark)
        .light_color(light)
        .build();

    eprintln!("Scan the QR code below with the Bilibili mobile app (Ctrl-C to cancel):");
    eprintln!("{image}");
    if opts.show_url {
        eprintln!("Login URL: {}", qr.url);
    }

    let mut scanned_notified = false;
    let refresh_token = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                return Err(AppError::runtime(anyhow!("QR login cancelled by user.")));
            }
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
        }

        let poll = client
            .qr_poll(&qr.qrcode_key)
            .await
            .context("Failed to poll Bilibili QR login status")?;

        match poll.code {
            0 => {
                info!("QR login confirmed.");
                break poll.refresh_token;
            }
            86101 => {}
            86090 => {
                if !scanned_notified {
                    eprintln!("QR code scanned — please confirm login on your phone.");
                    scanned_notified = true;
                }
            }
            86038 => {
                return Err(AppError::auth(anyhow!(
                    "QR code expired ({}). Re-run `fav auth login` to generate a new code.",
                    poll.message
                )));
            }
            other => {
                return Err(AppError::auth(anyhow!(
                    "QR login failed (code {other}): {}",
                    poll.message
                )));
            }
        }
    };

    let info = client
        .nav_info()
        .await
        .context("Failed to verify newly logged-in Bilibili account")?;
    if !info.is_login {
        return Err(AppError::auth(anyhow!(
            "Bilibili login succeeded but session verification returned isLogin=false."
        )));
    }
    let mid = info
        .mid
        .ok_or_else(|| AppError::auth(anyhow!("Bilibili nav response missing account mid.")))?;
    let uname = info.uname.unwrap_or_else(|| format!("uid_{mid}"));

    if let Err(e) = client.ensure_anti_bot_cookies().await {
        warn!("Could not bootstrap anti-bot cookies after login: {e:#}");
    }

    let cookies = client.export_cookies();
    ctx.db
        .upsert_account(account::Model {
            account_id: mid,
            name: uname.clone(),
            cookies,
            state: AccountState::Active.to_string(),
            refresh_token,
        })
        .await?;

    info!("Logged in as {uname} ({mid}).");
    if !ctx.runtime.quiet {
        eprintln!("Logged in as {uname} ({mid}).");
    }

    if opts.fetch {
        fetch(ctx, FetchOptions::default()).await?;
    } else {
        ctx.hint("Run `fav fetch` to load your favorite sets and followed UPs.");
    }
    Ok(())
}

pub async fn use_cookies(ctx: &AppContext, opts: UseCookiesOptions) -> AppResult<()> {
    let cookie_maps = collect_cookie_inputs(ctx, &opts).await?;
    if cookie_maps.is_empty() {
        return Err(AppError::usage(anyhow!(
            "No cookies provided. Pass --stdin, --file <PATH>, set FAV_COOKIES, or run interactively in a TTY."
        )));
    }

    let mut errors = Vec::new();
    let mut validation_errors = 0usize;
    let mut imported = 0usize;

    for (idx, map) in cookie_maps.into_iter().enumerate() {
        if let Err(e) = validate_sessdata(&map) {
            let msg = format!("Invalid cookies for entry #{}: {e:#}", idx + 1);
            error!("{msg}");
            errors.push(msg);
            validation_errors += 1;
            continue;
        }

        let cookie_header = format_cookie_header(&map);
        let client = ctx.client_for_account(&cookie_header)?;
        let nav = match client.nav_info().await {
            Ok(nav) => nav,
            Err(e) => {
                error!("Failed verifying cookies for entry #{}: {e:#}", idx + 1);
                errors.push(format!("entry #{}: {e:#}", idx + 1));
                continue;
            }
        };

        if !nav.is_login {
            let msg = format!(
                "Cookies for entry #{} are invalid or expired (Bilibili returned isLogin=false).",
                idx + 1
            );
            error!("{msg}");
            errors.push(msg);
            continue;
        }

        let Some(mid) = nav.mid else {
            let msg = format!(
                "Bilibili nav response missing account mid for entry #{}.",
                idx + 1
            );
            error!("{msg}");
            errors.push(msg);
            continue;
        };
        let uname = nav.uname.unwrap_or_else(|| format!("uid_{mid}"));

        if let Err(e) = client.ensure_anti_bot_cookies().await {
            warn!("Could not bootstrap anti-bot cookies for {uname}: {e:#}");
        }

        let final_cookies = client.export_cookies();
        let existing_refresh_token = ctx
            .db
            .get_account(mid)
            .await
            .ok()
            .and_then(|acct| acct.refresh_token);
        ctx.db
            .upsert_account(account::Model {
                account_id: mid,
                name: uname.clone(),
                cookies: final_cookies,
                state: AccountState::Active.to_string(),
                refresh_token: existing_refresh_token,
            })
            .await?;

        imported += 1;
        info!("Imported cookies for {uname} ({mid}).");
        if !ctx.runtime.quiet {
            eprintln!("Authenticated as {uname} ({mid}).");
        }
    }

    if imported == 0 && !errors.is_empty() {
        if validation_errors == errors.len() {
            return Err(AppError::usage(anyhow!(
                "Invalid cookies provided: {}",
                errors.join("; ")
            )));
        }
        return Err(AppError::auth(anyhow!(
            "Failed to authenticate with provided cookies: {}",
            errors.join("; ")
        )));
    }
    if !errors.is_empty() {
        return Err(AppError::partial(anyhow!(
            "Imported {imported} account(s), {} failed: {}",
            errors.len(),
            errors.join("; ")
        )));
    }

    if opts.fetch {
        fetch(ctx, FetchOptions::default()).await?;
    } else {
        ctx.hint("Run `fav fetch` to load your favorite sets and followed UPs.");
    }
    Ok(())
}

pub async fn logout(ctx: &AppContext, opts: LogoutOptions) -> AppResult<()> {
    let accounts = if opts.all {
        let all = ctx.db.all_accounts().await?;
        if all.is_empty() {
            info!("No accounts to logout.");
            return Ok(());
        }
        if !opts.yes {
            let term = Term::stderr();
            if ctx.runtime.non_interactive || !term.is_term() {
                return Err(AppError::usage(anyhow!(
                    "Pass --yes to logout all accounts non-interactively."
                )));
            }
            eprint!("Logout all {} account(s)? [y/N] ", all.len());
            let answer = term
                .read_line()
                .context("Failed reading confirmation from terminal")?;
            let normalized = answer.trim().to_ascii_lowercase();
            if normalized != "y" && normalized != "yes" {
                eprintln!("Aborted.");
                return Ok(());
            }
        }
        all
    } else if !opts.account_ids.is_empty() {
        let mut list = Vec::with_capacity(opts.account_ids.len());
        for &id in &opts.account_ids {
            let acct = ctx.db.get_account(id).await.map_err(AppError::usage)?;
            list.push(acct);
        }
        list
    } else {
        return Err(AppError::usage(anyhow!(
            "Specify account ID(s) or pass --all to logout."
        )));
    };

    let mut failures = Vec::new();
    for acct in accounts {
        if opts.local_only {
            ctx.db.delete_account(acct.account_id).await?;
            info!(
                "Removed account {} ({}) locally.",
                acct.name, acct.account_id
            );
            continue;
        }

        let cookie_map = parse_cookie_header(&acct.cookies);
        let Some(csrf) = extract_csrf(&cookie_map) else {
            if opts.force {
                warn!(
                    "Account {} ({}) missing bili_jct; removing locally due to --force.",
                    acct.name, acct.account_id
                );
                ctx.db.delete_account(acct.account_id).await?;
            } else {
                let msg = format!(
                    "Account {} ({}) has no `bili_jct` cookie for remote logout; re-run with --local or --force to remove it locally.",
                    acct.name, acct.account_id
                );
                error!("{msg}");
                failures.push(msg);
            }
            continue;
        };

        let client = ctx.client_for_account(&acct.cookies)?;
        match client.logout(&csrf).await {
            Ok(()) => {
                ctx.db.delete_account(acct.account_id).await?;
                info!("Logged out {} ({}).", acct.name, acct.account_id);
            }
            Err(e) => {
                if opts.force {
                    warn!(
                        "Remote logout failed for {} ({}): {e:#}; removing locally due to --force.",
                        acct.name, acct.account_id
                    );
                    ctx.db.delete_account(acct.account_id).await?;
                } else {
                    let msg = format!(
                        "Failed remote logout for {} ({}): {e:#}. Re-run with --local or --force to remove locally.",
                        acct.name, acct.account_id
                    );
                    error!("{msg}");
                    failures.push(msg);
                }
            }
        }
    }

    if !failures.is_empty() {
        return Err(AppError::partial(anyhow!(
            "{} account(s) failed to logout",
            failures.len()
        )));
    }
    Ok(())
}

pub async fn check(ctx: &AppContext, opts: CheckOptions) -> AppResult<()> {
    let accounts = if opts.all || opts.account_ids.is_empty() {
        ctx.db.all_accounts().await?
    } else {
        let mut list = Vec::with_capacity(opts.account_ids.len());
        for &id in &opts.account_ids {
            let acct = ctx.db.get_account(id).await.map_err(AppError::usage)?;
            list.push(acct);
        }
        list
    };

    if accounts.is_empty() {
        return Err(AppError::usage(anyhow!(
            "No accounts found. Run `fav auth login` or `fav auth usecookies` first."
        )));
    }

    let mut rows = Vec::with_capacity(accounts.len());
    let mut invalid_count = 0usize;

    for acct in accounts {
        let cookie_map = parse_cookie_header(&acct.cookies);
        let has_csrf = extract_csrf(&cookie_map).is_some();
        let has_refresh_token = acct.refresh_token.is_some();
        let client = ctx.client_for_account(&acct.cookies)?;

        let mut row = CheckRow {
            account_id: acct.account_id,
            name: acct.name.clone(),
            state: acct.state.clone(),
            valid: false,
            has_csrf,
            has_refresh_token,
            refreshed: false,
            message: String::new(),
        };

        match client.nav_info().await {
            Ok(info) if info.is_login && info.mid == Some(acct.account_id) => {
                row.valid = true;
                row.message = "ok".to_string();

                let needs_refresh = if opts.refresh {
                    true
                } else if has_refresh_token {
                    client
                        .cookie_info()
                        .await
                        .map(|ci| ci.refresh)
                        .unwrap_or(false)
                } else {
                    false
                };

                if needs_refresh {
                    if let Some(ref rt) = acct.refresh_token {
                        match try_refresh_cookies(&client, ctx, &acct, rt).await {
                            Ok(true) => {
                                row.refreshed = true;
                                row.message = "ok (refreshed)".to_string();
                            }
                            Ok(false) => {
                                row.message = "ok (refresh skipped: missing bili_jct)".to_string();
                            }
                            Err(e) => {
                                warn!(
                                    "Automatic cookie refresh was rejected for {} ({}): {e:#}. \
                                     Bilibili web refresh may require RSA-OAEP correspondPath CSRF verification; \
                                     re-authenticate with `fav auth login` or `fav auth usecookies` when cookies expire.",
                                    acct.name, acct.account_id
                                );
                                row.message = format!(
                                    "ok (refresh failed: {e}; re-run `fav auth login` or `fav auth usecookies` when expired)"
                                );
                            }
                        }
                    } else if opts.refresh {
                        row.message =
                            "ok (no refresh_token stored; login via QR to enable refresh)"
                                .to_string();
                    }
                }
            }
            Ok(info) if info.is_login => {
                invalid_count += 1;
                row.message = format!(
                    "mismatched account id: expected {}, got {:?}",
                    acct.account_id, info.mid
                );
                let _ = ctx
                    .db
                    .set_account_state(acct.account_id, AccountState::Expired)
                    .await;
                row.state = AccountState::Expired.to_string();
            }
            Ok(_) => {
                invalid_count += 1;
                row.message = "cookies expired (isLogin=false)".to_string();
                let _ = ctx
                    .db
                    .set_account_state(acct.account_id, AccountState::Expired)
                    .await;
                row.state = AccountState::Expired.to_string();
            }
            Err(e) => {
                invalid_count += 1;
                if e.downcast_ref::<BiliApiError>()
                    .is_some_and(BiliApiError::is_auth_expired)
                {
                    let _ = ctx
                        .db
                        .set_account_state(acct.account_id, AccountState::Expired)
                        .await;
                    row.state = AccountState::Expired.to_string();
                }
                row.message = format!("{e:#}");
            }
        }

        rows.push(row);
    }

    render_check_rows(&rows, &opts, ctx)?;

    if invalid_count > 0 {
        return Err(AppError::auth(anyhow!(
            "{invalid_count} account(s) failed authentication check."
        )));
    }
    Ok(())
}

async fn try_refresh_cookies(
    client: &BiliClient,
    ctx: &AppContext,
    acct: &account::Model,
    refresh_token: &str,
) -> Result<bool> {
    let cookie_map = client.export_cookie_map();
    let Some(csrf) = extract_csrf(&cookie_map) else {
        return Ok(false);
    };
    let refreshed = client
        .cookie_refresh(&CookieRefreshPayload {
            csrf: csrf.clone(),
            refresh_csrf: String::new(),
            source: "main_web".to_string(),
            refresh_token: refresh_token.to_string(),
        })
        .await
        .context(
            "Bilibili rejected cookie refresh (RSA-OAEP correspondPath refresh_csrf may be required)",
        )?;

    let new_cookies_map = client.export_cookie_map();
    let new_csrf = extract_csrf(&new_cookies_map).unwrap_or(csrf);
    let _ = client
        .confirm_refresh(&ConfirmRefreshPayload {
            csrf: new_csrf,
            refresh_token: refresh_token.to_string(),
        })
        .await;

    let updated_cookies = client.export_cookies();
    ctx.db
        .update_account_cookies(acct.account_id, updated_cookies, refreshed.refresh_token)
        .await?;
    Ok(true)
}

fn render_check_rows(rows: &[CheckRow], opts: &CheckOptions, ctx: &AppContext) -> AppResult<()> {
    let rendered = match opts.format {
        OutputFormat::Table => {
            let table_opts = TableOptions::detect(opts.wide, ctx.runtime.color);
            let records = rows.iter().map(|r| {
                [
                    r.account_id.to_string(),
                    table_opts.format_cell(&r.name, 56),
                    format_state_badge(&r.state, table_opts.colored),
                    if r.valid { "yes" } else { "no" }.to_string(),
                    if r.has_csrf { "yes" } else { "no" }.to_string(),
                    table_opts.format_cell(&r.message, 56),
                ]
            });
            render_table(
                ["account_id", "name", "state", "valid", "csrf", "detail"],
                records,
                table_opts,
            )
        }
        OutputFormat::Json => render_json(rows)?,
        OutputFormat::Jsonl => render_jsonl(rows)?,
        OutputFormat::Tsv => render_tsv(
            ["account_id", "name", "state", "valid", "csrf", "detail"],
            rows.iter().map(|r| {
                [
                    r.account_id.to_string(),
                    r.name.clone(),
                    r.state.clone(),
                    r.valid.to_string(),
                    r.has_csrf.to_string(),
                    r.message.clone(),
                ]
            }),
        ),
        OutputFormat::Ids => render_ids(rows.iter().map(|r| r.account_id.to_string())),
    };
    if !rendered.is_empty() {
        write_stdout_line(&rendered).map_err(AppError::runtime)?;
    }
    Ok(())
}

async fn collect_cookie_inputs(
    ctx: &AppContext,
    opts: &UseCookiesOptions,
) -> AppResult<Vec<BTreeMap<String, String>>> {
    let mut results = Vec::new();
    let mut wants_stdin = opts.stdin;

    if let Some(ref path) = opts.file {
        let content = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read cookie file '{}'", path.display()))?;
        results.extend(parse_cookie_blob(&content));
    }

    let mut cli_args = Vec::new();
    for raw in &opts.cookies {
        if raw.trim() == "-" {
            wants_stdin = true;
        } else if !raw.trim().is_empty() {
            cli_args.push(raw.clone());
        }
    }

    if !cli_args.is_empty() {
        warn!(
            "Passing cookies as positional CLI arguments exposes secrets in shell history and process listings; prefer --stdin, --file, or FAV_COOKIES."
        );
        for raw in cli_args {
            results.push(parse_cookie_header(&raw));
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
        .context("Failed to read cookies from stdin")?;
        results.extend(parse_cookie_blob(&buf));
    }

    if results.is_empty() {
        if let Ok(env_cookies) = std::env::var("FAV_COOKIES")
            && !env_cookies.trim().is_empty()
        {
            results.extend(parse_cookie_blob(&env_cookies));
        } else {
            let term = Term::stderr();
            if !ctx.runtime.non_interactive && term.is_term() {
                eprint!("Paste Bilibili cookies (input hidden): ");
                let line = term
                    .read_secure_line()
                    .context("Failed reading hidden cookie line from terminal")?;
                if !line.trim().is_empty() {
                    results.push(parse_cookie_header(&line));
                }
            }
        }
    }

    Ok(results)
}

fn looks_like_netscape_cookies(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("# Netscape HTTP Cookie File")
            || trimmed.starts_with("#HttpOnly_")
            || (!trimmed.starts_with('#') && trimmed.split('\t').count() >= 7)
    })
}

fn parse_cookie_blob(content: &str) -> Vec<BTreeMap<String, String>> {
    if looks_like_netscape_cookies(content) {
        let map = parse_netscape_cookies(content);
        if map.is_empty() {
            Vec::new()
        } else {
            vec![map]
        }
    } else {
        content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(parse_cookie_header)
            .filter(|m| !m.is_empty())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
        error::ExitKind,
        muxer::test_support::MockMuxer,
    };

    async fn spawn_auth_mock_server() -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
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
                    let body = if path.starts_with("/x/web-interface/nav") {
                        if req.contains("SESSDATA=valid_sess") || req.contains("SESSDATA=new_sess")
                        {
                            r#"{"code":0,"data":{"isLogin":true,"mid":1001,"uname":"Alice"}}"#
                        } else if req.contains("SESSDATA=second_sess") {
                            r#"{"code":0,"data":{"isLogin":true,"mid":1002,"uname":"Bob"}}"#
                        } else {
                            r#"{"code":-101,"data":{"isLogin":false}}"#
                        }
                    } else if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket")
                    {
                        r#"{"code":0,"data":{"ticket":"t_fresh","created_at":4000000000,"ttl":259200}}"#
                    } else if path.starts_with("/x/web-frontend/getbuvid") {
                        r#"{"code":0,"data":{"buvid":"BUV-1"}}"#
                    } else {
                        r#"{"code":-404,"message":"not found","data":null}"#
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
        Url::parse(&format!("http://{addr}")).unwrap()
    }

    fn test_api_config(base: Url) -> BiliClientConfig {
        BiliClientConfig {
            api_base_url: base.clone(),
            passport_base_url: base,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
            api_request_timeout: Duration::from_secs(5),
            max_retries: 0,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
            min_request_interval: Duration::ZERO,
            wbi_ttl: Duration::from_secs(60),
        }
    }

    #[tokio::test]
    async fn use_cookies_preserves_existing_refresh_token() {
        let base = spawn_auth_mock_server().await;
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(account::Model {
            account_id: 1001,
            name: "AliceOld".to_owned(),
            cookies: "SESSDATA=old_sess".to_owned(),
            state: AccountState::Expired.to_string(),
            refresh_token: Some("rt_preserve_me".to_owned()),
        })
        .await
        .unwrap();

        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            test_api_config(base),
        );

        use_cookies(
            &ctx,
            UseCookiesOptions {
                cookies: vec![
                    "SESSDATA=new_sess; bili_jct=csrf_new".to_owned(),
                    "SESSDATA=second_sess; bili_jct=csrf_bob".to_owned(),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let alice = ctx.db.get_account(1001).await.unwrap();
        assert_eq!(alice.name, "Alice");
        assert_eq!(alice.state, AccountState::Active.to_string());
        assert_eq!(alice.refresh_token.as_deref(), Some("rt_preserve_me"));
        assert!(alice.cookies.contains("SESSDATA=new_sess"));

        let bob = ctx.db.get_account(1002).await.unwrap();
        assert_eq!(bob.name, "Bob");
        assert_eq!(bob.refresh_token, None);
    }

    #[tokio::test]
    async fn use_cookies_collects_missing_sessdata_errors_for_usage_and_partial() {
        let base = spawn_auth_mock_server().await;
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::new(tmp.path().join("ws"));
        ws.ensure_dirs().unwrap();
        let db = Db::open_in_memory().await.unwrap();
        let ctx = AppContext::for_test(
            ws,
            Config::default(),
            db,
            Arc::new(MockMuxer::succeeding()),
            test_api_config(base),
        );

        // All entries missing SESSDATA -> Usage error covering all entries.
        let err_usage = use_cookies(
            &ctx,
            UseCookiesOptions {
                cookies: vec![
                    "bili_jct=only_csrf".to_owned(),
                    "buvid3=only_buv".to_owned(),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_usage.kind(), ExitKind::Usage);
        let msg = format!("{err_usage:#}");
        assert!(msg.contains("entry #1"), "msg={msg}");
        assert!(msg.contains("entry #2"), "msg={msg}");
        assert!(ctx.db.all_accounts().await.unwrap().is_empty());

        // First entry missing SESSDATA, second valid -> continues and imports second, returns PartialFailure.
        let err_partial = use_cookies(
            &ctx,
            UseCookiesOptions {
                cookies: vec![
                    "bili_jct=missing_sess".to_owned(),
                    "SESSDATA=valid_sess; bili_jct=csrf_ok".to_owned(),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_partial.kind(), ExitKind::PartialFailure);
        let alice = ctx.db.get_account(1001).await.unwrap();
        assert_eq!(alice.name, "Alice");
    }
}
