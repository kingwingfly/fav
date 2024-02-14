//! `fav_derive` is a crate for [fav_core](https://docs.rs/fav_core) to derive traits.

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]
#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

use proc_macro::TokenStream;

mod api;
mod attr;
mod status;

/// A derive macro helping implemente [`Api`] trait.
/// # Example
/// ```
/// use fav_core::api::Api;
///
/// #[derive(Api)]
/// #[api(endpoint("http://abc.com"), params(&["id", "pwd"]), cookies(&["c1"]), method(POST))]
/// struct LoginApi;
///
/// # fn main() {
/// let api = LoginApi;
/// assert_eq!(api.endpoint(), "http://abc.com");
/// assert_eq!(api.params(), &["id", "pwd"]);
/// assert_eq!(api.cookie_keys(), &["c1"]);
/// assert_eq!(api.method(), reqwest::Method::POST);
/// # }
/// ```
#[proc_macro_derive(Api, attributes(api))]
pub fn derive_api(input: TokenStream) -> TokenStream {
    api::derive_api(input)
}

/// A derive macro helping implemente [`Attr`] trait.
/// # Example
/// ```
/// use fav_core::attr::Attr;
///
/// #[derive(Attr)]
/// struct Res {
///    id: i32,
///    name: String,
/// }
///
/// #[derive(Attr)]
/// #[attr(id(res_id), name(res_name))]
/// struct Res_ {
///    res_id: i32,
///    res_name: String,
/// }
/// ```
/// Default fields are `id` and `name`.
/// In practice, the `Res` is comming from `protobuf-codegen`,
/// making the attribute `attr` referring to the fields needed.
#[proc_macro_derive(Attr, attributes(attr))]
pub fn derive_attr(input: TokenStream) -> TokenStream {
    attr::derive_attr(input)
}

/// A derive macro helping implemente [`Status`] trait.
/// # Example
/// ```
/// use fav_core::status::Status;
///
/// #[derive(Status)]
/// struct Res {
///   status: i32,
/// }
///
/// #[derive(Status)]
/// #[status(my_status)]
/// struct Res_ {
///   my_status: i32,
/// }
/// ```
/// Default field is `status`.
/// In practice, the `Res` is comming from `protobuf-codegen`,
/// making the attribute `status` referring to the fields needed.
#[proc_macro_derive(Status, attributes(status))]
pub fn derive_status(input: TokenStream) -> TokenStream {
    status::derive_status(input)
}
