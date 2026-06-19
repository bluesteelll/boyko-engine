//! I3 gate — `process_actions` aggregation, clash resolution, deadzone/clamp,
//! WASD normalization, and `consume` masking (plan §14 I3, §12).
//!
//! These exercise the full `PhysicalInput → InputMap → ActionState` pipeline
//! through the public API (the same path a gameplay system uses).

use boyko_input::prelude::*;

#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Jump,           // Button
    Fire,           // Button
    Crouch,         // Button
    #[actionlike(Axis2D)]
    Move,
    #[actionlike(Axis1D)]
    Throttle,
    #[actionlike(Axis1D)]
    Strafe,
    Unbound,        // Button with no binding
    QuickSave,      // Button (chord target)
}

fn down(p: &mut PhysicalInput, code: KeyCode) {
    p.apply(&RawInputEvent::Key {
        code,
        state: ButtonState::Pressed,
        repeat: false,
    });
}

fn up(p: &mut PhysicalInput, code: KeyCode) {
    p.apply(&RawInputEvent::Key {
        code,
        state: ButtonState::Released,
        repeat: false,
    });
}

fn mouse_down(p: &mut PhysicalInput, b: MouseButton) {
    p.apply(&RawInputEvent::MouseButton {
        button: b,
        state: ButtonState::Pressed,
    });
}

// --- Button aggregation: OR over bindings, max over values ---

#[test]
fn button_or_across_key_and_mouse() {
    let map = InputMap::builder()
        .bind(Act::Fire, BindSpec::Key(KeyCode::ControlLeft))
        .bind(Act::Fire, BindSpec::Mouse(MouseButton::Left))
        .build();
    let mut s = ActionState::<Act>::new();

    // Only the mouse binding is active — OR still fires the action.
    let mut p = PhysicalInput::new();
    p.begin_frame();
    mouse_down(&mut p, MouseButton::Left);
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(Act::Fire), "OR: either binding active fires the action");
    assert_eq!(s.value(Act::Fire), 1.0, "button value is the max (1.0)");
}

#[test]
fn button_not_pressed_when_no_binding_active() {
    let map = InputMap::builder()
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    // Nothing pressed.
    process_actions(&p, &map, &mut s);
    assert!(!s.pressed(Act::Jump));
    assert_eq!(s.value(Act::Jump), 0.0);
    assert!(!s.just_pressed(Act::Jump));
}

#[test]
fn unbound_action_is_always_inactive() {
    // An action declared in the enum but never bound must read as fully inactive.
    let map = InputMap::builder()
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::Space);
    process_actions(&p, &map, &mut s);
    assert!(!s.pressed(Act::Unbound), "no-binding action never fires");
    assert_eq!(s.axis2(Act::Unbound), [0.0, 0.0]);
}

#[test]
fn empty_map_yields_no_active_actions() {
    // A fully empty builder must process without panic and leave all inactive.
    let map = InputMap::builder().build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::Space);
    down(&mut p, KeyCode::KeyW);
    process_actions(&p, &map, &mut s);
    assert!(!s.pressed(Act::Jump));
    assert!(!s.pressed(Act::Fire));
    assert_eq!(s.axis2(Act::Move), [0.0, 0.0]);
}

// --- Button edge semantics through the action layer ---

#[test]
fn button_rising_edge_only_on_first_frame() {
    let map = InputMap::builder()
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    let mut s = ActionState::<Act>::new();

    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::Space);
    process_actions(&p, &map, &mut s);
    assert!(s.just_pressed(Act::Jump), "rising on first frame");

    // Frame 2: still held, but the edge cleared (begin_frame on PhysicalInput).
    p.begin_frame();
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(Act::Jump), "still held");
    assert!(!s.just_pressed(Act::Jump), "no second rising while held");
}

#[test]
fn button_falling_edge_on_release() {
    let map = InputMap::builder()
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    let mut s = ActionState::<Act>::new();

    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::Space);
    process_actions(&p, &map, &mut s);

    p.begin_frame();
    up(&mut p, KeyCode::Space);
    process_actions(&p, &map, &mut s);
    assert!(s.just_released(Act::Jump), "falling on release");
    assert!(!s.pressed(Act::Jump), "no longer held");
}

// --- Axis1D: sum, deadzone, clamp ---

#[test]
fn axis1d_opposing_keys_cancel_to_zero() {
    let map = InputMap::builder()
        .bind(
            Act::Throttle,
            BindSpec::Axis1 {
                neg: InputRef::Key(KeyCode::KeyS),
                pos: InputRef::Key(KeyCode::KeyW),
                dz: 0.0,
            },
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::KeyW); // +1
    down(&mut p, KeyCode::KeyS); // -1
    process_actions(&p, &map, &mut s);
    assert_eq!(s.value(Act::Throttle), 0.0, "opposing legs sum to zero");
}

