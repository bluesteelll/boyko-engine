//! Phase X.I — M-XI: Miri (Tree Borrows) coverage for ComponentPool
//! row-capacity growth (`docs/PHASE-XI-PLAN.md` §Test matrix, M-XI).
//!
//! Run via:
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly-x86_64-pc-windows-msvc miri test -p boyko-ecs --test miri_pool_growth
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
//! All raw-pool tests use a 1024-byte stride, so the commit ladder (packing
//! plan D2: doubling from one 4 KiB `COMMIT_PAGE`, measured from the data
//! sub-region's absolute page floor) passes a new frontier every few rows and
//! crosses >= 2 growth events within a Miri-cheap iteration count (the
//! small-ceiling D2 constructor mapping is the test knob — ★R1-9). Under Miri
//! `COMMIT_PAGE` is still 4 KiB: its cfg is `target_arch`, which Miri keeps.
//!
//! A frontier is `(committed data bytes - σ) / 1024` with `σ =
//! pool_base_stagger(id)`, and a derive-minted id depends on mint order, so
//! every frontier pinned below is computed by [`Ladder`] from the fixture's
//! OWN id (cut PC-6) rather than written down. The granule ladder's frontiers
//! (64 / 128 / 256) did not depend on σ. `miri_page_floor_ladder_at_a_pinned_
//! nonzero_stagger` pins a fixed id whose σ is known to be non-zero, which is
//! what the UG-08 mutation row (drop `- σ` from `grow_rows`' row count) needs:
//! the obligation-11 debug twin fires there under Miri.

#![cfg(miri)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::constants::{COMMIT_GRANULE, COMMIT_PAGE, POOL_MAX_SLAB, pool_base_stagger};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use boyko_macros::{Bundle, Component};

/// The packing plan's D2 data ladder for one explicit-ceiling pool of
/// `stride`-byte rows, re-derived from the pool's own stagger: doubling from
/// one `COMMIT_PAGE`, request-dominant, capped at `align_up_page(σ +
/// data_len)`, rows clamped to the ceiling.
struct Ladder {
    sigma: usize,
    stride: usize,
    reserve_rows: usize,
    data_cap: usize,
    data: usize,
    rows: usize,
}

impl Ladder {
    fn new(component_id: usize, stride: usize, reserve_rows: usize) -> Self {
        let sigma = pool_base_stagger(component_id);
        let data_len = (reserve_rows * stride).next_multiple_of(COMMIT_GRANULE);
        Self {
            sigma,
            stride,
            reserve_rows,
            data_cap: (sigma + data_len).next_multiple_of(COMMIT_PAGE),
            data: 0,
            rows: 0,
        }
    }

    /// The committed-row frontier after a request for `n` rows (an `add`
    /// grows exactly when it finds `count == committed_rows`, asking for
    /// `count + 1`).
    fn grow(&mut self, n: usize) -> usize {
        if n > self.rows && n <= self.reserve_rows {
            let needed = (self.sigma + n * self.stride).next_multiple_of(COMMIT_PAGE);
            let step = self.data.clamp(COMMIT_PAGE, POOL_MAX_SLAB).max(needed - self.data);
            self.data = (self.data + step).min(self.data_cap);
            self.rows = ((self.data - self.sigma) / self.stride).min(self.reserve_rows);
        }
        self.rows
    }

    /// The frontier after `adds` one-row-at-a-time adds into a fresh pool.
    fn after_adds(component_id: usize, stride: usize, reserve_rows: usize, adds: usize) -> usize {
        let mut ladder = Self::new(component_id, stride, reserve_rows);
        (1..=adds).map(|n| ladder.grow(n)).last().unwrap_or(0)
    }
}

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
/// A 1 x 256 pool; 200 adds drive the page ladder through its frontiers
/// (re-derived, packing plan S2: `(4096 · 2^k - σ) / 1024` capped at 256 —
/// 4, 8, 16, 32, 64, 128, 256 at σ = 0 and 3, 7, 15, 31, 63, 127, 255 at
/// σ = 64; it was 64 -> 128 -> 256 on the granule ladder, for every σ). Pins
/// the frontier after every add, the write-once base pointers across all
/// events, and full value round-trips through rows on every commit.
#[test]
fn miri_multi_slab_growth_bookkeeping_and_stability() {
    let mut pool = make_pad1k_pool(256);
    assert_eq!(pool.component_layout().size(), 1024, "fixture stride pin");
    assert_eq!(pool.capacity(), 256, "D2 mapping: reserve_rows = 1 * 256");
    assert_eq!(pool.committed_rows(), 0, "D3: zero initial commit");
    let mut ladder = Ladder::new(pool.component_id(), 1024, 256);

    pool.add_typed(Pad1K::new(0)).expect("row 0");
    assert_eq!(pool.committed_rows(), ladder.grow(1), "first commit: the first data page(s)");
    let base = pool.buffer_ptr();
    let row0 = pool.get_raw(0).expect("row 0 live");
    // (Tick-base stability is pinned by the in-file U-P2 unit test — the
    // tick accessors are pub(crate) and unreachable from tests/.)

    let mut events = 1;
    for i in 1..200u64 {
        let before = pool.committed_rows();
        pool.add_typed(Pad1K::new(i)).expect("under the 256-row ceiling");
        assert_eq!(
            pool.committed_rows(),
            ladder.grow(pool.count()),
            "frontier after {} adds",
            pool.count()
        );
        if pool.committed_rows() != before {
            events += 1;
        }
    }
    assert_eq!(pool.count(), 200);
    assert_eq!(pool.committed_rows(), Ladder::after_adds(pool.component_id(), 1024, 256, 200));
    assert!(events >= 3, "the 200 adds must cross >= 2 commit boundaries (events = {events})");

    // Write-once base pointers never moved (Soundness item 1 under TB).
    assert_eq!(pool.buffer_ptr(), base, "data base stable across every growth");
    assert_eq!(pool.get_raw(0).expect("row 0"), row0, "row-0 ptr stable");

    // Every row on every slab round-trips through the recomputed row_ptr.
    for i in 0..200u64 {
        let got = pool.get_typed::<Pad1K>(i as usize).expect("live row");
        assert_eq!(got.id, i, "row {i} id");
        assert_eq!(got.pad, Pad1K::new(i).pad, "row {i} payload");
    }
}

