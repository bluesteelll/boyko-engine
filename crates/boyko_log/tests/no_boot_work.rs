//! **A binary that boots diagnostics and never enables them.**
//!
//! This file exists as its own test target for one reason, and the reason is not tidiness: the
//! panic hook is **process-global and, by design, never uninstalled**. Once any test in a binary
//! calls `enable()`, the hook is installed for the rest of that binary's life — so *"`boot()` does
//! not install a panic hook"* stops being observable in the crate's unit-test binary the moment a
//! sibling test enables. The property is real and load-bearing; it just needs a process that never
//! enables.
//!
//! MEASURED: asserting it beside the unit tests failed deterministically, and the failure was the
//! fixture's, not the code's — the hook had already been installed by another test.
//!
//! **Nothing here may call `enable()`.** A future edit that does silently deletes the only place
//! this property is checked, which is why the whole file is one scenario with this note on it.

use boyko_log::lifecycle::{LogConfig, SinkState, boot, hook_fired, sink_passes, state};

#[test]
fn boot_alone_changes_nothing_a_process_can_observe() {
    // The starting point: `.bss` zero, and nothing has run.
    assert_eq!(state(), SinkState::NotBooted);
    assert_eq!(hook_fired(), 0, "no hook may have run before boot");
    assert_eq!(sink_passes(), 0, "no sink pass may have happened before boot");

    boot(LogConfig { console: true, sink_thread: true, ecs_ring: true });
    assert_eq!(state(), SinkState::Booted);

    // Both wishes were recorded and NEITHER was acted on.
    assert_eq!(
        boyko_log::sync_out::write_oracle_line("boyko: ", "must not be written"),
        None,
        "boot() opened the console destination the config asked for"
    );
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert_eq!(sink_passes(), 0, "boot() spawned the sink thread the config asked for");
    assert_eq!(
        boyko_log::sink::ecs::published(),
        0,
        "boot() touched the ECS handoff ring the config asked for"
    );

    // And the panic behaviour of the process is untouched. This is the assertion that needs its
    // own binary: it is only true in a process where `enable()` has never run.
    let caught = std::panic::catch_unwind(|| panic!("deliberate, boot-only"));
    assert!(caught.is_err(), "the panic must propagate exactly as it would have without us");
    assert_eq!(
        hook_fired(),
        0,
        "boot() installed a panic hook; only enable() may change process-global behaviour"
    );
}
