// Phase 7 Step 10 / D10 — Criterion bench suite for fast random component access.
//
// Measures the 11 benches enumerated in the architectural plan
// (`docs/PHASE-07-fast-random-access.md`, Step 10):
//
//   1. bench_get_component_raw_hot       — target ≤ 16 ns/op
//   2. bench_get_component_raw_cold      — target ≤ 90 ns/op (best-effort flush)
//   3. bench_get_component_typed         — target ≤ 16 ns/op
//   4. bench_has_entity                  — target ≤  5 ns/op
//   5. bench_set_component_raw           — target ≤ 18 ns/op
//   6. bench_iter_entities_dense_10k     — parity (no regression)
//   7. bench_iter_entities_sparse_churn  — documented baseline
//   8. bench_create_entity_10k           — within 5 % regression
//   9. bench_get_component_stale_gen     — target ≤  8 ns/op
//  10. bench_get_component_missing_comp  — target ≤ 10 ns/op
//  11. bench_create_1000_archetypes      — completes (no stack overflow)
//
// Component IDs 600-619 are reserved for this bench to avoid collisions with
// existing test/bench code in the global `OnceLock<ComponentLayout>` registry.
//   100-109: ecs_master tests
//   200-209: query tests
//   300-309: archetype_master tests
//   400-417: archetype unit tests + C-16 tests
//   420-435: archetype bench
//   450-465: drop_fn integration tests
//   470-479: query_iter bench
//   480-489: swap_remove bench
//   490-509: random_access bench (this file).
//   103-148 (less 128): bench #11 archetype-signature domain (this file).
//
// Note: `MAX_COMPONENTS = 512` is a hard cap; bench #11 fits within it by
// representing 1000 distinct archetype signatures as singleton + pair
// subsets over a 45-component domain (45 + C(45,2) = 1035 >= 1000).
//
// Each bench builds a deterministic scenario in `b.iter_batched(...)` or in
// the closure outer-scope; we lean on `black_box` on both inputs and outputs
// to keep the compiler from constant-folding the work away.

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
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};

// --- Component layout for the bench scenarios ---

const POS_ID: ComponentId = ComponentId(490);
const VEL_ID: ComponentId = ComponentId(491);

/// 45 distinct synthetic component IDs for bench #11: slots 103..=148 minus
/// 128 (taken by archetype_registry tests). 45 IDs yield 45 + C(45,2) = 1035
/// distinct signatures of size <= 2 — enough for the full 1000-archetype
/// plan target while staying inside the global MAX_COMPONENTS = 512 cap.
fn arch_domain_ids() -> Vec<ComponentId> {
    (103..=148).filter(|&v| v != 128).map(ComponentId).collect()
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Velocity {
    vx: f32,
    vy: f32,
    vz: f32,
}

// Manual Component impls so `EcsMaster::get_component::<Position>` works.
impl Component for Position {
    fn component_id() -> ComponentId {
        POS_ID
    }
}

impl Component for Velocity {
    fn component_id() -> ComponentId {
        VEL_ID
    }
}

fn register_bench_components() {
    component_registry::register_layout::<Position>(POS_ID.0);
    component_registry::register_layout::<Velocity>(VEL_ID.0);
}

// --- Scenario builders ---

/// Build an `EcsMaster` with `n` entities, each in the [Pos, Vel] archetype.
/// Returns the master and a list of entity handles in registration order.
fn build_dense_ecs(n: usize) -> (EcsMaster, ArchetypeId, Vec<Entity>) {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[POS_ID, VEL_ID]);

    let mut entities = Vec::with_capacity(n);
    for i in 0..n {
        let pos = Position {
            x: i as f32,
            y: 0.0,
            z: 0.0,
        };
        let vel = Velocity {
            vx: 1.0,
            vy: 0.0,
            vz: 0.0,
        };
        // SAFETY: Position/Velocity are #[repr(C)] POD; slices cover exactly
        // size_of::<T>() initialised bytes from each stack local.
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(
                &pos as *const Position as *const u8,
                std::mem::size_of::<Position>(),
            )
        };
        let vel_bytes = unsafe {
            std::slice::from_raw_parts(
                &vel as *const Velocity as *const u8,
                std::mem::size_of::<Velocity>(),
            )
        };
        let e = ecs
            .create_entity(arch, &[(POS_ID, pos_bytes), (VEL_ID, vel_bytes)])
            .expect("create_entity in bench setup must succeed");
        entities.push(e);
    }
    (ecs, arch, entities)
}

