//! Render wave PX0 (H3 Scope 1; `px0/cut.md` D-Z1..D-Z4) — **the POST / FWD zone-label gate**.
//!
//! PX0 brackets every anti-aliasing / sharpen / present pass (the POST family, ids 80..=87) and the
//! forward-shaded lit producers that had no zone (the FWD family, ids 96..=98), gives the Forward
//! recorder the frame witness the other two paths have, and puts a CPU zone on
//! `declare_frame_graph`. This file is the gate that the brackets sit where their ids say.
//!
//! # The label is the artifact row
//!
//! A bracketed, available pair is a `Measured` row; a zone no recorder opened has **no row at all**
//! (the reducer forms rows from `PairResult`s, and an unopened zone has none). So each cell's
//! expectation table says `Required` (a `Measured` row must exist) or `Forbidden` (no row may
//! exist) per id, and [`check_expectations`] is red in both directions — a missing bracket and a
//! bracket under the wrong family.
//!
//! # The cells
//!
//! `BOYKO_PX0_ZONE_CELL=<path>:<aa>` — paths `deferred` (Both legs), `vb` (Mesh), `vb_both`,
//! `vb_split` (Mesh + SSAO High, the split producer), `forward_both`, `forwardplus` (Mesh); AA
//! `off | taa | taa_cas | fxaa | smaa | ssaa`. The drivers run Deferred × every AA mode, VB × every
//! AA mode, `forward_both:off`, `forwardplus:off`, `vb_both:off`; a validation-on driver; the CPU
//! zone driver.
//!
//! # WR-1 (quiet window, orchestrator) — [`post_zone_wr_worker`]
//!
//! `BOYKO_PX0_WR=<path>:<aa>:<W>x<H>` boots one cell at that extent and inherits the artifact knobs
//! (`BOYKO_VB_ZONE`, `BOYKO_VB_BENCH_FRAMES`, `BOYKO_PROFILE_ARTIFACT`, `BOYKO_PROFILE_RUN_TOKEN`,
//! `BOYKO_PROFILE_WORKLOAD`) from the caller; it measures nothing itself and prints the CPU zone's
//! accumulated calls and mean at exit. The classified VB producer is the `vb` cell with
//! `BOYKO_VB_FORCE_CLASSIFIED=1` in the caller's environment.
//!
//! Windowed-test conventions: the device drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1` (the validation driver arms the layer for its own children).

use std::path::{Path, PathBuf};
use std::process::Command;

use boyko_app::prelude::*;
use boyko_app::profiling::artifact::{Artifact, ZoneLabel, ZoneRow};
use boyko_app::profiling::reduce::Expect;
use boyko_ecs::ecs::core::profiling::ROOT_SCOPE;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{
    AaConfig, AaMode, GeometryLegs, Material, RenderPath, RenderPathConfig, ResolvedRenderPath,
    SharpenMode, SsaoConfig, SsaoQuality, TaaConfig,
};
use boyko_rhi_vulkan::present::gpu_zone::{
    FWD_ZONE_COUNT, POST_ZONE_COUNT, RENDER_DECLARE_FRAME_GRAPH, ZONE_BASE_FWD, ZONE_BASE_POST,
    ZONE_FWD_OPAQUE, ZONE_FWD_SDF_MARCH, ZONE_FWD_VB_SKY, ZONE_GBUF_DEFERRED_RESOLVE,
    ZONE_POST_FINAL, ZONE_POST_RETIRED, ZONE_POST_FXAA, ZONE_POST_PRESENT, ZONE_POST_SMAA_BLEND, ZONE_POST_SMAA_EDGE,
    ZONE_POST_SMAA_WEIGHT, ZONE_POST_SSAA_DOWNSAMPLE, ZONE_POST_TAA_RESOLVE, ZONE_VB_RUN,
};

// ===============================================================================================
// Knobs
// ===============================================================================================

/// The label worker every driver re-executes.
const WORKER: &str = "post_zone_labels_worker";
/// The cell the label worker boots.
const ENV_CELL: &str = "BOYKO_PX0_ZONE_CELL";
/// The WR-1 worker's cell + extent (`<path>:<aa>:<W>x<H>`).
const ENV_WR: &str = "BOYKO_PX0_WR";
/// Timed frames per label worker. Odd, for `vg_occ_split_timing`'s reason (a median of an even
/// count is the mean of two samples).
const BENCH_FRAMES: u32 = 61;
/// The label worker's window edge.
const LABEL_WIN: u32 = 256;
/// The line the worker prints at exit — the CPU zone's accumulated count and ticks.
const CPU_LINE: &str = "PX0_CPU_ZONE";
/// The frame backstop every label child runs under, as `BOYKO_WINDOW_FRAMES` — the runner counts it
/// at its loop top, whether or not a frame retires. A healthy cell exits on its zone window first
/// (`VB_BENCH_WARMUP` = 20 plus [`BENCH_FRAMES`] RETIRED frames, about 90 iterations with the ring's
/// lag); a path whose frame witness never opens retires no frame, and without this bound its child
/// rendered forever and GL-PX0-Z hung instead of reddening (triage r1 RF-1).
const FRAME_BACKSTOP: u32 = 1_000;
const _: () = assert!(FRAME_BACKSTOP > 4 * (20 + BENCH_FRAMES), "the backstop must sit far past a healthy window");

