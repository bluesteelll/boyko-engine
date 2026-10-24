//! Render wave PX0 (H3 Scope 3b; `px0/cut.md` PC-PX0-6) — **the independent `f64` AgX
//! transcription**: the display transform the `lit` format decision is judged through, and the
//! reference PXt's host test 1 includes by `#[path]` (DS-H6).
//!
//! Transcribed from three.js `src/renderers/shaders/ShaderChunk/tonemapping_pars_fragment.glsl.js`
//! (`AgXToneMapping`, MIT) — itself Filament's implementation of Blender's AgX on Rec.2020
//! primaries — with every constant spelled here, so nothing is shared with an engine curve:
//! sRGB → Rec.2020, the inset matrix, `log2` over `[AGX_MIN_EV, AGX_MAX_EV]`, the 6th-order
//! sigmoid polynomial, the outset matrix, `pow(·, 2.2)`, Rec.2020 → sRGB, clamp to `[0, 1]`.
//!
//! GLSL's `mat3(a, b, c)` takes COLUMNS; the matrices below are written as the source writes those
//! columns and applied as `M · v = a·v.x + b·v.y + c·v.z` ([`mul_cols`]).
//!
//! The reported (never decisive) **Punchy look** is Filament's `agxLook` (`ToneMapper.cpp`,
//! Apache-2.0): ASC CDL with slope 1, offset 0, power 1.35, then saturation 1.4 about the Rec.709
//! luma of the value BEFORE the CDL — applied between the sigmoid and the outset (rev-6.2 H3 row).

#![allow(dead_code)] // each including test file uses the subset it needs

/// `LINEAR_SRGB_TO_LINEAR_REC2020`, as GLSL columns.
pub const SRGB_TO_REC2020: [[f64; 3]; 3] = [[0.6274, 0.0691, 0.0164], [0.3293, 0.9195, 0.0880], [0.0433, 0.0113, 0.8956]];
/// `LINEAR_REC2020_TO_LINEAR_SRGB`, as GLSL columns.
pub const REC2020_TO_SRGB: [[f64; 3]; 3] = [[1.6605, -0.1246, -0.0182], [-0.5876, 1.1329, -0.1006], [-0.0728, -0.0083, 1.1187]];
/// `AgXInsetMatrix`, as GLSL columns.
pub const AGX_INSET: [[f64; 3]; 3] = [
    [0.856_627_153_315_983, 0.137_318_972_929_847, 0.111_898_212_999_95],
    [0.095_121_240_538_158_8, 0.761_241_990_602_591, 0.076_799_418_603_190_3],
    [0.048_251_606_145_858_3, 0.101_439_036_467_562, 0.811_302_368_396_859],
];
/// `AgXOutsetMatrix`, as GLSL columns.
pub const AGX_OUTSET: [[f64; 3]; 3] = [
    [1.127_100_581_814_436_8, -0.141_329_763_498_438_3, -0.141_329_763_498_438_26],
    [-0.110_606_643_096_603_23, 1.157_823_702_216_272, -0.110_606_643_096_602_94],
    [-0.016_493_938_717_834_573, -0.016_493_938_717_834_257, 1.251_936_406_595_040_5],
];
/// `AgxMinEv = log2(2^-10 · 0.18)`.
pub const AGX_MIN_EV: f64 = -12.473_93;
/// `AgxMaxEv = log2(2^6.5 · 0.18)`.
pub const AGX_MAX_EV: f64 = 4.026_069;

/// The look applied between the sigmoid and the outset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgxLook {
    /// No look — the base AgX the decision is judged under.
    Base,
    /// Filament's PUNCHY: power 1.35, saturation 1.4 (reported, never decisive).
    Punchy,
}

/// `M · v` for a matrix given as its three COLUMNS (GLSL `mat3` order).
pub fn mul_cols(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    core::array::from_fn(|r| m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2])
}

/// `agxDefaultContrastApprox` — the 6th-order sigmoid fit.
pub fn contrast_approx(x: f64) -> f64 {
    let x2 = x * x;
    let x4 = x2 * x2;
    15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232
}

/// Filament's `agxLook` for [`AgxLook::Punchy`] (identity for [`AgxLook::Base`]).
pub fn agx_look(v: [f64; 3], look: AgxLook) -> [f64; 3] {
    match look {
        AgxLook::Base => v,
        AgxLook::Punchy => {
            let luma = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
            v.map(|c| {
                let cdl = c.max(0.0).powf(1.35);
                luma + 1.4 * (cdl - luma)
            })
        }
    }
}

/// three.js `AgXToneMapping` at exposure 1: linear sRGB in, linear sRGB in `[0, 1]` out.
pub fn agx_f64(c: [f64; 3], look: AgxLook) -> [f64; 3] {
    let v = mul_cols(&AGX_INSET, mul_cols(&SRGB_TO_REC2020, c));
    let v = v.map(|x| ((x.max(1e-10).log2() - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV)).clamp(0.0, 1.0));
    let v = agx_look(v.map(contrast_approx), look);
    let v = mul_cols(&AGX_OUTSET, v).map(|x| x.max(0.0).powf(2.2));
    mul_cols(&REC2020_TO_SRGB, v).map(|x| x.clamp(0.0, 1.0))
}

/// The engine's manual OETF, `pow(x, 1/2.2)`, in `f64`.
pub fn oetf_f64(x: f64) -> f64 {
    x.max(0.0).powf(1.0 / 2.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The matrices map neutral to neutral (each GLSL row sums to 1 within the source's rounding),
    /// so a column/row transposition — the classic transcription slip — is red here.
    #[test]
    fn matrices_preserve_neutral() {
        for (name, m, tol) in [
            ("inset", &AGX_INSET, 1e-9),
            ("outset", &AGX_OUTSET, 1e-9),
            ("srgb_to_2020", &SRGB_TO_REC2020, 2e-4),
            ("2020_to_srgb", &REC2020_TO_SRGB, 2e-4),
        ] {
            let w = mul_cols(m, [1.0, 1.0, 1.0]);
            for c in w {
                assert!((c - 1.0).abs() < tol, "{name}: {w:?}");
            }
        }
    }

    /// Anchors of the curve: black maps near 0, mid-grey to the sigmoid's mid-range, a bright
    /// value toward white, monotone on the grey axis.
    #[test]
    fn grey_axis_is_monotone_and_bounded() {
        let mut prev = -1.0;
        for i in 0..200 {
            let x = 2f64.powf(-14.0 + i as f64 * 0.1);
            let y = agx_f64([x, x, x], AgxLook::Base)[1];
            assert!((0.0..=1.0).contains(&y) && y >= prev, "x = {x}: {y} after {prev}");
            prev = y;
        }
        assert!(agx_f64([0.0; 3], AgxLook::Base)[0] < 1e-3);
        let mid = agx_f64([0.18; 3], AgxLook::Base)[0];
        assert!((0.1..0.3).contains(&mid), "0.18 maps to {mid}");
    }
}
