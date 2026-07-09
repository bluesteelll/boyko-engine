//! Increment 4b.2 — the `m2_surface_hit` REFINE-SPAN GENERATOR structure guard
//! (`feature = "emit"`).
//!
//! The cmp-`.spv` (in `boyko_rhi_vulkan`) is the AUTHORITATIVE byte-identity oracle; the
//! whole-`.spv` binary compare after the inline splice is the binding gate. THIS test is a
//! finer-grained generation-time guard on the emitted SPAN STRUCTURE — the load-bearing shapes
//! the design calls out:
//!
//! - The span is the REFINE LOOP+TAIL only (`float rt = cand_t; [loop] {...} return false;`);
//!   the integer cell-addressing PREAMBLE (the rel/tile guards, the M5 toroidal slot math) +
//!   the `m2_brick_span`/`m2_brick_cubic_hit`/`select_level` call sites are HAND-WRITTEN inline
//!   (framing (b)), so the generator emits NO `round` / `int3` / `%` / `m2_brick_span` etc.
//! - `rt` is a TRUE local (`float rt = cand_t;`).
//! - The runtime `[loop]` header spells the BOUND SYMBOL `M2_REFINE_ITERS`, NOT `8u`.
//! - `field_distance(ro + rd * rt)` is a CALL site (Inc-4b `call1`), not an inlined body.
//! - `d` and `step` are NAMED `float` temps (block FMA contraction at O3).
//! - The converged hit is `hit_t = rt; return true;` — `return true` is a REAL bool (the
//!   committed `OpConstantTrue`, NOT a `uint`), and `hit_t = rt;` is a bare out-param write.
//! - The step is `rt = rt + step;` (R1: the eDSL's natural `set_var` form — byte-identical to
//!   the committed `rt += step;` in the `.spv`).
//! - The escape `rt < 0.0 || rt > T_MAX` is a real `break;` (Inc-4b `brk`).
//! - The function tail is `return false;` (a REAL bool, the committed `OpConstantFalse`).
//!
//! Gated on `feature = "emit"` (the generator surface is `#[cfg(feature = "emit")]`).

#![cfg(feature = "emit")]

fn generated() -> String {
    boyko_shaderdsl::emit::emit_hlsl_m2_surface_hit_refine().replace("\r\n", "\n")
}

#[test]
fn span_is_refine_loop_plus_tail_only_no_preamble() {
    let g = generated();
    // The integer cell-addressing PREAMBLE + the call sites stay HAND-WRITTEN inline (framing
    // (b)) — the generator emits ONLY the refine loop+tail span between the sentinels. None of
    // the preamble's signature constructs may appear in the generated text.
    for forbidden in [
        "round(",         // the M5 origin_cell snap
        "int3",           // the signed slot math
        "%",              // the rem_euclid truncating modulo
        "m2_brick_span",  // the AABB-clip call site
        "m2_brick_cubic_hit", // the cubic-hit call site
        "select_level",   // the clip-map level scan
        "rel.x",          // the tile-rel float guard
        "cand_t = ",      // the candidate is an INPUT, never assigned in the span
    ] {
        assert!(
            !g.contains(forbidden),
            "the generated span must NOT emit the hand-written preamble construct `{forbidden}`:\n{g}"
        );
    }
}

#[test]
fn true_local_rt_declared_from_cand_t() {
    let g = generated();
    assert!(
        g.contains("float rt = cand_t;"),
        "must declare `float rt = cand_t;` (a true local seeded from the cubic candidate):\n{g}"
    );
}

#[test]
fn runtime_loop_header_spells_bound_symbol() {
    let g = generated();
    // The `[loop]` attribute + the BOUND SYMBOL `M2_REFINE_ITERS` in the header (NOT an `8u`
    // literal) — the difference from `[unroll]` that makes DXC emit a genuine OpLoop.
    assert!(g.contains("[loop]"), "must carry the `[loop]` attribute:\n{g}");
    assert!(
        g.contains("for (uint i = 0u; i < M2_REFINE_ITERS; ++i)"),
        "the loop header must spell the BOUND SYMBOL `M2_REFINE_ITERS`, not `8u`:\n{g}"
    );
    assert!(
        !g.contains("i < 8u"),
        "the loop header must spell the symbol, not the literal `8u`:\n{g}"
    );
}

