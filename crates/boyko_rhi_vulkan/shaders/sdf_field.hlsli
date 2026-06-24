// Render P4a: the SHARED SDF field-eval gateway (`sdf_field.hlsli`).
//
// This header is the SINGLE field gateway for the whole engine (the P4 invariant).
// It is a VERBATIM cut of the determinism-frozen field-eval region of the rung-10 /
// P1b marcher (`sdf_gbuffer_composite.hlsl` / `sdf_depth_composite.hlsl`): the
// field-layout consts, the kind/op enums, the `Edit` struct + `load_edit`, the
// primitive distances (`sd_sphere`/`sd_box`/`edit_distance`), the boolean ops +
// polynomial smooth-min/-max (`smin`/`smax`/`combine`), the edit-list field `sdf`,
// and the central-difference gradient `sdf_normal`. Nothing here was reordered or
// edited — the source is character-identical to the region it was cut from so DXC
// emits byte-identical field-eval SPIR-V ops and the host golden stays byte-exact.
//
// # INCLUDE CONTRACT (precondition)
//
// `StructuredBuffer<uint> Buf : register(t0)` MUST be declared and in scope BEFORE
// this header is `#include`d — the field eval reads the packed edit-list header
// out of `Buf` (`Buf[0]` = edit_count, then `MAX_SDF_EDITS` packed edits at
// `HEADER_BASE`). The including TU owns the binding; this header references it.
//
// # DETERMINISM CONTRACT (INVIOLABLE — the P4 invariant)
//
// The scalar field eval below is byte-shared across the entire engine:
//   * the GPU marcher (`sdf_gbuffer_composite.hlsl` / `sdf_depth_composite.hlsl`),
//   * the host golden mirror `golden_composite_pixel_ex` (compute.rs),
//   * the CPU physics evaluator `boyko_sdf_math::sdf_edit_list`,
//   * every future FIELD-CONSUMER (the P4b coarse cone-trace cull, B1 over-relaxation,
//     A1 cone-trace shadows, A2 ambient occlusion, P9 brick backend).
// Therefore: NO fast-math, NO reordered/contracted FMA, NO `rsqrt`/`rcp`, NO FP16.
// Plain IEEE ops only. Any divergence here breaks the golden-image gate AND the
// render<->physics geometric agreement.
//
// # The stable gateway
//
// `field_distance(p)` is the named gateway every consumer calls; today it is a
// plain alias of `sdf(p)`. When P9/P10/P12 swap the field BACKEND (brick fetch),
// ONLY `field_distance` (and the future `tile_bound`) change here — the consumers
// stay byte-untouched. The analytic `sdf`/`smin`/`smax`/`combine`/normal remain the
// FROZEN reference and the physics source of truth (physics never reads bricks).

// --- Field-eval tuning constants (frozen; parameterize the field functions) -----
static const float FAR    = 1.0e9;  // the "empty field" sentinel before the first edit
static const float GRAD_H = 0.0005; // central-difference half-step for the normal

// --- The edit-list packed-header contract (mirrored host-side) -----------------
// IDENTICAL to rung 9/10 up to the edit array. Unlike rung 10 there is NO depth
// region and NO pixel region in the buffer (depth is the sampled image, color is the
// storage image), so only the count + edit array are read here.
static const uint MAX_SDF_EDITS  = 16u;
static const uint SDF_EDIT_WORDS = 12u;       // size_of::<SdfEdit>() / 4
static const uint HEADER_BASE    = 4u;        // edit array word offset (count padded to 16 B)

// Primitive kinds.
static const uint KIND_SPHERE = 0u;
static const uint KIND_BOX    = 1u;

// Boolean ops.
static const uint OP_UNION     = 0u;
static const uint OP_SUBTRACT  = 1u;
static const uint OP_INTERSECT = 2u;

// One decoded edit (the in-register form of the packed std430 element).
struct Edit {
    float3 center;
    float3 params;     // radius (sphere) or half-extents (box)
    uint   kind;
    uint   op;
    float  smoothness;
};

// Reads `asfloat`/`asuint` of the i-th packed edit out of the header region.
Edit load_edit(uint i) {
    uint base = HEADER_BASE + i * SDF_EDIT_WORDS;
    Edit e;
    e.center     = float3(asfloat(Buf[base + 0u]), asfloat(Buf[base + 1u]), asfloat(Buf[base + 2u]));
    // word base+3 = center.w (unused)
    e.params     = float3(asfloat(Buf[base + 4u]), asfloat(Buf[base + 5u]), asfloat(Buf[base + 6u]));
    // word base+7 = params.w (unused)
    e.kind       = Buf[base + 8u];
    e.op         = Buf[base + 9u];
    e.smoothness = asfloat(Buf[base + 10u]);
    // word base+11 = _pad (unused)
    return e;
}

