//! R3 windowed smoke: drives the FULL room path headlessly — device singleton
//! boot → `WindowHost` boot (composite + G-buffer scene bundles) → startup
//! mesh registration + ECS spawns → ~10 presented G-buffer frames (camera +
//! instance uploads every frame) → D2 teardown — with the exit requested by an
//! ordinary `AppExit`-setting system. Asserts a clean run, the gather actually
//! bucketed the room, and the World is GPU-evicted afterward.
//!
//! Windowed-test conventions: `#[ignore]` (needs a real windowed GPU device),
//! graceful SKIP when boot fails, run with `BOYKO_DISABLE_VALIDATION=1` and
//! `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::{MeshRenderScratch, RhiContext};
use boyko_scene::ViewUniform;

/// The room camera's authored eye — must survive spawn → propagate → resolve.
const EYE: [f32; 3] = [0.0, 1.7, 6.0];
/// normalize(target − eye) for target = origin: (0, −1.7, −6) / |(0, 1.7, 6)|.
const FORWARD: [f32; 3] = [0.0, -0.272_31, -0.962_21];

/// Asserts a `ViewUniform` xyz lane is within `1e-3` of the authored vector.
fn assert_lane_near(x: f32, y: f32, z: f32, expected: [f32; 3], what: &str) {
    let d = Vec3::new(x - expected[0], y - expected[1], z - expected[2]).length();
    assert!(
        d < 1.0e-3,
        "{what}: expected ~{expected:?}, got ({x}, {y}, {z}) — |delta| = {d}"
    );
}

/// Frames left before the test requests exit. Decremented once per Main run.
#[derive(Resource)]
struct FrameBudget(u32);

/// Counts the budget down and requests exit on the last frame.
fn exit_after_budget(mut budget: ResMut<FrameBudget>, mut exit: ResMut<AppExit>) {
    if budget.0 > 0 {
        budget.0 -= 1;
        if budget.0 == 0 {
            exit.0 = true;
        }
    }
}

/// The room scene of `examples/room.rs`, spawned at startup (the device is
/// present — the runner inserts `GpuDevice` + `MeshRegistry` before finish).
fn setup(mut commands: Commands, mut meshes: NonSendResMut<MeshRegistry>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands.spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))));
    }
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.7, 6.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 320.0 / 240.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

const BUDGET: u32 = 10;

#[test]
#[ignore = "needs a real windowed GPU device; run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn room_smoke_ten_frames_then_clean_teardown() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.add_systems(exit_after_budget);
    app.add_startup_system(setup);
    app.add_plugins(EnginePlugins::window("boyko_app R3 room smoke", 320, 240));

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner
    // exits BEFORE the frame loop, so the budget is untouched — SKIP.
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP room_smoke_ten_frames_then_clean_teardown: windowed boot unavailable");
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    // The gather bucketed the room: 2 distinct meshes (floor + cube) ⇒ 2
    // batches; 5 instances total (1 floor + 4 cubes). This proves the ECS spawn
    // → visibility bridge → pack → gather chain actually drove the draws.
    let scratch = app.world().resource::<MeshRenderScratch>();
    assert_eq!(scratch.batch_count(), 2, "floor + cube => two draw batches");
    assert_eq!(scratch.instance_count(), 5, "1 floor + 4 cubes => five instances");

    // The camera CHAIN resolved the AUTHORED pose (R3 regression: a startup-
    // spawned camera whose `GlobalTransform` never left identity rendered the
    // room from the origin; `fov_y` alone cannot catch that — it is written
    // even when the pose stays identity). `ViewUniform` is derived from the
    // propagated `GlobalTransform` by `resolve_active_camera` every frame, so a
    // wrong camera pose can never pass this smoke again.
    let view = *app.world().resource::<ViewUniform>();
    assert!(view.fov_y > 0.0, "an ACTIVE camera resolved (fov_y > 0)");
    assert_lane_near(
        view.camera_pos.x, view.camera_pos.y, view.camera_pos.z,
        EYE, "ViewUniform camera_pos (authored eye)",
    );
    assert_lane_near(
        view.cam_forward.x, view.cam_forward.y, view.cam_forward.z,
        FORWARD, "ViewUniform cam_forward (authored look direction)",
    );

    // The one-frame-stale WindowInfo was published post-present.
    let info = *app.world().resource::<WindowInfo>();
    assert!(info.width > 0 && info.height > 0, "WindowInfo published post-present");

    // D2 teardown left the World GPU-evicted: no device-referencing NonSend
    // resident may survive `destroy_singleton` (the `'static` fiction ended).
    assert!(
        !app.world().contains_non_send_resource::<RhiContext>(),
        "teardown must evict the shared-mode RhiContext"
    );
    assert!(
        !app.world().contains_non_send_resource::<MeshRegistry>(),
        "teardown must evict + destroy the MeshRegistry"
    );
    assert!(
        !app.world().contains_non_send_resource::<GpuDevice>(),
        "teardown must evict the GpuDevice handle"
    );
}
