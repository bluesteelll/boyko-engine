//! Interior-mutability handle on `EcsMaster` for `SystemParam::get_param`.
//!
//! See Phase 8a plan §3 (Decision D1, C1 RESOLUTION) and invariants
//! `U_C1` / `U_C2` / `U_C3` in §10.
//!
//! # Why this exists (C1 — by-value receivers)
//!
//! Tuple impls of `SystemParam::get_param` need to fetch *several* params'
//! views of `EcsMaster` simultaneously — e.g. `(Res<A>, ResMut<B>)`. The
//! borrow checker forbids handing out two `&mut EcsMaster` borrows; we
//! sidestep it by passing a `Copy` raw-pointer cell whose method receivers
//! take `self` **by value**.
//!
//! By-value receivers are load-bearing under Tree Borrows. A `&self`
//! receiver would tag the cell's interior `ptr` as `SharedReadOnly` for the
//! call's duration; any `*mut` derived from that pointer inside the method
//! body would then be barred from writes — even though the underlying
//! `&mut EcsMaster` originally yielded write-capable provenance. Taking
//! `self` by value (the cell is `Copy`) flows the raw pointer through the
//! method without any intervening `&self` borrow.
//!
//! # !Send + !Sync
//!
//! `EcsMaster` is `!Send + !Sync`; the cell inherits the discipline. Phase 9
//! will introduce `Send/Sync` impls bound to an explicit scheduler-aliasing
//! contract; Phase 8a does not.

// Phase 8a Step 5: the remaining `pub(crate)` cell accessors are wired by
// Step 8 (`EcsMaster::run_system_once`). Step 7 (`Res::get_param`,
// `ResMut::get_param`) consumes `resources()` / `resources_mut()`; the other
// accessors (`world()`, `world_mut()`, `archetype_ptr*`) remain unused until
// Step 8 lands.
#![allow(dead_code)]

use std::marker::PhantomData;
use std::sync::atomic::AtomicUsize;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::resources::resources::Resources;
use crate::ecs::core::system::params::entity_counter::EntityCounter;
use crate::ecs::identifiers::primitives::ArchetypeId;

/// Copy-on-call interior-mutability handle on an `EcsMaster`.
///
/// Constructed via [`new_mutable`] (write-capable) or [`new_readonly`]
/// (read-only). All accessor methods take `self` **by value** — see the
/// module docs for the Tree Borrows rationale.
///
/// The cell is `Copy`, so tuple impls of `SystemParam::get_param` can hand
/// out one copy per param without contortions. Aliasing discipline between
/// the copies is enforced upstream by
/// [`FilteredAccessSet`] at `init_access` time
/// and the Phase 9 scheduler at run time — never by the cell itself.
///
/// [`new_mutable`]: UnsafeEcsCell::new_mutable
/// [`new_readonly`]: UnsafeEcsCell::new_readonly
/// [`FilteredAccessSet`]: super::FilteredAccessSet
#[derive(Clone, Copy)]
// `Copy` + by-value receivers are the C1 fix; this is the canonical Bevy
// `UnsafeWorldCell` shape. We do NOT apply `repr(transparent)` because the
// debug-only `allows_mutable_access` field violates the single-non-ZST-field
// requirement under `cfg(debug_assertions)`. The release-build layout is a
// raw pointer + ZSTs and the debug-build layout pads a `bool` — neither
// crosses the cell-Copy / by-value-receiver contract that matters for C1.
pub struct UnsafeEcsCell<'w> {
    /// Raw pointer to the underlying `EcsMaster`. Lifetime is enforced by
    /// `PhantomData<'w>` below — the cell may not outlive the borrow that
    /// produced it.
    ptr: *mut EcsMaster,
    /// Carries `&'w EcsMaster` (variance) + `&'w UnsafeCell<EcsMaster>`
    /// (interior-mutability marker). Same shape as Bevy's `UnsafeWorldCell`.
    _marker: PhantomData<(&'w EcsMaster, &'w core::cell::UnsafeCell<EcsMaster>)>,
    /// Debug-only sentinel: `true` for cells minted via [`new_mutable`],
    /// `false` for cells minted via [`new_readonly`]. `world_mut`,
    /// `resources_mut`, and `archetype_ptr_mut` `debug_assert!` on this.
    ///
    /// [`new_mutable`]: UnsafeEcsCell::new_mutable
    /// [`new_readonly`]: UnsafeEcsCell::new_readonly
    #[cfg(debug_assertions)]
    allows_mutable_access: bool,
}

