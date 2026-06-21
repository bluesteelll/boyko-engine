//! GUI P5b — the ECS-resident font table (Principle 0: a `Resource`-owned column,
//! NOT a `HashMap` side store).
//!
//! [`FontTable`] holds one [`FontEntry`] per loaded font (dense, indexed by
//! [`FontId`](super::components::FontId)). Each entry carries the dense
//! [`GlyphMetrics`] table (indexed by a per-font glyph slot), a sorted codepoint→slot
//! map + a sorted kerning table (both binary-searched — never a `HashMap`, mirroring
//! the engine's serialization `LoadEntityMap` sorted-Vec+binary-search precedent), and
//! the [`AtlasMeta`]. It is loaded ONCE at setup from a `.bfont` and never grows
//! in-frame; lookup is `O(log glyphs)` with a `cp < 128` direct-array fast path.
//!
//! The metrics/meta types are the boyko-fontbake POD records (load-time, off every hot
//! path); reusing them keeps a SINGLE source of truth for the baked layout (no
//! parallel copy — Principle 0).

use boyko_fontbake::atlas::{AtlasMeta, BakedFont, GlyphMetrics, KernPair, MappedCodepoint};
use boyko_macros::Resource;

use super::components::FontId;

/// ASCII fast-path size: codepoints `< 128` resolve via a direct array index rather
/// than a binary search (the Latin common case is `O(1)`).
const ASCII_FAST: usize = 128;

/// The `.notdef` glyph slot (always slot 0 in a baked font).
pub const NOTDEF_SLOT: u16 = 0;

/// One loaded font's CPU-side metadata (GUI P5b). Dense glyph metrics + a sorted
/// codepoint map + a sorted kerning table + the atlas metadata — all engine storage,
/// loaded once and immutable thereafter.
pub struct FontEntry {
    /// Dense glyph metrics, indexed by a per-font glyph slot (slot 0 == `.notdef`).
    glyphs: Box<[GlyphMetrics]>,
    /// `codepoint < 128` → glyph slot (`u16::MAX` ⇒ absent, falls back to `.notdef`).
    /// The direct-array Latin fast path.
    ascii: Box<[u16; ASCII_FAST]>,
    /// Sorted-by-codepoint `(codepoint, slot)` for `cp >= 128`; binary-searched.
    cmap: Box<[MappedCodepoint]>,
    /// Sorted-by-key `((left << 16) | right, adjust)` kerning pairs; binary-searched.
    /// May be empty.
    kern: Box<[KernPair]>,
    /// Per-font atlas metadata (`distance_range_texels`, `pixels_per_em`, line
    /// metrics, kind).
    meta: AtlasMeta,
}

impl FontEntry {
    /// Builds a [`FontEntry`] from a loaded [`BakedFont`] (the `.bfont` in-memory
    /// form). The cmap is split into the ASCII fast-path array + the sorted tail; the
    /// kern table is taken as-is (already sorted by the baker). Setup-time alloc only.
    pub fn from_baked(font: &BakedFont) -> Self {
        let mut ascii = Box::new([u16::MAX; ASCII_FAST]);
        // The baker emits a sorted cmap; partition the ASCII prefix into the fast array
        // and keep the rest as the binary-search tail.
        let mut tail: Vec<MappedCodepoint> = Vec::with_capacity(font.cmap.len());
        for &m in &font.cmap {
            if (m.codepoint as usize) < ASCII_FAST {
                ascii[m.codepoint as usize] = m.slot;
            } else {
                tail.push(m);
            }
        }
        debug_assert!(
            tail.windows(2).all(|w| w[0].codepoint < w[1].codepoint),
            "invariant: the cmap tail is strictly sorted by codepoint (binary search)"
        );
        debug_assert!(
            font.kern.windows(2).all(|w| w[0].key <= w[1].key),
            "invariant: the kern table is sorted by packed key (binary search)"
        );
        FontEntry {
            glyphs: font.glyphs.clone().into_boxed_slice(),
            ascii,
            cmap: tail.into_boxed_slice(),
            kern: font.kern.clone().into_boxed_slice(),
            meta: font.meta,
        }
    }

    /// The atlas metadata for this font.
    #[inline]
    pub fn meta(&self) -> &AtlasMeta {
        &self.meta
    }

    /// The number of glyph slots.
    #[inline]
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// The metrics for a glyph slot, or `None` if out of range.
    #[inline]
    pub fn glyph(&self, slot: u16) -> Option<&GlyphMetrics> {
        self.glyphs.get(slot as usize)
    }

    /// Resolves a codepoint to a glyph slot, falling back to `.notdef` (slot 0) for a
    /// missing codepoint. `cp < 128` hits the direct array; otherwise a binary search.
    #[inline]
    pub fn glyph_slot(&self, cp: char) -> u16 {
        let c = cp as u32;
        if (c as usize) < ASCII_FAST {
            let s = self.ascii[c as usize];
            return if s == u16::MAX { NOTDEF_SLOT } else { s };
        }
        match self.cmap.binary_search_by_key(&c, |m| m.codepoint) {
            Ok(i) => self.cmap[i].slot,
            Err(_) => NOTDEF_SLOT,
        }
    }

    /// The kerning adjustment between `left` and `right` glyph slots, in EM units
    /// (the baked `adjust` is font units quantized to `i16`, scaled by
    /// `1 / pixels_per_em`-free here — callers apply it relative to the em size). `0`
    /// when no pair is present.
    #[inline]
    pub fn kerning(&self, left: u16, right: u16) -> i16 {
        if self.kern.is_empty() {
            return 0;
        }
        let key = ((left as u32) << 16) | (right as u32);
        match self.kern.binary_search_by_key(&key, |k| k.key) {
            Ok(i) => self.kern[i].adjust,
            Err(_) => 0,
        }
    }
}

/// The ECS-resident font table (GUI P5b) — a `Resource`-owned dense column of loaded
/// fonts (Principle 0). Loaded once at setup; never grows in-frame.
///
/// P5b binds a SINGLE resident font/atlas on the GPU (Decision T4-E); this table is
/// nonetheless multi-font-capable so the data model is ready for the per-instance-lane
/// / texture-array multi-font seam without a schema change.
#[derive(Resource, Default)]
pub struct FontTable {
    /// Dense fonts, indexed by [`FontId`]`.0`. Setup-time alloc; never grows in-frame.
    fonts: Vec<FontEntry>,
}

impl FontTable {
    /// An empty table (no fonts loaded). Text emit/measure short-circuit until a font
    /// is loaded.
    #[inline]
    pub fn new() -> Self {
        FontTable { fonts: Vec::new() }
    }

    /// Loads a [`BakedFont`] into the table, returning its dense [`FontId`]. Setup-only
    /// (the table never grows in-frame); call once per `.bfont` at world build.
    pub fn load(&mut self, font: &BakedFont) -> FontId {
        let id = FontId(self.fonts.len() as u16);
        self.fonts.push(FontEntry::from_baked(font));
        id
    }

    /// The number of loaded fonts.
    #[inline]
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    /// Whether no font is loaded.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// Borrows the [`FontEntry`] for `font`, or `None` if the index is out of range
    /// (an author referencing an unloaded font — the emitter/measure skip the node).
    #[inline]
    pub fn entry(&self, font: FontId) -> Option<&FontEntry> {
        self.fonts.get(font.0 as usize)
    }
}
