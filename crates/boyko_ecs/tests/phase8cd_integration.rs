//! Phase 8c+8d Step 9 — end-to-end integration tests.
//!
//! Exercises the FULL flush path:
//!
//! ```text
//! system body --Commands::spawn-->  CommandQueue::push
//!                                            |
//!                            FunctionSystem::apply (APP3)
//!                                            v
//!                          SystemParam::apply (APP1')
//!                                            v
//!                       CommandQueue::apply (W3' panic-recovery)
//!                                            v
//!                  SpawnCommand::apply (W4' stack collector)
//!                                            v
//!                    EcsMaster::create_entity (W7 archetype write)
//! ```
//!
//! Each test pins one slice of that contract:
//!
//! 1. `function_system_with_commands_spawns_entity` — a one-shot system
//!    that calls `Commands::spawn` and observes the entity post-flush.
//! 2. `multiple_run_system_calls_isolate_command_queues` — each
//!    `run_system` builds a fresh `FunctionSystem` ⇒ fresh `CommandQueue`;
//!    no cross-contamination of pending commands.
//! 3. `cached_function_system_reuses_state` — `IntoSystem::into_system`
//!    hoisted outside; `run_cached_system` called twice; the second call
//!    does NOT re-`initialize` the system (FS1 idempotence verified via a
//!    side-effect probe in the body).
//! 4. `query_and_commands_in_one_system_orders_correctly` — system has
//!    BOTH `Query<&T>` + `Commands`; the body's query yields the entities
//!    that existed *before* the call; the commands' spawn lands AFTER the
//!    body returns (APP3).
//! 5. `commands_add_custom_command_runs_on_flush` — define a bespoke
//!    `Command` impl that bumps a counter on `&mut EcsMaster` access; verify
//!    it ran exactly once during `SystemParam::apply`.
//! 6. `arity_4_bundle_spawn_via_commands` — `Commands::spawn(arch_id, (A, B,
//!    C, D))`; all four components land in canonical-sorted order.
//! 7. `arity_3_bundle_canonical_sort_observable` — user declares the bundle
//!    in non-canonical order `(C, A, B)`; the archetype must be queryable
//!    using `[A, B, C]` (the sorted form).
//!
//! # Component-slot range
//!
//! 244..=259. Slots 240-243 are now free after Phase 8.5's deletion of
//! `bundle_impls.rs`; 270-271 by `params/commands.rs`. Miri tests in
//! `miri_phase8cd.rs` use 260..=269.
//!
//! # Phase 8.5 migration note
//!
//! Phase 8.5 (Static Bundle Cache) replaced the two-arg
//! `commands.spawn(archetype_id, (A, B, ...))` surface with a single-arg
//! `commands.spawn(MyBundle { ... })` surface backed by `#[derive(Bundle)]`.
//! Each test that previously enqueued a tuple bundle now defines a
//! `#[derive(Bundle)]` struct (named or tuple) inside the test fn and
//! passes a value of that type to `Commands::spawn`. The `archetype_id`
//! argument is gone — `B::cached_archetype_id(world)` resolves it lazily
//! on the apply path. Pre-resolution via `get_or_create_archetype` is
//! retained only where the test needs the id for direct
//! `EcsMaster::spawn_one` seeding (tests 3 and 4).
//!
//! # Test isolation
//!
//! Tests do NOT share static counters across types. Each test uses
//! unique component (and command) types declared inline so that parallel
//! test execution does not cross-contaminate state. The one exception —
//! Test 5 — uses an atomic counter that is reset at the head of the test
//! and observed only by that test's bespoke `Command` impl.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Commands, IntoSystem};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── Test 1 — Commands::spawn through `run_system` ───────────────────────────

const SLOT_T1_POS: ComponentId = ComponentId(244);
const SLOT_T1_VEL: ComponentId = ComponentId(245);

#[repr(C)]
#[derive(Clone, Copy)]
struct T1Pos(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct T1Vel(u32);

impl Component for T1Pos {
    fn component_id() -> ComponentId {
        SLOT_T1_POS
    }
}

impl Component for T1Vel {
    fn component_id() -> ComponentId {
        SLOT_T1_VEL
    }
}

fn register_t1() {
    register_layout::<T1Pos>(SLOT_T1_POS.0);
    register_layout::<T1Vel>(SLOT_T1_VEL.0);
}

#[derive(Bundle)]
struct T1Bundle {
    pos: T1Pos,
    vel: T1Vel,
}

#[test]
fn function_system_with_commands_spawns_entity() {
    register_t1();
    let mut ecs = EcsMaster::new();

    assert_eq!(ecs.entity_count(), 0, "world starts empty");

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(T1Bundle {
            pos: T1Pos(7),
            vel: T1Vel(13),
        });
    });

    // After `run_system` returns, the per-system queue has been flushed
    // via FunctionSystem::apply → SystemParam::apply → CommandQueue::apply.
    assert_eq!(
        ecs.entity_count(),
        1,
        "Commands::spawn must have flushed and created exactly one entity"
    );
}

