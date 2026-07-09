//! Harness smoke test: validates the integration harness compiles and the basic
//! spawn -> link -> run -> read-rect round-trip works before the full suite.

mod common;

use common::{approx_rect, NodeSpec, Ui};

use boyko_ui::components::UiLayout;
use boyko_ui::units::{LayoutType, Unit};

#[test]
fn harness_column_two_fixed_children_round_trip() {
    let mut ui = Ui::default_world();

    let root = ui.spawn_root(UiLayout {
        layout_type: LayoutType::Column,
        width: Unit::Px(200.0),
        height: Unit::Px(300.0),
        ..UiLayout::default()
    });
    let a = ui.spawn_child(
        UiLayout { height: Unit::Px(50.0), ..UiLayout::default() },
        root,
    );
    let b = ui.spawn_child(
        UiLayout { height: Unit::Px(70.0), ..UiLayout::default() },
        root,
    );

    ui.run();

    // Column: children stack on y at 0 and 50; default cross (width) is Auto, so
    // each child's width is its own content (0 here — no ContentSize). The exact
    // values are pinned in the dedicated tests; here we only assert the round trip
    // produced finite, stacked rects.
    let ra = ui.rect(a);
    let rb = ui.rect(b);
    approx_rect(ra, 0.0, 0.0, ra.w, 50.0, "child a");
    approx_rect(rb, 0.0, 50.0, rb.w, 70.0, "child b");

    let _ = NodeSpec::default();
}
