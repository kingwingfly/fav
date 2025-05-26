use super::{BiliRes, BiliSet, BiliSets};
use fav_core::prelude::*;
use std::ops::BitOrAssign;

impl BitOrAssign for BiliSets {
    fn bitor_assign(&mut self, rhs: Self) {
        let mut cache = vec![];
        rhs.list
            .into_iter()
            .for_each(|mut s| match self.iter_mut().find(|s1| s1.id == s.id) {
                Some(s1) => {
                    if s.media_count != 0 && s.title != "该合集已失效" {
                        s1.title = s.title;
                        s1.media_count = s.media_count;
                        s1.off_status(StatusFlags::EXPIRED);
                    } else if !s1.check_status(StatusFlags::EXPIRED) {
                        s1.title += "（已失效）";
                        s1.on_status(StatusFlags::EXPIRED);
                    }
                }
                None => {
                    if s.media_count == 0 && s.title == "该合集已失效" {
                        s.on_status(StatusFlags::EXPIRED);
                    }
                    cache.push(s)
                }
            });
        self.list.extend(cache);
    }
}

impl BitOrAssign for BiliSet {
    /// Merge two sets. If the left set is track, the resources merged into will be track
    fn bitor_assign(&mut self, rhs: Self) {
        rhs.medias
            .into_iter()
            .chain(rhs.archives)
            .for_each(|mut r| {
                if self.iter().all(|r1| r1.bvid != r.bvid) {
                    if self.check_status(StatusFlags::TRACK) {
                        r.on_status(StatusFlags::TRACK);
                    }
                    self.medias.push(r);
                }
            });
    }
}

impl BitOrAssign for BiliRes {
    /// Merge two resources. The status of the left-hand side will be preserved.
    fn bitor_assign(&mut self, rhs: Self) {
        let status = self.status;
        *self = rhs;
        self.status = status;
    }
}

impl Sets for BiliSets {
    type Set = BiliSet;

    fn iter(&self) -> impl Iterator<Item = &BiliSet> {
        self.list.iter()
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut BiliSet> {
        self.list.iter_mut()
    }
}

impl Set for BiliSet {
    type Res = BiliRes;

    fn iter(&self) -> impl Iterator<Item = &BiliRes> {
        self.medias.iter().chain(self.archives.iter())
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut BiliRes> {
        self.medias.iter_mut().chain(self.archives.iter_mut())
    }
}

impl Count for BiliSet {
    fn count(&self) -> i32 {
        self.media_count
    }
}

impl Res for BiliRes {}
