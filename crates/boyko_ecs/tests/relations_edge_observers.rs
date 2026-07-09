//! Relation-edge observers — `OnLink<R>` / `OnUnlink<R>` BEHAVIORAL +
//! fire-site-enumeration suite (the observer-side relation-aware DSL,
//! Decision 5 / critic W2).
//!
//! Pins the committed-edge fire rule for the built-in
//! [`OnLink`](boyko_ecs::ecs::core::relationship::OnLink) /
//! [`OnUnlink`](boyko_ecs::ecs::core::relationship::OnUnlink) triggers, which
//! fire INSIDE `LinkCommand::apply` / `UnlinkCommand::apply` (relationship/mod.rs
//! :418 / :444), AFTER the validity guard. Every test asserts a fire COUNT and,
//! where it is the contract, the fire ORDER.
//!
//! # Why `static` counters / per-test fresh `EcsMaster`
//!
//! A `TriggerFn` is a bare `unsafe fn` pointer — it cannot capture; each test
//! owns private module-level `static` counters and its OWN relation type so
//! concurrently-running tests never observe one another's fires (the trigger /
//! observer registries are process-wide in the test binary). The captured
//! `OnLink<R>` payload (the target `Entity`) is recorded into a `static`
//! `AtomicUsize` keyed by entity id for the order/target assertions.
//!
//! Mirrors `relations_derive.rs` (the `spawn_entities` Arc<Mutex> probe harness)
//! and `feature2_observers_behavioral.rs` (the `static AtomicUsize` runner
//! pattern). Genericity is proven by using DISTINCT user relations (`Likes`,
//! `Trusts`) — never `ChildOf`-special — for the edge-observer cases.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::trigger::TriggerContext;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::relationship::{
    OnLink, OnUnlink, Relationship, RelationshipSourceCollection, RelationshipTarget,
};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

const SEQ: Ordering = Ordering::SeqCst;

/// Process-global monotonic clock — records the ORDER fires happen in.
static CLOCK: AtomicUsize = AtomicUsize::new(0);
#[inline]
fn tick() -> usize {
    CLOCK.fetch_add(1, SEQ)
}

// ════════════════════════════════════════════════════════════════════════════
// Relations under test (generic, derive-built — NOT ChildOf-special)
// ════════════════════════════════════════════════════════════════════════════

/// Cascade-ON relation used by most edge-observer cases.
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

/// A SECOND, independent relation — proves the edge observers are keyed per-`R`
/// (the `(R, *)` analogue): an `OnLink<Trusts>` observer must NOT fire for a
/// `Likes` edge, and vice versa.
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = TrustedBy)]
struct Trusts(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Trusts, retain_empty)]
struct TrustedBy(Vec<Entity>);

/// A plain marker so a freshly-spawned entity has a concrete archetype before a
/// relationship FK migrates it.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

// ════════════════════════════════════════════════════════════════════════════
// Harness
// ════════════════════════════════════════════════════════════════════════════

/// Spawns `n` markers through the deferred queue; returns now-live handles in
/// spawn order (one apply window). Mirrors `relations_derive::spawn_entities`.
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
    assert_eq!(out.len(), n, "spawn helper produced n handles");
    for &e in &out {
        assert!(ecs.has_entity(e), "spawned entity is live after the apply window");
    }
    out
}

// ════════════════════════════════════════════════════════════════════════════
// A.1 — OnLink<R> fires ONCE on a fresh FK insert; OnUnlink<R> ONCE on remove
// ════════════════════════════════════════════════════════════════════════════

static A1_LINK: AtomicUsize = AtomicUsize::new(0);
static A1_UNLINK: AtomicUsize = AtomicUsize::new(0);
static A1_LINK_TARGET: AtomicUsize = AtomicUsize::new(usize::MAX);
static A1_UNLINK_TARGET: AtomicUsize = AtomicUsize::new(usize::MAX);

unsafe fn a1_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, ev: *const u8) {
    // SAFETY: the edge-fire walk pins a live `OnLink<Likes>` for the call.
    let e = unsafe { &*(ev as *const OnLink<Likes>) };
    A1_LINK.fetch_add(1, SEQ);
    A1_LINK_TARGET.store(e.target.id().0, SEQ);
}
unsafe fn a1_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, ev: *const u8) {
    let e = unsafe { &*(ev as *const OnUnlink<Likes>) };
    A1_UNLINK.fetch_add(1, SEQ);
    A1_UNLINK_TARGET.store(e.old_target.id().0, SEQ);
}

