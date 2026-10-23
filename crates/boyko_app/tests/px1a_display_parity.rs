//! Render wave PX1a — **the display parity of the tail** (`px1a/cut.md` C10; plan H4 Tests 14, 17, 20):
//!
//! - **Test 14 — tonemapper parity.** A 256-plateau HDR ramp (the emissive grid below: plateau `k`
//!   holds `2^(-9 + 13·k/255)`, 0.002 … 16) under `Aces`, `Neutral`, `ReinhardJodie` and (render
//!   wave PXt) `AgX`: per pixel, the `display` dump equals `post_final_eval`
//!   (`boyko_shaderdsl::post`, the eDSL body the shader is generated from) of the dumped `lit` under
//!   the receipt's own `PostFinalGpu` words, within `post_eval`'s bound (one display code + E0's
//!   bound on the OETF's `pow`). The receipt's tonemap word must name the requested curve, so a
//!   runner that ignores the config cannot pass on ACES. Neutral and Reinhard-Jodie had no host
//!   mirror before PX1a. Anti-vacuity (`pxt/cut.md` D-T4): display == eval holds even when a curve's
//!   arm is missing from BOTH the shader and the eDSL (each falls back to ACES for an unknown word),
//!   so for every curve but ACES the display must also differ by more than one code from the eval
//!   under the ACES word on at least half of the plateaus — the arm ran.
//! - **Test 17 — the `*_SRGB` swapchain** (D-P1). The same grid under ACES is presented twice — on
//!   the production (UNORM) swapchain and on a forced `B8G8R8A8_SRGB` one (`TestForce` key
//!   `srgb_swapchain`, which REFUSES the boot on a surface without an sRGB format, so the forced run
//!   cannot silently measure the UNORM regime) — and the two swapchain readbacks
//!   (`BOYKO_HOST_DUMP`) agree within one code per channel. Red: the variant off (the display encoded
//!   twice, up to ≈ +73 codes on this grid) and `pow(x, 2.2)` in place of `srgb_eotf` (≈ 7 codes at
//!   0.1; this grid's plateaus 20…30 sit there) — GL-PX1a-D's mutation legs.
//! - **Test 20 — the dark-gradient banding report** (K3; recorded, NOT gated). A SkyLight-only
//!   scene (sky `0.02`, ground `0`) through a 90° camera stores a smooth vertical radiance ramp
//!   ≈ 0.003 … 0.017; along the centre column the report prints the longest run of equal 8-bit
//!   `display` codes inside 1–5 % of white (codes 3 … 13) under ACES Hill, named explicitly (its
//!   worker word is `aces`, whatever the engine default; PX6 re-aims the report) — the number the
//!   PX6 dither decision reads. Its one assertion is anti-vacuity (the window holds a ramp).
//!
//! The drivers spawn this binary's `px1a_display_worker` (`BOYKO_PX1A_DISPLAY=<grid|dark>:<aces|
//! neutral|jodie|agx>`). Test 17's driver sets `BOYKO_TEST_FORCE` on each worker itself (empty for the
//! UNORM run, `srgb_swapchain` for the sRGB run) — do NOT set it in the leg's environment. Test 20
//! prints its row: run it with `--nocapture`.
//!
//! Windowed-test conventions: the drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use std::path::{Path, PathBuf};

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::light::{LightingConfig, TONEMAP_MODE_MASK, TONEMAP_MODE_SHIFT, Tonemapper};
use boyko_render::{AaConfig, AaMode, GeometryLegs, Material, RenderPath, RenderPathConfig};
use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::post::{OETF_GAMMA_EXP, eval_params, post_final_eval};
use boyko_shaderdsl::prims_probe::vk_pow_bound;

use graph_receipt::{Receipt, fresh_out, run_worker};
use image_dump::{DumpFile, files_of};

