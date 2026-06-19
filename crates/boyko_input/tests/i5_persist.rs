//! I5 gate (focused) — `.keys` persistence + rebind (plan §9 / §14 I5).
//!
//! These cover the critical, easily-broken rules: the canonical round-trip
//! (`parse ∘ serialize == identity` on canonical output), the paren-depth comma
//! split, the unquoted-`#` comment rule, `raw(N)`/`MouseOther(N)` lossless
//! round-trip, override-delta + `= none`, versioning, and the rebind
//! conflict/cancel state machine.
//!
//! The comprehensive property test (`parse ∘ serialize == identity` over random
//! canonical maps) and the full rebind matrix are the tester's job; this is the
//! minimal in-tree confidence net.

use boyko_input::action::map::InputMapBuilder;
use boyko_input::prelude::*;

#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Jump,
    Fire,
    #[actionlike(Axis2D)]
    Move,
    Quicksave,
    Exotic,
    Secondary,
}

fn default_map() -> InputMap<Action> {
    InputMap::<Action>::builder()
        .bind(Action::Jump, BindSpec::Key(KeyCode::Space))
        .bind(Action::Fire, BindSpec::Mouse(MouseButton::Left))
        .wasd(Action::Move)
        .bind(
            Action::Quicksave,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .bind(Action::Exotic, BindSpec::Key(KeyCode::Unidentified(0x56)))
        .bind(Action::Secondary, BindSpec::Mouse(MouseButton::Other(9)))
        .build()
}

#[test]
fn canonical_round_trip_is_byte_identical() {
    let map = default_map();
    let first = keys_to_string(&map);

    // Re-parse the canonical text onto a default-seeded builder, rebuild, and
    // re-serialize. The two texts must be byte-identical (the round-trip fixpoint).
    let mut builder = InputMapBuilder::<Action>::from_map(&map);
    let report = load_keys(&first, &mut builder);
    assert!(report.is_clean(), "unexpected parse errors: {:?}", report.errors);

    let reparsed = builder.build();
    let second = keys_to_string(&reparsed);
    assert_eq!(first, second, "canonical serialization is not a parser fixpoint");
}

#[test]
fn raw_and_other_round_trip_losslessly() {
    let map = default_map();
    let text = keys_to_string(&map);
    assert!(text.contains("raw(0x56)"), "exotic key must serialize as raw(0xNN): {text}");
    assert!(
        text.contains("MouseOther(9)"),
        "extra mouse button must serialize as MouseOther(N): {text}"
    );

    // And re-parse back to the same bindings.
    let mut builder = InputMapBuilder::<Action>::from_map(&default_map());
    load_keys(&text, &mut builder);
    let reparsed = builder.build();
    assert_eq!(
        reparsed.bindings_for(Action::Exotic),
        &[BindSpec::Key(KeyCode::Unidentified(0x56))]
    );
    assert_eq!(
        reparsed.bindings_for(Action::Secondary),
        &[BindSpec::Mouse(MouseButton::Other(9))]
    );
}

#[test]
fn override_delta_keeps_absent_actions_and_replaces_present() {
    // Only `Jump` is mentioned; every other action keeps its default.
    let src = "version = 1\n[gameplay clash=longest]\nJump = Enter\n";
    let mut builder = InputMapBuilder::<Action>::from_map(&default_map());
    let report = load_keys(src, &mut builder);
    assert!(report.is_clean(), "{:?}", report.errors);
    let map = builder.build();

    assert_eq!(map.bindings_for(Action::Jump), &[BindSpec::Key(KeyCode::Enter)]);
    // Fire kept its default mouse binding (absent from the file).
    assert_eq!(
        map.bindings_for(Action::Fire),
        &[BindSpec::Mouse(MouseButton::Left)]
    );
}

#[test]
fn explicit_none_unbinds() {
    let src = "Jump = none\n";
    let mut builder = InputMapBuilder::<Action>::from_map(&default_map());
    load_keys(src, &mut builder);
    let map = builder.build();
    // `= none` clears the slot to a single explicit-unbind marker; it serializes
    // back as `none`.
    let text = keys_to_string(&map);
    assert!(
        text.lines().any(|l| l.starts_with("Jump = none")),
        "explicit unbind must serialize as `none`: {text}"
    );
}

#[test]
fn unknown_token_is_recoverable() {
    let src = "Jump = NotARealKey\nFire = Mouse2\n";
    let mut builder = InputMapBuilder::<Action>::from_map(&default_map());
    let report = load_keys(src, &mut builder);
    // The bad spec is recorded, parsing continued, and the good line applied.
    assert!(!report.is_clean());
    assert_eq!(report.errors.len(), 1);
    let map = builder.build();
    assert_eq!(
        map.bindings_for(Action::Fire),
        &[BindSpec::Mouse(MouseButton::Right)]
    );
}

#[test]
fn higher_version_warns_never_fails() {
    let src = "version = 999\nJump = Space\n";
    let mut builder = InputMapBuilder::<Action>::from_map(&default_map());
    let report = load_keys(src, &mut builder);
    assert_eq!(report.version, 999);
    assert!(report.is_clean(), "higher version must not be a hard error");
    assert!(!report.warnings.is_empty(), "higher version must warn");
}

#[test]
fn paren_depth_split_keeps_axis2_intact() {
    let src = "Move = axis2(up=W, down=S, left=A, right=D, dz=0.15, mode=radial)\n";
    let mut builder = InputMapBuilder::<Action>::from_map(&default_map());
    let report = load_keys(src, &mut builder);
    assert!(report.is_clean(), "{:?}", report.errors);
    let map = builder.build();
    match map.bindings_for(Action::Move) {
        [BindSpec::Axis2 { dz, mode, .. }] => {
            assert!((dz - 0.15).abs() < 1e-6);
            assert_eq!(*mode, AxisMode::DigitalNormalized);
        }
        other => panic!("expected one axis2 binding, got {other:?}"),
    }
}

#[test]
fn comment_inside_quotes_is_not_a_comment() {
    // The `#` is inside quotes ⇒ literal; the line still parses (the quoted token
    // is an unknown key, recorded as a recoverable error, proving the `#` was not
    // treated as a comment that would have hidden the bad token).
    let src = "Jump = \"a#b\"\n";
    let mut builder = InputMapBuilder::<Action>::from_map(&default_map());
    let report = load_keys(src, &mut builder);
    assert_eq!(report.errors.len(), 1, "the quoted token survived comment stripping");
}

#[test]
fn rebind_binds_then_conflicts() {
    let mut map = default_map();

    // Rebind Jump (slot 0) to a fresh key with no conflict.
    let mut session = RebindSession::begin(Action::Jump, 0);
    let ev = RawInputEvent::Key {
        code: KeyCode::KeyJ,
        state: ButtonState::Pressed,
        repeat: false,
    };
    assert_eq!(session.feed(&ev, &mut map), Some(RebindOutcome::Bound));
    assert_eq!(map.bindings_for(Action::Jump), &[BindSpec::Key(KeyCode::KeyJ)]);

    // Now rebind Fire onto KeyJ — conflicts with Jump.
    let mut session = RebindSession::begin(Action::Fire, 0);
    let ev = RawInputEvent::Key {
        code: KeyCode::KeyJ,
        state: ButtonState::Pressed,
        repeat: false,
    };
    match session.feed(&ev, &mut map) {
        Some(RebindOutcome::Conflict { existing }) => assert_eq!(existing, "Jump"),
        other => panic!("expected conflict with Jump, got {other:?}"),
    }
}

#[test]
fn rebind_ignores_noise_until_press() {
    let mut map = default_map();
    let mut session = RebindSession::begin(Action::Jump, 0);

    // Motion / cursor / release / repeat are ignored — the session keeps waiting.
    assert_eq!(session.feed(&RawInputEvent::MouseMotion { dx: 1.0, dy: 2.0 }, &mut map), None);
    assert_eq!(session.feed(&RawInputEvent::CursorMoved { x: 1.0, y: 2.0 }, &mut map), None);
    assert_eq!(
        session.feed(
            &RawInputEvent::Key {
                code: KeyCode::KeyK,
                state: ButtonState::Released,
                repeat: false
            },
            &mut map
        ),
        None
    );
    assert_eq!(
        session.feed(
            &RawInputEvent::Key {
                code: KeyCode::KeyK,
                state: ButtonState::Pressed,
                repeat: true
            },
            &mut map
        ),
        None
    );
    assert!(!session.is_done());

    // A real press captures.
    let ev = RawInputEvent::Key {
        code: KeyCode::KeyK,
        state: ButtonState::Pressed,
        repeat: false,
    };
    assert!(session.feed(&ev, &mut map).is_some());
    assert!(session.is_done());
}

#[test]
fn rebind_cancel_binds_nothing() {
    let map = default_map();
    let before = map.bindings_for(Action::Jump).to_vec();
    let mut session = RebindSession::begin(Action::Jump, 0);
    assert_eq!(session.cancel(), RebindOutcome::Cancelled);
    assert!(session.is_done());
    assert_eq!(map.bindings_for(Action::Jump), before.as_slice());
}
