//! Defect A6, pool half — a panic inside a pool task must reach the thread that owns the scope,
//! and must never leave that thread blocked.
//!
//! # What is under test
//!
//! The expected behaviour is `std::thread::scope`'s and rayon's: a panic in a scoped task is
//! caught at the task boundary, the scope still waits for its other tasks, the FIRST payload is
//! resumed on the thread that called `install` / `scope` once the scope ends, and the pool stays
//! usable afterwards. A detached (`ThreadPool::spawn`) task has no joiner; its documented policy
//! is to abort the process (`worker::abort_on_task_panic`, code `E0201`), and what this file
//! requires of it is only that the process ENDS, visibly, instead of hanging in shutdown.
//!
//! # How a hang becomes a red instead of a stuck suite
//!
//! Every scenario runs on a spawned thread through [`watch`]; the test thread waits on a channel
//! with a [`DEADLINE`] and fails with a `WATCHDOG` message when it expires. A blocked scenario
//! therefore fails in ten seconds and leaves its thread parked or spinning until the test binary
//! exits — libtest ends the process when `main` returns, so a leaked thread cannot keep the run
//! alive.
//!
//! The scenarios whose failure mode is PROCESS DEATH (the detached spawns, whose policy is an
//! abort, and the double panics and drop-panicking payloads, which Rust can turn into an abort)
//! re-execute this test binary as a child and watch the child instead — an abort in-process would
//! take every other test in the binary down with it and report nothing.
//!
//! # The child watchdog judges the scope's time, not the runtime's or the OS's (ci-fix-2, F6)
//!
//! Two phases of a child's life are not the pool's work, and the old 10 s wall clock charged both
//! to it:
//!
//! - **The panic hook.** A child that panics a second time while unwinding (`panic_count >= 2`:
//!   the W1 and drop-bomb children) gets a FULL backtrace from std's default hook whatever
//!   `RUST_BACKTRACE` says, and on Windows that is dbghelp symbolisation. CI run 39663719484 killed
//!   both such children at the old 10 s wall clock with their stderr ending at `stack backtrace:`
//!   and nothing after it.
//! - **The exit after the documented abort.** A detached task's panic ends in
//!   `worker::abort_on_task_panic`: the `boyko-E0201` line, then `std::process::abort()`, which on
//!   Windows is a fail-fast that the OS's error reporting holds while it writes its report. Measured
//!   on the dev host (2026-10-22, 198 aborting children, before and after ci-fix-1, idle and
//!   loaded): median 2.9 s, maximum 33 s from the `boyko-E0201` line to the exit.
//!
//! So every child wraps its panic hook in two stdout markers ([`child_enter`]), and
//! [`run_child_with`] runs the 10 s [`DEADLINE`] only on the time after the child's first line,
//! outside hook episodes and before the abort decision. Separate bounds end every other shape: no
//! first line ([`Budget::startup_cap`]), one hook episode ([`Budget::hook_cap`]), all of them
//! together, which is a panic loop ([`Budget::hook_total_cap`]), the abort decision to the exit
//! ([`Budget::abort_cap`]), and the whole run ([`Budget::wall_cap`]). A scope that never drains
//! makes no hook call and prints no abort decision, so it is still BLOCKED after 10 s of work time,
//! exactly as before. The `run_child_*` controls prove each bound.
//!
//! Every child is also spawned with `_NT_SYMBOL_PATH` and `_NT_ALT_SYMBOL_PATH` removed, so a
//! forced backtrace symbolises from the local PDB and never waits on a symbol server a runner's
//! environment may name.
//!
//! # What makes each test red
//!
//! Stated per test in its doc comment. The controls ([`watchdog_reports_a_returning_scope`],
//! [`watchdog_reports_a_panicking_closure_without_a_pool`]) exist so that a red elsewhere is a
//! statement about the pool and not about the harness: they prove the watchdog distinguishes
//! "returned", "panicked" and — by construction of [`watch`] — "neither".

use std::any::Any;
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

/// How long a scenario may take before it is declared BLOCKED. Every scenario here finishes in
/// well under a second when it does finish; the margin is for a loaded machine, not for the work.
const DEADLINE: Duration = Duration::from_secs(10);

/// Poll period for the child-process watchers. A sleep rather than a spin, so a red scenario does
/// not burn a core for ten seconds on a machine somebody else is using.
const POLL: Duration = Duration::from_millis(5);

/// Workers for the in-process scenarios: enough that tasks genuinely run on other threads while
/// the owner joins, few enough to leave the machine's cores to the other test binaries.
const WORKERS: usize = 4;

/// The typed payload the scenarios panic with. A typed payload, not a string, because the claim
/// is that the ORIGINAL `Box<dyn Any>` reaches the owner — a string comparison would also accept
/// a scope that re-panicked with a fresh message of the same text.
#[derive(Debug, PartialEq, Eq)]
struct A6Marker(u32);

/// Runs `f` on its own thread and returns what it did within [`DEADLINE`]: `Ok(value)` if it
/// returned, `Err(payload)` if it panicked. Panics with a `WATCHDOG` message if it did neither.
///
/// `catch_unwind` runs on the SAME thread as `f`, so an `Err` here is a panic that surfaced on the
/// thread that made the call — which is the "owner thread" half of every claim below.
fn watch<T, F>(label: &str, f: F) -> Result<T, Box<dyn Any + Send>>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name(format!("a6-watched-{label}"))
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(f));
            // The receiver is gone only when the watchdog already fired; nothing to report to.
            let _ = tx.send(outcome);
        })
        .expect("test setup: spawn the watched thread");
    match rx.recv_timeout(DEADLINE) {
        Ok(outcome) => outcome,
        Err(RecvTimeoutError::Timeout) => panic!(
            "WATCHDOG: `{label}` neither returned nor panicked within {DEADLINE:?} -- the calling \
             thread is BLOCKED (defect A6)"
        ),
        Err(RecvTimeoutError::Disconnected) => panic!(
            "`{label}`: the watched thread ended without reporting an outcome -- an unwind \
             escaped `catch_unwind`, which is not a shape this harness can classify"
        ),
    }
}

/// Renders a panic payload for a failure message.
fn describe(payload: &(dyn Any + Send)) -> String {
    if let Some(m) = payload.downcast_ref::<A6Marker>() {
        format!("{m:?}")
    } else if let Some(s) = payload.downcast_ref::<&'static str>() {
        format!("&str {s:?}")
    } else if let Some(s) = payload.downcast_ref::<String>() {
        format!("String {s:?}")
    } else {
        "<payload of an unrecognised type>".to_owned()
    }
}

/// The owner-side verdict every propagation test shares: the call must have PANICKED, with an
/// `A6Marker` payload whose id satisfies `accept`.
fn expect_marker(
    label: &str,
    outcome: Result<(), Box<dyn Any + Send>>,
    accept: impl Fn(u32) -> bool,
) {
    match outcome {
        Ok(()) => panic!(
            "`{label}`: the scope call RETURNED normally -- the task's panic was swallowed \
             instead of being resumed on the owner thread"
        ),
        Err(payload) => match payload.downcast_ref::<A6Marker>() {
            Some(A6Marker(id)) => assert!(
                accept(*id),
                "`{label}`: the owner received A6Marker({id}), which no task of this scenario \
                 panicked with"
            ),
            None => panic!(
                "`{label}`: the owner received a panic, but not the task's original payload: {}",
                describe(&*payload)
            ),
        },
    }
}

/// Small amount of non-blocking work, so a task that is still running is observable. Yielding
/// rather than sleeping: a sibling must stay runnable by any joiner (see `tests/shutdown.rs`'s
/// note on blocking primitives inside scope tasks).
fn busy(rounds: usize) {
    for _ in 0..rounds {
        std::thread::yield_now();
    }
}

fn pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(WORKERS).build()
}

// ─────────────────────────────────────────────────────────────────────────────
// Controls — the harness itself
// ─────────────────────────────────────────────────────────────────────────────

/// CONTROL. A scope whose tasks do not panic must be reported as RETURNED, with every task run.
///
/// Red if the harness misclassifies a normal return, or if the fixture pool cannot run a scope at
/// all — either of which would make every propagation red below meaningless.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the scope protocol itself is Miri-covered by `miri_scope*.rs`. Runs natively."
)]
#[test]
fn watchdog_reports_a_returning_scope() {
    let outcome = watch("control-return", || {
        let pool = pool();
        let ran = AtomicUsize::new(0);
        pool.install(|scope| {
            for _ in 0..32 {
                scope.spawn(|| {
                    ran.fetch_add(1, Ordering::Relaxed);
                });
            }
        });
        ran.load(Ordering::Acquire)
    });
    match outcome {
        Ok(ran) => assert_eq!(ran, 32, "the control scope must run every task exactly once"),
        Err(p) => panic!("the control scope panicked: {}", describe(&*p)),
    }
}

/// CONTROL. A closure that panics WITHOUT any pool must be reported as that panic.
///
/// Red if [`watch`] loses or rewrites a payload, in which case the `expect_marker` verdicts below
/// would be testing the harness instead of the pool.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the scope protocol itself is Miri-covered by `miri_scope*.rs`. Runs natively."
)]
#[test]
fn watchdog_reports_a_panicking_closure_without_a_pool() {
    let outcome = watch::<(), _>("control-panic", || panic_any(A6Marker(99)));
    expect_marker("control-panic", outcome, |id| id == 99);
}

// ─────────────────────────────────────────────────────────────────────────────
// Scoped tasks
// ─────────────────────────────────────────────────────────────────────────────

/// One task of sixteen panics; `install` must panic on the owner thread with THAT payload.
///
/// Red: the watchdog fires (the owner is blocked), `install` returns normally (the panic was
/// swallowed), or the owner sees a different payload (the original was lost or replaced).
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the scope protocol itself is Miri-covered by `miri_scope*.rs`. Runs natively."
)]
#[test]
fn scoped_task_panic_resumes_on_owner_with_original_payload() {
    let outcome = watch("scoped-panic", || {
        let pool = pool();
        pool.install(|scope| {
            for i in 0..16u32 {
                scope.spawn(move || {
                    if i == 5 {
                        panic_any(A6Marker(5));
                    }
                    busy(8);
                });
            }
        });
    });
    expect_marker("scoped-panic", outcome, |id| id == 5);
}

