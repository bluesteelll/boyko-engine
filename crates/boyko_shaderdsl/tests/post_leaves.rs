//! Render wave PXe — the post leaves' `EvalCf` oracles (`plan/H2-PXe-post-edsl.md` tests 1–4, the
//! UBO reader).
//!
//! Every curve is compared with the INDEPENDENT `f64` transcription of its cited source
//! (`tests/post_f64/mod.rs` — it imports no `post.rs` constant) under the conditioning-scaled
//! running bound of render-wave rule (xvi) plus `2⁻¹²⁶` — not a fixed ULP count, which is red on a
//! correct `aces_hill` near its numerator root (`v ≈ 3.25·10⁻³`, where the output cancels to ≈ 0).
//! A transcription error in a matrix entry or a rational coefficient moves the leaf by far more
//! than that bound (≈ `10⁻⁶` relative); the run prints each leaf's worst `|err| / tol`.

mod post_f64;

use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::post::{
    aces_hill, eval_params, hdr_sanitize, khronos_pbr_neutral, oetf_gamma22, pre_ratio_apply,
    reinhard_jodie, srgb_eotf, tonemap_select,
};
use post_f64::{FLOOR, Tracked, aces_f64, neutral_f64, reinhard_jodie_f64, srgb_oetf_f64, ulp32};

/// The bound's safety factor (the image oracle's `k`).
const K: f64 = 4.0;

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

/// The curve-leaf domain: post-sanitize HDR colours — channels log-uniform over `2⁻²⁰ … 64512`,
/// exact zeros, greys, and a sweep through ACES's numerator root (`v ≈ 3.25·10⁻³`) and Neutral's
/// `peak = 0.76` and `x = 0.08` thresholds.
fn curve_inputs() -> Vec<[f32; 3]> {
    let mut rng = Rng(0x7E_AF00);
    let mut v: Vec<[f32; 3]> = (0..20_000)
        .map(|_| core::array::from_fn(|_| rng.log2_range(-20.0, 15.977)))
        .collect();
    v.push([0.0; 3]);
    for k in 0..2000 {
        let g = 1.0e-3 + 4.0e-3 * k as f32 / 2000.0;
        v.push([g, g, g]);
        let t = 0.7 + 0.12 * k as f32 / 2000.0;
        v.push([t, 0.5 * t, 0.25 * t]);
        let x = 0.06 + 0.04 * k as f32 / 2000.0;
        v.push([x, x + 0.3, x + 0.6]);
    }
    v
}

/// A curve leaf over the Eval backend.
type Leaf = fn([f32; 3]) -> [f32; 3];
/// Its f64 transcription.
type Reference = fn([Tracked; 3]) -> [Tracked; 3];

/// Sweeps one curve leaf against its f64 reference, printing the worst `|err| / tol`.
fn sweep(name: &str, leaf: Leaf, reference: Reference) {
    let mut worst = 0.0f64;
    let mut bad: Vec<String> = Vec::new();
    for c in curve_inputs() {
        let got = leaf(c);
        let want = reference(c.map(Tracked::exact));
        for ch in 0..3 {
            let err = (f64::from(got[ch]) - want[ch].v).abs();
            let tol = K * want[ch].b + FLOOR;
            worst = worst.max(err / tol);
            let within = err <= tol;
            if !within && bad.len() < 6 {
                bad.push(format!("{name}({c:?})[{ch}] = {:e}, f64 {:e}, |err| {err:e} > tol {tol:e}", got[ch], want[ch].v));
            }
        }
    }
    eprintln!("{name}: worst |err|/tol = {worst:.4}");
    assert!(bad.is_empty(), "{name} disagrees with its f64 transcription:\n{}", bad.join("\n"));
}

#[test]
fn aces_hill_matches_f64_bakinglab() {
    sweep("aces_hill", aces_hill::<EvalCf>, aces_f64);
}

#[test]
fn khronos_pbr_neutral_matches_f64_spec() {
    sweep("khronos_pbr_neutral", khronos_pbr_neutral::<EvalCf>, neutral_f64);
}

#[test]
fn reinhard_jodie_matches_f64() {
    sweep("reinhard_jodie", reinhard_jodie::<EvalCf>, reinhard_jodie_f64);
}

#[test]
fn tonemap_select_equals_each_curve() {
    // Word -> the curve it must dispatch to, bit for bit (the dispatch adds no arithmetic). Word 7
    // is unknown and falls back to ACES; the high-bit noise checks the field extraction.
    let cases: [(u32, Leaf, &str); 4] = [
        (0, aces_hill::<EvalCf>, "aces_hill"),
        (1, khronos_pbr_neutral::<EvalCf>, "khronos_pbr_neutral"),
        (2, reinhard_jodie::<EvalCf>, "reinhard_jodie"),
        (7, aces_hill::<EvalCf>, "aces_hill (unknown word 7)"),
    ];
    for (word, curve, name) in cases {
        let mode = (((word << 8) | 0x00FF_F0FF) >> 8) & 0xF;
        assert_eq!(mode, word, "the noise pattern must leave bits 8..11 alone");
        for c in curve_inputs().into_iter().step_by(7) {
            let mode_words = (word << 8) | 0xFFFF_F0FF;
            let got = tonemap_select::<EvalCf>(c, boyko_shaderdsl::post::tonemapper::<EvalCf>(mode_words));
            let want = curve(c);
            assert_eq!(got.map(f32::to_bits), want.map(f32::to_bits), "tonemap_select word {word} != {name} at {c:?}");
        }
    }
}

