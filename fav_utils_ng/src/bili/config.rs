use crate::proto::bili::{Bili, Headers};
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
        let headers_ptr: *mut Option<Box<Headers>> = &mut self.headers.0 as *mut _;
        unsafe {
            match *headers_ptr {
                Some(ref mut headers) => {
                    headers.cookies = cookies;
                }
                None => {
                    *headers_ptr = Some(Box::new(Headers {
                        cookies,
                        ..Default::default()
                    }));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cookie_test() {
        let mut bili = Bili::default();
        let mut cookies = HashMap::new();
        cookies.insert("test".to_string(), "test".to_string());
        bili.set_cookies(cookies);
    }
}
