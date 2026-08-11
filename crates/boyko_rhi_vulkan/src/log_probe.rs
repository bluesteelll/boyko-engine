//! Test-only observation of this crate's own log records.
//!
//! Four files emit `boyko-2xxx` codes from `#[cold]` reporters that are private to their modules,
//! and nothing outside the crate can call them: an integration test in `tests/` links the library
//! compiled **without** `cfg(test)`, so a `#[cfg(test)]`-only re-export would be invisible to it.
//! The observing tests therefore live in each file's own `#[cfg(test)] mod`, and this module is the
//! one copy of the three helpers all of them need.
//!
//! **Keeping the reporters private is not incidental — it is what gates their WIRING.** A private
//! `fn` whose only call site is deleted becomes `dead_code`, and `cargo clippy --all-targets
//! -- -D warnings` turns that into an error; measured at L7b by deleting the
//! `report_present_mode_fallback` call from `Swapchain::new_with_present_mode`, which produced
//! `error: function report_present_mode_fallback is never used` and exit 101. Making the reporters
//! `pub` so an integration test could reach them would have silently removed that gate, because
//! the tests below call the reporters directly and **cannot** tell whether production code still
//! does. The compiler proves reachability; these tests prove behaviour. Neither substitutes for
//! the other, and the same deletion leaves every test here green.
//!
//! # Why the lock exists, which cost one wrong assumption to learn
//!
//! [`observed`] is a per-TARGET counter, so any concurrent `RhiVulkan` emitter inflates it and an
//! `assert_eq!` becomes flaky. The first version of this module argued the risk away: the only
//! `VulkanContext::boot*` call in this crate's `--lib` test binary is
//! `device::tests::boot_singleton_destroy_singleton_round_trip`, and it passes
//! `enable_validation: false`, so neither `boyko-E2101` arm can fire — therefore, the argument
//! went, nothing else can emit.
//!
//! **That argument was refuted by running it.** It counted the emitters that existed *before* L7b
//! and missed the ones the rung was adding: the tests themselves. A `--lib w2106` run showed
//! `left: 2, right: 1`, the extra record being the sibling `e2103` test's, landing inside the
//! window while the two ran on different harness threads. The full run had passed — on scheduling
//! luck, not on soundness.
//!
//! So the observers serialize on [`observe_lock`]. It is harness plumbing, not engine state: it
//! guards a process-global counter between `#[test]` fns and is compiled out of every shipping
//! build.

use boyko_log::level::Level;
use boyko_log::target::{LogTarget, set_target_level, target_stats};

// Test-harness serialization only, the same exception `device::tests` already carries and for the
// same reason: this guards PROCESS-GLOBAL state between `#[test]` fns on the harness's own threads.
//
// Spelled out in full rather than imported, following `boyko_log::drain_owner`: a `use std::sync::
// {Mutex, ..}` line is ITSELF a use of the disallowed type and would need an `#[allow]` of its own,
// which puts the exception somewhere no reader looks for it.
#[allow(clippy::disallowed_types)]
static OBSERVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Serializes every test that counts `RhiVulkan` records. Held for the whole test body.
///
/// Poison-tolerant: one failing observer must not cascade-fail the rest, because a cascade hides
/// which one actually broke.
#[allow(clippy::disallowed_types)]
pub(crate) fn observe_lock() -> std::sync::MutexGuard<'static, ()> {
    OBSERVE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Records this process has produced on the `RhiVulkan` target, counting **both** routes.
///
/// `delivered + sync_routed`: a thread without a diagnostics lane sends `Warn`/`Error` down the
/// synchronous channel instead of the ring, and which of the two a harness thread does is not
/// something a test can choose. Asserting on `delivered` alone would pass or fail depending on
/// which thread the harness happened to run the test on.
pub(crate) fn observed() -> u64 {
    let s = target_stats(<boyko_log::RhiVulkan as LogTarget>::ID);
    s.0 + s.3
}

/// Raise the `RhiVulkan` ceiling so a `Warn` is admitted. Called **before** the emission.
///
/// The ceiling matters: `W2102`/`W2104`/`W2105`/`W2106` are `Warn`, and a default ceiling that
/// filtered them would make every one of these tests pass for the wrong reason — a green that
/// means "never emitted", not "emitted and counted".
pub(crate) fn arm() {
    set_target_level(<boyko_log::RhiVulkan as LogTarget>::ID, Level::Trace);
}

/// Drain whatever the emission put in the ring. Called **after** it, before reading [`observed`].
///
/// `delivered` is counted inside the drain closure, so a laned thread's record is not visible
/// until someone drains. One successful `drain_once` walks the whole ring, so one is enough.
pub(crate) fn drain() {
    for _ in 0..64 {
        if boyko_log::lifecycle::drain_once().is_some() {
            return;
        }
        std::thread::yield_now();
    }
}
