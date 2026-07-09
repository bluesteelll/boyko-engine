//! Core layout-value tests — the mandatory suite from the plan's "Metrics and
//! validation" section: Row/Column, Auto-cross, Pct (of-Auto-main and
//! of-definite), Overlay, Auto/content, Px/Pct, min/max non-stretch.
//!
//! Each test uses the plan's exact expected values. Where the plan leaves the
//! cross axis Auto (so a child's cross extent = its content, 0 without a
//! ContentSize), the assertion pins the documented behavior.

mod common;

use common::{approx, approx_rect, NodeSpec, Ui};

use boyko_ui::components::{ContentSize, UiAlign, UiLayout, UiSpacing};
use boyko_ui::units::{AlignCross, LayoutType, Unit};

fn col(width: Unit, height: Unit) -> UiLayout {
    UiLayout { layout_type: LayoutType::Column, width, height, ..UiLayout::default() }
}
fn row(width: Unit, height: Unit) -> UiLayout {
    UiLayout { layout_type: LayoutType::Row, width, height, ..UiLayout::default() }
}
fn px(v: f32) -> Unit {
    Unit::Px(v)
}

// ───────────────────────── Row / Column ───────────────────────────────────

#[test]
fn column_stacks_children_on_y_with_row_gap() {
    // Column 200×Auto, three Px(50) h children, row_gap=Px(10) -> y = 0,60,120;
    // Auto h = 170.
    //
    // The Auto-hugging container is nested under a fixed root: a ROOT with an Auto
    // axis is forced to the viewport extent (the documented "default Auto root ->
    // fills viewport" rule), so Auto-hugging is verified on a non-root container.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(col(px(200.0), Unit::Auto))
            .with_spacing(UiSpacing { row_gap: px(10.0), ..UiSpacing::default() }),
        Some(root),
    );
    let c0 = ui.spawn_child(col(px(50.0), px(50.0)), cont);
    let c1 = ui.spawn_child(col(px(50.0), px(50.0)), cont);
    let c2 = ui.spawn_child(col(px(50.0), px(50.0)), cont);
    ui.run();

    approx(ui.rect(c0).y, 0.0, "c0.y");
    approx(ui.rect(c1).y, 60.0, "c1.y");
    approx(ui.rect(c2).y, 120.0, "c2.y");
    approx(ui.rect(cont).h, 170.0, "container Auto height = 3*50 + 2*10");
}

#[test]
fn row_lays_children_on_x_with_column_gap() {
    // Row Auto×100, two Px(80) w children, column_gap=Px(20) -> x = 0,100.
    //
    // Nested under a Row root (SAME orientation) so this isolates the gap +
    // x-flow behavior. (A Row container nested under a COLUMN root hits the
    // mixed-orientation axis-fold bug documented separately in
    // `mixed_orientation_row_under_column_swaps_w_h` — kept out of this test so
    // the gap feature is asserted independently of that bug.)
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(row(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(row(Unit::Auto, px(100.0)))
            .with_spacing(UiSpacing { column_gap: px(20.0), ..UiSpacing::default() }),
        Some(root),
    );
    let c0 = ui.spawn_child(row(px(80.0), px(40.0)), cont);
    let c1 = ui.spawn_child(row(px(80.0), px(40.0)), cont);
    ui.run();

    approx(ui.rect(c0).x, 0.0, "c0.x");
    approx(ui.rect(c1).x, 100.0, "c1.x = 80 + 20 gap");
    approx(ui.rect(cont).w, 180.0, "container Auto width = 80 + 20 + 80");
    approx(ui.rect(cont).h, 100.0, "container cross height = Px(100)");
}

/// REGRESSION (documents a real bug): a Row container nested under a Column root
/// (mismatched axis orientation) gets its resolved width/height SWAPPED, because
/// the parent feeds `force_def`/`child_size` in the parent's (Column) axis frame
/// but the child's `measure_node` interprets `main`/`cross` in its own (Row)
/// frame. Expected: Row container hugs main(x)=180, cross(y)=100. Actual (buggy):
/// w=100, h=180. The inner children are placed correctly; only the container's
/// own rect dimensions are transposed.
///
/// This is an ALGORITHM bug (axis-frame mismatch in `position_descendants`'s
/// `force_def` round-trip), not a wrong expectation.
#[test]
fn mixed_orientation_row_under_column_swaps_w_h() {
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(row(Unit::Auto, px(100.0)))
            .with_spacing(UiSpacing { column_gap: px(20.0), ..UiSpacing::default() }),
        Some(root),
    );
    let _c0 = ui.spawn_child(row(px(80.0), px(40.0)), cont);
    let _c1 = ui.spawn_child(row(px(80.0), px(40.0)), cont);
    ui.run();

    let r = ui.rect(cont);
    approx(r.w, 180.0, "Row-under-Column container width should hug 80+20+80");
    approx(r.h, 100.0, "Row-under-Column container height should be Px(100)");
}

