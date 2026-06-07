//! Bug #56 — minimal Miri (Tree Borrows) coverage for the deferred-spawn →
//! `Added<T>` path end-to-end through `Schedule::run`.
//!
//! Run via:
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_phase_bugfix_56
//! ```
//!
//! `-Zmiri-ignore-leaks` masks the pre-existing #53 by-design spawn-cache leak
//! (`Box<[OnceLock<ArchetypeId>; 1024]>` static, leaked on purpose); the check
//! confirms there is NO double-free / NO aliasing UB on the deferred-spawn →
//! apply-window-bump → next-frame-`Added` path.
//!
//! # Why a separate, MINIMAL file
//!
//! The broader native suite (`phase_bugfix_deferred_change_detection.rs`)
//! exercises 5 cases over 4 frames each. Under Miri (which interprets the
//! threadpool `Scope::spawn` handshake AND the demand-zero arena allocation per
//! frame) that is far too slow. This file keeps the Miri budget tractable:
//!
//!   * exactly TWO `Schedule::run` frames — spawn on frame 1, observe on frame 2,
//!   * ONE spawner + ONE reader system,
//!   * a single-worker pool (`num_threads(1)`),
//!   * a single entity.
//!
//! That is the minimum that still drives the full unsafe surface the #56 change
//! touches: the frame-start + apply-window `bump_change_tick` (both plain atomic
//! `fetch_add(Relaxed)`, no unsafe of their own), the deferred `SpawnAtCommand`
//! apply at the apply-window barrier (the `CommandQueue` raw-twin + arena write),
//! and the next-frame `Added<T>` per-row tick read. `Schedule::run` itself is
//! independently Miri-TB-verified (Phase 9.3c: `miri_schedule_parallel.rs`,
//! `miri_phase10.rs`); this pins the #56-specific deferred-then-observed lane.
//!
//! Not `#[cfg(miri)]`-gated, so it also runs as a fast native smoke test.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::ThreadPoolBuilder;

const REL: Ordering = Ordering::Relaxed;

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct MiriC56 {
    v: u32,
}

#[derive(Bundle)]
struct MiriC56Bundle {
    c: MiriC56,
}

static M56_ADDED_HITS: AtomicUsize = AtomicUsize::new(0);
static M56_DO_SPAWN: AtomicBool = AtomicBool::new(false);

/// Two frames: deferred-spawn `MiriC56` on frame 1; a `Added<MiriC56>` reader
/// counts matches each frame. Under TB the whole deferred-spawn → apply-window
/// bump → next-frame Added read must be UB-clean (the only Miri complaint is the
/// masked #53 cache leak). The native assertion also pins the #56 semantic:
/// 0 hits on frame 1 (stamp at this_run+1, above frame-1 window), 1 hit on
/// frame 2.
#[test]
fn miri_deferred_spawn_added_path_tb_clean() {
    M56_ADDED_HITS.store(0, REL);
    M56_DO_SPAWN.store(true, REL);

    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut world = EcsMaster::new();
    let _ = MiriC56::component_id();

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let spawner = builder
        .add_system(move |mut cmds: Commands| {
            if M56_DO_SPAWN.load(REL) {
                cmds.spawn(MiriC56Bundle { c: MiriC56 { v: 1 } });
            }
        })
        .key();
    builder
        .add_system(|q: Query<&MiriC56, Added<MiriC56>>| {
            for _ in &q {
                M56_ADDED_HITS.fetch_add(1, REL);
            }
        })
        .after(spawner);
    let mut schedule = builder.build(&mut world);

    // Frame 1: spawn arms; the entity is created at the apply-window (stamped at
    // this_run+1). Reader window ends at this_run ⇒ NOT seen this frame.
    schedule.run(&mut world);
    let f1 = M56_ADDED_HITS.swap(0, REL);

    // Frame 2: no further spawn; the frame-2 window (this_run_1, this_run_1+2]
    // contains the this_run_1+1 stamp ⇒ seen exactly once.
    M56_DO_SPAWN.store(false, REL);
    schedule.run(&mut world);
    let f2 = M56_ADDED_HITS.load(REL);

    assert_eq!(f1, 0, "Bug#56: deferred-spawned C not observed on the spawn frame (stamp above window)");
    assert_eq!(f2, 1, "Bug#56: deferred-spawned C observed by Added exactly once the NEXT frame");
    assert_eq!(world.entity_count(), 1, "the deferred spawn landed one entity");
}

// ════════════════════════════════════════════════════════════════════════════
// Item B case 3 (minimal) — same-frame DIRECT Mut<C> write stays visible to a
// later-ordered Changed<C> reader in the SAME frame, under TB. ONE frame.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct MiriDirect56 {
    v: u32,
}

static M56_DIRECT_SEEN: AtomicUsize = AtomicUsize::new(0);

/// A query-`Mut<C>` writer (direct through-query write, stamped at the system's
/// frame-start `this_run`) followed by a `.after`-ordered `Changed<C>` reader in
/// the SAME frame. The apply-window bump (#56) must NOT evict the intra-frame
/// direct-write stamp from the reader window — the change is seen in-frame. Run
/// under TB this also pins the `Mut` deref-guard + per-row changed-tick write
/// path as UB-clean inside `Schedule::run`. ONE frame keeps the Miri budget low.
#[test]
fn miri_same_frame_direct_change_tb_clean() {
    M56_DIRECT_SEEN.store(0, REL);

    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[MiriDirect56::component_id()]);
    world
        .spawn_one(arch, MiriDirect56 { v: 0 })
        .expect("spawn {MiriDirect56}");

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let writer = builder
        .add_system(|mut q: Query<Mut<MiriDirect56>>| {
            for mut c in &mut q {
                c.v = c.v.wrapping_add(1);
            }
        })
        .key();
    builder
        .add_system(|q: Query<&MiriDirect56, Changed<MiriDirect56>>| {
            for _ in &q {
                M56_DIRECT_SEEN.fetch_add(1, REL);
            }
        })
        .after(writer);
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);

    assert_eq!(
        M56_DIRECT_SEEN.load(REL),
        1,
        "Bug#56: a same-frame query-Mut write is observed by a later Changed reader IN-frame \
         (apply-window bump must not evict the intra-frame stamp)"
    );
}
