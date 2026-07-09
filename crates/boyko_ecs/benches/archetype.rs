// Benchmark: Archetype::create_entity precheck cost (C-16 validation).
//
// Phase X.I note: this bench is RUNNABLE AGAIN. At pre-X.I HEAD it panicked
// ("Arena reserve exhausted"): every iter_batched setup created a fresh
// Archetype whose pools carved ~8 MiB from ONE shared 64 MiB arena that
// never freed across the sample run. Post-X.I each pool owns its own
// VmReservation, released when the Archetype drops — so per-setup memory is
// reclaimed every iteration. (Phase X.J retired the shared Arena outright.)
//
// Two groups:
//   1. archetype_create_entity_8c  — 8-component archetype, 10 000 entities per iteration
//   2. archetype_create_entity_16c — 16-component archetype, 10 000 entities per iteration
//
// Component ID range 420-435 reserved for this bench (verified free in test suites).

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use boyko_ecs::ecs::core::archetype::archetype::Archetype;
use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

// --- Component registration ---

const IDS_8: [ComponentId; 8] = [
    ComponentId(420), ComponentId(421), ComponentId(422), ComponentId(423),
    ComponentId(424), ComponentId(425), ComponentId(426), ComponentId(427),
];
const IDS_16: [ComponentId; 16] = [
    ComponentId(420), ComponentId(421), ComponentId(422), ComponentId(423),
    ComponentId(424), ComponentId(425), ComponentId(426), ComponentId(427),
    ComponentId(428), ComponentId(429), ComponentId(430), ComponentId(431),
    ComponentId(432), ComponentId(433), ComponentId(434), ComponentId(435),
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
        // A fresh Archetype per setup; its pools commit lazily during the
        // timed fill and release their reservations when the setup output
        // drops.
        b.iter_batched(
            || {
                let arch = Archetype::create_by_ids(ArchetypeId(1), &IDS_8);
                let bytes = vec![0u8; 4]; // all components are u32
                let components: Vec<(ComponentId, Vec<u8>)> = IDS_8.iter()
                    .map(|&id| (id, bytes.clone()))
                    .collect();
                (arch, components)
            },
            |(mut arch, components)| {
                for entity_id in 0..n {
                    let slices: Vec<(ComponentId, &[u8])> = components.iter()
                        .map(|(id, v)| (*id, v.as_slice()))
                        .collect();
                    let mut new_unit_index: u32 = 0;
                    let ok = arch.create_entity(EntityId(entity_id), &mut new_unit_index, &slices, Tick::new(1));
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
        b.iter_batched(
            || {
                let arch = Archetype::create_by_ids(ArchetypeId(2), &IDS_16);
                let bytes = vec![0u8; 4];
                let components: Vec<(ComponentId, Vec<u8>)> = IDS_16.iter()
                    .map(|&id| (id, bytes.clone()))
                    .collect();
                (arch, components)
            },
            |(mut arch, components)| {
                for entity_id in 0..n {
                    let slices: Vec<(ComponentId, &[u8])> = components.iter()
                        .map(|(id, v)| (*id, v.as_slice()))
                        .collect();
                    let mut new_unit_index: u32 = 0;
                    let ok = arch.create_entity(EntityId(entity_id), &mut new_unit_index, &slices, Tick::new(1));
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
