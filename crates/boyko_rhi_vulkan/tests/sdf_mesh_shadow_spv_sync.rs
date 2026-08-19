//! `sdf_mesh_shadow.comp.spv` byte-identity gate — the re-DXC oracle for the VB-SV0 DP pass.
//!
//! DP1 (`docs/VB-SV0-SDF-SHADOW-PLAN.md` Rev 10) adds ONE new module: the dedicated SDF-on-mesh
//! shadow + contact-AO prepass. Its committed `.spv` must byte-match its source under the frozen
//! recipe, exactly like every other variant family in this crate — and the gate exists from the
//! module's FIRST commit, because "no `*_spv_sync` for cluster_cull was a real gap" is a lesson
//! this repo already paid for once.
//!
//! The pass `#define`s `VB_SV0` at SOURCE level before including `vb_geom_fetch.hlsli`, which is
//! what unlocks the `tri_p0/1/2` exports and `vb_sv0_face_normal`. That define is deliberately
//! NOT a `-D`: the ten lit-producer tails never define it and preprocess character-identical to
//! their pre-SV0 form, which is what `vb_lit_producer_spv_sync.rs` (byte-identity on all ten)
//! proves holds after any edit to the shared fetch header.
//!
//! SKIPS (with an eprintln) when no `dxc` resolves on the host — the byte gate is only as
//! hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed artifact.

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

/// Re-DXCs the pass under the EXACT frozen recipe pinned in its own header comment
/// (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, no `-O`, no `-D`) into a fresh temp
/// `.spv` and returns the bytes. Never overwrites the committed artifact.
fn redxc(dxc: &PathBuf, dir: &PathBuf) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join("sdf_mesh_shadow.comp.spv.redxc.spv");
    let mut cmd = Command::new(dxc);
    cmd.current_dir(dir).args(["-spirv", "-T", "cs_6_0", "-E", "main"]);
    cmd.args(["-fspv-target-env=vulkan1.3", "sdf_mesh_shadow.comp.hlsl", "-Fo"]).arg(&out_spv);
    let status = cmd.status().expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling sdf_mesh_shadow.comp.hlsl under the frozen recipe");
    let bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy
    bytes
}

#[test]
fn sdf_mesh_shadow_spv_matches_source() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP sdf_mesh_shadow_spv_matches_source: no dxc on this host");
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("sdf_mesh_shadow.comp.spv");
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc(&dxc, &dir);
    assert!(
        committed == fresh,
        "sdf_mesh_shadow.comp.spv ({} bytes committed, {} bytes fresh) is NOT the re-DXC of its \
         source under the frozen recipe — either the committed .spv is stale (re-run the recipe \
         in the shader's header and commit it) or the host dxc is not the pinned VulkanSDK \
         1.4.350.0 toolchain.",
        committed.len(),
        fresh.len(),
    );

    // The module marches: an artifact this gate would also pass on an accidentally-empty compile
    // (a `#define` typo disabling every block) is the vacuity this floor refuses. The committed
    // pass carries the shadow loop and the AO taps; a module without loops has neither.
    let word_count = committed.len() / 4;
    assert!(
        word_count > 2000,
        "the committed pass is implausibly small ({word_count} words) — the march did not compile in"
    );
}
