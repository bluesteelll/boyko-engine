//! KE16 base items that ship under every configuration: the tournament
//! WITNESS (`KE16-DESIGN.md` §4) and App-11's `ThreadPool::parked_mask`
//! (`KE16-DESIGN-APP.md` §10).
//!
//! ## Why the witness needs a test of its own
//!
//! The witness exists so that "the `--features` line did not take" cannot
//! silently produce a number (`KE16-DESIGN-MEASUREMENT.md` §5 item 3). That
//! guarantee has two halves, and `KE16_EXPECT` only checks the second one:
//!
//! 1. the string must MOVE when a feature is enabled — a witness that reads
//!    `a0+b0+w0+c0` in every build would certify every run and forbid nothing;
//! 2. the string must match what the runner asked for.
//!
//! Half 1 is what this file pins, by deriving the expected variant a SECOND
//! time and by a different mechanism: `lib.rs` selects its consts with `#[cfg]`
//! attribute arms (a compile-time item choice), while the functions below are
//! ordinary `if` chains over `cfg!` booleans. A cfg arm whose condition is
//! wrong — the trap the implied features `ke16-a5 = ["ke16-a2"]` and
//! `ke16-w-fanout = ["ke16-c-batch"]` set, where the plain arm must exclude the
//! implying feature — produces two texts that disagree, and the assertion names
//! both.
//!
//! Run under every feature of the matrix; the file is compiled into every
//! configuration and asserts against the one it was built with.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{
    KE16_A, KE16_B, KE16_C, KE16_W, ThreadPool, ThreadPoolBuilder, ke16_check_expected_variant,
    ke16_variant,
};

/// The A token this binary's features imply. `ke16-a5` is tested FIRST because
/// it enables `ke16-a2` through Cargo, so "a2 is on" does not mean "A2 is the
/// candidate".
fn expected_a() -> &'static str {
    if cfg!(feature = "ke16-a5") {
        "a5"
    } else if cfg!(feature = "ke16-a1") {
        "a1"
    } else if cfg!(feature = "ke16-a1-fifo") {
        "a1f"
    } else if cfg!(feature = "ke16-a2") {
        "a2"
    } else if cfg!(feature = "ke16-a3") {
        "a3"
    } else {
        "a0"
    }
}

fn expected_b() -> &'static str {
    if cfg!(feature = "ke16-b1") {
        "b1"
    } else if cfg!(feature = "ke16-b3") {
        "b3"
    } else {
        "b0"
    }
}

fn expected_w() -> &'static str {
    match (
        cfg!(feature = "ke16-w-gate"),
        cfg!(feature = "ke16-w-count"),
    ) {
        (true, true) => "wgc",
        (true, false) => "wg",
        (false, true) => "wc",
        (false, false) => "w0",
    }
}

/// `ke16-w-fanout` enables `ke16-c-batch`, so the fan-out is tested first for
/// the same reason as `ke16-a5`.
fn expected_c() -> &'static str {
    if cfg!(feature = "ke16-w-fanout") {
        "c1f"
    } else if cfg!(feature = "ke16-c-batch") {
        "c1"
    } else {
        "c0"
    }
}

/// True iff this binary was built with any tournament switch at all.
fn any_ke16_feature() -> bool {
    cfg!(feature = "ke16-a1")
        || cfg!(feature = "ke16-a1-fifo")
        || cfg!(feature = "ke16-a2")
        || cfg!(feature = "ke16-a3")
        || cfg!(feature = "ke16-a5")
        || cfg!(feature = "ke16-b1")
        || cfg!(feature = "ke16-b3")
        || cfg!(feature = "ke16-w-gate")
        || cfg!(feature = "ke16-w-count")
        || cfg!(feature = "ke16-c-batch")
        || cfg!(feature = "ke16-w-fanout")
}

#[test]
fn ke16_axis_consts_name_the_features_this_binary_was_built_with() {
    assert_eq!(
        (KE16_A, KE16_B, KE16_W, KE16_C),
        (expected_a(), expected_b(), expected_w(), expected_c()),
        "the `#[cfg]`-selected witness consts disagree with the features this binary was \
         compiled with; a run certified by KE16_EXPECT would then be labelled wrong"
    );
}

