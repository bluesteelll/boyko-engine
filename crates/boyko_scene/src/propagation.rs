//! World-from-local transform propagation (S2).
//!
//! [`propagate_transforms`] composes every entity's [`GlobalTransform`] from its
//! [`Transform`] chain along the `ChildOf` / `Children` hierarchy, once per
//! frame, **alloc-free** and **dirty-gated**: a static subtree costs only a
//! per-row change-tick read, no affine compose.
//!
//! # Why an exclusive system
//!
//! The descent reads a parent's already-computed `GlobalTransform` and writes a
//! child's `GlobalTransform` — the **same component column, different rows**.
//! Expressing that requires by-entity raw access into the `GlobalTransform`
//! pool, which the kernel exposes only through `&mut EcsMaster`
//! ([`EcsMaster::get_component_raw`] / [`EcsMaster::get_component_raw_mut`]).
//! A non-exclusive `SystemParam` system cannot hold a parent `&GlobalTransform`
//! and a child `&mut GlobalTransform` from one `Query` at once, so
//! [`propagate_transforms`] is an **exclusive** `fn(&mut EcsMaster)`. The
//! schedule registers it per-frame, after the fixed (physics) schedule has fully
//! advanced and before the camera / light / GPU-upload readers (S2 schedule
//! table).
//!
//! # Alloc-free + dirty-gated
//!
//! All per-frame scratch (the descent worklist and the dirty-seed buffer) lives
//! in the [`TransformPropagationScratch`] resource and is `clear()`ed + refilled
//! each run — its capacity is reused, so steady state allocates nothing
//! (Principle 1/5). Dirtiness is read from the kernel's existing per-row change
//! ticks: a node is recomposed only when its own `Transform` **or** its `ChildOf`
//! link changed since the last run (the latter catches a re-parent / first-attach
//! that never touches the local `Transform`), or an ancestor was recomposed this
//! run. A fully static scene pays one linear change-tick read over the spatial
//! entities and **zero** affine composes (the `STILL_FRAME_COMPOSES == 0` debug
//! counter asserts this).
//!
//! # Cost model (honest)
//!
//! Per frame the dirty scan is an **O(entities-in-matched-archetypes)** cheap
//! per-row tick test — NOT the O(archetypes)+O(changed) streaming column scan the
//! plan's headline aspired to. Reaching the streaming form requires a public
//! per-archetype changed-tick-column accessor on the kernel (`read_changed_tick`
//! is `pub(crate)`), which is out of scope for this crate; S2 ships the cheap
//! per-row test (one inland+pool lookup + a tick compare per spatial entity, no
//! affine compose on a static row) and the descent that only visits dirty
//! subtrees. The `STILL_FRAME_COMPOSES` counter measures composes only; a real
//! bytes-touched / wall-time-vs-entity-count bench is the standing 0%-gate work
//! item for the streaming upgrade (RESULTS).

use std::sync::OnceLock;

use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverFn};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::identifiers::primitives::{ComponentId, InlandPoolId};
use boyko_macros::Resource;
use boyko_math::Affine3A;

use crate::transform::{GlobalTransform, Transform};

#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicU64, Ordering};

/// Hard cap on ancestor-chain / descent steps before the loop bails out.
///
/// `ChildOf` cycles are a documented kernel footgun (only the one-compare
/// self-reference is guarded; deep A→B→…→A cycles are **not** detected —
/// `hierarchy/mod.rs`). A malformed deep cycle would make [`root_ancestor`]'s
/// ascent and the descent re-enter unboundedly (hang / OOM). These bounds turn
/// that into a loud `debug_assert!` failure in debug and a quiet bail-out (the
/// subtree is left with its prior pose) in release, instead of a hang. The cap
/// is far above any plausible real tree depth / node count, so a cycle-free
/// scene never reaches it.
const MAX_ANCESTOR_DEPTH: usize = 1 << 20;

/// Hard cap on the number of nodes the descent will compose in one run, as the
/// release-mode termination guard for a malformed `ChildOf` cycle (see
/// [`MAX_ANCESTOR_DEPTH`]). A cycle-free tree visits each dirty subtree node a
/// bounded number of times and never approaches this.
const MAX_DESCENT_STEPS: usize = 1 << 24;

