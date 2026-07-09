//! `#[derive(SystemSet)]` implementation.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, parse_macro_input};

/// Implementation of `#[derive(SystemSet)]` (see the public entry in `lib.rs`).
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let name_span = name.span();

    // Generics: rejected for both structs and enums. Sets are keyed by
    // `(TypeId, discriminant)`; a generic set type would mint a fresh id per
    // monomorphisation, which is virtually never what the user wants.
    if !input.generics.params.is_empty() {
        return syn::Error::new(
            name_span,
            "SystemSet derive does not support generics (Phase 9 scope)",
        )
        .to_compile_error()
        .into();
    }

    // Body: the `set_discriminant` / `set_name` overrides (if any). Unit
    // structs emit nothing (trait defaults apply); enums emit both.
    let body = match &input.data {
        Data::Struct(s) => {
            // Only unit structs (no fields). A SystemSet is a pure marker;
            // per-instance state contradicts the identity model.
            if !matches!(&s.fields, Fields::Unit) {
                return syn::Error::new(
                    name_span,
                    "SystemSet derive requires a unit struct (no fields)",
                )
                .to_compile_error()
                .into();
            }
            // Unit struct → no override; trait defaults (disc 0, type name).
            TokenStream2::new()
        }
        Data::Enum(e) => match system_set_enum_body(&name, e) {
            Ok(tokens) => tokens,
            Err(err) => return err.to_compile_error().into(),
        },
        Data::Union(_) => {
            return syn::Error::new(
                name_span,
                "SystemSet can only be derived for unit structs or fieldless enums",
            )
            .to_compile_error()
            .into();
        }
    };

    let expanded = quote! {
        impl ::boyko_ecs::ecs::core::schedule::SystemSet for #name {
            #body
        }
    };

    expanded.into()
}

/// Generates the `set_discriminant` + `set_name` method bodies for an enum
/// `SystemSet`. Each fieldless variant maps to its index. A data-carrying
/// variant is a hard error (no stable type-level identity).
fn system_set_enum_body(
    name: &Ident,
    data: &syn::DataEnum,
) -> syn::Result<TokenStream2> {
    let mut disc_arms: Vec<TokenStream2> = Vec::with_capacity(data.variants.len());
    let mut name_arms: Vec<TokenStream2> = Vec::with_capacity(data.variants.len());

    for (index, variant) in data.variants.iter().enumerate() {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new(
                variant.ident.span(),
                "SystemSet enum variants must be unit variants (no fields)",
            ));
        }
        let variant_ident = &variant.ident;
        let disc = index as u32;
        let qualified = format!("{name}::{variant_ident}");
        disc_arms.push(quote! { #name::#variant_ident => #disc });
        name_arms.push(quote! { #name::#variant_ident => #qualified });
    }

    Ok(quote! {
        #[inline]
        fn set_discriminant(&self) -> u32 {
            match self {
                #(#disc_arms),*
            }
        }

        #[inline]
        fn set_name(&self) -> &'static str {
            match self {
                #(#name_arms),*
            }
        }
    })
}
