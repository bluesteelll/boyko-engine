//! **The absolute validation gate: every validated full-engine boot is clean on its own.**
//!
//! Every other validated gate in this tree compares two arms with each other, so a message present
//! in both arms is invisible to it — which is how 32 ERROR/WARNING messages sat on every validated
//! boot unseen (2026-09-18). This gate compares nothing: a worker boots the real engine through
//! `boyko_app`'s windowed runner with `VK_LAYER_KHRONOS_validation` armed, renders a few frames,
//! tears down, and is red on any ERROR (of any message type) and on any VALIDATION/PERFORMANCE
//! WARNING the layer delivered between `vkCreateInstance` and `vkDestroyInstance`. GENERAL-only
//! warnings (the loader's and the environment's channel) are counted and printed, never gated.
//! A GENERAL-only ERROR is the loader's or a layer's own failure: it is red too — an error is
//! never let through — but it is counted apart and reported as ENVIRONMENT ERROR, so the red
//! points at the machine rather than at the engine's API use.
//!
//! # The verdict is read from atomics, never from text
//!
//! `boyko_rhi_vulkan::debug::validation_ledger` is a process-wide set of counters the debug-utils
//! callback increments before it does anything else, and it is never dropped — so a message the
//! layer delivers inside `vkDestroyDevice` is still in the count after `App::run` returns. Each
//! worker reads the ledger in-process and reaches the driver ONLY as an exit code: `90` for a clean
//! path worker, `91` for a canary that heard its own leak. A worker that was filtered out or
//! skipped exits `0`, which is not a sentinel, so it is red rather than a vacuous pass. No
//! `[vk-validation]` line is parsed anywhere — those lines exist for the human reading a red.
//!
//! # Arming is witnessed, and so is hearing
//!
//! The backend arms the layer on a CONJUNCTION — `BOYKO_ENABLE_VALIDATION` set (`runner.rs`) AND
//! `BOYKO_DISABLE_VALIDATION` unset (`device.rs::validation_requested`) — and the previous gate in
//! this area ran for its whole life with only one half done. So:
//!
//! * **Armed:** `messengers_created == 1`. `create_instance` refuses a boot whose layer is absent
//!   (`ValidationUnavailable`), so a created messenger means the layer was loaded.
//! * **Teardown covered:** `messengers_destroyed == 1`. The persistent messenger is destroyed
//!   after `vkDestroyDevice`, so this proves the device-destroy window was listened to.
//! * **Heard:** the canary worker deliberately leaks a `VkShaderModule` and requires the ledger's
//!   ERROR count to grow between its last frame and the end of teardown
//!   (`VUID-vkDestroyDevice-device-05137`). Arming says the layer was loaded; only the canary says
//!   it reports to this process.
//!
//! # Running it
//!
//! ```text
//! cargo test -p boyko-app --test boot_validation_clean -- --ignored --exact \
//!     every_validated_boot_is_clean --test-threads=1 --nocapture
//! ```
//!
//! The driver scrubs every `BOYKO_*`, `VK_KHRONOS_VALIDATION_*` and `VK_LAYER_*` variable (keeping
//! only the layer-discovery paths) and the loader's layer-selection variables from each child,
//! sets exactly what the gate needs, and lifts the layer's duplicate-message limit so the counts a
//! red prints are the true ones. Worker output goes to per-worker files under
//! `temp_dir()/boyko_boot_validation/`.
//!
//! # Implicit layers are filtered out, and the filter is witnessed
//!
//! An implicit layer (a capture hook, an overlay, a vendor switchable-graphics shim) loads into
//! every Vulkan process on the machine and sits on the application side of the validation layer,
//! so its own Vulkan calls are validated and counted as this process's. Each child therefore runs
//! with `VK_LOADER_LAYERS_DISABLE=~implicit~`, which leaves only the explicitly enabled validation
//! layer. The worker refuses to boot if the filter did not reach it, and the loader itself names
//! each layer it filtered: one GENERAL-type `Layer "…" forced disabled because name matches filter
//! of env var 'VK_LOADER_LAYERS_DISABLE'` warning per layer, printed as a `[vk-validation]` line in
//! the child's own output and counted in `general_warnings` (never gated). The worker also prints
//! the GPU it booted on: device selection takes the first discrete GPU, and filtering implicit
//! layers does not change the enumerated devices (measured with `vulkaninfo --summary` on the
//! development machine, 2026-09-18: the same two GPUs in the same order either way).
//!
//! # What this gate cannot claim
//!
//! * Synchronization hazards: sync validation is measured dead on this machine.
//! * Configurations it does not boot: particles, `--features hwrt`, SSAA, SSAO, and every TAA arm
//!   but the one `deferred_taa` boots (Deferred, default `TaaConfig`): RCAS sharpening and TAA on
//!   the VisibilityBuffer path are not booted.
//! * That `deferred_taa` booted a TAA HISTORY. Its degrade witness is `JitterState.armed`, and
//!   `taa_hist` degrades silently: if allocating it (or its boot clear) fails after TAA is armed,
//!   the targets drop to no history (`build_and_clear_taa_hist` returns `None`) while
//!   `JitterState.armed` stays `true`. So the TAA worker can pass on a boot that has no history
//!   image, and on that boot the clear its `TRANSFER_DST` usage exists for never reaches a
//!   frame.
//! * A per-VUID mute installed through a loader settings file (Vulkan Configurator): the canary
//!   proves only that its own VUID is audible. An override delivered as an implicit layer is
//!   filtered out with the rest.
//! * Other GPUs and drivers.

