//! Phase 14a — the 0%-REGRESSION bench gate (the load-bearing acceptance
//! criterion).
//!
//! The whole feature's premise: when NO hooks are registered, the structural-op
//! hot path costs one extra `u16` `ArchetypeFlags` load + a `test`/`jz`
//! (predicted not-taken) at each fire site, and a cold helper that is never
//! entered — i.e. ZERO measurable regression (the Phase 10 "0% when unused"
//! mechanism applied to structural ops).
//!
//! Every bench below uses components with NO `#[component(...)]` attribute and
//! NEVER calls `register_component_hooks`, so the per-archetype flags are empty
//! and every fire site takes the not-taken branch. Compare these numbers to the
//! committed pre-14a baselines:
//!
//! | bench                       | historical baseline (per MEMORY)              |
//! |-----------------------------|-----------------------------------------------|
//! | `gate_spawn_batch_10k_1comp`| ~308 µs (Phase 8.5 batch_10k) / 35 ns·e warm  |
//! | `gate_query_iter_10k`       | warm QueryState walk ~3.6 ns/iter (Q-011)     |
//! | `gate_commands_spawn_single`| ±20-30% variance noted (Phase 12.6 g4)        |
//! | `gate_despawn_10k`          | swap_remove.rs delete_entity baseline         |
//! | `gate_create_entity_single`| P4 — grew a #[cold] helper; confirm flat      |
//! | `gate_delete_entity_single` | P4 — grew a #[cold] helper; confirm flat      |
//!
//! Acceptance: within criterion noise (±5%), mirroring Phase 10's "0% when
//! unused". Single-spawn benches carry ±20-30% variance (Phase 12.6) — note
//! that when reading the deltas; rely on the CLEAN-signal benches (spawn_batch,
//! query_iter) for the verdict.
//!
//! Invoked by the `tester`: `cargo bench -p boyko-ecs --bench phase14a_hooks_gate`.

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::query_view::QueryView;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_macros::Bundle;

// No-hook component slots, disjoint from the gate's hooked tests + every other
// bench (phase12_5 uses 363-365; we use 366-368).
const SLOT_POS: ComponentId = ComponentId(366);
const SLOT_VEL: ComponentId = ComponentId(367);
const SLOT_HEALTH: ComponentId = ComponentId(368);

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
    x: f32,
    y: f32,
    z: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Health(i32);

// Hand-written `impl Component` (NO `#[component(...)]` ⇒ HAS_HOOKS = false,
// register_hooks empty). These never install a HOOKS-table entry, so every
// archetype's ArchetypeFlags stay empty — the no-hook hot path.
impl Component for Position {
    fn component_id() -> ComponentId {
        SLOT_POS
    }
}
impl Component for Velocity {
    fn component_id() -> ComponentId {
        SLOT_VEL
    }
}
impl Component for Health {
    fn component_id() -> ComponentId {
        SLOT_HEALTH
    }
}

fn register() {
    register_layout::<Position>(SLOT_POS.0);
    register_layout::<Velocity>(SLOT_VEL.0);
    register_layout::<Health>(SLOT_HEALTH.0);
}

#[derive(Bundle)]
struct PosBundle {
    pos: Position,
}
#[derive(Bundle)]
struct PosVelHealth {
    pos: Position,
    vel: Velocity,
    health: Health,
}

// ── gate_spawn_batch_10k_1comp (CLEAN signal — flag-gate at SpawnAt site) ────

fn gate_spawn_batch_10k_1comp(c: &mut Criterion) {
    register();
    c.bench_function("gate_spawn_batch_10k_1comp", |b| {
        b.iter_with_setup(EcsMaster::new, |mut ecs| {
            ecs.run_system(|mut cmds: Commands| {
                for chunk in 0..2 {
                    let base = chunk * 5_000;
                    let _ = cmds
                        .spawn_batch((0..5_000).map(move |i| PosBundle {
                            pos: Position { x: (base + i) as f32, y: 0.0, z: 0.0 },
                        }))
                        .expect("5000 <= MAX_BATCH_HINT");
                }
            });
            black_box(ecs.entity_count());
        });
    });
}

