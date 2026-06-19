//! I2 gate — `#[derive(Actionlike)]` correctness across enum shapes
//! (plan §14 I2): index density, from_index round-trip, kind/name, the
//! single-variant minimum, and explicit `#[actionlike(Button)]`.
//!
//! Compile-fail cases (non-enum, data-carrying variant, generic enum,
//! empty enum, unknown kind) are documented and demonstrated in
//! `tests/i2_compile_fail.rs` — `trybuild` is not an in-house dependency, so
//! those are verified by an out-of-band `rustc` invocation in the test report
//! rather than an in-tree harness.

use boyko_input::prelude::*;

/// A pure Button enum (no attributes) — every variant must default to Button.
#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum ButtonOnly {
    A,
    B,
    C,
}

/// A single-variant enum — the COUNT == 1 minimum boundary.
#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum Solo {
    Only,
}

/// Mixed kinds, with one variant carrying an *explicit* `#[actionlike(Button)]`
/// to confirm the explicit form resolves identically to the default.
#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum Mixed {
    #[actionlike(Button)]
    Explicit,
    #[actionlike(Axis1D)]
    Slider,
    #[actionlike(Axis2D)]
    Stick,
    Implicit, // defaults to Button
}

#[test]
fn button_only_count_and_kinds() {
    assert_eq!(ButtonOnly::COUNT, 3);
    for a in [ButtonOnly::A, ButtonOnly::B, ButtonOnly::C] {
        assert_eq!(a.kind(), ActionKind::Button, "no attribute defaults to Button");
    }
}

#[test]
fn button_only_index_is_dense_0_to_count() {
    let all = [ButtonOnly::A, ButtonOnly::B, ButtonOnly::C];
    for (i, a) in all.into_iter().enumerate() {
        assert_eq!(a.index(), i, "index is declaration order");
        assert_eq!(ButtonOnly::from_index(i), Some(a), "round-trip");
    }
    assert_eq!(ButtonOnly::from_index(3), None, "out-of-range index is None");
    assert_eq!(ButtonOnly::from_index(usize::MAX), None, "huge index is None");
}

#[test]
fn button_only_names_match_variant_idents() {
    assert_eq!(ButtonOnly::A.name(), "A");
    assert_eq!(ButtonOnly::B.name(), "B");
    assert_eq!(ButtonOnly::C.name(), "C");
}

#[test]
fn solo_single_variant_count_one() {
    assert_eq!(Solo::COUNT, 1);
    assert_eq!(Solo::Only.index(), 0);
    assert_eq!(Solo::from_index(0), Some(Solo::Only));
    assert_eq!(Solo::from_index(1), None);
    assert_eq!(Solo::Only.kind(), ActionKind::Button);
    assert_eq!(Solo::Only.name(), "Only");
}

#[test]
fn mixed_kinds_resolve_per_variant() {
    assert_eq!(Mixed::COUNT, 4);
    assert_eq!(Mixed::Explicit.kind(), ActionKind::Button, "explicit Button == default");
    assert_eq!(Mixed::Slider.kind(), ActionKind::Axis1D);
    assert_eq!(Mixed::Stick.kind(), ActionKind::Axis2D);
    assert_eq!(Mixed::Implicit.kind(), ActionKind::Button, "implicit default Button");
}

#[test]
fn mixed_index_round_trips_for_every_variant() {
    let all = [Mixed::Explicit, Mixed::Slider, Mixed::Stick, Mixed::Implicit];
    for (i, a) in all.into_iter().enumerate() {
        assert_eq!(a.index(), i);
        assert_eq!(Mixed::from_index(i), Some(a));
    }
    assert_eq!(Mixed::from_index(4), None);
}

/// The derive must drive an `InputMap`/`ActionState` sized by `COUNT` with no
/// extra trait bounds beyond `Copy + Eq + 'static` — a build-and-use smoke
/// confirming the generated impl is sufficient for the array-shaped storage.
#[test]
fn derived_enum_drives_action_state_sized_by_count() {
    let map = InputMap::builder()
        .bind(ButtonOnly::A, BindSpec::Key(KeyCode::KeyA))
        .build();
    let mut s = ActionState::<ButtonOnly>::new();
    let mut p = PhysicalInput::new();
    p.begin_frame();
    p.apply(&RawInputEvent::Key {
        code: KeyCode::KeyA,
        state: ButtonState::Pressed,
        repeat: false,
    });
    process_actions(&p, &map, &mut s);
    assert!(s.pressed(ButtonOnly::A));
    assert!(!s.pressed(ButtonOnly::B), "unbound sibling inactive");
    assert!(!s.pressed(ButtonOnly::C));
}
