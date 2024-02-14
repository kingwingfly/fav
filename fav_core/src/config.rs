//! Config,
//! helping managing the configuration

use crate::local::ProtoLocal;
use reqwest::header::HeaderMap;
use std::collections::HashMap;

/// A HttpConfig, including headers and cookies.
/// # Example
/// ```
/// # #[path = "test_utils/mod.rs"]
/// # mod test_utils;
/// # use test_utils::data::Conf;
/// # use fav_core::config::HttpConfig;
/// # use reqwest::{header::HeaderMap, header};
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
