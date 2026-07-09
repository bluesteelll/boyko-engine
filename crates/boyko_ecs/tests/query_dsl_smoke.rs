//! End-to-end smoke tests for the Phase 8b typed `Query<D, F>` DSL.
//!
//! Exercises the public surface delivered in Steps 1–8:
//! - [`Query<&T>`] read-only iteration via `for x in &q` (C1 IntoIterator).
//! - [`Query<&mut T>`] mutable iteration via `for x in &mut q` (C1 IntoIterator).
//! - Tuple [`QueryData`]: `Query<(&A, &B)>`.
//! - Archetypal filters: `With<C>`, `Without<C>`, `Or<(With<A>, With<B>)>`.
//! - Empty-archetype-set behaviour (zero yields).
//! - Intra-system aliasing detection on
//!   `(Query<&mut T>, Query<&T>)` — panics at `init_access` with
//!   `boyko-B0002` before the closure body runs.
//!
//! # `#[derive(Component)]` reach-through
//!
//! `boyko-macros` lives in `[dev-dependencies]` of `boyko-ecs`, so the
//! derive is available to integration tests under `tests/` without polluting
//! the library's runtime dependencies. Each test uses a UNIQUE set of
//! component types (`T1Position`, `T2Position`, …) so the global
//! component-id registry never collides across tests, regardless of test
//! execution order.
//!
//! # Probe pattern
//!
//! Closures passed to [`EcsMaster::run_closure_once`] must be
//! `Send + Sync + 'static` per the `System` trait bound. To smuggle
//! observations out of the closure into the assertion site we use
//! [`std::sync::Arc<AtomicUsize>`] / `AtomicU32` probes (same pattern as
//! `tests/system_param_smoke.rs`).
//!
//! [`Query<&T>`]: boyko_ecs::ecs::core::iters::query::Query
//! [`QueryData`]: boyko_ecs::ecs::core::iters::query::QueryData
//! [`EcsMaster::run_closure_once`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::run_closure_once

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Query, With, Without};
// `Or<F>` is defined in `filter.rs` but not re-exported from `query::mod`,
// so we reach it via the explicit submodule path.
use boyko_ecs::ecs::core::iters::query::filter::Or;
use boyko_macros::Component;

// ── Test 1: `Query<&Position>` yields the components that were spawned ──────

#[derive(Component)]
#[repr(C)]
struct T1Position {
    x: f32,
    y: f32,
}

/// `run_closure_once` with `Query<&Position>` walks every entity in every
/// matched archetype and yields the component values that were spawned.
///
/// Probes a running sum + a row count out of the closure via `AtomicU32`s
/// so the parent test can assert against the values the closure observed.
#[test]
fn run_closure_once_with_query_yields_components() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T1Position::component_id()]);
    ecs.spawn_one(arch, T1Position { x: 1.0, y: 2.0 })
        .expect("spawn 1 must succeed");
    ecs.spawn_one(arch, T1Position { x: 3.0, y: 4.0 })
        .expect("spawn 2 must succeed");

    let count = Arc::new(AtomicUsize::new(0));
    let sum_x = Arc::new(AtomicU32::new(0));
    let sum_y = Arc::new(AtomicU32::new(0));
    let probe_count = count.clone();
    let probe_x = sum_x.clone();
    let probe_y = sum_y.clone();

    ecs.run_closure_once(move |q: Query<'_, '_, &T1Position>| {
        for pos in &q {
            probe_count.fetch_add(1, Ordering::Relaxed);
            // f32 sums via the AtomicU32 probe: encode the integer
            // sum by reading `pos.x as u32` / `pos.y as u32` — the
            // test inputs (1.0..4.0) round-trip losslessly.
            probe_x.fetch_add(pos.x as u32, Ordering::Relaxed);
            probe_y.fetch_add(pos.y as u32, Ordering::Relaxed);
        }
    });

    assert_eq!(
        count.load(Ordering::Relaxed),
        2,
        "Query<&T1Position> must yield both spawned entities"
    );
    assert_eq!(
        sum_x.load(Ordering::Relaxed),
        4,
        "yielded x values must sum to 1 + 3 = 4"
    );
    assert_eq!(
        sum_y.load(Ordering::Relaxed),
        6,
        "yielded y values must sum to 2 + 4 = 6"
    );
}

