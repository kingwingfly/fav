use super::client;
use super::error::Result;
use crate::meta::meta;
use crate::proto::data::{ListMeta, VideoMeta};
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
    meta.list = fetch_lists().await?;
    meta.unsav_but_fav = fetch_fav_videos(meta.list[0].id).await?;
    println!("{:#?}", meta);
    Ok(())
}

async fn fetch_lists() -> Result<Vec<ListMeta>> {
    let url = reqwest::Url::parse_with_params(LISTS_API, [("up_mid", &config().cookie.DedeUserID)])
        .unwrap();
    let resp = client().get(url).send().await?;
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

async fn fetch_fav_videos(list_id: i64) -> Result<Vec<VideoMeta>> {
    let mut ret = vec![];
    let mut has_more = true;
    let mut page = 1;
    while has_more {
        let url = reqwest::Url::parse_with_params(
            FAV_API,
            [
                ("media_id", list_id.to_string()),
                ("pn", page.to_string()),
                ("ps", "20".to_string()),
            ],
        )
        .unwrap();
        let resp = client().get(url).send().await?;
        let mut json: serde_json::Value = resp.json().await?;
        has_more = json
            .pointer_mut("/data/has_more")
            .unwrap()
            .as_bool()
            .unwrap();
        page += 1;
        ret.extend(
            json.pointer_mut("/data/medias")
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
                }),
        );
    }
    Ok(ret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_test() {
        assert!(fetch().await.is_ok());
    }
}