/// The worker's knob: `<grid|dark>:<aces|neutral|jodie>`.
const ENV_SCENE: &str = "BOYKO_PX1A_DISPLAY";
/// The fixture convention's force-key variable (decoded by the worker, `boyko_app::test_force`).
const ENV_TEST_FORCE: &str = "BOYKO_TEST_FORCE";
/// The window edge.
const WIN: u32 = 256;
/// Frames before the receipt / the dumps (a static scene; no TAA).
const SETTLE: u32 = 30;
/// The grid edge: `GRID²` = 256 plateaus.
const GRID: u32 = 16;
/// The ramp's ends, `log2` of the first and last plateau's radiance.
const RAMP_LOG2: (f32, f32) = (-9.0, 4.0);
/// The camera's distance to the grid plane.
const CAM_Z: f32 = 4.0;
/// Test 20's sky radiance (ground 0): the ramp `0.02 · t`, `t ∈ [0.146, 0.854]` at fov 90°.
const DARK_SKY: f32 = 0.02;
/// Test 20's window, in 8-bit codes: 1 % … 5 % of white.
const DARK_CODES: (u8, u8) = (3, 13);
/// Test 14's anti-vacuity: the fewest distinct `lit` radiances the grid must show.
const PLATEAUS_MIN: usize = 240;
/// Test 17's anti-vacuity: pixels around display 0.1 (codes 20 … 30, where `pow(x, 2.2)` misses the
/// sRGB decode by ≈ 7 codes) and in the mid-tones (codes 100 … 160, where a double encode adds
/// 40+ codes) the comparison must cover.
const SRGB_COVER_MIN: usize = 500;

/// The scene a worker renders.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scene {
    /// Test 14 / 17: the 256-plateau emissive grid (no light, no sky).
    Grid,
    /// Test 20: the SkyLight-only dark ramp.
    Dark,
}

#[derive(Resource, Clone, Copy)]
struct SceneRes(Scene, Tonemapper);

/// Plateau `k`'s radiance: `2^(lo + (hi - lo)·k/255)`.
fn plateau(k: u32) -> f32 {
    let (lo, hi) = RAMP_LOG2;
    (lo + (hi - lo) * k as f32 / (GRID * GRID - 1) as f32).exp2()
}

fn parse_tonemapper(word: &str) -> Tonemapper {
    match word {
        "aces" => Tonemapper::Aces,
        "neutral" => Tonemapper::Neutral,
        "jodie" => Tonemapper::ReinhardJodie,
        "agx" => Tonemapper::AgX,
        other => panic!("{ENV_SCENE}: `{other}` is not aces|neutral|jodie|agx"),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn display_setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    arming: Res<SceneRes>,
) {
    let SceneRes(scene, _) = *arming;
    let fov_y = match scene {
        Scene::Grid => core::f32::consts::FRAC_PI_3,
        Scene::Dark => core::f32::consts::FRAC_PI_2,
    };
    match scene {
        Scene::Grid => {
            // The grid fills the view at the plane z = 0: pitch = the visible height / GRID.
            let pitch = 2.0 * CAM_Z * (fov_y * 0.5).tan() / GRID as f32;
            let cube = meshes.cube(dev.get(), 0.7 * pitch);
            for k in 0..GRID * GRID {
                let e = plateau(k);
                let mat = materials.add(Material::new([0.0, 0.0, 0.0, 1.0], 0.0, 0.6, 0.5, [e; 3], 0));
                let row = u16::try_from(mat.index()).expect("invariant: the grid mints < 65536 rows");
                let (i, j) = ((k % GRID) as f32, (k / GRID) as f32);
                let at = Vec3::new((i + 0.5) * pitch - CAM_Z * (fov_y * 0.5).tan(), CAM_Z * (fov_y * 0.5).tan() - (j + 0.5) * pitch, 0.0);
                let pose = Transform { translation: at, rotation: Quat::IDENTITY, scale: Vec3::ONE };
                commands.spawn(MeshBundle { material: MaterialHandle(row), ..MeshBundle::new(cube, pose) });
            }
        }
        Scene::Dark => {
            commands.spawn(SkyLight::new([DARK_SKY; 3], [0.0; 3]));
        }
    }
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 0.0, CAM_Z), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y, aspect: 1.0, near: 0.1, far: 100.0 },
    });
}

/// The requested curve on the plugin-inserted `LightingConfig` (every other field at its default).
#[allow(clippy::needless_pass_by_value)]
fn apply_tonemapper(arming: Res<SceneRes>, mut cfg: ResMut<LightingConfig>) {
    cfg.tonemapper = arming.1;
}