#![cfg(windows)]

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{
    AaConfig, AaMode, BindlessTextureTable, ClusterConfig, GeometryLegs, JitterState, LightingConfig,
    Material, MaterialGpu, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
    ResolvedRenderPath, TextureGpu, load_material_folder,
};
use boyko_rhi::RhiDevice;
use boyko_rhi_vulkan::debug::{ValidationLedgerSnapshot, validation_ledger};

/// Set by the driver on every child. A worker without it prints SKIP and returns (exit `0`, which
/// the driver reads as "reached no verdict").
const DRIVER_MARKER: &str = "BOYKO_BOOT_VALIDATION_DRIVEN";

/// A path worker's exit code when its boot was clean.
const EXIT_CLEAN: i32 = 90;
/// The canary's exit code when its deliberate leak reached the ledger.
const EXIT_HEARD: i32 = 91;
/// libtest's exit code when a test panicked.
const EXIT_TEST_FAILED: i32 = 101;

/// Frames a path worker renders before it requests exit.
const PATH_BUDGET: u32 = 10;
/// Frames the canary renders before it requests exit.
const CANARY_BUDGET: u32 = 3;
/// The runner's own frame cap, set on every child — a hang cap only, far above either budget.
const HANG_CAP_FRAMES: &str = "200";

/// How long the driver waits for one child before killing it and calling it HUNG.
const WORKER_DEADLINE: Duration = Duration::from_secs(180);
/// The driver's `try_wait` poll interval.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Lines of a red worker's output the driver echoes into its own report.
const TAIL_LINES: usize = 40;

/// The file the validation layer reads from the process's working directory; its presence could
/// mute, redirect or reconfigure every message this gate counts.
const LAYER_SETTINGS_FILE: &str = "vk_layer_settings.txt";

/// The loader's layer filter, and the value every child runs with: disable every implicit layer.
const LAYERS_DISABLE_VAR: &str = "VK_LOADER_LAYERS_DISABLE";
/// The loader's keyword for "every implicit layer" in [`LAYERS_DISABLE_VAR`].
const DISABLE_IMPLICIT_LAYERS: &str = "~implicit~";

/// The committed PBR texture set the `vb_froxel` worker's textured sphere samples (the same folder
/// `vb_mesh_tex_froxel.rs` reads).
const TEXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pbr_fixtures/synth_bumps");

// ===============================================================================================
// The workers
// ===============================================================================================

