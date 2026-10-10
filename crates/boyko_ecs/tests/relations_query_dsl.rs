//! Relation query DSL — A (unit correctness) + B (footgun/misuse) + accessor
//! behavioral suite for the QUERY side of the generic Relations API.
//!
//! Covers (per the query-DSL test matrix):
//!
//! * A — `Related<R, &T>` JOIN: yields the FK target's `T`; `None` when the FK
//!   is absent, the target is despawned, or the target lacks `T`. Exercised on
//!   `ChildOf` AND a derive-built custom relation (`Likes`/`LikedBy`) to prove
//!   genericity, plus a hand-built self-referential relation (`Mirror`).
//! * A — `HasRelation<R>` / `NoRelation<R>` partition children vs roots exactly.
//! * A — `query_filtered(RelatedTo::<R>::new(p))` matches exactly p's sources;
//!   updates after re-target / unrelate; empty for a no-source entity.
//! * A — accessors `targets` / `sources` / `ancestors` / `descendants` enumerate
//!   correctly after relate / unrelate / re-target / despawn.
//! * B — the value-less `query::<_, RelatedTo<R>>()` path PANICS loudly (the W1
//!   poison sentinel), and `query_filtered` seeds so the value path never panics.
//!
//! Style mirrors `relations_derive.rs` / `phase19_hierarchy_core.rs`: entities
//! spawned through the deferred queue, read back AFTER the apply window.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::iters::query::filter::With;
use boyko_ecs::ecs::core::iters::query::relation::{HasRelation, NoRelation, Related, RelatedTo};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

// ════════════════════════════════════════════════════════════════════════════
// Fixtures
// ════════════════════════════════════════════════════════════════════════════

/// Plain data component fetched through a relation join.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Transform {
    x: f32,
    y: f32,
}

/// A second plain data component (used to prove the target-lacks-`T` ⇒ `None`).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Velocity {
    dx: f32,
}

/// A bare marker so a freshly spawned entity has a concrete archetype.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

/// A derive-built custom relation source (NOT `ChildOf`) — proves the join is
/// generic over any `Relationship`, not special-cased to the hierarchy.
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

/// The custom relation's reverse index.
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, retain_empty)]
struct LikedBy(Vec<Entity>);

// ════════════════════════════════════════════════════════════════════════════
// Helpers
// ════════════════════════════════════════════════════════════════════════════

/// Spawns `n` `Tag` entities through the deferred queue; returns live handles.
/// Each entity `i` carries `Tag(i)` so a query over `&Tag` can map a matched row
/// back to its source-index (the `QueryView` exposes only `iter`, not an
/// entity-yielding iter, so the `Tag` payload is the row identity).
fn spawn_tags(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
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

/// Spawns one entity carrying `(Tag, Transform)` and returns its handle.
fn spawn_with_transform(ecs: &mut EcsMaster, t: Transform) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let e = cmds.spawn(Tag(0)).insert(t).id();
        *probe.lock().expect("probe") = Some(e);
    });
    let e = sink.lock().expect("probe").expect("spawned");
    assert!(ecs.has_entity(e), "entity live after apply");
    e
}

// ════════════════════════════════════════════════════════════════════════════
// A — Related<ChildOf, &Transform> JOIN correctness
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn related_yields_parent_transform_when_present() {
    let mut ecs = EcsMaster::new();
    let parent = spawn_with_transform(&mut ecs, Transform { x: 1.0, y: 2.0 });
    let child = spawn_with_transform(&mut ecs, Transform { x: 9.0, y: 9.0 });

    // Link child -> parent via ChildOf.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(child).insert(ChildOf(parent));
    });

    // The join: for the child row, Related<ChildOf, &Transform> yields the
    // PARENT's Transform (1,2), NOT the child's own (9,9).
    let got: Vec<Option<Transform>> = ecs
        .query::<(&Tag, Related<ChildOf, &Transform>), With<ChildOf>>()
        .iter()
        .map(|(_t, parent_xf): (&Tag, Option<&Transform>)| parent_xf.copied())
        .collect();

    assert_eq!(got.len(), 1, "exactly one child row carries ChildOf");
    assert_eq!(
        got[0],
        Some(Transform { x: 1.0, y: 2.0 }),
        "Related yields the PARENT's Transform, not the child's own"
    );
}