/// Hard cap on the descent's PER-PATH depth (a single ancestor→descendant chain
/// length), the structural twin of `boyko_ui`'s `MAX_LAYOUT_DEPTH`.
///
/// `MAX_DESCENT_STEPS` bounds the TOTAL node visits across the whole frontier; a
/// deep `ChildOf` cycle (A→B→A) that the kernel does NOT detect (only the
/// one-compare self-reference is guarded — `hierarchy/mod.rs`) would otherwise
/// keep re-pushing the same two rows onto the frontier and walk the chain
/// without bound until `MAX_DESCENT_STEPS` trips. This per-path depth bound
/// terminates such a chain *earlier and locally* (the child carries its
/// chain-depth on the frontier; once it exceeds this cap the subtree is dropped
/// rather than re-descended), so a pathological cycle is bounded by chain length,
/// not by the global visit budget. The cap is far above any plausible real tree
/// depth, so a well-formed scene never reaches it.
const MAX_TRANSFORM_DEPTH: u32 = 1 << 16;

/// Debug-only counter of affine composes performed by the most recent
/// [`propagate_transforms`] run. A still-frame 0%-gate test asserts this is `0`
/// when nothing changed; it compiles to nothing in release.
#[cfg(debug_assertions)]
pub static STILL_FRAME_COMPOSES: AtomicU64 = AtomicU64::new(0);

/// Reused per-frame scratch for [`propagate_transforms`] (Principle 0: the only
/// transient storage, and it is kernel `Resource`-owned, never a side `Vec`).
///
/// Holds the descent worklist, the dirty-seed buffer for the spatial-entity
/// scan, the detach-queue (F1), and the propagation system's own `last_run`
/// change-tick baseline (an exclusive system has no `SystemMeta::last_run` in
/// its body, so the baseline lives here and is advanced at the end of every
/// run). All buffers are `clear()`ed + refilled per run — capacity is reused,
/// no per-frame heap alloc after warmup.
#[derive(Resource)]
pub struct TransformPropagationScratch {
    /// LIFO descent frontier of `(entity, depth)` pairs whose children still
    /// need composing. Holds `Entity` (what `Children` stores) plus the node's
    /// per-path chain depth, the [`MAX_TRANSFORM_DEPTH`] cycle guard. Reused
    /// across frames.
    stack: Vec<(Entity, u32)>,
    /// Dirty-seed buffer: the spatial entities whose own `Transform`/`ChildOf`
    /// changed this run, filled by the per-archetype dirty scan and consumed by
    /// the seeding pass. Reused across frames.
    dirty: Vec<Entity>,
    /// F1 detach queue: entities whose `ChildOf` was fully REMOVED (they are now
    /// roots) since the last propagate run. A removed `ChildOf` carries no
    /// per-row tick the dirty scan can read, so the [`child_of_on_remove`]
    /// observer appends the orphaned entity here; [`propagate_transforms`] drains
    /// it at the start of each run and recomposes each as a root. Appended ONLY
    /// on the (rare) detach event, so a frame with no detach pays nothing; the
    /// `Vec`'s capacity is reused (drained, never freed), so a detach-heavy frame
    /// allocates only on the first growth.
    detached: Vec<Entity>,
    /// The change-tick horizon of the previous run. A node is "dirty" when its
    /// `Transform`'s (or `ChildOf`'s) `changed_tick` is newer than this.
    /// `Tick::ZERO` on the first run makes every node dirty (full initial
    /// compose).
    last_run: Tick,
    /// World-local "the F1 `ChildOf` `on_remove` observer is installed" flag.
    /// Lives here (rather than a process-global `static`) so each world installs
    /// its OWN observer exactly once (observers are per-world). Set by
    /// [`ensure_detach_observer`]; never cleared.
    detach_observer_installed: OnceLock<()>,
}

impl Default for TransformPropagationScratch {
    #[inline]
    fn default() -> Self {
        Self {
            stack: Vec::new(),
            dirty: Vec::new(),
            detached: Vec::new(),
            last_run: Tick::ZERO,
            detach_observer_installed: OnceLock::new(),
        }
    }
}

