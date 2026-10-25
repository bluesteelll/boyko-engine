//! IBL1 R0 + R1 — the prefiltered environment: the read-side leaves the six lit producers share
//! through the generated `env_ibl.hlsli`, the deterministic host helpers, and the host bake of the
//! procedural studio room into a bordered octahedral 7-level B10G11R11 chain.
//!
//! # Two instantiations, one mapping (design D3)
//!
//! Every leaf the read side runs is authored ONCE over [`FieldScalar`] and instantiated twice:
//! over `f32` (the bake and the host oracles) and over `Emit` (the HLSL spans of
//! `crates/boyko_rhi_vulkan/shaders/env_ibl.hlsli`, `crate::emit::emit_hlsl_env_ibl`). So the
//! bake and the shader read the SAME roughness→lod map ([`env_lod_body`]), the SAME
//! direction→texel map ([`oct_env_uv_body`]) and the SAME combine ([`eval_pbr_ambient_env_body`]).
//!
//! # The tile (design D1/D2)
//!
//! A level of side `N` is an octahedral map with the pole on +y (the engine's `LIGHT_UP`): the
//! upper hemisphere is the central diamond. Its `(N − 2)²` interior texels cover the signed square
//! `[−1, 1]²`; a one-texel border repeats the octahedral CONTINUATION of the interior
//! ([`oct_env_border_source`]: an edge texel mirrors along its edge, a corner takes the diagonally
//! opposite interior corner — the DDGI rule `ddgi_oct_border_host_oracle.rs` proved), so a
//! bilinear fetch at any interior UV reads only true neighbours. Each level is fetched at its OWN
//! UV ([`oct_env_uv_body`] with that level's side): the interior scale `(N − 2)/N` differs per
//! level, so one trilinear fetch at one UV would misregister the coarser level by up to half a
//! texel at the tile edge.
//!
//! # Bit-reproducibility (cut D-P2a, critique W2)
//!
//! The committed chain must regenerate byte-for-byte on every host (MSVC here, glibc on CI), so
//! the bake runs on IEEE-exact operations only: `+ − × ÷`, `sqrt`, `min`/`max`, `abs`,
//! comparisons, integer and bit operations, numeric casts. Two leaves stand in for the platform
//! transcendentals: [`det_sincos_turns`] and [`det_log2`] (fixed polynomials after an exact range
//! reduction). The leaves are generic over [`FieldScalar`], whose surface HAS no transcendental
//! (a `S::sin` there is a compile error, not a review finding); the non-generic bake code is
//! held to the same rule by the `ibl_bake_is_libm_free` census over this file. Rust never
//! contracts `a·b + c` into an FMA.
//!
//! # No heap
//!
//! The bake is slice-in / slice-out: the caller owns the chain words and the source scratch
//! ([`env_source_scratch_len`]). Nothing here allocates.

use crate::scalar::{self, FieldScalar};

// ---- The SKY-row contract (D-P2d) ----------------------------------------------------------------

/// The SKY row's kind-word flag that marks it the ENVIRONMENT row (bit 24: above the P6 R1
/// `casts_sdf_shadow` bit 16 and the point/spot slot field, bits 17..22). Every consumer masks the
/// kind with `LIGHT_KIND_MASK` (0xFFFF), so a flagged row stays a SKY row to every dispatch;
/// only the REFL producers test the flag. `boyko_render::light::LIGHT_FLAG_ENV` is const-asserted
/// equal.
pub const LIGHT_FLAG_ENV: u32 = 1 << 24;

// ---- Read-side leaves (emitted into env_ibl.hlsli) ----------------------------------------------

/// The quadratic roughness→lod map's constant `2` (`L1 · pR · (2 − pR)`).
const LOD_TWO: f32 = 2.0;

/// The prefiltered level a perceptual roughness `pr` reads: `l1 · pr · (2 − pr)` (Filament's
/// `perceptualRoughnessToLod`; `l1` = levels − 1). The ONE mapping the bake inverts
/// ([`env_lod_inv_body`]) and the shader reads (design D3).
#[inline]
pub fn env_lod_body<S: FieldScalar>(pr: S, l1: S) -> S {
    l1.mul(pr.mul(S::lit(LOD_TWO).sub(pr)))
}

