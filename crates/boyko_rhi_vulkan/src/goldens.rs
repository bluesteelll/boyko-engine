//! CPU golden reference oracles — host mirrors of the marcher / lighting / SSAO /
//! cluster-cull shader math, split out of [`crate::compute`] so they compile ONLY
//! for the crate's own `#[cfg(test)]` builds and for callers that opt in via the
//! `goldens` cargo feature (integration tests in `tests/*.rs` and any dependent
//! crate that mirrors the GPU output bit-for-bit against these).
//!
//! Every `golden_*` function here is the SINGLE host definition of a shader
//! contract: the GPU readback is diffed against it (bit-exact, or within a small
//! DXC `mad`/`fma` tolerance) as the test oracle that substitutes for Miri on the
//! raw-FFI path. The production ([`crate::compute`]) surface — the committed
//! SPIR-V blobs, the pipeline constructors, and the push-constant PODs — no longer
//! carries this host-mirror weight in the shipped backend.
//!
//! Nothing in this module is on any hot path; it is pure host arithmetic driven by
//! the test harnesses.

use boyko_sdf_math::brick::{
    self, BRICK_VOXELS, PointerGrid, brick_cubic_hit, dist_to_brick_exit, fill_brick,
};
use boyko_sdf_math::{
    MAX_SDF_EDITS, SDF_GRAD_H, SDF_IMG_H, SDF_IMG_W, SdfEdit, SdfEditField, edit_distance,
    sdf_edit_list, sdf_edit_list_normal, v_dot, v_len, v_normalize, v_sub,
};

use crate::compute::{
    ALPHA_MARGIN, AO_FALLOFF, AO_STEP, AO_STRENGTH, BRICK_CLASS_EMPTY_OUTSIDE, BrickLevelParams,
    CompositeCamera, DEFAULT_LIGHT_DIR, EPS_COARSE, FIELD_LIPSCHITZ_L, GOLDEN_ATLAS_SLOT_MASK, GOLDEN_ATLAS_SLOT_SHIFT,
    GOLDEN_LIGHT_FLAG_CASTS_SHADOW, GOLDEN_LIGHT_KIND_DIRECTIONAL, GOLDEN_LIGHT_KIND_MASK, GOLDEN_LIGHT_KIND_POINT, GOLDEN_LIGHT_KIND_SKY, GOLDEN_LIGHT_KIND_SPOT,
    GOLDEN_SLOT_NONE, GOLDEN_SPOT_COS_OUTER_MAX, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, M2_GRID_DIM, M2_REFINE_ITERS, M2_REFINE_RELAX, M4GridParams, MAX_IT_COARSE, MAX_SDF_SHADOW_CASTERS_PER_PIXEL,
    MESH_COLOR, MESH_DEPTH_CLEAR, MESH_DEPTH_T_MAX, MESH_RASTER_ALBEDO, PBR_FAR, PBR_LIGHT_COLOR, PBR_LIGHT_DIR,
    PBR_SKY_DIFFUSE, PBR_SKY_SPEC, SDF_CAM_Z, SDF_EPS, SDF_HALF_EXTENT, SDF_MAX_IT,
    SDF_T_MAX, SHADOW_HIT_EPS, SHADOW_K, SHADOW_MINT, SHADOW_MINT_STEP, SHADOW_NDOTL_EPS,
    SHADOW_NORMAL_BIAS, SKY_SUN_EXPONENT, SSAO_ATROUS_H, SSAO_ATROUS_W_EPS, SSAO_BLUR_DEPTH_SIGMA, SSAO_BLUR_DEPTH_TOL, SSAO_BLUR_GRAD_CLAMP, SSAO_HILBERT_W, SSAO_R2_ALPHA1, SSAO_R2_ALPHA2, SSAO_RADIUS_PIX_MAX, SSAO_RADIUS_PIX_MIN,
    SSAO_ROT, SSAO_ROT_N, SSAO_VIEWT_BG, SUN_ENV_WEIGHT, SUN_KERNEL_EXPONENT_MAX, SUN_KERNEL_EXPONENT_MIN,
    SsaoParams, TILE_FLAG_EMPTY, TILE_SIZE, TileBound, composite_ray, depth_to_t,
    golden_f16_from_f32, pack_rgba, sdf_sphere,
};


/// The CPU golden for step 0c: `out[i] == i*2 + 1` (the `write_pattern` shader).
///
/// Exposed so the test (and any later golden harness) shares ONE definition of
/// the shader's contract rather than duplicating the arithmetic.
#[inline]
pub fn golden_write_pattern(i: u32) -> u32 {
    i.wrapping_mul(2).wrapping_add(1)
}

/// The CPU golden for the chained 0c→0d result: `(i*2 + 1) + 100` (the
/// `transform_add` shader applied on top of `write_pattern`).
#[inline]
pub fn golden_chained(i: u32) -> u32 {
    golden_write_pattern(i).wrapping_add(100)
}

const SDF_LIGHT_DIR: [f32; 3] = [0.0, 0.0, 1.0];

const SDF_BASE_COLOR: [f32; 3] = [0.8, 0.3, 0.2];

const SDF_AMBIENT: f32 = 0.1;

const SDF_BACKGROUND: [f32; 3] = [0.05, 0.05, 0.1];

/// The A1 host mirror of `sdf_soft_shadow`: a clamped-step Quilez BASIC cone-trace
/// (NO `sqrt` — minimal FP-parity surface) from the lit point `p` toward the
/// normalized light `l`, returning a soft visibility in `[0, 1]` (1 = fully lit,
/// 0 = fully occluded). Mirrors the shader within ±3/255 (consumer-side relaxable,
/// NOT bit-exact for the ON path). `field` is the FROZEN field gateway (the
/// edit-list `sdf_edit_list`); the min-track + Lipschitz-corrected step are
/// accumulated consumer-side. `n` is the surface normal, `l` the NORMALIZED light.
pub(crate) fn host_soft_shadow<F: Fn([f32; 3]) -> f32>(
    p: [f32; 3],
    n: [f32; 3],
    l: [f32; 3],
    field: &F,
) -> f32 {
    // Signed n·L: at/below the cutoff the surface faces away from the light — fully
    // shadowed, and the march would only graze the surface (acne). Replaces a
    // normal-offset bias on the march origin.
    if v_dot(n, l) <= SHADOW_NDOTL_EPS {
        return 0.0;
    }
    let mut res = 1.0_f32;
    let mut t = SHADOW_MINT;
    for _ in 0..SDF_MAX_IT {
        let q = [p[0] + l[0] * t, p[1] + l[1] * t, p[2] + l[2] * t];
        let d = field(q);
        res = res.min(SHADOW_K * d / t);
        if d < SHADOW_HIT_EPS {
            return 0.0;
        }
        // The `/L` Lipschitz correction on the STEP: without it the super-Lipschitz
        // smin leaks light through thin occluders. Floored at SHADOW_MINT_STEP so a
        // near-zero `d` cannot stall the march.
        t += (d / FIELD_LIPSCHITZ_L).max(SHADOW_MINT_STEP);
        if t > SDF_T_MAX {
            break;
        }
    }
    res.clamp(0.0, 1.0)
}

/// The P6 R1 `t_max`-RANGED host mirror of `sdf_soft_shadow_ranged` — IDENTICAL to
/// [`host_soft_shadow`] except the escape break bound is the runtime `t_max` (the per-caster
/// march range: the light DISTANCE for a punctual caster, `SDF_T_MAX` for an extra
/// directional) instead of the hardcoded `SDF_T_MAX`. The multi-light shadow term is
/// consumer-side (±2/255), not bit-exact. `field` is the FROZEN field gateway.
///
/// `pub` rather than `pub(crate)` because `tests/sdf_shadow_leaf_oracle.rs`'s layer 3a diffs this
/// mirror against the eDSL body that GENERATES the shipped HLSL, and an integration test is a
/// separate crate. The whole module is already `#[cfg(any(test, feature = "goldens"))]`, so this
/// widens no shipping surface.
pub fn host_soft_shadow_ranged<F: Fn([f32; 3]) -> f32>(
    p: [f32; 3],
    n: [f32; 3],
    l: [f32; 3],
    t_max: f32,
    field: &F,
) -> f32 {
    if v_dot(n, l) <= SHADOW_NDOTL_EPS {
        return 0.0;
    }
    let mut res = 1.0_f32;
    let mut t = SHADOW_MINT;
    for _ in 0..SDF_MAX_IT {
        let q = [p[0] + l[0] * t, p[1] + l[1] * t, p[2] + l[2] * t];
        let d = field(q);
        res = res.min(SHADOW_K * d / t);
        if d < SHADOW_HIT_EPS {
            return 0.0;
        }
        t += (d / FIELD_LIPSCHITZ_L).max(SHADOW_MINT_STEP);
        if t > t_max {
            break;
        }
    }
    res.clamp(0.0, 1.0)
}

/// The A2 host mirror of `sdf_ao`: a 5-tap ambient-occlusion estimate marching the
/// surface normal `n` from `p`, accumulating the `(h - d)` field-deficit weighted by
/// `AO_FALLOFF^i`, and returning an occlusion factor in `[0, 1]` (1 = unoccluded).
/// Mirrors the shader within ±3/255. `field` is the FROZEN field gateway.
///
/// `pub` for the same reason as [`host_soft_shadow_ranged`]: `tests/sdf_shadow_leaf_oracle.rs`'s
/// layer 3b is an integration test, i.e. a separate crate. The module is
/// `#[cfg(any(test, feature = "goldens"))]`, so no shipping build gains anything.
pub fn host_ao<F: Fn([f32; 3]) -> f32>(p: [f32; 3], n: [f32; 3], field: &F) -> f32 {
    let mut occ = 0.0_f32;
    for i in 1..=5u32 {
        let h = (i as f32) * AO_STEP;
        let q = [p[0] + n[0] * h, p[1] + n[1] * h, p[2] + n[2] * h];
        let d = field(q);
        occ += (h - d) * AO_FALLOFF.powi(i as i32);
    }
    (1.0 - AO_STRENGTH * occ).clamp(0.0, 1.0)
}

/// The single shading helper for every host golden (factored from the four inlined
/// `ndotl + ambient` Lambert sites). Computes the directional Lambert + ambient base
/// color, then — ONLY when `lighting_flags != 0` — multiplies in the A1 shadow and/or
/// A2 AO terms (the SAME gate the shader uses). With `lighting_flags == 0` the result
/// is the bare Lambert color, BYTE-IDENTICAL to the pre-A1/A2 inline arithmetic (the
/// 0%-gate): no extra multiply is performed (a structural `if`).
///
/// `base_color` is the surface albedo, `ambient` the ambient term, `p` the lit hit
/// point, `n` the surface normal, `light_dir` the (un-normalized) light direction,
/// and `field` the FROZEN field gateway the shadow/AO consumers call. The closure is
/// never invoked on the OFF path, so callers with no edit-list field may pass any
/// matching closure.
#[inline]
pub(crate) fn host_shade<F: Fn([f32; 3]) -> f32>(
    base_color: [f32; 3],
    ambient: f32,
    p: [f32; 3],
    n: [f32; 3],
    light_dir: [f32; 3],
    lighting_flags: u32,
    field: &F,
) -> [f32; 3] {
    let l = v_normalize(light_dir);
    let ndotl = v_dot(n, l).max(0.0);
    let base = [
        base_color[0] * ndotl + base_color[0] * ambient,
        base_color[1] * ndotl + base_color[1] * ambient,
        base_color[2] * ndotl + base_color[2] * ambient,
    ];
    if lighting_flags == 0 {
        // OFF path: byte-identical to today (NO extra multiply).
        return base;
    }
    let mut shadow = 1.0_f32;
    if lighting_flags & LIGHTING_FLAG_SHADOWS != 0 {
        // Normal-offset start bias: lift the march origin off the surface so grazing
        // (near-tangent) rays clear the curved surface instead of false-occluding.
        // MIRRORS the shader's `sdf_soft_shadow(p + n*SHADOW_NORMAL_BIAS, n, light)`.
        let pb = [
            p[0] + n[0] * SHADOW_NORMAL_BIAS,
            p[1] + n[1] * SHADOW_NORMAL_BIAS,
            p[2] + n[2] * SHADOW_NORMAL_BIAS,
        ];
        shadow = host_soft_shadow(pb, n, l, field);
    }
    let mut ao = 1.0_f32;
    if lighting_flags & LIGHTING_FLAG_AO != 0 {
        ao = host_ao(p, n, field);
    }
    [
        base[0] * shadow * ao,
        base[1] * shadow * ao,
        base[2] * shadow * ao,
    ]
}

/// Surface normal via central differences (the gradient of [`sdf_sphere`]),
/// mirroring the shader's `sdf_normal`.
#[inline]
pub(crate) fn sdf_normal(p: [f32; 3]) -> [f32; 3] {
    let h = SDF_GRAD_H;
    let n = [
        sdf_sphere([p[0] + h, p[1], p[2]]) - sdf_sphere([p[0] - h, p[1], p[2]]),
        sdf_sphere([p[0], p[1] + h, p[2]]) - sdf_sphere([p[0], p[1] - h, p[2]]),
        sdf_sphere([p[0], p[1], p[2] + h]) - sdf_sphere([p[0], p[1], p[2] - h]),
    ];
    v_normalize(n)
}

/// The CPU golden for one SDF pixel: reconstructs the orthographic ray for
/// `(px, py)`, sphere-traces the analytic field, lights the hit (Lambert +
/// ambient) or returns the background on a miss, and returns the packed
/// `0xAABBGGRR` color.
///
/// This is the single source of truth the rung-8 test asserts against: the
/// center pixel HITS (lit sphere color) and a corner pixel MISSES (background).
pub fn golden_sdf_pixel(px: u32, py: u32) -> u32 {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
    let rd = [0.0, 0.0, -1.0];

    let mut t = 0.0_f32;
    let mut hit = false;
    for _ in 0..SDF_MAX_IT {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_sphere(p);
        if d < SDF_EPS {
            hit = true;
            break;
        }
        t += d;
        if t > SDF_T_MAX {
            break;
        }
    }

    let color = if hit {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_normal(p);
        // The rung-8 sphere golden is always the OFF path (`lighting_flags == 0` ⇒ bare
        // Lambert, byte-identical); the field closure is never invoked.
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, SDF_LIGHT_DIR, 0, &sdf_sphere)
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

/// The CPU golden for one edit-list pixel: sphere-traces the folded edit-list
/// field, lights the hit (Lambert + ambient, the same scene constants as rung 8)
/// or returns the background on a miss, and returns the packed `0xAABBGGRR`
/// color. The rung-9 test diffs the GPU readback against this within the
/// `+/-2/255` per-channel tolerance.
pub fn golden_editlist_pixel(edits: &[SdfEdit], px: u32, py: u32) -> u32 {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
    let rd = [0.0, 0.0, -1.0];

    let mut t = 0.0_f32;
    let mut hit = false;
    for _ in 0..SDF_MAX_IT {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            break;
        }
        t += d;
        if t > SDF_T_MAX {
            break;
        }
    }

    let color = if hit {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        // The rung-9 edit-list golden is always the OFF path (`lighting_flags == 0` ⇒
        // bare Lambert, byte-identical); the field closure is never invoked.
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, SDF_LIGHT_DIR, 0, &|q| {
            sdf_edit_list(edits, q)
        })
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

/// The CPU golden for one composited pixel at the GOLDEN 64×64 ORTHO extent: a thin
/// wrapper over [`golden_composite_pixel_ex`] with `(SDF_IMG_W, SDF_IMG_H)` + ORTHO.
/// Bit-identical to the pre-P0a definition (same extent, same arithmetic), so the
/// rung-10 / window-present goldens are unchanged. See [`golden_composite_pixel_ex`]
/// for the per-pixel composite rules.
///
/// Returns the packed `0xAABBGGRR` color. This is the single source of truth the
/// rung-10 test diffs the GPU readback against (within `+/-2/255` per channel) and
/// that a future CPU physics evaluator can reuse for the same hybrid query.
#[inline]
pub fn golden_composite_pixel(edits: &[SdfEdit], mesh_depth: f32, px: u32, py: u32) -> u32 {
    golden_composite_pixel_ex(
        edits,
        mesh_depth,
        px,
        py,
        SDF_IMG_W,
        SDF_IMG_H,
        CompositeCamera::Ortho,
    )
}

/// The extent- and camera-aware CPU golden for one composited pixel (P0a). At
/// `(SDF_IMG_W, SDF_IMG_H)` + [`CompositeCamera::Ortho`] this is BIT-IDENTICAL to the
/// pre-P0a `golden_composite_pixel` (same `u`/`v`/`ro`/`rd` arithmetic), preserving
/// the rung-8..11 contract; with a runtime extent / [`CompositeCamera::Perspective`]
/// it mirrors the shader's P0a ray-gen so the host-vs-GPU agreement stays valid at
/// any resolution. Composites exactly as the shader:
///
/// - an SDF hit at `t_sdf < t_mesh` → the lit SDF surface color (Lambert + ambient);
/// - else if the mesh covered the pixel (`mesh_depth < 1.0`) → flat [`MESH_COLOR`];
/// - else → `SDF_BACKGROUND`.
///
/// The field eval (`sdf_edit_list` / `_normal`) is byte-identical to the ortho path;
/// only ray generation + the extent source change (the determinism boundary).
pub fn golden_composite_pixel_ex(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> u32 {
    // Render B1: the ω = 1.0 forwarder. At `omega == 1.0` the `_omega` variant's live
    // path is the frozen plain sphere-trace, so this stays BIT-IDENTICAL to the pre-B1
    // body and every existing caller is unchanged (the 0%-gate).
    golden_composite_pixel_ex_omega(edits, mesh_depth, px, py, img_w, img_h, camera, 1.0)
}

/// Render B1 — the over-relaxation-aware extent/camera golden. Mirrors the shader's
/// Keinert over-relaxation marcher EXACTLY: the `if omega > 1.0` gate, the
/// over-relaxed step `t += omega * d`, the sor-fail exact retreat (`t = safe_t` then a
/// permanent fall to plain), and the verbatim frozen else-arm `t += d`. At `omega == 1.0`
/// the live path is textually the frozen plain loop, so this is BIT-IDENTICAL to the
/// pre-B1 [`golden_composite_pixel_ex`] (the 0%-gate). `omega` is expected to already be
/// in `[1.0, 1.99]` (the host runtime clamp); higher values are unsound (the safeguard
/// holds only for `omega < 2`).
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_ex_omega(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
) -> u32 {
    golden_composite_pixel_ex_omega_lit(
        edits, mesh_depth, px, py, img_w, img_h, camera, omega, 0, DEFAULT_LIGHT_DIR,
    )
}

/// Render A1/A2 — the lighting-aware extent/camera/omega golden. Identical to
/// [`golden_composite_pixel_ex_omega`] but threads the `lighting_flags` + `light_dir`
/// the marcher push carries: on an SDF hit the lit color goes through [`host_shade`],
/// which multiplies in the A1 soft-shadow and/or A2 AO terms when the matching flag
/// bit is set (bit 0 = shadows, bit 1 = AO). With `lighting_flags == 0` this is
/// BYTE-IDENTICAL to [`golden_composite_pixel_ex_omega`] (the 0%-gate); the ON path
/// mirrors the shader within ±3/255 (consumer-side relaxable). `light_dir` is the
/// un-normalized directional-light direction; the field eval / march are untouched.
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_ex_omega_lit(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
) -> u32 {
    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    // A finite march bound only when the mesh covered the pixel; otherwise a value
    // larger than any `t` the march reaches (mirrors the shader's `1e30`).
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    let mut t = 0.0_f32;
    let t_seed = t; // the ORIGINAL seed (0.0 here) — the Candidate C re-march re-seeds from it
    let mut omega = omega; // [1.0, 1.99]; sor-fail latches it to 1.0 for the rest of the ray
    let mut hit = false;
    let mut safe_t = 0.0_f32; // probe param remembered for an exact retreat
    let mut sor_prev = 0.0_f32; // previous probe's d
    let mut sor_step_prev = 0.0_f32; // previous over-relaxed step length
    // BUG-B1-HOLE-3 (Candidate C): the EXHAUSTION flag. True iff the fast loop runs ALL
    // SDF_MAX_IT iterations with NO break — i.e. the ray neither converged, nor clearly
    // left the scene (`t > T_MAX`), nor hit the mesh (`t >= t_mesh`); it ran out of
    // budget mid-field. Starts `true`, cleared by EVERY in-loop break. Mirrors the shader.
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false; // mesh-occlusion termination — NOT budget exhaustion
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false; // converged — NOT budget exhaustion
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            // sor_fail: the over-step taken last iter overshot the previous unbounding
            // sphere (valid only for omega < 2 — spheres must overlap). Lipschitz-aware
            // (BUG-B1-HOLE-1): the guaranteed-empty radius at field value `f` is
            // `f / FIELD_LIPSCHITZ_L`, so the spheres cover the step iff
            // `sor_prev + d >= L * sor_step_prev`. Mirrors the shader exactly.
            //
            // The `it > 0` guard is LOAD-BEARING (do not remove): a sor-fail can only be
            // reached after at least one ACCEPTED over-relax step (it >= 1 ⟹ accepted >= 1),
            // which pre-pays the +1 retreat iteration in the budget proof.
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                // BUG-B1-HOLE-2: do NOT retreat to bare `safe_t` and re-probe (that re-evals
                // the field, costing +2 iters vs plain and overflowing the budget at the
                // MAX_IT cliff → a hole). RESUME the plain march one certified step past the
                // safe point: `safe_t` is the exact probe param, `sor_prev` the exact field
                // value there, so `safe_t + sor_prev` is precisely where a plain march lands
                // after probing safe_t — reusing the eval (no re-probe). One same-sign add
                // (both operands >= 0): no cancellation, unlike a `t - <correction>` form.
                // Net +1 iter vs plain, pre-paid by the >= 1 accepted over-step (it>0 guard).
                debug_assert!(it > 0, "B1 budget: a>=1 precondition");
                debug_assert!(sor_prev >= SDF_EPS); // safe-point field value >= EPS → retreat strictly advances
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                debug_assert!(t > safe_t, "B1 retreat must advance");
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d; // frozen plain arm — TEXTUALLY identical to the frozen loop
        }
        if t > SDF_T_MAX {
            exhausted = false; // clear-miss termination — NOT budget exhaustion
            break;
        }
    }

    // BUG-B1-HOLE-3 (Candidate C): the PROVABLY-hole-free fallback re-march, mirroring
    // the shader EXACTLY. The fast over-relaxed pass can fall BEHIND a plain march on a
    // non-monotone field (the `steps(omega) <= steps(1)` bound is genuinely violated and
    // unbounded), exhausting the budget mid-field on a ray the FROZEN plain marcher would
    // have hit. On `exhausted` (ran all SDF_MAX_IT with no break) RE-MARCH from the
    // ORIGINAL seed with a plain omega = 1.0 sphere-trace and use ITS result. This second
    // loop is the EXACT frozen marcher body (`t += d`), so any surface the frozen path
    // hits within MAX_IT it hits here too → B1's hit-set is identical to the frozen
    // hit-set, with NO dependence on a step-count bound. At omega == 1.0 the fast pass IS
    // the frozen plain loop, so on exhaustion this reproduces the identical frozen
    // (hit = false) result — the omega == 1.0 output is byte-unchanged (the 0%-gate).
    if exhausted {
        t = t_seed; // re-seed from the SAME original seed the fast pass used
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d; // frozen plain step
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, light_dir, lighting_flags, &|q| {
            sdf_edit_list(edits, q)
        })
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

/// Reads the pointer-grid cell containing world point `p`, returning `(class, cell_min)`
/// or `None` when `p` is OUTSIDE the bounded grid (the marcher then falls through to the
/// analytic field). Mirrors the shader's `brick_cell(p)` index + bounds check exactly.
#[inline]
pub(crate) fn host_brick_cell(grid: &PointerGrid, cells: &[u32], p: [f32; 3]) -> Option<(u32, [f32; 3])> {
    let rel = [
        (p[0] - grid.origin[0]) / grid.brick_world,
        (p[1] - grid.origin[1]) / grid.brick_world,
        (p[2] - grid.origin[2]) / grid.brick_world,
    ];
    // Outside the grid on any axis (incl. negative `rel`) → no cell. `floor` then a
    // signed range check; `rel < 0` is caught by the `>= dims` test after the cast only
    // if guarded — so test the float directly to avoid the wrap on a negative cast.
    if rel[0] < 0.0 || rel[1] < 0.0 || rel[2] < 0.0 {
        return None;
    }
    let ix = rel[0] as u32;
    let iy = rel[1] as u32;
    let iz = rel[2] as u32;
    if ix >= grid.dims[0] || iy >= grid.dims[1] || iz >= grid.dims[2] {
        return None;
    }
    let w = grid.dims[0];
    let h = grid.dims[1];
    let idx = (ix + iy * w + iz * w * h) as usize;
    debug_assert!(idx < cells.len(), "grid cell index in bounds");
    Some((cells[idx], grid.cell_min(ix, iy, iz)))
}

/// M1 — the empty-space-skip extent/camera/omega/lighting golden. Identical to
/// [`golden_composite_pixel_ex_omega_lit`] but the PRIMARY march runs the pointer-grid
/// empty skip when `brick_enabled == true`: an `EmptyOutside` cell at the march point
/// steps to the brick AABB exit ([`dist_to_brick_exit`], clamped to advance) instead of
/// folding the field; every other cell (and any point outside the bounded grid) folds the
/// EXACT analytic field. `grid` + `cells` are the [`build_pointer_grid`] bake the GPU
/// binds at binding 9 (the SAME origin/dims/brick_world the push carries).
///
/// With `brick_enabled == false` this delegates to [`golden_composite_pixel_ex_omega_lit`]
/// — BYTE-IDENTICAL to the pre-M1 golden (the 0%-gate). The re-march fallback, the
/// hit/normal, and the shade stay ANALYTIC (C1): the empty skip only accelerates EMPTY
/// traversal, so the hit `t` equals the pure-analytic hit `t` within `SDF_EPS` and the
/// composited color matches the analytic golden.
///
/// [`build_pointer_grid`]: boyko_sdf_math::brick::build_pointer_grid
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_brick(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
    brick_enabled: bool,
    grid: &PointerGrid,
    cells: &[u32],
) -> u32 {
    // The OFF path is byte-identical to the pre-M1 marcher (the 0%-gate). The grid is
    // never read; the march is the exact analytic sphere-trace.
    if !brick_enabled {
        return golden_composite_pixel_ex_omega_lit(
            edits, mesh_depth, px, py, img_w, img_h, camera, omega, lighting_flags, light_dir,
        );
    }

    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    let mut t = 0.0_f32;
    let t_seed = t;
    let mut omega = omega; // [1.0, 1.99]; sor-fail latches it to 1.0
    let mut hit = false;
    let mut safe_t = 0.0_f32;
    let mut sor_prev = 0.0_f32;
    let mut sor_step_prev = 0.0_f32;
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false; // mesh-occlusion termination
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];

        // M1 empty skip: an EmptyOutside cell at `p` has provably no surface within
        // band_half (conservative classifier), so step to the brick AABB exit and skip
        // the analytic fold. CONTINUE without touching `sdf`/the over-relax state — the
        // exit step is plain (no omega), so it cannot overshoot a surface (the next
        // brick is `Surface` if a surface is near). Sound by construction.
        //
        // EmptyInside / Surface (and an outside-grid `None`) fall THROUGH to the EXACT
        // analytic field below. EmptyInside is the start-inside case the analytic
        // negative-`d` handling already covers — a ray from outside reaches Surface first,
        // so a negative `sdf(p)` here means the seed began inside a solid; the analytic step
        // (which can be negative) is the consistent, unchanged behavior.
        if let Some((class, cell_min)) = host_brick_cell(grid, cells, p)
            && class == BRICK_CLASS_EMPTY_OUTSIDE
        {
            let exit = dist_to_brick_exit(p, rd, cell_min, grid.brick_world);
            t += exit;
            if t > SDF_T_MAX {
                exhausted = false; // clear-miss termination
                break;
            }
            continue; // skip the analytic fold this step
        }

        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false; // converged
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                debug_assert!(it > 0, "B1 budget: a>=1 precondition");
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d; // frozen plain arm
        }
        if t > SDF_T_MAX {
            exhausted = false; // clear-miss termination
            break;
        }
    }

    // The re-march fallback stays ANALYTIC (C1) — identical to the non-brick path. The
    // empty skip never reopens the B1 budget hole (its plain exit steps are bounded), so
    // `exhausted` here means the analytic field ran out of budget mid-field, exactly as
    // in the non-brick marcher; the frozen plain re-march resolves it.
    if exhausted {
        t = t_seed;
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d; // frozen plain step
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, light_dir, lighting_flags, &|q| {
            sdf_edit_list(edits, q)
        })
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

