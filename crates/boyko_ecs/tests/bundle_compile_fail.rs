//! Phase 8.5 Step 9 — `compile_fail` acceptance tests for `#[derive(Bundle)]`.
//!
//! Each `.rs` file in `tests/bundle_compile_fail/` is expected to fail to
//! compile with the diagnostic recorded in its matching `.stderr` file.
//! Regenerate the `.stderr` files when adding or revising cases via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test bundle_compile_fail
//! ```
//!
//! Covered cases (per plan §9 Step 9):
//!
//! * `unit_struct.rs`        — `#[derive(Bundle)] struct Marker;` →
//!                              "Bundle requires at least one field".
//! * `generic_struct.rs`     — `#[derive(Bundle)] struct G<T> { ... }` →
//!                              "Bundle derive does not support generics".
//! * `non_component_field.rs`— `#[derive(Bundle)] struct B { x: u32 }` →
//!                              `u32: Component` is not satisfied.
//! * `manual_impl_blocked.rs`— manual `impl Bundle for Foo` outside the
//!                              macro → seal-trait error.

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/bundle_compile_fail/*.rs");
}
