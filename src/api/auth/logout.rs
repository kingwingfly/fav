use crate::{
    api::error::{LogoutFail, Result},
    config::config,
};
use snafu::ResultExt;
use tracing::{info, warn};

const LOG_OUT_API: &str = "https://passport.bilibili.com/login/exit/v2";

pub(crate) async fn logout() {
    if let Err(e) = try_logout().await {
        warn!("{}", e);
    }
}

async fn try_logout() -> Result<()> {
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
    let json: serde_json::Value = resp.json().await.context(LogoutFail { msg: "Not login" })?;
    match json.pointer("/code").unwrap().as_i64().unwrap() {
        0 => info!("Logged out"),
        _ => warn!("Failed to log out"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn logout_test() {
        assert!(try_logout().await.is_ok());
    }
}
