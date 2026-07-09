//! Prefab `instantiate` × GENERIC relation relink suite — the fix under test
//! routes prefab instantiate through `remap_relink_generic_relations` (the ONE
//! deep-clone relink body, Principle 0) via a per-node `src_entity → instance`
//! [`EntityCloneMap`]. Before the fix, prefab instantiate remapped ONLY `ChildOf`,
//! so a captured node carrying a NON-`ChildOf` relation FK (`Likes(Y)`) produced a
//! ONE-DIRECTIONAL half-edge: the instance kept its `Likes` FK but never gained the
//! target's reverse-index (`LikedBy`) entry.
//!
//! This is the prefab analogue of `relations_deep_clone_external_target.rs` (T3/T4)
//! and `relations_exclusive_clone.rs` (D-(i)/(ii)/(iii)), but exercised through the
//! S7 clone-based `capture_prefab` / `instantiate` path instead of `clone_subtree`.
//!
//! # Relations under test (derive-built, NOT `ChildOf`-special)
//!
//! Using `#[derive(Relationship)]` / `#[derive(RelationshipTarget)]` proves the fix
//! is GENERIC (the in-crate `ChildOf`/`Children` are a hand-mirror; these derives
//! are the SOLE behavioral gate on the generic relation machinery, per
//! `relations_derive.rs`).
//!
//! * `Likes` / `LikedBy(Vec)` — a one-to-many (`Vec`) reverse collection.
//! * `Likes1` / `LikedBy1(Exclusive)` — a 1:1 (`Exclusive`) reverse slot, to
//!   exercise the C3 eviction-suppress / DETACH path under the prefab's
//!   `EvictionSuppressGuard` window.
//!
//! The `ChildOf` tree is the capture scaffold (capture walks the `ChildOf`
//! subtree): a relation target is "in-subtree" iff it is `ChildOf`-reachable from
//! the captured root, so the in-subtree cases wire the target in as a child.
//!
//! # Harness
//!
//! Mirrors `relations_derive.rs` / `miri_prefab_s7.rs`: freshly-spawned `Entity`
//! handles are smuggled out of the (`Send + Sync`) system closure through an
//! `Arc<Mutex<Vec<Entity>>>` probe, read back AFTER the apply window (the deferred
//! drain is what makes the reverse index consistent), then `capture_prefab` +
//! `instantiate` run on the direct `&mut EcsMaster` API.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::trigger::TriggerContext;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::relationship::{
    Exclusive, RelationshipSourceCollection, RelationshipTarget,
};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// Relation definitions (derive-built ⇒ proves the relink is GENERIC, not
// ChildOf-special).
// ════════════════════════════════════════════════════════════════════════════

/// One-to-many relation source. `Clone, Copy` ⇒ the `Component` derive classifies
/// the FK `CloneViaFn` (remap-eligible), exactly as the in-crate `ChildOf`.
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

/// One-to-many reverse index (`Vec` collection).
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

/// 1:1 relation source.
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy1)]
struct Likes1(pub Entity);

/// 1:1 reverse slot (`Exclusive`).
#[derive(Component, RelationshipTarget)]
#[relationship_target(source = Likes1, retain_empty)]
struct LikedBy1(Exclusive);

// `Exclusive` lacks `Default`, so the documented `#[derive(Default)]` on a 1:1
// target does not compile — hand-write it (mirrors `relations_exclusive_clone.rs`).
impl Default for LikedBy1 {
    fn default() -> Self {
        Self(Exclusive::with_capacity(0))
    }
}

/// A plain marker so a freshly-spawned entity has a concrete archetype before a
/// relationship FK migrates it. Tag only.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

// ════════════════════════════════════════════════════════════════════════════
// Harness helpers (mirror relations_derive.rs / relations_exclusive_clone.rs)
// ════════════════════════════════════════════════════════════════════════════

/// `target.LikedBy` (Vec) sources as a sorted `Vec<usize>` of ids (`None` if no
/// `LikedBy` component at all).
fn liked_by_ids(ecs: &EcsMaster, target: Entity) -> Option<Vec<usize>> {
    ecs.get_component::<LikedBy>(target).map(|c| {
        let mut v: Vec<usize> =
            RelationshipSourceCollection::iter(c.collection()).map(|e| e.id().0).collect();
        v.sort_unstable();
        v
    })
}

