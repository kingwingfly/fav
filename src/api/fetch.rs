use super::error::Result;
use crate::{cli::Kind, config::config};

const LISTS_API: &str = "https://api.bilibili.com/x/v3/fav/folder/created/list-all";
const LIST_API: &str = "https://api.bilibili.com/x/v3/fav/folder/collected/list";

pub(crate) async fn fetch() -> Result<()> {
    match config().kind {
        #[cfg(feature = "bili")]
        Kind::Bili => fetch_bili().await?,
    };

    Ok(())
}

#[cfg(feature = "bili")]
async fn fetch_bili() -> Result<()> {
    use reqwest::header::COOKIE;

    let url =
        reqwest::Url::parse_with_params(LISTS_API, &[("up_mid", &config().cookie.DedeUserID)])
            .unwrap();
    println!("{}", url.to_string());
    let resp = reqwest::Client::new()
        .get(url)
        .header(COOKIE, format!("SESSDATA={}", config().cookie.SESSDATA))
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
    async fn fetch_test() {
        assert!(fetch().await.is_ok());
    }
}
