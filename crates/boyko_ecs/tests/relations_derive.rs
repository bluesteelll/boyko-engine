//! Relations v1 — R2: the DERIVE-driven generic relation (`Likes`/`LikedBy`)
//! BEHAVIORAL + install-probe suite (`docs/RELATIONS-API-PLAN.md` §test-matrix R2).
//!
//! Because the in-crate `ChildOf`/`Children` use a HAND-MIRROR of the derive
//! output (the `boyko_macros` dev-dep cycle), the `#[derive(Relationship)]` /
//! `#[derive(RelationshipTarget)]` AUTO-EMIT path (hooks + clone-remap +
//! serialize-remap metadata + the generic link/unlink/cascade machinery) is
//! exercised ONLY by these external relations. This file is therefore the SOLE
//! behavioral gate on the two derives.
//!
//! # Harness
//!
//! Mirrors `phase19_hierarchy_core.rs`: freshly-spawned `Entity` handles are
//! smuggled out of the (`Send + Sync`) system closure through an
//! `Arc<Mutex<Vec<Entity>>>` probe, then read back AFTER the `run_system` apply
//! window (the deferred-hook drain is what makes the reverse index consistent).
//!
//! # Two relations under test
//!
//! * `Likes` / `LikedBy` — `linked_despawn` (cascade ON), `retain_empty`.
//! * `Follows` / `FollowedBy` — `linked_despawn` ABSENT (cascade OFF),
//!   `retain_empty`. Despawning a `FollowedBy` target unlinks but does NOT
//!   despawn the sources.
//!
//! Both relationship SOURCES derive `Clone, Copy` (mirroring the in-crate
//! `ChildOf`, which is `#[derive(Clone, Copy)]`): the relationship-source
//! clone/serialize remap metadata is emitted through the `Component` derive's
//! autoref classification, which requires the FK-carrying type to be `Clone`.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{
    Cloneability, Serializability, get_clone_info, get_hooks, get_map_entities_fn,
    get_serialize_info,
};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::relationship::{
    Relationship, RelationshipSourceCollection, RelationshipTarget,
};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

// ════════════════════════════════════════════════════════════════════════════
// Relation definitions
// ════════════════════════════════════════════════════════════════════════════

/// Cascade-ON relation source (the canonical R2 `Likes`). `Clone, Copy` so the
/// `Component` derive classifies the FK `CloneViaFn` (remap-eligible), exactly as
/// the in-crate `ChildOf`.
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

/// Cascade-ON reverse index. v1: `retain_empty` mandatory.
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

/// Cascade-OFF relation source (`linked_despawn` ABSENT on the target).
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = FollowedBy)]
struct Follows(pub Entity);

/// Cascade-OFF reverse index — `linked_despawn` NOT set ⇒ `LINKED_DESPAWN ==
/// false`. Despawning the target unlinks the sources but does NOT despawn them.
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Follows, retain_empty)]
struct FollowedBy(Vec<Entity>);

// A plain marker so a freshly-spawned entity has a concrete archetype before a
// relationship FK migrates it. Tag only.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

// ════════════════════════════════════════════════════════════════════════════
// Helpers (mirror phase19_hierarchy_core)
// ════════════════════════════════════════════════════════════════════════════

/// Spawns `n` marker entities through the deferred queue, returns their now-live
/// handles in spawn order. One apply window runs before return.
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

/// Reads a target's current `LikedBy` sources as an owned `Vec` (post-drain).
/// `None` when the target has no `LikedBy` component at all.
fn liked_by(ecs: &EcsMaster, target: Entity) -> Option<Vec<Entity>> {
    ecs.get_component::<LikedBy>(target)
        .map(|c| RelationshipSourceCollection::iter(c.collection()).collect())
}

/// Reads a source's current `Likes` target (the FK), if any.
fn likes_of(ecs: &EcsMaster, source: Entity) -> Option<Entity> {
    ecs.get_component::<Likes>(source).map(|r| r.target())
}

/// Reads a `FollowedBy` target's sources as an owned `Vec` (post-drain).
fn followed_by(ecs: &EcsMaster, target: Entity) -> Option<Vec<Entity>> {
    ecs.get_component::<FollowedBy>(target)
        .map(|c| RelationshipSourceCollection::iter(c.collection()).collect())
}

