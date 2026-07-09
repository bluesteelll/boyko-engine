//! SDFDDGI I2 — the `sdf_probe_update` eDSL generator drift gates + the CPU unit oracles
//! (`feature = "emit"` for the emit drift half; the eval half needs only the default build).
//!
//! Two halves (plan §6 gate 3-4, §9):
//!
//! 1. **Emit drift gates** — assert the committed `sdf_probe_update.comp.hlsl` still `.contains`
//!    the EXACT eDSL-generated span (`oct_decode` / `probe_march` / `probe_blend` /
//!    `probe_depth_blend`). A hand-edit of a spliced span fails here. The fix: re-run `cargo run -p
//!    boyko_shaderdsl --features emit --bin emit_probe_gi` + re-DXC `sdf_probe_update.comp.spv`.
//!    (A-1: `GI_MAX_IT` is a spec-const now, so ONE source carries every span — the former 4
//!    baked-const variant files are gone.)
//! 2. **CPU unit oracles** — `probe_march_body::<EvalCf>` hits a unit sphere / escapes to sky;
//!    `probe_blend_body`/`probe_depth_blend_body` normalize correctly (all-equal rays → uniform;
//!    a single ray → a cosine peak); `oct_decode_body::<EvalCf>` (after `normalize`) == the
//!    `oct_encode` inverse round-trip. The cross-crate sync pins
//!    (`oct_decode_edsl_matches_host` vs `goldens::oct_decode`,
//!    `sdf_soft_shadow_ranged_copy_matches_resolve` vs `deferred_pbr.hlsl`) live in
//!    `boyko_rhi_vulkan` (they need `goldens`/the committed shader, which this crate must not
//!    dev-depend on).

// ============================ CPU unit oracles (default build) ============================

use std::cell::Cell;

use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::oct::{oct_decode_body, oct_encode_body};
use boyko_shaderdsl::probe_blend::{probe_blend_body, probe_depth_blend_body};
use boyko_shaderdsl::probe_march::{GI_T_MAX, probe_march_body};

/// The guarded unit-vector (a byte-mirror of `boyko_sdf_math::v_normalize`) — the `oct_decode`
/// tail the emitter prints textually; applied here to compare the eDSL PRE-normalize lanes to
/// the fully-normalized `oct_encode` round-trip / host mirror.
fn v_normalize(a: [f32; 3]) -> [f32; 3] {
    let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if len <= f32::MIN_POSITIVE || !len.is_finite() {
        return [0.0, 0.0, 0.0];
    }
    [a[0] / len, a[1] / len, a[2] / len]
}

/// Runs `probe_march_body::<EvalCf>` over a host field closure, returning `(hit, t)`.
fn march<Fld: Fn([f32; 3]) -> f32>(ro: [f32; 3], rd: [f32; 3], field: &Fld) -> (bool, f32) {
    let t_out = Cell::new(0.0f32);
    let hit_out = Cell::new(false);
    probe_march_body::<EvalCf, _>(ro, rd, field, &t_out, &hit_out);
    (hit_out.get(), t_out.get())
}

#[test]
fn probe_march_hits_a_unit_sphere_at_expected_distance() {
    // A unit sphere at (0, 0, 3). A ray from the origin straight down +Z marches to the near
    // surface at t ≈ 2 (sphere center 3, radius 1). The hit flag is set and `t` lands near 2.
    let sphere = |q: [f32; 3]| -> f32 {
        let (dx, dy, dz) = (q[0], q[1], q[2] - 3.0);
        (dx * dx + dy * dy + dz * dz).sqrt() - 1.0
    };
    let (hit, t) = march([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], &sphere);
    assert!(hit, "a ray aimed at the unit sphere must hit");
    // The near surface is at distance 2; the sphere-trace stops within GI_HIT_EPS of it.
    assert!(
        (t - 2.0).abs() < 0.05,
        "the marched hit distance must be ~2 (sphere center 3, radius 1), got t = {t}"
    );
}

