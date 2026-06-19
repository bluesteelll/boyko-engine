//! Phase X.A — sequential `Query::for_each_chunk` driver.
//!
//! Shared by [`Query::for_each_chunk`] and (when Wave 5 lands)
//! [`QueryView::for_each_chunk`]. Mirrors the outer-loop shape of
//! [`super::iter`] but yields one [`ChunkedQueryData::ChunkItem`] per
//! matched archetype instead of one [`QueryData::Item`] per row.
//!
//! # Hot loop shape
//!
//! For each `arch_id` in the caller-supplied `ids` slice (Phase 22.1 Area A:
//! resolved once at the driver entry — `matched_ids_pre_terms()` with no
//! terms, or the per-epoch memoised term-filtered slice):
//!
//! 1. Mint a `*mut Archetype` (`mutable == true`) or reborrow `*const`
//!    as `*mut` (`mutable == false`) via `UnsafeEcsCell::archetype_ptr_mut`
//!    / `UnsafeEcsCell::archetype_ptr`. Stale ids (Q5) are skipped via
//!    `continue` — same semantics as `iter.rs:216-220`.
//! 2. Probe `entity_count`; skip archetypes whose row count is zero.
//! 3. Refresh the per-archetype `ChunkFetch` via
//!    [`ChunkedQueryData::set_chunk_mut`] or
//!    [`ChunkedQueryData::set_chunk_readonly`].
//! 4. Materialise the single full-archetype chunk via
//!    [`ChunkedQueryData::fetch_chunk(0, entity_count)`][fetch] and pass
//!    it to the user closure.
//!
//! No per-row inner loop exists; the user closure receives one slice (or
//! tuple of slices) per archetype.
//!
//! # NCD const-fold elision
//!
//! [`ChunkedQueryData`] excludes `Ref<T>` / `Mut<T>` (they need per-row
//! tick state); [`ArchetypalQueryFilter`] excludes `Added<C>` /
//! `Changed<C>`. Therefore `NEEDS_CHANGE_DETECTION` is always `false` at
//! this monomorphisation — no meta plumbing, no `_no_meta` dispatcher
//! split, no per-row tick check.
//!
//! [`Query::for_each_chunk`]: super::query::Query::for_each_chunk
//! [`QueryView::for_each_chunk`]: super::query_view::QueryView
//! [`ChunkedQueryData`]: super::chunked_data::ChunkedQueryData
//! [`ChunkedQueryData::set_chunk_mut`]: super::chunked_data::ChunkedQueryData::set_chunk_mut
//! [`ChunkedQueryData::set_chunk_readonly`]: super::chunked_data::ChunkedQueryData::set_chunk_readonly
//! [`ArchetypalQueryFilter`]: super::filter::ArchetypalQueryFilter
//! [`QueryData::Item`]: super::data::QueryData::Item
//! [fetch]: super::chunked_data::ChunkedQueryData::fetch_chunk

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::iters::query::chunked_data::ChunkedQueryData;
use crate::ecs::core::iters::query::filter::ArchetypalQueryFilter;
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::ArchetypeId;