#[test]
fn on_link_fires_once_on_fk_insert_and_on_unlink_once_on_remove() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes>(a1_on_link);
    ecs.observe_on_unlink::<Likes>(a1_on_unlink);

    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);
    A1_LINK.store(0, SEQ);
    A1_UNLINK.store(0, SEQ);

    // FK insert commits the edge → OnLink fires exactly once, carrying the target.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });
    assert_eq!(A1_LINK.load(SEQ), 1, "OnLink fires exactly once on the committed FK insert");
    assert_eq!(A1_LINK_TARGET.load(SEQ), target.id().0, "OnLink carries the committed target");
    assert_eq!(A1_UNLINK.load(SEQ), 0, "no OnUnlink yet");

    // FK remove destroys the edge → OnUnlink fires exactly once, carrying old_target.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).remove::<Likes>();
    });
    assert_eq!(A1_UNLINK.load(SEQ), 1, "OnUnlink fires exactly once on the FK removal");
    assert_eq!(
        A1_UNLINK_TARGET.load(SEQ),
        target.id().0,
        "OnUnlink carries the destroyed edge's old_target",
    );
    assert_eq!(A1_LINK.load(SEQ), 1, "OnLink did not fire again on the remove");
}

// ════════════════════════════════════════════════════════════════════════════
// A.2 — RE-TARGET ORDER: changing a source's FK fires OnUnlink{old} BEFORE
//       OnLink{new}. Proven on `Trusts` (NOT ChildOf) for genericity.
// ════════════════════════════════════════════════════════════════════════════

// The order is recorded via the global clock at fire time; targets are stashed
// so the assertion can prove WHICH target each fire carried.
static A2_LINK_TICK: AtomicUsize = AtomicUsize::new(usize::MAX);
static A2_UNLINK_TICK: AtomicUsize = AtomicUsize::new(usize::MAX);
static A2_LINK_TARGET: AtomicUsize = AtomicUsize::new(usize::MAX);
static A2_UNLINK_TARGET: AtomicUsize = AtomicUsize::new(usize::MAX);

unsafe fn a2_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, ev: *const u8) {
    let e = unsafe { &*(ev as *const OnLink<Trusts>) };
    A2_LINK_TICK.store(tick(), SEQ);
    A2_LINK_TARGET.store(e.target.id().0, SEQ);
}
unsafe fn a2_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, ev: *const u8) {
    let e = unsafe { &*(ev as *const OnUnlink<Trusts>) };
    A2_UNLINK_TICK.store(tick(), SEQ);
    A2_UNLINK_TARGET.store(e.old_target.id().0, SEQ);
}

#[test]
fn retarget_fires_unlink_old_before_link_new() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Trusts>(a2_on_link);
    ecs.observe_on_unlink::<Trusts>(a2_on_unlink);

    let e = spawn_entities(&mut ecs, 3);
    let (t1, t2, source) = (e[0], e[1], e[2]);

    // First link to t1 (this is the only OnLink before the retarget; reset the
    // tick stash AFTER so the retarget's fires are the ones we measure).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Trusts(t1));
    });
    A2_LINK_TICK.store(usize::MAX, SEQ);
    A2_UNLINK_TICK.store(usize::MAX, SEQ);

    // Overwrite Trusts(t1) → Trusts(t2): on_replace(t1) unlink THEN on_insert(t2)
    // link, applied in FIFO order — OnUnlink{t1} must precede OnLink{t2}.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Trusts(t2));
    });

    let unlink_tick = A2_UNLINK_TICK.load(SEQ);
    let link_tick = A2_LINK_TICK.load(SEQ);
    assert_ne!(unlink_tick, usize::MAX, "retarget fired OnUnlink (old side)");
    assert_ne!(link_tick, usize::MAX, "retarget fired OnLink (new side)");
    assert_eq!(A2_UNLINK_TARGET.load(SEQ), t1.id().0, "OnUnlink carried the OLD target (t1)");
    assert_eq!(A2_LINK_TARGET.load(SEQ), t2.id().0, "OnLink carried the NEW target (t2)");
    assert!(
        unlink_tick < link_tick,
        "ORDER: OnUnlink{{old=t1}} (tick {unlink_tick}) fires BEFORE OnLink{{new=t2}} (tick {link_tick})",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// A.3 — DEAD TARGET: linking to an already-despawned target fires NO OnLink
//       (the edge never commits — the LinkCommand dangling guard short-circuits
//       BEFORE the fire site).
// ════════════════════════════════════════════════════════════════════════════

static A3_LINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn a3_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A3_LINK.fetch_add(1, SEQ);
}

