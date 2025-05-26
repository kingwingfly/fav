//! List and Video are both resources that can be tracked. The difference is that
//! List is a collection of Videos, and Video is a single video.
//! So, the track method of ListMeta will also track all the videos in the list,
//! and the track method of VideoMeta will only track the video itself.
//! The same is untrack.

use crate::proto::data::{ListMeta, Meta, VideoMeta};
use tracing::info;

pub(crate) fn track(id: Vec<String>) {
    let mut meta = Meta::read();
    id.into_iter().for_each(|id| meta.track(id));
    meta.persist();
}

impl ListMeta {
    pub(super) fn track(&mut self) {
        match self.track {
            true => info!(
                "List id:{} title:{} is already tracked",
                self.id, self.title
            ),
            false => {
                self.track = true;
                info!("Track list id:{} title:{}", self.id, self.title);
            }
        }
    }
}

impl VideoMeta {
    pub(super) fn track(&mut self) {
        match self.track {
            true => info!(
                "Video id:{} title:{} is already tracked",
                self.bvid, self.title
            ),
            false => {
                self.track = true;
                info!("Track video id:{} title:{}", self.bvid, self.title);
            }
        }
    }
}

impl Meta {
    fn track(&mut self, id: String) {
        if let Some(target) = self.lists.iter_mut().find(|l| l.id.to_string() == id) {
            target.track();
            self.videos
                .iter_mut()
                .filter(|v| v.list_ids.contains(&target.id))
                .for_each(|v| v.track());
        } else if let Some(target) = self.videos.iter_mut().find(|v| v.bvid == id) {
            target.track();
        } else {
            info!("Mark video with id:{id} as tracked");
            self.videos.push(VideoMeta {
                bvid: id,
                track: true,
                ..Default::default()
            });
        }
    }
}
