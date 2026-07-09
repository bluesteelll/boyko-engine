//! The [`State<S>`] resource — the current value of state type `S` (Phase 17 D2).

use std::ops::Deref;

use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::resources::resource_type_registry::resource_id_for;
use crate::ecs::core::state::states::States;
use crate::ecs::identifiers::primitives::ResourceId;

/// The current value of state type `S`, stored as a world-global [`Resource`].
///
/// `#[repr(transparent)]` over `S`, so it is layout-identical to `S` and a
/// `Res<State<S>>` deref is a no-op pointer reuse. Written only by the
/// transition pass; read (shared) by the `in_state` condition.
///
/// [`Resource`]: crate::ecs::core::resources::resource::Resource
#[repr(transparent)]
pub struct State<S: States>(S);

impl<S: States> State<S> {
    /// Wraps `value` as the current state.
    #[inline]
    pub fn new(value: S) -> Self {
        Self(value)
    }

    /// Returns a shared reference to the wrapped state value.
    #[inline]
    pub fn get(&self) -> &S {
        &self.0
    }
}

impl<S: States> Deref for State<S> {
    type Target = S;

    #[inline]
    fn deref(&self) -> &S {
        &self.0
    }
}

impl<S: States> Resource for State<S> {
    // The `ResourceId` is minted through the `TypeId`-keyed process-global
    // registry, NOT a `static ID: OnceLock<_>` in this generic body: such a
    // static collapses across monomorphisations (rust#22991), aliasing every
    // `State<S>` onto one slot. See `resources::resource_type_registry`.
    #[inline]
    fn resource_id() -> ResourceId {
        resource_id_for::<State<S>>()
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

    /// `State::new(v).get()` returns the wrapped value (plan §9 unit:
    /// `state_get_returns_value`).
    #[test]
    fn state_get_returns_value() {
        let s = State::new(TestState::B);
        assert!(
            *s.get() == TestState::B,
            "get() must return the value passed to new()"
        );
    }

    /// `Deref` round-trips to the same value `get()` returns (plan §9 unit:
    /// the `Deref` half of `state_get_returns_value`).
    #[test]
    fn state_deref_round_trips() {
        let s = State::new(TestState::A);
        // Exercise the `Deref` impl explicitly via `*`.
        let via_deref: &TestState = &s;
        assert!(
            *via_deref == TestState::A && via_deref == s.get(),
            "Deref must yield the same value as get()"
        );
    }
}