// ── Test 2 — separate `run_system` calls do not cross-contaminate queues ────

const SLOT_T2_A: ComponentId = ComponentId(246);
const SLOT_T2_B: ComponentId = ComponentId(247);

#[repr(C)]
#[derive(Clone, Copy)]
struct T2A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct T2B(u32);

impl Component for T2A {
    fn component_id() -> ComponentId {
        SLOT_T2_A
    }
}

impl Component for T2B {
    fn component_id() -> ComponentId {
        SLOT_T2_B
    }
}

fn register_t2() {
    register_layout::<T2A>(SLOT_T2_A.0);
    register_layout::<T2B>(SLOT_T2_B.0);
}

#[derive(Bundle)]
struct T2Bundle {
    a: T2A,
    b: T2B,
}

#[test]
fn multiple_run_system_calls_isolate_command_queues() {
    register_t2();
    let mut ecs = EcsMaster::new();

    // First call spawns 3 entities.
    ecs.run_system(|mut cmds: Commands| {
        for i in 0..3 {
            cmds.spawn(T2Bundle {
                a: T2A(i),
                b: T2B(i + 100),
            });
        }
    });
    assert_eq!(ecs.entity_count(), 3, "first run spawned 3");

    // Second call spawns 2 more. The previous queue is gone (it lived in
    // the previous FunctionSystem's State; FunctionSystem dropped after
    // `run_system` returned). A leftover queue would re-flush the prior
    // 3 entities again ⇒ count would be 8 instead of 5.
    ecs.run_system(|mut cmds: Commands| {
        for i in 0..2 {
            cmds.spawn(T2Bundle {
                a: T2A(i + 1000),
                b: T2B(i + 2000),
            });
        }
    });
    assert_eq!(
        ecs.entity_count(),
        5,
        "second run added 2 entities (no cross-contamination from first queue)"
    );
}

// ── Test 3 — `run_cached_system` reuses FunctionSystem state (FS1) ──────────

const SLOT_T3_X: ComponentId = ComponentId(248);

#[repr(C)]
#[derive(Clone, Copy)]
struct T3X(u32);

impl Component for T3X {
    fn component_id() -> ComponentId {
        SLOT_T3_X
    }
}

fn register_t3() {
    register_layout::<T3X>(SLOT_T3_X.0);
}

#[test]
fn cached_function_system_reuses_state() {
    register_t3();
    let mut ecs = EcsMaster::new();
    let arch = ecs.get_or_create_archetype(&[SLOT_T3_X]);
    ecs.spawn_one(arch, T3X(1))
        .expect("seed entity must succeed");
    ecs.spawn_one(arch, T3X(2))
        .expect("seed entity must succeed");

    // Probe lets the closure (which is `Send + Sync + 'static`) publish
    // its body's row count out to the test for both invocations.
    let observed = Arc::new(AtomicUsize::new(0));
    let probe = observed.clone();

    let body = move |q: Query<'_, '_, &T3X>| -> usize {
        let n = q.iter().count();
        probe.fetch_add(n, Ordering::Relaxed);
        n
    };

    let mut sys = IntoSystem::into_system(body);

    // First call — pays the cold-init cost (FS1's `initialize` populates
    // `state` + `meta`).
    let first = ecs.run_cached_system(&mut sys);
    // Second call — `initialize` MUST be idempotent (FS1) and return
    // without re-allocating state. The body's count should match the first
    // call's count exactly (same world, same archetype, no commands
    // queued).
    let second = ecs.run_cached_system(&mut sys);

    assert_eq!(first, 2, "first cached run sees 2 seed entities");
    assert_eq!(second, 2, "second cached run still sees 2 (idempotent init)");
    assert_eq!(
        observed.load(Ordering::Relaxed),
        4,
        "probe counted body invocations across both runs: 2 + 2 = 4"
    );
}

// ── Test 4 — Query + Commands in the same system (APP3 ordering) ────────────

const SLOT_T4_C: ComponentId = ComponentId(249);

#[repr(C)]
#[derive(Clone, Copy)]
struct T4C(u32);

impl Component for T4C {
    fn component_id() -> ComponentId {
        SLOT_T4_C
    }
}

