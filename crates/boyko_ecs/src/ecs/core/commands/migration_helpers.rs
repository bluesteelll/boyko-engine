//! Shared low-level migration scaffolding for `InsertCommand` /
//! `RemoveCommand` (Phase 11 §7).
//!
//! Two flavours of archetype change:
//!
//! * [`migrate_entity_insert`] — source ∪ bundle. If `merged_archetype_id ==
//!   source_archetype_id` (canonicalization invariant cited from
//!   `archetype_master.rs:99-133, 462-473`), the in-place replace fast path
//!   [`apply_replace_in_place`] is taken — `bundle ⊆ source`.
//! * [`migrate_entity_remove`] — source \\ `{C}`. Single-component remove;
//!   absent-C is a silent no-op (W1 — Bevy Issue #10166).
//!
//! Both paths use the existing dense byte-buffer `ComponentPool`
//! storage (Round 3 C-N2): retained bytes are extracted via
//! `ComponentPool::unit_ptr(source_row)` (the computed `buffer + row*stride`) +
//! `from_raw_parts`, copied into the target via
//! `Archetype::create_entity_with_ticks`, and the source row is released
//! via `Archetype::move_out_entity` (no drop on retained or removed slots —
//! the caller owns byte tracking per W-N2).

#![allow(dead_code)]

use std::mem::{self, MaybeUninit};
use std::ptr::NonNull;

use crate::ecs::core::archetype::archetype::{Archetype, RemoveOutcome};
use crate::ecs::core::bundle::Bundle;
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::hooks::dispatch::{
    trigger_on_add, trigger_on_insert, trigger_on_remove, trigger_on_replace,
};
use crate::ecs::core::component::observers::dispatch::{
    fire_on_add_observers, fire_on_insert_observers, fire_on_remove_observers,
    fire_on_replace_observers,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, InlandPoolId};

/// Stack capacity for the retained / combined slot array in
/// `migrate_entity_insert` / `migrate_entity_remove` (plan §7.2).
///
/// Set to `MAX_COMPONENTS` so any archetype the engine supports fits on
/// the stack without spilling into `EcsMaster::migration_scratch`.
const MAX_MIGRATION_COLUMNS: usize = MAX_COMPONENTS;

/// Stack capacity for the bundle slot array — mirrors
/// `MAX_BUNDLE_ARITY` from `SpawnAtCommand` / `SpawnCommand`. Bundles
/// larger than this are rejected at the derive macro layer.
/// Phase 22: kept in lock-step with the derive macro's ceiling (16).
const MAX_BUNDLE_ARITY: usize = 16;

/// Resolves the `(source, target)` archetype-id pair for an insert
/// migration (plan §6.3). Falls back to `get_or_create_archetype` for the
/// merged set; uses the on-stack scratch when the union fits.
///
/// `out_target` is the destination archetype id. The function does NOT
/// guarantee `source != target` — the caller checks for the in-place
/// fast path (plan §7.4 / W-N1).
#[inline]
pub(crate) fn merged_archetype_id<B: Bundle>(
    world: &mut EcsMaster,
    source_archetype_id: ArchetypeId,
) -> ArchetypeId {
    let bundle_ids = B::component_ids();
    let source_ids = world
        .archetype_master()
        .get_archetype(source_archetype_id)
        .expect("invariant: source_archetype_id is live (resolved from EntityInland)")
        .component_ids()
        .to_vec();

    // Stack scratch: union with stable canonical order. Capped at
    // MAX_MIGRATION_COLUMNS — wider archetypes are out of scope for
    // Phase 11 (plan §7.6).
    let mut combined: [ComponentId; MAX_MIGRATION_COLUMNS] =
        [ComponentId(0); MAX_MIGRATION_COLUMNS];
    let mut len = 0usize;

    // Seed with the source ids (already canonical-sorted per
    // `Archetype::create_by_ids` / `Bundle::component_ids` contract).
    for &cid in source_ids.iter() {
        debug_assert!(
            len < MAX_MIGRATION_COLUMNS,
            "migration union exceeds MAX_COMPONENTS"
        );
        combined[len] = cid;
        len += 1;
    }

    // Union: insert bundle ids if absent. O(B × S) is fine — typical
    // bundles have ≤ 8 components, archetypes have ≤ 32.
    for &cid in bundle_ids {
        if !combined[..len].contains(&cid) {
            debug_assert!(
                len < MAX_MIGRATION_COLUMNS,
                "migration union exceeds MAX_COMPONENTS"
            );
            combined[len] = cid;
            len += 1;
        }
    }

    // Canonical-sort the combined set so `get_or_create_archetype`'s
    // `find_exact_match(ComponentMask::from_components(...))` collapses
    // equivalent unions to the same ArchetypeId regardless of insertion
    // order (B1 + canonicalization, plan §7.4).
    combined[..len].sort_unstable_by_key(|c| c.0);

    world.get_or_create_archetype(&combined[..len])
}

