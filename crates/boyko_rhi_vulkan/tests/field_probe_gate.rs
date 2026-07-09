//! GATE-1 — the determinism-frozen field-function tripwire (PBR MVP-2 Phase 0).
//!
//! The #1 risk of the PBR MVP-2 marcher refactor (helper split, ray-gen
//! extraction, material-id pick, full Cook-Torrance shading rewrite) is silently
//! perturbing the byte-shared, determinism-frozen SDF field eval in
//! `shaders/sdf_field.hlsli`. DXC fully INLINES every helper into `main`, so the
//! marcher's own SPIR-V is one flat `OpFunction` where field ops interleave with
//! shading ops — a per-function diff of the marcher cannot isolate the field math.
//!
//! `shaders/sdf_field_probe.hlsl` sidesteps that: it calls ONLY the frozen field
//! gateway (`field_distance` + `sdf_normal`) and writes 4 floats — its entire
//! instruction stream IS the field math. This test re-`spirv-dis`-es the committed
//! probe `.spv` and asserts the disassembly is BYTE-IDENTICAL to the committed
//! baseline (`shaders/sdf_field_probe.baseline.dis`), captured from the pre-refactor
//! (PBR MVP-1) field. Any field perturbation — the catastrophic failure mode — trips
//! this loudly, and it is the empirical proof the marcher refactor was field-neutral.
//!
//! # Why the committed `.spv`, not a fresh compile
//!
//! The committed `.spv` is the artifact the engine actually loads
//! ([`boyko_rhi_vulkan::compute`] never recompiles HLSL at runtime — DXC runs only
//! offline). Disassembling THAT proves the shipped field bytes still match the frozen
//! baseline. A fresh DXC compile would instead test the local toolchain; the repo
//! policy is "DXC → `.spv` committed", so the `.spv` is the source of truth here.
//!
//! # Toolchain gate
//!
//! `spirv-dis` ships with the Vulkan SDK. The CI / GPU-tester box (RTX 3060) has it
//! on `PATH` or under `$VULKAN_SDK/Bin`. On a host without it the test SKIPS (prints
//! a notice and returns) rather than failing — the byte-identity claim cannot be
//! evaluated without the disassembler, and a missing SDK is an environment gap, not a
//! field regression.

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where both the committed
/// probe `.spv` and its baseline `.dis` live.
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `spirv-dis` executable: first on `PATH`, then under
/// `$VULKAN_SDK/Bin`. Returns `None` if neither resolves (the test then SKIPS).
fn find_spirv_dis() -> Option<PathBuf> {
    // Prefer a PATH-resolvable `spirv-dis` (the SDK adds its `Bin` to PATH).
    let bare = if cfg!(windows) { "spirv-dis.exe" } else { "spirv-dis" };
    if Command::new(bare).arg("--version").output().is_ok() {
        return Some(PathBuf::from(bare));
    }
    // Fall back to `$VULKAN_SDK/Bin/spirv-dis`.
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Disassembles `spv_path` via `spirv-dis`, returning the textual SPIR-V. Panics on a
/// non-zero exit (a malformed committed `.spv` is a build-integrity bug, not a skip).
fn disassemble(spirv_dis: &PathBuf, spv_path: &PathBuf) -> String {
    let out = Command::new(spirv_dis)
        .arg(spv_path)
        .output()
        .expect("invariant: spirv-dis was located and must run");
    assert!(
        out.status.success(),
        "spirv-dis failed on {}: {}",
        spv_path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("invariant: spirv-dis emits UTF-8 disassembly")
}

/// Normalizes line endings so a CRLF-checkout of the baseline `.dis` does not produce
/// a spurious mismatch against `spirv-dis`'s LF output (and vice versa).
fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[test]
fn field_function_byte_identity() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "field_probe_gate: spirv-dis not found (not on PATH, no $VULKAN_SDK/Bin) — SKIPPING \
             the GATE-1 field byte-identity check on this host."
        );
        return;
    };

    let dir = shaders_dir();
    let spv = dir.join("sdf_field_probe.comp.spv");
    let baseline_path = dir.join("sdf_field_probe.baseline.dis");

    assert!(
        spv.exists(),
        "missing committed probe SPIR-V: {} (compile sdf_field_probe.hlsl with DXC and commit it)",
        spv.display()
    );
    let baseline = std::fs::read_to_string(&baseline_path).unwrap_or_else(|e| {
        panic!(
            "missing committed GATE-1 baseline {}: {e} (capture it with \
             `spirv-dis sdf_field_probe.comp.spv > sdf_field_probe.baseline.dis`)",
            baseline_path.display()
        )
    });

    let actual = disassemble(&spirv_dis, &spv);

    assert_eq!(
        normalize(&actual),
        normalize(&baseline),
        "GATE-1 TRIPWIRE: the committed sdf_field_probe.comp.spv disassembly DIVERGED from the \
         frozen baseline. The determinism-frozen SDF field eval (sdf_field.hlsli) was perturbed \
         — this is the #1 risk of the PBR MVP-2 refactor. If the field change is INTENTIONAL \
         (an owner-approved field fork), re-capture the baseline AND re-bake the distance/depth \
         golden + cpu_gpu_sdf_agreement. Otherwise REVERT the field change."
    );
}
