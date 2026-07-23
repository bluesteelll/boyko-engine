//! T2 — the in-house MSDF generator.
//!
//! Three independently-gated passes, matching the canonical Chlumsky pipeline:
//!
//! - [`color`] (T2a) — edge coloring (seeded switch, max-angle corner
//!   predicate, 0/1/N-corner handling).
//! - [`distance`] (T2a) — per-channel signed pseudo-distance (line closed-form,
//!   quad via Cardano, cubic via multi-seed Newton) + range mapping at the
//!   global pixels-per-em.
//! - [`sign`] (T2b) — overlap combiner + the scanline sign-correction pass that
//!   overrides the pseudo-distance sign with authoritative insideness.
//! - [`error_correct`] (T2c) — the mandatory msdfgen error-correction pass that
//!   removes interpolation/clash speckle.
//!
//! The public entry point is [`generate_glyph_field`], which runs all four
//! passes and returns a [`GlyphField`] (the MTSDF texel buffer for one glyph).
//! Per-texel distance generation is dispatched on the engine threadpool by
//! disjoint-output row partitioning.

pub mod color;
pub mod distance;
pub mod error_correct;
pub mod sign;

use std::sync::Arc;

use boyko_math::Vec2;
use boyko_threadpool::ThreadPool;

use crate::constants::{DISTANCE_RANGE_TEXELS, PIXELS_PER_EM};
use crate::extract::GlyphOutline;
use crate::msdf::color::ColoredOutline;

/// One glyph's generated MTSDF field: a tightly-packed RGBA texel buffer.
///
/// Channels: R/G/B are the per-channel MSDF; A is the true single-channel SDF
/// (the MTSDF 4th channel), sharing the identical range mapping and 0.5
/// zero-crossing as RGB (Decision T2-F). Values are in `[0, 1]` floats; the
/// atlas packer quantizes to RGBA8.
#[derive(Clone, Debug)]
pub struct GlyphField {
    /// Field width in texels (the expanded transition-band quad width).
    pub width: u32,
    /// Field height in texels.
    pub height: u32,
    /// `width * height * 4` floats, row-major, RGBA interleaved, in `[0, 1]`.
    pub texels: Vec<f32>,
    /// Em-space origin of texel `(0, 0)` center: the lower-left of the expanded
    /// region. Lets the atlas packer recover planeBounds.
    pub origin_em: Vec2,
    /// Em size of one texel edge (`1.0 / pixels_per_em`).
    pub texel_em: f32,
}

impl GlyphField {
    /// Reads the RGBA texel at `(x, y)` as a 4-tuple. Panics in debug on OOB.
    #[inline]
    pub fn texel(&self, x: u32, y: u32) -> [f32; 4] {
        debug_assert!(x < self.width && y < self.height);
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.texels[i],
            self.texels[i + 1],
            self.texels[i + 2],
            self.texels[i + 3],
        ]
    }
}

/// The em-space distance range (`distance_range_texels / pixels_per_em`).
/// Signed distances are mapped `value = sd / range_em + 0.5`, clamped `[0, 1]`.
#[inline]
pub fn range_em() -> f32 {
    DISTANCE_RANGE_TEXELS / PIXELS_PER_EM
}

/// Maps a signed distance in em units to a `[0, 1]` field value with the 0.5
/// zero-crossing convention shared by all four channels.
#[inline]
pub fn map_distance(signed_em: f32) -> f32 {
    (signed_em / range_em() + 0.5).clamp(0.0, 1.0)
}

/// Generates the full MTSDF field for one glyph outline, running all four
/// passes (color → distance → sign-correct → error-correct).
///
/// `pool` dispatches the per-texel distance work by disjoint-output row
/// partitioning. Pass `None` to run single-threaded (the scalar reference path
/// used by the determinism goldens, §Decision T2-E).
///
/// Returns `None` for an empty glyph (no contours → no atlas entry).
pub fn generate_glyph_field(outline: &GlyphOutline, pool: Option<&Arc<ThreadPool>>) -> Option<GlyphField> {
    if outline.is_empty() {
        return None;
    }

    let colored = color::color_outline(outline);
    let mut field = distance::generate_distance_field(&colored, pool);
    sign::correct_signs(&mut field, &colored);
    error_correct::correct_errors(&mut field, &colored);
    Some(field)
}

