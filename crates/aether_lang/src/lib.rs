//! `aether-lang` — parser, diagnostics and expander for the Aether authoring DSL
//! (docs/AETHER-LANG-PLAN.md; rung **A0**: `component` + `tag`, end to end).
//!
//! # Architecture (the plan's decisions, enforced by this crate's shape)
//!
//! * **One umbrella parse context** (Decision A1): [`expand_block`] parses a whole `aether!`
//!   block into a list of constructs, so later rungs' cross-construct references (a `system`
//!   naming a sibling for ordering, a `scene` naming a `material`) resolve inside ONE context —
//!   the reason per-construct macros were rejected.
//! * **A plain library** (Decision A2): everything here is testable without macro expansion;
//!   the `aether` proc-macro crate is a two-line shim.
//! * **Emit the canonical hand-written surface** (Decision A3): the expander produces exactly
//!   the `#[derive(::boyko_macros::Component)]` items a person would write, and `boyko_macros`
//!   remains the single codegen authority. Zero drift by construction; every engine path in the
//!   output is a TOKEN, never a dependency (the `boyko_macros` no-cycle rule).
//! * **Verbatim span-preserved Rust fragments** (§2): field types and hook paths pass through
//!   with their original spans, so rustc/rust-analyzer report errors at the user's tokens.
//!
//! # A0 test-channel note (a recorded deviation, not an omission)
//!
//! The plan's rung table names `macrotest` snapshots. `macrotest` is a NEW third-party
//! dev-dependency this workspace does not carry, and the no-new-third-party rule outranks the
//! rung table's tooling suggestion. The same coverage lands through this crate's own unit tests:
//! [`expand_block`] is a plain function, and the tests pin its output token-for-token — which is
//! strictly MORE precise than macrotest's rustfmt-normalized snapshots.

mod ast;
mod diag;
mod expand;
mod parse;

use proc_macro2::TokenStream;

/// Parse one `aether!` block and expand it to a flat list of top-level Rust items.
///
/// Never panics: every parse or validation failure becomes a `compile_error!` invocation
/// carrying the failure's own span, so the downstream crate's build shows the error at the
/// offending Aether token rather than at the macro call site.
pub fn expand_block(input: TokenStream) -> TokenStream {
    match syn::parse2::<ast::AetherBlock>(input) {
        Ok(block) => expand::expand(&block),
        Err(e) => e.to_compile_error(),
    }
}
