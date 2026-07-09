//! Nesting (incl. the 4-deep depth-pool isolation test), structural changes
//! (reparent-across-roots + despawn-of-middle), and determinism.

mod common;

use common::{approx, NodeSpec, Ui};

use boyko_ui::components::UiLayout;
use boyko_ui::units::{LayoutType, Unit};

fn col(width: Unit, height: Unit) -> UiLayout {
    UiLayout { layout_type: LayoutType::Column, width, height, ..UiLayout::default() }
}
fn px(v: f32) -> Unit {
    Unit::Px(v)
}

// ───────────────────────── nesting / reflow ───────────────────────────────

#[test]
fn nested_stretch_columns_reflow() {
    // Column(root 400×600) -> two Stretch-height children, each a Column with Px
    // children. The inner columns must reflow: their Px children land at the
    // inner column's absolute origin, proving the inner subtree was positioned
    // (>= the descendant reflow ran).
    let mut ui = Ui::new(boyko_ui::resources::UiViewport {
        width: 400.0,
        height: 600.0,
        scale_factor: 1.0,
        generation: 0,
    });
    let root = ui.spawn_root(col(px(400.0), px(600.0)));
    // Two children stretching the main (height) axis 1:1 -> 300 each.
    let top = ui.spawn_child(col(px(400.0), Unit::Stretch(1.0)), root);
    let bottom = ui.spawn_child(col(px(400.0), Unit::Stretch(1.0)), root);
    let top_a = ui.spawn_child(col(px(100.0), px(40.0)), top);
    let bottom_a = ui.spawn_child(col(px(100.0), px(40.0)), bottom);
    ui.run();

    approx(ui.rect(top).h, 300.0, "top stretch = 300");
    approx(ui.rect(bottom).h, 300.0, "bottom stretch = 300");
    approx(ui.rect(top).y, 0.0, "top at y 0");
    approx(ui.rect(bottom).y, 300.0, "bottom at y 300");
    // Inner Px children sit at their parent column's origin (descendant reflow).
    approx(ui.rect(top_a).y, 0.0, "top inner child at top column origin");
    approx(ui.rect(bottom_a).y, 300.0, "bottom inner child at bottom column origin (reflowed)");
}

#[test]
fn four_deep_with_siblings_isolates_depth_pools() {
    // The depth-pool isolation test (Decision 6): a 4-deep Column tree with TWO
    // siblings at each level. A parent is mid-loop (positioning sibling 0 then
    // sibling 1) while each sibling recurses into its own 2-child subtree. If the
    // child/stretch/size pools were shared across depths, recursing into a child
    // would clobber the parent's loop and corrupt sibling 1's position. We verify
    // EVERY node's rect.
    //
    // Layout (all fixed Px so the expected values are exact):
    //   root (Column 200x1000)
    //     L1a (h=100)            y=0
    //       L2a (h=20) y=0, L2b (h=20) y=20   (relative to L1a origin y=0)
    //     L1b (h=100)            y=100
    //       L2c (h=20) y=100, L2d (h=20) y=120
    //
    // Extend to 4 deep on one branch to exercise depth 0..=3:
    //   L2a -> L3a(h=8) y=0, L3b(h=8) y=8
    //     L3a -> L4a(h=4) y=0, L4b(h=4) y=4
    let mut ui = Ui::new(boyko_ui::resources::UiViewport {
        width: 200.0,
        height: 1000.0,
        scale_factor: 1.0,
        generation: 0,
    });
    let root = ui.spawn_root(col(px(200.0), px(1000.0)));
    let l1a = ui.spawn_child(col(px(200.0), px(100.0)), root);
    let l1b = ui.spawn_child(col(px(200.0), px(100.0)), root);
    let l2a = ui.spawn_child(col(px(200.0), px(20.0)), l1a);
    let l2b = ui.spawn_child(col(px(200.0), px(20.0)), l1a);
    let l2c = ui.spawn_child(col(px(200.0), px(20.0)), l1b);
    let l2d = ui.spawn_child(col(px(200.0), px(20.0)), l1b);
    let l3a = ui.spawn_child(col(px(200.0), px(8.0)), l2a);
    let l3b = ui.spawn_child(col(px(200.0), px(8.0)), l2a);
    let l4a = ui.spawn_child(col(px(200.0), px(4.0)), l3a);
    let l4b = ui.spawn_child(col(px(200.0), px(4.0)), l3a);
    ui.run();

    // Depth-1 siblings: l1b must NOT be corrupted by recursion into l1a's subtree.
    approx(ui.rect(l1a).y, 0.0, "L1a y");
    approx(ui.rect(l1b).y, 100.0, "L1b y (uncorrupted by L1a recursion)");
    // Depth-2 siblings under each L1.
    approx(ui.rect(l2a).y, 0.0, "L2a y");
    approx(ui.rect(l2b).y, 20.0, "L2b y (uncorrupted by L2a recursion)");
    approx(ui.rect(l2c).y, 100.0, "L2c y under L1b");
    approx(ui.rect(l2d).y, 120.0, "L2d y under L1b");
    // Depth-3 siblings under L2a (absolute coords add L2a origin y=0).
    approx(ui.rect(l3a).y, 0.0, "L3a y");
    approx(ui.rect(l3b).y, 8.0, "L3b y (uncorrupted by L3a recursion into depth 4)");
    // Depth-4 leaves under L3a.
    approx(ui.rect(l4a).y, 0.0, "L4a y");
    approx(ui.rect(l4b).y, 4.0, "L4b y (deepest sibling, uncorrupted)");
}

