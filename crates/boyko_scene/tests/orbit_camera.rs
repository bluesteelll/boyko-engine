//! S35 camera-rig SCENE matrix (STDLIB-S35-CAMERA-RIG-PLAN §7, `boyko_scene`).
//!
//! Drives the REAL `orbit_camera_system` → `propagate_transforms` →
//! `resolve_active_camera` chain on a live ECS world and asserts:
//!  8. **Eye geometry** — the derived `Transform.translation` for head-on
//!     (`yaw=0,pitch=0`), `yaw=π/2` (eye on +X), and `pitch=+PITCH_LIMIT` (eye
//!     near +Y).
//!  9. **Clamps** — `pitch=π` → clamped to `PITCH_LIMIT` (finite, off the pole);
//!     `distance=0` → `MIN_DISTANCE` (finite, `eye != target`); the rig fields
//!     themselves stay UNCHANGED (read-only rig).
//! 10. **Full pipeline** — rig → Transform → GlobalTransform → ViewUniform
//!     projects `target` to NDC ≈ origin at several poses (head-on + oblique).
//! 11. **C3 no-lag** — a lone-root camera: a frame writes Transform (via the
//!     rig), `propagate_transforms` recomposes the root's `GlobalTransform` to
//!     the JUST-WRITTEN pose (no one-frame lag / no stale identity); a SECOND
//!     pose tracks the new pose (not a one-shot spawn-only recompose).
//!
//! # The frame driver, and the BUG-S35-1 regression guard
//!
//! Tests 8/9 read the rig's `Transform` output DIRECTLY after a bare
//! `run_system(orbit_camera_system)` (no propagation) and PASS — the rig's eye
//! geometry and clamps are correct.
//!
//! Tests 10/11 route the rig THROUGH `propagate_transforms`, driven by a real
//! `Schedule` frame (`App::update` → `Schedule::run`, which bumps the world tick
//! at frame start and promotes each system's `this_run` — the production
//! vehicle `CameraPlugin` builds). They are the permanent regression guard for
//! **BUG-S35-1**: `orbit_camera_system` once wrote `Transform` through a bare
//! `&mut Transform` query term, whose `QueryData` impl declares
//! `NEEDS_CHANGE_DETECTION = false` (`boyko_ecs` data.rs:766-769 — only the
//! `Mut<T>` wrapper stamps the tick). That write updated the Transform VALUE but
//! NEVER advanced its `changed_tick`; `propagate_transforms` is dirty-gated on
//! exactly that tick (propagation.rs:412-414) and so never recomposed the
//! camera's `GlobalTransform`, and the rig never drove the on-screen
//! `ViewUniform` — the core promise of #35. The fix switched the rig to a
//! `Query<(&OrbitCamera, Mut<Transform>)>` writer, whose `DerefMut` stamps the
//! tick so propagation recomposes the same frame. These two tests assert that
//! CORRECT behaviour and run by default so the regression cannot silently
//! return.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;

use boyko_math::{Affine3A, Vec3, Vec4};

use boyko_scene::{
    ActiveCamera, Camera, GlobalTransform, OrbitCamera, Projection, Transform, ViewUniform,
    orbit_camera_system, propagate_transforms, resolve_active_camera,
};

use core::f32::consts::FRAC_PI_2;

/// Element-wise float tolerance for derived poses (a `sin_cos` + a look-at).
const EPS: f32 = 1.0e-4;

#[track_caller]
fn approx(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() <= EPS, "{what}: expected {b}, got {a} (|Δ|={})", (a - b).abs());
}

#[track_caller]
fn vec3_approx(a: Vec3, b: Vec3, what: &str) {
    assert!(
        (a.x - b.x).abs() <= EPS && (a.y - b.y).abs() <= EPS && (a.z - b.z).abs() <= EPS,
        "{what}: expected {b:?}, got {a:?}"
    );
}

/// A fixed per-update delta — keeps `Instant::now` jitter out of the frame
/// driver (the established timed-vehicle discipline from `tests/propagation.rs`).
const FIXED_DELTA: Duration = Duration::from_millis(16);

