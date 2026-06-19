//! Clone materialization — Algorithm A (Feature 3, plan §5).
//!
//! Produces a brand-new entity carrying clones of a source's components:
//! filtered id set → require-closure (C2) → target archetype → row push →
//! single-pass per-component clone (batch vs fn-ptr) → missing-required
//! reconstruct (C2) → gated fire. The `CloneRowGuard` (W5) gives strong
//! exception safety WITHOUT `catch_unwind`.
//!
//! # Soundness anchors
//!
//! * **S4 single-pass**: each component's `src_ptr → dst_ptr` is consumed inside
//!   its own iteration, never collected-then-read after a structural op.
//! * **S5 / W5 rollback**: the guard holds ONLY `target_archetype_ptr + new_row +
//!   committed`, all archetype-local slab provenance; on unwind it drops the
//!   already-cloned components via `drop_at` and uncommits the row. It NEVER
//!   touches `entity_master` — the entity→inland mapping is committed only AFTER
//!   materialization fully succeeds (so on the panic path the entity was never
//!   mapped). No cached world pointer is written in `Drop`.
//! * **S6 / F2 fire**: `world_ptr` is minted only after every `&mut Archetype` /
//!   `&mut pool` reborrow has dropped.

use std::ptr::NonNull;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::clone::cloner::EntityCloner;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::component::component_registry::{self, Cloneability, MAX_COMPONENTS};
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::hooks::dispatch::{trigger_on_add, trigger_on_insert};
use crate::ecs::core::component::observers::dispatch::{
    fire_on_add_observers, fire_on_insert_observers,
};
use crate::ecs::core::component::observers::ObserverKind;
use crate::ecs::core::component::observers::entity_store::fire_entity_observers;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

/// Stack capacity for the filtered / target id arrays. Matches
/// `MAX_MIGRATION_COLUMNS` (= `MAX_COMPONENTS`): any archetype the engine supports
/// fits on the stack without spilling to the heap.
const MAX_CLONE_COLUMNS: usize = MAX_COMPONENTS;

/// RAII rollback guard for a partially-materialized clone row (W5 / S5).
///
/// Holds ONLY archetype-local state: the raw target archetype pointer, the
/// reserved row, and the number of pools whose row at `new_row` has been committed
/// so far. On unwind it drops those committed components (each via its pool's
/// `drop_at`) and uncommits the row, leaving the target archetype exactly as it was
/// before the clone began. It NEVER touches `entity_master` and NEVER caches a
/// world pointer — the entity→inland mapping is committed by the caller only after
/// the guard is disarmed (so on the panic path the entity was never mapped).
struct CloneRowGuard {
    /// Raw, write-capable, interior-mutable slab provenance for the target
    /// archetype. Stable across the materialization (no archetype move occurs while
    /// the guard is live). `None` once disarmed.
    target_archetype_ptr: *mut Archetype,
    /// The reserved row index in the target archetype's pools.
    new_row: usize,
    /// The component ids already committed at `new_row`, in commit order. On
    /// rollback each is dropped + uncommitted. Bounded by `MAX_CLONE_COLUMNS`.
    committed: [ComponentId; MAX_CLONE_COLUMNS],
    /// How many entries of `committed` are live.
    committed_count: usize,
    /// `true` until the caller disarms after full success.
    armed: bool,
}

impl CloneRowGuard {
    #[inline]
    fn new(target_archetype_ptr: *mut Archetype, new_row: usize) -> Self {
        Self {
            target_archetype_ptr,
            new_row,
            committed: [ComponentId(0); MAX_CLONE_COLUMNS],
            committed_count: 0,
            armed: true,
        }
    }

    /// Records that `id`'s pool row at `new_row` is now committed (so a later panic
    /// rolls it back).
    #[inline]
    fn note_committed(&mut self, id: ComponentId) {
        debug_assert!(
            self.committed_count < MAX_CLONE_COLUMNS,
            "CloneRowGuard: committed overflow (> MAX_CLONE_COLUMNS)"
        );
        self.committed[self.committed_count] = id;
        self.committed_count += 1;
    }

