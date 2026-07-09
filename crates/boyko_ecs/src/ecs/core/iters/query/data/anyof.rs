//! Split from `data.rs` (mechanical move; see `super` for the shared
//! `QueryData` / `ReadOnlyQueryData` traits and imports).

use super::*;

// ── AnyOf<(D0, D1, …)> (task #9 — OR over real-component leaves) ────────────
//
// `AnyOf<(D0, …, Dn)>` yields a tuple `(Option<D0::Item>, …)` where at least
// one element is `Some`. It is the OR analogue of a data tuple's AND.
//
// Cost note (Decision 8): a SOLE `Query<AnyOf<(&A, &B)>>` has an EMPTY include
// mask ⇒ `update_archetypes` matches EVERY live archetype, then
// `post_filter_matched` trims to those containing (A or B). This is the
// `Or<F>` cost profile — paid per generation bump (per `update`), NOT per
// `iter()`. A `Query<(&A, AnyOf<(&B, &C)>)>` is bounded by `&A`'s include and
// pays no full-world scan. The full-world-scan cost scales with archetype
// count; do not mistake it for a bug (filed as an archetype-count-scaling
// bench note).

/// Sealed marker for the leaf types admissible as an [`AnyOf`] arm.
///
/// `AnyOf<(D0, …)>` bounds every arm `Di: AnyOfArm`. The seal compile-rejects
/// arms whose `matches_component_set` is not a single-component predicate —
/// `Option<_>` (unconditionally `true`), `()` (unconditionally `true`), nested
/// `AnyOf`, and tuple arms — every one of which would break the OR's ≥1-member
/// trim by matching the whole world (Decision 3). Mirrors the sealed
/// `OrComposable` bound (`filter.rs`).
///
/// # Members
///
/// `&T`, `&mut T`, [`Ref<'_, T>`](Ref), [`Mut<'_, T>`](Mut) for any
/// `T: Component`. NOT members: `Option<_>`, `()`, `AnyOf<_>`, tuples.
///
/// # Safety
///
/// A purely declarative marker — no method contract. `unsafe` signals that
/// membership is a deliberate, audited choice: an `AnyOfArm` must have a
/// single-component `matches_component_set` so the OR-trim is well-defined.
pub unsafe trait AnyOfArm: QueryData {}

// SAFETY: `&T::matches_component_set` is `mask.contains(id)` — a single
//   real-component predicate; the OR-trim over arms is well-defined.
unsafe impl<T: Component> AnyOfArm for &T {}

// SAFETY: `&mut T::matches_component_set` is `mask.contains(id)` — same.
unsafe impl<T: Component> AnyOfArm for &mut T {}

// SAFETY: `Ref<T>::matches_component_set` is `mask.contains(id)` — same.
unsafe impl<T: Component> AnyOfArm for Ref<'_, T> {}

// SAFETY: `Mut<T>::matches_component_set` is `mask.contains(id)` — same.
unsafe impl<T: Component> AnyOfArm for Mut<'_, T> {}

/// OR-combinator query data: yields `(Option<D0::Item>, …)` with the ≥1-member
/// guarantee (at least one arm is `Some` for every yielded row).
///
/// Every arm must be an [`AnyOfArm`] (a real-component leaf: `&T`, `&mut T`,
/// `Ref<T>`, `Mut<T>`). `Option`, `()`, nested `AnyOf`, and tuple arms are
/// compile-rejected (Decision 3). An empty `AnyOf<()>` has no impl ⇒
/// trait-not-satisfied compile error (Decision 7).
///
/// # Semantics
///
/// * `AnyOf<(&A, &B)>` → `(Option<&A>, Option<&B>)`, matched against
///   archetypes containing A OR B; at least one is `Some` per row.
/// * `AnyOf<(&A,)>` single arm → `(Option<&A>,)`, bounded to A-present
///   archetypes (always `Some`) — NOT equivalent to `&A` (the item is a
///   1-tuple of `Option`).
/// * `AnyOf<(&A, &A)>` overlapping read+read → legal.
/// * `AnyOf<(&mut A, &A)>` / `(&mut A, &mut A)` → trips the B0002 aliasing
///   detector (`init_access` forwards each arm).
///
/// # Cost
///
/// A SOLE `Query<AnyOf<…>>` scans the full archetype set on every generation
/// bump (empty include ⇒ the `Or<F>` cost profile) — see the module note
/// above. Bound it with a positive term (`Query<(&A, AnyOf<…>)>`) when
/// possible.
pub struct AnyOf<T>(PhantomData<fn() -> T>);

