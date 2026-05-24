// Benchmark: EcsMaster::delete_entity, which exercises ComponentPool::swap_remove
// on every deletion.
//
// Uses the raw byte API (create_entity with Vec<(ComponentId, &[u8])>) because
// no typed create_entity_typed exists yet (tracked as C-010 in the Phase 2a
// roadmap). Component IDs are in the 480-489 range, reserved for this bench
// to avoid collisions with the global OnceLock registry shared across all
// test/bench binaries. (MAX_COMPONENTS = 512; ranges 100-109, 200-209,
// 300-309, 400-409, 450-465 are owned by unit tests; 470-479 by query_iter
// bench; 480-489 by this bench.)

use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

const BENCH_POS_ID: usize = 480;
const BENCH_VEL_ID: usize = 481;

#[repr(C)]
struct BenchPos {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
struct BenchVel {
    vx: f32,
    vy: f32,
    vz: f32,
}

/// Register bench component types under fixed IDs (idempotent -- safe to call
/// multiple times across bench functions in the same process).
fn register_bench_components() {
    component_registry::register_layout::<BenchPos>(BENCH_POS_ID);
    component_registry::register_layout::<BenchVel>(BENCH_VEL_ID);
}

/// Builds an EcsMaster populated with `n` entities, each having Pos + Vel.
/// Returns the master and the list of entity handles for subsequent deletion.
fn build_ecs(n: usize) -> (EcsMaster, Vec<boyko_ecs::ecs::core::entity::entity::Entity>) {
    let mut ecs = EcsMaster::new();
    let arch_id = ecs.create_archetype(&[BENCH_POS_ID, BENCH_VEL_ID]);

    let mut entities = Vec::with_capacity(n);
    for i in 0..n {
        let pos = BenchPos {
            x: i as f32,
            y: 0.0,
            z: 0.0,
        };
        let vel = BenchVel {
            vx: 1.0,
            vy: 0.0,
            vz: 0.0,
        };

        // SAFETY: BenchPos and BenchVel are #[repr(C)] POD types; the byte
        // slices cover exactly size_of::<T>() initialised bytes.
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(
                &pos as *const BenchPos as *const u8,
                std::mem::size_of::<BenchPos>(),
            )
        };
        let vel_bytes = unsafe {
            std::slice::from_raw_parts(
                &vel as *const BenchVel as *const u8,
                std::mem::size_of::<BenchVel>(),
            )
        };

        let entity = ecs
            .create_entity(arch_id, &[(BENCH_POS_ID, pos_bytes), (BENCH_VEL_ID, vel_bytes)])
            .expect("create_entity must succeed in bench setup");
        entities.push(entity);
    }

    (ecs, entities)
}

fn bench_swap_remove(c: &mut Criterion) {
    register_bench_components();

    let mut group = c.benchmark_group("swap_remove");
    for &n in &[100usize, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || build_ecs(n),
                |(mut ecs, entities)| {
                    // Measured: delete every entity; each triggers swap_remove
                    // in all component pools.
                    for entity in &entities {
                        let _ = ecs.delete_entity(*entity);
                    }
                    std::hint::black_box(ecs);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_swap_remove);
criterion_main!(benches);
