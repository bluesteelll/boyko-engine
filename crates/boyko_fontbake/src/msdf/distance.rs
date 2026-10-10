//! T2a — per-channel signed pseudo-distance + range mapping.
//!
//! This mirrors the canonical msdfgen **two-distance model**. For each edge we
//! evaluate both metrics at the query point (`EdgeDistance::eval`):
//!
//! - the **true** signed distance: the distance to the *nearest point on the
//!   segment* (parameter clamped to `[0, 1]`), tagged with an **orthogonality**
//!   value (`|sin θ|` between the edge tangent and the offset at an endpoint)
//!   used as msdfgen's selection tie-break;
//! - the **pseudo** signed distance: when the foot of the perpendicular falls
//!   *outside* `[0, 1]` the magnitude is replaced by the perpendicular distance
//!   to the infinite extension of the edge's endpoint tangent (the
//!   corner-straightening term). Inside `[0, 1]` the pseudo distance equals the
//!   true distance.
//!
//! Per channel `c ∈ {R, G, B}` we select the **nearest edge by TRUE distance**
//! (with the orthogonality tie-break) among the edges whose color includes `c`,
//! then store that winning edge's **PSEUDO** distance. Lines and curves use the
//! identical metric, so they compare on the same scale and curves get the same
//! corner straightening as lines. The MTSDF 4th channel (A) is the true
//! single-channel SDF: the same selection over ALL edges regardless of color,
//! storing the winner's pseudo distance, identical range mapping (Decision T2-F).
//!
//! Per-primitive nearest-point math:
//! - **Line**: closed-form point-segment projection.
//! - **Quadratic**: `d/dt|B(t)−p|²=0` is a cubic in `t` → solved via Cardano,
//!   endpoints also tested.
//! - **Cubic**: the nearest-point is a quintic → multi-seed Newton on the
//!   squared-distance derivative from a fixed set of evenly-spaced seeds, plus
//!   both endpoints, global min (msdfgen-style).
//!
//! The sign carried here is the **provisional** pseudo-distance sign; the T2b
//! scanline pass overrides it with authoritative insideness.

use std::sync::Arc;

use boyko_math::Vec2;
use boyko_threadpool::ThreadPool;

use crate::constants::{CUBIC_NEWTON_ITERS, CUBIC_NEWTON_SEEDS};
use crate::extract::Segment;
use crate::msdf::color::{ColoredEdge, ColoredOutline};
use crate::msdf::{
    FieldLayout, GlyphField, MAX_FIELD_DIM, field_layout, map_distance, texel_center,
};

/// The msdfgen-style selection key for one edge at one query point: the **true**
/// signed distance (clamped to the segment) plus an orthogonality tie-break, and
/// the **pseudo** signed distance (perpendicular/extrapolated past the endpoints)
/// that is stored once this edge wins its channel.
#[derive(Clone, Copy, Debug)]
struct EdgeDistance {
    /// Unsigned true distance to the nearest point ON the segment (em).
    true_dist: f32,
    /// Sign of the true distance (+1 outside / −1 inside, by the tangent side).
    true_sign: f32,
    /// `|sin θ|` between the unit tangent and the unit offset at the nearest
    /// point. msdfgen's secondary selection key: among edges with equal true
    /// distance (a shared corner vertex), the one whose offset is MORE
    /// orthogonal to its tangent (larger value) is the better representative.
    orthogonality: f32,
    /// Signed pseudo-distance: equals the true distance when the foot lands
    /// inside `[0, 1]`; otherwise the perpendicular distance to the infinite
    /// extension of the endpoint tangent (the corner-straightening term).
    pseudo_signed: f32,
}

impl EdgeDistance {
    const FAR: EdgeDistance = EdgeDistance {
        true_dist: f32::INFINITY,
        true_sign: 1.0,
        orthogonality: 0.0,
        pseudo_signed: f32::INFINITY,
    };

    /// msdfgen edge ordering: nearer true distance wins; on a tie (a shared
    /// vertex) the more-orthogonal offset wins. Returns `true` when `self` is the
    /// better (closer) representative than `other`.
    #[inline]
    fn closer_than(&self, other: &EdgeDistance) -> bool {
        self.true_dist < other.true_dist
            || (self.true_dist == other.true_dist && self.orthogonality > other.orthogonality)
    }
}

