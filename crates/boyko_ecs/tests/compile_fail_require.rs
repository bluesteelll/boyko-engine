//! Feature 1 (required components) — `compile_fail` acceptance tests for the
//! `#[require(...)]` derive attribute.
//!
//! Spec `docs/REQUIRED-COMPONENTS-PLAN.md` Resolved open questions: a duplicate
//! same-id `#[require(B, B)]` is a COMPILE error (the macro sees both keys —
//! strictly better than Bevy's runtime panic); an empty `#[require()]` is
//! rejected.
//!
//! Each `.rs` file under `tests/compile_fail_require/` is compiled in isolation;
//! the matching `.stderr` baseline records the expected compiler diagnostic.
//! Regenerate the baselines after a rustc / syn point release that shifts
//! diagnostic wording via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test compile_fail_require
//! ```
//!
//! | File                          | Rejected input                              |
//! |-------------------------------|---------------------------------------------|
//! | `duplicate_require_rejected.rs` | `#[require(B, B)]` (same id twice)        |
//! | `empty_require_rejected.rs`     | `#[require()]` (no entries)               |
//!
//! Gated behind `#[cfg(not(miri))]` — trybuild is not wired under Miri (mirrors
//! `compile_fail_hooks.rs`).

#![cfg(not(miri))]

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_require/*.rs");
}
