// Benchmark: MemFreeBlockMaster allocator operations (M-012 validation).
//
// Five criterion groups covering:
//   1. insert_disjoint   — pure inserts with no coalescing (sweep N=64/1k/16k)
//   2. insert_coalescing — every insert merges both left and right neighbors
//   3. alloc_free_roundtrip — steady-state alloc + re-insert
//   4. alloc_cold        — single allocate_aligned against a pre-filled pool
//   5. arena_allocate_layout — end-to-end via Arena (headline metric, NIT3)
//
// Baseline + comparison numbers vs HashMap are captured post-merge.

use boyko_ecs::ecs::memory::arena::Arena;
use boyko_ecs::ecs::memory::free_mem_block::{MemFreeBlock, MemFreeBlockMaster};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use std::alloc::Layout;

// --- Group 1: insert without coalescing (disjoint blocks) ---

fn bench_insert_disjoint(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_disjoint");
    for &n in &[64usize, 1_000, 16_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || MemFreeBlockMaster::with_capacity(n),
                |mut master| {
                    for i in 0..n {
                        // Gap of 100 between each 100-byte block ensures no merging.
                        let start = i * 200;
                        master.insert(MemFreeBlock::new(start, start + 100));
                    }
                    black_box(master);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// --- Group 2: insert with full coalescing (each insert merges both neighbors) ---

fn bench_insert_coalescing(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_coalescing");
    for &n in &[64usize, 1_000, 16_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    // Pre-fill N blocks, each 100 bytes, with a 100-byte gap between.
                    let mut master = MemFreeBlockMaster::with_capacity(n * 2);
                    for i in 0..n {
                        let start = i * 200;
                        master.insert(MemFreeBlock::new(start, start + 100));
                    }
                    master
                },
                |mut master| {
                    // Fill each gap — every insert merges both its left and right neighbors.
                    for i in 0..(n.saturating_sub(1)) {
                        let start = i * 200 + 100;
                        master.insert(MemFreeBlock::new(start, start + 100));
                    }
                    black_box(master);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// --- Group 3: alloc + insert roundtrip (steady state) ---

fn bench_alloc_free_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("alloc_free_roundtrip");
    for &n in &[1_000usize, 16_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    // Single large block representing the arena free space.
                    let mut master = MemFreeBlockMaster::with_capacity(n);
                    master.insert(MemFreeBlock::new(0, n * 128));
                    master
                },
                |mut master| {
                    // Alternate alloc + re-insert: steady-state free-list churn.
                    for _ in 0..n {
                        if let Some(block) = master.allocate(64) {
                            master.insert(block);
                        }
                    }
                    black_box(master);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// --- Group 4: cold allocate_aligned (single alloc per iteration, fresh state) ---

fn bench_alloc_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("alloc_cold");
    for &n in &[1_000usize, 16_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    // Pre-fill pool with N disjoint 128-byte blocks.
                    let mut master = MemFreeBlockMaster::with_capacity(n);
                    for i in 0..n {
                        let start = i * 256;
                        master.insert(MemFreeBlock::new(start, start + 128));
                    }
                    master
                },
                |mut master| {
                    // Single aligned allocation — cold in the sense that the pool
                    // state is stable and we measure one lookup + remove.
                    let result = master.allocate_aligned(64, 64);
                    black_box(result);
                    black_box(master);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// --- Group 5: end-to-end via Arena::allocate_layout (headline metric, NIT3) ---

fn bench_arena_allocate_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena_allocate_layout");
    // Arena capacity must comfortably fit all iterations without OOM.
    // 16k * 128 B = 2 MB; 64 MB default arena is ample.
    for &n in &[1_000usize, 16_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let layout = Layout::from_size_align(64, 64).expect("valid layout");
            b.iter_batched(
                || Arena::with_capacity(n * 256),
                |arena| {
                    for _ in 0..n {
                        let ptr = arena.allocate_layout(layout);
                        black_box(ptr);
                    }
                    black_box(arena);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    allocator,
    bench_insert_disjoint,
    bench_insert_coalescing,
    bench_alloc_free_roundtrip,
    bench_alloc_cold,
    bench_arena_allocate_layout
);
criterion_main!(allocator);
