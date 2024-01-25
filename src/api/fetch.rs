use super::error::Result;
use crate::meta::meta;
use crate::proto::data::ListMeta;
use crate::{cli::Kind, config::config};
use protobuf_json_mapping::{parse_from_str_with_options, ParseOptions};

const LISTS_API: &str = "https://api.bilibili.com/x/v3/fav/folder/created/list-all";
const FAV_API: &str = "https://api.bilibili.com/x/v3/fav/resource/list";

pub(crate) async fn fetch() -> Result<()> {
    match config().kind {
        #[cfg(feature = "bili")]
        Kind::Bili => fetch_bili().await?,
    };

    Ok(())
}

#[cfg(feature = "bili")]
async fn fetch_bili() -> Result<()> {
    let mut meta = meta().clone();
    meta.list = extract_list().await?;
    println!("{:#?}", meta);
    Ok(())
}

async fn extract_list() -> Result<Vec<ListMeta>> {
    use reqwest::header::COOKIE;

    let url = reqwest::Url::parse_with_params(LISTS_API, [("up_mid", &config().cookie.DedeUserID)])
        .unwrap();
    let resp = reqwest::Client::new()
        .get(url)
        .header(COOKIE, format!("SESSDATA={}", config().cookie.SESSDATA))
        .send()
        .await?;
    let mut json: serde_json::Value = resp.json().await?;
    Ok(json
        .pointer_mut("/data/list")
        .unwrap()
        .take()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            parse_from_str_with_options(
                &v.to_string(),
                &ParseOptions {
                    ignore_unknown_fields: true,
                    ..Default::default()
                },
            )
            .unwrap()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_test() {
        assert!(fetch().await.is_ok());
    }
}
