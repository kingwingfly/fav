//! `fac_derive` is a crate for `fac_core` to derive traits.

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]
#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

use proc_macro::TokenStream;

mod api;

/// A derive macro helping implemente `Api` trait.
/// # Example
/// ```
/// use fav_core::api::Api;
///
/// #[derive(Api)]
/// #[api(endpoint("http://abc.com"), params(&["id", "pwd"]))]
/// struct LoginApi;
///
/// # fn main() {
/// let api = LoginApi;
/// assert_eq!(api.endpoint(), "http://abc.com");
/// assert_eq!(api.params(), &["id", "pwd"]);
/// # }
/// ```
#[proc_macro_derive(Api, attributes(api))]
pub fn derive_api(input: TokenStream) -> TokenStream {
    api::derive_api(input)
}