    /// Disarms the guard after the row has been fully materialized + the archetype
    /// bookkeeping (`entity_ids` / `current_index`) advanced. After this, `Drop` is
    /// a no-op.
    #[inline]
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CloneRowGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // SAFETY (S5 / W5):
        //   * `target_archetype_ptr` is write-capable, stable, interior-mutable
        //     (`SharedReadWrite`) slab provenance — minted under `&mut EcsMaster`
        //     and never moved while this guard is live (no archetype-slab growth
        //     occurs during a single clone's materialization loop).
        //   * For each `committed[0..committed_count]` id, that pool committed a
        //     fully-initialized clone at `new_row` (`note_committed` is called only
        //     after `commit_units(new_row, 1)`), so `new_row < pool.count()` and
        //     `drop_at(new_row)` runs the type's `drop_fn` exactly once.
        //   * After dropping, `pop_entity_no_drop` uncommits the row (the slot is
        //     now logically uninit and never deref'd again). The pools not yet
        //     committed (the panicking pool + the tail) were never touched, so the
        //     row count is back to where it started.
        //   * Single-threaded `&mut EcsMaster` is held by the unwinding frame, so
        //     no concurrent reader/writer exists.
        //   * `entity_master` is NOT touched: the caller commits the entity→inland
        //     mapping only AFTER `disarm`, so on this panic path the entity was
        //     never mapped (no half-alive entity). No world pointer is cached here.
        let archetype: &mut Archetype = unsafe { &mut *self.target_archetype_ptr };
        for &id in &self.committed[..self.committed_count] {
            if let Some(pool) = archetype.component_pools_mut().get_pool_mut(id) {
                // SAFETY: `new_row` is the last committed row of this pool (every
                // committed pool committed exactly the same `new_row`), so
                // `new_row < pool.count()` and the slot holds a valid clone.
                unsafe { pool.drop_at(self.new_row) };
                pool.pop_entity_no_drop();
            }
        }
    }
}

/// Outcome of [`materialize_clone`]: the new entity. (The target archetype
/// pointer / row are intentionally NOT exposed: the deep-clone walk re-resolves the
/// source per node and remaps through the fast store — W6 "re-resolve, don't cache
/// across structural ops".)
pub(crate) struct CloneResult {
    pub(crate) entity: Entity,
}

/// Algorithm A — materializes a single clone of `source` into a freshly-allocated
/// entity, per `cloner`'s configuration. The `source` MUST be alive (caller-checked
/// via `has_entity`). Returns the new entity + its location.
///
/// This is the shared primitive for both the shallow direct API and each node of
/// the deep-clone walk. It does NOT fire the deep-clone `ChildOf` remap (that is
/// the deep walk's job) and does NOT drain deferred commands (the caller drains).
#[inline]
pub(crate) fn materialize_clone(
    world: &mut EcsMaster,
    source: Entity,
    cloner: &EntityCloner,
) -> CloneResult {
    materialize_clone_into(world, source, None, cloner)
}

/// Deferred-path variant: materializes the clone into the PRE-RESERVED `reserved`
/// entity (minted by `Commands::clone_and_spawn` via the atomic counter), instead
/// of allocating a fresh id. Used by `CloneSpawnCommand::apply`.
#[inline]
pub(crate) fn materialize_clone_at(
    world: &mut EcsMaster,
    source: Entity,
    reserved: Entity,
    cloner: &EntityCloner,
) -> CloneResult {
    materialize_clone_into(world, source, Some(reserved), cloner)
}

