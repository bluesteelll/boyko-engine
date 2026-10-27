//! Render rung GB, gate **G2** (device): on armed normal frames, every scope of every frame
//! completes and every declared pass enters exactly one scope.
//!
//! Each driver spawns this binary's worker (`gb_g2_worker`) for one maximal configuration — OBS's
//! max-config scene (`common/graph_receipt.rs`: interp, particles, a material upload every frame,
//! an SDF sphere, CSM, punctual lights, and per path TAA, SSAO, DDGI, HZB) — booted at
//! `GpuCrashDiagLevel::Checkpoints`. The worker renders [`FRAMES`] frames and reads the
//! `GpuCrumbLedger` the runner publishes when its frame loop returns:
//!
//! | cell | boot |
//! |---|---|
//! | `deferred` | Deferred × Both: TAA, SSAO High, DDGI, particles (+ the SDF depth merge), interp, upload |
//! | `forward_plus` | ForwardPlus × Mesh: particles, interp, upload |
//! | `vb` | VisibilityBuffer × Both: TAA, SSAO High, DDGI, HZB, froxel cull, particles, interp, upload, and the two-phase occlusion split (`OcclusionCulling` on every mesh) |
//!
//! # The verdict, per cell
//!
//! - the labels and markers pieces armed (`frame_pieces`) — G2 needs breadcrumbs, so a device
//!   without `VK_AMD_buffer_marker` is RED here, never a skip;
//! - `frames_checked > 0` and `frames_checked + 3 >= frames` — the last two frames are never
//!   verified (no later fence wait), and a ledger nobody verified is a vacuous green;
//! - `frames_incomplete == 0` — every frame begin/end and every scope's begin/end carried the
//!   frame's serial after the fence wait (the frame-close host-read barrier makes them visible);
//! - `parity_mismatch == 0` — graph-pass scopes entered == passes declared, every frame;
//! - `serial_regressions == 0`, `labels_begun == labels_ended`.
//!
//! A debug build also asserts parity and completion inside the verification itself
//! (`crumb_verify_slot`), so a mismatch panics the worker on the first bad frame.
//!
//! # Running it
//!
//! ```text
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_WINDOW_FRAMES=40 cargo test -p boyko-app --test gpu_crumbs_frames \
//!     -- --ignored --test-threads=1 --nocapture
//! ```
#![cfg(windows)]

#[path = "common/graph_receipt.rs"]
mod graph_receipt;

use boyko_app::prelude::*;
use boyko_app::{GpuCrashDiag, GpuCrashDiagLevel, GpuCrumbLedger};
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Entities, ResMut};
use boyko_macros::Resource;
use boyko_render::{OcclusionConfig, OcclusionCulling, OcclusionMode, RenderPath};
use boyko_rhi_vulkan::gpu_crash::piece;
use graph_receipt::{ObsArming, assert_obs_resolved, build_obs_app, run_worker};

/// The worker's cell variable.
const ENV_CELL: &str = "BOYKO_GB_G2_CELL";

/// Frames each worker renders.
const FRAMES: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Deferred,
    ForwardPlus,
    Vb,
}

impl Cell {
    const ALL: [Cell; 3] = [Cell::Deferred, Cell::ForwardPlus, Cell::Vb];

    fn word(self) -> &'static str {
        match self {
            Cell::Deferred => "deferred",
            Cell::ForwardPlus => "forward_plus",
            Cell::Vb => "vb",
        }
    }

    fn arming(self) -> ObsArming {
        match self {
            Cell::Deferred => ObsArming {
                path: RenderPath::Deferred,
                interp: true,
                particles: true,
                upload: true,
                taa: true,
                ddgi: true,
            },
            Cell::ForwardPlus => ObsArming {
                path: RenderPath::ForwardPlus,
                interp: true,
                particles: true,
                upload: true,
                taa: false,
                ddgi: false,
            },
            Cell::Vb => ObsArming {
                path: RenderPath::VisibilityBuffer,
                interp: true,
                particles: true,
                upload: true,
                taa: true,
                ddgi: true,
            },
        }
    }
}

#[derive(Resource)]
struct Budget(u32);

fn exit_after_budget(mut budget: ResMut<Budget>, mut exit: ResMut<AppExit>) {
    if budget.0 > 0 {
        budget.0 -= 1;
        if budget.0 == 0 {
            exit.0 = true;
        }
    }
}