/// The world-space ray-AABB slab clip of brick `[cell_min, cell_min + brick_world]³` from `p` along
/// `rd`, returning `Some((t_enter, t_exit))` (world `t`, measured from `p`, `t_enter >= 0`) or `None`
/// on a miss. Mirrors the shader's `m2_brick_span` (the `tmin = 0` floor never marches behind the
/// current point). The brick edge `brick_world` is a PARAMETER (M4 Slice C): a clip-map level clips
/// against its `2^L`× larger cell; the level-0 caller passes [`M2_BRICK_WORLD`] (byte-identical M2).
#[inline]
pub(crate) fn brick_aabb_span_world(
    p: [f32; 3],
    rd: [f32; 3],
    cell_min: [f32; 3],
    brick_world: f32,
) -> Option<(f32, f32)> {
    let mut tmin = 0.0_f32; // never march behind the current march point
    let mut tmax = 1.0e30_f32;
    for a in 0..3 {
        let lo = cell_min[a];
        let hi = lo + brick_world;
        if rd[a].abs() <= 1.0e-20 {
            // Parallel to this slab: a miss only if the origin is outside it.
            if p[a] < lo || p[a] > hi {
                return None;
            }
            continue;
        }
        let inv = 1.0 / rd[a];
        let mut t1 = (lo - p[a]) * inv;
        let mut t2 = (hi - p[a]) * inv;
        if t1 > t2 {
            core::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
    }
    if tmax > tmin { Some((tmin, tmax)) } else { None }
}

/// The host mirror of the shader's `m2_surface_hit`: locates the M2 tile containing `p = ro + rd *
/// t_world`, bakes its brick ([`fill_brick`]), solves the JCGT cubic ([`brick_cubic_hit`]) for the
/// in-brick crossing, then VALIDATES the candidate analytically (the exact-CSG fallback). Returns
/// `Some(hit_t)` (world `t`, the accepted hit) or `None` (no crossing / the refine cleared it →
/// the caller falls through to the M1 analytic fold). `edits` is the authority the GPU baked the
/// atlas from (so the host bakes the SAME tile bit-for-bit).
///
/// A SIGNED, under-relaxed sphere-trace ([`M2_REFINE_ITERS`] steps, factor [`M2_REFINE_RELAX`])
/// refines the cubic candidate onto the EXACT field from EITHER side (an inside candidate from the
/// EPSILON_Q down-bias is pulled BACK to the surface, never committed deep — committing the raw
/// `cand_t` cratered the baked AO). ONLY a refine-CONVERGED candidate (`|d| < SDF_EPS`) is accepted;
/// a grazing silhouette point or a stalled hard crease falls to `None` → the analytic fold, which
/// resolves the pixel EXACTLY as the OFF path (this erased the BUG-M2-RIM silhouette ring). Mirrors
/// the shader's refine loop bit-for-bit.
pub(crate) fn host_m2_surface_hit(edits: &[SdfEdit], ro: [f32; 3], rd: [f32; 3], t_world: f32) -> Option<f32> {
    // The single-level M2 mirror delegates to the level-aware sibling at level-0 geometry
    // ([`BrickLevelParams::m2_near_field`]) — byte-identical to the pre-M4 hardcoded path.
    host_m2_surface_hit_at(edits, &BrickLevelParams::m2_near_field(), ro, rd, t_world)
}

/// The level-aware host mirror of the shader's `m2_surface_hit` (M4 Slice C): identical to
/// [`host_m2_surface_hit`] but the grid origin / brick world / voxel / band come from `geo` (the
/// clip-map level's [`BrickLevelParams`]) instead of the level-0 M2 consts. Locates the tile in THIS
/// level's `M2_GRID_DIM³` grid containing `p = ro + rd * t_world`, bakes it at the level's geometry,
/// solves the JCGT cubic, and validates analytically (the exact-CSG fallback, level-invariant). The
/// host `select_level` picks `geo`; this mirrors the shader's per-level branch-ladder arm.
pub(crate) fn host_m2_surface_hit_at(
    edits: &[SdfEdit],
    geo: &BrickLevelParams,
    ro: [f32; 3],
    rd: [f32; 3],
    t_world: f32,
) -> Option<f32> {
    let p = [
        ro[0] + rd[0] * t_world,
        ro[1] + rd[1] * t_world,
        ro[2] + rd[2] * t_world,
    ];
    let origin = geo.origin;
    let brick_world = geo.brick_world;
    let voxel_size = geo.voxel_size;
    let band_half = geo.band_half;
    // The tile containing `p` (mirror the shader: test the float directly so a negative coord is
    // caught before the cast). Outside the bounded grid → no atlas tile (the caller folds analytic).
    let rel = [
        (p[0] - origin[0]) / brick_world,
        (p[1] - origin[1]) / brick_world,
        (p[2] - origin[2]) / brick_world,
    ];
    if rel[0] < 0.0 || rel[1] < 0.0 || rel[2] < 0.0 {
        return None;
    }
    let tx = rel[0] as u32;
    let ty = rel[1] as u32;
    let tz = rel[2] as u32;
    if tx >= M2_GRID_DIM || ty >= M2_GRID_DIM || tz >= M2_GRID_DIM {
        return None;
    }
    let cell_min = geo.cell_min([tx, ty, tz]);

    // Clip the world ray to the brick AABB.
    let (t_enter, t_exit) = brick_aabb_span_world(p, rd, cell_min, brick_world)?;

    // Bake THIS tile from the authority (the SAME data the level's GPU atlas holds for this cell), then
    // run the JCGT cubic in interior-voxel units (world → voxel: (world - cell_min) / voxel_size). The
    // cubic's local `t` is in WORLD units (rd is divided by voxel_size to keep the world-t metric).
    let field = edits_field(edits);
    let mut tile = [0i8; BRICK_VOXELS];
    fill_brick(&field, cell_min, voxel_size, band_half, geo.c_max, &mut tile);
    let ro_v = [
        (p[0] - cell_min[0]) / voxel_size,
        (p[1] - cell_min[1]) / voxel_size,
        (p[2] - cell_min[2]) / voxel_size,
    ];
    let rd_v = [rd[0] / voxel_size, rd[1] / voxel_size, rd[2] / voxel_size];

    let local = brick_cubic_hit(&tile, ro_v, rd_v, t_enter, t_exit, band_half)?;

    // The candidate world `t` (local is measured from `p`, in world units).
    let cand_t = t_world + local;

    // ANALYTIC-RESIDUAL FALLBACK (the exact-CSG guarantee): a SIGNED, under-relaxed sphere-trace from
    // the cubic candidate onto the EXACT field decides BOTH whether this is a hit and where the
    // committed `t` lands. The committed `t` always satisfies `|sdf| < SDF_EPS` (on-surface) — never
    // the raw `cand_t`, which the down-biased brick (EPSILON_Q, scaled `2^L` per clip-map level) parks
    // INSIDE the surface (`d < 0`) where the baked AO cratered (BUG-M2-CRATER). A forward-only step
    // (`d.max(SDF_EPS)`) could never pull it back out; the signed step `rt += M2_REFINE_RELAX * d`
    // walks BACKWARD for `d < 0` (toward the surface) and forward for `d > 0` — a unit-gradient SDF
    // Newton step, under-relaxed against crease overshoot. Accept on `|d|` (not signed `d`) so an
    // inside candidate is corrected, never committed as-is. ONLY a refine-CONVERGED candidate
    // (`|d| < SDF_EPS`) is accepted; a grazing silhouette point (analytic miss within the old crease
    // band) or a hard crease where the refine stalls falls to `None` → the caller's M1 analytic fold,
    // which resolves the pixel EXACTLY as the OFF path. Removing the old trailing crease-accept band
    // (which accepted a NON-converged candidate within `M2_CREASE_EPS`) erased the 1-2px silhouette
    // rim where the brick hit but the analytic ray missed (BUG-M2-RIM). Mirrors the shader's refine
    // loop bit-for-bit.
    let mut rt = cand_t;
    for _ in 0..M2_REFINE_ITERS {
        let q = [ro[0] + rd[0] * rt, ro[1] + rd[1] * rt, ro[2] + rd[2] * rt];
        let d = sdf_edit_list(edits, q);
        if d.abs() < SDF_EPS {
            return Some(rt);
        }
        // Split the under-relaxed step into a named value then add it (no FMA contraction), so the
        // shader's `step = M2_REFINE_RELAX * d; rt += step;` rounds bit-identically (two roundings).
        let step = M2_REFINE_RELAX * d;
        rt += step;
        // Bail if the signed walk left the valid `t` span (the shader's `rt < 0.0 || rt > T_MAX`).
        // This is loop control flow, not bit-mirrored arithmetic — the range form is the same
        // truth table; clippy prefers it.
        if !(0.0..=SDF_T_MAX).contains(&rt) {
            break;
        }
    }
    // The refine did not reach `|d| < SDF_EPS` within M2_REFINE_ITERS: no confident hit in this brick
    // (a grazing silhouette point where the analytic ray passes the surface to the SIDE, or a hard
    // crease where the refine stalls). Return `None` → the caller folds the M1 analytic field for this
    // step, which resolves the pixel EXACTLY as the OFF path. Mirrors the shader's trailing `return
    // false`.
    None
}

/// Builds a transient single-`gen` [`SdfEditField`] from `edits` for the per-tile [`fill_brick`] /
/// [`classify_brick`] bake (these take the authority field, not a raw slice). The render/golden
/// path's authority is the SAME edit set, so the baked tile is bit-identical. `SdfEditField` is a
/// fixed-size `Copy` POD (no heap), so this is a cheap stack build — the host mirror is a CPU-only
/// reference, not the GPU hot path.
#[inline]
pub(crate) fn edits_field(edits: &[SdfEdit]) -> SdfEditField {
    let mut field = SdfEditField::new();
    for e in edits {
        debug_assert!(field.push(*e), "golden M2 scene must fit MAX_SDF_EDITS");
    }
    field.bump_gen();
    field
}

/// M2 — the trilinear+JCGT-cubic SURFACE-brick golden. Identical to
/// [`golden_composite_pixel_brick`] but the PRIMARY march runs the M2 SURFACE-brick path when
/// `brick_trilinear == true`: at each march point inside the bounded M2 grid the atlas cubic
/// ([`host_m2_surface_hit`]) is tried; a hit TERMINATES the march at the analytically-validated
/// `t` (hit/normal/shade stay ANALYTIC — C1), and a no-crossing / cleared-refine falls through to
/// the M1 step (empty-skip when `brick_enabled`, else the analytic fold). INDEPENDENT of
/// `brick_enabled` (the two gates are orthogonal).
///
/// With `brick_trilinear == false` this delegates to [`golden_composite_pixel_brick`] —
/// BYTE-IDENTICAL to the M1 golden (the M2 0%-gate). This is the bit-exact reference the GPU M2
/// golden compares against.
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_brick_m2(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
    brick_enabled: bool,
    brick_trilinear: bool,
    grid: &PointerGrid,
    cells: &[u32],
) -> u32 {
    // The OFF path is byte-identical to the M1 marcher (the M2 0%-gate): the atlas is never sampled.
    if !brick_trilinear {
        return golden_composite_pixel_brick(
            edits, mesh_depth, px, py, img_w, img_h, camera, omega, lighting_flags, light_dir,
            brick_enabled, grid, cells,
        );
    }

    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    let mut t = 0.0_f32;
    let t_seed = t;
    let mut omega = omega; // [1.0, 1.99]; sor-fail latches it to 1.0
    let mut hit = false;
    let mut safe_t = 0.0_f32;
    let mut sor_prev = 0.0_f32;
    let mut sor_step_prev = 0.0_f32;
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false; // mesh-occlusion termination
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];

        // M2 SURFACE-brick path: try the atlas cubic at `p`. A hit terminates the march at the
        // analytically-validated `t` (the hit/normal/shade stay analytic — C1). INDEPENDENT of the
        // M1 empty-skip below: the M2 step is taken FIRST (it owns the SURFACE cells the empty-skip
        // never skips), and a no-crossing falls through to the M1 / analytic step.
        if let Some(m2_hit_t) = host_m2_surface_hit(edits, ro, rd, t) {
            hit = true;
            exhausted = false; // M2 cubic+analytic-validated convergence
            t = m2_hit_t;
            break;
        }

        // M1 empty skip (when on): an EmptyOutside cell at `p` steps to the brick AABB exit. The
        // SURFACE cells the M2 step owns are NOT EmptyOutside, so this only accelerates EMPTY space.
        if brick_enabled
            && let Some((class, cell_min)) = host_brick_cell(grid, cells, p)
            && class == BRICK_CLASS_EMPTY_OUTSIDE
        {
            let exit = dist_to_brick_exit(p, rd, cell_min, grid.brick_world);
            t += exit;
            if t > SDF_T_MAX {
                exhausted = false; // clear-miss termination
                break;
            }
            continue; // skip the analytic fold this step
        }

        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false; // converged
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                debug_assert!(it > 0, "B1 budget: a>=1 precondition");
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d; // frozen plain arm
        }
        if t > SDF_T_MAX {
            exhausted = false; // clear-miss termination
            break;
        }
    }

    // The re-march fallback stays ANALYTIC (C1) — identical to the M1 path: the M2 step never
    // reopens the B1 budget hole (its hit terminates, its miss falls through), so `exhausted` here
    // means the analytic field ran out of budget mid-field; the frozen plain re-march resolves it.
    if exhausted {
        t = t_seed;
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d; // frozen plain step
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, light_dir, lighting_flags, &|q| {
            sdf_edit_list(edits, q)
        })
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

/// The host mirror of the shader's `select_level` (M4 Slice C): the FINEST enclosing clip-map level
/// for world point `p`, or `None` when `p` is outside EVERY active level (`0..brick_levels`). The
/// levels are strictly concentric (level `L`'s extent doubles), so the first-enclosing scan (level 0 =
/// finest) returns the tightest LOD. `brick_levels <= BRICK_LEVELS` is the runtime level count; the
/// containment test mirrors the shader (`all(p >= origin) && all(p < origin + dims * brick_world)`).
///
/// OFF/N=1 keystone: `brick_levels == 1` scans ONLY level 0 → `Some(0)` iff `p` is in the level-0 box,
/// else `None` — exactly the M2 single-grid containment, so the M4 golden reduces to the M2 golden.
pub(crate) fn host_select_level(params: &M4GridParams, brick_levels: u32, p: [f32; 3]) -> Option<usize> {
    let n = (brick_levels as usize).min(brick::BRICK_LEVELS);
    for level in 0..n {
        let geo = BrickLevelParams::at_level_from_params(params, level);
        let hi = [
            geo.origin[0] + M2_GRID_DIM as f32 * geo.brick_world,
            geo.origin[1] + M2_GRID_DIM as f32 * geo.brick_world,
            geo.origin[2] + M2_GRID_DIM as f32 * geo.brick_world,
        ];
        if p[0] >= geo.origin[0]
            && p[1] >= geo.origin[1]
            && p[2] >= geo.origin[2]
            && p[0] < hi[0]
            && p[1] < hi[1]
            && p[2] < hi[2]
        {
            return Some(level);
        }
    }
    None
}

/// M4 — the N-level brick CLIP-MAP golden (Slice C). Generalizes [`golden_composite_pixel_brick_m2`]
/// to `brick_levels` nested clip-map levels: at each primary march point the finest enclosing level is
/// picked ([`host_select_level`]); the M2 SURFACE-brick cubic runs at THAT level's geometry
/// ([`host_m2_surface_hit_at`]) and the M1 empty-skip reads THAT level's pointer grid. A hit TERMINATES
/// the march at the analytically-validated `t` (hit/normal/shade stay ANALYTIC — C1); a no-crossing /
/// outside-all-levels point folds the analytic field, exactly as the single-level M2 path. This is the
/// CPU oracle the offscreen RTX M4 test compares the GPU `gViewT`/LIT against.
///
/// `params` is the [`M4GridParams`] written into the b5 UBO tail (the per-level snapped origins). The
/// per-level EMPTY-SKIP pointer grids `level_grids[L] = (grid, cells)` mirror the GPU's per-level
/// `PointerGrid{L}` SSBOs (binding 9/11/13). These are DISTINCT from the per-level SURFACE atlas grids
/// (read inside [`host_m2_surface_hit_at`] via `at_level_from_params`), exactly as M2 keeps them
/// separate: level 0's empty-skip grid is the FINE `default_near_field` (`16³ @ 0.5`, the GPU binding-9
/// the shader reads via `pc.grid_*`), while its surface atlas is the COARSE `4³ @ 2.0` grid. With
/// `brick_trilinear == false` this delegates to [`golden_composite_pixel_brick`] (the M1 analytic
/// golden at level 0's empty-skip grid) — BYTE-IDENTICAL to the M1 golden (the OFF path).
///
/// # OFF/N=1 keystone
///
/// `brick_levels == 1` (and `params == M4GridParams::near_field_only()`, so level 0 == the M2
/// near-field) makes [`host_select_level`] reduce to the M2 containment, [`host_m2_surface_hit_at`]
/// bake at level-0 geometry, and the empty-skip read level 0's FINE `16³ @ 0.5` grid (the SAME grid
/// `golden_composite_pixel_brick_m2`'s empty-skip reads) — so the packed output is byte-IDENTICAL to
/// [`golden_composite_pixel_brick_m2`] (asserted in the tests, the 0%-gate).
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_brick_m4(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
    brick_enabled: bool,
    brick_trilinear: bool,
    brick_levels: u32,
    params: &M4GridParams,
    level_grids: &[(PointerGrid, &[u32])],
) -> u32 {
    // The OFF path (no trilinear) is byte-identical to the M1 marcher at level 0's grid (the M4 0%-gate
    // for the analytic-only path): the atlas is never sampled, the empty-skip reads level 0's grid.
    if !brick_trilinear {
        let (grid, cells) = &level_grids[0];
        return golden_composite_pixel_brick(
            edits, mesh_depth, px, py, img_w, img_h, camera, omega, lighting_flags, light_dir,
            brick_enabled, grid, cells,
        );
    }

    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    let mut t = 0.0_f32;
    let t_seed = t;
    let mut omega = omega; // [1.0, 1.99]; sor-fail latches it to 1.0
    let mut hit = false;
    let mut safe_t = 0.0_f32;
    let mut sor_prev = 0.0_f32;
    let mut sor_step_prev = 0.0_f32;
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false; // mesh-occlusion termination
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];

        // M4 clip-map LOD: pick the finest enclosing level for `p` (None ⇒ outside every active level
        // ⇒ fold the analytic field, exactly as M2 does outside its single grid).
        let lvl = host_select_level(params, brick_levels, p);

        // M2 SURFACE-brick path at the selected level: a hit terminates the march at the
        // analytically-validated `t`. Mirrors the shader's per-level branch-ladder (lvl >= 0 arm).
        if let Some(level) = lvl {
            let geo = BrickLevelParams::at_level_from_params(params, level);
            if let Some(m4_hit_t) = host_m2_surface_hit_at(edits, &geo, ro, rd, t) {
                hit = true;
                exhausted = false; // M2 cubic+analytic-validated convergence
                t = m4_hit_t;
                break;
            }
        }

        // M1 empty skip (when on) at the selected level's EMPTY-SKIP grid: an EmptyOutside cell steps to
        // the brick AABB exit. `level_grids[level]` is the empty-skip grid the GPU's `PointerGrid{level}`
        // holds — DISTINCT from the surface atlas grid the M2 step above reads (level 0's empty-skip is
        // the FINE `16³@0.5` near-field grid `pc.grid_*` carries, matching M2 bit-for-bit; coarse levels
        // use the per-level `4³@scaled` grid). The SURFACE cells the M2 step owns are NOT EmptyOutside, so
        // this only accelerates EMPTY space. `None` (outside all levels) ⇒ no skip (fold analytic).
        if brick_enabled
            && let Some(level) = lvl
            && let Some((grid, cells)) = level_grids.get(level)
            && let Some((class, cell_min)) = host_brick_cell(grid, cells, p)
            && class == BRICK_CLASS_EMPTY_OUTSIDE
        {
            let exit = dist_to_brick_exit(p, rd, cell_min, grid.brick_world);
            t += exit;
            if t > SDF_T_MAX {
                exhausted = false; // clear-miss termination
                break;
            }
            continue; // skip the analytic fold this step
        }

        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false; // converged
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                debug_assert!(it > 0, "B1 budget: a>=1 precondition");
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d; // frozen plain arm
        }
        if t > SDF_T_MAX {
            exhausted = false; // clear-miss termination
            break;
        }
    }

    // The re-march fallback stays ANALYTIC (C1) — identical to the M1/M2 path: the M4 per-level step
    // never reopens the B1 budget hole (its hit terminates, its miss falls through), so `exhausted`
    // here means the analytic field ran out of budget mid-field; the frozen plain re-march resolves it.
    if exhausted {
        t = t_seed;
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d; // frozen plain step
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, light_dir, lighting_flags, &|q| {
            sdf_edit_list(edits, q)
        })
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

/// A host material-table element mirroring `boyko_render::material::MaterialGpu` (3
/// std430 `vec4` lanes, 48 B). The vulkan crate cannot depend on `boyko_render` (the
/// dependency runs the other way), so the golden carries its own POD mirror; the layout
/// is the SAME the shader's `MaterialGpu` reads. All values are LINEAR.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoldenMaterial {
    /// `rgb` = LINEAR base color, `w` = alpha/cutoff (lane 0).
    pub base_color: [f32; 4],
    /// `[metallic, roughness, reflectance, bitcast(flags)]` (lane 1).
    pub mrr: [f32; 4],
    /// `rgb` = LINEAR emissive, `w` unused (lane 2).
    pub emissive: [f32; 4],
}

impl GoldenMaterial {
    /// A metallic-roughness material from LINEAR parameters (mirrors `MaterialGpu::new`).
    #[inline]
    pub fn new(
        base_color: [f32; 4],
        metallic: f32,
        roughness: f32,
        reflectance: f32,
        emissive: [f32; 3],
    ) -> Self {
        Self {
            base_color,
            mrr: [metallic, roughness, reflectance, 0.0],
            emissive: [emissive[0], emissive[1], emissive[2], 0.0],
        }
    }
}

impl Default for GoldenMaterial {
    /// The engine default material (table slot 0): a mid-gray dielectric (mirrors
    /// `MaterialGpu::default`).
    #[inline]
    fn default() -> Self {
        GoldenMaterial::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.5, 0.5, [0.0, 0.0, 0.0])
    }
}

/// Textured-PBR T6a: a host mirror of `boyko_render::MATERIAL_FLAG_TEXTURED` — a bit in
/// [`GoldenMaterial::mrr`]'s bitcast `flags` lane (`mrr[3]`), set iff the material carries a
/// texture sidecar. The vulkan crate cannot depend on `boyko_render` (the dependency runs the
/// other way; see [`GoldenMaterial`]'s doc), so this is a SEPARATE literal, cross-checked
/// against the real constant by a `boyko_render`-side test (which CAN see both crates via its
/// `boyko_rhi_vulkan` dev-dependency). [`GoldenMaterial::new`] never sets this bit (`mrr[3]`
/// stays `0.0`), so every EXISTING golden input is inert under
/// [`golden_deferred_resolve_with_pbr`]'s flag-gated override.
pub const GOLDEN_MATERIAL_FLAG_TEXTURED: u32 = 1;

/// A host light-table element mirroring `boyko_render::light::GpuLight` (3 std430 `vec4`
/// lanes, 48 B). The vulkan crate cannot depend on `boyko_render`, so the golden carries
/// its own POD mirror; the layout is the SAME the shader's `GpuLight` reads. LINEAR.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoldenLight {
    /// `xyz` = direction TO the light (directional/spot), `w` = bit-cast kind tag.
    pub dir_kind: [f32; 4],
    /// `xyz` = position (point/spot) or ground color (sky), `w` = cull radius.
    pub pos_range: [f32; 4],
    /// `rgb` = LINEAR color × baked intensity (or sky color), `w` = packed spot cones.
    pub color_cone: [f32; 4],
}

impl GoldenLight {
    /// A directional light (mirrors `GpuLight::from_directional`): `color × illuminance`
    /// premultiplied into the color lane.
    #[inline]
    pub fn directional(direction: [f32; 3], color: [f32; 3], illuminance: f32) -> Self {
        let d = v_normalize(direction);
        Self {
            dir_kind: [d[0], d[1], d[2], f32::from_bits(GOLDEN_LIGHT_KIND_DIRECTIONAL)],
            pos_range: [0.0, 0.0, 0.0, f32::INFINITY],
            color_cone: [color[0] * illuminance, color[1] * illuminance, color[2] * illuminance, 0.0],
        }
    }

    /// A sky/ambient light (mirrors `GpuLight::from_sky`): sky color in the color lane,
    /// ground color in the position lane.
    #[inline]
    pub fn sky(sky_color: [f32; 3], ground_color: [f32; 3]) -> Self {
        Self {
            dir_kind: [0.0, 0.0, 0.0, f32::from_bits(GOLDEN_LIGHT_KIND_SKY)],
            pos_range: [ground_color[0], ground_color[1], ground_color[2], 0.0],
            color_cone: [sky_color[0], sky_color[1], sky_color[2], 0.0],
        }
    }

    /// A point light (mirrors `GpuLight::from_point`, Lighting L0b): position + range in
    /// `pos_range`, the baked intensity `I = Φ / (4π)` premultiplied into the color lane.
    /// `power` is the luminous power `Φ`. The L0b resolve oracle consumes `pos_range` (the
    /// world position + the cull radius) + the baked color.
    #[inline]
    pub fn point(position: [f32; 3], color: [f32; 3], power: f32, range: f32) -> Self {
        let i = power / (4.0 * core::f32::consts::PI);
        Self {
            dir_kind: [0.0, 0.0, 0.0, f32::from_bits(GOLDEN_LIGHT_KIND_POINT)],
            pos_range: [position[0], position[1], position[2], range],
            color_cone: [color[0] * i, color[1] * i, color[2] * i, 0.0],
        }
    }

    /// A spot light (mirrors `GpuLight::from_spot`, Lighting L0b): the spot axis in
    /// `dir_kind.xyz`, position + range in `pos_range`, the baked reflector intensity
    /// `I = Φ / (2π(1 − cos(outer)))` premultiplied into the color lane, and the cone
    /// cosines packed (two f16) into `color_cone.w`. `inner_deg`/`outer_deg` are cone
    /// half-angles in degrees; `cos(outer)` is clamped to `SPOT_COS_OUTER_MAX` (0.9999) so
    /// the intensity stays bounded — mirroring the host constructor's release safety net.
    #[inline]
    pub fn spot(
        position: [f32; 3],
        direction: [f32; 3],
        color: [f32; 3],
        power: f32,
        range: f32,
        inner_deg: f32,
        outer_deg: f32,
    ) -> Self {
        let cos_inner = inner_deg.to_radians().cos();
        let cos_outer = outer_deg.to_radians().cos().min(GOLDEN_SPOT_COS_OUTER_MAX);
        let denom = 2.0 * core::f32::consts::PI * (1.0 - cos_outer);
        let i = power / denom;
        let d = v_normalize(direction);
        Self {
            dir_kind: [d[0], d[1], d[2], f32::from_bits(GOLDEN_LIGHT_KIND_SPOT)],
            pos_range: [position[0], position[1], position[2], range],
            color_cone: [
                color[0] * i,
                color[1] * i,
                color[2] * i,
                golden_pack_cones(cos_inner, cos_outer),
            ],
        }
    }

    /// The bit-cast kind tag from `dir_kind.w` (the flag bits masked off — mirrors the
    /// shader's `light_kind()`). The P6 R1 `casts_sdf_shadow` flag lives in bit 16 of the
    /// same word; on every pre-P6 light bit 16 is 0, so this is byte-equivalent to the raw
    /// bitcast (the 0%-gate).
    #[inline]
    pub fn kind(&self) -> u32 {
        self.dir_kind[3].to_bits() & GOLDEN_LIGHT_KIND_MASK
    }

    /// True iff this light is flagged a P6 R1 per-light SDF-shadow caster (bit 16 of the kind
    /// word — mirrors the shader's `light_casts_sdf_shadow()`).
    #[inline]
    pub fn casts_sdf_shadow(&self) -> bool {
        (self.dir_kind[3].to_bits() & GOLDEN_LIGHT_FLAG_CASTS_SHADOW) != 0
    }

    /// Flags this light a P6 R1 SDF-shadow caster (sets bit 16 of the kind word). The
    /// builder the multi-light goldens use; the kind enum (low bits) is preserved.
    #[inline]
    pub fn with_sdf_shadow(mut self) -> Self {
        let bits = self.dir_kind[3].to_bits() | GOLDEN_LIGHT_FLAG_CASTS_SHADOW;
        self.dir_kind[3] = f32::from_bits(bits);
        self
    }

    /// Packs a Shadow Phase 5 Inc-1-GPU atlas-SLOT index into bits `17..22` of the kind word — the
    /// host mirror of `boyko_render::shadow_atlas::pack_atlas_slot` (the SAME bit layout the resolve
    /// reads via `light_table.hlsli::light_atlas_slot`). The kind tag (bits 0..16) is preserved; a
    /// real slot (`slot != GOLDEN_SLOT_NONE`) also sets [`GOLDEN_LIGHT_FLAG_CASTS_SHADOW`] (bit 16),
    /// so the resolve branches onto the map sample. `slot` MUST be `< 16` (the layer budget) or
    /// exactly [`GOLDEN_SLOT_NONE`]; a debug build asserts it. The demo hand-builds the light table,
    /// so it stamps the slot directly with this builder; the real-app path is the
    /// `resolve_shadow_atlas` → light-table-assembly seam (`boyko_render::shadow_atlas`).
    #[inline]
    pub fn with_atlas_slot(mut self, slot: u32) -> Self {
        debug_assert!(
            slot < 16 || slot == GOLDEN_SLOT_NONE,
            "invariant: atlas slot must be a real layer (< M_SLOTS == 16) or GOLDEN_SLOT_NONE"
        );
        let base = self.dir_kind[3].to_bits()
            & !(GOLDEN_ATLAS_SLOT_MASK << GOLDEN_ATLAS_SLOT_SHIFT)
            & !GOLDEN_LIGHT_FLAG_CASTS_SHADOW;
        let with_slot = base | ((slot & GOLDEN_ATLAS_SLOT_MASK) << GOLDEN_ATLAS_SLOT_SHIFT);
        let bits = if slot == GOLDEN_SLOT_NONE {
            with_slot
        } else {
            with_slot | GOLDEN_LIGHT_FLAG_CASTS_SHADOW
        };
        self.dir_kind[3] = f32::from_bits(bits);
        self
    }

    /// The Shadow Phase 5 Inc-1-GPU atlas-slot index packed in the kind word (bits `17..22`) — the
    /// host mirror of `light_table.hlsli::light_atlas_slot`. Returns the layer index `[0, 16)` or
    /// [`GOLDEN_SLOT_NONE`].
    #[inline]
    pub fn atlas_slot(&self) -> u32 {
        (self.dir_kind[3].to_bits() >> GOLDEN_ATLAS_SLOT_SHIFT) & GOLDEN_ATLAS_SLOT_MASK
    }
}

/// Packs two cosines into the `f16 | f16` bit pattern carried in
/// [`GoldenLight::color_cone`]`.w` (`cos_inner` low half, `cos_outer` high half) — the
/// host mirror of `boyko_render::light::pack_cones`; the resolve oracle's
/// [`golden_unpack_cones`] is the inverse (matching the shader's `f16tof32`).
pub(crate) fn golden_pack_cones(cos_inner: f32, cos_outer: f32) -> f32 {
    let lo = golden_f16_from_f32(cos_inner) as u32;
    let hi = golden_f16_from_f32(cos_outer) as u32;
    f32::from_bits(lo | (hi << 16))
}

/// Unpacks two f16 cone cosines from a `color_cone.w` bit pattern — the host mirror of the
/// shader's `unpack_cones` (`f16tof32`). Returns `(cos_inner, cos_outer)`.
pub(crate) fn golden_unpack_cones(packed: f32) -> (f32, f32) {
    let bits = packed.to_bits();
    let lo = golden_f16_to_f32((bits & 0xFFFF) as u16);
    let hi = golden_f16_to_f32(((bits >> 16) & 0xFFFF) as u16);
    (lo, hi)
}

