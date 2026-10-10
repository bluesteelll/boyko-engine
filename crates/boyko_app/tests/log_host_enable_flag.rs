//! The other half of the host-reachability gate: `BOYKO_LOG` turns the logger **on**.
//!
//! A second binary because `EnginePlugins` cannot be built twice in one process (the second build
//! panics in `register_component_hooks::<DirectionalLight>`) and `BOYKO_LOG` is process-global.
//! See `log_host_reachable.rs` for why a gate that looks at a real host had to exist at all.
//!
//! # What this observes, and what it deliberately does not
//!
//! It sets the variable, builds the host, and asserts the lifecycle reached `Enabled` and the
//! engine targets were opened — i.e. that the **enable path runs from a host**. It does not assert
//! on the text that reaches `stderr`: the console destination writes to the real handle, and a
//! test that captured it would be gating the harness's plumbing rather than the engine's. What a
//! record's rendered text must look like is `boyko_log`'s own to gate, and it does.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_log::level::Level;
use boyko_log::lifecycle::{SinkState, state};
use boyko_log::target::{engine_targets, runtime_ceiling};

#[test]
fn the_flag_makes_the_host_enable_the_logger_and_open_every_engine_target() {
    // Set before the host builds, which is the whole point: the flag is read on the enable path,
    // once, and a host that reads it later would be reading it after the frames it was meant to
    // cover. Single-threaded by construction — this is the only test in the binary.
    // SAFETY: no other thread exists in this process at this point; the harness has not yet
    //   spawned anything and this binary carries exactly one test.
    unsafe { std::env::set_var("BOYKO_LOG", "debug") };

    assert_eq!(state(), SinkState::NotBooted, "nothing may touch the lifecycle before the host");

    // Armed BEFORE the host builds, because the record under test is emitted DURING the build and
    // the probe counts on the emitting thread. Watching `any` rather than a code: the session
    // header is an `info!`, which carries no code at all.
    boyko_log::probe::watch_any();

    let mut app = App::new();
    app.add_plugin(EnginePlugins::window("log host gate", 320, 240));

    // THE CLAIM, and it is the one no other logging gate can make: a REAL host turned it on.
    assert_eq!(
        state(),
        SinkState::Enabled,
        "BOYKO_LOG must make EnginePlugins take the enable path"
    );

    // Every engine target, not a sample: the flag's contract is the whole band, and a loop that
    // opened the first target and stopped would pass a spot check.
    let want = Level::Debug as u8;
    for (id, name) in engine_targets() {
        assert_eq!(
            runtime_ceiling(id),
            want,
            "target `{name}` was left closed after BOYKO_LOG=debug"
        );
    }

    // ── G16(d): THE THREE HEADER FACTS ACTUALLY REACH A READER ──────────────────────────────
    //
    // MEASURED by running this host with `BOYKO_LOG=debug` and reading its output: every census row
    // printed and **the header was absent**. `enable()` emits it, and the host armed its targets
    // AFTER calling `enable()` -- so `CONTROL` was still `.bss`-zero and gate (c) refused the one
    // record that says which build and which preset produced everything below it.
    //
    // No logging gate could see that: `l17_preset_boot` goes through `boot_preset`, which arms the
    // targets itself, so the shipped host was the only path carrying the defect. This assertion is
    // in the HOST test for the same reason this file exists at all -- the hole lies between the
    // gates, and only a test that builds a real host can stand in it.
    // FIRST, not last: the host emits its own "logging enabled at ..." line right after the
    // header, and `last_message` returned that one -- reporting the header missing while it was
    // there. An assertion that reads the wrong record is a red that accuses the wrong code.
    let seen = boyko_log::probe::first_message();
    assert!(
        seen.contains("build_profile=") && seen.contains("runtime_preset=")
            && seen.contains("ceiling=") && seen.contains("session="),
        "the host emitted no session header; G16(d)'s three facts reach nobody. First record was: {seen:?}"
    );

    // Teardown, so the sink thread does not outlive the harness with a destination open.
    let _ = boyko_log::lifecycle::shutdown();
}
