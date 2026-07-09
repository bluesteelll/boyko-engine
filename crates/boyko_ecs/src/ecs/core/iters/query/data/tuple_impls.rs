//! Split from `data.rs` (mechanical move; see `super` for the shared
//! `QueryData` / `ReadOnlyQueryData` traits and imports).

use super::*;

// ── Variadic tuple impls (§4.6 / §10.1, M4) ────────────────────────────────
//
// A single `macro_rules!` site emits `QueryData` impls for tuple arities
// `1..=MAX_QUERY_DATA_ARITY` (= 12). The paired-ident invocation syntax
// `((D, s, f), ...)` carries three distinct ident kinds per tuple element:
//
// * `$D` — type-ident used in trait bounds (`D0: QueryData`, etc.).
// * `$s` — value-ident bound to the per-element `State` inside `let
//   ($($s,)*) = state` destructures.
// * `$f` — value-ident bound to the per-element `Fetch<'w>` inside `let
//   ($($f,)*) = fetch` destructures.
//
// The pairing avoids `paste!` (no external dep) and the Round-1 pseudo
// `[< state_ $d >]` syntax (rejected per M4). See plan §25 for the
// concrete arity-3 expansion.
//
// `ReadOnlyQueryData` is auto-emitted alongside in a dedicated
// `impl_read_only_query_data_tuple!` macro (avoids requiring every
// `$D` simultaneously satisfy both `QueryData` and `ReadOnlyQueryData`
// at the `unsafe impl` site of the working macro — the gated
// `ReadOnlyQueryData` blanket has its own bound set).

