//! `DRAIN_OWNER` — the single CAS'd token every consumer must hold.
//!
//! # Why the consumer role is a token and not a state
//!
//! The lane rings are SPSC. Their whole safety argument has two halves: exactly one thread writes
//! (the lane's owner, by construction) and **exactly one thread reads**. The write half is free —
//! the substrate hands each live thread a distinct lane index. The read half is not: there are
//! **four** consumers over the same bytes — the sink thread, a manual `drain()`, the scheduled
//! ECS drain, and the crash drainer — and nothing about their types keeps them apart.
//!
//! The predecessor inferred exclusivity from a *state*: it CAS'd `SINK_STATE` out of
//! `{Exited, NotBooted, Manual}` and called all three quiescent. **`Manual` is not quiescent.** It
//! means an arbitrary thread may be inside `drain()` *right now*, so a panic elsewhere started a
//! second consumer over the very bytes the first was staging. CASing the **role** removes the gap
//! between "a state that correlates with exclusivity" and "exclusivity".
//!
//! # This lock is the opposite of `OUT_LOCK`, and deliberately
//!
//! [`crate::sync_out`]'s lock **steals** after a bounded wait and **tolerates re-entry**, because
//! its failure mode is a hung process on the error-of-the-error path and an interleaved line is
//! the lesser harm. Neither is acceptable here:
//!
//! - **No stealing.** Stealing the drain role *creates* the second consumer it exists to prevent.
//!   A missed drain costs latency; two consumers over one ring cost a torn header and a `decode`
//!   through a corrupted site pointer.
//! - **No re-entry.** A thread that re-enters `drain()` is still two consumers over the same
//!   bytes, one frame apart. `OUT_LOCK` can permit it because writing a line twice is harmless.
//!
//! So this one **refuses**: `try_claim` returns `None` and the caller does nothing. Every one of
//! the four consumers is periodic or best-effort, so "not this time" is a complete answer.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// 0 = free; otherwise the opaque token of the thread currently draining.
static DRAIN_OWNER: AtomicU64 = AtomicU64::new(0);

/// Claims that found the role taken and did nothing. Reported by the census.
static DRAIN_CONTENDED: AtomicU32 = AtomicU32::new(0);

thread_local! {
    /// One byte per thread, used only for its address — unique among live threads, never zero.
    static TOKEN_ANCHOR: u8 = const { 0 };
}

#[inline]
fn my_token() -> u64 {
    TOKEN_ANCHOR.with(|a| std::ptr::from_ref(a) as u64)
}

/// Proof that the bearer is the process's only consumer.
///
/// The only way to obtain one is [`try_claim`], and the only way to release is `Drop` — including
/// the unwinder's. A drain that panics mid-stage must not strand the role, or every later drain
/// refuses forever and the rings fill in silence.
pub struct DrainToken {
    _private: (),
}

impl Drop for DrainToken {
    fn drop(&mut self) {
        DRAIN_OWNER.store(0, Ordering::Release);
    }
}

/// Claim the consumer role, or return `None`.
///
/// `Acquire` on success pairs with the previous holder's `Release`, so a new drainer observes
/// everything the last one staged — including the `read` cursors it advanced.
///
/// Returns `None` when another thread holds the role **and also when this thread already does**.
/// The second case is not an oversight: a re-entrant drain is two consumers over one ring, one
/// stack frame apart, which is precisely what the token prevents between threads.
#[must_use]
pub fn try_claim() -> Option<DrainToken> {
    if DRAIN_OWNER
        .compare_exchange(0, my_token(), Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        Some(DrainToken { _private: () })
    } else {
        DRAIN_CONTENDED.fetch_add(1, Ordering::Relaxed);
        None
    }
}

/// Whether any thread currently holds the role. For assertions and the census; **never** as a
/// substitute for holding the token, which is the mistake the predecessor's `SINK_STATE` check was.
#[must_use]
pub fn is_claimed() -> bool {
    DRAIN_OWNER.load(Ordering::Relaxed) != 0
}

