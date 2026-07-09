//! Chunk-aware EnableTag filter — the PERF gate (rev. 3, gate 4).
//!
//! Proves the run-aware `for_each_chunk` enable-filtered path restores SoA speed
//! versus the scalar per-row `with_enabled().iter()` path that today pays the
//! `EnableTermCols::passes` per-row predicate (~+3.2 ns/row over the ~3.4 ns SoA
//! chunk baseline). Each density runs an A/B inside ONE binary so the numbers are
//! directly comparable:
//!
//! | bench id                              | path                                   |
//! |---------------------------------------|----------------------------------------|
//! | `enable_filter/scalar_iter/{d}`       | `with_enabled(tag).iter()` (per-row)   |
//! | `enable_filter/chunk/{d}`             | `with_enabled(tag).for_each_chunk()`   |
//! | `enable_filter/zero_gate/no_filter`   | plain `for_each_chunk()` (0%-gate ref) |
//! | `enable_filter/zero_gate/empty_term`  | `with_enabled` over an absent column   |
//!
//! Densities `d ∈ {99, 50, 1}` percent enabled over a 100k-row archetype (>24
//! pages), so the run extractor is exercised across many word / page boundaries.
//!
//! Divide the criterion estimate by the VISITED row count (≈ density·N) for the
//! per-row figure; the run-aware `chunk` path should be markedly below the
//! `scalar_iter` path at 99% / 50% and content-scaling (≈ flat absolute) at 1%.

// Phase X.E: opt-in low-variance allocator for A/B signal extraction. OFF by
// default; `cargo bench --features bench-alloc` swaps in mimalloc.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use boyko_ecs::ecs::core::component::component_registry::EnableTagId;
use boyko_ecs::prelude::{EcsMaster, Entity};
use boyko_macros::{Bundle, Component};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::hint::black_box as hint_black_box;

/// Real data component the queries read (table storage, derive-minted id).
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct EcfPos {
    x: u64,
    y: u64,
}

#[derive(Bundle)]
struct EcfBundle {
    pos: EcfPos,
}

/// 100k rows ⇒ > 24 EnablePages — the run extractor crosses many page/word
/// boundaries, so the per-row amortised summary load is exercised honestly.
const N: usize = 100_000;
const CHUNK: u32 = 5_000;

#[inline]
fn pos(i: u64) -> EcfPos {
    EcfPos {
        x: i,
        y: i.wrapping_mul(3),
    }
}

/// Spawns `count` `EcfPos` entities through `spawn_batch` (5k-chunked).
fn spawn_population(ecs: &mut EcsMaster, count: usize) -> Vec<Entity> {
    let mut entities: Vec<Entity> = Vec::with_capacity(count);
    let full_chunks = count / CHUNK as usize;
    let remainder = (count % CHUNK as usize) as u32;
    for chunk in 0..full_chunks as u64 {
        let base = chunk * u64::from(CHUNK);
        entities.extend(
            ecs.spawn_batch((0..CHUNK).map(move |i| EcfBundle {
                pos: pos(base + u64::from(i)),
            }))
            .expect("5000 <= MAX_BATCH_HINT"),
        );
    }
    if remainder > 0 {
        let base = full_chunks as u64 * u64::from(CHUNK);
        entities.extend(
            ecs.spawn_batch((0..remainder).map(move |i| EcfBundle {
                pos: pos(base + u64::from(i)),
            }))
            .expect("remainder < MAX_BATCH_HINT"),
        );
    }
    entities
}

/// Deterministic splitmix64 so a density pattern is reproducible.
struct SplitMix64(u64);
impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn chance(&mut self, pct: u32) -> bool {
        (self.next_u64() % 100) < u64::from(pct)
    }
}

/// Builds one populated world, enables `~pct%` of rows on a fresh dynamic tag,
/// and returns the world + the tag. The enable pattern is seeded per density so
/// the A and B arms of a density share an IDENTICAL bit pattern.
fn world_with_density(pct: u32, seed: u64) -> (EcsMaster, EnableTagId) {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("ecf_density_tag");
    let entities = spawn_population(&mut ecs, N);
    let mut r = SplitMix64(seed);
    for &e in &entities {
        if r.chance(pct) {
            ecs.enable_id(e, tag);
        }
    }
    (ecs, tag)
}

// ── A: scalar per-row `with_enabled().iter()` (the baseline) ────────────────

fn bench_scalar_iter(c: &mut Criterion, pct: u32) {
    let (mut ecs, tag) = world_with_density(pct, 0x5EED_0001 ^ u64::from(pct));
    c.bench_function(&format!("enable_filter/scalar_iter/{pct}pct"), |b| {
        b.iter(|| {
            let view = ecs.query::<&EcfPos, ()>().with_enabled(tag);
            let mut sum = 0u64;
            for p in view.iter() {
                sum = sum.wrapping_add(hint_black_box(p.x));
            }
            black_box(sum)
        });
    });
}

// ── B: chunk-aware `with_enabled().for_each_chunk()` (the new path) ─────────

fn bench_chunk(c: &mut Criterion, pct: u32) {
    let (mut ecs, tag) = world_with_density(pct, 0x5EED_0001 ^ u64::from(pct));
    c.bench_function(&format!("enable_filter/chunk/{pct}pct"), |b| {
        b.iter(|| {
            let mut view = ecs.query::<&EcfPos, ()>().with_enabled(tag);
            let mut sum = 0u64;
            view.for_each_chunk(|slice: &[EcfPos]| {
                for p in slice {
                    sum = sum.wrapping_add(hint_black_box(p.x));
                }
            });
            black_box(sum)
        });
    });
}

// ── 0%-gate references: no-filter chunk vs an absent-column enable term ──────

fn bench_zero_gate(c: &mut Criterion) {
    // Plain no-filter for_each_chunk over a fully-populated world — the byte
    // -identical single-fetch path. This is the loop-invariant 0%-gate reference.
    let mut ecs = EcsMaster::new();
    let _entities = spawn_population(&mut ecs, N);
    c.bench_function("enable_filter/zero_gate/no_filter", |b| {
        b.iter(|| {
            let mut view = ecs.query::<&EcfPos, ()>();
            let mut sum = 0u64;
            view.for_each_chunk(|slice: &[EcfPos]| {
                for p in slice {
                    sum = sum.wrapping_add(hint_black_box(p.x));
                }
            });
            black_box(sum)
        });
    });

    // `with_enabled` over a registered-but-never-toggled tag — the absent-column
    // (`is_empty()` term still present) path. The composite is empty ⇒ zero runs,
    // so this measures the per-archetype resolve + summary-skip overhead with no
    // rows visited (content-scaling floor).
    let mut ecs2 = EcsMaster::new();
    let tag = ecs2.register_enable_tag("ecf_zero_gate_absent");
    let _e2 = spawn_population(&mut ecs2, N);
    c.bench_function("enable_filter/zero_gate/with_enabled_absent_column", |b| {
        b.iter(|| {
            let mut view = ecs2.query::<&EcfPos, ()>().with_enabled(tag);
            let mut sum = 0u64;
            view.for_each_chunk(|slice: &[EcfPos]| {
                for p in slice {
                    sum = sum.wrapping_add(hint_black_box(p.x));
                }
            });
            black_box(sum)
        });
    });
}

fn bench_all(c: &mut Criterion) {
    for pct in [99u32, 50, 1] {
        bench_scalar_iter(c, pct);
        bench_chunk(c, pct);
    }
    bench_zero_gate(c);
}

criterion_group!(enable_chunk_filter, bench_all);
criterion_main!(enable_chunk_filter);
