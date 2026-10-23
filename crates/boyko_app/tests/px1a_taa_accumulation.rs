//! Render wave PX1a tests 13, 16 and 18 and the eval-post pin (`plan/H4-PX1a-hdr-lit.md` Tests 13,
//! 16, 18; `px1a/cut.md` C4, C8; ruling 67 OQ3 / OQ4, ruling 72 (1)): **TAA under the post seam — an
//! exposure change rescales the history, a camera cut resets it, and `display` is `post_final` of
//! this frame's accumulated image.**
//!
//! - **13 — the exposure ramp** (C8). A converged static scene (a SkyLight, so every `lit` texel —
//!   the background included — is pre-exposed radiance) under TAA with the PLAIN blend
//!   (`TaaConfig::luma_weight` off: the Karis weight is not homogeneous in the radiance scale, so a
//!   Karis resolve of a scaled scene is not the scaled resolve, and the statement would be red on
//!   correct code). From frame [`RAMP_AT`] + 1, `LightingConfig::exposure` is multiplied by
//!   [`RAMP_STEP`] per frame for [`RAMP_FRAMES`] frames — through `ResolvedPost`, the header-only
//!   light rewrite and `ResolvedTaa::pre_ratio_delta`. A second run keeps the exposure at 1. Per ramp
//!   frame `F` with `pre_F` read from the worker's `ResolvedPost` log (see [`ramp_verdict`]):
//!   1. **the header path:** `Σ lit_ramp(F) / Σ lit_steady(F) = pre_F`;
//!   2. **the history rescale:** over the HISTORY-BEARING pixels (the steady run's history differs
//!      from its current `lit` by more than [`HISTORY_BEARING`], relative — elsewhere the variance
//!      clip snaps any history to the current frame and cannot see a missing rescale),
//!      `Σ taa_hist_ramp(F) / Σ taa_hist_steady(F) = pre_F`;
//!
//!   each within [`RAMP_REL_TOL`], curve-free. Sums, not texels: a binary16 history is an EMA
//!   rounded every frame, and the two runs' roundings decorrelate once the ramp starts — over eight
//!   frames a texel drifts by a few format steps either way (zero-mean), which a per-texel "two
//!   steps" would read as a failure on correct code. Red: `pre_ratio_delta` forced 0 (statement 2);
//!   the header-only path removed (statement 1).
//! - **The eval-post pin** (ruling 72 (1), critic OQ3; C8). On every frame of the ramp run's burst,
//!   `display` equals `post_final_eval` of the dumped `taa_hist` — the image THIS frame's resolve
//!   wrote, `taa_hist[fi]` — pixel by pixel within `post_eval`'s bound (one display code + E0's
//!   bound on the OETF's `pow`; the scene input is a `Load`, so no filter term). The ramp moves
//!   the history by 25 % a frame, so a `post_final` reading `taa_hist[1 - fi]` (last frame's) is
//!   red on most pixels; S2's binary16 fingerprint cannot see that lag on a converged history.
//!
//! - **16 — the camera cut.** The worker switches the active camera at frame [`SWITCH_AT`]. The
//!   runner must turn `ResolvedPost::cut` into `TaaState::mark_reset` on that same frame, so the
//!   cut frame's resolve replaces the history. Read against `TaaState`'s contract ("the reset frame
//!   itself counts as frame 1", `taa_state.rs`), the per-frame log pins it to the frame (see
//!   [`cut_log_verdict`]), and among the (D) `lit` / `taa_hist` frames dumped around the switch,
//!   exactly the cut frame [`SWITCH_AT`] holds a history equal to its `lit` within one format step
//!   — a replaced (fresh) history — while every blended frame does not.
//! - **18 — accumulation, pre-tonemap.** A static scene with high-contrast edges — one of them HDR,
//!   an emissive bar at radiance 4.0 against a dark floor (ruling 67 OQ4: the HDR-space resolve
//!   differs from the display-space one only there) — under TAA, one 16-frame (D) burst (two
//!   periods of the 8-phase Halton jitter) of `lit` (the jittered current frame) and `display`.
//!   Three statements, in this order (see [`PixelBurst`]):
//!   1. **The HDR premise.** At least [`HDR_EDGE_MIN`] bar-edge pixels: `lit` above 1.0 in some
//!      frame of the burst, and the tonemapped HDR blend more than [`HDR_GAP`] above the
//!      display-space blend. An LDR `lit` (RGBA8 until the switch; a store clamped at 1.0) is RED
//!      here, before any statement that would compare two spaces.
//!   2. **Accumulation.** Over the edge pixels (those whose `OETF(ACES(lit))` moves by more than
//!      [`EDGE_STD`] across the burst) the mean frame-to-frame standard deviation of `display` is
//!      at most half that of `OETF(ACES(lit))` computed here from the dumped `lit`. A `post_final`
//!      that reads `lit` instead of `taa_hist` makes the ratio ≈ 1. This ratio does not depend on
//!      the history's converged mean, so it cannot tell the resolve's space (ruling 72 (3)).
//!   3. **The resolve space** (ruling 72 (3)). Over the HDR bar-edge pixels, `display`'s burst mean
//!      lies at least [`RESOLVE_SPACE_MIN`] of the way from the display-space blend (the burst mean
//!      of `OETF(ACES(lit))`) to the HDR-blend-then-tonemap expectation (`OETF(ACES(·))`, through
//!      `post_final_eval`, of the Karis-weighted burst mean of `lit` — the shipped resolve's blend
//!      space). A host model of the shipped recurrence (blend `clamp(1 / confidence, 0.015, 0.1)`,
//!      the Karis weight, 60 settle frames) at the bar edge puts the pre-tonemap resolve at 0.95, a
//!      resolve done after the tonemap at -0.02 (plain blend) or -0.61 (Karis-weighted blend).
//!
//! Windowed-test conventions: the drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use std::io::Write as _;
use std::path::{Path, PathBuf};

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::Handle;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::light::{LightingConfig, Tonemapper};
use boyko_render::post::{PostPolicySet, ResolvedPost};
use boyko_render::{AaConfig, AaMode, GeometryLegs, Material, RenderPath, RenderPathConfig, TaaConfig, TaaState};
use boyko_rhi_vulkan::present::ImageDumpName;
use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::post::{
    OETF_GAMMA_EXP, TONEMAP_ACES, TONEMAP_MODE_MASK, TONEMAP_MODE_SHIFT, eval_params, post_final_eval, ubo_word,
};
use boyko_shaderdsl::prims_probe::vk_pow_bound;

use graph_receipt::{fresh_out, run_worker};
use image_dump::{DumpFile, files_of};

