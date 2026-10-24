//! Render wave PX1a tests 11 (here from C4), 11b and 15 (the switch commit) and 12 (the NaN hook)
//! — `plan/H4-PX1a-hdr-lit.md` Tests 11, 11b, 12, 15; `px1a/cut.md` C4, C7, C11: **`lit` holds
//! scene-linear radiance above 1.0, and the display shoulders it.**
//!
//! **11 — the 50× sun.** A glossy SDF sphere under a sun at 50 × the reference illuminance (no
//! SkyLight, so the background is the dark constant). The sun core is the brightest `lit` texel
//! inside the sphere's region. There:
//! - (D) `lit` (`lit_native` under SSAA) holds radiance **> 1.0** — at HEAD, whose `lit` is
//!   display-referred RGBA8, it is ≤ 1.0 by construction;
//! - (D) `display` is ≥ 254/255 in every channel;
//! - **the display shoulders it** (re-stated, ruling 79 F2): over the SHOULDER SAMPLES — every pixel
//!   whose scene texel a tonemap and a hard clip display differently (≥ [`CLIP_GAP`] codes in some
//!   channel: `OETF(ACES(lit))` against `OETF(saturate(lit))`) — `display` equals `post_final_eval`
//!   of the scene texel within `post_eval`'s bound, and the samples cross the knee (some with `lit`
//!   above 1.0, some below). The ray walk this replaced read 25 pixels that all sat at 255 (`lit` ≥
//!   34 over the walk), so a hard clip passed it; on the shoulder samples a clip is ≥ [`CLIP_GAP`]
//!   codes off by construction (mutation leg "hard clip instead of the tonemap").
//!
//! **11b — every producer stores above 1.0** (from the switch commit, C7). Each of PX0's HOT branch
//! fixtures (`common/hdr_fixtures.rs`: one per `lit`-store producer — `deferred_pbr` surface and
//! sky, `forward_opaque` (+ `_froxel`), `forward_sky`, `vb_sky`, `sdf_forward_march` on Forward and
//! VB, `vb_resolve`, `vb_shade`, `vb_shade_split` — and the SSAA box): the fixture's receipt keys
//! select its producer, and (D) `lit` (`lit_native` under SSAA) over the fixture's region holds
//! `max > 1.0` and equals the region's representable radiance (`HOT_R = 2.0`, exact in binary16)
//! within one format step in every texel. A clamp left at ANY one store site reds exactly that
//! producer's driver (mutation leg C7 (i): `vb_shade_split.comp.hlsl`'s clamp).
//!
//! **15 — particle E1** (C7). The particle lab scene (`particle_scene`, VB, the `particle_additive`
//! pin's configuration) at exposure 1.0 and 0.5, plus the particle-off run at 1.0 that masks the
//! particle pixels: over every pixel a particle reaches, (D) `lit` at exposure 0.5 equals half of
//! `lit` at exposure 1.0 within one binary16 step per channel (a power-of-two scale commutes with
//! every rounding on the way, so the halving is exact up to the blend's measured one-step error,
//! `goldens::RGBA16F_ROUNDING`). Red without the FS's `× pre_exposure` (mutation leg C7 (iv)): the
//! particle's own radiance does not halve.
//!
//! **12 — NaN injection** (C11; H4 Tests 12, K8). PX0's `deferred_base` fixture under TAA, run twice
//! through the fixture worker: once with the test-only `inject_nan_lit` force key — the
//! `test_nan_inject` pass copies a binary16 NaN into `lit` at `present::NAN_INJECT_TEXEL` (inside the
//! flat top-left background) on engine frame `test_force::INJECT_NAN_LIT_FRAME` — and once clean.
//! Sixteen frames later (D) `taa_hist` holds no non-finite texel anywhere, the injection site
//! included, and (D) `lit` and `taa_hist` equal the clean run's within one binary16 step per
//! channel. Red on the mutated HDR tree (GL-PX1a-D, C11): `taa_resolve` without its K8 boundary
//! (the `hdr_sanitize` taps AND the `hdr_history_guard` write) carries the NaN into the history.
//!
//! Windowed-test conventions: the drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/hdr_fixtures.rs"]
mod hdr_fixtures;
#[path = "common/image_dump.rs"]
mod image_dump;
#[path = "particle_scene/mod.rs"]
mod particle_scene;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{AaConfig, AaMode, GeometryLegs, Material, RenderPath, RenderPathConfig};
use boyko_rhi_vulkan::present::ImageDumpName;

