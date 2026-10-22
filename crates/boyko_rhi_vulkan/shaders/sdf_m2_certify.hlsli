// SDF brick-atlas M2 — the FIRST-CROSSING CERTIFICATE (OPEN-ITEMS 16, `oi1516/cut.md` D16-1..D16-5).
//
// The M2 SURFACE-brick step commits the hit its trilinear cubic + signed analytic refine found. The
// reconstruction is not a lower bound on the field (a sub-`R_MIN` subtractor, a thin shell, a small
// primitive in front of a larger one), so the refine can converge on a surface BEHIND the first one:
// the crater's far wall at (39,22) once the refine budget reaches 19, or a small sphere in front of a
// big one at any budget. Before a hit is committed, this header proves the EXACT field has no zero
// crossing on the ray segment the march has not certified itself: from `a` (the last point the march
// certified — `safe_t + sor_prev` past an over-relaxed step, else `t`) to the cubic candidate when the
// refine walked FORWARD (its own under-relaxed sphere trace proved `[cand, hit)`), else to
// `hit - M2_CERT_TAIL` (the hit may sit up to `EPS` inside). A refused hit returns "no hit", and the
// caller latches M2 off for the rest of the ray: the analytic march — the definition of the first
// surface — finishes it.
//
// Why it is sound: every primitive kind is the exact signed distance of a CONVEX set, so along a ray
// it is convex and lies above its tangents at the segment ends (`convex_seg_lo`; exact when the
// target is approached from outside); the CSG fold is monotone in each operand after the subtrahend's
// negation (polynomial smin's partials are `h` and `1 - h`), so it carries `[lo, hi]` intervals
// (subtract pairs `lo` with the subtrahend's `hi`). A NEW primitive kind must add its convex rule to
// `boyko_shaderdsl::range` first (the `field_range_lo_bounds_the_field` census fails on a kind count).
//
// The per-edit value + slope and the segment bound are GENERATED from `boyko_shaderdsl::range` (the
// same bodies whose `f32` Eval instance is the host certificate, `goldens::host_field_range`); the
// edit loop, the kind / op dispatch and the subdivision driver stay hand-written here, mirrored
// statement-for-statement by the host (`goldens::host_m2_first_crossing_certified`).
//
// INCLUDE CONTRACT: after `sdf_field.hlsli` (`Edit`, `load_edit`, `combine`, `FAR`, `MAX_SDF_EDITS`,
// `KIND_*`, `OP_*`, `CAPSULE_DENOM_EPS`, `Buf`) and after the includer's `EPS` (the march's accept
// band). `M2_CERTIFY` (default 1): `-D M2_CERTIFY=0` compiles the certificate to `return true` — ONLY
// the rule-(xviii) register twin of `tests/marcher_occupancy.rs` sets it; no shipped artifact does.
//
// References: Hart 1996 (sphere tracing), Keinert et al. 2014 (enhanced sphere tracing — the start
// point), Galin et al. 2020 (segment tracing), Knoll et al. 2009 (interval certify-or-subdivide).

#ifndef M2_CERTIFY
#define M2_CERTIFY 1
#endif

// The range-evaluation cap: a COST cap, not a correctness parameter (a refusal hands the ray to the
// analytic march). Mirrors the host `compute::M2_CERT_EVALS`.
static const uint  M2_CERT_EVALS     = 8u;
// A segment is crossing-free when its range lower bound exceeds this (above the bound's f32
// evaluation error over the clip-map extent, 7.6e-5). Mirrors the host `compute::M2_CERT_EPS`.
static const float M2_CERT_EPS       = 1.0e-4;
// The certificate's resolution behind a backward-walk / immediate-accept hit (ruling 93 Q6).
static const float M2_CERT_TAIL      = 4.0 * EPS;
// The shortest piece subdivision goes to before it refuses.
static const float M2_CERT_MIN_PIECE = 0.5 * M2_CERT_TAIL;

