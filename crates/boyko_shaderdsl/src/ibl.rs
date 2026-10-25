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

use crate::pixfmt::{Rounding, pack_b10g11r11};
use crate::scalar::{self, FieldScalar};

// ---- The SKY-row contract (D-P2d) ----------------------------------------------------------------

/// The SKY row's kind-word flag that marks it the ENVIRONMENT row (bit 24: above the P6 R1
/// `casts_sdf_shadow` bit 16 and the point/spot slot field, bits 17..22). Every consumer masks the
/// kind with `LIGHT_KIND_MASK` (0xFFFF), so a flagged row stays a SKY row to every dispatch;
/// only the REFL producers test the flag. `boyko_render::light::LIGHT_FLAG_ENV` is const-asserted
/// equal.
pub const LIGHT_FLAG_ENV: u32 = 1 << 24;

// ---- The asset (D-P2a) ---------------------------------------------------------------------------

/// The 8-byte magic every baked environment blob opens with.
pub const ENV_BLOB_MAGIC: [u8; 8] = *b"BKENVO01";
/// The blob header's size in bytes (the chain follows it, level-major).
pub const ENV_HEADER_BYTES: usize = 64;
/// The chain's texel format, `VK_FORMAT_B10G11R11_UFLOAT_PACK32`.
pub const ENV_FORMAT_B10G11R11: u32 = 122;
/// The studio chain's level-0 side.
pub const STUDIO_TILE: u32 = 256;
/// The studio chain's level count (256² … 4²).
pub const STUDIO_LEVELS: u32 = 7;
/// The side of the unbordered f32 octahedral source map the prefilter samples.
pub const STUDIO_SOURCE_TILE: u32 = 512;
/// The Hammersley sample count per prefiltered level `k = 1..=6` (level 0 is the supersampled
/// mirror level; HDRP ships 21–89 FIS samples per mip, cut D-P2a).
pub const STUDIO_FIS_SAMPLES: [u32; 6] = [64, 96, 128, 192, 256, 256];
/// The level-0 supersampling grid side (4 × 4 = 16 stratified samples per texel).
pub const STUDIO_SUPERSAMPLE: u32 = 4;

/// The byte size of a `levels`-level B10G11R11 chain of level-0 side `tile` (level `k` is
/// `max(tile >> k, 1)²` texels of 4 bytes).
#[must_use]
pub const fn env_chain_bytes(tile: u32, levels: u32) -> usize {
    let mut k = 0;
    let mut total = 0usize;
    while k < levels {
        let side = tile >> k;
        let side = if side == 0 { 1 } else { side };
        total += (side as usize) * (side as usize) * 4;
        k += 1;
    }
    total
}

/// The studio chain's byte size: 349,504.
pub const STUDIO_CHAIN_BYTES: usize = env_chain_bytes(STUDIO_TILE, STUDIO_LEVELS);
/// The studio blob's byte size, header included: 349,568 (the design's "350 KB").
pub const STUDIO_BLOB_BYTES: usize = ENV_HEADER_BYTES + STUDIO_CHAIN_BYTES;

const _: () = assert!(STUDIO_CHAIN_BYTES == 349_504);
const _: () = assert!(STUDIO_BLOB_BYTES == 349_568);
const _: () = assert!(STUDIO_FIS_SAMPLES.len() == STUDIO_LEVELS as usize - 1);

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

// ---- The procedural studio (D-P2g) ---------------------------------------------------------------

/// The studio box's lower corner (world units, capture point at the origin).
const ROOM_MIN: [f32; 3] = [-6.0, -2.0, -6.0];
/// The studio box's upper corner.
const ROOM_MAX: [f32; 3] = [6.0, 6.0, 6.0];
/// The floor's base radiance (neutral grey).
const ROOM_FLOOR_L: f32 = 0.02;
/// The ceiling's base radiance.
const ROOM_CEILING_L: f32 = 0.08;
/// The walls' base radiance at the floor line; it rises linearly to twice this at the ceiling.
const ROOM_WALL_L: f32 = 0.04;
/// The panels' warm-white tint.
const PANEL_TINT: [f32; 3] = [1.00, 0.97, 0.92];

/// One emissive rectangle on a box face: `axis` is the face normal's axis and `at` its plane;
/// `(a_axis, a)` and `(b_axis, b)` its extent on the other two axes; `l` its radiance.
struct Panel {
    axis: usize,
    at: f32,
    a_axis: usize,
    a: (f32, f32),
    b_axis: usize,
    b: (f32, f32),
    l: f32,
}

