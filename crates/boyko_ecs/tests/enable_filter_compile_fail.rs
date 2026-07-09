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
//! * `dense_iter_mut_enable_rejected.rs` (`&mut Dense` + `Enabled`),
//!   `dense_iter_enable_rejected.rs` (`&Dense` + `Enabled`, the read-only twin),
//!   `query_dense_iter_mut_enable_rejected.rs` (`&mut Dense` + `Disabled`, the
//!   polarity twin — the `query_` filename prefix is historical; it is NOT a
//!   `Query`-vs-`QueryView` distinction, it is the `Disabled` polarity case) —
//!   Dense-enable plan D0: the archetype-agnostic dense fast path
//!   (`dense_iter` / `dense_iter_mut` on `Query` and `QueryView`) cannot honor a
//!   per-row enable term. All four methods gate on the shared
//!   `Query::assert_dense_iter_no_enable::<D, F>()` shape assert; these cases fire
//!   it in a `const ITEM` (the check-time trigger, since a `compile_fail`-only
//!   suite runs `cargo check` — the in-body `const {}` at each method top is the
//!   codegen-time trigger for real callers).
//!
//!   COVERAGE CAVEAT (reviewer P2-b): because a `compile_fail` suite only
//!   `cargo check`s, these three cases invoke the shared helper DIRECTLY in a
//!   `const ITEM` — they pin that the *helper* rejects an enable-bearing `F`, NOT
//!   that each of the four `dense_iter*` method bodies actually CALLS the helper.
//!   The real per-method-site `const {}` firing is a codegen-time guard, witnessed
//!   indirectly by the positive companion (the SAME query iterating via
//!   `iter_mut()`), the headless
//!   `state.rs::dense_enable::dense_enable_iter_mut_positive_companion`, plus the
//!   green `cargo check` proving the in-body `const {}` compiles for the
//!   non-enable case. A future edit deleting a `const {}` from one method body
//!   would not be caught here — the codegen guard is the load-bearing reject; this
//!   suite documents the intent.

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/enable_filter_compile_fail/*.rs");
}
