use crate::{
    meta::meta,
    proto::data::{ListMeta, Meta, VideoMeta},
};
use tracing::info;

impl ListMeta {
    pub(super) fn track(&mut self) {
        match self.track {
            true => info!(
                "List id:{} title:{} is already tracked",
                self.id, self.title
            ),
            false => {
                self.track = true;
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
                list_ids: vec![0], // avoid being removed by tidy()
                track: true,
                ..Default::default()
            });
        }
    }
}

pub(crate) fn track(id: String) {
    let mut meta = meta().clone();
    meta.track(id);
    meta.persist();
}
