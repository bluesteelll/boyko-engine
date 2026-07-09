//! Built-in run conditions (Phase 16).
//!
//! A run condition is any `impl IntoSystem<(), bool, M>` — it reuses the
//! whole `SystemParamFunction` / `FunctionSystem` machinery, so a plain
//! `fn(...) -> bool` becomes a condition for free (see
//! `PHASE-16-PLAN.md` §1 / §8). This module ships the only built-in that
//! Phase 16 commits to: [`run_once`].
//!
//! # Deferred (see `PHASE-16-PLAN.md` §8.4 / §1)
//!
//! * `resource_exists::<R>` — needs an `Option<Res<R>>` (or `Has<R>`)
//!   `SystemParam` that returns instead of panicking on a missing resource;
//!   boyko has none today, so it is deferred to a follow-up.
//! * Typed combinators (`.and` / `.or` / `.not`), `on_event` — deferred;
//!   AND-via-chaining covers the common case.
//!
//! Phase 17 adds the state run conditions [`in_state`] / [`on_enter`] /
//! [`on_exit`] / [`on_transition`].

use crate::ecs::core::state::State;
use crate::ecs::core::state::states::States;
use crate::ecs::core::state::transition_record::StateTransitionRecord;
use crate::ecs::core::system::into_system::IntoSystem;
use crate::ecs::core::system::params::local::Local;
use crate::ecs::core::system::params::res::Res;
use crate::ecs::core::system::system::System;

/// A condition that returns `true` exactly once — the first frame it is
/// evaluated — and `false` forever after.
///
/// Backed by a [`Local<bool>`] that flips on first run. Because conditions
/// are eager-folded (never short-circuited — `PHASE-16-PLAN.md` §6), the
/// `Local` advances every frame the condition is reached, so a system
/// gated by `.run_if(run_once)` runs on exactly one frame even when chained
/// behind another condition.
///
/// # Example
///
/// ```ignore
/// builder.add_system(spawn_player).run_if(run_once);
/// // `spawn_player` runs on frame 1 only.
/// ```
///
/// [`Local<bool>`]: crate::ecs::core::system::params::local::Local
#[inline]
pub fn run_once(mut has_run: Local<bool>) -> bool {
    if *has_run {
        false
    } else {
        *has_run = true;
        true
    }
}

/// A condition that holds while the current [`State<S>`] equals `target`
/// (Phase 17 D5).
///
/// Reads `State<S>` (shared), so multiple `in_state`-gated systems never
/// conflict. The returned closure captures `target` by value.
///
/// ```ignore
/// builder.add_system(run_physics).run_if(in_state(AppState::InGame));
/// ```
///
/// # Panics
///
/// The returned condition panics if `State<S>` was never inserted (require-
/// exists, D8): register the state with `init_state::<S>()` / `insert_state`
/// before adding any `in_state`/`on_enter`/`on_exit`/`on_transition`-gated
/// system.
///
/// # Missed-events footgun (§13-OQ2)
///
/// An `EventReader` inside an `in_state`-gated system advances its cursor only
/// on frames the system runs, so events sent while the state was inactive are
/// skipped (standard Bevy behaviour). This phase does not address it; route
/// state-spanning events through a system that is not `in_state`-gated if you
/// must observe them.
///
/// [`State<S>`]: crate::ecs::core::state::state::State
#[inline]
pub fn in_state<S: States>(target: S) -> impl System<Out = bool> {
    // The closure type is concrete HERE, so the double-`FnMut` HRTB bound
    // resolves; `into_system` produces a `FunctionSystem`. Returning it as
    // `impl System<Out = bool>` (a plain trait, unlike the closure's
    // HRTB-projected `FnMut` bound) survives the opaque-return boundary, and
    // the IS2 identity blanket re-bridges it to `IntoSystem` for `.run_if`.
    IntoSystem::into_system(move |current: Res<State<S>>| current.get() == &target)
}

