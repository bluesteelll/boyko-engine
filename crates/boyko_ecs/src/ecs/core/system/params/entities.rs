//! `Entities<'w>` — read-only `SystemParam` resolving an [`EntityId`] to a
//! live [`Entity`] handle.
//!
//! Aether v2 kernel backlog **KE2** (rung R1). Minimal projection of
//! [`EntityMaster::entities_inland`], the Phase-7 entity fast store.
//!
//! # Why it exists
//!
//! Query iteration yields [`EntityId`] (an index — no generation); events and
//! per-entity machines carry [`Entity`] (index **plus** generation). Before this
//! param the only resolver — [`EntityMaster::get_entity`] — sat behind
//! `&EcsMaster`, reachable only from an **exclusive** system. A per-entity
//! machine binding `me: Entity` cannot be exclusive, so the resolver had to
//! become an ordinary param.
//!
//! # Type-enforced narrowness (the `EntityCounter` precedent)
//!
//! The carried pointer's destination type is [`InlandStore`], **not**
//! [`EntityMaster`]. There is therefore no compile-time path from an `Entities`
//! to `free_entity_ids`, `live_count`, or the `next_entity_id` atomic — exactly
//! the EM6 field-restriction argument
//! [`EntityCounter`](super::entity_counter::EntityCounter) makes with its
//! `*const AtomicUsize`. The narrowness is a property of the type, not of a
//! convention some later edit could quietly drop.
//!
//! # Access declaration — deliberately empty
//!
//! `Entities` declares nothing to the [`FilteredAccessSet`]. That accumulator
//! models exactly two axes, components and resources; the entity fast store is
//! neither, so there is no bit to set and no conflict to express. Same footing
//! as [`Commands`](super::commands::Commands), whose EM6 counter projection also
//! declares nothing. This is not an under-declaration escape hatch: the reads
//! below are covered by the SEND5/SCH7 argument, not by the conflict graph.
//!
//! # Soundness (SEND5 / SCH7)
//!
//! `EntityMaster`'s own `unsafe impl Sync` already blesses `&self` reads of
//! `entities_inland` from worker threads — `is_entity_valid`, `get_entity`, and
//! the inline reads driven by `EcsMaster::get_component_raw` all take that
//! route. `Entities` is a strictly narrower projection of the same field, so its
//! reads inherit that argument verbatim: every structural mutator
//! (`allocate_entity`, `register_entity_with_ptr`, `register_batch`,
//! `ensure_capacity`, `clear`, …) takes `&mut self` and runs only on the
//! dispatcher inside the apply window (SCH7), where no worker is live.
//!
//! # The debug assert mirrors the writer guard
//!
//! The writer-side guard is the `&mut self` receiver on those mutators — a
//! type-level fact the reader cannot observe. What the reader *can* observe is
//! its consequence: `len` is a plain non-atomic `usize`, and SCH7 says nothing
//! may move it while a system body runs. So `get_param` snapshots `len` at mint
//! time and every [`get`](Entities::get) `debug_assert!`s that it has not
//! changed. A concurrent grow — the exact data race `EntityMaster`'s SEND5
//! comment names as "still a data race, SCH7 exclusivity remains the normative
//! argument" — is a loud debug failure instead of a torn read. The snapshot
//! field is `#[cfg(debug_assertions)]`, so release layout is one pointer.
//!
//! [`EntityMaster`]: crate::ecs::core::entity::entity_master::EntityMaster
//! [`EntityMaster::entities_inland`]: crate::ecs::core::entity::entity_master::EntityMaster
//! [`EntityMaster::get_entity`]: crate::ecs::core::entity::entity_master::EntityMaster::get_entity
//! [`FilteredAccessSet`]: crate::ecs::core::system::filtered_access_set::FilteredAccessSet

use std::marker::PhantomData;

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::inland_store::InlandStore;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::EntityId;

