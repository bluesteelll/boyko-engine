//! Phase 14a — `compile_fail` acceptance tests for the `#[component(...)]`
//! lifecycle-hook attribute (plan §8 test surface bullet 6).
//!
//! Each `.rs` file under `tests/compile_fail_hooks/` is compiled in isolation;
//! the matching `.stderr` baseline records the expected compiler diagnostic.
//! Regenerate the baselines after a rustc / syn point release that shifts
//! diagnostic wording via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test compile_fail_hooks
//! ```
//!
//! # Covered cases (per the derive macro's `parse_component_hooks`)
//!
//! | File                          | Rejected input                                  |
//! |-------------------------------|-------------------------------------------------|
//! | `on_despawn_rejected.rs`      | `#[component(on_despawn = x)]` (deferred to 14b)|
//! | `unknown_key_rejected.rs`     | `#[component(bogus = x)]` (unknown key)         |
//! | `duplicate_key_rejected.rs`   | `#[component(on_add = a, on_add = b)]`          |
//! | `duplicate_attr_rejected.rs`  | two separate `#[component(...)]` attrs          |
//! | `missing_value_rejected.rs`   | `#[component(on_add)]` (key missing `= path`)   |
//!
//! Gated behind `#[cfg(not(miri))]` — trybuild is not wired under Miri (mirrors
//! `compile_fail_chunk.rs`).

#![cfg(not(miri))]

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_hooks/*.rs");
}