#[test]
fn axis1d_deadzone_suppresses_small_values() {
    // Two Axis1 bindings summing to a small magnitude under the deadzone.
    // The first binding's deadzone is the one applied (axis2_params/binding_deadzone_1d
    // reads the first Axis1 binding). Here the single binding's value (+1) is well
    // above any deadzone < 1; to exercise the suppression we use a deadzone of 1.0
    // so a single +1 leg sits exactly at the boundary (suppressed: v.abs() <= dz).
    let map = InputMap::builder()
        .bind(
            Act::Strafe,
            BindSpec::Axis1 {
                neg: InputRef::Key(KeyCode::KeyA),
                pos: InputRef::Key(KeyCode::KeyD),
                dz: 1.0,
            },
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::KeyD); // +1, but |1| <= dz(1.0) ⇒ deadzoned to 0
    process_actions(&p, &map, &mut s);
    assert_eq!(s.value(Act::Strafe), 0.0, "value at the deadzone boundary is suppressed");
}

#[test]
fn axis1d_clamps_to_unit() {
    // A single +1 leg with no deadzone yields exactly +1 (already in range).
    let map = InputMap::builder()
        .bind(
            Act::Throttle,
            BindSpec::Axis1 {
                neg: InputRef::Key(KeyCode::KeyS),
                pos: InputRef::Key(KeyCode::KeyW),
                dz: 0.0,
            },
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::KeyW);
    process_actions(&p, &map, &mut s);
    assert_eq!(s.value(Act::Throttle), 1.0);
    assert!(s.pressed(Act::Throttle), "nonzero axis marks the action pressed");
}

// --- Axis2D: WASD diagonal normalization vs raw ---

#[test]
fn wasd_cardinal_is_unit_length() {
    let map = InputMap::builder().wasd(Act::Move).build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::KeyW); // up only
    process_actions(&p, &map, &mut s);
    let v = s.axis2(Act::Move);
    assert_eq!(v, [0.0, 1.0], "cardinal up is exactly (0,1)");
}

#[test]
fn wasd_diagonal_is_normalized_to_unit() {
    let map = InputMap::builder().wasd(Act::Move).build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::KeyW);
    down(&mut p, KeyCode::KeyD);
    process_actions(&p, &map, &mut s);
    let v = s.axis2(Act::Move);
    let mag = (v[0] * v[0] + v[1] * v[1]).sqrt();
    assert!((mag - 1.0).abs() < 1e-4, "diagonal magnitude must be 1, got {mag}");
    assert!(v[0] > 0.0 && v[1] > 0.0, "up-right quadrant");
}

#[test]
fn axis2_raw_mode_keeps_long_diagonal() {
    // DigitalRaw must NOT normalize — the diagonal stays sqrt(2)-ish... but the
    // finalizer clamps each component to [-1,1], so the raw diagonal is (1,1).
    let map = InputMap::builder()
        .bind(
            Act::Move,
            BindSpec::Axis2 {
                up: InputRef::Key(KeyCode::KeyW),
                down: InputRef::Key(KeyCode::KeyS),
                left: InputRef::Key(KeyCode::KeyA),
                right: InputRef::Key(KeyCode::KeyD),
                dz: 0.0,
                mode: AxisMode::DigitalRaw,
            },
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::KeyW);
    down(&mut p, KeyCode::KeyD);
    process_actions(&p, &map, &mut s);
    let v = s.axis2(Act::Move);
    assert_eq!(v, [1.0, 1.0], "raw diagonal is (1,1), each leg clamped not normalized");
    let mag = (v[0] * v[0] + v[1] * v[1]).sqrt();
    assert!(mag > 1.0, "raw diagonal magnitude exceeds 1 (no normalization)");
}

#[test]
fn axis2_opposing_keys_cancel() {
    let map = InputMap::builder().wasd(Act::Move).build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::KeyW);
    down(&mut p, KeyCode::KeyS); // up + down cancel
    down(&mut p, KeyCode::KeyA);
    down(&mut p, KeyCode::KeyD); // left + right cancel
    process_actions(&p, &map, &mut s);
    assert_eq!(s.axis2(Act::Move), [0.0, 0.0], "all four legs cancel to zero");
}

#[test]
fn axis2_deadzone_suppresses_below_threshold() {
    // A deadzone of 1.5 suppresses a cardinal (magnitude 1.0 <= 1.5).
    let map = InputMap::builder()
        .bind(
            Act::Move,
            BindSpec::Axis2 {
                up: InputRef::Key(KeyCode::KeyW),
                down: InputRef::Key(KeyCode::KeyS),
                left: InputRef::Key(KeyCode::KeyA),
                right: InputRef::Key(KeyCode::KeyD),
                dz: 1.5,
                mode: AxisMode::DigitalNormalized,
            },
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::KeyW);
    process_actions(&p, &map, &mut s);
    assert_eq!(s.axis2(Act::Move), [0.0, 0.0], "below-deadzone vector is zeroed");
}

// --- Clash resolution (PrioritizeLongest) ---

