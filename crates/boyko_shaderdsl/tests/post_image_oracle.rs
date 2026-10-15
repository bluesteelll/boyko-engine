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

mod post_f64;

use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::post::{eval_params, post_final_eval};
use post_f64::{Tracked, oetf_exponent, oetf_tolerance, sanitize_f64, tonemap_f64};

/// The image side.
const W: u32 = 64;
/// The bound's safety factor `k`.
const K: f64 = 4.0;
/// The tonemapper words (bits 8..11): ACES, Neutral, Reinhard-Jodie and an unknown 7 (→ ACES).
const MODES: [u32; 4] = [0, 1, 2, 7];
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
