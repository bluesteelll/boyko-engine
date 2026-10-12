//! **OBS (R) — the production frame-graph receipt, per declarator** (render-wave plan H0 Scope 5,
//! device test 1; OPEN-ITEMS 1's receipt census, `critique_verify_r5.md` I3).
//!
//! The device drivers spawn [`obs_receipt_worker`] (this binary, `--exact`) on one max-config boot
//! per declarator with `BOYKO_GRAPH_RECEIPT` armed, then read the receipt it wrote. Each requires:
//!
//! - the CLASS-TABLE CENSUS is clean: every receipt resource named by a `SEED_CLASS_TABLE` row of
//!   the declarator derives the row's class from its raw `seeded` bit and seed, carries the row's
//!   minimum stages, and every `expect_present` row is declared (checked FIRST, so a `--release`
//!   run — where the (P) class assert is compiled out — reports the census's own violation);
//! - the boot armed what max config means (the `[config]` premises);
//! - `present_sample` is the frame's last pass;
//! - every `[[sink]]` slot names the `ResId` it resolves (debug builds; a release receipt states
//!   that it carries no sink rows);
//! - `[counts]` lie within `FRAME_GRAPH_CAPACITY`, printed.
//!
//! `obs_graph_receipt_burst16` reads a 16-frame burst: 16 `[[frame]]` records over consecutive
//! presented frames, `submission_epoch` strictly increasing, the whole `[config]` in each.
//!
//! The three non-ignored tests run the reader, the census and the burst check over inline
//! fixtures, so "these checks can fail" is proven on every run, not only at a red-first.
//!
//! Windowed-test conventions: the device tests are ignored by default, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`, `BOYKO_WINDOW_FRAMES` set (each driver also sets a per-worker
//! backstop).

#[path = "common/graph_receipt.rs"]
mod graph_receipt;

use boyko_render::RenderPath;
use boyko_rhi_vulkan::present::seed_class::{DECL_DEFERRED, DECL_FORWARD, declarator_name};

use graph_receipt::{
    ENV_PATH, ObsArming, Receipt, assert_obs_resolved, build_obs_app, burst_check, class_table_census,
    counts_check, fresh_out, run_worker, sink_check,
};

/// The settle window every driver asks for (the instrument's default).
const SETTLE: u32 = 30;

// ===============================================================================================
// Host fixtures (not ignored)
// ===============================================================================================

/// A receipt as the writer lays it out, with two resources and two records.
const FIXTURE: &str = r#"# fixture
schema_version = 1
build = "debug"
sink_rows = true
declarator = "Deferred"
frames_captured = 31
ordinal = 30

[config]
path = 0
path_has_particles = 1

[counts]
resources = 2
res_state_total = 2
passes = 2
accesses = 2
img_barriers = 1
buf_barriers = 0
max_res = 96
max_pass = 64
max_acc = 384

[[resource]]
index = 0
name = "lit"
kind = "image"
mips = 1
seeded = false
seed_layout = 0
seed_flush_stages = 0
seed_flush_access = 0
seed_visible_stages = 0
seed_visible_access = 0
class = "Bare"

[[resource]]
index = 1
name = "fog_sun_far"
kind = "image"
mips = 1
seeded = false
seed_layout = 0
seed_flush_stages = 0
seed_flush_access = 0
seed_visible_stages = 0
seed_visible_access = 0
class = "Bare"

[[pass]]
index = 0
name = "fog"
accesses = 1
img_barriers = 1
buf_barriers = 0

[[access]]
pass = 0
res = 1
stage = 2048
access = 64
layout = 1
base_mip = 0
mip_count = 1

[[pass]]
index = 1
name = "present_sample"
accesses = 1
img_barriers = 0
buf_barriers = 0

[[access]]
pass = 1
res = 0
stage = 128
access = 32
layout = 5
base_mip = 0
mip_count = 1

[[sink]]
declarator = "Deferred"
kind = "image"
slot = 0
sink_name = "lit"
fif = "Ring"
res = 0
graph_name = "lit"

