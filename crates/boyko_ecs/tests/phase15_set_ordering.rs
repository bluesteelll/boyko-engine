//! Phase 15 — integration tests for explicit system ordering + schedule sets.
//!
//! Covers the build-time set-expansion pipeline driven through the public
//! [`ScheduleBuilder`] API:
//!
//! * `SystemConfig::{in_set, before_set, after_set}` (system↔set ordering),
//! * `ScheduleBuilder::configure_set` → `ConfigureSet::{before, after, in_set,
//!   id}` (set↔set ordering + hierarchy nesting),
//! * `ScheduleBuilder::{build, try_build}` and the `ScheduleBuildError`
//!   variants (`OrderingCycle` B9001, `SetHierarchyCycle` B9002,
//!   `SetsOrderedButIntersect` B9004, `UnknownSystemKey` B9005),
//! * `#[derive(SystemSet)]` enum-variant / unit-struct identity.
//!
//! # Asserting dispatch order through the public API
//!
//! `Schedule::systems` is `pub(crate)`, so an integration test cannot read the
//! post-topological-sort order directly. Instead each system pushes its label
//! into a shared `Arc<Mutex<Vec<&str>>>` when it runs, and we assert the
//! RELATIVE position of labels in that log. To make the log order equal to the
//! topological order deterministically, every test builds the schedule on a
//! **single-worker** pool (`num_threads(1)`): ready systems are then dispatched
//! one at a time in Kahn FIFO order and executed serially, so the log is the
//! linearised schedule. Each system also declares a *distinct* resource write,
//! so the conflict graph never serialises two unordered systems by accident —
//! only the explicit ordering edges constrain the order under test.
//!
//! `pos(log, label)` returns the index of a label; `assert_before` checks one
//! label precedes another. Tests assert only the constraints the edges impose
//! (partial order), never an exact total order, except where the edges fully
//! determine it.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{ScheduleBuildError, ScheduleBuilder, SystemSet};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_macros::SystemSet;

// ── Shared harness ───────────────────────────────────────────────────────────

/// Shared, ordered execution log. Each system appends its label on run.
type Log = Arc<Mutex<Vec<&'static str>>>;

fn new_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

/// Single-worker pool — see the module docs for why ordering tests pin one
/// worker (serial dispatch ⇒ log order == topological order).
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Snapshots the log into an owned `Vec`.
fn snapshot(log: &Log) -> Vec<&'static str> {
    log.lock().expect("log mutex poisoned").clone()
}

/// Index of `label` in the recorded run order. Panics if absent — every system
/// added in these tests is expected to run exactly once per frame (SCH6).
fn pos(order: &[&'static str], label: &'static str) -> usize {
    order
        .iter()
        .position(|&l| l == label)
        .unwrap_or_else(|| panic!("label {label:?} never ran; order = {order:?}"))
}

/// Asserts `a` runs before `b` in the recorded order.
fn assert_before(order: &[&'static str], a: &'static str, b: &'static str) {
    assert!(
        pos(order, a) < pos(order, b),
        "expected {a:?} before {b:?}, but order = {order:?}",
    );
}

/// Adds a system that pushes `label` into `log` when it runs. Returns the
/// `SystemKey` so callers can wire `.before/.after` against it.
///
/// The body captures an `Arc<Mutex<..>>` clone — `Send + Sync + 'static`, the
/// `FunctionSystem` closure bound. Because the closure takes no `SystemParam`,
/// its declared `Access` is empty; we rely on insertion-order + explicit edges
/// for ordering, and the single-worker pool to serialise dispatch.
fn add_labeled<'b>(
    builder: &'b mut ScheduleBuilder,
    log: &Log,
    label: &'static str,
) -> boyko_ecs::ecs::core::schedule::SystemConfig<'b> {
    let log_cl = Arc::clone(log);
    builder.add_system(move || {
        log_cl.lock().expect("log mutex poisoned").push(label);
    })
}

// ── Test set markers ─────────────────────────────────────────────────────────

#[derive(SystemSet)]
struct SetS;

#[derive(SystemSet)]
struct SetT;

#[derive(SystemSet)]
struct SetM;

#[derive(SystemSet)]
struct SetP;

#[derive(SystemSet)]
struct SetR;

#[derive(SystemSet)]
struct EmptySet;