/// Sequential chunked-iter driver. Shared between
/// [`Query::for_each_chunk`] and [`QueryView::for_each_chunk`].
///
/// # Hot loop
///
/// Outer loop iterates the caller-supplied `ids` slice. Per archetype: skip
/// stale ids (Q5), skip empty archetypes, refresh `ChunkFetch`, hand the full
/// slice to `f`.
///
/// # Inline policy
///
/// Deliberately **not** marked `#[inline]` — LLVM inlines this driver
/// into the (single-call-site) caller `Query::for_each_chunk` regardless
/// of the missing hint; an explicit `#[inline]` would force inlining in
/// every monomorphisation including cold-startup ones, bloating I-cache.
/// Mirrors the analogous decision on `par_iter::for_each_impl`
/// (`par_iter.rs:244`).
///
/// # Safety
///
/// The caller MUST satisfy two contracts:
///
/// * **Read/write contract of `D`** — when `mutable == false`, `world`
///   must carry read-only mint provenance (debug-asserted by the cell).
///   When `mutable == true`, `world` must carry write-capable provenance
///   (from [`UnsafeEcsCell::new_mutable`]). The conflict-graph upstream
///   guarantees no concurrent worker aliases any column touched by `D`
///   on the current dispatch round.
/// * **State-sync** — `state` must already be synced against the live
///   archetype set via [`QueryDataState::update`]. The driver does not
///   call `update` itself; it walks the caller-supplied `ids` slice
///   verbatim. Stale ids (archetypes removed after the last sync) are
///   skipped transparently via the `archetype_ptr(_mut)` `None` arm.
///
/// Phase 22.1 Area A: `ids` is resolved ONCE at the driver entry
/// (`Query::for_each_chunk` / `QueryView::for_each_chunk`) — no terms →
/// `matched_ids_pre_terms()`; terms → the per-epoch memoised term-filtered
/// slice. This driver carries no term code; the per-transition term test of
/// Phase 22 is gone.
///
/// [`Query::for_each_chunk`]: super::query::Query::for_each_chunk
/// [`QueryView::for_each_chunk`]: super::query_view::QueryView
/// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
pub(crate) unsafe fn for_each_chunk_impl<'q, 's, D, F, Func>(
    state: &'s QueryDataState<D, F>,
    ids: &[ArchetypeId],
    world: UnsafeEcsCell<'q>,
    mutable: bool,
    mut f: Func,
) where
    D: ChunkedQueryData,
    F: ArchetypalQueryFilter,
    Func: for<'c> FnMut(D::ChunkItem<'c>),
{
    // Dense plan D3: the chunk path takes whole archetype-aligned slices, which
    // a dense (signature-excluded, single-global-column) term cannot supply.
    // Compile-reject a dense `D`/`F` here — use `Query::iter` (mixed) or
    // `Query::dense_iter` (pure dense). Const-folds to nothing for a no-dense
    // query (the 0%-gate).
    const {
        assert!(
            !D::HAS_DENSE && !F::HAS_DENSE,
            "a dense (storage = \"dense\") term is not supported on `for_each_chunk` — \
             use `Query::iter` / `iter_mut` (mixed) or `Query::dense_iter` (pure dense)"
        )
    };
    let mut chunk_fetch = <D as ChunkedQueryData>::init_chunk_fetch(&state.data_state);

    for &arch_id in ids {
        // SAFETY (U_C2 / U_C3, Q5): mirrors the read-only / write-capable
        //   mint split from `iter.rs:217-220` (`QueryIter`) and
        //   `iter.rs:419-422` (`QueryIterMut`). The cell is scoped to
        //   `'q` per the caller contract; `archetype_ptr(_mut)` returns
        //   `None` for archetype ids whose slot was removed after `state`
        //   was last synced — those entries are transparently skipped via
        //   `continue`, the Q5 stale-id-skip path. When `mutable == false`
        //   we take the read-only mint and cast `*const → *mut` purely so
        //   `arch_ptr` has a single type; only the read-only set_chunk
        //   path is called below, so the cast is never dereferenced as
        //   write-capable (read-only-mint provenance preserved).
        let arch_ptr: *mut Archetype = unsafe {
            if mutable {
                match world.archetype_ptr_mut(arch_id) {
                    Some(p) => p,
                    None => continue,
                }
            } else {
                match world.archetype_ptr(arch_id) {
                    Some(p) => p as *mut Archetype,
                    None => continue,
                }
            }
        };

        // SAFETY (U1 / U2 — Phase 7 slab stability): `arch_ptr` is live
        //   for the surrounding cell scope (`'q`); the `&Archetype`
        //   reborrow materialised by the raw deref is bounded to this
        //   single expression. No `&mut Archetype` is produced.
        let entity_count = unsafe { (*arch_ptr).entity_count() };
        if entity_count == 0 {
            continue;
        }

        // SAFETY (CD1, CD4): write-capable / read-only dispatch chosen
        //   by `mutable`. For `mutable == true`, `arch_ptr` carries
        //   write-capable provenance (from `archetype_ptr_mut`); CD4
        //   forbids `set_chunk_readonly` on `D` containing `&mut T` —
        //   the `&mut T: ChunkedQueryData` impl panics if reached.
        //   For `mutable == false`, `arch_ptr` was minted via
        //   `archetype_ptr` (read-only); only `set_chunk_readonly` is
        //   called, so the `*const → *mut` cast above is not actually
        //   exercised as a write-capable pointer.
        //   No `meta` is forwarded — `ChunkedQueryData` excludes
        //   `Ref`/`Mut` and `ArchetypalQueryFilter` excludes
        //   `Added`/`Changed`, so `NEEDS_CHANGE_DETECTION = false` at
        //   this monomorphisation. The chunk path therefore does not
        //   need the meta-bearing dispatcher split present in `iter.rs`.
        unsafe {
            if mutable {
                <D as ChunkedQueryData>::set_chunk_mut(
                    &mut chunk_fetch,
                    &state.data_state,
                    arch_ptr,
                );
            } else {
                <D as ChunkedQueryData>::set_chunk_readonly(
                    &mut chunk_fetch,
                    &state.data_state,
                    arch_ptr as *const _,
                );
            }
        }

        // SAFETY (CD2): `set_chunk_*` above initialised `chunk_fetch`
        //   for the current archetype; `start = 0`, `len = entity_count`
        //   covers exactly the live row range `[0, entity_count)`. CD3
        //   disjointness is vacuous on the sequential path — exactly one
        //   `fetch_chunk` invocation per archetype.
        let item = unsafe {
            <D as ChunkedQueryData>::fetch_chunk(&chunk_fetch, 0, entity_count)
        };
        f(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::core::system::system_meta::SystemMeta;
    use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

    // Component ids reserved for the Phase X.A Wave 4 chunk_iter tests. The
    // free range below was verified at write time against the existing
    // crate-wide allocations:
    //   * 400-422 — archetype.rs / archetype_bundle.rs / component_pool_bundle.rs
    //   * 480-482 — archetype_bundle miri tests
    //   * 483-485 — query/iter.rs
    //   * 486-488 — query/query.rs
    //   * 490-497 — query_state / component_set
    //   * 503-504 — query/data.rs
    //   * 506-510 — query/state.rs / resource_registry
    // 460-462 is the closest contiguous free triplet.
    const COMP_A: ComponentId = ComponentId(460);
    const COMP_B: ComponentId = ComponentId(461);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompA(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompB(u32);

    impl Component for CompA {
        fn component_id() -> ComponentId {
            COMP_A
        }
    }
    impl Component for CompB {
        fn component_id() -> ComponentId {
            COMP_B
        }
    }

    /// Idempotent registry priming.
    fn register_test_components() {
        component_registry::register_layout::<CompA>(COMP_A.0);
        component_registry::register_layout::<CompB>(COMP_B.0);
    }

    /// Spawns a `CompA(value)` entity into `arch_id`.
    fn spawn_a(ecs: &mut EcsMaster, arch_id: ArchetypeId, value: u32) {
        let comp = CompA(value);
        // SAFETY: `CompA` is `#[repr(C)]` POD; reading its bytes produces a
        //   valid byte slice for the duration of this call.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &comp as *const CompA as *const u8,
                std::mem::size_of::<CompA>(),
            )
        };
        ecs.create_entity(arch_id, &[(COMP_A, bytes)])
            .expect("spawn_a: create_entity must succeed");
    }

    /// Spawns a `(CompA(a), CompB(b))` entity into `arch_id`.
    fn spawn_ab(ecs: &mut EcsMaster, arch_id: ArchetypeId, a: u32, b: u32) {
        let ca = CompA(a);
        let cb = CompB(b);
        // SAFETY: both are `#[repr(C)]` POD; the byte slices are valid for
        //   this call's duration.
        let a_bytes = unsafe {
            std::slice::from_raw_parts(
                &ca as *const CompA as *const u8,
                std::mem::size_of::<CompA>(),
            )
        };
        let b_bytes = unsafe {
            std::slice::from_raw_parts(
                &cb as *const CompB as *const u8,
                std::mem::size_of::<CompB>(),
            )
        };
        ecs.create_entity(arch_id, &[(COMP_A, a_bytes), (COMP_B, b_bytes)])
            .expect("spawn_ab: create_entity must succeed");
    }

    /// Single archetype with N entities — the closure fires exactly once and
    /// receives a slice covering every row.
    #[test]
    fn sequential_single_archetype_yields_full_slice() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A]);
        for i in 0..10u32 {
            spawn_a(&mut ecs, arch, i + 100);
        }

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        let _meta = SystemMeta::for_testing("chunk_iter::single_archetype");
        // SAFETY (U_C1): `cell` is consumed inside this scope; it does not
        //   outlive the `&mut ecs` borrow above.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let mut invocations = 0usize;
        let mut collected: Vec<u32> = Vec::with_capacity(10);
        // SAFETY (Q1, CD1-CD4): direct driver test; no aliasing accessor is
        //   live in this scope. `D = &CompA` ⇒ `IS_READ_ONLY = true` ⇒
        //   `mutable = false`. `F = ()` ⇒ archetypal.
        unsafe {
            let ids = state.archetype_state.matched_ids_pre_terms();
            for_each_chunk_impl::<&CompA, (), _>(&state, ids, cell, false, |slice: &[CompA]| {
                invocations += 1;
                for c in slice {
                    collected.push(c.0);
                }
            });
        }

        assert_eq!(invocations, 1, "single archetype ⇒ exactly one closure invocation");
        assert_eq!(collected.len(), 10, "slice must cover every row");
        // Values 100..=109 must all appear; order within the archetype is
        // insertion order (Phase 7 ComponentPool).
        for expected in 100..110u32 {
            assert!(
                collected.contains(&expected),
                "row {expected} must appear in collected = {collected:?}",
            );
        }
    }

    /// Two matched archetypes with disjoint row counts — the closure fires
    /// once per archetype with a slice of the correct length each time.
    #[test]
    fn sequential_multi_archetype_yields_per_archetype_slice() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch_a = ecs.create_archetype(&[COMP_A]);
        let arch_ab = ecs.create_archetype(&[COMP_A, COMP_B]);

        // 4 entities in arch_a, 6 in arch_ab.
        for i in 0..4u32 {
            spawn_a(&mut ecs, arch_a, i + 200);
        }
        for i in 0..6u32 {
            spawn_ab(&mut ecs, arch_ab, i + 300, 0);
        }

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms().len(),
            2,
            "both CompA-bearing archetypes must be matched",
        );

        // SAFETY (U_C1): cell consumed within this scope.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let mut slice_lens: Vec<usize> = Vec::with_capacity(2);
        let mut total: usize = 0;
        // SAFETY (Q1, CD1-CD4): direct driver test; `D = &CompA` read-only;
        //   `F = ()` archetypal. No aliasing live.
        unsafe {
            let ids = state.archetype_state.matched_ids_pre_terms();
            for_each_chunk_impl::<&CompA, (), _>(&state, ids, cell, false, |slice: &[CompA]| {
                slice_lens.push(slice.len());
                total += slice.len();
            });
        }

        assert_eq!(slice_lens.len(), 2, "two archetypes ⇒ two closure invocations");
        assert_eq!(total, 10, "total rows across both archetypes = 4 + 6 = 10");
        // Order is `QueryState::matched_ids` order — verify the multiset of
        // lengths without committing to a specific ordering.
        slice_lens.sort_unstable();
        assert_eq!(slice_lens, vec![4, 6], "per-archetype lengths must be {{4, 6}}");
    }

    /// A matched archetype with zero entities — the closure MUST NOT fire
    /// for that archetype (driver skip at the `entity_count == 0` guard).
    #[test]
    fn sequential_empty_archetype_skipped() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch_empty = ecs.create_archetype(&[COMP_A]);
        let _ = arch_empty;
        // Leave arch_empty entityless; deliberately do not populate.

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        // Matched-ids should still contain the empty archetype — the
        // archetype-match cache is rows-agnostic.
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms().len(),
            1,
            "the empty CompA archetype must still be matched at the cache level",
        );

        // SAFETY (U_C1): cell consumed within this scope.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let mut invocations = 0usize;
        // SAFETY (Q1, CD1-CD4): direct driver test; read-only; archetypal
        //   filter; no aliasing live.
        unsafe {
            let ids = state.archetype_state.matched_ids_pre_terms();
            for_each_chunk_impl::<&CompA, (), _>(&state, ids, cell, false, |_slice: &[CompA]| {
                invocations += 1;
            });
        }

        assert_eq!(
            invocations, 0,
            "empty archetype must trigger the `entity_count == 0` skip — closure must NOT fire",
        );
    }

    // ── Phase X.A Wave 7 Step 7A — additional §11.1 unit tests ──────────────
    //
    // Component-id slot reservations for Wave 7 chunk_iter unit tests
    // (in addition to the Wave 4 reservations 460-461 above):
    //
    //   * 467 — `CompC` (marker-style, used by the tuple-3 and With<C> tests)
    //   * 468 — `CompD` (used by the tuple-3 test as the third element)
    //   * 469 — `MarkerE` (used by the empty-tuple D With<MarkerE> test)
    //
    // The 467-479 range is reserved per Wave 7 plan §7A; the 470-479 slice is
    // left free for future expansion.

    /// Marker / payload component — third element of the 3-tuple test and the
    /// `With<CompC>` archetype distinguisher for `multi_archetype_dispatch`.
    const COMP_C: ComponentId = ComponentId(467);
    /// Fourth scratch component for the 3-tuple `(&A, &mut B, &C)` test.
    const COMP_D: ComponentId = ComponentId(468);
    /// Pure marker used by the empty-tuple `D = ()` + `With<MarkerE>` test.
    const COMP_E: ComponentId = ComponentId(469);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompC(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CompD(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MarkerE(u32);

    impl Component for CompC {
        fn component_id() -> ComponentId {
            COMP_C
        }
    }
    impl Component for CompD {
        fn component_id() -> ComponentId {
            COMP_D
        }
    }
    impl Component for MarkerE {
        fn component_id() -> ComponentId {
            COMP_E
        }
    }

    /// Idempotent registry priming for the Wave 7 component pack.
    fn register_wave7_components() {
        component_registry::register_layout::<CompC>(COMP_C.0);
        component_registry::register_layout::<CompD>(COMP_D.0);
        component_registry::register_layout::<MarkerE>(COMP_E.0);
    }

    /// Spawns a `(CompA(a), CompB(b), CompD(d))` entity into `arch_id`.
    fn spawn_abd(ecs: &mut EcsMaster, arch_id: ArchetypeId, a: u32, b: u32, d: u32) {
        let ca = CompA(a);
        let cb = CompB(b);
        let cd = CompD(d);
        // SAFETY: each component is `#[repr(C)]` POD; the byte slices are
        //   valid for this call's duration.
        let a_bytes = unsafe {
            std::slice::from_raw_parts(
                &ca as *const CompA as *const u8,
                std::mem::size_of::<CompA>(),
            )
        };
        let b_bytes = unsafe {
            std::slice::from_raw_parts(
                &cb as *const CompB as *const u8,
                std::mem::size_of::<CompB>(),
            )
        };
        let d_bytes = unsafe {
            std::slice::from_raw_parts(
                &cd as *const CompD as *const u8,
                std::mem::size_of::<CompD>(),
            )
        };
        ecs.create_entity(
            arch_id,
            &[(COMP_A, a_bytes), (COMP_B, b_bytes), (COMP_D, d_bytes)],
        )
        .expect("spawn_abd: create_entity must succeed");
    }

    /// Spawns a `(CompA(a), MarkerE)` entity into `arch_id`. The marker payload
    /// is `0u32`; only the presence of the component is meaningful for the
    /// `With<MarkerE>` filter.
    fn spawn_a_marker(ecs: &mut EcsMaster, arch_id: ArchetypeId, a: u32) {
        let ca = CompA(a);
        let m = MarkerE(0);
        // SAFETY: both are `#[repr(C)]` POD; byte slices valid for the call.
        let a_bytes = unsafe {
            std::slice::from_raw_parts(
                &ca as *const CompA as *const u8,
                std::mem::size_of::<CompA>(),
            )
        };
        let m_bytes = unsafe {
            std::slice::from_raw_parts(
                &m as *const MarkerE as *const u8,
                std::mem::size_of::<MarkerE>(),
            )
        };
        ecs.create_entity(arch_id, &[(COMP_A, a_bytes), (COMP_E, m_bytes)])
            .expect("spawn_a_marker: create_entity must succeed");
    }

    /// Q5 stale-id-skip path: push a synthetic non-existent `ArchetypeId` into
    /// `matched_ids` via the `matched_ids_mut` escape hatch — the driver must
    /// transparently skip it via the `archetype_ptr` `None` arm. Mirrors the
    /// pattern from `iter.rs::stale_id_skipped` (Phase 8b regression).
    #[test]
    fn stale_archetype_id_skipped() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch_a = ecs.create_archetype(&[COMP_A]);
        let arch_ab = ecs.create_archetype(&[COMP_A, COMP_B]);
        // Two real entities — one per archetype.
        spawn_a(&mut ecs, arch_a, 700);
        spawn_ab(&mut ecs, arch_ab, 701, 0);

        let mut state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms().len(),
            2,
            "both CompA-bearing archetypes must be matched before tampering",
        );

        // Push a synthetic stale ArchetypeId(999) — the master never minted
        // this slot, so `archetype_ptr(999)` returns `None` and the driver
        // skips via the `continue` arm.
        state
            .archetype_state
            .matched_ids_pre_terms_mut()
            .push(crate::ecs::identifiers::primitives::ArchetypeId(999));

        // SAFETY (U_C1): cell consumed inside this scope.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let mut invocations = 0usize;
        let mut total: usize = 0;
        // SAFETY (Q1, CD1-CD4): direct driver test; D = &CompA read-only; F =
        //   () archetypal. Stale id (999) is exactly the case the `continue`
        //   branch handles — driver returns `None` from `archetype_ptr` and
        //   skips without touching the user closure.
        unsafe {
            let ids = state.archetype_state.matched_ids_pre_terms();
            for_each_chunk_impl::<&CompA, (), _>(&state, ids, cell, false, |slice: &[CompA]| {
                invocations += 1;
                total += slice.len();
            });
        }

        assert_eq!(
            invocations, 2,
            "stale id must be skipped — only the 2 live archetypes invoke the closure",
        );
        assert_eq!(
            total, 2,
            "total rows across both live archetypes = 1 + 1 = 2 (no rows attributed to the stale id)",
        );
    }

    /// `Query<&mut CompA>::for_each_chunk` mutates every row of a 100-entity
    /// archetype; a fresh read-only `for_each_chunk` confirms every value
    /// doubled. Exercises the `set_chunk_mut` arm of the dispatcher and the
    /// `&mut [T]` chunk-item materialisation for the `&mut T` impl.
    #[test]
    fn single_component_write_doubles() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A]);
        for i in 0..100u32 {
            spawn_a(&mut ecs, arch, i);
        }

        // Phase 1 — mutate every row via the &mut driver.
        {
            let state = QueryDataState::<&mut CompA, ()>::new(&mut ecs);
            // SAFETY (U_C1): cell consumed in this block.
            let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
            // SAFETY (Q1, CD1-CD4): direct mut driver test; mutable = true;
            //   F = () archetypal; no aliasing accessor live.
            unsafe {
                let ids = state.archetype_state.matched_ids_pre_terms();
                for_each_chunk_impl::<&mut CompA, (), _>(
                    &state,
                    ids,
                    cell,
                    true,
                    |slice: &mut [CompA]| {
                        for c in slice.iter_mut() {
                            c.0 = c.0.wrapping_mul(2);
                        }
                    },
                );
            }
        }

        // Phase 2 — re-read with a fresh read-only driver; every row must be
        // `original * 2`.
        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        // SAFETY (U_C1): cell consumed in this block.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let mut collected: Vec<u32> = Vec::with_capacity(100);
        // SAFETY (Q1, CD1-CD4): read-only re-iteration; no aliasing live.
        unsafe {
            let ids = state.archetype_state.matched_ids_pre_terms();
            for_each_chunk_impl::<&CompA, (), _>(&state, ids, cell, false, |slice: &[CompA]| {
                for c in slice {
                    collected.push(c.0);
                }
            });
        }

        assert_eq!(collected.len(), 100, "every row must reappear after mutation");
        // The driver guarantees insertion-order; every spawned `i` should now
        // appear as `i * 2`. Sort to make the check order-independent in case
        // of future driver reordering.
        collected.sort_unstable();
        let expected: Vec<u32> = (0..100u32).map(|i| i.wrapping_mul(2)).collect();
        assert_eq!(
            collected, expected,
            "every CompA(i) must now read back as CompA(i*2)",
        );
    }

    /// 3-tuple `Query<(&CompA, &mut CompB, &CompD)>` on a single archetype with
    /// 7 entities — the closure must receive a 3-tuple of slices, each of the
    /// same length (= row count). Verifies the per-element `ChunkItem<'_>`
    /// projection and the tuple-fetch tuple-of-slices materialisation.
    #[test]
    fn tuple_3_yields_three_same_length_slices() {
        register_test_components();
        register_wave7_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[COMP_A, COMP_B, COMP_D]);
        for i in 0..7u32 {
            spawn_abd(&mut ecs, arch, i + 10, i + 20, i + 30);
        }

        let state = QueryDataState::<(&CompA, &mut CompB, &CompD), ()>::new(&mut ecs);
        // SAFETY (U_C1): cell consumed within this scope.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let mut invocations = 0usize;
        let mut observed_lens: Vec<(usize, usize, usize)> = Vec::new();
        // SAFETY (Q1, CD1-CD4): direct driver test; tuple `(&A, &mut B, &C)`
        //   ⇒ overall mutable (any element with `IS_READ_ONLY = false` flips
        //   the tuple flag); F = () archetypal; no aliasing live.
        unsafe {
            let ids = state.archetype_state.matched_ids_pre_terms();
            for_each_chunk_impl::<(&CompA, &mut CompB, &CompD), (), _>(
                &state,
                ids,
                cell,
                true,
                |(a, b, c): (&[CompA], &mut [CompB], &[CompD])| {
                    invocations += 1;
                    observed_lens.push((a.len(), b.len(), c.len()));
                },
            );
        }

        assert_eq!(invocations, 1, "single archetype ⇒ exactly one closure invocation");
        assert_eq!(observed_lens.len(), 1, "exactly one slice triple observed");
        let (la, lb, lc) = observed_lens[0];
        assert_eq!(la, 7, "&CompA slice length must equal row count");
        assert_eq!(lb, 7, "&mut CompB slice length must equal row count");
        assert_eq!(lc, 7, "&CompD slice length must equal row count");
        assert_eq!(
            la, lb,
            "all tuple slices must share a common length (row count)",
        );
        assert_eq!(
            lb, lc,
            "all tuple slices must share a common length (row count)",
        );
    }

    /// `Query<(), With<MarkerE>>::for_each_chunk(|()| count += 1)` — empty
    /// tuple `D = ()` on N matched archetypes invokes the closure with `()`
    /// exactly N times. The driver still walks `matched_ids_pre_terms()`; the `()` data
    /// fetch is a no-op materialiser that yields a unit value per non-empty
    /// archetype.
    #[test]
    fn empty_tuple_d_yields_unit_per_archetype() {
        register_test_components();
        register_wave7_components();
        let mut ecs = EcsMaster::new();
        // Two archetypes carrying the `MarkerE` marker — both should match
        // `With<MarkerE>`. A third archetype lacking the marker is the
        // negative-control; it must NOT contribute an invocation.
        let arch_marked_a = ecs.create_archetype(&[COMP_A, COMP_E]);
        let arch_marked_b = ecs.create_archetype(&[COMP_A, COMP_B, COMP_E]);
        let arch_unmarked = ecs.create_archetype(&[COMP_A]);

        // Populate both marked archetypes; leave the unmarked one alone (it
        // must still appear in the master's archetype set without participating
        // in the `With<MarkerE>` filter).
        spawn_a_marker(&mut ecs, arch_marked_a, 1);
        spawn_a_marker(&mut ecs, arch_marked_a, 2);
        // For arch_marked_b we need (A, B, MarkerE) — reuse the abd helper
        // by passing CompD slot? No, the archetype is registered with E not D.
        // Spawn a marker + a CompB row manually.
        {
            let ca = CompA(3);
            let cb = CompB(4);
            let m = MarkerE(0);
            // SAFETY: each is `#[repr(C)]` POD; byte slices valid for the call.
            let a_bytes = unsafe {
                std::slice::from_raw_parts(
                    &ca as *const CompA as *const u8,
                    std::mem::size_of::<CompA>(),
                )
            };
            let b_bytes = unsafe {
                std::slice::from_raw_parts(
                    &cb as *const CompB as *const u8,
                    std::mem::size_of::<CompB>(),
                )
            };
            let m_bytes = unsafe {
                std::slice::from_raw_parts(
                    &m as *const MarkerE as *const u8,
                    std::mem::size_of::<MarkerE>(),
                )
            };
            ecs.create_entity(
                arch_marked_b,
                &[(COMP_A, a_bytes), (COMP_B, b_bytes), (COMP_E, m_bytes)],
            )
            .expect("create_entity must succeed for arch_marked_b");
        }
        // The unmarked archetype is intentionally left without entities — we
        // only want to confirm it does not appear in the match cache.
        let _ = arch_unmarked;

        let state = QueryDataState::<(), crate::ecs::core::iters::query::With<MarkerE>>::new(
            &mut ecs,
        );
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms().len(),
            2,
            "exactly the two MarkerE-bearing archetypes must match (unmarked is filtered out)",
        );

        // SAFETY (U_C1): cell consumed within this scope.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let mut invocations = 0usize;
        // SAFETY (Q1, CD1-CD4): D = () (no component bytes touched); F =
        //   With<MarkerE> archetypal. No aliasing live.
        unsafe {
            let ids = state.archetype_state.matched_ids_pre_terms();
            for_each_chunk_impl::<(), crate::ecs::core::iters::query::With<MarkerE>, _>(
                &state,
                ids,
                cell,
                false,
                |_: ()| {
                    invocations += 1;
                },
            );
        }

        assert_eq!(
            invocations, 2,
            "exactly one closure invocation per matched non-empty archetype",
        );
    }
}
