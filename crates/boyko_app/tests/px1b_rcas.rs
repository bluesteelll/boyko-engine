//! Render wave PX1b (`plan/H5-PX1b-rcas.md` Tests Device (a), (b); `px1b/cut.md` D-T2, PC-PX1b-4):
//! **`SharpenMode::Rcas` runs FSR 1 RCAS inside `post_final`, on the frame.**
//!
//! A static TAA scene: two emissive bars on a dark floor — one at radiance 4.0 (an HDR edge: the
//! reversible space is what keeps it sharpenable) and one at 0.6 (an LDR edge) — with
//! `TaaConfig::rcas_sharpness = 1.0` (FSR 1's maximum, `con = 1`) so the effect sits far above the
//! 8-bit rounding floor. [`SETTLE`] frames, then a [`BURST`]-frame (D) burst of `taa_hist` and
//! `display`, and the graph receipt of the settle frame.
//!
//! - **(a) the device executed the traced body under sharpen** — Deferred and VisibilityBuffer:
//!   per burst frame `display == post_final_eval(taa_hist, the receipt's post_word0..7)` within
//!   `post_eval`'s bound (one display code + E0's bound on the OETF's `pow`, [`eval_bound_lsb`]).
//!   Premises: `post_word0` carries `POST_MODE_SHARPEN` and `post_word4` is `con`'s bits.
//!   Anti-vacuity: the SAME words with the bit cleared reject `display` on ≥ [`ANTI_VACUITY_SHARE`]
//!   of the edge pixels ([`edge_pixels`]: a `taa_hist` cross whose reversible-space contrast exceeds
//!   [`EDGE_CONTRAST`]) — so the agreement is not a sharpen-off frame agreeing with itself.
//!   **What this credits (PC-PX1b-4, OPEN-ITEMS 4 I2(c)):** the printer, the bindings, the five
//!   fetches and the border clamp on the device, the dumps and the device arithmetic. "Wrong space,
//!   missing inverse, wrong tap" are the host image oracle's to catch
//!   (`boyko_shaderdsl/tests/post_image_oracle.rs`'s six mis-composition references): this oracle
//!   evaluates the same traced body, so it cannot see a composition error it shares.
//! - **(b) acutance (Deferred)** — the mean AND the median per-row 10–90 % width across the HDR edge
//!   (and, separately, the LDR edge) are strictly smaller with sharpen on than off. Premises: the
//!   two runs' `taa_hist` bursts are byte-equal (RCAS never feeds the history) and at least
//!   [`ROWS_MIN`] sharpen-off rows of each edge are a ramp, at least [`RAMP_MIN_PX`] wide (there is
//!   an edge to sharpen). Not every row: the bars' edges step across the pixel grid, and a row the
//!   converged history resolves to a one-pixel step measures 0.8 px — the 10–90 % width of a hard
//!   step, which RCAS can only overshoot (at PX1b's C5: 29 of the HDR edge's 56 rows, 13 of the LDR
//!   edge's 56, the first at 0.873 px). The two statistics still span every row.
//!
//! The bound is spelled here as `px1a_taa_accumulation.rs` spells it (the PX1a eval-post pin's
//! precedent: `post_eval.rs` `channel_bound` with no filter row, the scene input being a `Load`);
//! [`the_eval_bound_is_one_code_plus_the_oetf_term`] pins that spelling to `post_eval`'s floor
//! shape (critique Q2). GL-H2 runs the `post_eval` binary itself (`golden_diff --eval-post`).
//!
//! Windowed-test conventions: the drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::Handle;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{
    AaConfig, AaMode, GeometryLegs, Material, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig, SharpenMode,
    TaaConfig,
};
use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::post::{OETF_GAMMA_EXP, POST_FINAL_RECEIPT_KEYS, POST_MODE_SHARPEN, eval_params, post_final_eval, ubo_word};
use boyko_shaderdsl::prims_probe::vk_pow_bound;

use graph_receipt::{Receipt, fresh_out, run_worker};
use image_dump::{DumpFile, files_of};

