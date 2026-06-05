// Benchmark: cold construction cost of the arena-backed objects (Phase X.C).
//
// THE primary gate for Phase X.C: cutting the arena-acquisition residual of
// `EcsMaster::new` from ~23-75 µs to <= 5 µs by replacing the eager 64 MB
// `std::alloc::alloc` (which memsets the whole buffer) with a single OS
// reserve+commit (`VirtualAlloc` on Windows / `mmap` on Unix) whose physical
// pages fault in lazily on first touch.
//
// Two groups:
//   1. bench_arena_new       — `Arena::new()` (the default 64 MB acquisition).
//   2. bench_ecs_master_new  — `EcsMaster::new()` (arena + the lazy-init field
//                              wrappers from Phase 12.6).
//
// Measurement methodology: `iter_batched(|| (), |_| Arena::new(),
// BatchSize::PerIteration)` so the constructed value is RETURNED from the timed
// closure and DROPPED by criterion OUTSIDE the timed region, ONE AT A TIME. We
// are measuring *acquisition* cost, not teardown (the `VirtualFree` / `munmap`
// in `Drop`). The returned value is black-boxed by criterion's batched harness,
// so the construction cannot be optimized away.
//
// Why `PerIteration` and not `SmallInput`: with the Phase X.C syscall backing,
// `VirtualAlloc(MEM_RESERVE | MEM_COMMIT)` charges the full 64 MB against the OS
// commit limit up front (same charge as the old eager `alloc` did). `SmallInput`
// batches dozens-to-hundreds of iterations and holds EVERY returned value alive
// until the batch ends, so N live 64 MB arenas = N * 64 MB committed
// simultaneously — on a commit-constrained host that exhausts the commit limit
// and `VirtualAlloc` returns NULL (the `expect` then panics). That is a
// benchmark artifact, not an engine defect (real usage holds exactly one arena
// per world). `PerIteration` keeps at most one arena alive at a time while still
// dropping it outside the timed closure.
//
// Verdict: median <= 5 µs == PASS. The expected win is ~10-15x, far above any
// thermal noise floor.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::memory::arena::Arena;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

// --- Group 1: Arena::new() — the headline 64 MB acquisition ---

fn bench_arena_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena_new");
    group.bench_function("Arena::new (64MB)", |b| {
        b.iter_batched(
            || (),
            // The constructed `Arena` is returned; criterion drops it OUTSIDE
            // the timed closure, so only acquisition is measured. `PerIteration`
            // keeps at most one 64 MB committed arena alive at a time (see the
            // file header). Returning it also prevents dead-code elimination of
            // the construction.
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

criterion_group!(arena_new, bench_arena_new, bench_ecs_master_new);
criterion_main!(arena_new);
