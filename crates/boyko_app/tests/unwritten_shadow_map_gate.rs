//! **No frame may depend on a shadow-map layer that no pass wrote** — the device gate of the
//! mesh-shadow producer fix (shadow gate SG1–SG5; `ResolvedRenderPath::mesh_shadow_producers`).
//!
//! # The defect it pins
//!
//! The CSM cascades and the punctual atlas are MESH-shadow producers: their depth passes raster
//! `ShadowCaster` meshes and nothing else. On a leg set without a mesh leg the host used to skip
//! both passes at its own recording seam while the light-header shadow bits, the cascade UBO and
//! the punctual light-table slots stayed armed — so a mesh-less frame sampled a cascade nothing
//! had ever written. On the development machine that memory happened to read as "lit", which is
//! why every golden but one passed; clearing it to `0.0` turned the SDF sphere black. The fix
//! moves the gate into the per-frame plan (`resolve_csm_cascades` / `resolve_shadow_atlas` publish
//! `DISABLED` fits), where every consumer derives from it.
//!
//! # How the gate sees it
//!
//! `BOYKO_SHADOW_POISON=<depth>` (`boyko_app`'s `shadow_poison` knob) clears every cascade and
//! atlas layer to `<depth>` at boot. A frame that samples only layers a pass wrote this frame
//! renders the same bytes whatever the poison; a frame that samples a never-written layer does not.
//! `BOYKO_SHADOW_POISON_PROBE` adds, at the dump frame, the centre texel of every layer and the
//! host's own view of the stream. Every run below is one worker process, re-executed by the
//! drivers from this same binary; **no hash is stored** — every assertion is an equality between
//! runs of one build, or a property of one run's probe.
//!
//! ## `mesh_less_legs_never_sample_an_unwritten_shadow_map`
//!
//! For each path P ∈ {Deferred, Forward, ForwardPlus, VisibilityBuffer} with legs = Sdf, three
//! runs: **a** = shadows on, poison 0.0; **b** = shadows on, poison 1.0; **c** = shadows off
//! (default `CsmConfig` / `ShadowConfig`, same entities), poison 0.0.
//!
//! * **I1** — `frame(a) == frame(b)`: the frame does not depend on the poison.
//! * **I2** — `frame(a) == frame(c)`: a mesh-less leg set with shadows configured renders the
//!   config-off world.
//! * **I3** — in a and b every centre texel of both maps equals the poison's bits. This is what
//!   proves the poison REACHED the maps: one texel cannot equal both `0.0` and `1.0`, so a dead
//!   poison or a dead readback fails here instead of letting I1 pass vacuously.
//! * **I4** — the probe reports the requested path, legs = Sdf, `mesh_leg == false` (no silent
//!   degrade), zero armed frames for either pass, header word 7 bits 2 and 3 clear, no staged
//!   light row carrying a real atlas slot, and (R1) no staged point/spot row the shader would
//!   sample the atlas for (`sampled_rows == 0`, the shader's own predicate
//!   `light_atlas_slot(kind) != SLOT_NONE`). Under the EMPTY handoff both flagged rows are
//!   un-slotted, so a row whose slot field decoded `0` instead of `SLOT_NONE` counts here.
//! * Exactly 12 runs executed.
//!
//! ## `mesh_legs_write_their_shadow_maps_before_sampling` (the positive controls)
//!
//! For P ∈ {Deferred, VisibilityBuffer} with legs = Both: **d** = on @ 0.0, **e** = on @ 1.0,
//! **f** = off @ 0.0.
//!
//! * **C1** — `frame(d) == frame(e)`: armed passes overwrite the poison before any sample.
//! * **C2** — in d, `csm_active_count >= 1`, `atlas_active_layers >= 7` (the spot's layer plus
//!   the point's six), and every ACTIVE layer's centre texel differs from the `0.0` poison — the
//!   passes really wrote.
//! * **C3** — in d and e, both armed-frame counters are non-zero and header bits 2 and 3 are set.
//!   It also requires `sampled_rows == slotted_rows`, as a CONSISTENCY CHECK between the probe's two
//!   row counts, not an R1 gate: every punctual row of this scene is slotted, so no R1 mutation can
//!   turn it red.
//! * **C4** — `frame(d) != frame(f)`: the shadows are visible, which is what proves I2 CAN fail.
//! * Exactly 6 runs executed.
//!
//! ## `unslotted_punctual_lights_never_sample_the_atlas` (R1)
//!
//! `BOYKO_SHADOW_GATE_SCENE` picks the punctual lights: `flagged` (the spot and the point both carry
//! `CastsPunctualShadow` — the scene of the two tests above), `point_unflagged` (the point without
//! it, so the resolve never assigns it a slot) or `no_point`. With legs = Both and shadows on, for
//! P ∈ {Deferred, Forward, ForwardPlus, VisibilityBuffer}: **u0** = `point_unflagged` @ 0.0 and
//! **u1** = `point_unflagged` @ 1.0. For P ∈ {Deferred, VisibilityBuffer} also **n0** = `no_point`
//! @ 0.0; for P ∈ {Forward, ForwardPlus} also the `flagged` pair **d** @ 0.0 and **e** @ 1.0.
//!
//! Only the spot holds a slot, so the atlas pass renders layer 0 and layers 1..15 keep the poison. A
//! point row whose slot field decoded `0` would take the spot's face record and read a cube face in
//! layers 0..5 — five of them never rendered.
//!
//! * **U1** — `frame(u0) == frame(u1)`: the un-slotted point never reads the atlas.
//! * **U2** — each u-run is the armed frame asked for: `punctual_armed_frames > 0`, header bit 3
//!   set, `atlas_active_layers == 1`, `slotted_rows == 1` and `sampled_rows == 1` (the spot only).
//! * **U3** — in u0 and u1 the atlas centre texels `[1..16)` equal the poison's bits, so the layers a
//!   defective read lands on DO hold the poison; and `atlas[0]` in u0 is not the `0.0` poison, so the
//!   spot's layer was written.
//! * **U5** — `frame(u0) != frame(n0)` (Deferred, VB): the point lights visible pixels, which is what
//!   proves U1 CAN fail.
//! * **UC** — `frame(d) == frame(e)` (Forward, ForwardPlus): C1 on the two paths the positive
//!   controls do not run. If either path depended on the poison for another reason (the cascades,
//!   say), U1 would be red there for a reason that is not R1's; a red UC makes a U1 red on the same
//!   path unattributable.
//! * Exactly 14 runs executed.
//!
//! The un-slotted SPOT is outside this test: a spot row whose field decoded `0` reads layer 0, which
//! IS written, so no poison can show it. The device-free gates cover it (`light_system.rs`'s
//! `every_punctual_row_decodes_exactly_its_assignment`, `mesh_shadow_arming_agreement.rs`); both
//! kinds reach the shaders through the same predicate line.
//!
//! # Mutation receipts (release builds, so the pixel invariants decide, not a `debug_assert!`)
//!
//! Named SG-M* so they cannot be confused with this lane's other mutation sets.
//!
//! * **SG-M0**, the pre-fix tree: I1 and I4 red.
//! * **SG-M1**, drop the SG1 arm in `resolve_csm_cascades`: I3, I4 and I2 red (the cascade pass now
//!   RUNS on a mesh-less boot — it overwrites the poison and draws cascade shadows no SDF leg
//!   owns). A debug build panics first, on `GpuSceneBundles::scene()`'s SG3 assert.
//! * **SG-M2**, the same for `resolve_shadow_atlas`: I3, I4 and I2 red.
//! * **SG-M3**, SG-M1 plus the old recording-seam filter re-added (`csm.filter(|_| mesh_leg)` in
//!   `scene()`): I1 red — the original defect's exact shape.
//! * **SG-M3a**, the atlas-only split of SG-M3 (SG-M2 plus `atlas.filter(|_| mesh_leg)`): the
//!   punctual half's own receipt. I4 is red on header bit 3 regardless of the scene's geometry.
//! * **SG-M4**, revert SG4 (upload the live fit on unarmed frames): this gate is expected to stay
//!   GREEN, because its worker leaves both derived header bits to their sync systems, so each bit
//!   starts OFF and can only trail the host's first arming — no header-ON, host-unarmed frame.
//!   That is a property of THIS worker, not of static scenes: a scene that hand-seeds a derived
//!   bit makes the header LEAD the host on frame 0, and `taa_jitter_eval` does (measured
//!   2026-09-18; the design's "a static scene has no header-lag frame" is refuted). SG4 is pinned
//!   by the `boyko_render` unit tests `an_unarmed_frame_uploads_the_disabled_{cascade,atlas}_bytes`
//!   and by the four golden pins that witness that frame — `taa_armed`, `taa_armed_basis`,
//!   `taa_rcas` and `vb_both_taa`, which reverting SG4 alone moves back to their pre-SG4 hashes.
//!
//! The literal mutation "remove the mesh-leg term" does NOT leave the read unguarded under this
//! design, and so does not by itself make the frame depend on the poison: the term sits upstream of
//! BOTH the pass and the sample, so removing it turns the producer ON (SG-M1/SG-M2 — caught by I3,
//! I4 and I2). Only the shape where the term survives on the recording side alone (SG-M3) is the
//! poison-dependent one, and I1 catches it.
//!
//! R1's receipts, named R1-M* after its design (`docs/render/light-table-defects/R1-DESIGN.md`):
//!
//! * **The pre-R1 tree** (point/spot rows built with slot field `0`): U1 red on every path, U2 red
//!   (`sampled_rows` 2), I4 red (`sampled_rows` 2 under the EMPTY handoff). A U1 that is GREEN
//!   there refutes the premise that the defective read reaches the poison: stop and read the run,
//!   do not adjust the gate.
//! * **R1-M1**, `GpuLight::from_point` back to the raw kind: U1, U2 and I4 red.
//! * **R1-M2**, `GpuLight::from_spot` back to the raw kind: I4 red (U cannot see a spot, above).
//!
//! # What it cannot claim
//!
//! * **I2 depends on an open owner decision (F3).** Header bit 3 has a second meaning in
//!   `deferred_pbr.hlsl` (the sun's analytic shadow on punctual lights). If "punctual lights own
//!   their visibility whenever an atlas is configured" is adopted, Deferred × Sdf with shadows on
//!   and off will LEGITIMATELY differ: narrow I2 then, do not "fix" the render to satisfy it.
//! * The `hwrt` leg (F4): the directional TLAS trace under bit 2 is closed by the same arm, read
//!   from the code, not measured here.
//! * Header-ON, host-unarmed frames (SG4, and the point-light residual F2): this worker produces
//!   none, because it seeds no derived header bit. Such frames DO occur in static scenes that
//!   hand-seed one — `taa_jitter_eval`'s frame 0 — and are covered there by pins, not here.
//! * I3 relies on the driver preserving a never-rendered layer across the frame graph's
//!   discard-legal `UNDEFINED → SHADER_READ_ONLY` transitions (measured on the development RTX
//!   machine; the investigation's poison runs moved pixels through exactly that path). On a driver
//!   that discards, I3 goes red — correctly, since the gate then cannot prove its poison arrived.
//!   U3 relies on the same preservation on ARMED frames, across the atlas pass's access over the
//!   whole array (`SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32)` in `graph_bridge.rs`); I3
//!   never measured that path, so
//!   U3's first run on a machine is what does.
//! * Other GPUs and drivers.
//!
//! # Running it
//!
//! ```text
//! cargo test -p boyko-app --test unwritten_shadow_map_gate -- --ignored --test-threads=1 \
//!     mesh_less_legs_never_sample_an_unwritten_shadow_map mesh_legs_write_their_shadow_maps_before_sampling \
//!     unslotted_punctual_lights_never_sample_the_atlas
//! ```
//!
//! The drivers scrub every `BOYKO_*` variable from each child and set exactly what the gate needs
//! (`BOYKO_DISABLE_VALIDATION=1`, `BOYKO_SHADOW_DENOISE=none`, AA unset). Worker output and the
//! per-run BMP / TOML land under `temp_dir()/boyko_unwritten_shadow_map_gate/`, deleted before each
//! run so a stale file can never stand in for one this run did not write.

