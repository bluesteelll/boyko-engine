//! T2c — the mandatory MSDF error-correction pass.
//!
//! The shader's `median(r,g,b)` is downstream of the bake and cannot recover an
//! inconsistent field: where two channels cross between adjacent texels, the
//! median can introduce a **spurious edge** (a stray bright/dark speck on a run
//! that the true distance shows is smooth). This pass detects those texels and
//! collapses their RGB channels to the true single-channel distance (the MTSDF
//! `.a` channel), which by construction has no such artifact.
//!
//! Detection (the msdfgen analytic-bilinear criterion, simplified to the
//! axis-neighbor predictor): for each texel and each 4-neighbor, compare the
//! actual `median(rgb)` transition between the two texels against the true
//! transition encoded by the `.a` channel. If the median crosses 0.5 (an edge)
//! where the true distance does NOT — or crosses in the opposite direction —
//! the texel is a clash and is collapsed.
//!
//! Detection is a read-only pass over the immutable input; the collapse is a
//! second pass writing only flagged texels, so the predictor never reads a
//! half-corrected neighbor (order-independent, parallelizable per texel).

use crate::msdf::GlyphField;
use crate::msdf::color::ColoredOutline;

/// The median of three channel values (the shader's reconstruction).
#[inline]
fn median(r: f32, g: f32, b: f32) -> f32 {
    r.max(g).min(r.min(g).max(b))
}

/// `true` when the transition `a → b` crosses the 0.5 isovalue (an edge).
#[inline]
fn crosses(a: f32, b: f32) -> bool {
    (a - 0.5) * (b - 0.5) < 0.0
}

/// Detects and corrects median clash artifacts (T2c).
///
/// `_colored` is accepted for parity with the other passes and to allow a
/// future exact-recompute variant; the current criterion is self-contained in
/// the field's `.a` (true SDF) channel.
pub fn correct_errors(field: &mut GlyphField, _colored: &ColoredOutline) {
    let w = field.width;
    let h = field.height;
    if w == 0 || h == 0 {
        return;
    }

    // First pass: flag clashing texels (read-only over the input field).
    let mut clash = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let base = idx * 4;
            let m_here = median(
                field.texels[base],
                field.texels[base + 1],
                field.texels[base + 2],
            );
            let a_here = field.texels[base + 3];

            // Axis neighbors: +x and +y (each edge is examined once).
            let mut neighbors: [(u32, u32); 2] = [(x, y); 2];
            let mut n = 0;
            if x + 1 < w {
                neighbors[n] = (x + 1, y);
                n += 1;
            }
            if y + 1 < h {
                neighbors[n] = (x, y + 1);
                n += 1;
            }

            for &(nx, ny) in &neighbors[..n] {
                let nbase = ((ny * w + nx) * 4) as usize;
                let m_n = median(
                    field.texels[nbase],
                    field.texels[nbase + 1],
                    field.texels[nbase + 2],
                );
                let a_n = field.texels[nbase + 3];

                // A clash: the median crosses the edge between the two texels
                // but the true single-channel distance does NOT (a spurious
                // median edge), OR they cross in opposite directions.
                let median_edge = crosses(m_here, m_n);
                let true_edge = crosses(a_here, a_n);
                if median_edge != true_edge {
                    clash[idx] = true;
                    clash[(ny * w + nx) as usize] = true;
                }
            }
        }
    }

    // Second pass: collapse flagged texels' RGB to the true SDF (.a). The .a
    // channel already carries the identical range mapping + 0.5 zero-crossing
    // (Decision T2-F), so collapsing keeps the edge position exact.
    for (idx, &flagged) in clash.iter().enumerate() {
        if flagged {
            let base = idx * 4;
            let a = field.texels[base + 3];
            field.texels[base] = a;
            field.texels[base + 1] = a;
            field.texels[base + 2] = a;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_math::Vec2;

    fn empty_colored() -> ColoredOutline {
        ColoredOutline {
            edges: Vec::new(),
            contour_ranges: Vec::new(),
            bbox_min: Vec2::ZERO,
            bbox_max: Vec2::ZERO,
        }
    }

    #[test]
    fn median_picks_middle_value() {
        assert_eq!(median(0.1, 0.5, 0.9), 0.5);
        assert_eq!(median(0.9, 0.1, 0.5), 0.5);
        assert_eq!(median(0.5, 0.5, 0.5), 0.5);
    }

    #[test]
    fn crosses_detects_isovalue_transition() {
        assert!(crosses(0.4, 0.6), "0.4 → 0.6 crosses 0.5");
        assert!(crosses(0.6, 0.4), "0.6 → 0.4 crosses 0.5");
        assert!(!crosses(0.6, 0.7), "both above ⇒ no crossing");
        assert!(!crosses(0.1, 0.4), "both below ⇒ no crossing");
    }

    #[test]
    fn zero_sized_field_is_noop() {
        let mut field = GlyphField {
            width: 0,
            height: 0,
            texels: Vec::new(),
            origin_em: Vec2::ZERO,
            texel_em: 1.0,
        };
        correct_errors(&mut field, &empty_colored());
        assert!(field.texels.is_empty(), "empty field unchanged, no panic");
    }

    #[test]
    fn injected_clash_collapses_to_alpha() {
        // Middle texel: median 0.1 (outside) but .a 0.9 (inside) = clash.
        let mut field = GlyphField {
            width: 3,
            height: 1,
            texels: vec![
                0.9, 0.9, 0.9, 0.9, // texel 0 clean inside
                0.9, 0.1, 0.1, 0.9, // texel 1 CLASH
                0.9, 0.9, 0.9, 0.9, // texel 2 clean inside
            ],
            origin_em: Vec2::ZERO,
            texel_em: 1.0,
        };
        correct_errors(&mut field, &empty_colored());
        assert_eq!(
            (field.texels[4], field.texels[5], field.texels[6]),
            (0.9, 0.9, 0.9),
            "clashing texel RGB collapsed to its .a value"
        );
        let m = median(field.texels[4], field.texels[5], field.texels[6]);
        assert!(m > 0.5, "post-correction median agrees with .a");
    }

    #[test]
    fn clean_field_is_unchanged() {
        let clean = vec![0.7_f32; 3 * 3 * 4];
        let mut field = GlyphField {
            width: 3,
            height: 3,
            texels: clean.clone(),
            origin_em: Vec2::ZERO,
            texel_em: 1.0,
        };
        correct_errors(&mut field, &empty_colored());
        assert_eq!(field.texels, clean, "no clash ⇒ byte-identical");
    }
}
