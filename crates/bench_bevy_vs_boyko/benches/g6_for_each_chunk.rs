// `f32::algebraic_add` is a nightly intrinsic (`float_algebraic`). It is
// enabled only under the `nightly` cargo feature; the `[[bench]]` entry for
// this file carries `required-features = ["nightly"]`, so the stable
// `--all-targets` build never compiles it (a bare `#![feature]` would be
// `error[E0554]` on the stable channel). Run via:
//     cargo +nightly bench --features nightly -p bench-bevy-vs-boyko
#![cfg_attr(feature = "nightly", feature(float_algebraic))]

//! Phase X.A Wave 8B + X.A.1 — bench harness for `Query::for_each_chunk`.
//!
//! Compares boyko's `Query::for_each_chunk` against Bevy 0.18's
//! `Query::iter().fold(_, f32::algebraic_add)` pattern. Two groups:
//!
//! * **`g6_*`** (Wave 8B) — single-component f32-sum reduction over
//!   `&VelF32`. Result: boyko ~5 % / ~11 % SLOWER than Bevy (both engines
//!   autovectorize the scalar inner loop; per-row state-machine cost
//!   amortises). The 5× target was NOT met on this shape — see
//!   `docs/PHASE-X.A-RESULTS.md`.
//! * **`g6b_*`** (Phase X.A.1) — multi-component reduction over a
//!   `(PosF32, VelF32, AccF32)` 3-tuple. Validates the plan §13 Risk 5
//!   hypothesis that the speedup widens when Bevy pays a per-row
//!   *tuple-fetch* state-machine cost (one `Iterator::next` advancing
//!   three column cursors per row) that boyko's batched path avoids
//!   (boyko yields three contiguous slices, one per component column).
//!
//! # Target
//!
//! Per plan §1.2 + §8.2: boyko median ≥ **5× Bevy median** over 60 criterion
//! samples. Bench PASS bar matches the §8.4 floor; expected combined gain is
//! 5-8× from slice-elision + `algebraic_add` autovec + 32-byte column-start
//! alignment (Phase X.A SIMD-A1). The plan's Risk 5 last bullet treats any
//! multi-component ratio ≥ 1.10× as a credible win even if the single-component
//! shape lands at parity.
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

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

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

// ---------------------------------------------------------------------------
// Phase X.A.1 — multi-component (3-tuple) reduction types
// ---------------------------------------------------------------------------
//
// Three scalar f32 columns reduced together. ComponentIds 491/492/493 are the
// next free triplet after the single-component 490 slot: Wave 7 reserved up to
// 481, the chunk_iter/par_chunk unit tests use 460-469, and the trybuild
// fixtures use 900-908 — so 491-493 are collision-free across the workspace
// (registering the same id with two distinct Layouts would panic on the second
// `register_layout`, hence the explicit allocation discipline).

const BOYKO_POS_ID: ComponentId = ComponentId(491);
const BOYKO_VEL3_ID: ComponentId = ComponentId(492);
const BOYKO_ACC_ID: ComponentId = ComponentId(493);

#[repr(transparent)]
#[derive(Clone, Copy)]
struct BoykoPosF32(f32);

#[repr(transparent)]
#[derive(Clone, Copy)]
struct BoykoVel3F32(f32);

#[repr(transparent)]
#[derive(Clone, Copy)]
struct BoykoAccF32(f32);

impl BoykoComponent for BoykoPosF32 {
    fn component_id() -> ComponentId {
        BOYKO_POS_ID
    }
}

impl BoykoComponent for BoykoVel3F32 {
    fn component_id() -> ComponentId {
        BOYKO_VEL3_ID
    }
}

impl BoykoComponent for BoykoAccF32 {
    fn component_id() -> ComponentId {
        BOYKO_ACC_ID
    }
}

fn register_boyko_triple() {
    register_layout::<BoykoPosF32>(BOYKO_POS_ID.0);
    register_layout::<BoykoVel3F32>(BOYKO_VEL3_ID.0);
    register_layout::<BoykoAccF32>(BOYKO_ACC_ID.0);
}

#[derive(BevyComponentDerive, Clone, Copy)]
#[repr(transparent)]
struct BevyPosF32(f32);