/// Deterministic shuffle (LCG) — avoids pulling rand as a dev-dep dependency
/// quirk inside benches and keeps the access pattern fully reproducible.
fn shuffle_indices(n: usize) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..n).collect();
    // Fisher-Yates with an LCG so the bench is deterministic and reproducible.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in (1..n).rev() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = (state >> 33) as usize % (i + 1);
        indices.swap(i, j);
    }
    indices
}

// --- D10 #1: bench_get_component_raw_hot ---
//
// 10K entities in one archetype, shuffled random index. Hot D-cache: the
// inland + column arrays stay resident across iterations because the working
// set is small (10K × 16B inland + ~64B archetype slot = ~160 KB; the loop
// touches them repeatedly).
//
// Target: ≤ 16 ns/op.
fn bench_get_component_raw_hot(c: &mut Criterion) {
    register_bench_components();
    let (ecs, _arch, entities) = build_dense_ecs(10_000);
    let shuffled: Vec<Entity> = shuffle_indices(entities.len())
        .into_iter()
        .map(|i| entities[i])
        .collect();

    let mut cursor = 0usize;
    c.bench_function("get_component_raw_hot", |b| {
        b.iter(|| {
            let e = shuffled[cursor];
            cursor = (cursor + 1) % shuffled.len();
            black_box(ecs.get_component_raw(black_box(e), POS_ID))
        });
    });
}

// --- D10 #2: bench_get_component_raw_cold ---
//
// Same setup as #1, but each lookup is preceded by a "dummy walk" that
// dirties the L1/L2 cache so subsequent loads must hit RAM. We don't use
// `clflush` (architecture-specific intrinsic, gated behind nightly +
// platform-specific guards); instead we walk a 32 MB buffer between
// iterations. This is documented "best-effort cold" — actual numbers
// reflect L3 / DRAM access cost, which is the right order of magnitude
// for the 90 ns target.
//
// Target: ≤ 90 ns/op.
fn bench_get_component_raw_cold(c: &mut Criterion) {
    register_bench_components();
    let (ecs, _arch, entities) = build_dense_ecs(10_000);
    let shuffled: Vec<Entity> = shuffle_indices(entities.len())
        .into_iter()
        .map(|i| entities[i])
        .collect();

    // 32 MB dummy buffer — larger than typical L2 (~256-512 KB), exceeds
    // L3 on most consumer Zen3/Alder Lake parts when fully walked, forcing
    // cold reads on the next ECS access.
    let cache_thrash: Vec<u8> = vec![0u8; 32 * 1024 * 1024];

    let mut cursor = 0usize;
    c.bench_function("get_component_raw_cold", |b| {
        b.iter(|| {
            // Best-effort cold: walk the dummy buffer to evict ECS data
            // from L1/L2/L3.
            let mut acc: u64 = 0;
            for chunk in cache_thrash.chunks(64) {
                acc = acc.wrapping_add(chunk[0] as u64);
            }
            black_box(acc);

            let e = shuffled[cursor];
            cursor = (cursor + 1) % shuffled.len();
            black_box(ecs.get_component_raw(black_box(e), POS_ID))
        });
    });
}

// --- D10 #3: bench_get_component_typed ---
//
// Exercises `ecs.get_component::<Position>(entity)`. Same hot-cache scenario
// as #1; this measures the typed wrapper overhead (component_id() lookup +
// pointer cast).
//
// Target: ≤ 16 ns/op (parity with raw).
fn bench_get_component_typed(c: &mut Criterion) {
    register_bench_components();
    let (ecs, _arch, entities) = build_dense_ecs(10_000);
    let shuffled: Vec<Entity> = shuffle_indices(entities.len())
        .into_iter()
        .map(|i| entities[i])
        .collect();

    let mut cursor = 0usize;
    c.bench_function("get_component_typed", |b| {
        b.iter(|| {
            let e = shuffled[cursor];
            cursor = (cursor + 1) % shuffled.len();
            black_box(ecs.get_component::<Position>(black_box(e)))
        });
    });
}

// --- D10 #4: bench_has_entity ---
//
// Single-line lookup: `entities_inland[entity.id().0]` + null/generation
// check. Hot cache, random shuffled index.
//
// Target: ≤ 5 ns/op.
fn bench_has_entity(c: &mut Criterion) {
    register_bench_components();
    let (ecs, _arch, entities) = build_dense_ecs(10_000);
    let shuffled: Vec<Entity> = shuffle_indices(entities.len())
        .into_iter()
        .map(|i| entities[i])
        .collect();

    let mut cursor = 0usize;
    c.bench_function("has_entity", |b| {
        b.iter(|| {
            let e = shuffled[cursor];
            cursor = (cursor + 1) % shuffled.len();
            black_box(ecs.has_entity(black_box(e)))
        });
    });
}