/// The five panels (cut D-P2g's table): top softbox, key strip (left), fill (right), rim line
/// (back, the horizon cue), bounce card (front). A look value, tuned at the owner-look sign-off
/// by editing this table and re-baking (cut §7 Q8).
const STUDIO_PANELS: [Panel; 5] = [
    Panel { axis: 1, at: 6.0, a_axis: 0, a: (-2.5, 2.5), b_axis: 2, b: (-1.5, 1.5), l: 4.0 },
    Panel { axis: 0, at: -6.0, a_axis: 1, a: (-0.5, 4.5), b_axis: 2, b: (0.8, 2.0), l: 8.0 },
    Panel { axis: 0, at: 6.0, a_axis: 1, a: (0.0, 3.0), b_axis: 2, b: (-2.5, 0.5), l: 2.0 },
    Panel { axis: 2, at: -6.0, a_axis: 0, a: (-4.0, 4.0), b_axis: 1, b: (3.6, 4.1), l: 6.0 },
    Panel { axis: 2, at: 6.0, a_axis: 0, a: (-5.0, -1.5), b_axis: 1, b: (-1.5, 0.5), l: 1.0 },
];

/// The studio's UNNORMALIZED radiance seen from the origin along `dir` (any non-zero length): the
/// box face the ray leaves through, its base radiance (floor 0.02; walls `0.04 + 0.04·(y + 2)/8`;
/// ceiling 0.08), replaced by a panel's `L × tint` inside a panel. Sun-less: the analytic
/// `DirectionalLight` stays the key.
#[must_use]
pub fn studio_radiance(dir: [f32; 3]) -> [f32; 3] {
    let mut t_exit = f32::INFINITY;
    let mut face = 0usize;
    for (axis, &d) in dir.iter().enumerate() {
        if d != 0.0 {
            let plane = if d > 0.0 { ROOM_MAX[axis] } else { ROOM_MIN[axis] };
            let t = plane / d;
            if t < t_exit {
                t_exit = t;
                face = axis;
            }
        }
    }
    let p = [dir[0] * t_exit, dir[1] * t_exit, dir[2] * t_exit];
    let plane = if dir[face] > 0.0 { ROOM_MAX[face] } else { ROOM_MIN[face] };
    for panel in &STUDIO_PANELS {
        if panel.axis == face
            && panel.at == plane
            && p[panel.a_axis] >= panel.a.0
            && p[panel.a_axis] <= panel.a.1
            && p[panel.b_axis] >= panel.b.0
            && p[panel.b_axis] <= panel.b.1
        {
            return [panel.l * PANEL_TINT[0], panel.l * PANEL_TINT[1], panel.l * PANEL_TINT[2]];
        }
    }
    let base = if face == 1 {
        if dir[1] > 0.0 { ROOM_CEILING_L } else { ROOM_FLOOR_L }
    } else {
        let height = (p[1] - ROOM_MIN[1]) / (ROOM_MAX[1] - ROOM_MIN[1]);
        ROOM_WALL_L + ROOM_WALL_L * height
    };
    [base, base, base]
}

// ---- The bake (D-P2a) ------------------------------------------------------------------------------

/// The shape of one bake: the chain's level-0 side and level count, the unbordered source map's
/// side, the FIS sample count per level `1..levels`, the level-0 supersampling side, and whether
/// the radiance is normalized to `E(+y) = 1`.
#[derive(Clone, Copy, Debug)]
pub struct EnvBakeParams<'a> {
    /// The chain's level-0 side (a power of two ≥ 4).
    pub tile: u32,
    /// The chain's level count (level `levels − 1` keeps a side ≥ 4).
    pub levels: u32,
    /// The unbordered f32 source map's side (a power of two).
    pub source_tile: u32,
    /// The FIS sample count of each level `1..levels` (`len == levels − 1`).
    pub fis_samples: &'a [u32],
    /// The level-0 supersampling grid side.
    pub supersample: u32,
    /// Scale the radiance so the luminance of `E(+y)` is 1 (the studio's normalization rule:
    /// intensity 1 ⇔ an up-facing surface receives unit irradiance). Off for the furnace and the
    /// lobe gates, which bake sources whose scale is the measurement.
    pub normalize: bool,
}

/// The studio's committed bake shape.
pub const STUDIO_BAKE: EnvBakeParams<'static> = EnvBakeParams {
    tile: STUDIO_TILE,
    levels: STUDIO_LEVELS,
    source_tile: STUDIO_SOURCE_TILE,
    fis_samples: &STUDIO_FIS_SAMPLES,
    supersample: STUDIO_SUPERSAMPLE,
    normalize: true,
};

