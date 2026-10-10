//! KE16 App-12 — what a short `park_timeout` **actually** waits, with and without
//! [`TimerResolutionGuard`].
//!
//! # This is a RECORDER, not a gate
//!
//! The number it exists to produce is machine-dependent in both directions. Unguarded, the expiry
//! of a 50 µs park is whatever the system timer period happens to be at that instant — ~15.6 ms on
//! an idle box, but ~1 ms the moment any *other* process on the machine holds a
//! `timeBeginPeriod(1)` of its own, because that request is global and this process cannot see it.
//! Guarded, it is ~1 ms plus scheduling jitter. Pinning either to an absolute microsecond figure
//! would be a gate that fails on a busy laptop and passes on a quiet server while measuring the
//! same engine, which is the definition of flaky.
//!
//! So the assertion is the weakest one that still catches the defect this rung could plausibly
//! introduce — **the guard must not make timed waits worse** — and the numbers are printed for a
//! human to read under `--nocapture`. A regression that mattered (the guard raising nothing, or
//! `Drop` releasing a period it never took and leaving the machine's rate raised) shows up in the
//! printed table, and the third configuration below is what makes the second of those visible at
//! all.
//!
//! # Why three configurations and not two
//!
//! `unguarded → guarded → restored` rather than `unguarded → guarded`. The third pass re-measures
//! after the guard has dropped, which is the only observation in this file that says anything about
//! the **balance** property the whole type exists for: a guard whose `Drop` did nothing would leave
//! the third table row looking like the second, and a reader comparing rows one and three sees it
//! immediately. It is recorded rather than asserted because a *different* process may raise or drop
//! its own request between the second pass and the third, and this one has no way to tell.
//!
//! # On non-Windows targets
//!
//! The guard holds nothing there by construction (POSIX timed waits already carry nanosecond
//! deadlines), so all three configurations measure the same quantity and the assertion degenerates
//! to a jitter check. That is stated rather than `cfg`-ed away: the `granted` column prints `false`
//! and the reader can see which of the two things they are looking at.
//!
//! # Run
//!
//! ```text
//! cargo test -p boyko-app --test app12_timer_resolution -- --nocapture --test-threads=1
//! ```

use std::thread;
use std::time::{Duration, Instant};

use boyko_app::TimerResolutionGuard;

/// Timed samples per configuration per timeout.
///
/// The brief's floor is 200. A median over 200 samples of a quantised quantity is stable to well
/// under the quantum itself, which is all this comparison needs; more would buy resolution the
/// assertion does not use and would cost wall clock linearly, since every sample IS a sleep.
const REPS: usize = 200;

/// Samples discarded before each timed run.
///
/// Two jobs, both measured rather than assumed: they absorb the first-touch cost of the sampling
/// path (page faults on the sample buffer, the initial `WaitOnAddress` entry), and they consume a
/// stray unpark token if one were ever pending on this thread — a token makes `park_timeout` return
/// instantly, which would bias a median toward zero and look like a spectacular improvement.
const WARMUP: usize = 20;

/// The threadpool's lost-wakeup backstop (`Scope`'s pre-park timeout).
const POOL_BACKSTOP: Duration = Duration::from_micros(50);

/// The ECS dispatcher's inter-round backstop (`Schedule`'s `PARK_TIMEOUT`).
const DISPATCHER_BACKSTOP: Duration = Duration::from_micros(100);

/// How much worse the guarded median may be before the test fails, in microseconds.
///
/// A bare `guarded <= unguarded` is not safe, and the reason is the same global-request property
/// the module doc opens with: when another process already holds a 1 ms period, both passes measure
/// the *same* ~1 ms quantum and the ordering between their medians is decided by scheduling noise —
/// a coin flip, run twice per invocation. The slack is half the finest quantum Windows can be asked
/// for, so it admits that noise and still refuses the failure the assertion is for: a guard that
/// raised nothing leaves the guarded pass at the unguarded quantum, and on any machine where that
/// distinction exists at all the gap is milliseconds, not hundreds of microseconds.
const REGRESSION_SLACK_US: u128 = 500;

