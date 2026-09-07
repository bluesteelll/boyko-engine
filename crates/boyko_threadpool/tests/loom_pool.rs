//! Phase 9.1 W3a — loom exhaustive models of the pool's own synchronization
//! primitives (plan D2 / D3 / §4).
//!
//! These models drive the **real** production methods (C1) over loom's
//! model-checked atomics, abstracting only the crossbeam-deque *transport*
//! (loom-opaque, Coq-verified upstream) behind a trivial loom-visible toy queue.
//! Concretely, via `boyko_threadpool::loom_exports`:
//!   - **M1** calls the real `ScopeShared::{register_task, complete_task,
//!     is_drained}` (`scope.rs`) — wrapped 1:1 by `LoomScopeShared` — so loom
//!     observes the genuine `AcqRel` / `Acquire` orderings. (Phase 9.2
//!     Candidate U: `complete_task` unparks UNCONDITIONALLY before its
//!     `fetch_sub`, so there is no `prev == 1` branch; M1 proves no lost wakeup
//!     via the total-ordered `fetch_sub` RMW chain driving `is_drained()` to 0
//!     in every interleaving. The joiner re-polls with `yield_now` as the
//!     TRANSPORT-AGNOSTIC shape, not because the pinned loom drops the token:
//!     loom 0.7.2 stores `Runnable { unparked: true }` in `set_unparked` and
//!     `rt::park` consumes it, so an unpark issued before the matching park IS
//!     persisted. The yield re-poll models the production `park_timeout`
//!     backstop and proves the property without relying on the token at all.)
//!   - **M2 / M2b** call the real `mark_idle` / `unmark_idle` (`worker.rs`) over
//!     a loom `AtomicU64`.
//!   - **M1c** (`ke16-w-count`) calls the same real `ScopeShared` methods as M1,
//!     but through `LoomScopeShared::new_worker_joined` — a non-null count-gate
//!     target — so `complete_task` takes its W-d′ arm: the decrement FIRST, the
//!     unpark only on `prev == 1`, aimed at memory the joiner cannot free. Its
//!     joiner parks FOR REAL, because there the token is the property.
//!   - **M4a / M4b / M4c** (`ke16-w-gate`) drive the real `mark_idle` /
//!     `unmark_idle` / `publish_fence` and loom's real `park` / `unpark` under
//!     W-b's two mechanisms: the ≤1 push gate and the self-excluding residue
//!     cascade. Every row of that family arms every oracle; M4c is a
//!     `should_panic` row that RECORDS a design finding (see its header) rather
//!     than a calibration copy.
//!
//! ## Fidelity note (the one transcription)
//!
//! `unpark_one_idle_excluding` takes `&PoolInner` (crossbeam-coupled, not
//! loom-buildable), so its rotate + `compare_exchange_weak` *claim* core cannot
//! be invoked over the loom shim directly. M2 / M2b / M2c / M4 therefore
//! transcribe **only that claim core** here (`claim_one_idle_bit`, mirroring
//! production's `worker::claim_one_idle`): the same single CAS attempt with
//! `Ordering::AcqRel` success / `Ordering::Acquire` failure, the same
//! rotate-by-`start` lowest-bit pick, and the same caller loop that re-reads the
//! mask when the CAS loses. The publish/clear sides (`mark_idle` /
//! `unmark_idle`) and — since KE16 W-a — the producer's `publish_fence` are the
//! real shared production code. The claim core is the sole model component that
//! is not literal shared code; it is flagged per the plan's D3 / §4 allowance.
//!
//! ## Run — the recipe that WORKS on this box, and the one that does not
//!
//! ```bash
//! # Build (the emitted path is printed by this command):
//! cargo --config 'target.x86_64-pc-windows-gnu.rustflags=["-C","target-cpu=x86-64-v3","--cfg","loom"]' \
//!   test -p boyko-threadpool --test loom_pool --features ke16-w-gate,ke16-w-count --no-run
//! # Run, per model, because the wall times are minutes and a filter that
//! # matches nothing exits 0:
//! LOOM_MAX_PREEMPTIONS=3 ./target/debug/deps/loom_pool-*.exe --test-threads=1 --exact <name>
//! ```
//!
//! ⚠ **`RUSTFLAGS="--cfg loom" cargo test …` — the spelling this header carried
//! until 2026-09-03, and the one `KE16-DESIGN-MEASUREMENT.md` inherited — does
//! not work here and must not be re-derived.** Measured 2026-09-02: the binary
//! it links dies at startup with `STATUS_ACCESS_VIOLATION` (0xC0000005) before
//! libtest prints `running N tests`, `--list` included. That reading was then
//! written up in this header as "loom does not run on this box", which converted
//! every model below into an argument and every absent loom row into an excused
//! one. It is a RECIPE fault, not a box fault: `RUSTFLAGS` REPLACES
//! `[target.<triple>].rustflags` rather than merging with it (`.cargo/config
//! .toml`'s `x86-64-v3` baseline), so that command builds a differently
//! configured tree; `cargo --config` sets the same key and therefore composes.
//!
//! **Readings, this checkout, 2026-09-03, `LOOM_MAX_PREEMPTIONS=3`,
//! `--features ke16-w-gate,ke16-w-count`, one model per process:**
//!
//! | Model | Colour | Wall |
//! |---|---|---|
//! | `loom_m1c_count_gated_completion_wakes_the_parked_worker_joiner` | ok | 0.40 s |
//! | `loom_m4a_gate_race_no_lost_wake` | ok | 95.65 s |
//! | `loom_m4b_decline_is_safe_when_the_pusher_drains_and_the_cascade_never_claims_self` | ok | 385.55 s |
//! | `loom_m4_calibration_empty_gate_is_lost` | should-panic ok (`M4: lost wake`) | 36.25 s |
//! | `loom_m4_calibration_no_self_exclusion_claims_self` | should-panic ok (`cascade claimed self`) | 221.36 s |
//! | `loom_m4c_stale_snapshot_strands_a_task_when_the_pusher_is_not_a_lane` | should-panic ok (`M4: lost wake`) | 40.71 s |
//!
//! `--list` prints all 11 tests and exits 0. An absent loom row in a results
//! file is therefore an UNRUN row with no excuse attached, and every colour
//! asserted in a model header below is a reading unless that header says
//! otherwise.
#![cfg(loom)]

use std::collections::VecDeque;
// M4's witness counters must survive ACROSS loom executions, so they are `std`
// atomics deliberately: loom's own atomics are reset with the modelled state at
// every execution and could not answer "was this branch ever reached".
#[cfg(feature = "ke16-w-gate")]
use std::sync::atomic::{AtomicUsize as StdAtomicUsize, Ordering as StdOrdering};

use loom::sync::{Arc, Mutex};
use loom::thread;

#[cfg(feature = "ke16-w-count")]
use boyko_threadpool::loom_exports::WakeHandle;
#[cfg(feature = "ke16-w-gate")]
use boyko_threadpool::loom_exports::sync::AtomicUsize;
#[cfg(feature = "ke16-w-gate")]
use boyko_threadpool::loom_exports::sync::thread::Thread;
use boyko_threadpool::loom_exports::sync::{AtomicBool, AtomicU64, Ordering, fence};
use boyko_threadpool::loom_exports::{LoomScopeShared, mark_idle, publish_fence, unmark_idle};

// =========================================================================
// M1 — fork/join no-lost-wakeup.
//
// Drives the real `ScopeShared::{register_task, complete_task, is_drained}`.
// N = 2 tasks. Main registers both, pushes 2 toy tasks into a loom queue, then
// runs the production-shaped join-wait loop (`is_drained()` poll + `park()`).
// Two loom task threads each pop one toy task and call `complete_task()`. A
// shadow `completed` counter (a plain loom `AtomicU64`) sits beside the real
// `pending`.
//
// Invariants (plan §4 / table):
//   - the join loop exits  ==>  `is_drained()` (real `pending == 0`);
//   - `completed == N` at exit (no task outlived join → the transmute premise);
//   - terminates (a lost wakeup in `complete_task`'s `prev==1` branch = a loom
//     deadlock report).
// =========================================================================

