//! GUI P5b — alloc-free Latin shaping: advance / kerning / line-wrap / baseline.
//!
//! [`shape_into`] is the SINGLE shaping core shared by the glyph EMITTER
//! ([`emit`](super::emit)) and the text MEASURE system ([`measure`](super::measure)),
//! so the pen positions a measure reports are byte-identical to the ones the emitter
//! lays down. It walks the UTF-8 content once, resolving each `char` to a glyph slot,
//! applying kerning, wrapping at whitespace when a word would overflow the content
//! width, and feeding each visible glyph's quad to a caller-supplied sink — no heap
//! allocation (the source bytes are bounded by `UiTextBuffer::CAP`, Decision T5-A; the
//! sink reuses the host scratch).
//!
//! Scope (Decision: Latin-first): left-to-right, no complex shaping / BiDi /
//! ligatures / CJK — the documented out-of-scope seam.

use boyko_fontbake::atlas::GlyphMetrics;

use super::components::TextAlign;
use super::font::FontEntry;

/// One shaped glyph quad in LOGICAL px, baseline-resolved, ready to fold into the
/// instance stream (the emitter premultiplies the color + folds `scale_factor`; the
/// measure ignores the geometry and reads only the run extent).
///
/// `rect` is `(x, y, w, h)` in logical px relative to the node's content origin
/// `(0, 0)` at its top-left; `uv` is the glyph's NORMALIZED atlas UV rect
/// `(left, top, right, bottom)` in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    /// Glyph quad `(x, y, w, h)`, logical px, content-origin-relative.
    pub rect: [f32; 4],
    /// Normalized atlas UV rect `(left, top, right, bottom)` in `[0, 1]`.
    pub uv: [f32; 4],
}

/// The result extent of a shaped run, in logical px (the measure output).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShapedExtent {
    /// The widest line's advance width, logical px.
    pub width: f32,
    /// Total height (line count × line height), logical px.
    pub height: f32,
}

/// Shapes `content` at `size_px` (logical em) into `font`, wrapping at whitespace when
/// `wrap_width > 0` and a word would overflow it, calling `sink` once per VISIBLE
/// glyph quad (content-origin-relative, logical px), and returning the run extent.
///
/// `align` shifts each line horizontally within `wrap_width` (when `> 0`). The first
/// baseline sits at `ascender_em * size_px` from the content top; each subsequent line
/// is `line_height` lower. Whitespace glyphs advance the pen but emit no quad (a space
/// has no atlas entry). Alloc-free: a single pass over `content`, no buffering — the
/// caller's `sink` is the only place quads land.
///
/// Returns [`ShapedExtent::default`] (zero) when `content` is empty.
pub fn shape_into(
    content: &str,
    font: &FontEntry,
    size_px: f32,
    wrap_width: f32,
    align: TextAlign,
    mut sink: impl FnMut(ShapedGlyph),
) -> ShapedExtent {
    let meta = font.meta();
    debug_assert!(size_px > 0.0, "invariant: text size_px is positive");
    if content.is_empty() || meta.pixels_per_em <= 0.0 {
        return ShapedExtent::default();
    }

    let aw = meta.atlas_w as f32;
    let ah = meta.atlas_h as f32;
    let inv_aw = if aw > 0.0 { 1.0 / aw } else { 0.0 };
    let inv_ah = if ah > 0.0 { 1.0 / ah } else { 0.0 };
    // Line height (em): ascender − descender + line gap (descender is negative em).
    let line_height = (meta.ascender_em - meta.descender_em + meta.line_gap_em) * size_px;
    let baseline0 = meta.ascender_em * size_px;
    let inv_upm = 1.0 / meta.pixels_per_em; // kerning adjust is font units / px-per-em

    // Two-pass per line would buffer; instead we lay out each line into a small
    // bounded staging window keyed by byte ranges. To stay alloc-free AND support
    // alignment, we shape line-by-line over byte spans of `content` (the wrap split
    // points are whitespace, so each line is a sub-&str), emitting per line.
    let mut max_width = 0.0f32;
    let mut line_index = 0usize;
    let mut consumed = 0usize; // byte offset into `content`

    while consumed < content.len() {
        let rest = &content[consumed..];
        let (line, next_consumed, hard_break) =
            next_line(rest, font, size_px, wrap_width, inv_upm);
        // Advance width of this line (for alignment + max width).
        let line_w = line_advance(line, font, size_px, inv_upm);
        max_width = max_width.max(line_w);
        let baseline_y = baseline0 + line_index as f32 * line_height;
        let x_offset = align_offset(align, wrap_width, line_w);

        emit_line(
            line,
            font,
            size_px,
            inv_upm,
            x_offset,
            baseline_y,
            inv_aw,
            inv_ah,
            &mut sink,
        );

        consumed += next_consumed;
        // A trailing explicit newline yields one more (empty) line; otherwise stop.
        line_index += 1;
        if next_consumed == 0 {
            break; // defensive: never spin on a zero-width line
        }
        let _ = hard_break;
    }

    let lines = line_index.max(1) as f32;
    ShapedExtent {
        width: max_width,
        height: lines * line_height,
    }
}