/// The worker's render path (`deferred` | `vb`).
const ENV_PATH: &str = "BOYKO_PX1B_PATH";
/// The worker's sharpen arm (`on` | `off`).
const ENV_SHARPEN: &str = "BOYKO_PX1B_SHARPEN";
/// The window edge.
const WIN: u32 = 256;
/// Frames before the burst (the history converges well before).
const SETTLE: u32 = 64;
/// The burst length (two periods of the 8-phase Halton jitter).
const BURST: u32 = 16;
/// `TaaConfig::rcas_sharpness` — FSR 1's maximum strength (`con = 1`).
const SHARPNESS: f32 = 1.0;
/// An edge pixel: some ring tap of its `taa_hist` cross differs from the centre by more than this
/// in reversible space (`y = c / (1 + max3(c))`, so the bars' 4.0 and 0.6 against the floor are
/// edges and the floor's own lighting gradient is not).
const EDGE_CONTRAST: f64 = 0.05;
/// Anti-vacuity: the bit-cleared words must reject `display` on at least this share of the edge
/// pixels (the PXe image oracle's class threshold).
const ANTI_VACUITY_SHARE: f64 = 0.30;
/// Anti-vacuity: the fewest edge pixels a frame may have (the two bars' four vertical edges).
const EDGE_MIN: usize = 100;
/// (b): the fewest rows a width statistic may average.
const ROWS_MIN: usize = 10;
/// (b): an edge row's plateaus must differ by at least this many display codes.
const STEP_MIN: f64 = 40.0;
/// (b): a sharpen-off row at least this wide is a ramp — its edge spans more than one pixel, so RCAS
/// has a slope to steepen. A row resolved to a one-pixel step measures 0.8 px (both crossings fall in
/// its one pixel interval), and RCAS narrows it only by overshooting it.
const RAMP_MIN_PX: f64 = 1.0;

/// One worker configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Arm {
    path: RenderPath,
    sharpen: bool,
}

impl Arm {
    fn path_word(self) -> &'static str {
        match self.path {
            RenderPath::VisibilityBuffer => "vb",
            _ => "deferred",
        }
    }

    fn tag(self) -> String {
        format!("px1b_rcas_{}_{}", self.path_word(), if self.sharpen { "on" } else { "off" })
    }
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

