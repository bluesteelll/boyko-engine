//! SDFDDGI I0b — the `probe_sample` resolve-side irradiance-lookup HOST ORACLE regression
//! (CPU-only — NO GPU required).
//!
//! This is the de-risk proof for the SDFDDGI per-pixel resolve GI sample: it exercises the
//! host `probe_sample` reference (`boyko_rhi_vulkan::goldens::probe_sample`) — the SAME
//! math the I3 shader will author — over the plan's world-fixed grid + octahedral tile
//! layout, BEFORE any GI logic ships. Per `docs/RENDER-SDFDDGI-PLAN.md` ("Host-oracle
//! bit-exactness", Decisions D2 / D6) the resolve path is R11G11B10F-no-gamma, so it MUST
//! be dot/max/sqrt/div/lerp-only — the `probe_sample` reference is transcendental-free
//! (proven op-by-op in the `goldens.rs` module header), and these tests pin its behaviour.
//!
//! # Scope — what I0b proves, and what it does NOT
//!
//! I0b proves, on the HOST side only: (1) the whole chain is transcendental-free, and (2) the
//! math is correct (trilinear blend / wrap weight / Chebyshev / sky fallback pin to hand
//! values). It does NOT prove the host `oct_decode` bit-matches the future I3 HLSL decode —
//! that decode is hand-written HLSL (like the marcher/SSAO oracles) verified by the GPU
//! golden at I3, and is DEFERRED to `probe_sample_gpu_eq_cpu_to_bits`. The encode side
//! diverges from the eDSL body by ≤2 ULP (`x*(1/s)` vs `x/s`; pinned by
//! `oct_encode_matches_edsl_within_2_ulp`), which is why the round-trip check below is a
//! SANITY test with a tolerance, not a bit-exact assertion.
//!
//! # The future GPU gate (I3)
//!
//! I3 will add `probe_sample_gpu_eq_cpu_to_bits`: dispatch the I3 HLSL `probe_sample`,
//! read the irradiance/depth atlas back, and diff to THIS host reference to bits (the same
//! GPU-vs-CPU oracle the SSAO / marcher / lighting mirrors carry). This file is the CPU
//! half of that contract, standing alone until the atlas + shader exist (I1 / I3).
//!
//! This file boots NO Vulkan context — it is a pure host-math regression the developer runs
//! as part of the non-GPU gate.

use boyko_rhi_vulkan::goldens::{
    ddgi_oct_encode, ddgi_texel_direction, probe_sample, DdgiProbeTap, DDGI_IRR_TILE_EDGE,
    DDGI_IRR_VALID_EXTENT, DDGI_MIN_SUM_WEIGHT, DDGI_TILE_BORDER, DDGI_WRAP_WEIGHT_BIAS,
};

// The plan's owner-locked world-fixed grid (`docs/RENDER-SDFDDGI-PLAN.md`, 2026-07-04:
// 16×8×16 = 2048 probes, spacing 2.0). Mirrors `DdgiConfig`'s defaults so the test grid IS
// the shipped grid.
const ORIGIN: [f32; 3] = [-16.0, -2.0, -16.0];
const SPACING: f32 = 2.0;
const INV_SPACING: f32 = 1.0 / SPACING;
const DIMS: [u32; 3] = [16, 8, 16];
const SKY: [f32; 3] = [0.05, 0.06, 0.08];

/// probe `i`'s world position — `origin + i · spacing` (the world-fixed grid, Decision D1).
fn probe_pos(i: [u32; 3]) -> [f32; 3] {
    [
        ORIGIN[0] + i[0] as f32 * SPACING,
        ORIGIN[1] + i[1] as f32 * SPACING,
        ORIGIN[2] + i[2] as f32 * SPACING,
    ]
}

/// A converged tap carrying a fixed irradiance and an "unshadowed" depth moment pair (a
/// large mean so `dist ≤ mean` ⇒ Chebyshev visibility 1: the probe sees the receiver).
fn open_tap(irr: [f32; 3]) -> DdgiProbeTap {
    DdgiProbeTap {
        irradiance: irr,
        depth_mean: 1.0e6,   // far beyond any grid distance ⇒ dist ≤ mean ⇒ cheb == 1
        depth_mean2: 1.0e12, // mean² (var == 0, but the dist ≤ mean arm short-circuits it)
        converged: true,
    }
}

// ---- tile-layout constants ------------------------------------------------------------

