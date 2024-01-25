use super::client;
use super::error::Result;
use crate::meta::meta;
use crate::proto::data::{ListMeta, Meta, VideoMeta};
use crate::{cli::Kind, config::config};
use protobuf_json_mapping::{parse_from_str_with_options, ParseOptions};
use tracing::info;

const LISTS_API: &str = "https://api.bilibili.com/x/v3/fav/folder/created/list-all";
const FAV_API: &str = "https://api.bilibili.com/x/v3/fav/resource/list";
static PARSE_OPTIONS: ParseOptions = ParseOptions {
    ignore_unknown_fields: true,
    _future_options: (),
};

pub(crate) async fn fetch() -> Result<()> {
    info!("fetching...");
    match config().kind {
        #[cfg(feature = "bili")]
        Kind::Bili => fetch_bili().await?,
    };

    Ok(())
}

#[cfg(feature = "bili")]
async fn fetch_bili() -> Result<()> {
    let mut meta = meta().clone();
    meta.lists.extend(fetch_lists().await?);
    meta.unsav_but_fav.extend(
        fetch_fav_videos(
            meta.lists
                .iter()
                .filter(|list| list.is_tracking)
                .map(|list| list.id),
        )
        .await?,
    );
    tidy(&mut meta);
    info!("not saved favirite: {}", meta.unsav_but_fav.len());
    meta.persist();
    Ok(())
}

#[cfg(feature = "bili")]
async fn fetch_bili_prune() -> Result<()> {
    let mut meta = meta().clone();
    meta.lists = fetch_lists().await?;
    meta.unsav_but_fav = fetch_fav_videos(
        meta.lists
            .iter()
            .filter(|list| list.is_tracking)
            .map(|list| list.id),
    )
    .await?;
    tidy(&mut meta);
    info!("not saved favirite: {}", meta.unsav_but_fav.len());
    meta.persist();
    Ok(())
}

/// This will keep `is_tracking`
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
            let mut ret: ListMeta =
                parse_from_str_with_options(&v.to_string(), &PARSE_OPTIONS).unwrap();
            ret.is_tracking = meta()
                .lists
                .iter()
                .find(|list| list.id == ret.id)
                .map(|list| list.is_tracking)
                .unwrap_or(false);
            ret
        })
        .collect())
}

async fn fetch_fav_videos(list_ids: impl IntoIterator<Item = i64>) -> Result<Vec<VideoMeta>> {
    let mut ret = vec![];
    for list_id in list_ids {
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
                        let mut ret: VideoMeta =
                            parse_from_str_with_options(&v.to_string(), &PARSE_OPTIONS).unwrap();
                        ret.list_id = Some(list_id);
                        ret
                    }),
            );
        }
    }
    Ok(ret)
}

fn tidy(meta: &mut Meta) {
    info!("tidyng...");
    meta.unsav_but_fav.retain(|v| {
        !meta
            .unsav_anymore
            .iter()
            .chain(meta.sav_and_fav.iter())
            .chain(meta.sav_but_unfav.iter())
            .any(|other| other.bvid == v.bvid)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_test() {
        assert!(fetch().await.is_ok());
    }
}