/// The scene: a dark floor, the HDR bar (emissive 4.0) on the right half of the frame and the LDR
/// bar (emissive 0.6) on the left, each wide enough for a plateau either side of its left edge.
/// The meshes go through the VB geometry table when the boot made one (a VisibilityBuffer boot
/// re-fetches every pixel's triangle from it, so a mesh must own a slot), else the plain path.
#[allow(clippy::needless_pass_by_value)]
fn rcas_setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
) {
    let floor_mat = materials.add(Material::new([0.05, 0.05, 0.06, 1.0], 0.0, 0.8, 0.5, [0.0; 3], 0));
    let hdr_mat = materials.add(Material::new([0.0, 0.0, 0.0, 1.0], 0.0, 0.5, 0.5, [4.0, 4.0, 4.0], 0));
    let ldr_mat = materials.add(Material::new([0.0, 0.0, 0.0, 1.0], 0.0, 0.5, 0.5, [0.6, 0.6, 0.6], 0));
    let row = |h: Handle<Material>| MaterialHandle(u16::try_from(h.index()).expect("invariant: < 65536 rows"));
    let (floor, cube) = match geo_table.0.as_mut() {
        Some(t) => (meshes.plane_vb(dev.get(), 12.0, t), meshes.cube_vb(dev.get(), 1.0, t)),
        None => (meshes.plane(dev.get(), 12.0), meshes.cube(dev.get(), 1.0)),
    };
    commands.spawn(MeshBundle { material: row(floor_mat), ..MeshBundle::new(floor, Transform::IDENTITY) });
    for (x, mat) in [(0.9f32, hdr_mat), (-0.9, ldr_mat)] {
        let bar = Transform { translation: Vec3::new(x, 0.6, 0.2), scale: Vec3::new(0.6, 1.2, 0.25), ..Transform::IDENTITY };
        commands.spawn(MeshBundle { material: row(mat), ..MeshBundle::new(cube, bar) });
    }
    let dir = [-0.45f32, 0.82, 0.36];
    let sun_pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(dir[0], dir[1], dir[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(sun_pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(dir, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(camera_rig(Vec3::new(0.0, 1.6, 5.0)));
}

/// **THE WORKER** — `BOYKO_PX1B_PATH=deferred|vb`, `BOYKO_PX1B_SHARPEN=on|off`.
#[test]
#[ignore = "gpu-windowed: the PX1b RCAS worker, spawned by the px1b_rcas drivers; run with --test-threads=1"]
fn px1b_rcas_worker() {
    let Ok(path) = std::env::var(ENV_PATH) else {
        eprintln!("px1b_rcas_worker: {ENV_PATH} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    let path = match path.trim() {
        "deferred" => RenderPath::Deferred,
        "vb" => RenderPath::VisibilityBuffer,
        other => panic!("{ENV_PATH}={other} is not deferred|vb"),
    };
    let sharpen = match std::env::var(ENV_SHARPEN).as_deref().map(str::trim) {
        Ok("on") => SharpenMode::Rcas,
        Ok("off") => SharpenMode::None,
        other => panic!("{ENV_SHARPEN}={other:?} is not on|off"),
    };
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine px1b rcas", WIN, WIN));
    app.add_startup_system(rcas_setup);
    app.insert_resource(RenderPathConfig { path, legs: GeometryLegs::Mesh });
    app.insert_resource(AaConfig { mode: AaMode::Taa });
    app.insert_resource(TaaConfig { sharpen, rcas_sharpness: SHARPNESS, ..TaaConfig::default() });
    app.run();
    assert!(app.world().resource::<HostFrameStats>().frames > 0, "the frame loop never ran — RED, never a skip");
}

/// One worker run: its burst of `taa_hist` and `display`, frame for frame, and the settle frame's
/// receipt words.
struct Run {
    hist: Vec<(u32, DumpFile)>,
    display: Vec<(u32, DumpFile)>,
    words: [u32; 8],
    rcas_key: u64,
}

fn run_arm(arm: Arm) -> Run {
    let dir = fresh_out(&arm.tag());
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let receipt = dir.join("receipt.toml");
    run_worker(
        "px1b_rcas_worker",
        &[
            (ENV_PATH, arm.path_word().to_owned()),
            (ENV_SHARPEN, if arm.sharpen { "on" } else { "off" }.to_owned()),
            ("BOYKO_IMAGE_DUMP", "taa_hist,display".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_IMAGE_DUMP_FRAMES", BURST.to_string()),
            ("BOYKO_GRAPH_RECEIPT", receipt.display().to_string()),
            ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + BURST + 10).to_string()),
        ],
        false,
    );
    let load = |name: &str| -> Vec<(u32, DumpFile)> {
        let files: Vec<(u32, DumpFile)> = files_of(&dir, name).into_iter().map(|(f, p)| (f, DumpFile::load(&p))).collect();
        assert_eq!(files.len(), BURST as usize, "{}: a {BURST}-frame burst of `{name}` in {}", arm.tag(), dir.display());
        files
    };
    let r = Receipt::load(&receipt);
    let words: [u32; 8] = core::array::from_fn(|i| {
        u32::try_from(r.config(POST_FINAL_RECEIPT_KEYS[i])).expect("invariant: a post word is a u32")
    });
    Run { hist: load("taa_hist"), display: load("display"), words, rcas_key: r.config("rcas") }
}

/// `post_eval`'s per-channel bound (`post_eval.rs` `channel_bound`, no filter row): one display code
/// (the two 8-bit roundings) + E0's bound on the OETF's `pow` at the body value `o` — the PX1a
/// eval-post pin's spelling (`px1a_taa_accumulation.rs`).
fn eval_bound_lsb(o: f32) -> f64 {
    let g = f64::from(OETF_GAMMA_EXP);
    let x = f64::from(o.max(0.0)).powf(1.0 / g);
    let oetf = if x <= 0.0 { 0.0 } else { 255.0 * vk_pow_bound(x as f32, OETF_GAMMA_EXP, x.log2(), x.powf(g)) };
    1.0 + oetf
}

/// The texels of a dump, row-major.
fn texels_of(d: &DumpFile) -> Vec<[f32; 4]> {
    (0..d.height).flat_map(|y| (0..d.width).map(move |x| (x, y))).map(|(x, y)| d.texel_f32(x, y)).collect()
}

/// The pixels whose `taa_hist` cross (FSR 1's taps, clamped to the edge) has a ring tap more than
/// [`EDGE_CONTRAST`] from the centre in reversible space.
fn edge_pixels(hist: &DumpFile) -> Vec<(u32, u32)> {
    let rev = |t: [f32; 4]| {
        let c = [t[0], t[1], t[2]].map(|v| if v.is_finite() { f64::from(v).max(0.0) } else { 0.0 });
        let m = c[0].max(c[1].max(c[2]));
        c.map(|v| v / (1.0 + m))
    };
    let (w, h) = (hist.width, hist.height);
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let e = rev(hist.texel_f32(x, y));
            let ring = [(x, y.saturating_sub(1)), (x.saturating_sub(1), y), ((x + 1).min(w - 1), y), (x, (y + 1).min(h - 1))];
            ring.iter().any(|&(tx, ty)| {
                let t = rev(hist.texel_f32(tx, ty));
                (0..3).any(|c| (t[c] - e[c]).abs() > EDGE_CONTRAST)
            })
        })
        .collect()
}

/// The pixels of `pick` (every pixel when `None`) where `display` differs from `post_final_eval` of
/// `hist` under `words` beyond [`eval_bound_lsb`] on some channel, and the first few, described.
fn eval_post_rejects(hist: &DumpFile, display: &DumpFile, words: [u32; 8], pick: Option<&[(u32, u32)]>) -> (usize, Vec<String>) {
    assert_eq!((hist.width, hist.height), (display.width, display.height), "display is the scene input's extent");
    let texels = texels_of(hist);
    let params = eval_params(words);
    let tex = HostTex2::new(&texels, hist.width, hist.height);
    let all: Vec<(u32, u32)>;
    let pixels = match pick {
        Some(p) => p,
        None => {
            all = (0..hist.height).flat_map(|y| (0..hist.width).map(move |x| (x, y))).collect();
            &all
        }
    };
    let (mut bad, mut first) = (0usize, Vec::new());
    for &(x, y) in pixels {
        let o = post_final_eval([x, y], &params, tex);
        let d = display.rgba8(x, y);
        let over = (0..3).find(|&c| (255.0 * f64::from(o[c]) - f64::from(d[c])).abs() > eval_bound_lsb(o[c]));
        if let Some(c) = over {
            bad += 1;
            if first.len() < 4 {
                first.push(format!("({x},{y}) ch{c}: display {} vs eval {:.3}", d[c], 255.0 * o[c]));
            }
        }
    }
    (bad, first)
}

/// (a) on one run: the premises, the agreement on every burst frame, the anti-vacuity.
fn check_eval_post_under_sharpen(tag: &str, run: &Run) -> Result<(), String> {
    let w0 = run.words[ubo_word("mode_words")];
    if w0 & POST_MODE_SHARPEN == 0 {
        return Err(format!("{tag}: PREMISE: post_word0 = {w0:#x} lacks POST_MODE_SHARPEN under SharpenMode::Rcas"));
    }
    let con = run.words[ubo_word("sharpness")];
    if con != SHARPNESS.to_bits() {
        return Err(format!(
            "{tag}: PREMISE: post_word4 = {con:#x} ({}), not con = {SHARPNESS}'s bits (rcas_sharpness {SHARPNESS})",
            f32::from_bits(con)
        ));
    }
    let mut off = run.words;
    off[ubo_word("mode_words")] &= !POST_MODE_SHARPEN;
    for ((f, hist), (fd, display)) in run.hist.iter().zip(&run.display) {
        if f != fd {
            return Err(format!("{tag}: the taa_hist and display bursts cover different frames ({f} vs {fd})"));
        }
        let (bad, first) = eval_post_rejects(hist, display, run.words, None);
        if bad != 0 {
            return Err(format!(
                "{tag} frame {f}: {bad} display pixels differ from post_final_eval(taa_hist) under the receipt's words beyond one \
                 code + the OETF bound (first: {first:?}): the device did not execute the traced body's sharpen stage"
            ));
        }
        let edges = edge_pixels(hist);
        if edges.len() < EDGE_MIN {
            return Err(format!("{tag} frame {f}: only {} edge pixels (min {EDGE_MIN}): the scene has no edge to sharpen", edges.len()));
        }
        let (rejected, _) = eval_post_rejects(hist, display, off, Some(&edges));
        let share = rejected as f64 / edges.len() as f64;
        if share < ANTI_VACUITY_SHARE {
            return Err(format!(
                "{tag} frame {f}: the sharpen-off words reject display on only {rejected}/{} edge pixels ({:.1} % < {:.0} %): \
                 the agreement does not show the sharpen stage ran",
                edges.len(),
                100.0 * share,
                100.0 * ANTI_VACUITY_SHARE
            ));
        }
    }
    Ok(())
}

/// Rec. 709 luma of an 8-bit display texel, in codes.
fn luma8(t: [u8; 4]) -> f64 {
    0.2126 * f64::from(t[0]) + 0.7152 * f64::from(t[1]) + 0.0722 * f64::from(t[2])
}

/// The sub-pixel position where `profile` first reaches `level` at or after `from`, rising.
fn crossing(profile: &[f64], from: usize, level: f64) -> Option<f64> {
    (from..profile.len().saturating_sub(1)).find_map(|x| {
        let (a, b) = (profile[x], profile[x + 1]);
        (a < level && b >= level).then(|| x as f64 + (level - a) / (b - a))
    })
}

/// The 10–90 % width of the rising (dark → bright) edge of `profile` inside `[lo, hi)`: the steepest
/// rise is the edge; its plateaus are the means 8..12 px either side (clear of a TAA-blurred edge
/// and of RCAS's one-pixel overshoot); `None` when the plateaus differ by less than [`STEP_MIN`]
/// codes (no edge on this row) or a crossing is missing.
fn rise_width(profile: &[f64], lo: usize, hi: usize) -> Option<f64> {
    let g = (lo + 12..hi.saturating_sub(13)).max_by(|&a, &b| {
        (profile[a + 1] - profile[a]).total_cmp(&(profile[b + 1] - profile[b]))
    })?;
    let mean = |r: core::ops::RangeInclusive<usize>| {
        let n = r.clone().count() as f64;
        r.map(|x| profile[x]).sum::<f64>() / n
    };
    let (dark, bright) = (mean(g - 12..=g - 8), mean(g + 8..=g + 12));
    if bright - dark < STEP_MIN {
        return None;
    }
    let from = g - 8;
    let x10 = crossing(profile, from, dark + 0.1 * (bright - dark))?;
    let x90 = crossing(profile, from, dark + 0.9 * (bright - dark))?;
    (x90 >= x10).then_some(x90 - x10)
}

/// Per-row 10–90 % widths of the rising edge inside columns `[lo, hi)` over every row that has one.
fn edge_widths(display: &DumpFile, lo: u32, hi: u32) -> Vec<f64> {
    (0..display.height)
        .filter_map(|y| {
            let profile: Vec<f64> = (0..display.width).map(|x| luma8(display.rgba8(x, y))).collect();
            rise_width(&profile, lo as usize, hi as usize)
        })
        .collect()
}

/// `(mean, median)` of `v` (non-empty).
fn mean_median(v: &[f64]) -> (f64, f64) {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    (s.iter().sum::<f64>() / s.len() as f64, s[s.len() / 2])
}

/// (b) on the last burst frame of the two runs: the premises, then each edge's mean and median
/// width strictly smaller with sharpen on.
fn check_acutance(on: &Run, off: &Run) -> Result<String, String> {
    for ((f_on, h_on), (f_off, h_off)) in on.hist.iter().zip(&off.hist) {
        if f_on != f_off || h_on.texels != h_off.texels {
            return Err(format!(
                "PREMISE: the two runs' taa_hist differ at frame {f_on}/{f_off}: RCAS fed the history, or the runs are not the same frames"
            ));
        }
    }
    let (d_on, d_off) = (&on.display.last().expect("invariant: a non-empty burst").1, &off.display.last().expect("invariant: a non-empty burst").1);
    let mut report = Vec::new();
    // The HDR bar stands on the right half of the frame, the LDR bar on the left.
    for (name, lo, hi) in [("HDR (4.0)", WIN / 2, WIN), ("LDR (0.6)", 0, WIN / 2)] {
        let (w_on, w_off) = (edge_widths(d_on, lo, hi), edge_widths(d_off, lo, hi));
        if w_on.len() < ROWS_MIN || w_off.len() < ROWS_MIN {
            return Err(format!("PREMISE: the {name} edge spans {} / {} rows (min {ROWS_MIN})", w_on.len(), w_off.len()));
        }
        let ramps = w_off.iter().filter(|w| **w >= RAMP_MIN_PX).count();
        if ramps < ROWS_MIN {
            return Err(format!(
                "PREMISE: only {ramps} of the {} sharpen-off {name} edge rows are a ramp (>= {RAMP_MIN_PX} px, min {ROWS_MIN}): \
                 no edge to sharpen",
                w_off.len()
            ));
        }
        let ((mean_on, med_on), (mean_off, med_off)) = (mean_median(&w_on), mean_median(&w_off));
        report.push(format!(
            "{name}: mean {mean_on:.3} vs {mean_off:.3} px, median {med_on:.3} vs {med_off:.3} px ({ramps} of {} sharpen-off \
             rows a ramp)",
            w_off.len()
        ));
        if !(mean_on < mean_off && med_on < med_off) {
            return Err(format!(
                "the {name} edge is not sharper with sharpen on: 10-90 % width mean {mean_on:.3} vs {mean_off:.3} px, median \
                 {med_on:.3} vs {med_off:.3} px (on vs off) — sharpen-on width == sharpen-off width (not strictly smaller)"
            ));
        }
    }
    Ok(report.join("; "))
}

#[test]
#[ignore = "gpu-windowed: PX1b device (a), display == post_final_eval(taa_hist) under the sharpen bit on Deferred and VB (spawns px1b_rcas_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1b_rcas_display_is_post_final_eval_under_sharpen() {
    for path in [RenderPath::Deferred, RenderPath::VisibilityBuffer] {
        let arm = Arm { path, sharpen: true };
        let run = run_arm(arm);
        assert_eq!(run.rcas_key, 1, "{}: the receipt's [config] rcas must say RCAS ran this frame", arm.tag());
        if let Err(e) = check_eval_post_under_sharpen(&arm.tag(), &run) {
            panic!("{e}");
        }
    }
}

#[test]
#[ignore = "gpu-windowed: PX1b device (b), the HDR and LDR edges are sharper with SharpenMode::Rcas on Deferred (spawns px1b_rcas_worker twice); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1b_rcas_sharpens_the_hdr_and_ldr_edges() {
    let on = run_arm(Arm { path: RenderPath::Deferred, sharpen: true });
    let off = run_arm(Arm { path: RenderPath::Deferred, sharpen: false });
    match check_acutance(&on, &off) {
        Ok(report) => eprintln!("px1b (b): {report}"),
        Err(e) => panic!("{e}"),
    }
}

