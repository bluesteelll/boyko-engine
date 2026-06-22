//! GATE 4 — `UiAnchor` pins a root to a screen edge/corner at the correct
//! `ComputedRect` for a known viewport (+ safe-area inset).
//!
//! Drives the FULL layout pass (the anchor resolve is folded into `layout_root`
//! after measure / before the rect write — the single-writer seam), so this is an
//! end-to-end check of the seam, not just the pure `resolve_anchor_origin` (which
//! has its own unit tests in `anchor.rs`). The root carries a definite Px size so
//! the measured `(w, h)` the right/bottom edges need is known.

mod p6a_common;

use p6a_common::{approx, P6a};

use boyko_ui::components::{AnchorEdge, UiAnchor, UiLayout};
use boyko_ui::resources::{UiSafeArea, UiViewport};
use boyko_ui::units::{LayoutType, Unit};

/// A definite Px node of the given size (a Column; orientation is irrelevant to the
/// root rect's w/h since both axes are explicit Px).
fn sized(w: f32, h: f32) -> UiLayout {
    UiLayout {
        layout_type: LayoutType::Column,
        width: Unit::Px(w),
        height: Unit::Px(h),
        ..UiLayout::default()
    }
}

fn anchor(edge: AnchorEdge, ox: f32, oy: f32, safe: bool) -> UiAnchor {
    UiAnchor { edge, offset_x: ox, offset_y: oy, use_safe_area: safe, _pad: [0; 3] }
}

fn world_1920() -> P6a {
    P6a::new(UiViewport { width: 1920.0, height: 1080.0, scale_factor: 1.0, generation: 0 })
}

#[test]
fn anchor_top_left_zero_offset_at_origin() {
    let mut w = world_1920();
    let n = w.spawn_anchored_root(sized(200.0, 100.0), anchor(AnchorEdge::TopLeft, 0.0, 0.0, false));
    w.run();
    let r = w.rect(n);
    approx(r.x, 0.0, "TopLeft x");
    approx(r.y, 0.0, "TopLeft y");
    approx(r.w, 200.0, "node width");
    approx(r.h, 100.0, "node height");
}

#[test]
fn anchor_bottom_right_no_safe_area() {
    let mut w = world_1920();
    let n = w.spawn_anchored_root(
        sized(200.0, 100.0),
        anchor(AnchorEdge::BottomRight, 16.0, 24.0, false),
    );
    w.run();
    let r = w.rect(n);
    // x = vw - w - off; y = vh - h - off.
    approx(r.x, 1920.0 - 200.0 - 16.0, "BottomRight x");
    approx(r.y, 1080.0 - 100.0 - 24.0, "BottomRight y");
}

#[test]
fn anchor_bottom_right_with_safe_area_inset() {
    let mut w = world_1920();
    w.set_safe_area(UiSafeArea { left: 10.0, top: 20.0, right: 30.0, bottom: 40.0 });
    let n = w.spawn_anchored_root(
        sized(200.0, 100.0),
        anchor(AnchorEdge::BottomRight, 16.0, 24.0, true),
    );
    w.run();
    let r = w.rect(n);
    // x = vw - safe.right - w - off; y = vh - safe.bottom - h - off.
    approx(r.x, 1920.0 - 30.0 - 200.0 - 16.0, "BottomRight x with safe-area");
    approx(r.y, 1080.0 - 40.0 - 100.0 - 24.0, "BottomRight y with safe-area");
}

#[test]
fn anchor_center_is_midpoint() {
    let mut w = P6a::new(UiViewport { width: 1000.0, height: 600.0, scale_factor: 1.0, generation: 0 });
    let n = w.spawn_anchored_root(sized(200.0, 100.0), anchor(AnchorEdge::Center, 0.0, 0.0, false));
    w.run();
    let r = w.rect(n);
    approx(r.x, (1000.0 - 200.0) * 0.5, "Center x");
    approx(r.y, (600.0 - 100.0) * 0.5, "Center y");
}

#[test]
fn anchor_top_right_pins_right_edge() {
    let mut w = world_1920();
    let n = w.spawn_anchored_root(sized(300.0, 50.0), anchor(AnchorEdge::TopRight, 8.0, 8.0, false));
    w.run();
    let r = w.rect(n);
    approx(r.x, 1920.0 - 300.0 - 8.0, "TopRight x");
    approx(r.y, 8.0, "TopRight y");
    // The node's right edge sits 8px from the screen right.
    approx(r.x + r.w, 1920.0 - 8.0, "TopRight: right edge inset by offset");
}

#[test]
fn anchor_bottom_center_uses_safe_area_band() {
    let mut w = P6a::new(UiViewport { width: 1000.0, height: 600.0, scale_factor: 1.0, generation: 0 });
    w.set_safe_area(UiSafeArea { left: 50.0, top: 0.0, right: 50.0, bottom: 60.0 });
    let n = w.spawn_anchored_root(
        sized(100.0, 40.0),
        anchor(AnchorEdge::BottomCenter, 0.0, 10.0, true),
    );
    w.run();
    let r = w.rect(n);
    // usable_w = 1000 - 100 = 900; x = 50 + (900 - 100)/2 = 50 + 400 = 450.
    approx(r.x, 450.0, "BottomCenter x within safe band");
    // y = vh - safe.bottom - h - off = 600 - 60 - 40 - 10 = 490.
    approx(r.y, 490.0, "BottomCenter y above safe bottom");
}

#[test]
fn no_anchor_lays_out_at_origin() {
    // A root WITHOUT a UiAnchor lays out at the viewport top-left (the default).
    let mut w = world_1920();
    let n = w.spawn_with(move |cmds| {
        let mut ec = cmds.spawn(sized(200.0, 100.0));
        ec.insert(boyko_ui::components::ComputedRect::default());
        ec.insert(boyko_ui::components::UiRoot);
        ec.id()
    });
    w.run();
    let r = w.rect(n);
    approx(r.x, 0.0, "no-anchor x at origin");
    approx(r.y, 0.0, "no-anchor y at origin");
}

#[test]
fn anchor_safe_area_ignored_when_flag_false() {
    // use_safe_area: false must ignore the inset even when one is set.
    let mut w = world_1920();
    w.set_safe_area(UiSafeArea { left: 99.0, top: 99.0, right: 99.0, bottom: 99.0 });
    let n = w.spawn_anchored_root(
        sized(200.0, 100.0),
        anchor(AnchorEdge::BottomRight, 0.0, 0.0, false),
    );
    w.run();
    let r = w.rect(n);
    approx(r.x, 1920.0 - 200.0, "safe-area ignored: x");
    approx(r.y, 1080.0 - 100.0, "safe-area ignored: y");
}
