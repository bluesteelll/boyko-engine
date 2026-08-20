//! Rung A3 end-to-end: an `aether!` `machine` rides the REAL kernel state machinery — the
//! plugin's `insert_state`, transition systems gated by `run_if(in_state(leaf))`, `NextState`
//! applied by the engine's own state pass, and the LCA-inlined `enter` action observed through
//! a resource (the plan's A3 gate: not just expansion, execution).
//!
//! The two Playing leaves transition on DIFFERENT event types deliberately: a `run_if`-disabled
//! system's `EventReader` cursor does not advance, so a shared event type would let the freshly
//! entered leaf read the SAME event from its backlog and bounce straight back — a semantic
//! hazard this test sidesteps (and documents) rather than depends on.

use aether::aether;
use boyko_ecs::App;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::state::State;

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
        .preregister_event::<AssetsReady>(EventConfig::default_for(2).expect("config"))
        .expect("preregister");
    app.world_mut()
        .preregister_event::<PauseOn>(EventConfig::default_for(2).expect("config"))
        .expect("preregister");
    app.world_mut()
        .preregister_event::<PauseOff>(EventConfig::default_for(2).expect("config"))
        .expect("preregister");
    app.insert_resource(Script { frame: 0, entered_playing: 0 });
    app.add_plugin(Flow);

    // Frame 1 sends AssetsReady (Boot → Playing.Running via the composite's `initial`);
    // frame 3 sends PauseOn (Running → Paused). Six frames comfortably cover both events'
    // one-frame delivery bounds plus the state passes that apply the transitions.
    for _ in 0..6 {
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
    assert_eq!(script.entered_playing, 1, "the Playing `enter` action ran exactly once");
}
