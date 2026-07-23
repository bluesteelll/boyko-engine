//! Cluster-cull `.spv` byte-identity gate — the re-DXC oracle for `cluster_cull.hlsl`.
//!
//! `cluster_cull.hlsl` is a single-variant compute shader (no `-D` defines), compiled offline
//! with the FROZEN recipe pinned in its own header comment. Nothing else proved the committed
//! `cluster_cull.comp.spv` BYTE-MATCHES that source under the frozen recipe — a stale or
//! hand-tweaked blob would ship undetected until a GPU frame-golden run. This test clones the
//! `redxc_to_bytes` idiom `marcher_spv_sync.rs` uses for the SDF marcher family, scoped to the
//! Lighting L1 froxel cull.
//!
//! SKIPS (with an eprintln) when no `dxc` resolves on the host — the byte gate is only as
//! hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed artifact; a
//! DIFFERENT dxc version failing this test means "wrong toolchain", not "drifted shader" (the
//! committed recipe header pins the exact version).

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC must run so any `#include` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `dxc` executable: first the pinned Vulkan-SDK path (the repo's offline recipe),
/// then `$VULKAN_SDK/Bin`, then `PATH`. Returns `None` if none resolve (the byte-identity test
/// then SKIPS) — the `marcher_spv_sync.rs` idiom verbatim.
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
    if Command::new(bare).arg("--version").output().is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// Re-DXCs `hlsl_name` (relative to the shaders dir) under the EXACT frozen recipe pinned in
/// `cluster_cull.hlsl`'s own header comment (`-spirv -T cs_6_0 -E main
/// -fspv-target-env=vulkan1.3`, no `-D`, no `-O`), into a fresh temp `.spv`, and returns the
/// bytes. Never overwrites a committed artifact.
fn redxc_to_bytes(dxc: &PathBuf, dir: &PathBuf, hlsl_name: &str, out_tag: &str) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{out_tag}.redxc.spv"));
    let status = Command::new(dxc)
        .current_dir(dir)
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3", hlsl_name, "-Fo"])
        .arg(&out_spv)
        .status()
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling {hlsl_name} under the frozen recipe");
    let bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy
    bytes
}

#[test]
fn cluster_cull_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "cluster_cull_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the cluster-cull re-DXC byte-identity \
             check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("cluster_cull.comp.spv");
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc_to_bytes(&dxc, &dir, "cluster_cull.hlsl", "cluster_cull.comp");
    assert!(
        committed == fresh,
        "cluster_cull.comp.spv ({} bytes committed, {} bytes fresh) is NOT the re-DXC of \
         cluster_cull.hlsl under the frozen recipe (`dxc.exe -spirv -T cs_6_0 -E main \
         -fspv-target-env=vulkan1.3`) — either the committed .spv is stale (re-run the recipe \
         in the shader's header and commit the fresh bytes) or the host dxc is not the pinned \
         VulkanSDK 1.4.350.0 toolchain.",
        committed.len(),
        fresh.len(),
    );
}
