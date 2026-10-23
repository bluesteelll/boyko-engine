//! Render wave PXe — the host-evaluable resource facets (`Cf::{tex2_load, tex2_sample_linear,
//! tex3_sample_linear, vec4_xyz}` over `HostTex2` / `HostTex3`; `plan/H2-PXe-post-edsl.md` test 6).
//!
//! - **Filter oracles** — Eval bilinear / trilinear against an `f64` evaluation of the stated
//!   sub-texel model (texel centres at `(i + ½)/n`, clamp-to-edge, exact weights) on random
//!   textures and coordinates, the clamped border included. Tolerance (rule (xvi)): the f32 texel-
//!   space coordinate carries two roundings, which move the result by at most its texel-difference
//!   Lipschitz constant times their size, plus the filter's own roundings — far below the
//!   half-texel error of a filter without the `− ½` (the mutation class this gate exists for).
//! - **Load bounds** — an in-range `Load` reads the addressed texel; `(w, 0)`, `(−1, 0)` and any
//!   read of the 0×0 disabled-row stand-in PANIC (always on, not a `debug_assert!`): an Eval `Load`
//!   that silently clamped would hide a body without its border clamp, and a flattened `y·w + x` at
//!   `x = w` would silently read the next row.
//! - **`usub` on Eval** (render wave PX1b) — the modular `uint` subtract RCAS's border clamp reads.
//! - **Emit** (`feature = "emit"`) — each facet's exact HLSL spelling.

use boyko_shaderdsl::cf::{Cf, EvalCf, HostTex2, HostTex3};

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
}

/// `2⁻²⁴`.
const U: f64 = 1.0 / 16_777_216.0;
/// `2⁻¹²⁶`.
const FLOOR: f64 = 1.175_494_350_822_287_5e-38;

/// The f64 linear-clamp weights of one axis: `p = u·n − ½`, `i0 = ⌊p⌋`, `α = p − i0`, both taps
/// clamped to `[0, n − 1]`. Returns the taps, the weight, and the bound on the f32 evaluation's
/// error in `p` (the product's and the subtraction's roundings).
fn axis64(u: f32, n: u32) -> (usize, usize, f64, f64) {
    let un = f64::from(u) * f64::from(n);
    let p = un - 0.5;
    let i0 = p.floor();
    let alpha = p - i0;
    let clamp = |i: f64| i.clamp(0.0, f64::from(n - 1)) as usize;
    let dp = U * un.abs() + U * (un.abs() + 0.5) + U;
    (clamp(i0), clamp(i0 + 1.0), alpha, dp)
}

/// The largest channel magnitude and the channel's spread `max − min` over a whole texture — the
/// bound of the filter's slope per unit of texel-space coordinate in any cell, so an f32 coordinate
/// that rounds into the NEIGHBOURING cell is covered too (the filter is continuous across cells).
fn spread(ts: &[[f32; 4]], ch: usize) -> (f64, f64) {
    let vals: Vec<f64> = ts.iter().map(|t| f64::from(t[ch])).collect();
    let max = vals.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let lo = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    (max, hi - lo)
}

fn random_texels(rng: &mut Rng, n: usize) -> Vec<[f32; 4]> {
    (0..n).map(|_| core::array::from_fn(|_| rng.unit() as f32)).collect()
}

#[test]
fn tex2_sample_linear_matches_f64_bilinear() {
    let (w, h) = (7u32, 5u32);
    let mut rng = Rng(0xB1_11EA);
    let texels = random_texels(&mut rng, (w * h) as usize);
    let tex = HostTex2::new(&texels, w, h);
    let at = |x: usize, y: usize| texels[y * w as usize + x];
    let mut worst = 0.0f64;
    for i in 0..20_000 {
        // uv over [-0.5, 1.5]²: the clamped border on every side, and texel centres exactly.
        let uv = if i < 35 {
            [((i % 7) as f32 + 0.5) / 7.0, ((i / 7) as f32 + 0.5) / 5.0]
        } else {
            [(rng.unit() * 2.0 - 0.5) as f32, (rng.unit() * 2.0 - 0.5) as f32]
        };
        let got = <EvalCf as Cf>::tex2_sample_linear(tex, uv);
        let (x0, x1, ax, dpx) = axis64(uv[0], w);
        let (y0, y1, ay, dpy) = axis64(uv[1], h);
        let taps = [at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1)];
        for ch in 0..4 {
            let t = |k: usize| f64::from(taps[k][ch]);
            let top = t(0) * (1.0 - ax) + t(1) * ax;
            let bottom = t(2) * (1.0 - ax) + t(3) * ax;
            let want = top * (1.0 - ay) + bottom * ay;
            let (m, d) = spread(&texels, ch);
            let tol = 2.0 * d * (dpx + dpy) + 10.0 * U * m + FLOOR;
            let err = (f64::from(got[ch]) - want).abs();
            worst = worst.max(err / tol);
            assert!(err <= tol, "bilinear uv {uv:?} ch {ch}: Eval {:e}, f64 {want:e}, |err| {err:e} > tol {tol:e}", got[ch]);
        }
    }
    eprintln!("tex2_sample_linear: worst |err|/tol = {worst:.4}");
}