/// Recomputes every entity's [`GlobalTransform`] from its [`Transform`] chain
/// (S2). Exclusive, per-frame, alloc-free, dirty-gated.
///
/// # Algorithm
///
/// 1. **Dirty scan.** Walk the archetypes hosting both spatial columns
///    (`Transform` + `GlobalTransform`) once, reusing the resource scratch. A
///    node is *dirty* when its `Transform`'s **or** its `ChildOf`'s per-row
///    `changed_tick` is newer than the previous run's horizon — the `ChildOf`
///    leg catches a re-parent / first-attach that left the local `Transform`
///    untouched. The dirty entities are collected into the seed buffer. The F1
///    **detach queue** (entities whose `ChildOf` was fully REMOVED, marked by the
///    `child_of_on_remove` observer because a removed link carries no tick) is
///    folded into the seed here too — those orphans are now roots that must
///    collapse to their own local pose.
/// 2. **Seeding.** For each dirty node: a *root* (no `ChildOf`) recomposes
///    `GlobalTransform = local.to_affine()` in place and is pushed onto the
///    descent frontier; a dirty non-root seeds the descent from its ROOT
///    ancestor (whose `GlobalTransform` is already finalized — recomposed this
///    run if the root is itself dirty, otherwise unchanged-and-correct from the
///    prior frame), so the descent re-walks the whole path down to (and through)
///    the dirty node.
/// 3. **Dirty-subtree descent.** Pop a `(node, depth)` off the frontier and, for
///    each of its `Children`, compose `child.global = node.global ∘ child.local`
///    at `depth + 1`, then push the child so its own subtree follows. A subtree
///    with no dirty node is never visited (its root/ancestor was not seeded in
///    step 2). The per-node `depth` bounds a malformed `ChildOf` cycle
///    ([`MAX_TRANSFORM_DEPTH`]).
///
/// # Change detection (F2)
///
/// Every `GlobalTransform` write goes through [`set_global_if_changed`]
/// ([`Mut::set_if_neq`]), which bumps the row's `changed_tick` ONLY on a real
/// change — so downstream `Changed<GlobalTransform>` consumers (camera, GPU
/// upload, lights) observe a propagated move, while an unchanged recompose stays
/// tick-silent (0%-overhead preserved).
///
/// # Determinism
///
/// A child's `GlobalTransform` depends ONLY on its own `Transform` and its
/// parent's (already-finalized) `GlobalTransform` — never on a sibling. So the
/// final per-entity values are invariant to the (unspecified) `Children` sibling
/// order; the gates compare values, never traversal order.
///
/// # Schedule placement
///
/// Register per-frame, after the fixed schedule has advanced and before the
/// camera / light / GPU readers (S2 table). It is the sole `GlobalTransform`
/// writer per frame.
pub fn propagate_transforms(world: &mut EcsMaster) {
    #[cfg(debug_assertions)]
    STILL_FRAME_COMPOSES.store(0, Ordering::Relaxed);

    // F1: ensure the detach observer on `ChildOf` is installed exactly once for
    // this world (idempotent; see `ensure_detach_observer`). Installing it here —
    // rather than only in `TransformPlugin` — keeps detach re-propagation working
    // when the system is driven directly (`run_system` / a hand-built schedule),
    // which the gates do.
    ensure_detach_observer(world);

    let transform_id = Transform::component_id();
    let global_id = GlobalTransform::component_id();
    let child_of_id = ChildOf::component_id();

    // The frame's observation horizon. Per-row `changed_tick`s are compared
    // against `(last_run, this_run]`; `this_run` is the world's current tick.
    let this_run = world.current_tick();

    // Pull the reused buffers out by value to drop the resource borrow for the
    // raw-access descent (the descent calls the `&mut self` raw accessors on
    // `world`, which cannot coexist with a live `&mut scratch` reborrow). The
    // buffers are put back at the end so their capacity survives to next frame.
    // `detached` is drained (not just cleared) into the dirty seed: those entries
    // are the F1 detach events queued by the observer since the last run.
    let (mut stack, mut dirty, mut detached, last_run) = {
        let scratch = scratch_mut(world);
        (
            std::mem::take(&mut scratch.stack),
            std::mem::take(&mut scratch.dirty),
            std::mem::take(&mut scratch.detached),
            scratch.last_run,
        )
    };
    stack.clear();
    dirty.clear();

    // ── Step 1: dirty scan ──────────────────────────────────────────────────
    // Walk the archetypes hosting both spatial columns and collect the entities
    // whose `Transform` OR `ChildOf` changed since `last_run`. Cost: O(entities
    // in matched archetypes) cheap per-row tick tests — an unchanged node never
    // touches an affine. (See the module-level "Cost model" note: the streaming
    // O(archetypes) form is gated on a kernel accessor that is out of scope.)
    collect_dirty(world, transform_id, global_id, child_of_id, last_run, this_run, &mut dirty);

    // F1: fold the detach queue into the dirty seed. A detached entity lost its
    // `ChildOf` (it is now a root), but the removal left no per-row tick the
    // dirty scan could read, so the observer queued it here. Append it to the
    // dirty set so the seeding pass below recomposes it as a root. (An entity
    // that was detached and then despawned, or re-parented again before this run,
    // is filtered by the liveness / `has_component` checks in the seeding pass.)
    // `append` moves every detached entry into `dirty` and leaves `detached`
    // EMPTY with its capacity retained (reused next frame — no per-frame free).
    dirty.append(&mut detached);

    // ── Step 2: seeding ─────────────────────────────────────────────────────
    for &entity in dirty.iter() {
        let is_root = !world.has_component(entity, child_of_id);
        if is_root {
            // Recompose the root in place: GlobalTransform = local.to_affine().
            let local = match world.get_component::<Transform>(entity) {
                Some(t) => *t,
                None => continue,
            };
            set_global_if_changed(world, entity, local.to_affine());
            #[cfg(debug_assertions)]
            STILL_FRAME_COMPOSES.fetch_add(1, Ordering::Relaxed);
            stack.push((entity, 0));
        } else {
            // A dirty non-root: seed the descent from this node's ROOT ancestor.
            // The root's `GlobalTransform` is always valid (recomposed in this
            // pass if the root itself is dirty; otherwise unchanged-and-correct
            // from the prior frame, since a root that did not move keeps its
            // world pose). Descending from a valid root recomposes the entire
            // path down to (and through) the dirty node — seeding from the
            // immediate parent would be unsound when that parent is itself a
            // not-yet-recomposed dirty non-root. Re-descending an already-correct
            // sibling subtree is idempotent (a node's value depends only on its
            // parent affine + its own local), so the redundancy is harmless.
            let root = root_ancestor(world, entity, child_of_id);
            stack.push((root, 0));
        }
    }

    // ── Step 3: dirty-subtree descent ───────────────────────────────────────
    // The frontier is a single reused `Vec<(Entity, depth)>`. Each iteration pops
    // a `(node, depth)` whose `GlobalTransform` is already finalized, composes
    // each of its children at `depth + 1`, and leaves the composed children on the
    // frontier so their own subtrees follow. A child without a `Transform` is
    // composed-skipped and truncated off the frontier (nothing to propagate
    // through it).
    //
    // `steps` (total visits, `MAX_DESCENT_STEPS`) and the per-node `depth`
    // (per-path chain length, `MAX_TRANSFORM_DEPTH`) are the release-mode
    // termination guards for a malformed `ChildOf` cycle the kernel does not
    // detect: a cycle-free tree never approaches either.
    let mut steps = 0usize;
    while let Some((parent, depth)) = stack.pop() {
        steps += 1;
        if steps > MAX_DESCENT_STEPS {
            descent_step_cap_hit();
            break;
        }

        // Per-path depth guard (the deep-cycle terminator). A cycle-free chain is
        // far shorter than `MAX_TRANSFORM_DEPTH`; a node reaching it can only be a
        // malformed `ChildOf` cycle the kernel does not detect. Drop this node's
        // subtree (do not push its children) so the chain terminates locally.
        if depth >= MAX_TRANSFORM_DEPTH {
            transform_depth_cap_hit();
            continue;
        }

        // Read the parent's finalized world affine by value (Copy). A node
        // missing `GlobalTransform` (e.g. an out-of-tree parent of a dirty
        // non-root) has no pose to compose its children from — skip its subtree.
        let parent_global = match read_global(world, parent, global_id) {
            Some(g) => g,
            None => continue,
        };

        // Copy the parent's children onto the frontier tail. We cannot hold the
        // `&Children` borrow across the `&mut self` child writes below, so copy
        // the `Entity` ids first (no heap: `extend` into the reused worklist).
        // `keep` is the compaction write cursor over the tail.
        let child_depth = depth + 1;
        let tail_start = stack.len();
        if let Some(children) = world.get_component::<Children>(parent) {
            stack.extend(children.as_slice().iter().map(|&c| (c, child_depth)));
        }
        let tail_end = stack.len();

        let mut keep = tail_start;
        for i in tail_start..tail_end {
            let (child, child_d) = stack[i];
            let Some(local) = world.get_component::<Transform>(child).copied() else {
                // No `Transform` ⇒ nothing to compose; drop it from the frontier
                // (do not descend — it carries no spatial pose).
                continue;
            };
            // child.global = parent.global ∘ child.local (Affine3A compose).
            let child_global = parent_global.mul(local.to_affine());
            set_global_if_changed(world, child, child_global);
            #[cfg(debug_assertions)]
            STILL_FRAME_COMPOSES.fetch_add(1, Ordering::Relaxed);
            // Keep the composed child on the frontier so its own subtree follows.
            stack[keep] = (child, child_d);
            keep += 1;
        }
        // Drop the trailing slots vacated by composed-skipped children.
        stack.truncate(keep);
    }

    // ── Restore the reused buffers + advance the horizon ────────────────────
    // `detached` was drained above (emptied, capacity retained); put it back so a
    // future detach observer reuses its capacity. A detach that fired DURING this
    // run's apply window cannot exist (propagation is exclusive `&mut world`, no
    // apply window is open inside it), so nothing was appended since the drain.
    let scratch = scratch_mut(world);
    scratch.stack = stack;
    scratch.dirty = dirty;
    scratch.detached = detached;
    scratch.last_run = this_run;
}

