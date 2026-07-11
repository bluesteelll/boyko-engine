//! Asset-streaming plan F7-hwrt (task#11) — REAL-DEVICE RT-leg grow-past-cap test.
//!
//! REPURPOSED from the pre-task#11 `w3_rt_cap_traps_an_over_capacity_instance_gather_
//! on_an_rt_device`, which asserted the RT hard-cap PANIC in `upload_instance_models`
//! that task#11 REMOVES (RT-leg instance-family growth is now IN SCOPE — see
//! `GpuSceneBundles::grow_instance_family_rt`). This file now proves the OPPOSITE: on
//! an RT device, the SAME `> INSTANCE_CAPACITY` gather must GROW (the shared instance
//! rings + the conditional `mv.grow_slot` + `tlas.grow_slot` + every AS-handle repoint
//! via `repoint_tlas_accel`) instead of panicking — the run must complete its full
//! frame budget cleanly.
//!
//! # No `catch_unwind` anymore
//!
//! The pre-task#11 version used `catch_unwind` because a panic was the EXPECTED,
//! load-bearing signal on an RT device (and a non-panic was ambiguous: boot-unavailable
//! or a legitimate non-RT growth path). Since task#11 removes the panic path entirely,
//! NO panic is expected on ANY device now — any panic here (RT or non-RT) is a genuine
//! regression, so this test lets it propagate as an ordinary test failure instead of
//! swallowing it.
//!
//! # Runtime-gated, not compile-time (read before treating a pass as final)
//!
//! Whether the instance family grows via the non-RT (`grow_instance_family_nonrt`,
//! already proven by the sibling phased test's Phase B) or RT
//! (`grow_instance_family_rt`) path is a RUNTIME device-capability gate
//! (`tlas.is_some()`), independent of the `hwrt` Cargo feature: an `hwrt`-feature build
//! on a non-RT GPU still takes the non-RT path. This file's `--features hwrt` gate only
//! ensures the RT-capable FFI/build surface compiles in; a LOAD-BEARING pass of the
//! RT-LEG claim specifically still requires running on hardware that negotiates
//! hardware ray tracing.
//!
//! # `MAX_INSTANCE_CAP` (the true ceiling) is NOT exercised here
//!
//! `MAX_INSTANCE_CAP` (1 << 22) stays the shared sanity-net ceiling for BOTH legs (no
//! separate RT ceiling, per the orchestrator's decision) — a `debug_assert`-only guard,
//! exactly like the non-RT leg's own (pre-existing, likewise untested-at-that-scale)
//! ceiling. Spawning millions of drawables in a headless smoke to trip it is
//! impractical (no existing test in this workspace does so for the non-RT leg either);
//! the ceiling remains guarded by the `debug_assert!` in `grow_instance_family_nonrt`/
//! `grow_instance_family_rt`, not by a dedicated headless test.
//!
//! # Asset-streaming plan F8 fold-in
//!
//! `spawn_many` gives every drawable a SHARED non-default material, so the
//! over-capacity gather this test drives is material-bearing — `pm_instance_material_
//! rings` grows in lockstep with `instance_rings` on BOTH legs (F8 §1.2/§7b +
//! F7-hwrt's `grow_shared_instance_rings`), so this also proves F8 did not introduce a
//! bypass that OOB-writes an un-grown material ring.
//!
//! # Separate file (structural constraint)
//!
//! This file keeps its OWN `App`/`#[test]`, matching this workspace's
//! one-windowed-`#[test]`-per-file convention (see the sibling file's module doc: a
//! second `App::new()` + `add_plugins` in the same process re-registers process-global
//! component hooks and panics).
//!
//! # Running (the orchestrator, NOT a subagent)
//!
//! ```text
//! cargo test -p boyko-app --features hwrt --test asset_streaming_f7_rt_cap_headless -- --ignored --test-threads=1
//! ```
//!
//! Not built/run at all without `--features hwrt` (the file's single `#[test]` is
//! itself `#[cfg(feature = "hwrt")]`-gated, and the whole file's scaffolding with it,
//! so a default build compiles this to an empty test binary).

