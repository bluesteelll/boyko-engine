//! GUI P5b — glyph LAYOUT correctness gate (the plan's G-T5.1–.4 CPU half).
//!
//! The shipped `text_measure` / `text_emit_zero_alloc` suites prove pen ADVANCE and the
//! measure→`ContentSize` seam, but neither exercises KERNING (the test fonts carry an
//! empty kern table), WORD-WRAP at whitespace, or MULTILINE BASELINE advance. This file
//! drives the SAME `shape_into` core the emitter + measure use and asserts each of those
//! four layout properties against hand-computed pen positions, so a regression in any one
//! arm of the shaper is caught in isolation:
//!
//! * **G-T5.1 pen positions** — N glyphs at `advance_em` step to monotone, exact pens.
//! * **G-T5.2 kerning tighter** — a kern pair pulls the second glyph LEFT (negative
//!   adjust) vs the same run with no kern pair; the delta equals the baked adjustment.
//! * **G-T5.3 word-wrap at whitespace** — a run that overflows `wrap_width` breaks at the
//!   last whitespace (the second word starts a new line, the space is consumed).
//! * **G-T5.4 multiline baseline** — each wrapped line's glyphs sit `line_height` lower;
//!   the run extent height == line_count × line_height.
//!
//! These are pure-function asserts on `shape_into` (no GPU, no scheduler) — the layout
//! geometry the GPU golden then renders, isolated from the device.

use boyko_fontbake::atlas::{AtlasImage, KernPair, MappedCodepoint};
use boyko_ui::text::{
    shape_into, AtlasKind, AtlasMeta, BakedFont, FontEntry, GlyphMetrics, ShapedGlyph, TextAlign,
};

/// Metrics shared by the layout fixtures: every printable ASCII codepoint is one visible
/// glyph of `advance_em = 0.5`, a non-degenerate plane `[0,0,0.5,0.7]` (so it emits a
/// quad), `pixels_per_em = 50.0` (so kerning arithmetic is round), and
/// `ascender 0.8 / descender -0.2 / gap 0.0` ⇒ `line_height = 1.0 em`.
fn base_meta() -> AtlasMeta {
    AtlasMeta {
        distance_range_texels: 6.0,
        pixels_per_em: 50.0,
        atlas_w: 1,
        atlas_h: 1,
        ascender_em: 0.8,
        descender_em: -0.2,
        line_gap_em: 0.0,
        kind: AtlasKind::Mtsdf,
    }
}

/// A printable-ASCII font with an optional kern table. Slot 0 is `.notdef`; slots 1..
/// map `0x20..0x7F` in sorted order (so `' '` is slot 1 with a zero-area plane — it
/// advances but emits no quad — and the visible glyphs follow).
fn ascii_font(kern: Vec<KernPair>) -> BakedFont {
    let visible = GlyphMetrics { advance_em: 0.5, plane: [0.0, 0.0, 0.5, 0.7], atlas: [0.0, 1.0, 1.0, 0.0] };
    let space = GlyphMetrics { advance_em: 0.5, plane: [0.0; 4], atlas: [0.0; 4] }; // zero-area ⇒ no quad
    let mut glyphs = vec![GlyphMetrics { advance_em: 0.0, plane: [0.0; 4], atlas: [0.0; 4] }];
    let mut cmap = Vec::new();
    for (slot, cp) in (1u16..).zip(0x20u32..0x7F) {
        glyphs.push(if cp == 0x20 { space } else { visible });
        cmap.push(MappedCodepoint { codepoint: cp, slot });
    }
    BakedFont {
        meta: base_meta(),
        glyphs,
        cmap,
        kern,
        atlas: AtlasImage { width: 1, height: 1, pixels: vec![0u8; 4] },
    }
}

/// Shapes `content` at `size_px` with `wrap_width`, collecting every emitted quad's
/// `(x, y)` top-left into a `Vec` (the geometry the emitter would lay down).
fn shape_positions(font: &FontEntry, content: &str, size_px: f32, wrap_width: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    shape_into(content, font, size_px, wrap_width, TextAlign::Left, |g: ShapedGlyph| {
        out.push([g.rect[0], g.rect[1]]);
    });
    out
}

/// The dense glyph slot a codepoint maps to in [`ascii_font`] (slot 1 == `' '`).
fn slot_of(cp: char) -> u16 {
    (cp as u32 - 0x20 + 1) as u16
}

const EPS: f32 = 1e-3;

/// G-T5.1 — pen positions are exact + monotone: `N` glyphs at `advance_em = 0.5` and
/// `size_px = 20` step by exactly `10 px`, starting at the content origin x = 0.
#[test]
fn shape_pens_advance_by_exact_glyph_advance() {
    let font = ascii_font(Vec::new());
    let entry = FontEntry::from_baked(&font);
    let pens = shape_positions(&entry, "AAAA", 20.0, 0.0);

    assert_eq!(pens.len(), 4, "four visible glyphs");
    let step = 0.5 * 20.0; // advance_em * size_px
    for (i, p) in pens.iter().enumerate() {
        let expected = i as f32 * step;
        assert!(
            (p[0] - expected).abs() < EPS,
            "glyph {i} pen-x must be {expected} (advance {step}/glyph), got {}",
            p[0]
        );
    }
    for w in pens.windows(2) {
        assert!(w[1][0] > w[0][0], "pen advances strictly rightward (monotone)");
    }
}