// The generated leaves: `m2_cert_sphere` / `m2_cert_box` / `m2_cert_capsule` (one edit's
// `float2(value, slope)` along the unit `rd`) and `convex_seg_lo` (the convex segment lower bound).
// Do NOT hand-edit between the sentinels: re-run
//   cargo run -p boyko_shaderdsl --features emit --bin emit_field
// and re-splice. `sdf_field_edsl_sync` pins this span to the generator.
// === GENERATED m2_cert_leaves BEGIN ===
float2 m2_cert_sphere(float3 p, float3 rd, float3 c, float r) {
    float t0 = p.x - c.x;
    float t1 = p.y - c.y;
    float t2 = p.z - c.z;
    float t3 = t0 * t0;
    float t4 = t1 * t1;
    float t5 = t3 + t4;
    float t6 = t2 * t2;
    float t7 = t5 + t6;
    float t8 = sqrt(t7);
    float t9 = t8 - r;
    float t10 = t0 * rd.x;
    float t11 = t1 * rd.y;
    float t12 = t10 + t11;
    float t13 = t2 * rd.z;
    float t14 = t12 + t13;
    float t15 = max(t8, 1.0e-30);
    float t16 = t14 / t15;
    return float2(t9, t16);
}

float2 m2_cert_box(float3 p, float3 rd, float3 c, float3 h) {
    float t0 = p.x - c.x;
    float t1 = p.y - c.y;
    float t2 = p.z - c.z;
    float t3 = abs(t0);
    float t4 = abs(t1);
    float t5 = abs(t2);
    float t6 = t3 - h.x;
    float t7 = t4 - h.y;
    float t8 = t5 - h.z;
    float t9 = max(t6, 0.0);
    float t10 = max(t7, 0.0);
    float t11 = max(t8, 0.0);
    float t12 = t9 * t9;
    float t13 = t10 * t10;
    float t14 = t12 + t13;
    float t15 = t11 * t11;
    float t16 = t14 + t15;
    float t17 = sqrt(t16);
    float t18 = max(t7, t8);
    float t19 = max(t6, t18);
    float t20 = min(t19, 0.0);
    float t21 = t17 + t20;
    float t22 = (t0 >= 0.0) ? 1.0 : -1.0;
    float t23 = (t1 >= 0.0) ? 1.0 : -1.0;
    float t24 = (t2 >= 0.0) ? 1.0 : -1.0;
    float t25 = t22 * t9;
    float t26 = t25 * rd.x;
    float t27 = t23 * t10;
    float t28 = t27 * rd.y;
    float t29 = t26 + t28;
    float t30 = t24 * t11;
    float t31 = t30 * rd.z;
    float t32 = t29 + t31;
    float t33 = max(t17, 1.0e-30);
    float t34 = t32 / t33;
    float t35 = t22 * rd.x;
    float t36 = t23 * rd.y;
    float t37 = t24 * rd.z;
    float t38 = max(t7, t8);
    float t39 = (t7 >= t8) ? t36 : t37;
    float t40 = (t6 >= t38) ? t35 : t39;
    float t41 = (t17 > 0.0) ? t34 : t40;
    return float2(t21, t41);
}

float2 m2_cert_capsule(float3 p, float3 rd, float3 a, float3 b, float r) {
    float t0 = p.x - a.x;
    float t1 = p.y - a.y;
    float t2 = p.z - a.z;
    float t3 = b.x - a.x;
    float t4 = b.y - a.y;
    float t5 = b.z - a.z;
    float t6 = t3 * t3;
    float t7 = t4 * t4;
    float t8 = t6 + t7;
    float t9 = t5 * t5;
    float t10 = t8 + t9;
    float t11 = max(t10, 1.0e-8);
    float t12 = t0 * t3;
    float t13 = t1 * t4;
    float t14 = t12 + t13;
    float t15 = t2 * t5;
    float t16 = t14 + t15;
    float t17 = t16 / t11;
    float t18 = clamp(t17, 0.0, 1.0);
    float t19 = t3 * t18;
    float t20 = t4 * t18;
    float t21 = t5 * t18;
    float t22 = t0 - t19;
    float t23 = t1 - t20;
    float t24 = t2 - t21;
    float t25 = t22 * t22;
    float t26 = t23 * t23;
    float t27 = t25 + t26;
    float t28 = t24 * t24;
    float t29 = t27 + t28;
    float t30 = sqrt(t29);
    float t31 = t30 - r;
    float t32 = t22 * rd.x;
    float t33 = t23 * rd.y;
    float t34 = t32 + t33;
    float t35 = t24 * rd.z;
    float t36 = t34 + t35;
    float t37 = max(t30, 1.0e-30);
    float t38 = t36 / t37;
    return float2(t31, t38);
}

