//! KE16 App-8 — the NEGATIVE-polarity `is_in_system_run()` consumers are still ARMED at depth 2.
//!
//! ## The hole this file closes
//!
//! App-8 turned `InSystemRunGuard` into a depth counter
//! (`docs/threadpool/KE16-DESIGN-APP.md` §7) and the design attached one obligation to it: every
//! consumer of `is_in_system_run()` stays a boolean predicate over the depth
//! (`KE16-DESIGN-B.md` §2.5 names eight sites, in two polarities). Three of the eight are
//! NEGATIVE — they assert the thread is NOT inside a system body:
//!
//! * `ecs_master.rs:677` — SAFETY-7's `drain_deferred_hook_queue` tripwire, reached from the
//!   direct-API `delete_entity` / `create_entity` / `create_entity_at`;
//! * `profiling/store.rs:749` — `Profiler::arm` is a setup call;
//! * `profiling/fold.rs:72` — the fold runs between schedule runs.
//!
//! At the time this file was written the corpus drove the negative polarity at depth 2 through
//! exactly ONE of them: `Time::advance_with` (`tests/ke16_app8_consumer_predicates.rs`, which says
//! so in its own header — "`Time::advance_with` is this file's representative of that family").
//! The other two were driven only at depth 0 (`tests/ke16_in_system_depth_balance.rs` asserts
//! `delete_entity` SUCCEEDS after a schedule run). Depth 0 and depth 2 are different claims, and
//! the difference is the whole of App-8:
//!
//! * If `is_in_system_run()` were simplified to `depth == 1` — the plausible "one guard per body"
//!   edit — then at depth 2, INSIDE the inline sibling system that App-8 exists to permit, the
//!   SAFETY-7 tripwire reads false and `delete_entity`'s deferred-hook drain runs inside a system
//!   body. That is the allocation-discipline window SAFETY-7 exists to keep closed, and no test
//!   in the corpus observed it.
//! * If `drop` stopped decrementing (or saturated), depth never returns to 0 and the same
//!   tripwires are disarmed for the rest of the process. That direction is covered at depth 0 by
//!   `ke16_in_system_depth_balance.rs`; the unwind test below re-checks it after a NESTED pair,
//!   which is the shape a helping joiner produces.
//!
//! `fold.rs:72` is deliberately NOT driven here: `fold` returns at `!profiler.is_armed()` before
//! reaching its assertion, so driving it would require arming the profiler, and `arm` publishes
//! into process-global `ARM_MASK` / `ARMED_STRIDE` state that outlives the test. `Profiler::arm`
//! itself is driven instead, and its assertion is the FIRST statement of the function — the panic
//! happens before any global is touched, so this file leaves the process's profiling state exactly
//! as it found it.
//!
//! ## No joiner is claimed
//!
//! Like its siblings, this file produces the SHAPE a helping joiner produces — a second
//! [`InSystemRunGuard`] entered inside a running one, which is what `Schedule::run` brackets every
//! system body in (`schedule.rs`, the one `InSystemRunGuard::enter()` call site in the ECS). It
//! does not claim that a joiner produced it; that receipt is `tests/ke16_nested_system_inline.rs`.
//!
//! ## Test count is profile-dependent, by construction
//!
//! Two of the three tests assert a `debug_assert!`, so they are `#[cfg(debug_assertions)]`:
//! `running 3 tests` in the default profile, `running 1 test` under `cargo test --release`. It is
//! `#[cfg]` and not `#[ignore]` because in a release build the behaviour under test does not
//! exist, and a `should_panic` whose panic is compiled out is a gate that cannot fail. Compare
//! NAMES, not counts (`KE16-DESIGN-MEASUREMENT.md` §5 shape 4).
//!
//! Component id 496 is reserved for this test binary (493 `ke16_occupancy_gate`, 494
//! `ke16_nested_system_inline`, 495 `ke16_in_system_depth_balance`, 492 the ECS KE16 bench).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::{InSystemRunGuard, is_in_system_run};

#[cfg(debug_assertions)]
use boyko_ecs::ecs::core::profiling::{Profiler, ProfilerConfig};

const SLOT_KE16_NEG: ComponentId = ComponentId(496);

#[repr(C)]
#[derive(Clone, Copy)]
struct Ke16Neg(u32);

impl Component for Ke16Neg {
    fn component_id() -> ComponentId {
        SLOT_KE16_NEG
    }
}

/// The KE16 witness. Printed per TEST rather than per binary: the measurement protocol runs these
/// filtered, so a banner emitted once from a harness `main` would not appear on the invocation
/// whose reading is recorded (`KE16-DESIGN.md` §4).
fn ke16_witness() {
    println!("KE16 variant: {}", boyko_threadpool::ke16_variant());
    boyko_threadpool::ke16_check_expected_variant();
}