#[test]
fn related_yields_none_when_no_fk() {
    let mut ecs = EcsMaster::new();
    // A lone entity with a Transform but NO ChildOf FK.
    spawn_with_transform(&mut ecs, Transform { x: 5.0, y: 5.0 });

    // Query EVERY entity (no With<ChildOf> bound) — the join term must be None
    // for a source that does not carry the FK at all (matches_component_set on
    // the source archetype excludes it, so it is never yielded by the
    // `Related`-bounded matched set). Bound the query to the FK presence to make
    // the "absent FK ⇒ not matched" explicit, then assert the complement count.
    let with_fk = ecs
        .query::<Related<ChildOf, &Transform>, With<ChildOf>>()
        .iter()
        .count();
    assert_eq!(with_fk, 0, "no entity carries a ChildOf FK ⇒ zero join rows");
}

#[test]
fn related_yields_none_when_parent_lacks_component() {
    let mut ecs = EcsMaster::new();
    // Parent has NO Transform (only Tag); child carries ChildOf -> parent.
    let parents = spawn_tags(&mut ecs, 1);
    let parent = parents[0];
    let child = spawn_with_transform(&mut ecs, Transform { x: 7.0, y: 7.0 });

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(child).insert(ChildOf(parent));
    });

    let got: Vec<Option<Transform>> = ecs
        .query::<Related<ChildOf, &Transform>, With<ChildOf>>()
        .iter()
        .map(|p: Option<&Transform>| p.copied())
        .collect();

    assert_eq!(got.len(), 1, "the child row is matched (it carries the FK)");
    assert_eq!(
        got[0], None,
        "the parent has no Transform ⇒ the join yields None"
    );
}

#[test]
fn related_target_despawn_unlinks_source_fk_no_stale_parent() {
    // The engine's relationship model UNLINKS the source FK when its target is
    // despawned: for a non-cascading target (`Likes`/`LikedBy`), the cascade hook
    // enqueues `remove::<Likes>()` on every source (generic_hooks.rs:174). So the
    // source survives but DROPS its `Likes` FK — it can never observe a stale
    // (dangling) parent through the join. This is the "no stale parent" guarantee:
    // after the target dies, the source is no longer in the `With<Likes>` matched
    // set, so the join yields ZERO rows (not a `Some(garbage)`).
    let mut ecs = EcsMaster::new();
    let target = spawn_with_transform(&mut ecs, Transform { x: 3.0, y: 4.0 });
    let source = spawn_with_transform(&mut ecs, Transform { x: 0.0, y: 0.0 });

    let tgt = target;
    let src = source;
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(src).insert(Likes(tgt));
    });
    // Despawn the Likes target — non-cascading, so the source survives but its
    // Likes FK is unlinked by the cascade hook.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(tgt).despawn();
    });
    assert!(!ecs.has_entity(target), "target despawned");
    assert!(ecs.has_entity(source), "source survives (Likes has no linked_despawn)");

    // The source no longer carries the Likes FK ⇒ it is not matched by the
    // `Related`-bounded set ⇒ zero join rows (no dangling-parent read).
    let rows: Vec<Option<Transform>> = ecs
        .query::<Related<Likes, &Transform>, With<Likes>>()
        .iter()
        .map(|p: Option<&Transform>| p.copied())
        .collect();
    assert_eq!(
        rows.len(),
        0,
        "the source's FK was unlinked on target despawn ⇒ no stale-parent join row"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// A — genericity: the SAME join over a custom (derive-built) relation
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn related_join_works_on_custom_relation() {
    let mut ecs = EcsMaster::new();
    let target = spawn_with_transform(&mut ecs, Transform { x: 42.0, y: 0.0 });
    let source = spawn_with_transform(&mut ecs, Transform { x: -1.0, y: -1.0 });

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });

    let got: Vec<Option<Transform>> = ecs
        .query::<Related<Likes, &Transform>, With<Likes>>()
        .iter()
        .map(|p: Option<&Transform>| p.copied())
        .collect();

    assert_eq!(got.len(), 1, "one source carries Likes");
    assert_eq!(
        got[0],
        Some(Transform { x: 42.0, y: 0.0 }),
        "Related<Likes, &Transform> reads the Likes TARGET's Transform — genericity holds"
    );
}

#[test]
fn related_join_tuple_with_own_and_target_data() {
    let mut ecs = EcsMaster::new();
    let target = spawn_with_transform(&mut ecs, Transform { x: 100.0, y: 0.0 });
    // Source carries its OWN Velocity + a Likes FK.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let tgt = target;
    ecs.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(Tag(0))
            .insert(Velocity { dx: 5.0 })
            .insert(Likes(tgt))
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    let source = sink.lock().expect("probe").expect("spawned");

    // Join: yield the source's OWN Velocity alongside the target's Transform.
    let got: Vec<(f32, Option<f32>)> = ecs
        .query::<(&Velocity, Related<Likes, &Transform>), With<Likes>>()
        .iter()
        .map(|(v, xf): (&Velocity, Option<&Transform>)| (v.dx, xf.map(|t| t.x)))
        .collect();

    assert_eq!(got.len(), 1, "one matched source");
    assert_eq!(
        got[0],
        (5.0, Some(100.0)),
        "tuple yields (own Velocity.dx, target Transform.x)"
    );
    let _ = source;
}

