//! **The Deferred SDF marcher must shadow toward the scene's primary directional light, read every
//! frame — not toward a boot constant, not toward a value latched at spawn, and not at all when the
//! scene has no directional light.** The device gate G3 of defect R2
//! (`docs/render/light-table-defects/R2-DESIGN.md`, as amended by `00-RULINGS.md`).
//!
//! # The defect it pins
//!
//! On Deferred the marcher (`sdf_gbuffer_composite.hlsl`) marches the analytic SDF soft shadow toward
//! its push field `light_dir` and writes the result to `gMaterial.r`, which the resolve takes as the
//! PRIMARY directional's visibility — the first directional row of the light table — and, while
//! punctual shadows are off, as every point/spot light's visibility too. `gpu_scene` filled that push
//! field from a boot constant, `[-0.45, 0.82, 0.36]`, so on any scene whose sun is
//! elsewhere the shadow is cast toward a sun that does not exist, and on a sunless scene the point
//! lights are masked by it.
//!
//! # The fixture
//!
//! Deferred × Both, AA off (the engine default), a `WIN`² window. Every run is one scene:
//!
//! * a 12-unit mesh floor at `y = 0` with **no `ShadowCaster`**, so no cascade or atlas pass arms and
//!   the marcher is the only shadow source (the probe's header word 7 bits 2 and 3 must read clear);
//! * one SDF sphere, radius [`SPHERE_RADIUS`], centred [`SPHERE_CENTER`] — 1.5 units above the floor;
//! * a camera straight above the sphere, looking down ([`EYE`]), so every candidate shadow spot is on
//!   screen and none is behind the sphere's own silhouette (which covers ~0.4 units around the
//!   origin; the nearest spot is 1.05 units out).
//!
//! **The light set is exactly what each scene below lists, and there is no `SkyLight`**: with no
//! ambient term an umbra is black, and the thresholds do not depend on an ambient level.
//!
//! | scene | lights | occluder |
//! |---|---|---|
//! | `sun` | the main sun (spawns at B, rotated to A at frame [`ROTATE_AT_FRAME`]), then a dimmer second directional at D | sphere |
//! | `sun_no_occluder` | the same two suns, the same rotation | none |
//! | `point` | one point light at [`POINT_POS`], no directional; punctual shadows off (the default) | sphere |
//! | `point_no_occluder` | the same point light | none |
//! | `dark` | none | sphere |
//!
//! The main sun is spawned before the dim one, into the same archetype, so it is the table's first
//! directional row — the resolve's primary. The worker records the staged table's directional rows at
//! the probe frame and the driver checks that premise (row 0 ≈ A, row 1 ≈ D) before reading a verdict.
//!
//! Both suns spawn with their `GlobalTransform` already posed to match their `Transform`, so the
//! table carries B from frame 0 — the frame of the first `scene()` call — rather than the identity
//! pose's `[0, 0, -1]`. That is what lets T10 name a spawn-time latch (M7) as B.
//!
//! # Measurement space and thresholds (derived from control runs, not guessed)
//!
//! The frame is the runner's `BOYKO_HOST_DUMP` BMP: the resolve's output after the Hill ACES tonemap
//! and the manual gamma-2.2 OETF (`pbr_lighting.hlsli`), written to an UNORM swapchain. A probe decodes
//! each channel with `(c / 255)^2.2` — back to display-linear, post-tonemap, NOT scene-linear — and
//! takes the Rec.709 luminance, averaged over a 5 × 5 patch centred on the projected world point.
//!
//! Each probe is normalised by two CONTROL runs of the same build: `Y_o` from the occluder-removed
//! scene (the spot's fully-lit level) and `Y_k` from `dark` (the no-light level):
//! `v = (Y − Y_k) / (Y_o − Y_k)`. `v` is the spot's visibility as the frame shows it, whatever the
//! floor's albedo, the tonemap's curve or the light intensities, so the verdict thresholds are
//! fractions of a measured range: a spot is DARK at `v < `[`DARK_MAX`] and LIT at `v > `[`LIT_MIN`];
//! anything between is AMBIGUOUS and red. The one absolute number, [`MIN_RANGE`], is an instrument
//! floor on the controls themselves: a probe whose lit and dark controls differ by less has no range
//! to normalise by, and the run is red for that reason, named.
//!
//! ## `marcher_shadow_follows_the_table_sun_after_runtime_rotation` (T10)
//!
//! Runs `sun`, `sun_no_occluder`, `dark`. The candidate spots are the umbra centres of the sphere
//! under A (the table's primary at the dump frame), B (the main sun's spawn direction), C (the boot
//! constant) and D (the second, dimmer directional), plus a reference R on open floor.
//!
//! * R is LIT.
//! * Exactly one of {A, B, C, D} is DARK and the other three are LIT, and the dark one is A.
//!
//! Only one spot can be dark in a single frame: the marcher writes one shadow, and the resolve applies
//! it to the primary AND (outside multi-light mode, the default) to the extra directional, so under the
//! marcher's umbra both suns are masked and the floor is black. The dark spot names the marcher's sun:
//! **C** — the boot constant (R2's defect); **B** — the direction was latched at spawn (M7); **D** — the
//! marcher took a directional that is not the table's primary row; **none** — no SDF shadow fell on
//! any candidate: shadows off, no occluder, or the marcher shadowed toward a direction none of the
//! four names (for instance one latched before the sun's pose reached the table, whose umbra is off
//! the probes or off the floor), so the frame proves nothing about which sun the marcher took.
//!
//! ## `sunless_scene_has_no_phantom_marcher_shadow` (T11)
//!
//! Runs `point`, `point_no_occluder`, `dark`. C is the boot constant's umbra centre; C′ is C mirrored
//! through the point light's vertical axis, which the camera shares, so it takes the same irradiance
//! and the same view.
//!
//! * C′ is LIT, C is LIT, and `v(C) ≥ `[`SYMMETRY_MIN`]` · v(C′)`.
//!
//! With no directional there is no sun shadow to cast, so nothing may darken C. A C darker than C′ is
//! the phantom boot-constant sun masking the point light (`deferred_pbr.hlsl`'s `vis = shadow` rule
//! for punctual lights while punctual shadows are off).
//!
//! # Mutation receipts
//!
//! Named after the design's mutation set, so they cannot be confused with this lane's R1-M* set.
//!
//! * **The pre-fix tree:** T10 red, C dark; T11 red, C dark.
//! * **M6**, `scene()` pushes the constant again: T10 red, C dark.
//! * **M7**, the primary latched on the first `scene()` call: T10 red, B dark. This relies on the
//!   suns being posed at spawn (see the fixture). With `GlobalTransform::IDENTITY` at spawn, measured
//!   2026-09-19: the first call received `[0, 0, -1]`, the same latch left NO spot dark, and T10's
//!   message blamed "shadows off, or no occluder"; B went dark only when the latch was moved to the
//!   second call.
//! * **M4**, SHADOWS kept on a sunless frame: the `[0, 0, 1]` placeholder gives `dot(n, L) = 0` on the
//!   floor, so the marcher's back-face early-out writes visibility 0 there with or without an
//!   occluder. `point_no_occluder` is then black, and T11 is red on [`MIN_RANGE`] at both probes.
//!
//! # What it cannot claim
//!
//! * The resolve's side of the agreement — that its primary is the first directional row — is read
//!   from the shaders, not measured here; the device-free gate G1, which lands with the fix
//!   (`boyko_render/tests/primary_directional.rs`), pins the host's copy of that rule.
//! * Multi-light mode (an extra flagged directional marching its own shadow) and punctual shadows on:
//!   both are off in every scene here, and R2 changes neither.
//! * Other GPUs and drivers.
//!
//! # Running it
//!
//! ```text
//! cargo test -p boyko-app --test sdf_marcher_sun -- --ignored --test-threads=1 \
//!     marcher_shadow_follows_the_table_sun_after_runtime_rotation sunless_scene_has_no_phantom_marcher_shadow
//! ```
//!
//! On both legs: add `--features hwrt` for the second. The drivers scrub every `BOYKO_*` variable
//! from each child and set exactly what the gate needs (`BOYKO_DISABLE_VALIDATION=1`,
//! `BOYKO_SHADOW_DENOISE=none`, AA unset). Worker output and the per-run BMP / probe land under
//! `temp_dir()/boyko_sdf_marcher_sun/`, deleted before each run so a stale file can never stand in
//! for one this run did not write. **No hash is stored**: every assertion compares runs of one build.
//!
//! SINGLE SCENE PER PROCESS: `EnginePlugins` composes `LightingPlugin`, whose light eviction hooks
//! are process-global, and the device singleton boots once — so every scene runs in its own worker
//! process, re-executed from this binary (`unwritten_shadow_map_gate.rs`'s pattern).