/// Read-only resolver from [`EntityId`] to a live [`Entity`] handle.
///
/// Obtained as an ordinary system parameter:
///
/// ```ignore
/// fn aim(ents: Entities, q: Query<&Target>) {
///     for t in &q {
///         if let Some(victim) = ents.get(t.id) {
///             // `victim` carries the CURRENT generation — a recycled id
///             // cannot be mistaken for the entity that used to hold it.
///         }
///     }
/// }
/// ```
///
/// # Layout
///
/// Release: one pointer (8 B on the 64-bit target, const-asserted below).
/// Debug builds add the `len_at_mint` snapshot that backs the SCH7 tripwire.
#[derive(Clone, Copy)]
pub struct Entities<'w> {
    /// Raw pointer to the world's entity fast store.
    ///
    /// Minted exclusively by [`UnsafeEcsCell::entity_inland_store`]. The
    /// destination type is [`InlandStore`], which is what makes the EM6
    /// narrowness type-enforced.
    store: *const InlandStore,

    /// Debug-only snapshot of the store's live length at `get_param` time —
    /// the reader-side mirror of the writers' `&mut self` guard. See the module
    /// docs.
    #[cfg(debug_assertions)]
    len_at_mint: usize,

    /// Variance marker — ties the pointer's apparent validity to `'w`.
    _marker: PhantomData<&'w InlandStore>,
}

// SAFETY (SEND5, SCH7 — mirrors `EntityMaster`'s own `unsafe impl Send`):
//   `Entities` carries a `*const InlandStore` and performs only shared reads
//   through it (`len`, then one 16-byte `EntityInland` slot). `EntityMaster` is
//   already `Send + Sync` under an argument that explicitly covers `&self` reads
//   of this very field from worker threads; every structural mutation of the
//   store takes `&mut EntityMaster` and runs dispatcher-solo inside the apply
//   window (SCH7), so no mutation can be in flight while a worker holds this
//   projection. The pointer's destination type forbids reaching any other
//   `EntityMaster` field (EM6).
unsafe impl Send for Entities<'_> {}

// SAFETY (SEND5, SCH7): same composition as `Send`. `&Entities` exposes only
//   `get(&self)`, a shared read; concurrent immutable references from multiple
//   threads alias read-only bytes that no live writer can be touching, by the
//   apply-window argument above.
unsafe impl Sync for Entities<'_> {}