/// **THE WORKER** — one scene under one curve (`BOYKO_PX1A_DISPLAY=<grid|dark>:<curve>`), Deferred,
/// no AA; `BOYKO_TEST_FORCE` decoded here (an unknown key refuses the boot, the fixture convention).
#[test]
#[ignore = "gpu-windowed: PX1a display-parity worker, spawned by the tests 14 / 17 / 20 drivers with BOYKO_PX1A_DISPLAY; run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_display_worker() {
    let Ok(spec) = std::env::var(ENV_SCENE) else {
        eprintln!("px1a_display_worker: {ENV_SCENE} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    let (scene, curve) = spec.split_once(':').unwrap_or_else(|| panic!("{ENV_SCENE}={spec} is not <scene>:<curve>"));
    let scene = match scene {
        "grid" => Scene::Grid,
        "dark" => Scene::Dark,
        other => panic!("{ENV_SCENE}: `{other}` is not grid|dark"),
    };
    let force = std::env::var(ENV_TEST_FORCE).unwrap_or_default();
    let force = boyko_app::test_force::parse(&force).unwrap_or_else(|e| panic!("{e}"));
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine px1a display parity", WIN, WIN));
    app.insert_resource(SceneRes(scene, parse_tonemapper(curve)));
    app.insert_resource(force);
    app.add_startup_system(display_setup);
    app.add_startup_system(apply_tonemapper);
    app.insert_resource(RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Mesh });
    app.insert_resource(AaConfig { mode: AaMode::Off });
    app.run();
    assert!(app.world().resource::<HostFrameStats>().frames > 0, "the frame loop never ran — RED, never a skip");
}

// ===============================================================================================
// The drivers' helpers
// ===============================================================================================

fn out_dir(tag: &str) -> PathBuf {
    let dir = fresh_out(tag);
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    dir
}

/// One `lit,display` capture of `scene` under `curve`, with the receipt.
fn capture_dumps(scene: &str, curve: &str) -> (DumpFile, DumpFile, Receipt) {
    let dir = out_dir(&format!("px1a_display_{scene}_{curve}"));
    let receipt = dir.join("receipt.toml");
    let env = [
        (ENV_SCENE, format!("{scene}:{curve}")),
        (ENV_TEST_FORCE, String::new()),
        ("BOYKO_GRAPH_RECEIPT", receipt.display().to_string()),
        ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
        ("BOYKO_WINDOW_FRAMES", (SETTLE + 20).to_string()),
        ("BOYKO_IMAGE_DUMP", "lit,display".to_owned()),
        ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
        ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
    ];
    run_worker("px1a_display_worker", &env, false);
    let one = |name: &str| {
        let files = files_of(&dir, name);
        assert_eq!(files.len(), 1, "{scene}:{curve}: one `{name}` capture expected: {files:?}");
        DumpFile::load(&files[0].1)
    };
    (one("lit"), one("display"), Receipt::load(&receipt))
}

/// One swapchain readback (`BOYKO_HOST_DUMP`) of the grid under ACES, with `force` as the worker's
/// `BOYKO_TEST_FORCE`.
fn capture_swapchain(tag: &str, force: &str) -> Vec<[u8; 3]> {
    let dir = out_dir(&format!("px1a_display_swap_{tag}"));
    let bmp = dir.join("host.bmp");
    let env = [
        (ENV_SCENE, "grid:aces".to_owned()),
        (ENV_TEST_FORCE, force.to_owned()),
        ("BOYKO_HOST_DUMP", bmp.display().to_string()),
        ("BOYKO_HOST_DUMP_SETTLE", SETTLE.to_string()),
        ("BOYKO_WINDOW_FRAMES", (SETTLE + 20).to_string()),
    ];
    run_worker("px1a_display_worker", &env, false);
    read_bmp_rgb(&bmp)
}

/// A 32-bpp bottom-up BMP's RGB, row-major top-down (the host dump's format).
fn read_bmp_rgb(path: &Path) -> Vec<[u8; 3]> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("no host dump at {} ({e})", path.display()));
    let le32 = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"));
    let (off, w) = (le32(10) as usize, le32(18) as usize);
    let h = (le32(22) as i32).unsigned_abs() as usize;
    assert_eq!((w, h), (WIN as usize, WIN as usize), "{}: the readback is not the window's extent", path.display());
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = off + ((h - 1 - y) * w + x) * 4;
            [b[p + 2], b[p + 1], b[p]]
        })
        .collect()
}