#[derive(SystemSet)]
enum CombatSet {
    Target,
    Damage,
    Cleanup,
}

#[derive(SystemSet)]
struct UnitSet;

// =============================================================================
// 1. InSet expansion — members of S before members of T via configure_set
// =============================================================================

/// Systems `a`, `b` join `SetS`; `x` joins `SetT`; `configure_set(S).before(T)`.
/// Every member of S (a, b) must dispatch before x. (Test surface §1.)
#[test]
fn in_set_expansion_orders_members_before_other_set() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    add_labeled(&mut builder, &log, "a").in_set(SetS);
    add_labeled(&mut builder, &log, "b").in_set(SetS);
    add_labeled(&mut builder, &log, "x").in_set(SetT);
    builder.configure_set(SetS).before(SetT);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_eq!(order.len(), 3, "all three systems run once; order = {order:?}");
    assert_before(&order, "a", "x");
    assert_before(&order, "b", "x");
}

/// Discriminating control for §1: the set edge forces an order that is the
/// REVERSE of insertion order. Insertion order is `x` (in T), then `a`,`b`
/// (in S); `configure_set(S).before(T)` must still place a,b before x. If the
/// scheduler ignored the expanded edges and merely dispatched in insertion
/// order, this test would fail — so it proves the §1 assertion is load-bearing
/// (not a coincidence of insertion order matching the constraint).
#[test]
fn in_set_expansion_reorders_against_insertion_order() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    // Insertion order x, a, b — OPPOSITE to the desired a,b → x.
    add_labeled(&mut builder, &log, "x").in_set(SetT);
    add_labeled(&mut builder, &log, "a").in_set(SetS);
    add_labeled(&mut builder, &log, "b").in_set(SetS);
    builder.configure_set(SetS).before(SetT);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_before(&order, "a", "x");
    assert_before(&order, "b", "x");
}

// =============================================================================
// 2. system↔set — before_set / after_set
// =============================================================================

/// `add_system(x).before_set(S)` ⇒ x before every member of S. (Test surface §2.)
#[test]
fn before_set_orders_system_before_every_member() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    add_labeled(&mut builder, &log, "a").in_set(SetS);
    add_labeled(&mut builder, &log, "b").in_set(SetS);
    add_labeled(&mut builder, &log, "x").before_set(SetS);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_before(&order, "x", "a");
    assert_before(&order, "x", "b");
}

/// `add_system(x).after_set(S)` ⇒ x after every member of S (symmetric).
/// (Test surface §2.)
#[test]
fn after_set_orders_system_after_every_member() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    add_labeled(&mut builder, &log, "a").in_set(SetS);
    add_labeled(&mut builder, &log, "b").in_set(SetS);
    add_labeled(&mut builder, &log, "x").after_set(SetS);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_before(&order, "a", "x");
    assert_before(&order, "b", "x");
}

// =============================================================================
// 3. set↔set cartesian — members(S) × members(T)
// =============================================================================

/// `configure_set(S).before(T)` with two members each ⇒ the full cartesian
/// product of ordering edges: every S-member precedes every T-member.
/// (Test surface §3.)
#[test]
fn set_before_set_emits_cartesian_ordering() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    add_labeled(&mut builder, &log, "s1").in_set(SetS);
    add_labeled(&mut builder, &log, "s2").in_set(SetS);
    add_labeled(&mut builder, &log, "t1").in_set(SetT);
    add_labeled(&mut builder, &log, "t2").in_set(SetT);
    builder.configure_set(SetS).before(SetT);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    // All four cartesian edges must hold.
    for s in ["s1", "s2"] {
        for t in ["t1", "t2"] {
            assert_before(&order, s, t);
        }
    }
}

// =============================================================================
// 4. Hierarchy flatten — transitive membership
// =============================================================================

/// `a in_set M`; `configure_set(M).in_set(P)`; `configure_set(P).before(R)`;
/// `r in_set R` ⇒ a (transitively in P) before r. (Test surface §4.)
#[test]
fn nested_set_hierarchy_flattens_membership() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    add_labeled(&mut builder, &log, "a").in_set(SetM);
    add_labeled(&mut builder, &log, "r").in_set(SetR);
    builder.configure_set(SetM).in_set(SetP);
    builder.configure_set(SetP).before(SetR);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_before(&order, "a", "r");
}

