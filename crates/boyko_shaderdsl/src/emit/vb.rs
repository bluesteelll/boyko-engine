//! The `emit_hlsl_vb_*` shader-body emitters for the Visibility-Buffer analytic math
//! ([`crate::vb`], Multi-Paradigm Render-Path Plan §C/§F, rung R7).
//!
//! Each function traces a [`crate::vb`] body over the [`Emit`]/[`FieldScalar`] backend and
//! returns a SELF-CONTAINED HLSL function span (a full `RetType name(params) { ... }`),
//! wrapped in `// === GENERATED <name> BEGIN/END ===` sentinels matching the format
//! [`super::shaders`]'s `emit_hlsl_transform_interp`/`emit_hlsl_oct_encode` use. R7 does
//! NOT splice these into any shader (no rendering, no framegraph yet — R8/`R-VBGEO`
//! consume them); the spans are ready for that future splice.
//!
//! `use super::*` surfaces the private printer plumbing (`Emit`, `Node`, `Names`,
//! `ARENA`, `emit_temps`, `operand_str`, `OperandPos`, the `NO_*_INPUTS` default tables)
//! [`super::shaders`] also uses — `vb` is a THIRD sibling submodule of `emit`, the SAME
//! registration shape as `cf`/`shaders`.

use super::*;

use crate::vb;

/// The 6 float inputs of `vb_barycentric_grad(float3 vx, float3 vy)`, in
/// [`crate::vb::vb_barycentric_grad_body`]'s parameter order (`x` then `y`, each 3-wide).
const VB_BARY_GRAD_INPUTS: &[&str] = &["vx.x", "vx.y", "vx.z", "vy.x", "vy.y", "vy.z"];

/// The 10 float inputs of `vb_barycentric_eval(float3 dlambda_dx, float3 dlambda_dy, float
/// x0, float y0, float px, float py)`, in [`crate::vb::BaryBasis`] field order (`dlambda_dx`,
/// `dlambda_dy`, `x0`, `y0`) followed by the pixel `(px, py)`.
const VB_BARY_EVAL_INPUTS: &[&str] = &[
    "dlambda_dx.x",
    "dlambda_dx.y",
    "dlambda_dx.z",
    "dlambda_dy.x",
    "dlambda_dy.y",
    "dlambda_dy.z",
    "x0",
    "y0",
    "px",
    "py",
];

/// The 16 float inputs of `vb_interp(float3 dlambda_dx, float3 dlambda_dy, float x0, float
/// y0, float px, float py, float3 a, float3 w)`: the [`VB_BARY_EVAL_INPUTS`] basis+pixel
/// prefix, followed by the per-vertex attribute `a` and clip-space `w`.
const VB_INTERP_INPUTS: &[&str] = &[
    "dlambda_dx.x",
    "dlambda_dx.y",
    "dlambda_dx.z",
    "dlambda_dy.x",
    "dlambda_dy.y",
    "dlambda_dy.z",
    "x0",
    "y0",
    "px",
    "py",
    "a.x",
    "a.y",
    "a.z",
    "w.x",
    "w.y",
    "w.z",
];

/// The 19 float inputs of `vb_uv_grad(float3 dlambda_dx, float3 dlambda_dy, float x0, float
/// y0, float px, float py, float3 u, float3 v, float3 w)`: the basis+pixel prefix, followed
/// by the per-vertex `u`, `v`, and clip-space `w`.
const VB_UV_GRAD_INPUTS: &[&str] = &[
    "dlambda_dx.x",
    "dlambda_dx.y",
    "dlambda_dx.z",
    "dlambda_dy.x",
    "dlambda_dy.y",
    "dlambda_dy.z",
    "x0",
    "y0",
    "px",
    "py",
    "u.x",
    "u.y",
    "u.z",
    "v.x",
    "v.y",
    "v.z",
    "w.x",
    "w.y",
    "w.z",
];

