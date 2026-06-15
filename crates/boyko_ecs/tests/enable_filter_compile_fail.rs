//! EnableTag Step 7 — `compile_fail` acceptance tests for the `Enabled<T>` /
//! `Disabled<T>` query filters.
//!
//! Each `.rs` file in `tests/enable_filter_compile_fail/` must fail to compile
//! with the diagnostic recorded in its matching `.stderr` file. Regenerate the
//! `.stderr` files on toolchain bumps (snapshot-based compile-fail tests are
//! toolchain-coupled) via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test enable_filter_compile_fail
//! ```
//!
//! Covered cases:
//!
//! * `or_enabled_rejected.rs` — `Or<(Enabled<A>, With<B>)>` → `Enabled<A>` is
//!   not `OrComposable` ⇒ `Or<..>: QueryFilter` is unsatisfied (M1).
//! * `or_disabled_rejected.rs` — `Or<(Disabled<A>, With<B>)>` → same (M1, the
//!   polarity twin).
//! * `for_each_chunk_enabled_rejected.rs` — `for_each_chunk` with `Enabled<A>`
//!   → `Enabled<A>` is not `ArchetypalQueryFilter` ⇒ the bound is unsatisfied.
//! * `changed_plus_enable_rejected.rs` — `Query<&P, (Changed<P>, Enabled<A>)>` →
//!   the `(D, F)`-seam `ASSERT_SHAPE` const-assert `_C3` fires (enable XOR
//!   change-detection in one query — amendment A3.4).
//! * `enable_tuple_no_positive_rejected.rs` — `Query<(), (Enabled<A>, Enabled<B>)>`
//!   → the narrowed `_C2` const-assert fires: a tuple of enable terms with no
//!   positive term is not a single leaf and is unbounded in v1 (amendment
//!   A3.2/A3.3). The SOLE single shape `Query<(), Enabled<A>>` is, by contrast,
//!   accepted (candidate-seeded).

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/enable_filter_compile_fail/*.rs");
}