#[test]
fn probe_march_escapes_to_sky_on_a_miss() {
    // The SAME sphere, but a ray pointing AWAY (-Z) never approaches it: the march runs the
    // budget out and escapes past GI_T_MAX with hit == false.
    let sphere = |q: [f32; 3]| -> f32 {
        let (dx, dy, dz) = (q[0], q[1], q[2] - 3.0);
        (dx * dx + dy * dy + dz * dz).sqrt() - 1.0
    };
    let (hit, t) = march([0.0, 0.0, 0.0], [0.0, 0.0, -1.0], &sphere);
    assert!(!hit, "a ray pointing away from all geometry must escape to sky");
    assert!(
        t > GI_T_MAX,
        "an escaped ray's t must exceed GI_T_MAX ({GI_T_MAX}), got t = {t}"
    );
}

#[test]
fn probe_blend_all_equal_rays_give_uniform_irradiance() {
    // Every ray carries the SAME radiance L and a direction; the cosine-weighted sum, divided by
    // the weight sum, recovers exactly L regardless of the texel direction (a uniform field).
    let texel_dir = [0.0f32, 0.0, 1.0];
    let l = [0.3f32, 0.6, 0.9];
    // A hemisphere of rays around +Z (all with dot > 0 against the texel).
    let rays = [
        [0.0f32, 0.0, 1.0],
        [0.5, 0.0, 0.866],
        [-0.5, 0.0, 0.866],
        [0.0, 0.5, 0.866],
    ];
    let (mut sr, mut sg, mut sb, mut sw) = (0.0f32, 0.0, 0.0, 0.0);
    for rd in rays {
        let (nr, ng, nb, nw) =
            probe_blend_body::<EvalCf>(texel_dir, rd, l[0], l[1], l[2], sr, sg, sb, sw);
        sr = nr;
        sg = ng;
        sb = nb;
        sw = nw;
    }
    // Uniform radiance ⇒ the weighted mean IS L (the weights cancel).
    let irr = [sr / sw, sg / sw, sb / sw];
    for k in 0..3 {
        assert!(
            (irr[k] - l[k]).abs() < 1e-5,
            "all-equal-radiance rays must average to L[{k}] = {}, got {}",
            l[k],
            irr[k]
        );
    }
}

#[test]
fn probe_blend_single_ray_peaks_at_the_aligned_texel() {
    // ONE ray straight up +Z. Its cosine weight against a texel is `max(0, dot)`: the aligned
    // texel (+Z) gets full weight (irr == L), a perpendicular texel (+X) gets zero weight (the
    // divide guard yields ~0), and a back-facing texel (-Z) gets zero.
    let ray = [0.0f32, 0.0, 1.0];
    let l = [1.0f32, 0.5, 0.25];
    let blend_one = |texel: [f32; 3]| -> ([f32; 3], f32) {
        let (sr, sg, sb, sw) =
            probe_blend_body::<EvalCf>(texel, ray, l[0], l[1], l[2], 0.0, 0.0, 0.0, 0.0);
        ([sr, sg, sb], sw)
    };

    // Aligned texel (+Z): weight 1, sum == L.
    let (aligned, w_aligned) = blend_one([0.0, 0.0, 1.0]);
    assert!((w_aligned - 1.0).abs() < 1e-6, "the aligned texel weight must be 1");
    for k in 0..3 {
        assert!((aligned[k] - l[k]).abs() < 1e-6, "aligned texel must accumulate L[{k}]");
    }

    // Perpendicular texel (+X): dot == 0 ⇒ weight 0 ⇒ no contribution.
    let (_, w_perp) = blend_one([1.0, 0.0, 0.0]);
    assert!(w_perp.abs() < 1e-6, "a perpendicular texel gets zero cosine weight");

    // Back-facing texel (-Z): dot < 0 ⇒ clamped to 0.
    let (_, w_back) = blend_one([0.0, 0.0, -1.0]);
    assert!(w_back.abs() < 1e-6, "a back-facing texel gets zero (clamped) weight");
}

#[test]
fn probe_depth_blend_accumulates_two_moments() {
    // A single ray with distance t: the mean is t and the second moment is t² (weight 1 for the
    // aligned texel), so the written tile `(dmean/dw, dmean2/dw)` is `(t, t²)`.
    let texel_dir = [0.0f32, 0.0, 1.0];
    let ray = [0.0f32, 0.0, 1.0];
    let t = 2.5f32;
    let (dmean, dmean2, dw) = probe_depth_blend_body::<EvalCf>(texel_dir, ray, t, 0.0, 0.0, 0.0);
    assert!((dw - 1.0).abs() < 1e-6, "aligned single-ray weight must be 1");
    assert!((dmean / dw - t).abs() < 1e-5, "the depth mean must be t = {t}");
    assert!(
        (dmean2 / dw - t * t).abs() < 1e-4,
        "the second moment must be t² = {}",
        t * t
    );
}

