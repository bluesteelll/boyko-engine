//! The crash path: what is on disk when the process does not get to exit cleanly *(L15)*.
//!
//! # The file is opened on the ENABLE path, never inside the hook
//!
//! A panic hook runs on a thread that is already unwinding, possibly out of stack, possibly
//! holding the allocator lock the panic came from. Opening a file there is the single most likely
//! way to turn a diagnosable crash into a silent one. So the handle is created when diagnostics are
//! turned on -- still before the first frame, still nowhere near the hook -- and the hook only
//! *writes* to something that already exists.
//!
//! `boyko-E0109` reports a crash file that could not be opened, at enable time, when there is still
//! a healthy process to tell.
//!
//! # The crash file IS the file sink, pointed at a different path
//!
//! [`arm`] calls [`file::set_path`](crate::sink::file::set_path) and opens it. There is no separate
//! crash destination, and there should not be: one that only received records at panic time would
//! be **empty**, because the drain empties the ring continuously under every `SinkMode`. What makes
//! a crash file work is that it is an ordinary continuous sink which happens to survive the crash.
//!
//! The consequence a caller must know: **arming replaces the file sink's path.** A process cannot
//! have an ordinary text log and a separate crash log; it has one file, which contains both. The
//! preset table in [`crate::preset`] used to imply otherwise and no longer does.
//!
//! # The hook's protocol, and why step 1.5 exists
//!
//! - **Step 1** -- mark the sink `Exiting`, so nothing new is admitted.
//! - **Step 1.5, `PRE_FLUSH`** -- publish that a flush is *about to* start. A second thread
//!   panicking concurrently must not both wait for and race the first one's flush; it observes
//!   `PRE_FLUSH` and writes its own line without re-entering the drain.
//! - **Step 2** -- claim `DRAIN_OWNER`. **`try_claim`, never a blocking wait**: the role may be
//!   held by a host draining by hand, and a panic hook that waits for it turns a crash into a
//!   hang. A refused claim is `boyko-E0118` -- the flush did not happen and the file says so,
//!   which is strictly better than a file that is merely short.
//! - **Step 3** -- drain into the crash file.
//!
//! **`SINK_STATE` does not regain an exclusivity role.** It says what the sink is doing, not who
//! may touch it; the drain token remains the only exclusion.

use core::sync::atomic::{AtomicU8, Ordering};

/// What the crash path is doing. Ordinary operation is `Ready`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum CrashState {
    /// No crash file. The default, and the state of a process that never enabled diagnostics.
    Absent = 0,
    /// A crash file exists and the hook can use it.
    Ready = 1,
    /// A panic is in progress and a flush is about to start (step 1.5).
    PreFlush = 2,
    /// The process is on its way out; nothing new is admitted.
    Exiting = 3,
}

static STATE: AtomicU8 = AtomicU8::new(CrashState::Absent as u8);

/// Read the crash path's state.
#[must_use]
pub fn state() -> CrashState {
    match STATE.load(Ordering::Acquire) {
        1 => CrashState::Ready,
        2 => CrashState::PreFlush,
        3 => CrashState::Exiting,
        _ => CrashState::Absent,
    }
}

/// Open the crash file. **Called on the enable path, never from a hook.**
///
/// Returns `false` and reports `boyko-E0109` if the destination cannot be opened -- reported HERE,
/// with a healthy process to receive it, rather than discovered during a panic when there is
/// nothing left to tell.
#[must_use]
pub fn arm(path: &str) -> bool {
    if !crate::sink::file::set_path(path) {
        report_unopenable(path);
        return false;
    }
    if !crate::sink::file::open(0) {
        report_unopenable(path);
        return false;
    }
    STATE.store(CrashState::Ready as u8, Ordering::Release);
    true
}

/// The `E0109` latch, a NAMED module-level `static` so an observer can control it.
///
/// Without it this site's `Once` was honoured by the CALL STRUCTURE -- `arm` runs once on the
/// enable path -- rather than by anything at the site. A process that disables and re-enables
/// calls `arm` again, and a row declaring `Once` would have reported again. Found by the `Once`
/// register (`crate::once_sites`), which is the first thing in this crate able to see it.
static UNOPENABLE_REPORTED: crate::codes::OnceSite = crate::codes::OnceSite::new();

/// `boyko-E0109`: the crash file could not be opened.
///
/// `Once`: one process has one crash destination, and a second attempt fails for the same reason.
#[cold]
#[inline(never)]
fn report_unopenable(path: &str) {
    if !UNOPENABLE_REPORTED.claim() {
        return;
    }
    crate::error!(
        crate::Log,
        crate::codes::E0109,
        "the crash file {} could not be opened; a panic in this process will leave no record",
        path
    );
}

/// `boyko-E0118`: the panic hook could not complete a flush.
///
/// `Every`: two threads panicking are two facts, and a latch would report one of them.
#[cold]
#[inline(never)]
fn report_flush_refused() {
    crate::error!(
        crate::Log,
        crate::codes::E0118,
        "the panic hook could not claim the drain role; the crash file is short by {} pass",
        1u32
    );
}

/// The hook's body: mark, pre-flush, try to drain. **Never blocks, never allocates a destination.**
///
/// Returns `true` when the flush completed. `false` means `E0118` was reported and the file is
/// short -- a fact stated in the file rather than a shortfall the reader has to infer.
pub fn on_panic() -> bool {
    // Step 1: nothing new is admitted. Step 1.5: publish that a flush is ABOUT to start, so a
    // concurrently-panicking thread neither waits for it nor races into the drain behind it.
    STATE.store(CrashState::Exiting as u8, Ordering::Release);
    STATE.store(CrashState::PreFlush as u8, Ordering::Release);

    // Step 2: `try_claim`, never a blocking wait. The role may be held by a host draining by hand,
    // and a hook that waits for it converts a crash into a hang -- which is worse, because a hang
    // has no artifact at all.
    let Some(token) = crate::drain_owner::try_claim() else {
        STATE.store(CrashState::Exiting as u8, Ordering::Release);
        report_flush_refused();
        return false;
    };
    drop(token);
    // Step 3: the ordinary drain does the writing; the hook does not reimplement it, because a
    // second renderer is a second thing to keep in step with the record format.
    let ran = crate::lifecycle::drain_once().is_some();
    STATE.store(CrashState::Exiting as u8, Ordering::Release);
    ran
}

/// Install the panic hook, chaining the previous one.
///
/// Chained rather than replaced: the default hook prints the panic message and location, and a
/// diagnostics subsystem that silences the standard panic output has made the process HARDER to
/// debug in exchange for making it easier.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        on_panic();
        previous(info);
    }));
}

/// Reset to `Absent`. For tests; a process has one crash path and does not disarm it in anger.
pub fn disarm() {
    STATE.store(CrashState::Absent as u8, Ordering::Release);
}
