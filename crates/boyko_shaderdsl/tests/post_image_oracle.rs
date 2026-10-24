//! Render wave PXe — the IMAGE-LEVEL `f64` oracle of the traced `post_final_main` (OPEN-ITEMS
//! item 4, part b; `e0-pxe/cut.md` D-P2).
//!
//! `post_final_main::<EvalCf>` runs over whole synthetic `HostTex2` images — its real fetch, its
//! real stage composition, its real mode dispatch — and every pixel, every channel, is compared in
//! the display domain with an INDEPENDENT `f64` transcription of the documented composition
//! (`tests/post_f64/mod.rs`, which calls no `post.rs` leaf): sanitize the whole pixel → multiply by
//! the exposure ratio → the curve the mode word selects → the gamma-2.2 OETF.
//!
//! **Tolerance (rule (xvi)).** The reference carries a running bound `B` on the pre-OETF value
//! (every rounded op adds `u·|result|` plus its operands' propagated bounds); the display gate is
//! the OETF's modulus of `k·B` (Hölder `(k·B)^(1/2.2)`, tightened by the mean-value bound away from
//! 0) plus 2 ULP of `pow` and `2⁻¹²⁶`, with `k = 4`. A fixed display tolerance would be red on a
//! correct body wherever `ACES_OUT` cancels a channel to ≈ 0 and the OETF's infinite slope at 0
//! amplifies the residual. The run prints the worst `|Δ|/τ` and the median `τ` in 8-bit LSB.
//!
//! **Discrimination is proven here, every run.** Four mis-composed `f64` references — the OETF
//! before the curve; the curve before the ratio; the ratio skipped; a per-CHANNEL sanitize — must
//! each REJECT the correct composition on ≥ 30 % of the pixels of their target class (finite pixels
//! for the first; ratio ≠ 1 for the two ratio mutants; NaN-channel pixels for the sanitize one), by
//! the same comparator. RCAS (PX1b), the bloom composite (PX4) and the LUT (PX5) extend this file
//! with their own f64 stage and their own mis-composition self-tests.
//!
//! **RCAS (render wave PX1b, OPEN-ITEMS item 4's H5 part, `px1b/cut.md` §1.3).** Under the sharpen
//! bit the body reads a five-tap cross through its border clamp, so this oracle owns the fetching
//! stage too: an independent f64 FSR 1 RCAS (`post_f64::rcas_f64` — the cross, clamp-to-edge at the
//! border, FSR 2's reversible pair, the lobe clamp, `con`) composed with the f64 chain above, over a
//! synthetic image with a 1-px border ring of distinct values, H5's step edges in
//! reversible-equivalent radiance, a 3-dark/1-bright cross, an HDR bar, saturated HDR blue, a
//! NaN-neighbour ring and textured noise. Six mis-composed references — a diagonal tap, the inverse
//! skipped, RCAS after the curve, no reversible wrap, a wrapped border, the luma wrap — must each be
//! rejected on ≥ 30 % of their target class (an EDGE class wherever the mis-composition is the
//! identity on a flat cross, critique O3) by the same comparator.

mod post_f64;

use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::post::{eval_params, post_final_eval};
use post_f64::{
    Cross, RCAS_INV_EPS, Tracked, oetf_exponent, oetf_tolerance, rcas_f64_checked, rcas_stage_f64,
    reversible_f64, sanitize_f64, tonemap_f64,
};

/// The image side.
const W: u32 = 64;
/// The bound's safety factor `k`.
const K: f64 = 4.0;
/// The tonemapper words (bits 8..11): ACES, Neutral, Reinhard-Jodie, AgX (render wave PXt) and an
/// unknown 7 (→ ACES).
const MODES: [u32; 5] = [0, 1, 2, 3, 7];
/// The exposure ratios: the identity and one each side.
const RATIOS: [f32; 3] = [1.0, 0.5, 2.0];

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// `2^e`, `e` uniform in `[lo, hi)`.
    fn log2_range(&mut self, lo: f64, hi: f64) -> f32 {
        (lo + (hi - lo) * self.unit()).exp2() as f32
    }
}