/// What a bake measured on its (normalized) source map — printed by the bake bin, gated in part
/// by `ibl_studio_numbers` and carried into the blob header (`k`, the two lanes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvBakeStats {
    /// The normalization factor applied to the radiance (1 when `normalize` is off).
    pub k: f32,
    /// The irradiance on a surface facing `+x, −x, +y, −y, +z, −z` (after `k`), per channel.
    pub irradiance: [[f32; 3]; 6],
    /// The SKY row's upper-hemisphere lane, `E(+y)/π` (D20's fill rule).
    pub sky_lane: [f32; 3],
    /// The SKY row's lower-hemisphere lane, `E(−y)/π`.
    pub ground_lane: [f32; 3],
    /// The sum of every source texel's solid angle (4π for an exact Jacobian).
    pub solid_angle_sum: f64,
}

/// The six axis normals [`EnvBakeStats::irradiance`] is indexed by.
pub const IRRADIANCE_NORMALS: [[f32; 3]; 6] =
    [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];

/// The f32 count of the source scratch for a source map of side `source_tile`: RGB over every box
/// mip down to 1².
#[must_use]
pub const fn env_source_scratch_len(source_tile: u32) -> usize {
    let mut side = source_tile;
    let mut total = 0usize;
    while side > 0 {
        total += (side as usize) * (side as usize) * 3;
        side >>= 1;
    }
    total
}

/// The number of words a chain of `levels` levels of level-0 side `tile` holds.
#[must_use]
pub const fn env_chain_words(tile: u32, levels: u32) -> usize {
    env_chain_bytes(tile, levels) / 4
}

/// The unbordered octahedral source map with its box mips, RGB f32, level-major in the caller's
/// scratch. Fetched bilinearly with the octahedral edge continuation, trilinearly across mips.
pub struct OctMipSource<'a> {
    data: &'a [f32],
    tile: u32,
    levels: u32,
}

impl OctMipSource<'_> {
    /// The f32 offset of level `level` in the scratch.
    fn level_offset(&self, level: u32) -> usize {
        let mut off = 0usize;
        let mut side = self.tile;
        for _ in 0..level {
            off += (side as usize) * (side as usize) * 3;
            side >>= 1;
        }
        off
    }

    /// Texel `(x, y)` of level `level` (side `m`), indices up to one texel outside the square
    /// folded back by the octahedral continuation (an edge reflects with the tangential index
    /// mirrored; a corner composes both).
    fn texel(&self, level: u32, x: i32, y: i32) -> [f32; 3] {
        let m = (self.tile >> level) as i32;
        let (mut x, mut y) = (x, y);
        if x < 0 {
            x = -1 - x;
            y = m - 1 - y;
        } else if x >= m {
            x = 2 * m - 1 - x;
            y = m - 1 - y;
        }
        if y < 0 {
            y = -1 - y;
            x = m - 1 - x;
        } else if y >= m {
            y = 2 * m - 1 - y;
            x = m - 1 - x;
        }
        let i = self.level_offset(level) + ((y as usize) * (m as usize) + x as usize) * 3;
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }

    /// The bilinear fetch of level `level` along `dir`.
    fn bilinear(&self, level: u32, dir: [f32; 3]) -> [f32; 3] {
        let m = (self.tile >> level) as f32;
        let s = oct_env_signed_body::<f32>(dir);
        let fx = (s[0] * 0.5 + 0.5) * m - 0.5;
        let fy = (s[1] * 0.5 + 0.5) * m - 0.5;
        // `fx + 1 >= 0.5`, so the cast truncates a non-negative value: ⌊fx + 1⌋ − 1 = ⌊fx⌋.
        let x0 = (fx + 1.0) as i32 - 1;
        let y0 = (fy + 1.0) as i32 - 1;
        let wx = fx - x0 as f32;
        let wy = fy - y0 as f32;
        let c00 = self.texel(level, x0, y0);
        let c10 = self.texel(level, x0 + 1, y0);
        let c01 = self.texel(level, x0, y0 + 1);
        let c11 = self.texel(level, x0 + 1, y0 + 1);
        let mut out = [0.0f32; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let top = c00[c] + (c10[c] - c00[c]) * wx;
            let bottom = c01[c] + (c11[c] - c01[c]) * wx;
            *o = top + (bottom - top) * wy;
        }
        out
    }

    /// The trilinear fetch at a fractional source `lod` (clamped to the chain).
    fn trilinear(&self, dir: [f32; 3], lod: f32) -> [f32; 3] {
        let top = (self.levels - 1) as f32;
        let lod = lod.max(0.0).min(top);
        let l0 = lod as u32;
        let l1 = (l0 + 1).min(self.levels - 1);
        let f = lod - l0 as f32;
        let a = self.bilinear(l0, dir);
        let b = self.bilinear(l1, dir);
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
    }
}