// ── Why the assertion is not stronger, MEASURED rather than predicted (2026-09-02) ───────────────
//
// The obvious stronger gate — "guarded must be at least 2x better" — was refuted by this file's own
// first run on the bench box. Four configurations were recorded while landing App-12:
//
//   idle box, run 2      50 us: 15333 -> 1041 us   (0.07x)   100 us: 15278 -> 1027 us   (0.07x)
//   all 16 cores loaded  50 us: 20221 -> 9527 us   (0.47x)   100 us: 19385 -> 10851 us  (0.56x)
//   idle box, run 1      50 us: 15371 -> 15368 us  (1.00x)   100 us: 15364 -> 15372 us  (1.00x)
//
// The third row is the one that decides the design. `timeBeginPeriod` answered TIMERR_NOERROR, the
// guard reported `granted=1ms`, and the expiry did not move — min 15095, max 16369, a spread too
// tight to be contention. Both hypotheses available were tested and neither explains it: CPU
// saturation produces the WIDE second row, not the third, and a `GetProcessInformation`
// (`ProcessPowerThrottling`) read on the same box returns `state_mask=0x0`, so Windows 11's
// documented `PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION` is not applied to this process.
// Four further acquire/release cycles in a standalone probe all worked (0.12x-0.46x).
//
// So a real machine can grant the request and deliver nothing, for a reason not yet identified, and
// a gate asserting an improvement would have been RED on correct code the first time it ran. The
// engine is unharmed in that state — it is exactly the pre-App-12 behaviour — which is why the
// weak assertion is the right one and the table is the product.

/// Median / min / max of one configuration at one timeout, in microseconds.
struct Expiry {
    median_us: u128,
    min_us: u128,
    max_us: u128,
}

/// Measures what `park_timeout(asked)` really waits, `REPS` times.
///
/// Sleeping rather than spinning is the correct instrument here: the quantity under test IS the
/// expiry of a sleep, so a spin would measure the thing that is not being asked about.
fn measure(asked: Duration) -> Expiry {
    for _ in 0..WARMUP {
        thread::park_timeout(asked);
    }

    let mut samples: Vec<u128> = Vec::with_capacity(REPS);
    for _ in 0..REPS {
        let started = Instant::now();
        thread::park_timeout(asked);
        samples.push(started.elapsed().as_micros());
    }
    samples.sort_unstable();

    Expiry {
        median_us: samples[samples.len() / 2],
        min_us: samples[0],
        max_us: samples[samples.len() - 1],
    }
}

/// One `unguarded | guarded | restored` pass over both engine backstops.
fn table(label: &str, granted: Option<u32>) -> (Expiry, Expiry) {
    let pool = measure(POOL_BACKSTOP);
    let dispatcher = measure(DISPATCHER_BACKSTOP);
    println!(
        "  {label:<10} granted={:<7} 50us: median {:>6} us  [min {:>6}, max {:>6}]   \
         100us: median {:>6} us  [min {:>6}, max {:>6}]",
        match granted {
            Some(ms) => format!("{ms}ms"),
            None => "no".to_string(),
        },
        pool.median_us,
        pool.min_us,
        pool.max_us,
        dispatcher.median_us,
        dispatcher.min_us,
        dispatcher.max_us,
    );
    (pool, dispatcher)
}

/// Records the real expiry of the engine's two `park_timeout` backstops with and without the
/// process timer-resolution guard, and asserts only that the guard does not make them worse.
#[test]
#[ignore = "slow: every sample IS a sleep, so the wall clock is the measurement — 1200 parks at \
            the 1-15.6 ms quantum under test. MEASURED on the bench box: 12.6 s idle, 22.1 s with \
            all 16 cores loaded. Run it with `-- --ignored --nocapture --test-threads=1`; the \
            thread cap is for fidelity, not correctness, since a parallel test perturbs the very \
            quantity being timed. The BALANCE contract this file also covers is checked by \
            `guard_holds_exactly_what_it_acquired_and_nests`, which is fast and stays live."]