/// The worker's mode (`cut` | `accumulate`).
const ENV_MODE: &str = "BOYKO_PX1A_TAA";
/// Render wave PXt (`pxt/cut.md` D-G2): the curve this binary's display oracle ([`display_of`]) and
/// thresholds ([`HDR_GAP`], [`HDR_EDGE_MIN`]) are calibrated under. Its subject is TAA accumulation,
/// not the look, so the worker, [`runner_post_words`] and [`display_of`] all name it instead of
/// reading the engine default (AgX since PXt).
const CURVE: Tonemapper = Tonemapper::Aces;
/// Where the cut worker writes its per-frame `TaaState` log.
const ENV_LOG: &str = "BOYKO_PX1A_TAA_LOG";
/// The window edge.
const WIN: u32 = 256;
/// The frame (`HostFrameStats::frames`) the cut worker switches the active camera on.
const SWITCH_AT: u64 = 40;
/// Frames before test 18's burst (the history converges well before).
const SETTLE: u32 = 60;
/// Test 18's burst length.
const BURST: u32 = 16;
/// An edge pixel: the tonemapped `lit` moves by more than this (display units) across the burst.
const EDGE_STD: f64 = 0.02;
/// An HDR bar-edge pixel: the tonemapped HDR blend exceeds the display-space blend by more than
/// this (display units). At the 4.0 bar's edge the gap is 0.07..0.20 for every partial coverage of
/// the 8-phase jitter (host model), well above the 8-bit `display` step.
const HDR_GAP: f64 = 0.05;
/// Test 18's HDR premise: the bar's two vertical edges (~53 px tall at 256 px) give ~100 such pixels.
const HDR_EDGE_MIN: usize = 20;
/// Test 18's resolve-space bar, the midpoint between a display-space resolve (≤ 0) and the shipped
/// pre-tonemap one (≈ 0.95).
const RESOLVE_SPACE_MIN: f64 = 0.5;
/// Rec. 709 luma weights — `taa_resolve.comp.hlsl`'s `luma`, the Karis weight's argument.
const REC709: [f64; 3] = [0.2126, 0.7152, 0.0722];
/// Test 13: the last frame at exposure 1 (the history has converged long before).
const RAMP_AT: u64 = 60;
/// Test 13: the ramp's length, in frames (H4 Tests 13: eight frames).
const RAMP_FRAMES: u32 = 8;
/// Test 13: the per-frame exposure factor (H4 Tests 13: × 1.25).
const RAMP_STEP: f32 = 1.25;
/// Test 13: the relative bound on a sum ratio — four binary16 steps (2^-8). The sums' rounding
/// noise is zero-mean and averages over thousands of channel samples; a missing rescale or a
/// missing header path moves the ratio by percents.
const RAMP_REL_TOL: f64 = 1.0 / 256.0;
/// Test 13: a history-bearing pixel's steady history differs from its steady `lit` by more than
/// this, relative (16 binary16 steps): the history carries what the current frame does not.
const HISTORY_BEARING: f64 = 1.0 / 64.0;
/// Test 13's anti-vacuity: the fewest history-bearing pixels a frame may have.
const BEARING_MIN: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Cut,
    Accumulate,
    /// Test 13: the exposure ramps from frame [`RAMP_AT`] + 1.
    Ramp,
    /// Test 13: the same scene at exposure 1 throughout.
    Steady,
}

#[derive(Resource)]
struct ModeRes(Mode);

/// The second camera (the cut's target).
#[derive(Resource, Default)]
struct Cameras {
    a: Option<Entity>,
    b: Option<Entity>,
}

fn camera_rig(eye: Vec3) -> CameraRig {
    let pose = Affine3A::look_at_rh(eye, Vec3::new(0.0, 0.4, 0.0), Vec3::new(0.0, 1.0, 0.0));
    CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: core::f32::consts::FRAC_PI_3, aspect: 1.0, near: 0.1, far: 100.0 },
    }
}

#[allow(clippy::needless_pass_by_value)]
fn taa_setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    mode: Res<ModeRes>,
    mut cams: ResMut<Cameras>,
) {
    let floor_mat = materials.add(Material::new([0.05, 0.05, 0.06, 1.0], 0.0, 0.8, 0.5, [0.0; 3], 0));
    let wall_mat = materials.add(Material::new([0.9, 0.9, 0.9, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0));
    // The HDR edge: an emissive bar at radiance 4.0 (> 1.0 after the switch's pre-exposure of 1).
    let hot_mat = materials.add(Material::new([0.0, 0.0, 0.0, 1.0], 0.0, 0.5, 0.5, [4.0, 4.0, 4.0], 0));
    let row = |h: Handle<Material>| MaterialHandle(u16::try_from(h.index()).expect("invariant: < 65536 rows"));
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    commands.spawn(MeshBundle { material: row(floor_mat), ..MeshBundle::new(floor, Transform::IDENTITY) });
    let wall = Transform { translation: Vec3::new(-0.8, 0.5, 0.0), ..Transform::IDENTITY };
    commands.spawn(MeshBundle { material: row(wall_mat), ..MeshBundle::new(cube, wall) });
    let bar = Transform { translation: Vec3::new(0.9, 0.6, 0.2), scale: Vec3::new(0.25, 1.2, 0.25), ..Transform::IDENTITY };
    commands.spawn(MeshBundle { material: row(hot_mat), ..MeshBundle::new(cube, bar) });
    let dir = [-0.45f32, 0.82, 0.36];
    let sun_pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(dir[0], dir[1], dir[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(sun_pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(dir, [1.0, 0.96, 0.90], 2.8),
    });
    if matches!(mode.0, Mode::Ramp | Mode::Steady) {
        // Test 13: with a SkyLight the background is pre-exposed sky radiance (the no-sky arm's
        // constant is not), so the whole image scales with the exposure.
        commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));
    }
    cams.a = Some(commands.spawn(camera_rig(Vec3::new(0.0, 1.6, 5.0))).id());
    if mode.0 == Mode::Cut {
        let mut b = camera_rig(Vec3::new(2.5, 1.2, 4.0));
        b.camera.is_active = false;
        cams.b = Some(commands.spawn(b).id());
    }
}

/// The cut worker's switch: at [`SWITCH_AT`] camera `b` becomes the active one.
#[allow(clippy::needless_pass_by_value)]
fn switch_camera(stats: Res<HostFrameStats>, cams: Res<Cameras>, mut q: Query<&mut Camera>) {
    if stats.frames != SWITCH_AT {
        return;
    }
    for (id, cam) in q.iter_entities_mut() {
        let is_b = cams.b.is_some_and(|b| b.id() == id);
        cam.is_active = is_b;
    }
}