/// One child process of the driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Worker {
    /// Hearing in the teardown window: leaks a shader module and requires the ERROR it causes.
    Canary,
    /// `VisibilityBuffer × Mesh` with the froxel light cull and a textured material: the largest
    /// pipeline set (every VB compute pipeline on Set 1, plus `vb_raster`).
    VbFroxel,
    /// `Forward × Both`: the sky pipeline at draw time, and the `sdf_forward_march` dispatch
    /// binding Set 1.
    ForwardBoth,
    /// `ForwardPlus × Mesh`: the depth prepass, the EQUAL-depth pipeline and the sky.
    ForwardPlus,
    /// The Deferred default with SMAA: the demote-to-helper shader at draw time.
    DeferredSmaa,
    /// The Deferred default with TAA: the `taa_hist` boot clear (a `vkCmdClearColorImage` on each
    /// history slot, legal only with `TRANSFER_DST` usage), the jittered raster and the temporal
    /// resolve dispatch.
    DeferredTaa,
}

impl Worker {
    /// Spawn order: the canary first, so a deaf oracle is the first line of the report.
    const ALL: [Worker; 6] = [
        Worker::Canary,
        Worker::VbFroxel,
        Worker::ForwardBoth,
        Worker::ForwardPlus,
        Worker::DeferredSmaa,
        Worker::DeferredTaa,
    ];

    /// The `#[test]` fn name the driver passes to `--exact`.
    const fn test_name(self) -> &'static str {
        match self {
            Worker::Canary => "boot_validation_worker_canary",
            Worker::VbFroxel => "boot_validation_worker_vb_froxel",
            Worker::ForwardBoth => "boot_validation_worker_forward_both",
            Worker::ForwardPlus => "boot_validation_worker_forward_plus",
            Worker::DeferredSmaa => "boot_validation_worker_deferred_smaa",
            Worker::DeferredTaa => "boot_validation_worker_deferred_taa",
        }
    }

    /// The exit code that means "this worker passed".
    const fn sentinel(self) -> i32 {
        match self {
            Worker::Canary => EXIT_HEARD,
            _ => EXIT_CLEAN,
        }
    }

    const fn budget(self) -> u32 {
        match self {
            Worker::Canary => CANARY_BUDGET,
            _ => PATH_BUDGET,
        }
    }
}

/// Frames left before the worker requests exit. Decremented once per Main run.
#[derive(Resource)]
struct FrameBudget(u32);

/// The ledger as it stood on the frame that requested exit — the canary's "before teardown".
#[derive(Resource)]
struct PreTeardownLedger(Option<ValidationLedgerSnapshot>);

/// Prints the GPU the boot chose — the witness that filtering implicit layers did not move the
/// run onto another device.
fn print_device(dev: NonSendRes<GpuDevice>) {
    eprintln!("BOOT-VALIDATION device: {}", dev.get().device_name());
}

/// `room_smoke.rs`'s budget system, plus the pre-teardown ledger snapshot on the exit frame.
fn exit_after_budget(
    mut budget: ResMut<FrameBudget>,
    mut exit: ResMut<AppExit>,
    mut pre_teardown: ResMut<PreTeardownLedger>,
) {
    if budget.0 > 0 {
        budget.0 -= 1;
        if budget.0 == 0 {
            exit.0 = true;
            pre_teardown.0 = Some(validation_ledger());
        }
    }
}

/// The sun direction TO the light (`room_smoke.rs`'s).
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];
/// The caster cubes' XZ placements (`room_smoke.rs`'s).
const CASTER_XZ: [(f32, f32); 4] = [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)];