/// Emits a `QueryData` impl for a tuple of the given paired idents (one
/// `(TypeIdent, state_value_ident, fetch_value_ident)` triple per
/// element). Invoked for arity `1..=MAX_QUERY_DATA_ARITY`.
macro_rules! impl_query_data_tuple {
    ( $( ($D:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY (QD1-QD4): the tuple impl forwards every method to its
        //   per-element delegate, which upholds QD1-QD4 by its own
        //   contract. `archetype` is the same pointer for every element in
        //   one `set_table_*` call (each element caches its own column).
        //   Intra-tuple aliasing among `$D`s is detected at `init_access`
        //   via `FilteredAccessSet`.
        #[allow(non_snake_case)]
        unsafe impl< $($D: QueryData),* > QueryData for ( $($D,)* ) {
            type State = ( $($D::State,)* );
            type Fetch<'w> = ( $($D::Fetch<'w>,)* );
            type Item<'w> = ( $($D::Item<'w>,)* );

            const IS_READ_ONLY: bool = true $( && $D::IS_READ_ONLY )*;

            // Phase 12.5 Track B NCD3: tuple propagation — any element
            // needing change detection forces the dispatcher to use the
            // meta-bearing variant for the whole tuple.
            const NEEDS_CHANGE_DETECTION: bool = false $( || $D::NEEDS_CHANGE_DETECTION )*;

            // EnableTag C2: a tuple touches a data component iff ANY element
            // does (OR-fold) — bounds an enable term's matched set.
            const HAS_DATA_COMPONENT: bool = false $( || $D::HAS_DATA_COMPONENT )*;

            // Task #9 M1: a tuple requires post-filter trim iff ANY element
            // does (OR-fold). Without this, a tuple wrapping `AnyOf<..>` falls
            // back to the trait default `false`, which re-seeds the candidate
            // path and skips `post_filter_matched` — AnyOf's >=1-member OR-trim
            // never runs, yielding phantom `(None,)` rows.
            const REQUIRES_POST_FILTER_TRIM: bool = false $( || $D::REQUIRES_POST_FILTER_TRIM )*;

            // Dense plan D3: a tuple touches dense storage iff ANY element does
            // (OR-fold). Drives the cursor's gated `resolve_dense` /
            // `dense_row_passes` forwarders; `false` for an all-table tuple
            // (the 0%-gate — the forwarders below const-fold to no-ops).
            const HAS_DENSE: bool = false $( || $D::HAS_DENSE )*;
            // Dense plan D3: a tuple has a dense INCLUDE term iff ANY element
            // does (OR-fold) — drives the candidate seed.
            const HAS_DENSE_INCLUDE: bool = false $( || $D::HAS_DENSE_INCLUDE )*;
            // Relation-DSL join: a tuple has a relation term iff ANY element
            // does (OR-fold) — drives the cursor's gated `resolve_related`
            // forwarder + the `par_iter` const-rejection. `false` for an
            // all-non-relation tuple (the 0%-gate).
            const HAS_RELATED: bool = false $( || $D::HAS_RELATED )*;

            #[inline]
            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$D as QueryData>::init_state(world), )* )
            }

            #[inline]
            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)* ) = state;
                $( <$D as QueryData>::init_access($s, access_set); )*
            }

            #[inline]
            fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
                let ( $($s,)* ) = state;
                true $( && <$D as QueryData>::matches_component_set($s, mask) )*
            }

            #[inline]
            fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
                let ( $($s,)* ) = state;
                $( <$D as QueryData>::aggregate_include($s, include); )*
            }

            #[inline]
            fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
                let ( $($s,)* ) = state;
                ( $( <$D as QueryData>::init_fetch($s), )* )
            }

            #[inline]
            unsafe fn set_table_readonly<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): forwarded per-element; `archetype`
                    //   carries read-only provenance and is identical for
                    //   every element. The caller of the tuple impl
                    //   upheld QD3/QD4 for every `$D`. `meta` is forwarded
                    //   by reference per Round 2 W7.
                    unsafe { <$D as QueryData>::set_table_readonly($f, $s, archetype, meta); }
                )*
            }

            #[inline]
            unsafe fn set_table_mut<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
                meta: &'_ SystemMeta,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): write-capable `archetype` is
                    //   forwarded to every element; the caller upheld
                    //   QD3/QD4. `meta` forwarded by reference.
                    unsafe { <$D as QueryData>::set_table_mut($f, $s, archetype, meta); }
                )*
            }

            #[inline]
            unsafe fn set_table_readonly_no_meta<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): forwarded per-element; the tuple's
                    //   NCD3 propagation guarantees this method is only
                    //   reached when no element needs change detection,
                    //   so every element's `_no_meta` body is the
                    //   meta-free re-impl (not the cold panic backstop).
                    unsafe { <$D as QueryData>::set_table_readonly_no_meta($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn set_table_mut_no_meta<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (QD3, QD4): write-capable `archetype` forwarded;
                    //   same NCD3-propagation note as the readonly variant.
                    unsafe { <$D as QueryData>::set_table_mut_no_meta($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn resolve_dense<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (D3): each element gates its own body on
                    //   `const { $D::HAS_DENSE }` (a non-dense element's
                    //   `resolve_dense` is the empty default — folds out). The
                    //   `world` cell is `Copy`, forwarded by value to preserve
                    //   provenance.
                    unsafe { <$D as QueryData>::resolve_dense($f, $s, world); }
                )*
            }

            #[inline]
            unsafe fn resolve_related<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (relation join): each element gates its own body on
                    //   `const { $D::HAS_RELATED }` (a non-relation element's
                    //   `resolve_related` is the empty default — folds out). The
                    //   `world` cell is `Copy`, forwarded by value to preserve
                    //   provenance.
                    unsafe { <$D as QueryData>::resolve_related($f, $s, world); }
                )*
            }

            #[inline]
            unsafe fn dense_row_passes<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
                let ( $($f,)* ) = fetch;
                // AND over elements: every REQUIRED dense term must have the
                // row's entity in its store. A non-dense element's
                // `dense_row_passes` is the const-`true` default (folds out).
                // SAFETY (D3): per-element contract — `fetch`/`row` were set up
                //   by `resolve_dense` + `set_table_*`; `row < entity_count`.
                true $( && unsafe { <$D as QueryData>::dense_row_passes($f, row) } )*
            }

            #[inline]
            fn dense_include_candidates(
                state: &Self::State,
                registry: &crate::ecs::core::component::dense::DenseRegistry,
                out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
            ) {
                let ( $($s,)* ) = state;
                // Each element ORs its own dense-include candidates (gated by its
                // own `HAS_DENSE_INCLUDE`). The UNION over a tuple of dense terms
                // is conservative (false positives trimmed per-row); the exact
                // AND-membership is the per-row `dense_row_passes`.
                $( <$D as QueryData>::dense_include_candidates($s, registry, out); )*
            }

            #[inline]
            unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
                let ( $($f,)* ) = fetch;
                (
                    $(
                        // SAFETY (QD2, QD3): per-element fetch contract
                        //   held by the caller; `row` is in range for the
                        //   archetype previously cached by `set_table_*`.
                        unsafe { <$D as QueryData>::fetch($f, row) },
                    )*
                )
            }
        }
    };
}

/// Emits a `ReadOnlyQueryData` impl for the tuple of the given type-idents.
/// Gated separately from [`impl_query_data_tuple!`] so the bound set is
/// `$D: ReadOnlyQueryData` (which transitively implies `$D: QueryData`)
/// without conflating the two trait bounds in a single `impl<>` header.
macro_rules! impl_read_only_query_data_tuple {
    ( $( $D:ident ),* ) => {
        // SAFETY: every `$D` is `ReadOnlyQueryData` (each `$D::IS_READ_ONLY
        //   = true` and the impl is gated to perform no writes). The tuple
        //   impl forwards every fetch to per-element fetch, which is
        //   read-only by induction.
        unsafe impl< $($D: ReadOnlyQueryData),* > ReadOnlyQueryData for ( $($D,)* ) {}
    };
}

// Empty-tuple base case: `Query<(), F>` yields `()` per row. Useful for
// entity-only / filter-only queries (e.g. `Query<(), With<Player>>`).
// SAFETY (QD1-QD4): vacuous — no state, no access surface, no fetched
//   columns, no per-row dereferences. All four invariants hold trivially.
unsafe impl QueryData for () {
    type State = ();
    type Fetch<'w> = ();
    type Item<'w> = ();
    const IS_READ_ONLY: bool = true;
    // Phase 12.5 Track B NCD2: vacuous — `()` touches no components.
    const NEEDS_CHANGE_DETECTION: bool = false;
    // EnableTag C2: `()` touches no data component (no positive include bit).
    const HAS_DATA_COMPONENT: bool = false;