/// Resolves the destination archetype id for a single-component remove
/// (plan §6.4). Returns `None` if the source archetype does not host `C`
/// (W1 — Bevy Issue #10166: absent-C is a no-op, NOT a debug_assert).
#[inline]
pub(crate) fn without_component_archetype_id<C: Component>(
    world: &mut EcsMaster,
    source_archetype_id: ArchetypeId,
) -> Option<ArchetypeId> {
    let removed_id = C::component_id();
    let source = world
        .archetype_master()
        .get_archetype(source_archetype_id)
        .expect("invariant: source_archetype_id is live (resolved from EntityInland)");

    if !source.component_ids().contains(&removed_id) {
        return None; // W1: silent no-op for absent component
    }

    let kept: Vec<ComponentId> = source
        .component_ids()
        .iter()
        .filter(|&&cid| cid != removed_id)
        .copied()
        .collect();

    // Phase 22 D5(2): removing the only component routes the entity to the
    // EMPTY archetype — `kept` is empty and `get_or_create_archetype(&[])`
    // lazily creates (and thereafter exact-mask-caches) the zero-component
    // archetype through the same funnel as any other migration target.
    // (Phase 11 shipped a debug_assert + bail here, silently no-op'ing the
    // removal.) The subsequent `migrate_entity_remove` then runs as an
    // ordinary migration: zero retained columns, hooks/observers fire on the
    // dying source row exactly as for any other remove.
    Some(world.get_or_create_archetype(&kept))
}