/// IEEE-754 binary16 → binary32 — the host mirror of the shader's `f16tof32`.
pub(crate) fn golden_f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x3ff) as u32;
    let out = if exp == 0 {
        if mant == 0 {
            sign << 31
        } else {
            let mut e = -1i32;
            let mut m = mant;
            loop {
                e += 1;
                m <<= 1;
                if m & 0x400 != 0 {
                    break;
                }
            }
            let new_exp = (127 - 15 - e) as u32;
            (sign << 31) | (new_exp << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 0x1f {
        (sign << 31) | 0x7f80_0000 | (mant << 13)
    } else {
        let new_exp = exp + (127 - 15);
        (sign << 31) | (new_exp << 23) | (mant << 13)
    };
    f32::from_bits(out)
}

/// A host light-table header mirroring `boyko_render::light::LightHeaderGpu` (4 std430
/// `vec4` lanes, 64 B). Carries the split counts + exposure (Decision 3 / O3).
///
/// See `boyko_render::light`'s "Light-header word 7 bit budget" table for the full bit
/// map this type's `with_*_mode` builders below pack into (word 7 / `sky_diffuse.w`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoldenLightHeader {
    /// `[bitcast(light_count), exposure, bitcast(l0a_count), bitcast(point_spot_count)]`.
    pub counts_exposure: [f32; 4],
    /// Ambient hemisphere diffuse `rgb`, `w` unused (carried; the L0a resolve drives
    /// ambient from the sky light entities, not these — see `golden_deferred_resolve_table`).
    pub sky_diffuse: [f32; 4],
    /// Ambient specular `rgb`, `w` unused (carried; see above).
    pub sky_spec: [f32; 4],
    /// L1 cluster params (zero in L0).
    pub cluster_params: [f32; 4],
}

impl GoldenLightHeader {
    /// Builds the header (mirrors `LightHeaderGpu::new`). `l0a_count` = directionals +
    /// sky; `point_spot_count` = the L0b block; exposure default 1.0.
    #[inline]
    pub fn new(l0a_count: u32, point_spot_count: u32, exposure: f32) -> Self {
        let light_count = l0a_count + point_spot_count;
        Self {
            counts_exposure: [
                f32::from_bits(light_count),
                exposure,
                f32::from_bits(l0a_count),
                f32::from_bits(point_spot_count),
            ],
            sky_diffuse: [PBR_SKY_DIFFUSE[0], PBR_SKY_DIFFUSE[1], PBR_SKY_DIFFUSE[2], 0.0],
            sky_spec: [PBR_SKY_SPEC[0], PBR_SKY_SPEC[1], PBR_SKY_SPEC[2], 0.0],
            cluster_params: [0.0, 0.0, 0.0, 0.0],
        }
    }

    /// The total `light_count` field (bit-cast back from `counts_exposure.x`).
    #[inline]
    pub fn light_count(&self) -> u32 {
        self.counts_exposure[0].to_bits()
    }

    /// The `l0a_count` field (bit-cast back from `counts_exposure.z`).
    #[inline]
    pub fn l0a_count(&self) -> u32 {
        self.counts_exposure[2].to_bits()
    }

    /// The `point_spot_count` field — the L0b block (bit-cast back from `counts_exposure.w`).
    #[inline]
    pub fn point_spot_count(&self) -> u32 {
        self.counts_exposure[3].to_bits()
    }

    /// The exposure field (`counts_exposure.y`).
    #[inline]
    pub fn exposure(&self) -> f32 {
        self.counts_exposure[1]
    }

    /// Sets the P6 R1 `shadow_mode` into BIT 0 of header WORD 7 (`sky_diffuse.w`), read by the
    /// shader's `load_shadow_mode` (which masks to bit 0). 0 = single-directional legacy (the
    /// BYTE-IDENTICAL 0%-gate); 1 = multi-light (the primary directional keeps `gMaterial.r`,
    /// every extra flagged caster gets a `sdf_soft_shadow_ranged` march). The builder the
    /// multi-light goldens use. Only BIT 0 is written — the Render Shadow Phase 3
    /// `contact_shadow_mode` (BIT 1, see [`with_contact_shadow_mode`]) in the same word is
    /// PRESERVED, so the two are order-independent. Byte-identical for every existing caller
    /// (all pass `shadow_mode ∈ {0,1}` on a fresh header whose word 7 is 0, so `0 | s == s`).
    ///
    /// [`with_contact_shadow_mode`]: Self::with_contact_shadow_mode
    #[inline]
    pub fn with_shadow_mode(mut self, shadow_mode: u32) -> Self {
        let word7 = (self.sky_diffuse[3].to_bits() & !1) | (shadow_mode & 1);
        self.sky_diffuse[3] = f32::from_bits(word7);
        self
    }

    /// The P6 R1 `shadow_mode` (header word 7, bit-cast back from `sky_diffuse.w`, masked to
    /// BIT 0 — the contact-shadow flag lives in BIT 1, see [`with_contact_shadow_mode`]). 0 on
    /// every pre-P6 scene (the 0%-gate).
    ///
    /// [`with_contact_shadow_mode`]: Self::with_contact_shadow_mode
    #[inline]
    pub fn shadow_mode(&self) -> u32 {
        self.sky_diffuse[3].to_bits() & 1
    }

    /// Sets the Render Shadow Phase 3 `contact_shadow_mode` — Screen-Space Contact Shadows
    /// (SSCS) — packed into BIT 1 of header WORD 7 (`sky_diffuse.w`), the SAME word
    /// [`with_shadow_mode`](Self::with_shadow_mode) packs the `shadow_mode` into (BIT 0). The
    /// header is FULL (16 words / 4 vec4), so a spare BIT is used rather than a new word (which
    /// would shift `LIGHT_HEADER_BASE` and re-encode every golden). `on` ORs/clears ONLY bit 1,
    /// preserving the `shadow_mode` bit, so the two are independent. `false` leaves word 7
    /// unchanged on a fresh header (BIT 1 already 0 — the 0%-gate: every pre-Phase-3 scene reads
    /// `contact_shadow_mode == 0`, so the resolve's SSCS march block is never run).
    #[inline]
    pub fn with_contact_shadow_mode(mut self, on: bool) -> Self {
        let mut word7 = self.sky_diffuse[3].to_bits();
        if on {
            word7 |= 0b10;
        } else {
            word7 &= !0b10;
        }
        self.sky_diffuse[3] = f32::from_bits(word7);
        self
    }

    /// The Render Shadow Phase 3 `contact_shadow_mode` (header word 7 BIT 1, bit-cast back from
    /// `sky_diffuse.w`). 0 on every pre-Phase-3 scene (the 0%-gate); 1 when SSCS is armed.
    #[inline]
    pub fn contact_shadow_mode(&self) -> u32 {
        (self.sky_diffuse[3].to_bits() >> 1) & 1
    }

    /// Sets the CSM Increment-1b `csm_mode` — Cascaded Shadow Maps — packed into BIT 2 of header
    /// WORD 7 (`sky_diffuse.w`), the SAME word [`with_shadow_mode`](Self::with_shadow_mode) packs
    /// `shadow_mode` (BIT 0) and [`with_contact_shadow_mode`](Self::with_contact_shadow_mode)
    /// packs `contact_shadow_mode` (BIT 1) into. The header is FULL (16 words / 4 vec4), so a
    /// spare BIT is used rather than a new word (which would shift `LIGHT_HEADER_BASE` and
    /// re-encode every golden). `on` ORs/clears ONLY bit 2, preserving bits 0/1, so the three
    /// flags are independent and order-agnostic. `false` leaves word 7 unchanged on a fresh
    /// header (BIT 2 already 0 — the 0%-gate: every pre-CSM scene reads `csm_mode == 0`, so the
    /// resolve's CSM sample block is never run and the bound-but-unread cascade map/sampler/UBO
    /// are never sampled). Read GPU-side by `light_table.hlsli::load_csm_mode` (`(word7 >> 2) &
    /// 1`).
    #[inline]
    pub fn with_csm_mode(mut self, on: bool) -> Self {
        let mut word7 = self.sky_diffuse[3].to_bits();
        if on {
            word7 |= 0b100;
        } else {
            word7 &= !0b100;
        }
        self.sky_diffuse[3] = f32::from_bits(word7);
        self
    }

    /// The CSM Increment-1b `csm_mode` (header word 7 BIT 2, bit-cast back from `sky_diffuse.w`).
    /// 0 on every pre-CSM scene (the 0%-gate); 1 when the cascade shadow map is armed.
    #[inline]
    pub fn csm_mode(&self) -> u32 {
        (self.sky_diffuse[3].to_bits() >> 2) & 1
    }

    /// Sets the Shadow Phase 5 Inc-1-GPU `punctual_shadow_mode` — sparse SPOT/POINT hardware shadow
    /// maps — packed into BIT 3 of header WORD 7 (`sky_diffuse.w`), the SAME word
    /// [`with_shadow_mode`](Self::with_shadow_mode) packs `shadow_mode` (BIT 0),
    /// [`with_contact_shadow_mode`](Self::with_contact_shadow_mode) packs `contact_shadow_mode`
    /// (BIT 1), and [`with_csm_mode`](Self::with_csm_mode) packs `csm_mode` (BIT 2) into. The header
    /// is FULL (16 words / 4 vec4), so a spare BIT is used rather than a new word (which would shift
    /// `LIGHT_HEADER_BASE` and re-encode every golden). `on` ORs/clears ONLY bit 3, preserving bits
    /// 0/1/2, so the four flags are independent and order-agnostic. `false` leaves word 7 unchanged
    /// on a fresh header (BIT 3 already 0 — the 0%-gate: every pre-Inc-1 scene reads
    /// `punctual_shadow_mode == 0`, so the resolve's spot-atlas sample block is never run and the
    /// bound-but-unread atlas map/sampler/UBO are never sampled). Read GPU-side by
    /// `light_table.hlsli::load_punctual_shadow_mode` (`(word7 >> 3) & 1`).
    #[inline]
    pub fn with_punctual_shadow_mode(mut self, on: bool) -> Self {
        let mut word7 = self.sky_diffuse[3].to_bits();
        if on {
            word7 |= 0b1000;
        } else {
            word7 &= !0b1000;
        }
        self.sky_diffuse[3] = f32::from_bits(word7);
        self
    }

    /// The Shadow Phase 5 Inc-1-GPU `punctual_shadow_mode` (header word 7 BIT 3, bit-cast back from
    /// `sky_diffuse.w`). 0 on every pre-Inc-1 scene (the 0%-gate); 1 when the sparse spot/point
    /// shadow atlas is armed.
    #[inline]
    pub fn punctual_shadow_mode(&self) -> u32 {
        (self.sky_diffuse[3].to_bits() >> 3) & 1
    }

    /// Sets the SDFDDGI I4 `ddgi_mode` — dynamic diffuse GI injection — packed into BIT 4 of header
    /// WORD 7 (`sky_diffuse.w`), the SAME word [`with_shadow_mode`](Self::with_shadow_mode) packs
    /// `shadow_mode` (BIT 0), [`with_contact_shadow_mode`](Self::with_contact_shadow_mode) packs
    /// `contact_shadow_mode` (BIT 1), [`with_csm_mode`](Self::with_csm_mode) packs `csm_mode` (BIT 2),
    /// and [`with_punctual_shadow_mode`](Self::with_punctual_shadow_mode) packs `punctual_shadow_mode`
    /// (BIT 3) into. `on` ORs/clears ONLY bit 4, preserving bits 0..3, so the five flags are
    /// independent and order-agnostic. `false` leaves word 7 byte-identical on a fresh header (BIT 4
    /// already 0 — the 0%-gate: every pre-DDGI scene reads `ddgi_mode == 0`, so the resolve's GI-
    /// injection block never runs and the bound-but-unread probe atlas/samplers/grid UBO are never
    /// sampled). This is the resolve's GI-injection GATE — the grid UBO's redundant `ddgi_mode_word`
    /// mirror is NOT what the resolve tests. Read GPU-side by `light_table.hlsli::load_ddgi_mode`
    /// (`(word7 >> 4) & 1`).
    #[inline]
    pub fn with_ddgi_mode(mut self, on: bool) -> Self {
        let mut word7 = self.sky_diffuse[3].to_bits();
        if on {
            word7 |= 1 << 4;
        } else {
            word7 &= !(1 << 4);
        }
        self.sky_diffuse[3] = f32::from_bits(word7);
        self
    }

    /// The SDFDDGI I4 `ddgi_mode` (header word 7 BIT 4, bit-cast back from `sky_diffuse.w`). 0 on
    /// every pre-DDGI scene (the 0%-gate); 1 when dynamic diffuse GI injection is armed.
    #[inline]
    pub fn ddgi_mode(&self) -> u32 {
        (self.sky_diffuse[3].to_bits() >> 4) & 1
    }

    /// Sets the Render P7 `ssao_mode` (header WORD 11 = `sky_spec.w`, read RAW by the
    /// resolve's `load_ssao_mode` — stored BIT-CAST, NOT as a float value, EXACTLY as
    /// [`with_shadow_mode`](Self::with_shadow_mode) does for word 7). `0` = SSAO OFF (the
    /// resolve combine is `ao_final == gMaterial.g`, the BYTE-IDENTICAL 0%-gate); a non-zero
    /// value arms the `ao_final = min(class_ao, gSsao)` cross-representation combine. Word 11
    /// (`sky_spec.w`) was previously always `0.0` (carried unused), so every pre-P7 scene's
    /// `ssao_mode()` reads 0 automatically. The builder the P7 SSAO goldens use.
    #[inline]
    pub fn with_ssao_mode(mut self, ssao_mode: u32) -> Self {
        self.sky_spec[3] = f32::from_bits(ssao_mode);
        self
    }

    /// The Render P7 `ssao_mode` (header word 11, bit-cast back from `sky_spec.w`). 0 on every
    /// pre-P7 scene (the 0%-gate), mirroring [`shadow_mode`](Self::shadow_mode) (word 7).
    #[inline]
    pub fn ssao_mode(&self) -> u32 {
        self.sky_spec[3].to_bits()
    }

    /// Builds the L1 CLUSTERED header (mirrors `LightHeaderGpu::new_clustered`): the
    /// `cluster_params` lane carries `[z_scale, z_bias, bitcast(packed_dims),
    /// bitcast(clusters_enabled=1)]`. The packed dims are `dim_x | dim_y<<8 | dim_z<<16`.
    #[inline]
    pub fn new_clustered(
        l0a_count: u32,
        point_spot_count: u32,
        exposure: f32,
        cfg: &GoldenClusterConfig,
    ) -> Self {
        let mut h = Self::new(l0a_count, point_spot_count, exposure);
        let packed = cfg.dim_x | (cfg.dim_y << 8) | (cfg.dim_z << 16);
        h.cluster_params = [
            cfg.z_scale(),
            cfg.z_bias(),
            f32::from_bits(packed),
            f32::from_bits(1),
        ];
        h
    }

    /// Whether the L1 cluster path is enabled (`cluster_params.w` bit-cast `!= 0`). Mirrors
    /// `LightHeaderGpu::clusters_enabled`.
    #[inline]
    pub fn clusters_enabled(&self) -> bool {
        self.cluster_params[3].to_bits() != 0
    }
}

/// Octahedral-encode a unit normal into `[0,1]^2` (mirrors the marcher's `oct_encode`).
pub(crate) fn oct_encode(n: [f32; 3]) -> [f32; 2] {
    let inv_l1 = 1.0 / (n[0].abs() + n[1].abs() + n[2].abs());
    let nx = n[0] * inv_l1;
    let ny = n[1] * inv_l1;
    let nz = n[2] * inv_l1;
    let (mut ex, mut ey) = (nx, ny);
    if nz < 0.0 {
        let sx = if nx >= 0.0 { 1.0 } else { -1.0 };
        let sy = if ny >= 0.0 { 1.0 } else { -1.0 };
        ex = (1.0 - ny.abs()) * sx;
        ey = (1.0 - nx.abs()) * sy;
    }
    [ex * 0.5 + 0.5, ey * 0.5 + 0.5]
}

/// Octahedral-decode (mirrors the resolve's `oct_decode`). `pub` (behind the `goldens` feature)
/// so the SDFDDGI I2 `oct_decode_edsl_matches_host` sync test can pin the new eDSL
/// `boyko_shaderdsl::oct::oct_decode_body::<EvalCf>` equal to this host mirror (plan §6 gate 4).
pub fn oct_decode(e: [f32; 2]) -> [f32; 3] {
    let ex = e[0] * 2.0 - 1.0;
    let ey = e[1] * 2.0 - 1.0;
    let mut n = [ex, ey, 1.0 - ex.abs() - ey.abs()];
    let t = (-n[2]).clamp(0.0, 1.0);
    n[0] += if n[0] >= 0.0 { -t } else { t };
    n[1] += if n[1] >= 0.0 { -t } else { t };
    v_normalize(n)
}

/// GGX/Trowbridge-Reitz NDF (mirrors the resolve's `D_GGX`).
pub(crate) fn d_ggx(noh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = (noh * a2 - noh) * noh + 1.0;
    a2 / (core::f32::consts::PI * d * d)
}

/// Height-correlated Smith visibility (mirrors the resolve's `V_SmithGGXCorrelated`).
pub(crate) fn v_smith_ggx_correlated(nov: f32, nol: f32, a: f32) -> f32 {
    let a2 = a * a;
    let lambda_v = nol * ((nov - a2 * nov) * nov + a2).sqrt();
    let lambda_l = nov * ((nol - a2 * nol) * nol + a2).sqrt();
    0.5 / (lambda_v + lambda_l).max(1e-5)
}

/// Schlick Fresnel (mirrors the resolve's `F_Schlick`).
pub(crate) fn f_schlick(u: f32, f0: [f32; 3]) -> [f32; 3] {
    let f = (1.0 - u).powf(5.0);
    [
        f0[0] + (1.0 - f0[0]) * f,
        f0[1] + (1.0 - f0[1]) * f,
        f0[2] + (1.0 - f0[2]) * f,
    ]
}

/// Karis mobile analytic environment BRDF (mirrors the resolve's `env_brdf_approx`).
pub(crate) fn env_brdf_approx(roughness: f32, nov: f32) -> [f32; 2] {
    let c0 = [-1.0_f32, -0.0275, -0.572, 0.022];
    let c1 = [1.0_f32, 0.0425, 1.04, -0.04];
    let r = [
        roughness * c0[0] + c1[0],
        roughness * c0[1] + c1[1],
        roughness * c0[2] + c1[2],
        roughness * c0[3] + c1[3],
    ];
    let a004 = (r[0] * r[0]).min((-9.28 * nov).exp2()) * r[0] + r[1];
    [-1.04 * a004 + r[2], 1.04 * a004 + r[3]]
}

/// HLSL `reflect(i, n)` intrinsic mirror (PBR P0-B): `i - 2.0 * dot(i, n) * n`, the reference
/// formula every DXC target lowers to. `d = dot(i, n) * 2.0` is a scalar op computed ONCE (the
/// scalar-then-vector grouping, not `2.0 * (dot(i,n) * n)`), matching the HLSL expression order.
#[inline]
pub(crate) fn v_reflect(i: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    let d = v_dot(i, n) * 2.0;
    [i[0] - n[0] * d, i[1] - n[1] * d, i[2] - n[2] * d]
}

/// PBR P0-D multi-scatter energy compensation (mirrors the resolve's hoisted
/// `energy_comp` term): `Ess = max(dfg.x + dfg.y, 1e-4)` (the Fdez-Aguera scale+bias energy
/// estimate — NOT `1/dfg.y`), `energy_comp = 1 + f0 * (1/Ess - 1)`. `dfg` is
/// [`env_brdf_approx`]`(roughness, NoV)`, computed ONCE per pixel by the caller and reused at
/// every specular site (direct + ambient).
#[inline]
pub(crate) fn multi_scatter_energy_comp(dfg: [f32; 2], f0: [f32; 3]) -> [f32; 3] {
    let ess = (dfg[0] + dfg[1]).max(1e-4);
    let inv_ess_m1 = 1.0 / ess - 1.0;
    [
        1.0 + f0[0] * inv_ess_m1,
        1.0 + f0[1] * inv_ess_m1,
        1.0 + f0[2] * inv_ess_m1,
    ]
}

/// PBR metal fix: decoupled specular occlusion (mirrors the resolve's hoisted `spec_ao`
/// term — Filament `SpecularAO_Lagarde` == Bevy deferred `specular_occlusion`):
/// `saturate(pow(NoV + ao, exp2(-16*roughness - 1)) - 1 + ao)`. Diffuse AO (`ao`/`ao_final`)
/// correctly darkens Lambert ambient, but a metal has `diffuse == 0` — its ambient SPECULAR
/// is its ENTIRE appearance, so multiplying that by diffuse AO reads as "AO-darkened matte
/// paint", not metal. `spec_ao` stays ~1 for smooth/metal surfaces and only gently occludes
/// rough+cavity surfaces, matching every competitor's diffuse/specular AO split. Computed
/// ONCE per pixel by the caller (NoV + roughness + ao only) and reused at every ambient-
/// specular site.
#[inline]
pub(crate) fn specular_ao(nov: f32, roughness: f32, ao: f32) -> f32 {
    let exponent = (-16.0 * roughness - 1.0).exp2();
    ((nov + ao).powf(exponent) - 1.0 + ao).clamp(0.0, 1.0)
}

/// PBR P1: the Blinn-Phong-equivalent specular exponent from the GGX alpha (roughness^2) —
/// mirrors the resolve's `sun_kernel_exponent`. `n = 2/alpha^2 - 2` (the standard Phong<->GGX
/// exponent conversion) blows up as `alpha -> 0` (a mirror-smooth surface), so it is clamped
/// to [`SUN_KERNEL_EXPONENT_MIN`, `SUN_KERNEL_EXPONENT_MAX`]: a smooth metal (low alpha) gets a
/// tight, sharp sun disc; a rough metal (`alpha -> 1`) gets a broad, soft glint (`n -> 0`,
/// floored at 1).
#[inline]
pub(crate) fn sun_kernel_exponent(alpha: f32) -> f32 {
    let n = 2.0 / (alpha * alpha).max(1e-6) - 2.0;
    n.clamp(SUN_KERNEL_EXPONENT_MIN, SUN_KERNEL_EXPONENT_MAX)
}

/// PBR P1: the analytic HDR sun-disc kernel — mirrors the resolve's `sun_kernel`:
/// `pow(saturate(dot(dir, sun_dir)), sun_kernel_exponent(alpha))`. `dir` is the REFLECTION
/// vector `R`; `sun_dir` is the directional light's unit direction `l`.
#[inline]
pub(crate) fn sun_kernel(dir: [f32; 3], sun_dir: [f32; 3], alpha: f32) -> f32 {
    let c = v_dot(dir, sun_dir).clamp(0.0, 1.0);
    c.powf(sun_kernel_exponent(alpha))
}

/// The 3x3 matrices of the Stephen Hill ACES-fitted tonemap (PBR P0-C), byte-mirroring the
/// resolve's `ACES_IN` — row-major as written, `mul(M, v)`-style (row i dotted with `v`).
const ACES_IN: [[f32; 3]; 3] = [
    [0.59719, 0.35458, 0.04823],
    [0.07600, 0.90834, 0.01566],
    [0.02840, 0.13383, 0.83777],
];

/// The output-side matrix of the Hill ACES fit (PBR P0-C), byte-mirroring the resolve's
/// `ACES_OUT`.
const ACES_OUT: [[f32; 3]; 3] = [
    [1.60475, -0.53108, -0.07367],
    [-0.10208, 1.10813, -0.00605],
    [-0.00327, -0.07276, 1.07602],
];

/// `mul(m, v)` mirror (row-major `m`, row `i` dotted with `v`, accumulated in column order
/// 0,1,2 — no reassociation) — the exact op-order the resolve's `aces_fitted` uses for both
/// `ACES_IN` and `ACES_OUT`.
#[inline]
fn mat3_mul_vec3(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// The Stephen Hill ACES-fitted filmic tonemap (PBR P0-C), byte-mirroring the resolve's
/// `aces_fitted`: `c = mul(ACES_IN, c); a = c*(c+0.0245786)-0.000090537; b =
/// c*(0.983729*c+0.432951)+0.238081; return saturate(mul(ACES_OUT, a/b));` — SAME op order,
/// no reassociation.
#[inline]
pub(crate) fn aces_fitted(c: [f32; 3]) -> [f32; 3] {
    let c = mat3_mul_vec3(&ACES_IN, c);
    let a = [
        c[0] * (c[0] + 0.0245786) - 0.000090537,
        c[1] * (c[1] + 0.0245786) - 0.000090537,
        c[2] * (c[2] + 0.0245786) - 0.000090537,
    ];
    let b = [
        c[0] * (0.983729 * c[0] + 0.432_951) + 0.238081,
        c[1] * (0.983729 * c[1] + 0.432_951) + 0.238081,
        c[2] * (0.983729 * c[2] + 0.432_951) + 0.238081,
    ];
    let ratio = [a[0] / b[0], a[1] / b[1], a[2] / b[2]];
    let out = mat3_mul_vec3(&ACES_OUT, ratio);
    [out[0].clamp(0.0, 1.0), out[1].clamp(0.0, 1.0), out[2].clamp(0.0, 1.0)]
}

/// The resolve's OUTPUT stage (PBR P0-C): the Hill ACES-fitted tonemap applied to the
/// exposed linear radiance, THEN the manual gamma-2.2 OETF (`gLit`/the swapchain are linear
/// UNORM end to end — no hardware sRGB encode; see the resolve's `aces_fitted` doc comment for
/// the OETF verification). Byte-mirrors `lit = aces_fitted(lit); lit = pow(lit, 1.0/2.2);`
/// EXACTLY (same op order, same two-step sequence).
#[inline]
pub(crate) fn tonemap_and_oetf(lit: [f32; 3]) -> [f32; 3] {
    const OETF_GAMMA_EXP: f32 = 1.0 / 2.2;
    let t = aces_fitted(lit);
    [
        t[0].powf(OETF_GAMMA_EXP),
        t[1].powf(OETF_GAMMA_EXP),
        t[2].powf(OETF_GAMMA_EXP),
    ]
}

/// The resolve's PROCEDURAL SKY BACKGROUND (mask == 0 pixels): mirrors the shader's
/// background branch op-order EXACTLY — scan the light table's L0a front block for a SKY
/// entry (`kind == Sky`, the SAME block the LIT arm's ambient loop reads) and fold in every
/// DIRECTIONAL light's fixed-exponent sun disc (`pow(saturate(dot(rd, l)), SKY_SUN_EXPONENT)`,
/// accumulated in TABLE order), then `sky = lerp(ground, sky, saturate(dot(rd, UP)*0.5+0.5));
/// sky += sun_disc; sky *= header.exposure();` (the FINAL multiply, O3), then
/// [`tonemap_and_oetf`]. Returns `None` when the table carries NO sky entry — the caller keeps
/// the dark pass-through (a scene without a SkyLight has no sky to render), matching the
/// shader's `has_sky` gate.
fn golden_sky_background(rd: [f32; 3], header: &GoldenLightHeader, lights: &[GoldenLight]) -> Option<u32> {
    let count = header.l0a_count() as usize;
    let mut sky_color: Option<[f32; 3]> = None;
    let mut ground_color = [0.0_f32; 3];
    let mut sun_disc = [0.0_f32; 3];
    for li in lights.iter().take(count) {
        match li.kind() {
            GOLDEN_LIGHT_KIND_SKY => {
                sky_color = Some([li.color_cone[0], li.color_cone[1], li.color_cone[2]]);
                ground_color = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
            }
            GOLDEN_LIGHT_KIND_DIRECTIONAL => {
                let l = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
                let k = v_dot(rd, l).clamp(0.0, 1.0).powf(SKY_SUN_EXPONENT);
                sun_disc[0] += li.color_cone[0] * k;
                sun_disc[1] += li.color_cone[1] * k;
                sun_disc[2] += li.color_cone[2] * k;
            }
            _ => {}
        }
    }
    let sky_color = sky_color?;
    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    let hemi = (v_dot(rd, UP) * 0.5 + 0.5).clamp(0.0, 1.0);
    let exposure = header.exposure();
    let sky = [
        (ground_color[0] + (sky_color[0] - ground_color[0]) * hemi + sun_disc[0]) * exposure,
        (ground_color[1] + (sky_color[1] - ground_color[1]) * hemi + sun_disc[1]) * exposure,
        (ground_color[2] + (sky_color[2] - ground_color[2]) * hemi + sun_disc[2]) * exposure,
    ];
    Some(pack_rgba(tonemap_and_oetf(sky)))
}

/// The per-pixel G-buffer attributes the PBR MVP-2 marcher writes, modelling the EXACT GPU
/// UNORM pack so [`golden_deferred_resolve`] can re-decode them and run the host BRDF
/// within ±2/255 of the GPU. On the mask == 0 arms (mesh / background / empty) `base_rgb`
/// is the RAW quantized base — a table WITHOUT a SKY entry keeps the byte-identical
/// pass-through of this field (the 0%-gate); with a SKY entry the resolve renders the
/// procedural sky over it instead (see [`golden_deferred_resolve`]'s doc).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarcherAttributes {
    /// gAlbedo R8G8B8: the RAW LINEAR base color (the picked material's `base_color.rgb`
    /// on an SDF hit, else MESH_COLOR / BACKGROUND), quantized via [`pack_rgba`] rounding.
    pub base_rgb: [u8; 3],
    /// gNormal R8G8: the octahedral-encoded world normal (SDF hit only; neutral otherwise).
    pub oct_rg: [u8; 2],
    /// gNormal B8/A8: the 16-bit material id packed low-byte → B, high-byte → A.
    pub mat_id: u16,
    /// gMaterial.r R8: the A1 soft-shadow visibility `round(255*clamp(shadow))`.
    pub shadow: u8,
    /// gMaterial.g R8: the A2 AO factor `round(255*clamp(ao))`.
    pub ao: u8,
    /// gMaterial.b decoded: 1 on the SDF-LIT arm, 0 on mesh / background / empty.
    pub mask: u8,
    /// Lighting L0b: the `gViewT` lane — the marcher's surface ray param `t` (the REAL
    /// marched `t` on the SDF-lit arm, the `1.0e30` sentinel on mesh / background / empty,
    /// mirroring the GPU's three terminal write sites, C2). The resolve oracle reconstructs
    /// `P = ro + rd * view_t` under `mask == 1` (the read-under-mask gate — the sentinel is
    /// never consumed on a non-lit pixel).
    pub view_t: f32,
}

/// Picks the nearest-surface material id at `p` by an argmin over the edit list, mirroring
/// the marcher's `pick_material_id` (the FROZEN `edit_distance` per primitive; the id from
/// the per-edit `center.w` free lane). Returns the default id (0) for an empty list.
pub(crate) fn pick_material_id(edits: &[SdfEdit], p: [f32; 3]) -> u16 {
    // The ≤16 scene contract: every committed scene fits the fixed cap, so this only
    // documents the invariant the marcher relies on (it never reads beyond the cap).
    debug_assert!(
        edits.len() <= MAX_SDF_EDITS,
        "invariant: edit count {} exceeds MAX_SDF_EDITS {MAX_SDF_EDITS}",
        edits.len()
    );
    // FAR (= 1e9), mirroring the shader's `FAR` sentinel exactly — see `PBR_FAR`.
    let mut best_d = PBR_FAR;
    let mut best_id = 0u16;
    // Clamp to the first MAX_SDF_EDITS edits: the GPU marcher iterates only
    // `min(Buf[0], MAX_SDF_EDITS)` candidates, so the host argmin must see the SAME
    // candidate set or the picked id (and thus gAlbedo) would diverge for >16 edits.
    for e in edits.iter().take(MAX_SDF_EDITS) {
        let d = edit_distance(e, p).abs();
        if d < best_d {
            best_d = d;
            best_id = (e.center[3].to_bits() & 0xFFFF) as u16;
        }
    }
    best_id
}

/// The CPU mirror of the PBR MVP-2 marcher's per-pixel ATTRIBUTE output. Runs the SAME
/// extent/camera ray-gen + over-relaxation march + arm selection as
/// [`golden_composite_pixel_ex_omega_lit`], then writes the repacked G-buffer attributes:
/// gAlbedo = the picked material's RAW LINEAR base color (via `materials`, indexed by the
/// argmin id), gNormal = (oct normal, 16-bit id), gMaterial = (shadow, ao, mask). On
/// mesh / background it emits the flat constant with mask = 0. `materials` is the host
/// material table; an out-of-range id falls back to the default material.
#[allow(clippy::too_many_arguments)]
pub fn golden_marcher_attributes(
    edits: &[SdfEdit],
    materials: &[GoldenMaterial],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
) -> MarcherAttributes {
    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    // The over-relaxation march + the Candidate-C re-march, mirroring
    // `golden_composite_pixel_ex_omega_lit` EXACTLY (the field/march is untouched).
    let mut t = 0.0_f32;
    let t_seed = t;
    let mut omega = omega;
    let mut hit = false;
    let mut safe_t = 0.0_f32;
    let mut sor_prev = 0.0_f32;
    let mut sor_step_prev = 0.0_f32;
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false;
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false;
            // BUG-B1-HOLE-4 (silhouette dark-ring) — re-seed the refine at `safe_t` on an
            // over-relaxed OVERSHOOT (d < 0). The signed refine converges to the NEAREST surface
            // from its seed; seeded at the overshot (inside) `t` near the silhouette it can settle
            // on the FAR surface → wrong normal → a thin dark ring at the grazing band. `safe_t`
            // (the last outside probe) forward-traces to the NEAR surface. Unreachable at omega == 1
            // (no overshoot → d >= 0 here) → the omega == 1 output is byte-unchanged (the 0%-gate).
            // Mirrors the shader's hand-written retreat.
            if d < 0.0 {
                t = safe_t;
            }
            // B1 over-relaxation accept-refine — the HOST MIRROR of the shader's analytic accept
            // (`sdf_gbuffer_composite.hlsl`). `d < SDF_EPS` is a one-sided upper bound: an
            // over-relaxed step (`omega > 1`) can overshoot DEEP inside the surface in one stride,
            // so the accepted `d` may be large-negative and the committed `t` would sit ~δ inside
            // the field → `host_soft_shadow` / `host_ao` sample inside → shadow == ao == 0 → BLACK.
            // Mirror the brick `host_m2_surface_hit` signed refine: the signed under-relaxed step
            // `t += M2_REFINE_RELAX * d` walks BACKWARD for `d < 0` (toward the surface) and
            // forward for `d > 0`; accept on `d.abs() < SDF_EPS`. The plain arm (omega == 1) is
            // sphere-traced from outside so its accept `d` is in `[0, SDF_EPS)` — the first
            // iteration accepts immediately (the omega==1 `t` is byte-unchanged, the 0%-gate).
            for _ri in 0..M2_REFINE_ITERS {
                let q = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let rd_ = sdf_edit_list(edits, q);
                if rd_.abs() < SDF_EPS {
                    break;
                }
                // Split the under-relaxed step into a named value then add it (no FMA contraction),
                // so the shader's `step = M2_REFINE_RELAX * rd_; t += step;` rounds bit-identically.
                let step = M2_REFINE_RELAX * rd_;
                t += step;
            }
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                t = safe_t + sor_prev;
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d;
        }
        if t > SDF_T_MAX {
            exhausted = false;
            break;
        }
    }
    if exhausted {
        t = t_seed;
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d;
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    // Quantize a `[0,1]` scalar to a byte with the GPU UNORM store's `(x*255+0.5)` rounding.
    let q8 = |x: f32| -> u8 { (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 };
    // The R8G8B8 bytes a `pack_rgba` of `c` would store (low 3 bytes of `0xAABBGGRR`).
    let base_bytes = |c: [f32; 3]| -> [u8; 3] {
        let packed = pack_rgba(c);
        [
            (packed & 0xFF) as u8,
            ((packed >> 8) & 0xFF) as u8,
            ((packed >> 16) & 0xFF) as u8,
        ]
    };

    if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);

        // ATTRIBUTE the hit to the nearest edit's material, then take its RAW LINEAR base
        // color — gAlbedo carries NO lighting (the resolve runs the full BRDF).
        let mat_id = pick_material_id(edits, p);
        let mat = materials
            .get(mat_id as usize)
            .copied()
            .unwrap_or_default();
        let base = [mat.base_color[0], mat.base_color[1], mat.base_color[2]];

        // The A1/A2 marches, gated by `lighting_flags` (kept SEPARATE: shadow → R, ao → G).
        let (shadow, ao) = if lighting_flags == 0 {
            (1.0_f32, 1.0_f32)
        } else {
            let l = v_normalize(light_dir);
            let field = |q: [f32; 3]| sdf_edit_list(edits, q);
            let mut shadow = 1.0_f32;
            if lighting_flags & LIGHTING_FLAG_SHADOWS != 0 {
                // Normal-offset start bias (mirrors the marcher's
                // `sdf_soft_shadow(p + n*SHADOW_NORMAL_BIAS, n, light)`).
                let pb = [
                    p[0] + n[0] * SHADOW_NORMAL_BIAS,
                    p[1] + n[1] * SHADOW_NORMAL_BIAS,
                    p[2] + n[2] * SHADOW_NORMAL_BIAS,
                ];
                shadow = host_soft_shadow(pb, n, l, &field);
            }
            let mut ao = 1.0_f32;
            if lighting_flags & LIGHTING_FLAG_AO != 0 {
                ao = host_ao(p, n, &field);
            }
            (shadow.clamp(0.0, 1.0), ao.clamp(0.0, 1.0))
        };

        let oct = oct_encode(n);
        MarcherAttributes {
            base_rgb: base_bytes(base),
            oct_rg: [q8(oct[0]), q8(oct[1])],
            mat_id,
            shadow: q8(shadow),
            ao: q8(ao),
            mask: 1,
            // Lighting L0b: the SDF-lit arm stores the REAL marched `t` (the same `t` the
            // hit point `p = ro + rd * t` above used) — the resolve reconstructs `P` from it.
            view_t: t,
        }
    } else if has_mesh {
        // Render P5 (r0+r1): a mesh-covered pixel the SDF did NOT win is RASTER-OWNED
        // (`!own_pixel` in `sdf_gbuffer_composite.hlsl`). The raster pass (`gbuffer_mrt.fs`)
        // is the SINGLE producer: it writes a first-class PBR G-buffer with `mask = 1`, so
        // the deferred resolve runs FULL Cook-Torrance on the mesh pixel — identical to an
        // SDF pixel — NOT the pre-P5 flat MESH_COLOR pass-through. Model that producer here:
        //   - gAlbedo = saturate(LINEAR vertex color) = MESH_RASTER_ALBEDO (the harness quad
        //     is white; `saturate` is the identity).
        //   - gNormal = oct_encode((0, 0, 1)) (the fronto-parallel quad's +Z) + mat_id 0.
        //   - gMaterial = (shadow, ao, mask = 1) — Render P7-r2: the SDF now casts a CLEAN
        //     ANALYTIC soft shadow + contact AO onto the mesh (the marcher's `else if (has_mesh)`
        //     arm marches `sdf_soft_shadow`/`sdf_ao` over the FROZEN field, the noise-free
        //     SDF-native replacement for the screen-space SSAO). Mirror that march here.
        //   - gViewT: Render P7/P5-r1b UNLOCK — the marcher writes the mesh surface ray-t
        //     `t_mesh` (= `depth_to_t(mesh_depth)`, the same value the ownership gate marched
        //     against) for a `!own_pixel` mesh pixel, NOT the old `1.0e30` sentinel. The resolve
        //     reconstructs the REAL mesh surface position `P = ro + rd * t_mesh`, so in-range
        //     point/spot lights now light the mesh (instead of being range-culled at infinity).
        //     Mirror that with `t_mesh` so the host oracle == the GPU on every equivalence golden.
        //
        // The harness quad is fronto-parallel (+Z); the GPU reads the raster normal back from
        // gNormal and `oct_decode`s it, and the oct round-trip of (0,0,1) is EXACT, so the host
        // normal == the GPU's decoded normal bit-for-bit. `P_mesh = ro + rd * t_mesh` mirrors the
        // marcher's reconstruct.
        let n = [0.0_f32, 0.0, 1.0];
        let oct = oct_encode(n);
        let p_mesh = [
            ro[0] + rd[0] * t_mesh,
            ro[1] + rd[1] * t_mesh,
            ro[2] + rd[2] * t_mesh,
        ];
        let (shadow, ao) = if lighting_flags == 0 {
            (1.0_f32, 1.0_f32)
        } else {
            let l = v_normalize(light_dir);
            let field = |q: [f32; 3]| sdf_edit_list(edits, q);
            let mut shadow = 1.0_f32;
            if lighting_flags & LIGHTING_FLAG_SHADOWS != 0 {
                let pb = [
                    p_mesh[0] + n[0] * SHADOW_NORMAL_BIAS,
                    p_mesh[1] + n[1] * SHADOW_NORMAL_BIAS,
                    p_mesh[2] + n[2] * SHADOW_NORMAL_BIAS,
                ];
                shadow = host_soft_shadow(pb, n, l, &field);
            }
            let mut ao = 1.0_f32;
            if lighting_flags & LIGHTING_FLAG_AO != 0 {
                ao = host_ao(p_mesh, n, &field);
            }
            (shadow.clamp(0.0, 1.0), ao.clamp(0.0, 1.0))
        };
        MarcherAttributes {
            base_rgb: base_bytes(MESH_RASTER_ALBEDO),
            oct_rg: [q8(oct[0]), q8(oct[1])],
            mat_id: 0,
            shadow: q8(shadow),
            ao: q8(ao),
            mask: 1,
            view_t: t_mesh,
        }
    } else {
        // Pure background / empty (NO mesh, SDF missed): mask == 0 pass-through. gNormal/id/
        // shadow/ao are unread by the resolve; model the marcher's neutral defaults so the
        // attribute struct round-trips deterministically.
        MarcherAttributes {
            base_rgb: base_bytes(SDF_BACKGROUND),
            oct_rg: [q8(0.5), q8(0.5)],
            mat_id: 0,
            shadow: 255,
            ao: 255,
            mask: 0,
            // Lighting L0b: the background / empty arm stores the `1.0e30` sentinel
            // (the GPU's mask == 0 write); never read on a non-lit pixel (read-under-mask).
            view_t: 1.0e30,
        }
    }
}