#[test]
fn axis_fold_transposes_row_vs_column() {
    // Same child set in Row vs Column produces transposed coordinates: a Column
    // advances on y, a Row advances on x.
    let mut col_ui = Ui::default_world();
    let col_root = col_ui.spawn_root(col(px(200.0), px(200.0)));
    let cc0 = col_ui.spawn_child(col(px(30.0), px(40.0)), col_root);
    let cc1 = col_ui.spawn_child(col(px(30.0), px(40.0)), col_root);
    col_ui.run();

    let mut row_ui = Ui::default_world();
    let row_root = row_ui.spawn_root(row(px(200.0), px(200.0)));
    let rc0 = row_ui.spawn_child(row(px(40.0), px(30.0)), row_root);
    let rc1 = row_ui.spawn_child(row(px(40.0), px(30.0)), row_root);
    row_ui.run();

    // Column advances the second child on y; Row advances it on x — transposed.
    approx(col_ui.rect(cc0).y, 0.0, "col c0.y");
    approx(col_ui.rect(cc1).y, 40.0, "col c1.y (main = y)");
    approx(row_ui.rect(rc0).x, 0.0, "row c0.x");
    approx(row_ui.rect(rc1).x, 40.0, "row c1.x (main = x)");
}

// ───────────────────────── Auto-cross (Pass-C cross fold) ──────────────────

#[test]
fn column_auto_cross_hugs_widest_child() {
    // Column Auto×Auto with children of differing widths (Px(40), Px(120),
    // Px(80)) -> container Auto cross (width) = 120. Nested under a fixed root so
    // the Auto axes hug instead of filling the viewport.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(1000.0), px(800.0)));
    let cont = ui.spawn(NodeSpec::new(col(Unit::Auto, Unit::Auto)), Some(root));
    let _a = ui.spawn_child(col(px(40.0), px(10.0)), cont);
    let _b = ui.spawn_child(col(px(120.0), px(10.0)), cont);
    let _c = ui.spawn_child(col(px(80.0), px(10.0)), cont);
    ui.run();

    approx(ui.rect(cont).w, 120.0, "Auto cross width = max child width");
}

#[test]
fn column_auto_cross_includes_cross_padding() {
    // Auto cross folds the widest child PLUS the cross padding/border.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(col(Unit::Auto, Unit::Auto)).with_spacing(UiSpacing {
            padding_left: px(5.0),
            padding_right: px(7.0),
            ..UiSpacing::default()
        }),
        Some(root),
    );
    let _a = ui.spawn_child(col(px(60.0), px(10.0)), cont);
    ui.run();

    approx(ui.rect(cont).w, 72.0, "Auto cross = 60 + 5 + 7 padding");
}

// ───────────────────────── Pct-of-Auto-main / Pct-of-definite ─────────────

#[test]
fn pct_main_of_auto_container_resolves_to_child_content() {
    // Column Auto main, a Px(100) h child and a Pct(50) h child whose own content
    // is Px(30) -> the Pct child resolves to its content (30) for the Auto total
    // (CSS indefinite-container behavior); container Auto main = 130.
    // (Auto-main container nested under a fixed root so it hugs.)
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(1000.0), px(800.0)));
    let cont = ui.spawn(NodeSpec::new(col(px(200.0), Unit::Auto)), Some(root));
    let _fixed = ui.spawn_child(col(px(50.0), px(100.0)), cont);
    // The Pct(50) height child hugs a Px(30) content via a fixed-height grandchild.
    let pct_child = ui.spawn_child(col(px(50.0), Unit::Pct(50.0)), cont);
    let _grand = ui.spawn_child(col(px(50.0), px(30.0)), pct_child);
    ui.run();

    approx(ui.rect(cont).h, 130.0, "Auto main = 100 + content(30) of Pct-of-indefinite child");
}