[[sink]]
declarator = "Deferred"
kind = "image"
slot = 1
sink_name = "fog_sun_far"
fif = "Single"
res = 1
graph_name = "fog_sun_far"

[[frame]]
ordinal = 30
epoch = 31
pass_count = 2
passes = "fog,present_sample"
cfg_path = 0

[[frame]]
ordinal = 31
epoch = 32
pass_count = 2
passes = "fog,present_sample"
cfg_path = 0
"#;

/// The fixture with `fog_sun_far` declared through its class (FrameLocal, a COMPUTE reader seed).
fn fixture_declared_by_class() -> String {
    FIXTURE.replacen(
        "name = \"fog_sun_far\"\nkind = \"image\"\nmips = 1\nseeded = false\nseed_layout = 0\nseed_flush_stages = 0\nseed_flush_access = 0\nseed_visible_stages = 0\nseed_visible_access = 0",
        "name = \"fog_sun_far\"\nkind = \"image\"\nmips = 1\nseeded = false\nseed_layout = 5\nseed_flush_stages = 0\nseed_flush_access = 0\nseed_visible_stages = 2048\nseed_visible_access = 32",
        1,
    )
}

#[test]
fn receipt_parser_reads_a_fixture() {
    let r = Receipt::parse(FIXTURE).expect("the fixture parses");
    assert_eq!(r.root().str("declarator"), "Deferred");
    assert_eq!(r.root().u64("schema_version"), 1);
    assert_eq!(r.config("path_has_particles"), 1);
    let res = r.resources();
    assert_eq!(res.len(), 2);
    assert_eq!((res[1].name.as_str(), res[1].is_image, res[1].derived_class()), ("fog_sun_far", true, "Bare"));
    assert_eq!(r.pass_names(), ["fog", "present_sample"]);
    let sinks = r.sinks().expect("a debug fixture carries sink rows");
    assert_eq!(sinks.len(), 2);
    assert_eq!(sink_check(&r), Ok(2));
    let frames = r.frames();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].passes, ["fog", "present_sample"]);
    assert_eq!(frames[1].config, [("path".to_owned(), 0)]);
    assert!(counts_check(&r).is_ok());

    // The release shape states itself instead of reading as an empty, clean sink table.
    let release = FIXTURE.replacen("sink_rows = true", "sink_rows = false", 1).replacen("build = \"debug\"", "build = \"release\"", 1);
    let err = Receipt::parse(&release).expect("parses").sinks().expect_err("a release receipt has no sink rows");
    assert!(err.contains("NO sink rows"), "{err}");

    // A drifted slot name is red, and so is a frame past the capacity.
    let drifted = FIXTURE.replacen("sink_name = \"lit\"", "sink_name = \"viewt\"", 1);
    assert!(sink_check(&Receipt::parse(&drifted).expect("parses")).is_err());
    let over = FIXTURE.replacen("resources = 2", "resources = 97", 1);
    let msg = counts_check(&Receipt::parse(&over).expect("parses")).expect_err("97 > 96");
    assert!(msg.contains("[counts] resources 97 > 96"), "{msg}");
}