/// The direction an UNBORDERED source texel `(x, y)` of side `m` holds.
fn source_texel_dir(x: u32, y: u32, m: u32) -> [f32; 3] {
    let u = (x as f32 + 0.5) / m as f32 * 2.0 - 1.0;
    let v = (y as f32 + 0.5) / m as f32 * 2.0 - 1.0;
    oct_env_decode_body::<f32>(u, v)
}

/// The solid angle of an UNBORDERED source texel of side `m` holding direction `dir`: the exact
/// octahedral Jacobian `dω = (2/m)²·|ω|₁³` (for `p = (u, v, 1 − |u| − |v|)`,
/// `|∂p/∂u × ∂p/∂v · p| = 1`, and `|p|₂ = 1/|ω|₁`).
fn source_texel_solid_angle(dir: [f32; 3], m: u32) -> f64 {
    let l1 = f64::from(dir[0].abs()) + f64::from(dir[1].abs()) + f64::from(dir[2].abs());
    let h = 2.0 / f64::from(m);
    h * h * l1 * l1 * l1
}

/// Fills `scratch` with the source map of `radiance` (one sample per texel centre) and its box
/// mips, measures the irradiance on the six axis normals and the solid-angle sum, applies the
/// `E(+y) = 1` normalization when `normalize`, and returns the view plus the stats.
///
/// # Panics
/// If `scratch` is shorter than [`env_source_scratch_len`]`(source_tile)`.
pub fn build_oct_source<'a, F: Fn([f32; 3]) -> [f32; 3]>(
    radiance: &F,
    source_tile: u32,
    normalize: bool,
    scratch: &'a mut [f32],
) -> (OctMipSource<'a>, EnvBakeStats) {
    let len = env_source_scratch_len(source_tile);
    assert!(scratch.len() >= len, "invariant: the source scratch holds {len} f32 (got {})", scratch.len());
    let m = source_tile;
    let mut irr = [[0f64; 3]; 6];
    let mut omega_sum = 0f64;
    for y in 0..m {
        for x in 0..m {
            let dir = source_texel_dir(x, y, m);
            let c = radiance(dir);
            let i = ((y * m + x) * 3) as usize;
            scratch[i..i + 3].copy_from_slice(&c);
            let d_omega = source_texel_solid_angle(dir, m);
            omega_sum += d_omega;
            for (n, e) in IRRADIANCE_NORMALS.iter().zip(irr.iter_mut()) {
                let cos = dir[0] * n[0] + dir[1] * n[1] + dir[2] * n[2];
                if cos > 0.0 {
                    for ch in 0..3 {
                        e[ch] += f64::from(c[ch]) * f64::from(cos) * d_omega;
                    }
                }
            }
        }
    }
    let e_up = irr[2];
    let luminance = 0.2126 * e_up[0] + 0.7152 * e_up[1] + 0.0722 * e_up[2];
    let k = if normalize { (1.0 / luminance) as f32 } else { 1.0 };
    if normalize {
        for v in &mut scratch[..(m * m * 3) as usize] {
            *v *= k;
        }
    }
    // Box mips: level j + 1 texel = the mean of its 2×2 block on level j.
    let mut levels = 1u32;
    let mut src_off = 0usize;
    let mut side = m;
    while side > 1 {
        let half = side / 2;
        let dst_off = src_off + (side as usize) * (side as usize) * 3;
        for y in 0..half {
            for x in 0..half {
                for ch in 0..3 {
                    let at = |xx: u32, yy: u32| scratch[src_off + ((yy * side + xx) * 3) as usize + ch];
                    let sum = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                    scratch[dst_off + ((y * half + x) * 3) as usize + ch] = sum * 0.25;
                }
            }
        }
        src_off = dst_off;
        side = half;
        levels += 1;
    }
    let kf = f64::from(k);
    let mut irradiance = [[0f32; 3]; 6];
    for (out, e) in irradiance.iter_mut().zip(irr.iter()) {
        for ch in 0..3 {
            out[ch] = (e[ch] * kf) as f32;
        }
    }
    let inv_pi = 1.0 / PI_F32;
    let stats = EnvBakeStats {
        k,
        irradiance,
        sky_lane: [irradiance[2][0] * inv_pi, irradiance[2][1] * inv_pi, irradiance[2][2] * inv_pi],
        ground_lane: [irradiance[3][0] * inv_pi, irradiance[3][1] * inv_pi, irradiance[3][2] * inv_pi],
        solid_angle_sum: omega_sum,
    };
    (OctMipSource { data: &scratch[..len], tile: m, levels }, stats)
}