/// The rig camera's full spawn bundle: the EXPLICIT 5-component list the plan
/// mandates (O2) — selection metadata, projection, the rig, and BOTH pose
/// columns (`propagate_transforms`'s archetype gate needs both present).
#[derive(Bundle)]
struct RigCameraBundle {
    camera: Camera,
    projection: Projection,
    rig: OrbitCamera,
    transform: Transform,
    global: GlobalTransform,
}

/// A perspective projection with a square aspect (so NDC x/y are symmetric).
fn perspective() -> Projection {
    Projection::Perspective {
        fov_y: FRAC_PI_2,
        aspect: 1.0,
        near: 0.1,
        far: 100.0,
    }
}

/// A bare (no-system) App with the camera resources seeded — for tests 8/9,
/// which drive `orbit_camera_system` once via `run_system` and read `Transform`
/// directly (no propagation, no tick dance).
fn rig_world() -> App {
    let mut app = App::new();
    app.insert_resource(ActiveCamera::default());
    app.insert_resource(ViewUniform::default());
    app.finish();
    app
}

/// A FULL-pipeline App for tests 10/11: the camera resources seeded AND the
/// three rig systems registered into the `Main` schedule with the §8 ordering
/// edges (`orbit_camera_system.before(propagate)`,
/// `resolve_active_camera.after(propagate)`). `App::update` then runs one
/// frame at a single promoted world tick — the production vehicle (cf.
/// `CameraPlugin`). NOT finished (the caller spawns first, then `update`).
fn rig_pipeline_world() -> App {
    let mut app = App::new();
    app.insert_resource(ActiveCamera::default());
    app.insert_resource(ViewUniform::default());
    app.add_systems_cfg(|b| {
        let propagate = b.add_system(propagate_transforms).key();
        // The rig must write Transform BEFORE propagation reads it; the resolver
        // reads the freshly-composed GlobalTransform AFTER propagation.
        b.add_system(orbit_camera_system).before(propagate);
        b.add_system(resolve_active_camera).after(propagate);
    });
    app
}

/// Advances the App one full frame (`Schedule::run`: bump world tick, run all
/// registered systems in order at the promoted tick).
#[inline]
fn frame(app: &mut App) {
    app.update_with_delta(FIXED_DELTA);
}

/// Spawns a rig camera with the given `OrbitCamera` and returns its live handle.
fn spawn_rig_camera(world: &mut EcsMaster, rig: OrbitCamera) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(RigCameraBundle {
                camera: Camera::DEFAULT,
                projection: perspective(),
                rig,
                transform: Transform::IDENTITY,
                global: GlobalTransform::IDENTITY,
            })
            .id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    let e = sink.lock().expect("probe lock").expect("spawn produced a handle");
    assert!(world.has_entity(e), "spawned rig camera is live after the apply window");
    e
}

/// Re-poses an existing entity's `OrbitCamera` in place (the §13 windowed-example
/// shape: animation mutates the rig fields through a `Query<&mut OrbitCamera>`).
/// Used to drive a SECOND pose between frames in the C3 test. The rig's own
/// changed-tick is irrelevant to propagation: `orbit_camera_system` is UNGATED
/// (it re-writes every rig's `Transform` each frame), so the next frame's
/// in-schedule rig write — stamped at the promoted frame tick — is what
/// propagation observes.
fn set_rig(world: &mut EcsMaster, target: Entity, rig: OrbitCamera) {
    world.run_system(move |mut q: Query<&mut OrbitCamera>| {
        for (id, r) in q.iter_entities_mut() {
            if id == target.id() {
                *r = rig;
            }
        }
    });
}

/// Reads the entity's local `Transform` (the rig's output).
fn transform_of(world: &EcsMaster, e: Entity) -> Transform {
    *world.get_component::<Transform>(e).expect("entity has Transform")
}

/// Reads the entity's propagated world `Affine3A`.
fn global_of(world: &EcsMaster, e: Entity) -> Affine3A {
    world
        .get_component::<GlobalTransform>(e)
        .expect("entity has GlobalTransform")
        .affine()
}

