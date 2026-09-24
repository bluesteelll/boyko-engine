//! `#[derive(Resource)]` implementation.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput};

/// Implementation of `#[derive(Resource)]` (see the public entry in `lib.rs`).
pub(crate) fn resource_macro_impl(input: TokenStream) -> TokenStream {
    let input = crate::common::parse2_or_compile_error!(input as DeriveInput);
    let name = input.ident;

    let expanded = quote! {
        impl ::boyko_ecs::ecs::core::resources::resource::Resource for #name {
            #[inline]
            fn resource_id() -> ::boyko_ecs::ecs::identifiers::primitives::ResourceId {
                static ID: ::std::sync::OnceLock<
                    ::boyko_ecs::ecs::identifiers::primitives::ResourceId
                > = ::std::sync::OnceLock::new();
                // Route through the `resources` module's `pub use` of
                // `register_new` so the macro path stays inside the public
                // surface — the `resource_registry` module itself is
                // `pub(crate)` per Q6 RESOLUTION.
                *ID.get_or_init(|| ::boyko_ecs::ecs::identifiers::primitives::ResourceId(
                    ::boyko_ecs::ecs::core::resources::register_new::<Self>()
                ))
            }
        }
    };

    expanded
}
