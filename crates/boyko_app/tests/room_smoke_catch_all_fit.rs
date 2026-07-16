//! CSM auto-fit plan (`docs/CSM-AUTOFIT-PLAN.md`) rung C5, test T22 (second row): the
//! SAME `examples/room.rs` scene as `room_smoke.rs`, but with `CsmConfig.fit_mode:
//! CatchAll` instead of the default `Fixed`. Proves `reduce_caster_bounds` — wired into
//! `EnginePlugins` at `boyko_app::plugins` (rung C5) — actually RAN and folded a
//! COMPLETE, non-empty caster bound.
//!
//! # Why this is a SEPARATE file, not a second `#[test]` in `room_smoke.rs`
//!
//! `room_smoke.rs` documents "SINGLE-TEST BINARY: `EnginePlugins` composes
//! `LightingPlugin`, whose light eviction hooks are process-global — do not co-locate a
//! second light-archetyping test here." This scene spawns the same sun/sky/point lights,
//! so it inherits that constraint; a separate `tests/*.rs` file compiles to its own test
//! binary (a separate process), which is exactly the isolation the warning asks for.
//!
//! # Why `total_batches > 0` is the RIGHT assertion here, not in `room_smoke.rs`
//!
//! `reduce_caster_bounds`'s own 0%-gate returns `CsmCasterBounds::EMPTY` whenever
//! `cfg.fit_mode == Fixed` (csm_caster.rs) — under `room_smoke.rs`'s default `Fixed`
//! config, `total_batches > 0` would NOT hold, even though the reducer is correctly
//! wired and ran every frame. `Fixed` and "the reducer was never registered" are
//! byte-identical at the `CsmCasterBounds` level (both leave it `EMPTY`) — `CatchAll` is
//! the only configuration that can tell them apart, which is the whole point of this
//! second, non-golden-pinned smoke world (`goldens/PINS.toml` has no `room_smoke*`
//! entry, so flipping `fit_mode` here moves no golden).
//!
//! Windowed-test conventions: `#[ignore]` (needs a real windowed GPU device),
//! graceful SKIP when boot fails, run with `BOYKO_DISABLE_VALIDATION=1` and
//! `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::{CsmCasterBounds, CsmFitMode};

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

/// The sun direction TO the light — mirrors `examples/room.rs` / `room_smoke.rs`.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The room scene of `examples/room.rs` (R4 form: casters + sun + sky + point),
/// spawned at startup (the device is present — the runner inserts `GpuDevice` +
/// `Assets<MeshGpu>` before finish). Mirrors `room_smoke.rs::setup` verbatim — kept
/// duplicated rather than shared, per that file's SINGLE-TEST BINARY isolation note.
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
fn room_smoke_catch_all_fit_folds_complete_caster_bounds() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.add_systems(exit_after_budget);
    app.add_startup_system(setup);
    app.add_plugins(EnginePlugins::window("boyko_app R4 room smoke (CatchAll fit)", 320, 240));
    // Enable CSM AND opt into the caster-aware fit — inserted AFTER add_plugins so it
    // overwrites CsmPlugin's default (`Fixed`, `cascade_count: 0`).
    app.insert_resource(CsmConfig {
        cascade_count: 3,
        fit_mode: CsmFitMode::CatchAll,
        ..CsmConfig::default()
    });

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner exits
    // BEFORE the frame loop, so the budget is untouched — SKIP (mirrors room_smoke.rs).
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP room_smoke_catch_all_fit_folds_complete_caster_bounds: windowed boot unavailable");
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    // THE pin (T22, second row): `reduce_caster_bounds` ran (it is wired into
    // `EnginePlugins` — rung C5) and folded a COMPLETE, non-empty bound. The 4
    // `ShadowCaster` cubes share one mesh (procedural, created synchronously via
    // `meshes.cube` — never streamed), so their batch is `Loaded` on every frame ⇒
    // `resolved_batches == total_batches`, never held/incomplete (D7).
    let bounds = *app.world().resource::<CsmCasterBounds>();
    assert!(
        bounds.total_batches > 0,
        "the caster gather must have emitted at least one batch (the 4 ShadowCaster cubes)"
    );
    assert_eq!(
        bounds.resolved_batches, bounds.total_batches,
        "the cube mesh was already Loaded (procedural, not streamed) => the fold is COMPLETE"
    );
    assert!(bounds.is_usable(), "a complete, non-empty fold is USABLE as a fit input");
}
