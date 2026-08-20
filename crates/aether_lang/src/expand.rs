//! The expander — Decision A3: emit the CANONICAL hand-written surface and let `boyko_macros`
//! do the codegen. One expansion authority, zero drift, minimal expansion volume (§8 R1); every
//! engine path below is a TOKEN resolved downstream, never a dependency of this crate.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::ast::{AetherBlock, ComponentDef, Construct, TagDef};

/// Expand a parsed block to the flat item list, in source order (deterministic output is what
/// the unit tests pin token-for-token).
pub fn expand(block: &AetherBlock) -> TokenStream {
    let mut out = TokenStream::new();
    for c in &block.constructs {
        match c {
            Construct::Component(def) => out.extend(component(def)),
            Construct::Tag(def) => out.extend(tag(def)),
        }
    }
    out
}

/// §3.1: `component` → `#[derive(::boyko_macros::Component)]` struct with the derive's own
/// attribute surface (`#[require(...)]`, `#[component(on_* = path, no_bundle)]`), fields `pub`.
fn component(def: &ComponentDef) -> TokenStream {
    let name = &def.name;
    let requires = (!def.requires.is_empty()).then(|| {
        let paths = &def.requires;
        quote! { #[require( #( #paths ),* )] }
    });
    let component_attr = {
        let mut keys: Vec<TokenStream> = Vec::new();
        for (kind, path) in &def.hooks {
            let key = Ident::new(kind.key(), proc_macro2::Span::call_site());
            keys.push(quote! { #key = #path });
        }
        if def.no_bundle {
            keys.push(quote! { no_bundle });
        }
        (!keys.is_empty()).then(|| quote! { #[component( #( #keys ),* )] })
    };
    let fields = def.fields.iter().map(|(fname, ty)| quote! { pub #fname: #ty });
    quote! {
        #[derive(::boyko_macros::Component)]
        #requires
        #component_attr
        pub struct #name {
            #( #fields ),*
        }
    }
}

/// §3.1: `tag` → a ZST component (the derive's auto-tag detection does the rest); `(bitset)`
/// adds `#[component(storage = "bitset")]` — the EnableTag backend. The parser already enforced
/// the "bitset ⇒ fieldless" rule by grammar (tags cannot carry fields at all), and the derive's
/// own check remains the authority.
fn tag(def: &TagDef) -> TokenStream {
    let name = &def.name;
    let storage = def.bitset.then(|| quote! { #[component(storage = "bitset")] });
    quote! {
        #[derive(::boyko_macros::Component)]
        #storage
        pub struct #name;
    }
}

#[cfg(test)]
mod tests {
    //! The A0 snapshot channel (see the crate doc's macrotest note): `expand_block` is a plain
    //! function, and these tests pin its output token-for-token — parse and expansion in one
    //! assertion, against the §3.1 before/after pair VERBATIM.

    use quote::quote;

    /// Normalized (whitespace-insensitive) token equality: `TokenStream::to_string` is already
    /// canonical for identical streams, so a plain string compare IS token equality.
    fn expands_to(input: proc_macro2::TokenStream, expected: proc_macro2::TokenStream) {
        let got = crate::expand_block(input).to_string();
        let want = expected.to_string();
        assert_eq!(got, want, "expansion drifted from the pinned §3.1 surface");
    }

    #[test]
    fn the_section_3_1_before_after_pair_holds_verbatim() {
        expands_to(
            quote! {
                component Health {
                    current: f32,
                    max: f32,
                    requires Regen,
                    on_add = heal_full,
                }

                tag Player;
                tag Stunned(bitset);
            },
            quote! {
                #[derive(::boyko_macros::Component)]
                #[require(Regen)]
                #[component(on_add = heal_full)]
                pub struct Health {
                    pub current: f32,
                    pub max: f32
                }
                #[derive(::boyko_macros::Component)]
                pub struct Player;
                #[derive(::boyko_macros::Component)]
                #[component(storage = "bitset")]
                pub struct Stunned;
            },
        );
    }

    #[test]
    fn no_bundle_and_multi_requires_and_every_hook_key_forward() {
        expands_to(
            quote! {
                component Rig {
                    bone: u32,
                    requires A, b::C,
                    on_insert = f::g,
                    on_remove = h,
                    no_bundle,
                }
            },
            quote! {
                #[derive(::boyko_macros::Component)]
                #[require(A, b::C)]
                #[component(on_insert = f::g, on_remove = h, no_bundle)]
                pub struct Rig {
                    pub bone: u32
                }
            },
        );
    }

    #[test]
    fn a_fieldless_component_is_a_plain_zst() {
        expands_to(
            quote! { component Marker {} },
            quote! {
                #[derive(::boyko_macros::Component)]
                pub struct Marker {}
            },
        );
    }

    /// Every diagnostic below asserts the MESSAGE (the contract a user reads), not the span —
    /// span pinning is rung A7's column-exact sweep.
    fn fails_with(input: proc_macro2::TokenStream, needle: &str) {
        let out = crate::expand_block(input).to_string();
        assert!(
            out.contains("compile_error") && out.contains(needle),
            "expected a compile_error containing {needle:?}, got: {out}"
        );
    }

    #[test]
    fn unknown_construct_lists_the_registry_and_suggests() {
        fails_with(
            quote! { compnent Health {} },
            "unknown construct `compnent`",
        );
        fails_with(quote! { compnent Health {} }, "did you mean `component`?");
    }

    #[test]
    fn planned_constructs_name_their_rung_instead_of_pretending_unknown() {
        fails_with(quote! { system foo() {} }, "lands at rung A2");
        fails_with(quote! { machine G {} }, "lands at rung A3");
    }

    #[test]
    fn case_convention_diagnosed_with_a_rename() {
        fails_with(quote! { component health { hp: f32 } }, "rename `health` to `Health`");
        fails_with(quote! { tag player; }, "rename `player` to `Player`");
    }

    #[test]
    fn duplicate_hooks_and_bad_tag_modifiers_are_refused() {
        fails_with(
            quote! { component A { on_add = f, on_add = g } },
            "duplicate hook `on_add`",
        );
        fails_with(quote! { tag T(dense); }, "unknown tag modifier `dense`");
    }
}
