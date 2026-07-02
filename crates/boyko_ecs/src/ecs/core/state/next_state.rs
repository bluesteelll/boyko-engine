//! The [`NextState<S>`] resource — a queued state-transition request (Phase 17 D2).

use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::resources::resource_type_registry::resource_id_for;
use crate::ecs::core::state::states::States;
use crate::ecs::identifiers::primitives::ResourceId;

/// A queued request to transition state type `S`, stored as a world-global
/// [`Resource`].
///
/// User systems write a request via `ResMut<NextState<S>>` (or
/// [`set`](NextState::set)); the transition pass drains it once per frame,
/// resetting it to [`Unchanged`](NextState::Unchanged). Kept separate from
/// [`State<S>`] so `in_state` readers (which touch only `State<S>`) never
/// share a slot with `NextState` writers — strictly more parallel.
///
/// [`Resource`]: crate::ecs::core::resources::resource::Resource
/// [`State<S>`]: crate::ecs::core::state::state::State
pub enum NextState<S: States> {
    /// No transition is queued.
    Unchanged,
    /// A transition to the contained value is queued; the pass applies it on
    /// the next `Schedule::run`.
    Pending(S),
}

impl<S: States> NextState<S> {
    /// Queues a transition to `value` (last-write-wins within a frame).
    ///
    /// Calling `set` twice in one frame keeps only the final value — the pass
    /// drains exactly one transition per frame.
    #[inline]
    pub fn set(&mut self, value: S) {
        *self = NextState::Pending(value);
    }

    /// Returns the queued value if a transition is pending, else `None`.
    #[inline]
    pub fn pending(&self) -> Option<&S> {
        match self {
            NextState::Unchanged => None,
            NextState::Pending(value) => Some(value),
        }
    }
}

impl<S: States> Default for NextState<S> {
    #[inline]
    fn default() -> Self {
        NextState::Unchanged
    }
}

impl<S: States> Resource for NextState<S> {
    // See `State<S>::resource_id` — the id is minted through the `TypeId`-keyed
    // registry, not a generic-body `static`, to avoid the rust#22991 collapse.
    #[inline]
    fn resource_id() -> ResourceId {
        resource_id_for::<NextState<S>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    enum TestState {
        A,
        B,
    }
    impl States for TestState {}

    /// `set(a); set(b)` keeps only the last value — last-write-wins within a
    /// frame (plan §9 unit: `next_state_set_overwrites`).
    #[test]
    fn next_state_set_overwrites() {
        let mut next: NextState<TestState> = NextState::Unchanged;
        next.set(TestState::A);
        next.set(TestState::B);
        assert!(
            next.pending() == Some(&TestState::B),
            "the second set must overwrite the first (last-write-wins)"
        );
    }

    /// `NextState::<S>::default()` is `Unchanged` — `pending()` is `None`
    /// (plan §9 unit: `next_state_default_unchanged`).
    #[test]
    fn next_state_default_unchanged() {
        let next: NextState<TestState> = NextState::default();
        assert!(
            matches!(next, NextState::Unchanged),
            "default must be the Unchanged variant"
        );
        assert!(
            next.pending().is_none(),
            "an Unchanged NextState has no pending value"
        );
    }

    /// A fresh `Unchanged` reports no pending value; after `set` it does.
    /// Guards the `pending()` accessor on both variants.
    #[test]
    fn next_state_pending_reflects_variant() {
        let mut next: NextState<TestState> = NextState::Unchanged;
        assert!(next.pending().is_none(), "Unchanged ⇒ pending None");
        next.set(TestState::A);
        assert!(
            next.pending() == Some(&TestState::A),
            "Pending(v) ⇒ pending Some(&v)"
        );
    }
}
