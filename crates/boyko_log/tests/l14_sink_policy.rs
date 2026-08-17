//! L14: per-sink state, floor and filter decide delivery -- read back off a real file.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::sink::slot::{
    SLOT_CONSOLE, SLOT_ECS, SLOT_FILE, SinkState, any_sink_accepts, floor, reset, set_floor,
    set_only_target, set_state, set_target,
};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Ecs, Level, Log, error, info};

/// Drain, then read everything the file sink has written SO FAR.
///
/// The file is never deleted between legs and every marker is unique, because the sink holds its
/// handle open: on Windows the delete either fails and is swallowed, or succeeds and every later
/// write lands in a file no one can read. MEASURED -- the first draft deleted between legs and the
/// second leg read `""`, which looks exactly like "the floor refused the record".
fn drained_text(path: &std::path::Path) -> String {
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn state_floor_and_filter_each_decide_delivery_on_their_own() {
    let path = std::env::temp_dir().join("boyko_l14_policy.log");
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
    reset();

    // ── THE DEFAULT MUST DELIVER. A policy field arriving at L14 and silencing L0..L13's output
    //    as a side effect is precisely the defect this ladder exists to remove. ───────────────
    assert_eq!(floor(SLOT_FILE), Level::Trace, "a fresh slot admits everything");
    info!(Log, "policy-default");
    assert!(drained_text(&path).contains("policy-default"), "the default policy dropped a record");

    // ── FLOOR. `Warn` admits Error and Warn and refuses Info -- one sink, without touching any
    //    target's ceiling, which is the whole reason the floor is per-sink. ────────────────────
    set_floor(SLOT_FILE, Level::Warn);
    info!(Log, "below-the-floor");
    // An `error!` and not a `warn!`: `Error` is level 1, so it clears a `Warn` floor, and the
    // code is `Every` -- a `Once` code would make the record's absence ambiguous between "the
    // floor refused it" and "the latch had already fired", which is the confusion this leg exists
    // to resolve.
    error!(Log, boyko_log::codes::E0107, "above-the-floor {}", 0u32);
    let text = drained_text(&path);
    assert!(!text.contains("below-the-floor"), "an Info record cleared a Warn floor: {text:?}");
    assert!(text.contains("above-the-floor"), "a Warn record did not clear a Warn floor: {text:?}");
    set_floor(SLOT_FILE, Level::Trace);

    // ── FILTER. The target is excluded by bit, with the level and state untouched. ───────────
    set_target(SLOT_FILE, <Log as LogTarget>::ID, false);
    info!(Log, "filtered-out");
    assert!(!drained_text(&path).contains("filtered-out"), "an excluded target was delivered");
    set_target(SLOT_FILE, <Log as LogTarget>::ID, true);
    info!(Log, "filtered-in");
    assert!(drained_text(&path).contains("filtered-in"), "re-admitting the target did not restore it");

    // ── STATE. `Paused` stops delivery and is NOT `Off`: it keeps the destination, which is why
    //    the file the operator paused to preserve is still there to read afterwards. ──────────
    set_state(SLOT_FILE, SinkState::Paused);
    info!(Log, "while-paused");
    assert!(!drained_text(&path).contains("while-paused"), "a paused sink received a record");
    set_state(SLOT_FILE, SinkState::Active);
    info!(Log, "after-resume");
    assert!(drained_text(&path).contains("after-resume"), "resuming did not restore delivery");

    // ── THE `UNPROVEN(unsunk)` QUESTION. A target enabled at Info with no sink accepting it
    //    produces silence indistinguishable from a clean run -- the vacuous gate in a new
    //    costume (disposition E20), so the census has to be able to ASK. ────────────────────
    reset();
    assert!(any_sink_accepts(<Log as LogTarget>::ID, Level::Info), "the default policy is unsunk");
    for slot in [SLOT_CONSOLE, SLOT_FILE, SLOT_ECS, 3] {
        set_state(slot, SinkState::Off);
    }
    assert!(
        !any_sink_accepts(<Log as LogTarget>::ID, Level::Info),
        "every sink is Off and the census would still have reported the silence as clean"
    );
    reset();

    // And `set_only_target` narrows to one WITHOUT clearing 255 bits one at a time, so no reader
    // drains against a half-built filter.
    set_only_target(SLOT_FILE, <Log as LogTarget>::ID);
    assert!(any_sink_accepts(<Log as LogTarget>::ID, Level::Info));
    let other = <Ecs as LogTarget>::ID;
    assert!(!boyko_log::sink::slot::filter_admits(SLOT_FILE, other), "set_only_target left a bit set");
    reset();
    let _ = std::fs::remove_file(&path);
}

#[test]
#[ignore = "drives every sink Off; run alone -- `--ignored --test-threads=1`"]
fn an_armed_target_no_sink_accepts_is_unsunk_and_says_so() {
    // Runs `#[ignore]`d and alone because its condition is EVERY SINK OFF, which is a process-wide
    // state: any other test in this binary emitting while it holds that state would have its
    // records discarded, and would then measure this test's setup instead of its own.
    let path = std::env::temp_dir().join("boyko_l14_unsunk.log");
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
    set_target_control(<Ecs as LogTarget>::ID, TargetControl::new(Level::Info, 0, false));
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));
    reset();

    // Armed, delivered nothing, and every sink refuses it: the row must NOT read plain `UNPROVEN`,
    // which a reader is entitled to interpret as "that subsystem had nothing to say".
    set_target(SLOT_FILE, <Ecs as LogTarget>::ID, false);
    set_target(SLOT_CONSOLE, <Ecs as LogTarget>::ID, false);
    set_target(SLOT_ECS, <Ecs as LogTarget>::ID, false);
    set_target(3, <Ecs as LogTarget>::ID, false);

    let row = boyko_log::census::rows()
        .find(|r| r.id == <Ecs as LogTarget>::ID)
        .expect("every engine target has a census row");
    // Asserted on `status_str` and not the enum: that string is the stable surface a support
    // ticket quotes, and it is what a reader greps for.
    assert_eq!(
        row.status_str(),
        "UNPROVEN(unsunk)",
        "an armed target no sink accepts reported as {} -- indistinguishable from a clean run",
        row.status_str()
    );

    // The record reaches a reader. `Log` is still sunk, which is why the report about `Ecs` being
    // unsunk is not itself unsunk -- a report that shared its subject's fate would be unreadable
    // exactly when it mattered.
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    let text = std::fs::read_to_string(&path).expect("the sink's file is readable");
    let code = format!("boyko-W{:04}", boyko_log::codes::W0111.number());
    assert!(text.contains(&code), "an unsunk target emitted no {code}: {text:?}");
    assert!(text.contains("ecs"), "W0111 must NAME the target: {text:?}");

    reset();
    let _ = std::fs::remove_file(&path);
}