/// Render P7 GROUP B: the ONE forward SSAO horizon tap — the eDSL-GENERATED span
/// (`// === GENERATED ssao_horizon_step BEGIN/END ===` in `shaders/sdf_ssao.comp.hlsl`)
/// re-derived as plain Rust. Factored out (matching the leaf shape of
/// `boyko_shaderdsl::ssao::ssao_horizon_step_body`) so the `ssao_edsl_sync` cross-check can
/// assert this host math == the eDSL `<EvalCf>` Eval before any GPU run.
///
/// Accumulates one tapped neighbour world position `pp` (`P'`, supplied by the forward-
/// reconstruct seam in [`golden_ssao_attributes`]) into the running per-half-slice horizon
/// max `hc`. `p` is the center world position (`P`), `n` the CENTER SURFACE NORMAL (`N`, the
/// elevation reference). The HBAO horizon step measures the neighbour's ELEVATION ABOVE THE
/// TANGENT PLANE (NO `sin`/`cos`/`acos`, NO `fract` — bit-comparable to the GPU):
///   `delta   = P' - P`
///   `falloff = clamp01(1 - dot(delta,delta) / (R*R))`   (the squared-distance range gate)
///   `elev    = max(dot(delta, N) / max(length(delta), SSAO_EPS), 0.0)`
///   `hc      = max(hc, elev * falloff)`
/// A flat surface (`delta ⊥ N` → `elev = 0`) raises no horizon (AO = 1); a crevice
/// (neighbours rising above the tangent → `dot(delta,N) > 0`) does (AO < 1). `dot` is INLINE
/// component-reads + mul/add; `length = sqrt(dot(delta,delta))`.
///
/// Render P7-Q2: `params` carries the active variant's `radius`/`eps` (the only two scalars this tap
/// reads). The arithmetic is UNCHANGED — feeding [`SsaoParams::default`] (== the Medium row == the
/// module `SSAO_RADIUS`/`SSAO_EPS` consts) reproduces the pre-Q2 result bit-for-bit. The variant
/// `.spv` spells `SSAO_RADIUS`/`SSAO_EPS` symbolically (the baked `static const` header supplies the
/// values), so this host scalar swap mirrors the GPU's swapped header exactly.
pub fn ssao_horizon_step(hc: f32, p: [f32; 3], pp: [f32; 3], n: [f32; 3], params: &SsaoParams) -> f32 {
    // float3 delta = P' - P;
    let delta = [pp[0] - p[0], pp[1] - p[1], pp[2] - p[2]];
    // dot(delta, delta) (INLINE).
    let d2 = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
    // float r2 = SSAO_RADIUS * SSAO_RADIUS;  (a named temp — the divisor prints `d2 / r2`,
    // NOT the precedence-wrong `(d2 / R) * R`; mirror the eDSL `temp_float("r2", ...)`).
    let r2 = params.radius * params.radius;
    // float falloff = clamp(1.0 - d2 / r2, 0.0, 1.0);
    let falloff = (1.0 - d2 / r2).clamp(0.0, 1.0);
    // dot(delta, N) (INLINE) — the unnormalized elevation against the center surface normal.
    let dn = delta[0] * n[0] + delta[1] * n[1] + delta[2] * n[2];
    // float elev = max(dot(delta, N) / max(sqrt(d2), SSAO_EPS), 0.0);  the sine of the
    // elevation above the tangent, clamped non-negative (neighbours below the tangent do not
    // occlude). `length(delta) = sqrt(d2)` reuses the `d2` value so the sqrt operand is
    // bit-identical to the GPU.
    let elev = (dn / d2.sqrt().max(params.eps)).max(0.0);
    // hc = max(hc, elev * falloff);
    hc.max(elev * falloff)
}

/// Render P7 GROUP B: Stage-1 of the SSAO host oracle — the FROZEN G-buffer the SSAO gather
/// reads, built ONCE by mapping [`golden_marcher_attributes`] over every pixel of the
/// `img_w × img_h` extent (row-major, `idx = py * img_w + px`, the SAME dispatch index the
/// shader's `idx % w` / `idx / w` decode). [`golden_ssao_attributes`] reads `mask` +
/// `view_t` + the center pixel's `oct_rg` normal from this buffer. The per-pixel
/// arguments match `golden_marcher_attributes` exactly — the scene (`edits`/`mesh_depth`),
/// the marcher tuning (`omega`), and the lighting gate (`lighting_flags`/`light_dir`) are
/// the same across the whole frame. `mesh_depth_of(px, py)` supplies the per-pixel mesh
/// depth (the raster producer's covered-pixel depth, or `MESH_DEPTH_CLEAR` for none).
#[allow(clippy::too_many_arguments)]
pub fn golden_gbuffer<M: Fn(u32, u32) -> f32>(
    edits: &[SdfEdit],
    materials: &[GoldenMaterial],
    mesh_depth_of: M,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
) -> Vec<MarcherAttributes> {
    let mut gbuf = Vec::with_capacity((img_w as usize) * (img_h as usize));
    for py in 0..img_h {
        for px in 0..img_w {
            gbuf.push(golden_marcher_attributes(
                edits,
                materials,
                mesh_depth_of(px, py),
                px,
                py,
                img_w,
                img_h,
                camera,
                omega,
                lighting_flags,
                light_dir,
            ));
        }
    }
    gbuf
}

/// The locality-preserving 2D->1D Hilbert index over an `n x n` tile (`n` a power of two) — the
/// host mirror of the shader's `ssao_hilbert(uint, uint, uint)`. Pure integer bit-twiddling (the
/// canonical `xy2d`), so it is bit-exact vs the SPIR-V `uint` arithmetic.
///
/// Replaces the prior per-pixel WHITE-NOISE PCG hash. White noise has energy at ALL spatial
/// frequencies, so the depth-aware blur (a low-pass) strips the highs but leaves low-frequency
/// noisy BLOBS (the "pixelated" look). Driving Martin Roberts' R2 sequence from a Hilbert index
/// is a LOW-DISCREPANCY / blue-noise basis (the XeGTAO no-TAA recipe) whose energy is HIGH
/// frequency only — the SAME blur removes it cleanly.
#[inline]
pub(crate) fn ssao_hilbert(n: u32, mut x: u32, mut y: u32) -> u32 {
    let mut d = 0u32;
    let mut s = n / 2;
    while s > 0 {
        let rx = if (x & s) > 0 { 1u32 } else { 0 };
        let ry = if (y & s) > 0 { 1u32 } else { 0 };
        d = d.wrapping_add(s.wrapping_mul(s).wrapping_mul((3u32.wrapping_mul(rx)) ^ ry));
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    d
}

/// The R2 fractional part in Q0.24 fixed point (the dither value) — the host mirror of the
/// shader's `ssao_r2(uint, uint)`. `(index * alpha) & 0xFFFFFF` is `frac(index * alpha)` in Q0.24;
/// the `u32` multiply wraps mod 2^32 identically on host and GPU (the low 24 bits are untouched by
/// the wrap), so there is NO `frac`/trig and the host oracle and GPU pick the SAME dither bit-for-bit.
#[inline]
pub(crate) fn ssao_r2(index: u32, alpha: u32) -> u32 {
    index.wrapping_mul(alpha) & 0x00FF_FFFF
}

/// Render P7 GROUP B: the host oracle for one SSAO output pixel — the line-for-line plain-
/// Rust translation of `shaders/sdf_ssao.comp.hlsl`'s `main()` `center_lit` path. Returns
/// the AO factor in `[0, 1]` (`1.0` = unoccluded; the resolve combines via
/// `min(class_ao, ssao)`). `gbuf` is the Stage-1 G-buffer ([`golden_gbuffer`], row-major).
///
/// The HBAO-lite reducer (NO trig, NO `fract`): for each of [`SSAO_SLICES`] rotated screen-
/// space slices, march [`SSAO_STEPS`] forward-projected neighbour taps in each `±dir` half-
/// slice tracking the horizon max via [`ssao_horizon_step`], fold the two half-slice maxes
/// into a per-slice occlusion, sum, then complement + square. A non-lit center pixel
/// (`mask <= 0.5 || view_t >= SSAO_VIEWT_BG`) returns the neutral `1.0`. The neighbour world
/// position is reconstructed FORWARD via the SAME [`composite_ray`] the marcher uses (no
/// proj-matrix inverse); an out-of-bounds or non-lit tap reconstructs `Pp = P` (zero
/// contribution). The rotation slot is the INTEGER hash (bit-exact vs the GPU `uint`).
///
/// Render P7-Q2: `params` selects the quality variant the GPU bound — its `radius`/`slices`/`steps`/
/// `strength`/`eps` REPLACE the module `SSAO_*` consts in the gather (the pix-radius, the slice/step
/// loop bounds, the `occ → ao` strength/slice-divisor fold, and the per-tap [`ssao_horizon_step`]).
/// The arithmetic is UNCHANGED: feeding [`SsaoParams::default`] (== `SSAO_PARAMS[SSAO_QUALITY_MEDIUM]`
/// == today's shipped consts) reproduces the pre-Q2 golden BIT-FOR-BIT. Feed `SSAO_PARAMS[q]` to
/// mirror the variant `q` `.spv`.
pub fn golden_ssao_attributes(
    gbuf: &[MarcherAttributes],
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    params: &SsaoParams,
) -> f32 {
    debug_assert_eq!(
        gbuf.len(),
        (img_w as usize) * (img_h as usize),
        "invariant: SSAO gbuf length must equal img_w * img_h"
    );
    // Read the center pixel's class. `mask == 1` is stored as a byte; test the decoded flag.
    let center = gbuf[(py as usize) * (img_w as usize) + (px as usize)];
    let center_lit = center.mask > 0 && center.view_t < SSAO_VIEWT_BG;
    if !center_lit {
        // A non-lit pixel carries no surface — the neutral factor; `min(class_ao, ssao)`
        // leaves it unchanged.
        return 1.0;
    }

    // Reconstruct the center world position P = ro + rd * view_t via the shared ray-gen.
    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);
    let view_t = center.view_t;
    let p = [
        ro[0] + rd[0] * view_t,
        ro[1] + rd[1] * view_t,
        ro[2] + rd[2] * view_t,
    ];

    // Decode the center surface normal ONCE (the same `oct_decode` the resolve mirror uses on
    // `gNormal.rg`). The horizon step measures each neighbour's elevation above the tangent
    // plane this normal defines — CONSTANT across all slices/taps.
    let center_n = oct_decode([
        center.oct_rg[0] as f32 / 255.0,
        center.oct_rg[1] as f32 / 255.0,
    ]);

    // The screen-pixel march radius (clamped band on PERSPECTIVE; the fixed ortho span).
    let pix_radius = match camera {
        CompositeCamera::Perspective {
            forward,
            tan_half_fov,
            ..
        } => {
            // z = max(dot(rd, cam_forward.xyz) * view_t, 1e-3).
            let z_view = rd[0] * forward[0] + rd[1] * forward[1] + rd[2] * forward[2];
            let z = (z_view * view_t).max(1.0e-3);
            // pr = R * (h/2) / (z * tan(fovY/2)); clamp(pr, MIN, MAX).
            let pr = params.radius * ((img_h as f32) * 0.5) / (z * tan_half_fov);
            pr.clamp(SSAO_RADIUS_PIX_MIN, SSAO_RADIUS_PIX_MAX)
        }
        CompositeCamera::Ortho => {
            // The view maps the [-HALF_EXTENT, HALF_EXTENT] span across h/2 pixels, so R spans
            // R*(h/2)/HALF_EXTENT. `SDF_HALF_EXTENT` IS the shader's `RAYGEN_HALF_EXTENT` (the
            // ortho half-extent `composite_ray` already uses — reuse it, do NOT hardcode a copy).
            params.radius * ((img_h as f32) * 0.5) / SDF_HALF_EXTENT
        }
    };

    // The Hilbert+R2 rotation slot (bit-exact vs HLSL — INTEGER pick, no div): ONE 64x64
    // Hilbert index drives two R2 channels; `slot = (r2 * SSAO_ROT_N) >> 24` maps the Q0.24
    // fraction into [0, ROT_N). The table is 64 entries (was 16): an even-slice axis set has
    // only `SSAO_ROT_N / slices` EFFECTIVE dither classes (rotating the set by its slice
    // spacing maps it onto itself) — 16 entries left 2 classes at 8 slices, whose coherent
    // layout read as un-blurrable streaks; 64 keeps >= 8 classes at a 2.8125° step.
    let hindex = ssao_hilbert(SSAO_HILBERT_W, px & (SSAO_HILBERT_W - 1), py & (SSAO_HILBERT_W - 1));
    let slot = ((ssao_r2(hindex, SSAO_R2_ALPHA1).wrapping_mul(SSAO_ROT_N)) >> 24) as usize;
    let rot = SSAO_ROT[slot];

    // The radial step-phase jitter from the SECOND R2 channel (mirror the shader): the top 8 bits
    // of the Q0.24 fraction map to [1, 256] -> [1/256, 1.0]. Strictly positive ⇒ the nearest tap
    // never self-samples the center; at phase 1.0 the farthest tap reaches exactly `pix_radius`.
    // Integer + one exact `/256.0` ⇒ bit-exact.
    let r2_rad = ssao_r2(hindex, SSAO_R2_ALPHA2);
    let radial_phase = ((r2_rad >> 16) + 1) as f32 / 256.0;

    // The forward neighbour reconstruct (the hand-written seam): offset in SCREEN pixels along
    // `sdir2 * sign`, round, bounds-clamp, reconstruct Pp = nro + nrd * nview_t; else Pp = P.
    let reconstruct = |sdir2: (f32, f32), advance: f32, sign: f32| -> [f32; 3] {
        let npx = ((px as f32) + sign * sdir2.0 * advance).round() as i32;
        let npy = ((py as f32) + sign * sdir2.1 * advance).round() as i32;
        if npx >= 0 && npy >= 0 && npx < (img_w as i32) && npy < (img_h as i32) {
            let n = gbuf[(npy as usize) * (img_w as usize) + (npx as usize)];
            if n.mask > 0 && n.view_t < SSAO_VIEWT_BG {
                let (nro, nrd) = composite_ray(npx as u32, npy as u32, img_w, img_h, camera);
                return [
                    nro[0] + nrd[0] * n.view_t,
                    nro[1] + nrd[1] * n.view_t,
                    nro[2] + nrd[2] * n.view_t,
                ];
            }
        }
        p
    };

    // The variant tap budget — the slice/step `[unroll]` bounds the matching `.spv` bakes.
    let steps_f = params.steps as f32;
    let mut occ = 0.0_f32;
    for sl in 0..params.slices {
        // The base slice axis (Change A): slice `s` at angle `s*(pi/N)` == `SSAO_ROT[s*STRIDE]`,
        // the EXACT `SSAO_ROT[sl * (SSAO_ROT_N / SSAO_SLICES)]` the shader bakes (evenly-spaced
        // real slices; the pre-A code hardcoded only 2 axes). `SSAO_SLICES` must divide
        // `SSAO_ROT_N` for exact spacing (asserted). The 2D screen axis picks the neighbour PIXEL
        // (the tap offset); the horizon math measures elevation against the center normal, NOT this.
        debug_assert_eq!(
            SSAO_ROT_N % params.slices,
            0,
            "invariant: SSAO_SLICES ({}) must divide SSAO_ROT_N ({SSAO_ROT_N}) for even slice spacing",
            params.slices
        );
        let base = SSAO_ROT[(sl * (SSAO_ROT_N / params.slices)) as usize];
        let sdir2 = (
            base.0 * rot.0 - base.1 * rot.1,
            base.0 * rot.1 + base.1 * rot.0,
        );

        // The + half-slice: `params.steps` forward taps, tracking the horizon max `hc`. The screen
        // offset advances along +sdir2; elevation is measured against the center normal.
        let mut hc_pos = 0.0_f32;
        for sp in 0..params.steps {
            let advance = (sp as f32 + radial_phase) * pix_radius / steps_f;
            let pp = reconstruct(sdir2, advance, 1.0);
            hc_pos = ssao_horizon_step(hc_pos, p, pp, center_n, params);
        }

        // The - half-slice: `params.steps` forward taps along the negated screen offset. Same
        // center normal (both half-slices measure elevation against the surface tangent).
        let mut hc_neg = 0.0_f32;
        for sn in 0..params.steps {
            let advance = (sn as f32 + radial_phase) * pix_radius / steps_f;
            let pp = reconstruct(sdir2, advance, -1.0);
            hc_neg = ssao_horizon_step(hc_neg, p, pp, center_n, params);
        }

        occ += hc_pos + hc_neg;
    }

    // The final occlusion complement (the eDSL `ssao_estimate` tail): the mean per-slice
    // horizon cosine scaled by strength, complemented, then squared (the integer self-mul). The
    // `occ / N` divisor reads `params.slices as f32` (the `SSAO_SLICES_F` the variant bakes).
    let ao = (1.0 - params.strength * occ / params.slices as f32).clamp(0.0, 1.0);
    ao * ao
}

