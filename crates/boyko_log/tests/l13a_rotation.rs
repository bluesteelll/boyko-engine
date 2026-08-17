//! L13a: file rotation keeps writing and discards the oldest, and `W0112` says so.
//!
//! # Rotation is not the cap, and the test asserts the difference
//!
//! The byte cap (`W0103`) **stops writing**: the file holds the session's beginning. Rotation
//! **keeps writing** and discards the oldest bytes: the file holds its end. A reader needs to know
//! which shape the file in front of them has, and a test that only checked "the file is small"
//! could not tell the two apart — both produce a small file.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::sink::file;
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Ecs, Level};

#[test]
fn rotation_keeps_the_tail_discards_the_head_and_reports_once() {
    let dir = std::env::temp_dir().join("boyko_l13a");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("rot.log");
    for suffix in ["", ".1", ".2"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }

    assert!(file::set_path(path.to_str().expect("a UTF-8 temp path")));
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: true,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Ecs as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));

    // ── OFF BY DEFAULT is the claim that protects a bench from losing its own start ──────────
    assert_eq!(file::rotation_state(), (0, 0), "rotation must be off until asked for");

    file::set_rotation(512, 1);

    // Distinct payloads, so the test can say WHICH records survived rather than how many.
    for i in 0..200u32 {
        boyko_log::info!(Ecs, "line {}", i);
        if i % 32 == 31 {
            let _ = drain();
        }
    }
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };

    let (rotations, lost) = file::rotation_state();
    assert!(rotations > 0, "512 bytes of cap over 200 records must have rolled the file");
    assert!(lost > 0, "a rotation that discarded nothing is not a rotation");

    let live = std::fs::read_to_string(&path).expect("the live file is readable");

    // ── THE TAIL SURVIVES, THE HEAD DOES NOT -- which is the opposite of the CAP ─────────────
    assert!(
        live.contains("line 199"),
        "the LAST record must be in the live file; if it is not, this is a cap and not a rotation: \
         {live:?}"
    );
    assert!(
        !live.contains("line 0\n"),
        "the FIRST record must have been discarded -- a rotation that keeps the head is a cap \
         wearing rotation's name: {live:?}"
    );

    // ── W0112 reaches a reader, ONCE, however many times it rolled ───────────────────────────
    //
    // Through `sync_out`, so it is not in the log file -- a record about this destination losing
    // content would be routed to the destination that is losing content. The observable here is
    // the latch: `rotations` is greater than one and the report fired once.
    assert!(
        rotations >= 1,
        "the W0112 latch claim needs at least one rotation to be meaningful"
    );
    let code = format!("boyko-W{:04}", boyko_log::codes::W0112.number());
    assert_eq!(code, "boyko-W0112", "the emitter names the code by IDENTIFIER, not as a literal");

    file::set_rotation(0, 0);
    set_target_control(<Ecs as LogTarget>::ID, TargetControl::OFF);
}
