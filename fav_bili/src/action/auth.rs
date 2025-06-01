use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use api_req::{ApiCaller as _, COOKIE_JAR, CookieStore as _};
use futures::StreamExt as _;
use qrcode::{QrCode, render::unicode};
use tokio::time::sleep;
use tracing::{error, info};

use crate::{
    api::{AuthApi, BiliApi},
    cookies::{parse_cookies, set_cookie_jar},
    db::db,
    entity::account,
    payload::{LogoutPayload, QrPayload, QrPollPayload, WbiPayload},
    response::{LogoutResp, QrData, QrPollData, QrPollResp, QrResp, WbiData, WbiResp},
    state::AccountState,
};

pub async fn login() -> Result<()> {
    let db = db().await;
    let QrResp {
        data: QrData { url, qrcode_key },
    } = AuthApi::request(QrPayload).await?;
    let code = QrCode::new(url.as_ref())?;
    let image = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build();
    println!("{}", image);
    loop {
        sleep(Duration::from_secs(3)).await;
        let QrPollResp {
            data: QrPollData { code, message },
        } = AuthApi::request(QrPollPayload {
            qrcode_key: qrcode_key.clone(),
        })
        .await?;
        match code {
            0 => {
                info!("Login successfully.");
                break;
            }
            86101 | 86090 => {}
            _ => {
                error!("{}", message);
                return Ok(());
            }
        }
    }
    let cookies = COOKIE_JAR
        .cookies(&"https://bilibili.com".parse().unwrap())
        .context("Auth related cookies should be set by bilibili.")?
        .to_str()?
        .to_owned();
    let WbiResp {
        data: WbiData { mid, uname, .. },
    } = BiliApi::request(WbiPayload).await?;
    db.upsert_account(account::Model {
        account_id: mid,
        name: uname.to_owned(),
        cookies,
        state: AccountState::Active.to_string(),
    })
    .await?;
    println!("Hello😊, {}.", uname);
    Ok(())
}

pub async fn usecookies(cookies: String) -> Result<()> {
    let db = db().await;
    set_cookie_jar(parse_cookies(&cookies));
    let cookies = COOKIE_JAR
        .cookies(&"https://bilibili.com".parse().unwrap())
        .context("Auth related cookies should be set by fav.")?
        .to_str()?
        .to_owned();
    let WbiResp {
        data: WbiData { mid, uname, .. },
    } = BiliApi::request(WbiPayload).await?;
    db.upsert_account(account::Model {
        account_id: mid,
        name: uname.to_owned(),
        cookies,
        state: AccountState::Active.to_string(),
    })
    .await?;
    println!("Hello😊, {}.", uname);
    Ok(())
}

pub async fn logout(account_id: i64) -> Result<()> {
    let db = db().await;
    let account = db.get_account(account_id).await?;
    logout_account(account_id, account.cookies).await?;
    info!("Logout successfully.");
    db.delete_account(account_id).await?;
    println!("Goodbye👋, {}", account.name);
    Ok(())
}

pub async fn logout_all() -> Result<()> {
    let db = db().await;
    let accounts = db.all_accounts().await?;
    let mut tasks = futures::stream::iter(accounts)
        .map(|account| async move {
            logout_account(account.account_id, account.cookies).await?;
            info!("Logout successfully.");
            db.delete_account(account.account_id).await?;
            println!("Goodbye👋, {}", account.name);
            Ok::<_, anyhow::Error>(())
        })
        .buffer_unordered(8);
    while let Some(res) = tasks.next().await {
        if let Err(e) = res {
            error!("{}", e);
        }
    }
    Ok(())
}

async fn logout_account(account_id: i64, cookies: String) -> Result<()> {
    let cookies = parse_cookies(&cookies).collect::<Vec<_>>();
    let bili_jct = cookies
        .iter()
        .find(|c| c.name() == "bili_jct")
        .map(|c| c.value().to_owned())
        .context(format!(
            "No bili_jct in cookies of account_id<{}>.",
            account_id
        ))?;
    set_cookie_jar(cookies.into_iter());
    let LogoutResp { code, message } =
        AuthApi::request(LogoutPayload { biliCSRF: bili_jct }).await?;
    match code {
        0 => Ok(()),
        _ => Err(anyhow!("Failed to logout: {}", message.unwrap_or_default())),
    }
}
