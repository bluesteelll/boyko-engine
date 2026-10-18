//! Render wave PX1a tests 16 and 18 (`plan/H4-PX1a-hdr-lit.md` Tests 16, 18; `px1a/cut.md` C4, C8;
//! ruling 67 OQ4): **TAA under the post seam — a camera cut resets the history, and `display` is the
//! accumulated image.**
//!
//! - **16 — the camera cut.** The worker switches the active camera at frame [`SWITCH_AT`]. The
//!   runner must turn `ResolvedPost::cut` into `TaaState::mark_reset` on that same frame, so the
//!   cut frame's resolve replaces the history. Read against `TaaState`'s contract ("the reset frame
//!   itself counts as frame 1", `taa_state.rs`), the per-frame log pins it to the frame (see
//!   [`cut_log_verdict`]), and among the (D) `lit` / `taa_hist` frames dumped around the switch,
//!   exactly the cut frame [`SWITCH_AT`] holds a history equal to its `lit` within one format step
//!   — a replaced (fresh) history — while every blended frame does not.
//! - **18 — accumulation.** A static scene with high-contrast edges — one of them HDR, an emissive
//!   bar at radiance 4.0 against a dark floor (ruling 67 OQ4: the HDR-space resolve differs from
//!   the display-space one only there) — under TAA, one 16-frame (D) burst of `lit` (the jittered
//!   current frame) and `display`. Over the edge pixels (those whose `OETF(ACES(lit))` moves by more
//!   than [`EDGE_STD`] across the burst) the mean frame-to-frame standard deviation of `display` is
//!   at most half that of `OETF(ACES(lit))` computed here from the dumped `lit`. A `post_final` that
//!   reads `lit` instead of `taa_hist` makes the ratio ≈ 1.
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
use boyko_render::{AaConfig, AaMode, GeometryLegs, Material, RenderPath, RenderPathConfig, TaaState};
use boyko_rhi_vulkan::present::ImageDumpName;
use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::post::{eval_params, post_final_eval, ubo_word};

use graph_receipt::{fresh_out, run_worker};
use image_dump::{DumpFile, files_of};

/// The worker's mode (`cut` | `accumulate`).
const ENV_MODE: &str = "BOYKO_PX1A_TAA";
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Cut,
    Accumulate,
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

/// The cut worker's log: one `frame frames_since_reset` line per frame.
#[allow(clippy::needless_pass_by_value)]
fn log_taa_state(stats: Res<HostFrameStats>, taa: Res<TaaState>) {
    let Ok(path) = std::env::var(ENV_LOG) else { return };
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).expect("invariant: the log is writable");
    writeln!(f, "{} {}", stats.frames, taa.frames_since_reset).expect("invariant: the log accepts a line");
}

/// **THE WORKER** — `BOYKO_PX1A_TAA=cut|accumulate`.
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
        other => panic!("{ENV_MODE}={other} is not cut|accumulate"),
    };
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine px1a taa", WIN, WIN));
    app.insert_resource(ModeRes(mode));
    app.insert_resource(Cameras::default());
    app.add_startup_system(taa_setup);
    app.insert_resource(RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Mesh });
    app.insert_resource(AaConfig { mode: AaMode::Taa });
    if mode == Mode::Cut {
        app.add_systems(switch_camera);
        app.add_systems(log_taa_state);
    }
    app.run();
    assert!(app.world().resource::<HostFrameStats>().frames > 0, "the frame loop never ran — RED, never a skip");
}

fn dump_dir(tag: &str) -> PathBuf {
    let dir = fresh_out(tag);
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    dir
}

/// `OETF(ACES(c))` of one channel through the traced `post_final` body (ACES Hill, ratio 1).
fn display_of(rgb: [f32; 3]) -> [f32; 3] {
    let mut words = [0u32; 8];
    words[ubo_word("exposure_ratio")] = 1.0f32.to_bits();
    words[ubo_word("extent")] = 1;
    words[ubo_word("extent") + 1] = 1;
    let texel = [[rgb[0], rgb[1], rgb[2], 1.0]];
    let o = post_final_eval([0, 0], &eval_params(words), HostTex2::new(&texel, 1, 1));
    [o[0], o[1], o[2]]
}

/// Per pixel, the mean over channels of the standard deviation across `frames` of `f(texel)`.
fn pixel_std(frames: &[DumpFile], f: impl Fn(&DumpFile, u32, u32) -> [f32; 3]) -> Vec<f64> {
    let (w, h) = (frames[0].width, frames[0].height);
    let n = frames.len() as f64;
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let vals: Vec<[f32; 3]> = frames.iter().map(|d| f(d, x, y)).collect();
            let mut s = 0.0;
            for c in 0..3 {
                let mean = vals.iter().map(|v| f64::from(v[c])).sum::<f64>() / n;
                let var = vals.iter().map(|v| (f64::from(v[c]) - mean).powi(2)).sum::<f64>() / n;
                s += var.sqrt();
            }
            out.push(s / 3.0);
        }
    }
    out
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
    let lit_std = pixel_std(&lit, |d, x, y| {
        let t = d.texel_f32(x, y);
        display_of([t[0], t[1], t[2]])
    });
    let display_std = pixel_std(&display, |d, x, y| {
        let t = d.texel_f32(x, y);
        [t[0], t[1], t[2]]
    });
    let (edges, ratio) = accumulation_ratio(&display_std, &lit_std);
    assert!(edges >= 50, "only {edges} edge pixels: the scene has no jittered edge to measure");
    assert!(ratio <= 0.5, "display's frame-to-frame std is {ratio:.3} x OETF(ACES(lit))'s over {edges} edge pixels: no accumulation");
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

#[test]
fn display_of_is_aces_then_the_oetf() {
    assert_eq!(display_of([0.0; 3]), [0.0; 3]);
    let w = display_of([64.0; 3]);
    assert!(w.iter().all(|&c| c > 0.99), "a hot radiance displays white: {w:?}");
}