/// The pixel classes of the synthetic image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    /// Finite, non-negative, below `HDR_MAX`.
    Finite,
    /// One channel NaN, the others finite (the ring).
    NanChannel,
    /// One channel `±Inf`.
    Inf,
    /// A negative channel (a Catmull-Rom history overshoot).
    Negative,
    /// A channel above `HDR_MAX`.
    AboveMax,
}

/// A 64×64 image: a NaN-channel ring, `±Inf`, negative and above-`HDR_MAX` pixels on fixed
/// diagonals, and finite HDR values over `2⁻²⁰ … 2¹⁶` elsewhere — 70 % in the curves' discriminating
/// mid-range `[2⁻⁶, 2⁴)`, 30 % over the whole range.
fn image() -> (Vec<[f32; 4]>, Vec<Class>) {
    let mut rng = Rng(0x9E_0F1A);
    let mut texels = Vec::with_capacity((W * W) as usize);
    let mut classes = Vec::with_capacity((W * W) as usize);
    let c = f64::from(W) / 2.0 - 0.5;
    for y in 0..W {
        for x in 0..W {
            let mut rgb: [f32; 3] = core::array::from_fn(|_| {
                if rng.unit() < 0.7 { rng.log2_range(-6.0, 4.0) } else { rng.log2_range(-20.0, 16.0) }
            });
            let ch = (rng.next_u64() % 3) as usize;
            let r = ((f64::from(x) - c).powi(2) + (f64::from(y) - c).powi(2)).sqrt();
            let class = if (r - 20.0).abs() < 0.75 {
                rgb[ch] = f32::NAN;
                Class::NanChannel
            } else if (x + 2 * y) % 41 == 0 {
                rgb[ch] = if x % 2 == 0 { f32::INFINITY } else { f32::NEG_INFINITY };
                Class::Inf
            } else if (3 * x + y) % 37 == 0 {
                rgb[ch] = -rng.log2_range(-12.0, 2.0);
                Class::Negative
            } else if (x + 5 * y) % 43 == 0 {
                rgb[ch] = rng.log2_range(16.0, 100.0);
                Class::AboveMax
            } else {
                Class::Finite
            };
            texels.push([rgb[0], rgb[1], rgb[2], 1.0]);
            classes.push(class);
        }
    }
    (texels, classes)
}

/// How a reference composes the stages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Composition {
    /// The documented order: sanitize → ratio → curve → OETF.
    Documented,
    /// Mis-composed: the OETF before the curve.
    OetfBeforeCurve,
    /// Mis-composed: the curve before the ratio.
    CurveBeforeRatio,
    /// Mis-composed: the ratio skipped.
    RatioSkipped,
    /// Mis-composed: the sanitize per channel, not per pixel.
    PerChannelSanitize,
}

/// One channel of a reference: the display value and its display-domain tolerance.
#[derive(Clone, Copy, Debug)]
struct RefChannel {
    d: f64,
    tau: f64,
}

/// The f64 reference of one pixel under `comp`.
fn reference(texel: [f32; 4], mode_words: u32, ratio: f32, comp: Composition) -> [RefChannel; 3] {
    let raw = [texel[0], texel[1], texel[2]];
    let h = if comp == Composition::PerChannelSanitize {
        raw.map(|v| if v.is_finite() { v.clamp(0.0, 64512.0) } else { 0.0 })
    } else {
        sanitize_f64(raw)
    };
    let h = h.map(Tracked::exact);
    let r = Tracked::exact(ratio);
    let p = oetf_exponent();
    let t: [Tracked; 3] = match comp {
        Composition::Documented | Composition::PerChannelSanitize => {
            tonemap_f64(mode_words, h.map(|v| v.mul(r)))
        }
        Composition::OetfBeforeCurve => {
            // The curve over the OETF-encoded ratio-scaled value; no OETF after it.
            let enc = h.map(|v| {
                let x = v.mul(r);
                Tracked { v: x.v.powf(p), b: oetf_tolerance(x.v, x.b, 1.0, x.v.powf(p)) }
            });
            let curved = tonemap_f64(mode_words, enc);
            return curved.map(|t| RefChannel { d: t.v, tau: K * t.b + 2.0 * post_f64::ulp32(t.v) + post_f64::FLOOR });
        }
        Composition::CurveBeforeRatio => tonemap_f64(mode_words, h).map(|v| v.mul(r)),
        Composition::RatioSkipped => tonemap_f64(mode_words, h),
    };
    t.map(|t| {
        let tv = t.v.max(0.0);
        let d = tv.powf(p);
        RefChannel { d, tau: oetf_tolerance(tv, t.b, K, d) }
    })
}

