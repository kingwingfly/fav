use tracing::{info, warn};

use crate::proto::data::{Meta, Qn};

pub(crate) fn modify(id: Vec<String>, save: Option<bool>, clarity: Option<Qn>) {
    let mut meta = Meta::read();
    match save {
        Some(save) => {
            for i in id.iter() {
                meta.mark_saved(i, save);
            }
        }
        None => {}
    }
    match clarity {
        Some(clarity) => {
            for i in id.iter() {
                meta.modify_clarity(i, clarity);
            }
        }
        None => {}
    }
    meta.persist();
}

impl Meta {
    pub(crate) fn mark_saved(&mut self, id: &str, saved: bool) {
        if let Some(target) = self.lists.iter().find(|l| l.id.to_string() == id) {
            self.videos
                .iter_mut()
                .filter(|v| v.list_ids.contains(&target.id))
                .for_each(|v| v.saved = saved);
            info!(
                "Mark videos in list id:{} title:{} as saved",
                target.id, target.title
            )
        } else if let Some(target) = self.videos.iter_mut().find(|v| v.bvid == id) {
            target.saved = saved;
            info!(
                "Mark video id:{} title:{} as saved",
                target.bvid, target.title
            );
        } else {
            warn!("id<{}> not found", id);
        }
    }

    pub(crate) fn modify_clarity(&mut self, id: &str, clarity: Qn) {
        if let Some(target) = self.lists.iter_mut().find(|l| l.id.to_string() == id) {
            target.clarity = clarity.into();
            self.videos
                .iter_mut()
                .filter(|v| v.list_ids.contains(&target.id))
                .for_each(|v| {
                    if v.clarity.unwrap() != clarity {
                        v.clarity = clarity.into();
                        v.saved = false;
                    }
                });
            info!(
                "Mofified clarity of id:{} title:{}",
                target.id, target.title,
            );
        } else if let Some(target) = self.videos.iter_mut().find(|v| v.bvid == id) {
            if target.clarity.unwrap() == clarity {
                info!(
                    "Clarity of video id:{} title:{} is not changed",
                    target.bvid, target.title,
                );
            } else {
                target.clarity = clarity.into();
                target.saved = false;
                info!(
                    "Mofified clarity of video id:{} title:{}",
                    target.bvid, target.title,
                );
            }
        } else {
            warn!("id<{}> not found", id);
        }
    }
}
