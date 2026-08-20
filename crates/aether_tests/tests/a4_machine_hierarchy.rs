//! Rung A4 end-to-end: the §3.5 `GameFlow` chart itself, and the §5.3 initial-enter chain,
//! executing on a REAL `App` — hierarchy is a transpiler-only concept, so the only honest proof
//! that the flattening is CORRECT is running the chart the plan draws.
//!
//! What the first test can observe that a snapshot cannot:
//!
//! * `Boot --AssetsReady--> Playing` retargets through the composite's `initial` to the
//!   `Playing.Running` LEAF, and runs `Playing`'s `enter` on the way in;
//! * the `Running <-> Paused` toggle rides ONE shared event type in both directions — and
//!   `Playing`'s `enter` still fires exactly once across the whole run, because both leaves sit
//!   under `Playing` so the LCA excludes it. A flattening that re-entered the target's whole
//!   lineage would show `entered_playing == 3` here;
//! * the superstate `PlayerDied` handler is inherited by the leaf that never declared it, and
//!   its guard blocks the transition while `lives != 0`;
//! * leaving `Playing` for `GameOver` runs `Playing`'s `exit` — an action declared two levels
//!   above the leaf that is actually current.
//!
//! Two concretizations of the plan's chart, both forced by the engine and neither semantic:
//! the plan's event types are fieldless and this kernel refuses ZST events at compile time (so
//! each carries a `tick`), and the plan's `exit` body is the comment `/* tear down */` — the
//! kernel has no `Entity` query-data form, so a `query<&HudRoot>` cannot despawn what it finds;
//! the body counts the HUD roots it would tear down instead, which is the observable half.
//!
//! Timing note (the A3 hazard, measured rather than assumed): a `run_if`-disabled system's
//! `EventReader` cursor does not advance, so the freshly entered leaf's reader is stale. It
//! cannot bounce here because the reader window is exactly ONE swap wide — the event that drove
//! the transition has already left `reader_buf` by the frame the new leaf first runs. The
//! driver therefore spaces presses two frames apart, which is the documented v1 requirement,
//! not a workaround for this test.
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
    event PausePressed { tick: u32, }
    event PlayerDied { tick: u32, }
    event RestartPressed { tick: u32, }

    tag HudRoot;

    plugin Flow;

    system driver(s: mut res<Script>, score: mut res<Score>,
                  ar: emit<AssetsReady>, pp: emit<PausePressed>,
                  pd: emit<PlayerDied>, rp: emit<RestartPressed>) on update {
        s.frame += 1;
        match s.frame {
            1 => {
                ar.send(AssetsReady {
                    participants: AssetsReadyParticipants {},
                    parameters: AssetsReadyParameters { tick: 1 },
                })
                .expect("send within lane capacity");
            }
            3 | 5 => {
                pp.send(PausePressed {
                    participants: PausePressedParticipants {},
                    parameters: PausePressedParameters { tick: s.frame },
                })
                .expect("send within lane capacity");
            }
            7 => {
                // `lives` is still 1 here: the inherited guard must REFUSE this one.
                pd.send(PlayerDied {
                    participants: PlayerDiedParticipants {},
                    parameters: PlayerDiedParameters { tick: 7 },
                })
                .expect("send within lane capacity");
            }
            9 => {
                score.lives = 0;
                pd.send(PlayerDied {
                    participants: PlayerDiedParticipants {},
                    parameters: PlayerDiedParameters { tick: 9 },
                })
                .expect("send within lane capacity");
            }
            11 => {
                rp.send(RestartPressed {
                    participants: RestartPressedParticipants {},
                    parameters: RestartPressedParameters { tick: 11 },
                })
                .expect("send within lane capacity");
            }
            _ => {}
        }
    }

    machine GameFlow {
        initial Boot;

        state Boot {
            on AssetsReady => Playing;
        }

        state Playing {
            initial Running;
            enter (mut cmds: commands, probe: mut res<Probe>) {
                cmds.spawn(HudRoot);
                probe.entered_playing += 1;
            }
            exit (mut cmds: commands, huds: query<&HudRoot>, probe: mut res<Probe>) {
                let mut n = 0u32;
                for _ in &huds {
                    n += 1;
                }
                probe.huds_at_exit = n;
                probe.exited_playing += 1;
                let _ = &mut cmds;
            }

            state Running {
                on PausePressed => Playing.Paused;
            }
            state Paused {
                on PausePressed => Playing.Running;
            }

            on PlayerDied (score: res<Score>) if score.lives == 0 => GameOver {
                probe.deaths += 1;
            }
        }

        state GameOver {
            on RestartPressed => Boot;
        }
    }
}

