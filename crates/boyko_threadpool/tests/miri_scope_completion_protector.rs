//! KE16 — the completer-side protector gate: at the instant `Scope::drop` frees
//! `ScopeShared`, no thread may hold a live Tree-Borrows protector over that
//! allocation.
//!
//! # The property, stated against the rule that judges it
//!
//! Tree Borrows installs a *protector* on every reference-typed function
//! ARGUMENT, and that protector is live for the whole call — to the closing
//! brace — whether or not the callee touches a byte. Deallocating a range a
//! live STRONG protector covers is UB, independently of happens-before. So the
//! obligation at the free site is NOT "every worker's last byte-access is
//! ordered before the join's `Acquire` load" (true, and about data races) but:
//!
//! > at the instant of the free, no thread holds a live activation of any
//! > function whose argument is a reference into this allocation.
//!
//! `ScopeShared::complete_task` is where that bites, because its own decrement
//! is what authorises the free: the joiner may observe `pending == 0` the
//! instant the RMW commits, strictly before the completer's frame ends. The fix
//! is the shape Phase 9.2 already used on the joiner side —
//! `join_workers_until_drained(inner, shared: *const ScopeShared)` takes the
//! address BY VALUE so the dispatcher holds no protected `&ScopeShared` across
//! the workers' writes — mirrored onto the completer:
//! `ScopeShared::complete_task(shared: *const Self)`.
//!
//! MEASURED MECHANISM, which is why this is UB and not a near miss: a payload
//! that is ENTIRELY interior-mutable gives `&self` only a WEAK protector, and
//! weak protectors do not forbid deallocation. `ScopeShared` carries FROZEN
//! bytes — `waker: Thread`, `joiner_wake: *const WakeHandle`, and
//! `CachePadded`'s padding — so the free is a foreign write to a strongly
//! protected frozen range ("protected tags must never be Disabled"). The
//! reported offset is inside that frozen range, not inside the atomic.
//!
//! # Why this file exists next to `miri_scope_free_window.rs`
//!
//! That file is the EVIDENCE PROBE: many scopes x few tasks, hunting the window
//! with Miri's random preemption, priced in hours. This file is the GATE: it
//! makes the same window deterministic and cheap enough to run on every change.
//! The two are not redundant — the probe measures how often the shipped
//! schedule OFFERS the window, this one decides whether the code is sound when
//! it is offered.
//!
//! # What makes the window OPEN, structurally rather than by luck
//!
//! The window is PER SCOPE: only the decrement that drives `pending` to zero can
//! race the free, and a scope whose last body ran inline on the joiner has no
//! window at all — the same thread decrements and frees, so nothing races. The
//! first draft of this file spawned bodies and hoped the joiner's batch steal
//! would leave the last one on a worker; MEASURED, that hope is schedule-noise.
//! Across a probe-width sweep the census swung between `windows == 2` and
//! `windows == 0` on the SAME code, because every change to the interleaving
//! also changes who claims which task.
//!
//! The shape below removes the luck. Each scope spawns exactly `TASKS_PER_SCOPE
//! == WORKERS` bodies; each body announces itself in `started` and then waits on
//! `go`; the scope's own closure waits, bounded, until every body has announced,
//! and only then sets `go` and returns. When `Scope::drop` finally polls, every
//! task is already CLAIMED and running on a genuine worker, so the joiner has
//! nothing to steal and cannot become a completer itself. The last decrement is
//! therefore a worker's by construction, and `windows == SCOPES` is asserted,
//! not hoped for.
//!
//! # What makes the verdict DETERMINISTIC — and the lines in `src/` that do it
//!
//! `ScopeShared::complete_task` carries a `#[cfg(miri)]` yield burst immediately
//! after each arm's release RMW, taken only by the completer whose `prev == 1`
//! (the one whose decrement authorised the free). It is the same device as the
//! yields in `join_workers_until_drained` and `worker::worker_main` (Phase 9.1
//! H2) and compiles to nothing natively.
//!
//! It is load-bearing, and MEASURED to be. Without it the window is a couple of
//! MIR steps wide against a joiner tail (loop exit, `scratch` drop,
//! `take_panic`, `Box::from_raw`) of order a hundred, so whether Miri ever
//! schedules into it is a coin flip on the seed — which is exactly how this
//! defect survived a 10 975-window probe. Run against a deliberately
//! reference-typed receiver at `-Zmiri-preemption-rate=0`, this very test is
//! GREEN with no probe and green with one `yield_now()`, and red from two
//! upwards: at rate 0 the yielded thread is re-enabled at once and the
//! round-robin can return to it before the joiner has run its tail. The burst
//! holds the completer's frame open across enough scheduling rounds for the
//! joiner to finish, which is precisely the interleaving a real OS produces
//! whenever it deschedules a thread right after a `lock xadd`.
//!
//! With the burst plus `-Zmiri-preemption-rate=0` the schedule is forced and the
//! verdict is a property of the RECEIVER, not of the seed. MEASURED 2026-09-05,
//! seeds 0 / 1 / 7 / 15:
//!
//! | `complete_task` receiver | probe | rate 0        | rate 0.01 (default) |
//! |--------------------------|-------|---------------|---------------------|
//! | `&self` (pre-fix)        | none  | 4/4 green     | —                   |
//! | `&self` (pre-fix)        | 16    | 4/4 UB        | 1/4 UB (seed 7)     |
//! | `*const Self` (shipped)  | 16    | 4/4 pass      | 4/4 pass            |
//!
//! Read the second row before dropping the flag: at the DEFAULT preemption rate
//! the defect is caught on a MINORITY of seeds, so `-Zmiri-preemption-rate=0` is
//! part of this gate's recipe and not a tuning preference. The first row is the
//! reason the probe exists at all — the shipped window is invisible to Miri
//! without it, on every seed tried.
//!
//! # Anti-vacuity
//!
//! Two censuses, both printed so a run's power is read off the run rather than
//! assumed: `bodies` (every body of every scope executed) and `windows` (scopes
//! whose last-finishing body ran on a genuine worker). `windows` is asserted
//! EQUAL to `SCOPES` under Miri — the shape above guarantees it, so anything
//! less means the schedule stopped matching the design and the gate has lost the
//! window it exists to decide. It is an estimator, not the truth (body-finish
//! order is not exactly completion-decrement order), and it is the same one
//! `miri_scope_free_window.rs` uses; see that file for why a NATIVE assert on it
//! would be a machine-load sensor rather than a gate.
//!
//! # Run
//!
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance
//!   -Zmiri-ignore-leaks -Zmiri-preemption-rate=0"
//!   cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-threadpool
//!   --test miri_scope_completion_protector -- --nocapture
//! ```
//!
//! `-Zmiri-tree-borrows` is not optional: the protector this gate hunts is a
//! Tree-Borrows object and Stacked Borrows does not install it on this shape.
//! `-Zmiri-preemption-rate=0` is not optional either — see the table above.
//! `-Zmiri-many-seeds=0..4` demonstrates the seed-independence rather than
//! assuming it.
//!
//! # The two arms, and the DEBT that belongs to the second one
//!
//! `complete_task` has two arms and both carry the protector, so both are gated
//! here, by one test each:
//!
//! * `completer_holds_no_protector_when_the_joiner_frees` — the EXTERNAL-joiner
//!   arm, which the default build ships and which an `install` from a non-worker
//!   thread reaches.
//! * `worker_joiner_completer_holds_no_protector_when_the_joiner_frees` — the
//!   `ke16-w-count` W-d′ arm, selected by a non-null `joiner_wake`, which
//!   requires the scope to have been opened BY A WORKER of its own pool.
//!
//! THE SECOND IS A DEBT OF THE ARM, NOT A LIMITATION OF THIS FILE. `ke16-w-count`
//! either gets deleted when the tournament closes, or it becomes the completion
//! path of every task in the engine — and the pass that makes it the default
//! must not ship an undecided one. Before this test existed, NO test in ANY
//! configuration decided the protector property for that arm: reaching it is not
//! enough, because Miri aborts on UB regardless of what a test asserts, and
//! MEASURED, `miri_scope.rs::nested_scope_from_worker_is_stolen_by_sibling` under
//! `--features ke16-w-count,ke16-a2` is GREEN with the defect deliberately
//! reintroduced. It reaches the arm; it does not force the interleaving.
//!
//! The W-d′ test needs an A arm as well as `ke16-w-count`, and that is a
//! property of the pool rather than of the test: without one, a worker-spawned
//! wave never leaves the lane it was spawned on, so the nested scope's last
//! completer is the joining worker itself and there is no window at all. It is
//! `#[ignore]`d with that reason in a `ke16-w-count`-only build rather than
//! passing vacuously.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

