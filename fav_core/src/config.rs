//! Config

use http::header::{self, HeaderMap, HeaderValue};
use protobuf::MessageFull;

/// A config
pub trait Config: MessageFull {
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