/// The 12 float inputs of `vb_near_clip(float4 v0, float4 v1, float4 v2)`.
const VB_NEAR_CLIP_INPUTS: &[&str] = &[
    "v0.x", "v0.y", "v0.z", "v0.w", "v1.x", "v1.y", "v1.z", "v1.w", "v2.x", "v2.y", "v2.z", "v2.w",
];

/// Builds a [`Names`] with `float_in` set to `float_in` and every other table defaulted
/// (the straight-line VB leaves take no `uint`/vector/`out`/named-literal/mutable-local/
/// resource parameter) — factored out so the four `emit_hlsl_vb_*` entries below do not
/// each repeat the same 13-field default block ([`super::shaders::trace`] repeats it inline
/// per call; this crate's newer leaves factor it once).
fn vb_names<'a>(float_in: &'a [&'a str]) -> Names<'a> {
    Names {
        float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: NO_VARS,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    }
}

/// Seeds `count` `float` [`Emit::input`] handles (index 0..count, in table order) after
/// clearing the [`ARENA`] — the common trace-setup [`super::shaders::trace`]/`trace_named`
/// also perform inline.
fn seed_inputs(count: usize) -> Vec<Emit> {
    ARENA.with(|a| a.borrow_mut().clear());
    (0..count).map(|i| Emit::input(i as u32)).collect()
}

/// Like [`super::shaders`]'s `emit_body_vec4`, but for a `float3(...)` construct return
/// (`vb_barycentric_eval`'s `lambda`) — no leaf currently returns a bare `float3`, so this
/// is a NEW, minimal printer helper (reusing the SAME shared `emit_temps`/`operand_str` walk
/// every `emit_body_*` variant shares).
fn emit_body_vec3(arena: &[Node], names: Names, roots: [u32; 3]) -> String {
    let (mut out, temps) = emit_temps(arena, names);
    let r = |id: u32| operand_str(arena, names, &temps, id, OperandPos::Root);
    out.push_str(&format!(
        "    return float3({}, {}, {});\n",
        r(roots[0]),
        r(roots[1]),
        r(roots[2])
    ));
    out
}

/// Like [`super::shaders`]'s `emit_body_rows12`, but for the `VbBaryGrad{dlambda_dx,
/// dlambda_dy}` struct return (`vb_barycentric_grad`'s two `float3` gradient fields).
fn emit_body_vb_bary_grad(
    arena: &[Node],
    names: Names,
    dlambda_dx: [u32; 3],
    dlambda_dy: [u32; 3],
) -> String {
    let (mut out, temps) = emit_temps(arena, names);
    let r = |id: u32| operand_str(arena, names, &temps, id, OperandPos::Root);
    out.push_str("    VbBaryGrad g;\n");
    out.push_str(&format!(
        "    g.dlambda_dx = float3({}, {}, {});\n",
        r(dlambda_dx[0]),
        r(dlambda_dx[1]),
        r(dlambda_dx[2])
    ));
    out.push_str(&format!(
        "    g.dlambda_dy = float3({}, {}, {});\n",
        r(dlambda_dy[0]),
        r(dlambda_dy[1]),
        r(dlambda_dy[2])
    ));
    out.push_str("    return g;\n");
    out
}

/// Like [`super::shaders`]'s `emit_body_rows12`, but for the `VbClippedTri{v0, v1, v2}`
/// struct return (`vb_near_clip`'s three shrunk `float4` vertices).
fn emit_body_vb_near_clip(arena: &[Node], names: Names, roots: [u32; 12]) -> String {
    let (mut out, temps) = emit_temps(arena, names);
    let r = |id: u32| operand_str(arena, names, &temps, id, OperandPos::Root);
    out.push_str("    VbClippedTri c;\n");
    out.push_str(&format!(
        "    c.v0 = float4({}, {}, {}, {});\n",
        r(roots[0]),
        r(roots[1]),
        r(roots[2]),
        r(roots[3])
    ));
    out.push_str(&format!(
        "    c.v1 = float4({}, {}, {}, {});\n",
        r(roots[4]),
        r(roots[5]),
        r(roots[6]),
        r(roots[7])
    ));
    out.push_str(&format!(
        "    c.v2 = float4({}, {}, {}, {});\n",
        r(roots[8]),
        r(roots[9]),
        r(roots[10]),
        r(roots[11])
    ));
    out.push_str("    return c;\n");
    out
}

/// Generates the HLSL `vb_barycentric_grad` (the per-triangle constant gradient,
/// [`crate::vb::vb_barycentric_grad_body`]) AND `vb_barycentric_eval` (the per-pixel
/// weights, [`crate::vb::vb_barycentric_eval_body`]) function spans, by tracing both over
/// the [`Emit`] backend. Returns BOTH functions concatenated (mirrors `emit_hlsl_field`'s
/// multi-function return), each independently wrapped in its own `GENERATED ... BEGIN/END`
/// sentinels — `R8`/`R-VBGEO` splices each into `vb_geom_fetch.hlsli` separately (the
/// gradient once per triangle, the eval once per pixel).
pub fn emit_hlsl_vb_barycentric() -> String {
    let ins = seed_inputs(VB_BARY_GRAD_INPUTS.len());
    let (dldx, dldy) =
        vb::vb_barycentric_grad_body::<Emit>([ins[0], ins[1], ins[2]], [ins[3], ins[4], ins[5]]);
    let grad_body = ARENA.with(|a| {
        let a = a.borrow();
        emit_body_vb_bary_grad(
            &a,
            vb_names(VB_BARY_GRAD_INPUTS),
            [dldx[0].0, dldx[1].0, dldx[2].0],
            [dldy[0].0, dldy[1].0, dldy[2].0],
        )
    });

    let ins = seed_inputs(VB_BARY_EVAL_INPUTS.len());
    let basis = vb::BaryBasis {
        dlambda_dx: [ins[0], ins[1], ins[2]],
        dlambda_dy: [ins[3], ins[4], ins[5]],
        x0: ins[6],
        y0: ins[7],
    };
    let lambda = vb::vb_barycentric_eval_body::<Emit>(basis, ins[8], ins[9]);
    let eval_body = ARENA.with(|a| {
        let a = a.borrow();
        emit_body_vec3(
            &a,
            vb_names(VB_BARY_EVAL_INPUTS),
            [lambda[0].0, lambda[1].0, lambda[2].0],
        )
    });

    format!(
        "// === GENERATED vb_barycentric_grad BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_barycentric)\n\
         struct VbBaryGrad {{ float3 dlambda_dx; float3 dlambda_dy; }};\n\
         VbBaryGrad vb_barycentric_grad(float3 vx, float3 vy) {{\n{grad_body}}}\n\
         // === GENERATED vb_barycentric_grad END ===\n\
         \n\
         // === GENERATED vb_barycentric_eval BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_barycentric)\n\
         float3 vb_barycentric_eval(float3 dlambda_dx, float3 dlambda_dy, float x0, float y0, float px, float py) {{\n{eval_body}}}\n\
         // === GENERATED vb_barycentric_eval END ===\n",
    )
}

/// Generates the HLSL `vb_interp` perspective-correct attribute-interpolation function
/// span ([`crate::vb::vb_interp_body`]) by tracing it over the [`Emit`] backend. Returns
/// the full `float3 vb_interp(...) { ... }` function wrapped in `GENERATED` sentinels.
pub fn emit_hlsl_vb_interp() -> String {
    let ins = seed_inputs(VB_INTERP_INPUTS.len());
    let basis = vb::BaryBasis {
        dlambda_dx: [ins[0], ins[1], ins[2]],
        dlambda_dy: [ins[3], ins[4], ins[5]],
        x0: ins[6],
        y0: ins[7],
    };
    let out3 = vb::vb_interp_body::<Emit>(
        basis,
        ins[8],
        ins[9],
        [ins[10], ins[11], ins[12]],
        [ins[13], ins[14], ins[15]],
    );
    let body = ARENA.with(|a| {
        let a = a.borrow();
        emit_body_vec3(
            &a,
            vb_names(VB_INTERP_INPUTS),
            [out3[0].0, out3[1].0, out3[2].0],
        )
    });

    format!(
        "// === GENERATED vb_interp BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_interp)\n\
         float3 vb_interp(float3 dlambda_dx, float3 dlambda_dy, float x0, float y0, float px, float py, float3 a, float3 w) {{\n{body}}}\n\
         // === GENERATED vb_interp END ===\n",
    )
}

/// Generates the HLSL `vb_uv_grad` texcoord-gradient function span
/// ([`crate::vb::vb_uv_grad_body`]) by tracing it over the [`Emit`] backend. Returns the
/// full `float4 vb_uv_grad(...) { ... }` function wrapped in `GENERATED` sentinels.
pub fn emit_hlsl_vb_uv_grad() -> String {
    let ins = seed_inputs(VB_UV_GRAD_INPUTS.len());
    let basis = vb::BaryBasis {
        dlambda_dx: [ins[0], ins[1], ins[2]],
        dlambda_dy: [ins[3], ins[4], ins[5]],
        x0: ins[6],
        y0: ins[7],
    };
    let out4 = vb::vb_uv_grad_body::<Emit>(
        basis,
        ins[8],
        ins[9],
        [ins[10], ins[11], ins[12]],
        [ins[13], ins[14], ins[15]],
        [ins[16], ins[17], ins[18]],
    );
    let body = ARENA.with(|a| {
        let a = a.borrow();
        emit_body_vec4(
            &a,
            vb_names(VB_UV_GRAD_INPUTS),
            [out4[0].0, out4[1].0, out4[2].0, out4[3].0],
        )
    });

    format!(
        "// === GENERATED vb_uv_grad BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_uv_grad)\n\
         float4 vb_uv_grad(float3 dlambda_dx, float3 dlambda_dy, float x0, float y0, float px, float py, float3 u, float3 v, float3 w) {{\n{body}}}\n\
         // === GENERATED vb_uv_grad END ===\n",
    )
}

/// Generates the HLSL `vb_near_clip` simplified near-plane clip function span
/// ([`crate::vb::vb_near_clip_body`]) by tracing it over the [`Emit`] backend. Returns the
/// full `VbClippedTri vb_near_clip(...) { ... }` function wrapped in `GENERATED` sentinels.
pub fn emit_hlsl_vb_near_clip() -> String {
    let ins = seed_inputs(VB_NEAR_CLIP_INPUTS.len());
    let out = vb::vb_near_clip_body::<Emit>([
        [ins[0], ins[1], ins[2], ins[3]],
        [ins[4], ins[5], ins[6], ins[7]],
        [ins[8], ins[9], ins[10], ins[11]],
    ]);
    let roots: [u32; 12] = [
        out[0][0].0,
        out[0][1].0,
        out[0][2].0,
        out[0][3].0,
        out[1][0].0,
        out[1][1].0,
        out[1][2].0,
        out[1][3].0,
        out[2][0].0,
        out[2][1].0,
        out[2][2].0,
        out[2][3].0,
    ];
    let body = ARENA.with(|a| {
        let a = a.borrow();
        emit_body_vb_near_clip(&a, vb_names(VB_NEAR_CLIP_INPUTS), roots)
    });

    format!(
        "// === GENERATED vb_near_clip BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_near_clip)\n\
         struct VbClippedTri {{ float4 v0; float4 v1; float4 v2; }};\n\
         VbClippedTri vb_near_clip(float4 v0, float4 v1, float4 v2) {{\n{body}}}\n\
         // === GENERATED vb_near_clip END ===\n",
    )
}