/// The scope still JOINS its non-panicking tasks before it resumes the panic.
///
/// The siblings borrow a stack counter and are slower than the panicking task, so a scope that
/// resumed the unwind as soon as it saw the panic would be observed here with fewer than fifteen
/// completions. Red: fewer than 15 siblings completed by the time `install` unwound, `install`
/// did not panic at all, or the watchdog fires.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the scope protocol itself is Miri-covered by `miri_scope*.rs`. Runs natively."
)]
#[test]
fn scoped_task_panic_still_joins_every_sibling_task() {
    let outcome = watch("scoped-siblings", || {
        let pool = pool();
        let siblings_done = AtomicUsize::new(0);
        let caught = catch_unwind(AssertUnwindSafe(|| {
            pool.install(|scope| {
                scope.spawn(|| panic_any(A6Marker(1)));
                for _ in 0..15 {
                    scope.spawn(|| {
                        busy(2_000);
                        siblings_done.fetch_add(1, Ordering::AcqRel);
                    });
                }
            });
        }));
        (caught.is_err(), siblings_done.load(Ordering::Acquire))
    });
    match outcome {
        Ok((panicked, done)) => {
            assert!(panicked, "`install` must resume the task's panic on the owner");
            assert_eq!(
                done, 15,
                "every non-panicking sibling must have completed before the scope resumed the panic"
            );
        }
        Err(p) => panic!("the watched scenario escaped its own catch: {}", describe(&*p)),
    }
}

/// After a task panic has been resumed, the SAME pool must run a later scope to completion.
///
/// Red: the second `install` blocks (watchdog), panics (stale payload or poisoned state), or runs
/// fewer than all of its tasks (a worker was lost to the first panic).
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the scope protocol itself is Miri-covered by `miri_scope*.rs`. Runs natively."
)]
#[test]
fn pool_runs_a_later_scope_after_a_task_panic() {
    let outcome = watch("reuse-after-panic", || {
        let pool = pool();
        let first = catch_unwind(AssertUnwindSafe(|| {
            pool.install(|scope| {
                scope.spawn(|| panic_any(A6Marker(2)));
            });
        }));
        assert!(first.is_err(), "stage 1: the first scope must resume the task panic");

        let ran = AtomicUsize::new(0);
        pool.install(|scope| {
            for _ in 0..256 {
                scope.spawn(|| {
                    ran.fetch_add(1, Ordering::Relaxed);
                });
            }
        });
        ran.load(Ordering::Acquire)
    });
    match outcome {
        Ok(ran) => assert_eq!(ran, 256, "stage 2: the later scope must run all of its tasks"),
        Err(p) => panic!("reuse-after-panic failed: {}", describe(&*p)),
    }
}

/// The panicking task is nested: a task opens `ThreadPool::scope` on a WORKER and its inner task
/// panics. The inner scope's join (the worker joiner) must resume it into the outer task, and the
/// outer scope must resume it on the owner — with the original payload.
///
/// Red: watchdog (a joiner blocked on either level), a normal return, or a replaced payload.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the scope protocol itself is Miri-covered by `miri_scope*.rs`. Runs natively."
)]
#[test]
fn nested_scoped_task_panic_resumes_on_outer_owner() {
    let outcome = watch("nested-panic", || {
        let pool = pool();
        let inner_pool = Arc::clone(&pool);
        pool.install(|outer| {
            for k in 0..4u32 {
                let inner_pool = Arc::clone(&inner_pool);
                outer.spawn(move || {
                    inner_pool.scope(|inner| {
                        for j in 0..8u32 {
                            inner.spawn(move || {
                                if k == 2 && j == 3 {
                                    panic_any(A6Marker(23));
                                }
                                busy(8);
                            });
                        }
                    });
                });
            }
        });
    });
    expect_marker("nested-panic", outcome, |id| id == 23);
}

/// After a NESTED task panic (the worker-joiner path), the pool must still run a later nested
/// wave to completion.
///
/// Red: the later wave blocks (watchdog), panics, or loses tasks.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the scope protocol itself is Miri-covered by `miri_scope*.rs`. Runs natively."
)]
#[test]
fn pool_runs_a_later_nested_wave_after_a_nested_panic() {
    let outcome = watch("nested-reuse", || {
        let pool = pool();
        let first = catch_unwind(AssertUnwindSafe(|| {
            let inner_pool = Arc::clone(&pool);
            pool.install(|outer| {
                outer.spawn(move || {
                    inner_pool.scope(|inner| {
                        inner.spawn(|| panic_any(A6Marker(3)));
                    });
                });
            });
        }));
        assert!(first.is_err(), "stage 1: the nested panic must reach the owner");

        let ran = Arc::new(AtomicUsize::new(0));
        let inner_pool = Arc::clone(&pool);
        pool.install(|outer| {
            for _ in 0..4 {
                let inner_pool = Arc::clone(&inner_pool);
                let ran = Arc::clone(&ran);
                outer.spawn(move || {
                    inner_pool.scope(|inner| {
                        for _ in 0..16 {
                            inner.spawn(|| {
                                ran.fetch_add(1, Ordering::Relaxed);
                            });
                        }
                    });
                });
            }
        });
        ran.load(Ordering::Acquire)
    });
    match outcome {
        Ok(ran) => assert_eq!(ran, 64, "stage 2: the later nested wave must run all 64 tasks"),
        Err(p) => panic!("nested-reuse failed: {}", describe(&*p)),
    }
}

/// Eight tasks all panic. Exactly one payload — any of the eight — must reach the owner (first
/// panic wins); the others are dropped without blocking or aborting.
///
/// Red: watchdog, a normal return, or a payload that is not one of the eight markers.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the scope protocol itself is Miri-covered by `miri_scope*.rs`. Runs natively."
)]
#[test]
fn several_panicking_tasks_resume_one_of_their_payloads() {
    let outcome = watch("several-panics", || {
        let pool = pool();
        pool.install(|scope| {
            for i in 0..8u32 {
                scope.spawn(move || {
                    busy(4);
                    panic_any(A6Marker(100 + i));
                });
            }
        });
    });
    expect_marker("several-panics", outcome, |id| (100..108).contains(&id));
}

/// `ThreadPool::scope` opened on the DISPATCHER (inside `install`, the shape `par_iter` takes when
/// called from an exclusive system) — the external-joiner path — must resume its task's panic.
///
/// Red: watchdog, a normal return, or a replaced payload.
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 10 s wall-clock watchdog on a spawned thread, and a red path that leaks a blocked thread by design; the scope protocol itself is Miri-covered by `miri_scope*.rs`. Runs natively."
)]
#[test]
fn dispatcher_side_scope_task_panic_resumes_on_owner() {
    let outcome = watch("dispatcher-scope", || {
        let pool = pool();
        pool.install(|_outer| {
            pool.scope(|inner| {
                for i in 0..16u32 {
                    inner.spawn(move || {
                        if i == 9 {
                            panic_any(A6Marker(9));
                        }
                        busy(8);
                    });
                }
            });
        });
    });
    expect_marker("dispatcher-scope", outcome, |id| id == 9);
}

// ─────────────────────────────────────────────────────────────────────────────
// Child-process scenarios — the failure mode is process death
// ─────────────────────────────────────────────────────────────────────────────

/// Selects the child half of a re-executed test; its value names the scenario.
const CHILD_ENV: &str = "BOYKO_A6_POOL_CHILD";

/// What a child's panic hook prints to stdout before the hook it wraps runs. The parent's work
/// clock stops here.
const HOOK_ENTER: &str = "A6-HOOK-ENTER";

/// What a child's panic hook prints after the hook it wraps returned, followed by the episode's
/// length in microseconds as the CHILD measured it. The parent's work clock resumes here.
const HOOK_EXIT: &str = "A6-HOOK-EXIT";

/// Harness controls only: the child's panic hook sleeps this many milliseconds before the hook it
/// wraps. [`run_child_with`] removes it from every child it is not passed to.
const HOOK_STALL_ENV: &str = "BOYKO_A6_HOOK_STALL_MS";

/// How much of each child stream the parent keeps from the start, and again from the end. A child
/// that panics in a loop must fill neither the parent's memory nor its failure message.
const STREAM_KEEP: usize = 128 * 1024;

/// The variables dbghelp's `SymInitialize` reads for its symbol search path when the caller passes
/// none, as std's backtrace does. A symbol server named in either would put network fetches inside
/// a child's forced backtrace, so [`child_command`] removes both from every child.
const SYMBOL_PATH_ENV: [&str; 2] = ["_NT_SYMBOL_PATH", "_NT_ALT_SYMBOL_PATH"];

/// The prefix of the line `worker::abort_on_task_panic` prints before it aborts the process (its
/// `NoConsumer` fallback: every child runs with `BOYKO_LOG` removed). The parent's work clock stops
/// for good at a stderr line that carries it.
fn abort_decision_tag() -> String {
    format!("boyko-E{:04}", boyko_log::codes::E0201.number())
}

/// The bound a child crossed when the watchdog killed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// No line on either stream within [`Budget::startup_cap`] of the spawn.
    Startup,
    /// The scope's own time, outside every panic-hook episode and before any abort decision,
    /// passed [`Budget::work`].
    Work,
    /// One panic-hook episode passed [`Budget::hook_cap`].
    Hook,
    /// All panic-hook episodes together passed [`Budget::hook_total_cap`]: a panic loop.
    HookTotal,
    /// The child printed its abort decision ([`abort_decision_tag`]) and was still alive
    /// [`Budget::abort_cap`] later.
    Abort,
    /// Spawn to now passed [`Budget::wall_cap`].
    Wall,
}

/// The bounds one child runs under.
#[derive(Clone, Copy, Debug)]
struct Budget {
    /// Work time: wall time after the child's first line, outside panic-hook episodes and before
    /// the abort decision.
    work: Duration,
    /// One panic-hook episode.
    hook_cap: Duration,
    /// Every panic-hook episode together.
    hook_total_cap: Duration,
    /// The abort decision to the exit.
    abort_cap: Duration,
    /// Spawn to the child's first line.
    startup_cap: Duration,
    /// Spawn to exit, whatever the time was spent on.
    wall_cap: Duration,
}

impl Budget {
    /// What every scenario runs under. The work budget is [`DEADLINE`], unchanged. The caps are
    /// generous because a forced full backtrace on a loaded runner, and the OS's report of a
    /// fail-fast exit (33 s at the most in the dev host's 198 measured aborts), are slow, not stuck;
    /// each of them still ends a stuck child.
    const DEFAULT: Budget = Budget {
        work: DEADLINE,
        hook_cap: Duration::from_secs(120),
        hook_total_cap: Duration::from_secs(240),
        abort_cap: Duration::from_secs(120),
        startup_cap: Duration::from_secs(60),
        wall_cap: Duration::from_secs(300),
    };