/// Picks the next line from `rest`, returning `(line_slice, bytes_consumed,
/// hard_break)`. A line ends at a `\n` (consumed, not emitted) or where the next word
/// would overflow `wrap_width` (`> 0`); the trailing wrap whitespace is consumed.
fn next_line<'a>(
    rest: &'a str,
    font: &FontEntry,
    size_px: f32,
    wrap_width: f32,
    inv_upm: f32,
) -> (&'a str, usize, bool) {
    // Hard break at the first newline.
    if let Some(nl) = rest.find('\n') {
        if wrap_width <= 0.0 {
            return (&rest[..nl], nl + 1, true);
        }
        // Still honor wrap within the pre-newline span.
        let span = &rest[..nl];
        let (line, used) = wrap_span(span, font, size_px, wrap_width, inv_upm);
        if used >= span.len() {
            return (line, nl + 1, true); // whole span fit; consume the newline too
        }
        return (line, used, false); // wrapped before the newline
    }
    if wrap_width <= 0.0 {
        return (rest, rest.len(), false);
    }
    let (line, used) = wrap_span(rest, font, size_px, wrap_width, inv_upm);
    (line, used, false)
}

/// Wraps `span` at the last whitespace that keeps the line within `wrap_width`,
/// returning `(line_slice, bytes_consumed)` (the consumed count skips the trailing
/// wrap whitespace). A single word longer than `wrap_width` is NOT split (it
/// overflows — the documented Latin policy).
fn wrap_span<'a>(
    span: &'a str,
    font: &FontEntry,
    size_px: f32,
    wrap_width: f32,
    inv_upm: f32,
) -> (&'a str, usize) {
    let mut pen = 0.0f32;
    let mut prev_slot: Option<u16> = None;
    let mut last_ws_byte: Option<usize> = None; // byte index of the last whitespace
    let mut byte = 0usize;
    for ch in span.chars() {
        let slot = font.glyph_slot(ch);
        if let (Some(p), false) = (prev_slot, ch.is_whitespace()) {
            pen += font.kerning(p, slot) as f32 * inv_upm * size_px;
        }
        let adv = glyph_advance(font, slot) * size_px;
        if ch.is_whitespace() {
            last_ws_byte = Some(byte);
        } else if pen + adv > wrap_width && pen > 0.0 {
            // Overflow: break at the last whitespace if any; else take what we have.
            if let Some(ws) = last_ws_byte {
                return (span[..ws].trim_end(), next_word_start(span, ws));
            }
            return (&span[..byte], byte);
        }
        pen += adv;
        prev_slot = Some(slot);
        byte += ch.len_utf8();
    }
    (span, span.len())
}

