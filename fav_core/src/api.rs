use http::Method;
use std::borrow::Borrow;
use url::Url;

trait Api {
    const API: &'static str;
    fn url<I, K, V>(params: I) -> Url
    where
        I: IntoIterator,
        I::Item: Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        url::Url::parse_with_params(Self::API, params).unwrap()
    }

    fn method() -> Method {
        Method::GET
    }
}