// --- D10 #5: bench_set_component_raw ---
//
// `ecs.set_component_raw(entity, comp_id, &bytes)`. Hot cache, random
// shuffled index. Re-uses the same `Position` value for every iteration —
// the bench measures the write path, not the value computation.
//
// Target: ≤ 18 ns/op.
fn bench_set_component_raw(c: &mut Criterion) {
    register_bench_components();
    let (mut ecs, _arch, entities) = build_dense_ecs(10_000);
    let shuffled: Vec<Entity> = shuffle_indices(entities.len())
        .into_iter()
        .map(|i| entities[i])
        .collect();

    let pos = Position {
        x: 42.0,
        y: 17.0,
        z: 99.0,
    };
    // SAFETY: Position is #[repr(C)] POD; the slice covers exactly its size.
    let pos_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            &pos as *const Position as *const u8,
            std::mem::size_of::<Position>(),
        )
    };

    let mut cursor = 0usize;
    c.bench_function("set_component_raw", |b| {
        b.iter(|| {
            let e = shuffled[cursor];
            cursor = (cursor + 1) % shuffled.len();
            black_box(ecs.set_component_raw(black_box(e), POS_ID, pos_bytes))
        });
    });
}

// --- D10 #6: bench_iter_entities_dense_10k ---
//
// Full sweep of 10K entities, no churn — the fast store is scanned
// sequentially; with no churn every slot is live, so iteration is a single
// contiguous pass. Parity bench: no regression vs. previous shape.
fn bench_iter_entities_dense_10k(c: &mut Criterion) {
    register_bench_components();
    let (ecs, _arch, _entities) = build_dense_ecs(10_000);

    c.bench_function("iter_entities_dense_10k", |b| {
        b.iter(|| {
            let mut count = 0usize;
            for e in ecs.iter_entities() {
                count = count.wrapping_add(black_box(e.id().0));
            }
            black_box(count)
        });
    });
}

// --- D10 #7: bench_iter_entities_sparse_post_churn ---
//
// Allocate 100K IDs (we scaled down from 1M to keep build time reasonable —
// 1M creates ~1.6 GB of column buffer with the current default arena, and
// repeated criterion warm-up runs blow heap), deallocate 99% so only 1K
// remain. Iterate the survivors. This stresses the iter_entities path on a
// sparse layout: post Phase-X.D the `active_ids` dense list no longer
// exists, so the scan is over `entities_inland` (mostly null sentinels
// after the churn). This bench is the documented O(capacity) baseline — no
// fail criterion.
fn bench_iter_entities_sparse_post_churn(c: &mut Criterion) {
    register_bench_components();

    // Setup is expensive; build once outside the iter loop. The bench reads,
    // never mutates, so reuse is safe.
    let total = 100_000usize;
    let survivors = 1_000usize;

    let (mut ecs, _arch, entities) = build_dense_ecs(total);

    // Deterministic deallocation: keep `entities[i]` iff i % (total/survivors)
    // == 0; remove the rest. Walks the list and nulls the corresponding
    // `entities_inland` slots inside EntityMaster, leaving null sentinels that
    // the post-churn O(capacity) `iter_entities` scan must skip.
    let keep_step = total / survivors;
    for (i, e) in entities.iter().enumerate() {
        if i % keep_step != 0 {
            let _ = ecs.delete_entity(*e);
        }
    }
    // Sanity: roughly `survivors` remain. The exact count depends on the
    // step's divisibility; we don't assert on it.
    let after = ecs.entity_count();

    c.bench_function("iter_entities_sparse_post_churn", |b| {
        b.iter(|| {
            let mut count = 0usize;
            for e in ecs.iter_entities() {
                count = count.wrapping_add(black_box(e.id().0));
            }
            black_box((count, after))
        });
    });
}

// --- X.D: bench_delete_entity_10k (transient despawn A/B) ---
//
// Times ONLY a `delete_entity` loop over N pre-spawned entities. `iter_batched`
// rebuilds a fresh dense population in `setup` so each measured pass deletes a
// freshly-populated ECS (delete is destructive — no in-place reset). Phase X.D
// shed per-despawn array touches (the `active_ids` swap-remove + the
// `sparse_to_active` fixup) and a branch; this bench isolates that saving from
// the create path. Setup cost is excluded from the timing by `iter_batched`.
fn bench_delete_entity_10k(c: &mut Criterion) {
    register_bench_components();
    let n = 10_000usize;

    c.bench_function("delete_entity_10k", |b| {
        b.iter_batched(
            || build_dense_ecs(n),
            |(mut ecs, _arch, entities)| {
                for e in &entities {
                    black_box(ecs.delete_entity(black_box(*e)));
                }
                black_box(ecs);
            },
            BatchSize::LargeInput,
        );
    });
}

