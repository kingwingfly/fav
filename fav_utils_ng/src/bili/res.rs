use crate::proto::bili::{BiliRes, BiliSet, BiliSets};
use fav_core::prelude::*;
use std::ops::BitOrAssign;

impl BitOrAssign for BiliSets {
    fn bitor_assign(&mut self, rhs: Self) {
        rhs.list.into_iter().for_each(|s| {
            if self.iter().all(|s1| s1.id != s.id) {
                self.list.push(s);
            }
        })
    }
}

impl BitOrAssign for BiliSet {
    fn bitor_assign(&mut self, rhs: Self) {
        rhs.medias.into_iter().for_each(|r| {
            if self.iter().all(|r1| r1.bvid != r.bvid) {
                self.medias.push(r);
            }
        });
    }
}

impl ResSets<BiliSet, BiliRes> for BiliSets {
    fn sets<'a>(&'a self) -> impl IntoIterator<Item = &'a BiliSet>
    where
        BiliRes: 'a,
    {
        &self.list
    }

    fn sets_mut<'a>(&'a mut self) -> impl IntoIterator<Item = &'a mut BiliSet>
    where
        BiliRes: 'a,
    {
        &mut self.list
    }
}

impl Res for BiliSet {
    fn upper(&self) -> &impl Attr {
        self.upper.as_ref().unwrap()
    }
}

impl ResSet<BiliRes> for BiliSet {
    fn res<'a>(&'a self) -> impl IntoIterator<Item = &'a BiliRes>
    where
        BiliRes: 'a,
    {
        &self.medias
    }

    fn res_mut<'a>(&'a mut self) -> impl IntoIterator<Item = &'a mut BiliRes>
    where
        BiliRes: 'a,
    {
        &mut self.medias
    }

    fn push(&mut self, resource: BiliRes) {
        self.medias.push(resource);
    }

    fn remove(&mut self, id: Id) {
        let id: String = id.into();
        self.medias.retain(|res| res.bvid != id);
    }
}

impl Res for BiliRes {
    fn upper(&self) -> &impl Attr {
        self.upper.as_ref().unwrap()
    }
}
