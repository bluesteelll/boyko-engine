//! Marcher `.spv` byte-identity gate — the re-DXC oracle for the SDF marcher family.
//!
//! `sdf_field_edsl_sync.rs` pins the marcher TEXT (every eDSL-generated span `.contains`-checked
//! against both committed `.hlsl`), but until this file nothing proved the committed `.spv`
//! artifacts BYTE-MATCH their committed `.hlsl` under the frozen recipe — a stale or
//! hand-tweaked marcher blob would ship undetected until a GPU frame-golden run (the
//! `redxc_to_bytes` pattern existed only for the SSAO variants + `vb_interp`). This test clones
//! `ssao_edsl_sync.rs`'s gate for:
//!
//! * the FOUR `sdf_forward_march` `{HAS_MESH} x {VIEWT}` variants (one source, four `-D`
//!   combinations — see `docs/SHADER-VARIANT-MANIFEST.md`'s `sdf_forward_march` table), and
//! * the deferred marcher `sdf_gbuffer_composite.comp.spv` (the single-variant compile whose
//!   freeze `sdf_field_edsl_sync.rs`'s doc comments reference).
//!
//! SKIPS (with an eprintln) when no `dxc` resolves on the host — the byte gate is only as
//! hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed artifacts;
//! a DIFFERENT dxc version failing this test means "wrong toolchain", not "drifted shader"
//! (the committed recipe headers pin the exact version).

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and
/// `.spv` live (and where DXC must run so its `#include "sdf_field.hlsli"` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `dxc` executable: first the pinned Vulkan-SDK path (the repo's offline
/// recipe), then `$VULKAN_SDK/Bin`, then `PATH`. Returns `None` if none resolve (the
/// byte-identity test then SKIPS) — the `ssao_edsl_sync.rs` idiom verbatim.
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

/// Re-DXCs `hlsl_name` (relative to the shaders dir) under the EXACT frozen recipe
/// (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, NO `-O`) plus the given `-D`
/// defines, into a fresh temp `.spv` named by `out_tag` (distinct per variant so parallel
/// test binaries never collide), and returns the bytes. Never overwrites a committed artifact.
fn redxc_with_defines(dxc: &PathBuf, dir: &PathBuf, hlsl_name: &str, defines: &[&str], out_tag: &str) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{out_tag}.redxc.spv"));
    let mut cmd = Command::new(dxc);
    cmd.current_dir(dir).args(["-spirv", "-T", "cs_6_0", "-E", "main"]);
    for d in defines {
        cmd.args(["-D", d]);
    }
    cmd.args(["-fspv-target-env=vulkan1.3", hlsl_name, "-Fo"]).arg(&out_spv);
    let status = cmd.status().expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling {hlsl_name} {defines:?} under the frozen recipe");
    let bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy
    bytes
}

/// One committed artifact must byte-equal its own re-DXC.
fn assert_spv_byte_identical(dxc: &PathBuf, dir: &PathBuf, hlsl_name: &str, defines: &[&str], spv_name: &str) {
    let committed_path = dir.join(spv_name);
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc_with_defines(dxc, dir, hlsl_name, defines, spv_name);
    assert!(
        committed == fresh,
        "{spv_name} ({} bytes committed, {} bytes fresh) is NOT the re-DXC of {hlsl_name} \
         {defines:?} under the frozen recipe — either the committed .spv is stale (re-run the \
         recipe in the shader's header and commit ALL sibling variants together) or the host \
         dxc is not the pinned VulkanSDK 1.4.350.0 toolchain.",
        committed.len(),
        fresh.len(),
    );
}

#[test]
fn sdf_forward_march_variant_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "marcher_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no $VULKAN_SDK/Bin, \
             not on PATH) — SKIPPING the marcher re-DXC byte-identity check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    // The {HAS_MESH} x {VIEWT} matrix — every row of the SHADER-VARIANT-MANIFEST table. All
    // four MUST be recommitted together on any .hlsl edit (one source, four artifacts —
    // forgetting a sibling silently forks the corresponding leg set).
    let variants: [(&[&str], &str); 4] = [
        (&["HAS_MESH=1"], "sdf_forward_march.comp.spv"),
        (&[], "sdf_forward_march_sdfonly.comp.spv"),
        (&["HAS_MESH=1", "VIEWT=1"], "sdf_forward_march_viewt.comp.spv"),
        (&["VIEWT=1"], "sdf_forward_march_sdfonly_viewt.comp.spv"),
    ];
    for (defines, spv_name) in variants {
        assert_spv_byte_identical(&dxc, &dir, "sdf_forward_march.comp.hlsl", defines, spv_name);
    }
}

#[test]
fn sdf_gbuffer_composite_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "marcher_spv_sync: dxc not found — SKIPPING the deferred-marcher re-DXC \
             byte-identity check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    // The deferred marcher is a single-variant compile (no -D) — the freeze the
    // `sdf_field_edsl_sync.rs` doc comments cite is enforced HERE.
    assert_spv_byte_identical(&dxc, &dir, "sdf_gbuffer_composite.hlsl", &[], "sdf_gbuffer_composite.comp.spv");
}
