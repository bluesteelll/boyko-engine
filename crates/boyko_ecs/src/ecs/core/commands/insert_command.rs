//! `InsertCommand<B>` — deferred "insert bundle `B` into existing entity".
//!
//! Phase 11 §6.3 / EC9. Constructed by
//! `EntityCommands::insert`.
//! Apply chooses one of two paths based on the canonicalization invariant
//! (plan §7.4 / W-N1):
//!
//! * **Replace-in-place fast path**: `target.mask == source.mask` ⇒
//!   `bundle ⊆ source`. Per-component `drop_at + write_at + bump
//!   changed_tick`. ~90 ns / 3-component bundle.
//! * **Migration**: `target != source`. Allocates / reuses target
//!   archetype, memcpys retained bytes via `create_entity_with_ticks`,
//!   releases source row via `move_out_entity`. ~590 ns / 3-component
//!   bundle, warm path.

use std::mem::{self, MaybeUninit};
use std::ptr::NonNull;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::bundle::Bundle;
use crate::ecs::core::commands::command::Command;
use crate::ecs::core::commands::migration_helpers::{merged_archetype_id, migrate_entity_insert};
use crate::ecs::core::component::component_registry::{self, StorageKind};
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::hooks::dispatch::{
    trigger_on_add, trigger_on_insert, trigger_on_replace,
};
use crate::ecs::core::component::observers::dispatch::{
    fire_on_add_observers, fire_on_insert_observers, fire_on_replace_observers,
};
use crate::ecs::core::component::observers::ObserverKind;
use crate::ecs::core::component::observers::entity_store::fire_entity_observers;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

/// Stack capacity for the dense-fire scratch (Dense plan D2). Mirrors the
/// derive macro's per-bundle arity ceiling and the sibling `MAX_BUNDLE_ARITY`
/// constants in `spawn_at_command.rs` / `migration_helpers.rs`.
const MAX_BUNDLE_ARITY: usize = 16;

/// Deferred "insert bundle `B` into existing entity" command.
///
/// # Layout (plan §11.3)
///
/// `8 + sizeof::<B>()` — identical to `SpawnAtCommand<B>`.
#[repr(C)]
pub(crate) struct InsertCommand<B: Bundle> {
    pub(crate) entity: Entity,
    pub(crate) bundle: B,
}

// SAFETY (mirrors `SpawnAtCommand` / B3): `B: Bundle ⇒ Send + Sync`.
unsafe impl<B: Bundle> Send for InsertCommand<B> {}
unsafe impl<B: Bundle> Sync for InsertCommand<B> {}

impl<B: Bundle> Command for InsertCommand<B> {
    fn apply(self, world: &mut EcsMaster) {
        let entity = self.entity;

        // Resolve source archetype id via the fast inland.
        let inland = match world.entity_master.entities_inland.get(entity.id().0) {
            Some(slot) => *slot,
            None => {
                debug_assert!(false, "InsertCommand::apply: entity {:?} never registered", entity);
                return; // EC8 silent no-op in release
            }
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            debug_assert!(false, "InsertCommand::apply: stale entity {:?}", entity);
            return;
        }
        // SAFETY (U1, U2, U11, F1): `archetype_ptr` is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — it survives sibling
        //   structural writes under TB/SB (the whole slab element is
        //   `UnsafeCell`-wrapped). Non-null + generation-matched above, so live.
        let source_archetype_id = unsafe { (*inland.archetype_ptr()).id() };

        // Compute the merged archetype id (canonical sort + dedup).
        let target_archetype_id = merged_archetype_id::<B>(world, source_archetype_id);

        if target_archetype_id == source_archetype_id {
            // Plan §7.4 W-N1 fast path: same archetype ⇒ `bundle ⊆ source`.
            self.apply_replace_in_place(world);
        } else {
            // Migrating path. Hand `self.bundle` by value.
            migrate_entity_insert::<B>(
                world,
                entity,
                source_archetype_id,
                target_archetype_id,
                self.bundle,
            );
        }
    }
}

