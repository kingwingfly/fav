use crate::proto::bili::Bili;
use fav_core::prelude::*;
use reqwest::{header, header::HeaderMap};
use std::collections::HashMap;

impl HttpConfig for Bili {
    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, self.headers.user_agent.parse().unwrap());
        headers.insert(header::REFERER, self.headers.referer.parse().unwrap());
        headers
    }

    fn cookies(&self) -> &HashMap<String, String> {
        &self.headers.cookies
    }

    fn set_cookies(&mut self, cookies: HashMap<String, String>) {
        self.headers.as_mut().unwrap().cookies = cookies;
    }
}