/// `LikedBy::collection().iter()` returns `Entity` by value — collect to a sorted
/// `Vec<usize>` of ids for order-independent set comparison.
fn id_set(v: &[Entity]) -> Vec<usize> {
    let mut out: Vec<usize> = v.iter().map(|e| e.id().0).collect();
    out.sort_unstable();
    out
}

// ════════════════════════════════════════════════════════════════════════════
// R2.1 — link: Likes(T) on S → T.LikedBy ∋ S
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_links_both_directions() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });

    assert_eq!(likes_of(&ecs, source), Some(target), "source.Likes points at target");
    let likers = liked_by(&ecs, target).expect("target gained a LikedBy collection");
    assert!(likers.contains(&source), "target.LikedBy contains the source");
    assert_eq!(likers.len(), 1, "exactly one source linked");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.2 — unlink: remove Likes from S → T.LikedBy no longer has S; LikedBy stays
//         present + empty (retain-empty, W1)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_unlink_on_remove_retains_empty_collection() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });
    assert!(ecs.get_component::<LikedBy>(target).is_some(), "LikedBy created on first link");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).remove::<Likes>();
    });

    assert_eq!(likes_of(&ecs, source), None, "source has no Likes after remove");
    // retain-empty (W1): the LikedBy component STAYS on the target even when empty.
    let likers = ecs
        .get_component::<LikedBy>(target)
        .expect("LikedBy component is RETAINED after emptying (W1)");
    assert!(likers.is_empty(), "retained LikedBy is empty");
    assert!(
        !RelationshipSourceCollection::iter(likers.collection()).any(|s| s == source),
        "source not in LikedBy",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// R2.3 — reparent: Likes(T1) → Likes(T2) moves S between collections atomically
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_retarget_moves_source_atomically() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (t1, t2, source) = (e[0], e[1], e[2]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(t1));
    });
    assert!(liked_by(&ecs, t1).unwrap().contains(&source), "S under T1 first");

    // Overwrite Likes(T1) → Likes(T2): on_replace(T1) unlink THEN on_insert(T2) link.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(t2));
    });

    assert_eq!(likes_of(&ecs, source), Some(t2), "source.Likes == T2 after retarget");
    assert!(liked_by(&ecs, t2).unwrap().contains(&source), "S ∈ T2.LikedBy");
    assert!(!liked_by(&ecs, t1).unwrap().contains(&source), "S ∉ T1.LikedBy");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.4 — multi-source: S1,S2,S3 all Likes(T) → T.LikedBy has all three (as a set)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_multi_source_collects_all() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 4);
    let target = e[0];
    let sources = [e[1], e[2], e[3]];

    ecs.run_system(move |mut cmds: Commands| {
        for &s in &sources {
            cmds.entity(s).insert(Likes(target));
        }
    });

    let likers = liked_by(&ecs, target).expect("target has LikedBy");
    assert_eq!(
        id_set(&likers),
        id_set(&sources),
        "all three sources present in LikedBy (order unspecified — set compare)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// R2.5 — swap_remove: remove the MIDDLE source → the others remain (set)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_swap_remove_middle_keeps_others() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 4);
    let target = e[0];
    let (s0, s1, s2) = (e[1], e[2], e[3]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(s0).insert(Likes(target));
        cmds.entity(s1).insert(Likes(target));
        cmds.entity(s2).insert(Likes(target));
    });
    assert_eq!(liked_by(&ecs, target).unwrap().len(), 3, "three sources linked");

    // Remove the MIDDLE source — swap_remove perturbs order but must keep the rest.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(s1).remove::<Likes>();
    });

    let likers = liked_by(&ecs, target).expect("LikedBy retained");
    assert_eq!(id_set(&likers), id_set(&[s0, s2]), "s0 + s2 remain after middle removal");
    assert!(!likers.contains(&s1), "the removed middle source is gone");
    assert_eq!(likes_of(&ecs, s1), None, "s1 lost its Likes");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.6 — self-ref guard: Likes(self) reactively rejected (default no self-ref)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_self_ref_is_rejected() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 1);
    let me = e[0];

    // ALLOW_SELF_REFERENTIAL defaults to false.
    const { assert!(!Likes::ALLOW_SELF_REFERENTIAL) };

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(me).insert(Likes(me));
    });

    assert_eq!(likes_of(&ecs, me), None, "self-referential Likes was removed");
    if let Some(likers) = liked_by(&ecs, me) {
        assert!(!likers.contains(&me), "no self-membership in LikedBy");
    }
    assert!(ecs.has_entity(me), "entity still alive (guard didn't corrupt it)");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.7 — dangling: Likes(dead) reactively rejected, no phantom source
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_dangling_target_is_rejected() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (source, victim) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(victim).despawn();
    });
    assert!(!ecs.has_entity(victim), "victim is dead — a dangling target");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(victim));
    });

    assert_eq!(likes_of(&ecs, source), None, "dangling Likes was removed");
    assert_eq!(liked_by(&ecs, victim), None, "no phantom LikedBy on the dead target");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.8 — first-source migrate: first Likes into an empty target migrate-inserts