#![cfg(windows)]

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::light_system::{GPU_LIGHT_BYTES, LIGHT_HEADER_BYTES, LightTableStaging};
use boyko_render::{GeometryLegs, RenderEpoch, RenderPath, RenderPathConfig, ResolvedRenderPath};

/// Set by a driver on every child. A worker without it prints SKIP and returns (exit `0`, which the
/// driver reads as "reached no verdict", never as a pass).
const DRIVER_MARKER: &str = "BOYKO_MARCHER_SUN_DRIVEN";
/// The worker's scene (see [`GateScene`]).
const ENV_SCENE: &str = "BOYKO_MARCHER_SUN_SCENE";
/// Where the worker writes its probe record.
const ENV_PROBE: &str = "BOYKO_MARCHER_SUN_PROBE";
/// The runner's frame dump (the frame the probes read).
const ENV_HOST_DUMP: &str = "BOYKO_HOST_DUMP";

/// The worker's exit code when it rendered and both artifacts exist.
const EXIT_RENDERED: i32 = 92;
/// libtest's exit code when a test panicked.
const EXIT_TEST_FAILED: i32 = 101;
/// The worker test's name, passed to `--exact`.
const WORKER: &str = "marcher_sun_worker";
/// Window side in pixels.
const WIN: u32 = 512;
/// The runner's own frame cap — a hang cap only, far above the dump's 34 presented frames.
const HANG_CAP_FRAMES: &str = "400";
/// How long a driver waits for one child before killing it and calling it HUNG.
const WORKER_DEADLINE: Duration = Duration::from_secs(300);
/// The driver's `try_wait` poll interval.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The [`RenderEpoch`] from which the main sun is rotated from B to A.
const ROTATE_AT_FRAME: u64 = 5;
/// The [`RenderEpoch`] at which the worker records the staged table: `host_dump`'s `SETTLE_FRAMES`,
/// the frame whose readback the dump requests (`taa_jitter_eval.rs`'s `CAPTURE_FRAME` derivation).
const PROBE_AT_FRAME: u64 = 30;

