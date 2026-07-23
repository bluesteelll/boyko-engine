//! Worker thread loop + idle-bitset primitives.
//!
//! Plan §4.3. The loop polls four sources in order
//! (`local_injector → own_deque → global_injector → sibling_steal`), then
//! backs off and parks. The post-`mark_idle` re-poll is load-bearing
//! against Race C (plan §13.4.1) — we keep it explicit in the code with a
//! comment, never collapse it into the pre-park branch.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use crossbeam_deque::{Steal, Worker};
use crossbeam_utils::Backoff;

use crate::sync::{AtomicU64, Ordering};
use crate::thread_pool::{PoolInner, TaskHandle};
use crate::tls;

/// Worker thread entry point. Runs until `inner.shutdown` is set and every
/// in-flight task has been drained.
pub(crate) fn worker_main(inner: Arc<PoolInner>, worker_id: u32, deque: Worker<TaskHandle>) {
    debug_assert!((worker_id as usize) < inner.workers.len());

    tls::set_current_worker_id(worker_id);
    // Deposit the worker-shared state pointer (decision E): the worker holds
    // `Arc<PoolInner>` for its whole life, so `Arc::as_ptr` is valid for the
    // duration of the deposit. The handle joins this worker before dropping
    // its own `Arc<PoolInner>`, so the pointee outlives this thread.
    tls::swap_active_pool(Arc::as_ptr(&inner));
    debug_assert!(
        !tls::active_pool_ptr().is_null(),
        "worker TLS pool deposit must be non-null"
    );

    // SplitMix64 seed for sibling steal randomization (plan §4.3 O1).
    let mut rng = XorShift64Star::new(splitmix64(worker_id as u64));

    'outer: loop {
        // Miri-only cooperative yield at the top of the worker loop so the
        // Miri scheduler can advance siblings even when this worker keeps
        // finding and running tasks via the `continue 'outer` arms (Phase 9.1
        // H2). Native: compiles to nothing — loop body byte-identical.
        #[cfg(miri)]
        std::thread::yield_now();

        // 1. Local injector — pushes that targeted this worker directly.
        if let Some(t) = pop_local_injector(inner.as_ref(), worker_id, &deque) {
            run_task(t);
            continue;
        }

        // 2. Own deque — own spawned tasks (FIFO ordering chosen at
        //    Worker::new_fifo() time).
        if let Some(t) = deque.pop() {
            run_task(t);
            continue;
        }

        // 3. Global injector — dispatcher-pushed tasks.
        if let Some(t) = pop_global_injector(inner.as_ref(), &deque) {
            run_task(t);
            continue;
        }

        // 4. Sibling steal.
        if let Some(t) = try_steal_random(inner.as_ref(), worker_id, &deque, &mut rng) {
            run_task(t);
            continue;
        }

        // 5. Backoff escalation, then mark_idle + re-poll + park.
        let backoff = Backoff::new();
        loop {
            // Miri-only cooperative yield: `Backoff::snooze()` (below) is a
            // pure-compute spin with no Miri yield point, and `pop_any` may keep
            // returning `None` for several iterations before this worker reaches
            // `mark_idle`+`park` (a real Miri yield point). Without this, the
            // bounded backoff spin still inflates Miri's interleaving search and
            // can starve siblings between rounds. Native: compiles to nothing.
            #[cfg(miri)]
            std::thread::yield_now();

            // Pre-mark_idle re-poll. Catches tasks that arrived while we
            // were spinning. (Race A in §13.4.1.)
            if let Some(t) = pop_any(inner.as_ref(), worker_id, &deque, &mut rng) {
                run_task(t);
                continue 'outer;
            }

            if backoff.is_completed() {
                mark_idle(&inner.idle, worker_id);

                // Post-mark_idle re-poll. Load-bearing against Race C: a
                // pusher reading idle==0 right before we set our bit must
                // not steal a wakeup that should have gone to us.
                if let Some(t) = pop_any(inner.as_ref(), worker_id, &deque, &mut rng) {
                    unmark_idle(&inner.idle, worker_id);
                    run_task(t);
                    continue 'outer;
                }

                // Shutdown check (Acquire) pairs with the Release-store
                // in `ThreadPool::drop`.
                if inner.shutdown.load(Ordering::Acquire) {
                    unmark_idle(&inner.idle, worker_id);
                    return;
                }

                std::thread::park();

                // After wakeup, clear our bit and loop back to poll.
                unmark_idle(&inner.idle, worker_id);
                continue 'outer;
            }

            backoff.snooze();
        }
    }
}