#[derive(BevyComponentDerive, Clone, Copy)]
#[repr(transparent)]
struct BevyVel3F32(f32);

#[derive(BevyComponentDerive, Clone, Copy)]
#[repr(transparent)]
struct BevyAccF32(f32);

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

// ── Phase X.A.1 multi-component (3-tuple) reductions ────────────────────────
//
// Reduce `pos[i] + vel[i] + acc[i]` across the whole archetype. The boyko
// lane receives three contiguous slices per archetype; the Bevy lane folds a
// per-row 3-tuple yielded by the iterator state machine.
//
// Two boyko inner-loop shapes are benched independently so the better-vectorising
// one can be reported (the architect noted the inner-loop shape is the user's
// responsibility, not the dispatcher's):
//
//   * `_idx`  — index-based `for i in 0..len`, nested `algebraic_add`. The
//     classic three-input reduction shape; LLVM can fuse the three column
//     loads + 2 algebraic adds into a packed-SIMD accumulator since the column
//     starts are `SIMD_BUFFER_ALIGN`-aligned (SIMD-A1) and the bounds are a
//     single `len`.
//   * `_zip`  — `iter().zip().zip().fold(...)`. A more idiomatic shape that
//     also feeds `algebraic_add`; relies on LLVM seeing through the nested
//     `Zip` adaptors.

/// boyko 3-tuple reduction, index-based inner loop.
fn boyko_for_each_chunk_triple_idx(world: &mut EcsMaster) -> f32 {
    let mut acc: f32 = 0.0;
    let mut query = world.query::<(&BoykoPosF32, &BoykoVel3F32, &BoykoAccF32), ()>();
    query.for_each_chunk(
        |(p_slice, v_slice, a_slice): (&[BoykoPosF32], &[BoykoVel3F32], &[BoykoAccF32])| {
            // All three slices share the archetype row count (CD2). The
            // index loop lets LLVM hoist the bounds check once and vectorize
            // the three aligned column loads into one packed reduction.
            let len = p_slice.len();
            for i in 0..len {
                acc = f32::algebraic_add(
                    acc,
                    f32::algebraic_add(
                        p_slice[i].0,
                        f32::algebraic_add(v_slice[i].0, a_slice[i].0),
                    ),
                );
            }
        },
    );
    acc
}

/// boyko 3-tuple reduction, zip-based inner loop.
fn boyko_for_each_chunk_triple_zip(world: &mut EcsMaster) -> f32 {
    let mut acc: f32 = 0.0;
    let mut query = world.query::<(&BoykoPosF32, &BoykoVel3F32, &BoykoAccF32), ()>();
    query.for_each_chunk(
        |(p_slice, v_slice, a_slice): (&[BoykoPosF32], &[BoykoVel3F32], &[BoykoAccF32])| {
            acc = p_slice
                .iter()
                .zip(v_slice.iter())
                .zip(a_slice.iter())
                .fold(acc, |a, ((p, v), c)| {
                    f32::algebraic_add(
                        a,
                        f32::algebraic_add(p.0, f32::algebraic_add(v.0, c.0)),
                    )
                });
        },
    );
    acc
}

