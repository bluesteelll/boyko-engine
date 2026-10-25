//! **`shadow_atrous*.comp.spv` byte-identity gate** — the re-DXC oracle for the two modules the
//! shadow-visibility à-trous denoise compiles from ONE source.
//!
//! Since IBL1 P1 (critique C1) `shadow_atrous.comp.hlsl` builds two committed modules: the
//! Deferred chain's base (`gNormal`, `rg16`) and the VisibilityBuffer split's `-D VB_THIN=1` twin
//! (`gThinNormal`, `rgba8`). Their filter body is the same text, compiled by two separate recipe
//! lines in the source header. Before this gate nothing compared either committed binary with its
//! source: `gnormal_precision`'s `gnormal_slot_spirv_formats` reads one `OpTypeImage` format per
//! named variable, not the bytes, and every golden pin runs `BOYKO_SHADOW_DENOISE=none`, so no pin
//! dispatches either module. An edit of the shared body re-DXC'd under the first recipe line only
//! would ship the VB split a stale filter with every gate green (review r1 W2). Each row is
//! re-DXC'd under the header's frozen recipe (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`,
//! no `-O`, plus the row's `-D`s) and must equal the committed artifact. Every row is compared
//! before the verdict, so the message names each stale module, and a pair that drifted apart (one
//! row fresh, the other stale) reads as such.
//!
//! SKIPS (with an eprintln) when no `dxc` resolves on the host, exactly like the `*_spv_sync`
//! precedents — a different dxc version failing this test means "wrong toolchain", not "drift".

use std::path::PathBuf;
use std::process::Command;

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

fn find_dxc() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/dxc.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(bare).arg("--version").output_within(DXC_DEADLINE).is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// The two modules of `shadow_atrous.comp.hlsl` (its header's two recipe lines and the manifest's
/// `shadow_atrous.comp.hlsl` table).
const ROWS: [(&[&str], &str); 2] = [
    (&[], "shadow_atrous.comp.spv"),
    (&["VB_THIN=1"], "shadow_atrous_vb.comp.spv"),
];

#[test]
fn shadow_atrous_modules_reproduce_under_frozen_recipe() {
    let Some(dxc) = find_dxc() else {
        eprintln!("shadow_atrous_spv_sync: dxc not found — SKIPPING the reproduction check on this host.");
        return;
    };
    let dir = shaders_dir();
    let mut stale = Vec::new();
    for (defines, spv) in ROWS {
        let out = child_guard::scratch_path(&format!("{spv}.redxc.spv"));
        let mut cmd = Command::new(&dxc);
        cmd.current_dir(&dir).args(["-spirv", "-T", "cs_6_0", "-E", "main"]);
        for d in defines {
            cmd.args(["-D", d]);
        }
        cmd.args(["-fspv-target-env=vulkan1.3", "shadow_atrous.comp.hlsl", "-Fo"]).arg(&out);
        let status = cmd.status_within(DXC_DEADLINE).expect("invariant: dxc was located and must run");
        assert!(status.success(), "dxc failed re-compiling shadow_atrous.comp.hlsl {defines:?}");
        let fresh = std::fs::read(&out).expect("invariant: dxc wrote the re-DXC .spv");
        let committed = std::fs::read(dir.join(spv)).unwrap_or_else(|e| panic!("{spv} is committed: {e}"));
        let verdict = if fresh == committed { "reproduces" } else { "STALE" };
        println!(
            "shadow_atrous_spv_sync: {spv} ({} bytes committed, {} fresh) {verdict} under {defines:?}",
            committed.len(),
            fresh.len()
        );
        if fresh != committed {
            stale.push(format!("{spv} {defines:?}"));
        }
    }
    assert!(
        stale.is_empty(),
        "{stale:?} is NOT the re-DXC of shadow_atrous.comp.hlsl under the frozen recipe — re-run EVERY dxc \
         line of the header (both modules share the filter body) and commit the results"
    );
}

// `status_within` / `output_within` (a deadline on each dxc this file spawns) and `scratch_path`
// live in `tests/child_guard/mod.rs`, whose module doc records the hung-dxc stall behind them.
mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};
