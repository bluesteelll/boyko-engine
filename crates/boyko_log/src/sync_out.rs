//! The synchronous channel: `OUT_LOCK` and `write_oracle_line`.
//!
//! This is the **error-of-the-error path**. Five of its seven callers are places something has
//! already gone wrong — the lane-exhaustion fallback, the pre-`enable` and post-`shutdown`
//! fallback, the panic message, `flush()`'s timeout line, and a panic inside the sink itself. A
//! lock that can hang here hangs the process at the exact moment the process is trying to explain
//! itself.
//!
//! # Why the lock steals instead of waiting
//!
//! The predecessor was an unbounded `AtomicBool` spin with no release-on-unwind and no
//! re-entrancy story, and three concrete hangs followed from it — each on that same path. A panic
//! *inside* the sink direct-writes, which self-deadlocks if the sink held the lock; `flush()`'s
//! bounded timeout terminated in an unbounded wait; and a `Display` that panicked mid-format
//! leaked the lock permanently, so the panic hook's flush hung.
//!
//! Against a repository invariant that admits no kill-after-timeout pattern to borrow, and whose
//! own bench gate states it plainly: *a worker that never terminates is a RED whose message is its
//! own silence.* So the protocol is:
//!
//! 1. **Format before you lock.** Callers render into their own stack buffer first, so no user
//!    `Display` and no `core::fmt` runs inside the critical section and an unwind cannot originate
//!    there.
//! 2. **Re-entrancy is detected, not deadlocked.** Acquire is `CAS(0 → my_token)`; on failure, a
//!    caller that finds its *own* token is re-entrant. Its bytes are written prefixed by a
//!    newline, so they cannot corrupt the **start** of the outer line, and the occurrence is
//!    counted.
//! 3. **Acquire is bounded** — spin, then yield, to a 50 ms deadline. On expiry the writer
//!    **steals**: it writes anyway and counts. An interleaved line is a legible defect; a hung
//!    process is not. That is the explicit trade.
//! 4. **Release is unwind-safe by construction**: [`OutGuard`]'s `Drop`, and the guard is the only
//!    way to obtain write access.
//!
//! # Line integrity, and what is NOT claimed
//!
//! Writes go through `std::io::stderr()`'s **own handle** — never a raw fd, never `libc::write` —
//! so they share stderr's inner lock with the engine's other stderr producer and neither can
//! splice a line into the other. **Ordering between the two is undefined and is not claimed**: a
//! log line may land between two validation lines. Line integrity is what the golden gate consumes
//! and line integrity is what the shared handle buys. Under a *steal*, two of this module's own
//! outputs may interleave with each other — which is why a non-zero steal count is itself a defect
//! signal rather than a statistic.

use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// How long an acquire may wait before stealing.
const ACQUIRE_DEADLINE: std::time::Duration = std::time::Duration::from_millis(50);

/// 0 = free; otherwise an opaque, non-zero, per-live-thread token.
static OUT_OWNER: AtomicU64 = AtomicU64::new(0);

/// Acquires that expired and wrote anyway.
static OUT_STEALS: AtomicU32 = AtomicU32::new(0);

/// Acquires that found their own token — the re-entrant, non-deadlocking case.
static OUT_REENTRANT: AtomicU32 = AtomicU32::new(0);

/// Whether any synchronous destination is configured.
///
/// **`.bss`-false, and that is the flag-off property.** With diagnostics disabled there is no
/// synchronous destination, [`write_oracle_line`] is a no-op, and every mechanism that depends on
/// it is inert — correct, because a player who did not ask for diagnostics gets none. The `off`
/// build reaches the same end by a different route: it deletes the call sites. Two cases, two
/// reasons, and this one is the run-time half.
static CONSOLE_ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// One byte per thread, used only for its ADDRESS.
    ///
    /// A thread-local's address is unique among live threads and never zero, which is exactly what
    /// the owner slot needs and what `ThreadId` cannot portably provide as a `u64`. The address
    /// may be recycled after a thread exits — harmless, because the slot is only compared while a
    /// thread holds it, and a dead thread holds nothing.
    static TOKEN_ANCHOR: u8 = const { 0 };
}