/// Collects the spatial entities whose `Transform` **or** `ChildOf`
/// `changed_tick` falls in the window `(last_run, this_run]` into `out`
/// (assumed pre-`clear()`ed by the caller). Walks the archetypes hosting both
/// spatial columns once; each row resolves its entity once and reads its change
/// ticks once — O(entities in matched archetypes), alloc-free into `out`.
///
/// The `ChildOf` leg is what makes a re-parent / first-attach dirty: that
/// operation stamps the child's `ChildOf` `changed_tick` (in-place replace:
/// `insert_command.rs`; migrate-insert: `migration_helpers.rs`) but never
/// touches the local `Transform`'s tick.
fn collect_dirty(
    world: &EcsMaster,
    transform_id: ComponentId,
    global_id: ComponentId,
    child_of_id: ComponentId,
    last_run: Tick,
    this_run: Tick,
    out: &mut Vec<Entity>,
) {
    // Hold the `&ArchetypeMaster` borrow only for the read walk; every call in
    // the body is `&self` (`get_entity`, `get_component_changed_tick`), so the
    // shared borrows coexist. No `&mut world` is taken here (the seeding pass
    // owns the writes), so the borrow is sound.
    let archetypes = world.archetype_master();
    for archetype in archetypes.iter_archetypes() {
        // Archetype-level gate: only walk archetypes hosting BOTH spatial
        // columns (the spatial set). A non-spatial archetype is skipped wholesale.
        let mask = archetype.signature().mask();
        if !mask.contains(transform_id) || !mask.contains(global_id) {
            continue;
        }
        let live = archetype.entity_count();
        for unit_index in 0..live {
            let Some(entity_id) = archetype.get_entity_id_at(InlandPoolId(unit_index)) else {
                continue;
            };
            let Some(entity) = world.get_entity(entity_id) else {
                continue;
            };
            let transform_changed = world
                .get_component_changed_tick(entity, transform_id)
                .is_some_and(|tick| tick.is_newer_than(last_run, this_run));
            // The `ChildOf` leg: present only on non-root entities; absent on
            // roots (then this is a cheap `None`).
            let link_changed = world
                .get_component_changed_tick(entity, child_of_id)
                .is_some_and(|tick| tick.is_newer_than(last_run, this_run));
            if transform_changed || link_changed {
                out.push(entity);
            }
        }
    }
}

