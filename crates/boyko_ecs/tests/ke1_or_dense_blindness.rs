//! KE1 / Aether rung R0 — `Or<…>` is blind to a DENSE arm.
//!
//! # The defect
//!
//! `impl_or_filter_tuple` (`ecs/core/iters/query/filter.rs`) forwards **none**
//! of the dense plumbing that its sibling `impl_query_filter_tuple_and`
//! declares. The AND tuple declares four items — `HAS_DENSE`,
//! `HAS_DENSE_INCLUDE`, `resolve_dense` and `dense_include_candidates`; `Or`
//! declares none of them and silently inherits the `QueryFilter` defaults
//! (`HAS_DENSE = false`, an EMPTY `resolve_dense`).
//!
//! `QueryIter::new` / `QueryIterMut::new` call `F::resolve_dense` only under
//! `if const { F::HAS_DENSE }`. With `Or::HAS_DENSE == false` that call is
//! never emitted, so every dense arm's `DenseFilterFetch::dense` /
//! `ChangedFetch::dense` pointer stays NULL for the whole iteration. The
//! per-row predicate then reads that NULL pointer as an answer:
//!
//! | dense arm inside `Or` | `filter_fetch` on a NULL store | wrong how |
//! |---|---|---|
//! | `Changed<D>` / `Added<D>` | `false` (`fetch.dense.is_null()` early-out) | arm is NEVER true |
//! | `With<D>` | `false` (`DenseFilterFetch::contains_row` early-out) | arm is NEVER true |
//! | `Without<D>` | `!false` = `true` | arm excludes NOTHING |
//!
//! Two *different* silent wrong answers, neither reported. Nothing panics, no
//! log line is emitted, and the query returns a plausible set.
//!
//! # `HAS_DENSE_INCLUDE` is deliberately NOT forwarded
//!
//! A dense INCLUDE term *bounds* the candidate archetype set
//! (`QueryDataState::dense_seed`). Under a disjunction it bounds nothing: an
//! archetype admitted through a SIBLING arm would be dropped by a seed taken
//! from one arm's `arch_presence`. This is the same reason `Or::aggregate_include`
//! is an explicit no-op, and the same disposition `AnyOf` already carries
//! (`HAS_DENSE = true`, `HAS_DENSE_INCLUDE = false`). Forwarding it would trade
//! this defect for a new one.
//!
//! # Shape matrix
//!
//! A fix that forwards the plumbing for one arm POSITION and not the other is
//! exactly the defect that survives a single-shape test, so every test below
//! pins a *hand oracle* — the literal set a reader computes by hand — not a
//! count:
//!
//! | test | shape | dense arm position |
//! |---|---|---|
//! | `or_changed_table_then_dense_*` | `Or<(Changed<T>, Changed<D>)>` | RHS |
//! | `or_changed_dense_then_table_*` | `Or<(Changed<D>, Changed<T>)>` | LHS |
//! | `or_changed_dense_alone_*` | `Or<(Changed<D>,)>` | sole |
//! | `or_with_dense_*` | `Or<(With<D>, With<T>)>` | LHS |
//! | `or_without_dense_*` | `Or<(Without<D>, With<T>)>` | LHS, opposite polarity |
//! | `or_nested_dense_*` | `Or<(Or<(With<D>,)>, With<T>)>` | nested |
//! | `and_tuple_wrapping_or_dense_*` | `(With<T>, Or<(With<D>,)>)` | `Or` inside an AND tuple |
//! | `or_all_table_unchanged_*` | `Or<(With<T>, Without<T2>)>` | none — over-correction guard |
//!
//! # Test idiom
//!
//! Mirrors `tests/bug_enable_pre_1_or_changed_leak.rs` (the sibling `Or`
//! regression suite) and `tests/dense_d4_change_detection.rs`.
//!
//! The change-detection shapes MUST be driven by a persistent `Schedule`:
//! `EcsMaster::run_closure_once` builds a fresh system whose `(last_run,
//! this_run]` window is EMPTY, so a `Changed<_>` arm matches no row there and
//! the test could not tell a fixed kernel from a broken one. The presence
//! shapes (`With` / `Without`) carry no tick dependence and use
//! `run_closure_once` directly.
//!
//! # Component id reservation
//!
//! This file claims slot range 740..=744 (unique per-test type names so the
//! global component-id registry never collides across tests regardless of
//! order).

