//! API

use http::Method;
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
/// # Example
/// ```
/// # use fav_core::api::{ApiProvider, Api, ApiKind};
/// # use url::Url;
/// struct Remote;
/// struct LoginApi;
///
/// impl Api for LoginApi {
///     fn api(&self) -> &'static str {
///         "http://abc.com"
///     }
///
///     fn params(&self) -> Vec<&'static str>
///     {
///         vec!["id", "pwd"]
///     }
/// }
///
/// impl ApiProvider for Remote {
///     fn api(&self, api_name: ApiKind) -> Box<dyn Api> {
///         Box::new(match api_name {
///             ApiKind::Login => LoginApi,
///             _ => unimplemented!()
///         })
///     }
/// }
///
/// # fn main() {
/// let remote = Remote;
/// let api = remote.api(ApiKind::Login);
/// let params = api.params().into_iter().zip(["Jake", "123"]).collect();
/// let url = api.url(params);
/// let expected = Url::parse_with_params("http://abc.com", vec![("id", "Jake"), ("pwd", "123")]).unwrap();
/// assert_eq!(url, expected);
/// # }
pub trait ApiProvider {
    /// Return the Api which implemented [`Api`]
    fn api(&self, api_name: ApiKind) -> Box<dyn Api>;
}

/// The trait `Api` is the base trait for all API endpoints.
/// # Example
/// See [`ApiProvider`]
pub trait Api {
    /// Return the base api
    fn api(&self) -> &'static str;
    /// Return empty params map needed
    fn params(&self) -> Vec<&'static str>;

    /// Return a `Url` with the API endpoint and the given parameters.
    fn url(&self, params: Vec<(&'static str, &'static str)>) -> Url {
        // Check params when testing
        #[cfg(test)]
        {
            use crate::error::FavCoreError;
            let need = self.params();
            if params.len() != need.len() || params.iter().any(|p| !need.contains(&p.0)) {
                let msg = format!("Need params: {:#?}; Got {:#?}", need, params);
                Result::<(), FavCoreError>::Err(FavCoreError::ParamsError(msg)).unwrap();
            }
        }
        Url::parse_with_params(self.api(), params).unwrap()
    }

    /// Return `Method::GET` on default.
    fn method(&self) -> Method {
        Method::GET
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Remote;

    impl ApiProvider for Remote {
        fn api(&self, api_name: ApiKind) -> Box<dyn Api> {
            match api_name {
                ApiKind::Login => todo!(),
                _ => todo!(),
            }
        }
    }

    struct LoginApi;

    impl Api for LoginApi {
        fn api(&self) -> &'static str {
            "http://abc.com"
        }

        fn params(&self) -> Vec<&'static str> {
            vec!["id", "pwd"]
        }
    }

    #[test]
    #[should_panic]
    fn params_panic_test() {
        let remote = Remote;
        let api = remote.api(ApiKind::Login);
        let _ = api.url(vec![("wrong_key", "")]);
    }
}
