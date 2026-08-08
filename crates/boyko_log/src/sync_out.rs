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
    let g = acquire();
    let mut err = std::io::stderr();
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
}
