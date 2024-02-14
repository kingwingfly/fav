// The derive macros for `fav_core::api`

use proc_macro::TokenStream;
use quote::quote;
use syn::{parenthesized, parse_macro_input, DeriveInput, Ident};

pub(crate) fn derive_attr(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name_ = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut id: Ident = syn::parse_str("id").unwrap();
    let mut name: Ident = syn::parse_str("name").unwrap();

    if let Some(attr) = input
        .attrs
        .iter()
        .find(|&attr| attr.path().is_ident("attr"))
    {
        attr.parse_nested_meta(|meta| {
            if let Some(i) = meta.path.get_ident() {
                let content;
                parenthesized!(content in meta.input);
                match i.to_string().as_str() {
                    "id" => {
                        id = content.parse()?;
                    }
                    "name" => {
                        name = content.parse()?;
                    }
                    attr => return Err(meta.error(format!("unknown attribute {attr}"))),
                }
            }
            Ok(())
        })
        .unwrap();
    };

    let expanded = quote! {
        impl #impl_generics fav_core::attr::Attr for #name_ #ty_generics #where_clause {
            #[inline]
            fn id(&self) -> fav_core::attr::Id {
                self.#id.into()
            }
            #[inline]
            fn name(&self) -> &str {
                &self.#name
            }
        }
    };

    TokenStream::from(expanded)
}
