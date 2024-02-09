//! API

use http::Method;
use std::borrow::Borrow;
use url::Url;

/// The ApiKind
pub enum ApiKind {
    /// The login Api
    Login,
    /// The logout Api
    Logout,
    /// The resource set api, like fav lists, up's works
    FetchResSet,
    /// The resource metadata api
    FetchRes,
    /// The pull api
    PullRes,
}

/// The trait `ApiProvider` makes resources able to provide the releted Apis that implemented [`Api`] trait.
pub trait ApiProvider {
    /// Return the Api which implemented [`AsApi`]
    fn api(&self, api_name: ApiKind) -> impl Api;
}

/// The trait `Api` is the base trait for all API endpoints.
pub trait Api {
    /// The API endpoint.
    const API: &'static str;

    /// Return empty params map needed
    fn params<I, K, V>() -> I
    where
        I: IntoIterator,
        I::Item: Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>;

    /// Return a `Url` with the API endpoint and the given parameters.
    fn url<I, K, V>(params: I) -> Url
    where
        I: IntoIterator,
        I::Item: Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Url::parse_with_params(Self::API, params).unwrap()
    }

    /// Return `Method::GET` on default.
    fn method() -> Method {
        Method::GET
    }
}
