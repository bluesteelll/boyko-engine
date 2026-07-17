//! Rung A0: the TAA jitter-REACH falsification harness (does the sub-pixel jitter actually
//! perturb shading beyond the raster mesh vertex push?), plus the rung-E2 in-motion scaffolding
//! (`BOYKO_TAA_MOTION`) kept dormant behind its own default.
//!
//! # Why A0 comes before an in-motion owner eval
//!
//! Three verified code facts refute the premise that shipped TAA improves shadow/SDF quality:
//!
//! 1. The jitter reaches ONLY the raster mesh vertex push: `runner.rs`'s frame loop selects
//!    [`boyko_render::gbuffer_push_from_view_jittered`] over the non-jittered fn precisely when
//!    TAA is armed, and that fn's own doc states the offset perturbs "ONLY ... this 88-byte
//!    VERTEX push's leading `view_proj`" — `cam_eye` and the trailing selectors (the SEPARATE
//!    b5 camera UBO) are untouched (`view.rs`).
//! 2. The deferred PBR shadow lookup reconstructs the shaded world position from the UNJITTERED
//!    b5 basis: `deferred_pbr.hlsl`'s `generate_ray(px, py, w, h, camera_mode, cam_eye.xyz,
//!    cam_forward, cam_right, cam_up.xyz, ro, rd)` feeds `float3 P = ro + rd * view_t;`, and `P`
//!    is what `vis = min(vis, csm_visibility(P, n, csm_view_z, NoL));` samples the cascade map
//!    against. `cam_eye`/`cam_forward`/`cam_right`/`cam_up` are the b5 UBO lane the jitter never
//!    touches (fact 1).
//! 3. [`boyko_render::taa_jitter`]'s own module doc states the C1 cut in its own words: v1 TAA
//!    "jitters ONLY the raster mesh vertex push; SDF-marched pixels stay temporally stable but
//!    un-supersampled" — the marcher ray-gen shares the SAME unjittered b5 basis as fact 2, so
//!    an SDF-marched pixel's shading input is bit-identical across Halton phases BY
//!    CONSTRUCTION, not merely "stable in practice".
//!
//! Consequence (the falsifiable hypothesis this harness dumps evidence for): rendering the
//! IDENTICAL static scene twice, TAA armed, with the jitter phase PINNED to two different
//! `HALTON_8` indices, should show
//!   * an SDF-marched region: **bit-identical** pixels between the two dumps (facts 2+3 predict
//!     the shadow AND the marched surface are both phase-invariant there),
//!   * a fronto-parallel MESH surface (near-normal incidence): **≈ zero** delta (only the tiny
//!     residual from the raster vertex-push jitter's sub-pixel silhouette wobble, which is
//!     ~nothing on a surface whose screen-space footprint barely moves per sub-pixel offset),
//!   * a grazing-incidence MESH surface (the floor): a **small but nonzero** delta (the
//!     silhouette / depth-edge wobble is largest at grazing angles).
//!
//! If the SDF region differs between the two dumps, facts 1-3 (or this harness's wiring) are
//! wrong and the whole in-motion-TAA campaign premise changes — this gate is meant to be
//! trustworthy enough to carry that conclusion, not merely suggestive.
//!
//! # `BOYKO_TAA_PHASE=<n>` — pinning the jitter phase (the A0 mechanism)
//!
//! [`JitterState`] is a `#[derive(Resource)]` singleton [`boyko_render::aa_plugin::AaPlugin`]
//! inserts; the runner's `frame_loop` mutates it via [`boyko_render::advance_jitter`] at
//! `runner.rs:1051` (`advance_jitter(jitter, taa_armed_now)`) — **raw host Rust code, not an ECS
//! system**, called AFTER `app.update_with_delta` has already run this frame's ECS `Update`
//! schedule. So there is no schedule slot that runs literally "after" it; instead
//! [`pin_jitter_phase`] runs INSIDE `Update` (before the host call, same iteration) and
//! pre-compensates for the runner's unconditional `phase = (phase + 1) % 8` bump (while armed):
//! it writes `phase = (requested + 7) % 8`, so the value `advance_jitter` leaves behind — and
//! [`boyko_render::ndc_jitter`] samples for the GPU upload — is exactly the REQUESTED
//! `HALTON_8` index, every frame. `BOYKO_TAA_PHASE` is meaningful only paired with
//! `BOYKO_AA=taa` (`JitterState.armed`); when disarmed [`boyko_render::ndc_jitter`]'s structural
//! zero-skip makes the phase value inert regardless (the OFF byte-identity discipline).
//!
//! [`ForcedJitterPhase`] exists ONLY when `BOYKO_TAA_PHASE` is set, and [`pin_jitter_phase`] is
//! registered ONLY then too — structurally inert (no resource, no system, zero per-frame cost)
//! when the env var is absent, so a plain `BOYKO_AA=taa` run (no phase pin) is unaffected.
//!
//! # Scene: the three pixel classes A0 needs
//!
//! * **SDF body** — one marched sphere ([`SdfPrimitive`] + [`SdfEdit::sphere`], mirroring
//!   `sdf_room_smoke.rs`), sitting among the mesh cubes.
//! * **Fronto-parallel mesh** — a vertical WALL (a [`meshes.plane`](MeshAssetsExt::plane) quad
//!   rotated 90 about X so its face stands in the world XY plane, near-normal incidence
//!   against the camera's view rays) standing behind the casters at `z = -4`.
//! * **Grazing mesh** — the horizontal floor (the standard room receiver plane), whose surface
//!   runs nearly parallel to the camera's view rays near the horizon.
//!
//! CSM is armed (`CsmConfig` + `LightingConfig::csm_shadows`) because the shadow lookup
//! (`csm_visibility`, fact 2) is the specific consumer this harness falsifies against.
//!
//! # Deferred-only (explicit, not implicit)
//!
//! TAA is implemented for [`boyko_render::RenderPath::Deferred`] only — `Forward`/`ForwardPlus`
//! force it off via `RenderPathDegrade::ForwardTaaNotYetImplemented`, and `VisibilityBuffer` via
//! `RenderPathDegrade::VbTaaNotYetImplemented` (`vb.rs`'s `record_vb` asserts TAA is already
//! forced off by the time it would matter — NOT a black-VB regression, a deliberate scope cut).
//! This harness never inserts `RenderPathConfig`, so it resolves to the engine default
//! (`RenderPath::Deferred`, `GeometryLegs::Both`) — the ONLY path this gate can run on, and the
//! reason it is never selected explicitly here.
//!
//! `BOYKO_AA` selects ONE of the mutually exclusive [`AaMode`] alternatives (FXAA / SMAA / TAA
//! are alternatives, not a chain — `GBufferTargets::create`'s exclusivity `debug_assert!`);
//! unset resolves to [`AaConfig::default`] (`AaMode::Off`), mirroring the convention
//! `csm_fit_eval.rs` established for `BOYKO_CSM_FIT`.
//!
//! # `BOYKO_TAA_MOTION` (rung E2, dormant here)
//!
//! Retained for a LATER in-motion ghosting/disocclusion eval, once a keystone fix (a host-side
//! camera-basis shear) lands — an in-motion eval today would be rejected on its own merits per
//! facts 1-3 above (nothing to see on SDF pixels; per-object mesh motion vectors stay
//! `hwrt`-gated, so a moving mesh ghosts by construction). This rung's own default and primary
//! path is `static` (no motion) — the clean A0 control. The four modes, unchanged from the
//! original design:
//!
//! * `static` (default) — no motion.
//! * `orbit` — the camera yaws around the room centre at [`ORBIT_YAW_RATE_PER_FRAME`] rad/frame
//!   (`0.006`). At the default `900`-px square window (`fov_y = pi/3`, `aspect = 1`), the
//!   pinhole small-angle factor is `k = (900/2) / tan(pi/6) ~= 779.4` px/rad, so this rate is a
//!   ~`4.7` px/frame drift (a pure-pan upper-bound estimate; the room casters sit near the
//!   orbit's pivot, so their true parallax is a fraction of this). At the capture frame
//!   ([`CAPTURE_FRAME`] = 30) the total sweep is `0.18` rad (~10.3 degrees) — mid-stroke, no
//!   wraparound.
//! * `strafe` — the camera translates along `+X` at [`LATERAL_STEP_PER_FRAME`] world units/frame
//!   (`0.04`) with a FIXED look direction (the disocclusion test: newly-revealed geometry has no
//!   history). For a feature at the casters' representative depth (`Z ~= 6`, matching
//!   `csm_fit_eval.rs`'s measured caster band `~4.5..8.5`), `dphi ~= dx/Z = 0.00667` rad/frame ->
//!   `~5.2` px/frame. At frame 30 the total lateral displacement is `1.2` world units.
//! * `object` — the camera is static; [`MovingCaster`] translates along `+X` at the SAME
//!   [`LATERAL_STEP_PER_FRAME`], reusing the strafe arithmetic (the apparent shift of a moving
//!   object at fixed depth `Z` from a static camera is the same `dx/Z` formula). This is the
//!   per-object-motion-vector gap probe (fact 3's sibling limitation) — EXPECTED to ghost.
//!
//! All four modes share the identical frame-0 pose (`camera_transform_for_frame`'s `Orbit`/
//! `Strafe` formulas both reduce to the `Static`/`Object` base pose at `frame == 0`), so the
//! converged-static leg is reproducible from any mode by simply not advancing past the settle
//! window.
//!
//! # Frame source: `RenderEpoch`, not a private counter
//!
//! [`boyko_render::RenderEpoch`] is a per-frame `Resource` the runner overwrites with
//! `host.renderer.submission_epoch()` BEFORE `app.update_with_delta` runs each iteration
//! (`runner.rs`, the asset-streaming F6 fence-clock publish) — so during iteration `i`'s
//! `Update` schedule it reads exactly `i` (0-indexed, steady state: no resize / minimize /
//! recreate-skip during a fixed-size settle window). `host_dump.rs`'s `SETTLE_FRAMES = 30`
//! counts PRESENTED frames the identical way (its `Settle` countdown decrements only on
//! `render_gbuffer_frame`'s `Ok(true)`, the same submit-gated signal `RenderEpoch` derives
//! from), so the capture (readback-request) frame is exactly `RenderEpoch == 30` — this is
//! [`CAPTURE_FRAME`]. Using the engine's own submission clock (rather than a private frame
//! counter) means the motion driver and the dump's settle countdown can never drift apart.
//!
//! # SINGLE-TEST BINARY
//!
//! `EnginePlugins` composes `LightingPlugin`, whose light eviction hooks are process-global — do
//! not co-locate a second light-archetyping test here (mirrors `room_smoke.rs`'s identical
//! warning).
//!
//! # Usage
//!
//! ```text
//! # A0 falsification: two static dumps at widely-separated Halton phases, TAA armed, CSM on.
//! BOYKO_AA=taa BOYKO_TAA_PHASE=0 BOYKO_HOST_DUMP=D:\tmp\phase0.bmp \
//!   cargo test -p boyko-app --test taa_jitter_eval -- --ignored --test-threads=1
//! BOYKO_AA=taa BOYKO_TAA_PHASE=4 BOYKO_HOST_DUMP=D:\tmp\phase4.bmp \
//!   cargo test -p boyko-app --test taa_jitter_eval -- --ignored --test-threads=1
//! # then diff the SDF sphere / wall / floor regions between phase0.bmp and phase4.bmp.
//! ```
//!
//! `#[ignore]`: needs a real windowed GPU device; the orchestrator runs it. Run with
//! `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::filter::With;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::{Component, Resource};
use boyko_render::{
    AaConfig, AaMode, CsmPcfKernel, HALTON_8, JitterScope, JitterState, LightingConfig, RenderEpoch,
    TaaConfig,
};