#[test]
fn loom_m1_fork_join_no_lost_wakeup() {
    loom::model(|| {
        const N: usize = 2;

        // Real ScopeShared (waker = the main / dispatcher loom thread).
        let shared = Arc::new(LoomScopeShared::new(thread::current()));
        // Toy transport replacing the crossbeam deque.
        let queue: Arc<Mutex<VecDeque<usize>>> = Arc::new(Mutex::new(VecDeque::new()));
        // Shadow completion counter beside the real atomic.
        let completed = Arc::new(AtomicU64::new(0));

        // Register N tasks (real fetch_add AcqRel) and enqueue N toy items.
        {
            let mut q = queue.lock().unwrap();
            for i in 0..N {
                shared.register_task();
                q.push_back(i);
            }
        }

        // Spawn N worker threads. Each pops one toy task then calls the real
        // `complete_task()` (fetch_sub AcqRel + prev==1 unpark).
        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let shared_cl = Arc::clone(&shared);
            let queue_cl = Arc::clone(&queue);
            let completed_cl = Arc::clone(&completed);
            handles.push(thread::spawn(move || {
                let item = { queue_cl.lock().unwrap().pop_front() };
                if item.is_some() {
                    // Account BEFORE the real completion so that, at the moment
                    // the joiner observes drained, `completed` is already final.
                    completed_cl.fetch_add(1, Ordering::AcqRel);
                    shared_cl.complete_task();
                }
            }));
        }

        // Production-shaped join wait: poll the REAL `is_drained()`, re-polling
        // via `yield_now` until drained. Phase 9.2 Candidate U makes
        // `complete_task` unpark BEFORE its `fetch_sub`. The pinned loom 0.7.2
        // DOES persist such a token — `set_unparked` stores
        // `Runnable { unparked: true }` and `rt::park` consumes it — so this
        // re-poll is not a workaround for a missing token. It is the
        // TRANSPORT-AGNOSTIC shape: what production relies on when the token is
        // absent is the `park_timeout` backstop's re-poll, and modelling that
        // re-poll directly proves the property from the total-ordered
        // `fetch_sub` RMW chain alone — `is_drained()` observes 0 in every
        // interleaving — without any wake transport being load-bearing, exactly
        // as M2/M2b/M3 abstract the loom-opaque deque transport. (M1c, which
        // gates the count-gated completion of the W axis, is the model that
        // does use a real `park()`, because there the token IS the property.)
        while !shared.is_drained() {
            thread::yield_now();
        }

        // Invariant: join exited ⟹ pending == 0 (re-assert the real method).
        assert!(shared.is_drained(), "join exited but pending != 0");

        for h in handles {
            h.join().unwrap();
        }

        // Invariant: every task completed before join returned.
        assert_eq!(
            completed.load(Ordering::Acquire),
            N as u64,
            "completed != N at join exit (a task outlived the join wait)"
        );
    });
}

// =========================================================================
// M2 — idle-bitset "Race C" (2 threads: 1 worker + 1 producer).
//
// Drives the real `mark_idle` / `unmark_idle` over a loom `AtomicU64`, with the
// transcribed claim core for the producer's wake. Models the load-bearing
// post-`mark_idle` re-poll of `worker_main`'s backoff/park loop (the poll
// between `mark_idle` and `unmark_idle`): a producer that publishes work and
// then tries to wake must not let the worker end up parked while work is
// visible.
//
// ## Transport-ordering fidelity (the SeqCst fence models the crossbeam injector)
//
// This is the "store-buffer" (SB) litmus pattern: the producer does (publish
// work; read idle); the worker does (mark idle; re-poll work). The lost-wakeup
// window — the worker's re-poll MISSING the work AND the producer's `claim`
// MISSING the idle bit, simultaneously — is, under the C11 model, NOT excluded
// by plain Release/Acquire on each location, nor even by per-op SeqCst on two
// *distinct* locations: an initial model with a Release/Acquire `work` flag AND
// a model with per-op-SeqCst on `work` BOTH reproduced the window
// (`bit_set=true work_present=true was_claimed=false`). That is the textbook SB
// outcome; closing it requires a `fence(SeqCst)` on EACH thread BETWEEN its two
// operations (the canonical SB fix; this is what `std`/`rayon` rely on).
//
// The two halves have DIFFERENT provenance, and since KE16 W-a only one of them
// is modelled.
//
// The PRODUCER's fence is production code: `worker::publish_fence` is the first
// statement of every wake decision, and this model calls it through
// `loom_exports::publish_fence` (C1). It has to be production code because the
// transport under it may publish with a PLAIN STORE — the KE16 A1 arms push
// onto a Chase-Lev deque, and a thief's residue store is plain too, so nothing
// else on that side would be a barrier. Its calibration
// copy below deletes the call and must go red for the model's own lost-wake
// message.
//
// The CONSUMER's fence is still the transport's, and is modelled: the worker's
// post-`mark_idle` re-poll goes through `Injector::steal_batch_and_pop`, whose
// explicit `fence(SeqCst)` sits on the empty-check path (the one the litmus
// needs), and `Stealer::steal_batch_and_pop`'s `epoch::pin`. crossbeam's
// internals are loom-opaque (the deque is out of loom scope by D3/§9, covered by
// the D6 stress test), so the model reintroduces that guarantee as the fence it
// actually is. The idle-bitset ops under test (`mark_idle` / `unmark_idle` / the
// claim CAS) remain REAL production code.
//
// Invariant: the worker never ends parked (`idle` bit still set) with unclaimed
// work present.
// =========================================================================

/// Transcription of production's `worker::claim_one_idle` (the only part of the
/// wake decision that is `&PoolInner`-bound and thus not loom-buildable): ONE
/// `compare_exchange_weak(AcqRel, Acquire)` attempt at the lowest set bit of
/// `observed & !exclude` at or above `start`, wrapping, with `observed` — the
/// whole idle word as the caller read it — as the CAS's expected operand.
/// `Some(id)` = claimed; `None` = nothing was claimed (the CAS lost, or no
/// claimable bit was set) and the caller must re-read the word. The two masks
/// are separate here for the same reason they are separate in production:
/// CASing against the excluded-out value makes the CAS unsatisfiable whenever an
/// excluded bit is set. The empty-candidate early return is production's too —
/// without it a CAS of `observed` against `observed` succeeds trivially and
/// reports a claim of a bit nobody set, which is a lost wake in the shape M4 is
/// built to catch.
fn claim_one_idle_bit(idle: &AtomicU64, observed: u64, exclude: u64, start: u32) -> Option<u32> {
    let candidates = observed & !exclude;
    if candidates == 0 {
        return None;
    }
    let rotated = candidates.rotate_right(start);
    let low = rotated & rotated.wrapping_neg();
    let id = (low.trailing_zeros() + start) % 64;
    let new = observed & !(1u64 << id);
    match idle.compare_exchange_weak(observed, new, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(id),
        Err(_) => None,
    }
}

/// The caller loop of production's `unpark_one_idle_excluding`, minus the
/// `unpark` (the model's workers never actually park) and minus the
/// `publish_fence` prologue, which each caller issues itself so the calibration
/// copy can delete exactly that one call.
///
/// FIX-3 divergence note: production rotates the search START offset (a
/// `Relaxed` `wake_rotor`) to remove the lowest-bit wake bias, but still claims
/// exactly ONE set bit per successful CAS with identical orderings. The property
/// M2/M2b verify — mutual exclusion of the claim + no lost wakeup under Race C —
/// is independent of *which* set bit is picked, so this model fixes
/// `start == 0` as a faithful representative; the rotor carries no data and so
/// introduces no synchronization edge to model.
fn claim_one_idle(idle: &AtomicU64) -> Option<u32> {
    loop {
        let observed = idle.load(Ordering::Acquire);
        if observed == 0 {
            return None;
        }
        // `exclude == 0`: M2/M2b model the producer's wake, which excludes
        // nothing. The non-zero-exclusion shape belongs to the thief-residue
        // cascade and is covered natively by
        // `worker::tests::unpark_one_idle_excluding_claims_a_sibling_while_the_
        // excluded_bit_is_set` until M4 (W axis) models it.
        if let Some(id) = claim_one_idle_bit(idle, observed, 0, 0) {
            return Some(id);
        }
    }
}

#[test]
fn loom_m2_idle_race_c_no_lost_wakeup() {
    loom::model(|| {
        let idle = Arc::new(AtomicU64::new(0));
        let work = Arc::new(AtomicBool::new(false));
        // Did the producer's claim succeed in waking the worker?
        let claimed = Arc::new(AtomicBool::new(false));

        // Producer: publish work, then take the wake decision the way production
        // takes it — the REAL `publish_fence()` (KE16 W-a's StoreLoad barrier,
        // driven here through `loom_exports`, C1) between the publish and the
        // idle load, then the claim.
        let idle_p = Arc::clone(&idle);
        let work_p = Arc::clone(&work);
        let claimed_p = Arc::clone(&claimed);
        let producer = thread::spawn(move || {
            work_p.store(true, Ordering::Release);
            publish_fence(); // production code: worker::publish_fence
            if claim_one_idle(&idle_p).is_some() {
                claimed_p.store(true, Ordering::Release);
            }
        });

        // Worker (id 0): the real Race-C sequence — pre-mark re-poll, mark_idle,
        // POST-mark re-poll (load-bearing), then "park" (modeled as: only park
        // if work is still not visible). The `fence(SeqCst)` between `mark_idle`
        // and the post-mark re-poll is the crossbeam injector steal transport
        // guarantee — the same fence production's `steal_batch_and_pop` carries.
        let idle_w = Arc::clone(&idle);
        let work_w = Arc::clone(&work);
        let worker = thread::spawn(move || {
            // Pre-mark_idle re-poll (Race A window).
            if work_w.load(Ordering::Acquire) {
                return; // grabbed work; never parks
            }
            // Mark ourselves idle (real production fetch_or, Release).
            mark_idle(&idle_w, 0);
            fence(Ordering::SeqCst); // injector steal transport fence
            // POST-mark_idle re-poll — load-bearing against Race C.
            if work_w.load(Ordering::Acquire) {
                // Grabbed work after marking: this arm never parks.
                unmark_idle(&idle_w, 0); // real fetch_and, Release
            }
            // Otherwise we would park here, awaiting the producer's unpark.
        });

        producer.join().unwrap();
        worker.join().unwrap();

        // Invariant: no lost wakeup. The forbidden state is "idle bit still set
        // (worker parked) + work present + the producer's claim never fired".
        // If the bit is set and work is present, the producer MUST have claimed
        // (and will unpark the worker). If the bit is clear, the worker
        // re-polled and grabbed the work itself. Both are correct.
        let bit_set = idle.load(Ordering::Acquire) & 1 != 0;
        let work_present = work.load(Ordering::Acquire);
        let was_claimed = claimed.load(Ordering::Acquire);
        if bit_set && work_present {
            assert!(
                was_claimed,
                "M2: lost wake — worker parked (idle bit set) with work present \
                 but no wake was issued (Race C)"
            );
        }
    });
}

