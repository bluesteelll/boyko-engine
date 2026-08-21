//! The UI-rect shaders' eDSL ↔ HLSL SINGLE-SOURCE GUARD (`docs/UI-PLAN-SPRITES.md` rung S1,
//! gate G1-1; architecture D30) — the `particle_edsl_sync` layer-1 idiom applied to
//! `shaders/ui_rect.{vs,fs}.hlsl`.
//!
//! Every `// === GENERATED <name> BEGIN/END ===` span in the two committed UI shaders IS the
//! output of `boyko_shaderdsl::emit`, and is inside the RIGHT function. A hand-edit of a span
//! diverges the GPU math from the `EvalCf` oracle (`boyko_shaderdsl/tests/ui_leaves.rs`)
//! silently — the shader still compiles and still draws plausible rects.
//!
//! The `ui_instance_mirror` span is pinned differently and more strongly: the printer is fed
//! the HOST's own `offset_of!`/`size_of`/flag constants (re-derived here from
//! `boyko_render::ui::instance`, NOT copied), so a `UiInstance` that moves on either side —
//! the Rust struct or the committed HLSL mirror — reds this test. That is S-D10's "no shader
//! ever spells a byte offset that a host `offset_of!` also spells" made checkable: the
//! generator bin (`emit_ui.rs`) spells literals, and THIS test is what pins those literals to
//! the live struct.
//!
//! The `.spv` half (each committed binary is the re-DXC of its own source) lives in
//! `ui_rect_spv_sync.rs` — M1-c is the mutation that says why both exist: a skeleton edit
//! OUTSIDE every sentinel is invisible here and red there.

use std::path::PathBuf;

use boyko_render::ui::instance::{
    UiInstance, FLAG_BORDER_ANY, FLAG_CLIP_PRESENT, FLAG_TEXT, UI_INSTANCE_SIZE,
};
use boyko_shaderdsl::emit::{self, UiInstanceLayout};

// ---- Shared plumbing (the `particle_edsl_sync` idioms) --------------------------------------

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and
/// `.spv` live.
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Reads a committed shader source, LF-normalized so a CRLF checkout does not spuriously
/// mismatch the LF generator output.
fn read_shader(name: &str) -> String {
    let path = shaders_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("invariant: shaders/{name} must exist next to this crate: {e}"))
        .replace("\r\n", "\n")
}

