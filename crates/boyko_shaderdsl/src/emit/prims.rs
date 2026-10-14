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
