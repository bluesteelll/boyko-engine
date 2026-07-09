//! T0 — the in-house font-extraction surface and its `ttf-parser` backend.
//!
//! The engine depends on the [`FontFace`] / [`OutlineSink`] **traits**, never
//! on the backend. This keeps font parsing isolated behind a wall the same way
//! the RHI is isolated: a future in-house `glyf` parser can implement the same
//! traits with zero call-site churn.
//!
//! The only concrete backend shipped is [`TtfFace`], a thin adapter over
//! `ttf-parser` (owner-locked: zero-dependency, `#![forbid(unsafe_code)]`,
//! zero-alloc, handles `glyf` + CFF/CFF2). Extraction is load-time only and
//! never touches the render hot path.

use ttf_parser::{Face, GlyphId as TtfGlyphId, OutlineBuilder};

/// A glyph identifier within a single face. Newtyped so it cannot be confused
/// with a codepoint or a dense atlas slot.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphId(pub u16);

/// A glyph bounding box in **font units** (not yet em-normalized). Returned by
/// [`FontFace::outline`] so the caller can size the rasterization region.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BBox {
    /// Minimum x (left), font units.
    pub x_min: i16,
    /// Minimum y (bottom), font units.
    pub y_min: i16,
    /// Maximum x (right), font units.
    pub x_max: i16,
    /// Maximum y (top), font units.
    pub y_max: i16,
}

/// Receives outline segments as the backend walks a glyph contour.
///
/// Coordinates are in raw **font units** (the caller em-normalizes by dividing
/// by `units_per_em`). The T1 extractor implements this directly so no
/// intermediate per-segment allocation is forced on the backend.
pub trait OutlineSink {
    /// Begin a new contour at `(x, y)`.
    fn move_to(&mut self, x: f32, y: f32);
    /// Straight segment to `(x, y)`.
    fn line_to(&mut self, x: f32, y: f32);
    /// Quadratic Bézier with control `(cx, cy)` to `(x, y)`.
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32);
    /// Cubic Bézier with controls `(c0x, c0y)`, `(c1x, c1y)` to `(x, y)`.
    fn cubic_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32);
    /// Close the current contour back to its `move_to` start.
    fn close(&mut self);
}

/// The in-house font-extraction surface. Load-time only — never on the render
/// hot path. The chosen backend ([`TtfFace`], or a future in-house parser)
/// implements it.
pub trait FontFace {
    /// Font design grid resolution (the divisor for em-normalization).
    fn units_per_em(&self) -> u16;
    /// Typographic ascender, font units.
    fn ascender(&self) -> i16;
    /// Typographic descender, font units (typically negative).
    fn descender(&self) -> i16;
    /// Recommended additional line gap, font units.
    fn line_gap(&self) -> i16;
    /// Resolve a Unicode codepoint to a glyph id, or `None` when unmapped.
    fn glyph_index(&self, cp: char) -> Option<GlyphId>;
    /// Horizontal advance for a glyph, font units.
    fn advance(&self, g: GlyphId) -> u16;
    /// Left side bearing for a glyph, font units.
    fn left_side_bearing(&self, g: GlyphId) -> i16;
    /// Walk the glyph outline into `sink`. Returns the glyph bounding box, or
    /// `None` for an empty glyph (e.g. space).
    fn outline(&self, g: GlyphId, sink: &mut dyn OutlineSink) -> Option<BBox>;
    /// Horizontal kerning adjustment between `left` and `right`, font units
    /// (0 when none).
    fn kerning(&self, left: GlyphId, right: GlyphId) -> i16;
}

/// `ttf-parser`-backed [`FontFace`]. Owns the font byte buffer so the borrowed
/// `Face<'a>` stays valid for the adapter's lifetime.
///
/// Field order is load-bearing: `face` is declared BEFORE `data`, so Rust's
/// declaration-order drop runs `face`'s destructor first and frees `data`'s heap
/// second. The borrower is therefore guaranteed to be dropped before the
/// borrowed allocation, which keeps the self-reference sound even if a future
/// backend's face type gains a `Drop` impl.
pub struct TtfFace {
    /// The borrowed `ttf-parser` face. Its lifetime is erased to `'static` and
    /// re-confined by only ever handing out `Face<'_>` views inside method
    /// bodies (see [`TtfFace::face`]). Declared first so it drops first.
    face: Face<'static>,
    /// The owned font bytes that `face` borrows. Never read directly: it exists
    /// purely to keep the backing allocation alive for as long as `face` borrows
    /// it. That ownership IS its load-bearing role; declared last so it drops
    /// last (after `face`).
    #[allow(dead_code)]
    data: Box<[u8]>,
}