/// The sun direction TO the light — mirrors `examples/room.rs` / `csm_fit_eval.rs`.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The presented-frame index (via [`RenderEpoch`]) at which `host_dump`'s `SETTLE_FRAMES`
/// window elapses and the readback is requested — see the module doc's "Frame source" section.
const CAPTURE_FRAME: u64 = 30;

/// The base (frame-0 / static-mode) camera pose: eye 6 units back on `+Z`, looking at the room
/// origin — mirrors `csm_fit_eval.rs` / `examples/room.rs`.
const BASE_EYE: [f32; 3] = [0.0, 1.7, 6.0];

/// `MotionMode::Orbit`'s fixed orbit radius (world units) — matches [`BASE_EYE`]'s `z` so
/// `frame == 0` reproduces the exact base pose.
const ORBIT_RADIUS: f32 = 6.0;

/// `MotionMode::Orbit`'s camera yaw rate, radians/frame. See the module doc's arithmetic.
const ORBIT_YAW_RATE_PER_FRAME: f32 = 0.006;

/// The per-frame lateral world-unit step shared by `MotionMode::Strafe` (camera `+X`) and
/// `MotionMode::Object` ([`MovingCaster`] `+X`) — one amplitude, one derivation (module doc).
const LATERAL_STEP_PER_FRAME: f32 = 0.04;