// =========================================================================
// M2 calibration — the model must be able to GO RED.
//
// The same model with the producer's `publish_fence()` deleted. Without a
// StoreLoad barrier between the publish and the idle load this is the bare
// store-buffer litmus, whose forbidden outcome IS reachable, so loom must find
// it and the model's own lost-wake oracle must fire. `expected` pins that
// oracle's message: a bare `#[should_panic]` would also pass on a loom deadlock
// report or on any unrelated assertion, and the copy could "go red as required"
// without ever exercising what it calibrates.
//
// This test PASSING means M2 can discriminate; this test failing (i.e. no
// panic, or a panic with another message) means M2's own green is worth
// nothing.
// =========================================================================

#[test]
#[should_panic(expected = "M2: lost wake")]
fn loom_m2_calibration_no_producer_fence_is_lost() {
    loom::model(|| {
        let idle = Arc::new(AtomicU64::new(0));
        let work = Arc::new(AtomicBool::new(false));
        let claimed = Arc::new(AtomicBool::new(false));

        let idle_p = Arc::clone(&idle);
        let work_p = Arc::clone(&work);
        let claimed_p = Arc::clone(&claimed);
        let producer = thread::spawn(move || {
            work_p.store(true, Ordering::Release);
            // NO `publish_fence()` here — that deletion is the calibration.
            if claim_one_idle(&idle_p).is_some() {
                claimed_p.store(true, Ordering::Release);
            }
        });

        let idle_w = Arc::clone(&idle);
        let work_w = Arc::clone(&work);
        let worker = thread::spawn(move || {
            if work_w.load(Ordering::Acquire) {
                return;
            }
            mark_idle(&idle_w, 0);
            fence(Ordering::SeqCst); // the CONSUMER's transport fence is kept
            if work_w.load(Ordering::Acquire) {
                unmark_idle(&idle_w, 0);
            }
        });

        producer.join().unwrap();
        worker.join().unwrap();

        let bit_set = idle.load(Ordering::Acquire) & 1 != 0;
        let work_present = work.load(Ordering::Acquire);
        let was_claimed = claimed.load(Ordering::Acquire);
        if bit_set && work_present {
            assert!(
                was_claimed,
                "M2: lost wake — worker parked (idle bit set) with work present \
                 but no wake was issued (Race C)"
            );
        }
    });
}

// =========================================================================
// M2b — idle-bitset CAS contention (3 threads: 2 idle workers + 1 producer).
//
// Two workers mark themselves idle; one producer issues a single
// `claim_one_idle`. Exercises the real `compare_exchange_weak` which-worker
// contention.
//
// Invariant: exactly one worker is claimed per successful `claim_one_idle`
// (the producer clears exactly one bit; the bitset never loses or double-counts
// a wake). With one producer claim and ≤ 2 set bits, the claim clears exactly
// one bit and reports a valid worker id.
// =========================================================================

#[test]
fn loom_m2b_idle_cas_contention_exactly_one() {
    loom::model(|| {
        let idle = Arc::new(AtomicU64::new(0));

        // Two workers mark idle concurrently (real fetch_or Release).
        let idle_a = Arc::clone(&idle);
        let wa = thread::spawn(move || {
            mark_idle(&idle_a, 0);
        });
        let idle_b = Arc::clone(&idle);
        let wb = thread::spawn(move || {
            mark_idle(&idle_b, 1);
        });

        // Producer issues exactly one claim, recording which worker it woke.
        let idle_p = Arc::clone(&idle);
        let claimed_id = Arc::new(AtomicU64::new(u64::MAX));
        let claimed_id_p = Arc::clone(&claimed_id);
        let producer = thread::spawn(move || {
            if let Some(id) = claim_one_idle(&idle_p) {
                claimed_id_p.store(id as u64, Ordering::Release);
            }
        });

        wa.join().unwrap();
        wb.join().unwrap();
        producer.join().unwrap();

        // After everyone has run: the single producer claim cleared at most one
        // bit. So whichever worker it claimed (if any) must be a real worker
        // (0 or 1), and that worker's bit must be cleared in the final mask
        // (no double-wake).
        let claimed = claimed_id.load(Ordering::Acquire);
        let final_mask = idle.load(Ordering::Acquire);

        if claimed != u64::MAX {
            assert!(
                claimed == 0 || claimed == 1,
                "claimed worker id must be a real worker (0 or 1), got {claimed}"
            );
            let claimed_bit = 1u64 << claimed;
            assert_eq!(
                final_mask & claimed_bit,
                0,
                "claimed worker {claimed}'s idle bit must be cleared (no double-wake)"
            );
        }
        // Exactly-one property: both marks always execute, and the single claim
        // can clear at most one bit, so the final popcount is ≥ 1 whenever the
        // claim succeeded — it can never wipe more than one bit.
        assert!(
            final_mask.count_ones() >= 1 || claimed != u64::MAX,
            "a single claim must not clear more than one bit"
        );
    });
}

// =========================================================================
// M2c — KE16 A5: idle-keyed placement (`ke16-a5`, `KE16-DESIGN-A.md` §4.4).
//
// A5 inverts the order of the wake protocol: instead of publishing work and
// then asking who is parked, the spawner reads the idle mask FIRST, CLAIMS one
// bit, places the task in THAT worker's queue and unparks it. The claim is the
// same production core M2/M2b drive (transcribed here as `claim_one_idle_bit`
// for the same `&PoolInner` reason), and the parked side is the real
// `mark_idle` / `unmark_idle` — but unlike M2 the worker here PARKS FOR REAL,
// because the property under test is A5-1: **a claimed bit is always followed by
// an unpark of that worker.** A worker parks with an untimed `park()`; a bit
// taken out of the mask without the matching unpark is a core lost until
// shutdown.
//
// The oracles, and what each can catch:
//   - `claims == unparks` — the structural form of A5-1. A claim path that
//     returns early between the CAS and the `unpark` fires it.
//   - **loom's own deadlock detection** is the liveness oracle: the worker's
//     `park()` can only return if the claimer unparked it, so a violation of
//     A5-1 in an interleaving where the worker did park hangs that execution and
//     loom reports it. Deleting the `target.unpark()` below is what makes this
//     model go red — that is its calibration, and it is stated rather than
//     shipped as a copy because the failure is loom's report, not an assertion
//     of ours (§5 item 15 governs the two copies that ARE assertion-shaped).
//   - `placed` at the end — claim, place, unpark is one indivisible sequence;
//     an arm that claimed and woke a worker without ever putting the task in a
//     queue would wake it to find nothing.
//
// What M2c deliberately does NOT re-derive: the fallback arm's store-buffer
// litmus. That arm is the un-inverted protocol with the production
// `publish_fence` prologue, and it is M2's whole subject; here it is exercised
// (the model reaches it whenever the mask was empty at the load) but its own
// oracle is not restated, because a worker that parks FOR REAL cannot be asked
// about its final state — it either wakes or the execution hangs, which is the
// deadlock report above.
//
// The producer holds the worker's `Thread` before the wave, the way a spawner
// reads `inner.workers[id].thread` — a handle registered once at build time,
// never discovered at push time.
// =========================================================================

