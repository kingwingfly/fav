use super::error::Result;
use crate::{cli::Kind, config::config};
use std::path::PathBuf;
use tracing::info;

const LOG_OUT_API: &str = "https://passport.bilibili.com/login/exit/v2";

pub(crate) async fn init(path: Option<PathBuf>, kind: Kind) -> Result<()> {
    let path = path
        .unwrap_or(std::path::PathBuf::from("."))
        .join(".backup");
    if path.join("cookie").exists() {
        info!("Try logging out");
        loggout().await?;
        info!("remove old {}", path.display());
    }
    std::fs::remove_dir_all(&path).ok();
    std::fs::create_dir_all(&path).unwrap();
    info!("init {}", path.display());
    match kind {
        #[cfg(feature = "bili")]
        Kind::Bili => std::fs::write(path.join("kind"), "bili").unwrap(),
    }
    Ok(())
}

async fn loggout() -> Result<()> {
    use reqwest::header::{CONTENT_TYPE, COOKIE};

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
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .send()
        .await?;
    let json: serde_json::Value = resp.json().await?;
    println!("{:#?}", json);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn logout_test() {
        assert!(loggout().await.is_ok());
    }
}