/// Builds an [`EdgeDistance`] from a resolved nearest-point query: the clamped
/// parameter `tc`, the nearest point `pt`, the (unnormalized) tangent there, the
/// offset `p − pt`, and whether the foot was clamped to an endpoint.
///
/// When clamped, the pseudo magnitude is the perpendicular distance to the
/// infinite line through `pt` along `tangent` (the extrapolation that straightens
/// corners). When not clamped, pseudo == true.
#[inline]
fn make_edge_distance(pt: Vec2, tangent: Vec2, p: Vec2, clamped: bool) -> EdgeDistance {
    let offset = p - pt;
    let true_dist = offset.length();
    // Unit tangent → `cross` is exactly the signed perpendicular distance from
    // `p` to the infinite line through `pt` (|offset|·sin θ).
    let tangent_n = tangent.normalize();
    let cross = tangent_n.cross(offset);
    let true_sign = if cross >= 0.0 { 1.0 } else { -1.0 };
    // Orthogonality = |sin θ| between unit tangent and unit offset; 0 when the
    // offset is degenerate. This is the msdfgen endpoint tie-break.
    let orthogonality = if true_dist > f32::MIN_POSITIVE {
        (cross / true_dist).abs()
    } else {
        0.0
    };
    let pseudo_signed = if clamped {
        // Perpendicular distance to the infinite extension of the endpoint
        // tangent — extrapolate past the segment end. `cross` already carries
        // both magnitude and side (unit tangent), so this is the signed value.
        cross
    } else {
        true_sign * true_dist
    };
    EdgeDistance {
        true_dist,
        true_sign,
        orthogonality,
        pseudo_signed,
    }
}

/// Evaluates a quadratic Bézier at parameter `t`.
#[inline]
fn quad_point(p0: Vec2, c: Vec2, p1: Vec2, t: f32) -> Vec2 {
    let mt = 1.0 - t;
    p0 * (mt * mt) + c * (2.0 * mt * t) + p1 * (t * t)
}

/// Evaluates a quadratic Bézier derivative at `t`.
#[inline]
fn quad_deriv(p0: Vec2, c: Vec2, p1: Vec2, t: f32) -> Vec2 {
    let mt = 1.0 - t;
    (c - p0) * (2.0 * mt) + (p1 - c) * (2.0 * t)
}

/// Evaluates a cubic Bézier at `t`.
#[inline]
fn cubic_point(p0: Vec2, c0: Vec2, c1: Vec2, p1: Vec2, t: f32) -> Vec2 {
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let t2 = t * t;
    p0 * (mt2 * mt) + c0 * (3.0 * mt2 * t) + c1 * (3.0 * mt * t2) + p1 * (t2 * t)
}

/// Evaluates a cubic Bézier derivative at `t`.
#[inline]
fn cubic_deriv(p0: Vec2, c0: Vec2, c1: Vec2, p1: Vec2, t: f32) -> Vec2 {
    let mt = 1.0 - t;
    (c0 - p0) * (3.0 * mt * mt) + (c1 - c0) * (6.0 * mt * t) + (p1 - c1) * (3.0 * t * t)
}

/// Both signed distances of a point to a line segment (the two-distance model).
///
/// The true distance clamps the projection to `[0, 1]`; the pseudo distance
/// extrapolates to the infinite line when the foot lands off the segment (the
/// corner-straightening behavior). A line's tangent is constant, so the infinite
/// extension is the line itself.
fn line_edge_distance(p0: Vec2, p1: Vec2, p: Vec2) -> EdgeDistance {
    let dir = p1 - p0;
    let len_sq = dir.length_squared();
    if len_sq <= f32::MIN_POSITIVE {
        // Degenerate (zero-length) edge: treat as the single point p0.
        return make_edge_distance(p0, dir, p, true);
    }
    let t = (p - p0).dot(dir) / len_sq;
    let tc = t.clamp(0.0, 1.0);
    let pt = p0 + dir * tc;
    let clamped = !(0.0..=1.0).contains(&t);
    make_edge_distance(pt, dir, p, clamped)
}

