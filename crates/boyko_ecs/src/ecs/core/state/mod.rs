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
//! The apply machinery (`StateEntry`, `apply_state_transition`) is `pub(crate)`:
//! the run conditions (`in_state` / `on_enter` / `on_exit` / `on_transition`, in
//! `schedule::common_conditions`) and the schedule wiring consume them. The
//! `ResourceId` for each generic state resource is minted through the shared
//! kernel registry [`resources::resource_type_registry`] (published for reuse by
//! `boyko_input`; see its module doc for the rust#22991 rationale).
//! `StateTransitionRecord` /
//! `Transition` are `pub` opaque PODs (L1 fallback, see
//! [`transition_record`]) — they appear in the public condition bounds but
//! carry no public mutators.
//!
//! [`Schedule`]: crate::ecs::core::schedule::schedule::Schedule
//! [`Resource`]: crate::ecs::core::resources::resource::Resource
//! [`resources::resource_type_registry`]: crate::ecs::core::resources::resource_type_registry

pub mod next_state;
#[allow(clippy::module_inception)]
pub mod state;
pub mod state_set;
pub mod states;
pub mod transition_record;

pub use next_state::NextState;
pub use state::State;
pub use state_set::StateTransitionSet;
pub use states::States;

// The generic-resource `ResourceId` minting lives in the shared kernel module
// `crate::ecs::core::resources::resource_type_registry` (published so
// `boyko_input` reuses it — Principle 0). The `State`/`NextState`/
// `StateTransitionRecord` `Resource` impls reach `resource_id_for` by its
// direct path there, so no re-export is needed here.
//
// `StateEntry` + `apply_state_transition` ARE re-exported here because this is
// the import surface the schedule + builder wiring (Phase 17 steps 12/13)
// consume — `schedule.rs` and `schedule_builder.rs` reach them through this
// short path. `StateTransitionRecord` / `Transition` are intentionally NOT
// re-exported: their only consumer (`common_conditions.rs`) reaches them via
// their direct `transition_record::` path, so a re-export here would be
// unreferenced.
pub(crate) use transition_record::{StateEntry, apply_state_transition};
