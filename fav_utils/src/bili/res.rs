use crate::proto::bili::{BiliRes, BiliSet, BiliSets};
use fav_core::prelude::*;
use std::ops::BitOrAssign;

impl BitOrAssign for BiliSets {
    fn bitor_assign(&mut self, rhs: Self) {
        let mut cache = vec![];
        rhs.list
            .into_iter()
            .for_each(|s| match self.iter_mut().find(|s1| s1.id == s.id) {
                Some(s1) => *s1 |= s,
                None => cache.push(s),
            });
        self.list.extend(cache);
    }
}

impl BitOrAssign for BiliSet {
    /// Merge two sets. If the left set is track, the resources merged into will be track
    fn bitor_assign(&mut self, rhs: Self) {
        rhs.medias.into_iter().for_each(|mut s| {
            if self.iter().all(|s1| s1.bvid != s.bvid) {
                if self.check_status(StatusFlags::TRACK) {
                    s.on_status(StatusFlags::TRACK);
                }
                self.medias.push(s);
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
        self.medias.iter()
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut BiliRes> {
        self.medias.iter_mut()
    }
}

impl Count for BiliSet {
    fn count(&self) -> i32 {
        self.media_count
    }
}

impl Res for BiliRes {}
