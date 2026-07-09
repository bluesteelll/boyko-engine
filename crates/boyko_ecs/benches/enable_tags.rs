//! EnableTag (Wave 6 / Step 11) — criterion benchmark suite for the enable-bit
//! non-fragmenting tag backend (`#[component(storage = "bitset")]` /
//! `register_enable_tag`).
//!
//! # Bench ids / role
//!
//! | Bench id                       | Target / role                            |
//! |--------------------------------|------------------------------------------|
//! | `enable_toggle`                | warm O(1) bit RMW (no migration). <5 ns  |
//! | `query_iter_enabled`           | `Query<&P, Enabled<A>>` iter. <=1.5 ns/row|
//! | `spawn_with_enable_tag`        | spawn + enable vs plain spawn (no churn) |
//! | `enable_toggle_large_archetype`| toggle in a >4096-row archetype (page 2+) |
//!
//! # What the enable-bit path costs (vs the Phase-22 signature tag)
//!
//! A signature (archetypal) tag toggle is a ROW-MOVE migration between two
//! archetypes (`phase22_tags::p22_tag_toggle_warm_x10k`). An ENABLE-bit toggle
//! is a single per-row read-modify-write at `(archetype, row)`: no migration,
//! no structural-generation bump, no hook / observer fire, no deferred drain
//! (flecs `CanToggle` semantics — see `enable_tag_api.rs`). These benches
//! isolate that RMW and the per-row query gate so the report can quote the
//! steady-state toggle cost and the iteration overhead of `Enabled<T>`.
//!
//! # Tag-budget hygiene
//!
//! Tag mints are process-global and idempotent per name / per type. This binary
//! mints exactly the derived bitset tags below (one `OnceLock` id each) plus the
//! ONE name-keyed dynamic enable tag `et_toggle_dyn` (idempotent re-mint per
//! `iter_batched` setup = no budget drain). No mint-to-ceiling loops; bitset
//! ids are always derive-minted or `register_enable_tag`-minted (never
//! hand-pinned — collision hazard in the shared bench process).

// Phase X.E: opt-in low-variance allocator for A/B signal extraction. OFF by
// default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc. See
// docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use boyko_ecs::ecs::core::iters::query::Enabled;
use boyko_ecs::prelude::{EcsMaster, Entity};
use boyko_macros::{Bundle, Component};
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use std::hint::black_box as hint_black_box;

// ── Component fixtures (derive-minted ids — no pinned slots) ────────────────

/// Real data component the queries read; a normal (table-storage) derive.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct EtPos {
    x: u64,
    y: u64,
}

/// Headline typed enable tag: a fieldless (ZST) bitset tag minted by the derive.
/// It is filtered out of every archetype signature and has NO `ComponentPool`;
/// you spawn with the real component(s), then `enable::<EtFlag>(entity)`.
#[derive(Component)]
#[component(storage = "bitset")]
struct EtFlag;

/// A second derived bitset tag for the large-archetype toggle bench, kept
/// distinct so the two benches do not share an `EnableColumn`.
#[derive(Component)]
#[component(storage = "bitset")]
struct EtBigFlag;

#[derive(Bundle)]
struct EtPosBundle {
    pos: EtPos,
}

const N: usize = 10_000;
/// Direct `spawn_batch` is chunked at 5_000 (<= MAX_BATCH_HINT), mirroring
/// `phase22_tags.rs`. `u32`, not `u64`: `spawn_batch` requires an
/// `ExactSizeIterator` and `Range<u64>` does not implement it.
const CHUNK: u32 = 5_000;

#[inline]
fn pos(i: u64) -> EtPos {
    EtPos {
        x: i,
        y: i.wrapping_mul(3),
    }
}

/// Spawns `count` `EtPos` entities through `spawn_batch` (5k-chunked) and
/// returns the handles. Used by every bench that needs a populated world.
fn spawn_pos_population(ecs: &mut EcsMaster, count: usize) -> Vec<Entity> {
    let mut entities: Vec<Entity> = Vec::with_capacity(count);
    let full_chunks = count / CHUNK as usize;
    let remainder = (count % CHUNK as usize) as u32;
    for chunk in 0..full_chunks as u64 {
        let base = chunk * u64::from(CHUNK);
        entities.extend(
            ecs.spawn_batch((0..CHUNK).map(move |i| EtPosBundle {
                pos: pos(base + u64::from(i)),
            }))
            .expect("5000 <= MAX_BATCH_HINT"),
        );
    }
    if remainder > 0 {
        let base = full_chunks as u64 * u64::from(CHUNK);
        entities.extend(
            ecs.spawn_batch((0..remainder).map(move |i| EtPosBundle {
                pos: pos(base + u64::from(i)),
            }))
            .expect("remainder < MAX_BATCH_HINT"),
        );
    }
    entities
}

// ════════════════════════════════════════════════════════════════════════════
// 1. enable_toggle — warm O(1) bit RMW (no archetype migration).
//
// The {EtPos, EtFlag} EnableColumn (and its first page) is pre-allocated in
// setup, so the timed region is the pure per-row read-modify-write of the
// `AtomicU64` enable word — no column alloc, no enable_generation bump, no
// migration. Target context: <5 ns warm (measured, not asserted).
// ════════════════════════════════════════════════════════════════════════════