// ===============================================================================================
// Cells
// ===============================================================================================

/// The path (and geometry legs) a cell boots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathCell {
    /// Deferred × Both — the SV0 marcher and the gbuffer family.
    Deferred,
    /// VisibilityBuffer × Mesh — the fused producer.
    Vb,
    /// VisibilityBuffer × Both — VB's `sdf_forward_march` site.
    VbBoth,
    /// VisibilityBuffer × Mesh with SSAO High — the split producer (WR-1's spread row).
    VbSplit,
    /// Forward × Both — `forward_opaque` and Forward's `sdf_forward_march` site.
    ForwardBoth,
    /// ForwardPlus × Mesh — the froxel `forward_opaque` variant.
    ForwardPlus,
}

impl PathCell {
    fn from_word(w: &str) -> Option<PathCell> {
        Some(match w {
            "deferred" => PathCell::Deferred,
            "vb" => PathCell::Vb,
            "vb_both" => PathCell::VbBoth,
            "vb_split" => PathCell::VbSplit,
            "forward_both" => PathCell::ForwardBoth,
            "forwardplus" => PathCell::ForwardPlus,
            _ => return None,
        })
    }

    fn word(self) -> &'static str {
        match self {
            PathCell::Deferred => "deferred",
            PathCell::Vb => "vb",
            PathCell::VbBoth => "vb_both",
            PathCell::VbSplit => "vb_split",
            PathCell::ForwardBoth => "forward_both",
            PathCell::ForwardPlus => "forwardplus",
        }
    }

    fn render_path(self) -> RenderPath {
        match self {
            PathCell::Deferred => RenderPath::Deferred,
            PathCell::Vb | PathCell::VbBoth | PathCell::VbSplit => RenderPath::VisibilityBuffer,
            PathCell::ForwardBoth => RenderPath::Forward,
            PathCell::ForwardPlus => RenderPath::ForwardPlus,
        }
    }

    fn legs(self) -> GeometryLegs {
        match self {
            PathCell::Deferred | PathCell::VbBoth | PathCell::ForwardBoth => GeometryLegs::Both,
            PathCell::Vb | PathCell::VbSplit | PathCell::ForwardPlus => GeometryLegs::Mesh,
        }
    }

    fn is_vb(self) -> bool {
        matches!(self, PathCell::Vb | PathCell::VbBoth | PathCell::VbSplit)
    }

    fn is_forward(self) -> bool {
        matches!(self, PathCell::ForwardBoth | PathCell::ForwardPlus)
    }
}

/// The anti-aliasing / sharpen arm a cell boots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AaCell {
    Off,
    Taa,
    TaaCas,
    Fxaa,
    Smaa,
    Ssaa,
}

impl AaCell {
    fn from_word(w: &str) -> Option<AaCell> {
        Some(match w {
            "off" => AaCell::Off,
            "taa" => AaCell::Taa,
            "taa_cas" => AaCell::TaaCas,
            "fxaa" => AaCell::Fxaa,
            "smaa" => AaCell::Smaa,
            "ssaa" => AaCell::Ssaa,
            _ => return None,
        })
    }

    fn word(self) -> &'static str {
        match self {
            AaCell::Off => "off",
            AaCell::Taa => "taa",
            AaCell::TaaCas => "taa_cas",
            AaCell::Fxaa => "fxaa",
            AaCell::Smaa => "smaa",
            AaCell::Ssaa => "ssaa",
        }
    }
}

/// One cell: a path and an AA arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ZoneCell {
    path: PathCell,
    aa: AaCell,
}

impl ZoneCell {
    /// Parses `<path>:<aa>`. A word outside the vocabulary is a named error, never a default — a
    /// mistyped cell must not boot some other configuration and pass.
    fn parse(s: &str) -> Result<ZoneCell, String> {
        let (p, a) = s.trim().split_once(':').ok_or_else(|| format!("`{s}` is not <path>:<aa>"))?;
        let path = PathCell::from_word(p).ok_or_else(|| {
            format!("path `{p}` is not deferred|vb|vb_both|vb_split|forward_both|forwardplus")
        })?;
        let aa = AaCell::from_word(a)
            .ok_or_else(|| format!("aa `{a}` is not off|taa|taa_cas|fxaa|smaa|ssaa"))?;
        if path.is_forward() && aa != AaCell::Off {
            return Err(format!("`{s}`: the Forward family runs no AA pass this rung covers"));
        }
        Ok(ZoneCell { path, aa })
    }

    fn word(self) -> String {
        format!("{}:{}", self.path.word(), self.aa.word())
    }
}

