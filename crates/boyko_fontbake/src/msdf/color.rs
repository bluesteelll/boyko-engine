//! T2a — edge coloring.
//!
//! Assigns each edge an [`EdgeColor`] RGB bitmask such that the two edges
//! meeting at a corner share **exactly one** channel: the shared channel keeps
//! the sharp median intersection while the others extrapolate straight. The
//! corner predicate matches msdfgen `edge-coloring.h` (max-angle cutoff,
//! `crossThreshold = sin(maxAngle)`), and the switch is driven by a pinned
//! deterministic seed (Decision T2-E).
//!
//! Explicit topology cases (§T2a):
//! - **0-corner** smooth loop (`O`/`o`): no corners, so a single color cannot
//!   represent the ring — it is split into ~3 single-color arcs at evenly-spaced
//!   synthetic boundaries (msdfgen fully-smooth handling). Each arc keeps one
//!   color, so there is no mid-curve channel seam.
//! - **1-corner** teardrop: the single corner splits the loop into thirds.
//! - **N-corner**: the per-corner switch walk, switching AT every corner
//!   including the starting one, so both edges at every corner (the wrap-around
//!   seam included) share exactly one channel.

use boyko_math::Vec2;

use crate::constants::{EDGE_COLORING_SEED, MAX_CORNER_ANGLE_RAD};
use crate::extract::{GlyphOutline, Segment};

/// An RGB edge-color bitmask. Every edge lights ≥2 channels (the MSDF
/// invariant), so the per-channel min reconstructs a 3-way median.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeColor(pub u8);

impl EdgeColor {
    /// R + G.
    pub const YELLOW: EdgeColor = EdgeColor(0b011);
    /// R + B.
    pub const MAGENTA: EdgeColor = EdgeColor(0b101);
    /// G + B.
    pub const CYAN: EdgeColor = EdgeColor(0b110);
    /// R + G + B (initial/whole color before any switch).
    pub const WHITE: EdgeColor = EdgeColor(0b111);

    /// `true` when this color includes channel `c` (0=R, 1=G, 2=B).
    #[inline]
    pub fn has_channel(self, c: usize) -> bool {
        (self.0 >> c) & 1 == 1
    }
}

/// One colored edge: the source segment plus its assigned channel mask.
#[derive(Clone, Copy, Debug)]
pub struct ColoredEdge {
    /// The geometry.
    pub seg: Segment,
    /// The channel mask.
    pub color: EdgeColor,
}

/// An outline with per-edge colors, ready for the per-channel distance pass.
#[derive(Clone, Debug)]
pub struct ColoredOutline {
    /// Colored edges, flattened across all contours (the distance pass iterates
    /// these per channel).
    pub edges: Vec<ColoredEdge>,
    /// Per-contour edge index ranges `[start, end)` into `edges`, so the
    /// scanline pass can walk contours independently.
    pub contour_ranges: Vec<(usize, usize)>,
    /// Tight em bbox minimum (copied from the outline for the field layout).
    pub bbox_min: Vec2,
    /// Tight em bbox maximum.
    pub bbox_max: Vec2,
}

/// A small deterministic xorshift PRNG seeded from the pinned bake seed. The
/// coloring switch consumes it so goldens are byte-reproducible (Decision
/// T2-E).
struct SwitchRng(u64);

impl SwitchRng {
    #[inline]
    fn new(seed: u64) -> Self {
        // Avoid the all-zero fixed point.
        Self(seed | 1)
    }

