//! SDFDDGI I2 — the cross-crate sync pins for the probe-update shader (plan §6 gate 4).
//!
//! These two gates need artifacts this test crate CAN reach but the `boyko_shaderdsl` crate
//! cannot (the committed `deferred_pbr.hlsl` + the `boyko_rhi_vulkan::goldens` host mirror), so
//! they live here rather than in `boyko_shaderdsl/tests/emit_probe_gi.rs`:
//!
//! 1. `sdf_soft_shadow_ranged_copy_matches_resolve` — the `sdf_soft_shadow_ranged` function the
//!    probe-update shader COPIES must stay token-identical (indentation-normalized) to the one the
//!    resolve consumes (plan §1.1). A drift means the GI shadow march diverged from the resolve's
//!    — the exact silent-fork the copy discipline guards. (The compiled byte-identity is gated
//!    separately; this is the source-level behavioral guard — see the extractor's doc.)
//!
//!    **VB-SV0 rung S2 re-targeted this from `deferred_pbr.hlsl` to `sdf_shadow_leaves.hlsli`**
//!    (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §4.1). SV0 needs the same leaf in the three VB
//!    lit-producer tails, so the function MOVED verbatim out of `deferred_pbr.hlsl` into a shared
//!    header, which `deferred_pbr.hlsl` now `#include`s at the point the span occupied. The pin's
//!    MEANING is unchanged — what it asserts is that the probe-update copy equals the resolve's,
//!    and that is independent of which file the resolve's copy is spelled in. What changed is
//!    only where the extractor looks; leaving it pointed at `deferred_pbr.hlsl` would panic on the
//!    missing signature, which is precisely the red this rung had to clear.
//! 2. `oct_decode_edsl_matches_host` — the new eDSL `oct_decode_body::<EvalCf>` (after the
//!    `normalize` tail the emitter prints textually) must equal the I0b host mirror
//!    `goldens::oct_decode` to floating tolerance over the whole `[0,1]²` domain (plan §6 gate 4,
//!    the P0-2 fix — NOT the phantom `_matches_resolve`). This certifies the WRITE-side decode
//!    the I2 pass uses is the SAME decode the I0b oracle chain already proved math-correct.

#![cfg(feature = "goldens")]

use std::path::PathBuf;