/// G-T5.2 — a kern pair pulls the second glyph LEFT: with a negative `adjust` on the
/// `(A,V)` pair, the `V` in `"AV"` sits `|adjust| / pixels_per_em * size_px` px tighter
/// than the unkerned run. The delta equals the baked adjustment scaled to px — proving
/// the kern table is consulted (not ignored) with the correct sign + magnitude.
#[test]
fn shape_kerning_pulls_the_pair_tighter() {
    let size = 20.0f32;
    let ppem = 50.0f32; // base_meta().pixels_per_em
    let adjust: i16 = -100; // font units; negative ⇒ tighter

    let key = ((slot_of('A') as u32) << 16) | (slot_of('V') as u32);
    let kerned = ascii_font(vec![KernPair { key, adjust }]);
    let plain = ascii_font(Vec::new());
    let kerned_e = FontEntry::from_baked(&kerned);
    let plain_e = FontEntry::from_baked(&plain);

    let kp = shape_positions(&kerned_e, "AV", size, 0.0);
    let pp = shape_positions(&plain_e, "AV", size, 0.0);

    assert_eq!(kp.len(), 2, "two glyphs (kerned)");
    assert_eq!(pp.len(), 2, "two glyphs (plain)");
    // The first glyph is unaffected by kerning (no left neighbor).
    assert!((kp[0][0] - pp[0][0]).abs() < EPS, "the first glyph is unkerned");

    // shape.rs applies `adjust as f32 * (1/pixels_per_em) * size_px` BEFORE the pen
    // reaches the second glyph, so the kerned V is exactly that much to the LEFT.
    let expected_delta = adjust as f32 * (1.0 / ppem) * size; // -40 px? -100/50*20 = -40
    let actual_delta = kp[1][0] - pp[1][0];
    assert!(
        (actual_delta - expected_delta).abs() < EPS,
        "kerning must shift the V by {expected_delta} px (baked adjust scaled), got {actual_delta}"
    );
    assert!(actual_delta < 0.0, "a negative kern adjust pulls the pair TIGHTER (left)");
}

/// G-T5.3 — word-wrap at whitespace: a two-word run wider than `wrap_width` breaks at the
/// space, so the second word's first glyph starts a NEW line at x = 0 (the space is
/// consumed, not re-emitted), and the run spans two lines.
#[test]
fn shape_wraps_at_whitespace_when_a_word_overflows() {
    let font = ascii_font(Vec::new());
    let entry = FontEntry::from_baked(&font);
    let size = 20.0f32;
    let step = 0.5 * size; // 10 px per glyph

    // "AAA BBB": each word is 3 glyphs (30 px). A wrap_width of 45 px holds the first
    // word (30) + the space (10) = 40, but not the second word — so it wraps.
    let positions = shape_positions(&entry, "AAA BBB", size, 45.0);

    assert_eq!(positions.len(), 6, "six visible glyphs (the space emits no quad)");
    let (line0, line1) = positions.split_at(3);
    // First line: AAA at x = 0,10,20 on the first baseline.
    let line0_y = line0[0][1];
    for (i, p) in line0.iter().enumerate() {
        assert!((p[0] - i as f32 * step).abs() < EPS, "word 1 glyph {i} pen-x");
        assert!((p[1] - line0_y).abs() < EPS, "word 1 stays on line 0");
    }
    // Second line: BBB restarts at x = 0 on a LOWER baseline (the space was consumed).
    let line1_y = line1[0][1];
    assert!(line1_y > line0_y, "the wrapped word drops to a new line");
    for (i, p) in line1.iter().enumerate() {
        assert!(
            (p[0] - i as f32 * step).abs() < EPS,
            "wrapped word glyph {i} restarts at the left margin (x = {})",
            i as f32 * step
        );
    }
}

/// G-T5.4 — multiline baseline advance: an explicit `\n` puts the second line exactly
/// `line_height = (ascender − descender + gap) * size_px` below the first, and the
/// shaped extent height == line_count × line_height.
#[test]
fn shape_multiline_baseline_advances_by_line_height() {
    let font = ascii_font(Vec::new());
    let entry = FontEntry::from_baked(&font);
    let size = 20.0f32;
    let line_height = (0.8 - (-0.2) + 0.0) * size; // 1.0 em * 20 px = 20 px

    let mut glyphs: Vec<ShapedGlyph> = Vec::new();
    let extent = shape_into("AB\nCD", &entry, size, 0.0, TextAlign::Left, |g| glyphs.push(g));

    assert_eq!(glyphs.len(), 4, "four visible glyphs across two lines");
    let line0_y = glyphs[0].rect[1];
    let line1_y = glyphs[2].rect[1]; // first glyph of the second line
    assert!(
        (line1_y - line0_y - line_height).abs() < EPS,
        "the second line's baseline sits exactly one line_height ({line_height} px) lower: \
         got Δ = {}",
        line1_y - line0_y
    );
    assert!(
        (extent.height - 2.0 * line_height).abs() < EPS,
        "two-line extent height == 2 × line_height ({}), got {}",
        2.0 * line_height,
        extent.height
    );
}
