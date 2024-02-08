//! Config

/// A config
pub trait Config {
    /// The default headers
    fn default_headers(&self) -> http::HeaderMap;
}
