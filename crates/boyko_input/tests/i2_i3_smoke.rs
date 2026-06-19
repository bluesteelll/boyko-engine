//! Integration smoke tests for I2 (`#[derive(Actionlike)]`) + I3 (the
//! `InputMap`/`ActionState`/`process_actions` pipeline).
//!
//! These live in `tests/` (a separate crate) so the derive's emitted
//! `::boyko_input::…` paths resolve through the normal dependency edge — the
//! same arrangement `boyko_ecs` uses for its `#[derive(Component)]` smoke tests.

use boyko_input::prelude::*;

#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum PlayerAction {
    Jump,
    Fire,
    #[actionlike(Axis2D)]
    Move,
    #[actionlike(Axis1D)]
    Throttle,
}

#[test]
fn derive_count_and_index_round_trip() {
    assert_eq!(PlayerAction::COUNT, 4);
    for (i, expected) in [
        PlayerAction::Jump,
        PlayerAction::Fire,
        PlayerAction::Move,
        PlayerAction::Throttle,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(expected.index(), i);
        assert_eq!(PlayerAction::from_index(i), Some(expected));
    }
    assert_eq!(PlayerAction::from_index(4), None);
}

#[test]
fn derive_kind_and_name() {
    assert_eq!(PlayerAction::Jump.kind(), ActionKind::Button);
    assert_eq!(PlayerAction::Move.kind(), ActionKind::Axis2D);
    assert_eq!(PlayerAction::Throttle.kind(), ActionKind::Axis1D);
    assert_eq!(PlayerAction::Jump.name(), "Jump");
    assert_eq!(PlayerAction::Move.name(), "Move");
}

fn press(p: &mut PhysicalInput, code: KeyCode) {
    p.apply(&RawInputEvent::Key {
        code,
        state: ButtonState::Pressed,
        repeat: false,
    });
}

#[test]
fn button_aggregation_or() {
    let map = InputMap::builder()
        .bind(PlayerAction::Jump, BindSpec::Key(KeyCode::Space))
        .bind(PlayerAction::Jump, BindSpec::Mouse(MouseButton::Right))
        .build();
    let mut state = ActionState::<PlayerAction>::new();

    let mut phys = PhysicalInput::new();
    phys.begin_frame();
    press(&mut phys, KeyCode::Space);
    process_actions(&phys, &map, &mut state);

    assert!(state.pressed(PlayerAction::Jump));
    assert!(state.just_pressed(PlayerAction::Jump));
    assert_eq!(state.value(PlayerAction::Jump), 1.0);
}

#[test]
fn wasd_diagonal_is_normalized() {
    let map = InputMap::builder().wasd(PlayerAction::Move).build();
    let mut state = ActionState::<PlayerAction>::new();

    let mut phys = PhysicalInput::new();
    phys.begin_frame();
    press(&mut phys, KeyCode::KeyW); // up
    press(&mut phys, KeyCode::KeyD); // right
    process_actions(&phys, &map, &mut state);

    let v = state.axis2(PlayerAction::Move);
    let mag = (v[0] * v[0] + v[1] * v[1]).sqrt();
    assert!((mag - 1.0).abs() < 1e-4, "diagonal magnitude must be 1, got {mag}");
    assert!(v[0] > 0.0 && v[1] > 0.0, "moving up-right");
}

#[test]
fn axis1d_sum_and_clamp() {
    let map = InputMap::builder()
        .bind(
            PlayerAction::Throttle,
            BindSpec::Axis1 {
                neg: InputRef::Key(KeyCode::KeyS),
                pos: InputRef::Key(KeyCode::KeyW),
                dz: 0.0,
            },
        )
        .build();
    let mut state = ActionState::<PlayerAction>::new();

    let mut phys = PhysicalInput::new();
    phys.begin_frame();
    press(&mut phys, KeyCode::KeyW);
    process_actions(&phys, &map, &mut state);
    assert_eq!(state.value(PlayerAction::Throttle), 1.0);

    let mut phys = PhysicalInput::new();
    phys.begin_frame();
    press(&mut phys, KeyCode::KeyS);
    process_actions(&phys, &map, &mut state);
    assert_eq!(state.value(PlayerAction::Throttle), -1.0);
}

#[test]
fn prioritize_longest_suppresses_subset() {
    // Ctrl+S (quicksave) must suppress bare S (some other action).
    let map = InputMap::builder()
        .clash(ClashStrategy::PrioritizeLongest)
        .bind(PlayerAction::Fire, BindSpec::Key(KeyCode::KeyS))
        .bind(
            PlayerAction::Jump,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .build();
    let mut state = ActionState::<PlayerAction>::new();

    let mut phys = PhysicalInput::new();
    phys.begin_frame();
    press(&mut phys, KeyCode::ControlLeft);
    press(&mut phys, KeyCode::KeyS);
    process_actions(&phys, &map, &mut state);

    assert!(state.pressed(PlayerAction::Jump), "the chord fires");
    assert!(
        !state.pressed(PlayerAction::Fire),
        "the bare-S subset action is suppressed"
    );
}

#[test]
fn same_frame_tap_just_pressed_via_action() {
    let map = InputMap::builder()
        .bind(PlayerAction::Fire, BindSpec::Key(KeyCode::KeyF))
        .build();
    let mut state = ActionState::<PlayerAction>::new();

    let mut phys = PhysicalInput::new();
    phys.begin_frame();
    phys.apply(&RawInputEvent::Key {
        code: KeyCode::KeyF,
        state: ButtonState::Pressed,
        repeat: false,
    });
    phys.apply(&RawInputEvent::Key {
        code: KeyCode::KeyF,
        state: ButtonState::Released,
        repeat: false,
    });
    process_actions(&phys, &map, &mut state);

    assert!(state.just_pressed(PlayerAction::Fire), "tap rising survives (W4)");
    assert!(state.just_released(PlayerAction::Fire), "tap falling survives (W4)");
}

#[test]
fn consume_masks_subsequent_reads() {
    let map = InputMap::builder()
        .bind(PlayerAction::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    let mut state = ActionState::<PlayerAction>::new();

    let mut phys = PhysicalInput::new();
    phys.begin_frame();
    press(&mut phys, KeyCode::Space);
    process_actions(&phys, &map, &mut state);

    assert!(state.just_pressed(PlayerAction::Jump));
    state.consume(PlayerAction::Jump);
    assert!(!state.just_pressed(PlayerAction::Jump), "consumed edge is masked");
    assert!(!state.pressed(PlayerAction::Jump), "consumed level is masked");
}