// ===============================================================================================
// Host fixtures (not ignored)
// ===============================================================================================

/// Critique Q2: the bound above is `post_eval`'s floor shape — one display code (the floor
/// `post_eval_cli.rs` pins `eval` to, within 1e-3 code) plus E0's OETF term, which never reaches a
/// hundredth of a code on the display range — and never less than one code.
#[test]
fn the_eval_bound_is_one_code_plus_the_oetf_term() {
    for k in 0..=1000u32 {
        let o = k as f32 / 1000.0;
        let b = eval_bound_lsb(o);
        assert!((1.0..=1.01).contains(&b), "eval_bound_lsb({o}) = {b}, outside one code + the OETF term");
    }
}

/// A synthetic RGBA8 display whose every row is `row(x)` (codes).
fn synthetic_display(row: impl Fn(u32) -> f64) -> DumpFile {
    synthetic_frame(|x, _| row(x))
}

/// A synthetic RGBA8 display whose pixel `(x, y)` is `px(x, y)` (codes).
fn synthetic_frame(px: impl Fn(u32, u32) -> f64) -> DumpFile {
    let mut t = Vec::with_capacity((WIN * WIN * 4) as usize);
    for y in 0..WIN {
        for x in 0..WIN {
            let c = px(x, y).round().clamp(0.0, 255.0) as u8;
            t.extend_from_slice(&[c, c, c, 255]);
        }
    }
    DumpFile::parse(&image_dump::encode(image_dump::VK_R8G8B8A8_UNORM, WIN, WIN, SETTLE, 4, &t)).expect("invariant: the fixture parses")
}

