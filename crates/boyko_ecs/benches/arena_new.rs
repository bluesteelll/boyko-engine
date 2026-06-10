// Benchmark: cold construction cost of the arena-backed objects (Phase X.C,
// re-premised by Phase X.F) + the X.F growth-event costs.
//
// Phase X.C cut `EcsMaster::new`'s arena residual to ~1.10 µs with a single
// reserve+commit syscall. Phase X.F goes further: `Arena::new()` is now
// RESERVE-ONLY (a multi-GB PAGE_NOACCESS/PROT_NONE reservation, ZERO commit
// charge), so construction pays one address-space syscall and nothing else.
// The deferred cost — one slab commit at the first pool allocation — is
// measured here too (`arena_first_pool_alloc`, gate B7), as is the raw
// per-slab commit syscall cost (`commit_slab`, gate B4).
//
// Groups:
//   1. bench_arena_new            — `Arena::new()` (default reserve-only
//                                   acquisition). Gate B2: <= 1.10 µs.
//   2. bench_ecs_master_new       — `EcsMaster::new()` (arena + the lazy-init
//                                   field wrappers from Phase 12.6). Gate B3.
//   3. bench_arena_first_pool_alloc — cold default arena + one 3 MiB
//                                   pool-class request: the deferred-commit
//                                   cost made visible. Gate B7: <= 10 µs.
//   4. bench_commit_slab          — isolated growth events of 2/16/64 MiB
//                                   (R2-N1 recipes). Gate B4: 2 MiB <= 10 µs,
//                                   64 MiB <= 50 µs.
//
// Measurement methodology: `iter_batched(.., BatchSize::PerIteration)` so the
// constructed value is RETURNED from the timed closure and DROPPED by
// criterion OUTSIDE the timed region, ONE AT A TIME. We are measuring
// *acquisition*/*growth* cost, not teardown (`VirtualFree` / `munmap` in
// `Drop`). The returned value is black-boxed by criterion's batched harness,
// so the construction cannot be optimized away.
//
// Why `PerIteration` is kept: the Phase X.C commit-charge constraint is GONE
// (a reserve-only arena charges nothing until it grows), but one-at-a-time
// keeps teardown symmetric and bounds live address-space usage — dozens of
// batched multi-GB reservations per batch would stress the VA allocator for
// no methodological gain. For the `commit_slab` group the constraint is
// real again in miniature: each iteration's arena holds its committed slab
// until criterion drops it, and PerIteration keeps exactly one alive.

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::alloc::Layout;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::memory::arena::Arena;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::hint::black_box;

// --- Group 1: Arena::new() — the headline reserve-only acquisition ---

fn bench_arena_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena_new");
    group.bench_function("Arena::new (default reserve)", |b| {
        b.iter_batched(
            || (),
            // The constructed `Arena` is returned; criterion drops it OUTSIDE
            // the timed closure, so only acquisition is measured (see the
            // file header). Phase X.F: this is one MEM_RESERVE/PROT_NONE
            // syscall — no commit, no charge, free list seeded empty.
            |_| Arena::new(),
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

// --- Group 2: EcsMaster::new() — arena + lazy-init field wrappers ---

fn bench_ecs_master_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecs_master_new");
    group.bench_function("EcsMaster::new", |b| {
        b.iter_batched(
            || (),
            // Same drop-outside, one-at-a-time discipline as `bench_arena_new`.
            // If this lands materially above `Arena::new`, the residual is some
            // OTHER lazy-init cost in `EcsMaster::new`, not the arena.
            |_| EcsMaster::new(),
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

// --- Group 3: first pool-class allocation on a cold default arena (B7) ---

fn bench_arena_first_pool_alloc(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena_first_pool_alloc");
    // The production shape: `ComponentPool::new` requests one contiguous
    // ~3 MiB block (128 chunks x 2048 tiny slots x 12 B) at
    // SIMD_BUFFER_ALIGN (32). The cold grow path commits
    // max(ARENA_MIN_SLAB, request) in ONE commit_frontier event.
    let layout = Layout::from_size_align(3 * 1024 * 1024, 32).expect("valid layout");
    group.bench_function("cold default arena + 3MiB pool request", |b| {
        b.iter_batched(
            Arena::new,
            |arena| {
                let p = arena.allocate_layout(layout);
                black_box(p);
                // Return the arena so its Drop (reservation release) runs
                // OUTSIDE the timed region.
                arena
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

// --- Group 4: isolated slab-commit events (B4, R2-N1 recipes) ---

fn bench_commit_slab(c: &mut Criterion) {
    let mut group = c.benchmark_group("commit_slab");

    // R2-N1 pinned recipes — fresh `with_reserve(256 MiB, 0)` per iteration
    // (cheap now: reserve-only), one allocation whose grow event commits
    // exactly the named slab size:
    //   2 MiB:  one 64 KiB request — step = ARENA_MIN_SLAB.
    //   16 MiB: one request of 16 MiB - GRANULE — request-dominant (`needed`
    //           granule-rounds to exactly 16 MiB).
    //   64 MiB: one request of 64 MiB - GRANULE — `needed` is NOT
    //           MAX_SLAB-clamped in D4's `max(clamp(..), needed)`; single
    //           huge requests must be one event, and this exercises exactly
    //           that.
    const GRANULE: usize = 64 * 1024;
    let cases: [(&str, usize); 3] = [
        ("2MiB", 64 * 1024),
        ("16MiB", 16 * 1024 * 1024 - GRANULE),
        ("64MiB", 64 * 1024 * 1024 - GRANULE),
    ];

    for (name, request) in cases {
        let layout = Layout::from_size_align(request, 64).expect("valid layout");
        group.bench_function(name, |b| {
            b.iter_batched(
                || Arena::with_reserve(256 * 1024 * 1024, 0),
                |arena| {
                    let p = arena.allocate_layout(layout);
                    black_box(p);
                    // Drop (release of the partially-committed reservation)
                    // happens outside the timed region.
                    arena
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group!(
    arena_new,
    bench_arena_new,
    bench_ecs_master_new,
    bench_arena_first_pool_alloc,
    bench_commit_slab,
);
criterion_main!(arena_new);
