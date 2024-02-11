//! Config

use crate::local::ProtoLocal;
use http::header::HeaderMap;
use std::collections::HashMap;

/// A HttpConfig
/// # Example
/// ```
/// # mod test_utils;
/// # use test_utils::data::Conf;
/// # use fav_core::config::HttpConfig;
/// # use http::{HeaderMap, header};
/// # use std::collections::HashMap;
///
/// impl HttpConfig for Conf {
///     fn headers(&self) -> HeaderMap {
///         let mut hp = HeaderMap::new();
///         hp.insert(header::USER_AGENT, self.headers.user_agent.parse().unwrap());
///         hp
///     }
///
///     fn cookies(&self) -> &HashMap<String, String> {
///         &self.headers.cookies
///     }
///
///     fn set_cookies(&mut self, cookies: HashMap<String, String>) {
///         self.headers.as_mut().unwrap().cookies = cookies;
///     }
/// }
/// # fn main() {}
/// ```
pub trait HttpConfig {
    /// The headers
    fn headers(&self) -> HeaderMap;
    /// The cookies
    fn cookies(&self) -> &HashMap<String, String>;
    /// Set default headers
    fn set_cookies(&mut self, cookies: HashMap<String, String>);
}

/// Mark it able to be a config, which concludes [`HttpConfig`], and can be persisted as protobuf through [`ProtoLocal`].
pub trait Config: HttpConfig + ProtoLocal {}
impl<T> Config for T where T: HttpConfig + ProtoLocal {}
