//! Phase 19 — Hierarchy (ChildOf / Children) BEHAVIORAL + integration tests.
//!
//! The relationship is driven ENTIRELY by `ChildOf` insert / remove via
//! `Commands` / `EntityCommands` (`add_child`, `set_parent`, `remove_parent`,
//! `remove_children`, `clear_children`, `despawn_without_children`) or direct
//! `ChildOf` insertion. `Children` is read-only to users.
//!
//! # The consistency window (the harness mechanism)
//!
//! `Children` is mutated by deferred commands the `ChildOf` hooks enqueue, so it
//! is consistent only AFTER the deferred-hook-queue drain. Each `run_system`
//! call (and each direct `delete_entity` / `despawn_without_children`) drives an
//! apply-window drain (`ecs_master.rs` / `schedule.rs`), which is the drain
//! trigger these tests rely on: every mutation is issued inside a `run_system`
//! closure (or via a direct-API call), and the assertion reads `Children`
//! AFTER that call returns.
//!
//! # Capturing freshly-spawned entity handles
//!
//! `Commands::spawn(bundle)` reserves an `Entity` (live only after apply), so
//! the IDs are smuggled out of the (`Send + Sync`) system closure through an
//! `Arc<Mutex<Vec<Entity>>>` probe — the established Phase 11 pattern — then
//! read back after the `run_system` apply window.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

/// A trivial marker so spawned entities have a concrete archetype to live in
/// before `ChildOf` migrates them. Tag only; never read.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Tag(u32);

#[derive(Bundle)]
struct TagBundle {
    t: Tag,
}

/// Spawns `n` marker entities through the deferred queue and returns their
/// (now-live) handles, in spawn order. One apply window runs before return, so
/// every returned entity satisfies `ecs.has_entity`.
fn spawn_entities(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut local = probe.lock().expect("probe lock");
        for i in 0..n {
            local.push(cmds.spawn(TagBundle { t: Tag(i as u32) }).id());
        }
    });
    let out = sink.lock().expect("probe lock").clone();
    assert_eq!(out.len(), n, "spawn helper produced n handles");
    for &e in &out {
        assert!(ecs.has_entity(e), "spawned entity is live after the apply window");
    }
    out
}

/// Reads a parent's current children as an owned `Vec` (post-drain snapshot).
/// `None` when the parent has no `Children` component at all (vs. an empty one).
fn children_of(ecs: &EcsMaster, parent: Entity) -> Option<Vec<Entity>> {
    ecs.get_component::<Children>(parent).map(|c| c.as_slice().to_vec())
}

/// Reads a child's current parent (the `ChildOf` FK), if any.
fn parent_of(ecs: &EcsMaster, child: Entity) -> Option<Entity> {
    ecs.get_component::<ChildOf>(child).map(|c| c.0)
}

// ════════════════════════════════════════════════════════════════════════════
// Test 1 — link: add_child establishes BOTH directions of the invariant
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn add_child_links_both_directions() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (parent, child) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });

    // Core invariant: c.ChildOf == parent  ⟺  parent.Children ∋ c.
    assert_eq!(parent_of(&ecs, child), Some(parent), "child.ChildOf points at parent");
    let kids = children_of(&ecs, parent).expect("parent gained a Children collection");
    assert!(kids.contains(&child), "parent.Children contains the child");
    assert_eq!(kids.len(), 1, "exactly one child linked");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 2 — unlink: remove_parent / remove_children sever BOTH directions
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn remove_parent_unlinks_both_directions() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (parent, child) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });
    assert_eq!(parent_of(&ecs, child), Some(parent), "linked before unlink");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(child).remove_parent();
    });

    assert_eq!(parent_of(&ecs, child), None, "child has no ChildOf after remove_parent");
    let kids = children_of(&ecs, parent).expect("Children retained when emptied");
    assert!(!kids.contains(&child), "parent.Children no longer holds the child");
}

#[test]
fn remove_children_unlinks_both_directions() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (parent, child) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).remove_children(&[child]);
    });

    assert_eq!(parent_of(&ecs, child), None, "child unlinked via remove_children");
    let kids = children_of(&ecs, parent).expect("Children retained");
    assert!(!kids.contains(&child), "child removed from parent.Children");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 3 — reparent BOTH paths (R2 §C2): fresh-add and overwrite-in-place
