//! Rung A3 end-to-end: an `aether!` `machine` rides the REAL kernel state machinery — the
//! plugin's `insert_state`, transition systems gated by `run_if(in_state(leaf))`, `NextState`
//! applied by the engine's own state pass, and the LCA-inlined `enter` action observed through
//! a resource (the plan's A3 gate: not just expansion, execution).
//!
//! The two Playing leaves transition on DIFFERENT event types deliberately: a `run_if`-disabled
//! system's `EventReader` cursor does not advance, so a shared event type would let the freshly
//! entered leaf read the SAME event from its backlog and bounce straight back — a semantic
//! hazard this test sidesteps (and documents) rather than depends on.
//!
//! ⚠️ Event lanes are sized for `MAX_EVENT_LANES`, not for a hard-coded 2, and that is
//! load-bearing. `EventConfig::default_for`'s argument is the WORKER-LANE COUNT, and
//! `EventWriter::send` picks its lane as `current_worker_id_or_dispatcher_lane(thread_count-1)`
//! — i.e. the id of whatever worker the scheduler put the sending system on. On a pool wider
//! than the configured lane count, `EventBuffer::send_one`'s
//! `debug_assert!(thread_index < thread_count)` trips ON A WORKER THREAD, which the test harness
//! surfaces as an infinite HANG, not a failure. A hard-coded `2` therefore passes or hangs by
//! scheduling luck; MEASURED, it hung here the moment an unrelated edit perturbed placement, on
//! `boyko-worker-4`. `preregister_event_default` is NOT the fix either — the dispatcher's
//! default is `EventDispatcher::new(1)`, i.e. ONE lane. Nothing in the public App surface
//! reports the pool width, so a test sizes for the kernel's maximum.


use aether::aether;
use boyko_ecs::App;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::state::State;

/// The kernel's maximum event-lane count (`EventConfig` validates `1..=64`). Sizing for it
/// makes lane selection independent of how wide THIS machine's worker pool is.
const MAX_EVENT_LANES: u32 = 64;

aether! {
    event AssetsReady { tick: u32, }
    event PauseOn { tick: u32, }

    plugin Flow;

    system driver(n: mut res<Script>, ar: emit<AssetsReady>, po: emit<PauseOn>) on update {
        n.frame += 1;
        if n.frame == 1 {
            ar.send(AssetsReady {
                participants: AssetsReadyParticipants {},
                parameters: AssetsReadyParameters { tick: 1 },
            })
            .expect("send within lane capacity");
        }
        if n.frame == 3 {
            po.send(PauseOn {
                participants: PauseOnParticipants {},
                parameters: PauseOnParameters { tick: 3 },
            })
            .expect("send within lane capacity");
        }
    }

    machine GameFlow {
        initial Boot;

        state Boot {
            on AssetsReady => Playing;
        }

        state Playing {
            initial Running;
            enter (log: mut res<Script>) { log.entered_playing += 1; }

            state Running {
                on PauseOn => Playing.Paused;
            }
            state Paused {
                on PauseOff => Playing.Running;
            }
        }
    }

    event PauseOff { tick: u32, }
}

/// The observation channel — machine actions are plain inlined Rust, so a resource is the
/// honest way to see them run.
#[derive(boyko_macros::Resource)]
struct Script {
    frame: u32,
    entered_playing: u32,
}

#[test]
fn machine_transitions_ride_real_events_and_state() {
    let mut app = App::new();
    app.world_mut()
        .preregister_event::<AssetsReady>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.world_mut()
        .preregister_event::<PauseOn>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.world_mut()
        .preregister_event::<PauseOff>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.insert_resource(Script { frame: 0, entered_playing: 0 });
    app.add_plugin(Flow);

    // The two hops are asserted SEPARATELY, and that is the point. Checking only the final
    // `PlayingPaused` cannot see the middle state: a composite-`initial` regression that landed
    // Boot directly in `Playing.Paused` would satisfy every end-state assertion while the
    // `PauseOn` edge never fired at all. Frame 1 sends AssetsReady; three frames cover its
    // one-frame delivery plus the state pass that applies the transition.
    for _ in 0..3 {
        app.update();
    }
    let mid = *app.world_mut().resource::<State<GameFlow>>().get();
    assert_eq!(
        mid,
        GameFlow::PlayingRunning,
        "AssetsReady targets the COMPOSITE `Playing`, which retargets through `initial Running` \
         to the Running leaf — not to Paused, and not to Playing itself"
    );
    assert!(mid.in_playing(), "the flattened superstate predicate holds for the Running leaf");
    assert_eq!(
        app.world_mut().resource::<Script>().entered_playing,
        1,
        "the Playing `enter` action ran on the way in"
    );

    // Frame 3 sent PauseOn; three more frames cover its delivery and state pass. Reaching
    // `PlayingPaused` from a state asserted to be `PlayingRunning` is a change only the
    // `Running -PauseOn-> Playing.Paused` edge can make.
    for _ in 0..3 {
        app.update();
    }

    let flow = *app.world_mut().resource::<State<GameFlow>>().get();
    assert_eq!(
        flow,
        GameFlow::PlayingPaused,
        "Boot -AssetsReady-> Playing.Running -PauseOn-> Playing.Paused"
    );
    assert!(flow.in_playing(), "the flattened superstate predicate holds for the Paused leaf");
    let script = app.world_mut().resource::<Script>();
    assert_eq!(
        script.entered_playing, 1,
        "the Playing `enter` action ran exactly once — the Running→Paused hop stays under \
         `Playing`, so the LCA excludes its enter"
    );
}
