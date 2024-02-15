use crate::proto::bili::{BiliRes, BiliSet, BiliSets};
use fav_core::prelude::*;

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
    fn uppers(&self) -> impl IntoIterator<Item = &impl Attr> {
        self.uppers.iter()
    }
}

impl ResSet<BiliRes> for BiliSet {
    fn res<'a>(&'a self) -> impl IntoIterator<Item = &'a BiliRes>
    where
        BiliRes: 'a,
    {
        &self.set
    }

    fn res_mut<'a>(&'a mut self) -> impl IntoIterator<Item = &'a mut BiliRes>
    where
        BiliRes: 'a,
    {
        &mut self.set
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