    /// The first bound `clocks` has crossed at `now`, if any.
    fn crossed(&self, clocks: &Clocks, spawned: Instant, now: Instant) -> Option<Phase> {
        if !clocks.started && now - spawned >= self.startup_cap {
            return Some(Phase::Startup);
        }
        if let Some(decided) = clocks.abort_start {
            // Terminal: the child has decided to end the process, and only the time that takes is
            // judged from here (the work and hook clocks no longer move).
            if now - decided >= self.abort_cap {
                return Some(Phase::Abort);
            }
        } else {
            if clocks.work >= self.work {
                return Some(Phase::Work);
            }
            if let Some(episode) = clocks.episode_start
                && now - episode >= self.hook_cap
            {
                return Some(Phase::Hook);
            }
            if clocks.hook >= self.hook_total_cap {
                return Some(Phase::HookTotal);
            }
        }
        if now - spawned >= self.wall_cap {
            return Some(Phase::Wall);
        }
        None
    }
}

/// What the parent observed of one child run.
struct ChildRun {
    /// `None` when the watchdog had to kill the child; [`ChildRun::blocked`] says why.
    success: Option<bool>,
    stdout: String,
    stderr: String,
    /// The bound the child crossed, when it was killed.
    blocked: Option<Phase>,
    /// The parent's work clock at the end.
    work: Duration,
    /// The parent's hook clock at the end: every panic-hook episode together.
    hook: Duration,
    /// From the child's abort decision to its exit (or its kill); `None` when it printed none.
    abort: Option<Duration>,
    /// Each completed hook episode as the CHILD measured it (the [`HOOK_EXIT`] markers).
    child_hooks: Vec<Duration>,
    /// The bounds the child ran under.
    budget: Budget,
}

impl ChildRun {
    /// The watchdog's verdict for a failure message: the bound and every clock.
    fn watchdog(&self) -> String {
        format!(
            "BLOCKED in phase {:?}: work {:.1?} of a {:.1?} budget, hook {:.1?} over {} completed \
             episode(s), abort decision to end {:.1?} (caps: episode {:.0?}, total {:.0?}, abort \
             {:.0?}, wall {:.0?})",
            self.blocked,
            self.work,
            self.budget.work,
            self.hook,
            self.child_hooks.len(),
            self.abort,
            self.budget.hook_cap,
            self.budget.hook_total_cap,
            self.budget.abort_cap,
            self.budget.wall_cap
        )
    }
}

/// Which child stream a line came from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Out,
    Err,
}

/// One line as a reader thread saw it (terminator included), or `None` at the stream's end.
struct Line {
    stream: Stream,
    at: Instant,
    bytes: Option<Vec<u8>>,
}

/// The watchdog's clocks, advanced line by line.
struct Clocks {
    /// The child has printed its first line; work time counts from here.
    started: bool,
    /// Open panic-hook episodes (threads that panic together nest).
    depth: u32,
    /// Where the clocks stand.
    since: Instant,
    /// The child's end (its exit seen, or its kill): no clock moves past it.
    until: Option<Instant>,
    work: Duration,
    hook: Duration,
    /// When the current outermost hook episode began.
    episode_start: Option<Instant>,
    /// When the child printed its abort decision; from here every span is the abort's.
    abort_start: Option<Instant>,
    /// [`abort_decision_tag`], computed once per child.
    abort_tag: String,
    child_hooks: Vec<Duration>,
}

impl Clocks {
    fn new(spawned: Instant) -> Self {
        Clocks {
            started: false,
            depth: 0,
            since: spawned,
            until: None,
            work: Duration::ZERO,
            hook: Duration::ZERO,
            episode_start: None,
            abort_start: None,
            abort_tag: abort_decision_tag(),
            child_hooks: Vec::new(),
        }
    }

    /// Charge the time from where the clocks stand to `to` to work or to the hook. Nothing is
    /// charged after the abort decision: that time is judged as a whole, from `abort_start`.
    fn advance(&mut self, to: Instant) {
        let to = self.until.map_or(to, |end| to.min(end)).max(self.since);
        let span = to - self.since;
        if self.abort_start.is_none() {
            if self.depth > 0 {
                self.hook += span;
            } else if self.started {
                self.work += span;
            }
        }
        self.since = to;
    }

    /// The child ended at `at`: charge the last span, and stop every clock there.
    fn end(&mut self, at: Instant) {
        self.advance(at);
        self.until = Some(self.since);
    }

    /// A line the child printed at `at`. Only stdout carries the hook markers; libtest may prefix
    /// the first one with its `test <name> ... ` text, so a marker is matched at the line's end.
    /// Only stderr carries the abort decision; it is matched anywhere in the line, because another
    /// thread's unterminated stderr text may precede it.
    fn line(&mut self, at: Instant, stream: Stream, bytes: &[u8]) {
        self.advance(at);
        self.started = true;
        let text = String::from_utf8_lossy(bytes);
        let text = text.trim_end();
        if stream == Stream::Err {
            if self.abort_start.is_none() && text.contains(self.abort_tag.as_str()) {
                self.abort_start = Some(self.since);
            }
            return;
        }
        if text.ends_with(HOOK_ENTER) {
            if self.depth == 0 {
                self.episode_start = Some(self.since);
            }
            self.depth += 1;
        } else if let Some((_, micros)) = text.rsplit_once(HOOK_EXIT) {
            self.depth = self.depth.saturating_sub(1);
            if self.depth == 0 {
                self.episode_start = None;
            }
            if let Ok(micros) = micros.trim().parse::<u64>() {
                self.child_hooks.push(Duration::from_micros(micros));
            }
        }
    }
}

/// A stream as the parent keeps it: the first [`STREAM_KEEP`] bytes and the last, by line.
#[derive(Default)]
struct Kept {
    head: Vec<u8>,
    tail: VecDeque<Vec<u8>>,
    tail_bytes: usize,
    dropped: usize,
}

impl Kept {
    fn push(&mut self, line: Vec<u8>) {
        if self.tail.is_empty() && self.head.len() < STREAM_KEEP {
            self.head.extend_from_slice(&line);
            return;
        }
        self.tail_bytes += line.len();
        self.tail.push_back(line);
        while self.tail_bytes > STREAM_KEEP {
            let Some(old) = self.tail.pop_front() else { break };
            self.tail_bytes -= old.len();
            self.dropped += old.len();
        }
    }

    fn into_string(self) -> String {
        let mut out = self.head;
        if self.dropped > 0 {
            let note = format!("\n[a6 harness: {} bytes dropped here]\n", self.dropped);
            out.extend_from_slice(note.as_bytes());
        }
        for line in self.tail {
            out.extend_from_slice(&line);
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

/// Reads `pipe` line by line WHILE the child runs, stamping each line on arrival. Draining while
/// the child runs also means a child can never block on a full pipe, which a parent that reads only
/// after exit allowed (a full backtrace can pass the pipe's buffer).
fn spawn_reader(stream: Stream, pipe: impl Read + Send + 'static, tx: Sender<Line>) {
    std::thread::Builder::new()
        .name("a6-child-reader".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(pipe);
            loop {
                let mut bytes = Vec::new();
                let read = reader.read_until(b'\n', &mut bytes);
                let at = Instant::now();
                let end = !matches!(read, Ok(n) if n > 0);
                let line = Line { stream, at, bytes: (!end).then_some(bytes) };
                if tx.send(line).is_err() || end {
                    return;
                }
            }
        })
        .expect("test setup: spawn a child-stream reader");
}

/// [`run_child_with`] under [`Budget::DEFAULT`].
fn run_child(test_name: &str, scenario: &str) -> ChildRun {
    run_child_with(test_name, scenario, Budget::DEFAULT, &[])
}

/// The command that re-executes `exe` running exactly `test_name` with `CHILD_ENV=scenario` and
/// `env`. The symbol-path removal comes last, so no caller's `env` can hand a child a symbol server.
fn child_command(exe: &Path, test_name: &str, scenario: &str, env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(exe);
    command
        .args(["--exact", test_name, "--nocapture", "--test-threads", "1"])
        .env(CHILD_ENV, scenario)
        // The default logging configuration, in which the abort sites' `NoConsumer` fallback
        // lines are the observable.
        .env_remove("BOYKO_LOG")
        .env_remove(HOOK_STALL_ENV)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    for key in SYMBOL_PATH_ENV {
        command.env_remove(key);
    }
    command
}

/// Re-executes this test binary running exactly `test_name` with `CHILD_ENV=scenario` and `env`,
/// and watches it under `budget`, killing it at the first bound it crosses.
fn run_child_with(test_name: &str, scenario: &str, budget: Budget, env: &[(&str, &str)]) -> ChildRun {
    let exe = std::env::current_exe().expect("test setup: this test binary's own path");
    let mut child = child_command(&exe, test_name, scenario, env)
        .spawn()
        .expect("test setup: re-exec of this test binary");
    let spawned = Instant::now();

    let (tx, rx) = channel();
    spawn_reader(Stream::Out, child.stdout.take().expect("test setup: piped stdout"), tx.clone());
    spawn_reader(Stream::Err, child.stderr.take().expect("test setup: piped stderr"), tx);

    let mut clocks = Clocks::new(spawned);
    let (mut out, mut err) = (Kept::default(), Kept::default());
    // Returns whether the line was a stream's end.
    let mut take = |line: Line, clocks: &mut Clocks| match line.bytes {
        None => true,
        Some(bytes) => {
            clocks.line(line.at, line.stream, &bytes);
            match line.stream {
                Stream::Out => out.push(bytes),
                Stream::Err => err.push(bytes),
            }
            false
        }
    };
    let mut open = 2;

    let mut blocked = None;
    let success = loop {
        while let Ok(line) = rx.try_recv() {
            if take(line, &mut clocks) {
                open -= 1;
            }
        }
        if let Some(status) = child.try_wait().expect("test setup: poll the child") {
            clocks.end(Instant::now());
            break Some(status.success());
        }
        let now = Instant::now();
        clocks.advance(now);
        blocked = budget.crossed(&clocks, spawned, now);
        if blocked.is_some() {
            clocks.end(now);
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(POLL);
    };

    // The child is gone, so both pipes reach their end; a bound keeps a stray inheritor of a pipe
    // from stalling the parent. Lines read now still count as markers (an abort decision printed
    // just before the exit is often read after it), but every clock already stopped at the end.
    let drain_until = Instant::now() + Duration::from_secs(5);
    while open > 0 {
        match rx.recv_timeout(drain_until.saturating_duration_since(Instant::now())) {
            Ok(line) => {
                if take(line, &mut clocks) {
                    open -= 1;
                }
            }
            Err(_) => break,
        }
    }
    let ended =clocks.until.expect("invariant: the watch loop ends the clocks before it breaks");
    ChildRun {
        success,
        stdout: out.into_string(),
        stderr: err.into_string(),
        blocked,
        work: clocks.work,
        hook: clocks.hook,
        abort: clocks.abort_start.map(|decided| ended.saturating_duration_since(decided)),
        child_hooks: clocks.child_hooks,
        budget,
    }
}

/// The first call of every child branch: wraps the panic hook in [`HOOK_ENTER`] / [`HOOK_EXIT`]
/// so the parent can tell the hook's time from the scope's.
///
/// The markers go to STDOUT, one `write_all` each under std's re-entrant stdout lock, so they never
/// interleave inside the default hook's stderr text. An abort inside the hook leaves an ENTER with
/// no EXIT, and the process exit ends the watch. The stall knob is read here, once, not inside the
/// hook.
fn child_enter() {
    let stall = std::env::var(HOOK_STALL_ENV)
        .ok()
        .and_then(|ms| ms.parse::<u64>().ok())
        .map(Duration::from_millis);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let start = Instant::now();
        hook_marker(HOOK_ENTER);
        if let Some(stall) = stall {
            std::thread::sleep(stall);
        }
        previous(info);
        hook_marker(&format!("{HOOK_EXIT} {}", start.elapsed().as_micros()));
    }));
}

