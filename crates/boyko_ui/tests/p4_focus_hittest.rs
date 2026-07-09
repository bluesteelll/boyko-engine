//! GATE 2 — HIT-TEST: Z-order total order, ComputedRect containment,
//! FocusPolicy::Block, RelativeCursorPosition correctness, and a HiDPI
//! (scale_factor=2.0) hit-test.
//!
//! Drives the exclusive `ui_focus_system` against a scripted `PhysicalInput`
//! cursor + mouse edges and asserts the resolved `Interaction`/relative-cursor
//! state. Each node is an interactive root unless parented (the focus DFS walks
//! `UiRoot`s; a child shares its root's paint subtree).

mod p4_common;

use p4_common::{InterWorld, NodeOpts};

use boyko_ui::interaction::components::Interaction;

// ───────────────────────── containment ─────────────────────────────────────

#[test]
fn hittest_cursor_inside_rect_hovers_the_node() {
    let mut w = InterWorld::new();
    let n = w.spawn_node_cfg(10.0, 10.0, 100.0, 50.0, NodeOpts::root(), None);
    w.set_cursor(50.0, 30.0);
    w.set_mouse(false, false, false);
    w.focus();
    assert_eq!(w.interaction(n), Interaction::Hovered, "cursor inside rect hovers");
    assert!(w.is_ui_hovered(n), "UiHovered bit set on hover");
}

#[test]
fn hittest_cursor_outside_rect_leaves_none() {
    let mut w = InterWorld::new();
    let n = w.spawn_node_cfg(10.0, 10.0, 100.0, 50.0, NodeOpts::root(), None);
    w.set_cursor(500.0, 500.0);
    w.focus();
    assert_eq!(w.interaction(n), Interaction::None, "cursor outside rect → None");
    assert!(!w.is_ui_hovered(n), "UiHovered bit clear when not hovered");
}

#[test]
fn hittest_far_edge_is_half_open() {
    // point_in_rect is half-open on the far edge: x in [x, x+w).
    let mut w = InterWorld::new();
    let n = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root(), None);
    // Exactly on the far edge (x == 100) → NOT inside.
    w.set_cursor(100.0, 50.0);
    w.focus();
    assert_eq!(w.interaction(n), Interaction::None, "far edge x==x+w is outside (half-open)");
    // Exactly on the near edge (x == 0) → inside (closed).
    w.set_cursor(0.0, 50.0);
    w.focus();
    assert_eq!(w.interaction(n), Interaction::Hovered, "near edge x==x is inside (closed)");
}

// ───────────────────────── Z-order total order ─────────────────────────────

#[test]
fn hittest_higher_stackindex_wins_over_overlap() {
    // Two fully-overlapping nodes; the higher StackIndex is the hovered one.
    let mut w = InterWorld::new();
    let low = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_stack(0), None);
    let high = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_stack(5), None);
    w.set_cursor(50.0, 50.0);
    w.focus();
    assert_eq!(w.interaction(high), Interaction::Hovered, "top-most StackIndex wins");
    assert_eq!(w.interaction(low), Interaction::None, "occluded lower node → None");
}

#[test]
fn hittest_tie_stackindex_paint_order_child_on_top() {
    // Parent + child both StackIndex 0 (default). The child is painted after the
    // parent (DFS document order), so it is on top — the documented overlap
    // resolution.
    let mut w = InterWorld::new();
    let parent = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root(), None);
    let child = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::default(), Some(parent));
    w.set_cursor(50.0, 50.0);
    w.focus();
    assert_eq!(w.interaction(child), Interaction::Hovered, "later-painted child wins the tie");
    assert_eq!(w.interaction(parent), Interaction::None, "parent occluded by its child");
}

#[test]
fn hittest_tie_is_deterministic_across_frames() {
    // The total order is a STABLE comparison; re-running the same scene yields the
    // same hovered node every frame.
    let mut w = InterWorld::new();
    let a = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root(), None);
    let b = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root(), None);
    w.set_cursor(50.0, 50.0);
    w.focus();
    let first = if w.interaction(a) == Interaction::Hovered { a } else { b };
    for _ in 0..5 {
        w.focus();
        let now = if w.interaction(a) == Interaction::Hovered { a } else { b };
        assert_eq!(now, first, "tie-break is deterministic frame-to-frame");
    }
}

// ───────────────────────── FocusPolicy::Block ──────────────────────────────

#[test]
fn hittest_block_node_stops_hover_and_resets_occluded_lower() {
    // A Block node painted on top occludes the lower node: the Block node is
    // hovered, the lower node is reset to None even though the cursor is inside it.
    let mut w = InterWorld::new();
    let lower = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root(), None);
    // First hover the lower node alone so it is Hovered.
    w.set_cursor(50.0, 50.0);
    w.focus();
    assert_eq!(w.interaction(lower), Interaction::Hovered, "lower hovered before block exists");

    // Now add a Block node fully over it (a later root → higher paint_seq).
    let block = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_block(true), None);
    w.focus();
    assert_eq!(w.interaction(block), Interaction::Hovered, "Block node becomes hovered");
    assert_eq!(
        w.interaction(lower),
        Interaction::None,
        "node occluded by Block is reset to None (unconditional reset pass)"
    );
    assert!(!w.is_ui_hovered(lower), "occluded node loses its UiHovered bit");
}

// ───────────────────────── ComputedClip ────────────────────────────────────

