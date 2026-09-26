//! DM1 live defect D-3 on **Forward x Both**: a material-table grow repoints the Forward Set 0 and
//! `sdf_forward_set`, so a row the grow seeded renders on mesh and SDF pixels. The scene, premises
//! and verdict are `dm1_common::grow_repoint`'s.
//!
//! Windowed-test conventions: ignored by default (needs a windowed GPU device), `--test-threads=1`, one
//! `#[test]` per file (the process-global hook rule — `asset_streaming_f7_grow_headless.rs`'s module
//! doc), `BOYKO_DISABLE_VALIDATION=1` for the default leg. The validation leg
//! (`BOYKO_ENABLE_VALIDATION=1`, `BOYKO_DISABLE_VALIDATION` unset) must print zero `[vk-validation]`
//! lines.

#![cfg(windows)]

mod dm1_common;

use boyko_render::RenderPath;

#[test]
#[ignore = "gpu-windowed: needs a windowed GPU device; DM1 D-3 grow-repoint gate on Forward x Both, run with --test-threads=1"]
fn dm1_grow_repoint_forward() {
    dm1_common::grow_repoint::run("dm1_grow_repoint_forward", RenderPath::Forward);
}
