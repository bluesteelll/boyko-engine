//! BUG-ENABLE-PRE-1 — `Or<…>` per-row leak via unconditional-`true`
//! archetypal arms.
//!
//! `Or<(F0, F1, …)>` admits an archetype iff ANY arm matches the archetype
//! mask (`matches_component_set` folds as OR). When `Or` is non-archetypal
//! (some arm is `Changed`/`Added`), the per-row `Or::filter_fetch` folds the
//! arms with OR. The defect was that an ARCHETYPAL arm (`With<B>` /
//! `Without<B>`) returns `true` UNCONDITIONALLY from its `filter_fetch` — it
//! assumes archetype-level matching already admitted it. But under `Or` the
//! archetype may have been admitted via a DIFFERENT arm, so the archetypal
//! arm's unconditional `true` made EVERY row pass.
//!
//! These tests construct archetypes where the archetypal arm does NOT match
//! the archetype, yet a sibling non-archetypal arm admits it, and assert
//! that only the rows satisfying the non-archetypal arm are visited (no
//! leak). Plus a positive guard (the archetypal arm DOES match → all rows)
//! and a fully-archetypal `Or` (the `IS_ARCHETYPAL` const-fold path).
//!
//! # Component id reservation
//!
//! This file claims slot range 720..=730 (unique per-test sets so the global
//! component-id registry never collides across tests regardless of order).
//!
//! # Test idiom
//!
//! Mirrors `tests/phase10_change_detection.rs`: spawn before the schedule
//! runs, drive frames via `Schedule::run`, bump `Changed` ticks by writing
//! through `Mut<T>`. A `static` probe counts visited rows; tests acquire a
//! process-wide mutex when they share probe state across frames.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter::Or;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query, With, Without};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_threadpool::ThreadPoolBuilder;
use boyko_macros::Component;

/// Process-wide mutex for tests that touch shared probe state across frames.
static TEST_MUTEX: Mutex<()> = Mutex::new(());

// ── Component types (slot range 720..=730) ─────────────────────────────────

/// Component `A` — the change-detected component. Carries a `marker` so the
/// writer system can select WHICH rows to mutate.
#[derive(Component)]
#[repr(C)]
struct OrA720 {
    val: u32,
    marker: u32,
}

/// Component `B` — the archetypal-arm subject (`With<B>` / `Without<B>`).
#[derive(Component)]
#[repr(C)]
struct OrB721 {
    b: u32,
}

/// Component `P` — the query payload (`Query<&P, …>`).
#[derive(Component)]
#[repr(C)]
struct OrP722 {
    p: u32,
}

// ── Local 3-component spawn helper ─────────────────────────────────────────

/// Spawns a 3-component entity via the low-level `create_entity` byte API
/// (there is no `spawn_three` on `EcsMaster`). Mirrors `spawn_two`'s byte
/// view + `mem::forget`-on-success discipline.
fn spawn_three<A: Component, B: Component, C: Component>(
    world: &mut EcsMaster,
    arch: ArchetypeId,
    a: A,
    b: B,
    c: C,
) {
    // SAFETY: `a`, `b`, `c` are valid, fully-initialised values on this
    // stack frame; we view `size_of::<T>()` bytes of each as `&[u8]`. The
    // three slices view distinct locals (no aliasing) and are scoped to the
    // `create_entity` call. On Ok the bytes are copied into the pools, which
    // become the owners, so the locals are `mem::forget`'d to avoid a double
    // free; on Err nothing was copied and the locals drop normally.
    let bytes_a =
        unsafe { std::slice::from_raw_parts(std::ptr::addr_of!(a) as *const u8, size_of::<A>()) };
    let bytes_b =
        unsafe { std::slice::from_raw_parts(std::ptr::addr_of!(b) as *const u8, size_of::<B>()) };
    let bytes_c =
        unsafe { std::slice::from_raw_parts(std::ptr::addr_of!(c) as *const u8, size_of::<C>()) };
    let result = world.create_entity(
        arch,
        &[
            (A::component_id(), bytes_a),
            (B::component_id(), bytes_b),
            (C::component_id(), bytes_c),
        ],
    );
    if result.is_ok() {
        std::mem::forget(a);
        std::mem::forget(b);
        std::mem::forget(c);
    }
    result.expect("spawn_three");
}