#![cfg(windows)]

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use boyko_app::prelude::*;
use boyko_render::{GeometryLegs, RenderPath, RenderPathConfig};

/// Set by a driver on every child. A worker without it prints SKIP and returns (exit `0`, which
/// the driver reads as "reached no verdict", never as a pass).
const DRIVER_MARKER: &str = "BOYKO_SHADOW_GATE_DRIVEN";
/// The worker's render path: `deferred` | `forward` | `forwardplus` | `vb`.
const ENV_PATH: &str = "BOYKO_SHADOW_GATE_PATH";
/// The worker's geometry legs: `sdf` | `both` | `mesh`.
const ENV_LEGS: &str = "BOYKO_SHADOW_GATE_LEGS";
/// `on` inserts `CsmConfig { cascade_count: 3 }` + an enabled `ShadowConfig`; `off` keeps the
/// plugins' disabled defaults with the SAME entities.
const ENV_SHADOWS: &str = "BOYKO_SHADOW_GATE_SHADOWS";
/// The worker's punctual lights: `flagged` | `point_unflagged` | `no_point` (see [`GateScene`]).
const ENV_SCENE: &str = "BOYKO_SHADOW_GATE_SCENE";
/// The runner's frame dump (the frame the invariants compare).
const ENV_HOST_DUMP: &str = "BOYKO_HOST_DUMP";
/// The poison knob.
const ENV_POISON: &str = "BOYKO_SHADOW_POISON";
/// The poison probe.
const ENV_PROBE: &str = "BOYKO_SHADOW_POISON_PROBE";

