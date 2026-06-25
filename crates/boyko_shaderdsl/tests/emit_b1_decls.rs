//! Increment 4d — the B1 `bool` PREAMBLE-DECL generator STRUCTURE guard (`feature = "emit"`).
//!
//! The cmp-`.spv` (in `boyko_rhi_vulkan`) is the AUTHORITATIVE byte-identity oracle; the
//! whole-`.spv` binary compare after the inline splice is the binding gate. THIS test is a
//! finer-grained generation-time guard on the emitted decl-line TEXT — the load-bearing shape
//! the design calls out:
//!
//! - The DECL-SITE type token is the LITERAL `bool` (NOT `bool1` or a vector form) — a DIFFERENT
//!   print path from the proven bool-RETURN literal.
//! - `hit` inits to `false`; `exhausted` inits to `true` (the BUG-B1-HOLE-3 flag).
//! - Each emitter returns ONLY its one decl line (the float decls + comments between the two
//!   non-contiguous decls stay hand-written — framing (b)).
//!
//! Gated on `feature = "emit"` (the generator surface is `#[cfg(feature = "emit")]`).

#![cfg(feature = "emit")]

#[test]
fn b1_decl_hit_emits_bool_false() {
    let g = boyko_shaderdsl::emit::emit_hlsl_b1_decl_hit().replace("\r\n", "\n");
    // The emitted line, indent-stripped (the depth-1 4-space indent is asserted separately by
    // `b1_decls_are_indented_for_main_body`). The token is `bool` (lowercase scalar, no vector
    // suffix), the init is `false`.
    assert_eq!(
        g.trim(),
        "bool hit = false;",
        "the `hit` decl must spell `bool hit = false;` (scalar `bool`, init false):\n{g:?}"
    );
    // Defend the scalar token against a vector-form regression (`bool1`).
    assert!(
        !g.contains("bool1"),
        "the decl type token must be scalar `bool`, never a vector form `bool1`:\n{g}"
    );
}

#[test]
fn b1_decl_exhausted_emits_bool_true() {
    let g = boyko_shaderdsl::emit::emit_hlsl_b1_decl_exhausted().replace("\r\n", "\n");
    assert_eq!(
        g.trim(),
        "bool exhausted = true;",
        "the `exhausted` decl must spell `bool exhausted = true;` (scalar `bool`, init true):\n{g:?}"
    );
    assert!(
        !g.contains("bool1"),
        "the decl type token must be scalar `bool`, never a vector form `bool1`:\n{g}"
    );
}

#[test]
fn b1_decls_are_indented_for_main_body() {
    // Both decls print at depth 1 (4-space indent — inside `main`, matching the committed
    // L1316/L1327). The splice reproduces this exact indent.
    let hit = boyko_shaderdsl::emit::emit_hlsl_b1_decl_hit().replace("\r\n", "\n");
    let exhausted = boyko_shaderdsl::emit::emit_hlsl_b1_decl_exhausted().replace("\r\n", "\n");
    assert_eq!(
        hit, "    bool hit = false;\n",
        "the `hit` decl must be 4-space indented (depth-1, inside `main`)"
    );
    assert_eq!(
        exhausted, "    bool exhausted = true;\n",
        "the `exhausted` decl must be 4-space indented (depth-1, inside `main`)"
    );
}
