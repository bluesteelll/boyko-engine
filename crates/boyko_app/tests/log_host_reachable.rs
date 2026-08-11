//! The gate that looks at a REAL host: does building `EnginePlugins` reach the logger at all?
//!
//! # Why this test exists, and why no logging gate could have replaced it
//!
//! Measured at the opening of logging rung L7: `boyko_log::lifecycle::boot` and `enable` were
//! called from **nowhere** outside tests. L5 landed the ECS seam, L6 landed the engine's own
//! emitters, and in a shipped run every record went into a `.bss` lane ring with no consumer.
//!
//! Every other logging gate is shaped like this:
//!
//! ```text
//! boot(cfg); enable(); set_target_level(..); <cause the condition>; drain_once(); assert!(delivered)
//! ```
//!
//! — it performs the host's steps **itself**, then asks whether a record arrived. The answer is
//! always yes, and the question "does any host perform them?" is one such a test structurally
//! cannot ask, because it has just performed them. The hole lies **between** the gates. This file
//! and its sibling `log_host_enable_flag.rs` are the two that build the host and look.
//!
//! The profiler's identical hole is why the pattern is named rather than merely fixed: fifteen
//! green rungs passed over `ProfilerPlugin` never being added, for the same reason.
//!
//! # Two binaries, and it is not a style choice
//!
//! `EnginePlugins` cannot be built twice in one process — the second build panics in
//! `register_component_hooks::<DirectionalLight>`. `BOYKO_LOG` is also process-global. So the
//! flag-off and flag-on claims are two test binaries, each with one `#[test]`.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_log::lifecycle::{FlushResult, SinkState, flush, state};

#[test]
fn the_host_boots_the_logger_and_leaves_it_off_without_the_flag() {
    // SAFETY-of-intent: the variable must be absent for this binary's claim. It is set by nothing
    // else here, and this is the only test in the binary.
    assert!(
        std::env::var_os("BOYKO_LOG").is_none(),
        "this binary asserts the FLAG-OFF behaviour; BOYKO_LOG must not be set for it"
    );

    assert_eq!(
        state(),
        SinkState::NotBooted,
        "nothing may touch the logging lifecycle before a host builds"
    );

    let mut app = App::new();
    app.add_plugin(EnginePlugins::window("log host gate", 320, 240));

    // THE CLAIM. Before L7 this was `NotBooted` after a full `EnginePlugins` build, and every
    // logging gate in the workspace was green anyway.
    assert_eq!(
        state(),
        SinkState::Booted,
        "EnginePlugins must record the logging configuration -- `boot` is a pure struct-fill, so \
         it is unconditional, and without it `enable()` has nothing to act on"
    );

    // And it must stop there: `boot` spawns no thread and opens no destination, so with the flag
    // unset there is still no consumer. This is the half that makes the cost claim true rather
    // than merely stated.
    assert_eq!(
        flush(),
        FlushResult::NoConsumer,
        "an un-enabled host must have no consumer; a thread spawned here would be a cost no \
         launch flag authorised"
    );
}
