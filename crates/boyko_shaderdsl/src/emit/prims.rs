//! The render-wave E0 emitters: the five shared primitives (`floor`, `frac`, `log2`, `exp2`,
//! `pow`) traced over BOTH codegen axes that carry them — [`EmitCf`] (the `Cf` probe bodies of
//! [`crate::prims_probe`]) and [`Emit`] as [`InterpBackend`] — so the emit pins see each node's
//! spelling, operand order included, on each axis.
//!
//! `use super::*` surfaces the private recorder state and printer plumbing — the same
//! registration shape as the [`super::depth`] / [`super::shaders`] siblings. The reset/seed/print
//! harness below is the `emit_depth_leaf` shape, kept private to this file.

use super::*;

use crate::interp::InterpBackend;
use crate::prims_probe::{self, E0Op};

/// The shared reset/seed/pop/print harness: clears the recorder, seeds the function-body block,
/// runs `body` and prints the recorded block at depth 1. `float_in` is the probe's
/// float-parameter name table.
fn emit_prims_leaf<F: FnOnce()>(float_in: &[&str], body: F) -> String {
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

/// The float-parameter names of a probe of `op` (`x`, plus `y` for `pow`).
const fn probe_params(op: E0Op) -> &'static [&'static str] {
    match op.arity() {
        2 => &["x", "y"],
        _ => &["x"],
    }
}

/// Generates the BODY span of the `e0_<op>` probe function by tracing its
/// [`crate::prims_probe`] body over [`EmitCf`]:
///
/// ```text
///     return floor(x);
/// ```
pub fn emit_hlsl_e0_op(op: E0Op) -> String {
    emit_prims_leaf(probe_params(op), || {
        let x = Emit::input(0);
        let ret_out = RetCellF;
        let _ = match op {
            E0Op::Floor => prims_probe::e0_floor_body::<EmitCf>(x, &ret_out),
            E0Op::Frac => prims_probe::e0_frac_body::<EmitCf>(x, &ret_out),
            E0Op::Log2 => prims_probe::e0_log2_body::<EmitCf>(x, &ret_out),
            E0Op::Exp2 => prims_probe::e0_exp2_body::<EmitCf>(x, &ret_out),
            E0Op::Pow => {
                let y = Emit::input(1);
                prims_probe::e0_pow_body::<EmitCf>(x, y, &ret_out)
            }
        };
    })
}

/// Generates the body of the same one-op probe traced over [`Emit`] as [`InterpBackend`] (the
/// TRS-interpolation axis) — the straight-line SSA printer's shape:
///
/// ```text
///     float t0 = floor(x);
///     return t0;
/// ```
pub fn emit_hlsl_e0_interp_op(op: E0Op) -> String {
    trace_named(probe_params(op), NO_UINT_INPUTS, |f, _| match op {
        E0Op::Floor => InterpBackend::floor(f[0]),
        E0Op::Frac => InterpBackend::frac(f[0]),
        E0Op::Log2 => InterpBackend::log2(f[0]),
        E0Op::Exp2 => InterpBackend::exp2(f[0]),
        E0Op::Pow => InterpBackend::pow(f[0], f[1]),
    })
}

/// Wraps a generated BODY span in its sentinel pair + signature + closing brace (the
/// `emit_depth_merge` layout: the sentinels enclose the WHOLE function).
fn generated_fn(name: &str, sig: &str, body: &str) -> String {
    format!("// === GENERATED {name} BEGIN ===\n{sig} {{\n{body}}}\n// === GENERATED {name} END ===\n")
}

/// The HLSL signature of the `e0_<op>` probe function.
fn probe_sig(op: E0Op) -> String {
    let params: Vec<String> = probe_params(op).iter().map(|p| format!("float {p}")).collect();
    format!("float e0_{}({})", op.name(), params.join(", "))
}

/// The probe call `main` stores for `op` (`e0_pow(x, y)`).
fn probe_call(op: E0Op) -> String {
    format!("e0_{}({})", op.name(), probe_params(op).join(", "))
}

/// Generates the WHOLE committed `boyko_rhi_vulkan/shaders/e0_prims_probe.comp.hlsl` — the
/// TEST-ONLY device probe of the five E0 primitives: one generated one-op function per primitive
/// (each the trace of its [`crate::prims_probe`] body), called by a template `main` that reads
/// `(x, y)` from the element's words and stores each result's bits. The buffer layout constants
/// are generator inputs read from [`crate::prims_probe`], which the device test reads too.
///
/// The template lives HERE (not in the bin) so the sync test compares the whole committed file to
/// the generator's output, not only its spans.
pub fn emit_e0_prims_probe_comp() -> String {
    let mut fns = String::new();
    for op in E0Op::ALL {
        fns.push_str(&generated_fn(&format!("e0_{}", op.name()), &probe_sig(op), &emit_hlsl_e0_op(op)));
        fns.push('\n');
    }
    let mut stores = String::new();
    for (k, op) in E0Op::ALL.into_iter().enumerate() {
        stores.push_str(&format!(
            "    gData[base + {}u] = asuint({});\n",
            prims_probe::E0_PROBE_OUT_BASE as usize + k,
            probe_call(op)
        ));
    }
    format!(
        r#"// e0_prims_probe.comp -- render wave E0: the TEST-ONLY device probe of the five shared eDSL
// primitives `Cf::{{floor, frac, log2, exp2, pow}}` (`boyko_shaderdsl::prims_probe`). No render
// path binds it; only `boyko_rhi_vulkan/tests/e0_prims_device.rs` (gpu-ignored) dispatches it.
//
// GENERATED by `cargo run -p boyko_shaderdsl --features emit --bin emit_e0_probe`. The whole file
// is the generator's output (`boyko_shaderdsl::emit::emit_e0_prims_probe_comp`); a hand-edit fails
// `boyko_rhi_vulkan/tests/e0_prims_edsl_sync.rs`, which also byte-gates the committed .spv and
// counts each op's one GLSL.std.450 instruction.
//
// # Layout (the fixed compute layout: one storage buffer at set 0, binding 0)
//
// Element i owns words [i * {stride}, (i + 1) * {stride}): word 0 = asuint(x), word 1 = asuint(y)
// (host-written), words {out}.. = asuint() of floor(x), frac(x), log2(x), exp2(x), pow(x, y) (the
// device's), the rest unused. A 4-byte push constant carries the element count.
//
// No float-controls execution mode is declared: denormals may be flushed, NaN / Inf / -0 are not
// preserved and the rounding mode of a correctly rounded result is implementation-defined, which
// is why the device test gates only finite normal rows by "E0's bound" and reports the rest.
//
// # Compile (offline + hermetic; the committed .spv is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 e0_prims_probe.comp.hlsl -Fo e0_prims_probe.comp.spv

RWStructuredBuffer<uint> gData : register(u0);

struct E0ProbePush {{
    uint count;
}};
[[vk::push_constant]] E0ProbePush pc;

static const uint E0_PROBE_STRIDE = {stride}u;

{fns}[numthreads({local}, 1, 1)]
void main(uint3 dtid : SV_DispatchThreadID) {{
    uint i = dtid.x;
    if (i >= pc.count) {{
        return;
    }}
    uint base = i * E0_PROBE_STRIDE;
    float x = asfloat(gData[base]);
    float y = asfloat(gData[base + 1u]);
{stores}}}
"#,
        stride = prims_probe::E0_PROBE_STRIDE,
        out = prims_probe::E0_PROBE_OUT_BASE,
        local = prims_probe::E0_PROBE_LOCAL_SIZE,
    )
}