float convex_seg_lo(float va, float vb, float sa, float sb, float len) {
    float t0 = sa - sb;
    float t1 = min(t0, -1.0e-30);
    float t2 = vb - va;
    float t3 = sb * len;
    float t4 = t2 - t3;
    float t5 = t4 / t1;
    float t6 = max(t5, 0.0);
    float t7 = min(t6, len);
    float t8 = sa * t7;
    float t9 = va + t8;
    float t10 = (sb <= 0.0) ? vb : t9;
    float t11 = (sa >= 0.0) ? va : t10;
    float t12 = min(t11, va);
    float t13 = min(t12, vb);
    return t13;
}
// === GENERATED m2_cert_leaves END ===

// One edit's `float2(value, slope)` at `p` along `rd` — the field's kind dispatch (BOX, then CAPSULE,
// else sphere), the host `goldens::host_edit_value_slope`.
float2 edit_value_slope(Edit e, float3 p, float3 rd) {
    if (e.kind == KIND_BOX) {
        return m2_cert_box(p, rd, e.center, e.params);
    } else if (e.kind == KIND_CAPSULE) {
        return m2_cert_capsule(p, rd, e.center, e.params, e.radius);
    }
    return m2_cert_sphere(p, rd, e.center, e.params.x);
}

// A lower bound of the EXACT edit-list field on the ray segment `ro + rd * t`, `t in [ta, tb]`: each
// edit's convex bound and end-value max, folded as intervals (the host `goldens::host_field_range`).
float field_range_lo(float3 ro, float3 rd, float ta, float tb) {
    uint n = min(Buf[0], MAX_SDF_EDITS);
    if (n == 0u) {
        return FAR;
    }
    float3 pa = ro + rd * ta;
    float3 pb = ro + rd * tb;
    float len = tb - ta;
    float lo = FAR;
    float hi = FAR;
    [loop]
    for (uint i = 0u; i < n; ++i) {
        Edit e = load_edit(i);
        float2 va = edit_value_slope(e, pa, rd);
        float2 vb = edit_value_slope(e, pb, rd);
        float lo_d = convex_seg_lo(va.x, vb.x, va.y, vb.y, len);
        float hi_d = max(va.x, vb.x);
        if (i == 0u) {
            lo = lo_d;
            hi = hi_d;
        } else if (e.op == OP_SUBTRACT) {
            float lo_next = combine(lo, hi_d, e.op, e.smoothness);
            hi = combine(hi, lo_d, e.op, e.smoothness);
            lo = lo_next;
        } else {
            lo = combine(lo, lo_d, e.op, e.smoothness);
            hi = combine(hi, hi_d, e.op, e.smoothness);
        }
    }
    return lo;
}

// `true` iff the exact field is proven above `M2_CERT_EPS` on `[a, goal]` — no crossing there.
// Certify-or-subdivide: the whole remaining segment first; a certified piece doubles the next, a
// refused one halves; a piece below `M2_CERT_MIN_PIECE` or `M2_CERT_EVALS` evaluations refuses.
// `goal <= a` is certified with no evaluation. The host `goldens::host_m2_first_crossing_certified`.
bool m2_first_crossing_certified(float3 ro, float3 rd, float a, float goal) {
#if M2_CERTIFY
    float lo_t = a;
    float piece = goal - a;
    [loop]
    for (uint k = 0u; k < M2_CERT_EVALS; ++k) {
        if (lo_t >= goal) {
            return true;
        }
        float hi_t = min(lo_t + piece, goal);
        if (field_range_lo(ro, rd, lo_t, hi_t) > M2_CERT_EPS) {
            lo_t = hi_t;
            piece = piece * 2.0;
        } else {
            piece = piece * 0.5;
            if (piece < M2_CERT_MIN_PIECE) {
                return false;
            }
        }
    }
    return lo_t >= goal;
#else
    return true;
#endif
}

// The certificate's segment end for a refined M2 hit (the host `goldens::m2_cert_goal`): the
// candidate for a forward walk, else `hit - M2_CERT_TAIL`.
float m2_cert_goal(float hit_t, float cand_t) {
    return (hit_t > cand_t) ? cand_t : hit_t - M2_CERT_TAIL;
}
