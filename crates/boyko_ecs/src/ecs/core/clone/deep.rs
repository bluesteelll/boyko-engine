//! Deep subtree clone — Algorithm B (Feature 3, plan §7 / D5).
//!
//! Clones `source` and its `ChildOf` subtree: each node is cloned via Algorithm A
//! (shallow — `ChildOf` copied verbatim), then a remap pass rewrites every cloned
//! child's `ChildOf` to point at its cloned parent (via the installed `ChildOf`
//! `MAP_ENTITIES` fn), and the `Children` reverse index is rebuilt through the
//! canonical hierarchy machinery so the cloned subtree is internally consistent.
//!
//! # W6 — no dangling children slice
//!
//! Each node's children are SNAPSHOTTED into the owned worklist BY VALUE before any
//! structural push; the [`Children::as_slice`] borrow is never held across a clone
//! (which reallocates pools). The source archetype pointer is re-resolved per node
//! (`materialize_clone` does this internally) — never cached across a structural op.

use crate::ecs::core::clone::cloner::EntityCloner;
use crate::ecs::core::clone::map::EntityCloneMap;
use crate::ecs::core::clone::materialize::{materialize_clone, materialize_clone_at};
use crate::ecs::core::commands::migration_helpers::{merged_archetype_id, migrate_entity_insert};
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::hierarchy::{ChildOf, Children};

/// Inline worklist capacity before spilling to the heap (mirrors
/// `CASCADE_FANOUT_INLINE` — small subtrees never touch the `Vec` allocator).
const DEEP_WORKLIST_INLINE: usize = 32;

/// Deep-clones `source` and its `ChildOf` subtree per `cloner`. Returns the cloned
/// root (a freshly-allocated id). Caller guarantees `source` is alive and
/// `cloner.is_deep()`. Used by the DIRECT API (`EcsMaster::clone_subtree`).
///
/// The deep path always SHALLOW-clones each node (the per-node `cloner` is forced
/// shallow so a node does not recursively re-enter this walk); the recursion is
/// driven explicitly by the worklist + the `Children` reverse index.
#[inline]
pub(crate) fn clone_subtree(
    world: &mut EcsMaster,
    source_root: Entity,
    cloner: &EntityCloner,
) -> Entity {
    clone_subtree_inner(world, source_root, None, cloner)
}

/// Deferred-path deep clone: the ROOT lands in the PRE-RESERVED `reserved_root`
/// (minted at the `Commands::clone_and_spawn` callsite); descendants get fresh ids.
/// Used by `CloneSpawnCommand::apply`.
#[inline]
pub(crate) fn clone_subtree_seeded(
    world: &mut EcsMaster,
    source_root: Entity,
    reserved_root: Entity,
    cloner: &EntityCloner,
) -> Entity {
    clone_subtree_inner(world, source_root, Some(reserved_root), cloner)
}

