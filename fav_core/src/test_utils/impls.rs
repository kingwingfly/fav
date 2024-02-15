use super::data::{App, TestRes, TestResSet, TestUpper};
use ::core::future::Future;
use bitflags::Flags;
use protobuf::Message;
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

impl Operations<DefaultApiKind> for App {
    async fn login(&mut self) -> FavCoreResult<()> {
        // let resp = self.request(DefaultApiKind::Login, vec![]).await?;
        Ok(())
    }

    async fn logout(&mut self) -> FavCoreResult<()> {
        todo!()
    }

    async fn fetch(&self, resource: &mut impl Meta) -> FavCoreResult<()> {
        todo!()
    }

    async fn pull(&self, resource: &mut impl Meta) -> FavCoreResult<()> {
        todo!()
    }

    async fn fetch_set(&self, resource: &mut impl Meta) -> FavCoreResult<()> {
        todo!()
    }

    async fn fetch_sets(&self) -> FavCoreResult<impl Message> {
        Ok(TestResSet::default())
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
    fn uppers(&self) -> impl IntoIterator<Item = &impl Attr> {
        &self.uppers
    }
}

impl Res for TestResSet {
    fn uppers(&self) -> impl IntoIterator<Item = &impl Attr> {
        &self.uppers
    }
}

impl ResSet for TestResSet {
    fn res(&self) -> impl IntoIterator<Item = &impl Meta> {
        &self.sets
    }

    fn res_mut(&mut self) -> impl IntoIterator<Item = &mut impl Meta> {
        &mut self.sets
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
        res_set.sets.push(res);
    }
}
