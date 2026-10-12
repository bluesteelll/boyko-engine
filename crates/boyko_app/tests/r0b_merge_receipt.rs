//! **RD1 structure (R) — the SDF depth merge in the production frame-graph receipt**
//! (render wave RD1, defect R0b, device test 5).
//!
//! The drivers boot [`r0b_receipt_worker`] (the `common/r0b_scene.rs` scene) with
//! `BOYKO_GRAPH_RECEIPT` armed and read OBS's receipt — the declared passes, their accesses and
//! derived barriers, and the `[config]` the declarator and the recorder both read:
//!
//! - **every declarator** (`_deferred`, `_forward`, `_forwardplus`, `_vb`): `sdf_depth_merge = 1`
//!   (Draw), the boot's `transparent_depth_capability` and the frame's set carry the particle bit,
//!   and the merge is declared IMMEDIATELY before `particle_draw` (after every opaque / SDF
//!   producer, every compute depth reader and both HZB slots). Forward / ForwardPlus also declare
//!   the `viewt` resource and the marcher writes it (`path_sdf_forward_writes_viewt = 1` without
//!   TAA);
//! - **VisibilityBuffer with both legs and the HZB armed**: the merge's index is greater than every
//!   `hzb_*` pass's, and it is the LAST writer of `vb_depth` before `particle_draw`;
//! - **VisibilityBuffer × Sdf**: the merge is the FIRST access of `vb_depth`, its derived barrier
//!   leaves `UNDEFINED`, and `sdf_depth_merge_clear = 1`;
//! - **an HZB-dump frame** (`BOYKO_HZB_DUMP` beside a receipt burst): on the frame that declares the
//!   dump the merge is absent (and the run does not panic — the guard is exempt by name); every
//!   other frame of the burst declares it;
//! - **a default boot** (`EnginePlugins`, no consumer armed) on Forward × Sdf and VB × Sdf without
//!   TAA: capability 0 and `path_sdf_forward_writes_viewt = 0`; the same world with particles
//!   armed: 1 and 1.
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`,
//! `BOYKO_WINDOW_FRAMES` set (each driver also sets a per-worker backstop).

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/r0b_scene.rs"]
mod r0b_scene;

use boyko_render::{GeometryLegs, RenderPath};

use graph_receipt::{Receipt, fresh_out, run_worker};
use r0b_scene::{
    ENV_EMIT, ENV_HZB, ENV_LEGS, ENV_PARTICLES, ENV_PATH, ENV_TAA, ENV_TEST_FORCE, R0bArming, WIN,
    assert_r0b_resolved, build_r0b_app,
};

/// The marker the drivers set.
const ENV_WORKER: &str = "BOYKO_R0B_RECEIPT_WORKER";
/// The settle window of a single-frame receipt.
const SETTLE: u32 = 20;

/// `VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT`.
const DS_WRITE: u64 = 0x0000_0400;
/// `VK_ACCESS_SHADER_WRITE_BIT`.
const SHADER_WRITE: u64 = 0x0000_0040;
/// `VK_IMAGE_LAYOUT_UNDEFINED`.
const UNDEFINED: u64 = 0;

/// **The receipt WORKER** — the R0b scene with the receipt (and, on one driver, the HZB dump) armed
/// by its driver. Booted without `BOYKO_GRAPH_RECEIPT` it SKIPS.
#[test]
#[ignore = "gpu-windowed: the RD1 receipt worker, spawned by the r0b_receipt_* drivers with BOYKO_GRAPH_RECEIPT set; run with --test-threads=1"]
fn r0b_receipt_worker() {
    if std::env::var_os(ENV_WORKER).is_none() || std::env::var_os("BOYKO_GRAPH_RECEIPT").is_none() {
        eprintln!("r0b_receipt_worker: {ENV_WORKER} / BOYKO_GRAPH_RECEIPT unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let arming = R0bArming::from_env();
    let mut app = build_r0b_app("boyko_app RD1 receipt worker", arming, WIN, WIN);
    app.run();
    assert_r0b_resolved(&app, arming);
}

/// One worker configuration.
#[derive(Clone, Copy)]
struct Cfg {
    path: RenderPath,
    legs: GeometryLegs,
    particles: bool,
    taa: bool,
    hzb: bool,
}

impl Cfg {
    const fn new(path: RenderPath, legs: GeometryLegs) -> Self {
        Self { path, legs, particles: true, taa: false, hzb: false }
    }
}

/// Boots the worker with `cfg` (+ `extra` env) and returns its receipt (`frames` records after
/// `settle` frames).
fn receipt_of(cfg: Cfg, frames: u32, settle: u32, extra: &[(&str, String)], tag: &str) -> Receipt {
    let out = fresh_out(&format!(
        "r0b_receipt_{}_{}_{tag}.toml",
        R0bArming::path_word(cfg.path),
        R0bArming::legs_word(cfg.legs)
    ));
    let flag = |b: bool| if b { "1" } else { "0" }.to_owned();
    let mut env = vec![
        (ENV_WORKER, "1".to_owned()),
        (ENV_PATH, R0bArming::path_word(cfg.path).to_owned()),
        (ENV_LEGS, R0bArming::legs_word(cfg.legs).to_owned()),
        (ENV_PARTICLES, flag(cfg.particles)),
        (ENV_EMIT, "both".to_owned()),
        (ENV_TAA, flag(cfg.taa)),
        (ENV_HZB, flag(cfg.hzb)),
        (ENV_TEST_FORCE, String::new()),
        ("BOYKO_GRAPH_RECEIPT", out.display().to_string()),
        ("BOYKO_GRAPH_RECEIPT_FRAMES", frames.to_string()),
        ("BOYKO_GRAPH_RECEIPT_SETTLE", settle.to_string()),
        ("BOYKO_WINDOW_FRAMES", (settle + frames + 40).to_string()),
    ];
    env.extend(extra.iter().map(|(k, v)| (*k, v.clone())));
    run_worker("r0b_receipt_worker", &env, false);
    Receipt::load(&out)
}

/// `[config]`'s value of an RD1 key, or a RED naming the key — at RD1's parent these keys do not
/// exist, and "absent" must not read as 0.
fn rd1_config(r: &Receipt, key: &str) -> u64 {
    let v = r.single("config").get(key).unwrap_or_else(|| {
        panic!("[config] key `{key}` absent — this receipt predates RD1's keys (no SDF depth merge in the build)")
    });
    v.parse().unwrap_or_else(|e| panic!("[config] `{key} = {v}` is not an integer ({e})"))
}

/// The index of the declared pass `name`.
fn pass_index(r: &Receipt, name: &str) -> Option<u64> {
    r.pass_names().iter().position(|n| n == name).map(|i| i as u64)
}

/// The `ResId` of the declared resource `name`.
fn res_index(r: &Receipt, name: &str) -> Option<u64> {
    r.resources().iter().find(|row| row.name == name).map(|row| row.index)
}

/// `(pass, stage, access)` of every declared access to resource `res`, in declaration order.
fn accesses_of(r: &Receipt, res: u64) -> Vec<(u64, u64, u64)> {
    r.array("access")
        .filter(|t| t.u64("res") == res)
        .map(|t| (t.u64("pass"), t.u64("stage"), t.u64("access")))
        .collect()
}

/// The checks every declarator shares; returns the merge's pass index.
fn merge_declared_before_the_draw(r: &Receipt, what: &str) -> u64 {
    let merge_cfg = rd1_config(r, "sdf_depth_merge");
    assert_eq!(merge_cfg, 1, "{what}: [config] sdf_depth_merge = {merge_cfg}, want 1 (Draw)");
    assert_eq!(rd1_config(r, "transparent_depth_capability"), 1, "{what}: the boot's capability lacks the particle bit");
    assert_eq!(rd1_config(r, "transparent_depth_frame"), 1, "{what}: the frame's set lacks the particle bit");
    let passes = r.pass_names();
    let merge = pass_index(r, "sdf_depth_merge")
        .unwrap_or_else(|| panic!("{what}: no `sdf_depth_merge` pass declared: {passes:?}"));
    let draw = pass_index(r, "particle_draw").unwrap_or_else(|| panic!("{what}: no `particle_draw` pass: {passes:?}"));
    assert_eq!(merge + 1, draw, "{what}: the merge is not declared immediately before particle_draw: {passes:?}");
    merge
}

/// The per-declarator checks; returns the receipt and the merge's pass index for a caller's own.
fn check_path(cfg: Cfg) -> (Receipt, u64) {
    let what = format!("{:?} x {:?}", cfg.path, cfg.legs);
    let r = receipt_of(cfg, 1, SETTLE, &[], "single");
    let merge = merge_declared_before_the_draw(&r, &what);
    let depth_name = match cfg.path {
        RenderPath::Deferred => "depth",
        RenderPath::VisibilityBuffer => "vb_depth",
        _ => "forward_depth",
    };
    let depth = res_index(&r, depth_name).unwrap_or_else(|| panic!("{what}: no `{depth_name}` resource"));
    let viewt = res_index(&r, "viewt").unwrap_or_else(|| panic!("{what}: no `viewt` resource declared"));
    // The merge writes the path depth and reads gViewT.
    let merge_depth: Vec<_> = accesses_of(&r, depth).into_iter().filter(|a| a.0 == merge).collect();
    assert!(
        merge_depth.iter().any(|a| a.2 & DS_WRITE != 0),
        "{what}: the merge does not declare a depth-attachment write of `{depth_name}`: {merge_depth:?}"
    );
    assert!(
        accesses_of(&r, viewt).iter().any(|a| a.0 == merge),
        "{what}: the merge does not read `viewt`"
    );
    // On the reverse-Z paths the marcher must PRODUCE `t` for it (no TAA in this boot).
    if cfg.path != RenderPath::Deferred {
        assert_eq!(
            rd1_config(&r, "path_sdf_forward_writes_viewt"),
            1,
            "{what}: an armed boot without TAA must select the VIEWT marcher"
        );
        let marcher = pass_index(&r, "sdf_forward_march").unwrap_or_else(|| panic!("{what}: no sdf_forward_march"));
        assert!(
            accesses_of(&r, viewt).iter().any(|a| a.0 == marcher && a.2 & SHADER_WRITE != 0),
            "{what}: the marcher does not declare its `viewt` write"
        );
    }
    eprintln!("{what}: sdf_depth_merge at pass {merge}, passes {:?}", r.pass_names());
    (r, merge)
}

#[test]
#[ignore = "gpu-windowed: RD1 receipt on Deferred x Both (spawns r0b_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn r0b_receipt_deferred() {
    let _ = check_path(Cfg::new(RenderPath::Deferred, GeometryLegs::Both));
}

#[test]
#[ignore = "gpu-windowed: RD1 receipt on Forward x Both (spawns r0b_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn r0b_receipt_forward() {
    let _ = check_path(Cfg::new(RenderPath::Forward, GeometryLegs::Both));
}

#[test]
#[ignore = "gpu-windowed: RD1 receipt on ForwardPlus x Both (spawns r0b_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn r0b_receipt_forwardplus() {
    let _ = check_path(Cfg::new(RenderPath::ForwardPlus, GeometryLegs::Both));
}

#[test]
#[ignore = "gpu-windowed: RD1 receipt on VisibilityBuffer x Both with the HZB armed (spawns r0b_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn r0b_receipt_vb() {
    let cfg = Cfg { hzb: true, ..Cfg::new(RenderPath::VisibilityBuffer, GeometryLegs::Both) };
    let (r, merge) = check_path(cfg);
    let passes = r.pass_names();
    let hzb: Vec<(usize, &String)> = passes.iter().enumerate().filter(|(_, n)| n.starts_with("hzb_")).collect();
    assert!(!hzb.is_empty(), "PREMISE: the HZB is armed but no hzb_* pass is declared: {passes:?}");
    for (i, n) in &hzb {
        assert!((*i as u64) < merge, "the merge (pass {merge}) is declared before `{n}` (pass {i}): {passes:?}");
    }
    let depth = res_index(&r, "vb_depth").expect("VB declares vb_depth");
    let draw = pass_index(&r, "particle_draw").expect("checked above");
    let last_writer = accesses_of(&r, depth)
        .into_iter()
        .filter(|a| a.0 < draw && a.2 & DS_WRITE != 0)
        .map(|a| a.0)
        .max();
    assert_eq!(last_writer, Some(merge), "the last writer of vb_depth before particle_draw is not the merge: {passes:?}");
}

#[test]
#[ignore = "gpu-windowed: RD1 receipt on VisibilityBuffer x Sdf, the merge as vb_depth's first writer (spawns r0b_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn r0b_receipt_vb_sdf() {
    let cfg = Cfg::new(RenderPath::VisibilityBuffer, GeometryLegs::Sdf);
    let r = receipt_of(cfg, 1, SETTLE, &[], "single");
    let merge = merge_declared_before_the_draw(&r, "VB x Sdf");
    let depth = res_index(&r, "vb_depth").expect("VB declares vb_depth");
    let first = accesses_of(&r, depth).first().map(|a| a.0);
    assert_eq!(first, Some(merge), "VB x Sdf: the merge is not the first access of vb_depth: {:?}", r.pass_names());
    assert_eq!(rd1_config(&r, "sdf_depth_merge_clear"), 1, "VB x Sdf: the merge must CLEAR its first touch");
    let first_touch = r
        .array("barrier")
        .any(|t| t.u64("pass") == merge && t.u64("res") == depth && t.u64("old_layout") == UNDEFINED);
    assert!(first_touch, "VB x Sdf: the merge's derived barrier on vb_depth does not leave UNDEFINED");
}

#[test]
#[ignore = "gpu-windowed: RD1 receipt burst over the BOYKO_HZB_DUMP frame on VisibilityBuffer x Both (spawns r0b_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn r0b_receipt_hzb_dump_frame() {
    let cfg = Cfg { hzb: true, ..Cfg::new(RenderPath::VisibilityBuffer, GeometryLegs::Both) };
    let dump = fresh_out("r0b_hzb_dump.bin");
    // The HZB dump settles 30 frames before its request; a burst of 10 from frame 25 straddles it.
    let r = receipt_of(cfg, 10, 25, &[("BOYKO_HZB_DUMP", dump.display().to_string())], "hzb_dump");
    assert!(dump.exists(), "PREMISE: the HZB dump never wrote {} — no dump frame ran", dump.display());
    let frames = r.frames();
    let key = |f: &graph_receipt::FrameRow, k: &str| {
        f.config.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or_else(|| {
            panic!("[[frame]] {} has no cfg_{k} — this receipt predates RD1's keys", f.ordinal)
        })
    };
    let mut dump_frames = 0;
    for f in &frames {
        let declared = key(f, "hzb_dump_declared");
        let merge = key(f, "sdf_depth_merge");
        let has_pass = f.passes.iter().any(|p| p == "sdf_depth_merge");
        if declared == 1 {
            dump_frames += 1;
            assert!(merge == 0 && !has_pass, "frame {}: the merge is declared on the HZB-dump frame", f.ordinal);
        } else {
            assert!(merge == 1 && has_pass, "frame {}: an ordinary frame of the burst lacks the merge", f.ordinal);
        }
    }
    assert!(dump_frames >= 1, "PREMISE: no frame of the burst declared the HZB dump ({} frames)", frames.len());
    eprintln!("hzb_dump burst: {} frames, {dump_frames} dump frame(s), merge absent on each", frames.len());
}

#[test]
#[ignore = "gpu-windowed: RD1 receipt on default boots (Forward x Sdf, VB x Sdf; particles off then on; spawns r0b_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn r0b_receipt_default_boot() {
    for path in [RenderPath::Forward, RenderPath::VisibilityBuffer] {
        for particles in [false, true] {
            let cfg = Cfg { particles, ..Cfg::new(path, GeometryLegs::Sdf) };
            let tag = if particles { "armed" } else { "default" };
            let r = receipt_of(cfg, 1, SETTLE, &[], tag);
            let want = u64::from(particles);
            let cap = rd1_config(&r, "transparent_depth_capability");
            let viewt = rd1_config(&r, "path_sdf_forward_writes_viewt");
            eprintln!("{path:?} x Sdf, particles {particles}: capability {cap}, VIEWT marcher {viewt}");
            assert_eq!(cap, want, "{path:?} x Sdf ({tag}): transparent_depth_capability");
            assert_eq!(viewt, want, "{path:?} x Sdf ({tag}): path_sdf_forward_writes_viewt");
        }
    }
}
