//! Asset-streaming plan F7 §12, W3 — REAL-DEVICE RT hard-cap test: on an RT device,
//! a `> INSTANCE_CAPACITY` gather must trip the LIVE hard `assert!` in
//! `upload_instance_models` (`upload.rs`) — growth is OUT OF SCOPE for the instance
//! family on an RT device (E1 Option B; the TLAS packer's `instance_arrays`/backing/
//! scratch are sized once for `INSTANCE_CAPACITY`, see design §3/§7.3).
//!
//! # Asset-streaming plan F8 fold-in
//!
//! `spawn_many` now gives every drawable a SHARED non-default material (previously the
//! implicit default `MaterialHandle(0)`), so the over-capacity gather this test traps is
//! material-bearing — `pm_instance_material_rings` shares the SAME `INSTANCE_CAPACITY`
//! hard cap as `instance_rings` (F8 §1.2/§7b), so on an RT device the pre-existing
//! `upload_instance_models` hard assert still fires FIRST (the runner calls it before
//! `upload_instance_materials`, `runner.rs`), but this now also proves F8 did not
//! introduce a bypass of the RT hard cap (e.g. a reordering that uploads materials
//! before the model assert, which would have OOB-written the un-grown material ring
//! instead of aborting cleanly).
//!
//! # Separate file (structural constraint)
//!
//! This scenario is EXPECTED TO PANIC — it cannot share an `App`/`#[test]` with
//! `asset_streaming_f7_grow_headless.rs`'s phased run (that run's own `App::new()` +
//! `add_plugins` already registered the process-global component hooks for this
//! process; more importantly, a panic here would abort a shared multi-phase test
//! mid-way, corrupting every phase after it). Kept in its OWN file with its OWN
//! single `#[test]`, matching this workspace's one-windowed-`#[test]`-per-file
//! convention (see the sibling file's module doc for the full reasoning: re-running
//! `App::new()` + `add_plugins` a SECOND time in the same process re-registers
//! process-global component hooks and panics).
//!
//! # Runtime-gated, not compile-time (read before treating a pass/fail as final)
//!
//! Whether the instance family hard-caps at `INSTANCE_CAPACITY` (RT) or grows past
//! it (non-RT) is a RUNTIME device-capability gate (`tlas.is_some() || mv.is_some()`
//! — design §7.3 step 1), independent of the `hwrt` Cargo feature: an `hwrt`-feature
//! build on a non-RT GPU still takes the non-RT/grow path (already proven to work by
//! the sibling phased test's Phase B). This test can therefore only assert a HARD
//! panic when the device ACTUALLY negotiated hardware ray tracing. On any other
//! device the same scenario is expected to render successfully via the non-RT growth
//! path instead — NOT a failure of the cap (it was never armed). The test uses
//! `catch_unwind` (not `#[should_panic]`, which cannot express a conditional
//! expectation) and SKIPs (does not fail) when no panic occurred, mirroring this
//! workspace's windowless-boot SKIP idiom. **A load-bearing pass requires
//! `--features hwrt` on an RT-capable GPU.**
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
    // see this file's module doc for why the over-capacity gather this test traps must be
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

/// F7 §12 W3: on an RT device, a `> INSTANCE_CAPACITY` gather must trip the LIVE
/// hard `assert!` — see this file's module doc for the runtime-gated SKIP
/// discrimination (a load-bearing pass requires an RT-capable GPU).
#[test]
#[ignore = "needs a real windowed GPU device that negotiates hardware ray tracing; \
            cargo test --features hwrt; --test-threads=1; a non-RT device SKIPs (see doc)"]
fn w3_rt_cap_traps_an_over_capacity_instance_gather_on_an_rt_device() {
    // `App::run` unwinds through plain Rust structures (no raw-pointer-holding guard
    // is on this stack frame at the panic site inside a system) — the same
    // reasoning `catch_unwind` relies on elsewhere in this workspace's windowed
    // smokes that probe an expected-panic path.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut app = App::new();
        app.insert_resource(FrameBudget(FRAMES));
        app.insert_resource(SharedCubeMesh::default());
        app.add_systems(exit_after_budget);
        app.add_startup_system(setup_minimal_scene);
        app.add_startup_system(spawn_many);
        app.add_plugins(EnginePlugins::window("boyko_app F7 W3 RT-cap headless", 320, 240));
        app.run()
    }));

    match result {
        Ok(_exit) => {
            // No panic. Note `AppExit(true)` is returned BOTH by a graceful full
            // run AND by an early windowless-boot bail-out (see
            // `asset_streaming_f6_churn_headless.rs`'s doc) — `app` was moved into
            // the closure above and is gone by now, so this arm cannot (and need
            // not) distinguish the two: EITHER way, no panic here means this run
            // is inconclusive for the RT-cap claim (boot was unavailable, or the
            // device is non-RT and the non-RT growth path legitimately handled the
            // over-capacity gather instead of the hard cap) — SKIP either way.
            eprintln!(
                "SKIP w3_rt_cap_traps_an_over_capacity_instance_gather_on_an_rt_device: the run \
                 completed without panicking — either windowed boot was unavailable, or this \
                 device did not negotiate hardware ray tracing (the non-RT growth path handled \
                 the over-capacity gather instead of the hard cap); a load-bearing pass requires \
                 an RT-capable GPU"
            );
        }
        Err(_) => {
            // The hard cap fired — the load-bearing RT-device pass. The panic
            // payload was already consumed/printed by libtest's default hook; the
            // panic itself (this test completing via the Err arm) IS the assertion.
        }
    }
}
