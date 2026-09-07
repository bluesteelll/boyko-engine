//! Phase 9.1 W3b — Miri-1: the `Scope::spawn` transmute on real worker threads
//! (plan D4 / D5 / D5.1 / H4).
//!
//! This is the soundness crux of Phase 9.1. It drives, under Miri, three
//! surfaces: (1) the `'scope -> 'static` lifetime erasure in `Scope::spawn`
//! (`Scope::prepare` -> `Task::new_scoped`; the `body_static` binding this line
//! used to name is gone with the `transmute`); (2) the cross-thread raw deref of a task's
//! `*const ScopeShared` on a **worker** thread; and (3) the `pending` join +
//! waker unpark + `ThreadPool::drop` shutdown/join.
//!
//! Surface (2) has been rewritten TWICE by fixes it outlived, and both are
//! recorded because the surface is the same memory each time — what changes is
//! how much of a reference anyone holds into it:
//!   1. It was "the `SharedPtr::as_ref` raw deref of `&ScopeShared`". KE16
//!      replaced that method with `SharedPtr::as_ptr`, handing out an address
//!      and minting no reference.
//!   2. `SharedPtr` itself is now GONE, with the `Box`ed task-body wrapper that
//!      was its only consumer. A scoped task is `task::run_scoped`, which reads
//!      the scope's ADDRESS out of its payload cell by place-read, takes one
//!      short reborrow for `capture_panic` (cold, strictly before the
//!      decrement) and NONE for the completion —
//!      `ScopeShared::complete_task(*const Self)`.
//!
//! Surface (1)'s `transmute` is likewise gone in form, not in obligation: the
//! erasure is now the coercion of `run_scoped::<F>` to `unsafe fn(*const ())`,
//! and this test drives it unchanged. See `src/task.rs`.
//!
//! All of this runs on genuinely distinct OS threads so that Tree Borrows (the
//! project default via `.cargo/config.toml`) observes the worker-side foreign
//! write to `pending` while the dispatcher holds `&ScopeShared` across the join
//! wait — the exact pattern the Phase-9 closeout deferred (see
//! `crates/boyko_ecs/tests/miri_phase9.rs:15-26`).
//!
//! ## H4 — genuine cross-thread execution is FORCED and ASSERTED
//!
//! `Scope::drop`'s wait steals and runs tasks inline on the dispatcher, so a
//! small task set can be drained entirely by the dispatcher, leaving the
//! worker-side deref of the wrapper's `*const ScopeShared` (the TB target)
//! un-exercised — a *vacuous* pass.
//! Two defenses, per H4:
//!   1. **Forced interleave**: task A sets `flag_a` then waits for `flag_b`;
//!      task B waits for `flag_a` then sets `flag_b`. If the dispatcher tried to
//!      run both inline-serially it would block inside A's body waiting on
//!      `flag_b` (B not yet started) — forward progress is possible *only* if a
//!      worker thread claims the other task. A genuine failure to run
//!      cross-thread therefore surfaces as the bounded-spin panic, never a
//!      false green.
//!   2. **Recorded worker ids**: each body records `current_worker_id()`; the
//!      test asserts at least one body ran on a real worker
//!      (`id < WORKER_ID_DISPATCHER`), i.e. NOT inline on the dispatcher.
//!
//! ## RESOLVED in Phase 9.2 — data-race-clean + Tree-Borrows-clean
//!
//! The landed `NonNull<ScopeShared>` field refactor (the joiner takes a by-value
//! `*const ScopeShared`, so the dispatcher holds no protected `&ScopeShared`
//! across the workers' `pending.fetch_sub` writes) cleared the Tree-Borrows
//! protected-tag conflict that earlier `#[ignore]`d these tests. That refactor
//! then unmasked a *separate, real* data race: the last completer read
//! `ScopeShared.waker` (non-atomic) AFTER its `pending.fetch_sub -> 0`, while the
//! dispatcher, seeing `pending == 0`, freed the box — Miri's data-race checker
//! flagged the post-decrement `waker` read vs the dispatcher's dealloc.
//!
//! Phase 9.2 fixes that with **Candidate U (unpark-before-decrement)**:
//! `ScopeShared::complete_task` calls `waker.unpark()` BEFORE
//! `pending.fetch_sub(1, AcqRel)`. While this task has not yet decremented,
//! `pending >= 1`, so the box cannot have been freed — the `waker` read is sound;
//! the `fetch_sub` is then the worker's LAST byte-access to the allocation.
//!
//! ⚠ THAT LAST CLAUSE WAS TRUE AND WAS NOT THE WHOLE OBLIGATION, and this header
//! asserted it as if it were — the sentence "All seeds are UB-clean" below was
//! read for a year as "the completion path is protector-clean", which it never
//! established. "Last byte-access" answers the DATA-RACE rule. The rule that
//! judges a deallocation is the PROTECTOR rule: Tree Borrows forbids freeing a
//! range covered by a live strong protector, and a protector lives as long as
//! the CALL whose reference-typed argument it guards, not as long as that call's
//! accesses. `complete_task(&self)` satisfied the byte-access clause while its
//! own `&self` protector was still live past the decrement that authorised the
//! free — a foreign write to the allocation's FROZEN bytes (`waker`,
//! `joiner_wake`, `CachePadded` padding). What establishes the property now is a
//! by-value receiver, `complete_task(shared: *const ScopeShared)`, mirroring the
//! joiner-side shape described two paragraphs up; the gate that decides it is
//! `tests/miri_scope_completion_protector.rs`, which is red on a reference
//! receiver and green on the shipped one, on every seed tried.
//!
//! MEASURED 2026-09-05, and the two halves point opposite ways, so neither is
//! the whole story:
//!   * THIS FILE, as it stood for the whole life of that claim, could not have
//!     caught it. With a deliberately reintroduced `&self` receiver and no
//!     scheduling probe, all 5 active tests here are GREEN at seeds 0 and 7 —
//!     the window is a couple of MIR steps wide and nothing here forced a
//!     schedule into it.
//!   * With the `#[cfg(miri)]` release probe that
//!     `ScopeShared::complete_task` now carries for the gate above,
//!     `miri_scope_forced_cross_thread_transmute_is_clean` DOES report the same
//!     `deallocation through <TAG> ... is forbidden`, at the DEFAULT preemption
//!     rate. So this file is now a second carrier of the property rather than a
//!     blind spot — not because the tests changed, but because the probe made
//!     the window reachable. The probe is `cfg(miri)`-only and costs the shipped
//!     artifact nothing.
//!
//! The box is freed UNCONDITIONALLY at the single `Scope::drop` site after the join
//! (tied to scope END, never to an intermediate wave's `pending -> 0` — that is
//! what makes it multi-drain-safe; an earlier `free_state` "second-swapper-frees"
//! handshake was abandoned because it double-freed across the executor's
//! per-wave `pending` oscillation). No handshake, no extra atomics, no
//! per-scope flag — a net deletion. These tests are now un-`#[ignore]`d and pass
//! under `-Zmiri-tree-borrows` at the default seed (data-race checker AND Tree
//! Borrows clean). NOTE: under `-Zmiri-many-seeds`, ~1/16 adversarial seeds hit a
//! *liveness* timeout (NOT UB) — Candidate U's lost-wakeup window, which Miri
//! cannot recover from because it does not model the `park_timeout` backstop that
//! recovers it on real hardware (bench-proven). All seeds are UB-clean. That
//! reading is EXTERNAL-ARM ONLY and stays true for these tests, because every
//! one of them joins from the test thread: an external joiner keeps the
//! unpark-before-decrement order and hence the window, on every build. The
//! WORKER-joiner arm is count-gated under `ke16-w-count` (the decrement first,
//! the unpark aimed at `PoolInner`-owned memory, KE16 W-d′), which closes that
//! window on route (b), and it is gated at ZERO liveness timeouts by
//! `nested_scope_from_worker_is_stolen_by_sibling` — the one test in this file
//! whose only join is a worker's, and only in a build that ALSO carries an A
//! arm, because the shape needs a sibling to reach the wave (see that test's
//! `# The W-d′ liveness gate` section: without one it now fails loudly rather
//! than skipping). A timeout in the three tests above is neither a W-d′ failure
//! nor a W-d′ pass (`KE16-DESIGN-MEASUREMENT.md` §5 item 13).
//!
//! ## Run (plan §5 / §12)
//! ```bash
//! # PRIMARY boyko surface — data-race checker AND Tree Borrows clean.
//! # `-Zmiri-permissive-provenance` isolates crossbeam's exposed-provenance
//! # int-to-ptr noise (third-party deque transport, out of the boyko proof
//! # surface) from boyko's own `scope.rs` frames.
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-many-seeds=0..16 \
//!   -Zmiri-permissive-provenance -Zmiri-ignore-leaks" \
//!   cargo +nightly miri test -p boyko-threadpool --test miri_scope
//! ```
//!
//! `-Zmiri-ignore-leaks` is STILL required after Phase 9.3b, but NOT because of
//! boyko: the 3 tests themselves pass clean (`test result: ok. 3 passed`), and
//! Phase 9.3b's `ThreadPool::drop` now genuinely joins the workers (visible as
//! `crossbeam_epoch::LocalHandle::drop` TLS-destructor frames running at join).
//! The residual "memory leaked" reports are entirely inside `crossbeam_epoch`
//! 0.9.18 — its epoch-GC `HANDLE` thread-local and the `SealedBag`s pushed into
//! the global retire-queue at thread exit are not reclaimed at process teardown.
//! That is a third-party deque-GC at-exit artifact, independent of the pool's
//! own lifecycle (the pool, `ScopeShared`, and `PoolInner` are all freed). So
//! 9.3b removed the *pool's* leak but cannot remove crossbeam-epoch's; the flag
//! stays for that reason alone.
#![cfg(miri)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use boyko_threadpool::{
    ThreadPoolBuilder, WORKER_ID_DISPATCHER, current_worker_id, try_with_active_pool,
};

