//! Increment 4a — the `m2_regula_falsi` GENERATOR structure guard (`feature = "emit"`).
//!
//! The cmp-`.spv` (in `boyko_rhi_vulkan`) is the AUTHORITATIVE byte-identity oracle; the
//! `m2_regula_falsi_matches_edsl_emit` text-sync test pins the committed shader to the
//! generator output. THIS test is a finer-grained generation-time guard on the emitted
//! STRUCTURE — the load-bearing shapes the design calls out:
//!
//! - The four carried params {lo, hi, f_lo, f_hi} are SUPPRESSED-DECL signature params:
//!   the body must NOT emit a `float lo = ` / `float hi = ` redecl (a spurious decl would
//!   diverge the text AND fork the `.spv`). It MUST emit the in-loop assigns `hi = mid;` /
//!   `lo = mid;` (the bracket update) — proving the suppressed-decl VARS entries resolve.
//! - `mid` is a TRUE local: `float mid = lo;` (a recorded DeclVar).
//! - `denom` / `f_mid` are NAMED `float` locals (`float denom = ` / `float f_mid = `), NOT
//!   anonymous `tN` temps (the design's source-found correction).
//! - The runtime `[loop]` header spells the BOUND SYMBOL `M2_MARMITT_ITERS`, NOT `8u`.
//! - The cubic is a CALL site `m2_cubic_eval(c, mid)`, not an inlined body.
//!
//! Gated on `feature = "emit"` (the generator surface is `#[cfg(feature = "emit")]`).

#![cfg(feature = "emit")]

fn generated() -> String {
    boyko_shaderdsl::emit::emit_hlsl_m2_regula_falsi().replace("\r\n", "\n")
}

#[test]
fn suppressed_decl_params_have_no_redecl() {
    let g = generated();
    // The four carried params are signature parameters — NO local redeclaration.
    assert!(
        !g.contains("float lo = "),
        "regula-falsi must NOT redeclare `lo` (it is a signature param — suppressed-decl):\n{g}"
    );
    assert!(
        !g.contains("float hi = "),
        "regula-falsi must NOT redeclare `hi` (it is a signature param — suppressed-decl):\n{g}"
    );
    assert!(
        !g.contains("float f_lo = "),
        "regula-falsi must NOT redeclare `f_lo` (signature param — suppressed-decl):\n{g}"
    );
    assert!(
        !g.contains("float f_hi = "),
        "regula-falsi must NOT redeclare `f_hi` (signature param — suppressed-decl):\n{g}"
    );
}

#[test]
fn true_local_mid_is_declared() {
    let g = generated();
    // `mid` is a TRUE local (not a param) -> `float mid = lo;`.
    assert!(
        g.contains("float mid = lo;"),
        "regula-falsi must declare `float mid = lo;` (the one true local):\n{g}"
    );
}

#[test]
fn named_float_locals_denom_and_f_mid() {
    let g = generated();
    // `denom` / `f_mid` are NAMED `float` locals, NOT anonymous `tN` temps.
    assert!(
        g.contains("float denom = "),
        "regula-falsi must declare a NAMED `float denom = ...;` local (not `tN`):\n{g}"
    );
    assert!(
        g.contains("float f_mid = "),
        "regula-falsi must declare a NAMED `float f_mid = ...;` local (not `tN`):\n{g}"
    );
    // No anonymous `tN` temps at all (every materialization is named here).
    assert!(
        !g.contains("float t0 ="),
        "regula-falsi must use NAMED locals, not anonymous `t0` temps:\n{g}"
    );
}

#[test]
fn in_loop_bracket_assigns_present() {
    let g = generated();
    // The bracket-update assigns prove the suppressed-decl VARS entries resolve to the
    // param names (a missing VARS entry would panic or mis-spell).
    assert!(
        g.contains("hi = mid;"),
        "regula-falsi must emit the bracket assign `hi = mid;`:\n{g}"
    );
    assert!(
        g.contains("f_hi = f_mid;"),
        "regula-falsi must emit the bracket assign `f_hi = f_mid;`:\n{g}"
    );
    assert!(
        g.contains("lo = mid;"),
        "regula-falsi must emit the bracket assign `lo = mid;`:\n{g}"
    );
    assert!(
        g.contains("f_lo = f_mid;"),
        "regula-falsi must emit the bracket assign `f_lo = f_mid;`:\n{g}"
    );
}

#[test]
fn runtime_loop_header_spells_bound_symbol() {
    let g = generated();
    // The `[loop]` attribute + the BOUND SYMBOL in the header (NOT a `8u` literal) — the
    // difference from `[unroll]` that makes DXC emit a genuine OpLoop.
    assert!(g.contains("[loop]"), "regula-falsi must carry the `[loop]` attribute:\n{g}");
    assert!(
        g.contains("for (uint i = 0u; i < M2_MARMITT_ITERS; ++i)"),
        "the loop header must spell the BOUND SYMBOL `M2_MARMITT_ITERS`, not `8u`:\n{g}"
    );
    assert!(
        !g.contains("i < 8u"),
        "the loop header must spell the symbol, not the literal `8u`:\n{g}"
    );
}

#[test]
fn cubic_is_a_call_site_not_inlined() {
    let g = generated();
    // The cubic is spelled as a CALL (the leaf body is generated separately).
    assert!(
        g.contains("m2_cubic_eval(c, mid)"),
        "the cubic must be a call site `m2_cubic_eval(c, mid)`:\n{g}"
    );
}

#[test]
fn ternary_arms_are_wrapped() {
    let g = generated();
    // The committed ternary form wraps BOTH arms (the SelectParen printer).
    assert!(
        g.contains("(abs(denom) > 1.0e-30) ? (lo - f_lo * (hi - lo) / denom) : (0.5 * (lo + hi))"),
        "the degenerate-bracket ternary must wrap both arms (the committed form):\n{g}"
    );
}