/// Every cell a label driver runs — the set [`expectation_table_names_every_post_and_fwd_id`]
/// checks covers every POST / FWD id as `Required` at least once.
const CELLS: [ZoneCell; 15] = {
    use AaCell::{Fxaa, Off, Smaa, Ssaa, Taa, TaaCas};
    use PathCell::{Deferred, ForwardBoth, ForwardPlus, Vb, VbBoth};
    [
        ZoneCell { path: Deferred, aa: Off },
        ZoneCell { path: Deferred, aa: Taa },
        ZoneCell { path: Deferred, aa: TaaCas },
        ZoneCell { path: Deferred, aa: Fxaa },
        ZoneCell { path: Deferred, aa: Smaa },
        ZoneCell { path: Deferred, aa: Ssaa },
        ZoneCell { path: Vb, aa: Off },
        ZoneCell { path: Vb, aa: Taa },
        ZoneCell { path: Vb, aa: TaaCas },
        ZoneCell { path: Vb, aa: Fxaa },
        ZoneCell { path: Vb, aa: Smaa },
        ZoneCell { path: Vb, aa: Ssaa },
        ZoneCell { path: ForwardBoth, aa: Off },
        ZoneCell { path: ForwardPlus, aa: Off },
        ZoneCell { path: VbBoth, aa: Off },
    ]
};

// ===============================================================================================
// The expectation table
// ===============================================================================================

/// One row of a cell's expectation table.
#[derive(Clone, Copy, Debug)]
struct Cell {
    zone: u16,
    name: &'static str,
    expect: Expect,
}

fn req(b: bool) -> Expect {
    if b { Expect::Required } else { Expect::Forbidden }
}

/// The table for one cell: every POST and FWD id, plus one sentinel of the two families that
/// already existed (the gbuffer resolve, the VB run bracket) — the "present exactly as today" half.
fn table(c: ZoneCell) -> Vec<Cell> {
    let post = !c.path.is_forward();
    let taa = post && matches!(c.aa, AaCell::Taa | AaCell::TaaCas);
    vec![
        Cell { zone: ZONE_POST_TAA_RESOLVE, name: "post_taa_resolve", expect: req(taa) },
        // Render wave PX1b (D-R14): the CAS pass is retired — the sharpen is `post_final`'s RCAS
        // stage (zone 88), so no cell may bracket id 81.
        Cell { zone: ZONE_POST_RETIRED[0], name: "post_cas", expect: Expect::Forbidden },
        Cell { zone: ZONE_POST_FXAA, name: "post_fxaa", expect: req(post && c.aa == AaCell::Fxaa) },
        Cell { zone: ZONE_POST_SMAA_EDGE, name: "post_smaa_edge", expect: req(post && c.aa == AaCell::Smaa) },
        Cell { zone: ZONE_POST_SMAA_WEIGHT, name: "post_smaa_weight", expect: req(post && c.aa == AaCell::Smaa) },
        Cell { zone: ZONE_POST_SMAA_BLEND, name: "post_smaa_blend", expect: req(post && c.aa == AaCell::Smaa) },
        Cell {
            zone: ZONE_POST_SSAA_DOWNSAMPLE,
            name: "post_ssaa_downsample",
            expect: req(post && c.aa == AaCell::Ssaa),
        },
        // Every path presents.
        Cell { zone: ZONE_POST_PRESENT, name: "post_present", expect: Expect::Required },
        // Render wave PX1a (D-Z1): every path records the post seam's tail.
        Cell { zone: ZONE_POST_FINAL, name: "post_final", expect: Expect::Required },
        Cell { zone: ZONE_FWD_OPAQUE, name: "fwd_opaque", expect: req(c.path.is_forward()) },
        Cell {
            zone: ZONE_FWD_SDF_MARCH,
            name: "fwd_sdf_march",
            expect: req(matches!(c.path, PathCell::ForwardBoth | PathCell::VbBoth)),
        },
        Cell { zone: ZONE_FWD_VB_SKY, name: "fwd_vb_sky", expect: req(c.path.is_vb()) },
        Cell {
            zone: ZONE_GBUF_DEFERRED_RESOLVE,
            name: "gbuf_deferred_resolve",
            expect: req(c.path == PathCell::Deferred),
        },
        Cell { zone: ZONE_VB_RUN, name: "vb_run", expect: req(c.path.is_vb()) },
    ]
}

/// Checks one cell's rows against its table — `Ok(())` or the first breach, named.
fn check_expectations(rows: &[ZoneRow], table: &[Cell]) -> Result<(), String> {
    for c in table {
        let row = rows.iter().find(|r| r.zone == c.zone);
        match (c.expect, row) {
            (Expect::Required, None) => {
                return Err(format!("zone {} (`{}`) expected Measured, absent", c.zone, c.name));
            }
            (Expect::Required, Some(r)) if r.label != ZoneLabel::Measured => {
                return Err(format!(
                    "zone {} (`{}`) expected Measured, its row is `{:?}`",
                    c.zone, c.name, r.label
                ));
            }
            (Expect::Forbidden, Some(r)) => {
                return Err(format!(
                    "zone {} (`{}`) is Forbidden on this cell and has a `{:?}` row (n={}): a \
                     bracket opened where this cell records no such pass",
                    c.zone, c.name, r.label, r.n
                ));
            }
            (Expect::Required, Some(_)) | (Expect::Forbidden, None) => {}
        }
    }
    Ok(())
}