/// The layout (size + em placement) of a glyph's expanded transition-band field
/// region, computed once and shared by the distance pass and the atlas packer.
#[derive(Clone, Copy, Debug)]
pub struct FieldLayout {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Em-space lower-left of the expanded region (texel `(0,0)` center maps
    /// here + half a texel).
    pub origin_em: Vec2,
    /// Em size of one texel.
    pub texel_em: f32,
}

/// Largest dimension, in texels, one glyph's field may occupy.
///
/// 2026-07 audit: `width`/`height` are derived from FONT-SUPPLIED bbox coordinates, and nothing
/// bounded them. `generate_distance_field` then sizes its buffer with `w * h * 4` in `u32`
/// arithmetic, so a malformed or hostile `.ttf` with a huge span wrapped that product to a small
/// allocation while the generation loops still indexed by the full `width`/`height` — a
/// guaranteed panic on parsed input, the same trust-boundary class as the `.bfont` extent
/// mismatch. Rust's float→int casts saturate rather than wrap, so an infinite span arrives at
/// the cast as `u32::MAX`; the clamp, not the cast, is what bounds it.
///
/// At [`PIXELS_PER_EM`] = 48 a full-em glyph is ~48 texels and the §T3 pad adds ~4 per side, so
/// this cap is ~20 em — an order of magnitude past any real glyph, and it bounds one field at
/// 1024 × 1024 × 4 `f32` = 16 MiB.
pub const MAX_FIELD_DIM: u32 = 1024;

/// Rounds a texel span up to at least 1 and at most [`MAX_FIELD_DIM`].
///
/// A non-finite span collapses to 1 rather than propagating: `NaN.ceil()` is `NaN`, and
/// `f32::max` returns the non-`NaN` operand.
#[inline]
fn field_dim(span_texels: f32) -> u32 {
    span_texels.ceil().max(1.0).min(MAX_FIELD_DIM as f32) as u32
}

/// Computes the expanded field layout for a colored outline: the tight bbox
/// padded by `distance_range_texels / 2 + 1` texels on every side (§T3), so the
/// AA transition band is fully represented in the field.
///
/// Both dimensions are clamped to [`MAX_FIELD_DIM`]. The clamp lives here, where the size is
/// born, so the distance pass and the atlas packer — which both call this — cannot disagree
/// about how large a glyph is.
pub fn field_layout(colored: &ColoredOutline) -> FieldLayout {
    let texel_em = 1.0 / PIXELS_PER_EM;
    // Half the distance range, plus one texel of slack, in em.
    let pad_em = (DISTANCE_RANGE_TEXELS * 0.5 + 1.0) * texel_em;

    let min = Vec2::new(colored.bbox_min.x - pad_em, colored.bbox_min.y - pad_em);
    let max = Vec2::new(colored.bbox_max.x + pad_em, colored.bbox_max.y + pad_em);

    let width = field_dim((max.x - min.x) * PIXELS_PER_EM);
    let height = field_dim((max.y - min.y) * PIXELS_PER_EM);

    FieldLayout {
        width,
        height,
        origin_em: min,
        texel_em,
    }
}

/// Em-space center of texel `(x, y)` for a given layout.
#[inline]
pub fn texel_center(layout: &FieldLayout, x: u32, y: u32) -> Vec2 {
    Vec2::new(
        layout.origin_em.x + (x as f32 + 0.5) * layout.texel_em,
        layout.origin_em.y + (y as f32 + 0.5) * layout.texel_em,
    )
}

/// Em-space center of texel `(x, y)` recovered from a generated [`GlyphField`].
/// Used by the sign-correction and error-correction passes, which carry the
/// field rather than the layout.
#[inline]
pub fn texel_center_from_field(field: &GlyphField, x: u32, y: u32) -> Vec2 {
    Vec2::new(
        field.origin_em.x + (x as f32 + 0.5) * field.texel_em,
        field.origin_em.y + (y as f32 + 0.5) * field.texel_em,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::GlyphOutline;
    use crate::msdf::color::color_outline;

    #[test]
    fn range_em_is_texels_over_ppem() {
        assert!((range_em() - DISTANCE_RANGE_TEXELS / PIXELS_PER_EM).abs() < 1e-9);
    }

    #[test]
    fn map_distance_zero_crossing_is_half() {
        assert!((map_distance(0.0) - 0.5).abs() < 1e-6, "zero distance maps to 0.5");
    }

    #[test]
    fn map_distance_sign_direction() {
        assert!(map_distance(range_em()) > 0.5, "positive (inside) maps above 0.5");
        assert!(map_distance(-range_em()) < 0.5, "negative (outside) maps below 0.5");
    }

    #[test]
    fn map_distance_clamps_to_unit_interval() {
        assert_eq!(map_distance(100.0), 1.0, "far inside saturates at 1.0");
        assert_eq!(map_distance(-100.0), 0.0, "far outside saturates at 0.0");
    }

    #[test]
    fn map_distance_is_inverse_of_unmapping_in_range() {
        // Within the unclamped band, (map(d)-0.5)*range_em == d.
        let d = range_em() * 0.3;
        let v = map_distance(d);
        let back = (v - 0.5) * range_em();
        assert!((back - d).abs() < 1e-6, "round-trips inside the band");
    }

    #[test]
    fn empty_glyph_yields_no_field() {
        let empty = GlyphOutline::default();
        assert!(generate_glyph_field(&empty, None).is_none(), "no contours ⇒ no field");
    }

    #[test]
    fn field_layout_pads_by_half_range_plus_one() {
        // A unit-square outline; the layout must extend beyond the bbox by the
        // transition-band padding on every side.
        let outline = GlyphOutline {
            contours: vec![vec![]],
            bbox_min: Vec2::new(0.0, 0.0),
            bbox_max: Vec2::new(1.0, 1.0),
        };
        let colored = color_outline(&outline);
        let layout = field_layout(&colored);
        let pad_em = (DISTANCE_RANGE_TEXELS * 0.5 + 1.0) / PIXELS_PER_EM;
        assert!(layout.origin_em.x <= -pad_em + 1e-4, "left padding present");
        assert!(layout.origin_em.y <= -pad_em + 1e-4, "bottom padding present");
        // Width spans bbox (1 em == PPEM texels) plus the padding band on BOTH
        // sides (distance_range + 2 slack texels). Bound it both ways rather than
        // recompute the exact ceil (float-order sensitive).
        let bbox_texels = PIXELS_PER_EM as u32; // 1 em
        let pad_band = DISTANCE_RANGE_TEXELS as u32 + 2; // ~range + 2 slack
        assert!(
            layout.width >= bbox_texels + pad_band - 1 && layout.width <= bbox_texels + pad_band + 1,
            "width {} must cover bbox {} + padding band ~{}",
            layout.width,
            bbox_texels,
            pad_band
        );
    }

    #[test]
    fn field_layout_texel_em_is_inverse_ppem() {
        let outline = GlyphOutline {
            contours: vec![vec![]],
            bbox_min: Vec2::ZERO,
            bbox_max: Vec2::new(0.5, 0.5),
        };
        let layout = field_layout(&color_outline(&outline));
        assert!((layout.texel_em - 1.0 / PIXELS_PER_EM).abs() < 1e-9);
    }

    #[test]
    fn texel_center_is_offset_by_half_texel() {
        let layout = FieldLayout {
            width: 4,
            height: 4,
            origin_em: Vec2::new(0.0, 0.0),
            texel_em: 0.25,
        };
        let c = texel_center(&layout, 0, 0);
        assert!((c.x - 0.125).abs() < 1e-6 && (c.y - 0.125).abs() < 1e-6, "first texel center is +half texel");
        let c2 = texel_center(&layout, 1, 0);
        assert!((c2.x - 0.375).abs() < 1e-6, "next texel center is +1 texel");
    }

    #[test]
    fn glyph_field_texel_reads_rgba() {
        let field = GlyphField {
            width: 2,
            height: 1,
            texels: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
            origin_em: Vec2::ZERO,
            texel_em: 1.0,
        };
        assert_eq!(field.texel(0, 0), [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(field.texel(1, 0), [0.5, 0.6, 0.7, 0.8]);
    }
}
