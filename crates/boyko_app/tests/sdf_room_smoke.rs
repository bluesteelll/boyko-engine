//! R7 windowed smoke: drives the `examples/sdf_room.rs` path headlessly — device
//! singleton boot → `WindowHost` boot (composite + G-buffer scene bundles) → startup
//! mesh registration + ECS spawns (meshes, casters, ONE `SdfPrimitive` sphere, sun +
//! sky + point lights) → the startup `collect_sdf_edits` gather → ~10 presented
//! G-buffer frames (the FIRST performs the one-shot boot-static edit-list upload) → D2
//! teardown — with the exit requested by an ordinary `AppExit`-setting system. Asserts a
//! clean run, that the SDF gather bucketed the sphere, that the one-shot upload ran
//! (the staging is no longer dirty), and that the World is GPU-evicted afterward.
//!
//! SINGLE-TEST BINARY: `EnginePlugins` composes `LightingPlugin`, whose light eviction
//! hooks are process-global — do not co-locate a second light-archetyping test here.
//!
//! Windowed-test conventions: `#[ignore]` (needs a real windowed GPU device), graceful
//! SKIP when boot fails, run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::light_system::LightTableGeneration;
use boyko_render::{MeshRenderScratch, RhiContext, SdfEditStaging};

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

/// The sun direction TO the light — mirrors `examples/sdf_room.rs`.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The sdf_room scene: the room (casters + sun + sky + point) PLUS one `SdfPrimitive`
/// sphere among the cubes, spawned at startup (the device is present — the runner
/// inserts `GpuDevice` + `Assets<MeshGpu>` before `finish`, which drains the SDF gather).
fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // The R7 SDF sphere among the cubes (material 0, UNION op, hard combine).
    commands.spawn(SdfPrimitive(SdfEdit::sphere([-0.9, 0.7, 0.4], 0.7, sdf_op::UNION, 0.0)));

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
    commands.spawn(PointLightObject {
        transform: Transform::from_translation(Vec3::new(0.6, 1.6, -0.8)),
        global: GlobalTransform::IDENTITY,
        light: PointLight::new([0.6, 1.6, -0.8], [1.0, 0.72, 0.45], 220.0, 7.0),
    });

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
fn sdf_room_smoke_ten_frames_then_clean_teardown() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.add_systems(exit_after_budget);
    // NATURAL user registration order (matching examples/sdf_room.rs): `add_plugins`
    // FIRST, THEN `add_startup_system(setup)`. This is the P0 regression witness — the
    // gather MUST see the `SdfPrimitive` even though `setup` is pushed AFTER the plugin.
    // Pre-fix (the gather registered inside `SdfPlugin::build`) it would run before
    // `setup` and gather 0 edits, so the `edits().len() == 1` assertion below would FAIL;
    // the post-`finish()` runner gather makes it order-proof.
    app.add_plugins(EnginePlugins::window("boyko_app R7 sdf_room smoke", 320, 240));
    app.add_startup_system(setup);
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner exits
    // BEFORE the frame loop, so the budget is untouched — SKIP.
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP sdf_room_smoke_ten_frames_then_clean_teardown: windowed boot unavailable");
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    // The raster gather still bucketed the room (floor + cube = 2 batches, 5 instances) —
    // the SDF path composites alongside the raster path, it does not replace it.
    let scratch = app.world().resource::<MeshRenderScratch>();
    assert_eq!(scratch.batch_count(), 2, "floor + cube => two draw batches");
    assert_eq!(scratch.instance_count(), 5, "1 floor + 4 cubes => five instances");

    // The R7 SDF gather bucketed the sphere: the startup `collect_sdf_edits` folded the
    // one `SdfPrimitive` into the staging (edits().len() == 1), and the runner's ONE-SHOT
    // boot-static upload ran on the first frame and cleared the dirty flag.
    let staging = app.world().resource::<SdfEditStaging>();
    assert_eq!(staging.edits().len(), 1, "the startup gather bucketed the one SdfPrimitive");
    assert!(
        !staging.is_dirty(),
        "the one-shot boot-static edit-list upload ran and marked the staging uploaded"
    );

    // Lighting still drove the frame (the SDF path shares the sun + G-buffer).
    let generation = app.world().resource::<LightTableGeneration>().0;
    assert!(generation > 0, "LightTableGeneration advanced past boot (lights were collected)");

    // D2 teardown left the World GPU-evicted.
    assert!(
        !app.world().contains_non_send_resource::<RhiContext>(),
        "teardown must evict the shared-mode RhiContext"
    );
    assert!(
        !app.world().contains_non_send_resource::<Assets<MeshGpu>>(),
        "teardown must evict + destroy the mesh Assets<MeshGpu> table"
    );
    assert!(
        !app.world().contains_non_send_resource::<GpuDevice>(),
        "teardown must evict the GpuDevice handle"
    );
}
