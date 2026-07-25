//! VB-SV0 rung S0 — the OFF-path harness (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §6 rung S0, §3.3
//! instrument G1). SV0 is not built yet: this rung answers one question before it is, so the
//! runtime-vs-`-D` decision (§3.2) does not ride on an unvalidated instrument for three more rungs.
//!
//! **No shader edit, no production code.** This clones the `redxc_with_defines` /
//! `assert_spv_byte_identical` / `find_dxc` idiom `cluster_cull_spv_sync.rs` and
//! `vb_froxel_spv_sync.rs` already established, scoped to the **ten shipping VB lit-producer
//! `.spv`** (`docs/SHADER-VARIANT-MANIFEST.md:91-107`, `compute.rs:811-1037`):
//!
//! * `vb_resolve.comp.hlsl` -> `vb_resolve.comp.spv`, `vb_resolve_froxel.comp.spv` (`-D FROXEL=1`).
//! * `vb_shade.comp.hlsl` -> `vb_shade.comp.spv`, `vb_shade_tex.comp.spv` (`-D TEXTURED=1`),
//!   `vb_shade_froxel.comp.spv` (`-D FROXEL=1`), `vb_shade_tex_froxel.comp.spv`
//!   (`-D TEXTURED=1 -D FROXEL=1`).
//! * `vb_shade_split.comp.hlsl` -> `vb_shade_split.comp.spv`, `vb_shade_split_tex.comp.spv`
//!   (`-D TEXTURED=1`), `vb_shade_split_hwrt.comp.spv` (`-D HWRT=1`),
//!   `vb_shade_split_tex_hwrt.comp.spv` (`-D TEXTURED=1 -D HWRT=1`).
//!
//! Two gates:
//!
//! 1. **Reproduction** — each of the ten rows, re-DXC'd under its own frozen recipe
//!    (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, no `-O`), is byte-identical to its
//!    committed `.spv`. RED here is a PRE-EXISTING defect this rung surfaces (the frozen recipe no
//!    longer reproduces), not something SV0 introduced.
//! 2. **Sensitivity — the assertion that validates the instrument.** A scratch copy of
//!    `vb_resolve.comp.hlsl` has its `:258` `NoV` epsilon (`1e-4`, a region SV0 will never touch —
//!    the term sits before any light-loop or AO/shadow combine) changed to `2e-4`, re-DXC'd via
//!    `-I` (never touching the committed source), and the resulting bytes must DIFFER from the
//!    committed `vb_resolve.comp.spv`. RED here means a re-DXC byte comparison is blind for these
//!    modules — §3.3's G1 kill-switch could not detect an SV0 guard placed outside its
//!    `#ifndef VB_SV0_KILL` span, and the runtime-gate design collapses to the `-D` fallback
//!    (§7 clause 1). Both outcomes are results worth having; this test does not get tuned to force
//!    a green.
//!
//! SKIPS (with an eprintln) when no `dxc` resolves on the host, exactly like the precedent files —
//! a DIFFERENT dxc version failing this test means "wrong toolchain", not "drifted shader".

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC must run so any `#include` resolves). Mirrors `cluster_cull_spv_sync.rs`.
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `dxc` executable: first the pinned Vulkan-SDK path (the repo's offline recipe),
/// then `$VULKAN_SDK/Bin`, then `PATH`. Returns `None` if none resolve (the byte-identity tests
/// then SKIP) — the `cluster_cull_spv_sync.rs` idiom verbatim.
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
/// (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, no `-O`) plus the given `-D` defines,
/// into a fresh temp `.spv` named by `out_tag` (distinct per variant so parallel test binaries
/// never collide), and returns the bytes. Never overwrites a committed artifact. Mirrors
/// `cluster_cull_spv_sync.rs:73-86`.
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

/// One committed artifact must byte-equal its own re-DXC. Mirrors `cluster_cull_spv_sync.rs:89-103`.
fn assert_spv_byte_identical(dxc: &PathBuf, dir: &PathBuf, hlsl_name: &str, defines: &[&str], spv_name: &str) {
    let committed_path = dir.join(spv_name);
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc_with_defines(dxc, dir, hlsl_name, defines, spv_name);
    assert!(
        committed == fresh,
        "{spv_name} ({} bytes committed, {} bytes fresh) is NOT the re-DXC of {hlsl_name} \
         {defines:?} under the frozen recipe — either the committed .spv is stale (re-run the \
         recipe in the shader's header and commit it) or the host dxc is not the pinned \
         VulkanSDK 1.4.350.0 toolchain. This is a PRE-EXISTING defect S0 surfaces, not one SV0 \
         introduces.",
        committed.len(),
        fresh.len(),
    );
}

/// A compact 64-bit FNV-1a fingerprint, used ONLY to make the S0 sensitivity report human-readable
/// in `--nocapture` output. Not a security or build-integrity primitive — the actual gate below
/// compares the full byte vectors, never this hash.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The ten shipping VB lit-producer `.spv`, each `(source, defines, committed .spv name)` — derived
/// from `docs/SHADER-VARIANT-MANIFEST.md:91-107` and `compute.rs:811-1037`, matching
/// `docs/VB-SV0-SDF-SHADOW-PLAN.md` §3.2's and S4's own row table.
const VB_SV0_ROWS: [(&str, &[&str], &str); 10] = [
    ("vb_resolve.comp.hlsl", &[], "vb_resolve.comp.spv"),
    ("vb_resolve.comp.hlsl", &["FROXEL=1"], "vb_resolve_froxel.comp.spv"),
    ("vb_shade.comp.hlsl", &[], "vb_shade.comp.spv"),
    ("vb_shade.comp.hlsl", &["TEXTURED=1"], "vb_shade_tex.comp.spv"),
    ("vb_shade.comp.hlsl", &["FROXEL=1"], "vb_shade_froxel.comp.spv"),
    ("vb_shade.comp.hlsl", &["TEXTURED=1", "FROXEL=1"], "vb_shade_tex_froxel.comp.spv"),
    ("vb_shade_split.comp.hlsl", &[], "vb_shade_split.comp.spv"),
    ("vb_shade_split.comp.hlsl", &["TEXTURED=1"], "vb_shade_split_tex.comp.spv"),
    ("vb_shade_split.comp.hlsl", &["HWRT=1"], "vb_shade_split_hwrt.comp.spv"),
    ("vb_shade_split.comp.hlsl", &["TEXTURED=1", "HWRT=1"], "vb_shade_split_tex_hwrt.comp.spv"),
];

/// S0 gate (1) — reproduction: every one of the ten shipping VB lit-producer `.spv`, re-DXC'd
/// under its own frozen recipe, byte-equals its committed artifact. RED for any row means the
/// frozen recipe no longer reproduces on this host — a pre-existing defect, surfaced before SV0
/// builds three more rungs on top of it.
#[test]
fn vb_sv0_offpath_ten_rows_reproduce_under_frozen_recipe() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_sv0_offpath: dxc not found (no C:/VulkanSDK/.../dxc.exe, no $VULKAN_SDK/Bin, not \
             on PATH) — SKIPPING the S0 reproduction check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    for (hlsl_name, defines, spv_name) in VB_SV0_ROWS {
        assert_spv_byte_identical(&dxc, &dir, hlsl_name, defines, spv_name);
    }
    eprintln!("vb_sv0_offpath: all {} rows reproduced byte-identically.", VB_SV0_ROWS.len());
}