/// A right-handed orthonormal basis `(t, b)` around the unit `n` (Duff et al. 2017, branchless
/// apart from the sign; IEEE operations only).
fn tangent_frame(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let sign = if n[2] >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (sign + n[2]);
    let b = n[0] * n[1] * a;
    ([1.0 + sign * n[0] * n[0] * a, sign * b, -sign * n[0]], [b, sign + n[1] * n[1] * a, -n[1]])
}

/// The prefiltered radiance of level `k` (of `l1 + 1`) along the unit `n`: GGX at the perceptual
/// roughness [`env_lod_inv_body`]`(k, l1)` — the ONE place a level is mapped to its roughness —
/// importance-sampled with `samples` Hammersley points under `N = V = R` (Karis 2013's split-sum
/// prefilter: `Σ L(l)·NoL / Σ NoL`), each sample read from the source at the Křivánek FIS lod
/// `½·log2(Ωs/Ωp) + 1` (`Ωs = 1/(samples·pdf)`, `pdf = D/4`, `Ωp` the source's mean texel solid
/// angle `4π/m²`).
#[must_use]
pub fn prefilter_texel(src: &OctMipSource<'_>, n: [f32; 3], k: u32, l1: u32, samples: u32) -> [f32; 3] {
    let pr = env_lod_inv_body::<f32>(k as f32, l1 as f32);
    let alpha = pr * pr;
    let (t, b) = tangent_frame(n);
    let omega_p = 4.0 * PI_F32 / (src.tile as f32 * src.tile as f32);
    let mut sum = [0f64; 3];
    let mut weight = 0f64;
    for i in 0..samples {
        let (h_t, d) = ggx_prefilter_sample(i, samples, alpha);
        let h = [
            t[0] * h_t[0] + b[0] * h_t[1] + n[0] * h_t[2],
            t[1] * h_t[0] + b[1] * h_t[1] + n[1] * h_t[2],
            t[2] * h_t[0] + b[2] * h_t[1] + n[2] * h_t[2],
        ];
        let noh = h_t[2];
        let l = [2.0 * noh * h[0] - n[0], 2.0 * noh * h[1] - n[1], 2.0 * noh * h[2] - n[2]];
        let nol = n[0] * l[0] + n[1] * l[1] + n[2] * l[2];
        if nol > 0.0 {
            let pdf = d * 0.25;
            let omega_s = 1.0 / (samples as f32 * pdf);
            let lod = 0.5 * det_log2(omega_s / omega_p) + 1.0;
            let c = src.trilinear(l, lod);
            for ch in 0..3 {
                sum[ch] += f64::from(c[ch]) * f64::from(nol);
            }
            weight += f64::from(nol);
        }
    }
    if weight > 0.0 {
        [(sum[0] / weight) as f32, (sum[1] / weight) as f32, (sum[2] / weight) as f32]
    } else {
        src.trilinear(n, 0.0)
    }
}

/// The supersampled level-0 radiance of interior texel `(x, y)` of side `tile`: the mean of
/// `ss × ss` stratified sub-texel directions through `radiance`, scaled by `k`.
fn supersample_texel<F: Fn([f32; 3]) -> [f32; 3]>(radiance: &F, x: u32, y: u32, tile: u32, ss: u32, k: f32) -> [f32; 3] {
    let extent = (tile - 2) as f32;
    let mut acc = [0f64; 3];
    for sy in 0..ss {
        for sx in 0..ss {
            let fx = (x - 1) as f32 + (sx as f32 + 0.5) / ss as f32;
            let fy = (y - 1) as f32 + (sy as f32 + 0.5) / ss as f32;
            let dir = oct_env_decode_body::<f32>(fx / extent * 2.0 - 1.0, fy / extent * 2.0 - 1.0);
            let c = radiance(dir);
            for ch in 0..3 {
                acc[ch] += f64::from(c[ch] * k);
            }
        }
    }
    let n = f64::from(ss * ss);
    [(acc[0] / n) as f32, (acc[1] / n) as f32, (acc[2] / n) as f32]
}

