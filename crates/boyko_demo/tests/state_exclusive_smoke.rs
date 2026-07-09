//! STEP-0 binding gate for Wave 5 (plan §3 C2).
//!
//! # Result of the gate: PASS (the recommended C2 path works)
//!
//! The whole mode-switch machinery rests on one combination Phase 17 never
//! exercised in-crate: an **exclusive** system (`fn(&mut EcsMaster)`) gated by a
//! state run-condition (`.run_if(on_exit(Mode::X))`). Phase 17 only gated
//! *function* systems with `.run_if`; exclusive systems are full-access nodes the
//! scheduler serializes (runs only when `running == 0`). This is exactly the
//! "primitive verified, headline combo never tried" class the critic flagged, so
//! it is proven HERE before §6.6 builds on it.
//!
//! **It compiles and fires correctly.** `SystemConfig::run_if<C, M>` infers the
//! condition marker `M` independently of the system's own `IntoSystem` marker
//! (the exclusive blanket uses `(ExclusiveSystemMarker, fn(&mut EcsMaster))`,
//! disjoint from the condition's function-system marker), so attaching
//! `.run_if(on_exit(..))` to an exclusive system Just Works. Wave 5 therefore
//! uses the critic's RECOMMENDED default — exclusive despawn/spawn systems gated
//! by `.run_if(on_exit/on_enter)` — NOT the `Commands::despawn` fallback.
//!
//! Note the fallback was doubly unavailable anyway: `StateTransitionRecord`'s
//! `current()` is `pub(crate)`, so an out-of-crate body cannot self-gate by
//! reading the record; the only route to state conditions from a downstream
//! crate is the public `on_enter`/`on_exit`/`in_state` functions via `.run_if`.
//! Good thing the gate passes.
//!
//! # Miri
//!
//! `#![cfg(not(miri))]`: like `tests/phase17_states.rs`, this drives
//! `Schedule::run`, which dispatches through `boyko_threadpool::Scope::spawn`
//! even on a one-thread pool, hitting the Tree-Borrows protected-tag conflict
//! deferred to Phase 9.1. The state algorithm's Miri coverage lives in the core
//! crate's own tests.
#![cfg(not(miri))]

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{ScheduleBuilder, in_state, on_enter, on_exit};
use boyko_ecs::ecs::core::state::states::States;
use boyko_macros::Resource;
use boyko_threadpool::ThreadPoolBuilder;

/// A two-variant state, the minimal shape of the demo's `Mode` (plan §6.6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Mode {
    A,
    B,
}

impl States for Mode {}

/// Counts how many times each gated body fired. Lives in a `Resource` because an
/// exclusive `fn(&mut EcsMaster)` cannot take a `ResMut<_>` param — it mutates
/// the world directly, so the side effect it records must be world state the test
/// reads back afterwards (exactly the shape the real despawn systems use:
/// `world.query_entities(..)` + `world.delete_entity(..)`).
#[derive(Resource, Default)]
struct Counters {
    /// Bumped by the exclusive system gated `.run_if(on_exit(Mode::A))`.
    exit_a: u32,
    /// Bumped by the exclusive system gated `.run_if(on_enter(Mode::B))`.
    enter_b: u32,
}

/// The headline combo: an exclusive system (takes the whole world by
/// `&mut EcsMaster`, so it could `query_entities` + `delete_entity`) gated
/// `.run_if(on_exit(Mode::A))`. Must fire only on the frame `Mode::A` is exited.
fn exclusive_exit_a(world: &mut EcsMaster) {
    world.resource_mut::<Counters>().exit_a += 1;
}

/// The enter twin: an exclusive system gated `.run_if(on_enter(Mode::B))`, the
/// shape the spawn-on-enter systems use (direct `create_entity`). Must fire only
/// on the frame `Mode::B` is entered.
fn exclusive_enter_b(world: &mut EcsMaster) {
    world.resource_mut::<Counters>().enter_b += 1;
}

/// The binding gate: an exclusive `fn(&mut EcsMaster)` gated
/// `.run_if(on_exit(Mode::A))` (and its `on_enter` twin) fires exactly on the
/// transition frame and never otherwise — the exact behavior the despawn/spawn
/// systems rely on (plan §6.6 / D16 / C2).
#[test]
fn exclusive_system_run_if_on_exit_fires_exactly_on_exit_frame() {
    let pool = ThreadPoolBuilder::new().num_threads(1).build();

    let mut world = EcsMaster::with_capacity(1, 1);
    world.insert_resource(Counters::default());

    let mut builder = ScheduleBuilder::new(pool);
    builder.insert_state(Mode::A);
    // The combination the gate exists to prove: exclusive systems + state
    // run-conditions, with intra-frame order pinned via `.before`/`.key`
    // (those compose with exclusive systems too — exercised in mode_switch.rs).
    builder
        .add_system(exclusive_exit_a)
        .run_if(on_exit(Mode::A));
    builder
        .add_system(exclusive_enter_b)
        .run_if(on_enter(Mode::B));
    let mut schedule = builder.build(&mut world);

    // Frame 1: no transition queued. The initial-transition synthesis (Phase 17
    // D7) enters Mode::A — it does NOT exit it, and does NOT enter B — so neither
    // condition holds.
    schedule.run(&mut world);
    {
        let c = world.resource::<Counters>();
        assert_eq!(c.exit_a, 0, "no exit of A without a transition out of A");
        assert_eq!(c.enter_b, 0, "B is not entered on frame 1");
    }

    // Queue A -> B and run: this is the transition frame. exit(A) AND enter(B)
    // both fire, exactly once each.
    world.set_next_state(Mode::B);
    schedule.run(&mut world);
    assert_eq!(
        *world.state::<Mode>(),
        Mode::B,
        "the transition pass must apply the queued A -> B"
    );
    {
        let c = world.resource::<Counters>();
        assert_eq!(c.exit_a, 1, "exclusive on_exit(A) fires once on the A->B frame");
        assert_eq!(c.enter_b, 1, "exclusive on_enter(B) fires once on the A->B frame");
    }

    // A further frame with nothing queued: no transition, no further firing.
    schedule.run(&mut world);
    {
        let c = world.resource::<Counters>();
        assert_eq!(c.exit_a, 1, "on_exit(A) must not fire again after the exit frame");
        assert_eq!(c.enter_b, 1, "on_enter(B) must not fire again after the enter frame");
    }
}

/// Companion check that `.run_if(in_state(..))` on a FUNCTION system (the form
/// the per-mode SIM systems use) gates correctly — so the gate covers all three
/// shapes Wave 5 leans on (`on_exit`/`on_enter` exclusive + `in_state` function).
#[test]
fn function_system_run_if_in_state_gates() {
    use boyko_ecs::ecs::core::system::ResMut;

    /// Counts frames a function system runs while `in_state(Mode::A)`.
    #[derive(Resource, Default)]
    struct InACount(u32);

    fn count_in_a(mut c: ResMut<InACount>) {
        c.0 += 1;
    }

    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut world = EcsMaster::with_capacity(1, 1);
    world.insert_resource(InACount::default());

    let mut builder = ScheduleBuilder::new(pool);
    builder.insert_state(Mode::A);
    builder.add_system(count_in_a).run_if(in_state(Mode::A));
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world); // in A
    schedule.run(&mut world); // still in A
    assert_eq!(world.resource::<InACount>().0, 2, "runs each frame while in A");

    world.set_next_state(Mode::B);
    schedule.run(&mut world); // now in B
    assert_eq!(
        world.resource::<InACount>().0,
        2,
        "must not run once the state left A"
    );
}