/// Real roots of `a t³ + b t² + c t + d = 0` via Cardano, returned in a fixed
/// buffer (up to 3 roots). Degrades gracefully to quadratic/linear.
fn solve_cubic(a: f32, b: f32, c: f32, d: f32, out: &mut [f32; 3]) -> usize {
    const EPS: f32 = 1e-9;
    if a.abs() < EPS {
        // Quadratic b t² + c t + d.
        return solve_quadratic(b, c, d, out);
    }
    // Normalize to t³ + A t² + B t + C.
    let inv_a = 1.0 / a;
    let aa = b * inv_a;
    let bb = c * inv_a;
    let cc = d * inv_a;

    // Depressed cubic t = x − A/3 → x³ + p x + q.
    let a_over_3 = aa / 3.0;
    let p = bb - aa * a_over_3;
    let q = 2.0 * a_over_3 * a_over_3 * a_over_3 - a_over_3 * bb + cc;

    let disc = q * q / 4.0 + p * p * p / 27.0;
    if disc > EPS {
        // One real root.
        let sqrt_disc = disc.sqrt();
        let u = (-q / 2.0 + sqrt_disc).cbrt();
        let v = (-q / 2.0 - sqrt_disc).cbrt();
        out[0] = u + v - a_over_3;
        1
    } else if disc < -EPS {
        // Three real roots (trigonometric form).
        let r = (-p / 3.0).sqrt();
        let phi = (-q / (2.0 * r * r * r)).clamp(-1.0, 1.0).acos();
        let two_r = 2.0 * r;
        out[0] = two_r * (phi / 3.0).cos() - a_over_3;
        out[1] = two_r * ((phi + 2.0 * std::f32::consts::PI) / 3.0).cos() - a_over_3;
        out[2] = two_r * ((phi + 4.0 * std::f32::consts::PI) / 3.0).cos() - a_over_3;
        3
    } else {
        // Repeated roots.
        let u = (-q / 2.0).cbrt();
        out[0] = 2.0 * u - a_over_3;
        out[1] = -u - a_over_3;
        2
    }
}

/// Real roots of `a t² + b t + c = 0`, returned in `out` (up to 2).
fn solve_quadratic(a: f32, b: f32, c: f32, out: &mut [f32; 3]) -> usize {
    const EPS: f32 = 1e-9;
    if a.abs() < EPS {
        if b.abs() < EPS {
            return 0;
        }
        out[0] = -c / b;
        return 1;
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        0
    } else {
        let sq = disc.sqrt();
        let inv = 1.0 / (2.0 * a);
        out[0] = (-b + sq) * inv;
        out[1] = (-b - sq) * inv;
        2
    }
}

/// Both signed distances of `p` to a quadratic Bézier (the two-distance model).
/// Solves the cubic `d/dt|B(t)−p|² = 0` via Cardano, tests the interior critical
/// points plus both endpoints, and keeps the closest-true point. Endpoint wins
/// extrapolate the pseudo distance along the endpoint tangent.
fn quad_edge_distance(p0: Vec2, c: Vec2, p1: Vec2, p: Vec2) -> EdgeDistance {
    // |B(t)−p|² derivative is a cubic in t. Coefficients (Bézier basis):
    // B(t)−p = (1−t)²p0 + 2(1−t)t c + t² p1 − p.
    // Let a = p0 − 2c + p1, b = 2(c − p0). Then B(t) = a t² + b t + p0.
    let a = p0 - c * 2.0 + p1;
    let b = (c - p0) * 2.0;
    let q0 = p0 - p;

    // d/dt |a t² + b t + q0|² = 0 expands to a cubic:
    // 2( (a·a) 2 t³ + (3 a·b) t² + (2 a·q0 + b·b) t + b·q0 ) — drop the 2.
    let ca = 2.0 * a.dot(a);
    let cb = 3.0 * a.dot(b);
    let cc = a.dot(q0) * 2.0 + b.dot(b);
    let cd = b.dot(q0);

    let mut roots = [0.0_f32; 3];
    let count = solve_cubic(ca, cb, cc, cd, &mut roots);

    let mut best = EdgeDistance::FAR;
    let mut consider = |t: f32| {
        let tc = t.clamp(0.0, 1.0);
        let pt = quad_point(p0, c, p1, tc);
        let tangent = quad_deriv(p0, c, p1, tc);
        let clamped = !(0.0..=1.0).contains(&t);
        let cand = make_edge_distance(pt, tangent, p, clamped);
        if cand.closer_than(&best) {
            best = cand;
        }
    };

    consider(0.0);
    consider(1.0);
    for &r in &roots[..count] {
        if (0.0..=1.0).contains(&r) {
            consider(r);
        }
    }
    best
}

