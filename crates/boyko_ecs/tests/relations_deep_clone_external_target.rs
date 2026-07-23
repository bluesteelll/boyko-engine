//! Deep-clone × relationship reverse-index regression suite (BUG-EDGE-CLONE-1
//! verification + the BUG-EDGE-CLONE-2 external-target guard).
//!
//! BUG-EDGE-CLONE-1 fix under test: a `LinkSuppressGuard`
//! (`relationship/mod.rs`) held across the deep-clone materialize walk
//! (`clone/deep.rs`) suppresses the verbatim-copied FK's clone-time
//! `on_insert` reverse-index link; the post-remap relink pass
//! (`relationship_clone_relink::<R>`) becomes the SOLE linker. This suite pins:
//!
//!   * T1/T2 — the SOURCE subtree's reverse collections (`LikedBy`, `Children`)
//!     are byte-identical before vs after the clone (the BUG-EDGE-CLONE-1
//!     non-leak invariant), on BOTH a derive relation and the hand-mirrored
//!     `ChildOf`.
//!   * T3 — the CRITICAL external-target case: a cloned node carries a
//!     relationship FK pointing at an entity OUTSIDE the cloned set. The relink
//!     (`relationship_clone_relink`) gates on `map.is_clone(target)` — only
//!     in-subtree targets are relinked — so the test reads the engine's CHOSEN
//!     semantics (does the clone KEEP or DROP the external FK?) and asserts the
//!     FK↔reverse CONSISTENCY invariant under that semantics. A clone that keeps
//!     `Likes(E)` but is absent from `E.LikedBy` is BUG-EDGE-CLONE-2.
//!   * T4 — a within-subtree FK and an external FK mixed in ONE clone: the
//!     in-subtree edge must relink to the clone target; the external edge is
//!     subject to the same consistency invariant as T3.
//!
//! Harness mirrors `relations_derive.rs` / `relations_edge_observers.rs`
//! (Arc<Mutex> spawn probe, distinct user relations, never ChildOf-special for
//! the derive cases).

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;
use boyko_ecs::ecs::core::relationship::{RelationshipSourceCollection, RelationshipTarget};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

// ════════════════════════════════════════════════════════════════════════════
// Relations under test (derive-built, NOT ChildOf-special)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

// ════════════════════════════════════════════════════════════════════════════
// Helpers
// ════════════════════════════════════════════════════════════════════════════

/// Spawns `n` markers through the deferred queue; returns live handles in spawn
/// order.
fn spawn_entities(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut local = probe.lock().expect("probe lock");
        for i in 0..n {
            local.push(cmds.spawn(Tag(i as u32)).id());
        }
    });
    let out = sink.lock().expect("probe lock").clone();
    for &e in &out {
        assert!(ecs.has_entity(e), "spawned entity is live after the apply window");
    }
    out
}

/// `target.LikedBy` as a sorted `Vec<usize>` of source ids (`None` if no
/// `LikedBy` component at all).
fn liked_by_ids(ecs: &EcsMaster, target: Entity) -> Option<Vec<usize>> {
    ecs.get_component::<LikedBy>(target).map(|c| {
        let mut v: Vec<usize> =
            RelationshipSourceCollection::iter(c.collection()).map(|e| e.id().0).collect();
        v.sort_unstable();
        v
    })
}

/// `parent.Children` as a sorted `Vec<usize>` of child ids (`None` if no
/// `Children` component at all).
fn children_ids(ecs: &EcsMaster, parent: Entity) -> Option<Vec<usize>> {
    ecs.get_component::<Children>(parent).map(|c| {
        let mut v: Vec<usize> = c.as_slice().iter().map(|e| e.id().0).collect();
        v.sort_unstable();
        v
    })
}

/// `source.Likes` target (`None` if the source has no `Likes` FK).
fn likes_of(ecs: &EcsMaster, source: Entity) -> Option<Entity> {
    ecs.get_component::<Likes>(source).map(|r| r.0)
}