/// Bakes `radiance` into a bordered octahedral B10G11R11 chain of `params`' shape (`chain`, level
/// major, row major, [`env_chain_words`] words), using `scratch` ([`env_source_scratch_len`] f32)
/// for the source map: level 0 supersampled from the analytic radiance, levels `1..levels`
/// prefiltered from the source ([`prefilter_texel`]), every border texel copied from its
/// [`oct_env_border_source`] after packing (`Rounding::NearestEven`).
///
/// # Panics
/// If `chain` or `scratch` is too short, `fis_samples.len() != levels − 1`, or a level's side
/// would drop under 4.
pub fn bake_env_into<F: Fn([f32; 3]) -> [f32; 3]>(
    radiance: &F,
    params: &EnvBakeParams<'_>,
    chain: &mut [u32],
    scratch: &mut [f32],
) -> EnvBakeStats {
    let words = env_chain_words(params.tile, params.levels);
    assert!(chain.len() >= words, "invariant: the chain holds {words} words (got {})", chain.len());
    assert_eq!(params.fis_samples.len() as u32, params.levels - 1, "invariant: one FIS count per level past 0");
    assert!(params.tile >> (params.levels - 1) >= 4, "invariant: every level keeps a side >= 4");
    let (src, stats) = build_oct_source(radiance, params.source_tile, params.normalize, scratch);
    let l1 = params.levels - 1;
    let mut off = 0usize;
    for k in 0..params.levels {
        let n = params.tile >> k;
        let level = &mut chain[off..off + (n * n) as usize];
        for y in 1..n - 1 {
            for x in 1..n - 1 {
                let c = if k == 0 {
                    supersample_texel(radiance, x, y, n, params.supersample, stats.k)
                } else {
                    prefilter_texel(&src, oct_env_texel_dir(x, y, n), k, l1, params.fis_samples[(k - 1) as usize])
                };
                level[(y * n + x) as usize] = pack_b10g11r11(c, Rounding::NearestEven);
            }
        }
        for y in 0..n {
            for x in 0..n {
                if x == 0 || y == 0 || x == n - 1 || y == n - 1 {
                    let (sx, sy) = oct_env_border_source(x, y, n);
                    level[(y * n + x) as usize] = level[(sy * n + sx) as usize];
                }
            }
        }
        off += (n * n) as usize;
    }
    stats
}

/// Bakes the procedural studio ([`studio_radiance`]) at [`STUDIO_BAKE`] into `chain`
/// ([`env_chain_words`]`(STUDIO_TILE, STUDIO_LEVELS)` words) with `scratch`
/// ([`env_source_scratch_len`]`(STUDIO_SOURCE_TILE)` f32).
pub fn bake_studio_into(chain: &mut [u32], scratch: &mut [f32]) -> EnvBakeStats {
    bake_env_into(&studio_radiance, &STUDIO_BAKE, chain, scratch)
}