/// Shared deep-clone body. `reserved_root` = `Some` for the deferred path (the root
/// lands in the pre-reserved id), `None` for the direct path (fresh root id).
fn clone_subtree_inner(
    world: &mut EcsMaster,
    source_root: Entity,
    reserved_root: Option<Entity>,
    cloner: &EntityCloner,
) -> Entity {
    let child_of_id = ChildOf::component_id();
    let children_id = Children::component_id();

    // A per-node cloner forced SHALLOW (so `materialize_clone` does not recurse)
    // but inheriting the filter / fire / strict / tick config.
    let node_cloner = {
        let mut c = *cloner;
        c.force_shallow();
        c
    };

    let mut map = EntityCloneMap::new();

    // Worklist of SOURCE entities still to clone. Seeded with the root; each popped
    // node is cloned, recorded in `map`, then its children are SNAPSHOTTED BY VALUE
    // and pushed (W6). A stack `Vec` is fine — small subtrees stay in the inline
    // capacity. `(source_entity)`; the parent mapping is resolved from `map` in the
    // remap pass, so we do not need to carry it on the worklist.
    let mut worklist: Vec<Entity> = Vec::with_capacity(DEEP_WORKLIST_INLINE);
    worklist.push(source_root);

    // The set of cloned nodes in source→clone order, for the remap + Children
    // rebuild passes (cloned children, NOT the root for the rebuild — the root's
    // ChildOf points outside the subtree and is kept verbatim).
    let mut cloned_nodes: Vec<Entity> = Vec::with_capacity(DEEP_WORKLIST_INLINE);

    let mut depth_guard = 0usize;
    while let Some(src) = worklist.pop() {
        // Diamond dedup (R2 #17726): an entity reachable via two links is cloned
        // once. (A pure ChildOf tree never hits this, but the guard is cheap.)
        if map.contains(src) {
            continue;
        }
        debug_assert!(
            depth_guard < crate::ecs::core::clone::MAX_CLONE_SUBTREE_NODES,
            "deep clone: subtree node cap exceeded (a ChildOf cycle? — only \
             self-reference is guarded)"
        );
        depth_guard += 1;

        // Clone the node SHALLOW (ChildOf copied verbatim). `materialize_clone*`
        // re-resolves the source archetype pointer internally (W6 re-resolve). The
        // ROOT lands in the pre-reserved id on the deferred path (the only
        // user-visible id); descendants always get fresh ids.
        let clone = match reserved_root {
            Some(reserved) if src == source_root => {
                materialize_clone_at(world, src, reserved, &node_cloner).entity
            }
            _ => materialize_clone(world, src, &node_cloner).entity,
        };
        map.insert(src, clone);
        cloned_nodes.push(src);

        // W6: SNAPSHOT this node's children BY VALUE into an owned buffer BEFORE any
        // further structural push. Never hold the `Children::as_slice()` borrow
        // across the next iteration's `materialize_clone` (which reallocates pools).
        let mut snapshot: Vec<Entity> = Vec::new();
        if let Some(children) = world.get_component::<Children>(src) {
            snapshot.extend_from_slice(children.as_slice());
        }
        // The `&Children` borrow is dropped here (snapshot owns plain `Entity`s).
        for child in snapshot {
            if !map.contains(child) {
                worklist.push(child);
            }
        }
    }

    let clone_root = map
        .get(source_root)
        .expect("invariant: the root was cloned first");

    // ── Remap pass: rewrite each cloned NON-root node's ChildOf to its cloned
    // parent. The root's ChildOf (pointing outside the subtree) is left verbatim
    // (the `map.get` returns None for an external parent, so the remap is a no-op
    // for it). After remap we also rebuild the Children reverse index.
    let Some(remap_fn) = component_registry::get_map_entities_fn(child_of_id.0) else {
        // ChildOf installs its remap fn in `hierarchy/mod.rs`; if absent the deep
        // clone cannot remap — return the (verbatim-ChildOf) root and warn.
        debug_assert!(false, "deep clone: ChildOf map_entities_fn not installed");
        return clone_root;
    };

    // For each cloned source node, look up its clone and (if it has a ChildOf)
    // remap the CLONE's ChildOf in place, then rebuild the parent's Children.
    for src in cloned_nodes {
        let clone = map.get(src).expect("invariant: every cloned node is in the map");
        // Resolve the clone's ChildOf dst pointer + remap it via the installed fn.
        // The remap reads `map` to translate source-parent → clone-parent; a parent
        // outside the subtree (the root's external parent) stays verbatim.
        let remapped_parent: Option<Entity> = remap_clone_child_of(world, clone, child_of_id, remap_fn, &map);

        // Rebuild the Children reverse index: if the clone now points at a cloned
        // parent (inside the subtree), link it there via the canonical path. The
        // root's external parent already has the SOURCE root in its Children; the
        // clone-root becomes a sibling (shallow ChildOf verbatim) — we do NOT add
        // the clone-root to the external parent's Children here (that is the
        // shallow-sibling semantics; the external parent's Children is the source
        // tree's concern, and a deep clone of a subtree does not re-link the root
        // into the external parent's collection — Bevy parity: the cloned root is a
        // detached copy unless explicitly reparented).
        if let Some(parent_clone) = remapped_parent
            && parent_clone != clone
        {
            link_child(world, parent_clone, clone, children_id);
        }
    }

    clone_root
}