impl TtfFace {
    /// Parse a font from its raw bytes (`.ttf` or `.otf`).
    ///
    /// Returns `None` when the bytes are not a parseable font. The bytes are
    /// copied into an owned buffer so the face outlives the caller's slice.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let data: Box<[u8]> = bytes.into();
        // SAFETY: we forge a `&'static [u8]` over `data`'s heap buffer and hand
        // it to `Face::parse`, then store the resulting `Face<'static>` and
        // `data` in the same struct (the standard owning-face self-reference).
        // The `'static` lifetime is a CONTROLLED LIE: the slice does NOT live
        // for the program, only for as long as `self` does, and that lie is
        // never observed by safe code because the only way out is `face()`,
        // which re-confines the view to `Face<'_>` borrowed from `&self`. So a
        // borrow of the forged slice can never outlive `data`. The concrete
        // invariants that make the forge sound for the whole life of `self`:
        //  1. Heap-stable backing: the bytes live behind a `Box<[u8]>`, so the
        //     allocation does not move when `self` is moved — only the 16-byte
        //     box pointer moves; the address the slice points into is fixed for
        //     the allocation's lifetime, so the pointer never dangles on a move.
        //  2. Frozen after construction: no `&mut` to `data` is ever handed out
        //     and the bytes are never re-read directly, so the buffer is neither
        //     mutated nor reallocated while `face` borrows it — the view stays
        //     valid and the aliasing model sees only shared reads.
        //  3. Drop order: `face` is declared before `data`, so declaration-order
        //     drop runs `face`'s destructor FIRST and frees `data`'s heap
        //     SECOND — the borrower is always dropped before the borrowed
        //     allocation, even if a future backend's face type gains a `Drop`.
        // Miri (Tree Borrows) confirms these hold: no use-after-free, no
        // aliasing violation across construction, use, and drop.
        let face = {
            let slice: &'static [u8] =
                unsafe { core::slice::from_raw_parts(data.as_ptr(), data.len()) };
            Face::parse(slice, 0).ok()?
        };
        Some(Self { face, data })
    }

    /// Borrow the underlying `ttf-parser` face. The returned reference is
    /// confined to the borrow of `self`.
    #[inline]
    fn face(&self) -> &Face<'_> {
        &self.face
    }
}

/// Adapts an [`OutlineSink`] to `ttf-parser`'s `OutlineBuilder`. Bridges the
/// two trait shapes without allocating.
struct SinkBridge<'s> {
    sink: &'s mut dyn OutlineSink,
}

impl OutlineBuilder for SinkBridge<'_> {
    #[inline]
    fn move_to(&mut self, x: f32, y: f32) {
        self.sink.move_to(x, y);
    }

    #[inline]
    fn line_to(&mut self, x: f32, y: f32) {
        self.sink.line_to(x, y);
    }

    #[inline]
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.sink.quad_to(x1, y1, x, y);
    }

    #[inline]
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.sink.cubic_to(x1, y1, x2, y2, x, y);
    }

    #[inline]
    fn close(&mut self) {
        self.sink.close();
    }
}

impl FontFace for TtfFace {
    #[inline]
    fn units_per_em(&self) -> u16 {
        self.face().units_per_em()
    }

    #[inline]
    fn ascender(&self) -> i16 {
        self.face().ascender()
    }

    #[inline]
    fn descender(&self) -> i16 {
        self.face().descender()
    }

    #[inline]
    fn line_gap(&self) -> i16 {
        self.face().line_gap()
    }

    #[inline]
    fn glyph_index(&self, cp: char) -> Option<GlyphId> {
        self.face().glyph_index(cp).map(|g| GlyphId(g.0))
    }

    #[inline]
    fn advance(&self, g: GlyphId) -> u16 {
        self.face().glyph_hor_advance(TtfGlyphId(g.0)).unwrap_or(0)
    }

    #[inline]
    fn left_side_bearing(&self, g: GlyphId) -> i16 {
        self.face()
            .glyph_hor_side_bearing(TtfGlyphId(g.0))
            .unwrap_or(0)
    }