/// Bounded cooperative spin. Yields so Miri can advance other threads; panics
/// (rather than hanging Miri forever) if the awaited flag never arrives, so a
/// genuine lost-wakeup / no-cross-thread-progress becomes a test FAILURE.
fn spin_until(flag: &AtomicBool, ctx: &str) {
    // Generous cap: under Miri each iteration yields, so a healthy run needs
    // only a handful of turns; an unhealthy one is bounded instead of hanging.
    for _ in 0..100_000 {
        if flag.load(Ordering::Acquire) {
            return;
        }
        std::thread::yield_now();
    }
    panic!("spin_until timed out waiting for {ctx} — no cross-thread progress");
}

/// Returns true iff `id` denotes a genuine worker thread (not the dispatcher,
/// not the unattached sentinel).
fn on_worker(id: u32) -> bool {
    id != WORKER_ID_DISPATCHER && id != u32::MAX
}

/// H4 core: two mutually-dependent tasks that can only both complete if they
/// run on different threads. Records which worker executed each body and the
/// values written through borrowed stack slots.
#[test]
fn miri_scope_forced_cross_thread_transmute_is_clean() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    // Distinct stack slots borrowed by the task bodies (the 'scope borrow the
    // transmute erases). If the transmute or the join were unsound, Miri would
    // report UB/UAF on these.
    let flag_a = AtomicBool::new(false);
    let flag_b = AtomicBool::new(false);
    let out_a = AtomicU32::new(0);
    let out_b = AtomicU32::new(0);
    let wid_a = AtomicU32::new(u32::MAX);
    let wid_b = AtomicU32::new(u32::MAX);

    pool.install(|scope| {
        scope.spawn(|| {
            wid_a.store(current_worker_id(), Ordering::Release);
            out_a.store(0xAA, Ordering::Release);
            flag_a.store(true, Ordering::Release);
            // Can only complete once task B (on another thread) sets flag_b.
            spin_until(&flag_b, "flag_b in task A");
        });
        scope.spawn(|| {
            wid_b.store(current_worker_id(), Ordering::Release);
            spin_until(&flag_a, "flag_a in task B");
            out_b.store(0xBB, Ordering::Release);
            flag_b.store(true, Ordering::Release);
        });
    });

    // Correct results (the borrowed stack slots survived the cross-thread run).
    assert_eq!(out_a.load(Ordering::Acquire), 0xAA, "task A wrote its slot");
    assert_eq!(out_b.load(Ordering::Acquire), 0xBB, "task B wrote its slot");

    // H4 assertion: at least one body ran on a genuine worker thread (not
    // inline on the dispatcher). A dispatcher-only (vacuous) run is a FAILURE.
    let ran_a = wid_a.load(Ordering::Acquire);
    let ran_b = wid_b.load(Ordering::Acquire);
    assert!(
        on_worker(ran_a) || on_worker(ran_b),
        "H4: neither body ran on a worker thread (a={ran_a}, b={ran_b}); the \
         cross-thread transmute path was not exercised — vacuous pass"
    );
}

