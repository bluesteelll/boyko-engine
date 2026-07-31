//! **VG-R0 rung R0c — the density census's gate** (plan §8 R0c).
//!
//! Five parts, and each is evaluated on the thing it names:
//!
//! * **(a)** every VB image golden byte-identical with the census UNARMED. Measured OUTSIDE cargo,
//!   by `scripts/golden.ps1` over the VB pins, because a pin is a GPU render against a blessed
//!   hash and no cargo test drives that. What IS machine-checked here is (a)'s DOMAIN —
//!   [`the_a_domain_is_exactly_the_vb_pins_that_were_measured`] pins the enumeration, so a
//!   fourteenth VB pin reds until it has been measured too. The distinction is the point: this
//!   file cannot claim to run (a), and a gate whose domain drifts silently is the vacuous-selection
//!   defect.
//! * **(b)** the census's modal bucket IS the procedural fixture's analytic bucket; its red is a
//!   4x subdivision, which must move the mode down by EXACTLY two.
//! * **(c)** the census's covered-pixel total agrees with `sv0_oracle::rasterize`'s `covered_count`
//!   on that same fixture at 512², within `[pre_registered].r0c_oracle_coverage_tolerance` read
//!   from the frozen file BY NAME.
//! * **(c′)** the non-degeneracy floors `[k1_instrument].min_covered_pixels` /
//!   `min_visible_tris` hold on the censused frame.
//! * **(d)** the ladder is driven from `[census].resolution_ladder`, one row per rung, and the
//!   readback's own dimensions equal the requested rung (`[census].assert_achieved_extent`).
//!
//! # One process per rung, and why
//!
//! The census is armed by env and the host loop exits when it fires, so a rung IS a process. The
//! driver re-executes THIS binary's worker test once per (fixture, rung), which is also what makes
//! the achieved extent an honest measurement: each rung negotiates its own window with the OS.
//!
//! Windowed-test conventions: `#[ignore]`, `BOYKO_DISABLE_VALIDATION=1`, `--test-threads=1`.

#![cfg(windows)]

use std::path::PathBuf;
use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::vg_census::Sha256;
use boyko_render::{GeometryLegs, Material, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig};
use boyko_scene::ViewUniform;

mod sv0_oracle;
mod vg_fixture;

use sv0_oracle::OracleVertex;
use vg_fixture::Fixture;

/// The frozen thresholds file every value below is read from BY NAME.
const THRESHOLDS: &str = "../../docs/VG-CAMPAIGN-THRESHOLDS.toml";

/// The sha256 gate (d) re-asserts — the value R0a recorded, mirrored here rather than re-derived.
/// A drift reds this rung as well as the tripwire, which is the whole point of a frozen file.
const THRESHOLDS_SHA256: &str = "fea8f9c5583fafd7de39360974d1a52d41f656ce7fd961282a145620a16a2d19";

/// The env knob the worker reads to know which ladder rung it is.
const ENV_RUNG: &str = "BOYKO_VG_RUNG";
/// The env knob the worker reads to know which fixture parameterisation to spawn.
const ENV_FIXTURE: &str = "BOYKO_VG_FIXTURE";

/// The VB golden pins (a) is measured over. Enumerated so the DOMAIN is machine-checked even
/// though the hashes are not checked here.
const VB_PINS: [&str; 13] = [
    "vb_mesh",
    "vb_both",
    "vb_both_sdf",
    "vb_both_sdf_tex",
    "vb_sdf_only",
    "vb_mesh_tex",
    "vb_taa",
    "vb_taa_rcas",
    "vb_mesh_ssao",
    "vb_mesh_froxel",
    "vb_mesh_tex_froxel",
    "vb_both_taa",
    "vb_sdf_taa",
];

// ===============================================================================================
// The frozen file
// ===============================================================================================

/// Repository-relative path resolved against this crate's manifest directory.
fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read_thresholds() -> String {
    std::fs::read_to_string(repo_path(THRESHOLDS))
        .expect("invariant: the frozen thresholds file is in the repository")
}

