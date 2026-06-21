//! GUI P5b text components: [`UiText`] (style), [`FontId`] (dense font handle), and
//! [`TextAlign`] (line alignment). AUTHOR-OWNED, OPT-IN.
//!
//! The CONTENT is the existing [`UiTextBuffer`](crate::binding::UiTextBuffer) (the P4
//! tick-bearing sink); `UiText` carries STYLE only, so a content-only change bumps
//! only `UiTextBuffer`'s tick and a style-only change bumps only `UiText`'s tick
//! (independent churn columns — Principle 2 hot/cold split). A node with `UiText` + a
//! non-empty `UiTextBuffer` renders text; absent `UiText` ⇒ no text (rect-only, the
//! P5a path unchanged).

use boyko_macros::Component;

/// Line alignment within the node's content box (GUI P5b). `#[repr(u8)]` POD.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// Lines hug the left content edge (the default).
    #[default]
    Left = 0,
    /// Lines are centered horizontally in the content box.
    Center = 1,
    /// Lines hug the right content edge.
    Right = 2,
}

/// A dense font handle — a `u16` index into the [`FontTable`](super::font::FontTable)
/// resource (Principle 1: a dense index, NOT a string or a `HashMap` key). `0` is the
/// first loaded font. P5b binds a SINGLE resident font/atlas (Decision T4-E); the
/// index space is reserved for the multi-font seam.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontId(pub u16);

/// Text style for a node (GUI P5b). 12 B (`u32 + f32 + u16 + u8 + u8`, align 4, no
/// tail pad — const-asserted), `#[repr(C)]`, POD `Copy`. AUTHOR-OWNED, OPT-IN. Pairs
/// with the content [`UiTextBuffer`](crate::binding::UiTextBuffer) — a node with both
/// (and a non-empty buffer) renders text.
///
/// Colors are authored STRAIGHT RGBA8 (`byte0=R .. byte3=A`); the emitter / pack
/// premultiplies them into the GPU record (the P5a premultiplied convention).
/// `size_px` is the logical-px em size; the host folds `scale_factor` at emit so the
/// shader works in physical px and the `screenPxRange` AA is one device pixel.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiText {
    /// Foreground color, STRAIGHT RGBA8 (`byte0=R .. byte3=A`); premultiplied at emit.
    pub color: u32,
    /// Logical-px font size (em). `scale_factor` is folded at emit.
    pub size_px: f32,
    /// Dense font handle (NOT a string / `HashMap` key).
    pub font: FontId,
    /// Line alignment within the content box.
    pub align: TextAlign,
    /// Pad to a 16 B, no-tail-pad `#[repr(C)]` record.
    pub _pad: u8,
}

const _: () = assert!(size_of::<UiText>() == 12);
const _: () = assert!(align_of::<UiText>() == 4);

impl Default for UiText {
    /// Opaque white, 16 px, font 0, left-aligned — a sensible visible default once a
    /// node opts in to `UiText` (the author still supplies the content buffer).
    #[inline]
    fn default() -> Self {
        UiText {
            color: 0xFFFF_FFFF,
            size_px: 16.0,
            font: FontId(0),
            align: TextAlign::Left,
            _pad: 0,
        }
    }
}
