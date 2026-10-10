//! `BOYKO_LOG_PRESET=off` beats `BOYKO_LOG`, and the reason is a cost rather than a precedence rule.
//!
//! # The state the other resolution produces
//!
//! Honouring the level would ARM every engine target while `Off` has opened no sink -- which is
//! exactly what `census` names `UNPROVEN(unsunk)`: every site pays gate (c)'s `.bss` load and one
//! branch, forever, and delivers nothing. And `boyko-W0111`, the code whose entire job is to report
//! that condition, could not be printed either: printing needs a destination `Off` did not open.
//!
//! MEASURED: a host run at `BOYKO_LOG_PRESET=off` with `BOYKO_LOG=debug` emitted not one line,
//! census included. The contradiction was resolvable only by reading the source.
//!
//! A fourth host binary for the reason the other three are separate: `EnginePlugins` cannot be
//! built twice in one process, and these variables are process-global.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_log::target::engine_targets;
use boyko_log::{Level, runtime_ceiling};

#[test]
fn off_configures_nothing_even_when_a_level_flag_asks_for_something() {
    // SAFETY-of-intent: this binary owns the process environment; it has one test.
    unsafe {
        std::env::set_var("BOYKO_LOG_PRESET", "off");
        std::env::set_var("BOYKO_LOG", "debug");
    }

    let mut app = App::new();
    app.add_plugin(EnginePlugins::window("off preset gate", 320, 240));

    // Every target, not a sample: the claim is that NOTHING was armed, and a loop that checked one
    // would pass on a configuration that armed the rest.
    for (id, name) in engine_targets() {
        assert_eq!(
            runtime_ceiling(id),
            Level::Off as u8,
            "`off` left target `{name}` armed at {}; an armed target with no sink pays gate (c) at \
             every site and delivers nothing, and cannot even report that it did",
            runtime_ceiling(id)
        );
    }

    // The preset IS recorded, so a header would name it -- the row was selected and honoured, not
    // ignored. (`Off` opens no destination, so nothing prints it; that is the row's meaning.)
    assert_eq!(
        boyko_log::lifecycle::boot_preset_recorded(),
        Some(boyko_log::preset::LogRuntimePreset::Off),
        "the `off` row must be recorded as chosen; falling through to the hand-built config would \
         open a console this run asked not to have"
    );

    let _ = boyko_log::lifecycle::shutdown();
}