    #[inline] fn init_state(_world: &mut EcsMaster) -> Self::State {}
    #[inline] fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {}
    #[inline] fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool { true }
    #[inline] fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {}
    #[inline] fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {}
    #[inline] unsafe fn set_table_readonly<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *const Archetype, _meta: &'_ SystemMeta) {}
    #[inline] unsafe fn set_table_mut<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *mut Archetype, _meta: &'_ SystemMeta) {}
    #[inline] unsafe fn set_table_readonly_no_meta<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *const Archetype) {}
    #[inline] unsafe fn set_table_mut_no_meta<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *mut Archetype) {}
    #[inline] unsafe fn fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> Self::Item<'w> {}
}

// SAFETY: () has IS_READ_ONLY = true.
unsafe impl ReadOnlyQueryData for () {}

impl_query_data_tuple!((D0, s0, f0));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3));
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10)
);
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11)
);

impl_read_only_query_data_tuple!(D0);
impl_read_only_query_data_tuple!(D0, D1);
impl_read_only_query_data_tuple!(D0, D1, D2);
impl_read_only_query_data_tuple!(D0, D1, D2, D3);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7, D8);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9, D10);
impl_read_only_query_data_tuple!(D0, D1, D2, D3, D4, D5, D6, D7, D8, D9, D10, D11);

// ── Arity-overflow stubs (arity 13..=24) — M7 + C-NEW-2 ────────────────────
//
// Same pattern as Phase 8a's `params/tuple_impl.rs::
// impl_system_param_tuple_too_large!`: each method body is `const {
// panic!(...) }`, which evaluates ONLY at monomorphization. Crates that
// never instantiate a 13+ arity `QueryData` tuple compile cleanly.
//
// `compile_error!` was rejected in Phase 8a (C-NEW-2): it fires at
// macro-expansion time, breaking the wider crate. `panic!()`
// requires `rustc >= 1.79`; boyko targets Rust 2024 (`>= 1.85`).

/// Emits a stub `QueryData` impl whose every method body is
/// `panic!(...)`. The const block fires at monomorphization;
/// the impl is never *successfully* used at runtime. `State`, `Fetch<'w>`,
/// and `Item<'w>` collapse to `()` so the stub type-checks in isolation.
macro_rules! impl_query_data_tuple_too_large {
    ( $( ($D:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY: stub impl whose every method body is `panic!(...)`.
        //   The impl is never *successfully* used at runtime — the const
        //   block fails at monomorphization with the diagnostic in
        //   `init_state`. QD1-QD4 are vacuously upheld because no code
        //   path that respects the contract ever observes the impl's
        //   effects.
        #[allow(non_snake_case, unused_variables)]
        unsafe impl< $($D: QueryData),* > QueryData for ( $($D,)* ) {
            type State = ();
            type Fetch<'w> = ();
            type Item<'w> = ();
            const IS_READ_ONLY: bool = true;
            // Vacuous — every method is a `panic!()` at monomorphisation,
            // so the const is unobservable on any reachable path.
            const NEEDS_CHANGE_DETECTION: bool = false;
            const HAS_DATA_COMPONENT: bool = false;

            fn init_state(_world: &mut EcsMaster) -> Self::State {
                panic!(
                        "tuple has too many QueryData elements. \
                         boyko-engine supports up to \
                         MAX_QUERY_DATA_ARITY = 12. Split your query into \
                         smaller queries or wrap related elements in a \
                         struct that implements QueryData."
                    )
            }

            fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
                panic!("tuple too large: see init_state diagnostic")
            }

            fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool {
                panic!("tuple too large: see init_state diagnostic")
            }

            fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {
                panic!("tuple too large: see init_state diagnostic")
            }

            fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_readonly<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *const Archetype,
                _meta: &'_ SystemMeta,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_mut<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *mut Archetype,
                _meta: &'_ SystemMeta,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_readonly_no_meta<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *const Archetype,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn set_table_mut_no_meta<'w>(
                _fetch: &mut Self::Fetch<'w>,
                _state: &Self::State,
                _archetype: *mut Archetype,
            ) {
                panic!("tuple too large: see init_state diagnostic")
            }

            unsafe fn fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> Self::Item<'w> {
                panic!("tuple too large: see init_state diagnostic")
            }
        }
    };
}

impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19),
    (D20, s20, f20)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19),
    (D20, s20, f20), (D21, s21, f21)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19),
    (D20, s20, f20), (D21, s21, f21), (D22, s22, f22)
);
impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11),
    (D12, s12, f12), (D13, s13, f13), (D14, s14, f14), (D15, s15, f15),
    (D16, s16, f16), (D17, s17, f17), (D18, s18, f18), (D19, s19, f19),
    (D20, s20, f20), (D21, s21, f21), (D22, s22, f22), (D23, s23, f23)
);

