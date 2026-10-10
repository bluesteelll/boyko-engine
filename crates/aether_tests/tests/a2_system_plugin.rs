//! Rung A2 end-to-end: an `aether!` block's `plugin` registers REAL systems on a REAL `App` —
//! startup one-shot spawning through `commands`, an `after` sibling edge that must hold every
//! frame, a `when` gate that must hold the gated system shut, and a `query<(&mut …)>` that
//! actually integrates component data (the plan's A2 gate: not just expansion, execution).
//!
//! # Why `check_order` is declared BEFORE `integrate`
//!
//! As first written, the `order_ok` assertion could not fail. The two systems conflict on
//! `mut res<SeqLog>`, and the scheduler serializes conflicting systems in registration order —
//! which, with `check_order` declared second, was already `integrate`-then-`check_order`. The
//! edge and the fallback agreed, so deleting `after integrate` changed nothing and the test
//! stayed green: an assertion about an ordering primitive that never exercised it.
//!
//! Reversing the declaration order makes the fallback `check_order`-then-`integrate`, so only
//! the `after` edge can produce the asserted sequence — the topological emission in
//! `bucket_stmts` registers the edge's TARGET first to capture its `SystemKey`.
//!
//! MEASURED: with the edge deleted, `order_ok` fails on the first frame
//! (`integrate_runs == check_runs + 1` reads `0 == 1`). Restored, the test is green.

use aether::aether;
use boyko_ecs::App;

aether! {
    component Position {
        x: f32,
    }

    component Velocity {
        v: f32,
    }

    bundle Mover {
        pos: Position,
        vel: Velocity,
    }

    plugin Movement;

    system boot(mut cmds: commands) on startup {
        cmds.spawn(Mover { pos: Position { x: 0.0 }, vel: Velocity { v: 2.0 } });
    }

    // DECLARED FIRST, ON PURPOSE — see the test's doc comment. Two systems that both take
    // `mut res<SeqLog>` conflict, and the scheduler serializes a conflict in REGISTRATION
    // order; registration order is source order EXCEPT where an `after` edge re-sorts it. With
    // `check_order` written after `integrate`, both orders agree and the assertion below could
    // never fail. Written before it, only the edge produces the asserted order.
    system check_order(log: mut res<SeqLog>) on update after integrate {
        // The after-edge's observable: integrate has ALREADY run this frame, every frame.
        log.order_ok = log.order_ok && (log.integrate_runs == log.check_runs + 1);
        log.check_runs += 1;
    }

    system integrate(q: query<(&mut Position, &Velocity)>, log: mut res<SeqLog>) on update {
        for (p, v) in &mut q {
            p.x += v.v;
        }
        log.integrate_runs += 1;
    }

    system observe(q: query<&Position>, log: mut res<SeqLog>) on update after integrate {
        for p in &q {
            log.x_seen = p.x;
        }
    }

    system frozen(log: mut res<SeqLog>) on update when never {
        log.frozen_ran = true;
    }
}

/// The cross-system observation channel — aether systems are plain fns (no captures), so a
/// resource is the only honest way to see them run.
#[derive(boyko_macros::Resource)]
struct SeqLog {
    integrate_runs: u32,
    check_runs: u32,
    order_ok: bool,
    frozen_ran: bool,
    x_seen: f32,
}

/// The `when` gate's condition — an ordinary fn, fully RA-visible (§3.3).
fn never() -> bool {
    false
}

#[test]
fn aether_plugin_registers_and_orders_real_systems() {
    let mut app = App::new();
    app.insert_resource(SeqLog {
        integrate_runs: 0,
        check_runs: 0,
        order_ok: true,
        frozen_ran: false,
        x_seen: -1.0,
    });
    app.add_plugin(Movement);

    app.update();
    app.update();

    let log = app.world_mut().resource::<SeqLog>();
    assert_eq!(log.integrate_runs, 2, "integrate ran once per frame");
    assert_eq!(log.check_runs, 2, "check_order ran once per frame");
    assert!(log.order_ok, "the `after integrate` edge held every frame");
    assert!(!log.frozen_ran, "the `when never` gate held the system shut");
    // The startup spawn was visible to the query and the &mut half actually wrote: two frames
    // of `x += 2.0` from `x = 0.0`.
    assert_eq!(log.x_seen, 4.0, "query<(&mut Position, &Velocity)> integrated the spawned entity");
}