/// Computes the world affine of `entity` by walking its `ChildOf` ancestors and
/// folding their `Transform`s — a COLD single-entity helper (does not consult the
/// cached `GlobalTransform`, so it is correct even mid-frame before
/// [`propagate_transforms`] has run).
///
/// Returns [`Affine3A::IDENTITY`] for an entity without a `Transform`. Walks at
/// most [`MAX_ANCESTOR_DEPTH`] ancestors (the cycle bail-out) before stopping;
/// allocation-free.
pub fn compute_global_transform(world: &EcsMaster, entity: Entity) -> Affine3A {
    let mut current = entity;
    // Fold from the leaf up: world = root.local ∘ … ∘ entity.local, applied as
    // we ascend by left-multiplying each ancestor (parent ∘ accumulated).
    let mut acc = match world.get_component::<Transform>(current) {
        Some(t) => t.to_affine(),
        None => return Affine3A::IDENTITY,
    };
    let mut depth = 0usize;
    while let Some(child_of) = world.get_component::<ChildOf>(current) {
        depth += 1;
        if depth > MAX_ANCESTOR_DEPTH {
            // A malformed deep `ChildOf` cycle (kernel footgun). Bail out loud in
            // debug, quietly in release, rather than spinning forever.
            debug_assert!(
                false,
                "compute_global_transform: ancestor depth cap hit — a ChildOf cycle?"
            );
            break;
        }
        let parent = child_of.0;
        let parent_local = match world.get_component::<Transform>(parent) {
            Some(t) => t.to_affine(),
            None => break,
        };
        acc = parent_local.mul(acc);
        current = parent;
    }
    acc
}