/// M-XI (2) — swap_remove ACROSS commit boundaries under TB: the last row
/// (committed several growth events after the first) is memcpy'd into a hole
/// in an earlier commit — the cross-boundary row_ptr pair the in-place design
/// must keep inside one allocated object (★R1-8).
#[test]
fn miri_swap_remove_across_slab_boundary() {
    let mut pool = make_pad1k_pool(256);

    for i in 0..130u64 {
        pool.add_typed(Pad1K::new(i)).expect("130 rows fit");
    }
    // Re-derived (S2): the frontier after 130 adds is the ladder's — 256 at
    // σ = 0, 255 at σ = 64 (was 256 for every σ).
    assert_eq!(
        pool.committed_rows(),
        Ladder::after_adds(pool.component_id(), 1024, 256, 130),
        "130 rows crossed the commit boundaries"
    );
    assert!(pool.committed_rows() >= 130);

    // Hole in the second commit at most (row 10), donor in the last (row 129).
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

/// M-XI (3) — drop-count-exact across growth boundaries under TB: 70 rows
/// cross the page ladder's boundaries up to the 64-KiB one; one swap_remove
/// drops exactly one; pool Drop
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
    // Re-derived (S2): 128 at σ = 0, 127 at σ = 64 (was 128 for every σ).
    assert_eq!(
        pool.committed_rows(),
        Ladder::after_adds(id.0, 1024, 128, M),
        "the boundary crossings grew the frontier"
    );

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

/// A fixed-id 1024-byte payload for the stagger-pinned ladder test below.
#[repr(C)]
#[derive(Clone, Copy)]
struct Pad1KPinned {
    id: u64,
    pad: [u64; 127],
}

/// Fixed, not derive-minted: σ = (500 % 64) · 64 = 3328 whatever the mint
/// order, far above this binary's handful of derive-minted ids.
const PINNED_ID: usize = 500;

impl Component for Pad1KPinned {
    fn component_id() -> ComponentId {
        component_registry::register_layout::<Pad1KPinned>(PINNED_ID);
        ComponentId(PINNED_ID)
    }
}

/// M-XI (4b) — the page ladder at a KNOWN non-zero stagger (critique O1):
/// σ = 3328, so `σ + 1024 > 4096` and the first data commit is TWO pages
/// (rows (8192 - 3328) / 1024 = 4), the D2 edge case where the stagger pad
/// pushes a row across the first page. Every frontier is pinned against
/// [`Ladder`], the data base is pinned at σ within its page, and every row
/// round-trips. This is the UG-08 mutation row's target: with the `- σ`
/// dropped from `grow_rows`' row count, the first grow exposes 8 rows and
/// the obligation-11 debug twin (`σ + rows · stride <= data_committed`)
/// fires under Miri.
#[test]
fn miri_page_floor_ladder_at_a_pinned_nonzero_stagger() {
    let id = Pad1KPinned::component_id();
    let sigma = pool_base_stagger(id.0);
    assert_eq!(sigma, 3328, "the fixture needs σ != 0 and σ + 1024 > COMMIT_PAGE");
    let mut pool = ComponentPool::new(id.0, 256);
    let mut ladder = Ladder::new(id.0, 1024, 256);
    assert_eq!(pool.buffer_ptr() as usize % COMMIT_PAGE, sigma, "the data base carries σ");

    for i in 0..40u64 {
        pool.add_typed(Pad1KPinned { id: i, pad: [i ^ 0x5A5A; 127] }).expect("under 256");
        assert_eq!(pool.committed_rows(), ladder.grow(pool.count()), "after {} adds", i + 1);
        if i == 0 {
            assert_eq!(pool.committed_rows(), (2 * COMMIT_PAGE - sigma) / 1024, "two pages");
        }
    }
    assert_eq!(pool.committed_rows(), Ladder::after_adds(id.0, 1024, 256, 40));
    for i in 0..40u64 {
        let got = pool.get_typed::<Pad1KPinned>(i as usize).expect("live row");
        assert_eq!((got.id, got.pad[126]), (i, i ^ 0x5A5A), "row {i} round-trips");
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
