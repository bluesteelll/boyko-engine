//! The round trip: records go in through the sink, and `logdec` prints them back as text.
//!
//! # This is the gate the format never had
//!
//! `l13b_binary_format` proves the codec round-trips bytes. `l13b_binary_sink_wired` proves the
//! sink writes frames. Neither proves the thing the format exists for: that a `.blog` on disk can
//! be turned back into readable lines by a tool a person can run. Every earlier gate would stay
//! green on a format whose only reader was a private walker inside a test — which is what it was.
//!
//! So this test runs the **actual binary**, on an **actual file**, and asserts the ARGUMENT VALUES
//! come back. Asserting on the format literal alone would pass on a decoder that printed the
//! literal and dropped every value, which is the failure L6 already found once in this crate: a
//! sink that carried every argument through the ring and threw it away.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::sink::slot::{SLOT_CONSOLE, SLOT_ECS, SLOT_FILE, SinkState, reset, set_state};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, info, warn};

#[test]
fn a_blog_written_by_the_sink_decodes_back_through_the_logdec_binary() {
    let path = std::env::temp_dir().join("boyko_l13b_logdec.blog");
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::binary::set_path(path.to_str().expect("a UTF-8 temp path")));

    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: false,
        binary: false,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));

    // ONLY the binary sink, so nothing here can be green because the text sink also had it.
    reset();
    for slot in [SLOT_CONSOLE, SLOT_FILE, SLOT_ECS] {
        set_state(slot, SinkState::Off);
    }
    assert!(boyko_log::sink::binary::open(), "the temp path is openable");

    info!(Log, "budget spent {} of {}", 137u32, 512u32);
    warn!(Log, boyko_log::codes::W0117, "late by {} us", 2_400u32);
    info!(Log, "asset {} loaded", "terrain_04");
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };

    // ── RUN THE TOOL, NOT A COPY OF IT ──────────────────────────────────────────────────────
    //
    // `CARGO_BIN_EXE_logdec` is the binary this workspace builds. A test that re-implemented the
    // decode would prove a decoder nobody runs, which is exactly the hole this rung closes.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_logdec"))
        .arg(&path)
        .output()
        .expect("logdec is built as part of this crate");
    assert!(out.status.success(), "logdec failed: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).expect("logdec prints UTF-8");

    // ── THE ARGUMENT VALUES SURVIVE, WHICH IS THE WHOLE CLAIM ───────────────────────────────
    //
    // The format literal alone would pass on a decoder that printed the literal and dropped every
    // value. Each of these three numbers reached the file as bytes and came back as text.
    assert!(text.contains("budget spent 137 of 512"), "a u32 pair did not survive: {text}");
    assert!(text.contains("late by 2400 us"), "a warn's argument did not survive: {text}");
    assert!(text.contains("asset terrain_04 loaded"), "a &str did not survive: {text}");

    // ── AND THE LINES ARE LOCATABLE ─────────────────────────────────────────────────────────
    assert!(
        text.contains("l13b_logdec_roundtrip.rs"),
        "logdec printed no source location, so a reader cannot find the emitter: {text}"
    );

    // ── THE STAMPS ARE TIMES, BECAUSE THE ANCHOR CARRIES THE SCALE ──────────────────────────
    //
    // The anchor used to carry ticks alone, which are a property of the producing CPU: a reader on
    // another machine could print a tick count and nothing better. `ticks_per_ns` in the anchor is
    // what turns a delta into a duration, and `NO ANCHOR` is what the tool prints without one.
    assert!(text.contains("ticks_per_ns="), "the anchor line does not carry the clock scale: {text}");
    assert!(!text.contains("NO ANCHOR"), "the file opened without an anchor: {text}");
    assert!(!text.contains("RAGGED TAIL"), "the sink left a truncated frame behind: {text}");
    // ── AND THE STAMPS ARE SMALL, WHICH IS THE ASSERTION THAT CATCHES A PLAUSIBLE WRONG ONE ──
    //
    // MEASURED: the first draft of this test asserted only that the output contained `ms`, and it
    // PASSED while every stamp read `+425.840ms` -- because `write_record` put the low 32 bits of
    // the ABSOLUTE tick counter into a field named `tsc_delta`. Three records emitted microseconds
    // apart cannot be 425 ms from the anchor. Asserting the UNIT and not the VALUE is how a number
    // that is plausible and wrong survives a green test.
    let stamps: Vec<f64> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix('+'))
        .filter_map(|l| l.split("ms").next())
        .filter_map(|n| n.parse::<f64>().ok())
        .collect();
    assert_eq!(stamps.len(), 3, "expected three stamped records: {text}");
    assert!(
        stamps.iter().all(|ms| *ms < 50.0),
        "a record emitted microseconds after the anchor is stamped {stamps:?} ms -- the delta is \
         not measured from the anchor: {text}"
    );

    boyko_log::sink::slot::reset();
    let _ = std::fs::remove_file(&path);
}
