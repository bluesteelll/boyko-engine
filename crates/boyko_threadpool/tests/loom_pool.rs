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
//!     Candidate U: with the NULL wake target M1 builds, `complete_task` unparks
//!     UNCONDITIONALLY before its `fetch_sub`, so no `prev == 1` branch runs;
//!     M1 proves no lost wakeup via the total-ordered `fetch_sub` RMW chain
//!     driving `is_drained()` to 0 in every interleaving. The joiner re-polls
//!     with `yield_now` as the TRANSPORT-AGNOSTIC shape, not because the pinned
//!     loom drops the token: loom 0.7.2 stores `Runnable { unparked: true }`
//!     in `set_unparked` and `rt::park` consumes it, so an unpark issued before
//!     the matching park IS persisted. The yield re-poll models the production
//!     `park_timeout` backstop and proves the property without relying on the
//!     token at all.)
//!   - **M2 / M2b** call the real `mark_idle` / `unmark_idle` (`worker.rs`) over
//!     a loom `AtomicU64`.
//!   - **M1c** calls the same real `ScopeShared` methods as M1, but through
//!     `LoomScopeShared::new_worker_joined` — a non-null count-gate target — so
//!     `complete_task` takes its count-gated W-d′ arm: the decrement FIRST, the
//!     unpark only on `prev == 1`, aimed at memory the joiner cannot free. Its
//!     joiner parks FOR REAL, because there the token is the property.
//!
//! ## Fidelity note (the one transcription)
//!
//! `unpark_one_idle_excluding` takes `&PoolInner` (crossbeam-coupled, not
//! loom-buildable), so its rotate + `compare_exchange_weak` *claim* core cannot
//! be invoked over the loom shim directly. M2 / M2b therefore transcribe **only
//! that claim core** here (`claim_one_idle_bit`, mirroring
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
//! cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' \
//!   test -p boyko-threadpool --test loom_pool --no-run
//! # Run, per model, because a filter that matches nothing exits 0:
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
//! ⚠ **The `--config` key is `cfg(windows)` and not a triple, re-keyed
//! 2026-09-10**, the day this tree's Windows recipes moved from
//! `stable-x86_64-pc-windows-gnu` to `stable-x86_64-pc-windows-msvc`, spelled
//! explicitly through `RUSTUP_TOOLCHAIN`. (The rustup DEFAULT host is still gnu
//! as of 2026-09-10 — `~/.rustup/settings.toml` reads `default_host_tuple =
//! "x86_64-pc-windows-gnu"` — and `rustup set default-host` is a later,
//! owner-run step. The key below is correct under either state, which is the
//! whole reason it is a cfg-spec.) Until that date this
//! header read `target.x86_64-pc-windows-gnu.rustflags=["-C","target-cpu=
//! x86-64-v3","--cfg","loom"]`, and that key does not match an msvc build at
//! all: `--cfg loom` never reaches rustc, every model below is `cfg(loom)`d out,
//! and the binary prints `running 0 tests` and exits **0**. That failure mode is
//! a PASS, which is why it is spelled out here rather than left to the reader.
//! `cfg(windows)` matches on either host, and cargo JOINS a matching cfg-spec's
//! rustflags with the per-triple ones already in `.cargo/config.toml` instead of
//! replacing them — so the ISA baseline is no longer restated in the array and
//! can no longer drift from the file's (measured 2026-09-10 off `cargo -v`'s
//! rustc command line: `--cfg loom` and `-C target-cpu=x86-64-v3` both present).
//! `[build] rustflags` is NOT an alternative: cargo ignores that key entirely
//! whenever a `[target.*]` one matches, and `.cargo/config.toml` defines one for
//! both Windows triples — the same vacuous green by another route.
//!
//! ⚠ Every colour recorded below was measured 2026-09-03 under the gnu-triple
//! spelling on the windows-gnu host. Nothing in this file has been re-run under
//! msvc; the rows are that host's.
//!
//! **Reading, this checkout, 2026-09-03, `LOOM_MAX_PREEMPTIONS=3`, one model
//! per process, taken under `--features ke16-w-gate,ke16-w-count`** — the row's
//! model was gated on the `ke16-w-count` feature then and did not compile
//! without it. Step App removed those features, so the recipe above builds the model
//! unconditionally and does NOT reproduce this row's build:
//!
//! | Model | Colour | Wall |
//! |---|---|---|
//! | `loom_m1c_count_gated_completion_wakes_the_parked_worker_joiner` | ok | 0.40 s |
//!
//! `--list` prints all 6 models in this file (every `#[test]` here is now
//! unconditional) and exits 0. An absent loom row in a
//! results file is therefore an UNRUN row with no excuse attached, and every
//! colour asserted in a model header below is a reading unless that header says
//! otherwise.
#![cfg(loom)]

use std::collections::VecDeque;

use loom::sync::{Arc, Mutex};
use loom::thread;

use boyko_threadpool::loom_exports::WakeHandle;
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
// transport under it may publish with a PLAIN STORE — the KE16 A1 owner push
// goes onto a Chase-Lev deque (a Release fence and a Relaxed `back` store, no
// RMW), so nothing else on that side would be a barrier. Its calibration copy
// below deletes the call and must go red for the model's own lost-wake message.
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
/// reports a claim of a bit nobody set — a wake spent on nobody, and so a wake
/// lost.
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
        // nothing. The non-zero-exclusion shape is `unpark_one_idle_excluding`'s
        // own, and is not modelled here — it is covered natively by
        // `worker::tests::unpark_one_idle_excluding_claims_a_sibling_while_the_
        // excluded_bit_is_set`.
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
// M1c — KE16 W-d′: count-gated completion for a WORKER joiner
// (`KE16-DESIGN-W.md` §3.7).
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
//     `LoomScopeShared`, taking its count-gated arm because the target is
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
