//! T2b — overlap combiner + scanline sign-correction.
//!
//! Bare nonzero-winding point-in-polygon is the known-insufficient sign source:
//! the unclamped extrapolated pseudo-distance can yield the wrong sign near
//! corners and overlaps. This pass re-derives the **authoritative** inside/
//! outside for every texel by horizontal ray-casting against the true outline
//! (nonzero winding rule, which handles overlapping/self-intersecting contours
//! as a union), and **overrides** the field sign where it disagrees.
//!
//! Overriding the sign in the `[0, 1]`-mapped field means reflecting the value
//! about the 0.5 zero-crossing: `v → 1 − v` for the channels whose mapped sign
//! is wrong. All four channels (RGB + the true-SDF A) are corrected against the
//! same authoritative insideness so they stay consistent (Decision T2-F).

use boyko_math::Vec2;

use crate::extract::Segment;
use crate::msdf::color::ColoredOutline;
use crate::msdf::{GlyphField, texel_center_from_field};

/// Number of subdivision samples per curve when accumulating winding crossings.
/// Curves are flattened to this many chords for the ray-cast; high enough that
/// the crossing parity is exact for glyph-scale outlines.
const CURVE_FLATTEN_SAMPLES: u32 = 16;

/// Accumulates the signed crossing count of a horizontal ray at height `y`
/// going in `+x` from `x0` against one straight edge `a → b`. Contributes `+1`
/// for an upward crossing, `−1` for a downward crossing (nonzero winding).
#[inline]
fn ray_cross_line(a: Vec2, b: Vec2, x0: f32, y: f32) -> i32 {
    // Half-open interval rule to avoid double-counting shared vertices.
    let (upward, lo, hi) = if a.y <= b.y { (true, a, b) } else { (false, b, a) };
    if y < lo.y || y >= hi.y {
        return 0;
    }
    // x of the edge at height y.
    let t = (y - lo.y) / (hi.y - lo.y);
    let x_at = lo.x + t * (hi.x - lo.x);
    if x_at <= x0 {
        return 0;
    }
    if upward { 1 } else { -1 }
}

/// Flattens a segment into chord endpoints and accumulates ray crossings.
fn ray_cross_segment(seg: &Segment, x0: f32, y: f32) -> i32 {
    match *seg {
        Segment::Line { p0, p1 } => ray_cross_line(p0, p1, x0, y),
        Segment::Quad { p0, c, p1 } => {
            let mut prev = p0;
            let mut acc = 0;
            for i in 1..=CURVE_FLATTEN_SAMPLES {
                let t = i as f32 / CURVE_FLATTEN_SAMPLES as f32;
                let mt = 1.0 - t;
                let cur = p0 * (mt * mt) + c * (2.0 * mt * t) + p1 * (t * t);
                acc += ray_cross_line(prev, cur, x0, y);
                prev = cur;
            }
            acc
        }
        Segment::Cubic { p0, c0, c1, p1 } => {
            let mut prev = p0;
            let mut acc = 0;
            for i in 1..=CURVE_FLATTEN_SAMPLES {
                let t = i as f32 / CURVE_FLATTEN_SAMPLES as f32;
                let mt = 1.0 - t;
                let mt2 = mt * mt;
                let t2 = t * t;
                let cur = p0 * (mt2 * mt) + c0 * (3.0 * mt2 * t) + c1 * (3.0 * mt * t2) + p1 * (t2 * t);
                acc += ray_cross_line(prev, cur, x0, y);
                prev = cur;
            }
            acc
        }
    }
}

/// `true` when `p` is inside the outline by the nonzero winding rule (a `+x`
/// ray-cast). Overlapping/self-intersecting contours combine correctly because
/// the winding sum is taken over ALL edges.
fn point_inside(colored: &ColoredOutline, p: Vec2) -> bool {
    let mut winding = 0;
    for e in &colored.edges {
        winding += ray_cross_segment(&e.seg, p.x, p.y);
    }
    winding != 0
}

