//! This crate's binding of [`boyko_log::probe`] to its own [`boyko_log::Render`] target.
//!
//! One function, because one is all this crate's observers need: they count **emissions on the
//! calling thread** via `boyko_log::probe::{watch, watched}`, which is target-agnostic and needs
//! no lock. What is still per-target is the ceiling — a `Warn` below the target's runtime ceiling
//! is never emitted at all, and an observer that forgot to raise it would pass for the wrong
//! reason, reporting "never emitted" as success.
//!
//! See that module's header for why emission is the observable here and delivery is not: this
//! crate has 468 lib tests and many of them legitimately drive an emitting path, so a
//! process-global delivery counter could not be isolated by any amount of locking.

/// Raise the `Render` ceiling so a `Warn` is admitted. Called **before** the emission.
pub(crate) fn arm() {
    boyko_log::probe::arm::<boyko_log::Render>();
}
