//! Phase 15 — `compile_fail` acceptance tests for `#[derive(SystemSet)]`.
//!
//! Each `.rs` file under `tests/system_set_compile_fail/` is compiled in
//! isolation; the matching `.stderr` baseline records the expected compiler
//! diagnostic. Regenerate the baselines after a rustc / syn point release that
//! shifts diagnostic wording via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test system_set_compile_fail
//! ```
//!
//! # Covered cases (per `system_set_macro` in `boyko_macros/src/lib.rs`)
//!
//! | File                          | Rejected input                                   |
//! |-------------------------------|--------------------------------------------------|
//! | `data_carrying_variant.rs`    | `enum E { V(u32) }` (variant with fields)        |
//! | `union_rejected.rs`           | `#[derive(SystemSet)] union U { .. }`            |
//! | `generic_rejected.rs`         | `#[derive(SystemSet)] struct G<T>(..)`           |
//! | `tuple_struct_rejected.rs`    | `#[derive(SystemSet)] struct T(u32)` (has field) |
//!
//! Gated behind `#[cfg(not(miri))]` — trybuild is not wired under Miri (mirrors
//! `compile_fail_hooks.rs` / `compile_fail_chunk.rs`).

#![cfg(not(miri))]

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/system_set_compile_fail/*.rs");
}