// --- Primitive distance functions (IQ; the frozen rung-9 primitive set) -------

// Sphere: distance to a sphere centered at `c` with radius `r`.
float sd_sphere(float3 p, float3 c, float r) {
    return length(p - c) - r;
}

// Box: distance to an axis-aligned box centered at `c` with half-extents `h`
// (the standard IQ exact box SDF).
float sd_box(float3 p, float3 c, float3 h) {
    float3 q = abs(p - c) - h;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}

// One edit's primitive distance at `p`.
float edit_distance(Edit e, float3 p) {
    if (e.kind == KIND_BOX) {
        return sd_box(p, e.center, e.params);
    }
    return sd_sphere(p, e.center, e.params.x);
}

// --- Boolean ops + polynomial smooth-min/-max (IQ) ----------------------------

// === GENERATED FIELD MATH BEGIN (boyko_shaderdsl::emit) ===
// The smooth-min/-max polynomial bodies below are MACHINE-GENERATED by tracing the
// generic `boyko_shaderdsl::field` body over the `Emit` backend (SSA temps; shared
// subtrees emitted once) — the SAME single source whose `Eval` backend (`impl
// FieldScalar for f32`) IS `boyko_sdf_math`'s host field (`sdf_edit_list`). This is
// the eDSL that kills the by-hand HLSL<->Rust field duplication (the source of ~5
// silent drift bugs). Regenerate with:
//     cargo run -p boyko_shaderdsl --features emit --bin emit_field
// Do NOT hand-edit these bodies — edit `boyko_shaderdsl::field` and re-emit. No
// `precise` qualifier is emitted, so the generated SSA compiles to SPIR-V
// BYTE-IDENTICAL to the prior hand-frozen bodies; the `field_probe_gate` GATE-1
// disassembly tripwire (vs `sdf_field_probe.baseline.dis`) is the empirical proof.

// Polynomial smooth-min (IQ `smin`): a soft union with blend radius `k`.
float smin(float a, float b, float k) {
    float t0 = b - a;
    float t1 = 0.5 * t0;
    float t2 = t1 / k;
    float t3 = 0.5 + t2;
    float t4 = clamp(t3, 0.0, 1.0);
    float t5 = lerp(b, a, t4);
    float t6 = k * t4;
    float t7 = 1.0 - t4;
    float t8 = t6 * t7;
    float t9 = t5 - t8;
    return t9;
}

// Polynomial smooth-max: the De Morgan dual of `smin` (inlined; was `-smin(-a,-b,k)`).
float smax(float a, float b, float k) {
    float t0 = -a;
    float t1 = -b;
    float t2 = t1 - t0;
    float t3 = 0.5 * t2;
    float t4 = t3 / k;
    float t5 = 0.5 + t4;
    float t6 = clamp(t5, 0.0, 1.0);
    float t7 = lerp(t1, t0, t6);
    float t8 = k * t6;
    float t9 = 1.0 - t6;
    float t10 = t8 * t9;
    float t11 = t7 - t10;
    float t12 = -t11;
    return t12;
}
// === GENERATED FIELD MATH END (boyko_shaderdsl::emit) ===

// Combine the accumulated field distance `acc` with one edit's distance `d` under
// the edit's boolean op, hard (`k <= 0`) or smooth (`k > 0`). NOT eDSL-generated:
// the `op` dispatch + the `(k>0)?smooth:hard` LAZY ternary are runtime control flow
// (Stage-2 eDSL territory), and the lazy branch matters — DXC lowers it to
// `OpBranchConditional`/`OpPhi` (smooth path skipped when `k<=0`), whereas the eager
// SSA-temp form the `Emit` backend produces lowers to `OpSelect` (both paths always
// evaluated). Same value, different SPIR-V: an eDSL `combine` would FORK the frozen
// `sdf_field_probe.baseline.dis`. So this stays hand-written; its only field MATH is
// the calls to the eDSL-generated `smin`/`smax` above (no polynomial lives here).
float combine(float acc, float d, uint op, float k) {
    if (op == OP_SUBTRACT) {
        return (k > 0.0) ? smax(acc, -d, k) : max(acc, -d);
    } else if (op == OP_INTERSECT) {
        return (k > 0.0) ? smax(acc, d, k) : max(acc, d);
    }
    return (k > 0.0) ? smin(acc, d, k) : min(acc, d);
}