/// Test 13's ramp: `LightingConfig::exposure = RAMP_STEP^k`, `k` the frames past [`RAMP_AT`]
/// (capped at [`RAMP_FRAMES`]). Ordered before `PostPolicySet`, so frame `F`'s policy reads it.
#[allow(clippy::needless_pass_by_value)]
fn ramp_exposure(stats: Res<HostFrameStats>, mut cfg: ResMut<LightingConfig>) {
    let k = stats.frames.saturating_sub(RAMP_AT).min(u64::from(RAMP_FRAMES));
    cfg.exposure = RAMP_STEP.powi(i32::try_from(k).expect("invariant: k <= RAMP_FRAMES"));
}

/// Test 13's log: one `frame pre_exposure_bits` line per frame — the value `ResolvedPost`
/// carries THIS frame (ordered after `PostPolicySet`).
#[allow(clippy::needless_pass_by_value)]
fn log_pre_exposure(stats: Res<HostFrameStats>, post: Res<ResolvedPost>) {
    let Ok(path) = std::env::var(ENV_LOG) else { return };
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).expect("invariant: the log is writable");
    writeln!(f, "{} {}", stats.frames, post.pre_exposure.to_bits()).expect("invariant: the log accepts a line");
}

/// The cut worker's log: one `frame frames_since_reset` line per frame.
#[allow(clippy::needless_pass_by_value)]
fn log_taa_state(stats: Res<HostFrameStats>, taa: Res<TaaState>) {
    let Ok(path) = std::env::var(ENV_LOG) else { return };
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).expect("invariant: the log is writable");
    writeln!(f, "{} {}", stats.frames, taa.frames_since_reset).expect("invariant: the log accepts a line");
}

/// **THE WORKER** — `BOYKO_PX1A_TAA=cut|accumulate|ramp|steady`.
#[test]
#[ignore = "gpu-windowed: the PX1a TAA worker, spawned by the px1a_taa_accumulation drivers; run with --test-threads=1"]
fn px1a_taa_worker() {
    let Ok(mode) = std::env::var(ENV_MODE) else {
        eprintln!("px1a_taa_worker: {ENV_MODE} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    let mode = match mode.trim() {
        "cut" => Mode::Cut,
        "accumulate" => Mode::Accumulate,
        "ramp" => Mode::Ramp,
        "steady" => Mode::Steady,
        other => panic!("{ENV_MODE}={other} is not cut|accumulate|ramp|steady"),
    };
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine px1a taa", WIN, WIN));
    app.insert_resource(ModeRes(mode));
    app.insert_resource(Cameras::default());
    app.add_startup_system(taa_setup);
    app.add_startup_system(apply_curve);
    app.insert_resource(RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Mesh });
    app.insert_resource(AaConfig { mode: AaMode::Taa });
    if mode == Mode::Cut {
        // A camera controller joins `CameraSet::Control` (`CameraSet`'s doc): the switch is then
        // ordered before the active-camera resolve AND `resolve_post_policy`, so the policy sees it
        // on frame SWITCH_AT itself — the frame test 16's oracle names.
        app.add_systems_cfg(|b| {
            b.add_system(switch_camera).in_set(CameraSet::Control);
        });
        app.add_systems(log_taa_state);
    }
    if matches!(mode, Mode::Ramp | Mode::Steady) {
        app.insert_resource(TaaConfig { luma_weight: false, ..TaaConfig::default() });
        app.add_systems_cfg(|b| {
            b.add_system(log_pre_exposure).after_set(PostPolicySet);
        });
    }
    if mode == Mode::Ramp {
        app.add_systems_cfg(|b| {
            b.add_system(ramp_exposure).before_set(PostPolicySet);
        });
    }
    app.run();
    assert!(app.world().resource::<HostFrameStats>().frames > 0, "the frame loop never ran — RED, never a skip");
}

/// [`CURVE`] on the plugin-inserted `LightingConfig` (every other field at its default).
#[allow(clippy::needless_pass_by_value)]
fn apply_curve(mut cfg: ResMut<LightingConfig>) {
    cfg.tonemapper = CURVE;
}

fn dump_dir(tag: &str) -> PathBuf {
    let dir = fresh_out(tag);
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    dir
}

/// `OETF(ACES(c))` of one channel through the traced `post_final` body ([`CURVE`], ratio 1).
fn display_of(rgb: [f32; 3]) -> [f32; 3] {
    let mut words = [0u32; 8];
    words[ubo_word("mode_words")] = (CURVE as u32) << TONEMAP_MODE_SHIFT;
    words[ubo_word("exposure_ratio")] = 1.0f32.to_bits();
    words[ubo_word("extent")] = 1;
    words[ubo_word("extent") + 1] = 1;
    let texel = [[rgb[0], rgb[1], rgb[2], 1.0]];
    let o = post_final_eval([0, 0], &eval_params(words), HostTex2::new(&texel, 1, 1));
    [o[0], o[1], o[2]]
}

/// The mean over channels of the standard deviation across `vals`.
fn std3(vals: &[[f32; 3]]) -> f64 {
    let n = vals.len() as f64;
    (0..3)
        .map(|c| {
            let mean = vals.iter().map(|v| f64::from(v[c])).sum::<f64>() / n;
            (vals.iter().map(|v| (f64::from(v[c]) - mean).powi(2)).sum::<f64>() / n).sqrt()
        })
        .sum::<f64>()
        / 3.0
}

/// The mean over channels.
fn mean3(v: [f32; 3]) -> f64 {
    (f64::from(v[0]) + f64::from(v[1]) + f64::from(v[2])) / 3.0
}

fn luma(c: [f64; 3]) -> f64 {
    REC709[0] * c[0] + REC709[1] * c[1] + REC709[2] * c[2]
}

/// The shipped resolve's blend space (`taa_resolve.comp.hlsl`'s default arm,
/// `TaaConfig::luma_weight` on): `w = c / (1 + luma(c))`.
fn karis(c: [f64; 3]) -> [f64; 3] {
    let k = 1.0 + luma(c);
    c.map(|x| x / k)
}

/// The inverse the resolve applies after the blend: `w / max(1 - luma(w), 1e-4)`.
fn karis_inv(w: [f64; 3]) -> [f64; 3] {
    let k = (1.0 - luma(w)).max(1e-4);
    w.map(|x| x / k)
}

/// One pixel's (D) burst, reduced to what test 18 reads.
#[derive(Clone, Copy, Debug)]
struct PixelBurst {
    /// The frame-to-frame std of `OETF(ACES(lit))` (mean over channels).
    lit_std: f64,
    /// The frame-to-frame std of `display` (mean over channels).
    display_std: f64,
    /// The largest `lit` channel over the burst: an HDR texel exceeds 1.0.
    lit_max: f32,
    /// What a pre-tonemap resolve converges to, through `post_final`: `OETF(ACES(·))` of the
    /// Karis-weighted burst mean of `lit` (mean over channels). The burst covers whole jitter
    /// periods, and past ~10 frames the resolve's blend is `1 / confidence`, a running mean.
    hdr_blend: f64,
    /// What a resolve done in display space converges to: the burst mean of `OETF(ACES(lit))`
    /// (mean over channels; a Karis-weighted display-space blend lands lower still).
    display_blend: f64,
    /// The burst mean of `display` (mean over channels).
    display_mean: f64,
}