#[cfg(feature = "ke16-a5")]
#[test]
fn loom_m2c_a5_claimed_worker_is_always_unparked() {
    loom::model(|| {
        let idle = Arc::new(AtomicU64::new(0));
        // The toy transport standing in for `injector_local[target]`.
        let placed = Arc::new(AtomicBool::new(false));
        let claims = Arc::new(AtomicU64::new(0));
        let unparks = Arc::new(AtomicU64::new(0));
        let parked = Arc::new(AtomicBool::new(false));
        let woke = Arc::new(AtomicBool::new(false));

        // Worker 0: the real park sequence — pre-mark re-poll, `mark_idle`, the
        // transport fence, the load-bearing post-mark re-poll, then a REAL park.
        let idle_w = Arc::clone(&idle);
        let placed_w = Arc::clone(&placed);
        let parked_w = Arc::clone(&parked);
        let woke_w = Arc::clone(&woke);
        let worker = thread::spawn(move || {
            if placed_w.load(Ordering::Acquire) {
                return; // took the work before ever marking idle
            }
            mark_idle(&idle_w, 0); // real production fetch_or, Release
            fence(Ordering::SeqCst); // the steal path's transport fence
            if placed_w.load(Ordering::Acquire) {
                unmark_idle(&idle_w, 0); // real production fetch_and, Release
                return;
            }
            parked_w.store(true, Ordering::Release);
            thread::park();
            woke_w.store(true, Ordering::Release);
            unmark_idle(&idle_w, 0);
        });
        let target = worker.thread().clone();

        let idle_p = Arc::clone(&idle);
        let placed_p = Arc::clone(&placed);
        let claims_p = Arc::clone(&claims);
        let unparks_p = Arc::clone(&unparks);
        let producer = thread::spawn(move || {
            // The A5 arm: mask first, ONE claim attempt (A5-4), place into the
            // claimed worker's queue, unpark it. No `publish_fence` here — this
            // arm does not rest on the SB litmus, because the wake is explicit
            // and program order plus the parker's own protocol orders the two.
            let observed = idle_p.load(Ordering::Acquire);
            if observed != 0
                && let Some(id) = claim_one_idle_bit(&idle_p, observed, 0, 0)
            {
                claims_p.fetch_add(1, Ordering::AcqRel);
                assert_eq!(id, 0, "the model has exactly one worker to claim");
                placed_p.store(true, Ordering::Release);
                target.unpark();
                unparks_p.fetch_add(1, Ordering::AcqRel);
                return;
            }
            // The fallback arm: A2's placement, then the fenced wake decision —
            // publish, `publish_fence()`, load, claim (M2's shape, production's
            // `unpark_one_idle`).
            placed_p.store(true, Ordering::Release);
            publish_fence();
            if claim_one_idle(&idle_p).is_some() {
                claims_p.fetch_add(1, Ordering::AcqRel);
                target.unpark();
                unparks_p.fetch_add(1, Ordering::AcqRel);
            }
        });

        producer.join().unwrap();
        worker.join().unwrap();

        // A5-1, structurally: no bit is ever claimed without its unpark.
        assert_eq!(
            claims.load(Ordering::Acquire),
            unparks.load(Ordering::Acquire),
            "M2c: A5-1 violated — a claimed idle bit was not followed by an unpark of that \
             worker, which loses the core until shutdown"
        );

        // Claim, place, unpark is one sequence: whichever arm ran, the task
        // reached a queue. (A5-2's other half — that the queue is one every
        // sibling scans — is the `ke16-a2` implication, a build fact, not an
        // interleaving one.)
        assert!(
            placed.load(Ordering::Acquire),
            "M2c: the producer finished without placing the task in any queue"
        );

        // Liveness, positively: whenever the worker did park, it woke. The flag
        // lives on the far side of `park()`, so the only other outcome is an
        // execution that never gets there — which loom reports as a deadlock.
        assert!(
            !parked.load(Ordering::Acquire) || woke.load(Ordering::Acquire),
            "M2c: the worker parked and its park never returned"
        );
    });
}

// =========================================================================
// M2c calibration — the A5-1 COUNTER oracle, shown to be able to fail.
//
// M2c's header states its liveness calibration ("deleting the `target.unpark()`
// below is what makes this model go red") but ships no copy of it, because that
// red is loom's deadlock report rather than an assertion of ours, and refused
// shape 15 (`KE16-DESIGN-MEASUREMENT.md` §5) requires a calibration copy to go
// red WITH ITS OWN ORACLE'S MESSAGE. The structural half of A5-1 —
// `claims == unparks` — can be calibrated that way, and until this copy existed
// nothing had shown it firing: the two counters sit in adjacent statements of
// one branch, so the assertion reads as a restatement of the transcription
// rather than as a check on it. An oracle that has never been observed to fail
// is not yet known to be a gate.
//
// The defect transcribed here is the one `KE16-DESIGN-A.md` §4.3 A5-1 names in
// words — "the claim and the unpark are one function, never split by an early
// return": the producer claims the bit and returns without waking anybody. The
// copy differs from the model in exactly that one statement.
//
// The worker does NOT park here, and that is what makes the red deterministic
// rather than incidental. A parking worker whose unpark never comes hangs its
// execution and loom reports a DEADLOCK — a different message, which
// `#[should_panic(expected = ...)]` refuses ("a red on a different message
// means the copy is broken, not calibrated", §5 item 15). With the park removed
// every interleaving terminates, and the interleaving in which the producer's
// load sees the freshly marked bit reaches the assertion with `claims == 1` and
// `unparks == 0`. The liveness half of A5-1 keeps the calibration M2c's own
// header gives it: loom's deadlock detection, stated in prose because its red
// cannot be spelled as an oracle message.
// =========================================================================

#[cfg(feature = "ke16-a5")]
#[test]
#[should_panic(expected = "M2c: A5-1 violated")]
fn loom_m2c_calibration_claim_without_unpark_fails_the_a5_1_oracle() {
    loom::model(|| {
        let idle = Arc::new(AtomicU64::new(0));
        let placed = Arc::new(AtomicBool::new(false));
        let claims = Arc::new(AtomicU64::new(0));
        let unparks = Arc::new(AtomicU64::new(0));

        // The worker of M2c minus its `park()`: it still marks idle, still
        // fences, still re-polls — so the bit the producer claims is a real one,
        // published by the real production `mark_idle` — but it leaves through
        // `unmark_idle` instead of parking, so no execution can hang.
        let idle_w = Arc::clone(&idle);
        let placed_w = Arc::clone(&placed);
        let worker = thread::spawn(move || {
            if placed_w.load(Ordering::Acquire) {
                return;
            }
            mark_idle(&idle_w, 0);
            fence(Ordering::SeqCst);
            let _ = placed_w.load(Ordering::Acquire);
            unmark_idle(&idle_w, 0);
        });
        let target = worker.thread().clone();

        let idle_p = Arc::clone(&idle);
        let placed_p = Arc::clone(&placed);
        let claims_p = Arc::clone(&claims);
        let unparks_p = Arc::clone(&unparks);
        let producer = thread::spawn(move || {
            let observed = idle_p.load(Ordering::Acquire);
            if observed != 0
                && let Some(id) = claim_one_idle_bit(&idle_p, observed, 0, 0)
            {
                claims_p.fetch_add(1, Ordering::AcqRel);
                assert_eq!(id, 0, "the model has exactly one worker to claim");
                placed_p.store(true, Ordering::Release);
                // THE MUTATION: `target.unpark()` and its count are gone. This
                // is A5-1's defect exactly — an early return between the CAS and
                // the wake.
                return;
            }
            // The fallback arm is the model's, unchanged: a claim taken here IS
            // followed by its unpark, so an execution that never reaches the
            // claim arm leaves the counters equal and asserts nothing. The red
            // must come from the mutated arm or not at all.
            placed_p.store(true, Ordering::Release);
            publish_fence();
            if claim_one_idle(&idle_p).is_some() {
                claims_p.fetch_add(1, Ordering::AcqRel);
                target.unpark();
                unparks_p.fetch_add(1, Ordering::AcqRel);
            }
        });

        producer.join().unwrap();
        worker.join().unwrap();

        assert_eq!(
            claims.load(Ordering::Acquire),
            unparks.load(Ordering::Acquire),
            "M2c: A5-1 violated — a claimed idle bit was not followed by an unpark of that \
             worker, which loses the core until shutdown"
        );
    });
}

// =========================================================================
// M3 — shutdown handshake.
//
// A coordinator sets `shutdown` (Release); the worker, after its re-poll, loads
// `shutdown` (Acquire) and must observe it and exit. Mirrors
// `ThreadPool::drop`'s `shutdown.store(Release)` vs the `shutdown.load(Acquire)`
// that `worker_main` performs between its post-`mark_idle` re-poll and its park.
//
// Invariant: every worker observes shutdown and exits (none parks forever).
// =========================================================================

#[test]
fn loom_m3_shutdown_handshake_worker_exits() {
    loom::model(|| {
        let shutdown = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));

        // Worker: re-poll for shutdown (Acquire). In production the worker would
        // park if not shut down; here the model checks that, across all
        // interleavings, the Acquire load eventually observes the Release store.
        let shutdown_w = Arc::clone(&shutdown);
        let exited_w = Arc::clone(&exited);
        let worker = thread::spawn(move || {
            // Bounded re-poll then yield, mirroring the worker's
            // "shutdown check after re-poll" without an unbounded park.
            for _ in 0..2 {
                if shutdown_w.load(Ordering::Acquire) {
                    exited_w.store(true, Ordering::Release);
                    return;
                }
                thread::yield_now();
            }
            // Final authoritative check (the post-park re-check in production).
            if shutdown_w.load(Ordering::Acquire) {
                exited_w.store(true, Ordering::Release);
            }
        });

        // Coordinator: publish shutdown (Release) — the ThreadPool::drop store.
        shutdown.store(true, Ordering::Release);

        worker.join().unwrap();

        // Invariant: the worker observed shutdown and exited.
        assert!(
            exited.load(Ordering::Acquire),
            "worker did not observe shutdown / did not exit"
        );
    });
}