// --- The edit-list field (the single source of truth, identical to rung 9/10) -
float sdf(float3 p) {
    uint n = min(Buf[0], MAX_SDF_EDITS); // word 0 = edit_count (clamped to capacity)
    float acc = FAR;
    [loop]
    for (uint i = 0u; i < n; ++i) {
        Edit e = load_edit(i);
        float d = edit_distance(e, p);
        if (i == 0u) {
            acc = d;
        } else {
            acc = combine(acc, d, e.op, e.smoothness);
        }
    }
    return acc;
}

// Surface normal via central differences of `sdf` (the gradient of the WHOLE
// edit-list field).
float3 sdf_normal(float3 p) {
    float2 e = float2(GRAD_H, 0.0);
    float3 n = float3(
        sdf(p + e.xyy) - sdf(p - e.xyy),
        sdf(p + e.yxy) - sdf(p - e.yxy),
        sdf(p + e.yyx) - sdf(p - e.yyx));
    return normalize(n);
}

// --- The stable field gateway (the P4 invariant) ------------------------------
// The named entry point every FIELD-CONSUMER (the P4b coarse cull, B1, A1, A2, P9)
// calls. Today a plain alias of the analytic `sdf`; when a backend swap lands
// (P9/P10/P12) ONLY this body changes — consumers stay byte-untouched.
float field_distance(float3 p) { return sdf(p); }

// --- The primary-march SKIP gateway (SDF brick-atlas campaign) -----------------
// CALLED ONLY by the primary-march skip/approach (M1); shadow/AO/normal/refine
// stay on analytic `sdf`/`field_distance`. Today a verbatim analytic alias; M1
// swaps THIS body to the brick fetch (a trilinear `R8_SNORM` sample that is a
// conservative LOWER BOUND on `sdf`, so the marcher never overshoots). Source-only
// for W0 — no shader yet calls it and no .spv is recompiled.
float field_skip(float3 p) { return sdf(p); }

// --- P4b: the conservative-lower-bound invariant + the smin Lipschitz constant -
//
// # FIELD LOWER-BOUND INVARIANT (D7; the sphere-tracing precondition — INVIOLABLE)
//
// Every op composing `field_distance` MUST return a value that is <= the true
// Euclidean distance from `p` to the field's surface (the Hart sphere-tracing
// precondition). A LOWER bound is safe: a sphere-trace / cone-trace step of that
// length can never overshoot the surface, so the cull (P4b) never carves a hole
// and over-relaxation (B1) / cone shadows (A1) / AO (A2) stay sound. An op that
// OVER-estimates the distance (steps too far) VOIDS P4/B1/A1/A2 — it would skip a
// surface contact. Audit of the current ops:
//   * sd_sphere / sd_box      — EXACT Euclidean distance (the bound is tight).
//   * min / max (hard CSG)    — EXACT for the boolean combination.
//   * smin / smax (k > 0)     — UNDER-report inside the blend band (the IQ poly
//                               carves the join inward), so the lower bound HOLDS.
// Any FUTURE op added here MUST preserve this invariant; the host property test
// `field_lipschitz_bound_holds` (compute.rs) is the numeric tripwire.
//
// # FIELD_LIPSCHITZ_L — the cone step's distance divisor (D7)
//
// `FIELD_LIPSCHITZ_L` is the k-INDEPENDENT worst-case spatial gradient magnitude
// of `field_distance`. The analytic primitives are unit-gradient (|grad| == 1);
// the IQ polynomial smin's steepest blend mixes two unit-gradient fields meeting
// at 90 degrees, whose combined gradient peaks at sqrt(2) (k sets the band WIDTH,
// not the peak slope). A cone-trace consumer divides the reported distance by L so
// the advance respects the TRUE (possibly steeper-than-1) clearance — `d / L` is a
// conservative lower bound on the Euclidean clearance even where smin is super-
// Lipschitz. Mirrored host-side as `FIELD_LIPSCHITZ_L` (compute.rs). OWNER CALL:
// sqrt(2) is the safe default; a hard-CSG-only (k == 0) scene could set L = 1 for
// tighter steps, but any future smooth edit re-introduces the super-Lipschitz peak
// (the host property test then fails loudly).
static const float FIELD_LIPSCHITZ_L = 1.41421356; // sqrt(2)
