//! Rung A2 end-to-end: an `aether!` block's `plugin` registers REAL systems on a REAL `App` —
//! startup one-shot spawning through `commands`, an `after` sibling edge that must hold every
//! frame, a `when` gate that must hold the gated system shut, and a `query<(&mut …)>` that
//! actually integrates component data (the plan's A2 gate: not just expansion, execution).

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

    system integrate(q: query<(&mut Position, &Velocity)>, log: mut res<SeqLog>) on update {
        for (p, v) in &mut q {
            p.x += v.v;
        }
        log.integrate_runs += 1;
    }

    system check_order(log: mut res<SeqLog>) on update after integrate {
        // The after-edge's observable: integrate has ALREADY run this frame, every frame.
        log.order_ok = log.order_ok && (log.integrate_runs == log.check_runs + 1);
        log.check_runs += 1;
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