/// Header word 7's CSM sample bit (`boyko_render::CSM_MODE_BIT`).
const CSM_BIT: u32 = 1 << 2;
/// Header word 7's punctual sample bit (`boyko_render::PUNCTUAL_MODE_BIT`).
const PUNCTUAL_BIT: u32 = 1 << 3;
/// The kind enum's bits in a row's kind word (`light_table.hlsli`'s `LIGHT_KIND_MASK`).
const KIND_MASK: u32 = 0xFFFF;
/// The directional kind tag (`light_table.hlsli`'s `LIGHT_KIND_DIRECTIONAL`).
const KIND_DIRECTIONAL: u32 = 0;
/// `u32` words per light row.
const ROW_WORDS: usize = GPU_LIGHT_BYTES / 4;

// ===============================================================================================
// The scene
// ===============================================================================================

/// The occluder: an SDF sphere 1.5 units above the floor.
const SPHERE_CENTER: [f32; 3] = [0.0, 1.5, 0.0];
const SPHERE_RADIUS: f32 = 0.3;
/// B: the main sun's spawn direction (TO the light).
const SUN_SPAWN_B: [f32; 3] = [0.0, 0.8, -0.6];
/// A: the main sun's direction from [`ROTATE_AT_FRAME`] on — the table's primary at the dump frame.
const SUN_ROTATED_A: [f32; 3] = [0.6, 0.8, 0.0];
/// D: the second, dimmer directional (not unit length; every consumer normalises it).
const SUN_SECOND_D: [f32; 3] = [0.3, 0.8, 0.5];
/// C: the boot constant `gpu_scene` pushed as the marcher's sun before R2.
const BOOT_CONSTANT_C: [f32; 3] = [-0.45, 0.82, 0.36];
/// The main sun's illuminance, which is also how the rotation system tells it from the dim one
/// (the two share an archetype on purpose; see the module doc).
const MAIN_ILLUMINANCE: f32 = 2.8;
/// The second directional's illuminance.
const SECOND_ILLUMINANCE: f32 = 0.9;
/// R: open floor, lit under every candidate sun (every candidate's shadow ray from here passes the
/// sphere's centre at about 1.2 units or more, four radii).
const REFERENCE_R: [f32; 3] = [1.2, 0.0, 0.9];
/// T11's point light: on the camera's axis, above the sphere.
const POINT_POS: [f32; 3] = [0.0, 4.0, 0.0];
/// T11's point light power (lumens; about 2.2 lux on the floor at the probes).
const POINT_POWER: f32 = 500.0;
/// T11's point light range: the whole visible floor is inside it.
const POINT_RANGE: f32 = 12.0;
/// The camera: straight above the sphere, looking down, image up = world `-Z`.
const EYE: [f32; 3] = [0.0, 6.0, 0.0];
const TARGET: [f32; 3] = [0.0, 0.0, 0.0];
const UP_HINT: [f32; 3] = [0.0, 0.0, -1.0];
const FOV_Y: f32 = core::f32::consts::FRAC_PI_3;

/// Which scene the worker renders (`ENV_SCENE`). See the module doc's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateScene {
    Sun,
    SunNoOccluder,
    Point,
    PointNoOccluder,
    Dark,
}

impl GateScene {
    /// The `ENV_SCENE` spelling.
    fn name(self) -> &'static str {
        match self {
            Self::Sun => "sun",
            Self::SunNoOccluder => "sun_no_occluder",
            Self::Point => "point",
            Self::PointNoOccluder => "point_no_occluder",
            Self::Dark => "dark",
        }
    }

    /// The scene an `ENV_SCENE` value names.
    fn parse(name: &str) -> Self {
        [Self::Sun, Self::SunNoOccluder, Self::Point, Self::PointNoOccluder, Self::Dark]
            .into_iter()
            .find(|s| s.name() == name)
            .unwrap_or_else(|| panic!("{ENV_SCENE}={name:?} names no scene"))
    }

    fn has_occluder(self) -> bool {
        matches!(self, Self::Sun | Self::Point | Self::Dark)
    }

    fn has_suns(self) -> bool {
        matches!(self, Self::Sun | Self::SunNoOccluder)
    }

    fn has_point(self) -> bool {
        matches!(self, Self::Point | Self::PointNoOccluder)
    }
}

/// A directional light's pose: its `-Z` aimed along `dir` (TO the light), which is what
/// `light_reconcile` reads back into the light's direction.
fn sun_pose(dir: [f32; 3]) -> Affine3A {
    Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(dir[0], dir[1], dir[2]), Vec3::new(0.0, 1.0, 0.0))
}

/// [`sun_pose`] as the local `Transform` that propagation recomposes and the rotation system writes.
fn sun_transform(dir: [f32; 3]) -> Transform {
    let pose = sun_pose(dir);
    Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE }
}

