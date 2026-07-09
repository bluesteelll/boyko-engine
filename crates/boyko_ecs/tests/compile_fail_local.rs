//! Phase 13 §6.3 — `compile_fail` acceptance test for the `Local<'s, T>`
//! bound surface.
//!
//! Proves the Decision A1 + B1 bounds (`T: Send + Sync + Default + 'static`)
//! are enforced at the use site: a `Local<NoDefault>` where `NoDefault` has no
//! `Default` impl must fail to compile, so `Local` cannot be silently
//! instantiated over a type that has no initial value.
//!
//! Each `.rs` file in `tests/compile_fail_local/` is expected to fail to
//! compile with the diagnostic recorded in its matching `.stderr` file.
//! Regenerate the `.stderr` baseline when revising a case via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test compile_fail_local
//! ```
//!
//! Covered case:
//!
//! * `local_non_default_rejected.rs` — a system signature taking
//!   `Local<NoDefault>` where `struct NoDefault;` derives no `Default`. The
//!   `SystemParam` impl bound `T: Default` is unsatisfied.

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_local/*.rs");
}
