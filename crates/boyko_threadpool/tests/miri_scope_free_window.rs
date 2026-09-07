//! KE16 evidence probe - does the SHIPPED DEFAULT build (a0, no features) exhibit
//! the Tree-Borrows protector-vs-deallocation window in
//! `ScopeShared::complete_task`'s EXTERNAL-joiner arm?
//!
//! # The window under test
//!
//! `ScopeShared::complete_task(&self)` (`src/scope.rs:312-347`) ends its
//! external-joiner arm with
//!
//! ```text
//! self.waker.unpark();                          // scope.rs:345
//! self.pending.fetch_sub(1, Ordering::AcqRel);  // scope.rs:346
//! }                                             // scope.rs:347 - the &self PROTECTOR ends here
//! ```
//!
//! `&self` is a reference-typed function argument, so Tree Borrows installs a
//! *protector* on it that is live for the whole call, i.e. up to the closing
//! brace at `:347`. The joiner (`join_workers_until_drained`, polled from
//! `Scope::drop`) may observe `pending == 0` the instant the RMW at `:346`
//! commits - strictly *before* `:347` - and then free the allocation at
//! `src/scope.rs:855` (`drop(Box::from_raw(raw))`). Deallocating memory covered
//! by a live strong protector is UB under Tree Borrows.
//!
//! Both sites are byte-identical between the default build and the KE16 `a1`
//! candidates: the only `cfg` inside `complete_task` is `ke16-w-count` (off in
//! all three), and `impl Drop for Scope` carries no `cfg` at all. So the WINDOW
//! provably exists in shipped code. What this file exists to settle is whether
//! Miri can be made to EXHIBIT it in the default build - evidence that stops
//! being obtainable the moment a fix lands.
//!
//! # Shape, and why each part of it is load-bearing
//!
//! The window is PER SCOPE, not per task: only the decrement that drives
//! `pending` to zero can race the free. Task count therefore buys Miri cost and
//! no statistical power at all. The shape is consequently MANY SCOPES x FEW
//! TASKS:
//!
//! * **`pool.install(...)` from the test's own thread** - `install` rewrites
//!   `CURRENT_WORKER_ID` to `WORKER_ID_DISPATCHER`, so `ScopeShared::new` is
//!   handed a null W-d-prime target and every completion takes the
//!   EXTERNAL-joiner arm (`scope.rs:343-346`), which is the arm that carries
//!   the window.
//! * **`TASKS_PER_SCOPE` tasks per scope** - enough that the last completion
//!   can land on a worker rather than on the joiner's own inline steal. A scope
//!   whose last body ran on the joiner has NO window: the same thread does the
//!   decrement and the free, so nothing races.
//! * **`SCOPES` scopes** - the sample size. Each scope offers at most one
//!   window.
//!
//! # Why the pool is REBUILT every `SCOPES_PER_POOL` scopes
//!
//! Because Miri's cost is SUPERLINEAR in the number of scopes driven through
//! ONE pool. The pool's long-lived allocations accumulate Tree-Borrows tree
//! nodes for every transient reborrow the tasks and the joiner make, so each
//! later scope on the same pool costs more than the one before it.
//!
//! MEASURED 2026-09-04/05 on this box, `-Zmiri-tree-borrows` plus
//! `-Zmiri-preemption-rate=0.5`, one seed, one pool:
//!
//! | scopes on one pool | wall                            | s per scope-window |
//! |--------------------|---------------------------------|--------------------|
//! | 8                  | 16 s                            | 2.0                |
//! | 32                 | 322 s                           | 10.1               |
//! | 256                | did not finish in 4700 s of CPU | > 18               |
//!
//! That is an exponent near 2.1, and it is the difference between a 64-seed
//! sweep costing under an hour and costing more than a day. Rebuilding the pool
//! every `SCOPES_PER_POOL` scopes resets the accumulation and makes the cost
//! linear in the sample size, while leaving every scope exactly what it was: an
//! `install` joined by the test's own thread, i.e. the external-joiner arm.
//!
//! # Anti-vacuity
//!
//! Two censuses, both printed so a sweep's POWER can be computed from the run
//! rather than assumed:
//!
//! 1. `ran` - every body of every scope executed. Asserted unconditionally. A
//!    run that spawned nothing, or a scope whose join returned early, cannot
//!    pass.
//! 2. `windows` - the number of scopes whose LAST-FINISHING BODY ran on a
//!    genuine worker thread (not inline on the dispatcher/joiner). This is the
//!    estimator for "scopes that actually offered the window", and it is the
//!    number a power statement is computed from. It is an estimator and not the
//!    truth - body-finish order is not exactly completion-decrement order, and
//!    the two can be preempted apart - but it is the closest observable this
//!    crate exposes without touching `src/`.
//!
//! `windows > 0` is asserted UNDER MIRI ONLY, and the asymmetry is measured,
//! not defensive. Under Miri the rate is total: every seed of every sweep run
//! for this file reported `windows == scopes`, because Miri's preemptive
//! scheduler always gets a worker onto the queue before the joiner has drained
//! it. NATIVELY the same shape is decided by thread-start latency against 16
//! trivial bodies, and it swings with machine load: an idle box gave 252-253
//! windows out of 256 on one warm pool, and the SAME binary on a box busy with
//! 13 Miri threads gave 0 out of 256. A native `windows > 0` gate would
//! therefore be a load sensor (red on a busy CI machine, green on an idle one)
//! while saying nothing about the property under test. The gate is applied
//! where the measurement is meaningful, and the census line is printed
//! everywhere, so a native run still reports what it drove.
//!
//! # Run
//!
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance
//!   -Zmiri-ignore-leaks -Zmiri-many-seeds=0..12 -Zmiri-preemption-rate=0.5"
//!   cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-threadpool
//!   --test miri_scope_free_window -- --nocapture
//! ```
//!
//! A run whose `MIRIFLAGS` lack `-Zmiri-tree-borrows` proves nothing here: the
//! protector this file hunts is a Tree-Borrows object and Stacked Borrows does
//! not install it on this shape. `-Zmiri-many-seeds` runs its seeds in
//! PARALLEL, one interpreter thread each, so the width of the range is the
//! machine occupancy - bound it rather than fanning out to every core.
//!
//! # No env knobs - a deliberate refusal
//!
//! The volume is `const`s and nothing reads the environment. An earlier draft
//! made them env-overridable so a sweep could be calibrated without
//! recompiling, and that draft silently ran the WRONG VOLUME: `cargo-miri`
//! captures the environment at BUILD time into its per-binary run info and
//! replays it at RUN time, so a calibration value exported once (`SCOPES=8`)
//! was still in force for a later 64-seed sweep whose own shell had it unset
//! and which reported `EXIT_CODE=0`. The census line is what caught it - a
//! sweep that prints its volume cannot quietly claim one it did not run - but
//! the knob that made the mistake possible is gone, and the census line stays.
//!
//! This test is deliberately NOT gated on any KE16 feature and NOT
//! `cfg_attr(miri, ignore)`d. The default build is the whole point.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use boyko_threadpool::{ThreadPoolBuilder, WORKER_ID_DISPATCHER, current_worker_id};