/// A ramp from `dark` to `bright` centred at `at`, `width` px wide (10-90 % ≈ 0.8·width).
fn ramp_edge(x: u32, at: f64, width: f64, dark: f64, bright: f64) -> f64 {
    let t = ((f64::from(x) - at) / width + 0.5).clamp(0.0, 1.0);
    dark + t * (bright - dark)
}

#[test]
fn the_edge_width_measures_a_ramp_and_sees_a_sharper_one() {
    let soft = synthetic_display(|x| ramp_edge(x, 64.0, 6.0, 40.0, 200.0));
    let hard = synthetic_display(|x| ramp_edge(x, 64.0, 3.0, 40.0, 200.0));
    let (ws, wh) = (edge_widths(&soft, 0, WIN / 2), edge_widths(&hard, 0, WIN / 2));
    assert_eq!((ws.len(), wh.len()), (WIN as usize, WIN as usize), "every row of the fixture has its edge");
    let ((ms, _), (mh, _)) = (mean_median(&ws), mean_median(&wh));
    assert!((ms - 4.8).abs() < 0.6, "a 6-px ramp measures {ms:.3} px between its 10 % and 90 % levels");
    assert!(mh < ms, "the 3-px ramp ({mh:.3}) must measure narrower than the 6-px one ({ms:.3})");
    let flat = synthetic_display(|_| 90.0);
    assert!(edge_widths(&flat, 0, WIN / 2).is_empty(), "a flat row has no edge");
}