/// Walks `entity`'s `ChildOf` chain to its root ancestor (the topmost node with
/// no `ChildOf`). Returns `entity` itself if it is already a root. Bounded by
/// [`MAX_ANCESTOR_DEPTH`] (the cycle bail-out); allocation-free.
#[inline]
fn root_ancestor(world: &EcsMaster, entity: Entity, child_of_id: ComponentId) -> Entity {
    let mut current = entity;
    let mut depth = 0usize;
    while world.has_component(current, child_of_id) {
        depth += 1;
        if depth > MAX_ANCESTOR_DEPTH {
            // A malformed deep `ChildOf` cycle (kernel footgun): stop ascending
            // and treat the current node as a pseudo-root. Loud in debug, quiet
            // in release — never an infinite loop.
            debug_assert!(
                false,
                "root_ancestor: ancestor depth cap hit — a ChildOf cycle?"
            );
            break;
        }
        match world.get_component::<ChildOf>(current) {
            Some(child_of) => current = child_of.0,
            None => break,
        }
    }
    current
}

/// Resolves (or lazily inserts) the propagation scratch resource on `world`.
#[inline]
fn scratch_mut(world: &mut EcsMaster) -> &mut TransformPropagationScratch {
    if world.try_resource::<TransformPropagationScratch>().is_none() {
        world.insert_resource(TransformPropagationScratch::default());
    }
    world
        .try_resource_mut::<TransformPropagationScratch>()
        .expect("invariant: scratch was just inserted")
}

/// Reads `entity`'s cached world [`Affine3A`] by value (`Copy`) via the
/// by-entity raw pool accessor. `None` when the entity is stale or lacks the
/// `GlobalTransform` column.
#[inline]
fn read_global(world: &EcsMaster, entity: Entity, global_id: ComponentId) -> Option<Affine3A> {
    let raw = world.get_component_raw(entity, global_id)?;
    // SAFETY: `get_component_raw` returns a `*const u8` to the entity's
    //   `GlobalTransform` row for the column registered under `global_id ==
    //   GlobalTransform::component_id()`, so the bytes are a valid, initialized
    //   `GlobalTransform` (`#[repr(C, align(16))]`, layout pinned by const-assert).
    //   The pointer is valid for the `&world` borrow; we read the whole value
    //   out by COPY (`GlobalTransform: Copy`) and drop the pointer on this line,
    //   so no reference into the row outlives this read — see the
    //   `set_global_if_changed` value-copy discipline doc for why this is what
    //   makes the descent sound.
    let global = unsafe { *(raw as *const GlobalTransform) };
    Some(global.0)
}

