//! GATE 5 — SCHEDULE: a UI click within one frame produces EXACTLY ONE action
//! edge (no miss, no double-count) across the `freeze_fixed_snapshot` window,
//! and a 0-substep frame STILL delivers it (Decision 9/10).
//!
//! This models the engine's per-frame ordering around the UI dispatch:
//!   1. `clear_fixed_edges` at Main start — GATED on `steps_this_frame > 0`;
//!   2. device `begin_frame` (clears the live edge set);
//!   3. `ui_focus_system` → `ui_dispatch_system::<A>` (`ui_press` ORs the live
//!      rising edge);
//!   4. `ui_refreeze_fixed_snapshot::<A>` (`freeze_fixed_snapshot` OR-accumulates
//!      the live edge into `fixed_just_pressed`);
//!   5. the fixed batch reads `fixed_just_pressed`.
//!
//! The asserts count how many fixed batches observe the edge: exactly one.

mod p4_common;

use p4_common::{InterWorld, NodeOpts, TestAction};

use boyko_input::ActionState;
use boyko_ui::interaction::dispatch::{ui_dispatch_system, ui_refreeze_fixed_snapshot};

/// One Main frame of the modeled order. `substeps` is the number of fixed
/// substeps the fixed loop runs THIS frame (drives the gated clear AND the batch
/// consume). Returns the number of substeps that observed `fixed_just_pressed`
/// for `action`.
fn main_frame(
    w: &mut InterWorld,
    prev_substeps: usize,
    this_substeps: usize,
    action: TestAction,
    drive_input: impl FnOnce(&mut InterWorld),
) -> usize {
    // (1) clear_consumed_fixed_edges — gated on the PREVIOUS frame's fixed loop
    //     having run ≥ 1 substep (the engine clears edges consumed by that loop).
    if prev_substeps > 0 {
        w.world.resource_mut::<ActionState<TestAction>>().clear_fixed_edges();
    }
    // (2) device begin_frame — clears the live edge set (no device input here).
    w.world.resource_mut::<ActionState<TestAction>>().begin_frame();

    // (3) script the cursor/mouse, then focus → dispatch (ui_press ORs live edge).
    drive_input(w);
    w.focus();
    ui_dispatch_system::<TestAction>(&mut w.world);

    // (4) re-freeze: OR-accumulate the live UI edge into the fixed snapshot.
    ui_refreeze_fixed_snapshot::<TestAction>(&mut w.world);

    // (5) the fixed batch reads the frozen edge once per substep; count observers.
    let mut observed = 0usize;
    for _ in 0..this_substeps {
        if w.world.resource::<ActionState<TestAction>>().fixed_just_pressed(action) {
            observed += 1;
        }
    }
    observed
}

#[test]
fn schedule_single_substep_frame_delivers_exactly_once() {
    let mut w = InterWorld::new();
    let _btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(0), None);

    // Frame A (1 substep): press inside.
    let press = main_frame(&mut w, 0, 1, TestAction::Jump, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(true, false, true);
    });
    // Frame B (1 substep): release inside same — THIS is the click edge frame.
    let click = main_frame(&mut w, 1, 1, TestAction::Jump, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(false, true, false);
    });
    assert_eq!(press, 0, "press-down frame produces no fixed edge (release-up only)");
    assert_eq!(click, 1, "the click frame is observed by exactly one substep");

    // Frame C (1 substep): still frame — the edge was consumed + cleared, no
    // double-count.
    let after = main_frame(&mut w, 1, 1, TestAction::Jump, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(false, false, false);
    });
    assert_eq!(after, 0, "no double-count on the following frame");
}

