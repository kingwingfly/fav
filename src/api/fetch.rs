use super::client;
use super::error::Result;
use crate::meta::meta;
use crate::proto::data::{ListMeta, Meta, UserMeta, VideoMeta};
use crate::{cli::Kind, config::config};
use protobuf_json_mapping::{parse_from_str_with_options, ParseOptions};
use tracing::info;

const LISTS_API: &str = "https://api.bilibili.com/x/v3/fav/folder/created/list-all";
const FAV_API: &str = "https://api.bilibili.com/x/v3/fav/resource/list";
const VIDEO_API: &str = "https://api.bilibili.com/x/web-interface/view";

static PARSE_OPTIONS: ParseOptions = ParseOptions {
    ignore_unknown_fields: true,
    _future_options: (),
};

pub(crate) async fn fetch(prune: bool) -> Result<()> {
    let mut meta = meta().clone();
    meta.before_fetch();
    match config().kind {
        #[cfg(feature = "bili")]
        Kind::Bili => meta.fetch_bili().await?,
    };
    if prune {
        meta.tidy();
    }
    meta.after_fetch();
    meta.persist();
    Ok(())
}

#[cfg(feature = "bili")]
impl Meta {
    async fn fetch_bili(&mut self) -> Result<()> {
        info!("fetching...");
        self.fetch_lists().await?;
        self.fetch_videos().await?;
        self.fetch_metas().await?;
        Ok(())
    }

    fn before_fetch(&mut self) {
        // assume all lists are expired, and will be set to false if fetched
        self.lists.iter_mut().for_each(|l| l.expired = true);
        // assume all video are not favorite, and will be set to true if fetched
        self.videos.iter_mut().for_each(|v| {
            v.fav = false;
            v.list_ids.clear();
        });
    }

    /// This will keep `track`
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

    async fn fetch_videos(&mut self) -> Result<()> {
        for (list_id, count) in self
            .lists
            .iter()
            .filter(|list| list.track)
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
                            v.fav = true;
                            if !v.list_ids.contains(&list_id) {
                                v.list_ids.push(list_id);
                            }
                        } else {
                            video.list_ids.push(list_id);
                            video.fav = true;
                            video.track = true;
                            self.videos.push(video);
                        }
                    });
            }
        }
        Ok(())
    }

    async fn fetch_metas(&mut self) -> Result<()> {
        for video in self.videos.iter_mut().filter(|v| v.title.is_empty()) {
            video.fetch().await?;
        }
        Ok(())
    }

    /// remove lists that are expired and untracked, then remove videos only in them and untracked
    fn tidy(&mut self) {
        info!("tidyng...");
        self.lists.retain(|l| !l.expired);
        self.videos.retain(|v| !v.list_ids.is_empty() && v.track);
    }

    fn after_fetch(&self) {
        self.status_video();
        self.status_expired();
        self.status_not_fav();
    }
}

impl VideoMeta {
    async fn fetch(&mut self) -> Result<()> {
        let url = reqwest::Url::parse_with_params(VIDEO_API, [("bvid", self.bvid.clone())]);
        let resp = client().get(url.unwrap()).send().await?;
        let mut json: serde_json::Value = resp.json().await?;
        let mut v = json.pointer_mut("/data").unwrap().take();
        let u = v.pointer_mut("/owner").unwrap().take();
        let upper: UserMeta = parse_from_str_with_options(&u.to_string(), &PARSE_OPTIONS).unwrap();
        let new: VideoMeta = parse_from_str_with_options(&v.to_string(), &PARSE_OPTIONS).unwrap();
        self.upper = protobuf::MessageField::some(upper);
        self.title = new.title;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_test() {
        assert!(fetch(false).await.is_ok());
    }

    #[tokio::test]
    async fn fetch_prune_test() {
        assert!(fetch(true).await.is_ok());
    }

    #[tokio::test]
    async fn update_meta_test() {
        let mut video = VideoMeta {
            bvid: "BV15u4y1L7xb".to_string(),
            ..Default::default()
        };
        assert!(video.fetch().await.is_ok());
        assert_eq!(video.title, "【文档生成PPT+ChatGPT命名实体识别+k8s】 Rust nextjs gRPC postgres k8s全栈项目 课程设计演示");
    }
}
