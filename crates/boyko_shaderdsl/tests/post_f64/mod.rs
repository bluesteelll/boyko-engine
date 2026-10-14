//! The INDEPENDENT `f64` transcription of the post tail (render wave PXe) shared by
//! `post_leaves.rs` and `post_image_oracle.rs`, with a running rounding-error bound.
//!
//! **Independence.** Nothing here calls `boyko_shaderdsl::post` or imports one of its constants:
//! every curve is transcribed from its cited source with the constants spelled here — the Hill
//! ACES fit from BakingLab's `ACESFitted` (as `pbr_lighting.hlsli` carries it), Khronos PBR Neutral
//! from the KhronosGroup/ToneMapping README, Reinhard-Jodie and the sRGB pieces from their
//! definitions. A transcription error in `post.rs` therefore disagrees with this file instead of
//! being copied into it.
//!
//! **The running bound (render-wave rule (xvi)).** A [`Tracked`] value carries the exact `f64`
//! result of the documented formula and a bound `b` on how far an f32 evaluation of the SAME
//! operation sequence can sit from it: every f32-rounded operation adds `u·|result|` (`u = 2⁻²⁴`)
//! plus its operands' bounds propagated through the operation's own derivative; every decimal
//! constant enters with its f32 rounding `u·|c|`; `min` / `max` / `clamp` are 1-Lipschitz; a
//! comparison whose operand is within its bound of the threshold may take either arm, so the arm
//! difference is added. Because the bound scales with the conditioning of each step, a correct
//! leaf stays green where a fixed ULP count would be red (`aces_hill` near its numerator root,
//! where the output cancels to ≈ 0).

#![allow(dead_code)] // each including test file uses the subset it needs

/// The f32 unit roundoff `2⁻²⁴`.
pub const U: f64 = 1.0 / 16_777_216.0;
/// `2⁻¹²⁶`, the absolute floor of every tolerance.
pub const FLOOR: f64 = 1.175_494_350_822_287_5e-38;

/// An exact `f64` value with a bound on the error of its f32 evaluation.
#[derive(Clone, Copy, Debug)]
pub struct Tracked {
    /// The exact (f64) value of the documented formula.
    pub v: f64,
    /// The bound on `|f32 evaluation − v|`.
    pub b: f64,
}

impl Tracked {
    /// An exact input (an f32 the evaluation starts from).
    pub fn exact(x: f32) -> Self {
        Tracked { v: f64::from(x), b: 0.0 }
    }

    /// A decimal constant as the source spells it; the f32 evaluation uses its f32 rounding.
    pub fn konst(c: f64) -> Self {
        Tracked { v: c, b: (c - f64::from(c as f32)).abs() }
    }

    fn rounded(v: f64, carried: f64) -> Self {
        Tracked { v, b: carried + U * (v.abs() + carried) }
    }

    /// `self + o`, rounded.
    pub fn add(self, o: Self) -> Self {
        Self::rounded(self.v + o.v, self.b + o.b)
    }

    /// `self − o`, rounded.
    pub fn sub(self, o: Self) -> Self {
        Self::rounded(self.v - o.v, self.b + o.b)
    }

    /// `self · o`, rounded.
    pub fn mul(self, o: Self) -> Self {
        Self::rounded(self.v * o.v, self.v.abs() * o.b + o.v.abs() * self.b + self.b * o.b)
    }

    /// `self / o`, rounded. The divisor must be bounded away from zero by its own bound.
    pub fn div(self, o: Self) -> Self {
        let q = self.v / o.v;
        let margin = o.v.abs() - o.b;
        assert!(margin > 0.0, "a divisor within its own error bound of zero ({o:?}) — the reference cannot bound this row");
        Self::rounded(q, (self.b + q.abs() * o.b) / margin)
    }

    /// `min(self, o)` (exact, 1-Lipschitz).
    pub fn min(self, o: Self) -> Self {
        Tracked { v: self.v.min(o.v), b: self.b.max(o.b) }
    }

