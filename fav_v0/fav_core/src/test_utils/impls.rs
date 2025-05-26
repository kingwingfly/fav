#[cfg(test)]
use crate::{api::DefaultApiKind, prelude::*};
#[cfg(not(test))]
use fav_core::{api::DefaultApiKind, prelude::*};

use super::data::{App, TestRes, TestSet, TestSets};
use core::future::Future;

impl HttpConfig for App {
    fn headers(&self) -> reqwest::header::HeaderMap {
        todo!()
    }

    fn cookies(&self) -> &std::collections::HashMap<String, String> {
        todo!()
    }

    fn cookies_mut(&mut self) -> &mut std::collections::HashMap<String, String> {
        todo!()
    }
}

impl ApiProvider for App {
    type ApiKind = DefaultApiKind;

    fn api(&self, _: Self::ApiKind) -> &dyn Api {
        todo!()
    }
}

impl AuthOps for App {
    async fn login(&mut self) -> FavCoreResult<()> {
        Ok(())
    }

    async fn logout(&mut self) -> FavCoreResult<()> {
        Ok(())
    }
}

impl ResOps for App {
    type Res = TestRes;

    async fn fetch_res<Fut, Any>(&self, _: &mut Self::Res, _: Fut) -> FavCoreResult<()>
    where
        Fut: Future<Output = Any> + Send,
        Any: Send,
    {
        todo!()
    }

    async fn pull_res<Fut, Any>(&self, _: &mut Self::Res, _: Fut) -> FavCoreResult<()>
    where
        Fut: Future<Output = Any> + Send,
        Any: Send,
    {
        todo!()
    }
}

impl SetOps for App {
    type Set = TestSet;

    async fn fetch_set<Fut, Any>(&self, _: &mut Self::Set, _: Fut) -> FavCoreResult<()>
    where
        Fut: Future<Output = Any> + Send,
        Any: Send,
    {
        todo!()
    }
}

impl Sets for TestSets {
    type Set = TestSet;

    fn iter(&self) -> impl Iterator<Item = &Self::Set> {
        self.sets.iter()
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Self::Set> {
        self.sets.iter_mut()
    }
}

impl Status for TestSet {
    fn status(&self) -> i32 {
        self.status
    }

    fn status_mut(&mut self) -> &mut i32 {
        &mut self.status
    }
}

impl Attr for TestRes {
    fn id(&self) -> Id {
        todo!()
    }

    fn title(&self) -> &str {
        todo!()
    }
}

impl Owner for TestRes {
    fn owner(&self) -> &str {
        todo!()
    }
}

impl Status for TestRes {
    fn status(&self) -> i32 {
        self.status
    }

    fn status_mut(&mut self) -> &mut i32 {
        &mut self.status
    }
}

impl Res for TestRes {}

impl Set for TestSet {
    type Res = TestRes;

    fn iter(&self) -> impl Iterator<Item = &Self::Res> {
        self.set.iter()
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Self::Res> {
        self.set.iter_mut()
    }
}
