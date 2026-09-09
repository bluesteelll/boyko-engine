//! Worker thread loop + idle-bitset primitives.
//!
//! Plan §4.3. The loop polls three sources in order
//! (`own_deque → global_injector → sibling_steal`), then backs off and parks.
//! The post-`mark_idle` re-poll is load-bearing against Race C (plan §13.4.1)
//! — we keep it explicit in the code with a comment, never collapse it into
//! the pre-park branch.
//!
//! A worker's own spawns land on its OWN registered deque, whose stealer every
//! sibling scans, so no per-worker injector is ever fed and the loop has no
//! stage that polls one. `push_task`, at the bottom of this file, is the write
//! side of that rule; these stages are the matching read side.

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
    // Publish the deque to this thread's `push_task`. The guard clears the slot
    // on every exit path; it is declared AFTER `deque` (a parameter) so it drops
    // BEFORE `deque` does. The pointer is a raw borrow of the place — no
    // reference to `deque` is created here (discipline D5, `KE16-DESIGN-A.md`
    // §1.7 — not the diagnostics rung named twenty lines above).
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

        // 1. Own deque — own spawned tasks, plus whatever this worker stole
        //    (FIFO owner end, fixed at construction).
        if let Some(t) = deque.pop() {
            run_task(t);
            continue;
        }

        // 2. Global injector — dispatcher-pushed tasks.
        if let Some(t) = pop_global_injector(inner.as_ref(), worker_id, &deque) {
            run_task(t);
            continue;
        }

        // 3. Sibling steal.
        if let Some(t) = try_steal_random(inner.as_ref(), worker_id, &deque, &mut rng) {
            run_task(t);
            continue;
        }

        // 4. Backoff escalation, then mark_idle + re-poll + park.
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
    if let Some(t) = local.pop() {
        return Some(t);
    }
    if let Some(t) = pop_global_injector(inner, worker_id, local) {
        return Some(t);
    }
    try_steal_random(inner, worker_id, local, rng)
}

/// Drain a batch from the global injector into the local deque.
///
/// `pub(crate)` for the KE16 B1 worker joiner, which reaches the injector
/// through this helper so the batch lands in its own REGISTERED deque and the
/// residue stays stealable (`KE16-DESIGN-B.md` §2.2 step 2).
///
/// `worker_id` names the caller's own lane, which the post-steal residue
/// decision ([`wake_after_residue`]) must never claim.
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
/// Each visited sibling is probed ONCE, at its registered deque: that deque is
/// where a worker's own spawns land, so scanning the deques is what makes a
/// worker-spawned wave reachable by anyone else. Self is skipped — a worker
/// pops its own deque at stage 1 of its loop.
///
/// `pub(crate)` for the KE16 B1 worker joiner, whose sweep IS this one (App-3,
/// `KE16-DESIGN-B.md` §2.2 step 3): random start, self skipped by the lane's own
/// `wid`, batch into the caller's registered deque.
///
/// A successful batch goes through the residue decision
/// ([`wake_after_residue`]): whatever this thief did not take is now in its OWN
/// registered deque.
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
/// placed for the purpose). The A1 owner push is a plain store, so without this
/// fence it would have no barrier at all.
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
/// `exclude` masks bits the caller must never claim. Three production sites
/// pass a non-zero mask, all of them the B1 joiner's own `self_bit`
/// (`scope.rs`, `join_on_worker`), in two shapes:
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
/// The unit tests at the foot of this file drive the non-zero shape from both
/// sides: once against real parked workers, once on a fixture that owns the
/// idle word and can therefore assert the mask it leaves behind.
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
/// **The empty-candidate input is answered, not asserted.** The contract is
/// total on purpose: a core that answered an exhausted candidate mask with
/// `Some(id)` — which a CAS of `observed` against an unchanged `observed` does
/// trivially — would charge an unpark to a worker whose bit was never set: an
/// out-of-bounds `inner.workers[id]` on a pool smaller than `id`, or a silently
/// wasted wake on a larger one. Any claimer that reads its mask once and then
/// loops (a bounded fan-out over a mask other threads are clearing, a claim
/// from a mask read earlier still) presents that input legitimately rather than
/// by a bug, which is why there is deliberately no `debug_assert` on it: the
/// assert would make the same value both a documented `None` and a debug-mode
/// panic.
///
/// **Why the two masks are separate parameters.** They matter only to a caller
/// that excludes something: the KE16 B1 joiner's pre-park wake. Folding them —
/// CASing against `observed & !exclude` — makes the CAS unsatisfiable whenever
/// an excluded bit is actually set, because then that value is not the word in
/// memory; the caller's retry loop re-derives the same value and spins forever.
/// That caller issues its wake with its own bit SET (between `mark_idle` and its
/// park), so the folded form would hang exactly where the parameter is needed.
/// See [`unpark_one_idle_excluding`]'s caller loop and the deviation recorded
/// against `KE16-DESIGN-W.md` §1.1.
///
/// One signature for every claimer, so the loom transcriptions have a single
/// shape to mirror.
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
    // `isolate_lowest_one()` rather than `rotated & rotated.wrapping_neg()`.
    // The two are the SAME operation — the standard library defines the method
    // as exactly that expression, and both lower to one `blsi` under this
    // tree's `x86-64-v3` baseline — so this is a rename of an idiom, not a
    // change to the wake path's arithmetic. Flagged by
    // `clippy::manual_isolate_lowest_one`, new in clippy 0.1.98; the method
    // itself is stable well before the pinned 1.97, probed on that toolchain
    // before the edit, so it is not a forward reference to the newer compiler.
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

