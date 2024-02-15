use crate::proto::bili::{BiliRes, BiliSet, BiliSets};
use fav_core::prelude::*;

impl<'s> ResSets<'s, BiliRes, BiliSet> for BiliSets {
    fn sets(&'s self) -> impl IntoIterator<Item = &'s BiliSet> {
        self.list.iter()
    }

    fn sets_mut(&'s mut self) -> impl IntoIterator<Item = &'s mut BiliSet> {
        self.list.iter_mut()
    }
}

impl Res for BiliSet {
    fn uppers(&self) -> impl IntoIterator<Item = &impl Attr> {
        self.uppers.iter()
    }
}

impl<'s> ResSet<'s, BiliRes> for BiliSet {
    fn res(&'s self) -> impl IntoIterator<Item = &'s BiliRes> {
        self.set.iter()
    }

    fn res_mut(&'s mut self) -> impl IntoIterator<Item = &'s mut BiliRes> {
        self.set.iter_mut()
    }

    fn push(&mut self, resource: BiliRes) {
        self.set.push(resource);
    }

    fn remove(&mut self, id: Id) {
        let id: String = id.into();
        self.set.retain(|res| res.id != id);
    }
}

impl Res for BiliRes {
    fn uppers(&self) -> impl IntoIterator<Item = &impl Attr> {
        self.uppers.iter()
    }
}
