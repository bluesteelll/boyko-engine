//! Worker thread loop + idle-bitset primitives.
//!
//! Plan §4.3. The loop polls four sources in order
//! (`local_injector → own_deque → global_injector → sibling_steal`), then
//! backs off and parks. The post-`mark_idle` re-poll is load-bearing
//! against Race C (plan §13.4.1) — we keep it explicit in the code with a
//! comment, never collapse it into the pre-park branch.
//!
//! KE16 (this paragraph and every `ke16-*` switch below are deleted with the
//! features): the four-source order above is the DEFAULT / `ke16-a2` /
//! `ke16-a5` build. Under `ke16-a1` / `ke16-a1-fifo` / `ke16-a3` nothing feeds
//! `injector_local`, so the first stage is compiled out with the queue it
//! drained and the loop polls THREE sources (`own_deque → global_injector →
//! sibling_steal`); the same deletion takes stage 1 out of `pop_any` and
//! leaves `pop_local_injector` without a caller, so it is cfg'd out too. The
//! A axis of the tournament is one function at the bottom of this file,
//! `push_task`, which decides where a spawn LANDS; these stages are the
//! matching read side.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use crossbeam_deque::{Steal, Worker};
use crossbeam_utils::Backoff;

use crate::sync::{AtomicU64, Ordering};
use crate::task::Task;
use crate::thread_pool::PoolInner;
use crate::tls;

/// Worker thread entry point. Runs until `inner.shutdown` is set and every
/// in-flight task has been drained.
pub(crate) fn worker_main(inner: Arc<PoolInner>, worker_id: u32, deque: Worker<Task>) {
    debug_assert!((worker_id as usize) < inner.workers.len());

    tls::set_current_worker_id(worker_id);
    // D1 lane write site 1 of 3, deliberately co-located with the pool's own
    // TLS write: the two slots hold the same fact for two different crates, and
    // co-location is what makes an edit that moves one and not the other a
    // two-line diff rather than a silent divergence. The cast is exact —
    // `worker_id < MAX_WORKERS` (clamped at build) and `LANE_WORKER_MAX ==
    // MAX_WORKERS` is a const assert in `thread_pool.rs` — so a worker can
    // never land on the dispatcher's lane.
    boyko_diag::lane::set_lane(worker_id as u16);
    // Deposit the worker-shared state pointer (decision E): the worker holds
    // `Arc<PoolInner>` for its whole life, so `Arc::as_ptr` is valid for the
    // duration of the deposit. The handle joins this worker before dropping
    // its own `Arc<PoolInner>`, so the pointee outlives this thread.
    tls::swap_active_pool(Arc::as_ptr(&inner));
    debug_assert!(
        !tls::active_pool_ptr().is_null(),
        "worker TLS pool deposit must be non-null"
    );
    // === KE16 A switch: ke16-a1 / ke16-a1-fifo ===
    // Publish the deque to this thread's `push_task`. The guard clears the slot
    // on every exit path; it is declared AFTER `deque` (a parameter) so it drops
    // BEFORE `deque` does. The pointer is a raw borrow of the place — no
    // reference to `deque` is created here (discipline D5, `KE16-DESIGN-A.md`
    // §1.7 — not the diagnostics rung named twenty lines above).
    #[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
    let _deque_deposit = tls::WorkerDequeDeposit::new(Arc::as_ptr(&inner), &raw const deque);

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
        // === KE16 A switch: ke16-a1 / ke16-a1-fifo / ke16-a3 ===
        // Those three arms never feed `injector_local`, and an empty `Injector`
        // probe is not free (two Acquire loads and a SeqCst fence per call), so
        // the stage is compiled out rather than left to poll a queue that can
        // never hold anything.
        #[cfg(not(any(feature = "ke16-a1", feature = "ke16-a1-fifo", feature = "ke16-a3")))]
        if let Some(t) = pop_local_injector(inner.as_ref(), worker_id, &deque) {
            run_task(t);
            continue;
        }

        // 2. Own deque — own spawned tasks, plus whatever this worker stole
        //    (end discipline chosen at construction: FIFO, or LIFO under
        //    `ke16-a1`).
        if let Some(t) = deque.pop() {
            run_task(t);
            continue;
        }

        // 3. Global injector — dispatcher-pushed tasks.
        if let Some(t) = pop_global_injector(inner.as_ref(), worker_id, &deque) {
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
/// - **Scope-spawned tasks** are already caught one frame in, by the
///   `catch_unwind` inside `task::run_scoped`, which stores the payload for
///   `Scope::Drop` to re-raise on the joining thread. Such a body cannot unwind
///   past its own run function, so the `catch_unwind` below never observes its
///   panic and their propagation semantics are unchanged.
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
pub(crate) fn run_task(t: Task) {
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
    // records the abort DECISION so the cause is unambiguous in logs.
    boyko_log::error!(
        boyko_log::Threadpool,
        boyko_log::codes::E0201,
        "a fire-and-forget task (ThreadPool::spawn) panicked; aborting the process \
         (no joiner can receive the payload)"
    );
    // BEFORE `abort`, because `abort` runs no destructor, no `atexit` handler and no sink
    // shutdown: a record still sitting in its lane ring at that instant dies with the process.
    if boyko_log::lifecycle::flush() == boyko_log::lifecycle::FlushResult::NoConsumer {
        // Diagnostics were never enabled, so the record above is in a ring nothing will ever
        // read and the abort decision would be INVISIBLE -- strictly worse than the
        // unconditional `eprintln!` this replaced. This is the only configuration in which the
        // pool writes for itself, and it is the configuration in which nothing else will.
        // L8c owes it a row in `print_allowlist.txt` naming exactly that reason.
        eprintln!(
            "boyko-E0201: a fire-and-forget task (ThreadPool::spawn) panicked; \
             aborting the process (no joiner can receive the payload)"
        );
    }
    std::process::abort();
}

/// Poll every source once. Used by the backoff/park loop.
///
/// `pub(crate)` for the KE16 B1 worker joiner: its post-`mark_idle` re-poll is
/// `worker_main`'s own scan, not a second implementation of it
/// (`KE16-DESIGN-B.md` §2.2 step 4).
#[inline]
pub(crate) fn pop_any(
    inner: &PoolInner,
    worker_id: u32,
    local: &Worker<Task>,
    rng: &mut XorShift64Star,
) -> Option<Task> {
    // === KE16 A switch: ke16-a1 / ke16-a1-fifo / ke16-a3 === (stage 1, as in
    // `worker_main`: an unfed queue is not probed)
    #[cfg(not(any(feature = "ke16-a1", feature = "ke16-a1-fifo", feature = "ke16-a3")))]
    if let Some(t) = pop_local_injector(inner, worker_id, local) {
        return Some(t);
    }
    if let Some(t) = local.pop() {
        return Some(t);
    }
    if let Some(t) = pop_global_injector(inner, worker_id, local) {
        return Some(t);
    }
    try_steal_random(inner, worker_id, local, rng)
}

/// Drain a batch from this worker's local injector into its deque,
/// returning the first task.
// === KE16 A switch: ke16-a1 / ke16-a1-fifo / ke16-a3 === (no arm feeds
// `injector_local`, so no arm polls it and the helper has no caller left)
#[cfg(not(any(feature = "ke16-a1", feature = "ke16-a1-fifo", feature = "ke16-a3")))]
#[inline]
fn pop_local_injector(inner: &PoolInner, worker_id: u32, local: &Worker<Task>) -> Option<Task> {
    let inj = &inner.injector_local[worker_id as usize];
    drain_one(|| inj.steal_batch_and_pop(local))
}

/// Drain a batch from the global injector into the local deque.
///
/// `pub(crate)` for the KE16 B1 worker joiner, which reaches the injector
/// through this helper so the batch lands in its own REGISTERED deque and the
/// residue stays stealable (`KE16-DESIGN-B.md` §2.2 step 2).
///
/// `worker_id` names the caller's own lane and is used only by W-b's residue
/// cascade ([`wake_after_residue`]), which must never claim the caller itself.
#[inline]
pub(crate) fn pop_global_injector(
    inner: &PoolInner,
    worker_id: u32,
    local: &Worker<Task>,
) -> Option<Task> {
    let popped = drain_one(|| inner.injector_global.steal_batch_and_pop(local));
    if popped.is_some() {
        wake_after_residue(inner, worker_id, local);
    }
    popped
}

/// Try to steal a batch from a random sibling. Returns the first stolen
/// task; the rest (if any) remain in the local deque.
///
/// KE16 (deleted with the features): under `ke16-a2` — and hence under
/// `ke16-a5`, which implies it — each visited sibling is probed TWICE, its
/// registered deque first and then its `injector_local` slot, because those
/// arms leave a worker's own spawns in that slot and this scan is the only
/// thing that makes them reachable by anyone else (A2's whole content). Self is
/// skipped for both probes: a worker drains its own slot at stage 1 of its loop.
///
/// `pub(crate)` for the KE16 B1 worker joiner, whose sweep IS this one (App-3,
/// `KE16-DESIGN-B.md` §2.2 step 3): random start, self skipped by the lane's own
/// `wid`, batch into the caller's registered deque — replacing the fixed,
/// self-including `0..n` sweep the B0 joiner runs.
///
/// A successful batch takes W-b's residue cascade ([`wake_after_residue`]):
/// whatever this thief did not take is now in its OWN registered deque, a 0→k
/// transition that the ≤1 push gate would have woken somebody for.
pub(crate) fn try_steal_random(
    inner: &PoolInner,
    worker_id: u32,
    local: &Worker<Task>,
    rng: &mut XorShift64Star,
) -> Option<Task> {
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
            wake_after_residue(inner, worker_id, local);
            return Some(t);
        }
        // === KE16 A switch: ke16-a2 === (shared with ke16-a5, which implies it)
        #[cfg(feature = "ke16-a2")]
        {
            let inj = &inner.injector_local[idx];
            if let Some(t) = drain_one(|| inj.steal_batch_and_pop(local)) {
                wake_after_residue(inner, worker_id, local);
                return Some(t);
            }
        }
    }
    None
}

/// Generic helper: invoke a `Steal`-returning closure, retrying on
/// `Retry`, returning `None` on `Empty`.
#[inline]
fn drain_one<F>(mut f: F) -> Option<Task>
where
    F: FnMut() -> Steal<Task>,
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

/// Clear worker `worker_id`'s idle bit and return the idle word as it stood
/// BEFORE the clear. Release matches subsequent loads by the next pusher.
///
/// The returned word answers one question: **was this bit still ours?** A wake
/// decision that claimed this lane CAS-cleared the bit ([`claim_one_idle`]) and
/// spent the pool's single wake on this thread, so a caller that reads its own
/// bit back as CLEAR owes that wake a lane which will actually scan. Reading it
/// off the RMW rather than off a preceding load is what makes the answer exact:
/// the value is this thread's immediate predecessor in the location's
/// modification order (an atomicity property, independent of the ordering
/// argument), whereas a separate load leaves the claim free to land between the
/// load and the clear.
///
/// `worker_main` ignores it and is right to: every one of its post-`mark_idle`
/// exits either runs a task or `continue 'outer`s into a full re-poll, so a
/// claim is always honoured by a scan. The KE16 B joiner cannot say that: it has
/// exits that return into a task body without scanning. It therefore reads this
/// word at EVERY post-`mark_idle` exit — the returning ones hand the claim to a
/// sibling, the ones that run a task carry it to the next scope check, which
/// scans or hands it on (`KE16-DESIGN-B.md` §2.2, rule B1-P; the round-4
/// review's blocking item 1, and the round-5 review's item 1 for the third
/// exit).
#[inline]
pub(crate) fn unmark_idle(idle: &AtomicU64, worker_id: u32) -> u64 {
    debug_assert!((worker_id as usize) < crate::thread_pool::MAX_WORKERS);
    let bit = 1u64 << worker_id;
    idle.fetch_and(!bit, Ordering::Release)
}

