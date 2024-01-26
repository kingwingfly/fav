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

pub(crate) async fn fetch(prune: bool) -> Result<()> {
    let mut meta = meta().clone();
    // assume all lists are expired, and will be set to false if fetched
    meta.lists.iter_mut().for_each(|l| l.expired = true);
    match config().kind {
        #[cfg(feature = "bili")]
        Kind::Bili => meta.fetch_bili().await?,
    };
    if prune {
        meta.tidy();
    }
    meta.persist();
    Ok(())
}

#[cfg(feature = "bili")]
impl Meta {
    async fn fetch_bili(&mut self) -> Result<()> {
        info!("fetching...");
        self.fetch_lists().await?;
        self.fetch_fav_videos().await?;
        self.tidy();
        info!("not saved favirite: {}", self.videos.len());
        Ok(())
    }

    /// This will keep `is_tracked`
    async fn fetch_lists(&mut self) -> Result<()> {
        let url =
            reqwest::Url::parse_with_params(LISTS_API, [("up_mid", &config().cookie.DedeUserID)])
                .unwrap();
        let resp = client().get(url).send().await?;
        let mut json: serde_json::Value = resp.json().await?;
        json.pointer_mut("/data/list")
            .unwrap()
            .take()
            .as_array()
            .unwrap()
            .iter()
            .for_each(|v| {
                let list: ListMeta =
                    parse_from_str_with_options(&v.to_string(), &PARSE_OPTIONS).unwrap();
                if let Some(l) = self.lists.iter_mut().find(|l| list.id == l.id) {
                    l.title = list.title;
                    l.media_count = list.media_count;
                    l.expired = false;
                } else {
                    self.lists.push(list);
                }
            });
        Ok(())
    }

    async fn fetch_fav_videos(&mut self) -> Result<()> {
        for (list_id, count) in self
            .lists
            .iter()
            .filter(|list| list.is_tracked)
            .map(|list| (list.id, list.media_count))
        {
            for page in 0..=count / 20 {
                let url = reqwest::Url::parse_with_params(
                    FAV_API,
                    [
                        ("media_id", list_id.to_string()),
                        ("pn", (page + 1).to_string()),
                        ("ps", "20".to_string()),
                    ],
                )
                .unwrap();
                let resp = client().get(url).send().await?;
                let mut json: serde_json::Value = resp.json().await?;
                json.pointer_mut("/data/medias")
                    .unwrap()
                    .take()
                    .as_array()
                    .unwrap()
                    .iter()
                    .for_each(|v| {
                        let mut video: VideoMeta =
                            parse_from_str_with_options(&v.to_string(), &PARSE_OPTIONS).unwrap();
                        if let Some(v) = self.videos.iter_mut().find(|v| v.bvid == video.bvid) {
                            v.expired = video.attr != 0;
                            if !v.list_ids.contains(&list_id) {
                                v.list_ids.push(list_id);
                            }
                        } else {
                            video.list_ids.push(list_id);
                            self.videos.push(video);
                        }
                    });
            }
        }
        Ok(())
    }

    /// remove expired list and videos
    fn tidy(&mut self) {
        info!("tidyng...");
        self.lists.retain(|l| {
            if l.expired {
                info!("remove list: {}", l.title);
                self.videos.iter_mut().for_each(|v| {
                    v.list_ids.retain(|id| id != &l.id);
                });
            }
            !l.expired
        });
        self.videos.retain(|v| !v.list_ids.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_test() {
        assert!(fetch(false).await.is_ok());
    }
}