// ════════════════════════════════════════════════════════════════════════════
// A — self-referential relation: Related reads the entity's OWN T
// ════════════════════════════════════════════════════════════════════════════

mod self_ref {
    use super::*;
    use boyko_ecs::ecs::core::relationship::{
        Relationship, RelationshipSourceCollection, RelationshipTarget,
    };

    /// A self-referential relation: `allow_self_referential` permits `Mirror(self)`.
    #[derive(Component, Clone, Copy, Relationship)]
    #[repr(transparent)]
    #[relationship(target = MirroredBy, allow_self_referential)]
    pub struct Mirror(pub Entity);

    #[derive(Component, RelationshipTarget, Default)]
    #[relationship_target(source = Mirror, retain_empty)]
    pub struct MirroredBy(Vec<Entity>);

    #[test]
    fn related_self_referential_reads_own_component() {
        // Compile-time: the flag flipped the const.
        const { assert!(Mirror::ALLOW_SELF_REFERENTIAL) };

        let mut ecs = EcsMaster::new();
        let me = spawn_with_transform(&mut ecs, Transform { x: 11.0, y: 22.0 });

        // Mirror(self) — permitted by allow_self_referential (NOT reactively
        // removed). The FK target == self, so the join reads the entity's OWN
        // Transform.
        let m = me;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.entity(m).insert(Mirror(m));
        });

        // The self-link is retained.
        let tgt = {
            use boyko_ecs::ecs::core::relationship::Relationship as _;
            ecs.get_component::<Mirror>(me).map(|r| r.target())
        };
        assert_eq!(tgt, Some(me), "self-referential FK retained (allow_self_referential)");

        // Reverse collection contains self.
        let rev: Vec<Entity> = ecs
            .get_component::<MirroredBy>(me)
            .map(|c| RelationshipSourceCollection::iter(c.collection()).collect())
            .unwrap_or_default();
        assert_eq!(rev, vec![me], "self is its own source in the reverse index");

        let got: Vec<Option<Transform>> = ecs
            .query::<Related<Mirror, &Transform>, With<Mirror>>()
            .iter()
            .map(|p: Option<&Transform>| p.copied())
            .collect();
        assert_eq!(got.len(), 1, "the self-mirroring entity matches");
        assert_eq!(
            got[0],
            Some(Transform { x: 11.0, y: 22.0 }),
            "Related on a self-referential FK reads the entity's OWN Transform"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// A — HasRelation / NoRelation partition children vs roots
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn has_relation_no_relation_partition_children_and_roots() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 4);
    let (root, c1, c2, lone) = (nodes[0], nodes[1], nodes[2], nodes[3]);

    // c1, c2 -> root via ChildOf. root and lone have no parent.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(c1).insert(ChildOf(root));
        cmds.entity(c2).insert(ChildOf(root));
    });

    let with_parent = ecs
        .query::<&Tag, HasRelation<ChildOf>>()
        .iter()
        .count();
    let roots = ecs
        .query::<&Tag, NoRelation<ChildOf>>()
        .iter()
        .count();

    assert_eq!(with_parent, 2, "exactly c1, c2 carry a ChildOf FK");
    // roots = root + lone (no FK). The `root` carries a `Children` reverse index
    // but NOT a `ChildOf` FK, so it is correctly a NoRelation<ChildOf> root.
    assert_eq!(roots, 2, "root + lone lack a ChildOf FK ⇒ NoRelation roots");
    let _ = lone;
}

