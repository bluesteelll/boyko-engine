//! SDFDDGI — the `sdf_probe_update.comp.spv` byte-identity gate (the re-DXC oracle).
//!
//! # The hole this closes
//!
//! `emit_probe_gi.rs`'s module header already claims "the `emit_probe_gi` drift/sync tests pin the
//! committed `.spv` to a fresh re-DXC of the re-emitted `.hlsl`". Before this file that claim was
//! **false**: a grep of the whole tree for `sdf_probe_update.comp.spv` found the embed site, two
//! doc comments and the emitter's own recipe line — and no test. What did exist gated only the
//! *source*: `boyko_shaderdsl/tests/emit_probe_gi.rs` `.contains`-checks the four eDSL spans
//! (`oct_decode` / `probe_march` / `probe_blend` / `probe_depth_blend`), and `ddgi_probe_gi_sync.rs`
//! pins the copied `sdf_soft_shadow_ranged` and the eDSL/host decode agreement. So a `.hlsl` edit
//! whose `.spv` was never re-compiled — or a `.spv` recompiled from a *different* `.hlsl` — shipped
//! green, and the compiled blob is the thing the device actually runs.
//!
//! This clones `marcher_spv_sync.rs`'s gate verbatim for the single-variant probe-update compile.
//!
//! # The frozen recipe
//!
//! From the shader's own header (`sdf_probe_update.comp.hlsl`), ONE file, no `-D` variants —
//! `GI_MAX_IT` is a Vulkan specialization constant (id 0, default 64) resolved at pipeline-create,
//! so the `GI_MAX_IT` sweep binds values onto this one artifact rather than forking it:
//!
//! ```text
//! dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//!     sdf_probe_update.comp.hlsl -Fo sdf_probe_update.comp.spv
//! ```
//!
//! DXC must run with the shaders dir as cwd so the relative `#include "sdf_field.hlsli"` resolves.
//!
//! # Line endings are not part of the artifact (measured)
//!
//! The generator writes LF and this checkout is CRLF (`core.autocrlf=true`), so the working-tree
//! bytes of the `.hlsl` depend on checkout configuration. That does NOT leak into the gate:
//! compiling the same source in both forms was measured to produce the identical SPIR-V
//! (`f8fe2da5…` either way), so this is a gate on the shader, not on the checkout — unlike a raw
//! byte-hash of the `.hlsl` itself, which would be a checkout hash.
//!
//! SKIPS (with an `eprintln`) when no `dxc` resolves on the host: the byte gate is only as
//! hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed artifact, so a
//! DIFFERENT dxc version failing this test means "wrong toolchain", not "drifted shader".

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`) — where the committed `.hlsl`/`.spv` live
/// and where DXC must run for the relative `#include` to resolve.
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates `dxc`: the pinned Vulkan-SDK path first (the repo's offline recipe), then
/// `$VULKAN_SDK/Bin`, then `PATH`. `None` ⇒ the byte-identity test SKIPS. The
/// `marcher_spv_sync.rs` idiom verbatim.
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

/// The committed `sdf_probe_update.comp.spv` must byte-equal a fresh re-DXC of the committed
/// `sdf_probe_update.comp.hlsl` under the frozen recipe.
///
/// A red here is one of exactly two things, and the message says which to check first: the
/// committed `.spv` is stale (a `.hlsl` edit — including a re-emit from
/// `boyko_shaderdsl --bin emit_probe_gi` — that was never re-compiled), or the host `dxc` is not
/// the pinned toolchain.
#[test]
fn sdf_probe_update_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "ddgi_probe_update_spv_sync: dxc not found (no C:/VulkanSDK/1.4.350.0/Bin/dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the probe-update re-DXC byte-identity check \
             on this host."
        );
        return;
    };
    let dir = shaders_dir();

    let committed_path = dir.join("sdf_probe_update.comp.spv");
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));

    // A per-test temp name so parallel test binaries never collide; never writes into `shaders/`.
    let out_spv = std::env::temp_dir().join("sdf_probe_update.comp.redxc.spv");
    let status = Command::new(&dxc)
        .current_dir(&dir)
        .args([
            "-spirv",
            "-T",
            "cs_6_0",
            "-E",
            "main",
            "-fspv-target-env=vulkan1.3",
            "sdf_probe_update.comp.hlsl",
            "-Fo",
        ])
        .arg(&out_spv)
        .status()
        .expect("invariant: dxc was located and must run");
    assert!(
        status.success(),
        "dxc failed re-compiling sdf_probe_update.comp.hlsl under the frozen recipe"
    );
    let fresh = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy

    assert!(
        committed == fresh,
        "sdf_probe_update.comp.spv ({} bytes committed, {} bytes fresh) is NOT the re-DXC of \
         sdf_probe_update.comp.hlsl under the frozen recipe — either the committed .spv is stale \
         (re-run the recipe in the shader's header after any .hlsl edit or re-emit) or the host \
         dxc is not the pinned VulkanSDK 1.4.350.0 toolchain.",
        committed.len(),
        fresh.len(),
    );
}
