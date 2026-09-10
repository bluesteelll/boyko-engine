//! Rung R2's oracle: **two events on one leaf in one frame run exactly ONE exit/action/enter
//! chain**, and the one that runs is the **first declared** (v2 DECISIONS M6).
//!
//! This is the arbitration axis the A4 suite never reached. `a4_machine_hierarchy.rs`'s
//! `two_same_frame_events_produce_exactly_one_transition` pins two events of the SAME type on one
//! leaf — one `EventReader`, one generated system, one drain, and the drain-then-act shape
//! already answers it. Two events of DIFFERENT types on one leaf are two `EventReader`s, and
//! under the pre-R2 emission they are two INDEPENDENT systems, each gated
//! `run_if(in_state(Leaf))`:
//!
//! * `in_state` reads `State<M>`, which the kernel swaps only at the state-transition apply
//!   point — so within the accepting frame BOTH gates are open;
//! * each system runs its own exit → action → enter chain and then writes
//!   `NextState::Pending(target)`, so the leaf is exited twice, two action blocks run, and two
//!   different targets are entered;
//! * the surviving state is whichever system the scheduler ran LAST — last-write-wins, which M6
//!   calls an artifact of one-system-per-route registration rather than a chosen semantics.
//!
//! The fix is the per-leaf **route merge**: one generated system per leaf, draining every event
//! lane the leaf inherits, selecting the first-declared accepting route, and running exactly one
//! chain. Both halves of M6 fall out of the same merge, which is why they ship together.
//!
//! Event-lane sizing follows `a4_machine_hierarchy.rs`'s measured note verbatim: lanes are sized
//! for the kernel maximum, never a hard-coded small count, because `EventWriter::send` picks its
//! lane by worker id and an under-sized config trips a `debug_assert` ON A WORKER THREAD, which
//! this harness surfaces as a HANG rather than a failure.

use aether::aether;
use boyko_ecs::App;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::state::State;

/// One event lane per pool worker (`boyko_threadpool::MAX_WORKERS` == 64). The kernel
/// ceiling is `MAX_EVENT_THREADS` == 65 since KE8 — one more, reserved for a
/// non-worker sender — so `EventConfig` validates `1..=65`.
const MAX_EVENT_LANES: u32 = 64;

aether! {
    event Alpha { tick: u32, }
    event Beta { tick: u32, }

    plugin DuelFlow;

    system duel_driver(s: mut res<DuelScript>, a: emit<Alpha>, b: emit<Beta>) on update {
        s.frame += 1;
        if s.frame == 1 {
            // ONE frame, TWO different event types, both handled by the SAME leaf.
            a.send(Alpha {
                participants: AlphaParticipants {},
                parameters: AlphaParameters { tick: 1 },
            })
            .expect("send within lane capacity");
            b.send(Beta {
                participants: BetaParticipants {},
                parameters: BetaParameters { tick: 1 },
            })
            .expect("send within lane capacity");
        }
    }

    machine Duel {
        initial Idle;

        state Idle {
            exit (p: mut res<DuelProbe>) { p.exited_idle += 1; }

            // FIRST declared: M6 says this one wins.
            on Alpha (p: mut res<DuelProbe>) => ToAlpha { p.alpha_action += 1; }
            // Second declared: must not run its chain at all on the arbitrated frame.
            on Beta (p: mut res<DuelProbe>) => ToBeta { p.beta_action += 1; }
        }

        state ToAlpha {
            enter (p: mut res<DuelProbe>) { p.entered_alpha += 1; }
        }
        state ToBeta {
            enter (p: mut res<DuelProbe>) { p.entered_beta += 1; }
        }
    }
}

/// The frame script — aether systems are plain fns, so a resource is the only honest driver.
#[derive(boyko_macros::Resource)]
struct DuelScript {
    frame: u32,
}

/// One counter per observable half of a chain, so a failure names WHICH half doubled.
#[derive(boyko_macros::Resource)]
struct DuelProbe {
    exited_idle: u32,
    alpha_action: u32,
    beta_action: u32,
    entered_alpha: u32,
    entered_beta: u32,
}

#[test]
fn two_events_on_one_leaf_in_one_frame_run_exactly_one_chain() {
    let mut app = App::new();
    app.world_mut()
        .preregister_event::<Alpha>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.world_mut()
        .preregister_event::<Beta>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.insert_resource(DuelScript { frame: 0 });
    app.insert_resource(DuelProbe {
        exited_idle: 0,
        alpha_action: 0,
        beta_action: 0,
        entered_alpha: 0,
        entered_beta: 0,
    });
    app.add_plugin(DuelFlow);

    // Frame 1 sends both events; frame 2 lets the transition systems read them and write
    // `NextState`; frame 3 applies it. Two spare frames confirm nothing fires afterwards.
    run(&mut app, 5);

    let p = app.world_mut().resource::<DuelProbe>();
    let (exited, aa, ba, ea, eb) =
        (p.exited_idle, p.alpha_action, p.beta_action, p.entered_alpha, p.entered_beta);

    assert_eq!(
        exited, 1,
        "the leaf's `exit` ran ONCE for the frame — two independent per-route systems run it twice"
    );
    assert_eq!(aa + ba, 1, "exactly ONE action block ran across both routes");
    assert_eq!(ea + eb, 1, "exactly ONE target's `enter` ran across both routes");

    // M6: first declared wins. The pre-R2 emission arbitrates by registration order instead.
    assert_eq!(aa, 1, "the FIRST-declared route (`on Alpha`) is the one that ran");
    assert_eq!(ba, 0, "the second-declared route (`on Beta`) did not run its action");
    assert_eq!(ea, 1, "…and its target is the one entered");
    assert_eq!(eb, 0, "…while the loser's target was never entered");
    assert_eq!(
        *app.world_mut().resource::<State<Duel>>().get(),
        Duel::ToAlpha,
        "the machine settled on the first-declared route's target"
    );
}

/// Advance `n` frames.
fn run(app: &mut App, n: u32) {
    for _ in 0..n {
        app.update();
    }
}