/// The wake decision after a push, shared by every placement.
///
/// The wake is unconditional. This is the ONE wake-decision point for the PUSH
/// side: no placement carries a wake rule of its own, and the StoreLoad barrier
/// lives one level down in [`unpark_one_idle_excluding`], so a decision that
/// issues no wake pays neither the fence nor the mask load.
#[inline]
pub(crate) fn wake_after_push(inner: &PoolInner, pre_len: usize) {
    if wake_is_due(pre_len) {
        unpark_one_idle(inner);
    }
}

/// The push side's wake rule: is a wake due after a push whose destination held
/// `pre_len` tasks? It always is — every push wakes, and `pre_len` is carried
/// down the push path without any rule reading it.
///
/// [`wake_after_push`] answers the question HERE rather than re-deciding it, so
/// the rule has exactly one body.
#[inline]
pub(crate) fn wake_is_due(pre_len: usize) -> bool {
    let _ = pre_len;
    true
}

/// The wake decision after a BATCH STEAL: none is owed, so none is taken.
///
/// A thief that batch-stole into its own registered deque and still holds
/// residue there has taken that deque from 0 to k without any push touching it.
/// The pool does not wake for that: every PUSH wakes ([`wake_after_push`]) and
/// no steal does, so this is a no-op at both of its call sites,
/// [`pop_global_injector`] and [`try_steal_random`].
///
/// `worker_id` is the caller's own lane, and it is a parameter because of where
/// both call sites are reached from: `worker_main`'s post-`mark_idle` re-poll,
/// and the B1-P joiner's, where the CALLER's own idle bit is still set. Any
/// wake issued from here would have to exclude that bit
/// ([`unpark_one_idle_excluding`]) or it would spend the pool's single wake on
/// the thread that issued it.
#[inline]
pub(crate) fn wake_after_residue(inner: &PoolInner, worker_id: u32, local: &Worker<Task>) {
    let _ = (inner, worker_id, local);
}

/// Whether a push is going to READ the destination's pre-push length, resolved
/// by the type system rather than by a branch.
///
/// The length is not free to compute. `Injector::len` is a stable-tail retry
/// loop of three `SeqCst` loads of `head`/`tail` — the lines every pusher and
/// thief touch — and `Worker::len` is a Relaxed `back` load plus a `SeqCst` load
/// of `front`, the line every thief CASes. Neither is dead code the compiler may
/// drop, so the only way not to pay is not to issue the call: an implementor
/// with no call in its body has nothing for the optimiser to remove. Static
/// dispatch on a ZST — no branch at any optimisation level, no code at run
/// time.
///
/// Nothing reads the pre-push length today: the wake decision is unconditional
/// ([`wake_is_due`]). [`ProbeLen`] is therefore the only implementor and it
/// answers `0` from both methods without issuing a probe.
trait LenProbe {
    /// The injector's length before a push, or `0` when nobody will read it.
    fn injector(inj: &crossbeam_deque::Injector<Task>) -> usize;

    /// The owner deque's length before a push, or `0` when nobody will read it.
    fn deque(local: &Worker<Task>) -> usize;
}

/// The implementor every push is instantiated with.
struct ProbeLen;

impl LenProbe for ProbeLen {
    #[inline]
    fn injector(inj: &crossbeam_deque::Injector<Task>) -> usize {
        // No reader for the length, so no probe: the pool must not pay a
        // three-`SeqCst`-load retry loop on the injector's head/tail for a
        // value nothing reads.
        let _ = inj;
        0
    }

    #[inline]
    fn deque(local: &Worker<Task>) -> usize {
        // No reader for the length, so no probe: the pool must not pay a
        // `SeqCst` load of the contended `front` line for a value nothing
        // reads.
        let _ = local;
        0
    }
}