#[test]
fn tile_layout_matches_the_plan() {
    // 8×8 tile = 6×6 valid interior + a 1-texel border on every side (Decision D2).
    assert_eq!(DDGI_IRR_TILE_EDGE, 8);
    assert_eq!(DDGI_IRR_VALID_EXTENT, 6);
    assert_eq!(DDGI_TILE_BORDER, 1);
    assert_eq!(DDGI_IRR_VALID_EXTENT + 2 * DDGI_TILE_BORDER, DDGI_IRR_TILE_EDGE);
    // The plan's wrap-weight small-bias.
    assert_eq!(DDGI_WRAP_WEIGHT_BIAS, 0.2);
}

// ---- the octahedral texel → direction chain -------------------------------------------

#[test]
fn every_interior_texel_decodes_to_a_unit_direction() {
    // The whole 6×6 interior must decode to finite UNIT directions (the oct_decode chain
    // ends in v_normalize). A non-unit / non-finite result would mean the texel→UV→decode
    // chain is broken.
    for ty in 0..DDGI_IRR_VALID_EXTENT {
        for tx in 0..DDGI_IRR_VALID_EXTENT {
            let d = ddgi_texel_direction(tx, ty);
            assert!(d.iter().all(|c| c.is_finite()), "texel ({tx},{ty}) non-finite: {d:?}");
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            assert!(
                (len - 1.0).abs() < 1.0e-5,
                "texel ({tx},{ty}) direction {d:?} not unit (len {len})"
            );
        }
    }
}

#[test]
fn texel_center_direction_round_trips_through_oct_encode() {
    // SANITY CHECK (NOT a bit-exact proof). Two honesty caveats, both deferred to the I3 GPU
    // golden `probe_sample_gpu_eq_cpu_to_bits`:
    //
    //   * DECODE: `boyko_shaderdsl` authors the octahedral ENCODE eDSL body
    //     (`oct::oct_encode_body`) but NO decode body, and `probe_sample`'s chain uses DECODE.
    //     The host `goldens::oct_decode` bit-parity against the I3 HLSL decode is NOT proven
    //     here — the HLSL decode is hand-written (like the marcher/SSAO host oracles) and is
    //     verified by the GPU golden at I3, not by a host round-trip. I0b proves only
    //     transcendental-freedom + host math correctness.
    //   * ENCODE: the host `goldens::oct_encode` L1-normalizes by MULTIPLY-BY-RECIPROCAL
    //     (`inv = 1/s; n*inv`); the eDSL `oct_encode_body::<EvalCf>` (and the HLSL
    //     `float3 / float`) DIVIDE (`n / s`). `x*(1/s)` and `x/s` are NOT IEEE-bit-identical —
    //     measured ≤2 ULP over the texel sweep (see `oct_encode_matches_edsl_within_2_ulp`).
    //     So this is NOT the eDSL body's byte-image; the 1e-4 tolerance below absorbs both the
    //     encode ≤2-ULP gap and the decode `v_normalize` round-off.
    //
    // What it DOES prove: `ddgi_texel_direction`'s texel→UV→decode chain is self-consistent —
    // re-encoding a texel-center direction lands back inside that texel's [0,1]² footprint.
    for ty in 0..DDGI_IRR_VALID_EXTENT {
        for tx in 0..DDGI_IRR_VALID_EXTENT {
            let d = ddgi_texel_direction(tx, ty);
            let e = ddgi_oct_encode(d);
            let extent = DDGI_IRR_VALID_EXTENT as f32;
            let expect_u = (tx as f32 + 0.5) / extent;
            let expect_v = (ty as f32 + 0.5) / extent;
            assert!(
                (e[0] - expect_u).abs() < 1.0e-4 && (e[1] - expect_v).abs() < 1.0e-4,
                "texel ({tx},{ty}) round-trip: encode {e:?} != UV ({expect_u},{expect_v})"
            );
        }
    }
}

