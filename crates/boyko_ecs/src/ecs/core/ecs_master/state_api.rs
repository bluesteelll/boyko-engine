//! State facade on [`EcsMaster`] (mechanical split; Phase 17).
//!
//! `insert_state` / `init_state` / `state` / `set_next_state`. Extracted
//! verbatim from `ecs_master.rs`.

use crate::ecs::core::state::states::States;
use crate::ecs::core::state::transition_record::StateTransitionRecord;
use crate::ecs::core::state::{NextState, State};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    // ── Phase 17: State facade ──────────────────────────────────────────────

    /// Inserts the three resources that back state type `S` — `State<S> =
    /// initial`, `NextState<S> = Unchanged`, and the per-`S` transition record
    /// — into the world (Phase 17 D7).
    ///
    /// This mutates the world only; it does **not** register the schedule-side
    /// `StateEntry` that drives the transition pass. Use
    /// `ScheduleBuilder::insert_state::<S>(initial)` to both insert the
    /// resources and have the schedule fire the initial `OnEnter` and drain
    /// transitions each frame.
    ///
    /// # Initial-transition interaction
    /// Calling `set_next_state::<S>(..)` (or otherwise queuing a `Pending`)
    /// **before the first `Schedule::run`** suppresses the initial `OnEnter`:
    /// the synthesized `none → initial` transition is overwritten in the same
    /// first pass by the real `initial → requested` transition, so
    /// `on_enter(initial)`-gated systems do NOT run — only `on_enter(requested)`
    /// does. Queue the first transition from *inside* a system (it lands on the
    /// next frame's pass) if you need the initial `OnEnter` to fire first.
    #[cold]
    pub fn insert_state<S: States>(&mut self, initial: S) {
        self.insert_resource(State::new(initial));
        self.insert_resource(NextState::<S>::Unchanged);
        self.insert_resource(StateTransitionRecord::<S>::default());
    }

    /// Inserts the resources backing state type `S` using `S::default()` as the
    /// initial value (Phase 17 D7). Shorthand for `insert_state(S::default())`.
    ///
    /// # Initial-transition interaction
    /// Calling `set_next_state::<S>(..)` (or otherwise queuing a `Pending`)
    /// **before the first `Schedule::run`** suppresses the initial `OnEnter`:
    /// the synthesized `none → initial` transition is overwritten in the same
    /// first pass by the real `initial → requested` transition, so
    /// `on_enter(initial)`-gated systems do NOT run — only `on_enter(requested)`
    /// does. Queue the first transition from *inside* a system (it lands on the
    /// next frame's pass) if you need the initial `OnEnter` to fire first.
    #[cold]
    pub fn init_state<S: States + Default>(&mut self) {
        self.insert_state(S::default());
    }

    /// Returns a shared reference to the current value of state type `S`.
    ///
    /// # Panics
    ///
    /// Panics if `State<S>` was never inserted (via `insert_state` /
    /// `init_state`, or the matching builder methods).
    #[inline]
    pub fn state<S: States>(&self) -> &S {
        self.resource::<State<S>>().get()
    }

    /// Queues a transition of state type `S` to `value`, applied by the next
    /// `Schedule::run`'s transition pass (last-write-wins within a frame).
    ///
    /// Shorthand for `self.resource_mut::<NextState<S>>().set(value)`.
    ///
    /// # Initial-transition interaction
    /// Calling `set_next_state::<S>(..)` (or otherwise queuing a `Pending`)
    /// **before the first `Schedule::run`** suppresses the initial `OnEnter`:
    /// the synthesized `none → initial` transition is overwritten in the same
    /// first pass by the real `initial → requested` transition, so
    /// `on_enter(initial)`-gated systems do NOT run — only `on_enter(requested)`
    /// does. Queue the first transition from *inside* a system (it lands on the
    /// next frame's pass) if you need the initial `OnEnter` to fire first.
    ///
    /// # Panics
    ///
    /// Panics if `NextState<S>` was never inserted (via `insert_state` /
    /// `init_state`, or the matching builder methods).
    #[inline]
    pub fn set_next_state<S: States>(&mut self, value: S) {
        self.resource_mut::<NextState<S>>().set(value);
    }

}
