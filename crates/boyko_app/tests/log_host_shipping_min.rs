//! `shipping-min` on a REAL host: the per-frame drain is the preset's whole promise.
//!
//! # What this binary measured before the fix, stated as the reason it exists
//!
//! Four defects, one root, and every logging gate in the workspace was green over all of them:
//!
//! * **No host added `LogPlugin`.** The L5 seam — `log_drain_system`, `LogRing`, `LogStats` —
//!   was registered by nothing outside its own tests. This is the profiler's L7 hole verbatim
//!   ("fifteen green rungs passed over `ProfilerPlugin` never being added"), in the subsystem
//!   whose reachability gate named that pattern and then gated only `boot`/`enable`.
//! * **`SinkMode::Scheduled` was a dead datum** — the sixth instance of the campaign's signature
//!   class. `boot` recorded the mode into a byte and NOTHING read it back: the "bounded per-frame
//!   drain" in the preset table was a row describing a behaviour nobody implemented. A
//!   `shipping-min` host opened its text file at `enable` and then never moved one record into
//!   it — the header included — for the whole life of the process.
//! * **`flush()`'s thread-less path bypassed every sink**: it hand-rolled a lane walk that
//!   rendered to the console oracle only — pre-L14 code that never learned the sinks exist. For
//!   this preset that console is OFF, so a `flush` "delivered" records to a destination the
//!   preset had disabled and the file stayed empty.
//! * **Thread-less `shutdown()` dropped the lane tail** — no final drain before the file closed.
//!
//! # Why the test runs frames instead of calling `drain()` itself
//!
//! A test that drains by hand proves the drain works, not that anyone drains. The hole in all
//! four defects is between the green gates and the shipped path, so the delivery here goes
//! through the SCHEDULE — `log_drain_system`, registered by `LogPlugin` the way a host registers
//! it — and never through a hand call.
//!
//! # Why the frames run in a second, minimal world
//!
//! `app.update()` on a bare-built `EnginePlugins` panics: the runner, not `build`, inserts the
//! world residents (`Assets<Material>` first among them), so the full host's schedule cannot run
//! without the window loop. The host app therefore carries the BUILD claims — the env route
//! enabled, the preset recorded, `LogPlugin` present — and a second `App` carrying only
//! `LogPlugin` runs the frames. The lifecycle is process-global: that second schedule's
//! `log_drain_system` consumes exactly the sinks and mode the HOST configured, so the only
//! substitution is which schedule instance ticks, not the path a record takes.
//!
//! Its own binary because `EnginePlugins` cannot be built twice in one process and the
//! environment is process-global — same as every other `log_host_*` claim.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_ecs::ecs::core::log::LogRing;
use boyko_log::lifecycle::{SinkState, state};
use boyko_log::preset::LogRuntimePreset;
use boyko_log::{Log, info};

#[test]
fn shipping_min_delivers_to_its_file_from_the_frame_loop() {
    let file = std::env::temp_dir().join("boyko_host_shipmin.log");
    let _ = std::fs::remove_file(&file);

    // SAFETY-of-intent: this binary owns the process environment; it has one test.
    unsafe {
        std::env::set_var("BOYKO_LOG_PRESET", "shipping-min");
        std::env::set_var("BOYKO_LOG_FILE", file.to_str().expect("a UTF-8 temp path"));
    }
    assert_eq!(state(), SinkState::NotBooted, "nothing may touch the lifecycle before the host");

    let mut app = App::new();
    app.add_plugin(EnginePlugins::window("shipping-min gate", 320, 240));

    assert_eq!(state(), SinkState::Enabled, "the preset route must enable, not merely boot");
    assert_eq!(
        boyko_log::lifecycle::boot_preset_recorded(),
        Some(LogRuntimePreset::ShippingMin),
        "the preset must be recorded, or the header cannot name it"
    );
    // The root defect: the seam's plugin was registered by NOTHING. The resource is the proof of
    // registration — `LogPlugin::build` is what inserts it.
    assert!(
        app.world().try_resource::<LogRing>().is_some(),
        "no host adds LogPlugin: the L5 seam exists only in its own tests — the ProfilerPlugin \
         hole again, in the subsystem whose reachability gate cited it"
    );

    // ── THE PRESET'S PROMISE, OBSERVED FROM THE FRAME LOOP ──────────────────────────────────
    //
    // No resident thread exists under `Scheduled`, so if these records reach the file, the
    // in-frame drain moved them. Two frames, not one: the drain system carries no edge against
    // the emitters, so a record from after its slot lands in the next frame's pass.
    let mut frames = App::new();
    frames.add_plugin(boyko_ecs::ecs::core::log::LogPlugin);
    for _ in 0..2 {
        frames.update();
    }
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(
        text.contains("runtime_preset=shipping-min"),
        "the session header never reached the preset's own file; with no per-frame drain the \
         file stays empty for the life of the process: {text:?}"
    );

    info!(Log, "a shipping-min record {}", 7u32);
    for _ in 0..2 {
        frames.update();
    }
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(
        text.contains("a shipping-min record 7"),
        "a record emitted between frames must be in the file within two frames: {text:?}"
    );

    // ── `flush()` DELIVERS THROUGH THE SINKS, NOT PAST THEM ─────────────────────────────────
    //
    // No resident thread exists here, so this takes `flush`'s inline arm — the one that used to
    // hand-roll a lane walk rendering to the console oracle only. Under this preset that console
    // is OFF: before the fix this record was "flushed" to a destination the preset had disabled
    // and never appeared in the file.
    info!(Log, "a flushed record {}", 8u32);
    assert_eq!(
        boyko_log::lifecycle::flush(),
        boyko_log::lifecycle::FlushResult::Flushed,
        "a thread-less flush has nothing to time out on"
    );
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(
        text.contains("a flushed record 8"),
        "flush()'s inline arm bypassed the sinks: {text:?}"
    );

    // ── AND THE TAIL SURVIVES SHUTDOWN ──────────────────────────────────────────────────────
    //
    // Emitted after the last frame's drain, so only `shutdown`'s own final pass can deliver it.
    // Before the fix the thread-less arm stored `Exited` and returned — the lanes still held the
    // record and the file was already closed.
    info!(Log, "the tail record {}", 11u32);
    assert!(boyko_log::lifecycle::shutdown(), "a thread-less shutdown has nothing to time out on");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(
        text.contains("the tail record 11"),
        "the lane tail was dropped at shutdown: {text:?}"
    );

    // ── AND THE FILE CARRIES ITS OWN LOSS REPORT ────────────────────────────────────────────
    //
    // The census used to go through the synchronous console channel alone, and this preset's
    // console is OFF: the uploaded log of a released title could not say whether it lost
    // anything — the first question its reader asks. Ring-borne now (owner-directed), delivered
    // by `shutdown`'s final pass, closed after.
    assert!(
        text.contains("LOG-CENSUS "),
        "the census never reached the preset's own file; a shipping log that cannot say whether \
         it lost records is silent about the one thing a reader must know: {text:?}"
    );
    assert!(
        text.contains("LOG-CENSUS limiter"),
        "the limiter line is part of the census and travels with it: {text:?}"
    );

    let _ = std::fs::remove_file(&file);
}