// =========================================================================
// M4 — KE16 W-b: the <=1 push gate and the self-excluding thief-residue
// cascade (`ke16-w-gate`, `KE16-DESIGN-W.md` §2.5).
//
// ## The transport, and why the length is a SEPARATE cell
//
// The owner's queue is a `Mutex<VecDeque<u32>>` with its length in a distinct
// loom `AtomicUsize`. The push is `snapshot = len.load` -> `lock; push_back` ->
// `len.fetch_add` -> `gate(snapshot)`: three operations, not one. That
// separation is the model's whole point. A length that moved ATOMICALLY with
// the push would remove the check-then-push race the gate exists to cover, and
// the model would pass without ever expressing it — the critic's warning about
// a vacuous shape. crossbeam's real `Worker::len()` is exactly this: two loads
// taken before a push that is a third operation.
//
// ## Every row arms every oracle; what varies is the PUSHER
//
// §2.5 specifies one model with three gated pushes and both oracles. Three is
// the smallest item count that can produce a pre-push snapshot of 2, and hence
// the smallest that can take the branch W-b actually ADDS —
// `worker::wake_after_push`'s `if pre_len > 1 { return; }`. Nothing below turns
// an oracle off: every row runs the lost-wake oracle AND the self-claim oracle,
// and every row asserts, over all of loom's executions, whether the decline was
// reached (`snapshots_above_one`) and whether a cascade ran (`cascades`), so no
// row can quietly stop exercising its own subject.
//
// The rows turn on one axis the design states but the first cut of this model
// did not represent: what the PUSHING thread does after its last push.
//
//   * `M4Pusher::DrainsItsOwnQueue` — the pusher is a LANE of the pool and the
//     destination is its OWN queue. §2.3 step 2: "A task in worker X's
//     registered deque was pushed by X itself ... X's next act is its join
//     (B0/B1: it consults its own deque first) or its loop's own pop. X does
//     not park with a non-empty own deque." That is the A1 spawn arm, the one
//     KE16 exists to introduce.
//   * `M4Pusher::NeverLooksAgain` — the pusher is NOT a lane of the destination
//     pool and never consults the queue again: the dispatcher's
//     `injector_global` pushes on the frame path, and, under B3, an external
//     joiner that parks instead of helping. §2.3's proof puts no such pusher
//     back on the queue.
//
// ## The finding this family records — ESCALATED, not silenced
//
// EVERY ROW BELOW IS A READING. The models run on this box under the recipe in
// the family header; the colours and wall times are tabled there. Until
// 2026-09-03 this paragraph said the opposite ("NOTHING BELOW IS A READING …
// the colours claimed per row are ARGUMENTS"), on the strength of a
// `RUSTFLAGS="--cfg loom"` build that aborts at startup. That was a recipe
// fault, and it had converted W-b's whole obligation into an excused blank.
//
// With `NeverLooksAgain` and three items the PRODUCTION <=1 gate IS RED on the
// lost-wake oracle (`loom_m4c_…`, 40.71 s), and it is red for a property of the
// candidate rather than of the model. §2.3 proves W-b-1 from the pre-lengths
// that were READ ("the push that took Q from 0 to 1 and the push that took it
// from 1 to 2 each read a pre-length <= 1"), and recovers what is left in step 4
// from the thieves that drained Q being awake and re-scanning "when their bodies
// end". A push whose snapshot is stale at 2 while the queue has since been
// emptied performs the real 0->1 transition with NO wake decision at all — no
// `publish_fence`, no mask load — so step 3's litmus never runs for it; and step
// 4's recovery holds only while that push lands DURING such a body. A push that
// lands after the last thief's post-`mark_idle` re-poll leaves the task in the
// queue with every lane asleep and no wake pending. §2.3's W-b-1 is therefore
// refuted by an executed model, not by an argument.
//
// WHAT RECOVERS IT DEPENDS ON THE ROUTE, and one route has no recovery quantum
// at all:
//
//   * A scope's push (`Scope::spawn` under `install`, the frame path) is
//     recovered by the JOINER's timed backstop (`scope.rs`'s `park_timeout`
//     over `JOIN_BACKSTOP`, measured on this box at ~1 ms with App-12's guard
//     held and ~15 ms without), which neither W-b-1 nor §2.3 invokes.
//   * `ThreadPool::spawn` is FIRE AND FORGET (`thread_pool.rs:536-541` ->
//     `PoolInner::spawn`, `:298-304` -> `worker::push_task`): it opens no
//     scope, so `join_workers_until_drained` is never on any stack and NO
//     `park_timeout` ever runs. A worker parks UNTIMED
//     (`worker::worker_main`'s `std::thread::park()`, `worker.rs:143`), so the
//     stranded task waits for `ThreadPool::shutdown_and_join`'s unpark of every
//     worker at pool teardown (`thread_pool.rs:600-608`) — a pool lifetime, not
//     a backstop quantum. This instance is UNBOUNDED, and the escalated cost
//     statement has to price it as such (code-reviewer, 2026-09-03).
//     `ThreadPool::spawn` has no production caller today — every `pool.spawn`
//     outside `tests/` and `benches/` is inside a `#[cfg(test)]` module — but it
//     is public API and the crate's own tests use it, where a strand hangs the
//     test until pool drop. The native corroboration row is
//     `ke16_w_wake_completion.rs::w_b_a_fire_and_forget_wave_into_a_parked_pool_
//     runs_every_task_exactly_once`, which waits on a per-task counter with the
//     pool deliberately kept alive so teardown cannot be what completes the wave.
//
// Whether that window is the candidate's priced cost or a defect is a design
// call, and it is escalated as one. It is not resolved by disarming an oracle,
// and it is recorded MECHANICALLY rather than in prose:
// `loom_m4c_stale_snapshot_strands_a_task_when_the_pusher_is_not_a_lane` runs
// exactly that configuration with the lost-wake oracle armed and is
// `#[should_panic(expected = "M4: lost wake")]`, so the row fails the moment
// the window closes or the oracle stops firing. A finding in a test cannot rot
// the way a finding in a comment can.
//
// M4b is its green counterpart: the same three items, the same production gate,
// the same oracles — with the pusher on its own queue. M4b's lost-wake oracle
// is armed and does NOT fire (measured green, 385.55 s — the header's table),
// because `pusher_quiescent` is set only after a `pop_front` returned `None` and
// nothing pushes afterwards; M4c is what
// keeps that from being a gate which cannot fail, the two rows differing in
// nothing but the pusher.
//
// ## What is production code here and what is transcribed
//
// Production: `mark_idle`, `unmark_idle`, `publish_fence`, and loom's REAL
// `park` / `unpark` on real `Thread` handles. Transcribed, for the same
// `&PoolInner` reason as M2: the one-attempt claim (`claim_one_idle_bit`) and
// the two callers around it — the fenced wake decision of
// `unpark_one_idle_excluding` and the gate of `worker::wake_after_push` /
// `worker::wake_after_residue`.
//
// ## Oracles
//
//   * **lost wake**: a thief whose `park()` returned WITHOUT its idle bit having
//     been claimed (it read its own bit back as still set from `unmark_idle`'s
//     RMW, so no `claim_one_idle` took it — it was woken by the model's shutdown
//     `unpark` alone), while work remains and every OTHER lane is quiescent: the
//     pusher has stopped consulting the queue, and each other thief is
//     parked-and-unclaimed or has left. Each guard is load-bearing against a
//     FALSE red and each corresponds to a state the design calls legal: an awake
//     sibling is the "one-body stall" of §2.3(i), a latency cost and not a lost
//     wake; a pusher still on the queue is §2.3 step 2; and a thief that
//     consumed a claim on a re-poll exit carries a spent token into its next
//     park (production's `owes_wake`), whose immediate return says nothing about
//     any wake being lost.
//   * **cascade claimed self**: the claim core, driven from a cascade, returning
//     the cascading thief's own id — the chain broken at hop one, a wake spent
//     on the thread that issued it while W-1 siblings sleep on.
//
// Plus, on every row: loom's own deadlock detection (a park nobody can end),
// and "every item ran exactly once".
//
// ## The thief's exit, and the one place this model refines §2.5
//
// §2.5's thief ends when the model's shutdown unpark reaches it. Production's
// worker leaves `worker_main` on the same signal, but only after a `pop_any`
// that found nothing (`worker.rs:119-141`), and production raises `shutdown`
// only from `ThreadPool::drop`, with no scope live. This model raises it while
// items are still in flight — that is what ends a park nobody else ends — so a
// thief that observed `shutdown` and left could carry away a claim just spent on
// it and strand an item: a red that says nothing about the gate. The thief
// therefore RE-SCANS once after observing `shutdown` and before leaving; the
// pushes precede the `Release` store of `shutdown` and the thief's load of it is
// `Acquire`, so that scan sees every pushed item and a thief that leaves leaves
// an empty queue behind it. For the same reason the first cut's round COUNT is
// replaced by a round BUDGET whose exhaustion is an assertion and not an exit:
// a truncated lane must never be able to count as a quiescent one.
// =========================================================================

/// How the owner gates its pushes. `AtMostOne` is production
/// (`worker::wake_after_push` under `ke16-w-gate`); `EmptyOnly` is the
/// calibration's "was empty" rule, which the critic's interleaving defeats.
#[cfg(feature = "ke16-w-gate")]
#[derive(Clone, Copy)]
enum M4Gate {
    AtMostOne,
    EmptyOnly,
}

/// Whether a cascading thief masks its own idle bit out of the claim.
/// `SelfExcluded` is production (`worker::wake_after_residue`); `Unmasked` is
/// the calibration.
#[cfg(feature = "ke16-w-gate")]
#[derive(Clone, Copy)]
enum M4Exclusion {
    SelfExcluded,
    Unmasked,
}

/// What the pushing thread does after its last push — the axis this family
/// turns on (see the header).
#[cfg(feature = "ke16-w-gate")]
#[derive(Clone, Copy)]
enum M4Pusher {
    /// A lane of the pool pushing into its own queue, which it then consults at
    /// its join (`KE16-DESIGN-W.md` §2.3 step 2): the A1 spawn arm.
    DrainsItsOwnQueue,
    /// A pusher that is not a lane of the destination pool: the dispatcher's
    /// `injector_global` pushes, and B3's parked external joiner.
    NeverLooksAgain,
}