/// The CPU zone's evidence: its accumulated count against the frames the GPU window folded.
///
/// `window_frames` is the `n` of a zone every recorded frame of the window stamps (the present
/// blit); every such frame declared its graph first, so `calls >= window_frames`. The worker's
/// warm-up frames declared too, so equality is not required.
fn check_cpu_zone(calls: u64, window_frames: u32) -> Result<(), String> {
    if calls == 0 || calls < u64::from(window_frames) {
        return Err(format!(
            "render.declare_frame_graph lifetime count {calls}, expected >= the {window_frames} \
             recorded frames"
        ));
    }
    Ok(())
}

/// Parses the worker's `PX0_CPU_ZONE calls=<n> ticks=<t>` record wherever it sits in a line: the
/// child runs under `--nocapture`, and libtest has already written `test <name> ... ` without a
/// newline when the worker's `println!` lands, so the record is glued to that prefix — it never
/// starts a line in the real output (triage r1 F2).
fn parse_cpu_line(out: &str) -> Option<(u64, u64)> {
    let line = out.lines().find_map(|l| l.find(CPU_LINE).map(|at| &l[at..]))?;
    let field = |k: &str| -> Option<u64> {
        line.split_whitespace().find_map(|t| t.strip_prefix(k)).and_then(|v| v.parse().ok())
    };
    Some((field("calls=")?, field("ticks=")?))
}

// ===============================================================================================
// The worker
// ===============================================================================================

const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// A small scene every cell can draw: a floor, a cube with real silhouette edges (for the AA
/// passes), an SDF sphere where the cell runs the SDF leg, a sun and a SkyLight.
#[allow(clippy::needless_pass_by_value)]
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let row = u16::try_from(red.index()).expect("invariant: the scene mints < 65536 rows");
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY)).insert(ShadowCaster);
    let cube_at = Transform::from_translation(Vec3::new(-0.9, 0.5, 0.0));
    commands
        .spawn(MeshBundle { material: MaterialHandle(row), ..MeshBundle::new(cube, cube_at) })
        .insert(ShadowCaster);
    commands.spawn(SdfPrimitive(SdfEdit::sphere([0.9, 0.6, 0.0], 0.6, sdf_op::UNION, 0.0)));
    let sun_pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
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
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.6, 5.0), Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.0, 1.0, 0.0));
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
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// Builds one cell's app at `w × h`.
fn build_cell_app(c: ZoneCell, w: u32, h: u32) -> App {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine px0 zone labels", w, h);
    // SSAA is boot-fixed: the scale is requested on the builder (`grand_showcase_2mat.rs`'s note).
    let plugins = if c.aa == AaCell::Ssaa { plugins.with_ssaa_scale(2) } else { plugins };
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Configuration AFTER `add_plugins`, which seeds every default.
    app.insert_resource(RenderPathConfig { path: c.path.render_path(), legs: c.path.legs() });
    match c.aa {
        AaCell::Off | AaCell::Ssaa => {}
        AaCell::Taa => {
            app.insert_resource(AaConfig { mode: AaMode::Taa });
        }
        AaCell::TaaCas => {
            app.insert_resource(AaConfig { mode: AaMode::Taa });
            app.insert_resource(TaaConfig { sharpen: SharpenMode::Rcas, ..TaaConfig::default() });
        }
        AaCell::Fxaa => {
            app.insert_resource(AaConfig { mode: AaMode::Fxaa });
        }
        AaCell::Smaa => {
            app.insert_resource(AaConfig { mode: AaMode::Smaa });
        }
    }
    if c.path == PathCell::VbSplit {
        app.insert_resource(SsaoConfig { quality: SsaoQuality::High, atrous_levels: 3 });
    }
    app
}

/// Prints the CPU zone's accumulated count and ticks — the handle's own accumulators, which every
/// closed interval increments (`ZoneGuard::drop`), whichever lane the render thread writes on.
fn print_cpu_zone() {
    let calls = RENDER_DECLARE_FRAME_GRAPH.calls();
    let ticks = RENDER_DECLARE_FRAME_GRAPH.ticks();
    let mean_ns = if calls == 0 {
        0.0
    } else {
        ticks as f64 / boyko_diag::clock::ticks_per_ns() / calls as f64
    };
    println!("{CPU_LINE} calls={calls} ticks={ticks} mean_ns={mean_ns:.1}");
}

/// A validated child must have armed the layer and heard no ERROR and no VALIDATION/PERFORMANCE
/// WARNING (`boot_validation_clean`'s verdict, read from the same ledger).
fn assert_validation_clean() {
    let l = boyko_rhi_vulkan::debug::validation_ledger();
    assert!(l.messengers_created >= 1, "validation was requested but no messenger was created: {l:?}");
    assert!(
        l.errors == 0 && l.general_errors == 0 && l.validation_warnings == 0,
        "the validation layer reported messages on a PX0 zone cell: {l:?}"
    );
}

