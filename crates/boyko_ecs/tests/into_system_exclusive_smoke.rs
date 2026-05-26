//! Phase 9 Wave 3 Step 8 smoke tests for the
//! [`ExclusiveSystemMarker`] blanket impl of [`IntoSystem`].
//!
//! Validates the user-facing path:
//!
//! ```text
//! fn body(world: &mut EcsMaster) { ... }
//! let mut sys = IntoSystem::into_system(body);
//! ecs.run_system_once(&mut sys);
//! ```
//!
//! and the coherence proof (§3 Q9.2 of the Phase 9 plan): the new
//! exclusive blanket coexists with the Phase 8c
//! `SystemParamFunction`-based blanket. Both impls compile in the same
//! translation unit, applied to closures of incompatible shapes, without
//! `IntoSystem` becoming ambiguous.
//!
//! [`ExclusiveSystemMarker`]: boyko_ecs::ecs::core::system::ExclusiveSystemMarker
//! [`IntoSystem`]: boyko_ecs::ecs::core::system::IntoSystem

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::IntoSystem;
use boyko_ecs::ecs::core::system::system::System;

/// Global probe counter — closures and fn-items below increment it so the
/// test can assert each body executed exactly once. `AtomicUsize` is
/// `Send + Sync`, satisfying the `Fn(&mut EcsMaster) + Send + Sync +
/// 'static` bound on the blanket.
static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Free fn-item exclusive body. Captures no state; the function
/// identifier itself is the closure substitute (the `IntoSystem` blanket
/// must accept named fn items as readily as closures).
fn exclusive_system(_world: &mut EcsMaster) {
    COUNTER.fetch_add(1, Ordering::Relaxed);
}

/// `IntoSystem::into_system(fn_item)` infers the
/// `(ExclusiveSystemMarker, fn(&mut EcsMaster))` blanket, produces an
/// `ExclusiveFunctionSystem<F>`, and `run_system_once` drives it end-to-end.
#[test]
fn into_system_exclusive_fn_compiles_and_runs() {
    COUNTER.store(0, Ordering::Relaxed);
    let mut world = EcsMaster::new();
    let mut sys = IntoSystem::into_system(exclusive_system);
    world.run_system_once(&mut sys);
    assert_eq!(
        COUNTER.load(Ordering::Relaxed),
        1,
        "exclusive system body must execute exactly once via run_system_once"
    );
}

/// Bare closure form — `|w: &mut EcsMaster| { ... }` resolves through the
/// new blanket without an explicit turbofish. Mirrors the Phase 8c
/// closure-inference reproducer for the `SystemParamFunction` path.
#[test]
fn into_system_exclusive_closure_works() {
    static C2: AtomicUsize = AtomicUsize::new(0);
    C2.store(0, Ordering::Relaxed);

    let mut world = EcsMaster::new();
    let mut sys = IntoSystem::into_system(|_w: &mut EcsMaster| {
        C2.fetch_add(1, Ordering::Relaxed);
    });
    world.run_system_once(&mut sys);
    assert_eq!(C2.load(Ordering::Relaxed), 1);
}

/// EXC2 acceptance — once `initialize` has run (implicitly via
/// `run_system_once`), the system's `access()` must report
/// `is_universal == true`. `SystemBox::new` (Wave 3 Step 8) will cache
/// this bit at build time.
#[test]
fn exclusive_system_has_universal_access() {
    let mut world = EcsMaster::new();
    let mut sys = IntoSystem::into_system(exclusive_system);
    // Drive a single dispatch to exercise `initialize`; access stays
    // identical post-init because EXC2 seeds it at construction.
    world.run_system_once(&mut sys);
    assert!(
        sys.access().is_universal(),
        "exclusive system must declare Access::universal()"
    );
}

/// Coherence smoke — both `IntoSystem` blankets coexist. A bare param-based
/// closure (`SystemParamFunction` blanket) and a `&mut EcsMaster` closure
/// (exclusive blanket) compile side by side. If the impls overlapped, the
/// coherence checker would reject the second `into_system` call at
/// compile time.
///
/// Phase 9 plan §3 Q9.2: the marker tuples
/// `(IsFunctionSystem, _)` and `(ExclusiveSystemMarker, _)` are nominal
/// distinct ZSTs; `&mut EcsMaster` is not a `SystemParam`; therefore the
/// two impl heads do not overlap on any single closure type.
#[test]
fn into_system_exclusive_and_param_blankets_coexist() {
    let mut world = EcsMaster::new();

    // Param-based blanket — empty-tuple `Param = ()` arity-0 closure
    // resolves through the Phase 8c `SystemParamFunction` blanket.
    let mut param_sys = IntoSystem::into_system(|| {
        // Body intentionally empty — we are testing trait resolution.
    });
    world.run_system_once(&mut param_sys);

    // Exclusive blanket — `&mut EcsMaster` closure resolves through the
    // Phase 9 `ExclusiveSystemMarker` blanket.
    let mut excl_sys = IntoSystem::into_system(|_w: &mut EcsMaster| {});
    world.run_system_once(&mut excl_sys);

    // The two system types differ; if both calls type-check, coherence
    // held. The post-init `access()` surfaces also differ — exclusive
    // is universal, param-based is empty.
    assert!(excl_sys.access().is_universal());
    assert!(
        !param_sys.access().is_universal(),
        "empty-param closure must NOT have universal access"
    );
}