    /// `max(self, o)` (exact, 1-Lipschitz).
    pub fn max(self, o: Self) -> Self {
        Tracked { v: self.v.max(o.v), b: self.b.max(o.b) }
    }

    /// `clamp(self, 0, 1)` (exact, 1-Lipschitz).
    pub fn saturate(self) -> Self {
        Tracked { v: self.v.clamp(0.0, 1.0), b: self.b }
    }

    /// `lerp(self, y, s) = self + s·(y − self)` (the HLSL definition), rounded per operation.
    pub fn lerp(self, y: Self, s: Self) -> Self {
        self.add(s.mul(y.sub(self)))
    }

    /// `self < t ? a : b` where the f32 evaluation may take either arm when `self` is within its
    /// bound of `t`: then the result's bound grows by the arms' difference.
    pub fn select_lt(self, t: Self, a: Self, b: Self) -> Self {
        let take_a = self.v < t.v;
        let near = (self.v - t.v).abs() <= self.b + t.b;
        let pick = if take_a { a } else { b };
        if near {
            Tracked { v: pick.v, b: a.b.max(b.b) + (a.v - b.v).abs() }
        } else {
            pick
        }
    }
}

/// `mul(m, v)` with row `i` folded in column order (`m[i][0]·v0 + m[i][1]·v1 + m[i][2]·v2`).
fn mat3(m: &[[f64; 3]; 3], v: [Tracked; 3]) -> [Tracked; 3] {
    core::array::from_fn(|i| {
        Tracked::konst(m[i][0])
            .mul(v[0])
            .add(Tracked::konst(m[i][1]).mul(v[1]))
            .add(Tracked::konst(m[i][2]).mul(v[2]))
    })
}

/// BakingLab `ACESFitted` (Stephen Hill), the matrices row-major as `pbr_lighting.hlsli` spells
/// them: `v = ACES_IN·c; (v(v + 0.0245786) − 0.000090537) / (v(0.983729v + 0.4329510) + 0.238081);
/// saturate(ACES_OUT·…)`.
pub fn aces_f64(c: [Tracked; 3]) -> [Tracked; 3] {
    const ACES_IN: [[f64; 3]; 3] = [
        [0.59719, 0.35458, 0.04823],
        [0.07600, 0.90834, 0.01566],
        [0.02840, 0.13383, 0.83777],
    ];
    const ACES_OUT: [[f64; 3]; 3] = [
        [1.60475, -0.53108, -0.07367],
        [-0.10208, 1.10813, -0.00605],
        [-0.00327, -0.07276, 1.07602],
    ];
    let v = mat3(&ACES_IN, c);
    let q = v.map(|v| {
        let a = v.mul(v.add(Tracked::konst(0.0245786))).sub(Tracked::konst(0.000090537));
        let b = v
            .mul(Tracked::konst(0.983729).mul(v).add(Tracked::konst(0.4329510)))
            .add(Tracked::konst(0.238081));
        a.div(b)
    });
    mat3(&ACES_OUT, q).map(Tracked::saturate)
}

/// Khronos PBR Neutral, transcribed from the KhronosGroup/ToneMapping README (F90 = 0.04,
/// `startCompression = 0.8 − 0.04`, `desaturation = 0.15`).
pub fn neutral_f64(c: [Tracked; 3]) -> [Tracked; 3] {
    let start = Tracked::konst(0.8).sub(Tracked::konst(0.04));
    let desaturation = Tracked::konst(0.15);
    let one = Tracked::konst(1.0);
    let x = c[0].min(c[1].min(c[2]));
    let toe = x.sub(Tracked::konst(6.25).mul(x).mul(x));
    let offset = x.select_lt(Tracked::konst(0.08), toe, Tracked::konst(0.04));
    let col = c.map(|v| v.sub(offset));
    let peak = col[0].max(col[1].max(col[2]));
    // `if (peak < startCompression) return color;` — certain only when `peak` is farther below the
    // threshold than both bounds; then the compressed arm (which divides by `peak`) is never read.
    if peak.v < start.v && start.v - peak.v > peak.b + start.b {
        return col;
    }
    let d = one.sub(start);
    let new_peak = one.sub(d.mul(d).div(peak.add(d).sub(start)));
    let g = one.sub(one.div(desaturation.mul(peak.sub(new_peak)).add(one)));
    col.map(|v| {
        let compressed = v.mul(new_peak.div(peak)).lerp(new_peak, g);
        peak.select_lt(start, v, compressed)
    })
}