/// Reads the entity's rig component.
fn rig_of(world: &EcsMaster, e: Entity) -> OrbitCamera {
    *world.get_component::<OrbitCamera>(e).expect("entity has OrbitCamera")
}

/// Reads the derived view resource.
fn view_of(app: &App) -> ViewUniform {
    *app.world().resource::<ViewUniform>()
}

// ════════════════════════════════════════════════════════════════════════════
// 8. Eye geometry
// ════════════════════════════════════════════════════════════════════════════

/// Head-on (`yaw=0, pitch=0, dist=d`): `eye == target + (0, 0, d)`.
#[test]
fn orbit_eye_head_on_is_target_plus_z() {
    let mut app = rig_world();
    let d = 7.0;
    let e = spawn_rig_camera(app.world_mut(), OrbitCamera::new([0.0, 0.0, 0.0], d, 0.0, 0.0));
    app.world_mut().run_system(orbit_camera_system);

    vec3_approx(
        transform_of(app.world(), e).translation,
        Vec3::new(0.0, 0.0, d),
        "yaw=0,pitch=0 → eye = target + (0,0,d)",
    );
}

/// At `yaw = π/2` the eye sweeps onto the +X side of the target (`x ≈ d, z ≈ 0`).
#[test]
fn orbit_eye_quarter_yaw_is_on_plus_x() {
    let mut app = rig_world();
    let d = 5.0;
    let target = Vec3::new(1.0, 2.0, 3.0);
    let e = spawn_rig_camera(
        app.world_mut(),
        OrbitCamera::new([target.x, target.y, target.z], d, FRAC_PI_2, 0.0),
    );
    app.world_mut().run_system(orbit_camera_system);

    let eye = transform_of(app.world(), e).translation;
    approx(eye.x, target.x + d, "yaw=π/2 → eye.x = target.x + d");
    approx(eye.y, target.y, "yaw=π/2 → eye.y = target.y (pitch 0)");
    approx(eye.z, target.z, "yaw=π/2 → eye.z = target.z");
}

/// At `pitch = +PITCH_LIMIT` the eye is raised near the +Y pole of the target:
/// `eye.y ≈ target.y + d·sin(PITCH_LIMIT)` (just under `target.y + d`) and the
/// horizontal radius is `d·cos(PITCH_LIMIT)` (near zero).
#[test]
fn orbit_eye_max_pitch_is_near_plus_y() {
    let mut app = rig_world();
    let d = 4.0;
    let e = spawn_rig_camera(
        app.world_mut(),
        OrbitCamera::new([0.0, 0.0, 0.0], d, 0.0, OrbitCamera::PITCH_LIMIT),
    );
    app.world_mut().run_system(orbit_camera_system);

    let eye = transform_of(app.world(), e).translation;
    let (sp, cp) = OrbitCamera::PITCH_LIMIT.sin_cos();
    approx(eye.y, d * sp, "pitch=PITCH_LIMIT → eye.y = d·sin(limit)");
    // The horizontal offset is d·cos(limit) (tiny but non-zero, off the pole).
    let horiz = (eye.x * eye.x + eye.z * eye.z).sqrt();
    approx(horiz, d * cp, "pitch=PITCH_LIMIT → horizontal radius = d·cos(limit)");
    assert!(eye.y > 0.0 && eye.y < d, "eye.y is near (but below) +d: {}", eye.y);
}

// ════════════════════════════════════════════════════════════════════════════
// 9. Clamps (read-only rig: math clamps, fields untouched)
// ════════════════════════════════════════════════════════════════════════════

