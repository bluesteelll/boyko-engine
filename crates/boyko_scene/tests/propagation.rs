//! S2 propagation GATES (integration).
//!
//! Builds real multi-level `ChildOf`/`Children` trees through the command API and
//! asserts each entity's [`GlobalTransform`] against hand-computed affines, the
//! two-implementation property (`compute_global_transform` == propagated value),
//! the structural-change (reparent / first-attach) dirtiness path, and the
//! still-frame 0%-gate (zero affine composes when nothing changed).
//!
//! # The deterministic frame vehicle
//!
//! `propagate_transforms` is dirty-gated against a `(last_run, this_run]` change
//! window, so the world's change tick must advance between propagation runs.
//! `EcsMaster::bump_change_tick` is `pub(crate)` (not callable here), so the tick
//! is advanced via a **systemless `App`** ([`advance_tick`]): each `App::update`
//! bumps the world's change tick without running any user system, giving precise
//! control over the window. Spawns / mutations and the propagation itself all run
//! through `world.run_system(...)` at the advanced (frozen-between-updates) tick:
//! a mutation stamps its component change tick at the current counter, then
//! [`propagate`] runs at the SAME counter, and because the propagation's
//! `last_run` baseline was frozen at a strictly-earlier `advance_tick`, the
//! mutation falls inside `(last_run, this_run]` and is observed exactly once.
//!
//! # The Miri descent target
//!
//! [`descent_two_deep_matches_hand_computed`] is the Miri-TB target: it runs the
//! parent-read / child-write raw-pointer descent on a real 2-deep tree, so the
//! descent's `unsafe` (`read_global` parent-copy + `write_global` child-write) is
//! fully exercised. Run under tree-borrows with `-Zmiri-ignore-leaks`:
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-scene descent_two_deep_matches_hand_computed
//! ```
//! `-Zmiri-ignore-leaks` isolates the Tree-Borrows signal from the `App` →
//! `ThreadPool` `Arc` teardown leak — an allocator artifact orthogonal to TB,
//! exactly as the kernel's threadpool-bearing Miri suites do (`miri_phase19`).

use std::f32::consts::FRAC_1_SQRT_2;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;
use boyko_math::{Affine3A, Quat, Vec3};

use boyko_scene::propagation::compute_global_transform;
use boyko_scene::{GlobalTransform, Transform, propagate_transforms};

#[cfg(debug_assertions)]
use std::sync::atomic::Ordering;

/// A fixed per-update delta — keeps `Instant::now` jitter out of the tick-advance
/// loop (the established timed-vehicle discipline).
const FIXED_DELTA: Duration = Duration::from_millis(16);

/// Binary-local serialization for the PROCESS-GLOBAL `STILL_FRAME_COMPOSES`
/// counter. `propagate_transforms` `store(0)`s that `static` on entry and
/// `fetch_add`s during the descent, so two propagation runs in flight at once on
/// different `cargo test` threads corrupt each other's count. `cargo test` runs
/// the tests in THIS binary in parallel, so the counter-reading test
/// ([`still_frame_does_zero_affine_composes`]) must not run concurrently with any
/// other `propagate` caller. Every non-measuring test wraps its `propagate` in
/// [`propagate`] (which takes this lock for the call), and the measuring test
/// holds the lock for its whole body — exactly the `ARM_LOCK` discipline the
/// sibling `gates_zero_overhead_alloc` binary uses for its global alloc counter.
static PROPAGATE_LOCK: Mutex<()> = Mutex::new(());

#[inline]
fn lock_propagate() -> std::sync::MutexGuard<'static, ()> {
    PROPAGATE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The spatial bundle every test entity carries: the LOCAL pose + the cached
/// WORLD pose slot the propagation fills in.
#[derive(Bundle)]
struct SpatialBundle {
    transform: Transform,
    global: GlobalTransform,
}

#[inline]
fn spatial(transform: Transform) -> SpatialBundle {
    SpatialBundle {
        transform,
        global: GlobalTransform::IDENTITY,
    }
}

