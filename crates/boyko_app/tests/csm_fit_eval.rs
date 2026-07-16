//! CSM auto-fit rung C6: the owner-facing `CsmFitMode` eval dump (NOT a golden).
//!
//! Renders `examples/room.rs`'s scene — a floor receiver plus four cube casters lit by an angled
//! sun — through the fit mode named by `BOYKO_CSM_FIT` (`fixed` | `shrink` | `catchall`, default
//! `fixed`), so the owner can judge whether the caster-fitted partition buys a VISIBLE win before
//! any pinned scene opts into it.
//!
//! # Why this scene answers the question
//!
//! The camera sits at `z = 6` looking at the origin and the casters occupy roughly view-depth
//! 4.5..8.5, while `CsmConfig::default()`'s `shadow_distance` is 30. `Fixed` therefore partitions
//! `[0.1, 30]` and spends cascade 0 (`[0.1, 2.549]`) on a range holding NO caster at all — the
//! exact waste rung C3 exists to reclaim. This is a shipped scene at the shipped config
//! (`cascade_count: 3`), not a constructed best case.
//!
//! # Reading the dumps
//!
//! The question the owner must settle is BLOCKS vs MUSH, because the two have different causes and
//! only one of them is this feature's to fix:
//! - **Blocks** (a stair-stepped shadow edge) is shadow-texel quantization ⇒ the fit is the fix,
//!   and `catchall` should visibly beat `fixed`.
//! - **Mush** (a smooth but wide edge) is `shadow_apply.hlsli`'s 13-tap PCF tent, which is measured
//!   in TEXELS (~10 of them) ⇒ the fit shrinks it proportionally but cannot remove it. If the three
//!   dumps look equally soft, the tent is the defect and this feature is secondary.
//!
//! `BOYKO_WIN` sets the square window edge (default `900`); `BOYKO_HOST_DUMP=<path.bmp>` arms the
//! capture; `BOYKO_CSM_CLOSEUP=1` pulls the camera in to `z = 2.6`, where the fit's gain is largest
//! (the plan's measured table: the win concentrates at cascade 0's NEAR end and fades to ~1.1× once
//! casters span a wide depth range).
//!
//! `#[ignore]`: needs a real windowed GPU device; the orchestrator runs it. SINGLE-TEST BINARY —
//! `EnginePlugins` composes `LightingPlugin`, whose light eviction hooks are process-global (see
//! `room_smoke.rs`'s identical warning).

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_render::{CsmFitMode, LightingConfig};

/// The sun direction TO the light — mirrors `examples/room.rs` / `room_smoke.rs`.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// `examples/room.rs`'s scene verbatim (floor receiver + four cube casters + sun/sky/point),
/// except that the camera distance is `BOYKO_CSM_CLOSEUP`-selectable.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    dev: NonSendRes<GpuDevice>,
) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    // Floor = receiver-only (no ShadowCaster); cubes = structural casters. That split is what
    // makes this a fit eval at all: the caster bounds must exclude the 12-unit floor, or the
    // fitted range would degenerate back to the whole scene.
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

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

    let (eye, target) = if std::env::var("BOYKO_CSM_CLOSEUP").is_ok() {
        (Vec3::new(0.9, 1.05, 2.6), Vec3::new(0.6, 0.25, 0.4))
    } else {
        (Vec3::new(0.0, 1.7, 6.0), Vec3::ZERO)
    };
    let pose = Affine3A::look_at_rh(eye, target, Vec3::new(0.0, 1.0, 0.0));
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
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// **The `CsmFitMode` eval dump** (owner-facing, NOT a golden — no `PINS.toml` entry).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1` and
/// `--test-threads=1`; see this module's doc for the env knobs and for how to read the result.
#[test]
#[ignore = "needs a real windowed GPU device; orchestrator-run CsmFitMode owner-eval dump"]
fn csm_fit_eval_screenshot_dump() {
    let win: u32 = std::env::var("BOYKO_WIN").ok().and_then(|s| s.parse().ok()).unwrap_or(900);
    let fit_mode = match std::env::var("BOYKO_CSM_FIT").ok().as_deref() {
        Some("shrink") => CsmFitMode::Shrink,
        Some("catchall") => CsmFitMode::CatchAll,
        _ => CsmFitMode::Fixed,
    };
    println!("csm_fit_eval: fit_mode={fit_mode:?} win={win}");

    let mut app = App::new();
    app.add_startup_system(setup);
    app.add_plugins(EnginePlugins::window("boyko_app CSM fit eval", win, win));
    // Inserted AFTER add_plugins so it overwrites CsmPlugin's disabled default. `cascade_count: 3`
    // is what every in-tree scene uses — the config the plan's gain table was measured at.
    app.insert_resource(CsmConfig { cascade_count: 3, fit_mode, ..CsmConfig::default() });
    app.insert_resource(LightingConfig { csm_shadows: true, ..LightingConfig::default() });

    app.run();
}
