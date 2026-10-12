//! The `emit_hlsl_sdf_t_to_depth_*` emitters for the SDF depth-merge encode ([`crate::depth`],
//! render wave RD1 / defect R0b).
//!
//! Each `emit_hlsl_*` function traces a [`crate::depth`] body over [`EmitCf`] and returns the BODY
//! span at depth 1 (4-space indent); [`emit_sdf_depth_merge_fs`] wraps each in its signature and its
//! `// === GENERATED <name> BEGIN/END ===` sentinels and returns the WHOLE committed
//! `sdf_depth_merge.fs.hlsl` (the `emit_depth_merge` bin only writes it to disk).
//!
//! `use super::*` surfaces the private recorder state and printer plumbing — the same registration
//! shape as the [`super::shaders`] / [`super::vb`] siblings. The harness below is the
//! `emit_ui_leaf` shape (named-literal table included, because both leaves spell
//! `SDF_DEPTH_MERGE_BIAS` as a symbol), kept private to this file.

use super::*;

use crate::depth;

/// The shared reset/seed/pop/print harness: clears the recorder, seeds the function-body block,
/// runs `body` and prints the recorded block at depth 1. `float_in` is the leaf's float-parameter
/// name table; the named-literal table is the one the trace interned.
fn emit_depth_leaf<F: FnOnce()>(float_in: &[&str], body: F) -> String {
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    body();

    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// Generates the **`sdf_t_to_depth_linear`** BODY span by tracing
/// [`crate::depth::sdf_t_to_depth_linear_body`] over `EmitCf` — the Deferred custom-linear encode.
///
/// ```text
///     float t_b = t * (1.0 + SDF_DEPTH_MERGE_BIAS);
///     return t_b / norm;
/// ```
pub fn emit_hlsl_sdf_t_to_depth_linear() -> String {
    emit_depth_leaf(&["t", "norm"], || {
        let t = Emit::input(0);
        let norm = Emit::input(1);
        let ret_out = RetCellF;
        let _ = depth::sdf_t_to_depth_linear_body::<EmitCf>(t, norm, &ret_out);
    })
}

/// Generates the **`sdf_t_to_depth_reverse_z`** BODY span by tracing
/// [`crate::depth::sdf_t_to_depth_reverse_z_body`] over `EmitCf` — the reverse-Z encode.
///
/// ```text
///     float view_z = t * (1.0 + SDF_DEPTH_MERGE_BIAS) * cos_fwd;
///     return a + b / view_z;
/// ```
pub fn emit_hlsl_sdf_t_to_depth_reverse_z() -> String {
    emit_depth_leaf(&["t", "cos_fwd", "a", "b"], || {
        let t = Emit::input(0);
        let cos_fwd = Emit::input(1);
        let a = Emit::input(2);
        let b = Emit::input(3);
        let ret_out = RetCellF;
        let _ = depth::sdf_t_to_depth_reverse_z_body::<EmitCf>(t, cos_fwd, a, b, &ret_out);
    })
}

/// Wraps a generated BODY span in its sentinel pair + doc comment + signature + closing brace
/// (the `emit_ui` / `emit_particles` layout: the sentinels enclose the WHOLE function).
fn generated_leaf(name: &str, doc: &str, sig: &str, body: &str) -> String {
    format!(
        "// === GENERATED {name} BEGIN ===\n{doc}{sig} {{\n{body}}}\n// === GENERATED {name} END ===\n"
    )
}

/// Generates the WHOLE committed `boyko_rhi_vulkan/shaders/sdf_depth_merge.fs.hlsl`: the
/// hand-written skeleton (bindings, push block, the `gViewT` load, the background `discard`, the
/// mode branch, `generate_ray`, the `SV_Depth` return) around the two generated leaf spans.
///
/// The skeleton lives HERE rather than in the `emit_depth_merge` bin so the sync test can compare
/// the committed file to the generator's output IN FULL — not only the sentinel spans. A
/// template that only the bin could print is exactly the `emit_ui` S5 hole: an edit to the
/// committed `main` that nothing compares, silently reverted by the next generator run.
pub fn emit_sdf_depth_merge_fs() -> String {
    let linear = generated_leaf(
        "sdf_t_to_depth_linear",
        "// Deferred custom-linear encode: the inverse of the composite's `t_mesh = md * mesh_norm`\n\
         // decode, biased (`boyko_shaderdsl::depth::sdf_t_to_depth_linear_body`).\n",
        "float sdf_t_to_depth_linear(float t, float norm)",
        &emit_hlsl_sdf_t_to_depth_linear(),
    );
    let reverse_z = generated_leaf(
        "sdf_t_to_depth_reverse_z",
        "// Reverse-Z encode: the inverse of the marcher's `t = (B / (d - A)) / cos_fwd` decode,\n\
         // biased (`boyko_shaderdsl::depth::sdf_t_to_depth_reverse_z_body`).\n",
        "float sdf_t_to_depth_reverse_z(float t, float cos_fwd, float a, float b)",
        &emit_hlsl_sdf_t_to_depth_reverse_z(),
    );

    format!(
        r#"// sdf_depth_merge.fs -- render wave RD1 (defect R0b): SDF surfaces write the path's depth, so
// depth-tested transparents (particles; translucent meshes from R1) are hidden behind them.
//
// GENERATED by `cargo run -p boyko_shaderdsl --features emit --bin emit_depth_merge`. The
// `// === GENERATED <name> BEGIN/END ===` spans are MACHINE-EMITTED; a hand-edit fails
// `boyko_rhi_vulkan/tests/sdf_depth_merge_edsl_sync.rs`, which also byte-gates the committed .spv.
//
// # What it does
//
// A full-screen triangle (`fullscreen_sample.vs`) covers every pixel. Each fragment reads the
// composite ray parameter `t` the SDF marcher left in gViewT, discards the background sentinel
// (`1.0e30`), and writes `encode(t * (1 + SDF_DEPTH_MERGE_BIAS))` through `SV_Depth`, depth-tested
// with the path's compare op (Deferred LESS, reverse-Z GREATER) and depth write ON. The bias is
// the discriminator: at a pixel a mesh won, `t` IS the decoded mesh `t`, so the biased encode is
// behind the stored mesh depth and the test rejects it; at a pixel the SDF won, it writes.
//
// The encode is the INVERSE of each path's marcher decode (`boyko_shaderdsl::depth`'s module doc):
// mode {mode_linear} = Deferred `t / mesh_norm` (k0 = 64 perspective, 10 ortho); mode {mode_rz} =
// reverse-Z `A + B / (t * max(dot(fwd, rd), 1e-4))` (k0 = A, k1 = B, `forward_view_z_coeffs`).
//
// # Resources (one dedicated set, FRAGMENT)
//
//   binding 0 : Texture2D<float> gViewT (SAMPLED, read at GENERAL -- `SampledImageAtGeneral`;
//               `.Load`, no sampler). NOT a storage image: a fragment-stage storage image without
//               `NonWritable` would need `fragmentStoresAndAtomics`, which the device does not
//               enable.
//   binding 1 : cbuffer Camera -- the 80-byte camera ring slot the marchers read (the SAME bytes,
//               so `generate_ray` reproduces the marcher's `rd` bit for bit).
//
// A 20-byte FRAGMENT `[[vk::push_constant]]` block: {{mode, k0, k1, img_w, img_h}}.
//
// # Compile (offline + hermetic; the committed .spv is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 sdf_depth_merge.fs.hlsl -Fo sdf_depth_merge.fs.spv

Texture2D<float> gViewT : register(t0);

// The 80-byte camera block -- byte-identical field layout to the marchers' `Camera`.
cbuffer Camera : register(b1) {{
    uint   count;        // UNREAD here (layout parity)
    uint   img_w_raw;    // UNREAD here: the push carries the extent the marcher's ray-gen used
    uint   img_h_raw;    // UNREAD here
    uint   camera_mode;  // RAYGEN_CAM_ORTHO | RAYGEN_CAM_PERSPECTIVE (see ray_gen.hlsli)
    float4 cam_eye;
    float4 cam_forward;  // xyz = forward basis, w = tan(fovY/2)
    float4 cam_right;    // xyz = right basis,   w = aspect (W/H)
    float4 cam_up;
}};

// The shared camera ray-gen (VERBATIM include -- the marchers' own; never a pasted copy).
#include "ray_gen.hlsli"

struct SdfDepthMergePush {{
    uint  mode;   // SDF_DEPTH_MERGE_MODE_LINEAR | SDF_DEPTH_MERGE_MODE_REVERSE_Z
    float k0;     // linear: the mesh-depth normalizer; reverse-Z: A
    float k1;     // reverse-Z: B (unread by the linear arm)
    uint  img_w;  // the extent the marcher's generate_ray used
    uint  img_h;
}};
[[vk::push_constant]] SdfDepthMergePush pc;

static const uint SDF_DEPTH_MERGE_MODE_LINEAR    = {mode_linear}u;
static const uint SDF_DEPTH_MERGE_MODE_REVERSE_Z = {mode_rz}u;
// beta = 2^-16 (`boyko_shaderdsl::depth::SDF_DEPTH_MERGE_BIAS`), exact in f32 and f64.
static const float SDF_DEPTH_MERGE_BIAS = 1.0 / {bias_denom}.0;
// Half the marchers' background sentinel (`1.0e30`): any t at or past it is "no surface".
static const float VIEWT_MISS = 0.5e30;

{linear}
{reverse_z}
float main(float4 frag_pos : SV_Position) : SV_Depth {{
    uint px = (uint)frag_pos.x;
    uint py = (uint)frag_pos.y;
    float t = gViewT.Load(int3((int)px, (int)py, 0));
    if (t >= VIEWT_MISS) {{
        discard;
    }}
    if (pc.mode == SDF_DEPTH_MERGE_MODE_LINEAR) {{
        return sdf_t_to_depth_linear(t, pc.k0);
    }}
    float3 ro;
    float3 rd;
    generate_ray(px, py, pc.img_w, pc.img_h, camera_mode, cam_eye.xyz, cam_forward, cam_right,
                 cam_up.xyz, ro, rd);
    // The marcher's own ray-to-view-depth factor, floor included (`sdf_forward_march.comp.hlsl`).
    float cos_fwd = max(dot(cam_forward.xyz, rd), 1.0e-4);
    return sdf_t_to_depth_reverse_z(t, cos_fwd, pc.k0, pc.k1);
}}
"#,
        mode_linear = depth::SDF_DEPTH_MERGE_MODE_LINEAR,
        mode_rz = depth::SDF_DEPTH_MERGE_MODE_REVERSE_Z,
        bias_denom = depth::SDF_DEPTH_MERGE_BIAS_DENOM,
    )
}