/// Bevy 3-tuple reduction — per-row tuple-fetch fold.
fn bevy_iter_fold_triple(
    world: &mut bevy_ecs::world::World,
    state: &mut QueryState<(&BevyPosF32, &BevyVel3F32, &BevyAccF32)>,
) -> f32 {
    // `Iterator::fold` override + `algebraic_add`. Each `next()` advances three
    // archetype column cursors and materialises a `(&P, &V, &A)` tuple — the
    // per-row tuple-fetch cost the chunked API elides.
    state.iter(world).fold(
        0.0_f32,
        |a, (p, v, c): (&BevyPosF32, &BevyVel3F32, &BevyAccF32)| {
            f32::algebraic_add(
                a,
                f32::algebraic_add(p.0, f32::algebraic_add(v.0, c.0)),
            )
        },
    )
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

// ── Phase X.A.1 multi-component bench bodies ────────────────────────────────
//
// The setup mirrors the single-component path exactly: 10k entities, one
// archetype, deterministic sequential spawn (so the reduction is reproducible
// and the asm inspection stable), `black_box` only at the accumulator sink.

/// Spawns the shared 10k-entity `(PosF32, Vel3F32, AccF32)` archetype.
fn spawn_boyko_triple_world() -> (EcsMaster, boyko_ecs::ecs::identifiers::primitives::ArchetypeId)
{
    register_boyko_triple();
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[BOYKO_POS_ID, BOYKO_VEL3_ID, BOYKO_ACC_ID]);
    for i in 0..N_ENTITIES {
        let f = i as f32;
        // SAFETY: `(BoykoPosF32, BoykoVel3F32, BoykoAccF32)` are `#[repr(transparent)]`
        //   POD; reading each as `&[u8]` is valid for this call's duration.
        let p = BoykoPosF32(f);
        let v = BoykoVel3F32(f);
        let a = BoykoAccF32(f);
        let p_bytes = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(p) as *const u8,
                std::mem::size_of::<BoykoPosF32>(),
            )
        };
        let v_bytes = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(v) as *const u8,
                std::mem::size_of::<BoykoVel3F32>(),
            )
        };
        let a_bytes = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(a) as *const u8,
                std::mem::size_of::<BoykoAccF32>(),
            )
        };
        world
            .create_entity(
                arch,
                &[
                    (BOYKO_POS_ID, p_bytes),
                    (BOYKO_VEL3_ID, v_bytes),
                    (BOYKO_ACC_ID, a_bytes),
                ],
            )
            .expect("spawn must succeed");
    }
    (world, arch)
}

fn bench_boyko_for_each_chunk_triple_idx(c: &mut Criterion) {
    let (mut world, _arch) = spawn_boyko_triple_world();

    static SUM_SINK: AtomicUsize = AtomicUsize::new(0);

    c.bench_function("g6b_boyko_for_each_chunk_triple_idx_10k", |b| {
        b.iter(|| {
            let sum = boyko_for_each_chunk_triple_idx(black_box(&mut world));
            SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
        });
    });
}

fn bench_boyko_for_each_chunk_triple_zip(c: &mut Criterion) {
    let (mut world, _arch) = spawn_boyko_triple_world();

    static SUM_SINK: AtomicUsize = AtomicUsize::new(0);

    c.bench_function("g6b_boyko_for_each_chunk_triple_zip_10k", |b| {
        b.iter(|| {
            let sum = boyko_for_each_chunk_triple_zip(black_box(&mut world));
            SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
        });
    });
}

fn bench_bevy_iter_fold_triple(c: &mut Criterion) {
    let mut world = bevy_ecs::world::World::new();
    for i in 0..N_ENTITIES {
        let f = i as f32;
        world.spawn((BevyPosF32(f), BevyVel3F32(f), BevyAccF32(f)));
    }
    let mut state: QueryState<(&BevyPosF32, &BevyVel3F32, &BevyAccF32)> = world.query();

    static SUM_SINK: AtomicUsize = AtomicUsize::new(0);

    c.bench_function("g6b_bevy_iter_fold_triple_10k", |b| {
        b.iter(|| {
            let sum = bevy_iter_fold_triple(black_box(&mut world), &mut state);
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
// `comparison_v2.rs` so bench-vs-bench comparability is preserved.
//
// Phase X.E: warm-up raised to 3 s (steady clock/cache before sampling) and a
// 5% noise threshold added (criterion's default is 1%) so this noisy Windows
// box stops flagging routine run-to-run jitter as a regression. Kept in step
// with `comparison_v2.rs`. See docs/BENCHMARKING.md.

fn configure() -> Criterion {
    Criterion::default()
        .sample_size(60)
        .measurement_time(Duration::from_secs(3))
        .warm_up_time(Duration::from_secs(3))
        .noise_threshold(0.05)
}

criterion_group! {
    name = g6_for_each_chunk;
    config = configure();
    targets =
        bench_boyko_for_each_chunk,
        bench_bevy_iter_fold,
        bench_boyko_for_each_chunk_triple_idx,
        bench_boyko_for_each_chunk_triple_zip,
        bench_bevy_iter_fold_triple,
}

criterion_main!(g6_for_each_chunk);
