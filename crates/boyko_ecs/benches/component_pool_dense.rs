//! Phase X.B — `ComponentPool` dense `Vec<Unit>`-elimination micro-benchmarks.
//!
//! These benches isolate the pool-level hot operations that the refactor
//! touched, with **no `EcsMaster::new` / command-queue allocator noise** in the
//! measured region (the spawn benches in `phase12_5_spawn_batch` /`phase8cd`
//! carry that ±20-30% setup variance). The structural win of Phase X.B is:
//!
//!   * `add` / batch-commit no longer writes a parallel `Unit { ptr, .. }` row
//!     per element (the per-row pointer is now *computed* via `row_ptr`);
//!   * the pool no longer heap-allocates the `Vec<Unit>` backing store.
//!
//! `bench_pool_fill` measures filling a pool from empty (the path where the
//! per-row `Unit` write used to live). `bench_pool_swap_remove` measures the
//! dense swap-remove (memcpy of the last row into the hole) in isolation.
//! `bench_pool_get_raw` measures the on-demand `row_ptr` recomputation that
//! replaced the cached `Unit.ptr()` read.
//!
//! Run: `cargo bench -p boyko-ecs --bench component_pool_dense`.

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::ecs::memory::arena::Arena;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use criterion::{
    BatchSize, BenchmarkId, Criterion, black_box, criterion_group, criterion_main,
};

// Slot range 240-241 — reserved for this bench (no collision with the
// component_pool unit tests at 220-225 or other bench/test binaries; the
// global OnceLock registry is shared across all binaries in the process).
const POOL_BENCH_ID: ComponentId = ComponentId(240);

/// A 16-byte POD payload (clean power-of-2 stride) representative of a small
/// gameplay component (e.g. `Vec2<f64>` or two `u64` handles).
#[repr(C)]
#[derive(Clone, Copy)]
struct Payload {
    a: u64,
    b: u64,
}

fn register() {
    component_registry::register_layout::<Payload>(POOL_BENCH_ID.0);
}

#[inline]
fn payload_bytes(p: &Payload) -> &[u8] {
    // SAFETY: Payload is #[repr(C)] POD; the slice covers exactly size_of bytes.
    unsafe {
        std::slice::from_raw_parts(
            (p as *const Payload).cast::<u8>(),
            std::mem::size_of::<Payload>(),
        )
    }
}

/// Builds an empty pool sized to hold at least `cap` rows in a single chunk.
fn empty_pool(arena: &Arena, cap: usize) -> ComponentPool {
    register();
    ComponentPool::new(arena, POOL_BENCH_ID.0, 1, cap)
}

/// A right-sized arena for `cap` `Payload` rows + alignment slack.
///
/// The default `Arena::new()` reserves+commits the full 64 MB
/// `DEFAULT_ARENA_SIZE`; criterion's batched setup would hold many such
/// arenas alive at once and exhaust the commit charge. The pool buffer needs
/// `cap * 16` bytes, so a 4 MB arena covers the largest (10k) case with room
/// to spare while keeping the simultaneous-setup footprint small.
fn sized_arena(cap: usize) -> Arena {
    let bytes = (cap * std::mem::size_of::<Payload>()).max(4 * 1024 * 1024);
    Arena::with_capacity(bytes)
}

// ── add: fill a pool from empty (the removed per-row Unit-write path) ────────

fn bench_pool_fill(c: &mut Criterion) {
    register();
    let mut group = c.benchmark_group("ComponentPool::add fill");
    for &n in &[100usize, 1_000, 10_000] {
        group.throughput(criterion::Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            // The arena alloc + pool construction are part of setup (NOT
            // measured) so the timed region is purely the `add` loop — i.e.
            // exactly the per-row work the `Vec<Unit>` removal affected.
            b.iter_batched_ref(
                || {
                    let arena = sized_arena(n);
                    let pool = empty_pool(&arena, n);
                    (arena, pool)
                },
                |(_arena, pool)| {
                    let p = Payload { a: 0xDEAD, b: 0xBEEF };
                    let bytes = payload_bytes(&p);
                    for _ in 0..n {
                        black_box(pool.add(black_box(bytes)));
                    }
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

// ── swap_remove: dense last-into-hole memcpy in isolation ───────────────────

fn bench_pool_swap_remove(c: &mut Criterion) {
    register();
    let mut group = c.benchmark_group("ComponentPool::swap_remove");
    for &n in &[100usize, 1_000, 10_000] {
        group.throughput(criterion::Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched_ref(
                || {
                    let arena = sized_arena(n);
                    let mut pool = empty_pool(&arena, n);
                    let p = Payload { a: 1, b: 2 };
                    let bytes = payload_bytes(&p);
                    for _ in 0..n {
                        pool.add(bytes).expect("setup fill within capacity");
                    }
                    (arena, pool)
                },
                |(_arena, pool)| {
                    // Always remove index 0 → every removal is a real
                    // last-into-hole memcpy (never the trivial last-row case).
                    while pool.count() > 1 {
                        black_box(pool.swap_remove(0));
                    }
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

// ── get_raw: on-demand row_ptr recompute (replaced cached Unit.ptr()) ────────

fn bench_pool_get_raw(c: &mut Criterion) {
    register();
    let arena = sized_arena(10_000);
    let mut pool = empty_pool(&arena, 10_000);
    let p = Payload { a: 7, b: 9 };
    let bytes = payload_bytes(&p);
    for _ in 0..10_000 {
        pool.add(bytes).expect("setup fill");
    }

    c.bench_function("ComponentPool::get_raw row_ptr recompute", |b| {
        let mut idx = 0usize;
        b.iter(|| {
            idx = idx.wrapping_add(1);
            let i = idx % 10_000;
            black_box(pool.get_raw(black_box(i)))
        });
    });
}

criterion_group!(
    benches,
    bench_pool_fill,
    bench_pool_swap_remove,
    bench_pool_get_raw,
);
criterion_main!(benches);