#[test]
fn pct_main_of_definite_container_resolves_against_size() {
    // Column Px(200) main, a Pct(50) h child -> 100 (definite base).
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(150.0), px(200.0)));
    let child = ui.spawn_child(col(px(50.0), Unit::Pct(50.0)), root);
    ui.run();

    approx(ui.rect(child).h, 100.0, "Pct(50) of definite 200 main = 100");
}

// ───────────────────────── Px / Pct ───────────────────────────────────────

#[test]
fn pct_cross_of_definite_parent() {
    // Pct(50) width of a Px(200) cross parent -> 100.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(200.0), px(300.0)));
    let child = ui.spawn_child(col(Unit::Pct(50.0), px(20.0)), root);
    ui.run();

    approx(ui.rect(child).w, 100.0, "Pct(50) of definite 200 cross = 100");
}

#[test]
fn pct_base_reduced_by_parent_padding() {
    // Pct base = parent content box, i.e. reduced by parent padding. Column
    // Px(200) cross with padding_left=Px(10), padding_right=Px(0): content cross =
    // 190, Pct(50) child width = 95.
    let mut ui = Ui::default_world();
    let root = ui.spawn(
        NodeSpec::root(col(px(200.0), px(300.0)))
            .with_spacing(UiSpacing { padding_left: px(10.0), ..UiSpacing::default() }),
        None,
    );
    let child = ui.spawn_child(col(Unit::Pct(50.0), px(20.0)), root);
    ui.run();

    approx(ui.rect(child).w, 95.0, "Pct(50) of (200-10) content cross = 95");
    approx(ui.rect(child).x, 10.0, "child x offset by padding_left");
}

// ───────────────────────── Overlay ────────────────────────────────────────

#[test]
fn overlay_children_share_box_positioned_by_align() {
    // Plan-mandated: Overlay 300×300, two Px(100)^2 children: {Start,Start} ->
    // (0,0), {Center,Center} -> (100,100); both share the box.
    //
    // The {align} notation in the plan is PER-CHILD (each child carries its own
    // UiAlign). The current implementation positions every overlay child by the
    // CONTAINER's UiAlign (`position_overlay` reads `m.align`, not the child's),
    // so a per-child centered overlay child is NOT centered. This test asserts the
    // plan's per-child expectation and therefore FAILS against the current code —
    // a spec/impl gap (overlay lacks per-child align-self). See
    // `overlay_positions_children_by_container_align` for the actual behavior.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(UiLayout {
        layout_type: LayoutType::Overlay,
        width: px(300.0),
        height: px(300.0),
        ..UiLayout::default()
    });
    let start = ui.spawn(
        NodeSpec::new(UiLayout {
            layout_type: LayoutType::Overlay,
            width: px(100.0),
            height: px(100.0),
            ..UiLayout::default()
        })
        .with_align(UiAlign::default()),
        Some(root),
    );
    let center = ui.spawn(
        NodeSpec::new(UiLayout {
            layout_type: LayoutType::Overlay,
            width: px(100.0),
            height: px(100.0),
            ..UiLayout::default()
        })
        .with_align(UiAlign {
            main: boyko_ui::units::AlignMain::Center,
            cross: AlignCross::Center,
        }),
        Some(root),
    );
    ui.run();

    approx_rect(ui.rect(start), 0.0, 0.0, 100.0, 100.0, "overlay start child");
    approx_rect(ui.rect(center), 100.0, 100.0, 100.0, 100.0, "overlay centered child");
}