fn park_timeout_expiry_with_and_without_raised_timer_resolution() {
    println!(
        "\nKE16 App-12 — park_timeout expiry, {REPS} samples per cell (+{WARMUP} discarded)\n"
    );

    let (unguarded_pool, unguarded_dispatcher) = table("unguarded", None);

    let (guarded_pool, guarded_dispatcher) = {
        let guard = TimerResolutionGuard::new_1ms();
        table("guarded", guard.held_period_ms())
    };

    // The guard is dropped: this row is the balance evidence, not a gate. See the module doc.
    let (restored_pool, restored_dispatcher) = table("restored", None);

    println!(
        "\n  ratio guarded/unguarded — 50us: {:.2}x   100us: {:.2}x",
        guarded_pool.median_us as f64 / unguarded_pool.median_us.max(1) as f64,
        guarded_dispatcher.median_us as f64 / unguarded_dispatcher.median_us.max(1) as f64,
    );
    println!(
        "  ratio restored/unguarded — 50us: {:.2}x   100us: {:.2}x\n",
        restored_pool.median_us as f64 / unguarded_pool.median_us.max(1) as f64,
        restored_dispatcher.median_us as f64 / unguarded_dispatcher.median_us.max(1) as f64,
    );

    assert!(
        guarded_pool.median_us <= unguarded_pool.median_us + REGRESSION_SLACK_US,
        "the guard made the {POOL_BACKSTOP:?} backstop WORSE: {} us guarded vs {} us unguarded \
         (slack {REGRESSION_SLACK_US} us)",
        guarded_pool.median_us,
        unguarded_pool.median_us,
    );
    assert!(
        guarded_dispatcher.median_us <= unguarded_dispatcher.median_us + REGRESSION_SLACK_US,
        "the guard made the {DISPATCHER_BACKSTOP:?} backstop WORSE: {} us guarded vs {} us \
         unguarded (slack {REGRESSION_SLACK_US} us)",
        guarded_dispatcher.median_us,
        unguarded_dispatcher.median_us,
    );
}

/// A guard that holds nothing must not release anything, and a guard that holds a period must
/// release **that** period — the two halves of the balance contract, checked at the type's own
/// surface rather than through the timer.
///
/// Fast and device-free, so it stays live in the default gate even if the recorder above ever does
/// not: the imbalance bug it covers is silent, machine-wide and survives process exit only by
/// luck, which is exactly the kind that must not depend on someone remembering to pass `--ignored`.
#[test]
fn guard_holds_exactly_what_it_acquired_and_nests() {
    let outer = TimerResolutionGuard::new_1ms();
    assert_eq!(
        outer.granted(),
        outer.held_period_ms().is_some(),
        "granted() and held_period_ms() must agree — they are one fact with two spellings"
    );
    if let Some(ms) = outer.held_period_ms() {
        assert_eq!(ms, 1, "new_1ms must record the period it actually asked for");
    }

    // Nesting is legal because the OS refcounts the requests. Nothing inside this process can read
    // that refcount, so what is checkable here is the half that lives in Rust: the type must not
    // have grown a global "already raised" flag, which would make this second acquisition report a
    // different period from the first and would make its `Drop` release the FIRST guard's hold.
    let outer_hold = outer.held_period_ms();
    {
        let inner = TimerResolutionGuard::new_1ms();
        assert_eq!(
            inner.held_period_ms(),
            outer_hold,
            "two guards asking for the same period must reach the same answer"
        );
    }
    assert_eq!(
        outer.held_period_ms(),
        outer_hold,
        "the inner guard's Drop must not have touched the outer guard's record of its own hold"
    );
}