#[test]
fn link_to_dead_target_fires_no_on_link() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes>(a3_on_link);

    let e = spawn_entities(&mut ecs, 2);
    let (source, victim) = (e[0], e[1]);
    A3_LINK.store(0, SEQ);

    // Kill the target first.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(victim).despawn();
    });
    assert!(!ecs.has_entity(victim), "victim is dead — a dangling target");

    // Link to the dead target. The on_insert self-ref/dangling guard removes the
    // bad FK and the LinkCommand dangling guard no-ops BEFORE the fire site.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(victim));
    });

    assert_eq!(
        A3_LINK.load(SEQ),
        0,
        "no OnLink for a never-committed edge (dead target — LinkCommand dangling guard)",
    );
    assert_eq!(
        ecs.get_component::<Likes>(source).map(|r| r.target()),
        None,
        "the dangling FK was reactively removed",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// A.4 — NO-OP semantics: re-inserting the SAME FK target. Documents the EXACT
//       semantics (the reinsert routes through on_replace(old==new) unlink +
//       on_insert(new) link, so it DOES fire one OnUnlink + one OnLink — it is
//       NOT a spurious-fire bug, it is the replace machinery).
// ════════════════════════════════════════════════════════════════════════════

static A4_LINK: AtomicUsize = AtomicUsize::new(0);
static A4_UNLINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn a4_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A4_LINK.fetch_add(1, SEQ);
}
unsafe fn a4_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A4_UNLINK.fetch_add(1, SEQ);
}

#[test]
fn reinsert_same_target_documented_replace_semantics() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes>(a4_on_link);
    ecs.observe_on_unlink::<Likes>(a4_on_unlink);

    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });
    // Baseline: exactly one OnLink, no OnUnlink from the first link.
    A4_LINK.store(0, SEQ);
    A4_UNLINK.store(0, SEQ);

    // Re-insert the SAME target. `insert` over an existing component fires
    // on_replace (unlink old==target) THEN on_insert (link target). The reverse
    // collection's `remove(source)` succeeds (source IS present) so OnUnlink
    // fires; the relink re-adds and OnLink fires. This is the DOCUMENTED replace
    // path — not a spurious double-fire on a no-op.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });

    assert_eq!(
        A4_UNLINK.load(SEQ),
        1,
        "re-inserting the same target fires ONE OnUnlink (the replace machinery removes the \
         old==new edge first) — documented semantics, not a spurious fire",
    );
    assert_eq!(
        A4_LINK.load(SEQ),
        1,
        "re-inserting the same target fires ONE OnLink (the relink re-commits the edge)",
    );
    // Final state is consistent: source still Likes target, present exactly once.
    let likers: Vec<Entity> = ecs
        .get_component::<LikedBy>(target)
        .map(|c| RelationshipSourceCollection::iter(c.collection()).collect())
        .expect("target retains LikedBy");
    assert_eq!(likers, vec![source], "the source is present exactly once after the reinsert");
}

// ════════════════════════════════════════════════════════════════════════════
// A.5 — CASCADE: explicit per-source FK removal (target ALIVE) fires OnUnlink
//       exactly ONCE per source (not double-counted). This is the cascade-shaped
//       multi-unlink with a LIVE target — the committed-edge case where OnUnlink
//       fires (the target's reverse collection still exists at apply time).
// ════════════════════════════════════════════════════════════════════════════

static A5_UNLINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn a5_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A5_UNLINK.fetch_add(1, SEQ);
}