/// Run a task body with a process-abort guard for the fire-and-forget path.
///
/// Two distinct panic disciplines meet here:
///
/// - **Scope-spawned tasks** already wrap their body in `catch_unwind` inside
///   [`Scope::spawn`](crate::Scope::spawn), storing the payload for
///   `Scope::Drop` to re-raise on the joining thread. Such a body cannot
///   unwind past its own wrapper, so the `catch_unwind` below never observes
///   its panic and their propagation semantics are unchanged.
/// - **Fire-and-forget tasks** (`ThreadPool::spawn`) have no joiner to receive
///   a payload. A raw unwind here would tear down `worker_main`, permanently
///   shrinking the pool to `n-1` threads for the rest of the process lifetime
///   — a silent, unrecoverable degradation. We instead adopt rayon's `spawn`
///   policy explicitly: catch the unwind and abort the process. The catch
///   frame costs nothing on the hot path beyond a landing pad; the abort path
///   is `#[cold]`.
///
/// `pub(crate)` because the SAME discipline must cover the dispatcher's
/// work-stealing join (`Scope::drop` -> `join_workers_until_drained`), which
/// steals from the very same `injector_global`/sibling deques a worker does and
/// therefore can pick up a fire-and-forget task. Running one bare there would
/// let its unwind escape `Drop for Scope` and ABANDON the join — and that join
/// is the entire justification for the `'scope -> 'static` transmute, so the
/// caller's frame (and every stack borrow the still-running tasks hold) would be
/// popped out from under them: a use-after-free reachable from safe code.
/// 2026-07 audit finding.
#[inline]
pub(crate) fn run_task(t: TaskHandle) {
    if catch_unwind(AssertUnwindSafe(|| t.run())).is_err() {
        abort_on_task_panic();
    }
}

/// Abort the process after a fire-and-forget task panicked. Kept `#[cold]` and
/// out-of-line so the `run_task` landing pad stays small (I-cache). Matches
/// rayon's documented `spawn` behaviour: an unhandled task panic aborts.
#[cold]
#[inline(never)]
fn abort_on_task_panic() -> ! {
    // The payload has already been printed by the default panic hook (which
    // runs at the unwind's origin, before it reaches this catch). This line
    // records the abort decision so the cause is unambiguous in logs.
    eprintln!(
        "boyko_threadpool: a fire-and-forget task (ThreadPool::spawn) panicked; \
         aborting the process (no joiner can receive the payload)"
    );
    std::process::abort();
}

/// Poll all four sources once. Used by the backoff/park loop.
#[inline]
fn pop_any(
    inner: &PoolInner,
    worker_id: u32,
    local: &Worker<TaskHandle>,
    rng: &mut XorShift64Star,
) -> Option<TaskHandle> {
    if let Some(t) = pop_local_injector(inner, worker_id, local) {
        return Some(t);
    }
    if let Some(t) = local.pop() {
        return Some(t);
    }
    if let Some(t) = pop_global_injector(inner, local) {
        return Some(t);
    }
    try_steal_random(inner, worker_id, local, rng)
}

/// Drain a batch from this worker's local injector into its deque,
/// returning the first task.
#[inline]
fn pop_local_injector(
    inner: &PoolInner,
    worker_id: u32,
    local: &Worker<TaskHandle>,
) -> Option<TaskHandle> {
    let inj = &inner.injector_local[worker_id as usize];
    drain_one(|| inj.steal_batch_and_pop(local))
}

