//! Render wave PX0 (H3 Scope 4; `px0/cut.md` D-C1) — **`FRAME_GRAPH_CAPACITY` against every
//! declarator's maximum configuration**, through OBS's receipt (`common/graph_receipt.rs`).
//!
//! OBS's own drivers (`obs_graph_receipt.rs`) run one max-config boot per declarator, but their max
//! config has no CAS, no SMAA, no VB occlusion split and no `hwrt`. This file adds those boots
//! without editing OBS's files: each cell is OBS's `build_obs_app` plus the extra arming below,
//! and the gate is OBS's `counts_check` — `[counts]` within `(max_res, max_res, max_pass,
//! max_acc)`, the marks printed per cell.
//!
//! | cell | boot |
//! |---|---|
//! | `deferred_taa_cas` | Deferred × Both, OBS max config (TAA, SSAO High, DDGI, CSM, punctual, particles, interp, upload) + `SharpenMode::Rcas` |
//! | `deferred_smaa` | the same with SMAA in TAA's place |
//! | `forward_both` | Forward × Both + particles |
//! | `forwardplus_mesh` | ForwardPlus × Mesh + particles |
//! | `vb_split_occlusion` | VB × Both, OBS max config (SSAO High split, TAA, DDGI, HZB, froxel cull) + `OcclusionMode::TwoPhase` and an `OcclusionCulling` marker on every mesh |
//!
//! # The markers, and which frame's receipt carries them
//!
//! `OcclusionCulling` is attached by a per-frame system ([`mark_occlusion`]), not by a second
//! startup system: two startup systems in one stage do not see each other's spawns (add-order is
//! not an order), so a startup marker pass could run before OBS's `obs_setup` spawned anything. The
//! per-frame system marks every mesh entity that lacks the marker on the first frame the entity is
//! visible to it; the receipt is captured after `BOYKO_GRAPH_RECEIPT_SETTLE = 30` frames, so the
//! receipt frame carries the marked, split frame — and the driver asserts
//! `path_vb_occlusion_split = 1` as a premise, so a frame without the split cannot pass silently.
//!
//! The `hwrt` legs (VB, Deferred) need `--features hwrt` and a ray-query device.
//!
//! Windowed-test conventions: the device tests are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::Entities;
use boyko_render::{
    AaConfig, AaMode, OcclusionConfig, OcclusionCulling, OcclusionMode, RenderPath, SharpenMode,
    TaaConfig,
};

use graph_receipt::{ObsArming, Receipt, assert_obs_resolved, build_obs_app, counts_check, fresh_out, run_worker};

/// The cell the worker boots.
const ENV_CELL: &str = "BOYKO_PX0_CAP_CELL";
/// The settle window (OBS's own).
const SETTLE: u32 = 30;

/// One max-config boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapCell {
    DeferredTaaCas,
    DeferredSmaa,
    ForwardBoth,
    ForwardPlusMesh,
    VbSplitOcclusion,
}

impl CapCell {
    const ALL: [CapCell; 5] = [
        CapCell::DeferredTaaCas,
        CapCell::DeferredSmaa,
        CapCell::ForwardBoth,
        CapCell::ForwardPlusMesh,
        CapCell::VbSplitOcclusion,
    ];

    fn word(self) -> &'static str {
        match self {
            CapCell::DeferredTaaCas => "deferred_taa_cas",
            CapCell::DeferredSmaa => "deferred_smaa",
            CapCell::ForwardBoth => "forward_both",
            CapCell::ForwardPlusMesh => "forwardplus_mesh",
            CapCell::VbSplitOcclusion => "vb_split_occlusion",
        }
    }

    fn from_word(w: &str) -> Option<CapCell> {
        CapCell::ALL.into_iter().find(|c| c.word() == w)
    }

    /// OBS's arming for this cell (OBS's legs rule: ForwardPlus is mesh-only, every other path
    /// runs both legs).
    fn arming(self) -> ObsArming {
        let path = match self {
            CapCell::DeferredTaaCas | CapCell::DeferredSmaa => RenderPath::Deferred,
            CapCell::ForwardBoth => RenderPath::Forward,
            CapCell::ForwardPlusMesh => RenderPath::ForwardPlus,
            CapCell::VbSplitOcclusion => RenderPath::VisibilityBuffer,
        };
        ObsArming {
            path,
            interp: true,
            particles: true,
            upload: true,
            // SMAA and TAA are exclusive; the SMAA cell arms SMAA in TAA's place below.
            taa: self != CapCell::DeferredSmaa,
            ddgi: true,
        }
    }

    /// The `[config]` premises the receipt must carry — a boot that did not arm what the cell
    /// names proves nothing about its maximum.
    fn premises(self) -> Vec<(&'static str, u64)> {
        let mut p = vec![("path_has_particles", 1), ("material_upload", 1), ("csm", 1)];
        match self {
            CapCell::DeferredTaaCas => p.extend([("taa", 1), ("rcas", 1)]),
            CapCell::DeferredSmaa => p.extend([("smaa", 1), ("taa", 0)]),
            CapCell::ForwardBoth => p.push(("path_has_sdf_forward", 1)),
            CapCell::ForwardPlusMesh => p.push(("path_is_forward_plus", 1)),
            CapCell::VbSplitOcclusion => {
                p.extend([("taa", 1), ("path_vb_split", 1), ("path_vb_occlusion_split", 1), ("hzb", 1)]);
            }
        }
        if cfg!(feature = "hwrt") {
            p.push(("hwrt", 1));
        }
        p
    }
}