/// `pitch = π` (past the +Y pole) is clamped to `+PITCH_LIMIT` for the math —
/// the eye is finite and OFF the singular pole — and the rig field itself is
/// UNCHANGED (`rig.pitch` still `π` after the run).
#[test]
fn orbit_pitch_pi_clamps_for_math_but_leaves_rig_unchanged() {
    let mut app = rig_world();
    let d = 3.0;
    let e = spawn_rig_camera(
        app.world_mut(),
        OrbitCamera::new([0.0, 0.0, 0.0], d, 0.0, core::f32::consts::PI),
    );
    app.world_mut().run_system(orbit_camera_system);

    let eye = transform_of(app.world(), e).translation;
    assert!(eye.is_finite(), "clamped-pitch eye must be finite: {eye:?}");
    // Clamped to +PITCH_LIMIT, so the eye matches that pose, NOT π's eye.
    let (sp, _cp) = OrbitCamera::PITCH_LIMIT.sin_cos();
    approx(eye.y, d * sp, "pitch=π clamps to +PITCH_LIMIT for the eye height");

    // Read-only rig: the stored field is still π.
    approx(rig_of(app.world(), e).pitch, core::f32::consts::PI, "rig.pitch stays π (read-only)");
}

/// `distance = 0` clamps to `MIN_DISTANCE` for the math — `eye != target` and the
/// pose is finite (not NaN) — and the rig field itself stays `0`.
#[test]
fn orbit_zero_distance_clamps_for_math_but_leaves_rig_unchanged() {
    let mut app = rig_world();
    let target = [1.0, 1.0, 1.0];
    let e = spawn_rig_camera(app.world_mut(), OrbitCamera::new(target, 0.0, 0.0, 0.0));
    app.world_mut().run_system(orbit_camera_system);

    let eye = transform_of(app.world(), e).translation;
    let target_v = Vec3::new(target[0], target[1], target[2]);
    assert!(eye.is_finite(), "clamped-distance eye must be finite: {eye:?}");
    let dist = (eye - target_v).length();
    approx(dist, OrbitCamera::MIN_DISTANCE, "distance=0 clamps to MIN_DISTANCE");
    assert!(dist > 0.0, "eye != target after the distance clamp");

    // Read-only rig: the stored field is still 0.
    approx(rig_of(app.world(), e).distance, 0.0, "rig.distance stays 0 (read-only)");
}

// ════════════════════════════════════════════════════════════════════════════
// 10. Full pipeline: rig → Transform → GlobalTransform → ViewUniform
// ════════════════════════════════════════════════════════════════════════════

/// Asserts the resolved `ViewUniform.view_proj` projects `target` to NDC ≈ origin.
#[track_caller]
fn assert_target_at_screen_center(app: &App, target: Vec3, ctx: &str) {
    let view = view_of(app);
    let clip = view.view_proj.mul_vec4(Vec4::from_vec3(target, 1.0));
    assert!(clip.w.abs() > EPS, "{ctx}: target in front of camera (w != 0): w={}", clip.w);
    approx(clip.x / clip.w, 0.0, &format!("{ctx}: target NDC x ≈ 0"));
    approx(clip.y / clip.w, 0.0, &format!("{ctx}: target NDC y ≈ 0"));
}