/// Marks every mesh entity for the occlusion split on the first frame it is visible (the
/// `px0_capacity_receipt` shape: a startup marker could run before the scene spawned).
fn mark_occlusion(mut commands: Commands, entities: Entities, q: Query<(&MeshHandle, Option<&OcclusionCulling>)>) {
    for (id, (_, marked)) in q.iter_entities() {
        if marked.is_none()
            && let Some(entity) = entities.get(id)
        {
            commands.entity(entity).insert(OcclusionCulling);
        }
    }
}

/// **THE WORKER** — one armed max-config boot, spawned by the drivers below with its cell set.
#[test]
#[ignore = "gpu-windowed: the GB G2 worker, spawned by the crumbs_* drivers with BOYKO_GB_G2_CELL set; run with --test-threads=1"]
fn gb_g2_worker() {
    let Ok(word) = std::env::var(ENV_CELL) else {
        eprintln!("gb_g2_worker: {ENV_CELL} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    let cell = Cell::ALL
        .into_iter()
        .find(|c| c.word() == word.trim())
        .unwrap_or_else(|| panic!("{ENV_CELL}={word} is not a cell"));
    let arming = cell.arming();
    let mut app = build_obs_app("boyko_app GB G2 worker", arming);
    if cell == Cell::Vb {
        app.insert_resource(OcclusionConfig { mode: OcclusionMode::TwoPhase });
        app.add_systems(mark_occlusion);
    }
    app.insert_resource(GpuCrashDiag { level: GpuCrashDiagLevel::Checkpoints });
    app.insert_resource(Budget(FRAMES));
    app.add_systems(exit_after_budget);
    app.run();

    assert_obs_resolved(&app, arming);
    let frames = app.world().resource::<HostFrameStats>().frames;
    let ledger = app.world().try_resource::<GpuCrumbLedger>().copied();
    println!("[G2] {} frames {frames} ledger {ledger:?}", cell.word());
    let l = ledger.unwrap_or_else(|| {
        panic!("[G2] {}: no GpuCrumbLedger -- the boot armed no GPU crash-diagnostics piece", cell.word())
    });
    let c = l.counts;
    let mut red = Vec::new();
    let wanted = piece::LABELS | piece::MARKERS;
    if l.frame_pieces & wanted != wanted {
        red.push(format!("frame_pieces {:#x}: labels and markers must both be armed (this device lacks one)", l.frame_pieces));
    }
    if c.frames_checked == 0 || c.frames_checked + 3 < frames {
        red.push(format!("frames_checked {} of {frames} frames: the verification did not run", c.frames_checked));
    }
    if c.frames_incomplete != 0 {
        red.push(format!("frames_incomplete {} of {}", c.frames_incomplete, c.frames_checked));
    }
    if c.parity_mismatch != 0 {
        red.push(format!("parity_mismatch {} of {}", c.parity_mismatch, c.frames_checked));
    }
    if c.serial_regressions != 0 {
        red.push(format!("serial_regressions {}", c.serial_regressions));
    }
    if c.labels_begun != c.labels_ended {
        red.push(format!("labels_begun {} != labels_ended {}", c.labels_begun, c.labels_ended));
    }
    assert!(red.is_empty(), "[G2] {} RED:\n{}", cell.word(), red.join("\n"));
}

fn drive(cell: Cell) {
    run_worker("gb_g2_worker", &[(ENV_CELL, cell.word().to_owned())], false);
}

#[test]
#[ignore = "gpu-windowed: GB G2 - Deferred max config (TAA, SSAO, DDGI, particles + SDF depth merge) at level checkpoints; spawns gb_g2_worker; run with --test-threads=1"]
fn crumbs_deferred_taa_particles_merge_ssao() {
    drive(Cell::Deferred);
}

#[test]
#[ignore = "gpu-windowed: GB G2 - ForwardPlus max config (particles, interp, upload) at level checkpoints; spawns gb_g2_worker; run with --test-threads=1"]
fn crumbs_forward_plus_particles() {
    drive(Cell::ForwardPlus);
}

#[test]
#[ignore = "gpu-windowed: GB G2 - VisibilityBuffer max config (TAA, occlusion split, particles, SSAO, DDGI, HZB) at level checkpoints; spawns gb_g2_worker; run with --test-threads=1"]
fn crumbs_vb_taa_occsplit_particles_ssao() {
    drive(Cell::Vb);
}