/// This thread's opaque token.
#[inline]
fn my_token() -> u64 {
    TOKEN_ANCHOR.with(|a| std::ptr::from_ref(a) as u64)
}

/// How write access was obtained.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutMode {
    /// The lock was free and this thread took it.
    Held,
    /// This thread already held it. Bytes are newline-prefixed.
    Reentrant,
    /// The deadline expired and the writer proceeded anyway.
    Stolen,
}

/// Write access to the synchronous channel. Releasing is `Drop`'s job, on the normal path **and**
/// on unwind.
pub struct OutGuard {
    mode: OutMode,
}

impl OutGuard {
    /// How this guard obtained access.
    #[must_use]
    pub fn mode(&self) -> OutMode {
        self.mode
    }
}

impl Drop for OutGuard {
    fn drop(&mut self) {
        // Only the thread that actually TOOK the lock may release it. A re-entrant guard is an
        // inner frame of an outer one and must leave the owner alone; a stolen guard never owned
        // it, and clearing the slot there would free a lock another thread is still inside.
        if self.mode == OutMode::Held {
            OUT_OWNER.store(0, Ordering::Release);
        }
    }
}

/// Acquire the synchronous channel, bounded.
///
/// Never blocks indefinitely and never returns an error: every outcome is a guard, and the mode
/// says which. A caller on the error-of-the-error path cannot be asked to handle a failure to
/// report a failure.
#[must_use]
pub fn acquire() -> OutGuard {
    let me = my_token();

    if OUT_OWNER
        .compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        return OutGuard { mode: OutMode::Held };
    }
    if OUT_OWNER.load(Ordering::Relaxed) == me {
        OUT_REENTRANT.fetch_add(1, Ordering::Relaxed);
        return OutGuard { mode: OutMode::Reentrant };
    }

    let start = std::time::Instant::now();
    let mut spins = 0u32;
    loop {
        if OUT_OWNER
            .compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return OutGuard { mode: OutMode::Held };
        }
        // The owner can change while we wait, and a thread that acquired, released and
        // re-acquired could hand us our own token back only if we still held it -- we do not.
        if start.elapsed() >= ACQUIRE_DEADLINE {
            OUT_STEALS.fetch_add(1, Ordering::Relaxed);
            return OutGuard { mode: OutMode::Stolen };
        }
        if spins < 64 {
            std::hint::spin_loop();
            spins += 1;
        } else {
            std::thread::yield_now();
        }
    }
}

/// Write one already-formatted line to every configured synchronous destination.
///
/// `prefix` and `body` are written back to back followed by a newline, under **one** guard, so a
/// line cannot be split across an acquire boundary. Both must already be rendered: formatting
/// inside the critical section is what the protocol's first rule forbids.
///
/// Returns how access was obtained, so a caller that cares — a golden gate, the census — can see a
/// steal. Returns `None` when no synchronous destination is configured, which is the flag-off and
/// `off`-profile case and is **not** an error.
pub fn write_oracle_line(prefix: &str, body: &str) -> Option<OutMode> {
    if !CONSOLE_ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    if refused_here() {
        return None;
    }
    write_line_with(acquire(), prefix, body)
}

/// [`write_oracle_line`] for a report about a stalled consumer: written only if the channel goes
/// idle within [`ACQUIRE_DEADLINE`], and never through a steal.
///
/// A thread stalled writing to the console holds this channel AND stderr's own lock for as long
/// as its consumer does not read. [`write_oracle_line`] would wait out the deadline, steal, and
/// then wait on stderr's lock without a bound. That holds whatever the stalled write was -- a
/// sink pass's staged lines or its overflow, the file sink's cap or rotation report, an un-laned
/// fallback line -- so the test is on the channel, not on the cause.
///
/// Returns `None` when nothing was written, which includes a channel still busy at the deadline.
/// A writer outside this module (an `eprintln!`) takes stderr's lock without this channel, and is
/// invisible here.
pub(crate) fn write_oracle_line_if_idle(prefix: &str, body: &str) -> Option<OutMode> {
    if !CONSOLE_ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    if refused_here() {
        return None;
    }
    write_line_with(acquire_idle()?, prefix, body)
}