/// One row of the M4 family. The two counters are `std` atomics on purpose:
/// they live OUTSIDE loom's modelled state and accumulate across every
/// execution, which is what lets a row assert that the branch it exists to test
/// was reached at least once somewhere in the search.
#[cfg(feature = "ke16-w-gate")]
#[derive(Clone, Copy)]
struct M4Config {
    items: usize,
    gate: M4Gate,
    exclusion: M4Exclusion,
    pusher: M4Pusher,
    /// Pushes whose pre-length snapshot was > 1 — the production decline.
    snapshots_above_one: &'static StdAtomicUsize,
    /// Batch steals that left residue and therefore issued the cascade.
    cascades: &'static StdAtomicUsize,
}

#[cfg(feature = "ke16-w-gate")]
const M4_THIEVES: u32 = 2;
/// A bound on a thief's loop iterations that must NEVER be reached: every exit
/// is the shutdown exit. Reaching it is an assertion failure, because a lane
/// truncated by the model would otherwise count as a quiescent one in the
/// lost-wake oracle. Each iteration either runs an item (at most `items` of
/// them) or consumes one wake (at most one per push, one per cascade, one at
/// shutdown), so this is generous for every row below.
#[cfg(feature = "ke16-w-gate")]
const M4_ROUND_BUDGET: usize = 16;
/// A batch take of 2 is the smallest one that can leave residue.
#[cfg(feature = "ke16-w-gate")]
const M4_BATCH: usize = 2;

#[cfg(feature = "ke16-w-gate")]
struct M4World {
    /// The owner's queue — the loom-visible stand-in for a registered deque.
    queue: Mutex<VecDeque<u32>>,
    /// Its length, as a cell distinct from the queue (see the header).
    len: AtomicUsize,
    /// The real idle bitset, driven by the real `mark_idle` / `unmark_idle`.
    idle: AtomicU64,
    /// Bit `i`: thief `i` holds one residue item it batch-stole and has not run.
    slots: AtomicU64,
    /// Bit `i`: thief `i` is inside `park()`. Set after its last scan, so a set
    /// bit means "this thief has finished re-polling and will not scan again
    /// until something wakes it".
    parked: AtomicU64,
    /// Bit `i`: thief `i` has left its loop, which it does only after a scan
    /// taken with `shutdown` already observed.
    finished: AtomicU64,
    /// Set once the pusher will never consult the queue again: immediately after
    /// the last push under `NeverLooksAgain`, and after its own drain under
    /// `DrainsItsOwnQueue`. Until then the pusher is an awake lane and the
    /// lost-wake oracle must not fire.
    pusher_quiescent: AtomicBool,
    /// Wake targets, registered by each thief as its first act — the model's
    /// stand-in for `inner.workers[id].thread`, which the pool registers at
    /// build time. A bit can only be set after its owner registered here, so a
    /// claimed id always has a handle.
    handles: Mutex<Vec<Option<Thread>>>,
    /// How many items have been run, by anyone.
    ran: AtomicUsize,
    /// The model's shutdown, which is what ends a park nobody else ends —
    /// production's `shutdown_and_join` plays the same role.
    shutdown: AtomicBool,
    cfg: M4Config,
}

#[cfg(feature = "ke16-w-gate")]
impl M4World {
    fn new(cfg: M4Config) -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            len: AtomicUsize::new(0),
            idle: AtomicU64::new(0),
            slots: AtomicU64::new(0),
            parked: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            pusher_quiescent: AtomicBool::new(false),
            handles: Mutex::new((0..M4_THIEVES).map(|_| None).collect()),
            ran: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            cfg,
        }
    }

    /// The owner's push and its gate — `worker::push_on_lane` followed by
    /// `worker::wake_after_push`.
    fn gated_push(&self, item: u32) {
        let pre = self.len.load(Ordering::Acquire);
        self.queue.lock().unwrap().push_back(item);
        self.len.fetch_add(1, Ordering::AcqRel);
        if pre > 1 {
            // The branch W-b adds, counted under either gate so that a row can
            // assert it WAS reached (M4b, M4c) or that it was not (M4a).
            self.cfg
                .snapshots_above_one
                .fetch_add(1, StdOrdering::Relaxed);
        }
        let wake = match self.cfg.gate {
            M4Gate::AtMostOne => pre <= 1,
            M4Gate::EmptyOnly => pre == 0,
        };
        if wake {
            // The owner is not a thief and has no bit of its own to exclude.
            self.wake_one(0, u32::MAX);
        }
    }

    /// The fenced wake decision: production's `unpark_one_idle_excluding` with
    /// the transcribed claim core. `caller` is the id for which a claim would be
    /// a self-claim (`u32::MAX` for the owner, which has no bit).
    fn wake_one(&self, exclude: u64, caller: u32) {
        publish_fence();
        let mut observed = self.idle.load(Ordering::Acquire);
        while observed & !exclude != 0 {
            if let Some(id) = claim_one_idle_bit(&self.idle, observed, exclude, 0) {
                assert_ne!(
                    id, caller,
                    "cascade claimed self — the thief spent the pool's single wake on the \
                     thread that issued it, so the activation chain is broken at hop one and \
                     every other parked lane sleeps on"
                );
                self.unpark(id);
                return;
            }
            observed = self.idle.load(Ordering::Acquire);
        }
    }

    fn unpark(&self, id: u32) {
        let handle = self.handles.lock().unwrap()[id as usize].clone();
        handle
            .expect("invariant: a thief registers its wake handle before it can be idle-marked")
            .unpark();
    }

    /// Pop up to `M4_BATCH` items under one lock, then correct the length —
    /// crossbeam's take-then-publish shape.
    fn take_batch(&self) -> usize {
        let mut n = 0;
        {
            let mut q = self.queue.lock().unwrap();
            while n < M4_BATCH && q.pop_front().is_some() {
                n += 1;
            }
        }
        if n > 0 {
            self.len.fetch_sub(n, Ordering::AcqRel);
        }
        n
    }

    /// The PUSHER consuming its own queue at its join — `DrainsItsOwnQueue`
    /// only. Returns false once the queue is empty, which is the point at which
    /// that lane stops looking.
    fn pusher_take_one(&self) -> bool {
        let took = self.queue.lock().unwrap().pop_front().is_some();
        if took {
            self.len.fetch_sub(1, Ordering::AcqRel);
            self.ran.fetch_add(1, Ordering::AcqRel);
        }
        took
    }

    /// One scan by thief `id`: its own residue first, then a batch from the
    /// owner's queue. Returns true iff an item was RUN.
    ///
    /// A batch that leaves residue takes the cascade from HERE — which, on one
    /// of the two call sites, is inside the post-`mark_idle` re-poll with this
    /// thief's own bit still set. That is the site the self-exclusion exists
    /// for.
    fn try_work(&self, id: u32) -> bool {
        let bit = 1u64 << id;
        // Only thief `id` ever sets or clears bit `id`, so the load-then-clear
        // needs no CAS.
        if self.slots.load(Ordering::Acquire) & bit != 0 {
            self.slots.fetch_and(!bit, Ordering::AcqRel);
            self.ran.fetch_add(1, Ordering::AcqRel);
            return true;
        }
        let took = self.take_batch();
        if took == 0 {
            return false;
        }
        if took > 1 {
            self.slots.fetch_or(bit, Ordering::AcqRel);
            self.cfg.cascades.fetch_add(1, StdOrdering::Relaxed);
            let exclude = match self.cfg.exclusion {
                M4Exclusion::SelfExcluded => bit,
                M4Exclusion::Unmasked => 0,
            };
            self.wake_one(exclude, id);
        }
        self.ran.fetch_add(1, Ordering::AcqRel);
        true
    }

    /// Is every thief other than `id` asleep-and-unclaimed or gone?
    ///
    /// One read of each of the three words, in one place, so the oracle can take
    /// this observation TWICE and bracket its own work read with it (see
    /// [`check_lost_wake`](Self::check_lost_wake)).
    fn others_quiescent(&self, id: u32) -> bool {
        let idle_now = self.idle.load(Ordering::Acquire);
        let parked_now = self.parked.load(Ordering::Acquire);
        let finished_now = self.finished.load(Ordering::Acquire);
        (0..M4_THIEVES).all(|other| {
            if other == id {
                return true;
            }
            let ob = 1u64 << other;
            finished_now & ob != 0 || (parked_now & ob != 0 && idle_now & ob != 0)
        })
    }

    /// The lost-wake oracle, applied by thief `id` after a `park()` that no
    /// claim accounted for. See the header for why each guard is here.
    ///
    /// **The reads are ordered, and the state read is only the QUEUE, because a
    /// multi-word snapshot of this model tears and the tear fires the oracle.**
    /// MEASURED on the first execution of this family (the rows were argued, not
    /// run, until 2026-09-03): M4a reported `queued=0 others_hold=1
    /// finished=0b1 ran=2` — a thief counted as holding residue that it had
    /// already run and left the model over. Nothing orders a plain `slots` load
    /// against a later `finished` load, so the oracle read a stale `slots` and a
    /// fresh `finished` and concluded that an item nobody would find was in the
    /// hand of a thread that had exited. Two rules follow, and both are about
    /// what a WRONG read may do, never about what a right one shows:
    ///
    /// 1. **Residue is not work that can be lost, so it is not counted.** A
    ///    thief cannot park or leave while its slot is set: `try_work` answers
    ///    the slot FIRST and returns `true`, so a set slot bit always has an
    ///    awake owner running it. Counting it could only ever add a false red —
    ///    which is exactly what it did. The residue's own oracle is elsewhere:
    ///    the self-claim assert on the cascade, and the model's closing
    ///    `slots == 0` / `ran == items` assertions.
    /// 2. **The quiescence observation brackets the work read.** Taken once
    ///    before and once after, so a stale-old quiescence read cannot fire the
    ///    oracle on its own: the second observation is ordered after the queue's
    ///    mutex release and has to agree.
    fn check_lost_wake(&self, id: u32) {
        // The pusher is a lane too whenever it goes on to consult the queue;
        // while it has not stopped, the pool is not asleep (§2.3 step 2).
        if !self.pusher_quiescent.load(Ordering::Acquire) {
            return;
        }
        if !self.others_quiescent(id) {
            return;
        }
        let queued = self.queue.lock().unwrap().len();
        if queued == 0 {
            return;
        }
        if !self.others_quiescent(id) {
            return;
        }
        panic!(
            "M4: lost wake — thief {id} came out of park with its own idle bit never claimed \
             (so nothing woke it for work) while {queued} item(s) were still queued, the pusher \
             had stopped consulting the queue and no other thief was awake to find them"
        );
    }
}