/// The scene `which` names. The main sun is spawned first, so it is the table's first directional.
fn scene(which: GateScene, mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));

    if which.has_occluder() {
        commands.spawn(SdfPrimitive(SdfEdit::sphere(SPHERE_CENTER, SPHERE_RADIUS, sdf_op::UNION, 0.0)));
    }
    if which.has_suns() {
        for (dir, color, illuminance) in [
            (SUN_SPAWN_B, [1.0, 0.96, 0.90], MAIN_ILLUMINANCE),
            (SUN_SECOND_D, [1.0, 1.0, 1.0], SECOND_ILLUMINANCE),
        ] {
            // Posed at spawn, not `IDENTITY`: the frame-0 table is the first `scene()` call's input,
            // and with an identity global it carried `[0, 0, -1]` (measured), a horizontal sun with
            // no umbra on the floor. See the M7 receipt in the module doc.
            commands.spawn(DirectionalLightObject {
                transform: sun_transform(dir),
                global: GlobalTransform(sun_pose(dir)),
                light: DirectionalLight::new(dir, color, illuminance),
            });
        }
    }
    if which.has_point() {
        commands.spawn(PointLightObject {
            transform: Transform::from_translation(Vec3::new(POINT_POS[0], POINT_POS[1], POINT_POS[2])),
            global: GlobalTransform::IDENTITY,
            light: PointLight::new(POINT_POS, [1.0, 1.0, 1.0], POINT_POWER, POINT_RANGE),
        });
    }

    let eye = Affine3A::look_at_rh(
        Vec3::new(EYE[0], EYE[1], EYE[2]),
        Vec3::new(TARGET[0], TARGET[1], TARGET[2]),
        Vec3::new(UP_HINT[0], UP_HINT[1], UP_HINT[2]),
    );
    commands.spawn(CameraRig {
        transform: Transform { translation: eye.translation, rotation: Quat::from_mat3(eye.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: FOV_Y, aspect: 1.0, near: 0.1, far: 100.0 },
    });
}

fn scene_sun(commands: Commands, meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    scene(GateScene::Sun, commands, meshes, dev);
}

fn scene_sun_no_occluder(commands: Commands, meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    scene(GateScene::SunNoOccluder, commands, meshes, dev);
}

fn scene_point(commands: Commands, meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    scene(GateScene::Point, commands, meshes, dev);
}

fn scene_point_no_occluder(commands: Commands, meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    scene(GateScene::PointNoOccluder, commands, meshes, dev);
}

fn scene_dark(commands: Commands, meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    scene(GateScene::Dark, commands, meshes, dev);
}

/// Rotates the main sun from B to A once [`RenderEpoch`] reaches [`ROTATE_AT_FRAME`]. Joins
/// `CameraSet::Control`, so `propagate_transforms` recomposes the `GlobalTransform` the same frame,
/// and writes through [`Mut::set_if_neq`], which stamps the change tick `light_reconcile` reads.
#[allow(clippy::needless_pass_by_value)]
fn rotate_main_sun(epoch: Res<RenderEpoch>, mut suns: Query<(&DirectionalLight, Mut<Transform>)>) {
    if epoch.0 < ROTATE_AT_FRAME {
        return;
    }
    let pose = sun_transform(SUN_ROTATED_A);
    for (light, mut transform) in suns.iter_mut() {
        if light.illuminance.to_bits() == MAIN_ILLUMINANCE.to_bits() {
            transform.set_if_neq(pose);
        }
    }
}

/// Where the probe goes, and whether it was written.
#[derive(Resource)]
struct ProbeOut {
    path: PathBuf,
    written: bool,
}

/// One native-endian `u32` word of the staged table (the bytes are `write_pod`'s POD image).
fn table_word(bytes: &[u8], word: usize) -> u32 {
    let at = word * 4;
    u32::from_ne_bytes(bytes[at..at + 4].try_into().expect("invariant: a 4-byte slice"))
}

/// Records, once, at [`PROBE_AT_FRAME`]: the resolved path and legs, the frame, header word 7, and
/// the direction bits of every directional row in the table's `[0, l0a_count)` block, in table
/// order. A hand parse of the staged bytes — this gate's own oracle, independent of any host helper.
#[allow(clippy::needless_pass_by_value)]
fn write_probe(
    epoch: Res<RenderEpoch>,
    staging: Res<LightTableStaging>,
    resolved: Res<ResolvedRenderPath>,
    mut out: ResMut<ProbeOut>,
) {
    if out.written || epoch.0 < PROBE_AT_FRAME {
        return;
    }
    let bytes = staging.bytes();
    assert!(bytes.len() >= LIGHT_HEADER_BYTES, "the staged table holds at least its header");
    let rows = (bytes.len() - LIGHT_HEADER_BYTES) / GPU_LIGHT_BYTES;
    let l0a = table_word(bytes, 2) as usize;
    let mut dirs: Vec<String> = Vec::new();
    for row in 0..l0a.min(rows) {
        let base = LIGHT_HEADER_BYTES / 4 + row * ROW_WORDS;
        if table_word(bytes, base + 3) & KIND_MASK == KIND_DIRECTIONAL {
            for lane in 0..3 {
                dirs.push(format!("{:#010x}", table_word(bytes, base + lane)));
            }
        }
    }
    let text = format!(
        "path = \"{:?}\"\nlegs = \"{:?}\"\nmesh_leg = {}\nepoch = {}\nheader_word7 = {:#010x}\n\
         l0a_count = {l0a}\ndirectional_dirs = [{}]\n",
        resolved.path,
        resolved.legs,
        resolved.mesh_leg,
        epoch.0,
        table_word(bytes, 7),
        dirs.join(", ")
    );
    std::fs::write(&out.path, text)
        .unwrap_or_else(|e| panic!("the probe could not be written to {} ({e})", out.path.display()));
    out.written = true;
}

/// Reads a required worker variable, panicking with its name when the driver did not set it.
fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("the driver did not set {name}"))
}

