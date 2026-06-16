//! Propagation control TLS for custom triggers (Feature 2 D7 / FIX W9).
//!
//! ONLY the `propagate` bool lives here — `target` / `original_target` /
//! `event_id` travel in [`TriggerContext`] BY VALUE (re-entrancy-safe, not TLS).
//! A trigger runner calls [`propagate`] to request or stop bubbling; the
//! [`EcsMaster::trigger`] walk reads it via [`get_propagate`].
//!
//! # Why a thread-local, not an `EcsMaster` field
//!
//! Cross-`Drop` / cross-fire mutable state written through a cached
//! `NonNull<EcsMaster>` is the F2 / 9.3c Tree-Borrows hazard (a foreign write
//! under the dispatcher's `&mut self` protector). A TLS `Cell` touches no world
//! field, so it cannot conflict with any live world reborrow.
//!
//! # Re-entrancy
//!
//! A trigger fired from inside a trigger runner re-enters `EcsMaster::trigger`.
//! [`PropagateGuard`] snapshots the current value on construction and restores
//! it on drop, so the inner walk's propagation requests do not leak into the
//! outer walk.
//!
//! [`TriggerContext`]: crate::ecs::core::component::observers::trigger::TriggerContext
//! [`EcsMaster::trigger`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::trigger

use core::cell::Cell;

thread_local! {
    /// Per-thread "should the current trigger keep bubbling?" flag. Reset per
    /// `trigger` call by [`PropagateGuard`].
    static PROPAGATE: Cell<bool> = const { Cell::new(false) };
}

/// Requests (`true`) or stops (`false`) propagation of the in-flight trigger.
///
/// Called from inside a custom-trigger runner. Has no effect outside a `trigger`
/// walk (the flag is reset on the next walk entry).
#[inline]
pub fn propagate(yes: bool) {
    PROPAGATE.with(|p| p.set(yes));
}

/// Reads the current propagation flag (the `trigger` walk's loop condition).
#[inline]
pub fn get_propagate() -> bool {
    PROPAGATE.with(Cell::get)
}

/// RAII guard that snapshots the propagation TLS on construction and restores it
/// on drop, then seeds it with the initial value for this walk.
///
/// Makes a re-entrant `trigger` (fired from within an observer) propagation-safe:
/// the inner walk's [`propagate`] requests are confined to the inner walk.
pub(crate) struct PropagateGuard {
    saved: bool,
}

impl PropagateGuard {
    /// Saves the current flag and seeds the TLS with `initial` (the event's
    /// `AUTO_PROPAGATE` constant) for this walk.
    #[inline]
    pub(crate) fn enter(initial: bool) -> Self {
        let saved = get_propagate();
        propagate(initial);
        Self { saved }
    }
}

impl Drop for PropagateGuard {
    #[inline]
    fn drop(&mut self) {
        // Restore the outer walk's flag. TLS-only: touches no world field, so
        // the restore cannot conflict with a live world reborrow (F2-safe).
        propagate(self.saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh walk re-seeds the TLS to its `initial` and restores the prior
    /// value on drop. Run first in a fresh-flag world (`Cell::new(false)`).
    #[test]
    fn guard_seeds_initial_and_restores_on_drop() {
        // Establish a known outer value.
        propagate(true);
        {
            let _g = PropagateGuard::enter(false);
            assert!(!get_propagate(), "enter(false) seeds the TLS to false for this walk");
            propagate(true);
            assert!(get_propagate(), "an in-walk propagate(true) is observed within the walk");
        }
        // Drop restored the saved outer value (true), NOT the in-walk mutation.
        assert!(
            get_propagate(),
            "PropagateGuard::drop restores the SAVED outer value, discarding the in-walk write"
        );
        // Clean up the TLS so a later test in this thread is not affected.
        propagate(false);
    }

    /// The re-entrancy invariant (plan item 8b, the NESTED half): a guard
    /// entered from INSIDE another guard's scope confines its `propagate`
    /// requests to the inner scope; the outer walk's flag is restored on the
    /// inner drop, exactly the `EcsMaster::trigger`-re-entered-from-an-observer
    /// case the read-only view cannot drive end-to-end.
    #[test]
    fn nested_guard_does_not_contaminate_outer_walk() {
        // Outer walk: a bubbling event re-seeds true.
        let _outer = PropagateGuard::enter(true);
        assert!(get_propagate(), "outer walk seeded propagate=true");
        {
            // Inner (re-entrant) walk: a non-bubbling event seeds false, and its
            // "observer" flips it (no-op here — already false) then drops.
            let _inner = PropagateGuard::enter(false);
            assert!(!get_propagate(), "inner walk seeded propagate=false");
            propagate(false); // an inner observer's propagate(false)
        }
        // The inner drop restored the OUTER walk's flag (true). The inner
        // propagate(false) did NOT leak out.
        assert!(
            get_propagate(),
            "after the nested walk returns, the outer propagate flag is RESTORED to true \
             (the inner propagate(false) was confined)"
        );
        // PropagateGuard `_outer` drops here, restoring the pre-test value.
    }
}