/// `post_eval`'s per-channel bound (`post_eval.rs` `channel_bound`, no filter row): one display code
/// (the two 8-bit roundings) + E0's bound on the OETF's `pow` at the body value `o`.
fn eval_bound_lsb(o: f32) -> f64 {
    let g = f64::from(OETF_GAMMA_EXP);
    let x = f64::from(o.max(0.0)).powf(1.0 / g);
    let oetf = if x <= 0.0 { 0.0 } else { 255.0 * vk_pow_bound(x as f32, OETF_GAMMA_EXP, x.log2(), x.powf(g)) };
    1.0 + oetf
}

/// Test 14's comparison: `display` against `post_final_eval` of `scene` under `words`, every pixel.
/// Returns the number of channel samples outside [`eval_bound_lsb`] and the first few, described.
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
                        first.push(format!("({x},{y}) ch{c}: display {} vs eval {:.3} (lit {:?})", d[c], 255.0 * o[c], texels[(y * scene.width + x) as usize]));
                    }
                }
            }
        }
    }
    (bad, first)
}

/// Render wave PXt (`pxt/cut.md` D-T4) — Test 14's anti-vacuity for a non-ACES curve: `(plateaus whose
/// display differs by more than one code from post_final_eval(lit) under `words` with the ACES
/// tonemap word, plateaus)`, a plateau being one distinct positive `lit` radiance (red channel).
fn plateaus_off_aces(scene: &DumpFile, display: &DumpFile, words: [u32; 8]) -> (usize, usize) {
    let field = TONEMAP_MODE_MASK << TONEMAP_MODE_SHIFT;
    let mut aces = words;
    aces[0] = (aces[0] & !field) | ((Tonemapper::Aces as u32) << TONEMAP_MODE_SHIFT);
    let texels: Vec<[f32; 4]> =
        (0..scene.height).flat_map(|y| (0..scene.width).map(move |x| (x, y))).map(|(x, y)| scene.texel_f32(x, y)).collect();
    let params = eval_params(aces);
    let tex = HostTex2::new(&texels, scene.width, scene.height);
    let mut moved: Vec<(u32, bool)> = Vec::new();
    for y in 0..scene.height {
        for x in 0..scene.width {
            let r = texels[(y * scene.width + x) as usize][0];
            if r <= 0.0 {
                continue;
            }
            let o = post_final_eval([x, y], &params, tex);
            let d = display.rgba8(x, y);
            let off = (0..3).any(|c| (255.0 * f64::from(o[c]) - f64::from(d[c])).abs() > 1.0);
            moved.push((r.to_bits(), off));
        }
    }
    moved.sort_unstable();
    let mut plateaus: Vec<(u32, bool)> = Vec::new();
    for (bits, off) in moved {
        match plateaus.last_mut() {
            Some(last) if last.0 == bits => last.1 |= off,
            _ => plateaus.push((bits, off)),
        }
    }
    (plateaus.iter().filter(|p| p.1).count(), plateaus.len())
}

/// The receipt's `[config]` `post_word0..7` — the `PostFinalGpu` words the frame's `post_final` read.
fn receipt_post_words(r: &Receipt) -> [u32; 8] {
    core::array::from_fn(|i| u32::try_from(r.config(&format!("post_word{i}"))).expect("invariant: a 32-bit post word"))
}

/// Test 14's anti-vacuity over the dumped `lit`: the distinct red radiances, the smallest positive one
/// and the largest.
fn ramp_cover(lit: &DumpFile) -> (usize, f32, f32) {
    let mut vals: Vec<u32> = Vec::new();
    let (mut lo, mut hi) = (f32::INFINITY, 0.0f32);
    for y in 0..lit.height {
        for x in 0..lit.width {
            let r = lit.texel_f32(x, y)[0];
            if r > 0.0 {
                lo = lo.min(r);
            }
            hi = hi.max(r);
            vals.push(r.to_bits());
        }
    }
    vals.sort_unstable();
    vals.dedup();
    (vals.len(), lo, hi)
}