#[test]
fn ke16_variant_is_the_four_axis_tokens_joined() {
    assert_eq!(
        ke16_variant(),
        format!(
            "{}+{}+{}+{}",
            expected_a(),
            expected_b(),
            expected_w(),
            expected_c()
        ),
        "the variant string must be `{{A}}+{{B}}+{{W}}+{{C}}` over this build's tokens"
    );
}

/// The half `KE16_EXPECT` cannot check: a witness that never moves certifies
/// everything. Any enabled switch must take the string off the default.
#[test]
fn ke16_variant_moves_off_the_default_when_a_switch_is_enabled() {
    let variant = ke16_variant();
    if any_ke16_feature() {
        assert_ne!(
            variant, "a0+b0+w0+c0",
            "a tournament switch is enabled but the witness still reads the default build; \
             every number taken from this binary would be attributed to the wrong candidate"
        );
    } else {
        assert_eq!(
            variant, "a0+b0+w0+c0",
            "no switch is enabled, so the witness must read the default build"
        );
    }
}

/// Makes the whole binary honour the measurement protocol's environment check:
/// with `KE16_EXPECT` set to anything but this build, the suite reds here
/// instead of producing test results attributed to the wrong candidate.
#[test]
fn ke16_check_expected_variant_accepts_this_build() {
    ke16_check_expected_variant();
}

/// Builds a pool and waits, bounded, for every worker to be parked.
fn quiescent_pool(workers: u32) -> Arc<ThreadPool> {
    let pool = ThreadPoolBuilder::new()
        .num_threads(workers as usize)
        .build();
    let deadline = Instant::now() + Duration::from_secs(10);
    while pool.parked_mask().count_ones() < workers && Instant::now() < deadline {
        std::thread::yield_now();
    }
    pool
}

/// App-11: a quiescent pool reports every worker parked. This is the receipt
/// the occupancy instruments read as "who was asleep while this wave ran", so
/// an accessor that answered `0` for a sleeping pool would make every such
/// column silently empty.
#[test]
fn parked_mask_sets_one_bit_per_worker_when_the_pool_is_quiescent() {
    let workers = 4u32;
    let pool = quiescent_pool(workers);
    assert_eq!(
        pool.parked_mask(),
        (1u64 << workers) - 1,
        "every worker of an unloaded pool must be parked and marked"
    );
}

/// App-11: no bit above the worker count is ever set — the receipt indexes
/// workers, and a bit outside `[0, worker_count)` would name a worker that does
/// not exist.
#[test]
fn parked_mask_never_sets_a_bit_above_the_worker_count() {
    let workers = 4u32;
    let pool = quiescent_pool(workers);
    assert_eq!(
        pool.parked_mask() >> workers,
        0,
        "the idle bitset must be confined to this pool's workers"
    );
}

/// App-11: a worker inside a task body is NOT marked idle. Each of the `4 * W`
/// tasks blocks until released, so a worker that starts one never returns to
/// the poll loop; `started == W` therefore proves all `W` are inside bodies,
/// and the mask must be empty at that instant.
#[test]
fn parked_mask_is_empty_while_every_worker_runs_a_task() {
    let workers = 4u32;
    let pool = quiescent_pool(workers);
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

    let deadline = Instant::now() + Duration::from_secs(10);
    while started.load(Ordering::Acquire) < workers && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let observed_mask = pool.parked_mask();
    let observed_started = started.load(Ordering::Acquire);
    release.store(true, Ordering::Release);

    assert_eq!(
        observed_started, workers,
        "not every worker entered a body within the deadline; the reading below would be \
         about a pool that was still partly idle"
    );
    assert_eq!(
        observed_mask, 0,
        "a worker inside a task body has already unmarked itself"
    );
}

// ---------------------------------------------------------------------------
// The witness's REFUSAL half, measured in a child process
// ---------------------------------------------------------------------------
//
// `ke16_check_expected_variant_accepts_this_build` above proves the checker is
// SILENT when the environment agrees with the build. That is only the half a
// test can assert in-process. The half the measurement protocol actually leans
// on is the other one — `KE16_EXPECT` naming a variant this binary is NOT must
// STOP the run (`KE16-DESIGN-MEASUREMENT.md` §5 item 3: "features not enabled"
// is a refused shape precisely because a mislabelled number is indistinguishable
// from a real one once it reaches the results file). A checker that had lost its
// `panic!` — or whose comparison had been inverted — would leave every test in
// this file green and every published row unattributable.
//
// It cannot be asserted in-process: `std::env::set_var` is `unsafe` in Rust 2024
// and racy against a concurrently running test, and the panic would have to be
// caught while the harness's other tests keep reading the same variable. So the
// probe is the process boundary: re-exec THIS test binary filtered to the one
// existing check test, with `KE16_EXPECT` overridden on the child. The child's
// exit status is the assertion, and the child is what a real measurement run
// would have been.

