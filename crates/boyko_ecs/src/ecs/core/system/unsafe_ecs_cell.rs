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
//!
//! # W0 — phase-frozen world bytes (PC-24)
//!
//! While any system body may be running on a worker, the only writes into the
//! `EcsMaster` allocation are atomic RMWs on `UnsafeCell`-backed fields (today
//! `EntityReservoir::{free_top, next_entity_id}`; by design
//! `ArchetypeMaster::enable_generation`), and no thread forms a `&mut` to
//! `EcsMaster` or to any of its inline sub-structures. Every `&mut EcsMaster` is
//! formed on the dispatcher with no worker live (`schedule.rs` apply-window /
//! EXC2 / condition gates, `exclusive_function_system.rs`, `DispatcherToken`).
//! Hence any number of workers may hold shared world references concurrently
//! with those RMWs: a shared retag reads only frozen bytes and never an
//! `UnsafeCell` byte, under Stacked Borrows (`nonfreeze_access: None`) and Tree
//! Borrows (`Cell`: no associated access) alike.
//!
//! A `&mut` retag, by contrast, covers its whole pointee, `UnsafeCell` bytes
//! included: a retag read of every byte under Tree Borrows, a retag write under
//! Stacked Borrows. A worker-side `&mut EcsMaster` therefore races the reservoir
//! RMWs of any concurrent `Commands` system, and under Stacked Borrows every
//! other worker's world retag too, even when the path writes nothing. That is
//! why the worker accessors below (`archetype_ptr_mut`, `resource_ptr_mut`)
//! reach their target through shared paths and hand out raw pointers instead.

// Phase 8a Step 5: the remaining `pub(crate)` cell accessors are wired by
// Step 8 (`EcsMaster::run_system_once`). Step 7 (`Res::get_param`,
// `ResMut::get_param`) consumes `resources()` / `resource_ptr_mut()`; the other
// accessors (`world()`, `world_mut()`, `archetype_ptr*`) remain unused until
// Step 8 lands.
#![allow(dead_code)]

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::resources::nonsend_resources::NonSendResources;
use crate::ecs::core::resources::resources::Resources;
use crate::ecs::core::system::params::entities::Entities;
use crate::ecs::core::system::params::entity_counter::EntityCounter;
use crate::ecs::identifiers::primitives::{ArchetypeId, ResourceId};