#[cfg(feature = "ke16-w-count")]
use boyko_threadpool::try_with_active_pool;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder, WORKER_ID_DISPATCHER, current_worker_id};

/// Worker threads. Two is enough: the window is per scope and needs exactly one
/// off-joiner completer, and every extra thread multiplies Miri's interleaving
/// cost.
const WORKERS: usize = 2;

/// Bodies per scope. MUST equal `WORKERS`: the shape waits for every body to be
/// claimed before releasing them, so a body with no free worker to claim it
/// would never announce itself and the bounded spin would fail the run.
const TASKS_PER_SCOPE: usize = WORKERS;

/// Scopes driven. Under a forced schedule ONE window decides the property; four
/// is a margin against a single anomalous scope, and small enough that Miri's
/// superlinear per-pool cost (`miri_scope_free_window.rs` measured an exponent
/// near 2.1) never starts.
const SCOPES: usize = 4;

/// Iterations before a cooperative spin gives up. Same convention and same value
/// as `miri_scope.rs::spin_until`: each turn yields, so a healthy run needs a
/// handful, and an unhealthy one FAILS instead of hanging Miri forever.
const SPIN_CAP: usize = 100_000;

/// Turns spent waiting for a release-probe firing to become observable. Small,
/// because the counter is bumped before the probe yields; see
/// `assert_probe_armed` for why the observation is a bounded spin at all.
#[cfg(miri)]
const PROBE_OBSERVE_CAP: usize = 1_000;