// ── Test 2: `Query<&mut Position>` mutations persist past the closure ────────

#[derive(Component)]
#[repr(C)]
struct T2Position {
    x: f32,
    y: f32,
}

/// `Query<&mut Position>` exposes per-row exclusive references; mutations
/// performed inside the closure must be observable by a subsequent
/// read-only `Query<&Position>` (round-trip through the world).
#[test]
fn run_closure_once_with_mut_query_mutates() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T2Position::component_id()]);
    ecs.spawn_one(arch, T2Position { x: 1.0, y: 2.0 })
        .expect("spawn 1 must succeed");
    ecs.spawn_one(arch, T2Position { x: 3.0, y: 4.0 })
        .expect("spawn 2 must succeed");

    // Write phase: bump every x by 10.
    ecs.run_closure_once(move |mut q: Query<'_, '_, &mut T2Position>| {
        for pos in &mut q {
            pos.x += 10.0;
        }
    });

    // Read phase: confirm the bumps are visible through a fresh `Query`.
    // The values must now be (11, 2) and (13, 4), summing to 24 on x.
    let sum_x = Arc::new(AtomicU32::new(0));
    let probe_x = sum_x.clone();
    ecs.run_closure_once(move |q: Query<'_, '_, &T2Position>| {
        for pos in &q {
            probe_x.fetch_add(pos.x as u32, Ordering::Relaxed);
        }
    });

    assert_eq!(
        sum_x.load(Ordering::Relaxed),
        24,
        "mutations made via &mut must round-trip through subsequent &-reads"
    );
}

// ── Test 3: tuple `Query<(&Position, &Velocity)>` yields paired components ──

#[derive(Component)]
#[repr(C)]
struct T3Position {
    x: f32,
}

#[derive(Component)]
#[repr(C)]
struct T3Velocity {
    dx: f32,
}

/// Tuple `QueryData` impls (Step 4) lift `(&A, &B)` into a `QueryData` that
/// yields `(&A, &B)` per row. Verify both elements of the tuple round-trip
/// from spawn → query.
#[test]
fn run_closure_once_with_tuple_query() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[
        T3Position::component_id(),
        T3Velocity::component_id(),
    ]);
    ecs.spawn_two(arch, T3Position { x: 10.0 }, T3Velocity { dx: 5.0 })
        .expect("spawn must succeed");
    ecs.spawn_two(arch, T3Position { x: 20.0 }, T3Velocity { dx: 7.0 })
        .expect("spawn must succeed");

    let pos_sum = Arc::new(AtomicU32::new(0));
    let vel_sum = Arc::new(AtomicU32::new(0));
    let probe_pos = pos_sum.clone();
    let probe_vel = vel_sum.clone();

    ecs.run_closure_once(move |q: Query<'_, '_, (&T3Position, &T3Velocity)>| {
        for (pos, vel) in &q {
            probe_pos.fetch_add(pos.x as u32, Ordering::Relaxed);
            probe_vel.fetch_add(vel.dx as u32, Ordering::Relaxed);
        }
    });

    assert_eq!(
        pos_sum.load(Ordering::Relaxed),
        30,
        "tuple query must yield T3Position::x for both rows (10 + 20)"
    );
    assert_eq!(
        vel_sum.load(Ordering::Relaxed),
        12,
        "tuple query must yield T3Velocity::dx for both rows (5 + 7)"
    );
}

// ── Test 4: `With<C>` filter restricts to archetypes containing C ───────────

#[derive(Component)]
#[repr(C)]
struct T4Position {
    x: f32,
}

#[derive(Component)]
#[repr(C)]
struct T4Marker {
    _pad: u32,
}

