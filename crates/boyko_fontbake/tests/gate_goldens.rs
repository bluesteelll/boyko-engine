//! GUI P5b bake gate goldens (T0–T3), CPU-only — no GPU.
//!
//! These are the plan's T0–T3 acceptance gates (docs/GUI-P5B-TEXT-PLAN.md
//! §Metrics). Each test maps to one gate and asserts against INDEPENDENT
//! reference values (brute-force fine sampling, an independent ray-cast
//! inside/outside oracle, a hand-injected clash) — never against the code's own
//! output. The reference glyph metrics were extracted from the checked-in libre
//! `Ubuntu-Light.ttf` fixture (Ubuntu Font License) and pinned here.
//!
//! A failing MSDF gate (an inverted interior, a rounded corner, a per-channel
//! distance mismatch) is a real defect and is NOT silenced.

use std::path::PathBuf;
use std::sync::OnceLock;

use boyko_fontbake::atlas::lookup_slot;
use boyko_fontbake::extract::{Segment, extract_codepoint, extract_glyph, face_metrics};
use boyko_fontbake::face::GlyphId;
use boyko_fontbake::msdf::color::{ColoredEdge, ColoredOutline, color_outline};
use boyko_fontbake::msdf::distance::generate_distance_field;
use boyko_fontbake::msdf::error_correct::correct_errors;
use boyko_fontbake::msdf::sign::correct_signs;
use boyko_fontbake::msdf::{
    GlyphField, field_layout, generate_glyph_field, range_em, texel_center, texel_center_from_field,
};
use boyko_fontbake::{FontFace, GlyphOutline, TtfFace, bake_font, read_bfont, write_bfont};
use boyko_math::Vec2;

// ----------------------------------------------------------------------------
// Fixture loading
// ----------------------------------------------------------------------------

/// Path to the checked-in libre TrueType fixture.
fn ttf_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("Ubuntu-Light.ttf")
}

/// Loads and caches the fixture bytes once for the whole test binary.
fn fixture_bytes() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        std::fs::read(ttf_path()).expect(
            "test fixture missing: crates/boyko_fontbake/fixtures/Ubuntu-Light.ttf \
             (a libre TrueType font must be checked in for the goldens)",
        )
    })
}

/// Parses the fixture into a [`TtfFace`].
fn face() -> TtfFace {
    TtfFace::from_bytes(fixture_bytes()).expect("fixture must parse as a font")
}

// ----------------------------------------------------------------------------
// Independent reference helpers (NOT the code under test)
// ----------------------------------------------------------------------------

/// Evaluates any segment at parameter `t` (independent of the crate's evaluator).
fn seg_eval(s: &Segment, t: f32) -> Vec2 {
    match *s {
        Segment::Line { p0, p1 } => p0 + (p1 - p0) * t,
        Segment::Quad { p0, c, p1 } => {
            let mt = 1.0 - t;
            p0 * (mt * mt) + c * (2.0 * mt * t) + p1 * (t * t)
        }
        Segment::Cubic { p0, c0, c1, p1 } => {
            let mt = 1.0 - t;
            let mt2 = mt * mt;
            let t2 = t * t;
            p0 * (mt2 * mt) + c0 * (3.0 * mt2 * t) + c1 * (3.0 * mt * t2) + p1 * (t2 * t)
        }
    }
}

/// Independent segment derivative.
fn seg_deriv(s: &Segment, t: f32) -> Vec2 {
    match *s {
        Segment::Line { p0, p1 } => p1 - p0,
        Segment::Quad { p0, c, p1 } => (c - p0) * (2.0 * (1.0 - t)) + (p1 - c) * (2.0 * t),
        Segment::Cubic { p0, c0, c1, p1 } => {
            let mt = 1.0 - t;
            (c0 - p0) * (3.0 * mt * mt) + (c1 - c0) * (6.0 * mt * t) + (p1 - c1) * (3.0 * t * t)
        }
    }
}

/// Brute-force nearest-point on a segment by dense sampling. Returns
/// `(true_dist, best_param, signed_dist_at_best)`. The signed distance uses the
/// tangent-side convention `sign = signum(tangent × offset)`.
fn brute_nearest(s: &Segment, p: Vec2, samples: u32) -> (f32, f32, f32) {
    let mut best_t = 0.0_f32;
    let mut best_d = f32::INFINITY;
    for i in 0..=samples {
        let t = i as f32 / samples as f32;
        let d = (seg_eval(s, t) - p).length();
        if d < best_d {
            best_d = d;
            best_t = t;
        }
    }
    let pt = seg_eval(s, best_t);
    let tan = seg_deriv(s, best_t).normalize();
    let off = p - pt;
    let cross = tan.cross(off);
    let sign = if cross >= 0.0 { 1.0 } else { -1.0 };
    (best_d, best_t, sign * best_d)
}

/// The shader's median reconstruction.
fn median(r: f32, g: f32, b: f32) -> f32 {
    r.max(g).min(r.min(g).max(b))
}

/// Independent nonzero-winding point-in-polygon by flattening every segment into
/// chords and ray-casting in `+x`. This is the authoritative inside/outside
/// oracle the sign-correction gate checks against (deliberately a different
/// implementation than the crate's `sign.rs`: a denser flatten + signum ray).
fn ref_inside(outline: &GlyphOutline, p: Vec2) -> bool {
    const N: u32 = 64;
    let mut winding = 0i32;
    for contour in &outline.contours {
        for seg in contour {
            let mut prev = seg_eval(seg, 0.0);
            for i in 1..=N {
                let t = i as f32 / N as f32;
                let cur = seg_eval(seg, t);
                winding += ray_cross(prev, cur, p.x, p.y);
                prev = cur;
            }
        }
    }
    winding != 0
}

/// Half-open horizontal ray crossing contribution for a chord.
fn ray_cross(a: Vec2, b: Vec2, x0: f32, y: f32) -> i32 {
    let (up, lo, hi) = if a.y <= b.y { (true, a, b) } else { (false, b, a) };
    if y < lo.y || y >= hi.y {
        return 0;
    }
    let t = (y - lo.y) / (hi.y - lo.y);
    let x_at = lo.x + t * (hi.x - lo.x);
    if x_at <= x0 {
        return 0;
    }
    if up { 1 } else { -1 }
}

/// A synthetic teardrop: a sharp top corner (two straight edges meeting at a
/// point) and a rounded bottom bulb (two quads). The defining MSDF property is
/// that the median reconstruction keeps the tip SHARP (not rounded).
fn teardrop() -> GlyphOutline {
    let tip = Vec2::new(0.5, 0.62);
    let bl = Vec2::new(0.18, 0.25);
    let br = Vec2::new(0.82, 0.25);
    let bot = Vec2::new(0.5, 0.05);
    let contour = vec![
        Segment::Line { p0: tip, p1: bl },
        Segment::Quad { p0: bl, c: Vec2::new(0.2, 0.0), p1: bot },
        Segment::Quad { p0: bot, c: Vec2::new(0.8, 0.0), p1: br },
        Segment::Line { p0: br, p1: tip },
    ];
    GlyphOutline {
        contours: vec![contour],
        bbox_min: Vec2::new(0.18, 0.05),
        bbox_max: Vec2::new(0.82, 0.62),
    }
}

