//! Test-only observation of this logger's own delivery, for the crates that emit into it.
//!
//! Enabled by the `test-probe` feature, which every emitting crate turns on **in its
//! `[dev-dependencies]` only** — so it reaches that crate's test and bench targets and never a
//! shipping build.
//!
//! # Why this is a feature and not a `#[cfg(test)]` module
//!
//! A `#[cfg(test)]` item in this crate is compiled only when *this crate's own* tests are built.
//! Downstream crates link the library compiled **without** `cfg(test)`, so they cannot see it.
//! Rung L7b hit that wall and answered it with a private copy in `boyko_rhi_vulkan/src/`; rung
//! L8a needed the same four helpers in four more crates, at which point one copy behind a feature
//! is the answer and five copies is not. The `boyko_rhi_vulkan` copy was deleted in the same
//! commit rather than left as a second mechanism.
//!
//! # What an observer proves, and the one thing it does not
//!
//! These helpers count **records**, which is what makes a claim like "this site reports once, not
//! once per frame" checkable. They say nothing about whether production code still *calls* the
//! reporter: a test here calls it directly, so deleting the call site leaves every test green.
//! That reachability gate is the **compiler's**, and it works only because the reporters are
//! private `#[cold]` functions — a private `fn` whose last call site is deleted becomes
//! `dead_code`, and `cargo clippy --all-targets -- -D warnings` turns that into an error.
//! Measured at L7b by deleting one call and observing `error: function
//! report_present_mode_fallback is never used`, exit 101. Making a reporter `pub` so a test in
//! `tests/` could reach it would silently remove that gate.
//!
//! # Two observables, and the one to reach for
//!
//! [`watch`]/[`watched`] count **emissions of one code on the calling thread**. [`observed`]
//! counts **deliveries on a target**, process-wide. Prefer the first: each `#[test]` runs on its
//! own thread, so a thread-local emission count is exact no matter what any other test is doing,
//! and it needs no lock at all.
//!
//! **The second one is only sound when every emitter in the test binary can be serialized**, and
//! learning that took two rungs and a measurement each.
//!
//! - L7b built `observed` and argued its risk away by enumerating the emitters that existed
//!   *before* the rung — missing the ones the rung was adding: its own new tests. A `--lib w2106`
//!   run showed `left: 2, right: 1`. The answer there was [`observe_lock`], and it held, because
//!   `boyko_rhi_vulkan`'s emitter set is exactly those tests.
//! - L8a tried the same thing in `boyko_render` and it did not hold. That crate has 468 lib tests,
//!   and *many* of them legitimately drive an emitting path — every test that folds a NaN light,
//!   every test that reads a diverged frozen config. `render_path_config`'s observer failed
//!   `left: 5, right: 2` on **every** run: three extra records from sibling tests that had every
//!   right to exist. Locking them one by one would have worked until the next test anyone wrote.
//!
//! So the observable changed rather than the world. [`note_emission`] is called from `emit_impl`
//! under this feature and charges the record to the **emitting thread**, which is the thread the
//! test is running on. What that counts is "the site emitted, past the ceiling gate" — not "the
//! record reached a sink", which is `boyko_log`'s own business and is tested in `boyko_log`.
//!
//! # An observer of a `Once` site needs THREE things, and each fixes a different failure
//!
//! This took three separate reddenings to establish, so it is written down rather than left to be
//! rediscovered. Counting emissions per thread is necessary and **not sufficient**, because a
//! `Once` latch is process state that a sibling test can spend:
//!
//! | Problem | What fixes it | What it looked like |
//! |---|---|---|
//! | another target's records inflate the count | [`watch`] — per thread, per code | `left: 7, right: 1`, nondeterministic |
//! | a test that ran EARLIER already spent the latch | [`OnceSite::reset`](crate::codes::OnceSite::reset) before the emission | `left: 0, right: 1`, on every run |
//! | a test running CONCURRENTLY spends it mid-window | [`observe_lock`], taken by every test that drives the site | `left: 0, right: 1`, one run in ten |
//!
//! The third is the one with a standing obligation: **a test that drives a `Once` site takes the
//! lock too**, not only the test that observes it. In `boyko_render`'s `light_system` that is five
//! sibling tests which fold NaN or overflowing light tables for reasons entirely their own. The
//! set is per-module and small; the alternative — serializing every test that emits anything —
//! is what L8a tried first and abandoned, because in a 468-test crate it is neither small nor
//! stable.