/// Strips a `#` comment, respecting nothing else — every value this file reads is a bare scalar or
/// an array of numbers, never a string containing `#`.
fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// The raw right-hand side of `table.key` in `src`.
fn field(src: &str, path: &str) -> String {
    let (table, key) = path.split_once('.').expect("a threshold path is `table.key`");
    let mut inside = false;
    for line in src.lines() {
        let l = strip_comment(line).trim();
        if l.starts_with('[') && l.ends_with(']') {
            inside = l.trim_start_matches('[').trim_end_matches(']') == table;
            continue;
        }
        if inside
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            return v.trim().to_string();
        }
    }
    panic!("thresholds: `{path}` is absent -- a gate that reads a missing field asserts nothing");
}

fn field_u64(src: &str, path: &str) -> u64 {
    field(src, path).parse().unwrap_or_else(|_| panic!("thresholds: `{path}` is not an integer"))
}

fn field_f64(src: &str, path: &str) -> f64 {
    field(src, path).parse().unwrap_or_else(|_| panic!("thresholds: `{path}` is not a number"))
}

fn field_bool(src: &str, path: &str) -> bool {
    field(src, path).parse().unwrap_or_else(|_| panic!("thresholds: `{path}` is not a bool"))
}

/// `[census].resolution_ladder` as `(width, height)` pairs.
fn resolution_ladder(src: &str) -> Vec<(u32, u32)> {
    let raw = field(src, "census.resolution_ladder");
    let inner = raw
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .expect("thresholds: resolution_ladder is an array");
    let mut out = Vec::new();
    let mut cur: Vec<u32> = Vec::new();
    let mut open = false;
    for tok in inner.split(['[', ']', ',']) {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        cur.push(t.parse().expect("thresholds: a ladder rung is two integers"));
        open = true;
        if cur.len() == 2 {
            out.push((cur[0], cur[1]));
            cur.clear();
            open = false;
        }
    }
    assert!(!open && cur.is_empty(), "thresholds: a ladder rung is missing a component");
    out
}

// ===============================================================================================
// The per-rung extent route (plan §9.1's MEASURED grant table)
// ===============================================================================================

/// The window client extent and SSAA scale that reach `rung` on this box.
///
/// From the plan's §9.1 table, MEASURED by `vg_extent_probe.rs`: 512², 1280×720 and 1920×1080
/// clients are granted exactly, while 2560×1440 and 3840×2160 are clamped to a 1133-pixel client
/// height. So the top two rungs ride the armed 2× SSAA composite instead of asking the OS for a
/// window it will refuse.
///
/// Returns `None` for a rung no route reaches — an instrument failure (d) reds on, with this table
/// as the diagnosis, rather than a silently substituted extent.
fn route_for(rung: (u32, u32)) -> Option<(u32, u32, u32)> {
    let (w, h) = rung;
    // Direct: the client area IS the composite.
    if matches!((w, h), (512, 512) | (1920, 1080)) {
        return Some((w, h, 1));
    }
    // Via the 2× composite: request half, assert the doubling actually armed.
    if w.is_multiple_of(2) && h.is_multiple_of(2) {
        let (cw, ch) = (w / 2, h / 2);
        if matches!((cw, ch), (1280, 720) | (960, 540) | (1920, 1080) | (256, 256)) {
            return Some((cw, ch, 2));
        }
    }
    None
}

// ===============================================================================================
// The census row a worker process writes
// ===============================================================================================

/// One parsed census row (`vg_census_dump`'s TOML).
#[derive(Debug, Clone)]
struct Row {
    achieved: (u32, u32),
    native: (u32, u32),
    ssaa_armed: bool,
    vb_mesh_leg: bool,
    covered_pixels: u64,
    visible_tris: u64,
    modal_bucket: Option<u32>,
    histogram: Vec<u64>,
    submitted_tris: u64,
    readback_sha256: String,
}

fn parse_row(src: &str) -> Row {
    let histogram = {
        let raw = field(src, "row.histogram");
        raw.trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().expect("a histogram entry is an integer"))
            .collect()
    };
    Row {
        achieved: (field_u64(src, "extent.achieved_width") as u32, field_u64(src, "extent.achieved_height") as u32),
        native: (field_u64(src, "extent.native_width") as u32, field_u64(src, "extent.native_height") as u32),
        ssaa_armed: field_bool(src, "extent.ssaa_armed"),
        vb_mesh_leg: field_bool(src, "extent.vb_mesh_leg"),
        covered_pixels: field_u64(src, "row.covered_pixels"),
        visible_tris: field_u64(src, "row.visible_tris"),
        // An absent mode is written as a COMMENT by the producer, never as a sentinel index.
        modal_bucket: src
            .lines()
            .map(|l| strip_comment(l).trim())
            .find_map(|l| l.strip_prefix("modal_bucket =").map(|v| v.trim().parse().expect("a bucket index")))
            ,
        histogram,
        submitted_tris: field_u64(src, "row.submitted_tris"),
        readback_sha256: field(src, "readback.sha256").trim_matches('"').to_string(),
    }
}