// --- D10 #8: bench_create_entity_10k ---
//
// Full `create_entity` loop 10K times. iter_batched setup so each measured
// pass starts from a fresh ECS — there is no in-place "reset" of a populated
// ECS without dropping it.
//
// Target: ≤ 5 % regression vs. baseline (compare against
// `archetype_create_entity_8c` from the `archetype` bench).
fn bench_create_entity_10k(c: &mut Criterion) {
    register_bench_components();
    let n = 10_000usize;

    let pos = Position {
        x: 1.0,
        y: 2.0,
        z: 3.0,
    };
    let vel = Velocity {
        vx: 0.5,
        vy: 0.0,
        vz: 0.0,
    };
    // SAFETY: same as build_dense_ecs.
    let pos_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            &pos as *const Position as *const u8,
            std::mem::size_of::<Position>(),
        )
    };
    let vel_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            &vel as *const Velocity as *const u8,
            std::mem::size_of::<Velocity>(),
        )
    };

    c.bench_function("create_entity_10k", |b| {
        b.iter_batched(
            || {
                let mut ecs = EcsMaster::new();
                let arch = ecs.create_archetype(&[POS_ID, VEL_ID]);
                (ecs, arch)
            },
            |(mut ecs, arch)| {
                for _ in 0..n {
                    let e = ecs
                        .create_entity(
                            arch,
                            &[(POS_ID, pos_bytes), (VEL_ID, vel_bytes)],
                        )
                        .expect("bench create_entity must succeed");
                    black_box(e);
                }
                black_box(ecs);
            },
            BatchSize::SmallInput,
        );
    });
}

// --- D10 #9: bench_get_component_stale_generation ---
//
// Look up a `Position` for entities whose generation has been bumped (entity
// was deallocated and the ID recycled into a new generation). The fast path
// should reject these in O(1) via the generation tag — no archetype deref.
//
// Target: ≤ 8 ns/op.
fn bench_get_component_stale_generation(c: &mut Criterion) {
    register_bench_components();
    let n = 1_000usize;
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[POS_ID, VEL_ID]);

    // Allocate, register, then deallocate so the IDs are recycled with
    // bumped generations. The original handles are now stale.
    let mut stale: Vec<Entity> = Vec::with_capacity(n);
    let pos = Position {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let vel = Velocity {
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
    };
    // SAFETY: same as build_dense_ecs.
    let pos_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            &pos as *const Position as *const u8,
            std::mem::size_of::<Position>(),
        )
    };
    let vel_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            &vel as *const Velocity as *const u8,
            std::mem::size_of::<Velocity>(),
        )
    };
    for _ in 0..n {
        let e = ecs
            .create_entity(arch, &[(POS_ID, pos_bytes), (VEL_ID, vel_bytes)])
            .expect("create_entity in stale-gen setup must succeed");
        stale.push(e);
        ecs.delete_entity(e);
    }
    // Now re-create entities so the IDs are recycled (each `e0` in `stale`
    // now has a stale generation relative to the slot's current generation).
    for _ in 0..n {
        ecs.create_entity(arch, &[(POS_ID, pos_bytes), (VEL_ID, vel_bytes)])
            .expect("recycle pass must succeed");
    }

    let shuffled: Vec<Entity> = shuffle_indices(stale.len())
        .into_iter()
        .map(|i| stale[i])
        .collect();

    let mut cursor = 0usize;
    c.bench_function("get_component_stale_generation", |b| {
        b.iter(|| {
            let e = shuffled[cursor];
            cursor = (cursor + 1) % shuffled.len();
            // Expected: None (generation mismatch). Time the rejection cost.
            let r = ecs.get_component_raw(black_box(e), POS_ID);
            black_box(r)
        });
    });
}