/// The per-channel mean of a burst.
fn channel_mean(burst: impl Iterator<Item = [f64; 3]>) -> [f64; 3] {
    let (mut sum, mut n) = ([0.0f64; 3], 0.0f64);
    for c in burst {
        for (acc, x) in sum.iter_mut().zip(c) {
            *acc += x;
        }
        n += 1.0;
    }
    sum.map(|x| x / n)
}

/// `OETF(ACES(·))` of the Karis-weighted mean of a `lit` burst: the converged pre-tonemap
/// resolve, displayed (`post_final` evaluates in f32).
fn hdr_blend_of(lit: &[[f32; 3]]) -> [f32; 3] {
    display_of(karis_inv(channel_mean(lit.iter().map(|c| karis(c.map(f64::from))))).map(|x| x as f32))
}

/// Reduces one pixel's `lit` and `display` bursts (equal length, frame for frame).
fn pixel_burst(lit: &[[f32; 3]], display: &[[f32; 3]]) -> PixelBurst {
    let n = lit.len() as f64;
    let toned: Vec<[f32; 3]> = lit.iter().map(|&c| display_of(c)).collect();
    PixelBurst {
        lit_std: std3(&toned),
        display_std: std3(display),
        lit_max: lit.iter().flatten().copied().fold(f32::MIN, f32::max),
        hdr_blend: mean3(hdr_blend_of(lit)),
        display_blend: toned.iter().map(|&t| mean3(t)).sum::<f64>() / n,
        display_mean: display.iter().map(|&t| mean3(t)).sum::<f64>() / display.len() as f64,
    }
}

/// [`pixel_burst`] for every pixel of two dumped bursts of one extent.
fn pixel_bursts(lit: &[DumpFile], display: &[DumpFile]) -> Vec<PixelBurst> {
    let (w, h) = (lit[0].width, lit[0].height);
    assert!(
        lit.iter().chain(display).all(|d| (d.width, d.height) == (w, h)),
        "lit and display share one extent (no SSAA in this scene)"
    );
    let rgb = |d: &DumpFile, x: u32, y: u32| {
        let t = d.texel_f32(x, y);
        [t[0], t[1], t[2]]
    };
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let l: Vec<[f32; 3]> = lit.iter().map(|d| rgb(d, x, y)).collect();
            let d: Vec<[f32; 3]> = display.iter().map(|d| rgb(d, x, y)).collect();
            out.push(pixel_burst(&l, &d));
        }
    }
    out
}

/// Test 18's HDR premise: the HDR bar-edge pixels (`lit` above 1.0 in some frame, the
/// tonemapped HDR blend more than [`HDR_GAP`] above the display-space blend), or the reason
/// there are fewer than [`HDR_EDGE_MIN`].
fn hdr_edge_pixels(px: &[PixelBurst]) -> Result<Vec<PixelBurst>, String> {
    let edge: Vec<PixelBurst> =
        px.iter().filter(|p| p.lit_max > 1.0 && p.hdr_blend - p.display_blend > HDR_GAP).copied().collect();
    if edge.len() < HDR_EDGE_MIN {
        let lit_max = px.iter().map(|p| p.lit_max).fold(f32::MIN, f32::max);
        return Err(format!(
            "only {} HDR bar-edge pixels (lit > 1.0 in some burst frame and OETF(ACES(Karis-mean lit)) - \
             mean OETF(ACES(lit)) > {HDR_GAP}), fewer than {HDR_EDGE_MIN}: lit is not HDR at the 4.0 emissive \
             bar (lit's max over the burst is {lit_max:.3})",
            edge.len()
        ));
    }
    Ok(edge)
}

/// Test 18's resolve-space fraction over the HDR bar-edge pixels: how far `display`'s burst mean
/// sits from the display-space blend (0) toward the HDR-blend-then-tonemap expectation (1).
fn resolve_space_fraction(edge: &[PixelBurst]) -> f64 {
    let num: f64 = edge.iter().map(|p| p.display_mean - p.display_blend).sum();
    let den: f64 = edge.iter().map(|p| p.hdr_blend - p.display_blend).sum();
    num / den.max(f64::MIN_POSITIVE)
}

/// Test 18's verdict over two per-pixel std fields: `(edge pixels, display / tonemapped-lit ratio)`.
fn accumulation_ratio(display_std: &[f64], lit_std: &[f64]) -> (usize, f64) {
    let edges: Vec<usize> = (0..lit_std.len()).filter(|&i| lit_std[i] > EDGE_STD).collect();
    let mean = |v: &[f64]| edges.iter().map(|&i| v[i]).sum::<f64>() / edges.len().max(1) as f64;
    (edges.len(), mean(display_std) / mean(lit_std).max(f64::MIN_POSITIVE))
}

fn load_burst(dir: &Path, name: &str) -> Vec<DumpFile> {
    files_of(dir, name).into_iter().map(|(_, p)| DumpFile::load(&p)).collect()
}

#[test]
#[ignore = "gpu-windowed: PX1a test 18, TAA accumulation through post_final (spawns px1a_taa_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_taa_display_is_the_accumulated_image() {
    assert!(ImageDumpName::parse("display").is_some(), "no image-dump name `display`");
    let dir = dump_dir("px1a_taa_accumulate");
    run_worker(
        "px1a_taa_worker",
        &[
            (ENV_MODE, "accumulate".to_owned()),
            ("BOYKO_IMAGE_DUMP", "lit,display".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_IMAGE_DUMP_FRAMES", BURST.to_string()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + BURST + 10).to_string()),
        ],
        false,
    );
    let lit = load_burst(&dir, "lit");
    let display = load_burst(&dir, "display");
    assert_eq!((lit.len(), display.len()), (BURST as usize, BURST as usize), "a {BURST}-frame burst of lit and display");
    let px = pixel_bursts(&lit, &display);
    let hdr_edge = hdr_edge_pixels(&px).unwrap_or_else(|e| panic!("{e}"));
    let lit_std: Vec<f64> = px.iter().map(|p| p.lit_std).collect();
    let display_std: Vec<f64> = px.iter().map(|p| p.display_std).collect();
    let (edges, ratio) = accumulation_ratio(&display_std, &lit_std);
    assert!(edges >= 50, "only {edges} edge pixels: the scene has no jittered edge to measure");
    assert!(ratio <= 0.5, "display's frame-to-frame std is {ratio:.3} x OETF(ACES(lit))'s over {edges} edge pixels: no accumulation");
    let space = resolve_space_fraction(&hdr_edge);
    assert!(
        space >= RESOLVE_SPACE_MIN,
        "display at the 4.0 bar's edge sits {space:.3} of the way from the display-space blend to the \
         HDR-blend-then-tonemap expectation over {} pixels (bar {RESOLVE_SPACE_MIN}): the TAA resolve ran \
         after the tonemap, not pre-tonemap",
        hdr_edge.len()
    );
}