/// The Eval body's display value for every pixel of `texels` under `(mode_words, ratio)`.
fn body_image(texels: &[[f32; 4]], mode_words: u32, ratio: f32) -> Vec<[f32; 4]> {
    let mut words = [0u32; 8];
    words[0] = mode_words;
    words[1] = ratio.to_bits();
    words[2] = W;
    words[3] = W;
    let p = eval_params(words);
    let scene = HostTex2::new(texels, W, W);
    (0..W * W).map(|i| post_final_eval([i % W, i / W], &p, scene)).collect()
}

#[test]
fn post_final_main_matches_f64_image_reference() {
    let (texels, _) = image();
    let mut worst = 0.0f64;
    let mut taus: Vec<f64> = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    let mut rejected = 0usize;
    let mut total = 0usize;
    for mode in MODES {
        let mode_words = mode << 8;
        for ratio in RATIOS {
            let got = body_image(&texels, mode_words, ratio);
            for (i, (texel, out)) in texels.iter().zip(&got).enumerate() {
                let reference = reference(*texel, mode_words, ratio, Composition::Documented);
                assert_eq!(out[3], 1.0, "pixel {i}: the body must return alpha 1");
                for (ch, r) in reference.iter().enumerate() {
                    total += 1;
                    let err = (f64::from(out[ch]) - r.d).abs();
                    worst = worst.max(err / r.tau);
                    taus.push(r.tau);
                    let within = err <= r.tau;
                    if !within {
                        rejected += 1;
                        if bad.len() < 8 {
                            bad.push(format!(
                                "mode {mode} ratio {ratio} pixel ({}, {}) ch {ch}: texel {:?} -> body {:e}, f64 {:e}, |err| {err:e} > tau {:e}",
                                i as u32 % W,
                                i as u32 / W,
                                texel,
                                out[ch],
                                r.d,
                                r.tau
                            ));
                        }
                    }
                }
            }
        }
    }
    taus.sort_by(f64::total_cmp);
    let median_lsb = taus[taus.len() / 2] * 255.0;
    eprintln!(
        "post image oracle: {total} channel checks, worst |err|/tau = {worst:.4}, median tau = {median_lsb:.4} LSB, {rejected} rejected"
    );
    assert!(bad.is_empty(), "post_final_main::<EvalCf> disagrees with the f64 image reference on {rejected}/{total} channels:\n{}", bad.join("\n"));
}

#[test]
fn image_oracle_rejects_miscomposed_references() {
    let (texels, classes) = image();
    let mutants = [
        (Composition::OetfBeforeCurve, "OETF before the curve"),
        (Composition::CurveBeforeRatio, "curve before the ratio"),
        (Composition::RatioSkipped, "ratio skipped"),
        (Composition::PerChannelSanitize, "per-channel sanitize"),
    ];
    for (comp, what) in mutants {
        let mut target = 0usize;
        let mut rejected = 0usize;
        for mode in MODES {
            let mode_words = mode << 8;
            for ratio in RATIOS {
                let in_class_ratio = match comp {
                    Composition::CurveBeforeRatio | Composition::RatioSkipped => ratio != 1.0,
                    _ => true,
                };
                if !in_class_ratio {
                    continue;
                }
                for (texel, class) in texels.iter().zip(&classes) {
                    let in_class = match comp {
                        Composition::PerChannelSanitize => *class == Class::NanChannel,
                        _ => *class == Class::Finite,
                    };
                    if !in_class {
                        continue;
                    }
                    // The candidate is the documented composition itself (its f64 value rounded to
                    // f32 — what a correct body produces up to its tolerance): the comparator is
                    // under test here, not the body.
                    let good = reference(*texel, mode_words, ratio, Composition::Documented);
                    let wrong = reference(*texel, mode_words, ratio, comp);
                    target += 1;
                    let reject = good.iter().zip(&wrong).any(|(g, w)| {
                        let within = (f64::from(g.d as f32) - w.d).abs() <= w.tau;
                        !within
                    });
                    if reject {
                        rejected += 1;
                    }
                }
            }
        }
        let share = rejected as f64 / target as f64;
        eprintln!("mutant `{what}`: rejected {rejected}/{target} target pixels ({:.1} %)", 100.0 * share);
        assert!(target >= 100, "mutant `{what}`: only {target} target pixels — the class is vacuous");
        assert!(share >= 0.30, "the comparator accepts the documented composition as `{what}` on {:.1} % of its target pixels — tau is too loose to see a mis-composition", 100.0 * (1.0 - share));
    }
}

