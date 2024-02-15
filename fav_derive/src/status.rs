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
            fn status(&self) -> fav_core::status::StatusFlags {
                self.#status.into()
            }
            #[inline]
            fn check_status(&self, status: fav_core::status::StatusFlags) -> bool {
                self.#status & status.bits() != 0
            }
            #[inline]
            fn set_status(&mut self, status: fav_core::status::StatusFlags) {
                self.#status = status.bits();
            }
            #[inline]
            fn on_status(&mut self, status: fav_core::status::StatusFlags) {
                self.#status |= status.bits();
            }
            #[inline]
            fn off_status(&mut self, status: fav_core::status::StatusFlags) {
                self.#status &= !status.bits();
            }
        }
    };

    TokenStream::from(expanded)
}
