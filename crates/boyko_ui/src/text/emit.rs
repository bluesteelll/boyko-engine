//! GUI P5b — the glyph-quad emitter (Decision T5-A/T5-B).
//!
//! Turns a node's `(UiText, UiTextBuffer, ComputedRect)` into a stream of
//! render-agnostic [`GlyphInstance`] descriptors (a logical-px quad + a normalized
//! atlas UV + the premultiplied-source color + the node's z key + the optional clip),
//! reusing a setup-sized [`TextEmitScratch`] so the per-frame path NEVER reallocates
//! (Decision T5-A: the content is bounded by `UiTextBuffer::CAP`, so the worst-case
//! glyph count is bounded). The host folds each [`GlyphInstance`] into the SAME P5a
//! z-sorted instance stream as the rects (one draw — Decision T4-G), keeping boyko-ui
//! free of any render-crate dependency.
//!
//! Glyph quads are content-relative from [`shape`](super::shape); this module offsets
//! them by the node's `ComputedRect` origin so they land in the node's content box.

use boyko_macros::Resource;

use crate::components::{ComputedClip, ComputedRect, StackIndex};

use super::components::{FontId, UiText};
use super::font::FontTable;
use super::shape::{shape_into, ShapedGlyph};

/// One glyph quad ready for the host to fold into the P5a instance stream. Logical px;
/// the host folds `scale_factor` + premultiplies the color exactly like a rect
/// (`PackInput { text_uv: Some(uv), .. }`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphInstance {
    /// Glyph quad `(x, y, w, h)`, logical px, screen-absolute (content origin folded).
    pub rect: [f32; 4],
    /// Normalized atlas UV rect `(left, top, right, bottom)` in `[0, 1]`.
    pub uv: [f32; 4],
    /// Foreground color, STRAIGHT RGBA8 (`byte0=R .. byte3=A`); premultiplied at pack.
    pub color: u32,
    /// The node's `StackIndex` (painter's-order z key; shared with the rects).
    pub stack: u32,
    /// The node's clip AABB `(x, y, w, h)` in logical px, if it carries one.
    pub clip: Option<[f32; 4]>,
}

/// One node's emit input (the world-agnostic row the [`emit_node`] core consumes), so
/// the emitter is driven by a host-owned `Query` without this crate naming the query
/// types and stays unit-testable.
#[derive(Clone, Copy, Debug)]
pub struct TextNode<'a> {
    /// The style.
    pub text: &'a UiText,
    /// The node's content box (logical px) — the glyph origin + the wrap width.
    pub rect: &'a ComputedRect,
    /// The UTF-8 content.
    pub content: &'a str,
    /// The node's `StackIndex` (0 if none).
    pub stack: u32,
    /// The node's clip, if any.
    pub clip: Option<ComputedClip>,
}

/// Reused glyph-emit scratch (Principle 0 storage — a `Resource`, NOT a side store).
/// Grown ONLY at setup or on a capacity-crossing frame (the same grow-only discipline
/// the P5a rect ring uses); a steady-state frame only `clear()`s + `push`es, so there
/// is zero steady-state allocation (Decision T5-A). The host drains this each frame
/// into its instance stream.
#[derive(Resource, Default)]
pub struct TextEmitScratch {
    /// The emitted glyph quads for all visible text nodes this frame.
    pub glyphs: Vec<GlyphInstance>,
}

impl TextEmitScratch {
    /// An empty scratch.
    #[inline]
    pub fn new() -> Self {
        TextEmitScratch { glyphs: Vec::new() }
    }

    /// Clears the scratch for a fresh frame (capacity persists — no realloc).
    #[inline]
    pub fn clear(&mut self) {
        self.glyphs.clear();
    }
}

/// Emits one text node's glyph quads into `out` (appended; the caller `clear()`s once
/// per frame). Resolves the node's font in `fonts`, shapes the content
/// (advance/kerning/wrap/baseline) wrapping at the node's content width, offsets each
/// content-relative quad by the node's `ComputedRect` origin, and pushes a
/// [`GlyphInstance`] per visible glyph. A node whose font is unloaded, whose size is
/// non-positive, or whose content is empty emits nothing. Alloc-free apart from `out`
/// growth on a capacity-crossing frame (Decision T5-A).
pub fn emit_node(node: &TextNode, fonts: &FontTable, out: &mut Vec<GlyphInstance>) {
    if node.content.is_empty() || node.text.size_px <= 0.0 {
        return;
    }
    let Some(font) = fonts.entry(node.text.font) else {
        debug_assert!(
            false,
            "invariant: UiText references an unloaded FontId (load it into FontTable at setup)"
        );
        return;
    };
    let origin_x = node.rect.x;
    let origin_y = node.rect.y;
    let wrap_width = node.rect.w; // 0 ⇒ no wrap (single line)
    let color = node.text.color;
    let stack = node.stack;
    let clip = node.clip.map(|c| [c.x, c.y, c.w, c.h]);

    shape_into(
        node.content,
        font,
        node.text.size_px,
        wrap_width,
        node.text.align,
        |g: ShapedGlyph| {
            out.push(GlyphInstance {
                rect: [
                    origin_x + g.rect[0],
                    origin_y + g.rect[1],
                    g.rect[2],
                    g.rect[3],
                ],
                uv: g.uv,
                color,
                stack,
                clip,
            });
        },
    );
}

/// Convenience over [`emit_node`] for a host that already split the node fields out of
/// its query (avoids constructing a [`TextNode`] borrow). Mirrors the `emit_node`
/// semantics exactly.
#[allow(clippy::too_many_arguments)]
pub fn emit_glyphs(
    text: &UiText,
    rect: &ComputedRect,
    content: &str,
    stack: StackIndex,
    clip: Option<ComputedClip>,
    fonts: &FontTable,
    out: &mut Vec<GlyphInstance>,
) {
    emit_node(
        &TextNode {
            text,
            rect,
            content,
            stack: stack.0,
            clip,
        },
        fonts,
        out,
    );
}

/// The default font for a node whose [`UiText`] left the font implicit — slot 0 (the
/// single resident P5b font, Decision T4-E). Exposed so a host can resolve the bind.
#[inline]
pub fn default_font() -> FontId {
    FontId(0)
}