/// `source.Likes` (Vec) FK target (`None` if no `Likes` FK).
fn likes_of(ecs: &EcsMaster, source: Entity) -> Option<Entity> {
    ecs.get_component::<Likes>(source).map(|r| r.0)
}

/// `target.LikedBy1` (Exclusive) 1:1 slot occupant.
fn liked_by1(ecs: &EcsMaster, target: Entity) -> Option<Entity> {
    ecs.get_component::<LikedBy1>(target)
        .and_then(|c| RelationshipSourceCollection::get(c.collection(), 0))
}

/// `source.Likes1` 1:1 FK target.
fn likes1_of(ecs: &EcsMaster, source: Entity) -> Option<Entity> {
    ecs.get_component::<Likes1>(source).map(|r| r.0)
}

/// The instance children of `inst_root` (rebuilt `Children` reverse index).
fn inst_children(ecs: &EcsMaster, inst_root: Entity) -> Vec<Entity> {
    ecs.get_component::<Children>(inst_root)
        .map(|c| c.as_slice().to_vec())
        .unwrap_or_default()
}

// ════════════════════════════════════════════════════════════════════════════
// CASE 1 — IN-SUBTREE generic relation (Vec): node X Likes(Y), Y ALSO captured.
//
// Topology: root --ChildOf-- x, root --ChildOf-- y.  x Likes(y) [in-subtree].
//   Capture {root, x, y}. Each instance's x_i Likes y_i (remapped to the cloned
//   target), and y_i.LikedBy CONTAINS x_i (the reverse relink — the fix).
//   Instantiate TWICE → two independent, internally-consistent instances; no
//   cross-instance leakage.
// ════════════════════════════════════════════════════════════════════════════

/// Resolves an instance's (root, x_i, y_i) from a captured `root --x, --y; x Likes y`
/// prefab, identifying x_i as the instance child carrying the `Likes` FK.
fn resolve_instance_xy(ecs: &EcsMaster, inst_root: Entity) -> (Entity, Entity) {
    let kids = inst_children(ecs, inst_root);
    assert_eq!(kids.len(), 2, "instance root has two children (x_i, y_i)");
    let mut x_i = None;
    let mut y_i = None;
    for &k in &kids {
        if likes_of(ecs, k).is_some() {
            x_i = Some(k);
        } else {
            y_i = Some(k);
        }
    }
    (
        x_i.expect("exactly one instance child carries the Likes FK (x_i)"),
        y_i.expect("the other instance child is the Likes target (y_i)"),
    )
}

