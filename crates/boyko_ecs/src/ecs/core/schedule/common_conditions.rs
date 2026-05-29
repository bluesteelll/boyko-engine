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
//! * Typed combinators (`.and` / `.or` / `.not`), `on_event`, `in_state` —
//!   deferred; AND-via-chaining covers the common case.

use crate::ecs::core::system::params::local::Local;

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