/// Quantizes `v` (clamped to `[0,1]`) to an R16_UNORM code point — `(v * 65535).round()`,
/// matching the SSAO à-trous chain's INTERIOR ping-pong storage. Shared by [`golden_ssao_atrous`],
/// the GPU-vs-host test harness, and the `ssao_atrous_edsl_sync` Track-1 sync test (ONE
/// quantization convention — see the SSAO à-trous plan's "inter-pass precision" note). Round-half-up
/// matches NVIDIA/Vulkan UNORM store rounding on the non-negative `[0,1]` domain.
#[inline]
pub fn quantize_r16_unorm(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// Decodes an R16_UNORM code point back to `[0,1]` — the GPU's `Load` decode of an R16_UNORM
/// storage image (the SSAO à-trous chain's interior ping-pong read).
#[inline]
pub fn decode_r16_unorm(code: u16) -> f32 {
    f32::from(code) / 65535.0
}

/// Quantizes `v` (clamped to `[0,1]`) to an R8_UNORM code point — `(v * 255).round()`, matching
/// the SSAO à-trous chain's TWO FROZEN ENDPOINTS (the raw `sdf_ssao` gather output, the final
/// filtered `gSsao` the resolve reads). Shared with [`golden_ssao_attributes`]'s quantization
/// convention (byte-identical rounding).
#[inline]
pub fn quantize_r8_unorm(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Linear view-Z for a pixel, reconstructed BIT-CONSISTENT with `ssao_atrous.comp.hlsl` /
/// `shadow_atrous.comp.hlsl`'s `linear_view_z`: PERSPECTIVE `dot(rd, cam_forward) * view_t`,
/// ORTHO `view_t` (a verbatim no-op — the bit-exact SSAO test fixtures are all ORTHO, so this
/// switch from the raw ray-param `view_t` gate is numerically free there). `rd` is the pixel's
/// own ray direction (`composite_ray`'s second return) — NOT the center pixel's, for a
/// neighbour tap.
#[inline]
pub fn linear_view_z(camera: CompositeCamera, rd: [f32; 3], view_t: f32) -> f32 {
    match camera {
        CompositeCamera::Perspective { forward, .. } => {
            let z_view = rd[0] * forward[0] + rd[1] * forward[1] + rd[2] * forward[2];
            z_view * view_t
        }
        CompositeCamera::Ortho => view_t,
    }
}

/// ONE SSAO à-trous pass over the WHOLE image — the host oracle mirror of
/// `ssao_atrous.comp.hlsl`'s `main()` for a single dispatch (`step = 1 << level`). `cur` is the
/// previous pass's DECODED `[0,1]` buffer (row-major, `img_w * img_h`); `gbuf` supplies each
/// pixel's `view_t` for the linear-Z reconstruct. Returns the RAW (unquantized) filtered buffer —
/// [`golden_ssao_atrous`] applies the inter-pass quantization convention around each call.
fn ssao_atrous_pass(
    cur: &[f32],
    gbuf: &[MarcherAttributes],
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    step: i32,
) -> Vec<f32> {
    let w = img_w as i32;
    let h = img_h as i32;
    let idx_of = |x: i32, y: i32| -> usize { (y * w + x) as usize };
    let z_at = |x: i32, y: i32| -> f32 {
        let (_, rd) = composite_ray(x as u32, y as u32, img_w, img_h, camera);
        linear_view_z(camera, rd, gbuf[idx_of(x, y)].view_t)
    };

    let mut out = vec![0.0_f32; cur.len()];
    for py in 0..h {
        for px in 0..w {
            let s_c = cur[idx_of(px, py)];
            let z_c = z_at(px, py);

            // The slope-aware (plane-fit) depth-gate gradient — min-magnitude ONE-SIDED
            // linear-Z differences at the FIXED ±1 pixel offset (coordinate-clamped: an
            // image-edge "neighbour" reuses the border pixel — the shader's clamp).
            let cx = |dx: i32| (px + dx).clamp(0, w - 1);
            let cy = |dy: i32| (py + dy).clamp(0, h - 1);
            let z_xp = z_at(cx(1), py);
            let z_xm = z_at(cx(-1), py);
            let z_yp = z_at(px, cy(1));
            let z_ym = z_at(px, cy(-1));
            let min_mag = |a: f32, b: f32| -> f32 { if a.abs() > b.abs() { b } else { a } };
            let dzdx = min_mag(z_xp - z_c, z_c - z_xm)
                .clamp(-SSAO_BLUR_GRAD_CLAMP, SSAO_BLUR_GRAD_CLAMP);
            let dzdy = min_mag(z_yp - z_c, z_c - z_ym)
                .clamp(-SSAO_BLUR_GRAD_CLAMP, SSAO_BLUR_GRAD_CLAMP);

            let depth_sigma2 = SSAO_BLUR_DEPTH_SIGMA * SSAO_BLUR_DEPTH_SIGMA;
            let mut sum = 0.0_f32;
            let mut wsum = 0.0_f32;
            for oy in -2..=2i32 {
                for ox in -2..=2i32 {
                    let tx = (px + ox * step).clamp(0, w - 1);
                    let ty = (py + oy * step).clamp(0, h - 1);
                    let s = cur[idx_of(tx, ty)];
                    let z_t = z_at(tx, ty);
                    let h_weight = SSAO_ATROUS_H[(ox + 2) as usize] * SSAO_ATROUS_H[(oy + 2) as usize];
                    let dz_pred = dzdx * (ox * step) as f32 + dzdy * (oy * step) as f32;
                    let dz = z_t - z_c - dz_pred;
                    if dz.abs() > SSAO_BLUR_DEPTH_TOL {
                        continue; // silhouette gate — HARD reject
                    }
                    let w_depth = (1.0 - dz * dz / depth_sigma2).clamp(0.0, 1.0);
                    let weight = h_weight * w_depth;
                    sum += weight * s;
                    wsum += weight;
                }
            }
            out[idx_of(px, py)] = if wsum > SSAO_ATROUS_W_EPS { sum / wsum } else { s_c };
        }
    }
    out
}

/// The multi-pass SSAO à-trous edge-avoiding denoise chain — the host oracle mirror of the
/// SHIPPED `ssao_atrous.comp.hlsl` dispatched `levels` times (mirrors
/// `shadow_atrous.comp.hlsl`'s Dammertz filter, transcendental-free — the depth gate is the SAME
/// plane-fit residual + polynomial falloff the RETIRED inline resolve blur used, now gating
/// LINEAR-Z (see [`linear_view_z`]) instead of the raw `gViewT` ray-param). `raw_ssao` is the R8
/// gather output ([`golden_ssao_attributes`] quantized via [`quantize_r8_unorm`]); `levels == 0`
/// returns `raw_ssao` UNCHANGED (the denoise-off byte-identical path).
///
/// INTER-PASS QUANTIZATION (research-specified, ONE shared convention — see [`quantize_r16_unorm`]
/// / [`quantize_r8_unorm`]): every pass except the LAST rounds its output to R16_UNORM (the
/// interior ping-pong ring's physical format); the LAST pass rounds to R8_UNORM (the frozen
/// `gSsao` endpoint). This mirrors the actual GPU store/load precision loss between dispatches,
/// so the host and GPU results agree within the existing SSAO tolerance band.
pub fn golden_ssao_atrous(
    raw_ssao: &[u8],
    gbuf: &[MarcherAttributes],
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    levels: u32,
) -> Vec<u8> {
    let n = (img_w as usize) * (img_h as usize);
    debug_assert_eq!(raw_ssao.len(), n, "invariant: raw_ssao length must equal img_w * img_h");
    debug_assert_eq!(gbuf.len(), n, "invariant: gbuf length must equal img_w * img_h");

    if levels == 0 {
        return raw_ssao.to_vec();
    }

    let mut cur: Vec<f32> = raw_ssao.iter().map(|&b| f32::from(b) / 255.0).collect();
    for level in 0..levels {
        let step = 1i32 << level;
        let raw_next = ssao_atrous_pass(&cur, gbuf, img_w, img_h, camera, step);
        let is_last = level + 1 == levels;
        cur = raw_next
            .into_iter()
            .map(|v| {
                if is_last {
                    f32::from(quantize_r8_unorm(v)) / 255.0
                } else {
                    decode_r16_unorm(quantize_r16_unorm(v))
                }
            })
            .collect();
    }
    cur.iter().map(|&v| quantize_r8_unorm(v)).collect()
}

/// The CPU mirror of the `deferred_pbr` RESOLVE (PBR MVP-2): given the marcher's
/// [`MarcherAttributes`], the camera ray for the pixel, and the material table, returns
/// the packed `0xAABBGGRR` LIT color the resolve stores.
///
/// Models the EXACT GPU double quantization: the attributes are already R8-quantized; the
/// resolve loads them back (UNORM decode), decodes the oct normal + 16-bit id, fetches
/// `materials[id]`, and runs the SAME Cook-Torrance the resolve runs (GGX D + height-
/// correlated Smith V + Schlick F + Lambert + EnvBRDFApprox ambient; shadow modulates the
/// direct term. PBR metal fix: AO is DECOUPLED — diffuse ambient by `ao`, specular ambient
/// by the roughness-aware [`specular_ao`]), then re-quantizes via [`pack_rgba`]. On the mask == 0 arms
/// it renders the PROCEDURAL SKY BACKGROUND along `rd` (the constant-path defaults — one
/// directional + `ground == sky`, exposure 1.0 — reproduce
/// [`golden_deferred_resolve_table`]'s background arm fed the degenerate 0%-gate table
/// BYTE-FOR-BYTE). `rd` is the pixel's ray direction (the view dir is `-rd`); supply the
/// SAME `composite_ray` the marcher used.
pub fn golden_deferred_resolve(
    attrs: MarcherAttributes,
    rd: [f32; 3],
    materials: &[GoldenMaterial],
) -> u32 {
    golden_deferred_resolve_with_pbr(attrs, rd, materials, None)
}

/// Textured-PBR T6a: [`golden_deferred_resolve`] PLUS an OPTIONAL `gPbr` texel sample
/// (`[metallic, roughness, ao_modulation, emissive_modulation]`), mirroring the SOFTWARE
/// resolve's `MATERIAL_FLAG_TEXTURED_BIT` override (`deferred_pbr.hlsl`'s `#if !HWRT` block:
/// `metallic`/`roughness` are REPLACED, `ao` is MULTIPLIED, `emissive` is MULTIPLIED). The
/// override applies ONLY when BOTH `gpbr` is `Some` AND `mat.mrr[3]`'s bitcast flags carry
/// [`GOLDEN_MATERIAL_FLAG_TEXTURED`]. [`golden_deferred_resolve`] is a thin `None`-forwarding
/// wrapper, so EVERY existing call site is untouched; every EXISTING [`GoldenMaterial`] leaves
/// `mrr[3] == 0.0` ([`GoldenMaterial::new`]), so this override is a bit-identical no-op for
/// every current oracle input (the flag=0 byte-identity invariant T6a requires).
pub fn golden_deferred_resolve_with_pbr(
    attrs: MarcherAttributes,
    rd: [f32; 3],
    materials: &[GoldenMaterial],
    gpbr: Option<[f32; 4]>,
) -> u32 {
    let base = [
        attrs.base_rgb[0] as f32 / 255.0,
        attrs.base_rgb[1] as f32 / 255.0,
        attrs.base_rgb[2] as f32 / 255.0,
    ];
    if attrs.mask != 1 {
        // mesh / background / empty: render the SAME analytic sky the LIT arm's ambient
        // samples (PBR sky background) — the constant-path's degenerate defaults
        // (`ground == sky == PBR_SKY_DIFFUSE`) fold the hemisphere lerp to a flat no-op
        // (the SAME fold `golden_deferred_resolve_table`'s LIT-arm ambient uses, see its doc
        // comment), so only the sun disc varies with `rd`. No exposure multiply (this path's
        // implicit exposure is 1.0, matching `golden_deferred_resolve_table`'s degenerate
        // 0%-gate table — `x * 1.0 == x` exactly).
        let l = v_normalize(PBR_LIGHT_DIR);
        let sun_k = v_dot(rd, l).clamp(0.0, 1.0).powf(SKY_SUN_EXPONENT);
        let sky = [
            PBR_SKY_DIFFUSE[0] + PBR_LIGHT_COLOR[0] * sun_k,
            PBR_SKY_DIFFUSE[1] + PBR_LIGHT_COLOR[1] * sun_k,
            PBR_SKY_DIFFUSE[2] + PBR_LIGHT_COLOR[2] * sun_k,
        ];
        return pack_rgba(tonemap_and_oetf(sky));
    }

    // Decode the world normal from the oct RG bytes (the SAME UNORM round-trip the GPU did).
    let n = oct_decode([attrs.oct_rg[0] as f32 / 255.0, attrs.oct_rg[1] as f32 / 255.0]);
    let mat = materials
        .get(attrs.mat_id as usize)
        .copied()
        .unwrap_or_default();

    let mut metallic = mat.mrr[0];
    let mut roughness = mat.mrr[1].clamp(0.045, 1.0);
    let reflectance = mat.mrr[2];
    let shadow = attrs.shadow as f32 / 255.0;
    let mut ao = attrs.ao as f32 / 255.0;
    let mut emissive = mat.emissive;

    // Textured-PBR T6a: mirrors `deferred_pbr.hlsl`'s `#if !HWRT` `MATERIAL_FLAG_TEXTURED_BIT`
    // override — `reflectance` is left UNTOUCHED (no texture channel carries it yet);
    // `metallic`/`roughness`/`ao`/`emissive` are REASSIGNED, matching the shader's override
    // exactly (`roughness` re-clamped to the SAME `[0.045, 1.0]` floor). `gpbr.is_none()` (every
    // [`golden_deferred_resolve`] call) or `mrr[3]`'s flag bit unset (every EXISTING
    // [`GoldenMaterial`]) makes this a no-op — the values below are then bit-for-bit copies of
    // their pre-branch assignment (the flag=0 byte-identity invariant).
    if let Some(pbr) = gpbr
        && mat.mrr[3].to_bits() & GOLDEN_MATERIAL_FLAG_TEXTURED != 0
    {
        metallic = pbr[0];
        roughness = pbr[1].clamp(0.045, 1.0);
        ao *= pbr[2];
        for e in emissive.iter_mut().take(3) {
            *e *= pbr[3];
        }
    }

    let a = roughness * roughness;

    // f0: dielectric reflectance lerped toward base by metallic; diffuse killed by metallic.
    let dielectric_f0 = 0.16 * reflectance * reflectance;
    let f0 = [
        dielectric_f0 + (base[0] - dielectric_f0) * metallic,
        dielectric_f0 + (base[1] - dielectric_f0) * metallic,
        dielectric_f0 + (base[2] - dielectric_f0) * metallic,
    ];
    let diffuse_color = [
        base[0] * (1.0 - metallic),
        base[1] * (1.0 - metallic),
        base[2] * (1.0 - metallic),
    ];

    let v = [-rd[0], -rd[1], -rd[2]]; // view dir = -ray_dir (the shared ray-gen)
    let l = v_normalize(PBR_LIGHT_DIR);
    let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
    let nov = v_dot(n, v).max(1e-4);
    let nol = v_dot(n, l).max(0.0);
    let noh = v_dot(n, hvec).clamp(0.0, 1.0);
    let loh = v_dot(l, hvec).clamp(0.0, 1.0);

    // PBR P0-D: the SAME per-pixel term the resolve hoists before its light loop, reused at
    // both the direct and ambient specular sites below.
    let dfg = env_brdf_approx(roughness, nov);
    let energy_comp = multi_scatter_energy_comp(dfg, f0);

    // PBR P1: the reflection vector, hoisted once (mirrors the resolve's hoisted `R`). Unlike
    // the sky gradient below (which folds to a flat no-op since `PBR_SKY_DIFFUSE ==
    // PBR_SKY_SPEC`), the sun-disc kernel is NOT degenerate in `r` — it must be computed.
    let r = v_reflect(rd, n);
    let sun_k = sun_kernel(r, l, a);

    // Direct term: (Lambert diffuse + D*V*F specular) * NoL * shadow * light color.
    let d_term = d_ggx(noh, a);
    let v_term = v_smith_ggx_correlated(nov, nol, a);
    let f_term = f_schlick(loh, f0);
    let pi = core::f32::consts::PI;
    // PBR metal fix: decoupled specular occlusion, hoisted once per pixel (see
    // `specular_ao`'s doc) and reused at both ambient-specular sites below.
    let spec_ao = specular_ao(nov, roughness, ao);
    let mut lit = [0.0_f32; 3];
    for c in 0..3 {
        let spec = d_term * v_term * f_term[c] * energy_comp[c]; // PBR P0-D
        let diff = diffuse_color[c] * (1.0 / pi);
        let direct = (diff + spec) * (nol * shadow) * PBR_LIGHT_COLOR[c];

        // Ambient accumulator, mirroring the table oracle's `ambient[c]` (zero-initialized,
        // the directional's PBR P1 sun disc added first, the sky term second — the SAME
        // per-light iteration order as the degenerate 2-entry table).
        let mut ambient = 0.0_f32;

        // PBR P1: the HDR sun disc — a SECOND, roughness-widened specular response from the
        // SAME directional light, sampled along `r` instead of `l`. NOT shadow-modulated
        // (AO-gated only, mirroring the sky ambient's own AO gate). PBR metal fix: a
        // SPECULAR term, decoupled onto `spec_ao` (not the diffuse `ao`).
        let sun_spec = (f0[c] * dfg[0] + dfg[1]) * PBR_LIGHT_COLOR[c] * sun_k * energy_comp[c] * SUN_ENV_WEIGHT;
        ambient += sun_spec * spec_ao;

        // Ambient: EnvBRDFApprox specular against the sky + hemisphere diffuse. PBR P0-B's
        // reflection-vector gradient reduces to the FLAT `PBR_SKY_SPEC` here because this
        // degenerate table's sky == ground (`PBR_SKY_DIFFUSE == PBR_SKY_SPEC`, see
        // `golden_deferred_resolve_table`'s doc): `lerp(ground, sky, refl_hemi) == sky` for
        // any `refl_hemi` when ground == sky, so the reflect/hemi/steepen computation is a
        // byte-identical no-op and is skipped. PBR metal fix: diffuse ambient stays on the
        // diffuse `ao`; specular ambient (a metal's ENTIRE appearance) decouples onto
        // `spec_ao`.
        let spec_ambient = (f0[c] * dfg[0] + dfg[1]) * PBR_SKY_SPEC[c] * energy_comp[c];
        let diff_ambient = diffuse_color[c] * PBR_SKY_DIFFUSE[c];
        ambient += diff_ambient * ao + spec_ambient * spec_ao;

        lit[c] = direct + ambient + emissive[c];
    }
    pack_rgba(tonemap_and_oetf(lit))
}

/// The host mirror of the resolve's Render P7 SSAO ambient-AO combine (`deferred_pbr.hlsl`):
/// the structural `if (ssao_mode != 0u) { ao_final = min((view_t >= 1e30 ? 1.0 : ao), ssao); }`
/// gate. `ssao_mode` is `GoldenLightHeader::ssao_mode()` (header word 11); `ao` is the A2 SDF
/// march (`gMaterial.g`, `attrs.ao / 255`); `view_t` is the `gViewT` lane (`attrs.view_t`);
/// `ssao` is the per-pixel SSAO term (`gSsao` texel, `[0,1]`).
///
/// When `ssao_mode == 0` the `ssao` argument is IGNORED and `ao` is returned UNCHANGED — the
/// BYTE-IDENTICAL 0%-gate (every pre-P7 scene). When armed, this `view_t >= 1e30` branch forces
/// `class_ao = 1.0` for the PURE-BACKGROUND sentinel pixel; an SDF or (Render P7/P5-r1b) a now-
/// finite-`view_t` MESH pixel takes `min(ao, ssao)` — for the mesh pixel `ao == 1.0` (no analytic
/// SDF AO), so that reduces to pure SSAO, while an SDF pixel takes the most-occluded of the two
/// (cross-representation). The op-order mirrors the shader exactly (the `min` and the sentinel
/// compare are plain IEEE); this RESOLVE-side function is unchanged by the gViewT unlock.
#[inline]
pub(crate) fn ssao_combine(ssao_mode: u32, ao: f32, view_t: f32, ssao: f32) -> f32 {
    if ssao_mode == 0 {
        return ao;
    }
    let ao_class = if view_t >= 1.0e30 { 1.0 } else { ao };
    ao_class.min(ssao)
}

/// The CPU mirror of the `deferred_pbr` RESOLVE driven by the L0a + L0b light TABLE
/// (Lighting L0a/L0b). Identical to [`golden_deferred_resolve`] except the single
/// compiled-in directional + the `SKY_*` ambient constants are replaced by:
/// - the no-`P` front block (`[0..header.l0a_count()]`): `kind == Directional` contributes
///   the Cook-Torrance direct term, `kind == Sky` the hemisphere ambient; and
/// - (L0b) the point/spot block (`[l0a_count..light_count)`): the surface world position
///   `P = ro + rd * attrs.view_t` (the `gViewT` lane, read under `mask == 1`) drives a
///   range cull + smooth windowed inverse-square attenuation + (spot) the O2 cone falloff,
///   each scaled into the SAME Cook-Torrance direct term. `ro`/`rd` are the pixel's shared
///   ray-gen origin/dir (rd unit, so `view_t` is true world distance).
///
/// The accumulated LINEAR radiance is multiplied by `header.exposure()` as the FINAL op
/// (O3).
///
/// # W1 byte-identity op-order (HARD requirement)
/// The per-light direct expression is `(diff + spec) * (nol * shadow) * color` with the
/// accumulator initialized to `0.0`; the sky ambient is `diff_ambient * ao + spec_ambient *
/// spec_ao` (PBR metal fix: decoupled diffuse/specular AO) accumulated from `0.0`; the
/// FINAL `* exposure` is literally last. Because `0.0 + x == x` and `x * 1.0 == x` are
/// exact, a degenerate table — one directional (dir = +Z, color = white, illuminance = 1.0)
/// plus one sky (`sky == ground ==` [`PBR_SKY_DIFFUSE`]) with exposure 1.0 — reproduces
/// [`golden_deferred_resolve`] BYTE-FOR-BYTE (the directional matches
/// `LIGHT_DIR`/`LIGHT_COLOR`; the sky `lerp` folds since sky == ground). No reassociation is
/// permitted. This BYTE-FOR-BYTE equivalence covers the mask == 0 arm too:
/// `golden_deferred_resolve`'s background sky uses the SAME degenerate defaults (see its
/// doc), so the mask == 0 sweep folds identically.
#[allow(clippy::too_many_arguments)]
pub fn golden_deferred_resolve_table(
    attrs: MarcherAttributes,
    ro: [f32; 3],
    rd: [f32; 3],
    materials: &[GoldenMaterial],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
) -> u32 {
    // Render P7: delegate to the SSAO-aware variant with a no-op `ssao = 1.0`. On a
    // `ssao_mode() == 0` scene (every pre-P7 scene) the `ssao_combine` gate is never taken,
    // so `ao_final == attrs.ao` and the result is BYTE-IDENTICAL to the pre-P7 code path.
    golden_deferred_resolve_table_ssao(attrs, ro, rd, materials, header, lights, 1.0)
}

/// The Render P7 SSAO-aware mirror of [`golden_deferred_resolve_table`]: identical EXCEPT the
/// ambient `ao` is replaced by [`ssao_combine`]`(header.ssao_mode(), attrs.ao, attrs.view_t,
/// ssao)` — the host mirror of the resolve's structural `ao_final = min(class_ao, gSsao)` gate.
/// `ssao` is the per-pixel SSAO term (`[0,1]`, the GPU's `gSsao` texel) the SSAO golden feeds.
///
/// On a `header.ssao_mode() == 0` scene (every pre-P7 scene) the combine returns `attrs.ao`
/// UNCHANGED (the `ssao` argument is IGNORED), so this is BYTE-IDENTICAL to
/// [`golden_deferred_resolve_table`] — which is why the latter delegates here with `ssao = 1.0`
/// and every existing caller stays byte-stable.
#[allow(clippy::too_many_arguments)]
pub fn golden_deferred_resolve_table_ssao(
    attrs: MarcherAttributes,
    ro: [f32; 3],
    rd: [f32; 3],
    materials: &[GoldenMaterial],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    ssao: f32,
) -> u32 {
    let base = [
        attrs.base_rgb[0] as f32 / 255.0,
        attrs.base_rgb[1] as f32 / 255.0,
        attrs.base_rgb[2] as f32 / 255.0,
    ];
    if attrs.mask != 1 {
        // mesh / background / empty: render the PROCEDURAL SKY along the view ray (mirrors
        // the resolve's background branch) when the table carries a SKY entry; otherwise
        // keep the byte-identical dark pass-through (the 0%-gate: no SkyLight, no sky).
        return golden_sky_background(rd, header, lights).unwrap_or_else(|| pack_rgba(base));
    }

    let n = oct_decode([attrs.oct_rg[0] as f32 / 255.0, attrs.oct_rg[1] as f32 / 255.0]);
    let mat = materials
        .get(attrs.mat_id as usize)
        .copied()
        .unwrap_or_default();

    let metallic = mat.mrr[0];
    let roughness = mat.mrr[1].clamp(0.045, 1.0);
    let reflectance = mat.mrr[2];
    let a = roughness * roughness;

    let dielectric_f0 = 0.16 * reflectance * reflectance;
    let f0 = [
        dielectric_f0 + (base[0] - dielectric_f0) * metallic,
        dielectric_f0 + (base[1] - dielectric_f0) * metallic,
        dielectric_f0 + (base[2] - dielectric_f0) * metallic,
    ];
    let diffuse_color = [
        base[0] * (1.0 - metallic),
        base[1] * (1.0 - metallic),
        base[2] * (1.0 - metallic),
    ];

    let v = [-rd[0], -rd[1], -rd[2]];
    let nov = v_dot(n, v).max(1e-4);
    let shadow = attrs.shadow as f32 / 255.0;
    let ao = attrs.ao as f32 / 255.0;
    // Render P7: the SSAO combine (the host mirror of the resolve's structural `if`). On a
    // `ssao_mode() == 0` scene `ao_final == ao` (the 0%-gate); when armed, a mesh pixel takes
    // pure SSAO and an SDF pixel takes `min(march, ssao)`.
    let ao_final = ssao_combine(header.ssao_mode(), ao, attrs.view_t, ssao);
    let pi = core::f32::consts::PI;
    // The hemisphere "up" the sky lerp interpolates against (world up).
    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    let hemi = v_dot(n, UP) * 0.5 + 0.5;
    // PBR P0-D: the SAME per-pixel term the resolve hoists before its light loop, reused at
    // every specular site below (direct directional/point/spot + sky ambient).
    let dfg_v = env_brdf_approx(roughness, nov);
    let energy_comp = multi_scatter_energy_comp(dfg_v, f0);
    // PBR P1: the reflection vector, hoisted ONCE (mirrors the resolve's hoisted `R`) — feeds
    // BOTH the sky-gradient ambient specular below AND the per-directional HDR sun-disc term.
    // reflect(-v, n) == reflect(rd, n) since v == -rd (double negation is exact).
    let r = v_reflect(rd, n);
    // PBR metal fix: decoupled specular occlusion, hoisted once per pixel (see
    // `specular_ao`'s doc) and reused at every ambient-specular site below.
    let spec_ao = specular_ao(nov, roughness, ao_final);

    let mut lit_direct = [0.0_f32; 3];
    let mut ambient = [0.0_f32; 3];
    let count = header.l0a_count() as usize;
    for li in lights.iter().take(count) {
        match li.kind() {
            GOLDEN_LIGHT_KIND_DIRECTIONAL => {
                let l = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
                let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
                let nol = v_dot(n, l).max(0.0);
                let noh = v_dot(n, hvec).clamp(0.0, 1.0);
                let loh = v_dot(l, hvec).clamp(0.0, 1.0);
                let d_term = d_ggx(noh, a);
                let v_term = v_smith_ggx_correlated(nov, nol, a);
                let f_term = f_schlick(loh, f0);
                // PBR P1: the HDR sun-disc kernel for THIS directional light, sampled along
                // `r` (not `l`) — the analytic environment's bright-sun response.
                let sun_k = sun_kernel(r, l, a);
                for c in 0..3 {
                    let spec = d_term * v_term * f_term[c] * energy_comp[c]; // PBR P0-D
                    let diff = diffuse_color[c] * (1.0 / pi);
                    lit_direct[c] += (diff + spec) * (nol * shadow) * li.color_cone[c];
                    // PBR P1: a SECOND, roughness-widened specular response from this SAME
                    // light, added to the ambient — NOT shadow-modulated (AO-gated only,
                    // mirroring the sky ambient specular's own AO gate below).
                    let sun_spec = (f0[c] * dfg_v[0] + dfg_v[1]) * li.color_cone[c] * sun_k * energy_comp[c] * SUN_ENV_WEIGHT;
                    ambient[c] += sun_spec * spec_ao;
                }
            }
            GOLDEN_LIGHT_KIND_SKY => {
                // PBR P0-B: the ambient specular samples the sky/ground gradient along the
                // REFLECTION vector `R` (PBR P1: hoisted above the loop as `r`; a metal must
                // mirror its surroundings), while the diffuse hemisphere stays along `n`
                // (Lambert integrates the whole hemisphere).
                let sky = [li.color_cone[0], li.color_cone[1], li.color_cone[2]];
                let ground = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
                let refl_hemi_lin = v_dot(r, UP) * 0.5 + 0.5;
                // PBR metal fix: steepen the reflected hemisphere (smoothstep) so a metal
                // sweeps a real bright-cap -> dark-belly gradient instead of a flat mid-tone.
                // The DIFFUSE `hemi` above stays LINEAR — only the specular lobe steepens.
                let refl_hemi = refl_hemi_lin * refl_hemi_lin * (3.0 - 2.0 * refl_hemi_lin);
                for c in 0..3 {
                    // hemisphere diffuse = lerp(ground, sky, hemi); spec = EnvBRDFApprox
                    // against lerp(ground, sky, refl_hemi), P0-D energy-compensated.
                    let hemi_c = ground[c] + (sky[c] - ground[c]) * hemi;
                    let refl_c = ground[c] + (sky[c] - ground[c]) * refl_hemi;
                    let spec_ambient = (f0[c] * dfg_v[0] + dfg_v[1]) * refl_c * energy_comp[c];
                    let diff_ambient = diffuse_color[c] * hemi_c;
                    ambient[c] += diff_ambient * ao_final + spec_ambient * spec_ao;
                }
            }
            // Point/spot (kinds 1/2) are the L0b block (handled after this loop).
            _ => {}
        }
    }

    // L0b: reconstruct the surface world position from the `gViewT` lane (under `mask == 1`
    // only — `attrs.view_t` carries the sentinel on a non-lit pixel, but this whole function
    // already early-returned on `mask != 1`, so the read is gated). Then loop the point/spot
    // block `[l0a_count .. light_count)`, mirroring the shader's `deferred_pbr.hlsl` math
    // bit-for-bit (range cull → windowed inverse-square → O2 spot cone → Cook-Torrance).
    let p = [
        ro[0] + rd[0] * attrs.view_t,
        ro[1] + rd[1] * attrs.view_t,
        ro[2] + rd[2] * attrs.view_t,
    ];
    let l0a = header.l0a_count() as usize;
    let total = header.light_count() as usize;
    for li in lights.iter().take(total).skip(l0a) {
        let kind = li.kind();
        if kind != GOLDEN_LIGHT_KIND_POINT && kind != GOLDEN_LIGHT_KIND_SPOT {
            continue;
        }
        let pos = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
        let range = li.pos_range[3];
        let to_l = [pos[0] - p[0], pos[1] - p[1], pos[2] - p[2]];
        let d2 = v_dot(to_l, to_l);
        let range2 = range * range;
        if d2 > range2 {
            continue; // outside the cull sphere
        }
        // l = unit surface->light; mirrors the shader's `rsqrt(max(d2, 1e-8))`.
        let inv_d = 1.0 / d2.max(1e-8).sqrt();
        let l = [to_l[0] * inv_d, to_l[1] * inv_d, to_l[2] * inv_d];
        // Smooth windowed inverse-square (the shader's `(1 - (d2/range2)^2)^2` window).
        let win = (1.0 - (d2 * d2) / (range2 * range2)).clamp(0.0, 1.0);
        let mut atten = (1.0 / d2.max(1e-4)) * win * win;
        if kind == GOLDEN_LIGHT_KIND_SPOT {
            // O2 cone falloff (mirrors the shader): cos between -l and the spot axis,
            // smoothstepped between the outer and inner cone cosines, squared.
            let (cos_inner, cos_outer) = golden_unpack_cones(li.color_cone[3]);
            let spot_dir = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
            let cos_a = v_dot([-l[0], -l[1], -l[2]], spot_dir);
            let denom = (cos_inner - cos_outer).max(1e-4);
            let tt = ((cos_a - cos_outer) / denom).clamp(0.0, 1.0);
            atten *= tt * tt;
        }
        // The SAME Cook-Torrance direct term as the directional path, scaled by the
        // distance/cone attenuation and the light's baked color.
        let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
        let nol = v_dot(n, l).max(0.0);
        let noh = v_dot(n, hvec).clamp(0.0, 1.0);
        let loh = v_dot(l, hvec).clamp(0.0, 1.0);
        let d_term = d_ggx(noh, a);
        let v_term = v_smith_ggx_correlated(nov, nol, a);
        let f_term = f_schlick(loh, f0);
        for c in 0..3 {
            let spec = d_term * v_term * f_term[c] * energy_comp[c]; // PBR P0-D
            let diff = diffuse_color[c] * (1.0 / pi);
            lit_direct[c] += (diff + spec) * (nol * shadow) * atten * li.color_cone[c];
        }
    }

    let exposure = header.exposure();
    let mut lit = [0.0_f32; 3];
    for c in 0..3 {
        lit[c] = (lit_direct[c] + ambient[c] + mat.emissive[c]) * exposure;
    }
    pack_rgba(tonemap_and_oetf(lit))
}

/// The P6 R1 MULTI-LIGHT SDF-shadow CPU mirror of the `deferred_pbr` resolve — the
/// `shadow_mode != 0` oracle. Identical to [`golden_deferred_resolve_table`] EXCEPT each
/// per-light visibility `vis` is the per-caster shadow term (Decision 1/2/7). The PRIMARY
/// directional (the FIRST directional — the one the marcher marched into `gMaterial.r`) KEEPS
/// `attrs.shadow` (never re-marched, byte-stable across 1→N); an EXTRA flagged directional
/// gets `host_soft_shadow_ranged(P, n, L, SDF_T_MAX)` (it reaches everywhere — unbounded,
/// capped by dominant-N + NoL skip); a flagged point/spot caster gets `host_soft_shadow_
/// ranged(P, n, L, dist)` (the light DISTANCE bound); otherwise `vis` DEFAULTS to
/// `attrs.shadow` (the legacy L0b modulation).
///
/// At most [`MAX_SDF_SHADOW_CASTERS_PER_PIXEL`] extra casters are marched; the `NoL <= 0`
/// front-of-loop skip elides the march (and the term). `field` is the FROZEN edit-list
/// gateway (`sdf_edit_list`); the multi-light shadow term is consumer-side (±2/255), NOT
/// bit-exact — but with `header.shadow_mode() == 0` this is BYTE-IDENTICAL to
/// [`golden_deferred_resolve_table`] (no caster is marched, every `vis == shadow`).
#[allow(clippy::too_many_arguments)]
pub fn golden_deferred_resolve_table_shadowed<F: Fn([f32; 3]) -> f32>(
    attrs: MarcherAttributes,
    ro: [f32; 3],
    rd: [f32; 3],
    materials: &[GoldenMaterial],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    field: &F,
) -> u32 {
    // Render P7: delegate to the SSAO-aware variant with a no-op `ssao = 1.0`. On a
    // `ssao_mode() == 0` scene the `ssao_combine` gate is never taken, so this is
    // BYTE-IDENTICAL to the pre-P7 multi-light shadow path.
    golden_deferred_resolve_table_shadowed_ssao(attrs, ro, rd, materials, header, lights, field, 1.0)
}

/// The Render P7 SSAO-aware mirror of [`golden_deferred_resolve_table_shadowed`]: identical
/// EXCEPT the ambient `ao` is replaced by [`ssao_combine`]`(header.ssao_mode(), attrs.ao,
/// attrs.view_t, ssao)`. `ssao` is the per-pixel SSAO term (`gSsao` texel, `[0,1]`).
///
/// On a `header.ssao_mode() == 0` scene the combine returns `attrs.ao` UNCHANGED (the `ssao`
/// argument is IGNORED), so this is BYTE-IDENTICAL to
/// [`golden_deferred_resolve_table_shadowed`] — which delegates here with `ssao = 1.0`,
/// keeping every existing caller byte-stable.
#[allow(clippy::too_many_arguments)]
pub fn golden_deferred_resolve_table_shadowed_ssao<F: Fn([f32; 3]) -> f32>(
    attrs: MarcherAttributes,
    ro: [f32; 3],
    rd: [f32; 3],
    materials: &[GoldenMaterial],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    field: &F,
    ssao: f32,
) -> u32 {
    let base = [
        attrs.base_rgb[0] as f32 / 255.0,
        attrs.base_rgb[1] as f32 / 255.0,
        attrs.base_rgb[2] as f32 / 255.0,
    ];
    if attrs.mask != 1 {
        // mesh / background / empty: render the PROCEDURAL SKY along the view ray (mirrors
        // the resolve's background branch) when the table carries a SKY entry; otherwise
        // keep the byte-identical dark pass-through (the 0%-gate: no SkyLight, no sky).
        return golden_sky_background(rd, header, lights).unwrap_or_else(|| pack_rgba(base));
    }

    let n = oct_decode([attrs.oct_rg[0] as f32 / 255.0, attrs.oct_rg[1] as f32 / 255.0]);
    let mat = materials
        .get(attrs.mat_id as usize)
        .copied()
        .unwrap_or_default();

    let metallic = mat.mrr[0];
    let roughness = mat.mrr[1].clamp(0.045, 1.0);
    let reflectance = mat.mrr[2];
    let a = roughness * roughness;
    let dielectric_f0 = 0.16 * reflectance * reflectance;
    let f0 = [
        dielectric_f0 + (base[0] - dielectric_f0) * metallic,
        dielectric_f0 + (base[1] - dielectric_f0) * metallic,
        dielectric_f0 + (base[2] - dielectric_f0) * metallic,
    ];
    let diffuse_color = [
        base[0] * (1.0 - metallic),
        base[1] * (1.0 - metallic),
        base[2] * (1.0 - metallic),
    ];

    let v = [-rd[0], -rd[1], -rd[2]];
    let nov = v_dot(n, v).max(1e-4);
    let shadow = attrs.shadow as f32 / 255.0;
    let ao = attrs.ao as f32 / 255.0;
    // Render P7: the SSAO combine (host mirror of the resolve's structural `if`). On a
    // `ssao_mode() == 0` scene `ao_final == ao` (the 0%-gate).
    let ao_final = ssao_combine(header.ssao_mode(), ao, attrs.view_t, ssao);
    let pi = core::f32::consts::PI;
    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    let hemi = v_dot(n, UP) * 0.5 + 0.5;
    // PBR P0-D: the SAME per-pixel term the resolve hoists before its light loop, reused at
    // every specular site below (direct directional/point/spot + sky ambient).
    let dfg_v = env_brdf_approx(roughness, nov);
    let energy_comp = multi_scatter_energy_comp(dfg_v, f0);
    // PBR P1: the reflection vector, hoisted ONCE (mirrors the resolve's hoisted `R`) — feeds
    // BOTH the sky-gradient ambient specular below AND the per-directional HDR sun-disc term.
    // reflect(-v, n) == reflect(rd, n) since v == -rd (double negation is exact).
    let r = v_reflect(rd, n);
    // PBR metal fix: decoupled specular occlusion, hoisted once per pixel (see
    // `specular_ao`'s doc) and reused at every ambient-specular site below.
    let spec_ao = specular_ao(nov, roughness, ao_final);

    // P6 R1: the shadow_mode gate + the surface world position `P` (hoisted, mirroring the
    // shader) + the dominant-N march counter.
    let multi_light = header.shadow_mode() != 0;
    let p = [
        ro[0] + rd[0] * attrs.view_t,
        ro[1] + rd[1] * attrs.view_t,
        ro[2] + rd[2] * attrs.view_t,
    ];
    // Normal-offset start bias for the per-light ranged shadow march: lift the origin off
    // the surface so grazing rays clear it (mirrors the resolve's
    // `sdf_soft_shadow_ranged(P + n*SHADOW_NORMAL_BIAS, n, l, t_max)`).
    let pb = [
        p[0] + n[0] * SHADOW_NORMAL_BIAS,
        p[1] + n[1] * SHADOW_NORMAL_BIAS,
        p[2] + n[2] * SHADOW_NORMAL_BIAS,
    ];
    let mut marched = 0u32;

    let mut lit_direct = [0.0_f32; 3];
    let mut ambient = [0.0_f32; 3];
    let mut primary_dir_seen = false;
    let count = header.l0a_count() as usize;
    for li in lights.iter().take(count) {
        match li.kind() {
            GOLDEN_LIGHT_KIND_DIRECTIONAL => {
                let l = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
                let nol = v_dot(n, l).max(0.0);
                // The primary directional KEEPS gMaterial.r; an extra flagged directional
                // marches with t_max = SDF_T_MAX. `vis` defaults to `shadow` (legacy).
                let mut vis = shadow;
                if !primary_dir_seen {
                    primary_dir_seen = true;
                } else if multi_light
                    && li.casts_sdf_shadow()
                    && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL
                    && nol > SHADOW_NDOTL_EPS
                {
                    vis = host_soft_shadow_ranged(pb, n, l, SDF_T_MAX, field);
                    marched += 1;
                }
                let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
                let noh = v_dot(n, hvec).clamp(0.0, 1.0);
                let loh = v_dot(l, hvec).clamp(0.0, 1.0);
                let d_term = d_ggx(noh, a);
                let v_term = v_smith_ggx_correlated(nov, nol, a);
                let f_term = f_schlick(loh, f0);
                // PBR P1: the HDR sun-disc kernel for THIS directional light, sampled along
                // `r` (not `l`) — the analytic environment's bright-sun response.
                let sun_k = sun_kernel(r, l, a);
                for c in 0..3 {
                    let spec = d_term * v_term * f_term[c] * energy_comp[c]; // PBR P0-D
                    let diff = diffuse_color[c] * (1.0 / pi);
                    lit_direct[c] += (diff + spec) * (nol * vis) * li.color_cone[c];
                    // PBR P1: a SECOND, roughness-widened specular response from this SAME
                    // light, added to the ambient — NOT shadow-modulated (AO-gated only,
                    // mirroring the sky ambient specular's own AO gate below).
                    let sun_spec = (f0[c] * dfg_v[0] + dfg_v[1]) * li.color_cone[c] * sun_k * energy_comp[c] * SUN_ENV_WEIGHT;
                    ambient[c] += sun_spec * spec_ao;
                }
            }
            GOLDEN_LIGHT_KIND_SKY => {
                // PBR P0-B: ambient specular samples the sky/ground gradient along `R` (PBR
                // P1: hoisted above the loop as `r`) instead of the flat sky color; diffuse
                // stays along n.
                let sky = [li.color_cone[0], li.color_cone[1], li.color_cone[2]];
                let ground = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
                let refl_hemi_lin = v_dot(r, UP) * 0.5 + 0.5;
                // PBR metal fix: steepen the reflected hemisphere (smoothstep) so a metal
                // sweeps a real bright-cap -> dark-belly gradient instead of a flat mid-tone.
                // The DIFFUSE `hemi` above stays LINEAR — only the specular lobe steepens.
                let refl_hemi = refl_hemi_lin * refl_hemi_lin * (3.0 - 2.0 * refl_hemi_lin);
                for c in 0..3 {
                    let hemi_c = ground[c] + (sky[c] - ground[c]) * hemi;
                    let refl_c = ground[c] + (sky[c] - ground[c]) * refl_hemi;
                    let spec_ambient = (f0[c] * dfg_v[0] + dfg_v[1]) * refl_c * energy_comp[c];
                    let diff_ambient = diffuse_color[c] * hemi_c;
                    ambient[c] += diff_ambient * ao_final + spec_ambient * spec_ao;
                }
            }
            _ => {}
        }
    }

    // L0b point/spot block (flat) with the per-caster ranged march.
    let l0a = header.l0a_count() as usize;
    let total = header.light_count() as usize;
    for li in lights.iter().take(total).skip(l0a) {
        let kind = li.kind();
        if kind != GOLDEN_LIGHT_KIND_POINT && kind != GOLDEN_LIGHT_KIND_SPOT {
            continue;
        }
        let pos = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
        let range = li.pos_range[3];
        let to_l = [pos[0] - p[0], pos[1] - p[1], pos[2] - p[2]];
        let d2 = v_dot(to_l, to_l);
        let range2 = range * range;
        if d2 > range2 {
            continue;
        }
        let inv_d = 1.0 / d2.max(1e-8).sqrt();
        let l = [to_l[0] * inv_d, to_l[1] * inv_d, to_l[2] * inv_d];
        let win = (1.0 - (d2 * d2) / (range2 * range2)).clamp(0.0, 1.0);
        let mut atten = (1.0 / d2.max(1e-4)) * win * win;
        if kind == GOLDEN_LIGHT_KIND_SPOT {
            let (cos_inner, cos_outer) = golden_unpack_cones(li.color_cone[3]);
            let spot_dir = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
            let cos_a = v_dot([-l[0], -l[1], -l[2]], spot_dir);
            let denom = (cos_inner - cos_outer).max(1e-4);
            let tt = ((cos_a - cos_outer) / denom).clamp(0.0, 1.0);
            atten *= tt * tt;
        }
        let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
        let nol = v_dot(n, l).max(0.0);
        let noh = v_dot(n, hvec).clamp(0.0, 1.0);
        let loh = v_dot(l, hvec).clamp(0.0, 1.0);
        let d_term = d_ggx(noh, a);
        let v_term = v_smith_ggx_correlated(nov, nol, a);
        let f_term = f_schlick(loh, f0);
        // `vis` defaults to `shadow` (legacy L0b modulation); a flagged caster marches with
        // t_max = the light DISTANCE (`sqrt(d2)`).
        let mut vis = shadow;
        if multi_light
            && li.casts_sdf_shadow()
            && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL
            && nol > SHADOW_NDOTL_EPS
        {
            let t_max = d2.sqrt();
            vis = host_soft_shadow_ranged(pb, n, l, t_max, field);
            marched += 1;
        }
        for c in 0..3 {
            let spec = d_term * v_term * f_term[c] * energy_comp[c]; // PBR P0-D
            let diff = diffuse_color[c] * (1.0 / pi);
            lit_direct[c] += (diff + spec) * (nol * vis) * atten * li.color_cone[c];
        }
    }

    let exposure = header.exposure();
    let mut lit = [0.0_f32; 3];
    for c in 0..3 {
        lit[c] = (lit_direct[c] + ambient[c] + mat.emissive[c]) * exposure;
    }
    pack_rgba(tonemap_and_oetf(lit))
}

/// The host cluster-cull config (mirrors `boyko_render::light::ClusterConfig`). The vulkan
/// crate cannot depend on `boyko_render`, so the golden carries its own POD mirror; the
/// dims + exp-Z near/far + the caps are the SAME the GPU cull uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoldenClusterConfig {
    /// Froxel grid X dimension.
    pub dim_x: u32,
    /// Froxel grid Y dimension.
    pub dim_y: u32,
    /// Froxel grid Z (exp-Z slice) dimension.
    pub dim_z: u32,
    /// Per-froxel light-index cap (O2 clamp-and-drop).
    pub max_lights_per_cluster: u32,
    /// Exp-Z near plane (slice 0 view-z).
    pub z_near: f32,
    /// Exp-Z far plane (slice `dim_z` view-z).
    pub z_far: f32,
}

impl GoldenClusterConfig {
    /// The total froxel count (`dim_x * dim_y * dim_z`).
    #[inline]
    pub const fn cluster_count(&self) -> u32 {
        self.dim_x * self.dim_y * self.dim_z
    }

    /// The exp-Z slice scale `dim_z / ln(far/near)` (mirrors `ClusterConfig::z_scale`).
    #[inline]
    pub fn z_scale(&self) -> f32 {
        (self.dim_z as f32) / (self.z_far / self.z_near).ln()
    }

    /// The exp-Z slice bias `-ln(near) * z_scale` (mirrors `ClusterConfig::z_bias`).
    #[inline]
    pub fn z_bias(&self) -> f32 {
        -self.z_near.ln() * self.z_scale()
    }
}

/// Linearizes froxel `(x, y, z)` → flat index `(y * dim_x + x) * dim_z + z` — the host mirror
/// of `light::cluster_index` and the shader's `cluster_linear_index` (Z innermost). THE one
/// linearization the host + both shaders share.
#[inline]
pub fn golden_cluster_index(x: u32, y: u32, z: u32, dim_x: u32, dim_z: u32) -> u32 {
    (y * dim_x + x) * dim_z + z
}

/// Maps a view-space depth `view_z` to its exp-Z froxel slice, clamped to `[0, dim_z-1]`
/// (mirrors the shader's `cluster_z_slice`). A `view_z <= 0` clamps to slice 0.
#[inline]
pub fn golden_cluster_z_slice(view_z: f32, cfg: &GoldenClusterConfig) -> u32 {
    if view_z <= 0.0 {
        return 0;
    }
    let slice = view_z.ln() * cfg.z_scale() + cfg.z_bias();
    let si = slice.floor() as i32;
    si.clamp(0, cfg.dim_z as i32 - 1) as u32
}

/// Maps pixel `(px, py)` at extent `(w, h)` to its froxel `(x, y)` tile, clamped to the grid
/// (mirrors the shader's `cluster_xy_tile`).
#[inline]
pub fn golden_cluster_xy_tile(
    px: u32,
    py: u32,
    w: u32,
    h: u32,
    cfg: &GoldenClusterConfig,
) -> (u32, u32) {
    let tx = ((px * cfg.dim_x) / w.max(1)).min(cfg.dim_x - 1);
    let ty = ((py * cfg.dim_y) / h.max(1)).min(cfg.dim_y - 1);
    (tx, ty)
}

/// Converts a slice view-z to the world ray parameter `t` for `(ro, rd)` (mirrors the cull's
/// `view_z_to_t`): PERSP `view_z / dot(rd, fwd)`, ORTHO `view_z` (rd = (0,0,-1)). `fwd` is the
/// camera forward axis (O1: NORMALIZED); for ORTHO it is ignored.
#[inline]
pub(crate) fn golden_view_z_to_t(view_z: f32, rd: [f32; 3], camera: CompositeCamera) -> f32 {
    match camera {
        CompositeCamera::Perspective { forward, .. } => {
            let cos_axis = v_dot(rd, forward).max(1e-4);
            view_z / cos_axis
        }
        CompositeCamera::Ortho => view_z,
    }
}

/// The exp-Z view-z at slice boundary `k` (mirrors the cull's `slice_view_z`).
#[inline]
pub(crate) fn golden_slice_view_z(k: u32, cfg: &GoldenClusterConfig) -> f32 {
    cfg.z_near * (cfg.z_far / cfg.z_near).powf(k as f32 / cfg.dim_z as f32)
}

/// Squared distance from a point to an AABB (0 inside) — mirrors the shader's
/// `sq_dist_point_aabb` (the canonical clustered-cull sphere-vs-AABB test). Since H1.6
/// (`docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` D10) the shader computes this exact sum through
/// explicit, `NoContraction`-decorated `OpFSub`/`OpFMul`/`OpFAdd` rather than `dot()`, in the
/// identical `((dx^2+dy^2)+dz^2)` association this loop accumulates — Rust `f32` never fuses
/// `a*b+c` by default, so the two sides are now bit-exact BY CONSTRUCTION, not by accident of a
/// particular driver's `OpDot` lowering (the same argument `shaders/ddgi_resolve.hlsli:136-141`
/// already carries for DDGI).
#[inline]
pub(crate) fn golden_sq_dist_point_aabb(c: [f32; 3], aabb_min: [f32; 3], aabb_max: [f32; 3]) -> f32 {
    let mut s = 0.0_f32;
    for i in 0..3 {
        let d = (aabb_min[i] - c[i]).max(c[i] - aabb_max[i]).max(0.0);
        s += d * d;
    }
    s
}

/// Builds one froxel's world-space AABB from its screen-tile corners at the slice's near/far
/// view-z (mirrors `cluster_cull.hlsl`'s phase-0 unprojection — the SAME `composite_ray` the
/// resolve uses). Shared verbatim by [`golden_cluster_cull`] and [`golden_cluster_cull_hier`]
/// so the two host mirrors' phase 0 is bit-identical BY CONSTRUCTION rather than by
/// re-derivation — exactly the property the hierarchical design's D2 relies on (the coarse AABB
/// is a reduction over values already computed here, never a second geometric computation).
pub fn golden_froxel_aabb(
    x: u32,
    y: u32,
    z: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    cfg: &GoldenClusterConfig,
) -> ([f32; 3], [f32; 3]) {
    // The tile's inclusive corner pixels (mirror the cull's px0/py0/px1/py1).
    let px0 = (x * img_w) / cfg.dim_x;
    let py0 = (y * img_h) / cfg.dim_y;
    let px1 = (((x + 1) * img_w) / cfg.dim_x).saturating_sub(1).max(px0);
    let py1 = (((y + 1) * img_h) / cfg.dim_y).saturating_sub(1).max(py0);
    let corners = [(px0, py0), (px1, py0), (px0, py1), (px1, py1)];
    let vz_near = golden_slice_view_z(z, cfg);
    let vz_far = golden_slice_view_z(z + 1, cfg);
    let mut aabb_min = [1.0e30_f32; 3];
    let mut aabb_max = [-1.0e30_f32; 3];
    for &(cx, cy) in &corners {
        let (ro, rd) = composite_ray(cx, cy, img_w, img_h, camera);
        for &vz in &[vz_near, vz_far] {
            let t = golden_view_z_to_t(vz, rd, camera);
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            for i in 0..3 {
                aabb_min[i] = aabb_min[i].min(p[i]);
                aabb_max[i] = aabb_max[i].max(p[i]);
            }
        }
    }
    (aabb_min, aabb_max)
}

/// The host clustered froxel light cull (mirrors `cluster_cull.hlsl`). For each froxel it
/// builds the WORLD-space AABB from the screen-tile corners at the slice's near/far view-z
/// (the SAME `composite_ray` the resolve uses) and records the surviving POINT/SPOT light
/// indices (`sqDistPointAABB <= r²`) in table order, clamped to `max_lights_per_cluster`
/// (O2). Returns a `Vec` of per-froxel index `Vec`s, flat-indexed by
/// [`golden_cluster_index`]. Directional/sky are GLOBAL (never in the per-froxel sets).
///
/// The cull is geometric + deterministic; the resolve's per-froxel sum is order-stable
/// (table order), so a froxel whose set contains every in-range light reproduces the
/// brute-force resolve bit-for-bit.
///
/// `inject_nan_froxel`, when `Some(fi)`, overwrites froxel `fi`'s own AABB to an all-NaN box
/// (bit pattern [`GOLDEN_QUIET_NAN_BITS`], matching HLSL's `asfloat(0x7FC00000u)`) immediately
/// after phase 0 — the "base module" leg of H3 mutation (vii)'s two-sided, three-implementation
/// fault injection (plan §8.3 "Mutation (vii) in full", §8.10 item 5: "mirrored in the HIER
/// module, the base module AND the host mirror"). Mirrors [`golden_cluster_cull_hier`]'s own
/// `inject_nan_froxel` parameter so the SAME poisoned froxel can be compared between the flat
/// oracle and the hierarchical mirror's mitigated arm (§5 Case B's corollary: froxel `fi`'s own
/// fine test computes the identical all-NaN `sq_dist` in both, so an equally-poisoned flat arm is
/// required for the comparison to mean anything). `None` performs no injection — every existing
/// call site.
pub fn golden_cluster_cull(
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    cfg: &GoldenClusterConfig,
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    inject_nan_froxel: Option<u32>,
) -> Vec<Vec<u32>> {
    let count = cfg.cluster_count() as usize;
    let mut grid: Vec<Vec<u32>> = vec![Vec::new(); count];
    let l0a = header.l0a_count();
    let total = header.light_count();
    let nan = f32::from_bits(GOLDEN_QUIET_NAN_BITS);
    for y in 0..cfg.dim_y {
        for x in 0..cfg.dim_x {
            for z in 0..cfg.dim_z {
                let fi = golden_cluster_index(x, y, z, cfg.dim_x, cfg.dim_z);
                let (mut aabb_min, mut aabb_max) =
                    golden_froxel_aabb(x, y, z, img_w, img_h, camera, cfg);
                if inject_nan_froxel == Some(fi) {
                    aabb_min = [nan; 3];
                    aabb_max = [nan; 3];
                }
                let cell = &mut grid[fi as usize];
                for i in l0a..total {
                    let li = &lights[i as usize];
                    let kind = li.kind();
                    if kind != GOLDEN_LIGHT_KIND_POINT && kind != GOLDEN_LIGHT_KIND_SPOT {
                        continue;
                    }
                    let pos = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
                    let r = li.pos_range[3];
                    if golden_sq_dist_point_aabb(pos, aabb_min, aabb_max) <= r * r
                        && (cell.len() as u32) < cfg.max_lights_per_cluster
                    {
                        cell.push(i);
                    }
                }
            }
        }
    }
    grid
}

/// Host mirror of the HIER shader's group width (VB-P1e design §4,
/// `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md`) — `[numthreads(HIER_TPG, 1, 1)]` under `-D HIER=1`
/// (`cluster_cull.hlsl:176`, `#define HIER_TPG 256u`, `cluster_cull.hlsl:147`). Rung H2 shipped the
/// GPU-side pin: `cluster_cull.hlsl`'s `#if (HIER_TPG) != 256u` → `#error` (`cluster_cull.hlsl:157`)
/// keeps the shader's own literal from silently drifting off this value, and
/// `cluster_cull_hier_dis_gate.rs`'s gate (h) independently pins the emitted
/// `OpExecutionMode %main LocalSize 256 1 1` on the committed `.spv` itself, since neither the
/// `#error` guard nor the source-level `numthreads`/`HIER_TPG` tie protects a stale or
/// hand-crafted artifact.
pub const HIER_GROUP_THREADS: u32 = 256;

/// Host mirror of `HIER_MASK_WORDS` (VB-P1e design D6): `MAX_LIGHTS / 32`
/// (`cluster_cull.hlsl:148`, `#define HIER_MASK_WORDS 32u`). `boyko_render::light.rs:51`
/// (`pub const MAX_LIGHTS: u32 = 1024`) carries no cross-check with THIS mirror today — the vulkan
/// crate cannot depend on `boyko_render` (see [`GoldenClusterConfig`]'s own doc comment), so this
/// stays a separately mirrored literal, not a shared constant. Rung H2 shipped the equivalent
/// compile-time pin on the `boyko_render` side instead: `boyko_render::light.rs:65-69`
/// (`const _: () = assert!(MAX_LIGHTS == HIER_MASK_WORDS * 32, ..)`, against `light.rs`'s own
/// `HIER_MASK_WORDS` at `light.rs:56`) enforces the equality D6 requires at the one call site that
/// CAN see both constants.
pub const HIER_MASK_WORDS: u32 = 32;

/// Host mirror of `HIER_MASK_BITS` (`HIER_MASK_WORDS * 32`) — equal to `MAX_LIGHTS` (1024) by
/// D6's pinned equality, and the bound D7's `ps_room` clamp is derived from.
pub const HIER_MASK_BITS: u32 = HIER_MASK_WORDS * 32;

/// `gps` — HIER groups per z-slice (D3): `max(1, ceil(dim_x * dim_y / HIER_GROUP_THREADS))`.
/// The host mirror of the shader's `uint gps = max(1u, (bdx*bdy+255u)/256u)`, in the SAME
/// arithmetic form D11's `ClusterConfig::hier_group_count` uses — checkable by eye against the
/// shader, not merely equivalent to it (Rev 5 P2-4).
// The `+ HIER_GROUP_THREADS - 1) / HIER_GROUP_THREADS` form is `div_ceil` written out, kept
// deliberately: D11's own Rev 5 P2-4 fix rejects `.div_ceil()` here — its const-stability is "a
// separate question this plan should not depend on" — and the written-out form is the shader's
// `(bdx*bdy+255u)/256u` TOKEN-FOR-TOKEN, which is the whole point (a reviewer checks it by eye
// against the HLSL, not by trusting an equivalence proof).
#[allow(clippy::manual_div_ceil)]
#[inline]
pub const fn golden_hier_groups_per_slice(dim_x: u32, dim_y: u32) -> u32 {
    let gps = (dim_x * dim_y + HIER_GROUP_THREADS - 1) / HIER_GROUP_THREADS;
    if gps == 0 { 1 } else { gps }
}

/// Pure-arithmetic replica of the HIER shader's thread-to-froxel map for one `(group_id, lane)`
/// pair — D3's `gps`/`slice`/`s`/`x`/`y`/`z`/`fi` plus D8's three-term `valid` predicate.
/// Returns `(x, y, z, fi, valid)`.
///
/// **Scope (H1 assertion 7, §8.6).** This is a Rust RE-IMPLEMENTATION of the shader's walk, not
/// a pin on the HLSL — if the shader and this mirror drift, only H3 (device) sees it. It exists
/// so degenerate/non-64-aligned dims matrices (`16x9x23`, `1x1x1`, `0x0x0`, `255x255x255`) can be
/// swept with no camera, no lights and no GPU.
// The `if dim_x != 0 { .. } else { 0 }` guards are the shader's own D8-obligation ternaries
// (`x = (bdx != 0u) ? (s % bdx) : 0u;`, `y = (bdx != 0u) ? (s / bdx) : 0u;`) written out
// token-for-token rather than as `checked_div`/`checked_rem` — a reviewer checks this function
// against the HLSL by eye (§8.6 assertion 7's own stated scope), and a `checked_*` idiom would
// break that direct correspondence.
#[allow(clippy::manual_checked_ops)]
#[inline]
pub fn golden_hier_thread_map(
    group_id: u32,
    lane: u32,
    dim_x: u32,
    dim_y: u32,
    dim_z: u32,
    capacity: u32,
) -> (u32, u32, u32, u32, bool) {
    let gps = golden_hier_groups_per_slice(dim_x, dim_y);
    let slice = group_id / gps;
    let s = (group_id % gps) * HIER_GROUP_THREADS + lane;
    let x = if dim_x != 0 { s % dim_x } else { 0 };
    let y = if dim_x != 0 { s / dim_x } else { 0 };
    let z = slice;
    let fi = golden_cluster_index(x, y, z, dim_x, dim_z);
    let valid = s < dim_x * dim_y && slice < dim_z && fi < capacity;
    (x, y, z, fi, valid)
}

/// Host mirror of the BASE cull arm's group width — `cluster_cull.hlsl`'s `[numthreads(64,1,1)]`
/// on the no-`-D` compile, and the divisor every dispatch site uses
/// (`scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X)` in `passes/{vb,gbuffer,forward}.rs`).
/// Mirrors [`HIER_GROUP_THREADS`]'s role for the `-D HIER=1` arm.
pub const BASE_GROUP_THREADS: u32 = 64;

/// Pure-arithmetic replica of the BASE cull arm's thread-to-froxel map for one dispatch thread
/// — `cluster_cull.hlsl`'s `#else` prologue: the VB-P1j capacity-clamped `cluster_count`, the
/// `fi >= cluster_count` early return, and the `(x, y, z)` delinearization behind it. Returns
/// `(x, y, z, fi, valid)`, the SAME shape [`golden_hier_thread_map`] returns for the other arm.
///
/// `capacity` is `ClusterGrid`'s own element count — the buffer's BOOT size, which the shader
/// reads back with `GetDimensions` (SPIR-V `OpArrayLength`) rather than from any push word.
/// `dim_x`/`dim_y`/`dim_z` are the LIVE light-table header's dims, which
/// `sync_cluster_light_gate` republishes every frame from the LIVE `ClusterConfig` and which a
/// post-boot edit can therefore move away from `capacity`. The VB-P1j clamp is exactly the `min`
/// below: without it, `valid` holds for `fi` up to `live_cc - 1`, which exceeds `capacity - 1`
/// whenever the live dims grow — the out-of-bounds device write this mirror exists to pin.
///
/// **Scope (mirrors [`golden_hier_thread_map`]'s own stated scope).** This is a Rust
/// RE-IMPLEMENTATION of the shader's prologue, not a pin on the HLSL: if the shader and this
/// mirror drift, only a device run sees it. The artifact-level pin that the clamp is PRESENT in
/// the committed `.spv` is `cluster_cull_spv_sync.rs`'s census (`op_array_length`), which counts
/// the emitted `OpArrayLength` on the real module.
// The delinearization is written out in the shader's own `% dim_z` / `/ dim_z` / `% dim_x` /
// `/ dim_x` form rather than via a helper, for the same reason `golden_hier_thread_map` writes
// its ternaries out: a reviewer checks this function against the HLSL by eye.
#[inline]
pub fn golden_base_thread_map(
    tid: u32,
    dim_x: u32,
    dim_y: u32,
    dim_z: u32,
    capacity: u32,
) -> (u32, u32, u32, u32, bool) {
    // `uint cluster_count = min(cp.dim_x * cp.dim_y * cp.dim_z, grid_capacity);`
    let cluster_count = (dim_x * dim_y * dim_z).min(capacity);
    let fi = tid;
    if fi >= cluster_count {
        // `if (fi >= cluster_count) { return; }` — nothing below the early return executes, so
        // the delinearization (which divides by `dim_z`/`dim_x`) is never reached on a
        // degenerate all-zero-dims header either.
        return (0, 0, 0, fi, false);
    }
    let z = fi % dim_z;
    let xy = fi / dim_z;
    let x = xy % dim_x;
    let y = xy / dim_x;
    (x, y, z, fi, true)
}

/// Aggregate pair-count diagnostics [`golden_cluster_cull_hier`] returns alongside its
/// per-froxel index grid — the raw numerator H1's selectivity gate
/// (`pairs_hier() as f64 / (capacity * ps_n) as f64 <= 1.0/8.0`, §8.6 assertion 5) is computed
/// from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HierCullStats {
    /// Groups dispatched (`gps * dim_z`, D3).
    pub groups: u32,
    /// The point/spot scan range after D7's clamp: `min(ps_total, ps_room)`. Group-uniform —
    /// evaluated once from the header, identical for every group in the dispatch.
    pub ps_n: u32,
    /// Total `valid` `(group, lane)` pairs across the whole dispatch — the count H1 assertion
    /// 2's second clause pins against `capacity` (§8.2(B1): totality + this count together
    /// derive device exactly-once).
    pub valid_lanes: u32,
    /// Coarse (froxel-GROUP, light) pair tests — phase 4. Exactly `groups * ps_n`, since `ps_n`
    /// is group-uniform and phase 4 tests it once per group (D7).
    pub pairs_coarse: u64,
    /// Fine (froxel, light) pair tests — phase 5. Summed over every VALID froxel's own
    /// coarse-accepted candidate count (the group's coarse-mask population, shared by every
    /// froxel of that group — D8 review item 6).
    pub pairs_fine: u64,
    /// Per-group coarse-accept count (phase 4's popcount of the group's coarse mask), indexed by
    /// `group_id` in `[0, groups)` — deliverable 8 (plan §8.6, Rev 6). This is a PRECONDITION
    /// SOURCE, not a diagnostic: H3 mutation (vii)'s rig requirement needs "group 0's coarse box
    /// rejects >= 1 punctual light" (`group_coarse_accept[0] < ps_n`) asserted from the
    /// `inject_nan_froxel: None` run *before* the arm comparison is evaluated, so a vacuous run
    /// (the coarse box already accepts everything) is reported as invalid rather than a pass
    /// (plan §8.3 "Mutation (vii) in full", §8.10 item 5b). Not a new gate: `pairs_fine` is
    /// already the sum of these counts over valid froxels, so assertion 5's selectivity number is
    /// unchanged — this only exposes the per-group decomposition already computed.
    pub group_coarse_accept: Vec<u32>,
}