/// A spawn whose body reads-then-writes a borrowed stack `AtomicUsize`, forced
/// to wait on a gate the *second* task opens. Exercises the read-modify-write of
/// borrowed stack data across the transmute on (at least sometimes) a worker.
#[test]
fn miri_scope_read_modify_write_borrowed_stack() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    let accum = AtomicUsize::new(1);
    let gate = AtomicBool::new(false);

    pool.install(|scope| {
        // Task 1: wait for the gate, then RMW the borrowed stack slot.
        scope.spawn(|| {
            spin_until(&gate, "gate in RMW task");
            let prev = accum.fetch_add(41, Ordering::AcqRel);
            assert_eq!(prev, 1, "RMW observed the initial value");
        });
        // Task 2: open the gate (must run before task 1 can finish).
        scope.spawn(|| {
            gate.store(true, Ordering::Release);
        });
    });

    assert_eq!(
        accum.load(Ordering::Acquire),
        42,
        "borrowed stack slot reflects the cross-thread RMW"
    );
}

/// Multiple tasks each borrowing a *distinct* stack slot, rendezvousing on a
/// `go` gate so several worker-side accesses to the shared allocation overlap
/// the dispatcher's join wait.
///
/// The overlap this shape was built to create no longer exists in the form the
/// name suggests, and the doc is corrected rather than the test: NEITHER side
/// holds a reference across the other's writes any more. The dispatcher takes
/// `*const ScopeShared` by value into `join_workers_until_drained` (Phase 9.2),
/// and since KE16 the workers take an address too — the reborrows that used to
/// overlap are now per-statement on both sides. What still overlaps, and is
/// still worth driving, is the concurrent ACCESS: several workers writing
/// `pending` and the borrowed stack slots while the joiner polls.
#[test]
fn miri_scope_multiple_distinct_borrows() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    let slots = [
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
    ];
    let started = AtomicUsize::new(0);
    let go = AtomicBool::new(false);

    pool.install(|scope| {
        // Shadow as shared references so the `move` closures capture the (Copy)
        // references rather than moving the non-Copy atomics into the first
        // iteration's closure.
        let started = &started;
        let go = &go;
        for (i, slot) in slots.iter().enumerate() {
            scope.spawn(move || {
                started.fetch_add(1, Ordering::AcqRel);
                // All tasks rendezvous on `go` so their bodies overlap in time
                // (concurrent worker-side reborrows of ScopeShared). Bounded so
                // a stuck scheduler fails instead of hanging.
                spin_until(go, "go gate");
                slot.store((i as u32) + 1, Ordering::Release);
            });
        }
        // Release everyone once spawned. The dispatcher then enters the join
        // wait (holding &ScopeShared) while workers run the bodies.
        go.store(true, Ordering::Release);
    });

    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(
            slot.load(Ordering::Acquire),
            (i as u32) + 1,
            "slot {i} written by its task across the transmute"
        );
    }
    assert_eq!(
        started.load(Ordering::Acquire),
        slots.len(),
        "every task body started"
    );
}