/// The CORE PROMISE: at several (yaw, pitch) poses the rig drives a view that
/// looks AT the target (target projects to the screen center). Driven through a
/// real `Schedule` frame (`orbit → propagate → resolve`).
///
/// REGRESSION GUARD for **BUG-S35-1**: the rig once wrote `Transform` through a
/// bare `&mut Transform` query term, whose `QueryData` impl declares
/// `NEEDS_CHANGE_DETECTION = false` (`boyko_ecs` data.rs:766-769) — so the write
/// NEVER stamped the row's `changed_tick`. `propagate_transforms` is dirty-gated
/// on exactly that tick (propagation.rs:412-414), so the rig's pose was never
/// folded into `GlobalTransform`/`ViewUniform` and `target` projected to NDC
/// x ≈ −0.25 (the stale identity view), not the screen centre. The fix writes
/// through `Mut<Transform>`, whose `DerefMut` stamps the tick; this test asserts
/// the target now projects to the centre at every pose.
#[test]
fn full_pipeline_looks_at_target_at_multiple_poses() {
    let target = Vec3::new(0.5, -1.0, 2.0);
    let target_arr = [target.x, target.y, target.z];

    // head-on, oblique +, oblique −, a quarter turn.
    let poses = [
        (0.0_f32, 0.0_f32),
        (0.7, 0.4),
        (-1.2, -0.3),
        (FRAC_PI_2, 0.25),
    ];

    for (yaw, pitch) in poses {
        let mut app = rig_pipeline_world();
        let _e = spawn_rig_camera(app.world_mut(), OrbitCamera::new(target_arr, 6.0, yaw, pitch));
        frame(&mut app);
        assert_target_at_screen_center(&app, target, &format!("pose (yaw {yaw}, pitch {pitch})"));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// 11. C3 no-lag propagation (the staleness guard)
// ════════════════════════════════════════════════════════════════════════════

/// A lone-root camera (no parent): a frame writes `Transform` (via the rig) and
/// `propagate_transforms` recomposes the root's `GlobalTransform` to the
/// JUST-WRITTEN pose THE SAME FRAME (no one-frame lag / no stale identity). A
/// SECOND pose (a fresh frame) tracks the new pose, proving the recompose is
/// per-frame, not a one-shot spawn-only event.
///
/// REGRESSION GUARD for **BUG-S35-1**. The recompose is dirty-gated on the
/// `Transform`'s `changed_tick` (propagation.rs:412-414); the rig once wrote
/// `Transform` through a bare `&mut Transform` whose `QueryData` declares
/// `NEEDS_CHANGE_DETECTION = false` (`boyko_ecs` data.rs:766-769), so the row
/// tick was NEVER stamped. Driven through a real `Schedule` frame (where
/// `Schedule::run` promotes each system's `this_run`), the rig's write value
/// landed correctly (`compute_global_transform` returned the right pose) but its
/// `changed_tick` stayed frozen at the spawn tick, so `GlobalTransform` stayed
/// IDENTITY across frames. The fix writes through `Mut<Transform>`, whose
/// `DerefMut` stamps the tick so propagation tracks it; this test asserts the
/// recompose lands the same frame and tracks a second pose.
#[test]
fn c3_lone_root_global_tracks_just_written_transform_no_lag() {
    let mut app = rig_pipeline_world();

    let target = [0.0, 0.0, 0.0];
    let e = spawn_rig_camera(app.world_mut(), OrbitCamera::new(target, 5.0, 0.6, 0.3));

    // ── Pose 1 ───────────────────────────────────────────────────────────────
    frame(&mut app);
    let local1 = transform_of(app.world(), e);

    // The lone root was recomposed THIS frame: global == the just-written affine.
    assert_global_eq_affine(
        global_of(app.world(), e),
        local1.to_affine(),
        "pose 1: GlobalTransform tracks the just-written Transform (no lag)",
    );
    // And it is NOT the stale identity (the failure mode the dirty-gate window
    // would otherwise produce — see the negative-control note).
    assert_ne!(
        global_of(app.world(), e),
        Affine3A::IDENTITY,
        "pose 1: GlobalTransform is the rig pose, not stale identity"
    );

    // ── Pose 2 (a different yaw) ───────────────────────────────────────────────
    // Re-pose the rig, then run a fresh frame: the in-schedule rig write (stamped
    // at the new promoted tick) lands inside the new window, so propagation
    // tracks the SECOND pose — not a one-shot spawn-only recompose.
    set_rig(app.world_mut(), e, OrbitCamera::new(target, 5.0, -1.0, -0.4));
    frame(&mut app);
    let local2 = transform_of(app.world(), e);
    // The pose actually moved (so "tracks pose 2" is a real claim).
    assert!(
        (local2.translation - local1.translation).length() > 0.1,
        "pose 2 must differ from pose 1 (translation moved)"
    );

    assert_global_eq_affine(
        global_of(app.world(), e),
        local2.to_affine(),
        "pose 2: GlobalTransform tracks the SECOND pose (not a one-shot recompose)",
    );
}

/// Per-element affine equality (matrix rows + translation) within `EPS`.
#[track_caller]
fn assert_global_eq_affine(got: Affine3A, want: Affine3A, ctx: &str) {
    for r in 0..3 {
        vec3_approx(got.matrix3.rows[r], want.matrix3.rows[r], &format!("{ctx}: matrix3 row {r}"));
    }
    vec3_approx(got.translation, want.translation, &format!("{ctx}: translation"));
}