#[test]
fn hittest_outside_clip_is_not_hovered() {
    use boyko_ui::components::ComputedClip;
    // A node whose rect contains the cursor but whose clip excludes it is NOT
    // hovered (clip narrows the hit region).
    let mut w = InterWorld::new();
    let clip = ComputedClip { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
    let n = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_clip(clip), None);
    // Inside the rect (0..100) but outside the clip (0..40).
    w.set_cursor(60.0, 60.0);
    w.focus();
    assert_eq!(w.interaction(n), Interaction::None, "cursor outside clip → not hovered");
    // Inside both.
    w.set_cursor(20.0, 20.0);
    w.focus();
    assert_eq!(w.interaction(n), Interaction::Hovered, "cursor inside clip → hovered");
}

// ───────────────────────── RelativeCursorPosition ──────────────────────────

#[test]
fn relative_cursor_center_is_zero() {
    let mut w = InterWorld::new();
    let n = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_relative_cursor(), None);
    w.set_cursor(50.0, 50.0); // exact center
    w.focus();
    let rel = w.rel(n).expect("node has RelativeCursorPosition");
    assert!(rel.cursor_over, "cursor_over set when hovered");
    assert!(rel.normalized[0].abs() < 1e-4, "center x normalized ~ 0, got {}", rel.normalized[0]);
    assert!(rel.normalized[1].abs() < 1e-4, "center y normalized ~ 0, got {}", rel.normalized[1]);
}

#[test]
fn relative_cursor_corners_are_plus_minus_half() {
    let mut w = InterWorld::new();
    let n = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_relative_cursor(), None);
    // Top-left corner → (-0.5, -0.5).
    w.set_cursor(0.0, 0.0);
    w.focus();
    let tl = w.rel(n).expect("rel present");
    assert!((tl.normalized[0] + 0.5).abs() < 1e-4, "top-left x ~ -0.5, got {}", tl.normalized[0]);
    assert!((tl.normalized[1] + 0.5).abs() < 1e-4, "top-left y ~ -0.5, got {}", tl.normalized[1]);
    // Near bottom-right (99.9,99.9) → ~ (+0.5, +0.5) (far edge is half-open).
    w.set_cursor(99.9, 99.9);
    w.focus();
    let br = w.rel(n).expect("rel present");
    assert!((br.normalized[0] - 0.499).abs() < 2e-3, "bottom-right x ~ +0.5, got {}", br.normalized[0]);
}

#[test]
fn relative_cursor_resets_to_canonical_on_leave() {
    let mut w = InterWorld::new();
    let n = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_relative_cursor(), None);
    w.set_cursor(20.0, 20.0);
    w.focus();
    assert!(w.rel(n).expect("rel").cursor_over, "over before leave");
    // Move off the node.
    w.set_cursor(500.0, 500.0);
    w.focus();
    let rel = w.rel(n).expect("rel");
    assert!(!rel.cursor_over, "cursor_over cleared on leave");
    assert_eq!(rel.normalized, [0.0, 0.0], "normalized reset to canonical [0,0] on leave");
}

// ───────────────────────── HiDPI (scale_factor = 2.0) ──────────────────────

#[test]
fn hittest_hidpi_scale_two_converts_physical_to_logical() {
    // scale_factor = 2.0: a PHYSICAL cursor at (200,200) is logical (100,100); a
    // logical rect at (0,0,150,150) is hit (the Decision-13 mandatory test).
    let mut w = InterWorld::with_scale(2.0);
    let n = w.spawn_node_cfg(0.0, 0.0, 150.0, 150.0, NodeOpts::root(), None);
    w.set_cursor(200.0, 200.0); // physical → logical (100,100)
    w.focus();
    assert_eq!(
        w.interaction(n),
        Interaction::Hovered,
        "HiDPI: physical (200,200)/2 = logical (100,100) hits a (0,0,150,150) logical rect"
    );
}

#[test]
fn hittest_hidpi_unscaled_compare_would_miss() {
    // Sanity: the same physical cursor (200,200) at scale 2.0 lands at logical
    // (100,100), INSIDE a (0,0,150,150) rect — but were the conversion skipped,
    // the raw 200 would be OUTSIDE the 150-wide rect. This pins that the scale is
    // genuinely applied (not a coincidental pass).
    let mut w = InterWorld::with_scale(2.0);
    let n = w.spawn_node_cfg(0.0, 0.0, 150.0, 150.0, NodeOpts::root(), None);
    // Physical (320,320) → logical (160,160): OUTSIDE the 150 rect.
    w.set_cursor(320.0, 320.0);
    w.focus();
    assert_eq!(
        w.interaction(n),
        Interaction::None,
        "HiDPI: physical (320,320)/2 = logical (160,160) is outside a 150 rect"
    );
}

#[test]
fn relative_cursor_hidpi_center_is_zero() {
    // At scale 2.0, physical (200,200) → logical (100,100) = center of a
    // (0,0,200,200) logical rect → normalized ~ 0.
    let mut w = InterWorld::with_scale(2.0);
    let n = w.spawn_node_cfg(0.0, 0.0, 200.0, 200.0, NodeOpts::root().with_relative_cursor(), None);
    w.set_cursor(200.0, 200.0);
    w.focus();
    let rel = w.rel(n).expect("rel");
    assert!(rel.cursor_over, "hovered at HiDPI center");
    assert!(rel.normalized[0].abs() < 1e-4, "HiDPI center x ~ 0, got {}", rel.normalized[0]);
    assert!(rel.normalized[1].abs() < 1e-4, "HiDPI center y ~ 0, got {}", rel.normalized[1]);
}