/// The producer-side StoreLoad barrier of the wake protocol (KE16 W-a).
///
/// One `fence(SeqCst)` — a local `mfence` on x86, no cache line touched —
/// issued between publishing a task and reading the idle mask. It is what makes
/// the store-buffer litmus of Race C formal for a PLAIN-STORE transport: the
/// producer does `publish; fence; load idle`, the parking worker does
/// `fetch_or; fence (supplied by the crossbeam steal path); re-poll`, and the
/// fenced SB shape forbids both loads reading stale — the outcome "worker parked
/// with work visible and no wake issued". Before W-a the barrier was supplied by
/// accident (the `Injector::push` CAS and the `wake_rotor` `lock xadd`, neither
/// placed for the purpose). The KE16 A1 arms push with a plain store and would
/// have no barrier at all without this one.
///
/// Routed through `crate::sync` so the loom M2 model drives THIS function
/// (`loom_exports::publish_fence`) rather than a model-local copy.
#[inline]
pub(crate) fn publish_fence() {
    crate::sync::fence(Ordering::SeqCst);
}

/// Wake one parked worker, if any. Returns `true` on success.
///
/// Equivalent to [`unpark_one_idle_excluding`] with an empty exclusion mask;
/// see it for the algorithm.
#[inline]
pub(crate) fn unpark_one_idle(inner: &PoolInner) -> bool {
    unpark_one_idle_excluding(inner, 0)
}

/// Wake one parked worker whose bit is not in `exclude`. Returns `true` on
/// success.
///
/// Algorithm (plan §4.3 with the FIX-3 rotation, KE16 W-a's fenced prologue):
/// 1. [`publish_fence`] — the producer's StoreLoad barrier, ahead of the load.
/// 2. Acquire-load the idle bitset WHOLE (it is the CAS's expected operand)
///    and test it against `exclude` for a claimable bit.
/// 3. If none remains, no claimable worker is parked → return false. A busy
///    pool therefore pays one local fence and one load, and NO shared RMW: the
///    rotor sits behind the mask load, so it is touched only when a wake is
///    actually issued (it advances once per wake instead of once per push — a
///    different, equally fair rotation).
/// 4. Pick ONE set bit starting from the rotating offset (`wake_rotor`), so
///    successive wakes do not always target the lowest-id parked worker (that
///    systematic bias starves high-id workers and was the fairness enabler
///    behind the cross-pool misrouting hazard), and CAS-clear it
///    ([`claim_one_idle`]). On a lost CAS, re-read the mask and decide again.
/// 5. `unpark` the claimed worker.
///
/// `exclude` masks bits the caller must never claim. Under `ke16-b1` /
/// `ke16-b3` three production sites pass a non-zero mask, all of them the B
/// joiner's own `self_bit` (`scope.rs`, `join_on_worker`), in two shapes:
///
/// - the PRE-PARK wake, issued with the bit still SET. The exclusion is
///   load-bearing there: this thread is about to sleep on the very work it is
///   offering, so a claim of its own bit would spend the pool's single wake on
///   the thread that issued it.
/// - the two HANDOFF wakes of rule B1-P — a joiner whose scope drained while it
///   carried a claim it cannot discharge — issued AFTER `unmark_idle` has
///   already cleared the bit. The mask is a no-op on the word as it stands and
///   is passed for uniformity: every wake from that function means "reach a lane
///   OTHER than this one", and no site there has to prove locally that its own
///   bit is currently clear.
///
/// The default build has no such caller, and the unit test at the foot of this
/// file drives the non-zero shape there. Under `ke16-w-gate` there is a fourth
/// site of the first shape, and it is the one the design calls load-bearing:
/// W-b's thief-residue cascade ([`wake_after_residue`]) — a thief that finds
/// residue inside its own post-`mark_idle` re-poll hands the cascade to a
/// SIBLING rather than waking itself, or the O(log W) chain breaks at hop one.
///
/// The claim is still a single-bit CAS: exactly one worker is claimed per
/// successful call (the property the loom M2/M2b models verify over their
/// transcribed copy — rotation changes *which* set bit, never that precisely
/// one is claimed). A retry happens only after a LOST CAS — that is, after
/// another thread's own successful claim or `unmark_idle` changed the word — so
/// the loop makes progress against a bounded number of concurrent claimers and
/// an excluded bit, which no retry can ever consume, cannot drive it. Contention
/// is rare anyway (the bit count equals the parked worker count, and at most one
/// wake-up per push is required).
pub(crate) fn unpark_one_idle_excluding(inner: &PoolInner, exclude: u64) -> bool {
    publish_fence();
    let mut observed = inner.idle.load(Ordering::Acquire);
    if observed & !exclude == 0 {
        return false;
    }
    // Relaxed: the rotor only spreads the wake target; no data is published
    // through it, so its inter-thread order is immaterial (correctness rests
    // entirely on the idle-bitset CAS below).
    let start = (inner.wake_rotor.fetch_add(1, Ordering::Relaxed) % 64) as u32;
    loop {
        match claim_one_idle(inner, observed, exclude, start) {
            Some(id) => {
                inner.workers[id as usize].thread.unpark();
                return true;
            }
            None => {
                observed = inner.idle.load(Ordering::Acquire);
                if observed & !exclude == 0 {
                    return false;
                }
            }
        }
    }
}

/// The claim core: ONE attempt to take a claimable parked worker out of the
/// idle word `observed`.
///
/// `observed` is the whole idle word as the caller last loaded it — it is the
/// CAS's EXPECTED operand, so it must be the value the caller actually read,
/// never a masked derivative of it. `exclude` names bits the caller may not
/// take; the candidate is the lowest set bit of `observed & !exclude` at or
/// above `start` (wrapping) — a rotate-right by `start` bringing bit `start` to
/// position 0, a lowest-bit pick, then the index rotated back — and the attempt
/// is ONE `compare_exchange_weak(observed, observed & !bit, AcqRel / Acquire)`.
/// `Some(id)` means the bit was claimed and the caller MUST unpark worker `id`.
/// `None` means NOTHING was claimed and the caller must re-read the idle word
/// and decide again; it covers both ways that happens — the CAS lost, or
/// `observed & !exclude` held no claimable bit in the first place. Never loops,
/// never unparks.
///
/// **The empty-candidate input is answered, not asserted.** A total contract is
/// the point: the natural transcription of W-f ("wake `min(n, popcount(idle))`
/// workers, one claim + unpark each") is a bounded loop over a mask other
/// threads are concurrently clearing, and it reaches an exhausted mask on any
/// lost CAS with iterations left; A5 claims from a mask read earlier still. A
/// core that answered those with `Some(id)` — which a CAS of `observed` against
/// an unchanged `observed` does trivially — would charge an unpark to a worker
/// whose bit was never set: an out-of-bounds `inner.workers[id]` on a pool
/// smaller than `id`, or a silently wasted wake on a larger one. There is
/// deliberately no `debug_assert` on this input: it would make the same value
/// both a documented `None` and a debug-mode panic, and it would fire on those
/// two callers, which reach it legitimately rather than by a bug.
///
/// **Why the two masks are separate parameters.** They matter only to a caller
/// that excludes something: the KE16 B joiner's pre-park wake, built under
/// `ke16-b1` / `ke16-b3`, and W-b's thief-residue cascade
/// ([`wake_after_residue`]), built under `ke16-w-gate`. Folding them — CASing against `observed & !exclude` — makes the CAS
/// unsatisfiable whenever an excluded bit is actually set, because then that
/// value is not the word in memory; the caller's retry loop re-derives the same
/// value and spins forever. Both of those callers issue their wake with their
/// own bit SET (the joiner between `mark_idle` and its park; the cascade from
/// inside the post-`mark_idle` re-poll), so the folded form would hang exactly
/// where the parameter is needed. See [`unpark_one_idle_excluding`]'s caller loop and the deviation
/// recorded against `KE16-DESIGN-W.md` §1.1.
///
/// One signature for every claimer of the tournament (the base wake decision
/// and, when they land, the A5 placement and the W-f fan-out), so the loom
/// transcriptions have a single shape to mirror.
#[inline]
pub(crate) fn claim_one_idle(
    inner: &PoolInner,
    observed: u64,
    exclude: u64,
    start: u32,
) -> Option<u32> {
    let candidates = observed & !exclude;
    if candidates == 0 {
        return None;
    }
    let rotated = candidates.rotate_right(start);
    let low = rotated.isolate_lowest_one();
    let id = (low.trailing_zeros() + start) % 64;
    let new = observed & !(1u64 << id);
    match inner
        .idle
        .compare_exchange_weak(observed, new, Ordering::AcqRel, Ordering::Acquire)
    {
        Ok(_) => Some(id),
        Err(_) => None,
    }
}

/// The wake decision after a push, shared by every placement arm.
///
/// `pre_len` is the destination queue's length read immediately before the
/// push; it is read only where the length is free or where a gate consumes it
/// (see [`LenProbe`]). Without `ke16-w-gate` the wake is unconditional —
/// today's behaviour. This is the ONE W switch point for the PUSH side: no
/// placement arm carries a wake rule of its own, and the StoreLoad barrier lives
/// one level down in [`unpark_one_idle_excluding`], so a gated-out push pays
/// neither the fence nor the mask load.
///
/// KE16 W-b (`KE16-DESIGN-W.md` §2.1) is the ≤1 rule here plus the thief-residue
/// cascade in [`wake_after_residue`] — the second is where the candidate's
/// O(log W) fan-out lives, and the two are one mechanism: the gate makes a wave
/// silent after its second push, the cascade is what re-fans it out. Why ≤1 and
/// not "was empty": a queue holding exactly one task when the length is read can
/// be emptied by a thief before the push lands, and the wake on the 1→2
/// transition covers that thief's departure. This is JDK 8
/// `ForkJoinPool.WorkQueue.push`'s `n = s − b; if (n <= 1) signalWork`.
#[inline]
pub(crate) fn wake_after_push(inner: &PoolInner, pre_len: usize) {
    if wake_is_due(pre_len) {
        unpark_one_idle(inner);
    }
}

/// The W gate's rule, and the ONE place `ke16-w-gate` is spelled on the push
/// side: is a wake due after a push whose destination held `pre_len` tasks?
///
/// Every push-side wake site answers this question and none of them re-decides
/// it: [`wake_after_push`] (one wake) and [`wake_after_push_up_to`] (W-f's
/// fan-out) differ in HOW WIDE the wake is, never in WHETHER one is owed. That
/// is what keeps `grep 'feature = "ke16-w-gate"'` on the push side down to this
/// body, so the removal step has one site to delete, and what keeps a `c1f`
/// row's delta over `c1` to the fan-out COUNT alone: were the fan-out
/// gate-exempt, that delta would also contain "the ≤1 rule lifted on the wave's
/// first push", and the cell Step W/C decides W-f on would be measuring two
/// mechanisms (`KE16-DESIGN.md` §4, "the W gate is ONE site", against
/// `KE16-DESIGN-W.md` §4.1's literal `wake_up_to(inner, n)` — the deviation is
/// recorded there).
// === KE16 W switch: ke16-w-gate ===
#[inline]
pub(crate) fn wake_is_due(pre_len: usize) -> bool {
    #[cfg(feature = "ke16-w-gate")]
    {
        pre_len <= 1
    }
    #[cfg(not(feature = "ke16-w-gate"))]
    {
        let _ = pre_len;
        true
    }
}

