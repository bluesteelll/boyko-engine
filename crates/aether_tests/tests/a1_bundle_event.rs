//! Rung A1 end-to-end: an `aether!` block's `bundle` spawns through the derive's own path, and
//! its `event` rides the REAL kernel event lanes — written by an `EventWriter`, read by an
//! `EventReader`, through an `App` frame (the plan's A1 gate verbatim).
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

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use boyko_ecs::App;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::system::{EventReader, EventWriter, ResMut};


use aether::aether;

/// The kernel's maximum event-lane count (`EventConfig` validates `1..=64`). Sizing for it
/// makes lane selection independent of how wide THIS machine's worker pool is.
const MAX_EVENT_LANES: u32 = 64;

aether! {
    component Position {
        x: f32,
    }

    component Health {
        hp: f32,
    }

    bundle Pawn {
        pos: Position,
        health: Health,
    }

    event Damage {
        victim: entity(Position, Health),
        amount: f32,
    }
}

/// One-shot send gate — the writer system fires exactly once, then the reader sums.
#[derive(boyko_macros::Resource)]
struct SendOnce(bool);

#[test]
fn aether_event_rides_the_kernel_lanes_through_an_app() {
    let seen = Arc::new(AtomicU32::new(0));
    let s = Arc::clone(&seen);

    let mut app = App::new();
    app.world_mut()
        .preregister_event::<Damage>(EventConfig::default_for(MAX_EVENT_LANES).expect("config"))
        .expect("preregister");
    app.world_mut().insert_resource(SendOnce(true));

    // The participant slot wants a real Entity — the bundle half of the rung provides it: a
    // `Pawn` spawned through the derive's own Bundle path (Commands::spawn), proving the two
    // constructs compose the way a game would use them.
    app.add_systems(
        move |mut once: ResMut<SendOnce>,
              mut w: EventWriter<Damage>,
              mut cmds: boyko_ecs::ecs::core::system::Commands| {
            if once.0 {
                once.0 = false;
                let victim = cmds
                    .spawn(Pawn { pos: Position { x: 1.0 }, health: Health { hp: 9.5 } })
                    .id();
                // The #[event] macro's two-band rewrite: the surface fields land in the
                // generated `<Name>Participants` / `<Name>Parameters` substructs — the SAME
                // construction every hand-written #[event] user performs (Decision A3: Aether
                // emits the canonical surface, so it inherits the canonical construction too).
                w.send(Damage {
                    participants: DamageParticipants { victim },
                    parameters: DamageParameters { amount: 2.5 },
                })
                .expect("send within lane capacity");
            }
        },
    );
    app.add_systems(move |mut r: EventReader<Damage>| {
        for e in r.read() {
            // The parameter band survived the two-band rewrite with its value intact.
            assert_eq!(e.parameters.amount, 2.5, "the parameter field carries its payload");
            s.fetch_add(1, Ordering::Relaxed);
        }
    });

    // Two updates: frame 1 sends (the reader may or may not see it the same frame, depending on
    // system order); frame 2 guarantees delivery — the kernel's own one-frame bound.
    app.update();
    app.update();

    assert_eq!(seen.load(Ordering::Relaxed), 1, "exactly one Damage event was read");
}