// ===========================================================================
// KE16 — the two nested-scope Miri shapes (`KE16-DESIGN-A.md` §1.3, listed for
// the base commit by `KE16-DESIGN-MEASUREMENT.md` §6).
//
// Both were RED-FIRST and unconditionally `#[ignore]`d until the A-axis push
// arms landed. They now run in exactly the builds whose arm makes their claim
// true, and stay ignored in the others — the `cfg_attr` on each names which:
//
//   * the reachability shape runs under every A arm (`ke16-a1`,
//     `ke16-a1-fifo`, `ke16-a2`, `ke16-a3`, and `ke16-a5`, which implies
//     `ke16-a2`), because each of them closes reachability by its own route;
//     in the DEFAULT build it stays ignored, since defect A denies its
//     assertion and running it would report a defect the tournament exists to
//     remove;
//   * the TLS-deque receipt shape runs only under `ke16-a1` / `ke16-a1-fifo`,
//     the arms that have a TLS deque. In any other build it would pass by
//     draining the injector path and certify it under the deque's name — a
//     green worth nothing.
//
// Run them under `ke16-a1` / `ke16-a1-fifo` with B0 and with B1
// (`KE16-DESIGN-MEASUREMENT.md` §8).
//
// How the first one fails while defect A stands, so the reading is not mistaken
// for a hang: the nested bodies deadlock on their handshake (the outer worker is
// the only thread that can reach them and it can only run one at a time), so the
// bounded `spin_until` panics inside a body of a DETACHED `pool.spawn`, which
// the pool answers by aborting the process (`abort_on_task_panic`). An abort
// with the `spin_until timed out` message ahead of it is the red-first reading;
// a Tree-Borrows `error: Undefined Behavior:` report is the different, real one
// (`KE16-DESIGN-MEASUREMENT.md` §5 item 14 classifies the three kinds).
// ===========================================================================

/// Refuses to let the W-d′ route-(b) liveness gate report a result it did not
/// measure: panics naming the skip, in exactly the builds that claim the gate
/// but cannot run it (`ke16-w-count` with no A arm).
///
/// Why a panic and not a wider `ignore`: the silence IS the defect — an ignored
/// gate whose result is being counted reads as a pass. Why not simply let the
/// model run: under A0 the two handshaked nested bodies are reachable only by
/// the one worker that spawned them, so the run would end in `spin_until timed
/// out` inside a detached body and the pool would abort the process — the same
/// output a genuine lost wake produces, which is exactly the reading the gate
/// exists to take. A false red indistinguishable from the signal is no better
/// than the false green it replaces, so this panic carries its own literal
/// (`W-d′ route-(b) liveness gate DID NOT RUN`) and the two cannot be confused.
///
/// Cold per ERG-28, but deliberately NOT `-> !`: the other `cfg` arm of this
/// name is a no-op, and a diverging signature here would type the whole model
/// body below the call site as unreachable in exactly the build that runs it.
#[cfg(not(any(
    feature = "ke16-a1",
    feature = "ke16-a1-fifo",
    feature = "ke16-a2",
    feature = "ke16-a3"
)))]
#[cold]
fn refuse_to_certify_without_a_reachability_arm() {
    panic!(
        "W-d′ route-(b) liveness gate DID NOT RUN in variant `{}`: the shape needs a SIBLING to \
         take one of the two handshaked nested bodies, and this build carries no A arm, so a \
         worker-spawned wave never leaves the lane it was spawned on (defect A) and the shape \
         would hang rather than measure. Re-run the `KE16-DESIGN-MEASUREMENT.md` §8 recipe with \
         an A arm as well, e.g. `--features ke16-w-count,ke16-a2`. No W-d′ liveness row may be \
         filed from this build (`KE16-DESIGN-W.md` §3.7).",
        boyko_threadpool::ke16_variant()
    );
}