/// Marks every mesh entity without [`OcclusionCulling`] — once per entity, from the first frame
/// the entity exists (see the module doc for why this is a per-frame system).
#[allow(clippy::needless_pass_by_value)] // SystemParams are by value
fn mark_occlusion(mut commands: Commands, entities: Entities, q: Query<(&MeshHandle, Option<&OcclusionCulling>)>) {
    for (id, (_, marked)) in q.iter_entities() {
        if marked.is_none()
            && let Some(entity) = entities.get(id)
        {
            commands.entity(entity).insert(OcclusionCulling);
        }
    }
}

/// **THE WORKER** — one max-config boot with `BOYKO_GRAPH_RECEIPT` armed.
#[test]
#[ignore = "gpu-windowed: the PX0 capacity worker, spawned by the px0_capacity_* drivers with BOYKO_GRAPH_RECEIPT set; run with --test-threads=1"]
fn px0_capacity_worker() {
    if std::env::var_os("BOYKO_GRAPH_RECEIPT").is_none() {
        eprintln!("px0_capacity_worker: BOYKO_GRAPH_RECEIPT is unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let word = std::env::var(ENV_CELL).unwrap_or_else(|_| panic!("{ENV_CELL} is unset: name the cell"));
    let cell = CapCell::from_word(word.trim()).unwrap_or_else(|| panic!("{ENV_CELL}={word} is not a cell"));
    let arming = cell.arming();
    let mut app = build_obs_app("boyko_app PX0 capacity worker", arming);
    match cell {
        CapCell::DeferredTaaCas => {
            app.insert_resource(TaaConfig { sharpen: SharpenMode::Rcas, ..TaaConfig::default() });
        }
        CapCell::DeferredSmaa => {
            app.insert_resource(AaConfig { mode: AaMode::Smaa });
        }
        CapCell::VbSplitOcclusion => {
            app.insert_resource(OcclusionConfig { mode: OcclusionMode::TwoPhase });
            app.add_systems(mark_occlusion);
        }
        CapCell::ForwardBoth | CapCell::ForwardPlusMesh => {}
    }
    app.run();
    assert_obs_resolved(&app, arming);
}

/// Boots `cell`, checks its premises and its `[counts]` against the capacity, prints the marks.
fn check_cell(cell: CapCell) {
    let out = fresh_out(&format!("px0_capacity_{}.toml", cell.word()));
    run_worker(
        "px0_capacity_worker",
        &[
            ("BOYKO_GRAPH_RECEIPT", out.display().to_string()),
            ("BOYKO_GRAPH_RECEIPT_FRAMES", "1".to_owned()),
            ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
            (ENV_CELL, cell.word().to_owned()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + 31).to_string()),
        ],
        false,
    );
    let r = Receipt::load(&out);
    let mut failures = Vec::new();
    for (key, want) in cell.premises() {
        let got = r.config(key);
        if got != want {
            failures.push(format!("PREMISE: [config] {key} = {got}, the cell needs {want}"));
        }
    }
    match counts_check(&r) {
        Ok(line) => eprintln!("PX0 capacity {} (hwrt={}): {line}", cell.word(), cfg!(feature = "hwrt")),
        Err(e) => failures.push(e),
    }
    assert!(
        failures.is_empty(),
        "PX0 capacity receipt `{}` ({} failure(s)):\n{}",
        cell.word(),
        failures.len(),
        failures.join("\n")
    );
}

#[test]
#[ignore = "gpu-windowed: PX0 capacity on Deferred + TAA + CAS (spawns px0_capacity_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn px0_capacity_deferred_taa_cas() {
    check_cell(CapCell::DeferredTaaCas);
}

#[test]
#[ignore = "gpu-windowed: PX0 capacity on Deferred + SMAA (spawns px0_capacity_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn px0_capacity_deferred_smaa() {
    check_cell(CapCell::DeferredSmaa);
}

#[test]
#[ignore = "gpu-windowed: PX0 capacity on Forward x Both + particles (spawns px0_capacity_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn px0_capacity_forward_both() {
    check_cell(CapCell::ForwardBoth);
}

#[test]
#[ignore = "gpu-windowed: PX0 capacity on ForwardPlus x Mesh + particles (spawns px0_capacity_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn px0_capacity_forwardplus_mesh() {
    check_cell(CapCell::ForwardPlusMesh);
}

#[test]
#[ignore = "gpu-windowed: PX0 capacity on VB split + two-phase occlusion (spawns px0_capacity_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn px0_capacity_vb_split_occlusion() {
    check_cell(CapCell::VbSplitOcclusion);
}

/// The VB max config under `hwrt` — the declarator the capacity's `max_res` was sized on.
#[cfg(feature = "hwrt")]
#[test]
#[ignore = "feature+gpu-windowed+gpu-cap: PX0 capacity on VB split + occlusion under --features hwrt (needs ray query; spawns px0_capacity_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn px0_capacity_vb_split_occlusion_hwrt() {
    check_cell(CapCell::VbSplitOcclusion);
}

/// Deferred's max config under `hwrt`.
#[cfg(feature = "hwrt")]
#[test]
#[ignore = "feature+gpu-windowed+gpu-cap: PX0 capacity on Deferred + TAA + CAS under --features hwrt (needs ray query; spawns px0_capacity_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn px0_capacity_deferred_taa_cas_hwrt() {
    check_cell(CapCell::DeferredTaaCas);
}

/// The cell words round-trip and every cell names its path's premises (host, no GPU).
#[test]
fn every_cell_word_round_trips() {
    for c in CapCell::ALL {
        assert_eq!(CapCell::from_word(c.word()), Some(c));
        assert!(c.premises().iter().any(|(k, _)| *k == "path_has_particles"));
    }
    assert_eq!(CapCell::from_word("vb_split"), None);
}