/// The frame script — aether systems are plain fns, so a resource is the only honest driver.
#[derive(boyko_macros::Resource)]
struct Script {
    frame: u32,
}

/// The guard's input, mutated by the driver so the SAME event type is refused once and accepted
/// once — proving the guard is evaluated per event, not per registration.
#[derive(boyko_macros::Resource)]
struct Score {
    lives: u32,
}

/// The observation channel for the inlined enter/exit/action bodies.
#[derive(boyko_macros::Resource)]
struct Probe {
    entered_playing: u32,
    exited_playing: u32,
    huds_at_exit: u32,
    deaths: u32,
}

#[test]
fn the_section_3_5_gameflow_chart_runs_on_a_real_app() {
    let mut app = App::new();
    app.world_mut()
        .preregister_event::<AssetsReady>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.world_mut()
        .preregister_event::<PausePressed>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.world_mut()
        .preregister_event::<PlayerDied>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.world_mut()
        .preregister_event::<RestartPressed>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.insert_resource(Script { frame: 0 });
    app.insert_resource(Score { lives: 1 });
    app.insert_resource(Probe {
        entered_playing: 0,
        exited_playing: 0,
        huds_at_exit: 0,
        deaths: 0,
    });
    app.add_plugin(Flow);

    // Each leg is two frames: one for the transition system to read the event and write
    // `NextState::Pending`, one for the engine's state pass to apply it.
    run(&mut app, 3);
    assert_eq!(
        state(&mut app),
        GameFlow::PlayingRunning,
        "`=> Playing` retargeted through the composite's `initial` to the Running LEAF"
    );
    assert_eq!(probe(&mut app).entered_playing, 1, "`Playing`'s enter ran on the way in");

    run(&mut app, 2);
    assert_eq!(state(&mut app), GameFlow::PlayingPaused, "Running -PausePressed-> Paused");

    run(&mut app, 2);
    assert_eq!(
        state(&mut app),
        GameFlow::PlayingRunning,
        "Paused -PausePressed-> Running: the SAME event type drives both directions"
    );
    assert_eq!(
        probe(&mut app).entered_playing,
        1,
        "both toggles stayed under `Playing`, so the LCA excluded its enter — a lineage-wide \
         re-enter would read 3 here"
    );

    run(&mut app, 2);
    assert_eq!(
        state(&mut app),
        GameFlow::PlayingRunning,
        "the inherited `PlayerDied` handler fired, and its guard refused the transition"
    );
    assert_eq!(probe(&mut app).deaths, 0, "a refused guard runs no action block");

    run(&mut app, 2);
    assert_eq!(
        state(&mut app),
        GameFlow::GameOver,
        "the superstate handler is inherited by `Playing.Running`, which never declared it"
    );
    let p = probe(&mut app);
    assert_eq!(p.exited_playing, 1, "leaving the composite ran `Playing`'s exit exactly once");
    assert_eq!(p.huds_at_exit, 1, "the exit body saw the HUD root its own enter spawned");
    assert_eq!(p.deaths, 1, "the action block ran after the exit, on the accepting frame");

    run(&mut app, 2);
    let flow = state(&mut app);
    assert_eq!(flow, GameFlow::Boot, "GameOver -RestartPressed-> Boot closes the chart");
    assert!(!flow.in_playing(), "the flattened superstate predicate excludes `Boot`");
    assert_eq!(
        probe(&mut app).entered_playing,
        1,
        "one entry into `Playing` across the whole chart"
    );
}

aether! {
    event Go { tick: u32, }
    event Stop { tick: u32, }

    tag Ground;

    plugin BootFlow;

    system sim_driver(s: mut res<Script>, go: emit<Go>, stop: emit<Stop>) on update {
        s.frame += 1;
        if s.frame == 1 {
            go.send(Go {
                participants: GoParticipants {},
                parameters: GoParameters { tick: 1 },
            })
            .expect("send within lane capacity");
        }
        if s.frame == 3 {
            stop.send(Stop {
                participants: StopParticipants {},
                parameters: StopParameters { tick: 3 },
            })
            .expect("send within lane capacity");
        }
    }

    machine Sim {
        initial World;

        state World {
            initial Field;
            enter (mut cmds: commands, log: mut res<BootProbe>) {
                cmds.spawn(Ground);
                log.world_entered += 1;
            }

            state Field {
                initial Idle;
                enter (log: mut res<BootProbe>) { log.field_entered += 1; }

                state Idle {
                    enter (log: mut res<BootProbe>) { log.idle_entered += 1; }
                    on Go => World.Field.Busy;
                }
                state Busy {
                    on Stop => World.Field.Idle;
                }
            }
        }
    }
}

