//! Render wave PXe — the post leaves' `EvalCf` oracles (`plan/H2-PXe-post-edsl.md` tests 1–4, the
//! UBO reader).
//!
//! Every curve is compared with the INDEPENDENT `f64` transcription of its cited source
//! (`tests/post_f64/mod.rs` — it imports no `post.rs` constant) under the conditioning-scaled
//! running bound of render-wave rule (xvi) plus `2⁻¹²⁶` — not a fixed ULP count, which is red on a
//! correct `aces_hill` near its numerator root (`v ≈ 3.25·10⁻³`, where the output cancels to ≈ 0).
//! A transcription error in a matrix entry or a rational coefficient moves the leaf by far more
//! than that bound (≈ `10⁻⁶` relative); the run prints each leaf's worst `|err| / tol`.
//!
//! Render wave PX1b (`px1b/cut.md` D-T1, PC-PX1b-3): the RCAS leaves (`rcas_lobe` + `rcas_resolve`)
//! against the f64 transcription of FSR 1's `FsrRcasF`, in reversible space, on H5's step fixtures
//! and 10⁴ seeded crosses; the body's NaN guard against the LITERAL FSR 1 on all-black ring channels;
//! FSR 2's reversible pair round trip; and RCAS's identity on a flat cross — each under the same
//! running bound plus `2⁻¹²⁶`, not a fixed ULP count.

mod post_f64;

use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::post::{
    aces_hill, eval_params, hdr_sanitize, khronos_pbr_neutral, oetf_gamma22, pre_ratio_apply,
    rcas_lobe, rcas_resolve, reinhard_jodie, srgb_eotf, tonemap_reversible, tonemap_reversible_inverse,
    tonemap_select,
};
use post_f64::{
    Cross, FLOOR, Tracked, aces_f64, neutral_f64, rcas_f64, rcas_fsr1_literal, rcas_lobe_f64,
    reinhard_jodie_f64, reversible_f64, reversible_inverse_f64, srgb_oetf_f64, ulp32,
};

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

// ---- Render wave PX1b: RCAS ------------------------------------------------------------------------

/// The `con` values swept: FSR 1's maximum, one stop, the default `rcas_sharpness = 0.25` (1.5 stops,
/// ≈ 0.354), FSR 2's mildest (2 stops) and none.
const CONS: [f32; 5] = [1.0, 0.5, 0.354, 0.25, 0.0];

/// The Eval RCAS of a reversible-space cross: `(lobe, pixel)`.
fn rcas_eval(x: Cross<[f32; 3]>, con: f32) -> (f32, [f32; 3]) {
    let lobe = rcas_lobe::<EvalCf>(x.b, x.d, x.e, x.f, x.h, con);
    (lobe, rcas_resolve::<EvalCf>(lobe, x.b, x.d, x.e, x.f, x.h))
}

/// The cross whose tap `k` (0 = b, 1 = d, 2 = e, 3 = f, 4 = h) is `taps[k]`.
fn cross(taps: [[f32; 3]; 5]) -> Cross<[f32; 3]> {
    Cross { b: taps[0], d: taps[1], e: taps[2], f: taps[3], h: taps[4] }
}

/// H5's step fixtures in reversible space — every assignment of a step's two levels to the five taps
/// (`0.3 | 0.6` and `0.1 | 0.9`, grey and per channel, so the step edge on either side, the
/// 3-dark/1-bright cross and its inverse, and the flat crosses are all in) — then 10⁴ seeded crosses
/// over `[0, 1 − 2⁻¹²]` (reversible space never reaches 1: `y ≤ 64512/64513`).
fn rcas_crosses() -> Vec<Cross<[f32; 3]>> {
    let mut v = Vec::new();
    for (lo, hi) in [(0.3f32, 0.6f32), (0.1, 0.9)] {
        for mask in 0..32u32 {
            let level = |k: u32| if (mask >> k) & 1 == 1 { hi } else { lo };
            v.push(cross(core::array::from_fn(|k| [level(k as u32); 3])));
            // The same step on one channel only (the lobe is the channels' max).
            v.push(cross(core::array::from_fn(|k| [level(k as u32), 0.5, 0.2])));
        }
    }
    let mut rng = Rng(0x1B_5C01);
    let top = 1.0 - 1.0 / 4096.0;
    for _ in 0..10_000 {
        v.push(cross(core::array::from_fn(|_| core::array::from_fn(|_| (rng.unit() * top) as f32))));
    }
    v
}

