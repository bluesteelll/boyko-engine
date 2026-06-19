//! Runtime rebinding + conflict detection (plan §9.4).
//!
//! A [`RebindSession`] enters "listen mode" for one `(action, slot)`; the
//! application feeds it raw events from its UI (the engine never owns rebind UI).
//! The first **gameplay-relevant** event — a key *press* or a mouse-button
//! *press* — is captured, written into the map's slot, and checked for a
//! conflict against the rest of the map. Motion, cursor, wheel, text, releases,
//! and OS auto-repeats are ignored so the session waits for a deliberate bind.
//!
//! This is a **cold** path (a UI interaction, never per-frame): the conflict scan
//! is O(active context bindings) and may allocate nothing on the hot path.

use crate::action::actionlike::Actionlike;
use crate::action::map::{BindSpec, InputMap, InputRef};
use crate::raw::event::RawInputEvent;
use crate::raw::keycode::{KeyCode, MouseButton};

/// The result of a rebind capture (plan §9.4 / §8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebindOutcome {
    /// The input was captured and written into the slot with no conflict.
    Bound,
    /// The input was captured and written, but it already binds another action
    /// in the same map; `existing` is that action's [`Actionlike::name`]. The
    /// application decides whether to keep the new binding, revert, or unbind the
    /// other action — the session does not auto-resolve.
    Conflict { existing: &'static str },
    /// The session was cancelled before any input was captured.
    Cancelled,
}

/// A single in-progress rebind for one `(action, slot)` (plan §9.4).
///
/// Create with [`RebindSession::begin`], then forward UI input through
/// [`RebindSession::feed`] until it returns `Some(outcome)`. A session captures
/// exactly one input; reuse means a fresh `begin`.
pub struct RebindSession<A: Actionlike> {
    action: A,
    slot: usize,
    /// `true` once an input has been captured or the session cancelled. After
    /// that, [`feed`](Self::feed) returns `None` (it does not replay the prior
    /// outcome); the caller should drop the session.
    done: bool,
}

impl<A: Actionlike> RebindSession<A> {
    /// Begins listening for the next gameplay-relevant input, to be written into
    /// `action`'s binding `slot` (0-based; `slot == current binding count` appends
    /// a new binding — see [`InputMap::set_binding`]).
    #[inline]
    pub fn begin(action: A, slot: u8) -> Self {
        Self {
            action,
            slot: slot as usize,
            done: false,
        }
    }

    /// The action this session is rebinding.
    #[inline]
    pub fn action(&self) -> A {
        self.action
    }

    /// `true` once an input has been captured or the session cancelled.
    #[inline]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Cancels the session without binding anything.
    ///
    /// Returns [`RebindOutcome::Cancelled`]. Idempotent: a second call still
    /// reports `Cancelled` (the session is already finished).
    #[inline]
    pub fn cancel(&mut self) -> RebindOutcome {
        self.done = true;
        RebindOutcome::Cancelled
    }

    /// Feeds one raw event. Returns `Some(outcome)` once a gameplay-relevant
    /// input is captured (and written into `map`), or `None` while still waiting
    /// (ignored events: motion, cursor, wheel, text, releases, auto-repeats).
    ///
    /// On capture the input is written into `map` at `(action, slot)` **before**
    /// the conflict scan, so the returned map already reflects the new binding
    /// regardless of conflict — the application decides whether to revert. If the
    /// slot write fails (an out-of-range slot), the session reports `Cancelled`.
    pub fn feed(&mut self, ev: &RawInputEvent, map: &mut InputMap<A>) -> Option<RebindOutcome> {
        if self.done {
            return None;
        }
        // Not a deliberate bind input (motion/cursor/wheel/text/release/repeat) ⇒
        // keep listening.
        let captured = capture(ev)?;

        self.done = true;

        if !map.set_binding(self.action, self.slot, captured) {
            return Some(RebindOutcome::Cancelled);
        }

        // Conflict scan: does the captured input already drive a *different*
        // action anywhere in the map? O(Σ bindings), cold. Mirrors the v1
        // single-context model; per-context scoping lands with contexts (V3).
        match find_conflict(map, self.action, &captured) {
            Some(existing) => Some(RebindOutcome::Conflict { existing }),
            None => Some(RebindOutcome::Bound),
        }
    }
}

