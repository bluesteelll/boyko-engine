//! L15: the crash path is armed BEFORE a panic, and says so when it cannot flush.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::sink::crash::{CrashState, arm, disarm, on_panic, state};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, info};

#[test]
fn the_crash_path_is_armed_at_enable_reports_an_unopenable_file_and_flushes_on_panic() {
    let path = std::env::temp_dir().join("boyko_l15_crash.log");
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::file::set_path(path.to_str().expect("a UTF-8 temp path")));
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: true,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));
    boyko_log::sink::slot::reset();
    disarm();
    assert_eq!(state(), CrashState::Absent, "a disarmed process reports Absent");

    // ── E0109: AN UNOPENABLE DESTINATION IS REPORTED WHILE THERE IS STILL A PROCESS TO TELL ──
    //
    // A directory is the portable "cannot be opened as a file": it exists, so the failure is the
    // open and not the path's absence, which is the case a deployment actually hits.
    let dir = std::env::temp_dir().join("boyko_l15_dir");
    let _ = std::fs::create_dir_all(&dir);
    assert!(!arm(dir.to_str().expect("a UTF-8 temp path")), "a directory opened as a crash file");
    assert_eq!(state(), CrashState::Absent, "a failed arm left the state claiming Ready");

    // The report must reach a reader. Re-point the byte sink at the readable file and drain.
    assert!(boyko_log::sink::file::set_path(path.to_str().expect("a UTF-8 temp path")));
    assert!(boyko_log::sink::file::open(0), "the temp file is openable");
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let e0109 = format!("boyko-E{:04}", boyko_log::codes::E0109.number());
    assert!(text.contains(&e0109), "an unopenable crash file emitted no {e0109}: {text:?}");

    // ── THE HOOK'S PROTOCOL: it FLUSHES, and it leaves the sink Exiting ───────────────────────
    assert!(arm(path.to_str().expect("a UTF-8 temp path")), "the temp file is armable");
    assert_eq!(state(), CrashState::Ready);
    info!(Log, "still-in-the-ring-when-it-died");
    assert!(on_panic(), "the hook could not flush with the drain role free");
    assert_eq!(state(), CrashState::Exiting, "the hook left the sink admitting records");
    let text = std::fs::read_to_string(&path).expect("the crash file is readable");
    assert!(
        text.contains("still-in-the-ring-when-it-died"),
        "the hook did not write the records that were pending when it ran: {text:?}"
    );

    // ── E0118: THE HOOK DOES NOT WAIT FOR THE DRAIN ROLE, IT REPORTS ─────────────────────────
    //
    // Held by a host draining by hand. A hook that waited would turn a crash into a HANG, which
    // leaves no artifact at all -- so a short file that SAYS it is short is the better outcome.
    let held = boyko_log::drain_owner::try_claim().expect("the role is free to take");
    assert!(!on_panic(), "the hook claimed a role another holder had");
    drop(held);
    let DrainResult::Ran(_) = drain() else { panic!("the role is free again") };
    let text = std::fs::read_to_string(&path).expect("the crash file is readable");
    let e0118 = format!("boyko-E{:04}", boyko_log::codes::E0118.number());
    assert!(text.contains(&e0118), "a refused flush emitted no {e0118}: {text:?}");

    disarm();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}