/// Both signed distances of `p` to a cubic Bézier (the two-distance model). The
/// exact nearest-point is a quintic, so we seed Newton from `CUBIC_NEWTON_SEEDS`
/// evenly-spaced parameters, refine each on the squared-distance derivative, test
/// both endpoints, and keep the closest-true point (msdfgen multi-seed strategy).
fn cubic_edge_distance(p0: Vec2, c0: Vec2, c1: Vec2, p1: Vec2, p: Vec2) -> EdgeDistance {
    let mut best = EdgeDistance::FAR;

    let mut consider = |t: f32| {
        let tc = t.clamp(0.0, 1.0);
        let pt = cubic_point(p0, c0, c1, p1, tc);
        let tangent = cubic_deriv(p0, c0, c1, p1, tc);
        let clamped = !(0.0..=1.0).contains(&t);
        let cand = make_edge_distance(pt, tangent, p, clamped);
        if cand.closer_than(&best) {
            best = cand;
        }
    };

    // Endpoints.
    consider(0.0);
    consider(1.0);

    // Multi-seed Newton on f(t) = (B(t)−p)·B'(t) (the squared-distance
    // derivative / 2). f'(t) = B'·B' + (B(t)−p)·B''.
    let denom = (CUBIC_NEWTON_SEEDS.max(1)) as f32;
    for s in 0..CUBIC_NEWTON_SEEDS {
        let mut t = (s as f32 + 0.5) / denom;
        for _ in 0..CUBIC_NEWTON_ITERS {
            let bt = cubic_point(p0, c0, c1, p1, t);
            let d1 = cubic_deriv(p0, c0, c1, p1, t);
            // Second derivative B''(t).
            let mt = 1.0 - t;
            let d2 = (c1 - c0 * 2.0 + p0) * (6.0 * mt) + (p1 - c1 * 2.0 + c0) * (6.0 * t);
            let off = bt - p;
            let f = off.dot(d1);
            let fp = d1.dot(d1) + off.dot(d2);
            if fp.abs() < 1e-12 {
                break;
            }
            let step = f / fp;
            t -= step;
            if step.abs() < 1e-6 {
                break;
            }
            if !(0.0..=1.0).contains(&t) {
                t = t.clamp(0.0, 1.0);
            }
        }
        consider(t);
    }

    best
}

/// Both signed distances of a point to a single segment (dispatches by type).
#[inline]
fn segment_edge_distance(seg: &Segment, p: Vec2) -> EdgeDistance {
    match *seg {
        Segment::Line { p0, p1 } => line_edge_distance(p0, p1, p),
        Segment::Quad { p0, c, p1 } => quad_edge_distance(p0, c, p1, p),
        Segment::Cubic { p0, c0, c1, p1 } => cubic_edge_distance(p0, c0, c1, p1, p),
    }
}

