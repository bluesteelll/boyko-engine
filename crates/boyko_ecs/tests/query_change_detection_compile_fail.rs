//! W4 / I-NEW-4 / QV11 — `compile_fail` acceptance tests for the
//! change-detection reject on `EcsMaster::query<D, F>()`.
//!
//! `EcsMaster::query` does **not** support change-detection filters: any call
//! with `D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION == true`
//! (`Ref<T>` / `Mut<T>` / `Added<C>` / `Changed<C>`) is a compile error via the
//! W4 `const`-assert. Each `.rs` file in
//! `tests/query_change_detection_compile_fail/` must fail to compile with the
//! diagnostic recorded in its matching `.stderr` file. Regenerate the `.stderr`
//! files on toolchain bumps (snapshot-based compile-fail tests are
//! toolchain-coupled) via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test query_change_detection_compile_fail
//! ```
//!
//! # Two-trigger note
//!
//! The W4 reject has two `const`-evaluation triggers (the Phase-12.5 "const
//! must be in a forcing context" lesson): the inline `const {}` block inside
//! the generic `EcsMaster::query` body fires only at CODEGEN (`build` / `test`),
//! while `trybuild`'s `compile_fail` runs a metadata-only `cargo check` which
//! does NOT instantiate an external caller's generic-fn body. So each fixture
//! ALSO forces the CHECK-time trigger
//! `assert_query_no_change_detection::<D, F>()` from a `const _: () = ...` item
//! (eagerly const-evaluated under `cargo check`). The fixture additionally
//! calls `world.query::<…>()` to document the rejected real-API shape.
//!
//! Covered cases:
//!
//! * `query_ref_rejected.rs` — `world.query::<Ref<P>, ()>()` → `Ref` data has
//!   `NEEDS_CHANGE_DETECTION = true`.
//! * `query_mut_rejected.rs` — `world.query::<Mut<P>, ()>()` → `Mut` twin.
//! * `query_changed_filter_rejected.rs` — `world.query::<&P, Changed<P>>()` →
//!   `Changed` filter has `NEEDS_CHANGE_DETECTION = true`.
//! * `query_added_filter_rejected.rs` — `world.query::<&P, Added<P>>()` →
//!   `Added` filter twin.

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/query_change_detection_compile_fail/*.rs");
}