/// One marker line on stdout, flushed.
fn hook_marker(line: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(format!("{line}\n").as_bytes());
    let _ = out.flush();
}

/// Prints a child marker and flushes, so it survives a following abort.
fn child_mark(line: &str) {
    use std::io::Write;
    println!("{line}");
    std::io::stdout().flush().expect("test setup: flush");
}

/// Spins until `flag` is set, for at most [`DEADLINE`]. Returns whether it was set.
///
/// Bounded rather than open-ended so that "the task never ran" is a DIFFERENT observation from
/// "shutdown blocked": an unbounded wait here would be killed by the parent and reported as a
/// shutdown hang, which is a red for the wrong reason.
fn wait_for(flag: &AtomicBool) -> bool {
    let start = Instant::now();
    while !flag.load(Ordering::Acquire) {
        if start.elapsed() >= DEADLINE {
            return false;
        }
        std::thread::yield_now();
    }
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Controls — the child watchdog (ci-fix-2, F6)
// ─────────────────────────────────────────────────────────────────────────────

/// Never returns: the child stays alive, doing nothing, until the parent kills it.
fn park_forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Prints an abort decision the way `worker::abort_on_task_panic` does when no log consumer is
/// installed: one stderr line that starts with [`abort_decision_tag`].
fn print_abort_decision() {
    eprintln!(
        "{}: a6 harness control: an abort decision, printed the way the pool's backstop prints it",
        abort_decision_tag()
    );
}

/// Child half of the `hook-stall` scenario: one ordinary (count-1) panic, caught, with the hook
/// stalled by [`HOOK_STALL_ENV`]; then the child finishes.
fn child_hook_stall() {
    child_enter();
    child_mark("CHILD-HOOK-STALL-START");
    let caught = catch_unwind::<_, ()>(|| panic_any(A6Marker(90)));
    child_mark(if caught.is_err() { "CHILD-HOOK-STALL-DONE" } else { "CHILD-HOOK-STALL-NO-PANIC" });
}

const SLOW_HOOK_TEST: &str = "run_child_does_not_charge_a_slow_panic_hook_to_the_scope";

/// CONTROL. A child whose panic hook takes 3 s, longer than this run's whole 2 s work budget, and
/// that then finishes, must be reported as FINISHED: the hook's time is not the scope's.
///
/// This is the shape CI run 39663719484 hit (a forced full backtrace on a loaded runner), injected
/// by the stall knob. Red under the old wall-clock watchdog: killed at 2 s with stdout ending at
/// `A6-HOOK-ENTER`.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn run_child_does_not_charge_a_slow_panic_hook_to_the_scope() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("hook-stall") {
        return child_hook_stall();
    }
    let budget = Budget {
        work: Duration::from_secs(2),
        hook_cap: Duration::from_secs(30),
        ..Budget::DEFAULT
    };
    let run = run_child_with(SLOW_HOOK_TEST, "hook-stall", budget, &[(HOOK_STALL_ENV, "3000")]);
    assert_eq!(
        run.blocked,
        None,
        "a 3 s panic hook was charged to the scope's 2 s work budget: {}.\nstdout:\n{}\nstderr:\n{}",
        run.watchdog(),
        run.stdout,
        run.stderr
    );
    assert_eq!(
        run.success,
        Some(true),
        "the child must finish and exit 0 -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("CHILD-HOOK-STALL-DONE"),
        "the child never reached its end -- stdout:\n{}",
        run.stdout
    );
    // The child's own clock: deterministic, it slept that long inside the hook.
    assert!(
        run.child_hooks.iter().any(|d| *d >= Duration::from_millis(3000)),
        "fixture: no hook episode of >= 3 s by the child's own clock ({:?}), so the stall knob did \
         not reach the hook",
        run.child_hooks
    );
    // The parent's clock, with 0.5 s of margin for a reader thread's wake-up latency.
    assert!(
        run.hook >= Duration::from_millis(2500),
        "the parent charged only {:?} to the hook for a 3 s stall: the episode markers were not read \
         as an episode",
        run.hook
    );
}

const BLOCKED_TEST: &str = "run_child_reports_a_child_blocked_outside_its_hook";

/// CONTROL. A child whose scope never drains, with no panic at all, must be reported BLOCKED in the
/// `Work` phase once its work budget is spent: the watchdog still catches the hang it exists for.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn run_child_reports_a_child_blocked_outside_its_hook() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("blocked") {
        child_enter();
        let pool = ThreadPoolBuilder::new().num_threads(1).build();
        let never = AtomicBool::new(false);
        child_mark("CHILD-BLOCKED-START");
        pool.install(|scope| {
            scope.spawn(|| {
                while !never.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            });
        });
        child_mark("CHILD-BLOCKED-RETURNED");
        return;
    }
    let budget = Budget { work: Duration::from_secs(2), ..Budget::DEFAULT };
    let run = run_child_with(BLOCKED_TEST, "blocked", budget, &[]);
    assert_eq!(
        run.blocked,
        Some(Phase::Work),
        "a scope that never drains must be BLOCKED in the work phase; the watchdog said {}.\n\
         stdout:\n{}\nstderr:\n{}",
        run.watchdog(),
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("CHILD-BLOCKED-START"),
        "fixture: the child never started its scope -- stdout:\n{}",
        run.stdout
    );
    assert_eq!(run.hook, Duration::ZERO, "no panic happened, so no time is the hook's");
}

const STUCK_HOOK_TEST: &str = "run_child_reports_a_child_stuck_inside_its_panic_hook";

/// CONTROL. A child stuck inside its panic hook past the episode cap must be reported BLOCKED in
/// the `Hook` phase: the time the work clock does not count is still bounded.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn run_child_reports_a_child_stuck_inside_its_panic_hook() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("hook-stall") {
        return child_hook_stall();
    }
    let budget = Budget { hook_cap: Duration::from_secs(2), ..Budget::DEFAULT };
    let run = run_child_with(STUCK_HOOK_TEST, "hook-stall", budget, &[(HOOK_STALL_ENV, "5000")]);
    assert_eq!(
        run.blocked,
        Some(Phase::Hook),
        "a 5 s hook episode must cross a 2 s episode cap; the watchdog said {}.\nstdout:\n{}\n\
         stderr:\n{}",
        run.watchdog(),
        run.stdout,
        run.stderr
    );
}

const HOOK_LOOP_TEST: &str = "run_child_reports_a_child_that_panics_in_a_loop";

/// CONTROL. A child that panics again and again, each panic a short hook episode, spends almost no
/// time outside its hook, so its work clock barely moves. It must be reported BLOCKED in the
/// `HookTotal` phase within its ceiling — the shape a per-episode cap alone would let run for hours.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn run_child_reports_a_child_that_panics_in_a_loop() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("hook-loop") {
        child_enter();
        child_mark("CHILD-HOOK-LOOP-START");
        loop {
            let _ = catch_unwind::<_, ()>(|| panic_any(A6Marker(91)));
        }
    }
    let budget = Budget {
        hook_total_cap: Duration::from_secs(2),
        wall_cap: Duration::from_secs(20),
        ..Budget::DEFAULT
    };
    let run = run_child_with(HOOK_LOOP_TEST, "hook-loop", budget, &[(HOOK_STALL_ENV, "50")]);
    assert_eq!(
        run.blocked,
        Some(Phase::HookTotal),
        "a child panicking in a loop must cross the 2 s total hook cap; the watchdog said {}.\n\
         stdout:\n{}\nstderr:\n{}",
        run.watchdog(),
        run.stdout,
        run.stderr
    );
    assert!(
        run.child_hooks.len() >= 2,
        "fixture: the loop completed {} hook episodes",
        run.child_hooks.len()
    );
}

const SLOW_ABORT_TEST: &str = "run_child_does_not_charge_a_slow_exit_after_the_abort_decision_to_the_scope";

/// The exit code of the abort-decision controls' children. Not `std::process::abort()`: the
/// property is the time from the decision to the exit, and a real fail-fast would add an OS crash
/// report and a dump per run to what the control injects.
const ABORT_CONTROL_EXIT: i32 = 3;

/// CONTROL. A child that prints its abort decision and then takes 3 s to exit, longer than this
/// run's whole 2 s work budget, must be reported as ENDED, not BLOCKED: the time from the decision
/// to the exit is not the scope's.
///
/// This is the shape the dev host reproduces (2026-10-22): an aborting child held for seconds by
/// the OS's error reporting after `boyko-E0201`, and BLOCKED by the old 10 s wall clock. Red under
/// the old wall-clock watchdog, and under a watchdog that judges only the hook (killed in `Work`
/// at 2 s).
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn run_child_does_not_charge_a_slow_exit_after_the_abort_decision_to_the_scope() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("abort-slow") {
        child_enter();
        child_mark("CHILD-ABORT-SLOW-START");
        print_abort_decision();
        std::thread::sleep(Duration::from_millis(3000));
        std::process::exit(ABORT_CONTROL_EXIT);
    }
    let budget = Budget {
        work: Duration::from_secs(2),
        abort_cap: Duration::from_secs(30),
        ..Budget::DEFAULT
    };
    let run = run_child_with(SLOW_ABORT_TEST, "abort-slow", budget, &[]);
    assert_eq!(
        run.blocked,
        None,
        "a 3 s exit after the abort decision was charged to the scope's 2 s work budget: {}.\n\
         stdout:\n{}\nstderr:\n{}",
        run.watchdog(),
        run.stdout,
        run.stderr
    );
    assert_eq!(
        run.success,
        Some(false),
        "the child must end by its own non-zero exit -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("CHILD-ABORT-SLOW-START"),
        "fixture: the child never ran its scenario -- stdout:\n{}",
        run.stdout
    );
    let abort = run.abort.unwrap_or_else(|| {
        panic!(
            "the child printed an abort decision, and the watchdog never read it as one: {}.\n\
             stderr:\n{}",
            run.watchdog(),
            run.stderr
        )
    });
    // The parent's clock, with 0.5 s of margin for a reader thread's wake-up latency.
    assert!(
        abort >= Duration::from_millis(2500),
        "the parent charged only {abort:?} to the abort for a 3 s wait after the decision"
    );
}

