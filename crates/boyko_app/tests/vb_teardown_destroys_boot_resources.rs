//! VB-01, written red-first: the teardown of a `VisibilityBuffer × Mesh × TAA` boot must destroy the
//! device objects that boot created, before the device itself is destroyed.
//!
//! # What is checked
//!
//! The run, the watches, the controls C1–C4 and the summary are in `teardown_probe/mod.rs`. On this
//! boot the geometry table's two buffers are watched, and three defect assertions follow the
//! preconditions and controls, in this order:
//!
//! - **D-buf**: the geometry table's `bounds_buffer` (watch slot 0) and `meta_buffer` (slot 1) are no
//!   longer live host-pool sub-allocations at the post-teardown sample. They stay live unless
//!   `MeshGeometryTable::destroy` runs on the table before the sample.
//! - **D-pool**: no persistent descriptor pool is left at the sample. On this boot the objects under
//!   test own the geometry set's pool (1), one pool per `ForwardTargets.set0` and `set1` group
//!   (2 × `FRAMES_IN_FLIGHT`), and one per `viewt_from_vb_depth_set` group, a ring TAA arms on a
//!   VisibilityBuffer mesh boot (`FRAMES_IN_FLIGHT`).
//! - **D-table**: `MeshGeometryTableSlot` no longer holds a table at the sample.
//!
//! # Running
//!
//! ```text
//! cargo test -p boyko-app --test vb_teardown_destroys_boot_resources -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Only `BOYKO_DISABLE_VALIDATION`, `BOYKO_ENABLE_VALIDATION`, `BOYKO_WIN_HIDDEN` and `BOYKO_LOG` may
//! be set; any other `BOYKO_*` variable fails a precondition before boot. `--test-threads=1` is
//! required (one process-global GPU device). A windowless box, or a device whose VisibilityBuffer
//! resolve degrades (no geometry table), prints `SKIP` and returns: a skip, not a pass.

#![cfg(windows)]

mod teardown_probe;

use boyko_render::{AaMode, GeometryLegs, RenderPath};
use boyko_rhi_vulkan::present::FRAMES_IN_FLIGHT;

use teardown_probe::ProbeCfg;

#[test]
#[ignore = "gpu-windowed: needs a real windowed GPU device with the VisibilityBuffer descriptor-indexing cap; run with --test-threads=1"]
fn a_visibility_buffer_teardown_destroys_its_geometry_table_and_persistent_descriptor_pools() {
    let probe = teardown_probe::run(ProbeCfg {
        label: "boyko_app VB teardown probe",
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
        aa: AaMode::Taa,
        watch_table: true,
    });
    if !teardown_probe::assert_preconditions_and_controls(&probe) {
        return;
    }
    let s = &probe.summary;
    let stats = &probe.stats;

    assert!(
        !stats.watch[0].live_at_teardown && !stats.watch[1].live_at_teardown,
        "DEFECT D-buf: the geometry table's bounds_buffer (watch 0, live={}) and meta_buffer (watch 1, live={}) \
         are still live host-pool sub-allocations after teardown — MeshGeometryTable::destroy did not run on the \
         table: teardown's force-drain (retire_deferred_frees) reinserts MeshGeometryTableSlot and nothing \
         destroys it\n{s}",
        stats.watch[0].live_at_teardown,
        stats.watch[1].live_at_teardown,
    );

    let predicted = 1 + 3 * FRAMES_IN_FLIGHT;
    assert_eq!(
        stats.persistent_descriptor_pools_live, 0,
        "DEFECT D-pool: persistent descriptor pools still alive after teardown; predicted today \
         1 + 2·F + F = {predicted} (geometry set, ForwardTargets set0/set1, viewt ring; F = FRAMES_IN_FLIGHT = \
         {FRAMES_IN_FLIGHT})\n{s}"
    );

    assert!(
        !stats.geometry_table_present_at_sample,
        "DEFECT D-table: MeshGeometryTableSlot still held a table at the post-teardown sample — teardown did \
         not remove and destroy it\n{s}"
    );
}
