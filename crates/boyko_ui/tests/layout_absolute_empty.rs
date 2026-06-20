//! Absolute positioning + empty-container tests.

mod common;

use common::{approx, approx_rect, NodeSpec, Ui};

use boyko_ui::components::{UiAbsolute, UiLayout, UiSpacing};
use boyko_ui::units::{LayoutType, PositionType, Unit};

fn col(width: Unit, height: Unit) -> UiLayout {
    UiLayout { layout_type: LayoutType::Column, width, height, ..UiLayout::default() }
}
fn px(v: f32) -> Unit {
    Unit::Px(v)
}

#[test]
fn absolute_child_offsets_without_disturbing_flow() {
    // Container 200×200, relative Px(50) child + absolute UiAbsolute{left:Px(10),
    // top:Px(20)} Px(30)^2 -> relative at flow pos (0,0), absolute at (10,20),
    // relative flow UNAFFECTED.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(200.0), px(200.0)));
    let rel = ui.spawn_child(col(px(50.0), px(50.0)), root);
    let abs = ui.spawn(
        NodeSpec::new(UiLayout {
            layout_type: LayoutType::Column,
            position_type: PositionType::Absolute,
            width: px(30.0),
            height: px(30.0),
            ..UiLayout::default()
        })
        .with_absolute(UiAbsolute { left: px(10.0), top: px(20.0), ..UiAbsolute::default() }),
        Some(root),
    );
    ui.run();

    approx_rect(ui.rect(rel), 0.0, 0.0, 50.0, 50.0, "relative child at flow origin");
    approx_rect(ui.rect(abs), 10.0, 20.0, 30.0, 30.0, "absolute child at its offset");
}

#[test]
fn absolute_child_does_not_consume_flow_space() {
    // Two relative children stack normally even with an absolute sibling between
    // them in spawn order: the absolute one consumes no main-flow space.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(200.0), px(400.0)));
    let r0 = ui.spawn_child(col(px(50.0), px(50.0)), root);
    let _abs = ui.spawn(
        NodeSpec::new(UiLayout {
            layout_type: LayoutType::Column,
            position_type: PositionType::Absolute,
            width: px(30.0),
            height: px(30.0),
            ..UiLayout::default()
        })
        .with_absolute(UiAbsolute { left: px(5.0), top: px(5.0), ..UiAbsolute::default() }),
        Some(root),
    );
    let r1 = ui.spawn_child(col(px(50.0), px(60.0)), root);
    ui.run();

    approx(ui.rect(r0).y, 0.0, "first relative at 0");
    approx(ui.rect(r1).y, 50.0, "second relative directly after first (absolute took no flow space)");
}

#[test]
fn absolute_before_wins_over_after() {
    // left & top (before) both set alongside right & bottom (after) -> before
    // (left/top) wins.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(200.0), px(200.0)));
    let abs = ui.spawn(
        NodeSpec::new(UiLayout {
            layout_type: LayoutType::Column,
            position_type: PositionType::Absolute,
            width: px(20.0),
            height: px(20.0),
            ..UiLayout::default()
        })
        .with_absolute(UiAbsolute {
            left: px(15.0),
            top: px(25.0),
            right: px(99.0),
            bottom: px(99.0),
        }),
        Some(root),
    );
    ui.run();

    approx(ui.rect(abs).x, 15.0, "left (before) wins over right");
    approx(ui.rect(abs).y, 25.0, "top (before) wins over bottom");
}

#[test]
fn empty_container_hugs_padding_only() {
    // Column Auto, no children, padding=Px(5) all -> 10×10. No stretch loop, no
    // panic. (Nested under a fixed root so the Auto container hugs.)
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(col(Unit::Auto, Unit::Auto)).with_spacing(UiSpacing {
            padding_left: px(5.0),
            padding_right: px(5.0),
            padding_top: px(5.0),
            padding_bottom: px(5.0),
            ..UiSpacing::default()
        }),
        Some(root),
    );
    ui.run();

    approx(ui.rect(cont).w, 10.0, "empty Auto container width = 5 + 5 padding");
    approx(ui.rect(cont).h, 10.0, "empty Auto container height = 5 + 5 padding");
}

#[test]
fn empty_root_does_not_panic() {
    // A root with no children lays out (fills viewport) without panicking.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(Unit::Auto, Unit::Auto));
    ui.run();

    let r = ui.rect(root);
    assert!(r.w.is_finite() && r.h.is_finite(), "empty root rect is finite");
}