/// Bounded cooperative spin on an arbitrary condition.
fn spin_until(mut cond: impl FnMut() -> bool, ctx: &str) {
    for _ in 0..SPIN_CAP {
        if cond() {
            return;
        }
        std::thread::yield_now();
    }
    panic!("spin_until timed out waiting for {ctx} — no cross-thread progress");
}

/// True iff `id` denotes a genuine worker thread — not the dispatcher sentinel
/// an `install` frame carries, and not the unattached sentinel.
fn ran_on_worker(id: u32) -> bool {
    id != WORKER_ID_DISPATCHER && id != u32::MAX
}

/// Reads the release probe's firing counter, or 0 natively where there is none.
fn probe_firings() -> usize {
    #[cfg(miri)]
    {
        boyko_threadpool::miri_release_probe_firings()
    }
    #[cfg(not(miri))]
    {
        0
    }
}

/// Reads the count of frees that landed inside a release window, or 0 natively.
fn frees_inside_window() -> usize {
    #[cfg(miri)]
    {
        boyko_threadpool::miri_frees_inside_a_release_window()
    }
    #[cfg(not(miri))]
    {
        0
    }
}

/// Asserts that this run actually produced the interleaving it claims to decide.
///
/// # Why a gate needs this at all
///
/// `bodies` and `windows` are properties of the SCHEDULE. The thing that decides
/// this gate is the OVERLAP between a completer's open post-decrement frame and
/// the joiner's free, which happens where no body observes anything — so every
/// census the test prints stays green over a defective receiver whenever the
/// probe stops working. MEASURED by the KE16 code review with a probe of zero
/// yields: `running 1 test`, `1 passed`, full census, `windows == SCOPES`, and
/// the gate deciding nothing whatever.
///
/// # Two observations, because they fail differently
///
/// * `firings` — the probe was CALLED, once per `pending -> 0`. Catches a probe
///   deleted, `cfg`'d away, or moved off the `prev == 1` path.
/// * `frees_inside_window` — a free actually landed while a completer of that
///   same allocation was inside its window. This is the property; the other is a
///   proxy. MEASURED by the KE16 tester, the mutation that defeats the proxy
///   alone: keep the `fetch_add`, delete only the yield loop, and both the
///   firing counter and a compile-time floor on the yield count stay green while
///   the gate is disarmed and the defect ships.
///
/// # Why not a floor on the yield count
///
/// Tried, and refuted twice. The threshold was measured at 4 rather than the 2
/// that was asserted; and armed-ness is NOT MONOTONIC in the count — two
/// reconstructions differing by one MIR statement put a disarmed point at 8
/// between armed points at 7 and 10, because the mechanism is round-robin
/// ALIGNMENT, not duration. No constant certifies it. This does, by observing
/// the result rather than the input.
///
/// # Why both are `>=` and both SPIN
///
/// `>=`: the counters are process-global and libtest runs this file's tests in
/// parallel, so a concurrent test can only ADD. Under-counting — the failure
/// mode that matters, since a broken probe fires for nobody — is still caught.
/// SPIN: the counters are bumped by the COMPLETER after its release RMW, and
/// nothing orders that store before this thread's read (the joiner learns of
/// completion through the decrement, and the bump follows it). MEASURED while
/// building this gate: a single sample read 1 where 2 probes had fired, i.e. a
/// FALSE RED. If the probe is gone, no amount of spinning invents a firing.
///
/// # The two thresholds differ, and the difference is the point
///
/// `firings` is required `>= expected` — one per `pending -> 0`, exact, and
/// MEASURED stable by the KE16 tester across 24 process runs at two parallelism
/// settings and 64 seeds with zero variation.
///
/// `overlaps` is required only `>= 1`, and that is the PROPERTY's own threshold
/// rather than a concession. Miri aborts the interpreter on the first UB, so ONE
/// free landing inside ONE release window is exactly what makes this gate able
/// to report a reference-typed receiver; requiring every scope to overlap would
/// be stricter than the thing being certified and would turn ordinary scheduling
/// variation into red. MEASURED on the shipped tree: 3 of 4 scopes overlap in a
/// default run — the fourth's joiner reaches its free after the burst has
/// elapsed, which is the same round-robin ALIGNMENT effect that makes armed-ness
/// non-monotonic in the yield count. The count is printed rather than asserted
/// so that a drift from 3/4 toward 1/4 is visible before it reaches 0/4 and
/// fails.
fn assert_probe_armed(firings_before: usize, frees_before: usize, expected: usize, ctx: &str) {
    #[cfg(miri)]
    {
        for _ in 0..PROBE_OBSERVE_CAP {
            if probe_firings().saturating_sub(firings_before) >= expected
                && frees_inside_window().saturating_sub(frees_before) >= 1
            {
                break;
            }
            std::thread::yield_now();
        }

        let fired = probe_firings().saturating_sub(firings_before);
        let overlapped = frees_inside_window().saturating_sub(frees_before);

        // One pre-formatted line, for the same reason as the censuses above:
        // under `-Zmiri-many-seeds` every seed writes to the same unbuffered
        // stderr and a multi-fragment write interleaves mid-line.
        {
            use std::io::Write as _;
            let line = format!(
                "KE16-PROTECTOR-GATE-ARMED ctx={ctx} firings={fired}/{expected} \
                 overlaps={overlapped}/{expected}\n"
            );
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(line.as_bytes());
            let _ = err.flush();
        }

        assert!(
            fired >= expected,
            "{ctx}: the release probe fired {fired} times, expected at least {expected}. The \
             gate is DISARMED - the probe was deleted, cfg'd away, or moved off the `prev == 1` \
             path. Every census above can stay green while this test decides nothing."
        );

        assert!(
            overlapped >= 1,
            "{ctx}: the probe fired {fired} times but NOT ONE of {expected} frees landed inside \
             a release window. The gate is DISARMED in the way no counter of calls and no floor \
             on MIRI_RELEASE_PROBE_YIELDS can see: the burst no longer overlaps the free, so a \
             reference-typed receiver would not be caught here. Armed-ness is not monotonic in \
             the yield count - re-tune MIRI_RELEASE_PROBE_YIELDS against THIS observation rather \
             than reasoning about its size."
        );
    }
    #[cfg(not(miri))]
    {
        let _ = (firings_before, frees_before, expected, ctx);
    }
}

