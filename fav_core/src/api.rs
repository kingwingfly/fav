//! API

use http::Method;
use std::borrow::Borrow;
use url::Url;

/// The trait `Api` is the base trait for all API endpoints.
pub trait Api {
    /// The API endpoint.
    const API: &'static str;

    /// Return a `Url` with the API endpoint and the given parameters.
    fn url<I, K, V>(params: I) -> Url
    where
        I: IntoIterator,
        I::Item: Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        url::Url::parse_with_params(Self::API, params).unwrap()
    }

    /// Return `Method::GET` on default.
    fn method() -> Method {
        Method::GET
    }
}
