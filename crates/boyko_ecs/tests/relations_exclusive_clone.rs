//! Relations v1.1 — `Exclusive` 1:1 collection × DEEP CLONE suite (category D:
//! the C3 clone-time eviction-suppression / DETACH semantic).
//!
//! During a deep clone the 1:1 eviction is SUPPRESSED
//! ([`EvictionSuppressGuard`] held across the relink pass, `clone/deep.rs`):
//!
//! * (i) IN-SUBTREE 1:1 FK — both the cloned source AND its 1:1 target are inside
//!   the cloned subtree → the clone's edge re-establishes to the CLONED target (an
//!   empty slot, no eviction). NO OnUnlink, exactly one genuine new OnLink.
//! * (ii) OCCUPIED-EXTERNAL 1:1 FK (DETACH) — the cloned source's FK targets an
//!   OCCUPIED external 1:1 target `T` (held by a real source `A`). Under suppression
//!   the clone DETACHES: its own (dangling) FK is dropped, the external incumbent
//!   `A` + the reverse slot are byte-for-byte untouched. NO OnUnlink fires;
//!   FK↔reverse stays globally consistent (no dangling FK, no orphan reverse entry).
//! * (iii) reviewer M2 — an in-subtree clone whose remapped 1:1 target slot is
//!   ALREADY correct/empty (bypasses the detach arm): links correctly, no spurious
//!   fire.
//!
//! The 1:1 target is itself part of the cloned `ChildOf` subtree for the in-subtree
//! cases (so `map.is_clone(target)` holds and the FK is remapped to the clone).
//!
//! Harness mirrors `relations_deep_clone_external_target.rs` (`Arc<Mutex>` spawn
//! probe; a derive-built 1:1 relation pair `Likes1`/`LikedBy1(Exclusive)`, NOT
//! ChildOf-special; the `ChildOf` tree is only the clone-subtree scaffold).
//!
//! [`EvictionSuppressGuard`]: boyko_ecs internal (clone/deep.rs)

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::trigger::TriggerContext;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;
use boyko_ecs::ecs::core::relationship::{
    Exclusive, RelationshipSourceCollection, RelationshipTarget,
};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// A derive-built 1:1 relation (NOT ChildOf). `LikedBy1(Exclusive)` ⇒ 1:1 slot.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy1)]
struct Likes1(pub Entity);

#[derive(Component, RelationshipTarget)]
#[relationship_target(source = Likes1, retain_empty)]
struct LikedBy1(Exclusive);

// FINDING-1 WORKAROUND: `Exclusive` lacks `Default`, so the documented
// `#[derive(Default)]` on a 1:1 target does not compile — hand-write it.
impl Default for LikedBy1 {
    fn default() -> Self {
        Self(Exclusive::with_capacity(0))
    }
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

// ════════════════════════════════════════════════════════════════════════════
// Harness
// ════════════════════════════════════════════════════════════════════════════

/// `target.LikedBy1` 1:1 slot occupant.
fn liked_by1(ecs: &EcsMaster, target: Entity) -> Option<Entity> {
    ecs.get_component::<LikedBy1>(target)
        .and_then(|c| RelationshipSourceCollection::get(c.collection(), 0))
}

/// `source.Likes1` FK target.
fn likes1_of(ecs: &EcsMaster, source: Entity) -> Option<Entity> {
    ecs.get_component::<Likes1>(source).map(|r| r.0)
}

/// The cloned children of `clone_parent` (rebuilt `Children` reverse index).
fn clone_children(ecs: &EcsMaster, clone_parent: Entity) -> Vec<Entity> {
    ecs.get_component::<Children>(clone_parent)
        .map(|c| c.as_slice().to_vec())
        .unwrap_or_default()
}

// ════════════════════════════════════════════════════════════════════════════
// D-(i) — IN-SUBTREE 1:1 clone: the cloned source's 1:1 target is ALSO in the
//         subtree → the clone's edge re-establishes to the CLONED target (empty
//         slot, no eviction). NO OnUnlink; exactly one genuine new OnLink.
//
// Topology: parent --ChildOf-- child.  child Likes1(parent)  [1:1, in-subtree].
//   So parent.LikedBy1 == child (the 1:1 slot). Cloning the parent subtree clones
//   {parent, child}; the clone's Likes1 FK remaps to clone_parent (in-subtree),
//   and the relink establishes clone_parent.LikedBy1 == clone_child.
// ════════════════════════════════════════════════════════════════════════════

static DI_LINK: AtomicUsize = AtomicUsize::new(0);
static DI_UNLINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn di_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    DI_LINK.fetch_add(1, SEQ);
}
unsafe fn di_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    DI_UNLINK.fetch_add(1, SEQ);
}