#[test]
fn oetf_gamma22_matches_f64() {
    // pow(t, 1/2.2) over the curves' output range [0, 1]: within 2 ULP of the f64 power of the
    // f32 exponent, plus the 2⁻¹²⁶ floor.
    let p = post_f64::oetf_exponent();
    let mut worst = 0.0f64;
    for k in 0..=100_000u32 {
        let t = if k == 0 { 0.0 } else { (-30.0 * f64::from(100_000 - k) / 100_000.0).exp2() as f32 };
        let got = oetf_gamma22::<EvalCf>([t, t, t]);
        let want = f64::from(t).powf(p);
        let tol = 2.0 * ulp32(want) + FLOOR;
        for g in got {
            let err = (f64::from(g) - want).abs();
            worst = worst.max(err / tol);
            assert!(err <= tol, "oetf_gamma22({t:e}) = {g:e}, f64 {want:e}, |err| {err:e} > tol {tol:e}");
        }
    }
    eprintln!("oetf_gamma22: worst |err|/tol = {worst:.4}");
}

#[test]
fn srgb_eotf_round_trips_every_8bit_code() {
    for k in 0..=255u32 {
        let e = k as f32 / 255.0;
        let l = srgb_eotf::<EvalCf>([e, e, e]);
        for v in l {
            let back = (255.0 * srgb_oetf_f64(f64::from(v))).round() as u32;
            assert_eq!(back, k, "srgb_eotf({k}/255) = {v:e} re-encodes to {back}, not {k}");
        }
    }
}

#[test]
fn hdr_sanitize_table() {
    // H2 test 2: NaN, ±Inf, −1, 64512, 64513, 1e30 → 0, 0, 0, 64512, 64512, 64512 (each as a grey
    // pixel); one NaN channel kills the whole pixel; a negative channel clamps alone.
    let rows: [([f32; 3], [f32; 3]); 8] = [
        ([f32::NAN; 3], [0.0; 3]),
        ([f32::INFINITY; 3], [0.0; 3]),
        ([f32::NEG_INFINITY; 3], [0.0; 3]),
        ([-1.0; 3], [0.0; 3]),
        ([64512.0; 3], [64512.0; 3]),
        ([64513.0; 3], [64512.0; 3]),
        ([1.0e30; 3], [64512.0; 3]),
        ([f32::NAN, 0.5, 0.5], [0.0; 3]),
    ];
    for (input, want) in rows {
        let got = hdr_sanitize::<EvalCf>(input);
        assert_eq!(got.map(f32::to_bits), want.map(f32::to_bits), "hdr_sanitize({input:?}) = {got:?}, want {want:?}");
    }
    let got = hdr_sanitize::<EvalCf>([-0.01, 0.5, 0.5]);
    assert_eq!(got, [0.0, 0.5, 0.5], "a negative channel clamps to 0 alone (the pixel is finite)");
}

#[test]
fn pre_ratio_apply_scales() {
    // One f32 multiply per channel: the f64 product rounded once (the correctly rounded product).
    let mut rng = Rng(0x9E_EA71);
    for _ in 0..10_000 {
        let h: [f32; 3] = core::array::from_fn(|_| rng.log2_range(-20.0, 15.9));
        for r in [1.0f32, 0.5, 2.0, rng.log2_range(-4.0, 4.0)] {
            let got = pre_ratio_apply::<EvalCf>(h, r);
            let want = h.map(|v| (f64::from(v) * f64::from(r)) as f32);
            assert_eq!(got, want, "pre_ratio_apply({h:?}, {r})");
        }
    }
}

#[test]
fn eval_params_reads_the_ubo_words() {
    // The words as `PostFinalGpu` lays them out: mode_words@0, exposure_ratio@4, extent@8.
    let words = [0x0000_0200u32, 0.5f32.to_bits(), 1920, 1080, 0xDEAD_0001, 0xDEAD_0002, 0xDEAD_0003, 0xDEAD_0004];
    let p = eval_params(words);
    assert_eq!(p.mode_words, 0x0000_0200, "mode_words is word 0");
    assert_eq!(p.exposure_ratio.to_bits(), 0.5f32.to_bits(), "exposure_ratio is word 1");
    assert_eq!(p.extent, [1920, 1080], "extent is words 2..4");
}