#![cfg(windows)]
#![cfg(feature = "hwrt")]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::MaterialGpu;

/// Frames left before the test requests exit. Mirrors
/// `asset_streaming_f6_churn_headless.rs`'s / the sibling F7 file's budget idiom.
#[derive(Resource)]
struct FrameBudget(u32);

fn exit_after_budget(mut budget: ResMut<FrameBudget>, mut exit: ResMut<AppExit>) {
    if budget.0 > 0 {
        budget.0 -= 1;
        if budget.0 == 0 {
            exit.0 = true;
        }
    }
}

/// Comfortably past `INSTANCE_CAPACITY` (1024, `boyko_app::gpu_scene`'s
/// `pub(crate)` constant — not nameable here).
const DRAWABLES: u32 = 1024 + 200;
const FRAMES: u32 = 8;

/// `Option`-wrapped + `Default`-derived so it can be `app.insert_resource`d BEFORE
/// `app.run()` (a startup system has no `insert_resource` command) — mirrors the
/// sibling F7 file's `SharedCubeMesh`.
#[derive(Resource, Default, Clone, Copy)]
struct SharedCubeMesh(Option<MeshHandle>);

impl SharedCubeMesh {
    fn get(self) -> MeshHandle {
        self.0.expect("invariant: setup_minimal_scene populates this before any reader runs")
    }
}

fn setup_minimal_scene(
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    dev: NonSendRes<GpuDevice>,
    mut commands: Commands,
    mut shared: ResMut<SharedCubeMesh>,
) {
    let cube = meshes.cube(dev.get(), 1.0);
    shared.0 = Some(cube);

    const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];
    let sun_pose =
        Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
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

fn spawn_many(mut commands: Commands, cube: Res<SharedCubeMesh>, mut materials: ResMut<Assets<MaterialGpu>>) {
    // Asset-streaming plan F8 fold-in: a SHARED non-default material for every drawable —
    // see this file's module doc for why the over-capacity gather this test drives must be
    // material-bearing.
    let pm_material = materials.add(MaterialGpu::new([0.9, 0.1, 0.1, 1.0], 1.0, 0.3, 0.5, [0.0, 0.0, 0.0], 0));
    for i in 0..DRAWABLES {
        commands.spawn(MeshBundle {
            material: MaterialHandle(pm_material.index() as u16),
            ..MeshBundle::new(
                cube.get(),
                Transform::from_translation(Vec3::new((i % 64) as f32, (i / 64) as f32, 0.0)),
            )
        });
    }
}

/// F7-hwrt (task#11): a `> INSTANCE_CAPACITY` gather must GROW (not panic) on every
/// device tier — see this file's module doc for the RT-leg-specific load-bearing
/// requirement.
#[test]
#[ignore = "needs a real windowed GPU device; a load-bearing pass of the RT-leg growth \
            claim requires --features hwrt on hardware that negotiates hardware ray \
            tracing; cargo test --features hwrt; --test-threads=1"]
fn rt_leg_grows_past_instance_capacity_instead_of_panicking() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(FRAMES));
    app.insert_resource(SharedCubeMesh::default());
    app.add_systems(exit_after_budget);
    app.add_startup_system(setup_minimal_scene);
    app.add_startup_system(spawn_many);
    app.add_plugins(EnginePlugins::window("boyko_app F7-hwrt RT-leg grow-past-cap headless", 320, 240));
    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner exits
    // BEFORE the frame loop, so the budget is untouched — SKIP (mirrors the sibling F7
    // file's / `asset_streaming_f6_churn_headless.rs`'s idiom).
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == FRAMES {
        eprintln!(
            "SKIP rt_leg_grows_past_instance_capacity_instead_of_panicking: windowed \
             boot unavailable"
        );
        return;
    }
    assert_eq!(
        remaining, 0,
        "the frame loop must have run the full {FRAMES}-frame budget WITHOUT a panic — \
         a > INSTANCE_CAPACITY gather must grow the instance family (non-RT or RT, \
         whichever path this device took), not panic (task#11 removed the RT hard cap)"
    );
}
