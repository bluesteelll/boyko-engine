//! **RD1 WR probe — the SDF depth merge's cost cell** (render wave RD1, defect R0b; the WR row).
//!
//! NO TIMING VERDICT here: the probe boots one cell at 2560×1440 and lets the GPU zone leg write
//! its profile artifact; the orchestrator's quiet-window protocol reads the artifacts (zone
//! `ZONE_SDF_DEPTH_MERGE` plus the Δ of `ZONE_PARTICLE_DRAW`, armed against
//! `BOYKO_TEST_FORCE=disarm_sdf_merge`).
//!
//! The cell is `BOYKO_R0B_COST_CELL`:
//!
//! | cell | path | TAA |
//! |---|---|---|
//! | `deferred` | Deferred × Both | off |
//! | `vb_taa` | VisibilityBuffer × Both | on |
//! | `vb_notaa` | VisibilityBuffer × Both | off |
//!
//! The scene is `common/r0b_scene.rs` with the CONTROL emitter only — a particle in front of the
//! background, so the merge occludes nothing and the particle draw does the same work on both
//! arms. The fixture decodes `BOYKO_TEST_FORCE` itself (the A/B arm). The artifact knobs are the
//! runner's own and are inherited from the caller: `BOYKO_VB_ZONE=1`, `BOYKO_VB_BENCH_FRAMES`,
//! `BOYKO_PROFILE_ARTIFACT`, `BOYKO_PROFILE_RUN_TOKEN`.
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/r0b_scene.rs"]
mod r0b_scene;

use boyko_render::{GeometryLegs, RenderPath};

use graph_receipt::run_worker;
use r0b_scene::{ENV_EMIT, ENV_LEGS, ENV_PATH, ENV_TAA, R0bArming, assert_r0b_resolved, build_r0b_app};

/// The marker the driver sets.
const ENV_WORKER: &str = "BOYKO_R0B_COST_WORKER";
/// The cell knob.
const ENV_CELL: &str = "BOYKO_R0B_COST_CELL";
/// The WR row's extent.
const COST_W: u32 = 2560;
/// The WR row's extent.
const COST_H: u32 = 1440;

/// The `(path, taa)` a cell word names.
fn cell(word: &str) -> (RenderPath, bool) {
    match word {
        "deferred" => (RenderPath::Deferred, false),
        "vb_taa" => (RenderPath::VisibilityBuffer, true),
        "vb_notaa" => (RenderPath::VisibilityBuffer, false),
        other => panic!("{ENV_CELL}={other} is not deferred|vb_taa|vb_notaa"),
    }
}

/// **The cost WORKER** — one cell at 2560×1440.
#[test]
#[ignore = "gpu-windowed: the RD1 merge-cost worker, spawned by r0b_merge_cost_probe; run with --test-threads=1"]
fn r0b_merge_cost_worker() {
    if std::env::var_os(ENV_WORKER).is_none() {
        eprintln!("r0b_merge_cost_worker: {ENV_WORKER} is unset -- SKIPPED (spawned by its driver).");
        return;
    }
    let arming = R0bArming::from_env();
    let mut app = build_r0b_app("boyko_app RD1 merge-cost worker", arming, COST_W, COST_H);
    app.run();
    assert_r0b_resolved(&app, arming);
}

/// **The cost probe** — spawns the worker on `BOYKO_R0B_COST_CELL` and checks that the artifact it
/// was asked for exists. It measures nothing itself.
#[test]
#[ignore = "gpu-windowed: the RD1 WR merge-cost probe (spawns r0b_merge_cost_worker; set BOYKO_R0B_COST_CELL, BOYKO_VB_ZONE=1, BOYKO_PROFILE_ARTIFACT); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn r0b_merge_cost_probe() {
    let Ok(word) = std::env::var(ENV_CELL) else {
        panic!("{ENV_CELL} is unset: name the cell (deferred | vb_taa | vb_notaa) — a probe of no cell measures nothing");
    };
    let (path, taa) = cell(word.trim());
    let artifact = std::env::var_os("BOYKO_PROFILE_ARTIFACT").map(std::path::PathBuf::from);
    if let Some(a) = &artifact {
        let _ = std::fs::remove_file(a);
    }
    run_worker(
        "r0b_merge_cost_worker",
        &[
            (ENV_WORKER, "1".to_owned()),
            (ENV_PATH, R0bArming::path_word(path).to_owned()),
            (ENV_LEGS, R0bArming::legs_word(GeometryLegs::Both).to_owned()),
            (ENV_EMIT, "control".to_owned()),
            (ENV_TAA, if taa { "1" } else { "0" }.to_owned()),
        ],
        false,
    );
    match artifact {
        Some(a) => {
            assert!(a.exists(), "the cost worker exited 0 but wrote no artifact at {}", a.display());
            eprintln!("r0b_merge_cost_probe: cell {word} -> {}", a.display());
        }
        None => eprintln!("r0b_merge_cost_probe: cell {word} ran with no BOYKO_PROFILE_ARTIFACT — nothing recorded"),
    }
}