/// A systemless `App` whose sole purpose is to advance the world change tick on
/// demand (no user systems run, so the propagation's window is fully controlled
/// by the test). One [`advance_tick`] before the first mutation lifts the counter
/// off the `Tick::ZERO` sentinel (which the `(last_run, this_run]` lower bound
/// excludes).
fn ticker() -> App {
    let mut app = App::new();
    app.finish();
    app
}

/// Advances the world change tick by one `App::update` (no user systems run).
#[inline]
fn advance_tick(app: &mut App) {
    app.update_with_delta(FIXED_DELTA);
}

/// Runs `propagate_transforms` once on the app's world at the current tick,
/// serialized against the counter-reading test via [`PROPAGATE_LOCK`] so a
/// concurrent run never corrupts that test's `STILL_FRAME_COMPOSES` measurement.
/// The lock is held only for the single call (released on return), so the
/// measuring test — which holds the lock across its WHOLE body — never contends
/// with a partial measurement here.
#[inline]
fn propagate(app: &mut App) {
    let _guard = lock_propagate();
    app.world_mut().run_system(propagate_transforms);
}

/// Spawns one spatial entity with `transform` and returns its (now-live) handle.
fn spawn_spatial(world: &mut EcsMaster, transform: Transform) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds.spawn(spatial(transform)).id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    let e = sink.lock().expect("probe lock").expect("spawn produced a handle");
    assert!(world.has_entity(e), "spawned entity is live after the apply window");
    e
}

/// Reads an entity's propagated world affine.
fn global_of(world: &EcsMaster, e: Entity) -> Affine3A {
    world
        .get_component::<GlobalTransform>(e)
        .expect("entity has GlobalTransform")
        .affine()
}

/// Asserts two affines agree within an absolute float epsilon (per element). The
/// propagation and the hand-computed reference both use the same scalar
/// `from_translation_rotation_scale` + `mul`, so the only slack is f32 rounding.
fn assert_affine_eq(got: Affine3A, want: Affine3A, ctx: &str) {
    const EPS: f32 = 1e-4;
    for r in 0..3 {
        let g = got.matrix3.rows[r];
        let w = want.matrix3.rows[r];
        assert!(
            (g.x - w.x).abs() < EPS && (g.y - w.y).abs() < EPS && (g.z - w.z).abs() < EPS,
            "{ctx}: matrix3 row {r} mismatch: got {g:?} want {w:?}"
        );
    }
    let gt = got.translation;
    let wt = want.translation;
    assert!(
        (gt.x - wt.x).abs() < EPS && (gt.y - wt.y).abs() < EPS && (gt.z - wt.z).abs() < EPS,
        "{ctx}: translation mismatch: got {gt:?} want {wt:?}"
    );
}

