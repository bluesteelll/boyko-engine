//! Screen-edge anchoring (GUI P6a) — the pure origin-resolve folded into the
//! layout apply.
//!
//! Anchoring is a POSITIONING transform, not a new layout algorithm: it changes
//! only a root's screen ORIGIN (top-left). [`resolve_anchor_origin`] is a pure
//! function of `(viewport, safe-area, anchor, measured root size)`; the layout
//! apply seeds `layout_root`'s origin with it AFTER the root is measured (so the
//! root's `w/h` are known — right/bottom edges need them) and BEFORE the root
//! rect is written. The layout pass therefore stays the single `ComputedRect`
//! writer (no pre-pass write race — the seam Decision 3 identifies).
//!
//! It is `O(R_anchored)` (one lookup per cached root, only when relaying) — no
//! new tree traversal, so the layout O(N) complexity guard is untouched.

use crate::components::{AnchorEdge, UiAnchor};
use crate::resources::{UiSafeArea, UiViewport};

/// The resolved screen-space top-left an anchored root is placed at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchorOrigin {
    /// Top-left x, logical px.
    pub x: f32,
    /// Top-left y, logical px.
    pub y: f32,
}

/// Resolves the screen-space top-left for a root of measured size `(w, h)` under
/// `anchor`, against `viewport` and (when `anchor.use_safe_area`) `safe`.
///
/// The anchor's `offset_{x,y}` always insets TOWARD the screen interior from the
/// pinned edge: positive `offset_x` moves a left-anchored node right and a
/// right-anchored node left; positive `offset_y` moves a top-anchored node down
/// and a bottom-anchored node up (center edges treat the offset as a signed nudge
/// in `+x`/`+y`). Safe-area shrinks the usable rectangle on each inset side.
///
/// Pure + total: no allocation, no world access, branch-light (a 9-arm match =
/// a jump table). The result is finite when the inputs are finite
/// (`debug_assert`ed below).
pub fn resolve_anchor_origin(
    viewport: &UiViewport,
    safe: &UiSafeArea,
    anchor: &UiAnchor,
    w: f32,
    h: f32,
) -> AnchorOrigin {
    debug_assert!(
        viewport.width.is_finite() && viewport.height.is_finite(),
        "invariant: viewport extent must be finite for anchor resolve"
    );

    // The usable rectangle after the safe-area inset (or the full viewport).
    let (sl, st, sr, sb) = if anchor.use_safe_area {
        (safe.left, safe.top, safe.right, safe.bottom)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    let usable_left = sl;
    let usable_top = st;
    let usable_right = viewport.width - sr;
    let usable_bottom = viewport.height - sb;
    let usable_w = usable_right - usable_left;
    let usable_h = usable_bottom - usable_top;

    let ox = anchor.offset_x;
    let oy = anchor.offset_y;

    // Per-axis anchor fractions: 0.0 = before-edge, 0.5 = center, 1.0 = after.
    // The offset insets toward the interior, so it is ADDED at the before-edge
    // and SUBTRACTED at the after-edge; at center it is a signed `+` nudge.
    let x = match anchor.edge {
        AnchorEdge::TopLeft | AnchorEdge::CenterLeft | AnchorEdge::BottomLeft => usable_left + ox,
        AnchorEdge::TopCenter | AnchorEdge::Center | AnchorEdge::BottomCenter => {
            usable_left + (usable_w - w) * 0.5 + ox
        }
        AnchorEdge::TopRight | AnchorEdge::CenterRight | AnchorEdge::BottomRight => {
            usable_right - w - ox
        }
    };
    let y = match anchor.edge {
        AnchorEdge::TopLeft | AnchorEdge::TopCenter | AnchorEdge::TopRight => usable_top + oy,
        AnchorEdge::CenterLeft | AnchorEdge::Center | AnchorEdge::CenterRight => {
            usable_top + (usable_h - h) * 0.5 + oy
        }
        AnchorEdge::BottomLeft | AnchorEdge::BottomCenter | AnchorEdge::BottomRight => {
            usable_bottom - h - oy
        }
    };

    debug_assert!(x.is_finite() && y.is_finite(), "invariant: anchor origin must be finite");
    AnchorOrigin { x, y }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(w: f32, h: f32) -> UiViewport {
        UiViewport { width: w, height: h, scale_factor: 1.0, generation: 0 }
    }

    fn anchor(edge: AnchorEdge, ox: f32, oy: f32, use_safe_area: bool) -> UiAnchor {
        UiAnchor { edge, offset_x: ox, offset_y: oy, use_safe_area, _pad: [0; 3] }
    }

    #[test]
    fn top_left_zero_offset_is_origin() {
        let o = resolve_anchor_origin(
            &vp(1920.0, 1080.0),
            &UiSafeArea::default(),
            &anchor(AnchorEdge::TopLeft, 0.0, 0.0, false),
            200.0,
            100.0,
        );
        assert_eq!(o, AnchorOrigin { x: 0.0, y: 0.0 });
    }

    #[test]
    fn bottom_right_no_safe_area() {
        // x = vw - w - off, y = vh - h - off.
        let o = resolve_anchor_origin(
            &vp(1920.0, 1080.0),
            &UiSafeArea::default(),
            &anchor(AnchorEdge::BottomRight, 16.0, 24.0, false),
            200.0,
            100.0,
        );
        assert_eq!(o, AnchorOrigin { x: 1920.0 - 200.0 - 16.0, y: 1080.0 - 100.0 - 24.0 });
    }

    #[test]
    fn bottom_right_with_safe_area() {
        // x = vw - safe.right - w - off, y = vh - safe.bottom - h - off.
        let safe = UiSafeArea { left: 10.0, top: 20.0, right: 30.0, bottom: 40.0 };
        let o = resolve_anchor_origin(
            &vp(1920.0, 1080.0),
            &safe,
            &anchor(AnchorEdge::BottomRight, 16.0, 24.0, true),
            200.0,
            100.0,
        );
        assert_eq!(
            o,
            AnchorOrigin {
                x: 1920.0 - 30.0 - 200.0 - 16.0,
                y: 1080.0 - 40.0 - 100.0 - 24.0,
            }
        );
    }

    #[test]
    fn center_is_midpoint() {
        let o = resolve_anchor_origin(
            &vp(1000.0, 600.0),
            &UiSafeArea::default(),
            &anchor(AnchorEdge::Center, 0.0, 0.0, false),
            200.0,
            100.0,
        );
        assert_eq!(o, AnchorOrigin { x: (1000.0 - 200.0) * 0.5, y: (600.0 - 100.0) * 0.5 });
    }

    #[test]
    fn top_center_uses_safe_area_horizontal_band() {
        let safe = UiSafeArea { left: 50.0, top: 0.0, right: 50.0, bottom: 0.0 };
        let o = resolve_anchor_origin(
            &vp(1000.0, 600.0),
            &safe,
            &anchor(AnchorEdge::TopCenter, 0.0, 8.0, true),
            100.0,
            40.0,
        );
        // usable_w = 1000 - 100 = 900; x = 50 + (900 - 100)/2 = 50 + 400 = 450.
        assert_eq!(o, AnchorOrigin { x: 450.0, y: 8.0 });
    }
}