#[test]
#[ignore = "gpu-windowed: worker for sdf_marcher_sun, not a standalone check"]
fn marcher_sun_worker() {
    if std::env::var_os(DRIVER_MARKER).is_none() {
        eprintln!(
            "SKIP {WORKER}: {DRIVER_MARKER} is unset. This worker is spawned by the sdf_marcher_sun \
             drivers, which set its configuration; run those."
        );
        return;
    }
    let which = GateScene::parse(&required_env(ENV_SCENE));
    let dump = PathBuf::from(required_env(ENV_HOST_DUMP));
    let probe = PathBuf::from(required_env(ENV_PROBE));
    eprintln!("MARCHER-SUN worker: scene={which:?}");

    let mut app = App::new();
    // `add_plugins` FIRST, then the startup system: the order a scene carrying an `SdfPrimitive`
    // needs (`sdf_room_smoke.rs`, `taa_jitter_eval.rs`).
    app.add_plugins(EnginePlugins::window(WORKER, WIN, WIN));
    match which {
        GateScene::Sun => app.add_startup_system(scene_sun),
        GateScene::SunNoOccluder => app.add_startup_system(scene_sun_no_occluder),
        GateScene::Point => app.add_startup_system(scene_point),
        GateScene::PointNoOccluder => app.add_startup_system(scene_point_no_occluder),
        GateScene::Dark => app.add_startup_system(scene_dark),
    };
    app.insert_resource(RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Both });
    app.insert_resource(ProbeOut { path: probe.clone(), written: false });
    app.add_systems_cfg(|b| {
        b.add_system(rotate_main_sun).in_set(CameraSet::Control);
        b.add_system(write_probe);
    });
    app.run();

    // The run must have reached the dump frame: a boot that failed (no device, a refused feature)
    // returns from `run` without either artifact, and that must be a red, not a skip.
    assert!(
        dump.is_file() && probe.is_file(),
        "{WORKER} returned without its artifacts (dump {}: {}, probe {}: {}). A boot that fails or \
         never reaches the dump frame is an instrument failure; look for boyko-E3002 above.",
        dump.display(),
        dump.is_file(),
        probe.display(),
        probe.is_file()
    );
    // `process::exit` skips libtest's own reporting and its stream flush, so flush first.
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(EXIT_RENDERED);
}

// ===============================================================================================
// The drivers
// ===============================================================================================

/// A spot is DARK below this normalised visibility.
const DARK_MAX: f64 = 0.5;
/// A spot is LIT above this normalised visibility.
const LIT_MIN: f64 = 0.85;
/// T11: `v(C)` may not fall below this fraction of `v(C′)`.
const SYMMETRY_MIN: f64 = 0.9;
/// The instrument floor on a probe's control range `Y_o − Y_k`, in display-linear luminance. The
/// lit floor sits near 0.5 in this space and the no-light floor at 0, so only a fixture that lights
/// nothing at the probe (or renders its occluder-free control black) falls under it.
const MIN_RANGE: f64 = 0.05;
/// Half the probe patch's side: a 5 × 5 patch, about 0.07 units of floor — inside every umbra (whose
/// hard core is at least 0.3 units in radius) with room to spare.
const PATCH_HALF: i64 = 2;

/// What the worker's probe recorded.
#[derive(Debug)]
struct Probe {
    path: String,
    legs: String,
    mesh_leg: bool,
    epoch: u64,
    header_word7: u32,
    /// Every directional row's direction, in table order.
    dirs: Vec<[f32; 3]>,
}

/// A decoded 32-bpp bottom-up BMP.
struct Frame {
    width: i64,
    height: i64,
    bytes: Vec<u8>,
}

impl Frame {
    /// Parses the runner's BMP (`host_dump.rs`'s `write_bmp`: a 54-byte header, 32 bpp, BGRA,
    /// positive height = bottom-up rows).
    fn parse(bytes: Vec<u8>, file: &Path) -> Self {
        let le32 = |at: usize| i32::from_le_bytes(bytes[at..at + 4].try_into().expect("invariant: 4 bytes"));
        assert!(bytes.len() > 54 && &bytes[..2] == b"BM", "{}: not a BMP", file.display());
        let offset = le32(10);
        let (width, height) = (i64::from(le32(18)), i64::from(le32(22)));
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert!(
            offset == 54 && bpp == 32 && width > 0 && height > 0,
            "{}: unexpected BMP layout (offset {offset}, {bpp} bpp, {width}x{height})",
            file.display()
        );
        assert_eq!(bytes.len() as i64, 54 + width * height * 4, "{}: truncated BMP", file.display());
        Self { width, height, bytes }
    }