/// `Query<&T4Position, With<T4Marker>>` matches archetypes containing both
/// `T4Position` AND `T4Marker`. Spawn two archetypes (with and without
/// the marker) and verify only the marked archetype yields rows.
#[test]
fn run_closure_once_with_with_filter() {
    let mut ecs = EcsMaster::new();
    let arch_marked = ecs.create_archetype(&[
        T4Position::component_id(),
        T4Marker::component_id(),
    ]);
    let arch_unmarked = ecs.create_archetype(&[T4Position::component_id()]);

    // Marked archetype: x = 1.0 and x = 2.0.
    ecs.spawn_two(arch_marked, T4Position { x: 1.0 }, T4Marker { _pad: 0 })
        .expect("spawn must succeed");
    ecs.spawn_two(arch_marked, T4Position { x: 2.0 }, T4Marker { _pad: 0 })
        .expect("spawn must succeed");
    // Unmarked archetype: x = 99.0 — must NOT be yielded.
    ecs.spawn_one(arch_unmarked, T4Position { x: 99.0 })
        .expect("spawn must succeed");

    let sum = Arc::new(AtomicU32::new(0));
    let probe = sum.clone();

    ecs.run_closure_once(move |q: Query<'_, '_, &T4Position, With<T4Marker>>| {
        for pos in &q {
            probe.fetch_add(pos.x as u32, Ordering::Relaxed);
        }
    });

    assert_eq!(
        sum.load(Ordering::Relaxed),
        3,
        "With<T4Marker> must only yield from the marked archetype (1 + 2); \
         the unmarked archetype (99) must be excluded"
    );
}

// ── Test 5: `Without<C>` filter excludes archetypes containing C ────────────

#[derive(Component)]
#[repr(C)]
struct T5Position {
    x: f32,
}

#[derive(Component)]
#[repr(C)]
struct T5Frozen {
    _pad: u32,
}

/// `Query<&T5Position, Without<T5Frozen>>` matches archetypes with
/// `T5Position` but NOT `T5Frozen`. Spawn entities into both kinds of
/// archetypes and verify the frozen archetype is excluded.
#[test]
fn run_closure_once_with_without_filter() {
    let mut ecs = EcsMaster::new();
    let arch_active = ecs.create_archetype(&[T5Position::component_id()]);
    let arch_frozen = ecs.create_archetype(&[
        T5Position::component_id(),
        T5Frozen::component_id(),
    ]);

    ecs.spawn_one(arch_active, T5Position { x: 5.0 })
        .expect("spawn must succeed");
    ecs.spawn_one(arch_active, T5Position { x: 7.0 })
        .expect("spawn must succeed");
    ecs.spawn_two(arch_frozen, T5Position { x: 99.0 }, T5Frozen { _pad: 0 })
        .expect("spawn must succeed");

    let sum = Arc::new(AtomicU32::new(0));
    let count = Arc::new(AtomicUsize::new(0));
    let probe_sum = sum.clone();
    let probe_count = count.clone();

    ecs.run_closure_once(move |q: Query<'_, '_, &T5Position, Without<T5Frozen>>| {
        for pos in &q {
            probe_count.fetch_add(1, Ordering::Relaxed);
            probe_sum.fetch_add(pos.x as u32, Ordering::Relaxed);
        }
    });

    assert_eq!(
        count.load(Ordering::Relaxed),
        2,
        "Without<T5Frozen> must yield exactly the 2 active entities"
    );
    assert_eq!(
        sum.load(Ordering::Relaxed),
        12,
        "Without<T5Frozen> must only yield active rows (5 + 7); \
         the frozen archetype (99) must be excluded"
    );
}

// ── Test 6: `Or<(With<A>, With<B>)>` is the union of archetype matches ──────

#[derive(Component)]
#[repr(C)]
struct T6Position {
    x: f32,
}

#[derive(Component)]
#[repr(C)]
struct T6Player {
    _pad: u32,
}

#[derive(Component)]
#[repr(C)]
struct T6Enemy {
    _pad: u32,
}

