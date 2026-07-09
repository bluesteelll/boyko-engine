//! Relations v1 — R5: `compile_fail` acceptance tests for the two derives + the
//! W2 structural cascade-soundness guard (`docs/RELATIONS-API-PLAN.md` §R5).
//!
//! Each `.rs` under `tests/compile_fail_relations/` is compiled in ISOLATION; the
//! matching `.stderr` baseline records the expected diagnostic. Regenerate after a
//! rustc point release that shifts the wording via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test compile_fail_relations
//! ```
//!
//! Gated behind `#[cfg(not(miri))]` — trybuild is not wired under Miri (mirrors
//! `compile_fail_observers.rs` / `compile_fail_hooks.rs`).

#![cfg(not(miri))]

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_relations/*.rs");
}
