//! SDFDDGI host-hook DEVICE gate: the GI-ON production dump differs from the GI-OFF one.
//!
//! # Why this binary, and not `engine_grand_showcase_512_ddgi_screenshot_dump`
//!
//! The RHI-level DDGI dump (`boyko_rhi_vulkan/tests/window_present_gbuffer.rs`) hand-writes the
//! b18 grid UBO and drives its own header bit at the RHI layer — it never touches
//! `boyko_app`, so it was GREEN for the whole time the host hook was broken (no `DdgiPlugin`
//! composed, `sync_ddgi_light_gate` registered nowhere, the b18 buffer never written after its
//! zero seed, the b6 update UBO packed from a LOCAL default config). A gate that cannot see the
//! defect is not a gate for it. This binary drives the PRODUCTION runner
//! (`EnginePlugins` → `boyko_app::runner` → `gpu_scene::GpuSceneBundles`) with a GI-enabled
//! config and dumps the presented frame.
//!
//! # The scene and the grid
//!
//! The `sdf_room_smoke.rs` scene verbatim: Deferred, the casters, ONE `SdfPrimitive` sphere, and
//! the sun / sky / point lights. GI applies only to `is_sdf_lit` pixels (`deferred_pbr.hlsl`, the
//! marched SDF receiver branch), so the SDF sphere is the receiver. The grid is NON-default
//! (`origin [-6, -0.5, -6]`, `spacing 0.75`, `dims 16x8x16` — a 11.25 x 5.25 x 11.25 box around
//! the room): a resurrected local-default b6 pack (the old `DdgiConfig { ddgi_indirect: true,
//! ..Default }` at the arm site) would update probes over the 32 x 16 x 32 default box while
//! the resolve samples this one, which renders spatially wrong GI — visible, not byte-identical.
//!
//! # Gate statement (run by the orchestrator on the GPU; NOT run in this lane)
//!
//! With `BOYKO_HOST_DUMP=D:\tmp\sdf_room_ddgi.bmp BOYKO_DISABLE_VALIDATION=1 --test-threads=1`:
//!
//! 1. `sha256(sdf_room_ddgi.bmp) != sha256(sdf_room_smoke dumped with the same env, GI-OFF)` —
//!    the GI term reaches the screen (before the fix the two were byte-identical: the header
//!    bit was never set, so the resolve never entered `if (ddgi_mode != 0u)`).
//! 2. The pinned GI-OFF production goldens that bind the DDGI descriptors —
//!    `[grand_showcase_2mat]`, `[vb_both_sdf]`, `[sdf_forward_only]`, `[vb_both]` in
//!    `goldens/PINS.toml` — stay byte-identical after `DdgiPlugin`'s unconditional composition
//!    (the 0%-gate for Decision 4: the default carrier is the all-zero DISABLED image, the
//!    header bit stays 0, `ddgi_update` stays `None`).
//!
//! SINGLE-TEST BINARY: `EnginePlugins` composes `LightingPlugin`, whose light eviction hooks
//! are process-global — do not co-locate a second light-archetyping test here.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::light_system::LightTableGeneration;
use boyko_render::{DdgiConfig, LightingConfig, ResolvedDdgi};

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

/// The sun direction TO the light — mirrors `examples/sdf_room.rs` / `tests/sdf_room_smoke.rs`.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The NON-default GI grid — see the module doc for why it must not be the default.
const GI_GRID: DdgiConfig = DdgiConfig {
    ddgi_indirect: true,
    origin: [-6.0, -0.5, -6.0],
    spacing: 0.75,
    dims: [16, 8, 16],
};

/// The sdf_room scene verbatim (`tests/sdf_room_smoke.rs::setup`).
fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // The R7 SDF sphere among the cubes — the GI RECEIVER (GI applies to `is_sdf_lit` pixels).
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

/// Enough frames for the round-robin probe update to cover every subset at least once before
/// the `BOYKO_HOST_DUMP` frame (the runner dumps around frame ~34 and exits).
const BUDGET: u32 = 40;

#[test]
#[ignore = "gpu-windowed: needs a real windowed GPU device; run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1 and BOYKO_HOST_DUMP; the orchestrator runs it on the GPU for the SDFDDGI host-hook GI-ON dump"]
fn sdf_room_ddgi_screenshot_dump() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.add_systems(exit_after_budget);
    app.add_plugins(EnginePlugins::window("boyko_app SDFDDGI host-hook GI-ON dump", 320, 240));
    app.add_startup_system(setup);
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    // GI ON over the NON-default grid — inserted AFTER `add_plugins`, replacing `DdgiPlugin`'s
    // DISABLED default (the same "overwrite after add_plugins" contract `CsmConfig` uses).
    app.insert_resource(GI_GRID);

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner exits BEFORE the
    // frame loop, so the budget is untouched — SKIP (never a vacuous green: the dump itself is
    // the gate, and it does not exist on a skipped run).
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP sdf_room_ddgi_screenshot_dump: windowed boot unavailable");
        return;
    }

    // The host hook's device-side claims, readable back from the World after the run:
    // (1) the single writer resolved the NON-default grid (not the owner-locked default) —
    //     on a no-storage device it resolves DISABLED instead, which is the plan-§3 degrade, so
    //     the assertion is conditional on the carrier being armed at all;
    let resolved = *app.world().resource::<ResolvedDdgi>();
    if resolved.ddgi_mode_word != 0 {
        assert_eq!(resolved.origin, [-6.0, -0.5, -6.0, 0.0], "the carrier is the test's grid");
        assert_eq!(resolved.inv_spacing_dims[0], 1.0 / 0.75, "the carrier's inv_spacing");
        assert_eq!(resolved.inv_spacing_dims[1].to_bits(), 16);
        assert_eq!(resolved.inv_spacing_dims[2].to_bits(), 8);
        assert_eq!(resolved.inv_spacing_dims[3].to_bits(), 16);
        // (2) the header bit followed the carrier (the SOLE production writer ran in the
        //     production Main — the registration the headless gate (a2) pins).
        assert!(
            app.world().resource::<LightingConfig>().ddgi_indirect,
            "sync_ddgi_light_gate set LightingConfig::ddgi_indirect from the armed carrier"
        );
    } else {
        eprintln!("NOTE sdf_room_ddgi_screenshot_dump: DdgiCaps clamped the carrier to DISABLED (no B10G11R11/RG16F storage) -- the dump is GI-OFF on this device");
    }
    let generation = app.world().resource::<LightTableGeneration>().0;
    assert!(generation > 0, "LightTableGeneration advanced past boot (lights were collected)");
}
