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
//! * `added_on_bitset_tag_rejected.rs` — `Added<C>` on a `STORAGE_IS_BITSET`
//!   component → the D4 storage-shape const-assert
//!   `Added::assert_storage_supports_change_detection` fires (a bitset enable
//!   tag has no per-row tick storage). HAND-IMPL `Component` fixture.
//! * `changed_on_bitset_tag_rejected.rs` — `Changed<C>` on a `STORAGE_IS_BITSET`
//!   component → the D4 const-assert `Changed::assert_storage_supports_change_detection`
//!   fires (the polarity twin). HAND-IMPL `Component` fixture.
//! * `added_on_derived_bitset_tag_rejected.rs` /
//!   `changed_on_derived_bitset_tag_rejected.rs` — the D4 reject reached through
//!   the REAL `#[component(storage = "bitset")]` derive (Wave 5 Step 10), not a
//!   hand-impl: the derived `STORAGE_IS_BITSET = true` const fires the assert.
//! * `storage_unknown_value_rejected.rs` — `#[component(storage = "typo")]` → the
//!   derive's `LitStr` storage arm rejects any value other than `"bitset"`,
//!   naming the allowed value (W1-r6 / Step 10 (1)).
//! * `storage_bitset_with_fields_rejected.rs` —
//!   `#[component(storage = "bitset")] struct Bad(u32);` → a fielded bitset tag is
//!   rejected (no `ComponentPool`, so field data has nowhere to live; Step 10 (3)).
//! * `storage_bitset_not_a_bundle_rejected.rs` — a derived bitset tag is NOT a
//!   `Bundle` (single-component `Bundle` emission suppressed; D6 / Step 10 (2c)).

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/enable_filter_compile_fail/*.rs");
}