/// Per-channel + true-SDF signed distances at a single point.
///
/// For each RGB channel the nearest edge is selected by TRUE distance (with the
/// orthogonality tie-break); the stored value is that winning edge's PSEUDO
/// distance. Lines and curves share the metric, so the selection is consistent
/// and curves get the same corner straightening as lines. The A channel is the
/// MTSDF true single-channel SDF: the winner's TRUE (un-extrapolated) signed
/// distance over ALL edges (Decision T2-F).
#[inline]
fn distances_at(edges: &[ColoredEdge], p: Vec2) -> [f32; 4] {
    let mut chan = [EdgeDistance::FAR; 3];
    let mut true_sdf = EdgeDistance::FAR;
    for e in edges {
        let ed = segment_edge_distance(&e.seg, p);
        // True SDF: nearest over ALL edges regardless of color.
        if ed.closer_than(&true_sdf) {
            true_sdf = ed;
        }
        for (c, slot) in chan.iter_mut().enumerate() {
            if e.color.has_channel(c) && ed.closer_than(slot) {
                *slot = ed;
            }
        }
    }
    [
        map_distance(chan[0].pseudo_signed),
        map_distance(chan[1].pseudo_signed),
        map_distance(chan[2].pseudo_signed),
        map_distance(true_sdf.true_sign * true_sdf.true_dist),
    ]
}

/// Fills `rows` (a disjoint vertical slice of the output) at row offset
/// `y_start`. Each task owns its rows exclusively — no shared mutable state.
fn fill_rows(edges: &[ColoredEdge], layout: &FieldLayout, y_start: u32, rows: &mut [f32]) {
    let w = layout.width;
    for (ry, row) in rows.chunks_exact_mut((w * 4) as usize).enumerate() {
        let y = y_start + ry as u32;
        for x in 0..w {
            let p = texel_center(layout, x, y);
            let rgba = distances_at(edges, p);
            let base = (x * 4) as usize;
            row[base] = rgba[0];
            row[base + 1] = rgba[1];
            row[base + 2] = rgba[2];
            row[base + 3] = rgba[3];
        }
    }
}

/// Generates the per-channel signed pseudo-distance field over the expanded
/// transition-band region (T2a). Parallelizes per-texel work by disjoint output
/// rows on `pool` when provided; runs single-threaded (the scalar reference
/// path) when `None`.
pub fn generate_distance_field(colored: &ColoredOutline, pool: Option<&Arc<ThreadPool>>) -> GlyphField {
    let layout = field_layout(colored);
    let w = layout.width;
    let h = layout.height;
    // Widened to u64 before multiplying: `field_layout`'s clamp already makes the `u32` product
    // safe, but this is the allocation the 2026-07 audit found wrapping, so the arithmetic states
    // its own safety instead of depending on a constant three modules away staying small.
    debug_assert!(
        w <= MAX_FIELD_DIM && h <= MAX_FIELD_DIM,
        "field_layout must clamp both dimensions to MAX_FIELD_DIM"
    );
    let texel_count = (w as u64) * (h as u64) * 4;
    let mut texels = vec![0.0_f32; texel_count as usize];

    match pool {
        Some(pool) => {
            // Partition the output rows into bands, one task per band. Each band
            // is a disjoint `&mut [f32]` slice — no aliasing, no atomics.
            let band_rows = pick_band_rows(h, pool.worker_count());
            let row_stride = (w * 4) as usize;
            // `install` (not `scope`): the bake entry point is normally called
            // from an application thread that is NOT already inside a pool
            // `install`/worker frame. `scope` debug-asserts an ambient pool and
            // would panic here; `install` sets the ambient-pool + worker-id TLS
            // for the duration of this call (restored on return and on unwind),
            // so the parallel path works from ANY thread. The closure body is
            // identical to a `scope` body — the disjoint-row partition and the
            // per-texel math are unchanged, so the output is bit-identical to
            // the scalar path.
            pool.install(|scope| {
                let mut y_start = 0u32;
                let mut rest = texels.as_mut_slice();
                while y_start < h {
                    let this_rows = band_rows.min(h - y_start);
                    let split = (this_rows as usize) * row_stride;
                    let (band, tail) = rest.split_at_mut(split);
                    let edges = colored.edges.as_slice();
                    let layout_ref = &layout;
                    scope.spawn(move || {
                        fill_rows(edges, layout_ref, y_start, band);
                    });
                    rest = tail;
                    y_start += this_rows;
                }
            });
        }
        None => {
            fill_rows(&colored.edges, &layout, 0, &mut texels);
        }
    }

    GlyphField {
        width: w,
        height: h,
        texels,
        origin_em: layout.origin_em,
        texel_em: layout.texel_em,
    }
}

