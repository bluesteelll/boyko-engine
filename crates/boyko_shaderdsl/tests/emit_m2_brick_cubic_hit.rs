//! Increment 5c — the `m2_brick_cubic_hit` 3D-DDA marcher BODY-SPAN GENERATOR structure guard
//! (`feature = "emit"`).
//!
//! EMIT-ONLY: `m2_brick_cubic_hit` calls `m2_corner` → `atlas.SampleLevel(...)` (a `Texture3D` the
//! CPU cannot run), so `m2_brick_cubic_hit_body::<EvalCf>` is NEVER instantiated and there is NO
//! eval sweep. The cmp-`.spv` (in `boyko_rhi_vulkan`) is the AUTHORITATIVE byte-identity oracle —
//! the whole-`.spv` binary compare after the inline splice. THIS test is a finer-grained
//! generation-time guard on the emitted SPAN STRUCTURE — the load-bearing shapes the design calls
//! out:
//!
//! - The span is the BODY only; the hand-written signature, the `t_exit <= t_enter` early-out, the
//!   `const uint W = M2_BRICK_ALLOC;` decl, and the closing `}` stay un-generated (framing (b)), so
//!   the generator emits NO signature / no `const uint W`.
//! - The four named LOCAL ARRAYS are UNINITIALIZED decls (`int cell[3];` / `float s[8];`).
//! - The DDA step is the `+=` TOKEN (`cell[axis] += step[axis]`), NOT the `= cell[axis] +` form (the
//!   R1 finding — the `= +` form is byte-divergent at `-O0`).
//! - The 8 corners are `m2_corner(atlas, atlas_smp, tile_org, cx, cy, cz, inv_atlas, band_half)`.
//! - The cubic is `m2_jcgt_cubic_coeffs(s, lo_g, rd_v)` (a by-name array arg) then
//!   `m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo)`.
//! - The nearest-axis is the nested `uint` select `(... && ...) ? 0u : ((...) ? 1u : 2u)`.
//! - The captured `uint W` rides as `W - 2u` / `W - 1u` (NOT a value).
//! - The `(float)(c0 + 1)` cast WRAPS its additive operand (the O1 cast-operand paren).
//!
//! Gated on `feature = "emit"` (the generator surface is `#[cfg(feature = "emit")]`).

#![cfg(feature = "emit")]

fn generated() -> String {
    boyko_shaderdsl::emit::emit_hlsl_m2_brick_cubic_hit().replace("\r\n", "\n")
}

#[test]
fn span_is_body_only_no_signature_no_const_w() {
    let g = generated();
    // The hand-written signature + the `const uint W` decl stay un-generated (framing (b)).
    assert!(
        !g.contains("float m2_brick_cubic_hit"),
        "the generated span must NOT emit the hand-written signature:\n{g}"
    );
    assert!(
        !g.contains("const uint W"),
        "the `const uint W = M2_BRICK_ALLOC;` decl stays hand-written ABOVE the span:\n{g}"
    );
    assert!(
        !g.contains("if (t_exit <= t_enter)"),
        "the `t_exit <= t_enter` early-out stays hand-written ABOVE the span:\n{g}"
    );
}

#[test]
fn named_local_arrays_are_uninitialized_decls() {
    let g = generated();
    for decl in [
        "int cell[3];",
        "int step[3];",
        "float t_next[3];",
        "float t_delta[3];",
        "float s[8];",
    ] {
        assert!(
            g.contains(decl),
            "the span must declare the UNINITIALIZED array `{decl}`:\n{g}"
        );
    }
}

#[test]
fn dda_step_is_compound_add_token_not_desugared() {
    let g = generated();
    // The R1 finding: the `+=` TOKEN (one access-chain), NOT `cell[axis] = cell[axis] + step[axis]`.
    assert!(
        g.contains("cell[axis] += step[axis];"),
        "the cell step must be the `+=` token `cell[axis] += step[axis];`:\n{g}"
    );
    assert!(
        g.contains("t_next[axis] += t_delta[axis];"),
        "the t_next advance must be the `+=` token `t_next[axis] += t_delta[axis];`:\n{g}"
    );
    assert!(
        !g.contains("cell[axis] = cell[axis] +"),
        "the cell step must NOT be the desugared `= cell[axis] +` form (R1: byte-divergent):\n{g}"
    );
}