#[test]
fn multi_source_unlink_live_target_fires_once_per_source() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_unlink::<Trusts>(a5_on_unlink);

    let e = spawn_entities(&mut ecs, 4);
    let target = e[0];
    let sources = [e[1], e[2], e[3]];
    ecs.run_system(move |mut cmds: Commands| {
        for &s in &sources {
            cmds.entity(s).insert(Trusts(target));
        }
    });
    A5_UNLINK.store(0, SEQ);

    // Remove each source's Trusts FK while the target is ALIVE. Each removal's
    // UnlinkCommand finds the target's live TrustedBy and removes the source
    // (committed-edge present) → exactly one OnUnlink per source, no double-count.
    ecs.run_system(move |mut cmds: Commands| {
        for &s in &sources {
            cmds.entity(s).remove::<Trusts>();
        }
    });

    assert_eq!(
        A5_UNLINK.load(SEQ),
        3,
        "OnUnlink fires exactly once per source on multi-source unlink with a LIVE target \
         (3 sources, no double-count)",
    );
    assert!(ecs.has_entity(target), "live target survives the source unlinks");
}

// ════════════════════════════════════════════════════════════════════════════
// A.5b — DOCUMENTED SEMANTIC: despawning a NON-LINKED_DESPAWN target fires NO
//        OnUnlink for the unlinked sources. The teardown removes each source's
//        FK, but by the time the source's UnlinkCommand applies the target's
//        reverse collection is already gone (target despawned) — so the
//        committed-edge test (`get_component_mut::<TrustedBy>(target)` → None)
//        short-circuits before the fire. This pins the contract; it is NOT a bug.
// ════════════════════════════════════════════════════════════════════════════

static A5B_UNLINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn a5b_on_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A5B_UNLINK.fetch_add(1, SEQ);
}

