use proc_macro::TokenStream;
use quote::quote;
use syn::{parenthesized, parse_macro_input, DeriveInput, Ident};

pub(crate) fn derive_owner(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut owner: Ident = syn::parse_str("owner").unwrap();

    if let Some(attr) = input
        .attrs
        .iter()
        .find(|&attr| attr.path().is_ident("owner"))
    {
        attr.parse_nested_meta(|meta| {
            if let Some(i) = meta.path.get_ident() {
                let content;
                parenthesized!(content in meta.input);
                match i.to_string().as_str() {
                    "owner" => {
                        owner = content.parse()?;
                    }
                    attr => return Err(meta.error(format!("unknown attribute {attr}"))),
                }
            }
            Ok(())
        })
        .unwrap();
    };

    let expanded = quote! {
        impl #impl_generics fav_core::attr::Owner for #name #ty_generics #where_clause {
            #[inline]
            fn owner(&self) -> &str {
                use fav_core::attr::Attr;
                self.#owner.title()
            }
        }
    };

    TokenStream::from(expanded)
}