/// A variant string no build can ever produce: every axis token is outside its
/// own alphabet (`KE16-DESIGN.md` §4 fixes `A ∈ {a0,a1,a1f,a2,a3,a5}`,
/// `B ∈ {b0,b1,b3}`, `W ∈ {w0,wg,wc,wgc}`, `C ∈ {c0,c1,c1f}`), so the mismatch
/// this drives cannot become a match under some later feature.
const IMPOSSIBLE_VARIANT: &str = "zz+zz+zz+zz";

/// The name of the in-process check test above, re-run as the child probe.
/// Naming the EXISTING test rather than a private helper is deliberate: what the
/// protocol relies on is that an ordinary test binary refuses a wrong
/// `KE16_EXPECT`, and that is the test that carries the call.
const CHILD_PROBE: &str = "ke16_check_expected_variant_accepts_this_build";

/// Runs this test binary again, filtered to `CHILD_PROBE` alone, with
/// `KE16_EXPECT` set to `expect`. Returns the child's success flag and its
/// combined output.
///
/// `--exact` keeps the filter from matching the parent tests (which would
/// recurse), and `--test-threads=1` keeps the output attributable.
fn run_child_probe(expect: &str) -> (bool, String) {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let output = std::process::Command::new(&exe)
        .arg(CHILD_PROBE)
        .args(["--exact", "--nocapture", "--test-threads=1"])
        .env("KE16_EXPECT", expect)
        .output()
        .expect("re-executing the test binary as a child process");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), combined)
}

/// A `KE16_EXPECT` that names another build must void the run, not merely
/// annotate it. Without this the whole anti-mislabelling protocol rests on an
/// untested `panic!`.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: Miri does not support spawning a child process; the witness \
              refusal is a native-build gate and is unrelated to the aliasing model"
)]
fn ke16_expect_naming_another_build_fails_the_run() {
    let (succeeded, out) = run_child_probe(IMPOSSIBLE_VARIANT);

    assert!(
        out.contains("running 1 test"),
        "the child ran no test — the probe name `{CHILD_PROBE}` no longer exists, so this gate \
         would pass vacuously whatever the checker did; child output:\n{out}"
    );
    assert!(
        !succeeded,
        "a child told `KE16_EXPECT={IMPOSSIBLE_VARIANT}` exited SUCCESSFULLY on a `{}` build: \
         a measured run could be filed against a candidate it was not built as; child output:\n{out}",
        ke16_variant()
    );
    assert!(
        out.contains("KE16_EXPECT mismatch"),
        "the child failed, but not through the witness check — its own oracle message never \
         appeared, so this gate does not observe what it claims to; child output:\n{out}"
    );
    assert!(
        out.contains(IMPOSSIBLE_VARIANT) && out.contains(&ke16_variant()),
        "the mismatch message must name BOTH the variant asked for and the one built, or the \
         operator cannot tell which side is wrong; child output:\n{out}"
    );
}

/// The calibration of the test above: the same child, told the truth, must pass.
/// Without this row a checker that rejected EVERY value — including the correct
/// one — would satisfy the refusal test while making every measured run
/// impossible.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: Miri does not support spawning a child process; see the sibling \
              refusal test"
)]
fn ke16_expect_naming_this_build_lets_the_run_proceed() {
    let variant = ke16_variant();
    let (succeeded, out) = run_child_probe(&variant);

    assert!(
        out.contains("running 1 test"),
        "the child ran no test; the calibration is vacuous. Child output:\n{out}"
    );
    assert!(
        succeeded,
        "a child told the truth (`KE16_EXPECT={variant}`) still failed — the witness check \
         refuses its own build, so no configuration of the tournament can be measured; \
         child output:\n{out}"
    );
}