/// Runs one worker process and returns its census row.
fn run_rung(fixture: &str, rung_index: usize) -> Row {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let out = std::env::temp_dir().join(format!("vg_census_{fixture}_{rung_index}.toml"));
    let _ = std::fs::remove_file(&out);

    let status = Command::new(&exe)
        .args(["vg_census_rung_dump", "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(ENV_FIXTURE, fixture)
        .env(ENV_RUNG, rung_index.to_string())
        .env("BOYKO_VG_CENSUS", &out)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // The worker must not also arm the frame dump: the census is the only capture this run is
        // for, and a second one would render the same frames for no reason.
        .env_remove("BOYKO_HOST_DUMP")
        .status()
        .expect("invariant: the worker process spawns");
    assert!(status.success(), "census worker ({fixture}, rung {rung_index}) exited {status}");

    let text = std::fs::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "census worker ({fixture}, rung {rung_index}) wrote no row at {}: {e}. A worker that \
             renders and produces nothing is an instrument failure, not an empty scene.",
            out.display()
        )
    });
    parse_row(&text)
}

// ===============================================================================================
// The worker: one process, one rung, one row
// ===============================================================================================

fn fixture_by_name(name: &str) -> Fixture {
    match name {
        "base" => vg_fixture::BASE,
        "subdivided" => vg_fixture::SUBDIVIDED,
        "starved" => vg_fixture::STARVED,
        other => panic!("unknown census fixture `{other}`"),
    }
}

fn camera_transform() -> Transform {
    let pose = Affine3A::look_at_rh(
        Vec3::new(0.0, 0.0, vg_fixture::CAMERA_DISTANCE),
        Vec3::ZERO,
        Vec3::new(0.0, 1.0, 0.0),
    );
    Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE }
}

fn camera_projection() -> Projection {
    Projection::Perspective { fov_y: vg_fixture::FOV_Y, aspect: 1.0, near: 0.1, far: 100.0 }
}

fn setup_fixture(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let f = fixture_by_name(&std::env::var(ENV_FIXTURE).expect("the worker is told its fixture"));
    let (verts, idx) = f.mesh();
    let mesh = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };
    let mat = materials.add(Material::new([0.75, 0.75, 0.78, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0));
    let e = commands.spawn(MeshBundle::new(mesh, Transform::IDENTITY)).id();
    commands.entity(e).insert(MaterialHandle(mat.index() as u16));

    // Lighting exists so the frame is a normal VB frame; `vb_id` is unaffected by it either way.
    let sun = [-0.40, 0.78, 0.48];
    let sun_pose =
        Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(sun[0], sun[1], sun[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(sun, [1.0, 0.97, 0.92], 3.1),
    });
    commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));

    commands.spawn(CameraRig {
        transform: camera_transform(),
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: camera_projection(),
    });
}

/// **The census WORKER** — one process, one `(fixture, ladder rung)` pair, one census row.
///
/// Driven entirely by env so the driver can spawn it; run directly it needs [`ENV_FIXTURE`],
/// [`ENV_RUNG`] and `BOYKO_VG_CENSUS`.
#[test]
#[ignore = "needs a real windowed GPU device; the census driver spawns it once per (fixture, ladder rung)"]
fn vg_census_rung_dump() {
    let src = read_thresholds();
    let ladder = resolution_ladder(&src);
    let rung_index: usize = std::env::var(ENV_RUNG)
        .expect("the worker is told its rung")
        .parse()
        .expect("the rung index is an integer");
    let rung = ladder[rung_index];
    let (cw, ch, ssaa) = route_for(rung).unwrap_or_else(|| {
        panic!(
            "no route on this box reaches ladder rung {rung:?} -- see the plan's §9.1 grant table; \
             this is an instrument failure, and substituting a reachable extent would fabricate \
             the curve the census exists to measure"
        )
    });

    let mut app = App::new();
    let mut plugins = EnginePlugins::window("boyko_engine vg density census", cw, ch);
    if ssaa > 1 {
        plugins = plugins.with_ssaa_scale(ssaa);
    }
    app.add_plugins(plugins);
    app.add_startup_system(setup_fixture);
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
    app.run();
}