use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::oct::oct_decode_body;

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Extracts the `float sdf_soft_shadow_ranged(...) { ... }` function body text from `hlsl` (the
/// signature line through the matching closing brace), normalizing line endings AND per-line
/// leading/trailing whitespace. Panics if the function is absent (a malformed shader — surfaced
/// loudly).
///
/// # Why indentation is normalized (not a byte-for-byte compare)
///
/// The pin's invariant is that the GI shadow march is **behaviorally** identical to the resolve's:
/// every token, constant, operator, and statement structure must match. Indentation is inert —
/// HLSL is whitespace-insensitive, so the copy is spliced flush-left into the generated shader
/// while the resolve carries the frozen file's 4-space body indent. Trimming each line ignores that
/// (a spurious reindent of the frozen resolve would otherwise break the pin) while preserving every
/// token and the line structure (a real change — a renamed symbol, a changed constant, a collapsed
/// statement — still fails). The **compiled** byte-identity is guaranteed separately by the emit
/// drift gate + the committed `.spv` re-DXC check; this is the source-level behavioral guard.
fn extract_soft_shadow_ranged(hlsl: &str, which: &str) -> String {
    let hlsl = hlsl.replace("\r\n", "\n");
    let sig = "float sdf_soft_shadow_ranged(float3 p, float3 n, float3 L, float t_max) {";
    let start = hlsl
        .find(sig)
        .unwrap_or_else(|| panic!("{which} must contain the `sdf_soft_shadow_ranged` signature"));
    // Brace-match from the signature's opening `{` to its closing `}`.
    let bytes = hlsl.as_bytes();
    let mut depth = 0i32;
    let mut i = start;
    let mut end = None;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let end = end.unwrap_or_else(|| panic!("{which} `sdf_soft_shadow_ranged` has an unmatched brace"));
    // Trim per-line leading/trailing whitespace so the flush-left splice compares equal to the
    // frozen resolve's indented body while every token + the line structure stay strict.
    hlsl[start..end]
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn sdf_soft_shadow_ranged_copy_matches_resolve() {
    // The probe-update shader COPIES `sdf_soft_shadow_ranged` from the shared leaf header (VB-SV0
    // S2 moved it there out of `deferred_pbr.hlsl`; the generator constant
    // `emit_probe_gi.rs::SDF_SOFT_SHADOW_RANGED_COPY` is deliberately NOT re-pointed, so the probe
    // shader and its frozen `.spv` do not move). Extract both function bodies and assert
    // token-equality (indentation-normalized, see the extractor): a drift means the GI shadow
    // march no longer matches the resolve's, the silent-fork the plan §1.1 copy discipline guards.
    // (A-1: ONE `sdf_probe_update.comp.hlsl`, `GI_MAX_IT` now a spec-const.)
    let resolve = std::fs::read_to_string(shaders_dir().join("sdf_shadow_leaves.hlsli"))
        .expect("invariant: shaders/sdf_shadow_leaves.hlsli must exist next to this crate");
    let update = std::fs::read_to_string(shaders_dir().join("sdf_probe_update.comp.hlsl"))
        .expect(
            "invariant: shaders/sdf_probe_update.comp.hlsl must exist (run `cargo run -p \
             boyko_shaderdsl --features emit --bin emit_probe_gi`)",
        );

    let resolve_fn = extract_soft_shadow_ranged(&resolve, "sdf_shadow_leaves.hlsli");
    let update_fn = extract_soft_shadow_ranged(&update, "sdf_probe_update.comp.hlsl");

    assert_eq!(
        update_fn, resolve_fn,
        "the probe-update shader's copied `sdf_soft_shadow_ranged` DRIFTED from the committed \
         `sdf_shadow_leaves.hlsli` function it was copied from (indentation-normalized token \
         compare). The GI shadow march must stay behaviorally identical to the resolve's. Re-copy \
         the function into `emit_probe_gi`'s `SDF_SOFT_SHADOW_RANGED_COPY` and re-run the emitter \
         + re-DXC."
    );
}

#[test]
fn oct_decode_edsl_matches_host() {
    // The new eDSL `oct_decode_body::<EvalCf>` (after the `normalize` tail) must equal the I0b
    // host mirror `goldens::oct_decode` over the whole `[0,1]²` tile-UV domain. This is the P0-2
    // gate: the I2 WRITE-side decode is the SAME decode the I0b oracle chain proved math-correct,
    // so the I2 write iteration and the I3 read cannot desync. Tolerance (not bit-exact): the
    // marched-radiance write is GPU-golden + tolerance regardless (D6), and both decodes share
    // the identical `abs`/`clamp`/`select`/`normalize` op set.
    let normalize = |a: [f32; 3]| -> [f32; 3] {
        let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        if len <= f32::MIN_POSITIVE || !len.is_finite() {
            return [0.0, 0.0, 0.0];
        }
        [a[0] / len, a[1] / len, a[2] / len]
    };

    // Sweep the [0,1]² domain on a fine grid (including the corners/edges the octahedral fold
    // exercises both hemispheres over).
    const N: u32 = 33;
    let mut max_dev = 0.0f32;
    for iy in 0..N {
        for ix in 0..N {
            let ex = ix as f32 / (N - 1) as f32;
            let ey = iy as f32 / (N - 1) as f32;

            let edsl = normalize(oct_decode_body::<EvalCf>(ex, ey));
            let host = boyko_rhi_vulkan::goldens::oct_decode([ex, ey]);

            for k in 0..3 {
                max_dev = max_dev.max((edsl[k] - host[k]).abs());
            }
        }
    }
    assert!(
        max_dev < 1.0e-6,
        "the eDSL oct_decode_body::<EvalCf> DRIFTED from the host mirror goldens::oct_decode \
         (max component deviation {max_dev} > 1e-6). The I2 write-side decode must match the I0b \
         oracle chain — audit `boyko_shaderdsl::oct::oct_decode_body` against `goldens::oct_decode`."
    );
}