#[test]
fn oct_encode_matches_edsl_within_2_ulp() {
    // The DIRECT encode reference check the review asked for: `goldens::oct_encode` vs the eDSL
    // `oct_encode_body::<EvalCf>` (the REAL reference — the same body the committed HLSL
    // `oct_encode` is spliced from, byte-identity-gated by `gbuffer_mrt_edsl_sync` /
    // `sdf_field_edsl_sync`). NOT bit-identical: `goldens::oct_encode` multiplies by the
    // reciprocal (`1/s` then `n*inv`) while `EvalCf::vec3_div_scalar` DIVIDES (`n/s`) — the
    // `x*(1/s)` vs `x/s` gap. We do NOT change `goldens::oct_encode` (it backs the existing
    // G-buffer NORMAL goldens `golden_marcher_attributes` — altering its op sequence would
    // break their byte-identity), so we PIN the divergence at ≤2 ULP instead of asserting bits.
    // The HLSL-decode-side parity is proven later by the I3 GPU golden, not here.
    let extent = DDGI_IRR_VALID_EXTENT as f32;
    let mut max_ulp = 0i64;
    for ty in 0..DDGI_IRR_VALID_EXTENT {
        for tx in 0..DDGI_IRR_VALID_EXTENT {
            let d = ddgi_texel_direction(tx, ty);
            let host = ddgi_oct_encode(d); // goldens::oct_encode (mul-by-reciprocal)
            let edsl = edsl_oct_encode(d); // oct_encode_body::<EvalCf> (divide)
            for k in 0..2 {
                let ulp = (host[k].to_bits() as i64 - edsl[k].to_bits() as i64).abs();
                max_ulp = max_ulp.max(ulp);
            }
            let _ = extent;
        }
    }
    assert!(
        max_ulp <= 2,
        "goldens::oct_encode diverges from the eDSL body by {max_ulp} ULP (> 2 expected): the \
         x*(1/s) vs x/s gap widened — re-audit before relaxing the bound"
    );
}

/// The eDSL `oct_encode_body::<EvalCf>` reading the `float2` its ret-cell holds — the REAL
/// reference (the body the committed HLSL `oct_encode` is spliced from). Mirrors
/// `boyko_shaderdsl/tests/eval_byte_identity.rs::refactored_oct_encode`.
fn edsl_oct_encode(n: [f32; 3]) -> [f32; 2] {
    use std::cell::Cell;
    let ret_out = Cell::new([0.0f32; 2]);
    let _ = boyko_shaderdsl::oct::oct_encode_body::<boyko_shaderdsl::cf::EvalCf>(n, &ret_out);
    ret_out.get()
}

// ---- probe_sample known-value / property cases ----------------------------------------

#[test]
fn receiver_at_probe_center_with_uniform_irradiance_returns_that_irradiance() {
    // A receiver EXACTLY at a probe center, all 8 corners converged with the SAME irradiance
    // C, and every probe unshadowed ⇒ the weighted mean is C regardless of the (positive)
    // weights — the normalize divides the summed weight out. The canonical DDGI sanity case.
    let target = probe_pos([8, 4, 8]); // an interior probe center
    let c = [0.4_f32, 0.5, 0.6];
    // Normal points "up" — every surrounding probe gets some facing weight, but uniform C
    // means the exact weights are irrelevant to the result.
    let n = [0.0, 1.0, 0.0];
    let out = probe_sample(
        target,
        n,
        ORIGIN,
        INV_SPACING,
        DIMS,
        SKY,
        probe_pos,
        |_i, _dir| open_tap(c),
    );
    for k in 0..3 {
        assert!(
            (out[k] - c[k]).abs() < 1.0e-5,
            "uniform-irradiance mean lane {k}: {out:?} != {c:?}"
        );
    }
}

#[test]
fn trilinear_corner_weights_sum_to_one() {
    // With every corner converged + unshadowed + a NEUTRALIZED wrap weight (normal zero ⇒
    // dot 0 ⇒ wrap == bias, a constant across all 8 corners) and per-corner DISTINCT
    // irradiance equal to the corner's trilinear weight, the normalized result equals the
    // sum of (weight · weight) / sum(weight). We instead assert the simpler invariant the
    // uniform case proves the trilinear normalization holds: sample a mid-cell receiver with
    // uniform C and confirm it equals C (weights summing to 1 after normalize).
    let base = probe_pos([5, 3, 7]);
    // A receiver at the cell center (frac 0.5 on every axis) — all 8 trilinear weights 0.125.
    let mid = [
        base[0] + 0.5 * SPACING,
        base[1] + 0.5 * SPACING,
        base[2] + 0.5 * SPACING,
    ];
    let c = [0.3_f32, 0.3, 0.3];
    // Zero normal ⇒ wrap weight is the constant bias on every corner, so the ONLY variation
    // is trilinear; uniform C ⇒ result C proves the trilinear weights normalize to 1.
    let out = probe_sample(mid, [0.0, 0.0, 0.0], ORIGIN, INV_SPACING, DIMS, SKY, probe_pos, |_i, _dir| {
        open_tap(c)
    });
    for k in 0..3 {
        assert!(
            (out[k] - c[k]).abs() < 1.0e-5,
            "trilinear normalize lane {k}: {out:?} != {c:?}"
        );
    }
}

