//! Phase X.I — integration tests for ComponentPool row-capacity growth
//! (`docs/PHASE-XI-PLAN.md` §Test matrix, I-1 … I-5 + the archetype-level
//! half of U-P8).
//!
//! Covers the END-TO-END growth surface that the in-file pool unit tests
//! (`component_pool.rs`, U-P2 … U-P8) cannot reach: the `Commands` apply
//! paths (`SpawnAtCommand` / `SpawnBatchCommand` / `InsertCommand`
//! migrations), the schedule apply-window, hook re-entrancy during a
//! growth event, `for_each_chunk` single-slice contiguity, and the
//! re-worded reserve-ceiling panic (★R1-7).
//!
//! # Component-id strategy
//!
//! Every component uses `#[derive(Component)]` (lazily-minted ids from the
//! global atomic counter) — no collisions with the explicit
//! `register_layout` slot ranges other test files claim.
//!
//! # Change-detection frames (#56)
//!
//! `Added<T>` produced via deferred `Commands` becomes visible ONE frame
//! after the apply (the apply-window stamps at `this_run + 1`); every
//! schedule-driven assertion below follows the
//! `phase_bugfix_deferred_change_detection.rs` model: arm on frame 1,
//! observe on frame 2, assert silence afterwards.
//!
//! # Miri
//!
//! Heavy tests (thousands of rows) are `#[cfg_attr(miri, ignore)]`; the
//! dedicated `miri_pool_growth.rs` (M-XI) suite covers the same growth
//! bookkeeping with small-granule-count geometry under Tree Borrows.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Added, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

const SEQ: Ordering = Ordering::SeqCst;

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

// ════════════════════════════════════════════════════════════════════════════
// I-1 — cross-ceiling spawn: ONE archetype grown to 100,000 rows via Commands
//        sub-batches (past the pre-X.I 65,536 medium-class ceiling). Handles
//        stay valid, iteration count + checksum match, Added semantics intact.
// ════════════════════════════════════════════════════════════════════════════

/// 16-byte payload carrying its own spawn index (`v`) — self-validating rows.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I1Pos {
    v: u64,
    w: u64,
}

/// 12-byte second column (D2 "~12-64 B strides" mix).
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I1Vel {
    a: u32,
    b: u32,
    c: u32,
}

/// 8-byte third column.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I1Tag {
    t: u64,
}

#[derive(Bundle)]
struct I1Bundle {
    p: I1Pos,
    v: I1Vel,
    t: I1Tag,
}

const I1_TOTAL: usize = 100_000;
/// `Commands::spawn_batch` caps at MAX_BATCH_HINT = 8192 per call — the
/// spawner loops sub-batches to push one archetype 1.5x past the old ceiling.
const I1_SUB: usize = 8192;

static I1_ENTITIES: Mutex<Vec<Entity>> = Mutex::new(Vec::new());
static I1_DO_SPAWN: AtomicBool = AtomicBool::new(false);
static I1_FRAME_ADDED: AtomicUsize = AtomicUsize::new(0);