fn spawn_camera(commands: &mut Commands) {
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

/// The path workers' shared scene: a floor, four `ShadowCaster` cubes, the sun, a sky fill, one
/// shadow-casting point light, one shadow-casting spot light and the camera. Meshes claim a VB
/// geometry-table slot when the boot resolved one.
fn shared_scene(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (floor, cube) = match geo_table.0.as_mut() {
        Some(table) => (meshes.plane_vb(dev.get(), 12.0, table), meshes.cube_vb(dev.get(), 1.0, table)),
        None => (meshes.plane(dev.get(), 12.0), meshes.cube(dev.get(), 1.0)),
    };
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    for (x, z) in CASTER_XZ {
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

    let point = [0.6, 1.6, -0.8];
    commands
        .spawn(PointLightObject {
            transform: Transform::from_translation(Vec3::new(point[0], point[1], point[2])),
            global: GlobalTransform::IDENTITY,
            light: PointLight::new(point, [1.0, 0.72, 0.45], 220.0, 7.0),
        })
        .insert(CastsPunctualShadow);

    let spot = [-2.5, 3.0, 3.0];
    let spot_p = Vec3::new(spot[0], spot[1], spot[2]);
    let spot_pose = Affine3A::look_at_rh(spot_p, Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.0, 1.0, 0.0));
    commands
        .spawn(SpotLightObject {
            transform: Transform {
                translation: spot_p,
                rotation: Quat::from_mat3(spot_pose.matrix3),
                scale: Vec3::ONE,
            },
            global: GlobalTransform::IDENTITY,
            light: SpotLight::new(spot, [0.0, -1.0, 0.0], [1.0, 0.85, 0.7], 200.0, 6.0, 15.0, 30.0),
        })
        .insert(CastsPunctualShadow);

    spawn_camera(&mut commands);
}

/// A UV sphere with tangents (`vb_mesh_tex_froxel.rs::uv_sphere`'s generator) — the textured
/// material needs real UVs and tangents to drive the textured VB shade.
fn uv_sphere(radius: f32, stacks: u32, slices: u32) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi;
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32;
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi);
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st];
            let mut vertex =
                Vertex::new([n[0] * radius, n[1] * radius, n[2] * radius], n, [0.7, 0.7, 0.72, 1.0]);
            vertex.uv = [j as f32 / slices as f32, v];
            verts.push(vertex);
        }
    }
    let stride = slices + 1;
    let mut idx = Vec::with_capacity((stacks * slices * 6) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * stride + j;
            let b = (i + 1) * stride + j;
            idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// The `vb_froxel` worker's addition: one sphere carrying a material loaded from the committed
/// `synth_bumps` folder, so the classified textured VB shade pipeline records.
fn vb_textured_sphere(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = uv_sphere(0.62, 28, 40);
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };
    let material_textures =
        load_material_folder(&mut textures, dev.get(), &mut bindless, Path::new(TEXTURE_DIR));
    let textured = materials.add(Material::with_textures(
        MaterialGpu::new([1.0, 1.0, 1.0, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0),
        material_textures,
    ));
    commands
        .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(-0.8, 0.62, 1.0))))
        .insert(MaterialHandle(textured.index() as u16));
}

/// The canary's scene: a floor and the camera — the smallest Deferred frame — plus the deliberate
/// leak.
///
/// The leaked module has no registry and no `Drop` today (`VulkanShaderModule` is a bare handle,
/// destroyed only by an explicit `destroy_shader_module`). It is passed to `forget` anyway, so a
/// `Drop` added to the type later cannot silently un-leak the canary and turn it deaf.
#[allow(clippy::forget_non_drop)]
fn canary_scene(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    spawn_camera(&mut commands);

    let leaked = RhiDevice::create_shader_module(dev.get(), boyko_rhi_vulkan::compute::write_pattern_spirv())
        .expect("invariant: the canary's shader module creates on a booted device");
    core::mem::forget(leaked);
}

/// The red message of a path worker whose ledger is not clean. Validation findings and
/// environment faults are named separately, so a loader or layer failure sends the reader to the
/// machine and a validation finding sends them to the engine.
fn unclean_verdict(name: &str, after: &ValidationLedgerSnapshot) -> String {
    let mut message = String::new();
    if after.errors > 0 || after.validation_warnings > 0 {
        message.push_str(&format!(
            "VALIDATION NOT CLEAN on {name}: {} error(s), {} validation/perf warning(s). The \
             [vk-validation] lines above name each VUID.\n",
            after.errors, after.validation_warnings
        ));
    }
    if after.general_errors > 0 {
        message.push_str(&format!(
            "ENVIRONMENT ERROR on {name}: {} GENERAL-type error(s) from the loader or a layer, not a \
             finding about the engine's API use: a stale layer or ICD manifest, or a layer library \
             that failed to load. The [vk-validation] lines above name each; fix the machine and \
             re-run.\n",
            after.general_errors
        ));
    }
    message.push_str(&format!(
        "({} general warning(s) not gated; {} message(s) arrived inside vkCreateInstance / \
         vkDestroyInstance; message limit off.)",
        after.general_warnings, after.instance_window
    ));
    message
}