const STUCK_ABORT_TEST: &str = "run_child_reports_a_child_stuck_after_its_abort_decision";

/// CONTROL, the first of the two that prove the abort bound can fail. A child that prints its abort
/// decision and then never exits must be reported BLOCKED in the `Abort` phase at its cap, and not
/// before: a 2 s work budget beside a 4 s abort cap tells the two apart.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn run_child_reports_a_child_stuck_after_its_abort_decision() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("abort-stuck") {
        child_enter();
        child_mark("CHILD-ABORT-STUCK-START");
        print_abort_decision();
        park_forever();
    }
    let budget = Budget {
        work: Duration::from_secs(2),
        abort_cap: Duration::from_secs(4),
        ..Budget::DEFAULT
    };
    let run = run_child_with(STUCK_ABORT_TEST, "abort-stuck", budget, &[]);
    assert_eq!(
        run.blocked,
        Some(Phase::Abort),
        "a child alive 4 s after its abort decision must cross the 4 s abort cap; the watchdog said \
         {}.\nstdout:\n{}\nstderr:\n{}",
        run.watchdog(),
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("CHILD-ABORT-STUCK-START"),
        "fixture: the child never ran its scenario -- stdout:\n{}",
        run.stdout
    );
}

const BLOCKED_BEFORE_ABORT_TEST: &str = "run_child_reports_a_child_blocked_before_its_abort_decision";

/// CONTROL, the second of the two. A child whose task panicked (one hook episode, as a detached
/// task's panic makes) and that then blocks without ever printing the abort decision must be
/// reported BLOCKED in the `Work` phase: neither the panic nor its hook opens the abort phase.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn run_child_reports_a_child_blocked_before_its_abort_decision() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("abort-blocked") {
        child_enter();
        child_mark("CHILD-ABORT-BLOCKED-START");
        let caught = catch_unwind::<_, ()>(|| panic_any(A6Marker(92)));
        child_mark(if caught.is_err() {
            "CHILD-ABORT-BLOCKED-PANICKED"
        } else {
            "CHILD-ABORT-BLOCKED-NO-PANIC"
        });
        park_forever();
    }
    let budget = Budget {
        work: Duration::from_secs(2),
        abort_cap: Duration::from_secs(4),
        ..Budget::DEFAULT
    };
    let run = run_child_with(BLOCKED_BEFORE_ABORT_TEST, "abort-blocked", budget, &[]);
    assert_eq!(
        run.blocked,
        Some(Phase::Work),
        "a child blocked before its abort decision must be BLOCKED in the work phase; the watchdog \
         said {}.\nstdout:\n{}\nstderr:\n{}",
        run.watchdog(),
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("CHILD-ABORT-BLOCKED-PANICKED"),
        "fixture: the child's panic never happened, so no hook episode preceded the block -- \
         stdout:\n{}",
        run.stdout
    );
    assert_eq!(run.abort, None, "the child printed no abort decision, so no time is the abort's");
    assert_eq!(run.child_hooks.len(), 1, "fixture: exactly one hook episode, the task's panic");
}

/// CONTROL. Every child is launched with both symbol-path variables REMOVED from its environment,
/// even when a caller passes them: a forced backtrace in a child symbolises from the local PDB and
/// never waits on a symbol server a runner's environment names.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: on Windows a `Command`'s environment keys are ordered by `CompareStringOrdinal`, a foreign call Miri does not support; the command serves only the child-process tests Miri skips. Runs natively."
)]
#[test]
fn run_child_launches_children_without_a_symbol_server_path() {
    let poisoned: Vec<(&str, &str)> =
        SYMBOL_PATH_ENV.iter().map(|key| (*key, "srv*https://symbols.invalid")).collect();
    let command = child_command(Path::new("a6-child"), "a6-test", "a6-scenario", &poisoned);
    for key in SYMBOL_PATH_ENV {
        let removed = command.get_envs().any(|(k, v)| k == OsStr::new(key) && v.is_none());
        assert!(
            removed,
            "`{key}` is not removed from the child's environment, so a forced backtrace in a child \
             may wait on the symbol server it names -- the child's explicit environment: {:?}",
            command.get_envs().collect::<Vec<_>>()
        );
    }
}

const DETACHED_TEST: &str = "detached_spawn_panic_does_not_block_pool_shutdown";
const DETACHED_MESSAGE: &str = "A6 detached task panic";

/// A detached `ThreadPool::spawn` task panics, then the pool is dropped. The process must END
/// within the deadline: either shutdown completes (`CHILD-POOL-DROPPED`), or the documented abort
/// fires — in which case the task's own message must be visible on stderr.
///
/// Red: the child is still alive at the deadline (shutdown blocked), the child did not run the
/// scenario at all, the detached task never started, or the child died without the panic text ever
/// reaching stderr (a silent death) or without the `E0201` abort line (an unexplained death).
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: gpu-free child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn detached_spawn_panic_does_not_block_pool_shutdown() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("detached") {
        // ── CHILD ──
        child_enter();
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let reached = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&reached);
        pool.spawn(move || {
            flag.store(true, Ordering::Release);
            panic!("{}", DETACHED_MESSAGE);
        });
        // Wait until the task has at least started, so the drop below races a panicking
        // worker rather than an idle one.
        if !wait_for(&reached) {
            child_mark("CHILD-TASK-NEVER-STARTED");
            return;
        }
        child_mark("CHILD-TASK-STARTED");
        drop(pool);
        child_mark("CHILD-POOL-DROPPED");
        return;
    }

    // ── PARENT ──
    let run = run_child(DETACHED_TEST, "detached");
    let Some(success) = run.success else {
        panic!(
            "WATCHDOG: {} -- after a detached task panicked, pool shutdown is BLOCKED (defect \
             A6).\nstdout:\n{}\nstderr:\n{}",
            run.watchdog(),
            run.stdout,
            run.stderr
        );
    };
    assert!(
        run.stdout.contains("running 1 test"),
        "the child must have RUN this test rather than filtering it away -- stdout:\n{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("CHILD-TASK-NEVER-STARTED"),
        "the detached task never started within {DEADLINE:?} -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    // Reached = the child saw the task's flag, or the child died and the task's own panic text is
    // on stderr. The documented abort can end the process between the task's flag store and the
    // child thread's marker, so a child that dies may never print `CHILD-TASK-STARTED`: CI run 4
    // (release, a 4-vCPU runner) failed this way, and so did 1 of 5 runs on the dev host under a
    // 4-CPU affinity mask with libtest's 16 threads (MEASURED 2026-10-10). The task prints that
    // text itself, after its flag store, so it is the same observation made by the task.
    let reached = run.stdout.contains("CHILD-TASK-STARTED")
        || (!success && run.stderr.contains(DETACHED_MESSAGE));
    assert!(
        reached,
        "the child must have reached the panicking detached task -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    if success {
        assert!(
            run.stdout.contains("CHILD-POOL-DROPPED"),
            "a child that exited cleanly must have completed the pool drop -- stdout:\n{}",
            run.stdout
        );
    } else {
        assert!(
            run.stderr.contains(DETACHED_MESSAGE),
            "the child died, and the detached task's panic text never reached stderr -- a \
             silent death.\nstdout:\n{}\nstderr:\n{}",
            run.stdout,
            run.stderr
        );
        // The documented abort must also SAY it is the abort: with `BOYKO_LOG` unset (the child
        // runs with it removed) the record is never constructed, and `abort_on_task_panic`'s
        // `NoConsumer` fallback line is the only trace of the decision. This is the observation
        // `untested_codes.txt` once said needed an out-of-process fixture; this child is one.
        let expected = format!("boyko-E{:04}", boyko_log::codes::E0201.number());
        assert!(
            run.stderr.lines().any(|l| l.starts_with(&expected)),
            "the child died without the `{expected}` abort line on stderr -- the abort decision \
             is invisible.\nstdout:\n{}\nstderr:\n{}",
            run.stdout,
            run.stderr
        );
        // The watchdog must have read that same line as the abort decision, or the time the OS
        // takes to end an aborting process is charged to the scope (ci-fix-2, F6).
        assert!(
            run.abort.is_some(),
            "the child printed `{expected}`, and the watchdog never read it as the abort decision: \
             {}.\nstderr:\n{}",
            run.watchdog(),
            run.stderr
        );
    }
}

const DOUBLE_TEST: &str = "owner_and_task_panicking_together_resume_on_owner_without_abort";

/// The owner's closure panics WHILE one of its tasks has also panicked. `std::thread::scope`
/// resumes the closure's panic in this case; here the scope's join runs during the closure's
/// unwind, and a `resume_unwind` from inside that `Drop` would be a panic during unwinding, which
/// Rust turns into a process abort.
///
/// Beyond the literal A6 brief (which names task panics), and kept because its failure mode is
/// the same class — a panic that ends the process instead of reaching the owner. The child
/// catches the panic around `install` and prints `CHILD-CAUGHT`.
///
/// Red: the child hangs (watchdog), dies without printing `CHILD-CAUGHT` (the abort), or returns
/// from `install` without any panic.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: gpu-free child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn owner_and_task_panicking_together_resume_on_owner_without_abort() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("double") {
        // ── CHILD ──
        child_enter();
        let pool = pool();
        let task_panicked = AtomicBool::new(false);
        let caught = catch_unwind(AssertUnwindSafe(|| {
            pool.install(|scope| {
                scope.spawn(|| {
                    task_panicked.store(true, Ordering::Release);
                    panic_any(A6Marker(41));
                });
                // Make the task's panic land before the owner's own, so the join below has a
                // payload to resume.
                if !wait_for(&task_panicked) {
                    child_mark("CHILD-TASK-NEVER-STARTED");
                    return;
                }
                panic_any(A6Marker(42));
            });
        }));
        match caught {
            Ok(()) => child_mark("CHILD-RETURNED"),
            Err(p) => child_mark(&format!("CHILD-CAUGHT {}", describe(&*p))),
        }
        return;
    }

    // ── PARENT ──
    let run = run_child(DOUBLE_TEST, "double");
    let Some(success) = run.success else {
        panic!(
            "WATCHDOG: {} -- after the owner and a task both panicked.\nstdout:\n{}\nstderr:\n{}",
            run.watchdog(),
            run.stdout,
            run.stderr
        );
    };
    assert!(
        run.stdout.contains("running 1 test"),
        "the child must have RUN this test rather than filtering it away -- stdout:\n{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("CHILD-TASK-NEVER-STARTED"),
        "the child's task never started, so the double-panic shape was never produced -- \
         stdout:\n{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("CHILD-RETURNED"),
        "`install` returned normally although both the owner and a task panicked -- stdout:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("CHILD-CAUGHT"),
        "the owner never received a panic: the child died (exit success = {success}) before its \
         `catch_unwind` around `install` returned -- a panic during the scope's unwinding join \
         aborted the process.\nstdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(success, "the child caught the panic but did not exit cleanly -- stderr:\n{}", run.stderr);
}