use boyko_render::light::LightingConfig;
use boyko_render::post::ResolvedPost;
use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::pixfmt::{Channel, steps_between};
use boyko_shaderdsl::post::{OETF_GAMMA_EXP, TONEMAP_ACES, TONEMAP_MODE_SHIFT, eval_params, post_final_eval, ubo_word};
use boyko_shaderdsl::prims_probe::vk_pow_bound;
use graph_receipt::{fresh_out, run_worker};
use hdr_fixtures::{ENV_AA, ENV_FIXTURE, ENV_TEST_FORCE, Expect, HOT_R, Hot, capture_scene, fixture, fixture_worker_main};
use image_dump::{DumpFile, files_of};

/// The worker's knob: `sun` | `sun_ssaa`.
const ENV_HOT: &str = "BOYKO_PX1A_HOT";
/// The window edge.
const WIN: u32 = 256;
/// Frames before the capture.
const SETTLE: u32 = 30;
/// The sun's illuminance: 50 × the OBS scenes' 2.8 lux.
const SUN_LUX: f32 = 2.8 * 50.0;
/// The sun's direction TO the light: up, right and toward the camera, so the highlight faces it.
const SUN_DIR: [f32; 3] = [0.35, 0.55, 1.0];
/// A shoulder sample: the tonemap and a hard clip display its scene texel at least this many codes
/// apart in some channel (well above the eval bound's one code, so a clip cannot pass on it).
const CLIP_GAP: f64 = 8.0;
/// Test 11's anti-vacuity: the fewest shoulder samples, and the fewest on each side of `lit` = 1.0.
const SHOULDER_MIN: usize = 64;
const SHOULDER_SIDE_MIN: usize = 8;

#[derive(Resource)]
struct HotRes(&'static str);

#[allow(clippy::needless_pass_by_value)]
fn sun_setup(mut commands: Commands, mut materials: ResMut<Assets<Material>>, hot: Res<HotRes>) {
    let _ = hot.0;
    let glossy = materials.add(Material::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.15, 0.5, [0.0; 3], 0));
    let row = u16::try_from(glossy.index()).expect("invariant: < 65536 rows");
    commands.spawn(SdfPrimitive(SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0).with_material(row)));
    let pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], SUN_LUX),
    });
    let cam = Affine3A::look_at_rh(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: cam.translation, rotation: Quat::from_mat3(cam.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: core::f32::consts::FRAC_PI_3, aspect: 1.0, near: 0.1, far: 100.0 },
    });
}

/// **THE WORKER** — `BOYKO_PX1A_HOT=sun|sun_ssaa`.
#[test]
#[ignore = "gpu-windowed: the PX1a hot-fixture worker, spawned by the px1a_hot_fixtures drivers; run with --test-threads=1"]
fn px1a_hot_worker() {
    let Ok(which) = std::env::var(ENV_HOT) else {
        eprintln!("px1a_hot_worker: {ENV_HOT} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    let which: &'static str = match which.trim() {
        "sun" => "sun",
        "sun_ssaa" => "sun_ssaa",
        other => panic!("{ENV_HOT}={other} is not sun|sun_ssaa"),
    };
    let ssaa = which == "sun_ssaa";
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine px1a hot fixture", WIN, WIN);
    app.add_plugins(if ssaa { plugins.with_ssaa_scale(2) } else { plugins });
    app.insert_resource(HotRes(which));
    app.add_startup_system(sun_setup);
    app.insert_resource(RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Sdf });
    if !ssaa {
        app.insert_resource(AaConfig { mode: AaMode::Off });
    }
    app.run();
    assert!(app.world().resource::<HostFrameStats>().frames > 0, "the frame loop never ran — RED, never a skip");
}

fn luma(t: [f32; 4]) -> f32 {
    0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]
}

/// The sun core: the brightest texel of `scene` (the radiance image) — the sphere fills the centre.
fn sun_core(scene: &DumpFile) -> (u32, u32) {
    let mut best = (0, 0);
    let mut best_y = f32::MIN;
    for y in 0..scene.height {
        for x in 0..scene.width {
            let l = luma(scene.texel_f32(x, y));
            if l > best_y {
                best_y = l;
                best = (x, y);
            }
        }
    }
    best
}