/// Drain a batch from the global injector into the local deque.
#[inline]
fn pop_global_injector(inner: &PoolInner, local: &Worker<TaskHandle>) -> Option<TaskHandle> {
    drain_one(|| inner.injector_global.steal_batch_and_pop(local))
}

/// Try to steal a batch from a random sibling. Returns the first stolen
/// task; the rest (if any) remain in the local deque.
fn try_steal_random(
    inner: &PoolInner,
    worker_id: u32,
    local: &Worker<TaskHandle>,
    rng: &mut XorShift64Star,
) -> Option<TaskHandle> {
    let n = inner.stealers.len();
    if n <= 1 {
        return None;
    }
    // Try up to n-1 siblings with a randomised starting offset. Order
    // doesn't matter for correctness — only for load balance.
    let start = (rng.next() as usize) % n;
    for k in 0..n {
        let idx = (start + k) % n;
        if idx as u32 == worker_id {
            continue;
        }
        let stealer = &inner.stealers[idx];
        if let Some(t) = drain_one(|| stealer.steal_batch_and_pop(local)) {
            return Some(t);
        }
    }
    None
}

/// Generic helper: invoke a `Steal`-returning closure, retrying on
/// `Retry`, returning `None` on `Empty`.
#[inline]
fn drain_one<F>(mut f: F) -> Option<TaskHandle>
where
    F: FnMut() -> Steal<TaskHandle>,
{
    loop {
        match f() {
            Steal::Success(t) => return Some(t),
            Steal::Empty => return None,
            Steal::Retry => {
                // Miri-only cooperative yield in this unbounded steal-retry
                // loop (Phase 9.1 H2). Byte-identical native: compiles away.
                #[cfg(miri)]
                std::thread::yield_now();
                continue;
            }
        }
    }
}

/// Mark worker `worker_id` as parked / about-to-park.
///
/// Release ordering publishes the bit so that any pusher that subsequently
/// `Acquire`-loads `idle` and sees the bit will know to call `unpark` —
/// pairs with `unpark_one_idle`'s Acquire/CAS sequence.
#[inline]
pub(crate) fn mark_idle(idle: &AtomicU64, worker_id: u32) {
    debug_assert!((worker_id as usize) < crate::thread_pool::MAX_WORKERS);
    let bit = 1u64 << worker_id;
    idle.fetch_or(bit, Ordering::Release);
}

/// Clear worker `worker_id`'s idle bit. Release matches subsequent loads
/// by the next pusher.
#[inline]
pub(crate) fn unmark_idle(idle: &AtomicU64, worker_id: u32) {
    debug_assert!((worker_id as usize) < crate::thread_pool::MAX_WORKERS);
    let bit = 1u64 << worker_id;
    idle.fetch_and(!bit, Ordering::Release);
}

/// Wake one parked worker, if any. Returns `true` on success.
///
/// Algorithm (plan §4.3, with the FIX-3 rotation):
/// 1. Acquire-load the idle bitset.
/// 2. If empty, no worker is parked → return false.
/// 3. Pick ONE set bit, starting the search from a rotating offset
///    (`wake_rotor`) so that successive wakes do not always target the
///    lowest-id parked worker (that systematic bias starves high-id workers
///    and was the fairness enabler behind the cross-pool misrouting hazard).
/// 4. CAS-clear that bit (AcqRel on success). On contention, restart.
/// 5. `unpark` the corresponding worker.
///
/// The claim is still a single-bit CAS: exactly one worker is claimed per
/// successful call (the property the loom M2/M2b models verify over their
/// transcribed copy — rotation changes *which* set bit, never that precisely
/// one is claimed). The CAS spin is bounded by the number of bits set;
/// contention is rare (the bit count equals the parked worker count, and at
/// most one wake-up per push is required).
pub(crate) fn unpark_one_idle(inner: &PoolInner) -> bool {
    // Relaxed: the rotor only spreads the wake target; no data is published
    // through it, so its inter-thread order is immaterial (correctness rests
    // entirely on the idle-bitset CAS below).
    let start = (inner.wake_rotor.fetch_add(1, Ordering::Relaxed) % 64) as u32;
    loop {
        let mask = inner.idle.load(Ordering::Acquire);
        if mask == 0 {
            return false;
        }
        // Pick the lowest set bit at or above `start`, wrapping around. This
        // is a rotate-right by `start` (bringing bit `start` to position 0),
        // lowest-bit pick, then rotate the index back.
        let rotated = mask.rotate_right(start);
        let low = rotated & rotated.wrapping_neg();
        let id = ((low.trailing_zeros() + start) % 64) as u64;
        let bit = 1u64 << id;
        let new = mask & !bit;
        match inner
            .idle
            .compare_exchange_weak(mask, new, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => {
                inner.workers[id as usize].thread.unpark();
                return true;
            }
            Err(_) => continue,
        }
    }
}

