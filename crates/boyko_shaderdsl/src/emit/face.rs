//! The `emit_hlsl_mesh_face_*` printers for RD6 / D-DS's facing leaves ([`crate::face`]).
//!
//! Each returns a self-contained HLSL span the `emit_mesh_face` generator wraps in
//! `// === GENERATED <name> BEGIN/END ===` sentinels; the three opaque producer sources splice
//! them verbatim and `boyko_rhi_vulkan/tests/mesh_face_edsl_sync.rs` pins every splice to this
//! output. `use super::*` reaches the recorder plumbing exactly as the [`super::vb`] /
//! [`super::shaders`] siblings do.

use super::*;

use crate::face;

/// Traces `body` over a fresh recorder state and prints the function body block — the
/// [`super::shaders`] `emit_ui_leaf` discipline (named `uint` symbols reach the printer through
/// the interned table).
fn emit_face_leaf<F: FnOnce()>(float_in: &[&str], uint_in: &[&str], body: F) -> String {
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

/// The face-bit constants span — the one HLSL spelling of
/// `boyko_rhi::enums::MESH_FACE_*` (generated from [`crate::face`]'s constants).
pub fn emit_hlsl_mesh_face_consts() -> String {
    format!(
        "\
static const uint MESH_FACE_DOUBLE_SIDED_SHIFT = {ds_shift}u;
static const uint MESH_FACE_MIRRORED_SHIFT = {mi_shift}u;
static const uint MESH_FACE_DOUBLE_SIDED = 1u << MESH_FACE_DOUBLE_SIDED_SHIFT;
static const uint MESH_FACE_MIRRORED = 1u << MESH_FACE_MIRRORED_SHIFT;
",
        ds_shift = face::MESH_FACE_DOUBLE_SIDED_SHIFT,
        mi_shift = face::MESH_FACE_MIRRORED_SHIFT,
    )
}

/// The full `uint mesh_face_flip(uint front_ccw, uint face)` function, traced from
/// [`face::mesh_face_flip_body`].
pub fn emit_hlsl_mesh_face_flip() -> String {
    let body = emit_face_leaf(&[], &["front_ccw", "face"], || {
        let front_ccw = Emit::uint_input(0);
        let word = Emit::uint_input(1);
        let ret = RetCell;
        let _ = face::mesh_face_flip_body::<EmitCf>(front_ccw, word, &ret);
    });
    format!("uint mesh_face_flip(uint front_ccw, uint face) {{\n{body}}}\n")
}

/// The full `uint vb_screen_front_ccw(float3 vx, float3 vy)` function, traced from
/// [`face::vb_screen_front_ccw_body`].
pub fn emit_hlsl_vb_screen_front_ccw() -> String {
    const INPUTS: &[&str] = &["vx.x", "vx.y", "vx.z", "vy.x", "vy.y", "vy.z"];
    let body = emit_face_leaf(INPUTS, &[], || {
        let vx = [Emit::input(0), Emit::input(1), Emit::input(2)];
        let vy = [Emit::input(3), Emit::input(4), Emit::input(5)];
        let ret = RetCell;
        let _ = face::vb_screen_front_ccw_body::<EmitCf>(vx, vy, &ret);
    });
    format!("uint vb_screen_front_ccw(float3 vx, float3 vy) {{\n{body}}}\n")
}
