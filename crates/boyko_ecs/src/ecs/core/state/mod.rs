//! Phase 17 — application/game states layered on the single [`Schedule`].
//!
//! Bevy-style states implemented as ordinary condition-gated systems fed by a
//! built-in per-frame transition pass. See `docs/PHASE-17-PLAN.md`.
//!
//! * [`States`] — the marker trait users implement on their state enum.
//! * [`State<S>`] — the current value (a [`Resource`]).
//! * [`NextState<S>`] — a queued transition request (a [`Resource`]).
//! * [`StateTransitionSet`] — an opt-in ordering hook (D10).
//!
//! The apply machinery (`StateEntry`, `apply_state_transition`) and the
//! `ResourceId` registry are `pub(crate)`: the run conditions (`in_state` /
//! `on_enter` / `on_exit` / `on_transition`, in `schedule::common_conditions`)
//! and the schedule wiring consume them. `StateTransitionRecord` /
//! `Transition` are `pub` opaque PODs (L1 fallback, see
//! [`transition_record`]) — they appear in the public condition bounds but
//! carry no public mutators.
//!
//! [`Schedule`]: crate::ecs::core::schedule::schedule::Schedule
//! [`Resource`]: crate::ecs::core::resources::resource::Resource

pub mod next_state;
#[allow(clippy::module_inception)]
pub mod state;
pub mod state_resource_registry;
pub mod state_set;
pub mod states;
pub mod transition_record;

pub use next_state::NextState;
pub use state::State;
pub use state_set::StateTransitionSet;
pub use states::States;

// `state_resource_registry::resource_id_for` is intentionally NOT re-exported
// here: its only callers are the `State`/`NextState`/`StateTransitionRecord`
// `Resource` impls inside this module, which reach it by its direct path. A
// `pub(crate) use` would be an unreferenced re-export (the module itself is
// `pub mod`, so any future consumer can still use the full path).
//
// `StateEntry` + `apply_state_transition` ARE re-exported here because this is
// the import surface the schedule + builder wiring (Phase 17 steps 12/13)
// consume — `schedule.rs` and `schedule_builder.rs` reach them through this
// short path. `StateTransitionRecord` / `Transition` are intentionally NOT
// re-exported: their only consumer (`common_conditions.rs`) reaches them via
// their direct `transition_record::` path, so a re-export here would be
// unreferenced.
pub(crate) use transition_record::{StateEntry, apply_state_transition};