#[test]
fn exclusive_clone_in_subtree_relinks_to_clone_target_no_eviction() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes1>(di_on_link);
    ecs.observe_on_unlink::<Likes1>(di_on_unlink);

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let parent = cmds.spawn(Tag(0)).id();
        let child = cmds.spawn(Tag(1)).id();
        cmds.entity(parent).add_child(child);
        cmds.entity(child).insert(Likes1(parent)); // 1:1, both in-subtree
        probe.lock().expect("probe").extend([parent, child]);
    });
    let v = sink.lock().expect("probe").clone();
    let (parent, child) = (v[0], v[1]);
    assert_eq!(liked_by1(&ecs, parent), Some(child), "source: parent.LikedBy1 == child");

    DI_LINK.store(0, SEQ);
    DI_UNLINK.store(0, SEQ);

    let clone_parent = ecs.clone_subtree(parent);
    assert_ne!(clone_parent, parent, "the clone parent is distinct");
    let kids = clone_children(&ecs, clone_parent);
    assert_eq!(kids.len(), 1, "the subtree cloned the single child");
    let clone_child = kids[0];
    assert_ne!(clone_child, child, "the cloned child is distinct");

    // The clone's 1:1 edge re-established to the CLONED target (empty slot).
    assert_eq!(
        likes1_of(&ecs, clone_child),
        Some(clone_parent),
        "clone_child.Likes1 remapped to the in-subtree clone_parent",
    );
    assert_eq!(
        liked_by1(&ecs, clone_parent),
        Some(clone_child),
        "clone_parent.LikedBy1 == clone_child (1:1 slot re-established, no eviction)",
    );

    // SOURCE untouched.
    assert_eq!(liked_by1(&ecs, parent), Some(child), "source parent.LikedBy1 still == child");
    assert_eq!(likes1_of(&ecs, child), Some(parent), "source child.Likes1 still == parent");

    // NO eviction → NO OnUnlink; exactly one genuine new OnLink (the clone edge).
    assert_eq!(
        DI_UNLINK.load(SEQ),
        0,
        "in-subtree 1:1 clone fires NO OnUnlink (the clone slot was empty — no eviction)",
    );
    assert_eq!(
        DI_LINK.load(SEQ),
        1,
        "in-subtree 1:1 clone fires exactly ONE OnLink (the single re-established edge)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// D-(ii) — OCCUPIED-EXTERNAL 1:1 clone (DETACH): the cloned source's FK targets an
//          OCCUPIED EXTERNAL 1:1 target T. Under suppression the clone DETACHES —
//          its own FK is dropped; the external incumbent + the reverse slot are
//          untouched; NO OnUnlink; FK↔reverse globally consistent.
//
// Topology: T is EXTERNAL (NOT cloned). subtree: parent --ChildOf-- child.
//   child Likes1(T)  ⇒ T.LikedBy1 == child  (child is the external incumbent of T's
//   1:1 slot). Clone {parent, child}. The clone_child's Likes1(T) stays VERBATIM (T
//   not in the subtree), so under the EvictionSuppressGuard the relink hits T's
//   OCCUPIED slot (held by the source child) and DETACHES the clone instead of
//   evicting the source child. (Production 1:1 cannot have TWO distinct entities
//   carry Likes1(T) — the slot occupant IS the only Likes1(T) holder — so the
//   genuine occupied-external incumbent is the source child itself.)
// ════════════════════════════════════════════════════════════════════════════

static DII_LINK: AtomicUsize = AtomicUsize::new(0);
static DII_UNLINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn dii_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    DII_LINK.fetch_add(1, SEQ);
}
unsafe fn dii_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    DII_UNLINK.fetch_add(1, SEQ);
}

