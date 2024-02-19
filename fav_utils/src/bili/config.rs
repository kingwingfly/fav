use crate::proto::bili::Bili;
use fav_core::prelude::*;
use reqwest::{header, header::HeaderMap};
use std::collections::HashMap;

pub(super) const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.3 Safari/605.1.15";
pub(super) const REFERER: &str = "https://www.bilibili.com/";

impl HttpConfig for Bili {
    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, USER_AGENT.parse().unwrap());
        headers.insert(header::REFERER, REFERER.parse().unwrap());
        headers
    }
    #[inline]
    fn cookies(&self) -> &HashMap<String, String> {
        &self.cookies
    }
    #[inline]
    fn cookies_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.cookies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cookie_test() {
        {
            let mut bili = Bili::default();
            let mut cookies = HashMap::new();
            cookies.insert("1".to_string(), "1".to_string());
            bili.extend_cookies(cookies);
            assert_eq!(bili.cookies().len(), 1);
            assert_eq!(bili.cookies().get("1").unwrap(), "1");
        }
        let mut bili = Bili::read().unwrap();
        assert_eq!(bili.cookies().len(), 1);
        assert_eq!(bili.cookies().get("1").unwrap(), "1");
        let mut cookies = HashMap::new();
        cookies.insert("1".to_string(), "one".to_string());
        cookies.insert("2".to_string(), "two".to_string());
        bili.extend_cookies(cookies);
        assert_eq!(bili.cookies().len(), 2);
        assert_eq!(bili.cookies().get("1").unwrap(), "one");
        assert_eq!(bili.cookies().get("2").unwrap(), "two");
    }
}