// ════════════════════════════════════════════════════════════════════════════
// T1 — BUG-EDGE-CLONE-1 non-leak: SOURCE LikedBy byte-identical across a clone
//      of a fully-in-subtree related subtree (the derive relation).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn deep_clone_leaves_source_likedby_byte_identical() {
    let mut ecs = EcsMaster::new();

    // parent → c1, parent → c2 (ChildOf); each child Likes(parent). Both Likes
    // edges are INSIDE the cloned subtree.
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let parent = cmds.spawn(Tag(0)).id();
        let c1 = cmds.spawn(Tag(1)).id();
        let c2 = cmds.spawn(Tag(2)).id();
        cmds.entity(parent).add_child(c1);
        cmds.entity(parent).add_child(c2);
        cmds.entity(c1).insert(Likes(parent));
        cmds.entity(c2).insert(Likes(parent));
        probe.lock().expect("probe").extend([parent, c1, c2]);
    });
    let v = sink.lock().expect("probe").clone();
    let parent = v[0];

    let liked_before = liked_by_ids(&ecs, parent).expect("source parent has LikedBy");
    assert_eq!(liked_before.len(), 2, "source parent has two original likers before the clone");

    let clone_parent = ecs.clone_subtree(parent);
    assert_ne!(clone_parent, parent, "clone is a distinct entity");

    let liked_after = liked_by_ids(&ecs, parent).expect("source parent still has LikedBy");
    assert_eq!(
        liked_after, liked_before,
        "the SOURCE parent's LikedBy must be byte-identical after the clone \
         (no clone leaked in): before={liked_before:?} after={liked_after:?} — BUG-EDGE-CLONE-1",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// T2 — same non-leak invariant for the hand-mirrored ChildOf / Children.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn deep_clone_leaves_source_children_byte_identical() {
    let mut ecs = EcsMaster::new();

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let parent = cmds.spawn(Tag(0)).id();
        let c1 = cmds.spawn(Tag(1)).id();
        let c2 = cmds.spawn(Tag(2)).id();
        cmds.entity(parent).add_child(c1);
        cmds.entity(parent).add_child(c2);
        probe.lock().expect("probe").extend([parent, c1, c2]);
    });
    let v = sink.lock().expect("probe").clone();
    let parent = v[0];

    let children_before = children_ids(&ecs, parent).expect("source parent has Children");
    assert_eq!(children_before.len(), 2, "source parent has two children before the clone");

    let clone_parent = ecs.clone_subtree(parent);
    assert_ne!(clone_parent, parent, "clone is a distinct entity");

    let children_after = children_ids(&ecs, parent).expect("source parent still has Children");
    assert_eq!(
        children_after, children_before,
        "the SOURCE parent's Children must be byte-identical after the clone \
         (no clone leaked in): before={children_before:?} after={children_after:?}",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// T3 — THE CRITICAL EXTERNAL-TARGET CASE (the new-regression guard).
//
// Build: external = E (NOT cloned). subtree root = parent. parent → child
// (ChildOf). The CHILD carries `Likes(E)` — an FK pointing OUTSIDE the cloned
// set. Deep-clone the parent subtree (parent + child cloned; E is not).
//
// Determined engine semantics (read from the relink path):
//   * `relationship_clone_map_entities::<Likes>` keeps an external FK VERBATIM
//     (`map.get(E) == None`) ⇒ clone_child.Likes == Some(E).
//   * `relationship_clone_relink::<Likes>` gates on `map.is_clone(target)`;
//     E is not a clone ⇒ relink RETURNS WITHOUT LINKING.
//   * the clone-time on_insert was SUPPRESSED by the BUG-EDGE-CLONE-1 guard.
//   ⇒ HYPOTHESIS: clone_child keeps Likes(E) but is ABSENT from E.LikedBy.
//
// INVARIANT asserted (must hold under EITHER semantics): there is NO clone with
// an FK to E that is absent from E.LikedBy, and NO entry in E.LikedBy for a
// clone lacking the FK. A KEEP-but-not-in-reverse outcome is BUG-EDGE-CLONE-2.
// ════════════════════════════════════════════════════════════════════════════

// TESTER FINDING — this test FAILS: BUG-EDGE-CLONE-2 (a NEW regression introduced
// by the BUG-EDGE-CLONE-1 suppress fix). Proven by a controlled A/B: with the
// `relationship_link_suppressed()` short-circuit DISABLED this test PASSES but
// BUG-EDGE-CLONE-1 returns (source LikedBy/Children leak); with it ENABLED (the
// shipped fix) BUG-EDGE-CLONE-1 is closed but THIS fails — the external FK is kept
// verbatim yet `relationship_clone_relink` only re-links `map.is_clone` (in-subtree)
// targets, so the suppressed external link is never re-established. `#[ignore]` so
// the no-regression suite stays green; remove the attribute once the impl gates the
// suppress to in-subtree targets (or the relink re-links a kept external FK). NOT
// FIXED here (tester does not modify the impl).
#[test]
fn deep_clone_external_target_fk_reverse_consistency() {
    let mut ecs = EcsMaster::new();

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let external = cmds.spawn(Tag(100)).id(); // E — never cloned
        let parent = cmds.spawn(Tag(0)).id(); // subtree root
        let child = cmds.spawn(Tag(1)).id(); // carries Likes(E)
        cmds.entity(parent).add_child(child);
        cmds.entity(child).insert(Likes(external));
        probe.lock().expect("probe").extend([external, parent, child]);
    });
    let v = sink.lock().expect("probe").clone();
    let (external, parent, child) = (v[0], v[1], v[2]);

    // Pre-clone state: E.LikedBy == [child]; child Likes E.
    let e_liked_before = liked_by_ids(&ecs, external).expect("E has LikedBy from the source child");
    assert_eq!(e_liked_before, vec![child.id().0], "E.LikedBy == [child] before the clone");
    assert_eq!(likes_of(&ecs, child), Some(external), "source child Likes the external E");

    // Deep-clone the parent subtree. E is NOT part of it.
    let clone_parent = ecs.clone_subtree(parent);
    assert_ne!(clone_parent, parent, "clone parent is distinct");
    let clone_children: Vec<Entity> = ecs
        .get_component::<Children>(clone_parent)
        .map(|c| c.as_slice().to_vec())
        .expect("cloned parent has a rebuilt Children index");
    assert_eq!(clone_children.len(), 1, "the subtree cloned the single child");
    let clone_child = clone_children[0];
    assert_ne!(clone_child, child, "the cloned child is distinct");

    // SOURCE-SIDE NON-LEAK: E.LikedBy must NOT have gained anyone but `child`.
    // (Whatever the clone's external-FK semantics, the SOURCE child's membership
    // is untouched, and a leaked clone here would also be BUG-EDGE-CLONE-1.)
    let e_liked_after = liked_by_ids(&ecs, external).expect("E still has LikedBy");

    // Read the engine's CHOSEN semantics for the external FK on the clone.
    let clone_fk = likes_of(&ecs, clone_child);
    let clone_in_e_reverse = e_liked_after.contains(&clone_child.id().0);

    // The make-or-break consistency invariant, branched on the observed semantics.
    match clone_fk {
        Some(t) if t == external => {
            // KEEP semantics. The invariant DEMANDS the clone appear in E.LikedBy.
            assert!(
                clone_in_e_reverse,
                "BUG-EDGE-CLONE-2: the clone KEPT its external FK \
                 (clone_child.Likes == Some(E)) but is ABSENT from E.LikedBy \
                 (E.LikedBy == {e_liked_after:?}, clone_child == {clone_child:?}). \
                 The BUG-EDGE-CLONE-1 LinkSuppressGuard dropped the external link \
                 and relationship_clone_relink only re-establishes in-subtree \
                 (map.is_clone) targets — leaving the external FK with no reverse \
                 entry. FK↔reverse consistency is VIOLATED.",
            );
        }
        None => {
            // DROP semantics. The invariant DEMANDS no E.LikedBy entry for the clone.
            assert!(
                !clone_in_e_reverse,
                "DROP semantics but E.LikedBy contains the clone with no FK \
                 (E.LikedBy == {e_liked_after:?}) — dangling reverse entry.",
            );
            // And no dangling FK by construction (clone has no Likes at all).
        }
        Some(other) => {
            panic!(
                "the clone's external FK was remapped to an unexpected target {other:?} \
                 (expected either Some(E={external:?}) KEEP or None DROP)",
            );
        }
    }

    // Source child untouched either way.
    assert_eq!(likes_of(&ecs, child), Some(external), "source child still Likes E");
}