/// I-1 (plan §Test matrix): 100 k rows through the schedule apply-window in
/// 13 sub-batches; the archetype's pools grow across MANY slab boundaries
/// mid-apply (the pre-X.I panic site `SpawnBatchCommand::apply` is now the
/// growth funnel). Pins: per-frame `Added<I1Pos>` counts `[0, 100000, 0, 0]`
/// (the #56 one-frame-later window, full count, never twice), iteration
/// count + checksum over the direct query API, spot-checked random-access
/// handles over the whole range, and the pool frontier
/// (`committed_rows >= 100_000 > 65_536`).
#[test]
#[cfg_attr(miri, ignore)]
fn cross_ceiling_spawn_100k_one_archetype() {
    I1_ENTITIES.lock().expect("probe").clear();
    I1_DO_SPAWN.store(false, SEQ);

    let pool = serial_pool();
    let mut world = EcsMaster::new();
    let _ = I1Pos::component_id();
    let _ = I1Vel::component_id();
    let _ = I1Tag::component_id();

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let spawner = builder
        .add_system(|mut cmds: Commands| {
            if !I1_DO_SPAWN.load(SEQ) {
                return;
            }
            let mut spawned = 0usize;
            while spawned < I1_TOTAL {
                let n = I1_SUB.min(I1_TOTAL - spawned);
                let base = spawned as u64;
                let ents: Vec<Entity> = cmds
                    .spawn_batch((0..n).map(move |k| {
                        let k = k as u64;
                        I1Bundle {
                            p: I1Pos { v: base + k, w: !(base + k) },
                            v: I1Vel { a: (base + k) as u32, b: 0, c: 0 },
                            t: I1Tag { t: base + k },
                        }
                    }))
                    .expect("sub-batch n <= MAX_BATCH_HINT")
                    .collect();
                I1_ENTITIES.lock().expect("probe").extend(ents);
                spawned += n;
            }
        })
        .key();
    builder
        .add_system(|q: Query<&I1Pos, Added<I1Pos>>| {
            for _ in &q {
                I1_FRAME_ADDED.fetch_add(1, SEQ);
            }
        })
        .after(spawner);
    let mut schedule = builder.build(&mut world);

    let mut per_frame = Vec::new();
    for frame in 1..=4 {
        I1_FRAME_ADDED.store(0, SEQ);
        I1_DO_SPAWN.store(frame == 1, SEQ);
        schedule.run(&mut world);
        per_frame.push(I1_FRAME_ADDED.load(SEQ));
    }

    // #56 window: stamped at frame 1's apply-window (this_run + 1) ->
    // observed exactly once, on frame 2, with the FULL count.
    assert_eq!(
        per_frame,
        vec![0, I1_TOTAL, 0, 0],
        "Added<I1Pos> must observe all 100k deferred-spawned rows exactly once, \
         one frame after the apply (#56 semantics). Got {per_frame:?}"
    );

    assert_eq!(world.entity_count(), I1_TOTAL, "all 100k entities live");

    // Iteration count + checksum over the direct query API (the typed-iter
    // read path is growth-transparent — D10).
    let view = world.query::<&I1Pos, ()>();
    let (count, sum) = view
        .iter()
        .fold((0usize, 0u64), |(c, s), p: &I1Pos| (c + 1, s + p.v));
    assert_eq!(count, I1_TOTAL, "query iteration must visit every row");
    let expected: u64 = (I1_TOTAL as u64 - 1) * (I1_TOTAL as u64) / 2;
    assert_eq!(sum, expected, "checksum over v == sum(0..100000)");

    // Spot-check random-access handles across the whole range (row pointers
    // recomputed from the write-once base — X.B identity).
    let entities = I1_ENTITIES.lock().expect("probe");
    assert_eq!(entities.len(), I1_TOTAL, "every sub-batch's handles collected");
    for idx in (0..I1_TOTAL).step_by(9_973).chain([0, I1_TOTAL - 1]) {
        let e = entities[idx];
        let p = world
            .get_component::<I1Pos>(e)
            .unwrap_or_else(|| panic!("handle {idx} must stay valid across growth"));
        assert_eq!(p.v, idx as u64, "handle {idx} reads its own spawn index");
        assert_eq!(p.w, !(idx as u64), "handle {idx} payload intact");
    }

    // The frontier actually crossed the old ceiling: committed_rows tracks
    // 100k rows in ONE archetype (pre-X.I: panic at 65,536).
    let arch_id = world
        .get_entity_archetype_id(entities[0])
        .expect("entity registered");
    drop(entities);
    let arch = world
        .archetype_master()
        .get_archetype(arch_id)
        .expect("archetype registered");
    for id in [I1Pos::component_id(), I1Vel::component_id(), I1Tag::component_id()] {
        let pool = arch.component_pools().get_pool(id).expect("pool exists");
        assert_eq!(pool.count(), I1_TOTAL, "pool {id:?} row count");
        assert!(
            pool.committed_rows() >= I1_TOTAL,
            "pool {id:?} frontier must cover 100k rows (committed = {})",
            pool.committed_rows()
        );
        assert!(
            pool.capacity() > 65_536,
            "the reserve ceiling must sit beyond the pre-X.I medium ceiling"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// I-2 — migration-into-grown-target: 2500 entities migrate {Src} -> {Src,Tag}
//        in one apply window; the target's 64-B pool grows mid-apply (3 commit
//        events); source bytes preserved; retained ticks preserved (no Added
//        re-fire), inserted component fires Added exactly once.
// ════════════════════════════════════════════════════════════════════════════

/// 64-byte source payload: one granule = 1024 rows, so 2500 migrations cross
/// TWO slab boundaries inside the target pool mid-apply.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I2Src {
    id: u64,
    pad: [u64; 7],
}

impl I2Src {
    fn new(id: u64) -> Self {
        Self { id, pad: [id.rotate_left(17) ^ 0x5151_5151_5151_5151; 7] }
    }
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I2Tag {
    m: u32,
}

#[derive(Bundle)]
struct I2SrcBundle {
    s: I2Src,
}

#[derive(Bundle)]
struct I2TagBundle {
    m: I2Tag,
}

const I2_N: usize = 2_500;

static I2_ENTITIES: Mutex<Vec<Entity>> = Mutex::new(Vec::new());
static I2_DO_SPAWN: AtomicBool = AtomicBool::new(false);
static I2_DO_INSERT: AtomicBool = AtomicBool::new(false);
static I2_FRAME_ADDED_SRC: AtomicUsize = AtomicUsize::new(0);
static I2_FRAME_ADDED_TAG: AtomicUsize = AtomicUsize::new(0);

/// I-2 (plan §Test matrix): frame 1 spawns 2500 `{I2Src}` rows; frame 3
/// deferred-inserts `I2Tag` onto every entity — 2500 `migrate_entity_insert`
/// calls into the (initially empty) target archetype, whose 64-B pool grows
/// across >= 2 slab boundaries MID-apply-window. Pins:
/// * source bytes preserved post-migration (self-validating `pad == f(id)`),
/// * the migration tick contract — retained `I2Src` keeps its ORIGINAL
///   added tick (per-frame `Added<I2Src>` = `[0, 2500, 0, 0, 0]`: NO re-fire
///   on the migration frame 4), while the freshly-inserted `I2Tag` fires
///   `Added` exactly once, one frame after the apply (#56:
///   `[0, 0, 0, 2500, 0]`),
/// * the target pool's frontier covers all 2500 rows.
#[test]
#[cfg_attr(miri, ignore)]
fn migration_into_grown_target_preserves_bytes_and_ticks() {
    I2_ENTITIES.lock().expect("probe").clear();
    I2_DO_SPAWN.store(false, SEQ);
    I2_DO_INSERT.store(false, SEQ);

    let pool = serial_pool();
    let mut world = EcsMaster::new();
    let _ = I2Src::component_id();
    let _ = I2Tag::component_id();

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let spawner = builder
        .add_system(|mut cmds: Commands| {
            if !I2_DO_SPAWN.load(SEQ) {
                return;
            }
            let ents: Vec<Entity> = cmds
                .spawn_batch((0..I2_N).map(|i| I2SrcBundle { s: I2Src::new(i as u64) }))
                .expect("2500 <= MAX_BATCH_HINT")
                .collect();
            I2_ENTITIES.lock().expect("probe").extend(ents);
        })
        .key();
    let mutator = builder
        .add_system(|mut cmds: Commands| {
            if !I2_DO_INSERT.load(SEQ) {
                return;
            }
            for (i, &e) in I2_ENTITIES.lock().expect("probe").iter().enumerate() {
                cmds.entity(e).insert(I2TagBundle { m: I2Tag { m: i as u32 ^ 0x55 } });
            }
        })
        .after(spawner)
        .key();
    builder
        .add_system(|q: Query<&I2Src, Added<I2Src>>| {
            for _ in &q {
                I2_FRAME_ADDED_SRC.fetch_add(1, SEQ);
            }
        })
        .after(mutator);
    builder
        .add_system(|q: Query<&I2Tag, Added<I2Tag>>| {
            for _ in &q {
                I2_FRAME_ADDED_TAG.fetch_add(1, SEQ);
            }
        })
        .after(mutator);
    let mut schedule = builder.build(&mut world);

    let mut added_src = Vec::new();
    let mut added_tag = Vec::new();
    for frame in 1..=5 {
        I2_FRAME_ADDED_SRC.store(0, SEQ);
        I2_FRAME_ADDED_TAG.store(0, SEQ);
        I2_DO_SPAWN.store(frame == 1, SEQ);
        I2_DO_INSERT.store(frame == 3, SEQ);
        schedule.run(&mut world);
        added_src.push(I2_FRAME_ADDED_SRC.load(SEQ));
        added_tag.push(I2_FRAME_ADDED_TAG.load(SEQ));
    }

    // Tick contract, retained side: the migration (frame 3 apply) must NOT
    // re-stamp the retained component — Added<I2Src> fires only for the
    // original spawn (frame 2 per #56), NEVER on frame 4.
    assert_eq!(
        added_src,
        vec![0, I2_N, 0, 0, 0],
        "retained I2Src keeps its ORIGINAL added tick across the migration \
         (no Added re-fire on frame 4). Got {added_src:?}"
    );
    // Tick contract, inserted side: the fresh bundle component is stamped at
    // the migration apply-window -> seen exactly once, frame 4.
    assert_eq!(
        added_tag,
        vec![0, 0, 0, I2_N, 0],
        "inserted I2Tag fires Added exactly once, one frame after the \
         migration apply (#56). Got {added_tag:?}"
    );

    // Byte preservation: every migrated row's source bytes are intact
    // (self-validating pad), and the inserted component carries its value.
    let view = world.query::<(&I2Src, &I2Tag), ()>();
    let mut count = 0usize;
    let mut id_sum = 0u64;
    for (s, t) in view.iter() {
        assert_eq!(
            s.pad,
            I2Src::new(s.id).pad,
            "source bytes (row id {}) must survive the migration memcpy intact",
            s.id
        );
        assert_eq!(
            t.m,
            (s.id as u32) ^ 0x55,
            "inserted I2Tag value must match its enqueue payload"
        );
        count += 1;
        id_sum += s.id;
    }
    assert_eq!(count, I2_N, "every entity migrated into {{Src, Tag}}");
    assert_eq!(
        id_sum,
        (I2_N as u64 - 1) * (I2_N as u64) / 2,
        "every distinct source id present exactly once"
    );

    // The target archetype's 64-B pool grew mid-apply to cover 2500 rows
    // (>= 2 slab boundaries crossed inside one apply window).
    let entities = I2_ENTITIES.lock().expect("probe");
    let target_arch_id = world
        .get_entity_archetype_id(entities[0])
        .expect("entity registered");
    let target = world
        .archetype_master()
        .get_archetype(target_arch_id)
        .expect("target archetype registered");
    assert!(
        target.has_component_id(I2Tag::component_id()),
        "entities live in the migrated {{Src, Tag}} archetype"
    );
    let src_pool = target
        .component_pools()
        .get_pool(I2Src::component_id())
        .expect("target Src pool");
    assert_eq!(src_pool.count(), I2_N);
    assert!(
        src_pool.committed_rows() >= I2_N && src_pool.committed_rows() >= 2048,
        "target pool grew across >= 2 slab boundaries mid-apply (committed = {})",
        src_pool.committed_rows()
    );
}

// ════════════════════════════════════════════════════════════════════════════
// I-3 — hooks-during-growth re-entrancy: an on_add hook defers spawns into the
//        SAME archetype exactly when the outer apply reaches the slab boundary;
//        the nested drain grows the pool; no double-apply; counts exact.
// ════════════════════════════════════════════════════════════════════════════

/// 64-byte hooked payload: the first slab covers rows [0, 1024), so the
/// 1024th fire (row 1023 — the LAST row of slab 1) arms the nested spawns,
/// which land at rows 1024-1026 and drive `grow_rows` from INSIDE the
/// re-entrant deferred-hook drain while the outer apply window is live
/// (plan D4 "Re-entrancy" — pinned by I-3).
#[derive(Component)]
#[component(on_add = i3_on_add)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I3Pay {
    id: u64,
    pad: [u64; 7],
}

impl I3Pay {
    fn new(id: u64) -> Self {
        Self { id, pad: [id ^ 0x0F0F_0F0F_0F0F_0F0F; 7] }
    }
}

#[derive(Bundle)]
struct I3PayBundle {
    c: I3Pay,
}

const I3_OUTER: usize = 1024;
const I3_NESTED: usize = 3;

static I3_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn i3_on_add(mut w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    let n = I3_FIRES.fetch_add(1, SEQ) + 1;
    // Fire #1024 == the spawn that fills the first slab's last row. Enqueue
    // nested spawns INTO THE SAME ARCHETYPE; they apply during the
    // re-entrant drain and cross the slab boundary. The nested entities'
    // own on_add fires re-enter this hook with n > 1024 -> no re-arm ->
    // the chain terminates (the 14a re-entrancy contract).
    if n == I3_OUTER {
        for k in 0..I3_NESTED as u64 {
            w.commands().spawn(I3PayBundle { c: I3Pay::new(1_000_000 + k) });
        }
    }
}

/// I-3 (plan §Test matrix): nested growth works, no double-apply, final
/// counts exact. 1024 outer deferred spawns + 3 hook-deferred nested spawns
/// = 1027 entities, 1027 hook fires (each spawn fires on_add EXACTLY once —
/// a double-apply would inflate both), pool frontier grown past the
/// boundary, and the value checksum proves every row landed exactly once.
#[test]
#[cfg_attr(miri, ignore)]
fn hook_deferred_spawns_grow_same_archetype_at_slab_boundary() {
    I3_FIRES.store(0, SEQ);

    let mut world = EcsMaster::new();
    let _ = I3Pay::component_id();

    world.run_system(|mut cmds: Commands| {
        for i in 0..I3_OUTER as u64 {
            cmds.spawn(I3PayBundle { c: I3Pay::new(i) });
        }
    });

    let total = I3_OUTER + I3_NESTED;
    assert_eq!(
        world.entity_count(),
        total,
        "1024 outer + 3 nested entities — nested spawns applied exactly once"
    );
    assert_eq!(
        I3_FIRES.load(SEQ),
        total,
        "on_add fired exactly once per spawned entity (no double-apply)"
    );

    // Checksum: every outer id 0..1024 and every nested id 1e6..1e6+3
    // present exactly once; pads intact (the boundary-crossing rows were
    // written into freshly committed pages).
    let view = world.query::<&I3Pay, ()>();
    let (count, sum) = view
        .iter()
        .inspect(|p: &&I3Pay| {
            assert_eq!(p.pad, I3Pay::new(p.id).pad, "row {} payload intact", p.id);
        })
        .fold((0usize, 0u64), |(c, s), p| (c + 1, s + p.id));
    assert_eq!(count, total);
    let expected_outer: u64 = (I3_OUTER as u64 - 1) * (I3_OUTER as u64) / 2;
    let expected_nested: u64 = (0..I3_NESTED as u64).map(|k| 1_000_000 + k).sum();
    assert_eq!(sum, expected_outer + expected_nested, "id checksum exact");

    // The nested drain grew the pool across the slab boundary.
    let arch_id = world.get_or_create_archetype(&[I3Pay::component_id()]);
    let arch = world
        .archetype_master()
        .get_archetype(arch_id)
        .expect("archetype registered");
    let pool = arch
        .component_pools()
        .get_pool(I3Pay::component_id())
        .expect("pool exists");
    assert_eq!(pool.count(), total);
    assert!(
        pool.committed_rows() >= total && pool.committed_rows() > I3_OUTER,
        "nested growth committed past the 1024-row boundary (committed = {})",
        pool.committed_rows()
    );
}

// ════════════════════════════════════════════════════════════════════════════
// I-4 — for_each_chunk single-slice witness after crossing a slab boundary.
// ════════════════════════════════════════════════════════════════════════════

/// 64-byte payload (boundary at 1024 rows): 3000 rows span 3 commit events.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I4Pay {
    id: u64,
    pad: [u64; 7],
}

#[derive(Bundle)]
struct I4Bundle {
    c: I4Pay,
}

const I4_N: usize = 3_000;

/// I-4 (plan §Test matrix / D10): after the pool crosses >= 2 slab
/// boundaries, `for_each_chunk` over the archetype must still yield exactly
/// ONE contiguous slice (`len == entity_count`) whose contents match
/// per-entity random-access reads — in-place extension preserves the
/// whole-archetype single-slice contract (the demo's zero-copy GPU upload
/// depends on it).
#[test]
#[cfg_attr(miri, ignore)]
fn for_each_chunk_single_slice_after_growth() {
    let mut world = EcsMaster::new();
    let _ = I4Pay::component_id();

    world.run_system(|mut cmds: Commands| {
        let _ = cmds
            .spawn_batch((0..I4_N).map(|i| {
                let i = i as u64;
                I4Bundle {
                    c: I4Pay { id: i, pad: [i.wrapping_mul(0x9E37_79B9_7F4A_7C15); 7] },
                }
            }))
            .expect("3000 <= MAX_BATCH_HINT");
    });
    assert_eq!(world.entity_count(), I4_N);

    let mut calls = 0usize;
    let mut seen_len = 0usize;
    let mut mismatches = 0usize;
    {
        let mut view = world.query::<&I4Pay, ()>();
        view.for_each_chunk(|slice: &[I4Pay]| {
            calls += 1;
            seen_len = slice.len();
            for (i, c) in slice.iter().enumerate() {
                // spawn_batch appends rows in iterator order -> row i holds id i.
                if c.id != i as u64
                    || c.pad != [(i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15); 7]
                {
                    mismatches += 1;
                }
            }
        });
    }
    assert_eq!(
        calls, 1,
        "for_each_chunk must yield exactly ONE slice per archetype — \
         in-place growth must not fragment the column"
    );
    assert_eq!(seen_len, I4_N, "the single slice covers every row");
    assert_eq!(mismatches, 0, "slice contents match the spawn payloads row-for-row");

    // Cross-validate the slice against per-entity random-access reads.
    let entities = world.query_entities(&[I4Pay::component_id()]);
    assert_eq!(entities.len(), I4_N);
    for idx in (0..I4_N).step_by(499).chain([I4_N - 1]) {
        let p = world
            .get_component::<I4Pay>(entities[idx])
            .expect("handle valid");
        assert_eq!(
            p.id, idx as u64,
            "random-access read at row {idx} matches the slice contents"
        );
    }

    // The witness is only meaningful if growth actually crossed boundaries.
    let arch_id = world.get_or_create_archetype(&[I4Pay::component_id()]);
    let pool = world
        .archetype_master()
        .get_archetype(arch_id)
        .expect("archetype")
        .component_pools()
        .get_pool(I4Pay::component_id())
        .expect("pool");
    assert!(
        pool.committed_rows() >= I4_N && pool.committed_rows() > 2048,
        "the pool crossed >= 2 slab boundaries (committed = {})",
        pool.committed_rows()
    );
}

// ════════════════════════════════════════════════════════════════════════════
// I-5 (★R1-7) — the re-worded reserve-ceiling panic on the Commands path.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I5Comp(u32);

#[derive(Bundle)]
struct I5Bundle {
    c: I5Comp,
}

/// I-5 (★R1-7, plan §Test matrix): drives `Commands::spawn` past a pool's
/// `reserve_rows` and pins the re-worded `SpawnAtCommand::apply` `.expect`
/// MESSAGE — the previously-untested panic surface (R-§6).
///
/// # Layer pinned: the REAL `SpawnAtCommand::apply` expect
///
/// `Commands` always routes into archetypes whose pools were built by
/// `with_default_sizes` (65,536+ row floors on the syscall arms — natively
/// unreachable in a test). Instead of a new test hook, the test swaps the
/// archetype's pool for an explicit small-ceiling `ComponentPool::new(_, _,
/// 1, 4)` (the D2 mapping) THROUGH THE EXISTING PUB SURFACE
/// (`archetype_master_mut() -> get_archetype_mut -> component_pools_mut ->
/// get_pool_mut`), then spawns 5 entities via `Commands` — the 5th apply
/// hits `reserve_capacity(1) -> Err -> .expect` with the ceiling wording.
///
/// # Why the pool swap is sound here
///
/// The swap drops the old (empty, zero-committed) pool and leaves the
/// archetype's inline `columns` entry pointing at the RELEASED reservation
/// — a dangling pointer VALUE that this test never dereferences: the
/// spawn-apply path writes through the bundle's pool vector
/// (`pool_at_unchecked_mut`), the `BundleColumnCache` stores pool INDICES,
/// `I5Comp` has no hooks, and no read path (random access / query) runs
/// before the expected panic unwinds the test. Do NOT extend this test
/// with reads after the swap.
#[test]
#[should_panic(expected = "SpawnAtCommand: pool reserve ceiling (rows) exhausted")]
fn commands_spawn_past_reserve_ceiling_panics_with_ceiling_wording() {
    let mut world = EcsMaster::new();
    let arch_id = world.create_archetype(&[I5Comp::component_id()]);

    // Swap in the tiny D2-mapped pool (ceiling = 1 * 4 rows) BEFORE any row
    // exists, so the old pool drops empty.
    {
        let small = ComponentPool::new(I5Comp::component_id().0, 4);
        let arch = world
            .archetype_master_mut()
            .get_archetype_mut(arch_id)
            .expect("archetype registered");
        let pool = arch
            .component_pools_mut()
            .get_pool_mut(I5Comp::component_id())
            .expect("pool exists for I5Comp");
        *pool = small;
        assert_eq!(pool.capacity(), 4, "explicit-ceiling constructor mapping (★R1-9)");
    }

    // Spawns 1-4 fill the ceiling; the 5th SpawnAtCommand::apply hits
    // `reserve_capacity(1)` Phase A -> Err -> the re-worded expect.
    world.run_system(|mut cmds: Commands| {
        for i in 0..5u32 {
            cmds.spawn(I5Bundle { c: I5Comp(i) });
        }
    });
}

// ════════════════════════════════════════════════════════════════════════════
// U-P8 (archetype half) — reserve_capacity idempotence through the public
// surface: a second spawn into an already-committed archetype must leave
// every pool's frontier EXACTLY unchanged (★R1-1).
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct UP8Small {
    a: u64,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct UP8Big {
    x: u64,
    pad: [u64; 7],
}

#[derive(Bundle)]
struct UP8Bundle {
    s: UP8Small,
    b: UP8Big,
}

/// U-P8, archetype level (★R1-1, plan §Test matrix): every
/// `SpawnAtCommand::apply` calls `Archetype::reserve_capacity(1)`
/// UNCONDITIONALLY (Phase B `grow_rows` on every pool) — legal only because
/// of the idempotent no-op arm. Witness through the public surface: the
/// first spawn commits each pool's first slab; subsequent spawns (capacity
/// already committed) must leave `committed_rows` of EVERY pool exactly
/// unchanged. Without the ★R1-1 early-out each satisfied
/// `reserve_capacity(1)` would commit a fresh doubling slab per spawn — the
/// critic-Round-1 CRITICAL memory explosion.
#[test]
fn reserve_capacity_idempotent_across_repeat_spawns() {
    let mut world = EcsMaster::new();
    let _ = UP8Small::component_id();
    let _ = UP8Big::component_id();

    // First spawn: grows every pool once (first slab).
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(UP8Bundle { s: UP8Small { a: 0 }, b: UP8Big { x: 0, pad: [0; 7] } });
    });
    assert_eq!(world.entity_count(), 1);

    let arch_id = world.get_or_create_archetype(&[
        UP8Small::component_id(),
        UP8Big::component_id(),
    ]);
    let snapshot = |world: &EcsMaster| -> Vec<usize> {
        let arch = world
            .archetype_master()
            .get_archetype(arch_id)
            .expect("archetype registered");
        [UP8Small::component_id(), UP8Big::component_id()]
            .iter()
            .map(|id| {
                arch.component_pools()
                    .get_pool(*id)
                    .expect("pool exists")
                    .committed_rows()
            })
            .collect()
    };

    let after_first = snapshot(&world);
    assert!(
        after_first.iter().all(|&c| c > 0),
        "the first spawn committed each pool's first slab: {after_first:?}"
    );

    // Ten more spawns: each apply's reserve_capacity(1) is satisfied -> the
    // grow_rows no-op arm -> frontiers must not move by a single row.
    world.run_system(|mut cmds: Commands| {
        for i in 1..=10u64 {
            cmds.spawn(UP8Bundle {
                s: UP8Small { a: i },
                b: UP8Big { x: i, pad: [i; 7] },
            });
        }
    });
    assert_eq!(world.entity_count(), 11);

    let after_more = snapshot(&world);
    assert_eq!(
        after_first, after_more,
        "reserve_capacity(1) with capacity already committed must be a ZERO-state-change \
         no-op on every pool (★R1-1); a moving frontier here means the idempotence arm broke"
    );
}