/// `post_eval`'s per-channel bound (`post_eval.rs` `channel_bound`, no filter row): one display code
/// (the two 8-bit roundings) + E0's bound on the OETF's `pow` at the body value `o`.
fn eval_bound_lsb(o: f32) -> f64 {
    let g = f64::from(OETF_GAMMA_EXP);
    let x = f64::from(o.max(0.0)).powf(1.0 / g);
    let oetf = if x <= 0.0 { 0.0 } else { 255.0 * vk_pow_bound(x as f32, OETF_GAMMA_EXP, x.log2(), x.powf(g)) };
    1.0 + oetf
}

/// The `post_final` words the runner uploads for the default config at `extent` (the default
/// curve — Khronos PBR Neutral since render wave PXt —, ratio 1).
fn runner_post_words(extent: [u32; 2]) -> [u32; 8] {
    let post = ResolvedPost::from_config(&LightingConfig::default());
    boyko_render::post_final_words(&boyko_app::post_host::post_final_gpu(&post, extent))
}

/// Test 11's display half, over the scene image (`w × h` RGBA texels) and `display` (RGB codes):
/// `display` at the core is ≥ 254 in every channel; and over the shoulder samples (module doc)
/// `display` is `post_final_eval` of the scene texel under `words` within [`eval_bound_lsb`],
/// with at least [`SHOULDER_MIN`] samples, [`SHOULDER_SIDE_MIN`] of them on each side of `lit` =
/// 1.0. `Ok(samples)` or the failure, named.
fn check_display_shoulder(
    scene: &[[f32; 4]],
    display: &[[u8; 3]],
    (w, h): (u32, u32),
    core: (u32, u32),
    words: [u32; 8],
) -> Result<usize, String> {
    let c = display[(core.1 * w + core.0) as usize];
    if c.iter().any(|&v| v < 254) {
        return Err(format!("display at the sun core {core:?} is {c:?}, not >= 254/255"));
    }
    let params = eval_params(words);
    let tex = HostTex2::new(scene, w, h);
    let clip_code = |x: f32| 255.0 * f64::from(x.clamp(0.0, 1.0)).powf(f64::from(OETF_GAMMA_EXP));
    let (mut samples, mut above, mut below) = (0usize, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let o = post_final_eval([x, y], &params, tex);
            if !(0..3).any(|k| (255.0 * f64::from(o[k]) - clip_code(scene[i][k])).abs() >= CLIP_GAP) {
                continue;
            }
            samples += 1;
            if scene[i][..3].iter().any(|&v| v > 1.0) {
                above += 1;
            } else {
                below += 1;
            }
            for k in 0..3 {
                let got = f64::from(display[i][k]);
                if (255.0 * f64::from(o[k]) - got).abs() > eval_bound_lsb(o[k]) {
                    return Err(format!(
                        "display at ({x},{y}) ch{k} is {got}, the tonemap of lit {:.4} is {:.2} and a hard clip {:.2}: \
                         the display does not shoulder the highlight",
                        scene[i][k],
                        255.0 * o[k],
                        clip_code(scene[i][k])
                    ));
                }
            }
        }
    }
    if samples < SHOULDER_MIN || above < SHOULDER_SIDE_MIN || below < SHOULDER_SIDE_MIN {
        return Err(format!(
            "{samples} shoulder samples ({above} with lit above 1.0, {below} at or below), fewer than \
             {SHOULDER_MIN} ({SHOULDER_SIDE_MIN} per side): the scene does not cross the tonemap's knee"
        ));
    }
    Ok(samples)
}

