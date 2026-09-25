//! DM1 red-first test (1) on **Deferred x Both**: an authored material edit made after boot reaches the
//! GPU table, for a mesh pixel and an SDF pixel. The scene, premises and verdict are
//! `dm1_common::edit_reaches_gpu`'s (one thin binary per path, so every leg runs each path once
//! and no path is an environment default a run can forget).
//!
//! Windowed-test conventions: ignored by default (needs a windowed GPU device), `--test-threads=1`, one
//! `#[test]` per file (the process-global hook rule — `asset_streaming_f7_grow_headless.rs`'s module
//! doc), `BOYKO_DISABLE_VALIDATION=1` for the default leg.

#![cfg(windows)]

mod dm1_common;

use boyko_render::RenderPath;

#[test]
#[ignore = "gpu-windowed: needs a windowed GPU device; DM1 red-first gate (1) on Deferred x Both, run with --test-threads=1"]
fn dm1_edit_reaches_gpu_deferred() {
    dm1_common::edit_reaches_gpu::run("dm1_edit_reaches_gpu_deferred", RenderPath::Deferred);
}