/// Remaps `clone`'s `ChildOf` in place (if it has one) via the installed
/// `map_entities_fn`, returning the (possibly-remapped) parent the clone now points
/// at, or `None` if the clone has no `ChildOf`.
///
/// `pub(crate)` so the S7 prefab `instantiate` path reuses it VERBATIM (the map is
/// populated source-parent-Entity → instance parent; `remap_fn` is unchanged).
pub(crate) fn remap_clone_child_of(
    world: &mut EcsMaster,
    clone: Entity,
    child_of_id: crate::ecs::identifiers::primitives::ComponentId,
    remap_fn: component_registry::MapEntitiesFn,
    map: &EntityCloneMap,
) -> Option<Entity> {
    // Resolve the clone's ChildOf raw slot. We need a `*mut u8` into the clone's
    // ChildOf pool row; `get_component_raw_mut` gives exactly that.
    let dst = world.get_component_raw_mut(clone, child_of_id)?;
    // SAFETY (D5): `dst` is a live, initialized `ChildOf` row (resolved through the
    //   fast store for this entity's archetype, which hosts ChildOf). `remap_fn`
    //   (= `child_of_map_entities`) forms `&mut ChildOf`, reads `map` (a shared
    //   borrow, not aliased mutably), and rewrites the inner `Entity` in place. It
    //   receives ONLY the raw pointer + the map — no world view (W7-class). Single-
    //   threaded `&mut EcsMaster`.
    unsafe {
        remap_fn(dst, map);
    }
    // Read back the (now-remapped) parent.
    world.get_component::<ChildOf>(clone).map(|c| c.0)
}

/// Links `child` into `parent`'s `Children` reverse index through the canonical
/// hierarchy machinery (mirrors `LinkChildCommand::apply`): an in-place push if the
/// parent already has `Children`, else a `Children` insert via the audited insert
/// path. Runs directly under `&mut EcsMaster` (the deep clone is a direct-API op).
///
/// `pub(crate)` so the S7 prefab `instantiate` path rebuilds each instance parent's
/// `Children` through the IDENTICAL audited machinery (no byte-stored `Children`).
pub(crate) fn link_child(
    world: &mut EcsMaster,
    parent: Entity,
    child: Entity,
    _children_id: crate::ecs::identifiers::primitives::ComponentId,
) {
    if !world.has_entity(parent) {
        return;
    }
    match world.get_component_mut::<Children>(parent) {
        Some(mut children) => children.push(child),
        None => {
            // First child: insert a one-child `Children` via the same audited
            // migration machinery `LinkChildCommand::apply` uses. This fires
            // `on_add` + `on_insert` for `Children` (it registers neither, so no
            // spurious cascade).
            let inland = world.entity_master.entities_inland[parent.id().0];
            // SAFETY (verbatim copy of the audited `LinkChildCommand::apply` /
            //   `insert_command.rs` F1 pattern): `archetype_ptr` is write-capable,
            //   stable, interior-mutable slab provenance — non-null + generation-
            //   matched by the preceding `has_entity`, so the slot is live.
            // BUG-MIGRATE-TB-1: raw projection of `id` — a `.id()` method call
            // auto-refs `&Archetype` (a foreign read that freezes a sibling
            // structural write to `current_index`/`entity_ids`).
            let src = unsafe { core::ptr::addr_of!((*inland.archetype_ptr()).id).read() };
            let tgt = merged_archetype_id::<Children>(world, src);
            migrate_entity_insert::<Children>(world, parent, src, tgt, Children::with_one(child));
        }
    }
}