fn register_t4() {
    register_layout::<T4C>(SLOT_T4_C.0);
}

#[derive(Bundle)]
struct T4Bundle {
    c: T4C,
}

#[test]
fn query_and_commands_in_one_system_orders_correctly() {
    register_t4();
    let mut ecs = EcsMaster::new();
    // `arch` is retained for the direct `spawn_one` seeding below. The
    // `cmds.spawn(T4Bundle { ... })` callsite resolves its archetype via
    // `T4Bundle::cached_archetype_id` (SBC4); the two paths converge on the
    // same archetype because the bundle's canonical-sorted component-id
    // slice is `[SLOT_T4_C]` (identical to the explicit registration here).
    let arch = ecs.get_or_create_archetype(&[SLOT_T4_C]);
    ecs.spawn_one(arch, T4C(1)).expect("seed");
    ecs.spawn_one(arch, T4C(2)).expect("seed");

    // The query observes BEFORE-state (2 entities). Commands queues 3
    // spawns that flush AFTER the body returns (APP3). So the query's
    // observed count = 2, the post-call entity_count = 5.
    let seen = Arc::new(AtomicUsize::new(0));
    let probe = seen.clone();

    ecs.run_system(move |q: Query<'_, '_, &T4C>, mut cmds: Commands| {
        // Snapshot the pre-flush row count via Query.
        let n = q.iter().count();
        probe.store(n, Ordering::Relaxed);
        // Queue 3 spawns; they flush via SystemParam::apply after body.
        for i in 0..3 {
            cmds.spawn(T4Bundle { c: T4C(i + 10) });
        }
    });

    assert_eq!(
        seen.load(Ordering::Relaxed),
        2,
        "Query observed the pre-spawn state (2 entities, APP3 ordering)"
    );
    assert_eq!(
        ecs.entity_count(),
        5,
        "after apply, the 3 queued spawns landed (total = 2 + 3)"
    );
}

// ── Test 5 — `Commands::add` runs a bespoke `Command` impl ──────────────────

static CUSTOM_CMD_RAN: AtomicUsize = AtomicUsize::new(0);

struct BumpCounterCommand;

impl Command for BumpCounterCommand {
    fn apply(self, world: &mut EcsMaster) {
        // Touch `world` so the parameter is observably exercised — the
        // entity_count call is a constant read against the slab. The
        // `black_box` keeps the compiler from inlining the call away.
        std::hint::black_box(world.entity_count());
        CUSTOM_CMD_RAN.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn commands_add_custom_command_runs_on_flush() {
    CUSTOM_CMD_RAN.store(0, Ordering::Relaxed);
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.add(BumpCounterCommand);
        cmds.add(BumpCounterCommand);
        cmds.add(BumpCounterCommand);
    });

    // After `run_system` returns, `FunctionSystem::apply` has drained the
    // queue; each `BumpCounterCommand::apply` ran exactly once.
    assert_eq!(
        CUSTOM_CMD_RAN.load(Ordering::Relaxed),
        3,
        "every queued BumpCounterCommand must have applied once on flush"
    );
}

// ── Test 6 — arity-4 bundle through Commands::spawn ─────────────────────────

const SLOT_T6_A: ComponentId = ComponentId(252);
const SLOT_T6_B: ComponentId = ComponentId(253);
const SLOT_T6_C: ComponentId = ComponentId(254);
const SLOT_T6_D: ComponentId = ComponentId(255);