// Test oracle model: the std collections / `Arc<Mutex<_>>` in this suite are the
// REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, the dense stores) are
// differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::Arc;
use std::sync::Mutex;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter::Or;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query, With, Without};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::ThreadPoolBuilder;

// ── Components (slot range 740..=744) ───────────────────────────────────────

/// TABLE, change-detected. `marker == 1` selects the rows the writer bumps.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct TCh740 {
    v: u32,
    marker: u32,
}

/// DENSE, change-detected — the arm `Or` is blind to.
#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct DCh741 {
    v: u32,
    marker: u32,
}

/// TABLE payload. Its `p` value is the identity the hand oracle is written in.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct P742 {
    p: u32,
}

/// DENSE presence subject for the `With` / `Without` shapes.
#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct DPres743 {
    v: u32,
}

/// TABLE presence subject — the sibling arm that DOES work today.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct TW744 {
    v: u32,
}

// ── Bundles ─────────────────────────────────────────────────────────────────

/// `{P, TCh}` + a dense `DCh` member.
#[derive(Bundle)]
struct BPayloadTableDense {
    p: P742,
    t: TCh740,
    d: DCh741,
}

/// `{P, TCh}`, NO dense member. Signature-identical to `BPayloadTableDense`'s
/// archetype — the dense membership is the ONLY thing that differs, which is
/// exactly where archetype-level filtering cannot help.
#[derive(Bundle)]
struct BPayloadTable {
    p: P742,
    t: TCh740,
}

/// `{P}` only.
#[derive(Bundle)]
struct PresNeither {
    p: P742,
}

/// `{P}` + a dense `DPres` member.
#[derive(Bundle)]
struct PresDenseOnly {
    p: P742,
    d: DPres743,
}

/// `{P, TW}`.
#[derive(Bundle)]
struct PresTableOnly {
    p: P742,
    t: TW744,
}

/// `{P, TW}` + a dense `DPres` member.
#[derive(Bundle)]
struct PresTableDense {
    p: P742,
    t: TW744,
    d: DPres743,
}

// ── Worlds ──────────────────────────────────────────────────────────────────

/// The change-detection world. Every entity lives in the SAME archetype
/// `{P, TCh}`; `DCh` membership varies per row.
///
/// | p | `DCh`? | writer bumps |
/// |---|---|---|
/// | 1 | yes | its DENSE `DCh` only |
/// | 2 | yes | nothing |
/// | 3 | no  | its TABLE `TCh` only |
/// | 4 | no  | nothing |
fn build_change_world(world: &mut EcsMaster) {
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(BPayloadTableDense {
            p: P742 { p: 1 },
            t: TCh740 { v: 10, marker: 0 },
            d: DCh741 { v: 10, marker: 1 },
        });
        cmds.spawn(BPayloadTableDense {
            p: P742 { p: 2 },
            t: TCh740 { v: 20, marker: 0 },
            d: DCh741 { v: 20, marker: 0 },
        });
        cmds.spawn(BPayloadTable {
            p: P742 { p: 3 },
            t: TCh740 { v: 30, marker: 1 },
        });
        cmds.spawn(BPayloadTable {
            p: P742 { p: 4 },
            t: TCh740 { v: 40, marker: 0 },
        });
    });
}