/// S0 gate (2) — the harness sensitivity control (§3.3's validation, before G1 is trusted): a
/// scratch copy of `vb_resolve.comp.hlsl` has its `:258` `NoV` epsilon (`1e-4` -> `2e-4`, a region
/// SV0 will never touch — it precedes every light-loop and AO/shadow combine) re-DXC'd via `-I`
/// (never touching the committed source or the committed `.spv`), and the resulting bytes must
/// DIFFER from the committed `vb_resolve.comp.spv`.
///
/// RED here means a re-DXC byte comparison is blind for these modules: §3.3's G1 kill-switch could
/// not detect an SV0 guard placed outside its `#ifndef VB_SV0_KILL` span, and the runtime-gate
/// design (§3.2) collapses to the `-D SV0=1` fallback — an owner VALUES call (§7 clause 1), not a
/// test to retune.
#[test]
fn vb_sv0_offpath_harness_is_sensitive_to_an_untouched_literal() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_sv0_offpath: dxc not found (no C:/VulkanSDK/.../dxc.exe, no $VULKAN_SDK/Bin, not \
             on PATH) — SKIPPING the S0 sensitivity control on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let source = std::fs::read_to_string(dir.join("vb_resolve.comp.hlsl"))
        .expect("invariant: vb_resolve.comp.hlsl is the committed shader source");
    let needle = "max(dot(n, v), 1e-4)";
    assert!(
        source.contains(needle),
        "invariant: {needle:?} must appear verbatim in vb_resolve.comp.hlsl (line 258's `NoV` \
         epsilon) for this mutation to be meaningful — if the expression changed, update this test"
    );
    let mutated = source.replacen(needle, "max(dot(n, v), 2e-4)", 1);

    let scratch_path = std::env::temp_dir().join("vb_sv0_offpath_nov_epsilon_mutant.hlsl");
    std::fs::write(&scratch_path, &mutated).expect("invariant: temp dir is writable");
    let out_spv = std::env::temp_dir().join("vb_sv0_offpath_nov_epsilon_mutant.spv");
    // `-I <shaders_dir>` lets the mutant (living outside the shaders dir) still resolve its
    // `#include`s against the real, unmodified headers — the `cluster_cull_hier_dis_gate.rs:539`
    // idiom for a scratch-copy compile that must never touch the committed tree.
    let status = Command::new(&dxc)
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3", "-I"])
        .arg(&dir)
        .arg(&scratch_path)
        .arg("-Fo")
        .arg(&out_spv)
        .status()
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed compiling the mutated vb_resolve.comp.hlsl scratch copy");
    let mutated_bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the mutant .spv");
    let _ = std::fs::remove_file(&scratch_path);
    let _ = std::fs::remove_file(&out_spv);

    let committed = std::fs::read(dir.join("vb_resolve.comp.spv"))
        .expect("invariant: vb_resolve.comp.spv is the committed artifact");

    eprintln!(
        "vb_sv0_offpath sensitivity control: vb_resolve.comp.hlsl:258 `1e-4` -> `2e-4`; committed \
         vb_resolve.comp.spv ({} bytes, fnv1a_64={:#018x}) vs mutant re-DXC ({} bytes, \
         fnv1a_64={:#018x})",
        committed.len(),
        fnv1a_64(&committed),
        mutated_bytes.len(),
        fnv1a_64(&mutated_bytes),
    );

    assert!(
        committed != mutated_bytes,
        "S0 RED: vb_resolve.comp.hlsl's NoV epsilon 1e-4 -> 2e-4 (a region SV0 never touches) \
         re-DXC'd to a BYTE-IDENTICAL .spv. A re-DXC byte comparison is therefore BLIND for this \
         module — §3.3's G1 kill-switch cannot detect an SV0 guard placed outside its \
         `#ifndef VB_SV0_KILL` span, and the runtime-gate design collapses to the `-D` fallback \
         (§7 clause 1). This is a real finding — do not tune the mutation to force a green."
    );
}