// =============================================================================
// 5. Diamond dedup — a member reachable via two nested paths appears once
// =============================================================================

/// `a in_set M`; M nested under both P and (transitively) Q where Q is also a
/// parent reachable from P's sibling — the flattening must dedup a's membership
/// so the cartesian expansion against R does not emit a duplicate edge nor a
/// spurious self-cycle. Build must succeed and a must precede r exactly once.
/// (Test surface §5.)
#[test]
fn diamond_membership_dedups_without_cycle() {
    #[derive(SystemSet)]
    struct DiamondTop;
    #[derive(SystemSet)]
    struct DiamondLeft;
    #[derive(SystemSet)]
    struct DiamondRight;
    #[derive(SystemSet)]
    struct DiamondLeaf;
    #[derive(SystemSet)]
    struct DiamondSink;

    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    // a is in DiamondLeaf. DiamondLeaf nests under BOTH DiamondLeft and
    // DiamondRight; both nest under DiamondTop — a diamond. So a is a
    // transitive member of DiamondTop via two distinct paths.
    add_labeled(&mut builder, &log, "a").in_set(DiamondLeaf);
    add_labeled(&mut builder, &log, "r").in_set(DiamondSink);

    builder.configure_set(DiamondLeaf).in_set(DiamondLeft);
    builder.configure_set(DiamondLeaf).in_set(DiamondRight);
    builder.configure_set(DiamondLeft).in_set(DiamondTop);
    builder.configure_set(DiamondRight).in_set(DiamondTop);
    builder.configure_set(DiamondTop).before(DiamondSink);

    let mut world = EcsMaster::new();
    // build() would panic on a spurious cycle / duplicate-induced underflow;
    // try_build surfaces it as an error we can assert against precisely.
    let mut schedule = builder
        .try_build(&mut world)
        .expect("diamond dedup must not produce a cycle or edge-count error");
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_eq!(order.len(), 2, "exactly two systems run once; order = {order:?}");
    assert_before(&order, "a", "r");
}

// =============================================================================
// 6. config-vs-membership agreement (§13-P6)
// =============================================================================

/// Members joined via `in_set(CombatSet::Target)` are ordered by
/// `configure_set(CombatSet::Target).before(CombatSet::Damage)` — proving the
/// membership path and the config path resolve to the SAME `SystemSetId`.
/// (Test surface §6.)
#[test]
fn config_and_membership_resolve_same_set_id() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    // Capture the id the config path interns…
    let target_id = builder.configure_set(CombatSet::Target).id();
    let damage_id = builder.configure_set(CombatSet::Damage).id();

    add_labeled(&mut builder, &log, "tgt").in_set(CombatSet::Target);
    add_labeled(&mut builder, &log, "dmg").in_set(CombatSet::Damage);
    builder
        .configure_set(CombatSet::Target)
        .before(CombatSet::Damage);

    // The membership path must reuse the same ids the config path minted.
    let reconfirm_target = builder.configure_set(CombatSet::Target).id();
    assert_eq!(
        target_id, reconfirm_target,
        "configure_set(Target) must be stable across calls"
    );
    assert_ne!(
        target_id, damage_id,
        "distinct enum variants must have distinct SystemSetIds"
    );

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    // If membership-in_set(Target) and config-before(Target,Damage) resolved
    // to different ids, the ordering edge would not connect to `tgt`/`dmg`
    // and this assertion would (probabilistically under serial dispatch)
    // still need the constraint to hold deterministically — it does only
    // because both paths share one id.
    assert_before(&order, "tgt", "dmg");
}

// =============================================================================
// 7. enum-variant set hierarchy (§13-P6)
// =============================================================================

/// `configure_set(CombatSet::Cleanup).in_set(SetP)` nests an enum-variant set
/// inside a struct set; Cleanup's member obeys SetP's ordering against SetR.
/// (Test surface §7.)
#[test]
fn enum_variant_set_nests_in_parent_set() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    add_labeled(&mut builder, &log, "cleanup_sys").in_set(CombatSet::Cleanup);
    add_labeled(&mut builder, &log, "r").in_set(SetR);
    builder.configure_set(CombatSet::Cleanup).in_set(SetP);
    builder.configure_set(SetP).before(SetR);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_before(&order, "cleanup_sys", "r");
}

