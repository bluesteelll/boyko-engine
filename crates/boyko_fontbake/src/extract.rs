//! T1 — glyph outline + metrics extraction via the T0 [`FontFace`].
//!
//! Produces an em-normalized [`GlyphOutline`] (segments divided by
//! `units_per_em`) plus the per-glyph and face metrics the MSDF generator and
//! atlas packer consume. All coordinates downstream of this module are in **em
//! units** (1.0 == one em), so the global pixels-per-em scale (§T2c) is the
//! single place font-unit scale is applied.

use boyko_math::Vec2;

use crate::face::{BBox, FontFace, GlyphId};

/// One outline edge in em-normalized coordinates. Closed contours are a list of
/// these; the last segment's end point equals the contour's start point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    /// Straight line `p0 → p1`.
    Line { p0: Vec2, p1: Vec2 },
    /// Quadratic Bézier `p0 → p1` with control `c`.
    Quad { p0: Vec2, c: Vec2, p1: Vec2 },
    /// Cubic Bézier `p0 → p1` with controls `c0`, `c1` (CFF/OTF-PS only).
    Cubic { p0: Vec2, c0: Vec2, c1: Vec2, p1: Vec2 },
}

impl Segment {
    /// Start point of the segment.
    #[inline]
    pub fn start(&self) -> Vec2 {
        match *self {
            Segment::Line { p0, .. } | Segment::Quad { p0, .. } | Segment::Cubic { p0, .. } => p0,
        }
    }

    /// End point of the segment.
    #[inline]
    pub fn end(&self) -> Vec2 {
        match *self {
            Segment::Line { p1, .. } | Segment::Quad { p1, .. } | Segment::Cubic { p1, .. } => p1,
        }
    }
}

/// A single contour: a closed loop of edge segments, em-normalized.
pub type Contour = Vec<Segment>;

/// The full em-normalized outline of one glyph: a list of contours.
#[derive(Clone, Debug, Default)]
pub struct GlyphOutline {
    /// Closed contours, in font order.
    pub contours: Vec<Contour>,
    /// Em-normalized tight bounding box `(min, max)`. `(ZERO, ZERO)` for an
    /// empty glyph.
    pub bbox_min: Vec2,
    /// Tight bounding box maximum (em).
    pub bbox_max: Vec2,
}

impl GlyphOutline {
    /// `true` when the glyph has no drawable contours (e.g. space).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.contours.is_empty()
    }

    /// Total segment count across all contours (used by goldens).
    #[inline]
    pub fn segment_count(&self) -> usize {
        self.contours.iter().map(Vec::len).sum()
    }

    /// Signed area of all contours via the shoelace formula on segment
    /// endpoints (control points do not affect the sign of the integral for a
    /// closed contour's chord polygon's orientation in practice for winding
    /// determination — this gives the gross winding sign). Positive is
    /// counter-clockwise in a y-up coordinate system. Provisional sign only;
    /// authoritative insideness comes from the T2b scanline pass.
    pub fn signed_area(&self) -> f32 {
        let mut area = 0.0_f32;
        for contour in &self.contours {
            for seg in contour {
                let a = seg.start();
                let b = seg.end();
                area += a.cross(b);
            }
        }
        0.5 * area
    }
}

/// A glyph's extracted outline plus its layout metrics, all em-normalized.
#[derive(Clone, Debug)]
pub struct Glyph {
    /// The glyph id within the face.
    pub id: GlyphId,
    /// The em-normalized outline.
    pub outline: GlyphOutline,
    /// Horizontal pen advance in em units.
    pub advance_em: f32,
    /// Left side bearing in em units.
    pub lsb_em: f32,
}

/// Face-wide metrics in em units (font-unit values divided by `units_per_em`).
#[derive(Clone, Copy, Debug)]
pub struct FaceMetrics {
    /// Design grid resolution (font units per em).
    pub units_per_em: u16,
    /// Ascender, em.
    pub ascender_em: f32,
    /// Descender, em (typically negative).
    pub descender_em: f32,
    /// Line gap, em.
    pub line_gap_em: f32,
}

/// Reads the face-wide metrics, em-normalized.
pub fn face_metrics(face: &dyn FontFace) -> FaceMetrics {
    let upem = face.units_per_em().max(1);
    let inv = 1.0 / upem as f32;
    FaceMetrics {
        units_per_em: upem,
        ascender_em: face.ascender() as f32 * inv,
        descender_em: face.descender() as f32 * inv,
        line_gap_em: face.line_gap() as f32 * inv,
    }
}

/// Collects raw font-unit outline segments from a [`FontFace`], then
/// em-normalizes them. Implements [`crate::face::OutlineSink`].
struct OutlineCollector {
    inv_upem: f32,
    contours: Vec<Contour>,
    current: Contour,
    start: Vec2,
    cursor: Vec2,
}

impl OutlineCollector {
    #[inline]
    fn new(inv_upem: f32) -> Self {
        Self {
            inv_upem,
            contours: Vec::new(),
            current: Vec::new(),
            start: Vec2::ZERO,
            cursor: Vec2::ZERO,
        }
    }

    #[inline]
    fn norm(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(x * self.inv_upem, y * self.inv_upem)
    }

    fn finish_contour(&mut self) {
        if !self.current.is_empty() {
            // Implicitly close: if the last point does not return to the
            // contour start, append a closing line (TrueType `glyf` contours
            // are always closed; CFF charstrings emit an explicit close).
            if self.cursor != self.start {
                self.current.push(Segment::Line {
                    p0: self.cursor,
                    p1: self.start,
                });
            }
            let done = core::mem::take(&mut self.current);
            self.contours.push(done);
        }
    }
}

