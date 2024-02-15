use crate::proto::bili;
use fav_core::prelude::*;

impl ResSets for bili::ResSets {
    fn sets(&self) -> impl IntoIterator<Item = &impl ResSet> {
        self.list.iter()
    }

    fn sets_mut(&mut self) -> impl IntoIterator<Item = &mut impl ResSet> {
        self.list.iter_mut()
    }
}

impl Res for bili::ResSet {
    fn uppers(&self) -> impl IntoIterator<Item = &impl Attr> {
        self.uppers.iter()
    }
}

impl ResSet for bili::ResSet {
    fn res(&self) -> impl IntoIterator<Item = &impl Meta> {
        self.set.iter()
    }

    fn res_mut(&mut self) -> impl IntoIterator<Item = &mut impl Meta> {
        self.set.iter_mut()
    }
}

impl Res for bili::Res {
    fn uppers(&self) -> impl IntoIterator<Item = &impl Attr> {
        self.uppers.iter()
    }
}