/// Performs the insert migration `source → target` over the existing
/// dense byte-buffer storage (Round 3 C-N2, plan §7.2). Caller
/// guarantees `source_archetype_id != target_archetype_id`.
///
/// The retained components' bytes are memcpy'd into the target via
/// `create_entity_with_ticks`; the source row is released via
/// `move_out_entity` (no drop on retained or unaffected slots). Bundle
/// components override retained on overlap (Q6 — replace semantic).
#[cold]
#[inline(never)]
pub(crate) fn migrate_entity_insert<B: Bundle>(
    world: &mut EcsMaster,
    entity: Entity,
    source_archetype_id: ArchetypeId,
    target_archetype_id: ArchetypeId,
    bundle: B,
) {
    debug_assert_ne!(
        source_archetype_id, target_archetype_id,
        "migrate_entity_insert: caller must filter the in-place replace fast path"
    );

    let current_tick = world.current_tick();

    let source_ptr = world
        .archetype_master_mut()
        .archetype_ptr_for(source_archetype_id)
        .expect("invariant: source archetype exists");
    let target_ptr = world
        .archetype_master_mut()
        .archetype_ptr_for(target_archetype_id)
        .expect("invariant: target archetype just resolved");

    let inland = world.entity_master.entities_inland[entity.id().0];
    debug_assert!(
        !inland.is_null() && inland.generation() == entity.generation(),
        "migrate_entity_insert: stale entity passed (caller must filter)"
    );
    let source_row = inland.unit_index() as usize;

    // Phase 14a §3.4 / NEW-2 (P3): capture the bundle's component-id set and a
    // per-id "newly-added" flag (== `cid` NOT in source). `T == S ∪ I`, so
    // "target ids not in source" == "bundle ids not in source" — capturing per
    // bundle id is sufficient and avoids a 512-bit mask scan. Filled inside the
    // Phase-1 block (Step 2) while `source` is &mut-live; read in Phase 2.
    let mut bundle_ids = [ComponentId(0); MAX_BUNDLE_ARITY];
    let mut bundle_added = [false; MAX_BUNDLE_ARITY];
    let mut bundle_id_count = 0usize;

    // Phase 14a §3.4 (C2): the entire `source` / `target` `&mut` lifetime is
    // confined to this Phase-1 block. The Step-6 `EntityInland` repoint is
    // HOISTED INTO it (it touches `world.entity_master`, not the archetypes), so
    // after the block the entity is fully in `target` and BOTH `&mut Archetype`
    // are dead — Phase 2 can mint `world_ptr` with no live reborrow (SAFETY-1).
    {
        // SAFETY (U1, U2, U14, SCH7, F1):
        //   * `source_ptr` / `target_ptr` carry write-capable, interior-mutable
        //     (`SharedReadWrite`, F4-rooted) provenance minted under `&mut self`
        //     via the bundle's `UnsafeCell::raw_get` helper — each survives
        //     sibling structural writes (incl. the OTHER archetype's
        //     `current_index` bump) under TB/SB because every slab element is a
        //     distinct `UnsafeCell`.
        //   * `source != target` (debug-asserted), so the two `&mut Archetype`
        //     reborrows alias disjoint slots.
        //   * `&mut EcsMaster` exclusivity prevents any sibling reader (SCH7).
        //   * The reborrows are confined to this block (Phase 1).
        let source: &mut Archetype = unsafe { &mut *source_ptr };
        let target: &mut Archetype = unsafe { &mut *target_ptr };

        // NEW-1 (use-after-free fix): the prior shape collected the bundle's
        // `&[u8]` slices into a stack array inside `for_each_component_bytes`'s
        // closure and read them back AFTER the closure returned (Steps 2-4 +
        // `create_entity_with_ticks`). Those slices borrow the bundle's
        // `ManuallyDrop` locals, which live ONLY for the duration of
        // `for_each_component_bytes`'s stack frame (macro contract,
        // `boyko_macros/src/lib.rs:1062`) — reading them afterwards is a
        // dangling-reference UAF (Miri-TB).
        //
        // FIX (mirrors `SpawnAtCommand::apply` + `apply_replace_in_place`):
        // consume every `&[u8]` AT THE POINT IT IS LIVE. Reserve one row in
        // `target`, write the RETAINED bytes (live in `source`) into the row,
        // then — INSIDE the closure — write each bundle component's bytes
        // straight into the row's `target` pool (bundle wins on overlap, Q6).
        // No `&[u8]` from the closure survives the call. The per-pool dense
        // index stays in lockstep: every `target` pool ends with one extra
        // committed row at `row` (retained pools committed in the loop below,
        // bundle-only pools committed in the closure).
        target
            .reserve_capacity(1)
            .expect(
                "insert-migration: target pool reserve ceiling (rows) exhausted — \
                 committed capacity grows on demand (Phase X.I), so this fires only \
                 when the target archetype outgrows a pool's reserve_rows",
            );
        let new_row: u32 = target.current_index as u32;
        let row = target.current_index;

        // Step 1: write retained components (present in BOTH source and target)
        // into the reserved target row, copying directly from the live source
        // pool. The source `&[u8]` is valid throughout this loop (it borrows the
        // live `source` pool), so the memcpy completes before any aliasing
        // mutation of `source`.
        let target_cids: Vec<ComponentId> = target.component_ids().to_vec();
        for target_cid in target_cids.iter().copied() {
            if !source.component_ids().contains(&target_cid) {
                continue;
            }
            let src_pool = source
                .component_pools()
                .get_pool(target_cid)
                .expect("invariant: retained component must exist in source");
            debug_assert!(
                source_row < src_pool.count(),
                "source_row out of bounds for retained component"
            );
            let stride = src_pool.component_layout().size();
            // SAFETY (Round 3 C-N2 / Phase 10 STORE3):
            //   * `source_row < src_pool.count()` (debug-asserted) ⇒
            //     `unit_ptr(source_row)` addresses an initialized arena slot.
            //   * The slice borrows the live `source` pool (held via `&source`);
            //     it is consumed by the `write_at_unchecked_initialized` memcpy
            //     in this same iteration, before `source` is mutated (Step 5).
            //   * `&mut source` ⇒ no concurrent writer; the tick sub-regions
            //     are committed for every row `< src_pool.count()` (Phase X.I).
            let bytes =
                unsafe { core::slice::from_raw_parts(src_pool.unit_ptr(source_row), stride) };
            let added = unsafe { src_pool.read_added_tick(source_row) };
            let changed = unsafe { src_pool.read_changed_tick(source_row) };

            let dst_pool = target
                .component_pools_mut()
                .get_pool_mut(target_cid)
                .expect("invariant: retained component must exist in target");
            // SAFETY (mirrors `SpawnAtCommand::apply`):
            //   * `row == target.current_index == dst_pool.count()` (pools grow
            //     in lockstep) and `reserve_capacity(1)` guaranteed a committed
            //     slot (Phase X.I Phase B), so `row < dst_pool committed_rows` —
            //     `write_at_unchecked_initialized` targets a logically-uninit
            //     slot (no drop runs).
            //   * `bytes.len() == stride == dst_pool.component_layout().size()`
            //     (same `ComponentId` ⇒ same registry layout).
            //   * `&mut target` ⇒ exclusive access; `commit_units(row, 1)`
            //     extends the dense tail by one (pre: `row == count`), after
            //     which `write_added_tick`/`write_changed_tick` stamp the
            //     ORIGINAL source ticks into the now-live slot.
            unsafe {
                dst_pool.write_at_unchecked_initialized(row, bytes);
                dst_pool.commit_units(row, 1);
                dst_pool.write_added_tick(row, added);
                dst_pool.write_changed_tick(row, changed);
            }
        }

        // Step 2 + 3 (fused): write the bundle components into the same target
        // row. ALL bundle-byte consumption happens INSIDE the closure, where the
        // bundle's `ManuallyDrop` locals are alive (NEW-1). Bundle wins on
        // overlap (Q6): a retained pool already committed `row` in Step 1 with a
        // bitwise copy of the source value, so we `drop_at` that displaced value
        // (issue #55 leak fix — mirrors `apply_replace_in_place`) and then
        // overwrite via `write_at`; a bundle-only (newly-added) pool has not yet
        // reached `row`, so we `write_at_unchecked_initialized` + `commit_units`
        // to extend it.
        bundle.for_each_component_bytes(|id, bytes| {
            // Phase 14a P3: record the bundle id + whether it is newly-added
            // (NOT already in source). This read is against the pre-migration
            // `source` row (Step 5 `move_out_entity` has not run yet).
            debug_assert!(bundle_id_count < MAX_BUNDLE_ARITY);
            bundle_ids[bundle_id_count] = id;
            bundle_added[bundle_id_count] = !source.component_ids().contains(&id);
            bundle_id_count += 1;

            let dst_pool = target
                .component_pools_mut()
                .get_pool_mut(id)
                .expect("invariant: bundle component must exist in target (T = S ∪ I)");
            if dst_pool.has_row(row) {
                // Overlap: the retained pass (Step 1) committed `row` with a
                // bitwise copy of the SOURCE value. Drop that displaced value,
                // then overwrite the now-uninit slot with the bundle bytes
                // (bundle wins, Q6) — mirroring
                // `InsertCommand::apply_replace_in_place` (`insert_command.rs`
                // ~160-209: `drop_at; write_at; write_changed_tick`).
                // SAFETY (no-double-free):
                //   * `dst_pool.has_row(row)` ⇒ `row < dst_pool.count()`, which
                //     is the precondition of BOTH `drop_at` and `write_at`.
                //   * `drop_at(row)` runs the component's type-erased `drop_fn`
                //     (= `drop_in_place::<T>`) on the displaced (source-copy)
                //     value. The Step-1 copy and the source original are the
                //     SAME byte image (a bitwise copy shares any owned heap), so
                //     this single drop frees that shared heap EXACTLY once. It
                //     logically uninitialises the slot.
                //   * `write_at(row, bytes)` is a memcpy with NO drop into the
                //     now-logically-uninit slot ⇒ no double-drop of the old
                //     value. It re-initialises the row with the bundle value.
                //   * Step 5 (`move_out_entity`) releases the SOURCE bytes
                //     WITHOUT drop (W-N2): the source slot is bitwise-released
                //     and never deref'd, so the already-freed heap is NOT
                //     double-freed. Net: the old value is dropped exactly once
                //     (here); the new bundle value is dropped exactly once later
                //     when `target`'s row is removed.
                //   * `bytes.len() == dst_pool.component_layout().size()` by the
                //     Bundle/macro contract (B/B2).
                //   * `&mut target` ⇒ exclusive access. `bytes` is live — it
                //     borrows the bundle's `ManuallyDrop` locals inside this
                //     closure invocation.
                unsafe {
                    dst_pool.drop_at(row);
                    dst_pool.write_at(row, bytes);
                    dst_pool.write_added_tick(row, current_tick);
                    dst_pool.write_changed_tick(row, current_tick);
                }
            } else {
                // Newly-added: this pool has not reached `row` yet (Step 1 skips
                // bundle-only components). Extend the dense tail by one.
                // SAFETY (mirrors `SpawnAtCommand::apply`):
                //   * `row == dst_pool.count()` (lockstep with the other target
                //     pools) and `reserve_capacity(1)` guaranteed a committed
                //     slot (Phase X.I Phase B) ⇒
                //     `write_at_unchecked_initialized` targets a logically-uninit
                //     slot (no drop). `commit_units(row, 1)` extends the tail
                //     (pre: `row == count`). `fill_ticks(row, 1, current_tick)`
                //     stamps both ticks.
                //   * `bytes.len() == dst_pool.component_layout().size()` (B/B2).
                //   * `&mut target` ⇒ exclusive access. `bytes` is live inside
                //     this closure invocation.
                unsafe {
                    dst_pool.write_at_unchecked_initialized(row, bytes);
                    dst_pool.commit_units(row, 1);
                    dst_pool.fill_ticks(row, 1, current_tick);
                }
            }
        });

        // Step 4: complete the target row's archetype-side bookkeeping. Every
        // target pool now holds one committed row at `row` (Step 1 + the closure
        // above), so the entity-id list and `current_index` advance in lockstep —
        // replicating `create_entity_with_ticks`'s tail.
        target.entity_ids.push(entity.id());
        target.current_index = row + 1;

        // Step 5: release source's bytes WITHOUT drop (C5 + W-N2). The source
        // slot is bitwise-released and never deref'd, so running no drop here is
        // required for no-double-free:
        //   * Retained (non-overlap) components were memcpy'd into target
        //     (Step 1) — the live value now belongs to the target row; dropping
        //     the source copy would double-free it.
        //   * Overlap components had their displaced source-copy ALREADY dropped
        //     in the bundle-write closure above (the `has_row` branch's
        //     `drop_at`), which freed any shared heap exactly once; dropping the
        //     bitwise-identical source bytes here would double-free that heap.
        // So the only live drops are: the overlap old value (dropped above,
        // once) and every target value (dropped once when target is removed or
        // on archetype teardown).
        match source.move_out_entity(InlandPoolId(source_row)) {
            RemoveOutcome::Last => {}
            RemoveOutcome::Swapped { moved_entity } => {
                // The entity at source's last_row took source_row's slot.
                // Fix its EntityInland.unit_index. Touches `world.entity_master`,
                // NOT `source`/`target` — independent of the archetype reborrows.
                if let Some(slot) = world.entity_master.entities_inland.get_mut(moved_entity.0) {
                    slot.set_unit_index(source_row as u32);
                }
            }
            RemoveOutcome::PoolFailure => {
                panic!("invariant: migration source removal must succeed");
            }
        }

        // Step 6 (HOISTED into Phase 1, §3.4): update `entity`'s `EntityInland`
        // to point at target's slot. Touches `world.entity_master` (not the
        // archetypes), so it is hoistable; after this the entity is fully in
        // `target` and add/insert hooks (Phase 2) read the NEW target row
        // (Bevy-parity asymmetry, §0).
        world.entity_master.entities_inland[entity.id().0] =
            EntityInland::new(target_ptr, new_row, entity.generation());
        // <-- `source` / `target` `&mut Archetype` DROP here (block close).
    }

    // PHASE 2 (§3.4 / P3): fire hooks. The entity is repointed into `target`;
    // both `&mut Archetype` are dead — only `target_ptr` (*mut, Copy) survives.
    //
    // SAFETY (F1): `target_ptr` is write-capable, stable, interior-mutable
    //   (`SharedReadWrite`, F4-rooted) slab provenance — it survived the
    //   Phase-1 push into `target` (which bumped `target.current_index` through
    //   a same-cell-derived pointer) under TB/SB because the whole slab element
    //   is `UnsafeCell`-wrapped. Reading `flags` is one `u16` load (no `&mut`).
    let flags = unsafe { (*target_ptr).flags };
    if !flags.is_empty() {
        // MINT: no `world`-derived `&mut Archetype` is live (SAFETY-1).
        let world_ptr = NonNull::from(&mut *world);
        let bundle_id_set = &bundle_ids[..bundle_id_count];
        // Ordering (SAFETY-2): ALL on_add (over I\S — newly added), THEN ALL
        // on_insert (over I — the inserted bundle set, NOT all target_ids; P3).
        // Observers fire in the same window as their matching hook (hooks first),
        // over the SAME iteration set — on_add keeps the `bundle_added[i]` filter.
        if flags.contains(ArchetypeFlags::ON_ADD_ANY) {
            if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
                for (i, &cid) in bundle_id_set.iter().enumerate() {
                    if bundle_added[i] {
                        trigger_on_add(world_ptr, cid, entity);
                    }
                }
            }
            if flags.contains(ArchetypeFlags::ON_ADD_OBSERVER) {
                for (i, &cid) in bundle_id_set.iter().enumerate() {
                    if bundle_added[i] {
                        fire_on_add_observers(world_ptr, cid, entity);
                    }
                }
            }
        }
        if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
            if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                for &cid in bundle_id_set {
                    trigger_on_insert(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                for &cid in bundle_id_set {
                    fire_on_insert_observers(world_ptr, cid, entity);
                }
            }
        }
    }
    // NO drain (Q-A1): runs at depth >= 1 inside the per-system apply; the
    // outermost schedule drive drains.
}

