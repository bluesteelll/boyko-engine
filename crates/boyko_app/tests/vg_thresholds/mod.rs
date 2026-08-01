//! **VG-R0 — the frozen-threshold readers, the per-rung extent route, and the census-row parser.**
//!
//! Shared by R0c's gate (`vg_density_census.rs`) and R0d's (`vg_r0d_census.rs`). Both drive the
//! SAME ladder from the SAME frozen file and parse the SAME row format, and two copies of that
//! would be two texts that can disagree — which is the defect this campaign has spent its whole
//! history on, reached from the code side.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

use boyko_render::vg_census::Sha256;

/// The frozen thresholds file, relative to `crates/boyko_app`.
pub const THRESHOLDS: &str = "../../docs/VG-CAMPAIGN-THRESHOLDS.toml";

/// The sha256 R0a recorded. Re-asserted by every rung that DRIVES the ladder, because a ladder read
/// from an edited frozen file is not read from the frozen file.
pub const THRESHOLDS_SHA256: &str =
    "137379553feafa19217ce1b964f1663d3912815f12c8ebfd0ca14e94eedc41fa";

/// Repository-relative path resolved against this crate's manifest directory.
pub fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

pub fn read_thresholds() -> String {
    std::fs::read_to_string(repo_path(THRESHOLDS))
        .expect("invariant: the frozen thresholds file is in the repository")
}

/// Re-asserts the frozen file's digest.
pub fn assert_thresholds_frozen() {
    let mut h = Sha256::new();
    h.update(&std::fs::read(repo_path(THRESHOLDS)).expect("the frozen file is readable"));
    assert_eq!(
        h.finish_hex(),
        THRESHOLDS_SHA256,
        "docs/VG-CAMPAIGN-THRESHOLDS.toml has moved since R0a recorded its hash"
    );
}

/// Strips a `#` comment. Every value read here is a bare scalar or an array of numbers, never a
/// string containing `#`.
pub fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// The raw right-hand side of `table.key` in `src`.
pub fn field(src: &str, path: &str) -> String {
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

pub fn field_u64(src: &str, path: &str) -> u64 {
    field(src, path).parse().unwrap_or_else(|_| panic!("thresholds: `{path}` is not an integer"))
}

pub fn field_f64(src: &str, path: &str) -> f64 {
    field(src, path).parse().unwrap_or_else(|_| panic!("thresholds: `{path}` is not a number"))
}

pub fn field_bool(src: &str, path: &str) -> bool {
    field(src, path).parse().unwrap_or_else(|_| panic!("thresholds: `{path}` is not a bool"))
}

pub fn field_str(src: &str, path: &str) -> String {
    field(src, path).trim_matches('"').to_string()
}

/// `[census].resolution_ladder` as `(width, height)` pairs.
pub fn resolution_ladder(src: &str) -> Vec<(u32, u32)> {
    let raw = field(src, "census.resolution_ladder");
    let inner = raw
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .expect("thresholds: resolution_ladder is an array");
    let mut out = Vec::new();
    let mut cur: Vec<u32> = Vec::new();
    for tok in inner.split(['[', ']', ',']) {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        cur.push(t.parse().expect("thresholds: a ladder rung is two integers"));
        if cur.len() == 2 {
            out.push((cur[0], cur[1]));
            cur.clear();
        }
    }
    assert!(cur.is_empty(), "thresholds: a ladder rung is missing a component");
    out
}

/// The index of `[census].decision_resolution` in the ladder.
pub fn decision_rung(src: &str) -> usize {
    let raw = field(src, "census.decision_resolution");
    let mut parts = raw
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().parse::<u32>().expect("decision_resolution is two integers"));
    let want = (
        parts.next().expect("decision_resolution width"),
        parts.next().expect("decision_resolution height"),
    );
    resolution_ladder(src)
        .iter()
        .position(|r| *r == want)
        .expect("thresholds: decision_resolution must BE a ladder rung, or D_est's denominator is unmeasured")
}