fn drive_sun(ssaa: bool) {
    let scene_name = if ssaa { "lit_native" } else { "lit" };
    assert!(ImageDumpName::parse(scene_name).is_some(), "no image-dump name `{scene_name}`");
    let display_known = ImageDumpName::parse("display").is_some();
    let names = if display_known { format!("{scene_name},display") } else { scene_name.to_owned() };
    let tag = if ssaa { "px1a_hot_sun_ssaa" } else { "px1a_hot_sun" };
    let dir = fresh_out(tag);
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    run_worker(
        "px1a_hot_worker",
        &[
            (ENV_HOT, if ssaa { "sun_ssaa" } else { "sun" }.to_owned()),
            ("BOYKO_IMAGE_DUMP", names),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + 10).to_string()),
        ],
        false,
    );
    let scene_files = files_of(&dir, scene_name);
    assert_eq!(scene_files.len(), 1, "one `{scene_name}` capture expected: {scene_files:?}");
    let scene = DumpFile::load(&scene_files[0].1);
    let core = sun_core(&scene);
    let t = scene.texel_f32(core.0, core.1);
    let max = t[0].max(t[1]).max(t[2]);
    assert!(max > 1.0, "(D) {scene_name} max <= 1.0 at the sun core {core:?}: {t:?} — the store is display-referred");
    assert!(display_known, "no image-dump name `display`");
    let disp_files = files_of(&dir, "display");
    assert_eq!(disp_files.len(), 1, "one `display` capture expected: {disp_files:?}");
    let d = DumpFile::load(&disp_files[0].1);
    // Under SSAA `lit_native` and `display` share the native extent; otherwise `lit` and `display` do.
    assert_eq!((d.width, d.height), (scene.width, scene.height), "display and the scene image differ in extent");
    let px: Vec<[u8; 3]> = (0..d.height)
        .flat_map(|y| (0..d.width).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = d.rgba8(x, y);
            [p[0], p[1], p[2]]
        })
        .collect();
    match check_display_shoulder(&texels(&scene), &px, (d.width, d.height), core, runner_post_words([d.width, d.height])) {
        Ok(n) => println!("{tag}: {n} shoulder samples, display = the tonemap of {scene_name} on every one"),
        Err(e) => panic!("{tag}: {e}"),
    }
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11, the 50x sun on Deferred (spawns px1a_hot_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_sun_core_is_hdr_and_shoulders() {
    drive_sun(false);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11, the 50x sun under SSAA (spawns px1a_hot_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_sun_core_is_hdr_and_shoulders_under_ssaa() {
    drive_sun(true);
}

// ---- host checks of the verdict (not ignored) ------------------------------------------------

#[test]
fn the_shoulder_check_reds_a_hard_clip_and_a_dim_core() {
    // A falloff from radiance 16 at the core (x = 0) to 0.01: it crosses the knee.
    let (w, h) = (64u32, 4u32);
    let scene: Vec<[f32; 4]> = (0..h)
        .flat_map(|_| (0..w).map(|x| {
            let v = 16.0 * 0.86f32.powi(x as i32) + 0.01;
            [v, v * 0.95, v * 0.9, 1.0]
        }))
        .collect();
    // The falloff (its core of 16, `CLIP_GAP`, the sample floors) is calibrated under ACES Hill,
    // so this host check of the verdict names that curve (render wave PXt, `pxt/cut.md` D-G2's
    // rule): under AgX a core of 16 does not display >= 254 in blue. The device drivers judge
    // under the run's default words.
    let mut words = runner_post_words([w, h]);
    words[ubo_word("mode_words")] = TONEMAP_ACES << TONEMAP_MODE_SHIFT;
    let params = eval_params(words);
    let tex = HostTex2::new(&scene, w, h);
    let q = |c: f64| c.round().clamp(0.0, 255.0) as u8;
    let toned: Vec<[u8; 3]> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let o = post_final_eval([x, y], &params, tex);
            [q(255.0 * f64::from(o[0])), q(255.0 * f64::from(o[1])), q(255.0 * f64::from(o[2]))]
        })
        .collect();
    let n = check_display_shoulder(&scene, &toned, (w, h), (0, 0), words).expect("the tonemapped display shoulders");
    assert!(n >= SHOULDER_MIN, "{n}");
    // A hard clip instead of the tonemap: `OETF(saturate(lit))`.
    let clipped: Vec<[u8; 3]> = scene
        .iter()
        .map(|t| [0, 1, 2].map(|k| q(255.0 * f64::from(t[k].clamp(0.0, 1.0)).powf(f64::from(OETF_GAMMA_EXP)))))
        .collect();
    assert!(check_display_shoulder(&scene, &clipped, (w, h), (0, 0), words).unwrap_err().contains("does not shoulder"));
    let mut dim = toned.clone();
    dim[0] = [200; 3];
    assert!(check_display_shoulder(&scene, &dim, (w, h), (0, 0), words).unwrap_err().contains("not >= 254"));
    // A scene that never leaves the clip's plateau has no shoulder to judge.
    let flat = vec![[40.0f32, 40.0, 40.0, 1.0]; (w * h) as usize];
    let white = vec![[255u8; 3]; (w * h) as usize];
    assert!(check_display_shoulder(&flat, &white, (w, h), (0, 0), words).unwrap_err().contains("knee"));
}

// ---- 11b: every producer stores above 1.0 (the switch commit) -----------------------------------

/// The worker the 11b drivers spawn — PX0's fixture body (`BOYKO_PX0_FIXTURE=<name>:hot`).
const FIXTURE_WORKER: &str = "px1a_hot_fixture_worker";

/// **THE 11b WORKER** — one PX0 branch fixture (`BOYKO_PX0_FIXTURE=<name>[:hot]`).
#[test]
#[ignore = "gpu-windowed: the PX1a 11b fixture worker, spawned by the px1a_hot_11b drivers; run with --test-threads=1"]
fn px1a_hot_fixture_worker() {
    fixture_worker_main(FIXTURE_WORKER);
}

/// Test 11b's verdict over one hot region: some channel above 1.0, and every texel's rgb within one
/// binary16 step of `want`. `Err` names the first failure.
fn check_hot_scene(region: &[[f32; 4]], want: f32) -> Result<(), String> {
    let max = region.iter().flat_map(|t| t[..3].iter().copied()).fold(f32::MIN, f32::max);
    if max <= 1.0 {
        return Err(format!("(D) max {max} <= 1.0 over the region — the store is display-referred or clamped"));
    }
    for (i, t) in region.iter().enumerate() {
        for (k, &v) in t[..3].iter().enumerate() {
            match steps_between(Channel::F16, v, want) {
                Ok(n) if n <= 1 => {}
                Ok(n) => return Err(format!("texel {i} channel {k} = {v}: {n} binary16 steps from {want}")),
                Err(e) => return Err(format!("texel {i} channel {k} = {v}: not a finite binary16 value ({e:?})")),
            }
        }
    }
    Ok(())
}

/// Test 12's dump frame: sixteen engine frames after the injection.
const NAN_DUMP_FRAME: u32 = boyko_app::test_force::INJECT_NAN_LIT_FRAME + 16;

/// One test-12 run of `deferred_base` under TAA, with `force` as the worker's `BOYKO_TEST_FORCE`:
/// its `(lit, taa_hist)` dumps at [`NAN_DUMP_FRAME`].
fn capture_nan(force: &str) -> (DumpFile, DumpFile) {
    let tag = if force.is_empty() { "clean" } else { "inject" };
    let dir = fresh_out(&format!("px1a_nan_{tag}"));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let env = [
        (ENV_FIXTURE, "deferred_base".to_owned()),
        (ENV_AA, "taa".to_owned()),
        (ENV_TEST_FORCE, force.to_owned()),
        ("BOYKO_WINDOW_FRAMES", (NAN_DUMP_FRAME + 20).to_string()),
        ("BOYKO_IMAGE_DUMP", "lit,taa_hist".to_owned()),
        ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
        ("BOYKO_IMAGE_DUMP_SETTLE", NAN_DUMP_FRAME.to_string()),
    ];
    run_worker(FIXTURE_WORKER, &env, false);
    let one = |name: &str| {
        let files = files_of(&dir, name);
        assert_eq!(files.len(), 1, "12 ({tag}): one `{name}` capture expected: {files:?}");
        DumpFile::load(&files[0].1)
    };
    (one("lit"), one("taa_hist"))
}

/// Test 12's verdict over the injected run's and the clean run's `(lit, taa_hist)` at the same
/// frame, at least sixteen frames after the injection: every `taa_hist` texel finite (the injection
/// site included), and both images within one binary16 step of the clean run per channel. `Err`
/// names the first failure.
fn nan_verdict(inj: (&DumpFile, &DumpFile), clean: (&DumpFile, &DumpFile), inject_frame: u32) -> Result<(), String> {
    for (d, name) in [(inj.0, "lit"), (inj.1, "taa_hist"), (clean.0, "clean lit"), (clean.1, "clean taa_hist")] {
        if d.frame != inj.0.frame || d.frame < inject_frame + 16 {
            return Err(format!(
                "{name} was dumped at frame {}, not the common frame >= {} (the injection's + 16)",
                d.frame,
                inject_frame + 16
            ));
        }
    }
    let [x, y] = boyko_rhi_vulkan::present::NAN_INJECT_TEXEL;
    let hist = inj.1;
    for ty in 0..hist.height {
        for tx in 0..hist.width {
            let t = hist.texel_f32(tx, ty);
            if t.iter().any(|c| !c.is_finite()) {
                let site = if (tx, ty) == (x, y) { " (the injection site)" } else { "" };
                return Err(format!("taa_hist ({tx},{ty}){site} holds a non-finite texel {t:?}: the NaN reached the history"));
            }
        }
    }
    for ((a, b), name) in [((inj.0, clean.0), "lit"), ((inj.1, clean.1), "taa_hist")] {
        if (a.width, a.height) != (b.width, b.height) {
            return Err(format!("{name}: the two runs' extents differ"));
        }
        for ty in 0..a.height {
            for tx in 0..a.width {
                let (p, q) = (a.texel_f32(tx, ty), b.texel_f32(tx, ty));
                for c in 0..4 {
                    match steps_between(Channel::F16, p[c], q[c]) {
                        Ok(n) if n <= 1 => {}
                        other => {
                            return Err(format!(
                                "{name} ({tx},{ty}) ch{c}: injected {} vs clean {} ({other:?} binary16 steps): the \
                                 injection did not heal",
                                p[c], q[c]
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "gpu-windowed: PX1a test 12, a NaN injected into lit under TAA never reaches the history and heals (spawns px1a_hot_fixture_worker twice, sets BOYKO_TEST_FORCE itself); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_nan_injection_never_reaches_the_history() {
    let inj = capture_nan("inject_nan_lit");
    let clean = capture_nan("");
    let frame = boyko_app::test_force::INJECT_NAN_LIT_FRAME;
    match nan_verdict((&inj.0, &inj.1), (&clean.0, &clean.1), frame) {
        Ok(()) => println!("px1a test 12: no non-finite taa_hist texel at frame {}; lit and taa_hist equal the clean run", inj.0.frame),
        Err(e) => panic!("12: {e}"),
    }
}

fn synthetic_rgba16f(frame: u32, w: u32, h: u32, texel: impl Fn(u32, u32) -> [u16; 4]) -> DumpFile {
    let mut texels = Vec::new();
    for y in 0..h {
        for x in 0..w {
            texels.extend(texel(x, y).iter().flat_map(|c| c.to_le_bytes()));
        }
    }
    DumpFile { format: image_dump::VK_R16G16B16A16_SFLOAT, width: w, height: h, mip: 0, layers: 1, frame, texel_bytes: 8, texels }
}

#[test]
fn the_nan_verdict_reds_a_nan_in_the_history_an_unhealed_texel_and_a_stale_frame() {
    let one = 0x3C00u16;
    let clean = synthetic_rgba16f(56, 16, 16, |_, _| [one, one, one, one]);
    assert!(nan_verdict((&clean, &clean), (&clean, &clean), 40).is_ok(), "identical runs pass");
    let next = synthetic_rgba16f(56, 16, 16, |x, _| [if x == 3 { one + 1 } else { one }, one, one, one]);
    assert!(nan_verdict((&next, &next), (&clean, &clean), 40).is_ok(), "one binary16 step is inside");
    let [nx, ny] = boyko_rhi_vulkan::present::NAN_INJECT_TEXEL;
    let nan = synthetic_rgba16f(56, 16, 16, |x, y| if (x, y) == (nx, ny) { boyko_rhi_vulkan::present::NAN_INJECT_TEXEL_BITS } else { [one; 4] });
    let e = nan_verdict((&clean, &nan), (&clean, &clean), 40).unwrap_err();
    assert!(e.contains("the injection site") && e.contains("non-finite"), "{e}");
    let off = synthetic_rgba16f(56, 16, 16, |x, y| if (x, y) == (nx, ny) { [0, 0, 0, one] } else { [one; 4] });
    assert!(nan_verdict((&clean, &off), (&clean, &clean), 40).unwrap_err().contains("did not heal"));
    let early = synthetic_rgba16f(50, 16, 16, |_, _| [one; 4]);
    assert!(nan_verdict((&early, &early), (&early, &early), 40).unwrap_err().contains("common frame"));
}

fn drive_11b(name: &str) {
    let f = fixture(name);
    assert_ne!(f.hot, Hot::None, "{name} has no hot variant");
    let c = capture_scene(FIXTURE_WORKER, f, true);
    if let Expect::Producer(keys) = f.expect {
        for (k, v) in keys {
            let got = c.receipt.config(k);
            assert_eq!(got, *v, "{name}: PREMISE [config] {k} = {got}, the fixture needs {v}");
        }
    }
    if let Err(e) = check_hot_scene(&c.region, HOT_R) {
        panic!("{name} (11b): {e}");
    }
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, deferred_pbr surface stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_deferred_surface() {
    drive_11b("deferred_surface");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, deferred_pbr sky stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_deferred_sky() {
    drive_11b("deferred_sky");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, forward_opaque stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_forward_opaque() {
    drive_11b("forward_opaque");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, forward_opaque_froxel stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_forward_opaque_froxel() {
    drive_11b("forward_opaque_froxel");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, forward_sky stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_forward_sky() {
    drive_11b("forward_sky");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, vb_sky stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_vb_sky() {
    drive_11b("vb_sky");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, sdf_forward_march on Forward stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_sdf_forward_march_forward() {
    drive_11b("sdf_forward_march_forward");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, sdf_forward_march on VB stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_sdf_forward_march_vb() {
    drive_11b("sdf_forward_march_vb");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, vb_resolve stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_vb_resolve() {
    drive_11b("vb_resolve");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, vb_shade stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_vb_shade() {
    drive_11b("vb_shade");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, vb_shade_split stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_vb_shade_split() {
    drive_11b("vb_shade_split");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, the SSAA box stores above 1.0 into lit_native (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_ssaa_downsample() {
    drive_11b("ssaa_downsample");
}

// ---- 15: particle E1 (the switch commit) ---------------------------------------------------------

/// The 15 worker's exposure knob (`LightingConfig::exposure` for the run).
const ENV_PARTICLE_EXPOSURE: &str = "BOYKO_PX1A_PARTICLE_EXPOSURE";
/// The particle runs' window edge (the lab scene's `BOYKO_WIN`).
const PARTICLE_WIN: u32 = 256;
/// A pixel is a particle pixel when the particle-on run exceeds the particle-off run there by this
/// much luminance at exposure 1.0.
const PARTICLE_MASK_LUMA: f32 = 0.05;
/// Anti-vacuity: the fewest particle pixels the verdict needs.
///
/// **The margin (ruling 79 F4).** The C7 checkpoint measured **84** particle pixels against this
/// floor (`gl/c7/check.md` F4); this doc's former "hundreds even at 256 px" was false. 84 is the
/// lab's coverage at [`particle_scene::CAPTURE_FRAME`] scaled to this window: the lab measured 354
/// saturated pixels at its 512-px pin window, and a quarter of the area leaves ~88 before the
/// luminance mask. The count is not a sample of a distribution — the lab's clock takes exactly one
/// substep per frame and `particle_emit` seeds its RNG with the frame index (`LabClockWitness`), so
/// the same frame covers the same pixels every run. The floor is anti-vacuity only (a particle path
/// that reached nothing): every counted pixel is an independent exact-halving check, so 64 needs no
/// statistical headroom, and 1.3× of it absorbs a driver's rasterisation differences at the
/// sprites' edges. A change of the window, the capture frame or the lab's fan restates this number.
const MIN_PARTICLE_PIXELS: usize = 64;

/// **THE 15 WORKER** — the particle lab scene at `BOYKO_PX1A_PARTICLE_EXPOSURE`.
#[test]
#[ignore = "gpu-windowed: the PX1a test 15 particle worker, spawned by px1a_particle_e1_pre_exposure; run with --test-threads=1"]
fn px1a_particle_worker() {
    let Ok(e) = std::env::var(ENV_PARTICLE_EXPOSURE) else {
        eprintln!("px1a_particle_worker: {ENV_PARTICLE_EXPOSURE} is unset -- SKIPPED (spawned by its driver).");
        return;
    };
    let exposure: f32 = e.trim().parse().unwrap_or_else(|_| panic!("{ENV_PARTICLE_EXPOSURE}={e} is not an f32"));
    let mut app = particle_scene::build_app("boyko_engine px1a particle E1");
    app.world_mut().resource_mut::<boyko_render::LightingConfig>().exposure = exposure;
    app.run();
    assert!(app.world().resource::<HostFrameStats>().frames > 0, "the frame loop never ran — RED, never a skip");
}

/// One particle run's `lit` at the lab's capture frame.
fn particle_lit(tag: &str, exposure: f32, particles_on: bool) -> DumpFile {
    let dir = fresh_out(&format!("px1a_particle_{tag}"));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let mut env = vec![
        (ENV_PARTICLE_EXPOSURE, exposure.to_string()),
        ("BOYKO_RENDER_PATH", "vb".to_owned()),
        ("BOYKO_WIN", PARTICLE_WIN.to_string()),
        ("BOYKO_IMAGE_DUMP", "lit".to_owned()),
        ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
        ("BOYKO_IMAGE_DUMP_SETTLE", particle_scene::CAPTURE_FRAME.to_string()),
        ("BOYKO_WINDOW_FRAMES", (particle_scene::CAPTURE_FRAME + 10).to_string()),
    ];
    if !particles_on {
        env.push(("BOYKO_PARTICLE_MODE", "off".to_owned()));
    }
    run_worker("px1a_particle_worker", &env, false);
    let files = files_of(&dir, "lit");
    assert_eq!(files.len(), 1, "{tag}: one `lit` capture expected: {files:?}");
    DumpFile::load(&files[0].1)
}

/// Test 15's verdict: over the particle pixels (`on1` above `off1` by [`PARTICLE_MASK_LUMA`]),
/// `half` is `on1 / 2` within one binary16 step per channel. `Ok(particle pixels)` or the failure.
fn check_particle_e1(on1: &[[f32; 4]], half: &[[f32; 4]], off1: &[[f32; 4]]) -> Result<usize, String> {
    let mut n = 0;
    for (i, ((a, h), o)) in on1.iter().zip(half).zip(off1).enumerate() {
        if luma(*a) - luma(*o) <= PARTICLE_MASK_LUMA {
            continue;
        }
        n += 1;
        for (k, (&full, &got)) in a[..3].iter().zip(&h[..3]).enumerate() {
            let want = full * 0.5;
            match steps_between(Channel::F16, got, want) {
                Ok(s) if s <= 1 => {}
                Ok(s) => {
                    return Err(format!(
                        "particle pixel {i} channel {k}: lit at exposure 0.5 = {got}, half of exposure 1.0's = {want} — {s} binary16 steps apart (no pre-exposure on the particle?)"
                    ));
                }
                Err(e) => return Err(format!("particle pixel {i} channel {k}: {e:?}")),
            }
        }
    }
    if n < MIN_PARTICLE_PIXELS {
        return Err(format!("only {n} particle pixels (floor {MIN_PARTICLE_PIXELS}) — the particles did not reach lit"));
    }
    Ok(n)
}

fn texels(d: &DumpFile) -> Vec<[f32; 4]> {
    (0..d.height).flat_map(|y| (0..d.width).map(move |x| (x, y))).map(|(x, y)| d.texel_f32(x, y)).collect()
}

#[test]
#[ignore = "gpu-windowed: PX1a test 15, particle E1 at exposure 1.0 vs 0.5 (spawns px1a_particle_worker three times); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_particle_e1_pre_exposure() {
    let on1 = particle_lit("on1", 1.0, true);
    let half = particle_lit("on_half", 0.5, true);
    let off1 = particle_lit("off1", 1.0, false);
    for d in [&half, &off1] {
        assert_eq!((d.width, d.height), (on1.width, on1.height), "the three captures' extents");
    }
    match check_particle_e1(&texels(&on1), &texels(&half), &texels(&off1)) {
        Ok(n) => println!("px1a_particle_e1_pre_exposure: {n} particle pixels halve within one binary16 step"),
        Err(e) => panic!("15: {e}"),
    }
}

#[test]
fn the_11b_and_15_verdicts_red_their_failure_classes() {
    let hot = [[2.0, 2.0, 2.0, 1.0]; 4];
    check_hot_scene(&hot, 2.0).expect("an exact hot region");
    assert!(check_hot_scene(&[[1.0, 1.0, 1.0, 1.0]; 4], 2.0).unwrap_err().contains("<= 1.0"), "a clamped store");
    let mut two_steps = hot;
    two_steps[2][1] = 2.0 + 2.0 / 512.0;
    assert!(check_hot_scene(&two_steps, 2.0).unwrap_err().contains("binary16 steps"), "a texel two steps off");

    let n = MIN_PARTICLE_PIXELS;
    let on1 = vec![[1.5, 1.0, 0.5, 1.0]; n];
    let off1 = vec![[0.25, 0.25, 0.25, 1.0]; n];
    let half: Vec<[f32; 4]> = on1.iter().map(|t| [t[0] * 0.5, t[1] * 0.5, t[2] * 0.5, 1.0]).collect();
    assert_eq!(check_particle_e1(&on1, &half, &off1), Ok(n));
    // No pre-exposure on the particle: only the background halves.
    let unscaled: Vec<[f32; 4]> =
        on1.iter().zip(&off1).map(|(a, o)| [a[0] - o[0] * 0.5, a[1] - o[1] * 0.5, a[2] - o[2] * 0.5, 1.0]).collect();
    assert!(check_particle_e1(&on1, &unscaled, &off1).unwrap_err().contains("no pre-exposure"));
    assert!(check_particle_e1(&on1[..8], &half[..8], &off1[..8]).unwrap_err().contains("did not reach lit"));
}