#[test]
fn trilinear_weights_blend_eight_distinct_corners_by_hand() {
    // The NON-VACUOUS trilinear test: 8 DISTINCT corner irradiances at a NON-center frac
    // (0.25, 0.5, 0.75) so no common factor cancels. Zero normal ⇒ the wrap weight is the
    // constant `bias` on every corner (a shared factor that DOES cancel), and every corner is
    // unshadowed (cheb == 1), so the ONLY surviving discriminator is the trilinear weight
    // `wx·wy·wz`. The oracle result must equal the HAND-COMPUTED `Σ(wx·wy·wz·Ci)/Σ(wx·wy·wz)`
    // — this is the test that actually pins the per-corner trilinear arithmetic the I3 GPU
    // golden will diff against (the two "normalization" tests above reduce to uniform-C-in →
    // C-out, where the weights cancel and pin nothing).
    let base_idx = [5u32, 3, 7]; // an interior base cell (base+1 stays in-bounds on every axis)
    let base = probe_pos(base_idx);
    // Non-center fractions so wx≠1-wx etc. — every corner carries a distinct weight.
    let (fx, fy, fz) = (0.25_f32, 0.5, 0.75);
    let receiver = [
        base[0] + fx * SPACING,
        base[1] + fy * SPACING,
        base[2] + fz * SPACING,
    ];

    // 8 distinct corner irradiances, keyed by the corner's local (cx,cy,cz) so the injected
    // `tap` can look each corner up by its grid index.
    let corner_irr = |cx: u32, cy: u32, cz: u32| -> [f32; 3] {
        let seed = (cx + 2 * cy + 4 * cz) as f32; // 0..7, all distinct
        [1.0 + seed, 10.0 + seed, 100.0 + seed]
    };

    let out = probe_sample(
        receiver,
        [0.0, 0.0, 0.0], // zero normal ⇒ wrap == bias on every corner (cancels)
        ORIGIN,
        INV_SPACING,
        DIMS,
        SKY,
        probe_pos,
        |i, _dir| {
            // Recover the corner's local offset from its grid index (base_idx + {0,1}³).
            let cx = i[0] - base_idx[0];
            let cy = i[1] - base_idx[1];
            let cz = i[2] - base_idx[2];
            open_tap(corner_irr(cx, cy, cz))
        },
    );

    // Hand-compute Σ(w·Ci)/Σw in the SAME corner order the oracle accumulates (z outer, y, x
    // inner). Since every wrap==bias and cheb==1, they cancel between numerator and denominator,
    // leaving the pure trilinear blend.
    let mut num = [0.0_f32; 3];
    let mut den = 0.0_f32;
    for cz in 0..2u32 {
        let wz = if cz == 0 { 1.0 - fz } else { fz };
        for cy in 0..2u32 {
            let wy = if cy == 0 { 1.0 - fy } else { fy };
            for cx in 0..2u32 {
                let wx = if cx == 0 { 1.0 - fx } else { fx };
                let w = wx * wy * wz;
                let ci = corner_irr(cx, cy, cz);
                num[0] += w * ci[0];
                num[1] += w * ci[1];
                num[2] += w * ci[2];
                den += w;
            }
        }
    }
    let expected = [num[0] / den, num[1] / den, num[2] / den];

    for k in 0..3 {
        assert!(
            (out[k] - expected[k]).abs() < 1.0e-5,
            "trilinear blend lane {k}: oracle {out:?} != hand {expected:?}"
        );
    }
}

#[test]
fn receiver_at_probe_center_has_neutral_wrap_bias() {
    // G2 (nice-to-have): a receiver EXACTLY at a probe center has `to_probe = normalize(0)`
    // (v_normalize's zero-guard ⇒ [0,0,0]) ⇒ `dot(to_probe, n) == 0` ⇒ wrap == `((0+1)·0.5)² +
    // bias == 0.25 + bias`, the NEUTRAL wrap. All 8 trilinear corners collapse onto that single
    // probe (frac 0), so with a single converged irradiance C the result is exactly C — the
    // wrap factor cancels in the normalize. Documents the on-center degeneracy (the case the
    // Chebyshev test deliberately avoids).
    let target = probe_pos([9, 5, 9]);
    let c = [0.7_f32, 0.2, 0.9];
    let out = probe_sample(target, [0.0, 1.0, 0.0], ORIGIN, INV_SPACING, DIMS, SKY, probe_pos, |_i, _dir| {
        open_tap(c)
    });
    for k in 0..3 {
        assert!(
            (out[k] - c[k]).abs() < 1.0e-5,
            "on-center neutral-wrap lane {k}: {out:?} != {c:?}"
        );
    }
}

