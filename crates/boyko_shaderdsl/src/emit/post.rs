//! The render-wave PXe emitters: the `post_final` generator (the traced
//! [`crate::post::post_final_main`] body, the declarations and constants printed from the
//! single-source tables of [`crate::post`], and the whole-file template) and the per-facet probes
//! that pin each resource facet's HLSL spelling.
//!
//! `use super::*` surfaces the private recorder state and printer plumbing — the same registration
//! shape as the [`super::depth`] / [`super::prims`] siblings. The texture handles name their
//! resources through the printer's existing `Names::res_in` table (the Increment-5c resource
//! table), so no printer name table is widened.

use super::*;

use crate::cf::Cf;
use crate::scalar::FieldScalar;

/// The shared reset/seed/pop/print harness (the `emit_depth_leaf` shape): clears the recorder,
/// seeds the function-body block, runs `body` and prints the recorded block at depth 1 against the
/// given parameter-name tables (the named-literal and mutable-local tables are the ones the trace
/// interned).
fn emit_post_leaf<F: FnOnce()>(
    float_in: &[&str],
    uint_in: &[&str],
    vec_in: &[&str],
    vec4_in: &[&str],
    res_in: &[&str],
    body: F,
) -> String {
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
        uint_in,
        vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in,
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// One resource facet of the post body, for its emit pin ([`emit_hlsl_post_facet`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostFacet {
    /// `Cf::tex2_load` over a `Load`-only `gTex` at `(int)px`.
    Tex2Load,
    /// `Cf::tex2_sample_linear` over `gTex` + `gLinearClamp` at `uv`.
    Tex2SampleLinear,
    /// `Cf::tex3_sample_linear` over `gLut` + `gLinearClamp` at `uvw`.
    Tex3SampleLinear,
    /// `Cf::vec4_xyz` of a `float4 v`, materialised and returned with alpha 1.
    Vec4Xyz,
}

/// The probe's resource-name table: `res_in[0] = gTex`, `[1] = gLinearClamp`, `[2] = gLut`.
const FACET_RES: &[&str] = &["gTex", "gLinearClamp", "gLut"];

/// Traces one facet in a one-statement probe body and returns the printed span — the facet's
/// HLSL spelling, pinned by `boyko_shaderdsl/tests/post_facets.rs`.
pub fn emit_hlsl_post_facet(facet: PostFacet) -> String {
    emit_post_leaf(&[], &["px.x", "px.y"], &["uv", "uvw"], &["v"], FACET_RES, || {
        let ret = RetCellV4;
        let value = match facet {
            PostFacet::Tex2Load => {
                let t = TexParam { tex: 0, smp: TexParam::NO_SAMPLER };
                let x = EmitCf::int_from_uint(Emit::uint_input(0));
                let y = EmitCf::int_from_uint(Emit::uint_input(1));
                EmitCf::tex2_load(t, x, y)
            }
            PostFacet::Tex2SampleLinear => {
                let t = TexParam { tex: 0, smp: 1 };
                EmitCf::tex2_sample_linear(t, Emit(push(Node::Vec2Param(0))))
            }
            PostFacet::Tex3SampleLinear => {
                let t = TexParam { tex: 2, smp: 1 };
                EmitCf::tex3_sample_linear(t, Emit(push(Node::Vec3Param(1))))
            }
            PostFacet::Vec4Xyz => {
                let rgb = EmitCf::temp_vec3("rgb", EmitCf::vec4_xyz(Emit(push(Node::Vec4Param(0)))));
                EmitCf::vec4_from_scalars(
                    EmitCf::vec3_x(rgb),
                    EmitCf::vec3_y(rgb),
                    EmitCf::vec3_z(rgb),
                    <Emit as FieldScalar>::lit(1.0),
                )
            }
        };
        let _ = EmitCf::ret_vec4(&ret, value);
    })
}