    /// Display-linear Rec.709 luminance of the top-down pixel `(x, y)`.
    fn luminance(&self, x: i64, y: i64) -> f64 {
        let row = self.height - 1 - y;
        let at = (54 + (row * self.width + x) * 4) as usize;
        let lin = |c: u8| (f64::from(c) / 255.0).powf(2.2);
        let (b, g, r) = (self.bytes[at], self.bytes[at + 1], self.bytes[at + 2]);
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    /// The mean luminance of the 5 × 5 patch centred on `(x, y)`.
    fn patch(&self, (x, y): (i64, i64)) -> f64 {
        let mut sum = 0.0;
        for dy in -PATCH_HALF..=PATCH_HALF {
            for dx in -PATCH_HALF..=PATCH_HALF {
                sum += self.luminance(x + dx, y + dy);
            }
        }
        let side = (2 * PATCH_HALF + 1) as f64;
        sum / (side * side)
    }
}

/// One completed run.
struct Run {
    scene: GateScene,
    frame: Frame,
    probe: Probe,
}

type V3 = [f64; 3];

fn v3(a: [f32; 3]) -> V3 {
    [f64::from(a[0]), f64::from(a[1]), f64::from(a[2])]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(a: V3) -> V3 {
    let len = dot(a, a).sqrt();
    [a[0] / len, a[1] / len, a[2] / len]
}

/// The floor point whose ray toward `dir` (TO the light) passes through the sphere's centre — the
/// centre of the sphere's umbra under that sun.
fn umbra_centre(dir: [f32; 3]) -> V3 {
    let (c, d) = (v3(SPHERE_CENTER), v3(dir));
    let t = c[1] / d[1];
    [c[0] - t * d[0], 0.0, c[2] - t * d[2]]
}

/// The pixel `generate_ray` (`ray_gen.hlsli`) sends through world point `p`: the camera basis of
/// `Affine3A::look_at_rh(EYE, TARGET, UP_HINT)`, `ndc = ((px + 0.5) / w) * 2 − 1` with `y` flipped.
fn project(p: V3, frame: &Frame) -> (i64, i64) {
    let eye = v3(EYE);
    let back = normalize(sub(eye, v3(TARGET)));
    let right = normalize(cross(v3(UP_HINT), back));
    let up = cross(back, right);
    let rel = sub(p, eye);
    let depth = -dot(rel, back);
    assert!(depth > 0.0, "a probe point is behind the camera");
    let tan_half = (f64::from(FOV_Y) * 0.5).tan();
    let aspect = frame.width as f64 / frame.height as f64;
    let ndc_x = dot(rel, right) / (depth * tan_half * aspect);
    let ndc_y = dot(rel, up) / (depth * tan_half);
    let px = (ndc_x + 1.0) * 0.5 * frame.width as f64 - 0.5;
    let py = (1.0 - ndc_y) * 0.5 * frame.height as f64 - 0.5;
    let (x, y) = (px.round() as i64, py.round() as i64);
    assert!(
        x - PATCH_HALF >= 0 && y - PATCH_HALF >= 0 && x + PATCH_HALF < frame.width && y + PATCH_HALF < frame.height,
        "the probe patch at ({x}, {y}) leaves the {}x{} frame",
        frame.width,
        frame.height
    );
    (x, y)
}

/// Parses a TOML integer the probe writes: decimal, or `0x`-prefixed hex.
fn parse_int(v: &str, key: &str, file: &Path) -> u64 {
    let v = v.trim();
    let parsed = match v.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => v.parse::<u64>(),
    };
    parsed.unwrap_or_else(|e| panic!("{}: `{key} = {v}` is not an integer ({e})", file.display()))
}

/// Reads the probe the worker wrote. Every key is required: a record missing one is an
/// emitter/reader drift, and must not decode as a zero.
fn read_probe(file: &Path) -> Probe {
    let text = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("the worker wrote no probe at {} ({e})", file.display()));
    let field = |key: &str| -> String {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_else(|| panic!("{}: the probe record has no `{key}`", file.display()))
    };
    let raw = field("directional_dirs");
    let inner = raw
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .unwrap_or_else(|| panic!("{}: `directional_dirs` is not an array", file.display()));
    let lanes: Vec<f32> = inner
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let bits = u32::try_from(parse_int(s, "directional_dirs", file)).expect("invariant: f32 bits fit a u32");
            f32::from_bits(bits)
        })
        .collect();
    let (triples, rest) = lanes.as_chunks::<3>();
    assert!(rest.is_empty(), "{}: `directional_dirs` holds {} lanes", file.display(), lanes.len());
    Probe {
        path: field("path").trim_matches('"').to_string(),
        legs: field("legs").trim_matches('"').to_string(),
        mesh_leg: match field("mesh_leg").as_str() {
            "true" => true,
            "false" => false,
            other => panic!("{}: mesh_leg = {other} is not a bool", file.display()),
        },
        epoch: parse_int(&field("epoch"), "epoch", file),
        header_word7: u32::try_from(parse_int(&field("header_word7"), "header_word7", file))
            .expect("invariant: the probe writes a u32 word"),
        dirs: triples.to_vec(),
    }
}

/// Whether an environment variable must not reach a child. Windows variable names are
/// case-insensitive, so the comparison is on the upper-cased name.
fn scrubbed(name: &str) -> bool {
    name.to_ascii_uppercase().starts_with("BOYKO_")
}

/// The gate's shared output directory.
fn out_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("boyko_sdf_marcher_sun");
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    dir
}

