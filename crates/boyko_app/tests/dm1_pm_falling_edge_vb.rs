//! DM1 red-first test (4) on **VisibilityBuffer x Mesh**: live defect D-2, the `PerInstanceMaterial` ring's
//! falling edge. The scene, premises and verdict are `dm1_common::pm_falling_edge`'s. VB and
//! Forward are the two paths whose shaders read the ring on every frame (the Deferred `pm` raster is
//! deselected with the flag), so each gets its own binary.
//!
//! Windowed-test conventions: ignored by default (needs a windowed GPU device), `--test-threads=1`, one
//! `#[test]` per file, `BOYKO_DISABLE_VALIDATION=1` for the default leg.

#![cfg(windows)]

mod dm1_common;

use boyko_render::RenderPath;

#[test]
#[ignore = "gpu-windowed: needs a windowed GPU device; DM1 red-first gate (4) / D-2 on VisibilityBuffer x Mesh, run with --test-threads=1"]
fn dm1_pm_falling_edge_vb() {
    dm1_common::pm_falling_edge::run("dm1_pm_falling_edge_vb", RenderPath::VisibilityBuffer);
}
