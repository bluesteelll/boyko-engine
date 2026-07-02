// Benchmark: cached query-state iteration + zero-alloc archetype matching.
//
// The legacy one-shot `Query<'a>` stack (and its `entity_count` / `raw_ptr` /
// `with_typed` / `with_component_ids` arms) has been retired — the typed
// `Query<D, F>` DSL is the single production stack. What remains here measures
// the two primitives that stack is built on:
//
//   1. `QueryState` cached warm-path iteration (`iter_pre_terms`).
//   2. The zero-alloc `find_archetypes_with_components_into` registry scan.
//
// Component IDs 470-479 are reserved for this bench to avoid collisions with
// the global OnceLock registry shared across all test/bench binaries.
// (MAX_COMPONENTS = 512; ranges 100-109, 200-209, 300-309, 400-409, 450-465
// are owned by unit tests; 470-479 by this bench; 480-489 by swap_remove bench.)

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query_state::QueryState;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;

const QBENCH_POS_ID: ComponentId = ComponentId(470);
const QBENCH_VEL_ID: ComponentId = ComponentId(471);

#[repr(C)]
struct QBenchPos {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
struct QBenchVel {
    vx: f32,
    vy: f32,
    vz: f32,
}

impl Component for QBenchPos {
    fn component_id() -> ComponentId { QBENCH_POS_ID }
}

impl Component for QBenchVel {
    fn component_id() -> ComponentId { QBENCH_VEL_ID }
}

fn register_query_bench_components() {
    component_registry::register_layout::<QBenchPos>(QBENCH_POS_ID.0);
    component_registry::register_layout::<QBenchVel>(QBENCH_VEL_ID.0);
}

/// Builds an EcsMaster with `n` entities split across two archetypes:
///   - archetype A: [Pos, Vel]  (n entities)
///   - archetype B: [Pos]       (n entities)
///
/// A query for [Pos] matches both; a query for [Pos, Vel] matches only A.
/// This exercises `find_archetypes_with_components` across multiple archetypes.
fn build_query_ecs(n: usize) -> EcsMaster {
    let mut ecs = EcsMaster::new();
    let arch_pos_vel = ecs.create_archetype(&[QBENCH_POS_ID, QBENCH_VEL_ID]);
    let arch_pos = ecs.create_archetype(&[QBENCH_POS_ID]);

    for i in 0..n {
        let pos = QBenchPos {
            x: i as f32,
            y: 0.0,
            z: 0.0,
        };
        let vel = QBenchVel {
            vx: 1.0,
            vy: 0.0,
            vz: 0.0,
        };

        // SAFETY: QBenchPos/QBenchVel are #[repr(C)] POD; slices cover exactly
        // size_of::<T>() initialised bytes.
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(
                &pos as *const QBenchPos as *const u8,
                std::mem::size_of::<QBenchPos>(),
            )
        };
        let vel_bytes = unsafe {
            std::slice::from_raw_parts(
                &vel as *const QBenchVel as *const u8,
                std::mem::size_of::<QBenchVel>(),
            )
        };

        // Entity in Pos+Vel archetype
        ecs.create_entity(
            arch_pos_vel,
            &[(QBENCH_POS_ID, pos_bytes), (QBENCH_VEL_ID, vel_bytes)],
        )
        .expect("create_entity Pos+Vel must succeed");

        // Entity in Pos-only archetype
        ecs.create_entity(arch_pos, &[(QBENCH_POS_ID, pos_bytes)])
            .expect("create_entity Pos must succeed");
    }

    ecs
}

/// Cached path: `QueryState` is built and warmed up **once** outside `b.iter`.
/// Each iteration calls `state.iter_pre_terms()` which, on the warm path
/// (generation unchanged), costs: one `generation` load + compare, then a
/// slice walk + per-id `get_archetype` (SparseMap O(1)).
fn bench_query_state_iter(c: &mut Criterion) {
    register_query_bench_components();

    let mut group = c.benchmark_group("query_state_iter");
    for &n in &[1_000usize, 10_000, 100_000] {
        let ecs = build_query_ecs(n);

        // Build QueryState ONCE outside b.iter — this is the cached warm path.
        let mut state = QueryState::with_component_ids(&[QBENCH_POS_ID, QBENCH_VEL_ID]);
        state.update_archetypes(ecs.archetype_master());

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                // Warm path: generation unchanged, no archetype scan.
                // Phase 22 D4: `iter_pre_terms` — the raw term-agnostic walk
                // (this bench measures the shared cache, not tag terms).
                let mut sum = 0usize;
                for arch in state.iter_pre_terms(ecs.archetype_master()) {
                    sum += arch.entity_count();
                }
                black_box(sum)
            });
        });
    }
    group.finish();
}

/// Q-013 baseline: the zero-alloc steady-state `find_*_into` path with a
/// pre-warmed `Vec` — the registry-scan primitive the typed query stack calls
/// under `QueryState::update_archetypes`.
fn bench_query_find_into(c: &mut Criterion) {
    register_query_bench_components();

    let mut group = c.benchmark_group("query_find_into");
    for &n in &[1_000usize, 10_000, 100_000] {
        let ecs = build_query_ecs(n);
        let registry = ecs.archetype_master().archetype_registry();

        // Pre-allocate the output buffer once outside the iter loop so the
        // steady-state (post-warmup) measurements reflect zero-alloc behaviour.
        let mut out: Vec<ArchetypeId> = Vec::with_capacity(8);
        // Warmup: fill the buffer and let it reach stable capacity.
        registry.find_archetypes_with_components_into(&[QBENCH_POS_ID, QBENCH_VEL_ID], &mut out);

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                // Measures the Q-013 win directly — `out` is reused each iteration.
                registry.find_archetypes_with_components_into(
                    &[QBENCH_POS_ID, QBENCH_VEL_ID],
                    &mut out,
                );
                black_box(out.len())
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_query_state_iter,
    bench_query_find_into,
);
criterion_main!(benches);
