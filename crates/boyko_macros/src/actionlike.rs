//! `#[derive(Actionlike)]` implementation.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, parse_macro_input};

/// Implementation of `#[derive(Actionlike)]` (see the public entry in `lib.rs`).
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let name_span = name.span();

    // Generics defeat the fixed `[…; COUNT]` array sizing (a generic action set
    // would mint a fresh COUNT per monomorphisation).
    if !input.generics.params.is_empty() {
        return syn::Error::new(
            name_span,
            "Actionlike derive does not support generics (the action set must be a fixed enum)",
        )
        .to_compile_error()
        .into();
    }

    let data = match &input.data {
        Data::Enum(e) => e,
        Data::Struct(_) | Data::Union(_) => {
            return syn::Error::new(
                name_span,
                "Actionlike can only be derived for a fieldless enum",
            )
            .to_compile_error()
            .into();
        }
    };

    if data.variants.is_empty() {
        return syn::Error::new(
            name_span,
            "Actionlike enum must declare at least one variant (COUNT == 0 is unusable)",
        )
        .to_compile_error()
        .into();
    }

    let count = data.variants.len();

    let mut index_arms: Vec<TokenStream2> = Vec::with_capacity(count);
    let mut from_index_arms: Vec<TokenStream2> = Vec::with_capacity(count);
    let mut kind_arms: Vec<TokenStream2> = Vec::with_capacity(count);
    let mut name_arms: Vec<TokenStream2> = Vec::with_capacity(count);

    for (index, variant) in data.variants.iter().enumerate() {
        if !matches!(variant.fields, Fields::Unit) {
            return syn::Error::new(
                variant.ident.span(),
                "Actionlike variants must be fieldless (no stable dense index otherwise)",
            )
            .to_compile_error()
            .into();
        }

        let variant_ident = &variant.ident;
        let idx = index;
        let variant_name = variant_ident.to_string();

        let kind = match actionlike_variant_kind(variant) {
            Ok(k) => k,
            Err(err) => return err.to_compile_error().into(),
        };

        index_arms.push(quote! { #name::#variant_ident => #idx });
        from_index_arms.push(quote! { #idx => ::core::option::Option::Some(#name::#variant_ident) });
        kind_arms.push(quote! { #name::#variant_ident => #kind });
        name_arms.push(quote! { #name::#variant_ident => #variant_name });
    }

    let expanded = quote! {
        impl ::boyko_input::action::actionlike::Actionlike for #name {
            const COUNT: usize = #count;

            #[inline]
            fn index(self) -> usize {
                match self {
                    #(#index_arms),*
                }
            }

            #[inline]
            fn from_index(i: usize) -> ::core::option::Option<Self> {
                match i {
                    #(#from_index_arms,)*
                    _ => ::core::option::Option::None,
                }
            }

            #[inline]
            fn kind(self) -> ::boyko_input::action::actionlike::ActionKind {
                match self {
                    #(#kind_arms),*
                }
            }

            #[inline]
            fn name(self) -> &'static str {
                match self {
                    #(#name_arms),*
                }
            }
        }

        // V8: the action count must fit a `BitSet256`. A `COUNT > 256` enum is a
        // cold exotic case (real maps run 10–60); enforce the cap at compile
        // time so `ActionState`'s bitset addressing is always in bounds.
        const _: () = ::core::assert!(
            <#name as ::boyko_input::action::actionlike::Actionlike>::COUNT <= 256,
            "Actionlike enum exceeds BitSet256 capacity (256 actions max)"
        );
    };

    expanded.into()
}

/// Parses the optional `#[actionlike(Button|Axis1D|Axis2D)]` attribute on a
/// variant, returning the `ActionKind` token (defaulting to `Button`).
fn actionlike_variant_kind(
    variant: &syn::Variant,
) -> syn::Result<TokenStream2> {
    let mut kind: Option<TokenStream2> = None;

    for attr in &variant.attrs {
        if !attr.path().is_ident("actionlike") {
            continue;
        }
        let attr_span = attr
            .path()
            .get_ident()
            .map_or_else(Span::call_site, |i| i.span());
        if kind.is_some() {
            return Err(syn::Error::new(
                attr_span,
                "duplicate #[actionlike(...)] on a single variant",
            ));
        }
        // The attribute body is a single identifier: Button | Axis1D | Axis2D.
        let ident: Ident = attr.parse_args().map_err(|_| {
            syn::Error::new(
                attr_span,
                "expected #[actionlike(Button | Axis1D | Axis2D)]",
            )
        })?;
        let resolved = match ident.to_string().as_str() {
            "Button" => quote! { ::boyko_input::action::actionlike::ActionKind::Button },
            "Axis1D" => quote! { ::boyko_input::action::actionlike::ActionKind::Axis1D },
            "Axis2D" => quote! { ::boyko_input::action::actionlike::ActionKind::Axis2D },
            other => {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("unknown action kind `{other}` (expected Button, Axis1D, or Axis2D)"),
                ));
            }
        };
        kind = Some(resolved);
    }

    Ok(kind
        .unwrap_or_else(|| quote! { ::boyko_input::action::actionlike::ActionKind::Button }))
}