/// `|got − want| ≤ K·b + 2⁻¹²⁶` per channel, collecting at most six failures.
fn check3(what: &str, got: [f32; 3], want: [Tracked; 3], worst: &mut f64, bad: &mut Vec<String>) {
    for ch in 0..3 {
        let err = (f64::from(got[ch]) - want[ch].v).abs();
        let tol = K * want[ch].b + FLOOR;
        *worst = worst.max(err / tol);
        let within = err <= tol;
        if !within && bad.len() < 6 {
            bad.push(format!("{what}[{ch}] = {:e}, f64 {:e}, |err| {err:e} > tol {tol:e}", got[ch], want[ch].v));
        }
    }
}

#[test]
fn rcas_matches_f64_fsr1_on_the_step_fixtures() {
    let mut worst = 0.0f64;
    let mut bad: Vec<String> = Vec::new();
    for x in rcas_crosses() {
        for con in CONS {
            let (lobe, got) = rcas_eval(x, con);
            let xt = x.map(|t| t.map(Tracked::exact));
            let want_lobe = rcas_lobe_f64(xt, Tracked::exact(con));
            let err = (f64::from(lobe) - want_lobe.v).abs();
            let tol = K * want_lobe.b + FLOOR;
            worst = worst.max(err / tol);
            let within = err <= tol;
            if !within && bad.len() < 6 {
                bad.push(format!("rcas_lobe({x:?}, {con}) = {lobe:e}, f64 {:e}, |err| {err:e} > tol {tol:e}", want_lobe.v));
            }
            check3(&format!("rcas({x:?}, {con})"), got, rcas_f64(xt, Tracked::exact(con)), &mut worst, &mut bad);
        }
    }
    eprintln!("rcas: worst |err|/tol = {worst:.4}");
    assert!(bad.is_empty(), "the RCAS leaves disagree with FSR 1's FsrRcasF in f64:\n{}", bad.join("\n"));
}

#[test]
fn rcas_lobe_on_a_black_ring_equals_fsr1() {
    // `px1b/cut.md` D-R3: an all-black ring channel (mx4 = 0) is FSR 1's `hitMin = 0·rcp(0)`, a NaN
    // FSR 1 survives only through `max`'s NaN rule; the body guards it with a select instead. The
    // reference here is FSR 1 LITERALLY, NaN and all, so the guard is checked, not assumed.
    let mut rng = Rng(0x1B_5C02);
    let mut cases: Vec<Cross<[f32; 3]>> = Vec::new();
    for centre in [0.0f32, 0.25, 0.5, 0.9] {
        cases.push(cross([[0.0; 3], [0.0; 3], [centre; 3], [0.0; 3], [0.0; 3]]));
        for ch in 0..3 {
            let mut taps: [[f32; 3]; 5] = core::array::from_fn(|_| core::array::from_fn(|_| (rng.unit() * 0.99) as f32));
            for (k, t) in taps.iter_mut().enumerate() {
                t[ch] = if k == 2 { centre } else { 0.0 };
            }
            cases.push(cross(taps));
        }
    }
    let mut worst = 0.0f64;
    let mut bad: Vec<String> = Vec::new();
    for x in cases {
        for con in CONS {
            let (lobe, got) = rcas_eval(x, con);
            let (want_lobe, want, nan_hit_min) = rcas_fsr1_literal(x.map(|t| t.map(f64::from)), f64::from(con));
            assert!(nan_hit_min, "PREMISE: {x:?} drives FSR 1's hitMin through 0·rcp(0)");
            assert!(lobe.is_finite() && got.iter().all(|v| v.is_finite()), "the body produced a non-finite value on {x:?}: lobe {lobe}, {got:?}");
            // The tolerance is the running bound of the same cross (`Tracked`, whose lazy arm is the
            // literal result); the VALUE compared is FSR 1's own.
            let xt = x.map(|t| t.map(Tracked::exact));
            let bound_lobe = rcas_lobe_f64(xt, Tracked::exact(con));
            let tol = K * bound_lobe.b + FLOOR;
            let err = (f64::from(lobe) - want_lobe).abs();
            worst = worst.max(err / tol);
            if err > tol && bad.len() < 6 {
                bad.push(format!("rcas_lobe({x:?}, {con}) = {lobe:e}, FSR 1 {want_lobe:e}, |err| {err:e} > tol {tol:e}"));
            }
            let bound = rcas_f64(xt, Tracked::exact(con));
            let literal: [Tracked; 3] = core::array::from_fn(|c| Tracked { v: want[c], b: bound[c].b });
            check3(&format!("rcas({x:?}, {con}) vs literal FSR 1"), got, literal, &mut worst, &mut bad);
        }
    }
    eprintln!("rcas black ring: worst |err|/tol = {worst:.4}");
    assert!(bad.is_empty(), "the guarded lobe is not FSR 1's on an all-black ring channel:\n{}", bad.join("\n"));
}

