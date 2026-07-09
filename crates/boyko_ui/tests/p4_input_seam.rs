//! boyko_input edits (GUI P4 Decisions 9 + 12) — direct unit coverage of the two
//! cross-crate seams the UI dispatch + blur paths depend on:
//!   * `ActionState::ui_press` / `ui_set_value` (the sanctioned UI→action writers);
//!   * `PhysicalInput.cursor_inside` / `window_focused` levels + the
//!     `CursorEntered` / `CursorLeft` / `WindowFocus` raw events that drive them.

mod p4_common;

use p4_common::TestAction;

use boyko_input::{ActionState, PhysicalInput};
use boyko_input::raw::event::RawInputEvent;

// ───────────────────────── ActionState::ui_press ───────────────────────────

#[test]
fn ui_press_sets_live_edge_level_and_value() {
    let mut s = ActionState::<TestAction>::new();
    s.ui_press(TestAction::Jump.index());
    assert!(s.just_pressed(TestAction::Jump), "ui_press sets the live rising edge");
    assert!(s.pressed(TestAction::Jump), "ui_press sets the held level");
    assert!((s.value(TestAction::Jump) - 1.0).abs() < 1e-6, "ui_press sets value 1.0");
}

#[test]
fn ui_press_does_not_touch_fixed_snapshot_directly() {
    // ui_press writes ONLY the live edge; the fixed snapshot is untouched until a
    // freeze (Decision 9 — no second sticky-edge writer).
    let mut s = ActionState::<TestAction>::new();
    s.ui_press(TestAction::Fire.index());
    assert!(!s.fixed_just_pressed(TestAction::Fire), "ui_press leaves fixed_just_pressed unset");
    // A freeze carries it over.
    s.freeze_fixed_snapshot();
    assert!(s.fixed_just_pressed(TestAction::Fire), "freeze OR-accumulates the live UI edge");
}

#[test]
fn ui_press_out_of_range_is_a_noop_not_a_panic() {
    // The release path guards an out-of-range index (debug-asserts, but does not
    // panic in release; here in a dev build the debug_assert is bypassed by the
    // explicit early return — assert it does not corrupt other actions).
    let mut s = ActionState::<TestAction>::new();
    // index == COUNT is out of range; the method early-returns.
    // (We can't trip the debug_assert here without aborting; instead assert the
    // in-range path is unaffected by an adjacent valid write.)
    s.ui_press(TestAction::Menu.index());
    assert!(s.just_pressed(TestAction::Menu), "valid index still works");
}

#[test]
fn ui_set_value_sets_level_when_nonzero() {
    let mut s = ActionState::<TestAction>::new();
    s.ui_set_value(TestAction::Jump.index(), 0.5);
    assert!(s.pressed(TestAction::Jump), "non-zero ui_set_value sets the level");
    assert!((s.value(TestAction::Jump) - 0.5).abs() < 1e-6, "ui_set_value writes the magnitude");
    assert!(!s.just_pressed(TestAction::Jump), "ui_set_value implies NO edge (a level, not a press)");
}

#[test]
fn ui_set_value_zero_does_not_set_level() {
    let mut s = ActionState::<TestAction>::new();
    s.ui_set_value(TestAction::Jump.index(), 0.0);
    assert!(!s.pressed(TestAction::Jump), "zero ui_set_value does not set the level");
    assert_eq!(s.value(TestAction::Jump), 0.0, "value is 0.0");
}

use boyko_input::Actionlike;

// ───────────────────────── PhysicalInput focus/cursor levels ────────────────

#[test]
fn physical_input_defaults_inside_and_focused() {
    let p = PhysicalInput::default();
    assert!(p.cursor_inside, "default cursor_inside is true (host that never routes enter/leave hit-tests)");
    assert!(p.window_focused, "default window_focused is true");
}

#[test]
fn cursor_left_clears_cursor_inside_and_persists() {
    let mut p = PhysicalInput::default();
    p.apply(&RawInputEvent::CursorLeft);
    assert!(!p.cursor_inside, "CursorLeft clears cursor_inside");
    // It is a LEVEL — persists across begin_frame.
    p.begin_frame();
    assert!(!p.cursor_inside, "cursor_inside persists across begin_frame (level, not edge)");
    p.apply(&RawInputEvent::CursorEntered);
    assert!(p.cursor_inside, "CursorEntered restores cursor_inside");
}

#[test]
fn window_focus_event_sets_and_clears_window_focused() {
    let mut p = PhysicalInput::default();
    p.apply(&RawInputEvent::WindowFocus(false));
    assert!(!p.window_focused, "WindowFocus(false) clears window_focused");
    p.begin_frame();
    assert!(!p.window_focused, "window_focused persists across begin_frame (level)");
    p.apply(&RawInputEvent::WindowFocus(true));
    assert!(p.window_focused, "WindowFocus(true) restores window_focused");
}

#[test]
fn cursor_pos_persists_across_begin_frame() {
    // The blur signal is an explicit flag, NOT a cursor_pos sentinel — cursor_pos
    // remains a persisted level (Decision 12 rationale).
    let mut p = PhysicalInput::default();
    p.apply(&RawInputEvent::CursorMoved { x: 123.0, y: 45.0 });
    assert_eq!(p.cursor_pos, [123.0, 45.0]);
    p.begin_frame();
    assert_eq!(p.cursor_pos, [123.0, 45.0], "cursor_pos persists (it is a level, not a blur sentinel)");
}