/// Copy-on-call interior-mutability handle on an `EcsMaster`.
///
/// Constructed via `new_mutable` (write-capable) or `new_readonly`
/// (read-only). All accessor methods take `self` **by value** — see the
/// module docs for the Tree Borrows rationale.
///
/// The cell is `Copy`, so tuple impls of `SystemParam::get_param` can hand
/// out one copy per param without contortions. Aliasing discipline between
/// the copies' component and resource views is enforced upstream by
/// [`FilteredAccessSet`] at `init_access` time and the Phase 9 scheduler at
/// run time. The world object itself is shared by every copy: no accessor
/// reachable from a worker forms a `&mut` over it (invariant W0, module docs).
///
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
    /// Debug-only sentinel: `true` for cells minted via `new_mutable`,
    /// `false` for cells minted via `new_readonly`. `world_mut`,
    /// `resource_ptr_mut`, and `archetype_ptr_mut` `debug_assert!` on this.
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
    /// Methods that require write capability (`world_mut`, `resource_ptr_mut`,
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
        // SAFETY (U_C2, W0): caller upholds the access contract; the raw
        //   pointer was produced by `new_mutable` / `new_readonly` from a
        //   live borrow scoped to `'w`. The shared retag reads only the
        //   world's frozen bytes, which nothing writes while a system body may
        //   run: the concurrent writes are atomic RMWs on `UnsafeCell` bytes,
        //   which a shared retag does not access, and no thread holds a `&mut`
        //   over the world meanwhile (W0). The by-value receiver consumes a
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
    /// * The cell was minted via `new_mutable` (debug-asserted).
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
        // SAFETY (U_C2, W0): by-value receiver — `self.world()` is a
        //   self-by-value call so the raw pointer is not retagged. The shared
        //   `&EcsMaster` / `&ArchetypeMaster` / `&ArchetypeBundle` retags on the
        //   way read only frozen bytes, which nothing writes during a phase
        //   (W0). The returned pointer is valid for `'w` (Phase 7 U1/U2 slab
        //   stability).
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
    /// * The cell was minted via `new_mutable` (debug-asserted).
    /// * Rides the shared [`archetype_ptr`](UnsafeEcsCell::archetype_ptr)
    ///   route and forms no `&mut` over the world (PC-24, W0): this runs on
    ///   workers, where a whole-world `&mut` retag races the reservoir RMWs of
    ///   any concurrent `Commands` system. The read route mints the SAME
    ///   write-capable provenance (`archetype_bundle.rs`'s F4 notes on
    ///   `get_archetype_ptr_mut` / `get_archetype_ptr`), so the cast back to
    ///   `*mut` restores the type and nothing else.
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
        // SAFETY (U_C3, W0 — PC-24):
        //   - No `&mut` to `EcsMaster`, `ArchetypeMaster` or `ArchetypeBundle`
        //     is formed. This runs on workers, and a `&mut` retag covers its
        //     whole pointee, `UnsafeCell` bytes included (Tree Borrows: a retag
        //     read of every byte; Stacked Borrows: a retag write). The
        //     reservoir that a concurrent `Commands` system RMWs lives inline
        //     in the world (`EntityMaster::reservoir`), so such a retag is a
        //     data race even though this path writes nothing.
        //   - `archetype_ptr` reaches the slot through
        //     `ArchetypeBundle::get_archetype_ptr`, i.e. `slot_ptr_mut`'s
        //     `UnsafeCell::raw_get` over the slab's own allocation. That is the
        //     SAME write-capable provenance `get_archetype_ptr_mut` mints (F4:
        //     the read/write split is a caller contract, not a provenance
        //     one), so the cast restores the type and nothing else.
        //   - The shared retags on the way read only frozen bytes, which
        //     nothing writes during a phase (W0).
        //   - Writing through the result stays exclusive by the caller's
        //     declared column write (U_C3), as before.
        unsafe { self.archetype_ptr(id) }.map(<*const Archetype>::cast_mut)
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

    /// Phase 11 (Round 3 C-N1), re-based by EM2′: mints an
    /// [`EntityCounter<'s>`] projecting only `EntityMaster::reservoir` — the
    /// fresh-id counter plus the claimable recycled-entity stack. The returned
    /// counter cannot reach any other `EntityMaster` field — the EM6′ aliasing
    /// rule is type-enforced (the carried pointer's type is
    /// `*const EntityReservoir`, not `*const EntityMaster`).
    ///
    /// The lifetime `'s` may be shorter than `'w`; the caller (typically
    /// `Commands::get_param`) ties `'s` via `PhantomData` re-tag per the
    /// Phase 8c IntoSystem contract (plan §8.7 — `get_param` runs once
    /// per system invocation; `'w >= 's`).
    ///
    /// # Safety (U_C2, EM6′, SCH7)
    ///
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   permits conflict-free reservoir access — `Commands` declares
    ///   no access in the conflict graph (EVT1 precedent + EM6′): the atomics
    ///   are RMW'd from any thread, and the stack entries a claim reads are
    ///   immutable for the phase because every write to them takes
    ///   `&mut EntityMaster`, which runs dispatcher-solo in the apply window
    ///   (SCH7 / EM2′-K), and no worker forms a `&mut` over the world or any
    ///   of its sub-structures at all (W0).
    /// * The by-value receiver preserves the raw pointer's provenance: no
    ///   `&self` retag downgrades the carried `*mut EcsMaster` before the
    ///   field projection.
    /// * `'s <= 'w` by the caller's PhantomData re-tag (the SystemParam
    ///   protocol enforces this on the consumer side).
    #[inline]
    pub(crate) unsafe fn entity_counter<'s>(self) -> EntityCounter<'s> {
        // SAFETY (U_C2, EM6′):
        //   * By-value receiver — no `&self` retag. The underlying
        //     `*mut EcsMaster` is valid for `'w` and carries the original
        //     provenance from `new_mutable`.
        //   * `&raw const` projects the `reservoir` field's address without
        //     materializing any reference to `EcsMaster` or `EntityMaster`, so
        //     no intermediate borrow narrows or retags the provenance; the
        //     pointer covers exactly the `EntityReservoir`.
        let reservoir = unsafe { &raw const (*self.ptr).entity_master.reservoir };
        // SAFETY (`EntityCounter::from_ptr` contract, plan §5.5):
        //   * Pointer was just projected from a live `EcsMaster` reachable
        //     through `self.ptr`, valid for `'w >= 's`.
        //   * It aims at the master's `reservoir` field — the only blessed
        //     projection (EM6′).
        //   * No `&mut EntityMaster` is used while the counter lives: the
        //     counter is dropped with its `Commands<'s>` at system-body end,
        //     and W0 keeps every `&mut` over the world, dispatcher's or
        //     worker's, out of the phase (SCH7, EM2′-K).
        unsafe { EntityCounter::from_ptr(reservoir) }
    }

    /// Aether v2 KE2: mints an [`Entities<'s>`] projecting only the entity fast
    /// store (`EntityMaster::entities_inland`). The returned param cannot reach
    /// any other `EntityMaster` field — the EM6 aliasing rule is type-enforced
    /// (the carried pointer's type is `*const InlandStore`, not
    /// `*const EntityMaster`), exactly as [`entity_counter`] does with
    /// `*const EntityReservoir`.
    ///
    /// # Safety (U_C2, EM6, SCH7)
    ///
    /// * The caller asserts that the active `SystemParam::init_access` permits
    ///   a shared read of the entity fast store. `Entities` declares no access
    ///   in the conflict graph (there is no entity-store axis to declare on);
    ///   soundness rests on SCH7 — structural mutation of the store takes
    ///   `&mut EntityMaster` and runs dispatcher-solo inside the apply window,
    ///   so it cannot overlap a live system body. This is the same argument
    ///   `EntityMaster`'s own `unsafe impl Sync` (SEND5) already makes for the
    ///   `&self` reads of this field on the `get_component_raw` path.
    /// * The by-value receiver preserves the raw pointer's provenance: no
    ///   `&self` retag downgrades the carried `*mut EcsMaster` before the field
    ///   projection.
    /// * `'s <= 'w` by the caller's PhantomData re-tag (the SystemParam
    ///   protocol enforces this on the consumer side).
    ///
    /// [`Entities<'s>`]: super::params::entities::Entities
    /// [`entity_counter`]: UnsafeEcsCell::entity_counter
    #[inline]
    pub(crate) unsafe fn entity_inland_store<'s>(self) -> Entities<'s> {
        // SAFETY (U_C2, EM6):
        //   * By-value receiver — no `&self` retag. The underlying
        //     `*mut EcsMaster` is valid for `'w` and carries the provenance
        //     minted by `new_mutable` / `new_readonly`.
        //   * The `&raw const` projection takes the address of the
        //     `entity_master.entities_inland` field without materialising an
        //     intermediate `&EcsMaster`, so the pointer is not
        //     SharedReadOnly-downgraded.
        //   * The destination type is `InlandStore` — no compile-time path
        //     leads from the carried pointer to any other `EntityMaster` field,
        //     type-enforcing EM6.
        let store_ptr = unsafe { &raw const (*self.ptr).entity_master.entities_inland };
        // SAFETY (`Entities::from_store_ptr` contract):
        //   * The pointer was just minted from a live `EntityMaster` reachable
        //     through `self.ptr`, valid for `'w >= 's`.
        //   * It aims at the `entities_inland` field — the only blessed
        //     projection for this param.
        //   * The no-structural-mutation clause is SCH7, argued above.
        unsafe { Entities::from_store_ptr(store_ptr) }
    }

    /// Write-capable pointer to resource `id`'s value, or `None` if it is
    /// absent. Hot path for [`ResMut<R>::get_param`]. Deliberately not a
    /// `&mut Resources` (PC-24 / W0): that retag would alias the `Resources`
    /// view of any other worker running a `Res` / `ResMut` of another id.
    ///
    /// # Safety (U_C3)
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   declared a write of resource `id` that does not conflict with
    ///   sibling params or other systems; the `&mut R` it mints from the
    ///   returned pointer is exclusive by that declaration.
    /// * The cell was minted via `new_mutable` (debug-asserted).
    /// * The by-value receiver consumes a `Copy` of the cell — no `&self`
    ///   retag occurs.
    ///
    /// [`ResMut<R>::get_param`]: super::params::resmut::ResMut
    /// [`new_mutable`]: UnsafeEcsCell::new_mutable
    #[inline]
    pub(crate) unsafe fn resource_ptr_mut(self, id: ResourceId) -> Option<*mut u8> {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.allows_mutable_access,
            "invariant U_C3: resource_ptr_mut() called on a read-only UnsafeEcsCell \
             minted via new_readonly"
        );
        // SAFETY (U_C3, W0 — PC-24):
        //   - `resources()` projects the field without forming a world
        //     reference. Its shared retag reads only the field's frozen bytes
        //     (the slab `Box` pointer and `registered_mask`), which nothing
        //     writes during a phase: `insert` / `remove` take `&mut EcsMaster`
        //     in apply windows (SCH7, W0).
        //   - A `&mut Resources` here would be a retag write under Stacked
        //     Borrows and would alias the `Resources` view of any other worker
        //     running a `Res` / `ResMut` of another id, which the conflict
        //     graph allows.
        //   - `get_ptr_by_id` returns the slot's stored `*mut u8`
        //     (`Box::into_raw` of the value) by value, so the pointer keeps the
        //     value's own allocation's provenance; `cast_mut` restores the type
        //     only. It stays valid for `'w`: the value is replaced or removed
        //     only under `&mut EcsMaster`.
        //   - Exclusivity of the `&mut R` the caller mints is the declared
        //     resource write (SP1 / SP2), unchanged.
        unsafe { self.resources() }.get_ptr_by_id(id).map(<*const u8>::cast_mut)
    }

    /// Direct read-only access to the **non-`Send`** resource slab, or `None`
    /// if it was never materialised (Phase 4 Seam 2). Hot path for
    /// [`NonSendRes<R>::get_param`].
    ///
    /// # Safety (U_C2 + CR-A — the apply-window single-thread-touch invariant)
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   declared the NonSend access (universal — CR-B), so the system
    ///   resolves `SystemKind::CpuExclusive` and runs ONLY on the dispatcher
    ///   thread inside the apply window (`running == 0`). At that point no
    ///   worker is live, so the `!Send` payload reachable through the
    ///   returned slab is touched single-threaded on its owning thread — the
    ///   external-synchronisation contract `!Send` types need.
    /// * The by-value receiver consumes a `Copy` of the cell — no `&self`
    ///   retag occurs; the raw pointer's provenance is preserved.
    ///
    /// [`NonSendRes<R>::get_param`]: super::params::nonsend_res::NonSendRes
    #[inline]
    pub(crate) unsafe fn nonsend_resources(self) -> Option<&'w NonSendResources> {
        // SAFETY (U_C2, CR-A): by-value receiver; the raw `*mut EcsMaster` is
        //   not retagged. The `&` projects through `*self.ptr` onto the lazy
        //   `nonsend_resources` field; `as_deref()` yields `Option<&'w
        //   NonSendResources>`. The single-thread-touch invariant above makes
        //   reading the `!Send` payload behind it sound. `'w` is upheld by
        //   `new_*()` postconditions.
        let slab = unsafe { (*self.ptr).nonsend_resources.as_deref() }?;
        // M2 (Phase 5 Option C): tripwire a projection off the owning thread.
        slab.debug_assert_owning_thread();
        Some(slab)
    }

    /// Direct mutable access to the **non-`Send`** resource slab, or `None`
    /// if it was never materialised (Phase 4 Seam 2). Hot path for
    /// [`NonSendResMut<R>::get_param`].
    ///
    /// # Safety (U_C3 + CR-A)
    /// * Same single-thread-touch invariant as [`nonsend_resources`]: the
    ///   NonSend system is `CpuExclusive`, so this runs on the dispatcher when
    ///   `running == 0` — no worker aliases the `&mut`.
    /// * The cell was minted via `new_mutable` (debug-asserted).
    /// * By-value receiver — no `&self` retag.
    ///
    /// [`nonsend_resources`]: UnsafeEcsCell::nonsend_resources
    /// [`NonSendResMut<R>::get_param`]: super::params::nonsend_resmut::NonSendResMut
    /// [`new_mutable`]: UnsafeEcsCell::new_mutable
    #[inline]
    pub(crate) unsafe fn nonsend_resources_mut(self) -> Option<&'w mut NonSendResources> {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.allows_mutable_access,
            "invariant U_C3: nonsend_resources_mut() called on a read-only \
             UnsafeEcsCell minted via new_readonly"
        );
        // SAFETY (U_C3, CR-A): by-value receiver; the raw pointer carries
        //   write-capable provenance (minted from `&mut EcsMaster`). The
        //   `&mut` projects through `*self.ptr` onto the `nonsend_resources`
        //   field; `as_deref_mut()` yields `Option<&'w mut NonSendResources>`.
        //   The CpuExclusive dispatcher-solo invariant guarantees no aliasing
        //   worker cell. Aliasing is the caller's responsibility per the
        //   SystemParam protocol.
        let slab = unsafe { (*self.ptr).nonsend_resources.as_deref_mut() }?;
        // M2 (Phase 5 Option C): tripwire a projection off the owning thread.
        slab.debug_assert_owning_thread();
        Some(slab)
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
//     aliasing — no two concurrent systems hold overlapping `&mut` views of
//     component columns or resource values through their cell copies; no
//     worker holds a `&mut` view of the world at all, W0; shared views of
//     the world overlap freely).
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

    // ════════════════════════════════════════════════════════════════════════
    // PC-24 world-retag matrix (T1–T7): two systems on two threads over ONE
    // write-capable cell.
    // ════════════════════════════════════════════════════════════════════════
    //
    // This is what `Schedule::try_dispatch_ready` does with a round's ready set
    // (`schedule.rs`, the `scope.spawn` loop): one cell minted from the
    // dispatcher's `&mut EcsMaster`, a `Copy` of it moved into each worker task,
    // each body run under an `InSystemRunGuard`. The pool is left out on
    // purpose: crossbeam-epoch's own Stacked Borrows violation ends a
    // pool-backed SB run inside the first `schedule.run`, before any kernel
    // system executes, so only a pool-free harness gives this matrix a usable
    // Stacked Borrows leg.
    //
    // Every pair is conflict-free in the declared access graph (asserted by the
    // harness), so the scheduler is free to run it in one round. Natively the
    // race is not observable; the matrix is a Miri gate, run on BOTH models
    // with `MIRIFLAGS` spelled in full (`.cargo/config.toml`'s `[env]` default
    // is Tree Borrows, so an unset `MIRIFLAGS` is never a Stacked Borrows run):
    //
    //   MIRIFLAGS="-Zmiri-strict-provenance" \
    //     cargo +nightly-x86_64-pc-windows-msvc miri test --locked -p boyko-ecs --lib -- pc24_
    //   MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-strict-provenance" \
    //     cargo +nightly-x86_64-pc-windows-msvc miri test --locked -p boyko-ecs --lib -- pc24_
    //
    // A worker-side `&mut` over the world (or over `Resources`) covers bytes
    // another worker touches: Tree Borrows reports its retag as a read, which
    // races the reservoir's atomic RMWs (T1); Stacked Borrows reports it as a
    // write, which races every other worker's retag as well (T1–T6). T7 is the
    // control: shared retags beside the reservoir RMWs are race-free on both.

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Once, OnceLock};

    use boyko_threadpool::InSystemRunGuard;

    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::entity::entity::Entity;
    use crate::ecs::core::iters::query::query::Query;
    use crate::ecs::core::resources::resource::Resource;
    use crate::ecs::core::resources::resource_registry::register_new;
    use crate::ecs::core::system::into_system::IntoSystem;
    use crate::ecs::core::system::params::commands::Commands;
    use crate::ecs::core::system::params::res::Res;
    use crate::ecs::core::system::params::resmut::ResMut;
    use crate::ecs::core::system::system::System;
    use crate::ecs::identifiers::primitives::{ComponentId, ResourceId};

    /// Hand-picked ids, grep-verified free across `src/` and `tests/` (no
    /// `387`–`389` literal anywhere in the crate). Hand-picked rather than
    /// minted for the reason `check_ticks.rs`'s dense fixtures give:
    /// `boyko_macros` is a dev-dependency, so the derive is unavailable in
    /// `src/`, and a verified-free high number fails visibly.
    const PC24_P_ID: ComponentId = ComponentId(387);
    const PC24_V_ID: ComponentId = ComponentId(388);
    const PC24_Q_ID: ComponentId = ComponentId(389);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Pc24P(u32);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Pc24V(u32);

    /// Shares the `{P, Q}` archetype with `Pc24P` (T3).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Pc24Q(u32);

    impl Component for Pc24P {
        fn component_id() -> ComponentId {
            PC24_P_ID
        }
    }

    impl Component for Pc24V {
        fn component_id() -> ComponentId {
            PC24_V_ID
        }
    }

    impl Component for Pc24Q {
        fn component_id() -> ComponentId {
            PC24_Q_ID
        }
    }

    struct Pc24R1(u64);
    struct Pc24R2(u64);

    impl Resource for Pc24R1 {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    impl Resource for Pc24R2 {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    /// `Pc24P` over `{P}` (0..4, summing to 6) and `{P, Q}` (10..14).
    const PC24_P_SUM0: u32 = 6 + (10 + 11 + 12 + 13);
    /// `Pc24V` over `{V}` (30..34).
    const PC24_V_SUM0: u32 = 30 + 31 + 32 + 33;
    /// `Pc24Q` over `{P, Q}` (20..24).
    const PC24_Q_SUM0: u32 = 20 + 21 + 22 + 23;
    /// `Pc24R2`'s seed, distinct from `Pc24R1`'s so a crossed read shows.
    const PC24_R2_SEED: u64 = 7;
    /// Entities the fixture spawns: four per archetype, three archetypes.
    const PC24_ENTITIES: usize = 12;

    /// Reinterprets a single-`u32` `#[repr(C)]` fixture as the byte blob the
    /// direct `create_entity` API takes.
    fn pc24_bytes<T: Copy>(value: &T) -> &[u8] {
        // SAFETY: every caller passes a `#[repr(C)]` struct holding one `u32`,
        //   so all `size_of::<T>()` bytes are initialised (no padding) and any
        //   bit pattern is a valid `u8`. The slice borrows `value`, so the
        //   bytes cannot move or drop while it is live, and it is read-only.
        unsafe {
            std::slice::from_raw_parts(std::ptr::from_ref(value).cast::<u8>(), size_of::<T>())
        }
    }

    /// Archetypes `{P}`, `{V}` and `{P, Q}` with four entities each, plus both
    /// resources (`Pc24R1 = 0`, `Pc24R2 = PC24_R2_SEED`).
    fn pc24_world() -> EcsMaster {
        static REGISTERED: Once = Once::new();
        REGISTERED.call_once(|| {
            component_registry::register_layout::<Pc24P>(PC24_P_ID.0);
            component_registry::register_layout::<Pc24V>(PC24_V_ID.0);
            component_registry::register_layout::<Pc24Q>(PC24_Q_ID.0);
        });
        let mut world = EcsMaster::new();
        let arch_p = world.create_archetype(&[PC24_P_ID]);
        let arch_v = world.create_archetype(&[PC24_V_ID]);
        let arch_pq = world.create_archetype(&[PC24_P_ID, PC24_Q_ID]);
        for i in 0..4u32 {
            let p = Pc24P(i);
            world
                .create_entity(arch_p, &[(PC24_P_ID, pc24_bytes(&p))])
                .expect("invariant: {P} was just created and P is registered");
            let v = Pc24V(30 + i);
            world
                .create_entity(arch_v, &[(PC24_V_ID, pc24_bytes(&v))])
                .expect("invariant: {V} was just created and V is registered");
            let (p, q) = (Pc24P(10 + i), Pc24Q(20 + i));
            world
                .create_entity(
                    arch_pq,
                    &[(PC24_P_ID, pc24_bytes(&p)), (PC24_Q_ID, pc24_bytes(&q))],
                )
                .expect("invariant: {P, Q} was just created and both ids are registered");
        }
        world.insert_resource(Pc24R1(0));
        world.insert_resource(Pc24R2(PC24_R2_SEED));
        assert_eq!(world.entity_count(), PC24_ENTITIES, "fixture precondition");
        world
    }

    /// Builds a one-shot system from a closure; `Out` is inferred from the body.
    fn pc24_system<F, M, Out>(body: F) -> F::System
    where
        F: IntoSystem<(), Out, M>,
    {
        F::into_system(body)
    }

    /// Start rendezvous: returns once both threads of a pair have arrived.
    ///
    /// Without it one body can run to completion, thread exit included, before
    /// the other thread's first allocation. Miri models an allocator's
    /// cross-thread address reuse as a happens-before edge
    /// (`-Zmiri-address-reuse-cross-thread-rate`, default 0.1), so the late
    /// thread's TLS allocation can then order the two bodies and hide the race.
    /// Measured on the trunk text: T3 went GREEN under Stacked Borrows as the
    /// first test of the full `pc24_` run, and RED alone and with that rate set
    /// to 0. Arriving after `InSystemRunGuard::enter` puts each thread's TLS
    /// allocation before the rendezvous, and a body forms its first world
    /// reference before it allocates anything. The spin yields, which is also
    /// what lets Miri switch threads.
    fn pc24_rendezvous(arrived: &AtomicUsize) {
        arrived.fetch_add(1, Ordering::AcqRel);
        while arrived.load(Ordering::Acquire) < 2 {
            std::thread::yield_now();
        }
    }

    /// Runs `a` and `b` concurrently on two threads over copies of one
    /// write-capable cell, then applies both on this thread, in that order.
    ///
    /// Mirrors one scheduler round: `initialize` at build, the pair checked
    /// conflict-free (the only condition under which the scheduler co-runs
    /// two systems), one cell minted from `&mut world`, one `Copy` per worker
    /// under an `InSystemRunGuard`, the joins, then the apply window.
    fn pc24_run_pair<A, B>(world: &mut EcsMaster, mut a: A, mut b: B) -> (A::Out, B::Out)
    where
        A: System,
        B: System,
        A::Out: Send,
        B::Out: Send,
    {
        a.initialize(world);
        b.initialize(world);
        assert!(
            !a.access().conflicts_with(b.access()),
            "harness precondition: the pair must be conflict-free, or no schedule \
             would co-run it ({} vs {})",
            a.name(),
            b.name(),
        );
        let (sys_a, sys_b) = (&mut a, &mut b);
        let arrived = AtomicUsize::new(0);
        let arrived = &arrived;
        // SAFETY (U_C1): `cell` is used only inside the scope below, whose
        //   threads are all joined before the scope returns; `world` is not
        //   touched again until then, so the cell never outlives the
        //   `&mut world` borrow it was minted from.
        let cell = unsafe { UnsafeEcsCell::new_mutable(world) };
        let outs = std::thread::scope(|s| {
            let run_a = s.spawn(move || {
                let _in_body = InSystemRunGuard::enter();
                pc24_rendezvous(arrived);
                // SAFETY (S1 as the scheduler discharges it, SCH3): the two
                //   systems' declared accesses are disjoint (asserted above),
                //   which is exactly when `Schedule` runs two bodies over
                //   copies of one cell in the same round; both were
                //   initialised on `world`, and `world` is not reborrowed
                //   until both threads have joined.
                unsafe { sys_a.run_unsafe(cell) }
            });
            let run_b = s.spawn(move || {
                let _in_body = InSystemRunGuard::enter();
                pc24_rendezvous(arrived);
                // SAFETY (S1 as the scheduler discharges it, SCH3): as above.
                unsafe { sys_b.run_unsafe(cell) }
            });
            (
                run_a.join().expect("system A must not panic"),
                run_b.join().expect("system B must not panic"),
            )
        });
        a.apply(world);
        b.apply(world);
        outs
    }

    fn pc24_sum_p(world: &mut EcsMaster) -> u32 {
        world.run_closure_once(|q: Query<&Pc24P>| q.iter().map(|c| c.0).sum::<u32>())
    }

    fn pc24_sum_v(world: &mut EcsMaster) -> u32 {
        world.run_closure_once(|q: Query<&Pc24V>| q.iter().map(|c| c.0).sum::<u32>())
    }

    fn pc24_sum_q(world: &mut EcsMaster) -> u32 {
        world.run_closure_once(|q: Query<&Pc24Q>| q.iter().map(|c| c.0).sum::<u32>())
    }

    /// T1 — the measured PC-24 pair: a `Commands` spawner (a reservoir RMW
    /// through `EntityCounter`) beside a `Query<&mut P>` over archetypes the
    /// spawn does not touch.
    #[test]
    fn pc24_spawn_vs_mut_query() {
        let mut world = pc24_world();
        let spawner = pc24_system(|mut c: Commands| c.spawn_empty().id());
        let writer = pc24_system(|mut q: Query<&mut Pc24P>| {
            for p in q.iter_mut() {
                p.0 += 1;
            }
        });
        let (spawned, ()): (Entity, ()) = pc24_run_pair(&mut world, spawner, writer);
        assert!(world.has_entity(spawned), "the spawn must materialise at apply");
        assert_eq!(world.entity_count(), PC24_ENTITIES + 1);
        assert_eq!(pc24_sum_p(&mut world), PC24_P_SUM0 + 8, "every P incremented once");
    }

    /// T2 — two mutable queries over disjoint components and archetypes.
    #[test]
    fn pc24_two_mut_queries() {
        let mut world = pc24_world();
        let p_writer = pc24_system(|mut q: Query<&mut Pc24P>| {
            for p in q.iter_mut() {
                p.0 += 1;
            }
        });
        let v_writer = pc24_system(|mut q: Query<&mut Pc24V>| {
            for v in q.iter_mut() {
                v.0 += 1;
            }
        });
        pc24_run_pair(&mut world, p_writer, v_writer);
        assert_eq!(pc24_sum_p(&mut world), PC24_P_SUM0 + 8);
        assert_eq!(pc24_sum_v(&mut world), PC24_V_SUM0 + 4);
    }

    /// T3 — two mutable queries over disjoint columns of ONE archetype
    /// (`{P, Q}`). Besides the world retag it probes the next allocation over:
    /// both workers form shared `&Archetype` views of the same slab slot, so a
    /// red in an `Archetype`-slot frame here is a new finding, not PC-24.
    #[test]
    fn pc24_mut_queries_one_archetype() {
        let mut world = pc24_world();
        let p_writer = pc24_system(|mut q: Query<&mut Pc24P>| {
            for p in q.iter_mut() {
                p.0 += 1;
            }
        });
        let q_writer = pc24_system(|mut q: Query<&mut Pc24Q>| {
            for c in q.iter_mut() {
                c.0 += 1;
            }
        });
        pc24_run_pair(&mut world, p_writer, q_writer);
        assert_eq!(pc24_sum_p(&mut world), PC24_P_SUM0 + 8);
        assert_eq!(pc24_sum_q(&mut world), PC24_Q_SUM0 + 4);
    }

    /// T4 — a mutable query beside a shared one (`miri_schedule_parallel`'s
    /// pair, without the pool).
    #[test]
    fn pc24_mut_query_vs_shared_query() {
        let mut world = pc24_world();
        let p_writer = pc24_system(|mut q: Query<&mut Pc24P>| {
            for p in q.iter_mut() {
                p.0 += 1;
            }
        });
        let v_reader = pc24_system(|q: Query<&Pc24V>| q.iter().map(|v| v.0).sum::<u32>());
        let ((), v_seen): ((), u32) = pc24_run_pair(&mut world, p_writer, v_reader);
        assert_eq!(v_seen, PC24_V_SUM0, "the reader sees V untouched");
        assert_eq!(pc24_sum_p(&mut world), PC24_P_SUM0 + 8);
    }

    /// T5 — two `ResMut`s of different resources.
    #[test]
    fn pc24_resmut_vs_resmut() {
        let mut world = pc24_world();
        let r1_writer = pc24_system(|mut r: ResMut<Pc24R1>| (*r).0 += 1);
        let r2_writer = pc24_system(|mut r: ResMut<Pc24R2>| (*r).0 += 1);
        pc24_run_pair(&mut world, r1_writer, r2_writer);
        assert_eq!(world.resource::<Pc24R1>().0, 1);
        assert_eq!(world.resource::<Pc24R2>().0, PC24_R2_SEED + 1);
    }

    /// T6 — a `ResMut` beside a `Res` of a different resource.
    #[test]
    fn pc24_resmut_vs_res() {
        let mut world = pc24_world();
        let r1_writer = pc24_system(|mut r: ResMut<Pc24R1>| (*r).0 += 1);
        let r2_reader = pc24_system(|r: Res<Pc24R2>| (*r).0);
        let ((), r2_seen): ((), u64) = pc24_run_pair(&mut world, r1_writer, r2_reader);
        assert_eq!(r2_seen, PC24_R2_SEED);
        assert_eq!(world.resource::<Pc24R1>().0, 1);
    }

    /// T7 — the CONTROL: a `Commands` spawner beside a shared query. Shared
    /// world retags read only frozen bytes, so they tolerate the reservoir
    /// RMWs on both models; a red here is a harness red, not PC-24.
    #[test]
    fn pc24_spawn_vs_shared_query() {
        let mut world = pc24_world();
        let spawner = pc24_system(|mut c: Commands| c.spawn_empty().id());
        let v_reader = pc24_system(|q: Query<&Pc24V>| q.iter().map(|v| v.0).sum::<u32>());
        let (spawned, v_seen): (Entity, u32) = pc24_run_pair(&mut world, spawner, v_reader);
        assert_eq!(v_seen, PC24_V_SUM0);
        assert!(world.has_entity(spawned), "the spawn must materialise at apply");
        assert_eq!(world.entity_count(), PC24_ENTITIES + 1);
    }
}
