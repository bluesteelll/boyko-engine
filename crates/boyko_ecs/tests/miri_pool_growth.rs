//! Phase X.I — M-XI: Miri (Tree Borrows) coverage for ComponentPool
//! row-capacity growth (`docs/PHASE-XI-PLAN.md` §Test matrix, M-XI).
//!
//! Run via:
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_pool_growth
//! ```
//!
//! `-Zmiri-ignore-leaks` is needed ONLY for the EcsMaster churn test, which
//! spawns via `Commands` and reaches the by-design bounded
//! `BundleColumnCache` `Box::leak` (#53, NOT-A-BUG) — matching the sibling
//! suites (`miri_phase14b` / `miri_phase19` / …). The raw-pool tests leak
//! nothing.
//!
//! # What Miri proves here (D9)
//!
//! Under Miri the `VmReservation` compiles its FALLBACK arm: `reserve` is an
//! eager `alloc_zeroed` of the full `os_len` and `commit` is a no-op — but
//! the growth BOOKKEEPING (frontier math, doubling step, row/tick lockstep,
//! ceiling arm, `committed_rows` oracle) runs IDENTICALLY to the syscall
//! arms, and every row/tick access goes through the same `row_ptr` /
//! tick-base derivations. Tree Borrows therefore audits the real provenance
//! story: three sub-region base pointers derived once from `vm.base()`
//! (ONE allocated object — ★R1-8), rows written/read across slab-boundary
//! growth, swap_remove across a boundary, and the Drop loop.
//!
//! # Geometry
//!
//! All raw-pool tests use a 1024-byte stride so ONE commit granule
//! (64 KiB) covers exactly 64 rows — slab boundaries at rows 64 / 128 / 256
//! keep iteration counts Miri-cheap while still crossing >= 2 growth events
//! (the small-ceiling D2 constructor mapping is the test knob — ★R1-9).

#![cfg(miri)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use boyko_macros::{Bundle, Component};

/// 1024-byte POD payload: one granule = 64 rows (see file header geometry).
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Pad1K {
    id: u64,
    pad: [u64; 127],
}

impl Pad1K {
    fn new(id: u64) -> Self {
        Self { id, pad: [id ^ 0xABCD_EF01_2345_6789; 127] }
    }
}

fn make_pad1k_pool(cap: usize) -> ComponentPool {
    let id = Pad1K::component_id(); // derive registers the layout on first call
    ComponentPool::new(id.0, cap)
}

/// M-XI (1) — multi-slab growth bookkeeping + address stability under TB.
///
/// A 1 x 256 pool spans 4 granules; 200 adds drive 3 growth events
/// (committed_rows 64 -> 128 -> 256). Pins the frontier transitions at the
/// exact boundary adds, the write-once base pointers across all events,
/// and full value round-trips through rows on every slab.
#[test]
fn miri_multi_slab_growth_bookkeeping_and_stability() {
    let mut pool = make_pad1k_pool(256);
    assert_eq!(pool.component_layout().size(), 1024, "fixture stride pin");
    assert_eq!(pool.capacity(), 256, "D2 mapping: reserve_rows = 1 * 256");
    assert_eq!(pool.committed_rows(), 0, "D3: zero initial commit");

    pool.add_typed(Pad1K::new(0)).expect("row 0");
    assert_eq!(pool.committed_rows(), 64, "first commit = one granule = 64 rows");
    let base = pool.buffer_ptr();
    let row0 = pool.get_raw(0).expect("row 0 live");
    // (Tick-base stability is pinned by the in-file U-P2 unit test — the
    // tick accessors are pub(crate) and unreachable from tests/.)

    for i in 1..200u64 {
        pool.add_typed(Pad1K::new(i)).expect("under the 256-row ceiling");
        // Frontier transitions exactly at the slab boundaries (D4 doubling:
        // 64 KiB -> 128 KiB -> 256 KiB).
        let expected = match pool.count() {
            n if n <= 64 => 64,
            n if n <= 128 => 128,
            _ => 256,
        };
        assert_eq!(
            pool.committed_rows(),
            expected,
            "frontier after {} adds",
            pool.count()
        );
    }
    assert_eq!(pool.count(), 200);
    assert_eq!(pool.committed_rows(), 256);

    // Write-once base pointers never moved (Soundness item 1 under TB).
    assert_eq!(pool.buffer_ptr(), base, "data base stable across 3 growths");
    assert_eq!(pool.get_raw(0).expect("row 0"), row0, "row-0 ptr stable");

    // Every row on every slab round-trips through the recomputed row_ptr.
    for i in 0..200u64 {
        let got = pool.get_typed::<Pad1K>(i as usize).expect("live row");
        assert_eq!(got.id, i, "row {i} id");
        assert_eq!(got.pad, Pad1K::new(i).pad, "row {i} payload");
    }
}

