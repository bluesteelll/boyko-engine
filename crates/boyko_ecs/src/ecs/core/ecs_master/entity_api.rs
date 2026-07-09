//! Entity & archetype lifecycle surface on [`EcsMaster`] (mechanical split).
//!
//! Archetype creation, single/batch spawn, despawn, clone / prefab, and the
//! private structural helpers they route through. Extracted verbatim from
//! `ecs_master.rs`; the inherent-`impl` methods keep their exact paths.
use std::ptr::NonNull;

use crate::ecs::core::archetype::archetype::{Archetype, RemoveOutcome};
use crate::ecs::core::component::component_registry::{self, MAX_COMPONENTS};
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::hooks::dispatch::{
    trigger_on_add, trigger_on_despawn, trigger_on_insert, trigger_on_remove, trigger_on_replace,
};
use crate::ecs::core::component::hooks::scope::DeferredScopeGuard;
use crate::ecs::core::component::observers::dispatch::{
    fire_on_add_observers, fire_on_despawn_observers, fire_on_insert_observers,
    fire_on_remove_observers, fire_on_replace_observers,
};
use crate::ecs::core::component::observers::entity_store::fire_entity_observers;
use crate::ecs::core::component::observers::ObserverKind;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, InlandPoolId};
use crate::ecs::error::{EcsError, EcsResult};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

/// One `(ComponentId, &[u8])` component-data entry, as accepted by the direct
/// `create_entity` / `create_entity_at` API and partitioned into table / dense
/// subsets by [`EcsMaster::partition_dense_components`] (Dense plan D2). Aliased
/// to keep the partition signature readable (clippy::type_complexity).
type ComponentEntry<'a> = (ComponentId, &'a [u8]);

/// Dense plan D2 — `true` iff `cid` is a signature-storage (table) id. The
/// structural-op fire loops iterate an archetype's RETAINED `component_ids`
/// (which keeps non-signature ids since D0), so they skip a dense (or bitset) id
/// via this predicate — dense is fired by the dedicated D2 routing, never the
/// table `component_ids` machinery. For a table-only world this is always `true`
/// (cold load + branch on an already-cold path; the 0%-gate).
#[inline]
fn is_signature_cid(cid: ComponentId) -> bool {
    component_registry::is_signature_id(cid)
}

