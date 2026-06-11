// Benchmark: cold construction cost of `EcsMaster::new()` (Phase X.C gate,
// re-homed by Phase X.J).
//
// History: this group lived in `benches/arena_new.rs` next to the shared
// Arena's acquisition/growth groups (`arena_new`, `arena_first_pool_alloc`,
// `commit_slab`). Phase X.I moved component storage onto per-pool
// `VmReservation`s, and Phase X.J retired the Arena outright — the
// arena-only groups were deleted with it and the world-construction gate
// moved here unchanged.
//
// Gate (binding): `ecs_master_new/EcsMaster::new` <= 7.5 µs (XI-B3 ceiling).
// Post-X.J expectation: equal-or-faster vs the X.I number — the dead
// reserve-only arena acquisition (~0.5-1 µs syscall) is gone.
//
// Measurement methodology: `iter_batched(.., BatchSize::PerIteration)` so the
// constructed world is RETURNED from the timed closure and DROPPED by
// criterion OUTSIDE the timed region, ONE AT A TIME. We are measuring
// *construction* cost, not teardown. The returned value is black-boxed by
// criterion's batched harness, so the construction cannot be optimized away.

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

fn bench_ecs_master_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecs_master_new");
    group.bench_function("EcsMaster::new", |b| {
        b.iter_batched(
            || (),
            // Drop-outside, one-at-a-time discipline (see the file header).
            |_| EcsMaster::new(),
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

criterion_group!(ecs_master_new, bench_ecs_master_new);
criterion_main!(ecs_master_new);