//        LikedBy::with_capacity(1) — no spurious cascade
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_first_source_migrate_no_spurious_cascade() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);

    // Target had NO LikedBy. The first link migrate-inserts LikedBy (on_add +
    // on_insert). LikedBy registers ONLY on_replace, so no cascade fires.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });

    assert!(ecs.has_entity(target), "target alive — no spurious cascade on first source");
    assert!(ecs.has_entity(source), "source alive — first-source migrate did not cascade-despawn it");
    let likers = liked_by(&ecs, target).expect("target has LikedBy after migrate");
    assert_eq!(likers, vec![source], "target.LikedBy == [source]");
    assert_eq!(ecs.entity_count(), 2, "both entities survive");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.9 — wide fanout (> CASCADE_FANOUT_INLINE) exercises the cold WIDE cascade
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_wide_fanout_cascades_all() {
    let mut ecs = EcsMaster::new();
    const FANOUT: usize = 40; // > 32 (CASCADE_FANOUT_INLINE) → wide path
    let e = spawn_entities(&mut ecs, FANOUT + 1);
    let target = e[0];
    let sources: Vec<Entity> = e[1..].to_vec();

    let sources_for_link = sources.clone();
    ecs.run_system(move |mut cmds: Commands| {
        for &s in &sources_for_link {
            cmds.entity(s).insert(Likes(target));
        }
    });
    assert_eq!(liked_by(&ecs, target).unwrap().len(), FANOUT, "all {FANOUT} sources linked");

    // Despawn the target → cascade (LINKED_DESPAWN) over the WIDE path.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(target).despawn();
    });

    assert!(!ecs.has_entity(target), "target gone");
    // Iterate via `as_slice`: `sources` is a `Vec<Entity>`, for which the in-scope
    // `RelationshipSourceCollection` is implemented (so `Vec::iter` is ambiguous);
    // `&[Entity]` does NOT impl the trait, so the slice `.iter()` is unambiguous.
    for (i, &s) in sources.as_slice().iter().enumerate() {
        assert!(!ecs.has_entity(s), "wide-path source {i} cascaded");
    }
    assert_eq!(ecs.entity_count(), 0, "wide cascade removed every source");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.10 — cascade ON (LINKED_DESPAWN): despawn T → all Likes sources despawned
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_linked_despawn_cascade_despawns_sources() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 4);
    let target = e[0];
    let sources = [e[1], e[2], e[3]];

    ecs.run_system(move |mut cmds: Commands| {
        for &s in &sources {
            cmds.entity(s).insert(Likes(target));
        }
    });
    assert_eq!(liked_by(&ecs, target).unwrap().len(), 3, "three sources linked");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(target).despawn();
    });

    assert!(!ecs.has_entity(target), "target gone");
    for (i, &s) in sources.iter().enumerate() {
        assert!(!ecs.has_entity(s), "source {i} despawned by the LINKED_DESPAWN cascade");
    }
    assert_eq!(ecs.entity_count(), 0, "target + all sources gone (generic cascade)");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.11 — cascade OFF (LINKED_DESPAWN absent): despawn T → sources survive
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn no_cascade_when_linked_despawn_off() {
    // FollowedBy::LINKED_DESPAWN must be false (the flag is absent).
    const { assert!(!FollowedBy::LINKED_DESPAWN) };
    const { assert!(LikedBy::LINKED_DESPAWN) };

    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let target = e[0];
    let sources = [e[1], e[2]];

    ecs.run_system(move |mut cmds: Commands| {
        for &s in &sources {
            cmds.entity(s).insert(Follows(target));
        }
    });
    assert_eq!(followed_by(&ecs, target).unwrap().len(), 2, "two followers linked");

    // Despawn the target. LINKED_DESPAWN == false: the non-cascading branch unlinks
    // the sources' Follows (enqueues remove::<Follows>) but does NOT despawn them.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(target).despawn();
    });

    assert!(!ecs.has_entity(target), "target gone");
    for (i, &s) in sources.iter().enumerate() {
        assert!(ecs.has_entity(s), "source {i} SURVIVES (LINKED_DESPAWN off — no cascade)");
        // The non-cascading branch unlinks: each source's Follows is removed.
        assert_eq!(
            ecs.get_component::<Follows>(s).map(|r| r.target()),
            None,
            "source {i}'s Follows is unlinked (non-cascading on_replace removes the FK)",
        );
    }
    assert_eq!(ecs.entity_count(), 2, "both sources survive the non-cascading despawn");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.W3 — install-probe: LikedBy registers EXACTLY on_replace; on_add/on_insert/