// Release layout contract: exactly one pointer. Gated to 64-bit (the engine's
// supported platform — see CLAUDE.md) and to release, because the debug build
// deliberately carries the SCH7 snapshot alongside (the same shape
// `UnsafeEcsCell` uses for its `allows_mutable_access` sentinel).
#[cfg(all(target_pointer_width = "64", not(debug_assertions)))]
const _: () = assert!(core::mem::size_of::<Entities<'static>>() == 8);
#[cfg(all(target_pointer_width = "64", not(debug_assertions)))]
const _: () = assert!(core::mem::align_of::<Entities<'static>>() == 8);

impl<'w> Entities<'w> {
    /// Constructs an `Entities` from a raw pointer to the world's entity fast
    /// store.
    ///
    /// # Safety
    ///
    /// * `store` must be a valid `*const InlandStore` for the entirety of `'w`.
    /// * It must point at the `entities_inland` field of an
    ///   [`EntityMaster`](crate::ecs::core::entity::entity_master::EntityMaster)
    ///   whose lifetime contains `'w` — minted by
    ///   [`UnsafeEcsCell::entity_inland_store`].
    /// * No structural mutation of that store may run for the lifetime `'w`
    ///   (SCH7 — the apply-window exclusivity every writer already relies on).
    /// * EM6 is upheld by the destination type: the pointer cannot be projected
    ///   to a sibling `EntityMaster` field.
    #[inline]
    pub(crate) unsafe fn from_store_ptr(store: *const InlandStore) -> Self {
        Self {
            store,
            #[cfg(debug_assertions)]
            // SAFETY: the caller's first contract clause — `store` is a valid
            //   `*const InlandStore` for `'w`; `len()` is a shared read of the
            //   `Deref` slice length.
            len_at_mint: unsafe { (&*store).len() },
            _marker: PhantomData,
        }
    }

    /// Resolves `id` to the live [`Entity`] currently occupying it, or `None`
    /// if the id is out of the store's live range or its slot is dead.
    ///
    /// The returned handle carries the slot's **current** generation, so a
    /// caller holding a pre-despawn handle for a recycled id can tell the two
    /// apart by comparing handles (not ids).
    ///
    /// Behaviourally identical to
    /// [`EntityMaster::get_entity`](crate::ecs::core::entity::entity_master::EntityMaster::get_entity),
    /// which is the same three-step read behind an exclusive `&EcsMaster`.
    ///
    /// # Cost
    ///
    /// O(1): one length compare, one 16-byte slot load, one null test.
    ///
    /// # Panics
    ///
    /// Debug builds only: panics if the store's live length changed since this
    /// param was minted (an SCH7 violation — see the module docs).
    #[inline]
    pub fn get(&self, id: EntityId) -> Option<Entity> {
        // SAFETY (SEND5, SCH7, EM6, `from_store_ptr` contract):
        //   * `store` was minted by `UnsafeEcsCell::entity_inland_store` from a
        //     live `EntityMaster` reachable through the cell, valid for `'w`.
        //   * The reborrow is shared and dies at the end of this function; no
        //     structural mutation can be in flight (apply-window exclusivity),
        //     which is the same condition `EntityMaster::get_entity` runs under.
        //   * The destination type is `InlandStore`, so no sibling field of
        //     `EntityMaster` is reachable through this pointer.
        let store = unsafe { &*self.store };
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            store.len(),
            self.len_at_mint,
            "invariant SCH7: the entity fast store grew while an `Entities` \
             param was live. Every structural mutator takes `&mut EntityMaster` \
             and runs dispatcher-solo inside the apply window; a length change \
             here means a writer ran concurrently with this system body"
        );
        let inland = store.get(id.0)?;
        if inland.is_null() {
            return None;
        }
        Some(Entity::new(id, inland.generation()))
    }
}

// SAFETY (SP1, SP2, SP4):
//   - SP1: `init_access` declares nothing, and nothing is the honest summary —
//     the param reads the entity fast store, an axis the `FilteredAccessSet`
//     does not model (it carries component and resource bits only). The same
//     declaration `Commands` makes for its EM6 counter projection.
//   - SP2: `get_param` mints a read-only projection of `entities_inland`. Its
//     soundness rests on SCH7 (no structural mutation while a system body runs)
//     rather than on the conflict graph — see the module docs.
//   - SP4: `init_state` is a no-op; it touches no registry.
unsafe impl<'a> SystemParam for Entities<'a> {
    type State = ();
    type Item<'w, 's> = Entities<'w>;

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {}

    #[inline]
    fn init_access(
        _state: &Self::State,
        _system_meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // Deliberately empty — see the module docs' "Access declaration"
        // section. The `FilteredAccessSet` has no entity-store axis to declare
        // against; SCH7 carries the soundness argument instead.
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        _state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (U_C2, EM6): the projection is a shared read of the world's
        //   `entity_master.entities_inland` field, scoped to `'w`. The by-value
        //   cell receiver preserves the raw pointer's provenance (no `&self`
        //   retag — Phase 8a C1).
        unsafe { world.entity_inland_store() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::entity::entity_inland::EntityInland;

    /// The layout pin — release is one pointer, debug carries the SCH7
    /// `len_at_mint` snapshot beside it.
    ///
    /// The EXPECTATION is profile-dependent; the TEST is not. Written the other
    /// way round — the whole `#[test]` behind `#[cfg(not(debug_assertions))]`,
    /// which is how it first landed — the pin vanished from every debug run,
    /// and every gate this repo runs by default is a debug build, so nothing any
    /// gate executed checked the zero-cost claim; the module-level
    /// `const _: () = assert!(…)` is likewise release-only. The layout PROPERTY
    /// holds in both profiles, so hiding the test in one of them was a gate that
    /// could not fail rather than a claim that did not apply.
    ///
    /// `get_debug_asserts_when_the_store_grew_under_the_param` below stays
    /// `#[cfg(debug_assertions)]` and is NOT the same case: a `debug_assert!`
    /// does not exist in release, so there is no release behaviour for it to
    /// assert. This module's test count therefore still differs by that one
    /// between profiles, deliberately.
    #[test]
    fn entities_layout_is_one_pointer_plus_the_debug_snapshot() {
        // Stated in `usize` units rather than literal 8/16 so the pin says what
        // it means on any pointer width: the store pointer, plus the debug-only
        // length snapshot.
        let want = if cfg!(debug_assertions) {
            2 * core::mem::size_of::<usize>()
        } else {
            core::mem::size_of::<usize>()
        };
        assert_eq!(
            core::mem::size_of::<Entities<'_>>(),
            want,
            "Entities is the store pointer (+ the debug-only len_at_mint snapshot)"
        );
        assert_eq!(
            core::mem::align_of::<Entities<'_>>(),
            core::mem::align_of::<usize>()
        );
    }

    /// `Entities<'static>` satisfies Send + Sync — the unsafe impls above are
    /// the load-bearing declarations; this is the compile-time gate on them.
    #[test]
    fn entities_is_send_and_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<Entities<'static>>();
        assert_sync::<Entities<'static>>();
    }

    /// Compile-only shim — instantiating this proves `T: SystemParam`.
    fn assert_impl<T: SystemParam>() {}

    /// `Entities<'_>` satisfies the `SystemParam` bound.
    #[test]
    fn entities_is_system_param() {
        assert_impl::<Entities<'static>>();
    }

    /// Direct unit coverage of the three-branch read, against a store this test
    /// owns: out of range → `None`; live slot → the stored generation;
    /// null slot → `None`.
    #[test]
    fn get_resolves_range_liveness_and_generation() {
        let mut store = InlandStore::new();
        store.ensure(3);
        // Slot 1 is live with generation 5; slots 0 and 2 stay NULL
        // (invariant J — a never-written slot reads all-zero).
        store[1] = EntityInland::dangling_for_test(9, 5);

        // SAFETY: `store` outlives `ents`; no structural mutation runs between
        //   the mint and the reads below.
        let ents = unsafe { Entities::from_store_ptr(&store) };

        assert_eq!(
            ents.get(EntityId(1)),
            Some(Entity::new(EntityId(1), 5)),
            "a live slot must resolve with its STORED generation"
        );
        assert_eq!(ents.get(EntityId(0)), None, "a NULL slot must resolve to None");
        assert_eq!(
            ents.get(EntityId(3)),
            None,
            "an id past the live range must resolve to None, not panic"
        );
    }

    /// The SCH7 tripwire fires when the store grows under a live param.
    ///
    /// This test deliberately violates `from_store_ptr`'s third clause (no
    /// structural mutation for `'w`) — that violation is the thing under test.
    /// It is safe to *perform* because `InlandStore` never reallocates: the
    /// grow commits fresh pages at the frontier and every prior slot address is
    /// stable (Phase X.G, witness `addresses_stable_across_multi_slab_growth`),
    /// so nothing is dangling. The assert fires before any slot is read.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "invariant SCH7")]
    fn get_debug_asserts_when_the_store_grew_under_the_param() {
        let mut store = InlandStore::new();
        store.ensure(2);

        // SAFETY: minted from a raw pointer so the returned `Entities<'_>`
        //   carries a free lifetime — the borrow checker does not tie it to
        //   `store`, which is what lets this test stage the violation.
        let ents = unsafe { Entities::from_store_ptr(std::ptr::addr_of!(store)) };
        assert_eq!(ents.get(EntityId(0)), None, "baseline read must not trip");

        // The SCH7 violation: a structural grow while the param is live.
        store.ensure(64);
        let _ = ents.get(EntityId(0));
    }
}
