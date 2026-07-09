//! EnableTag D4 (Step 8) — POSITIVE regression: `Added<C>` / `Changed<C>` on a
//! NORMAL (table-storage) component still compile AND run.
//!
//! The D4 storage-shape const-assert
//! (`filter::Added::assert_storage_supports_change_detection`) compile-rejects
//! `Added<C>` / `Changed<C>` only when `C::STORAGE_IS_BITSET == true`. A normal
//! `#[derive(Component)]` type keeps the trait default `STORAGE_IS_BITSET ==
//! false`, so the const-assert is a no-op and the monomorphization compiles.
//! The trybuild rejection fixtures live in `tests/enable_filter_compile_fail/`
//! (`added_on_bitset_tag_rejected.rs` / `changed_on_bitset_tag_rejected.rs`).
//!
//! These tests drive a real `Schedule`, which forces CODEGEN of
//! `Added::<C>::init_state` / `Changed::<C>::init_state` — the codegen-time
//! `const {}` trigger inside `init_state` is therefore evaluated here. A
//! regression that flipped a normal component's `STORAGE_IS_BITSET` (or broke
//! the default) would fail this test at compile time.
//!
//! # Component-id minting
//!
//! `#[derive(Component)]` mints each id lazily via `register_new` (a process
//! atomic), so these fixtures never collide with the shared lib-test id space
//! regardless of execution order — no fixed ids (Step 7's lazy-mint lesson).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_macros::Component;
use boyko_threadpool::ThreadPoolBuilder;

#[derive(Component)]
#[repr(C)]
struct D4NormalAdded {
    x: u32,
}

#[derive(Component)]
#[repr(C)]
struct D4NormalChanged {
    hp: u32,
}

/// The trait default `STORAGE_IS_BITSET == false` holds for a normal derived
/// component (the const-assert input D4 reads). Pinned in `const {}` blocks so
/// a regression is a compile error, not just a runtime assertion.
#[test]
fn normal_component_storage_is_not_bitset() {
    const { assert!(!D4NormalAdded::STORAGE_IS_BITSET) };
    const { assert!(!D4NormalChanged::STORAGE_IS_BITSET) };
}

/// `Query<&C, Added<C>>` for a normal table-storage `C` compiles and runs:
/// the D4 const-assert passes (no bitset storage), and `Added` matches the
/// pre-existing row on frame 1.
#[test]
fn added_on_normal_component_still_compiles_and_runs() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[D4NormalAdded::component_id()]);
    world
        .spawn_one(arch, D4NormalAdded { x: 7 })
        .expect("spawn");

    static MATCHES: AtomicUsize = AtomicUsize::new(0);
    MATCHES.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&D4NormalAdded, Added<D4NormalAdded>>| {
        for _ in &q {
            MATCHES.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(
        MATCHES.load(Ordering::Relaxed),
        1,
        "Added<C> on a normal component must match the pre-existing row on frame 1"
    );
}

/// `Query<&C, Changed<C>>` for a normal table-storage `C` compiles and runs:
/// the D4 const-assert passes. The insert bumps the row's `changed` tick, so
/// `Changed` matches the pre-existing row on frame 1.
#[test]
fn changed_on_normal_component_still_compiles_and_runs() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[D4NormalChanged::component_id()]);
    world
        .spawn_one(arch, D4NormalChanged { hp: 100 })
        .expect("spawn");

    static MATCHES: AtomicUsize = AtomicUsize::new(0);
    MATCHES.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&D4NormalChanged, Changed<D4NormalChanged>>| {
        for _ in &q {
            MATCHES.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(
        MATCHES.load(Ordering::Relaxed),
        1,
        "Changed<C> on a normal component must match the freshly-inserted row on frame 1"
    );
}