// --- D10 #10: bench_get_component_missing_component ---
//
// Look up a `ComponentId` (`Velocity`) on an archetype that hosts only
// `Position`. The fast path resolves the inland, derefs the archetype
// slab, and finds `columns[VEL_ID.0].ptr.is_null()` — rejected without
// touching pool memory.
//
// Target: ≤ 10 ns/op.
fn bench_get_component_missing_component(c: &mut Criterion) {
    register_bench_components();
    let n = 10_000usize;
    let mut ecs = EcsMaster::new();
    // Archetype hosts ONLY Position — querying Velocity must return None.
    let arch_pos_only = ecs.create_archetype(&[POS_ID]);

    let pos = Position {
        x: 1.0,
        y: 2.0,
        z: 3.0,
    };
    // SAFETY: same as build_dense_ecs.
    let pos_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            &pos as *const Position as *const u8,
            std::mem::size_of::<Position>(),
        )
    };
    let mut entities: Vec<Entity> = Vec::with_capacity(n);
    for _ in 0..n {
        let e = ecs
            .create_entity(arch_pos_only, &[(POS_ID, pos_bytes)])
            .expect("create in missing-comp setup must succeed");
        entities.push(e);
    }
    let shuffled: Vec<Entity> = shuffle_indices(entities.len())
        .into_iter()
        .map(|i| entities[i])
        .collect();

    let mut cursor = 0usize;
    c.bench_function("get_component_missing_component", |b| {
        b.iter(|| {
            let e = shuffled[cursor];
            cursor = (cursor + 1) % shuffled.len();
            // Query Velocity on Position-only archetype: expect None.
            let r = ecs.get_component_raw(black_box(e), VEL_ID);
            black_box(r)
        });
    });
}

// --- D10 #11: bench_create_1000_archetypes_no_stack_overflow ---
//
// Register many distinct archetypes back-to-back. Each `Archetype` contains
// the 8 KB inline `columns` table — the Phase 7 W6 fix guarantees this is
// constructed in-place inside the slab, never on the stack. If W6 regresses,
// this bench overflows the stack on call frame 1.
//
// # Scaling — restored to the plan's full 1000 archetypes (Phase X.F)
//
// The pre-X.F version trimmed this to 200 (in practice 55) archetypes
// because each archetype allocates at least one ComponentPool (~256 KB
// chunked buffer for u8 layouts under `with_default_sizes`) and the old
// 64 MB fixed arena saturated at ~200 single-component archetypes. Phase
// X.F arena growth (lazy slab commit inside a multi-GB reservation) removed
// that ceiling, so the bench now runs the original plan target: 1000
// distinct signatures (45 singletons + 955 pairs over the 45-id domain),
// committing ~0.5 GB of pools per constructed world — bench-only weight.
//
// Target: completes without stack overflow. Numbers are reported for
// reference, not as a target.
fn bench_create_1000_archetypes_no_stack_overflow(c: &mut Criterion) {
    register_bench_components();
    let domain = arch_domain_ids();
    // Register the 45 bench domain components once (idempotent).
    for id in &domain {
        component_registry::register_layout::<u8>(id.0);
    }

    // Pre-build distinct signatures up-front so the measured loop only does
    // archetype-registration work, not subset arithmetic: all 45 singletons,
    // then pairs in lexicographic order until 1000 signatures total.
    const N_ARCHETYPES: usize = 1000;
    let mut signatures: Vec<Vec<ComponentId>> = Vec::with_capacity(N_ARCHETYPES);
    for &id in &domain {
        signatures.push(vec![id]);
    }
    'pairs: for i in 0..domain.len() {
        for j in (i + 1)..domain.len() {
            if signatures.len() == N_ARCHETYPES {
                break 'pairs;
            }
            signatures.push(vec![domain[i], domain[j]]);
        }
    }
    assert_eq!(signatures.len(), N_ARCHETYPES, "domain must yield 1000 signatures");

    c.bench_function("create_1000_archetypes_no_stack_overflow", |b| {
        b.iter_batched(
            EcsMaster::new,
            |mut ecs| {
                for sig in &signatures {
                    let id = ecs.create_archetype(sig);
                    black_box(id);
                }
                black_box(&ecs);
                // Return the world so its ~0.5 GB of committed pools are
                // dropped by criterion outside the timed region.
                ecs
            },
            // PerIteration: each constructed world commits ~0.5 GB of pools;
            // LargeInput would keep a whole batch of them alive at once
            // (the X.C arena_new lesson, in miniature).
            BatchSize::PerIteration,
        );
    });
}

criterion_group!(
    random_access,
    bench_get_component_raw_hot,
    bench_get_component_raw_cold,
    bench_get_component_typed,
    bench_has_entity,
    bench_set_component_raw,
    bench_iter_entities_dense_10k,
    bench_iter_entities_sparse_post_churn,
    bench_delete_entity_10k,
    bench_create_entity_10k,
    bench_get_component_stale_generation,
    bench_get_component_missing_component,
    bench_create_1000_archetypes_no_stack_overflow,
);
criterion_main!(random_access);
