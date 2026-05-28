//! Phase X.A — sequential `Query::for_each_chunk` driver.
//!
//! Shared by [`Query::for_each_chunk`] and (when Wave 5 lands)
//! [`QueryView::for_each_chunk`]. Mirrors the outer-loop shape of
//! [`super::iter`] but yields one [`ChunkedQueryData::ChunkItem`] per
//! matched archetype instead of one [`QueryData::Item`] per row.
//!
//! # Hot loop shape
//!
//! For each `arch_id` in `state.archetype_state.matched_ids()`:
//!
//! 1. Mint a `*mut Archetype` (`mutable == true`) or reborrow `*const`
//!    as `*mut` (`mutable == false`) via [`UnsafeEcsCell::archetype_ptr_mut`]
//!    / [`UnsafeEcsCell::archetype_ptr`]. Stale ids (Q5) are skipped via
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
//! [`UnsafeEcsCell::archetype_ptr`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::archetype_ptr
//! [`UnsafeEcsCell::archetype_ptr_mut`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::archetype_ptr_mut
//! [fetch]: super::chunked_data::ChunkedQueryData::fetch_chunk

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::iters::query::chunked_data::ChunkedQueryData;
use crate::ecs::core::iters::query::filter::ArchetypalQueryFilter;
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Sequential chunked-iter driver. Shared between
/// [`Query::for_each_chunk`] and [`QueryView::for_each_chunk`].
///
/// # Hot loop
///
/// Outer loop iterates `state.archetype_state.matched_ids()`. Per
/// archetype: skip stale ids (Q5), skip empty archetypes, refresh
/// `ChunkFetch`, hand the full slice to `f`.
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
///   call `update` itself; it walks `state.archetype_state.matched_ids()`
///   verbatim. Stale ids (archetypes removed after the last sync) are
///   skipped transparently via the `archetype_ptr(_mut)` `None` arm.
///
/// [`Query::for_each_chunk`]: super::query::Query::for_each_chunk
/// [`QueryView::for_each_chunk`]: super::query_view::QueryView
/// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
pub(crate) unsafe fn for_each_chunk_impl<'q, 's, D, F, Func>(
    state: &'s QueryDataState<D, F>,
    world: UnsafeEcsCell<'q>,
    mutable: bool,
    mut f: Func,
) where
    D: ChunkedQueryData,
    F: ArchetypalQueryFilter,
    Func: for<'c> FnMut(D::ChunkItem<'c>),
{
    let mut chunk_fetch = <D as ChunkedQueryData>::init_chunk_fetch(&state.data_state);

    for &arch_id in state.archetype_state.matched_ids() {
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
            for_each_chunk_impl::<&CompA, (), _>(&state, cell, false, |slice: &[CompA]| {
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
            state.archetype_state.matched_ids().len(),
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
            for_each_chunk_impl::<&CompA, (), _>(&state, cell, false, |slice: &[CompA]| {
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
            state.archetype_state.matched_ids().len(),
            1,
            "the empty CompA archetype must still be matched at the cache level",
        );

        // SAFETY (U_C1): cell consumed within this scope.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        let mut invocations = 0usize;
        // SAFETY (Q1, CD1-CD4): direct driver test; read-only; archetypal
        //   filter; no aliasing live.
        unsafe {
            for_each_chunk_impl::<&CompA, (), _>(&state, cell, false, |_slice: &[CompA]| {
                invocations += 1;
            });
        }

        assert_eq!(
            invocations, 0,
            "empty archetype must trigger the `entity_count == 0` skip — closure must NOT fire",
        );
    }
}