/// Shared body. `reserved` = `Some` for the deferred path (write into a pre-reserved
/// id; the slot is `ensure`d), `None` for the direct path (allocate a fresh id).
fn materialize_clone_into(
    world: &mut EcsMaster,
    source: Entity,
    reserved: Option<Entity>,
    cloner: &EntityCloner,
) -> CloneResult {
    let current_tick = world.current_tick();
    // `Children` is ALWAYS cloner-denied (a derived reverse index — a deep clone
    // rebuilds it via `LinkChildCommand`, never byte-copies it). Resolve its id once
    // to filter it out below.
    let children_id = crate::ecs::core::hierarchy::Children::component_id();

    // ── Resolve the source's location (generation-checked) ─────────────────
    let source_inland = world.entity_master.entities_inland[source.id().0];
    debug_assert!(
        !source_inland.is_null() && source_inland.generation() == source.generation(),
        "materialize_clone: source must be alive (caller checks has_entity)"
    );
    let source_ptr: *mut Archetype = source_inland.archetype_ptr();
    let source_row = source_inland.unit_index() as usize;
    // Capture the source archetype id NOW (before any structural op that could
    // grow / move the archetype slab), so the post-`get_or_create_archetype`
    // re-resolve uses a stable id, not a possibly-stale pointer.
    // SAFETY: `source_ptr` is live stable slab provenance (source is alive); reading
    //   the `id` field is one load through a shared deref.
    let source_archetype_id = unsafe { (*source_ptr).id() };

    // ── Step 2 + C2: build the FILTERED id set, then the require-CLOSURE ────
    // `filtered`: source ids that pass the filter, minus `Children` (always
    // cloner-denied — a derived reverse index, never directly cloned), minus (in
    // non-strict mode) ids with no clone fn (skipped). In strict mode an
    // `Ignore` source component panics.
    let mut target_ids: [ComponentId; MAX_CLONE_COLUMNS] =
        [ComponentId(0); MAX_CLONE_COLUMNS];
    let mut target_len = 0usize;
    // Mask of ids that come from the source AND will copy REAL bytes (vs the
    // reconstructed-required set). Used in the materialization loop to decide
    // copy-vs-construct, and (C2) to keep a present required component's real value.
    let mut copy_from_source = ComponentMask::new();

    {
        // SAFETY: `source_ptr` is stable slab provenance; the shared `&Archetype`
        // view is scoped to this block and dropped before the target reborrow.
        let source_arch: &Archetype = unsafe { &*source_ptr };
        for &id in source_arch.component_ids() {
            if id == children_id {
                continue; // always denied (D5)
            }
            // W1 / Dense plan C1 #11: a non-signature-storage id (`Bitset` OR
            // `Dense`) is RETAINED in `component_ids()` but has NO per-archetype
            // `ComponentPool` — it would panic at `get_pool(id).expect(...)`
            // below. Skip it so it never enters the table row clone
            // (`target_ids` / `copy_from_source`). A bitset tag's enable-state is
            // not carried through a clone (a v1.1 follow-up). For a dense id, ONLY
            // the row-clone EXCLUSION is D0 — materializing dense membership in
            // the clone target + firing its hooks is D2, deliberately NOT done
            // here.
            if !component_registry::is_signature_storage(component_registry::storage_kind(id.0)) {
                continue;
            }
            if !cloner.filter_allows(id) {
                continue; // denied by allow/deny filter
            }
            let info = component_registry::get_clone_info(id.0);
            let cloneable = matches!(
                info.map(|i| i.cloneability),
                Some(Cloneability::TriviallyCopyable) | Some(Cloneability::CloneViaFn)
            );
            if !cloneable {
                if cloner.strict {
                    strict_ignore_panic(id);
                }
                // opt-out / non-strict: skip + diagnose (the "missing component"
                // surprise is debug-loggable, not silent).
                debug_assert!(
                    false,
                    "clone: skipping non-cloneable component {} (Cloneability::Ignore) \
                     in non-strict mode — the clone lands in a smaller archetype",
                    id.0
                );
                continue;
            }
            debug_assert!(target_len < MAX_CLONE_COLUMNS, "clone id set overflow");
            target_ids[target_len] = id;
            target_len += 1;
            copy_from_source.set(id);
        }
    }

    // C2 require-CLOSURE: union the transitive `#[require]` closure of the
    // CLONED-component set into the target set. A required id ALREADY present
    // (copy_from_source) keeps its real bytes (Decision 7 preserved — no
    // re-default). A required id ABSENT (source lacked it, or the filter denied it
    // but a cloned component requires it) is added here and will be RECONSTRUCTED
    // via its ctor below — preserving the require-invariant an on_add observer may
    // rely on. A filter-DENIED required is overridden (re-added) + debug-logged
    // (Bevy 0.17 "allowed components also allow their required").
    let cloned_set_snapshot: [ComponentId; MAX_CLONE_COLUMNS] = target_ids;
    let cloned_set = &cloned_set_snapshot[..target_len];
    component_registry::for_each_required_id_excluding(cloned_set, |req_id| {
        if target_ids[..target_len].contains(&req_id) {
            return; // already in the target set (present⇒skip)
        }
        // A required id absent from the target set. If the FILTER denied it, the
        // require-closure OVERRIDES the deny (the invariant wins): a cloned
        // component's required dependency is always present, so a denied required is
        // reconstructed anyway (Bevy 0.17 "allowed components also allow their
        // required"). Emit a debug-only diagnostic so the override is diagnosable.
        #[cfg(debug_assertions)]
        if !cloner.filter_allows(req_id) {
            // A denied required is being reconstructed to preserve the
            // require-invariant (Feature 1 cross-feature C2). Not an error — a
            // documented override; surfaced here for diagnosability.
            debug_clone_required_override(req_id);
        }
        debug_assert!(target_len < MAX_CLONE_COLUMNS, "clone require-closure overflow");
        target_ids[target_len] = req_id;
        target_len += 1;
        // NOT added to `copy_from_source`: this id is reconstructed, not copied.
    });

    // Canonical-sort so `get_or_create_archetype` collapses equivalent sets to the
    // same `ArchetypeId` regardless of insertion order.
    target_ids[..target_len].sort_unstable_by_key(|c| c.0);
    let target_archetype_id = world.get_or_create_archetype(&target_ids[..target_len]);

    // ── Step 4: obtain the new entity id (NOT yet mapped — W5) ──────────────
    // Direct path: allocate a fresh id. Deferred path: use the pre-reserved id
    // (minted at the `Commands::clone_and_spawn` callsite) and grow the fast store
    // to cover its slot (mirrors `SpawnAtCommand::apply` Step 3). In BOTH cases the
    // entity→inland MAPPING is committed only as the LAST step (W5), so a panic
    // mid-materialization leaves `entity_master` untouched.
    let entity = match reserved {
        Some(reserved_entity) => {
            world
                .entity_master
                .entities_inland
                .ensure(reserved_entity.id().0 + 1);
            reserved_entity
        }
        None => world.entity_master.allocate_entity(),
    };

    // ── Resolve the (possibly grown) source + target archetype pointers ─────
    // `get_or_create_archetype` may have grown the archetype slab, invalidating any
    // earlier `source_ptr`; re-resolve both from the archetype master.
    let source_ptr = world
        .archetype_master_mut()
        .archetype_ptr_for(source_archetype_id)
        .expect("invariant: source archetype is live");
    let target_ptr = world
        .archetype_master_mut()
        .archetype_ptr_for(target_archetype_id)
        .expect("invariant: target archetype just resolved");

    // ── Step 5 + 6: reserve a row, materialize each component single-pass ───
    // The CloneRowGuard arms now (touches ONLY the archetype row). On a panic in a
    // user `Clone::clone` it rolls back the committed components.
    //
    // ALIASING DISCIPLINE (the F2 / W5 soundness anchor): NO `&mut Archetype` is
    // held across a `clone_fn` call (or across any guard-Drop point). Each pool
    // operation derives a `&mut Archetype` in a TIGHT scope and drops it before the
    // next step; the panic-prone `clone_fn(src_ptr, dst_ptr)` runs with ONLY raw
    // pointers live (`*mut u8`, no protector under Tree Borrows). So on unwind the
    // guard's `Drop` (`&mut *target_ptr`) is the SOLE `&mut Archetype` accessor — no
    // aliasing `&mut` from this frame is live (mirrors `migrate_entity_insert`'s
    // confined-reborrow discipline).
    let new_row: usize;
    {
        // SAFETY: `target_ptr` is write-capable, interior-mutable slab provenance
        //   under `&mut EcsMaster`; this reborrow is confined to the reserve call.
        new_row = unsafe {
            let target: &mut Archetype = &mut *target_ptr;
            target.reserve_capacity(1).expect(
                "clone: target pool reserve ceiling (rows) exhausted — committed \
                 capacity grows on demand (Phase X.I)",
            );
            target.current_index
        };

        let mut guard = CloneRowGuard::new(target_ptr, new_row);

        let target_cids: [ComponentId; MAX_CLONE_COLUMNS] = target_ids;
        let target_cid_count = target_len;
        for &id in &target_cids[..target_cid_count] {
            if copy_from_source.contains(id) {
                // Copy REAL source bytes (Decision 7: a present cloned/required
                // component keeps its real, possibly-mutated value — no re-default).
                let info = component_registry::get_clone_info(id.0).expect(
                    "invariant: a copy_from_source id was classified cloneable above",
                );
                // Read the source row once (the source pool is only READ, through a
                // shared deref — disjoint from the target's `&mut` pools when
                // archetypes differ; for a same-archetype clone `source_row` and the
                // fresh `new_row` are distinct rows, so read/write never overlap).
                // O2: the source change-detection ticks are read ONLY when
                // `preserve_ticks` is set (the non-default). On the default reset path
                // they are replaced by `current_tick` downstream, so the two `Tick`
                // loads per cloned component are skipped on the hot clone path.
                // SAFETY: `source_row < src_pool.count()` (source is live);
                //   `unit_ptr` yields a live, aligned, initialized row pointer;
                //   `read_*_tick` reads the committed source slot.
                let (src_ptr, stride, src_added, src_changed) = unsafe {
                    let src_arch: &Archetype = &*source_ptr;
                    let src_pool = src_arch
                        .component_pools()
                        .get_pool(id)
                        .expect("invariant: copy_from_source id exists in source");
                    debug_assert!(
                        source_row < src_pool.count(),
                        "clone: source_row out of bounds"
                    );
                    let stride = src_pool.component_layout().size();
                    let (src_added, src_changed) = if cloner.preserve_ticks {
                        (
                            src_pool.read_added_tick(source_row),
                            src_pool.read_changed_tick(source_row),
                        )
                    } else {
                        (current_tick, current_tick)
                    };
                    (src_pool.unit_ptr(source_row), stride, src_added, src_changed)
                };

                match info.cloneability {
                    Cloneability::TriviallyCopyable => {
                        // O2 batch-by-column: a plain byte copy driven by the pool
                        // layout (the fn-ptr is `None`). The `&mut Archetype` reborrow
                        // is confined to this scope — `write_at` cannot panic, so no
                        // guard-Drop point spans it.
                        // SAFETY (S1 / W7):
                        //   * `src_ptr` is a live, aligned, initialized row readable
                        //     for `stride` bytes; `new_row < committed_rows` post
                        //     `reserve_capacity(1)`; `write_at_unchecked_initialized`
                        //     memcpys into the reserved-uninit slot without dropping.
                        //   * `src` and `dst` are disjoint; the byte copy reaches no
                        //     world state. `&mut target` confined to this block.
                        let bytes = unsafe { core::slice::from_raw_parts(src_ptr, stride) };
                        unsafe {
                            let target: &mut Archetype = &mut *target_ptr;
                            let dst_pool = target
                                .component_pools_mut()
                                .get_pool_mut(id)
                                .expect("invariant: copy_from_source id exists in target");
                            dst_pool.write_at_unchecked_initialized(new_row, bytes);
                        }
                    }
                    Cloneability::CloneViaFn => {
                        let clone_fn = info.clone_fn.expect(
                            "invariant: CloneViaFn installs Some(clone_via_clone::<C>)",
                        );
                        // Derive the dst `*mut u8` in a TIGHT scope, then drop the
                        // `&mut Archetype` BEFORE calling the (panic-prone) user
                        // `clone_fn`. At the `clone_fn` call NO `&mut Archetype` is
                        // live — only `dst_ptr: *mut u8` (no TB protector across the
                        // call) — so the guard's Drop is the sole `&mut Archetype`
                        // accessor on unwind (the F2 / W5 anchor).
                        // SAFETY: `new_row < committed_rows`; the slot is reserved-
                        //   uninit; the returned `*mut u8` carries the pool's
                        //   reservation provenance, aligned for this component's type.
                        let dst_ptr = unsafe {
                            let target: &mut Archetype = &mut *target_ptr;
                            let dst_pool = target
                                .component_pools_mut()
                                .get_pool_mut(id)
                                .expect("invariant: copy_from_source id exists in target");
                            clone_row_ptr(dst_pool, new_row, stride)
                        };
                        // SAFETY (S1/S2/S3/W7):
                        //   * `src_ptr` is a live, aligned, initialized `C`; `dst_ptr`
                        //     is the reserved-uninit `new_row` slot aligned for the
                        //     same `C`; they are disjoint (distinct pools, or distinct
                        //     rows of one pool).
                        //   * `clone_fn` (= `clone_via_clone::<C>`) forms `&C`, calls
                        //     `Clone::clone`, and `ptr::write`s the result without
                        //     dropping the uninit dst. It receives ONLY raw pointers —
                        //     user `Clone` code cannot reach world state (W7). NO
                        //     `&mut Archetype` is live across this call (dropped above).
                        //   * On a panic INSIDE `clone_fn`, `dst` is left uninit and
                        //     this iteration's `commit` below does NOT run, so the
                        //     guard does NOT record this id (no double-drop).
                        unsafe { clone_fn(src_ptr, dst_ptr) };
                    }
                    Cloneability::Ignore => unreachable!(
                        "copy_from_source only holds cloneable ids"
                    ),
                }
                // Commit the row + stamp ticks (reset to current, or preserve). A
                // fresh, confined `&mut Archetype` reborrow.
                let (added, changed) = if cloner.preserve_ticks {
                    (src_added, src_changed)
                } else {
                    (current_tick, current_tick)
                };
                // SAFETY: `new_row < committed_rows`; `commit_units` makes the slot
                //   live; `write_*_tick` then stamps the committed slot. `&mut target`
                //   confined to this block; `commit`/`write_tick` cannot panic.
                unsafe {
                    let target: &mut Archetype = &mut *target_ptr;
                    let dst_pool = target
                        .component_pools_mut()
                        .get_pool_mut(id)
                        .expect("invariant: copy_from_source id exists in target");
                    dst_pool.commit_units(new_row, 1);
                    dst_pool.write_added_tick(new_row, added);
                    dst_pool.write_changed_tick(new_row, changed);
                }
                guard.note_committed(id);
            } else {
                // C2 RECONSTRUCT: a required id absent from the source (or
                // filter-denied) — construct one value via its capture-free ctor.
                let ctor = component_registry::required_ctor_for(cloned_set, id).expect(
                    "invariant: a reconstructed id came from the cloned set's required \
                     closure, so a ctor exists for it",
                );
                // A capture-free ctor (Feature 1's derive-generated free fn) does not
                // panic by design; still, the `&mut Archetype` reborrow is confined.
                // SAFETY (mirrors SpawnAtCommand's required ctor pass):
                //   * `new_row < committed_rows`; the slot is reserved-uninit. `ctor`
                //     writes one value of this pool's registered type (registry-paired)
                //     without dropping the uninit dst. `&mut target` confined here.
                unsafe {
                    let target: &mut Archetype = &mut *target_ptr;
                    let dst_pool = target
                        .component_pools_mut()
                        .get_pool_mut(id)
                        .expect("invariant: reconstructed id exists in target archetype");
                    dst_pool.construct_at_uninitialized(new_row, ctor);
                    dst_pool.commit_units(new_row, 1);
                    dst_pool.fill_ticks(new_row, 1, current_tick);
                }
                guard.note_committed(id);
            }
        }

        // All components materialized: advance the archetype bookkeeping so the row
        // is a real entity row, THEN disarm (the row is now consistent). A confined
        // `&mut Archetype` reborrow; neither op can panic.
        // SAFETY: `target_ptr` is write-capable slab provenance; `&mut` confined.
        unsafe {
            let target: &mut Archetype = &mut *target_ptr;
            target.entity_ids.push(entity.id());
            target.current_index = new_row + 1;
        }
        guard.disarm();
        // <-- the guard (now disarmed) drops harmlessly at the block close.
    }

    // ── Step (W5 LAST): commit the entity→inland mapping ───────────────────
    // Only NOW — after full success — is the entity mapped. On any panic above the
    // guard rolled back the row and the entity was never registered, so
    // `entity_master` is untouched on the unwinding path (W5 / S5).
    world
        .entity_master
        .register_entity_with_ptr(entity, target_ptr, new_row as u32);

    // ── Dense plan D2 — materialize the source's DENSE memberships ──────────
    // The TABLE row clone above EXCLUDED dense ids (D0, materialize.rs ~:235).
    // Here we copy each dense membership of `source` that passes the cloner
    // filter into the clone's `DenseStore` (no archetype migration) and, when
    // `cloner.fire_hooks`, fire dense on_add/on_insert for it. 0%-gated: a
    // table-only world (`dense_registry.is_empty()`) skips the whole walk.
    if !world.dense_registry.is_empty() {
        materialize_dense_memberships(
            world,
            source,
            entity,
            target_archetype_id,
            cloner,
        );
    }

    // ── Step 7: gated fire (S6 / F2) ───────────────────────────────────────
    // The entity is fully materialized + mapped; no `&mut Archetype` reborrow is
    // live. Mint `world_ptr` only here.
    if cloner.fire_hooks {
        // Feature 2: raise the sticky entity-observer bit on the destination before
        // reading flags (mirrors `migrate_entity_insert`; no-op without an observer).
        world.migrate_entity_observer_bit(entity);
        // SAFETY (F2): `target_ptr` is write-capable, stable slab provenance that
        //   survived the Phase-5 push; reading `flags` is one `u16` load. No
        //   `world`-derived `&mut Archetype` is live.
        let flags = unsafe { (*target_ptr).flags };
        if !flags.is_empty() {
            // MINT: no `world`-derived `&mut Archetype` is live (S6).
            let world_ptr = NonNull::from(&mut *world);
            // The clone gained EVERY target component, so on_add fires over the
            // full target id set (same shape as the spawn fire in SpawnAtCommand).
            // SAFETY: `target_ptr` is a valid `*const Archetype`; the shared id
            //   slice is transient and not aliased by any live `&mut` (hooks receive
            //   `world_ptr`, not the slice).
            // Dense plan D2: the clone's archetype RETAINS non-signature ids in
            // `component_ids` (it dedups to the source's archetype, which kept the
            // dense id since D0), so these table-fire loops SKIP dense via
            // `is_signature_id` — dense is fired by `materialize_dense_memberships`,
            // never here (no double-fire). For a dense-free clone the skip is never
            // taken (the 0%-gate).
            if flags.contains(ArchetypeFlags::ON_ADD_ANY) {
                let ids = unsafe { (*target_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
                    for &cid in ids {
                        if !component_registry::is_signature_id(cid) {
                            continue;
                        }
                        trigger_on_add(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_ADD_OBSERVER) {
                    for &cid in ids {
                        if !component_registry::is_signature_id(cid) {
                            continue;
                        }
                        fire_on_add_observers(world_ptr, cid, entity);
                    }
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
                // SAFETY: same as the on_add slice read above.
                let ids = unsafe { (*target_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                    for &cid in ids {
                        if !component_registry::is_signature_id(cid) {
                            continue;
                        }
                        trigger_on_insert(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                    for &cid in ids {
                        if !component_registry::is_signature_id(cid) {
                            continue;
                        }
                        fire_on_insert_observers(world_ptr, cid, entity);
                    }
                }
            }
            if flags.contains(ArchetypeFlags::HAS_ENTITY_OBSERVER) {
                // SAFETY: same as the on_add slice read above.
                let ids = unsafe { (*target_ptr).component_ids.as_slice() };
                for &cid in ids {
                    if !component_registry::is_signature_id(cid) {
                        continue;
                    }
                    fire_entity_observers(world_ptr, ObserverKind::Add, cid, entity);
                }
                for &cid in ids {
                    if !component_registry::is_signature_id(cid) {
                        continue;
                    }
                    fire_entity_observers(world_ptr, ObserverKind::Insert, cid, entity);
                }
            }
        }
    }

    CloneResult { entity }
}

/// Computes the `*mut u8` row pointer for `new_row` in `pool` (the
/// reserved-but-uncommitted slot), via the same stride math the pool uses
/// internally. Used for the `CloneViaFn` dst (which needs a `*mut u8`, not a
/// `&[u8]`). `stride` is the pool's component size (already read from the source).
///
/// # Safety
/// `new_row < pool.committed_rows` (caller pre-reserved via
/// `Archetype::reserve_capacity`); the slot is reserved-uninit and exclusively
/// owned via `&mut pool`.
#[inline]
unsafe fn clone_row_ptr(
    pool: &mut crate::ecs::memory::component_pool::ComponentPool,
    new_row: usize,
    stride: usize,
) -> *mut u8 {
    debug_assert_eq!(
        stride,
        pool.component_layout().size(),
        "clone_row_ptr: stride mismatch (source vs target pool layout)"
    );
    // Reuse the pool's own reserved-slot pointer. `write_at_unchecked_initialized`
    // would memcpy, but `CloneViaFn` must write THROUGH the pointer, so we expose
    // the raw dst via the pool's reserved-row accessor.
    // SAFETY: forwarded to `reserved_row_ptr`; the caller upholds its contract
    //   (`new_row < committed_rows`, reserved-uninit, exclusive `&mut pool`).
    unsafe { pool.reserved_row_ptr(new_row) }
}

/// Cold debug-only diagnostic: a filter-denied required component is being
/// reconstructed to preserve the require-invariant (C2 override). Not an error — a
/// documented override, surfaced here for diagnosability (the tester can breakpoint
/// or extend this to a log). The function exists so the override is greppable and
/// the hot/cold split keeps the reason out of the materialization body.
#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn debug_clone_required_override(_id: ComponentId) {
    // Intentionally a no-op marker (no panic, no log dependency). A filter-denied
    // required component is reconstructed by design (C2); breakpoint here to observe.
}

/// Dense plan D2 — materializes `source`'s dense memberships into the clone
/// `entity` and (when `cloner.fire_hooks`) fires dense on_add/on_insert.
///
/// The TABLE row clone excludes dense ids (D0); this is the dense companion. For
/// each dense store the SOURCE belongs to and the cloner filter allows, copies the
/// source value's bytes into the clone's store (a fresh slot, no migration). A
/// non-cloneable (`Cloneability::Ignore`) dense component is skipped (panics in
/// `strict` mode, mirroring the table path). The source bytes are copied into an
/// owned buffer BEFORE the clone insert, because both touch the SAME store
/// (`&store` to read + `&mut store` to insert cannot coexist).
///
/// Cold (clone is not a per-frame path). Caller gates on
/// `!world.dense_registry.is_empty()` (the 0%-gate).
#[cold]
#[inline(never)]
fn materialize_dense_memberships(
    world: &mut EcsMaster,
    source: Entity,
    entity: Entity,
    target_archetype_id: crate::ecs::identifiers::primitives::ArchetypeId,
    cloner: &EntityCloner,
) {
    // Snapshot the dense ids the SOURCE belongs to + their bytes into owned
    // buffers, so the subsequent `&mut store` inserts don't alias the `&store`
    // reads. `dense_ids` is small; only memberships are copied.
    let source_id = source.id();
    let mut cloned: Vec<(ComponentId, Vec<u8>)> = Vec::new();
    for &cid in world.dense_registry.dense_ids() {
        // Filter / cloneability gate (mirrors the table path).
        if !cloner.filter_allows(cid) {
            continue;
        }
        let info = component_registry::get_clone_info(cid.0);
        let cloneable = matches!(
            info.map(|i| i.cloneability),
            Some(Cloneability::TriviallyCopyable) | Some(Cloneability::CloneViaFn)
        );
        let Some(store) = world.dense_registry.store(cid) else {
            continue;
        };
        let Some(slot) = store.slot_of(source_id) else {
            continue; // source is not a member of this dense store
        };
        if !cloneable {
            if cloner.strict {
                strict_ignore_panic(cid);
            }
            // Non-strict: skip a non-cloneable dense membership (the clone simply
            // lacks it), mirroring the table clone's non-cloneable skip.
            continue;
        }
        let stride = store.stride();
        let view = store.solve_view();
        // SAFETY: `slot` came from `slot_of(source_id)`, so it is a LIVE slot
        //   (`< len`, live-bit set), satisfying `row_ptr`'s contract. The pointer
        //   is valid for `stride` bytes of the source value.
        let src_bytes: &[u8] = unsafe {
            let ptr = view.row_ptr(slot as usize);
            core::slice::from_raw_parts(ptr, stride)
        };
        // For a `TriviallyCopyable` dense component a byte copy reproduces the
        // value exactly. `CloneViaFn` dense components fall back to the byte copy
        // here too (D2 scope: the physics-body dense use case is POD; a deep
        // owning dense `Clone` is a documented v1.1 follow-up — the value is still
        // bit-copied, sound for `Copy`-with-`Entity` shapes that own no heap).
        cloned.push((cid, src_bytes.to_vec()));
    }

    if cloned.is_empty() {
        return;
    }

    // Insert each snapshotted value into the clone's store (no archetype change).
    // Dense plan D4: a cloned dense membership is Added on the clone this frame —
    // stamp both ticks at the current world tick (mirrors the table clone, whose
    // `fill_ticks` stamps the cloned rows).
    let current_tick = world.current_tick();
    for (cid, bytes) in &cloned {
        let store = world.dense_registry.store_mut(*cid);
        store.insert(entity.id(), bytes, current_tick);
        store.mark_arch_present(target_archetype_id);
    }

    // Fire dense on_add/on_insert for the materialized memberships (gated by
    // `cloner.fire_hooks`, mirroring the table clone fire). NOT gated by archetype
    // flags (dense ids are not in the signature); the `trigger`/`fire` self-gate.
    if cloner.fire_hooks {
        // MINT: no `world`-derived `&mut` into storage is live (the store inserts
        // above returned; only the owned `cloned` buffer survives).
        let world_ptr = NonNull::from(&mut *world);
        for (cid, _) in &cloned {
            trigger_on_add(world_ptr, *cid, entity);
            fire_on_add_observers(world_ptr, *cid, entity);
        }
        for (cid, _) in &cloned {
            trigger_on_insert(world_ptr, *cid, entity);
            fire_on_insert_observers(world_ptr, *cid, entity);
        }
    }
}

/// Cold fail-loud panic for `strict(true)` over an `Ignore` source component.
#[cold]
#[inline(never)]
fn strict_ignore_panic(id: ComponentId) -> ! {
    let name = component_registry::get_layout(id.0)
        .map(|l| l.type_name)
        .unwrap_or("<unregistered>");
    panic!(
        "EntityCloner::strict: source component {} ({}) is not cloneable \
         (Cloneability::Ignore — not Clone / #[component(no_clone)]). Either make it \
         cloneable, deny it from the clone, or drop strict mode.",
        id.0, name,
    )
}