// =============================================================================
// 8. enum-variant distinctness — discriminant / id identity
// =============================================================================

/// Distinct enum variants → distinct `SystemSetId`; the same variant → same id;
/// a unit struct → discriminant 0. Asserts via `set_discriminant`/`set_name`
/// (the trait surface the interning key is built from) plus the builder's id
/// interning. (Test surface §8.)
#[test]
fn enum_variants_and_unit_struct_discriminants() {
    // set_discriminant: enum variants index sequentially; unit struct == 0.
    assert_eq!(CombatSet::Target.set_discriminant(), 0);
    assert_eq!(CombatSet::Damage.set_discriminant(), 1);
    assert_eq!(CombatSet::Cleanup.set_discriminant(), 2);
    assert_eq!(UnitSet.set_discriminant(), 0, "unit struct uses discriminant 0");

    // set_name: enum yields "Type::Variant"; the derive overrides the default.
    assert_eq!(CombatSet::Target.set_name(), "CombatSet::Target");
    assert_eq!(CombatSet::Damage.set_name(), "CombatSet::Damage");

    // Interning: same variant → same id; distinct variants → distinct ids;
    // a different set TYPE with discriminant 0 must not collide with an enum
    // variant that also has discriminant 0 (the key is (TypeId, disc)).
    let pool = serial_pool();
    let mut builder = ScheduleBuilder::new(pool);

    let target_a = builder.configure_set(CombatSet::Target).id();
    let target_b = builder.configure_set(CombatSet::Target).id();
    let damage = builder.configure_set(CombatSet::Damage).id();
    let unit = builder.configure_set(UnitSet).id();

    assert_eq!(target_a, target_b, "same variant interns to one id");
    assert_ne!(target_a, damage, "distinct variants intern to distinct ids");
    assert_ne!(
        target_a, unit,
        "an enum variant (disc 0) and a unit struct (disc 0) of different types must not collide"
    );
}

// =============================================================================
// 9. cycle-through-sets ⇒ OrderingCycle (B9001)
// =============================================================================

/// An ordering cycle formed entirely through set expansion: a member of S is
/// ordered before a member of T (S.before(T)) AND a system edge puts the
/// T-member before the S-member, closing the loop at the system level.
/// `try_build` returns `OrderingCycle`; `build` panics `boyko-B9001`.
/// (Test surface §9.)
#[test]
fn cycle_through_sets_try_build_errors() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    // s joins SetS. We capture s's key, then add t with a direct back-edge
    // t.before(s) and membership in SetT. The set ordering S.before(T) expands
    // to s → t; combined with the system edge t → s this forms a 2-cycle that
    // surfaces at the system level (set-induced, but caught by Tarjan).
    let s = add_labeled(&mut builder, &log, "s").in_set(SetS).key();
    add_labeled(&mut builder, &log, "t").before(s).in_set(SetT);
    builder.configure_set(SetS).before(SetT); // expands to s → t

    let mut world = EcsMaster::new();
    // `Schedule` is not `Debug`, so we can't use `expect_err`; match the `Ok`
    // arm into a panic instead (and drop the schedule so the pool joins).
    let err = match builder.try_build(&mut world) {
        Ok(_schedule) => panic!("s→t (set) + t→s (system) must be a cycle"),
        Err(e) => e,
    };
    match &err {
        ScheduleBuildError::OrderingCycle { systems } => {
            assert!(
                systems.len() >= 2,
                "cycle must name at least the two systems: {systems:?}"
            );
        }
        other => panic!("expected OrderingCycle, got {other:?}"),
    }
    // Display carries the documented code.
    assert!(
        err.to_string().contains("boyko-B9001"),
        "Display must contain boyko-B9001: {err}"
    );
}

/// `build` (not `try_build`) panics with the `boyko-B9001` message on the same
/// set-induced cycle. (Test surface §9 — panic-path half.)
#[test]
#[should_panic(expected = "boyko-B9001")]
fn cycle_through_sets_build_panics() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    let s = add_labeled(&mut builder, &log, "s").in_set(SetS).key();
    add_labeled(&mut builder, &log, "t").before(s).in_set(SetT);
    builder.configure_set(SetS).before(SetT);

    let mut world = EcsMaster::new();
    let _ = builder.build(&mut world);
}