/// The initial-enter chain's observation channel.
#[derive(boyko_macros::Resource)]
struct BootProbe {
    world_entered: u32,
    field_entered: u32,
    idle_entered: u32,
}

#[test]
fn the_section_5_3_initial_enter_chain_runs_the_whole_ancestor_path_once() {
    let mut app = App::new();
    app.world_mut()
        .preregister_event::<Go>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.world_mut()
        .preregister_event::<Stop>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.insert_resource(Script { frame: 0 });
    app.insert_resource(BootProbe { world_entered: 0, field_entered: 0, idle_entered: 0 });
    app.add_plugin(BootFlow);

    // `insert_state` seeds `World.Field.Idle`; nothing in the kernel runs an entry action for a
    // state nobody transitioned into, so the §5.3 startup system is what makes the three
    // ancestor `enter` bodies run at all.
    run(&mut app, 1);
    let p = boot_probe(&mut app);
    assert_eq!(p.world_entered, 1, "the outermost ancestor's enter ran");
    assert_eq!(p.field_entered, 1, "the middle ancestor's enter ran");
    assert_eq!(p.idle_entered, 1, "the initial LEAF's own enter ran");

    // Idle -Go-> Busy -Stop-> Idle: the second entry into `Idle` must NOT replay the ancestors,
    // because both leaves share them — the LCA bound, observed rather than pinned.
    run(&mut app, 4);
    let p = boot_probe(&mut app);
    assert_eq!(p.world_entered, 1, "an intra-`Field` transition never re-enters `World`");
    assert_eq!(p.field_entered, 1, "…nor `Field`");
    assert_eq!(p.idle_entered, 2, "…but it does re-enter the leaf it lands on");
    assert_eq!(
        *app.world_mut().resource::<State<Sim>>().get(),
        Sim::WorldFieldIdle,
        "the round trip closed on the leaf it started from"
    );
}

aether! {
    event Tick { n: u32, }

    plugin DrainFlow;

    system drain_driver(s: mut res<Script>, tx: emit<Tick>) on update {
        s.frame += 1;
        if s.frame == 1 {
            // TWO events in ONE frame — the exact shape §5.1 arbitrates.
            tx.send(Tick { participants: TickParticipants {}, parameters: TickParameters { n: 1 } })
                .expect("send within lane capacity");
            tx.send(Tick { participants: TickParticipants {}, parameters: TickParameters { n: 2 } })
                .expect("send within lane capacity");
        }
    }

    machine Pulse {
        initial A;

        state A {
            on Tick (p: mut res<DrainProbe>) => A {
                p.transitions += 1;
            }
        }
    }
}

/// Counts the transitions the self-targeting handler actually performs.
#[derive(boyko_macros::Resource)]
struct DrainProbe {
    transitions: u32,
}

#[test]
fn two_same_frame_events_produce_exactly_one_transition() {
    let mut app = App::new();
    app.world_mut()
        .preregister_event::<Tick>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.insert_resource(Script { frame: 0 });
    app.insert_resource(DrainProbe { transitions: 0 });
    app.add_plugin(DrainFlow);

    run(&mut app, 4);

    // §5.1: "one transition per machine per frame" — the events after the accepted one are
    // observed and DISCARDED. The emitted body therefore drains the reader fully and acts once
    // afterwards. The `return`-in-loop shape §3.5 sketches leaves the second event unread (the
    // kernel's `EventIter` advances the cursor only past what it yielded), and this
    // self-targeting handler stays enabled, so the next frame re-reads it and fires again —
    // this assertion reads 2 under that shape.
    assert_eq!(
        app.world_mut().resource::<DrainProbe>().transitions,
        1,
        "two events in one frame accept ONE transition, and the remainder is discarded rather \
         than carried into the next frame"
    );
}

/// Advance `n` frames.
fn run(app: &mut App, n: u32) {
    for _ in 0..n {
        app.update();
    }
}

/// The machine's current leaf.
fn state(app: &mut App) -> GameFlow {
    *app.world_mut().resource::<State<GameFlow>>().get()
}

fn probe(app: &mut App) -> &Probe {
    app.world_mut().resource::<Probe>()
}

fn boot_probe(app: &mut App) -> &BootProbe {
    app.world_mut().resource::<BootProbe>()
}
