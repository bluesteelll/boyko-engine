//! KE3 (Aether v2 rung R1) — `Query::{get, get_mut, single, single_mut,
//! contains, first}`.
//!
//! # Provenance is SPLIT, and the tree confirms it is split further than the
//! row says
//!
//! `docs/aether-v2/KERNEL-BACKLOG.md` KE3 states that `get` / `get_mut` /
//! `single` / `single_mut` port from `QueryView` while `contains` / `first` are
//! new. Checked against this tree, the *test* provenance is thinner still:
//! `crates/boyko_ecs/tests/phase12_5_track_b_query_view.rs` carries FIVE tests
//! over only TWO of the four methods —
//!
//! | ported from | covers |
//! |---|---|
//! | `query_view_single_smoke` | `single`, one row |
//! | `query_view_single_panics_on_zero_rows` | `single`, zero rows |
//! | `query_view_single_panics_on_many_rows` | `single`, many rows |
//! | `query_view_get_smoke` | `get`, matched |
//! | `query_view_get_returns_none_for_unmatched_entity` | `get`, unmatched |
//!
//! — and NOTHING for `get_mut` or `single_mut`. Those two are written fresh
//! here alongside `contains` and `first`.
//!
//! # The two defects this file is the oracle for
//!
//! A verbatim port of `QueryView::get` / `get_mut` onto `Query` reproduces two
//! wrong answers, both silent:
//!
//! 1. **`SystemMeta::dummy()` instead of the system's own meta.**
//!    `QueryView` has no meta and passes the `'static` dummy, whose `this_run`
//!    is `Tick::ZERO`. `Query` uniquely holds the live `SystemMeta`. Ported
//!    verbatim, a `Mut<T>` obtained through `get_mut` stamps `Tick::ZERO` into
//!    the row's `changed_tick` on `DerefMut` — the write happens, and no
//!    `Changed<T>` reader ever sees it. Pinned by
//!    `get_mut_stamps_the_changed_tick_from_the_system_meta`.
//!
//! 2. **`F::filter_fetch` is never called** — KE13, filed against
//!    `QueryView::get`/`get_mut` and measured red in
//!    `tests/ke13_query_view_get_ignores_dense_filter.rs`. On `QueryView` the
//!    blast radius is bounded because `EcsMaster::query` const-rejects
//!    change-detection filters; on `Query` it is NOT — `Changed<C>` / `Added<C>`
//!    are the whole reason `Query` exists. Pinned by
//!    `get_applies_a_change_detection_filter` (the change-detection leg) and
//!    `get_applies_a_dense_with_filter` (KE13's own dense leg).
//!
//! Both are demonstrated red against the verbatim port before the fix.
//!
//! # `first` states its order; it does not inherit one
//!
//! `QueryView` defines no order for a point/first lookup. This file's
//! `first_yields_the_row_iter_yields_first` pins the order KE3 chooses:
//! **`first()` is `iter().next()`** — matched archetypes in driver order, rows
//! ascending inside each. The rationale lives on the method's doc comment.
//!
//! # Change-detection shapes need a persistent `Schedule`
//!
//! `EcsMaster::run_system` builds a fresh system whose `(last_run, this_run]`
//! window is EMPTY, so a `Changed<_>` term matches no row and the test could not
//! tell a fixed kernel from a broken one. The two change-detection tests below
//! drive a `ScheduleBuilder` across two frames and assert the steady state — the
//! idiom `tests/ke1_or_dense_blindness.rs` established.

// Test oracle model: the std collections in this suite are the REFERENCE
// implementations the engine's VM-native structures are verified against.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::Arc;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query, With};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::{Commands, Res, ResMut};
use boyko_macros::{Bundle, Component, Resource};
use boyko_threadpool::ThreadPoolBuilder;

// ── Components ──────────────────────────────────────────────────────────────

/// Payload — its `p` is the identity every hand oracle is written in.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct KE3Pay {
    p: u32,
}

/// TABLE, change-detected. `marker == 1` selects the rows a writer bumps.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct KE3Hp {
    v: u32,
    marker: u32,
}