#[test]
fn non_cascading_target_despawn_fires_no_unlink_target_gone() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_unlink::<Trusts>(a5b_on_unlink);

    let e = spawn_entities(&mut ecs, 4);
    let target = e[0];
    let sources = [e[1], e[2], e[3]];
    ecs.run_system(move |mut cmds: Commands| {
        for &s in &sources {
            cmds.entity(s).insert(Trusts(target));
        }
    });
    A5B_UNLINK.store(0, SEQ);

    // Despawn the target. TrustedBy::LINKED_DESPAWN == false → the non-cascading
    // teardown enqueues remove::<Trusts> on each source; but the target's
    // TrustedBy is gone by apply time, so UnlinkCommand finds no collection and
    // fires nothing (the committed-edge guard short-circuits).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(target).despawn();
    });

    assert_eq!(
        A5B_UNLINK.load(SEQ),
        0,
        "OnUnlink does NOT fire when the target is despawned (its reverse collection is gone \
         at apply time — the committed-edge guard short-circuits): documented semantic",
    );
    for (i, &s) in sources.iter().enumerate() {
        assert!(ecs.has_entity(s), "non-cascading: source {i} survives the target despawn");
        assert_eq!(
            ecs.get_component::<Trusts>(s).map(|r| r.target()),
            None,
            "source {i}'s Trusts FK was still removed by the non-cascading teardown",
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// A.6 — CLONE-RELINK: deep-cloning a related subtree fires OnLink per
//       re-established cloned edge (the intended burst). Build a ChildOf subtree
//       whose nodes also `Likes` so the clone re-establishes Likes edges.
//
// TESTER FINDING (BUG-EDGE-CLONE-1, see report — this test FAILS): a deep clone
// of a subtree whose non-root nodes carry a generic relationship FK
// DOUBLE-RELINKS each cloned source. For ONE cloned `Likes` edge the observer
// fires TWICE: once `(clone_child -> clone_parent)` (CORRECT) and once
// `(clone_child -> ORIGINAL parent)` (WRONG) — and the second relink actually
// MUTATES the SOURCE subtree's reverse index: `original_parent.LikedBy` ends up
// `[original_child, clone_child]` (the clone leaks into the source's collection).
// The cloned child's FK itself IS correctly remapped (so the existing
// `relations_derive::likes_deep_clone_remaps_foreign_key`, which only checks the
// FK + the CLONE-side LikedBy, passes and misses this). The fault is in the
// deep-clone generic relink pass (`clone/deep.rs::remap_relink_generic_relations`
// + `relationship_clone_relink`): the cloned source is relinked into both the
// remapped (clone) target AND, via a stale path, the original target. NOT FIXED
// here (tester does not modify the impl). This test asserts the CORRECT contract
// so the regression is loud.
// ════════════════════════════════════════════════════════════════════════════

static A6_LINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn a6_on_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A6_LINK.fetch_add(1, SEQ);
}

#[test]
fn deep_clone_fires_on_link_per_reestablished_edge() {
    use boyko_ecs::ecs::core::hierarchy::Children;

    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes>(a6_on_link);

    // parent → c1, parent → c2 (ChildOf); each child Likes(parent). Two Likes
    // edges inside the cloned subtree.
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
        let mut v = probe.lock().expect("probe");
        v.extend([parent, c1, c2]);
    });
    let v = sink.lock().expect("probe").clone();
    let parent = v[0];

    // The original parent's LikedBy now has both original children.
    let orig_before: Vec<usize> = ecs
        .get_component::<LikedBy>(parent)
        .map(|c| RelationshipSourceCollection::iter(c.collection()).map(|e| e.id().0).collect())
        .expect("original parent has LikedBy");
    assert_eq!(orig_before.len(), 2, "original parent has two original likers before the clone");

    // Two Likes edges established during build. Reset; the clone is what we count.
    A6_LINK.store(0, SEQ);

    let clone_parent = ecs.clone_subtree(parent);
    assert_ne!(clone_parent, parent, "clone is a distinct entity");
    let clone_children = ecs
        .get_component::<Children>(clone_parent)
        .map(|c| c.as_slice().len())
        .expect("cloned parent has a rebuilt Children index");
    assert_eq!(clone_children, 2, "the subtree cloned both children");

    // CONTRACT (currently violated — BUG-EDGE-CLONE-1): the clone re-establishes
    // EXACTLY the two cloned Likes edges (cloned child → cloned parent) through
    // the LinkCommand::apply fire site → exactly two OnLink fires.
    assert_eq!(
        A6_LINK.load(SEQ),
        2,
        "deep clone fires OnLink once per RE-ESTABLISHED cloned edge (2 cloned Likes edges) \
         — the intended clone-relink burst (currently fires 4: BUG-EDGE-CLONE-1 double-relink)",
    );

    // TRIPWIRE: the SOURCE subtree's reverse index must be UNTOUCHED by the clone.
    let orig_after: Vec<usize> = ecs
        .get_component::<LikedBy>(parent)
        .map(|c| RelationshipSourceCollection::iter(c.collection()).map(|e| e.id().0).collect())
        .expect("original parent still has LikedBy");
    assert_eq!(
        orig_after.len(),
        2,
        "the original parent's LikedBy must NOT gain the clone's child \
         (currently {orig_after:?} — the clone leaked into the source's reverse index: \
         BUG-EDGE-CLONE-1 reverse-index corruption)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// A.7 — per-`R` keying: an OnLink<Trusts> observer does NOT fire for a Likes
//       edge (the trigger id is per-monomorphisation — the `(R, *)` analogue).
// ════════════════════════════════════════════════════════════════════════════

static A7_TRUSTS_LINK: AtomicUsize = AtomicUsize::new(0);
static A7_LIKES_LINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn a7_trusts_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A7_TRUSTS_LINK.fetch_add(1, SEQ);
}
unsafe fn a7_likes_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A7_LIKES_LINK.fetch_add(1, SEQ);
}

#[test]
fn edge_observers_are_keyed_per_relation_type() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Trusts>(a7_trusts_link);
    ecs.observe_on_link::<Likes>(a7_likes_link);

    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);
    A7_TRUSTS_LINK.store(0, SEQ);
    A7_LIKES_LINK.store(0, SEQ);

    // Commit ONLY a Likes edge.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });

    assert_eq!(A7_LIKES_LINK.load(SEQ), 1, "OnLink<Likes> fired for the Likes edge");
    assert_eq!(
        A7_TRUSTS_LINK.load(SEQ),
        0,
        "OnLink<Trusts> did NOT fire for a Likes edge — edge observers are keyed per-R \
         (distinct TriggerId per monomorphisation, the (R, *) analogue)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// A.8 — ENTITY-TARGETED edge observer fires for ITS source on a committed edge;
//       and the DOCUMENTED semantic that an entity-targeted OnUnlink on a SOURCE
//       being despawned is intentionally skipped (only global fires there).
// ════════════════════════════════════════════════════════════════════════════

static A8_GLOBAL_LINK: AtomicUsize = AtomicUsize::new(0);
static A8_ENTITY_LINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn a8_global_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A8_GLOBAL_LINK.fetch_add(1, SEQ);
}
unsafe fn a8_entity_link(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A8_ENTITY_LINK.fetch_add(1, SEQ);
}