#[test]
fn instantiate_in_subtree_vec_relation_relinks_reverse_index() {
    let mut ecs = EcsMaster::new();

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let root = cmds.spawn(Tag(0)).id();
        let x = cmds.spawn(Tag(1)).id();
        let y = cmds.spawn(Tag(2)).id();
        cmds.entity(root).add_child(x);
        cmds.entity(root).add_child(y);
        cmds.entity(x).insert(Likes(y)); // in-subtree FK: y is captured too
        probe.lock().expect("probe").extend([root, x, y]);
    });
    let v = sink.lock().expect("probe").clone();
    let (src_root, src_x, src_y) = (v[0], v[1], v[2]);

    // Source pre-state: y.LikedBy == [x].
    assert_eq!(
        liked_by_ids(&ecs, src_y),
        Some(vec![src_x.id().0]),
        "source y.LikedBy == [x] before capture",
    );

    let prefab = ecs.capture_prefab(src_root);
    assert_eq!(prefab.node_count(), 3, "captured root + x + y");

    // ── Instance A ──────────────────────────────────────────────────────────
    let inst_a_root = ecs.instantiate(&prefab);
    let (ax, ay) = resolve_instance_xy(&ecs, inst_a_root);
    assert_ne!(ax, src_x, "instance A x is fresh");
    assert_ne!(ay, src_y, "instance A y is fresh");

    // Forward FK remapped to the cloned in-subtree target.
    assert_eq!(
        likes_of(&ecs, ax),
        Some(ay),
        "instance A: ax.Likes remapped to the cloned in-subtree target ay",
    );
    // REVERSE relink — THE FIX: ay.LikedBy CONTAINS ax (previously a half-edge).
    assert_eq!(
        liked_by_ids(&ecs, ay),
        Some(vec![ax.id().0]),
        "instance A: ay.LikedBy CONTAINS ax (the prefab generic-relink fix — FK↔reverse \
         consistent; before the fix ay.LikedBy would be empty, a one-directional half-edge)",
    );

    // ── Instance B (instantiate TWICE) ───────────────────────────────────────
    let inst_b_root = ecs.instantiate(&prefab);
    assert_ne!(inst_b_root, inst_a_root, "the two instances are independent roots");
    let (bx, by) = resolve_instance_xy(&ecs, inst_b_root);

    assert_eq!(likes_of(&ecs, bx), Some(by), "instance B: bx.Likes == by");
    assert_eq!(
        liked_by_ids(&ecs, by),
        Some(vec![bx.id().0]),
        "instance B: by.LikedBy CONTAINS bx (independent, internally consistent)",
    );

    // NO CROSS-INSTANCE LEAKAGE: A's reverse holds only ax; B's only bx.
    assert_eq!(
        liked_by_ids(&ecs, ay),
        Some(vec![ax.id().0]),
        "instance A reverse untouched by instantiating B (ay.LikedBy == [ax], no bx leak)",
    );
    assert!(!by.eq(&ay) && !bx.eq(&ax), "instance B entities distinct from A's");

    // SOURCE untouched by either instantiate.
    assert_eq!(
        liked_by_ids(&ecs, src_y),
        Some(vec![src_x.id().0]),
        "source y.LikedBy still == [x] after two instantiates (no source-side leak)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// CASE 2 — EXTERNAL generic relation (Vec): node X Likes(E), E OUTSIDE the
// captured subtree (a live external entity present at instantiate).
//
// THE HALF-EDGE FIX: the instance keeps Likes(E) (E absent from the map ⇒ verbatim)
// AND E.LikedBy gains the instance (the relink establishes the EXTERNAL reverse
// entry). This is the prefab analogue of the deep-clone T3 case that was previously
// `#[ignore]`'d as BUG-EDGE-CLONE-2 — the relink-through-the-shared-body fixes it.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn instantiate_external_vec_relation_links_external_reverse_entry() {
    let mut ecs = EcsMaster::new();

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let external = cmds.spawn(Tag(100)).id(); // E — never captured
        let root = cmds.spawn(Tag(0)).id();
        let child = cmds.spawn(Tag(1)).id();
        cmds.entity(root).add_child(child);
        cmds.entity(child).insert(Likes(external)); // external FK
        probe.lock().expect("probe").extend([external, root, child]);
    });
    let v = sink.lock().expect("probe").clone();
    let (external, src_root, src_child) = (v[0], v[1], v[2]);

    // Source pre-state: E.LikedBy == [child].
    assert_eq!(
        liked_by_ids(&ecs, external),
        Some(vec![src_child.id().0]),
        "E.LikedBy == [child] before capture",
    );

    // Capture only {root, child}; E is NOT in the subtree.
    let prefab = ecs.capture_prefab(src_root);
    assert_eq!(prefab.node_count(), 2, "captured root + child only (E external)");

    let inst_root = ecs.instantiate(&prefab);
    let kids = inst_children(&ecs, inst_root);
    assert_eq!(kids.len(), 1, "the subtree cloned the single child");
    let inst_child = kids[0];
    assert_ne!(inst_child, src_child, "the instance child is fresh");

    // The instance KEEPS Likes(E) (external target stays verbatim).
    assert_eq!(
        likes_of(&ecs, inst_child),
        Some(external),
        "instance child KEEPS Likes(E) (external target absent from the map ⇒ verbatim)",
    );

    // THE FIX: E.LikedBy now CONTAINS the instance child (external reverse entry
    // established — the half-edge is closed). Source `child` stays in there too.
    let e_liked = liked_by_ids(&ecs, external).expect("E still has LikedBy");
    assert!(
        e_liked.contains(&inst_child.id().0),
        "E.LikedBy CONTAINS the instance child (the external reverse relink — the prefab \
         half-edge fix; before the fix the instance kept Likes(E) but was ABSENT here). \
         E.LikedBy == {e_liked:?}, instance child == {inst_child:?}",
    );
    assert!(
        e_liked.contains(&src_child.id().0),
        "E.LikedBy still CONTAINS the SOURCE child (source side untouched): {e_liked:?}",
    );
    assert_eq!(e_liked.len(), 2, "exactly source child + instance child in E.LikedBy");

    // FK↔reverse consistency: every E.LikedBy member holds a matching Likes(E) FK.
    for &id in &e_liked {
        let member = if id == src_child.id().0 { src_child } else { inst_child };
        assert_eq!(
            likes_of(&ecs, member),
            Some(external),
            "FK↔reverse consistency: each E.LikedBy member holds Likes(E)",
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// CASE 3 — EXTERNAL OCCUPIED Exclusive (1:1): node X Likes1(T), T external and
// ALREADY held by a real source A.
//
// DETACH semantic (v1.1, under the now-load-bearing EvictionSuppressGuard): the
// instance has NO FK (dropped), the external incumbent A is UNTOUCHED (A.FK == T,
// T.slot == A), NO spurious OnUnlink. FK↔reverse consistent globally.
//
// (In production a 1:1 slot has a SINGLE occupant which IS the only Likes1(T)
// holder, so the genuine external incumbent is the captured `child` itself — the
// same construction as relations_exclusive_clone.rs D-(ii).)
// ════════════════════════════════════════════════════════════════════════════

static C3_LINK: AtomicUsize = AtomicUsize::new(0);
static C3_UNLINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn c3_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    C3_LINK.fetch_add(1, SEQ);
}
unsafe fn c3_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    C3_UNLINK.fetch_add(1, SEQ);
}