/// The worker's exit code when it rendered and both artifacts exist.
const EXIT_RENDERED: i32 = 92;
/// libtest's exit code when a test panicked.
const EXIT_TEST_FAILED: i32 = 101;
/// The worker test's name, passed to `--exact`.
const WORKER: &str = "shadow_poison_worker";
/// Window side in pixels.
const WIN: u32 = 256;
/// The runner's own frame cap — a hang cap only, far above the dump's 34 presented frames.
const HANG_CAP_FRAMES: &str = "400";
/// How long a driver waits for one child before killing it and calling it HUNG.
const WORKER_DEADLINE: Duration = Duration::from_secs(300);
/// The driver's `try_wait` poll interval.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Header word 7's CSM sample bit (`boyko_render::CSM_MODE_BIT`).
const CSM_BIT: u32 = 1 << 2;
/// Header word 7's punctual sample bit (`boyko_render::PUNCTUAL_MODE_BIT`).
const PUNCTUAL_BIT: u32 = 1 << 3;

// ===============================================================================================
// The worker
// ===============================================================================================

/// The sun direction TO the light (`room_smoke.rs`'s).
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];
/// The SDF sphere every shadow source is aimed across.
const SPHERE_CENTER: [f32; 3] = [0.0, 0.7, 0.0];
const SPHERE_RADIUS: f32 = 0.7;

/// `SPHERE_CENTER + t · SUN_DIR` — a point on the sun ray through the sphere's centre.
fn on_sun_ray(t: f32) -> Vec3 {
    Vec3::new(
        SPHERE_CENTER[0] + t * SUN_DIR[0],
        SPHERE_CENTER[1] + t * SUN_DIR[1],
        SPHERE_CENTER[2] + t * SUN_DIR[2],
    )
}