#[test]
#[ignore = "gpu-windowed: PX1a test 16, a camera cut resets the TAA history (spawns px1a_taa_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_taa_camera_switch_resets_the_history() {
    let dir = dump_dir("px1a_taa_cut");
    let log = dir.join("taa_state.log");
    run_worker(
        "px1a_taa_worker",
        &[
            (ENV_MODE, "cut".to_owned()),
            (ENV_LOG, log.display().to_string()),
            ("BOYKO_IMAGE_DUMP", "lit,taa_hist".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", (SWITCH_AT - 2).to_string()),
            ("BOYKO_IMAGE_DUMP_FRAMES", "6".to_owned()),
            ("BOYKO_WINDOW_FRAMES", (SWITCH_AT + 20).to_string()),
        ],
        false,
    );
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("no TaaState log at {} ({e})", log.display()));
    let rows: Vec<(u64, u32)> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    if let Err(e) = cut_log_verdict(&rows, SWITCH_AT) {
        panic!("TAA reset flag unset on the active-camera switch: {e}");
    }

    let lit = files_of(&dir, "lit");
    let hist = files_of(&dir, "taa_hist");
    assert!(!lit.is_empty() && lit.len() == hist.len(), "lit / taa_hist bursts of equal length: {} vs {}", lit.len(), hist.len());
    let fresh: Vec<u32> = lit
        .iter()
        .zip(&hist)
        .filter(|((fl, pl), (fh, ph))| {
            assert_eq!(fl, fh, "the two bursts cover the same frames");
            history_is_fresh(&DumpFile::load(pl), &DumpFile::load(ph))
        })
        .map(|((f, _), _)| *f)
        .collect();
    let cut_frame = u32::try_from(SWITCH_AT).expect("invariant: SWITCH_AT fits a dump frame number");
    assert_eq!(fresh, [cut_frame], "exactly the cut frame {SWITCH_AT} must hold a fresh history; fresh frames: {fresh:?}");
}

/// One test-13 worker run: its `ResolvedPost` log (`frame -> pre_exposure`) and its dumps of the
/// burst, frame for frame.
struct RampRun {
    pre: Vec<(u64, f32)>,
    lit: Vec<(u32, DumpFile)>,
    hist: Vec<(u32, DumpFile)>,
    display: Vec<(u32, DumpFile)>,
}

fn ramp_run(mode: &str) -> RampRun {
    let dir = dump_dir(&format!("px1a_taa_{mode}"));
    let log = dir.join("pre_exposure.log");
    let settle = RAMP_AT + 1;
    run_worker(
        "px1a_taa_worker",
        &[
            (ENV_MODE, mode.to_owned()),
            (ENV_LOG, log.display().to_string()),
            ("BOYKO_IMAGE_DUMP", "lit,taa_hist,display".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", settle.to_string()),
            ("BOYKO_IMAGE_DUMP_FRAMES", RAMP_FRAMES.to_string()),
            ("BOYKO_WINDOW_FRAMES", (settle + u64::from(RAMP_FRAMES) + 10).to_string()),
        ],
        false,
    );
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("no ResolvedPost log at {} ({e})", log.display()));
    let pre = text
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, f32::from_bits(it.next()?.parse().ok()?)))
        })
        .collect();
    let load = |name: &str| -> Vec<(u32, DumpFile)> {
        let files: Vec<(u32, DumpFile)> = files_of(&dir, name).into_iter().map(|(f, p)| (f, DumpFile::load(&p))).collect();
        assert_eq!(files.len(), RAMP_FRAMES as usize, "{mode}: a {RAMP_FRAMES}-frame burst of `{name}`");
        files
    };
    RampRun { pre, lit: load("lit"), hist: load("taa_hist"), display: load("display") }
}

/// The sum of every texel's RGB over `pick`ed pixels (`None` = every pixel).
fn rgb_sum(d: &DumpFile, pick: Option<&[(u32, u32)]>) -> f64 {
    let px = |x: u32, y: u32| {
        let t = d.texel_f32(x, y);
        f64::from(t[0]) + f64::from(t[1]) + f64::from(t[2])
    };
    match pick {
        Some(set) => set.iter().map(|&(x, y)| px(x, y)).sum(),
        None => (0..d.height).flat_map(|y| (0..d.width).map(move |x| (x, y))).map(|(x, y)| px(x, y)).sum(),
    }
}

/// The steady run's history-bearing pixels at one frame: some channel of its history differs from
/// its current `lit` by more than [`HISTORY_BEARING`] of the larger of the two.
fn history_bearing(lit: &DumpFile, hist: &DumpFile) -> Vec<(u32, u32)> {
    (0..lit.height)
        .flat_map(|y| (0..lit.width).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let (l, h) = (lit.texel_f32(x, y), hist.texel_f32(x, y));
            (0..3).any(|c| {
                let big = f64::from(l[c].abs().max(h[c].abs()));
                big > 1e-3 && f64::from((h[c] - l[c]).abs()) > HISTORY_BEARING * big
            })
        })
        .collect()
}

/// One ramp frame, reduced to what test 13 judges.
#[derive(Clone, Copy, Debug)]
struct RampFrame {
    /// The dump frame.
    frame: u32,
    /// `ResolvedPost::pre_exposure` the ramp run logged for this frame.
    pre: f32,
    /// The same, logged by the steady run.
    steady_pre: f32,
    /// `Σ lit_ramp / Σ lit_steady` over every pixel.
    lit_ratio: f64,
    /// `Σ taa_hist_ramp / Σ taa_hist_steady` over the history-bearing pixels.
    hist_ratio: f64,
    /// How many history-bearing pixels the frame has.
    bearing: usize,
}

