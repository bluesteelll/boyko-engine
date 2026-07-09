//! Increment 5a — the `select_level` clip-map SCAN-SPAN GENERATOR structure guard
//! (`feature = "emit"`).
//!
//! The cmp-`.spv` (in `boyko_rhi_vulkan`) is the AUTHORITATIVE byte-identity oracle; the
//! whole-`.spv` binary compare after the inline splice is the binding gate. THIS test is a
//! finer-grained generation-time guard on the emitted SPAN STRUCTURE — the load-bearing shapes the
//! design calls out:
//!
//! - The span is the SCAN only; the hand-written `int select_level(float3 p) {` signature + the
//!   closing `}` stay un-generated (framing (b)), so the generator emits NO `int select_level` /
//!   `{` signature wrap.
//! - The `[unroll]` loop header spells the BOUND SYMBOL `BRICK_LEVELS`, NOT `3u`, and the iv `L`.
//! - The runtime count guard is `if (L >= pc.brick_levels)` -> `break;`.
//! - `o` / `bw` / `hi` are NAMED temps (`float3 o`, `float bw`, `float3 hi`) read by ACCESS TEXT
//!   (`m2_levels[L].origin_brick_world.xyz` / `.w` / `dims_atlas_dim.xyz`).
//! - The containment hit is `if (all(p >= o) && all(p < hi))` -> `return (int)L;` — the `(int)L`
//!   is a SIGNED cast (NOT a `<x>u`), and the condition is the `all(...) && all(...)` bool3
//!   reduction.
//! - The tail is `return -1;` — a BARE signed literal (the committed `OpConstant -1`, NOT a
//!   `4294967295u` unsigned).
//!
//! Gated on `feature = "emit"` (the generator surface is `#[cfg(feature = "emit")]`).

#![cfg(feature = "emit")]

fn generated() -> String {
    boyko_shaderdsl::emit::emit_hlsl_select_level().replace("\r\n", "\n")
}

#[test]
fn span_is_scan_only_no_signature() {
    let g = generated();
    // The hand-written signature stays un-generated (framing (b)) — the generator emits ONLY the
    // scan span between the sentinels.
    assert!(
        !g.contains("int select_level"),
        "the generated span must NOT emit the hand-written `int select_level` signature:\n{g}"
    );
}

#[test]
fn unroll_header_spells_bound_symbol_and_iv() {
    let g = generated();
    // The `[unroll]` attribute + the BOUND SYMBOL `BRICK_LEVELS` in the header (NOT a `3u` literal)
    // + the iv `L` — the committed clip-map scan header.
    assert!(g.contains("[unroll]"), "must carry the `[unroll]` attribute:\n{g}");
    assert!(
        g.contains("for (uint L = 0u; L < BRICK_LEVELS; ++L)"),
        "the loop header must spell the BOUND SYMBOL `BRICK_LEVELS` + the iv `L`, not `3u`:\n{g}"
    );
    assert!(
        !g.contains("L < 3u"),
        "the loop header must spell the symbol, not the literal `3u`:\n{g}"
    );
}

#[test]
fn runtime_count_guard_breaks() {
    let g = generated();
    assert!(
        g.contains("if (L >= pc.brick_levels) {"),
        "the runtime count guard must spell `if (L >= pc.brick_levels) {{`:\n{g}"
    );
    assert!(g.contains("break;"), "the inactive-level guard must `break;`:\n{g}");
}

#[test]
fn level_fields_read_by_access_text_named_temps() {
    let g = generated();
    // `o` / `bw` / `hi` are NAMED temps read by the M4Level access text (the struct layout is NOT
    // modeled — only `m2_levels[L].<member>.<swizzle>`).
    assert!(
        g.contains("float3 o = m2_levels[L].origin_brick_world.xyz;"),
        "`o` must read `m2_levels[L].origin_brick_world.xyz`:\n{g}"
    );
    assert!(
        g.contains("float bw = m2_levels[L].origin_brick_world.w;"),
        "`bw` must read the scalar `m2_levels[L].origin_brick_world.w`:\n{g}"
    );
    assert!(
        g.contains("float3 hi = o + m2_levels[L].dims_atlas_dim.xyz * bw;"),
        "`hi` must be `o + m2_levels[L].dims_atlas_dim.xyz * bw`:\n{g}"
    );
}

#[test]
fn containment_hit_is_all_reduction_returns_signed_int_cast() {
    let g = generated();
    // The hit condition is the `all(p >= o) && all(p < hi)` bool3 reduction (the upper bound
    // EXCLUSIVE via `<`); the return is the SIGNED `(int)L` cast (NOT a `<x>u`).
    assert!(
        g.contains("if (all(p >= o) && all(p < hi)) {"),
        "the hit guard must spell the `all(p >= o) && all(p < hi)` reduction:\n{g}"
    );
    assert!(
        g.contains("return (int)L;"),
        "the hit must `return (int)L;` (a SIGNED cast, NOT a `<x>u`):\n{g}"
    );
    assert!(
        !g.contains("return Lu;") && !g.contains("(uint)L"),
        "the hit return must be the signed `(int)L`, never a uint form:\n{g}"
    );
}

#[test]
fn tail_returns_bare_signed_minus_one() {
    let g = generated();
    // The tail is `return -1;` — a BARE signed literal (the committed `OpConstant -1`, NOT a
    // `4294967295u` unsigned).
    assert!(
        g.contains("return -1;"),
        "the tail must spell `return -1;` (a BARE signed literal):\n{g}"
    );
    assert!(
        !g.contains("4294967295u") && !g.contains("return -1u;"),
        "the outside sentinel must be the signed `-1`, never a uint:\n{g}"
    );
}

#[test]
fn full_span_brace_matched_golden() {
    let g = generated();
    // The WHOLE span, brace-matched — the canonical generated text (the committed L1222-1234 with
    // the two in-body comments stripped). A single golden so a structural drift (a missing
    // statement, a reordered line, a stray temp) fails loudly.
    const GOLDEN: &str = "    [unroll]
    for (uint L = 0u; L < BRICK_LEVELS; ++L) {
        if (L >= pc.brick_levels) {
            break;
        }
        float3 o = m2_levels[L].origin_brick_world.xyz;
        float bw = m2_levels[L].origin_brick_world.w;
        float3 hi = o + m2_levels[L].dims_atlas_dim.xyz * bw;
        if (all(p >= o) && all(p < hi)) {
            return (int)L;
        }
    }
    return -1;
";
    assert_eq!(g, GOLDEN, "the generated select_level span must match the brace-matched golden");
}