// ---- Render wave PX1b: the RCAS stage ------------------------------------------------------------

/// Mode-word bit 1, the sharpen bit (H5; spelled here, not imported, like the word indices above).
const SHARPEN_BIT: u32 = 1 << 1;
/// The `sharpness` word's index (byte offset 16 of `PostFinal`).
const SHARPNESS_WORD: usize = 4;
/// The sharpened words' exposure ratios.
const RCAS_RATIOS: [f32; 2] = [1.0, 2.0];
/// RCAS's `con` values: FSR 1's maximum, one stop, 1.5 stops (`rcas_sharpness = 0.25`), the mildest. The
/// default `rcas_sharpness = 0.78` (`con ≈ 0.737`) lies between the first two.
const RCAS_CONS: [f32; 4] = [1.0, 0.5, 0.354, 0.25];

/// The RCAS fixture: a 64×64 image whose ring (x or y at 0 or 63) carries distinct colours, and whose
/// interior is, by bands of rows: H5's `0.3 | 0.6` step in reversible-equivalent radiance (`0.4286
/// | 1.5`, `y = c / (1 + c)`), the `0.1 | 0.9` step (`0.111 | 9.0`), isolated bright pixels on dark
/// (each neighbour's cross is 3-dark/1-bright), an HDR bar (4.0) on a dark floor, saturated HDR
/// blue `(0, 0, 10)` stripes, one-channel-NaN pixels on a grey (their neighbours read a sanitized
/// black tap), and textured noise.
fn rcas_image() -> Vec<[f32; 4]> {
    let mut rng = Rng(0x1B_5CA5);
    let last = W - 1;
    let mut texels = Vec::with_capacity((W * W) as usize);
    for y in 0..W {
        for x in 0..W {
            let ring = x == 0 || y == 0 || x == last || y == last;
            let rgb: [f32; 3] = if ring {
                // The position along the ring, so no two ring texels a wrap could swap are equal.
                let k = if y == 0 {
                    x
                } else if x == last {
                    W + y
                } else if y == last {
                    2 * W + (last - x)
                } else {
                    3 * W + (last - y)
                };
                [0.02 + 0.031 * k as f32, 0.05 + 0.017 * k as f32, 0.3 + 0.009 * k as f32]
            } else if y < 12 {
                [if x < 32 { 0.428_571_43 } else { 1.5 }; 3]
            } else if y < 24 {
                [if x < 32 { 0.111_111_11 } else { 9.0 }; 3]
            } else if y < 32 {
                [if x % 4 == 2 && y % 4 == 2 { 9.0 } else { 0.111_111_11 }; 3]
            } else if y < 40 {
                [if y == 35 || y == 36 { 4.0 } else { 0.05 }; 3]
            } else if y < 48 {
                if (x / 3) % 2 == 0 { [0.0, 0.0, 10.0] } else { [0.05; 3] }
            } else if y < 54 {
                if y == 50 && x % 5 == 2 { [f32::NAN, 0.5, 0.5] } else { [0.5; 3] }
            } else {
                core::array::from_fn(|_| rng.log2_range(-6.0, 4.0))
            };
            texels.push([rgb[0], rgb[1], rgb[2], 1.0]);
        }
    }
    texels
}

