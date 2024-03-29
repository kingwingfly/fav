// The derive macros for `fav_core::api`

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Ident};

pub(crate) fn derive_status(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut status: Ident = syn::parse_str("status").unwrap();

    if let Some(attr) = input
        .attrs
        .iter()
        .find(|&attr| attr.path().is_ident("status"))
    {
        attr.parse_nested_meta(|meta| {
            if let Some(i) = meta.path.get_ident() {
                status = i.clone();
            }
            Ok(())
        })
        .unwrap();
    };

    let expanded = quote! {
        impl #impl_generics fav_core::status::Status for #name #ty_generics #where_clause {
            #[inline]
            fn status(&self) -> i32 {
                self.#status
            }
            #[inline]
            fn status_mut(&mut self) -> &mut i32 {
                &mut self.#status
            }
        }
    };

    TokenStream::from(expanded)
}