#[test]
fn overlay_positions_children_by_container_align() {
    // Documents the ACTUAL overlay behavior: the CONTAINER's UiAlign places every
    // child. With container align {Center,Center}, a Px(100)^2 child in a 300×300
    // overlay is centered at (100,100). Both children share the box.
    let mut ui = Ui::default_world();
    let root = ui.spawn(
        NodeSpec::root(UiLayout {
            layout_type: LayoutType::Overlay,
            width: px(300.0),
            height: px(300.0),
            ..UiLayout::default()
        })
        .with_align(UiAlign {
            main: boyko_ui::units::AlignMain::Center,
            cross: AlignCross::Center,
        }),
        None,
    );
    let a = ui.spawn_child(
        UiLayout {
            layout_type: LayoutType::Overlay,
            width: px(100.0),
            height: px(100.0),
            ..UiLayout::default()
        },
        root,
    );
    let b = ui.spawn_child(
        UiLayout {
            layout_type: LayoutType::Overlay,
            width: px(100.0),
            height: px(100.0),
            ..UiLayout::default()
        },
        root,
    );
    ui.run();

    approx_rect(ui.rect(a), 100.0, 100.0, 100.0, 100.0, "container-centered child a");
    approx_rect(ui.rect(b), 100.0, 100.0, 100.0, 100.0, "container-centered child b (shares box)");
}

// ───────────────────────── Auto / content (no text) ───────────────────────

#[test]
fn auto_leaf_without_content_is_zero() {
    // Auto leaf, no ContentSize, no children -> 0×0.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(100.0), px(100.0)));
    let leaf = ui.spawn_child(col(Unit::Auto, Unit::Auto), root);
    ui.run();

    approx(ui.rect(leaf).w, 0.0, "Auto leaf no content -> w 0");
    approx(ui.rect(leaf).h, 0.0, "Auto leaf no content -> h 0");
}

#[test]
fn auto_leaf_with_content_hugs_it() {
    // Auto leaf + ContentSize{40,12} -> 40×12.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(100.0), px(100.0)));
    let leaf = ui.spawn(
        NodeSpec::new(col(Unit::Auto, Unit::Auto))
            .with_content(ContentSize { width: 40.0, height: 12.0 }),
        Some(root),
    );
    ui.run();

    approx(ui.rect(leaf).w, 40.0, "Auto leaf hugs content width");
    approx(ui.rect(leaf).h, 12.0, "Auto leaf hugs content height");
}

#[test]
fn auto_container_hugs_children_main() {
    // Column Auto, two Px(30) h children -> h = 60. (Nested under fixed root.)
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(1000.0), px(800.0)));
    let cont = ui.spawn(NodeSpec::new(col(px(100.0), Unit::Auto)), Some(root));
    let _a = ui.spawn_child(col(px(10.0), px(30.0)), cont);
    let _b = ui.spawn_child(col(px(10.0), px(30.0)), cont);
    ui.run();

    approx(ui.rect(cont).h, 60.0, "Auto container hugs 30+30");
}

#[test]
fn auto_container_hugs_children_main_with_padding() {
    // Auto container + padding adds to the hugged main. (Nested under fixed root.)
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(col(px(100.0), Unit::Auto)).with_spacing(UiSpacing {
            padding_top: px(4.0),
            padding_bottom: px(6.0),
            ..UiSpacing::default()
        }),
        Some(root),
    );
    let _a = ui.spawn_child(col(px(10.0), px(30.0)), cont);
    ui.run();

    approx(ui.rect(cont).h, 40.0, "Auto main 30 + 4 + 6 padding");
}

// ───────────────────────── min / max (non-stretch) ────────────────────────

#[test]
fn max_width_clamps_oversized_px() {
    // Px(500) max_width=Px(200) -> 200.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(row(px(800.0), px(100.0)));
    let child = ui.spawn_child(
        UiLayout {
            layout_type: LayoutType::Row,
            width: px(500.0),
            height: px(20.0),
            max_width: px(200.0),
            ..UiLayout::default()
        },
        root,
    );
    ui.run();

    approx(ui.rect(child).w, 200.0, "max_width clamps 500 -> 200");
}

#[test]
fn min_width_raises_undersized_px() {
    // Px(10) min_width=Px(50) -> 50.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(row(px(800.0), px(100.0)));
    let child = ui.spawn_child(
        UiLayout {
            layout_type: LayoutType::Row,
            width: px(10.0),
            height: px(20.0),
            min_width: px(50.0),
            ..UiLayout::default()
        },
        root,
    );
    ui.run();

    approx(ui.rect(child).w, 50.0, "min_width raises 10 -> 50");
}