/// No-op: an A arm closes the reachability the shape needs, so the model itself
/// runs and IS the evidence.
#[cfg(any(
    feature = "ke16-a1",
    feature = "ke16-a1-fifo",
    feature = "ke16-a2",
    feature = "ke16-a3"
))]
fn refuse_to_certify_without_a_reachability_arm() {}

/// KE16 A-axis Miri gate 1: a scope opened from inside a worker's task body
/// must be reachable by a SIBLING worker.
///
/// Shape (design §1.3 gate 1): the test thread uses `pool.spawn`, so the outer
/// body is guaranteed to run on a worker and the only join in the test is that
/// worker's own — which is also what makes this test the W-d′ route-(b)
/// liveness gate. The two nested bodies are locked into the H4 handshake, so
/// the joining worker alone cannot finish them: forward progress exists only if
/// a sibling takes one. Under Tree Borrows this is the shape that puts a
/// sibling's `Stealer::steal_batch_and_pop` against a deque whose owner reaches
/// it through the A1 TLS pointer.
///
/// # The W-d′ liveness gate (`KE16-DESIGN-W.md` §3.7, `-MEASUREMENT.md` §8)
///
/// Under `ke16-w-count` this test's ONE join is count-gated, so a liveness
/// timeout here is a lost wake in the gated arm — a defect to fix, never a
/// reason to switch designs (W17 keeps the unpark-before-decrement order and
/// would show the same timeout). Required: `running 1 test`, ZERO timeouts
/// across 32 seeds. The full flag string, which REPLACES the `[env]` default of
/// `.cargo/config.toml` rather than merging with it:
///
/// ```powershell
/// $env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-many-seeds=0..32"
/// Write-Output "MIRIFLAGS=$env:MIRIFLAGS"
/// cargo +nightly miri test -p boyko-threadpool --test miri_scope nested_scope_from_worker_is_stolen_by_sibling --features ke16-w-count,ke16-a2
/// Remove-Item Env:MIRIFLAGS
/// ```
///
/// An echoed string without `-Zmiri-tree-borrows` ran under Stacked Borrows and
/// one without `-Zmiri-many-seeds=` measured a single seed; either way the
/// result is discarded and the run repeated (refused shape 12).
///
/// **The feature list must name an A arm as well as `ke16-w-count`** — any of
/// `ke16-a1`, `ke16-a1-fifo`, `ke16-a2`, `ke16-a3`, `ke16-a5`; `ke16-a2` above
/// only because it closes reachability with the fewest other changes. MEASURED
/// 2026-09-03, the defect this paragraph and
/// [`refuse_to_certify_without_a_reachability_arm`] exist to close: the recipe
/// as `-MEASUREMENT.md` §8 wrote it (`--features ke16-w-count`, no A arm)
/// printed `running 1 test` and `test result: ok. 0 passed; 0 failed; 1 ignored`
/// on every one of the 32 seeds and exited 0, because the `cfg_attr` below
/// ignored the test unless an A arm was on. A build with no A arm still cannot
/// execute the shape (A0 does not deliver a worker-spawned wave to a sibling, so
/// the H4 handshake would hang instead of measuring), but it no longer passes:
/// the ignore lifts under `ke16-w-count` and the first line of the body panics
/// with what was skipped and why.
///
/// # What this run IS for W-d′, and what it is not
///
/// `KE16-DESIGN-W.md` §3.7 makes this run W-d′'s SOLE liveness evidence under a
/// CONDITION — "whenever loom M1c degrades to count-only" — and that condition
/// is NOT met at this checkout, so the fallback branch must not be applied when
/// filing the W-d′ row. MEASURED 2026-09-03, this worktree: M1c runs here with
/// a REAL parked loom joiner (loom's own `park()`, not the M1 yield re-poll) and
/// is green — `loom_m1c_count_gated_completion_wakes_the_parked_worker_joiner`,
/// `running 1 test … ok`, 0.40 s. Recipe, in full because the obvious spelling
/// is a trap: `cargo --config 'target.x86_64-pc-windows-gnu.rustflags=["-C",
/// "target-cpu=x86-64-v3","--cfg","loom"]' test -p boyko-threadpool --test
/// loom_pool --features ke16-w-gate,ke16-w-count --no-run`, then the emitted
/// `loom_pool-<hash>.exe` under `LOOM_MAX_PREEMPTIONS=3 --test-threads=1
/// --exact <name>`; `tests/loom_pool.rs`'s header carries it verbatim together
/// with every model's colour and wall. W-d′ therefore HAS its exhaustive model,
/// and this Miri run is CORROBORATION on real hardware, not a substitute for
/// one.
///
/// Retracted 2026-09-03, kept here so it cannot be re-derived: this paragraph
/// used to assert "on this box M1c cannot be executed at all", and concluded
/// that W-d′'s only liveness gate was a run that never happened. That was a
/// RECIPE fault, not a box fault — `RUSTFLAGS="--cfg loom" cargo test …`
/// REPLACES `[target.<triple>].rustflags` instead of merging with it, so it
/// links a differently configured tree that dies at startup before libtest
/// prints anything. §3.7's fallback stays the conditional it always was: a
/// reader who does hit a degraded M1c must name the degradation in the results
/// file first, and only then may this run be filed as the sole gate.
#[test]
#[cfg_attr(
    not(any(
        feature = "ke16-a1",
        feature = "ke16-a1-fifo",
        feature = "ke16-a2",
        feature = "ke16-a3"
    )),
    ignore = "deferred: KE16 — asserts a worker-spawned wave reaches a sibling, which defect A \
              denies in the DEFAULT build. It runs under every A arm (ke16-a1, ke16-a1-fifo, \
              ke16-a2, ke16-a3, and ke16-a5 through the a2 it implies), each of which closes \
              that reachability. Under ke16-w-count WITH NO A ARM the shape needs a SIBLING to \
              take one of the two handshaked nested bodies and has none, so it would hang rather \
              than measure: NO W-d-prime liveness row may be filed from that build \
              (KE16-DESIGN-W.md 3.7). Re-run the KE16-DESIGN-MEASUREMENT.md 8 recipe with an A \
              arm as well, e.g. --features ke16-w-count,ke16-a2. Forcing this test with \
              --ignored in such a build does not evade that: the body's first statement is \
              refuse_to_certify_without_a_reachability_arm(), which panics with the same \
              information."
)]
fn nested_scope_from_worker_is_stolen_by_sibling() {
    refuse_to_certify_without_a_reachability_arm();

    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    let done = Arc::new(AtomicBool::new(false));
    let outer_wid = Arc::new(AtomicU32::new(u32::MAX));
    let wid_a = Arc::new(AtomicU32::new(u32::MAX));
    let wid_b = Arc::new(AtomicU32::new(u32::MAX));

    {
        let done = Arc::clone(&done);
        let outer_wid = Arc::clone(&outer_wid);
        let wid_a = Arc::clone(&wid_a);
        let wid_b = Arc::clone(&wid_b);
        pool.spawn(move || {
            outer_wid.store(current_worker_id(), Ordering::Release);

            // Stack slots of the WORKER's frame, borrowed by the nested bodies:
            // the nested scope's lifetime erasure is over this frame, not the
            // test thread's.
            let flag_a = AtomicBool::new(false);
            let flag_b = AtomicBool::new(false);

            let dispatched = try_with_active_pool(|inner| {
                inner.scope(|nested| {
                    nested.spawn(|| {
                        wid_a.store(current_worker_id(), Ordering::Release);
                        flag_a.store(true, Ordering::Release);
                        spin_until(&flag_b, "flag_b in nested body A");
                    });
                    nested.spawn(|| {
                        wid_b.store(current_worker_id(), Ordering::Release);
                        spin_until(&flag_a, "flag_a in nested body B");
                        flag_b.store(true, Ordering::Release);
                    });
                });
            });
            assert!(
                dispatched.is_some(),
                "the outer body must run inside an active pool frame"
            );
            done.store(true, Ordering::Release);
        });
    }

    spin_until(&done, "the worker's nested scope to drain");

    let outer = outer_wid.load(Ordering::Acquire);
    let a = wid_a.load(Ordering::Acquire);
    let b = wid_b.load(Ordering::Acquire);
    assert!(
        on_worker(outer),
        "the outer body must have run on a worker (got {outer}); a dispatcher-run outer body \
         measures the healthy route twice"
    );
    assert!(
        on_worker(a) && on_worker(b),
        "both nested bodies must have run on workers (a={a}, b={b})"
    );
    assert!(
        a != outer || b != outer,
        "both nested bodies ran on the spawning worker {outer} — the wave never left the lane \
         it was spawned on, which is defect A"
    );
}