/// A 90°-about-Z unit quaternion (`sin45 = cos45 = 1/√2`).
fn quat_z_90() -> Quat {
    Quat::new(0.0, 0.0, FRAC_1_SQRT_2, FRAC_1_SQRT_2)
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 0 — a FRESH-WORLD spawn at world tick 0 composes on the FIRST run
// ════════════════════════════════════════════════════════════════════════════

/// Regression for the R3 room-camera bug — the exact gap the sibling gates
/// masked: every other gate calls [`advance_tick`] BEFORE its first spawn
/// ("lift the tick off the `Tick::ZERO` sentinel"), so no row in this suite was
/// ever stamped at world tick 0 — yet that is precisely where an `App` startup
/// system's `Commands` spawn lands. With the propagation `last_run` baseline at
/// literal `Tick::ZERO`, the `(last_run, this_run]` window's EXCLUSIVE lower
/// bound hid those rows forever: a startup-spawned camera kept its identity
/// `GlobalTransform` and the room rendered from the origin. This gate spawns
/// WITHOUT the tick warm-up and requires the FIRST propagate run to compose the
/// pose (the TICK8 never-run baseline, `current_tick - MAX_CHANGE_AGE`).
#[test]
fn fresh_world_tick_zero_spawn_composes_on_first_run() {
    let mut app = ticker(); // NO advance_tick: the spawn stamps at world tick 0.
    let t = Transform {
        translation: Vec3::new(0.0, 1.7, 6.0),
        rotation: quat_z_90(),
        scale: Vec3::ONE,
    };
    let e = spawn_spatial(app.world_mut(), t);
    propagate(&mut app);
    assert_affine_eq(global_of(app.world(), e), t.to_affine(), "tick-0 root spawn");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 1 — identity Transform → identity GlobalTransform
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn identity_transform_yields_identity_global() {
    let mut app = ticker();
    advance_tick(&mut app);
    let e = spawn_spatial(app.world_mut(), Transform::IDENTITY);
    propagate(&mut app);
    assert_affine_eq(global_of(app.world(), e), Affine3A::IDENTITY, "identity root");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 2 — a lone root composes to its own to_affine()
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn root_composition_equals_to_affine() {
    let mut app = ticker();
    advance_tick(&mut app);
    let t = Transform {
        translation: Vec3::new(1.0, 2.0, 3.0),
        rotation: quat_z_90(),
        scale: Vec3::new(2.0, 3.0, 4.0),
    };
    let e = spawn_spatial(app.world_mut(), t);
    propagate(&mut app);
    assert_affine_eq(global_of(app.world(), e), t.to_affine(), "root == to_affine");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 3 — Without<ChildOf> root path: an unparented entity is a root
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn unparented_entity_is_a_root() {
    let mut app = ticker();
    advance_tick(&mut app);
    let t = Transform::from_translation(Vec3::new(5.0, -7.0, 0.5));
    let e = spawn_spatial(app.world_mut(), t);
    propagate(&mut app);
    // No ChildOf ⇒ the root branch runs ⇒ global == local affine.
    assert_affine_eq(global_of(app.world(), e), t.to_affine(), "Without<ChildOf> root");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 4 — 2-deep chain == hand-computed parent ∘ child (the Miri descent target)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn descent_two_deep_matches_hand_computed() {
    let mut app = ticker();
    advance_tick(&mut app);
    let pt = Transform {
        translation: Vec3::new(10.0, 0.0, 0.0),
        rotation: quat_z_90(),
        scale: Vec3::ONE,
    };
    let ct = Transform {
        translation: Vec3::new(0.0, 4.0, 0.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };
    let parent = spawn_spatial(app.world_mut(), pt);
    let child = spawn_spatial(app.world_mut(), ct);

    let (p, c) = (parent, child);
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(p).add_child(c);
    });

    propagate(&mut app);

    // Hand-computed reference: child.global = parent.affine ∘ child.affine.
    let want_parent = pt.to_affine();
    let want_child = want_parent.mul(ct.to_affine());
    assert_affine_eq(global_of(app.world(), parent), want_parent, "2-deep parent");
    assert_affine_eq(global_of(app.world(), child), want_child, "2-deep child");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 5 — 3-deep chain == hand-computed grandparent ∘ parent ∘ child
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn descent_three_deep_matches_hand_computed() {
    let mut app = ticker();
    advance_tick(&mut app);
    let gt = Transform {
        translation: Vec3::new(1.0, 0.0, 0.0),
        rotation: quat_z_90(),
        scale: Vec3::new(2.0, 2.0, 2.0),
    };
    let pt = Transform {
        translation: Vec3::new(0.0, 3.0, 0.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };
    let ct = Transform::from_translation(Vec3::new(0.0, 0.0, 5.0));

    let g = spawn_spatial(app.world_mut(), gt);
    let p = spawn_spatial(app.world_mut(), pt);
    let c = spawn_spatial(app.world_mut(), ct);

    let (gg, pp, cc) = (g, p, c);
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(gg).add_child(pp);
        cmds.entity(pp).add_child(cc);
    });

    propagate(&mut app);

    let want_g = gt.to_affine();
    let want_p = want_g.mul(pt.to_affine());
    let want_c = want_p.mul(ct.to_affine());
    assert_affine_eq(global_of(app.world(), g), want_g, "3-deep grandparent");
    assert_affine_eq(global_of(app.world(), p), want_p, "3-deep parent");
    assert_affine_eq(global_of(app.world(), c), want_c, "3-deep child");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 6 — non-uniform parent scale × rotated child ⇒ correct SHEAR
//          (the affine-not-TRS test, transpose-sensitive)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn nonuniform_scale_times_rotated_child_produces_shear() {
    let mut app = ticker();
    advance_tick(&mut app);
    // Parent: non-uniform scale, no rotation.
    let pt = Transform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::new(2.0, 1.0, 1.0),
    };
    // Child: rotated 90° about Z, unit scale. parent.scale ∘ child.rotation is a
    // SHEAR: it is NOT expressible as any single T·R·S, so a wrong (transposed)
    // matrix product would diverge here.
    let ct = Transform {
        translation: Vec3::ZERO,
        rotation: quat_z_90(),
        scale: Vec3::ONE,
    };

    let parent = spawn_spatial(app.world_mut(), pt);
    let child = spawn_spatial(app.world_mut(), ct);
    let (p, c) = (parent, child);
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(p).add_child(c);
    });

    propagate(&mut app);

    let want_child = pt.to_affine().mul(ct.to_affine());
    let got = global_of(app.world(), child);
    assert_affine_eq(got, want_child, "shear child");

    // Sanity: the result is a genuine shear that exposes the NON-UNIFORM scale
    // applied AFTER the child rotation. The child's 90°-Z rotation maps the unit
    // Y basis to -X, and the parent's scale(2,1,1) then stretches that by 2 along
    // X — so Y maps to (-2, 0, 0). A naive (transposed / scale-before-rotate)
    // product would instead leave Y at unit length, diverging here.
    let y_image = got.transform_vector(Vec3::new(0.0, 1.0, 0.0));
    assert!(
        (y_image.x.abs() - 2.0).abs() < 1e-4 && y_image.y.abs() < 1e-4,
        "shear: Y basis should map to (±2, 0, 0), got {y_image:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 7 — reparent updates next run, even with an UNCHANGED local Transform
//          (the structural-change dirtiness gate — finding #2)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn reparent_with_unchanged_transform_updates_next_run() {
    let mut app = ticker();
    advance_tick(&mut app);
    let a_t = Transform::from_translation(Vec3::new(100.0, 0.0, 0.0));
    let b_t = Transform::from_translation(Vec3::new(0.0, 200.0, 0.0));
    let child_t = Transform::from_translation(Vec3::new(1.0, 1.0, 1.0));

    let a = spawn_spatial(app.world_mut(), a_t);
    let b = spawn_spatial(app.world_mut(), b_t);
    let child = spawn_spatial(app.world_mut(), child_t);

    // First-attach child to A. (`add_child` migrate-inserts `ChildOf` on the
    // child, stamping its `ChildOf` changed tick — the structural dirtiness the
    // local Transform tick does NOT carry.)
    let (aa, cc) = (a, child);
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(aa).add_child(cc);
    });
    propagate(&mut app);
    assert_affine_eq(
        global_of(app.world(), child),
        a_t.to_affine().mul(child_t.to_affine()),
        "child under A after first-attach",
    );

    // Reparent child A→B WITHOUT writing child's local Transform. `set_parent`
    // overwrites `ChildOf` in place, stamping the child's `ChildOf` changed tick
    // while leaving its local Transform tick untouched. The Changed<ChildOf> leg
    // of the dirty scan must flag the child so its GlobalTransform recomposes
    // from B on the next run.
    advance_tick(&mut app);
    let (bb, cc2) = (b, child);
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(cc2).set_parent(bb);
    });
    propagate(&mut app);

    assert_affine_eq(
        global_of(app.world(), child),
        b_t.to_affine().mul(child_t.to_affine()),
        "child under B after reparent (unchanged local Transform)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 8 — property: compute_global_transform == propagated value, for EVERY
//          entity in a random-ish tree (order-independent, values only)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn property_compute_equals_propagated() {
    let mut app = ticker();
    advance_tick(&mut app);

    // A deterministic pseudo-random forest: 40 entities, each (after the first
    // few roots) parented to an earlier node, with varied local transforms.
    let n = 40usize;
    let mut transforms = Vec::with_capacity(n);
    let mut state = 0x9e3779b97f4a7c15u64;
    let mut next = || {
        // splitmix64
        state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    };
    let mut f = |lo: f32, hi: f32| {
        let u = (next() >> 11) as f32 / (1u64 << 53) as f32;
        lo + u * (hi - lo)
    };

    for _ in 0..n {
        let q = Quat::new(f(-1.0, 1.0), f(-1.0, 1.0), f(-1.0, 1.0), f(-1.0, 1.0)).normalize();
        transforms.push(Transform {
            translation: Vec3::new(f(-5.0, 5.0), f(-5.0, 5.0), f(-5.0, 5.0)),
            rotation: q,
            scale: Vec3::new(f(0.5, 2.0), f(0.5, 2.0), f(0.5, 2.0)),
        });
    }

    let mut entities = Vec::with_capacity(n);
    for &t in &transforms {
        entities.push(spawn_spatial(app.world_mut(), t));
    }

    // Parent each node (after the first 3 roots) to a uniformly-earlier node.
    let parents: Vec<Option<usize>> = (0..n)
        .map(|i| if i < 3 { None } else { Some((next() as usize) % i) })
        .collect();
    let ents = entities.clone();
    let par = parents.clone();
    app.world_mut().run_system(move |mut cmds: Commands| {
        for i in 0..n {
            if let Some(p) = par[i] {
                cmds.entity(ents[p]).add_child(ents[i]);
            }
        }
    });

    propagate(&mut app);

    for (i, &e) in entities.iter().enumerate() {
        let propagated = global_of(app.world(), e);
        let reference = compute_global_transform(app.world(), e);
        assert_affine_eq(propagated, reference, &format!("property entity {i}"));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 9 — still-frame 0%-gate: zero affine composes when nothing changed
//          (debug-only counter; finding #4's compose-side assertion)
// ════════════════════════════════════════════════════════════════════════════

#[cfg(debug_assertions)]
#[test]
fn still_frame_does_zero_affine_composes() {
    use boyko_scene::propagation::STILL_FRAME_COMPOSES;

    // Hold `PROPAGATE_LOCK` for the WHOLE body: every `store(0)` / `fetch_add` on
    // the process-global `STILL_FRAME_COMPOSES` from this test, and every read of
    // it, must be exclusive of any sibling test's `propagate` (which also touches
    // the counter). The non-measuring tests take this lock per-call via
    // [`propagate`]; this test holds it across all three runs, so it never calls
    // that (would-deadlock) helper — it drives propagation directly under the lock.
    let _measure = lock_propagate();
    let propagate_direct = |app: &mut App| app.world_mut().run_system(propagate_transforms);

    let mut app = ticker();
    advance_tick(&mut app);
    let g = spawn_spatial(app.world_mut(), Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)));
    let p = spawn_spatial(app.world_mut(), Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)));
    let c = spawn_spatial(app.world_mut(), Transform::from_translation(Vec3::new(0.0, 0.0, 1.0)));
    let (gg, pp, cc) = (g, p, c);
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(gg).add_child(pp);
        cmds.entity(pp).add_child(cc);
    });

    // Run 1: everything is dirty (first run) — composes happen.
    propagate_direct(&mut app);
    assert!(
        STILL_FRAME_COMPOSES.load(Ordering::Relaxed) > 0,
        "first run composes the whole tree"
    );

    // Run 2: tick advanced but nothing changed — the dirty scan finds no newer
    // tick, so ZERO affine composes occur (the still-frame 0%-gate).
    advance_tick(&mut app);
    propagate_direct(&mut app);
    assert_eq!(
        STILL_FRAME_COMPOSES.load(Ordering::Relaxed),
        0,
        "a fully-static frame performs zero affine composes"
    );

    // Run 3: touch the grandparent's local Transform; ONLY its subtree
    // recomposes (3 nodes), proving the dirty gate is value-driven.
    advance_tick(&mut app);
    let gg2 = g;
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(gg2)
            .insert(Transform::from_translation(Vec3::new(9.0, 0.0, 0.0)));
    });
    propagate_direct(&mut app);
    assert_eq!(
        STILL_FRAME_COMPOSES.load(Ordering::Relaxed),
        3,
        "moving the grandparent recomposes exactly its 3-node subtree"
    );
}
