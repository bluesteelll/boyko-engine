// Benchmark: full query iteration over entities -- Phase 2a Q-011 baseline.
//
// Current Query API iterates archetypes, not individual entities. Per-entity
// component access is via `archetype.get_component_raw(inland, id)`, which
// requires an EntityInland per entity. The EcsMaster does not currently expose
// a typed per-entity iterator (tracked as Q-011 in the Phase 2a roadmap).
//
// This bench establishes the baseline cost of:
//   1. Query::with_component_ids -- rebuilds Vec<&Archetype> per call (Q-011).
//   2. Iterating archetypes + counting entities.
//   3. Reading raw component pointers per entity via get_component_raw.
//
// After Q-011 lands (QueryState cache), re-run this bench and compare.
//
// Component IDs 470-479 are reserved for this bench to avoid collisions with
// the global OnceLock registry shared across all test/bench binaries.
// (MAX_COMPONENTS = 512; ranges 100-109, 200-209, 300-309, 400-409, 450-465
// are owned by unit tests; 470-479 by this bench; 480-489 by swap_remove bench.)

use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query_state::QueryState;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;

const QBENCH_POS_ID: usize = 470;
const QBENCH_VEL_ID: usize = 471;

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

fn register_query_bench_components() {
    component_registry::register_layout::<QBenchPos>(QBENCH_POS_ID);
    component_registry::register_layout::<QBenchVel>(QBENCH_VEL_ID);
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
            vec![(QBENCH_POS_ID, pos_bytes), (QBENCH_VEL_ID, vel_bytes)],
        )
        .expect("create_entity Pos+Vel must succeed");

        // Entity in Pos-only archetype
        ecs.create_entity(arch_pos, vec![(QBENCH_POS_ID, pos_bytes)])
            .expect("create_entity Pos must succeed");
    }

    ecs
}

/// Baseline: Query::with_component_ids rebuilds Vec<&Archetype> on every call.
/// Measures archetype-scan cost (Q-011 hot path).
fn bench_query_iter_entity_count(c: &mut Criterion) {
    register_query_bench_components();

    let mut group = c.benchmark_group("query_iter");
    for &n in &[1_000usize, 10_000, 100_000] {
        let ecs = build_query_ecs(n);

        group.bench_with_input(BenchmarkId::new("entity_count", n), &n, |b, _| {
            b.iter(|| {
                // Q-011 baseline: rebuild Vec<&Archetype> per call.
                let query = Query::with_component_ids(
                    ecs.archetype_master(),
                    &[QBENCH_POS_ID, QBENCH_VEL_ID],
                );
                // Iterate archetypes and sum entity counts -- exercises the
                // archetype loop without per-entity pointer resolution.
                let mut total: usize = 0;
                for archetype in query.iter() {
                    total += archetype.entity_count();
                }
                black_box(total)
            });
        });
    }
    group.finish();
}

/// Extended baseline: read raw component pointer for every entity in every
/// matching archetype. Exercises the full per-entity access path.
///
/// Note: this requires an EntityInland per entity, which the current public
/// API does not expose in a bulk-iteration form. We approximate by scanning
/// entity_ids via `get_entity_id_at` + looking up inland via
/// `entity_master().get_entity_inland(entity)`. This is the slow path; Q-011
/// will eliminate the per-call Vec rebuild and Phase 3a will provide a faster
/// inland-access pattern.
fn bench_query_iter_raw_ptr(c: &mut Criterion) {
    register_query_bench_components();

    let mut group = c.benchmark_group("query_iter_raw_ptr");
    for &n in &[1_000usize, 10_000] {
        let ecs = build_query_ecs(n);

        group.bench_with_input(BenchmarkId::new("sum_x", n), &n, |b, _| {
            b.iter(|| {
                let query = Query::with_component_ids(
                    ecs.archetype_master(),
                    &[QBENCH_POS_ID, QBENCH_VEL_ID],
                );
                let mut sum = 0.0f32;
                for archetype in query.iter() {
                    for unit_index in 0..archetype.entity_count() {
                        if let Some(entity_id) = archetype.get_entity_id_at(unit_index) {
                            if let Some(entity) = ecs.get_entity(entity_id) {
                                if let Some(ptr) = ecs.get_component_raw(entity, QBENCH_POS_ID) {
                                    // SAFETY: ptr returned by get_component_raw points to a
                                    // fully-initialised QBenchPos stored in the ComponentPool.
                                    // The pool lives inside ecs which is borrowed immutably for
                                    // the duration of this loop -- no writes occur concurrently.
                                    let pos = unsafe { &*(ptr as *const QBenchPos) };
                                    sum += pos.x;
                                }
                            }
                        }
                    }
                }
                black_box(sum)
            });
        });
    }
    group.finish();
}

/// Cached path: `QueryState` is built and warmed up **once** outside `b.iter`.
/// Each iteration calls `state.iter()` which, on the warm path (generation
/// unchanged), costs: one `generation` load + compare, then a slice walk +
/// per-id `get_archetype` (SparseMap O(1)).
///
/// Compare against `query_iter::entity_count` to measure the per-call
/// `find_archetypes_with_components` + `Vec` allocation savings.
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
                let mut sum = 0usize;
                for arch in state.iter(ecs.archetype_master()) {
                    sum += arch.entity_count();
                }
                black_box(sum)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_query_iter_entity_count, bench_query_iter_raw_ptr, bench_query_state_iter);
criterion_main!(benches);
