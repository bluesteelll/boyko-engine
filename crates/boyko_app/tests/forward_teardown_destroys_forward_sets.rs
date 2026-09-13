//! VB-01 (the Forward family), written red-first: the teardown of a `Forward × Mesh` boot must destroy
//! the `ForwardTargets` descriptor sets that boot created, before the device itself is destroyed.
//!
//! # What is checked
//!
//! The run, the controls C1–C4 and the summary are in `teardown_probe/mod.rs`. This boot has no
//! geometry table, so only the buffer control is watched (slot 0). One defect assertion follows the
//! preconditions and controls:
//!
//! - **D-pool**: no persistent descriptor pool is left at the post-teardown sample. On this boot the
//!   objects under test own one pool per `ForwardTargets.set0` and `set1` group
//!   (2 × `FRAMES_IN_FLIGHT`).
//!
//! `AaMode::Off`: Forward v1 has no TAA producer.
//!
//! # Running
//!
//! ```text
//! cargo test -p boyko-app --test forward_teardown_destroys_forward_sets -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Only `BOYKO_DISABLE_VALIDATION`, `BOYKO_ENABLE_VALIDATION`, `BOYKO_WIN_HIDDEN` and `BOYKO_LOG` may
//! be set; any other `BOYKO_*` variable fails a precondition before boot. `--test-threads=1` is
//! required (one process-global GPU device). A windowless box prints `SKIP` and returns: a skip, not
//! a pass.

#![cfg(windows)]

mod teardown_probe;

use boyko_render::{AaMode, GeometryLegs, RenderPath};
use boyko_rhi_vulkan::present::FRAMES_IN_FLIGHT;

use teardown_probe::ProbeCfg;

#[test]
#[ignore = "gpu-windowed: needs a real windowed GPU device; run with --test-threads=1"]
fn a_forward_teardown_destroys_its_forward_descriptor_sets() {
    let probe = teardown_probe::run(ProbeCfg {
        label: "boyko_app Forward teardown probe",
        path: RenderPath::Forward,
        legs: GeometryLegs::Mesh,
        aa: AaMode::Off,
        watch_table: false,
    });
    if !teardown_probe::assert_preconditions_and_controls(&probe) {
        return;
    }
    let s = &probe.summary;

    let predicted = 2 * FRAMES_IN_FLIGHT;
    assert_eq!(
        probe.stats.persistent_descriptor_pools_live, 0,
        "DEFECT D-pool: persistent descriptor pools still alive after teardown; predicted today \
         2·F = {predicted} (ForwardTargets set0/set1; F = FRAMES_IN_FLIGHT = {FRAMES_IN_FLIGHT})\n{s}"
    );
}