/// Drives `SCOPES` dispatcher-joined scopes in which the last completion is a
/// worker's by construction, so each one's `pending -> 0` races that scope's
/// `Box::from_raw`.
///
/// With a reference-typed `complete_task` receiver this reports
/// `error: Undefined Behavior: deallocation through <TAG> ... is forbidden`,
/// the freeing thread inside `Scope::drop` and the protector inside
/// `ScopeShared::complete_task`. With the by-value receiver it passes.
#[test]
fn completer_holds_no_protector_when_the_joiner_frees() {
    let probes_before = probe_firings();
    let frees_before = frees_inside_window();
    // Every body of every scope executed — the primary anti-vacuity census.
    let ran = AtomicUsize::new(0);
    // Scopes whose last-finishing body ran on a genuine worker.
    let mut windows = 0usize;

    // One pool for the whole run: at this volume the superlinear per-scope cost
    // has not begun to matter, and rebuilding would add worker start-up to every
    // scope's schedule.
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

    for k in 0..SCOPES {
        let started = AtomicUsize::new(0);
        let go = AtomicBool::new(false);
        let done = AtomicUsize::new(0);
        let last_wid = AtomicU32::new(u32::MAX);

        // `install` from the test's own thread rewrites `CURRENT_WORKER_ID` to
        // `WORKER_ID_DISPATCHER`, so `ScopeShared::new` is handed a null W-d′
        // target and every completion takes the EXTERNAL-joiner arm — the arm
        // the default build ships, and the only one this shape can reach (see
        // the header's coverage note).
        pool.install(|scope| {
            let ran = &ran;
            let started = &started;
            let go = &go;
            let done = &done;
            let last_wid = &last_wid;
            for _ in 0..TASKS_PER_SCOPE {
                scope.spawn(move || {
                    started.fetch_add(1, Ordering::AcqRel);
                    spin_until(|| go.load(Ordering::Acquire), "the go gate");
                    ran.fetch_add(1, Ordering::Relaxed);
                    // The body that observes `TASKS_PER_SCOPE - 1` predecessors
                    // is the last to finish; its thread is the one whose
                    // `complete_task` drives `pending` to zero.
                    if done.fetch_add(1, Ordering::AcqRel) == TASKS_PER_SCOPE - 1 {
                        last_wid.store(current_worker_id(), Ordering::Release);
                    }
                });
            }

            // THE STRUCTURAL PART. Returning only once every body has announced
            // itself means every body is claimed by a genuine worker before the
            // join below begins, so the joiner has nothing to steal and cannot
            // become a completer. That is what makes the last decrement a
            // worker's in every scope instead of in a lucky fraction of them.
            spin_until(
                || started.load(Ordering::Acquire) == TASKS_PER_SCOPE,
                "every body to be claimed by a worker",
            );
            go.store(true, Ordering::Release);
        });

        assert_eq!(
            done.load(Ordering::Acquire),
            TASKS_PER_SCOPE,
            "scope {k}: the join returned with bodies still unfinished"
        );
        if ran_on_worker(last_wid.load(Ordering::Acquire)) {
            windows += 1;
        }
    }

    assert_eq!(
        ran.load(Ordering::Acquire),
        SCOPES * TASKS_PER_SCOPE,
        "every spawned body must have run: a short count means the gate measured less than it \
         claims"
    );

    // ONE `write_all` of a pre-formatted line, not `eprintln!`: under
    // `-Zmiri-many-seeds` every seed is a separate interpreted process writing to
    // the SAME unbuffered stderr, and a multi-fragment `write_fmt` interleaves
    // mid-line across seeds (measured 2026-09-04 on the sibling probe: 29 of 64
    // census lines survived).
    let census = format!(
        "KE16-PROTECTOR-GATE-CENSUS variant={} scopes={SCOPES} tasks_per_scope={TASKS_PER_SCOPE} \
         workers={WORKERS} bodies={} windows={windows}\n",
        boyko_threadpool::ke16_variant(),
        ran.load(Ordering::Acquire),
    );
    {
        use std::io::Write as _;
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(census.as_bytes());
        let _ = err.flush();
    }

    // Miri only — see the module header. A green with `windows < SCOPES` would be
    // a gate that had lost part of the window it exists to decide.
    #[cfg(miri)]
    assert_eq!(
        windows, SCOPES,
        "the shape guarantees every scope's last body runs on a worker; a shortfall means the \
         schedule stopped matching the design and this run decided less than it claims"
    );

    // ...and no census above can see whether the probe that OPENS the window ran
    // at all. One firing per scope's `pending -> 0`.
    assert_probe_armed(probes_before, frees_before, SCOPES, "external-joiner arm");
}