/// Emits a `QueryData` impl for `AnyOf<(D0, …)>` over the given paired idents.
/// Each arm is bounded `$D: AnyOfArm` (the seal — Decision 3). Mirrors
/// [`impl_query_data_tuple!`]'s `(TypeIdent, state_ident, fetch_ident)`
/// triples; the `bool` flag rides alongside each arm's `Fetch` as
/// `($D::Fetch<'w>, bool)`.
macro_rules! impl_any_of {
    ( $( ($D:ident, $s:ident, $f:ident) ),+ ) => {
        // SAFETY (QD1-QD4): each arm forwards to its own `QueryData` impl
        //   (QD1-QD4 by induction). Per-arm `set_table` is GATED on that
        //   arm's own `matches` (Decision 1) — never an unconditional
        //   forward. `archetype` is identical for every arm in one call.
        //   Intra-`AnyOf` aliasing among arms is detected at `init_access`
        //   via `FilteredAccessSet` (Decision 8).
        #[allow(non_snake_case)]
        unsafe impl< $($D: AnyOfArm),+ > QueryData for AnyOf<( $($D,)+ )> {
            type State = ( $($D::State,)+ );
            type Fetch<'w> = ( $(($D::Fetch<'w>, bool),)+ );
            type Item<'w> = ( $(Option<$D::Item<'w>>,)+ );

            const IS_READ_ONLY: bool = true $( && $D::IS_READ_ONLY )+;
            const NEEDS_CHANGE_DETECTION: bool = false $( || $D::NEEDS_CHANGE_DETECTION )+;
            // Non-filtering at the archetype level (the OR-trim lives in
            // `post_filter_matched`, not in a positive include bit).
            const HAS_DATA_COMPONENT: bool = false;
            // The ≥1-member OR-trim runs in the post-filter pass — so
            // `Query<AnyOf<…>, Enabled<C>>` must NOT be candidate-seeded
            // (Decision 4).
            const REQUIRES_POST_FILTER_TRIM: bool = true;

            // Dense plan D3: an `AnyOf` arm may be a dense leaf; OR-fold so the
            // cursor resolves each dense arm's `DenseStore` pointer (avoids a
            // NULL-store deref in the arm's `fetch`). `dense_row_passes` is NOT
            // forwarded (AnyOf's ≥1-member semantics keep the default `true` —
            // a row missing one OR-arm still yields `(…, None, …)`, never a
            // skip). W1 (None-on-absence): a present dense arm yields its value;
            // an absent dense arm yields `None` (per-row membership via the
            // arm's `dense_row_passes`, checked inside `fetch`).
            const HAS_DENSE: bool = false $( || $D::HAS_DENSE )+;
            // Relation-DSL join: an `AnyOf` arm may be a relation leaf; OR-fold
            // so the cursor resolves each relation arm's world cell.
            const HAS_RELATED: bool = false $( || $D::HAS_RELATED )+;

            #[inline]
            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$D as QueryData>::init_state(world), )+ )
            }

            #[inline]
            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)+ ) = state;
                // Decision 8: forward each arm — declares the full read/write
                // surface so `AnyOf<(&mut A, &A)>` trips B0002.
                $( <$D as QueryData>::init_access($s, access_set); )+
            }

            #[inline]
            unsafe fn resolve_dense<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (D3): each arm gates its body on its own
                    //   `const { $D::HAS_DENSE }`; the `world` cell is `Copy`,
                    //   forwarded by value to preserve provenance. `$f.0` is the
                    //   arm's inner `Fetch`.
                    unsafe { <$D as QueryData>::resolve_dense(&mut $f.0, $s, world); }
                )+
            }

            #[inline]
            unsafe fn resolve_related<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (relation join): each arm gates its body on its own
                    //   `const { $D::HAS_RELATED }`; the `world` cell is `Copy`,
                    //   forwarded by value to preserve provenance. `$f.0` is the
                    //   arm's inner `Fetch`.
                    unsafe { <$D as QueryData>::resolve_related(&mut $f.0, $s, world); }
                )+
            }

            #[inline]
            fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
                let ( $($s,)+ ) = state;
                // OR of arms (the ≥1-member predicate).
                false $( || <$D as QueryData>::matches_component_set($s, mask) )+
            }

            #[inline]
            fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {
                // No-op: AnyOf's OR predicate has NO common required bit; the
                // membership trim is applied in `post_filter_matched` via
                // `matches_component_set`. Populating `include` would WRONGLY
                // require ALL arms present (an AND, not an OR).
            }

            #[inline]
            fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
                let ( $($s,)+ ) = state;
                ( $( (<$D as QueryData>::init_fetch($s), false), )+ )
            }

            #[inline]
            unsafe fn set_table_readonly<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (Decision 1, QD3, QD4): per-arm gate. The mask
                    //   read uses a shared reborrow; `matches` re-derives the
                    //   arm's predicate. When `true`, the forwarded
                    //   `set_table_readonly`'s QD1/QD3 hold (arm's columns
                    //   non-null); the arm's QD4 readonly backstop on a
                    //   write-arm is preserved (unreachable in well-typed
                    //   `iter()` code). When `false`, the arm's inner stays
                    //   NULL-init and is never read.
                    {
                        let m = <$D as QueryData>::matches_component_set(
                            $s,
                            unsafe { (*archetype).component_mask() },
                        );
                        if m {
                            unsafe {
                                <$D as QueryData>::set_table_readonly(
                                    &mut $f.0, $s, archetype, meta,
                                );
                            }
                        }
                        $f.1 = m;
                    }
                )+
            }

            #[inline]
            unsafe fn set_table_mut<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (Decision 1, QD1, QD3, QD4): per-arm gate with
                    //   write-capable `archetype`. When `true`, the forwarded
                    //   `set_table_mut` consumes that arm's write provenance.
                    {
                        let m = <$D as QueryData>::matches_component_set(
                            $s,
                            unsafe { (*archetype).component_mask() },
                        );
                        if m {
                            unsafe {
                                <$D as QueryData>::set_table_mut(
                                    &mut $f.0, $s, archetype, meta,
                                );
                            }
                        }
                        $f.1 = m;
                    }
                )+
            }

            #[inline]
            unsafe fn set_table_readonly_no_meta<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (Decision 1/2, QD3, QD4): per-arm gate, meta-free.
                    //   Reached only when no arm needs change detection
                    //   (`AnyOf::NCD == false` ⇒ every arm's NCD == false ⇒
                    //   every arm's `_no_meta` is its real meta-free body, not
                    //   the cold panic).
                    {
                        let m = <$D as QueryData>::matches_component_set(
                            $s,
                            unsafe { (*archetype).component_mask() },
                        );
                        if m {
                            unsafe {
                                <$D as QueryData>::set_table_readonly_no_meta(
                                    &mut $f.0, $s, archetype,
                                );
                            }
                        }
                        $f.1 = m;
                    }
                )+
            }

            #[inline]
            unsafe fn set_table_mut_no_meta<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)+ ) = fetch;
                let ( $($s,)+ ) = state;
                $(
                    // SAFETY (Decision 1/2, QD1, QD3, QD4): per-arm gate,
                    //   write-capable, meta-free. Same NCD-propagation note as
                    //   the readonly variant.
                    {
                        let m = <$D as QueryData>::matches_component_set(
                            $s,
                            unsafe { (*archetype).component_mask() },
                        );
                        if m {
                            unsafe {
                                <$D as QueryData>::set_table_mut_no_meta(
                                    &mut $f.0, $s, archetype,
                                );
                            }
                        }
                        $f.1 = m;
                    }
                )+
            }

            #[inline]
            unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
                let ( $($f,)+ ) = fetch;
                (
                    $(
                        // SAFETY (Decision 1, QD2, QD3): when the arm's flag is
                        //   set, its inner was initialised by the gated
                        //   `set_table_*` forward and `row` is in range; when
                        //   clear, the inner is NULL-init and not read.
                        // Dense plan D3 (W1 — None-on-absence): a dense arm's
                        //   `$f.1` is archetype-level (always `true` — dense is
                        //   signature-excluded), so the REAL per-row membership
                        //   is the arm's `dense_row_passes` (≡ `slot_of(...)`).
                        //   An entity lacking the dense member yields `None` for
                        //   that arm; `AnyOf` still matches the row iff any arm
                        //   is `Some` (the post-filter trim already admitted the
                        //   row). For a table arm this const-folds OUT (0%-gate).
                        if $f.1 {
                            if const { <$D as QueryData>::HAS_DENSE } {
                                // SAFETY (D3): the arm's `set_table_*` forward
                                //   cached `entity_ids` and `resolve_dense` the
                                //   `DenseStore` pointer (both run for `$f.1 ==
                                //   true` before any `fetch`); `row` in range.
                                //   An absent slot ⟹ `dense_row_passes` is
                                //   `false` ⟹ `$D::fetch` is never called.
                                if unsafe { <$D as QueryData>::dense_row_passes(&$f.0, row) } {
                                    Some(unsafe { <$D as QueryData>::fetch(&$f.0, row) })
                                } else {
                                    None
                                }
                            } else {
                                Some(unsafe { <$D as QueryData>::fetch(&$f.0, row) })
                            }
                        } else {
                            None
                        },
                    )+
                )
            }
        }
    };
}