use core::cell::Cell;

use crate::level::Level;
use crate::target::{LogTarget, set_target_level, target_stats};

thread_local! {
    /// `(class, number)` this thread is counting, or `None` for "every code".
    static WATCH: Cell<Option<(u8, u16)>> = const { Cell::new(None) };
    /// Emissions on this thread matching [`WATCH`] since the last [`watch`] call.
    static COUNT: Cell<u64> = const { Cell::new(0) };
    /// The rendered message of the most recent matching emission on this thread.
    ///
    /// A `Cell<String>`, not a `RefCell`: `RefCell` is a banned type workspace-wide and this
    /// needs none of what it buys. `Cell::take`/`Cell::set` move the `String` in and out, which
    /// is all a single-owner thread-local ever does.
    static LAST: Cell<String> = const { Cell::new(String::new()) };
    // The FIRST matching message since `watch`. See `first_message` for why both exist.
    // A `///` here does not parse: `thread_local!`'s pattern takes attributes, not doc comments.
    static FIRST: Cell<String> = const { Cell::new(String::new()) };
}

/// Charge one emission to the calling thread, and keep its rendered message. Called from
/// `emit_impl` under this feature.
///
/// `Info`/`Debug`/`Trace` arrive with `class == 0` and `code == 0` (Decision 7 gives them no
/// code), so a [`watch`] on a real code never counts them and a [`watch_any`] does.
///
/// # Why the message is rendered here and not read from a sink
///
/// The rendered text is the only place a record's ARGUMENTS are observable, and a count alone
/// cannot see them. L8a found that the hard way: reverting `boyko-W2204` to its pre-migration
/// shape — a reporter called once per dropped light instead of once per fold with a tally —
/// **left every count-based assertion green**, because "one record saying three" and "one record
/// saying one" are both one record. A gate that cannot tell those apart cannot gate the claim the
/// rung was making.
///
/// Rendering at the emission site keeps the observation thread-local, which is the property the
/// whole module exists for. Rendering it at a sink would put it back on the process-global path,
/// where any concurrent emitter can land between a test and its own record.
#[inline]
pub(crate) fn note_emission<A: crate::record::LogArgs>(
    site: &'static crate::site::LogSite,
    args: &A,
) {
    let watched = WATCH.with(Cell::get);
    if !watched.is_none_or(|w| w == (site.class, site.code)) {
        return;
    }
    COUNT.with(|c| c.set(c.get() + 1));

    // A local scratch: this is a test build, the buffer is one stack frame, and it must not be
    // shared with the ring the record is about to go into.
    let mut payload = [0u8; crate::record::MAX_RECORD_BYTES];
    let len = args.encoded_len();
    if len > payload.len() {
        return;
    }
    // SAFETY: `payload` is a local array of `MAX_RECORD_BYTES`, and the branch above established
    //   `len <= payload.len()`, which is exactly the number of bytes `encode` writes.
    let written = unsafe { args.encode(payload.as_mut_ptr()) };

    let mut line = String::new();
    let mut f = crate::site::LogFormatter::new(&mut line);
    crate::record::render_payload(&payload[..written], site.fmt, &mut f);
    // Take ONCE, decide, put back. The first draft called `take()` inside the condition and again
    // in the else arm -- and `Cell::take` leaves the default behind, so the second call returned
    // `""` and the FIRST message was destroyed by the arrival of the second. A datum written and
    // then thrown away, which is the defect this whole campaign is about, in the accessor built to
    // observe it.
    FIRST.with(|fst| {
        let held = fst.take();
        fst.set(if held.is_empty() { line.clone() } else { held });
    });
    LAST.with(|l| l.set(line));
}