/// Test 17's verdict over the two readbacks: `Ok(max |Δ|)` iff every channel agrees within one code
/// and the UNORM image covers both the dark (codes 20 … 30) and the mid-tone (100 … 160) bands.
fn srgb_parity_verdict(unorm: &[[u8; 3]], srgb: &[[u8; 3]]) -> Result<u8, String> {
    if unorm.len() != srgb.len() {
        return Err(format!("the readbacks differ in size: {} vs {}", unorm.len(), srgb.len()));
    }
    let band = |lo: u8, hi: u8| unorm.iter().filter(|p| (lo..=hi).contains(&p[1])).count();
    let (dark, mid) = (band(20, 30), band(100, 160));
    if dark < SRGB_COVER_MIN || mid < SRGB_COVER_MIN {
        return Err(format!(
            "the grid covers {dark} pixels at display 20..30 and {mid} at 100..160 (need {SRGB_COVER_MIN} each): \
             the comparison cannot see a gamma-2.2 decode or a double encode"
        ));
    }
    let mut worst = (0u8, 0usize, 0usize);
    for (i, (a, b)) in unorm.iter().zip(srgb).enumerate() {
        for c in 0..3 {
            let d = a[c].abs_diff(b[c]);
            if d > worst.0 {
                worst = (d, i, c);
            }
        }
    }
    let (d, i, c) = worst;
    if d > 1 {
        return Err(format!(
            "pixel ({}, {}) ch{c}: UNORM {} vs sRGB {} ({d} codes): the sRGB swapchain does not reproduce the display \
             (a double encode, or a decode that is not srgb_eotf)",
            i % WIN as usize,
            i / WIN as usize,
            unorm[i][c],
            srgb[i][c]
        ));
    }
    Ok(d)
}

/// Test 20's report over one display column (top to bottom): the longest run of equal codes among the
/// pixels inside [`DARK_CODES`], the number of distinct codes there, and the pixels counted.
fn banding_report(column: &[u8]) -> (usize, usize, usize) {
    let (lo, hi) = DARK_CODES;
    let inside: Vec<u8> = column.iter().copied().filter(|c| (lo..=hi).contains(c)).collect();
    let mut longest = 0usize;
    let mut run = 0usize;
    let mut prev = None;
    for &c in &inside {
        run = if prev == Some(c) { run + 1 } else { 1 };
        longest = longest.max(run);
        prev = Some(c);
    }
    let mut distinct = inside.clone();
    distinct.sort_unstable();
    distinct.dedup();
    (longest, distinct.len(), inside.len())
}

// ===============================================================================================
// The drivers
// ===============================================================================================

#[test]
#[ignore = "gpu-windowed: PX1a test 14, display == post_final_eval(lit) under Aces / Neutral / ReinhardJodie / AgX on a 256-plateau HDR ramp, each non-ACES curve off the ACES display (spawns px1a_display_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_display_tonemapper_parity() {
    let curves = [
        ("aces", Tonemapper::Aces),
        ("neutral", Tonemapper::Neutral),
        ("jodie", Tonemapper::ReinhardJodie),
        ("agx", Tonemapper::AgX),
    ];
    for (curve, tm) in curves {
        let (lit, display, receipt) = capture_dumps("grid", curve);
        let words = receipt_post_words(&receipt);
        let mode = (words[0] >> TONEMAP_MODE_SHIFT) & TONEMAP_MODE_MASK;
        assert_eq!(mode, tm as u32, "{curve}: the receipt's post_word0 names tonemap mode {mode}, not the requested {tm:?}");
        let (plateaus, lo, hi) = ramp_cover(&lit);
        assert!(
            plateaus >= PLATEAUS_MIN && lo < 0.004 && hi > 8.0,
            "{curve}: lit holds {plateaus} distinct radiances over [{lo}, {hi}] — not the 256-plateau HDR ramp"
        );
        let (bad, first) = eval_post_mismatches(&lit, &display, words);
        assert!(
            bad == 0,
            "{curve}: {bad} display channel samples differ from post_final_eval(lit) beyond one code + the OETF bound \
             (first: {first:?}): the device's post_final is not the eDSL's {tm:?} body"
        );
        if tm != Tonemapper::Aces {
            let (off, total) = plateaus_off_aces(&lit, &display, words);
            assert!(
                2 * off >= total,
                "{curve}: the display equals the ACES display on {} of {total} plateaus — the {tm:?} arm did not run",
                total - off
            );
            println!("px1a test 14: {curve}: off the ACES display on {off} of {total} plateaus");
        }
        println!("px1a test 14: {curve}: {plateaus} radiances in [{lo:.5}, {hi:.3}], display == post_final_eval(lit)");
    }
}

