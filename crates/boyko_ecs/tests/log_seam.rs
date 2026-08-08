//! Rung L5's gate — the ECS logging seam, end to end.
//!
//! # One test, and the reason it is one test
//!
//! Everything this file touches is process-global: the lifecycle state, the target control array,
//! the lanes, the drain token, the handoff ring. Two `#[test]` functions in one binary run on two
//! threads with no ordering between them, so "the ring was not materialized" and "the ring now
//! holds a line" would be a coin flip rather than an assertion. A process-global scenario is one
//! test or it is flaky by construction — this campaign has paid for that lesson four times.
//!
//! The sequence is deliberate: the flag-off assertions come FIRST, because once `boot` has
//! recorded `ecs_ring: true` there is no way back to a process that never asked for it.

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::log::{LogPlugin, LogRing, LogStats};
use boyko_log::Ecs;
use boyko_log::level::Level;
use boyko_log::lifecycle::{LogConfig, boot, drain_once, enable, shutdown};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};

#[test]
fn the_seam_carries_a_record_and_costs_nothing_until_it_does() {
    // ── 1. `build` reserves nothing ──────────────────────────────────────────────────────────
    //
    // The plugin runs before any launch flag is read, so it may not make a syscall the flag has
    // not authorised. `VmColumn` is lazy by construction; this asserts the plugin did not defeat
    // that by pre-growing.
    let mut app = App::new();
    app.add_plugin(LogPlugin);
    app.finish();
    assert!(
        !app.world().resource::<LogRing>().is_materialized(),
        "LogPlugin::build reserved or committed the ring's columns"
    );

    // ── 2. With the flag off, a whole frame touches nothing ──────────────────────────────────
    //
    // MEASURED, AND NOT WHAT IT LOOKS LIKE: these three assertions do **not** discriminate the
    // drain's flag check. Deleting `if !ecs_ring_enabled() { return; }` from `log_drain_system`
    // and re-running this test leaves it GREEN — because at this rung the system's only duty is
    // consuming the handoff ring, and an empty ring is a no-op with or without the check. The
    // check is nevertheless correct and load-bearing: L16 gives the system two duties it performs
    // on its OWN account (the `TARGET_STATS` snapshot and the per-frame `frame_epoch` record), and
    // those materialize the columns on frame 1 in a process that never enabled logging. It is
    // written now so the hole is not left for a later rung to fall into.
    //
    // **L16 obligation**: when `frame_epoch` lands, delete the check and confirm THIS assertion
    // reds. Until then the check is present and argued, not verified — which is the honest label
    // and is why it is written here rather than left to read as tested.
    app.update();
    assert!(
        !app.world().resource::<LogRing>().is_materialized(),
        "the flag-off drain materialized the ring"
    );
    assert_eq!(app.world().resource::<LogRing>().cursor(), 0);
    assert_eq!(
        boyko_log::sink::ecs::published(),
        0,
        "the flag-off drain published into the handoff ring"
    );

    // ── 3. Turn it on, and emit exactly one record ───────────────────────────────────────────
    boot(LogConfig { console: false, sink_thread: false, ecs_ring: true, ..LogConfig::default() });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Ecs as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));

    boyko_log::info!(Ecs, "seam probe {}", 7u32);

    // The consumer role, taken by hand rather than by a thread: a sink thread would make this
    // test wait on an adaptive park, and what is being checked is the transport, not the park.
    let drained = drain_once().expect("the drain role is free in a process with no sink thread");
    assert_eq!(drained.records, 1, "the lane did not carry the record");
    assert!(
        boyko_log::sink::ecs::published() > 0,
        "the consumer role drained the lane but did not feed the handoff ring"
    );

    // ── 4. The frame that copies it out ──────────────────────────────────────────────────────
    app.update();

    let ring = app.world().resource::<LogRing>();
    assert!(ring.is_materialized(), "the drain carried a line but grew no column");
    assert_eq!(ring.cursor(), 1, "exactly one line should have been stored");
    assert_eq!(ring.len(), 1);

    let (line, text) = ring.line(0).expect("line 0 is live");
    assert_eq!(line.level, Level::Info as u8);
    assert_eq!(line.target, <Ecs as LogTarget>::ID.index() as u8);
    assert_eq!(line.seq_lo, 0, "the first line's sequence is 0");
    assert_eq!(line.len as usize, text.len());
    let text = std::str::from_utf8(text).expect("the sink renders ASCII");
    assert!(text.contains("seam probe"), "the stored text is not the record's: {text:?}");

    assert_eq!(
        app.world().resource::<LogStats>().handoff_lost,
        0,
        "nothing may be lost with one record in a 256 KiB ring"
    );

    // ── 5. A second frame with nothing to carry stores nothing ───────────────────────────────
    //
    // The drain is not a heartbeat: an empty pass must not advance the cursor. If it did, a
    // reader's `since(cursor)` would report lines that do not exist — and at L16 that is exactly
    // the confusion the `frame_epoch` record has to be distinguishable from.
    app.update();
    assert_eq!(app.world().resource::<LogRing>().cursor(), 1, "an empty drain stored a line");

    // ── teardown ─────────────────────────────────────────────────────────────────────────────
    //
    // The ring's own wrap and eviction arithmetic is NOT exercised here: `LogRing::store` is
    // `pub(crate)` on purpose — a public one would let anything write the ring the seam claims is
    // fed only by `log_drain_system` — so that half is a unit test beside the code, in `ring.rs`.
    set_target_control(<Ecs as LogTarget>::ID, TargetControl::OFF);
    shutdown();
}