/// Runs `oct_encode_body::<EvalCf>` returning the `[0,1]²` pair.
fn oct_encode(n: [f32; 3]) -> [f32; 2] {
    let out = Cell::new([0.0f32; 2]);
    let _ = oct_encode_body::<EvalCf>(n, &out);
    out.get()
}

/// Runs `oct_decode_body::<EvalCf>` and applies the emitter's textual `normalize` tail.
fn oct_decode(e: [f32; 2]) -> [f32; 3] {
    v_normalize(oct_decode_body::<EvalCf>(e[0], e[1]))
}

#[test]
fn oct_decode_is_the_inverse_of_oct_encode() {
    // The octahedral encode/decode round-trip: encode a unit normal, decode it, and recover the
    // original direction (to floating tolerance). Both hemispheres + all four sign quadrants are
    // exercised so the `if (n.z < 0)` fold and the two sign-ternaries round-trip.
    let normals: [[f32; 3]; 10] = [
        v_normalize([0.0, 0.0, 1.0]),
        v_normalize([0.0, 0.0, -1.0]),
        v_normalize([1.0, 0.0, 0.5]),
        v_normalize([-1.0, 0.0, 0.5]),
        v_normalize([0.0, 1.0, -0.5]),
        v_normalize([0.0, -1.0, -0.5]),
        v_normalize([0.6, 0.6, 0.6]),
        v_normalize([-0.6, 0.6, -0.6]),
        v_normalize([0.6, -0.6, -0.6]),
        v_normalize([0.3, -0.8, 0.5]),
    ];
    for n in normals {
        let round = oct_decode(oct_encode(n));
        let dot = n[0] * round[0] + n[1] * round[1] + n[2] * round[2];
        assert!(
            dot > 0.9995,
            "oct_decode must invert oct_encode: n = {n:?}, round-trip = {round:?}, dot = {dot}"
        );
    }
}

// ================================ Emit drift gates (feature = "emit") =========================

#[cfg(feature = "emit")]
mod emit_drift {
    use std::path::PathBuf;

    fn shaders_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("boyko_rhi_vulkan")
            .join("shaders")
    }

    /// Asserts the committed `sdf_probe_update.comp.hlsl` still contains the exact eDSL-generated
    /// `span`. (A-1: the former 4 `GI_MAX_IT` variant files collapsed to ONE — `GI_MAX_IT` is now a
    /// spec-const, so a single source carries every eDSL span.)
    fn assert_span_in_all_variants(span: &str, which: &str) {
        let span = span.replace("\r\n", "\n");
        let path = shaders_dir().join("sdf_probe_update.comp.hlsl");
        let shader = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("invariant: {} must exist: {e}", path.display()))
            .replace("\r\n", "\n");
        assert!(
            shader.contains(&span),
            "sdf_probe_update `{which}` span DRIFTED from boyko_shaderdsl::emit — the committed span \
             no longer matches the generator. Re-run `cargo run -p boyko_shaderdsl --features emit \
             --bin emit_probe_gi` + re-DXC `sdf_probe_update.comp.spv`.\n--- expected \
             (eDSL-generated) ---\n{span}"
        );
    }

    #[test]
    fn oct_decode_matches_edsl_emit() {
        assert_span_in_all_variants(&boyko_shaderdsl::emit::emit_hlsl_oct_decode(), "oct_decode");
    }

    #[test]
    fn probe_march_matches_edsl_emit() {
        assert_span_in_all_variants(&boyko_shaderdsl::emit::emit_hlsl_probe_march(), "probe_march");
    }

    #[test]
    fn probe_blend_matches_edsl_emit() {
        assert_span_in_all_variants(&boyko_shaderdsl::emit::emit_hlsl_probe_blend(), "probe_blend");
    }

    #[test]
    fn probe_depth_blend_matches_edsl_emit() {
        assert_span_in_all_variants(
            &boyko_shaderdsl::emit::emit_hlsl_probe_depth_blend(),
            "probe_depth_blend",
        );
    }
}