/// `Query<&T6Position, Or<(With<T6Player>, With<T6Enemy>)>>` matches the
/// UNION of `Player` archetypes and `Enemy` archetypes — but NOT a third
/// archetype containing neither marker.
#[test]
fn run_closure_once_with_or_filter() {
    let mut ecs = EcsMaster::new();
    let arch_player = ecs.create_archetype(&[
        T6Position::component_id(),
        T6Player::component_id(),
    ]);
    let arch_enemy = ecs.create_archetype(&[
        T6Position::component_id(),
        T6Enemy::component_id(),
    ]);
    let arch_neutral = ecs.create_archetype(&[T6Position::component_id()]);

    ecs.spawn_two(arch_player, T6Position { x: 1.0 }, T6Player { _pad: 0 })
        .expect("spawn must succeed");
    ecs.spawn_two(arch_enemy, T6Position { x: 2.0 }, T6Enemy { _pad: 0 })
        .expect("spawn must succeed");
    // Neutral archetype: x = 99 — must NOT be yielded.
    ecs.spawn_one(arch_neutral, T6Position { x: 99.0 })
        .expect("spawn must succeed");

    let sum = Arc::new(AtomicU32::new(0));
    let count = Arc::new(AtomicUsize::new(0));
    let probe_sum = sum.clone();
    let probe_count = count.clone();

    // Filter-shape alias: factors out the `Or<(With<_>, With<_>)>` tuple so
    // the `Query<'_, '_, _, _>` at the closure signature is well within
    // clippy::type_complexity's tolerance. The lifetime is on the outer
    // `Query` (not threaded through `&T6Position`), keeping `D` invariant
    // resolution unambiguous.
    type T6Filter = Or<(With<T6Player>, With<T6Enemy>)>;

    ecs.run_closure_once(move |q: Query<'_, '_, &T6Position, T6Filter>| {
        for pos in &q {
            probe_count.fetch_add(1, Ordering::Relaxed);
            probe_sum.fetch_add(pos.x as u32, Ordering::Relaxed);
        }
    });

    assert_eq!(
        count.load(Ordering::Relaxed),
        2,
        "Or<(With<T6Player>, With<T6Enemy>)> must yield exactly 2 rows \
         (player + enemy); the neutral archetype must be excluded"
    );
    assert_eq!(
        sum.load(Ordering::Relaxed),
        3,
        "Or filter must yield the player row (1) and enemy row (2), \
         summing to 3; the neutral row (99) must be excluded"
    );
}

// ── Test 7: `for x in &q` sugar (IntoIterator for &Query, read-only) ────────

#[derive(Component)]
#[repr(C)]
struct T7Position {
    x: f32,
}

/// C1: `for x in &q { ... }` desugars to `(&q).into_iter()` which resolves
/// to the `IntoIterator for &Query<D, F>` impl gated on
/// `D: ReadOnlyQueryData`. This test exercises the desugar at runtime,
/// not just compile-time (the in-tree `_check_into_iter_ref` helper in
/// `query.rs` already covers the compile-only check).
#[test]
fn iter_into_iterator_syntax_works() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T7Position::component_id()]);
    ecs.spawn_one(arch, T7Position { x: 10.0 })
        .expect("spawn must succeed");
    ecs.spawn_one(arch, T7Position { x: 20.0 })
        .expect("spawn must succeed");
    ecs.spawn_one(arch, T7Position { x: 30.0 })
        .expect("spawn must succeed");

    let sum = Arc::new(AtomicU32::new(0));
    let probe = sum.clone();

    ecs.run_closure_once(move |q: Query<'_, '_, &T7Position>| {
        // The IntoIterator-for-&Query impl is the path under test:
        // `&q` borrows shared; `.into_iter()` returns a `QueryIter`;
        // the for-loop yields `&T7Position` per row.
        for pos in &q {
            probe.fetch_add(pos.x as u32, Ordering::Relaxed);
        }
    });

    assert_eq!(
        sum.load(Ordering::Relaxed),
        60,
        "for x in &q must yield all three rows (10 + 20 + 30 = 60)"
    );
}

// ── Test 8: `for x in &mut q` sugar (IntoIterator for &mut Query) ───────────

#[derive(Component)]
#[repr(C)]
struct T8Position {
    x: f32,
}

/// C1: `for x in &mut q { ... }` desugars to `(&mut q).into_iter()` which
/// resolves to the `IntoIterator for &mut Query<D, F>` impl. Accepts any
/// `D: QueryData` (including `&mut T`) — the `&mut self` borrow enforces
/// cursor uniqueness (Q3). This test mutates each row and then verifies
/// the mutations persist via a follow-up read-only query.
#[test]
fn iter_mut_into_iterator_syntax_works() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[T8Position::component_id()]);
    ecs.spawn_one(arch, T8Position { x: 1.0 })
        .expect("spawn must succeed");
    ecs.spawn_one(arch, T8Position { x: 2.0 })
        .expect("spawn must succeed");

    // Mutate every row through the IntoIterator-for-&mut-Query path.
    ecs.run_closure_once(move |mut q: Query<'_, '_, &mut T8Position>| {
        for pos in &mut q {
            pos.x *= 100.0;
        }
    });

    // Verify the mutations rounded through the world.
    let sum = Arc::new(AtomicU32::new(0));
    let probe = sum.clone();
    ecs.run_closure_once(move |q: Query<'_, '_, &T8Position>| {
        for pos in &q {
            probe.fetch_add(pos.x as u32, Ordering::Relaxed);
        }
    });

    assert_eq!(
        sum.load(Ordering::Relaxed),
        300,
        "for x in &mut q must mutate every row in place (100 + 200 = 300)"
    );
}