impl<B: Bundle> InsertCommand<B> {
    /// Replace-in-place fast path (plan §7.4 / W-N1).
    ///
    /// Pre-condition: caller has verified `target_archetype_id ==
    /// source_archetype_id`. Under the canonicalization invariant cited
    /// in plan §7.4 (`ArchetypeRegistry::find_exact_match` is exact-mask
    /// match), this implies `bundle ⊆ source` — so every bundle
    /// component's `get_pool_mut(component_id)` lookup is guaranteed to
    /// succeed.
    #[cold]
    #[inline(never)]
    fn apply_replace_in_place(self, world: &mut EcsMaster) {
        let current_tick = world.current_tick();
        let entity = self.entity;
        let entity_id = entity.id().0;

        // Re-resolve inland for the archetype_ptr (we dropped the borrow
        // back at the dispatch site).
        let inland = world.entity_master.entities_inland[entity_id];
        debug_assert!(
            !inland.is_null() && inland.generation() == entity.generation(),
            "apply_replace_in_place: re-resolution of stale entity"
        );
        let archetype_ptr = inland.archetype_ptr();
        let row = inland.unit_index() as usize;

        // Dense plan D2 / decision 3 — `is_dense(cid)` filters dense bundle ids
        // out of the TABLE replace path (a dense id has no archetype pool, so its
        // table-flag-gated fire + `get_pool_mut` are wrong/absent). `has_dense`
        // gates the dense PRE-on_replace probe + the closure dense branch + the
        // POST dense on_add/on_insert. For a table-only bundle `has_dense` is
        // false and every dense branch folds out (the 0%-gate).
        let is_dense = |cid: ComponentId| {
            matches!(component_registry::storage_kind(cid.0), StorageKind::Dense)
        };
        let has_dense = B::component_ids().iter().copied().any(is_dense);

        // Phase 14a §3.3 / O3 / Q7: read the archetype flags once. The
        // overwrite loop below confines its per-invocation `&mut *archetype_ptr`
        // to each closure call, so no `world`-derived `&mut Archetype` is live
        // when we mint `world_ptr` (SAFETY-1).
        //
        // SAFETY (F1): `archetype_ptr` is write-capable, stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — it survives sibling
        //   structural writes under TB/SB (the whole slab element is
        //   `UnsafeCell`-wrapped). Reading `flags` is one `u16` load (no `&mut`).
        let flags = unsafe { (*archetype_ptr).flags };

        // Dense plan D2 — the entity's current archetype id, seeded into each
        // dense store's `arch_presence` on the closure's dense route. Read once
        // here (a `usize` load through the raw deref; no `&mut` taken).
        // SAFETY (F1): `archetype_ptr` is interior-mutable, stable slab provenance.
        let source_archetype_id_for_dense = unsafe { (*archetype_ptr).id() };

        // Dense plan D2 — PRE-overwrite on_replace for each PRESENT dense bundle
        // id (the dense value is still old). NOT gated by the archetype's
        // `ON_REPLACE_ANY` flag (dense ids are not in the signature); the
        // `trigger`/`fire` self-gate. An absent dense id fires on_add later (POST)
        // instead, never on_replace. 0%-gated by `has_dense`.
        if has_dense {
            let world_ptr = NonNull::from(&mut *world);
            for &cid in B::component_ids() {
                if !is_dense(cid) {
                    continue;
                }
                let present = world
                    .dense_registry
                    .store(cid)
                    .is_some_and(|s| s.contains(entity.id()));
                if present {
                    trigger_on_replace(world_ptr, cid, entity);
                    fire_on_replace_observers(world_ptr, cid, entity);
                }
            }
        }

        // PRE-overwrite (Q7): fire `on_replace` for each TABLE bundle component
        // while the row still holds the OLD value — the read-only view reads the
        // dying bytes. `EntityInland` still points at this row. No `&mut
        // Archetype` is live here (only `archetype_ptr`, raw). Dense ids are
        // skipped (handled above).
        if flags.contains(ArchetypeFlags::ON_REPLACE_ANY) {
            let world_ptr = NonNull::from(&mut *world);
            if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
                for &cid in B::component_ids() {
                    if is_dense(cid) {
                        continue;
                    }
                    trigger_on_replace(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_REPLACE_OBSERVER) {
                for &cid in B::component_ids() {
                    if is_dense(cid) {
                        continue;
                    }
                    fire_on_replace_observers(world_ptr, cid, entity);
                }
            }
        }
        // Feature 2 — entity-targeted on_replace observers (in-place overwrite),
        // gated by the archetype's sticky HAS_ENTITY_OBSERVER bit. Dense ids are
        // skipped (their entity-targeted fires are out of D2 scope; D2 routes
        // dense component-level hooks/observers only).
        if flags.contains(ArchetypeFlags::HAS_ENTITY_OBSERVER) {
            let world_ptr = NonNull::from(&mut *world);
            for &cid in B::component_ids() {
                if is_dense(cid) {
                    continue;
                }
                fire_entity_observers(world_ptr, ObserverKind::Replace, cid, entity);
            }
        }

        // BUG FIX (Phase 11 follow-up): the prior two-pass approach
        // (collect slots, then iterate) stored `&[u8]` slices that became
        // DANGLING after `for_each_component_bytes` returned — the
        // bundle's `ManuallyDrop` locals are owned by that function's
        // stack frame, not by `self`. Stack frame reuse caused random
        // bytes to be observed where the original component data lived.
        //
        // FIX: do all the pool work INSIDE the FnMut closure. The
        // `ManuallyDrop` locals live for the entire duration of
        // `for_each_component_bytes(self, ...)` — i.e. throughout the
        // closure's invocations — so `bytes` is valid AT THE MOMENT
        // we call `pool.write_at(row, bytes)`.
        //
        // Soundness: the closure captures `archetype_ptr` (raw) and
        // `current_tick` (Copy) and `row` (Copy). On each invocation it
        // reborrows the archetype via `&mut *archetype_ptr`. The
        // reborrow lives only for that single invocation, then drops —
        // so successive callbacks each get a fresh `&mut Archetype`
        // without any cross-call borrow overlap.

        // Dense plan D2 — record dense bundle ids inserted in this replace, with
        // whether each was newly added (absent before), so the POST window fires
        // on_add (newly) + on_insert (all). The dense store op happens INSIDE the
        // closure where `bytes` is live; the fire is deferred to the POST window.
        // Reach `world.dense_registry` from the closure via this separate
        // `&mut *world` capture (`archetype_ptr` is a disjoint raw reborrow).
        let world_ref: &mut EcsMaster = world;
        let mut dense_fire_buf = [(ComponentId(0), false); MAX_BUNDLE_ARITY];
        let mut dense_fire_n = 0usize;
        let entity_id_for_dense = entity.id();

        self.bundle.for_each_component_bytes(|component_id, bytes| {
            // Dense plan D2 / decision 3: a dense bundle id has NO archetype pool
            // — route it to its `DenseStore` (insert-or-replace at a stable slot,
            // no migration), record the id + newly-added flag for the POST fire,
            // and skip the pool path entirely. For a table-only bundle this branch
            // is never taken (the 0%-gate).
            if matches!(component_registry::storage_kind(component_id.0), StorageKind::Dense) {
                let store = world_ref.dense_registry.store_mut(component_id);
                let newly_added =
                    store.insert_or_replace(entity_id_for_dense, bytes, current_tick);
                store.mark_arch_present(source_archetype_id_for_dense);
                debug_assert!(dense_fire_n < MAX_BUNDLE_ARITY);
                dense_fire_buf[dense_fire_n] = (component_id, newly_added);
                dense_fire_n += 1;
                return;
            }

            // SAFETY (U1, U2, U14, SCH7, F1):
            //   * archetype_ptr is write-capable, stable, interior-mutable
            //     (`SharedReadWrite`, F4-rooted) slab provenance — it survives
            //     sibling structural writes under TB/SB (whole slab element is
            //     `UnsafeCell`-wrapped).
            //   * &mut EcsMaster (held by caller) ⇒ no sibling reader.
            //   * The &mut Archetype reborrow is scoped to this closure
            //     invocation only — it does NOT survive across calls.
            let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };

            debug_assert!(
                archetype.component_pools().has_pool(component_id),
                "W-N1: bundle component {:?} absent from source archetype despite \
                 canonicalization invariant — get_or_create_archetype regression?",
                component_id
            );

            let pool = archetype
                .component_pools_mut()
                .get_pool_mut(component_id)
                .expect(
                    "invariant: target == source ⇒ bundle ⊆ source (canonicalization, plan §7.4)",
                );
            debug_assert!(pool.has_row(row), "replace-in-place row out of bounds");

            // SAFETY (plan §7.4):
            //   * `row < pool.count()` (debug-asserted via `has_row`).
            //   * Exclusive `&mut pool` for this invocation.
            //   * `drop_at(row)` runs the old destructor; slot becomes
            //     logically uninit. `write_at(row, bytes)` re-initialises
            //     from the still-alive ManuallyDrop bytes (lifetime
            //     bound to `for_each_component_bytes`'s stack frame).
            //   * `write_changed_tick(row, current_tick)` stamps STORE3.
            //   * `added_tick` intentionally NOT bumped (EC9 / OQ5).
            unsafe {
                pool.drop_at(row);
                pool.write_at(row, bytes);
                pool.write_changed_tick(row, current_tick);
            }
        });

        // POST-overwrite (Q7): fire `on_insert` for each TABLE bundle component
        // now that the row holds the NEW value. The closure's per-invocation
        // `&mut *archetype_ptr` has dropped (and the `world_ref` dense capture
        // ended at the closure return), so minting `world_ptr` aliases no reborrow
        // (SAFETY-1). `on_add` does NOT fire for table — the component was already
        // present (in-place replace, Q7). Dense ids are skipped (handled below).
        if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
            let world_ptr = NonNull::from(&mut *world);
            if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                for &cid in B::component_ids() {
                    if is_dense(cid) {
                        continue;
                    }
                    trigger_on_insert(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                for &cid in B::component_ids() {
                    if is_dense(cid) {
                        continue;
                    }
                    fire_on_insert_observers(world_ptr, cid, entity);
                }
            }
        }
        // Feature 2 — entity-targeted on_insert observers (in-place overwrite),
        // gated by the archetype's sticky HAS_ENTITY_OBSERVER bit. Dense skipped.
        if flags.contains(ArchetypeFlags::HAS_ENTITY_OBSERVER) {
            let world_ptr = NonNull::from(&mut *world);
            for &cid in B::component_ids() {
                if is_dense(cid) {
                    continue;
                }
                fire_entity_observers(world_ptr, ObserverKind::Insert, cid, entity);
            }
        }

        // Dense plan D2 — POST dense fires: on_add for every NEWLY-added dense id
        // (absent before this insert), then on_insert for ALL inserted dense ids
        // (newly-added OR replaced). Mirrors the table on_add/on_insert ordering;
        // a replaced dense id already fired on_replace PRE-overwrite above. NOT
        // gated by archetype flags (dense ids are not in the signature); the
        // `trigger`/`fire` self-gate. 0%-gated by `dense_fire_n`.
        if dense_fire_n != 0 {
            let world_ptr = NonNull::from(&mut *world);
            for &(cid, newly_added) in &dense_fire_buf[..dense_fire_n] {
                if newly_added {
                    trigger_on_add(world_ptr, cid, entity);
                    fire_on_add_observers(world_ptr, cid, entity);
                }
            }
            for &(cid, _) in &dense_fire_buf[..dense_fire_n] {
                trigger_on_insert(world_ptr, cid, entity);
                fire_on_insert_observers(world_ptr, cid, entity);
            }
        }

        // Silence unused-import warnings now that the two-pass scratch is gone.
        let _ = (mem::size_of::<()>(), MaybeUninit::<()>::uninit, ComponentId(0));
    }
}