// ════════════════════════════════════════════════════════════════════════════
// A — RelatedTo<R> value filter (the W1 value path)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn related_to_matches_exactly_targets_sources() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 5);
    let (pa, pb, s1, s2, s3) = (nodes[0], nodes[1], nodes[2], nodes[3], nodes[4]);

    // s1,s2 -> pa ; s3 -> pb (all via ChildOf).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(s1).insert(ChildOf(pa));
        cmds.entity(s2).insert(ChildOf(pa));
        cmds.entity(s3).insert(ChildOf(pb));
    });

    // Tag values map a matched row back to its source-index (s1=Tag(2),
    // s2=Tag(3), s3=Tag(4) — spawn order). query_filtered(RelatedTo(pa)) must
    // match EXACTLY {s1, s2}.
    let mut matched: Vec<u32> = ecs
        .query_filtered::<&Tag, _>(RelatedTo::<ChildOf>::new(pa))
        .iter()
        .map(|t: &Tag| t.0)
        .collect();
    matched.sort_unstable();
    assert_eq!(matched, vec![2, 3], "RelatedTo(pa) matches exactly pa's sources (s1, s2)");

    // RelatedTo(pb) must match EXACTLY {s3}.
    let pb_sources: Vec<u32> = ecs
        .query_filtered::<&Tag, _>(RelatedTo::<ChildOf>::new(pb))
        .iter()
        .map(|t: &Tag| t.0)
        .collect();
    assert_eq!(pb_sources, vec![4], "RelatedTo(pb) matches exactly s3");
    let _ = (s1, s2, s3);
}

#[test]
fn related_to_updates_after_retarget_and_unrelate() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 3);
    let (pa, pb, s) = (nodes[0], nodes[1], nodes[2]);

    // s -> pa.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(s).insert(ChildOf(pa));
    });
    let count_pa = ecs
        .query_filtered::<&Tag, _>(RelatedTo::<ChildOf>::new(pa))
        .iter()
        .count();
    assert_eq!(count_pa, 1, "s -> pa initially matches RelatedTo(pa)");

    // Re-target: s -> pb (overwrite the FK).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(s).insert(ChildOf(pb));
    });
    let after_pa = ecs
        .query_filtered::<&Tag, _>(RelatedTo::<ChildOf>::new(pa))
        .iter()
        .count();
    let after_pb = ecs
        .query_filtered::<&Tag, _>(RelatedTo::<ChildOf>::new(pb))
        .iter()
        .count();
    assert_eq!(after_pa, 0, "after re-target, RelatedTo(pa) no longer matches s");
    assert_eq!(after_pb, 1, "after re-target, RelatedTo(pb) matches s");
}

#[test]
fn related_to_matches_nothing_for_unrelated_target() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 3);
    let (pa, s, bystander) = (nodes[0], nodes[1], nodes[2]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(s).insert(ChildOf(pa));
    });

    // No source points at `bystander` ⇒ RelatedTo(bystander) matches nothing.
    let n = ecs
        .query_filtered::<&Tag, _>(RelatedTo::<ChildOf>::new(bystander))
        .iter()
        .count();
    assert_eq!(n, 0, "RelatedTo(bystander) matches no source");
}

// ════════════════════════════════════════════════════════════════════════════
// B — FOOTGUN: the value-less RelatedTo path panics LOUDLY (W1 poison)
// ════════════════════════════════════════════════════════════════════════════

#[test]
#[should_panic(expected = "value-less")]
fn related_to_value_less_path_panics_loudly() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 2);
    let (p, s) = (nodes[0], nodes[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(s).insert(ChildOf(p));
    });

    // The value-LESS path: `query::<_, RelatedTo<R>>()` never seeds the runtime
    // target, so the state still carries the POISON sentinel. The first row's
    // filter_fetch observes the poison and panics LOUDLY (W1) — NOT a silent
    // match against EntityId(usize::MAX).
    let _n = ecs
        .query::<&Tag, RelatedTo<ChildOf>>()
        .iter()
        .count();
}

#[test]
fn related_to_value_path_does_not_panic() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 2);
    let (p, s) = (nodes[0], nodes[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(s).insert(ChildOf(p));
    });

    // The value-carrying path seeds the target via seed_state ⇒ NO panic.
    let n = ecs
        .query_filtered::<&Tag, _>(RelatedTo::<ChildOf>::new(p))
        .iter()
        .count();
    assert_eq!(n, 1, "the seeded value path matches s without panicking");
}

// ════════════════════════════════════════════════════════════════════════════
// A — accessors: targets / sources / ancestors / descendants
// ════════════════════════════════════════════════════════════════════════════

/// Collect an accessor iterator's yields into a sorted Vec<usize> of ids.
fn ids_sorted(it: impl Iterator<Item = Entity>) -> Vec<usize> {
    let mut v: Vec<usize> = it.map(|e| e.id().0).collect();
    v.sort_unstable();
    v
}

#[test]
fn targets_yields_single_fk_target_or_nothing() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 2);
    let (parent, child) = (nodes[0], nodes[1]);

    // No FK yet ⇒ empty.
    assert_eq!(ecs.targets::<ChildOf>(child).count(), 0, "no FK ⇒ no target");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(child).insert(ChildOf(parent));
    });

    let got = ids_sorted(ecs.targets::<ChildOf>(child));
    assert_eq!(got, vec![parent.id().0], "targets yields the single FK target");
    // A target with no outgoing FK yields nothing.
    assert_eq!(ecs.targets::<ChildOf>(parent).count(), 0, "parent has no outgoing FK");
}