#[test]
fn field_distance_is_a_call_site_with_named_d_temp() {
    let g = generated();
    // The field is spelled as a CALL (Inc-4b `call1`); `d` is a NAMED `float` local (NOT inlined
    // into the `abs(d)` guard — the two-rounding discipline that blocks FMA contraction).
    assert!(
        g.contains("float d = field_distance(ro + rd * rt);"),
        "the field must be a named call site `float d = field_distance(ro + rd * rt);`:\n{g}"
    );
}

#[test]
fn step_is_named_temp_not_inlined_no_fma() {
    let g = generated();
    // `step` is a NAMED `float` temp (`float step = M2_REFINE_RELAX * d;`) — pinned so DXC at O3
    // does NOT contract `rt = rt + M2_REFINE_RELAX * d` into an `Fma` (the committed two-rounding
    // discipline: one rounding for the multiply, one for the add).
    assert!(
        g.contains("float step = M2_REFINE_RELAX * d;"),
        "`step` must be a named temp `float step = M2_REFINE_RELAX * d;` (block FMA):\n{g}"
    );
    assert!(
        !g.contains("rt = rt + M2_REFINE_RELAX * d"),
        "the step must NOT inline the multiply into the add (would risk FMA contraction):\n{g}"
    );
}

#[test]
fn step_accumulation_is_set_var_plus_form_r1() {
    let g = generated();
    // R1: the eDSL's natural `set_var` form `rt = rt + step;` (byte-identical to the committed
    // `rt += step;` in the `.spv`), so NO compound-assign leaf was added.
    assert!(
        g.contains("rt = rt + step;"),
        "the step must spell `rt = rt + step;` (R1 — the natural set_var form):\n{g}"
    );
}

#[test]
fn converged_hit_writes_hit_t_then_returns_true() {
    let g = generated();
    // The in-loop converged-hit guard `if (abs(d) < EPS)` records BOTH `hit_t = rt;` (the bare
    // out-param write) THEN `return true;` (a REAL bool) — the composite combinator.
    assert!(
        g.contains("if (abs(d) < EPS) {"),
        "the converged-hit guard must spell `if (abs(d) < EPS) {{`:\n{g}"
    );
    assert!(
        g.contains("hit_t = rt;"),
        "the hit must write the bare out-param `hit_t = rt;`:\n{g}"
    );
    assert!(
        g.contains("return true;"),
        "the hit must `return true;` (a REAL bool, the committed OpConstantTrue — NOT `1u`):\n{g}"
    );
    // The bool return must NOT be a uint sentinel.
    assert!(
        !g.contains("return 1u;") && !g.contains("return 1;"),
        "the bool return must be `true`, never a uint `1u`/`1`:\n{g}"
    );
}

#[test]
fn escape_is_a_real_break_on_signed_or_guard() {
    let g = generated();
    // The `rt < 0.0 || rt > T_MAX` escape is a real `break;` (Inc-4b `brk`), guarded by an `if`.
    assert!(
        g.contains("if (rt < 0.0 || rt > T_MAX) {"),
        "the escape guard must spell `if (rt < 0.0 || rt > T_MAX) {{`:\n{g}"
    );
    assert!(g.contains("break;"), "the escape must emit a real `break;`:\n{g}");
}

#[test]
fn tail_returns_false() {
    let g = generated();
    // The function tail is `return false;` (a REAL bool, the committed OpConstantFalse — NOT a
    // uint sentinel).
    assert!(
        g.contains("return false;"),
        "the tail must spell `return false;` (a REAL bool, the committed OpConstantFalse):\n{g}"
    );
    assert!(
        !g.contains("return 0u;") && !g.contains("return 0;"),
        "the bool tail must be `false`, never a uint `0u`/`0`:\n{g}"
    );
}

#[test]
fn full_span_brace_matched_golden() {
    let g = generated();
    // The WHOLE span, brace-matched — the canonical generated text (the committed L1184-1205 with
    // comments stripped + the R1 `rt = rt + step;` form). A single golden so a structural drift
    // (a missing statement, a reordered line, a stray temp) fails loudly.
    const GOLDEN: &str = "    float rt = cand_t;
    [loop]
    for (uint i = 0u; i < M2_REFINE_ITERS; ++i) {
        float d = field_distance(ro + rd * rt);
        if (abs(d) < EPS) {
            hit_t = rt;
            return true;
        }
        float step = M2_REFINE_RELAX * d;
        rt = rt + step;
        if (rt < 0.0 || rt > T_MAX) {
            break;
        }
    }
    return false;
";
    assert_eq!(g, GOLDEN, "the generated span must match the brace-matched golden");
}