/// Process-level state for the W-d′ test, and the reason it is `static` rather
/// than borrowed from the outer body.
///
/// The obvious shape — `Arc`s moved into the fire-and-forget outer closure, with
/// the nested bodies borrowing them — tripped a SECOND, SEPARATE instance of
/// exactly the class this file exists to gate, one level further out, and would
/// have made this gate report that one instead of the one it names. MEASURED
/// 2026-09-05 against the THEN-shipped `Box<dyn FnOnce>` task element, with the
/// raw-pointer `complete_task` receiver, under `ke16-w-count,ke16-a2`:
///
/// ```text
/// error: Undefined Behavior: deallocation through <425590> (root of the allocation)
///        at alloc134213[0x10] is forbidden
///   the accessed tag <425590> was created here
///     --> tests/miri_scope_completion_protector.rs:570:20   (pool.spawn(move || {  -- the outer box)
///   the protected tag <499383> was created here, in the initial state Frozen
///     --> src/scope.rs:1075:22                 (let wrapped = move || -- the task wrapper,
///                                               a line that NO LONGER EXISTS; see below)
///   note: this is on thread `boyko-worker-2`
/// ```
///
/// Read it as the same rule judging a different allocation: the pool's task-body
/// wrapper `wrapped` held PROTECTED references into whatever the body borrowed,
/// and that protector lived until the wrapper's frame RETURNED — which is after
/// its own `complete_task` decrement has told the joiner the body is done. A
/// sibling was still inside `wrapped` when the outer body's `call_once` freed the
/// closure box the sibling had borrowed from. It was not caused by the release
/// probe; the probe only widens the same few-instruction window the probe exists
/// to widen, which is precisely how it became visible.
///
/// # STATUS: CLOSED BY CONSTRUCTION, and by the change this file gates
///
/// Both halves of that report are gone, and neither by care:
///
/// * `wrapped` does not exist. The task element is now
///   `Task { payload, execute: unsafe fn(*const ()) }` (`src/task.rs`), so no
///   activation between the body and the release takes a reference-shaped
///   argument at all — which is the property `body_environment_protector_
///   expires_before_the_borrowed_frame_pops` decides, below.
/// * The freed allocation named above — the enclosing fire-and-forget task's
///   closure box — is gone too, and EARLIER than the report's window:
///   `task::run_detached` moves the body out of its cell and frees the cell
///   BEFORE invoking it, so at the inner scope's release that allocation no
///   longer exists.
///
/// This test's `static`s are therefore no longer load-bearing for soundness,
/// only for keeping the gate pointed at ONE thing: they remove every reference
/// from a nested body into memory the outer body owns, so this arm decides the
/// `ScopeShared` protector and nothing else. What is NOT carried here is an
/// EXECUTED arm in the original nested-in-detached shape; that is a coverage
/// item, tracked for the stage that owns nested-scope arms, not an open defect.
#[cfg(feature = "ke16-w-count")]
mod w_state {
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize};

    /// Nested bodies that have announced themselves in the current scope.
    pub static STARTED: AtomicUsize = AtomicUsize::new(0);
    /// Release gate for the current scope's nested bodies.
    pub static GO: AtomicBool = AtomicBool::new(false);
    /// Nested bodies finished in the current scope.
    pub static DONE: AtomicUsize = AtomicUsize::new(0);
    /// Worker that ran the current scope's LAST-finishing nested body.
    pub static LAST_WID: AtomicU32 = AtomicU32::new(u32::MAX);
    /// Nested bodies executed across the whole run.
    pub static RAN: AtomicUsize = AtomicUsize::new(0);
    /// The worker the outer body ran on.
    pub static OUTER_WID: AtomicU32 = AtomicU32::new(u32::MAX);
    /// Set once the outer body has driven every nested scope.
    pub static OUTER_DONE: AtomicBool = AtomicBool::new(false);
    /// Scopes whose last nested body ran on a SIBLING of the joining worker.
    pub static WINDOWS: AtomicUsize = AtomicUsize::new(0);
    /// Receipt that the nested scopes really were opened by a worker.
    pub static JOINER_WAS_A_WORKER: AtomicBool = AtomicBool::new(false);
}