/// Which punctual lights the worker's scene spawns (`ENV_SCENE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateScene {
    /// The spot and the point both carry `CastsPunctualShadow`: the SG1–SG5 scene.
    Flagged,
    /// Only the spot carries it, so the point's row is never assigned a slot (R1).
    PointUnflagged,
    /// The spot alone: the control that shows the point lights visible pixels.
    NoPoint,
}

/// [`scene`] with [`GateScene::Flagged`], as a startup system.
fn scene_flagged(commands: Commands, meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    scene(GateScene::Flagged, commands, meshes, dev);
}

/// [`scene`] with [`GateScene::PointUnflagged`], as a startup system.
fn scene_point_unflagged(commands: Commands, meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    scene(GateScene::PointUnflagged, commands, meshes, dev);
}

/// [`scene`] with [`GateScene::NoPoint`], as a startup system.
fn scene_no_point(commands: Commands, meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    scene(GateScene::NoPoint, commands, meshes, dev);
}

/// The gate's scene: a receiver floor, an SDF sphere, two `ShadowCaster` cubes — one on the sun
/// ray (which the point light sits further along, so it occludes both), one between the spot and
/// the sphere — a `CastsPunctualShadow` spot aimed at the sphere, a point in range of it (flagged,
/// un-flagged or absent per `which`), the sun, a sky fill and the camera.
fn scene(
    which: GateScene,
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    dev: NonSendRes<GpuDevice>,
) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 0.6);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));

    commands.spawn(SdfPrimitive(SdfEdit::sphere(SPHERE_CENTER, SPHERE_RADIUS, sdf_op::UNION, 0.0)));

    let spot_pos = Vec3::new(2.4, 2.6, 1.8);
    let sphere = Vec3::new(SPHERE_CENTER[0], SPHERE_CENTER[1], SPHERE_CENTER[2]);
    for at in [on_sun_ray(1.6), (spot_pos + sphere) * 0.5] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(at)))
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

    let spot_pose = Affine3A::look_at_rh(spot_pos, sphere, Vec3::new(0.0, 1.0, 0.0));
    let aim = (sphere - spot_pos).normalize();
    commands
        .spawn(SpotLightObject {
            transform: Transform {
                translation: spot_pos,
                rotation: Quat::from_mat3(spot_pose.matrix3),
                scale: Vec3::ONE,
            },
            global: GlobalTransform::IDENTITY,
            light: SpotLight::new(
                [spot_pos.x, spot_pos.y, spot_pos.z],
                [aim.x, aim.y, aim.z],
                [1.0, 0.85, 0.7],
                220.0,
                8.0,
                15.0,
                30.0,
            ),
        })
        .insert(CastsPunctualShadow);

    let point = on_sun_ray(3.2);
    let point_light = PointLightObject {
        transform: Transform::from_translation(point),
        global: GlobalTransform::IDENTITY,
        light: PointLight::new([point.x, point.y, point.z], [1.0, 0.72, 0.45], 220.0, 8.0),
    };
    match which {
        GateScene::Flagged => {
            commands.spawn(point_light).insert(CastsPunctualShadow);
        }
        GateScene::PointUnflagged => {
            commands.spawn(point_light);
        }
        GateScene::NoPoint => {}
    }

    let eye = Affine3A::look_at_rh(Vec3::new(0.0, 1.7, 6.0), Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: eye.translation,
            rotation: Quat::from_mat3(eye.matrix3),
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

/// Reads a required worker variable, panicking with its name when the driver did not set it.
fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("the driver did not set {name}"))
}