// ─────────────────────────────────────────────────────────────────────────────
// Design Rev 7 validation row P6 — a payload whose own `Drop` panics
// ─────────────────────────────────────────────────────────────────────────────

const BOMB_TEST: &str = "a_payload_that_panics_on_drop_does_not_block_the_scope";

/// What a [`SecondFault`]'s destructor panics with, so a leaked-vs-dropped second fault is legible
/// in the child's stderr.
const SECOND_FAULT_MESSAGE: &str = "A6 second-fault payload panicked while being dropped";

/// The payload a [`Bomb`]'s destructor panics WITH — and its own destructor panics too.
///
/// This is what gives mutation M9 (`discard_payload` DROPPING the second fault instead of leaking
/// it) a red at all. A `Bomb` whose destructor panicked with a plain `String` hands
/// `discard_payload` a second payload whose drop is harmless, and `forget` -> `drop` is then an
/// equivalent mutation: measured, with this destructor changed to `panic!("…")` and M9 applied,
/// this whole file reads 12 passed.
struct SecondFault(u32);

impl Drop for SecondFault {
    fn drop(&mut self) {
        panic!("{SECOND_FAULT_MESSAGE} ({})", self.0);
    }
}

/// A panic payload whose DESTRUCTOR panics (with a [`SecondFault`]).
///
/// `rust#86027`: a panic payload's `Drop` running on a panic path is a real shape, not a contrived
/// one, and `Commands::add<C: Command>` makes user-authored `Drop` glue reachable from this engine's
/// own hot path.
struct Bomb(u32);

impl Drop for Bomb {
    fn drop(&mut self) {
        panic_any(SecondFault(self.0));
    }
}

/// What a [`SimpleBomb`]'s destructor panics with.
const SIMPLE_BOMB_MESSAGE: &str = "A6 simple bomb payload panicked while being dropped";

/// A panic payload whose destructor panics with a plain message — a second fault whose own drop is
/// harmless. Stage 2a uses it so that M8's red is the one the design names: the loser's drop escapes
/// `capture_panic`, the worker backstop catches a `String` and aborts with `boyko-E0201`.
///
/// It also arms [`a_detached_task_whose_payload_panics_on_drop_still_aborts`]: a backstop that
/// drops the payload it caught prints [`SIMPLE_BOMB_MESSAGE`], one that forgets it does not.
///
/// History, because it explains why stage 2a exists separately from 2b: before finding D3 was
/// fixed, `worker::run_task` DROPPED the payload it caught, so under M8 a [`Bomb`] faulted a second
/// time inside the backstop, the worker thread died without aborting, and the scope hung. With the
/// backstop now forgetting its payload, M8 aborts with `boyko-E0201` for either payload.
struct SimpleBomb(u32);

impl Drop for SimpleBomb {
    fn drop(&mut self) {
        panic!("{SIMPLE_BOMB_MESSAGE} ({})", self.0);
    }
}

/// Two tasks panic with `make(i)`; one wins the first-wins CAS and reaches the owner, the other is
/// the LOSER whose payload `capture_panic` discards. Reports what the owner received.
fn child_loser_stage<P: Any + Send + 'static>(pool: &ThreadPool, label: &str, make: fn(u32) -> P) {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|scope| {
            for i in 0..2u32 {
                scope.spawn(move || {
                    child_mark(&format!("CHILD-{label}-LAUNCHED {i}"));
                    panic_any(make(i));
                });
            }
        });
    }));
    match caught {
        Ok(()) => child_mark(&format!("CHILD-{label}-NO-PAYLOAD")),
        Err(p) => {
            let is_ours = p.downcast_ref::<P>().is_some();
            // LEAKED on purpose: this payload panics on drop, so dropping it here would fault
            // inside the test harness and destroy the observation the parent is making.
            core::mem::forget(p);
            child_mark(&format!("CHILD-{label}-PAYLOAD {}", if is_ours { "original" } else { "other" }));
        }
    }
}

/// The parent-side read of one [`child_loser_stage`].
fn assert_loser_stage(run: &ChildRun, label: &str) {
    for i in 0..2 {
        assert!(
            run.stdout.contains(&format!("CHILD-{label}-LAUNCHED {i}")),
            "stage {label}: both drop-panicking tasks must have run, or the loser-payload path was \
             never entered -- stdout:\n{}",
            run.stdout
        );
    }
    assert!(
        !run.stdout.contains(&format!("CHILD-{label}-NO-PAYLOAD")),
        "stage {label}: the scope returned normally: a task's panic was swallowed -- stdout:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(&format!("CHILD-{label}-PAYLOAD original")),
        "stage {label}: the owner must receive one of the two ORIGINAL payloads -- stdout:\n{}",
        run.stdout
    );
}

/// Runs one owner-already-panicking scope with a `Bomb` task and reports, on the OWNER thread, how
/// many records of code `(class, number)` that thread emitted while the scope dropped.
///
/// The owner thread is the one `Scope::drop` runs on, and the already-panicking branch discards
/// the task's payload there — so the thread-local probe observes that discard deterministically. The
/// loser path of stage 2 is NOT observed this way: the loser's `capture_panic` runs on whichever
/// thread ran the loser, which may be a worker.
fn child_cleanup_stage(pool: &ThreadPool, label: &str, class: u8, number: u16) {
    boyko_log::probe::watch(class, number);
    let task_panicked = AtomicBool::new(false);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|scope| {
            scope.spawn(|| {
                child_mark(&format!("CHILD-{label}-BOMB-LAUNCHED"));
                task_panicked.store(true, Ordering::Release);
                panic_any(Bomb(7));
            });
            if !wait_for(&task_panicked) {
                child_mark("CHILD-TASK-NEVER-STARTED");
                return;
            }
            // The owner's own panic: the scope now drops inside a cleanup pad, holding the task's
            // `Bomb` payload.
            panic_any(A6Marker(77));
        });
    }));
    // Read the probe BEFORE anything else can emit on this thread.
    let records = boyko_log::probe::watched();
    let message = boyko_log::probe::last_message();
    match caught {
        Ok(()) => child_mark(&format!("CHILD-{label}-RETURNED")),
        Err(p) => {
            let verdict = match p.downcast_ref::<A6Marker>() {
                Some(A6Marker(77)) => "owner",
                _ if p.downcast_ref::<Bomb>().is_some() => "bomb",
                _ => "other",
            };
            // A `Bomb` must not be dropped here; anything else may be.
            if verdict == "bomb" {
                core::mem::forget(p);
            }
            child_mark(&format!("CHILD-{label}-CAUGHT {verdict}"));
        }
    }
    child_mark(&format!("CHILD-{label}-RECORDS {records}"));
    child_mark(&format!("CHILD-{label}-MESSAGE {message}"));
}

/// The parent-side read of one [`child_cleanup_stage`].
fn assert_cleanup_stage(run: &ChildRun, label: &str, message_part: &str) {
    assert!(
        run.stdout.contains(&format!("CHILD-{label}-BOMB-LAUNCHED")),
        "stage {label}: the `Bomb` task never ran, so the already-panicking discard was never \
         reached -- stdout:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(&format!("CHILD-{label}-CAUGHT owner")),
        "stage {label}: the OWNER's panic must reach the caller when the owner and a task panic \
         together (`std::thread::scope`'s rule); the task's payload is recorded and discarded -- \
         stdout:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(&format!("CHILD-{label}-RECORDS 1\n"))
            || run.stdout.contains(&format!("CHILD-{label}-RECORDS 1\r\n")),
        "stage {label}: the owner thread must emit exactly ONE record of this code while the scope \
         drops -- stdout:\n{}",
        run.stdout
    );
    let prefix = format!("CHILD-{label}-MESSAGE ");
    let line = run
        .stdout
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("stage {label}: no message line -- stdout:\n{}", run.stdout));
    assert!(
        line.contains(message_part),
        "stage {label}: the record must say {message_part:?}; it said: {line}"
    );
}