/// The window client extent and SSAA scale that reach `rung` on this box.
///
/// From the plan's §9.1 table, MEASURED by `vg_extent_probe.rs`: 512², 1280×720 and 1920×1080
/// clients are granted exactly, while 2560×1440 and 3840×2160 are clamped to a 1133-pixel client
/// height. So the top two rungs ride the armed 2× SSAA composite instead of asking the OS for a
/// window it will refuse.
///
/// Returns `None` for a rung no route reaches — an instrument failure the gate reds on, with this
/// table as the diagnosis, rather than a silently substituted extent.
pub fn route_for(rung: (u32, u32)) -> Option<(u32, u32, u32)> {
    let (w, h) = rung;
    if matches!((w, h), (512, 512) | (1920, 1080)) {
        return Some((w, h, 1));
    }
    // Composite routes, tried finest-scale-first so a rung reachable at 2x never takes 4x.
    for scale in [2u32, 4u32] {
        if w.is_multiple_of(scale) && h.is_multiple_of(scale) {
            let (cw, ch) = (w / scale, h / scale);
            if matches!((cw, ch), (1280, 720) | (960, 540) | (1920, 1080) | (256, 256)) {
                return Some((cw, ch, scale));
            }
        }
    }
    None
}

/// One parsed census row (`boyko_app::vg_census_dump`'s TOML).
#[derive(Debug, Clone)]
pub struct Row {
    pub achieved: (u32, u32),
    pub native: (u32, u32),
    pub ssaa_armed: bool,
    pub ssaa_scale: u32,
    pub vb_mesh_leg: bool,
    pub covered_pixels: u64,
    pub visible_tris: u64,
    pub modal_bucket: Option<u32>,
    pub histogram: Vec<u64>,
    pub submitted_tris: u64,
    pub readback_sha256: String,
}

impl Row {
    pub fn visible_tri_per_covered_pixel(&self) -> f64 {
        if self.covered_pixels == 0 {
            return 0.0;
        }
        self.visible_tris as f64 / self.covered_pixels as f64
    }

    pub fn submitted_per_covered_pixel(&self) -> f64 {
        if self.covered_pixels == 0 {
            return 0.0;
        }
        self.submitted_tris as f64 / self.covered_pixels as f64
    }
}

pub fn parse_row(src: &str) -> Row {
    let histogram = field(src, "row.histogram")
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("a histogram entry is an integer"))
        .collect();
    Row {
        achieved: (
            field_u64(src, "extent.achieved_width") as u32,
            field_u64(src, "extent.achieved_height") as u32,
        ),
        native: (
            field_u64(src, "extent.native_width") as u32,
            field_u64(src, "extent.native_height") as u32,
        ),
        ssaa_armed: field_bool(src, "extent.ssaa_armed"),
        ssaa_scale: field_u64(src, "extent.ssaa_scale") as u32,
        vb_mesh_leg: field_bool(src, "extent.vb_mesh_leg"),
        covered_pixels: field_u64(src, "row.covered_pixels"),
        visible_tris: field_u64(src, "row.visible_tris"),
        // An absent mode is written as a COMMENT by the producer, never as a sentinel index, so a
        // missing key here means "no visible triangle" rather than "bucket 0".
        modal_bucket: src.lines().map(|l| strip_comment(l).trim()).find_map(|l| {
            l.strip_prefix("modal_bucket =").map(|v| v.trim().parse().expect("a bucket index"))
        }),
        histogram,
        submitted_tris: field_u64(src, "row.submitted_tris"),
        readback_sha256: field_str(src, "readback.sha256"),
    }
}

/// Spawns one census worker process and returns its row.
///
/// `worker` is the `#[ignore]`d test in the CALLING binary that boots the app; `env` is what tells
/// that worker which scene and rung it is. One process per row is not incidental — it is what makes
/// the achieved extent a measurement (each rung negotiates its own window with the OS) and what
/// gives R0d(a) genuinely separate processes to compare.
pub fn run_worker(worker: &str, tag: &str, env: &[(&str, String)]) -> Row {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let out = std::env::temp_dir().join(format!("vg_census_{tag}.toml"));
    let _ = std::fs::remove_file(&out);

    let mut cmd = Command::new(&exe);
    cmd.args([worker, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env("BOYKO_VG_CENSUS", &out)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // The census is the only capture this run is for; a second one would render the same frames
        // for no reason and, worse, make the run's exit depend on which fired first.
        .env_remove("BOYKO_HOST_DUMP");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.status().expect("invariant: the worker process spawns");
    assert!(status.success(), "census worker `{tag}` exited {status}");

    let text = std::fs::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "census worker `{tag}` wrote no row at {}: {e}. A worker that renders and produces \
             nothing is an instrument failure, not an empty scene.",
            out.display()
        )
    });
    parse_row(&text)
}