    #[inline]
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Unit direction of a segment at its start point (used for the corner
/// predicate at the join with the previous edge's end direction).
#[inline]
fn start_direction(seg: &Segment) -> Vec2 {
    match *seg {
        Segment::Line { p0, p1 } => (p1 - p0).normalize(),
        Segment::Quad { p0, c, p1 } => {
            // B'(0) = 2(c - p0); fall back to the chord if degenerate.
            let d = (c - p0) * 2.0;
            if d.length_squared() > f32::MIN_POSITIVE {
                d.normalize()
            } else {
                (p1 - p0).normalize()
            }
        }
        Segment::Cubic { p0, c0, c1, p1 } => {
            let d = (c0 - p0) * 3.0;
            if d.length_squared() > f32::MIN_POSITIVE {
                d.normalize()
            } else {
                let d2 = (c1 - p0) * 3.0;
                if d2.length_squared() > f32::MIN_POSITIVE {
                    d2.normalize()
                } else {
                    (p1 - p0).normalize()
                }
            }
        }
    }
}

/// Unit direction of a segment at its end point.
#[inline]
fn end_direction(seg: &Segment) -> Vec2 {
    match *seg {
        Segment::Line { p0, p1 } => (p1 - p0).normalize(),
        Segment::Quad { p0, c, p1 } => {
            let d = (p1 - c) * 2.0;
            if d.length_squared() > f32::MIN_POSITIVE {
                d.normalize()
            } else {
                (p1 - p0).normalize()
            }
        }
        Segment::Cubic { p0, c0, c1, p1 } => {
            let d = (p1 - c1) * 3.0;
            if d.length_squared() > f32::MIN_POSITIVE {
                d.normalize()
            } else {
                let d2 = (p1 - c0) * 3.0;
                if d2.length_squared() > f32::MIN_POSITIVE {
                    d2.normalize()
                } else {
                    (p1 - p0).normalize()
                }
            }
        }
    }
}

/// The corner predicate (msdfgen `edge-coloring.h`): a vertex where the
/// incoming `a_dir` meets the outgoing `b_dir` is a corner when the turn is
/// sharper than `maxAngle`. With `cross = a×b` and `dot = a·b`, it is a corner
/// when `dot <= 0 || |cross| > sin(maxAngle)`.
#[inline]
fn is_corner(a_dir: Vec2, b_dir: Vec2) -> bool {
    let cross = a_dir.cross(b_dir);
    let dot = a_dir.dot(b_dir);
    let cross_threshold = MAX_CORNER_ANGLE_RAD.sin();
    dot <= 0.0 || cross.abs() > cross_threshold
}

/// The three two-channel edge colors. Any two of them share exactly one
/// channel, so switching between distinct entries gives the MSDF corner
/// invariant for free.
const PALETTE: [EdgeColor; 3] = [EdgeColor::YELLOW, EdgeColor::MAGENTA, EdgeColor::CYAN];

/// Rotates the active color to a different two-channel value at a corner so the
/// incoming and outgoing edges share exactly one channel. The seeded bit pick
/// breaks the choice deterministically (Decision T2-E). `current == WHITE`
/// (the unseeded initial color) matches no palette entry, so the first switch
/// picks freely.
#[inline]
fn switch_color(current: EdgeColor, rng: &mut SwitchRng) -> EdgeColor {
    let pick = (rng.next() % 3) as usize;
    // Any palette entry != current shares exactly one channel with current
    // (each pair of two-channel masks overlaps in one bit).
    let mut idx = pick;
    for _ in 0..3 {
        if PALETTE[idx] != current {
            return PALETTE[idx];
        }
        idx = (idx + 1) % 3;
    }
    PALETTE[pick]
}

/// Picks a two-channel color that differs from BOTH `a` and `b` (the unique
/// third palette entry). Used to repair a wrap-around where the last span would
/// otherwise collide with the first span's color, restoring the
/// exactly-one-shared-channel invariant at the seam corner.
#[inline]
fn third_color(a: EdgeColor, b: EdgeColor) -> EdgeColor {
    for &c in &PALETTE {
        if c != a && c != b {
            return c;
        }
    }
    // a == b (degenerate): return any entry distinct from a.
    PALETTE.iter().copied().find(|&c| c != a).unwrap_or(PALETTE[0])
}

/// Synthetic switch boundaries for a corner-poor contour: up to three
/// evenly-spaced edge indices around the ring starting at `origin`. A smooth
/// loop or teardrop is thereby split into ~3 single-color arcs (msdfgen's
/// fully-smooth handling) instead of alternating color per edge, which would
/// otherwise paint visible channel seams mid-curve.
///
/// Returns 1 boundary for `n == 1`, 2 for `n == 2`, else 3 (so the wrap-around
/// repair — which needs an even count to trigger — never fires for the odd
/// 3-arc case, and a 2-edge loop still gets a clean two-color split).
fn synthetic_thirds(origin: usize, n: usize) -> Vec<usize> {
    debug_assert!(n >= 1);
    match n {
        1 => vec![origin % n],
        2 => vec![origin % n, (origin + 1) % n],
        _ => {
            // Evenly spaced thirds: offsets round(k·n/3) for k = 0, 1, 2. For
            // n ≥ 3 these are three distinct indices in [0, n).
            let o1 = n / 3;
            let o2 = (2 * n) / 3;
            vec![origin % n, (origin + o1) % n, (origin + o2) % n]
        }
    }
}

/// Colors a single contour in place, writing into `out` and recording the
/// edge-index range. Handles the 0/1/N-corner topology cases.
fn color_contour(contour: &[Segment], rng: &mut SwitchRng, out: &mut Vec<ColoredEdge>) -> (usize, usize) {
    let start = out.len();
    let n = contour.len();
    if n == 0 {
        return (start, start);
    }

    // Locate corners (a join sharper than maxAngle) around the closed loop.
    let mut corners: Vec<usize> = Vec::new();
    for i in 0..n {
        let prev = (i + n - 1) % n;
        let a_dir = end_direction(&contour[prev]);
        let b_dir = start_direction(&contour[i]);
        if is_corner(a_dir, b_dir) {
            corners.push(i);
        }
    }

    // Resolve the span boundaries: the edge indices (in walk order from the
    // first boundary) where the color must switch. For ≥2 real corners these are
    // the corners themselves; the 0/1-corner topology cases synthesize a small
    // number of boundaries so a smooth loop / teardrop is split into a few
    // single-color arcs rather than alternating per edge (msdfgen behaviour).
    let (origin, boundaries) = match corners.len() {
        0 => {
            // Smooth loop (`O`/`o`): no corner exists, so one color cannot
            // represent the ring. Split it into THREE arcs at evenly-spaced
            // boundaries; each arc keeps a single color and adjacent arcs share
            // exactly one channel — no mid-curve seam within an arc.
            (0usize, synthetic_thirds(0, n))
        }
        1 => {
            // Teardrop: split the loop into three arcs about the single corner so
            // the tip is preserved (the corner is boundary 0).
            (corners[0], synthetic_thirds(corners[0], n))
        }
        _ => {
            // N-corner: every corner is a boundary. The walk starts AT the first
            // corner, so boundary 0 (the start) is a switch too — this is what
            // gives the edge ending at `first` and the edge starting at `first`
            // different colors (the previous bug shared both channels there).
            (corners[0], corners.clone())
        }
    };

    // Map global edge indices to "is this index a switch boundary?".
    let mut is_boundary = vec![false; n];
    for &b in &boundaries {
        is_boundary[b % n] = true;
    }

    // Walk from `origin`, switching color whenever we enter a boundary edge —
    // INCLUDING k == 0, which seeds the first span's color and ensures the
    // wrap-around edge (the last one written, ending at `origin`) differs from
    // the first span. Track the first span's color and the previous span's color
    // for the wrap-around repair.
    let span_start = out.len();
    let mut color = EdgeColor::WHITE;
    let mut first_color = EdgeColor::WHITE;
    let mut prev_span_color = EdgeColor::WHITE;
    for k in 0..n {
        let idx = (origin + k) % n;
        if is_boundary[idx] {
            prev_span_color = color;
            color = switch_color(color, rng);
            if k == 0 {
                first_color = color;
            }
        }
        out.push(ColoredEdge {
            seg: contour[idx],
            color,
        });
    }

    // Wrap-around repair: if the LAST span's color collides with the FIRST
    // span's color, the two edges meeting at `origin` would share BOTH channels
    // (a rounded corner). This happens when the boundary count is even, so the
    // color rotation returns to its start. Recolor the final span to the unique
    // third color distinct from both its predecessor span and the first span,
    // restoring exactly-one-shared-channel at the seam.
    if n >= 2 && boundaries.len() >= 2 && color == first_color {
        let repair = third_color(first_color, prev_span_color);
        for ce in out[span_start..].iter_mut().rev() {
            if ce.color != color {
                break;
            }
            ce.color = repair;
        }
    }

    (start, out.len())
}

/// Colors a whole outline (T2a). Returns a flattened colored-edge list plus the
/// per-contour ranges and the tight bbox carried for the field layout.
pub fn color_outline(outline: &GlyphOutline) -> ColoredOutline {
    let mut rng = SwitchRng::new(EDGE_COLORING_SEED);
    let mut edges: Vec<ColoredEdge> = Vec::new();
    let mut contour_ranges: Vec<(usize, usize)> = Vec::with_capacity(outline.contours.len());

    for contour in &outline.contours {
        let range = color_contour(contour, &mut rng, &mut edges);
        contour_ranges.push(range);
    }

    ColoredOutline {
        edges,
        contour_ranges,
        bbox_min: outline.bbox_min,
        bbox_max: outline.bbox_max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::GlyphOutline;

    fn line(ax: f32, ay: f32, bx: f32, by: f32) -> Segment {
        Segment::Line { p0: Vec2::new(ax, ay), p1: Vec2::new(bx, by) }
    }

    #[test]
    fn edge_color_has_channel_reads_bitmask() {
        assert!(EdgeColor::YELLOW.has_channel(0), "YELLOW has R");
        assert!(EdgeColor::YELLOW.has_channel(1), "YELLOW has G");
        assert!(!EdgeColor::YELLOW.has_channel(2), "YELLOW has no B");
        assert!(EdgeColor::CYAN.has_channel(2), "CYAN has B");
        assert!(!EdgeColor::CYAN.has_channel(0), "CYAN has no R");
    }

    #[test]
    fn every_palette_entry_lights_exactly_two_channels() {
        for c in PALETTE {
            let bits = (0..3).filter(|&i| c.has_channel(i)).count();
            assert_eq!(bits, 2, "{:?} must light exactly two channels (MSDF invariant)", c);
        }
    }

    #[test]
    fn any_two_palette_entries_share_exactly_one_channel() {
        for (i, &a) in PALETTE.iter().enumerate() {
            for &b in PALETTE.iter().skip(i + 1) {
                let shared = (0..3).filter(|&c| a.has_channel(c) && b.has_channel(c)).count();
                assert_eq!(shared, 1, "{:?} and {:?} must share exactly one channel", a, b);
            }
        }
    }

    #[test]
    fn switch_rng_is_deterministic_for_a_seed() {
        let mut a = SwitchRng::new(EDGE_COLORING_SEED);
        let mut b = SwitchRng::new(EDGE_COLORING_SEED);
        for _ in 0..16 {
            assert_eq!(a.next(), b.next(), "same seed yields the same stream");
        }
    }

    #[test]
    fn switch_rng_avoids_all_zero_fixed_point() {
        // Seed 0 would be a fixed point for xorshift; the constructor must |1 it.
        let mut r = SwitchRng::new(0);
        assert_ne!(r.next(), 0, "a zero seed must still produce nonzero output");
    }

    #[test]
    fn switch_color_returns_distinct_palette_color() {
        let mut rng = SwitchRng::new(EDGE_COLORING_SEED);
        let next = switch_color(EdgeColor::YELLOW, &mut rng);
        assert_ne!(next, EdgeColor::YELLOW, "switch must change the color");
        assert!(PALETTE.contains(&next), "switch yields a palette color");
    }

    #[test]
    fn third_color_is_distinct_from_both_inputs() {
        let t = third_color(EdgeColor::YELLOW, EdgeColor::MAGENTA);
        assert_eq!(t, EdgeColor::CYAN, "the unique third color");
        assert_ne!(t, EdgeColor::YELLOW);
        assert_ne!(t, EdgeColor::MAGENTA);
    }

    #[test]
    fn is_corner_detects_right_angle() {
        // A 90° turn (rightward then upward) is sharper than maxAngle ⇒ corner.
        let a = Vec2::new(1.0, 0.0);
        let b = Vec2::new(0.0, 1.0);
        assert!(is_corner(a, b), "a 90° turn is a corner");
    }

    #[test]
    fn is_corner_ignores_near_straight_join() {
        // Almost collinear (a 1° kink) is flatter than maxAngle ⇒ NOT a corner.
        let a = Vec2::new(1.0, 0.0);
        let rad = 1.0_f32.to_radians();
        let b = Vec2::new(rad.cos(), rad.sin());
        assert!(!is_corner(a, b), "a near-straight join is not a corner");
    }

    #[test]
    fn is_corner_detects_reversal() {
        // A full reversal (dot <= 0) is always a corner.
        let a = Vec2::new(1.0, 0.0);
        let b = Vec2::new(-1.0, 0.0);
        assert!(is_corner(a, b), "a reversal is a corner");
    }

    #[test]
    fn square_contour_colors_share_one_channel_at_each_corner() {
        // A unit square: four lines, four 90° corners. Adjacent edges must share
        // exactly one channel (the MSDF corner invariant), including the
        // wrap-around seam.
        let contour = vec![
            line(0.0, 0.0, 1.0, 0.0),
            line(1.0, 0.0, 1.0, 1.0),
            line(1.0, 1.0, 0.0, 1.0),
            line(0.0, 1.0, 0.0, 0.0),
        ];
        let outline = GlyphOutline {
            contours: vec![contour],
            bbox_min: Vec2::ZERO,
            bbox_max: Vec2::new(1.0, 1.0),
        };
        let colored = color_outline(&outline);
        let n = colored.edges.len();
        assert_eq!(n, 4);
        for i in 0..n {
            let a = colored.edges[i].color;
            let b = colored.edges[(i + 1) % n].color;
            let shared = (0..3).filter(|&c| a.has_channel(c) && b.has_channel(c)).count();
            assert_eq!(shared, 1, "edges {} and {} must share exactly one channel", i, (i + 1) % n);
        }
    }

    #[test]
    fn coloring_is_deterministic() {
        // Same outline ⇒ same coloring (pinned seed, Decision T2-E).
        let contour = vec![
            line(0.0, 0.0, 1.0, 0.0),
            line(1.0, 0.0, 1.0, 1.0),
            line(1.0, 1.0, 0.0, 0.0),
        ];
        let outline = GlyphOutline {
            contours: vec![contour],
            bbox_min: Vec2::ZERO,
            bbox_max: Vec2::new(1.0, 1.0),
        };
        let a = color_outline(&outline);
        let b = color_outline(&outline);
        let ca: Vec<u8> = a.edges.iter().map(|e| e.color.0).collect();
        let cb: Vec<u8> = b.edges.iter().map(|e| e.color.0).collect();
        assert_eq!(ca, cb, "coloring must be reproducible for a fixed seed");
    }

    #[test]
    fn empty_outline_colors_to_no_edges() {
        let outline = GlyphOutline::default();
        let colored = color_outline(&outline);
        assert!(colored.edges.is_empty(), "no contours ⇒ no colored edges");
        assert!(colored.contour_ranges.is_empty());
    }

    #[test]
    fn synthetic_thirds_counts_match_topology() {
        assert_eq!(synthetic_thirds(0, 1).len(), 1, "1-edge loop ⇒ 1 boundary");
        assert_eq!(synthetic_thirds(0, 2).len(), 2, "2-edge loop ⇒ 2 boundaries");
        assert_eq!(synthetic_thirds(0, 9).len(), 3, "n≥3 ⇒ 3 evenly-spaced boundaries");
        let b = synthetic_thirds(0, 9);
        assert_eq!(b, vec![0, 3, 6], "evenly spaced thirds of 9");
    }
}
