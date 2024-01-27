use super::{client, error::Result};
use crate::{config::config, proto::data::Cookie};
use qrcode::{render::unicode, QrCode};
use tracing::{info, instrument, warn};

const QR_API: &str = "https://passport.bilibili.com/x/passport-login/web/qrcode/generate";
const QR_RET_API: &str = "https://passport.bilibili.com/x/passport-login/web/qrcode/poll";
const LOG_OUT_API: &str = "https://passport.bilibili.com/login/exit/v2";
const POLL_INTERVAL: u64 = 3;

/// Login with QR code.
#[instrument(name = "QR Login", ret)]
pub(crate) async fn qr_login() -> Result<()> {
    let QrInfo { url, qrcode_key } = qr_info().await?;
    show_qr_code(url).await?;
    qr_ret(qrcode_key).await?;
    Ok(())
}

#[cfg(feature = "bili")]
#[derive(serde::Deserialize)]
struct QrInfo {
    url: String,
    qrcode_key: String,
}

async fn try_persist_cookie(resp: &reqwest::Response) {
    let mut buffer = vec![];
    for c in resp.cookies() {
        buffer.push(c);
    }
    if !buffer.is_empty() {
        let mut cookie = Cookie::default();
        for c in buffer {
            match c.name() {
                "DedeUserID" => cookie.DedeUserID = c.value().to_string(),
                "DedeUserID__ckMd5" => cookie.DedeUserID__ckMd5 = c.value().to_string(),
                "SESSDATA" => cookie.SESSDATA = c.value().to_string(),
                "bili_jct" => cookie.bili_jct = c.value().to_string(),
                name => warn!("unknown cookie: {}", name),
            }
        }
        cookie.buvid3 = get_buvid().await.unwrap();
        cookie.persist();
    }
}

async fn qr_info() -> Result<QrInfo> {
    let resp = reqwest::get(QR_API).await?;
    let mut json: serde_json::Value = resp.json().await?;
    tracing::debug!("{:#?}", json);
    Ok(serde_json::from_value(json.pointer_mut("/data").unwrap().take()).unwrap())
}

async fn show_qr_code(url: String) -> Result<()> {
    let code = QrCode::new(url).unwrap();
    let image = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build();
    println!("\n{}", image);
    Ok(())
}

async fn qr_ret(qrcode_key: String) -> Result<()> {
    let url = reqwest::Url::parse_with_params(QR_RET_API, [("qrcode_key", qrcode_key)]).unwrap();
    loop {
        let resp = client().get(url.clone()).send().await?;
        try_persist_cookie(&resp).await;
        let json: serde_json::Value = resp.json().await?;
        match json.pointer("/data/code").unwrap().as_i64().unwrap() {
            0 => {
                break;
            }
            86038 => warn!("QR code expired"),
            _ => tracing::debug!("{:#?}", json.pointer("/data/message").unwrap()),
        }
        tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL)).await;
    }
    Ok(())
}

async fn get_buvid() -> Result<String> {
    let resp = client()
        .get(" https://api.bilibili.com/x/frontend/finger/spi")
        .send()
        .await?;
    let json: serde_json::Value = resp.json().await?;
    Ok(json.pointer("/data/b_3").unwrap().to_string())
}

pub(crate) async fn logout() -> Result<()> {
    use reqwest::header::COOKIE;

    let cookie = &config().cookie;
    let url =
        reqwest::Url::parse_with_params(LOG_OUT_API, [("biliCSRF", &cookie.bili_jct)]).unwrap();
    let resp = reqwest::Client::new()
        .post(url)
        .header(
            COOKIE,
            format!(
                "DedeUserID={}; bili_jct={}; SESSDATA={}",
                cookie.DedeUserID, cookie.bili_jct, cookie.SESSDATA
            ),
        )
        .send()
        .await?;
    let json: serde_json::Value = resp.json().await?;
    match json.pointer("/code").unwrap().as_i64().unwrap() {
        0 => info!("logged out"),
        _ => warn!("failed to log out"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn logout_test() {
        assert!(logout().await.is_ok());
    }
}