#[test]
#[ignore = "gpu-windowed: worker for unwritten_shadow_map_gate, not a standalone check"]
fn shadow_poison_worker() {
    if std::env::var_os(DRIVER_MARKER).is_none() {
        eprintln!(
            "SKIP {WORKER}: {DRIVER_MARKER} is unset. This worker is spawned by the \
             unwritten_shadow_map_gate drivers, which set its configuration; run those."
        );
        return;
    }
    let path = match required_env(ENV_PATH).as_str() {
        "deferred" => RenderPath::Deferred,
        "forward" => RenderPath::Forward,
        "forwardplus" => RenderPath::ForwardPlus,
        "vb" => RenderPath::VisibilityBuffer,
        other => panic!("{ENV_PATH}={other:?} names no render path"),
    };
    let legs = match required_env(ENV_LEGS).as_str() {
        "sdf" => GeometryLegs::Sdf,
        "both" => GeometryLegs::Both,
        "mesh" => GeometryLegs::Mesh,
        other => panic!("{ENV_LEGS}={other:?} names no leg set"),
    };
    let shadows = match required_env(ENV_SHADOWS).as_str() {
        "on" => true,
        "off" => false,
        other => panic!("{ENV_SHADOWS}={other:?} is neither on nor off"),
    };
    let which = match required_env(ENV_SCENE).as_str() {
        "flagged" => GateScene::Flagged,
        "point_unflagged" => GateScene::PointUnflagged,
        "no_point" => GateScene::NoPoint,
        other => panic!("{ENV_SCENE}={other:?} names no scene"),
    };
    let dump = PathBuf::from(required_env(ENV_HOST_DUMP));
    let probe = PathBuf::from(required_env(ENV_PROBE));
    let _ = required_env(ENV_POISON);
    eprintln!("SHADOW-GATE worker: path={path:?} legs={legs:?} shadows={shadows} scene={which:?}");

    let mut app = App::new();
    // `add_plugins` FIRST, then the startup system: the order a scene carrying an `SdfPrimitive`
    // needs (`sdf_room_smoke.rs`, `taa_jitter_eval.rs`).
    app.add_plugins(EnginePlugins::window(WORKER, WIN, WIN));
    match which {
        GateScene::Flagged => {
            app.add_startup_system(scene_flagged);
        }
        GateScene::PointUnflagged => {
            app.add_startup_system(scene_point_unflagged);
        }
        GateScene::NoPoint => {
            app.add_startup_system(scene_no_point);
        }
    }
    // Configuration after `add_plugins`, so it overwrites the plugins' defaults.
    app.insert_resource(RenderPathConfig { path, legs });
    if shadows {
        app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
        app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    }
    app.run();

    // The run must have reached the dump frame: a boot that failed (no device, a refused
    // feature) returns from `run` without either artifact, and that must be a red, not a skip.
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

/// One run's configuration.
#[derive(Clone, Copy, Debug)]
struct RunSpec {
    /// The worker's `ENV_PATH` value.
    path: &'static str,
    /// The worker's `ENV_LEGS` value.
    legs: &'static str,
    /// Shadows configured on (`true`) or left at the disabled defaults.
    shadows: bool,
    /// The poison depth.
    poison: f32,
    /// The worker's `ENV_SCENE` value.
    scene: &'static str,
}

impl RunSpec {
    /// A file-name-safe label, unique per spec within one gate.
    fn label(&self) -> String {
        format!(
            "{}_{}_{}_{}_{}",
            self.path,
            self.legs,
            if self.shadows { "on" } else { "off" },
            if self.poison == 0.0 { "p0" } else { "p1" },
            self.scene
        )
    }
}

/// What the probe TOML recorded.
#[derive(Debug)]
struct Probe {
    path: String,
    legs: String,
    mesh_leg: bool,
    presented_frames: u32,
    csm_armed_frames: u64,
    punctual_armed_frames: u64,
    header_word7: u32,
    slotted_rows: u32,
    sampled_rows: u32,
    csm_active_count: u32,
    atlas_active_layers: u32,
    poison_bits: u32,
    cascade: Vec<u32>,
    atlas: Vec<u32>,
}

/// One completed run: its frame and its probe.
struct Run {
    spec: RunSpec,
    frame: Vec<u8>,
    probe: Probe,
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

/// Reads the probe file the worker wrote. Every key is required: a record missing one is an
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
    let int = |key: &str| parse_int(&field(key), key, file);
    let word = |key: &str| u32::try_from(int(key)).expect("invariant: the probe writes u32 words");
    let list = |key: &str| -> Vec<u32> {
        let raw = field(key);
        let inner = raw
            .strip_prefix('[')
            .and_then(|r| r.strip_suffix(']'))
            .unwrap_or_else(|| panic!("{}: `{key}` is not an array", file.display()));
        inner
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| u32::try_from(parse_int(s, key, file)).expect("invariant: texel bits fit a u32"))
            .collect()
    };
    let string = |key: &str| field(key).trim_matches('"').to_string();
    Probe {
        path: string("path"),
        legs: string("legs"),
        mesh_leg: match field("mesh_leg").as_str() {
            "true" => true,
            "false" => false,
            other => panic!("{}: mesh_leg = {other} is not a bool", file.display()),
        },
        presented_frames: word("presented_frames"),
        csm_armed_frames: int("csm_armed_frames"),
        punctual_armed_frames: int("punctual_armed_frames"),
        header_word7: word("header_word7"),
        slotted_rows: word("slotted_rows"),
        sampled_rows: word("sampled_rows"),
        csm_active_count: word("csm_active_count"),
        atlas_active_layers: word("atlas_active_layers"),
        poison_bits: word("poison_bits"),
        cascade: list("cascade_center_bits"),
        atlas: list("atlas_center_bits"),
    }
}

/// Whether an environment variable must not reach a child. Windows variable names are
/// case-insensitive, so the comparison is on the upper-cased name.
fn scrubbed(name: &str) -> bool {
    name.to_ascii_uppercase().starts_with("BOYKO_")
}

/// Runs one worker under `spec` and returns its frame and probe, or panics with the child's
/// output files named. Stale artifacts are deleted first.
fn run(spec: RunSpec, out_dir: &Path) -> Run {
    let label = spec.label();
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
        .env(ENV_PATH, spec.path)
        .env(ENV_LEGS, spec.legs)
        .env(ENV_SHADOWS, if spec.shadows { "on" } else { "off" })
        .env(ENV_SCENE, spec.scene)
        .env(ENV_POISON, spec.poison.to_string())
        .env(ENV_PROBE, &probe)
        .env(ENV_HOST_DUMP, &dump)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        .env("BOYKO_SHADOW_DENOISE", "none")
        .env("BOYKO_WINDOW_FRAMES", HANG_CAP_FRAMES);

    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("{label}: the worker did not spawn ({e})"));
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
    let frame = std::fs::read(&dump)
        .unwrap_or_else(|e| panic!("{label}: no frame at {} ({e})", dump.display()));
    assert!(frame.len() > 54, "{label}: the frame dump at {} is empty", dump.display());
    Run { spec, frame, probe: read_probe(&probe) }
}