// ════════════════════════════════════════════════════════════════════════════

/// (a) FRESH `ChildOf` on a child that had no parent — the MIGRATE path
/// (on_add + on_insert, link-only). Post-drain: child in B's list only.
#[test]
fn reparent_fresh_add_is_link_only() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (b, child) = (e[0], e[1]);

    // Child had NO parent → this is a fresh add (migrate), not an overwrite.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(child).set_parent(b);
    });

    assert_eq!(parent_of(&ecs, child), Some(b), "child.ChildOf == B after fresh add");
    let b_kids = children_of(&ecs, b).expect("B gained Children");
    assert!(b_kids.contains(&child), "child ∈ B.Children");
    assert_eq!(b_kids.len(), 1, "exactly one child in B");
}

/// (b) OVERWRITE `ChildOf` A→B on a child that already had parent A — the
/// in-place replace (on_replace → Unlink(A) THEN on_insert → Link(B)).
/// Reparent atomicity: child in EXACTLY ONE parent's list post-drain.
#[test]
fn reparent_overwrite_moves_child_to_new_parent_atomically() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (a, b, child) = (e[0], e[1], e[2]);

    // First link to A.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).add_child(child);
    });
    assert_eq!(parent_of(&ecs, child), Some(a), "child under A first");
    assert!(children_of(&ecs, a).unwrap().contains(&child), "A has the child");

    // Overwrite ChildOf A → B (reparent). The child already has ChildOf, so this
    // is the in-place-replace path firing on_replace(A) THEN on_insert(B).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(child).set_parent(b);
    });

    assert_eq!(parent_of(&ecs, child), Some(b), "child.ChildOf == B after reparent");
    assert!(children_of(&ecs, b).unwrap().contains(&child), "child ∈ B.Children");
    assert!(!children_of(&ecs, a).unwrap().contains(&child), "child ∉ A.Children");

    // Atomicity: in exactly ONE parent's list.
    let in_a = children_of(&ecs, a).unwrap().contains(&child);
    let in_b = children_of(&ecs, b).unwrap().contains(&child);
    assert!(in_b && !in_a, "child is in exactly one parent's list (B's)");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 4 — recursive despawn depth >= 2: a 3-level tree vanishes entirely
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn recursive_despawn_three_levels_removes_all() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (grandparent, parent, child) = (e[0], e[1], e[2]);

    // grandparent → parent → child.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(grandparent).add_child(parent);
        cmds.entity(parent).add_child(child);
    });
    assert_eq!(parent_of(&ecs, parent), Some(grandparent));
    assert_eq!(parent_of(&ecs, child), Some(parent));

    // Default-recursive despawn of the root.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(grandparent).despawn();
    });

    assert!(!ecs.has_entity(grandparent), "grandparent gone");
    assert!(!ecs.has_entity(parent), "parent gone (cascade depth 1)");
    assert!(!ecs.has_entity(child), "child gone (cascade depth 2)");
    assert_eq!(ecs.entity_count(), 0, "no entity survives the recursive despawn");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 5 — despawn_without_children: the opt-out; children survive (dangling)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn despawn_without_children_keeps_children_alive() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (parent, c0, c1) = (e[0], e[1], e[2]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(c0);
        cmds.entity(parent).add_child(c1);
    });
    assert_eq!(children_of(&ecs, parent).unwrap().len(), 2, "two children linked");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).despawn_without_children();
    });

    assert!(!ecs.has_entity(parent), "parent despawned");
    assert!(ecs.has_entity(c0), "child 0 survives the opt-out despawn");
    assert!(ecs.has_entity(c1), "child 1 survives the opt-out despawn");
    // Documented footgun: each child's ChildOf now dangles (points at the freed
    // parent). It is NOT auto-cleared (no cascade ran).
    assert_eq!(parent_of(&ecs, c0), Some(parent), "child 0 ChildOf dangles at freed parent");
    assert_eq!(parent_of(&ecs, c1), Some(parent), "child 1 ChildOf dangles at freed parent");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 6 — self-ref guard: ChildOf(self) is removed, no corruption, no panic
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn self_referential_child_of_is_removed_without_corruption() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 1);
    let me = e[0];

    // Insert ChildOf(self). The on_insert guard must reject it.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(me).set_parent(me);
    });

    assert_eq!(parent_of(&ecs, me), None, "self-referential ChildOf was removed");
    // No phantom self-membership: even if a Children collection exists, it must
    // not contain `me` (the spurious UnlinkChild(me, me) no-oped).
    if let Some(kids) = children_of(&ecs, me) {
        assert!(!kids.contains(&me), "no self-membership in Children");
    }
    assert!(ecs.has_entity(me), "entity still alive (guard didn't corrupt it)");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 7 — dangling-parent guard: ChildOf(nonexistent) is removed, no phantom
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dangling_parent_child_of_is_removed_no_phantom() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (child, victim) = (e[0], e[1]);

    // Despawn `victim` so its handle is stale, then point `child` at it.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(victim).despawn();
    });
    assert!(!ecs.has_entity(victim), "victim is dead — a dangling target");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(child).set_parent(victim);
    });

    assert_eq!(parent_of(&ecs, child), None, "dangling ChildOf was removed");
    assert!(!ecs.has_entity(victim), "no phantom resurrection of the dead parent");
    // No Children collection conjured on the dead id (it cannot host components).
    assert_eq!(children_of(&ecs, victim), None, "no phantom Children on the dead parent");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 8 — #20106 read-before-remove: the cascade despawns the CURRENT children
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn cascade_despawns_all_current_children_read_before_remove() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 5);
    let parent = e[0];
    let kids = [e[1], e[2], e[3], e[4]];

    ecs.run_system(move |mut cmds: Commands| {
        for &c in &kids {
            cmds.entity(parent).add_child(c);
        }
    });
    assert_eq!(
        children_of(&ecs, parent).unwrap().len(),
        4,
        "all four children are live links at despawn time"
    );

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).despawn();
    });

    // Every child that was a live link at despawn time is cascaded (#20106: the
    // cascade reads the CURRENT collection, not a stale snapshot).
    for (i, &c) in kids.iter().enumerate() {
        assert!(!ecs.has_entity(c), "child {i} (live at despawn) was cascaded");
    }
    assert_eq!(ecs.entity_count(), 0, "parent + all current children gone");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 9 — re-entrancy single-drain: 3-level tree completes in one outermost