/// [`MovingCaster`]'s base position — a fifth cube alongside `csm_fit_eval.rs`'s four.
const MOVING_CASTER_BASE: [f32; 3] = [-1.4, 0.5, 1.6];

/// The fronto-parallel mesh WALL's side length (world units, a square [`meshes.plane`] quad).
const WALL_SIZE: f32 = 6.0;

/// The wall's fixed world-`Z` (behind the casters, which span roughly `z in [-2.5, 1.2]`).
const WALL_Z: f32 = -4.0;

/// 90-degree rotation about the world `+X` axis: turns a [`meshes.plane`] quad (default normal
/// `+Y`, spanning the local XZ plane) into a vertical wall standing in the XY plane with normal
/// `+Z` (facing the camera at `z = 6`) — `Quat::new(sin(45deg), 0, 0, cos(45deg))`.
const WALL_ROTATION: Quat = Quat::new(
    core::f32::consts::FRAC_1_SQRT_2,
    0.0,
    0.0,
    core::f32::consts::FRAC_1_SQRT_2,
);

/// The SDF pixel class: one marched sphere among the cubes (mirrors `sdf_room_smoke.rs`).
const SDF_SPHERE_CENTER: [f32; 3] = [-0.9, 0.7, 0.4];
const SDF_SPHERE_RADIUS: f32 = 0.7;