/// The wake decision after a BATCH STEAL: the thief-residue cascade, W-b's
/// other half (`KE16-DESIGN-W.md` §2.1).
///
/// A thief that batch-stole into its own registered deque and still holds
/// residue there (`!local.is_empty()`) has just taken that deque from 0 to k —
/// the same empty→non-empty transition [`wake_after_push`]'s ≤1 rule wakes for,
/// on a queue no push touched. It therefore wakes ONE worker, and that is what
/// turns a wave whose spawner fell silent after two pushes into FJP's activation
/// chain: each activated worker activates the next, O(log W) to full activation.
/// The residue is published by a plain store (crossbeam `deque.rs:1160-1170`),
/// so this wake needs the fenced prologue exactly like a push's.
///
/// **The self-exclusion is load-bearing, not hygiene.** Both call sites are
/// reachable from `worker_main`'s post-`mark_idle` re-poll — and, under
/// `ke16-b1` / `ke16-b3`, from the B1-P joiner's — where the CALLER's own idle
/// bit is still set. An unmasked claim there can pick the caller (which bit is
/// picked depends on the rotor), leaving a stale token on its own parker and
/// W−1 parked siblings unwoken: the chain broken at hop one, which is the whole
/// of W-b's fan-out.
///
/// Without `ke16-w-gate` there is no cascade — today's behaviour, where every
/// push wakes and no steal does.
#[inline]
pub(crate) fn wake_after_residue(inner: &PoolInner, worker_id: u32, local: &Worker<Task>) {
    // === KE16 W switch: ke16-w-gate === (the cascade half of the one gate above)
    #[cfg(feature = "ke16-w-gate")]
    {
        debug_assert!((worker_id as usize) < crate::thread_pool::MAX_WORKERS);
        if !local.is_empty() {
            unpark_one_idle_excluding(inner, 1u64 << worker_id);
        }
    }
    #[cfg(not(feature = "ke16-w-gate"))]
    let _ = (inner, worker_id, local);
}

/// The wake decision after a BATCH's first push under W-f: [`wake_after_push`]'s
/// rule, [`wake_up_to`]'s width.
///
/// The fan-out is **not** gate-exempt. It asks [`wake_is_due`] the same question
/// every other push-side site asks, and only then fans out; a first push that
/// lands in a queue which already held ≥ 2 tasks wakes nobody under
/// `ke16-w-gate`, exactly as it would without `ke16-w-fanout`. Two reasons, and
/// they are the ones that decide the arm rather than tidiness:
///
/// - **The `c1f` row must be the fan-out COUNT and nothing else.** Step W/C keeps
///   W-f only if a cell improves beyond 2× the band, and it reads that cell as
///   `wgc+c1f` minus `wgc+c1` (`KE16-DESIGN.md` §3 step 4). A gate-exempt
///   fan-out would put "the ≤1 rule lifted on the wave's first push" into the
///   same delta, and the row deciding W-f would mix two mechanisms.
/// - **The probe would be paid and thrown away.** The first push is
///   `push_task_no_wake`, i.e. `ProbeLen`, so under `ke16-w-gate` it issues
///   `Injector::len` (a three-`SeqCst`-load retry loop) or `Worker::len` (a
///   `SeqCst` load of the contended `front`). Discarding `pre_len` here would
///   charge that probe to the one push under measurement — the very cost
///   [`NoProbe`] exists to remove.
///
/// This departs from `KE16-DESIGN-W.md` §4.1's literal spelling ("under
/// `ke16-w-fanout`: `wake_up_to(inner, n)`") in favour of `KE16-DESIGN.md` §4's
/// "the W gate is ONE site"; the two clauses conflict and the reviewable,
/// separable reading is taken.
///
/// Answers the unpark count for the tests; the production caller discards it.
// === KE16 W switch: ke16-w-fanout ===
#[cfg(feature = "ke16-w-fanout")]
#[inline]
pub(crate) fn wake_after_push_up_to(inner: &PoolInner, pre_len: usize, k: usize) -> usize {
    if !wake_is_due(pre_len) {
        return 0;
    }
    wake_up_to(inner, k)
}

/// KE16 W-f — the fan-out wake of a BATCH's first push: wake
/// `min(k, popcount(idle))` workers instead of one (`KE16-DESIGN-W.md` §5).
///
/// One [`publish_fence`] for the whole fan-out — the barrier orders this
/// thread's publication of the first task against every mask load below, and
/// the pushes that follow the fan-out are ordered by the woken workers' own
/// steal-path fences — then one ENTRY SNAPSHOT of the idle word, and at most `k`
/// rounds that each claim one still-unclaimed bit OF THAT SNAPSHOT and unpark
/// it. Two numbers bound the call: the round count is `k`, and the UNPARK count
/// is `popcount` of the snapshot, because `unclaimed` only ever loses bits.
///
/// **Why the candidate set is the snapshot and not a fresh mask per round.** A
/// worker this fan-out has already woken can find the queue holding only the
/// wave's first task, snooze, `mark_idle` and park again while the same call is
/// still running — its bit is back in the word within microseconds. Claiming
/// from a re-read mask would take that bit a second time, leaving the unpark
/// count bounded by `k` ALONE — the same worker woken over and over, up to `k`
/// ~1–5 µs syscalls on the spawner's critical path against the ≤ W the design
/// prices. The waves this tree emits make that a factor of `k/W`, not a rounding
/// error: the largest production wave is the physics colored solve's
/// `num_threads() * CHUNKS_PER_WORKER` = 96 at W = 16
/// (`boyko_physics/src/solver/colored.rs`, and the soft solve mirrors it), while
/// `par_iter` over 65536 rows emits 16 — one chunk per worker under the default
/// `BatchingStrategy`. Confining the candidates to the entry snapshot is
/// what makes `min(k, popcount(idle))` the arm's actual behaviour rather than
/// its advertised one — see the deviation recorded against `KE16-DESIGN-W.md`
/// §5, whose step list says "re-read the mask each round" while its own cost
/// sentence and the index row (`KE16-DESIGN.md` §2, W-f) state the popcount
/// bound.
///
/// The idle word is still re-read once per round, but only as the CAS's
/// EXPECTED operand — [`claim_one_idle`] CASes the whole word, so a stale
/// operand can never succeed — with `!unclaimed` passed as the exclusion mask so
/// the candidate stays inside the snapshot. A round that loses its CAS, or finds
/// none of its remaining candidates still parked, ends or retries without
/// charging an unpark to a bit nobody set.
///
/// `k` is the wave size, which is why this exists only under `ke16-c-batch`:
/// per-task spawning never knows it. The cost is up to `min(k, popcount(idle))`
/// CASes and syscalls on the spawner's critical path, ahead of its remaining
/// pushes — the trade the tournament measures against W-b's O(log W) activation
/// chain.
///
/// Answers how many workers it unparked, which is the bound stated above made
/// assertable (`wake_up_to_never_wakes_more_workers_than_the_entry_mask_held`).
/// The production caller discards it and the count folds away with the branch
/// that would have read it.
// === KE16 W switch: ke16-w-fanout ===
#[cfg(feature = "ke16-w-fanout")]
pub(crate) fn wake_up_to(inner: &PoolInner, k: usize) -> usize {
    publish_fence();
    // Acquire: matches the Release `fetch_or` in `mark_idle`, so a set bit
    // observed here means that worker's park sequence has been published.
    // This one load fixes the fan-out's target set for the whole call.
    let mut unclaimed = inner.idle.load(Ordering::Acquire);
    if unclaimed == 0 {
        return 0;
    }
    // Relaxed: the rotor only spreads which parked worker is picked first; no
    // data travels through it (correctness rests on the claims' CASes alone).
    // One draw for the whole fan-out — the claimed bits are cleared from
    // `unclaimed`, so the walk moves on without a second RMW per round.
    let start = (inner.wake_rotor.fetch_add(1, Ordering::Relaxed) % 64) as u32;
    let mut woke = 0usize;
    for _ in 0..k {
        // Acquire: the CAS below needs the word as it is NOW as its expected
        // operand; the candidate set stays `unclaimed` through the exclusion.
        let observed = inner.idle.load(Ordering::Acquire);
        if observed & unclaimed == 0 {
            break;
        }
        if let Some(id) = claim_one_idle(inner, observed, !unclaimed, start) {
            debug_assert!((id as usize) < inner.workers.len());
            // A5-1's rule, which every claimer owes: the claim and the unpark
            // are one branch with no early return between them, or the claimed
            // worker is a lane lost until shutdown.
            unclaimed &= !(1u64 << id);
            inner.workers[id as usize].thread.unpark();
            woke += 1;
        }
    }
    woke
}

/// Whether a push is going to READ the destination's pre-push length, resolved
/// by the type system rather than by a branch.
///
/// The length is not free to compute. `Injector::len` is a stable-tail retry
/// loop of three `SeqCst` loads of `head`/`tail` — the lines every pusher and
/// thief touch — and `Worker::len` is a Relaxed `back` load plus a `SeqCst` load
/// of `front`, the line every thief CASes. Neither is dead code the compiler may
/// drop, so the only way not to pay is not to issue the call: the two
/// implementors below give the placement functions ONE body with two
/// instantiations, [`ProbeLen`] issuing the probe and [`NoProbe`] having no call
/// in it to remove. Static dispatch on a ZST — no branch at any optimisation
/// level, no code at run time.
///
/// Two things suppress the probe, and they are separate. Whether the BUILD has
/// a reader at all is `ke16-w-gate`, and it is [`ProbeLen`]'s own `#[cfg]`: the
/// un-gated build wakes unconditionally, so even the probing implementor issues
/// nothing. Whether THIS PUSH has a reader is [`NoProbe`], and its one caller is
/// every push of a batch wave after the FIRST, whose wake decision has already
/// been taken. Without the second a wave of `n` under the gate would pay `n`
/// probes for a value read once — the physics colored solve emits
/// `num_threads() * CHUNKS_PER_WORKER` = 96 at W = 16 — and App-4 would remove
/// none of the "N−1 `len()` loads" `KE16-DESIGN-W.md` §4.3 prices it at.
// === KE16 W switch: ke16-w-gate === (the probe half of the one gate above)
trait LenProbe {
    /// The injector's length before a push, or `0` when nobody will read it.
    fn injector(inj: &crossbeam_deque::Injector<Task>) -> usize;

    /// The owner deque's length before a push, or `0` when nobody will read it.
    #[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
    fn deque(local: &Worker<Task>) -> usize;
}

/// The pre-push length is read — by [`wake_after_push`]'s ≤1 gate, the only
/// reader there is.
struct ProbeLen;

/// The pre-push length is not read, so it is not computed. Reached only by a
/// batch wave's silent pushes.
// === KE16 C switch: ke16-c-batch ===
#[cfg(feature = "ke16-c-batch")]
struct NoProbe;

