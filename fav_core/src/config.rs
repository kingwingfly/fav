//! Config,
//! helping managing the configuration

use std::collections::HashMap;

use crate::local::ProtoLocal;
use reqwest::header::{HeaderMap, HeaderValue};

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
///         hp.insert(header::USER_AGENT, "Mozilla/5.0".parse().unwrap());
///         hp
///     }
///
///     fn cookies(&self) -> &HashMap<String, String> {
///         &self.cookies
///     }
///
///     fn extend_cookies(&mut self, cookies: HashMap<String, String>) {
///         self.cookies.extend(cookies);
///     }
/// }
/// ```
pub trait HttpConfig {
    /// The headers
    fn headers(&self) -> HeaderMap;
    /// The cookies. [`HttpConfig::cookie_value`] uses this to generate the `Cookie` header.
    /// # Caution
    /// `HttpConfig::cookie_value` will omit the keys that are not in the cookies
    /// without any warning.
    fn cookies(&self) -> &HashMap<String, String>;
    /// Set the cookies
    fn extend_cookies(&mut self, cookies: HashMap<String, String>);
    /// Acquire the cookie value by keys
    fn cookie_value(&self, keys: impl IntoIterator<Item = impl AsRef<str>>) -> HeaderValue {
        let cookies = self.cookies();
        keys.into_iter()
            .filter_map(|k| {
                let k = k.as_ref();
                cookies.get(k).map(|v| format!("{k}={v}; "))
            })
            .collect::<String>()
            .parse()
            .unwrap()
    }
}

/// Mark it able to be a config, which concludes [`HttpConfig`], and can be persisted as protobuf through [`ProtoLocal`].
pub trait Config: HttpConfig + ProtoLocal {}
impl<T> Config for T where T: HttpConfig + ProtoLocal {}