impl<'w> UnsafeEcsCell<'w> {
    /// Mints a write-capable cell from `&mut EcsMaster`.
    ///
    /// # Safety (U_C1)
    /// * The returned cell must not outlive `'w`. The lifetime is carried
    ///   through `PhantomData`; releasing the cell to a longer scope is UB.
    /// * Aliasing discipline between cell copies is the caller's
    ///   responsibility per the `SystemParam` protocol (see SP1 / U_C2 /
    ///   U_C3). Phase 9 scheduler enforces cross-system aliasing; the
    ///   `FilteredAccessSet` accumulator enforces intra-system aliasing.
    #[inline]
    pub(crate) unsafe fn new_mutable(world: &'w mut EcsMaster) -> Self {
        Self {
            ptr: world as *mut EcsMaster,
            _marker: PhantomData,
            #[cfg(debug_assertions)]
            allows_mutable_access: true,
        }
    }

    /// Mints a read-only cell from `&EcsMaster`.
    ///
    /// Methods that require write capability (`world_mut`, `resources_mut`,
    /// `archetype_ptr_mut`) will `debug_assert!` on a cell minted via this
    /// constructor and panic in debug builds.
    ///
    /// # Safety (U_C1)
    /// * The returned cell must not outlive `'w`.
    /// * Only `world()` / `archetype_ptr()` (read-only) methods are
    ///   reachable per the protocol; calling the `_mut` variants is a
    ///   programmer bug detected via `debug_assert!`.
    #[inline]
    pub(crate) unsafe fn new_readonly(world: &'w EcsMaster) -> Self {
        // Cast `*const → *mut`: the `allows_mutable_access` sentinel keeps
        // write-capable methods from being reachable; only `world()` /
        // `archetype_ptr()` are usable on this cell.
        Self {
            ptr: world as *const EcsMaster as *mut EcsMaster,
            _marker: PhantomData,
            #[cfg(debug_assertions)]
            allows_mutable_access: false,
        }
    }

    /// Returns a shared reference to the `EcsMaster`.
    ///
    /// # Safety (U_C2)
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   declared a read; no `&mut EcsMaster` (or sibling-cell-mediated
    ///   write) aliases this borrow for the returned reference's scope.
    /// * The by-value receiver consumes a `Copy` of the cell — no `&self`
    ///   retag occurs and the raw pointer's provenance is preserved.
    #[inline]
    pub(crate) unsafe fn world(self) -> &'w EcsMaster {
        // SAFETY (U_C2): caller upholds the access contract; the raw
        //   pointer was produced by `new_mutable` / `new_readonly` from a
        //   live borrow scoped to `'w`. The by-value receiver consumes a
        //   Copy of the cell, so no `&self` borrow on the cell exists that
        //   could downgrade `ptr`'s provenance to SharedReadOnly.
        unsafe { &*self.ptr }
    }

