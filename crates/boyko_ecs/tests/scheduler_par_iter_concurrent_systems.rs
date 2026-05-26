//! Phase 9 Wave 6 Step 16 — `par_iter` × `Schedule::run` integration.
//!
//! Verifies that:
//!
//! 1. `Query::par_iter().for_each(...)` invoked from inside a scheduler
//!    system body runs without deadlock. The outer `Schedule::run` is
//!    inside a `pool.install(...)` frame; the inner `par_iter` calls
//!    `pool.scope(...)`, which is the re-entrant scope API (plan §4.5.5
//!    work-stealing Drop / Round 2 C3).
//!
//! 2. Archetypes below `MIN_ARCHETYPE_FOR_PARALLEL` (= 1024 rows) bypass
//!    the dispatch path and run inline on the calling thread — exercises
//!    PAR9 / Round 2 O2. The body still observes every spawned row.
//!
//! Both tests exercise the `boyko_threadpool::try_with_active_pool` lookup
//! (Wave 1) and the Wave 5 `Schedule::executor_main_loop` outer install
//! frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::par_iter::MIN_ARCHETYPE_FOR_PARALLEL;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::ThreadPoolBuilder;

// ── Test fixtures ────────────────────────────────────────────────────────────
//
// Component slot range 320..=329 reserved for the Phase 9 Wave 6 Step 16
// integration tests. Verified disjoint at write time against:
//   - 200-203 — legacy_query.rs
//   - 244-271 — phase8cd_integration.rs / commands.rs
//   - 400-417 — archetype.rs unit tests
//   - 480-510 — query DSL tests
//   - 503-509 — query/data.rs / query/state.rs

const SLOT_PAR_VALUE: ComponentId = ComponentId(320);
const SLOT_PAR_TAG: ComponentId = ComponentId(321);

#[repr(C)]
#[derive(Clone, Copy)]
struct ParValue(u32);

impl Component for ParValue {
    fn component_id() -> ComponentId {
        SLOT_PAR_VALUE
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ParTag(#[allow(dead_code)] u32);

impl Component for ParTag {
    fn component_id() -> ComponentId {
        SLOT_PAR_TAG
    }
}

fn register_test_components() {
    register_layout::<ParValue>(SLOT_PAR_VALUE.0);
    register_layout::<ParTag>(SLOT_PAR_TAG.0);
}

// ── Test 1 — par_iter from a system body, large archetype ───────────────────

/// Probe: holds the accumulated sum across worker chunks.
///
/// `Arc<AtomicUsize>` shared via the scheduler-bound closure (which is
/// `Send + Sync + 'static`); the `par_iter` closure further requires
/// `Send + Sync` (PAR1).
static PAR_ITER_SUM: AtomicUsize = AtomicUsize::new(0);

/// 4096 rows × value `i` sums to (0 + 1 + ... + 4095) = 4095 * 4096 / 2.
const PAR_TEST_N: u32 = 4096;
const EXPECTED_SUM: usize = (PAR_TEST_N as usize) * ((PAR_TEST_N as usize) - 1) / 2;

/// A scheduler system that calls `par_iter().for_each(...)` must not
/// deadlock and must observe every matched row exactly once.
///
/// The archetype holds 4096 rows — well above
/// `MIN_ARCHETYPE_FOR_PARALLEL = 1024`, so the par_iter driver dispatches
/// chunks onto the pool's workers via `scope.spawn`. The scheduler's
/// outer `install` frame is already active (Wave 5); `par_iter` opens a
/// nested `scope` (plan §4.5.5) whose work-stealing Drop prevents the
/// deadlock condition flagged in Round 2 C3.
#[test]
fn par_iter_from_system_body_no_deadlock() {
    PAR_ITER_SUM.store(0, Ordering::Relaxed);
    register_test_components();

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let mut world = EcsMaster::new();

    // Seed the archetype with PAR_TEST_N rows. The seed path uses
    // `EcsMaster::spawn_one` directly (no `Commands`) so the entities
    // are visible immediately — the schedule's per-system apply window
    // never runs because we never enqueue commands.
    let arch = world.create_archetype(&[SLOT_PAR_VALUE]);
    for i in 0..PAR_TEST_N {
        world
            .spawn_one(arch, ParValue(i))
            .expect("seed entity must succeed");
    }

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&ParValue>| {
        q.par_iter().for_each(|v: &ParValue| {
            PAR_ITER_SUM.fetch_add(v.0 as usize, Ordering::Relaxed);
        });
    });

    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let observed = PAR_ITER_SUM.load(Ordering::Relaxed);
    assert_eq!(
        observed, EXPECTED_SUM,
        "par_iter must visit every row exactly once (observed {} of {})",
        observed, EXPECTED_SUM,
    );
}

// ── Test 2 — tiny archetype runs inline (PAR9) ──────────────────────────────

static TINY_ITER_COUNT: AtomicUsize = AtomicUsize::new(0);
static TINY_ITER_SUM: AtomicUsize = AtomicUsize::new(0);

const TINY_N: u32 = 10;

/// Archetypes with fewer than `MIN_ARCHETYPE_FOR_PARALLEL` rows run
/// inline on the calling thread — no `scope.spawn` round-trip.
///
/// We cannot easily assert "no worker activity" without instrumentation
/// (the pool has no public per-frame spawn counter), but we CAN assert
/// that the body observes every row exactly once and that the test
/// terminates cleanly. The combination of "completes in a reasonable
/// time on a small dataset" + "result correct" + "did not deadlock"
/// is the integration-level proof that the PAR9 inline path is wired.
///
/// (The unit-level proof — that the inline branch is selected — lives in
/// `par_iter::tests` once Wave 7 lands the dedicated `par_iter_stress`
/// criterion bench.)
#[test]
fn tiny_archetype_runs_inline_no_deadlock() {
    TINY_ITER_COUNT.store(0, Ordering::Relaxed);
    TINY_ITER_SUM.store(0, Ordering::Relaxed);
    register_test_components();

    // Sanity at compile time: the TINY_N constant we use must be well
    // below the inline threshold so this test stays meaningful even if
    // the threshold value changes in the future. The const assertion
    // catches accidental regressions at compile time.
    const _: () = assert!(
        (TINY_N as usize) < MIN_ARCHETYPE_FOR_PARALLEL,
        "TINY_N must be below MIN_ARCHETYPE_FOR_PARALLEL for the inline path to be exercised"
    );

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[SLOT_PAR_VALUE]);
    for i in 0..TINY_N {
        world
            .spawn_one(arch, ParValue(i))
            .expect("seed entity must succeed");
    }

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&ParValue>| {
        q.par_iter().for_each(|v: &ParValue| {
            TINY_ITER_COUNT.fetch_add(1, Ordering::Relaxed);
            TINY_ITER_SUM.fetch_add(v.0 as usize, Ordering::Relaxed);
        });
    });

    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let expected_sum = (0..TINY_N).map(|i| i as usize).sum::<usize>();
    assert_eq!(
        TINY_ITER_COUNT.load(Ordering::Relaxed),
        TINY_N as usize,
        "tiny archetype: each row must be visited once"
    );
    assert_eq!(
        TINY_ITER_SUM.load(Ordering::Relaxed),
        expected_sum,
        "tiny archetype: sum of values must match the seeded set"
    );
}
