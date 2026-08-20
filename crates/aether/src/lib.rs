//! `aether!` — the boyko-engine authoring DSL's ONE umbrella macro
//! (docs/AETHER-LANG-PLAN.md Decision A1).
//!
//! This crate is deliberately a two-line shim: all parsing, diagnostics and expansion live in
//! [`aether_lang`], which is a PLAIN library and therefore unit-testable without macro expansion
//! (Decision A2 — the Dioxus-proven tooling split). Keeping the proc-macro crate empty of logic
//! is what keeps the logic testable.
//!
//! # Debugging an expansion: `cargo expand` is the front door
//!
//! ```text
//! cargo expand -p aether_tests --test a2_system_plugin    # one test target's expansion
//! cargo expand -p my_game 2>&1 | less                     # a whole crate's
//! ```
//!
//! Everything `aether!` emits is ordinary Rust that a person could have written (Decision A3), so
//! the expansion is readable rather than a wall of generated glue: `component` becomes a
//! `#[derive(::boyko_macros::Component)]` struct, `system` a `pub fn`, `machine` a flat enum plus
//! one drain-and-act fn per (leaf, event), `scene` a single spawn fn. If a construct's output
//! surprises you, `aether_lang`'s unit tests pin every one of those shapes token-for-token —
//! they are the same content a `cargo expand` shows, versioned.
//!
//! Generated internal names are `__aether_`-prefixed (`__aether_game_flow__boot__assets_ready`,
//! `__aether_commands`) and never collide with user names, so a name WITHOUT that prefix in an
//! error message is one you wrote.
//!
//! # rust-analyzer, honestly
//!
//! * Completion, hover and go-to-definition work inside `EXPR` / `TYPE` / `BLOCK` positions —
//!   those are verbatim Rust tokens with your spans (§7.2) — **once the block parses**.
//! * They do NOT work mid-keyword, while a new clause head is half-typed. That is inherent to
//!   macro DSLs, not specific to this one.
//! * A block that does not parse still analyzes: one broken construct yields ONE error plus a
//!   name-resolving stub for it, and every other construct in the block expands in full (§7.3).
//!   So the file around the line you are editing stays healthy.
//! * Formatting inside bodies is preserved as written. There is no `aetherfmt` in v1 (planned
//!   for, not blocked on — the `leptosfmt` lesson).
//! * Syntax highlighting is Rust's; a TextMate/tree-sitter grammar is post-v1. The grammar is
//!   Rust-lexable and bodies ARE Rust, which is why v1 reads acceptably without one.
//!
//! # Syntax version
//!
//! A block may open with `aether v1;` (§6.3). Absent, it is read as the current version — the
//! header is insurance for the day a v2 grammar breaks v1, not something v1 code needs. A version
//! this crate does not speak is refused on the version token itself.
//!
//! # DX checklist — for anyone adding or changing a construct
//!
//! The quality bar §7 sets, as the list a change is reviewed against. Every item here is
//! something that has already gone wrong somewhere in this repo or in the prior art the plan
//! surveyed:
//!
//! 1. **Verbatim tokens, never strings.** User fragments (idents, types, exprs, blocks) are
//!    carried as parsed `syn` nodes and re-emitted unchanged. A `stringify!` + re-parse round
//!    trip loses spans, and everything below depends on spans.
//! 2. **The narrowest applicable span.** The offending token — not the construct, not the block.
//!    An error spanned at `Span::call_site()` lands on the `aether!` token, where the reader has
//!    no idea which of their forty lines is meant.
//! 3. **Every diagnostic gets a trybuild golden.** The message is half the contract; the LINE AND
//!    COLUMN in the `.stderr` are the other half, and they are the half that silently degrades.
//! 4. **Accumulate, do not abort.** Independent constructs all report; a broken one still emits
//!    its stub so its name resolves (§7.3).
//! 5. **Pre-check only where Aether is strictly better.** A fault rustc or a derive reports
//!    against the user's own tokens is left to them — duplicated checks drift apart. Aether owns
//!    the faults rustc can only report on GENERATED tokens.
//! 6. **One table, spelling and dispatch together.** What a message advertises and what the
//!    parser accepts must be the same rows (`MATERIAL_KEYS`, the node key tables, the version
//!    table). Two parallel lists drift in opposite directions and each direction is a bug.
//! 7. **"Did you mean" at edit distance ≤ 2**, against the same table.
//! 8. **Emit the canonical hand-written surface.** Codegen belongs in `boyko_macros`; Aether's
//!    output should read like what a person would have typed. `aether_lang`'s expansion-volume
//!    test measures the result and fails on drift in either direction.
//! 9. **Engine paths are TOKENS, never dependencies** — and they must be the REAL nested paths
//!    (`::boyko_ecs::ecs::core::system::Res`, not the plan's idealized spelling), verified by an
//!    `aether_tests` target that compiles them against the real crates.
//! 10. **Never panic.** A panicking proc-macro erases the block from analysis; every internal
//!     failure becomes a spanned `compile_error!`.

use proc_macro::TokenStream;

/// The Aether authoring block. The whole v1 construct registry dispatches: `component`, `tag`,
/// `bundle`, `system`, `event`, `plugin`, `machine`, `material`, `scene`
/// (docs/AETHER-LANG-PLAN.md §3, §9 — complete as of rung A7, the plan's last).
///
/// ```ignore
/// aether! {
///     aether v1;          // optional (§6.3); absent = this crate's current syntax
///
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