impl LenProbe for ProbeLen {
    #[inline]
    fn injector(inj: &crossbeam_deque::Injector<Task>) -> usize {
        // Without the gate there is no reader even on this arm, and the un-gated
        // build must not start paying for a value nothing reads.
        #[cfg(feature = "ke16-w-gate")]
        return inj.len();
        #[cfg(not(feature = "ke16-w-gate"))]
        {
            let _ = inj;
            0
        }
    }

    #[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
    #[inline]
    fn deque(local: &Worker<Task>) -> usize {
        #[cfg(feature = "ke16-w-gate")]
        return local.len();
        #[cfg(not(feature = "ke16-w-gate"))]
        {
            let _ = local;
            0
        }
    }
}

#[cfg(feature = "ke16-c-batch")]
impl LenProbe for NoProbe {
    #[inline]
    fn injector(_inj: &crossbeam_deque::Injector<Task>) -> usize {
        0
    }

    #[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
    #[inline]
    fn deque(_local: &Worker<Task>) -> usize {
        0
    }
}

/// Push into the global injector, answering the pre-push length for the wake
/// decision the caller owes (`0` under [`NoProbe`], where none is owed). Every
/// arm's destination for a push whose origin is not a lane of `inner`.
#[inline]
fn push_global_no_wake<P: LenProbe>(inner: &PoolInner, task: Task) -> usize {
    let pre_len = P::injector(&inner.injector_global);
    inner.injector_global.push(task);
    pre_len
}

/// Push onto the calling worker's OWN registered Chase-Lev deque (KE16 A1),
/// answering the pre-push length for the wake decision the caller owes.
///
/// This is the arm that closes defect A by locality rather than by a shared
/// queue: `inner.stealers[lane.wid]` is the stealer of exactly this deque, so
/// the wave is reachable by every sibling's `try_steal_random` and by any
/// joiner, while the push itself costs no RMW at all.
// === KE16 A switch: ke16-a1 / ke16-a1-fifo ===
#[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
#[inline]
fn push_on_lane_no_wake<P: LenProbe>(
    _inner: &PoolInner,
    lane: tls::WorkerLane,
    task: Task,
) -> usize {
    // D5 (`KE16-DESIGN-A.md` §1.3): every `&Worker` minted from the TLS slot is
    // consumed by ONE method call in its own statement, or handed to a helper
    // that runs no task body — `LenProbe::deque` is such a helper — so no
    // protected tag on the deque spans a task body and none is used after one.
    let pre_len = P::deque(lane.deque());
    // The Chase-Lev owner push publishes with a PLAIN STORE (a Release fence and
    // a Relaxed `back` store, no RMW). Nothing in it is a StoreLoad barrier,
    // which is why the wake decision one level down opens with `publish_fence`.
    lane.deque().push(task);
    pre_len
}

/// Push into the calling worker's own local injector — today's placement, and
/// A2's — answering the pre-push length for the wake decision the caller owes.
// === KE16 A switch: default / ke16-a2 / ke16-a5's fallback ===
#[cfg(not(any(feature = "ke16-a1", feature = "ke16-a1-fifo", feature = "ke16-a3")))]
#[inline]
fn push_on_lane_no_wake<P: LenProbe>(
    inner: &PoolInner,
    lane: tls::WorkerLane,
    task: Task,
) -> usize {
    let inj = &inner.injector_local[lane.wid as usize];
    let pre_len = P::injector(inj);
    inj.push(task);
    pre_len
}

