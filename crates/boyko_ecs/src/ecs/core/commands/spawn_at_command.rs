//! `SpawnAtCommand<B>` — deferred "spawn entity with bundle `B` at a
//! pre-allocated [`Entity`]" command.
//!
//! Phase 11 §6.1 (plan Q9). Replaces the Phase 8.5 `SpawnCommand<B>`. The
//! deferred path now carries a pre-allocated `Entity` minted by
//! [`EntityCounter::reserve_entity`](crate::ecs::core::system::params::entity_counter::EntityCounter::reserve_entity)
//! at the `Commands::spawn` callsite, so the user can call
//! `.id()` synchronously before apply (EC2 + EC13). Apply delegates to
//! [`EcsMaster::create_entity_at`](crate::ecs::core::ecs_master::ecs_master::EcsMaster::create_entity_at)
//! which writes into the pre-allocated slot.
//!
//! # Cost (Phase 12.6 — collapsed inline write loop)
//!
//! The apply path skips the `[MaybeUninit<(ComponentId, &[u8])>; 8]` stack
//! scratch and the `create_entity_at_with_pool_ids` →
//! `archetype.create_entity_with_pool_ids` re-marshal chain. Per-component
//! work happens inline inside the bundle's `for_each_component_bytes`
//! callback, mirroring Bevy's `BundleSpawner::spawn_at` and the Phase 11
//! `InsertCommand::apply_replace_in_place` pattern. The lifetime trap that
//! a two-pass collect-then-iterate would create (the bundle's
//! `ManuallyDrop` locals live only for the duration of
//! `for_each_component_bytes`'s stack frame) is sidestepped by performing
//! every pool write before the closure returns.

#![allow(dead_code)]