#[test]
fn all_unconverged_probes_fall_back_to_sky() {
    // Every surrounding probe is unconverged (converged == false) ⇒ zero summed weight ⇒ the
    // receiver resolves to the sky-ambient fallback (the first-frames / outside-coverage arm,
    // Decision D2 storage-class 3: the converged-once bit gates unconverged probes to sky).
    let p = probe_pos([2, 2, 2]);
    let out = probe_sample(p, [0.0, 1.0, 0.0], ORIGIN, INV_SPACING, DIMS, SKY, probe_pos, |_i, _dir| {
        DdgiProbeTap { irradiance: [9.0, 9.0, 9.0], depth_mean: 1.0e6, depth_mean2: 1.0e12, converged: false }
    });
    assert_eq!(out, SKY, "unconverged coverage must fall back to sky ambient");
}

#[test]
fn receiver_far_outside_the_aabb_falls_back_when_boundary_probes_unconverged() {
    // A receiver far outside the box clamps onto the boundary cell; if those boundary probes
    // are unconverged, the summed weight stays below eps ⇒ sky. (A converged boundary probe
    // would legitimately extrapolate — the clamp is a benign edge behaviour, not a fallback.)
    let far = [ORIGIN[0] - 1000.0, ORIGIN[1] - 1000.0, ORIGIN[2] - 1000.0];
    let out = probe_sample(far, [0.0, 1.0, 0.0], ORIGIN, INV_SPACING, DIMS, SKY, probe_pos, |_i, _dir| {
        DdgiProbeTap { irradiance: [1.0, 1.0, 1.0], depth_mean: 1.0e6, depth_mean2: 1.0e12, converged: false }
    });
    assert_eq!(out, SKY);
}

#[test]
fn wrap_weight_zeroes_a_fully_back_facing_probe() {
    // Two probes flank the receiver along Y: the one BELOW is fully back-facing to an up
    // normal (dot(dirToProbe, n) == -1 ⇒ wrap == ((-1+1)*0.5)² + bias == bias, the FLOOR),
    // the one ABOVE is fully front-facing (dot == +1 ⇒ wrap == 1 + bias, the MAX). With
    // DISTINCT irradiance per probe, the result must be pulled strongly toward the
    // front-facing probe's value — proving the wrap weight suppresses the back-facing probe.
    //
    // Put the receiver ON a probe row so only the two Y-adjacent probes carry non-zero
    // trilinear weight (frac 0 on X/Z ⇒ the +1 X/Z corners get weight 0, and their
    // duplicate-index min-clamp still lands on the same in-plane probes — but all share the
    // same Y split, so the Y wrap asymmetry is the ONLY discriminator).
    let ix = 8u32;
    let iz = 8u32;
    let iy = 4u32;
    // Receiver just above probe (ix, iy, iz), so the base cell is iy and iy+1 straddles it
    // with a small `ty`; the up-normal favours the iy+1 (above) probe.
    let below = probe_pos([ix, iy, iz]);
    let receiver = [below[0], below[1] + 0.25 * SPACING, below[2]];
    let up = [0.0, 1.0, 0.0];

    let front_irr = [10.0_f32, 0.0, 0.0]; // the ABOVE (front-facing) probe: bright red
    let back_irr = [0.0_f32, 0.0, 10.0]; // the BELOW (back-facing) probe: bright blue
    let out = probe_sample(receiver, up, ORIGIN, INV_SPACING, DIMS, SKY, probe_pos, |i, _dir| {
        // The row ABOVE (iy+1) is front-facing; the row AT/below (iy) is back-facing.
        let irr = if i[1] > iy { front_irr } else { back_irr };
        open_tap(irr)
    });
    // The front-facing (red) contribution must dominate the back-facing (blue): the wrap
    // weight floor (bias) on the back probe + the up-normal facing on the front probe pull
    // the mean toward red.
    assert!(
        out[0] > out[2],
        "wrap weight must favour the front-facing probe: {out:?} (red should beat blue)"
    );
    assert!(out[0] > 1.0, "front-facing red must carry real weight: {out:?}");
}