/// KE16 W-d′ — the same property for the arm a WORKER joiner takes.
///
/// # Why this test has to exist separately
///
/// `complete_task`'s `ke16-w-count` arm is selected by a non-null `joiner_wake`,
/// which `ScopeShared::new` writes only when `tls::worker_lane_for(inner)`
/// answers `Some` — i.e. only when the scope was opened BY A WORKER of its own
/// pool. No `install` from the test thread can reach it, so the sibling test
/// above certifies nothing about it however it is built.
///
/// And REACHING the arm is not enough either, which is the trap this test was
/// written to close. Miri aborts the interpreter on UB regardless of what a test
/// asserts, so it is tempting to treat any test that runs the arm as a gate for
/// it. MEASURED by the KE16 code review: with the defect deliberately
/// reintroduced, `miri_scope.rs::nested_scope_from_worker_is_stolen_by_sibling`
/// under `--features ke16-w-count,ke16-a2` is GREEN. It reaches the arm; it does
/// not force the interleaving, so it decides nothing about the protector.
///
/// # Shape
///
/// The same announce/`go` handshake as the external test, opened one level in:
///
/// * the test thread `pool.spawn`s ONE outer body, which lands on a worker;
/// * that worker opens a NESTED scope through `try_with_active_pool` +
///   `PoolInner::scope`, so it is a WORKER joiner and `joiner_wake` is non-null;
/// * the nested scope spawns `NESTED_BODIES` bodies that announce and wait;
/// * the outer body releases them only once every one has announced, so all of
///   them are claimed by SIBLING workers and the joining worker has nothing to
///   steal and cannot become the last completer itself.
///
/// Hence `WORKERS_W == 1 + NESTED_BODIES`: one lane for the joiner and one per
/// sibling. Fewer, and the joining worker runs a nested body inline and closes
/// the window.
///
/// # Why an A arm is required, and why that is the ARM's debt and not this
/// file's limitation
///
/// Without an A arm a worker-spawned wave never leaves the lane it was spawned
/// on: the nested bodies stay in the joining worker's own queue, it runs them
/// itself, and its own `complete_task` is the last completer — the same thread
/// decrements and frees, so there is no window to decide. The test is therefore
/// `#[ignore]`d, with that reason, in a `ke16-w-count`-only build rather than
/// passing vacuously. `ke16-w-count` either gets deleted when the tournament
/// closes or becomes the completion path of every task in the engine, and the
/// pass that makes it the default must not ship an undecided one.
///
/// # Verdict, MEASURED 2026-09-05 under `--features ke16-w-count,ke16-a2` at
/// `-Zmiri-preemption-rate=0`
///
/// | `complete_task` receiver | burst placement   | verdict |
/// |--------------------------|-------------------|---------|
/// | `&self` (pre-fix)        | after the unpark  | UB      |
/// | `&self` (pre-fix)        | before the unpark | UB      |
/// | `*const Self` (shipped)  | after the unpark  | pass    |
///
/// The middle row REFUTES what `scope.rs` claimed before this test existed. That
/// comment asserted the gate would go green with the burst moved above the
/// unpark, on the reasoning that this arm's joiner may be parked and a burst
/// before the wake would hold the completer's frame open while the only thread
/// that could free the allocation was still blocked. It does not reproduce: the
/// nested joiner here is polling its own join, not parked, so the burst reaches
/// it either way. This gate therefore decides the RECEIVER, which is what it is
/// for, and does NOT decide the placement. The placement is kept after the
/// unpark because it dominates — it makes the probe independent of the joiner's
/// park state — and `scope.rs` now says so at that strength.
#[cfg(feature = "ke16-w-count")]
#[test]
#[cfg_attr(
    not(any(
        feature = "ke16-a1",
        feature = "ke16-a1-fifo",
        feature = "ke16-a2",
        feature = "ke16-a3",
        feature = "ke16-a5"
    )),
    ignore = "deferred: KE16 — the W-d′ window needs a worker-spawned wave to reach SIBLING \
              lanes, which defect A denies in a build with no A arm: the nested bodies stay in \
              the joining worker's own queue, it runs them itself, and the last completer is the \
              joiner, so there is no completion-vs-free window to decide. This is a DEBT OF THE \
              ARM, not of this file — `ke16-w-count` must not become the default until this test \
              runs and passes in the configuration that ships."
)]
fn worker_joiner_completer_holds_no_protector_when_the_joiner_frees() {
    use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};

    /// Bodies inside the nested scope, one per SIBLING lane.
    const NESTED_BODIES: usize = 2;
    /// One lane for the worker joiner plus one per sibling body.
    const WORKERS_W: usize = 1 + NESTED_BODIES;
    /// Nested scopes driven. Each offers one window.
    const SCOPES_W: usize = 2;

    let probes_before = probe_firings();
    let frees_before = frees_inside_window();
    let pool = ThreadPoolBuilder::new().num_threads(WORKERS_W).build();

    // The outer body captures NOTHING — see `w_state` for the measured reason.
    pool.spawn(|| {
        let me = current_worker_id();
        w_state::OUTER_WID.store(me, Release);

        for _ in 0..SCOPES_W {
            w_state::STARTED.store(0, Release);
            w_state::DONE.store(0, Release);
            w_state::LAST_WID.store(u32::MAX, Release);
            w_state::GO.store(false, Release);

            let dispatched = try_with_active_pool(|inner| {
                // Opened from a worker OF THIS POOL, so `ScopeShared::new` is
                // handed a non-null `joiner_wake` and every completion takes the
                // W-d′ arm. The receipt guards against a dispatcher-run outer
                // body, which would silently measure the external arm twice.
                w_state::JOINER_WAS_A_WORKER.store(ran_on_worker(me), Release);

                inner.scope(|nested| {
                    for _ in 0..NESTED_BODIES {
                        nested.spawn(|| {
                            w_state::STARTED.fetch_add(1, AcqRel);
                            spin_until(|| w_state::GO.load(Acquire), "the nested go gate");
                            w_state::RAN.fetch_add(1, Relaxed);
                            if w_state::DONE.fetch_add(1, AcqRel) == NESTED_BODIES - 1 {
                                w_state::LAST_WID.store(current_worker_id(), Release);
                            }
                        });
                    }

                    // Every nested body is now claimed by a SIBLING, so this
                    // joining worker has nothing to steal and the last decrement
                    // is a sibling's.
                    spin_until(
                        || w_state::STARTED.load(Acquire) == NESTED_BODIES,
                        "every nested body to be claimed by a sibling worker",
                    );
                    w_state::GO.store(true, Release);
                });
            });

            assert!(
                dispatched.is_some(),
                "the outer body must run inside an active pool frame, otherwise the nested scope \
                 has no worker lane and the W-d-prime arm is never selected"
            );
            assert_eq!(
                w_state::DONE.load(Acquire),
                NESTED_BODIES,
                "the nested join returned with bodies still unfinished"
            );

            // The window estimator for this arm: the last nested body must have
            // finished on a SIBLING, not on the joining worker.
            let last = w_state::LAST_WID.load(Acquire);
            if ran_on_worker(last) && last != me {
                w_state::WINDOWS.fetch_add(1, Relaxed);
            }
        }

        w_state::OUTER_DONE.store(true, Release);
    });

    spin_until(
        || w_state::OUTER_DONE.load(Acquire),
        "the worker's nested scopes to drain",
    );

    let outer = w_state::OUTER_WID.load(Acquire);
    let windows = w_state::WINDOWS.load(Relaxed);
    let census = format!(
        "KE16-PROTECTOR-GATE-CENSUS-W variant={} scopes={SCOPES_W} nested_bodies={NESTED_BODIES} \
         workers={WORKERS_W} outer_wid={outer} bodies={} windows={windows}\n",
        boyko_threadpool::ke16_variant(),
        w_state::RAN.load(Acquire),
    );
    {
        use std::io::Write as _;
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(census.as_bytes());
        let _ = err.flush();
    }

    assert!(
        ran_on_worker(outer) && w_state::JOINER_WAS_A_WORKER.load(Acquire),
        "the outer body must have run on a worker (got {outer}); a dispatcher-run outer body \
         would open the nested scope with a NULL joiner_wake and measure the external arm again"
    );
    assert_eq!(
        w_state::RAN.load(Acquire),
        SCOPES_W * NESTED_BODIES,
        "every nested body must have run"
    );

    // Miri only, for the same reason as the sibling test: natively this number is
    // decided by thread-start latency and swings with machine load.
    #[cfg(miri)]
    assert_eq!(
        windows, SCOPES_W,
        "the shape guarantees every nested scope's last body runs on a SIBLING of the joining \
         worker; a shortfall means the wave stayed in its own lane and no window was offered"
    );

    assert_probe_armed(probes_before, frees_before, SCOPES_W, "W-d-prime arm");
}