/// Reinhard-Jodie: `l = dot(v, (0.2126, 0.7152, 0.0722)); tv = v / (1 + v);
/// saturate(lerp(v / (1 + l), tv, tv))`.
pub fn reinhard_jodie_f64(c: [Tracked; 3]) -> [Tracked; 3] {
    let one = Tracked::konst(1.0);
    let l = c[0]
        .mul(Tracked::konst(0.2126))
        .add(c[1].mul(Tracked::konst(0.7152)))
        .add(c[2].mul(Tracked::konst(0.0722)));
    c.map(|v| {
        let tv = v.div(one.add(v));
        v.div(one.add(l)).lerp(tv, tv).saturate()
    })
}

/// The tonemapper the word's bits 8..11 select: 1 Neutral, 2 Reinhard-Jodie, everything else ACES
/// (the resolve's documented fallback).
pub fn tonemap_f64(mode_words: u32, c: [Tracked; 3]) -> [Tracked; 3] {
    match (mode_words >> 8) & 0xF {
        1 => neutral_f64(c),
        2 => reinhard_jodie_f64(c),
        _ => aces_f64(c),
    }
}

/// The OETF exponent the shader spells: `1/2.2` rounded to f32 (the reference models that constant
/// exactly; its own rounding is not an evaluation error).
pub fn oetf_exponent() -> f64 {
    f64::from(1.0f32 / 2.2f32)
}

/// The f32 ULP at `|v|`.
pub fn ulp32(v: f64) -> f64 {
    let a = (v.abs() as f32).max(f32::MIN_POSITIVE);
    f64::from(f32::from_bits(a.to_bits() + 1) - a)
}

/// The display-domain tolerance of `pow(t, p)` for an f32 evaluation whose input is within `b` of
/// the exact `t ∈ [0, 1]`: the OETF's Hölder modulus `(k·b)^p` (`|x^p − y^p| ≤ |x − y|^p`, `p ≤ 1`,
/// finite wherever `t` cancels to ≈ 0), tightened by the mean value bound `p·lo^(p−1)·k·b` where
/// the interval stays away from 0, plus 2 ULP of `pow`'s own rounding and the `2⁻¹²⁶` floor.
pub fn oetf_tolerance(t: f64, b: f64, k: f64, d: f64) -> f64 {
    let p = oetf_exponent();
    let kb = k * b;
    let holder = kb.powf(p);
    let lo = t - kb;
    let mvt = if lo > 0.0 { p * lo.powf(p - 1.0) * kb } else { f64::INFINITY };
    holder.min(mvt) + 2.0 * ulp32(d) + FLOOR
}

/// The f64 sRGB OETF (the encode `*_SRGB` hardware applies).
pub fn srgb_oetf_f64(l: f64) -> f64 {
    if l <= 0.003_130_8 { 12.92 * l } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
}

/// The f64 sRGB EOTF (the decode).
pub fn srgb_eotf_f64(e: f64) -> f64 {
    if e <= 0.04045 { e / 12.92 } else { ((e + 0.055) / 1.055).powf(2.4) }
}

/// The whole-pixel sanitize of the documented composition: any non-finite channel ⇒ all zero, else
/// each channel clamped to `[0, 64512]` (exact on f32 inputs).
pub fn sanitize_f64(c: [f32; 3]) -> [f32; 3] {
    if c.iter().any(|v| !v.is_finite()) { [0.0; 3] } else { c.map(|v| v.clamp(0.0, 64512.0)) }
}