#[test]
fn tonemap_reversible_round_trips() {
    // FSR 2's pair, both ways. Forward then inverse over the sanitized domain [0, HDR_MAX]: the
    // inverse's floor `1 − max3(y) ≥ 1/(1 + 64512) > RCAS_INV_EPS` is never reached there (the
    // round-trip domain of the wrap's INPUT is [0, 1/RCAS_INV_EPS − 1 = 65503]). Inverse then forward
    // over [0, 1 − RCAS_INV_EPS].
    let mut rng = Rng(0x1B_5C03);
    let mut worst = 0.0f64;
    let mut bad: Vec<String> = Vec::new();
    let mut hdr: Vec<[f32; 3]> = (0..20_000).map(|_| core::array::from_fn(|_| rng.log2_range(-20.0, 15.977))).collect();
    hdr.extend([[0.0; 3], [64512.0; 3], [64512.0, 0.0, 1.0], [0.0, 0.0, 10.0]]);
    for c in hdr {
        let got = tonemap_reversible_inverse::<EvalCf>(tonemap_reversible::<EvalCf>(c));
        let want = reversible_inverse_f64(reversible_f64(c.map(Tracked::exact)));
        for ch in 0..3 {
            assert!((want[ch].v - f64::from(c[ch])).abs() <= 1e-9 * f64::from(c[ch]).max(1.0), "PREMISE: the f64 pair round-trips {c:?}");
        }
        check3(&format!("inverse(forward({c:?}))"), got, want, &mut worst, &mut bad);
    }
    let top = 1.0 - 1.0 / 65504.0;
    let mut ys: Vec<[f32; 3]> = (0..20_000).map(|_| core::array::from_fn(|_| (rng.unit() * top) as f32)).collect();
    ys.extend([[0.0; 3], [top as f32; 3], [top as f32, 0.0, 0.5]]);
    for y in ys {
        let got = tonemap_reversible::<EvalCf>(tonemap_reversible_inverse::<EvalCf>(y));
        let want = reversible_f64(reversible_inverse_f64(y.map(Tracked::exact)));
        check3(&format!("forward(inverse({y:?}))"), got, want, &mut worst, &mut bad);
    }
    eprintln!("tonemap_reversible pair: worst |err|/tol = {worst:.4}");
    assert!(bad.is_empty(), "FSR 2's reversible pair does not round-trip within its bound:\n{}", bad.join("\n"));
}

#[test]
fn rcas_is_the_identity_on_a_flat_cross() {
    // On a flat cross RCAS is the identity (`(e + 4·lobe·e) / (4·lobe + 1) = e`), but NOT because the
    // lobe vanishes: hitMin = 1/4 and hitMax = −1/4 there, so the lobe sits at its clamp −0.1875·con.
    let mut rng = Rng(0x1B_5C04);
    let mut worst = 0.0f64;
    let mut bad: Vec<String> = Vec::new();
    let mut levels: Vec<[f32; 3]> = (0..=64).map(|k| [k as f32 / 65.0; 3]).collect();
    levels.extend((0..2000).map(|_| core::array::from_fn(|_| (rng.unit() * 0.999) as f32)));
    for e in levels {
        let x = cross([e; 5]);
        for con in CONS {
            let (lobe, got) = rcas_eval(x, con);
            assert_eq!(lobe.to_bits(), (-0.1875f32 * con).to_bits(), "the flat lobe of {e:?} at con {con} is {lobe}, not the clamp −0.1875·con");
            let want = e.map(|v| Tracked { v: f64::from(v), b: 0.0 });
            let bound = rcas_f64(x.map(|t| t.map(Tracked::exact)), Tracked::exact(con));
            let want: [Tracked; 3] = core::array::from_fn(|c| Tracked { b: bound[c].b, ..want[c] });
            check3(&format!("rcas(flat {e:?}, {con})"), got, want, &mut worst, &mut bad);
        }
    }
    eprintln!("rcas flat: worst |err|/tol = {worst:.4}");
    assert!(bad.is_empty(), "RCAS is not the identity on a flat cross:\n{}", bad.join("\n"));
}

#[test]
fn eval_params_reads_the_ubo_words() {
    // The words as `PostFinalGpu` lays them out: mode_words@0, exposure_ratio@4, extent@8,
    // sharpness@16 (render wave PX1b; the three reserved words after it are read by no one).
    let words = [0x0000_0200u32, 0.5f32.to_bits(), 1920, 1080, 0.375f32.to_bits(), 0xDEAD_0002, 0xDEAD_0003, 0xDEAD_0004];
    let p = eval_params(words);
    assert_eq!(p.mode_words, 0x0000_0200, "mode_words is word 0");
    assert_eq!(p.exposure_ratio.to_bits(), 0.5f32.to_bits(), "exposure_ratio is word 1");
    assert_eq!(p.extent, [1920, 1080], "extent is words 2..4");
    assert_eq!(p.sharpness.to_bits(), 0.375f32.to_bits(), "sharpness is word 4");
}
