//! Feature 1 (required components) — the 0%-gate (sacred).
//!
//! Spec `docs/REQUIRED-COMPONENTS-PLAN.md` §"0%-gate": a `#[derive(Component)]`
//! with NO `#[require]`, spawned/inserted, must be byte-identical to the
//! pre-feature path. The require-free constructor pass is
//! `if const { B::HAS_REQUIRES }`-gated (const-folds away), the archetype-
//! expansion union loop runs zero inner iterations, and the apply-time
//! `required_missing` slice is empty (an empty-slice check).
//!
//! This bench drives the require-free spawn AND insert paths so the tester can
//! run a SAME-BINARY A/B (`--save-baseline` then `--baseline`, no source change
//! between) to defeat the box's ±13% cross-commit drift. A >~5% regression on
//! the require-free path is a FAIL to escalate. The review flagged two 0%-gate
//! risk points now fixed (the `any_requires` early-out on require-free insert;
//! the Box-not-stack 4 KiB scratch) — these benches exercise exactly that path.
//!
//! Component slots: 380..=382 — disjoint from phase 12.5 (360-365), phase 11
//! (411-413), below MAX_COMPONENTS = 512.

#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

const SLOT_POS: ComponentId = ComponentId(380);
const SLOT_VEL: ComponentId = ComponentId(381);
const SLOT_ANCHOR: ComponentId = ComponentId(382);

#[repr(C)]
#[derive(Clone, Copy)]
struct GPos {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GVel {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GAnchor(u32);

impl Component for GPos {
    fn component_id() -> ComponentId {
        SLOT_POS
    }
}
impl Component for GVel {
    fn component_id() -> ComponentId {
        SLOT_VEL
    }
}
impl Component for GAnchor {
    fn component_id() -> ComponentId {
        SLOT_ANCHOR
    }
}

fn register() {
    register_layout::<GPos>(SLOT_POS.0);
    register_layout::<GVel>(SLOT_VEL.0);
    register_layout::<GAnchor>(SLOT_ANCHOR.0);
}

#[derive(Bundle)]
struct GPosVel {
    pos: GPos,
    vel: GVel,
}

#[derive(Bundle)]
struct GVelBundle {
    vel: GVel,
}

/// 0%-gate A: deferred spawn + apply of a require-free 2-component bundle.
/// This is the warm spawn path the require-free fast-path must not regress.
fn bench_spawn_no_require(c: &mut Criterion) {
    register();
    c.bench_function("gate_spawn_no_require/spawn_apply_2comp", |b| {
        b.iter(|| {
            let mut ecs = EcsMaster::new();
            ecs.run_system(|mut cmds: Commands| {
                for i in 0..256u32 {
                    cmds.spawn(GPosVel {
                        pos: GPos {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                        vel: GVel {
                            x: 0.0,
                            y: i as f32,
                            z: 0.0,
                        },
                    });
                }
            });
            black_box(ecs.entity_count());
        });
    });
}

/// 0%-gate B: the require-free INSERT/migration path. Spawn an Anchor-only
/// entity, then insert a require-free Vel bundle ⇒ migrate {Anchor}→{Anchor,Vel}.
/// This is where the `any_requires` early-out (review fix) must show no cost.
fn bench_insert_no_require(c: &mut Criterion) {
    register();
    c.bench_function("gate_spawn_no_require/insert_migrate_no_require", |b| {
        b.iter(|| {
            let mut ecs = EcsMaster::new();
            let arch = ecs.create_archetype(&[GAnchor::component_id()]);
            let mut ents = Vec::with_capacity(256);
            for i in 0..256u32 {
                ents.push(ecs.spawn_one(arch, GAnchor(i)).expect("spawn anchor"));
            }
            ecs.run_system(move |mut cmds: Commands| {
                for e in &ents {
                    cmds.entity(*e).insert(GVelBundle {
                        vel: GVel {
                            x: 1.0,
                            y: 2.0,
                            z: 3.0,
                        },
                    });
                }
            });
            black_box(ecs.entity_count());
        });
    });
}

criterion_group!(benches, bench_spawn_no_require, bench_insert_no_require);
criterion_main!(benches);