//          drain (grandchildren enqueued mid-drain are absorbed)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn recursive_despawn_completes_in_single_outermost_drain() {
    let mut ecs = EcsMaster::new();
    // A wider, deeper tree to make a broken single-drain leave observable
    // orphans: gp → {p0, p1}; p0 → {c0, c1}; p1 → {c2}.
    let e = spawn_entities(&mut ecs, 6);
    let (gp, p0, p1, c0, c1, c2) = (e[0], e[1], e[2], e[3], e[4], e[5]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(gp).add_child(p0);
        cmds.entity(gp).add_child(p1);
        cmds.entity(p0).add_child(c0);
        cmds.entity(p0).add_child(c1);
        cmds.entity(p1).add_child(c2);
    });

    // A single despawn of the root. If the single-drain were broken,
    // grandchildren enqueued mid-drain would survive as orphans.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(gp).despawn();
    });

    for (name, ent) in [("gp", gp), ("p0", p0), ("p1", p1), ("c0", c0), ("c1", c1), ("c2", c2)] {
        assert!(!ecs.has_entity(ent), "{name} cascaded in the single outermost drain");
    }
    assert_eq!(ecs.entity_count(), 0, "no orphan survives (single-drain absorbed grandchildren)");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 9b — WIDE cascade path (> CASCADE_FANOUT_INLINE children): the cold