/// Boots `worker`'s configuration, reads the ledger, asserts the protocol, and exits with the
/// worker's sentinel. Returns only when the worker was not spawned by the driver.
fn run_worker(worker: Worker) {
    let name = worker.test_name();
    if std::env::var_os(DRIVER_MARKER).is_none() {
        eprintln!(
            "SKIP {name}: {DRIVER_MARKER} is unset. This worker is spawned by \
             every_validated_boot_is_clean, which arms the validation layer for it; run that test."
        );
        return;
    }
    let layer_filter = std::env::var_os(LAYERS_DISABLE_VAR);
    assert!(
        layer_filter.as_deref() == Some(std::ffi::OsStr::new(DISABLE_IMPLICIT_LAYERS)),
        "IMPLICIT LAYERS NOT CONTROLLED ({name}: {LAYERS_DISABLE_VAR} = {layer_filter:?}, expected \
         {DISABLE_IMPLICIT_LAYERS}). Every implicit layer on the machine would load into this \
         process and have its own calls validated as the engine's."
    );
    eprintln!("BOOT-VALIDATION {name}: {LAYERS_DISABLE_VAR}={DISABLE_IMPLICIT_LAYERS}");

    let mut app = App::new();
    app.insert_resource(FrameBudget(worker.budget()));
    app.insert_resource(PreTeardownLedger(None));
    app.add_systems(exit_after_budget);
    app.add_startup_system(print_device);
    if worker == Worker::Canary {
        app.add_startup_system(canary_scene);
    } else {
        app.add_startup_system(shared_scene);
    }
    if worker == Worker::VbFroxel {
        app.add_startup_system(vb_textured_sphere);
    }
    app.add_plugins(EnginePlugins::window(name, 320, 240));
    // Configuration is inserted AFTER `add_plugins`, so it overwrites the plugins' defaults.
    if worker != Worker::Canary {
        app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
        app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    }
    match worker {
        Worker::Canary | Worker::DeferredSmaa | Worker::DeferredTaa => {}
        Worker::VbFroxel => {
            app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
            app.insert_resource(LightingConfig { clusters_enabled: true, ..LightingConfig::default() });
            app.insert_resource(ClusterConfig::default());
        }
        Worker::ForwardBoth => {
            app.insert_resource(RenderPathConfig { path: RenderPath::Forward, legs: GeometryLegs::Both });
        }
        Worker::ForwardPlus => {
            app.insert_resource(RenderPathConfig { path: RenderPath::ForwardPlus, legs: GeometryLegs::Mesh });
        }
    }
    if worker == Worker::DeferredSmaa {
        app.insert_resource(AaConfig { mode: AaMode::Smaa });
    }
    if worker == Worker::DeferredTaa {
        app.insert_resource(AaConfig { mode: AaMode::Taa });
    }

    app.run();

    // Every Vulkan object is gone here (`destroy_singleton` is the runner's last statement), so
    // nothing can call back and five separate loads are a consistent reading.
    let after = validation_ledger();
    eprintln!("BOOT-VALIDATION {name}: ledger after teardown {after:?}");

    let remaining = app.world().resource::<FrameBudget>().0;
    assert!(
        remaining == 0,
        "BOOT DID NOT REACH THE FRAME LOOP ({name}: {remaining} of {} frames left). Look for \
         boyko-E3002; with validation armed, a missing layer fails boot with \
         ValidationUnavailable, and a device missing a required feature or subgroup property \
         fails with RequiredFeatureUnsupported or RequiredSubgroupPropertyUnsupported.",
        worker.budget()
    );
    assert!(
        after.messengers_created == 1,
        "DEAD ORACLE: no validation messenger was created in this process ({name}: \
         messengers_created = {}).",
        after.messengers_created
    );
    assert!(
        after.messengers_destroyed == 1,
        "TEARDOWN WINDOW NOT COVERED: the context was not destroyed before run() returned ({name}: \
         messengers_destroyed = {}).",
        after.messengers_destroyed
    );

    if worker == Worker::Canary {
        let before = app
            .world()
            .resource::<PreTeardownLedger>()
            .0
            .expect("invariant: the exit frame stored the pre-teardown ledger (the budget reached 0)");
        let teardown_errors = after
            .errors
            .checked_sub(before.errors)
            .expect("invariant: the ledger's counters are monotonic");
        eprintln!(
            "BOOT-VALIDATION {name}: pre-teardown ledger {before:?}; teardown-window errors {teardown_errors}"
        );
        assert!(
            teardown_errors >= 1,
            "DEAF ORACLE: a deliberately leaked VkShaderModule reached vkDestroyDevice and no ERROR \
             reached the ledger. The layer is loaded but not reporting to this process (a settings \
             file, an override layer, or a broken callback or ledger)."
        );
    } else {
        let resolved = *app.world().resource::<ResolvedRenderPath>();
        let (requested, witnessed) = match worker {
            Worker::VbFroxel => (
                "VisibilityBuffer with the VB geometry table and the froxel light cull",
                resolved.path == RenderPath::VisibilityBuffer
                    && resolved.vb_geometry_table
                    && resolved.froxel_light_cull,
            ),
            Worker::ForwardBoth => (
                "Forward with the SDF leg forward-marched",
                resolved.path == RenderPath::Forward && resolved.sdf_forward_marched,
            ),
            Worker::ForwardPlus => (
                "ForwardPlus with the depth prepass",
                resolved.path == RenderPath::ForwardPlus && resolved.needs_depth_prepass,
            ),
            Worker::DeferredSmaa => ("Deferred", resolved.path == RenderPath::Deferred),
            // `JitterState.armed` is the runner's `taa_armed_now` on the last frame, the same
            // predicate `GpuSceneBundles::scene` arms `scene.taa` (and so `taa_hist`) by.
            Worker::DeferredTaa => (
                "Deferred with TAA armed",
                resolved.path == RenderPath::Deferred
                    && app.world().try_resource::<JitterState>().is_some_and(|j| j.armed),
            ),
            Worker::Canary => unreachable!("invariant: the canary took the branch above"),
        };
        assert!(
            witnessed,
            "PATH DEGRADED: requested {requested}, resolved {resolved:?}. This worker would \
             adjudicate the wrong pipelines."
        );
        assert!(
            after.errors == 0 && after.validation_warnings == 0 && after.general_errors == 0,
            "{}",
            unclean_verdict(name, &after)
        );
    }

    // `process::exit` skips libtest's own reporting and its stream flush, so flush first.
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(worker.sentinel());
}