    fn outline(&self, g: GlyphId, sink: &mut dyn OutlineSink) -> Option<BBox> {
        let mut bridge = SinkBridge { sink };
        let rect = self.face().outline_glyph(TtfGlyphId(g.0), &mut bridge)?;
        Some(BBox {
            x_min: rect.x_min,
            y_min: rect.y_min,
            x_max: rect.x_max,
            y_max: rect.y_max,
        })
    }

    #[inline]
    fn kerning(&self, left: GlyphId, right: GlyphId) -> i16 {
        // Legacy `kern` table only (Latin-first; GPOS is an out-of-scope seam).
        // ttf-parser exposes kern as a subtable iterator; scan for the first
        // horizontal subtable that yields a pair.
        let Some(kern) = self.face().tables().kern else {
            return 0;
        };
        for subtable in kern.subtables {
            if !subtable.horizontal {
                continue;
            }
            if let Some(v) = subtable.glyphs_kerning(TtfGlyphId(left.0), TtfGlyphId(right.0)) {
                return v;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loads the checked-in libre fixture. Tests that need the font are skipped
    /// (return early) if it is absent, since the crate never reads it itself.
    fn fixture() -> Option<TtfFace> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/Ubuntu-Light.ttf");
        let bytes = std::fs::read(path).ok()?;
        TtfFace::from_bytes(&bytes)
    }

    #[test]
    fn from_bytes_rejects_non_font() {
        assert!(TtfFace::from_bytes(b"not a font").is_none(), "garbage bytes ⇒ None");
        assert!(TtfFace::from_bytes(&[]).is_none(), "empty bytes ⇒ None");
    }

    #[test]
    fn owning_face_survives_move() {
        // The self-referential face must stay valid after the owner is moved
        // (the Box pointer moves, the heap buffer does not). Exercise the unsafe
        // self-reference: move the face into a Vec and read through it.
        let Some(face) = fixture() else { return };
        let upem_before = face.units_per_em();
        let moved = [face];
        assert_eq!(moved[0].units_per_em(), upem_before, "face valid after move");
        assert!(moved[0].glyph_index('A').is_some(), "outline access valid after move");
    }

    #[test]
    fn metrics_are_positive_and_sane() {
        let Some(face) = fixture() else { return };
        assert!(face.units_per_em() >= 16, "units_per_em is a real grid size");
        assert!(face.ascender() > 0, "ascender above baseline");
        assert!(face.descender() < 0, "descender below baseline");
    }

    #[test]
    fn glyph_index_maps_ascii() {
        let Some(face) = fixture() else { return };
        assert!(face.glyph_index('A').is_some(), "'A' is mapped");
        assert!(face.glyph_index('z').is_some(), "'z' is mapped");
    }

    #[test]
    fn advance_is_positive_for_letter() {
        let Some(face) = fixture() else { return };
        let a = face.glyph_index('A').expect("'A' mapped");
        assert!(face.advance(a) > 0, "'A' has a positive advance");
    }

    #[test]
    fn outline_sink_receives_segments_for_letter() {
        let Some(face) = fixture() else { return };
        struct Counter {
            moves: usize,
            lines: usize,
            quads: usize,
            cubics: usize,
            closes: usize,
        }
        impl OutlineSink for Counter {
            fn move_to(&mut self, _: f32, _: f32) {
                self.moves += 1;
            }
            fn line_to(&mut self, _: f32, _: f32) {
                self.lines += 1;
            }
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
                self.quads += 1;
            }
            fn cubic_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
                self.cubics += 1;
            }
            fn close(&mut self) {
                self.closes += 1;
            }
        }
        let mut c = Counter { moves: 0, lines: 0, quads: 0, cubics: 0, closes: 0 };
        let a = face.glyph_index('A').expect("'A' mapped");
        let bbox = face.outline(a, &mut c);
        assert!(bbox.is_some(), "'A' produces a bounding box");
        assert!(c.moves >= 1, "at least one contour start");
        assert!(c.lines + c.quads + c.cubics > 0, "'A' emits drawable segments");
    }

    #[test]
    fn outline_of_space_is_empty() {
        let Some(face) = fixture() else { return };
        struct Nop;
        impl OutlineSink for Nop {
            fn move_to(&mut self, _: f32, _: f32) {}
            fn line_to(&mut self, _: f32, _: f32) {}
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
            fn cubic_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn close(&mut self) {}
        }
        if let Some(space) = face.glyph_index(' ') {
            let mut nop = Nop;
            // space has no outline ⇒ outline returns None.
            assert!(face.outline(space, &mut nop).is_none(), "space has no outline");
        }
    }
}
