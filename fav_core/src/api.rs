//! API,
//! making the api easy to use

use reqwest::Method;
use url::Url;

#[cfg(feature = "derive")]
pub use fav_derive::Api;

#[allow(missing_docs)]
/// The DefaultApiKind
pub enum DefaultApiKind {
    Login,
    QrLogin,
    QrCheck,
    Logout,
    FetchResSet,
    FetchRes,
    PullRes,
}

/// The trait `ApiProvider` makes resources able to provide the releted Apis that implemented [`Api`] trait.
/// # Example
/// ```
/// # use fav_core::api::{ApiProvider, Api, DefaultApiKind};
/// # use url::Url;
/// struct Remote;
/// struct LoginApi;
///
/// impl Api for LoginApi {
///     fn endpoint(&self) -> &'static str {
///         "http://abc.com"
///     }
///
///     fn params(&self) -> &[&str] {
///         &["id", "pwd"]
///     }
/// }
///
/// impl ApiProvider<DefaultApiKind> for Remote {
///     fn api(&self, api_name: DefaultApiKind) -> Box<dyn Api + Send> {
///         Box::new(match api_name {
///             DefaultApiKind::Login => LoginApi,
///             _ => unimplemented!()
///         })
///     }
/// }
///
/// # fn main() {
/// let remote = Remote;
/// let api = remote.api(DefaultApiKind::Login);
/// let params = api.params().iter().copied().zip(["Jake", "123"]).collect();
/// let url = api.url(params);
/// let expected = Url::parse_with_params("http://abc.com", vec![("id", "Jake"), ("pwd", "123")]).unwrap();
/// assert_eq!(url, expected);
/// # }
pub trait ApiProvider<K> {
    /// Return the Api which implemented [`Api`]
    fn api(&self, api_kind: K) -> Box<dyn Api + Send>;
}

/// The trait `Api` is the base trait for all API endpoints.
/// This trait should be object-safe.
/// # Example
/// See [`ApiProvider`]
pub trait Api {
    /// Return the endpoint
    fn endpoint(&self) -> &'static str;
    /// Return empty params map needed
    fn params(&self) -> &[&str];

    /// Return a `Url` with the API endpoint and the given parameters.
    fn url(&self, params: Vec<(&str, &str)>) -> Url {
        // Check params when testing
        #[cfg(test)]
        {
            use crate::error::FavCoreError;
            let need = self.params();
            if params.len() != need.len() || params.iter().any(|p| !need.contains(&p.0)) {
                let msg = format!("Need params: {:#?}; Got {:#?}", need, params);
                panic!("{:?}", FavCoreError::ParamsError(msg));
            }
        }
        Url::parse_with_params(self.endpoint(), params).unwrap()
    }

    /// Return `Method::GET` by default.
    fn method(&self) -> Method {
        Method::GET
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Remote;

    impl ApiProvider<DefaultApiKind> for Remote {
        fn api(&self, api_name: DefaultApiKind) -> Box<dyn Api + Send> {
            match api_name {
                DefaultApiKind::Login => todo!(),
                _ => todo!(),
            }
        }
    }

    struct LoginApi;

    impl Api for LoginApi {
        fn endpoint(&self) -> &'static str {
            "http://abc.com"
        }

        fn params(&self) -> &[&str] {
            &["id", "pwd"]
        }
    }

    #[test]
    #[should_panic]
    fn params_panic_test() {
        let remote = Remote;
        let api = remote.api(DefaultApiKind::Login);
        let _ = api.url(vec![("wrong_key", "")]);
    }
}