/// Push a task into the pool, targeting the calling thread's local
/// injector when it belongs to *this* pool (cache locality) and the global
/// injector otherwise. Wakes one idle worker.
///
/// The local-injector fast path is gated on pool identity, not on the bare
/// TLS worker id: `injector_local[wid]` is polled only by *this* pool's
/// worker `wid` (siblings reach it via the local-injector steal in stage 1.5,
/// but only within the same pool). A worker of pool A (id `wid`) that pushes
/// into pool B must NOT land the task in pool B's `injector_local[wid]` — that
/// slot's owner in pool B may be parked while `unpark_one_idle` wakes the
/// lowest idle bit, so the cross-pool task could sit undrained indefinitely.
/// We compare the TLS active-pool pointer against the target `inner` by
/// pointer equality; on any mismatch (cross-pool spawn, or an unattached
/// thread) we route to the global injector, which every worker polls.
pub(crate) fn push_task(inner: &PoolInner, task: TaskHandle) {
    let wid = tls::current_worker_id();
    // Same-pool fast path: the calling thread is a worker whose active pool is
    // exactly `inner`. Pointer equality is branch-cheap and the common case.
    let same_pool = std::ptr::eq(tls::active_pool_ptr(), inner);
    if same_pool && (wid as usize) < inner.injector_local.len() {
        inner.injector_local[wid as usize].push(task);
    } else {
        inner.injector_global.push(task);
    }
    unpark_one_idle(inner);
}

/// SplitMix64 mixer (Sebastiano Vigna, 2014). Used as a seed generator
/// for the per-worker [`XorShift64Star`] PRNG so that workers spawned
/// with adjacent ids don't start in adjacent PRNG states.
#[inline]
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut x = z;
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// XorShift64* — fast non-crypto PRNG used for sibling steal order
/// randomization. Seeded via SplitMix64 to mix adjacent worker ids.
pub(crate) struct XorShift64Star {
    state: u64,
}

impl XorShift64Star {
    #[inline]
    pub(crate) fn new(seed: u64) -> Self {
        // The XorShift64* state must be non-zero; replace 0 with a
        // canonical fallback constant.
        let state = if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        };
        Self { state }
    }

    #[inline]
    pub(crate) fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift_is_non_zero_and_progresses() {
        let mut r = XorShift64Star::new(0);
        let a = r.next();
        let b = r.next();
        assert_ne!(a, 0);
        assert_ne!(a, b, "PRNG advanced");
    }

    #[test]
    fn splitmix_disperses_adjacent_ids() {
        let a = splitmix64(0);
        let b = splitmix64(1);
        assert_ne!(a, b);
        // Hamming-distance sanity: 1-bit input should produce a high-bit-
        // count diff. Not a rigorous test, just a smell check.
        let diff = (a ^ b).count_ones();
        assert!(
            diff > 8,
            "splitmix64 should disperse adjacent inputs; got Hamming diff {diff}"
        );
    }

    #[test]
    fn idle_bitset_mark_unmark_round_trip() {
        let bitset = AtomicU64::new(0);
        mark_idle(&bitset, 3);
        assert_eq!(bitset.load(Ordering::Acquire) & (1 << 3), 1 << 3);
        unmark_idle(&bitset, 3);
        assert_eq!(bitset.load(Ordering::Acquire) & (1 << 3), 0);
    }
}