fn gate_spawn_batch_10k_3comp(c: &mut Criterion) {
    register();
    c.bench_function("gate_spawn_batch_10k_3comp", |b| {
        b.iter_with_setup(EcsMaster::new, |mut ecs| {
            ecs.run_system(|mut cmds: Commands| {
                for chunk in 0..2 {
                    let base = chunk * 5_000;
                    let _ = cmds
                        .spawn_batch((0..5_000).map(move |i| PosVelHealth {
                            pos: Position { x: (base + i) as f32, y: 0.0, z: 0.0 },
                            vel: Velocity { x: 0.0, y: (base + i) as f32, z: 0.0 },
                            health: Health(base + i),
                        }))
                        .expect("5000 <= MAX_BATCH_HINT");
                }
            });
            black_box(ecs.entity_count());
        });
    });
}

// ── gate_query_iter_10k (CLEAN signal — change detection / iter untouched) ───

fn gate_query_iter_10k(c: &mut Criterion) {
    register();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[SLOT_POS]);
    for i in 0..10_000u32 {
        ecs.spawn_one(arch, Position { x: i as f32, y: 0.0, z: 0.0 })
            .expect("seed");
    }
    c.bench_function("gate_query_iter_10k", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            let mut q: QueryView<'_, &Position, ()> = ecs.query::<&Position, ()>();
            q.for_each_chunk(|slice: &[Position]| {
                for p in slice {
                    sum += black_box(p.x);
                }
            });
            black_box(sum);
        });
    });
}

// ── gate_commands_spawn_single (NOISY — ±20-30%, Phase 12.6) ─────────────────

fn gate_commands_spawn_single(c: &mut Criterion) {
    register();
    c.bench_function("gate_commands_spawn_single", |b| {
        b.iter_with_setup(EcsMaster::new, |mut ecs| {
            ecs.run_system(|mut cmds: Commands| {
                cmds.spawn(PosBundle { pos: Position { x: 1.0, y: 2.0, z: 3.0 } });
            });
            black_box(ecs.entity_count());
        });
    });
}

// ── gate_create_entity_single (P4 — delete/create grew a #[cold] frame) ──────

fn gate_create_entity_single(c: &mut Criterion) {
    register();
    c.bench_function("gate_create_entity_single", |b| {
        b.iter_with_setup(
            || {
                let mut ecs = EcsMaster::new();
                let arch = ecs.create_archetype(&[SLOT_POS]);
                (ecs, arch)
            },
            |(mut ecs, arch): (EcsMaster, ArchetypeId)| {
                let e = ecs
                    .spawn_one(arch, Position { x: 1.0, y: 2.0, z: 3.0 })
                    .expect("spawn");
                black_box(e);
            },
        );
    });
}

// ── gate_delete_entity_single (P4 — fire_despawn_hooks #[cold] helper) ───────

fn gate_delete_entity_single(c: &mut Criterion) {
    register();
    c.bench_function("gate_delete_entity_single", |b| {
        b.iter_with_setup(
            || {
                let mut ecs = EcsMaster::new();
                let arch = ecs.create_archetype(&[SLOT_POS]);
                let e = ecs
                    .spawn_one(arch, Position { x: 1.0, y: 2.0, z: 3.0 })
                    .expect("spawn");
                (ecs, e)
            },
            |(mut ecs, e)| {
                black_box(ecs.delete_entity(e));
            },
        );
    });
}

// ── gate_despawn_10k (delete_entity in a loop — swap_remove baseline) ────────

fn gate_despawn_10k(c: &mut Criterion) {
    register();
    c.bench_function("gate_despawn_10k", |b| {
        b.iter_with_setup(
            || {
                let mut ecs = EcsMaster::new();
                let arch = ecs.create_archetype(&[SLOT_POS]);
                let mut ents = Vec::with_capacity(10_000);
                for i in 0..10_000u32 {
                    ents.push(
                        ecs.spawn_one(arch, Position { x: i as f32, y: 0.0, z: 0.0 })
                            .expect("seed"),
                    );
                }
                (ecs, ents)
            },
            |(mut ecs, ents): (EcsMaster, Vec<_>)| {
                for e in ents {
                    black_box(ecs.delete_entity(e));
                }
                black_box(ecs.entity_count());
            },
        );
    });
}

criterion_group!(
    benches,
    gate_spawn_batch_10k_1comp,
    gate_spawn_batch_10k_3comp,
    gate_query_iter_10k,
    gate_commands_spawn_single,
    gate_create_entity_single,
    gate_delete_entity_single,
    gate_despawn_10k,
);
criterion_main!(benches);