impl crate::face::OutlineSink for OutlineCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        let p = self.norm(x, y);
        self.start = p;
        self.cursor = p;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p1 = self.norm(x, y);
        self.current.push(Segment::Line {
            p0: self.cursor,
            p1,
        });
        self.cursor = p1;
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let c = self.norm(cx, cy);
        let p1 = self.norm(x, y);
        self.current.push(Segment::Quad {
            p0: self.cursor,
            c,
            p1,
        });
        self.cursor = p1;
    }

    fn cubic_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        let c0 = self.norm(c0x, c0y);
        let c1 = self.norm(c1x, c1y);
        let p1 = self.norm(x, y);
        self.current.push(Segment::Cubic {
            p0: self.cursor,
            c0,
            c1,
            p1,
        });
        self.cursor = p1;
    }

    fn close(&mut self) {
        self.finish_contour();
    }
}

/// Extracts and em-normalizes a single glyph's outline + metrics.
///
/// Returns a [`Glyph`] with an empty outline (and advance only) for glyphs that
/// have no drawable contours (e.g. space).
pub fn extract_glyph(face: &dyn FontFace, id: GlyphId) -> Glyph {
    let upem = face.units_per_em().max(1);
    let inv = 1.0 / upem as f32;

    let mut collector = OutlineCollector::new(inv);
    let bbox: Option<BBox> = face.outline(id, &mut collector);
    collector.finish_contour();

    let (bbox_min, bbox_max) = match bbox {
        Some(b) => (
            Vec2::new(b.x_min as f32 * inv, b.y_min as f32 * inv),
            Vec2::new(b.x_max as f32 * inv, b.y_max as f32 * inv),
        ),
        None => (Vec2::ZERO, Vec2::ZERO),
    };

    Glyph {
        id,
        outline: GlyphOutline {
            contours: collector.contours,
            bbox_min,
            bbox_max,
        },
        advance_em: face.advance(id) as f32 * inv,
        lsb_em: face.left_side_bearing(id) as f32 * inv,
    }
}

/// Extracts the glyph for a codepoint, falling back to glyph id 0 (`.notdef`)
/// when the codepoint is unmapped.
pub fn extract_codepoint(face: &dyn FontFace, cp: char) -> Glyph {
    let id = face.glyph_index(cp).unwrap_or(GlyphId(0));
    extract_glyph(face, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn segment_start_end_for_each_kind() {
        let l = Segment::Line { p0: v(0.0, 0.0), p1: v(1.0, 2.0) };
        assert_eq!(l.start(), v(0.0, 0.0));
        assert_eq!(l.end(), v(1.0, 2.0));
        let q = Segment::Quad { p0: v(0.0, 0.0), c: v(0.5, 1.0), p1: v(1.0, 0.0) };
        assert_eq!(q.start(), v(0.0, 0.0));
        assert_eq!(q.end(), v(1.0, 0.0));
        let cu = Segment::Cubic { p0: v(0.0, 0.0), c0: v(0.3, 1.0), c1: v(0.7, 1.0), p1: v(1.0, 0.0) };
        assert_eq!(cu.start(), v(0.0, 0.0));
        assert_eq!(cu.end(), v(1.0, 0.0));
    }

    #[test]
    fn empty_outline_is_empty_and_zero_segments() {
        let o = GlyphOutline::default();
        assert!(o.is_empty());
        assert_eq!(o.segment_count(), 0);
        assert_eq!(o.signed_area(), 0.0);
    }

    #[test]
    fn segment_count_sums_all_contours() {
        let o = GlyphOutline {
            contours: vec![
                vec![Segment::Line { p0: v(0.0, 0.0), p1: v(1.0, 0.0) }],
                vec![
                    Segment::Line { p0: v(0.0, 0.0), p1: v(1.0, 0.0) },
                    Segment::Line { p0: v(1.0, 0.0), p1: v(0.0, 0.0) },
                ],
            ],
            bbox_min: Vec2::ZERO,
            bbox_max: Vec2::ZERO,
        };
        assert_eq!(o.segment_count(), 3, "1 + 2 segments");
        assert!(!o.is_empty());
    }

    #[test]
    fn signed_area_ccw_unit_square_is_positive() {
        // CCW unit square in y-up: shoelace area = +1.
        let sq = vec![
            Segment::Line { p0: v(0.0, 0.0), p1: v(1.0, 0.0) },
            Segment::Line { p0: v(1.0, 0.0), p1: v(1.0, 1.0) },
            Segment::Line { p0: v(1.0, 1.0), p1: v(0.0, 1.0) },
            Segment::Line { p0: v(0.0, 1.0), p1: v(0.0, 0.0) },
        ];
        let o = GlyphOutline { contours: vec![sq], bbox_min: Vec2::ZERO, bbox_max: v(1.0, 1.0) };
        assert!((o.signed_area() - 1.0).abs() < 1e-5, "CCW square area = +1, got {}", o.signed_area());
    }

    #[test]
    fn signed_area_cw_unit_square_is_negative() {
        let sq = vec![
            Segment::Line { p0: v(0.0, 0.0), p1: v(0.0, 1.0) },
            Segment::Line { p0: v(0.0, 1.0), p1: v(1.0, 1.0) },
            Segment::Line { p0: v(1.0, 1.0), p1: v(1.0, 0.0) },
            Segment::Line { p0: v(1.0, 0.0), p1: v(0.0, 0.0) },
        ];
        let o = GlyphOutline { contours: vec![sq], bbox_min: Vec2::ZERO, bbox_max: v(1.0, 1.0) };
        assert!((o.signed_area() + 1.0).abs() < 1e-5, "CW square area = -1, got {}", o.signed_area());
    }
}
