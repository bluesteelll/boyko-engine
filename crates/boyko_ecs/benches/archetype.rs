// Benchmark: Archetype::create_entity precheck cost (C-16 validation).
//
// Two groups:
//   1. archetype_create_entity_8c  — 8-component archetype, 10 000 entities per iteration
//   2. archetype_create_entity_16c — 16-component archetype, 10 000 entities per iteration
//
// Component ID range 420-435 reserved for this bench (verified free in test suites).

use boyko_ecs::ecs::core::archetype::archetype::Archetype;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::entity::entity_inland::EntityInland;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::ecs::memory::arena::Arena;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

// --- Component registration ---

const IDS_8: [ComponentId; 8] = [420, 421, 422, 423, 424, 425, 426, 427];
const IDS_16: [ComponentId; 16] = [
    420, 421, 422, 423, 424, 425, 426, 427,
    428, 429, 430, 431, 432, 433, 434, 435,
];

fn register_bench_components() {
    // 16 distinct u32-sized components registered under IDs 420-435.
    // OnceLock makes this idempotent across repeated criterion iterations.
    macro_rules! reg {
        ($id:expr, $t:ident) => {{
            #[repr(C)]
            struct $t(u32);
            component_registry::register_layout::<$t>($id);
        }};
    }
    reg!(420, BC420); reg!(421, BC421); reg!(422, BC422); reg!(423, BC423);
    reg!(424, BC424); reg!(425, BC425); reg!(426, BC426); reg!(427, BC427);
    reg!(428, BC428); reg!(429, BC429); reg!(430, BC430); reg!(431, BC431);
    reg!(432, BC432); reg!(433, BC433); reg!(434, BC434); reg!(435, BC435);
}

// --- Group 1: 8-component archetype ---

fn bench_archetype_create_entity_8c(c: &mut Criterion) {
    register_bench_components();

    let mut group = c.benchmark_group("archetype_create_entity_8c");
    let n = 10_000usize;

    group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
        // 64 MB arena: 8 pools × 10k entities × 4 bytes = ~320 KB; ample.
        let arena = Arena::with_capacity(64 * 1024 * 1024);

        // One archetype lives outside iter_batched so pools are pre-allocated.
        // We reset via a fresh Archetype in the setup closure.
        b.iter_batched(
            || {
                let arch = Archetype::create_by_ids(1, &IDS_8, &arena);
                let bytes = vec![0u8; 4]; // all components are u32
                let components: Vec<(ComponentId, Vec<u8>)> = IDS_8.iter()
                    .map(|&id| (id, bytes.clone()))
                    .collect();
                (arch, components)
            },
            |(mut arch, components)| {
                for entity_id in 0..n {
                    let mut inland = EntityInland::new(arch.id(), 0, 0);
                    arch.init_entity_inland(&mut inland);
                    let slices: Vec<(ComponentId, &[u8])> = components.iter()
                        .map(|(id, v)| (*id, v.as_slice()))
                        .collect();
                    let ok = arch.create_entity(entity_id, &mut inland, &slices);
                    black_box(ok);
                }
                black_box(arch);
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

// --- Group 2: 16-component archetype ---

fn bench_archetype_create_entity_16c(c: &mut Criterion) {
    register_bench_components();

    let mut group = c.benchmark_group("archetype_create_entity_16c");
    let n = 10_000usize;

    group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
        let arena = Arena::with_capacity(128 * 1024 * 1024);

        b.iter_batched(
            || {
                let arch = Archetype::create_by_ids(2, &IDS_16, &arena);
                let bytes = vec![0u8; 4];
                let components: Vec<(ComponentId, Vec<u8>)> = IDS_16.iter()
                    .map(|&id| (id, bytes.clone()))
                    .collect();
                (arch, components)
            },
            |(mut arch, components)| {
                for entity_id in 0..n {
                    let mut inland = EntityInland::new(arch.id(), 0, 0);
                    arch.init_entity_inland(&mut inland);
                    let slices: Vec<(ComponentId, &[u8])> = components.iter()
                        .map(|(id, v)| (*id, v.as_slice()))
                        .collect();
                    let ok = arch.create_entity(entity_id, &mut inland, &slices);
                    black_box(ok);
                }
                black_box(arch);
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(
    archetype,
    bench_archetype_create_entity_8c,
    bench_archetype_create_entity_16c
);
criterion_main!(archetype);