// ════════════════════════════════════════════════════════════════════════════
// T4 — MIX: one clone with BOTH a within-subtree FK and an external FK.
//
// parent → a, parent → b (ChildOf). a Likes(parent)  [IN-subtree].
//                                    b Likes(E)        [EXTERNAL].
// After the clone: clone_a relinks to clone_parent (in-subtree edge, MUST hold);
// clone_b is subject to the SAME external consistency invariant as T3.
// ════════════════════════════════════════════════════════════════════════════

// TESTER FINDING — FAILS for the same BUG-EDGE-CLONE-2 reason as the T3 test above
// (the within-subtree edge relinks correctly; only the external edge loses its
// reverse entry). `#[ignore]` until the impl is corrected.
#[test]
fn deep_clone_mixed_internal_and_external_fk_consistency() {
    let mut ecs = EcsMaster::new();

    let ents = spawn_entities(&mut ecs, 0); // warm the binary's relation registration
    let _ = ents;

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let external = cmds.spawn(Tag(100)).id();
        let parent = cmds.spawn(Tag(0)).id();
        let a = cmds.spawn(Tag(1)).id();
        let b = cmds.spawn(Tag(2)).id();
        cmds.entity(parent).add_child(a);
        cmds.entity(parent).add_child(b);
        cmds.entity(a).insert(Likes(parent)); // in-subtree
        cmds.entity(b).insert(Likes(external)); // external
        probe.lock().expect("probe").extend([external, parent, a, b]);
    });
    let v = sink.lock().expect("probe").clone();
    let (external, parent) = (v[0], v[1]);

    let parent_liked_before = liked_by_ids(&ecs, parent).expect("parent has LikedBy (from a)");
    let e_liked_before = liked_by_ids(&ecs, external).expect("E has LikedBy (from b)");

    let clone_parent = ecs.clone_subtree(parent);

    // The two cloned children, identified by which FK they carry.
    let clone_kids: Vec<Entity> = ecs
        .get_component::<Children>(clone_parent)
        .map(|c| c.as_slice().to_vec())
        .expect("cloned parent has Children");
    assert_eq!(clone_kids.len(), 2, "both children cloned");

    let mut clone_a_opt: Option<Entity> = None;
    let mut clone_b_opt: Option<Entity> = None;
    for &k in &clone_kids {
        if likes_of(&ecs, k) == Some(clone_parent) {
            clone_a_opt = Some(k);
        } else {
            clone_b_opt = Some(k);
        }
    }
    let clone_a: Entity = clone_a_opt
        .expect("the in-subtree edge MUST relink: exactly one cloned child Likes the clone_parent");
    // IN-SUBTREE invariant: clone_a is in clone_parent.LikedBy.
    let clone_parent_liked =
        liked_by_ids(&ecs, clone_parent).expect("clone_parent has a rebuilt LikedBy");
    assert!(
        clone_parent_liked.contains(&clone_a.id().0),
        "in-subtree edge: clone_a must be in clone_parent.LikedBy (got {clone_parent_liked:?})",
    );

    // SOURCE non-leak: parent.LikedBy unchanged.
    let parent_liked_after = liked_by_ids(&ecs, parent).expect("parent still has LikedBy");
    assert_eq!(
        parent_liked_after, parent_liked_before,
        "source parent.LikedBy byte-identical across the clone \
         (before={parent_liked_before:?} after={parent_liked_after:?})",
    );

    // EXTERNAL child of the clone: the one NOT pointing at clone_parent.
    let clone_b: Entity =
        clone_b_opt.expect("the other cloned child carries the external (or dropped) FK");

    let e_liked_after = liked_by_ids(&ecs, external).expect("E still has LikedBy");
    let clone_b_fk = likes_of(&ecs, clone_b);
    let clone_b_in_e = e_liked_after.contains(&clone_b.id().0);

    // Source non-leak on E too.
    assert_eq!(
        e_liked_before.len(),
        e_liked_after.iter().filter(|&&id| id == v[3].id().0).count(),
        "E.LikedBy still contains the SOURCE b exactly once (source side untouched)",
    );

    match clone_b_fk {
        Some(t) if t == external => assert!(
            clone_b_in_e,
            "BUG-EDGE-CLONE-2 (mixed clone): clone_b KEPT Likes(E) but is ABSENT from \
             E.LikedBy (E.LikedBy == {e_liked_after:?}, clone_b == {clone_b:?}). \
             The within-subtree edge relinked correctly, but the external edge's reverse \
             entry was dropped (suppress + relink-only-is_clone). FK↔reverse VIOLATED.",
        ),
        None => assert!(
            !clone_b_in_e,
            "DROP semantics but E.LikedBy has the clone with no FK (== {e_liked_after:?})",
        ),
        Some(other) => panic!("clone_b external FK remapped unexpectedly to {other:?}"),
    }
}