/// Overrides the field sign with authoritative scanline insideness (T2b).
///
/// For every texel: if its true insideness disagrees with the sign encoded in
/// the field (value > 0.5 means inside under the 0.5 zero-crossing), reflect all
/// four channels about 0.5 so the sign matches truth.
pub fn correct_signs(field: &mut GlyphField, colored: &ColoredOutline) {
    let w = field.width;
    let h = field.height;
    for y in 0..h {
        for x in 0..w {
            let p = texel_center_from_field(field, x, y);
            let inside = point_inside(colored, p);
            let base = ((y * w + x) * 4) as usize;
            // Use the true-SDF channel (A) as the sign oracle for "what the
            // field currently encodes": A is the unambiguous single-channel
            // distance, so its >0.5 cleanly means "field thinks inside".
            let field_inside = field.texels[base + 3] > 0.5;
            if field_inside != inside {
                for k in 0..4 {
                    field.texels[base + k] = 1.0 - field.texels[base + k];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::GlyphOutline;
    use crate::msdf::color::color_outline;

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn ray_cross_counts_upward_crossing() {
        // An upward edge crossed to the right of x0 contributes +1.
        let c = ray_cross_line(v(1.0, 0.0), v(1.0, 2.0), 0.0, 1.0);
        assert_eq!(c, 1, "upward edge to the right ⇒ +1");
    }

    #[test]
    fn ray_cross_counts_downward_crossing() {
        let c = ray_cross_line(v(1.0, 2.0), v(1.0, 0.0), 0.0, 1.0);
        assert_eq!(c, -1, "downward edge to the right ⇒ -1");
    }

    #[test]
    fn ray_cross_ignores_edge_left_of_origin() {
        let c = ray_cross_line(v(-1.0, 0.0), v(-1.0, 2.0), 0.0, 1.0);
        assert_eq!(c, 0, "edge to the LEFT of x0 ⇒ 0");
    }

    #[test]
    fn ray_cross_half_open_excludes_top_vertex() {
        // y == hi.y is excluded (half-open) to avoid double-counting shared
        // vertices; y == lo.y is included.
        let top = ray_cross_line(v(1.0, 0.0), v(1.0, 2.0), 0.0, 2.0);
        assert_eq!(top, 0, "y at the top vertex is excluded");
        let bottom = ray_cross_line(v(1.0, 0.0), v(1.0, 2.0), 0.0, 0.0);
        assert_eq!(bottom, 1, "y at the bottom vertex is included");
    }

    #[test]
    fn ray_cross_ignores_out_of_range_scanline() {
        let c = ray_cross_line(v(1.0, 0.0), v(1.0, 1.0), 0.0, 5.0);
        assert_eq!(c, 0, "scanline above the edge ⇒ 0");
    }

    #[test]
    fn point_inside_unit_square() {
        let sq = vec![
            Segment::Line { p0: v(0.0, 0.0), p1: v(1.0, 0.0) },
            Segment::Line { p0: v(1.0, 0.0), p1: v(1.0, 1.0) },
            Segment::Line { p0: v(1.0, 1.0), p1: v(0.0, 1.0) },
            Segment::Line { p0: v(0.0, 1.0), p1: v(0.0, 0.0) },
        ];
        let o = GlyphOutline { contours: vec![sq], bbox_min: Vec2::ZERO, bbox_max: v(1.0, 1.0) };
        let colored = color_outline(&o);
        assert!(point_inside(&colored, v(0.5, 0.5)), "center is inside");
        assert!(!point_inside(&colored, v(1.5, 0.5)), "right exterior is outside");
        assert!(!point_inside(&colored, v(-0.5, 0.5)), "left exterior is outside");
    }

    #[test]
    fn point_inside_handles_hole_via_winding() {
        // Outer CCW square with an inner CW square (a hole). A point inside the
        // hole reads OUTSIDE under nonzero winding.
        let outer = vec![
            Segment::Line { p0: v(0.0, 0.0), p1: v(4.0, 0.0) },
            Segment::Line { p0: v(4.0, 0.0), p1: v(4.0, 4.0) },
            Segment::Line { p0: v(4.0, 4.0), p1: v(0.0, 4.0) },
            Segment::Line { p0: v(0.0, 4.0), p1: v(0.0, 0.0) },
        ];
        let inner = vec![
            // CW (reversed winding) hole.
            Segment::Line { p0: v(1.0, 1.0), p1: v(1.0, 3.0) },
            Segment::Line { p0: v(1.0, 3.0), p1: v(3.0, 3.0) },
            Segment::Line { p0: v(3.0, 3.0), p1: v(3.0, 1.0) },
            Segment::Line { p0: v(3.0, 1.0), p1: v(1.0, 1.0) },
        ];
        let o = GlyphOutline { contours: vec![outer, inner], bbox_min: Vec2::ZERO, bbox_max: v(4.0, 4.0) };
        let colored = color_outline(&o);
        assert!(point_inside(&colored, v(0.5, 2.0)), "between hole and edge is inside");
        assert!(!point_inside(&colored, v(2.0, 2.0)), "inside the hole reads outside");
    }

    #[test]
    fn correct_signs_flips_only_disagreeing_texels() {
        // A 1x1 field whose .a says inside (0.9) but the point is OUTSIDE the
        // (empty) outline ⇒ the sign must flip to outside.
        let colored = color_outline(&GlyphOutline {
            contours: vec![vec![
                Segment::Line { p0: v(10.0, 10.0), p1: v(11.0, 10.0) },
                Segment::Line { p0: v(11.0, 10.0), p1: v(10.0, 10.0) },
            ]],
            bbox_min: v(10.0, 10.0),
            bbox_max: v(11.0, 11.0),
        });
        let mut field = GlyphField {
            width: 1,
            height: 1,
            texels: vec![0.9, 0.9, 0.9, 0.9],
            origin_em: Vec2::ZERO,
            texel_em: 1.0,
        };
        correct_signs(&mut field, &colored);
        // Texel center (0.5,0.5) is far from the degenerate outline ⇒ outside ⇒
        // the field (which claimed inside) must be reflected below 0.5.
        assert!(field.texels[3] < 0.5, "sign reflected to outside, got {}", field.texels[3]);
    }
}