    /// Returns an exclusive reference to the `EcsMaster`.
    ///
    /// # Safety (U_C3)
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   declared a write that does not conflict with sibling params or
    ///   other systems; no other access through any cell copy aliases this
    ///   borrow for the returned reference's scope.
    /// * The cell was minted via [`new_mutable`] (debug-asserted).
    /// * The by-value receiver consumes a `Copy` of the cell — no `&self`
    ///   retag occurs.
    ///
    /// [`new_mutable`]: UnsafeEcsCell::new_mutable
    #[inline]
    pub(crate) unsafe fn world_mut(self) -> &'w mut EcsMaster {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.allows_mutable_access,
            "invariant U_C3: world_mut() called on a read-only UnsafeEcsCell \
             minted via new_readonly"
        );
        // SAFETY (U_C3): caller upholds the access contract; the raw
        //   pointer carries write-capable provenance (minted from
        //   `&mut EcsMaster` in `new_mutable`). By-value receiver consumes
        //   the cell Copy, so the pointer is not retagged to SharedReadOnly
        //   before the dereference.
        unsafe { &mut *self.ptr }
    }

    /// Read-only Phase 7 U11 recipe: mints a `*const Archetype` for `id`.
    ///
    /// Returns `None` if no archetype is registered for `id`.
    ///
    /// # Safety (U_C2)
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   declared a read of the archetype's columns; no `&mut Archetype`
    ///   alias is live through any cell copy.
    #[inline]
    pub(crate) unsafe fn archetype_ptr(self, id: ArchetypeId) -> Option<*const Archetype> {
        // SAFETY (U_C2): by-value receiver — `self.world()` is a self-by-value
        //   call so the raw pointer is not retagged. The returned reference
        //   is scoped to `'w` (Phase 7 U1/U2 slab stability).
        unsafe { self.world().archetype_master().get_archetype_ptr(id) }
    }

    /// Write-capable Phase 7 U11 recipe: mints a `*mut Archetype` for
    /// `id`.
    ///
    /// Returns `None` if no archetype is registered for `id`.
    ///
    /// # Safety (U_C3)
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   declared a write to the archetype's columns; no other reference
    ///   aliases.
    /// * The cell was minted via [`new_mutable`] (debug-asserted).
    /// * The by-value receiver keeps the raw pointer's write-capable
    ///   provenance intact — the C1 fix for Round 1's `&self` retag bug.
    ///
    /// [`new_mutable`]: UnsafeEcsCell::new_mutable
    #[inline]
    pub(crate) unsafe fn archetype_ptr_mut(
        self,
        id: ArchetypeId,
    ) -> Option<*mut Archetype> {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.allows_mutable_access,
            "invariant U_C3: archetype_ptr_mut() called on a read-only UnsafeEcsCell"
        );
        // SAFETY (U_C3): by-value `self.world_mut()` consumes the cell
        //   Copy; the underlying raw pointer keeps write-capable provenance
        //   and is reborrowed as `&mut EcsMaster` inside the call. The
        //   minted `*mut Archetype` follows Phase 7's U14 raw-provenance
        //   recipe (`archetype_ptr_for` under `&mut`).
        unsafe { self.world_mut().archetype_master_mut().archetype_ptr_for(id) }
    }

    /// Direct read-only access to the resources subsystem. Hot path for
    /// [`Res<R>::get_param`] — avoids the full [`world`] materialisation when
    /// only the resources slab is needed.
    ///
    /// # Safety (U_C2)
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   declared a resource read; no `&mut Resources` aliases this borrow
    ///   through any cell copy for the returned reference's scope.
    /// * The by-value receiver consumes a `Copy` of the cell — no `&self`
    ///   retag occurs and the raw pointer's provenance is preserved.
    ///
    /// [`Res<R>::get_param`]: super::params::res::Res
    /// [`world`]: UnsafeEcsCell::world
    #[inline]
    pub(crate) unsafe fn resources(self) -> &'w Resources {
        // SAFETY (U_C2): by-value receiver; the raw `*mut EcsMaster` is not
        //   retagged (no intermediate `&self` borrow). The `&` operator
        //   applies directly to the projected field through `*self.ptr`,
        //   never constructing an `&EcsMaster` temporary that would
        //   SharedReadOnly-downgrade the pointer. `'w` lifetime is upheld by
        //   `new_*()` postconditions.
        unsafe { &(*self.ptr).resources }
    }

    /// Phase 11 (Round 3 C-N1): mints an [`EntityCounter<'s>`] projecting
    /// only the atomic `next_entity_id` counter from `EntityMaster`. The
    /// returned counter cannot reach any other `EntityMaster` field — the
    /// EM6 aliasing rule is type-enforced (the carried pointer's type is
    /// `*const AtomicUsize`, not `*const EntityMaster`).
    ///
    /// The lifetime `'s` may be shorter than `'w`; the caller (typically
    /// `Commands::get_param`) ties `'s` via `PhantomData` re-tag per the
    /// Phase 8c IntoSystem contract (plan §8.7 — `get_param` runs once
    /// per system invocation; `'w >= 's`).
    ///
    /// # Safety (U_C2, EM6)
    ///
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   permits conflict-free atomic-counter access — `Commands` declares
    ///   no access in the conflict graph (EVT1 precedent + EM6).
    /// * The by-value receiver preserves the raw pointer's provenance: no
    ///   `&self` retag downgrades the carried `*mut EcsMaster` before the
    ///   field projection.
    /// * `'s <= 'w` by the caller's PhantomData re-tag (the SystemParam
    ///   protocol enforces this on the consumer side).
    #[inline]
    pub(crate) unsafe fn entity_counter<'s>(self) -> EntityCounter<'s> {
        // SAFETY (U_C2, EM6):
        //   * By-value receiver — no `&self` retag. The underlying
        //     `*mut EcsMaster` is valid for `'w` and carries the
        //     original write-capable provenance from `new_mutable`.
        //   * Projecting `(*ptr).entity_master.next_id_atomic()` produces
        //     a `&AtomicUsize`. Going `&AtomicUsize -> *const AtomicUsize`
        //     keeps the atomic's address; this raw pointer is what
        //     `EntityCounter::from_ptr` re-tags to `'s`.
        //   * The field type at the destination is `AtomicUsize` — no
        //     compile-time path leads from the carried pointer to any
        //     other `EntityMaster` field, type-enforcing EM6.
        let em = unsafe { &(*self.ptr).entity_master };
        let atomic_ptr = em.next_id_atomic() as *const AtomicUsize;
        // SAFETY (`EntityCounter::from_ptr` contract, plan §5.5):
        //   * Pointer was just minted from a live `EntityMaster`
        //     reachable through `self.ptr`, valid for `'w >= 's`.
        //   * The pointer aims at the master's `next_entity_id` field
        //     (EM1) — the only blessed projection.
        //   * EM6 is upheld by the destination type — see above.
        unsafe { EntityCounter::from_ptr(atomic_ptr) }
    }

    /// Direct mutable access to the resources subsystem. Hot path for
    /// [`ResMut<R>::get_param`].
    ///
    /// # Safety (U_C3)
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   declared a resource write that does not conflict with sibling
    ///   params or other systems; no other access through any cell copy
    ///   aliases this borrow for the returned reference's scope.
    /// * The cell was minted via [`new_mutable`] (debug-asserted).
    /// * The by-value receiver consumes a `Copy` of the cell — no `&self`
    ///   retag occurs.
    ///
    /// [`ResMut<R>::get_param`]: super::params::resmut::ResMut
    /// [`new_mutable`]: UnsafeEcsCell::new_mutable
    #[inline]
    pub(crate) unsafe fn resources_mut(self) -> &'w mut Resources {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.allows_mutable_access,
            "invariant U_C3: resources_mut() called on a read-only UnsafeEcsCell \
             minted via new_readonly"
        );
        // SAFETY (U_C3): by-value receiver; raw pointer carries write-capable
        //   provenance (minted from `&mut EcsMaster` in `new_mutable`). The
        //   `&mut` operator projects directly through `*self.ptr` onto the
        //   `resources` field — no intermediate `&mut EcsMaster` reborrow
        //   that could downgrade the tag stack. Aliasing is the caller's
        //   responsibility per the SystemParam protocol.
        unsafe { &mut (*self.ptr).resources }
    }
}