/// The byte index where the word AFTER the whitespace at `ws` begins (skips the run of
/// whitespace), so the wrap consumes the separator without re-emitting it next line.
fn next_word_start(span: &str, ws: usize) -> usize {
    let mut i = ws;
    for ch in span[ws..].chars() {
        if ch.is_whitespace() {
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    i
}

/// The advance width of a whole line (logical px), including kerning.
fn line_advance(line: &str, font: &FontEntry, size_px: f32, inv_upm: f32) -> f32 {
    let mut pen = 0.0f32;
    let mut prev: Option<u16> = None;
    for ch in line.chars() {
        let slot = font.glyph_slot(ch);
        if let Some(p) = prev {
            pen += font.kerning(p, slot) as f32 * inv_upm * size_px;
        }
        pen += glyph_advance(font, slot) * size_px;
        prev = Some(slot);
    }
    pen
}

/// Emits one line's visible glyph quads to `sink` at `baseline_y` (content-relative,
/// logical px), starting at `x_offset`.
#[allow(clippy::too_many_arguments)]
fn emit_line(
    line: &str,
    font: &FontEntry,
    size_px: f32,
    inv_upm: f32,
    x_offset: f32,
    baseline_y: f32,
    inv_aw: f32,
    inv_ah: f32,
    sink: &mut impl FnMut(ShapedGlyph),
) {
    let mut pen = x_offset;
    let mut prev: Option<u16> = None;
    for ch in line.chars() {
        let slot = font.glyph_slot(ch);
        if let Some(p) = prev {
            pen += font.kerning(p, slot) as f32 * inv_upm * size_px;
        }
        if let Some(g) = font.glyph(slot) {
            // A glyph with a zero-area atlas quad (space) emits no quad.
            if !is_empty_quad(g) {
                let (x0, y0, w, h) = quad_logical(g, pen, baseline_y, size_px);
                let uv = quad_uv(g, inv_aw, inv_ah);
                sink(ShapedGlyph {
                    rect: [x0, y0, w, h],
                    uv,
                });
            }
            pen += g.advance_em * size_px;
        }
        prev = Some(slot);
    }
}

/// The horizontal offset to apply to a line given the alignment + the wrap width.
#[inline]
fn align_offset(align: TextAlign, wrap_width: f32, line_w: f32) -> f32 {
    if wrap_width <= 0.0 {
        return 0.0;
    }
    match align {
        TextAlign::Left => 0.0,
        TextAlign::Center => ((wrap_width - line_w) * 0.5).max(0.0),
        TextAlign::Right => (wrap_width - line_w).max(0.0),
    }
}

/// A glyph's pen advance (em); `0` for a slot with no metrics.
#[inline]
fn glyph_advance(font: &FontEntry, slot: u16) -> f32 {
    font.glyph(slot).map_or(0.0, |g| g.advance_em)
}

/// Whether a glyph's plane quad has zero area (e.g. a space — advance only, no atlas).
#[inline]
fn is_empty_quad(g: &GlyphMetrics) -> bool {
    (g.plane[2] - g.plane[0]).abs() <= f32::EPSILON || (g.plane[3] - g.plane[1]).abs() <= f32::EPSILON
}

/// Converts a glyph's planeBounds (em, baseline-relative, Y-up) to a content-relative
/// screen rect `(x, y, w, h)` in logical px (Y-down) at pen `pen_x` + baseline
/// `baseline_y`. `plane = [left, bottom, right, top]`.
#[inline]
fn quad_logical(g: &GlyphMetrics, pen_x: f32, baseline_y: f32, size_px: f32) -> (f32, f32, f32, f32) {
    let x0 = pen_x + g.plane[0] * size_px;
    let x1 = pen_x + g.plane[2] * size_px;
    // Y-up plane → Y-down screen: top edge = baseline − plane.top; bottom = baseline −
    // plane.bottom.
    let y_top = baseline_y - g.plane[3] * size_px;
    let y_bot = baseline_y - g.plane[1] * size_px;
    (x0, y_top, x1 - x0, y_bot - y_top)
}

/// The glyph's NORMALIZED atlas UV rect `(left, top, right, bottom)` in `[0, 1]`. The
/// baked `atlas = [left, bottom, right, top]` is in TEXELS with Y measured from the
/// image top, so `top` is the smaller texel-Y; the FS lerps `xy..zw` with the 0..1
/// quad corner (Decision T4-B), so we order `(left, top, right, bottom)`.
#[inline]
fn quad_uv(g: &GlyphMetrics, inv_aw: f32, inv_ah: f32) -> [f32; 4] {
    let left = g.atlas[0] * inv_aw;
    let right = g.atlas[2] * inv_aw;
    // `atlas[3]` (top) is the smaller texel-Y (image-top origin) and maps to the quad
    // corner v=0; `atlas[1]` (bottom) is the larger texel-Y → v=1.
    let top = g.atlas[3] * inv_ah;
    let bottom = g.atlas[1] * inv_ah;
    [
        left.clamp(0.0, 1.0),
        top.clamp(0.0, 1.0),
        right.clamp(0.0, 1.0),
        bottom.clamp(0.0, 1.0),
    ]
}
