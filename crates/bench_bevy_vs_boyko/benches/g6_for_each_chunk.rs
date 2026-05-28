#![feature(float_algebraic)]

//! Phase X.A Wave 8B — bench harness for `Query::for_each_chunk`.
//!
//! Compares boyko's `Query::for_each_chunk` against Bevy 0.18's
//! `Query::iter().fold(_, f32::algebraic_add)` pattern on a single-archetype
//! 10k-entity f32-sum reduction.
//!
//! # Target
//!
//! Per plan §1.2 + §8.2: boyko median ≥ **5× Bevy median** over 60 criterion
//! samples. Bench PASS bar matches the §8.4 floor; expected combined gain is
//! 5-8× from slice-elision + `algebraic_add` autovec + 32-byte column-start
//! alignment (Phase X.A SIMD-A1).
//!
//! # Fairness
//!
//! Both engines reduce via `f32::algebraic_add` (nightly `float_algebraic`)
//! to eliminate the `black_box`-per-element optimisation barrier on both
//! sides. The harness therefore measures the API-shape delta (per-row state
//! machine vs typed slice), not a measurement artefact.
//!
//! # Toolchain
//!
//! Requires nightly Rust for `f32::algebraic_add`. The bench crate carries a
//! per-package `rust-toolchain.toml` override (Step 8A); workspace root stays
//! on stable. Invoke from inside `crates/bench_bevy_vs_boyko/` or with
//! `cargo +nightly bench -p bench-bevy-vs-boyko --bench g6_for_each_chunk`.
//!
//! Run via `cargo bench --bench g6_for_each_chunk`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

// boyko imports
use boyko_ecs::ecs::core::component::component::Component as BoykoComponent;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

// bevy imports
use bevy_ecs::prelude::Component as BevyComponentDerive;
use bevy_ecs::prelude::*;

const N_ENTITIES: usize = 10_000;

// ComponentId 490 reserved per Wave 7 allocation map (range 480-899 free).
// Distinct from the 350/351 slots used by `comparison.rs` / `comparison_v2.rs`
// to avoid the cross-bench shared-state hazard: registering the same ID with
// two different Layouts would panic on the second `register_layout` call.
const BOYKO_VEL_ID: ComponentId = ComponentId(490);

#[repr(transparent)]
#[derive(Clone, Copy)]
struct BoykoVelF32(f32);

impl BoykoComponent for BoykoVelF32 {
    fn component_id() -> ComponentId {
        BOYKO_VEL_ID
    }
}

fn register_boyko_velocity() {
    register_layout::<BoykoVelF32>(BOYKO_VEL_ID.0);
}

#[derive(BevyComponentDerive, Clone, Copy)]
#[repr(transparent)]
struct BevyVelF32(f32);

// ===========================================================================
// Reductions
// ===========================================================================
//
// Both reductions use `f32::algebraic_add` (nightly `float_algebraic`). The
// `black_box` ring-fences the final accumulator only, NOT each element — the
// inner-loop autovec is therefore unobstructed on both sides.

fn boyko_for_each_chunk_sum(world: &mut EcsMaster) -> f32 {
    let mut acc: f32 = 0.0;
    let mut query = world.query::<&BoykoVelF32, ()>();
    query.for_each_chunk(|slice: &[BoykoVelF32]| {
        // SoA inner reduction. `slice` is the full archetype column for
        // `BoykoVelF32` — 32-byte-aligned start per SIMD-A1.
        for v in slice.iter().copied() {
            acc = f32::algebraic_add(acc, v.0);
        }
    });
    acc
}

fn bevy_iter_fold_sum(
    world: &mut bevy_ecs::world::World,
    state: &mut QueryState<&BevyVelF32>,
) -> f32 {
    // Bevy's fairest baseline: `Iterator::fold` override (PR #6773) + algebraic
    // add to eliminate the per-element reorder barrier.
    state
        .iter(world)
        .fold(0.0_f32, |a, v: &BevyVelF32| f32::algebraic_add(a, v.0))
}

// ===========================================================================
// Bench bodies
// ===========================================================================

fn bench_boyko_for_each_chunk(c: &mut Criterion) {
    register_boyko_velocity();
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[BOYKO_VEL_ID]);
    for i in 0..N_ENTITIES {
        // Deterministic spawn — sequential indices keep the reduction
        // reproducible (and the cargo-asm inspection stable).
        world
            .spawn_one(arch, BoykoVelF32(i as f32))
            .expect("spawn must succeed");
    }

    // `black_box`-anchored sink prevents the optimiser from constant-folding
    // the final sum across iterations.
    static SUM_SINK: AtomicUsize = AtomicUsize::new(0);

    c.bench_function("g6_boyko_for_each_chunk_algebraic_sum_10k", |b| {
        b.iter(|| {
            let sum = boyko_for_each_chunk_sum(black_box(&mut world));
            SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
        });
    });
}

fn bench_bevy_iter_fold(c: &mut Criterion) {
    let mut world = bevy_ecs::world::World::new();
    for i in 0..N_ENTITIES {
        world.spawn(BevyVelF32(i as f32));
    }
    let mut state: QueryState<&BevyVelF32> = world.query();

    static SUM_SINK: AtomicUsize = AtomicUsize::new(0);

    c.bench_function("g6_bevy_iter_fold_algebraic_sum_10k", |b| {
        b.iter(|| {
            let sum = bevy_iter_fold_sum(black_box(&mut world), &mut state);
            SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
        });
    });
}

// ===========================================================================
// Criterion configuration
// ===========================================================================
//
// Plan §8.2 mandates `sample_size(60)` — ≥30 (criterion's default analytic
// floor) with headroom for variance smoothing per Risk 5 (Phase 12.6 g5d
// signal-to-noise lesson). `measurement_time` / `warm_up_time` matched to
// `comparison_v2.rs` defaults so bench-vs-bench comparability is preserved.

fn configure() -> Criterion {
    Criterion::default()
        .sample_size(60)
        .measurement_time(Duration::from_secs(3))
        .warm_up_time(Duration::from_millis(500))
}

criterion_group! {
    name = g6_for_each_chunk;
    config = configure();
    targets = bench_boyko_for_each_chunk, bench_bevy_iter_fold,
}

criterion_main!(g6_for_each_chunk);
