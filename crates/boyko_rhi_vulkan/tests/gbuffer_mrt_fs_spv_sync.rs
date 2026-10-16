//! **`gbuffer_mrt*.fs.spv` byte-identity gate** — the re-DXC oracle for the five Deferred mesh
//! G-buffer fragment variants.
//!
//! Before RD6 / D-DS nothing in the workspace re-compiled these five modules: the
//! `gbuffer_mrt_edsl_sync` test pins the spliced `oct_encode` / `pack_material_id_ba` spans in the
//! SOURCE, `textured_nonuniform_spirv` walks one committed binary, and no test compared the
//! committed bytes to their source. RD6 edits the source (the `mesh_face_flip` span and its call),
//! so a stale `.spv` would ship a producer without the double-sided orientation while every source
//! gate stayed green. Each row is re-DXC'd under the source header's frozen recipe
//! (`-spirv -T ps_6_0 -E main -fspv-target-env=vulkan1.3`, no `-O`, plus the row's `-D`s) and must
//! equal the committed artifact.
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

/// The five variants of `gbuffer_mrt.fs.hlsl` (its header's recipe and the manifest rows).
const ROWS: [(&[&str], &str); 5] = [
    (&[], "gbuffer_mrt.fs.spv"),
    (&["MOTION_VECTORS=1"], "gbuffer_mrt_mv.fs.spv"),
    (&["PER_INSTANCE_MATERIAL=1"], "gbuffer_mrt_pm.fs.spv"),
    (&["MOTION_VECTORS=1", "PER_INSTANCE_MATERIAL=1"], "gbuffer_mrt_mvpm.fs.spv"),
    (&["TEXTURED=1"], "gbuffer_mrt_tex.fs.spv"),
];

#[test]
fn gbuffer_mrt_fs_variants_reproduce_under_frozen_recipe() {
    let Some(dxc) = find_dxc() else {
        eprintln!("gbuffer_mrt_fs_spv_sync: dxc not found — SKIPPING the reproduction check on this host.");
        return;
    };
    let dir = shaders_dir();
    for (defines, spv) in ROWS {
        let out = child_guard::scratch_path(&format!("{spv}.redxc.spv"));
        let mut cmd = Command::new(&dxc);
        cmd.current_dir(&dir).args(["-spirv", "-T", "ps_6_0", "-E", "main"]);
        for d in defines {
            cmd.args(["-D", d]);
        }
        cmd.args(["-fspv-target-env=vulkan1.3", "gbuffer_mrt.fs.hlsl", "-Fo"]).arg(&out);
        let status = cmd.status_within(DXC_DEADLINE).expect("invariant: dxc was located and must run");
        assert!(status.success(), "dxc failed re-compiling gbuffer_mrt.fs.hlsl {defines:?}");
        let fresh = std::fs::read(&out).expect("invariant: dxc wrote the re-DXC .spv");
        let committed = std::fs::read(dir.join(spv)).unwrap_or_else(|e| panic!("{spv} is committed: {e}"));
        assert!(
            fresh == committed,
            "{spv} ({} bytes committed, {} fresh) is NOT the re-DXC of gbuffer_mrt.fs.hlsl {defines:?} \
             under the frozen recipe — re-run the header's dxc line and commit the result",
            committed.len(),
            fresh.len()
        );
    }
}

// `status_within` / `output_within` (a deadline on each dxc this file spawns) and `scratch_path`
// live in `tests/child_guard/mod.rs`, whose module doc records the hung-dxc stall behind them.
mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};
