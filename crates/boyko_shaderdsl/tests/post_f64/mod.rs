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
//!
//! **AgX (render wave PXt, DS-H6).** The VALUE reference is render wave PX0's independent
//! transcription of three.js's `AgXToneMapping` ([`agx_f64`], included by path — one transcription
//! for the `lit` format decision and for the leaf). [`agx_tracked`] bounds an f32 evaluation of the
//! LEAF's operation sequence (one fused input matrix, the log encoding as a multiply, the sigmoid by
//! Horner); it reads `agx_f64`'s four matrices and spells only the sigmoid's coefficients, and the
//! tests hold its value to `agx_f64` within `1e-12`, so a slip in the twin fails there rather than
//! loosening the leaf comparison.

#![allow(dead_code)] // each including test file uses the subset it needs

/// Render wave PX0's `f64` AgX transcription (three.js `AgXToneMapping`), shared with
/// `boyko_rhi_vulkan`'s `lit_format` model: PXt's host test 1 reference (DS-H6).
#[path = "../../../boyko_rhi_vulkan/tests/lit_format/agx_f64.rs"]
pub mod agx_f64;

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
        self.div_checked(o).unwrap_or_else(|| {
            panic!("a divisor within its own error bound of zero ({o:?}) — the reference cannot bound this row")
        })
    }

    /// [`Tracked::div`], or `None` when the divisor is within its own bound of zero (render wave
    /// PX1b: a mis-composed RCAS reference may divide by `4·mn4 − 4 ≈ 0`, where it has no bound).
    pub fn div_checked(self, o: Self) -> Option<Self> {
        let q = self.v / o.v;
        let margin = o.v.abs() - o.b;
        (margin > 0.0).then(|| Self::rounded(q, (self.b + q.abs() * o.b) / margin))
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

    /// `−self` (exact).
    pub fn neg(self) -> Self {
        Tracked { v: -self.v, b: self.b }
    }

    /// `log2(self)`, as the f32 `log2` of an input within `b` of `v` (render wave PXt): the
    /// propagated `b / ((v − b)·ln 2)` — the derivative's largest value over `[v − b, v + b]` — plus
    /// 2 ULP of the result for the f32 function's own error, plus `2⁻¹²⁶`.
    ///
    /// # Panics
    /// Where `v − b ≤ 0`: the input may then be 0, where `log2` has no bound.
    pub fn log2(self) -> Self {
        let lo = self.v - self.b;
        assert!(lo > 0.0, "log2 of {self:?}: the input is within its own bound of 0 — the reference cannot bound this row");
        let r = self.v.log2();
        Tracked { v: r, b: self.b / (lo * core::f64::consts::LN_2) + 2.0 * ulp32(r) + FLOOR }
    }

    /// `pow(self, p)` as the f32 `pow` of an input within `b` of `v`, with the exponent rounded to
    /// f32 (render wave PXt). The value is the exact `v^p`. The bound is the propagated
    /// `p·(v + b)^(p − 1)·b` (the derivative's largest value over the interval, `p ≥ 1`), plus the
    /// exponent's rounding `sup t^p·|ln t| · |f32(p) − p|` over `[max(v − b, 0), v + b]`, plus 2 ULP of
    /// the result for the f32 `pow`, plus `2⁻¹²⁶`.
    ///
    /// `t^p·|ln t|` is taken as its limit, 0, at `t = 0` (`pxt/critique.md` W1): an input clamped to
    /// zero — every dark AgX channel after `max(d, 0)` — keeps a finite bound instead of `∞·0`.
    ///
    /// # Panics
    /// When `v < 0` or `p < 1` (the bound assumes both), or when the bound is not finite.
    pub fn powf(self, p: f64) -> Self {
        assert!(self.v >= 0.0 && p >= 1.0, "powf({self:?}, {p}): the bound needs v ≥ 0 and p ≥ 1");
        let r = self.v.powf(p);
        let (lo, hi) = ((self.v - self.b).max(0.0), self.v + self.b);
        let g = |t: f64| if t > 0.0 { t.powf(p) * t.ln().abs() } else { 0.0 };
        // `t^p·|ln t|` rises to its one maximum on (0, 1) at `e^(−1/p)`, falls to 0 at 1, then
        // rises again: its supremum over [lo, hi] is at an end or at that peak.
        let peak = (-1.0 / p).exp();
        let interior = if (lo..=hi).contains(&peak) { g(peak) } else { 0.0 };
        let g_sup = g(lo).max(g(hi)).max(interior);
        let exponent_rounding = (f64::from(p as f32) - p).abs();
        let b = p * hi.powf(p - 1.0) * self.b + g_sup * exponent_rounding + 2.0 * ulp32(r) + FLOOR;
        assert!(b.is_finite(), "powf({self:?}, {p}): a non-finite bound — the reference cannot bound this row");
        Tracked { v: r, b }
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

/// The OETF exponent the shader spells: `1/2.2` rounded once to f32 — the value DXC folds the
/// shipped `1.0 / 2.2` to (the reference models that constant exactly; its own rounding is not an
/// evaluation error).
pub fn oetf_exponent() -> f64 {
    f64::from((1.0f64 / 2.2) as f32)
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

// ---- Render wave PX1b: FSR 1 RCAS and FSR 2's reversible pair ----------------------------------------
//
// Transcribed from `ffx_fsr1.h` (`FsrRcasF`, the 32-bit path; `FSR_RCAS_DENOISE` undefined, FSR 1's
// default) and `ffx_fsr2_common.h` (`Tonemap` / `InverseTonemap`), with each reciprocal an exact
// division (`px1b/cut.md` D-R1, D-R2) and FSR 1's operand order. The constants are spelled as those
// sources spell them.

/// `FSR_RCAS_LIMIT`.
pub const RCAS_LIMIT: f64 = 0.25 - 1.0 / 16.0;
/// `FSR2_TONEMAP_EPSILON = 1 / FSR2_FP16_MAX`.
pub const RCAS_INV_EPS: f64 = 1.0 / 65504.0;

/// The RCAS cross in FSR 1's tap names: `b` above, `d` left, `e` the centre, `f` right, `h` below.
#[derive(Clone, Copy, Debug)]
pub struct Cross<T> {
    pub b: T,
    pub d: T,
    pub e: T,
    pub f: T,
    pub h: T,
}

impl<T: Copy> Cross<T> {
    /// The same map over the five taps.
    pub fn map<U>(self, g: impl Fn(T) -> U) -> Cross<U> {
        Cross { b: g(self.b), d: g(self.d), e: g(self.e), f: g(self.f), h: g(self.h) }
    }
}

/// FSR 2's `Tonemap`: `c / (max3(c) + 1)`, in the op sequence of the body (`w = 1 / (1 + max3)`,
/// `y = c · w`).
pub fn reversible_f64(c: [Tracked; 3]) -> [Tracked; 3] {
    let one = Tracked::konst(1.0);
    let w = one.div(one.add(c[0].max(c[1].max(c[2]))));
    c.map(|v| v.mul(w))
}

/// FSR 2's `InverseTonemap`: `y / max(EPS, 1 − max3(y))`, as `y · (1 / max(1 − max3(y), EPS))`.
pub fn reversible_inverse_f64(y: [Tracked; 3]) -> [Tracked; 3] {
    let one = Tracked::konst(1.0);
    let floor = one.sub(y[0].max(y[1].max(y[2]))).max(Tracked::konst(RCAS_INV_EPS));
    let w = one.div(floor);
    y.map(|v| v.mul(w))
}

/// `FsrRcasF`'s lobe over a reversible-space cross, times `con`, as [`Tracked`]; `None` when a
/// divisor sits within its own bound of zero (only a cross with a channel at ≈ 1 on all four ring
/// taps — outside reversible space, so only a mis-composed reference reaches it).
///
/// An all-black ring channel (`mx4 = 0`) makes FSR 1's `hitMin = 0·rcp(0)` a NaN that its
/// `max(−hitMin, hitMax)` drops, so that channel's lobe IS `hitMax` — taken here lazily (an eager
/// division by zero would refuse the row), which is the literal FSR 1 result, not the body's guard.
pub fn rcas_lobe_f64_checked(x: Cross<[Tracked; 3]>, con: Tracked) -> Option<Tracked> {
    let one = Tracked::konst(1.0);
    let four = Tracked::konst(4.0);
    let lobe = |c: usize| -> Option<Tracked> {
        let (b, d, e, f, h) = (x.b[c], x.d[c], x.e[c], x.f[c], x.h[c]);
        let mn4 = b.min(d.min(f)).min(h);
        let mx4 = b.max(d.max(f)).max(h);
        let hit_max = one.sub(mx4.max(e)).div_checked(four.mul(mn4).add(Tracked::konst(-4.0)))?;
        if mx4.v == 0.0 {
            Some(hit_max)
        } else {
            Some(mn4.min(e).div_checked(four.mul(mx4))?.neg().max(hit_max))
        }
    };
    let peak = lobe(0)?.max(lobe(1)?.max(lobe(2)?));
    Some(Tracked::konst(-RCAS_LIMIT).max(peak.min(Tracked::konst(0.0))).mul(con))
}

/// [`rcas_lobe_f64_checked`] on a reversible-space cross, where every divisor is bounded.
pub fn rcas_lobe_f64(x: Cross<[Tracked; 3]>, con: Tracked) -> Tracked {
    rcas_lobe_f64_checked(x, con).expect("invariant: a reversible-space cross keeps 4·mn4 − 4 away from zero")
}

/// `FsrRcasF`'s resolve: `(lobe·b + lobe·d + lobe·h + lobe·f + e) / (4·lobe + 1)`, as `… · (1 / (4·lobe
/// + 1))`, per channel (`4·lobe + 1 ≥ 1 − 4·RCAS_LIMIT = 0.25`).
pub fn rcas_resolve_f64(lobe: Tracked, x: Cross<[Tracked; 3]>) -> [Tracked; 3] {
    let one = Tracked::konst(1.0);
    let rcp = one.div(Tracked::konst(4.0).mul(lobe).add(one));
    core::array::from_fn(|c| {
        lobe.mul(x.b[c]).add(lobe.mul(x.d[c])).add(lobe.mul(x.h[c])).add(lobe.mul(x.f[c])).add(x.e[c]).mul(rcp)
    })
}

/// The whole of `FsrRcasF` on a reversible-space cross.
pub fn rcas_f64(x: Cross<[Tracked; 3]>, con: Tracked) -> [Tracked; 3] {
    rcas_resolve_f64(rcas_lobe_f64(x, con), x)
}

/// [`rcas_f64`] on any cross (a mis-composed reference's), `None` where it has no bound: outside
/// reversible space the lobe's bound can exceed `4·lobe + 1` itself, so the resolve divides
/// checked too.
pub fn rcas_f64_checked(x: Cross<[Tracked; 3]>, con: Tracked) -> Option<[Tracked; 3]> {
    let lobe = rcas_lobe_f64_checked(x, con)?;
    let one = Tracked::konst(1.0);
    let rcp = one.div_checked(Tracked::konst(4.0).mul(lobe).add(one))?;
    Some(core::array::from_fn(|c| {
        lobe.mul(x.b[c]).add(lobe.mul(x.d[c])).add(lobe.mul(x.h[c])).add(lobe.mul(x.f[c])).add(x.e[c]).mul(rcp)
    }))
}

/// `FsrRcasF` in plain f64, LITERALLY: `hitMin = min(mn4, e) · rcp(4·mx4)` (a NaN on an all-black
/// ring channel, `0 · ∞`) and `lobe = max(−hitMin, hitMax)` through `f64::max`, which returns the
/// other operand of a NaN — the GLSL.std.450 `NMax` rule FSR 1 relies on. Returns `(lobe, pixel,
/// whether some channel's hitMin was a NaN)`.
pub fn rcas_fsr1_literal(x: Cross<[f64; 3]>, con: f64) -> (f64, [f64; 3], bool) {
    let mut nan_hit_min = false;
    let mut lobes = [0.0f64; 3];
    for (c, lobe) in lobes.iter_mut().enumerate() {
        let (b, d, e, f, h) = (x.b[c], x.d[c], x.e[c], x.f[c], x.h[c]);
        let mn4 = b.min(d.min(f)).min(h);
        let mx4 = b.max(d.max(f)).max(h);
        let hit_min = mn4.min(e) * (1.0 / (4.0 * mx4));
        let hit_max = (1.0 - mx4.max(e)) * (1.0 / (4.0 * mn4 - 4.0));
        nan_hit_min |= hit_min.is_nan();
        *lobe = (-hit_min).max(hit_max);
    }
    let lobe = (-RCAS_LIMIT).max(lobes[0].max(lobes[1].max(lobes[2])).min(0.0)) * con;
    let rcp = 1.0 / (4.0 * lobe + 1.0);
    let pix = core::array::from_fn(|c| (lobe * x.b[c] + lobe * x.d[c] + lobe * x.h[c] + lobe * x.f[c] + x.e[c]) * rcp);
    (lobe, pix, nan_hit_min)
}

/// The documented RCAS stage on five SANITIZED linear taps at exposure `ratio` (`px1b/cut.md` D-R4):
/// each tap times the ratio, into reversible space, `FsrRcasF`, back out — the value the tonemap
/// curve then reads.
pub fn rcas_stage_f64(taps: Cross<[f32; 3]>, ratio: f32, con: f32) -> [Tracked; 3] {
    let r = Tracked::exact(ratio);
    let y = taps.map(|t| reversible_f64(t.map(|v| Tracked::exact(v).mul(r))));
    reversible_inverse_f64(rcas_f64(y, Tracked::exact(con)))
}

// ---- Render wave PXt: AgX in the leaf's operation sequence ------------------------------------------
//
// The matrices are `agx_f64`'s (three.js's GLSL columns, read here as rows); the sigmoid's seven
// coefficients are spelled here, as three.js's `agxDefaultContrastApprox` spells them.

/// The AgX sigmoid's coefficients, highest power first: `15.5x⁶ − 40.14x⁵ + 31.96x⁴ − 6.868x³ +
/// 0.4298x² + 0.1191x − 0.00232`.
pub const AGX_POLY: [f64; 7] = [15.5, -40.14, 31.96, -6.868, 0.4298, 0.1191, -0.00232];

/// The exponent of AgX's linearisation, `max(d, 0)^2.2`.
pub const AGX_LINEARIZE_EXP: f64 = 2.2;

/// The rows of a matrix `agx_f64` spells as its three GLSL columns.
fn rows_of(cols: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    core::array::from_fn(|r| core::array::from_fn(|c| cols[c][r]))
}

/// `a · b` for two row-major matrices, each entry summed over `k = 0, 1, 2` left to right in f64 —
/// the order the leaf's `const` fusion uses, so both round the same f64 entry to f32.
fn mat3_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    core::array::from_fn(|r| core::array::from_fn(|c| a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c]))
}

/// AgX in the LEAF's operation sequence (`pxt/cut.md` D-A1, D-A2, D-T1): `ci = (INSET ·
/// SRGB_TO_REC2020) · c` with the product multiplied out in f64 (each entry a constant rounded once
/// to f32); `x = saturate((log2(max(ci, 1e-10)) − MIN_EV) · (1 / (MAX_EV − MIN_EV)))`; the sigmoid
/// by Horner (a multiply, then an add); `d = OUTSET · p`; `lin = max(d, 0)^2.2`;
/// `saturate(REC2020_TO_SRGB · lin)`.
///
/// # Panics
/// When a value or a bound is not finite: a reference-side red, never a verdict on the leaf.
pub fn agx_tracked(c: [Tracked; 3]) -> [Tracked; 3] {
    agx_tracked_with(c, &AGX_POLY)
}

/// [`agx_tracked`] over the sigmoid coefficients `poly` (the slipped-coefficient fixture's entry).
pub fn agx_tracked_with(c: [Tracked; 3], poly: &[f64; 7]) -> [Tracked; 3] {
    use agx_f64::{AGX_INSET, AGX_MAX_EV, AGX_MIN_EV, AGX_OUTSET, REC2020_TO_SRGB, SRGB_TO_REC2020};
    let fused = mat3_mul(&rows_of(&AGX_INSET), &rows_of(&SRGB_TO_REC2020));
    let floor = Tracked::konst(1e-10);
    let min_ev = Tracked::konst(AGX_MIN_EV);
    let inv_range = Tracked::konst(1.0 / (AGX_MAX_EV - AGX_MIN_EV));
    let p = mat3(&fused, c).map(|ci| {
        let x = ci.max(floor).log2().sub(min_ev).mul(inv_range).saturate();
        poly[1..].iter().fold(Tracked::konst(poly[0]), |acc, &k| acc.mul(x).add(Tracked::konst(k)))
    });
    let lin = mat3(&rows_of(&AGX_OUTSET), p).map(|d| d.max(Tracked::konst(0.0)).powf(AGX_LINEARIZE_EXP));
    let out = mat3(&rows_of(&REC2020_TO_SRGB), lin).map(Tracked::saturate);
    assert!(
        out.iter().all(|t| t.v.is_finite() && t.b.is_finite()),
        "agx_tracked({c:?}) = {out:?}: a non-finite value or bound — a reference-side red, never a verdict on the leaf"
    );
    out
}