use std::ptr::NonNull;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::bundle::Bundle;
use crate::ecs::core::commands::command::Command;
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::hooks::dispatch::{trigger_on_add, trigger_on_insert};
use crate::ecs::core::component::observers::dispatch::{
    fire_on_add_observers, fire_on_insert_observers,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::InlandPoolId;

/// Deferred "spawn entity with bundle `B` at the pre-allocated `entity`"
/// command (Phase 11 Q9 — replaces `SpawnCommand<B>`).
///
/// # Layout (plan §11.2)
///
/// ```text
/// +0  : entity: Entity   (8 B)
/// +8  : bundle: B        (sizeof B, aligned to align_of B)
/// ```
///
/// Total: `8 + sizeof::<B>()` rounded up to align_of `B`. A 3-component
/// bundle clocks in around 64 B — one cache line per queue slot.
#[repr(C)]
pub(crate) struct SpawnAtCommand<B: Bundle> {
    /// The pre-allocated entity. Captured at `Commands::spawn` callsite
    /// via `EntityCounter::reserve_entity`. The slot in
    /// `EntityMaster::entities_inland` for `entity.id().0` is NULL when
    /// the queue is built; apply populates it via `create_entity_at`.
    pub(crate) entity: Entity,

    /// The bundle, moved into the queue's bytes by `CommandQueue::push`.
    /// Drop discipline mirrors `SpawnCommand<B>`: on the `apply` path the
    /// bundle bytes are memcpy'd into archetype storage and the local
    /// stack copy never runs Drop (per-component `ManuallyDrop` is
    /// `Bundle::for_each_component_bytes`'s contract).
    pub(crate) bundle: B,
}

// SAFETY (B3 + CQ-SEND1, mirrors `SpawnCommand<B>`):
//   `B: Bundle` ⇒ `B: Send + Sync + 'static`. Therefore
//   `SpawnAtCommand<B>` is Send by composition. The explicit impl
//   documents the contract.
unsafe impl<B: Bundle> Send for SpawnAtCommand<B> {}

// SAFETY (B3): same composition as `Send`.
unsafe impl<B: Bundle> Sync for SpawnAtCommand<B> {}

impl<B: Bundle> Command for SpawnAtCommand<B> {
    /// Phase 12.6 collapsed apply: a single inline write loop mirrors
    /// Bevy's `BundleSpawner::spawn_at`. The hot path no longer materialises
    /// a `[MaybeUninit<(ComponentId, &[u8])>; 8]` scratch nor hops through
    /// `create_entity_at_with_pool_ids` → `archetype.create_entity_with_pool_ids`.
    ///
    /// Steps:
    ///
    /// 1. Resolve the destination archetype id + pointer once.
    /// 2. Resolve `pool_ids` via the per-world `BundleColumnCache` (cold
    ///    branch leaks the canonical-sorted slice; warm branch hits one
    ///    Acquire load).
    /// 3. Grow `entities_inland` to cover the pre-allocated entity slot
    ///    (dispatcher-only; SEND5 preserved).
    /// 4. Reserve archetype capacity for 1 row.
    /// 5. Inside `for_each_component_bytes` (single pass), write each
    ///    component's bytes into its pool, commit the unit, and fill its
    ///    `(added, changed)` ticks. The closure body works exclusively
    ///    with locals scoped to the bundle's stack frame — no scratch
    ///    array survives the call.
    /// 6. Push the entity id, advance `current_index`, register in the
    ///    Phase 7 fast store.
    ///
    /// # Lifetime soundness
    ///
    /// The bundle's `for_each_component_bytes` contract (B2/B4) keeps the
    /// per-component `ManuallyDrop` locals alive across every callback
    /// invocation. We perform the `pool.write_at_unchecked_initialized`
    /// memcpy + `commit_units` + `fill_ticks` INSIDE the closure, so the
    /// source `&[u8]` is live for the full memcpy. The two-pass
    /// `collect-then-iterate` shape (which the legacy
    /// `create_entity_at_with_pool_ids` chain relied on via a stack array)
    /// would dangle these slices after the bundle's frame popped —
    /// avoided here by single-pass inline work.
    fn apply(self, world: &mut EcsMaster) {
        // ── Step 1: resolve archetype id + pointer ─────────────────────
        let archetype_id = B::cached_archetype_id(world);
        let archetype_ptr = world
            .archetype_master_mut()
            .archetype_ptr_for(archetype_id)
            .expect("invariant: cached_archetype_id returns a registered id");
        let current_tick = world.current_tick();
        let entity = self.entity;

        // EC7 (debug): the pre-allocated slot must currently be NULL
        // (never registered, never spawned-at-twice). A stale `is_null()`
        // here means a SpawnAtCommand was applied twice or the user
        // smuggled an ID through `Commands::entity(reserved_id)` and
        // tried to insert before SpawnAtCommand ran.
        debug_assert!(
            world
                .entity_master
                .entities_inland
                .get(entity.id().0)
                .is_none_or(|i| i.is_null()),
            "SpawnAtCommand applied to an already-registered entity {:?}",
            entity
        );

        // ── Step 2: resolve column ids (Opt-A3 cache) ──────────────────
        // Single accessor call: `bundle_column_cache()` performs ONE
        // Acquire load on the outer OnceLock per spawn. The inner
        // `get_resolved::<B>()` performs ONE Acquire load on the per-bundle
        // slot. Cold path runs `resolve_and_cache` once per (B, world).
        //
        // Required components (Feature 1, D4): the same record carries
        // `required_missing` + `required_pool_ids` — empty `&'static []` for a
        // require-free bundle (the apply-time 0%-gate). Captured here so Step 5b
        // (the constructor pass) does not re-touch the cache.
        let (pool_ids, required_missing, required_pool_ids): (
            &'static [InlandPoolId],
            &'static [crate::ecs::core::component::component_registry::RequiredEntry],
            &'static [InlandPoolId],
        ) = {
            let cache = world.bundle_column_cache();
            let record = if let Some(r) = cache.get_resolved::<B>() {
                *r
            } else {
                // SAFETY (U1, U2, U14): `archetype_ptr` is write-capable
                //   provenance under `&mut EcsMaster`; the shared
                //   `&Archetype` view is scoped to this cold branch and
                //   dropped before the subsequent `&mut Archetype`
                //   reborrow below.
                let archetype_shared: &Archetype = unsafe { &*archetype_ptr };
                *cache.resolve_and_cache::<B>(archetype_id, archetype_shared)
            };
            (
                record.pool_ids,
                record.required_missing,
                record.required_pool_ids,
            )
        };

        // ── Step 3: grow entity fast-store on demand ──────────────────
        // Single-row growth. The apply path holds `&mut EcsMaster`, so
        // worker `&self` reads on `entities_inland` cannot race the growth
        // (SEND5) — and since Phase X.G the store's base is write-once
        // anyway (`InlandStore::ensure`: frontier commit, no realloc).
        let id_raw = entity.id().0;
        world.entity_master.entities_inland.ensure(id_raw + 1);

        // ── Step 4: reborrow archetype + reserve capacity ─────────────
        // SAFETY (U1, U2, U14): `archetype_ptr` is write-capable; we hold
        //   `&mut EcsMaster` so no aliasing reader/writer is in flight.
        //   The `archetype_shared` cold-branch borrow above (if any) has
        //   already been dropped — the if/else returned `&'static [..]`.
        let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
        archetype
            .reserve_capacity(1)
            .expect(
                "SpawnAtCommand: pool reserve ceiling (rows) exhausted — committed \
                 capacity grows on demand (Phase X.I), so this fires only when the \
                 archetype outgrows a pool's reserve_rows",
            );
        let row = archetype.current_index;

        // ── Step 5: per-component inline write ────────────────────────
        // The closure captures `archetype` (a &mut reborrow lives only
        // for the duration of the closure body), `row`, `current_tick`,
        // and `pool_ids` (Copy slice).
        //
        // canonical_idx is the iteration counter into `pool_ids` —
        // matches B2 canonical-sorted order with `B::component_ids()`.
        // Phase 12.6 lifetime contract: every pool write must complete
        // BEFORE the closure returns, since the source `&[u8]` borrows
        // from the bundle's `ManuallyDrop` locals which live only for
        // the duration of `for_each_component_bytes`'s stack frame.
        let mut canonical_idx = 0usize;
        // Phase 22 D5(4): a zero-component bundle (`Commands::spawn_empty` /
        // `EmptyBundle`) is legal — `pool_ids` is empty and the per-component
        // closure below runs zero times; `row` already comes from
        // `archetype.current_index` (line above), so the empty-archetype row
        // math is correct without it.
        debug_assert!(
            pool_ids.len() <= MAX_BUNDLE_ARITY,
            "Phase 11 arity ceiling: 0..={} (got {})",
            MAX_BUNDLE_ARITY,
            pool_ids.len(),
        );

        self.bundle.for_each_component_bytes(|_id, bytes| {
            debug_assert!(canonical_idx < pool_ids.len());
            debug_assert!(
                B::component_ids()[canonical_idx] == _id,
                "B2/SBO-B2 violation: bundle emit order mismatch at idx {}",
                canonical_idx,
            );

            let pool_idx = pool_ids[canonical_idx];
            // SAFETY (SBO13 + SBO-N + SBO-B2):
            //   - `pool_idx.0 < pools.len()` by SBO-N (push-only Vec) +
            //     the cache install-time bound check; the canonical
            //     ordering match above proves index alignment with
            //     `B::component_ids()`.
            //   - `row < committed_rows` post `reserve_capacity(1)`
            //     (Phase X.I: Phase B grew every pool).
            //   - `&mut archetype` provides exclusive access; no
            //     concurrent reader of this slot exists.
            //   - `bytes.len() == pool.component_layout().size()` by
            //     Bundle/macro contract (B/B2).
            unsafe {
                let pool = archetype
                    .component_pools_mut()
                    .pool_at_unchecked_mut(pool_idx);
                pool.write_at_unchecked_initialized(row, bytes);
                pool.commit_units(row, 1);
                pool.fill_ticks(row, 1, current_tick);
            }
            canonical_idx += 1;
        });
        debug_assert_eq!(
            canonical_idx,
            pool_ids.len(),
            "Bundle invoked for_each_component_bytes {} times, expected {}",
            canonical_idx,
            pool_ids.len(),
        );

        // ── Step 5b: required-component constructor pass (Feature 1, D5) ──
        // For each transitively-required component the bundle did NOT supply,
        // construct one value via its capture-free ctor directly into the
        // reserved-but-uncommitted slot at `row`, commit it, and fill its ticks.
        // The spawn fire (Step 8) iterates the FULL archetype `component_ids`, so
        // these constructed columns fire on_add/on_insert automatically there —
        // no second fire here (C1: spawn-path fire already covers the full
        // archetype). For a require-free bundle `required_missing` is empty and
        // this loop runs zero iterations (the 0%-gate).
        debug_assert_eq!(
            required_missing.len(),
            required_pool_ids.len(),
            "required_missing / required_pool_ids length mismatch",
        );
        for (entry, &pool_idx) in required_missing.iter().zip(required_pool_ids.iter()) {
            // SAFETY (mirrors the bundle write above; Feature 1 D5):
            //   - `pool_idx.0 < pools.len()` — resolved at cache install time
            //     against the same archetype (`resolve_required_missing`).
            //   - `row < committed_rows` post `reserve_capacity(1)` (Phase X.I).
            //   - `&mut archetype` provides exclusive access; no concurrent
            //     reader of this slot exists.
            //   - `entry.ctor` writes exactly one value of the pool's registered
            //     type (the registry paired the ctor with `entry.component_id`,
            //     and `pool_idx` is that id's column).
            unsafe {
                let pool = archetype
                    .component_pools_mut()
                    .pool_at_unchecked_mut(pool_idx);
                pool.construct_at_uninitialized(row, entry.ctor);
                pool.commit_units(row, 1);
                pool.fill_ticks(row, 1, current_tick);
            }
        }

        // ── Step 6: archetype-side bookkeeping ────────────────────────
        archetype.entity_ids.push(entity.id());
        archetype.current_index = row + 1;

        // ── Step 7: fast-store registration ────────────────────────────
        world
            .entity_master
            .register_entity_with_ptr(entity, archetype_ptr, row as u32);

        // ── Step 8 (Phase 14a §3.1): fire on_add / on_insert hooks ──────
        // The closure's per-invocation `&mut *archetype_ptr` (Step 5) dropped
        // at the closure return; the Step-6 field writes used the same `&mut`
        // (last use `current_index = row + 1`). Here we hold ONLY `archetype_ptr`
        // (*mut, Copy) and `entity` (Copy) — no `world`-derived `&mut Archetype`
        // is live, so minting `world_ptr` aliases no reborrow (SAFETY-1).
        //
        // SAFETY: `archetype_ptr` is write-capable + stable slab provenance;
        //   reading `flags` is one `u16` load (no `&mut` taken).
        let flags = unsafe { (*archetype_ptr).flags };
        if !flags.is_empty() {
            // MINT: at this point no `world`-derived `&mut` is live.
            let world_ptr = NonNull::from(&mut *world);
            // Ordering (SAFETY-2): ALL on_add, THEN ALL on_insert (Bevy bundle
            // order — add-before-insert across the whole bundle, not interleaved).
            // Observers fire in the same window as their matching hook (hooks
            // first, then observers over the SAME `component_ids` slice).
            if flags.contains(ArchetypeFlags::ON_ADD_ANY) {
                // SAFETY: `archetype_ptr` is a valid `*const Archetype`; the
                //   shared `&[ComponentId]` is transient and not aliased by any
                //   live `&mut` (the hooks/observers receive `world_ptr`, not the
                //   slice).
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
                    for &cid in ids {
                        trigger_on_add(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_ADD_OBSERVER) {
                    for &cid in ids {
                        fire_on_add_observers(world_ptr, cid, entity);
                    }
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
                // SAFETY: same as the on_add slice read above.
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                    for &cid in ids {
                        trigger_on_insert(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                    for &cid in ids {
                        fire_on_insert_observers(world_ptr, cid, entity);
                    }
                }
            }
        }
        // NO drain here (Q-A1 / C1): this command runs at depth >= 1 (the
        // per-system `CommandQueue::apply` bracket); the outermost schedule
        // drive drains after the apply returns.
    }
}

/// Mirrors the Phase 8.5 `SpawnCommand::apply` ceiling (`SBC10`). Kept
/// in step with the derive macro's per-bundle arity check.
/// Phase 22: kept in lock-step with the derive macro's ceiling (16).
const MAX_BUNDLE_ARITY: usize = 16;
