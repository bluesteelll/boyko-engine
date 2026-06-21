//! GATE 1 — CLICK→ACTION fidelity (Decision 2):
//!   * press-inside + release-inside-same-node writes the OnClick action edge;
//!   * release OUTSIDE the press-origin does NOT;
//!   * mid-press despawn of the origin drops the click silently;
//!   * reparent (same id/gen) still fires.
//!
//! Drives `ui_focus_system` (which stamps `pending_click` at press and
//! `click_fired` at release-inside-same) then `ui_dispatch_system::<TestAction>`
//! (which re-validates the origin and writes `ActionState::ui_press`).

mod p4_common;

use p4_common::{InterWorld, NodeOpts, TestAction};

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_input::ActionState;
use boyko_ui::interaction::components::Interaction;
use boyko_ui::interaction::dispatch::ui_dispatch_system;

/// Runs focus then dispatch for one frame.
fn frame(w: &mut InterWorld) {
    w.focus();
    ui_dispatch_system::<TestAction>(&mut w.world);
}

/// Did the dispatch fire `action` this frame (live just_pressed edge)?
fn just_pressed(w: &InterWorld, action: TestAction) -> bool {
    w.world.resource::<ActionState<TestAction>>().just_pressed(action)
}

// ───────────────────────── press-in / release-in fires ─────────────────────

#[test]
fn click_press_in_release_in_same_node_fires_action() {
    let mut w = InterWorld::new();
    let btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(0), None);

    // Frame 1: press inside.
    w.set_cursor(50.0, 50.0);
    w.set_mouse(true, false, true);
    frame(&mut w);
    assert_eq!(w.interaction(btn), Interaction::Pressed, "node is Pressed during the press");
    assert_eq!(w.pending_click(), Some((btn, 0)), "press stamps pending_click with the action");
    assert!(!just_pressed(&w, TestAction::Jump), "no action fired on press-down (release-up only)");

    // Frame 2: release inside the same node.
    w.set_mouse(false, true, false);
    frame(&mut w);
    assert_eq!(w.click_fired(), Some((btn, 0)), "release-inside-same stamps click_fired");
    assert!(just_pressed(&w, TestAction::Jump), "release-up over origin fires the OnClick action");
}

#[test]
fn click_same_frame_press_release_fires() {
    // A press AND release in the SAME frame (a one-frame click) MUST still fire
    // the OnClick action (GATE 1 — press-inside + release-inside-same).
    //
    // KNOWN-FAILING — BUG-P4-CLICK-1 (real production bug in
    // `focus.rs::resolve_pointer`): on `clicked && released`, the release branch
    // (line ~502) consumes `pending_click` (sets it to `None`) AND stamps
    // `click_fired = Some((origin, 1))`; the FOLLOWING same-frame branch
    // (line ~515) then reads the now-`None` `pending_click`, falls to
    // `unwrap_or(NO_ACTION)`, and OVERWRITES `click_fired` with
    // `Some((origin, NO_ACTION))`. Dispatch skips `NO_ACTION`, so a one-frame
    // click silently fires NOTHING. Expected: the action index (1) survives.
    let mut w = InterWorld::new();
    let btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(1), None);
    w.set_cursor(50.0, 50.0);
    w.set_mouse(true, true, false);
    frame(&mut w);
    assert_eq!(
        w.click_fired(),
        Some((btn, 1)),
        "same-frame press+release must stamp click_fired with the action index (not NO_ACTION)"
    );
    assert!(just_pressed(&w, TestAction::Fire), "same-frame click fires the action");
}

// ───────────────────────── release-out cancels ─────────────────────────────

#[test]
fn click_release_outside_origin_does_not_fire() {
    let mut w = InterWorld::new();
    let btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(0), None);

    // Press inside.
    w.set_cursor(50.0, 50.0);
    w.set_mouse(true, false, true);
    frame(&mut w);
    assert_eq!(w.pending_click(), Some((btn, 0)), "press stamped");

    // Drag off the node, then release OUTSIDE.
    w.set_cursor(500.0, 500.0);
    w.set_mouse(false, true, false);
    frame(&mut w);
    assert_eq!(w.click_fired(), None, "release outside the origin does not stamp click_fired");
    assert!(!just_pressed(&w, TestAction::Jump), "drag-off then release fires nothing");
}

#[test]
fn click_release_over_different_node_does_not_fire_origin() {
    // Press on A, release while hovering a DIFFERENT node B → A's click is cancelled.
    let mut w = InterWorld::new();
    let a = w.spawn_node_cfg(0.0, 0.0, 50.0, 50.0, NodeOpts::root().with_click(0), None);
    let _b = w.spawn_node_cfg(200.0, 200.0, 50.0, 50.0, NodeOpts::root().with_click(1), None);

    w.set_cursor(25.0, 25.0); // over A
    w.set_mouse(true, false, true);
    frame(&mut w);
    assert_eq!(w.pending_click(), Some((a, 0)), "press stamped on A");

    w.set_cursor(225.0, 225.0); // over B
    w.set_mouse(false, true, false);
    frame(&mut w);
    assert_eq!(w.click_fired(), None, "release over B does not fire A's click");
    assert!(!just_pressed(&w, TestAction::Jump), "A's action not fired");
    assert!(!just_pressed(&w, TestAction::Fire), "B's action not fired (B was not the press origin)");
}