#[test]
fn entity_targeted_edge_observer_fires_for_its_source() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes>(a8_global_link);

    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);
    A8_GLOBAL_LINK.store(0, SEQ);
    A8_ENTITY_LINK.store(0, SEQ);

    // Attach an ENTITY-TARGETED OnLink observer on the SOURCE (the edge fires on
    // the source). `OnLink<Likes>` IS a Trigger, so observe_entity_event applies.
    ecs.observe_entity_event::<OnLink<Likes>>(source, a8_entity_link);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });

    assert_eq!(A8_GLOBAL_LINK.load(SEQ), 1, "the GLOBAL OnLink observer fired");
    assert_eq!(
        A8_ENTITY_LINK.load(SEQ),
        1,
        "the ENTITY-TARGETED OnLink observer on the source fired for the committed edge",
    );
}

static A8B_GLOBAL_UNLINK: AtomicUsize = AtomicUsize::new(0);
static A8B_ENTITY_UNLINK: AtomicUsize = AtomicUsize::new(0);

unsafe fn a8b_global_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A8B_GLOBAL_UNLINK.fetch_add(1, SEQ);
}
unsafe fn a8b_entity_unlink(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    A8B_ENTITY_UNLINK.fetch_add(1, SEQ);
}

#[test]
fn entity_targeted_unlink_on_despawning_source_is_skipped_only_global_fires() {
    // DOCUMENTED semantic (not a bug): when the SOURCE is itself being despawned,
    // its archetype is being torn down, so the per-entity observer dispatch on the
    // source is intentionally skipped — only the GLOBAL OnUnlink fires for the
    // unlink that the source's teardown drives. This test ENCODES that contract.
    let mut ecs = EcsMaster::new();
    ecs.observe_on_unlink::<Trusts>(a8b_global_unlink);

    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Trusts(target));
    });

    // Attach an entity-targeted OnUnlink on the source, THEN despawn the source.
    ecs.observe_entity_event::<OnUnlink<Trusts>>(source, a8b_entity_unlink);
    A8B_GLOBAL_UNLINK.store(0, SEQ);
    A8B_ENTITY_UNLINK.store(0, SEQ);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).despawn();
    });

    // The source's Trusts is removed by its own teardown → the edge unlinks.
    assert_eq!(
        A8B_GLOBAL_UNLINK.load(SEQ),
        1,
        "the GLOBAL OnUnlink fires for the edge destroyed by the source's despawn",
    );
    assert_eq!(
        A8B_ENTITY_UNLINK.load(SEQ),
        0,
        "the ENTITY-TARGETED OnUnlink on a SOURCE-being-despawned is intentionally skipped \
         (documented semantic — only global fires there)",
    );
    assert!(!ecs.has_entity(source), "source despawned");
}

// ════════════════════════════════════════════════════════════════════════════
// A.9 — 0%-gate proxy: a world that registered NO edge observer commits edges
//       with no observable side effect and stays consistent (the has_edge_observer
//       cold-probe early-out is exercised by every relations_derive test; this
//       pins that a NON-registering world is byte-behaviourally identical).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn no_edge_observer_world_commits_edges_unaffected() {
    let mut ecs = EcsMaster::new();
    // Deliberately register NOTHING.
    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });
    assert_eq!(
        ecs.get_component::<Likes>(source).map(|r| r.target()),
        Some(target),
        "the edge commits identically with no observer registered (0%-gate path)",
    );
    let likers: Vec<Entity> = ecs
        .get_component::<LikedBy>(target)
        .map(|c| RelationshipSourceCollection::iter(c.collection()).collect())
        .expect("LikedBy created");
    assert_eq!(likers, vec![source], "reverse index consistent without any edge observer");
}
