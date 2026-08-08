//! Rung L4's gate — the file sink, its cap, the manual drain, and the census's two verdicts.
//!
//! # One test, and the reason it is one test
//!
//! Every object here is process-global: the lifecycle state, the file handle, the control array,
//! the lanes, the per-target counters. Two `#[test]` functions in one binary run on two threads
//! with no ordering between them, so "the file holds exactly these lines" and "this target has
//! dropped nothing yet" would be coin flips. A process-global scenario is one test or it is flaky
//! by construction.
//!
//! The order is deliberate: the census's `MEASURED` verdict is taken **before** anything is
//! dropped, because `UNPROVEN(lossy)` is a one-way transition and a session cannot go back to
//! having lost nothing.

use boyko_log::census;
use boyko_log::level::Level;
use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable, shutdown};
use boyko_log::sink::file;
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Fontbake, Image};

/// Small enough that a few dozen lines reach it, large enough that the first drain does not.
const CAP: u64 = 4 * 1024;

#[test]
fn the_file_sink_writes_caps_and_the_census_tells_the_two_silences_apart() {
    // ── 1. The path is recorded, and a path that cannot be honoured is REFUSED ────────────────
    //
    // Refused rather than truncated: a truncated path names a different file, and writing a log to
    // the wrong file is worse than not writing one.
    assert!(!file::set_path(""), "an empty path must be refused");
    let too_long = "x".repeat(file::MAX_PATH_BYTES + 1);
    assert!(!file::set_path(&too_long), "a path past the buffer must be refused, not cut");
    assert!(!file::path_recorded(), "a refused path must not be recorded");

    let path = std::env::temp_dir().join("boyko_l4_file_sink.log");
    let _ = std::fs::remove_file(&path);
    assert!(file::set_path(path.to_str().expect("a UTF-8 temp path")));
    assert!(file::path_recorded());
    assert!(!file::is_open(), "set_path must open nothing");

    // ── 2. `boot` opens nothing; `enable` opens the file ──────────────────────────────────────
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: true,
        file_cap_bytes: CAP,
        sink_mode: SinkMode::Manual,
    });
    assert!(!file::is_open(), "boot() opened the file the config asked for");
    assert!(!path.exists(), "boot() created the file the config asked for");

    assert!(enable(), "enable() refused a freshly booted process");
    assert!(file::is_open(), "enable() did not open the recorded destination");
    assert_eq!(boyko_log::lifecycle::sink_mode(), SinkMode::Manual);

    // ── 3. Nothing drains until a host asks ───────────────────────────────────────────────────
    set_target_control(<Image as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));
    for i in 0..8u32 {
        boyko_log::info!(Image, "file sink probe {}", i);
    }
    assert_eq!(file::state().0, 0, "records reached the file with no drain: who drained?");

    let DrainResult::Ran(stats) = drain() else { panic!("the drain role is free in this process") };
    assert_eq!(stats.records, 8, "the manual drain did not carry every record");
    let (written, capped) = file::state();
    assert!(written > 0, "the drain ran but the file sink wrote nothing");
    assert!(!capped, "8 short lines must not reach a {CAP}-byte cap");

    // The bytes are really on disk, and they are the records.
    let text = std::fs::read_to_string(&path).expect("the sink's file is readable");
    assert!(text.contains("file sink probe"), "the file does not hold the records: {text:?}");
    assert_eq!(
        text.lines().count(),
        8,
        "one line per record, newline-terminated -- saw {text:?}"
    );

    // ── 4. The census, BEFORE anything is lost ────────────────────────────────────────────────
    //
    // Two verdicts that a naive census would collapse into one: a target that delivered is
    // MEASURED, and a target that delivered nothing is UNPROVEN — never *clean*. "No warnings from
    // the fontbaker" and "the fontbaker's warnings were switched off" produce the same empty log.
    let row = row_for(<Image as LogTarget>::ID);
    assert_eq!(row.records, 8);
    assert_eq!(row.dropped, 0);
    assert_eq!(row.status_str(), "MEASURED");

    let quiet = row_for(<Fontbake as LogTarget>::ID);
    assert_eq!(quiet.records, 0);
    assert_eq!(quiet.status_str(), "UNPROVEN", "a silent target is never clean");
    assert!(!census::lossy(), "nothing has been dropped yet");

    // ── 5. The cap stops the sink, and says so once ───────────────────────────────────────────
    //
    // Each drain writes what it carried; the cap is checked per line, so the file stops at the
    // last line that fitted rather than at the first one that did not start.
    let mut rounds = 0;
    while !file::state().1 {
        for i in 0..64u32 {
            boyko_log::info!(Image, "cap probe {}", i);
        }
        let _ = drain();
        rounds += 1;
        assert!(rounds < 200, "the {CAP}-byte cap was never reached in {rounds} rounds");
    }
    let (at_cap, _) = file::state();
    assert!(at_cap <= CAP, "the sink wrote {at_cap} bytes past a {CAP}-byte cap");

    // Past the cap the sink is inert: more records, more drains, not one more byte.
    for i in 0..64u32 {
        boyko_log::info!(Image, "post-cap probe {}", i);
    }
    let _ = drain();
    assert_eq!(file::state().0, at_cap, "a capped sink kept writing");
    let on_disk = std::fs::read_to_string(&path).expect("readable");
    assert!(!on_disk.contains("post-cap probe"), "a capped sink kept writing to the file");

    // ── 6. A drop makes the target's counts a LOWER BOUND, and the census says so ─────────────
    //
    // The lane is 16 KiB and nothing drains here, so this overflows it. `UNPROVEN(lossy)` is what
    // stops a reader adding these numbers up and calling the sum a total.
    let before = row_for(<Image as LogTarget>::ID).dropped;
    for i in 0..4096u32 {
        boyko_log::info!(Image, "overflow probe {} with some padding to fill the lane", i);
    }
    let row = row_for(<Image as LogTarget>::ID);
    assert!(row.dropped > before, "4096 undrained records did not overflow a 16 KiB lane");
    assert_eq!(row.status_str(), "UNPROVEN(lossy)", "a dropped record makes the count a bound");
    assert!(census::lossy(), "the single bit a UI must read before printing a total");

    // ── 7. The ring's wrap, swept ─────────────────────────────────────────────────────────────
    //
    // REGRESSION, and it is L1's rather than L4's. The producer's wrap rule has two arms: a tail
    // long enough for a header but too short for the record carries an explicit PAD, and **a tail
    // shorter than a header carries nothing at all** — there is no room for a PAD header, so the
    // producer just advances `write` past those bytes. The consumer had only the first arm. It read
    // a "header" out of bytes the producer had skipped and never written, took `len` from
    // uninitialised memory, and walked off into the ring — a torn read through a corrupted
    // `&'static LogSite` in release, and a `debug_assert` here.
    //
    // It needs the cursor to land in a 19-byte window out of 16 384, so a fixed record size either
    // hits it every lap or never. Cycling the payload length sweeps every residue class, and
    // draining every seventh record keeps the lane from simply overflowing instead.
    let filler = "x".repeat(96);
    for i in 0..4096u32 {
        let n = 1 + (i as usize % 61);
        boyko_log::info!(Image, "wrap probe {} {}", i, &filler[..n]);
        if i % 7 == 0 {
            let _ = drain();
        }
    }
    let _ = drain();

    // ── teardown ─────────────────────────────────────────────────────────────────────────────
    set_target_control(<Image as LogTarget>::ID, TargetControl::OFF);
    shutdown();
    assert!(!file::is_open(), "shutdown left the handle open");
    let _ = std::fs::remove_file(&path);
}

/// The census row for one target. Panics if the target has no row — which is itself the check that
/// every engine target gets one.
fn row_for(id: boyko_log::TargetId) -> census::CensusRow {
    census::rows().find(|r| r.id == id).expect("every engine target has a census row")
}
