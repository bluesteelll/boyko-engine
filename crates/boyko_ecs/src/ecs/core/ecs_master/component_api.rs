//! Component data surface on [`EcsMaster`] (mechanical split).
//!
//! Typed / raw component get / set / has, change-tick reads, and the dense
//! (non-fragmenting) storage routing. Extracted verbatim from `ecs_master.rs`.
use std::cell::UnsafeCell;
use std::ptr::NonNull;

use crate::ecs::core::archetype::archetype::Column;
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use crate::ecs::core::component::hooks::dispatch::{
    trigger_on_add, trigger_on_insert, trigger_on_remove, trigger_on_replace,
};
use crate::ecs::core::component::observers::dispatch::{
    fire_on_add_observers, fire_on_insert_observers,
    fire_on_remove_observers, fire_on_replace_observers,
};
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::core::iters::query::data::Mut;
use crate::ecs::identifiers::primitives::{
    ArchetypeId, ComponentId,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    // ── Dense (non-fragmenting) storage routing (Dense plan D2) ──────────────
    //
    // The SINGLE implementation every structural site routes a dense component
    // through (Commands spawn/insert/remove + the direct create_entity* API +
    // clone-materialize + despawn). A dense component is NOT in any archetype
    // signature, so the per-archetype `ArchetypeFlags` gate does NOT cover it —
    // these helpers fire by reading the per-component hook table / observer
    // registry directly. Both `trigger_on_*` and `fire_*_observers` SELF-GATE
    // (no-op when the component has no hook / no observer registered), so calling
    // them unconditionally per dense id is correct and costs one cold table read
    // when nothing is installed.
    //
    // 0%-gate: a table-only world never has a dense id reach here (the callers all
    // branch on `storage_kind == Dense`, and the despawn walk is gated by
    // `dense_registry.is_empty()`), so this whole cluster is dead on the
    // table-only path.

    /// Returns `true` iff `entity` is a member of the `component_id` dense store
    /// (Dense plan D2 read accessor). `false` if the component is not dense, no
    /// store exists yet, or the entity is not a member. A read-only membership
    /// oracle (D3 will build the typed query path on top of the same `e2s`).
    #[inline]
    pub fn dense_contains(&self, entity: Entity, component_id: ComponentId) -> bool {
        self.dense_registry
            .store(component_id)
            .is_some_and(|s| s.contains(entity.id()))
    }

    /// Returns the slot `entity` occupies in the `component_id` dense store, or
    /// `None` if it is not a member (Dense plan D2 read accessor).
    #[inline]
    pub fn dense_slot_of(&self, entity: Entity, component_id: ComponentId) -> Option<u32> {
        self.dense_registry
            .store(component_id)
            .and_then(|s| s.slot_of(entity.id()))
    }

    /// Reads `entity`'s `component_id` dense value as raw bytes, or `None` if it
    /// is not a member (Dense plan D2 read accessor). The pointer is valid for the
    /// component's stride; the caller casts it to the registered type.
    ///
    /// # Safety
    /// The returned pointer borrows the dense column for `&self`; it must not be
    /// read across a structural mutation of the same store. The cast type must
    /// match the store's registered component type.
    #[inline]
    pub fn dense_get_raw(&self, entity: Entity, component_id: ComponentId) -> Option<*const u8> {
        let store = self.dense_registry.store(component_id)?;
        let slot = store.slot_of(entity.id())?;
        let view = store.solve_view();
        // SAFETY: `slot` came from `slot_of`, so it is a LIVE slot (`< len`,
        //   live-bit set) — `row_ptr`'s contract holds. The pointer is valid for
        //   the store's stride; the `&self` borrow keeps the column alive.
        Some(unsafe { view.row_ptr(slot as usize) as *const u8 })
    }

    /// Inserts `bytes` for `entity` into the `component_id` dense store (creating
    /// the store lazily), marks `archetype_id` present in the store's
    /// `arch_presence` seed, then fires dense `on_add` + `on_insert` (hooks first,
    /// then observers) for the component.
    ///
    /// `archetype_id` is the entity's CURRENT archetype (the dense insert does NOT
    /// migrate it). Used by the spawn paths (`SpawnAtCommand` / `create_entity*`)
    /// and the dense subset of `InsertCommand`.
    pub(crate) fn dense_insert_and_fire(
        &mut self,
        entity: Entity,
        archetype_id: ArchetypeId,
        component_id: ComponentId,
        bytes: &[u8],
    ) {
        let current_tick = self.current_tick();
        {
            let store = self.dense_registry.store_mut(component_id);
            store.insert(entity.id(), bytes, current_tick);
            store.mark_arch_present(archetype_id);
            // <-- the `&mut DenseStore` borrow of `self.dense_registry` ends here,
            // BEFORE `world_ptr` is minted (no `self`-derived `&mut` is live at
            // the fire, mirroring the archetypal SAFETY-1 discipline).
        }
        // MINT: no `self`-derived `&mut` into storage is live (the store borrow
        // above dropped at the block close).
        let world_ptr = NonNull::from(&mut *self);
        // on_add THEN on_insert (Bevy add-before-insert ordering). Hooks first,
        // then observers, per component (both self-gate to a no-op when nothing
        // is registered).
        trigger_on_add(world_ptr, component_id, entity);
        fire_on_add_observers(world_ptr, component_id, entity);
        trigger_on_insert(world_ptr, component_id, entity);
        fire_on_insert_observers(world_ptr, component_id, entity);
    }

    /// Removes `entity`'s `component_id` dense membership (tombstone), firing
    /// dense `on_replace` + `on_remove` (hooks first, then observers) PRE-tombstone
    /// so the handler reads the dying value. Returns `true` iff the entity was
    /// present in the store. No archetype migration (the dense payoff).
    ///
    /// A no-op (returns `false`) if no store exists for `component_id` yet or the
    /// entity is not a member — matching the table remove's absent-component
    /// silent no-op (W1 / Bevy #10166).
    pub(crate) fn dense_remove_and_fire(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
    ) -> bool {
        // Presence probe without creating a store (remove of an untouched dense id
        // is a no-op, never a lazy store creation).
        let present = self
            .dense_registry
            .store(component_id)
            .is_some_and(|s| s.contains(entity.id()));
        if !present {
            return false;
        }
        // PRE-tombstone fire (Q7 ordering): on_replace then on_remove, reading the
        // still-live dying value. No `self`-derived `&mut` into storage is live.
        let world_ptr = NonNull::from(&mut *self);
        trigger_on_replace(world_ptr, component_id, entity);
        fire_on_replace_observers(world_ptr, component_id, entity);
        trigger_on_remove(world_ptr, component_id, entity);
        fire_on_remove_observers(world_ptr, component_id, entity);
        // Tombstone AFTER the fire (the value was live for the handlers).
        let removed = self
            .dense_registry
            .store_existing_mut(component_id)
            .expect("invariant: store existed at the presence probe above")
            .remove(entity.id());
        debug_assert!(removed, "dense_remove_and_fire: presence probe / remove disagree");
        removed
    }

    /// Fast random access read: 3-4 cache lines, ~12-16 ns target.
    ///
    /// Lookup sequence:
    ///   1. `entity_master.entities_inland[entity.id().0]` — 1 line.
    ///   2. Null check + generation check (both fields in the same line as 1).
    ///   3. `(*archetype_ptr).columns[component_id.0]` — 1 line (`columns` at
    ///      offset 0; for `ComponentId.0 < 4` shares the line with the
    ///      archetype deref).
    ///   4. `column.ptr.add(unit_index * stride)` — arithmetic on the
    ///      cached pointer; final line is the component itself.
    ///
    /// Returns `None` for stale entities (generation mismatch), missing
    /// components (column is null), or never-registered entities
    /// (archetype_ptr is null).
    #[inline]
    pub fn get_component_raw(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> Option<*const u8> {
        // Line 1: entity_master.entities_inland[entity.id().0]
        let inland = self.entity_master.entities_inland.get(entity.id().0)?;
        // Null check (dead slot) + generation check (stale handle).
        // Order chosen so the null check covers never-registered IDs first.
        if inland.is_null() {
            return None;
        }
        if inland.generation() != entity.generation() {
            return None;
        }
        let archetype_ptr = inland.archetype_ptr();
        debug_assert!(component_id.0 < MAX_COMPONENTS);

        // BUG-MIGRATE-TB-1 (Tree Borrows): do NOT form `&*archetype_ptr` here.
        // A `&Archetype` covers the WHOLE struct (incl. `current_index`); a
        // sibling structural migration writes `current_index` through a
        // same-cell-derived pointer, transitioning the interior-mutable slab
        // cell to Active. This shared (foreign) read would then FREEZE that
        // cell — and the `Box`-of-slab deallocation on `EcsMaster` drop is
        // forbidden through a `Frozen` tag (alloc/boxed.rs). The F4 read
        // discipline is: read the single `Column` we need through a raw-pointer
        // PROJECTION (`addr_of!((*p).columns)`), never a struct-wide reference.
        // `Column` is `Copy`, so we read it by value.
        //
        // SAFETY (U1, U2, U4, U11, F1): `archetype_ptr` was minted via the
        //   bundle's `UnsafeCell::raw_get` helper (Step 4 + F4); the slab heap
        //   address is stable for the EcsMaster's lifetime, and the pointer is
        //   interior-mutable (`SharedReadWrite`, F4-rooted) so it survives
        //   sibling structural writes (e.g. a later spawn's / migration's
        //   `current_index` bump) under TB/SB — the whole slab element is
        //   `UnsafeCell`-wrapped, and projecting `columns` (offset 0) reads only
        //   the live lookup table, never freezing the cell. `&self` gives
        //   shared access to the slab; `component_id.0 < MAX_COMPONENTS` (asserted)
        //   keeps the `[Column; MAX_COMPONENTS]` index in bounds (U4).
        let column = unsafe {
            let columns_ptr = core::ptr::addr_of!((*archetype_ptr).columns).cast::<Column>();
            *columns_ptr.add(component_id.0)
        };
        if column.ptr.is_null() {
            return None;
        }

        // SAFETY (U5, U6, U10):
        //   - U5: column.ptr / stride are set by refresh_column after add_pool.
        //   - U6: pool buffer pointer is write-once at add_pool (Phase 7 D5
        //     audit table).
        //   - U10: unit_index < archetype.current_index for any alive
        //     entity; multiplication fits because `stride * MAX_ENTITIES`
        //     ≤ pool buffer size, and `unit_index < MAX_ENTITIES`.
        Some(unsafe {
            column.ptr.add(inland.unit_index() as usize * column.stride as usize) as *const u8
        })
    }

    /// Mutable fast random access. `EntityInland` is `Copy`; we copy
    /// 16 B to drop the `EntityMaster` borrow before reborrowing the slab
    /// pointer as `&mut Archetype` (W4 / U14).
    #[inline]
    pub fn get_component_raw_mut(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
    ) -> Option<*mut u8> {
        // Copy the inland by value to release the entity_master borrow.
        let inland: EntityInland = *self.entity_master.entities_inland
            .get(entity.id().0)?;
        if inland.is_null() {
            return None;
        }
        if inland.generation() != entity.generation() {
            return None;
        }
        debug_assert!(component_id.0 < MAX_COMPONENTS);

        let archetype_ptr = inland.archetype_ptr();

        // BUG-MIGRATE-TB-1 (Tree Borrows): do NOT form `&mut *archetype_ptr`
        // here — a struct-wide `&mut Archetype` covers `current_index` and would
        // narrow the interior-mutable slab cell to `Unique`, which a later
        // sibling read can freeze (see `get_component_raw`). Read the single
        // `Column` we need through a raw-pointer PROJECTION of `columns`
        // (offset 0); `Column` is `Copy`.
        //
        // SAFETY (U1, U2, U4, U11, U14, F1):
        //   - U14: archetype_ptr is write-capable provenance (minted via the
        //     bundle's `UnsafeCell::raw_get` helper during create_entity);
        //     single-threaded &mut self gives exclusive access; no other
        //     live borrow into the slot exists.
        //   - F1: interior-mutable (`SharedReadWrite`, F4-rooted) — survives
        //     sibling structural writes under TB/SB (whole slab element is
        //     `UnsafeCell`-wrapped); projecting `columns` reads only the lookup
        //     table, never narrowing/freezing the cell.
        //   - U4: `component_id.0 < MAX_COMPONENTS` (asserted) keeps the
        //     `[Column; MAX_COMPONENTS]` index in bounds.
        let column = unsafe {
            let columns_ptr = core::ptr::addr_of!((*archetype_ptr).columns).cast::<Column>();
            *columns_ptr.add(component_id.0)
        };
        if column.ptr.is_null() {
            return None;
        }

        // SAFETY (U5, U6, U10): same as get_component_raw plus
        //   &mut self exclusivity ⇒ the returned *mut points to a uniquely
        //   accessible byte range.
        Some(unsafe {
            column.ptr.add(inland.unit_index() as usize * column.stride as usize)
        })
    }

    /// Returns the stored `changed_tick` of `entity`'s `component_id` column row,
    /// or `None` if the entity is dead/stale or its archetype does not host the
    /// component (GUI P4 Decision 5).
    ///
    /// **Read-only**: unlike [`get_component_mut`](Self::get_component_mut)'s
    /// `Mut<T>` (whose `DerefMut` bumps the row's `changed_tick`), this never
    /// mutates any change-detection state. It is the entity-keyed read-with-tick
    /// primitive the change-gated UI data-bind path uses to compare a source
    /// field's `changed_tick` against the bind system's `last_run` via
    /// [`Tick::is_newer_than`] — reading it must NOT mark the source dirty, or it
    /// would corrupt the very `Changed<Source>` signal the bind discovery reads.
    ///
    /// Reuses the [`get_component_raw`](Self::get_component_raw) prologue (null +
    /// generation check) and the same-crate `ComponentPool::read_changed_tick`.
    #[inline]
    pub fn get_component_changed_tick(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> Option<Tick> {
        // Same prologue as `get_component_raw`: resolve + null/generation check.
        let inland = self.entity_master.entities_inland.get(entity.id().0)?;
        if inland.is_null() {
            return None;
        }
        if inland.generation() != entity.generation() {
            return None;
        }
        debug_assert!(component_id.0 < MAX_COMPONENTS);
        let archetype_ptr = inland.archetype_ptr();
        // BUG-MIGRATE-TB-1 (Tree Borrows): do NOT form `&*archetype_ptr` here. A
        // struct-wide `&Archetype` covers `current_index`; a sibling structural
        // migration writes `current_index` through a same-cell-derived pointer
        // (transitioning the interior-mutable slab cell to Active), and a prior
        // shared read over the WHOLE cell would freeze it — then the `Box`-of-slab
        // dealloc on `EcsMaster` drop is forbidden through a `Frozen` tag. Project
        // the cold `component_pools` field (a sub-region that excludes
        // `current_index`) through a raw pointer instead, mirroring
        // `get_component_raw`'s `columns` projection — the uniform F4 read
        // discipline.
        //
        // SAFETY (U1, U2, U4, F1): `archetype_ptr` is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — non-null +
        //   generation-matched above ⇒ the slot is live; `&self` gives shared
        //   access. `addr_of!((*p).component_pools)` reads only the cold pool table
        //   (never `current_index`), so the shared `&ComponentPoolBundle` narrows
        //   nothing the sibling migration writes — no freeze of the slab cell.
        let pools = unsafe { &*core::ptr::addr_of!((*archetype_ptr).component_pools) };
        let pool = pools.get_pool(component_id)?;
        let row = inland.unit_index() as usize;
        if row >= pool.count() {
            return None;
        }
        // SAFETY: `row < pool.count() <= committed_rows` (checked above), so the
        //   tick slot lies in the committed prefix of the pool's `changed` tick
        //   sub-region; `&self` ⇒ at least shared access, no concurrent writer in
        //   the single-threaded direct-API context (Phase 9 SCH3).
        Some(unsafe { pool.read_changed_tick(row) })
    }

    /// Returns `true` iff ANY archetype hosting one of `ids` has a row whose
    /// `changed_tick` falls in the window `(last_run, this_run]` (GUI P4
    /// Decision 6 — the `.ui`-dynamic outer 0%-gate probe).
    ///
    /// **Read-only**: takes `&self`, mutates nothing. Bounded to the archetypes
    /// that actually host a bound id (typically 1–few), short-circuits on the
    /// first changed row, and on a still frame finds no changed column and
    /// returns `false` after scanning only the hosting archetypes' live rows.
    /// Reflection-free — keyed purely by `ComponentId`.
    //
    // BUG-MIGRATE-TB-1 note: this forms `&Archetype` via the existing
    // `iter_archetypes()` read API and reaches only the cold `component_pools`
    // field. The freeze hazard the per-entity `get_component_raw` projection
    // guards against (a shared whole-cell read freezing the interior-mutable slab
    // cell, then a sibling `current_index` write / slab `Box` dealloc tripping the
    // `Frozen` tag) DOES NOT APPLY here: this is a `&self` read invoked ONLY from
    // `ui_bind_discovery`, an EXCLUSIVE system holding `&mut EcsMaster`, so no
    // sibling structural migration and no slab dealloc can interleave with the
    // read — the `&Archetype` and its derived `&ComponentPoolBundle` are dropped
    // before control returns to the scheduler. `iter_archetypes()` is the same
    // sanctioned `&Archetype` read API Phase 10's check-ticks scan uses.
    pub fn any_changed_since(&self, ids: &[ComponentId], last_run: Tick, this_run: Tick) -> bool {
        for archetype in self.archetype_master().iter_archetypes() {
            for &id in ids {
                let Some(pool) = archetype.component_pools().get_pool(id) else {
                    continue;
                };
                let live = pool.count();
                for row in 0..live {
                    // SAFETY: `row < pool.count() <= committed_rows`, so the tick
                    //   slot is in the committed prefix of the `changed` tick
                    //   sub-region; `&self` ⇒ at least shared access (Phase 9
                    //   SCH3, single-threaded probe context).
                    let tick = unsafe { pool.read_changed_tick(row) };
                    if tick.is_newer_than(last_run, this_run) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Fast component write: `~15-18 ns` target. Returns `false`
    /// for stale entities, missing components, or never-registered entities.
    /// On success, byte-copies the provided slice into the component slot.
    ///
    /// `component_bytes.len()` must equal the pool's stride; mismatched
    /// sizes produce undefined behavior in release. Callers should obtain
    /// the slice from a properly-sized `&T` for the target component type
    /// (see `get_component_mut` typed wrappers).
    #[inline]
    pub fn set_component_raw(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        component_bytes: &[u8],
    ) -> bool {
        let Some(dst) = self.get_component_raw_mut(entity, component_id) else {
            return false;
        };
        // Stride is not re-queried here; the size invariant lives at the
        // caller boundary (typed wrappers downcast from `&T` with
        // `size_of::<T>()`). A debug-assertable stride check would require
        // threading the column reference back out of the inner lookup,
        // which defeats the fast-path goal. The pool layer carries the
        // ultimate size guarantee through `Layout`.
        // SAFETY (U5, U6, U10):
        //   - dst is a valid *mut u8 to a byte range of size `stride` for
        //     the target component (U5/U6 — column resolved through the
        //     same fast path as get_component_raw_mut).
        //   - The caller's slice is sized to match by API contract; typed
        //     wrappers enforce this via `size_of::<T>()`.
        //   - Single-threaded &mut self ⇒ no concurrent reader.
        //   - copy_nonoverlapping is sound because the slice and the pool
        //     buffer live in disjoint allocations (slice is a caller-stack
        //     view; the pool buffer lives in the pool's own reservation).
        unsafe {
            std::ptr::copy_nonoverlapping(component_bytes.as_ptr(), dst, component_bytes.len());
        }
        true
    }

    /// Typed read accessor. Returns a shared reference to the
    /// component of type `T` owned by `entity`, or `None` if the entity is
    /// stale, the archetype does not host `T`, or the entity was never
    /// registered.
    #[inline]
    pub fn get_component<T: crate::ecs::core::component::component::Component>(
        &self,
        entity: Entity,
    ) -> Option<&T> {
        let raw = self.get_component_raw(entity, T::component_id())?;
        // SAFETY: the pool was registered with T::component_id(), so the
        //   bytes at `raw` are a valid `T` (M-001 drop-fn / layout guarantee
        //   from the component registry). The lifetime of the returned
        //   reference is bounded by &self.
        Some(unsafe { &*(raw as *const T) })
    }

    /// Typed mutable accessor returning a change-detection-aware [`Mut<T>`]
    /// (Phase 14b W6).
    ///
    /// The direct-API counterpart of querying `Mut<T>` inside a system: writing
    /// through the returned guard (any `DerefMut`, or [`Mut::set_if_neq`]) bumps
    /// the row's `changed_tick`, so a subsequent `Changed<T>` query observes the
    /// write. Returns `None` if the entity is stale (wrong generation), was
    /// never registered, or its archetype does not host `T`.
    ///
    /// # `is_added` / `is_changed` semantics (O4)
    ///
    /// Outside a system there is no `last_run` frame boundary, so this `Mut` is
    /// constructed with `last_run == this_run == current_tick()`. Its
    /// [`Mut::is_added`] / [`Mut::is_changed`] therefore report whether the row
    /// was touched **at the current tick** ("changed relative to the current
    /// tick"), NOT "changed since a previous system run". For frame-delta
    /// semantics, query `Mut<T>` inside a system.
    ///
    /// ## Inside a `Schedule` frame (Bug #56 interaction)
    ///
    /// When called from within a running `Schedule` frame (e.g. an exclusive
    /// `|w: &mut EcsMaster|` system), `current_tick()` is the **apply-window
    /// tick** — one past the frame-start `this_run` that scheduled systems'
    /// `Changed<T>` / `Added<T>` windows are keyed on (the apply-window bump that
    /// makes deferred-command changes observable; see [`Schedule::run`]). A write
    /// made through this guard is therefore observed by a `Changed<T>` /
    /// `Added<T>` reader on the **following** frame (exactly once), like a
    /// deferred-command change — NOT the same frame. For same-frame change
    /// detection within a system, query `Mut<T>` from the system instead (it
    /// stamps at the system's `this_run`).
    ///
    /// [`Schedule::run`]: crate::ecs::core::schedule::schedule::Schedule::run
    #[inline]
    pub fn get_component_mut<T: crate::ecs::core::component::component::Component>(
        &mut self,
        entity: Entity,
    ) -> Option<Mut<'_, T>> {
        // Resolve the inland by value (releases the entity_master borrow before
        // the raw archetype_ptr deref) — same prologue as get_component_raw_mut.
        let inland: EntityInland = *self.entity_master.entities_inland.get(entity.id().0)?;
        if inland.is_null() || inland.generation() != entity.generation() {
            return None;
        }
        let cid = T::component_id();
        debug_assert!(cid.0 < MAX_COMPONENTS);
        let idx = inland.unit_index() as usize;
        let this_run = self.current_tick();

        // BUG-MIGRATE-TB-1: project the individual fields (`columns`,
        // `component_pools`) through the raw slab pointer; do NOT form a
        // struct-wide `&mut Archetype` (a foreign read/retag that freezes a
        // sibling-written `current_index`/`entity_ids`).
        // SAFETY (OBS-MUT1): `inland.archetype_ptr()` is write-capable, stable,
        //   interior-mutable (`SharedReadWrite`, F4-rooted) slab provenance
        //   (U1/U14/F1); it survives sibling structural writes under TB/SB.
        //   `&mut self` ⇒ exclusive access — no other thread or borrow can read
        //   or write any slot in this archetype for the `Mut`'s lifetime.
        let archetype_ptr = inland.archetype_ptr();
        // SAFETY (U4): `cid.0 < MAX_COMPONENTS` (debug-asserted above; the column
        //   table is `[Column; MAX_COMPONENTS]`). `Column` is `Copy`.
        let column = unsafe {
            let columns_ptr = core::ptr::addr_of!((*archetype_ptr).columns).cast::<Column>();
            *columns_ptr.add(cid.0)
        };
        if column.ptr.is_null() {
            return None;
        }

        // Per-row tick slots come from the COLUMN BASE + idx (NOT the column base
        // alone). `tick_column_base` reads only `self.component_pools`; reborrow
        // ONLY that field (sub-range), never the whole struct.
        // SAFETY: same provenance note as above; `tick_column_base` takes `&self`
        //   over the `component_pools` field only.
        let (added_base, changed_base) = unsafe {
            (*core::ptr::addr_of!((*archetype_ptr).component_pools))
                .get_pool(cid)
                .map(|pool| (pool.added_ticks_ptr(), pool.changed_ticks_ptr()))
        }?;

        // SAFETY (OBS-MUT2): the row is live (`inland` non-null + generation
        //   match), so `idx < pool.count() <= committed_rows`; both tick bases
        //   are write-once sub-region pointers into the pool's own
        //   `VmReservation` (address-stable for the pool's lifetime — Phase
        //   X.I), and the access stays inside the committed prefix
        //   `[0, committed_rows)` by the bound above.
        //   The `added` read is an eager `Copy` snapshot; `changed_tick` is
        //   offset to this row. The `&mut T` reborrows `column.ptr + idx*stride`,
        //   whose exclusivity rests SOLELY on `&mut self` (OBS-MUT — NOT SCH3:
        //   this is the system-less direct-API path with no conflict graph in
        //   play). The returned `Mut<'_, T>` is tied to `&mut self`, so no
        //   concurrent reader/writer of this row's value or tick can exist.
        let added: Tick = unsafe { *(*added_base.add(idx)).get() };
        let changed_tick: *const UnsafeCell<Tick> = unsafe { changed_base.add(idx) };
        let value: &mut T =
            unsafe { &mut *(column.ptr.add(idx * column.stride as usize) as *mut T) };

        Some(Mut {
            value,
            added,
            changed_tick,
            // O4: no system ran this — there is no frame delta. `last_run ==
            // this_run` makes is_added/is_changed report "newer than
            // (this_run - 1)", i.e. "changed relative to the current tick".
            last_run: this_run,
            this_run,
            deref_mut_called: false,
        })
    }

    /// Checks if an entity has a specific component.
    ///
    /// Uses the fast inland + column lookup: a null `column.ptr` is the
    /// single source of truth for "archetype does not host this component".
    #[inline]
    pub fn has_component(&self, entity: Entity, component_id: ComponentId) -> bool {
        let Some(inland) = self.entity_master.entities_inland.get(entity.id().0) else {
            return false;
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return false;
        }
        if component_id.0 >= MAX_COMPONENTS {
            return false;
        }
        // BUG-MIGRATE-TB-1: project `columns` (offset 0) through the raw slab
        // pointer instead of forming `&Archetype` — a foreign `&Archetype` read
        // would freeze a concurrently sibling-written `current_index`, making the
        // bundle-`Box` dealloc on world drop UB.
        // SAFETY (U1, U2, U4, U11, F1): archetype_ptr is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — survives sibling
        //   structural writes under TB/SB (whole slab element is
        //   `UnsafeCell`-wrapped); `component_id.0 < MAX_COMPONENTS` (checked)
        //   keeps the index in bounds. `Column` is `Copy`.
        let column = unsafe {
            let columns_ptr =
                core::ptr::addr_of!((*inland.archetype_ptr()).columns).cast::<Column>();
            *columns_ptr.add(component_id.0)
        };
        !column.ptr.is_null()
    }

    /// Gets raw pointers to multiple components for an entity.
    ///
    /// Resolves the inland record once, then walks `component_ids` reading
    /// the cached `Column` table inline. Returns `(ComponentId, *const u8)`
    /// pairs only for components actually hosted by the entity's archetype.
    pub fn get_components_raw(
        &self,
        entity: Entity,
        component_ids: &[ComponentId],
    ) -> Vec<(ComponentId, *const u8)> {
        let mut result = Vec::with_capacity(component_ids.len());
        let Some(inland) = self.entity_master.entities_inland.get(entity.id().0) else {
            return result;
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return result;
        }
        // BUG-MIGRATE-TB-1: project `columns` (offset 0) through the raw slab
        // pointer; do NOT form `&Archetype` (which would freeze a concurrently
        // sibling-written `current_index`).
        // SAFETY (U1, U2, U11, F1): archetype_ptr is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — survives sibling
        //   structural writes under TB/SB (whole slab element is
        //   `UnsafeCell`-wrapped).
        let columns_ptr =
            unsafe { core::ptr::addr_of!((*inland.archetype_ptr()).columns).cast::<Column>() };
        let unit_index = inland.unit_index() as usize;
        for &component_id in component_ids {
            if component_id.0 >= MAX_COMPONENTS {
                continue;
            }
            // SAFETY (U4): bounded by check above; `Column` is `Copy`.
            let column = unsafe { *columns_ptr.add(component_id.0) };
            if column.ptr.is_null() {
                continue;
            }
            // SAFETY (U5, U6, U10): same as get_component_raw.
            let ptr = unsafe { column.ptr.add(unit_index * column.stride as usize) } as *const u8;
            result.push((component_id, ptr));
        }
        result
    }

    /// Gets mutable raw pointers to multiple components for an entity.
    ///
    /// Mutable counterpart of `get_components_raw`; the inland is copied
    /// by value (16 B) to release the `entity_master` borrow before the
    /// `archetype_ptr` is reborrowed as `&mut Archetype` (W4 / U14).
    pub fn get_components_raw_mut(
        &mut self,
        entity: Entity,
        component_ids: &[ComponentId],
    ) -> Vec<(ComponentId, *mut u8)> {
        let mut result = Vec::with_capacity(component_ids.len());
        let inland: EntityInland = match self.entity_master.entities_inland
            .get(entity.id().0)
        {
            Some(i) => *i,
            None => return result,
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return result;
        }
        // BUG-MIGRATE-TB-1: project `columns` (offset 0) through the raw slab
        // pointer; do NOT form `&mut Archetype` (which would narrow / freeze a
        // concurrently sibling-written `current_index`).
        // SAFETY (U1, U2, U11, U14, F1): write-capable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance under &mut self —
        //   survives sibling structural writes under TB/SB (whole slab element
        //   is `UnsafeCell`-wrapped); no other live borrow into this slot.
        let columns_ptr =
            unsafe { core::ptr::addr_of!((*inland.archetype_ptr()).columns).cast::<Column>() };
        let unit_index = inland.unit_index() as usize;
        for &component_id in component_ids {
            if component_id.0 >= MAX_COMPONENTS {
                continue;
            }
            // SAFETY (U4): bounded by check above; `Column` is `Copy`.
            let column = unsafe { *columns_ptr.add(component_id.0) };
            if column.ptr.is_null() {
                continue;
            }
            // SAFETY (U5, U6, U10): same as get_component_raw_mut.
            let ptr = unsafe { column.ptr.add(unit_index * column.stride as usize) };
            result.push((component_id, ptr));
        }
        result
    }

}