/// Push into the global injector, answering the pre-push length for the wake
/// decision the caller owes. The destination for a push whose origin is not a
/// lane of `inner`.
#[inline]
fn push_global_no_wake<P: LenProbe>(inner: &PoolInner, task: Task) -> usize {
    let pre_len = P::injector(&inner.injector_global);
    inner.injector_global.push(task);
    pre_len
}

/// Push onto the calling worker's OWN registered Chase-Lev deque (KE16 A1),
/// answering the pre-push length for the wake decision the caller owes.
///
/// This is what closes defect A by locality rather than by a shared queue:
/// `inner.stealers[lane.wid]` is the stealer of exactly this deque, so the wave
/// is reachable by every sibling's `try_steal_random` and by any joiner, while
/// the push itself costs no RMW at all.
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
pub(crate) fn push_task(inner: &PoolInner, task: Task) {
    if let Some(pre_len) = push_task_no_wake(inner, task) {
        wake_after_push(inner, pre_len);
    }
}

/// Place a task exactly as [`push_task`] does but take NO wake decision,
/// answering what the caller owes: `Some(pre_len)` — the destination queue's
/// pre-push length, to be handed to [`wake_after_push`]. The answer is an
/// `Option` because a placement is permitted to carry its own targeted wake and
/// leave nothing for the caller to decide; no placement here does, so the answer
/// is always `Some`.
#[inline]
pub(crate) fn push_task_no_wake(inner: &PoolInner, task: Task) -> Option<usize> {
    place_task::<ProbeLen>(inner, task)
}

/// The placement itself.
///
/// `P` says whether the caller will read the pre-push length — one body, one
/// destination rule, one place where placement lives.
#[inline]
fn place_task<P: LenProbe>(inner: &PoolInner, task: Task) -> Option<usize> {
    Some(match tls::worker_lane_for(inner) {
        Some(lane) => push_on_lane_no_wake::<P>(inner, lane, task),
        None => push_global_no_wake::<P>(inner, task),
    })
}

/// SplitMix64 mixer (Sebastiano Vigna, 2014). Used as a seed generator
/// for the per-worker [`XorShift64Star`] PRNG so that workers spawned
/// with adjacent ids don't start in adjacent PRNG states.
///
/// `pub(crate)` for the KE16 B1 joiner, which seeds its sweep from the joining
/// lane and the scope's address.
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
    /// less. No in-crate caller reaches this input: both check the mask first.
    /// This row is what keeps the contract total anyway, for the claimer that
    /// reads its mask once and then loops.
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
    /// shape the B1 joiner's pre-park wake has (it issues its wake between
    /// `mark_idle` and its own park, holding its own bit). This is the only
    /// exercise of a non-zero `exclude` against a LIVE pool.
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
    /// the caller's own. This is the pre-park-wake shape the `exclude`
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

    /// The push side's W semantics: the wake is UNCONDITIONAL. `pre_len` is
    /// carried down the push path but reaches no rule, so a pre-push length far
    /// above any plausible threshold must still wake a parked worker.
    #[test]
    fn wake_after_push_wakes_on_every_push() {
        let q = QuiescedIdle::new(4);
        q.set_idle(0b0011);
        wake_after_push(q.inner(), 999);
        assert_eq!(
            q.idle().count_ones(),
            1,
            "the wake decision is unconditional; a large pre-push length must not gate it"
        );
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
    /// The B1 joiner the `exclude` parameter exists for issues its wake from
    /// inside its own pre-park sequence; a rotor RMW ahead of that test would
    /// charge the shared line to a decision that never wakes anyone.
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
    /// production caller presents today — the contract is total, and this is
    /// what holds it total.
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
            // the set bits -- the pre-park-wake shape, where the excluded bit
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

    /// A1's PLACEMENT receipt: a spawn made from inside a worker body lands on
    /// THAT worker's own registered deque (`KE16-DESIGN-A.md` §1.2), and not in
    /// `injector_global`.
    ///
    /// No other row in the crate asserts the placement. The `tls` rows observe
    /// the PREDICATE — `worker_lane_for_is_some_on_a_registered_worker` does
    /// read the production deposit through it — and the deposit guard's own
    /// contract; a [`push_task`] whose `Some(lane)` branch pushed to the global
    /// injector anyway would leave every one of them green, and so would the
    /// occupancy harness, because a global-injector placement reaches W workers
    /// too. This row is the only thing that separates the two: it asserts WHICH
    /// queue the task landed in, not merely that some worker eventually ran it.
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
        // pops it at stage 1 of its loop once the outer body returns.
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
}
