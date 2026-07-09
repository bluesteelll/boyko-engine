//! End-to-end smoke tests for the Phase 8a `SystemParam` facade.
//!
//! Exercises the public surface delivered in Steps 7 / 8 / 9:
//! - [`EcsMaster::insert_resource`] / [`remove_resource`] / [`contains_resource`]
//! - [`EcsMaster::resource`] / [`resource_mut`] / [`try_resource`] / [`try_resource_mut`]
//! - [`EcsMaster::run_closure_once`] reading via `Res<R>` and writing via
//!   `ResMut<R>`.
//! - Intra-system conflict detection (`FilteredAccessSet` / boyko-B0002).
//! - Missing-resource panic from `Res<R>::get_param`.
//! - Drop-once-on-replace + drop-on-`EcsMaster::drop` semantics (R3/R4).
//!
//! # `#[derive(Resource)]` reach-through
//!
//! `boyko-macros` lives in `[dev-dependencies]` of `boyko-ecs`, so the
//! derive is available to integration tests under `tests/` without polluting
//! the library's runtime dependencies. The derive expands to a `Resource`
//! impl that calls `register_new::<Self>()` through the per-type `OnceLock`,
//! mirroring `#[derive(Component)]`.
//!
//! [`EcsMaster::insert_resource`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::insert_resource
//! [`remove_resource`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::remove_resource
//! [`contains_resource`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::contains_resource
//! [`EcsMaster::resource`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::resource
//! [`resource_mut`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::resource_mut
//! [`try_resource`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::try_resource
//! [`try_resource_mut`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::try_resource_mut
//! [`EcsMaster::run_closure_once`]: boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster::run_closure_once

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;

// Test resources. Each type carries its own per-type `OnceLock<ResourceId>`
// via `#[derive(Resource)]`, so registration is one-shot and stable for the
// lifetime of the process.

#[derive(Resource)]
struct Tick(u32);

#[derive(Resource)]
struct Score(i32);

/// `run_closure_once` with a `ResMut<Tick>` param mutates the resource in
/// place; the side effect is observable via the public `resource::<Tick>()`
/// facade after the call.
///
/// Phase 8c Step 5: the W3 turbofish form `::<ResMut<'_, Tick>, _, _>` is
/// replaced by a closure-arg annotation `|t: ResMut<Tick>|` — `IntoSystem`
/// infers the `SystemParam` tuple from the signature.
#[test]
fn run_closure_once_resmut_increments_resource() {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Tick(0));
    ecs.run_closure_once(|mut tick: ResMut<Tick>| {
        tick.0 += 1;
    });
    assert_eq!(
        ecs.resource::<Tick>().0,
        1,
        "ResMut increment must round-trip through `EcsMaster::resource`"
    );
}

/// `run_closure_once` with a `Res<Score>` param reads the resource and
/// publishes the observed value through an `AtomicI32` probe. The probe
/// pattern is required because the closure must be `Send + Sync + 'static`
/// per the `System` trait bound — we cannot capture a `&mut` of a local
/// stack variable.
#[test]
fn run_closure_once_res_reads_resource() {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Score(42));
    let observed = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
    let probe = observed.clone();
    ecs.run_closure_once(move |score: Res<Score>| {
        // `score: Res<Score>` derefs to `Score`; `(*score).0: i32`. The inner
        // `&'w R` field of `Res` is `pub(crate)`, so external crates must
        // reach the resource through `Deref` rather than field access.
        probe.store((*score).0, std::sync::atomic::Ordering::Relaxed);
    });
    assert_eq!(
        observed.load(std::sync::atomic::Ordering::Relaxed),
        42,
        "Res<R> must observe the value inserted via `insert_resource`"
    );
}

/// Declaring both `Res<Tick>` and `ResMut<Tick>` in the same system body is
/// an intra-system aliasing conflict. `FilteredAccessSet::add_resource_*`
/// (C4 + M8 resolution) catches the conflict at `init_access` time and
/// panics with the `boyko-B0002` diagnostic before the system body runs.
#[test]
#[should_panic(expected = "boyko-B0002")]
fn res_and_resmut_same_type_in_same_system_panics() {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Tick(0));
    ecs.run_closure_once(|(_r, _w): (Res<Tick>, ResMut<Tick>)| {});
}

/// `Res<R>::get_param` panics via the cold-path diagnostic helper when no
/// resource of type `R` has been inserted into the world. The wording is
/// distinct from the facade `EcsMaster::resource` panic so the user can
/// tell the two call sites apart.
#[test]
#[should_panic(expected = "not registered")]
fn res_on_unregistered_resource_panics() {
    let mut ecs = EcsMaster::new();
    // No `insert_resource::<Tick>` — `Res<Tick>::get_param` must panic.
    ecs.run_closure_once(|_: Res<Tick>| {});
}

/// `insert_resource` → `contains_resource` → `remove_resource` →
/// `try_resource` round-trips the facade in a single test.
#[test]
fn insert_then_remove_round_trip() {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Tick(7));
    assert!(
        ecs.contains_resource::<Tick>(),
        "contains_resource must be true after insert"
    );
    let removed = ecs.remove_resource::<Tick>();
    assert!(
        matches!(removed, Some(Tick(7))),
        "remove_resource must return the typed value"
    );
    assert!(
        !ecs.contains_resource::<Tick>(),
        "contains_resource must be false after remove"
    );
    assert!(
        ecs.try_resource::<Tick>().is_none(),
        "try_resource must be None after remove"
    );
}

/// Re-inserting a resource of the same type runs `R::drop` exactly once on
/// the previous value (R4 — clear-bit-first replace), and the final
/// `EcsMaster::drop` runs it again on the second value (R3 — slab walk).
#[test]
fn insert_replace_drops_old_value() {
    use std::sync::atomic::{AtomicU32, Ordering};
    static DROP_COUNT: AtomicU32 = AtomicU32::new(0);

    #[derive(Resource)]
    struct Counter(#[allow(dead_code)] u32);
    impl Drop for Counter {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }

    DROP_COUNT.store(0, Ordering::Relaxed);
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Counter(1));
    ecs.insert_resource(Counter(2));
    // First insert's value dropped during the replace (R4).
    assert_eq!(
        DROP_COUNT.load(Ordering::Relaxed),
        1,
        "exactly one drop must run during the replace"
    );
    drop(ecs);
    // Second insert's value dropped during `EcsMaster::drop` (R3).
    assert_eq!(
        DROP_COUNT.load(Ordering::Relaxed),
        2,
        "after EcsMaster::drop, total drop count must be 2 (old + final)"
    );
}