/// Writes `value` into `entity`'s `GlobalTransform` **only if it differs** from
/// the current cached value, bumping the row's `changed_tick` on a real change
/// (F2). Silently skips a stale entity / missing column.
///
/// # Why a tick-bumping set-if-changed write (F2)
///
/// The earlier implementation wrote through `get_component_raw_mut`, which
/// BYPASSES the `Mut`-guard change-detection stamp — so a recomposed
/// `GlobalTransform` never advanced its `changed_tick`, and downstream
/// `Changed<GlobalTransform>` consumers (camera `ViewUniform`, GPU-instance
/// upload, lights) could not observe a propagated move. This routes the write
/// through [`EcsMaster::get_component_mut`] + [`Mut::set_if_neq`] instead:
///
/// * `set_if_neq` writes (and bumps the tick) ONLY when the new affine differs
///   from the cached one (`GlobalTransform: PartialEq`). An unchanged recompose
///   (e.g. a re-descent of an already-correct sibling subtree) bumps NO tick, so
///   `Changed<GlobalTransform>` stays precise and the 0%-overhead property holds
///   for values that did not actually move.
/// * On this direct (`&mut world`, system-less) path the `Mut` stamps the row at
///   `current_tick()`, so a real move is observed by a `Changed<GlobalTransform>`
///   reader on the following frame (the established `get_component_mut` semantics).
///
/// # Aliasing soundness (the hierarchy descent's CRITICAL discipline)
///
/// The descent reads a parent's `GlobalTransform` ([`read_global`]) and writes a
/// child's `GlobalTransform` (here). The TRUE invariants that make this sound:
///
/// * **Value-copy sequencing.** [`read_global`] reads the parent's affine out
///   **by value** (`Copy`) and drops its raw `*const` on that same line; the
///   parent affine is held only as a plain `Affine3A` local by the time this
///   write runs. No reference or pointer into any `GlobalTransform` row is live
///   across the write — `get_component_mut` borrows the whole world afresh and
///   returns a `Mut` that is dropped at the end of this call.
/// * **Whole-world exclusivity.** The descent is single-threaded (v1) and runs
///   under `&mut EcsMaster`; `get_component_mut` reborrows `&mut world` for the
///   guard's lifetime, the sole access to the pool for the write's duration.
/// * **Distinct entities ⇒ distinct rows.** In a well-formed tree a parent and
///   its child are different entities, hence different storage rows; the write
///   never targets the row whose value the matching read just copied out. Memory
///   safety does NOT rely on this (the value-copy sequencing above already makes
///   even a repeat-row visit sound), but it is the structural truth.
///
/// # Termination (the deep-cycle guard)
///
/// Deep `ChildOf` cycles (A→B→A) are NOT detected by the kernel — only the
/// one-compare self-reference (A→A) is guarded (`hierarchy/mod.rs`). The earlier
/// SAFETY comment's claim that "cycles are impossible" was false. The descent
/// therefore carries a real per-path depth bound ([`MAX_TRANSFORM_DEPTH`], the
/// twin of `boyko_ui`'s `MAX_LAYOUT_DEPTH`) plus the global visit cap
/// ([`MAX_DESCENT_STEPS`]) and the ascent cap ([`MAX_ANCESTOR_DEPTH`] in
/// [`root_ancestor`]): a pathological chain terminates (fail loud in debug, bail
/// out in release) rather than hang or read out of bounds.
#[inline]
fn set_global_if_changed(world: &mut EcsMaster, entity: Entity, value: Affine3A) {
    if let Some(mut guard) = world.get_component_mut::<GlobalTransform>(entity) {
        // `set_if_neq` writes through the inner `&mut GlobalTransform` and bumps
        // the row's `changed_tick` (to `current_tick()`) ONLY on a real change —
        // no raw pointer, no `unsafe` here (the Mut guard owns the soundness).
        guard.set_if_neq(GlobalTransform(value));
    }
}

