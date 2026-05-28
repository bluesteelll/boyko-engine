//! Phase 13 §6.2 — behavioral integration tests for the `Local<'s, T>`
//! `SystemParam`.
//!
//! These tests prove the three claims that the in-file unit tests
//! (`params/local.rs::tests`) cannot reach because they require the *running*
//! system machinery rather than direct trait calls:
//!
//! 1. **Cross-run persistence** — a `Local<T>` keeps its value between runs of
//!    the *same cached* `FunctionSystem` (frame-to-frame under the scheduler).
//! 2. **Positional distinctness** — two `Local<T>` of the same `T` in one
//!    system get two independent slots (the tuple `State = (T, T)`); and two
//!    *separate* systems each get their own slot.
//! 3. **No access** — a `Local` declares nothing on the conflict graph, so its
//!    system's `Access` is empty (does not conflict with a universal writer).
//!
//! # R1 — persistence requires a *cached* system (read twice)
//!
//! `Local`'s `T` lives in `FunctionSystem::state`, which only persists if the
//! SAME `FunctionSystem` instance is reused across runs. Therefore every
//! persistence test below builds the system ONCE via
//! [`IntoSystem::into_system`], hoists the resulting `FunctionSystem` into a
//! `let mut sys`, and calls [`EcsMaster::run_cached_system`] repeatedly.
//!
//! Using `ecs.run_system(closure)` / `run_closure_once(closure)` here would be
//! WRONG: each call rebuilds a fresh `FunctionSystem` (fresh `T::default()`),
//! so a counter would read `1, 1, 1` instead of `1, 2, 3` and falsely look
//! like a bug. Template: `run_cached_system_reused_twice_reads_updated_resource`
//! (`ecs_master.rs:2548`).
//!
//! # Probe mechanism
//!
//! Each system body publishes the observed `Local` value through an
//! `Arc<AtomicU32>` captured by a `move` closure. `IntoSystem::into_system`
//! accepts capturing closures as long as the closure is `Send + Sync +
//! 'static` (an `Arc<AtomicU32>` is), exactly as the cached-system template and
//! `system_param_smoke.rs` already do. A per-test local `Arc` (rather than a
//! module-level `static`) keeps the tests independent and concurrency-safe.
//!
//! [`EcsMaster::run_cached_system`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::run_cached_system

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::{Access, FunctionSystem, IntoSystem, Local, System};

/// Test 1 — a `Local<u32>` counter persists across runs of the same cached
/// system. Running the hoisted `FunctionSystem` three times must observe the
/// monotonically increasing sequence `1, 2, 3`, proving the state slot is
/// reused (FS1 idempotent `initialize`) rather than reset per call.
#[test]
fn local_counter_persists_across_runs() {
    let mut ecs = EcsMaster::new();

    let observed = Arc::new(AtomicU32::new(0));
    let probe = Arc::clone(&observed);
    // Build once, hoist — state survives across `run_cached_system` calls.
    let body = move |mut n: Local<u32>| {
        *n += 1;
        probe.store(*n, Ordering::Relaxed);
    };
    let mut sys: FunctionSystem<_, _> = IntoSystem::into_system(body);

    ecs.run_cached_system(&mut sys);
    assert_eq!(
        observed.load(Ordering::Relaxed),
        1,
        "first run: Local<u32> default 0 incremented to 1"
    );

    ecs.run_cached_system(&mut sys);
    assert_eq!(
        observed.load(Ordering::Relaxed),
        2,
        "second run: Local must persist, 1 -> 2 (not reset to default)"
    );

    ecs.run_cached_system(&mut sys);
    assert_eq!(
        observed.load(Ordering::Relaxed),
        3,
        "third run: Local must persist, 2 -> 3"
    );
}

/// Test 2 — two `Local<u32>` of the *same* type in one system address two
/// independent slots. After two runs, the first local (step +1) reads `2` and
/// the second (step +10) reads `20`. Equal values would prove the slots
/// aliased; distinct values prove positional distinctness via the tuple
/// `State = (u32, u32)`.
#[test]
fn two_locals_same_type_independent() {
    let mut ecs = EcsMaster::new();

    let observed_a = Arc::new(AtomicU32::new(0));
    let observed_b = Arc::new(AtomicU32::new(0));
    let probe_a = Arc::clone(&observed_a);
    let probe_b = Arc::clone(&observed_b);
    let body = move |mut a: Local<u32>, mut b: Local<u32>| {
        *a += 1;
        *b += 10;
        probe_a.store(*a, Ordering::Relaxed);
        probe_b.store(*b, Ordering::Relaxed);
    };
    let mut sys: FunctionSystem<_, _> = IntoSystem::into_system(body);

    ecs.run_cached_system(&mut sys);
    ecs.run_cached_system(&mut sys);

    assert_eq!(
        observed_a.load(Ordering::Relaxed),
        2,
        "first Local (step +1) must accumulate independently to 2"
    );
    assert_eq!(
        observed_b.load(Ordering::Relaxed),
        20,
        "second Local (step +10) must accumulate independently to 20"
    );
}