#[test]
fn corner_fetch_is_eight_arg_resource_call() {
    let g = generated();
    // s[0] is the canonical 8-arg `m2_corner` call (resource params `atlas`/`atlas_smp` + the
    // tile/cx/cy/cz/scalars). The `cx + 1u` neighbours exercise the `uadd(c, 1u)` path.
    assert!(
        g.contains("s[0] = m2_corner(atlas, atlas_smp, tile_org, cx, cy, cz, inv_atlas, band_half);"),
        "s[0] must be the 8-arg `m2_corner(atlas, atlas_smp, tile_org, cx, cy, cz, inv_atlas, band_half)`:\n{g}"
    );
    assert!(
        g.contains("s[7] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy + 1u, cz + 1u, inv_atlas, band_half);"),
        "s[7] must read the `cx + 1u`/`cy + 1u`/`cz + 1u` neighbour corner:\n{g}"
    );
}

#[test]
fn cubic_form_and_solve_calls() {
    let g = generated();
    // The by-name array arg `s` + the `float3` `lo_g`/`rd_v`; then the marmitt root over `[0, seg_hi
    // - seg_lo]`.
    assert!(
        g.contains("float4 coeffs = m2_jcgt_cubic_coeffs(s, lo_g, rd_v);"),
        "the cubic fold must be `m2_jcgt_cubic_coeffs(s, lo_g, rd_v)` (a by-name array arg):\n{g}"
    );
    assert!(
        g.contains("float local_t = m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo);"),
        "the root solve must be `m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo)`:\n{g}"
    );
}

#[test]
fn nearest_axis_is_nested_uint_select() {
    let g = generated();
    // The nested `uint` axis-select — the `&&` reduction + the nested ternary (the else arm wrapped).
    assert!(
        g.contains(
            "uint axis = (t_next[0] <= t_next[1] && t_next[0] <= t_next[2]) ? 0u : ((t_next[1] <= t_next[2]) ? 1u : 2u);"
        ),
        "the nearest-axis must be the nested `uint` select `(... && ...) ? 0u : ((...) ? 1u : 2u)`:\n{g}"
    );
}

#[test]
fn captured_w_spells_symbolic_subtract() {
    let g = generated();
    // The captured `uint W` rides as `W - 2u` (the cell clamp) and `W - 1u` (the DDA-exit guard) —
    // the SYMBOL, never a value.
    assert!(
        g.contains("min((uint)max(cell[0], 0), W - 2u)"),
        "the cell clamp must spell `min((uint)max(cell[0], 0), W - 2u)`:\n{g}"
    );
    assert!(
        g.contains("(uint)cell[axis] >= W - 1u"),
        "the DDA-exit high-face guard must spell `(uint)cell[axis] >= W - 1u`:\n{g}"
    );
    assert!(
        !g.contains("W - 2") || g.contains("W - 2u"),
        "the `W` subtract operands must be `uint` literals (`2u`/`1u`):\n{g}"
    );
}

#[test]
fn float_cast_wraps_additive_operand() {
    let g = generated();
    // The O1 cast-operand paren: `(float)(c0 + 1)`, NOT `(float)c0 + 1` (a cast binds tighter than
    // `+`). The plain `(float)c0` (no wrap) is the down branch.
    assert!(
        g.contains("float boundary = (float)(c0 + 1);"),
        "the `+1` boundary cast must WRAP its additive operand: `(float)(c0 + 1)`:\n{g}"
    );
    assert!(
        !g.contains("(float)c0 + 1"),
        "the cast must NOT spell the unwrapped `(float)c0 + 1` (precedence bug):\n{g}"
    );
    assert!(
        g.contains("float boundary = (float)c0;"),
        "the down-branch boundary must be the un-wrapped leaf cast `(float)c0`:\n{g}"
    );
}

#[test]
fn else_if_is_reflowed_to_else_block_if() {
    let g = generated();
    // The 3-way `else if` is authored as a nested `if_else` → printed `else {\n    if (...)` (the
    // `.spv`-neutral reflow). There is NO `else if` token in the generated text.
    assert!(
        !g.contains("else if"),
        "the 3-way branch must be the nested `else {{ if }}` reflow, NOT `else if`:\n{g}"
    );
}

