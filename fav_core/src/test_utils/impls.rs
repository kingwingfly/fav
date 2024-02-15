use super::data::{App, TestRes, TestResSet, TestResSets, TestUpper};
use ::core::future::Future;
use bitflags::Flags;
use protobuf::{Message, MessageFull};
use reqwest::{header::HeaderMap, Client, Method, Response};
use serde::de::IntoDeserializer;
use std::collections::HashMap;
use std::future::IntoFuture;
use url::Url;

#[cfg(test)]
use crate::prelude::*;

#[cfg(not(test))]
use fav_core::prelude::*;

impl HttpConfig for App {
    fn headers(&self) -> HeaderMap {
        HeaderMap::new()
    }

    fn cookies(&self) -> &HashMap<String, String> {
        todo!()
    }

    fn extend_cookies(&mut self, cookies: HashMap<String, String>) {
        todo!()
    }
}

impl PathInfo for App {
    const PATH: &'static str = "temp/app";
}

struct LoginApi;

impl Api for LoginApi {
    fn endpoint(&self) -> &'static str {
        "http://www.example.com"
    }

    fn params(&self) -> &[&str] {
        &[]
    }
}

struct LogoutApi;

impl Api for LogoutApi {
    fn endpoint(&self) -> &'static str {
        "http://www.example.com"
    }

    fn params(&self) -> &[&str] {
        &[]
    }
}

impl ApiProvider<DefaultApiKind> for App {
    fn api(&self, api_kind: DefaultApiKind) -> &dyn Api {
        match api_kind {
            DefaultApiKind::Login => &LoginApi,
            DefaultApiKind::Logout => &LogoutApi,
            _ => unimplemented!(),
        }
    }
}

impl ResSets<TestResSet, TestRes> for TestResSets {
    fn sets<'a>(&'a self) -> impl IntoIterator<Item = &'a TestResSet>
    where
        TestResSet: 'a,
    {
        &self.sets
    }

    fn sets_mut<'a>(&'a mut self) -> impl IntoIterator<Item = &'a mut TestResSet>
    where
        TestResSet: 'a,
    {
        &mut self.sets
    }
}

impl Operations<TestResSets, TestResSet, TestRes, DefaultApiKind> for App {
    async fn login(&mut self) -> FavCoreResult<()> {
        todo!()
    }

    async fn logout(&mut self) -> FavCoreResult<()> {
        todo!()
    }

    async fn fetch_sets(&self) -> FavCoreResult<TestResSets> {
        todo!()
    }

    async fn fetch_set(&self, resource: &mut TestResSet) -> FavCoreResult<()> {
        todo!()
    }

    async fn fetch(&self, set: &mut TestRes) -> FavCoreResult<()> {
        todo!()
    }

    async fn pull(&self, resource: &mut TestRes) -> FavCoreResult<()> {
        todo!()
    }
}

impl Attr for TestRes {
    fn id(&self) -> Id {
        self.id.into()
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn set_id(&mut self, id: Id) {
        self.id = id.into();
    }

    fn set_title(&mut self, title: &str) {
        self.title = title.into()
    }
}

impl Status for TestRes {
    fn status(&self) -> StatusFlags {
        self.status.into()
    }

    fn check_status(&self, status: StatusFlags) -> bool {
        self.status & status.bits() != 0
    }

    fn set_status(&mut self, status: StatusFlags) {
        self.status = status.bits();
    }

    fn on_status(&mut self, status: StatusFlags) {
        self.status |= status.bits();
    }

    fn off_status(&mut self, status: StatusFlags) {
        self.status &= !status.bits();
    }
}

impl Attr for TestResSet {
    fn id(&self) -> Id {
        self.id.into()
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn set_id(&mut self, id: Id) {
        todo!()
    }

    fn set_title(&mut self, title: &str) {
        todo!()
    }
}

impl Attr for TestUpper {
    fn id(&self) -> Id {
        self.id.into()
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn set_id(&mut self, id: Id) {
        todo!()
    }

    fn set_title(&mut self, title: &str) {
        todo!()
    }
}

impl Res for TestRes {
    fn upper(&self) -> &impl Attr {
        self.upper.as_ref().unwrap()
    }
}

impl Status for TestResSet {
    fn status(&self) -> StatusFlags {
        todo!()
    }

    fn check_status(&self, status: StatusFlags) -> bool {
        todo!()
    }

    fn set_status(&mut self, status: StatusFlags) {
        todo!()
    }

    fn on_status(&mut self, status: StatusFlags) {
        todo!()
    }

    fn off_status(&mut self, status: StatusFlags) {
        todo!()
    }
}

impl Res for TestResSet {
    fn upper(&self) -> &impl Attr {
        self.upper.as_ref().unwrap()
    }
}

impl ResSet<TestRes> for TestResSet {
    fn res<'a>(&'a self) -> impl IntoIterator<Item = &'a TestRes>
    where
        TestRes: 'a,
    {
        &self.set
    }

    fn res_mut<'a>(&'a mut self) -> impl IntoIterator<Item = &'a mut TestRes>
    where
        TestRes: 'a,
    {
        &mut self.set
    }

    fn push(&mut self, resource: TestRes) {
        todo!()
    }

    fn remove(&mut self, id: Id) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_status_test() {
        let mut res_set = TestResSet::default();
        let status = StatusFlags::empty();
        let mut res = TestRes {
            status: status.bits(),
            ..Default::default()
        };
        res_set.set.push(res);
    }
}
