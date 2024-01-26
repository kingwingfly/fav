use crate::{
    meta::meta,
    proto::data::{Meta, VideoMeta},
};
use tracing::info;

impl Meta {
    fn track(&mut self, id: String) {
        if let Some(target) = self.lists.iter_mut().find(|l| l.id.to_string() == id) {
            target.is_tracked = true;
            info!("Tracking list id:{} title:{}", target.id, target.title);
        } else if let Some(target) = self.videos.iter_mut().find(|v| v.bvid == id) {
            info!(
                "Already Tracking video id:{} title:{}",
                target.bvid, target.title
            );
        } else {
            info!("Tracking video id:{id}");
            self.videos.push(VideoMeta {
                bvid: id,
                list_ids: vec![0], // avoid being removed by tidy()
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