#[test]
fn clash_chord_suppresses_bare_subset_key() {
    let map = InputMap::builder()
        .clash(ClashStrategy::PrioritizeLongest)
        .bind(Act::Fire, BindSpec::Key(KeyCode::KeyS))
        .bind(
            Act::QuickSave,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::ControlLeft);
    down(&mut p, KeyCode::KeyS);
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(Act::QuickSave), "Ctrl+S superset fires");
    assert!(!s.pressed(Act::Fire), "bare S subset is suppressed");
}

#[test]
fn clash_bare_key_alone_is_not_suppressed() {
    // Without the chord active, the bare key must fire normally.
    let map = InputMap::builder()
        .clash(ClashStrategy::PrioritizeLongest)
        .bind(Act::Fire, BindSpec::Key(KeyCode::KeyS))
        .bind(
            Act::QuickSave,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::KeyS); // Ctrl NOT held → chord inactive
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(Act::Fire), "bare S fires when no superset is active");
    assert!(!s.pressed(Act::QuickSave), "chord needs Ctrl too");
}

#[test]
fn clash_allow_all_does_not_suppress() {
    let map = InputMap::builder()
        .clash(ClashStrategy::AllowAll)
        .bind(Act::Fire, BindSpec::Key(KeyCode::KeyS))
        .bind(
            Act::QuickSave,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::ControlLeft);
    down(&mut p, KeyCode::KeyS);
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(Act::QuickSave), "chord fires");
    assert!(s.pressed(Act::Fire), "AllowAll: bare S also fires (no suppression)");
}

#[test]
fn clash_three_key_chord_suppresses_two_key_subset() {
    // Ctrl+Shift+S (superset) suppresses Ctrl+S (strict subset).
    let map = InputMap::builder()
        .clash(ClashStrategy::PrioritizeLongest)
        .bind(
            Act::QuickSave,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .bind(
            Act::Crouch,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::ShiftLeft, KeyCode::KeyS]),
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::ControlLeft);
    down(&mut p, KeyCode::ShiftLeft);
    down(&mut p, KeyCode::KeyS);
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(Act::Crouch), "3-key superset fires");
    assert!(!s.pressed(Act::QuickSave), "2-key strict subset suppressed");
}

#[test]
fn clash_disjoint_chords_do_not_suppress_each_other() {
    // Ctrl+A and Ctrl+S share Ctrl but neither is a subset of the other.
    let map = InputMap::builder()
        .clash(ClashStrategy::PrioritizeLongest)
        .bind(
            Act::Jump,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyA]),
        )
        .bind(
            Act::Fire,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::ControlLeft);
    down(&mut p, KeyCode::KeyA);
    down(&mut p, KeyCode::KeyS);
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(Act::Jump), "Ctrl+A fires (not a subset of Ctrl+S)");
    assert!(s.pressed(Act::Fire), "Ctrl+S fires (not a subset of Ctrl+A)");
}

// --- consume() masking ---

#[test]
fn consume_masks_pressed_and_edges() {
    let map = InputMap::builder()
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::Space);
    process_actions(&p, &map, &mut s);

    assert!(s.just_pressed(Act::Jump));
    assert!(s.pressed(Act::Jump));
    s.consume(Act::Jump);
    assert!(!s.just_pressed(Act::Jump), "consumed edge masked");
    assert!(!s.pressed(Act::Jump), "consumed level masked");
    assert_eq!(s.value(Act::Jump), 0.0, "consumed value masked to 0");
}

#[test]
fn consume_does_not_affect_other_actions() {
    let map = InputMap::builder()
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .bind(Act::Fire, BindSpec::Key(KeyCode::ControlLeft))
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::Space);
    down(&mut p, KeyCode::ControlLeft);
    process_actions(&p, &map, &mut s);

    s.consume(Act::Jump);
    assert!(!s.pressed(Act::Jump), "consumed action masked");
    assert!(s.pressed(Act::Fire), "a sibling action is unaffected by consume");
}

#[test]
fn consume_is_reset_by_next_process() {
    // process_actions calls begin_frame, which clears the consumed set — a new
    // frame must see the action live again.
    let map = InputMap::builder()
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::Space);
    process_actions(&p, &map, &mut s);
    s.consume(Act::Jump);
    assert!(!s.pressed(Act::Jump));

    // Next frame: still physically held, consume cleared.
    p.begin_frame();
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(Act::Jump), "consume does not persist across frames");
}

// --- chord same-frame completion edge (W4) ---

#[test]
fn chord_rising_edge_when_last_key_completes_it() {
    let map = InputMap::builder()
        .bind(
            Act::QuickSave,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .build();
    let mut s = ActionState::<Act>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    down(&mut p, KeyCode::ControlLeft);
    down(&mut p, KeyCode::KeyS); // completes the chord this frame
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(Act::QuickSave), "chord held");
    assert!(s.just_pressed(Act::QuickSave), "chord rising edge on completion (W4)");
}