/// The octahedral fold of a direction `d` (any length) into the signed square `[−1, 1]²`, pole on
/// +y: `p = d / |d|₁`; the upper hemisphere keeps `(p.x, p.z)`, the lower folds to
/// `((1 − |p.z|)·sgn p.x, (1 − |p.x|)·sgn p.z)` with `sgn 0 = +1`. Branchless (two selects).
#[inline]
pub fn oct_env_signed_body<S: FieldScalar<Vec3 = [S; 3]>>(d: [S; 3]) -> [S; 2] {
    let zero = S::lit(0.0);
    let one = S::lit(1.0);
    let inv = one.div(d[0].abs().add(d[1].abs()).add(d[2].abs()));
    let px = d[0].mul(inv);
    let py = d[1].mul(inv);
    let pz = d[2].mul(inv);
    let sx = S::select(px.ge(zero), one, S::lit(-1.0));
    let sz = S::select(pz.ge(zero), one, S::lit(-1.0));
    let fu = one.sub(pz.abs()).mul(sx);
    let fv = one.sub(px.abs()).mul(sz);
    let lower = py.lt(zero);
    [S::select(lower, fu, px), S::select(lower, fv, pz)]
}

/// The UV a direction `d` reads in a bordered level of side `tile`: the signed fold
/// ([`oct_env_signed_body`]) remapped to `e = s·½ + ½ ∈ [0, 1]²`, then into the interior texels
/// `[1, tile − 1]` as `uv = e·(tile − 2)/tile + 1/tile`.
#[inline]
pub fn oct_env_uv_body<S: FieldScalar<Vec3 = [S; 3]>>(d: [S; 3], tile: S) -> [S; 2] {
    let half = S::lit(0.5);
    let one = S::lit(1.0);
    let s = oct_env_signed_body(d);
    let scale = tile.sub(S::lit(2.0)).div(tile);
    let bias = one.div(tile);
    [
        s[0].mul(half).add(half).mul(scale).add(bias),
        s[1].mul(half).add(half).mul(scale).add(bias),
    ]
}

/// The specular dominant direction of a GGX lobe (Lagarde & de Rousiers 2014; Unity's
/// `GetSpecularDominantDir`): `normalize(lerp(n, r, (1 − pr)·(√(1 − pr) + pr)))` — the reflection
/// vector bent toward the normal as the lobe widens. `pr = 0` gives `r`, `pr = 1` gives `n`.
#[inline]
pub fn spec_dominant_dir_body<S: FieldScalar<Vec3 = [S; 3]>>(n: [S; 3], r: [S; 3], pr: S) -> [S; 3] {
    let one = S::lit(1.0);
    let smooth = one.sub(pr);
    let f = smooth.mul(smooth.sqrt().add(pr));
    let d = [n[0].lerp(r[0], f), n[1].lerp(r[1], f), n[2].lerp(r[2], f)];
    let inv = one.div(scalar::v_dot(d, d).sqrt());
    scalar::v_scale(d, inv)
}

/// The environment yaw: an environment rotated by `+yaw` about +y (right-handed) is looked up
/// along `R_y(−yaw)·d = (c·d.x − s·d.z, d.y, s·d.x + c·d.z)` with `c = cos yaw`, `s = sin yaw`
/// (the host evaluates the pair once at the fold and ships it in the SKY row), so a feature at
/// map direction `e` appears at world direction `R_y(yaw)·e`.
#[inline]
pub fn env_yaw_rotate_body<S: FieldScalar<Vec3 = [S; 3]>>(d: [S; 3], c: S, s: S) -> [S; 3] {
    [c.mul(d[0]).sub(s.mul(d[2])), d[1], s.mul(d[0]).add(c.mul(d[2]))]
}

/// The ambient term of the ENVIRONMENT arm: `eval_pbr_ambient_hemi`'s combine with the reflected
/// hemisphere colour replaced by the prefiltered `env_spec` (`pbr_lighting.hlsli`, the existing
/// split sum: `(f0·dfg.x + dfg.y)·refl·energy_comp`, the Karis `env_brdf_approx` fit). The diffuse
/// keeps the linear hemisphere lerp along the normal (`hemi`), its sky/ground lanes filled from
/// the map (`E(±y)/π`, D20). AO stays decoupled: diffuse × `ao_final`, specular × `spec_ao`.
///
/// Parameter order is the HLSL signature's: `f0`, `energy_comp`, `diffuse_color` are the
/// `Surface` lanes; then `dfg`, `env_spec`, `sky`, `ground`, `hemi`, `ao_final`, `spec_ao`.
#[allow(clippy::too_many_arguments)] // one argument per HLSL parameter of the emitted signature
#[inline]
pub fn eval_pbr_ambient_env_body<S: FieldScalar<Vec3 = [S; 3]>>(
    f0: [S; 3],
    energy_comp: [S; 3],
    diffuse_color: [S; 3],
    dfg: [S; 2],
    env_spec: [S; 3],
    sky: [S; 3],
    ground: [S; 3],
    hemi: S,
    ao_final: S,
    spec_ao: S,
) -> [S; 3] {
    let lane = |i: usize| {
        let hemi_color = ground[i].lerp(sky[i], hemi);
        let spec = f0[i].mul(dfg[0]).add(dfg[1]).mul(env_spec[i]).mul(energy_comp[i]);
        let diff = diffuse_color[i].mul(hemi_color);
        diff.mul(ao_final).add(spec.mul(spec_ao))
    };
    [lane(0), lane(1), lane(2)]
}

