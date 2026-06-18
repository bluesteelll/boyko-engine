//! Shared low-level migration scaffolding for `InsertCommand` /
//! `RemoveCommand` (Phase 11 §7).
//!
//! Two flavours of archetype change:
//!
//! * `migrate_entity_insert` — source ∪ bundle. If `merged_archetype_id ==
//!   source_archetype_id` (canonicalization invariant cited from
//!   `archetype_master.rs:99-133, 462-473`), the in-place replace fast path
//!   `apply_replace_in_place` is taken — `bundle ⊆ source`.
//! * `migrate_entity_remove` — source \\ `{C}`. Single-component remove;
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
use crate::ecs::core::component::component_registry::{self, MAX_COMPONENTS};
use crate::ecs::core::component::enable::enable_store::SmallList4;
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::hooks::dispatch::{
    trigger_on_add, trigger_on_insert, trigger_on_remove, trigger_on_replace,
};
use crate::ecs::core::component::observers::dispatch::{
    fire_on_add_observers, fire_on_insert_observers, fire_on_remove_observers,
    fire_on_replace_observers,
};
use crate::ecs::core::component::observers::ObserverKind;
use crate::ecs::core::component::observers::entity_store::fire_entity_observers;
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

// ═════════════════════════════════════════════════════════════════════════════
// EnableTag Step 6 — cross-archetype enable-bit migration (Decision D1 / D3 /
// the C4 READ-before-swap ordering).
//
// When an entity migrates archetypes, its EnableTag bits live per-archetype-
// per-row, so they must be COPIED from the source row to the target append row
// — otherwise a toggled flag is silently lost on the next structural op. The
// copy is a 3-phase, borrow-free, single-fire snapshot+restore of the MIGRATING
// entity's bits ONLY:
//
//   PHASE 1 (READ): snapshot the source row's bits into a borrow-free owned
//     `SmallList4<(ComponentId, bool)>` scratch BEFORE the source
//     `move_out_entity` runs (the C4 ordering — the source swap-fix in
//     `move_out_entity`, already wired in Wave 2, relocates the source's OTHER
//     rows' bits, so the migrating entity's bits must be snapshotted first).
//     The snapshot owns plain `bool`s (never a borrow into the source store),
//     so it survives the source mutation (W3-r6 — the Phase-11 dangling-slice
//     class does not apply).
//
//   PHASE 2 (WRITE): after the target append (the entity is at `target_row`),
//     write each snapshotted bit into the target archetype's `EnableStore`. A
//     genuinely-new target column triggers the one-time O2 bookkeeping
//     (`note_enable_column_alloc`) — fired by the caller AFTER both
//     `&mut Archetype` reborrows drop (it touches `world.archetype_master`,
//     mirroring `set_enable_bit`'s Step 4 borrow discipline).
//
// The source-row swap-fix is ALREADY handled by `move_out_entity` (Wave 2,
// O1-r7); Step 6 is ONLY the migrating entity's source→target copy, so the bit
// op fires exactly once per migration.
//
// 0%-gate (non-enable entities): a source archetype with an empty `EnableStore`
// skips the entire copy via the `is_empty()` fast path, so a migration of an
// entity that never touched any EnableTag is byte-identical to before Step 6.
// ═════════════════════════════════════════════════════════════════════════════

/// PHASE 1: snapshots the migrating entity's enable bits at `source_row` into
/// the borrow-free `scratch` (cleared first), returning `true` iff any column
/// was present (i.e. the WRITE phase has work to do).
///
/// Gated by [`EnableStore::is_empty`]: a source archetype that owns no enable
/// columns leaves `scratch` empty and returns `false` (the 0%-gate fast path).
/// MUST be called BEFORE the source `move_out_entity` (the C4 READ-before-swap
/// ordering — see the module banner above).
#[inline]
fn read_source_enable_bits(
    source: &Archetype,
    source_row: usize,
    scratch: &mut SmallList4<(ComponentId, bool)>,
) -> bool {
    // 0%-gate: skip the snapshot for an enable-free source.
    if source.enable_store.is_empty() {
        scratch.clear();
        return false;
    }
    // Borrow-free Copy snapshot (W3-r6): `scratch` owns its bools and does NOT
    // borrow `source`, so it survives the later source `move_out_entity`.
    source.enable_store.read_row_bits(source_row, scratch);
    !scratch.is_empty()
}

/// PHASE 2: writes the snapshotted `scratch` bits into `target`'s `EnableStore`
/// at `target_row`, appending each tag whose first column had to be allocated to
/// `newly_allocated` so the caller can fire the one-time O2 bookkeeping once the
/// `&mut Archetype` reborrows drop.
///
/// A clear (`false`) bit never allocates a column or page
/// ([`EnableStore::write_row_bit`] short-circuits it), so only a `true` bit into
/// a previously-absent target column counts as a new allocation.
#[inline]
fn write_target_enable_bits(
    target: &mut Archetype,
    target_row: usize,
    scratch: &SmallList4<(ComponentId, bool)>,
    newly_allocated: &mut SmallList4<ComponentId>,
) {
    if scratch.is_empty() {
        return;
    }
    let reserve_rows = target.enable_reserve_rows();
    for &(tag, value) in scratch.iter() {
        // A `true` bit into an absent target column allocates it — record that
        // tag so the caller fires `note_enable_column_alloc` exactly once (O2),
        // mirroring `Archetype::set_enable_bit`'s `newly_allocated` return.
        if value && target.enable_store.column(tag).is_none() {
            newly_allocated.push(tag);
        }
        target
            .enable_store
            .write_row_bit(tag, target_row, value, reserve_rows);
    }
}