#[test]
#[ignore = "gpu-windowed: worker spawned by every_validated_boot_is_clean; returns at once unless BOYKO_BOOT_VALIDATION_DRIVEN is set"]
fn boot_validation_worker_canary() {
    run_worker(Worker::Canary);
}

#[test]
#[ignore = "gpu-windowed: worker spawned by every_validated_boot_is_clean; returns at once unless BOYKO_BOOT_VALIDATION_DRIVEN is set"]
fn boot_validation_worker_vb_froxel() {
    run_worker(Worker::VbFroxel);
}

#[test]
#[ignore = "gpu-windowed: worker spawned by every_validated_boot_is_clean; returns at once unless BOYKO_BOOT_VALIDATION_DRIVEN is set"]
fn boot_validation_worker_forward_both() {
    run_worker(Worker::ForwardBoth);
}

#[test]
#[ignore = "gpu-windowed: worker spawned by every_validated_boot_is_clean; returns at once unless BOYKO_BOOT_VALIDATION_DRIVEN is set"]
fn boot_validation_worker_forward_plus() {
    run_worker(Worker::ForwardPlus);
}

#[test]
#[ignore = "gpu-windowed: worker spawned by every_validated_boot_is_clean; returns at once unless BOYKO_BOOT_VALIDATION_DRIVEN is set"]
fn boot_validation_worker_deferred_smaa() {
    run_worker(Worker::DeferredSmaa);
}

