//! Increment 4b — the `sdf_soft_shadow` GENERATOR structure guard (`feature = "emit"`).
//!
//! The cmp-`.spv` (in `boyko_rhi_vulkan`) is the AUTHORITATIVE byte-identity oracle; the
//! whole-`.spv` binary compare after the inline splice is the binding gate. THIS test is a
//! finer-grained generation-time guard on the emitted SPAN STRUCTURE — the load-bearing
//! shapes the design calls out:
//!
//! - The span is the LOOP+TAIL only (`float res; float t; [loop] {...} return clamp(...)`);
//!   the `dot(n, L)` preamble is HAND-WRITTEN inline (framing (b)), so the generator emits NO
//!   `dot(` / `<= SHADOW_NDOTL_EPS` / preamble `return 0.0` outside the loop.
//! - `res` / `t` are TRUE locals (`float res = 1.0;` / `float t = SHADOW_MINT;`).
//! - The runtime `[loop]` header spells the BOUND SYMBOL `MAX_IT`, NOT `128u`.
//! - `field_distance(p + L * t)` is a CALL site (Inc-4b `call1`), not an inlined body.
//! - The penumbra-min is `min(res, SHADOW_K * d / t)` un-parenthesized (R2 precedence).
//! - The step is `t = t + max(...)` (R1: the eDSL's natural `set_var` form — byte-identical to
//!   the committed `t += max(...)` in the `.spv`).
//! - The `t > T_MAX` break is a real `break;` (Inc-4b `brk`).
//! - The tail is `return clamp(res, 0.0, 1.0);` (the `clamp01` primitive).
//!
//! Gated on `feature = "emit"` (the generator surface is `#[cfg(feature = "emit")]`).

#![cfg(feature = "emit")]

fn generated() -> String {
    boyko_shaderdsl::emit::emit_hlsl_sdf_soft_shadow().replace("\r\n", "\n")
}

#[test]
fn span_is_loop_plus_tail_only_no_preamble() {
    let g = generated();
    // The `dot(n, L)` early-return PREAMBLE stays HAND-WRITTEN inline (framing (b)) — the
    // generator emits ONLY the loop+tail span between the sentinels.
    assert!(
        !g.contains("dot("),
        "the generated span must NOT emit the `dot(n, L)` preamble (hand-written inline):\n{g}"
    );
    assert!(
        !g.contains("SHADOW_NDOTL_EPS"),
        "the generated span must NOT reference the preamble's SHADOW_NDOTL_EPS:\n{g}"
    );
}

#[test]
fn true_locals_res_and_t_declared() {
    let g = generated();
    assert!(
        g.contains("float res = 1.0;"),
        "must declare `float res = 1.0;` (a true local):\n{g}"
    );
    // `t` initializes to the SYMBOL `SHADOW_MINT` (not the value-spelled `0.008`).
    assert!(
        g.contains("float t = SHADOW_MINT;"),
        "must declare `float t = SHADOW_MINT;` (the symbol, not a literal):\n{g}"
    );
}

#[test]
fn runtime_loop_header_spells_bound_symbol() {
    let g = generated();
    // The `[loop]` attribute + the BOUND SYMBOL `MAX_IT` in the header (NOT a `128u` literal)
    // — the difference from `[unroll]` that makes DXC emit a genuine OpLoop.
    assert!(g.contains("[loop]"), "must carry the `[loop]` attribute:\n{g}");
    assert!(
        g.contains("for (uint i = 0u; i < MAX_IT; ++i)"),
        "the loop header must spell the BOUND SYMBOL `MAX_IT`, not `128u`:\n{g}"
    );
    assert!(
        !g.contains("i < 128u"),
        "the loop header must spell the symbol, not the literal `128u`:\n{g}"
    );
}

#[test]
fn field_distance_is_a_call_site_not_inlined() {
    let g = generated();
    // The field is spelled as a CALL (Inc-4b `call1`); the `d` is a NAMED `float` local.
    assert!(
        g.contains("float d = field_distance(p + L * t);"),
        "the field must be a call site `float d = field_distance(p + L * t);`:\n{g}"
    );
}

#[test]
fn penumbra_min_is_unparenthesized() {
    let g = generated();
    // `min(res, SHADOW_K * d / t)` — the R2 precedence: `((K*d)/t)` prints un-parenthesized.
    assert!(
        g.contains("res = min(res, SHADOW_K * d / t);"),
        "the penumbra-min must spell `min(res, SHADOW_K * d / t)` un-parenthesized:\n{g}"
    );
}

#[test]
fn step_is_set_var_plus_form_r1() {
    let g = generated();
    // R1: the eDSL's natural `set_var` form `t = t + max(...)` (byte-identical to the committed
    // `t += max(...)` in the `.spv`), so NO compound-assign leaf was added.
    assert!(
        g.contains("t = t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP);"),
        "the step must spell `t = t + max(...)` (R1 — the natural set_var form):\n{g}"
    );
}

#[test]
fn escape_is_a_real_break() {
    let g = generated();
    // The `t > T_MAX` escape is a real `break;` (Inc-4b `brk`), guarded by an `if`.
    assert!(
        g.contains("if (t > T_MAX) {"),
        "the escape guard must spell `if (t > T_MAX) {{`:\n{g}"
    );
    assert!(g.contains("break;"), "the escape must emit a real `break;`:\n{g}");
}

#[test]
fn occluder_hit_early_return_and_clamp_tail() {
    let g = generated();
    // The in-loop occluder-hit early return.
    assert!(
        g.contains("if (d < SHADOW_HIT_EPS) {"),
        "the occluder-hit guard must spell `if (d < SHADOW_HIT_EPS) {{`:\n{g}"
    );
    assert!(
        g.contains("return 0.0;"),
        "the occluder hit must `return 0.0;`:\n{g}"
    );
    // The tail clamp (the `clamp01` primitive bakes `clamp(x, 0.0, 1.0)`).
    assert!(
        g.contains("return clamp(res, 0.0, 1.0);"),
        "the tail must spell `return clamp(res, 0.0, 1.0);` (clamp01):\n{g}"
    );
}

#[test]
fn full_span_brace_matched_golden() {
    let g = generated();
    // The WHOLE span, brace-matched — the canonical generated text (the committed L454-468 with
    // comments stripped + the R1 `t = t + ...` form). A single golden so a structural drift
    // (a missing statement, a reordered line, a stray temp) fails loudly.
    const GOLDEN: &str = "    float res = 1.0;
    float t = SHADOW_MINT;
    [loop]
    for (uint i = 0u; i < MAX_IT; ++i) {
        float d = field_distance(p + L * t);
        res = min(res, SHADOW_K * d / t);
        if (d < SHADOW_HIT_EPS) {
            return 0.0;
        }
        t = t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP);
        if (t > T_MAX) {
            break;
        }
    }
    return clamp(res, 0.0, 1.0);
";
    assert_eq!(g, GOLDEN, "the generated span must match the brace-matched golden");
}