/// Emits a `ReadOnlyQueryData` impl for `AnyOf<(D0, …)>` — read-only iff every
/// arm is. Gated separately from [`impl_any_of!`] so the bound is
/// `$D: AnyOfArm + ReadOnlyQueryData` without conflating the two at the
/// working impl's header.
macro_rules! impl_any_of_read_only {
    ( $( $D:ident ),+ ) => {
        // SAFETY: every arm is `ReadOnlyQueryData` (each arm's
        //   `IS_READ_ONLY = true`); `AnyOf`'s per-arm fetch forwards to
        //   read-only arm fetches by induction.
        unsafe impl< $($D: AnyOfArm + ReadOnlyQueryData),+ > ReadOnlyQueryData
            for AnyOf<( $($D,)+ )> {}
    };
}

impl_any_of!((D0, s0, f0));
impl_any_of!((D0, s0, f0), (D1, s1, f1));
impl_any_of!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2));
impl_any_of!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3));
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10)
);
impl_any_of!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11)
);

impl_any_of_read_only!(D0);
impl_any_of_read_only!(D0, D1);
impl_any_of_read_only!(D0, D1, D2);
impl_any_of_read_only!(D0, D1, D2, D3);
impl_any_of_read_only!(D0, D1, D2, D3, D4);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7, D8);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9, D10);
impl_any_of_read_only!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9, D10, D11);