/// The BODY-ENVIRONMENT half of the same rule, judged by a FRAME POP instead of
/// by `Box::from_raw` — the shape the task representation exists to close.
///
/// # Why the two tests above do not cover it
///
/// Both of them decide the property for ONE memory: the `ScopeShared`
/// allocation, freed at `scope.rs`'s single `Box::from_raw`. Neither can see the
/// SECOND memory the same rule judges — whatever the body borrowed. Their bodies
/// capture `&started` / `&go` / `&done` / `&last_wid` out of the TEST FUNCTION's
/// own frame, and that frame does not pop until the run is over, so no
/// reclamation of the borrowed memory ever races a completer and there is
/// nothing for a protector over it to be UB against. The W-d′ test goes further
/// and moves that state into `static`s DELIBERATELY, because its natural shape
/// tripped exactly this defect one level out and would have reported it instead
/// of the one that test names (see `w_state`'s header, which records the
/// measurement verbatim).
///
/// # What this arm changes, and it is one thing
///
/// The four locations the bodies borrow are allocated in a helper frame that
/// RETURNS as soon as the join does. Popping that frame deallocates them, and
/// that deallocation is the event Tree Borrows judges: if any thread is still
/// inside an activation that received one of those references as an argument —
/// which, before the fix, is every completer, because the task wrapper's
/// `call_once` takes the whole environment BY VALUE and its own `complete_task`
/// is what authorises the joiner to run on — the pop is a deallocation through a
/// live protected tag.
///
/// The interleaving is the same one the sibling tests force, and it is forced by
/// the same device: the completer's `#[cfg(miri)]` yield burst holds its frame
/// open past the release, and at `-Zmiri-preemption-rate=0` the joiner then runs
/// its whole tail — free, `install` return, helper return, frame pop — in one
/// scheduling turn. So this arm is armed exactly when the sibling arm is, and
/// `assert_probe_armed` certifies it by the same two observations.
///
/// # Verdict, and what the two REDs behind it each prove
///
/// The failure is always
/// `error: Undefined Behavior: deallocation through <TAG> ... is forbidden`,
/// naming the protected tag created at whatever frame took the body by value.
///
/// * RED on the PRE-CHANGE tree, tag at `src/scope.rs:1075:22`
///   (`let wrapped = move ||`). This shows the arm decides the shipped defect.
///   It confounds two variables, though: it varies the defect AND the task
///   representation at once.
/// * RED on the POST-CHANGE tree with the defect REINTRODUCED inside the new
///   representation — `run_scoped`'s tail rewritten as
///   `let wrapped = move || { catch_unwind(..); complete_task(shared) };
///   wrapped();` — tag at `src/task.rs`, in a scratch copy, 2026-09-06 by the
///   reviewer. This is the discriminating one: representation held fixed, only
///   the defect varied. It is what certifies that the arm keys on the DEFECT
///   and not on the shape, and it is why a future refactor may not treat the
///   green below as a property of `Task` merely being 16 POD bytes.
/// * Armedness survives the obvious silencer: the same mutant run WITHOUT
///   `-Zmiri-preemption-rate=0` does not go quietly green — `assert_probe_armed`
///   fires with `overlaps=0/4`.
///
/// GREEN after the change, because the activation that receives the body — and
/// every `&T` nested inside it — has RETURNED before the activation that
/// performs the release is entered.
#[test]
fn body_environment_protector_expires_before_the_borrowed_frame_pops() {
    let probes_before = probe_firings();
    let frees_before = frees_inside_window();
    // Every body of every scope executed — the primary anti-vacuity census.
    let ran = AtomicUsize::new(0);
    // Scopes whose last-finishing body ran on a genuine worker.
    let mut windows = 0usize;

    let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

    for k in 0..SCOPES {
        let (done, last_wid) = scope_over_a_frame_that_pops(&pool, &ran);
        assert_eq!(
            done, TASKS_PER_SCOPE,
            "scope {k}: the join returned with bodies still unfinished"
        );
        if ran_on_worker(last_wid) {
            windows += 1;
        }
    }

    assert_eq!(
        ran.load(Ordering::Acquire),
        SCOPES * TASKS_PER_SCOPE,
        "every spawned body must have run: a short count means the gate measured less than it \
         claims"
    );

    // One pre-formatted line, for the `-Zmiri-many-seeds` reason given above.
    let census = format!(
        "KE16-PROTECTOR-GATE-CENSUS-ENV variant={} scopes={SCOPES} \
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

    #[cfg(miri)]
    assert_eq!(
        windows, SCOPES,
        "the shape guarantees every scope's last body runs on a worker; a shortfall means the \
         schedule stopped matching the design and this run decided less than it claims"
    );

    assert_probe_armed(probes_before, frees_before, SCOPES, "body-environment arm");
}

/// Drives ONE scope whose bodies borrow only locations of THIS frame, and
/// returns as soon as the join does.
///
/// `#[inline(never)]` so the frame is a frame: inlined into the loop above, the
/// four locals would live as long as the test and the reclamation this arm is
/// built to observe would never happen. Everything the caller needs is READ OUT
/// before the return — two atomic loads — so nothing after the join delays the
/// pop, which is the event under test.
///
/// `ran` is the caller's, deliberately: it accumulates across scopes and must
/// NOT be reclaimed here, so a completer holding it is never the reason this arm
/// is red.
#[inline(never)]
fn scope_over_a_frame_that_pops(pool: &ThreadPool, ran: &AtomicUsize) -> (usize, u32) {
    let started = AtomicUsize::new(0);
    let go = AtomicBool::new(false);
    let done = AtomicUsize::new(0);
    let last_wid = AtomicU32::new(u32::MAX);

    // Same handshake as `completer_holds_no_protector_when_the_joiner_frees`:
    // every body is claimed by a genuine worker before the join begins, so the
    // last decrement is a worker's by construction rather than by luck.
    pool.install(|scope| {
        let ran = &ran;
        let started = &started;
        let go = &go;
        let done = &done;
        let last_wid = &last_wid;
        for _ in 0..TASKS_PER_SCOPE {
            scope.spawn(move || {
                started.fetch_add(1, Ordering::AcqRel);
                spin_until(|| go.load(Ordering::Acquire), "the go gate");
                ran.fetch_add(1, Ordering::Relaxed);
                if done.fetch_add(1, Ordering::AcqRel) == TASKS_PER_SCOPE - 1 {
                    last_wid.store(current_worker_id(), Ordering::Release);
                }
            });
        }

        spin_until(
            || started.load(Ordering::Acquire) == TASKS_PER_SCOPE,
            "every body to be claimed by a worker",
        );
        go.store(true, Ordering::Release);
    });

    (
        done.load(Ordering::Acquire),
        last_wid.load(Ordering::Acquire),
    )
}