/// Chooses a band height (rows per task) so the work splits into roughly
/// `4 × workers` bands for steal balance, with at least one row per band.
#[inline]
fn pick_band_rows(height: u32, workers: u32) -> u32 {
    let target_bands = workers.max(1) * 4;
    (height.div_ceil(target_bands)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f32 = 1e-4;

    #[test]
    fn line_distance_perpendicular_foot_inside() {
        // Point above the middle of a horizontal unit segment: distance 1.0.
        let d = line_edge_distance(Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0), Vec2::new(1.0, 1.0));
        assert!((d.true_dist - 1.0).abs() < TOL, "true dist 1.0, got {}", d.true_dist);
        // foot inside ⇒ pseudo == true (signed); the point is above ⇒ +.
        assert!((d.pseudo_signed.abs() - 1.0).abs() < TOL, "pseudo magnitude 1.0");
    }

    #[test]
    fn line_distance_clamps_true_to_endpoint() {
        // Point beyond the right endpoint: true distance is to the endpoint.
        let d = line_edge_distance(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(3.0, 0.0));
        assert!((d.true_dist - 2.0).abs() < TOL, "true dist to endpoint == 2.0, got {}", d.true_dist);
    }

    #[test]
    fn line_distance_pseudo_extrapolates_past_endpoint() {
        // Beyond the endpoint but ON the infinite line: the pseudo perpendicular
        // distance to the line is ~0 even though the true (clamped) distance is 2.
        let d = line_edge_distance(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(3.0, 0.0));
        assert!(d.pseudo_signed.abs() < TOL, "pseudo (on the infinite line) ≈ 0, got {}", d.pseudo_signed);
        assert!(d.true_dist > d.pseudo_signed.abs(), "pseudo straightens past the corner");
    }

    #[test]
    fn line_distance_degenerate_zero_length() {
        // A zero-length edge is treated as a point.
        let d = line_edge_distance(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(4.0, 5.0));
        let expected = ((4.0_f32 - 1.0).powi(2) + (5.0 - 1.0_f32).powi(2)).sqrt();
        assert!((d.true_dist - expected).abs() < TOL, "distance to the degenerate point");
    }

    #[test]
    fn quad_distance_matches_known_point() {
        // B(t) = (1-t)²p0 + 2(1-t)t·c + t²p1. With p0=(0,0), c=(1,2), p1=(2,0)
        // the apex B(0.5) = (1,1). A query AT the apex must read ~0 distance.
        let d = quad_edge_distance(
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(1.0, 1.0),
        );
        assert!(d.true_dist < 1e-2, "query at the curve apex ⇒ ~0 distance, got {}", d.true_dist);
    }

    #[test]
    fn quad_distance_to_apex_from_above() {
        // From a point one unit above the apex (1,1) ⇒ at (1,2): distance 1.0.
        let d = quad_edge_distance(
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(1.0, 2.0),
        );
        assert!((d.true_dist - 1.0).abs() < 1e-2, "1 unit above the apex ⇒ dist 1.0, got {}", d.true_dist);
    }

    #[test]
    fn quad_distance_endpoint_case() {
        // Query far beyond p1: nearest is the p1 endpoint.
        let p1 = Vec2::new(2.0, 0.0);
        let q = Vec2::new(5.0, 0.0);
        let d = quad_edge_distance(Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), p1, q);
        assert!((d.true_dist - 3.0).abs() < 1e-2, "distance to p1 endpoint ≈ 3.0, got {}", d.true_dist);
    }

    #[test]
    fn cubic_distance_matches_bruteforce_on_s_curve() {
        // Independent dense-sample brute force for an S-shaped cubic (the local
        // minimum trap multi-seed Newton handles).
        let p0 = Vec2::new(0.0, 0.0);
        let c0 = Vec2::new(1.0, 0.2);
        let c1 = Vec2::new(0.0, 0.8);
        let p1 = Vec2::new(1.0, 1.0);
        let probes = [
            Vec2::new(0.5, 0.5),
            Vec2::new(0.2, 0.8),
            Vec2::new(0.8, 0.2),
            Vec2::new(0.5, 0.0),
        ];
        for q in probes {
            let got = cubic_edge_distance(p0, c0, c1, p1, q).true_dist;
            let mut brute = f32::INFINITY;
            for i in 0..=50_000u32 {
                let t = i as f32 / 50_000.0;
                let pt = cubic_point(p0, c0, c1, p1, t);
                let d = (pt - q).length();
                if d < brute {
                    brute = d;
                }
            }
            assert!((got - brute).abs() < 2e-3, "cubic dist {} vs brute {} at {:?}", got, brute, q);
        }
    }

    #[test]
    fn solve_cubic_finds_linear_factor_roots() {
        // (t-1)(t-2)(t-3) = t³ - 6t² + 11t - 6.
        let mut out = [0.0_f32; 3];
        let n = solve_cubic(1.0, -6.0, 11.0, -6.0, &mut out);
        assert_eq!(n, 3, "three distinct real roots");
        let mut roots = out;
        roots.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((roots[0] - 1.0).abs() < 1e-3, "root 1");
        assert!((roots[1] - 2.0).abs() < 1e-3, "root 2");
        assert!((roots[2] - 3.0).abs() < 1e-3, "root 3");
    }

    #[test]
    fn solve_cubic_single_real_root() {
        // t³ + t = 0 has one real root at 0 (others complex).
        let mut out = [0.0_f32; 3];
        let n = solve_cubic(1.0, 0.0, 1.0, 0.0, &mut out);
        assert_eq!(n, 1, "one real root");
        assert!(out[0].abs() < 1e-3, "the real root is 0, got {}", out[0]);
    }

    #[test]
    fn solve_cubic_degrades_to_quadratic() {
        // a=0 ⇒ quadratic t² - 3t + 2 = (t-1)(t-2).
        let mut out = [0.0_f32; 3];
        let n = solve_cubic(0.0, 1.0, -3.0, 2.0, &mut out);
        assert_eq!(n, 2, "two roots from the quadratic fallback");
        let mut r = [out[0], out[1]];
        r.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((r[0] - 1.0).abs() < 1e-3 && (r[1] - 2.0).abs() < 1e-3, "roots 1 and 2");
    }

    #[test]
    fn solve_quadratic_no_real_roots() {
        // t² + 1 = 0 ⇒ no real roots.
        let mut out = [0.0_f32; 3];
        let n = solve_quadratic(1.0, 0.0, 1.0, &mut out);
        assert_eq!(n, 0, "negative discriminant ⇒ zero real roots");
    }

    #[test]
    fn solve_quadratic_linear_fallback() {
        // a=0 ⇒ linear 2t - 4 = 0 ⇒ t=2.
        let mut out = [0.0_f32; 3];
        let n = solve_quadratic(0.0, 2.0, -4.0, &mut out);
        assert_eq!(n, 1, "linear fallback yields one root");
        assert!((out[0] - 2.0).abs() < 1e-4, "root == 2");
    }

    #[test]
    fn closer_than_prefers_nearer_true_distance() {
        let near = EdgeDistance { true_dist: 1.0, ..EdgeDistance::FAR };
        let far = EdgeDistance { true_dist: 2.0, ..EdgeDistance::FAR };
        assert!(near.closer_than(&far), "nearer true distance wins");
        assert!(!far.closer_than(&near));
    }

    #[test]
    fn closer_than_breaks_ties_by_orthogonality() {
        let ortho = EdgeDistance { true_dist: 1.0, orthogonality: 0.9, ..EdgeDistance::FAR };
        let oblique = EdgeDistance { true_dist: 1.0, orthogonality: 0.1, ..EdgeDistance::FAR };
        assert!(ortho.closer_than(&oblique), "on a tie, more-orthogonal wins");
    }

    #[test]
    fn pick_band_rows_at_least_one() {
        assert_eq!(pick_band_rows(0, 8), 1, "never zero rows per band");
        assert_eq!(pick_band_rows(1, 8), 1);
        assert!(pick_band_rows(1000, 4) >= 1);
    }
}