// ───────────────────────── mid-press despawn drops silently ─────────────────

#[test]
fn click_mid_press_despawn_drops_silently() {
    let mut w = InterWorld::new();
    let btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(0), None);

    // Press inside.
    w.set_cursor(50.0, 50.0);
    w.set_mouse(true, false, true);
    frame(&mut w);
    assert_eq!(w.pending_click(), Some((btn, 0)), "press stamped");

    // Despawn the origin mid-press.
    w.world.run_system(move |mut cmds: Commands| {
        cmds.entity(btn).despawn();
    });
    assert!(!w.world.has_entity(btn), "origin despawned");

    // Release where the node used to be.
    w.set_mouse(false, true, false);
    frame(&mut w);
    // dispatch re-validates via get_component_raw(origin, OnClick) → None → drop.
    assert!(
        !just_pressed(&w, TestAction::Jump),
        "mid-press despawn drops the click silently (re-validation fails)"
    );
}

// ───────────────────────── reparent (same id/gen) still fires ───────────────

#[test]
fn click_reparent_origin_still_fires() {
    // Reparenting does NOT change the Entity id/generation, so the release-time
    // re-validation (get_component_raw) still succeeds → the click fires.
    let mut w = InterWorld::new();
    let new_parent = w.spawn_node_cfg(0.0, 0.0, 300.0, 300.0, NodeOpts::root(), None);
    let btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(2), None);

    // Press inside btn.
    w.set_cursor(50.0, 50.0);
    w.set_mouse(true, false, true);
    frame(&mut w);
    let pending = w.pending_click();
    assert_eq!(pending, Some((btn, 2)), "press stamped on btn");

    // Reparent btn under new_parent (same id/gen; btn keeps OnClick + rect).
    let id_before = btn;
    w.world.run_system(move |mut cmds: Commands| {
        cmds.entity(btn).set_parent(new_parent);
    });
    assert!(w.world.has_entity(id_before), "reparented entity is the same live id/gen");

    // Release over the (still-alive, still-at-same-rect) node.
    w.set_cursor(50.0, 50.0);
    w.set_mouse(false, true, false);
    frame(&mut w);
    assert_eq!(w.click_fired(), Some((btn, 2)), "reparented origin still stamps click_fired");
    assert!(just_pressed(&w, TestAction::Menu), "reparented (same id/gen) origin still fires");
}

// ───────────────────────── NO_ACTION origin fires nothing ───────────────────

#[test]
fn click_origin_without_onclick_fires_nothing() {
    // A clickable node with no OnClick stamps NO_ACTION; dispatch fires nothing.
    let mut w = InterWorld::new();
    let n = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root(), None);
    w.set_cursor(50.0, 50.0);
    w.set_mouse(true, false, true);
    frame(&mut w);
    w.set_mouse(false, true, false);
    frame(&mut w);
    // click_fired carries NO_ACTION, dispatch skips it.
    assert!(
        !just_pressed(&w, TestAction::Jump)
            && !just_pressed(&w, TestAction::Fire)
            && !just_pressed(&w, TestAction::Menu),
        "no-OnClick node fires no action"
    );
    let _ = n;
}

// ───────────────────────── OnHover edge fires once ──────────────────────────

#[test]
fn hover_enter_fires_onhover_once_per_enter() {
    let mut w = InterWorld::new();
    let n = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_hover(1), None);

    // Off → no fire.
    w.set_cursor(500.0, 500.0);
    frame(&mut w);
    assert!(!just_pressed(&w, TestAction::Fire), "no hover, no fire");

    // Enter → fire once.
    w.set_cursor(50.0, 50.0);
    frame(&mut w);
    assert!(just_pressed(&w, TestAction::Fire), "None→Hovered fires OnHover");
    assert_eq!(w.interaction(n), Interaction::Hovered, "node is hovered");

    // Still hovering (held hover) → no re-fire (the edge buffer is empty).
    // begin_frame is not run in this harness, so to observe "no new edge" we
    // check that the hover_entered buffer produced nothing: re-run dispatch only.
    ui_dispatch_system::<TestAction>(&mut w.world);
    // The action bit is still set from the enter frame; assert the edge buffer is
    // empty by confirming a fresh ActionState would not re-receive it. We model
    // this by clearing the edge and re-running focus on a still frame.
    w.world.resource_mut::<ActionState<TestAction>>().begin_frame();
    frame(&mut w);
    assert!(
        !just_pressed(&w, TestAction::Fire),
        "a held hover (no None→Hovered edge) re-fires nothing"
    );
}

/// Helper kept for symmetry with the entity-based assertions.
#[allow(dead_code)]
fn entity_eq(a: Entity, b: Entity) -> bool {
    a == b
}