/// R5.1 (OPEN-ITEMS 1): a receipt that declares `fog_sun_far` — a pre-registered wave row — bare
/// is red through the table DATA, and the same receipt declaring it by its class is clean on that
/// row. A census green on the bare fixture would mean it is not wired to `SEED_CLASS_TABLE`.
#[test]
fn class_table_census_reds_a_bare_row_fixture() {
    let bare = class_table_census(&Receipt::parse(FIXTURE).expect("parses"), DECL_DEFERRED);
    let fog: Vec<_> = bare.iter().filter(|v| v.name == "fog_sun_far").collect();
    assert_eq!(fog.len(), 1, "the bare fog_sun_far row must be exactly one violation: {bare:?}");
    assert_eq!((fog[0].rule, fog[0].derived, fog[0].row_class), ("WrongClass", "Bare", "FrameLocal"));
    assert_eq!(fog[0].declarator, declarator_name(DECL_DEFERRED));
    // The fixture declares only two resources, so every `expect_present` HEAD row is Absent —
    // the census's presence half, exercised on the same run.
    assert!(bare.iter().any(|v| v.name == "material_table" && v.rule == "Absent"), "{bare:?}");

    let by_class = class_table_census(&Receipt::parse(&fixture_declared_by_class()).expect("parses"), DECL_DEFERRED);
    assert!(by_class.iter().all(|v| v.name != "fog_sun_far"), "declared by its class it must be clean: {by_class:?}");

    // W2's M3.2 (R) shape: `material_table` declared bare on Forward is the census's own red.
    let forward_bare = FIXTURE
        .replacen("declarator = \"Deferred\"", "declarator = \"Forward\"", 1)
        .replacen("name = \"fog_sun_far\"\nkind = \"image\"", "name = \"material_table\"\nkind = \"buffer\"", 1);
    let v = class_table_census(&Receipt::parse(&forward_bare).expect("parses"), DECL_FORWARD);
    assert!(
        v.iter().any(|x| x.name == "material_table" && x.declarator == "Forward" && x.rule == "WrongClass" && x.derived == "Bare"),
        "a bare material_table on Forward must be reported as WrongClass/Bare: {v:?}"
    );
}

/// M5.1's shape: a burst whose records repeat the first frame is red on `submission_epoch`.
#[test]
fn burst_check_reds_a_repeated_record_fixture() {
    let r = Receipt::parse(FIXTURE).expect("parses");
    let mut key_lines = String::new();
    for k in boyko_rhi_vulkan::present::RECEIPT_CONFIG_KEYS {
        key_lines.push_str(&format!("cfg_{k} = 0\n"));
    }
    let full = FIXTURE.replace("cfg_path = 0\n", &key_lines);
    assert_eq!(burst_check(&Receipt::parse(&full).expect("parses"), 2), Ok(()));
    let err = burst_check(&r, 2).expect_err("a record without the whole [config] is red");
    assert!(err.contains("lacks cfg_"), "{err}");
    let repeated = full.replacen("ordinal = 31\nepoch = 32", "ordinal = 30\nepoch = 31", 1);
    let err = burst_check(&Receipt::parse(&repeated).expect("parses"), 2).expect_err("a repeated record is red");
    assert!(err.contains("submission_epoch not strictly increasing"), "{err}");
    assert!(burst_check(&Receipt::parse(&full).expect("parses"), 3).is_err(), "a short burst is red");
}

// ===============================================================================================
// The worker
// ===============================================================================================