/// The sanitized texel at `(x, y)` (`x`, `y` already inside the image).
fn sanitized(texels: &[[f32; 4]], x: u32, y: u32) -> [f32; 3] {
    let t = texels[(y * W + x) as usize];
    sanitize_f64([t[0], t[1], t[2]])
}

/// The documented cross at `(x, y)`: FSR 1's taps, clamped to the image's edge.
fn clamped_cross(x: u32, y: u32) -> Cross<(u32, u32)> {
    let last = W - 1;
    Cross {
        b: (x, y.saturating_sub(1)),
        d: (x.saturating_sub(1), y),
        e: (x, y),
        f: ((x + 1).min(last), y),
        h: (x, (y + 1).min(last)),
    }
}

/// How an RCAS reference composes the stage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RcasComp {
    /// The documented stage (`px1b/cut.md` D-R1..R6).
    Documented,
    /// (m1) the right tap read at the diagonal `(x + 1, y + 1)`.
    DiagonalTap,
    /// (m2) RCAS's reversible-space output fed to the curve without the inverse.
    InverseSkipped,
    /// (m3) RCAS after the curve (on display-referred `[0, 1]` values, no wrap).
    AfterCurve,
    /// (m4) RCAS on the raw HDR taps (no reversible wrap, no inverse).
    NoReversibleWrap,
    /// (m5) the border taps wrapped around the image instead of clamped to its edge.
    WrappedBorder,
    /// (m6) the luma wrap `c / (1 + luma(c))` and its inverse instead of FSR 2's `max3`.
    LumaWrap,
}

/// The taps a composition reads at `(x, y)`.
fn taps_of(x: u32, y: u32, comp: RcasComp) -> Cross<(u32, u32)> {
    let last = W - 1;
    let c = clamped_cross(x, y);
    match comp {
        RcasComp::DiagonalTap => Cross { f: ((x + 1).min(last), (y + 1).min(last)), ..c },
        RcasComp::WrappedBorder => Cross {
            b: (x, (y + last) % W),
            d: ((x + last) % W, y),
            f: ((x + 1) % W, y),
            h: (x, (y + 1) % W),
            ..c
        },
        _ => c,
    }
}

/// Rec.709 luma (the weights the luma-wrap mutant uses).
fn luma(c: [Tracked; 3]) -> Tracked {
    c[0].mul(Tracked::konst(0.2126)).add(c[1].mul(Tracked::konst(0.7152))).add(c[2].mul(Tracked::konst(0.0722)))
}

/// `s` when its running bound still discriminates (every channel's bound at most a quarter of its
/// magnitude, plus `10⁻³`), else `None` — a mis-composed RCAS outside reversible space can divide
/// by `4·mn4 − 4 ≈ 0`, and a reference that lost its precision there would be accepted by any
/// comparator rather than tested (and its curve could divide within its own bound of zero).
fn bounded(s: [Tracked; 3]) -> Option<[Tracked; 3]> {
    s.iter().all(|t| t.b <= 0.25 * t.v.abs() + 1e-3).then_some(s)
}