// ===============================================================================================
// (a)'s domain — machine-checked here; the hashes are measured by scripts/golden.ps1
// ===============================================================================================

/// R0c gate (a)'s DOMAIN. The hashes are re-measured outside cargo; what this asserts is that the
/// set of VB pins has not grown or shrunk since they were, so "all VB goldens byte-identical" keeps
/// meaning what it meant when it was measured.
#[test]
fn the_a_domain_is_exactly_the_vb_pins_that_were_measured() {
    let pins = std::fs::read_to_string(repo_path("../../goldens/PINS.toml"))
        .expect("invariant: goldens/PINS.toml is in the repository");
    let mut found: Vec<String> = pins
        .lines()
        .map(|l| strip_comment(l).trim())
        .filter_map(|l| l.strip_prefix('[').and_then(|s| s.strip_suffix(']')))
        // `[<pin>.env]` sub-tables are not pins.
        .filter(|s| !s.contains('.'))
        .filter(|s| s.starts_with("vb"))
        .map(str::to_string)
        .collect();
    found.sort();
    let mut want: Vec<String> = VB_PINS.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(
        found, want,
        "the VB pin set has changed. R0c gate (a) was MEASURED over the pins listed in VB_PINS \
         (all byte-identical with the census unarmed); a pin added or removed since must be \
         re-measured with scripts/golden.ps1 and this list updated in the same act."
    );
}

/// The thresholds file gate (d) reads is the file R0a froze — re-asserted here as well as in the
/// tripwire, because a ladder driven from an EDITED frozen file is not driven from the frozen file.
#[test]
fn the_thresholds_file_is_the_one_r0a_froze() {
    let mut h = Sha256::new();
    h.update(&std::fs::read(repo_path(THRESHOLDS)).expect("the frozen file is readable"));
    assert_eq!(
        h.finish_hex(),
        THRESHOLDS_SHA256,
        "docs/VG-CAMPAIGN-THRESHOLDS.toml has moved since R0a recorded its hash"
    );
}

/// The ladder this rung drives is the frozen one, read by name and non-empty — the half of (d)
/// that needs no GPU, so a truncated or malformed ladder reds in a plain `cargo test`.
#[test]
fn the_ladder_is_read_from_the_frozen_file_and_every_rung_has_a_route() {
    let src = read_thresholds();
    let ladder = resolution_ladder(&src);
    assert_eq!(ladder, vec![(512, 512), (1920, 1080), (2560, 1440), (3840, 2160)]);
    assert!(field_bool(&src, "census.assert_achieved_extent"));
    for (i, rung) in ladder.iter().enumerate() {
        let (cw, ch, ssaa) = route_for(*rung).unwrap_or_else(|| {
            panic!("ladder rung {i} {rung:?} has no route in §9.1's measured grant table")
        });
        assert_eq!(
            (cw * ssaa, ch * ssaa),
            *rung,
            "rung {i}'s route must COMPOSE to the rung, or the census measures a different \
             per-pixel workload than the one it reports"
        );
    }
}

/// The fixture's own arithmetic is gate (b)'s content, so it is checked without a device too.
#[test]
fn the_fixture_arithmetic_is_what_gate_b_compares_against() {
    assert_eq!(vg_fixture::BASE.analytic_bucket(), 5);
    assert_eq!(vg_fixture::SUBDIVIDED.analytic_bucket(), 3);
    assert_eq!(
        vg_fixture::BASE.analytic_bucket() - vg_fixture::SUBDIVIDED.analytic_bucket(),
        2,
        "gate (b)'s red mutation is a TWO-bucket move; a control asserting only that the number \
         changed is not a gate"
    );
    let src = read_thresholds();
    let min_tris = field_u64(&src, "k1_instrument.min_visible_tris");
    let min_px = field_u64(&src, "k1_instrument.min_covered_pixels");
    assert!(u64::from(vg_fixture::BASE.triangle_count()) >= min_tris);
    assert!((u64::from(vg_fixture::STARVED.triangle_count())) < min_tris);
    assert!(
        u64::from(vg_fixture::BASE.triangle_count()) * vg_fixture::BASE.analytic_pixels() >= min_px
    );
}