#[test]
#[ignore = "gpu-windowed: worker spawned by every_validated_boot_is_clean; returns at once unless BOYKO_BOOT_VALIDATION_DRIVEN is set"]
fn boot_validation_worker_deferred_taa() {
    run_worker(Worker::DeferredTaa);
}

// ===============================================================================================
// The driver
// ===============================================================================================

/// How one child ended, classified from its exit code alone.
#[derive(Debug)]
enum Outcome {
    /// Exited with its sentinel.
    Pass,
    /// Exited `0`: the worker returned without a verdict — skipped or filtered out.
    NoVerdict,
    /// Exited `101`: one of the worker's assertions failed.
    AssertionFailed,
    /// Any other exit code, or none: the process died.
    Died(Option<i32>),
    /// Still running at the deadline; killed.
    Hung,
    /// The child could not be spawned or waited on.
    DriverError(String),
}

impl Outcome {
    fn from_status(status: ExitStatus, sentinel: i32) -> Self {
        match status.code() {
            Some(code) if code == sentinel => Outcome::Pass,
            Some(0) => Outcome::NoVerdict,
            Some(EXIT_TEST_FAILED) => Outcome::AssertionFailed,
            code => Outcome::Died(code),
        }
    }

    fn describe(&self) -> String {
        match self {
            Outcome::Pass => "pass".to_owned(),
            Outcome::NoVerdict => "reached no verdict: skipped or filtered out (exit 0)".to_owned(),
            Outcome::AssertionFailed => "worker assertion failed; see its output (exit 101)".to_owned(),
            Outcome::Died(code) => format!(
                "worker process died (the layer or the driver), exit {code:?}. Not classified as \
                 environment."
            ),
            Outcome::Hung => format!("HUNG: still running after {} s; killed", WORKER_DEADLINE.as_secs()),
            Outcome::DriverError(e) => format!("the driver could not run the worker: {e}"),
        }
    }
}

/// Whether an environment variable must not reach a child. Windows variable names are
/// case-insensitive, so the comparison is on the upper-cased name.
fn scrubbed(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("BOYKO_") || upper.starts_with("VK_KHRONOS_VALIDATION_") {
        return true;
    }
    if upper.starts_with("VK_LAYER_") {
        // The two discovery variables say WHERE layers live, not which run or how they speak.
        return upper != "VK_LAYER_PATH" && upper != "VK_ADD_LAYER_PATH";
    }
    // `VK_LOADER_LAYERS_ALLOW` overrides the disable filter, so an inherited value could re-admit
    // the implicit layers the driver filters out; `VK_LOADER_LAYERS_DISABLE` is re-set below.
    matches!(
        upper.as_str(),
        "VK_INSTANCE_LAYERS" | "VK_LOADER_LAYERS_ENABLE" | "VK_LOADER_LAYERS_DISABLE" | "VK_LOADER_LAYERS_ALLOW"
    )
}

