//! Event-lane width validation + worker-panic propagation.
//!
//! Both halves guard the same discovered hazard: `EventWriter::send` routes
//! by the sending WORKER's id, so an event buffer preregistered with fewer
//! lanes than `worker_count + 1` fires `EventBuffer::send_one`'s lane
//! assertion on a worker thread — and only when scheduling happens to land a
//! sender on a high-id worker. Before the fixes this surfaced as an INFINITE
//! HANG (the panicking task never published its executor completion, so the
//! dispatcher waited forever), not as a test failure.
//!
//! * Registration gate — `App` wires `worker_count + 1` into the world's
//!   `EventDispatcher`; an under-provisioned custom `EventConfig` is rejected
//!   LOUDLY at `preregister_event` time on the main thread.
//! * Panic propagation — a panic in a scheduled (worker-dispatched) system
//!   body re-raises from `Schedule::run` on the dispatcher. If a regression
//!   reintroduces the hang, these tests time out under the harness instead of
//!   passing — run the suite with a timeout wrapper.
//!
//! Reserved EventId range for this suite: **140-149** (the suites before us
//! hold 10-29, 50-70, 80, 90, 100-119, 130-139).

#![cfg(not(miri))]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::error::EcsError;
use boyko_ecs::prelude::*;

// ── Event plumbing (manual impls; ids 140-141) ──────────────────────────────

#[derive(Clone, Copy)]
struct NoParticipants;
impl Participants for NoParticipants {
    fn participant_count() -> usize {
        0
    }
    fn participant_info() -> &'static [ParticipantInfo] {
        &[]
    }
}

#[derive(Clone, Copy)]
struct NoParameters;
impl Parameters for NoParameters {}

macro_rules! decl_event {
    ($name:ident, $id:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct $name {
            value: u32,
        }
        impl Event for $name {
            type Participants = NoParticipants;
            type Parameters = NoParameters;
            fn event_id() -> u64 {
                $id
            }
            fn event_name() -> &'static str {
                stringify!($name)
            }
            fn new(_: NoParticipants, _: NoParameters) -> Self {
                $name { value: 0 }
            }
            fn participants(&self) -> &NoParticipants {
                unimplemented!()
            }
            fn participants_mut(&mut self) -> &mut NoParticipants {
                unimplemented!()
            }
            fn parameters(&self) -> &NoParameters {
                unimplemented!()
            }
            fn parameters_mut(&mut self) -> &mut NoParameters {
                unimplemented!()
            }
        }
    };
}
decl_event!(NarrowEvt, 140);
decl_event!(FlowEvt, 141);

// ── Registration gate ────────────────────────────────────────────────────────

/// An `EventConfig` with fewer lanes than the App's pool requires is rejected
/// at `preregister_event` time — on the main thread, deterministically —
/// with the numbers in the error. The exact required width passes.
#[test]
fn undersized_event_config_rejected_at_registration_on_app() {
    register_event::<NarrowEvt>(140);

    // 2 workers + 1 dispatcher lane ⇒ required = 3.
    let mut app = App::with_threads(2);
    assert_eq!(
        app.world_mut().events().default_thread_count(),
        3,
        "App must wire worker_count + 1 into the dispatcher"
    );

    let err = app
        .world_mut()
        .preregister_event::<NarrowEvt>(EventConfig::default_for(2).expect("2 lanes is a valid config shape"))
        .expect_err("an under-provisioned config must be rejected at registration");
    assert!(
        matches!(
            err,
            EcsError::EventConfigTooFewLanes { lanes: 2, required: 3, .. }
        ),
        "expected EventConfigTooFewLanes {{ lanes: 2, required: 3 }}, got {err:?}"
    );

    // The exact requirement (readable from the dispatcher) is accepted.
    let required = app.world_mut().events().default_thread_count();
    app.world_mut()
        .preregister_event::<NarrowEvt>(EventConfig::default_for(required).expect("valid"))
        .expect("exact-width config must register");
}

/// `preregister_event_default` sizes buffers from the pool automatically and
/// events flow end-to-end from worker-dispatched writer systems: every worker
/// lane plus the dispatcher lane is in range by construction.
#[test]
fn preregister_event_default_sizes_to_pool_and_flows() {
    register_event::<FlowEvt>(141);

    let received = Arc::new(AtomicU32::new(0));
    let r = Arc::clone(&received);

    let mut app = App::with_threads(2);
    app.world_mut()
        .preregister_event_default::<FlowEvt>()
        .expect("default preregistration derives the lane count from the pool");

    app.add_systems(move |mut w: EventWriter<FlowEvt>| {
        w.send(FlowEvt { value: 1 }).expect("send within lane capacity");
    });
    app.add_systems(move |mut rd: EventReader<FlowEvt>| {
        r.fetch_add(rd.read().count() as u32, Ordering::Relaxed);
    });

    const FRAMES: u32 = 8;
    for _ in 0..FRAMES {
        app.update_with_delta(Duration::from_millis(16));
    }

    // Double-buffer rhythm: frame k's send is readable in frame k+1, so the
    // last frame's send is still in flight when the loop ends.
    assert_eq!(
        received.load(Ordering::Relaxed),
        FRAMES - 1,
        "every event sent from a worker must arrive exactly once"
    );
}

// ── Worker-panic propagation ─────────────────────────────────────────────────

/// A panic inside a concurrent (worker-dispatched) system body must re-raise
/// from `Schedule::run` on the dispatcher — the pre-fix behaviour was an
/// infinite hang (the panicking task never published its executor completion,
/// so the dispatcher waited on `running`/`pending_apply` forever).
///
/// The system is a no-param closure: its access set is NOT universal, so the
/// executor dispatches it via `scope.spawn` onto a worker (only
/// `CpuExclusive` systems run inline on the dispatcher), which is exactly the
/// path the fix covers.
#[test]
fn worker_panic_surfaces_as_schedule_run_panic() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let mut builder = ScheduleBuilder::new(pool);
    builder.add_system(|| {
        panic!("planned worker panic");
    });
    let mut schedule = builder.build(&mut world);

    // `Schedule` / `EcsMaster` carry interior mutability and are not
    // `UnwindSafe`; `AssertUnwindSafe` is the canonical opt-in — both values
    // are dropped right after the catch, never re-observed as consistent.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        schedule.run(&mut world);
    }));

    let payload = result.expect_err("a worker-side system panic must propagate, not hang");
    let msg = payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string payload>");
    assert!(
        msg.contains("planned worker panic"),
        "the ORIGINAL panic payload must reach the dispatcher, got: {msg}"
    );
}