/// The presence world.
///
/// | p | archetype | `DPres`? |
/// |---|---|---|
/// | 11 | `{P}`     | yes |
/// | 12 | `{P}`     | no  |
/// | 13 | `{P, TW}` | no  |
/// | 14 | `{P, TW}` | yes |
fn build_presence_world(world: &mut EcsMaster) {
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(PresDenseOnly { p: P742 { p: 11 }, d: DPres743 { v: 1 } });
        cmds.spawn(PresNeither { p: P742 { p: 12 } });
        cmds.spawn(PresTableOnly { p: P742 { p: 13 }, t: TW744 { v: 1 } });
        cmds.spawn(PresTableDense {
            p: P742 { p: 14 },
            t: TW744 { v: 2 },
            d: DPres743 { v: 2 },
        });
    });
}

/// Sorted payload ids collected by a probe — the shape the hand oracles are
/// written in.
type Probe = Arc<Mutex<Vec<u32>>>;

fn probe() -> Probe {
    // Recover from poisoning: the probe is write-only observation state, reset
    // by its owning test, so a sibling panic cannot corrupt it.
    Arc::new(Mutex::new(Vec::new()))
}

fn take_sorted(p: &Probe) -> Vec<u32> {
    let mut v = p.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = std::mem::take(&mut *v);
    out.sort_unstable();
    out
}

// ════════════════════════════════════════════════════════════════════════════
// Change-detection shapes — the dense arm's POSITION is varied.
// ════════════════════════════════════════════════════════════════════════════

/// `Or<(Changed<TABLE>, Changed<DENSE>)>` — the oracle KE1 / R0 names. Dense
/// arm on the RHS.
///
/// Hand oracle for the steady-state frame: `p == 1` (its dense `DCh` was
/// bumped) and `p == 3` (its table `TCh` was bumped). `p == 2` and `p == 4`
/// were touched by neither writer.
///
/// Pre-fix this observes `[3]` — the dense arm's store pointer is NULL, so
/// `Changed<DCh741>` is false for every row and `p == 1` vanishes.
#[test]
#[cfg_attr(miri, ignore = "threadpool busy-wait stalls under Miri; the dense \
                           membership paths are covered natively")]