/// **The OBS receipt WORKER** — one max-config boot of the path `BOYKO_OBS_PATH` names, with the
/// receipt armed by its driver. Booted without `BOYKO_GRAPH_RECEIPT` it SKIPS: no armed capture
/// could end its loop.
#[test]
#[ignore = "gpu-windowed: the OBS receipt worker, spawned by the obs_graph_receipt_* drivers with BOYKO_GRAPH_RECEIPT set; run with --test-threads=1"]
fn obs_receipt_worker() {
    if std::env::var_os("BOYKO_GRAPH_RECEIPT").is_none() {
        eprintln!("obs_receipt_worker: BOYKO_GRAPH_RECEIPT is unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let arming = ObsArming::from_env();
    let mut app = build_obs_app("boyko_app OBS receipt worker", arming);
    app.run();
    assert_obs_resolved(&app, arming);
}

// ===============================================================================================
// The drivers
// ===============================================================================================

/// Boots the worker on `path` and returns its receipt (`frames` records).
fn receipt_of(path: RenderPath, frames: u32, tag: &str) -> Receipt {
    let word = ObsArming::path_word(path);
    let out = fresh_out(&format!("receipt_{word}_{tag}.toml"));
    run_worker(
        "obs_receipt_worker",
        &[
            ("BOYKO_GRAPH_RECEIPT", out.display().to_string()),
            ("BOYKO_GRAPH_RECEIPT_FRAMES", frames.to_string()),
            ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
            (ENV_PATH, word.to_owned()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + frames + 30).to_string()),
        ],
        false,
    );
    Receipt::load(&out)
}

/// Device test 1 on one declarator.
fn check_declarator(path: RenderPath) {
    let arming = ObsArming { path, interp: true, particles: true, upload: true };
    let declarator = arming.declarator();
    let r = receipt_of(path, 1, "single");
    let mut failures = Vec::new();

    // The census first: on a `--release` leg it is the only class gate left.
    for v in class_table_census(&r, declarator) {
        failures.push(format!("CLASS TABLE: {v}"));
    }

    assert_eq!(r.root().str("declarator"), declarator_name(declarator), "PREMISE: the receipt's declarator");
    let want_path = match path {
        RenderPath::Deferred => 0,
        RenderPath::Forward => 1,
        RenderPath::ForwardPlus => 2,
        _ => 3,
    };
    let mut premise = |key: &str, want: u64| {
        let got = r.config(key);
        if got != want {
            failures.push(format!("PREMISE: [config] {key} = {got}, max config needs {want} — the worker is not max-config"));
        }
    };
    premise("path", want_path);
    premise("path_has_particles", 1);
    premise("material_upload", 1);
    premise("csm", 1);
    if path == RenderPath::Deferred {
        premise("interp", 1);
    }
    if matches!(path, RenderPath::Deferred | RenderPath::VisibilityBuffer) {
        premise("taa", 1);
    }

    let passes = r.pass_names();
    if passes.last().map(String::as_str) != Some("present_sample") {
        failures.push(format!("the last declared pass is {:?}, not present_sample: {passes:?}", passes.last()));
    }

    match sink_check(&r) {
        Ok(n) => eprintln!("{}: {n} sink rows, every one names its ResId", declarator_name(declarator)),
        Err(e) if !cfg!(debug_assertions) && e.contains("NO sink rows") => {
            eprintln!("NOTE {}: {e} — the sink half of this gate runs in the debug legs", declarator_name(declarator));
        }
        Err(e) => failures.push(format!("SINK: {e}")),
    }
    match counts_check(&r) {
        Ok(line) => eprintln!("{} {line}", declarator_name(declarator)),
        Err(e) => failures.push(e),
    }
    let cfg: Vec<String> = r.single("config").fields.iter().map(|(k, v)| format!("{k}={v}")).collect();
    eprintln!("{} [config] {}", declarator_name(declarator), cfg.join(" "));
    assert!(
        failures.is_empty(),
        "OBS receipt on {path:?} ({} failure(s)):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
#[ignore = "gpu-windowed: OBS device test 1 on Deferred (spawns obs_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_graph_receipt_deferred() {
    check_declarator(RenderPath::Deferred);
}

#[test]
#[ignore = "gpu-windowed: OBS device test 1 on Forward (spawns obs_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_graph_receipt_forward() {
    check_declarator(RenderPath::Forward);
}

#[test]
#[ignore = "gpu-windowed: OBS device test 1 on ForwardPlus (spawns obs_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_graph_receipt_forwardplus() {
    check_declarator(RenderPath::ForwardPlus);
}

#[test]
#[ignore = "gpu-windowed: OBS device test 1 on VisibilityBuffer (spawns obs_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_graph_receipt_vb() {
    check_declarator(RenderPath::VisibilityBuffer);
}

/// Device test 1's burst case: 16 records over 16 consecutive presented frames.
#[test]
#[ignore = "gpu-windowed: OBS device test 1's 16-frame receipt burst on Deferred (spawns obs_receipt_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_graph_receipt_burst16() {
    let r = receipt_of(RenderPath::Deferred, 16, "burst16");
    if let Err(e) = burst_check(&r, 16) {
        panic!("OBS receipt burst: {e}");
    }
    let epochs: Vec<u64> = r.frames().iter().map(|f| f.epoch).collect();
    eprintln!("burst16 epochs {epochs:?}");
}