/// Writes the 64-byte header and the chain (little-endian words) into `out`
/// (`ENV_HEADER_BYTES + 4·chain.len()` bytes). Header: magic `BKENVO01` @0, tile @8, levels @12,
/// format @16 ([`ENV_FORMAT_B10G11R11`]), flags @20 (0: sun-less), sky lane @24, ground lane @36,
/// `k` @48, chain bytes @52, zero @56..64.
///
/// # Panics
/// If `out` is not exactly the blob's size.
pub fn write_env_blob(params: &EnvBakeParams<'_>, stats: &EnvBakeStats, chain: &[u32], out: &mut [u8]) {
    let chain_bytes = chain.len() * 4;
    assert_eq!(out.len(), ENV_HEADER_BYTES + chain_bytes, "invariant: the blob is the header plus the chain");
    out[..ENV_HEADER_BYTES].fill(0);
    out[0..8].copy_from_slice(&ENV_BLOB_MAGIC);
    out[8..12].copy_from_slice(&params.tile.to_le_bytes());
    out[12..16].copy_from_slice(&params.levels.to_le_bytes());
    out[16..20].copy_from_slice(&ENV_FORMAT_B10G11R11.to_le_bytes());
    out[20..24].copy_from_slice(&0u32.to_le_bytes());
    for c in 0..3 {
        out[24 + 4 * c..28 + 4 * c].copy_from_slice(&stats.sky_lane[c].to_le_bytes());
        out[36 + 4 * c..40 + 4 * c].copy_from_slice(&stats.ground_lane[c].to_le_bytes());
    }
    out[48..52].copy_from_slice(&stats.k.to_le_bytes());
    out[52..56].copy_from_slice(&(chain_bytes as u32).to_le_bytes());
    for (dst, w) in out[ENV_HEADER_BYTES..].as_chunks_mut::<4>().0.iter_mut().zip(chain) {
        *dst = w.to_le_bytes();
    }
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

    // ---- The bake ------------------------------------------------------------------------------

    /// The identifiers that name a platform-libm transcendental (or the eDSL's own trait surface
    /// over one) in any spelling the bake could reach: method form (`x.sin()`), path form
    /// (`f32::sin(x)`, `C::cos(t)`, `libm::sinf(x)`), the `cf.rs` helpers (`exp`, `ln`, `rsqrt`),
    /// and FMA contraction (`mul_add`). Matched as whole identifiers directly followed by `(`.
    const LIBM_IDENTS: [&str; 33] = [
        "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "sin_cos", "exp", "exp2",
        "exp_m1", "ln", "ln_1p", "log", "log2", "log10", "powf", "powi", "pow", "cbrt", "hypot", "mul_add",
        "rsqrt", "sinf", "cosf", "expf", "logf", "log2f", "exp2f", "to_radians",
    ];

    /// The eDSL control-axis types; the bake must not reach them (its leaves are over
    /// `FieldScalar`, which has no transcendental).
    const CF_IDENTS: [&str; 2] = ["Cf", "EvalCf"];

    /// `src` with `//` comments and string / char literals blanked (so a doc sentence or a
    /// message naming `sin` is not a call).
    fn strip_comments_and_strings(src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        for line in src.lines() {
            let mut in_str = false;
            let mut prev = '\0';
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                if !in_str && c == '/' && chars.peek() == Some(&'/') {
                    break;
                }
                if c == '"' && prev != '\\' {
                    in_str = !in_str;
                    out.push(' ');
                } else if in_str {
                    out.push(' ');
                } else {
                    out.push(c);
                }
                prev = c;
            }
            out.push('\n');
        }
        out
    }

    /// Every `(line, ident)` in `code` where a forbidden identifier is called (`ident(`, spaces
    /// allowed) or, for the `Cf` names, named at all.
    fn libm_hits(code: &str) -> Vec<(usize, String)> {
        let mut hits = Vec::new();
        for (no, line) in code.lines().enumerate() {
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c.is_ascii_alphabetic() || c == '_' {
                    let start = i;
                    while i < bytes.len() && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                    let ident = &line[start..i];
                    let mut j = i;
                    while j < bytes.len() && bytes[j] == b' ' {
                        j += 1;
                    }
                    let called = j < bytes.len() && bytes[j] == b'(';
                    if (called && LIBM_IDENTS.contains(&ident)) || CF_IDENTS.contains(&ident) {
                        hits.push((no + 1, ident.to_string()));
                    }
                } else {
                    i += 1;
                }
            }
        }
        hits
    }

    /// W2: the bake is libm-free in every spelling. This file's non-test code — every leaf the
    /// bake calls lives here (`scalar.rs`' `FieldScalar` has `sqrt` as its only non-arithmetic
    /// op, so the generic leaves cannot reach one at all) — names no transcendental call and no
    /// `Cf` type. A libm call would make the committed blob host-dependent: MSVC would bless it
    /// and only Linux CI would red, after the merge.
    #[test]
    fn ibl_bake_is_libm_free() {
        let src = include_str!("ibl.rs");
        let end = src.find("#[cfg(test)]\nmod tests").or_else(|| src.find("#[cfg(test)]\r\nmod tests"));
        let end = end.expect("invariant: ibl.rs ends with its test module");
        let code = strip_comments_and_strings(&src[..end]);
        assert!(code.contains("fn bake_env_into") && code.contains("fn det_sincos_turns"), "anti-vacuity: the scan sees the bake");
        let hits = libm_hits(&code);
        assert!(hits.is_empty(), "the bake reaches a platform transcendental (host-dependent blob): {hits:?}");
    }

    /// The census can fail, in every spelling: method, path, trait (`C::cos`), libm, `mul_add`
    /// and a `Cf` bound are flagged; a comment, a string and the deterministic leaves are not.
    #[test]
    fn libm_census_flags_every_spelling_fixture() {
        let fixture = "let a = x.sin();\nlet b = C::cos(t);\nlet c = f32::powf(x, y);\nlet d = libm::sinf(x);\n\
                       let e = a.mul_add(b, c);\nfn f<C: Cf>() {}\n// x.sin() in a comment\nlet g = \"cos(\";\n\
                       let h = det_sincos_turns(t);\nlet i = det_log2(x);\nlet j = exp (x);\n";
        let hits = libm_hits(&strip_comments_and_strings(fixture));
        let lines: Vec<usize> = hits.iter().map(|h| h.0).collect();
        assert_eq!(lines, [1, 2, 3, 4, 5, 6, 11], "flagged lines: {hits:?}");
    }

    /// The radiance of a narrow cap of half-angle 0.5° around +y: the lobe gate's delta source.
    fn cap_radiance(d: [f32; 3]) -> [f32; 3] {
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if d[1] / len >= 0.999_961_9 { [1.0; 3] } else { [0.0; 3] }
    }

    /// The perceptual roughness the READ side assigns to level `k`: `env_lod_body` inverted by
    /// bisection (independent of `env_lod_inv_body`, which the bake uses).
    fn read_side_roughness(k: f32, l1: f32) -> f64 {
        let (mut lo, mut hi) = (0f32, 1f32);
        for _ in 0..64 {
            let mid = 0.5 * (lo + hi);
            if env_lod_body::<f32>(mid, l1) < k {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        f64::from(0.5 * (lo + hi))
    }

    /// The RMS angular radius `√(∫ v θ² sinθ dθ / ∫ v sinθ dθ)` of an azimuthally symmetric
    /// response sampled at `thetas`.
    fn rms_radius(thetas: &[f64], v: &[f64]) -> f64 {
        let (mut num, mut den) = (0f64, 0f64);
        for (t, w) in thetas.iter().zip(v) {
            num += w * t * t * t.sin();
            den += w * t.sin();
        }
        (num / den).sqrt()
    }

    /// W3: each level is convolved at the roughness the read side assigns it. The bake's own
    /// per-texel kernel (`prefilter_texel`, the one place a level maps to a roughness) responds to
    /// a 0.5° cap at +y with a lobe whose RMS radius matches GGX at the read side's roughness for
    /// that level (`D(cos θ/2)·cos θ`, the split-sum prefilter's response under `N = V = R`) within
    /// [0.8, 1.35]. Measured: 1.03–1.245 (levels 6 and 3) — the baked lobe is wider, mostly by
    /// Křivánek's `+1` FIS lod bias, which reads each sample one source mip blurrier than its own
    /// solid angle. A bake that convolves level `k` at `k/L1`, or at α where pR is meant, measured
    /// 1.68–3.38 on levels 1–4 (the Godot #69514 class): every other gate re-reads its own blob.
    #[test]
    fn ibl_bake_level_roughness_matches_env_lod() {
        let src_tile = 512;
        let mut scratch = vec![0f32; env_source_scratch_len(src_tile)];
        let (src, _) = build_oct_source(&cap_radiance, src_tile, false, &mut scratch);
        let l1 = STUDIO_LEVELS - 1;
        let mut report = Vec::new();
        for k in 1..=l1 {
            let pr = read_side_roughness(k as f32, l1 as f32);
            let alpha = pr * pr;
            let theta_max = (0.08 + 10.0 * alpha).min(std::f64::consts::FRAC_PI_2);
            let steps = 600;
            let thetas: Vec<f64> = (0..steps).map(|i| (i as f64 + 0.5) / steps as f64 * theta_max).collect();
            let analytic: Vec<f64> = thetas
                .iter()
                .map(|&t| f64::from(d_ggx((t * 0.5).cos() as f32, alpha as f32)) * t.cos().max(0.0))
                .collect();
            let measured: Vec<f64> = thetas
                .iter()
                .map(|&t| {
                    let dir = [t.sin() as f32, t.cos() as f32, 0.0];
                    f64::from(prefilter_texel(&src, dir, k, l1, STUDIO_FIS_SAMPLES[(k - 1) as usize])[0])
                })
                .collect();
            let want = rms_radius(&thetas, &analytic);
            let got = rms_radius(&thetas, &measured);
            report.push((k, pr, want.to_degrees(), got.to_degrees(), got / want));
        }
        for &(k, pr, want, got, ratio) in &report {
            println!("level {k}: pR {pr:.4}  GGX RMS radius {want:.3} deg  baked {got:.3} deg  ratio {ratio:.3}");
        }
        for &(k, _, _, _, ratio) in &report {
            assert!((0.8..=1.35).contains(&ratio), "level {k}: the baked lobe is {ratio:.3}x the read side's GGX lobe");
        }
    }

    /// The furnace: a constant-1 source baked through the same path leaves every texel of every
    /// level within one B10G11R11 step of 1.0 (a missing `Σ NoL` normalization fails it).
    #[test]
    fn ibl_bake_furnace() {
        let params = EnvBakeParams { tile: 32, levels: 4, source_tile: 64, fis_samples: &[32, 32, 32], supersample: 2, normalize: false };
        let mut chain = vec![0u32; env_chain_words(params.tile, params.levels)];
        let mut scratch = vec![0f32; env_source_scratch_len(params.source_tile)];
        let stats = bake_env_into(&|_| [1.0; 3], &params, &mut chain, &mut scratch);
        assert_eq!(stats.k, 1.0);
        for (i, &w) in chain.iter().enumerate() {
            let rgb = crate::pixfmt::unpack_b10g11r11(w);
            for c in rgb {
                assert!((c - 1.0).abs() <= 1.0 / 32.0, "texel {i}: {rgb:?} is not within one step of 1");
            }
        }
    }
}
