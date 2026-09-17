//! Controls for `tests/child_guard/mod.rs`, the deadline every re-DXC gate waits under.
//!
//! The re-DXC gates cannot show that their watchdog works: a real `dxc` finishes in about a second,
//! so under the real deadline the kill path never runs. These tests drive the same code with
//! ordinary system programs instead: a sleeper that outlives a short deadline (the NEGATIVE
//! control, which must end as a hang, killed and reaped, near the deadline rather than at the
//! sleeper's own end), a child that finishes in time, a child that fails, and a program that does
//! not exist; and a scratch file that a panicking gate must not leave behind. None needs a GPU or
//! the Vulkan SDK.

use std::panic::{self, AssertUnwindSafe};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A deadline the sleeper outlives by a wide margin.
const SHORT: Duration = Duration::from_millis(500);

/// How long the sleeper would run if nothing killed it.
const SLEEPER_SECS: u64 = 30;

/// A process that sleeps for about [`SLEEPER_SECS`] seconds and spawns no child of its own, so a
/// kill ends it and closes its pipes. `ping -n N` waits about N - 1 seconds on Windows.
fn sleeper() -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("ping");
        cmd.args(["-n", &(SLEEPER_SECS + 1).to_string(), "127.0.0.1"]);
        cmd
    } else {
        let mut cmd = Command::new("sleep");
        cmd.arg(SLEEPER_SECS.to_string());
        cmd
    }
}

/// A shell running `script`, which finishes at once.
fn shell(script: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", script]);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }
}

/// Runs `f`, which must panic, and returns the panic message and how long `f` took.
fn expect_hang(f: impl FnOnce()) -> (String, Duration) {
    let start = Instant::now();
    let payload = panic::catch_unwind(AssertUnwindSafe(f))
        .expect_err("a sleeper under a 0.5 s deadline must end in the hung-child panic");
    let elapsed = start.elapsed();
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .expect("invariant: the hung-child panic carries a formatted String message");
    (message, elapsed)
}

/// Asserts the parts of a hung-child message that a reader and a triager rely on.
fn assert_hang_report(message: &str, elapsed: Duration) {
    assert!(message.starts_with(HUNG_PREFIX), "the message must open with the hang marker: {message}");
    assert!(
        message.contains(if cfg!(windows) { "ping" } else { "sleep" }),
        "the message must name the program that hung: {message}"
    );
    assert!(
        message.contains("kill, then reap: Ok("),
        "the child must be killed AND reaped before the test fails: {message}"
    );
    assert!(
        !message.contains("dxc failed") && !message.contains("is NOT the re-DXC"),
        "a hang must not read like a failed compile or a stale artifact: {message}"
    );
    // The bound is loose against the 0.5 s deadline and tight against the sleeper's 30 s: a
    // watchdog that only returned when the child ended on its own would take the full 30 s.
    assert!(
        elapsed < Duration::from_secs(10),
        "the wait must end near the deadline, not when the sleeper finishes: took {elapsed:?}"
    );
}

#[test]
fn a_status_child_that_outlives_its_deadline_is_killed_and_reported_as_hung() {
    let (message, elapsed) = expect_hang(|| {
        let _ = sleeper().stdout(Stdio::null()).status_within(SHORT);
    });
    assert_hang_report(&message, elapsed);
}

#[test]
fn an_output_child_that_outlives_its_deadline_is_killed_and_reported_as_hung() {
    let (message, elapsed) = expect_hang(|| {
        let _ = sleeper().output_within(SHORT);
    });
    assert_hang_report(&message, elapsed);
}

#[test]
fn a_child_that_finishes_in_time_returns_its_status_and_output() {
    let status = shell("exit 0")
        .status_within(DXC_DEADLINE)
        .expect("invariant: the system shell runs");
    assert!(status.success(), "`exit 0` must succeed: {status:?}");

    let out = shell("echo child-guard-ok")
        .output_within(DXC_DEADLINE)
        .expect("invariant: the system shell runs");
    assert!(out.status.success(), "`echo` must succeed: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("child-guard-ok"), "the captured stdout must hold the echo: {stdout:?}");
}

#[test]
fn a_failing_child_is_an_exit_status_not_a_hang() {
    let status = shell("exit 3")
        .status_within(DXC_DEADLINE)
        .expect("invariant: the system shell runs");
    assert_eq!(status.code(), Some(3), "a failing child must report its own exit code");
}

#[test]
fn a_missing_program_is_a_spawn_error_not_a_hang() {
    let err = Command::new("boyko-child-guard-no-such-program")
        .status_within(SHORT)
        .expect_err("a program that does not exist cannot be spawned");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "unexpected spawn error: {err}");
}

#[test]
fn scratch_paths_differ_per_call_and_keep_the_extension() {
    let a = child_guard::scratch_path("same.redxc.spv");
    let b = child_guard::scratch_path("same.redxc.spv");
    assert_ne!(a, b, "two calls with one name must not share a temp file");
    assert!(a.to_string_lossy().ends_with(".redxc.spv"), "the extension must survive: {}", a.display());
    assert!(
        a.to_string_lossy().contains(&std::process::id().to_string()),
        "the path must carry this process's id: {}",
        a.display()
    );
}

#[test]
fn a_scratch_file_is_deleted_when_a_panic_unwinds_past_it() {
    let mut kept = None;
    let unwound = panic::catch_unwind(AssertUnwindSafe(|| {
        let scratch = child_guard::scratch_path("unwind-probe.hlsl");
        std::fs::write(&scratch, "// probe").expect("invariant: the temp dir is writable");
        assert!(scratch.exists(), "the probe file must exist before the panic");
        kept = Some(scratch.to_path_buf());
        panic!("a gate failing mid-way, before its own tidy-up");
    }));
    assert!(unwound.is_err(), "the closure must have panicked");
    let path = kept.expect("invariant: the closure recorded its path before panicking");
    assert!(
        !path.exists(),
        "a failing gate must not leave its scratch file behind: {}",
        path.display()
    );
}

mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE, HUNG_PREFIX};