/// The f64 reference of one sharpened pixel under `comp`; `None` where a mis-composed reference has
/// no bound (a divisor within its own bound of zero — never on the documented composition).
fn rcas_reference(texels: &[[f32; 4]], x: u32, y: u32, mode_words: u32, ratio: f32, con: f32, comp: RcasComp) -> Option<[RefChannel; 3]> {
    let taps = taps_of(x, y, comp).map(|(tx, ty)| sanitized(texels, tx, ty));
    let r = Tracked::exact(ratio);
    let k = Tracked::exact(con);
    let lin = taps.map(|t| t.map(|v| Tracked::exact(v).mul(r)));
    let p = oetf_exponent();
    let curved: [Tracked; 3] = match comp {
        RcasComp::Documented | RcasComp::DiagonalTap | RcasComp::WrappedBorder => {
            tonemap_f64(mode_words, rcas_stage_f64(taps, ratio, con))
        }
        RcasComp::InverseSkipped => tonemap_f64(mode_words, bounded(rcas_f64_checked(lin.map(reversible_f64), k)?)?),
        RcasComp::AfterCurve => rcas_f64_checked(lin.map(|t| tonemap_f64(mode_words, t)), k)?,
        RcasComp::NoReversibleWrap => tonemap_f64(mode_words, bounded(rcas_f64_checked(lin, k)?)?),
        RcasComp::LumaWrap => {
            let one = Tracked::konst(1.0);
            let fwd = lin.map(|c| {
                let w = one.div(one.add(luma(c)));
                c.map(|v| v.mul(w))
            });
            let s = bounded(rcas_f64_checked(fwd, k)?)?;
            let w = one.div_checked(one.sub(luma(s)).max(Tracked::konst(RCAS_INV_EPS)))?;
            tonemap_f64(mode_words, bounded(s.map(|v| v.mul(w)))?)
        }
    };
    Some(curved.map(|t| {
        let tv = t.v.max(0.0);
        let d = tv.powf(p);
        RefChannel { d, tau: oetf_tolerance(tv, t.b, K, d) }
    }))
}

/// The eight words of a sharpened frame: the tonemapper, the sharpen bit, the ratio, the extent and
/// `con` in the sharpness word.
fn rcas_words(mode_words: u32, ratio: f32, con: f32) -> [u32; 8] {
    let mut words = [0u32; 8];
    words[0] = mode_words | SHARPEN_BIT;
    words[1] = ratio.to_bits();
    words[2] = W;
    words[3] = W;
    words[SHARPNESS_WORD] = con.to_bits();
    words
}

/// Whether the five sanitized taps of the documented cross at `(x, y)` are not all equal.
fn is_edge(texels: &[[f32; 4]], x: u32, y: u32) -> bool {
    let t = clamped_cross(x, y).map(|(tx, ty)| sanitized(texels, tx, ty));
    [t.b, t.d, t.f, t.h].iter().any(|v| v.map(f32::to_bits) != t.e.map(f32::to_bits))
}