/// Test 13's verdict. `Err` names the first failing frame and statement.
fn ramp_verdict(frames: &[RampFrame]) -> Result<(), String> {
    if frames.len() != RAMP_FRAMES as usize {
        return Err(format!("{} ramp frames, expected {RAMP_FRAMES}", frames.len()));
    }
    for (k, f) in (1..).zip(frames) {
        let want = RAMP_STEP.powi(k);
        if f.pre != want || f.steady_pre != 1.0 {
            return Err(format!(
                "frame {}: ResolvedPost::pre_exposure = {} (steady run {}), expected {want} (the ramp's k = {k}): \
                 the policy did not read this frame's exposure, or the dump frames are not the log's",
                f.frame, f.pre, f.steady_pre
            ));
        }
        let rel = |r: f64| (r / f64::from(f.pre) - 1.0).abs();
        if rel(f.lit_ratio) > RAMP_REL_TOL {
            return Err(format!(
                "frame {}: sum lit_ramp / sum lit_steady = {:.5}, not pre = {} (within {RAMP_REL_TOL}): the pre-exposure \
                 did not reach the producers this frame (the header-only light rewrite)",
                f.frame, f.lit_ratio, f.pre
            ));
        }
        if f.bearing < BEARING_MIN {
            return Err(format!("frame {}: only {} history-bearing pixels (min {BEARING_MIN})", f.frame, f.bearing));
        }
        if rel(f.hist_ratio) > RAMP_REL_TOL {
            return Err(format!(
                "frame {}: over {} history-bearing pixels sum taa_hist_ramp / sum taa_hist_steady = {:.5}, not pre = {} \
                 (within {RAMP_REL_TOL}): the history was not rescaled by the exposure ratio (pre_ratio_delta)",
                f.frame, f.bearing, f.hist_ratio, f.pre
            ));
        }
    }
    Ok(())
}

#[test]
#[ignore = "gpu-windowed: PX1a test 13, the exposure ramp through ResolvedPost (spawns px1a_taa_worker twice); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_taa_exposure_ramp_rescales_the_history() {
    let ramp = ramp_run("ramp");
    let steady = ramp_run("steady");
    let pre_of = |run: &RampRun, f: u32| {
        run.pre.iter().find(|r| r.0 == u64::from(f)).map(|r| r.1).unwrap_or_else(|| panic!("no log row for frame {f}"))
    };
    let frames: Vec<RampFrame> = (0..RAMP_FRAMES as usize)
        .map(|i| {
            let f = ramp.lit[i].0;
            for run in [&ramp, &steady] {
                assert_eq!((run.lit[i].0, run.hist[i].0), (f, f), "the two runs' bursts cover the same frames");
            }
            let bearing = history_bearing(&steady.lit[i].1, &steady.hist[i].1);
            let row = RampFrame {
                frame: f,
                pre: pre_of(&ramp, f),
                steady_pre: pre_of(&steady, f),
                lit_ratio: rgb_sum(&ramp.lit[i].1, None) / rgb_sum(&steady.lit[i].1, None),
                hist_ratio: rgb_sum(&ramp.hist[i].1, Some(&bearing)) / rgb_sum(&steady.hist[i].1, Some(&bearing)),
                bearing: bearing.len(),
            };
            eprintln!("px1a test 13: {row:?}");
            row
        })
        .collect();
    if let Err(e) = ramp_verdict(&frames) {
        panic!("{e}");
    }
}

/// `post_eval`'s per-channel bound (`post_eval.rs` `channel_bound`, no filter row): one display code
/// (the two 8-bit roundings) + E0's bound on the OETF's `pow` at the body value `o`.
fn eval_bound_lsb(o: f32) -> f64 {
    let g = f64::from(OETF_GAMMA_EXP);
    let x = f64::from(o.max(0.0)).powf(1.0 / g);
    let oetf = if x <= 0.0 { 0.0 } else { 255.0 * vk_pow_bound(x as f32, OETF_GAMMA_EXP, x.log2(), x.powf(g)) };
    1.0 + oetf
}

/// The eval-post comparison of one frame: `display` against `post_final_eval` of `scene` under
/// `words`, every pixel. Returns the number of channel samples outside [`eval_bound_lsb`] and the
/// first few, described.
fn eval_post_mismatches(scene: &DumpFile, display: &DumpFile, words: [u32; 8]) -> (usize, Vec<String>) {
    assert_eq!((scene.width, scene.height), (display.width, display.height), "display is the scene input's extent");
    let texels: Vec<[f32; 4]> =
        (0..scene.height).flat_map(|y| (0..scene.width).map(move |x| (x, y))).map(|(x, y)| scene.texel_f32(x, y)).collect();
    let params = eval_params(words);
    let tex = HostTex2::new(&texels, scene.width, scene.height);
    let (mut bad, mut first) = (0usize, Vec::new());
    for y in 0..scene.height {
        for x in 0..scene.width {
            let o = post_final_eval([x, y], &params, tex);
            let d = display.rgba8(x, y);
            for c in 0..3 {
                let delta = (255.0 * f64::from(o[c]) - f64::from(d[c])).abs();
                if delta > eval_bound_lsb(o[c]) {
                    bad += 1;
                    if first.len() < 4 {
                        first.push(format!("({x},{y}) ch{c}: display {} vs eval {:.3}", d[c], 255.0 * o[c]));
                    }
                }
            }
        }
    }
    (bad, first)
}

/// The `post_final` words the runner uploads for this worker: [`CURVE`] (the worker sets it) through
/// `post_host::post_final_gpu` (ratio 1), the window's extent.
fn runner_post_words() -> [u32; 8] {
    let post = ResolvedPost::from_config(&LightingConfig { tonemapper: CURVE, ..LightingConfig::default() });
    boyko_render::post_final_words(&boyko_app::post_host::post_final_gpu(&post, [WIN, WIN]))
}

#[test]
#[ignore = "gpu-windowed: PX1a eval-post pin, display == post_final_eval(taa_hist[fi]) on the ramp's TAA frames (spawns px1a_taa_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_taa_display_is_post_final_of_this_frames_history() {
    let run = ramp_run("ramp");
    let words = runner_post_words();
    for ((f, hist), (fd, display)) in run.hist.iter().zip(&run.display) {
        assert_eq!(f, fd, "the taa_hist and display bursts cover the same frames");
        let (bad, first) = eval_post_mismatches(hist, display, words);
        assert!(
            bad == 0,
            "frame {f}: {bad} display channel samples differ from post_final_eval(taa_hist[fi]) beyond one code + \
             the OETF bound (first: {first:?}): post_final did not read this frame's history"
        );
    }
}