#[test]
fn schedule_zero_substep_frame_keeps_edge_sticky_then_consumed_once() {
    // The click lands on a frame whose fixed loop runs 0 substeps. The edge must
    // be STICKY (not lost), then consumed by exactly ONE substep on the next
    // frame that actually steps (no-miss + no-double-count across the 0-step gap).
    let mut w = InterWorld::new();
    let _btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(1), None);

    // Frame A (press, 1 substep just to advance).
    let a = main_frame(&mut w, 0, 1, TestAction::Fire, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(true, false, true);
    });
    assert_eq!(a, 0, "press frame fires no edge");

    // Frame B: RELEASE (the click edge) on a 0-SUBSTEP frame. No batch consumes it.
    let b = main_frame(&mut w, 1, 0, TestAction::Fire, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(false, true, false);
    });
    assert_eq!(b, 0, "a 0-substep frame runs no batch, so 0 observers THIS frame");

    // Frame C: a stepping frame. prev_substeps == 0 → clear is SKIPPED, so the
    // sticky frozen edge survives and is consumed exactly once.
    let c = main_frame(&mut w, 0, 1, TestAction::Fire, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(false, false, false);
    });
    assert_eq!(c, 1, "the sticky edge is delivered to exactly one substep after the 0-step frame");

    // Frame D: now prev_substeps == 1 → clear runs, no double-count.
    let d = main_frame(&mut w, 1, 1, TestAction::Fire, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(false, false, false);
    });
    assert_eq!(d, 0, "no double-count after the consuming batch cleared the edge");
}

#[test]
fn schedule_multi_substep_batch_observes_edge_every_substep_one_frame() {
    // An N-substep batch sees the frame-stable frozen edge on every substep of
    // the consuming frame (the documented "fire once per press, idempotent per
    // frame" fixed contract) — and then it is cleared (no carry to the next).
    let mut w = InterWorld::new();
    let _btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(2), None);

    let _ = main_frame(&mut w, 0, 1, TestAction::Menu, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(true, false, true);
    });
    // Click frame consumed by a 4-substep batch: frame-stable → 4 observations,
    // all within ONE frame (this is the engine's "stable across substeps" contract,
    // not a double-count — a double-count would be a SECOND frame re-observing it).
    let observed = main_frame(&mut w, 1, 4, TestAction::Menu, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(false, true, false);
    });
    assert_eq!(observed, 4, "frame-stable frozen edge is visible to every substep of the batch");

    // Next frame: cleared → 0 (no cross-frame double-count).
    let next = main_frame(&mut w, 4, 4, TestAction::Menu, |w| {
        w.set_cursor(50.0, 50.0);
        w.set_mouse(false, false, false);
    });
    assert_eq!(next, 0, "the consumed edge does not carry into the next frame's batch");
}

#[test]
fn schedule_main_facing_edge_visible_same_frame() {
    // For non-fixed (Main) consumers, the UI live edge is visible the SAME frame
    // the click resolves (ui_press ORs it before any Main consumer reads). Driven
    // as a two-frame click (press frame, then release frame) — the release frame
    // is the one that lowers the action; the Main `just_pressed` must be set on
    // that frame. (A SAME-frame press+release is exercised by p4_click_action's
    // `click_same_frame_press_release_fires`, which is KNOWN-FAILING on
    // BUG-P4-CLICK-1 — avoided here so this test isolates the Main-visibility
    // contract, not the same-frame bug.)
    let mut w = InterWorld::new();
    let _btn = w.spawn_node_cfg(0.0, 0.0, 100.0, 100.0, NodeOpts::root().with_click(0), None);

    // Frame 1: press.
    w.world.resource_mut::<ActionState<TestAction>>().begin_frame();
    w.set_cursor(50.0, 50.0);
    w.set_mouse(true, false, true);
    w.focus();
    ui_dispatch_system::<TestAction>(&mut w.world);

    // Frame 2: release-inside-same → the click lowers; Main edge visible NOW.
    w.world.resource_mut::<ActionState<TestAction>>().begin_frame();
    w.set_mouse(false, true, false);
    w.focus();
    ui_dispatch_system::<TestAction>(&mut w.world);
    assert!(
        w.world.resource::<ActionState<TestAction>>().just_pressed(TestAction::Jump),
        "the Main-facing live just_pressed edge is visible the frame the click resolves"
    );
}