/// M-XI (2) — swap_remove ACROSS a slab boundary under TB: the last row
/// (living on the second/third slab) is memcpy'd into a hole on the first
/// slab — the cross-boundary row_ptr pair the in-place design must keep
/// inside one allocated object (★R1-8).
#[test]
fn miri_swap_remove_across_slab_boundary() {
    let mut pool = make_pad1k_pool(256);

    for i in 0..130u64 {
        pool.add_typed(Pad1K::new(i)).expect("130 rows fit");
    }
    assert_eq!(pool.committed_rows(), 256, "130 rows crossed two boundaries");

    // Hole on slab 1 (row 10), donor on slab 3 (row 129).
    assert!(pool.swap_remove(10), "swap_remove(10) in bounds");
    assert_eq!(pool.count(), 129);
    let moved = pool.get_typed::<Pad1K>(10).expect("hole refilled");
    assert_eq!(moved.id, 129, "last row's value moved into the hole");
    assert_eq!(moved.pad, Pad1K::new(129).pad, "moved payload intact");

    // Neighbours untouched.
    assert_eq!(pool.get_typed::<Pad1K>(9).expect("row 9").id, 9);
    assert_eq!(pool.get_typed::<Pad1K>(11).expect("row 11").id, 11);

    // Pop back below the boundary; values stay coherent.
    for _ in 0..70 {
        assert!(pool.pop(), "pop while non-empty");
    }
    assert_eq!(pool.count(), 59);
    assert_eq!(pool.get_typed::<Pad1K>(58).expect("tail row").id, 58);
}

/// 1024-byte drop-counting payload (8-byte Arc + 1016 pad).
#[repr(C)]
struct DropPad1K {
    counter: Arc<AtomicUsize>,
    pad: [u8; 1016],
}

impl Drop for DropPad1K {
    fn drop(&mut self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Component)]
#[repr(C)]
struct DropPad1KComp {
    inner: DropPad1K,
}

/// M-XI (3) — drop-count-exact across a growth boundary under TB: 70 rows
/// cross the 64-row boundary; one swap_remove drops exactly one; pool Drop
/// drops exactly the 69 survivors (and never touches the uninit
/// `[len, committed_rows)` tail — a drop there would be TB-UB on top of
/// the count mismatch).
#[test]
fn miri_drop_count_exact_across_boundary() {
    let id = DropPad1KComp::component_id();
    let mut pool = ComponentPool::new(id.0, 128);

    let counter = Arc::new(AtomicUsize::new(0));
    const M: usize = 70;
    for _ in 0..M {
        pool.add_typed(DropPad1KComp {
            inner: DropPad1K { counter: Arc::clone(&counter), pad: [0; 1016] },
        })
        .expect("70 rows fit under the 128 ceiling");
    }
    assert_eq!(counter.load(Ordering::Relaxed), 0, "growth dropped nothing");
    assert_eq!(pool.committed_rows(), 128, "the boundary crossing grew the frontier");

    assert!(pool.swap_remove(3), "swap_remove(3) in bounds");
    assert_eq!(counter.load(Ordering::Relaxed), 1, "swap_remove drops exactly one");

    drop(pool);
    assert_eq!(
        counter.load(Ordering::Relaxed),
        M,
        "pool Drop drops each survivor exactly once (total {M})"
    );
}