/// Test 16's verdict over the cut worker's `frame frames_since_reset` log.
///
/// **Why these rows.** `log_taa_state` runs in the ECS schedule, BEFORE the runner's reset site
/// (`mark_reset()` then `advance()`), so row `f` reads the state frame `f - 1` left, and
/// `TaaState`'s contract makes the reset frame itself count as frame 1. The boot reset at frame 0
/// therefore logs `0 0`, `1 1`, `2 2`, …; a cut applied on the switch frame `cut` (where the policy
/// sees the new active camera) logs row `cut` = the pre-cut count (`row(cut - 1) + 1`) and row
/// `cut + 1` = 1. A reset placed one frame late (after `advance()`) logs 0 at `cut + 1` and 1 at
/// `cut + 2`; a missing one keeps counting — both are RED here. (Ruling 70 re-stated this oracle:
/// the first one asked for 0 at `cut + 1`, which only the late reset satisfies.)
///
/// Checked: the log is gap-free from frame 0; row `cut` reads the pre-cut count; row `cut + 1`
/// reads 1; and the only discontinuity after the boot reset (a row `f >= 2` that is not
/// `row(f - 1) + 1`) is at `cut + 1` — exactly one reset in the run after the boot reset.
fn cut_log_verdict(rows: &[(u64, u32)], cut: u64) -> Result<(), String> {
    if rows.first().map(|r| r.0) != Some(0) || rows.windows(2).any(|w| w[1].0 != w[0].0 + 1) {
        return Err(format!("the log is not one row per frame from frame 0: {:?}", rows.iter().map(|r| r.0).collect::<Vec<_>>()));
    }
    let row = |f: u64| rows.get(usize::try_from(f).ok()?).map(|r| r.1);
    let (Some(before), Some(at), Some(after)) = (row(cut - 1), row(cut), row(cut + 1)) else {
        return Err(format!("the log ends before frame {} ({} rows)", cut + 1, rows.len()));
    };
    if at != before + 1 {
        return Err(format!("row {cut} = {at}, not the pre-cut count {} (a reset before the switch frame)", before + 1));
    }
    if after != 1 {
        return Err(format!("row {} = {after}, not 1: the switch frame {cut} did not reset the history", cut + 1));
    }
    let resets: Vec<u64> = rows.windows(2).filter(|w| w[1].0 >= 2 && w[1].1 != w[0].1 + 1).map(|w| w[1].0).collect();
    if resets != [cut + 1] {
        return Err(format!("resets after the boot reset (rows that do not count on): {resets:?}, expected exactly [{}]", cut + 1));
    }
    Ok(())
}

/// `true` iff `taa_hist` equals `lit` at every texel within one binary16 step (a replaced
/// history): the steady state blends and differs at the jittered edges.
fn history_is_fresh(lit: &DumpFile, hist: &DumpFile) -> bool {
    (0..lit.height).all(|y| {
        (0..lit.width).all(|x| {
            let (a, b) = (lit.texel_f32(x, y), hist.texel_f32(x, y));
            (0..3).all(|c| (a[c] - b[c]).abs() <= a[c].abs().max(1.0) * (1.0 / 1024.0))
        })
    })
}

// ---- host checks of the verdicts (not ignored) -----------------------------------------------

/// A 47-row cut-worker log (R0's length): the boot reset at frame 0, then the runner's reset at
/// each frame of `resets_at` (`mark_reset` then `advance`), or one frame late (`advance` then
/// `mark_reset`) when `late`.
fn synthetic_cut_log(resets_at: &[u64], late: bool) -> Vec<(u64, u32)> {
    let mut count = 0u32;
    (0..47u64)
        .map(|f| {
            let row = (f, count);
            if f == 0 || (!late && resets_at.contains(&f)) {
                count = 1;
            } else if late && resets_at.contains(&f) {
                count = 0;
            } else {
                count += 1;
            }
            row
        })
        .collect()
}

#[test]
fn the_cut_log_verdict_pins_the_reset_to_the_switch_frame() {
    assert_eq!(synthetic_cut_log(&[], false)[..3], [(0, 0), (1, 1), (2, 2)], "the boot shape R0 logged");
    assert_eq!(cut_log_verdict(&synthetic_cut_log(&[SWITCH_AT], false), SWITCH_AT), Ok(()));
    assert!(cut_log_verdict(&synthetic_cut_log(&[], false), SWITCH_AT).is_err(), "no cut reset (R0's log) must red");
    assert!(cut_log_verdict(&synthetic_cut_log(&[SWITCH_AT], true), SWITCH_AT).is_err(), "a reset after advance() must red");
    assert!(cut_log_verdict(&synthetic_cut_log(&[SWITCH_AT - 1], false), SWITCH_AT).is_err(), "a reset one frame early must red");
    assert!(cut_log_verdict(&synthetic_cut_log(&[SWITCH_AT + 1], false), SWITCH_AT).is_err(), "a reset one frame late must red");
    let twice = synthetic_cut_log(&[SWITCH_AT, SWITCH_AT + 3], false);
    assert!(cut_log_verdict(&twice, SWITCH_AT).is_err(), "a second reset after the cut must red");
    let mut gap = synthetic_cut_log(&[SWITCH_AT], false);
    gap.remove(10);
    assert!(cut_log_verdict(&gap, SWITCH_AT).is_err(), "a missing row must red, never hide a reset");
}

#[test]
fn the_accumulation_ratio_separates_a_resolved_display_from_a_jittered_one() {
    let lit_std: Vec<f64> = (0..400).map(|i| if i % 4 == 0 { 0.2 } else { 0.0 }).collect();
    let resolved: Vec<f64> = lit_std.iter().map(|s| s * 0.2).collect();
    let (edges, ratio) = accumulation_ratio(&resolved, &lit_std);
    assert_eq!(edges, 100);
    assert!(ratio <= 0.5, "{ratio}");
    let (_, bypass) = accumulation_ratio(&lit_std, &lit_std);
    assert!(bypass > 0.5, "a display that IS the tonemapped lit must red: {bypass}");
}

/// A synthetic 4.0-bar edge: 12 pixels per coverage `k / 8` of the 8-phase jitter, `k` in 0..=8
/// (the bar on `k` of every 8 frames, the floor otherwise; `lit` mapped through `lit_of`), each
/// pixel's `display` burst produced from its `lit` burst by `resolve`.
fn synthetic_bar_edge(
    lit_of: impl Fn([f32; 3]) -> [f32; 3],
    resolve: impl Fn(&[[f32; 3]]) -> Vec<[f32; 3]>,
) -> Vec<PixelBurst> {
    (0..=8u32)
        .flat_map(|k| std::iter::repeat_n(k, 12))
        .map(|k| {
            let lit: Vec<[f32; 3]> =
                (0..BURST).map(|f| lit_of(if f % 8 < k { [4.0; 3] } else { [0.0365; 3] })).collect();
            pixel_burst(&lit, &resolve(&lit))
        })
        .collect()
}

