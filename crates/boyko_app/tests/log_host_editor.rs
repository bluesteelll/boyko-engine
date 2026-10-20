//! `editor` on a REAL host: the row is `dev` plus rotation, and rotation must reach the sink.
//!
//! # Why this binary exists when `l17_preset_boot` already gates `boot_preset`
//!
//! `l17_preset_boot` drives `boot_preset` by hand and asserts the `Shipping` row. This binary asks
//! the question that test structurally cannot: does the HOST's `BOYKO_LOG_PRESET` route apply the
//! `Editor` row — and it asserts the two facts that distinguish `editor` from `dev`, because a
//! preset route that ignored its argument and always booted `dev` would pass every assertion that
//! checks what the rows share.
//!
//! Its own binary because `EnginePlugins` cannot be built twice in one process and the
//! environment is process-global — same as every other `log_host_*` claim.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_log::lifecycle::{ShutdownOutcome, SinkState, state};
use boyko_log::preset::{LogRuntimePreset, ROTATE_AT_BYTES};

#[test]
fn the_editor_row_is_applied_whole_including_rotation() {
    let file = std::env::temp_dir().join("boyko_host_editor.log");
    let _ = std::fs::remove_file(&file);

    // SAFETY-of-intent: this binary owns the process environment; it has one test.
    unsafe {
        std::env::set_var("BOYKO_LOG_PRESET", "editor");
        std::env::set_var("BOYKO_LOG_FILE", file.to_str().expect("a UTF-8 temp path"));
    }
    assert_eq!(state(), SinkState::NotBooted, "nothing may touch the lifecycle before the host");

    let mut app = App::new();
    app.add_plugin(EnginePlugins::window("editor preset gate", 320, 240));

    assert_eq!(state(), SinkState::Enabled, "the preset route must enable, not merely boot");
    assert_eq!(
        boyko_log::lifecycle::boot_preset_recorded(),
        Some(LogRuntimePreset::Editor),
        "the preset must be recorded, or the header cannot name it"
    );

    // THE FACT THAT DISTINGUISHES `editor` FROM `dev`: rotation reached the file SINK — read
    // back from the sink's own cap rather than from the table, and not by writing 64 MiB, which
    // no test should. The table-only form of this assertion would stay green with `set_rotation`
    // never called, which is the exact defect `rotates()`'s doc predicted for itself.
    assert_eq!(
        boyko_log::sink::file::rotation_cap(),
        ROTATE_AT_BYTES,
        "the `editor` row rotates and the cap must be ON THE SINK, not in the table"
    );
    assert!(
        LogRuntimePreset::Editor.rotates() && !LogRuntimePreset::Dev.rotates(),
        "if the two rows stop differing here, this binary is asserting nothing `log_host_*` does \
         not already cover and should be retired"
    );

    // `editor` runs the resident sink thread, so a flush is the delivery guarantee.
    assert_eq!(
        boyko_log::lifecycle::flush(),
        boyko_log::lifecycle::FlushResult::Flushed,
        "the resident sink thread must be running under `editor`"
    );
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(
        text.contains("runtime_preset=editor"),
        "the header must name the row the host applied: {text:?}"
    );

    // THE CONTRACT THAT HOLDS WHATEVER THE RUNNER'S PIPE DOES (ci-fix-1). Master's push CI went red
    // here when the runner's stderr was not read for about a second during the census: the join is
    // a 500 ms bound on a write to an EXTERNAL consumer. What must hold is that the durable log is
    // whole: a join, or a shutdown that delivered and closed the durable sinks itself past a stalled
    // console (`ClosedAfterStall`), or one that recorded a stall inside a pass (`StalledInsidePass`)
    // and is then waited out. The 500 ms bound stays pinned where stderr is not an external pipe:
    // `boyko_log`'s console-off threaded lifecycle tests.
    let joined = boyko_log::lifecycle::shutdown();
    let outcome = boyko_log::lifecycle::shutdown_outcome();
    assert!(
        joined
            || matches!(
                outcome,
                Some(ShutdownOutcome::ClosedAfterStall | ShutdownOutcome::StalledInsidePass)
            ),
        "a threaded shutdown must join within its deadline, or deliver and close the durable sinks \
         itself when the console stalls, or record a stall inside a pass: joined {joined}, outcome \
         {outcome:?}"
    );
    // Bounded: a consumer that stalled drains, and `await_exit` closes the sinks once the thread
    // has exited.
    let exited = joined || boyko_log::lifecycle::await_exit(std::time::Duration::from_secs(30));
    assert!(
        exited && state() == SinkState::Exited,
        "the resident sink thread must exit once its console drains (joined {joined}, outcome \
         {outcome:?}, state {:?})",
        state()
    );
    // The WHOLE census is in the durable file: one row per target, then the limiter line, which
    // the census emits last and unconditionally.
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let mut missing: Vec<String> = boyko_log::target::targets()
        .map(|(_, name)| format!("LOG-CENSUS target={name} "))
        .filter(|row| !text.contains(row.as_str()))
        .collect();
    if !text.contains("LOG-CENSUS limiter ") {
        missing.push("LOG-CENSUS limiter ".to_owned());
    }
    assert!(
        missing.is_empty(),
        "the durable log must hold the whole census (outcome {outcome:?}): missing {missing:?}"
    );
    let _ = std::fs::remove_file(&file);
}