/// Bytes that differ between two frames of equal length (`usize::MAX` for unequal lengths).
fn differing_bytes(a: &[u8], b: &[u8]) -> usize {
    if a.len() != b.len() {
        return usize::MAX;
    }
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

/// The gate's shared output directory.
fn out_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("boyko_unwritten_shadow_map_gate");
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    dir
}

/// The `Debug` spelling of the resolved path the probe must report for a worker `ENV_PATH` value.
fn resolved_path_name(path: &str) -> &'static str {
    match path {
        "deferred" => "Deferred",
        "forward" => "Forward",
        "forwardplus" => "ForwardPlus",
        "vb" => "VisibilityBuffer",
        other => panic!("no path {other}"),
    }
}

/// The `Debug` spelling of the resolved legs for a worker `ENV_LEGS` value.
fn resolved_legs_name(legs: &str) -> &'static str {
    match legs {
        "sdf" => "Sdf",
        "both" => "Both",
        "mesh" => "Mesh",
        other => panic!("no legs {other}"),
    }
}

/// The instrument clause every run shares: the probe describes the configuration asked for.
/// Returned as findings, so one red run does not hide the others.
fn instrument_findings(r: &Run, want_mesh_leg: bool) -> Vec<String> {
    let mut out = Vec::new();
    let label = r.spec.label();
    let want_path = resolved_path_name(r.spec.path);
    let want_legs = resolved_legs_name(r.spec.legs);
    if r.probe.path != want_path || r.probe.legs != want_legs || r.probe.mesh_leg != want_mesh_leg {
        out.push(format!(
            "{label}: DEGRADED — requested {want_path} × {want_legs} (mesh_leg {want_mesh_leg}), the \
             probe reports {} × {} (mesh_leg {}). This run would adjudicate a different leg set.",
            r.probe.path, r.probe.legs, r.probe.mesh_leg
        ));
    }
    if r.probe.poison_bits != r.spec.poison.to_bits() {
        out.push(format!(
            "{label}: the probe reports poison bits {:#010x}, the driver set {} ({:#010x})",
            r.probe.poison_bits,
            r.spec.poison,
            r.spec.poison.to_bits()
        ));
    }
    if r.probe.cascade.len() != 4 || r.probe.atlas.len() != 16 {
        out.push(format!(
            "{label}: the probe carries {} cascade and {} atlas texels, expected 4 and 16",
            r.probe.cascade.len(),
            r.probe.atlas.len()
        ));
    }
    if r.probe.presented_frames == 0 {
        out.push(format!("{label}: the probe counted no presented frame"));
    }
    out
}

/// **I1–I4** over every mesh-less path. See the module doc.
#[test]
#[ignore = "gpu-windowed: re-executes its worker on a windowed GPU with poisoned shadow maps; --test-threads=1"]
fn mesh_less_legs_never_sample_an_unwritten_shadow_map() {
    let dir = out_dir();
    let mut findings: Vec<String> = Vec::new();
    let mut runs = 0usize;
    for path in ["deferred", "forward", "forwardplus", "vb"] {
        let spec = |shadows: bool, poison: f32| RunSpec { path, legs: "sdf", shadows, poison, scene: "flagged" };
        let a = run(spec(true, 0.0), &dir);
        let b = run(spec(true, 1.0), &dir);
        let c = run(spec(false, 0.0), &dir);
        runs += 3;

        for r in [&a, &b, &c] {
            findings.extend(instrument_findings(r, false));
            // I4: the leg set owns no mesh-shadow producer, so nothing arms, whatever the config.
            let p = &r.probe;
            if p.csm_armed_frames != 0
                || p.punctual_armed_frames != 0
                || p.header_word7 & (CSM_BIT | PUNCTUAL_BIT) != 0
                || p.slotted_rows != 0
                || p.sampled_rows != 0
            {
                findings.push(format!(
                    "{} I4: a mesh-less leg set armed a shadow producer — csm_armed_frames {}, \
                     punctual_armed_frames {}, header word 7 {:#010x} (bits 2/3 must be clear), \
                     slotted light rows {}, rows whose slot field the shader would sample {} \
                     (R1: an un-slotted row must carry SLOT_NONE)",
                    r.spec.label(),
                    p.csm_armed_frames,
                    p.punctual_armed_frames,
                    p.header_word7,
                    p.slotted_rows,
                    p.sampled_rows
                ));
            }
        }
        // I3: the poison reached every layer of both maps, under both poisons.
        for r in [&a, &b] {
            let bits = r.spec.poison.to_bits();
            let off: Vec<String> = r
                .probe
                .cascade
                .iter()
                .enumerate()
                .map(|(i, t)| ("cascade", i, *t))
                .chain(r.probe.atlas.iter().enumerate().map(|(i, t)| ("atlas", i, *t)))
                .filter(|(_, _, t)| *t != bits)
                .map(|(map, i, t)| format!("{map}[{i}]={t:#010x}"))
                .collect();
            if !off.is_empty() {
                findings.push(format!(
                    "{} I3: the poison ({bits:#010x}) did not reach {} layer(s): {}. The gate cannot \
                     claim a frame is independent of a poison that never arrived.",
                    r.spec.label(),
                    off.len(),
                    off.join(", ")
                ));
            }
        }
        // I1: poison-independent.
        let d_ab = differing_bytes(&a.frame, &b.frame);
        if d_ab != 0 {
            findings.push(format!(
                "{path} × Sdf I1: the frame DEPENDS ON THE POISON ({d_ab} byte(s) differ between \
                 poison 0.0 and 1.0) — some frame sampled a shadow layer no pass wrote"
            ));
        }
        // I2: equals the config-off world.
        let d_ac = differing_bytes(&a.frame, &c.frame);
        if d_ac != 0 {
            findings.push(format!(
                "{path} × Sdf I2: shadows-on differs from shadows-off ({d_ac} byte(s)) on a leg set \
                 with no mesh-shadow producer. Read the module doc's F3 note before changing this."
            ));
        }
        eprintln!("SHADOW-GATE {path} × Sdf: a/b differ {d_ab}, a/c differ {d_ac}");
    }
    assert_eq!(runs, 12, "the mesh-less sweep must execute exactly 12 runs");
    assert!(
        findings.is_empty(),
        "{} finding(s) over {runs} mesh-less runs (artifacts: {}):\n{}",
        findings.len(),
        dir.display(),
        findings.join("\n")
    );
}

