//! R3+R4 windowed smoke: drives the FULL room path headlessly — device
//! singleton boot → `WindowHost` boot (composite + G-buffer scene bundles) →
//! startup mesh registration + ECS spawns (meshes, casters, sun + sky + point
//! lights) → ~10 presented G-buffer frames (camera + instance uploads every
//! frame; gen-gated light uploads; CSM armed) → D2 teardown — with the exit
//! requested by an ordinary `AppExit`-setting system. Asserts a clean run, the
//! gathers actually bucketed the room + casters, the light generation protocol
//! gated the uploads, the CSM lock-step armed, and the World is GPU-evicted
//! afterward.
//!
//! SINGLE-TEST BINARY: `EnginePlugins` composes `LightingPlugin`, whose light
//! eviction hooks are process-global — do not co-locate a second
//! light-archetyping test here.
//!
//! Windowed-test conventions: `#[ignore]` (needs a real windowed GPU device),
//! graceful SKIP when boot fails, run with `BOYKO_DISABLE_VALIDATION=1` and
//! `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::light_system::LightTableGeneration;
use boyko_render::{CsmCasterScratch, LightingConfig, MeshRenderScratch, ResolvedCsm, RhiContext};
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

/// The sun direction TO the light — mirrors `examples/room.rs`.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The room scene of `examples/room.rs` (R4 form: casters + sun + sky + point),
/// spawned at startup (the device is present — the runner inserts `GpuDevice` +
/// `Assets<MeshGpu>` before finish).
fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    // Floor = receiver-only (no ShadowCaster); cubes = structural casters.
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // ECS lighting (R4): the angled sun (transform-oriented so the reconcile
    // derives the same direction), a sky fill, a warm point accent.
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
fn room_smoke_ten_frames_then_clean_teardown() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.add_systems(exit_after_budget);
    app.add_startup_system(setup);
    app.add_plugins(EnginePlugins::window("boyko_app R4 room smoke", 320, 240));
    // Enable CSM (the owner-set knob; the plugin default is DISABLED). Inserted
    // AFTER add_plugins so it overwrites CsmPlugin's default.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });

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

    // ── R4: ECS lighting drove the frame. ───────────────────────────────────

    // The staged light table was actually rebuilt from the spawned lights: the
    // writer-side generation advanced past the boot 0.
    let generation = app.world().resource::<LightTableGeneration>().0;
    assert!(generation > 0, "LightTableGeneration advanced past boot (lights were collected)");

    // The caster gather bucketed EXACTLY the casters: 4 cube instances of one
    // mesh (the receiver-only floor is structurally excluded).
    let casters = app.world().resource::<CsmCasterScratch>();
    assert_eq!(casters.batch_count(), 1, "one caster mesh (the cube) => one caster batch");
    assert_eq!(casters.instance_count(), 4, "4 ShadowCaster cubes; the floor is excluded");

    // The CSM chain resolved and stayed in lock-step: the fit is armed (a sun +
    // an enabled CsmConfig), and the header gate the resolve samples under was
    // synced ON by `sync_csm_light_gate` (same predicate the runner armed the
    // depth pass with).
    let resolved = app.world().resource::<ResolvedCsm>();
    assert_eq!(resolved.csm_mode_word, 1, "ResolvedCsm armed (sun + enabled config)");
    assert_eq!(resolved.active_count, 3, "the owner-set 3-cascade fit");
    assert!(
        app.world().resource::<LightingConfig>().csm_shadows,
        "the light-header CSM gate synced ON (lock-step with the armed depth pass)"
    );

    // The host probe: the depth pass was armed on presented frames, and the
    // light-upload gate actually GATED — a bounded number of catch-up uploads
    // (2 boot slots + 2 per writer-side bump), strictly fewer than frames.
    // The budget's LAST decrement requests exit at step 3 (before that frame
    // presents), so presented frames == BUDGET - 1.
    let stats = *app.world().resource::<HostFrameStats>();
    assert_eq!(stats.frames, u64::from(BUDGET) - 1, "the probe counted the presented frames");
    assert!(stats.csm_armed_frames > 0, "scene.csm was armed on at least one frame");
    assert!(stats.light_uploads >= 2, "both in-flight slots caught up at least once");
    assert!(
        stats.light_uploads < stats.frames,
        "the generation gate closed on steady-state frames ({} uploads / {} frames)",
        stats.light_uploads,
        stats.frames
    );

    // D2 teardown left the World GPU-evicted: no device-referencing NonSend
    // resident may survive `destroy_singleton` (the `'static` fiction ended).
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