fn bench_enable_toggle(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    let entities = spawn_pos_population(&mut ecs, 1);
    let e = entities[0];
    // Pre-warm: first toggle allocates the column + first page + bumps
    // enable_generation once. The timed loop reuses them.
    ecs.enable::<EtFlag>(e);
    ecs.disable::<EtFlag>(e);

    c.bench_function("enable_toggle", |b| {
        b.iter(|| {
            ecs.enable::<EtFlag>(black_box(e));
            ecs.disable::<EtFlag>(black_box(e));
        });
    });
}

// ════════════════════════════════════════════════════════════════════════════
// 2. query_iter_enabled — `Query<&P, Enabled<A>>` over 10k rows, ~half enabled.
//
// A single 10k-row {EtPos} archetype with an allocated EtFlag EnableColumn;
// every even-indexed row is enabled (~5k visited). The criterion estimate is
// the whole-archetype walk INCLUDING the per-row enable gate; divide by N for
// the per-row context figure (target <=1.5 ns/row). `black_box` per element
// blocks the inner loop from being optimised away.
// ════════════════════════════════════════════════════════════════════════════

fn bench_query_iter_enabled(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    let entities = spawn_pos_population(&mut ecs, N);
    // Enable ~half: every even-indexed row.
    for (i, &e) in entities.iter().enumerate() {
        if i % 2 == 0 {
            ecs.enable::<EtFlag>(e);
        }
    }

    c.bench_function("query_iter_enabled", |b| {
        b.iter(|| {
            let view = ecs.query::<&EtPos, Enabled<EtFlag>>();
            let mut sum = 0u64;
            for p in view.iter() {
                sum = sum.wrapping_add(hint_black_box(p.x));
            }
            black_box(sum)
        });
    });
}

// ════════════════════════════════════════════════════════════════════════════
// 3. spawn_with_enable_tag — spawn N then enable N, vs a plain spawn-N
//    baseline. Shows the enable-bit path adds no spawn-time archetype churn /
//    tick-pool floor: the `enable` arm spawns into the SAME {EtPos} archetype
//    the baseline does (the bitset id is filtered out of the signature) and
//    then flips a per-row bit — no second archetype, no migration. Target:
//    the `_enable` group ≈ the plain `_spawn` group + the cheap RMW pass.
//
//    Each group measures: build a fresh world, spawn N {EtPos}, (enable arm)
//    set the EtFlag bit on all N. `iter_batched` with `PerIteration` so each
//    world's teardown (VM release) drops OUTSIDE the timed region.
// ════════════════════════════════════════════════════════════════════════════

fn bench_spawn_with_enable_tag(c: &mut Criterion) {
    let mut group = c.benchmark_group("spawn_with_enable_tag");

    group.bench_function("plain_spawn", |b| {
        b.iter_batched(
            EcsMaster::new,
            |mut ecs| {
                let entities = spawn_pos_population(&mut ecs, N);
                black_box(entities.len());
                ecs
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function("spawn_then_enable", |b| {
        b.iter_batched(
            EcsMaster::new,
            |mut ecs| {
                let entities = spawn_pos_population(&mut ecs, N);
                for &e in &entities {
                    ecs.enable::<EtFlag>(black_box(e));
                }
                black_box(entities.len());
                ecs
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();
}

// ════════════════════════════════════════════════════════════════════════════
// 4. enable_toggle_large_archetype — toggle the bit on an entity living in a
//    >4096-row archetype, forcing a 2nd+ `EnablePage` (each page is 64 rows ×
//    64 bits = 4096 rows; row 4096 lives on page 1). The entity toggled is at
//    row >4096, so its bit is on a non-first page. This confirms page
//    allocation is bounded/lazy (each page ≤512 B) and the toggle stays O(1)
//    regardless of which page the row falls on. Measures the warm RMW.
// ════════════════════════════════════════════════════════════════════════════

const LARGE_ROWS: usize = 5_000;
/// A row strictly beyond the page-0 boundary (4096) — lives on page 1.
const PAGE1_ROW: usize = 4_500;

fn bench_enable_toggle_large_archetype(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    let entities = spawn_pos_population(&mut ecs, LARGE_ROWS);
    assert!(
        entities.len() > 4096,
        "the large-archetype bench needs >4096 rows to force a 2nd EnablePage"
    );
    let e = entities[PAGE1_ROW];
    // Pre-warm: this first toggle allocates the column AND page 1 (the page that
    // owns PAGE1_ROW). The timed loop is then the pure warm RMW on that page.
    ecs.enable::<EtBigFlag>(e);
    ecs.disable::<EtBigFlag>(e);

    c.bench_function("enable_toggle_large_archetype", |b| {
        b.iter(|| {
            ecs.enable::<EtBigFlag>(black_box(e));
            ecs.disable::<EtBigFlag>(black_box(e));
        });
    });
}

// ════════════════════════════════════════════════════════════════════════════

criterion_group!(
    enable_tags,
    bench_enable_toggle,
    bench_query_iter_enabled,
    bench_spawn_with_enable_tag,
    bench_enable_toggle_large_archetype,
);
criterion_main!(enable_tags);