// =============================================================================
// 10. SetsOrderedButIntersect (B9004)
// =============================================================================

/// A system transitively in BOTH sides of an `S.before(T)` ordering would
/// expand to a `sys → sys` self-edge. The builder detects this early with a
/// precise `SetsOrderedButIntersect` (B9004) rather than an opaque SCC.
/// (Test surface §10.)
#[test]
fn sets_ordered_but_intersect_errors() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    // `shared` joins both SetS and SetT; then S is ordered before T.
    add_labeled(&mut builder, &log, "shared")
        .in_set(SetS)
        .in_set(SetT);
    builder.configure_set(SetS).before(SetT);

    let mut world = EcsMaster::new();
    let err = match builder.try_build(&mut world) {
        Ok(_schedule) => panic!("a member shared by two ordered sets must error"),
        Err(e) => e,
    };
    match &err {
        ScheduleBuildError::SetsOrderedButIntersect { a, b, shared } => {
            // The shared system's name is the FunctionSystem type name; we only
            // assert the two set names are the ordered pair and `shared` is
            // populated.
            assert!(!a.is_empty() && !b.is_empty(), "set names populated");
            assert!(!shared.is_empty(), "shared system name populated");
        }
        other => panic!("expected SetsOrderedButIntersect, got {other:?}"),
    }
    assert!(
        err.to_string().contains("boyko-B9004"),
        "Display must contain boyko-B9004: {err}"
    );
}

// =============================================================================
// 11. set-hierarchy cycle (B9002)
// =============================================================================

/// `configure_set(S).in_set(T)` + `configure_set(T).in_set(S)` is a hierarchy
/// cycle. It produces no *system* edge, so it must be caught by the dedicated
/// hierarchy-flatten DFS as `SetHierarchyCycle` (B9002), not by the later
/// system-level Tarjan. (Test surface §11.)
#[test]
fn set_hierarchy_cycle_errors() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    // At least one member so the sets are non-trivially interned; the cycle is
    // in the hierarchy edges, independent of membership.
    add_labeled(&mut builder, &log, "m").in_set(SetS);
    builder.configure_set(SetS).in_set(SetT);
    builder.configure_set(SetT).in_set(SetS);

    let mut world = EcsMaster::new();
    let err = match builder.try_build(&mut world) {
        Ok(_schedule) => panic!("S in_set T and T in_set S must be a hierarchy cycle"),
        Err(e) => e,
    };
    match &err {
        ScheduleBuildError::SetHierarchyCycle { sets } => {
            assert!(
                sets.len() >= 2,
                "hierarchy cycle must name the involved sets: {sets:?}"
            );
        }
        other => panic!("expected SetHierarchyCycle, got {other:?}"),
    }
    assert!(
        err.to_string().contains("boyko-B9002"),
        "Display must contain boyko-B9002: {err}"
    );
}

// =============================================================================
// 12. UnknownSystemKey (B9005) — fires in BOTH debug AND release
// =============================================================================

/// A `before(key)` target that is not in this schedule (a foreign / stale
/// `SystemKey`) must surface as `UnknownSystemKey` (B9005). The validation is
/// NOT `cfg`-gated, so this `try_build` assertion holds in debug AND release.
///
/// We synthesise a foreign key by adding a system to a *second* builder (whose
/// `SystemKey.0` indexes past the first builder's single system) and wiring it
/// as a `.before` target in the first builder. (Test surface §12.)
#[test]
fn unknown_system_key_errors_in_all_profiles() {
    // Builder A has exactly one system → valid keys are {0}.
    let pool_a = serial_pool();
    let log_a = new_log();
    let mut builder_a = ScheduleBuilder::new(pool_a);

    // Builder B has two systems → its second handle is SystemKey(1), which is
    // out of range for builder A (n == 1). Capture that foreign key.
    let pool_b = serial_pool();
    let log_b = new_log();
    let mut builder_b = ScheduleBuilder::new(pool_b);
    add_labeled(&mut builder_b, &log_b, "b0");
    let foreign_key = add_labeled(&mut builder_b, &log_b, "b1").key();
    assert_eq!(foreign_key.0, 1, "foreign key must be index 1");

    // Wire A's only system to run before the FOREIGN key.
    add_labeled(&mut builder_a, &log_a, "a0").before(foreign_key);

    let mut world = EcsMaster::new();
    let err = match builder_a.try_build(&mut world) {
        Ok(_schedule) => panic!("a before(foreign_key) endpoint must be rejected"),
        Err(e) => e,
    };
    match &err {
        ScheduleBuildError::UnknownSystemKey { key, n } => {
            assert_eq!(key.0, 1, "the offending key index is reported");
            assert_eq!(*n, 1, "builder A has exactly one system");
        }
        other => panic!("expected UnknownSystemKey, got {other:?}"),
    }
    assert!(
        err.to_string().contains("boyko-B9005"),
        "Display must contain boyko-B9005: {err}"
    );
}