/// A second table component, used to build an archetype the payload query does
/// NOT match (the alive-but-unmatched case).
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct KE3Other {
    v: u32,
}

/// DENSE presence subject — the `With<_>` term whose per-row membership test
/// only runs if `filter_fetch` is called (KE13's shape).
#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct KE3Dense {
    v: u32,
}

// ── Bundles ─────────────────────────────────────────────────────────────────

#[derive(Bundle)]
struct BPayHp {
    p: KE3Pay,
    h: KE3Hp,
}

#[derive(Bundle)]
struct BOther {
    o: KE3Other,
}

/// `{Pay}` plus a dense member.
#[derive(Bundle)]
struct BPayDense {
    p: KE3Pay,
    d: KE3Dense,
}

/// `{Pay}` only — signature-identical archetype to `BPayDense`'s, so dense
/// membership is the ONLY difference. Archetype-level filtering cannot help.
#[derive(Bundle)]
struct BPay {
    p: KE3Pay,
}

// ── Probe resources ─────────────────────────────────────────────────────────

/// Two entity handles carried into a system body.
#[derive(Resource)]
struct KE3Pair {
    a: Entity,
    b: Entity,
}

/// Two `Option`-shaped answers, recorded as payload ids so a failure message
/// names the row rather than a bool.
#[derive(Resource, Default)]
struct KE3TwoAnswers {
    a: Option<u32>,
    b: Option<u32>,
}

/// Payload ids a reader collected in one frame.
#[derive(Resource, Default)]
struct KE3Seen(Vec<u32>);

// ════════════════════════════════════════════════════════════════════════════
// Ported half — restated against `Query`
// ════════════════════════════════════════════════════════════════════════════

/// Port of `query_view_single_smoke`.
#[test]
fn single_returns_the_sole_row() {
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(BPay { p: KE3Pay { p: 99 } });
    });

    let seen = world.run_system(|q: Query<&KE3Pay>| q.single().p);
    assert_eq!(seen, 99, "single must return the sole matched row");
}

/// Port of `query_view_single_panics_on_zero_rows`.
#[test]
#[should_panic(expected = "yielded zero rows")]
fn single_panics_on_zero_rows() {
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(BOther { o: KE3Other { v: 1 } });
    });
    world.run_system(|q: Query<&KE3Pay>| {
        let _ = q.single();
    });
}

/// Port of `query_view_single_panics_on_many_rows`.
#[test]
#[should_panic(expected = "yielded more than one row")]
fn single_panics_on_many_rows() {
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(BPay { p: KE3Pay { p: 1 } });
        cmds.spawn(BPay { p: KE3Pay { p: 2 } });
    });
    world.run_system(|q: Query<&KE3Pay>| {
        let _ = q.single();
    });
}

/// Port of `query_view_get_smoke`.
#[test]
fn get_returns_the_row_for_a_matched_entity() {
    let mut world = EcsMaster::new();
    let e = world.run_system(|mut cmds: Commands| {
        cmds.spawn(BPay { p: KE3Pay { p: 33 } }).id()
    });
    world.insert_resource(KE3Pair { a: e, b: e });

    let seen = world.run_system(|q: Query<&KE3Pay>, pair: Res<KE3Pair>| {
        q.get(pair.a).map(|p| p.p)
    });
    assert_eq!(seen, Some(33), "get must return the matched entity's row");
}