// SAFETY (SEND2 / SEND3 — Phase 9 §2.4, §9.1):
//
// `UnsafeEcsCell<'w>` becomes `Send + Sync` under the Phase 9 contract. The
// cell holds a raw `*mut EcsMaster` plus `PhantomData`; worker threads receive
// `Copy` clones from the dispatcher per dispatch round (Round 2 O3). Aliasing
// discipline is enforced upstream by:
//
//   - `FilteredAccessSet` accumulation at `SystemParam::init_access` time
//     (intra-system aliasing).
//   - The scheduler's `ConflictGraph` (SCH3) at run time (cross-system
//     aliasing — no two concurrent systems hold overlapping `&/&mut` views
//     through their cell copies).
//   - The apply-window barrier (SCH7) — the only context in which the
//     dispatcher reborrows `&mut EcsMaster` is gated on `running == 0`, so
//     no live worker cell aliases the dispatcher reborrow.
//
// The cell itself never dereferences `ptr` outside an `unsafe` method whose
// SAFETY block documents the aliasing precondition.
unsafe impl<'w> Send for UnsafeEcsCell<'w> {}
unsafe impl<'w> Sync for UnsafeEcsCell<'w> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: a mutable cell can be constructed from `&mut EcsMaster`,
    /// then `world()` reads back the same address as the original borrow.
    /// Verifies the by-value `world()` path (C1 / U_C2).
    #[test]
    fn new_mutable_carries_write_capable_provenance() {
        let mut ecs = EcsMaster::new();
        let original_addr = (&raw const ecs) as usize;

        // SAFETY (U_C1): the cell does not outlive the `&mut ecs` borrow
        //   below — it is consumed (by value) by the read-back inside this
        //   function and never escapes.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };

        // SAFETY (U_C2): no other `&mut` borrow is live during this call;
        //   we only inspect the address.
        let observed: &EcsMaster = unsafe { cell.world() };
        let observed_addr = (observed as *const EcsMaster) as usize;
        assert_eq!(
            observed_addr, original_addr,
            "world() must yield a reference to the same EcsMaster"
        );
    }

    /// A read-only cell minted via `new_readonly` permits `world()` access.
    #[test]
    fn new_readonly_world_reads() {
        let ecs = EcsMaster::new();
        let original_addr = (&raw const ecs) as usize;

        // SAFETY (U_C1): cell does not outlive `&ecs`.
        let cell = unsafe { UnsafeEcsCell::new_readonly(&ecs) };
        // SAFETY (U_C2): only a read; no aliasing write live.
        let observed: &EcsMaster = unsafe { cell.world() };
        let observed_addr = (observed as *const EcsMaster) as usize;
        assert_eq!(observed_addr, original_addr);
    }

    /// In debug builds, calling `world_mut` on a `new_readonly` cell must
    /// trip the `allows_mutable_access` `debug_assert`.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "U_C3")]
    fn world_mut_on_readonly_cell_panics_in_debug() {
        let ecs = EcsMaster::new();
        let cell = unsafe { UnsafeEcsCell::new_readonly(&ecs) };
        // SAFETY: deliberately violates the U_C3 contract to verify the
        //   debug-mode assertion fires; the actual `&mut *ptr` deref never
        //   happens because the assert panics first.
        let _ = unsafe { cell.world_mut() };
    }

    /// `UnsafeEcsCell` is `Copy` — the by-value receiver pattern relies on
    /// this.
    #[test]
    fn cell_is_copy() {
        let mut ecs = EcsMaster::new();
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        let copy_a = cell;
        let copy_b = cell;
        // Both copies dereference to the same master.
        let addr_a = unsafe { copy_a.world() } as *const EcsMaster as usize;
        let addr_b = unsafe { copy_b.world() } as *const EcsMaster as usize;
        assert_eq!(addr_a, addr_b, "Copy cells must dereference identically");
    }
}