/// O2 bookkeeping: records the per-world presence bit + bumps `enable_generation`
/// once for every target tag whose first column was allocated by the migration
/// copy ([`write_target_enable_bits`]).
///
/// MUST be called with NO `&mut Archetype` reborrow live: it borrows
/// `world.archetype_master`, which aliases the slab the migration's
/// `&mut Archetype` reborrows are derived from (mirrors `set_enable_bit`'s
/// Step-4 discipline — Phase 14a §3.4). An empty list is a no-op (the common
/// case: no new column, or an enable-free migration).
#[inline]
fn fire_enable_column_alloc_bookkeeping(
    world: &mut EcsMaster,
    target_archetype_id: ArchetypeId,
    newly_allocated: &SmallList4<ComponentId>,
) {
    if newly_allocated.is_empty() {
        return;
    }
    let master = world.archetype_master_mut();
    for &tag in newly_allocated.iter() {
        master.note_enable_column_alloc(tag, target_archetype_id);
    }
}

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

    // Required components (Feature 1, D4): union the transitive closure of the
    // BUNDLE ids' `#[require]`s into the effective set (present⇒skip is handled
    // by the `contains` check — a required id already in source OR bundle is not
    // re-added). For a require-free bundle `for_each_required_id_excluding` runs
    // zero inner iterations — the 0%-gate. The closure is computed over
    // `bundle_ids` (the inserted set); a component required only by an
    // already-present source component is not auto-inserted (Bevy parity:
    // requires expand the INSERTED bundle, not the resident archetype).
    component_registry::for_each_required_id_excluding(bundle_ids, |cid| {
        if !combined[..len].contains(&cid) {
            debug_assert!(
                len < MAX_MIGRATION_COLUMNS,
                "migration union exceeds MAX_COMPONENTS (required expansion)"
            );
            combined[len] = cid;
            len += 1;
        }
    });

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

    // Required components (Feature 1, MAJOR 0%-gate): the ENTIRE Step-2b required
    // block — the constructor pass, the `required_fire_ids` 4 KiB scratch, and the
    // `for_each_required_id_excluding`/`Vec`-allocating walk — is gated behind this
    // one-shot `any_requires` early-out (mirrors the `merged_archetype_id` required
    // expansion 0%-gate, ~:225). `any_requires` is alloc-free: a ≤MAX_BUNDLE_ARITY
    // loop over `get_required_plan(cid).entries.is_empty()` (all empty + memoized
    // for a require-free bundle — NO `Vec`). On a require-free insert the block does
    // not allocate, does not zero a 4 KiB array (its decl is moved INSIDE the
    // guard), and does not loop. `B::component_ids()` is the inserted set — the same
    // set `merged_archetype_id` expanded over.
    let has_requires = component_registry::any_requires(B::component_ids());

    // C1 + C2: the constructed-required fire scratch, materialised ONLY on a
    // require-bearing insert. `None` on the require-free path leaves the 4 KiB
    // `[ComponentId; MAX_MIGRATION_COLUMNS]` array unallocated + unzeroed (the
    // 0%-gate). Sized `MAX_MIGRATION_COLUMNS` (NOT `MAX_BUNDLE_ARITY` — required
    // components are an archetype-level concern with the larger bound; C2). Every
    // entry is absent-in-source AND absent-in-bundle, so it fires on_add
    // (added=true) AND on_insert in Phase 2 exactly once (C1: the bundle-only fire
    // loop would otherwise cover NEITHER hook for a constructed required id).
    // Filled by the gated Step-2b constructor pass inside the Phase-1 block; read in
    // Phase 2. `Box<[..]>` keeps the 4 KiB off the stack frame entirely on the cold
    // require-bearing path.
    let mut required_fire: Option<(Box<[ComponentId; MAX_MIGRATION_COLUMNS]>, usize)> = None;

    // EnableTag Step 6: borrow-free scratch for the migrating entity's enable
    // bits (READ in Phase 1 before the source swap, WRITTEN into the target row)
    // and the list of target tags whose first column had to be allocated (O2
    // bookkeeping fired after the block, when no `&mut Archetype` is live).
    let mut enable_scratch: SmallList4<(ComponentId, bool)> = SmallList4::new();
    let mut enable_newly_allocated: SmallList4<ComponentId> = SmallList4::new();

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

        // EnableTag Step 6 PHASE 1 (C4 READ-before-swap): snapshot the migrating
        // entity's enable bits at `source_row` into the borrow-free scratch
        // BEFORE the source `move_out_entity` (Step 5) relocates the source's
        // OTHER rows' bits. The 0%-gate `is_empty()` fast path skips this for an
        // enable-free source.
        read_source_enable_bits(source, source_row, &mut enable_scratch);

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

        // Step 2b: required-component constructor pass (Feature 1, D5 + C1 + C2).
        // GATED behind the `has_requires` 0%-gate (MAJOR fix): for a require-free
        // insert this whole block is skipped — no `required_fire` allocation, no
        // 4 KiB array, no `for_each_required_id_excluding`/`Vec` walk.
        //
        // For each transitively-required component the BUNDLE pulls that is absent
        // in BOTH the source AND the bundle (present⇒skip), construct one value via
        // its capture-free ctor directly into `row`, commit it, fill its ticks, AND
        // push its id into `required_fire` so the Phase-2 fire loop covers it (C1: a
        // constructed required id is not a bundle id, so without this it would fire
        // NEITHER on_add NOR on_insert on the insert path — the Phase-14b
        // "undercounting fire sites" class).
        //
        // `B::component_ids()` is the supplied (inserted) set — the walk is computed
        // over it, matching the `merged_archetype_id` expansion. The
        // `source.component_ids().contains` / `bundle_id_set` checks together yield
        // "in target, absent in source, absent in bundle" = exactly the constructed
        // set.
        if has_requires {
            let bundle_supplied = B::component_ids();
            let bundle_id_set = &bundle_ids[..bundle_id_count];
            // Materialise the fire scratch lazily — only on this require-bearing
            // path. `Box::new([..])` zeroes the 4 KiB on the heap (cold path).
            let (fire_ids, fire_count) =
                required_fire.get_or_insert_with(|| (Box::new([ComponentId(0); MAX_MIGRATION_COLUMNS]), 0));
            component_registry::for_each_required_id_excluding(bundle_supplied, |req_id| {
                // present⇒skip: a required id already hosted by the source keeps
                // its existing value (no overwrite, no construct, no re-fire —
                // C1's "present does not fire" path). A required id supplied by
                // the bundle was already written by the closure above.
                if source.component_ids().contains(&req_id) || bundle_id_set.contains(&req_id) {
                    return;
                }
                // Resolve the ctor for `req_id` from the bundle's transitive
                // closure (the same W1-resolved ctor the expansion used).
                let ctor = component_registry::required_ctor_for(bundle_supplied, req_id).expect(
                    "invariant: req_id came from the bundle's required closure, so a ctor \
                     exists for it",
                );
                let dst_pool = target
                    .component_pools_mut()
                    .get_pool_mut(req_id)
                    .expect("invariant: target hosts every required id (expanded archetype)");
                debug_assert!(
                    !dst_pool.has_row(row),
                    "required ctor pass: pool already committed row (id supplied twice?)"
                );
                // SAFETY (mirrors the bundle newly-added arm + SpawnAtCommand):
                //   * `row == dst_pool.count()` (lockstep) and `reserve_capacity(1)`
                //     committed the slot ⇒ `construct_at_uninitialized` targets a
                //     logically-uninit slot (no drop). `commit_units(row, 1)`
                //     extends the tail; `fill_ticks` stamps both ticks.
                //   * `ctor` writes one value of `req_id`'s registered type into
                //     the slot (registry-paired). `&mut target` ⇒ exclusive.
                unsafe {
                    dst_pool.construct_at_uninitialized(row, ctor);
                    dst_pool.commit_units(row, 1);
                    dst_pool.fill_ticks(row, 1, current_tick);
                }
                // C2: `*fire_count < MAX_MIGRATION_COLUMNS` holds — the constructed
                // set ⊆ target ids ⊆ MAX_MIGRATION_COLUMNS columns.
                debug_assert!(
                    *fire_count < MAX_MIGRATION_COLUMNS,
                    "required fire scratch overflow (constructed > MAX_MIGRATION_COLUMNS)"
                );
                fire_ids[*fire_count] = req_id;
                *fire_count += 1;
            });
        }

        // Step 4: complete the target row's archetype-side bookkeeping. Every
        // target pool now holds one committed row at `row` (Step 1 + the closure
        // above + the required ctor pass), so the entity-id list and
        // `current_index` advance in lockstep — replicating
        // `create_entity_with_ticks`'s tail.
        target.entity_ids.push(entity.id());
        target.current_index = row + 1;

        // EnableTag Step 6 PHASE 2: restore the snapshotted enable bits into the
        // target's new row (`new_row`). Must precede the source `move_out_entity`
        // only insofar as both touch the archetypes — the scratch is borrow-free,
        // so ordering vs the source swap is immaterial; placed here (before
        // Step 5) so the whole copy completes while `target` is `&mut`-live.
        write_target_enable_bits(
            target,
            new_row as usize,
            &enable_scratch,
            &mut enable_newly_allocated,
        );

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

    // EnableTag Step 6 O2: fire the one-time presence + `enable_generation`
    // bookkeeping for every target column the copy newly allocated. Deferred to
    // here (no `&mut Archetype` is live) because `note_enable_column_alloc`
    // borrows `world.archetype_master`, which aliases the slab the dropped
    // `&mut Archetype` reborrows came from (mirrors `set_enable_bit`'s Step 4).
    fire_enable_column_alloc_bookkeeping(world, target_archetype_id, &enable_newly_allocated);

    // Feature 2 (FIX C1): raise the sticky HAS_ENTITY_OBSERVER bit on the
    // DESTINATION archetype BEFORE the destination `flags` are read for this
    // entity's fires. On the FIRST migration of an observed entity into an
    // archetype that never held an observed member, the destination's bit 10 is
    // still clear; raising it AFTER the flags read (the prior shape) skipped the
    // entity-targeted on_add/on_insert observers for exactly this migration and
    // raised the bit one frame too late. The entity is fully repointed into
    // `target` now (Phase-1 block closed), so `&mut world` is usable and the
    // probe resolves against the destination archetype. `migrate_entity_observer_bit`
    // is gated by `has_observer(entity)` — a no-op (one `Option::is_none()`) for
    // an entity with no entity observer (the 0%-gate). The bit is sticky, so the
    // subsequent `flags` read observes the raise. (The remove path raises on the
    // SOURCE archetype where the bit was already present, so it does NOT need
    // this hoist — see `migrate_entity_remove`.)
    world.migrate_entity_observer_bit(entity);

    // PHASE 2 (§3.4 / P3): fire hooks. The entity is repointed into `target`;
    // both `&mut Archetype` are dead — only `target_ptr` (*mut, Copy) survives.
    //
    // SAFETY (F1): `target_ptr` is write-capable, stable, interior-mutable
    //   (`SharedReadWrite`, F4-rooted) slab provenance — it survived the
    //   Phase-1 push into `target` (which bumped `target.current_index` through
    //   a same-cell-derived pointer) under TB/SB because the whole slab element
    //   is `UnsafeCell`-wrapped. Reading `flags` is one `u16` load (no `&mut`).
    //   The C1 `migrate_entity_observer_bit` call above may have just set
    //   HAS_ENTITY_OBSERVER on this same archetype; the bit is sticky and the
    //   write completed under `&mut world` before this read, so it is observed.
    let flags = unsafe { (*target_ptr).flags };
    if !flags.is_empty() {
        // MINT: no `world`-derived `&mut Archetype` is live (SAFETY-1).
        let world_ptr = NonNull::from(&mut *world);
        let bundle_id_set = &bundle_ids[..bundle_id_count];
        // Required components (Feature 1, C1): the constructed required ids. EVERY
        // entry here is absent-in-source AND absent-in-bundle, so it is ALWAYS
        // newly-added (the on_add filter is unconditional, unlike `bundle_added`)
        // AND is inserted (on_insert). Iterated alongside the bundle set in both
        // windows so the existing insert fire loop covers it — the C1 headline
        // fix (a constructed required id is not a bundle id, so without this it
        // fires NEITHER on_add NOR on_insert on the insert path).
        let required_fire_set: &[ComponentId] = match &required_fire {
            Some((ids, count)) => &ids[..*count],
            None => &[],
        };
        // Ordering (SAFETY-2): ALL on_add (over I\S — newly added — PLUS the
        // constructed required ids), THEN ALL on_insert (over I PLUS the
        // constructed required ids). Observers fire in the same window as their
        // matching hook (hooks first), over the SAME iteration set — on_add keeps
        // the `bundle_added[i]` filter for the bundle ids.
        if flags.contains(ArchetypeFlags::ON_ADD_ANY) {
            if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
                for (i, &cid) in bundle_id_set.iter().enumerate() {
                    if bundle_added[i] {
                        trigger_on_add(world_ptr, cid, entity);
                    }
                }
                for &cid in required_fire_set {
                    trigger_on_add(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_ADD_OBSERVER) {
                for (i, &cid) in bundle_id_set.iter().enumerate() {
                    if bundle_added[i] {
                        fire_on_add_observers(world_ptr, cid, entity);
                    }
                }
                for &cid in required_fire_set {
                    fire_on_add_observers(world_ptr, cid, entity);
                }
            }
        }
        if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
            if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                for &cid in bundle_id_set {
                    trigger_on_insert(world_ptr, cid, entity);
                }
                for &cid in required_fire_set {
                    trigger_on_insert(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                for &cid in bundle_id_set {
                    fire_on_insert_observers(world_ptr, cid, entity);
                }
                for &cid in required_fire_set {
                    fire_on_insert_observers(world_ptr, cid, entity);
                }
            }
        }
        // Feature 2 — entity-targeted on_add / on_insert observers, over the
        // SAME iteration sets as the component-level fires above, gated by the
        // archetype's sticky HAS_ENTITY_OBSERVER bit.
        if flags.contains(ArchetypeFlags::HAS_ENTITY_OBSERVER) {
            for (i, &cid) in bundle_id_set.iter().enumerate() {
                if bundle_added[i] {
                    fire_entity_observers(world_ptr, ObserverKind::Add, cid, entity);
                }
            }
            for &cid in required_fire_set {
                fire_entity_observers(world_ptr, ObserverKind::Add, cid, entity);
            }
            for &cid in bundle_id_set {
                fire_entity_observers(world_ptr, ObserverKind::Insert, cid, entity);
            }
            for &cid in required_fire_set {
                fire_entity_observers(world_ptr, ObserverKind::Insert, cid, entity);
            }
        }
    }
    // Feature 2 (FIX C1): the sticky HAS_ENTITY_OBSERVER bit was raised on the
    // DESTINATION archetype BEFORE the `flags` read above (so the entity-targeted
    // fires for THIS migration are not skipped), superseding the prior late
    // re-raise that sat here.
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

    // EnableTag Step 6: borrow-free scratch for the migrating entity's enable
    // bits + the list of target tags whose first column had to be allocated (O2
    // bookkeeping fired after Phase 3, when no `&mut Archetype` is live).
    let mut enable_scratch: SmallList4<(ComponentId, bool)> = SmallList4::new();
    let mut enable_newly_allocated: SmallList4<ComponentId> = SmallList4::new();

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

        // EnableTag Step 6 PHASE 1 (C4 READ-before-swap): snapshot the migrating
        // entity's enable bits at `source_row` BEFORE the Phase-3
        // `move_out_entity` relocates the source's OTHER rows' bits. The scratch
        // is borrow-free, so it survives that later source mutation (W3-r6).
        // 0%-gate via `is_empty()`.
        read_source_enable_bits(source, source_row, &mut enable_scratch);

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

        // EnableTag Step 6 PHASE 2: restore the snapshotted enable bits into the
        // target's new row while `target` is `&mut`-live (the entity is now in
        // BOTH rows; `EntityInland` still points at SOURCE until Phase 3 — the
        // copy writes the TARGET row regardless).
        write_target_enable_bits(
            target,
            new_row as usize,
            &enable_scratch,
            &mut enable_newly_allocated,
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
        || flags.contains(ArchetypeFlags::HAS_ENTITY_OBSERVER)
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
        // Feature 2 — entity-targeted on_replace / on_remove observers for the
        // removed component, gated by the SOURCE archetype's sticky
        // HAS_ENTITY_OBSERVER bit (the entity still lives in source here).
        if flags.contains(ArchetypeFlags::HAS_ENTITY_OBSERVER) {
            fire_entity_observers(world_ptr, ObserverKind::Replace, removed_id, entity);
            fire_entity_observers(world_ptr, ObserverKind::Remove, removed_id, entity);
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

    // EnableTag Step 6 O2: fire presence + `enable_generation` bookkeeping for
    // every target column the copy newly allocated. Deferred to here (after the
    // Phase-3 `&mut source` dropped) so `note_enable_column_alloc`'s
    // `&mut world.archetype_master` aliases no live `&mut Archetype` reborrow.
    fire_enable_column_alloc_bookkeeping(world, target_archetype_id, &enable_newly_allocated);
    // Feature 2 (FIX W2/C4/C5): re-raise the sticky HAS_ENTITY_OBSERVER bit on
    // the DESTINATION archetype if this entity still carries an entity observer.
    // A no-op for an entity with no entity observer (the 0%-gate).
    world.migrate_entity_observer_bit(entity);
    // NO drain (Q-A1): runs at depth >= 1 inside the per-system apply; the
    // outermost schedule drive drains.
}

// ═════════════════════════════════════════════════════════════════════════════
// Phase 22 D9 — dynamic id-keyed migration (tags). Non-generic siblings of the
// typed helpers above: no `B: Bundle`, no `C: Component` — a `&[ComponentId]`
// drives the set algebra. Allocation-free by construction (stack arrays
// bounded by `MAX_MIGRATION_COLUMNS`); the generic versions keep their
// `to_vec()` / `kept: Vec` untouched (cold-path debt, out of scope).
// ═════════════════════════════════════════════════════════════════════════════

/// Resolves the destination archetype id for a dynamic id-keyed attach
/// (Phase 22 D9): `source ∪ extra`, canonical-sorted, resolved through the
/// single `get_or_create_archetype` funnel.
///
/// Allocation-free: the union lives in a stack array bounded by
/// `MAX_MIGRATION_COLUMNS` (plan D9 — mirrors the `retained` stack-array
/// precedent in [`migrate_entity_remove`]).
///
/// The function does NOT guarantee `source != target`: if every id in
/// `extra` is already hosted by the source, the union collapses to the
/// source set and the funnel returns `source_archetype_id` — the caller
/// decides the present-tag in-place path BEFORE calling (presence test on
/// the source signature, [`retag_in_place`]).
#[inline]
pub(crate) fn merged_archetype_id_dyn(
    world: &mut EcsMaster,
    source_archetype_id: ArchetypeId,
    extra: &[ComponentId],
) -> ArchetypeId {
    let mut combined: [ComponentId; MAX_MIGRATION_COLUMNS] =
        [ComponentId(0); MAX_MIGRATION_COLUMNS];
    let mut len = 0usize;

    {
        let source = world
            .archetype_master()
            .get_archetype(source_archetype_id)
            .expect("invariant: source_archetype_id is live (resolved from EntityInland)");
        let source_ids = source.component_ids();
        // Canonical-sortedness precondition (plan: `_dyn` union asserts):
        // `Archetype::create_by_ids` / the funnel guarantee sorted ids; the
        // union below relies on it only for the cheap post-sort, but a broken
        // seed would silently fork archetype identities — assert loudly.
        debug_assert!(
            source_ids.is_sorted_by_key(|c| c.0),
            "merged_archetype_id_dyn: source component ids must be canonical-sorted"
        );
        for &cid in source_ids {
            debug_assert!(
                len < MAX_MIGRATION_COLUMNS,
                "migration union exceeds MAX_COMPONENTS"
            );
            combined[len] = cid;
            len += 1;
        }
        // <-- the shared `world` borrow (via `source`) dies at block close,
        // freeing `world` for the `&mut` funnel call below. The generic
        // `merged_archetype_id` pays a `to_vec()` for the same decoupling;
        // the stack array makes this variant allocation-free (plan D9).
    }

    // Union: O(E × S) scan — `extra` is ≤ a handful of tag ids, archetypes
    // host ≤ 32 components typically (same rationale as the generic helper).
    for &cid in extra {
        if !combined[..len].contains(&cid) {
            debug_assert!(
                len < MAX_MIGRATION_COLUMNS,
                "migration union exceeds MAX_COMPONENTS"
            );
            combined[len] = cid;
            len += 1;
        }
    }

    // Canonical-sort so `find_exact_match` collapses equivalent unions to one
    // ArchetypeId regardless of insertion order (B1 + canonicalization).
    combined[..len].sort_unstable_by_key(|c| c.0);

    world.get_or_create_archetype(&combined[..len])
}

/// Resolves the destination archetype id for a dynamic id-keyed detach
/// (Phase 22 D9): `source \ removed`, resolved through the single
/// `get_or_create_archetype` funnel.
///
/// Allocation-free: the difference lives in a stack array bounded by
/// `MAX_MIGRATION_COLUMNS`.
///
/// **Precondition (caller-decided no-op)**: every id in `removed` IS hosted
/// by the source — the absent-tag no-op is decided BEFORE calling via a
/// presence test on the source signature (plan D9; contrast the generic
/// [`without_component_archetype_id`], which folds the test in and returns
/// `Option`).
///
/// **O3 — empty target**: removing every component yields `kept.is_empty()`
/// and routes to the EMPTY archetype — `get_or_create_archetype(&[])`
/// resolves it through the same funnel (the D5(2) rule mirrored; the funnel
/// supports the empty mask since Wave 1A).
#[inline]
pub(crate) fn without_ids_archetype_id(
    world: &mut EcsMaster,
    source_archetype_id: ArchetypeId,
    removed: &[ComponentId],
) -> ArchetypeId {
    let mut kept: [ComponentId; MAX_MIGRATION_COLUMNS] = [ComponentId(0); MAX_MIGRATION_COLUMNS];
    let mut len = 0usize;

    {
        let source = world
            .archetype_master()
            .get_archetype(source_archetype_id)
            .expect("invariant: source_archetype_id is live (resolved from EntityInland)");
        let source_ids = source.component_ids();
        debug_assert!(
            source_ids.is_sorted_by_key(|c| c.0),
            "without_ids_archetype_id: source component ids must be canonical-sorted"
        );
        debug_assert!(
            removed.iter().all(|cid| source_ids.contains(cid)),
            "without_ids_archetype_id: caller decides the absent-id no-op BEFORE \
             calling (presence test on the source signature)"
        );
        for &cid in source_ids {
            if !removed.contains(&cid) {
                // len < MAX_MIGRATION_COLUMNS holds: kept ⊆ source and the
                // source set already fits the bound.
                kept[len] = cid;
                len += 1;
            }
        }
        // <-- shared `world` borrow dies here (same decoupling as the union).
    }

    // The difference of a canonical-sorted set preserves canonical order —
    // assert the invariant the exact-mask funnel relies on (plan: `_dyn`
    // canonical-sortedness debug_asserts).
    debug_assert!(
        kept[..len].is_sorted_by_key(|c| c.0),
        "without_ids_archetype_id: kept set must stay canonical-sorted"
    );

    world.get_or_create_archetype(&kept[..len])
}

/// Performs the id-keyed attach migration `source → target` (Phase 22 D9):
/// `target = source ∪ added`, where every added id is a size-0 tag column
/// (debug-asserted via the registry layout). Structurally a clone of
/// [`migrate_entity_insert`] minus the bundle-byte-write step: retained
/// columns are memcpy'd with their original ticks, the added tag columns get
/// tick-init only (a tag has no bytes), and `on_add` / `on_insert` hooks +
/// observers fire for the added ids in Phase 2 with no live archetype
/// reborrow — Phase 14a §3.4 confinement replicated verbatim.
///
/// Caller guarantees:
/// * `source_archetype_id != target_archetype_id` (the present-tag in-place
///   path is decided before calling — [`retag_in_place`]);
/// * every id in `added` is hosted by `target` and NOT hosted by `source`
///   (`T = S ⊎ A`, debug-asserted);
/// * `entity` is live and resolves to the source archetype.
///
/// **O3 — zero-retained shape is first-class**: attaching FROM the empty
/// archetype means the source has zero pools — the retained-copy loop runs
/// zero times and no pool pointer is minted; `move_out_entity` over the
/// pool-less archetype releases only the `entity_ids` slot.
#[cold]
#[inline(never)]
pub(crate) fn migrate_entity_attach_ids(
    world: &mut EcsMaster,
    entity: Entity,
    source_archetype_id: ArchetypeId,
    target_archetype_id: ArchetypeId,
    added: &[ComponentId],
) {
    debug_assert_ne!(
        source_archetype_id, target_archetype_id,
        "migrate_entity_attach_ids: caller must filter the in-place path (retag_in_place)"
    );
    debug_assert!(
        !added.is_empty(),
        "migrate_entity_attach_ids: empty attach set is a caller bug"
    );
    // D9: this path skips byte-writes for `added` — sound ONLY for size-0
    // columns. A data component routed through here would leave its bytes
    // uninitialized; the registry layout is the oracle.
    debug_assert!(
        added
            .iter()
            .all(|cid| component_registry::get_layout(cid.0).is_some_and(|l| l.is_zst())),
        "migrate_entity_attach_ids: every added id must be a size-0 (tag) column"
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
        "migrate_entity_attach_ids: stale entity passed (caller must filter)"
    );
    let source_row = inland.unit_index() as usize;

    // EnableTag Step 6: borrow-free scratch for the migrating entity's enable
    // bits + the list of target tags whose first column had to be allocated (O2
    // bookkeeping fired after the block).
    let mut enable_scratch: SmallList4<(ComponentId, bool)> = SmallList4::new();
    let mut enable_newly_allocated: SmallList4<ComponentId> = SmallList4::new();

    // Phase 14a §3.4 (C2): the entire `source` / `target` `&mut` lifetime is
    // confined to this Phase-1 block. The EntityInland repoint is HOISTED INTO
    // it (it touches `world.entity_master`, not the archetypes), so after the
    // block the entity is fully in `target` and BOTH `&mut Archetype` are
    // dead — Phase 2 can mint `world_ptr` with no live reborrow (SAFETY-1).
    {
        // SAFETY (U1, U2, U14, SCH7, F1): mirrors `migrate_entity_insert` —
        //   * `source_ptr` / `target_ptr` carry write-capable, interior-mutable
        //     (`SharedReadWrite`, F4-rooted) slab provenance minted under
        //     `&mut self`; each survives sibling structural writes (incl. the
        //     OTHER archetype's `current_index` bump) under TB/SB because every
        //     slab element is a distinct `UnsafeCell`.
        //   * `source != target` (debug-asserted), so the two `&mut Archetype`
        //     reborrows alias disjoint slots.
        //   * `&mut EcsMaster` exclusivity prevents any sibling reader (SCH7).
        //   * The reborrows are confined to this block (Phase 1).
        let source: &mut Archetype = unsafe { &mut *source_ptr };
        let target: &mut Archetype = unsafe { &mut *target_ptr };

        // EnableTag Step 6 PHASE 1 (C4 READ-before-swap): snapshot the migrating
        // entity's enable bits BEFORE the source `move_out_entity` (Step 4)
        // relocates the source's OTHER rows' bits. 0%-gate via `is_empty()`.
        read_source_enable_bits(source, source_row, &mut enable_scratch);

        debug_assert!(
            added.iter().all(|&cid| !source.component_ids().contains(&cid)),
            "migrate_entity_attach_ids: added ids must be NEW to the source \
             (present-tag re-add is retag_in_place's job)"
        );
        debug_assert!(
            added.iter().all(|&cid| target.has_component_id(cid)),
            "migrate_entity_attach_ids: target must host every added id (T = S ∪ A)"
        );

        target.reserve_capacity(1).expect(
            "attach-migration: target pool reserve ceiling (rows) exhausted — \
             committed capacity grows on demand (Phase X.I), so this fires only \
             when the target archetype outgrows a pool's reserve_rows",
        );
        let new_row: u32 = target.current_index as u32;
        let row = target.current_index;

        // Step 1: copy every RETAINED column into the reserved target row.
        // The retained set is EXACTLY the source set (`T = S ⊎ A` by
        // precondition), so the loop walks `source.component_ids()` directly —
        // no scratch copy, no allocation. Attach FROM the empty archetype:
        // zero source columns ⇒ zero iterations, no pool pointers minted (O3).
        for &retained_cid in source.component_ids() {
            let src_pool = source
                .component_pools()
                .get_pool(retained_cid)
                .expect("invariant: source hosts its own component id");
            debug_assert!(
                source_row < src_pool.count(),
                "source_row out of bounds for retained component"
            );
            let stride = src_pool.component_layout().size();
            // SAFETY (Round 3 C-N2 / Phase 10 STORE3) — mirrors
            // `migrate_entity_insert` Step 1:
            //   * `source_row < src_pool.count()` (debug-asserted) ⇒
            //     `unit_ptr(source_row)` addresses an initialized arena slot.
            //   * The slice borrows the live `source` pool; it is consumed by
            //     the `write_at_unchecked_initialized` memcpy in this same
            //     iteration, before `source` is mutated (Step 3).
            //   * `&mut EcsMaster` ⇒ no concurrent writer; the tick sub-regions
            //     are committed for every row `< src_pool.count()` (Phase X.I).
            let bytes =
                unsafe { core::slice::from_raw_parts(src_pool.unit_ptr(source_row), stride) };
            // SAFETY: same conditions as the byte read above (committed,
            //   initialized row; exclusive world access).
            let added_tick = unsafe { src_pool.read_added_tick(source_row) };
            // SAFETY: same as above.
            let changed_tick = unsafe { src_pool.read_changed_tick(source_row) };

            let dst_pool = target
                .component_pools_mut()
                .get_pool_mut(retained_cid)
                .expect("invariant: retained component must exist in target (T = S ∪ A)");
            // SAFETY (mirrors `migrate_entity_insert` Step 1):
            //   * `row == target.current_index == dst_pool.count()` (pools grow
            //     in lockstep) and `reserve_capacity(1)` guaranteed a committed
            //     slot (Phase X.I Phase B) ⇒ `write_at_unchecked_initialized`
            //     targets a logically-uninit slot (no drop runs).
            //   * `bytes.len() == stride == dst_pool.component_layout().size()`
            //     (same `ComponentId` ⇒ same registry layout).
            //   * `&mut target` ⇒ exclusive access; `commit_units(row, 1)`
            //     extends the dense tail by one (pre: `row == count`), after
            //     which the tick writes stamp the ORIGINAL source ticks into
            //     the now-live slot.
            unsafe {
                dst_pool.write_at_unchecked_initialized(row, bytes);
                dst_pool.commit_units(row, 1);
                dst_pool.write_added_tick(row, added_tick);
                dst_pool.write_changed_tick(row, changed_tick);
            }
        }

        // Step 2: commit the ADDED tag columns. A size-0 column has no bytes —
        // the byte-write step of `migrate_entity_insert` vanishes; the
        // `write_at_unchecked_initialized` pre-write contract is vacuous for a
        // 0-byte row. `commit_units` pre: `row == count` (lockstep,
        // debug-asserted) with committed capacity covering `row`
        // (`reserve_capacity(1)` above). Both ticks land at `current_tick` —
        // uniform with a fresh insert (D1: ticks ARE maintained in tag pools,
        // so a future `DynAdded(TagId)` term needs no storage change).
        for &added_cid in added {
            let dst_pool = target
                .component_pools_mut()
                .get_pool_mut(added_cid)
                .expect("invariant: added id must exist in target (T = S ∪ A)");
            debug_assert_eq!(
                dst_pool.count(),
                row,
                "added tag pool out of lockstep with the target row"
            );
            dst_pool.commit_units(row, 1);
            dst_pool.fill_ticks(row, 1, current_tick);
        }

        // Step 3: archetype-side bookkeeping (mirrors `migrate_entity_insert`
        // Step 4): every target pool now holds one committed row at `row`, so
        // the entity-id list and `current_index` advance in lockstep.
        target.entity_ids.push(entity.id());
        target.current_index = row + 1;

        // EnableTag Step 6 PHASE 2: restore the snapshotted enable bits into the
        // target's new row while `target` is `&mut`-live.
        write_target_enable_bits(
            target,
            new_row as usize,
            &enable_scratch,
            &mut enable_newly_allocated,
        );

        // Step 4: release source's bytes WITHOUT drop (C5 + W-N2). Every
        // retained value was memcpy'd into target (Step 1) and now belongs to
        // the target row — dropping the source copy would double-free. The
        // added ids never existed in source; nothing to release for them.
        match source.move_out_entity(InlandPoolId(source_row)) {
            RemoveOutcome::Last => {}
            RemoveOutcome::Swapped { moved_entity } => {
                // The entity at source's last row took source_row's slot. Fix
                // its EntityInland.unit_index — touches `world.entity_master`,
                // NOT `source`/`target` (independent of the reborrows).
                if let Some(slot) = world.entity_master.entities_inland.get_mut(moved_entity.0) {
                    slot.set_unit_index(source_row as u32);
                }
            }
            RemoveOutcome::PoolFailure => {
                panic!("invariant: migration source removal must succeed");
            }
        }

        // Step 5 (HOISTED into Phase 1, §3.4): repoint `entity`'s EntityInland
        // at the target slot. Touches `world.entity_master` (not the
        // archetypes); after this the entity is fully in `target` and add /
        // insert hooks (Phase 2) read the NEW target row (Bevy-parity
        // asymmetry, §0).
        world.entity_master.entities_inland[entity.id().0] =
            EntityInland::new(target_ptr, new_row, entity.generation());
        // <-- `source` / `target` `&mut Archetype` DROP here (block close).
    }

    // EnableTag Step 6 O2: fire presence + `enable_generation` bookkeeping for
    // every target column the copy newly allocated (no `&mut Archetype` live).
    fire_enable_column_alloc_bookkeeping(world, target_archetype_id, &enable_newly_allocated);

    // PHASE 2 (§3.4): fire on_add, then on_insert, for the ADDED ids. The
    // entity is repointed into `target`; both `&mut Archetype` are dead —
    // only `target_ptr` (*mut, Copy) survives.
    //
    // SAFETY (F1): `target_ptr` is write-capable, stable, interior-mutable
    //   (`SharedReadWrite`, F4-rooted) slab provenance — it survived the
    //   Phase-1 push into `target` (which bumped `target.current_index`
    //   through a same-cell-derived pointer) under TB/SB because the whole
    //   slab element is `UnsafeCell`-wrapped. Reading `flags` is one `u16`
    //   load (no `&mut`).
    let flags = unsafe { (*target_ptr).flags };
    if !flags.is_empty() {
        // MINT: no `world`-derived `&mut Archetype` is live (SAFETY-1).
        let world_ptr = NonNull::from(&mut *world);
        // Ordering (SAFETY-2): ALL on_add, THEN ALL on_insert — every attached
        // id is newly added by precondition, so both kinds iterate `added`.
        // Per kind: hooks first, then observers (Phase 14b §5).
        if flags.contains(ArchetypeFlags::ON_ADD_ANY) {
            if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
                for &cid in added {
                    trigger_on_add(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_ADD_OBSERVER) {
                for &cid in added {
                    fire_on_add_observers(world_ptr, cid, entity);
                }
            }
        }
        if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
            if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                for &cid in added {
                    trigger_on_insert(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                for &cid in added {
                    fire_on_insert_observers(world_ptr, cid, entity);
                }
            }
        }
    }
    // NO drain (Q-A1): the caller owns the drain — `EcsMaster::add_tag` drains
    // at depth 0; the deferred `AddTagCommand` path delegates to `add_tag`,
    // whose drain no-ops at depth >= 1 (the outermost drive drains).
}

/// Performs the id-keyed detach migration `source → target` (Phase 22 D9):
/// `target = source \ removed`. Mirrors [`migrate_entity_remove`] (Phase 14a
/// §3.5 confinement) generalized to an id set: retained bytes are collected
/// while the source is live, pushed via `create_entity_with_ticks`, the
/// `on_replace` / `on_remove` hooks + observers fire for each removed id
/// against the DYING source row (EntityInland still points at source), then
/// each removed id is dropped exactly once (`drop_fn` runs uniformly when
/// present — covering Drop-impl ZSTs) and the source row is released without
/// drop.
///
/// Caller guarantees:
/// * `source_archetype_id != target_archetype_id`;
/// * every id in `removed` IS hosted by the source and NOT by the target
///   (`T = S \ R`, debug-asserted) — the absent-id no-op is decided BEFORE
///   calling (presence test on the source signature);
/// * `entity` is live and resolves to the source archetype.
///
/// **O3 — detach-to-empty is first-class**: removing the last component(s)
/// pushes a zero-component row into the EMPTY archetype
/// (`create_entity_with_ticks` with an empty retained slice — the Wave-1A
/// `current_index`-driven row fix makes this exact).
#[cold]
#[inline(never)]
pub(crate) fn migrate_entity_detach_ids(
    world: &mut EcsMaster,
    entity: Entity,
    source_archetype_id: ArchetypeId,
    target_archetype_id: ArchetypeId,
    removed: &[ComponentId],
) {
    debug_assert_ne!(
        source_archetype_id, target_archetype_id,
        "migrate_entity_detach_ids: caller must filter the no-op case"
    );
    debug_assert!(
        !removed.is_empty(),
        "migrate_entity_detach_ids: empty detach set is a caller bug"
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
        "migrate_entity_detach_ids: stale entity passed (caller must filter)"
    );
    let source_row = inland.unit_index() as usize;

    // EnableTag Step 6: borrow-free scratch for the migrating entity's enable
    // bits + the list of target tags whose first column had to be allocated (O2
    // bookkeeping fired after Phase 3, when no `&mut Archetype` is live).
    let mut enable_scratch: SmallList4<(ComponentId, bool)> = SmallList4::new();
    let mut enable_newly_allocated: SmallList4<ComponentId> = SmallList4::new();

    // PHASE 1 (§3.5 / C2): confine `source` / `target` `&mut` to this block —
    // collect retained byte slices and push them into `target`. After the push
    // the entity exists in BOTH the source row (removed ids still live) and
    // the target row; `EntityInland` STILL points at SOURCE (the repoint is
    // Phase 3). The block produces only `new_row` (Copy); both `&mut
    // Archetype` drop at its close, so Phase 2 mints `world_ptr` with no live
    // reborrow.
    let new_row: u32 = {
        // SAFETY: same rationale as `migrate_entity_insert` (U1, U2, U14,
        //   SCH7, F1) — `source_ptr` / `target_ptr` are interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance that survives the
        //   sibling `current_index` bump under TB/SB (whole slab element is
        //   `UnsafeCell`-wrapped); confined to Phase 1.
        let source: &mut Archetype = unsafe { &mut *source_ptr };
        let target: &mut Archetype = unsafe { &mut *target_ptr };

        // EnableTag Step 6 PHASE 1 (C4 READ-before-swap): snapshot the migrating
        // entity's enable bits at `source_row` BEFORE the Phase-3
        // `move_out_entity` relocates the source's OTHER rows' bits. The scratch
        // is borrow-free (W3-r6). 0%-gate via `is_empty()`.
        read_source_enable_bits(source, source_row, &mut enable_scratch);

        debug_assert!(
            removed.iter().all(|&cid| source.has_component_id(cid)),
            "migrate_entity_detach_ids: every removed id must be hosted by the source \
             (caller decides the absent-id no-op)"
        );
        debug_assert!(
            removed.iter().all(|&cid| !target.has_component_id(cid)),
            "migrate_entity_detach_ids: target must not host any removed id (T = S \\ R)"
        );

        // Step 1: collect retained byte slices. The retained set is
        // `source \ removed` — walked in SOURCE order, which equals the
        // target's canonical order (difference preserves sortedness), so no
        // target borrow and no scratch copy are needed (allocation-free in
        // this fn; `create_entity_with_ticks`'s internal tick-strip Vec is
        // pre-existing shared machinery). Same cast-via-`*mut u8` E0521
        // workaround as `migrate_entity_remove`.
        type DetachRetainedSlot<'b> = MaybeUninit<(ComponentId, &'b [u8], Tick, Tick)>;
        let mut retained: [DetachRetainedSlot<'_>; MAX_MIGRATION_COLUMNS] =
            [const { MaybeUninit::uninit() }; MAX_MIGRATION_COLUMNS];
        let retained_base: *mut u8 = retained.as_mut_ptr() as *mut u8;
        let retained_stride: usize = mem::size_of::<DetachRetainedSlot<'_>>();
        let mut retained_count = 0usize;
        for &retained_cid in source.component_ids() {
            if removed.contains(&retained_cid) {
                continue; // dropped in Phase 3, not migrated
            }
            let pool = source
                .component_pools()
                .get_pool(retained_cid)
                .expect("invariant: source hosts its own component id");
            let stride = pool.component_layout().size();
            // SAFETY (Round 3 C-N2): see `migrate_entity_insert` retained-bytes
            //   block — `source_row` addresses an initialized arena slot; the
            //   slice borrows the live `source` pool and is consumed by
            //   `create_entity_with_ticks` below, before `source` is mutated
            //   (Phase 3).
            let bytes = unsafe { core::slice::from_raw_parts(pool.unit_ptr(source_row), stride) };
            // SAFETY (Phase 10): same conditions as in the insert path.
            let added = unsafe { pool.read_added_tick(source_row) };
            // SAFETY: same as above.
            let changed = unsafe { pool.read_changed_tick(source_row) };
            debug_assert!(retained_count < MAX_MIGRATION_COLUMNS);
            // SAFETY (E0521 workaround): cast-via-`*mut u8` keeps the inner
            //   `&[u8]` lifetime bound to the current iteration's `source`
            //   reborrow (mirrors `migrate_entity_remove` verbatim).
            unsafe {
                let slot_ptr = retained_base.add(retained_count * retained_stride)
                    as *mut DetachRetainedSlot<'_>;
                slot_ptr.write(MaybeUninit::new((retained_cid, bytes, added, changed)));
            }
            retained_count += 1;
        }

        // SAFETY: retained[0..retained_count] are all written; cast
        //   `*const DetachRetainedSlot<'_>` → `*const (ComponentId, &[u8],
        //   Tick, Tick)` is layout-compatible (`MaybeUninit<T>` is
        //   `repr(transparent)` over `T`).
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
            "detach-migration: target archetype rejected the push — pool reserve \
             ceiling (rows) exhausted (committed capacity grows on demand per \
             Phase X.I) or signature mismatch",
        );

        // EnableTag Step 6 PHASE 2: restore the snapshotted enable bits into the
        // target's new row while `target` is `&mut`-live.
        write_target_enable_bits(
            target,
            new_row as usize,
            &enable_scratch,
            &mut enable_newly_allocated,
        );

        new_row
        // <-- `source` / `target` `&mut Archetype` DROP here (block close).
    };

    // PHASE 2 (§3.5 / §0): fire `on_replace` + `on_remove` for every removed
    // id reading the SOURCE (dying) row. `EntityInland` STILL points at SOURCE
    // (the repoint is Phase 3), so a hook's read-only view reads the dying
    // bytes — the §0 asymmetry vs the attach path (which reads the NEW target
    // row). Both `&mut Archetype` are dead; only `source_ptr` (*mut, Copy)
    // survives.
    //
    // SAFETY (F1): `source_ptr` is write-capable, stable, interior-mutable
    //   (`SharedReadWrite`, F4-rooted) slab provenance — it survived the
    //   Phase-1 push into `target` (a sibling `current_index` bump) under
    //   TB/SB because the whole slab element is `UnsafeCell`-wrapped. Reading
    //   `flags` is one `u16` load (no `&mut`).
    let flags = unsafe { (*source_ptr).flags };
    if flags.contains(ArchetypeFlags::ON_REPLACE_ANY)
        || flags.contains(ArchetypeFlags::ON_REMOVE_ANY)
    {
        // MINT: no `world`-derived `&mut Archetype` is live (SAFETY-1).
        let world_ptr = NonNull::from(&mut *world);
        // PRE-`drop_at` (SAFETY-2): ALL on_replace, THEN ALL on_remove, over
        // the removed id set. Per kind: hooks first, then observers (§5).
        if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
            for &cid in removed {
                trigger_on_replace(world_ptr, cid, entity);
            }
        }
        if flags.contains(ArchetypeFlags::ON_REPLACE_OBSERVER) {
            for &cid in removed {
                fire_on_replace_observers(world_ptr, cid, entity);
            }
        }
        if flags.contains(ArchetypeFlags::ON_REMOVE_HOOK) {
            for &cid in removed {
                trigger_on_remove(world_ptr, cid, entity);
            }
        }
        if flags.contains(ArchetypeFlags::ON_REMOVE_OBSERVER) {
            for &cid in removed {
                fire_on_remove_observers(world_ptr, cid, entity);
            }
        }
    }

    // PHASE 3 (§3.5): re-resolve `&mut source`; drop every removed id ONCE;
    // release the source row without drop; repoint to target. Deferred
    // commands the hooks enqueued do NOT apply until the outermost drain
    // (Q-A1), so nothing can mutate the source archetype between Phase 2 and
    // here — the `RemoveOutcome::Swapped` fixup cannot observe a stale
    // `source_row`.
    {
        // SAFETY (F1): re-resolved AFTER the Phase-2 hooks returned;
        //   `source_ptr` is write-capable, stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — it survived the
        //   Phase-1 sibling `current_index` bump under TB/SB (whole slab
        //   element is `UnsafeCell`-wrapped); single-threaded `&mut EcsMaster`
        //   (SCH7).
        let source: &mut Archetype = unsafe { &mut *source_ptr };

        // C5 discipline: each removed id's bytes are STILL owned by source.
        // Drop them explicitly before `move_out_entity` (which skips drop per
        // W-N2) — without this, a Drop-impl component (incl. a Drop-impl ZST
        // tag, D9) would leak. Dynamic tags carry `drop_fn: None`, making
        // `drop_at` a uniform no-op for them.
        for &removed_cid in removed {
            let removed_pool = source
                .component_pools_mut()
                .get_pool_mut(removed_cid)
                .expect("invariant: source hosts every removed id (verified by caller)");
            // SAFETY (C5):
            //   * `source_row < removed_pool.count()` (initialized slot).
            //   * `&mut source` gives exclusive access to every owned pool.
            //   * After drop_at, the slot is logically uninit; the subsequent
            //     `move_out_entity` swap-removes it bytewise (no drop) — so
            //     each removed value is dropped EXACTLY once across the
            //     dual-presence window.
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

    // EnableTag Step 6 O2: fire presence + `enable_generation` bookkeeping for
    // every target column the copy newly allocated (after the Phase-3
    // `&mut source` dropped — no live `&mut Archetype` reborrow).
    fire_enable_column_alloc_bookkeeping(world, target_archetype_id, &enable_newly_allocated);
    // NO drain (Q-A1): the caller owns the drain — see `migrate_entity_attach_ids`.
}

/// In-place re-attach of already-present tag columns (Phase 22 D8): fires
/// `on_replace` (pre) + `on_insert` (post) and stamps the changed tick —
/// uniform with data replace semantics
/// ([`InsertCommand::apply_replace_in_place`](crate::ecs::core::commands::insert_command::InsertCommand)),
/// minus the byte overwrite (a tag has no bytes) and minus the drop (no old
/// value to destroy). `added_tick` is intentionally NOT bumped (EC9 / OQ5).
/// `on_add` does NOT fire — the column is already present (Q7).
///
/// Caller guarantees `entity` is live and every id in `ids` is hosted by its
/// archetype (presence test on the source signature).
#[cold]
#[inline(never)]
pub(crate) fn retag_in_place(world: &mut EcsMaster, entity: Entity, ids: &[ComponentId]) {
    let current_tick = world.current_tick();
    let inland = world.entity_master.entities_inland[entity.id().0];
    debug_assert!(
        !inland.is_null() && inland.generation() == entity.generation(),
        "retag_in_place: stale entity passed (caller must filter)"
    );
    let archetype_ptr = inland.archetype_ptr();
    let row = inland.unit_index() as usize;

    // SAFETY (F1): `archetype_ptr` is write-capable, stable, interior-mutable
    //   (`SharedReadWrite`, F4-rooted) slab provenance — it survives sibling
    //   structural writes under TB/SB (the whole slab element is
    //   `UnsafeCell`-wrapped). Reading `flags` is one `u16` load (no `&mut`).
    let flags = unsafe { (*archetype_ptr).flags };

    // PRE (Q7): on_replace while the row still holds the "old" value. For a
    // tag the value is the empty byte string — the window is kept for ordering
    // parity with the data replace path. No `&mut Archetype` is live here
    // (only `archetype_ptr`, raw), so minting `world_ptr` aliases no reborrow
    // (SAFETY-1).
    if flags.contains(ArchetypeFlags::ON_REPLACE_ANY) {
        let world_ptr = NonNull::from(&mut *world);
        if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
            for &cid in ids {
                trigger_on_replace(world_ptr, cid, entity);
            }
        }
        if flags.contains(ArchetypeFlags::ON_REPLACE_OBSERVER) {
            for &cid in ids {
                fire_on_replace_observers(world_ptr, cid, entity);
            }
        }
    }

    for &cid in ids {
        // SAFETY (U1, U2, U14, SCH7, F1): mirrors `apply_replace_in_place` —
        //   `archetype_ptr` is write-capable, stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance; `&mut EcsMaster`
        //   (held by the caller) bars sibling readers; the `&mut Archetype`
        //   reborrow is scoped to this single iteration and is dead before the
        //   post-fire below mints `world_ptr`.
        let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
        let pool = archetype
            .component_pools_mut()
            .get_pool_mut(cid)
            .expect("invariant: caller verified the id is hosted (source-signature presence test)");
        debug_assert!(pool.has_row(row), "retag_in_place: row out of bounds");
        // SAFETY (STORE3): `row < pool.count()` (debug-asserted via `has_row`);
        //   exclusive access for this iteration. Stamps the changed tick only —
        //   replace semantics leave `added_tick` untouched (EC9 / OQ5).
        unsafe { pool.write_changed_tick(row, current_tick) };
    }

    // POST (Q7): on_insert now that the row is "re-inserted". The per-
    // iteration `&mut *archetype_ptr` reborrows are dead; only the raw
    // pointer survives (SAFETY-1).
    if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
        let world_ptr = NonNull::from(&mut *world);
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
    // NO drain (Q-A1): the caller owns the drain — see `migrate_entity_attach_ids`.
}

// ═════════════════════════════════════════════════════════════════════════════
// EnableTag Step 6 — cross-archetype enable-bit copy tests.
//
// Fixed component ids live in the grep-verified-free block [328, 340) (disjoint
// from the 320-327 EnableTag-test usage and every other test fixed-id range).
// The tests drive the four `pub(crate)` migration helpers DIRECTLY so the
// source/target archetypes and the migrating entity's row are fully controlled;
// the post-migration target row is read back from `entities_inland`.
// ═════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod step6_enable_migration_tests {
    use std::collections::{HashMap, HashSet};

    use proptest::prelude::*;

    use super::*;
    use crate::ecs::core::component::component_registry::{self, StorageKind};
    use crate::ecs::identifiers::primitives::ComponentId;

    // ── Fixed ids: free sub-block [335, 340) ─────────────────────────────────
    //
    // The shared lib-test process registers ComponentIds process-globally, so
    // these must be disjoint from EVERY other `#[cfg(test)]` module in the lib.
    // The Step-7 wave-mate's `filter_enable.rs` claims 328-330 inside this same
    // lib-test binary, so this Step-6 module uses the upper sub-block 335-339
    // (grep-verified free in `src/`) to avoid the Wave-2-class full-suite-only
    // id-collision regression.
    //
    // The `migrate_entity_insert` helper is generic over `B: Bundle`, and the
    // sealed `Bundle` trait can only be implemented by `#[derive(Bundle)]`, whose
    // generated code references the EXTERNAL `boyko_ecs` crate path — unavailable
    // inside this crate. The insert path is therefore covered by the companion
    // integration test `tests/enable_tag_migration_step6.rs` (which drives the
    // public `EntityCommands::insert` + a dynamic enable tag, verified via the
    // public `is_enabled_id`). These in-crate unit tests cover the other three
    // helpers (`remove` / `attach_ids` / `detach_ids`) DIRECTLY; `attach_ids`
    // shares the insert path's exact single-block READ-then-WRITE Step-6 shape.
    const SRC_DATA: ComponentId = ComponentId(335); // table anchor (source+target)
    const BUNDLE_DATA: ComponentId = ComponentId(336); // removable data component
    const TAG_ENABLE: ComponentId = ComponentId(337); // enable tag (Bitset), copied bit
    const TAG_ENABLE2: ComponentId = ComponentId(338); // second enable tag (Bitset)
    const NORMAL_ZST_TAG: ComponentId = ComponentId(339); // Table ZST tag (attach/detach id)

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SrcData(u32);
    impl Component for SrcData {
        fn component_id() -> ComponentId {
            SRC_DATA
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct BundleData(u32);
    impl Component for BundleData {
        fn component_id() -> ComponentId {
            BUNDLE_DATA
        }
    }

    /// Enable-tag flag types (classified `StorageKind::Bitset`, mirroring what
    /// `#[component(storage = "bitset")]` emits — filtered out of every archetype
    /// signature, so they never enter a migration's component union).
    #[repr(C)]
    struct TagEnable;
    impl Component for TagEnable {
        fn component_id() -> ComponentId {
            TAG_ENABLE
        }
    }
    #[repr(C)]
    struct TagEnable2;
    impl Component for TagEnable2 {
        fn component_id() -> ComponentId {
            TAG_ENABLE2
        }
    }

    /// Normal-storage ZST tag (`StorageKind::Table`, size 0): the id attached /
    /// detached by the dynamic `attach_ids` / `detach_ids` migrations.
    #[repr(C)]
    struct NormalZstTag;
    impl Component for NormalZstTag {
        fn component_id() -> ComponentId {
            NORMAL_ZST_TAG
        }
    }

    fn register() {
        component_registry::register_layout::<SrcData>(SRC_DATA.0);
        component_registry::register_layout::<BundleData>(BUNDLE_DATA.0);
        component_registry::register_layout::<TagEnable>(TAG_ENABLE.0);
        component_registry::register_layout::<TagEnable2>(TAG_ENABLE2.0);
        component_registry::register_layout::<NormalZstTag>(NORMAL_ZST_TAG.0);
        component_registry::set_storage_kind(TAG_ENABLE.0, StorageKind::Bitset);
        component_registry::set_storage_kind(TAG_ENABLE2.0, StorageKind::Bitset);
    }

    /// Spawns one `SrcData` entity into `arch`.
    fn spawn_src(ecs: &mut EcsMaster, arch: ArchetypeId, v: u32) -> Entity {
        let d = SrcData(v);
        // SAFETY (test): `d` outlives the borrow; byte view of a `#[repr(C)]`.
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &d as *const _ as *const u8,
                core::mem::size_of::<SrcData>(),
            )
        };
        ecs.create_entity(arch, &[(SRC_DATA, bytes)])
            .expect("spawn must succeed")
    }

    /// Reads `entity`'s current `(archetype_ptr, row)` from its inland slot.
    fn current_loc(ecs: &EcsMaster, e: Entity) -> (*mut Archetype, usize) {
        let inland = ecs.entity_master.entities_inland[e.id().0];
        assert!(!inland.is_null() && inland.generation() == e.generation());
        (inland.archetype_ptr(), inland.unit_index() as usize)
    }

    /// Tests the enable bit for `tag` at `entity`'s current row.
    fn entity_bit(ecs: &EcsMaster, e: Entity, tag: ComponentId) -> bool {
        let (arch_ptr, row) = current_loc(ecs, e);
        // SAFETY (test): `arch_ptr` is live slab provenance (generation-matched);
        //   a shared reborrow reading an `AtomicU64` enable bit — no `&mut`.
        let arch = unsafe { &*arch_ptr };
        match arch.enable_store.column(tag) {
            Some(col) => col.test(row),
            None => false,
        }
    }

    // ── Per-helper bit-survival tests ────────────────────────────────────────

    #[test]
    fn remove_migration_preserves_enable_bit() {
        register();
        let mut ecs = EcsMaster::new();
        // Source hosts SrcData + a real data component so the remove has a
        // distinct target. Use BundleData as the removable component.
        let src = ecs.create_archetype(&[SRC_DATA, BUNDLE_DATA]);
        // Spawn with both components.
        let d = SrcData(1);
        let b = BundleData(2);
        // SAFETY (test): both locals outlive the borrow; byte views of #[repr(C)].
        let sd = unsafe {
            core::slice::from_raw_parts(&d as *const _ as *const u8, core::mem::size_of::<SrcData>())
        };
        let bd = unsafe {
            core::slice::from_raw_parts(
                &b as *const _ as *const u8,
                core::mem::size_of::<BundleData>(),
            )
        };
        let e = ecs
            .create_entity(src, &[(SRC_DATA, sd), (BUNDLE_DATA, bd)])
            .expect("spawn");
        ecs.enable::<TagEnable>(e);

        let target = without_component_archetype_id::<BundleData>(&mut ecs, src)
            .expect("BundleData is hosted by source");
        assert_ne!(src, target);
        migrate_entity_remove::<BundleData>(&mut ecs, e, src, target);

        assert!(
            entity_bit(&ecs, e, TAG_ENABLE),
            "remove-migration must copy the enable bit to the target append row"
        );
    }

    #[test]
    fn attach_ids_migration_preserves_enable_bit() {
        register();
        let mut ecs = EcsMaster::new();
        let src = ecs.create_archetype(&[SRC_DATA]);
        let e = spawn_src(&mut ecs, src, 5);
        ecs.enable::<TagEnable>(e);

        let added = [NORMAL_ZST_TAG];
        let target = merged_archetype_id_dyn(&mut ecs, src, &added);
        assert_ne!(src, target);
        migrate_entity_attach_ids(&mut ecs, e, src, target, &added);

        assert!(
            entity_bit(&ecs, e, TAG_ENABLE),
            "attach_ids-migration must copy the enable bit to the target append row"
        );
    }

    #[test]
    fn detach_ids_migration_preserves_enable_bit() {
        register();
        let mut ecs = EcsMaster::new();
        let src = ecs.create_archetype(&[SRC_DATA, NORMAL_ZST_TAG]);
        // Spawn with the ZST tag present.
        let d = SrcData(3);
        // SAFETY (test): local outlives the borrow; byte view of a #[repr(C)].
        let sd = unsafe {
            core::slice::from_raw_parts(&d as *const _ as *const u8, core::mem::size_of::<SrcData>())
        };
        let e = ecs
            .create_entity(src, &[(SRC_DATA, sd), (NORMAL_ZST_TAG, &[])])
            .expect("spawn");
        ecs.enable::<TagEnable>(e);

        let removed = [NORMAL_ZST_TAG];
        let target = without_ids_archetype_id(&mut ecs, src, &removed);
        assert_ne!(src, target);
        migrate_entity_detach_ids(&mut ecs, e, src, target, &removed);

        assert!(
            entity_bit(&ecs, e, TAG_ENABLE),
            "detach_ids-migration must copy the enable bit to the target append row"
        );
    }

    // ── Multi-tag + clear-bit fidelity ──────────────────────────────────────

    #[test]
    fn attach_migration_preserves_multiple_tags_and_clears() {
        register();
        let mut ecs = EcsMaster::new();
        let src = ecs.create_archetype(&[SRC_DATA]);
        let e = spawn_src(&mut ecs, src, 1);
        // TagEnable set, TagEnable2 explicitly toggled then cleared (allocating
        // its source column so the snapshot carries a `false` for it).
        ecs.enable::<TagEnable>(e);
        ecs.enable::<TagEnable2>(e);
        ecs.disable::<TagEnable2>(e);
        assert!(ecs.is_enabled::<TagEnable>(e));
        assert!(!ecs.is_enabled::<TagEnable2>(e));

        let added = [NORMAL_ZST_TAG];
        let target = merged_archetype_id_dyn(&mut ecs, src, &added);
        migrate_entity_attach_ids(&mut ecs, e, src, target, &added);

        assert!(entity_bit(&ecs, e, TAG_ENABLE), "set tag must survive");
        assert!(
            !entity_bit(&ecs, e, TAG_ENABLE2),
            "cleared tag must stay cleared at the target (no spurious set)"
        );
    }

    // ── 0%-gate: enable-free migration must not allocate an enable column ─────

    #[test]
    fn no_enable_entity_migration_skips_copy() {
        register();
        let mut ecs = EcsMaster::new();
        let src = ecs.create_archetype(&[SRC_DATA]);
        let e = spawn_src(&mut ecs, src, 4);
        // No enable toggle: the source EnableStore stays empty.
        let enable_before = ecs.archetype_master().enable_generation();

        let added = [NORMAL_ZST_TAG];
        let target = merged_archetype_id_dyn(&mut ecs, src, &added);
        migrate_entity_attach_ids(&mut ecs, e, src, target, &added);

        // The fast path skipped the copy: no column allocated anywhere, so
        // enable_generation is unmoved (the byte-identical-to-pre-Step-6 path).
        assert_eq!(
            ecs.archetype_master().enable_generation(),
            enable_before,
            "an enable-free migration must not allocate a column / bump enable_generation"
        );
        let (arch_ptr, _) = current_loc(&ecs, e);
        // SAFETY (test): live slab provenance, shared reborrow.
        let arch = unsafe { &*arch_ptr };
        assert!(
            arch.enable_store.is_empty(),
            "target EnableStore must stay empty for an enable-free migration"
        );
    }

    // ── Single-fire: enable_generation bumps exactly once per new column ──────

    #[test]
    fn single_fire_enable_generation_bumps_once_per_migration() {
        register();
        let mut ecs = EcsMaster::new();
        let src = ecs.create_archetype(&[SRC_DATA]);
        let e = spawn_src(&mut ecs, src, 2);
        ecs.enable::<TagEnable>(e); // first source column → one bump already counted
        let before = ecs.archetype_master().enable_generation();

        let added = [NORMAL_ZST_TAG];
        let target = merged_archetype_id_dyn(&mut ecs, src, &added);
        migrate_entity_attach_ids(&mut ecs, e, src, target, &added);

        // Exactly one NEW column was allocated (the target's TagEnable column), so
        // the migration bookkeeping fires exactly once — never double-counted
        // (O1-r7: the source swap-fix in move_out_entity allocates no column).
        assert_eq!(
            ecs.archetype_master().enable_generation(),
            before + 1,
            "a single-tag migration must bump enable_generation exactly once (the new \
             target column), proving the O2 bookkeeping is not double-fired"
        );

        // A SECOND migration of another entity carrying the same tag from `src`
        // into the SAME target reuses the existing target column → no further bump.
        let e2 = spawn_src(&mut ecs, src, 3);
        ecs.disable::<TagEnable>(e2); // touches the source column only (already present)
        ecs.enable::<TagEnable>(e2);
        let before2 = ecs.archetype_master().enable_generation();
        let target2 = merged_archetype_id_dyn(&mut ecs, src, &added);
        assert_eq!(target, target2, "same union resolves to the same target archetype");
        migrate_entity_attach_ids(&mut ecs, e2, src, target2, &added);
        assert_eq!(
            ecs.archetype_master().enable_generation(),
            before2,
            "reusing the target's existing column must NOT bump enable_generation again"
        );
    }

    // ── Cross-page: source row in page 0, target append in page 1 (>4096) ─────

    #[test]
    fn cross_page_migration_source_page0_target_page1() {
        register();
        let mut ecs = EcsMaster::new();
        let src = ecs.create_archetype(&[SRC_DATA]);
        // Pre-fill the target archetype past one 4096-row page so the migrated
        // entity's append row lands in page 1.
        let target = ecs.create_archetype(&[SRC_DATA, NORMAL_ZST_TAG]);
        let fill = ROWS_PER_PAGE_TEST + 1; // 4097 rows in target
        for i in 0..fill {
            let d = SrcData(i as u32);
            // SAFETY (test): local outlives the borrow; #[repr(C)] byte view.
            let sd = unsafe {
                core::slice::from_raw_parts(
                    &d as *const _ as *const u8,
                    core::mem::size_of::<SrcData>(),
                )
            };
            ecs.create_entity(target, &[(SRC_DATA, sd), (NORMAL_ZST_TAG, &[])])
                .expect("fill spawn");
        }

        // Migrating entity: source row 0 (page 0), enable bit set.
        let e = spawn_src(&mut ecs, src, 99);
        ecs.enable::<TagEnable>(e);
        let (_, src_row) = current_loc(&ecs, e);
        assert_eq!(src_row, 0, "migrating entity is at source page 0");

        let added = [NORMAL_ZST_TAG];
        migrate_entity_attach_ids(&mut ecs, e, src, target, &added);

        let (_, target_row) = current_loc(&ecs, e);
        assert!(
            target_row >= ROWS_PER_PAGE_TEST,
            "target append row {target_row} must be in page 1 (>= {ROWS_PER_PAGE_TEST})"
        );
        assert!(
            entity_bit(&ecs, e, TAG_ENABLE),
            "the enable bit must survive a cross-page (page0 -> page1) migration"
        );
    }

    // Local mirror of `ROWS_PER_PAGE` (the enable_store paging constant) so the
    // cross-page test reads naturally without importing the private const.
    const ROWS_PER_PAGE_TEST: usize = 4096;

    // ── proptest oracle: READ-before-swap correctness under interleaving ──────
    //
    // Models a sequence of (toggle, migrate, despawn-causing-swap) operations on
    // a small entity set and checks the engine's per-entity enable bit against a
    // ground-truth `HashMap<Entity, HashSet<tag>>`. The migrate step exercises
    // the Step-6 copy; the despawn step exercises the source swap-fix that the
    // copy's READ must precede.

    #[derive(Clone, Debug)]
    enum Op {
        Enable(usize),
        Disable(usize),
        Migrate(usize),
        Despawn(usize),
    }

    fn op_strategy(n: usize) -> impl Strategy<Value = Op> {
        prop_oneof![
            (0..n).prop_map(Op::Enable),
            (0..n).prop_map(Op::Disable),
            (0..n).prop_map(Op::Migrate),
            (0..n).prop_map(Op::Despawn),
        ]
    }

    proptest! {
        // `failure_persistence: None` keeps proptest from touching the
        // filesystem, so the test is runnable under Miri's `-Zmiri-disable-
        // isolation`-free default (the persistence file write is the only fs op).
        #![proptest_config(ProptestConfig {
            cases: 48,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]
        #[test]
        fn enable_bit_survives_migration_and_swap_oracle(
            ops in {
                const N: usize = 6;
                proptest::collection::vec(op_strategy(N), 1..40)
            }
        ) {
            register();
            let mut ecs = EcsMaster::new();
            let src = ecs.create_archetype(&[SRC_DATA]);

            // Live entities indexed by their original spawn slot; None once
            // despawned. The oracle tracks the set of set tags per slot.
            const N: usize = 6;
            let mut ents: Vec<Option<Entity>> = Vec::with_capacity(N);
            let mut oracle: HashMap<usize, HashSet<ComponentId>> = HashMap::new();
            for i in 0..N {
                let e = spawn_src(&mut ecs, src, i as u32);
                ents.push(Some(e));
                oracle.insert(i, HashSet::new());
            }

            for op in ops {
                match op {
                    Op::Enable(i) => {
                        if let Some(e) = ents[i] {
                            ecs.enable::<TagEnable>(e);
                            oracle.get_mut(&i).unwrap().insert(TAG_ENABLE);
                        }
                    }
                    Op::Disable(i) => {
                        if let Some(e) = ents[i] {
                            ecs.disable::<TagEnable>(e);
                            oracle.get_mut(&i).unwrap().remove(&TAG_ENABLE);
                        }
                    }
                    Op::Migrate(i) => {
                        if let Some(e) = ents[i] {
                            // Only migrate from `src` (single-hop): resolve a
                            // distinct target and copy the bit.
                            let cur = current_loc(&ecs, e).0;
                            // SAFETY (test): live slab provenance, shared read.
                            let cur_id = unsafe { (*cur).id() };
                            let target = merged_archetype_id_dyn(
                                &mut ecs, cur_id, &[NORMAL_ZST_TAG],
                            );
                            if target != cur_id {
                                migrate_entity_attach_ids(
                                    &mut ecs, e, cur_id, target, &[NORMAL_ZST_TAG],
                                );
                            }
                        }
                    }
                    Op::Despawn(i) => {
                        if let Some(e) = ents[i] {
                            ecs.despawn_without_children(e);
                            ents[i] = None;
                            oracle.remove(&i);
                        }
                    }
                }
            }

            // Verify every live entity's enable bit matches the oracle.
            for (i, slot) in ents.iter().enumerate() {
                if let Some(e) = slot {
                    let want = oracle.get(&i).map(|s| s.contains(&TAG_ENABLE)).unwrap_or(false);
                    let got = entity_bit(&ecs, *e, TAG_ENABLE);
                    prop_assert_eq!(
                        got, want,
                        "entity slot {} enable bit mismatch (got {}, want {})",
                        i, got, want
                    );
                }
            }
        }
    }
}