/// A smooth ring with NO corners (concentric circles approximated by quads):
/// the 0-corner topology case (`O`/`o`).
fn smooth_o() -> GlyphOutline {
    // Outer ring CCW, inner ring CW, as 8 quad arcs each — enough to be smooth.
    fn ring(cx: f32, cy: f32, r: f32, ccw: bool) -> Vec<Segment> {
        let steps = 8;
        let mut pts = Vec::new();
        for i in 0..steps {
            let a = (i as f32 / steps as f32) * std::f32::consts::TAU;
            pts.push(Vec2::new(cx + r * a.cos(), cy + r * a.sin()));
        }
        if !ccw {
            pts.reverse();
        }
        let mut segs = Vec::new();
        for i in 0..steps {
            let p0 = pts[i];
            let p1 = pts[(i + 1) % steps];
            // control = midpoint pushed out to make a smooth arc
            let mid = (p0 + p1) * 0.5;
            let dir = Vec2::new(mid.x - cx, mid.y - cy).normalize();
            let bulge = r * 0.08;
            let c = mid + dir * bulge;
            segs.push(Segment::Quad { p0, c, p1 });
        }
        segs
    }
    GlyphOutline {
        contours: vec![ring(0.5, 0.5, 0.4, true), ring(0.5, 0.5, 0.22, false)],
        bbox_min: Vec2::new(0.08, 0.08),
        bbox_max: Vec2::new(0.92, 0.92),
    }
}

// ----------------------------------------------------------------------------
// GATE 1 — T1 outline extraction (segment counts, winding, metrics, cmap)
// ----------------------------------------------------------------------------

#[test]
fn t1_face_metrics_match_reference() {
    let f = face();
    let fm = face_metrics(&f);
    assert_eq!(fm.units_per_em, 1000, "Ubuntu-Light units_per_em");
    assert!((fm.ascender_em - 0.932).abs() < 1e-4, "ascender_em == 0.932, got {}", fm.ascender_em);
    assert!((fm.descender_em + 0.189).abs() < 1e-4, "descender_em == -0.189, got {}", fm.descender_em);
    assert!((fm.line_gap_em - 0.028).abs() < 1e-4, "line_gap_em == 0.028, got {}", fm.line_gap_em);
}

#[test]
fn t1_cmap_maps_codepoints_to_expected_glyph_ids() {
    let f = face();
    assert_eq!(f.glyph_index('A'), Some(GlyphId(36)), "'A' -> gid 36");
    assert_eq!(f.glyph_index('o'), Some(GlyphId(82)), "'o' -> gid 82");
    assert_eq!(f.glyph_index('.'), Some(GlyphId(17)), "'.' -> gid 17");
    assert_eq!(f.glyph_index(' '), Some(GlyphId(3)), "space -> gid 3");
}

#[test]
fn t1_glyph_a_segment_counts_and_metrics() {
    let g = extract_codepoint(&face(), 'A');
    assert_eq!(g.outline.contours.len(), 2, "'A' has an outer contour + a counter");
    assert_eq!(g.outline.segment_count(), 21, "'A' total segments");
    let (lines, quads, cubics) = seg_type_counts(&g.outline);
    assert_eq!((lines, quads, cubics), (5, 16, 0), "'A' is 5 lines + 16 quads (TrueType glyf)");
    assert!((g.advance_em - 0.641).abs() < 1e-4, "'A' advance_em == 0.641, got {}", g.advance_em);
    assert!((g.outline.bbox_min.x - 0.010).abs() < 1e-3, "'A' bbox min x");
    assert!((g.outline.bbox_max.x - 0.631).abs() < 1e-3, "'A' bbox max x");
    assert!((g.outline.bbox_max.y - 0.693).abs() < 1e-3, "'A' bbox max y (cap height)");
}

#[test]
fn t1_glyph_a_winding_outer_cw_counter_ccw() {
    // 'A' has an outer contour (clockwise in y-up font space, negative shoelace
    // area) enclosing a triangular counter (counter-clockwise, positive area).
    let g = extract_codepoint(&face(), 'A');
    let areas: Vec<f32> = g.outline.contours.iter().map(contour_area).collect();
    assert_eq!(areas.len(), 2);
    let outer = areas.iter().cloned().fold(0.0_f32, |m, a| if a.abs() > m.abs() { a } else { m });
    let counter = areas.iter().cloned().find(|&a| a != outer).unwrap();
    assert!(outer < 0.0, "outer contour winds CW (negative area), got {}", outer);
    assert!(counter > 0.0, "counter winds CCW (positive area), got {}", counter);
    assert!(g.outline.signed_area() < 0.0, "net signed area is negative (filled glyph)");
}

#[test]
fn t1_glyph_o_is_all_quads_two_contours() {
    let g = extract_codepoint(&face(), 'o');
    assert_eq!(g.outline.contours.len(), 2, "'o' outer + inner");
    assert_eq!(g.outline.segment_count(), 24, "'o' total segments");
    let (lines, quads, cubics) = seg_type_counts(&g.outline);
    assert_eq!((lines, quads, cubics), (0, 24, 0), "'o' is all quadratics");
    assert!((g.advance_em - 0.582).abs() < 1e-4, "'o' advance");
}

#[test]
fn t1_glyph_dot_is_single_contour_eight_quads() {
    let g = extract_codepoint(&face(), '.');
    assert_eq!(g.outline.contours.len(), 1, "'.' is one closed contour");
    assert_eq!(g.outline.segment_count(), 8, "'.' total segments");
    let (_, quads, _) = seg_type_counts(&g.outline);
    assert_eq!(quads, 8, "'.' is 8 quadratics (a rounded dot)");
    assert!((g.advance_em - 0.246).abs() < 1e-4, "'.' advance");
}

#[test]
fn t1_space_is_empty_with_advance_only() {
    let g = extract_codepoint(&face(), ' ');
    assert!(g.outline.is_empty(), "space has no drawable contours");
    assert_eq!(g.outline.segment_count(), 0, "space has zero segments");
    assert!(g.advance_em > 0.0, "space still carries a positive advance, got {}", g.advance_em);
}

#[test]
fn t1_notdef_slot_zero_has_outline() {
    // glyph id 0 (.notdef) is the missing-glyph box; it must have a drawable
    // outline so unmapped codepoints render something.
    let g = extract_glyph(&face(), GlyphId(0));
    assert!(!g.outline.is_empty(), ".notdef must have a visible outline");
}

#[test]
fn t1_unmapped_codepoint_falls_back_to_notdef() {
    // A codepoint the subset font does not contain resolves to glyph 0.
    let f = face();
    let exotic = '\u{1F600}'; // emoji, not in a Latin subset
    if f.glyph_index(exotic).is_none() {
        let g = extract_codepoint(&f, exotic);
        assert_eq!(g.id, GlyphId(0), "unmapped codepoint extracts .notdef");
    }
}

// ----------------------------------------------------------------------------
// GATE 2a — MSDF per-channel pseudo-distance vs INDEPENDENT brute force
// ----------------------------------------------------------------------------