/// Extracts a `<ret> NAME(...) {{ ... }}` function — the signature line through its MATCHING
/// closing brace — out of a shader source. A BRACE COUNTER, not a first-`\n}}` scan (the
/// committed `main` nests `if` blocks). String and comment braces do not occur inside the UI
/// shaders' function bodies, so a raw brace count is exact.
fn extract_fn(src: &str, sig: &str) -> String {
    let start = src
        .find(sig)
        .unwrap_or_else(|| panic!("the committed shader is missing `{sig}`"));
    let after = &src[start..];
    let open = after
        .find('{')
        .expect("a function must have an opening brace");
    let mut depth = 0i32;
    for (i, ch) in after[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return after[..open + i + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces extracting `{sig}` — the function never closed");
}

/// Extracts the text BETWEEN a `// === GENERATED <name> BEGIN ===` line and its matching
/// `END` line (exclusive of both sentinel lines).
fn extract_span(src: &str, name: &str) -> String {
    let begin = format!("// === GENERATED {name} BEGIN ===\n");
    let end = format!("// === GENERATED {name} END ===");
    let b = src
        .find(&begin)
        .unwrap_or_else(|| panic!("the committed shader is missing the `{name}` BEGIN sentinel"));
    let body_start = b + begin.len();
    let e = src[body_start..]
        .find(&end)
        .unwrap_or_else(|| panic!("the committed shader is missing the `{name}` END sentinel"));
    src[body_start..body_start + e].to_string()
}

/// Asserts that `span` — a freshly generated eDSL body — is the body of the committed `sig`
/// function in `file`. The `extract_fn` step is what makes this a REAL pin rather than a
/// substring search: a span pasted into some other function, or into a comment, would satisfy
/// a bare `src.contains(span)` while the function the draw actually calls carried something
/// else.
fn assert_span_is_the_body_of(file: &str, sig: &str, leaf: &str, span: &str) {
    let src = read_shader(file);
    let func = extract_fn(&src, sig);
    assert!(
        func.contains(span),
        "{file} `{leaf}` DRIFTED from boyko_shaderdsl::emit — the committed body no longer \
         matches the generator. Re-run `cargo run -p boyko_shaderdsl --features emit --bin \
         emit_ui`, re-DXC the affected `.spv` with the recipe in the shader's header, and \
         commit both.\n--- expected (eDSL-generated) ---\n{span}\n--- committed function ---\n{func}"
    );
}

/// The HOST-derived `UiInstanceLayout` — every number comes from
/// `boyko_render::ui::instance` itself (`offset_of!`, `size_of`, `trailing_zeros` of the live
/// flag constants), never from a copy. This is the half that makes the mirror pin S-D10's
/// gate: the generator bin spells literals, and this derivation is what they must agree with.
fn host_layout() -> UiInstanceLayout {
    UiInstanceLayout {
        size: UI_INSTANCE_SIZE as u32,
        min_px: core::mem::offset_of!(UiInstance, min_px) as u32,
        size_px: core::mem::offset_of!(UiInstance, size_px) as u32,
        clip: core::mem::offset_of!(UiInstance, clip) as u32,
        corner_radius: core::mem::offset_of!(UiInstance, corner_radius) as u32,
        uv: core::mem::offset_of!(UiInstance, uv) as u32,
        color: core::mem::offset_of!(UiInstance, color) as u32,
        border_color: core::mem::offset_of!(UiInstance, border_color) as u32,
        border_width: core::mem::offset_of!(UiInstance, border_width) as u32,
        flags: core::mem::offset_of!(UiInstance, flags) as u32,
        flag_border_any_bit: FLAG_BORDER_ANY.trailing_zeros(),
        flag_clip_present_bit: FLAG_CLIP_PRESENT.trailing_zeros(),
        flag_text_bit: FLAG_TEXT.trailing_zeros(),
    }
}

// ---- The per-file generator pins (gate G1-1) ------------------------------------------------

/// G1-1, VS half: the vertex stage's one generated span — the `UiInstance` struct mirror —
/// IS the printer's output for the HOST-derived layout.
#[test]
fn ui_rect_vs_matches_edsl_emit() {
    let src = read_shader("ui_rect.vs.hlsl");
    let committed = extract_span(&src, "ui_instance_mirror");
    let fresh = emit::emit_hlsl_ui_instance_mirror(&host_layout()).replace("\r\n", "\n");
    assert_eq!(
        committed, fresh,
        "ui_rect.vs.hlsl's UiInstance mirror DRIFTED from the host struct / the printer. \
         Re-run `cargo run -p boyko_shaderdsl --features emit --bin emit_ui` (and fix the \
         bin's UI_INSTANCE_LAYOUT literals if the Rust struct moved), re-DXC, commit both."
    );
}

/// G1-1, FS half: all eight generated spans — the struct mirror, the flag constants, and the
/// six leaf function bodies — are the printers' output, each inside the right function.
#[test]
fn ui_rect_fs_matches_edsl_emit() {
    let src = read_shader("ui_rect.fs.hlsl");

    // The struct mirror and the flag constants: exact span equality against the HOST-derived
    // layout (the same pin the VS carries — one printer, two splice sites).
    let l = host_layout();
    assert_eq!(
        extract_span(&src, "ui_instance_mirror"),
        emit::emit_hlsl_ui_instance_mirror(&l).replace("\r\n", "\n"),
        "ui_rect.fs.hlsl's UiInstance mirror drifted — re-run emit_ui, re-DXC, commit both"
    );
    assert_eq!(
        extract_span(&src, "ui_flag_consts"),
        emit::emit_hlsl_ui_flag_consts(&l).replace("\r\n", "\n"),
        "ui_rect.fs.hlsl's flag constants drifted — re-run emit_ui, re-DXC, commit both"
    );

    // The six leaves: span-inside-the-right-function containment (the particle layer-1 pin).
    let span = emit::emit_hlsl_ui_unpack_rgba8().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "ui_rect.fs.hlsl",
        "float4 ui_unpack_rgba8(uint c) {",
        "ui_unpack_rgba8",
        &span,
    );
    let span = emit::emit_hlsl_ui_sd_rounded_box().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "ui_rect.fs.hlsl",
        "float ui_sd_rounded_box(float2 p, float2 half_size, float4 r) {",
        "ui_sd_rounded_box",
        &span,
    );
    let span = emit::emit_hlsl_ui_clip_coverage().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "ui_rect.fs.hlsl",
        "float ui_clip_coverage(float2 pos, float4 clip, float fw) {",
        "ui_clip_coverage",
        &span,
    );
    let span = emit::emit_hlsl_ui_median3().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "ui_rect.fs.hlsl",
        "float ui_median3(float r, float g, float b) {",
        "ui_median3",
        &span,
    );
    let span = emit::emit_hlsl_ui_screen_px_range().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "ui_rect.fs.hlsl",
        "float ui_screen_px_range(float2 uv) {",
        "ui_screen_px_range",
        &span,
    );
    let span = emit::emit_hlsl_ui_premultiplied_over().replace("\r\n", "\n");
    assert_span_is_the_body_of(
        "ui_rect.fs.hlsl",
        "float4 ui_premultiplied_over(float4 bc, float border_cov, float4 fill, float inner_cov) {",
        "ui_premultiplied_over",
        &span,
    );
}