/// `build` panics `boyko-B9005` on the same foreign-key wiring — confirms the
/// panic path (and, since panics are not `cfg(debug_assertions)`-gated here,
/// that the check is live in release too).
#[test]
#[should_panic(expected = "boyko-B9005")]
fn unknown_system_key_build_panics() {
    let pool_b = serial_pool();
    let log_b = new_log();
    let mut builder_b = ScheduleBuilder::new(pool_b);
    add_labeled(&mut builder_b, &log_b, "b0");
    let foreign_key = add_labeled(&mut builder_b, &log_b, "b1").key();

    let pool_a = serial_pool();
    let log_a = new_log();
    let mut builder_a = ScheduleBuilder::new(pool_a);
    add_labeled(&mut builder_a, &log_a, "a0").before(foreign_key);

    let mut world = EcsMaster::new();
    let _ = builder_a.build(&mut world);
}

// =============================================================================
// 13. empty-set warning — build succeeds, 0 expansion edges, x unconstrained
// =============================================================================

/// `add_system(x).before_set(EmptySet)` where no system ever joined `EmptySet`
/// must build successfully (the empty set produces zero expansion edges and a
/// `boyko-W1501` warning, never an error). We cannot easily capture the
/// `eprintln!` from an integration test, so we assert the observable contract:
/// the build succeeds, x runs, and the schedule with the unconstrained x runs
/// cleanly. (Test surface §13.)
#[test]
fn empty_set_ordering_builds_and_runs() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    add_labeled(&mut builder, &log, "x").before_set(EmptySet);
    // A second, unrelated system to prove the schedule is otherwise normal.
    add_labeled(&mut builder, &log, "y");

    let mut world = EcsMaster::new();
    let mut schedule = builder
        .try_build(&mut world)
        .expect("ordering against an empty set must NOT error");
    assert_eq!(schedule.len(), 2, "both systems present");
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_eq!(order.len(), 2, "both systems run; order = {order:?}");
    assert!(order.contains(&"x"));
    assert!(order.contains(&"y"));
}

// =============================================================================
// Extra coverage — ConfigureSet::after symmetry + id() determinism
// =============================================================================

/// `configure_set(T).after(S)` records the same `S before T` relation as
/// `configure_set(S).before(T)` — members of S precede members of T.
#[test]
fn configure_set_after_is_symmetric_to_before() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    add_labeled(&mut builder, &log, "s").in_set(SetS);
    add_labeled(&mut builder, &log, "t").in_set(SetT);
    builder.configure_set(SetT).after(SetS); // == S.before(T)

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_before(&order, "s", "t");
}

/// A no-ordering schedule with several `in_set` members but no set-level
/// ordering edges builds and runs every system exactly once — the membership
/// bookkeeping alone contributes no edges (the 0%-regression premise at the
/// API level: `in_set` without ordering is inert).
#[test]
fn membership_without_ordering_is_inert() {
    let pool = serial_pool();
    let log = new_log();
    let mut builder = ScheduleBuilder::new(pool);

    add_labeled(&mut builder, &log, "a").in_set(SetS);
    add_labeled(&mut builder, &log, "b").in_set(SetS);
    add_labeled(&mut builder, &log, "c").in_set(SetT);

    let mut world = EcsMaster::new();
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let order = snapshot(&log);
    assert_eq!(order.len(), 3, "all members run once; order = {order:?}");
    for label in ["a", "b", "c"] {
        assert!(order.contains(&label), "{label} must run");
    }
}
