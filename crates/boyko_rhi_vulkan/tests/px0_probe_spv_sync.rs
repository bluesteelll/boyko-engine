//! Render wave PX0 (`px0/cut.md` PX0-6, PC-PX0-14) — **the byte gate of the four test-only probe
//! kernels**: each committed `.spv` is the re-DXC of its `.hlsl` under the frozen recipe in that
//! file's header (the three sources and their `-D` rows in `docs/SHADER-VARIANT-MANIFEST.md`).
//!
//! The kernels are hand-written test HLSL (not eDSL-owned: they compute nothing the host model must
//! mirror beyond A3's `lit * t + s`), so this file is their only gate. SKIPS with an `eprintln`
//! when no `dxc` resolves — a skipped run is not a pass, and the lane's tester runs it on the
//! workstation, where the pinned VulkanSDK 1.4.350.0 is installed.

use std::path::PathBuf;
use std::process::Command;

mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};

/// `(source, profile, defines, committed .spv)`.
const KERNELS: [(&str, &str, &[&str], &str); 4] = [
    ("px0_probe_store.comp.hlsl", "cs_6_0", &["-D", "FMT_R11G11B10F"], "px0_probe_store_r11g11b10f.comp.spv"),
    ("px0_probe_store.comp.hlsl", "cs_6_0", &["-D", "FMT_RGBA16F"], "px0_probe_store_rgba16f.comp.spv"),
    ("px0_probe_raster.fs.hlsl", "ps_6_0", &[], "px0_probe_raster.fs.spv"),
    ("px0_fetch_rate.comp.hlsl", "cs_6_0", &[], "px0_fetch_rate.comp.spv"),
];

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// The pinned SDK's `dxc`, then `$VULKAN_SDK/Bin`, then `PATH`.
fn find_dxc() -> Option<PathBuf> {
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    let pinned = PathBuf::from(format!("C:/VulkanSDK/1.4.350.0/Bin/{bare}"));
    if pinned.exists() {
        return Some(pinned);
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let c = PathBuf::from(sdk).join("Bin").join(bare);
        if c.exists() {
            return Some(c);
        }
    }
    Command::new(bare).arg("--version").output_within(DXC_DEADLINE).is_ok().then(|| PathBuf::from(bare))
}

#[test]
fn every_probe_spv_is_the_redxc_of_its_source() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP px0_probe_spv_sync: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let dir = shaders_dir();
    let mut stale = Vec::new();
    for (src, profile, defines, spv) in KERNELS {
        let out = child_guard::scratch_path(&format!("{spv}.redxc"));
        let status = Command::new(&dxc)
            .current_dir(&dir)
            .args(["-spirv", "-T", profile, "-E", "main", "-fspv-target-env=vulkan1.3"])
            .args(defines)
            .arg(src)
            .arg("-Fo")
            .arg(&out)
            .status_within(DXC_DEADLINE)
            .expect("invariant: dxc was located and must run");
        assert!(status.success(), "dxc failed on {src} {defines:?} under the frozen recipe");
        let fresh = std::fs::read(&out).expect("invariant: dxc wrote the re-DXC .spv");
        let _ = std::fs::remove_file(&out);
        let committed = std::fs::read(dir.join(spv)).unwrap_or_else(|e| panic!("{spv}: {e}"));
        if fresh != committed {
            stale.push(format!("{spv} ({} bytes committed, {} re-DXC'd) is stale against {src} {defines:?}", committed.len(), fresh.len()));
        }
    }
    assert!(stale.is_empty(), "re-compile the probe kernels under their header recipes:\n{}", stale.join("\n"));
}

/// The table names four distinct `.spv`, each committed.
#[test]
fn the_kernel_table_names_every_committed_spv() {
    for (i, (_, _, _, spv)) in KERNELS.iter().enumerate() {
        assert!(KERNELS[i + 1..].iter().all(|k| k.3 != *spv), "{spv} twice");
        assert!(shaders_dir().join(spv).exists(), "{spv} is not committed");
    }
}