/// KE16 A-axis Miri gate 2: a body running INLINE on the joining worker opens
/// its own scope and pushes through the same TLS deque slot while the outer
/// join is live — the protector-bearing shape D5 exists to answer.
///
/// `2 * MIN_CAP + 1 = 129` pushes per inner scope force at least one
/// `Buffer` resize (crossbeam `MIN_CAP = 64`), which is the ONE write to the
/// `Worker` allocation's own bytes; that write must not conflict with any tag
/// minted on the deque earlier in the outer frame. The RECEIPT that the shape
/// was actually exercised is a nested-spawning body reporting the outer
/// worker's id; per design §1.3 it is deterministic only under `ke16-a1` +
/// `ke16-b1`, so the whole shape is retried on a fresh pool up to four times
/// and a miss panics with the literal `receipt not observed`, which is red
/// kind (c) — NOT a defect — and is re-run at `-Zmiri-many-seeds=0..64`.
#[test]
#[cfg_attr(
    not(any(feature = "ke16-a1", feature = "ke16-a1-fifo")),
    ignore = "deferred: KE16 — the receipt names the A1 TLS deque, which only ke16-a1 / \
              ke16-a1-fifo push through; in any other build it would drain the injector path \
              and certify it under the deque's name."
)]
fn nested_scope_inline_body_spawns_through_tls_deque_under_live_join() {
    /// crossbeam's `MIN_CAP` is 64; `2 * 64 + 1` pushes force a resize.
    const NESTED: usize = 129;
    const ATTEMPTS: usize = 4;

    for _ in 0..ATTEMPTS {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();

        let done = Arc::new(AtomicBool::new(false));
        let outer_wid = Arc::new(AtomicU32::new(u32::MAX));
        // Each outer body records the worker it ran on, and how many of its own
        // nested tasks completed.
        let body_wid = Arc::new([AtomicU32::new(u32::MAX), AtomicU32::new(u32::MAX)]);
        let body_count = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);

        {
            let done = Arc::clone(&done);
            let outer_wid = Arc::clone(&outer_wid);
            let body_wid = Arc::clone(&body_wid);
            let body_count = Arc::clone(&body_count);
            pool.spawn(move || {
                outer_wid.store(current_worker_id(), Ordering::Release);
                let dispatched = try_with_active_pool(|inner| {
                    inner.scope(|outer| {
                        for slot in 0..2usize {
                            let body_wid = Arc::clone(&body_wid);
                            let body_count = Arc::clone(&body_count);
                            outer.spawn(move || {
                                body_wid[slot].store(current_worker_id(), Ordering::Release);
                                try_with_active_pool(|inner| {
                                    inner.scope(|nested| {
                                        for _ in 0..NESTED {
                                            let body_count = Arc::clone(&body_count);
                                            nested.spawn(move || {
                                                body_count[slot].fetch_add(1, Ordering::AcqRel);
                                            });
                                        }
                                    });
                                })
                                .expect("a task body always has an active pool");
                            });
                        }
                    });
                });
                assert!(dispatched.is_some(), "the outer body needs a pool frame");
                done.store(true, Ordering::Release);
            });
        }

        spin_until(&done, "the two nested-spawning bodies to drain");

        for slot in 0..2usize {
            assert_eq!(
                body_count[slot].load(Ordering::Acquire),
                NESTED,
                "body {slot} did not complete its {NESTED} nested tasks"
            );
        }

        let outer = outer_wid.load(Ordering::Acquire);
        let ran_inline = (0..2usize).any(|s| body_wid[s].load(Ordering::Acquire) == outer);
        if ran_inline {
            return; // receipt observed: a nested-spawning body ran on the joining worker
        }
    }
    panic!(
        "receipt not observed: in {ATTEMPTS} attempts no nested-spawning body ran on the \
         joining worker, so the protector-bearing inline shape was never exercised"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// KE16 App-4 (`ke16-c-batch`) — the batch wave under Tree Borrows
// ═══════════════════════════════════════════════════════════════════════════
//
// `Scope::spawn_batch` is the ONE place in `scope.rs` that departs from
// `Scope::spawn`'s transient-reborrow discipline: `spawn` bounds
// `unsafe { self.shared.as_ref() }` to a single expression, while the batched
// arm mints one `&ScopeShared` and holds it across the whole push loop, handing
// it to the `WaveRegistration` guard (`scope.rs`, the `ke16-c-batch` arm of
// `spawn_batch`). Every byte a worker writes through that allocation while the
// reference is live is inside an `UnsafeCell` — `pending`'s `AtomicUsize` and
// `panic_payload`'s `AtomicPtr` — so the shape is sound, and Tree Borrows
// tracks interior mutability byte-precisely, which is exactly why THIS model is
// the one that can decide it.
//
// Nothing in the rest of this file reaches `spawn_batch`: every other shape
// calls `Scope::spawn`. Without the two tests below, the Miri leg of the C
// axis's gate (`KE16-DESIGN-MEASUREMENT.md` §8) compiles the batched arm and
// executes none of it — a green over code that never ran.
//
// Both run in BOTH arms. Without `ke16-c-batch` `spawn_batch` is one `spawn`
// per body and the long-lived reference does not exist, so a green there is the
// control and a green under the feature is the result.

/// A batch wave whose worker completion — the `pending` RMW through the
/// allocation the spawner is holding a `&ScopeShared` to — is FORCED to land
/// while the spawner is still inside its push loop.
///
/// The iterator is the instrument, as it is for the native wake receipt: while
/// producing body 1 the spawner waits, bounded, for body 0 to have finished.
/// Body 0 finishing IS `ScopeShared::complete_task`'s `fetch_sub` on `pending`,
/// so the interleaving Tree Borrows is asked about — a worker writing the
/// allocation under the spawner's live reborrow — is not left to chance.
///
/// ⚠ **This shape is ALSO a liveness gate on the wave's wake, and as of
/// 2026-09-04 it is RED under `ke16-w-fanout`.** The spawner here deliberately
/// blocks BEFORE `Scope::drop`, so it cannot help-steal its own wave: the only
/// thing that can complete body 0 is a sibling the wave's ONE wake decision
/// woke. A wake decision that wakes nobody therefore shows up as
/// `spin_until timed out` — `KE16-DESIGN-MEASUREMENT.md` §5 item 14 kind (b),
/// a liveness defect in the wake protocol, never a Tree-Borrows result.
/// MEASURED on `-Zmiri-many-seeds=0..64`, one filtered test per run:
///
/// | features | `KE16_C` | timeouts / 64 seeds |
/// |---|---|---|
/// | (none) | `c0` | 0 |
/// | `ke16-c-batch` | `c1` | 0 |
/// | `ke16-w-gate,ke16-c-batch` | `c1` | 0 |
/// | `ke16-w-fanout` | `c1f` | **1** (seed 31) |
/// | `ke16-w-gate,ke16-w-fanout` | `c1f` | **1** |
///
/// Single-seed reproduction, deterministic — the same seed passes with
/// `--features ke16-c-batch` and `--features ke16-w-gate,ke16-c-batch`:
///
/// ```text
/// MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-seed=31"
/// cargo +nightly miri test -p boyko-threadpool --test miri_scope ///     --features ke16-w-fanout miri_spawn_batch_worker_completion
/// ```
///
/// No production caller blocks the way this one does — every `spawn_batch`
/// caller returns and lets `Scope::drop` join, and the joining thread helps —
/// so the production consequence of the same window is a LOST LANE rather than
/// a hang: the wave runs on the joiner while a parked sibling sleeps, which is
/// the throughput this campaign exists to measure. The test is left un-ignored
/// and red because that is the reading the design asks for.
#[test]
fn miri_spawn_batch_worker_completion_lands_under_the_spawners_live_reborrow() {
    const N: usize = 4;

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let first_done = AtomicBool::new(false);
    let ran = AtomicUsize::new(0);
    let observed_completion_mid_wave = AtomicBool::new(false);

    // Shared as `&` copies rather than by move: the produced bodies must
    // BORROW these stack slots (that is the 'scope borrow under test), while
    // still owning their `i`.
    let first_done_ref = &first_done;
    let ran_ref = &ran;

    pool.install(|scope| {
        scope.spawn_batch(
            N,
            (0..N).map(|i| {
                if i == 1 {
                    // Still inside `spawn_batch`: the batched arm's
                    // `&ScopeShared` and its `WaveRegistration` are both live
                    // on this thread's stack right now.
                    spin_until(
                        first_done_ref,
                        "body 0 to complete while the wave is still being pushed",
                    );
                    observed_completion_mid_wave.store(true, Ordering::Release);
                }
                move || {
                    ran_ref.fetch_add(1, Ordering::AcqRel);
                    if i == 0 {
                        first_done_ref.store(true, Ordering::Release);
                    }
                }
            }),
        );
    });

    assert_eq!(
        ran.load(Ordering::Acquire),
        N,
        "every body of the wave ran exactly once"
    );
    assert!(
        observed_completion_mid_wave.load(Ordering::Acquire),
        "no completion landed while the wave was still being pushed: the interleaving this \
         model exists to place under Tree Borrows was never reached — vacuous pass"
    );
}

/// The `k < n` correction under Tree Borrows: `WaveRegistration::drop` gives
/// back the unclaimed registrations through the same long-lived reference,
/// concurrently with workers still completing the ones that were claimed.
///
/// The failure mode if the give-back were wrong is a HANG rather than a UB
/// report, and `spin_until`'s bound does not cover `Scope::drop`'s join — so
/// what this adds over the native `spawn_batch_with_fewer_bodies_than_promised_
/// drains` is the BORROW question, not the accounting one: is the guard's
/// `unregister_tasks` write to `pending` legal while workers are writing the
/// same line.
#[test]
fn miri_spawn_batch_short_wave_gives_back_its_registrations_under_tree_borrows() {
    const PROMISED: usize = 8;
    const YIELDED: usize = 3;

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let ran = AtomicUsize::new(0);

    let ran_ref = &ran;

    pool.install(|scope| {
        scope.spawn_batch(
            PROMISED,
            (0..YIELDED).map(move |_| {
                move || {
                    ran_ref.fetch_add(1, Ordering::AcqRel);
                }
            }),
        );
    });

    assert_eq!(
        ran.load(Ordering::Acquire),
        YIELDED,
        "exactly the yielded bodies ran, and the join returned rather than waiting for the \
         `PROMISED - YIELDED` registrations no body ever claimed"
    );
}
