//! `#[derive(Bindable)]` implementation.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, parse_macro_input};

/// Implementation of `#[derive(Bindable)]` (see the public entry in `lib.rs`).
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();

    // Bindable supports NAMED structs only — binding is by field name.
    let named = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(named) => named,
            _ => {
                return syn::Error::new(
                    name.span(),
                    "#[derive(Bindable)] requires a struct with named fields \
                     (binding is by field name)",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new(
                name.span(),
                "#[derive(Bindable)] can only be derived for structs with named fields",
            )
            .to_compile_error()
            .into();
        }
    };

    let field_idents: Vec<Ident> = named
        .named
        .iter()
        .map(|f| f.ident.clone().expect("named field has ident"))
        .collect();
    let field_count = field_idents.len();
    if field_count > u8::MAX as usize {
        return syn::Error::new(
            name.span(),
            "#[derive(Bindable)] supports at most 255 fields",
        )
        .to_compile_error()
        .into();
    }
    let field_count_u8 = field_count as u8;

    // Per-field `u8` ids (declaration order) + name literals.
    let ids: Vec<u8> = (0..field_count as u8).collect();
    let name_strs: Vec<String> = field_idents.iter().map(|i| i.to_string()).collect();

    // `fmt_field` arms: `id => write!(out, "{}", self.<field>)`.
    let fmt_arms = field_idents.iter().zip(ids.iter()).map(|(id_field, k)| {
        quote! { #k => ::core::write!(out, "{}", self.#id_field), }
    });
    // `value_field` arms: `id => self.<field> as f32`.
    let value_arms = field_idents.iter().zip(ids.iter()).map(|(id_field, k)| {
        quote! { #k => self.#id_field as f32, }
    });
    // `field_id` arms: `"name" => Some(id)`.
    let name_arms = name_strs.iter().zip(ids.iter()).map(|(s, k)| {
        quote! { #s => ::core::option::Option::Some(#k), }
    });

    let expanded = quote! {
        impl ::boyko_ui::binding::Bindable for #name {
            const FIELD_COUNT: u8 = #field_count_u8;

            #[inline]
            fn fmt_field(
                &self,
                field: u8,
                out: &mut dyn ::core::fmt::Write,
            ) -> ::core::fmt::Result {
                match field {
                    #(#fmt_arms)*
                    _ => ::core::result::Result::Ok(()),
                }
            }

            #[inline]
            fn value_field(&self, field: u8) -> f32 {
                match field {
                    #(#value_arms)*
                    _ => 0.0,
                }
            }

            fn field_id(name: &str) -> ::core::option::Option<u8> {
                match name {
                    #(#name_arms)*
                    _ => ::core::option::Option::None,
                }
            }

            fn register_bind_accessor() {
                fn fmt_erased(
                    p: *const u8,
                    f: u8,
                    out: &mut dyn ::core::fmt::Write,
                ) -> ::core::fmt::Result {
                    // SAFETY: `p` was obtained by the caller (`ui_bind_apply`) from
                    // `EcsMaster::get_component_raw(source, Self::component_id())`,
                    // which returns `Some` ONLY when `source` is alive AND its
                    // archetype hosts this exact ComponentId — so the bytes at `p`
                    // are a live, aligned instance of this type. `ComponentId` IS
                    // the type's identity, so no TypeId check is needed; the
                    // None-skip in the caller is the precondition.
                    let this: &#name = unsafe { &*(p as *const #name) };
                    <#name as ::boyko_ui::binding::Bindable>::fmt_field(this, f, out)
                }
                fn value_erased(p: *const u8, f: u8) -> f32 {
                    // SAFETY: as `fmt_erased` — `p` is a live, aligned `*const`
                    // instance of this type (caller's None-skip precondition).
                    let this: &#name = unsafe { &*(p as *const #name) };
                    <#name as ::boyko_ui::binding::Bindable>::value_field(this, f)
                }
                ::boyko_ecs::ecs::core::component::component_registry::install_bind_accessor(
                    <#name as ::boyko_ecs::ecs::core::component::component::Component>::component_id().0,
                    ::boyko_ecs::ecs::core::component::component_registry::BindAccessor {
                        fmt: fmt_erased,
                        value: value_erased,
                    },
                );
            }
        }
    };

    expanded.into()
}