impl HierCullStats {
    /// Total (coarse + fine) pair tests the hierarchical arm performs — the selectivity gate's
    /// numerator.
    #[inline]
    pub const fn pairs_hier(&self) -> u64 {
        self.pairs_coarse + self.pairs_fine
    }
}

/// The IEEE-754 quiet-NaN bit pattern matching HLSL's `asfloat(0x7FC00000u)` literal — used by
/// [`golden_cluster_cull_hier`]'s `inject_nan_froxel` poison (§8.3 "Mutation (vii) in full":
/// "NOT `0.0/0.0`, which is a constant expression a compiler may fold or reject").
const GOLDEN_QUIET_NAN_BITS: u32 = 0x7FC0_0000;

/// The host mirror of the `-D HIER=1` block-decomposed cull (VB-P1e design; §4/§5/D2/D3/D7/D8/D9,
/// `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §8.6 rung H1). Builds the SAME per-froxel AABB as
/// [`golden_cluster_cull`] (phase 0, shared via [`golden_froxel_aabb`] so the two host mirrors
/// cannot drift from each other), partitions froxels into [`HIER_GROUP_THREADS`]-lane groups per
/// D3's `gps`/`slice`/`s` map ([`golden_hier_thread_map`] — the FULL form, not the degenerate
/// `slice = gid; s = lane` collapse the default 16x9x24 grid happens to reduce to), folds each
/// group's lanes into ONE coarse AABB — the componentwise min/max of the lanes' OWN
/// already-computed (and already-substituted) AABBs, never a recomputation from block geometry
/// (D2) — tests the point/spot light table against that coarse box once per group (phase 4), and
/// re-walks only the coarse-accepted candidates, ASCENDING (table order), with the
/// token-identical fine test for every valid froxel (phase 5).
///
/// D8's two-constant substitution, applied per lane before the fold:
/// - `!valid` (no froxel): the min/max IDENTITY `(+1e30, -1e30)` — contributes nothing to the
///   fold, so padding lanes never widen the coarse box;
/// - `valid && !finite` (some component of the lane's own AABB has `abs > 1e30` — true for NaN
///   and for `+-inf`, since an ordered compare is false for both): the ABSORBING
///   `(-f32::MAX, +f32::MAX)` — forces the coarse box to the universe, so the WHOLE group
///   degrades to exactly the flat arm's walk (§5 Case B). **`f32::MAX`, not `1e30`** — the Rev 5
///   fix; `1e30` is the finiteness THRESHOLD classifying the lane, a different constant serving a
///   different purpose, and is never the value a poisoned lane stores;
/// - `valid && finite`: the lane's own AABB, unmodified.
///
/// `inject_nan_froxel`, when `Some(fi)`, overwrites froxel `fi`'s OWN (pre-substitution) AABB to
/// an all-NaN box on all six components (bit pattern [`GOLDEN_QUIET_NAN_BITS`], matching HLSL's
/// `asfloat(0x7FC00000u)`) immediately after phase 0 — the host leg of mutation (vii)'s
/// two-sided, three-implementation fault injection (§8.3 "Mutation (vii) in full": mirrored
/// identically in the HIER module, the base module and this host mirror). `None` performs no
/// injection — every existing call site.
///
/// Returns the per-froxel index grid — same shape and ordering contract as
/// [`golden_cluster_cull`] (flat-indexed by [`golden_cluster_index`], ascending table order,
/// clamped to `cfg.max_lights_per_cluster`) — alongside [`HierCullStats`] for the selectivity
/// gate. `header`/`cfg` describe a BOOT-sourced dispatch with no boot/live skew (D11's skew class
/// is H3's concern, on device; this mirror always uses `cfg.cluster_count()` as the write-bound
/// `capacity`, matching a well-formed, non-skewed frame).
pub fn golden_cluster_cull_hier(
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    cfg: &GoldenClusterConfig,
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    inject_nan_froxel: Option<u32>,
) -> (Vec<Vec<u32>>, HierCullStats) {
    let capacity = cfg.cluster_count();
    let mut grid: Vec<Vec<u32>> = vec![Vec::new(); capacity as usize];

    let ps_begin = header.l0a_count();
    let ps_room = HIER_MASK_BITS.saturating_sub(ps_begin);
    let ps_total = header.light_count().saturating_sub(ps_begin);
    let ps_n = ps_total.min(ps_room);

    let gps = golden_hier_groups_per_slice(cfg.dim_x, cfg.dim_y);
    let groups = gps * cfg.dim_z;
    let mut stats = HierCullStats {
        groups,
        ps_n,
        valid_lanes: 0,
        pairs_coarse: u64::from(groups) * u64::from(ps_n),
        pairs_fine: 0,
        group_coarse_accept: Vec::with_capacity(groups as usize),
    };

    let nan = f32::from_bits(GOLDEN_QUIET_NAN_BITS);
    let mut own_min = [[0.0_f32; 3]; HIER_GROUP_THREADS as usize];
    let mut own_max = [[0.0_f32; 3]; HIER_GROUP_THREADS as usize];
    let mut lane_valid = [false; HIER_GROUP_THREADS as usize];
    let mut lane_fi = [0_u32; HIER_GROUP_THREADS as usize];
    let mut coarse_mask = [false; HIER_MASK_BITS as usize];

    for group_id in 0..groups {
        // Phase 0/1: every lane's own AABB (or the injected poison), then D8's substitution,
        // folded in place into the group's coarse box — fold order is irrelevant (D2/D9).
        let mut coarse_min = [1.0e30_f32; 3];
        let mut coarse_max = [-1.0e30_f32; 3];
        for lane in 0..HIER_GROUP_THREADS {
            let (x, y, z, fi, valid) =
                golden_hier_thread_map(group_id, lane, cfg.dim_x, cfg.dim_y, cfg.dim_z, capacity);
            let li = lane as usize;
            lane_valid[li] = valid;
            lane_fi[li] = fi;

            let (store_min, store_max) = if valid {
                stats.valid_lanes += 1;
                let (mut amin, mut amax) = golden_froxel_aabb(x, y, z, img_w, img_h, camera, cfg);
                if inject_nan_froxel == Some(fi) {
                    amin = [nan; 3];
                    amax = [nan; 3];
                }
                own_min[li] = amin;
                own_max[li] = amax;
                let finite = amin.iter().chain(amax.iter()).all(|c| c.abs() <= 1.0e30);
                if finite { (amin, amax) } else { ([-f32::MAX; 3], [f32::MAX; 3]) }
            } else {
                ([1.0e30_f32; 3], [-1.0e30_f32; 3])
            };

            for i in 0..3 {
                coarse_min[i] = coarse_min[i].min(store_min[i]);
                coarse_max[i] = coarse_max[i].max(store_max[i]);
            }
        }

        // Phase 4: the coarse scan is group-uniform, so it runs once per group, not once per
        // lane (striping across 256 lanes is a parallelism detail with no effect on the result).
        let ps_n_usize = ps_n as usize;
        for slot in coarse_mask.iter_mut().take(ps_n_usize) {
            *slot = false;
        }
        let mut e_coarse = 0_u64;
        for j in 0..ps_n {
            let i = ps_begin + j;
            let lgt = &lights[i as usize];
            let kind = lgt.kind();
            if kind != GOLDEN_LIGHT_KIND_POINT && kind != GOLDEN_LIGHT_KIND_SPOT {
                continue;
            }
            let pos = [lgt.pos_range[0], lgt.pos_range[1], lgt.pos_range[2]];
            let r = lgt.pos_range[3];
            if golden_sq_dist_point_aabb(pos, coarse_min, coarse_max) <= r * r {
                coarse_mask[j as usize] = true;
                e_coarse += 1;
            }
        }
        debug_assert!(e_coarse <= u64::from(ps_n), "invariant: e_coarse cannot exceed ps_n");
        stats.group_coarse_accept.push(e_coarse as u32);

        // Phase 5/6: every VALID lane walks the SAME coarse mask, ascending, against its own
        // (pre-substitution) AABB — table order, identical to the flat arm's range.
        let mut valid_count = 0_u64;
        for lane in 0..HIER_GROUP_THREADS {
            let li = lane as usize;
            if !lane_valid[li] {
                continue;
            }
            valid_count += 1;
            let cell = &mut grid[lane_fi[li] as usize];
            for j in 0..ps_n {
                if !coarse_mask[j as usize] {
                    continue;
                }
                let i = ps_begin + j;
                let lgt = &lights[i as usize];
                let kind = lgt.kind();
                if kind != GOLDEN_LIGHT_KIND_POINT && kind != GOLDEN_LIGHT_KIND_SPOT {
                    continue;
                }
                let pos = [lgt.pos_range[0], lgt.pos_range[1], lgt.pos_range[2]];
                let r = lgt.pos_range[3];
                if golden_sq_dist_point_aabb(pos, own_min[li], own_max[li]) <= r * r
                    && (cell.len() as u32) < cfg.max_lights_per_cluster
                {
                    cell.push(i);
                }
            }
        }
        stats.pairs_fine += valid_count * e_coarse;
    }

    (grid, stats)
}