#[test]
fn instantiate_external_occupied_exclusive_detaches_incumbent_untouched() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes1>(c3_on_link);
    ecs.observe_on_unlink::<Likes1>(c3_on_unlink);

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let t = cmds.spawn(Tag(100)).id(); // external 1:1 target — never captured
        let root = cmds.spawn(Tag(0)).id();
        let child = cmds.spawn(Tag(1)).id(); // the external incumbent A of T's slot
        cmds.entity(root).add_child(child);
        cmds.entity(child).insert(Likes1(t)); // child Likes1(T) ⇒ T.LikedBy1 == child
        probe.lock().expect("probe").extend([t, root, child]);
    });
    let v = sink.lock().expect("probe").clone();
    let (t, src_root, incumbent) = (v[0], v[1], v[2]);

    // Pre-state: T's 1:1 slot OCCUPIED by the incumbent (the captured child).
    assert_eq!(liked_by1(&ecs, t), Some(incumbent), "external incumbent: T.LikedBy1 == child");
    assert_eq!(likes1_of(&ecs, incumbent), Some(t), "incumbent child.Likes1 == T");

    let prefab = ecs.capture_prefab(src_root);
    assert_eq!(prefab.node_count(), 2, "captured root + child (T external)");

    C3_LINK.store(0, SEQ);
    C3_UNLINK.store(0, SEQ);

    let inst_root = ecs.instantiate(&prefab);
    let kids = inst_children(&ecs, inst_root);
    assert_eq!(kids.len(), 1, "cloned the single child");
    let inst_child = kids[0];
    assert_ne!(inst_child, incumbent, "the instance child is fresh");

    // DETACH: the instance's FK toward the occupied external T is DROPPED.
    assert_eq!(
        likes1_of(&ecs, inst_child),
        None,
        "DETACH (C3): the instance's Likes1(T) FK was dropped under the eviction-suppress \
         guard — the instance is unrelated to the occupied external 1:1 target T (no \
         eviction of the incumbent)",
    );

    // External incumbent + the reverse slot byte-for-byte UNTOUCHED (no theft).
    assert_eq!(
        liked_by1(&ecs, t),
        Some(incumbent),
        "external incumbent untouched: T.LikedBy1 still == child (no theft)",
    );
    assert_eq!(likes1_of(&ecs, incumbent), Some(t), "incumbent child.Likes1 still == T");

    // NO spurious OnUnlink (the detach is NOT an eviction — the incumbent is never
    // unlinked). No genuine OnLink either (the instance detached, established nothing).
    assert_eq!(
        C3_UNLINK.load(SEQ),
        0,
        "DETACH fires NO OnUnlink — the external incumbent is never evicted",
    );
    assert_eq!(
        C3_LINK.load(SEQ),
        0,
        "DETACH establishes no edge ⇒ no OnLink for the dropped instance FK",
    );

    // FK↔reverse GLOBAL CONSISTENCY: the instance carries no dangling FK; T's slot
    // occupant holds the matching forward FK.
    assert_ne!(likes1_of(&ecs, inst_child), Some(t), "instance carries NO dangling Likes1(T)");
    let occupant = liked_by1(&ecs, t).expect("T slot occupied");
    assert_eq!(
        likes1_of(&ecs, occupant),
        Some(t),
        "FK↔reverse consistency: T's slot occupant holds the matching Likes1(T) FK",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// CASE 4 — IN-SUBTREE Exclusive (1:1): node X Likes1(Y), Y ALSO captured ⇒ the
// instance relinks to the CLONED target (empty fresh slot, no eviction), consistent.
//
// Topology: root --ChildOf-- x, root --ChildOf-- y.  x Likes1(y) [1:1, in-subtree].
// ════════════════════════════════════════════════════════════════════════════

static C4_LINK: AtomicUsize = AtomicUsize::new(0);
static C4_UNLINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn c4_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    C4_LINK.fetch_add(1, SEQ);
}
unsafe fn c4_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    C4_UNLINK.fetch_add(1, SEQ);
}