// ---- Host-side leaves (the bake) ----------------------------------------------------------------

/// The perceptual roughness a prefiltered level `k` is convolved at: the inverse of
/// [`env_lod_body`], `1 − √(1 − k/l1)`. `env_lod(env_lod_inv(k)) == k` within 2 ulp in f32 for
/// `k = 0..=l1` (G3; measured, `env_lod_inv_round_trips`).
#[inline]
pub fn env_lod_inv_body<S: FieldScalar>(k: S, l1: S) -> S {
    let one = S::lit(1.0);
    one.sub(one.sub(k.div(l1)).sqrt())
}

/// The unit direction a signed square point `(u, v) ∈ [−1, 1]²` stands for — the inverse of
/// [`oct_env_signed_body`]: `p = (u, 1 − |u| − |v|, v)`, the lower hemisphere (`p.y < 0`)
/// unfolded to `((1 − |v|)·sgn u, p.y, (1 − |u|)·sgn v)`, normalized.
#[inline]
pub fn oct_env_decode_body<S: FieldScalar<Vec3 = [S; 3]>>(u: S, v: S) -> [S; 3] {
    let zero = S::lit(0.0);
    let one = S::lit(1.0);
    let y = one.sub(u.abs()).sub(v.abs());
    let su = S::select(u.ge(zero), one, S::lit(-1.0));
    let sv = S::select(v.ge(zero), one, S::lit(-1.0));
    let lower = y.lt(zero);
    let x = S::select(lower, one.sub(v.abs()).mul(su), u);
    let z = S::select(lower, one.sub(u.abs()).mul(sv), v);
    let p = [x, y, z];
    let inv = one.div(scalar::v_dot(p, p).sqrt());
    scalar::v_scale(p, inv)
}

/// The signed square coordinate of texel index `i`'s centre along one axis of a bordered level of
/// side `tile`: interior texels `1..=tile − 2` cover `[−1, 1]`, so the centre is
/// `2·(i − ½)/(tile − 2) − 1`; a border index lands just outside the square.
#[inline]
#[must_use]
pub fn oct_env_texel_signed(i: u32, tile: u32) -> f32 {
    (i as f32 - 0.5) / (tile - 2) as f32 * 2.0 - 1.0
}

/// The direction an interior texel `(x, y)` of a bordered level of side `tile` holds (its centre
/// decoded by [`oct_env_decode_body`]).
#[inline]
#[must_use]
pub fn oct_env_texel_dir(x: u32, y: u32, tile: u32) -> [f32; 3] {
    oct_env_decode_body::<f32>(oct_env_texel_signed(x, tile), oct_env_texel_signed(y, tile))
}

/// The interior texel whose value a texel `(x, y)` of a bordered level of side `tile` carries:
/// itself if interior; for an EDGE border texel its reflection about the crossed edge with the
/// tangential index mirrored (the octahedral square folds each edge onto itself, reversed); for a
/// CORNER the diagonally opposite interior corner (the two reflections composed). The DDGI rule
/// (`ddgi_oct_border_host_oracle.rs`); the opposite-edge copy is the defect class it refuted.
#[must_use]
pub const fn oct_env_border_source(x: u32, y: u32, tile: u32) -> (u32, u32) {
    let last = tile - 1;
    let (mut sx, mut sy) = (x, y);
    if x == 0 {
        sx = 1;
        sy = last - sy;
    } else if x == last {
        sx = tile - 2;
        sy = last - sy;
    }
    // The second reflection tests the index the first one produced: a corner's x-reflection
    // lands it on the far y edge, and only that composed pair is the diagonal corner.
    if sy == 0 {
        sy = 1;
        sx = last - sx;
    } else if sy == last {
        sy = tile - 2;
        sx = last - sx;
    }
    (sx, sy)
}

/// `π/2` in f32.
const FRAC_PI_2_F32: f32 = core::f32::consts::FRAC_PI_2;
/// `π` in f32.
const PI_F32: f32 = core::f32::consts::PI;
/// `√2` in f32 — the mantissa fold point of [`det_log2`].
const SQRT_2_F32: f32 = core::f32::consts::SQRT_2;
/// `log2(e)` in f32.
const LOG2_E_F32: f32 = core::f32::consts::LOG2_E;