#[test]
fn post_final_main_matches_f64_image_reference_with_rcas() {
    let texels = rcas_image();
    let scene = HostTex2::new(&texels, W, W);
    let mut worst = 0.0f64;
    let mut bad: Vec<String> = Vec::new();
    let (mut total, mut rejected_edge, mut rejected_flat) = (0usize, 0usize, 0usize);
    for mode in MODES {
        let mode_words = mode << 8;
        for ratio in RCAS_RATIOS {
            for con in RCAS_CONS {
                let p = eval_params(rcas_words(mode_words, ratio, con));
                for y in 0..W {
                    for x in 0..W {
                        let out = post_final_eval([x, y], &p, scene);
                        let reference = rcas_reference(&texels, x, y, mode_words, ratio, con, RcasComp::Documented)
                            .expect("invariant: the documented composition is bounded on every pixel");
                        assert_eq!(out[3], 1.0, "pixel ({x}, {y}): the body must return alpha 1");
                        let mut pixel_bad = false;
                        for (ch, r) in reference.iter().enumerate() {
                            total += 1;
                            let err = (f64::from(out[ch]) - r.d).abs();
                            worst = worst.max(err / r.tau);
                            let within = err <= r.tau;
                            if !within {
                                pixel_bad = true;
                                if bad.len() < 8 {
                                    bad.push(format!(
                                        "mode {mode} ratio {ratio} con {con} pixel ({x}, {y}) ch {ch}: body {:e}, f64 {:e}, |err| {err:e} > tau {:e}",
                                        out[ch], r.d, r.tau
                                    ));
                                }
                            }
                        }
                        if pixel_bad {
                            if is_edge(&texels, x, y) {
                                rejected_edge += 1;
                            } else {
                                rejected_flat += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "post image oracle (RCAS): {total} channel checks, worst |err|/tau = {worst:.4}, rejected pixels: {rejected_edge} edge, {rejected_flat} flat"
    );
    assert!(
        bad.is_empty(),
        "post_final_main::<EvalCf> under the sharpen bit disagrees with the f64 RCAS image reference ({rejected_edge} edge pixels, {rejected_flat} flat pixels):\n{}",
        bad.join("\n")
    );
}

/// Whether `(x, y)` is in a mis-composition's target class: the pixels where the mis-composition
/// changes RCAS's input or its space — at an EDGE wherever the mis-composition is the identity on a
/// flat cross (critique O3: every reversible wrap round-trips and RCAS is the identity there).
fn in_target(texels: &[[f32; 4]], x: u32, y: u32, ratio: f32, comp: RcasComp) -> bool {
    let tap = |(tx, ty): (u32, u32)| sanitized(texels, tx, ty);
    let max3 = |c: [f32; 3]| c[0].max(c[1].max(c[2]));
    let edge = is_edge(texels, x, y);
    let doc = clamped_cross(x, y);
    match comp {
        RcasComp::Documented => true,
        RcasComp::DiagonalTap => tap(taps_of(x, y, comp).f).map(f32::to_bits) != tap(doc.f).map(f32::to_bits),
        RcasComp::InverseSkipped => max3(tap(doc.e)) > 0.0,
        RcasComp::AfterCurve => edge,
        RcasComp::NoReversibleWrap => edge && [doc.b, doc.d, doc.e, doc.f, doc.h].iter().any(|&t| max3(tap(t)) * ratio > 1.0),
        RcasComp::WrappedBorder => {
            let mis = taps_of(x, y, comp);
            [(mis.b, doc.b), (mis.d, doc.d), (mis.f, doc.f), (mis.h, doc.h)]
                .iter()
                .any(|&(m, d)| tap(m).map(f32::to_bits) != tap(d).map(f32::to_bits))
        }
        RcasComp::LumaWrap => {
            let c = tap(doc.e);
            let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            edge && max3(c) * ratio > 1.0 && max3(c) >= 3.0 * l
        }
    }
}

#[test]
fn image_oracle_rejects_miscomposed_rcas_references() {
    let texels = rcas_image();
    let mutants = [
        (RcasComp::DiagonalTap, "(m1) a diagonal tap (x + 1, y + 1)"),
        (RcasComp::InverseSkipped, "(m2) the inverse skipped"),
        (RcasComp::AfterCurve, "(m3) RCAS after the curve"),
        (RcasComp::NoReversibleWrap, "(m4) no reversible wrap (raw HDR)"),
        (RcasComp::WrappedBorder, "(m5) the border wrapped, not clamped"),
        (RcasComp::LumaWrap, "(m6) the luma wrap c / (1 + luma)"),
    ];
    for (comp, what) in mutants {
        let (mut target, mut rejected, mut unbounded) = (0usize, 0usize, 0usize);
        for mode in MODES {
            let mode_words = mode << 8;
            for ratio in RCAS_RATIOS {
                for con in RCAS_CONS {
                    for y in 0..W {
                        for x in 0..W {
                            if !in_target(&texels, x, y, ratio, comp) {
                                continue;
                            }
                            // The candidate is the documented composition itself (its f64 value
                            // rounded to f32): the comparator is under test here, not the body.
                            let good = rcas_reference(&texels, x, y, mode_words, ratio, con, RcasComp::Documented)
                                .expect("invariant: the documented composition is bounded on every pixel");
                            let Some(wrong) = rcas_reference(&texels, x, y, mode_words, ratio, con, comp) else {
                                unbounded += 1;
                                continue;
                            };
                            target += 1;
                            let reject = good.iter().zip(&wrong).any(|(g, w)| (f64::from(g.d as f32) - w.d).abs() > w.tau);
                            if reject {
                                rejected += 1;
                            }
                        }
                    }
                }
            }
        }
        let share = rejected as f64 / target as f64;
        eprintln!("mutant `{what}`: rejected {rejected}/{target} target pixels ({:.1} %), {unbounded} unbounded", 100.0 * share);
        assert!(target >= 100, "mutant `{what}`: only {target} target pixels — the class is vacuous");
        assert!(share >= 0.30, "the comparator accepts the documented RCAS composition as `{what}` on {:.1} % of its target pixels", 100.0 * (1.0 - share));
    }
}