/// The scripted in-motion pattern `BOYKO_TAA_MOTION` selects (rung E2; dormant default `Static`
/// here — see the module doc). Plain data, not a `Component`: held inside [`EvalMotion`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MotionMode {
    /// No motion — the A0 converged-static control.
    Static,
    /// The camera yaws around the room centre.
    Orbit,
    /// The camera translates laterally with a fixed look direction.
    Strafe,
    /// The camera is static; [`MovingCaster`] translates (the missing-motion-vector probe).
    Object,
}

/// The resolved motion mode for this run — a `World`-singleton `Resource` (Principle 0: no
/// process-global `static`). The frame index itself comes from [`RenderEpoch`] (module doc's
/// "Frame source" section), not a field here, so the motion driver and `host_dump`'s settle
/// countdown observe the identical per-frame quantity.
#[derive(Resource, Clone, Copy, Debug)]
struct EvalMotion {
    mode: MotionMode,
    /// A FIXED camera yaw (radians) applied under [`MotionMode::Static`] only — `BOYKO_EVAL_YAW`.
    ///
    /// This is the shadow-CRAWL probe (`0.0` = the plain static pose, so it is inert by default).
    /// Crawl is magnification aliasing of a binary depth compare: the shadow edge requantizes as
    /// the camera drifts sub-pixel. Measuring it needs TWO STATIC poses a hair apart — NOT motion —
    /// so each frame stays a pure function of the pose and any byte difference is requantization
    /// rather than history, timing, or a moving object. `0.003` rad (~0.17 deg) is the same micro-yaw
    /// `shadow_motion_ab_dump` (`boyko_rhi_vulkan/tests/window_present_gbuffer.rs`) uses; at
    /// fov_y 60 deg it shifts the image ~2.6 px at 900p.
    static_yaw: f32,
}