/// Port of `query_view_get_returns_none_for_unmatched_entity`.
#[test]
fn get_returns_none_for_an_unmatched_entity() {
    let mut world = EcsMaster::new();
    let other = world.run_system(|mut cmds: Commands| {
        cmds.spawn(BOther { o: KE3Other { v: 7 } }).id()
    });
    world.insert_resource(KE3Pair { a: other, b: other });

    let seen = world.run_system(|q: Query<&KE3Pay>, pair: Res<KE3Pair>| {
        q.get(pair.a).map(|p| p.p)
    });
    assert_eq!(
        seen, None,
        "an entity in a non-matching archetype must not resolve"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// New half — get_mut / single_mut (no QueryView test existed to port)
// ════════════════════════════════════════════════════════════════════════════

/// `get_mut` hands out a writable row and the write lands in the column.
#[test]
fn get_mut_writes_through_to_the_column() {
    let mut world = EcsMaster::new();
    let e = world.run_system(|mut cmds: Commands| {
        cmds.spawn(BPayHp { p: KE3Pay { p: 1 }, h: KE3Hp { v: 10, marker: 0 } })
            .id()
    });
    world.insert_resource(KE3Pair { a: e, b: e });

    world.run_system(|mut q: Query<&mut KE3Hp>, pair: Res<KE3Pair>| {
        let row = q.get_mut(pair.a).expect("entity must be matched");
        row.v = 42;
    });

    let read_back = world.run_system(|q: Query<&KE3Hp>| q.single().v);
    assert_eq!(read_back, 42, "get_mut's write must persist in the column");
}

/// `single_mut` returns the sole row writably.
#[test]
fn single_mut_returns_and_writes_the_sole_row() {
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(BPayHp { p: KE3Pay { p: 1 }, h: KE3Hp { v: 5, marker: 0 } });
    });

    world.run_system(|mut q: Query<&mut KE3Hp>| {
        let row = q.single_mut();
        row.v = 77;
    });

    let read_back = world.run_system(|q: Query<&KE3Hp>| q.single().v);
    assert_eq!(read_back, 77, "single_mut's write must persist");
}

/// `single_mut` panics on zero rows, same contract as `single`.
#[test]
#[should_panic(expected = "yielded zero rows")]
fn single_mut_panics_on_zero_rows() {
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(BOther { o: KE3Other { v: 1 } });
    });
    world.run_system(|mut q: Query<&mut KE3Hp>| {
        let _ = q.single_mut();
    });
}

/// A handle whose generation is stale (the id was recycled) must NOT resolve —
/// otherwise `get` hands the caller a DIFFERENT entity's row under the old
/// handle.
///
/// # Why the direct `EcsMaster` path and not `Commands`
///
/// `Commands::spawn` mints its id through `EntityCounter::reserve_entity`,
/// which by EM2 **never pops the free list** — recycling is dispatcher-only, so
/// a `Commands`-driven despawn/respawn hands out a FRESH id and this test would
/// be vacuous. (Measured: the fixture precondition below caught exactly that on
/// the first draft.) `spawn_one` / `delete_entity` go through
/// `EntityMaster::allocate_entity`, which is the recycling path.
#[test]
fn get_returns_none_for_a_stale_generation_handle() {
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[<KE3Pay as boyko_ecs::ecs::core::component::component::Component>::component_id()]);

    let first = world
        .spawn_one(arch, KE3Pay { p: 1 })
        .expect("spawn_one must succeed");
    assert!(world.delete_entity(first), "delete must succeed");
    let second = world
        .spawn_one(arch, KE3Pay { p: 2 })
        .expect("respawn must succeed");
    assert_eq!(
        second.id(),
        first.id(),
        "fixture precondition: the id must have been recycled"
    );
    assert_ne!(
        second.generation(),
        first.generation(),
        "fixture precondition: recycling must bump the generation"
    );

    world.insert_resource(KE3Pair { a: first, b: second });
    let (stale, live) = world.run_system(|q: Query<&KE3Pay>, pair: Res<KE3Pair>| {
        (q.get(pair.a).map(|p| p.p), q.get(pair.b).map(|p| p.p))
    });

    assert_eq!(stale, None, "a stale-generation handle must resolve to None");
    assert_eq!(live, Some(2), "the live handle must still resolve");
}

