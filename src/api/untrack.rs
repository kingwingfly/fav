use tracing::info;

use crate::{
    meta::meta,
    proto::data::{ListMeta, Meta, VideoMeta},
};

impl ListMeta {
    pub(super) fn untrack(&mut self) {
        match self.track {
            true => {
                self.track = false;
                info!("Untrack list id:{} title:{}", self.id, self.title);
            }
            false => info!(
                "List id:{} title:{} is already untracked",
                self.id, self.title
            ),
        }
    }
}

impl VideoMeta {
    pub(super) fn untrack(&mut self) {
        match self.track {
            true => {
                self.track = false;
                info!("Untrack video id:{} title:{}", self.bvid, self.title);
            }
            false => info!(
                "Video id:{} title:{} is already untracked",
                self.bvid, self.title
            ),
        }
    }
}

pub(crate) fn untrack(id: String) {
    let mut meta = meta().clone();
    meta.untrack(id);
    meta.persist();
}

impl Meta {
    pub(crate) fn untrack(&mut self, id: String) {
        if let Some(target) = self.lists.iter_mut().find(|l| l.id.to_string() == id) {
            target.untrack();
            self.videos
                .iter_mut()
                .filter(|v| v.list_ids.len() == 1 && v.list_ids[0] == target.id)
                .for_each(|v| v.untrack());
        } else if let Some(target) = self.videos.iter_mut().find(|v| v.bvid == id) {
            target.untrack();
        } else {
            info!("Mark video with id:{id} as untracked");
            self.videos.push(VideoMeta {
                bvid: id,
                ..Default::default()
            });
        }
    }
}