/// One thief: `worker_main`'s scan / park sequence (`worker.rs:65-152`), with
/// the post-shutdown re-scan the header argues for.
#[cfg(feature = "ke16-w-gate")]
fn m4_thief(w: &M4World, id: u32) {
    let bit = 1u64 << id;
    w.handles.lock().unwrap()[id as usize] = Some(thread::current());

    // A claim consumed on a re-poll exit leaves a token on this parker that the
    // NEXT park returns from at once. Production carries the same fact as
    // `owes_wake` (`scope.rs`, the B1-P joiner); here it only has to silence the
    // oracle for that one return, which says nothing about any wake being lost.
    let mut spent_token = false;
    let mut budget = M4_ROUND_BUDGET;

    loop {
        budget -= 1;
        assert!(
            budget > 0,
            "M4: thief {id} exhausted its round budget — the model's only truncation has become \
             reachable, and a truncated lane would count as a quiescent one in the lost-wake \
             oracle; raise M4_ROUND_BUDGET rather than let that happen"
        );

        if w.try_work(id) {
            continue;
        }
        mark_idle(&w.idle, id); // real production fetch_or, Release
        fence(Ordering::SeqCst); // the crossbeam steal path's transport fence
        // The post-`mark_idle` re-poll, load-bearing against Race C — and the
        // cascade site where this thief's own bit is set.
        if w.try_work(id) {
            if unmark_idle(&w.idle, id) & bit == 0 {
                spent_token = true;
            }
            continue;
        }
        if w.shutdown.load(Ordering::Acquire) {
            // The re-scan the header argues for: `shutdown` is Release-stored
            // after the last push, so this Acquire load makes every pushed item
            // visible and a thief never leaves work behind it.
            if w.try_work(id) {
                if unmark_idle(&w.idle, id) & bit == 0 {
                    spent_token = true;
                }
                continue;
            }
            unmark_idle(&w.idle, id);
            break;
        }
        w.parked.fetch_or(bit, Ordering::AcqRel);
        thread::park();
        w.parked.fetch_and(!bit, Ordering::AcqRel);
        // Read the claim off the RMW, not off a preceding load: that is what
        // makes "was the bit still mine?" exact (`worker::unmark_idle`).
        if unmark_idle(&w.idle, id) & bit == 0 {
            continue; // a claim took our bit: we were woken for work
        }
        if spent_token {
            spent_token = false;
            continue;
        }
        w.check_lost_wake(id);
    }

    w.finished.fetch_or(bit, Ordering::AcqRel);
}

/// Drive one row of the M4 family.
#[cfg(feature = "ke16-w-gate")]
fn m4_model(cfg: M4Config) {
    loom::model(move || {
        let w = Arc::new(M4World::new(cfg));

        let mut joins = Vec::with_capacity(M4_THIEVES as usize);
        for id in 0..M4_THIEVES {
            let w_t = Arc::clone(&w);
            joins.push(thread::spawn(move || m4_thief(&w_t, id)));
        }
        let targets: Vec<Thread> = joins.iter().map(|h| h.thread().clone()).collect();

        for item in 0..cfg.items {
            w.gated_push(item as u32);
        }

        if matches!(cfg.pusher, M4Pusher::DrainsItsOwnQueue) {
            // The spawning lane's next act is its own join, which consults its
            // own queue first (`KE16-DESIGN-W.md` §2.3 step 2). It does not park
            // while that queue holds work.
            while w.pusher_take_one() {}
        }
        w.pusher_quiescent.store(true, Ordering::Release);

        // Shutdown: the flag first, then one unpark per thief. This is what ends
        // a park no claim ended, exactly as `ThreadPool::drop` does.
        w.shutdown.store(true, Ordering::Release);
        for t in &targets {
            t.unpark();
        }
        for h in joins {
            h.join().unwrap();
        }

        assert!(
            w.queue.lock().unwrap().is_empty(),
            "M4: a thief left the queue non-empty — its post-shutdown re-scan is not doing its job"
        );
        assert_eq!(
            w.slots.load(Ordering::Acquire),
            0,
            "M4: a thief left with residue still in its slot"
        );
        assert_eq!(
            w.ran.load(Ordering::Acquire),
            cfg.items,
            "M4: item accounting — every pushed item must be run exactly once"
        );
    });
}

/// M4a — the gate race at the 0/1 transitions. Two items, so no push can
/// snapshot above one: this row isolates what the <=1 rule was WIDENED to cover
/// (a thief that empties a queue of one between the snapshot and the push), and
/// its calibration is the pure-empty gate that loses exactly that wake. The
/// decline branch is M4b's and M4c's subject, and the assertion below pins that
/// division so the two rows cannot be mistaken for one another.
#[cfg(feature = "ke16-w-gate")]
#[test]
fn loom_m4a_gate_race_no_lost_wake() {
    static SNAPSHOTS: StdAtomicUsize = StdAtomicUsize::new(0);
    static CASCADES: StdAtomicUsize = StdAtomicUsize::new(0);
    m4_model(M4Config {
        items: 2,
        gate: M4Gate::AtMostOne,
        exclusion: M4Exclusion::SelfExcluded,
        pusher: M4Pusher::NeverLooksAgain,
        snapshots_above_one: &SNAPSHOTS,
        cascades: &CASCADES,
    });
    assert_eq!(
        SNAPSHOTS.load(StdOrdering::Relaxed),
        0,
        "M4a: two items cannot produce a pre-push snapshot above one, so this row does not \
         exercise `wake_after_push`'s decline; if it now does, the row's claim about what it \
         isolates is stale"
    );
}

// =========================================================================
// M4a calibration — the pure-empty gate, which MUST lose the wake.
//
// The only change is `pre <= 1` becoming `pre == 0`. The critic's interleaving
// is then reachable: a thief pops the single item after the owner's snapshot of
// 1, the owner pushes with no wake, and the thieves park with the item visible.
// `expected` pins this model's OWN lost-wake message — a bare `#[should_panic]`
// would be satisfied by a loom deadlock report or by the item-accounting
// assertion, and the copy could "go red as required" without its oracle ever
// firing (`KE16-DESIGN-MEASUREMENT.md` §5 item 15).
// =========================================================================

#[cfg(feature = "ke16-w-gate")]
#[test]
#[should_panic(expected = "M4: lost wake")]
fn loom_m4_calibration_empty_gate_is_lost() {
    static SNAPSHOTS: StdAtomicUsize = StdAtomicUsize::new(0);
    static CASCADES: StdAtomicUsize = StdAtomicUsize::new(0);
    m4_model(M4Config {
        items: 2,
        gate: M4Gate::EmptyOnly,
        exclusion: M4Exclusion::SelfExcluded,
        pusher: M4Pusher::NeverLooksAgain,
        snapshots_above_one: &SNAPSHOTS,
        cascades: &CASCADES,
    });
}

/// M4b — three items with the production gate and the pusher on its own queue.
/// This is the row in which the `pre_len > 1` DECLINE is taken (asserted below)
/// with the lost-wake oracle armed, and it is green for the reason §2.3 step 2
/// gives: the lane that pushed is the lane that consumes, so no interleaving
/// leaves the task in the queue with the pool asleep. Three items is also what
/// lets a thief batch-take two, keep one and cascade to the other from inside
/// its post-`mark_idle` re-poll with its own bit still set — the self-claim
/// oracle's subject, whose calibration is the copy below.
#[cfg(feature = "ke16-w-gate")]
#[test]
fn loom_m4b_decline_is_safe_when_the_pusher_drains_and_the_cascade_never_claims_self() {
    static SNAPSHOTS: StdAtomicUsize = StdAtomicUsize::new(0);
    static CASCADES: StdAtomicUsize = StdAtomicUsize::new(0);
    m4_model(M4Config {
        items: 3,
        gate: M4Gate::AtMostOne,
        exclusion: M4Exclusion::SelfExcluded,
        pusher: M4Pusher::DrainsItsOwnQueue,
        snapshots_above_one: &SNAPSHOTS,
        cascades: &CASCADES,
    });
    assert!(
        SNAPSHOTS.load(StdOrdering::Relaxed) > 0,
        "M4b: no execution reached a pre-push snapshot above one, so `wake_after_push`'s decline \
         — the branch W-b adds — never ran and this row proves nothing about it"
    );
    assert!(
        CASCADES.load(StdOrdering::Relaxed) > 0,
        "M4b: no thief ever kept residue, so the self-excluding cascade never ran"
    );
}