/// Runs one worker on `scene` and returns its frame and probe, or panics with the child's output
/// files named. Stale artifacts are deleted first.
fn run(scene: GateScene, out_dir: &Path) -> Run {
    let label = scene.name();
    let dump = out_dir.join(format!("{label}.bmp"));
    let probe = out_dir.join(format!("{label}.toml"));
    let stdout_path = out_dir.join(format!("{label}.stdout.txt"));
    let stderr_path = out_dir.join(format!("{label}.stderr.txt"));
    for p in [&dump, &probe] {
        let _ = std::fs::remove_file(p);
        assert!(!p.exists(), "a stale {} could not be deleted", p.display());
    }
    let stdout = File::create(&stdout_path).expect("invariant: the gate's temp dir accepts files");
    let stderr = File::create(&stderr_path).expect("invariant: the gate's temp dir accepts files");

    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    // `--nocapture`: libtest's capture buffer is discarded by `process::exit`, which would lose the
    // worker's own report of what it booted.
    cmd.args([WORKER, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    for (key, _) in std::env::vars_os() {
        if scrubbed(&key.to_string_lossy()) {
            cmd.env_remove(&key);
        }
    }
    cmd.env(DRIVER_MARKER, "1")
        .env(ENV_SCENE, label)
        .env(ENV_PROBE, &probe)
        .env(ENV_HOST_DUMP, &dump)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        .env("BOYKO_SHADOW_DENOISE", "none")
        .env("BOYKO_WINDOW_FRAMES", HANG_CAP_FRAMES);

    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("{label}: the worker did not spawn ({e})"));
    let deadline = Instant::now() + WORKER_DEADLINE;
    let status: ExitStatus = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "{label}: HUNG after {} s and killed (output: {}, {})",
                    WORKER_DEADLINE.as_secs(),
                    stdout_path.display(),
                    stderr_path.display()
                );
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{label}: waiting on the worker failed ({e})");
            }
        }
    };
    let verdict = match status.code() {
        Some(EXIT_RENDERED) => None,
        Some(0) => Some("reached no verdict (exit 0: skipped or filtered out)".to_owned()),
        Some(EXIT_TEST_FAILED) => Some("a worker assertion failed (exit 101)".to_owned()),
        code => Some(format!("the worker process died (exit {code:?})")),
    };
    if let Some(why) = verdict {
        panic!(
            "{label}: {why}. A missing device, a failed boot or a run that never reached the dump \
             frame is RED here, never a skip. Output: {}, {}",
            stdout_path.display(),
            stderr_path.display()
        );
    }
    let bytes = std::fs::read(&dump).unwrap_or_else(|e| panic!("{label}: no frame at {} ({e})", dump.display()));
    Run { scene, frame: Frame::parse(bytes, &dump), probe: read_probe(&probe) }
}

/// The instrument clause every run shares: the probe describes the configuration asked for, CSM and
/// the punctual atlas are unarmed (the marcher is the only shadow source), and the table's
/// directional rows are the ones the scene spawned, in the order the verdict assumes.
fn instrument_findings(r: &Run) -> Vec<String> {
    let mut out = Vec::new();
    let label = r.scene.name();
    let p = &r.probe;
    if p.path != "Deferred" || p.legs != "Both" || !p.mesh_leg {
        out.push(format!(
            "{label}: DEGRADED — requested Deferred × Both, the probe reports {} × {} (mesh_leg {})",
            p.path, p.legs, p.mesh_leg
        ));
    }
    if p.epoch < PROBE_AT_FRAME {
        out.push(format!("{label}: the probe was written at frame {}, before {PROBE_AT_FRAME}", p.epoch));
    }
    if p.header_word7 & (CSM_BIT | PUNCTUAL_BIT) != 0 {
        out.push(format!(
            "{label}: header word 7 is {:#010x} — a cascade or atlas shadow is armed, so the marcher is \
             not the only shadow source this gate assumes",
            p.header_word7
        ));
    }
    if r.frame.width != i64::from(WIN) || r.frame.height != i64::from(WIN) {
        out.push(format!("{label}: the frame is {}x{}, want {WIN}x{WIN}", r.frame.width, r.frame.height));
    }
    let want: &[[f32; 3]] = if r.scene.has_suns() { &[SUN_ROTATED_A, SUN_SECOND_D] } else { &[] };
    let matches = p.dirs.len() == want.len()
        && p.dirs.iter().zip(want).all(|(got, w)| dot(normalize(v3(*got)), normalize(v3(*w))) > 0.99999);
    if !matches {
        out.push(format!(
            "{label}: the table's directional rows are {:?}, want (normalised) {:?} in that order — the \
             fixture's premise (row 0 = the rotated main sun, the resolve's primary) does not hold, so no \
             verdict below can be attributed",
            p.dirs, want
        ));
    }
    out
}

/// A probe's normalised visibility — `None`, with a finding, when its controls leave no range to
/// normalise by.
fn visibility(name: &str, at: V3, subject: &Run, lit: &Run, dark: &Run, findings: &mut Vec<String>) -> Option<f64> {
    let px = project(at, &subject.frame);
    let (y, y_lit, y_dark) = (subject.frame.patch(px), lit.frame.patch(px), dark.frame.patch(px));
    let range = y_lit - y_dark;
    eprintln!(
        "MARCHER-SUN {name} at pixel {px:?}: {}={y:.5} {}={y_lit:.5} {}={y_dark:.5}",
        subject.scene.name(),
        lit.scene.name(),
        dark.scene.name()
    );
    if range < MIN_RANGE {
        findings.push(format!(
            "{name}: the controls leave no range at pixel {px:?} — {} reads {y_lit:.5} and {} reads \
             {y_dark:.5} (want a difference >= {MIN_RANGE}). The floor is not lit even with no occluder, \
             which is what a sunless frame that keeps SHADOWS renders (M4), or the light does not reach \
             the probe",
            lit.scene.name(),
            dark.scene.name()
        ));
        return None;
    }
    Some((y - y_dark) / range)
}

/// A spot's class under the thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Dark,
    Lit,
    Ambiguous,
}

fn classify(v: f64) -> Class {
    if v < DARK_MAX {
        Class::Dark
    } else if v > LIT_MIN {
        Class::Lit
    } else {
        Class::Ambiguous
    }
}

/// `v` for a summary line (`-` when unmeasurable).
fn show(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |v| format!("{v:.3}"))
}

