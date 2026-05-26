//! Phase 8.5 Step 10 — Criterion bench suite for the Static Bundle Cache.
//!
//! Targets per plan §9 Step 10:
//!
//! | Bench                              | Target  |
//! |------------------------------------|---------|
//! | `component_ids_cached_lookup`      | ≤ 2 ns  |
//! | `cached_archetype_id_cached_lookup`| ≤ 3 ns  |
//! | `commands_spawn_enqueue`           | ≤ 18 ns |
//! | `spawn_command_apply_arity_4`      | ≤ 200 ns|
//! | `batch_10k_spawn_apply`            | ≤ 1.2 ms|
//!
//! The batch bench's pre-Phase-8.5 baseline (Phase 8d) sat around 3 ms
//! end-to-end; the static cache should produce ≥ 2× speedup per plan
//! acceptance §9 Step 10.
//!
//! All benches use `criterion::black_box` on inputs / outputs to defeat
//! constant-folding. Per-call hot-path numbers warm the OnceLock and the
//! per-world cache slot ONCE outside the timed loop, so the measured
//! cost is the steady-state (cached) cost — that's the contract Phase 9
//! and downstream gameplay loops will read against.
//!
//! No `#[inline(always)]` per CLAUDE.md principle #7 — Criterion's
//! framework handles dispatch overhead.
//!
//! # Component-slot range
//!
//! 340..=360 per the Step 10 bench spec. Disjoint from Phase 8c+8d
//! (244..=259, 280..=281), Phase 8d Miri (260..=269), Phase 8.5 Step 7
//! smoke (290..=309), Step 8 panic (310..=312), Step 8 Miri (320..=339).

use std::hint::black_box;

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

// ── Component types ─────────────────────────────────────────────────────────

const SLOT_BSC_A: ComponentId = ComponentId(340);
const SLOT_BSC_B: ComponentId = ComponentId(341);
const SLOT_BSC_C: ComponentId = ComponentId(342);
const SLOT_BSC_D: ComponentId = ComponentId(343);

