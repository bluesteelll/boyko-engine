//! R5 windowed interp smoke: the production host port of the RHI harness's
//! `run_interp_smoke`. Drives the FULL interpolated room path headlessly through
//! the REAL runner — device singleton boot → `WindowHost` boot → startup mesh
//! registration + ECS spawns (a mesh carrying the `GpuTransform3D` interpolation
//! pair, plus a sun + camera) → ~10 presented frames (the runner uploads the pair
//! ring + arms `GBufferScene::interp` every frame the gather produced pairs) → D2
//! teardown. Asserts the interp pre-pass was LIVE (`HostFrameStats::interp_armed_frames`
//! > 0) and the pair gather actually bucketed the interpolated instances.
//!
//! This is the SUCCESSOR to `window_present_gbuffer::run_interp_smoke` on the
//! production host path — the old RHI-harness interp machinery is NOT deleted (it
//! stays the low-level keystone; this proves the SAME chain through the host).
//!
//! SINGLE-TEST BINARY: `EnginePlugins` composes `LightingPlugin`, whose light
//! eviction hooks are process-global — do not co-locate a second light-archetyping
//! test here.
//!
//! Windowed-test conventions: `#[ignore]` (needs a real windowed GPU device),
//! graceful SKIP when boot fails, run with `BOYKO_DISABLE_VALIDATION=1` and
//! `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::{Bundle, Resource};
use boyko_render::mesh_draw::MeshRenderScratch;
use boyko_render::{GpuTransform3D, RhiContext};

/// The sun direction TO the light — mirrors `examples/room.rs`.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// A dense-only attach payload for the interpolation pair (a dense component's
/// single-component self-`Bundle` is suppressed; a one-field bundle is the path).
#[derive(Bundle)]
struct PairOnly {
    pair: GpuTransform3D,
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

/// The interpolated room: one cube carrying BOTH the mesh draw components AND the
/// `GpuTransform3D` interpolation pair (the opt-in), a floor, a sun, a camera.
fn setup(mut commands: Commands, mut meshes: NonSendResMut<MeshRegistry>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);

    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));

    // The interpolated cube: spawn the drawable, then attach the interpolation pair
    // (seeded prev == curr from its transform) — the pair's PRESENCE opts the body
    // into the pair gather + the interp pre-pass.
    let cube_transform = Transform::from_translation(Vec3::new(0.0, 0.5, 0.0));
    commands
        .spawn(MeshBundle::new(cube, cube_transform))
        .insert(ShadowCaster)
        .insert(PairOnly { pair: GpuTransform3D::from_transform(&cube_transform) });

    // The sun (an angled directional light), oriented so the reconcile derives the
    // authored direction.
    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));

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
fn interp_smoke_pair_path_is_live_over_the_host() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.add_systems(exit_after_budget);
    app.add_startup_system(setup);
    app.add_plugins(EnginePlugins::window("boyko_app R5 interp smoke", 320, 240));

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner exits
    // BEFORE the frame loop, so the budget is untouched — SKIP.
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP interp_smoke_pair_path_is_live_over_the_host: windowed boot unavailable");
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    // The UNIFIED gather kept BOTH drawables (the R5 review P0): the floor batch is
    // NOT dropped by the interpolated cube's presence. TWO batches (floor mesh + cube
    // mesh) cover the whole ring, and exactly ONE dynamic instance (the cube; the
    // floor carries no GpuTransform3D) was recorded into the pair / out-slot lanes.
    let scratch = app.world().resource::<MeshRenderScratch>();
    assert_eq!(
        scratch.batch_count(),
        2,
        "the unified gather kept both drawables (floor + cube) — none dropped (P0)"
    );
    assert_eq!(
        scratch.instance_count(),
        2,
        "the unified ring holds both the floor and the cube instances"
    );
    assert_eq!(
        scratch.dynamic_count(),
        1,
        "exactly one interpolated instance (the cube) went into the pair lanes"
    );
    assert_eq!(
        scratch.pair_out_slot.len(),
        1,
        "the cube's out-slot was recorded parallel to its pair"
    );

    // The interp pre-pass was ARMED on the presented frames: the runner uploaded
    // the pair ring and set GBufferScene::interp = Some(activation) every frame the
    // pair gather produced instances — the R5 keystone, proven through the host.
    let stats = *app.world().resource::<HostFrameStats>();
    assert_eq!(stats.frames, u64::from(BUDGET) - 1, "the probe counted the presented frames");
    assert_eq!(
        stats.interp_armed_frames, stats.frames,
        "the interp pre-pass was armed on every presented frame (the pair ring was non-empty)"
    );

    // D2 teardown left the World GPU-evicted.
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
