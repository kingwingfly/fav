use super::error::Result;
use super::{client, parse_message};
use crate::meta::meta;
use crate::proto::data::{ListMeta, Meta, UserMeta, VideoMeta};
use crate::{cli::Kind, config::config};
use tracing::{info, warn};

const LISTS_API: &str = "https://api.bilibili.com/x/v3/fav/folder/created/list-all";
const FAV_API: &str = "https://api.bilibili.com/x/v3/fav/resource/list";
const VIDEO_API: &str = "https://api.bilibili.com/x/web-interface/view";

pub(crate) async fn fetch(prune: bool) -> Result<()> {
    let mut meta = meta().clone();
    match config().kind {
        #[cfg(feature = "bili")]
        Kind::Bili => meta.fetch_bili(prune).await?,
    };
    Ok(())
}

#[cfg(feature = "bili")]
impl Meta {
    fn before_fetch(&mut self) {
        // assume all lists are expired, and will be set to false if fetched
        self.lists.iter_mut().for_each(|l| l.expired = true);
        // assume all video are not favorite, and will be set to true if fetched
        self.videos.iter_mut().for_each(|v| {
            v.fav = false;
            v.list_ids.clear();
        });
    }

    async fn fetch_bili(&mut self, prune: bool) -> Result<()> {
        info!("Fetching...");
        self.before_fetch();
        self.fetch_lists().await?;
        self.fetch_videos().await?;
        self.fetch_metas().await?;
        if prune {
            self.tidy();
        }
        self.after_fetch();
        self.persist();
        Ok(())
    }

    /// This will keep `track`
    async fn fetch_lists(&mut self) -> Result<()> {
        info!("Fetching fave lists");
        let url =
            reqwest::Url::parse_with_params(LISTS_API, [("up_mid", &config().cookie.DedeUserID)])
                .unwrap();
        let resp = client().get(url).send().await?;
        let json: serde_json::Value = resp.json().await?;
        if json["data"].is_null() {
            warn!("No list found; Ensure you have created at least one list or logged in");
            return Ok(());
        }
        json["data"]["list"]
            .as_array()
            .unwrap()
            .iter()
            .for_each(|v| {
                let list: ListMeta = parse_message(v);
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
        info!("Fetching fave videos tracked");
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
                let json: serde_json::Value = resp.json().await?;
                json["data"]["medias"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .for_each(|v| {
                        let mut video: VideoMeta = parse_message(v);
                        if let Some(v) = self.videos.iter_mut().find(|v| v.bvid == video.bvid) {
                            v.expired = video.attr != 0;
                            v.fav = true;
                            if !v.list_ids.contains(&list_id) {
                                v.list_ids.push(list_id);
                            }
                        } else {
                            video.list_ids.push(list_id);
                            video.fav = true;
                            video.expired = video.attr != 0;
                            video.track = true;
                            self.videos.push(video);
                        }
                    });
            }
        }
        Ok(())
    }

    async fn fetch_metas(&mut self) -> Result<()> {
        info!("Fetching video metadatas");

        let videos = std::mem::take(&mut self.videos);
        let jhs: Vec<_> = videos
            .into_iter()
            .map(|mut v| {
                tokio::spawn(async move {
                    if v.cid == 0 {
                        v.fetch().await.unwrap();
                    }
                    v
                })
            })
            .collect();
        for jh in jhs {
            self.videos.push(jh.await.unwrap())
        }
        Ok(())
    }

    /// remove lists that are expired and untracked, and remove videos untracked
    fn tidy(&mut self) {
        info!("Tidyng...");
        self.lists
            .iter_mut()
            .filter(|l| l.expired)
            .for_each(|l| l.untrack());
        self.lists.retain(|l| l.track);
        self.videos.retain(|v| v.track);
    }

    fn after_fetch(&self) {
        self.status_video(false);
        self.status_expired();
        self.status_not_fav();
    }
}

impl VideoMeta {
    async fn fetch(&mut self) -> Result<()> {
        let url = reqwest::Url::parse_with_params(VIDEO_API, [("bvid", self.bvid.clone())]);
        let resp = client().get(url.unwrap()).send().await?;
        let json: serde_json::Value = resp.json().await?;
        match json["code"].as_i64().unwrap() {
            0 => {
                let new: VideoMeta = parse_message(&json["data"]);
                let upper: UserMeta = parse_message(&json["data"]["owner"]);
                self.upper = protobuf::MessageField::some(upper);
                self.title = new.title;
                self.cid = new.cid;
            }
            _ => self.cid = -1,
        }

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