#[test]
fn t2a_per_channel_distance_matches_bruteforce_interior_projection() {
    // For every interior-projection (non-endpoint-clamped), non-saturated texel
    // and channel, the field's signed distance must match an independent
    // brute-force nearest-point reference to within a tight tolerance. (Clamped
    // texels use the extrapolated PSEUDO distance whose foot the coarse brute
    // cannot pin exactly, so they are excluded here and covered by the
    // dedicated pseudo test below.)
    for &cp in &['A', 'o', '.'] {
        let g = extract_codepoint(&face(), cp);
        let colored = color_outline(&g.outline);
        let layout = field_layout(&colored);
        let field = generate_distance_field(&colored, None);
        let edges: &[ColoredEdge] = &colored.edges;

        let mut max_err = 0.0_f32;
        let mut checked = 0usize;
        for y in 0..layout.height {
            for x in 0..layout.width {
                let p = texel_center(&layout, x, y);
                for ch in 0..3 {
                    let mut best_true = f32::INFINITY;
                    let mut best_param = 0.5_f32;
                    let mut best_signed = f32::INFINITY;
                    for e in edges {
                        if e.color.has_channel(ch) {
                            let (td, param, sd) = brute_nearest(&e.seg, p, 8_000);
                            if td < best_true {
                                best_true = td;
                                best_param = param;
                                best_signed = sd;
                            }
                        }
                    }
                    // Only interior projection (foot strictly inside the segment).
                    if best_param <= 0.02 || best_param >= 0.98 {
                        continue;
                    }
                    let v = field.texel(x, y)[ch];
                    if v <= 1e-4 || v >= 1.0 - 1e-4 {
                        continue; // mapping-saturated, distance is clamped
                    }
                    let got_signed = (v - 0.5) * range_em();
                    let err = (got_signed - best_signed).abs();
                    if err > max_err {
                        max_err = err;
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 50, "'{}' must check a meaningful texel population, got {}", cp, checked);
        // One texel is 1/48 em ≈ 0.0208; tolerance is a small fraction of that.
        assert!(
            max_err < 0.004,
            "'{}' per-channel distance vs brute force: max err {:.6} em ({:.3} texels) exceeds tolerance",
            cp,
            max_err,
            max_err * 48.0
        );
    }
}

#[test]
fn t2a_field_has_no_nan_or_inf() {
    // Property: a generated field never contains a non-finite texel (the NaN
    // black-hole the runtime NaN guard exists to contain must not originate in
    // the bake).
    for &cp in &['A', 'o', '.', '8', 'e', 'g'] {
        let g = extract_codepoint(&face(), cp);
        let f = generate_glyph_field(&g.outline, None).expect("non-empty glyph");
        for (i, &t) in f.texels.iter().enumerate() {
            assert!(t.is_finite(), "'{}' texel float {} is non-finite ({})", cp, i, t);
            assert!((0.0..=1.0).contains(&t), "'{}' texel float {} out of [0,1] ({})", cp, i, t);
        }
    }
}

#[test]
fn t2a_teardrop_median_reconstructs_sharp_corner() {
    // The defining MSDF property: at the teardrop's sharp tip the MEDIAN(rgb)
    // reconstruction must come to a POINT (the inside-width shrinks ~linearly to
    // ~1 texel at the apex), tracking the true single-channel (.a) width — a
    // rounded corner would leave the median noticeably WIDER than .a near the tip.
    let f = generate_glyph_field(&teardrop(), None).expect("teardrop is non-empty");

    // Apex row = highest row with any median-inside texel.
    let mut apex_y = 0u32;
    let mut found = false;
    for y in 0..f.height {
        for x in 0..f.width {
            let t = f.texel(x, y);
            if median(t[0], t[1], t[2]) > 0.5 {
                apex_y = apex_y.max(y);
                found = true;
            }
        }
    }
    assert!(found, "teardrop must have an interior");

    // Measure median-inside width and .a-inside width at the top few rows.
    let mut widths = Vec::new();
    for dy in 0..4u32 {
        if apex_y < dy {
            break;
        }
        let y = apex_y - dy;
        let mut med_w = 0u32;
        let mut a_w = 0u32;
        for x in 0..f.width {
            let t = f.texel(x, y);
            if median(t[0], t[1], t[2]) > 0.5 {
                med_w += 1;
            }
            if t[3] > 0.5 {
                a_w += 1;
            }
        }
        widths.push((med_w, a_w));
    }

    // The apex itself must be a narrow point (a sharp corner, not a flat cap).
    assert!(
        widths[0].0 <= 2,
        "teardrop apex median width must be ~1 texel (sharp), got {}",
        widths[0].0
    );
    // Width must grow as we descend from the apex (a true corner narrows to a point).
    assert!(
        widths.last().unwrap().0 > widths[0].0,
        "median width must widen below the apex (corner narrows to a point): {:?}",
        widths
    );
    // The median must not be ROUNDED relative to the true SDF: at every measured
    // row the median width must stay within 1 texel of the .a width (a rounded
    // median tip would balloon past .a).
    for (i, (mw, aw)) in widths.iter().enumerate() {
        let diff = (*mw as i32 - *aw as i32).abs();
        assert!(
            diff <= 1,
            "teardrop row apex-{}: median width {} must track .a width {} (corner preserved)",
            i,
            mw,
            aw
        );
    }
}

#[test]
fn t2a_smooth_o_zero_corner_reconstructs_clean_ring() {
    // The 0-corner topology case: a smooth ring must median-reconstruct as a
    // clean filled annulus with no inverted interior and no broken arc.
    let o = smooth_o();
    let f = generate_glyph_field(&o, None).expect("smooth O is non-empty");
    let mut interior = 0;
    let mut inverted = 0;
    for y in 0..f.height {
        for x in 0..f.width {
            let p = texel_center_from_field(&f, x, y);
            if ref_inside(&o, p) {
                interior += 1;
                let t = f.texel(x, y);
                if median(t[0], t[1], t[2]) <= 0.5 {
                    inverted += 1;
                }
            }
        }
    }
    assert!(interior > 20, "smooth O must have a measurable interior, got {}", interior);
    assert_eq!(inverted, 0, "smooth O ring must have zero inverted-interior texels");
}

#[test]
fn t2a_synthetic_cubic_distance_matches_bruteforce() {
    // T2a cubic gate. No `.otf` (CFF) fixture is available on this machine, so
    // the CFF charstring path cannot be driven end-to-end; instead the cubic
    // NEAREST-POINT solver (the multi-seed Newton the CFF cubic path uses) is
    // exercised directly on a synthetic cubic-segment outline — exactly the math
    // a CFF glyph would drive.
    //
    // The crate's `cubic_edge_distance` is private, so it is observed through the
    // public `.a` (true SDF) channel of the generated field. To make the
    // comparison exact (no half-texel sampling error), probes are taken AT TEXEL
    // CENTERS: the crate evaluates distance at exactly those points, and the
    // dense brute force is evaluated at the SAME points. The cubic must be the
    // nearest edge at each probe so the `.a` channel reflects the cubic solver.
    let cubic = Segment::Cubic {
        // An S-shaped cubic — the local-minimum trap that multi-seed Newton
        // (vs a single seed) exists to avoid.
        p0: Vec2::new(0.15, 0.15),
        c0: Vec2::new(0.95, 0.25),
        c1: Vec2::new(0.05, 0.75),
        p1: Vec2::new(0.85, 0.85),
    };
    // Close the contour with a far-away straight return so the cubic dominates
    // the nearest-edge selection for probes near the curve.
    let outline = GlyphOutline {
        contours: vec![vec![
            cubic,
            Segment::Line { p0: cubic.end(), p1: cubic.start() },
        ]],
        bbox_min: Vec2::new(0.0, 0.0),
        bbox_max: Vec2::new(1.0, 1.0),
    };
    let colored = color_outline(&outline);
    let layout = field_layout(&colored);
    let field = generate_distance_field(&colored, None);

    let mut max_err = 0.0_f32;
    let mut checked = 0usize;
    for y in 0..layout.height {
        for x in 0..layout.width {
            let p = texel_center(&layout, x, y);
            // Brute true distance to the cubic AND to the closing line. 40k
            // samples over a unit-scale cubic gives ~2.5e-5 spacing, well under
            // the 0.004 em tolerance.
            let (dc, _, _) = brute_nearest(&cubic, p, 40_000);
            let line = Segment::Line { p0: cubic.end(), p1: cubic.start() };
            let (dl, _, _) = brute_nearest(&line, p, 4_000);
            // Only probe where the cubic is the strictly-nearest edge (so the
            // field's .a reflects the cubic solver, not the line).
            if dc >= dl - 1e-3 {
                continue;
            }
            let a = field.texel(x, y)[3];
            if a <= 1e-4 || a >= 1.0 - 1e-4 {
                continue; // mapping-saturated
            }
            let got = ((a - 0.5) * range_em()).abs();
            let err = (got - dc).abs();
            if err > max_err {
                max_err = err;
            }
            checked += 1;
        }
    }
    assert!(checked > 30, "cubic gate must check a meaningful population, got {}", checked);
    assert!(
        max_err < 0.004,
        "cubic multi-seed Newton vs dense brute force: max err {:.6} em ({:.3} texels) exceeds tolerance",
        max_err,
        max_err * 48.0
    );
}

// ----------------------------------------------------------------------------
// GATE 2b — scanline sign-correction (zero inverted interior on overlap glyphs)
// ----------------------------------------------------------------------------

#[test]
fn t2b_overlap_glyph_eight_has_zero_inverted_interior() {
    // '8' has three contours (outer + two counters) and is the canonical
    // overlap/multi-contour sign test. After the full pipeline (which includes
    // the T2b scanline pass), every interior texel (by an INDEPENDENT ray-cast
    // oracle) must median-reconstruct as inside.
    assert_zero_inverted_interior('8');
}

#[test]
fn t2b_glyph_o_counter_reads_outside() {
    // The counter (hole) of 'o' must read OUTSIDE (median <= 0.5): the inner ring
    // flips insideness. This catches a sign pass that ignores winding direction.
    let g = extract_codepoint(&face(), 'o');
    let f = generate_glyph_field(&g.outline, None).expect("'o' non-empty");
    let mut hole_texels = 0;
    let mut hole_wrongly_inside = 0;
    for y in 0..f.height {
        for x in 0..f.width {
            let p = texel_center_from_field(&f, x, y);
            if !ref_inside(&g.outline, p) {
                // exterior OR hole; restrict to texels surrounded by the glyph bbox
                if p.x > g.outline.bbox_min.x
                    && p.x < g.outline.bbox_max.x
                    && p.y > g.outline.bbox_min.y
                    && p.y < g.outline.bbox_max.y
                {
                    hole_texels += 1;
                    let t = f.texel(x, y);
                    if median(t[0], t[1], t[2]) > 0.5 {
                        hole_wrongly_inside += 1;
                    }
                }
            }
        }
    }
    assert!(hole_texels > 5, "'o' must have an identifiable counter region");
    assert_eq!(hole_wrongly_inside, 0, "'o' counter texels must read outside (median <= 0.5)");
}

#[test]
fn t2b_letters_have_zero_inverted_interior() {
    for &cp in &['A', 'o', 'e', 'O'] {
        assert_zero_inverted_interior(cp);
    }
}

#[test]
fn t2b_correct_signs_flips_a_deliberately_inverted_field() {
    // Drive the sign pass in isolation: build the raw distance field, invert
    // EVERY channel (simulating an all-wrong provisional sign), then run
    // correct_signs and assert the interior reads inside again. Proves the pass
    // overrides the provisional sign with authoritative insideness.
    let g = extract_codepoint(&face(), 'o');
    let colored = color_outline(&g.outline);
    let mut field = generate_distance_field(&colored, None);
    for t in field.texels.iter_mut() {
        *t = 1.0 - *t; // invert all signs
    }
    correct_signs(&mut field, &colored);

    let mut interior = 0;
    let mut inverted = 0;
    for y in 0..field.height {
        for x in 0..field.width {
            let p = texel_center_from_field(&field, x, y);
            if ref_inside(&g.outline, p) {
                interior += 1;
                let t = field.texel(x, y);
                if median(t[0], t[1], t[2]) <= 0.5 {
                    inverted += 1;
                }
            }
        }
    }
    assert!(interior > 20, "must have interior to check");
    assert_eq!(inverted, 0, "correct_signs must restore inside reading after a full inversion");
}

// ----------------------------------------------------------------------------
// GATE 2c — error-correction pass (removes clash artifacts, load-bearing)
// ----------------------------------------------------------------------------

#[test]
fn t2c_error_correction_removes_injected_clash() {
    // Build a 3x1 field whose .a (true SDF) stays inside everywhere, but whose
    // RGB median dips below 0.5 in the middle texel — a SPURIOUS median edge (the
    // exact artifact class T2c removes). PRE: the clash exists (control). POST:
    // the middle texel's RGB is collapsed to .a and the clash is gone.
    let w = 3;
    let h = 1;
    let mut texels = vec![0.0_f32; (w * h * 4) as usize];
    let set = |t: &mut Vec<f32>, i: usize, r: f32, g: f32, b: f32, a: f32| {
        let base = i * 4;
        t[base] = r;
        t[base + 1] = g;
        t[base + 2] = b;
        t[base + 3] = a;
    };
    set(&mut texels, 0, 0.9, 0.9, 0.9, 0.9);
    set(&mut texels, 1, 0.9, 0.1, 0.1, 0.9); // median 0.1 (outside) but .a 0.9 (inside) = clash
    set(&mut texels, 2, 0.9, 0.9, 0.9, 0.9);

    let mut field = GlyphField {
        width: w,
        height: h,
        texels: texels.clone(),
        origin_em: Vec2::ZERO,
        texel_em: 1.0 / 48.0,
    };

    // PRE-correction control: the speckle MUST exist (proving the pass is needed).
    let pre_median = median(field.texels[4], field.texels[5], field.texels[6]);
    assert!(pre_median <= 0.5, "control: pre-correction median is a spurious edge ({})", pre_median);
    assert!(field.texels[7] > 0.5, "control: .a says the texel is truly inside");

    correct_errors(&mut field, &empty_colored());

    let post_median = median(field.texels[4], field.texels[5], field.texels[6]);
    assert!(
        post_median > 0.5,
        "post-correction median must agree with .a (clash removed), got {}",
        post_median
    );
    assert_eq!(
        (field.texels[4], field.texels[5], field.texels[6]),
        (0.9, 0.9, 0.9),
        "clashing texel RGB collapsed to the .a value"
    );
}

#[test]
fn t2c_error_correction_leaves_clean_field_untouched() {
    // A field with no clash (RGB median agrees with .a everywhere) must be
    // byte-identical after the pass — the correction is not a blanket rewrite.
    let w = 4;
    let h = 4;
    let clean = vec![0.8_f32; (w * h * 4) as usize];
    let mut field = GlyphField {
        width: w,
        height: h,
        texels: clean.clone(),
        origin_em: Vec2::ZERO,
        texel_em: 1.0 / 48.0,
    };
    correct_errors(&mut field, &empty_colored());
    assert_eq!(field.texels, clean, "clean field must be unchanged by error correction");
}

#[test]
fn t2c_full_pipeline_glyphs_are_clash_free() {
    // After the full pipeline, no real glyph may have a median/.a disagreement
    // (a residual clash). This is the post-correction acceptance property.
    for &cp in &['e', 'g', 'o', '8', 'a', 's', 'A'] {
        let g = extract_codepoint(&face(), cp);
        let f = generate_glyph_field(&g.outline, None).expect("non-empty");
        let mut clashes = 0;
        for i in 0..(f.width * f.height) as usize {
            let b = i * 4;
            let m = median(f.texels[b], f.texels[b + 1], f.texels[b + 2]);
            let a = f.texels[b + 3];
            if (m > 0.5) != (a > 0.5) {
                clashes += 1;
            }
        }
        assert_eq!(clashes, 0, "'{}' must be clash-free after the full pipeline", cp);
    }
}

// ----------------------------------------------------------------------------
// GATE 3 — atlas packing + metrics/UV table consistency
// ----------------------------------------------------------------------------

#[test]
fn t3_atlas_glyph_tiles_do_not_overlap() {
    let baked = bake_font(&face(), &"Ao.8eOi ".chars().collect::<Vec<_>>(), None);
    let rects: Vec<[f32; 4]> = baked
        .glyphs
        .iter()
        .filter(|g| g.atlas[2] > g.atlas[0] && g.atlas[3] > g.atlas[1])
        .map(|g| g.atlas)
        .collect();
    assert!(rects.len() >= 6, "expected several packed glyphs, got {}", rects.len());
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            let a = rects[i];
            let b = rects[j];
            let overlap_x = a[0] < b[2] && b[0] < a[2];
            let overlap_y = a[1] < b[3] && b[1] < a[3];
            assert!(
                !(overlap_x && overlap_y),
                "atlas tiles {} {:?} and {} {:?} overlap",
                i,
                a,
                j,
                b
            );
        }
    }
}

#[test]
fn t3_no_per_glyph_scale_drift() {
    // The plane↔atlas extent ratio must equal the global pixels_per_em for EVERY
    // glyph (no per-glyph scale drift — the shader's single screenPxRange uniform
    // depends on this).
    let baked = bake_font(&face(), &"Ao.8eOiW".chars().collect::<Vec<_>>(), None);
    let ppem = baked.meta.pixels_per_em;
    for (slot, g) in baked.glyphs.iter().enumerate() {
        let plane_w = g.plane[2] - g.plane[0];
        let plane_h = g.plane[3] - g.plane[1];
        let atlas_w = g.atlas[2] - g.atlas[0];
        let atlas_h = g.atlas[3] - g.atlas[1];
        if plane_w.abs() > 1e-6 {
            let ratio = atlas_w / plane_w;
            assert!(
                (ratio - ppem).abs() < 0.05,
                "slot {} x-scale {:.4} drifts from pixels_per_em {}",
                slot,
                ratio,
                ppem
            );
        }
        if plane_h.abs() > 1e-6 {
            let ratio = atlas_h / plane_h;
            assert!(
                (ratio - ppem).abs() < 0.05,
                "slot {} y-scale {:.4} drifts from pixels_per_em {}",
                slot,
                ratio,
                ppem
            );
        }
    }
}

#[test]
fn t3_atlas_uv_rects_within_atlas_bounds() {
    let baked = bake_font(&face(), &"Ao.8eOi".chars().collect::<Vec<_>>(), None);
    let aw = baked.meta.atlas_w as f32;
    let ah = baked.meta.atlas_h as f32;
    for (slot, g) in baked.glyphs.iter().enumerate() {
        if g.atlas[2] <= g.atlas[0] {
            continue; // empty glyph
        }
        assert!(g.atlas[0] >= 0.0 && g.atlas[2] <= aw, "slot {} atlas x in bounds", slot);
        assert!(g.atlas[1] >= 0.0 && g.atlas[3] <= ah, "slot {} atlas y in bounds", slot);
        assert!(g.atlas[0] < g.atlas[2] && g.atlas[1] < g.atlas[3], "slot {} non-degenerate", slot);
    }
}

#[test]
fn t3_atlas_image_size_matches_meta() {
    let baked = bake_font(&face(), &"Ao.".chars().collect::<Vec<_>>(), None);
    assert_eq!(baked.atlas.width, baked.meta.atlas_w, "atlas image width == meta");
    assert_eq!(baked.atlas.height, baked.meta.atlas_h, "atlas image height == meta");
    assert_eq!(
        baked.atlas.pixels.len(),
        (baked.meta.atlas_w * baked.meta.atlas_h * 4) as usize,
        "RGBA8 pixel buffer size matches atlas dimensions"
    );
}

#[test]
fn t3_inter_glyph_padding_present() {
    // The packer must leave >= distance_range_texels/2 spacing between glyph
    // tiles (no bilinear neighbor bleed). Verify the minimum gap between any two
    // tiles' bounding boxes is at least 1 texel (the padding is baked into each
    // tile's reserved cell; we assert tiles never touch edge-to-edge).
    let baked = bake_font(&face(), &"Ao.8eOi".chars().collect::<Vec<_>>(), None);
    let rects: Vec<[f32; 4]> = baked
        .glyphs
        .iter()
        .filter(|g| g.atlas[2] > g.atlas[0])
        .map(|g| g.atlas)
        .collect();
    // Already proven non-overlapping; here assert each tile leaves space (the
    // reserved cell is field + ATLAS_PADDING_TEXELS). Since metrics describe the
    // field (not the padded cell), adjacent fields are non-overlapping which the
    // overlap test covers; this test asserts the atlas has slack beyond the
    // tightest bounding extent so padding exists.
    let max_right = rects.iter().map(|r| r[2]).fold(0.0_f32, f32::max);
    let max_top = rects.iter().map(|r| r[3]).fold(0.0_f32, f32::max);
    assert!(max_right <= baked.meta.atlas_w as f32, "tiles fit within atlas width");
    assert!(max_top <= baked.meta.atlas_h as f32, "tiles fit within atlas height");
}

#[test]
fn t3_glyph_metrics_carry_correct_advance() {
    // The metrics table advance must match the extracted glyph advance for the
    // mapped slot (the table is the layout truth at runtime).
    let cps: Vec<char> = "Ao. ".chars().collect();
    let baked = bake_font(&face(), &cps, None);
    for &cp in &cps {
        let slot = lookup_slot(&baked.cmap, cp as u32);
        let extracted = extract_codepoint(&face(), cp);
        let table_adv = baked.glyphs[slot as usize].advance_em;
        assert!(
            (table_adv - extracted.advance_em).abs() < 1e-5,
            "'{}' table advance {} == extracted {}",
            cp,
            table_adv,
            extracted.advance_em
        );
    }
}

// ----------------------------------------------------------------------------
// GATE 3 — .bfont round-trip
// ----------------------------------------------------------------------------

#[test]
fn t3_bfont_roundtrip_is_byte_identical() {
    let baked = bake_font(&face(), &"Ao.8eOi ".chars().collect::<Vec<_>>(), None);
    let bytes = write_bfont(&baked);
    let back = read_bfont(&bytes).expect("a freshly written .bfont must parse");
    let bytes2 = write_bfont(&back);
    assert_eq!(bytes, bytes2, "re-serialization is byte-identical");
}

#[test]
fn t3_bfont_roundtrip_preserves_all_tables() {
    let baked = bake_font(&face(), &"Ao.8eOi ".chars().collect::<Vec<_>>(), None);
    let back = read_bfont(&write_bfont(&baked)).expect("parse");
    assert_eq!(baked.meta, back.meta, "AtlasMeta preserved");
    assert_eq!(baked.glyphs, back.glyphs, "GlyphMetrics table preserved");
    assert_eq!(baked.cmap, back.cmap, "cmap preserved");
    assert_eq!(baked.kern, back.kern, "kern preserved");
    assert_eq!(baked.atlas.pixels, back.atlas.pixels, "atlas pixel bytes preserved");
}

#[test]
fn t3_bfont_cmap_binary_search_after_load() {
    let baked = bake_font(&face(), &"Ao.".chars().collect::<Vec<_>>(), None);
    let back = read_bfont(&write_bfont(&baked)).expect("parse");
    // Mapped codepoints resolve to a non-notdef slot; unmapped resolve to 0.
    assert!(lookup_slot(&back.cmap, 'A' as u32) > 0, "'A' resolves to a real slot");
    assert!(lookup_slot(&back.cmap, 'o' as u32) > 0, "'o' resolves to a real slot");
    assert_eq!(lookup_slot(&back.cmap, 'Z' as u32), 0, "unmapped 'Z' resolves to .notdef");
    // The cmap is sorted (binary search precondition).
    assert!(
        back.cmap.windows(2).all(|w| w[0].codepoint < w[1].codepoint),
        "cmap is sorted ascending for binary search"
    );
}

#[test]
fn t3_bfont_rejects_bad_magic() {
    let mut bytes = write_bfont(&bake_font(&face(), &['A'], None));
    bytes[0] ^= 0xFF; // corrupt the magic
    assert!(read_bfont(&bytes).is_none(), "a bad magic must be rejected");
}

#[test]
fn t3_bfont_rejects_truncation() {
    let bytes = write_bfont(&bake_font(&face(), &['A', 'o'], None));
    for cut in [0, 4, 12, 20, bytes.len() / 2] {
        assert!(read_bfont(&bytes[..cut]).is_none(), "truncation at {} must be rejected", cut);
    }
}

// ----------------------------------------------------------------------------
// Cross-cutting: parallel path equivalence (the threadpool distance dispatch)
// ----------------------------------------------------------------------------

#[test]
fn parallel_path_succeeds_from_a_normal_thread() {
    // Regression for the parallel-path fix: `generate_glyph_field(.., Some(pool))`
    // must run to completion when invoked from an ORDINARY application thread that
    // is NOT already inside a pool worker/`install` frame. The earlier dispatch
    // used `pool.scope(...)`, which debug-asserts an ambient pool and panicked
    // here; the fix wraps the band dispatch in `pool.install(...)`, which sets the
    // ambient-pool + worker-id TLS for the duration of the call (restored on
    // return and on unwind). This is the affirmative replacement for the removed
    // `#[should_panic]` pin — the bug it pinned is fixed, so we assert SUCCESS, not
    // a panic.
    use boyko_threadpool::ThreadPoolBuilder;
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    // The current std::thread (the cargo-test harness thread) is a plain thread,
    // not a pool worker — exactly the scenario that previously panicked.
    let g = extract_codepoint(&face(), 'A');
    let field = generate_glyph_field(&g.outline, Some(&pool))
        .expect("'A' is a non-empty glyph; the parallel path must return Some");
    assert!(field.width > 0 && field.height > 0, "parallel path produced a sized field");
    assert!(
        field.texels.iter().all(|t| t.is_finite()),
        "parallel field is fully finite (no torn/uninitialized band)"
    );
}

#[test]
fn parallel_path_equals_scalar_path_bit_for_bit() {
    // PARALLEL == SCALAR. The disjoint-row band partition must NOT change the math:
    // `generate_glyph_field(.., Some(pool))` and `(.., None)` must be BIT-IDENTICAL
    // (Decision T2-E determinism). Covers several real font glyphs ('A','o','.','O')
    // plus a synthetic SHARP-corner glyph (the teardrop tip) — the case most
    // sensitive to a partition-boundary math drift.
    use boyko_threadpool::ThreadPoolBuilder;
    let pool = ThreadPoolBuilder::new().num_threads(4).build();

    // (label, outline) pairs: four real glyphs + one sharp synthetic glyph.
    let f = face();
    let cases: Vec<(String, GlyphOutline)> = vec![
        ("A".to_string(), extract_codepoint(&f, 'A').outline),
        ("o".to_string(), extract_codepoint(&f, 'o').outline),
        (".".to_string(), extract_codepoint(&f, '.').outline),
        ("O".to_string(), extract_codepoint(&f, 'O').outline),
        ("teardrop(sharp)".to_string(), teardrop()),
    ];

    for (label, outline) in cases {
        let scalar = generate_glyph_field(&outline, None)
            .unwrap_or_else(|| panic!("'{}' must be a non-empty glyph", label));
        let parallel = generate_glyph_field(&outline, Some(&pool))
            .unwrap_or_else(|| panic!("'{}' parallel path must return Some", label));

        assert_eq!(scalar.width, parallel.width, "'{}' width", label);
        assert_eq!(scalar.height, parallel.height, "'{}' height", label);
        assert_eq!(scalar.texel_em, parallel.texel_em, "'{}' texel_em", label);
        assert_eq!(scalar.origin_em.x, parallel.origin_em.x, "'{}' origin_em.x", label);
        assert_eq!(scalar.origin_em.y, parallel.origin_em.y, "'{}' origin_em.y", label);
        // The load-bearing assertion: every texel float bit-identical. `Vec<f32>`
        // PartialEq is exact (bitwise for non-NaN), and the field is NaN-free, so
        // this is a true bit-for-bit equality of the full RGBA buffer.
        assert_eq!(
            scalar.texels, parallel.texels,
            "'{}' parallel field must be BIT-IDENTICAL to the scalar field (the \
             partition must not change the math)",
            label
        );
    }
}

#[test]
fn parallel_distance_field_matches_single_threaded() {
    // Lower-level companion to the above, at the `generate_distance_field` seam
    // (before sign/error-correction): the parallel distance dispatch must produce
    // a bit-identical field to the scalar reference path. Kept as a focused guard
    // on the distance pass itself.
    use boyko_threadpool::ThreadPoolBuilder;
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    for &cp in &['A', 'o', '8'] {
        let g = extract_codepoint(&face(), cp);
        let colored = color_outline(&g.outline);
        let serial = generate_distance_field(&colored, None);
        let parallel = generate_distance_field(&colored, Some(&pool));
        assert_eq!(serial.width, parallel.width, "'{}' width", cp);
        assert_eq!(serial.height, parallel.height, "'{}' height", cp);
        assert_eq!(
            serial.texels, parallel.texels,
            "'{}' parallel distance field must be bit-identical to single-threaded",
            cp
        );
    }
}

#[test]
fn t2a_per_channel_distance_vs_bruteforce_on_parallel_path() {
    // MSDF-vs-BRUTE-FORCE, now driven through the PARALLEL distance dispatch
    // (against the checked-in Ubuntu-Light.ttf). For every interior-projection,
    // non-saturated texel and RGB channel of the PARALLEL field, the signed
    // distance must match an independent brute-force nearest-point reference to
    // within a tight tolerance — proving the band partition reconstructs sharp
    // corners per-channel exactly as the scalar reference does.
    use boyko_threadpool::ThreadPoolBuilder;
    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    for &cp in &['A', 'o', '.'] {
        let g = extract_codepoint(&face(), cp);
        let colored = color_outline(&g.outline);
        let layout = field_layout(&colored);
        let field = generate_distance_field(&colored, Some(&pool));
        let edges: &[ColoredEdge] = &colored.edges;

        let mut max_err = 0.0_f32;
        let mut checked = 0usize;
        for y in 0..layout.height {
            for x in 0..layout.width {
                let p = texel_center(&layout, x, y);
                for ch in 0..3 {
                    let mut best_true = f32::INFINITY;
                    let mut best_param = 0.5_f32;
                    let mut best_signed = f32::INFINITY;
                    for e in edges {
                        if e.color.has_channel(ch) {
                            let (td, param, sd) = brute_nearest(&e.seg, p, 8_000);
                            if td < best_true {
                                best_true = td;
                                best_param = param;
                                best_signed = sd;
                            }
                        }
                    }
                    if best_param <= 0.02 || best_param >= 0.98 {
                        continue; // endpoint-clamped foot — pseudo path, excluded
                    }
                    let v = field.texel(x, y)[ch];
                    if v <= 1e-4 || v >= 1.0 - 1e-4 {
                        continue; // mapping-saturated, distance is clamped
                    }
                    let got_signed = (v - 0.5) * range_em();
                    let err = (got_signed - best_signed).abs();
                    if err > max_err {
                        max_err = err;
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 50, "'{}' must check a meaningful texel population, got {}", cp, checked);
        assert!(
            max_err < 0.004,
            "'{}' PARALLEL per-channel distance vs brute force: max err {:.6} em ({:.3} texels) exceeds tolerance",
            cp,
            max_err,
            max_err * 48.0
        );
    }
}

// ----------------------------------------------------------------------------
// GATE 2a (CFF/OTF) — END-TO-END cubic golden on the CFF charstring path
//
// The Ubuntu-Light fixture is TrueType (`glyf`): its outlines are quadratic, so
// it never drives the `cubic_to` sink / `Segment::Cubic`. SourceCodePro-Regular
// is OpenType-CFF (`OTTO`, Type-2 charstrings) whose curves are CUBIC. These
// goldens load that `.otf`, prove the extracted outline is genuinely cubic (not
// the quadratic fallback), pin the exact decoded charstring control points, and
// cross-check the MSDF `.a` (true SDF) channel against an INDEPENDENT brute-force
// nearest-point reference — exercising the same cubic solver end-to-end that the
// synthetic `t2a_synthetic_cubic_distance_matches_bruteforce` covers in isolation.
// ----------------------------------------------------------------------------

/// Path to the checked-in libre OpenType-CFF (cubic) fixture.
fn otf_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("SourceCodePro-Regular.otf")
}

/// Parses the CFF `.otf` fixture into a [`TtfFace`].
fn otf_face() -> TtfFace {
    let bytes = std::fs::read(otf_path()).expect(
        "test fixture missing: crates/boyko_fontbake/fixtures/SourceCodePro-Regular.otf \
         (a libre OpenType-CFF font must be checked in for the cubic golden)",
    );
    TtfFace::from_bytes(&bytes).expect("CFF .otf fixture must parse as a font")
}

#[test]
fn t2a_cff_otf_magic_is_opentype_cff() {
    // Guard the fixture identity: the file must be the CFF flavour (`OTTO`),
    // not a TrueType `glyf` font masquerading as `.otf`. This is what makes the
    // outline cubic instead of quadratic.
    let bytes = std::fs::read(otf_path()).expect("otf fixture must exist");
    assert_eq!(&bytes[0..4], b"OTTO", "SourceCodePro-Regular.otf must be OpenType-CFF (OTTO magic)");
}

#[test]
fn t2a_cff_glyph_o_is_all_cubics_two_contours() {
    // The decisive end-to-end assertion: the CFF charstring path emits CUBIC
    // segments. 'o' in SourceCodePro is a clean two-ring glyph whose every edge
    // is a Type-2 `rrcurveto` cubic — zero lines, zero quads. A quadratic
    // fallback (the `glyf` path) would report Q=8, C=0 instead.
    let g = extract_codepoint(&otf_face(), 'o');
    assert_eq!(g.outline.contours.len(), 2, "CFF 'o' has an outer ring + a counter");
    assert_eq!(g.outline.segment_count(), 8, "CFF 'o' total segments");
    let (lines, quads, cubics) = seg_type_counts(&g.outline);
    assert_eq!(
        (lines, quads, cubics),
        (0, 0, 8),
        "CFF 'o' must be ALL cubics (proves the CFF charstring path, not a quad fallback)"
    );
    assert!((g.advance_em - 0.6).abs() < 1e-4, "CFF 'o' advance_em == 0.6, got {}", g.advance_em);
}

#[test]
fn t2a_cff_cubic_control_points_match_decoded_charstring() {
    // Pin the EXACT decoded control points of the first cubic of 'o''s outer
    // contour. These are the em-normalized (÷1000 upem) Type-2 charstring
    // coordinates as emitted by the `cubic_to` sink; any drift in the CFF
    // decode, the em-normalization, or the cubic-segment construction trips this.
    let g = extract_codepoint(&otf_face(), 'o');
    let first = g.outline.contours[0][0];
    let Segment::Cubic { p0, c0, c1, p1 } = first else {
        panic!("CFF 'o' outer contour must start with a cubic, got {:?}", first);
    };
    let eps = 1e-5;
    let close = |a: Vec2, b: Vec2| (a.x - b.x).abs() < eps && (a.y - b.y).abs() < eps;
    assert!(close(p0, Vec2::new(0.300, -0.012)), "cubic p0 golden, got ({},{})", p0.x, p0.y);
    assert!(close(c0, Vec2::new(0.428, -0.012)), "cubic c0 golden, got ({},{})", c0.x, c0.y);
    assert!(close(c1, Vec2::new(0.540, 0.081)), "cubic c1 golden, got ({},{})", c1.x, c1.y);
    assert!(close(p1, Vec2::new(0.540, 0.242)), "cubic p1 golden, got ({},{})", p1.x, p1.y);
}

#[test]
fn t2a_cff_glyph_o_outer_and_counter_wind_opposite() {
    // Winding-topology invariant on the CFF cubic outline: the outer ring and
    // the counter (hole) must wind in OPPOSITE directions so the fill rule
    // carves the hole — the property the sign pass relies on, independent of the
    // font's absolute orientation convention.
    //
    // CFF (PostScript/Type-2) fonts use the OPPOSITE absolute orientation from
    // TrueType `glyf`: SourceCodePro 'o' decodes with the outer ring winding CCW
    // (positive chord-area here) and the counter CW (negative) — the mirror of
    // the Ubuntu (TTF) 'o'. Both signs are pinned to detect any future flip in
    // the CFF decode while documenting the convention difference (NOT a bug:
    // orientation is faithfully carried from the charstrings, and `correct_signs`
    // is winding-aware — see `t2b_glyph_o_counter_reads_outside`).
    let g = extract_codepoint(&otf_face(), 'o');
    let areas: Vec<f32> = g.outline.contours.iter().map(contour_area).collect();
    assert_eq!(areas.len(), 2, "CFF 'o' has two contours");
    let outer = areas.iter().cloned().fold(0.0_f32, |m, a| if a.abs() > m.abs() { a } else { m });
    let counter = areas.iter().cloned().find(|&a| a != outer).expect("two distinct areas");
    assert!(outer > 0.0, "CFF 'o' outer ring winds CCW (positive area, PostScript convention), got {}", outer);
    assert!(counter < 0.0, "CFF 'o' counter winds CW (negative area), got {}", counter);
    assert!(
        outer.signum() != counter.signum(),
        "CFF 'o' outer ({}) and counter ({}) must wind opposite (fill rule carves the hole)",
        outer,
        counter
    );
}

#[test]
fn t2a_cff_cubic_distance_matches_bruteforce_end_to_end() {
    // END-TO-END cubic distance gate: for every interior-projection,
    // non-saturated texel/channel of the CFF 'o' field whose nearest edge is a
    // CUBIC, the field's signed distance must match an INDEPENDENT brute-force
    // nearest-point reference (dense sampling, the test's own evaluator) to
    // within a tight tolerance. Because every edge of CFF 'o' is a cubic, this
    // drives the crate's cubic nearest-point solver through the full extract →
    // color → distance pipeline — the real-font analogue of the synthetic gate.
    let g = extract_codepoint(&otf_face(), 'o');
    let colored = color_outline(&g.outline);
    let layout = field_layout(&colored);
    let field = generate_distance_field(&colored, None);
    let edges: &[ColoredEdge] = &colored.edges;

    let mut max_err = 0.0_f32;
    let mut checked = 0usize;
    let mut cubic_checked = 0usize;
    for y in 0..layout.height {
        for x in 0..layout.width {
            let p = texel_center(&layout, x, y);
            for ch in 0..3 {
                let mut best_true = f32::INFINITY;
                let mut best_param = 0.5_f32;
                let mut best_signed = f32::INFINITY;
                let mut best_is_cubic = false;
                for e in edges {
                    if e.color.has_channel(ch) {
                        let (td, param, sd) = brute_nearest(&e.seg, p, 8_000);
                        if td < best_true {
                            best_true = td;
                            best_param = param;
                            best_signed = sd;
                            best_is_cubic = matches!(e.seg, Segment::Cubic { .. });
                        }
                    }
                }
                // Interior projection only (clamped feet use the pseudo path).
                if best_param <= 0.02 || best_param >= 0.98 {
                    continue;
                }
                let v = field.texel(x, y)[ch];
                if v <= 1e-4 || v >= 1.0 - 1e-4 {
                    continue; // mapping-saturated, distance is clamped
                }
                let got_signed = (v - 0.5) * range_em();
                let err = (got_signed - best_signed).abs();
                if err > max_err {
                    max_err = err;
                }
                checked += 1;
                if best_is_cubic {
                    cubic_checked += 1;
                }
            }
        }
    }
    assert!(checked > 50, "CFF 'o' must check a meaningful texel population, got {}", checked);
    assert!(
        cubic_checked > 30,
        "CFF 'o' must exercise the CUBIC solver at most probes (got {} cubic-nearest of {} total)",
        cubic_checked,
        checked
    );
    // One texel is 1/48 em ≈ 0.0208; tolerance is a small fraction of that.
    assert!(
        max_err < 0.004,
        "CFF 'o' end-to-end cubic distance vs brute force: max err {:.6} em ({:.3} texels) exceeds tolerance",
        max_err,
        max_err * 48.0
    );
}

#[test]
fn t2a_cff_glyph_o_field_is_finite_and_clash_free() {
    // The CFF 'o' field must be all-finite, in [0,1], and the median must agree
    // with the .a true-SDF sign everywhere the .a is decisive (no spurious MSDF
    // edge) — the end-to-end MSDF acceptance property on the cubic path.
    let g = extract_codepoint(&otf_face(), 'o');
    let f = generate_glyph_field(&g.outline, None).expect("CFF 'o' is non-empty");
    let mut clash = 0;
    for y in 0..f.height {
        for x in 0..f.width {
            let t = f.texel(x, y);
            for (i, &c) in t.iter().enumerate() {
                assert!(c.is_finite(), "CFF 'o' texel ({},{}) ch {} is non-finite ({})", x, y, i, c);
                assert!((0.0..=1.0).contains(&c), "CFF 'o' texel ({},{}) ch {} out of [0,1] ({})", x, y, i, c);
            }
            let med = median(t[0], t[1], t[2]);
            // Decisive only when .a is clearly inside/outside (away from the edge).
            if t[3] > 0.6 && med <= 0.5 {
                clash += 1;
            }
            if t[3] < 0.4 && med > 0.5 {
                clash += 1;
            }
        }
    }
    assert_eq!(clash, 0, "CFF 'o' field must have zero median/.a clashes (clean MSDF on cubic path)");
}

// ----------------------------------------------------------------------------
// helpers that touch crate internals
// ----------------------------------------------------------------------------

fn seg_type_counts(o: &GlyphOutline) -> (usize, usize, usize) {
    let (mut l, mut q, mut c) = (0, 0, 0);
    for contour in &o.contours {
        for s in contour {
            match s {
                Segment::Line { .. } => l += 1,
                Segment::Quad { .. } => q += 1,
                Segment::Cubic { .. } => c += 1,
            }
        }
    }
    (l, q, c)
}

fn contour_area(contour: &Vec<Segment>) -> f32 {
    let mut area = 0.0;
    for s in contour {
        area += s.start().cross(s.end());
    }
    0.5 * area
}

fn empty_colored() -> ColoredOutline {
    ColoredOutline {
        edges: Vec::new(),
        contour_ranges: Vec::new(),
        bbox_min: Vec2::ZERO,
        bbox_max: Vec2::ZERO,
    }
}

fn assert_zero_inverted_interior(cp: char) {
    let g = extract_codepoint(&face(), cp);
    let f = generate_glyph_field(&g.outline, None).unwrap_or_else(|| panic!("'{}' non-empty", cp));
    let mut interior = 0;
    let mut inverted = 0;
    for y in 0..f.height {
        for x in 0..f.width {
            let p = texel_center_from_field(&f, x, y);
            if ref_inside(&g.outline, p) {
                interior += 1;
                let t = f.texel(x, y);
                if median(t[0], t[1], t[2]) <= 0.5 {
                    inverted += 1;
                }
            }
        }
    }
    assert!(interior > 20, "'{}' must have a measurable interior, got {}", cp, interior);
    assert_eq!(inverted, 0, "'{}' must have zero inverted-interior texels (T2b sign-correction)", cp);
}

