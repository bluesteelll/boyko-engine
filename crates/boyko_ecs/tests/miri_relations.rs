//! Relations v1 — R3: Miri (Tree Borrows) coverage for the GENERIC cascade
//! (`docs/RELATIONS-API-PLAN.md` §test-matrix R3). THE soundness gate for the
//! generalized BUG-P19-TB-1 cascade — proves the disjoint-allocation drain holds
//! for an ARBITRARY relation, not just `ChildOf`.
//!
//! Run via (NOTE `-Zmiri-ignore-leaks` — the by-design bounded `BundleColumnCache`
//! `Box::leak`, #53 NOT-A-BUG, is orthogonal to Tree Borrows; matches the sibling
//! Miri suites):
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! rustup run nightly-2026-05-20-x86_64-pc-windows-gnu cargo miri test -p boyko-ecs --test miri_relations
//! ```
//!
//! The generic `relationship_target_on_replace::<T>` body relocates the Phase-19
//! cascade's TWO unsafe surfaces verbatim, now monomorphized per relation type —
//! the INLINE path (`n <= CASCADE_FANOUT_INLINE == 32`: the `[MaybeUninit<Entity>;
//! 32]` buffer + read-then-`assume_init`, with the `&LikedBy` dropped BEFORE
//! `commands()`, the F2 / OBS-FIRE-LOOP anchor) and the WIDE path (`n > 32`: the
//! per-turn re-derive, no buffer, no unsafe). Plus the generic
//! `LinkCommand::<R>::apply` first-source migrate (the audited raw archetype-id
//! projection) and `UnlinkCommand::<R>::apply` swap_remove.
//!
//! `#![cfg(miri)]` — only compiles under Miri; native runs ignore the file (the
//! `relations_derive` / `relations_cyclic_cascade` suites cover the same semantics
//! end-to-end natively). Entity counts are kept TINY (Miri is ~100x slower).

#![cfg(miri)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::relationship::{Relationship, RelationshipTarget};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

/// A `LINKED_DESPAWN` generic relation (NOT `ChildOf`).
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct MTag(u32);

/// Spawns `n` markers; returns now-live handles (one apply window). Tiny `n`.
fn spawn_entities(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut local = probe.lock().expect("probe lock");
        for i in 0..n {
            local.push(cmds.spawn(MTag(i as u32)).id());
        }
    });
    sink.lock().expect("probe lock").clone()
}

// ════════════════════════════════════════════════════════════════════════════
// Target 1 — link enqueue + LinkCommand::<Likes>::apply first-source migrate
//            (the audited raw archetype-id projection) + second-source in-place
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_likes_link_first_and_second_source() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (target, s0, s1) = (e[0], e[1], e[2]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(s0).insert(Likes(target)); // first source → migrate (None-arm)
        cmds.entity(s1).insert(Likes(target)); // second source → in-place push
    });

    let liked = ecs.get_component::<LikedBy>(target).expect("LikedBy present");
    assert_eq!(liked.len(), 2, "first-source migrate + second-source in-place push");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 2 — Likes::on_replace UNLINK + UnlinkCommand::<Likes>::apply swap_remove
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_likes_unlink_swap_remove() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).remove::<Likes>();
    });

    assert_eq!(ecs.get_component::<Likes>(source).map(|r| r.target()), None, "unlinked");
    let liked = ecs.get_component::<LikedBy>(target).expect("LikedBy retained empty");
    assert!(liked.is_empty(), "swap_remove emptied the collection");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 3 — the GENERALIZED BUG-P19-TB-1 cascade INLINE path: a ≥2-level Likes
//            cascade. The MaybeUninit buffer + read-then-assume_init, with the
//            re-entrant despawn under the disjoint-allocation drain. THE gate.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_likes_cascade_reentrant_push_two_levels() {
    let mut ecs = EcsMaster::new();
    // T2 ← {T1} ← {a, b}: T1 Likes T2; a,b Like T1. Despawning T2 cascades T1;
    // T1's cascade (re-entrant) despawns a, b. Two cascade levels through the
    // flat deferred-hook drain — the canonical re-entrant-push surface.
    let e = spawn_entities(&mut ecs, 4);
    let (t2, t1, a, b) = (e[0], e[1], e[2], e[3]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(t1).insert(Likes(t2));
        cmds.entity(a).insert(Likes(t1));
        cmds.entity(b).insert(Likes(t1));
    });

    // Despawn T2 → cascade despawns T1 (T2.LikedBy = {T1}); T1's on_replace
    // cascade (mid-drain, re-entrant) despawns a, b (T1.LikedBy = {a, b}).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(t2).despawn();
    });

    assert!(!ecs.has_entity(t2), "T2 gone");
    assert!(!ecs.has_entity(t1), "T1 cascaded (level 1)");
    assert!(!ecs.has_entity(a), "a cascaded (level 2, re-entrant)");
    assert!(!ecs.has_entity(b), "b cascaded (level 2, re-entrant)");
    assert_eq!(ecs.entity_count(), 0, "the whole generic cascade completed in one drain");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 4 — the cyclic LINKED_DESPAWN cascade under TB (C1): a 2-cycle re-enters
//            an already-dead entity (generation-checked no-op). No UB, no hang.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_likes_cyclic_cascade_terminates() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (a, b) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).insert(Likes(b));
        cmds.entity(b).insert(Likes(a));
    });

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).despawn();
    });

    assert!(!ecs.has_entity(a), "A despawned");
    assert!(!ecs.has_entity(b), "B cascaded; cyclic re-entry of A was a dead no-op");
    assert_eq!(ecs.entity_count(), 0, "cyclic cascade terminated under TB");
}
