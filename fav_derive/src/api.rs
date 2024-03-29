// The derive macros for `fav_core::api`

use proc_macro::TokenStream;
use quote::quote;
use syn::{parenthesized, parse_macro_input, DeriveInput, Expr, Ident, LitStr};

pub(crate) fn derive_api(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut endpoint: LitStr = syn::parse_str("\"\"").unwrap();
    let mut params: Expr = syn::parse_str("&[]").unwrap();
    let mut method: Ident = syn::parse_str("GET").unwrap();
    let mut cookies: Expr = syn::parse_str("&[]").unwrap();

    if let Some(attr) = input.attrs.iter().find(|&attr| attr.path().is_ident("api")) {
        attr.parse_nested_meta(|meta| {
            if let Some(i) = meta.path.get_ident() {
                let content;
                parenthesized!(content in meta.input);
                match i.to_string().as_str() {
                    "endpoint" => {
                        endpoint = content.parse()?;
                    }
                    "params" => {
                        params = content.parse()?;
                    }
                    "method" => {
                        method = content.parse()?;
                    }
                    "cookies" => {
                        cookies = content.parse()?;
                    }
                    attr => return Err(meta.error(format!("unknown attribute {attr}"))),
                }
            }
            Ok(())
        })
        .unwrap();
    };

    let expanded = quote! {
        impl #impl_generics fav_core::api::Api for #name #ty_generics #where_clause {
            #[inline]
            fn endpoint(&self) -> &'static str {
                #endpoint
            }
            #[inline]
            fn params(&self) -> &[&str] {
                #params
            }
            #[inline]
            fn cookie_keys(&self) -> &[&str] {
                #cookies
            }
            #[inline]
            fn method(&self) -> reqwest::Method {
                reqwest::Method::#method
            }
        }
    };

    TokenStream::from(expanded)
}