/// One line under a guard the caller obtained: the write itself, counted in [`WRITES_IN_FLIGHT`].
fn write_line_with(g: OutGuard, prefix: &str, body: &str) -> Option<OutMode> {
    // A local drops before the parameter `g`: the count falls before `g` releases.
    let _writing = InFlight::enter();
    let mut err = std::io::stderr();
    #[cfg(test)]
    test_stall::hold_if_armed();
    // A re-entrant write is an inner frame of a line already in progress. The leading newline is
    // what stops it corrupting the START of the outer line, which is the property the golden
    // gate's line-start match consumes.
    if g.mode() == OutMode::Reentrant {
        let _ = err.write_all(b"\n");
    }
    let _ = err.write_all(prefix.as_bytes());
    let _ = err.write_all(body.as_bytes());
    let _ = err.write_all(b"\n");
    // Errors are dropped deliberately: this is the channel of last resort, and a failure to
    // report a failure has nowhere left to go.
    let _ = err.flush();
    Some(g.mode())
}

/// Synchronous writes between their acquire and their flush, in every [`OutMode`].
///
/// `OUT_OWNER` alone cannot see a writer that STOLE the channel: once the thread it stole from
/// releases, the owner slot reads free while the stealer may still be blocked in its write,
/// holding stderr's lock. [`acquire_idle`] reads this as well.
static WRITES_IN_FLIGHT: AtomicU32 = AtomicU32::new(0);

/// One write counted in [`WRITES_IN_FLIGHT`] for the guard's lifetime, unwinding included.
struct InFlight;

impl InFlight {
    fn enter() -> InFlight {
        // Relaxed: nothing is published through the increment. A stealer's increment has no edge
        // to `acquire_idle` to be ordered by; that race is stated there.
        WRITES_IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
        InFlight
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        // Release: pairs with the Acquire load in `acquire_idle`, through the writer's own
        // `OutGuard` release when it held the channel (the guard drops after this).
        WRITES_IN_FLIGHT.fetch_sub(1, Ordering::Release);
    }
}

/// [`acquire`] without the steal: `None` if the channel is still busy at [`ACQUIRE_DEADLINE`] --
/// held by another thread, or being written by one that stole it.
///
/// A caller already inside a write of its own on this thread is refused too: its own token is in
/// the slot, and this never re-enters.
///
/// What it cannot close: a writer that has been waiting out the deadline on a previous owner can
/// steal between the check below and this caller's write, and block in front of it. That takes a
/// third writer, and a deadline that expires inside the microseconds this caller needs.
fn acquire_idle() -> Option<OutGuard> {
    let me = my_token();
    let start = std::time::Instant::now();
    let mut spins = 0u32;
    loop {
        if OUT_OWNER
            .compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            let held = OutGuard { mode: OutMode::Held };
            // Acquire: pairs with the Release in `InFlight::drop`. A writer that HELD the channel
            // decremented before its guard's Release freed the slot this CAS just read, so what
            // this load can still find is a writer that stole.
            if WRITES_IN_FLIGHT.load(Ordering::Acquire) == 0 {
                return Some(held);
            }
            drop(held);
        }
        if start.elapsed() >= ACQUIRE_DEADLINE {
            return None;
        }
        if spins < 64 {
            std::hint::spin_loop();
            spins += 1;
        } else {
            std::thread::yield_now();
        }
    }
}

/// Writes this channel refused because the calling thread was inside a late close.
///
/// A `shutdown` whose join expired delivers the remaining records to the durable sinks itself, on
/// the calling thread, while the sink thread may still be blocked writing to a console consumer
/// that stopped reading — and that thread holds stderr's lock while it is blocked. A write from
/// the late close would wait on that lock without a bound, so the late close refuses the channel
/// and counts what it refused. Read alongside
/// [`shutdown_outcome`](crate::lifecycle::shutdown_outcome).
static SUPPRESSED: AtomicU32 = AtomicU32::new(0);