/// Claims that found the role taken. A steadily rising count means two consumers are configured
/// and one of them is doing nothing — a configuration defect, not a data race.
#[must_use]
pub fn drain_contended() -> u32 {
    DRAIN_CONTENDED.load(Ordering::Relaxed)
}

/// The ONE serialization point for tests that claim the drain role.
///
/// **There is exactly one drain token in the process, so there must be exactly one test lock over
/// it.** MEASURED: an earlier attempt had `drain_owner`'s tests and `lane`'s ring tests each
/// holding their *own* mutex, which serializes each module against itself and neither against the
/// other — six tests failed in a full run and every one of them passed alone. Two independent
/// serialization domains over one global resource is not serialization.
#[cfg(test)]
#[allow(clippy::disallowed_types)]
pub(crate) static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the process-wide test lock, ignoring poisoning — a test that panicked while holding it has
/// already reported, and refusing every later test would turn one failure into a cascade.
#[cfg(test)]
pub(crate) fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    TEST_SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;


    #[test]
    fn the_role_is_taken_once_and_released_by_drop() {
        let _s = super::test_serial();
        {
            let _t = try_claim().expect("a free role must be claimable");
            assert!(is_claimed());
        }
        assert!(!is_claimed(), "Drop must release, or every later drain refuses forever");
    }

    #[test]
    fn a_second_claim_refuses_rather_than_stealing() {
        // The sharp contrast with OUT_LOCK. Stealing here would CREATE the second consumer the
        // token exists to prevent, so "not this time" is the correct and complete answer.
        let _s = super::test_serial();
        let held = try_claim().expect("free");
        let before = drain_contended();

        let refused = std::thread::spawn(|| try_claim().is_none())
            .join()
            .expect("probe thread panicked");
        assert!(refused, "a second thread must be refused, never allowed to steal");
        assert_eq!(drain_contended(), before + 1, "a refusal is counted");
        drop(held);
    }

    #[test]
    fn re_entry_on_one_thread_is_also_refused() {
        // Two consumers over one ring one stack frame apart is still two consumers. `OUT_LOCK`
        // permits re-entry because writing a line twice is harmless; staging the same bytes twice
        // is not.
        let _s = super::test_serial();
        let outer = try_claim().expect("free");
        assert!(try_claim().is_none(), "a re-entrant drain must be refused");
        drop(outer);
        assert!(!is_claimed());
    }

    #[test]
    fn release_happens_on_the_unwinding_path() {
        let _s = super::test_serial();
        let caught = catch_unwind(AssertUnwindSafe(|| {
            let _t = try_claim().expect("free");
            panic!("a drain that panics mid-stage");
        }));
        assert!(caught.is_err());
        assert!(
            !is_claimed(),
            "a stranded role means the rings fill in silence -- the failure mode with no symptom"
        );
    }

    #[test]
    fn exactly_one_of_many_racing_claimants_wins_at_a_time() {
        // The property the SPSC read side rests on, asserted against a real race rather than
        // argued. A `SINK_STATE`-shaped check passes this test only by accident of timing; a CAS
        // on the role passes it by construction.
        let _s = super::test_serial();
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let mut hs = Vec::new();
        for _ in 0..8 {
            let (c, m) = (Arc::clone(&concurrent), Arc::clone(&max_seen));
            hs.push(std::thread::spawn(move || {
                for _ in 0..2000 {
                    if let Some(_t) = try_claim() {
                        let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                        m.fetch_max(n, Ordering::SeqCst);
                        std::hint::spin_loop();
                        c.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            }));
        }
        for h in hs {
            h.join().expect("claimant thread panicked");
        }
        assert_eq!(
            max_seen.load(Ordering::SeqCst),
            1,
            "two threads held the drain role at once; the SPSC read side is unsound"
        );
        assert!(!is_claimed());
    }
}