#[test]
fn tex3_sample_linear_matches_f64_trilinear() {
    let (w, h, d) = (5u32, 4u32, 3u32);
    let mut rng = Rng(0x7E_1EA3);
    let texels = random_texels(&mut rng, (w * h * d) as usize);
    let tex = HostTex3::new(&texels, w, h, d);
    let at = |x: usize, y: usize, z: usize| texels[(z * h as usize + y) * w as usize + x];
    let mut worst = 0.0f64;
    for _ in 0..20_000 {
        let uvw = [(rng.unit() * 2.0 - 0.5) as f32, (rng.unit() * 2.0 - 0.5) as f32, (rng.unit() * 2.0 - 0.5) as f32];
        let got = <EvalCf as Cf>::tex3_sample_linear(tex, uvw);
        let (x0, x1, ax, dpx) = axis64(uvw[0], w);
        let (y0, y1, ay, dpy) = axis64(uvw[1], h);
        let (z0, z1, az, dpz) = axis64(uvw[2], d);
        let taps = [
            at(x0, y0, z0),
            at(x1, y0, z0),
            at(x0, y1, z0),
            at(x1, y1, z0),
            at(x0, y0, z1),
            at(x1, y0, z1),
            at(x0, y1, z1),
            at(x1, y1, z1),
        ];
        for ch in 0..4 {
            let t = |k: usize| f64::from(taps[k][ch]);
            let plane = |o: usize| {
                let top = t(o) * (1.0 - ax) + t(o + 1) * ax;
                let bottom = t(o + 2) * (1.0 - ax) + t(o + 3) * ax;
                top * (1.0 - ay) + bottom * ay
            };
            let want = plane(0) * (1.0 - az) + plane(4) * az;
            let (m, dd) = spread(&texels, ch);
            let tol = 2.0 * dd * (dpx + dpy + dpz) + 14.0 * U * m + FLOOR;
            let err = (f64::from(got[ch]) - want).abs();
            worst = worst.max(err / tol);
            assert!(err <= tol, "trilinear uvw {uvw:?} ch {ch}: Eval {:e}, f64 {want:e}, |err| {err:e} > tol {tol:e}", got[ch]);
        }
    }
    eprintln!("tex3_sample_linear: worst |err|/tol = {worst:.4}");
}

#[test]
fn tex2_load_reads_the_addressed_texel() {
    let (w, h) = (7u32, 5u32);
    let texels: Vec<[f32; 4]> = (0..w * h).map(|i| [i as f32, -(i as f32), 0.5, 1.0]).collect();
    let tex = HostTex2::new(&texels, w, h);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let got = <EvalCf as Cf>::tex2_load(tex, x, y);
            assert_eq!(got, texels[(y as u32 * w + x as u32) as usize], "Load({x}, {y})");
        }
    }
}

#[test]
#[should_panic(expected = "tex2_load out of the texture")]
fn tex2_load_past_the_right_edge_panics() {
    let texels = vec![[0.0f32; 4]; 7 * 5];
    let tex = HostTex2::new(&texels, 7, 5);
    let _ = <EvalCf as Cf>::tex2_load(tex, 7, 0);
}

#[test]
#[should_panic(expected = "tex2_load out of the texture")]
fn tex2_load_negative_coordinate_panics() {
    let texels = vec![[0.0f32; 4]; 7 * 5];
    let tex = HostTex2::new(&texels, 7, 5);
    let _ = <EvalCf as Cf>::tex2_load(tex, -1, 0);
}

#[test]
#[should_panic(expected = "tex2_load out of the texture")]
fn tex2_load_on_the_disabled_stand_in_panics() {
    // A disabled binding row resolves to the 0×0 stand-in: ANY read of it is a red.
    let _ = <EvalCf as Cf>::tex2_load(HostTex2::EMPTY, 0, 0);
}

#[test]
fn eval_usub_wraps_like_hlsls_isub() {
    // Render wave PX1b (`px1b/cut.md` D-R6, O10): RCAS's border clamp `min(x − 1u, x)` relies on
    // `0u − 1u` wrapping to `0xFFFFFFFF` (SPIR-V's modular `OpISub`), so Eval must wrap too — never a
    // debug-build overflow panic, never a saturating 0 (which would hide a missing clamp).
    assert_eq!(<EvalCf as Cf>::usub(0, 1), u32::MAX, "0u - 1u wraps");
    assert_eq!(<EvalCf as Cf>::usub(7, 3), 4, "an in-range uint subtract");
    assert_eq!(<EvalCf as Cf>::umin(<EvalCf as Cf>::usub(0, 1), 0), 0, "the clamp's left edge: min(0u - 1u, 0u) = 0");
}

#[cfg(feature = "emit")]
#[test]
fn each_facet_prints_its_hlsl() {
    use boyko_shaderdsl::emit::{PostFacet, emit_hlsl_post_facet};
    let want = [
        (PostFacet::Tex2Load, "    return gTex.Load(int3((int)px.x, (int)px.y, 0));\n"),
        (PostFacet::Tex2SampleLinear, "    return gTex.SampleLevel(gLinearClamp, uv, 0);\n"),
        (PostFacet::Tex3SampleLinear, "    return gLut.SampleLevel(gLinearClamp, uvw, 0);\n"),
        (PostFacet::Vec4Xyz, "    float3 rgb = v.xyz;\n    return float4(rgb.x, rgb.y, rgb.z, 1.0);\n"),
    ];
    let mut bad = Vec::new();
    for (facet, text) in want {
        let got = emit_hlsl_post_facet(facet);
        if got != text {
            bad.push(format!("{facet:?}: printed {got:?}, want {text:?}"));
        }
    }
    assert!(bad.is_empty(), "facet emit spellings wrong:\n{}", bad.join("\n"));
}