/// **C1–C4**, the positive controls. See the module doc.
#[test]
#[ignore = "gpu-windowed: re-executes its worker on a windowed GPU with poisoned shadow maps; --test-threads=1"]
fn mesh_legs_write_their_shadow_maps_before_sampling() {
    let dir = out_dir();
    let mut findings: Vec<String> = Vec::new();
    let mut runs = 0usize;
    for path in ["deferred", "vb"] {
        let spec = |shadows: bool, poison: f32| RunSpec { path, legs: "both", shadows, poison, scene: "flagged" };
        let d = run(spec(true, 0.0), &dir);
        let e = run(spec(true, 1.0), &dir);
        let f = run(spec(false, 0.0), &dir);
        runs += 3;

        for r in [&d, &e, &f] {
            findings.extend(instrument_findings(r, true));
        }
        // C3: both passes armed, both header bits set.
        for r in [&d, &e] {
            let p = &r.probe;
            if p.csm_armed_frames == 0
                || p.punctual_armed_frames == 0
                || p.header_word7 & (CSM_BIT | PUNCTUAL_BIT) != (CSM_BIT | PUNCTUAL_BIT)
            {
                findings.push(format!(
                    "{} C3: a mesh leg with casters did not arm both passes — csm_armed_frames {}, \
                     punctual_armed_frames {}, header word 7 {:#010x}",
                    r.spec.label(),
                    p.csm_armed_frames,
                    p.punctual_armed_frames,
                    p.header_word7
                ));
            }
            // A consistency check between the probe's two row counts, not an R1 gate: every
            // punctual row of this scene is slotted, so both counts see the same two rows.
            if p.sampled_rows != p.slotted_rows {
                findings.push(format!(
                    "{} C3: the probe counts {} slotted row(s) but {} row(s) the shader would sample",
                    r.spec.label(),
                    p.slotted_rows,
                    p.sampled_rows
                ));
            }
        }
        // C2: the passes really wrote every ACTIVE layer over the 0.0 poison.
        let p = &d.probe;
        if p.csm_active_count < 1 || p.atlas_active_layers < 7 {
            findings.push(format!(
                "{} C2: csm_active_count {} (want >= 1), atlas_active_layers {} (want >= 7: the \
                 spot's layer plus the point's six)",
                d.spec.label(),
                p.csm_active_count,
                p.atlas_active_layers
            ));
        }
        let cascades = (p.csm_active_count as usize).min(p.cascade.len());
        let layers = (p.atlas_active_layers as usize).min(p.atlas.len());
        let unwritten: Vec<String> = p.cascade[..cascades]
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == 0.0f32.to_bits())
            .map(|(i, _)| format!("cascade[{i}]"))
            .chain(
                p.atlas[..layers]
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| **t == 0.0f32.to_bits())
                    .map(|(i, _)| format!("atlas[{i}]")),
            )
            .collect();
        if !unwritten.is_empty() {
            findings.push(format!(
                "{} C2: active layer(s) still hold the 0.0 poison at their centre: {}",
                d.spec.label(),
                unwritten.join(", ")
            ));
        }
        // C1: armed passes overwrite the poison before any sample.
        let d_de = differing_bytes(&d.frame, &e.frame);
        if d_de != 0 {
            findings.push(format!(
                "{path} × Both C1: the frame DEPENDS ON THE POISON on a mesh leg ({d_de} byte(s))"
            ));
        }
        // C4: the shadows are visible — the proof that I2 can fail.
        let d_df = differing_bytes(&d.frame, &f.frame);
        if d_df == 0 {
            findings.push(format!(
                "{path} × Both C4: shadows-on renders byte-identical to shadows-off — the scene \
                 shows no shadow, so I2's equality would prove nothing"
            ));
        }
        eprintln!("SHADOW-GATE {path} × Both: d/e differ {d_de}, d/f differ {d_df}");
    }
    assert_eq!(runs, 6, "the mesh-leg sweep must execute exactly 6 runs");
    assert!(
        findings.is_empty(),
        "{} finding(s) over {runs} mesh-leg runs (artifacts: {}):\n{}",
        findings.len(),
        dir.display(),
        findings.join("\n")
    );
}

