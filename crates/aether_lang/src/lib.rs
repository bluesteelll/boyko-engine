//! `aether-lang` — parser, diagnostics and expander for the Aether authoring DSL
//! (docs/AETHER-LANG-PLAN.md; through rung **A7**, the plan's last: the whole §6.1 construct
//! registry — `component`, `tag`, `bundle`, `system`, `event`, `plugin`, `machine`, `material`,
//! `scene` — plus §6.3's `aether v1;` header, §7.3's recovery stubs and §8 R1's expansion-volume
//! measurement).
//!
//! # Architecture (the plan's decisions, enforced by this crate's shape)
//!
//! * **One umbrella parse context** (Decision A1): [`expand_block`] parses a whole `aether!`
//!   block into a list of constructs, so cross-construct references (a `system` naming a sibling
//!   for ordering, a `scene` naming a `material`) resolve inside ONE context — the reason
//!   per-construct macros were rejected. §4's pipeline is literal here: `parse.rs` builds the
//!   construct list, `ctx.rs` turns it into the block's symbol table and runs every whole-block
//!   rule, and `expand.rs` emits with that table in hand.
//! * **A plain library** (Decision A2): everything here is testable without macro expansion;
//!   the `aether` proc-macro crate is a two-line shim.
//! * **Emit the canonical hand-written surface** (Decision A3): the expander produces exactly
//!   the `#[derive(::boyko_macros::Component)]` items a person would write, and `boyko_macros`
//!   remains the single codegen authority. Zero drift by construction; every engine path in the
//!   output is a TOKEN, never a dependency (the `boyko_macros` no-cycle rule).
//! * **Verbatim span-preserved Rust fragments** (§2): field types and hook paths pass through
//!   with their original spans, so rustc/rust-analyzer report errors at the user's tokens.
//!
//! # Test-channel note (a recorded deviation, not an omission)
//!
//! The plan's rung table names `macrotest` snapshots. `macrotest` is a NEW third-party
//! dev-dependency this workspace does not carry, and the no-new-third-party rule outranks the
//! rung table's tooling suggestion. The same coverage lands through this crate's own unit tests:
//! [`expand_block`] is a plain function, and the tests pin its output token-for-token — which is
//! strictly MORE precise than macrotest's rustfmt-normalized snapshots.
//!
//! §8 R1's expansion-size measurement rides on that same corpus (`expand::tests`'s
//! `expansion_volume_stays_inside_its_measured_band`), which is what the plan asked macrotest's
//! output for: expanded volume per snapshot, measured in CI rather than noticed in a build time.
//!
//! # Error recovery (§7.3), for a reader of this crate
//!
//! A parse failure does NOT abort the block. Each construct is parsed speculatively; a failure
//! records its error plus a best-effort stub and the parser resyncs on the next construct head,
//! so a block yields one error per fault, a name-resolving stub per fault, and the expansion of
//! every construct that parsed.
//!
//! A broken construct is not treated as absent. §4's whole-block rules run over
//! `constructs ∪ broken` ([`mod@ctx`]): a `plugin` that failed to parse still holds the plugin
//! slot, so its siblings' clauses do not report a second fault; a `material gold` that failed to
//! parse still occupies the name `gold`, so a real duplicate is still Aether's own two-span
//! diagnostic rather than rustc's E0428 on the macro token. What stays suppressed is narrower and
//! specific: a rule whose failure could not exist WITHOUT the break — the broken plugin's own
//! registration contents, an ordering edge naming a system that did not parse, a scene minting a
//! material that did not parse. Those come back the moment the construct does.

mod ast;
mod ctx;
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