fn assert_resolved(app: &App, c: ZoneCell) {
    let frames = app.world().resource::<HostFrameStats>().frames;
    assert!(frames > 0, "the frame loop never ran (no windowed device?) — RED, never a skip");
    let resolved = *app.world().resource::<ResolvedRenderPath>();
    assert_eq!(
        resolved.path,
        c.path.render_path(),
        "PREMISE: asked for {:?}, the boot resolved {resolved:?}",
        c.path
    );
}

/// **THE LABEL WORKER** — one cell with the zone recorder armed.
#[test]
#[ignore = "gpu-windowed: spawned by the post_zone_labels drivers; needs a real windowed GPU device"]
fn post_zone_labels_worker() {
    let Ok(word) = std::env::var(ENV_CELL) else {
        eprintln!("{WORKER}: {ENV_CELL} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    if std::env::var_os("BOYKO_VB_ZONE").is_none() {
        eprintln!("{WORKER}: BOYKO_VB_ZONE is unset, so no exit condition is armed -- SKIPPED.");
        return;
    }
    let c = ZoneCell::parse(&word).unwrap_or_else(|e| panic!("{ENV_CELL}: {e}"));
    let mut app = build_cell_app(c, LABEL_WIN, LABEL_WIN);
    app.run();
    print_cpu_zone();
    assert_resolved(&app, c);
    if std::env::var_os("BOYKO_ENABLE_VALIDATION").is_some() {
        assert_validation_clean();
    }
}

/// **THE WR-1 WORKER** — one cell at the extent `BOYKO_PX0_WR` names; the artifact knobs are the
/// caller's. NO TIMING VERDICT: the orchestrator's quiet-window protocol reads the artifacts.
#[test]
#[ignore = "gpu-windowed: the PX0 WR-1 worker; run by the orchestrator in a quiet window with BOYKO_PX0_WR, BOYKO_VB_ZONE=1, BOYKO_VB_BENCH_FRAMES, BOYKO_PROFILE_ARTIFACT, --test-threads=1"]
fn post_zone_wr_worker() {
    let Ok(spec) = std::env::var(ENV_WR) else {
        eprintln!("post_zone_wr_worker: {ENV_WR} is unset -- SKIPPED.");
        return;
    };
    if std::env::var_os("BOYKO_VB_ZONE").is_none() {
        panic!("post_zone_wr_worker: BOYKO_VB_ZONE is unset — a WR run with no exit condition renders forever");
    }
    let (cell, ext) = spec
        .rsplit_once(':')
        .unwrap_or_else(|| panic!("{ENV_WR}=`{spec}` is not <path>:<aa>:<W>x<H>"));
    let c = ZoneCell::parse(cell).unwrap_or_else(|e| panic!("{ENV_WR}: {e}"));
    let (w, h) = ext
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .unwrap_or_else(|| panic!("{ENV_WR}: extent `{ext}` is not <W>x<H>"));
    let mut app = build_cell_app(c, w, h);
    app.run();
    print_cpu_zone();
    assert_resolved(&app, c);
}

// ===============================================================================================
// Driving the worker
// ===============================================================================================

/// Every knob an operator's shell might carry that would change the scene, the cell or the stream —
/// removed from the label child rather than assumed unset (`vb_sv0_produce_run_timing`'s reason).
/// `BOYKO_WINDOW_FRAMES` is NOT one of them: the child's is [`FRAME_BACKSTOP`], set by
/// [`worker_command`], never inherited and never removed (that precedent strips it; here a strip
/// would undo the backstop — [`every_label_child_runs_under_the_frame_backstop`]).
const STRIPPED_KNOBS: [&str; 11] = [
    "BOYKO_HOST_DUMP",
    "BOYKO_VB_BENCH",
    "BOYKO_VB_CULL_READBACK",
    "BOYKO_VB_PROBE",
    "BOYKO_HZB_DUMP",
    "BOYKO_AA",
    "BOYKO_SSAO",
    "BOYKO_GRAPH_RECEIPT",
    "BOYKO_IMAGE_DUMP",
    "BOYKO_VB_FORCE_CLASSIFIED",
    "BOYKO_PX0_WR",
];

/// The label child for one cell, writing its artifact to `artifact` under run token `token` —
/// built here, apart from [`run_cell`]'s spawn, so a host test reads the environment it carries.
fn worker_command(c: ZoneCell, artifact: &Path, token: &str, validation_on: bool, profile_on: bool) -> Command {
    let tag = c.word().replace(':', "_");
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(exe);
    cmd.args([WORKER, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(ENV_CELL, c.word())
        .env("BOYKO_VB_ZONE", "1")
        .env("BOYKO_PROFILE_ARTIFACT", artifact)
        .env("BOYKO_PROFILE_RUN_TOKEN", token)
        .env("BOYKO_PROFILE_WORKLOAD", format!("px0_{tag}"))
        .env("BOYKO_VB_BENCH_FRAMES", BENCH_FRAMES.to_string())
        .env("BOYKO_WINDOW_FRAMES", FRAME_BACKSTOP.to_string());
    for k in STRIPPED_KNOBS {
        cmd.env_remove(k);
    }
    if validation_on {
        cmd.env_remove("BOYKO_DISABLE_VALIDATION")
            .env("BOYKO_ENABLE_VALIDATION", "1")
            .env("VK_KHRONOS_VALIDATION_ENABLE_MESSAGE_LIMIT", "false")
            .env("VK_LOADER_LAYERS_DISABLE", "~implicit~");
    } else {
        cmd.env("BOYKO_DISABLE_VALIDATION", "1").env_remove("BOYKO_ENABLE_VALIDATION");
    }
    if profile_on {
        cmd.env("BOYKO_PROFILE_ON", "1");
    } else {
        cmd.env_remove("BOYKO_PROFILE_ON");
    }
    cmd
}

/// The worker's artifact and merged output for one cell, or `None` when the device serves no
/// timestamps (the zone leg cannot run there).
fn run_cell(c: ZoneCell, validation_on: bool, profile_on: bool) -> Option<(Artifact, String)> {
    let tag = c.word().replace(':', "_");
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("boyko_px0_zones_{tag}.toml"));
    let _ = std::fs::remove_file(&path);
    let token = format!("px0-zones-{tag}");

    let out = worker_command(c, &path, &token, validation_on, profile_on)
        .output()
        .expect("invariant: the PX0 zone worker spawns");
    let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&out.stderr));
    if merged.contains("no usable timestamps") {
        eprintln!("{WORKER}: this device serves no timestamps -- SKIPPED");
        return None;
    }
    assert!(
        out.status.success(),
        "the PX0 zone worker (`{}`) exited {}.\n---- worker output ----\n{merged}",
        c.word(),
        out.status
    );
    // The window writes its artifact as its last act; a clean exit without one is the backstop's.
    if !path.exists() {
        panic!("{}\n---- worker output ----\n{merged}", no_window_verdict(c));
    }
    let art = Artifact::read(&path, &token).unwrap_or_else(|e| {
        panic!("`{}`: the worker completed but its artifact is unusable: {e}\n---- worker output ----\n{merged}", c.word())
    });
    let _ = std::fs::remove_file(&path);
    Some((art, merged))
}