#[test]
fn exclusive_clone_external_occupied_target_detaches_incumbent_untouched() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes1>(dii_on_link);
    ecs.observe_on_unlink::<Likes1>(dii_on_unlink);

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let t = cmds.spawn(Tag(100)).id(); // external 1:1 target — never cloned
        let parent = cmds.spawn(Tag(0)).id(); // subtree root
        let child = cmds.spawn(Tag(1)).id(); // the external incumbent of T's slot
        cmds.entity(parent).add_child(child);
        cmds.entity(child).insert(Likes1(t)); // child Likes1(T) ⇒ T.LikedBy1 == child
        probe.lock().expect("probe").extend([t, parent, child]);
    });
    let v = sink.lock().expect("probe").clone();
    let (t, parent, child) = (v[0], v[1], v[2]);

    // Pre-clone: T's 1:1 slot is OCCUPIED by the source child (the external incumbent).
    assert_eq!(liked_by1(&ecs, t), Some(child), "external incumbent: T.LikedBy1 == child");
    assert_eq!(likes1_of(&ecs, child), Some(t), "source child.Likes1 == T");

    DII_LINK.store(0, SEQ);
    DII_UNLINK.store(0, SEQ);

    // Deep-clone the parent subtree (T is NOT in it).
    let clone_parent = ecs.clone_subtree(parent);
    let kids = clone_children(&ecs, clone_parent);
    assert_eq!(kids.len(), 1, "cloned the single child");
    let clone_child = kids[0];
    assert_ne!(clone_child, child, "the cloned child is distinct");

    // DETACH: the clone's FK toward the occupied external T is DROPPED.
    assert_eq!(
        likes1_of(&ecs, clone_child),
        None,
        "DETACH (C3): the clone's Likes1(T) FK was dropped — the clone is unrelated to \
         the occupied external 1:1 target T (no eviction of the incumbent)",
    );

    // External incumbent (the source child) + the reverse slot are byte-for-byte UNTOUCHED.
    assert_eq!(
        liked_by1(&ecs, t),
        Some(child),
        "the external incumbent is untouched: T.LikedBy1 still == child (no theft)",
    );
    assert_eq!(likes1_of(&ecs, child), Some(t), "the incumbent child.Likes1 still == T");

    // NO OnUnlink fired (the detach is NOT an eviction — the incumbent is never unlinked).
    assert_eq!(
        DII_UNLINK.load(SEQ),
        0,
        "DETACH fires NO OnUnlink — the external incumbent is never evicted",
    );

    // FK↔REVERSE GLOBAL CONSISTENCY:
    //   (1) the clone carries NO dangling Likes1(T) (it detached);
    //   (2) T's slot occupant holds the matching forward FK (no orphan reverse entry).
    assert_ne!(
        likes1_of(&ecs, clone_child),
        Some(t),
        "the clone must NOT carry a dangling Likes1(T)",
    );
    let slot_occupant = liked_by1(&ecs, t).expect("T slot is occupied");
    assert_eq!(
        likes1_of(&ecs, slot_occupant),
        Some(t),
        "FK↔reverse consistency: T's slot occupant holds the matching Likes1(T) FK",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// D-(iii) — reviewer M2: an in-subtree clone whose remapped 1:1 target slot is
//           ALREADY empty/correct (the detach arm is bypassed). Links correctly,
//           no spurious fire. This is the SAME structural shape as D-(i) but
//           explicitly pins the "empty clone slot ⇒ plain add, not eviction" arm:
//           a 1:1 target that is cloned FRESH (its slot starts empty) takes the
//           `Added` arm, never the `Evicted`/`Detach` arm.
//
// Topology: a --ChildOf-- root; b --ChildOf-- root. a Likes1(b) [1:1, in-subtree].
//   b is a FRESH clone target with an empty slot ⇒ clone_a links into clone_b's
//   empty slot via the plain-add arm.
// ════════════════════════════════════════════════════════════════════════════

static DIII_LINK: AtomicUsize = AtomicUsize::new(0);
static DIII_UNLINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn diii_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    DIII_LINK.fetch_add(1, SEQ);
}
unsafe fn diii_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    DIII_UNLINK.fetch_add(1, SEQ);
}

#[test]
fn exclusive_clone_in_subtree_empty_target_slot_plain_add_no_spurious_fire() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes1>(diii_on_link);
    ecs.observe_on_unlink::<Likes1>(diii_on_unlink);

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let root = cmds.spawn(Tag(0)).id();
        let a = cmds.spawn(Tag(1)).id();
        let b = cmds.spawn(Tag(2)).id();
        cmds.entity(root).add_child(a);
        cmds.entity(root).add_child(b);
        cmds.entity(a).insert(Likes1(b)); // a Likes1(b) — both in-subtree, b's slot full of a
        probe.lock().expect("probe").extend([root, a, b]);
    });
    let v = sink.lock().expect("probe").clone();
    let (root, a, b) = (v[0], v[1], v[2]);
    assert_eq!(liked_by1(&ecs, b), Some(a), "source: b.LikedBy1 == a");

    DIII_LINK.store(0, SEQ);
    DIII_UNLINK.store(0, SEQ);

    let clone_root = ecs.clone_subtree(root);
    let kids = clone_children(&ecs, clone_root);
    assert_eq!(kids.len(), 2, "both children cloned");

    // Identify clone_a (the one carrying a Likes1 FK) and clone_b (its target).
    let mut clone_a = None;
    let mut clone_b = None;
    for &k in &kids {
        if likes1_of(&ecs, k).is_some() {
            clone_a = Some(k);
        } else {
            clone_b = Some(k);
        }
    }
    let clone_a = clone_a.expect("exactly one cloned child carries the Likes1 FK");
    let clone_b = clone_b.expect("the other cloned child is the 1:1 target");

    // clone_a's FK remapped to clone_b (in-subtree); clone_b's FRESH slot took the add.
    assert_eq!(
        likes1_of(&ecs, clone_a),
        Some(clone_b),
        "clone_a.Likes1 remapped to the cloned target clone_b",
    );
    assert_eq!(
        liked_by1(&ecs, clone_b),
        Some(clone_a),
        "clone_b's FRESH (empty) 1:1 slot took clone_a via the plain-add arm (M2: not eviction)",
    );

    // No eviction (clone_b's slot was empty) ⇒ no OnUnlink; exactly one new OnLink.
    assert_eq!(DIII_UNLINK.load(SEQ), 0, "M2: an empty clone target slot fires NO OnUnlink");
    assert_eq!(DIII_LINK.load(SEQ), 1, "M2: exactly one OnLink for the single re-established edge");

    // Source side untouched.
    assert_eq!(liked_by1(&ecs, b), Some(a), "source b.LikedBy1 still == a");
}