// ───────────────────────── structural: reparent across roots ──────────────

#[test]
fn reparent_across_roots_relays_both() {
    // Two roots A, B. Move a subtree from a child of A to a child of B. After
    // discovery+apply, BOTH roots are relaid: A reclaims the vacated flow (its
    // remaining child moves up), B accommodates the arrival (its column grows /
    // the moved node gets a position under B).
    let mut ui = Ui::new(boyko_ui::resources::UiViewport {
        width: 1000.0,
        height: 1000.0,
        scale_factor: 1.0,
        generation: 0,
    });
    // Root A: a column holding parent_a with two children (first + movable).
    let root_a = ui.spawn_root(col(px(300.0), px(1000.0)));
    let parent_a = ui.spawn_child(col(px(300.0), Unit::Auto), root_a);
    let a_first = ui.spawn_child(col(px(300.0), px(40.0)), parent_a);
    let movable = ui.spawn_child(col(px(300.0), px(40.0)), parent_a);
    // Root B: a column holding parent_b (initially empty -> 0 main).
    let root_b = ui.spawn_root(col(px(300.0), px(1000.0)));
    let parent_b = ui.spawn_child(col(px(300.0), Unit::Auto), root_b);
    ui.run();

    // Initially: parent_a hugs two 40-children -> y of movable = 40; parent_b 0.
    approx(ui.rect(movable).y, 40.0, "movable starts as parent_a's 2nd child at y=40");
    approx(ui.rect(parent_b).h, 0.0, "parent_b initially empty -> 0 main");

    // Reparent `movable` from parent_a to parent_b.
    ui.set_parent(movable, parent_b);
    ui.run();

    // Root A relaid: parent_a now hugs one child -> a_first at 0, parent_a h = 40.
    approx(ui.rect(a_first).y, 0.0, "A reclaimed: a_first stays at 0");
    approx(ui.rect(parent_a).h, 40.0, "A reclaimed vacated flow: parent_a hugs one child");
    // Root B relaid: parent_b now hosts movable at its origin.
    approx(ui.rect(parent_b).h, 40.0, "B accommodated arrival: parent_b hugs the moved 40-child");
    approx(ui.rect(movable).y, 0.0, "moved node is parent_b's first child at y=0");
}

// ───────────────────────── structural: despawn of middle ──────────────────

#[test]
fn despawn_middle_child_reflows_remaining() {
    // Spawn a 3-child column, despawn the middle, run -> the remaining two reflow
    // (the third moves up into the vacated slot).
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(200.0), px(600.0)));
    let c0 = ui.spawn_child(col(px(200.0), px(50.0)), root);
    let c1 = ui.spawn_child(col(px(200.0), px(50.0)), root);
    let c2 = ui.spawn_child(col(px(200.0), px(50.0)), root);
    ui.run();

    approx(ui.rect(c0).y, 0.0, "c0 at 0");
    approx(ui.rect(c1).y, 50.0, "c1 at 50");
    approx(ui.rect(c2).y, 100.0, "c2 at 100");

    ui.despawn(c1);
    ui.run();

    approx(ui.rect(c0).y, 0.0, "c0 stays at 0");
    approx(ui.rect(c2).y, 50.0, "c2 reflowed up into the vacated middle slot");
}

// ───────────────────────── determinism ────────────────────────────────────

#[test]
fn flow_order_is_deterministic_after_sibling_removal() {
    // Spawn children, remove a middle sibling (triggers Children swap_remove on
    // the parent), relayout -> the remaining children keep id-sorted flow order
    // (the swap_remove of an unrelated sibling does NOT reorder the visual flow).
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(200.0), px(600.0)));
    let c0 = ui.spawn_child(col(px(200.0), px(30.0)), root);
    let c1 = ui.spawn_child(col(px(200.0), px(30.0)), root);
    let c2 = ui.spawn_child(col(px(200.0), px(30.0)), root);
    let c3 = ui.spawn_child(col(px(200.0), px(30.0)), root);
    ui.run();

    // Remove c1 (a middle sibling). Children uses swap_remove, so c3 may take c1's
    // slot in the raw Children order — but layout sorts by Entity::id, so the
    // visual order stays c0, c2, c3 (ascending id).
    ui.despawn(c1);
    ui.run();

    let y0 = ui.rect(c0).y;
    let y2 = ui.rect(c2).y;
    let y3 = ui.rect(c3).y;
    approx(y0, 0.0, "c0 first");
    approx(y2, 30.0, "c2 second (id order preserved despite swap_remove)");
    approx(y3, 60.0, "c3 third (id order preserved)");
    assert!(y0 < y2 && y2 < y3, "flow order is the ascending-id order, deterministic");
}

#[test]
fn relayout_is_idempotent_across_runs() {
    // Determinism across frames: running the same unchanged tree twice produces
    // bit-identical rects.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(col(px(200.0), px(300.0)));
    let a = ui.spawn_child(col(px(50.0), px(40.0)), root);
    let b = ui.spawn_child(col(px(80.0), px(60.0)), root);
    let _ = NodeSpec::default();
    ui.run();
    let ra1 = ui.rect(a);
    let rb1 = ui.rect(b);
    ui.run();
    let ra2 = ui.rect(a);
    let rb2 = ui.rect(b);

    assert_eq!(ra1, ra2, "child a rect identical across runs");
    assert_eq!(rb1, rb2, "child b rect identical across runs");
}
