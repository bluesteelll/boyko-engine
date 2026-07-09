//! GATE 3 — a Button's `Interaction` edge fires its `OnClick` through the P4
//! dispatch.
//!
//! A Button widget is the interactive-panel component set PLUS the `Button` marker.
//! The marker carries no behavior of its own — interaction + dispatch are the
//! EXISTING P4 systems (`ui_focus_system` -> `ui_dispatch_system`), which a Button
//! composes verbatim. This reuses the P4 focus/dispatch harness (`p4_common`) and
//! adds the `Button` marker to the spawned node, then asserts the same press-in /
//! release-in-same click edge fires the action — proving the marker does not break
//! the dispatch path and a Button is a first-class clickable.

mod p4_common;

use p4_common::{InterWorld, NodeOpts, TestAction};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use boyko_input::ActionState;
use boyko_ui::components::Button;
use boyko_ui::interaction::components::Interaction;
use boyko_ui::interaction::dispatch::ui_dispatch_system;

/// Runs focus then dispatch for one frame.
fn frame(w: &mut InterWorld) {
    w.focus();
    ui_dispatch_system::<TestAction>(&mut w.world);
}

fn just_pressed(w: &InterWorld, action: TestAction) -> bool {
    w.world.resource::<ActionState<TestAction>>().just_pressed(action)
}

/// Adds the `Button` marker to an already-spawned interactive node (so the node is
/// the full Button component set: the P4 interactive set + the marker).
fn mark_button(w: &mut InterWorld, e: Entity) {
    w.world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(Button);
    });
    assert!(w.world.has_component(e, Button::component_id()), "node carries the Button marker");
}

#[test]
fn button_press_release_fires_onclick() {
    let mut w = InterWorld::new();
    // A clickable interactive root with OnClick action index 0 (Jump).
    let btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(0), None);
    mark_button(&mut w, btn);

    // Frame 1: press inside.
    w.set_cursor(50.0, 50.0);
    w.set_mouse(true, false, true);
    frame(&mut w);
    assert_eq!(w.interaction(btn), Interaction::Pressed, "Button is Pressed during the press");
    assert_eq!(w.pending_click(), Some((btn, 0)), "press stamps pending_click");
    assert!(!just_pressed(&w, TestAction::Jump), "no action on press-down (release-up only)");

    // Frame 2: release inside the same node -> the OnClick action fires.
    w.set_mouse(false, true, false);
    frame(&mut w);
    assert_eq!(w.click_fired(), Some((btn, 0)), "release-inside-same stamps click_fired");
    assert!(just_pressed(&w, TestAction::Jump), "Button release-up fires the OnClick action");
}

#[test]
fn button_release_outside_does_not_fire() {
    // The marker must not change the cancel semantics: drag-off then release fires
    // nothing.
    let mut w = InterWorld::new();
    let btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(1), None);
    mark_button(&mut w, btn);

    w.set_cursor(50.0, 50.0);
    w.set_mouse(true, false, true);
    frame(&mut w);
    assert_eq!(w.pending_click(), Some((btn, 1)), "press stamped");

    // Drag off and release outside.
    w.set_cursor(500.0, 500.0);
    w.set_mouse(false, true, false);
    frame(&mut w);
    assert_eq!(w.click_fired(), None, "release outside the Button does not fire");
    assert!(!just_pressed(&w, TestAction::Fire), "Button drag-off then release fires nothing");
}

#[test]
fn two_buttons_only_the_clicked_one_fires() {
    let mut w = InterWorld::new();
    let a = w.spawn_node_cfg(0.0, 0.0, 50.0, 50.0, NodeOpts::root().with_click(0), None);
    let b = w.spawn_node_cfg(200.0, 200.0, 50.0, 50.0, NodeOpts::root().with_click(1), None);
    mark_button(&mut w, a);
    mark_button(&mut w, b);

    // Click B (action 1 = Fire).
    w.set_cursor(225.0, 225.0);
    w.set_mouse(true, false, true);
    frame(&mut w);
    assert_eq!(w.pending_click(), Some((b, 1)), "press on B");
    w.set_mouse(false, true, false);
    frame(&mut w);
    assert_eq!(w.click_fired(), Some((b, 1)), "B's click fired");
    assert!(just_pressed(&w, TestAction::Fire), "B's action fired");
    assert!(!just_pressed(&w, TestAction::Jump), "A's action did NOT fire");
    let _ = a;
}