#[test]
fn the_resolve_space_fraction_separates_a_pre_tonemap_resolve_from_a_display_space_one() {
    let hdr = |c: [f32; 3]| c;
    let steady = |c: [f32; 3], n: usize| vec![c; n];
    let pre_tonemap = synthetic_bar_edge(hdr, |lit| steady(hdr_blend_of(lit), lit.len()));
    let edge = hdr_edge_pixels(&pre_tonemap).expect("the HDR bar edge has its pixels");
    assert_eq!(edge.len(), 7 * 12, "every partial coverage is an HDR bar-edge pixel, no full one");
    let f = resolve_space_fraction(&edge);
    assert!(f >= RESOLVE_SPACE_MIN, "the converged pre-tonemap resolve must pass: {f}");

    let toned = |lit: &[[f32; 3]]| lit.iter().map(|&c| display_of(c)).collect::<Vec<_>>();
    let plain = synthetic_bar_edge(hdr, |lit| {
        let m = channel_mean(toned(lit).into_iter().map(|t| t.map(f64::from))).map(|x| x as f32);
        steady(m, lit.len())
    });
    let f = resolve_space_fraction(&hdr_edge_pixels(&plain).expect("the same lit"));
    assert!(f < RESOLVE_SPACE_MIN, "a plain display-space blend must red: {f}");
    let karis_display = synthetic_bar_edge(hdr, |lit| {
        let w = channel_mean(toned(lit).into_iter().map(|t| karis(t.map(f64::from))));
        steady(karis_inv(w).map(|x| x as f32), lit.len())
    });
    let f = resolve_space_fraction(&hdr_edge_pixels(&karis_display).expect("the same lit"));
    assert!(f < RESOLVE_SPACE_MIN, "a Karis-weighted display-space blend must red: {f}");

    let ldr = synthetic_bar_edge(display_of, |lit| lit.to_vec());
    assert!(hdr_edge_pixels(&ldr).is_err(), "a display-referred lit (RGBA8 until the switch) must red the premise");
    let clamped = synthetic_bar_edge(|c| c.map(|x| x.min(1.0)), |lit| steady(hdr_blend_of(lit), lit.len()));
    assert!(hdr_edge_pixels(&clamped).is_err(), "a lit clamped at 1.0 must red the premise");
}

/// A correct ramp frame `k` (1-based), with the given ratio errors.
fn synthetic_ramp_frame(k: i32, lit_err: f64, hist_err: f64) -> RampFrame {
    let pre = RAMP_STEP.powi(k);
    RampFrame {
        frame: u32::try_from(RAMP_AT).expect("invariant: fits") + k as u32,
        pre,
        steady_pre: 1.0,
        lit_ratio: f64::from(pre) * (1.0 + lit_err),
        hist_ratio: f64::from(pre) * (1.0 + hist_err),
        bearing: 400,
    }
}

#[test]
fn the_ramp_verdict_reds_a_missing_header_path_and_a_missing_rescale() {
    let ok: Vec<RampFrame> = (1..=RAMP_FRAMES as i32).map(|k| synthetic_ramp_frame(k, 1e-4, -1e-4)).collect();
    assert_eq!(ramp_verdict(&ok), Ok(()));
    // The header-only path removed: `lit` stays at exposure 1 while the policy ramps.
    let no_header: Vec<RampFrame> =
        ok.iter().map(|f| RampFrame { lit_ratio: 1.0, hist_ratio: 1.0, ..*f }).collect();
    assert!(ramp_verdict(&no_header).unwrap_err().contains("header-only light rewrite"));
    // `pre_ratio_delta` forced 0: the history lags the new scale (one step of 1.25 ≈ 20 % on the
    // history-bearing pixels before the blend).
    let no_rescale: Vec<RampFrame> = ok.iter().map(|f| RampFrame { hist_ratio: f.lit_ratio * 0.9, ..*f }).collect();
    assert!(ramp_verdict(&no_rescale).unwrap_err().contains("pre_ratio_delta"));
    // The policy a frame late (the ramp system unordered against `PostPolicySet`).
    let late: Vec<RampFrame> = ok.iter().map(|f| RampFrame { pre: f.pre / RAMP_STEP, ..*f }).collect();
    assert!(ramp_verdict(&late).unwrap_err().contains("this frame's exposure"));
    let empty: Vec<RampFrame> = ok.iter().map(|f| RampFrame { bearing: 3, ..*f }).collect();
    assert!(ramp_verdict(&empty).unwrap_err().contains("history-bearing"));
}

/// An RGBA16F / RGBA8 dump pair built in memory (the `OBSD` header `DumpFile::parse` reads).
fn synthetic_dump(format: u32, w: u32, h: u32, texel: impl Fn(u32, u32) -> Vec<u8>) -> DumpFile {
    let texel_bytes = if format == image_dump::VK_R16G16B16A16_SFLOAT { 8 } else { 4 };
    let mut texels = Vec::new();
    for y in 0..h {
        for x in 0..w {
            texels.extend(texel(x, y));
        }
    }
    DumpFile { format, width: w, height: h, mip: 0, layers: 1, frame: 0, texel_bytes, texels }
}

#[test]
fn the_eval_post_comparison_separates_this_frames_history_from_last_frames() {
    let (w, h) = (16u32, 8u32);
    let radiance = |x: u32, y: u32| 0.02 + 0.3 * x as f32 + 0.1 * y as f32;
    let f16 = |v: f32| u16::try_from(boyko_shaderdsl::half::f32_to_f16_bits(v)).expect("invariant: a binary16 code");
    let hist_of = |scale: f32| {
        synthetic_dump(image_dump::VK_R16G16B16A16_SFLOAT, w, h, |x, y| {
            let v = f16(radiance(x, y) * scale);
            [v, v, v, f16(1.0)].iter().flat_map(|b| b.to_le_bytes()).collect()
        })
    };
    let words = runner_post_words();
    let this = hist_of(1.0);
    let texels: Vec<[f32; 4]> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| this.texel_f32(x, y)).collect();
    let params = eval_params(words);
    let display = synthetic_dump(image_dump::VK_R8G8B8A8_UNORM, w, h, |x, y| {
        let o = post_final_eval([x, y], &params, HostTex2::new(&texels, w, h));
        let q = |c: f32| (c * 255.0).round().clamp(0.0, 255.0) as u8;
        vec![q(o[0]), q(o[1]), q(o[2]), 255]
    });
    assert_eq!(eval_post_mismatches(&this, &display, words).0, 0, "this frame's history, quantised: within the bound");
    let last = hist_of(1.0 / RAMP_STEP);
    let (bad, _) = eval_post_mismatches(&last, &display, words);
    assert!(bad > (w * h) as usize, "last frame's history (1/1.25 of this one) must red on most samples: {bad}");
}

/// Render wave PXt (D-G2): the words this binary's device pins compare against name ACES whatever
/// the engine default is — the thresholds were calibrated under it.
#[test]
fn the_runner_words_name_the_curve_this_binary_is_calibrated_under() {
    let mode = (runner_post_words()[ubo_word("mode_words")] >> TONEMAP_MODE_SHIFT) & TONEMAP_MODE_MASK;
    assert_eq!(mode, TONEMAP_ACES, "runner_post_words names tonemap word {mode}, not ACES: the oracle and thresholds are ACES-calibrated");
}

#[test]
fn display_of_is_aces_then_the_oetf() {
    assert_eq!(display_of([0.0; 3]), [0.0; 3]);
    let w = display_of([64.0; 3]);
    assert!(w.iter().all(|&c| c > 0.99), "a hot radiance displays white: {w:?}");
}
