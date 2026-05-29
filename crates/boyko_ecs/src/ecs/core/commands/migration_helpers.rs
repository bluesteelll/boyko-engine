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
//! Both paths use the existing chunked + Unit-pointer `ComponentPool`
//! storage (Round 3 C-N2): retained bytes are extracted via
//! `units[source_row].ptr()` + `from_raw_parts`, copied into the target via
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
const MAX_BUNDLE_ARITY: usize = 8;

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

    // Edge case: removing the only component leaves an empty archetype.
    // Phase 11 ships without empty-archetype support; debug_assert + bail.
    // Real ECSs end up with at least one component per live entity, so
    // this branch is reachable only via test code.
    debug_assert!(
        !kept.is_empty(),
        "RemoveCommand: removing the only component yields an empty archetype \
         (unsupported in Phase 11; see plan §7.3 limitation)"
    );
    if kept.is_empty() {
        return None;
    }

    Some(world.get_or_create_archetype(&kept))
}

/// Performs the insert migration `source → target` over the existing
/// chunked + Unit-pointer storage (Round 3 C-N2, plan §7.2). Caller
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

        // Step 1: collect retained-byte slices + original ticks for
        // components present in BOTH source and target. Same cast-via-`*mut u8`
        // pattern as the bundle/combined arrays so retained slots can store
        // `&[u8]` slices whose lifetime is bounded by the per-iteration
        // `source` reborrow without forcing the outer array's invariance to
        // outlive the loop.
        type RetainedSlot<'b> = MaybeUninit<(ComponentId, &'b [u8], Tick, Tick)>;
        let mut retained: [RetainedSlot<'_>; MAX_MIGRATION_COLUMNS] =
            [const { MaybeUninit::uninit() }; MAX_MIGRATION_COLUMNS];
        let retained_base: *mut u8 = retained.as_mut_ptr() as *mut u8;
        let retained_stride: usize = mem::size_of::<RetainedSlot<'_>>();
        let mut retained_count = 0usize;

        let target_cids: Vec<ComponentId> = target.component_ids().to_vec();
        for target_cid in target_cids.iter().copied() {
            if source.component_ids().contains(&target_cid) {
                let pool = source
                    .component_pools()
                    .get_pool(target_cid)
                    .expect("invariant: retained component must exist in source");
                debug_assert!(
                    source_row < pool.count(),
                    "source_row out of bounds for retained component"
                );
                let stride = pool.component_layout().size();

                // SAFETY (Round 3 C-N2):
                //   * `pool.unit_ptr(source_row)` returns a valid arena
                //     pointer (initialized slot, < pool.count()).
                //   * Read-only valid for the lifetime of the `source`
                //     reborrow (we hold &mut source through which we obtain
                //     the &pool — shared reborrow).
                //   * Bytes will be memcpy'd into target via
                //     `create_entity_with_ticks` BEFORE we mutate source.
                //   * Slice length is `stride` bytes; `pool.layout().size()
                //     == stride`.
                let bytes =
                    unsafe { core::slice::from_raw_parts(pool.unit_ptr(source_row), stride) };
                // SAFETY (Phase 10 STORE3): `source_row < pool.count()`;
                //   `&mut source` ensures no concurrent writer; the tick
                //   buffer is sized to `pool.capacity() >= pool.count()`.
                let added = unsafe { pool.read_added_tick(source_row) };
                // SAFETY: same as above.
                let changed = unsafe { pool.read_changed_tick(source_row) };

                debug_assert!(retained_count < MAX_MIGRATION_COLUMNS);
                // SAFETY (E0521 workaround): same cast-via-`*mut u8` pattern
                //   as the bundle/combined arrays — keeps the inner `&[u8]`
                //   lifetime bound to the current iteration's `source` reborrow.
                unsafe {
                    let slot_ptr = retained_base.add(retained_count * retained_stride)
                        as *mut RetainedSlot<'_>;
                    slot_ptr.write(MaybeUninit::new((target_cid, bytes, added, changed)));
                }
                retained_count += 1;
            }
        }

        // Step 2: collect bundle byte slices into a parallel stack array.
        // ManuallyDrop discipline is upheld by `for_each_component_bytes`.
        // Mirrors the `SpawnAtCommand::apply` cast-via-`*mut u8` pattern
        // (E0521 workaround): the slot stores `&[u8]` whose lifetime is
        // exactly the closure's per-call scope, so direct slot indexing
        // would force the outer array's invariance to outlive the closure.
        type BundleSlot<'b> = MaybeUninit<(ComponentId, &'b [u8], Tick, Tick)>;
        let mut bundle_slots: [BundleSlot<'_>; MAX_BUNDLE_ARITY] =
            [const { MaybeUninit::uninit() }; MAX_BUNDLE_ARITY];
        let bundle_base: *mut u8 = bundle_slots.as_mut_ptr() as *mut u8;
        let bundle_stride: usize = mem::size_of::<BundleSlot<'_>>();
        let mut bundle_count = 0usize;
        bundle.for_each_component_bytes(|id, bytes| {
            debug_assert!(bundle_count < MAX_BUNDLE_ARITY);
            // Phase 14a P3: record the bundle id + whether it is newly-added (NOT
            // already in source). This runs BEFORE `move_out_entity` (Step 5), so
            // the source membership read is against the pre-migration source row.
            debug_assert!(bundle_id_count < MAX_BUNDLE_ARITY);
            bundle_ids[bundle_id_count] = id;
            bundle_added[bundle_id_count] = !source.component_ids().contains(&id);
            bundle_id_count += 1;
            // SAFETY (E0521 workaround, mirrors `SpawnAtCommand::apply`):
            //   * `bundle_base` is a `*mut u8` minted from a live mutable
            //     borrow of `bundle_slots`. The cast-back at the closure's
            //     lifetime context makes the inner `&'callback [u8]` match
            //     the slot type exactly.
            //   * `bundle_count < MAX_BUNDLE_ARITY` (debug-asserted) keeps
            //     the offset within the array's bounds.
            //   * `MaybeUninit::write` is a bitwise copy into the slot.
            unsafe {
                let slot_ptr = bundle_base.add(bundle_count * bundle_stride) as *mut BundleSlot<'_>;
                slot_ptr.write(MaybeUninit::new((id, bytes, current_tick, current_tick)));
            }
            bundle_count += 1;
        });

        // Step 3: merge retained + bundle. Bundle WINS on overlap (Q6 —
        // replace semantic). Same `*mut u8` cast pattern as bundle_slots so
        // the entries we read back carry the inner `&[u8]`'s closure
        // lifetime (the closure has exited; ManuallyDrop locals are still
        // alive until `bundle` itself drops at function end — see
        // `Bundle::for_each_component_bytes` B4 contract).
        type CombinedSlot<'b> = MaybeUninit<(ComponentId, &'b [u8], Tick, Tick)>;
        let mut combined: [CombinedSlot<'_>; MAX_MIGRATION_COLUMNS] =
            [const { MaybeUninit::uninit() }; MAX_MIGRATION_COLUMNS];
        let combined_base: *mut u8 = combined.as_mut_ptr() as *mut u8;
        let combined_stride: usize = mem::size_of::<CombinedSlot<'_>>();
        let mut combined_count = 0usize;

        // Seed combined with retained entries.
        for i in 0..retained_count {
            debug_assert!(combined_count < MAX_MIGRATION_COLUMNS);
            // SAFETY: read retained[i] through the cast-via-`*mut u8`
            //   pattern; the inner `&[u8]` lifetime is bounded by the
            //   `source` reborrow and is still alive here.
            let entry = unsafe {
                let slot_ptr = retained_base.add(i * retained_stride) as *const RetainedSlot<'_>;
                (*slot_ptr).assume_init()
            };
            // SAFETY: cast-via-`*mut u8` preserves the inner lifetime; the
            //   write is a bitwise copy.
            unsafe {
                let slot_ptr =
                    combined_base.add(combined_count * combined_stride) as *mut CombinedSlot<'_>;
                slot_ptr.write(MaybeUninit::new(entry));
            }
            combined_count += 1;
        }

        // Layer bundle entries on top of combined. Bundle wins on overlap.
        for i in 0..bundle_count {
            // SAFETY: `bundle_slots[i]` was written by the closure above;
            //   the inner `&[u8]` lifetime is bound to `bundle`'s
            //   ManuallyDrop locals (alive until function end).
            let entry = unsafe {
                let slot_ptr = bundle_base.add(i * bundle_stride) as *const BundleSlot<'_>;
                (*slot_ptr).assume_init()
            };
            let (b_id, _, _, _) = entry;
            let mut replaced = false;
            for j in 0..combined_count {
                // SAFETY: `combined[j]` was written by an earlier iteration
                //   of this function. Inner `&[u8]` lifetime is still alive.
                let (c_id, _, _, _) = unsafe {
                    let slot_ptr =
                        combined_base.add(j * combined_stride) as *const CombinedSlot<'_>;
                    (*slot_ptr).assume_init()
                };
                if c_id == b_id {
                    // Override: write the bundle entry into combined[j].
                    // SAFETY: same cast pattern; replaces the existing
                    //   initialized slot bitwise.
                    unsafe {
                        let slot_ptr =
                            combined_base.add(j * combined_stride) as *mut CombinedSlot<'_>;
                        slot_ptr.write(MaybeUninit::new(entry));
                    }
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                debug_assert!(combined_count < MAX_MIGRATION_COLUMNS);
                // SAFETY: same cast pattern; appending an initialized slot.
                unsafe {
                    let slot_ptr = combined_base.add(combined_count * combined_stride)
                        as *mut CombinedSlot<'_>;
                    slot_ptr.write(MaybeUninit::new(entry));
                }
                combined_count += 1;
            }
        }

        // SAFETY (mirrors `SpawnAtCommand::apply`): combined[0..combined_count]
        //   are initialised; the cast `*const CombinedSlot<'_>` → `*const
        //   (ComponentId, &[u8], Tick, Tick)` is layout-compatible because
        //   `MaybeUninit<T>` and `T` share layout. The inner `&[u8]` lifetime
        //   inherits from the bundle's ManuallyDrop locals + the source pool's
        //   bytes (both alive until function end — bundle drops on Drop of
        //   `self`, source-pool bytes outlive this function's `&mut source`
        //   borrow).
        let combined_slice: &[(ComponentId, &[u8], Tick, Tick)] = unsafe {
            core::slice::from_raw_parts(
                combined_base as *const (ComponentId, &[u8], Tick, Tick),
                combined_count,
            )
        };

        // Step 4: push into target with explicit ticks. This memcpy'd every
        // retained byte slice INTO target's pools, completing the "move
        // out" semantics required by `move_out_entity`'s W-N2 contract.
        let mut new_row: u32 = 0;
        let pushed = target.create_entity_with_ticks(
            entity.id(),
            &mut new_row,
            combined_slice,
            current_tick,
        );
        assert!(pushed, "target archetype rejected migration push");

        // Step 5: release source's bytes WITHOUT drop (C5 + W-N2). The
        // retained components were just memcpy'd into target; bundle
        // components that overrode retained on overlap have their old source
        // bytes left behind, but those are the SAME byte image already
        // copied into target — bitwise-identical, so the bundle's new bytes
        // logically replaced them. Either way, no drop should run on source's
        // pools (a single drop will occur eventually when target is removed
        // or via Drop on archetype teardown).
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
        if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
            for (i, &cid) in bundle_id_set.iter().enumerate() {
                if bundle_added[i] {
                    trigger_on_add(world_ptr, cid, entity);
                }
            }
        }
        if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
            for &cid in bundle_id_set {
                trigger_on_insert(world_ptr, cid, entity);
            }
        }
    }
    // NO drain (Q-A1): runs at depth >= 1 inside the per-system apply; the
    // outermost schedule drive drains.
}

/// Performs the remove migration `source → target` over the existing
/// chunked + Unit-pointer storage (Round 3 C-N2). Caller guarantees
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
        assert!(pushed);
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
    if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK)
        || flags.contains(ArchetypeFlags::ON_REMOVE_HOOK)
    {
        // MINT: no `world`-derived `&mut Archetype` is live (SAFETY-1).
        let world_ptr = NonNull::from(&mut *world);
        // PRE-`drop_at` (SAFETY-2): on_replace then on_remove for the removed C.
        if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
            trigger_on_replace(world_ptr, removed_id, entity);
        }
        if flags.contains(ArchetypeFlags::ON_REMOVE_HOOK) {
            trigger_on_remove(world_ptr, removed_id, entity);
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
