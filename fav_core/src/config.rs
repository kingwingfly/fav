//! Config

use http::header::{self, HeaderMap, HeaderValue};
use protobuf::MessageFull;

/// A HttpConfig
/// # Example
/// ```
/// #[cfg(feature = "test_utils")]
/// # {
/// # use fav_core::config::HttpConfig;
/// # use http::header::{HeaderMap, self};
/// struct Config {
///     headers: HeaderMap,
/// }
///
/// impl HttpConfig for Config {
///     fn default_headers(&self) -> &HeaderMap {
///         &self.headers
///     }
///
///     fn default_headers_mut(&mut self) -> &mut HeaderMap {
///         &mut self.headers
///     }
/// }
///
/// # fn main() {
/// let mut headers = HeaderMap::new();
/// headers.insert(header::REFERER, "www.abc.com".parse().unwrap());
/// let mut config = Config {
///     headers: headers.clone(),
/// };
/// assert_eq!(config.default_headers(), &headers);
/// config.set_default_cookies("SESSDATA=123123");
/// headers.insert(header::COOKIE, "SESSDATA=123123".parse().unwrap());
/// assert_eq!(config.default_headers(), &headers);
/// # }
/// # }
/// ```
pub trait HttpConfig {
    /// The default headers
    fn default_headers(&self) -> &HeaderMap;
    /// The mut default headers
    fn default_headers_mut(&mut self) -> &mut HeaderMap;

    /// Set default headers
    fn set_default_cookies(&mut self, cookies: &str) {
        self.default_headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(cookies).unwrap());
    }
}

/// Making it able to be a config, which concludes HttpConfig, and can be persisted into protobuf.
pub trait Config: HttpConfig + MessageFull {}
