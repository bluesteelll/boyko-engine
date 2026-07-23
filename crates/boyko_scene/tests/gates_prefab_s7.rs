//! Std-lib S7 GATES (REDO, clone-based Prefab) — the integration gates the
//! workflow tester never reached (transient API outage). Authored + run by the
//! orchestrator as the load-bearing verification.
//!
//! The v1 (reverted) design SILENTLY DROPPED `Transform` (a non-`SerPod`
//! component), so [`transform_captured_correct_on_3deep_tree`] is THE gate: it
//! proves the clone-based prefab round-trips the real spatial component. The others
//! prove structure (ChildOf remapped to FRESH ids, `Children` rebuilt), independence
//! (instantiate-N), and source-independence (survives the source subtree despawn).

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` out of the `Send + Sync` one-shot system closure, and the
// file-static `Mutex<()>` guards serialize tests that arm a process-global (allocator /
// propagation counter). Neither is engine code — the whole file is compiled out of every
// shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;
use boyko_math::{Affine3A, Quat, Vec3};

use boyko_scene::{GlobalTransform, Transform, propagate_transforms};

const FIXED_DELTA: Duration = Duration::from_millis(16);

/// The spatial bundle every test entity carries (LOCAL pose + cached WORLD slot).
#[derive(Bundle)]
struct SpatialBundle {
    transform: Transform,
    global: GlobalTransform,
}

#[inline]
fn spatial(transform: Transform) -> SpatialBundle {
    SpatialBundle { transform, global: GlobalTransform::IDENTITY }
}

/// A systemless `App` to advance the world change tick (so `propagate_transforms`
/// observes the freshly-instantiated rows — instances are "Added at instantiate").
fn ticker() -> App {
    let mut app = App::new();
    app.finish();
    app
}

#[inline]
fn advance_tick(app: &mut App) {
    app.update_with_delta(FIXED_DELTA);
}

#[inline]
fn propagate(app: &mut App) {
    app.world_mut().run_system(propagate_transforms);
}

/// Spawns a `g -> p -> c` chain (3-deep) with the given local transforms, applies
/// the `ChildOf` links, and returns `(g, p, c)` — all live.
fn spawn_chain(
    world: &mut EcsMaster,
    gt: Transform,
    pt: Transform,
    ct: Transform,
) -> (Entity, Entity, Entity) {
    let sink: Arc<Mutex<Option<(Entity, Entity, Entity)>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let g = cmds.spawn(spatial(gt)).id();
        let p = cmds.spawn(spatial(pt)).id();
        let c = cmds.spawn(spatial(ct)).id();
        cmds.entity(g).add_child(p);
        cmds.entity(p).add_child(c);
        *probe.lock().expect("probe") = Some((g, p, c));
    });
    let ids = sink.lock().expect("probe").expect("spawn produced handles");
    assert!(world.has_entity(ids.0) && world.has_entity(ids.1) && world.has_entity(ids.2));
    ids
}

fn transform_of(world: &EcsMaster, e: Entity) -> Transform {
    *world.get_component::<Transform>(e).expect("entity has Transform")
}

fn child_of(world: &EcsMaster, e: Entity) -> Option<Entity> {
    world.get_component::<ChildOf>(e).map(|c| c.0)
}

fn children_of(world: &EcsMaster, e: Entity) -> Vec<Entity> {
    world
        .get_component::<Children>(e)
        .map(|c| c.as_slice().to_vec())
        .unwrap_or_default()
}

fn global_of(world: &EcsMaster, e: Entity) -> Affine3A {
    world.get_component::<GlobalTransform>(e).expect("has GlobalTransform").affine()
}

fn assert_transform_eq(got: Transform, want: Transform, ctx: &str) {
    const EPS: f32 = 1e-6;
    let dt = got.translation - want.translation;
    let ds = got.scale - want.scale;
    assert!(
        dt.x.abs() < EPS && dt.y.abs() < EPS && dt.z.abs() < EPS,
        "{ctx}: translation got {:?} want {:?}",
        got.translation,
        want.translation
    );
    assert!(
        ds.x.abs() < EPS && ds.y.abs() < EPS && ds.z.abs() < EPS,
        "{ctx}: scale got {:?} want {:?}",
        got.scale,
        want.scale
    );
    let (gr, wr) = (got.rotation, want.rotation);
    assert!(
        (gr.x - wr.x).abs() < EPS
            && (gr.y - wr.y).abs() < EPS
            && (gr.z - wr.z).abs() < EPS
            && (gr.w - wr.w).abs() < EPS,
        "{ctx}: rotation got {gr:?} want {wr:?}"
    );
}

fn assert_affine_eq(got: Affine3A, want: Affine3A, ctx: &str) {
    const EPS: f32 = 1e-4;
    for r in 0..3 {
        let (g, w) = (got.matrix3.rows[r], want.matrix3.rows[r]);
        assert!(
            (g.x - w.x).abs() < EPS && (g.y - w.y).abs() < EPS && (g.z - w.z).abs() < EPS,
            "{ctx}: matrix3 row {r}: got {g:?} want {w:?}"
        );
    }
    let (gt, wt) = (got.translation, want.translation);
    assert!(
        (gt.x - wt.x).abs() < EPS && (gt.y - wt.y).abs() < EPS && (gt.z - wt.z).abs() < EPS,
        "{ctx}: translation got {gt:?} want {wt:?}"
    );
}

fn quat_z_90() -> Quat {
    use std::f32::consts::FRAC_1_SQRT_2;
    Quat::new(0.0, 0.0, FRAC_1_SQRT_2, FRAC_1_SQRT_2)
}

/// Distinct transforms for the 3 chain levels (translation + rotation + non-uniform
/// scale, so a dropped/garbled capture diverges immediately).
fn chain_transforms() -> (Transform, Transform, Transform) {
    let gt = Transform {
        translation: Vec3::new(1.0, 2.0, 3.0),
        rotation: quat_z_90(),
        scale: Vec3::new(2.0, 3.0, 4.0),
    };
    let pt = Transform {
        translation: Vec3::new(-5.0, 0.5, 10.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::new(1.5, 1.5, 1.5),
    };
    let ct = Transform::from_translation(Vec3::new(0.0, 7.0, -2.0));
    (gt, pt, ct)
}

/// Walks the instantiated `(root, p, c)` chain via `Children`, asserting the
/// structure is a 3-deep chain of FRESH entities with correctly-remapped `ChildOf`.
fn instance_chain(world: &EcsMaster, root: Entity) -> (Entity, Entity) {
    // The instance ROOT is detached (Decision 5): no ChildOf.
    assert!(child_of(world, root).is_none(), "instance root is detached (no ChildOf)");
    let rc = children_of(world, root);
    assert_eq!(rc.len(), 1, "root has exactly one child (p')");
    let ip = rc[0];
    assert_eq!(child_of(world, ip), Some(root), "p'.ChildOf points to the FRESH root");
    let pc = children_of(world, ip);
    assert_eq!(pc.len(), 1, "p' has exactly one child (c')");
    let ic = pc[0];
    assert_eq!(child_of(world, ic), Some(ip), "c'.ChildOf points to the FRESH p'");
    assert!(children_of(world, ic).is_empty(), "c' is a leaf");
    (ip, ic)
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 1 — THE v1-failure gate: Transform is captured + correct on every node.
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn transform_captured_correct_on_3deep_tree() {
    let mut app = ticker();
    advance_tick(&mut app);
    let (gt, pt, ct) = chain_transforms();
    let (g, _p, _c) = spawn_chain(app.world_mut(), gt, pt, ct);

    let prefab = app.world_mut().capture_prefab(g);
    assert_eq!(prefab.node_count(), 3, "captured the whole 3-deep subtree");

    let root = app.world_mut().instantiate(&prefab);
    let (ip, ic) = instance_chain(app.world(), root);

    // THE gate: every instance node carries the source's (non-SerPod) Transform.
    assert_transform_eq(transform_of(app.world(), root), gt, "instance root Transform");
    assert_transform_eq(transform_of(app.world(), ip), pt, "instance p Transform");
    assert_transform_eq(transform_of(app.world(), ic), ct, "instance c Transform");

    // And the rebuilt Children + remapped ChildOf compose to the right world poses.
    advance_tick(&mut app);
    propagate(&mut app);
    let want_root = gt.to_affine(); // detached root => global == local
    let want_p = want_root.mul(pt.to_affine());
    let want_c = want_p.mul(ct.to_affine());
    assert_affine_eq(global_of(app.world(), root), want_root, "instance root global");
    assert_affine_eq(global_of(app.world(), ip), want_p, "instance p global");
    assert_affine_eq(global_of(app.world(), ic), want_c, "instance c global");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 2 — instantiate the SAME prefab twice => two independent trees.
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn instantiate_twice_independent() {
    let mut app = ticker();
    advance_tick(&mut app);
    let (gt, pt, ct) = chain_transforms();
    let (g, p, c) = spawn_chain(app.world_mut(), gt, pt, ct);

    let prefab = app.world_mut().capture_prefab(g);
    let r1 = app.world_mut().instantiate(&prefab);
    let r2 = app.world_mut().instantiate(&prefab);

    assert_ne!(r1, r2, "two distinct instance roots");
    let (ip1, ic1) = instance_chain(app.world(), r1);
    let (ip2, ic2) = instance_chain(app.world(), r2);
    // Every instance entity is fresh + distinct from the source AND the other instance.
    for &e in &[r1, ip1, ic1] {
        assert!(e != g && e != p && e != c, "instance 1 entities are fresh");
    }
    assert!(r1 != r2 && ip1 != ip2 && ic1 != ic2, "the two instances share no entity");

    // Mutating instance 1's leaf Transform must not touch instance 2's.
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(ic1).insert(Transform::from_translation(Vec3::new(99.0, 99.0, 99.0)));
    });
    assert_transform_eq(transform_of(app.world(), ic2), ct, "instance 2 leaf unchanged");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 3 — the prefab is SOURCE-INDEPENDENT: instantiate after despawning the
//          captured source subtree still yields a correct tree.
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn survives_source_despawn() {
    let mut app = ticker();
    advance_tick(&mut app);
    let (gt, pt, ct) = chain_transforms();
    let (g, _p, _c) = spawn_chain(app.world_mut(), gt, pt, ct);

    let prefab = app.world_mut().capture_prefab(g);

    // Despawn the whole captured source subtree (P19 ChildOf cascade from the root).
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(g).despawn();
    });
    assert!(!app.world().has_entity(g), "source root despawned");

    // The frozen prefab OWNS its bytes — instantiate is still correct after the
    // source is gone.
    let root = app.world_mut().instantiate(&prefab);
    let (ip, ic) = instance_chain(app.world(), root);
    assert_transform_eq(transform_of(app.world(), root), gt, "post-despawn root Transform");
    assert_transform_eq(transform_of(app.world(), ip), pt, "post-despawn p Transform");
    assert_transform_eq(transform_of(app.world(), ic), ct, "post-despawn c Transform");
}