/// Fails with every finding, or passes.
fn verdict(test: &str, runs: usize, findings: &[String]) {
    assert!(
        findings.is_empty(),
        "{test}: {} finding(s) over {runs} runs (artifacts: {}):\n{}",
        findings.len(),
        out_dir().display(),
        findings.join("\n")
    );
}

/// **T10.** See the module doc.
#[test]
#[ignore = "gpu-windowed: needs a Vulkan device + window; re-executes its worker per scene; --test-threads=1"]
fn marcher_shadow_follows_the_table_sun_after_runtime_rotation() {
    let dir = out_dir();
    let subject = run(GateScene::Sun, &dir);
    let lit = run(GateScene::SunNoOccluder, &dir);
    let dark = run(GateScene::Dark, &dir);
    let mut findings: Vec<String> = Vec::new();
    for r in [&subject, &lit, &dark] {
        findings.extend(instrument_findings(r));
    }

    let v_r = visibility("R", v3(REFERENCE_R), &subject, &lit, &dark, &mut findings);
    if let Some(v) = v_r
        && classify(v) != Class::Lit
    {
        findings.push(format!(
            "R (open floor) is not lit: v = {v:.3} (want > {LIT_MIN}) — the floor is darkened away from \
             every candidate shadow"
        ));
    }
    let candidates = [
        ("A", SUN_ROTATED_A, "the table's primary at the dump frame — the correct shadow"),
        ("B", SUN_SPAWN_B, "the main sun's SPAWN direction: the primary was latched, not read per frame (M7)"),
        ("C", BOOT_CONSTANT_C, "gpu_scene's boot-constant sun: the marcher ignores the scene's sun (R2)"),
        ("D", SUN_SECOND_D, "the SECOND directional: the marcher took a row that is not the table's primary"),
    ];
    let mut dark_spots: Vec<&str> = Vec::new();
    let mut measured = 0usize;
    let mut summary: Vec<String> = Vec::new();
    for (name, dir, meaning) in candidates {
        let v = visibility(name, umbra_centre(dir), &subject, &lit, &dark, &mut findings);
        summary.push(format!("{name}={}", show(v)));
        let Some(v) = v else { continue };
        measured += 1;
        match classify(v) {
            Class::Dark => dark_spots.push(name),
            Class::Lit => {}
            Class::Ambiguous => findings.push(format!(
                "{name} is neither dark nor lit: v = {v:.3} (dark < {DARK_MAX}, lit > {LIT_MIN}) — {name} \
                 is the umbra under {meaning}"
            )),
        }
    }
    // A verdict needs all four candidates measured; a missing range is already a finding.
    if measured == candidates.len() {
        match dark_spots.as_slice() {
            ["A"] => {}
            [] => findings.push(
                "no candidate spot is dark: no SDF shadow fell on any of A, B, C or D — shadows off, no \
                 occluder, or the marcher shadowed toward a direction none of them names (for instance \
                 one latched before the sun's pose reached the table), so this frame proves nothing \
                 about which sun the marcher took"
                    .to_owned(),
            ),
            [one] => {
                let meaning = candidates.iter().find(|c| c.0 == *one).map_or("", |c| c.2);
                findings.push(format!("the dark spot is {one}, not A: the marcher shadowed toward {meaning}"));
            }
            many => findings.push(format!(
                "{} candidate spots are dark ({}): the marcher writes one shadow per frame, so this is \
                 not a sun-direction verdict — read the frames",
                many.len(),
                many.join(", ")
            )),
        }
    }
    eprintln!("MARCHER-SUN T10: R={} {} dark={dark_spots:?}", show(v_r), summary.join(" "));
    verdict("T10", 3, &findings);
}

/// **T11.** See the module doc.
#[test]
#[ignore = "gpu-windowed: needs a Vulkan device + window; re-executes its worker per scene; --test-threads=1"]
fn sunless_scene_has_no_phantom_marcher_shadow() {
    let dir = out_dir();
    let subject = run(GateScene::Point, &dir);
    let lit = run(GateScene::PointNoOccluder, &dir);
    let dark = run(GateScene::Dark, &dir);
    let mut findings: Vec<String> = Vec::new();
    for r in [&subject, &lit, &dark] {
        findings.extend(instrument_findings(r));
    }

    let c = umbra_centre(BOOT_CONSTANT_C);
    let axis = v3(POINT_POS);
    let c_mirror = [2.0 * axis[0] - c[0], 0.0, 2.0 * axis[2] - c[2]];
    let v_c = visibility("C", c, &subject, &lit, &dark, &mut findings);
    let v_m = visibility("C'", c_mirror, &subject, &lit, &dark, &mut findings);
    if let (Some(v_c), Some(v_m)) = (v_c, v_m) {
        if classify(v_m) != Class::Lit {
            findings.push(format!(
                "C' (C mirrored through the point light's axis) is not lit: v = {v_m:.3} (want > \
                 {LIT_MIN}) — the point light does not light the floor, so C's comparison proves nothing"
            ));
        } else if classify(v_c) != Class::Lit || v_c < SYMMETRY_MIN * v_m {
            findings.push(format!(
                "C is darker than its mirror: v(C) = {v_c:.3}, v(C') = {v_m:.3} (want both > {LIT_MIN} \
                 and v(C) >= {SYMMETRY_MIN} * v(C')) — a scene with no directional light carries a \
                 PHANTOM sun shadow toward the boot constant, and it masks the point light"
            ));
        }
    }
    eprintln!("MARCHER-SUN T11: C={} C'={}", show(v_c), show(v_m));
    verdict("T11", 3, &findings);
}