// ── Test 9: empty matched-archetype set yields no rows ──────────────────────

#[derive(Component)]
#[repr(C)]
struct T9Position {
    _pad: u32,
}

#[derive(Component)]
#[repr(C)]
struct T9Other {
    _pad: u32,
}

/// `Query<&T9Position>` over an `EcsMaster` whose only archetype lacks
/// `T9Position` must yield zero rows. The closure body must execute (so
/// the test exercises a no-rows iteration, not a no-closure-invocation
/// short-circuit), but the for-loop body must run 0 times.
#[test]
fn empty_query_yields_nothing() {
    let mut ecs = EcsMaster::new();
    // Archetype contains T9Other but not T9Position — must NOT match.
    let arch = ecs.create_archetype(&[T9Other::component_id()]);
    ecs.spawn_one(arch, T9Other { _pad: 7 })
        .expect("spawn must succeed");

    let body_ran = Arc::new(AtomicUsize::new(0));
    let row_count = Arc::new(AtomicUsize::new(0));
    let probe_body = body_ran.clone();
    let probe_rows = row_count.clone();

    ecs.run_closure_once(move |q: Query<'_, '_, &T9Position>| {
        // Mark that the closure body did run.
        probe_body.fetch_add(1, Ordering::Relaxed);
        for _ in &q {
            probe_rows.fetch_add(1, Ordering::Relaxed);
        }
    });

    assert_eq!(
        body_ran.load(Ordering::Relaxed),
        1,
        "the closure body must execute exactly once — Query yielding no \
         rows must NOT short-circuit the system invocation"
    );
    assert_eq!(
        row_count.load(Ordering::Relaxed),
        0,
        "Query<&T9Position> with no matching archetype must yield 0 rows"
    );
}

// ── Test 10: intra-system aliasing — `(Query<&mut A>, Query<&A>)` panics ────

#[derive(Component)]
#[repr(C)]
struct T10Position {
    _pad: u32,
}

/// Intra-system aliasing detection: declaring both `Query<&mut T10Position>`
/// AND `Query<&T10Position>` as siblings in the same system body is an
/// aliasing conflict. The `&mut`-side declares a write of `T10Position`;
/// the `&`-side declares a read; `FilteredAccessSet` catches the conflict
/// during `init_access` (via the tuple `SystemParam` impl) and panics
/// with `boyko-B0002` BEFORE the closure body runs.
///
/// Tests the conservative conflict declared by `<&mut T as QueryData>` (W)
/// vs `<&T as QueryData>` (R) inside the `FilteredAccessSet::add_component_*`
/// path — same machinery used by `Res` / `ResMut` (Phase 8a). The order in
/// the tuple is irrelevant: whichever element is registered first wins, the
/// second triggers the conflict.
#[test]
#[should_panic(expected = "boyko-B0002")]
fn intra_system_conflict_query_mut_query_ref_same_component_panics() {
    let mut ecs = EcsMaster::new();
    // Force registration of T10Position so the conflict path is reached
    // (Without a real archetype the QueryData state still calls
    // `T10Position::component_id()` which lazily registers).
    let _arch = ecs.create_archetype(&[T10Position::component_id()]);

    // `(Query<&mut T10Position>, Query<&T10Position>)` is the SystemParam
    // tuple under test. The tuple `init_access` walks elements in order;
    // the first element registers a write; the second's read collides
    // and panics in `intra_system_conflict_panic` with `boyko-B0002`.
    ecs.run_closure_once(
        |(_w, _r): (
            Query<'_, '_, &mut T10Position>,
            Query<'_, '_, &T10Position>,
        )| {
            // Unreachable — `init_access` panics before any closure body runs.
        },
    );
}