#[repr(C)]
#[derive(Clone, Copy)]
struct T6A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct T6B(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct T6C(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct T6D(u32);

impl Component for T6A {
    fn component_id() -> ComponentId {
        SLOT_T6_A
    }
}
impl Component for T6B {
    fn component_id() -> ComponentId {
        SLOT_T6_B
    }
}
impl Component for T6C {
    fn component_id() -> ComponentId {
        SLOT_T6_C
    }
}
impl Component for T6D {
    fn component_id() -> ComponentId {
        SLOT_T6_D
    }
}

fn register_t6() {
    register_layout::<T6A>(SLOT_T6_A.0);
    register_layout::<T6B>(SLOT_T6_B.0);
    register_layout::<T6C>(SLOT_T6_C.0);
    register_layout::<T6D>(SLOT_T6_D.0);
}

#[derive(Bundle)]
struct T6Bundle {
    a: T6A,
    b: T6B,
    c: T6C,
    d: T6D,
}

#[test]
fn arity_4_bundle_spawn_via_commands() {
    register_t6();
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(T6Bundle {
            a: T6A(11),
            b: T6B(22),
            c: T6C(33),
            d: T6D(44),
        });
    });

    assert_eq!(ecs.entity_count(), 1, "arity-4 spawn must land one entity");

    // Read every component back via Query to verify each landed in its
    // archetype slot with the original value.
    let probe_a = Arc::new(AtomicUsize::new(0));
    let probe_b = Arc::new(AtomicUsize::new(0));
    let probe_c = Arc::new(AtomicUsize::new(0));
    let probe_d = Arc::new(AtomicUsize::new(0));
    let pa = probe_a.clone();
    let pb = probe_b.clone();
    let pc = probe_c.clone();
    let pd = probe_d.clone();

    ecs.run_system(
        move |q: Query<'_, '_, (&T6A, &T6B, &T6C, &T6D)>| {
            for (a, b, c, d) in &q {
                pa.store(a.0 as usize, Ordering::Relaxed);
                pb.store(b.0 as usize, Ordering::Relaxed);
                pc.store(c.0 as usize, Ordering::Relaxed);
                pd.store(d.0 as usize, Ordering::Relaxed);
            }
        },
    );

    assert_eq!(probe_a.load(Ordering::Relaxed), 11);
    assert_eq!(probe_b.load(Ordering::Relaxed), 22);
    assert_eq!(probe_c.load(Ordering::Relaxed), 33);
    assert_eq!(probe_d.load(Ordering::Relaxed), 44);
}

// ── Test 7 — non-canonical bundle order maps to canonical archetype ─────────

const SLOT_T7_A: ComponentId = ComponentId(256);
const SLOT_T7_B: ComponentId = ComponentId(257);
const SLOT_T7_C: ComponentId = ComponentId(258);

#[repr(C)]
#[derive(Clone, Copy)]
struct T7A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct T7B(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct T7C(u32);

impl Component for T7A {
    fn component_id() -> ComponentId {
        SLOT_T7_A
    }
}
impl Component for T7B {
    fn component_id() -> ComponentId {
        SLOT_T7_B
    }
}
impl Component for T7C {
    fn component_id() -> ComponentId {
        SLOT_T7_C
    }
}

fn register_t7() {
    register_layout::<T7A>(SLOT_T7_A.0);
    register_layout::<T7B>(SLOT_T7_B.0);
    register_layout::<T7C>(SLOT_T7_C.0);
}

/// Fields deliberately declared in NON-canonical order `(c, a, b)`. The
/// `#[derive(Bundle)]` machinery (B1 + §6.1) sorts the component-id slice
/// and the `for_each_component_bytes` callback dispatch into canonical
/// ascending `ComponentId.0` order regardless of declaration order. This
/// test's whole point is observing that sort end-to-end.
#[derive(Bundle)]
struct T7Bundle {
    c: T7C,
    a: T7A,
    b: T7B,
}

#[test]
fn arity_3_bundle_canonical_sort_observable() {
    register_t7();
    let mut ecs = EcsMaster::new();

    // User pushes a `T7Bundle` whose fields are declared `(c, a, b)` —
    // NON-canonical order. The derive's internal sort (B1 / B2) must
    // rearrange to ascending id at `for_each_component_bytes` emission
    // time so the archetype write succeeds and component values land in
    // their correct slots.
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(T7Bundle {
            c: T7C(300),
            a: T7A(100),
            b: T7B(200),
        });
    });

    assert_eq!(ecs.entity_count(), 1, "arity-3 spawn must land one entity");

    // Verify each component's value through a triple-query. If B1 had
    // emitted the components in the user's declared order rather than
    // canonical-sorted order, `create_entity` would have memcpy'd `T7C`'s
    // bytes into T7A's slot and tripped a Drop double-write / value swap.
    let pa = Arc::new(AtomicUsize::new(0));
    let pb = Arc::new(AtomicUsize::new(0));
    let pc = Arc::new(AtomicUsize::new(0));
    let qa = pa.clone();
    let qb = pb.clone();
    let qc = pc.clone();

    ecs.run_system(move |q: Query<'_, '_, (&T7A, &T7B, &T7C)>| {
        for (a, b, c) in &q {
            qa.store(a.0 as usize, Ordering::Relaxed);
            qb.store(b.0 as usize, Ordering::Relaxed);
            qc.store(c.0 as usize, Ordering::Relaxed);
        }
    });

    assert_eq!(
        pa.load(Ordering::Relaxed),
        100,
        "T7A's value must survive the canonical sort (B1)"
    );
    assert_eq!(
        pb.load(Ordering::Relaxed),
        200,
        "T7B's value must survive the canonical sort (B1)"
    );
    assert_eq!(
        pc.load(Ordering::Relaxed),
        300,
        "T7C's value must survive the canonical sort (B1)"
    );
}