/// P6 — a payload whose `Drop` panics does not block the scope and does not kill the process.
///
/// Child process, because every failure mode here is process death: `worker::run_task`'s abort if
/// a loser's drop escapes `capture_panic`, and Rust's own `panic in a destructor during cleanup`
/// abort if a fault escapes a destructor that runs during the owner's unwind.
///
/// The child runs three stages on ONE pool:
///
/// 1. **Owner already panicking** (twice, once per code): a task panics with a `Bomb`, then the
///    owner panics, so `Scope::drop` discards the `Bomb` inside a cleanup pad. Its drop faults with
///    a `SecondFault`, which `discard_payload` must LEAK. The owner thread's probe must see one
///    `E0202` record naming the reason on the first run, and one `E0203` record on the second.
/// 2. **Loser payload**, twice: two tasks panic with a drop-panicking payload; one wins the
///    first-wins CAS and reaches the owner (the child `mem::forget`s it); the other goes through
///    `capture_panic`'s discard. First with a [`SimpleBomb`] (a plain second fault), then with a
///    [`Bomb`] (a second fault that faults again).
/// 3. A later scope on the same pool runs 64/64, and the child exits 0.
///
/// Red: the child hangs (watchdog); stderr carries `panic in a destructor during cleanup` (M9 —
/// the second fault DROPPED inside the cleanup pad; also M7, the re-raise inside it); stderr
/// carries `boyko-E0201` (M8 — the loser's drop escaped `capture_panic` into the worker backstop,
/// which cannot tell a scoped task from a fire-and-forget one); a stage marker, record count or
/// reason is missing; the later wave does not complete; or the child exits non-zero.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: gpu-free child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn a_payload_that_panics_on_drop_does_not_block_the_scope() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("bomb") {
        // ── CHILD ──
        child_enter();
        let pool = pool();
        // Raise the pool target's ceiling, or the records below are never constructed and the
        // probe counts zero for the wrong reason (`BOYKO_LOG` is unset in the child).
        boyko_log::probe::arm::<boyko_log::Threadpool>();

        // Stage 1 — the owner is already unwinding when the scope drops.
        child_cleanup_stage(&pool, "E0202", b'E', boyko_log::codes::E0202.number());
        child_cleanup_stage(&pool, "E0203", b'E', boyko_log::codes::E0203.number());

        // Stage 2a — the loser payload, with a PLAIN second fault. First, so that M8 (the loser's
        // drop escaping `capture_panic`) is observed as the design states it: the worker backstop
        // catches a `String` and aborts with `boyko-E0201`.
        child_loser_stage(&pool, "LOSER-SIMPLE", SimpleBomb);
        // Stage 2b — the loser payload, with a second fault that panics on drop too, on whichever
        // thread ran the loser (usually a worker).
        child_loser_stage(&pool, "LOSER-DOUBLE", Bomb);

        // Stage 3 — the pool must still be usable.
        let ran = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&ran);
        pool.install(|scope| {
            for _ in 0..64 {
                let counter = Arc::clone(&counter);
                scope.spawn(move || {
                    busy(4);
                    counter.fetch_add(1, Ordering::AcqRel);
                });
            }
        });
        child_mark(&format!("CHILD-WAVE {}", ran.load(Ordering::Acquire)));
        drop(pool);
        child_mark("CHILD-DONE");
        return;
    }

    // ── PARENT ──
    let run = run_child(BOMB_TEST, "bomb");
    let Some(success) = run.success else {
        panic!(
            "WATCHDOG: {} -- after drop-panicking payloads, the scope is BLOCKED.\nstdout:\n{}\n\
             stderr:\n{}",
            run.watchdog(),
            run.stdout,
            run.stderr
        );
    };
    assert!(
        run.stdout.contains("running 1 test"),
        "the child must have RUN this test rather than filtering it away -- stdout:\n{}",
        run.stdout
    );
    // The two abort signatures FIRST: each kills the child before the stage markers below, and a
    // missing marker would otherwise be reported instead of the cause.
    assert!(
        !run.stderr.contains("panic in a destructor during cleanup"),
        "a panic escaped a destructor that was running during the owner's unwind: either \
         `discard_payload` DROPPED the second fault instead of leaking it (M9), or `Scope::drop` \
         re-raised the task's payload inside the cleanup pad (M7) -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        !run.stderr.contains("boyko-E0201"),
        "a SCOPED task's payload reached the detached-task abort backstop: a discarded payload's \
         drop escaped `capture_panic` (M8) -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        !run.stdout.contains("CHILD-TASK-NEVER-STARTED"),
        "the child's `Bomb` task never started, so no discard was produced -- stdout:\n{}",
        run.stdout
    );
    assert_cleanup_stage(&run, "E0202", "the owner thread was already panicking");
    assert_cleanup_stage(&run, "E0203", "leaked rather than dropped again");
    assert_loser_stage(&run, "LOSER-SIMPLE");
    assert_loser_stage(&run, "LOSER-DOUBLE");
    assert!(
        run.stdout.contains("CHILD-WAVE 64"),
        "stage 3: the pool must still run a later scope's 64 tasks after the discarded payloads \
         faulted -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("CHILD-DONE"),
        "the child must reach its end -- stdout:\n{}",
        run.stdout
    );
    assert!(
        success,
        "the child must exit 0: a discarded payload that panics on drop is contained, not fatal \
         -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Finding D3 — the detached-task backstop must not DROP the payload it caught
// ─────────────────────────────────────────────────────────────────────────────

const DETACHED_BOMB_TEST: &str = "a_detached_task_whose_payload_panics_on_drop_still_aborts";

/// Thread-name prefix of the D3 children's pool, so "which thread ran the bomb" is read off a name
/// this file chose rather than off the pool's default.
const D3_WORKER_PREFIX: &str = "a6-d3-worker";

/// How long a D3 child waits for an observation that only a BROKEN backstop lets it make: a later
/// task running on the lone worker, or the lone worker's blocking task being released. Shorter than
/// [`DEADLINE`], so the child reports the defect itself instead of being killed by the parent and
/// reported as a hang.
const D3_CHILD_WAIT: Duration = Duration::from_secs(3);

/// [`wait_for`] with a caller-chosen bound.
fn wait_for_within(flag: &AtomicBool, limit: Duration) -> bool {
    let start = Instant::now();
    while !flag.load(Ordering::Acquire) {
        if start.elapsed() >= limit {
            return false;
        }
        std::thread::yield_now();
    }
    true
}

/// The detached task both D3 children spawn: it reports which kind of thread is running it, then
/// panics with a payload whose destructor panics.
fn d3_bomb(n: u32) {
    let on_worker = std::thread::current()
        .name()
        .is_some_and(|name| name.starts_with(D3_WORKER_PREFIX));
    child_mark(&format!("CHILD-BOMB-ON-WORKER {}", if on_worker { "yes" } else { "no" }));
    panic_any(SimpleBomb(n));
}

/// A one-worker pool with [`D3_WORKER_PREFIX`] names.
fn d3_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new()
        .num_threads(1)
        .thread_name_prefix(D3_WORKER_PREFIX)
        .build()
}

/// Child half, scenario `d3-worker`: the lone WORKER runs the detached bomb.
fn d3_child_on_worker() {
    child_enter();
    let pool = d3_pool();
    let bomb_started = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&bomb_started);
    pool.spawn(move || {
        flag.store(true, Ordering::Release);
        d3_bomb(31);
    });
    if !wait_for(&bomb_started) {
        child_mark("CHILD-TASK-NEVER-STARTED");
        return;
    }
    // Only reached before the abort lands, or when it never does. A backstop that let the
    // payload's drop unwind has killed the lone worker, and this task then never runs.
    let later = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&later);
    pool.spawn(move || flag.store(true, Ordering::Release));
    if wait_for_within(&later, D3_CHILD_WAIT) {
        child_mark("CHILD-LATER-TASK-RAN");
    } else {
        child_mark("CHILD-LATER-TASK-NEVER-RAN");
    }
    // Never shut the pool down here: on the defective path its only worker is dead, and what the
    // shutdown join does with that is not this test's observation.
    core::mem::forget(pool);
}

/// Child half, scenario `d3-join`: the lone worker is held inside a scoped task, so the detached
/// bomb can only be run by the scope's own join, on the thread that called `install`.
fn d3_child_on_join() {
    child_enter();
    let pool = d3_pool();
    let blocker_started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let blocker_done = Arc::new(AtomicBool::new(false));
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|scope| {
            let started = Arc::clone(&blocker_started);
            let released = Arc::clone(&release);
            let done = Arc::clone(&blocker_done);
            scope.spawn(move || {
                started.store(true, Ordering::Release);
                // Bounded: if the join never takes the bomb, the worker does after this, and the
                // parent reports that the fixture missed the join path.
                let _ = wait_for_within(&released, D3_CHILD_WAIT);
                done.store(true, Ordering::Release);
            });
            if !wait_for(&blocker_started) {
                child_mark("CHILD-TASK-NEVER-STARTED");
                release.store(true, Ordering::Release);
                return;
            }
            // From a thread with no lane in this pool the task lands on the global injector, which
            // the external joiner probes before it looks anywhere else.
            pool.spawn(|| d3_bomb(32));
        });
    }));
    match caught {
        Ok(()) => child_mark("CHILD-JOIN-RETURNED"),
        Err(_) if !blocker_done.load(Ordering::Acquire) => {
            child_mark("CHILD-JOIN-ABANDONED");
            // The abandoned scope's task still holds a pointer into the unwound frame; end the
            // process before it is released and writes through it.
            std::process::exit(0);
        }
        Err(_) => child_mark("CHILD-JOIN-PANICKED"),
    }
}