/// Performs the remove migration `source → target` over the existing
/// dense byte-buffer storage (Round 3 C-N2). Caller guarantees
/// `source_archetype_id != target_archetype_id` and that `C` is hosted by
/// the source archetype (W1 caller-side check).
#[cold]
#[inline(never)]
pub(crate) fn migrate_entity_remove<C: Component>(
    world: &mut EcsMaster,
    entity: Entity,
    source_archetype_id: ArchetypeId,
    target_archetype_id: ArchetypeId,
) {
    debug_assert_ne!(
        source_archetype_id, target_archetype_id,
        "migrate_entity_remove: caller must filter the no-op case"
    );

    let current_tick = world.current_tick();
    let source_ptr = world
        .archetype_master_mut()
        .archetype_ptr_for(source_archetype_id)
        .expect("invariant: source archetype exists");
    let target_ptr = world
        .archetype_master_mut()
        .archetype_ptr_for(target_archetype_id)
        .expect("invariant: target archetype just resolved");

    let inland = world.entity_master.entities_inland[entity.id().0];
    debug_assert!(!inland.is_null() && inland.generation() == entity.generation());
    let source_row = inland.unit_index() as usize;

    let removed_id = C::component_id();

    // PHASE 1 (§3.5 / C2): confine `source` / `target` `&mut` to this block —
    // collect retained byte slices and push them into `target`. After the push
    // the entity exists in BOTH the source row (`C` still live) and the target
    // row (`C` absent); `EntityInland` STILL points at SOURCE (the repoint is
    // Phase 3). The block produces only `new_row` (Copy); both `&mut Archetype`
    // drop at its close, so Phase 2 mints `world_ptr` with no live reborrow.
    let new_row: u32 = {
        // SAFETY: same rationale as `migrate_entity_insert` (U1, U2, U14, SCH7,
        //   F1) — `source_ptr` / `target_ptr` are interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance that survives the
        //   sibling `current_index` bump under TB/SB (whole slab element is
        //   `UnsafeCell`-wrapped); confined to Phase 1.
        let source: &mut Archetype = unsafe { &mut *source_ptr };
        let target: &mut Archetype = unsafe { &mut *target_ptr };

        // Step 1: collect retained byte slices (target ⊂ source). Same cast-
        // via-`*mut u8` E0521 workaround as `migrate_entity_insert`.
        type RemoveRetainedSlot<'b> = MaybeUninit<(ComponentId, &'b [u8], Tick, Tick)>;
        let mut retained: [RemoveRetainedSlot<'_>; MAX_MIGRATION_COLUMNS] =
            [const { MaybeUninit::uninit() }; MAX_MIGRATION_COLUMNS];
        let retained_base: *mut u8 = retained.as_mut_ptr() as *mut u8;
        let retained_stride: usize = mem::size_of::<RemoveRetainedSlot<'_>>();
        let mut retained_count = 0usize;
        let target_cids: Vec<ComponentId> = target.component_ids().to_vec();
        for target_cid in target_cids.iter().copied() {
            let pool = source
                .component_pools()
                .get_pool(target_cid)
                .expect("invariant: target ⊂ source");
            let stride = pool.component_layout().size();
            // SAFETY (Round 3 C-N2): see `migrate_entity_insert` retained-bytes block.
            let bytes = unsafe { core::slice::from_raw_parts(pool.unit_ptr(source_row), stride) };
            // SAFETY (Phase 10): same conditions as in insert path.
            let added = unsafe { pool.read_added_tick(source_row) };
            // SAFETY: same as above.
            let changed = unsafe { pool.read_changed_tick(source_row) };
            debug_assert!(retained_count < MAX_MIGRATION_COLUMNS);
            // SAFETY (E0521 workaround): cast-via-`*mut u8` keeps the inner
            //   `&[u8]` lifetime bound to the current iteration's `source`
            //   reborrow.
            unsafe {
                let slot_ptr = retained_base.add(retained_count * retained_stride)
                    as *mut RemoveRetainedSlot<'_>;
                slot_ptr.write(MaybeUninit::new((target_cid, bytes, added, changed)));
            }
            retained_count += 1;
        }

        // SAFETY: retained[0..retained_count] are all written; cast
        //   `*const RetainedSlot<'_>` → `*const (ComponentId, &[u8], Tick, Tick)`
        //   is layout-compatible.
        let combined_slice: &[(ComponentId, &[u8], Tick, Tick)] = unsafe {
            core::slice::from_raw_parts(
                retained_base as *const (ComponentId, &[u8], Tick, Tick),
                retained_count,
            )
        };

        let mut new_row: u32 = 0;
        let pushed = target.create_entity_with_ticks(
            entity.id(),
            &mut new_row,
            combined_slice,
            current_tick,
        );
        assert!(
            pushed,
            "remove-migration: target archetype rejected the push — pool reserve \
             ceiling (rows) exhausted (committed capacity grows on demand per \
             Phase X.I) or signature mismatch",
        );
        new_row
        // <-- `source` / `target` `&mut Archetype` DROP here (block close).
    };

    // PHASE 2 (§3.5 / §0): fire `on_replace` + `on_remove` for `C` reading the
    // SOURCE (dying) row. `EntityInland` STILL points at SOURCE (repoint is
    // Phase 3), so `get_component::<C>` via the view reads the dying bytes —
    // the §0 asymmetry vs insert-migration (which reads the NEW target row).
    // Both `&mut Archetype` are dead; only `source_ptr` (*mut, Copy) survives.
    //
    // SAFETY (F1): `source_ptr` is write-capable, stable, interior-mutable
    //   (`SharedReadWrite`, F4-rooted) slab provenance — it survived the
    //   Phase-1 push into `target` (a sibling `current_index` bump) under TB/SB
    //   because the whole slab element is `UnsafeCell`-wrapped. Reading `flags`
    //   is one `u16` load (no `&mut`).
    let flags = unsafe { (*source_ptr).flags };
    if flags.contains(ArchetypeFlags::ON_REPLACE_ANY)
        || flags.contains(ArchetypeFlags::ON_REMOVE_ANY)
    {
        // MINT: no `world`-derived `&mut Archetype` is live (SAFETY-1).
        let world_ptr = NonNull::from(&mut *world);
        // PRE-`drop_at` (SAFETY-2): on_replace then on_remove for the removed C.
        // Observers fire in the same window as their matching hook (hooks first),
        // for the SAME single `removed_id`.
        if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
            trigger_on_replace(world_ptr, removed_id, entity);
        }
        if flags.contains(ArchetypeFlags::ON_REPLACE_OBSERVER) {
            fire_on_replace_observers(world_ptr, removed_id, entity);
        }
        if flags.contains(ArchetypeFlags::ON_REMOVE_HOOK) {
            trigger_on_remove(world_ptr, removed_id, entity);
        }
        if flags.contains(ArchetypeFlags::ON_REMOVE_OBSERVER) {
            fire_on_remove_observers(world_ptr, removed_id, entity);
        }
    }

    // PHASE 3 (§3.5): re-resolve `&mut source`; drop `C` ONCE; release the
    // source row without drop; repoint to target. Deferred commands the hooks
    // enqueued do NOT apply until the outermost drain (Q-A1), so nothing can
    // mutate the source archetype between Phase 2 and here — the
    // `RemoveOutcome::Swapped` fixup cannot observe a stale `source_row`.
    {
        // SAFETY (F1): re-resolved AFTER the Phase-2 hooks returned;
        //   `source_ptr` is write-capable, stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — it survived the
        //   Phase-1 sibling `current_index` bump under TB/SB (whole slab element
        //   is `UnsafeCell`-wrapped); single-threaded `&mut EcsMaster` (SCH7).
        let source: &mut Archetype = unsafe { &mut *source_ptr };

        // C5 discipline: the removed component C's bytes are STILL owned by
        // source. We MUST drop them explicitly before `move_out_entity`
        // (which skips drop on all components per W-N2). Without this drop,
        // the C value would leak (no destructor would ever run on its bytes).
        {
            let removed_pool = source
                .component_pools_mut()
                .get_pool_mut(removed_id)
                .expect("invariant: source hosts C (verified by caller)");
            // SAFETY (C5):
            //   * `source_row < removed_pool.count()` (initialized slot).
            //   * `&mut source` gives exclusive access to every owned pool.
            //   * After drop_at, the slot is logically uninit; the next
            //     `move_out_entity` swap-removes it bytewise (no drop) — so `C`
            //     is dropped EXACTLY once across the dual-presence window.
            unsafe { removed_pool.drop_at(source_row) };
        }

        match source.move_out_entity(InlandPoolId(source_row)) {
            RemoveOutcome::Last => {}
            RemoveOutcome::Swapped { moved_entity } => {
                if let Some(slot) = world.entity_master.entities_inland.get_mut(moved_entity.0) {
                    slot.set_unit_index(source_row as u32);
                }
            }
            RemoveOutcome::PoolFailure => panic!("invariant: source removal must succeed"),
        }
        // <-- `&mut source` DROP here.
    }

    world.entity_master.entities_inland[entity.id().0] =
        EntityInland::new(target_ptr, new_row, entity.generation());
    // NO drain (Q-A1): runs at depth >= 1 inside the per-system apply; the
    // outermost schedule drive drains.
}