#[test]
#[ignore = "gpu-windowed: PX1a test 17, the forced B8G8R8A8_SRGB swapchain reproduces the UNORM readback within one code (spawns px1a_display_worker twice, sets BOYKO_TEST_FORCE itself); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_display_srgb_swapchain_matches_unorm() {
    let unorm = capture_swapchain("unorm", "");
    let srgb = capture_swapchain("srgb", "srgb_swapchain");
    match srgb_parity_verdict(&unorm, &srgb) {
        Ok(d) => println!("px1a test 17: the sRGB swapchain readback equals the UNORM one within {d} code(s)"),
        Err(e) => panic!("{e}"),
    }
}

#[test]
#[ignore = "gpu-windowed: PX1a test 20, the dark-gradient banding report (recorded, not gated; spawns px1a_display_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1 --nocapture"]
fn px1a_display_dark_gradient_banding_report() {
    let (lit, display, _) = capture_dumps("dark", "aces");
    let x = WIN / 2;
    let column: Vec<u8> = (0..display.height).map(|y| display.rgba8(x, y)[1]).collect();
    let (longest, distinct, pixels) = banding_report(&column);
    let (top, bottom) = (lit.texel_f32(x, 0)[1], lit.texel_f32(x, WIN - 1)[1]);
    println!(
        "px1a test 20 (K3 report, not gated): column x={x}, lit {top:.5} (top) .. {bottom:.5} (bottom); \
         {pixels} px at display codes {}..{} hold {distinct} distinct codes; longest equal-code run {longest} px",
        DARK_CODES.0, DARK_CODES.1
    );
    assert!(distinct >= 5, "test 20's window holds {distinct} distinct codes over {pixels} px — not a dark ramp, nothing to report");
}

// ===============================================================================================
// Host checks of the verdicts
// ===============================================================================================

fn synthetic_grid(shift: impl Fn(u8) -> u8) -> Vec<[u8; 3]> {
    (0..WIN * WIN).map(|i| (i % 256) as u8).map(|g| [shift(g), shift(g), shift(g)]).collect()
}

/// sRGB encode of a linear channel (the hardware's `*_SRGB` write).
fn srgb_encode(c: f64) -> f64 {
    if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

#[test]
fn the_srgb_parity_verdict_separates_the_decode_from_a_double_encode_and_a_gamma_decode() {
    let q = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let unorm = synthetic_grid(|g| g);
    assert_eq!(srgb_parity_verdict(&unorm, &unorm.clone()), Ok(0), "the exact decode-then-encode");
    let off_by_one = synthetic_grid(|g| g.saturating_add(u8::from(g % 7 == 0)));
    assert_eq!(srgb_parity_verdict(&unorm, &off_by_one), Ok(1), "one code is inside");
    let double = synthetic_grid(|g| q(srgb_encode(f64::from(g) / 255.0)));
    let e = srgb_parity_verdict(&unorm, &double).unwrap_err();
    assert!(e.contains("does not reproduce"), "variant off: {e}");
    let gamma = synthetic_grid(|g| q(srgb_encode((f64::from(g) / 255.0).powf(2.2))));
    let e = srgb_parity_verdict(&unorm, &gamma).unwrap_err();
    assert!(e.contains("does not reproduce"), "pow 2.2 decode: {e}");
    let flat = vec![[200u8; 3]; (WIN * WIN) as usize];
    assert!(srgb_parity_verdict(&flat, &flat).unwrap_err().contains("cannot see"), "a flat image is vacuous");
}

#[test]
fn the_banding_report_counts_runs_inside_the_dark_window_only() {
    let column = [0u8, 0, 2, 3, 3, 3, 4, 4, 5, 5, 5, 5, 13, 14, 14];
    assert_eq!(banding_report(&column), (4, 4, 10));
    assert_eq!(banding_report(&[200u8; 8]), (0, 0, 0));
}

#[test]
fn the_ramp_spans_the_curve_and_every_plateau_is_distinct() {
    assert!((plateau(0) - 2.0f32.powi(-9)).abs() < 1e-9);
    assert!((plateau(GRID * GRID - 1) - 16.0).abs() < 1e-4);
    assert!((1..GRID * GRID).all(|k| plateau(k) > plateau(k - 1)));
}
