//! `aether!` — the boyko-engine authoring DSL's ONE umbrella macro
//! (docs/AETHER-LANG-PLAN.md Decision A1).
//!
//! This crate is deliberately a two-line shim: all parsing, diagnostics and expansion live in
//! [`aether_lang`], which is a PLAIN library and therefore unit-testable without macro expansion
//! (Decision A2 — the Dioxus-proven tooling split). Keeping the proc-macro crate empty of logic
//! is what keeps the logic testable.

use proc_macro::TokenStream;

/// The Aether authoring block: components, tags (rung A0) — later rungs add bundles, events,
/// systems, machines, materials and scenes (docs/AETHER-LANG-PLAN.md §3, §9).
///
/// ```ignore
/// aether! {
///     component Health {
///         current: f32,
///         max: f32,
///     }
///
///     tag Player;
///     tag Stunned(bitset);
/// }
/// ```
///
/// Expands to the canonical hand-written surface (`#[derive(::boyko_macros::Component)]` items —
/// Decision A3: one expansion authority, zero drift), so everything the derive supports flows
/// through unchanged.
#[proc_macro]
pub fn aether(input: TokenStream) -> TokenStream {
    aether_lang::expand_block(input.into()).into()
}