/// Synchronous writes refused inside a late close (see [`SUPPRESSED`]); `0` in every process
/// whose shutdown joined.
#[must_use]
pub fn suppressed_writes() -> u32 {
    SUPPRESSED.load(Ordering::Relaxed)
}

thread_local! {
    /// Set while THIS thread runs a late close ([`refuse_on_this_thread`]).
    ///
    /// Per thread, not process-wide: the refusal is about the late close's own writes, and a
    /// panic message from another thread at the same moment must still reach the console.
    static REFUSED_HERE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Whether this thread is inside a late close; counts the refusal when it is.
#[inline]
fn refused_here() -> bool {
    let refused = REFUSED_HERE.with(core::cell::Cell::get);
    if refused {
        SUPPRESSED.fetch_add(1, Ordering::Relaxed);
    }
    refused
}

/// Refuses this channel on the calling thread until dropped (see [`SUPPRESSED`]).
pub(crate) struct RefuseGuard {
    was: bool,
}

impl Drop for RefuseGuard {
    fn drop(&mut self) {
        REFUSED_HERE.with(|r| r.set(self.was));
    }
}

/// Refuse the synchronous channel on the calling thread for the guard's lifetime.
pub(crate) fn refuse_on_this_thread() -> RefuseGuard {
    RefuseGuard { was: REFUSED_HERE.with(|r| r.replace(true)) }
}

/// Write a block of already-formatted, newline-terminated lines to every configured synchronous
/// destination: one guard, one write, one flush.
///
/// The resident sink thread stages a pass's console lines and writes them here AFTER it has
/// released the drain role (`lifecycle`'s `sink_loop`). A console consumer that stops reading then
/// blocks this call and nothing else: the durable sinks already hold the pass, and a `shutdown`
/// whose join expires can still claim the role and close them. The bytes are the ones
/// [`write_oracle_line`] would have written line by line.
pub(crate) fn write_oracle_block(block: &[u8]) -> Option<OutMode> {
    if !CONSOLE_ENABLED.load(Ordering::Relaxed) || block.is_empty() {
        return None;
    }
    if refused_here() {
        return None;
    }
    let g = acquire();
    // Declared after `g`, so it drops first: the count falls before `g` releases.
    let _writing = InFlight::enter();
    let mut err = std::io::stderr();
    #[cfg(test)]
    test_stall::hold_if_armed();
    if g.mode() == OutMode::Reentrant {
        let _ = err.write_all(b"\n");
    }
    let _ = err.write_all(block);
    let _ = err.flush();
    Some(g.mode())
}

/// A console consumer that stops reading, modelled for the lifecycle tests.
///
/// Armed, it stalls the next synchronous write ON THE SINK THREAD (the thread named
/// `boyko-log-sink`) exactly where a blocked `WriteFile` would: after the channel's guard is
/// taken, holding stderr's lock, until disarmed. Another thread's stderr write then waits on that
/// lock, as it would behind a full pipe. The stall gives up after `GIVE_UP_AFTER` and records that
/// it did, so a test that would otherwise hang reds with a reason instead.
#[cfg(test)]
pub(crate) mod test_stall {
    use std::sync::atomic::{AtomicBool, Ordering};

    static ARMED: AtomicBool = AtomicBool::new(false);
    static PANIC: AtomicBool = AtomicBool::new(false);
    static ENTERED: AtomicBool = AtomicBool::new(false);
    static GAVE_UP: AtomicBool = AtomicBool::new(false);

    /// The longest a stall waits to be disarmed.
    const GIVE_UP_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

    pub(crate) fn arm() {
        ENTERED.store(false, Ordering::Release);
        GAVE_UP.store(false, Ordering::Release);
        PANIC.store(false, Ordering::Release);
        ARMED.store(true, Ordering::Release);
    }

    /// Arm a sink-thread write to PANIC instead of stalling: a sink thread that dies.
    pub(crate) fn arm_panic() {
        arm();
        PANIC.store(true, Ordering::Release);
    }

    pub(crate) fn disarm() {
        ARMED.store(false, Ordering::Release);
        PANIC.store(false, Ordering::Release);
    }

    /// Whether an armed stall has caught a sink-thread write.
    pub(crate) fn entered() -> bool {
        ENTERED.load(Ordering::Acquire)
    }

    /// Whether a stall waited out `GIVE_UP_AFTER`: nothing disarmed it, so the test that armed it
    /// was itself waiting on the stalled console.
    pub(crate) fn gave_up() -> bool {
        GAVE_UP.load(Ordering::Acquire)
    }

    pub(super) fn hold_if_armed() {
        if !ARMED.load(Ordering::Acquire)
            || std::thread::current().name() != Some("boyko-log-sink")
        {
            return;
        }
        if PANIC.swap(false, Ordering::AcqRel) {
            ARMED.store(false, Ordering::Release);
            ENTERED.store(true, Ordering::Release);
            panic!("test_stall: the sink thread dies in a console write");
        }
        let _held = std::io::stderr().lock();
        ENTERED.store(true, Ordering::Release);
        let start = std::time::Instant::now();
        while ARMED.load(Ordering::Acquire) {
            if start.elapsed() >= GIVE_UP_AFTER {
                GAVE_UP.store(true, Ordering::Release);
                return;
            }
            std::thread::yield_now();
        }
    }
}

/// Turn the console destination on. Called from the enable path, never at process start.
pub fn set_console_enabled(on: bool) {
    CONSOLE_ENABLED.store(on, Ordering::Relaxed);
}

/// Acquires that expired and wrote anyway. **A non-zero value in a golden run is a defect
/// signal**, not a statistic: it means two synchronous lines may have interleaved.
#[must_use]
pub fn out_steals() -> u32 {
    OUT_STEALS.load(Ordering::Relaxed)
}

/// Acquires that found their own token. Reported by the census; expected to be zero outside the
/// panic-inside-the-sink path.
#[must_use]
pub fn out_reentrant() -> u32 {
    OUT_REENTRANT.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    // These tests share `OUT_OWNER`, so they take it in turn under one mutex of their own. The
    // alternative -- letting them race -- would make a steal test pass by stealing from a sibling
    // test rather than from the thread it set up, which is a green for the wrong reason.
    #[allow(clippy::disallowed_types)]

    #[test]
    fn a_free_lock_is_taken_and_released() {
        let _s = crate::drain_owner::test_serial();
        {
            let g = acquire();
            assert_eq!(g.mode(), OutMode::Held);
            assert_ne!(OUT_OWNER.load(Ordering::Relaxed), 0);
        }
        assert_eq!(OUT_OWNER.load(Ordering::Relaxed), 0, "Drop must release");
    }

    #[test]
    fn release_happens_on_the_unwinding_path_too() {
        // The third of the three hangs the protocol replaced: a `Display` that panicked mid-format
        // leaked the lock permanently, and the panic hook's flush then hung the process.
        let _s = crate::drain_owner::test_serial();
        let caught = catch_unwind(AssertUnwindSafe(|| {
            let _g = acquire();
            panic!("deliberate");
        }));
        assert!(caught.is_err());
        assert_eq!(
            OUT_OWNER.load(Ordering::Relaxed),
            0,
            "a guard dropped by the unwinder must release; otherwise the next writer hangs"
        );
    }

    #[test]
    fn re_entry_is_detected_and_completes_instead_of_deadlocking() {
        // The panic-inside-the-sink case: the sink holds the lock, catches, and direct-writes.
        let _s = crate::drain_owner::test_serial();
        let before = out_reentrant();
        let outer = acquire();
        assert_eq!(outer.mode(), OutMode::Held);
        let inner = acquire();
        assert_eq!(inner.mode(), OutMode::Reentrant, "a self-deadlock here hangs the process");
        assert_eq!(out_reentrant(), before + 1);
        drop(inner);
        assert_ne!(
            OUT_OWNER.load(Ordering::Relaxed),
            0,
            "dropping the INNER guard must not release the outer one"
        );
        drop(outer);
        assert_eq!(OUT_OWNER.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_held_lock_is_stolen_rather_than_waited_on_forever() {
        let _s = crate::drain_owner::test_serial();
        let before = out_steals();

        // A foreign thread holds the lock for longer than the deadline.
        let holder = std::thread::spawn(|| {
            let _g = acquire();
            std::thread::sleep(ACQUIRE_DEADLINE + std::time::Duration::from_millis(80));
        });
        // Let the holder win the race for the lock.
        while OUT_OWNER.load(Ordering::Relaxed) == 0 {
            std::thread::yield_now();
        }

        let t0 = std::time::Instant::now();
        let g = acquire();
        let waited = t0.elapsed();

        assert_eq!(g.mode(), OutMode::Stolen, "the acquire must terminate by stealing");
        assert_eq!(out_steals(), before + 1, "a steal must be counted; it is a defect signal");
        assert!(
            waited >= ACQUIRE_DEADLINE,
            "the steal must not happen early: it is the bound that makes it acceptable"
        );
        assert!(waited < ACQUIRE_DEADLINE * 4, "the bound must actually bound: waited {waited:?}");

        drop(g);
        assert_ne!(
            OUT_OWNER.load(Ordering::Relaxed),
            0,
            "dropping a STOLEN guard must not free a lock another thread is still inside"
        );
        holder.join().expect("holder thread panicked");
        assert_eq!(OUT_OWNER.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn with_no_destination_configured_the_write_is_a_no_op() {
        // The flag-off property: no synchronous destination, so nothing is written and no lock is
        // taken. A caller cannot tell the difference from a successful write, and must not need to.
        let _s = crate::drain_owner::test_serial();
        // The precondition is SET, not asserted. `CONSOLE_ENABLED` is process-global and another
        // test in this binary legitimately turns it on; asserting the `.bss` default here would
        // make this a test of scheduling order rather than of the no-destination behaviour.
        let was = CONSOLE_ENABLED.load(Ordering::Relaxed);
        set_console_enabled(false);
        assert_eq!(write_oracle_line("boyko: ", "nothing should be written"), None);
        assert_eq!(OUT_OWNER.load(Ordering::Relaxed), 0, "a no-op must not acquire");
        set_console_enabled(was);
    }

    #[test]
    fn an_idle_write_never_waits_on_a_writer_that_stole_the_channel() {
        // A stealer does not own `OUT_OWNER`. Once the thread it stole from releases, the slot
        // reads free while the stealer is still blocked in its write, holding stderr's lock; only
        // `WRITES_IN_FLIGHT` still sees it (ci-fix-1, review r1 W1).
        let _s = crate::drain_owner::test_serial();
        let was = CONSOLE_ENABLED.load(Ordering::Relaxed);
        set_console_enabled(true);
        let held = acquire();
        assert_eq!(held.mode(), OutMode::Held, "fixture: the channel must start free");
        // Disarmed on every exit, a red included: a stall left armed would catch the next test's
        // sink thread.
        struct Disarm;
        impl Drop for Disarm {
            fn drop(&mut self) {
                test_stall::disarm();
            }
        }
        test_stall::arm();
        let _disarm = Disarm;
        // `test_stall` stalls only a thread named like the sink thread.
        let stealer = std::thread::Builder::new()
            .name("boyko-log-sink".into())
            .spawn(|| write_oracle_line("boyko-log-test: ", "a writer that stole the channel"))
            .expect("fixture: the stealer thread spawns");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !test_stall::entered() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(test_stall::entered(), "fixture: the stealer never reached its write");
        drop(held);

        let idle = write_oracle_line_if_idle("boyko-log-test: ", "must not be written");
        let gave_up = test_stall::gave_up();
        test_stall::disarm();
        let stolen = stealer.join().expect("fixture: the stealer thread panicked");
        set_console_enabled(was);

        assert_eq!(stolen, Some(OutMode::Stolen), "fixture: the stalled writer must have stolen");
        assert!(!gave_up, "the idle write waited on the stealer's stalled write until it gave up");
        assert_eq!(idle, None, "an idle write must refuse while a stealer is still writing");
    }
}
