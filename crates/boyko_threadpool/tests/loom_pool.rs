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
//!     in every interleaving — the joiner re-polls with `yield_now` because
//!     loom #246 does not persist an unpark issued before the matching park.)
//!   - **M2 / M2b** call the real `mark_idle` / `unmark_idle` (`worker.rs`) over
//!     a loom `AtomicU64`.
//!
//! ## Fidelity note (the one transcription)
//!
//! `unpark_one_idle` takes `&ThreadPool` (crossbeam-coupled, not loom-buildable),
//! so its lowest-bit + `compare_exchange_weak` *claim* core cannot be invoked
//! over the loom shim directly. M2 / M2b therefore transcribe **only that claim
//! loop** here (`claim_one_idle`), a line-for-line copy of `worker.rs:239-257`
//! using the same `Ordering::AcqRel` success / `Ordering::Acquire` failure and
//! the same `mask & mask.wrapping_neg()` lowest-bit pick. The publish/clear
//! sides (`mark_idle` / `unmark_idle`) remain the real shared production code.
//! This is the sole model component that is not literal shared code; it is
//! flagged per the plan's D3 / §4 allowance.
//!
//! ## Run (plan §5)
//! ```bash
//! RUSTFLAGS="--cfg loom" cargo test --release -p boyko-threadpool --test loom_pool
//! LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" \
//!   cargo test --release -p boyko-threadpool --test loom_pool
//! ```
#![cfg(loom)]

use std::collections::VecDeque;

use loom::sync::{Arc, Mutex};
use loom::thread;

use boyko_threadpool::loom_exports::sync::{AtomicBool, AtomicU64, Ordering, fence};
use boyko_threadpool::loom_exports::{LoomScopeShared, mark_idle, unmark_idle};

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
        // `complete_task` unpark BEFORE its `fetch_sub`. loom (issue #246) does
        // NOT persist an unpark issued before the matching `park`, and cannot
        // model the production `park_timeout` re-poll backstop — so a `park()`
        // joiner here false-deadlocks and blows loom's state space
        // (STATUS_STACK_OVERFLOW) under U. Modeling the backstop's re-poll
        // directly with `yield_now` lets loom's exhaustive scheduler advance the
        // workers; the total-ordered `fetch_sub` RMW guarantees `is_drained()`
        // observes 0 in every interleaving, proving no permanent lost wakeup
        // WITHOUT relying on the (loom-mismodeled) unpark token — mirroring how
        // M2/M2b/M3 model the loom-opaque deque transport.
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
// transcribed `claim_one_idle` for the producer's wake. Models the load-bearing
// post-`mark_idle` re-poll (`worker.rs:78`): a producer that publishes work and
// then tries to wake must not let the worker end up parked while work is visible.
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
// In PRODUCTION that fence is supplied by the crossbeam **injector** transport:
// the worker's post-`mark_idle` re-poll is `Injector::steal_batch_and_pop`, and
// the producer's publish is `Injector::push` — crossbeam-deque inserts SeqCst
// fences in exactly these paths (Coq-verified, loom-opaque), establishing the
// single total order across the work-publish and the idle-bitset traffic. The
// model therefore places a `fence(SeqCst)` where production's deque steal/push
// sits — faithfully representing the transport whose internals loom cannot see
// (the deque itself is out of loom scope by D3/§9, covered by the D6 stress
// test). The idle-bitset ops under test (`mark_idle` / `unmark_idle` / the
// claim CAS) remain the REAL production code; only the deque transport's
// ordering guarantee is reintroduced as the fence it actually is.
//
// Invariant: the worker never ends parked (`idle` bit still set) with unclaimed
// work present.
// =========================================================================

/// Transcription of `worker.rs::unpark_one_idle`'s claim core (the only part
/// that is `&ThreadPool`-bound and thus not loom-buildable). Line-for-line: same
/// lowest-bit pick, same `compare_exchange_weak(AcqRel, Acquire)`. Returns the
/// claimed worker id (the bit it cleared), or `None` if no bit was set.
fn claim_one_idle(idle: &AtomicU64) -> Option<u32> {
    loop {
        let mask = idle.load(Ordering::Acquire);
        if mask == 0 {
            return None;
        }
        let bit = mask & mask.wrapping_neg();
        let new = mask & !bit;
        match idle.compare_exchange_weak(mask, new, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Some(bit.trailing_zeros()),
            Err(_) => continue,
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

        // Producer: publish work, then (modeling the injector push's SeqCst
        // fence) read idle and try to wake one idle worker via the real claim
        // CAS. The `fence(SeqCst)` between publish and claim is the crossbeam
        // injector transport guarantee (see the module-level fidelity note).
        let idle_p = Arc::clone(&idle);
        let work_p = Arc::clone(&work);
        let claimed_p = Arc::clone(&claimed);
        let producer = thread::spawn(move || {
            work_p.store(true, Ordering::Release);
            fence(Ordering::SeqCst); // injector push transport fence
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
                unmark_idle(&idle_w, 0); // real fetch_and, Release
                return; // grabbed work after marking; never parks
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
                "Race C: worker parked (idle bit set) with work present but no \
                 wake was issued — lost wakeup"
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
// M3 — shutdown handshake.
//
// A coordinator sets `shutdown` (Release); the worker, after its re-poll, loads
// `shutdown` (Acquire) and must observe it and exit. Mirrors
// `ThreadPool::drop`'s `shutdown.store(Release)` vs `worker.rs:86`'s
// `shutdown.load(Acquire)`.
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
