//! UG-04 — the committed-bytes census on a kernel scene (unified plan 03 UG-04; KC-01; allocator
//! G2b).
//!
//! Every kernel commit goes through `boyko_memory`'s one choke point and is counted there under
//! its owner (`boyko_memory::committed_bytes::<O>()`). This scene reads the counter over three
//! windows of one world and pins what each must be:
//!
//! | window | what runs | `Column` delta |
//! |---|---|---|
//! | setup | `EcsMaster::new`, a spawn of `SETUP_ENTITIES` tracked 16 B rows, one frame | `> 0` |
//! | steady | `STEADY_FRAMES` frames of a read/write system, no structural change | `== 0` |
//! | canary | a system spawns ONE entity of a type no archetype holds yet, then `STEADY_FRAMES` frames | `== CANARY_BYTES` |
//!
//! `Chunk` and `Table` have no producer until rungs D-M2 and D-M1, so they are reported, and read
//! 0 here by construction.
//!
//! # Why each pin is what it is
//!
//! - **setup `> 0`** is the anti-vacuity half: a counter that is not wired reads 0 in every window,
//!   which would pass the steady pin for the wrong reason. The exact value depends on the stagger
//!   the minted component id selects (D-M0), so it is printed, not pinned.
//! - **steady `== 0`** is the rung's red: a commit from a system body in a frame with no structural
//!   change is a steady-state allocation the kernel must not make.
//! - **canary `== CANARY_BYTES`** proves the steady pin can fail. A structural change inside a
//!   system body commits exactly: one page per sub-region of the new archetype's tracked pool
//!   (data, added ticks, changed ticks; a one-row pool commits one page each, because the stagger
//!   is below one page), plus one `POOL_MIN_SLAB` for the new archetype's lazy `entity_ids`. The
//!   inland store does not grow: entity `SETUP_ENTITIES + 1` is far inside its first slab.
//!
//! One `#[test]` in this binary, on purpose: the counter is process-global, and a second test in
//! the same binary would commit concurrently into every window.
//!
//! # Two workers, and Miri
//!
//! The schedule runs on a two-thread pool, so the `Query<&mut Body>` pass and the canary's
//! `Commands` spawn can run in one round, as they would in a game frame. The counter itself does
//! not depend on parallelism. Under Miri the scene reads the native numbers (the fallback arm
//! commits nothing but counts the same ranges):
//!
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" \
//!   cargo +nightly miri test -p boyko-ecs --test ug04_committed_bytes_census
//! ```
//!
//! That recipe is Tree Borrows, the one model this scene runs under. Until PC-24 closed, two
//! workers raced here under Tree Borrows (measured twice; Stacked Borrows was never run on two
//! workers), so the scene ran on one. A pool-backed Stacked Borrows run stops inside
//! crossbeam-epoch before any kernel system executes (measured on one worker); the kernel's
//! Stacked Borrows leg for this pair of systems is the pool-free `pc24_` matrix in the
//! `UnsafeEcsCell` module.
//!
//! `-Zmiri-ignore-leaks` for the reason every pool-backed test here gives: the pool's
//! crossbeam-epoch handles and the per-bundle column cache are still allocated at process exit.
//! `-Zmiri-strict-provenance` does not apply: crossbeam-epoch casts integers to pointers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use boyko_ecs::ecs::constants::{COMMIT_PAGE, POOL_MIN_SLAB};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};
use boyko_memory::{ChunkOwner, ColumnOwner, TableOwner, committed_bytes};
use boyko_threadpool::ThreadPoolBuilder;

/// Rows spawned in the setup window.
const SETUP_ENTITIES: usize = 1000;

/// Frames in the steady window, and after the canary spawn.
const STEADY_FRAMES: usize = 10;

/// What the canary spawn commits: three pool pages and one `entity_ids` slab.
const CANARY_BYTES: usize = 3 * COMMIT_PAGE + POOL_MIN_SLAB;

#[cfg(target_arch = "x86_64")]
const _: () = assert!(CANARY_BYTES == 16384);

/// The scene's row payload: a tracked (change-ticked) 16 B component.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Body {
    p: [f32; 4],
}

#[derive(Bundle)]
struct BodyBundle {
    body: Body,
}

/// A tracked 16 B component that no archetype holds until the canary window.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Canary {
    a: u64,
    b: u64,
}

#[derive(Bundle)]
struct CanaryBundle {
    canary: Canary,
}

const _: () = assert!(size_of::<Body>() == 16 && size_of::<Canary>() == 16);

/// Arms the canary system for exactly one frame.
static SPAWN_CANARY: AtomicBool = AtomicBool::new(false);

/// `[Column, Chunk, Table]` committed bytes, now.
fn snapshot() -> [usize; 3] {
    [
        committed_bytes::<ColumnOwner>(),
        committed_bytes::<ChunkOwner>(),
        committed_bytes::<TableOwner>(),
    ]
}

/// Per-owner bytes committed between two snapshots (the counters never decrease).
fn delta(before: [usize; 3], after: [usize; 3]) -> [usize; 3] {
    [after[0] - before[0], after[1] - before[1], after[2] - before[2]]
}

/// The frame: a read/write pass over every `Body` row, and the canary system, which spawns one
/// `Canary` entity through `Commands` on the one frame it is armed.
fn build_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut q: Query<&mut Body>| {
        for body in q.iter_mut() {
            body.p[0] += 1.0;
        }
    });
    builder.add_system(|mut cmds: Commands| {
        if SPAWN_CANARY.swap(false, Ordering::Relaxed) {
            cmds.spawn(CanaryBundle { canary: Canary { a: 1, b: 2 } });
        }
    });
    builder.build(world)
}

#[test]
fn steady_frames_commit_nothing_and_a_structural_change_commits_its_pages() {
    let s0 = snapshot();
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        for i in 0..SETUP_ENTITIES {
            cmds.spawn(BodyBundle { body: Body { p: [i as f32, 0.0, 0.0, 0.0] } });
        }
    });
    let mut schedule = build_schedule(&mut world);
    schedule.run(&mut world);
    let s1 = snapshot();

    for _ in 0..STEADY_FRAMES {
        schedule.run(&mut world);
    }
    let s2 = snapshot();

    SPAWN_CANARY.store(true, Ordering::Relaxed);
    for _ in 0..=STEADY_FRAMES {
        schedule.run(&mut world);
    }
    let s3 = snapshot();

    let (setup, steady, canary) = (delta(s0, s1), delta(s1, s2), delta(s2, s3));
    println!("UG-04 committed bytes [Column, Chunk, Table]");
    println!("  setup  (EcsMaster::new + {SETUP_ENTITIES} rows + 1 frame): {setup:?}");
    println!("  steady ({STEADY_FRAMES} frames):                          {steady:?}");
    println!("  canary (1 spawn frame + {STEADY_FRAMES} frames):             {canary:?}");

    let canaries = world.run_system(|q: Query<&Canary>| q.iter().count());
    assert_eq!(canaries, 1, "the canary frame must spawn exactly one Canary entity");
    assert!(
        setup[0] > 0,
        "setup Column commit_delta is 0: the counter does not see the kernel's commits, so the \
         steady pin below would pass for the wrong reason"
    );
    assert_eq!(
        steady[0], 0,
        "steady Column commit_delta {} != 0: a system body committed memory in a frame with no \
         structural change",
        steady[0]
    );
    assert_eq!(
        canary[0], CANARY_BYTES,
        "canary Column commit_delta: one spawn into a new archetype commits one page per \
         sub-region of its tracked pool (3 x COMMIT_PAGE) plus one POOL_MIN_SLAB of entity_ids"
    );
}