#[test]
fn chebyshev_shadows_a_probe_behind_geometry() {
    // A probe whose depth moment says geometry is NEAR (mean small) while the receiver is FAR
    // (dist ≫ mean) ⇒ the receiver is behind an occluder from the probe's view ⇒ Chebyshev
    // visibility drops below 1, suppressing that probe's leak.
    //
    // Chebyshev (like trilinear) is a per-corner MULTIPLIER, so a UNIFORM-irradiance grid
    // would cancel it in the normalize (`Σ w·cheb·C / Σ w·cheb == C`) — the on-center /
    // uniform setups prove NOTHING about cheb. To make suppression OBSERVABLE we split the
    // cell into two rows with DISTINCT irradiances and shadow ONE row: the result must move
    // AWAY from the shadowed row's colour (its weight is throttled) toward the open row's.
    //
    // The receiver is placed BETWEEN probes (the cell-diagonal midpoint of `probe_pos([6,3,6])`
    // and `probe_pos([7,4,7])`), NOT on a probe center: with spacing 2.0 the distance to each
    // surrounding probe is ≈ 1–1.7, so for the shadowed tap `Δ = dist - mean ≫ 0` ⇒ a strong
    // `cheb ≈ var/Δ² ≪ 1`. A receiver AT a probe center would give `dist = 0 ≤ mean` on the
    // collapsed corner ⇒ the `cheb = 1` arm (the degenerate case that is NOT under test).
    let lo = probe_pos([6, 3, 6]);
    let hi = probe_pos([7, 4, 7]);
    let p = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ];
    let up = [0.0, 1.0, 0.0];

    // The ABOVE row (iy+1 == 4) is bright RED and always OPEN; the BELOW row (iy == 3) is
    // bright BLUE. `to_row` maps a probe's grid Y to its row colour.
    let red = [10.0_f32, 0.0, 0.0];
    let blue = [0.0_f32, 0.0, 10.0];
    let row_irr = |gy: u32| if gy >= 4 { red } else { blue };

    // Baseline: BOTH rows open ⇒ the blend carries both red and blue.
    let both_open = probe_sample(p, up, ORIGIN, INV_SPACING, DIMS, SKY, probe_pos, |i, _dir| {
        open_tap(row_irr(i[1]))
    });

    // Test: the BELOW (blue) row is SHADOWED — a small mean (geometry right at those probes)
    // with a tiny variance so the receiver's `Δ ≫ mean` drives their `cheb → 0`, throttling
    // blue's weight. The red row stays open.
    let blue_shadowed = probe_sample(p, up, ORIGIN, INV_SPACING, DIMS, SKY, probe_pos, |i, _dir| {
        if i[1] >= 4 {
            open_tap(red) // the above (red) row: unshadowed
        } else {
            DdgiProbeTap {
                irradiance: blue,
                depth_mean: 0.01,
                depth_mean2: 0.01 * 0.01 + 1.0e-6, // var ≈ 1e-6 ⇒ cheb ≈ var/Δ² ≪ 1
                converged: true,
            }
        }
    });

    // Shadowing blue must suppress the blue channel (its weight is throttled) AND raise the
    // red fraction — the result moves away from blue, toward red.
    assert!(
        blue_shadowed[2] < both_open[2],
        "Chebyshev must suppress the shadowed (blue) row: shadowed b {} !< open b {}",
        blue_shadowed[2],
        both_open[2]
    );
    assert!(
        blue_shadowed[0] > both_open[0],
        "suppressing blue must raise the red fraction: shadowed r {} !> open r {}",
        blue_shadowed[0],
        both_open[0]
    );
}

#[test]
fn output_is_finite_and_the_epsilon_guard_is_real() {
    // No path may produce a NaN/Inf (the div-by-zero guards + v_normalize zero-guard). Sweep a
    // few receivers, including one on an exact probe center (frac 0 ⇒ duplicate corner indices,
    // the min-clamp collapse) and one at the far corner.
    let eps = DDGI_MIN_SUM_WEIGHT;
    assert!(eps > 0.0 && eps < 1.0e-3, "the summed-weight guard must be a small positive eps");

    let probes = [
        probe_pos([0, 0, 0]),
        probe_pos([15, 7, 15]),
        probe_pos([8, 4, 8]),
    ];
    for &p in &probes {
        let out = probe_sample(p, [0.0, 1.0, 0.0], ORIGIN, INV_SPACING, DIMS, SKY, probe_pos, |_i, _dir| {
            open_tap([0.2, 0.2, 0.2])
        });
        assert!(out.iter().all(|c| c.is_finite()), "non-finite probe_sample at {p:?}: {out:?}");
    }
}