//           per-turn-re-derive branch in children_on_replace must despawn all
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn recursive_despawn_wide_fanout_cascades_all() {
    let mut ecs = EcsMaster::new();
    // 40 children > CASCADE_FANOUT_INLINE (32) forces the wide path.
    const FANOUT: usize = 40;
    let e = spawn_entities(&mut ecs, FANOUT + 1);
    let parent = e[0];
    let kids: Vec<Entity> = e[1..].to_vec();

    let kids_for_link = kids.clone();
    ecs.run_system(move |mut cmds: Commands| {
        for &c in &kids_for_link {
            cmds.entity(parent).add_child(c);
        }
    });
    assert_eq!(
        children_of(&ecs, parent).unwrap().len(),
        FANOUT,
        "all {FANOUT} children linked (over the inline threshold)"
    );

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).despawn();
    });

    assert!(!ecs.has_entity(parent), "parent gone");
    for (i, &c) in kids.iter().enumerate() {
        assert!(!ecs.has_entity(c), "wide-path child {i} cascaded");
    }
    assert_eq!(ecs.entity_count(), 0, "wide cascade removed every child");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 10 — clear_children: children survive, none has ChildOf, list empties
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn clear_children_unlinks_all_without_despawning() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 4);
    let parent = e[0];
    let kids = [e[1], e[2], e[3]];

    ecs.run_system(move |mut cmds: Commands| {
        for &c in &kids {
            cmds.entity(parent).add_child(c);
        }
    });
    assert_eq!(children_of(&ecs, parent).unwrap().len(), 3, "three children linked");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).clear_children();
    });

    assert!(ecs.has_entity(parent), "parent survives clear_children");
    for (i, &c) in kids.iter().enumerate() {
        assert!(ecs.has_entity(c), "child {i} survives clear_children (NOT despawned)");
        assert_eq!(parent_of(&ecs, c), None, "child {i} has no ChildOf after clear");
    }
    let kids_after = children_of(&ecs, parent).expect("Children retained (empty)");
    assert!(kids_after.is_empty(), "parent.Children is empty after clear_children");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 11 — remove_children(&[subset]): only the listed children are unlinked
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn remove_children_subset_keeps_others_linked() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 4);
    let parent = e[0];
    let (c_keep, c_drop0, c_drop1) = (e[1], e[2], e[3]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(c_keep);
        cmds.entity(parent).add_child(c_drop0);
        cmds.entity(parent).add_child(c_drop1);
    });
    assert_eq!(children_of(&ecs, parent).unwrap().len(), 3, "three linked");

    // Remove ONLY two of the three.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).remove_children(&[c_drop0, c_drop1]);
    });

    let kids = children_of(&ecs, parent).expect("Children retained");
    assert!(kids.contains(&c_keep), "the unlisted child stays linked");
    assert!(!kids.contains(&c_drop0), "listed child 0 unlinked");
    assert!(!kids.contains(&c_drop1), "listed child 1 unlinked");
    assert_eq!(kids.len(), 1, "remove_children did NOT clear all — only the subset");

    assert_eq!(parent_of(&ecs, c_keep), Some(parent), "kept child still points at parent");
    assert_eq!(parent_of(&ecs, c_drop0), None, "dropped child 0 lost its ChildOf");
    assert_eq!(parent_of(&ecs, c_drop1), None, "dropped child 1 lost its ChildOf");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 12 — first-child insert fires NO spurious cascade
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn first_child_insert_fires_no_spurious_cascade() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (parent, child) = (e[0], e[1]);

    // Parent had NO Children. Linking the first child inserts `Children`
    // (migrate, on_add + on_insert). `Children` registers only on_replace, so no
    // cascade fires — neither parent nor child is despawned.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });

    assert!(ecs.has_entity(parent), "parent alive — no spurious cascade on first-child insert");
    assert!(ecs.has_entity(child), "child alive — first-child insert did not cascade-despawn it");
    let kids = children_of(&ecs, parent).expect("parent has Children");
    assert_eq!(kids.as_slice(), &[child], "parent.Children == [child]");
    assert_eq!(ecs.entity_count(), 2, "both entities survive");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 13 — keep-empty Children (R2 W1): an emptied collection is RETAINED
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn emptied_children_collection_is_retained_not_removed() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (parent, child) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });
    assert!(ecs.get_component::<Children>(parent).is_some(), "Children created on first link");

    // Remove the only child.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(child).remove_parent();
    });

    // The collection is RETAINED (not removed) and is empty — the W1 no-archetype-
    // thrash decision.
    let kids = ecs
        .get_component::<Children>(parent)
        .expect("Children component is RETAINED after emptying (W1)");
    assert!(kids.is_empty(), "retained Children is empty");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 14 — cascade-suppress scoping: despawn_without_children suppresses ONLY
