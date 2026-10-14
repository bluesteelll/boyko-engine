//! **RD6 WR probe — the D-DS cost cells** (render wave RD6, defect D-DS; the cut's WR-DS row).
//!
//! NO TIMING VERDICT here: the probe boots one cell at 2560×1440 and lets the GPU zone leg write
//! its profile artifact; the orchestrator's quiet-window protocol reads the artifacts — the camera
//! raster zone and the lit producer zones — interleaving the D-4 head (A, HEAD's culling) and the
//! D-5 head (B, the face members and the facing flip), 10 runs per cell, median.
//!
//! The cell is `BOYKO_DDS_COST_CELL`: `deferred`, `forward` or `vb`. The scene is
//! `common/dds_scene.rs`'s `caster` scene with a double-sided card under the sun (a floor receiver,
//! the shadow cascades, one double-sided caster): the common single-sided floor plus one instance
//! of every face class the fix changes. The artifact knobs are the runner's own and are inherited
//! from the caller: `BOYKO_VB_ZONE=1`, `BOYKO_VB_BENCH_FRAMES`, `BOYKO_PROFILE_ARTIFACT`,
//! `BOYKO_PROFILE_RUN_TOKEN`.
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/dds_scene.rs"]
mod dds_scene;
#[path = "common/graph_receipt.rs"]
mod graph_receipt;

use boyko_render::RenderPath;

use dds_scene::{DdsArming, ENV_DS, ENV_PATH, ENV_SCENE, assert_dds_resolved, build_dds_app_sized};
use graph_receipt::run_worker;

/// The marker the driver sets.
const ENV_WORKER: &str = "BOYKO_DDS_COST_WORKER";
/// The cell knob.
const ENV_CELL: &str = "BOYKO_DDS_COST_CELL";
/// The WR row's extent.
const COST_W: u32 = 2560;
/// The WR row's extent.
const COST_H: u32 = 1440;

/// The path a cell word names.
fn cell(word: &str) -> RenderPath {
    match word {
        "deferred" => RenderPath::Deferred,
        "forward" => RenderPath::Forward,
        "vb" => RenderPath::VisibilityBuffer,
        other => panic!("{ENV_CELL}={other} is not deferred|forward|vb"),
    }
}

/// **The cost WORKER** — one cell at 2560×1440.
#[test]
#[ignore = "gpu-windowed: the RD6 D-DS cost worker, spawned by dds_cost_probe; run with --test-threads=1"]
fn dds_cost_worker() {
    if std::env::var_os(ENV_WORKER).is_none() {
        eprintln!("dds_cost_worker: {ENV_WORKER} is unset -- SKIPPED (spawned by its driver).");
        return;
    }
    let arming = DdsArming::from_env();
    let mut app = build_dds_app_sized("boyko_app RD6 D-DS cost worker", arming, COST_W, COST_H);
    app.run();
    assert_dds_resolved(&app, arming);
}

/// **The cost probe** — spawns the worker on `BOYKO_DDS_COST_CELL` and checks that the artifact it
/// was asked for exists. It measures nothing itself.
#[test]
#[ignore = "gpu-windowed: the RD6 WR D-DS cost probe (spawns dds_cost_worker; set BOYKO_DDS_COST_CELL, BOYKO_VB_ZONE=1, BOYKO_PROFILE_ARTIFACT); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_cost_probe() {
    let Ok(word) = std::env::var(ENV_CELL) else {
        panic!("{ENV_CELL} is unset: name the cell (deferred | forward | vb) — a probe of no cell measures nothing");
    };
    let path = cell(word.trim());
    let artifact = std::env::var_os("BOYKO_PROFILE_ARTIFACT").map(std::path::PathBuf::from);
    if let Some(a) = &artifact {
        let _ = std::fs::remove_file(a);
    }
    run_worker(
        "dds_cost_worker",
        &[
            (ENV_WORKER, "1".to_owned()),
            (ENV_PATH, DdsArming::path_word(path).to_owned()),
            (ENV_SCENE, "caster".to_owned()),
            (ENV_DS, "1".to_owned()),
        ],
        false,
    );
    match artifact {
        Some(a) => {
            assert!(a.exists(), "the cost worker exited 0 but wrote no artifact at {}", a.display());
            eprintln!("dds_cost_probe: cell {word} -> {}", a.display());
        }
        None => eprintln!("dds_cost_probe: cell {word} ran with no BOYKO_PROFILE_ARTIFACT — nothing recorded"),
    }
}
