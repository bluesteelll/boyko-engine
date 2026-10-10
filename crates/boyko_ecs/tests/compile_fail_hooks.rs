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
//! | File                            | Rejected input                                |
//! |---------------------------------|-----------------------------------------------|
//! | `bitset_on_despawn_rejected.rs` | `storage = "bitset"` + a lifecycle hook       |
//! | `unknown_key_rejected.rs`       | `#[component(bogus = x)]` (unknown key)       |
//! | `duplicate_key_rejected.rs`     | `#[component(on_add = a, on_add = b)]`        |
//! | `duplicate_attr_rejected.rs`    | two separate `#[component(...)]` attrs        |
//! | `missing_value_rejected.rs`     | `#[component(on_add)]` (key missing `= path`) |
//!
//! KM2 retired `on_despawn_rejected.rs`: the key it pinned ("not supported in
//! this version, deferred to Phase 14b") is now a valid key, so the fixture
//! guarded nothing. `bitset_on_despawn_rejected.rs` replaces it and pins the
//! refusal KM2 *creates* instead.
//!
//! Gated behind `#[cfg(not(miri))]` — trybuild is not wired under Miri (mirrors
//! `compile_fail_chunk.rs`).

#![cfg(not(miri))]

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_hooks/*.rs");
}