/// The CPU mirror of the L1 CLUSTERED `deferred_pbr` resolve. Identical to
/// [`golden_deferred_resolve_table`] except the point/spot block is driven by the pixel's
/// froxel index SET (from [`golden_cluster_cull`]) instead of the flat `[l0a..light_count)`
/// range. When `header.clusters_enabled()` is false this DELEGATES to the brute-force table
/// resolve (the L1 0%-gate == L0b). The per-light shading expression is byte-identical to the
/// table resolve, so a cluster set that contains every in-range light reproduces it exactly.
#[allow(clippy::too_many_arguments)]
pub fn golden_deferred_resolve_clustered(
    attrs: MarcherAttributes,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    materials: &[GoldenMaterial],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    cfg: &GoldenClusterConfig,
    grid: &[Vec<u32>],
) -> u32 {
    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);
    // L1 OFF (or a non-lit pixel): the flat brute-force path (the 0%-gate).
    if !header.clusters_enabled() || attrs.mask != 1 {
        return golden_deferred_resolve_table(attrs, ro, rd, materials, header, lights);
    }

    let base = [
        attrs.base_rgb[0] as f32 / 255.0,
        attrs.base_rgb[1] as f32 / 255.0,
        attrs.base_rgb[2] as f32 / 255.0,
    ];
    let n = oct_decode([attrs.oct_rg[0] as f32 / 255.0, attrs.oct_rg[1] as f32 / 255.0]);
    let mat = materials
        .get(attrs.mat_id as usize)
        .copied()
        .unwrap_or_default();

    let metallic = mat.mrr[0];
    let roughness = mat.mrr[1].clamp(0.045, 1.0);
    let reflectance = mat.mrr[2];
    let a = roughness * roughness;
    let dielectric_f0 = 0.16 * reflectance * reflectance;
    let f0 = [
        dielectric_f0 + (base[0] - dielectric_f0) * metallic,
        dielectric_f0 + (base[1] - dielectric_f0) * metallic,
        dielectric_f0 + (base[2] - dielectric_f0) * metallic,
    ];
    let diffuse_color = [
        base[0] * (1.0 - metallic),
        base[1] * (1.0 - metallic),
        base[2] * (1.0 - metallic),
    ];

    let v = [-rd[0], -rd[1], -rd[2]];
    let nov = v_dot(n, v).max(1e-4);
    let shadow = attrs.shadow as f32 / 255.0;
    let ao = attrs.ao as f32 / 255.0;
    let pi = core::f32::consts::PI;
    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    let hemi = v_dot(n, UP) * 0.5 + 0.5;
    // PBR P0-D: the SAME per-pixel term the resolve hoists before its light loop, reused at
    // every specular site below (direct directional/point/spot + sky ambient).
    let dfg_v = env_brdf_approx(roughness, nov);
    let energy_comp = multi_scatter_energy_comp(dfg_v, f0);
    // PBR P1: the reflection vector, hoisted ONCE (mirrors the resolve's hoisted `R`) — feeds
    // BOTH the sky-gradient ambient specular below AND the per-directional HDR sun-disc term.
    // reflect(-v, n) == reflect(rd, n) since v == -rd (double negation is exact).
    let r = v_reflect(rd, n);
    // PBR metal fix: decoupled specular occlusion, hoisted once per pixel (see
    // `specular_ao`'s doc) and reused at every ambient-specular site below.
    let spec_ao = specular_ao(nov, roughness, ao);

    // The no-`P` front block (directionals + sky) is GLOBAL — identical to the table resolve.
    let mut lit_direct = [0.0_f32; 3];
    let mut ambient = [0.0_f32; 3];
    let l0a = header.l0a_count() as usize;
    for li in lights.iter().take(l0a) {
        match li.kind() {
            GOLDEN_LIGHT_KIND_DIRECTIONAL => {
                let l = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
                let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
                let nol = v_dot(n, l).max(0.0);
                let noh = v_dot(n, hvec).clamp(0.0, 1.0);
                let loh = v_dot(l, hvec).clamp(0.0, 1.0);
                let d_term = d_ggx(noh, a);
                let v_term = v_smith_ggx_correlated(nov, nol, a);
                let f_term = f_schlick(loh, f0);
                // PBR P1: the HDR sun-disc kernel for THIS directional light, sampled along
                // `r` (not `l`) — the analytic environment's bright-sun response.
                let sun_k = sun_kernel(r, l, a);
                for c in 0..3 {
                    let spec = d_term * v_term * f_term[c] * energy_comp[c]; // PBR P0-D
                    let diff = diffuse_color[c] * (1.0 / pi);
                    lit_direct[c] += (diff + spec) * (nol * shadow) * li.color_cone[c];
                    // PBR P1: a SECOND, roughness-widened specular response from this SAME
                    // light, added to the ambient — NOT shadow-modulated (AO-gated only,
                    // mirroring the sky ambient specular's own AO gate below).
                    let sun_spec = (f0[c] * dfg_v[0] + dfg_v[1]) * li.color_cone[c] * sun_k * energy_comp[c] * SUN_ENV_WEIGHT;
                    ambient[c] += sun_spec * spec_ao;
                }
            }
            GOLDEN_LIGHT_KIND_SKY => {
                // PBR P0-B: ambient specular samples the sky/ground gradient along `R` (PBR
                // P1: hoisted above the loop as `r`) instead of the flat sky color; diffuse
                // stays along n.
                let sky = [li.color_cone[0], li.color_cone[1], li.color_cone[2]];
                let ground = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
                let refl_hemi_lin = v_dot(r, UP) * 0.5 + 0.5;
                // PBR metal fix: steepen the reflected hemisphere (smoothstep) so a metal
                // sweeps a real bright-cap -> dark-belly gradient instead of a flat mid-tone.
                // The DIFFUSE `hemi` above stays LINEAR — only the specular lobe steepens.
                let refl_hemi = refl_hemi_lin * refl_hemi_lin * (3.0 - 2.0 * refl_hemi_lin);
                for c in 0..3 {
                    let hemi_c = ground[c] + (sky[c] - ground[c]) * hemi;
                    let refl_c = ground[c] + (sky[c] - ground[c]) * refl_hemi;
                    let spec_ambient = (f0[c] * dfg_v[0] + dfg_v[1]) * refl_c * energy_comp[c];
                    let diff_ambient = diffuse_color[c] * hemi_c;
                    ambient[c] += diff_ambient * ao + spec_ambient * spec_ao;
                }
            }
            _ => {}
        }
    }

    // L1: map the pixel to its froxel and loop ONLY that cluster's point/spot indices. The
    // froxel z-slice uses the SAME view-z the cull used.
    let p = [
        ro[0] + rd[0] * attrs.view_t,
        ro[1] + rd[1] * attrs.view_t,
        ro[2] + rd[2] * attrs.view_t,
    ];
    let view_z = match camera {
        CompositeCamera::Perspective { forward, .. } => v_dot(rd, forward) * attrs.view_t,
        CompositeCamera::Ortho => attrs.view_t,
    };
    let (tx, ty) = golden_cluster_xy_tile(px, py, img_w, img_h, cfg);
    let zsl = golden_cluster_z_slice(view_z, cfg);
    let cluster = golden_cluster_index(tx, ty, zsl, cfg.dim_x, cfg.dim_z) as usize;
    let slice = grid.get(cluster).map(Vec::as_slice).unwrap_or(&[]);
    for &j in slice {
        let li = &lights[j as usize];
        let kind = li.kind();
        let pos = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
        let range = li.pos_range[3];
        let to_l = [pos[0] - p[0], pos[1] - p[1], pos[2] - p[2]];
        let d2 = v_dot(to_l, to_l);
        let range2 = range * range;
        if d2 > range2 {
            continue;
        }
        let inv_d = 1.0 / d2.max(1e-8).sqrt();
        let l = [to_l[0] * inv_d, to_l[1] * inv_d, to_l[2] * inv_d];
        let win = (1.0 - (d2 * d2) / (range2 * range2)).clamp(0.0, 1.0);
        let mut atten = (1.0 / d2.max(1e-4)) * win * win;
        if kind == GOLDEN_LIGHT_KIND_SPOT {
            let (cos_inner, cos_outer) = golden_unpack_cones(li.color_cone[3]);
            let spot_dir = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
            let cos_a = v_dot([-l[0], -l[1], -l[2]], spot_dir);
            let denom = (cos_inner - cos_outer).max(1e-4);
            let tt = ((cos_a - cos_outer) / denom).clamp(0.0, 1.0);
            atten *= tt * tt;
        }
        let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
        let nol = v_dot(n, l).max(0.0);
        let noh = v_dot(n, hvec).clamp(0.0, 1.0);
        let loh = v_dot(l, hvec).clamp(0.0, 1.0);
        let d_term = d_ggx(noh, a);
        let v_term = v_smith_ggx_correlated(nov, nol, a);
        let f_term = f_schlick(loh, f0);
        for c in 0..3 {
            let spec = d_term * v_term * f_term[c] * energy_comp[c]; // PBR P0-D
            let diff = diffuse_color[c] * (1.0 / pi);
            lit_direct[c] += (diff + spec) * (nol * shadow) * atten * li.color_cone[c];
        }
    }

    let exposure = header.exposure();
    let mut lit = [0.0_f32; 3];
    for c in 0..3 {
        lit[c] = (lit_direct[c] + ambient[c] + mat.emissive[c]) * exposure;
    }
    pack_rgba(tonemap_and_oetf(lit))
}

/// The P6 R1 MULTI-LIGHT SDF-shadow CPU mirror of the L1 CLUSTERED `deferred_pbr` resolve —
/// the `shadow_mode != 0` clustered oracle. Identical to [`golden_deferred_resolve_clustered`]
/// EXCEPT each per-light visibility is the per-caster shadow term (the same primary-directional
/// rule + ranged march + dominant-N cap + NoL skip as [`golden_deferred_resolve_table_shadowed`]).
/// When clusters are OFF (or a non-lit pixel) this DELEGATES to the flat shadowed table oracle.
///
/// # VB-P1-0: the host + GPU cull now agree
/// `golden_cluster_cull` masks via `GoldenLight::kind()`, and (since VB-P1-0) `cluster_cull.hlsl`
/// masks via `light_kind()` too, so a shadow-flagged / atlas-slotted punctual SURVIVES both culls
/// identically. R1's multi-light shadow GPU golden still exercises the NON-clustered path (a
/// harness-structure choice — its runner never dispatches `cluster_cull.hlsl`, not a cull-drop
/// workaround); this clustered oracle exists for parity (casting DIRECTIONALS + non-casting
/// clustered punctual lights match).
#[allow(clippy::too_many_arguments)]
pub fn golden_deferred_resolve_clustered_shadowed<F: Fn([f32; 3]) -> f32>(
    attrs: MarcherAttributes,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    materials: &[GoldenMaterial],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    cfg: &GoldenClusterConfig,
    grid: &[Vec<u32>],
    field: &F,
) -> u32 {
    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);
    // L1 OFF (or a non-lit pixel): the flat shadowed path.
    if !header.clusters_enabled() || attrs.mask != 1 {
        return golden_deferred_resolve_table_shadowed(attrs, ro, rd, materials, header, lights, field);
    }

    let base = [
        attrs.base_rgb[0] as f32 / 255.0,
        attrs.base_rgb[1] as f32 / 255.0,
        attrs.base_rgb[2] as f32 / 255.0,
    ];
    let n = oct_decode([attrs.oct_rg[0] as f32 / 255.0, attrs.oct_rg[1] as f32 / 255.0]);
    let mat = materials
        .get(attrs.mat_id as usize)
        .copied()
        .unwrap_or_default();

    let metallic = mat.mrr[0];
    let roughness = mat.mrr[1].clamp(0.045, 1.0);
    let reflectance = mat.mrr[2];
    let a = roughness * roughness;
    let dielectric_f0 = 0.16 * reflectance * reflectance;
    let f0 = [
        dielectric_f0 + (base[0] - dielectric_f0) * metallic,
        dielectric_f0 + (base[1] - dielectric_f0) * metallic,
        dielectric_f0 + (base[2] - dielectric_f0) * metallic,
    ];
    let diffuse_color = [
        base[0] * (1.0 - metallic),
        base[1] * (1.0 - metallic),
        base[2] * (1.0 - metallic),
    ];

    let v = [-rd[0], -rd[1], -rd[2]];
    let nov = v_dot(n, v).max(1e-4);
    let shadow = attrs.shadow as f32 / 255.0;
    let ao = attrs.ao as f32 / 255.0;
    let pi = core::f32::consts::PI;
    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    let hemi = v_dot(n, UP) * 0.5 + 0.5;
    // PBR P0-D: the SAME per-pixel term the resolve hoists before its light loop, reused at
    // every specular site below (direct directional/point/spot + sky ambient).
    let dfg_v = env_brdf_approx(roughness, nov);
    let energy_comp = multi_scatter_energy_comp(dfg_v, f0);
    // PBR P1: the reflection vector, hoisted ONCE (mirrors the resolve's hoisted `R`) — feeds
    // BOTH the sky-gradient ambient specular below AND the per-directional HDR sun-disc term.
    // reflect(-v, n) == reflect(rd, n) since v == -rd (double negation is exact).
    let r = v_reflect(rd, n);
    // PBR metal fix: decoupled specular occlusion, hoisted once per pixel (see
    // `specular_ao`'s doc) and reused at every ambient-specular site below.
    let spec_ao = specular_ao(nov, roughness, ao);

    let multi_light = header.shadow_mode() != 0;
    let p = [
        ro[0] + rd[0] * attrs.view_t,
        ro[1] + rd[1] * attrs.view_t,
        ro[2] + rd[2] * attrs.view_t,
    ];
    // Normal-offset start bias for the per-light ranged shadow march (mirrors the resolve's
    // `sdf_soft_shadow_ranged(P + n*SHADOW_NORMAL_BIAS, n, l, t_max)`).
    let pb = [
        p[0] + n[0] * SHADOW_NORMAL_BIAS,
        p[1] + n[1] * SHADOW_NORMAL_BIAS,
        p[2] + n[2] * SHADOW_NORMAL_BIAS,
    ];
    let mut marched = 0u32;

    let mut lit_direct = [0.0_f32; 3];
    let mut ambient = [0.0_f32; 3];
    let mut primary_dir_seen = false;
    let l0a = header.l0a_count() as usize;
    for li in lights.iter().take(l0a) {
        match li.kind() {
            GOLDEN_LIGHT_KIND_DIRECTIONAL => {
                let l = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
                let nol = v_dot(n, l).max(0.0);
                let mut vis = shadow;
                if !primary_dir_seen {
                    primary_dir_seen = true;
                } else if multi_light
                    && li.casts_sdf_shadow()
                    && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL
                    && nol > SHADOW_NDOTL_EPS
                {
                    vis = host_soft_shadow_ranged(pb, n, l, SDF_T_MAX, field);
                    marched += 1;
                }
                let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
                let noh = v_dot(n, hvec).clamp(0.0, 1.0);
                let loh = v_dot(l, hvec).clamp(0.0, 1.0);
                let d_term = d_ggx(noh, a);
                let v_term = v_smith_ggx_correlated(nov, nol, a);
                let f_term = f_schlick(loh, f0);
                // PBR P1: the HDR sun-disc kernel for THIS directional light, sampled along
                // `r` (not `l`) — the analytic environment's bright-sun response.
                let sun_k = sun_kernel(r, l, a);
                for c in 0..3 {
                    let spec = d_term * v_term * f_term[c] * energy_comp[c]; // PBR P0-D
                    let diff = diffuse_color[c] * (1.0 / pi);
                    lit_direct[c] += (diff + spec) * (nol * vis) * li.color_cone[c];
                    // PBR P1: a SECOND, roughness-widened specular response from this SAME
                    // light, added to the ambient — NOT shadow-modulated (AO-gated only,
                    // mirroring the sky ambient specular's own AO gate below).
                    let sun_spec = (f0[c] * dfg_v[0] + dfg_v[1]) * li.color_cone[c] * sun_k * energy_comp[c] * SUN_ENV_WEIGHT;
                    ambient[c] += sun_spec * spec_ao;
                }
            }
            GOLDEN_LIGHT_KIND_SKY => {
                // PBR P0-B: ambient specular samples the sky/ground gradient along `R` (PBR
                // P1: hoisted above the loop as `r`) instead of the flat sky color; diffuse
                // stays along n.
                let sky = [li.color_cone[0], li.color_cone[1], li.color_cone[2]];
                let ground = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
                let refl_hemi_lin = v_dot(r, UP) * 0.5 + 0.5;
                // PBR metal fix: steepen the reflected hemisphere (smoothstep) so a metal
                // sweeps a real bright-cap -> dark-belly gradient instead of a flat mid-tone.
                // The DIFFUSE `hemi` above stays LINEAR — only the specular lobe steepens.
                let refl_hemi = refl_hemi_lin * refl_hemi_lin * (3.0 - 2.0 * refl_hemi_lin);
                for c in 0..3 {
                    let hemi_c = ground[c] + (sky[c] - ground[c]) * hemi;
                    let refl_c = ground[c] + (sky[c] - ground[c]) * refl_hemi;
                    let spec_ambient = (f0[c] * dfg_v[0] + dfg_v[1]) * refl_c * energy_comp[c];
                    let diff_ambient = diffuse_color[c] * hemi_c;
                    ambient[c] += diff_ambient * ao + spec_ambient * spec_ao;
                }
            }
            _ => {}
        }
    }

    let view_z = match camera {
        CompositeCamera::Perspective { forward, .. } => v_dot(rd, forward) * attrs.view_t,
        CompositeCamera::Ortho => attrs.view_t,
    };
    let (tx, ty) = golden_cluster_xy_tile(px, py, img_w, img_h, cfg);
    let zsl = golden_cluster_z_slice(view_z, cfg);
    let cluster = golden_cluster_index(tx, ty, zsl, cfg.dim_x, cfg.dim_z) as usize;
    let slice = grid.get(cluster).map(Vec::as_slice).unwrap_or(&[]);
    for &j in slice {
        let li = &lights[j as usize];
        let kind = li.kind();
        let pos = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
        let range = li.pos_range[3];
        let to_l = [pos[0] - p[0], pos[1] - p[1], pos[2] - p[2]];
        let d2 = v_dot(to_l, to_l);
        let range2 = range * range;
        if d2 > range2 {
            continue;
        }
        let inv_d = 1.0 / d2.max(1e-8).sqrt();
        let l = [to_l[0] * inv_d, to_l[1] * inv_d, to_l[2] * inv_d];
        let win = (1.0 - (d2 * d2) / (range2 * range2)).clamp(0.0, 1.0);
        let mut atten = (1.0 / d2.max(1e-4)) * win * win;
        if kind == GOLDEN_LIGHT_KIND_SPOT {
            let (cos_inner, cos_outer) = golden_unpack_cones(li.color_cone[3]);
            let spot_dir = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
            let cos_a = v_dot([-l[0], -l[1], -l[2]], spot_dir);
            let denom = (cos_inner - cos_outer).max(1e-4);
            let tt = ((cos_a - cos_outer) / denom).clamp(0.0, 1.0);
            atten *= tt * tt;
        }
        let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
        let nol = v_dot(n, l).max(0.0);
        let noh = v_dot(n, hvec).clamp(0.0, 1.0);
        let loh = v_dot(l, hvec).clamp(0.0, 1.0);
        let d_term = d_ggx(noh, a);
        let v_term = v_smith_ggx_correlated(nov, nol, a);
        let f_term = f_schlick(loh, f0);
        let mut vis = shadow;
        if multi_light
            && li.casts_sdf_shadow()
            && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL
            && nol > SHADOW_NDOTL_EPS
        {
            let t_max = d2.sqrt();
            vis = host_soft_shadow_ranged(pb, n, l, t_max, field);
            marched += 1;
        }
        for c in 0..3 {
            let spec = d_term * v_term * f_term[c] * energy_comp[c]; // PBR P0-D
            let diff = diffuse_color[c] * (1.0 / pi);
            lit_direct[c] += (diff + spec) * (nol * vis) * atten * li.color_cone[c];
        }
    }

    let exposure = header.exposure();
    let mut lit = [0.0_f32; 3];
    for c in 0..3 {
        lit[c] = (lit_direct[c] + ambient[c] + mat.emissive[c]) * exposure;
    }
    pack_rgba(tonemap_and_oetf(lit))
}

/// Reconstructs the `(ray_origin, ray_dir)` for the coarse ray through tile
/// `(tx, ty)`'s TRUE geometric center, derived line-for-line from [`composite_ray`]'s
/// EXACT arithmetic so the host + the shader emit identical ops (D1).
///
/// Tile `(tx, ty)` covers fine pixels `[tx*8 .. tx*8+7]²`; its center fine pixel is
/// `px_c = tx*8 + 3.5`, so the fine ray-gen's `(px + 0.5)` becomes `tx*8 + 4.0`
/// (3.5 + 0.5 = 4.0 exact in fp). This is NOT half-res-grid sampling
/// (`(tx + 0.5) / (w / 8)` is not fp-identical and would drift the center, eating
/// the cone margin). The result is the SAME `ro`/`rd` the fine marcher would shoot
/// for a (fractional) center pixel under `camera` — ortho or perspective.
#[inline]
pub(crate) fn coarse_ray(
    tx: u32,
    ty: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> ([f32; 3], [f32; 3]) {
    // `px_c + 0.5 = tx*8 + 4.0` and `py_c + 0.5 = ty*8 + 4.0` (the +0.5 of the fine
    // ray-gen folded into the exact-fp tile center). The rest is `composite_ray`'s
    // arithmetic byte-for-byte.
    let cx = (tx * TILE_SIZE) as f32 + 4.0;
    let cy = (ty * TILE_SIZE) as f32 + 4.0;
    match camera {
        CompositeCamera::Ortho => {
            let u = (cx / (img_w as f32)) * 2.0 - 1.0;
            let v = -((cy / (img_h as f32)) * 2.0 - 1.0);
            let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
            let rd = [0.0, 0.0, -1.0];
            (ro, rd)
        }
        CompositeCamera::Perspective {
            eye,
            forward,
            right,
            up,
            tan_half_fov,
            aspect,
        } => {
            let ndc_x = (cx / (img_w as f32)) * 2.0 - 1.0;
            let ndc_y = -((cy / (img_h as f32)) * 2.0 - 1.0);
            let sx = ndc_x * aspect * tan_half_fov;
            let sy = ndc_y * tan_half_fov;
            let dir = [
                forward[0] + right[0] * sx + up[0] * sy,
                forward[1] + right[1] * sx + up[1] * sy,
                forward[2] + right[2] * sx + up[2] * sy,
            ];
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            let rd = [dir[0] / len, dir[1] / len, dir[2] / len];
            (eye, rd)
        }
    }
}

/// The ORTHO cone radius — a constant-radius cylinder enclosing the 8×8 tile's
/// fine-pixel footprint with one full pixel of fp-ULP-safe margin (D2).
///
/// Parallel ortho rays → a constant-radius cylinder around the tile-center axis.
/// The fine ortho ray-gen maps `u = (px+0.5)/w·2−1` (world X pitch `Δx = (2/w)·HE`)
/// and `v` over `h` (world Y pitch `Δy = (2/h)·HE`), so a non-square image has
/// `Δx ≠ Δy`; the enclosing-cylinder radius must use the LARGER pitch
/// `Δ = (2 / min(w,h)) · HE`. The tight footprint-enclosing radius is
/// `sqrt(2) · 4 · Δ = sqrt(2) · (8/min(w,h)) · HE` (center-to-corner-center
/// `sqrt(2)·3.5·Δ` plus the half-pixel footprint `sqrt(2)·0.5·Δ`). A FULL extra
/// pixel of slack gives `r_ortho = sqrt(2) · (9/min(w,h)) · HE` (the old `(8/w)` was
/// zero-margin, a C1 hole; per design D2 — generalized from `w` to `min(w,h)` so a
/// non-square ortho extent stays conservative, BYTE-IDENTICAL at the square golden
/// where `min(w,h) == w`).
#[inline]
pub(crate) fn ortho_cone_radius(img_w: u32, img_h: u32) -> f32 {
    let min_wh = img_w.min(img_h) as f32;
    core::f32::consts::SQRT_2 * (9.0 / min_wh) * SDF_HALF_EXTENT
}

/// The PER-TILE perspective cone half-angle (radians) from the exact ray-gen (D3):
/// the max over the tile's 4 corner pixels' OUTER-EDGE directions of the angle to
/// the tile-center direction, plus [`ALPHA_MARGIN`].
///
/// `alpha_tile = max_i acos(dot(d_center, d_corner_edge_i))` where each direction is
/// the exact perspective ray-gen `dir = forward + right·(ndc_x·aspect·tan) +
/// up·(ndc_y·tan)`, normalized. The 4 corners use the tile footprint's OUTER edges
/// (`px = tx*8 − 0.5 .. tx*8 + 7.5` → `(px+0.5) = tx*8 .. tx*8 + 8`), which capture
/// the per-pixel footprint via the ±4.0-from-center offset, the aspect anisotropy,
/// AND the tan-convexity of edge tiles (a scalar `4√2·center-angle` under-encloses
/// → holes). The half-angle is per-tile (tighter `near_t`, same shader cost).
///
/// `camera` MUST be [`CompositeCamera::Perspective`] (the eye is unused for the
/// half-angle — directions only; the basis + `tan_half_fov` + `aspect` are read from
/// it). An ORTHO camera is not a perspective cone (the callers gate on the camera mode
/// and use [`ortho_cone_radius`] instead) and returns [`ALPHA_MARGIN`] (a degenerate
/// zero-angle cone) — but no caller passes one.
#[inline]
pub(crate) fn perspective_alpha_tile(tx: u32, ty: u32, img_w: u32, img_h: u32, camera: CompositeCamera) -> f32 {
    let CompositeCamera::Perspective {
        forward,
        right,
        up,
        tan_half_fov,
        aspect,
        ..
    } = camera
    else {
        return ALPHA_MARGIN;
    };
    // The exact perspective ray direction for a (fractional) pixel whose
    // `(px + 0.5)` sample is `sx_px` and `(py + 0.5)` is `sy_px`, normalized — the
    // same op sequence as `composite_ray`'s perspective arm.
    let dir_for = |sx_px: f32, sy_px: f32| -> [f32; 3] {
        let ndc_x = (sx_px / (img_w as f32)) * 2.0 - 1.0;
        let ndc_y = -((sy_px / (img_h as f32)) * 2.0 - 1.0);
        let sx = ndc_x * aspect * tan_half_fov;
        let sy = ndc_y * tan_half_fov;
        let dir = [
            forward[0] + right[0] * sx + up[0] * sy,
            forward[1] + right[1] * sx + up[1] * sy,
            forward[2] + right[2] * sx + up[2] * sy,
        ];
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        [dir[0] / len, dir[1] / len, dir[2] / len]
    };

    // The tile-center axis: `(px_c + 0.5) = tx*8 + 4.0` (matches `coarse_ray`).
    let cx = (tx * TILE_SIZE) as f32 + 4.0;
    let cy = (ty * TILE_SIZE) as f32 + 4.0;
    let d_center = dir_for(cx, cy);

    // The 4 corner OUTER edges: `(px + 0.5)` at `tx*8 + 0.0` (left/top outer) and
    // `tx*8 + 8.0` (right/bottom outer) — the footprint's outermost sample points.
    let lo_x = (tx * TILE_SIZE) as f32;
    let hi_x = (tx * TILE_SIZE) as f32 + (TILE_SIZE as f32);
    let lo_y = (ty * TILE_SIZE) as f32;
    let hi_y = (ty * TILE_SIZE) as f32 + (TILE_SIZE as f32);

    let mut max_angle = 0.0_f32;
    for &(sxp, syp) in &[(lo_x, lo_y), (hi_x, lo_y), (lo_x, hi_y), (hi_x, hi_y)] {
        let dc = dir_for(sxp, syp);
        let cos = (d_center[0] * dc[0] + d_center[1] * dc[1] + d_center[2] * dc[2]).clamp(-1.0, 1.0);
        let angle = cos.acos();
        if angle > max_angle {
            max_angle = angle;
        }
    }
    max_angle + ALPHA_MARGIN
}

/// The host cone-trace mirror (Algorithm A): computes the [`TileBound`] for tile
/// `(tx, ty)` from the per-tile mesh depths + the edit-list field, EXACTLY as
/// `sdf_tile_cull.hlsl` does. The single source of truth the host conservative-
/// invariant tests + the GPU `Tiles`-buffer agreement check assert against.
///
/// `tile_depths` is the 8×8 block of per-pixel mesh depths covering the tile (the
/// fine `mesh_depth` values, clear `1.0` outside the mesh / out of image range); the
/// caller supplies them in any order (only the MAX is read — D5). The algorithm:
///   1. `coarse_ray` (D1) → the tile-center axis.
///   2. `far_t = min(max over the depths of depth→t, T_MAX)` (D5: a cleared /
///      out-of-range texel decodes to `T_MAX`, so a partial-edge tile bounds at
///      `T_MAX`, not clamp-to-edge). The covered-texel decode is CAMERA-AWARE:
///      PERSPECTIVE uses [`MESH_DEPTH_T_MAX`] (64, decoupled from `T_MAX` so far
///      raster geometry doesn't saturate to no-mesh), ORTHO uses `T_MAX` (its MVP
///      bakes it).
///   3. The cone-aware march (D4): at `t`, `d = field`, cone radius `r(t)` (ortho:
///      `r_const`; perspective: `t · tan(alpha_safe)`); budget `= d/L − r(t)`. When
///      the budget `<= EPS_COARSE` RECORD `near_t = t` and STOP (cone-entry). Else
///      advance `t += budget / (1 + tan(alpha_safe))` (ortho: `/(1+0)`).
///   4. Reaching `far_t` (or `T_MAX`) without cone-entry ⇒ EMPTY (`near_t = 0`,
///      flags = `TILE_FLAG_EMPTY`); exhausting `MAX_IT_COARSE` ⇒ NON-empty,
///      `near_t = 0` (the safe full-march fallback — NEVER `near_t = last_t`).
///
/// `near_t` is clamped to `[0, far_t]`; an EMPTY tile has `near_t == 0`.
pub fn golden_tile_bound(
    edits: &[SdfEdit],
    tile_depths: &[f32],
    tx: u32,
    ty: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> TileBound {
    let (ro, rd) = coarse_ray(tx, ty, img_w, img_h, camera);

    // far_t = min(max over the 8×8 depth texels of depth→t, T_MAX). A cleared
    // (>= MESH_DEPTH_CLEAR) texel decodes to T_MAX (conservative: no mesh bound). The
    // covered-texel decode is CAMERA-AWARE (mirrors the shader's `mesh_norm` /
    // `sdf_gbuffer_composite.hlsl`'s `mesh_norm`): PERSPECTIVE uses [`MESH_DEPTH_T_MAX`]
    // (64), ORTHO uses [`depth_to_t`] (`d * SDF_T_MAX`, its MVP bakes `SDF_T_MAX`).
    let mesh_norm = match camera {
        CompositeCamera::Perspective { .. } => MESH_DEPTH_T_MAX,
        CompositeCamera::Ortho => SDF_T_MAX,
    };
    let mut max_t_mesh = 0.0_f32;
    for &md in tile_depths {
        let t_mesh = if md < MESH_DEPTH_CLEAR { md * mesh_norm } else { SDF_T_MAX };
        if t_mesh > max_t_mesh {
            max_t_mesh = t_mesh;
        }
    }
    let far_t = max_t_mesh.min(SDF_T_MAX);

    // The cone parameters: ortho → a constant radius, tan = 0; perspective → a
    // per-tile half-angle whose tangent grows the radius linearly with t.
    let (r_const, tan_a) = match camera {
        CompositeCamera::Ortho => (ortho_cone_radius(img_w, img_h), 0.0_f32),
        CompositeCamera::Perspective { .. } => {
            let alpha_safe = perspective_alpha_tile(tx, ty, img_w, img_h, camera);
            (0.0_f32, alpha_safe.tan())
        }
    };

    let mut t = 0.0_f32;
    let mut near_t = 0.0_f32;
    let mut entered = false;
    let mut exhausted = true; // cleared when the loop breaks by cone-entry or far_t.
    for _ in 0..MAX_IT_COARSE {
        if t >= far_t {
            exhausted = false; // reached far_t without entering: EMPTY.
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        let r = r_const + t * tan_a; // ortho: r_const; perspective: t*tan(alpha_safe).
        let budget = d / FIELD_LIPSCHITZ_L - r;
        if budget <= EPS_COARSE {
            near_t = t;
            entered = true;
            exhausted = false;
            break;
        }
        t += budget / (1.0 + tan_a); // ortho: /(1+0); perspective: /(1+tan).
        if t > SDF_T_MAX {
            exhausted = false; // walked past T_MAX without entering: EMPTY.
            break;
        }
    }

    let (near_t, flags) = if entered {
        (near_t.clamp(0.0, far_t), 0u32)
    } else if exhausted {
        // MAX_IT_COARSE exhaustion ⇒ NON-empty, near_t = 0 (the safe full-march
        // fallback — NEVER near_t = last_t, which would skip a surface = a hole).
        (0.0, 0u32)
    } else {
        // Reached far_t / T_MAX without cone-entry ⇒ EMPTY (near_t = 0).
        (0.0, TILE_FLAG_EMPTY)
    };

    debug_assert!(
        (0.0..=far_t).contains(&near_t),
        "invariant: near_t {near_t} must be in [0, far_t={far_t}]"
    );
    debug_assert!(far_t <= SDF_T_MAX, "invariant: far_t {far_t} must be <= T_MAX");
    debug_assert!(
        flags & TILE_FLAG_EMPTY == 0 || near_t == 0.0,
        "invariant: an EMPTY tile must have near_t == 0"
    );

    TileBound { near_t, far_t, flags, _pad: 0 }
}

/// The culled fine marcher (Algorithm B): one composited pixel, gated by the tile's
/// [`TileBound`]. With `coarse_enabled == false` this is BIT-IDENTICAL to
/// [`golden_composite_pixel_ex`] (the `t = 0.0` seed, no cull prefix — the 0%-gate
/// anchor); with `coarse_enabled == true`:
///   * an EMPTY tile (flags & [`TILE_FLAG_EMPTY`]) skips the march and composites
///     the mesh / background directly (D6 — an EMPTY tile can still be MESH-covered);
///   * else the march SEEDS `t = near_t` (the proven-empty prefix is skipped).
///
/// The field eval + lighting are byte-shared with `golden_composite_pixel_ex` (this
/// wraps its body); only the `t` seed + the EMPTY fast-path are added (the
/// determinism boundary — INVIOLABLE). `tile` is the [`TileBound`] for the tile the
/// pixel belongs to (`golden_tile_bound` for tile `(px / 8, py / 8)`).
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_culled(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    coarse_enabled: bool,
    tile: TileBound,
) -> u32 {
    // Render B1: the ω = 1.0 forwarder. Stays BIT-IDENTICAL to the pre-B1 culled marcher
    // (the `_omega` variant's live path is the frozen plain loop at `omega == 1.0`), so
    // every existing caller is unchanged (the 0%-gate).
    golden_composite_pixel_culled_omega(
        edits,
        mesh_depth,
        px,
        py,
        img_w,
        img_h,
        camera,
        coarse_enabled,
        tile,
        1.0,
    )
}

/// Render B1 — the over-relaxation-aware culled fine marcher. Identical to
/// [`golden_composite_pixel_culled`] but threads `omega` through the march: the cull-off
/// arm delegates to [`golden_composite_pixel_ex_omega`], the EMPTY fast-path and the
/// `near_t` seed are preserved, and the non-EMPTY march mirrors the shader's Keinert
/// over-relaxation EXACTLY (gate, over-relaxed step, sor-fail exact retreat, frozen
/// else-arm). At `omega == 1.0` this is BIT-IDENTICAL to the pre-B1 path (the 0%-gate).
/// `omega` is expected in `[1.0, 1.99]` (the host runtime clamp).
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_culled_omega(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    coarse_enabled: bool,
    tile: TileBound,
    omega: f32,
) -> u32 {
    golden_composite_pixel_culled_omega_lit(
        edits, mesh_depth, px, py, img_w, img_h, camera, coarse_enabled, tile, omega, 0,
        DEFAULT_LIGHT_DIR,
    )
}

/// Render A1/A2 — the lighting-aware culled fine marcher. Identical to
/// [`golden_composite_pixel_culled_omega`] but threads `lighting_flags` + `light_dir`:
/// the cull-off arm delegates to [`golden_composite_pixel_ex_omega_lit`], and the
/// non-EMPTY march lights the SDF hit through [`host_shade`] (A1 shadow / A2 AO gated
/// by the flag bits). The EMPTY fast-path composites mesh / background ONLY (no SDF
/// surface ⇒ no shadow/AO), so it is unaffected by lighting. With `lighting_flags == 0`
/// this is BYTE-IDENTICAL to [`golden_composite_pixel_culled_omega`] (the 0%-gate); the
/// ON path mirrors the shader within ±3/255.
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_culled_omega_lit(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    coarse_enabled: bool,
    tile: TileBound,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
) -> u32 {
    // The OFF path is byte-identical to the un-culled marcher (the 0%-gate).
    if !coarse_enabled {
        return golden_composite_pixel_ex_omega_lit(
            edits, mesh_depth, px, py, img_w, img_h, camera, omega, lighting_flags, light_dir,
        );
    }

    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);
    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    // EMPTY fast-path (D6): no SDF surface in the cone in front of the deepest mesh,
    // but the pixel can still be MESH-covered → composite mesh / background (the
    // marcher's else-if(has_mesh)/else arms with hit = false). NOT blind background.
    if tile.flags & TILE_FLAG_EMPTY != 0 {
        let color = if has_mesh { MESH_COLOR } else { SDF_BACKGROUND };
        return pack_rgba(color);
    }

    // Non-EMPTY: SEED the march at the proven-empty prefix end. `near_t` is a
    // conservative lower bound on every in-tile pixel's first hit (the cull's
    // contract), so seeding `t = near_t` never skips this pixel's surface.
    let mut t = tile.near_t;
    let t_seed = t; // the ORIGINAL seed (near_t when culled) — Candidate C re-march re-seeds from it
    let mut omega = omega; // [1.0, 1.99]; sor-fail latches it to 1.0 for the rest of the ray
    let mut hit = false;
    let mut safe_t = 0.0_f32; // probe param remembered for an exact retreat
    let mut sor_prev = 0.0_f32; // previous probe's d
    let mut sor_step_prev = 0.0_f32; // previous over-relaxed step length
    // BUG-B1-HOLE-3 (Candidate C): the EXHAUSTION flag. True iff the fast loop runs ALL
    // SDF_MAX_IT iterations with NO break — ran out of budget mid-field (neither
    // converged, nor clear-miss `t > T_MAX`, nor mesh-occluded `t >= t_mesh`). Starts
    // `true`, cleared by EVERY in-loop break. Mirrors the shader.
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false; // mesh-occlusion termination — NOT budget exhaustion
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false; // converged — NOT budget exhaustion
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            // sor_fail: the over-step taken last iter overshot the previous unbounding
            // sphere (valid only for omega < 2). Lipschitz-aware (BUG-B1-HOLE-1): the
            // guaranteed-empty radius at field value `f` is `f / FIELD_LIPSCHITZ_L`, so the
            // spheres cover the step iff `sor_prev + d >= L * sor_step_prev`. Mirrors the
            // shader exactly. Kept byte-identical to `golden_composite_pixel_ex_omega`'s loop.
            //
            // The `it > 0` guard is LOAD-BEARING (do not remove): a sor-fail can only be
            // reached after at least one ACCEPTED over-relax step (it >= 1 ⟹ accepted >= 1),
            // which pre-pays the +1 retreat iteration in the budget proof.
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                // BUG-B1-HOLE-2: do NOT retreat to bare `safe_t` and re-probe (that re-evals
                // the field, costing +2 iters vs plain and overflowing the budget at the
                // MAX_IT cliff → a hole). RESUME the plain march one certified step past the
                // safe point: `safe_t` is the exact probe param, `sor_prev` the exact field
                // value there, so `safe_t + sor_prev` is precisely where a plain march lands
                // after probing safe_t — reusing the eval (no re-probe). One same-sign add
                // (both operands >= 0): no cancellation, unlike a `t - <correction>` form.
                // Net +1 iter vs plain, pre-paid by the >= 1 accepted over-step (it>0 guard).
                debug_assert!(it > 0, "B1 budget: a>=1 precondition");
                debug_assert!(sor_prev >= SDF_EPS); // safe-point field value >= EPS → retreat strictly advances
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                debug_assert!(t > safe_t, "B1 retreat must advance");
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d; // frozen plain arm — TEXTUALLY identical to the frozen loop
        }
        if t > SDF_T_MAX {
            exhausted = false; // clear-miss termination — NOT budget exhaustion
            break;
        }
    }

    // BUG-B1-HOLE-3 (Candidate C): the PROVABLY-hole-free fallback re-march, mirroring the
    // shader EXACTLY. On `exhausted` (ran all SDF_MAX_IT with no break), RE-MARCH from the
    // ORIGINAL seed (`near_t` here, the same seed the fast pass used) with a plain
    // omega = 1.0 sphere-trace and use ITS result. This second loop is the EXACT frozen
    // marcher body (`t += d`) seeded from `near_t`, so any surface the frozen culled path
    // hits within MAX_IT it hits here too → no hole, with NO step-count dependence. At
    // omega == 1.0 the fast pass IS the frozen plain loop, so on exhaustion this reproduces
    // the identical frozen (hit = false) result — the omega == 1.0 output is byte-unchanged.
    if exhausted {
        t = t_seed; // re-seed from the SAME original seed the fast pass used (near_t)
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d; // frozen plain step
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, light_dir, lighting_flags, &|q| {
            sdf_edit_list(edits, q)
        })
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

// ======================================================================
// SDFDDGI I0b — the `probe_sample` resolve-side irradiance-lookup HOST ORACLE.
//
// This is the CPU reference for the SDFDDGI per-pixel resolve GI sample (Decision D6:
// R11G11B10F-no-gamma → the resolve path is bit-exact against the GPU). It renders
// NOTHING and is NOT wired into any resolve (that is I3); it is the de-risk proof that
// the whole world→probe→trilinear→octahedral-direction→weight→accumulate chain is
// TRANSCENDENTAL-FREE (dot/max/sqrt/div/lerp/select only) BEFORE the shader is authored.
// I3 will add `probe_sample_gpu_eq_cpu_to_bits` (dispatch the I3 HLSL `probe_sample`,
// read the atlas back, and diff to THIS reference to bits — the same GPU-vs-CPU gate the
// SSAO / marcher mirrors carry). See `docs/RENDER-SDFDDGI-PLAN.md`
// ("Host-oracle bit-exactness", Decisions D2 / D6).
//
// # Transcendental-freedom (the load-bearing property — proven op-by-op)
//
// Every arithmetic op in the chain is in the ACCEPTED host-oracle set
// `{+ - * / abs min max clamp/saturate floor sqrt/normalize select/ternary}`:
//   * world→probe fractional index — `-` (P-origin), `*` (inv_spacing), `clamp`;
//   * base cell + trilinear frac — `floor` (the `floor` intrinsic — deterministic and NOT a
//     transcendental; GPU/CPU agree bit-for-bit here ONLY because the input is `clamp`ed to
//     `[0, dims-1] ≥ 0` on the line above, where `floor == trunc` and HLSL `floor` matches;
//     do NOT move `floor` before the clamp), `-` (frac = f - floor), `select` (corner pick);
//   * texel→tile-UV→direction — `+ 0.5`, `/ VALID_EXTENT`, `* 2 - 1`, then `oct_decode`
//     which is `abs / clamp / select / +/-/* ` ending in `v_normalize` (= `sqrt` + `/`);
//   * wrap/backface weight — `v_dot`, `+ 1`, `* 0.5`, `* self` (the square), `+ bias`,
//     `max(_, 0)`;
//   * Chebyshev — `-`, `* self`, `max(0, _)`, `+`, `/` (`var / (var + d²)`);
//   * accumulate / normalize — `+`, `*`, `/ (sum weight)` guarded by `max(_, eps)`,
//     `select` (sky fallback).
// NO `atan2`/`asin`/`acos`/`sin`/`cos`/`pow`/`exp`/`log` appears anywhere. VERDICT:
// transcendental-free — the resolve golden's op set STAYS host-oracle bit-exact-CAPABLE (no
// re-classify to GPU-only+tolerance is forced by a transcendental).
//
// # Host-vs-HLSL bit-parity is DEFERRED to the I3 GPU golden (NOT proven here)
//
// I0b proves transcendental-freedom + HOST math correctness. It does NOT prove the host
// `oct_decode` bit-matches the future I3 hand-written HLSL decode: there is NO `oct_decode`
// eDSL body (only an ENCODE body exists), so the decode has no eDSL reference to lock
// against — the HLSL decode is hand-authored like the marcher/SSAO oracles and is verified
// at I3 by `probe_sample_gpu_eq_cpu_to_bits` (dispatch the HLSL `probe_sample`, read the
// atlas back, diff to THIS host reference to bits). The host `oct_encode` reused for the
// round-trip parity check ALSO differs from the eDSL encode body by ≤2 ULP (`x*(1/s)` vs
// `x/s`; see `oct_encode_matches_edsl_within_2_ulp` in the I0b test). So "bit-exact" is the
// TARGET the I3 GPU golden certifies, not a property I0b asserts by itself.

/// The DDGI octahedral IRRADIANCE tile edge in texels (Decision D2: 8×8 tile). The tile
/// is `6×6` VALID interior texels ([`DDGI_IRR_VALID_EXTENT`]) plus a 1-texel border on
/// every side (the standard DDGI/RTXGI octahedral layout: `6 + 2 = 8`).
pub const DDGI_IRR_TILE_EDGE: u32 = 8;

/// The VALID interior edge of the irradiance tile in texels (Decision D2: `6×6` valid).
/// The octahedral map is parameterized across exactly these interior texels; the 1-texel
/// border is a wrap-copy for clamp-free bilinear addressing (the border WRAP-COPY / sampler
/// addressing is pinned at I7 — this I0b reference resolves the interior direction chain).
pub const DDGI_IRR_VALID_EXTENT: u32 = 6;

/// The 1-texel border width on each side of an octahedral tile (`(8 - 6) / 2 == 1`).
pub const DDGI_TILE_BORDER: u32 = 1;

/// The plan's per-probe wrap/backface weight small-bias `+0.2` — keeps a grazing-but-valid
/// probe (`dot(dirToProbe, n) ≈ -1`) from being fully zero-weighted, avoiding a hard cut
/// (`docs/RENDER-SDFDDGI-PLAN.md`, "Host-oracle bit-exactness": `((dot+1)*0.5)²+0.2`).
pub const DDGI_WRAP_WEIGHT_BIAS: f32 = 0.2;

/// The minimum summed weight before the normalize divides — guards the all-corner-zero /
/// fully-unconverged case against a `0/0` NaN (a `max(sum, eps)` select, accepted). Below
/// this the sample is treated as no coverage ⇒ the sky-ambient fallback.
pub const DDGI_MIN_SUM_WEIGHT: f32 = 1.0e-6;

/// Maps an octahedral IRRADIANCE-tile interior texel `(tx, ty)` (each in
/// `0..DDGI_IRR_VALID_EXTENT`) to the world-space DIRECTION its texel CENTER encodes — the
/// texel→tile-UV→`[-1,1]²`→`oct_decode` chain the I3 shader will author for probe-tile
/// construction and readback. The UV is the texel CENTER `(t + 0.5) / VALID_EXTENT` over the
/// `6×6` interior (border-exclusive; the border is the I7 wrap-copy of these interior texels).
///
/// The chain is transcendental-free: `+0.5`, `/VALID_EXTENT`, `*2-1` (the `[0,1]→[-1,1]`
/// remap `oct_decode` inverts), then [`oct_decode`] (`abs`/`clamp`/`select`/±/`*` +
/// `v_normalize`). Reuses the SAME [`oct_decode`] the G-buffer resolve uses — no re-derived
/// octahedral math.
///
/// # Host-vs-HLSL bit-parity is DEFERRED (not proven here)
///
/// There is no `oct_decode` eDSL body to lock this decode against; its bit-parity with the
/// I3 hand-written HLSL decode is certified at I3 by `probe_sample_gpu_eq_cpu_to_bits` (the
/// standard host-oracle discipline — the marcher/SSAO decode oracles work the same way).
/// I0b proves the HOST chain is transcendental-free + math-correct, nothing about the GPU
/// yet.
///
/// # Panics (debug)
///
/// Debug-asserts `tx < DDGI_IRR_VALID_EXTENT && ty < DDGI_IRR_VALID_EXTENT` — an
/// out-of-interior texel is a caller bug (the border texels are wrap-copies, never
/// parameterized directly).
pub fn ddgi_texel_direction(tx: u32, ty: u32) -> [f32; 3] {
    debug_assert!(
        tx < DDGI_IRR_VALID_EXTENT && ty < DDGI_IRR_VALID_EXTENT,
        "invariant: DDGI interior texel ({tx},{ty}) must be < DDGI_IRR_VALID_EXTENT {DDGI_IRR_VALID_EXTENT}"
    );
    let extent = DDGI_IRR_VALID_EXTENT as f32;
    // Texel CENTER → [0,1] tile UV → the [0,1]² pair oct_decode remaps to [-1,1]², decoded
    // to a unit direction.
    let u = (tx as f32 + 0.5) / extent;
    let v = (ty as f32 + 0.5) / extent;
    oct_decode([u, v])
}

/// Octahedral-ENCODES a unit direction into the `[0,1]²` tile UV — the inverse of
/// [`ddgi_texel_direction`]'s decode, the direction→texel mapping the I3 atlas WRITE + the
/// resolve READ use to address a probe tile. A thin `pub` seam over the crate-private
/// `oct_encode` (the SAME encode the G-buffer marcher writes, which backs the existing
/// `golden_marcher_attributes` normal goldens), so the I0b round-trip sanity test can reach
/// it without widening `oct_encode`.
///
/// # NOT bit-identical to the eDSL encode body (≤2 ULP)
///
/// `oct_encode` L1-normalizes by MULTIPLY-BY-RECIPROCAL (`inv = 1/s; n*inv`); the eDSL
/// `oct_encode_body::<EvalCf>` — and the committed HLSL `float3 / float` it is spliced from —
/// DIVIDE (`n / s`). `x*(1/s)` ≠ `x/s` in IEEE; the gap is ≤2 ULP over the DDGI texel sweep
/// (pinned by `oct_encode_matches_edsl_within_2_ulp` in the I0b test). `oct_encode`'s op
/// sequence is NOT changed here — altering it would break the byte-identity of the G-buffer
/// normal goldens that already depend on it. The encode is used only for the round-trip
/// SANITY check (tolerance, not bits); the real GPU parity is the I3 golden's job.
///
/// Transcendental-free: `abs`/`select`/±/`*`/`/` (`oct_encode`'s L1-normalize + fold) — no
/// transcendental in the direction→UV chain either.
#[inline]
pub fn ddgi_oct_encode(dir: [f32; 3]) -> [f32; 2] {
    oct_encode(dir)
}

/// A single probe's contribution to a [`probe_sample`] receiver — the RG16F depth moments
/// and the (already atlas-fetched) octahedral irradiance in the receiver-normal direction.
/// Modelled as an explicit struct so [`probe_sample`] takes the atlas read as an injected
/// dependency (no atlas is allocated until I1): the resolve fetches `irradiance` from the
/// irradiance atlas via `oct_encode(dir)` and `depth_mean`/`depth_mean2` from the depth
/// atlas at the same probe, exactly the two texture reads I3 will author.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DdgiProbeTap {
    /// The probe's octahedral irradiance in the receiver's chosen direction (the atlas
    /// texel `oct_encode(dir)` decodes to) — linear R11G11B10F, no gamma (Decision D6:
    /// the pow encode is DROPPED so this is a plain stored value).
    pub irradiance: [f32; 3],
    /// The depth tile's first moment `E[dist]` (mean distance to geometry along the probe's
    /// octahedral direction) — the RG16F `.r` lane.
    pub depth_mean: f32,
    /// The depth tile's second moment `E[dist²]` — the RG16F `.g` lane. `var = mean2 -
    /// mean²` feeds the Chebyshev visibility (two-moment leak suppression).
    pub depth_mean2: f32,
    /// Whether this probe has been written at least once (the converged-once bit, Decision
    /// D2 / storage-class 3). An unconverged probe contributes ZERO weight so the receiver
    /// falls back to sky-ambient until first coverage.
    pub converged: bool,
}