impl EcsMaster {
    /// Creates a new archetype with the specified component IDs
    /// Returns the ID of the created archetype
    #[inline]
    pub fn create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        self.archetype_master.create_archetype(component_ids)
    }

    /// Gets or creates an archetype with the specified component IDs
    /// Returns the ID of the archetype
    #[inline]
    pub fn get_or_create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        self.archetype_master.get_or_create_archetype(component_ids)
    }

    /// Creates a new entity with components in the specified archetype.
    ///
    /// Takes a borrowed slice of `(ComponentId, &[u8])` pairs for component data —
    /// zero allocation per call on the components argument. Returns the created
    /// entity if successful.
    ///
    /// # Guard pattern (C-007)
    ///
    /// Preconditions are validated **before** `allocate_entity` so that
    /// no `EntityId` is leaked if the archetype lookup fails. Specifically:
    /// 1. `has_archetype(archetype_id)` is checked first.
    /// 2. Only then is `allocate_entity` called.
    /// 3. If `create_entity` fails, `rewind_allocate` undoes the allocation
    ///    (fresh-ID path) so the ID is not silently wasted.
    ///
    /// # W7 choreography (Phase 7)
    ///
    /// 1. Mint write-capable `*mut Archetype` via `archetype_ptr_for` under
    ///    `&mut self`. The raw pointer does not participate in borrow
    ///    checking, so it lives across the subsequent `&mut entity_master`
    ///    call without conflict.
    /// 2. Allocate the entity id.
    /// 3. Reborrow the raw pointer as `&mut Archetype` (scoped tightly to
    ///    the `create_entity` call) to push the new row.
    /// 4. On rejection, rewind the entity-id allocation (C-007 rewind path).
    /// 5. On success, register the entity in the Phase 7 fast store
    ///    (read path of `get_component_raw`, `has_entity`, and friends).
    ///
    /// Dense plan D2 — partitions a `(ComponentId, &[u8])` input list into a
    /// TABLE subset (signature-storage ids, fed to `Archetype::create_entity`)
    /// and a DENSE subset (`StorageKind::Dense` ids, routed to `DenseStore`).
    ///
    /// 0%-gate: when the input has NO dense id, the returned table slice is the
    /// ORIGINAL `components` (no copy) and the dense slice is empty — the
    /// pre-dense codegen path is preserved byte-for-byte. The filtered table copy
    /// into `table_buf` only happens when at least one dense id is present.
    ///
    /// `table_buf` / `dense_buf` are caller-provided stack scratch sized to
    /// `MAX_COMPONENTS`; the returned slices borrow from them (or from
    /// `components` for the no-dense table case).
    #[inline]
    fn partition_dense_components<'a>(
        components: &'a [ComponentEntry<'a>],
        table_buf: &'a mut [ComponentEntry<'a>],
        dense_buf: &'a mut [ComponentEntry<'a>],
    ) -> (&'a [ComponentEntry<'a>], &'a [ComponentEntry<'a>]) {
        // Cheap pre-scan: detect any dense id. Cold registration-table read per
        // component, but only at structural-op time (never the per-frame path).
        let has_dense = components.iter().any(|&(cid, _)| {
            matches!(
                component_registry::storage_kind(cid.0),
                component_registry::StorageKind::Dense
            )
        });
        if !has_dense {
            // 0%-gate: hand back the original slice (no copy) + an empty dense set.
            return (components, &[]);
        }
        let mut t = 0usize;
        let mut d = 0usize;
        for &(cid, bytes) in components {
            if matches!(
                component_registry::storage_kind(cid.0),
                component_registry::StorageKind::Dense
            ) {
                debug_assert!(d < dense_buf.len());
                dense_buf[d] = (cid, bytes);
                d += 1;
            } else {
                debug_assert!(t < table_buf.len());
                table_buf[t] = (cid, bytes);
                t += 1;
            }
        }
        (&table_buf[..t], &dense_buf[..d])
    }

    /// Audit: C-010 — switched from Vec to &[...].
    pub fn create_entity(
        &mut self,
        archetype_id: ArchetypeId,
        components: &[(ComponentId, &[u8])],
    ) -> EcsResult<Entity> {
        // Phase 14a §3.2 / §8 P1: RAII depth bracket. `Drop` decrements the
        // depth on EVERY exit (Ok / Err / panic), so the early `return Err`
        // paths below (which all PRECEDE the hook-fire point) strand nothing.
        let scope = DeferredScopeGuard::enter();

        // Guard: validate archetype exists BEFORE allocating an EntityId.
        // Previously, allocate_entity() was called first, and if the archetype
        // lookup subsequently failed the ID was permanently leaked (C-007).
        if !self.archetype_master.has_archetype(archetype_id) {
            return Err(EcsError::ArchetypeNotFound(archetype_id));
        }

        // Step 1 of W7: mint write-capable *mut Archetype. The raw pointer is
        // not subject to borrow checking, so it can outlive the &mut borrow
        // on archetype_master that produced it — see U14. F4: this is a
        // FRESH same-frame local (no sibling structural write intervenes before
        // its reborrows below), so it was already legal pre-fix; it is now also
        // interior-mutable (`SharedReadWrite`, F4-rooted) like every slab ptr.
        let archetype_ptr = self.archetype_master
            .archetype_ptr_for(archetype_id)
            .expect("invariant: archetype existed at guard check; single-threaded");

        // Phase 10 INIT3 / Round 2 W4: the world owns the change-detection
        // tick. Read it once here and thread it into `Archetype::create_entity`
        // so the per-row `added`/`changed` ticks land at the correct value.
        // No caller of `EcsMaster::create_entity` needs to know the tick
        // (single source of truth).
        let current_tick = self.current_tick();

        // Dense plan D2 — partition the input into a TABLE subset (written into
        // the archetype) and a DENSE subset (routed to `DenseStore`, no
        // migration). `Archetype::create_entity` rejects any id with no
        // per-archetype pool, so a dense id MUST NOT reach it. 0%-gate: when no
        // dense id is present, `table_components == components` (the same slice,
        // not a copy) and the dense vec is empty — the path is unchanged.
        let mut table_buf = [(ComponentId(0), &[][..]); MAX_COMPONENTS];
        let mut dense_buf = [(ComponentId(0), &[][..]); MAX_COMPONENTS];
        let (table_components, dense_components) =
            Self::partition_dense_components(components, &mut table_buf, &mut dense_buf);

        // Step 2 of W7: allocate the entity id (fresh or recycled).
        let entity = self.entity_master.allocate_entity();

        // Step 3 of W7: reborrow archetype_ptr as &mut Archetype inside a
        // tight scope so the &mut reference is dropped before any further
        // entity_master mutation.
        let mut new_unit_index: u32 = 0;
        let pushed = {
            // SAFETY (U14, U1, U2):
            //   - U14: archetype_ptr was just minted via archetype_ptr_for
            //     under &mut self, so the provenance is write-capable; the
            //     bundle slab address is stable; no other live borrow into
            //     this slot exists (single-threaded EcsMaster).
            //   - U1/U2: slab address stable, slab slot lifetime ⊇
            //     EcsMaster lifetime (bundle invariants).
            //   - The reborrow is scoped to this block; once create_entity
            //     returns, the &mut Archetype is dropped before any further
            //     self.entity_master calls.
            let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
            archetype.create_entity(
                entity.id(),
                &mut new_unit_index,
                table_components,
                current_tick,
            )
        };

        if !pushed {
            // Step 4 of W7: archetype rejected the push — signature mismatch,
            // or the pool reserve ceiling (rows). Phase X.I: committed
            // capacity below the ceiling grows on demand inside the pools,
            // so a capacity rejection here means the archetype outgrew a
            // pool's reserve_rows. Undo the allocation so the EntityId is
            // not leaked.
            let rewound = self.entity_master.rewind_allocate(entity);
            if !rewound {
                // rewind_allocate returns false for recycled IDs; fall back
                // to the full deallocate path so the ID returns to the free
                // list.
                self.entity_master.deallocate_entity(entity);
            }
            return Err(EcsError::ArchetypeRejectedEntity { archetype_id });
        }

        // Step 5 of W7: register the entity in the Phase 7 fast store. This
        // is the read path consumed by get_component_raw, has_entity,
        // set_component_raw, and the typed get_component<T> /
        // get_component_mut<T> wrappers.
        self.entity_master.register_entity_with_ptr(entity, archetype_ptr, new_unit_index);

        // Step 6 (Phase 14a §3.2): fire on_add / on_insert hooks. The Step-3
        // `&mut Archetype` was block-scoped (`let pushed = { ... }`) and is
        // dead; only `archetype_ptr` (*mut, Copy) survives — no `world`-derived
        // `&mut` is live, so minting `world_ptr` aliases no reborrow (SAFETY-1).
        //
        // P1 invariant: there is NO fallible step after this fire point — every
        // `return Err` above precedes it, so no deferred command is ever
        // enqueued on an `Err` path (nothing to strand).
        debug_assert!(
            self.archetype_master.has_archetype(archetype_id),
            "P1: no fallible step may follow the hook-fire point in a bracketed body"
        );
        // SAFETY: `archetype_ptr` is write-capable + stable slab provenance;
        //   reading `flags` is one `u16` load (no `&mut` taken).
        let flags = unsafe { (*archetype_ptr).flags };
        if !flags.is_empty() {
            let world_ptr = NonNull::from(&mut *self);
            // Phase 14b: inner gates widen HOOK -> ANY (hook OR observer). Hooks
            // fire first, then observers (per-kind block shape, §5). The two
            // nested `contains` are sub-tests of the already-loaded `flags` u16
            // (no extra load); the `ids` slice is read once per kind.
            if flags.contains(ArchetypeFlags::ON_ADD_ANY) {
                // SAFETY: `archetype_ptr` is a valid `*const Archetype`; the
                //   shared slice is transient and not aliased by a live `&mut`.
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
                    for &cid in ids {
                        if !is_signature_cid(cid) {
                            continue;
                        }
                        trigger_on_add(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_ADD_OBSERVER) {
                    for &cid in ids {
                        if !is_signature_cid(cid) {
                            continue;
                        }
                        fire_on_add_observers(world_ptr, cid, entity);
                    }
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
                // SAFETY: same as the on_add slice read above.
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                    for &cid in ids {
                        if !is_signature_cid(cid) {
                            continue;
                        }
                        trigger_on_insert(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                    for &cid in ids {
                        if !is_signature_cid(cid) {
                            continue;
                        }
                        fire_on_insert_observers(world_ptr, cid, entity);
                    }
                }
            }
        }

        // Dense plan D2 — route the dense subset AFTER the entity is registered
        // (so a fired handler can read it) and AFTER the table fires (consistent
        // spawn-time ordering). Each dense insert + on_add/on_insert fire is
        // handled by the shared `dense_insert_and_fire`. 0%-gate: `dense_components`
        // is empty for a table-only input, so this loop runs zero times.
        for &(cid, bytes) in dense_components {
            self.dense_insert_and_fire(entity, archetype_id, cid, bytes);
        }

        // Direct API: drop the bracket (depth back to 0) then drain on the
        // success path (Q-A1 / §8 P1). On a panic above, `scope`'s `Drop`
        // restores the depth and we do NOT drain (running deferred user code
        // mid-unwind is wrong).
        drop(scope);
        self.drain_deferred_hook_queue();

        Ok(entity)
    }

    /// Phase 11 (plan §6.2): pushes an entity row into the specified
    /// archetype, registering an **already-reserved** `Entity` handle in
    /// the Phase 7 fast store.
    ///
    /// Used by `SpawnAtCommand::apply`
    /// after the deferred-spawn path has minted an `Entity` via
    /// `EntityCounter::reserve_entity`.
    /// Unlike [`create_entity`](Self::create_entity), this function does
    /// NOT mint a fresh `Entity` — it expects the caller to pass the
    /// pre-allocated handle.
    ///
    /// # Pre-conditions (debug-asserted)
    ///
    /// * `archetype_id` is registered.
    /// * `entity.id().0`'s slot in `entities_inland` is currently NULL
    ///   (never registered, never spawned-at). The atomic counter ensures
    ///   uniqueness; double-apply on the same handle is a bug at the
    ///   `SpawnAtCommand` enqueue layer.
    ///
    /// # Behaviour
    ///
    /// 1. Resolves the archetype's write-capable raw pointer.
    /// 2. Resizes `entities_inland` if `entity.id().0` is past the current
    ///    length. Phase 12.6 — single-row growth via `Vec::resize` is the
    ///    canonical lazy path; the dispatcher's `&mut self` borrow
    ///    guarantees workers are not in flight.
    /// 3. Pushes the row into the archetype with the world's current tick
    ///    (same INIT3 contract as `create_entity`).
    /// 4. Registers `(entity, archetype_ptr, unit_index)` in the Phase 7
    ///    fast store via `register_entity_with_ptr`.
    pub fn create_entity_at(
        &mut self,
        entity: Entity,
        archetype_id: ArchetypeId,
        components: &[(ComponentId, &[u8])],
    ) -> EcsResult<Entity> {
        // Phase 14a §3.2 / §8 P1: RAII depth bracket (every `return Err` below
        // precedes the hook-fire point, so they strand nothing).
        let scope = DeferredScopeGuard::enter();

        // Guard: archetype existence is checked BEFORE any state mutation.
        if !self.archetype_master.has_archetype(archetype_id) {
            return Err(EcsError::ArchetypeNotFound(archetype_id));
        }

        // EC7 (debug): slot must be NULL (never registered, never
        // spawned-at) at this point.
        debug_assert!(
            self.entity_master
                .entities_inland
                .get(entity.id().0)
                .is_none_or(|i| i.is_null()),
            "create_entity_at: entity {:?} is already registered (double-apply?)",
            entity
        );

        let archetype_ptr = self
            .archetype_master
            .archetype_ptr_for(archetype_id)
            .expect("invariant: archetype existed at guard check; single-threaded");

        let current_tick = self.current_tick();

        // Dense plan D2 — partition into TABLE + DENSE subsets (mirrors
        // `create_entity`). 0%-gate: no dense id ⇒ `table_components == components`
        // (no copy), `dense_components` empty.
        let mut table_buf = [(ComponentId(0), &[][..]); MAX_COMPONENTS];
        let mut dense_buf = [(ComponentId(0), &[][..]); MAX_COMPONENTS];
        let (table_components, dense_components) =
            Self::partition_dense_components(components, &mut table_buf, &mut dense_buf);

        // Phase 12.6 — lazy growth path; Phase X.G — `InlandStore::ensure`
        // extends it on demand under `&mut self` (no worker race per
        // SEND5/SBO16) with zero copies and zero fills.
        let id_raw = entity.id().0;
        self.entity_master.entities_inland.ensure(id_raw + 1);

        let mut new_unit_index: u32 = 0;
        let pushed = {
            // SAFETY (U14, U1, U2, mirrors `create_entity`):
            //   * `archetype_ptr` was just minted via `archetype_ptr_for`
            //     under `&mut self`; provenance is write-capable.
            //   * Bundle slab address is stable; no other live borrow.
            //   * The reborrow is scoped to this block.
            let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
            archetype.create_entity(
                entity.id(),
                &mut new_unit_index,
                table_components,
                current_tick,
            )
        };

        if !pushed {
            return Err(EcsError::ArchetypeRejectedEntity { archetype_id });
        }

        // Register in the Phase 7 fast store. The entity carries its own
        // generation (typically `0` for fresh reserves); we propagate it
        // verbatim through `register_entity_with_ptr`.
        self.entity_master
            .register_entity_with_ptr(entity, archetype_ptr, new_unit_index);

        // Phase 14a §3.2: fire on_add / on_insert hooks (mirrors `create_entity`).
        // The Step-3 `&mut Archetype` was block-scoped and is dead; only
        // `archetype_ptr` survives at the mint (SAFETY-1). P1: no fallible step
        // follows.
        debug_assert!(
            self.archetype_master.has_archetype(archetype_id),
            "P1: no fallible step may follow the hook-fire point in a bracketed body"
        );
        // SAFETY: `archetype_ptr` is write-capable + stable slab provenance.
        let flags = unsafe { (*archetype_ptr).flags };
        if !flags.is_empty() {
            let world_ptr = NonNull::from(&mut *self);
            // Phase 14b: inner gates widen HOOK -> ANY; hooks first, then
            // observers (mirrors `create_entity`, §5).
            if flags.contains(ArchetypeFlags::ON_ADD_ANY) {
                // SAFETY: transient shared slice, not aliased by a live `&mut`.
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
                    for &cid in ids {
                        if !is_signature_cid(cid) {
                            continue;
                        }
                        trigger_on_add(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_ADD_OBSERVER) {
                    for &cid in ids {
                        if !is_signature_cid(cid) {
                            continue;
                        }
                        fire_on_add_observers(world_ptr, cid, entity);
                    }
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
                // SAFETY: same as the on_add slice read above.
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                    for &cid in ids {
                        if !is_signature_cid(cid) {
                            continue;
                        }
                        trigger_on_insert(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                    for &cid in ids {
                        if !is_signature_cid(cid) {
                            continue;
                        }
                        fire_on_insert_observers(world_ptr, cid, entity);
                    }
                }
            }
        }

        // Dense plan D2 — route the dense subset (mirrors `create_entity`).
        // 0%-gate: empty for a table-only input.
        for &(cid, bytes) in dense_components {
            self.dense_insert_and_fire(entity, archetype_id, cid, bytes);
        }

        drop(scope);
        self.drain_deferred_hook_queue();

        Ok(entity)
    }

    /// Phase 12.5 Opt-A3 (§6.4): `create_entity_at` variant that consumes
    /// pre-resolved `pool_ids` from the per-world
    /// [`BundleColumnCache`](crate::ecs::core::bundle::BundleColumnCache),
    /// bypassing the 4× SparseMap lookup of the legacy path.
    ///
    /// `components` MUST be canonical-sorted by `ComponentId.0` (B1/B2);
    /// `pool_ids[i]` corresponds to `components[i].0`. Caller is
    /// `SpawnAtCommand::apply` post-Opt-A3 wiring.
    ///
    /// # Phase 12.6 — legacy bridge
    ///
    /// `SpawnAtCommand::apply` no longer routes through this method; it
    /// inlines the equivalent write loop to avoid the per-spawn slot-array
    /// rebuild + cross-call hop. Retained as the `EcsMaster`-side primitive
    /// reachable by external benchmarks that model the pre-Phase-12.6
    /// dispatch shape (see
    /// `crates/bench_bevy_vs_boyko/benches/profile_spawn_*.rs`).
    #[allow(dead_code)]
    pub(crate) fn create_entity_at_with_pool_ids(
        &mut self,
        entity: Entity,
        archetype_id: ArchetypeId,
        components: &[(ComponentId, &[u8])],
        pool_ids: &[InlandPoolId],
    ) -> EcsResult<Entity> {
        if !self.archetype_master.has_archetype(archetype_id) {
            return Err(EcsError::ArchetypeNotFound(archetype_id));
        }
        debug_assert!(
            self.entity_master
                .entities_inland
                .get(entity.id().0)
                .is_none_or(|i| i.is_null()),
            "create_entity_at_with_pool_ids: entity {:?} is already registered",
            entity
        );

        let archetype_ptr = self
            .archetype_master
            .archetype_ptr_for(archetype_id)
            .expect("invariant: archetype existed at guard check; single-threaded");
        let current_tick = self.current_tick();

        let id_raw = entity.id().0;
        self.entity_master.entities_inland.ensure(id_raw + 1);

        let mut new_unit_index: u32 = 0;
        let pushed = {
            // SAFETY (U14, U1, U2, mirrors `create_entity_at`):
            //   write-capable provenance under `&mut self`; reborrow
            //   scoped to this block.
            let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
            archetype.create_entity_with_pool_ids(
                entity.id(),
                &mut new_unit_index,
                components,
                pool_ids,
                current_tick,
            )
        };
        if !pushed {
            return Err(EcsError::ArchetypeRejectedEntity { archetype_id });
        }
        self.entity_master
            .register_entity_with_ptr(entity, archetype_ptr, new_unit_index);
        Ok(entity)
    }

    /// Type-safe wrapper around `create_entity` for a single component (Phase 2e — Q-024 follow-up).
    ///
    /// The caller supplies the value by move; this function reads its bytes via
    /// `std::slice::from_raw_parts` and forwards to `create_entity`. No heap
    /// allocation, no `Vec` materialisation, no manual `ComponentId` lookup.
    ///
    /// ```ignore
    /// // Before:
    /// ecs.create_entity(arch_id, &[(Position::component_id(), &pos_bytes)])
    /// // After:
    /// ecs.spawn_one(arch_id, Position { x: 1.0, y: 2.0, z: 3.0 })
    /// ```
    ///
    /// # Drop discipline
    ///
    /// On success, `a` is byte-copied into the pool by `ComponentPool::add`
    /// (`ptr::copy_nonoverlapping`) and the pool's registered `drop_fn` (set up
    /// by `register_layout::<A>`, M-001) becomes the new drop owner. The local
    /// `a` value must NOT run its destructor — `std::mem::forget(a)` suppresses
    /// the local drop only on the Ok path.
    ///
    /// On failure, NO bytes were copied into the pool (the failure modes are
    /// either an early `ArchetypeNotFound` guard or a pool rejection that
    /// rewinds without writing). `a` retains its full identity and runs its
    /// destructor at function-exit scope as usual — no leak, no double-free.
    ///
    /// Bounded 1-arity typed spawn helper; a generic tuple version is a
    /// Phase 2e-extension.
    pub fn spawn_one<A: crate::ecs::core::component::component::Component>(
        &mut self,
        archetype_id: ArchetypeId,
        a: A,
    ) -> EcsResult<Entity> {
        // SAFETY: `a` is a valid, fully-initialised `A` living on the caller's
        // stack; we read `size_of::<A>()` bytes out of it as `&[u8]`. The slice
        // borrow is scoped to this call.
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(a) as *const u8,
                std::mem::size_of::<A>(),
            )
        };
        let result = self.create_entity(archetype_id, &[(A::component_id(), bytes)]);
        if result.is_ok() {
            // Bytes are now in the pool; pool's drop_fn is the new owner.
            std::mem::forget(a);
        }
        // On Err: no bytes copied; `a` drops normally at scope exit.
        result
    }

    /// Type-safe two-component spawn — see `spawn_one` for rationale.
    ///
    /// Bounded 2-arity typed spawn helper.
    ///
    /// # Drop discipline
    ///
    /// Same as `spawn_one`: on Ok, both `a` and `b` are byte-copied into
    /// their respective pools and `mem::forget`'d locally so their pool
    /// `drop_fn`s become the new owners. On Err, NEITHER value was copied
    /// (either the archetype guard fired before any copy, or the pool's
    /// `can_push_entity_components` rejected the batch before any pool was
    /// mutated — two-phase commit, C-009), so both values drop normally
    /// at function-exit scope.
    pub fn spawn_two<
        A: crate::ecs::core::component::component::Component,
        B: crate::ecs::core::component::component::Component,
    >(
        &mut self,
        archetype_id: ArchetypeId,
        a: A,
        b: B,
    ) -> EcsResult<Entity> {
        // SAFETY: same rationale as `spawn_one`, applied to both inputs.
        // The two slices view distinct stack locals — no aliasing.
        let bytes_a: &[u8] = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(a) as *const u8,
                std::mem::size_of::<A>(),
            )
        };
        let bytes_b: &[u8] = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(b) as *const u8,
                std::mem::size_of::<B>(),
            )
        };
        let result = self.create_entity(
            archetype_id,
            &[
                (A::component_id(), bytes_a),
                (B::component_id(), bytes_b),
            ],
        );
        if result.is_ok() {
            std::mem::forget(a);
            std::mem::forget(b);
        }
        result
    }

    /// Spawns an entity with ZERO components (Phase 22 D5(4)).
    ///
    /// The EMPTY archetype is resolved lazily through the normal
    /// [`get_or_create_archetype`](Self::get_or_create_archetype) funnel on
    /// first use — no reserved constant, no eager creation (the Phase 12.6
    /// lazy `EcsMaster::new` budget is preserved) — and is found by the
    /// registry's exact-mask match thereafter.
    ///
    /// The returned entity matches NO component query (the empty signature
    /// is matched only by zero-required-component filters — flecs-invariant
    /// subset matching, D5(5)) and can receive components later through the
    /// ordinary insert/migration funnel.
    pub fn spawn_empty(&mut self) -> Entity {
        let empty_archetype_id = self.get_or_create_archetype(&[]);
        self.create_entity(empty_archetype_id, &[]).expect(
            "invariant: the empty archetype accepts every zero-component push \
             (signature-subset and pool-capacity checks are vacuous)",
        )
    }

    /// Cold despawn-hook fire (Phase 14a §3.6 / W1 / §8 P4; Feature 2 despawn).
    ///
    /// Fires `on_despawn` (Feature 2, Despawn-FIRST), then `on_replace` +
    /// `on_remove`, for EVERY component of the dying entity, reading the row
    /// PRE-`remove_entity`.
    /// Called by [`Self::delete_entity`] ONLY when the archetype's flag set is
    /// non-empty (some component is hooked), so the ~4 KB id buffer below never
    /// touches `delete_entity`'s prologue — the no-hook hot path keeps its slim
    /// frame (the 0% bench gate). `#[cold] #[inline(never)]` keeps it out of the
    /// hot path's I-cache footprint.
    ///
    /// `archetype_ptr` is the dying entity's `EntityInland::archetype_ptr()`
    /// (write-capable, stable slab provenance); `flags` and `component_ids` are
    /// re-read through it here so the caller carries only the pointer + entity.
    #[cold]
    #[inline(never)]
    fn fire_despawn_hooks(&mut self, entity: Entity, archetype_ptr: *mut Archetype) {
        // Stack buffer (W1): only the touched `[0..n)` prefix is written
        // (`n` ≤ ~32 typical, ≤ MAX_COMPONENTS worst case) — no full memset,
        // no `to_vec()` per-despawn heap alloc. This array lives in THIS cold
        // frame, not `delete_entity`'s.
        let mut id_buf = [ComponentId(0); MAX_COMPONENTS];
        // SAFETY (F1): `archetype_ptr` is the caller's `inland.archetype_ptr()`
        //   — write-capable, stable, interior-mutable (`SharedReadWrite`,
        //   F4-rooted) slab provenance for the EcsMaster's lifetime; it survives
        //   sibling structural writes under TB/SB (whole slab element is
        //   `UnsafeCell`-wrapped). Re-reading `flags` is one `u16` load (no
        //   `&mut` taken).
        let flags = unsafe { (*archetype_ptr).flags };
        let n = {
            // SAFETY (F1): transient SHARED `&Archetype` for the id copy; dropped
            //   at the block close before `world_ptr` is minted, so no `world`-
            //   derived `&mut`/`&` is live across the fire point (SAFETY-1). The
            //   pointer is interior-mutable (`SharedReadWrite`, F4-rooted), so a
            //   prior sibling structural write did not invalidate it.
            let arche = unsafe { &*archetype_ptr };
            // Dense plan D2: copy ONLY signature (table) ids into the fire buffer.
            // The archetype's `component_ids` RETAINS non-signature ids (dense /
            // bitset, since D0), but dense despawn fires are owned by the dedicated
            // `dense_despawn_fire_and_tombstone` routing — so the table despawn
            // loops below must skip them. For a table-only archetype this filter is
            // a verbatim copy (every id is `Table`) — the 0%-gate.
            let mut count = 0usize;
            for &cid in arche.component_ids() {
                if is_signature_cid(cid) {
                    id_buf[count] = cid;
                    count += 1;
                }
            }
            count
            // <-- `&Archetype` drops here.
        };
        // MINT: the shared borrow is dead; no `world`-derived `&mut` is live.
        // The helper takes `&mut self`, so `NonNull::from(&mut *self)` reborrows
        // the dispatcher's exclusive access for the cold fire only.
        let world_ptr = NonNull::from(&mut *self);
        // PRE-DROP (Feature 2, Despawn-FIRST): on_despawn for ALL components,
        // BEFORE the on_replace/on_remove passes and BEFORE remove. The handler
        // reads the fully-intact dying entity (every component still present and
        // un-replaced). Within one entity the order is Despawn -> Replace ->
        // Remove (all pre-drop). For the parent-first cascade contract (FIX
        // W10): the parent's on_despawn fires here (seeing its intact subtree),
        // then the parent's `Children::on_replace` enqueues the children for
        // deferred despawn, so each child's on_despawn fires later as the
        // deferred cascade drains.
        if flags.contains(ArchetypeFlags::ON_DESPAWN_ANY) {
            if flags.contains(ArchetypeFlags::ON_DESPAWN_HOOK) {
                for &cid in &id_buf[..n] {
                    trigger_on_despawn(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_DESPAWN_OBSERVER) {
                for &cid in &id_buf[..n] {
                    fire_on_despawn_observers(world_ptr, cid, entity);
                }
            }
        }
        // Feature 2 — entity-targeted on_despawn observers (one fire per dying
        // entity, gated by the archetype's sticky HAS_ENTITY_OBSERVER bit). Per
        // component cid so an entity observer registered for a specific
        // component's despawn fires; the fire loop filters by (key, component).
        if flags.contains(ArchetypeFlags::HAS_ENTITY_OBSERVER) {
            for &cid in &id_buf[..n] {
                fire_entity_observers(world_ptr, ObserverKind::Despawn, cid, entity);
            }
        }
        // PRE-DROP (SAFETY-2): on_replace + on_remove for ALL, BEFORE remove.
        // Phase 14b: inner gates widen HOOK -> ANY; per kind, hooks fire first,
        // then observers (§5). The outer `!flags.is_empty()` gate (in
        // `delete_entity`) already covers the observer bits — same `u16` — so it
        // is unchanged; only these inner per-kind tests widen.
        if flags.contains(ArchetypeFlags::ON_REPLACE_ANY) {
            if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
                for &cid in &id_buf[..n] {
                    trigger_on_replace(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_REPLACE_OBSERVER) {
                for &cid in &id_buf[..n] {
                    fire_on_replace_observers(world_ptr, cid, entity);
                }
            }
        }
        if flags.contains(ArchetypeFlags::ON_REMOVE_ANY) {
            if flags.contains(ArchetypeFlags::ON_REMOVE_HOOK) {
                for &cid in &id_buf[..n] {
                    trigger_on_remove(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_REMOVE_OBSERVER) {
                for &cid in &id_buf[..n] {
                    fire_on_remove_observers(world_ptr, cid, entity);
                }
            }
        }
    }

    /// Deletes an entity and all its components from the system.
    ///
    /// Returns `true` on success, `false` if the entity does not exist or if
    /// archetype removal fails (`RemoveOutcome::PoolFailure`). The explicit
    /// [`RemoveOutcome`] enum (C-006) replaces the previous fragile
    /// `Option<EntityId>`-based logic.
    pub fn delete_entity(&mut self, entity: Entity) -> bool {
        let result = self.delete_entity_core(entity);
        // Direct API: drain on this (post-fire) path. When this method is reached
        // from `DespawnCommand::apply` at depth >= 1, the drain observes
        // `depth != 0` and returns immediately — the outermost owner drains
        // (Q-A1 / C1).
        self.drain_deferred_hook_queue();
        result
    }

    /// Despawns `entity` WITHOUT cascading to its children (Phase 19 W4).
    ///
    /// The opt-out to the default-recursive despawn: the [`Children`] cascade
    /// hook is suppressed for exactly this one removal, so the children survive
    /// — each keeps a now-**dangling** [`ChildOf`] pointing at the freed parent
    /// (a documented footgun; reparent or despawn them explicitly). Equivalent
    /// to Bevy 0.16's `despawn_related`-less single despawn.
    ///
    /// Returns `true` on success, `false` for a stale / never-registered handle
    /// (same contract as [`delete_entity`](Self::delete_entity)).
    ///
    /// [`Children`]: crate::ecs::core::hierarchy::Children
    /// [`ChildOf`]: crate::ecs::core::hierarchy::ChildOf
    pub fn despawn_without_children(&mut self, entity: Entity) -> bool {
        let result = {
            // The guard spans ONLY the hook-fire core, NOT the drain below: the
            // suppress is for THIS entity's cascade hook, and over-suppressing
            // the subsequent drain would wrongly stop unrelated despawns enqueued
            // by other hooks from cascading. Mirrors `DeferredScopeGuard`'s
            // TLS-only discipline (touches no `EcsMaster` field → cannot be
            // frozen by the `&mut self` reborrow).
            let _suppress = crate::ecs::core::hierarchy::commands::CascadeSuppressGuard::enter();
            self.delete_entity_core(entity)
            // <-- `_suppress` drops here, BEFORE the drain.
        };
        self.drain_deferred_hook_queue();
        result
    }

    // ── Entity cloning (Feature 3) ──────────────────────────────────────────

    /// Clones `source` into a brand-new entity, cloning all cloneable components
    /// (opt-out, Bevy `clone_and_spawn` parity). Shallow, fires `on_add` /
    /// `on_insert`. Returns the new entity.
    ///
    /// Drains the deferred-hook queue at the outermost depth, like the spawn /
    /// despawn direct APIs.
    ///
    /// # Panics
    ///
    /// If `source` is not alive (stale / never-registered handle).
    #[inline]
    pub fn clone_and_spawn(&mut self, source: Entity) -> Entity {
        let cloner = crate::ecs::core::clone::EntityCloner::default_built();
        self.clone_and_spawn_with(source, &cloner)
    }

    /// Clones `source` into a new entity using `cloner`'s configuration (filter,
    /// shallow/deep, fire-hooks, strict, preserve-ticks). Returns the new (root)
    /// entity. Panics if `source` is not alive.
    pub fn clone_and_spawn_with(
        &mut self,
        source: Entity,
        cloner: &crate::ecs::core::clone::EntityCloner,
    ) -> Entity {
        assert!(
            self.has_entity(source),
            "clone_and_spawn: source entity {:?} is not alive",
            source
        );
        // Depth bracket + outermost drain (mirrors `create_entity`): nested fires
        // (from on_add/on_insert) enqueue commands; only the outermost owner drains.
        let scope = DeferredScopeGuard::enter();
        let entity = if cloner.is_deep() {
            crate::ecs::core::clone::deep::clone_subtree(self, source, cloner)
        } else {
            crate::ecs::core::clone::materialize::materialize_clone(self, source, cloner).entity
        };
        drop(scope);
        self.drain_deferred_hook_queue();
        entity
    }

    /// Deep-clones `source` and its `ChildOf` subtree (convenience for
    /// `EntityCloner::new().linked(true)`). Returns the cloned root. Panics if
    /// `source` is not alive.
    #[inline]
    pub fn clone_subtree(&mut self, source: Entity) -> Entity {
        let cloner = crate::ecs::core::clone::EntityCloner::new().linked(true).build();
        self.clone_and_spawn_with(source, &cloner)
    }

    /// Captures `source` and its `ChildOf` subtree into a frozen, source-independent
    /// [`Prefab`](crate::ecs::core::clone::Prefab) using the default opt-out cloner
    /// (all cloneable components, Bevy parity).
    ///
    /// The returned prefab OWNS its component bytes — built once on the audited clone
    /// machinery (`clone_fn` per component, so non-`SerPod` components like
    /// `Transform` round-trip) — and **survives `source` (and its subtree) being
    /// despawned**. Instantiate it any number of times via
    /// [`instantiate`](Self::instantiate).
    ///
    /// # Panics
    ///
    /// If `source` is not alive (stale / never-registered handle).
    #[inline]
    pub fn capture_prefab(&mut self, source: Entity) -> crate::ecs::core::clone::Prefab {
        let cloner = crate::ecs::core::clone::EntityCloner::default_built();
        self.capture_prefab_with(source, &cloner)
    }

    /// Captures `source` and its `ChildOf` subtree into a frozen
    /// [`Prefab`](crate::ecs::core::clone::Prefab) using `cloner`'s configuration
    /// (filter / strict / fire-hooks). The subtree is always captured deeply (a
    /// prefab is a subtree); `cloner.linked` is therefore ignored, and
    /// `cloner.preserve_ticks` is ignored by the prefab path (instances are "Added"
    /// at instantiate time — see [`instantiate`](Self::instantiate)).
    ///
    /// # Panics
    ///
    /// If `source` is not alive.
    pub fn capture_prefab_with(
        &mut self,
        source: Entity,
        cloner: &crate::ecs::core::clone::EntityCloner,
    ) -> crate::ecs::core::clone::Prefab {
        assert!(
            self.has_entity(source),
            "capture_prefab: source entity {:?} is not alive",
            source
        );
        crate::ecs::core::clone::prefab::capture(self, source, cloner)
    }

    /// Instantiates `prefab` into this world, returning the **detached** instance
    /// root (no `ChildOf` — the caller parents it as it wishes).
    ///
    /// Each call yields an independent deep copy (re-runs each component's `clone_fn`
    /// from the template, so instances never share bytes). Internal `ChildOf` is
    /// remapped to the fresh instance parents and `Children` is rebuilt; non-`ChildOf`
    /// entity refs are kept verbatim (the v1 clone boundary).
    ///
    /// Instances are **Added at instantiate time**: their change-detection ticks are
    /// reset to the current tick, so `Added` / `Changed` fire the frame they are
    /// instantiated. `cloner.preserve_ticks` is ignored by the prefab path (a frozen
    /// template's capture-time ticks are stale by instantiate). `on_add` / `on_insert`
    /// fire per the cloner captured into the prefab.
    ///
    /// Drains the deferred-hook queue at the outermost depth, like the other
    /// structural direct APIs.
    pub fn instantiate(&mut self, prefab: &crate::ecs::core::clone::Prefab) -> Entity {
        // Depth bracket + outermost drain (mirrors `clone_and_spawn_with`): nested
        // fires from on_add/on_insert enqueue commands; only the outermost owner
        // drains.
        let scope = DeferredScopeGuard::enter();
        let entity = crate::ecs::core::clone::prefab::instantiate(self, prefab);
        drop(scope);
        self.drain_deferred_hook_queue();
        entity
    }

    /// Removal core shared by [`delete_entity`](Self::delete_entity) and
    /// [`despawn_without_children`](Self::despawn_without_children): fires the
    /// pre-remove hooks and releases the row, but does NOT drain the deferred
    /// queue (the caller owns the drain so the suppress window can be scoped
    /// tightly around the fire — Phase 19 W4).
    fn delete_entity_core(&mut self, entity: Entity) -> bool {
        // Phase 14a §3.6 / §8 P1: RAII depth bracket. The two early `return
        // false` paths below PRECEDE the hook-fire point (no command can have
        // been enqueued), so the guard's `Drop` simply restores the depth.
        let scope = DeferredScopeGuard::enter();

        // Resolve the fast inland by value. Copying 16 B releases the
        // entity_master borrow before we dereference the raw archetype_ptr.
        let inland: EntityInland = {
            let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
                return false;
            };
            if slot.is_null() || slot.generation() != entity.generation() {
                return false;
            }
            *slot
        };
        let removed_unit_index = InlandPoolId(inland.unit_index() as usize);

        // Re-derive the dying entity's slab pointer FRESHLY under the live
        // `&mut self` protector, then drive every slab access (flags read,
        // hook fire, `remove_entity` write) through it.
        //
        // TB rationale (BUG-P3-TB-1): the cached `inland.archetype_ptr()` was
        // minted via `archetype_ptr_for` during a now-DEAD registration borrow,
        // so it is NOT a descendant of the live, EcsMaster-lifetime,
        // interior-mutable slab protector. In a move-then-query window an
        // earlier sibling migration has narrowed that protector to `Unique`
        // (`migration_helpers.rs`'s `&mut Archetype` reborrow); a subsequent
        // access through the stale-rooted cached pointer is then a FOREIGN
        // read/write to the protector — the `&*archetype_ptr` hook read freezes
        // it and the `current_index`/`entity_ids` structural write disables it,
        // after which the next `&self` slab read (`query_entities`) reborrows a
        // child of the dead tag and traps. Re-minting via `archetype_ptr_for`
        // under the current `&mut self.archetype_master` makes every access a
        // CHILD of the live protector, so none is foreign. Same discipline as
        // the Phase 9.3 / BUG-P19-TB-1 / BUG-MIGRATE-TB-1 fixes (mutate/read
        // through the protected chain, not a separately-rooted cached pointer).
        //
        // SAFETY (U1, U2, U11, F1, BUG-MIGRATE-TB-1): the `id` is read via a raw
        //   `addr_of!` projection + `.read()` — NO intermediate `&Archetype` is
        //   formed, so this read does not freeze a sibling-narrowed protector
        //   (a `.id()` method call would auto-ref `&Archetype` and freeze). The
        //   cached pointer is stable, interior-mutable (`SharedReadWrite`,
        //   F4-rooted) slab provenance; the slot is live (`is_null`/generation
        //   checked above), so the `Archetype` is initialised and `id` is valid.
        let archetype_id =
            unsafe { core::ptr::addr_of!((*inland.archetype_ptr()).id).read() };

        // SAFETY (U1, U2, U11, U14, F1): `archetype_ptr_for` mints write-capable
        //   provenance under the current `&mut self.archetype_master` borrow —
        //   a CHILD of the live slab protector. The id was just read from the
        //   live slot, so the archetype is registered (the lookup cannot miss).
        let archetype_ptr = self
            .archetype_master
            .archetype_ptr_for(archetype_id)
            .expect("invariant: archetype of a live entity is registered; single-threaded");

        // Phase 14a §3.6 / W1: PRE-`remove_entity` fire of `on_replace` +
        // `on_remove` for ALL components, reading the dying row. The flags read
        // is one `u16` load (the cheap gate that stays inline here); the
        // ~4 KB `[ComponentId; MAX_COMPONENTS]` id buffer + the trigger loops
        // live in the cold `fire_despawn_hooks` helper, so this hot fn's
        // prologue never reserves that stack slot (§8 P4).
        //
        // SAFETY (F1, BUG-P3-TB-1): `archetype_ptr` is the freshly re-minted,
        //   protector-rooted, interior-mutable slab pointer. Reading `flags` via
        //   `addr_of!` (no `&Archetype` reborrow) is a child read of the live
        //   protector and never freezes/disables it.
        let flags = unsafe { core::ptr::addr_of!((*archetype_ptr).flags).read() };
        if !flags.is_empty() {
            self.fire_despawn_hooks(entity, archetype_ptr);
        }

        // Dense plan D2 — fire dense on_despawn / on_replace / on_remove for every
        // dense membership of the dying entity, then tombstone each membership in
        // its `DenseStore`. Runs PRE-`remove_entity` (same window as the table
        // despawn fire above), reading the dying dense state. 0%-gated: a
        // table-only world (`dense_registry.is_empty()`) skips this entirely.
        // Rides `delete_entity_core`, so the hierarchy despawn-cascade (each
        // cascaded child despawn flows through this same core) tombstones + fires
        // for cascaded entities too.
        if !self.dense_registry.is_empty() {
            self.dense_despawn_fire_and_tombstone(entity);
        }

        // Feature 2 — reclaim this entity's entity-targeted observer slot AFTER
        // its on_despawn observers fired, so a recycled `EntityId` never inherits
        // a dead observer (the recycle guard). Idempotent + lazy: a no-op (one
        // `Option::is_none()`) for a world that has no entity observers.
        self.entity_observers.retire(entity);

        // Drive the structural removal through the SAME freshly-minted,
        // protector-rooted `archetype_ptr` (re-derived above under
        // `&mut self.archetype_master`). The `&mut Archetype` reborrow here
        // narrows the interior-mutable cell to `Unique` for the duration of
        // `remove_entity`, but because the pointer is a CHILD of the live
        // protector the `current_index -= 1` / `entity_ids.swap_remove` writes
        // are child writes (not foreign) and never disable it.
        //
        // SAFETY (U1, U2, U11, U14, F1, BUG-P3-TB-1): `archetype_ptr` is
        //   write-capable, protector-rooted, interior-mutable slab provenance
        //   re-minted under the current `&mut self`. Single-threaded `&mut self`
        //   gives exclusive access; no other live borrow into this slot exists.
        //   Re-resolved AFTER the hooks returned (no live reborrow during the
        //   fire).
        let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
        let outcome = archetype.remove_entity(removed_unit_index);

        let result = match outcome {
            RemoveOutcome::Last => {
                self.entity_master.deallocate_entity(entity);
                true
            }
            RemoveOutcome::Swapped { moved_entity: swapped_entity_id } => {
                // The entity that moved into the vacated slot needs its
                // fast-store unit_index updated.
                if let Some(slot) = self.entity_master.entities_inland
                    .get_mut(swapped_entity_id.0)
                {
                    slot.set_unit_index(removed_unit_index.0 as u32);
                }
                self.entity_master.deallocate_entity(entity);
                true
            }
            RemoveOutcome::PoolFailure => false,
        };

        // Drop the bracket (depth back to 0) so the caller's drain runs as the
        // outermost owner.
        drop(scope);
        result
    }

    /// Despawn-path dense fire + tombstone (Dense plan D2): for EVERY dense
    /// membership of the dying `entity`, fires `on_despawn` first (all
    /// memberships, Despawn-first ordering — mirrors `fire_despawn_hooks`), then
    /// `on_replace` + `on_remove`, then tombstones each membership.
    ///
    /// Caller gates on `!dense_registry.is_empty()` (the 0%-gate). Reads the
    /// dying dense state (runs PRE-`remove_entity`). Rides `delete_entity_core`,
    /// so the hierarchy despawn-cascade covers cascaded entities too.
    #[cold]
    #[inline(never)]
    fn dense_despawn_fire_and_tombstone(&mut self, entity: Entity) {
        // Snapshot the membership set into a stack buffer so no `dense_registry`
        // borrow is live across the `world_ptr` mint / fire (the OBS-FIRE-LOOP /
        // SAFETY-1 discipline). `dense_ids` is push-only and small; the membership
        // subset is ≤ MAX_COMPONENTS but typically a handful.
        let mut member_buf = [ComponentId(0); MAX_COMPONENTS];
        let mut n = 0usize;
        for &cid in self.dense_registry.dense_ids() {
            if self
                .dense_registry
                .store(cid)
                .is_some_and(|s| s.contains(entity.id()))
            {
                debug_assert!(n < MAX_COMPONENTS);
                member_buf[n] = cid;
                n += 1;
            }
        }
        if n == 0 {
            return;
        }
        let members = &member_buf[..n];

        // MINT: the membership probe's `&dense_registry` borrows above all ended
        // (the snapshot owns plain `ComponentId`s). No `self`-derived `&mut` into
        // storage is live.
        let world_ptr = NonNull::from(&mut *self);
        // Despawn-first (Feature 2): all dense on_despawn, reading the intact row.
        for &cid in members {
            trigger_on_despawn(world_ptr, cid, entity);
            fire_on_despawn_observers(world_ptr, cid, entity);
        }
        // Then on_replace + on_remove for every membership (still pre-tombstone).
        for &cid in members {
            trigger_on_replace(world_ptr, cid, entity);
            fire_on_replace_observers(world_ptr, cid, entity);
        }
        for &cid in members {
            trigger_on_remove(world_ptr, cid, entity);
            fire_on_remove_observers(world_ptr, cid, entity);
        }
        // Tombstone every membership now that the fires read the dying values.
        for &cid in members {
            let removed = self
                .dense_registry
                .store_existing_mut(cid)
                .expect("invariant: membership snapshot implies a live store")
                .remove(entity.id());
            debug_assert!(removed, "dense despawn: membership snapshot / remove disagree");
        }
    }
}