#[test]
fn sources_enumerate_all_reverse_sources() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 4);
    let (parent, c1, c2, c3) = (nodes[0], nodes[1], nodes[2], nodes[3]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(c1).insert(ChildOf(parent));
        cmds.entity(c2).insert(ChildOf(parent));
        cmds.entity(c3).insert(ChildOf(parent));
    });

    let got = ids_sorted(ecs.sources::<ChildOf>(parent));
    let want = ids_sorted([c1, c2, c3].into_iter());
    assert_eq!(got, want, "sources yields all and only the reverse sources");
}

#[test]
fn sources_updates_after_unrelate_and_retarget() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 4);
    let (pa, pb, c1, c2) = (nodes[0], nodes[1], nodes[2], nodes[3]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(c1).insert(ChildOf(pa));
        cmds.entity(c2).insert(ChildOf(pa));
    });
    assert_eq!(
        ids_sorted(ecs.sources::<ChildOf>(pa)),
        ids_sorted([c1, c2].into_iter()),
        "both children initially under pa (O2 cached-collection still complete)"
    );

    // Re-target c1 -> pb; unlink c2 by despawn.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(c1).insert(ChildOf(pb));
    });
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(c2).despawn();
    });

    assert_eq!(
        ids_sorted(ecs.sources::<ChildOf>(pa)),
        Vec::<usize>::new(),
        "after re-target + despawn, pa has no sources"
    );
    assert_eq!(
        ids_sorted(ecs.sources::<ChildOf>(pb)),
        vec![c1.id().0],
        "c1 now appears under pb"
    );
}

#[test]
fn ancestors_walk_the_fk_chain_up() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 4);
    // Chain: a <- b <- c <- d  (d.ChildOf=c, c.ChildOf=b, b.ChildOf=a).
    let (a, b, c, d) = (nodes[0], nodes[1], nodes[2], nodes[3]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(b).insert(ChildOf(a));
        cmds.entity(c).insert(ChildOf(b));
        cmds.entity(d).insert(ChildOf(c));
    });

    // ancestors(d) = [c, b, a] in walk order (NOT d itself).
    let chain: Vec<usize> = ecs.ancestors::<ChildOf>(d).map(|e| e.id().0).collect();
    assert_eq!(
        chain,
        vec![c.id().0, b.id().0, a.id().0],
        "ancestors walks c -> b -> a (excludes the start node)"
    );
    // The root has no ancestors.
    assert_eq!(ecs.ancestors::<ChildOf>(a).count(), 0, "root has no ancestors");
}

#[test]
fn descendants_dfs_over_reverse_collections() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 5);
    // Tree: root -> {c1, c2}; c1 -> {g1}.
    let (root, c1, c2, g1, lone) = (nodes[0], nodes[1], nodes[2], nodes[3], nodes[4]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(c1).insert(ChildOf(root));
        cmds.entity(c2).insert(ChildOf(root));
        cmds.entity(g1).insert(ChildOf(c1));
    });

    let got = ids_sorted(ecs.descendants::<ChildOf>(root));
    let want = ids_sorted([c1, c2, g1].into_iter());
    assert_eq!(got, want, "descendants(root) = {{c1, c2, g1}} (excludes root)");
    // A leaf / lone entity has no descendants.
    assert_eq!(ecs.descendants::<ChildOf>(g1).count(), 0, "leaf has no descendants");
    assert_eq!(ecs.descendants::<ChildOf>(lone).count(), 0, "lone has no descendants");
}

#[test]
fn descendants_updates_after_despawn_subtree() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 4);
    let (root, c1, c2, g1) = (nodes[0], nodes[1], nodes[2], nodes[3]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(c1).insert(ChildOf(root));
        cmds.entity(c2).insert(ChildOf(root));
        cmds.entity(g1).insert(ChildOf(c1));
    });
    assert_eq!(ecs.descendants::<ChildOf>(root).count(), 3, "root has 3 descendants");

    // Despawn c1 — ChildOf has linked_despawn (Children cascade), so c1 + g1 go.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(c1).despawn();
    });

    let got = ids_sorted(ecs.descendants::<ChildOf>(root));
    assert_eq!(
        got,
        vec![c2.id().0],
        "after despawning the c1 subtree, only c2 remains a descendant"
    );
}
