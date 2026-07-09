//! Dense plan D4 — Miri-TB coverage for the per-slot change-detection paths.
//!
//! The full change-detection suite (`dense_d4_change_detection.rs`) drives the
//! PARALLEL scheduler, whose crossbeam work-stealing internals Miri cannot model
//! (the same limitation the Phase-9 executor suite documents — loom is the
//! concurrency oracle there). This file exercises the SAME dense tick read/write
//! `unsafe` single-threaded via `EcsMaster::run_closure_once` (the
//! `miri_phase10.rs` pattern), so the actual `unsafe` — the dense `Mut` deref
//! changed-tick bump, the `Added`/`Changed` per-slot tick read, and the
//! insert/reuse tick stamping — is UB-checked without the threadpool.
//!
//! Run (Tree Borrows, the project oracle; `-Zmiri-ignore-leaks` for the
//! documented Commands-apply RawVec leak):
//!
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks -Zmiri-disable-isolation" \
//!   cargo +nightly miri test --test dense_d4_miri
//! ```

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Mut, Query};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

/// 16-byte POD dense body (the physics-body shape).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct MBody {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

/// A table key so the dense entity lives in a real archetype.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct MKey {
    k: u32,
}

#[derive(Bundle)]
struct KeyBody {
    key: MKey,
    body: MBody,
}

#[derive(Bundle)]
struct BodyOnly {
    body: MBody,
}

/// Single-thread dense `Mut` deref bump + `Changed`/`Added` slot reads — exercises
/// the `UnsafeCell<Tick>` write/read into the dense column's per-slot tick
/// sub-regions through `row_ptr(slot)` provenance.
#[test]
fn miri_dense_mut_deref_and_change_reads_no_ub() {
    let mut ecs = EcsMaster::new();
    ecs.run_system(|mut cmds: Commands| {
        for i in 0..4u32 {
            cmds.spawn(KeyBody {
                key: MKey { k: i },
                body: MBody { x: i as f32, y: 0.0, z: 0.0, w: 0.0 },
            });
        }
    });

    // Mut deref writes through the dense slot's data pointer AND bumps the dense
    // column's changed-tick at the slot (the D4 deref guard).
    ecs.run_closure_once(|mut q: Query<Mut<MBody>>| {
        for mut b in &mut q {
            b.x += 1.0;
        }
    });

    // Added<dense> reads the slot's added tick; Changed<dense> reads the slot's
    // changed tick — both through `UnsafeCell::get()` at the gathered slot.
    ecs.run_closure_once(|q: Query<&MBody, Added<MBody>>| {
        for _ in &q {}
    });
    ecs.run_closure_once(|q: Query<&MBody, Changed<MBody>>| {
        for _ in &q {}
    });
}

/// Single-thread insert / remove (tombstone) / reuse tick re-stamping — the
/// insert path's `write_added_tick` / `write_changed_tick` and the reused-slot
/// re-stamp, exercised through the dense store insert routing.
#[test]
fn miri_dense_insert_remove_reuse_tick_stamp_no_ub() {
    let mut ecs = EcsMaster::new();

    // Seed + despawn so the dense free-list has reusable slots.
    let e0 = ecs.run_system(|mut cmds: Commands| cmds.spawn(BodyOnly { body: MBody { x: 0.0, y: 0.0, z: 0.0, w: 0.0 } }).id());
    let e1 = ecs.run_system(|mut cmds: Commands| cmds.spawn(BodyOnly { body: MBody { x: 1.0, y: 0.0, z: 0.0, w: 0.0 } }).id());
    ecs.delete_entity(e0);
    ecs.delete_entity(e1);

    // Fresh inserts reuse the freed slots and re-stamp their ticks (write into the
    // reused slot's tick sub-region — the write-before-read property).
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(BodyOnly { body: MBody { x: 2.0, y: 0.0, z: 0.0, w: 0.0 } });
        cmds.spawn(BodyOnly { body: MBody { x: 3.0, y: 0.0, z: 0.0, w: 0.0 } });
    });

    // Read the reused slots' ticks through Added (validates the re-stamp read path).
    ecs.run_closure_once(|q: Query<&MBody, Added<MBody>>| {
        for _ in &q {}
    });
}