// ===============================================================================================
// The oracle cross-check — pure CPU, so it runs without the census
// ===============================================================================================

/// `sv0_oracle::rasterize`'s covered count for `f` at 512², under the SAME projection the VB raster
/// uploads (`forward_view_proj_rows`, the engine's own construction site).
fn oracle_covered(f: Fixture) -> usize {
    let (verts, idx) = f.mesh();
    let oracle_verts: Vec<OracleVertex> = verts
        .iter()
        .map(|v| OracleVertex { position: v.position, normal: v.normal })
        .collect();
    let view = ViewUniform::from_camera(camera_transform().to_affine(), camera_projection());
    let rows = boyko_render::forward_view_proj_rows(
        &view,
        vg_fixture::REFERENCE_EXTENT,
        vg_fixture::REFERENCE_EXTENT,
    );
    sv0_oracle::rasterize(
        &oracle_verts,
        &idx,
        &[[0.0, 0.0, 0.0]],
        rows,
        vg_fixture::REFERENCE_EXTENT,
        vg_fixture::REFERENCE_EXTENT,
        0.1,
    )
    .covered_count()
}

/// The oracle reproduces the fixture's ANALYTIC coverage EXACTLY, before it is ever asked to agree
/// with the GPU. Without this, gate (c) would compare two numbers whose only common ancestor is the
/// fixture — and a fixture bug would show up as agreement.
///
/// Exact equality rather than a band: [`vg_fixture::SUBPIXEL_OFFSET_PX`] leaves no pixel centre on
/// any edge, so there is no fill rule left for the two rasterisers to disagree about. The first
/// draft of this fixture had no offset and the two landed on opposite ends of the band — an 18.2%
/// disagreement — which is why this assertion is an equality now.
#[test]
fn the_oracle_reproduces_the_fixture_analytic_coverage() {
    for f in [vg_fixture::BASE, vg_fixture::SUBDIVIDED, vg_fixture::STARVED] {
        let covered = oracle_covered(f) as u64;
        let want = u64::from(f.triangle_count()) * f.analytic_pixels();
        assert_eq!(
            covered, want,
            "oracle covered {covered} px against the fixture's analytic {want} (leg_px={})",
            f.leg_px
        );
    }
}

// ===============================================================================================
// The gate
// ===============================================================================================