/// A world holding one deletable entity. `register_layout` is `OnceLock`-based and same-type
/// idempotent, so two tests of this binary may build a world concurrently.
fn world_with_one_entity() -> (EcsMaster, Entity) {
    register_layout::<Ke16Neg>(SLOT_KE16_NEG.0);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[SLOT_KE16_NEG]);
    let victim = world
        .spawn_one(arch, Ke16Neg(0))
        .expect("invariant: an archetype built from SLOT_KE16_NEG accepts a Ke16Neg");
    (world, victim)
}

/// SAFETY-7's hook-drain tripwire (`ecs_master.rs:677`) at depth TWO.
///
/// What it loses to: `is_in_system_run()` becoming `depth == 1`. The predicate would then read
/// FALSE two deep, this call would drain the deferred hook queue inside a system body, and the
/// assertion this test expects would not fire. Nothing else in the corpus observes that: the
/// direct-API SAFETY-7 drain is driven at depth 0 (`ke16_in_system_depth_balance.rs`) and inside
/// ONE guard nowhere.
///
/// `delete_entity` is the direct-API method the design names for this drain, and it is given a
/// LIVE entity so the call reaches the drain rather than returning early on a stale handle.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "SAFETY-7: hook drain must run with IN_SYSTEM_RUN == false")]
fn hook_drain_safety_7_still_fires_at_guard_depth_two() {
    ke16_witness();
    let (mut world, victim) = world_with_one_entity();

    let _outer = InSystemRunGuard::enter();
    let _inner = InSystemRunGuard::enter();
    assert!(
        is_in_system_run(),
        "depth 2 must read as inside a system run; without that the panic below would prove \
         nothing about the depth counter"
    );

    // Panics via the SAFETY-7 `debug_assert!` inside `drain_deferred_hook_queue`.
    let _ = world.delete_entity(victim);
}

/// The second negative consumer, `Profiler::arm` (`profiling/store.rs:749`), at depth TWO.
///
/// Same loss as above, on a different site — a per-site test rather than one test for the family
/// because the family is only a family in the design document: each site spells its own
/// `debug_assert!`, and a future edit that keys one of them off something other than the predicate
/// would leave the others green.
///
/// The assertion is the first statement of `arm`, so the panic happens before the profiler
/// reserves, publishes or sets `ARM_MASK`: this test leaves the process's global profiling state
/// untouched, which is why it can live beside the others without `--test-threads=1`.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "Profiler::arm is a setup call and must not run inside a system")]
fn profiler_arm_is_still_refused_at_guard_depth_two() {
    ke16_witness();
    let mut profiler = Profiler::new();

    let _outer = InSystemRunGuard::enter();
    let _inner = InSystemRunGuard::enter();
    assert!(is_in_system_run(), "depth 2 must read as inside a system run");

    let _ = profiler.arm(ProfilerConfig::default());
}

/// The other half: the tripwire DISARMS again once the nesting unwinds.
///
/// What it loses to: a `drop` that stops decrementing, or a `saturating_sub` that swallows one of
/// the two. The thread would then stay "inside a system" for the rest of the process and this
/// `delete_entity` — the ordinary post-frame call every host makes — would abort a debug build.
///
/// This is the one test of the file that is NOT `#[cfg(debug_assertions)]`: it asserts a returned
/// value and a predicate, not a `debug_assert!`, so it is a real gate in a release run too.
#[test]
fn the_negative_consumers_are_disarmed_again_after_the_depth_two_unwind() {
    ke16_witness();
    let (mut world, victim) = world_with_one_entity();

    assert!(!is_in_system_run(), "the test thread must start outside a system body");
    {
        let _outer = InSystemRunGuard::enter();
        {
            let _inner = InSystemRunGuard::enter();
            assert!(is_in_system_run(), "depth 2 is inside a system run");
        }
        assert!(
            is_in_system_run(),
            "the INNER guard's drop ended the outer system's run: under the retired `Cell<bool>` \
             this was the defect App-8 fixed, and every negative tripwire would now fire early \
             inside the outer body"
        );
    }
    assert!(
        !is_in_system_run(),
        "the depth did not return to zero after both guards dropped: SAFETY-7's hook drain, the \
         `Time` advance guard and the two profiling asserts are now permanently disarmed on this \
         thread, silently"
    );

    assert!(
        world.delete_entity(victim),
        "the post-nesting delete failed. If it aborted on SAFETY-7 the depth leaked; if it \
         returned false the fixture, not the depth counter, is broken"
    );
}
