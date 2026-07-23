//! R6 fly-camera matrix (`boyko_scene`, host plan R6).
//!
//! Drives the REAL `fly_camera_system` → `propagate_transforms` →
//! `resolve_active_camera` chain on a live ECS world over a SYNTHETIC
//! [`PhysicalInput`] snapshot (no OS, no GPU) and asserts:
//!
//!  2. **FlyCamera moves Transform** — a seeded `W` + mouse-X snapshot moves the
//!     eye along the rotated forward, rotates yaw by `mouse.x · sensitivity`,
//!     drives the resolved `ViewUniform`, AND advances the `Transform`'s
//!     `changed_tick` (the `Mut<Transform>` contract — the untracked-`&mut` trap).
//!  4. **Parity trig** — `forward(yaw=0, pitch=0) == [0, 0, -1]`; the basis
//!     matches the reference formula at several angles; the pitch clamps at
//!     `±FlyCamera::PITCH_LIMIT`.
//!
//! The alloc-free gate for the pure fly step (Test 6, fly half) is co-located in
//! `boyko_scene::camera`'s own test module (it reaches the crate-private
//! `fly_step` without the scheduler's per-run scaffolding).

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` out of the `Send + Sync` one-shot system closure, and the
// file-static `Mutex<()>` guards serialize tests that arm a process-global (allocator /
// propagation counter). Neither is engine code — the whole file is compiled out of every
// shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use boyko_input::raw::keycode::KeyCode;
use boyko_input::raw::queue::PhysicalInput;

use boyko_math::Vec3;

use boyko_scene::{
    ActiveCamera, Camera, FlyCamera, FlyCameraBundle, GlobalTransform, Projection, Transform,
    ViewUniform, fly_camera_system, propagate_transforms, resolve_active_camera,
};

/// Element-wise float tolerance for derived poses (a `sin_cos` + a look-at).
const EPS: f32 = 1.0e-4;

#[track_caller]
fn approx(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() <= EPS, "{what}: expected {b}, got {a} (|Δ|={})", (a - b).abs());
}

/// A fixed per-update delta (keeps `Instant` jitter out of the driver).
const FIXED_DELTA: Duration = Duration::from_millis(16);

/// The reference forward formula: `forward(yaw, pitch) = norm([sy·cp, sp, -cy·cp])`.
fn ref_forward(yaw: f32, pitch: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    Vec3::new(sy * cp, sp, -cy * cp).normalize()
}

/// A square-aspect perspective (so NDC x/y are symmetric).
fn perspective() -> Projection {
    Projection::Perspective { fov_y: core::f32::consts::FRAC_PI_2, aspect: 1.0, near: 0.1, far: 100.0 }
}

/// A FULL-pipeline App: camera resources seeded + the three systems registered
/// with the R6 ordering edges (`fly.before(propagate)`,
/// `resolve.after(propagate)`), and a seeded `PhysicalInput`. NOT finished (the
/// caller spawns, then `update`).
fn fly_pipeline_world(input: PhysicalInput) -> App {
    let mut app = App::new();
    app.insert_resource(ActiveCamera::default());
    app.insert_resource(ViewUniform::default());
    app.insert_resource(input);
    app.add_systems_cfg(|b| {
        let propagate = b.add_system(propagate_transforms).key();
        b.add_system(fly_camera_system).before(propagate);
        b.add_system(resolve_active_camera).after(propagate);
    });
    app
}

/// Spawns a fly camera at `eye` with the given `fly` state; returns its handle.
fn spawn_fly_camera(world: &mut EcsMaster, eye: Vec3, fly: FlyCamera) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(FlyCameraBundle {
                transform: Transform::from_translation(eye),
                global: GlobalTransform::IDENTITY,
                camera: Camera::DEFAULT,
                projection: perspective(),
                fly,
            })
            .id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    let e = sink.lock().expect("probe lock").expect("spawn produced a handle");
    assert!(world.has_entity(e), "spawned fly camera is live after the apply window");
    e
}

fn transform_of(world: &EcsMaster, e: Entity) -> Transform {
    *world.get_component::<Transform>(e).expect("entity has Transform")
}

fn fly_of(world: &EcsMaster, e: Entity) -> FlyCamera {
    *world.get_component::<FlyCamera>(e).expect("entity has FlyCamera")
}

// ════════════════════════════════════════════════════════════════════════════
// 4. Parity trig (pure math over the reference formula)
// ════════════════════════════════════════════════════════════════════════════

/// `forward(0, 0) == [0, 0, -1]` — the `-Z` look at the start pose.
#[test]
fn forward_at_origin_is_neg_z() {
    let f = ref_forward(0.0, 0.0);
    approx(f.x, 0.0, "forward(0,0).x");
    approx(f.y, 0.0, "forward(0,0).y");
    approx(f.z, -1.0, "forward(0,0).z");
}

/// The system's driven pose matches the reference forward formula at several
/// angles: after one frame with NO input, the camera's local `-Z` (the third
/// column of its rotation basis) equals `ref_forward(yaw, pitch)`.
#[test]
fn driven_forward_matches_reference_formula() {
    for (yaw, pitch) in [(0.0_f32, 0.0_f32), (0.7, 0.4), (-1.2, -0.3), (1.2, 0.9)] {
        let mut app = fly_pipeline_world(PhysicalInput::new());
        let e = spawn_fly_camera(app.world_mut(), Vec3::new(0.0, 1.0, 5.0), FlyCamera::new(yaw, pitch));
        app.update_with_delta(FIXED_DELTA);

        // The camera's world forward = rotation · local -Z. Reconstruct via the
        // rotation quaternion the system wrote.
        let rot = transform_of(app.world(), e).rotation;
        let world_forward = rot.rotate(Vec3::new(0.0, 0.0, -1.0));
        let want = ref_forward(yaw, pitch);
        approx(world_forward.x, want.x, &format!("forward.x @ ({yaw},{pitch})"));
        approx(world_forward.y, want.y, &format!("forward.y @ ({yaw},{pitch})"));
        approx(world_forward.z, want.z, &format!("forward.z @ ({yaw},{pitch})"));
    }
}

/// Pitch is CLAMPED to `±FlyCamera::PITCH_LIMIT`: a mouse-Y delta large enough to
/// drive pitch past the limit is clamped (both signs), and the stored `fly.pitch`
/// reflects the clamp (the accumulator is written back).
#[test]
fn pitch_clamps_at_limit_both_signs() {
    // Sensitivity 1.0 so the raw mouse delta IS the radians delta.
    let fly = FlyCamera { yaw: 0.0, pitch: 0.0, speed: 0.0, sensitivity: 1.0 };

    // Look DOWN hard: pitch -= mouse.y, so a large +mouse.y drives pitch to -LIMIT.
    let mut down = PhysicalInput::new();
    down.mouse_delta = [0.0, 100.0];
    let mut app = fly_pipeline_world(down);
    let e = spawn_fly_camera(app.world_mut(), Vec3::ZERO, fly);
    app.update_with_delta(FIXED_DELTA);
    approx(fly_of(app.world(), e).pitch, -FlyCamera::PITCH_LIMIT, "pitch clamps to -LIMIT");

    // Look UP hard: a large -mouse.y drives pitch to +LIMIT.
    let mut up = PhysicalInput::new();
    up.mouse_delta = [0.0, -100.0];
    let mut app = fly_pipeline_world(up);
    let e = spawn_fly_camera(app.world_mut(), Vec3::ZERO, fly);
    app.update_with_delta(FIXED_DELTA);
    approx(fly_of(app.world(), e).pitch, FlyCamera::PITCH_LIMIT, "pitch clamps to +LIMIT");
}

// ════════════════════════════════════════════════════════════════════════════
// 2. FlyCamera moves Transform (+ the Mut<Transform> changed-tick contract)
// ════════════════════════════════════════════════════════════════════════════

/// A seeded `W` + mouse-X snapshot: the eye moves along the rotated forward, yaw
/// advances by `mouse.x · sensitivity`, the resolved `ViewUniform` reflects the
/// new eye, AND the `Transform`'s `changed_tick` advanced past the spawn tick
/// (the `Mut<Transform>` contract — a bare `&mut Transform` would leave the tick
/// frozen and `propagate_transforms` would never recompose the view).
#[test]
fn fly_moves_transform_and_stamps_changed_tick() {
    let sens = FlyCamera::DEFAULT_SENSITIVITY;
    let speed = FlyCamera::DEFAULT_SPEED;

    let mut input = PhysicalInput::new();
    let w = KeyCode::KeyW.dense_index().expect("W has a dense index");
    input.keys_pressed.set(w);
    input.mouse_delta = [100.0, 0.0];

    let start_eye = Vec3::new(0.0, 1.0, 5.0);
    let mut app = fly_pipeline_world(input);
    let e = spawn_fly_camera(app.world_mut(), start_eye, FlyCamera::new(0.0, 0.0));

    let tid = Transform::component_id();
    let spawn_tick = app
        .world()
        .get_component_changed_tick(e, tid)
        .expect("spawned Transform has a changed tick")
        .get();

    app.update_with_delta(FIXED_DELTA);
    let dt = FIXED_DELTA.as_secs_f32();

    // Yaw advanced by mouse.x · sensitivity.
    approx(fly_of(app.world(), e).yaw, 100.0 * sens, "yaw = mouse.x · sensitivity");

    // The eye moved along the rotated forward by speed · dt. `W` alone ⇒ the
    // move direction IS the (normalized) forward at the NEW yaw (pitch 0).
    let forward = ref_forward(100.0 * sens, 0.0);
    let want_eye = start_eye + forward * (speed * dt);
    let got_eye = transform_of(app.world(), e).translation;
    approx(got_eye.x, want_eye.x, "eye.x moved along forward");
    approx(got_eye.y, want_eye.y, "eye.y moved along forward");
    approx(got_eye.z, want_eye.z, "eye.z moved along forward");

    // The resolved ViewUniform reflects the new eye (camera_pos == eye).
    let view = *app.world().resource::<ViewUniform>();
    approx(view.camera_pos.x, want_eye.x, "ViewUniform camera_pos.x tracks the eye");
    approx(view.camera_pos.y, want_eye.y, "ViewUniform camera_pos.y tracks the eye");
    approx(view.camera_pos.z, want_eye.z, "ViewUniform camera_pos.z tracks the eye");

    // THE PIN: the Transform's changed_tick advanced past the spawn tick — the
    // write went through `Mut<Transform>` (stamps the tick), not a bare `&mut`.
    let after_tick = app
        .world()
        .get_component_changed_tick(e, tid)
        .expect("Transform still has a changed tick")
        .get();
    assert!(
        after_tick != spawn_tick,
        "Transform.changed_tick must advance (Mut<Transform> contract): spawn={spawn_tick} after={after_tick}"
    );
}

/// A no-input frame leaves the eye put (a zero move vector normalizes to zero, so
/// no drift) but still writes a well-formed rotation.
#[test]
fn no_input_keeps_eye_put() {
    let start_eye = Vec3::new(2.0, 3.0, 4.0);
    let mut app = fly_pipeline_world(PhysicalInput::new());
    let e = spawn_fly_camera(app.world_mut(), start_eye, FlyCamera::new(0.3, -0.1));
    app.update_with_delta(FIXED_DELTA);
    let eye = transform_of(app.world(), e).translation;
    approx(eye.x, start_eye.x, "no input ⇒ eye.x unchanged");
    approx(eye.y, start_eye.y, "no input ⇒ eye.y unchanged");
    approx(eye.z, start_eye.z, "no input ⇒ eye.z unchanged");
}
