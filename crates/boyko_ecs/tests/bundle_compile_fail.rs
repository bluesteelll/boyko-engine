//! Phase 8.5 Step 9 — `compile_fail` acceptance tests for `#[derive(Bundle)]`
//! (extended in Phase 22 D7 with the `#[derive(Component)]` Bundle-emission
//! cases).
//!
//! Each `.rs` file in `tests/bundle_compile_fail/` is expected to fail to
//! compile with the diagnostic recorded in its matching `.stderr` file.
//! Regenerate the `.stderr` files when adding or revising cases — and as the
//! **standard procedure on toolchain bumps** (snapshot-based compile-fail
//! tests are inherently toolchain-coupled) — via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test bundle_compile_fail
//! ```
//!
//! Covered cases (per plan §9 Step 9 + Phase 22 D7):
//!
//! * `unit_struct.rs` — `#[derive(Bundle)] struct Marker;` →
//!   "Bundle requires at least one field" (now pointing at
//!   `Commands::spawn_empty()`).
//! * `generic_struct.rs` — `#[derive(Bundle)] struct G<T> { ... }` →
//!   "Bundle derive does not support generics".
//! * `non_component_field.rs` — `#[derive(Bundle)] struct B { x: u32 }` →
//!   `u32: Component` is not satisfied.
//! * `manual_impl_blocked.rs` — manual `impl Bundle for Foo` outside the
//!   macro → seal-trait error.
//! * `component_bundle_double_derive.rs` (Phase 22) — `Component` + `Bundle`
//!   double derive → E0119 duplicate impls (escape hatch:
//!   `#[component(no_bundle)]`).
//! * `over_max_arity.rs` (Phase 22, review M1) — a 17-field
//!   `#[derive(Bundle)]` → "Bundle supports at most 16 components
//!   (MAX_BUNDLE_ARITY)" at expansion time, before any trait check (the
//!   runtime stack collectors are sized to exactly `MAX_BUNDLE_ARITY`).
//! * `non_send_component_without_no_bundle.rs` (Phase 22) — an `Rc`-bearing
//!   `#[derive(Component)]` without `no_bundle` → **BOTH** the named
//!   const-assert E0277 AND the impl-level supertrait E0277. Their relative
//!   order is not guaranteed across rustc versions; the load-bearing anchor
//!   in the snapshot is the named symbol
//!   `_boyko_component_as_bundle_requires_send_sync_unpin`. After any
//!   `TRYBUILD=overwrite` regeneration, verify the symbol is still present
//!   in the refreshed `.stderr`.

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/bundle_compile_fail/*.rs");
}