#[test]
fn instantiate_in_subtree_exclusive_relinks_to_clone_target_no_eviction() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes1>(c4_on_link);
    ecs.observe_on_unlink::<Likes1>(c4_on_unlink);

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let root = cmds.spawn(Tag(0)).id();
        let x = cmds.spawn(Tag(1)).id();
        let y = cmds.spawn(Tag(2)).id();
        cmds.entity(root).add_child(x);
        cmds.entity(root).add_child(y);
        cmds.entity(x).insert(Likes1(y)); // 1:1, both in-subtree
        probe.lock().expect("probe").extend([root, x, y]);
    });
    let v = sink.lock().expect("probe").clone();
    let (src_root, src_x, src_y) = (v[0], v[1], v[2]);
    assert_eq!(liked_by1(&ecs, src_y), Some(src_x), "source: y.LikedBy1 == x");

    let prefab = ecs.capture_prefab(src_root);
    assert_eq!(prefab.node_count(), 3, "captured root + x + y");

    C4_LINK.store(0, SEQ);
    C4_UNLINK.store(0, SEQ);

    let inst_root = ecs.instantiate(&prefab);
    let kids = inst_children(&ecs, inst_root);
    assert_eq!(kids.len(), 2, "both children cloned");

    // Identify x_i (carries Likes1) and y_i (its 1:1 target).
    let mut x_i = None;
    let mut y_i = None;
    for &k in &kids {
        if likes1_of(&ecs, k).is_some() {
            x_i = Some(k);
        } else {
            y_i = Some(k);
        }
    }
    let x_i = x_i.expect("exactly one instance child carries the Likes1 FK");
    let y_i = y_i.expect("the other instance child is the 1:1 target");

    // Forward FK remapped to the cloned in-subtree target; y_i's FRESH (empty) slot
    // took x_i via the plain-add arm (NOT eviction).
    assert_eq!(
        likes1_of(&ecs, x_i),
        Some(y_i),
        "instance x_i.Likes1 remapped to the cloned in-subtree target y_i",
    );
    assert_eq!(
        liked_by1(&ecs, y_i),
        Some(x_i),
        "instance y_i.LikedBy1 == x_i (1:1 slot re-established on the fresh empty slot)",
    );

    // No eviction (y_i's slot was empty) ⇒ NO OnUnlink; exactly one new OnLink.
    assert_eq!(C4_UNLINK.load(SEQ), 0, "empty clone target slot fires NO OnUnlink");
    assert_eq!(C4_LINK.load(SEQ), 1, "exactly one OnLink for the single re-established edge");

    // SOURCE untouched.
    assert_eq!(liked_by1(&ecs, src_y), Some(src_x), "source y.LikedBy1 still == x");
    assert_eq!(likes1_of(&ecs, src_x), Some(src_y), "source x.Likes1 still == y");
}