// =========================================================================
// M4b calibration — the cascade without its self-exclusion.
//
// The only change is the `exclude` mask handed to the claim: production passes
// the cascading thief's own bit, this copy passes zero. Both cascade sites are
// reached from inside the post-`mark_idle` re-poll, where that bit is set, so an
// unmasked claim can take it — and does, whenever the lowest-bit pick lands
// there. `expected` pins the self-claim oracle's own message.
// =========================================================================

#[cfg(feature = "ke16-w-gate")]
#[test]
#[should_panic(expected = "cascade claimed self")]
fn loom_m4_calibration_no_self_exclusion_claims_self() {
    static SNAPSHOTS: StdAtomicUsize = StdAtomicUsize::new(0);
    static CASCADES: StdAtomicUsize = StdAtomicUsize::new(0);
    m4_model(M4Config {
        items: 3,
        gate: M4Gate::AtMostOne,
        exclusion: M4Exclusion::Unmasked,
        pusher: M4Pusher::DrainsItsOwnQueue,
        snapshots_above_one: &SNAPSHOTS,
        cascades: &CASCADES,
    });
}

// =========================================================================
// M4c — THE FINDING, kept as a gate rather than as a paragraph.
//
// M4b's configuration with ONE field changed: the pusher is not a lane of the
// destination pool and never consults the queue again (`ThreadPool::spawn` from
// any non-worker thread; the dispatcher's `injector_global` pushes; B3's parked
// external joiner). The lost-wake oracle is ARMED, and it FIRES: a snapshot
// taken at 2 and stale by the time the push lands makes that push the real
// 0->1 transition, and W-b declines it, so the task can sit in the queue with
// both thieves parked and unclaimed and no wake pending — the state W-b-1
// forbids.
//
// ⚠ MEASURED, not predicted. 2026-09-03, this checkout, recipe in the family
// header, `LOOM_MAX_PREEMPTIONS=3 --test-threads=1 --exact`: the row is
// should-panic ok in 40.71 s, and the message the `expected` clause matched is
// the lost-wake oracle's own `M4: lost wake`. `KE16-DESIGN-W.md` §2.3's W-b-1
// proof (steps 3-4) is therefore refuted by an EXECUTED model. The escalated
// question is to be ruled on that footing, and the ruling is materially
// different from one taken on a prediction. (Until 2026-09-03 this paragraph
// said the panic was predicted because loom was believed unrunnable here; that
// belief was a recipe fault, see the family header.)
//
// What recovers the strand depends on the route, and `ThreadPool::spawn`'s
// route has no recovery quantum at all — see the family header's two bullets.
// The recovery for THAT route is pool teardown.
//
// This row is `#[should_panic(expected = "M4: lost wake")]` because the finding
// is a DESIGN call (escalated to the orchestrator: `KE16-DESIGN-W.md` §2.3's
// proof against §2.5's "no lost-wake panic"), not an implementation choice this
// pass may settle — pricing the window as the candidate's cost, or closing it,
// changes W-b itself. Written this way the finding cannot rot: the row goes red
// the day the window closes, the day the oracle stops firing, or the day the
// model stops reaching the decline, and whoever sees that must re-open the
// design question instead of re-reading a comment. It is NOT a calibration
// copy — nothing in it is deliberately wrong. If a FUTURE change makes this row
// green, that is the answer to the escalated question and not a maintenance
// chore: rewrite it as a plain assertion with the reason recorded — do not
// delete it, and do not leave a `should_panic` that passes because loom timed
// out or reported something else (`expected` already blocks that second
// failure mode).
// =========================================================================

#[cfg(feature = "ke16-w-gate")]
#[test]
#[should_panic(expected = "M4: lost wake")]
fn loom_m4c_stale_snapshot_strands_a_task_when_the_pusher_is_not_a_lane() {
    static SNAPSHOTS: StdAtomicUsize = StdAtomicUsize::new(0);
    static CASCADES: StdAtomicUsize = StdAtomicUsize::new(0);
    m4_model(M4Config {
        items: 3,
        gate: M4Gate::AtMostOne,
        exclusion: M4Exclusion::SelfExcluded,
        pusher: M4Pusher::NeverLooksAgain,
        snapshots_above_one: &SNAPSHOTS,
        cascades: &CASCADES,
    });
}

// =========================================================================
// M1c — KE16 W-d′: count-gated completion for a WORKER joiner
// (`ke16-w-count`, `KE16-DESIGN-W.md` §3.7).
//
// M1 models the external arm: an unconditional `unpark` before the decrement,
// and a joiner that re-polls with `yield_now` — a shape that terminates whatever
// the wake does, and therefore cannot observe a lost wake at all. M1c models the
// arm where the token IS the property, so it parks FOR REAL:
//
//   - the target is a model-owned `Box<WakeHandle>` — the loom counting newtype
//     over `loom::thread::Thread` — declared before everything else in the
//     closure, so it outlives every completer that unparks through it. That is
//     the model's stand-in for `inner.workers[wid].thread`, which `PoolInner`
//     owns and which outlives the `ScopeShared` the decrement releases. No
//     production `WakeHandle` exists under loom: `PoolInner::joiner_wake_target`
//     returns null there, and no pool can be built under loom anyway.
//   - `complete_task` is the REAL production method (C1) through
//     `LoomScopeShared`, taking its `ke16-w-count` arm because the target is
//     non-null: `pending.fetch_sub(1, AcqRel)` FIRST, `unpark` only on
//     `prev == 1`.
//   - the joiner is `while !shared.is_drained() { thread::park(); }` — loom's
//     real park. Sound under the pinned loom 0.7.2, which persists a token
//     issued before the matching park (`rt/thread.rs` `set_unparked` ->
//     `Runnable { unparked: true }`, consumed by `rt::park`).
//
// Assertions: the model terminates (a lost wake here is a loom deadlock report,
// not an assertion); `unparks() == 1` — exactly one wake was issued, and by the
// `prev == 1` gate it can only be the last completer's; `completed == N`.
//
// If loom ever reports a false deadlock on that park and the joiner has to fall
// back to M1's yield re-poll, the row is recorded as "M1c-count: green (count
// only)" and the route-(b) many-seeds Miri run becomes the sole liveness gate
// (`KE16-DESIGN-MEASUREMENT.md` §5 item 16). It is NOT recorded as "M1c green".
// =========================================================================

#[cfg(feature = "ke16-w-count")]
#[test]
fn loom_m1c_count_gated_completion_wakes_the_parked_worker_joiner() {
    loom::model(|| {
        const N: usize = 2;

        // Declared FIRST so it is dropped LAST: the completers reach it through
        // a raw pointer stored in `ScopeShared`, exactly as production reaches
        // `PoolInner`-owned memory after the decrement.
        let target = Box::new(WakeHandle::new(thread::current()));
        let target_ptr: *const WakeHandle = &*target;

        // SAFETY: `target_ptr` points at the `Box` above, a local of this model
        //   closure that outlives `shared`, every spawned completer (all joined
        //   below) and therefore every `complete_task` that can dereference it.
        //   This is the model's discharge of the production invariant that
        //   `ScopeShared.joiner_wake` points into `PoolInner`-owned memory.
        let shared =
            Arc::new(unsafe { LoomScopeShared::new_worker_joined(thread::current(), target_ptr) });
        let queue: Arc<Mutex<VecDeque<usize>>> = Arc::new(Mutex::new(VecDeque::new()));
        let completed = Arc::new(AtomicU64::new(0));

        {
            let mut q = queue.lock().unwrap();
            for i in 0..N {
                shared.register_task();
                q.push_back(i);
            }
        }

        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let shared_cl = Arc::clone(&shared);
            let queue_cl = Arc::clone(&queue);
            let completed_cl = Arc::clone(&completed);
            handles.push(thread::spawn(move || {
                let item = { queue_cl.lock().unwrap().pop_front() };
                if item.is_some() {
                    // Accounted BEFORE the completion, so that at the moment the
                    // joiner observes drained, `completed` is already final.
                    completed_cl.fetch_add(1, Ordering::AcqRel);
                    shared_cl.complete_task();
                }
            }));
        }

        // The worker joiner's wait: check, then park. Under the count gate this
        // is race-free — a check that saw `pending >= 1` precedes the last
        // decrement in the RMW total order, and that decrement's unpark either
        // finds this thread parked or leaves the token its next park consumes.
        while !shared.is_drained() {
            thread::park();
        }

        assert!(shared.is_drained(), "M1c: join exited but pending != 0");

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            completed.load(Ordering::Acquire),
            N as u64,
            "M1c: completed != N at join exit (a task outlived the join wait)"
        );
        assert_eq!(
            target.unparks(),
            1,
            "M1c: the count gate must issue EXACTLY one wake per scope — the last \
             completer's; a different number means the `prev == 1` gate is not what fired"
        );
    });
}