/// The red of a child that exited at [`FRAME_BACKSTOP`] with no zone window retired (no artifact):
/// every zone it should have bracketed is absent, and the message names the first Required one the
/// way a present-but-short artifact would.
fn no_window_verdict(c: ZoneCell) -> String {
    let why = check_expectations(&[], &table(c)).err().unwrap_or_else(|| "the cell requires no zone".to_owned());
    format!(
        "[{}] the worker reached the {FRAME_BACKSTOP}-frame backstop without retiring a zone window \
         (no artifact: no frame witness opened?) — {why}",
        c.word()
    )
}

/// One label driver: the cell's artifact against its table.
fn drive(word: &str) {
    let c = ZoneCell::parse(word).expect("invariant: the driver names a valid cell");
    let Some((art, _)) = run_cell(c, false, false) else { return };
    if let Err(why) = check_expectations(&art.zones, &table(c)) {
        panic!("[{word}] {why}");
    }
    eprintln!("[{word}] every POST / FWD zone labelled as its table says ({} rows)", art.zones.len());
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_deferred_off() {
    drive("deferred:off");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_deferred_taa() {
    drive("deferred:taa");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_deferred_taa_cas() {
    drive("deferred:taa_cas");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_deferred_fxaa() {
    drive("deferred:fxaa");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_deferred_smaa() {
    drive("deferred:smaa");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_deferred_ssaa() {
    drive("deferred:ssaa");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_vb_off() {
    drive("vb:off");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_vb_taa() {
    drive("vb:taa");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_vb_taa_cas() {
    drive("vb:taa_cas");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_vb_fxaa() {
    drive("vb:fxaa");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_vb_smaa() {
    drive("vb:smaa");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_vb_ssaa() {
    drive("vb:ssaa");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_forward_both_off() {
    drive("forward_both:off");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_forwardplus_off() {
    drive("forwardplus:off");
}

#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver (spawns post_zone_labels_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_vb_both_off() {
    drive("vb_both:off");
}

/// **The validation-on driver** — three cells that exercise the three structural changes
/// (Deferred's TAA + sharpen brackets, VB's SMAA partition, Forward's new frame witness) with the
/// layer armed; each child asserts the ledger clean in-process and the labels hold. Since render
/// wave PX1b the Deferred cell's sharpen is `post_final`'s RCAS stage, bracketed by
/// `post_final`'s own zone — no CAS pass, its id Forbidden.
#[test]
#[ignore = "gpu-windowed: PX0 zone-label driver with the validation layer armed in each child; run with --test-threads=1"]
fn zones_validation_on() {
    for word in ["deferred:taa_cas", "vb:smaa", "forward_both:off"] {
        let c = ZoneCell::parse(word).expect("invariant: a valid cell");
        let Some((art, _)) = run_cell(c, true, false) else { return };
        if let Err(why) = check_expectations(&art.zones, &table(c)) {
            panic!("[{word}, validation on] {why}");
        }
    }
}

/// **The CPU-zone driver** — `render.declare_frame_graph` closes once per declared frame.
#[test]
#[ignore = "gpu-windowed: PX0 CPU-zone driver (BOYKO_PROFILE_ON in the child); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn zones_cpu_declare_frame_graph() {
    let c = ZoneCell { path: PathCell::Deferred, aa: AaCell::Off };
    let Some((art, merged)) = run_cell(c, false, true) else { return };
    let (calls, _ticks) = parse_cpu_line(&merged)
        .unwrap_or_else(|| panic!("the worker printed no `{CPU_LINE}` line:\n{merged}"));
    // The window's frame count, from a zone that existed before PX0 (so this clause does not
    // depend on PX0's own brackets): every recorded Deferred frame stamps the resolve.
    let n = art
        .zones
        .iter()
        .find(|r| r.zone == ZONE_GBUF_DEFERRED_RESOLVE)
        .map_or(0, |r| r.n);
    assert!(n > 0, "PREMISE: the Deferred window folded no frame of the resolve zone");
    if let Err(why) = check_cpu_zone(calls, n) {
        panic!("[deferred:off, profiler armed] {why}");
    }
}

// ===============================================================================================
// The host half — no GPU
// ===============================================================================================

/// The POST ids a retired pass held (render wave PX1b D-R14: `ZONE_POST_RETIRED`, the CAS pass's
/// 81) — Forbidden in every cell, so exempt from "Required in some cell".
const RETIRED: [u16; 1] = ZONE_POST_RETIRED;

/// **Every POST and FWD id has exactly one row in every cell's table, and every live one is
/// Required in at least one cell** — red when a family grows an id the gate does not name, or when
/// no cell would ever observe a bracket. A RETIRED id is Forbidden in every cell instead.
#[test]
fn expectation_table_names_every_post_and_fwd_id() {
    let ids: Vec<u16> = (ZONE_BASE_POST..ZONE_BASE_POST + POST_ZONE_COUNT)
        .chain(ZONE_BASE_FWD..ZONE_BASE_FWD + FWD_ZONE_COUNT)
        .collect();
    for c in CELLS {
        let t = table(c);
        for id in &ids {
            let n = t.iter().filter(|r| r.zone == *id).count();
            assert_eq!(n, 1, "cell {} names zone {id} {n} times (expected exactly once)", c.word());
        }
        for (i, a) in t.iter().enumerate() {
            assert!(
                t[i + 1..].iter().all(|b| b.zone != a.zone),
                "cell {} names zone {} twice",
                c.word(),
                a.zone
            );
        }
    }
    for id in &ids {
        if RETIRED.contains(id) {
            assert!(
                CELLS.iter().all(|c| table(*c).iter().any(|r| r.zone == *id && r.expect == Expect::Forbidden)),
                "the retired zone {id} must be Forbidden in every cell"
            );
            continue;
        }
        assert!(
            CELLS.iter().any(|c| table(*c).iter().any(|r| r.zone == *id && r.expect == Expect::Required)),
            "zone {id} is Required in no cell, so no driver can observe its bracket"
        );
    }
    // Every cell word round-trips — the drivers' literals are cells of this list.
    for c in CELLS {
        assert_eq!(ZoneCell::parse(&c.word()), Ok(c));
    }
}

fn measured(zone: u16) -> ZoneRow {
    ZoneRow {
        zone,
        label: ZoneLabel::Measured,
        n: 40,
        median_ns: 1_000.0,
        mean_ns: 1_000.0,
        p95_ns: 1_000.0,
        stddev_ns: 0.0,
        begin_off_ns: 0.0,
        end_off_ns: 1_000.0,
    }
}

/// [`check_expectations`] reds in both directions and passes the exact set.
#[test]
fn check_expectations_reds_both_ways() {
    let c = ZoneCell { path: PathCell::Deferred, aa: AaCell::Smaa };
    let t = table(c);
    let exact: Vec<ZoneRow> =
        t.iter().filter(|r| r.expect == Expect::Required).map(|r| measured(r.zone)).collect();
    assert_eq!(check_expectations(&exact, &t), Ok(()));
    let missing: Vec<ZoneRow> =
        exact.iter().copied().filter(|r| r.zone != ZONE_POST_SMAA_WEIGHT).collect();
    let e = check_expectations(&missing, &t).expect_err("a missing Required zone must red");
    assert!(e.contains("84") && e.contains("absent"), "{e}");
    let mut extra = exact.clone();
    extra.push(measured(ZONE_POST_RETIRED[0]));
    let e = check_expectations(&extra, &t).expect_err("a Forbidden zone present must red");
    assert!(e.contains("81") && e.contains("Forbidden"), "{e}");
    let mut torn = exact;
    torn[0].label = ZoneLabel::Torn;
    assert!(check_expectations(&torn, &t).is_err(), "a non-Measured Required row must red");
}

/// **Every label child runs under the frame backstop** (triage r1 RF-1, r2 G3) — the hang cure is
/// the child's environment, so it is read off the `Command` every driver spawns, for every cell and
/// every driver mode: `BOYKO_WINDOW_FRAMES` is SET to [`FRAME_BACKSTOP`] and the exit window is
/// armed. `get_envs` reports the LAST `env` / `env_remove` per key, so this reds both when the
/// `.env` is deleted and when the strip list re-gains the variable (the base list and its cited
/// precedent both strip it). Every [`STRIPPED_KNOBS`] entry is removed.
#[test]
fn every_label_child_runs_under_the_frame_backstop() {
    let artifact = std::env::temp_dir().join("boyko_px0_zones_host_check.toml");
    for c in CELLS {
        for (validation_on, profile_on) in [(false, false), (true, false), (false, true)] {
            let cmd = worker_command(c, &artifact, "px0-zones-host-check", validation_on, profile_on);
            let env = |k: &str| {
                cmd.get_envs()
                    .find(|(n, _)| *n == std::ffi::OsStr::new(k))
                    .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
            };
            let at = format!("[{}, validation {validation_on}, profile {profile_on}]", c.word());
            assert_eq!(
                env("BOYKO_WINDOW_FRAMES"),
                Some(Some(FRAME_BACKSTOP.to_string())),
                "{at} the child must run under the frame backstop (Some(None) = removed, None = inherited)"
            );
            assert_eq!(env("BOYKO_VB_ZONE"), Some(Some("1".to_owned())), "{at} the zone window must be armed");
            for k in STRIPPED_KNOBS {
                assert_eq!(env(k), Some(None), "{at} the operator knob {k} must be removed");
            }
        }
    }
}

/// **The backstop's red names the cell's zones absent** (triage r1 RF-1): for every cell the
/// verdict a child stopped by the backstop gets names a Required zone as absent — so a path whose
/// frame witness never opens (Forward before PX0-2) reds GL-PX0-Z — and a cell that required
/// nothing would be caught here rather than pass silently. This checks the verdict's TEXT; that the
/// child is bounded at all is [`every_label_child_runs_under_the_frame_backstop`]'s.
#[test]
fn a_child_stopped_by_the_backstop_names_its_zones_absent() {
    for c in CELLS {
        let v = no_window_verdict(c);
        assert!(v.contains("absent") && v.contains(&c.word()) && v.contains("backstop"), "{v}");
    }
    let v = no_window_verdict(ZoneCell { path: PathCell::ForwardPlus, aa: AaCell::Off });
    assert!(v.contains(&format!("zone {ZONE_POST_PRESENT} ")), "the unwitnessed Forward path loses its present zone first: {v}");
}

/// The CPU-zone check and its line parser, both directions — the parser on the form the worker's
/// output really has: under `--nocapture` libtest has printed `test <name> ... ` with no newline
/// when the worker's `println!` runs, so the record is glued to that prefix
/// (`px0/gl/zones_af9146ee.log`, triage r1 F2), as well as on a line of its own.
#[test]
fn cpu_zone_check_and_parser() {
    let glued = "running 1 test\ntest post_zone_labels_worker ... PX0_CPU_ZONE calls=83 ticks=23945031 mean_ns=87639.5\nok\n";
    assert_eq!(parse_cpu_line(glued), Some((83, 23_945_031)), "the libtest-glued record must parse");
    assert_eq!(parse_cpu_line("x\nPX0_CPU_ZONE calls=70 ticks=12345 mean_ns=3.0\n"), Some((70, 12_345)));
    assert_eq!(parse_cpu_line("no line here"), None);
    assert_eq!(parse_cpu_line("test w ... PX0_CPU_ZONE mean_ns=1.0\n"), None, "a record without its fields is no record");
    assert!(check_cpu_zone(70, 61).is_ok());
    let e = check_cpu_zone(0, 61).expect_err("a zero count must red");
    assert!(e.contains("lifetime count 0"), "{e}");
    assert!(check_cpu_zone(60, 61).is_err(), "fewer closes than recorded frames must red");
}

/// **D-Z4's equality pin** — the CPU zone is declared with the literal `0` because
/// `boyko_rhi_vulkan` has no `boyko_ecs` edge; this crate has both, so the equality is checked here.
#[test]
fn cpu_zone_scope_is_the_kernel_root_scope() {
    assert_eq!(RENDER_DECLARE_FRAME_GRAPH::SCOPE, ROOT_SCOPE, "the CPU zone must sit on ROOT_SCOPE");
    assert_eq!(RENDER_DECLARE_FRAME_GRAPH.desc.name, "render.declare_frame_graph");
    assert_eq!(RENDER_DECLARE_FRAME_GRAPH::TIER, boyko_diag::profiling_abi::ZoneTier::Dev);
}

/// A mistyped cell is a named error, never a default boot.
#[test]
fn a_mistyped_cell_is_refused() {
    assert!(ZoneCell::parse("deferred:fxa").is_err());
    assert!(ZoneCell::parse("deferd:fxaa").is_err());
    assert!(ZoneCell::parse("deferred").is_err());
    assert!(ZoneCell::parse("forward_both:smaa").is_err());
}
