//! Regression oracle for the room-example camera chain (owner-reported R3 bug:
//! a wrongly-oriented camera rendered one stray cube instead of the room).
//!
//! Two layers, so a failure convicts the exact link:
//! 1. `look_at_pose_math_round_trip` — pure math: `Affine3A::look_at_rh` →
//!    `Quat::from_mat3` → `Transform::to_affine` must reproduce the camera
//!    world basis (the rotation survives the quaternion round trip).
//! 2. `camera_rig_resolves_to_view_uniform` — the full ECS chain: spawn the
//!    room's exact `CameraRig`, run propagation + `resolve_active_camera`, and
//!    assert the `ViewUniform` eye/forward lanes match the authored pose.
//!
//! Headless (no window / GPU needed) — runs in every suite.

use boyko_app::prelude::*;
use boyko_scene::{CameraPlugin, ViewUniform};

const EYE: [f32; 3] = [0.0, 1.7, 6.0];
/// normalize(target − eye) for target = origin: (0, −1.7, −6) / |(0, 1.7, 6)|.
const FORWARD: [f32; 3] = [0.0, -0.272_31, -0.962_21];
const TOL: f32 = 1.0e-3;

fn assert_xyz_near(x: f32, y: f32, z: f32, expected: [f32; 3], what: &str) {
    let d = Vec3::new(x - expected[0], y - expected[1], z - expected[2]).length();
    assert!(
        d < TOL,
        "{what}: expected ~{expected:?}, got ({x}, {y}, {z}) — |delta| = {d}"
    );
}

/// The exact pose construction `examples/room.rs` performs.
fn room_camera_transform() -> Transform {
    let pose = Affine3A::look_at_rh(
        Vec3::new(EYE[0], EYE[1], EYE[2]),
        Vec3::ZERO,
        Vec3::new(0.0, 1.0, 0.0),
    );
    Transform {
        translation: pose.translation,
        rotation: Quat::from_mat3(pose.matrix3),
        scale: Vec3::ONE,
    }
}

/// Layer 1: the pose math the room example performs, without any ECS.
#[test]
fn look_at_pose_math_round_trip() {
    let pose = Affine3A::look_at_rh(
        Vec3::new(EYE[0], EYE[1], EYE[2]),
        Vec3::ZERO,
        Vec3::new(0.0, 1.0, 0.0),
    );
    // Column 2 of the world basis is camera +Z ("back"); M·e_z extracts it
    // without a column accessor. The view direction is its negation.
    let back = pose.matrix3.mul_vec(Vec3::new(0.0, 0.0, 1.0));
    assert_xyz_near(-back.x, -back.y, -back.z, FORWARD, "look_at_rh forward (-col2)");
    assert_xyz_near(
        pose.translation.x, pose.translation.y, pose.translation.z,
        EYE, "look_at_rh translation",
    );

    // The quaternion round trip the example takes: basis → Quat → basis.
    let affine = room_camera_transform().to_affine();
    let rt_back = affine.matrix3.mul_vec(Vec3::new(0.0, 0.0, 1.0));
    assert_xyz_near(-rt_back.x, -rt_back.y, -rt_back.z, FORWARD, "Quat round-trip forward");
    assert_xyz_near(
        affine.translation.x, affine.translation.y, affine.translation.z,
        EYE, "Quat round-trip translation",
    );
}

/// Startup: spawn the room's exact camera rig (device-free — camera only).
fn spawn_room_camera(mut commands: Commands) {
    commands.spawn(CameraRig {
        transform: room_camera_transform(),
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 800.0 / 600.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// Layer 2: the full spawn → propagate → resolve chain the windowed host runs.
///
/// Regression pin for the R3 startup-spawn hole: a `Commands` startup spawn is
/// stamped at world tick 0, and a propagation `last_run` baseline of literal
/// `Tick::ZERO` (exclusive lower bound) could never see it — the camera kept an
/// identity `GlobalTransform` and the room rendered from the origin. The fix is
/// the TICK8 never-run baseline (`current_tick - MAX_CHANGE_AGE`) in
/// `TransformPropagationScratch`; this test MUST stay a fresh-spawn oracle (no
/// post-spawn `Transform` writes before the assertions).
#[test]
fn camera_rig_resolves_to_view_uniform() {
    let mut app = App::new();
    // The camera-relevant subset of EnginePlugins' composition (no window/runner).
    app.add_plugin(CameraPlugin);
    app.add_startup_system(spawn_room_camera);

    // Two updates: startup flush + propagation + resolve settle.
    app.update_with_delta(core::time::Duration::from_millis(16));
    app.update_with_delta(core::time::Duration::from_millis(16));

    let view = *app.world().resource::<ViewUniform>();
    assert!(view.fov_y > 0.0, "an ACTIVE camera resolved (fov_y > 0)");
    assert_xyz_near(
        view.camera_pos.x, view.camera_pos.y, view.camera_pos.z,
        EYE, "ViewUniform camera_pos (fresh spawn, no post-spawn writes)",
    );
    assert_xyz_near(
        view.cam_forward.x, view.cam_forward.y, view.cam_forward.z,
        FORWARD, "ViewUniform cam_forward (fresh spawn, no post-spawn writes)",
    );
}