/// Test 3 — two *separate* cached systems each own a private `Local<u32>`.
/// Running system A twice and system B once must leave A's probe at `2` and
/// B's probe at `1`: the locals do not leak across systems (each
/// `FunctionSystem` holds its own `state`).
#[test]
fn two_systems_independent_locals() {
    let mut ecs = EcsMaster::new();

    let observed_a = Arc::new(AtomicU32::new(0));
    let observed_b = Arc::new(AtomicU32::new(0));
    let probe_a = Arc::clone(&observed_a);
    let probe_b = Arc::clone(&observed_b);

    let body_a = move |mut n: Local<u32>| {
        *n += 1;
        probe_a.store(*n, Ordering::Relaxed);
    };
    let body_b = move |mut n: Local<u32>| {
        *n += 1;
        probe_b.store(*n, Ordering::Relaxed);
    };
    let mut sys_a: FunctionSystem<_, _> = IntoSystem::into_system(body_a);
    let mut sys_b: FunctionSystem<_, _> = IntoSystem::into_system(body_b);

    ecs.run_cached_system(&mut sys_a);
    ecs.run_cached_system(&mut sys_a);
    ecs.run_cached_system(&mut sys_b);

    assert_eq!(
        observed_a.load(Ordering::Relaxed),
        2,
        "system A ran twice: its Local must read 2"
    );
    assert_eq!(
        observed_b.load(Ordering::Relaxed),
        1,
        "system B ran once: its own Local must read 1, unaffected by A"
    );
}

/// Test 4 — first access default-initializes the `Local` from `T::default()`,
/// not from a zeroed slot. `Counter` has a manual `Default` returning `42`; a
/// single run must observe `42`, proving B1 default-init forwards the real
/// `Default` value rather than `mem::zeroed`-style `0`.
#[test]
fn default_init_uses_default_value() {
    // A non-trivial `Default` whose value (42) is distinguishable from the
    // zero a naive slot would carry.
    #[derive(Clone, Copy)]
    struct Counter(u32);
    impl Default for Counter {
        fn default() -> Self {
            Counter(42)
        }
    }

    let mut ecs = EcsMaster::new();

    let observed = Arc::new(AtomicU32::new(0));
    let probe = Arc::clone(&observed);
    let body = move |c: Local<Counter>| {
        // Read through `Deref` — `c.0` reaches the inner `u32`.
        probe.store(c.0, Ordering::Relaxed);
    };
    let mut sys: FunctionSystem<_, _> = IntoSystem::into_system(body);

    ecs.run_cached_system(&mut sys);

    assert_eq!(
        observed.load(Ordering::Relaxed),
        42,
        "Local<Counter> must be default-initialized to Counter::default() == 42"
    );
}

/// Test 5 — a `Local` declares NO access, so its system adds no conflict-graph
/// edge. After `initialize` populates the declared access surface, the
/// system's `Access` must NOT conflict with `Access::universal()` (a writer of
/// every component and every resource). A universal writer conflicts with any
/// *non-empty* access; non-conflict therefore proves the surface is empty.
///
/// This is the inverse of `res.rs::init_access_adds_resource_read_to_set`
/// (which asserts a conflict *exists*): here we assert a conflict is *absent*.
#[test]
fn local_registers_no_access() {
    let mut ecs = EcsMaster::new();

    let body = |_: Local<u32>| {};
    let mut sys: FunctionSystem<_, _> = IntoSystem::into_system(body);

    // `initialize` runs `init_access` for every param; `Local::init_access`
    // declares nothing, so the finalized `Access` must stay empty.
    sys.initialize(&mut ecs);

    let universal = Access::universal();
    assert!(
        !sys.access().conflicts_with(&universal),
        "Local declares no access: its system must not conflict with a \
         universal writer (an empty access conflicts with nothing)"
    );
    // Symmetric direction — the relation is symmetric, but assert it to rule
    // out a one-sided bitmask error.
    assert!(
        !universal.conflicts_with(sys.access()),
        "conflict relation is symmetric: universal writer must not conflict \
         with the Local-only system either"
    );
}