/// Pools built per run. See the module header for why the pool is recycled.
const POOLS: usize = 32;

/// Scopes driven on each pool before it is torn down and rebuilt.
const SCOPES_PER_POOL: usize = 8;

/// Total scopes, and therefore the number of windows the run offers. This is
/// the sample size and the only knob that buys statistical power.
const SCOPES: usize = POOLS * SCOPES_PER_POOL;

/// Tasks per scope. Not a power knob - its only job is to make it likely that
/// the last completion lands on a worker instead of on the joiner's own inline
/// steal. Bigger values buy Miri cost, not windows.
const TASKS_PER_SCOPE: usize = 16;

/// Worker threads per pool. Three plus the joiner: enough contention for the
/// last completion to land off-joiner, few enough that Miri's per-thread
/// scheduling cost stays bounded.
const WORKERS: usize = 3;

/// True iff `id` denotes a genuine worker thread - not the dispatcher sentinel
/// an `install` frame carries, and not the unattached sentinel.
fn ran_on_worker(id: u32) -> bool {
    id != WORKER_ID_DISPATCHER && id != u32::MAX
}

/// Drives `SCOPES` dispatcher-joined scopes of `TASKS_PER_SCOPE` trivial bodies
/// each, so that `SCOPES` independent `pending -> 0` decrements race `SCOPES`
/// independent `Scope::drop` frees.
///
/// Under `-Zmiri-tree-borrows` a hit is reported as
/// `error: Undefined Behavior: deallocation through <TAG> ... is forbidden`,
/// with the freeing thread inside `Scope::drop` and the protector inside
/// `ScopeShared::complete_task`.
#[test]
fn a0_external_joiner_free_races_completion_protector() {
    // Total bodies executed across every scope - the primary anti-vacuity
    // census.
    let ran = AtomicUsize::new(0);
    // Scopes whose last-finishing body ran on a genuine worker: the window
    // estimator, and the number a power statement is computed from.
    let mut windows = 0usize;

    for p in 0..POOLS {
        let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

        for k in 0..SCOPES_PER_POOL {
            // Per-scope, so each scope's census is independent and a scope that
            // failed to drain is caught at its own iteration.
            let done = AtomicUsize::new(0);
            let last_wid = AtomicU32::new(u32::MAX);

            pool.install(|scope| {
                let ran = &ran;
                let done = &done;
                let last_wid = &last_wid;
                for _ in 0..TASKS_PER_SCOPE {
                    scope.spawn(move || {
                        ran.fetch_add(1, Ordering::Relaxed);
                        // The body that observes `TASKS_PER_SCOPE - 1`
                        // predecessors is the last one to finish; its thread is
                        // (almost always) the one whose `complete_task` drives
                        // `pending` to zero.
                        if done.fetch_add(1, Ordering::AcqRel) == TASKS_PER_SCOPE - 1 {
                            last_wid.store(current_worker_id(), Ordering::Release);
                        }
                    });
                }
            });

            // The join returned, so every body of THIS scope has completed.
            assert_eq!(
                done.load(Ordering::Acquire),
                TASKS_PER_SCOPE,
                "pool {p} scope {k}: the join returned with bodies still unfinished"
            );
            if ran_on_worker(last_wid.load(Ordering::Acquire)) {
                windows += 1;
            }
        }
    }

    assert_eq!(
        ran.load(Ordering::Acquire),
        SCOPES * TASKS_PER_SCOPE,
        "every spawned body must have run: a short count means the sweep measured less than it \
         claims"
    );

    // Printed (needs `-- --nocapture`) so a sweep's power is read off the run
    // instead of assumed, and so a stale artifact cannot let a sweep report a
    // volume it did not run.
    //
    // ONE `write_all` of a pre-formatted line, not `eprintln!`: under
    // `-Zmiri-many-seeds` every seed is a separate interpreted process writing
    // to the SAME pipe, `std::io::Stderr` is unbuffered, and a multi-fragment
    // `write_fmt` therefore interleaves MID-LINE across seeds. Measured
    // 2026-09-04: a 64-seed sweep left only 29 of its 64 census lines intact
    // under `eprintln!`, i.e. more than half the power evidence was unreadable.
    let census = format!(
        "KE16-FREE-WINDOW-CENSUS variant={} pools={POOLS} scopes={SCOPES} \
         tasks_per_scope={TASKS_PER_SCOPE} workers={WORKERS} bodies={} windows={windows}\n",
        boyko_threadpool::ke16_variant(),
        ran.load(Ordering::Acquire),
    );
    {
        use std::io::Write as _;
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(census.as_bytes());
        let _ = err.flush();
    }

    // Miri only - see the module header's "Anti-vacuity" section: natively this
    // number is decided by thread-start latency against 16 trivial bodies and
    // swings with machine load, so a native gate would measure the box.
    #[cfg(miri)]
    assert!(
        windows > 0,
        "vacuous run: not one of the {SCOPES} scopes ended with its last body on a worker thread, \
         so no scope offered the completion-vs-free window this test exists to hunt"
    );
}