#[test]
fn full_span_brace_matched_golden() {
    let g = generated();
    // The WHOLE body span, brace-matched — the canonical generated text (the committed L1021-1102
    // with the in-body comments stripped, the `else if`→`else { if }` reflow, and the single-line
    // `lo_g` ctor). A single golden so a structural drift (a missing statement, a reordered line, a
    // stray temp) fails loudly.
    const GOLDEN: &str = "    float t = t_enter;
    int cell[3];
    int step[3];
    float t_next[3];
    float t_delta[3];
    [unroll]
    for (uint axis = 0u; axis < 3u; ++axis) {
        float g_entry = ro_v[axis] + rd_v[axis] * t + M2_APRON - 0.5 + M2_ATLAS_BIAS;
        int c0 = (int)m2_clamp_index(g_entry);
        cell[axis] = c0;
        if (rd_v[axis] > 0.0) {
            step[axis] = 1;
            float boundary = (float)(c0 + 1);
            t_next[axis] = t + (boundary - g_entry) / rd_v[axis];
            t_delta[axis] = 1.0 / rd_v[axis];
        } else {
            if (rd_v[axis] < 0.0) {
                step[axis] = -1;
                float boundary = (float)c0;
                t_next[axis] = t + (boundary - g_entry) / rd_v[axis];
                t_delta[axis] = -1.0 / rd_v[axis];
            } else {
                step[axis] = 0;
                t_next[axis] = 1.0e30;
                t_delta[axis] = 1.0e30;
            }
        }
    }
    [loop]
    for (uint iter = 0u; iter < M2_MAX_CELLS; ++iter) {
        uint cx = min((uint)max(cell[0], 0), W - 2u);
        uint cy = min((uint)max(cell[1], 0), W - 2u);
        uint cz = min((uint)max(cell[2], 0), W - 2u);
        float s[8];
        s[0] = m2_corner(atlas, atlas_smp, tile_org, cx, cy, cz, inv_atlas, band_half);
        s[1] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy, cz, inv_atlas, band_half);
        s[2] = m2_corner(atlas, atlas_smp, tile_org, cx, cy + 1u, cz, inv_atlas, band_half);
        s[3] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy + 1u, cz, inv_atlas, band_half);
        s[4] = m2_corner(atlas, atlas_smp, tile_org, cx, cy, cz + 1u, inv_atlas, band_half);
        s[5] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy, cz + 1u, inv_atlas, band_half);
        s[6] = m2_corner(atlas, atlas_smp, tile_org, cx, cy + 1u, cz + 1u, inv_atlas, band_half);
        s[7] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy + 1u, cz + 1u, inv_atlas, band_half);
        float t_cell_exit = min(min(min(t_next[0], t_next[1]), t_next[2]), t_exit);
        float seg_lo = max(t, t_enter);
        float seg_hi = min(t_cell_exit, t_exit);
        if (seg_hi > seg_lo) {
            float3 lo_g = float3(ro_v[0] + rd_v[0] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS - (float)cx, ro_v[1] + rd_v[1] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS - (float)cy, ro_v[2] + rd_v[2] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS - (float)cz);
            float4 coeffs = m2_jcgt_cubic_coeffs(s, lo_g, rd_v);
            float local_t = m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo);
            if (local_t >= 0.0) {
                return seg_lo + local_t;
            }
        }
        if (t_cell_exit >= t_exit) {
            break;
        }
        uint axis = (t_next[0] <= t_next[1] && t_next[0] <= t_next[2]) ? 0u : ((t_next[1] <= t_next[2]) ? 1u : 2u);
        t = t_next[axis];
        cell[axis] += step[axis];
        t_next[axis] += t_delta[axis];
        if (step[axis] == 0 || cell[axis] < 0 || (uint)cell[axis] >= W - 1u) {
            break;
        }
    }
    return -1.0;
";
    assert_eq!(
        g, GOLDEN,
        "the generated m2_brick_cubic_hit span must match the brace-matched golden"
    );
}
