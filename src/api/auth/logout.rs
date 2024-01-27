use crate::{api::error::Result, config::config};
use tracing::{info, warn};

const LOG_OUT_API: &str = "https://passport.bilibili.com/login/exit/v2";

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