/// Marks the extra `ShadowCaster` cube [`drive_moving_caster_motion`] animates under
/// `MotionMode::Object`; static (pinned at [`MOVING_CASTER_BASE`]) in every other mode.
#[derive(Component, Clone, Copy)]
struct MovingCaster;

/// Owner-forced `HALTON_8` phase index (`BOYKO_TAA_PHASE`) — a `World`-singleton `Resource` that
/// exists ONLY when the env var is set (structurally inert otherwise: no resource, no system,
/// zero per-frame cost). Read by [`pin_jitter_phase`]; see the module doc's "pinning the jitter
/// phase" section for the pre-compensation arithmetic.
#[derive(Resource, Clone, Copy, Debug)]
struct ForcedJitterPhase(u32);

/// The room scene: a grazing floor receiver, a fronto-parallel wall receiver, four static
/// `ShadowCaster` cubes, one animatable `ShadowCaster` cube ([`MovingCaster`]), one SDF sphere,
/// and the usual sun/sky/camera. Mirrors `csm_fit_eval.rs`'s room with the two additions the A0
/// gate needs: the wall and the SDF sphere.
fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let wall = meshes.plane(dev.get(), WALL_SIZE);
    let cube = meshes.cube(dev.get(), 1.0);

    // GRAZING receiver: the floor, normal +Y, nearly parallel to the camera's view rays near
    // the horizon (csm_fit_eval.rs's room, verbatim).
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));

    // FRONTO-PARALLEL receiver: a vertical wall standing behind the casters, its face rotated
    // to near-normal incidence against the camera's view rays.
    commands.spawn(MeshBundle::new(
        wall,
        Transform {
            translation: Vec3::new(0.0, WALL_SIZE * 0.5, WALL_Z),
            rotation: WALL_ROTATION,
            scale: Vec3::ONE,
        },
    ));

    // The four static casters (csm_fit_eval.rs's room, verbatim).
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // The fifth, animatable caster (rung E2's `MotionMode::Object` probe; static here).
    commands
        .spawn(MeshBundle::new(
            cube,
            Transform::from_translation(Vec3::new(
                MOVING_CASTER_BASE[0],
                MOVING_CASTER_BASE[1],
                MOVING_CASTER_BASE[2],
            )),
        ))
        .insert(ShadowCaster)
        .insert(MovingCaster);

    // SDF receiver: the marched sphere (mirrors sdf_room_smoke.rs).
    commands.spawn(SdfPrimitive(SdfEdit::sphere(SDF_SPHERE_CENTER, SDF_SPHERE_RADIUS, sdf_op::UNION, 0.0)));

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

    commands.spawn(CameraRig {
        // The spawn pose only has to be SOME valid pose: `drive_camera_motion` overwrites it from
        // `EvalMotion` on the first `Update`, before any frame is presented.
        transform: camera_transform_for_frame(MotionMode::Static, 0, 0.0),
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

/// Derives the camera's world [`Transform`] for `frame` under `mode` — pure, no world access
/// (unit-testable in isolation). `Static`/`Object` return the fixed [`BASE_EYE`] pose; `Orbit`/
/// `Strafe` both reduce to it at `frame == 0` (see the module doc's "Frame source" section).
fn camera_transform_for_frame(mode: MotionMode, frame: u64, static_yaw: f32) -> Transform {
    let base_eye = Vec3::new(BASE_EYE[0], BASE_EYE[1], BASE_EYE[2]);
    let (eye, target) = match mode {
        // `static_yaw` orbits the FIXED pose by a constant angle (the crawl probe — see
        // `EvalMotion::static_yaw`). `0.0` reproduces `BASE_EYE` exactly: `sin(0) == 0` and
        // `cos(0) == 1` are exact in IEEE, so the default pose is bit-identical to the
        // pre-knob one rather than merely close.
        MotionMode::Static | MotionMode::Object => (
            Vec3::new(
                ORBIT_RADIUS * static_yaw.sin(),
                BASE_EYE[1],
                ORBIT_RADIUS * static_yaw.cos(),
            ),
            Vec3::ZERO,
        ),
        MotionMode::Orbit => {
            let yaw = ORBIT_YAW_RATE_PER_FRAME * frame as f32;
            (
                Vec3::new(ORBIT_RADIUS * yaw.sin(), BASE_EYE[1], ORBIT_RADIUS * yaw.cos()),
                Vec3::ZERO,
            )
        }
        MotionMode::Strafe => {
            let dx = LATERAL_STEP_PER_FRAME * frame as f32;
            (Vec3::new(base_eye.x + dx, base_eye.y, base_eye.z), Vec3::new(dx, 0.0, 0.0))
        }
    };
    let pose = Affine3A::look_at_rh(eye, target, Vec3::new(0.0, 1.0, 0.0));
    Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE }
}

/// Derives [`MovingCaster`]'s world [`Transform`] for `frame` under `mode` — pure, no world
/// access. Only `Object` moves it (along `+X` from [`MOVING_CASTER_BASE`]); every other mode
/// returns the fixed base position.
fn moving_caster_transform_for_frame(mode: MotionMode, frame: u64) -> Transform {
    let mut translation = Vec3::new(MOVING_CASTER_BASE[0], MOVING_CASTER_BASE[1], MOVING_CASTER_BASE[2]);
    if mode == MotionMode::Object {
        translation.x += LATERAL_STEP_PER_FRAME * frame as f32;
    }
    Transform::from_translation(translation)
}

/// Scripted per-frame camera pose driver (rung E2 scaffolding; a no-op churn under the default
/// `MotionMode::Static` after the first frame, since [`Mut::set_if_neq`] skips the write once
/// the pose stops changing).
///
/// Joins `CameraSet::Control` (the `fly_camera_system`/`orbit_camera_system` ordering slot):
/// `CameraPlugin` orders `Control.before(Resolve)`, so `propagate_transforms` recomposes the
/// camera's `GlobalTransform` the SAME frame this system writes — required because a plain
/// `&mut Transform` query term does NOT stamp `changed_tick` in this engine; only the
/// [`Mut<Transform>`] guard does (see `orbit_camera_system`'s doc for the load-bearing pin).
#[allow(clippy::needless_pass_by_value)]
fn drive_camera_motion(
    epoch: Res<RenderEpoch>,
    motion: Res<EvalMotion>,
    mut cameras: Query<Mut<Transform>, With<Camera>>,
) {
    let pose = camera_transform_for_frame(motion.mode, epoch.0, motion.static_yaw);
    for mut transform in cameras.iter_mut() {
        transform.set_if_neq(pose);
    }
}

/// Scripted per-frame [`MovingCaster`] pose driver — the `Object`-mode sibling of
/// [`drive_camera_motion`], kept as a SEPARATE system (rather than a second `Query` in the same
/// system) so two `Mut<Transform>` fetches never alias within one system's parameter set.
#[allow(clippy::needless_pass_by_value)]
fn drive_moving_caster_motion(
    epoch: Res<RenderEpoch>,
    motion: Res<EvalMotion>,
    mut casters: Query<Mut<Transform>, With<MovingCaster>>,
) {
    let pose = moving_caster_transform_for_frame(motion.mode, epoch.0);
    for mut transform in casters.iter_mut() {
        transform.set_if_neq(pose);
    }
}

/// Pins [`JitterState::phase`] to [`ForcedJitterPhase`]'s requested `HALTON_8` index every
/// frame — registered ONLY when `BOYKO_TAA_PHASE` is set (see this struct's + the module doc's
/// "pinning the jitter phase" section for why the write must PRE-compensate the runner's own
/// `advance_jitter` bump rather than run "after" it).
#[allow(clippy::needless_pass_by_value)]
fn pin_jitter_phase(target: Res<ForcedJitterPhase>, mut jitter: ResMut<JitterState>) {
    let phase_count = HALTON_8.len() as u32;
    let requested = target.0 % phase_count;
    // `advance_jitter` (`runner.rs:1051`) runs AFTER this system, later in the SAME frame
    // iteration, and unconditionally does `phase = (phase + 1) % phase_count` while armed.
    // Writing `requested - 1` here (mod `phase_count`) makes that bump land exactly on
    // `requested` by the time `ndc_jitter` samples it for the GPU upload.
    jitter.phase = (requested + phase_count - 1) % phase_count;
}

/// **The A0 jitter-reach falsification dump** (owner/orchestrator-facing, NOT a golden — no
/// `PINS.toml` entry). See the module doc for the env-var contract and how to read the result.
#[test]
#[ignore = "needs a real windowed GPU device; orchestrator-run TAA jitter-reach falsification (A0) / dormant in-motion scaffolding (E2)"]
fn taa_jitter_eval_screenshot_dump() {
    let win: u32 = std::env::var("BOYKO_WIN").ok().and_then(|s| s.parse().ok()).unwrap_or(900);

    // An UNSET `BOYKO_AA` renders the engine's own default (`AaMode::Off`) rather than a
    // hardcoded mode — mirrors the convention `csm_fit_eval.rs` established for `BOYKO_CSM_FIT`.
    let aa_mode = match std::env::var("BOYKO_AA").ok().as_deref() {
        Some("fxaa") => AaMode::Fxaa,
        Some("smaa") => AaMode::Smaa,
        Some("taa") => AaMode::Taa,
        _ => AaConfig::default().mode,
    };

    // Rung E2 scaffolding; unset (or unrecognized) falls back to the A0 control, `static`.
    let motion_mode = match std::env::var("BOYKO_TAA_MOTION").ok().as_deref() {
        Some("orbit") => MotionMode::Orbit,
        Some("strafe") => MotionMode::Strafe,
        Some("object") => MotionMode::Object,
        _ => MotionMode::Static,
    };

    // The A0 mechanism: `None` leaves `JitterState.phase` to advance normally (the plain
    // `BOYKO_AA=taa` in-motion path); `Some` arms the pin (see `pin_jitter_phase`'s doc).
    let forced_phase: Option<u32> = std::env::var("BOYKO_TAA_PHASE").ok().and_then(|s| s.parse().ok());

    // Rung C1's proof-of-life switch. `basis` opts into the b5 camera-basis shear, which is what
    // makes the jitter reach the marcher/resolve/shadow sample position at all. Unset renders the
    // engine default (`RasterOnly`) rather than a hardcoded value, per the `BOYKO_AA` convention
    // above — so the A0 falsification (jitter reaches no SDF shading) and its refutation (it now
    // does) are the SAME binary at two env settings, not two code paths.
    let jitter_scope = match std::env::var("BOYKO_TAA_SCOPE").ok().as_deref() {
        Some("basis" | "raster_and_basis") => JitterScope::RasterAndBasis,
        Some("raster" | "raster_only") => JitterScope::RasterOnly,
        _ => TaaConfig::default().jitter_scope,
    };

    println!(
        "taa_jitter_eval: aa_mode={aa_mode:?} motion_mode={motion_mode:?} forced_phase={forced_phase:?} \
         jitter_scope={jitter_scope:?} win={win} capture_frame={CAPTURE_FRAME}"
    );

    let mut app = App::new();
    // NATURAL user registration order (matches `sdf_room_smoke.rs`'s documented-correct order
    // for a scene carrying an `SdfPrimitive`): `add_plugins` FIRST, THEN `add_startup_system`.
    app.add_plugins(EnginePlugins::window("boyko_app TAA jitter-reach eval", win, win));
    app.add_startup_system(setup);
    // CSM armed: `csm_visibility` is the specific shadow-lookup consumer the A0 falsification
    // targets (module doc fact 2). Inserted AFTER `add_plugins` so it overwrites `CsmPlugin`'s
    // disabled default (`csm_fit_eval.rs`'s established idiom).
    // `BOYKO_CSM_OFF=1` disarms the cascade shadow map structurally (`cascade_count == 0` IS
    // "disabled" — the capability-is-structural predicate), so a dump pair with and without it
    // ISOLATES what CSM contributes to a pixel. That is a structural mask, not a guess about
    // which pixels a shadow covers — the same discipline `BOYKO_GEOMETRY_LEGS` gives for the
    // SDF/mesh split, and the only kind of mask this harness's findings are allowed to rest on.
    let csm_on = std::env::var("BOYKO_CSM_OFF").is_err();
    // Rung E1's shadow-sharpness knob. Unset renders the engine default (`Tent13`) rather than a
    // hardcoded value — the same convention `BOYKO_AA` / `BOYKO_TAA_SCOPE` follow above, so the
    // no-env dump always answers "what does a scene that sets nothing look like?".
    let pcf_kernel = match std::env::var("BOYKO_CSM_PCF").ok().as_deref() {
        Some("tent13") => CsmPcfKernel::Tent13,
        Some("cross5") => CsmPcfKernel::Cross5,
        Some("bilinear1") => CsmPcfKernel::Bilinear1,
        _ => CsmConfig::default().pcf_kernel,
    };
    println!("taa_jitter_eval: csm_on={csm_on} pcf_kernel={pcf_kernel:?}");
    app.insert_resource(CsmConfig {
        cascade_count: if csm_on { 3 } else { 0 },
        pcf_kernel,
        ..CsmConfig::default()
    });
    app.insert_resource(LightingConfig { csm_shadows: csm_on, ..LightingConfig::default() });
    app.insert_resource(AaConfig { mode: aa_mode });
    // Overwrites `AaPlugin`'s default `TaaConfig` — same AFTER-`add_plugins` idiom as `CsmConfig`.
    app.insert_resource(TaaConfig { jitter_scope, ..TaaConfig::default() });
    // `BOYKO_EVAL_YAW` (radians) — the shadow-crawl probe; see `EvalMotion::static_yaw`. Default
    // `0.0` reproduces `BASE_EYE` bit-exactly, so an unset var leaves every other dump unmoved.
    let static_yaw: f32 =
        std::env::var("BOYKO_EVAL_YAW").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    println!("taa_jitter_eval: static_yaw={static_yaw}");
    app.insert_resource(EvalMotion { mode: motion_mode, static_yaw });

    app.add_systems_cfg(|b| {
        b.add_system(drive_camera_motion).in_set(CameraSet::Control);
        b.add_system(drive_moving_caster_motion).in_set(CameraSet::Control);
    });
    if let Some(phase) = forced_phase {
        app.insert_resource(ForcedJitterPhase(phase));
        app.add_systems_cfg(|b| {
            b.add_system(pin_jitter_phase);
        });
    }

    app.run();
}
