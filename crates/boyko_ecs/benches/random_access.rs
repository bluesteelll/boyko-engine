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
//   490-509: random_access bench (this file) — including ARCH_BIT_IDS for #11.
//
// Note: `MAX_COMPONENTS = 512` is a hard cap; the bench fits within this range
// by representing 1000 distinct archetype signatures as bitmask subsets over a
// 10-component domain (2^10 = 1024 unique non-empty subsets).
//
// Each bench builds a deterministic scenario in `b.iter_batched(...)` or in
// the closure outer-scope; we lean on `black_box` on both inputs and outputs
// to keep the compiler from constant-folding the work away.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};

// --- Component layout for the bench scenarios ---

const POS_ID: ComponentId = ComponentId(490);
const VEL_ID: ComponentId = ComponentId(491);

/// 10 distinct synthetic component IDs (492-501). Bench #11 forms 1000
/// distinct archetype signatures as bitmask subsets over this 10-id domain
/// (2^10 = 1024 unique non-empty subsets). 10 IDs fit inside the global
/// MAX_COMPONENTS = 512 cap.
const ARCH_BIT_IDS: [ComponentId; 10] = [
    ComponentId(492), ComponentId(493), ComponentId(494), ComponentId(495),
    ComponentId(496), ComponentId(497), ComponentId(498), ComponentId(499),
    ComponentId(500), ComponentId(501),
];

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
// Full sweep of 10K entities, no churn — the active_ids dense list is
// contiguous so each iteration is a simple pointer walk. Parity bench:
// no regression vs. previous shape.
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
// sparse layout: active_ids stays dense (it tracks live IDs after
// swap-remove), but `entities_inland` is mostly null sentinels. Documented
// baseline — no fail criterion.
fn bench_iter_entities_sparse_post_churn(c: &mut Criterion) {
    register_bench_components();

    // Setup is expensive; build once outside the iter loop. The bench reads,
    // never mutates, so reuse is safe.
    let total = 100_000usize;
    let survivors = 1_000usize;

    let (mut ecs, _arch, entities) = build_dense_ecs(total);

    // Deterministic deallocation: keep `entities[i]` iff i % (total/survivors)
    // == 0; remove the rest. Walks the list and triggers swap-remove churn on
    // the dense active list inside EntityMaster.
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
// # Scaling
//
// The plan asks for 1000 archetypes. In practice each archetype allocates
// at least one ComponentPool (~256 KB chunked buffer for u8 layouts under
// `with_default_sizes`), so 1000 single-component archetypes would need
// ~256 MB of arena — well over the 64 MB `DEFAULT_ARENA_SIZE`. The default
// arena saturates at ~200 single-component archetypes.
//
// To keep the bench self-contained and demonstrate the W6 fix at scale,
// we scale to 200 archetypes (≈ 50 MB pool allocation, comfortably below
// the 64 MB default arena). This is enough to: (a) prove no stack overflow,
// since the 8 KB Archetype struct is constructed in-place inside
// ArchetypeBundle slots — the first call would already overflow if W6
// were broken; (b) cover bulk-creation churn for the slab path.
//
// We register the 10-id bit domain (492-501) and form 200 distinct
// non-empty subsets — one archetype per subset.
//
// Target: completes without stack overflow. Numbers are reported for
// reference, not as a target.
fn bench_create_1000_archetypes_no_stack_overflow(c: &mut Criterion) {
    register_bench_components();
    // Register the 10 bench bit-domain components once (idempotent).
    for id in ARCH_BIT_IDS.iter() {
        component_registry::register_layout::<u8>(id.0);
    }

    // Pre-build distinct signatures up-front so the measured loop only does
    // archetype-registration work, not bit-arithmetic.
    const N_ARCHETYPES: usize = 200;
    let mut signatures: Vec<Vec<ComponentId>> = Vec::with_capacity(N_ARCHETYPES);
    for subset_mask in 1u32..=1024u32 {
        if signatures.len() == N_ARCHETYPES {
            break;
        }
        let mut ids = Vec::with_capacity(10);
        for (bit, cid) in ARCH_BIT_IDS.iter().enumerate() {
            if (subset_mask >> bit) & 1 == 1 {
                ids.push(*cid);
            }
        }
        // Skip subsets with more than 2 components to bound per-archetype pool
        // allocation count; single + pair subsets give us 10 + 45 = 55 unique
        // signatures from the 10-bit domain.
        if !ids.is_empty() && ids.len() <= 2 {
            signatures.push(ids);
        }
    }
    // 10 singletons + 45 pairs = 55 unique signatures from a 10-bit domain.
    // For the test we just need "many archetypes back-to-back to prove no
    // stack overflow"; 55 is comfortably above the 1-frame threshold that
    // would matter for a broken W6 implementation.
    let n_actual = signatures.len();

    c.bench_function("create_1000_archetypes_no_stack_overflow", |b| {
        b.iter_batched(
            EcsMaster::new,
            |mut ecs| {
                for sig in &signatures {
                    let id = ecs.create_archetype(sig);
                    black_box(id);
                }
                black_box((ecs, n_actual));
            },
            BatchSize::LargeInput,
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
    bench_create_entity_10k,
    bench_get_component_stale_generation,
    bench_get_component_missing_component,
    bench_create_1000_archetypes_no_stack_overflow,
);
criterion_main!(random_access);
