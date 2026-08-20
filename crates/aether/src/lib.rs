//! `aether!` — the boyko-engine authoring DSL's ONE umbrella macro
//! (docs/AETHER-LANG-PLAN.md Decision A1).
//!
//! This crate is deliberately a two-line shim: all parsing, diagnostics and expansion live in
//! [`aether_lang`], which is a PLAIN library and therefore unit-testable without macro expansion
//! (Decision A2 — the Dioxus-proven tooling split). Keeping the proc-macro crate empty of logic
//! is what keeps the logic testable.

use proc_macro::TokenStream;

/// The Aether authoring block. As of rung A6 the whole v1 construct registry dispatches:
/// `component`, `tag`, `bundle`, `system`, `event`, `plugin`, `machine`, `material`, `scene`
/// (docs/AETHER-LANG-PLAN.md §3, §9).
///
/// ```ignore
/// aether! {
///     plugin Arena;
///
///     component Health {
///         current: f32,
///         max: f32,
///     }
///
///     tag Player;
///     tag Stunned(bitset);
///
///     material gold { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
///
///     // A scene declares the world's initial entities IN THE LANGUAGE, and the sibling
///     // `plugin` registers its spawn fn as a startup one-shot.
///     scene arena {
///         let floor = plane(22.0);
///
///         mesh floor;
///         mesh floor at (0.0, 1.0, 0.0) { material: gold, casts_shadow };
///
///         sun { dir: (-0.42, 0.80, 0.42), lux: 3.2 }
///     }
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