/// The DDGI resolve-side irradiance lookup HOST ORACLE (SDFDDGI I0b) — the bit-exact CPU
/// reference the I3 shader's `probe_sample` will be diffed against (`docs/RENDER-SDFDDGI-PLAN.md`,
/// Decision D6). Given a receiver world position `p` + normal `n`, the world-fixed grid
/// (`origin` / `inv_spacing` / `dims`, mirroring the `ResolvedDdgi` carrier that
/// `boyko_render::ddgi_config` derives — `inv_spacing_dims` there), and a per-probe tap
/// provider, returns the trilinearly-blended, wrap- + Chebyshev-weighted indirect
/// irradiance, or `sky_ambient` when no surrounding probe has coverage.
///
/// # The chain (all transcendental-free — see the module header proof)
///
/// 1. `world → probe`: `f = (p - origin) * inv_spacing`, clamped to `[0, dims-1]` so the base
///    cell + its `+1` neighbour stay in-bounds; `base = floor(f)`, `frac = f - base`.
/// 2. The 8 surrounding probes (`base + {0,1}³`) with TRILINEAR corner weights
///    (`lerp` factors `frac` / `1-frac` per axis, multiplied).
/// 3. Per probe: skip if `!converged` (contributes zero); else weight =
///    `trilinear · wrap · chebyshev`:
///    - wrap/backface `w = ((dot(dirToProbe, n) + 1) · 0.5)² + 0.2` (clamped `≥ 0`),
///    - Chebyshev `cheb = var / (var + max(0, dist - mean)²)` with `var = mean2 - mean²`
///      (`≥ 0`), `dist = |p - probe_pos|`; if `dist ≤ mean` the probe is unshadowed ⇒
///      `cheb = 1`.
/// 4. Accumulate `weight · irradiance`; normalize by `max(sum_weight, eps)`. If
///    `sum_weight < eps` (all corners out-of-bounds or unconverged) ⇒ `sky_ambient`.
///
/// `probe_pos(i)` returns probe `i`'s world position (`origin + i · spacing`) — the caller's
/// closure OWNS the `spacing` reconstruction, so `probe_sample` itself needs only
/// `inv_spacing` (`== 1/spacing`, mirroring `ResolvedDdgi::inv_spacing_dims[0]`) for the
/// world→probe fractional index. `tap(i, dir)` returns probe `i`'s [`DdgiProbeTap`] for the
/// octahedral `dir` (the atlas read, injected because the atlas is not allocated until I1).
///
/// # Determinism / stable accumulation order
///
/// The corner iteration order is fixed (`z` outer, `y`, `x` inner — the `i = ((z·dy)+y)·dx +
/// x` grid index order), so the floating-point accumulation order is stable and matches the
/// order the I3 GPU shader will unroll. No op leaves the accepted set. (Actual host-vs-GPU
/// bit-parity is certified by the I3 `probe_sample_gpu_eq_cpu_to_bits` golden, not here.)
#[allow(clippy::too_many_arguments)]
pub fn probe_sample(
    p: [f32; 3],
    n: [f32; 3],
    origin: [f32; 3],
    inv_spacing: f32,
    dims: [u32; 3],
    sky_ambient: [f32; 3],
    probe_pos: impl Fn([u32; 3]) -> [f32; 3],
    tap: impl Fn([u32; 3], [f32; 3]) -> DdgiProbeTap,
) -> [f32; 3] {
    // world → fractional probe coords, clamped so base and base+1 both stay in [0, dims-1].
    // A receiver outside the AABB clamps onto the boundary cell (a benign edge extrapolation);
    // fully-outside coverage still resolves to sky via the summed-weight guard when the
    // clamped corners are unconverged.
    let frac_coord = |axis: usize| -> f32 {
        let f = (p[axis] - origin[axis]) * inv_spacing;
        // Upper bound is dims-1 so the +1 neighbour is the last valid index; guard dims==0.
        let hi = (dims[axis].max(1) - 1) as f32;
        f.clamp(0.0, hi)
    };
    let fx = frac_coord(0);
    let fy = frac_coord(1);
    let fz = frac_coord(2);

    // Base cell + trilinear fractions. `floor` is the `floor` intrinsic — deterministic and
    // NOT a transcendental; it is GPU/CPU-agreement-safe here ONLY because `frac_coord`
    // clamped the input to `[0, dims-1] ≥ 0`, where `floor == trunc` and HLSL `floor` matches
    // bit-for-bit (do NOT move `floor` before that clamp).
    let bx = fx.floor();
    let by = fy.floor();
    let bz = fz.floor();
    let tx = fx - bx;
    let ty = fy - by;
    let tz = fz - bz;
    let (bx, by, bz) = (bx as u32, by as u32, bz as u32);

    let mut sum_irr = [0.0_f32; 3];
    let mut sum_w = 0.0_f32;

    // The 8 surrounding probes: z outer, y, x inner (the grid index order — a stable
    // accumulation order matching the shader's unroll).
    for cz in 0..2u32 {
        let wz = if cz == 0 { 1.0 - tz } else { tz };
        let pz = (bz + cz).min(dims[2].max(1) - 1);
        for cy in 0..2u32 {
            let wy = if cy == 0 { 1.0 - ty } else { ty };
            let py = (by + cy).min(dims[1].max(1) - 1);
            for cx in 0..2u32 {
                let wx = if cx == 0 { 1.0 - tx } else { tx };
                let px = (bx + cx).min(dims[0].max(1) - 1);
                let idx = [px, py, pz];

                let ppos = probe_pos(idx);
                // Direction receiver → probe (the wrap-weight axis). `to_probe` degenerate
                // (receiver AT the probe) → v_normalize returns ZERO ⇒ dot 0 ⇒ neutral
                // wrap weight, never a NaN.
                let to_probe = v_normalize(v_sub(ppos, p));

                let probe = tap(idx, n);
                if !probe.converged {
                    continue; // unconverged probe: zero weight (sky fallback until first write)
                }

                // Wrap / backface weight: ((dot + 1) * 0.5)² + bias, floored at 0.
                let facing = (v_dot(to_probe, n) + 1.0) * 0.5;
                let wrap = (facing * facing + DDGI_WRAP_WEIGHT_BIAS).max(0.0);

                // Chebyshev two-moment visibility: var = E[d²] - E[d]²; if the receiver is
                // nearer than the mean it is unshadowed (cheb = 1), else var / (var + Δ²).
                let mean = probe.depth_mean;
                let var = (probe.depth_mean2 - mean * mean).max(0.0);
                let dist = v_len(v_sub(p, ppos));
                let delta = (dist - mean).max(0.0);
                let cheb = if dist <= mean {
                    1.0
                } else {
                    var / (var + delta * delta).max(DDGI_MIN_SUM_WEIGHT)
                };

                let w = wx * wy * wz * wrap * cheb;
                sum_irr[0] += w * probe.irradiance[0];
                sum_irr[1] += w * probe.irradiance[1];
                sum_irr[2] += w * probe.irradiance[2];
                sum_w += w;
            }
        }
    }

    // Normalize by the summed weight; below the epsilon there was no coverage ⇒ sky fallback.
    if sum_w < DDGI_MIN_SUM_WEIGHT {
        return sky_ambient;
    }
    let inv = 1.0 / sum_w;
    [sum_irr[0] * inv, sum_irr[1] * inv, sum_irr[2] * inv]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SDF-lit `MarcherAttributes` (mask == 1) with plausible mid-range G-buffer bytes and a
    /// finite `view_t` — enough to exercise the full Cook-Torrance path in
    /// [`golden_deferred_resolve_with_pbr`] without any shadow/GI-mode header wiring (the
    /// zero-`LightHeader` default this oracle's degenerate-table siblings use).
    fn lit_attrs() -> MarcherAttributes {
        MarcherAttributes {
            base_rgb: [180, 120, 90],
            oct_rg: [140, 160],
            mat_id: 0,
            shadow: 255,
            ao: 200,
            mask: 1,
            view_t: 4.0,
        }
    }

    const RD: [f32; 3] = [0.0, 0.0, -1.0];

    /// Textured-PBR T6a: [`GoldenMaterial::new`] never sets the `mrr[3]` flag bit (every
    /// EXISTING oracle input), so [`golden_deferred_resolve_with_pbr`]'s override must be inert
    /// REGARDLESS of what `gpbr` sample is supplied — matching [`golden_deferred_resolve`]'s
    /// `None`-forwarding output exactly. This is the flag=0 byte-identity invariant T6a requires.
    #[test]
    fn textured_override_is_inert_when_flag_bit_unset() {
        let attrs = lit_attrs();
        let mats = [GoldenMaterial::new([0.6, 0.3, 0.2, 1.0], 0.2, 0.4, 0.5, [0.0, 0.0, 0.0])];
        assert_eq!(mats[0].mrr[3].to_bits() & GOLDEN_MATERIAL_FLAG_TEXTURED, 0);

        let without_pbr = golden_deferred_resolve(attrs, RD, &mats);
        // A deliberately DIFFERENT-looking sample: if the flag gate were broken (always-live),
        // this would visibly change the packed output.
        let with_pbr_but_flag_unset =
            golden_deferred_resolve_with_pbr(attrs, RD, &mats, Some([0.95, 0.05, 0.1, 3.0]));

        assert_eq!(
            without_pbr, with_pbr_but_flag_unset,
            "an Option::Some gPbr sample must be a no-op when mrr[3]'s flag bit is unset"
        );
    }

    /// The converse of the inertness test: WITH the flag bit set AND a `gpbr` sample supplied,
    /// the override must actually change the output relative to the unmodulated material — proof
    /// the gate is live, not merely always-false-by-construction.
    #[test]
    fn textured_override_changes_output_when_flag_bit_set_and_sample_supplied() {
        let attrs = lit_attrs();
        let mut textured = GoldenMaterial::new([0.6, 0.3, 0.2, 1.0], 0.2, 0.4, 0.5, [0.1, 0.05, 0.02]);
        textured.mrr[3] = f32::from_bits(GOLDEN_MATERIAL_FLAG_TEXTURED);
        let mats = [textured];

        let unmodulated = golden_deferred_resolve_with_pbr(attrs, RD, &mats, None);
        let modulated =
            golden_deferred_resolve_with_pbr(attrs, RD, &mats, Some([0.95, 0.05, 0.2, 4.0]));

        assert_ne!(
            unmodulated, modulated,
            "a live gPbr sample on a flag-set material must change the packed LIT output"
        );
    }

    /// `None` is a pure notational shorthand for "no sample" — passing it through
    /// `golden_deferred_resolve_with_pbr` directly must match the [`golden_deferred_resolve`]
    /// convenience wrapper bit-for-bit, on EVERY mask arm (lit + background).
    #[test]
    fn golden_deferred_resolve_is_exactly_the_none_wrapper() {
        let mats = [GoldenMaterial::default()];
        for attrs in [lit_attrs(), MarcherAttributes { mask: 0, ..lit_attrs() }] {
            assert_eq!(
                golden_deferred_resolve(attrs, RD, &mats),
                golden_deferred_resolve_with_pbr(attrs, RD, &mats, None)
            );
        }
    }

    /// W4 (VB-P1e H1 code review, plan §5 Case B / §8.3 "Mutation (vii) in full"): pins that
    /// Rust's `f32::max` reproduces GLSL.std.450 `NMax`'s non-NaN-preferring semantics EXACTLY
    /// for the all-NaN AABB the exactness proof's Case B depends on — "Rust's `f32::max` returns
    /// the non-NaN operand exactly as `NMax` does" was previously a plan-text review item, not a
    /// pin. Every axis of `sq_dist_point_aabb` computes `(min - c).max(c - max).max(0.0)`: with
    /// `min == max == NaN`, both `min - c` and `c - max` are NaN, `NaN.max(NaN)` returns NaN (no
    /// non-NaN operand exists), and `NaN.max(0.0)` returns the non-NaN operand `0.0` — so every
    /// axis contributes exactly `0.0`, and `F(d) == 0.0` for ANY finite center, matching `NMax`'s
    /// device semantics bit-for-bit (`golden_cluster_cull_hier`'s mitigated-arm equality claim,
    /// and this crate's `hier_cull_mutation_vii_host_leg_mitigated_arm_matches_flat`, rest on
    /// this holding).
    #[test]
    fn sq_dist_point_aabb_all_nan_aabb_matches_the_nmax_semantics_case_b_needs() {
        let nan = f32::from_bits(GOLDEN_QUIET_NAN_BITS); // asfloat(0x7FC00000u) -- the shader's bit pattern.
        assert_eq!(
            golden_sq_dist_point_aabb([1.5, -2.25, 100.0], [nan; 3], [nan; 3]),
            0.0,
            "an all-NaN AABB must give sq_dist == 0.0 for this finite center (§5 Case B's \
             absorbing element) -- if this pin moves, Rust's f32::max no longer agrees with NMax \
             and the mitigated-arm equality claim is unsound"
        );
        // A second, very different finite center: Case B claims this for EVERY finite center,
        // not merely a convenient one.
        assert_eq!(
            golden_sq_dist_point_aabb([-1.0e6, 0.0, 42.5], [nan; 3], [nan; 3]),
            0.0,
            "an all-NaN AABB must give sq_dist == 0.0 for ANY finite center, not just one sample"
        );
    }

    // ========================================================================================
    // P1-4 (VB-P1e H4, adversarial review): the production hierarchical-cull dispatch-shape
    // parity pin. `boyko_render::ClusterConfig::hier_group_count`/`hier_group_threads` (the
    // formula `GpuSceneBundles::build_froxel_light_cull` actually feeds `cmd_dispatch`) is a
    // THIRD independent copy of `cluster_cull.hlsl`'s own `gps` alongside this module's
    // `golden_hier_groups_per_slice`/`HIER_GROUP_THREADS` (H3's device-proven mirror) — nothing
    // previously asserted the two agree. `use boyko_render::ClusterConfig` is scoped to THIS
    // `#[cfg(test)]` module deliberately (not the `goldens` module above, which also compiles
    // under `feature = "goldens"` for external dependents): `boyko-render` is a dev-dependency
    // BACK-EDGE of this crate (`Cargo.toml`'s own comment on that entry), live only when
    // `boyko_rhi_vulkan` itself is under test.
    // ========================================================================================
    use boyko_render::ClusterConfig;

    /// One grid config in the parity matrix, dims-only (group-count parity does not depend on
    /// the camera or the light rig).
    struct ParityCase {
        dim_x: u32,
        dim_y: u32,
        dim_z: u32,
        label: &'static str,
    }

    /// The SAME grid matrix H3's device oracle sweeps
    /// (`lighting_l1_host_oracle.rs`'s `hier_matrix_cases`, M1/M2 collapsed to one entry since
    /// both share dims `16x9x24` and group-count parity is camera-independent): `gps=1`
    /// (M1/M2, the shipped default), `gps=1`-from-above (E1, `dim_x*dim_y == 256` exactly),
    /// `gps=2`-exact (E2), `gps=2`-ragged (E3, the guard-tail config), `gps=3`-exact (E4).
    fn well_formed_matrix() -> [ParityCase; 5] {
        [
            ParityCase { dim_x: 16, dim_y: 9, dim_z: 24, label: "M1/M2 16x9x24 gps=1" },
            ParityCase { dim_x: 16, dim_y: 16, dim_z: 24, label: "E1 16x16x24 gps=1-from-above" },
            ParityCase { dim_x: 32, dim_y: 16, dim_z: 24, label: "E2 32x16x24 gps=2-exact" },
            ParityCase { dim_x: 16, dim_y: 17, dim_z: 24, label: "E3 16x17x24 gps=2-ragged" },
            ParityCase { dim_x: 32, dim_y: 24, dim_z: 24, label: "E4 32x24x24 gps=3-exact" },
        ]
    }

    /// P1-4: pins [`ClusterConfig::hier_group_threads`]/[`ClusterConfig::hier_group_count`] —
    /// the PRODUCTION dispatch shape — against [`HIER_GROUP_THREADS`]/
    /// [`golden_hier_groups_per_slice`] — H3's own device-proven oracle — over the exact grid
    /// matrix H3 sweeps. If this ever fails, H3's on-hardware proof no longer covers the shape
    /// production actually dispatches.
    #[test]
    fn production_hier_dispatch_shape_matches_h3_device_oracle() {
        assert_eq!(
            ClusterConfig::hier_group_threads(),
            HIER_GROUP_THREADS,
            "invariant: the production workgroup width must equal H3's device-proven width"
        );
        for case in well_formed_matrix() {
            let cfg = ClusterConfig {
                dim_x: case.dim_x,
                dim_y: case.dim_y,
                dim_z: case.dim_z,
                ..ClusterConfig::default()
            };
            let golden_groups = golden_hier_groups_per_slice(case.dim_x, case.dim_y) * case.dim_z;
            assert_eq!(
                cfg.hier_group_count(),
                golden_groups,
                "{}: production hier_group_count() diverges from H3's \
                 golden_hier_groups_per_slice() * dim_z",
                case.label,
            );
        }
    }

    /// P1-4 follow-up: the degenerate `dim_x * dim_y == 0` case is a KNOWN, DOCUMENTED,
    /// INTENTIONAL divergence, not a bug — see [`ClusterConfig::hier_group_count`]'s own doc
    /// (D11/Rev 5 P2). Production dispatches ZERO groups (`cluster_count() == 0` means there is
    /// nothing to cull — every `ClusterGrid` write would be out of bounds anyway, so skipping
    /// the dispatch entirely is strictly safer than dispatching phantom work); the golden
    /// mirrors the SHADER's own `max(1, gps)` totality clamp (a D8 obligation for the shader's
    /// per-group math, independent of whether any lane's work is `valid`). This test PINS the
    /// divergence explicitly (STOP AND REPORT territory, per the adversarial review) so a
    /// future change that makes one side match the other is a deliberate, reviewed decision,
    /// not an accidental drift this suite would otherwise miss.
    #[test]
    fn degenerate_zero_dim_diverges_from_the_shader_totality_clamp_by_design() {
        let cfg = ClusterConfig { dim_x: 0, dim_y: 9, dim_z: 24, ..ClusterConfig::default() };
        let golden_groups = golden_hier_groups_per_slice(cfg.dim_x, cfg.dim_y) * cfg.dim_z;
        assert_eq!(
            cfg.hier_group_count(),
            0,
            "invariant: production dispatches zero groups when cluster_count() == 0"
        );
        assert_eq!(
            golden_groups, cfg.dim_z,
            "invariant: the golden's max(1, gps) clamp yields dim_z phantom groups"
        );
        assert_ne!(
            cfg.hier_group_count(),
            golden_groups,
            "P1-4: the degenerate-dims divergence is intentional (see \
             ClusterConfig::hier_group_count's own doc) -- if this now holds, the clamp \
             behavior changed and this pin must be re-reviewed, not deleted"
        );
    }
}