// ════════════════════════════════════════════════════════════════════════════
// CASE 5 — MULTI-SOURCE into one in-subtree target (Vec): TWO captured nodes both
// Likes(Y), Y in-subtree ⇒ the instance y_i.LikedBy contains BOTH instance sources
// (collection-add ordering correct, no half-edge for either).
//
// Topology: root --ChildOf-- a, --ChildOf-- b, --ChildOf-- y.
//   a Likes(y), b Likes(y). Capture {root, a, b, y}.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn instantiate_multi_source_in_subtree_vec_collects_both() {
    let mut ecs = EcsMaster::new();

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let root = cmds.spawn(Tag(0)).id();
        let a = cmds.spawn(Tag(1)).id();
        let b = cmds.spawn(Tag(2)).id();
        let y = cmds.spawn(Tag(3)).id();
        cmds.entity(root).add_child(a);
        cmds.entity(root).add_child(b);
        cmds.entity(root).add_child(y);
        cmds.entity(a).insert(Likes(y));
        cmds.entity(b).insert(Likes(y));
        probe.lock().expect("probe").extend([root, a, b, y]);
    });
    let v = sink.lock().expect("probe").clone();
    let (src_root, src_y) = (v[0], v[3]);

    // Source pre-state: y.LikedBy == {a, b}.
    let src_liked = liked_by_ids(&ecs, src_y).expect("source y.LikedBy");
    assert_eq!(src_liked.len(), 2, "source y.LikedBy has both a and b");

    let prefab = ecs.capture_prefab(src_root);
    assert_eq!(prefab.node_count(), 4, "captured root + a + b + y");

    let inst_root = ecs.instantiate(&prefab);
    let kids = inst_children(&ecs, inst_root);
    assert_eq!(kids.len(), 3, "three children cloned (a, b, y)");

    // y_i is the child without a Likes FK; the two sources carry Likes(y_i).
    let mut y_i = None;
    let mut sources: Vec<Entity> = Vec::new();
    for &k in &kids {
        match likes_of(&ecs, k) {
            Some(_) => sources.push(k),
            None => y_i = Some(k),
        }
    }
    let y_i = y_i.expect("exactly one instance child has no Likes FK (the target y_i)");
    assert_eq!(sources.len(), 2, "two instance sources carry the Likes FK");

    // Both instance sources point at y_i (in-subtree remap).
    for &s in &sources {
        assert_eq!(
            likes_of(&ecs, s),
            Some(y_i),
            "each instance source remapped Likes to the cloned in-subtree target y_i",
        );
    }

    // REVERSE relink: y_i.LikedBy contains BOTH instance sources (collection-add).
    let mut want: Vec<usize> = sources.iter().map(|e| e.id().0).collect();
    want.sort_unstable();
    assert_eq!(
        liked_by_ids(&ecs, y_i),
        Some(want.clone()),
        "instance y_i.LikedBy contains BOTH instance sources (multi-source collection-add; \
         no half-edge for either source)",
    );

    // SOURCE untouched.
    assert_eq!(
        liked_by_ids(&ecs, src_y).map(|v| v.len()),
        Some(2),
        "source y.LikedBy still has both source likers (no leak)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// CASE 6 — ChildOf NON-REGRESSION: the superset src_entity→instance map did not
// break the ChildOf remap. The instance's Children/ChildOf must match the template.
//
// Topology: root --ChildOf-- a; a --ChildOf-- grandchild. Plus a generic Likes
// edge present (root Likes a, in-subtree) to prove the ChildOf remap still holds
// when the new generic-relink pass ALSO runs over the same nodes.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn instantiate_childof_hierarchy_non_regression_with_generic_relation() {
    let mut ecs = EcsMaster::new();

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let root = cmds.spawn(Tag(0)).id();
        let a = cmds.spawn(Tag(1)).id();
        let grandchild = cmds.spawn(Tag(2)).id();
        cmds.entity(root).add_child(a);
        cmds.entity(a).add_child(grandchild);
        cmds.entity(root).insert(Likes(a)); // generic edge over the same nodes
        probe.lock().expect("probe").extend([root, a, grandchild]);
    });
    let v = sink.lock().expect("probe").clone();
    let src_root = v[0];

    let prefab = ecs.capture_prefab(src_root);
    assert_eq!(prefab.node_count(), 3, "captured root + a + grandchild");

    let inst_root = ecs.instantiate(&prefab);

    // Instance root is detached (no ChildOf).
    assert!(
        ecs.get_component::<ChildOf>(inst_root).is_none(),
        "instance root is detached (no ChildOf)",
    );

    // ChildOf chain rebuilt: root → a → grandchild.
    let root_kids = inst_children(&ecs, inst_root);
    assert_eq!(root_kids.len(), 1, "instance root has exactly one child (a_i)");
    let a_i = root_kids[0];
    assert_eq!(
        ecs.get_component::<ChildOf>(a_i).map(|c| c.0),
        Some(inst_root),
        "a_i.ChildOf remapped to the fresh instance root",
    );

    let a_kids = inst_children(&ecs, a_i);
    assert_eq!(a_kids.len(), 1, "a_i has exactly one child (grandchild_i)");
    let gc_i = a_kids[0];
    assert_eq!(
        ecs.get_component::<ChildOf>(gc_i).map(|c| c.0),
        Some(a_i),
        "grandchild_i.ChildOf remapped to a_i (two-level hierarchy rebuilt correctly)",
    );
    assert!(
        ecs.get_component::<Children>(gc_i).is_none(),
        "grandchild_i is a leaf (no Children)",
    );

    // And the generic Likes edge ALSO relinked (the new pass coexists with ChildOf).
    assert_eq!(
        likes_of(&ecs, inst_root),
        Some(a_i),
        "instance root.Likes remapped to a_i (generic pass coexists with ChildOf remap)",
    );
    assert_eq!(
        liked_by_ids(&ecs, a_i),
        Some(vec![inst_root.id().0]),
        "a_i.LikedBy CONTAINS inst_root (generic reverse relink present alongside ChildOf)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// CASE 7 — combined fixture for Miri-TB (the relink + deferred-drain window):
// a single instantiate carrying a Vec in-subtree edge, a Vec EXTERNAL edge, AND an
// Exclusive external-OCCUPIED edge (the detach RemoveCommand under the eviction-
// suppress guard). Run under -Zmiri-tree-borrows to confirm TB-clean. As a plain
// test it ALSO asserts all three semantics hold simultaneously in one window.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn instantiate_combined_relink_window_miri_tb_fixture() {
    let mut ecs = EcsMaster::new();

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        // External entities (not captured).
        let e_vec = cmds.spawn(Tag(100)).id(); // external Vec target
        let t_excl = cmds.spawn(Tag(101)).id(); // external occupied 1:1 target

        // Subtree: root --ChildOf-- {x, y, sx, se}.
        let root = cmds.spawn(Tag(0)).id();
        let x = cmds.spawn(Tag(1)).id(); // x Likes(y) [in-subtree Vec]
        let y = cmds.spawn(Tag(2)).id();
        let sx = cmds.spawn(Tag(3)).id(); // sx Likes(e_vec) [external Vec]
        let se = cmds.spawn(Tag(4)).id(); // se Likes1(t_excl) [external occupied 1:1]
        let incumbent = cmds.spawn(Tag(5)).id(); // real 1:1 incumbent of t_excl (external)

        cmds.entity(root).add_child(x);
        cmds.entity(root).add_child(y);
        cmds.entity(root).add_child(sx);
        cmds.entity(root).add_child(se);
        cmds.entity(x).insert(Likes(y));
        cmds.entity(sx).insert(Likes(e_vec));
        cmds.entity(incumbent).insert(Likes1(t_excl)); // occupies t_excl's 1:1 slot
        cmds.entity(se).insert(Likes1(t_excl)); // se ALSO points at the occupied slot

        probe.lock().expect("probe").extend([e_vec, t_excl, root, incumbent]);
    });
    let v = sink.lock().expect("probe").clone();
    let (e_vec, t_excl, src_root, incumbent) = (v[0], v[1], v[2], v[3]);

    // NOTE on the 1:1 source-side construction: production 1:1 forbids two distinct
    // entities both holding Likes1(T) at once (the second insert would evict). Here
    // the LAST applied insert (`se`) wins the slot, so capture the world's actual
    // 1:1 occupant of t_excl as the genuine incumbent BEFORE instantiate.
    let pre_occupant = liked_by1(&ecs, t_excl).expect("t_excl 1:1 slot occupied pre-instantiate");
    assert!(
        pre_occupant == incumbent || pre_occupant == v[2] || ecs.has_entity(pre_occupant),
        "t_excl has a live 1:1 occupant before instantiate",
    );
    let pre_occupant_fk = likes1_of(&ecs, pre_occupant);

    let prefab = ecs.capture_prefab(src_root);
    // root + x + y + sx + se = 5 captured nodes (the incumbent is external).
    assert_eq!(prefab.node_count(), 5, "captured root + x + y + sx + se");

    let inst_root = ecs.instantiate(&prefab);
    let kids = inst_children(&ecs, inst_root);
    assert_eq!(kids.len(), 4, "four instance children (x_i, y_i, sx_i, se_i)");

    // Classify the four instance children by their FK shape.
    let mut x_i = None; // Likes(y_i), in-subtree
    let mut y_i = None; // no Likes FK (the in-subtree Vec target)
    let mut sx_i = None; // Likes(e_vec), external Vec
    let mut se_i = None; // Likes1 detached (no FK)
    for &k in &kids {
        if likes1_of(&ecs, k).is_some() {
            // shouldn't happen — the external 1:1 detaches; recorded for diagnostics
            se_i = Some(k);
        } else if let Some(t) = likes_of(&ecs, k) {
            if t == e_vec {
                sx_i = Some(k);
            } else {
                x_i = Some(k);
            }
        } else {
            // no Likes and no Likes1: either y_i (the Vec target) or se_i (detached 1:1).
            // y_i has the LikedBy reverse populated by x_i; se_i does not. Disambiguate
            // after x_i is known — provisionally stash both candidates.
            if y_i.is_none() {
                y_i = Some(k);
            } else {
                se_i = Some(k);
            }
        }
    }
    let x_i = x_i.expect("instance x_i carries the in-subtree Likes FK");
    let sx_i = sx_i.expect("instance sx_i carries the external Likes(e_vec) FK");

    // y_i is the in-subtree Vec target that x_i points at; whichever of the two
    // "no-FK" candidates x_i targets IS y_i, the other is se_i (detached).
    let x_target = likes_of(&ecs, x_i).expect("x_i has a Likes FK");
    let (y_i, se_i) = {
        let cand_a = y_i.expect("first no-FK candidate");
        match se_i {
            Some(cand_b) if x_target == cand_b => (cand_b, cand_a),
            _ => (cand_a, se_i.unwrap_or(cand_a)),
        }
    };

    // ── (a) in-subtree Vec edge ──────────────────────────────────────────────
    assert_eq!(likes_of(&ecs, x_i), Some(y_i), "x_i.Likes remapped to in-subtree y_i");
    assert_eq!(
        liked_by_ids(&ecs, y_i),
        Some(vec![x_i.id().0]),
        "y_i.LikedBy CONTAINS x_i (in-subtree reverse relink)",
    );

    // ── (b) external Vec edge ────────────────────────────────────────────────
    assert_eq!(likes_of(&ecs, sx_i), Some(e_vec), "sx_i KEEPS external Likes(e_vec)");
    let e_liked = liked_by_ids(&ecs, e_vec).expect("e_vec has LikedBy");
    assert!(
        e_liked.contains(&sx_i.id().0),
        "e_vec.LikedBy CONTAINS sx_i (external reverse relink): {e_liked:?}",
    );

    // ── (c) external occupied 1:1 edge — DETACH ──────────────────────────────
    assert_eq!(
        likes1_of(&ecs, se_i),
        None,
        "se_i.Likes1(t_excl) DETACHED (external occupied 1:1 ⇒ FK dropped under suppress)",
    );
    // The incumbent slot is untouched (no theft): the pre-instantiate occupant holds
    // the slot and the matching FK.
    let post_occupant = liked_by1(&ecs, t_excl).expect("t_excl slot still occupied");
    assert_eq!(post_occupant, pre_occupant, "t_excl 1:1 slot occupant UNCHANGED (no theft)");
    assert_eq!(
        likes1_of(&ecs, post_occupant),
        pre_occupant_fk,
        "FK↔reverse consistency: t_excl occupant still holds its matching Likes1 FK",
    );
    assert_ne!(se_i, post_occupant, "the detached instance is NOT the slot occupant");
}