// ── Test 1: Or<(Changed<A>, With<B>)> over a B-lacking archetype ────────────

/// Archetype `{A, P}` (HAS A, LACKS B). The `With<B>` arm does NOT match the
/// archetype; the `Changed<A>` arm does. After a steady-state frame where
/// only SOME rows' `A` was mutated, `Query<&P, Or<(Changed<A>, With<B>)>>`
/// must visit ONLY the mutated rows — the unchanged rows satisfy neither
/// `Changed<A>` (their tick is stale) nor `With<B>` (the archetype lacks B),
/// so they must NOT leak.
#[test]
// The boyko thread pool's work-stealing busy-wait does not make progress
// under the Miri interpreter (cf. Phase 9.x "multi-thread Miri deferred").
// The Miri UB coverage for this fix lives in the single-threaded
// `run_closure_once` tests below (mirrors `tests/miri_phase10.rs`).
#[cfg_attr(miri, ignore = "threadpool busy-wait stalls under Miri; see miri_* tests")]
fn or_changed_with_does_not_leak_unchanged_in_b_lacking_archetype() {
    // Recover from poisoning: the guard only serialises probe access, and
    // every test resets its probes at the start, so a panic in a sibling
    // test does not corrupt the data this lock protects.
    let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Archetype with A and P only — NO B.
    let arch = world.create_archetype(&[OrA720::component_id(), OrP722::component_id()]);
    // marker == 1 → "mutate me"; marker == 0 → "leave alone".
    world
        .spawn_two(arch, OrA720 { val: 10, marker: 1 }, OrP722 { p: 100 })
        .expect("spawn r0 (mutated)");
    world
        .spawn_two(arch, OrA720 { val: 11, marker: 0 }, OrP722 { p: 101 })
        .expect("spawn r1 (unchanged)");
    world
        .spawn_two(arch, OrA720 { val: 12, marker: 1 }, OrP722 { p: 102 })
        .expect("spawn r2 (mutated)");
    world
        .spawn_two(arch, OrA720 { val: 13, marker: 0 }, OrP722 { p: 103 })
        .expect("spawn r3 (unchanged)");

    static OR_VISITED: AtomicUsize = AtomicUsize::new(0);
    static SUM_P: AtomicUsize = AtomicUsize::new(0);
    OR_VISITED.store(0, Ordering::Relaxed);
    SUM_P.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Writer: bump `Changed<A>` ONLY for marker==1 rows (deref-bump via
    // `Mut<T>`; a raw `&mut T` write does NOT bump the changed tick).
    builder.add_system(|mut q: Query<Mut<OrA720>>| {
        for mut a in &mut q {
            if a.marker == 1 {
                a.val = a.val.wrapping_add(1);
            }
        }
    });
    // Reader: count + accumulate `p` for the Or-matched rows.
    #[allow(clippy::type_complexity)] // query DSL type under test
    builder.add_system(|q: Query<&OrP722, Or<(Changed<OrA720>, With<OrB721>)>>| {
        for p in &q {
            OR_VISITED.fetch_add(1, Ordering::Relaxed);
            SUM_P.fetch_add(p.p as usize, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1 — insert bumped every row's changed_tick to its spawn tick,
    // which lies inside the reader's first window, so ALL rows match
    // `Changed<A>`. (Not the case under test; just consumes the spawn ticks.)
    schedule.run(&mut world);

    // Frame 2 — only marker==1 rows are mutated by the writer. The reader
    // runs after the writer (W/R conflict on A orders them). The matched set
    // MUST be exactly the two mutated rows; the unchanged rows must NOT leak
    // via the `With<B>`-returns-true defect.
    OR_VISITED.store(0, Ordering::Relaxed);
    SUM_P.store(0, Ordering::Relaxed);
    schedule.run(&mut world);

    assert_eq!(
        OR_VISITED.load(Ordering::Relaxed),
        2,
        "Or<(Changed<A>, With<B>)> over a B-lacking archetype must visit ONLY \
         the changed rows (the With<B> arm must NOT leak unchanged rows)"
    );
    assert_eq!(
        SUM_P.load(Ordering::Relaxed),
        // p of the two mutated rows: 100 + 102.
        202,
        "exact membership: only the marker==1 rows (p=100, p=102) must pass"
    );
}

// ── Test 2: Or<(Changed<A>, Without<B>)> over a B-present archetype ─────────

/// Archetype `{A, B, P}` (HAS A and B). The `Without<B>` arm does NOT match
/// the archetype (it HAS B); the `Changed<A>` arm does. After a frame where
/// only SOME rows' `A` was mutated, `Query<&P, Or<(Changed<A>, Without<B>)>>`
/// must visit ONLY the mutated rows — `Without<B>` must NOT contribute its
/// unconditional `true`.
#[test]
#[cfg_attr(miri, ignore = "threadpool busy-wait stalls under Miri; see miri_* tests")]
fn or_changed_without_does_not_leak_in_b_present_archetype() {
    // Recover from poisoning: the guard only serialises probe access, and
    // every test resets its probes at the start, so a panic in a sibling
    // test does not corrupt the data this lock protects.
    let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Archetype with A, B, and P — B IS present.
    let arch = world.create_archetype(&[
        OrA720::component_id(),
        OrB721::component_id(),
        OrP722::component_id(),
    ]);
    spawn_three(
        &mut world,
        arch,
        OrA720 { val: 20, marker: 1 },
        OrB721 { b: 1 },
        OrP722 { p: 200 },
    );
    spawn_three(
        &mut world,
        arch,
        OrA720 { val: 21, marker: 0 },
        OrB721 { b: 2 },
        OrP722 { p: 201 },
    );
    spawn_three(
        &mut world,
        arch,
        OrA720 { val: 22, marker: 1 },
        OrB721 { b: 3 },
        OrP722 { p: 202 },
    );

    static OR_VISITED: AtomicUsize = AtomicUsize::new(0);
    static SUM_P: AtomicUsize = AtomicUsize::new(0);
    OR_VISITED.store(0, Ordering::Relaxed);
    SUM_P.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut q: Query<Mut<OrA720>>| {
        for mut a in &mut q {
            if a.marker == 1 {
                a.val = a.val.wrapping_add(1);
            }
        }
    });
    #[allow(clippy::type_complexity)] // query DSL type under test
    builder.add_system(|q: Query<&OrP722, Or<(Changed<OrA720>, Without<OrB721>)>>| {
        for p in &q {
            OR_VISITED.fetch_add(1, Ordering::Relaxed);
            SUM_P.fetch_add(p.p as usize, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1 — consume spawn ticks.
    schedule.run(&mut world);

    // Frame 2 — only marker==1 rows mutated. `Without<B>` must NOT match (the
    // archetype HAS B), so only the two changed rows pass.
    OR_VISITED.store(0, Ordering::Relaxed);
    SUM_P.store(0, Ordering::Relaxed);
    schedule.run(&mut world);

    assert_eq!(
        OR_VISITED.load(Ordering::Relaxed),
        2,
        "Or<(Changed<A>, Without<B>)> over a B-present archetype must visit \
         ONLY the changed rows (Without<B> must NOT match → no leak)"
    );
    assert_eq!(
        SUM_P.load(Ordering::Relaxed),
        // p of the two mutated rows: 200 + 202.
        402,
        "exact membership: only the marker==1 rows (p=200, p=202) must pass"
    );
}

// ── Test 3: positive guard — the archetypal arm DOES match ─────────────────

/// Archetype `{B, P}` (LACKS A, HAS B). `Query<&P, Or<(Changed<A>, With<B>)>>`
/// must visit ALL rows: every row has B ⇒ the `With<B>` arm matches the
/// archetype ⇒ the archetypal-arm short-circuit admits every row. Guards
/// against over-correcting (the fix must not start dropping legitimate
/// archetypal-arm rows).
#[test]
#[cfg_attr(miri, ignore = "threadpool busy-wait stalls under Miri; see miri_* tests")]
fn or_still_admits_archetypal_arm_rows() {
    // Recover from poisoning: the guard only serialises probe access, and
    // every test resets its probes at the start, so a panic in a sibling
    // test does not corrupt the data this lock protects.
    let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Archetype with B and P only — NO A.
    let arch = world.create_archetype(&[OrB721::component_id(), OrP722::component_id()]);
    world
        .spawn_two(arch, OrB721 { b: 1 }, OrP722 { p: 300 })
        .expect("spawn r0");
    world
        .spawn_two(arch, OrB721 { b: 2 }, OrP722 { p: 301 })
        .expect("spawn r1");
    world
        .spawn_two(arch, OrB721 { b: 3 }, OrP722 { p: 302 })
        .expect("spawn r2");

    static OR_VISITED: AtomicUsize = AtomicUsize::new(0);
    static SUM_P: AtomicUsize = AtomicUsize::new(0);
    OR_VISITED.store(0, Ordering::Relaxed);
    SUM_P.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    #[allow(clippy::type_complexity)] // query DSL type under test
    builder.add_system(|q: Query<&OrP722, Or<(Changed<OrA720>, With<OrB721>)>>| {
        for p in &q {
            OR_VISITED.fetch_add(1, Ordering::Relaxed);
            SUM_P.fetch_add(p.p as usize, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Even on a later (idle) frame, every row must still pass via With<B>.
    schedule.run(&mut world);
    OR_VISITED.store(0, Ordering::Relaxed);
    SUM_P.store(0, Ordering::Relaxed);
    schedule.run(&mut world);

    assert_eq!(
        OR_VISITED.load(Ordering::Relaxed),
        3,
        "every row has B ⇒ With<B> matches the archetype ⇒ all 3 rows pass \
         (the archetypal arm must keep admitting its rows)"
    );
    assert_eq!(
        SUM_P.load(Ordering::Relaxed),
        300 + 301 + 302,
        "all P payloads must be visited via the With<B> arm"
    );
}

// ── Test 4: fully-archetypal Or — the IS_ARCHETYPAL const-fold path ─────────

/// `Or<(With<A>, With<B>)>` is fully archetypal (`IS_ARCHETYPAL == true`), so
/// `filter_fetch` const-folds to `return true`. Over mixed archetypes it must
/// return every row of any archetype containing A OR B, and none of an
/// archetype containing neither.
#[test]
#[cfg_attr(miri, ignore = "threadpool busy-wait stalls under Miri; see miri_* tests")]
fn or_fully_archetypal_unaffected() {
    // Recover from poisoning: the guard only serialises probe access, and
    // every test resets its probes at the start, so a panic in a sibling
    // test does not corrupt the data this lock protects.
    let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Archetype 1: {A, P} — matches via With<A>.
    let arch_a = world.create_archetype(&[OrA720::component_id(), OrP722::component_id()]);
    world
        .spawn_two(arch_a, OrA720 { val: 1, marker: 0 }, OrP722 { p: 400 })
        .expect("spawn a0");
    world
        .spawn_two(arch_a, OrA720 { val: 2, marker: 0 }, OrP722 { p: 401 })
        .expect("spawn a1");

    // Archetype 2: {B, P} — matches via With<B>.
    let arch_b = world.create_archetype(&[OrB721::component_id(), OrP722::component_id()]);
    world
        .spawn_two(arch_b, OrB721 { b: 5 }, OrP722 { p: 402 })
        .expect("spawn b0");

    // Archetype 3: {P} only — matches NEITHER arm, must contribute 0 rows.
    let arch_p = world.create_archetype(&[OrP722::component_id()]);
    world.spawn_one(arch_p, OrP722 { p: 999 }).expect("spawn p0");
    world.spawn_one(arch_p, OrP722 { p: 998 }).expect("spawn p1");

    static OR_VISITED: AtomicUsize = AtomicUsize::new(0);
    static SUM_P: AtomicUsize = AtomicUsize::new(0);
    OR_VISITED.store(0, Ordering::Relaxed);
    SUM_P.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    #[allow(clippy::type_complexity)] // query DSL type under test
    builder.add_system(|q: Query<&OrP722, Or<(With<OrA720>, With<OrB721>)>>| {
        for p in &q {
            OR_VISITED.fetch_add(1, Ordering::Relaxed);
            SUM_P.fetch_add(p.p as usize, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);

    assert_eq!(
        OR_VISITED.load(Ordering::Relaxed),
        3,
        "fully-archetypal Or<(With<A>, With<B>)> must visit the 2 A-rows + 1 \
         B-row and skip the P-only archetype"
    );
    assert_eq!(
        SUM_P.load(Ordering::Relaxed),
        400 + 401 + 402,
        "only A-bearing and B-bearing rows pass; the P-only rows (999, 998) \
         must NOT appear"
    );
}

// ── Miri-compatible single-threaded UB coverage ────────────────────────────
//
// The schedule-based tests above are `#[cfg_attr(miri, ignore)]` because the
// boyko thread pool's work-stealing busy-wait does not make progress under
// the Miri interpreter (Phase 9.x "multi-thread Miri deferred"). The tests
// below drive the SAME fixed `Or<…>` code paths single-threaded via
// `EcsMaster::run_closure_once` — mirroring `tests/miri_phase10.rs` — so the
// fix's new code is exercised under Tree Borrows:
//   * the `(arm_fetch, matches: bool)` Fetch shape (`init_fetch`),
//   * the `(*archetype).component_mask()` deref + flag set in every
//     `set_table_*` variant,
//   * the per-row `$f.1 && filter_fetch(&$f.0, row)` gate in `filter_fetch`,
//     including the archetypal arm whose flag is `false` on a non-matching
//     archetype.
// `run_closure_once` builds a FRESH system each call whose `(last_run,
// this_run]` window is empty (`SystemMeta::new` sets `this_run == last_run`;
// only the persistent `Schedule` advances `this_run` per frame). So a
// `Changed<A>` arm matches NO row through `run_closure_once`. This is exactly
// what makes the two tests below leak-detecting WITHOUT a schedule: the
// `Changed<A>` arm contributes 0, so a non-zero result could ONLY come from
// an archetypal arm's unconditional `true` leaking through — the very defect
// under fix. Pre-fix these asserted-0 tests would observe 2 (the leak);
// post-fix they observe 0.

/// `Or<(Changed<A>, With<B>)>` over a B-lacking `{A, P}` archetype, single
/// threaded. `Changed<A>`'s window is empty (fresh system → 0 rows) and
/// `With<B>` is flagged `false` (B absent), so the OR fold must yield 0.
/// Pre-fix the unconditional-`true` `With<B>` arm leaked all rows (→ 2).
/// Also drives the `component_mask()` deref + gated `filter_fetch` under Miri.
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test
fn miri_or_changed_with_b_lacking_no_ub() {
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[OrA720::component_id(), OrP722::component_id()]);
    world
        .spawn_two(arch, OrA720 { val: 1, marker: 1 }, OrP722 { p: 10 })
        .expect("spawn r0");
    world
        .spawn_two(arch, OrA720 { val: 2, marker: 0 }, OrP722 { p: 11 })
        .expect("spawn r1");

    let visited = world.run_closure_once(
        |q: Query<&OrP722, Or<(Changed<OrA720>, With<OrB721>)>>| {
            let mut n = 0usize;
            for _ in &q {
                n += 1;
            }
            n
        },
    );
    assert_eq!(
        visited, 0,
        "Changed<A> arm empty (fresh-system window) + With<B> flagged false \
         (B absent) ⇒ 0 rows; a non-zero result would be the With<B> leak"
    );
}

/// `Or<(Changed<A>, Without<B>)>` over a B-present `{A, B, P}` archetype,
/// single threaded. `Changed<A>`'s window is empty (→ 0) and `Without<B>` is
/// flagged `false` (B present), so the OR fold must yield 0. Pre-fix the
/// unconditional-`true` `Without<B>` arm leaked all rows (→ 2).
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test
fn miri_or_changed_without_b_present_no_ub() {
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[
        OrA720::component_id(),
        OrB721::component_id(),
        OrP722::component_id(),
    ]);
    spawn_three(
        &mut world,
        arch,
        OrA720 { val: 1, marker: 1 },
        OrB721 { b: 1 },
        OrP722 { p: 20 },
    );
    spawn_three(
        &mut world,
        arch,
        OrA720 { val: 2, marker: 0 },
        OrB721 { b: 2 },
        OrP722 { p: 21 },
    );

    let visited = world.run_closure_once(
        |q: Query<&OrP722, Or<(Changed<OrA720>, Without<OrB721>)>>| {
            let mut n = 0usize;
            for _ in &q {
                n += 1;
            }
            n
        },
    );
    assert_eq!(
        visited, 0,
        "Changed<A> arm empty (fresh-system window) + Without<B> flagged \
         false (B present) ⇒ 0 rows; a non-zero result would be the leak"
    );
}

/// Positive guard, single-threaded: `Or<(Changed<A>, With<B>)>` over a
/// `{B, P}` archetype (no A). `Changed<A>`'s arm is flagged `false` (A
/// absent → null tick base); `With<B>`'s arm is flagged `true` and admits
/// every row. Confirms the gate still admits the archetypal arm's rows.
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test
fn miri_or_admits_archetypal_arm_no_ub() {
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[OrB721::component_id(), OrP722::component_id()]);
    world
        .spawn_two(arch, OrB721 { b: 1 }, OrP722 { p: 30 })
        .expect("spawn r0");
    world
        .spawn_two(arch, OrB721 { b: 2 }, OrP722 { p: 31 })
        .expect("spawn r1");

    let visited = world.run_closure_once(
        |q: Query<&OrP722, Or<(Changed<OrA720>, With<OrB721>)>>| {
            let mut n = 0usize;
            for _ in &q {
                n += 1;
            }
            n
        },
    );
    assert_eq!(
        visited, 2,
        "With<B> arm (flagged true) admits all rows; Changed<A> arm flagged \
         false (A absent) contributes nothing"
    );
}

/// Fully-archetypal `Or<(With<A>, With<B>)>`, single-threaded: drives the
/// `IS_ARCHETYPAL` const-fold `filter_fetch` early-return path under Miri
/// (the `matches` flags are computed but never read).
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test
fn miri_or_fully_archetypal_no_ub() {
    let mut world = EcsMaster::new();
    let arch_a = world.create_archetype(&[OrA720::component_id(), OrP722::component_id()]);
    world
        .spawn_two(arch_a, OrA720 { val: 1, marker: 0 }, OrP722 { p: 40 })
        .expect("spawn a0");
    let arch_p = world.create_archetype(&[OrP722::component_id()]);
    world.spawn_one(arch_p, OrP722 { p: 99 }).expect("spawn p0");

    let visited = world.run_closure_once(
        |q: Query<&OrP722, Or<(With<OrA720>, With<OrB721>)>>| {
            let mut n = 0usize;
            for _ in &q {
                n += 1;
            }
            n
        },
    );
    assert_eq!(
        visited, 1,
        "fully-archetypal Or<(With<A>, With<B>)>: only the {{A, P}} row \
         matches; the P-only archetype contributes nothing"
    );
}