/// Maps a raw event to the [`BindSpec`] it would bind, or `None` if the event is
/// not a deliberate bind input (motion / cursor / wheel / text / release /
/// auto-repeat).
fn capture(ev: &RawInputEvent) -> Option<BindSpec> {
    match ev {
        RawInputEvent::Key {
            code,
            state,
            repeat,
        } if state.is_pressed() && !*repeat => Some(BindSpec::Key(*code)),
        RawInputEvent::MouseButton { button, state } if state.is_pressed() => {
            Some(BindSpec::Mouse(*button))
        }
        _ => None,
    }
}

/// Scans `map` for an action other than `skip` whose bindings already include the
/// just-captured input, returning that action's name. The captured spec is a
/// bare `Key`/`Mouse`, so the scan compares against each binding's primary input
/// (a chord conflicts if the captured single key is one of its keys; a composite
/// conflicts if the captured input is one of its legs).
fn find_conflict<A: Actionlike>(
    map: &InputMap<A>,
    skip: A,
    captured: &BindSpec,
) -> Option<&'static str> {
    let skip_idx = skip.index();
    for i in 0..map.action_count() {
        if i == skip_idx {
            continue;
        }
        let Some(action) = A::from_index(i) else {
            continue;
        };
        for spec in map.bindings_at(i) {
            if binding_uses(spec, captured) {
                return Some(action.name());
            }
        }
    }
    None
}

/// Returns `true` if `existing` already uses the input `captured` names (a bare
/// `Key`/`Mouse`).
fn binding_uses(existing: &BindSpec, captured: &BindSpec) -> bool {
    match captured {
        BindSpec::Key(code) => binding_uses_key(existing, *code),
        BindSpec::Mouse(button) => binding_uses_mouse(existing, *button),
        // `capture` only ever produces bare Key/Mouse specs.
        _ => false,
    }
}

/// Whether `existing` references key `code` (as a bare key, a chord member, or a
/// composite leg).
fn binding_uses_key(existing: &BindSpec, code: KeyCode) -> bool {
    match existing {
        BindSpec::Key(k) => *k == code,
        BindSpec::Chord { keys, len } => keys[..*len as usize].contains(&code),
        BindSpec::Axis1 { neg, pos, .. } => {
            input_ref_is_key(*neg, code) || input_ref_is_key(*pos, code)
        }
        BindSpec::Axis2 {
            up,
            down,
            left,
            right,
            ..
        } => {
            input_ref_is_key(*up, code)
                || input_ref_is_key(*down, code)
                || input_ref_is_key(*left, code)
                || input_ref_is_key(*right, code)
        }
        BindSpec::Mouse(_) | BindSpec::Stick | BindSpec::None => false,
    }
}

/// Whether `existing` references mouse `button` (as a bare button or a composite
/// leg).
fn binding_uses_mouse(existing: &BindSpec, button: MouseButton) -> bool {
    match existing {
        BindSpec::Mouse(b) => *b == button,
        BindSpec::Axis1 { neg, pos, .. } => {
            input_ref_is_mouse(*neg, button) || input_ref_is_mouse(*pos, button)
        }
        BindSpec::Axis2 {
            up,
            down,
            left,
            right,
            ..
        } => {
            input_ref_is_mouse(*up, button)
                || input_ref_is_mouse(*down, button)
                || input_ref_is_mouse(*left, button)
                || input_ref_is_mouse(*right, button)
        }
        BindSpec::Key(_) | BindSpec::Chord { .. } | BindSpec::Stick | BindSpec::None => false,
    }
}

#[inline]
fn input_ref_is_key(r: InputRef, code: KeyCode) -> bool {
    matches!(r, InputRef::Key(k) if k == code)
}

#[inline]
fn input_ref_is_mouse(r: InputRef, button: MouseButton) -> bool {
    matches!(r, InputRef::Mouse(b) if b == button)
}