/// KE16 A5: place `task` directly in a PARKED sibling's local injector and wake
/// it. `Err(task)` hands the task back when no worker was idle or the claim lost
/// its CAS, and the caller falls through to A2's placement.
///
/// The idle mask is read BEFORE the placement rather than after it, so for once
/// the wake target and the destination agree. Four invariants shape the body
/// (`KE16-DESIGN-A.md` §4.3):
///
/// - **A5-1** a claimed bit is ALWAYS unparked. A worker parks with an untimed
///   `park()`, so a bit taken out of the mask without the matching `unpark` is a
///   core lost until shutdown; the claim and the unpark are one branch with no
///   early return between them.
/// - **A5-2** the destination must be STEALABLE, or a target that wakes and then
///   loses the race for other work parks again with this task stranded behind
///   it. `injector_local[target]` is stealable only because `ke16-a2` — which
///   this feature implies — put it in every sibling's scan set.
/// - **A5-3** never a foreign pool's queue: the mask read, the slot indexed and
///   the worker unparked all belong to `inner`, the pool this spawn targets, so
///   a cross-pool spawn takes the fallback rather than a lane of its own pool.
/// - **A5-4** ONE CAS attempt, never a spin: a spawner must not loop on the idle
///   line against W parking workers.
///
/// This arm needs no `publish_fence`: the claimed worker is unparked
/// EXPLICITLY after the push, and the parker's own Release/Acquire park protocol
/// orders the two. The fallback arm goes through the fenced prologue as usual.
// === KE16 A switch: ke16-a5 (on top of ke16-a2) ===
#[cfg(feature = "ke16-a5")]
fn try_place_on_idle_sibling(inner: &PoolInner, task: Task) -> Result<(), Task> {
    // Acquire: matches the Release `fetch_or` in `mark_idle`, so a set bit
    // observed here means that worker's park sequence has been published.
    let observed = inner.idle.load(Ordering::Acquire);
    if observed == 0 {
        return Err(task);
    }
    // Relaxed: the rotor only spreads which parked worker is picked; no data
    // travels through it (correctness rests on the claim's CAS alone).
    let start = (inner.wake_rotor.fetch_add(1, Ordering::Relaxed) % 64) as u32;
    match claim_one_idle(inner, observed, 0, start) {
        Some(target) => {
            debug_assert!((target as usize) < inner.workers.len());
            // A5-1's receipt, in the crate's own test build only (the measured
            // arm pays no counter RMW — `PoolInner::a5_claimed`): the claim is
            // counted here and the unpark below, so any control flow inserted
            // between them shows up as unequal counters instead of as a core
            // that is lost until shutdown and a throughput number blamed on A5.
            #[cfg(test)]
            inner.a5_claimed.fetch_add(1, Ordering::Relaxed);
            inner.injector_local[target as usize].push(task);
            inner.workers[target as usize].thread.unpark();
            #[cfg(test)]
            inner.a5_unparked.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        None => Err(task),
    }
}

/// Push a task into the pool and take the wake decision ([`wake_after_push`]).
///
/// The destination is chosen by the ONE identity predicate
/// [`tls::worker_lane_for`] (KE16 App-6), not by the bare TLS worker id: only a
/// thread that IS a registered worker of `inner`, acting as that worker, has a
/// lane here. On any other answer — a cross-pool spawn, an `install` frame, the
/// dispatcher, an unattached thread — the push goes to the global injector,
/// which every worker polls. A worker of pool A (id `wid`) pushing into pool B
/// must not land the task in a per-worker slot of B indexed by ITS id: that
/// slot's owner in B may be parked while the wake claims some other bit, so the
/// task could sit undrained indefinitely.
///
/// KE16 (this paragraph and its switches are deleted with the features): the
/// placement axis of the tournament is exactly this function.
///
/// - default: the lane's `injector_local[wid]`, which only that worker drains —
///   so a wave spawned from inside a worker runs serially on it (defect A).
/// - `ke16-a1` / `ke16-a1-fifo`: the lane's own REGISTERED deque, whose stealer
///   every sibling scans. The queue op becomes a plain store with no RMW.
/// - `ke16-a2`: placement unchanged; reachability comes from the sibling scan
///   gaining a second probe instead.
/// - `ke16-a3`: everything to `injector_global` — the reachability floor against
///   which locality has to justify itself.
/// - `ke16-a5`: into a claimed idle sibling's injector, with a targeted wake,
///   before the lane test is even reached (so it changes the dispatcher's pushes
///   too); A2's placement is its fallback.
pub(crate) fn push_task(inner: &PoolInner, task: Task) {
    if let Some(pre_len) = push_task_no_wake(inner, task) {
        wake_after_push(inner, pre_len);
    }
}

/// Place a task exactly as [`push_task`] does but take NO wake decision,
/// answering what the caller owes: `Some(pre_len)` — the destination queue's
/// pre-push length, to be handed to [`wake_after_push`] — or `None` when the
/// placement already carried its own targeted wake and no further decision is
/// due (the A5 arm unparks the sibling whose bit it claimed).
///
/// KE16 App-4 is why the wake is separable from the placement: a batch spawn
/// takes ONE wake decision for the whole wave, right after its FIRST push, and
/// leaves the remaining pushes silent (`KE16-DESIGN-W.md` §4.1). Every
/// placement arm of the tournament lives here rather than in [`push_task`], so
/// per-task and batch spawns share one destination rule.
#[inline]
pub(crate) fn push_task_no_wake(inner: &PoolInner, task: Task) -> Option<usize> {
    place_task::<ProbeLen>(inner, task)
}

/// Place a task exactly as [`push_task_no_wake`] does, but for a push whose
/// wake decision has ALREADY been taken by the wave it belongs to — so the
/// destination's pre-push length is never read.
///
/// This exists because reading it is not free: under `ke16-w-gate` the probe is
/// a `SeqCst` load of the victim deque's `front` line, or a three-load retry
/// loop on the injector's head/tail (see [`LenProbe`]), and a wave of `n` that
/// probed on every push would pay `n` of them for a value read once.
/// `KE16-DESIGN-W.md` §4.3 prices App-4 as removing "N−1 `len()` loads" under
/// W-b; this is the function that removes them. The W gate itself stays at its
/// one site ([`wake_after_push`], `KE16-DESIGN.md` §4) — this arm does not test
/// the gate, it declines to compute the gate's input.
///
/// Under `ke16-a5` the placement still carries its own targeted wake (the
/// claimed sibling's `unpark`), which is the placement's, not the wave's; the
/// answer that no decision is due has no reader here and is dropped.
// === KE16 C switch: ke16-c-batch ===
#[cfg(feature = "ke16-c-batch")]
#[inline]
pub(crate) fn push_task_silent(inner: &PoolInner, task: Task) {
    let _no_decision_owed = place_task::<NoProbe>(inner, task);
}

/// The placement itself, shared by the wake-deciding and the silent push.
///
/// `P` says whether the caller will read the pre-push length, and is the ONLY
/// difference between the two entry points above — one body, one destination
/// rule, one place where the A axis lives.
#[inline]
fn place_task<P: LenProbe>(inner: &PoolInner, task: Task) -> Option<usize> {
    // === KE16 A switch: ke16-a5 ===
    #[cfg(feature = "ke16-a5")]
    let task = match try_place_on_idle_sibling(inner, task) {
        // The claimed sibling was unparked by the placement itself, so the
        // caller owes nothing: A5's wake IS its placement.
        Ok(()) => return None,
        Err(task) => task,
    };

    // === KE16 A switch: ke16-a3 ===
    #[cfg(feature = "ke16-a3")]
    return Some(push_global_no_wake::<P>(inner, task));

    #[cfg(not(feature = "ke16-a3"))]
    Some(match tls::worker_lane_for(inner) {
        Some(lane) => push_on_lane_no_wake::<P>(inner, lane, task),
        None => push_global_no_wake::<P>(inner, task),
    })
}

/// SplitMix64 mixer (Sebastiano Vigna, 2014). Used as a seed generator
/// for the per-worker [`XorShift64Star`] PRNG so that workers spawned
/// with adjacent ids don't start in adjacent PRNG states.
///
/// `pub(crate)` for the KE16 B joiner arms, which seed their sweep from the
/// joining lane and the scope's address.
#[inline]
pub(crate) fn splitmix64(mut z: u64) -> u64 {
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

    /// W-a: the claim core answers an exhausted mask with `None` rather than a
    /// claim of a bit nobody set. A CAS of `observed` against an unchanged
    /// `observed` succeeds trivially, so a core that picked its candidate
    /// without this check would report `Some(start)` here — and its caller would
    /// unpark worker `start`, out of bounds on a pool of `start` workers or
    /// less. No in-crate caller reaches this input today (both check the mask
    /// first); the two that will, W-f's bounded fan-out loop and A5's claim from
    /// a stale mask, are why the contract is total.
    #[test]
    fn claim_one_idle_answers_none_when_every_set_bit_is_excluded() {
        let pool = crate::ThreadPoolBuilder::new().num_threads(2).build();
        let observed = 0b0001u64;
        for start in [0u32, 1, 5, 63] {
            assert!(
                claim_one_idle(&pool.inner, observed, observed, start).is_none(),
                "an exhausted candidate mask must claim nothing (start = {start})"
            );
        }
        assert!(
            claim_one_idle(&pool.inner, 0, 0, 7).is_none(),
            "an empty idle word must claim nothing"
        );
    }

    /// W-a: the exclusion is exercised with the excluded bit ACTUALLY SET in
    /// `idle` — the only shape in which the two masks can be confused, and the
    /// shape the W axis's thief-residue cascade will have (it issues its wake
    /// from inside the post-`mark_idle` re-poll, holding its own bit). Until that
    /// caller is built this test is the only exercise of a non-zero `exclude`.
    ///
    /// The call runs on a helper thread so that a claim core which can never
    /// satisfy its CAS reports as a timed-out receive with a message, instead of
    /// hanging the test binary at 100 % CPU.
    #[test]
    fn unpark_one_idle_excluding_claims_a_sibling_while_the_excluded_bit_is_set() {
        use std::sync::Arc;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let pool = crate::ThreadPoolBuilder::new().num_threads(4).build();

        // Workers park untimed, so once two bits are set the mask is stable
        // until someone claims one.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut mask = 0u64;
        while Instant::now() < deadline {
            mask = pool.parked_mask();
            if mask.count_ones() >= 2 {
                break;
            }
            std::thread::yield_now();
        }
        assert!(
            mask.count_ones() >= 2,
            "the pool never had two workers parked; the exclusion cannot be exercised"
        );
        let excluded = 1u64 << mask.trailing_zeros();

        let inner = Arc::clone(&pool.inner);
        let (tx, rx) = mpsc::channel();
        let caller = std::thread::spawn(move || {
            let woke = unpark_one_idle_excluding(&inner, excluded);
            let _ = tx.send(woke);
        });
        let woke = rx.recv_timeout(Duration::from_secs(10)).expect(
            "invariant: unpark_one_idle_excluding returns. It did not, which means its claim CAS \
             can never be satisfied while an excluded bit is set — the folded-mask form, whose \
             expected operand is not the word in memory",
        );
        caller.join().expect("the calling thread must not panic");

        assert!(
            woke,
            "a claimable sibling bit was set, so a wake had to be issued"
        );
        assert_eq!(
            pool.parked_mask() & excluded,
            excluded,
            "the excluded worker was claimed anyway"
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

    /// A pool whose every worker is stopped inside a task body, so that
    /// `inner.idle` is the test's alone to write and read back.
    ///
    /// Needed because both [`claim_one_idle`] and [`unpark_one_idle_excluding`]
    /// read-modify-write that one word, and so does every worker sitting in
    /// `worker_main`'s poll loop (`mark_idle` before a park, `unmark_idle`
    /// after) — including after a SPURIOUS `park()` return, which no test can
    /// prevent. An assertion about the exact resulting word is therefore only
    /// deterministic while no worker is in that loop.
    ///
    /// Why `started == workers` proves that: each of the `4 * workers` tasks
    /// blocks until `release`, so a worker that begins one never finishes it and
    /// never returns to the loop. The counter can therefore only be raised by
    /// DISTINCT workers, and reaching `workers` means all of them are inside a
    /// body. A worker in a body has already run `unmark_idle`, so the word is
    /// `0` at that instant and stays there.
    // Spelled out rather than taken from : this fixture is native
    // test code ( never runs under loom), and the flag is only a
    // release signal for the blocking bodies, not part of a modelled protocol.
    use std::sync::atomic::AtomicBool;

    struct QuiescedIdle {
        pool: std::sync::Arc<crate::ThreadPool>,
        release: std::sync::Arc<AtomicBool>,
    }

    impl QuiescedIdle {
        fn new(workers: u32) -> Self {
            use core::sync::atomic::AtomicU32;
            use std::sync::Arc;

            let pool = crate::ThreadPoolBuilder::new()
                .num_threads(workers as usize)
                .build();
            let release = Arc::new(AtomicBool::new(false));
            let started = Arc::new(AtomicU32::new(0));
            for _ in 0..workers * 4 {
                let release = Arc::clone(&release);
                let started = Arc::clone(&started);
                pool.spawn(move || {
                    started.fetch_add(1, Ordering::AcqRel);
                    while !release.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                });
            }

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while started.load(Ordering::Acquire) < workers && std::time::Instant::now() < deadline
            {
                std::thread::yield_now();
            }
            let this = Self { pool, release };
            assert_eq!(
                started.load(Ordering::Acquire),
                workers,
                "not every worker entered a task body within the deadline; the idle word would \
                 not be the test's alone"
            );
            assert_eq!(
                this.idle(),
                0,
                "a worker inside a task body has already unmarked itself, so the idle word must \
                 be empty before the test writes it"
            );
            this
        }

        fn inner(&self) -> &PoolInner {
            &self.pool.inner
        }

        fn set_idle(&self, word: u64) {
            self.pool.inner.idle.store(word, Ordering::Release);
        }

        fn idle(&self) -> u64 {
            self.pool.inner.idle.load(Ordering::Acquire)
        }
    }

    impl Drop for QuiescedIdle {
        fn drop(&mut self) {
            // Leave no synthetic bit behind: a stale bit would charge the pool's
            // shutdown wake to a worker that is not parked.
            self.set_idle(0);
            self.release.store(true, Ordering::Release);
        }
    }

    /// One claim attempt, retried past a spurious `compare_exchange_weak`
    /// failure. Nothing else can make the CAS lose here: the word in memory IS
    /// `observed` (the fixture owns it), so a real losing racer does not exist.
    fn claim_until_decided(
        q: &QuiescedIdle,
        observed: u64,
        exclude: u64,
        start: u32,
    ) -> Option<u32> {
        for _ in 0..64 {
            if let Some(id) = claim_one_idle(q.inner(), observed, exclude, start) {
                return Some(id);
            }
            if observed & !exclude == 0 {
                return None; // the documented total answer, not a lost CAS
            }
            q.set_idle(observed);
        }
        panic!("64 consecutive claim attempts on an uncontended word all failed");
    }

    /// W-a, the claim core's contract in its general form: a `Some(id)` names a
    /// bit that WAS set in the caller's word and is NOT excluded, and it clears
    /// exactly that bit. `claim_one_idle` never indexes `inner.workers`, so bits
    /// above the worker count are legitimate inputs here and are used
    /// deliberately — a caller holding a stale word may present any of them.
    #[test]
    fn claim_one_idle_claims_only_a_bit_that_was_set_and_not_excluded() {
        let q = QuiescedIdle::new(4);
        let cases: [(u64, u64); 8] = [
            (0, 0),
            (0b1, 0),
            (0b1, 0b1),
            (0b1011, 0b0001),
            (u64::MAX, 0),
            (u64::MAX, u64::MAX),
            (1 << 63, 0),
            (0b1010_1010, 0b1010_0000),
        ];
        for (observed, exclude) in cases {
            for start in (0u32..64).step_by(7) {
                q.set_idle(observed);
                match claim_until_decided(&q, observed, exclude, start) {
                    Some(id) => {
                        assert_ne!(
                            observed & (1u64 << id),
                            0,
                            "claimed bit {id} was not set in the observed word {observed:#x}"
                        );
                        assert_eq!(
                            exclude & (1u64 << id),
                            0,
                            "claimed bit {id} is in the exclusion mask {exclude:#x}"
                        );
                        assert_eq!(
                            q.idle(),
                            observed & !(1u64 << id),
                            "a successful claim must clear exactly the claimed bit"
                        );
                    }
                    None => {
                        assert_eq!(
                            observed & !exclude,
                            0,
                            "the only total `None` is an exhausted candidate mask \
                             (observed {observed:#x}, exclude {exclude:#x})"
                        );
                        assert_eq!(q.idle(), observed, "a refused claim must not write");
                    }
                }
            }
        }
    }

    /// W-a: the rotation rule the doc comment states — the lowest set candidate
    /// at or above `start`, wrapping. The expectation is recomputed here by a
    /// linear scan over the wrapped index order, which is a different
    /// formulation from production's rotate-right / `trailing_zeros` pick, so
    /// the two agreeing is a real cross-check rather than a transcription.
    #[test]
    fn claim_one_idle_picks_the_lowest_candidate_at_or_above_start_wrapping() {
        let q = QuiescedIdle::new(4);
        let bits = [3u32, 17, 40];
        let observed = bits.iter().fold(0u64, |w, b| w | (1u64 << b));
        for start in 0u32..64 {
            q.set_idle(observed);
            let expected = (0u32..64)
                .map(|k| (start + k) % 64)
                .find(|c| bits.contains(c))
                .expect("the mask is non-empty");
            assert_eq!(
                claim_until_decided(&q, observed, 0, start),
                Some(expected),
                "rotation from start {start} must select the next candidate in wrapped order"
            );
        }
    }

    /// W-a: `None` when the word moved under the caller — the lost-CAS half of
    /// the contract, and the reason the caller re-reads instead of retrying with
    /// the same operand.
    #[test]
    fn claim_one_idle_answers_none_when_the_idle_word_moved_under_the_caller() {
        let q = QuiescedIdle::new(4);
        q.set_idle(0b0110);
        assert_eq!(
            claim_one_idle(q.inner(), 0b0011, 0, 0),
            None,
            "a stale expected operand must not claim"
        );
        assert_eq!(
            q.idle(),
            0b0110,
            "a lost claim must leave the word untouched"
        );
    }

    /// W-a: the wake decision refuses, and terminates, when every parked bit is
    /// the caller's own. This is the residue-cascade shape the `exclude`
    /// parameter exists for; the deterministic fixture makes the "and the mask
    /// is untouched" half assertable (on a live pool a spurious park return can
    /// clear a bit under the assertion).
    #[test]
    fn unpark_one_idle_excluding_returns_false_when_every_parked_bit_is_excluded() {
        let q = QuiescedIdle::new(4);
        q.set_idle(0b0101);
        assert!(
            !unpark_one_idle_excluding(q.inner(), 0b0101),
            "no claimable bit remained, so no wake may be reported"
        );
        assert_eq!(q.idle(), 0b0101, "a refused wake must not disturb the mask");
    }

    /// W-a: a wake claims exactly one bit — never two, never a bit that was not
    /// set. All three bits are below the worker count, so the `unpark` this
    /// issues addresses a real `WorkerHandle`.
    #[test]
    fn unpark_one_idle_clears_exactly_one_parked_bit() {
        let q = QuiescedIdle::new(4);
        q.set_idle(0b1011);
        assert!(unpark_one_idle(q.inner()), "three bits were claimable");
        let after = q.idle();
        assert_eq!(
            after.count_ones(),
            2,
            "exactly one of the three claimable bits must be gone (word {after:#b})"
        );
        assert_eq!(
            after & !0b1011,
            0,
            "no bit outside the original mask may appear"
        );
    }

    /// W-f's bound: ONE fan-out unparks at most as many workers as its ENTRY
    /// mask held — `min(k, popcount(idle))`, the number the index row and
    /// `KE16-DESIGN-W.md` §5's cost sentence price the arm at — however many
    /// times a bit comes back while it runs.
    ///
    /// The re-arming thread is that hazard, forced. A worker this fan-out wakes
    /// can find the queue holding only the wave's FIRST task, snooze,
    /// `mark_idle` and park again microseconds later, putting its bit back in
    /// the word; a fan-out that re-read the mask as its CANDIDATE set would
    /// claim that bit again and keep going, bounded only by `k` — one syscall
    /// per round, however few workers were ever parked. The re-armer restores the mask
    /// in a tight loop, and the fan-out is not entered until it reports its
    /// first restore, so a re-reading implementation runs its `k` rounds over a
    /// mask that is never empty for long.
    ///
    /// The assertion is ONE-SIDED, which is what makes it a gate rather than a
    /// coin toss: the re-armer only ORs bits back, so the entry snapshot is at
    /// most `MASK` and the shipped form cannot exceed `popcount(MASK)` under any
    /// interleaving. Only the direction that would let a defect through — the
    /// re-armer losing every one of `k` races — is probabilistic, and it was
    /// checked by patching the candidate set back to a per-round re-read:
    /// MEASURED, that form woke 2785, 695 and 4096 workers on three runs of
    /// this test, against the 4 its mask held.
    #[test]
    #[cfg(feature = "ke16-w-fanout")]
    fn wake_up_to_never_wakes_more_workers_than_the_entry_mask_held() {
        use std::sync::Arc;

        // Four workers, so every bit of the mask addresses a real `WorkerHandle`.
        const MASK: u64 = 0b1111;
        // Far above any wave this tree emits (the largest is the physics
        // colored solve's `num_threads() * CHUNKS_PER_WORKER` = 96 at W = 16;
        // `par_iter` over 65536 rows emits 16), chosen so that a re-reading
        // candidate set cannot hide behind a small round count.
        const K: usize = 4096;

        let q = QuiescedIdle::new(4);
        q.set_idle(MASK);

        let stop = Arc::new(AtomicBool::new(false));
        let armed = Arc::new(AtomicBool::new(false));
        let stop_cl = Arc::clone(&stop);
        let armed_cl = Arc::clone(&armed);
        let inner = Arc::clone(&q.pool.inner);
        let rearm = std::thread::spawn(move || {
            while !stop_cl.load(Ordering::Acquire) {
                // Release: pairs with the fan-out's Acquire load, exactly as
                // `mark_idle`'s `fetch_or` does for a real parking worker.
                inner.idle.fetch_or(MASK, Ordering::Release);
                armed_cl.store(true, Ordering::Release);
            }
        });
        // No yield in that loop and none here: the window the race needs is the
        // few microseconds `wake_up_to` spends on its unparks, so the re-armer
        // must already be hammering when the fan-out starts. Bounded, because a
        // thread that never starts must fail the test rather than hang it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !armed.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::hint::spin_loop();
        }
        let was_armed = armed.load(Ordering::Acquire);

        let woke = wake_up_to(q.inner(), K);

        stop.store(true, Ordering::Release);
        rearm.join().expect("the re-arming thread must not panic");

        assert!(
            was_armed,
            "the re-arming thread never ran, so nothing put a claimed bit back and the bound was \
             never under test"
        );
        assert!(
            woke <= MASK.count_ones() as usize,
            "one fan-out woke {woke} workers from an entry mask of {} — the candidate set was \
             re-read instead of being fixed at entry, so `k` is the only bound left",
            MASK.count_ones()
        );
        assert!(
            woke >= 1,
            "the entry mask was non-empty, so the fan-out had to wake at least one worker"
        );
    }

    /// W-f is GATED, not gate-exempt: the fan-out changes the WIDTH of a wake,
    /// never whether one is owed.
    ///
    /// This is the receipt for the decision recorded on
    /// [`wake_after_push_up_to`] — without it a `wgc+c1f` row would carry "the
    /// ≤1 rule lifted on the wave's first push" on top of the fan-out count
    /// Step W/C reads it for. Both directions are asserted from one body: with
    /// `ke16-w-gate` a first push into a queue that already held 2 tasks must
    /// wake NOBODY and leave the mask whole; without it the wake is
    /// unconditional and the fan-out runs, which is exactly what
    /// `wake_after_push` does at the same `pre_len`.
    #[test]
    #[cfg(feature = "ke16-w-fanout")]
    fn wake_after_push_up_to_obeys_the_push_gate() {
        const MASK: u64 = 0b1111;
        const PRE_LEN_ABOVE_THE_GATE: usize = 2;

        let q = QuiescedIdle::new(4);
        q.set_idle(MASK);
        let woke = wake_after_push_up_to(q.inner(), PRE_LEN_ABOVE_THE_GATE, 4);

        if cfg!(feature = "ke16-w-gate") {
            assert_eq!(
                woke, 0,
                "the destination already held 2 tasks, so the ≤1 rule owes no wake and the \
                 fan-out must not take one"
            );
            assert_eq!(
                q.idle(),
                MASK,
                "a gated-out fan-out must not claim a bit (word {:#b})",
                q.idle()
            );
        } else {
            assert!(
                woke >= 1,
                "without the gate every push wakes, so the fan-out must run at any pre-push length"
            );
        }

        // The gate's OTHER side, on the same build: a first push into an empty
        // destination is owed a wake and fans out.
        let q = QuiescedIdle::new(4);
        q.set_idle(MASK);
        assert!(
            wake_after_push_up_to(q.inner(), 0, 4) >= 1,
            "an empty destination is owed a wake under every W arm"
        );
    }

    /// The base build's W semantics: the wake is UNCONDITIONAL. `pre_len` is
    /// carried for the W axis's `<= 1` gate and must not change the base
    /// decision, so a pre-push length far above any gate still wakes.
    #[test]
    #[cfg(not(feature = "ke16-w-gate"))]
    fn wake_after_push_wakes_on_every_push_in_the_base_build() {
        let q = QuiescedIdle::new(4);
        q.set_idle(0b0011);
        wake_after_push(q.inner(), 999);
        assert_eq!(
            q.idle().count_ones(),
            1,
            "the base wake decision is unconditional; a large pre-push length must not gate it"
        );
    }

    /// A5's native gate (`KE16-DESIGN.md` §8, the A5 row: "stress test with W−1
    /// workers parked: `claimed == unparked` counters"; `KE16-DESIGN-A.md`
    /// §4.4). The loom half of that row lives in `tests/loom_pool.rs`, which is
    /// `#![cfg(loom)]` and contributes `running 0 tests` to every native run —
    /// so without this row no assertion in any native binary distinguishes the
    /// A5 arm from the default build.
    ///
    /// The shape the design names: one worker running (the spawner), W−1 parked,
    /// N spawns from that worker. Each spawn that meets a set bit claims it,
    /// pushes into THAT worker's `injector_local` and MUST unpark it (A5-1).
    ///
    /// Why the counters and not a completion assertion: a claim whose unpark
    /// went missing strands no task — `ke16-a5` implies `ke16-a2`, so the
    /// destination is in every sibling's scan set and whoever is awake drains it
    /// (A5-2). The pool simply runs one lane short for the rest of the process,
    /// which every existing test passes over and which a grid run would record
    /// as "A5 is slow" rather than as a defect. `claimed == unparked` is the
    /// only observation that separates the two.
    #[test]
    #[cfg(feature = "ke16-a5")]
    fn a5_every_claimed_idle_bit_is_unparked_and_every_task_completes() {
        use core::sync::atomic::AtomicU32;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        const WORKERS: usize = 4;
        const TASKS: u32 = 256;

        let pool = crate::ThreadPoolBuilder::new().num_threads(WORKERS).build();
        let done = Arc::new(AtomicU32::new(0));
        // u32::MAX = the spawner body never ran at all.
        let parked_before = Arc::new(AtomicU32::new(u32::MAX));
        let spawner_finished = Arc::new(AtomicBool::new(false));

        let pool_cl = Arc::clone(&pool);
        let done_cl = Arc::clone(&done);
        let parked_before_cl = Arc::clone(&parked_before);
        let spawner_finished_cl = Arc::clone(&spawner_finished);
        pool.spawn(move || {
            // Wait for the siblings to reach their untimed park, so the spawns
            // below meet a non-zero idle mask and take the claim path at all.
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut parked = 0;
            while Instant::now() < deadline {
                parked = pool_cl.parked_mask().count_ones();
                if parked as usize >= WORKERS - 1 {
                    break;
                }
                std::thread::yield_now();
            }
            parked_before_cl.store(parked, Ordering::Release);

            for _ in 0..TASKS {
                let done = Arc::clone(&done_cl);
                pool_cl.spawn(move || {
                    done.fetch_add(1, Ordering::AcqRel);
                });
            }
            // Release: matches the Acquire load below, and it is what makes the
            // counter reads sound. Both increments happen on THIS thread; a task
            // body could otherwise complete the wave while its own spawn's
            // unpark count was still unpublished, which would read as a false
            // `claimed != unparked`.
            spawner_finished_cl.store(true, Ordering::Release);
        });

        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline
            && !(spawner_finished.load(Ordering::Acquire) && done.load(Ordering::Acquire) == TASKS)
        {
            std::thread::yield_now();
        }

        assert!(
            spawner_finished.load(Ordering::Acquire),
            "the spawner body never finished pushing (siblings parked when it looked: {})",
            parked_before.load(Ordering::Acquire)
        );
        assert_eq!(
            done.load(Ordering::Acquire),
            TASKS,
            "every placed task must run"
        );
        assert!(
            parked_before.load(Ordering::Acquire) as usize >= WORKERS - 1,
            "the shape was never established: fewer than W-1 siblings were parked, so the claim \
             path may never have been taken (u32::MAX = the spawner body never ran)"
        );

        // Relaxed: the happens-before comes from `spawner_finished`, loaded
        // Acquire above, not from these counters.
        let claimed = pool.inner.a5_claimed.load(Ordering::Relaxed);
        let unparked = pool.inner.a5_unparked.load(Ordering::Relaxed);
        assert!(
            claimed > 0,
            "no idle bit was ever claimed, so the A5 placement arm was not exercised and this \
             gate is vacuous"
        );
        assert_eq!(
            claimed, unparked,
            "A5-1: a claimed idle bit must ALWAYS be unparked. A worker parks untimed, so a claim \
             whose unpark went missing is a core lost until shutdown"
        );

        // The counters see control flow inserted BETWEEN the claim and the
        // unpark; they cannot see the unpark call deleted (they would simply
        // both still be incremented — MEASURED, by mutating the arm). This is
        // the observable half of A5-1, and it is the half that is total: the
        // claim CLEARED the target's idle bit, so a target that was never
        // unparked is still inside `worker_main`'s untimed `park()` with its bit
        // down, and can therefore never re-enter the idle set. A pool that comes
        // back fully parked has no such worker.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut mask = 0u64;
        while Instant::now() < deadline {
            mask = pool.parked_mask();
            if mask.count_ones() as usize == WORKERS {
                break;
            }
            std::thread::yield_now();
        }
        let stranded = !mask & ((1u64 << WORKERS) - 1);
        assert_eq!(
            mask.count_ones() as usize,
            WORKERS,
            "worker(s) {stranded:#b} never came back to the idle set: a claim took their idle bit \
             and no unpark followed, so they are still inside an untimed `park()` and the pool is \
             short a lane until shutdown (A5-1)"
        );
        for (i, inj) in pool.inner.injector_local.iter().enumerate() {
            assert!(
                inj.is_empty(),
                "worker {i}'s local injector still holds a task after the wave drained: a \
                 placement outlived the wake that was meant to serve it"
            );
        }
    }

    // ---------------------------------------------------------------------
    // W-a, the half no other assertion in this crate can fail on: WHERE the
    // rotor RMW sits. `KE16-DESIGN-W.md` section 1.1 step 3 buys the base's
    // only hot-path change with it -- "a busy pool pays one local fence and one
    // load, and NO shared RMW" -- and the rotor is invisible to every
    // behavioural test, because it only chooses WHICH parked bit a wake takes.
    // Moved back ahead of the idle load (the shape HEAD ships) every existing
    // test here still passes while every push on a busy pool pays a contended
    // `lock xadd` again. These three rows are what makes that a red.
    // ---------------------------------------------------------------------

    /// W-a step 3: nothing parked means the rotor is not touched at all.
    #[test]
    fn unpark_one_idle_leaves_the_wake_rotor_untouched_on_a_busy_pool() {
        let q = QuiescedIdle::new(4);
        // The fixture holds every worker inside a task body with an empty idle
        // word, so this thread is the only one that can move the rotor.
        let before = q.inner().wake_rotor.load(Ordering::Relaxed);
        for _ in 0..64 {
            assert!(
                !unpark_one_idle(q.inner()),
                "nothing is parked, so no wake may be reported"
            );
        }
        assert_eq!(
            q.inner().wake_rotor.load(Ordering::Relaxed),
            before,
            "64 wake decisions on a pool with nothing parked advanced the rotor: the RMW is ahead \
             of the idle load again, so every push on a busy pool pays a shared-line xadd \
             (KE16-DESIGN-W.md 1.1 step 3)"
        );
    }

    /// W-a step 3, the other early return: the refusal is decided against
    /// `observed & !exclude`, so an all-excluded word must skip the rotor too.
    /// The residue cascade the `exclude` parameter exists for issues its wake
    /// from the hottest place a thief can be; a rotor RMW ahead of that test
    /// would charge the shared line to a decision that never wakes anyone.
    #[test]
    fn unpark_one_idle_excluding_leaves_the_wake_rotor_untouched_when_every_bit_is_excluded() {
        let q = QuiescedIdle::new(4);
        q.set_idle(0b0101);
        let before = q.inner().wake_rotor.load(Ordering::Relaxed);
        assert!(
            !unpark_one_idle_excluding(q.inner(), 0b0101),
            "every parked bit was the caller's own, so no wake may be reported"
        );
        assert_eq!(
            q.inner().wake_rotor.load(Ordering::Relaxed),
            before,
            "a refused wake reached the rotor: the exclusion test must precede the RMW, not \
             follow it"
        );
    }

    /// W-a step 3's positive half: the rotor advances once per WAKE -- the
    /// "different but equally fair rotation" the design trades the per-push
    /// advance for. A rotor that stopped advancing would re-introduce the
    /// systematic lowest-id bias the rotation was added to remove.
    #[test]
    fn unpark_one_idle_advances_the_wake_rotor_exactly_once_per_wake() {
        const WAKES: u64 = 8;
        let q = QuiescedIdle::new(4);
        let before = q.inner().wake_rotor.load(Ordering::Relaxed);
        for round in 0..WAKES {
            q.set_idle(0b1111);
            assert!(
                unpark_one_idle(q.inner()),
                "round {round}: four bits were claimable"
            );
        }
        assert_eq!(
            q.inner().wake_rotor.load(Ordering::Relaxed) - before,
            WAKES,
            "the rotor must advance exactly once per issued wake: a lost CAS re-reads the word \
             and must not take a second offset, and a wake must not skip the rotor"
        );
    }

    /// W-a: the claim core's rotation and exclusion arithmetic over a random
    /// input domain, against an oracle written as a linear scan of the wrapped
    /// index order -- a different formulation from production's rotate-right /
    /// `trailing_zeros` / rotate-back, so agreement is a cross-check and not a
    /// transcription. The hand-picked case set above fixes three bits and
    /// sweeps `start`; this sweeps the whole word, including the words no
    /// production caller can present today but A5 (a stale mask) and W-f (a
    /// mask other threads are clearing) will.
    ///
    /// `observed` is deliberately unconstrained by the pool's worker count:
    /// `claim_one_idle` never indexes `inner.workers`, and a claimer holding a
    /// stale word may present any bit.
    #[test]
    fn claim_one_idle_agrees_with_an_independent_oracle_over_random_inputs() {
        const CASES: u32 = 4096;
        let q = QuiescedIdle::new(4);
        let mut rng = XorShift64Star::new(0x4b45_3136);
        for case in 0..CASES {
            let observed = match case % 8 {
                0 => 0,
                1 => u64::MAX,
                2 => 1u64 << (rng.next() % 64),
                _ => rng.next(),
            };
            // Four exclusion shapes: none, independent, total, and a subset of
            // the set bits -- the residue-cascade shape, where the excluded bit
            // is genuinely set in the word the CAS expects.
            let exclude = match case % 4 {
                0 => 0,
                1 => rng.next(),
                2 => observed,
                _ => rng.next() & observed,
            };
            let start = (rng.next() % 64) as u32;

            q.set_idle(observed);
            let got = claim_until_decided(&q, observed, exclude, start);

            let candidates = observed & !exclude;
            let expected = (0u32..64)
                .map(|k| (start + k) % 64)
                .find(|c| candidates & (1u64 << c) != 0);
            assert_eq!(
                got, expected,
                "case {case}: observed {observed:#018x}, exclude {exclude:#018x}, start {start} - \
                 the claim must be the lowest candidate at or above start, wrapping"
            );
            match got {
                Some(id) => assert_eq!(
                    q.idle(),
                    observed & !(1u64 << id),
                    "case {case}: a claim of bit {id} must clear that bit and no other"
                ),
                None => assert_eq!(
                    q.idle(),
                    observed,
                    "case {case}: a refused claim must leave the word exactly as it was"
                ),
            }
        }
    }

    /// W-a: the non-zero-`exclude` wake, made DETERMINISTIC. The live-pool
    /// sibling above establishes the same property against real parked
    /// workers, but it reads `parked_mask()` back on a pool whose workers may
    /// return spuriously from `park()` and clear a bit under the assertion.
    /// Here the fixture owns the word, and the 64 rounds sweep 64 consecutive
    /// rotor offsets -- so the excluded bit is refused from every starting
    /// position, including the ones where it IS the lowest candidate.
    #[test]
    fn unpark_one_idle_excluding_refuses_the_excluded_bit_from_every_rotor_offset() {
        const WORD: u64 = 0b1011;
        const EXCLUDED: u64 = 0b0001;
        let q = QuiescedIdle::new(4);
        for round in 0..64 {
            q.set_idle(WORD);
            assert!(
                unpark_one_idle_excluding(q.inner(), EXCLUDED),
                "round {round}: bits 1 and 3 were claimable"
            );
            let after = q.idle();
            assert_eq!(
                after & EXCLUDED,
                EXCLUDED,
                "round {round}: the excluded bit was claimed anyway (word {after:#b})"
            );
            assert_eq!(
                after.count_ones(),
                2,
                "round {round}: exactly one of the two claimable bits must go (word {after:#b})"
            );
        }
    }

    /// `KE16-DESIGN-A.md` §5: under A5 the DISPATCHER's pushes take the
    /// idle-keyed arm too — "the idle-keyed branch precedes the lane test" — so
    /// each ready system of the ECS frame path lands on a distinct parked worker
    /// with a targeted wake, instead of going to `injector_global` the way it
    /// does under A1/A1-fifo/A2/A3.
    ///
    /// Why this needs its own row. Every other A5 observation in this crate is
    /// taken from a spawn made INSIDE a worker body
    /// (`a5_every_claimed_idle_bit_is_unparked_and_every_task_completes`), and
    /// a mis-ordered arm — the claim placed AFTER the `worker_lane_for` test
    /// rather than before it, or gated on `Some(lane)` — passes every one of
    /// them: the worker route still claims. The dispatcher route is the ONLY
    /// caller for which the two orders differ, and it is the route
    /// `empty_schedule_control` and `ke16_par_iter_in_system` measure. Without
    /// this test the A5 frame-path number could be produced by a build in which
    /// the frame path never takes the A5 arm at all, and the witness would still
    /// read `a5` — the quiet mislabelling `KE16-DESIGN.md` §4 forbids.
    ///
    /// Anti-vacuity: the claim counter must MOVE. `claimed == unparked` alone is
    /// satisfied by `0 == 0`, which is exactly what a dispatcher push that took
    /// the fallback arm would leave behind.
    #[test]
    #[cfg(feature = "ke16-a5")]
    fn a5_dispatcher_route_push_takes_the_idle_keyed_arm_too() {
        use core::sync::atomic::AtomicU32;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        const WORKERS: usize = 4;
        const TASKS: u32 = 64;
        // Three throws of the same shape, not three chances at a scheduling
        // coincidence: an attempt is only taken once the whole pool is observed
        // parked, and the claim is then made by the push itself. A retry exists
        // only for the case where a spurious `park()` return emptied the mask
        // between the observation and the first push.
        const ATTEMPTS: usize = 3;

        let pool = crate::ThreadPoolBuilder::new().num_threads(WORKERS).build();
        let mut established = false;
        let mut claimed = 0u64;

        for _ in 0..ATTEMPTS {
            // The dispatcher waits for every worker to reach its untimed park,
            // so the pushes below meet a full idle mask.
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut parked = 0;
            while Instant::now() < deadline {
                parked = pool.parked_mask().count_ones() as usize;
                if parked == WORKERS {
                    break;
                }
                std::thread::yield_now();
            }
            if parked != WORKERS {
                continue;
            }
            established = true;

            let done = Arc::new(AtomicU32::new(0));
            for _ in 0..TASKS {
                let done = Arc::clone(&done);
                // THE SUBJECT: a push issued on the dispatcher thread, where
                // `worker_lane_for` answers `None`.
                pool.spawn(move || {
                    done.fetch_add(1, Ordering::AcqRel);
                });
            }

            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline && done.load(Ordering::Acquire) != TASKS {
                std::thread::yield_now();
            }
            assert_eq!(
                done.load(Ordering::Acquire),
                TASKS,
                "every dispatcher-pushed task must run"
            );

            claimed = pool.inner.a5_claimed.load(Ordering::Relaxed);
            if claimed > 0 {
                break;
            }
        }

        assert!(
            established,
            "the shape was never established: the pool never reached {WORKERS} parked workers, so \
             no push could have met a non-empty idle mask"
        );
        assert!(
            claimed > 0,
            "A5 §5: a push made on the DISPATCHER thread against a fully parked pool did not take \
             the idle-keyed arm — the claim counter never moved, so the frame path is running \
             A2's placement while the witness reads `a5`"
        );
        assert_eq!(
            claimed,
            pool.inner.a5_unparked.load(Ordering::Relaxed),
            "A5-1 on the dispatcher route: a claimed idle bit must always be unparked"
        );
    }

    /// A1's PLACEMENT receipt: a spawn made from inside a worker body lands on
    /// THAT worker's own registered deque (`KE16-DESIGN-A.md` §1.2), and not in
    /// `injector_global`.
    ///
    /// No other row in the crate asserts the placement. The `tls` rows observe
    /// the PREDICATE — `worker_lane_for_is_some_on_a_registered_worker` does
    /// read the production deposit through it — and the deposit guard's own
    /// contract; a [`push_task`] whose `Some(lane)` branch pushed to the global
    /// injector anyway would leave every one of them green, and so would the
    /// occupancy harness, because A3's placement reaches W too. That is the
    /// whole point of A3 being the control, and it is why such a build would be
    /// A3 wearing A1's label with `KE16_A` still `"a1"` and `KE16_EXPECT`
    /// certifying the run (`KE16-DESIGN.md` §4).
    ///
    /// Measured against the second mutation discussed in `tls`'s
    /// `worker_deque_deposit_publishes_the_pair_and_clears_it_on_drop` — the
    /// production deposit bound as `let _ = ...`, so the guard drops at the end
    /// of its own statement and the lane is `None` for the whole worker loop —
    /// this row reports 0b00 while that one stays green.
    ///
    /// ONE worker, so the receipt is deterministic rather than racy: no sibling
    /// exists to steal the pushed task between the push and the length read.
    #[test]
    #[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
    fn a1_a_spawn_from_a_worker_body_lands_on_its_own_deque() {
        use core::sync::atomic::AtomicU32;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let pool = crate::ThreadPoolBuilder::new().num_threads(1).build();
        // u32::MAX = the outer body never ran; otherwise bit 0 = the worker's
        // own deque grew by exactly one, bit 1 = `injector_global` — the
        // fall-through destination — stayed empty. Required: 0b11.
        let receipt = Arc::new(AtomicU32::new(u32::MAX));
        let ran = Arc::new(AtomicU32::new(0));

        let pool_cl = Arc::clone(&pool);
        let receipt_cl = Arc::clone(&receipt);
        let ran_cl = Arc::clone(&ran);
        // Fire-and-forget, so the body is guaranteed to run ON the worker and
        // not on this thread.
        pool.spawn(move || {
            let inner: &PoolInner = &pool_cl.inner;
            // A missing lane is a RESULT (the 0b00 receipt), not an `expect`: a
            // panic inside a worker body aborts the process
            // (`abort_on_task_panic`), and an abort would take the rest of the
            // binary's tests with it and print as a crash rather than as this
            // assertion.
            let bits = match tls::worker_lane_for(inner) {
                Some(lane) => {
                    // D5 (`KE16-DESIGN-A.md` §1.3): every `&Worker` minted from
                    // the TLS slot is consumed by ONE method call in its own
                    // statement, and no task body runs between the two reads.
                    let before = lane.deque().len();
                    // THE SUBJECT: a production spawn from inside a worker body.
                    pool_cl.spawn(move || {
                        ran_cl.fetch_add(1, Ordering::Relaxed);
                    });
                    let after = lane.deque().len();
                    u32::from(after == before + 1)
                        | (u32::from(inner.injector_global.is_empty()) << 1)
                }
                None => 0,
            };
            receipt_cl.store(bits, Ordering::Release);
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && receipt.load(Ordering::Acquire) == u32::MAX {
            std::thread::yield_now();
        }
        assert_eq!(
            receipt.load(Ordering::Acquire),
            0b11,
            "A1 §1.2: a spawn from a worker body must land on THAT worker's own registered deque              (bit 0 = the deque grew by one, bit 1 = `injector_global` stayed empty; 0b00 = the              worker had no lane at all, i.e. the deque deposit never reached the loop; u32::MAX =              the body never ran)"
        );

        // Placement is a fix only if the task is still drained: the same worker
        // pops it at stage 2 once the outer body returns.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && ran.load(Ordering::Relaxed) == 0 {
            std::thread::yield_now();
        }
        assert_eq!(
            ran.load(Ordering::Relaxed),
            1,
            "the task placed on the worker's own deque was never executed"
        );
    }

    /// KE16 W-b, the push gate's ONE behavioural claim: a push into a
    /// destination that already held more than one task issues no wake at all.
    ///
    /// The base build's counterpart
    /// (`wake_after_push_wakes_on_every_push_in_the_base_build`) makes the same
    /// call with the same argument and requires the opposite outcome, so the two
    /// together are what the feature switch means.
    #[cfg(feature = "ke16-w-gate")]
    #[test]
    fn wake_after_push_is_silent_when_the_destination_already_held_two() {
        let q = QuiescedIdle::new(4);
        q.set_idle(0b0011);
        wake_after_push(q.inner(), 2);
        assert_eq!(
            q.idle(),
            0b0011,
            "the <=1 gate must not even read the idle mask for a pre-push length of 2; a \
             cleared bit here means a claim was made, and the wake this candidate exists to \
             remove was issued after all"
        );
    }

    /// The other side of the same claim: the 0->1 and 1->2 transitions DO wake.
    /// The 1->2 case is the whole reason the threshold is `<= 1` and not
    /// "was empty" — it covers a thief that took the last task between the
    /// spawner's length read and its push.
    #[cfg(feature = "ke16-w-gate")]
    #[test]
    fn wake_after_push_wakes_at_both_gated_transitions() {
        for pre_len in [0usize, 1] {
            let q = QuiescedIdle::new(4);
            q.set_idle(0b0011);
            wake_after_push(q.inner(), pre_len);
            assert_eq!(
                q.idle().count_ones(),
                1,
                "a push with pre-length {pre_len} must claim exactly one parked worker"
            );
        }
    }

    /// KE16 W-b's cascade, and the property its self-exclusion exists for: a
    /// thief left with residue wakes a SIBLING, never itself.
    ///
    /// The caller's own bit is set here because that is where the cascade is
    /// reached from in production — inside the post-`mark_idle` re-poll, before
    /// `unmark_idle`. An unmasked claim there can take the caller's own bit
    /// (which bit is picked depends on the rotor), spending the pool's single
    /// wake on the thread that issued it and breaking the activation chain at
    /// hop one.
    #[cfg(feature = "ke16-w-gate")]
    #[test]
    fn wake_after_residue_hands_the_cascade_to_a_sibling_not_to_itself() {
        let q = QuiescedIdle::new(4);
        let local: Worker<Task> = Worker::new_lifo();
        local.push(Task::new_detached(|| {}));

        // Every rotor offset, because the claim's start is a rotating counter
        // and the excluded bit must be refused from all of them.
        for start in 0..64u64 {
            q.set_idle(0b0011);
            q.inner().wake_rotor.store(start, Ordering::Release);
            wake_after_residue(q.inner(), 0, &local);
            let idle = q.idle();
            assert_eq!(
                idle & 0b0001,
                0b0001,
                "rotor {start}: the cascading thief's own bit was claimed — the wake was spent \
                 on the thread that issued it"
            );
            assert_eq!(
                idle, 0b0001,
                "rotor {start}: the cascade must claim exactly the one claimable sibling"
            );
        }
    }

    /// A thief with no residue cascades nothing: the rule is keyed on the
    /// thief's own queue state, which is what keeps a batch that took
    /// everything from issuing a wake nobody needs.
    #[cfg(feature = "ke16-w-gate")]
    #[test]
    fn wake_after_residue_is_silent_when_the_thief_kept_nothing() {
        let q = QuiescedIdle::new(4);
        let local: Worker<Task> = Worker::new_lifo();
        q.set_idle(0b0011);
        wake_after_residue(q.inner(), 0, &local);
        assert_eq!(
            q.idle(),
            0b0011,
            "an empty deque is no 0->k transition; no wake is owed and none must be issued"
        );
    }

    /// KE16 base-pool, the axis's own NEGATIVE claim: apart from W-a, the
    /// default build's spawn path is today's. A spawn from inside a worker body
    /// still lands in `injector_local[wid]` — not on that worker's own deque
    /// (A1), not in `injector_global` (A3), not on a claimed sibling (A5).
    ///
    /// Why the base pass needs a falsifier of its own: every other instrument in
    /// the tree either measures an A arm or is the red-first occupancy gate,
    /// which is `#[ignore]`d in exactly this build. So "the default build is
    /// behaviourally identical apart from W-a" was, until this row, checked by
    /// nothing — and the one thing W-a moves (the wake decision) sits on the
    /// same code path as the placement it must not touch.
    ///
    /// Compiled in the builds whose placement rule IS `injector_local[wid]`:
    /// the default and pure `ke16-a2`, which changes the SCAN set rather than
    /// the push. Deleted with the feature gates at Step App.
    ///
    /// ONE worker, so the receipt is deterministic: no sibling exists to drain
    /// the slot between the push and the length read.
    #[test]
    #[cfg(not(any(
        feature = "ke16-a1",
        feature = "ke16-a1-fifo",
        feature = "ke16-a3",
        feature = "ke16-a5"
    )))]
    fn a0_a_spawn_from_a_worker_body_still_lands_in_its_local_injector() {
        use core::sync::atomic::AtomicU32;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let pool = crate::ThreadPoolBuilder::new().num_threads(1).build();
        // u32::MAX = the outer body never ran; otherwise bit 0 = this worker's
        // own local injector grew by exactly one, bit 1 = `injector_global`
        // stayed empty. Required: 0b11.
        let receipt = Arc::new(AtomicU32::new(u32::MAX));
        let ran = Arc::new(AtomicU32::new(0));

        let pool_cl = Arc::clone(&pool);
        let receipt_cl = Arc::clone(&receipt);
        let ran_cl = Arc::clone(&ran);
        // Fire-and-forget, so the body is guaranteed to run ON the worker.
        pool.spawn(move || {
            let inner: &PoolInner = &pool_cl.inner;
            let wid = tls::current_worker_id() as usize;
            // A worker id outside the slot array is a RESULT (the 0b00
            // receipt), not an `expect`: a panic inside a task body aborts the
            // process and would print as a crash rather than as this assertion.
            let bits = if wid < inner.injector_local.len() {
                let before = inner.injector_local[wid].len();
                // THE SUBJECT: a production spawn from inside a worker body.
                pool_cl.spawn(move || {
                    ran_cl.fetch_add(1, Ordering::Relaxed);
                });
                let after = inner.injector_local[wid].len();
                u32::from(after == before + 1)
                    | (u32::from(inner.injector_global.is_empty()) << 1)
            } else {
                0
            };
            receipt_cl.store(bits, Ordering::Release);
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && receipt.load(Ordering::Acquire) == u32::MAX {
            std::thread::yield_now();
        }
        assert_eq!(
            receipt.load(Ordering::Acquire),
            0b11,
            "the default build must place a worker-spawned task in `injector_local[wid]` \
             (bit 0 = that slot grew by one, bit 1 = `injector_global` stayed empty; 0b00 = the \
             worker id was out of range; u32::MAX = the body never ran)"
        );

        // Placement is only unchanged if the task is still drained: the same
        // worker polls its local injector at stage 1 once the outer body returns.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && ran.load(Ordering::Relaxed) == 0 {
            std::thread::yield_now();
        }
        assert_eq!(
            ran.load(Ordering::Relaxed),
            1,
            "the task placed in the worker's local injector was never executed"
        );
    }
}
