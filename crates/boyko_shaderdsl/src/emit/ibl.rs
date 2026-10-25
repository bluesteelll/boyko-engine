//! IBL1 R1 — the `env_ibl.hlsli` emitters: the read-side leaves of [`crate::ibl`] traced over the
//! `Emit` backend into the spans between the header's `// === GENERATED env_ibl BEGIN/END ===`
//! sentinels (`boyko_rhi_vulkan/tests/env_ibl_edsl_sync.rs` re-runs [`emit_hlsl_env_ibl`] and
//! fails on any committed span that differs). The leaves are `FieldScalar` bodies, so every span is
//! a scalar SSA trace: one `float tN` per non-leaf node, the inputs spelled as the HLSL parameter
//! lanes (`d.x`, `s.f0.y`, …).
//!
//! Split from `shaders.rs` like `face.rs`; `use super::*` surfaces the recorder internals.

use super::*;

use crate::ibl;

/// The `float3` parameter `name` spelled as its three traced scalar inputs (`name.x` …).
fn lanes3(name: &str) -> [String; 3] {
    [format!("{name}.x"), format!("{name}.y"), format!("{name}.z")]
}

/// Traces `body` over `float_names` scalar inputs and prints `return floatN(r0, …);` over its
/// `N` roots (the `trace_named_vec2` shape, any width).
fn trace_lanes<const N: usize, F: FnOnce(&[Emit]) -> [Emit; N]>(float_names: &[&str], body: F) -> String {
    let names = Names {
        float_in: float_names,
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
    };
    ARENA.with(|a| a.borrow_mut().clear());
    let mut floats = Vec::with_capacity(float_names.len());
    for i in 0..float_names.len() {
        floats.push(Emit::input(i as u32));
    }
    let result = body(&floats);
    ARENA.with(|a| {
        let arena = a.borrow();
        let (mut out, temps) = emit_temps(&arena, names);
        let lanes: Vec<String> =
            result.iter().map(|r| operand_str(&arena, names, &temps, r.0, OperandPos::Root)).collect();
        out.push_str(&format!("    return float{N}({});\n", lanes.join(", ")));
        out
    })
}

/// `float env_lod(float pr, float l1)` — [`ibl::env_lod_body`].
pub fn emit_hlsl_env_lod() -> String {
    let body = trace_named(&["pr", "l1"], NO_UINT_INPUTS, |f, _u| ibl::env_lod_body::<Emit>(f[0], f[1]));
    format!("float env_lod(float pr, float l1) {{\n{body}}}\n")
}

/// `float2 oct_env_uv(float3 d, float tile)` — [`ibl::oct_env_uv_body`].
pub fn emit_hlsl_oct_env_uv() -> String {
    let names: Vec<String> = lanes3("d").into_iter().chain(["tile".to_string()]).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let body = trace_lanes(&refs, |f| ibl::oct_env_uv_body::<Emit>([f[0], f[1], f[2]], f[3]));
    format!("float2 oct_env_uv(float3 d, float tile) {{\n{body}}}\n")
}

/// `float3 spec_dominant_dir(float3 n, float3 r, float pr)` — [`ibl::spec_dominant_dir_body`].
pub fn emit_hlsl_spec_dominant_dir() -> String {
    let names: Vec<String> = [lanes3("n"), lanes3("r")].concat().into_iter().chain(["pr".to_string()]).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let body = trace_lanes(&refs, |f| ibl::spec_dominant_dir_body::<Emit>([f[0], f[1], f[2]], [f[3], f[4], f[5]], f[6]));
    format!("float3 spec_dominant_dir(float3 n, float3 r, float pr) {{\n{body}}}\n")
}

/// `float3 env_yaw_rotate(float3 d, float c, float s)` — [`ibl::env_yaw_rotate_body`].
pub fn emit_hlsl_env_yaw_rotate() -> String {
    let names: Vec<String> = lanes3("d").into_iter().chain(["c".to_string(), "s".to_string()]).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let body = trace_lanes(&refs, |f| ibl::env_yaw_rotate_body::<Emit>([f[0], f[1], f[2]], f[3], f[4]));
    format!("float3 env_yaw_rotate(float3 d, float c, float s) {{\n{body}}}\n")
}

/// `float3 eval_pbr_ambient_env(Surface s, float2 dfg, float3 env_spec, float3 sky_color,
/// float3 ground_color, float hemi, float ao_final, float spec_ao)` —
/// [`ibl::eval_pbr_ambient_env_body`], the `eval_pbr_ambient_hemi` signature with `R` replaced by
/// the prefiltered `env_spec` (`Surface` is `pbr_lighting.hlsli`'s carrier).
pub fn emit_hlsl_eval_pbr_ambient_env() -> String {
    let mut names: Vec<String> = Vec::with_capacity(23);
    for v in ["s.f0", "s.energy_comp", "s.diffuse_color"] {
        names.extend(lanes3(v));
    }
    names.extend(["dfg.x", "dfg.y"].map(String::from));
    for v in ["env_spec", "sky_color", "ground_color"] {
        names.extend(lanes3(v));
    }
    names.extend(["hemi", "ao_final", "spec_ao"].map(String::from));
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let body = trace_lanes(&refs, |f| {
        ibl::eval_pbr_ambient_env_body::<Emit>(
            [f[0], f[1], f[2]],
            [f[3], f[4], f[5]],
            [f[6], f[7], f[8]],
            [f[9], f[10]],
            [f[11], f[12], f[13]],
            [f[14], f[15], f[16]],
            [f[17], f[18], f[19]],
            f[20],
            f[21],
            f[22],
        )
    });
    format!(
        "float3 eval_pbr_ambient_env(Surface s, float2 dfg, float3 env_spec, float3 sky_color, \
         float3 ground_color, float hemi, float ao_final, float spec_ao) {{\n{body}}}\n"
    )
}

/// The SKY-row flag constant and its predicate, printed from [`ibl::LIGHT_FLAG_ENV`] (the host's
/// `boyko_render::light::LIGHT_FLAG_ENV` is pinned equal by a test).
pub fn emit_hlsl_light_env_armed() -> String {
    format!(
        "static const uint LIGHT_FLAG_ENV = 0x{:X}u;\n\nbool light_env_armed(LightElem L) {{\n    return (L.kind & LIGHT_FLAG_ENV) != 0u;\n}}\n",
        ibl::LIGHT_FLAG_ENV
    )
}

/// Every generated span of `env_ibl.hlsli`, in file order, one blank line apart.
pub fn emit_hlsl_env_ibl() -> String {
    [
        emit_hlsl_light_env_armed(),
        emit_hlsl_env_lod(),
        emit_hlsl_oct_env_uv(),
        emit_hlsl_spec_dominant_dir(),
        emit_hlsl_env_yaw_rotate(),
        emit_hlsl_eval_pbr_ambient_env(),
    ]
    .join("\n")
}