/// A condition that holds on the single frame state type `S` transitions
/// *into* `target` (Phase 17 D5).
///
/// Fires on the exact frame the transition pass records an entry into
/// `target`, including the synthesized initial `none → target` transition on
/// frame 1 (D7). Reads the per-`S` transition record (shared).
///
/// ```ignore
/// builder.add_system(spawn_level).run_if(on_enter(AppState::InGame));
/// ```
///
/// # Panics
///
/// Panics if `S` was never registered — see [`in_state`].
#[inline]
pub fn on_enter<S: States>(target: S) -> impl System<Out = bool> {
    // See `in_state` for why the closure is wrapped via `into_system` and
    // returned as `impl System<Out = bool>` rather than `impl FnMut(..)`.
    IntoSystem::into_system(move |rec: Res<StateTransitionRecord<S>>| {
        matches!(rec.current(), Some(t) if t.entered == target)
    })
}

/// A condition that holds on the single frame state type `S` transitions *out
/// of* `target` (Phase 17 D5).
///
/// Fires on the exact frame the transition pass records an exit from `target`.
/// The synthesized initial transition has no `exited` value, so `on_exit` is
/// naturally false on frame 1. Reads the per-`S` transition record (shared).
///
/// ```ignore
/// builder.add_system(teardown_menu).run_if(on_exit(AppState::Menu));
/// ```
///
/// # Panics
///
/// Panics if `S` was never registered — see [`in_state`].
#[inline]
pub fn on_exit<S: States>(target: S) -> impl System<Out = bool> {
    // See `in_state` for why the closure is wrapped via `into_system` and
    // returned as `impl System<Out = bool>` rather than `impl FnMut(..)`.
    IntoSystem::into_system(move |rec: Res<StateTransitionRecord<S>>| {
        matches!(rec.current(), Some(t) if t.exited.as_ref() == Some(&target))
    })
}

/// A condition that holds on the single frame state type `S` transitions
/// exactly from `from` into `to` (Phase 17 D11).
///
/// Reads the same per-`S` transition record as [`on_enter`] / [`on_exit`],
/// matching both endpoints. Fires only for the exact `(from, to)` pair.
///
/// ```ignore
/// builder.add_system(fade).run_if(on_transition(AppState::Menu, AppState::InGame));
/// ```
///
/// # Panics
///
/// Panics if `S` was never registered — see [`in_state`].
#[inline]
pub fn on_transition<S: States>(from: S, to: S) -> impl System<Out = bool> {
    // See `in_state` for why the closure is wrapped via `into_system` and
    // returned as `impl System<Out = bool>` rather than `impl FnMut(..)`.
    IntoSystem::into_system(move |rec: Res<StateTransitionRecord<S>>| {
        matches!(rec.current(), Some(t) if t.exited.as_ref() == Some(&from) && t.entered == to)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `run_once` returns `true` on the first evaluation against a fresh
    /// (`false`) `Local<bool>` and flips the flag. This is the unit-level twin
    /// of the integration `run_once_runs_body_exactly_once_over_three_frames` —
    /// it drives the `Local<bool>` slot directly (no scheduler) to isolate the
    /// built-in's own logic.
    #[test]
    fn run_once_true_on_first_eval_flips_flag() {
        let mut flag = false;
        let verdict = run_once(Local(&mut flag));
        assert!(verdict, "first eval returns true");
        assert!(flag, "first eval sets the Local flag");
    }

    /// `run_once` returns `false` on every subsequent evaluation (the flag is
    /// already set). Drives the same `Local` slot twice to model the
    /// frame-to-frame persistence the real `FunctionSystem::state` provides.
    #[test]
    fn run_once_false_after_flag_set() {
        let mut flag = false;
        let _ = run_once(Local(&mut flag)); // frame 1
        let second = run_once(Local(&mut flag)); // frame 2 — flag now true
        assert!(!second, "second eval returns false");
        let third = run_once(Local(&mut flag)); // frame 3
        assert!(!third, "third eval also returns false");
        assert!(flag, "the flag remains set");
    }
}