/// Parent-side read of one D3 child.
fn assert_d3_child(run: &ChildRun, scenario: &str, on_worker: bool, survived: &[(&str, &str)]) {
    let Some(success) = run.success else {
        panic!(
            "{scenario}: WATCHDOG: {} -- after a detached task panicked with a drop-panicking \
             payload.\nstdout:\n{}\nstderr:\n{}",
            run.watchdog(),
            run.stdout,
            run.stderr
        );
    };
    assert!(
        run.stdout.contains("running 1 test"),
        "{scenario}: the child must have RUN this test rather than filtering it away -- stdout:\n{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("CHILD-TASK-NEVER-STARTED"),
        "{scenario}: the child's first task never started -- stdout:\n{}",
        run.stdout
    );
    let reached = format!("CHILD-BOMB-ON-WORKER {}", if on_worker { "yes" } else { "no" });
    assert!(
        run.stdout.contains(&reached),
        "{scenario}: fixture: the bomb must have run on {} -- expected `{reached}`, so the \
         `run_task` caller under test was never reached.\nstdout:\n{}\nstderr:\n{}",
        if on_worker { "the pool's worker" } else { "the thread joining the scope" },
        run.stdout,
        run.stderr
    );
    for (marker, meaning) in survived {
        assert!(
            !run.stdout.contains(marker),
            "{scenario}: {meaning} (`{marker}`).\nstdout:\n{}\nstderr:\n{}",
            run.stdout,
            run.stderr
        );
    }
    assert!(
        !run.stderr.contains(SIMPLE_BOMB_MESSAGE),
        "{scenario}: the detached-task backstop DROPPED the payload it caught: the payload's \
         destructor ran, and its panic is on stderr (finding D3).\nstdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        !success,
        "{scenario}: the process survived a detached task's panic; the documented policy is an \
         abort.\nstdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    let expected = format!("boyko-E{:04}", boyko_log::codes::E0201.number());
    assert!(
        run.stderr.lines().any(|l| l.starts_with(&expected)),
        "{scenario}: the child died without the `{expected}` abort line on stderr -- not the \
         documented abort.\nstdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    // The watchdog must have read that same line as the abort decision, or the time the OS takes
    // to end an aborting process is charged to the scope (ci-fix-2, F6).
    assert!(
        run.abort.is_some(),
        "{scenario}: the child printed `{expected}`, and the watchdog never read it as the abort \
         decision: {}.\nstderr:\n{}",
        run.watchdog(),
        run.stderr
    );
}

/// Finding D3 — a DETACHED task panics with a payload whose destructor panics. `worker::run_task`
/// must abort with `boyko-E0201` without ever dropping that payload, on both of its callers.
///
/// Before the fix, `run_task` tested `catch_unwind(..).is_err()`: the payload was a temporary of
/// the `if` condition and was dropped before the abort, so its destructor's panic unwound out of
/// `run_task` and reached exactly what the backstop exists to prevent.
///
/// Two children, one per caller:
///
/// - `d3-worker` — the lone worker runs the bomb. Defective: the worker thread dies, no abort, and
///   a later task never runs (`CHILD-LATER-TASK-NEVER-RAN`): the pool has silently shrunk to zero.
/// - `d3-join` — the lone worker is held inside a scoped task, and the scope's external joiner
///   steals the bomb from the global injector. Defective: the panic unwinds out of `Scope::drop`,
///   and `install` returns by panic while its scoped task is still running
///   (`CHILD-JOIN-ABANDONED`) — the abandoned join that `run_task`'s doc calls a use-after-free.
///
/// Red: either child hangs; the bomb ran on the wrong kind of thread (the fixture missed its
/// caller); a survival marker is printed (`CHILD-LATER-TASK-*`, `CHILD-JOIN-*`); the payload's
/// destructor message is on stderr (the payload was dropped); the child exits 0; or stderr lacks
/// the `boyko-E0201` line.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: gpu-free child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn a_detached_task_whose_payload_panics_on_drop_still_aborts() {
    match std::env::var(CHILD_ENV).as_deref() {
        Ok("d3-worker") => return d3_child_on_worker(),
        Ok("d3-join") => return d3_child_on_join(),
        _ => {}
    }

    // ── PARENT ──
    let run = run_child(DETACHED_BOMB_TEST, "d3-worker");
    assert_d3_child(
        &run,
        "d3-worker",
        true,
        &[
            (
                "CHILD-LATER-TASK-NEVER-RAN",
                "the lone worker is DEAD: the payload's drop unwound out of `run_task` and ended \
                 the worker thread instead of the process -- the pool silently shrank to zero \
                 workers (finding D3)",
            ),
            (
                "CHILD-LATER-TASK-RAN",
                "the worker survived a detached task's panic and ran another task: no abort",
            ),
        ],
    );

    let run = run_child(DETACHED_BOMB_TEST, "d3-join");
    assert_d3_child(
        &run,
        "d3-join",
        false,
        &[
            (
                "CHILD-JOIN-ABANDONED",
                "the payload's drop unwound out of the scope's join while a scoped task was still \
                 running -- the join was ABANDONED (finding D3)",
            ),
            (
                "CHILD-JOIN-RETURNED",
                "`install` returned normally after its join ran a panicking detached task: no abort",
            ),
            (
                "CHILD-JOIN-PANICKED",
                "`install` panicked after its scoped task had finished: no abort",
            ),
        ],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Review finding W1 — the re-raise decision belongs to the scope, not to the thread
// ─────────────────────────────────────────────────────────────────────────────

const W1_TEST: &str = "a_scope_closed_during_another_scopes_unwind_still_resumes_its_task_panic";

/// The payload the OUTER owner panics with; it must be what the caller catches.
const W1_OUTER: u32 = 60;

/// The payload the INNER scope's task panics with; the inner scope must resume it.
const W1_INNER: u32 = 61;

/// Child half, scenario `w1`.
///
/// The lone worker is held by a detached task, so the only thread that can run the outer scope's
/// task is the owner itself, from inside `Scope::drop`'s join — and that join runs while the outer
/// closure's panic is unwinding. The task opens and closes a scope of its OWN whose task panics.
/// That inner scope's closure returned normally, so its owner is not panicking even though
/// `std::thread::panicking()` is true on the thread.
fn w1_child() {
    child_enter();
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    // Raise the pool target's ceiling, or the E0202 record is never constructed and the probe
    // counts zero for the wrong reason (`BOYKO_LOG` is unset in the child).
    boyko_log::probe::arm::<boyko_log::Threadpool>();

    let hold = Arc::new(AtomicBool::new(true));
    let held = Arc::new(AtomicBool::new(false));
    {
        let hold = Arc::clone(&hold);
        let held = Arc::clone(&held);
        pool.spawn(move || {
            held.store(true, Ordering::Release);
            while hold.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        });
    }
    if !wait_for(&held) {
        child_mark("CHILD-TASK-NEVER-STARTED");
        return;
    }
    child_mark("CHILD-W1-WORKER-HELD");

    let owner = std::thread::current().id();
    let reached = AtomicBool::new(false);
    boyko_log::probe::watch(b'E', boyko_log::codes::E0202.number());
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pool.install(|outer| {
            outer.spawn(|| {
                // The shape under test, printed so the parent can refuse a run that missed it.
                let inline = std::thread::current().id() == owner;
                let unwinding = std::thread::panicking();
                child_mark(&format!("CHILD-W1-TASK inline={inline} unwinding={unwinding}"));
                let inner = catch_unwind(AssertUnwindSafe(|| {
                    pool.scope(|scope| {
                        scope.spawn(|| panic_any(A6Marker(W1_INNER)));
                    });
                }));
                match inner {
                    Ok(()) => child_mark("CHILD-W1-INNER returned"),
                    Err(p) => {
                        child_mark(&format!("CHILD-W1-INNER panicked {}", describe(&*p)));
                        // Keep the task's own panic policy in the picture: the payload goes on to
                        // the outer scope, exactly as it would without this observation.
                        std::panic::resume_unwind(p);
                    }
                }
                reached.store(true, Ordering::Release);
            });
            panic_any(A6Marker(W1_OUTER));
        });
    }));
    // Read the probe BEFORE anything else can emit on this thread.
    let records = boyko_log::probe::watched();
    let message = boyko_log::probe::last_message();
    hold.store(false, Ordering::Release);

    match caught {
        Ok(()) => child_mark("CHILD-W1-CAUGHT nothing"),
        Err(p) => child_mark(&format!("CHILD-W1-CAUGHT {}", describe(&*p))),
    }
    child_mark(&format!("CHILD-W1-REACHED {}", reached.load(Ordering::Acquire)));
    child_mark(&format!("CHILD-W1-RECORDS {records}"));
    child_mark(&format!("CHILD-W1-MESSAGE {message}"));
    drop(pool);
    child_mark("CHILD-W1-DONE");
}

/// Review finding W1 — `Scope::drop` must decide "re-raise or discard" from whether ITS OWN closure
/// returned, not from `std::thread::panicking()`.
///
/// The child (see [`w1_child`]) runs a scope task inline on the owner during the owner's unwind, and
/// that task closes a scope whose own task panicked. The inner scope's owner never panicked, so the
/// inner `pool.scope` must panic with the inner payload (`CHILD-W1-INNER panicked A6Marker(61)`) and
/// the code after it must not run. The outer scope's owner IS panicking, so the outer scope discards
/// the payload the task passed on, with one `E0202` record on the owner thread, and the caller
/// catches the owner's own `A6Marker(60)`.
///
/// Red: the child hangs; stderr carries `panic in a destructor during cleanup` (the outer scope
/// re-raised inside its cleanup pad, M7); the worker was never held or the task did not run inline
/// during the unwind (the fixture missed the shape, so nothing below would mean anything); the inner
/// scope RETURNED normally and the code after it ran (a thread-wide predicate: the defect W1 names);
/// the caller caught anything but `A6Marker(60)`; the owner thread emitted other than one `E0202`
/// record, or the record does not name the owner-already-panicking reason; or the child did not exit
/// 0.
#[cfg_attr(
    miri,
    ignore = "miri-unsupported: child process: re-executes this test binary, which Miri does not support. Runs natively."
)]
#[test]
fn a_scope_closed_during_another_scopes_unwind_still_resumes_its_task_panic() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("w1") {
        return w1_child();
    }

    // ── PARENT ──
    let run = run_child(W1_TEST, "w1");
    let Some(success) = run.success else {
        panic!(
            "WATCHDOG: the W1 child is {}.\nstdout:\n{}\nstderr:\n{}",
            run.watchdog(),
            run.stdout,
            run.stderr
        );
    };
    assert!(
        run.stdout.contains("running 1 test"),
        "the child must have RUN this test rather than filtering it away -- stdout:\n{}",
        run.stdout
    );
    assert!(
        !run.stderr.contains("panic in a destructor during cleanup"),
        "a panic escaped a destructor running during the owner's unwind: the OUTER scope re-raised \
         its task's payload inside its cleanup pad (M7) -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("CHILD-W1-WORKER-HELD"),
        "fixture: the lone worker was never held, so the outer task need not run inline -- \
         stdout:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("CHILD-W1-TASK inline=true unwinding=true"),
        "fixture: the outer scope's task must run ON THE OWNER while the owner's panic unwinds, or \
         the thread-wide predicate and the per-scope one agree and this test proves nothing -- \
         stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    let inner = format!("CHILD-W1-INNER panicked A6Marker({W1_INNER})");
    assert!(
        run.stdout.contains(&inner),
        "the inner scope's owner never panicked, so its task's panic must be resumed on it \
         (`{inner}`). A scope that returned normally here discarded the payload because the THREAD \
         was unwinding for an outer frame's reason (review finding W1) -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("CHILD-W1-REACHED false"),
        "the code after the inner scope ran although that scope's task panicked -- stdout:\n{}",
        run.stdout
    );
    let caught = format!("CHILD-W1-CAUGHT A6Marker({W1_OUTER})");
    assert!(
        run.stdout.contains(&caught),
        "the caller must catch the owner's own panic (`{caught}`) -- stdout:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("CHILD-W1-RECORDS 1\n") || run.stdout.contains("CHILD-W1-RECORDS 1\r\n"),
        "the owner thread must emit exactly ONE E0202 record: the outer scope's discard of the \
         payload its task passed on -- stdout:\n{}",
        run.stdout
    );
    let line = run
        .stdout
        .lines()
        .find(|l| l.starts_with("CHILD-W1-MESSAGE "))
        .unwrap_or_else(|| panic!("no W1 message line -- stdout:\n{}", run.stdout));
    assert!(
        line.contains("the owner thread was already panicking"),
        "the E0202 record must name the owner-already-panicking reason; it said: {line}"
    );
    assert!(
        run.stdout.contains("CHILD-W1-DONE"),
        "the child must reach its end -- stdout:\n{}",
        run.stdout
    );
    assert!(
        success,
        "the child must exit 0 -- stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
}