/// **U1–U5 and UC**, R1's device gate: an un-slotted point never samples the atlas. See the module
/// doc.
#[test]
#[ignore = "gpu-windowed: re-executes its worker on a windowed GPU with poisoned shadow maps; --test-threads=1"]
fn unslotted_punctual_lights_never_sample_the_atlas() {
    let dir = out_dir();
    let mut findings: Vec<String> = Vec::new();
    let mut runs = 0usize;
    for path in ["deferred", "forward", "forwardplus", "vb"] {
        let spec = |scene: &'static str, poison: f32| RunSpec { path, legs: "both", shadows: true, poison, scene };
        let u0 = run(spec("point_unflagged", 0.0), &dir);
        let u1 = run(spec("point_unflagged", 1.0), &dir);
        runs += 2;

        for r in [&u0, &u1] {
            findings.extend(instrument_findings(r, true));
            // U2: the armed frame asked for, with the spot as the only slotted and sampled row.
            let p = &r.probe;
            if p.punctual_armed_frames == 0
                || p.header_word7 & PUNCTUAL_BIT == 0
                || p.atlas_active_layers != 1
                || p.slotted_rows != 1
                || p.sampled_rows != 1
            {
                findings.push(format!(
                    "{} U2: want an armed frame with the spot as the only slotted and sampled row — \
                     punctual_armed_frames {}, header word 7 {:#010x} (bit 3 must be set), \
                     atlas_active_layers {} (want 1), slotted rows {} (want 1), rows the shader \
                     would sample {} (want 1; the un-slotted point must carry SLOT_NONE)",
                    r.spec.label(),
                    p.punctual_armed_frames,
                    p.header_word7,
                    p.atlas_active_layers,
                    p.slotted_rows,
                    p.sampled_rows
                ));
            }
            // U3: the layers a defective point read lands on still hold the poison.
            let bits = r.spec.poison.to_bits();
            let off: Vec<String> = p
                .atlas
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, t)| **t != bits)
                .map(|(i, t)| format!("atlas[{i}]={t:#010x}"))
                .collect();
            if !off.is_empty() {
                findings.push(format!(
                    "{} U3: {} never-rendered atlas layer(s) do not hold the poison ({bits:#010x}): \
                     {}. U1 cannot see a read of memory that does not carry the poison.",
                    r.spec.label(),
                    off.len(),
                    off.join(", ")
                ));
            }
        }
        // U3: the spot's own layer was written over the 0.0 poison.
        if u0.probe.atlas.first() == Some(&0.0f32.to_bits()) {
            findings.push(format!(
                "{} U3: atlas[0] still holds the 0.0 poison at its centre — the spot's layer was not \
                 written",
                u0.spec.label()
            ));
        }
        // U1: the un-slotted point does not read the atlas.
        let d_u = differing_bytes(&u0.frame, &u1.frame);
        if d_u != 0 {
            findings.push(format!(
                "{path} × Both U1: with an un-slotted point the frame DEPENDS ON THE POISON ({d_u} \
                 byte(s) differ between poison 0.0 and 1.0) — the point sampled atlas layers no pass \
                 wrote"
            ));
        }
        let controls = if matches!(path, "deferred" | "vb") {
            // U5: the point lights visible pixels, so U1 can fail.
            let n0 = run(spec("no_point", 0.0), &dir);
            runs += 1;
            findings.extend(instrument_findings(&n0, true));
            let d_un = differing_bytes(&u0.frame, &n0.frame);
            if d_un == 0 {
                findings.push(format!(
                    "{path} × Both U5: the un-flagged point renders byte-identical to no point — it \
                     lights no visible pixel, so U1's equality would prove nothing"
                ));
            }
            format!(", u0/n0 differ {d_un}")
        } else {
            // UC: the flagged scene is poison-independent on this path, so a U1 red here is R1's.
            let d = run(spec("flagged", 0.0), &dir);
            let e = run(spec("flagged", 1.0), &dir);
            runs += 2;
            for r in [&d, &e] {
                findings.extend(instrument_findings(r, true));
            }
            let d_de = differing_bytes(&d.frame, &e.frame);
            if d_de != 0 {
                findings.push(format!(
                    "{path} × Both UC: the FLAGGED scene's frame depends on the poison ({d_de} \
                     byte(s)) — this path reads unwritten shadow memory for a reason that is not \
                     R1's, so a U1 red on it is not attributable to R1"
                ));
            }
            format!(", flagged d/e differ {d_de}")
        };
        eprintln!("SHADOW-GATE {path} × Both, un-slotted point: u0/u1 differ {d_u}{controls}");
    }
    assert_eq!(runs, 14, "the un-slotted sweep must execute exactly 14 runs");
    assert!(
        findings.is_empty(),
        "{} finding(s) over {runs} un-slotted runs (artifacts: {}):\n{}",
        findings.len(),
        dir.display(),
        findings.join("\n")
    );
}