//         on_remove UNSET (B7). Likes wires on_insert + on_replace.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likedby_registers_only_on_replace() {
    let likes = get_hooks(Likes::component_id().0).expect("Likes hooks installed");
    assert!(likes.on_insert.is_some(), "Likes wires on_insert (link)");
    assert!(likes.on_replace.is_some(), "Likes wires on_replace (unlink)");
    assert!(likes.on_add.is_none(), "Likes must NOT wire on_add");
    assert!(likes.on_remove.is_none(), "Likes must NOT wire on_remove");

    let liked = get_hooks(LikedBy::component_id().0).expect("LikedBy hooks installed");
    assert!(liked.on_replace.is_some(), "LikedBy wires on_replace (cascade)");
    assert!(liked.on_add.is_none(), "LikedBy must NOT wire on_add (B7)");
    assert!(liked.on_insert.is_none(), "LikedBy must NOT wire on_insert (B7)");
    assert!(liked.on_remove.is_none(), "LikedBy must NOT wire on_remove");
}

// ════════════════════════════════════════════════════════════════════════════
// R2.C2(a) — install-probe: the source derive auto-emits the remap metadata
//            (CloneViaFn + map_entities_fn + SerializeViaFn + non-trivial
//            LAYOUT_FINGERPRINT ≠ 0 and ≠ ChildOf's). The target is Ignore.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_derive_installs_clone_remap_metadata() {
    let likes = get_clone_info(Likes::component_id().0).expect("Likes clone info installed");
    assert_eq!(
        likes.cloneability,
        Cloneability::CloneViaFn,
        "Likes carries an Entity FK ⇒ CloneViaFn (so the deep-clone remap can run, B10)",
    );
    assert!(likes.clone_fn.is_some(), "Likes installs Some(clone_via_clone)");
    assert!(
        get_map_entities_fn(Likes::component_id().0).is_some(),
        "Likes auto-emits its map_entities_fn (the FK clone-remap, B10)",
    );

    let liked = get_clone_info(LikedBy::component_id().0).expect("LikedBy clone info installed");
    assert_eq!(
        liked.cloneability,
        Cloneability::Ignore,
        "LikedBy is the reverse index ⇒ Ignore (rebuilt via Link commands, B12)",
    );
    assert!(liked.clone_fn.is_none(), "LikedBy installs no clone fn");
}

#[test]
fn likes_derive_installs_serialize_remap_metadata() {
    let info = get_serialize_info(Likes::component_id().0).expect("Likes serialize info installed");
    assert_eq!(
        info.serializability,
        Serializability::SerializeViaFn,
        "Likes carries an Entity FK ⇒ SerializeViaFn (the saved id is remapped on load, B11)",
    );
    assert!(info.serialize_fn.is_some(), "Likes installs the WireBridge encoder");
    assert!(info.deserialize_fn.is_some(), "Likes installs the WireBridge decoder");
    assert!(
        info.map_entities_fn.is_some(),
        "Likes installs the load-direction FK remap (B11)",
    );
    assert_ne!(
        info.layout_fingerprint, 0,
        "the fingerprint is computed from Likes's real layout (not zero, not copied)",
    );

    // The Likes fingerprint need not differ from ChildOf's (both are
    // `#[repr(transparent)]` single-Entity newtypes — identical layout), but it
    // must be a real non-zero value. (The plan's "ideally ≠ ChildOf" is layout-
    // dependent; a transparent Entity newtype legitimately matches ChildOf.)
    let child_of = get_serialize_info(ChildOf::component_id().0).expect("ChildOf serialize info");
    let _ = child_of.layout_fingerprint; // observed, not asserted-distinct (same layout)
}

