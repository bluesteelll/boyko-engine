//! `BOYKO_LOG_PRESET` naming nothing reports `boyko-W1803` instead of quietly falling back.
//!
//! # A third host binary, and it is not a style choice
//!
//! `EnginePlugins` cannot be built twice in one process -- the second build panics in
//! `register_component_hooks::<DirectionalLight>` -- and the environment variables under test are
//! process-global. So each host claim is its own binary with one `#[test]`, exactly as
//! `log_host_reachable.rs` and `log_host_enable_flag.rs` already are.
//!
//! # What the fallback costs without this record
//!
//! It is a working configuration, so nothing breaks. What breaks is the operator's belief: someone
//! who typed `shiping` gets a run whose header says `runtime_preset=custom`, and `custom` is also
//! what an ordinary un-flagged run says. The two are indistinguishable, and the reader most likely
//! to be misled is the one who set the flag.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_log::lifecycle::{SinkState, state};

#[test]
fn a_preset_name_the_table_does_not_carry_is_reported_and_not_guessed() {
    // SAFETY-of-intent: this binary owns the process environment; it has one test.
    unsafe {
        std::env::set_var("BOYKO_LOG_PRESET", "shiping");
        std::env::set_var("BOYKO_LOG", "debug");
    }
    assert_eq!(state(), SinkState::NotBooted, "nothing may touch the lifecycle before the host");

    // Watched by CODE, not `any`: the host emits its own lifecycle lines around this one, and a
    // count over everything would pass on any of them.
    boyko_log::probe::watch(b'W', boyko_log::codes::W1803.number());

    let mut app = App::new();
    app.add_plugin(EnginePlugins::window("bad preset gate", 320, 240));

    assert_eq!(
        boyko_log::probe::watched(),
        1,
        "a preset name the table does not carry must be REPORTED; the fallback is a working \
         configuration, which is exactly why silence about it misleads"
    );
    assert!(
        boyko_log::probe::last_message().contains("shiping"),
        "the record must name the value that was rejected, or the operator cannot see their typo: \
         {:?}",
        boyko_log::probe::last_message()
    );

    // And the fallback really did happen: the host is up, on the hand-built configuration.
    assert_eq!(state(), SinkState::Enabled, "the fallback must still bring the logger up");
    assert_eq!(
        boyko_log::lifecycle::boot_preset_recorded(),
        None,
        "a refused preset must leave NO preset recorded, or the header would name one nobody chose"
    );

    let _ = boyko_log::lifecycle::shutdown();
}