/// **Load-bearing red #1.** `get_mut` must stamp the changed tick from the
/// system's OWN `SystemMeta`, not from `SystemMeta::dummy()`.
///
/// The writer bumps entity `a` through `get_mut` every frame; the reader is a
/// `Changed<KE3Hp>` query. Hand oracle for the steady-state frame: `[1]` — only
/// `a`'s payload.
///
/// A verbatim port of `QueryView::get_mut` passes `SystemMeta::dummy()`, whose
/// `this_run` is `Tick::ZERO`, so `Mut::deref_mut` stamps an ancient tick and
/// the reader observes `[]` — the write happened and nothing saw it.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: threadpool busy-wait stalls under Miri; the tick-stamping \
              path is covered natively"
)]
fn get_mut_stamps_the_changed_tick_from_the_system_meta() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let a = world.run_system(|mut cmds: Commands| {
        let a = cmds
            .spawn(BPayHp { p: KE3Pay { p: 1 }, h: KE3Hp { v: 10, marker: 1 } })
            .id();
        cmds.spawn(BPayHp { p: KE3Pay { p: 2 }, h: KE3Hp { v: 20, marker: 0 } });
        a
    });
    world.insert_resource(KE3Pair { a, b: a });
    world.insert_resource(KE3Seen::default());

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut q: Query<Mut<KE3Hp>>, pair: Res<KE3Pair>| {
        let mut row = q.get_mut(pair.a).expect("target must be matched");
        // DerefMut → bumps the row's changed tick with the meta's `this_run`.
        row.v = row.v.wrapping_add(1);
    });
    builder.add_system(|q: Query<&KE3Pay, Changed<KE3Hp>>, mut seen: ResMut<KE3Seen>| {
        seen.0.clear();
        for p in &q {
            seen.0.push(p.p);
        }
        seen.0.sort_unstable();
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1 consumes the spawn ticks (every row's insert tick lies in the
    // reader's first window).
    schedule.run(&mut world);
    // Frame 2 — the steady state under test.
    schedule.run(&mut world);

    assert_eq!(
        world.resource::<KE3Seen>().0,
        vec![1u32],
        "get_mut must stamp the changed tick from the system's own SystemMeta. \
         An observed [] is the SystemMeta::dummy() port — this_run is Tick::ZERO, \
         so the write is stamped ancient and no Changed<_> reader ever sees it"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Filter application — `get` must run the per-row predicate (KE13's shape)
// ════════════════════════════════════════════════════════════════════════════

/// **Load-bearing red #2.** `get` must apply a change-detection filter.
///
/// `a` is bumped every frame, `b` never. Hand oracle for the steady state:
/// `get(a) == Some(1)`, `get(b) == None`.
///
/// A port that omits `F::filter_fetch` answers `Some` for BOTH — the query
/// compiles, runs, and returns a row the filter excludes.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: threadpool busy-wait stalls under Miri; the filter path is \
              covered natively by the dense leg"
)]
fn get_applies_a_change_detection_filter() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let (a, b) = world.run_system(|mut cmds: Commands| {
        let a = cmds
            .spawn(BPayHp { p: KE3Pay { p: 1 }, h: KE3Hp { v: 10, marker: 1 } })
            .id();
        let b = cmds
            .spawn(BPayHp { p: KE3Pay { p: 2 }, h: KE3Hp { v: 20, marker: 0 } })
            .id();
        (a, b)
    });
    world.insert_resource(KE3Pair { a, b });
    world.insert_resource(KE3TwoAnswers::default());

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut q: Query<Mut<KE3Hp>>| {
        for mut h in &mut q {
            if h.marker == 1 {
                h.v = h.v.wrapping_add(1);
            }
        }
    });
    builder.add_system(
        |q: Query<&KE3Pay, Changed<KE3Hp>>,
         pair: Res<KE3Pair>,
         mut out: ResMut<KE3TwoAnswers>| {
            out.a = q.get(pair.a).map(|p| p.p);
            out.b = q.get(pair.b).map(|p| p.p);
        },
    );
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    schedule.run(&mut world);

    let out = world.resource::<KE3TwoAnswers>();
    assert_eq!(out.a, Some(1), "the bumped row must resolve through get");
    assert_eq!(
        out.b, None,
        "the UNCHANGED row must NOT resolve — an observed Some(2) means \
         F::filter_fetch was never called (KE13's shape, on Query)"
    );
}

/// KE13's own leg, on the new surface: a **dense** `With<_>` is non-archetypal
/// (`IS_ARCHETYPAL = false`, and `matches_component_set` admits every
/// archetype), so its ONLY gate is the per-row `filter_fetch` — plus the
/// `resolve_dense` call that populates it. Both entities live in the SAME table
/// archetype `{KE3Pay}`; dense membership is the only difference.
///
/// Hand oracle: `get(a) == Some(1)` (has the dense member),
/// `get(b) == None` (does not). Needs no schedule.
#[test]
fn get_applies_a_dense_with_filter() {
    let mut world = EcsMaster::new();
    let (a, b) = world.run_system(|mut cmds: Commands| {
        let a = cmds
            .spawn(BPayDense { p: KE3Pay { p: 1 }, d: KE3Dense { v: 1 } })
            .id();
        let b = cmds.spawn(BPay { p: KE3Pay { p: 2 } }).id();
        (a, b)
    });
    world.insert_resource(KE3Pair { a, b });

    let (ra, rb) = world.run_system(
        |q: Query<&KE3Pay, With<KE3Dense>>, pair: Res<KE3Pair>| {
            (q.get(pair.a).map(|p| p.p), q.get(pair.b).map(|p| p.p))
        },
    );

    assert_eq!(ra, Some(1), "the dense member must resolve through get");
    assert_eq!(
        rb, None,
        "the non-member must NOT resolve — an observed Some(2) is the KE13 \
         defect (no F::filter_fetch / no F::resolve_dense on the point lookup)"
    );
}

/// The mutable twin: `get_mut` applies the same dense `With<_>` gate.
#[test]
fn get_mut_applies_a_dense_with_filter() {
    let mut world = EcsMaster::new();
    let (a, b) = world.run_system(|mut cmds: Commands| {
        let a = cmds
            .spawn(BPayDense { p: KE3Pay { p: 1 }, d: KE3Dense { v: 1 } })
            .id();
        let b = cmds.spawn(BPay { p: KE3Pay { p: 2 } }).id();
        (a, b)
    });
    world.insert_resource(KE3Pair { a, b });

    let (ra, rb) = world.run_system(
        |mut q: Query<&mut KE3Pay, With<KE3Dense>>, pair: Res<KE3Pair>| {
            let ra = q.get_mut(pair.a).map(|p| p.p);
            let rb = q.get_mut(pair.b).map(|p| p.p);
            (ra, rb)
        },
    );

    assert_eq!(ra, Some(1), "the dense member must resolve through get_mut");
    assert_eq!(rb, None, "the non-member must NOT resolve through get_mut");
}

// ════════════════════════════════════════════════════════════════════════════
// New — `contains`
// ════════════════════════════════════════════════════════════════════════════

/// `contains` is true for an entity the query matches.
#[test]
fn contains_is_true_for_a_matched_entity() {
    let mut world = EcsMaster::new();
    let e = world.run_system(|mut cmds: Commands| {
        cmds.spawn(BPay { p: KE3Pay { p: 1 } }).id()
    });
    world.insert_resource(KE3Pair { a: e, b: e });

    let seen =
        world.run_system(|q: Query<&KE3Pay>, pair: Res<KE3Pair>| q.contains(pair.a));
    assert!(seen, "contains must be true for a matched entity");
}

/// The case KE3's row names explicitly: **alive but unmatched**. The entity is
/// live — a bare liveness check would say `true` — but it sits in an archetype
/// the query does not match.
#[test]
fn contains_is_false_for_an_alive_but_unmatched_entity() {
    let mut world = EcsMaster::new();
    let (matched, unmatched) = world.run_system(|mut cmds: Commands| {
        let m = cmds.spawn(BPay { p: KE3Pay { p: 1 } }).id();
        let u = cmds.spawn(BOther { o: KE3Other { v: 9 } }).id();
        (m, u)
    });
    world.insert_resource(KE3Pair { a: matched, b: unmatched });

    let (a, b) = world.run_system(|q: Query<&KE3Pay>, pair: Res<KE3Pair>| {
        (q.contains(pair.a), q.contains(pair.b))
    });

    assert!(a, "fixture precondition: the matched entity must be contained");
    assert!(
        !b,
        "an ALIVE entity outside the matched archetype set must not be \
         contained — contains is a query-membership test, not a liveness test"
    );
}

/// A despawned entity is not contained.
#[test]
fn contains_is_false_for_a_despawned_entity() {
    let mut world = EcsMaster::new();
    let e = world.run_system(|mut cmds: Commands| {
        cmds.spawn(BPay { p: KE3Pay { p: 1 } }).id()
    });
    world.run_system(move |mut cmds: Commands| cmds.despawn(e));
    world.insert_resource(KE3Pair { a: e, b: e });

    let seen =
        world.run_system(|q: Query<&KE3Pay>, pair: Res<KE3Pair>| q.contains(pair.a));
    assert!(!seen, "a despawned entity must not be contained");
}

/// `contains` agrees with `get`: it applies the per-row filter, not just
/// archetype membership.
#[test]
fn contains_applies_the_filter() {
    let mut world = EcsMaster::new();
    let (a, b) = world.run_system(|mut cmds: Commands| {
        let a = cmds
            .spawn(BPayDense { p: KE3Pay { p: 1 }, d: KE3Dense { v: 1 } })
            .id();
        let b = cmds.spawn(BPay { p: KE3Pay { p: 2 } }).id();
        (a, b)
    });
    world.insert_resource(KE3Pair { a, b });

    let (ca, cb, ga, gb) = world.run_system(
        |q: Query<&KE3Pay, With<KE3Dense>>, pair: Res<KE3Pair>| {
            (
                q.contains(pair.a),
                q.contains(pair.b),
                q.get(pair.a).is_some(),
                q.get(pair.b).is_some(),
            )
        },
    );

    assert_eq!((ca, cb), (true, false), "contains must apply the dense filter");
    assert_eq!(
        (ca, cb),
        (ga, gb),
        "contains must agree with get row for row — a divergence between the \
         two is the silent-wrong-answer class this rung exists to avoid"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// New — `first`
// ════════════════════════════════════════════════════════════════════════════

/// `first` yields exactly what `iter().next()` yields — the STATED order.
///
/// Within one archetype, rows are visited in ascending row index, i.e. spawn
/// order here. Both the absolute value and the agreement with `iter` are pinned:
/// the absolute assertion catches an ordering change, the agreement assertion
/// catches a `first` that drifts away from `iter` without changing the order of
/// either.
#[test]
fn first_yields_the_row_iter_yields_first() {
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(BPay { p: KE3Pay { p: 1 } });
        cmds.spawn(BPay { p: KE3Pay { p: 2 } });
        cmds.spawn(BPay { p: KE3Pay { p: 3 } });
    });

    let (first, iter_first) = world.run_system(|q: Query<&KE3Pay>| {
        (q.first().map(|p| p.p), q.iter().next().map(|p| p.p))
    });

    assert_eq!(
        first,
        Some(1),
        "first must be the first row of the first matched archetype \
         (ascending row index inside an archetype)"
    );
    assert_eq!(first, iter_first, "first must equal iter().next()");
}

/// `first` applies the filter: the first row `iter()` yields under the filter,
/// not the first row of the archetype.
#[test]
fn first_applies_the_filter() {
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        // Spawned FIRST, but has no dense member — the filter must skip it.
        cmds.spawn(BPay { p: KE3Pay { p: 1 } });
        cmds.spawn(BPayDense { p: KE3Pay { p: 2 }, d: KE3Dense { v: 1 } });
    });

    let (first, iter_first) = world.run_system(|q: Query<&KE3Pay, With<KE3Dense>>| {
        (q.first().map(|p| p.p), q.iter().next().map(|p| p.p))
    });

    assert_eq!(
        first,
        Some(2),
        "first must skip the filtered-out row that precedes it"
    );
    assert_eq!(first, iter_first, "first must equal iter().next()");
}

/// `first` on an empty match is `None`, not a panic — that is the difference
/// from `single`.
#[test]
fn first_is_none_on_an_empty_match() {
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(BOther { o: KE3Other { v: 1 } });
    });

    let first = world.run_system(|q: Query<&KE3Pay>| q.first().map(|p| p.p));
    assert_eq!(first, None, "first must be None when nothing matches");
}