fn or_changed_table_then_dense_sees_the_dense_arm() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    build_change_world(&mut world);

    let seen = probe();
    let seen_r = Arc::clone(&seen);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut qd: Query<Mut<DCh741>>, mut qt: Query<Mut<TCh740>>| {
        for mut d in &mut qd {
            if d.marker == 1 {
                d.v = d.v.wrapping_add(1); // deref_mut → bumps the dense slot tick
            }
        }
        for mut t in &mut qt {
            if t.marker == 1 {
                t.v = t.v.wrapping_add(1);
            }
        }
    });
    #[allow(clippy::type_complexity)] // query DSL type under test
    builder.add_system(move |q: Query<&P742, Or<(Changed<TCh740>, Changed<DCh741>)>>| {
        for p in &q {
            seen_r.lock().unwrap_or_else(|e| e.into_inner()).push(p.p);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1 consumes the spawn ticks (every row's insert tick lies in the
    // reader's first window).
    schedule.run(&mut world);
    let _ = take_sorted(&seen);

    // Frame 2 — the steady state under test.
    schedule.run(&mut world);
    assert_eq!(
        take_sorted(&seen),
        vec![1u32, 3],
        "Or<(Changed<TABLE>, Changed<DENSE>)>: p=1 is changed via the DENSE arm \
         and p=3 via the TABLE arm. A result of [3] is the KE1 blindness — the \
         dense arm never resolved its store"
    );
}

/// `Or<(Changed<DENSE>, Changed<TABLE>)>` — dense arm on the LHS. Same hand
/// oracle; a fix that forwards plumbing for one arm position only would part
/// company with the previous test here.
#[test]
#[cfg_attr(miri, ignore = "threadpool busy-wait stalls under Miri; the dense \
                           membership paths are covered natively")]
fn or_changed_dense_then_table_sees_the_dense_arm() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    build_change_world(&mut world);

    let seen = probe();
    let seen_r = Arc::clone(&seen);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut qd: Query<Mut<DCh741>>, mut qt: Query<Mut<TCh740>>| {
        for mut d in &mut qd {
            if d.marker == 1 {
                d.v = d.v.wrapping_add(1);
            }
        }
        for mut t in &mut qt {
            if t.marker == 1 {
                t.v = t.v.wrapping_add(1);
            }
        }
    });
    #[allow(clippy::type_complexity)] // query DSL type under test
    builder.add_system(move |q: Query<&P742, Or<(Changed<DCh741>, Changed<TCh740>)>>| {
        for p in &q {
            seen_r.lock().unwrap_or_else(|e| e.into_inner()).push(p.p);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    let _ = take_sorted(&seen);

    schedule.run(&mut world);
    assert_eq!(
        take_sorted(&seen),
        vec![1u32, 3],
        "Or<(Changed<DENSE>, Changed<TABLE>)>: arm ORDER must not change the \
         answer — the dense arm is the LHS here"
    );
}

/// `Or<(Changed<DENSE>,)>` — the sole arm is dense (arity 1). Nothing else can
/// mask the blindness.
///
/// Hand oracle: `[1]`. Pre-fix: `[]`.
#[test]
#[cfg_attr(miri, ignore = "threadpool busy-wait stalls under Miri; the dense \
                           membership paths are covered natively")]
fn or_changed_dense_alone_sees_the_dense_arm() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    build_change_world(&mut world);

    let seen = probe();
    let seen_r = Arc::clone(&seen);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut qd: Query<Mut<DCh741>>| {
        for mut d in &mut qd {
            if d.marker == 1 {
                d.v = d.v.wrapping_add(1);
            }
        }
    });
    #[allow(clippy::type_complexity)] // query DSL type under test
    builder.add_system(move |q: Query<&P742, Or<(Changed<DCh741>,)>>| {
        for p in &q {
            seen_r.lock().unwrap_or_else(|e| e.into_inner()).push(p.p);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: both dense members were just inserted, so BOTH are Changed.
    schedule.run(&mut world);
    assert_eq!(
        take_sorted(&seen),
        vec![1u32, 2],
        "frame 1: both dense members' insert ticks lie in the first window. An \
         empty result is the KE1 blindness"
    );

    // Frame 2: only p=1's dense component was written.
    schedule.run(&mut world);
    assert_eq!(
        take_sorted(&seen),
        vec![1u32],
        "frame 2: only p=1's dense DCh was bumped"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Presence shapes — `With` / `Without` over a dense arm inside `Or`.
// These carry no tick dependence, so `run_closure_once` drives them directly.
// ════════════════════════════════════════════════════════════════════════════

/// `Or<(With<DENSE>, With<TABLE>)>`.
///
/// Hand oracle: `p == 11` (dense member, no `TW`), `p == 13` (`TW`, no dense
/// member), `p == 14` (both). `p == 12` has neither.
///
/// Pre-fix this observes `[13, 14]` — `DenseFilterFetch::contains_row`
/// short-circuits on the NULL store, so the dense `With` arm rejects every row
/// and `p == 11` is lost.
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test
fn or_with_dense_arm_matches_dense_members() {
    let mut world = EcsMaster::new();
    build_presence_world(&mut world);

    let mut seen = world.run_closure_once(
        |q: Query<&P742, Or<(With<DPres743>, With<TW744>)>>| {
            let mut v: Vec<u32> = Vec::new();
            for p in &q {
                v.push(p.p);
            }
            v
        },
    );
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![11u32, 13, 14],
        "Or<(With<DENSE>, With<TABLE>)>: p=11 matches through the DENSE arm \
         alone. A result of [13, 14] is the KE1 blindness"
    );
}

/// `Or<(Without<DENSE>, With<TABLE>)>` — the OPPOSITE silent wrong answer.
///
/// A NULL dense store makes `contains_row` false, so `Without`'s
/// `!contains_row` is `true` for EVERY row: pre-fix the arm excludes nothing
/// and the query returns all four entities.
///
/// Hand oracle: pass iff (NOT a `DPres` member) OR (has `TW`) — `p == 12`
/// (no member), `p == 13` (no member, has TW), `p == 14` (member BUT has TW).
/// `p == 11` is a member with no `TW` and must be excluded.
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test
fn or_without_dense_arm_excludes_dense_members() {
    let mut world = EcsMaster::new();
    build_presence_world(&mut world);

    let mut seen = world.run_closure_once(
        |q: Query<&P742, Or<(Without<DPres743>, With<TW744>)>>| {
            let mut v: Vec<u32> = Vec::new();
            for p in &q {
                v.push(p.p);
            }
            v
        },
    );
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![12u32, 13, 14],
        "Or<(Without<DENSE>, With<TABLE>)>: p=11 IS a dense member and has no \
         TW, so it must be excluded. A result containing 11 is the KE1 \
         blindness in its excludes-nothing polarity"
    );
}

/// Nested `Or` — `Or<(Or<(With<DENSE>,)>, With<TABLE>)>`. The plumbing must
/// forward through BOTH `Or` levels, not just the outer one.
///
/// Hand oracle: identical to the flat `Or<(With<D>, With<T>)>` case.
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test
fn or_nested_dense_arm_matches_dense_members() {
    let mut world = EcsMaster::new();
    build_presence_world(&mut world);

    let mut seen = world.run_closure_once(
        |q: Query<&P742, Or<(Or<(With<DPres743>,)>, With<TW744>)>>| {
            let mut v: Vec<u32> = Vec::new();
            for p in &q {
                v.push(p.p);
            }
            v
        },
    );
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![11u32, 13, 14],
        "nested Or<(Or<(With<DENSE>,)>, With<TABLE>)>: the dense plumbing must \
         forward through both Or levels"
    );
}

/// `Or` nested inside an AND tuple — `(With<TABLE>, Or<(With<DENSE>,)>)`. The
/// AND tuple OR-folds `HAS_DENSE` over its members, so it can only see the
/// dense term if `Or` reports one.
///
/// Hand oracle: `[14]` — the only entity that is BOTH a `DPres` member and has
/// `TW`. Pre-fix: `[]` (the tuple's `HAS_DENSE` folds to false, so nothing in
/// the whole query resolves a dense store).
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test
fn and_tuple_wrapping_or_dense_matches_dense_members() {
    let mut world = EcsMaster::new();
    build_presence_world(&mut world);

    let mut seen = world.run_closure_once(
        |q: Query<&P742, (With<TW744>, Or<(With<DPres743>,)>)>| {
            let mut v: Vec<u32> = Vec::new();
            for p in &q {
                v.push(p.p);
            }
            v
        },
    );
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![14u32],
        "(With<TABLE>, Or<(With<DENSE>,)>): only p=14 has TW AND is a dense \
         member. An empty result is the KE1 blindness reached through the AND \
         tuple's HAS_DENSE OR-fold"
    );
}

/// Over-correction guard — an all-TABLE `Or` must answer exactly as before.
///
/// `Or<(With<TW744>, Without<TCh740>)>` over the presence world: every entity
/// lacks `TCh740`, so the `Without` arm admits all four; the shape is fully
/// archetypal and must NOT be perturbed by the dense forwarding.
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test
fn or_all_table_arms_are_unchanged() {
    let mut world = EcsMaster::new();
    build_presence_world(&mut world);

    let mut seen = world.run_closure_once(
        |q: Query<&P742, Or<(With<TW744>, Without<TCh740>)>>| {
            let mut v: Vec<u32> = Vec::new();
            for p in &q {
                v.push(p.p);
            }
            v
        },
    );
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![11u32, 12, 13, 14],
        "an all-table Or must keep its pre-fix answer (no dense term ⇒ the \
         0%-gate: HAS_DENSE folds false and resolve_dense is never emitted)"
    );
}