/// Spawns `worker` in this same test binary with the scrubbed, armed environment, waits for it
/// under the watchdog, and classifies the exit.
fn run_child(worker: Worker, out_dir: &Path, cwd: &Path) -> (Outcome, PathBuf, PathBuf) {
    let name = worker.test_name();
    let stdout_path = out_dir.join(format!("{name}.stdout.txt"));
    let stderr_path = out_dir.join(format!("{name}.stderr.txt"));
    let files = File::create(&stdout_path).and_then(|o| File::create(&stderr_path).map(|e| (o, e)));
    let (stdout, stderr) = match files {
        Ok(pair) => pair,
        Err(e) => return (Outcome::DriverError(format!("output files: {e}")), stdout_path, stderr_path),
    };

    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    // `--nocapture`: libtest's capture buffer is discarded by `process::exit` and by a watchdog
    // kill, which would lose every `[vk-validation]` line of exactly the runs that need them.
    cmd.args([name, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    for (key, _) in std::env::vars_os() {
        if scrubbed(&key.to_string_lossy()) {
            cmd.env_remove(&key);
        }
    }
    // Both halves of the backend's arming conjunction: the first is set here, the second
    // (`BOYKO_DISABLE_VALIDATION`) was removed with every other `BOYKO_*` above.
    cmd.env("BOYKO_ENABLE_VALIDATION", "1")
        .env(DRIVER_MARKER, "1")
        .env("BOYKO_WINDOW_FRAMES", HANG_CAP_FRAMES)
        // Diagnostic truth only: the verdict is 0-vs-nonzero either way, but a capped count is
        // not the count a reader needs to triage a red.
        .env("VK_KHRONOS_VALIDATION_ENABLE_MESSAGE_LIMIT", "false")
        // Only the explicitly enabled validation layer loads (see the module doc); the worker
        // asserts this value arrived.
        .env(LAYERS_DISABLE_VAR, DISABLE_IMPLICIT_LAYERS);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (Outcome::DriverError(format!("spawn: {e}")), stdout_path, stderr_path),
    };
    let deadline = Instant::now() + WORKER_DEADLINE;
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Outcome::from_status(status, worker.sentinel()),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break Outcome::Hung;
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Outcome::DriverError(format!("wait: {e}"));
            }
        }
    };
    (outcome, stdout_path, stderr_path)
}

/// The last [`TAIL_LINES`] lines of `path`, for a red worker's report.
fn tail(path: &Path) -> String {
    let Ok(file) = File::open(path) else {
        return format!("    <cannot open {}>\n", path.display());
    };
    let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    let mut out = String::new();
    for line in &lines[start..] {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// **The driver.** Spawns the canary, then the five path workers, each in its own process with the
/// validation layer armed, and asserts once over all six outcomes.
#[test]
#[ignore = "gpu-windowed: live GPU gate; the driver arms VK_LAYER_KHRONOS_validation itself and spawns six windowed workers; run with --test-threads=1"]
fn every_validated_boot_is_clean() {
    let cwd = std::env::current_dir().expect("invariant: the test process has a working directory");
    let settings = cwd.join(LAYER_SETTINGS_FILE);
    assert!(
        !settings.exists(),
        "PRECONDITION: {} exists in the workers' working directory. The validation layer reads it \
         and it can mute or redirect the messages this gate counts; remove it and re-run.",
        settings.display()
    );

    let out_dir = std::env::temp_dir().join("boyko_boot_validation");
    std::fs::create_dir_all(&out_dir).expect("invariant: the temp dir accepts a subdirectory");

    let mut report = String::new();
    let mut red = 0usize;
    for worker in Worker::ALL {
        let started = Instant::now();
        let (outcome, stdout_path, stderr_path) = run_child(worker, &out_dir, &cwd);
        let line = format!(
            "{:<40} {:<6} {:>5.1} s  {}\n",
            worker.test_name(),
            if matches!(outcome, Outcome::Pass) { "GREEN" } else { "RED" },
            started.elapsed().as_secs_f64(),
            outcome.describe()
        );
        eprint!("{line}");
        report.push_str(&line);
        if !matches!(outcome, Outcome::Pass) {
            red += 1;
            report.push_str(&format!("  stderr tail ({}):\n", stderr_path.display()));
            report.push_str(&tail(&stderr_path));
            report.push_str(&format!("  stdout tail ({}):\n", stdout_path.display()));
            report.push_str(&tail(&stdout_path));
        }
    }

    assert!(
        red == 0,
        "{red} of {} validated boots are not clean (worker output: {}):\n{report}",
        Worker::ALL.len(),
        out_dir.display()
    );
}