/// The descent's per-path depth-cap branch (the deep-cycle terminator). Cold:
/// reached only on a malformed `ChildOf` cycle a cycle-free tree never builds.
#[cold]
#[inline(never)]
fn transform_depth_cap_hit() {
    debug_assert!(
        false,
        "propagate_transforms: per-path depth cap (MAX_TRANSFORM_DEPTH) hit — a \
         ChildOf cycle? (deep A→B→A cycles are a documented kernel footgun, \
         undetected by the kernel's self-reference-only guard)"
    );
}

/// The descent's total-visit-cap branch. Cold: see [`transform_depth_cap_hit`].
#[cold]
#[inline(never)]
fn descent_step_cap_hit() {
    debug_assert!(
        false,
        "propagate_transforms: descent step cap (MAX_DESCENT_STEPS) hit — a \
         ChildOf cycle? (deep cycles are a documented kernel footgun)"
    );
}

/// F1 detach observer: marks an entity dirty when its `ChildOf` is fully REMOVED.
///
/// Fires (as an `on_remove` observer on `ChildOf`) when a child is detached
/// — `remove_parent` / `remove_children` / `clear_children` route through a full
/// `ChildOf` removal, which the kernel's remove-migration site reports to
/// `on_remove` observers. The detached entity is now a ROOT, but the removal left
/// no per-row `changed_tick` the propagation's dirty scan can read (the row no
/// longer hosts `ChildOf`), so the orphaned entity would otherwise keep its STALE
/// parent-relative `GlobalTransform`. This appends it to the propagation scratch's
/// detach queue, which [`propagate_transforms`] drains at the next run and
/// recomposes as a root (`GlobalTransform = local.to_affine()`).
///
/// A reparent (`set_parent`) is a `ChildOf` REPLACE, not a remove — it does NOT
/// fire this observer; it is already caught by the dirty scan's `ChildOf`-tick
/// leg. So this observer fires ONLY on the genuinely-broken detach leg, keeping
/// the 0%-overhead property (a non-detach frame never touches the queue).
///
/// # Safety
///
/// Matches the [`ObserverFn`](boyko_ecs::ecs::core::component::observers::ObserverFn)
/// contract: invoked only inside the single-threaded apply window with a valid
/// [`DeferredEcsMaster`]; the body takes no `&mut`-into-storage (it only mutates
/// the propagation `Resource`, which lives outside archetype storage).
unsafe fn child_of_on_remove(mut world: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    // The orphaned child (the entity losing `ChildOf`). `resource_mut` reaches the
    // propagation scratch (a `Resource`, disjoint from archetype storage); the
    // queue is appended only on this rare detach event.
    if let Some(scratch) = world.resource_mut::<TransformPropagationScratch>() {
        scratch.detached.push(ctx.entity);
    }
    // else: no propagation scratch yet ⇒ propagation has never run for this world,
    // so there is no cached `GlobalTransform` to go stale — nothing to mark.
}

/// Installs [`child_of_on_remove`] as a `ChildOf` `on_remove` observer exactly
/// once per world (F1), and ensures the propagation scratch resource exists so
/// the observer's `resource_mut` succeeds for any detach after this point.
///
/// Idempotent and lazily called from [`propagate_transforms`] (and eagerly from
/// [`TransformPlugin`](crate::plugin::TransformPlugin)). The `OnceLock` is the
/// world-local "already installed" flag stored in the scratch resource, so a
/// second world gets its own observer (observers are per-world —
/// `observers/mod.rs`). Registration allocates only the first time (the observer
/// list's first entry), never per frame — the 0%-steady-state property holds.
pub(crate) fn ensure_detach_observer(world: &mut EcsMaster) {
    // Materialize the scratch first (so the observer's `resource_mut` will hit),
    // then check the once-flag and drop the scratch borrow before registering
    // (`observe_on_remove` needs its own `&mut world`).
    if scratch_mut(world).detach_observer_installed.get().is_some() {
        return;
    }
    world.observe_on_remove::<ChildOf>(child_of_on_remove as ObserverFn);
    // Mark installed. Single-threaded `&mut world`, so this `set` always wins;
    // the `Err` arm is unreachable but ignored rather than `unwrap`ed.
    let _ = scratch_mut(world).detach_observer_installed.set(());
}
