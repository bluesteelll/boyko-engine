//! `InsertCommand<B>` — deferred "insert bundle `B` into existing entity".
//!
//! Phase 11 §6.3 / EC9. Constructed by
//! [`EntityCommands::insert`](crate::ecs::core::system::params::entity_commands::EntityCommands::insert).
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

#![allow(dead_code)]

use std::mem::{self, MaybeUninit};
use std::ptr::NonNull;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::bundle::Bundle;
use crate::ecs::core::commands::command::Command;
use crate::ecs::core::commands::migration_helpers::{merged_archetype_id, migrate_entity_insert};
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::hooks::dispatch::{trigger_on_insert, trigger_on_replace};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

/// Stack-collector ceiling. Matches the `MAX_BUNDLE_ARITY` used by
/// `SpawnAtCommand` / `SpawnCommand` so that any bundle the derive macro
/// accepts fits in the replace-in-place fast path's scratch.
const MAX_BUNDLE_ARITY: usize = 8;

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

        // PRE-overwrite (Q7): fire `on_replace` for each bundle component while
        // the row still holds the OLD value — the read-only view reads the
        // dying bytes. `EntityInland` still points at this row. No `&mut
        // Archetype` is live here (only `archetype_ptr`, raw).
        if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
            let world_ptr = NonNull::from(&mut *world);
            for &cid in B::component_ids() {
                trigger_on_replace(world_ptr, cid, entity);
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

        self.bundle.for_each_component_bytes(|component_id, bytes| {
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

        // POST-overwrite (Q7): fire `on_insert` for each bundle component now
        // that the row holds the NEW value — the read-only view reads the fresh
        // bytes. The closure's per-invocation `&mut *archetype_ptr` has dropped;
        // only `archetype_ptr` (raw) survives, so minting `world_ptr` aliases no
        // reborrow (SAFETY-1). `on_add` does NOT fire — the component was already
        // present (in-place replace, Q7).
        if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
            let world_ptr = NonNull::from(&mut *world);
            for &cid in B::component_ids() {
                trigger_on_insert(world_ptr, cid, entity);
            }
        }

        // Silence unused-import warnings now that the two-pass scratch is gone.
        let _ = (mem::size_of::<()>(), MaybeUninit::<()>::uninit, ComponentId(0));
    }
}