/// M-XI (4) — ceiling exhaustion under TB: a tiny D2-mapped 1 x 4 pool
/// rejects the 5th add with `None` and ZERO observable state change
/// (U-P3's witness re-run under the fallback arm, where the ceiling check
/// and the no-op bookkeeping are the SAME code as native — D9 unified path).
#[test]
fn miri_ceiling_exhaustion_zero_state_change() {
    let mut pool = make_pad1k_pool(4);

    for i in 0..4u64 {
        pool.add_typed(Pad1K::new(i)).expect("rows 0..4 fit");
    }
    assert!(pool.is_full(), "len == reserve_rows");

    let before = (
        pool.count(),
        pool.committed_rows(),
        pool.capacity(),
        pool.buffer_ptr() as usize,
        pool.remaining_capacity(),
    );
    assert_eq!(pool.add_typed(Pad1K::new(99)), None, "ceiling -> None");
    let after = (
        pool.count(),
        pool.committed_rows(),
        pool.capacity(),
        pool.buffer_ptr() as usize,
        pool.remaining_capacity(),
    );
    assert_eq!(before, after, "rejected add: state EXACTLY unchanged");
    for i in 0..4u64 {
        assert_eq!(pool.get_typed::<Pad1K>(i as usize).expect("live").id, i);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// M-XI (5) — EcsMaster-level churn traversing grow_rows under TB.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct MChurn {
    v: u32,
}

#[derive(Bundle)]
struct MChurnBundle {
    c: MChurn,
}

/// M-XI (5) — spawn/despawn/respawn churn through the full `Commands` apply
/// path: every world's first spawn traverses the REAL `grow_rows` (D9
/// unified path — the fallback arm runs identical bookkeeping), despawns
/// vacate rows whose tick slots go stale (the J-XI ★R1-4 scenario), and
/// respawns re-stamp them write-before-read. Re-uses the SAME bundle type
/// across cycles deliberately: no `clear()` is involved, so the Phase-8.5
/// stale-bundle-cache footgun does not apply, and re-adding over vacated
/// rows is exactly the churn J-XI must survive under TB.
#[test]
fn miri_ecs_churn_spawn_despawn_respawn() {
    let mut world = EcsMaster::new();
    let _ = MChurn::component_id();

    let mut expected = 0usize;
    for cycle in 0..3u32 {
        // Spawn 8 via deferred Commands (SpawnAtCommand::apply -> the
        // reserve_capacity(1) funnel; cycle 0's first apply grows the pool).
        world.run_system(move |mut cmds: Commands| {
            for k in 0..8u32 {
                cmds.spawn(MChurnBundle { c: MChurn { v: cycle * 100 + k } });
            }
        });
        expected += 8;
        assert_eq!(world.entity_count(), expected, "cycle {cycle}: spawns landed");

        // Despawn the first 4 of the CURRENT population (swap_remove churn:
        // tail rows move into vacated slots; their stale tick slots above
        // `len` are never read — J-XI never-written/never-read-above-len).
        let victims: Vec<_> = world
            .query_entities(&[MChurn::component_id()])
            .into_iter()
            .take(4)
            .collect();
        assert_eq!(victims.len(), 4, "cycle {cycle}: victims resolved");
        world.run_system(move |mut cmds: Commands| {
            for &e in &victims {
                cmds.entity(e).despawn();
            }
        });
        expected -= 4;
        assert_eq!(world.entity_count(), expected, "cycle {cycle}: despawns landed");
    }

    // 3 cycles of +8/-4 leave 12 live rows; the pool's frontier covers the
    // 8-row high-water mark of cycle 0 plus growth (committed monotonic).
    assert_eq!(world.entity_count(), 12);
    let arch_id = world.get_or_create_archetype(&[MChurn::component_id()]);
    let pool = world
        .archetype_master()
        .get_archetype(arch_id)
        .expect("archetype")
        .component_pools()
        .get_pool(MChurn::component_id())
        .expect("pool");
    assert_eq!(pool.count(), 12, "dense row count matches the live population");
    assert!(
        pool.committed_rows() >= 12 && pool.committed_rows() <= pool.capacity(),
        "len <= committed_rows <= reserve_rows invariant holds after churn"
    );

    // Every surviving value reads back through the live rows.
    let view = world.query::<&MChurn, ()>();
    let count = view.iter().count();
    assert_eq!(count, 12, "query sees exactly the survivors");
}
