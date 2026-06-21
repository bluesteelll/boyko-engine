//! GUI P5b — the measure→layout CPU-boundary seam (Decision T5-B, host-driven).
//!
//! P5b's text measure is HOST-DRIVEN: `boyko_ui` ships `ui_text_measure_system` and the
//! host registers it `.before(ui_layout_discovery)` (the same way it orders the layout
//! pair) — there is no in-crate App schedule binding the order (matching P5a Decision
//! T5-B). The reviewer asked that the seam be PROVEN at the CPU boundary regardless, so
//! these tests assert the two halves a correct ordering relies on:
//!
//! 1. `measure_one`'s reported extent equals the SHAPED run (same `shape_into` core the
//!    emitter uses) — so the size fed into `ContentSize` is the size the emitter lays
//!    down (no measure/emit drift).
//! 2. An `Auto`-sized leaf node fed that `ContentSize` HUGS it through the layout's leaf
//!    intrinsic-size fallback — so once the host writes `ContentSize` before discovery,
//!    the relayout sizes the node to the text.
//!
//! These do not need a scheduler: the measure is a pure function and the layout's
//! `ContentSize` read is exercised directly via the shared `Ui` harness.

mod common;

use common::{approx, NodeSpec, Ui};

use boyko_fontbake::atlas::AtlasImage;
use boyko_ui::components::{ComputedRect, ContentSize, UiLayout};
use boyko_ui::text::{
    measure_one, shape_into, AtlasKind, AtlasMeta, BakedFont, FontId, FontTable, GlyphMetrics,
    TextAlign, UiText,
};
use boyko_ui::units::{LayoutType, Unit};

/// A font with a known, simple metric so the shaped extent is hand-computable. Every
/// printable ASCII codepoint maps to one visible glyph of `advance_em = 0.5` and a
/// `plane` of `[0, 0, 0.5, 0.7]` (non-degenerate ⇒ emitted), line metrics
/// `ascender 0.8 / descender -0.2 / gap 0.0` ⇒ `line_height = 1.0 * size_px`.
fn known_font() -> BakedFont {
    use boyko_fontbake::atlas::MappedCodepoint;
    let visible = GlyphMetrics {
        advance_em: 0.5,
        plane: [0.0, 0.0, 0.5, 0.7],
        atlas: [0.0, 1.0, 1.0, 0.0],
    };
    let mut glyphs = vec![GlyphMetrics { advance_em: 0.0, plane: [0.0; 4], atlas: [0.0; 4] }];
    let mut cmap = Vec::new();
    for (slot, cp) in (1u16..).zip(0x20u32..0x7F) {
        glyphs.push(visible);
        cmap.push(MappedCodepoint { codepoint: cp, slot });
    }
    BakedFont {
        meta: AtlasMeta {
            distance_range_texels: 6.0,
            pixels_per_em: 48.0,
            atlas_w: 1,
            atlas_h: 1,
            ascender_em: 0.8,
            descender_em: -0.2,
            line_gap_em: 0.0,
            kind: AtlasKind::Mtsdf,
        },
        glyphs,
        cmap,
        kern: Vec::new(),
        atlas: AtlasImage { width: 1, height: 1, pixels: vec![0u8; 4] },
    }
}

fn font_table() -> (FontTable, FontId) {
    let mut t = FontTable::new();
    let id = t.load(&known_font());
    (t, id)
}

/// `measure_one`'s extent equals the shaped run extent (no measure/emit drift): the
/// measure shapes through the SAME `shape_into` and reports its returned extent, so a
/// re-shape that counts glyphs and tracks the run width must agree.
#[test]
fn measure_one_matches_the_shaped_run() {
    let (fonts, id) = font_table();
    let text = UiText { color: 0xFFFF_FFFF, size_px: 20.0, font: id, align: TextAlign::Left, _pad: 0 };
    let body = "Hello";
    let rect = ComputedRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; // unwrapped (single line)

    // Re-shape the run independently, summing what the emitter would lay down.
    let entry = fonts.entry(id).expect("loaded font");
    let mut emitted = 0usize;
    let extent = shape_into(body, entry, text.size_px, 0.0, TextAlign::Left, |_g| {
        emitted += 1;
    });

    let measured = measure_one(&text, body, &rect, &fonts);
    approx(measured.width, extent.width, "measure width == shaped run width");
    approx(measured.height, extent.height, "measure height == shaped run height");
    assert_eq!(emitted, 5, "all five glyphs shaped (no whitespace ⇒ single line)");

    // Hand-check: 5 glyphs × advance 0.5 em × 20 px = 50 px wide; one line × 1.0 em ×
    // 20 px = 20 px tall (no kerning in this font).
    approx(measured.width, 50.0, "5 * 0.5em * 20px");
    approx(measured.height, 20.0, "one line, line_height 1.0em * 20px");
}

/// An empty / whitespace-only content measures to nothing (an empty label hugs to zero),
/// so the relayout does not reserve space for an absent value.
#[test]
fn measure_one_empty_content_is_zero() {
    let (fonts, id) = font_table();
    let text = UiText { color: 0xFFFF_FFFF, size_px: 16.0, font: id, align: TextAlign::Left, _pad: 0 };
    let rect = ComputedRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    let measured = measure_one(&text, "", &rect, &fonts);
    approx(measured.width, 0.0, "empty content width");
    approx(measured.height, 0.0, "empty content height");
}

/// An `Auto`×`Auto` leaf node fed the measured `ContentSize` HUGS it through the
/// layout's leaf intrinsic-size fallback (`relative_count == 0` ⇒ use `ContentSize`).
/// This is the downstream half of the measure→layout seam: once the host writes
/// `ContentSize` before discovery, the relayout sizes the node to the text. A Column
/// leaf takes `(width=cross=cw, height=main=ch)`.
#[test]
fn auto_leaf_hugs_the_measured_content_size() {
    let (fonts, id) = font_table();
    let text = UiText { color: 0xFFFF_FFFF, size_px: 20.0, font: id, align: TextAlign::Left, _pad: 0 };
    let body = "Hello"; // 50 × 20 px measured above
    let content_rect = ComputedRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    let measured = measure_one(&text, body, &content_rect, &fonts);

    // A fixed root with one Auto×Auto Column leaf carrying the measured ContentSize.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(UiLayout {
        layout_type: LayoutType::Column,
        width: Unit::Px(400.0),
        height: Unit::Px(300.0),
        ..UiLayout::default()
    });
    let leaf = ui.spawn(
        NodeSpec::new(UiLayout {
            layout_type: LayoutType::Column,
            width: Unit::Auto,
            height: Unit::Auto,
            ..UiLayout::default()
        })
        .with_content(ContentSize { width: measured.width, height: measured.height }),
        Some(root),
    );
    ui.run();

    let r = ui.rect(leaf);
    approx(r.w, measured.width, "Auto leaf width hugs ContentSize.width");
    approx(r.h, measured.height, "Auto leaf height hugs ContentSize.height");
}
