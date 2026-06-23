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

// ════════════════════════════════════════════════════════════════════════════
// Target 4 — BUG-EDGE-CLONE-1 fix: the deep-clone materialize+relink path under TB.
//            Exercises the `LinkSuppressGuard` crossing each node's
//            `materialize_clone` unsafe AND the generic relink
//            (`relationship_clone_relink::<Likes>` → `LinkCommand::apply` migrate)
//            of the IN-SUBTREE edge. Tiny subtree (Miri ~100x). Asserts only the
//            in-subtree consistency that DOES hold (clone_child Likes clone_parent,
//            clone_parent.LikedBy ∋ clone_child) — the soundness probe, not the
//            BUG-EDGE-CLONE-2 external case (that is an integration-level invariant
//            test, here we only need the unsafe to be TB-clean).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_deep_clone_related_subtree_relink_tb_clean() {
    use boyko_ecs::ecs::core::hierarchy::Children;

    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (parent, child) = (e[0], e[1]);

    // parent → child (ChildOf); child Likes(parent) — one in-subtree Likes edge.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
        cmds.entity(child).insert(Likes(parent));
    });

    // Deep-clone the subtree: the suppress guard wraps the materialize walk, then
    // the relink re-establishes the single in-subtree Likes edge toward the clone.
    let clone_parent = ecs.clone_subtree(parent);
    assert_ne!(clone_parent, parent, "clone parent distinct");

    let clone_child = ecs
        .get_component::<Children>(clone_parent)
        .map(|c| c.as_slice()[0])
        .expect("cloned parent has a rebuilt Children index");

    assert_eq!(
        ecs.get_component::<Likes>(clone_child).map(|r| r.target()),
        Some(clone_parent),
        "in-subtree Likes FK remapped to the clone parent",
    );
    let clone_liked = ecs.get_component::<LikedBy>(clone_parent).expect("clone parent LikedBy");
    assert_eq!(clone_liked.len(), 1, "clone_parent.LikedBy has exactly the clone_child (relink)");

    // Source untouched (the BUG-EDGE-CLONE-1 non-leak, also asserted natively in
    // relations_deep_clone_external_target; here it doubles as a TB sanity read).
    let src_liked = ecs.get_component::<LikedBy>(parent).expect("source parent LikedBy");
    assert_eq!(src_liked.len(), 1, "source parent.LikedBy unchanged (no clone leaked in)");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 5 — BUG-EDGE-CLONE-2 fix: the EXTERNAL-FK relink path under TB. The
//            cloned child carries `Likes(E)` where E is OUTSIDE the cloned set,
//            so `relationship_clone_map_entities` keeps the FK verbatim and the
//            relink (`relationship_clone_relink::<Likes>`, no longer gated on
//            `map.is_clone`) routes `LinkCommand::<Likes>::apply` into E's reverse
//            index — the exact link surface the BUG-EDGE-CLONE-2 gate-removal newly
//            activates and that Target 4 (in-subtree FK only) never reaches. E
//            already hosts a `LikedBy` (its source liker, the original child), so
//            the relink takes the IN-PLACE push arm (`collection_mut_risky().add`)
//            on an EXTERNAL target during the deep-clone relink pass under
//            `&mut EcsMaster`. Tiny subtree (Miri ~100x). Asserts the kept external
//            FK gained its reverse entry, with no source-side leak (E's source
//            liker stays, the clone is appended exactly once).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_deep_clone_external_fk_relink_tb_clean() {
    use boyko_ecs::ecs::core::hierarchy::Children;

    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (external, parent, child) = (e[0], e[1], e[2]);

    // parent → child (ChildOf); child Likes(E) — one EXTERNAL Likes edge (E is not
    // in the cloned subtree). The build-time on_insert already migrated E.LikedBy
    // to host the source child, so the post-clone relink is an in-place push.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
        cmds.entity(child).insert(Likes(external));
    });

    // Deep-clone parent's subtree: E is NOT cloned. The suppress guard wraps the
    // materialize walk; the relink then links the clone_child into E.LikedBy via
    // the kept verbatim external FK (the BUG-EDGE-CLONE-2 path).
    let clone_parent = ecs.clone_subtree(parent);
    assert_ne!(clone_parent, parent, "clone parent distinct");

    let clone_child = ecs
        .get_component::<Children>(clone_parent)
        .map(|c| c.as_slice()[0])
        .expect("cloned parent has a rebuilt Children index");

    // The external FK is kept verbatim (points at E, NOT at any clone).
    assert_eq!(
        ecs.get_component::<Likes>(clone_child).map(|r| r.target()),
        Some(external),
        "external Likes FK kept verbatim (target outside the cloned subtree)",
    );

    // BUG-EDGE-CLONE-2: the kept external FK now HAS its reverse entry. E.LikedBy
    // gained the clone_child alongside the original source child (relink pushed it).
    let e_liked = ecs.get_component::<LikedBy>(external).expect("E LikedBy");
    assert_eq!(
        e_liked.len(),
        2,
        "E.LikedBy contains the SOURCE child and the relinked clone_child \
         (external-FK reverse entry restored under TB)",
    );
}