/// Start counting emissions of one code on this thread, from zero.
///
/// Takes the class byte as well as the number because the registry allows neither to be inferred
/// from the other at a call site — `warn!` writes `b'W'` into the site and `error!` writes `b'E'`,
/// and a test that watched the number alone would count a same-numbered code of another class.
/// (Check 1 forbids that pair existing, so this is belt-and-braces — but a probe that quietly
/// depended on another check holding is how a green test comes to mean nothing.)
pub fn watch(class: u8, code: u16) {
    WATCH.with(|w| w.set(Some((class, code))));
    COUNT.with(|c| c.set(0));
    LAST.with(|l| l.set(String::new()));
    FIRST.with(|f| f.set(String::new()));
}

/// Start counting **every** emission on this thread, from zero.
pub fn watch_any() {
    WATCH.with(|w| w.set(None));
    COUNT.with(|c| c.set(0));
    LAST.with(|l| l.set(String::new()));
    FIRST.with(|f| f.set(String::new()));
}

/// Emissions matching the current [`watch`] since it was set.
#[must_use]
pub fn watched() -> u64 {
    COUNT.with(Cell::get)
}

/// The rendered message of the most recent matching emission on this thread.
///
/// Empty when nothing has matched since the last [`watch`]. Use it to gate a record's
/// **arguments**, which is the half a count cannot reach — see [`note_emission`] for the
/// measurement that established the difference.
/// The rendered message of the **first** matching emission since [`watch`].
///
/// The mirror of [`last_message`], and it exists because the two answer different questions.
/// MEASURED: the host gate for the session header used `last_message` and read
/// `"logging enabled at debug ..."` — the line the host emits AFTER the header — so it reported the
/// header missing while it was there. An assertion that reads the wrong record accuses the wrong
/// code.
#[must_use]
pub fn first_message() -> String {
    // Take, clone, put back, for the reason `last_message` gives: reading must not consume.
    FIRST.with(|f| {
        let s = f.take();
        let out = s.clone();
        f.set(s);
        out
    })
}

#[must_use]
pub fn last_message() -> String {
    // Take, clone, put back: reading the message must not consume it, or a second assertion in
    // the same test would silently see an empty string and pass.
    LAST.with(|l| {
        let s = l.take();
        let out = s.clone();
        l.set(s);
        out
    })
}

// Test-harness serialization only, and the same exception `drain_owner` already carries for the
// same reason: this guards PROCESS-GLOBAL state between `#[test]` fns on the harness's threads.
//
// Spelled out in full rather than imported: a `use std::sync::{Mutex, ..}` line is ITSELF a use of
// the disallowed type and would need an `#[allow]` of its own, which puts the exception somewhere
// no reader looks for it.
#[allow(clippy::disallowed_types)]
static OBSERVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Serializes every test that counts records. Hold it for the whole test body.
///
/// Poison-tolerant: one failing observer must not cascade-fail the rest, because a cascade hides
/// which one actually broke.
#[allow(clippy::disallowed_types)]
pub fn observe_lock() -> std::sync::MutexGuard<'static, ()> {
    OBSERVE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Records this process has produced on target `T`, counting **both** routes.
///
/// `delivered + sync_routed`: a thread without a diagnostics lane sends `Warn`/`Error` down the
/// synchronous channel instead of the ring, and which of the two a harness thread does is not
/// something a test can choose. Asserting on `delivered` alone would pass or fail depending on
/// which thread the harness happened to run the test on.
#[must_use]
pub fn observed<T: LogTarget>() -> u64 {
    let s = target_stats(<T as LogTarget>::ID);
    s.0 + s.3
}

/// Raise target `T`'s ceiling so a `Warn` is admitted. Call **before** the emission.
///
/// The ceiling matters: most migrated codes are `Warn`, and a default ceiling that filtered them
/// would make every observer pass for the wrong reason — a green that means "never emitted", not
/// "emitted and counted".
pub fn arm<T: LogTarget>() {
    set_target_level(<T as LogTarget>::ID, Level::Trace);
}

/// Drain whatever the emission put in the ring. Call **after** it, before reading [`observed`].
///
/// `delivered` is counted inside the drain closure, so a laned thread's record is not visible
/// until someone drains. One successful `drain_once` walks the whole ring, so one is enough.
pub fn drain() {
    for _ in 0..64 {
        if crate::lifecycle::drain_once().is_some() {
            return;
        }
        std::thread::yield_now();
    }
}
