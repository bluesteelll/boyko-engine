//! VB-P1a ("dark infra") `.spv` byte-identity gate — the re-DXC oracle for the three new
//! `_froxel` VisibilityBuffer shading variants.
//!
//! `vb_resolve.comp.hlsl`/`vb_shade.comp.hlsl` grew an `#ifdef FROXEL` seam at rung VB-P1a
//! (`docs/VB-PERFORMANCE-TRACK.md`): the froxel-culled point/spot walk, compiled in ONLY under
//! `-D FROXEL=1`. This test clones the `redxc_with_defines` idiom `marcher_spv_sync.rs` uses for
//! the SDF marcher `{HAS_MESH} x {VIEWT}` matrix, scoped to the three froxel VB variants:
//!
//! * `vb_resolve_froxel.comp.spv` (`vb_resolve.comp.hlsl`, `-D FROXEL=1`)
//! * `vb_shade_froxel.comp.spv` (`vb_shade.comp.hlsl`, `-D FROXEL=1`)
//! * `vb_shade_tex_froxel.comp.spv` (`vb_shade.comp.hlsl`, `-D TEXTURED=1 -D FROXEL=1`)
//!
//! The arm bit (`ResolvedRenderPath::froxel_light_cull`) is **default-OFF, not hardcoded off** —
//! an owner opt-in via `LightingConfig::clusters_enabled` — so a DEFAULT boot loads none of these
//! modules, while `vb_mesh_froxel` / `vb_mesh_tex_froxel` do load them and are golden-pinned.
//! Either way the committed `.spv` must byte-match their source under the frozen recipe, exactly
//! like every other variant family in this crate.
//!
//! SKIPS (with an eprintln) when no `dxc` resolves on the host — the byte gate is only as
//! hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed artifacts; a
//! DIFFERENT dxc version failing this test means "wrong toolchain", not "drifted shader" (the
//! committed recipe headers pin the exact version).

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

/// Re-DXCs `hlsl_name` (relative to the shaders dir) under the EXACT frozen recipe
/// (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, NO `-O`) plus the given `-D` defines,
/// into a fresh temp `.spv` named by `out_tag` (distinct per variant so parallel test binaries
/// never collide), and returns the bytes. Never overwrites a committed artifact.
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
fn vb_froxel_variant_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_froxel_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no $VULKAN_SDK/Bin, \
             not on PATH) — SKIPPING the VB-froxel re-DXC byte-identity check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let variants: [(&str, &[&str], &str); 3] = [
        ("vb_resolve.comp.hlsl", &["FROXEL=1"], "vb_resolve_froxel.comp.spv"),
        ("vb_shade.comp.hlsl", &["FROXEL=1"], "vb_shade_froxel.comp.spv"),
        ("vb_shade.comp.hlsl", &["TEXTURED=1", "FROXEL=1"], "vb_shade_tex_froxel.comp.spv"),
    ];
    for (hlsl_name, defines, spv_name) in variants {
        assert_spv_byte_identical(&dxc, &dir, hlsl_name, defines, spv_name);
    }
}

/// VB-P1a byte-identity oracle (the whole point of the rung): an unarmed boot takes the base arm,
/// so the BASE (non-FROXEL) `.spv` for every VB shading variant this seam touched must stay
/// byte-identical to its pre-VB-P1a build — the `#else` arm is token-for-token the prior flat
/// scan, so a base compile (no `-D FROXEL`) is physically unperturbed by the seam.
#[test]
fn vb_base_variant_spv_unperturbed_by_the_froxel_seam() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_froxel_spv_sync: dxc not found — SKIPPING the VB base-variant byte-identity \
             check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let variants: [(&str, &[&str], &str); 3] = [
        ("vb_resolve.comp.hlsl", &[], "vb_resolve.comp.spv"),
        ("vb_shade.comp.hlsl", &[], "vb_shade.comp.spv"),
        ("vb_shade.comp.hlsl", &["TEXTURED=1"], "vb_shade_tex.comp.spv"),
    ];
    for (hlsl_name, defines, spv_name) in variants {
        assert_spv_byte_identical(&dxc, &dir, hlsl_name, defines, spv_name);
    }
}