// ════════════════════════════════════════════════════════════════════════════
// R2.C2(b) — clone-remap BEHAVIORAL tripwire (the silent-corruption gate).
//
// The deep clone walks the ChildOf subtree; to make a Likes FK eligible for the
// generic clone-remap, the referenced entity must be IN the clone set. We build a
// ChildOf subtree (parent → child) where the CHILD also `Likes` the PARENT. A deep
// clone of the parent clones both nodes; the cloned child's Likes FK must be
// REMAPPED to the cloned parent (not the original parent, not dangling).
//
// A derive that forgot the clone-remap passes every link/unlink/cascade test but
// FAILS this — the load-bearing C2 tripwire.
//
// TESTER FINDING (BUG-RELATIONS-CLONE-1, see report): this test currently FAILS.
// The deep-clone walk (`clone/deep.rs` + `clone/materialize.rs`) is hard-wired to
// `ChildOf`/`Children`:
//   (1) `materialize::select_clone_ids` denies ONLY `Children` (the literal
//       `children_id`), so a GENERIC `RelationshipTarget` reverse index (here
//       `LikedBy`, `Cloneability::Ignore`) is NOT auto-denied → the clone of the
//       parent (which carries `LikedBy`) trips the `materialize.rs:126`
//       `debug_assert!(false, "skipping non-cloneable component …")`.
//   (2) `deep::clone_subtree`'s remap pass calls `get_map_entities_fn(child_of_id)`
//       ONLY — it does not remap a generic `Likes` FK (the clone-direction
//       `MAP_ENTITIES` slot is never installed for `Likes`; see
//       `likes_derive_installs_clone_remap_metadata`, which also fails on the
//       missing `get_map_entities_fn(Likes)`).
// Net effect: a generic `Relationship`'s FK is NOT entity-remapped on deep clone —
// the silent-corruption this tripwire exists to catch. NOT FIXED here (tester does
// not modify the impl).
// ════════════════════════════════════════════════════════════════════════════

/// Builds `parent → child` (ChildOf) where the child ALSO `Likes(parent)`, all
/// live after one apply window. Returns `(parent, child)`.
fn spawn_parent_child_with_likes(ecs: &mut EcsMaster) -> (Entity, Entity) {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let parent = cmds.spawn(Tag(1)).id();
        let child = cmds.spawn(Tag(2)).id();
        cmds.entity(parent).add_child(child);
        cmds.entity(child).insert(Likes(parent));
        let mut v = probe.lock().expect("probe");
        v.push(parent);
        v.push(child);
    });
    let v = sink.lock().expect("probe").clone();
    (v[0], v[1])
}

#[test]
fn likes_deep_clone_remaps_foreign_key() {
    let mut ecs = EcsMaster::new();
    let (parent, child) = spawn_parent_child_with_likes(&mut ecs);
    assert_eq!(
        likes_of(&ecs, child),
        Some(parent),
        "source child Likes the parent",
    );

    // Deep-clone the parent subtree (parent + child are both cloned).
    let clone_parent = ecs.clone_subtree(parent);
    assert_ne!(clone_parent, parent, "clone parent is a distinct entity");

    // The cloned child is the clone-parent's sole child.
    let clone_children: Vec<Entity> = ecs
        .get_component::<boyko_ecs::ecs::core::hierarchy::Children>(clone_parent)
        .map(|c| c.as_slice().to_vec())
        .expect("the cloned parent has a rebuilt Children index");
    assert_eq!(clone_children.len(), 1, "the cloned parent has one cloned child");
    let clone_child = clone_children[0];
    assert_ne!(clone_child, child, "the cloned child is a distinct entity");

    // THE TRIPWIRE: the cloned child's Likes FK must be REMAPPED to the cloned
    // parent — not the original parent (verbatim), not dangling.
    let cloned_fk = likes_of(&ecs, clone_child);
    assert_eq!(
        cloned_fk,
        Some(clone_parent),
        "C2: the cloned child's Likes FK is REMAPPED to the cloned parent \
         (got {cloned_fk:?}; a verbatim copy would be Some({parent:?}) — silent corruption)",
    );
    // Reverse-index consistency on the clone side: clone_parent.LikedBy ∋ clone_child.
    let clone_likers = liked_by(&ecs, clone_parent).expect("clone parent has LikedBy rebuilt");
    assert!(
        clone_likers.contains(&clone_child),
        "the cloned parent's LikedBy reverse index is consistent with the remapped Likes",
    );

    // The SOURCE is unchanged.
    assert_eq!(likes_of(&ecs, child), Some(parent), "source child still Likes the source parent");
}
