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

/// Builds an empty pool with an exact `cap`-row ceiling.
///
/// Phase X.I/X.J note: the pool is reserve-only at construction, so the
/// fill loop's first adds include the pool's own cold `grow_rows` commit
/// events — identical to the post-X.I shape of this bench (the historical
/// `sized_arena` fixture stopped pre-committing pool memory in X.I and was
/// deleted with the shared Arena in X.J).
fn empty_pool(cap: usize) -> ComponentPool {
    register();
    ComponentPool::new(POOL_BENCH_ID.0, cap)
}

// ── add: fill a pool from empty (the removed per-row Unit-write path) ────────

fn bench_pool_fill(c: &mut Criterion) {
    register();
    let mut group = c.benchmark_group("ComponentPool::add fill");
    for &n in &[100usize, 1_000, 10_000] {
        group.throughput(criterion::Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            // Pool construction + pre-grow are part of setup (NOT
            // measured) so the timed region is purely the `add` loop — i.e.
            // exactly the per-row work the `Vec<Unit>` removal affected.
            b.iter_batched_ref(
                || empty_pool(n),
                |pool| {
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
                    let mut pool = empty_pool(n);
                    let p = Payload { a: 1, b: 2 };
                    let bytes = payload_bytes(&p);
                    for _ in 0..n {
                        pool.add(bytes).expect("setup fill within capacity");
                    }
                    pool
                },
                |pool| {
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
    let mut pool = empty_pool(10_000);
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