/// The odd Taylor coefficients of `sin θ` past `θ` (θ³ … θ⁹) on `[0, π/4]`.
const SIN_COEFFS: [f32; 4] = [-1.0 / 6.0, 1.0 / 120.0, -1.0 / 5040.0, 1.0 / 362_880.0];
/// The even Taylor coefficients of `cos θ` past `1` (θ² … θ¹⁰) on `[0, π/4]`.
const COS_COEFFS: [f32; 5] = [-0.5, 1.0 / 24.0, -1.0 / 720.0, 1.0 / 40_320.0, -1.0 / 3_628_800.0];

/// `(sin 2πt, cos 2πt)` for `t ∈ [0, 1)` from IEEE operations only (|err| ≤ 2⁻²¹, gated): the
/// quadrant `q = ⌊4t⌋` and the in-quadrant fraction `r = 4t − q` are exact (a power-of-two scale
/// and an integer subtraction), `r > ½` folds to `1 − r` (exact by Sterbenz) with sin/cos
/// swapped, and the remaining `θ = r·π/2 ∈ [0, π/4]` runs fixed Taylor polynomials.
#[must_use]
pub fn det_sincos_turns(t: f32) -> (f32, f32) {
    debug_assert!((0.0..1.0).contains(&t), "invariant: det_sincos_turns takes t in [0, 1) (got {t})");
    let x4 = t * 4.0;
    let q = x4 as i32;
    let r = x4 - q as f32;
    let fold = r > 0.5;
    let r = if fold { 1.0 - r } else { r };
    let th = r * FRAC_PI_2_F32;
    let t2 = th * th;
    let s = th * (1.0 + t2 * (SIN_COEFFS[0] + t2 * (SIN_COEFFS[1] + t2 * (SIN_COEFFS[2] + t2 * SIN_COEFFS[3]))));
    let c = 1.0
        + t2 * (COS_COEFFS[0] + t2 * (COS_COEFFS[1] + t2 * (COS_COEFFS[2] + t2 * (COS_COEFFS[3] + t2 * COS_COEFFS[4]))));
    let (s, c) = if fold { (c, s) } else { (s, c) };
    match q & 3 {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// `log2 x` for a positive finite `x` from IEEE operations only (|err| ≤ 2⁻²⁰ plus the f32
/// rounding of the result, gated — above `|log2 x| = 8` the result's own spacing exceeds 2⁻²⁰): the
/// exponent from the bits, the mantissa `m ∈ [1, 2)` folded to `[√½, √2]` (an exact halving), and
/// `ln m = 2·atanh z` with `z = (m − 1)/(m + 1)` (`m − 1` exact by Sterbenz) by its odd series to
/// `z¹¹`. A subnormal is pre-scaled by 2²³; zero or a negative returns −149 (below any lod the
/// prefilter clamps to), a non-finite input 128.
#[must_use]
pub fn det_log2(x: f32) -> f32 {
    if x.is_nan() || x <= 0.0 {
        return -149.0;
    }
    if x.is_infinite() {
        return 128.0;
    }
    let (x, bias) = if x < f32::MIN_POSITIVE { (x * 8_388_608.0, 23) } else { (x, 0) };
    let bits = x.to_bits();
    let mut e = ((bits >> 23) & 0xFF) as i32 - 127 - bias;
    let mut m = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    if m > SQRT_2_F32 {
        m *= 0.5;
        e += 1;
    }
    let z = (m - 1.0) / (m + 1.0);
    let z2 = z * z;
    let series = 1.0 + z2 * (1.0 / 3.0 + z2 * (1.0 / 5.0 + z2 * (1.0 / 7.0 + z2 * (1.0 / 9.0 + z2 * (1.0 / 11.0)))));
    e as f32 + 2.0 * z * series * LOG2_E_F32
}

/// The `i`-th of `n` Hammersley points: `(i/n, radical_inverse₂(i))`, the radical inverse exact
/// as the top 24 reversed bits scaled by 2⁻²⁴ (so it never rounds up to 1).
#[inline]
#[must_use]
pub fn hammersley(i: u32, n: u32) -> (f32, f32) {
    (i as f32 / n as f32, (i.reverse_bits() >> 8) as f32 * (1.0 / 16_777_216.0))
}

/// The GGX / Trowbridge-Reitz NDF `D(NoH) = α² / (π·(NoH²·(α² − 1) + 1)²)`.
#[inline]
#[must_use]
pub fn d_ggx(noh: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let t = noh * noh * (a2 - 1.0) + 1.0;
    a2 / (PI_F32 * t * t)
}

/// The `i`-th of `n` GGX importance samples at `alpha` (= pR²): the half vector in the tangent
/// frame (`z` = the normal) and `D(NoH)`. `cos²θ = (1 − ξ₂)/(1 + (α² − 1)·ξ₂)` (Karis 2013),
/// `φ = 2π·ξ₁` through [`det_sincos_turns`]. With `N = V = R`, the pdf of the reflected direction
/// is `D/4` (`D·NoH/(4·VoH)` and `NoH = VoH`).
#[must_use]
pub fn ggx_prefilter_sample(i: u32, n: u32, alpha: f32) -> ([f32; 3], f32) {
    let (u1, u2) = hammersley(i, n);
    let (sp, cp) = det_sincos_turns(u1);
    let a2 = alpha * alpha;
    let cos2 = (1.0 - u2) / (1.0 + (a2 - 1.0) * u2);
    // `FieldScalar::sqrt`, not the inherent `f32::sqrt`: the Eval path stays `no_std`-clean.
    let ct = FieldScalar::sqrt(cos2);
    let st = FieldScalar::sqrt((1.0 - cos2).max(0.0));
    ([st * cp, st * sp, ct], d_ggx(ct, alpha))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The f32 spacing at `x` (one ulp), at least the spacing at 1.
    fn ulp(x: f32) -> f32 {
        let x = x.abs().max(1.0);
        f32::from_bits(x.to_bits() + 1) - x
    }

    fn normalize64(v: [f64; 3]) -> [f64; 3] {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    }

    /// A deterministic unit direction from an index (a fixed hash → the uniform sphere; f64 trig
    /// is fine in a test).
    fn hashed_dir(i: u32) -> [f32; 3] {
        let mut h = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
        h ^= h >> 15;
        h = h.wrapping_mul(0x2C1B_3C6D);
        h ^= h >> 12;
        let g = h.wrapping_mul(0x297A_2D39) ^ (h >> 16);
        let u = f64::from(h) / f64::from(u32::MAX);
        let v = f64::from(g) / f64::from(u32::MAX);
        let z = 2.0 * u - 1.0;
        let r = (1.0 - z * z).max(0.0).sqrt();
        let phi = std::f64::consts::TAU * v;
        [(r * phi.cos()) as f32, z as f32, (r * phi.sin()) as f32]
    }

    /// G3: the bake's inverse and the shader's map are one mapping. Measured: ≤ 2 ulp at every
    /// integer level of the 7-level chain (k = 2 and 3 land 2 ulp high in f32 under any natural
    /// spelling of `l1·pr·(2 − pr)`); pR = 1 reads exactly the last level.
    #[test]
    fn env_lod_inv_round_trips() {
        // The 7-level chain the studio bakes (256² … 4²).
        let levels = 7u32;
        let l1 = (levels - 1) as f32;
        for k in 0..levels {
            let k = k as f32;
            let pr = env_lod_inv_body::<f32>(k, l1);
            let back = env_lod_body::<f32>(pr, l1);
            assert!((back - k).abs() <= 2.0 * ulp(k), "env_lod(env_lod_inv({k})) = {back}");
        }
        assert_eq!(env_lod_body::<f32>(1.0, l1), l1, "pR 1 reads the last level");
        assert_eq!(env_lod_body::<f32>(0.0, l1), 0.0, "pR 0 reads the mirror level");
        // A linear inverse (`k / l1`) is the mutation: it is far from the quadratic map.
        assert!((env_lod_body::<f32>(1.0 / l1, l1) - 1.0).abs() > 0.5);
    }

    #[test]
    fn env_lod_is_monotone_over_roughness() {
        let mut prev = -1.0f32;
        for i in 0..=4096 {
            let lod = env_lod_body::<f32>(i as f32 / 4096.0, 6.0);
            assert!(lod >= prev, "env_lod decreases at pR {}", i as f32 / 4096.0);
            prev = lod;
        }
    }

    /// The direction→UV map inverts the texel→direction map at every interior texel centre of
    /// every level side 4..=256: the centre `(x + ½, y + ½)` comes back within 2e-3 texel. A UV map
    /// without the `(N − 2)/N` interior scale is off by up to a texel at the edges.
    #[test]
    fn oct_env_uv_inverts_the_texel_dir() {
        for shift in 2..=8 {
            let n = 1u32 << shift;
            let mut worst = 0f32;
            for y in 1..n - 1 {
                for x in 1..n - 1 {
                    let uv = oct_env_uv_body::<f32>(oct_env_texel_dir(x, y, n), n as f32);
                    let ex = (uv[0] * n as f32 - (x as f32 + 0.5)).abs();
                    let ey = (uv[1] * n as f32 - (y as f32 + 0.5)).abs();
                    worst = worst.max(ex).max(ey);
                }
            }
            assert!(worst <= 2e-3, "tile {n}: a texel centre comes back {worst} texel off");
        }
    }

    /// Every direction lands in the interior texels `[1, N − 1]` of a bordered level, so a
    /// bilinear footprint never reads past the border.
    #[test]
    fn oct_env_uv_maps_into_the_interior() {
        for n in [4u32, 16, 256] {
            for i in 0..20_000 {
                let uv = oct_env_uv_body::<f32>(hashed_dir(i), n as f32);
                for c in uv {
                    let texel = c * n as f32;
                    assert!((1.0 - 1e-4..=(n - 1) as f32 + 1e-4).contains(&texel), "tile {n}: texel coord {texel}");
                }
            }
            for axis in [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]] {
                let uv = oct_env_uv_body::<f32>(axis, n as f32);
                for c in uv {
                    let texel = c * n as f32;
                    assert!((1.0..=(n - 1) as f32).contains(&texel), "tile {n}: axis {axis:?} at {texel}");
                }
            }
        }
    }

    /// The octahedral continuation of a signed coordinate pair just outside `[−1, 1]²` (the DDGI
    /// oracle's `reflect_into_square`).
    fn reflect_into_square(sx: f32, sy: f32) -> (f32, f32) {
        let (mut sx, mut sy) = (sx, sy);
        if sx > 1.0 {
            sx = 2.0 - sx;
            sy = -sy;
        } else if sx < -1.0 {
            sx = -2.0 - sx;
            sy = -sy;
        }
        if sy > 1.0 {
            sy = 2.0 - sy;
            sx = -sx;
        } else if sy < -1.0 {
            sy = -2.0 - sy;
            sx = -sx;
        }
        (sx, sy)
    }

    /// Every border texel names the interior texel that holds its octahedral continuation: the
    /// border centre's signed coordinate, reflected into the square, decodes to the same direction
    /// as the named source texel. Corners name the diagonally opposite interior corner. The
    /// opposite-edge copy (`230585d0`'s defect class) fails every edge run.
    #[test]
    fn oct_env_border_source_is_the_octahedral_continuation() {
        for n in [4u32, 8, 32, 256] {
            for y in 0..n {
                for x in 0..n {
                    let (sx, sy) = oct_env_border_source(x, y, n);
                    assert!((1..n - 1).contains(&sx) && (1..n - 1).contains(&sy), "tile {n}: ({x},{y}) -> ({sx},{sy})");
                    let interior = (1..n - 1).contains(&x) && (1..n - 1).contains(&y);
                    if interior {
                        assert_eq!((sx, sy), (x, y));
                        continue;
                    }
                    let (rx, ry) = reflect_into_square(oct_env_texel_signed(x, n), oct_env_texel_signed(y, n));
                    let want = oct_env_decode_body::<f32>(rx, ry);
                    let got = oct_env_texel_dir(sx, sy, n);
                    let dot = want[0] * got[0] + want[1] * got[1] + want[2] * got[2];
                    assert!(dot > 1.0 - 1e-5, "tile {n}: border ({x},{y}) holds ({sx},{sy}), not its continuation (dot {dot})");
                }
            }
            let l = n - 2;
            assert_eq!(oct_env_border_source(0, 0, n), (l, l));
            assert_eq!(oct_env_border_source(n - 1, 0, n), (1, l));
            assert_eq!(oct_env_border_source(0, n - 1, n), (l, 1));
            assert_eq!(oct_env_border_source(n - 1, n - 1, n), (1, 1));
        }
    }

    fn spec_dominant_dir_f64(n: [f32; 3], r: [f32; 3], pr: f32) -> [f64; 3] {
        let pr = f64::from(pr);
        let f = (1.0 - pr) * ((1.0 - pr).sqrt() + pr);
        let d = [0, 1, 2].map(|i| f64::from(n[i]) + (f64::from(r[i]) - f64::from(n[i])) * f);
        normalize64(d)
    }

    /// The dominant direction equals the f64 formula within 2⁻²⁰ per component over normals and
    /// reflection vectors in the normal's hemisphere; pR 0 gives R and pR 1 gives n.
    #[test]
    fn spec_dominant_dir_matches_the_f64_formula() {
        let bound = 2f64.powi(-20);
        let mut checked = 0;
        for i in 0..4000 {
            let n = hashed_dir(i);
            let r = hashed_dir(i + 100_000);
            let nr = n[0] * r[0] + n[1] * r[1] + n[2] * r[2];
            if nr < 0.1 {
                continue;
            }
            for step in 0..=10 {
                let pr = step as f32 / 10.0;
                let got = spec_dominant_dir_body::<f32>(n, r, pr);
                let want = spec_dominant_dir_f64(n, r, pr);
                for c in 0..3 {
                    assert!((f64::from(got[c]) - want[c]).abs() <= bound, "n {n:?} r {r:?} pR {pr}: {got:?} vs {want:?}");
                }
            }
            let at0 = spec_dominant_dir_body::<f32>(n, r, 0.0);
            let at1 = spec_dominant_dir_body::<f32>(n, r, 1.0);
            for c in 0..3 {
                assert!((at0[c] - r[c]).abs() <= 1e-6, "pR 0 is R");
                assert!((at1[c] - n[c]).abs() <= 1e-6, "pR 1 is n");
            }
            checked += 1;
        }
        assert!(checked > 1000, "anti-vacuity: {checked} pairs");
    }

    /// The yaw is a rotation about +y: identity at yaw 0 (exactly), length-preserving, inverted by
    /// `-s`, and right-handed: at yaw 90° a feature at map +x is seen at world −z.
    #[test]
    fn env_yaw_rotate_is_a_rotation_about_up() {
        for i in 0..2000 {
            let d = hashed_dir(i);
            assert_eq!(env_yaw_rotate_body::<f32>(d, 1.0, 0.0), d);
            let yaw = i as f64 * 0.0137;
            let (c, s) = (yaw.cos() as f32, yaw.sin() as f32);
            let r = env_yaw_rotate_body::<f32>(d, c, s);
            let len = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5);
            assert_eq!(r[1], d[1], "the up component is untouched");
            let back = env_yaw_rotate_body::<f32>(r, c, -s);
            for k in 0..3 {
                assert!((back[k] - d[k]).abs() < 1e-5);
            }
        }
        assert_eq!(env_yaw_rotate_body::<f32>([0.0, 0.0, -1.0], 0.0, 1.0), [1.0, 0.0, 0.0], "yaw 90°: world −z looks up map +x");
    }

    /// `eval_pbr_ambient_hemi` (`pbr_lighting.hlsli`) transcribed op for op, with its reflected
    /// colour passed in — the goldens' `ground + (sky − ground)·h` lerp spelling.
    #[allow(clippy::too_many_arguments)]
    fn hemi_combine(
        f0: [f32; 3],
        ec: [f32; 3],
        diffuse: [f32; 3],
        dfg: [f32; 2],
        refl_color: [f32; 3],
        sky: [f32; 3],
        ground: [f32; 3],
        hemi: f32,
        ao_final: f32,
        spec_ao: f32,
    ) -> [f32; 3] {
        [0, 1, 2].map(|c| {
            let hemi_c = ground[c] + (sky[c] - ground[c]) * hemi;
            let spec_ambient = (f0[c] * dfg[0] + dfg[1]) * refl_color[c] * ec[c];
            let diff_ambient = diffuse[c] * hemi_c;
            diff_ambient * ao_final + spec_ambient * spec_ao
        })
    }

    /// The environment arm IS the hemisphere combine with `refl_color := env_spec`: bit-exact in
    /// f32 (same operations, same order), and the AO split is not swapped.
    #[test]
    fn eval_pbr_ambient_env_is_the_hemi_combine_with_env_spec() {
        for i in 0..5000u32 {
            let a = hashed_dir(i);
            let b = hashed_dir(i + 7_000);
            let c = hashed_dir(i + 14_000);
            let pos = |v: [f32; 3]| v.map(|x| x.abs() + 0.01);
            let f0 = pos(a);
            let ec = [1.0 + a[0].abs(), 1.0 + a[1].abs(), 1.0 + a[2].abs()];
            let diffuse = pos(b);
            let dfg = [b[0].abs(), c[1].abs() * 0.1];
            let env = [c[0].abs() * 3.0, c[1].abs() * 2.0, c[2].abs()];
            let sky = pos(c);
            let ground = pos([a[2], b[1], c[0]]);
            let hemi = (a[0] * 0.5 + 0.5).clamp(0.0, 1.0);
            let ao = (b[2] * 0.5 + 0.5).clamp(0.0, 1.0);
            let sao = (c[2] * 0.5 + 0.5).clamp(0.0, 1.0);
            let got = eval_pbr_ambient_env_body::<f32>(f0, ec, diffuse, dfg, env, sky, ground, hemi, ao, sao);
            let want = hemi_combine(f0, ec, diffuse, dfg, env, sky, ground, hemi, ao, sao);
            assert_eq!(got.map(f32::to_bits), want.map(f32::to_bits), "case {i}");
            if (ao - sao).abs() > 0.1 {
                let swapped = hemi_combine(f0, ec, diffuse, dfg, env, sky, ground, hemi, sao, ao);
                assert_ne!(got, swapped, "case {i}: the AO split must not be symmetric");
            }
        }
    }

    /// |err| ≤ 2⁻²¹ against f64 over a dense grid of `[0, 1)` turns, quadrant seams included.
    #[test]
    fn det_sincos_turns_error_bound() {
        let bound = 2f64.powi(-21);
        let mut worst = 0f64;
        for i in 0..(1u32 << 18) {
            let t = i as f32 / (1u32 << 18) as f32;
            let (s, c) = det_sincos_turns(t);
            let phi = std::f64::consts::TAU * f64::from(t);
            worst = worst.max((f64::from(s) - phi.sin()).abs()).max((f64::from(c) - phi.cos()).abs());
        }
        for q in 0..4 {
            for t in [q as f32 * 0.25, f32::from_bits((q as f32 * 0.25 + 0.125).to_bits()), 0.999_999_9] {
                if t < 1.0 {
                    let (s, c) = det_sincos_turns(t);
                    let phi = std::f64::consts::TAU * f64::from(t);
                    worst = worst.max((f64::from(s) - phi.sin()).abs()).max((f64::from(c) - phi.cos()).abs());
                }
            }
        }
        assert!(worst <= bound, "det_sincos_turns worst error {worst:e} > 2^-21");
    }

    /// |err| ≤ 2⁻²⁰ plus one ulp of the f32 result against f64 over positive normals and subnormals:
    /// the f32 result cannot resolve 2⁻²⁰ once `|log2 x| > 8` (a plain 2⁻²⁰ bound measured a
    /// 7.7e-6 worst error over this sweep, where `|log2 x|` reaches 133 and one ulp is 2⁻¹⁶).
    #[test]
    fn det_log2_error_bound() {
        // The worst error in units of the per-point bound, and where it sits.
        let mut worst = (0f64, 0f32);
        let mut check = |x: f32| {
            let got = f64::from(det_log2(x));
            let want = f64::from(x).log2();
            let bound = 2f64.powi(-20) + f64::from(ulp(want as f32));
            let r = (got - want).abs() / bound;
            if r > worst.0 {
                worst = (r, x);
            }
        };
        let mut x = 1.0e-40f32;
        while x < 1.0e30 {
            check(x);
            x *= 1.000_37;
        }
        for bits in (0x3F80_0000u32..0x4000_0000).step_by(97) {
            check(f32::from_bits(bits));
        }
        println!("det_log2: worst error {:.3}x its bound at x = {:e}", worst.0, worst.1);
        assert!(worst.0 <= 1.0, "det_log2 worst error is {:.3}x its bound (2^-20 + one f32 ulp) at x = {:e}", worst.0, worst.1);
        assert!(det_log2(0.0) <= -149.0 && det_log2(-1.0) <= -149.0, "non-positive inputs fall below every lod");
    }

    /// The radical inverse is exact: bit `j` of `i` weighs `2^-(j+1)`, kept to 24 bits.
    #[test]
    fn hammersley_radical_inverse_is_exact() {
        for i in 0..(1u32 << 16) {
            let mut want = 0f64;
            for j in 0..24 {
                if (i >> j) & 1 == 1 {
                    want += 2f64.powi(-(j + 1));
                }
            }
            let (u, v) = hammersley(i, 1 << 16);
            assert_eq!(f64::from(v), want, "i {i}");
            assert_eq!(u, i as f32 / 65_536.0);
        }
    }

    /// The GGX half-vector samples follow the GGX CDF: `P(cosθ ≥ c) = (1 − c²)/(1 + (α² − 1)c²)`.
    /// 16 bins of cosθ over 4096 Hammersley samples at α = 0.3 each hold their expected count
    /// within 3σ; every H is in the upper hemisphere and every D is non-negative. An α → α² slip
    /// in the sampler moves the histogram by tens of σ.
    #[test]
    fn ggx_prefilter_sample_follows_the_ggx_cdf() {
        let n = 4096u32;
        for alpha in [0.3f32, 0.6] {
            let a2 = f64::from(alpha * alpha);
            let cdf = |c: f64| (1.0 - c * c) / (1.0 + (a2 - 1.0) * c * c);
            let mut bins = [0u32; 16];
            for i in 0..n {
                let (h, d) = ggx_prefilter_sample(i, n, alpha);
                assert!(h[2] >= 0.0, "H below the horizon");
                assert!(d >= 0.0);
                let len = h[0] * h[0] + h[1] * h[1] + h[2] * h[2];
                assert!((len - 1.0).abs() < 1e-5, "H is unit");
                let b = ((h[2] * 16.0) as usize).min(15);
                bins[b] += 1;
            }
            for (b, &count) in bins.iter().enumerate() {
                let lo = b as f64 / 16.0;
                let hi = (b + 1) as f64 / 16.0;
                let expected = f64::from(n) * (cdf(lo) - cdf(hi));
                let sigma = expected.max(1.0).sqrt();
                assert!(
                    (f64::from(count) - expected).abs() <= 3.0 * sigma + 1.0,
                    "alpha {alpha}: bin {b} holds {count}, the GGX CDF expects {expected:.1}"
                );
            }
        }
    }
}