/// **R0c's gate, parts (b), (c), (c′) and (d).**
///
/// Part (a) is measured by `scripts/golden.ps1` and its domain is pinned by
/// [`the_a_domain_is_exactly_the_vb_pins_that_were_measured`]; part (e) is R0d's, measured on the
/// corpus at the top rung where it can actually fail.
#[test]
#[ignore = "needs a real windowed GPU device; drives one worker process per ladder rung"]
fn vg_density_census_gate() {
    let src = read_thresholds();
    let ladder = resolution_ladder(&src);
    let tolerance = field_f64(&src, "pre_registered.r0c_oracle_coverage_tolerance");
    let min_px = field_u64(&src, "k1_instrument.min_covered_pixels");
    let min_tris = field_u64(&src, "k1_instrument.min_visible_tris");

    // ---- (d): one row per rung, at the rung's own extent -------------------------------------
    let mut rows = Vec::with_capacity(ladder.len());
    for (i, rung) in ladder.iter().enumerate() {
        let row = run_rung("base", i);
        let (_, _, ssaa) = route_for(*rung).expect("every rung has a route (checked without a GPU)");
        assert_eq!(
            row.achieved, *rung,
            "(d) rung {i}: the readback is {:?} but the rung is {rung:?}. `assert_achieved_extent` \
             exists because an OS-clamped window silently measures a different per-pixel workload.",
            row.achieved
        );
        assert_eq!(
            row.ssaa_armed,
            ssaa > 1,
            "(d) rung {i}: SSAA arming is ASSERTED, not trusted -- the probe degrades to Off \
             silently on a caps or VRAM miss, and this box routes the top two rungs through the \
             armed composite"
        );
        assert!(row.vb_mesh_leg, "(d) rung {i}: the census frame must carry a VB mesh leg");
        rows.push(row);
    }
    assert_eq!(rows.len(), ladder.len(), "(d): one census row per rung");

    let rung0 = &rows[0];

    // ---- (c′): non-degeneracy on the censused frame -------------------------------------------
    for (i, row) in rows.iter().enumerate() {
        assert!(
            row.covered_pixels >= min_px && row.visible_tris >= min_tris,
            "(c′) rung {i}: covered={} (floor {min_px}), visible_tris={} (floor {min_tris}). \
             D_est and the convergence check are both DIVISIONS; a frame that cannot be \
             adjudicated must be refused, not divided by.",
            row.covered_pixels,
            row.visible_tris
        );
    }

    // ---- (b): the modal bucket IS the analytic bucket -----------------------------------------
    assert_eq!(
        rung0.modal_bucket,
        Some(vg_fixture::BASE.analytic_bucket()),
        "(b): the census's mode must be the fixture's analytic bucket; histogram = {:?}",
        rung0.histogram
    );
    // ...and every triangle is accounted for: the fixture emits isolated triangles, so the census
    // must see exactly as many as it spawned.
    assert_eq!(
        rung0.visible_tris,
        u64::from(vg_fixture::BASE.triangle_count()),
        "(b): the fixture's triangles are isolated and fully on screen, so none may go missing"
    );

    // ---- (b)'s RED MUTATION: 4x subdivision moves the mode down by exactly two -----------------
    let sub = run_rung("subdivided", 0);
    assert_eq!(
        sub.modal_bucket,
        Some(vg_fixture::SUBDIVIDED.analytic_bucket()),
        "(b) mutation: the subdivided fixture's mode must be its own analytic bucket"
    );
    let shift = rung0.modal_bucket.expect("(b) base mode") - sub.modal_bucket.expect("(b) sub mode");
    assert_eq!(
        shift, 2,
        "(b) mutation: a 4x subdivision must move the mode down by EXACTLY two buckets, measured \
         {shift}"
    );

    // ---- (c): the census's covered total agrees with the CPU oracle ---------------------------
    let oracle = oracle_covered(vg_fixture::BASE) as f64;
    let census = rung0.covered_pixels as f64;
    let rel = (census - oracle).abs() / oracle;
    assert!(
        rel <= tolerance,
        "(c): census covered {census} vs oracle {oracle} -- relative disagreement {rel:.5} exceeds \
         [pre_registered].r0c_oracle_coverage_tolerance = {tolerance}"
    );

    // ---- (c)'s RED MUTATION: pair (b) and (c), which is what proves (c) is not self-referential.
    // Feeding the reducer the oracle's own coverage would pass (c) vacuously while (b) failed; the
    // pairing above is that mutation's standing form -- (b) reads the census's histogram and (c)
    // reads its covered total, both from the SAME row, so a census replaced by the oracle would
    // agree on (c) and lose the histogram (b) needs.
    assert!(
        !rung0.histogram.is_empty(),
        "(c) is only meaningful paired with (b): a covered-pixel total with no histogram behind it \
         could have come from the oracle itself"
    );

    // ---- (c′)'s RED MUTATION: the starved fixture ---------------------------------------------
    let starved = run_rung("starved", 0);
    assert!(
        starved.visible_tris < min_tris,
        "(c′) mutation: the starved fixture must fall BELOW the triangle floor, measured {}",
        starved.visible_tris
    );
    assert_eq!(
        starved.modal_bucket,
        Some(vg_fixture::STARVED.analytic_bucket()),
        "(c′) mutation must ISOLATE: the triangle SIZE is untouched, so (b) stays green"
    );

    // ---- The record -------------------------------------------------------------------------
    eprintln!("VG-R0c census ladder:");
    for (i, r) in rows.iter().enumerate() {
        eprintln!(
            "  rung {i} {:?}x{:?}  covered={} visible_tris={} mode={:?} submitted={} sha256={}",
            r.achieved.0,
            r.achieved.1,
            r.covered_pixels,
            r.visible_tris,
            r.modal_bucket,
            r.submitted_tris,
            &r.readback_sha256[..16]
        );
        assert_eq!(r.native.0 * if r.ssaa_armed { 2 } else { 1 }, r.achieved.0);
    }
}