/// (b)'s premise and verdict on synthetic frames, both edges alike. A one-pixel step measures 0.8 px.
/// A staircase — even rows such a step, odd rows a ramp: the device scene's shape, which an every-row
/// 1 px bound rejected — passes the premise, and narrowing its ramps passes the check; the same frame
/// unchanged (sharpening silently off, W1 / W2) fails the check; a frame of steps only fails the premise.
#[test]
fn the_acutance_check_takes_a_staircase_edge_and_rejects_an_unsharpened_one() {
    // One rising edge per half: the LDR window [0, WIN/2) and the HDR window [WIN/2, WIN).
    let edge = |x: u32, y: u32, ramp: f64| {
        let at = if x < WIN / 2 { 64.0 } else { 192.0 };
        if y.is_multiple_of(2) { ramp_edge(x, at - 0.5, 1.0, 40.0, 200.0) } else { ramp_edge(x, at, ramp, 40.0, 200.0) }
    };
    let run = |ramp: f64, steps_only: bool| Run {
        hist: Vec::new(),
        display: vec![(SETTLE, synthetic_frame(|x, y| edge(x, if steps_only { 0 } else { y }, ramp)))],
        words: [0; 8],
        rcas_key: 0,
    };
    let step = synthetic_display(|x| ramp_edge(x, 63.5, 1.0, 40.0, 200.0));
    let w = edge_widths(&step, 0, WIN / 2);
    assert!(w.len() == WIN as usize && w.iter().all(|v| (v - 0.8).abs() < 1e-9), "a one-pixel step measures 0.8 px: {w:?}");
    let off = run(6.0, false);
    let report = check_acutance(&run(3.0, false), &off).unwrap_or_else(|e| panic!("a staircase whose ramps narrow: {e}"));
    assert!(report.contains("128 of 256 sharpen-off rows a ramp"), "{report}");
    let same = check_acutance(&run(6.0, false), &off).expect_err("an unchanged frame is not sharper");
    assert!(same.contains("is not sharper with sharpen on"), "{same}");
    let steps = check_acutance(&run(3.0, true), &run(6.0, true)).expect_err("an edge of steps only has nothing to sharpen");
    assert!(steps.starts_with("PREMISE: only 0 of the 256"), "{steps}");
}