//           this entity's cascade, not an unrelated despawn an observer enqueues
// ════════════════════════════════════════════════════════════════════════════

use std::sync::atomic::{AtomicU64, Ordering};

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::ObserverContext;
use boyko_ecs::ecs::identifiers::primitives::EntityId;

const SEQ: Ordering = Ordering::SeqCst;

/// Packs an Entity (id + generation) into a `u64` static so a bare-fn observer
/// (which cannot capture) can read it. Mirrors the 14b `TargetCell` helper.
struct TargetCell(AtomicU64);
impl TargetCell {
    const fn new() -> Self {
        Self(AtomicU64::new(u64::MAX))
    }
    fn set(&self, e: Entity) {
        self.0.store((e.id().0 as u64) | ((e.generation() as u64) << 32), SEQ);
    }
    fn get(&self) -> Entity {
        let packed = self.0.load(SEQ);
        Entity::new(EntityId((packed & 0xFFFF_FFFF) as usize), (packed >> 32) as u32)
    }
}

/// `Y`, an UNRELATED parent-with-children, is despawned by `X`'s on_remove
/// observer. `X` is despawned via `despawn_without_children`, so X's OWN cascade
/// is suppressed — but the suppress guard drops before the drain, so Y's cascade
/// (triggered by the observer-enqueued despawn) is NOT suppressed.
static SUPPRESS_SCOPE_Y: TargetCell = TargetCell::new();

unsafe fn x_on_remove_despawns_y(mut w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    // Enqueue an UNRELATED despawn of Y. This rides the same outermost drain as
    // X's removal, but the cascade-suppress window closed before the drain.
    w.commands().entity(SUPPRESS_SCOPE_Y.get()).despawn();
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct SuppressX(u32);

#[derive(Bundle)]
struct SuppressXBundle {
    x: SuppressX,
}

#[test]
fn despawn_without_children_suppresses_only_self_not_unrelated_cascade() {
    let mut ecs = EcsMaster::new();
    // Observe on_remove of the X-marker → enqueues an unrelated despawn of Y.
    ecs.observe_on_remove::<SuppressX>(x_on_remove_despawns_y);

    // X: has the marker AND children of its own (so X's cascade WOULD fire).
    let x_kids = spawn_entities(&mut ecs, 2);
    let (xc0, xc1) = (x_kids[0], x_kids[1]);

    // Spawn X carrying the SuppressX marker (a separate bundle), then link its
    // children. The marker drives the on_remove observer.
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let x = cmds.spawn(SuppressXBundle { x: SuppressX(1) }).id();
        probe.lock().unwrap().push(x);
    });
    let x = sink.lock().unwrap()[0];
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(x).add_child(xc0);
        cmds.entity(x).add_child(xc1);
    });
    assert_eq!(children_of(&ecs, x).unwrap().len(), 2, "X has two children");

    // Y: an UNRELATED parent with its own children.
    let y_set = spawn_entities(&mut ecs, 3);
    let (y, yc0, yc1) = (y_set[0], y_set[1], y_set[2]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(y).add_child(yc0);
        cmds.entity(y).add_child(yc1);
    });
    assert_eq!(children_of(&ecs, y).unwrap().len(), 2, "Y has two children");
    SUPPRESS_SCOPE_Y.set(y);

    // despawn_without_children(X): X's cascade is suppressed (its children
    // survive), but X's on_remove observer enqueues despawn(Y) — and Y's cascade
    // is NOT suppressed (guard dropped before the drain).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(x).despawn_without_children();
    });

    // X gone; X's OWN children survive (the suppress worked for X).
    assert!(!ecs.has_entity(x), "X despawned");
    assert!(ecs.has_entity(xc0), "X's child 0 survives (X's cascade suppressed)");
    assert!(ecs.has_entity(xc1), "X's child 1 survives (X's cascade suppressed)");

    // Y AND Y's children gone — Y's cascade was NOT suppressed.
    assert!(!ecs.has_entity(y), "Y despawned by the observer (unrelated to the suppress)");
    assert!(!ecs.has_entity(yc0), "Y's child 0 cascaded (Y's cascade NOT suppressed)");
    assert!(!ecs.has_entity(yc1), "Y's child 1 cascaded (Y's cascade NOT suppressed)");
}