#[repr(C)]
#[derive(Clone, Copy)]
struct BscA(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct BscB(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct BscC(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct BscD(u32);

impl Component for BscA {
    fn component_id() -> ComponentId {
        SLOT_BSC_A
    }
}
impl Component for BscB {
    fn component_id() -> ComponentId {
        SLOT_BSC_B
    }
}
impl Component for BscC {
    fn component_id() -> ComponentId {
        SLOT_BSC_C
    }
}
impl Component for BscD {
    fn component_id() -> ComponentId {
        SLOT_BSC_D
    }
}

fn register_bsc() {
    register_layout::<BscA>(SLOT_BSC_A.0);
    register_layout::<BscB>(SLOT_BSC_B.0);
    register_layout::<BscC>(SLOT_BSC_C.0);
    register_layout::<BscD>(SLOT_BSC_D.0);
}

// ── Bundle types — declared at module scope so per-impl OnceLocks are
//                    stable across iter_batched batches ────────────────────

#[derive(Bundle)]
struct BscBundle4 {
    a: BscA,
    b: BscB,
    c: BscC,
    d: BscD,
}

#[derive(Bundle)]
struct BscBundle1 {
    a: BscA,
}

// =============================================================================
// 1. component_ids_cached_lookup (≤ 2 ns)
// =============================================================================
//
// Warm the `OnceLock<BundleStaticInfo>` outside the loop with a single
// `component_ids()` call; the timed loop measures the steady-state Acquire
// load on the cached slot. The black_box on the returned slice prevents
// the optimizer from CSE-ing the call across iterations.

fn bench_component_ids_cached_lookup(c: &mut Criterion) {
    register_bsc();
    // Warm-up — first call populates the per-impl OnceLock.
    let _ = BscBundle4::component_ids();

    c.bench_function("component_ids_cached_lookup", |b| {
        b.iter(|| {
            let ids = BscBundle4::component_ids();
            black_box(ids);
        });
    });
}

// =============================================================================
// 2. cached_archetype_id_cached_lookup (≤ 3 ns)
// =============================================================================
//
// Two-tier cache hit: `B::bundle_type_id()` (OnceLock Acquire) +
// `world.bundle_archetype_cache[id.0].get()` (Acquire on the boxed-array
// slot). Both slots are warmed by a single call outside the loop.

fn bench_cached_archetype_id_cached_lookup(c: &mut Criterion) {
    register_bsc();
    let mut ecs = EcsMaster::new();
    // Warm-up — populates both the per-impl OnceLock and the per-world
    // cache slot.
    let _ = BscBundle4::cached_archetype_id(&mut ecs);

    c.bench_function("cached_archetype_id_cached_lookup", |b| {
        b.iter(|| {
            let id = BscBundle4::cached_archetype_id(&mut ecs);
            black_box(id);
        });
    });
}

// =============================================================================
// 3. commands_spawn_enqueue (≤ 18 ns per spawn)
// =============================================================================
//
// `Commands::spawn(bundle)` → `CommandQueue::push(SpawnCommand { bundle })`.
// At enqueue time the cache is NOT touched — `B::cached_archetype_id` is
// resolved later inside `SpawnCommand::apply`. So the per-spawn cost is
// dominated by the queue's two `write_unaligned` calls + amortised Vec
// growth.
//
// The production `Commands::spawn` surface lives inside the SystemParam
// machinery; the only way to exercise it from outside a system is via
// `run_system(|cmds: Commands| ...)`. That trampoline brings ~25 ns of
// FunctionSystem dispatch overhead AND the apply step at the end.
//
// To isolate the per-spawn enqueue cost, we measure `N = 1024` spawns
// per outer iter and divide. The fixed per-call cost (dispatch + apply
// of 1024 entities) is amortised: dispatch ≈ 25 ns / 1024 ≈ 0.025
// ns/spawn (negligible), while the apply itself processes the queue at
// roughly the same per-entity cost as the cached_archetype_id +
// for_each + create_entity hot path — see bench #4 for that figure.
// The PURE enqueue cost is therefore approximated by:
//
//   per_spawn ≈ (per_iter - per_apply * N) / N
//
// where `per_apply` is the figure from bench #4 (≤ 200 ns). For the
// target check, we report the raw per-iter / N number and note the
// enqueue-vs-apply split in the report.
//
// Each outer iter resets the world via `EcsMaster::clear()` so the entity
// count stays bounded.

fn bench_commands_spawn_enqueue(c: &mut Criterion) {
    register_bsc();
    // 1024 spawns per iter amortises the arena setup + drop cost into a
    // small fixed addend. Per-spawn cost is reported via the iter total
    // divided by 1024 (see the run report).
    c.bench_function("commands_spawn_enqueue_x1024", |b| {
        b.iter_batched(
            || {
                let mut ecs = EcsMaster::new();
                let _ = BscBundle4::cached_archetype_id(&mut ecs);
                ecs
            },
            |mut ecs| {
                ecs.run_system(|mut cmds: Commands| {
                    for i in 0..1024u32 {
                        cmds.spawn(BscBundle4 {
                            a: BscA(i),
                            b: BscB(i),
                            c: BscC(i),
                            d: BscD(i),
                        });
                    }
                });
                black_box(ecs);
            },
            BatchSize::LargeInput,
        );
    });
}

// =============================================================================
// 4. spawn_command_apply_arity_4 (≤ 200 ns)
// =============================================================================
//
// Single arity-4 SpawnCommand from enqueue through apply. Per-iter cost:
// push (≤ 18 ns) + apply (cached_archetype_id ≤ 3 ns + for_each callback
// chain + create_entity memcpy of 4 components).
//
// `iter_batched` builds a fresh world per batch so entity_count doesn't
// grow unbounded across the sample loop.

fn bench_spawn_command_apply_arity_4(c: &mut Criterion) {
    register_bsc();
    // Per-iter EcsMaster construction would dominate the measurement
    // (each `EcsMaster::new` allocates a 64 MB arena; per-iter drop pays
    // the arena unmap cost ~tens of µs). Hoisting the world is also a
    // trap — the per-archetype pool capacity is bounded by
    // `get_optimal_chunk_capacity` so a long sample run fills the pool
    // and `create_entity` errors out.
    //
    // Bench compromise: iter_batched + LargeInput so each timed iter
    // builds + drops one fresh world. The arena cost gets folded into
    // the per-iter cost, but the per-iter cost ALSO covers a clean
    // create_entity + create_archetype-warm path (the cache is warmed
    // inside the setup, so the timed body only pays the hot-path apply
    // + a single create_entity memcpy). Reported number includes the
    // arena unmap; the ≤ 200 ns plan target was set against a hoisted
    // world that is structurally impossible here. Report the raw figure
    // and note the breakdown in the bench report.
    c.bench_function("spawn_command_apply_arity_4", |b| {
        b.iter_batched(
            || {
                let mut ecs = EcsMaster::new();
                let _ = BscBundle4::cached_archetype_id(&mut ecs);
                ecs
            },
            |mut ecs| {
                ecs.run_system(|mut cmds: Commands| {
                    cmds.spawn(BscBundle4 {
                        a: BscA(1),
                        b: BscB(2),
                        c: BscC(3),
                        d: BscD(4),
                    });
                });
                black_box(ecs);
            },
            BatchSize::LargeInput,
        );
    });
}

// =============================================================================
// 5. batch_10k_spawn_apply (≤ 1.2 ms — Phase 8d baseline ~3 ms)
// =============================================================================
//
// Full 10 000-entity spawn cycle. Headline metric for the Phase 8.5
// rewrite. Pre-Phase-8.5 baseline (Phase 8d's per-callsite
// get_or_create_archetype + tuple bundle) was roughly 3 ms; Phase 8.5's
// static cache must hit ≤ 1.2 ms (plan §9 Step 10).
//
// `iter_batched` builds a fresh world per outer iteration (LargeInput so
// criterion budgets correctly for the heavy per-iter setup + work).
// Inside, one `run_system` call queues all 10 000 spawns; the apply
// drains them in a single CommandQueue::apply.

fn bench_batch_10k_spawn_apply(c: &mut Criterion) {
    register_bsc();
    c.bench_function("batch_10k_spawn_apply", |b| {
        b.iter_batched(
            || {
                let mut ecs = EcsMaster::with_capacity(16_384, 64);
                // Pre-warm the cache so we measure the steady-state
                // 10k cycle, not the cold init.
                let _ = BscBundle1::cached_archetype_id(&mut ecs);
                ecs
            },
            |mut ecs| {
                ecs.run_system(|mut cmds: Commands| {
                    for i in 0..10_000u32 {
                        cmds.spawn(BscBundle1 { a: BscA(i) });
                    }
                });
                black_box(ecs);
            },
            BatchSize::LargeInput,
        );
    });
}

criterion_group!(
    bundle_static_cache_benches,
    bench_component_ids_cached_lookup,
    bench_cached_archetype_id_cached_lookup,
    bench_commands_spawn_enqueue,
    bench_spawn_command_apply_arity_4,
    bench_batch_10k_spawn_apply,
);
criterion_main!(bundle_static_cache_benches);
