//! Phase 22 (Tags) — benchmark suite per docs/PHASE-22-TAGS-PLAN.md §"New
//! benches".
//!
//! # Gates / report figures
//!
//! | Bench id                            | Target / role                       |
//! |-------------------------------------|-------------------------------------|
//! | `p22_spawn_batch_10k_tag_only`      | report figure (ZST-column spawn)    |
//! | `p22_spawn_batch_10k_2data`         | comparison baseline                 |
//! | `p22_spawn_batch_10k_2data_2tags`   | gate: <= baseline x 1.10            |
//! | `p22_has_tag_hit` / `_miss`         | gate: <= 5 ns                       |
//! | `p22_tag_first_attach_cold`         | ATTRIBUTION: incl. archetype create |
//! | `p22_tag_toggle_warm_x10k`          | ATTRIBUTION: pure row-move toggles  |
//! | `p22_query_iter_10k_{no,one}_term`  | term overhead, iter driver          |
//! | `p22_for_each_chunk_10k_{no,one}_term` | term overhead, chunk driver      |
//! | `p22_zst_pool_grow_ladder`          | cold ZST grow events (printed lines)|
//!
//! # Attribution note (plan §"New benches")
//!
//! The dynamic attach/detach cost decomposes into two distinct regimes which
//! this suite separates instead of averaging:
//!
//! - `p22_tag_first_attach_cold` — one `add_tag` against a FRESH world where
//!   the `{data, tag}` archetype does not exist yet: the figure INCLUDES
//!   `get_or_create_archetype` (mask build + archetype construction + pool
//!   reservation) and the hook/observer gate checks.
//! - `p22_tag_toggle_warm_x10k` — 10k attach + 10k detach against a world
//!   where BOTH archetypes already exist: the figure is the pure row-move
//!   migration (retained-column memcpy + swap_remove + inland update), i.e.
//!   the steady-state cost of toggling a tag at runtime.
//!
//! # Tag-budget hygiene
//!
//! Tag mints are process-global and idempotent per name. This binary mints
//! exactly FOUR uniquely-prefixed dynamic tags (`p22b_*`); `iter_batched`
//! setups re-mint the same name (idempotent re-mint = success, no budget
//! drain). No mint-to-ceiling loops.

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc. See
// docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use boyko_ecs::prelude::{Component, EcsMaster, Entity, TagId};
use boyko_macros::{Bundle, Component};
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};

// ── Component fixtures (derive-minted ids — no pinned slots) ────────────────

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct P22bPos {
    x: u64,
    y: u64,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct P22bVel {
    x: u64,
    y: u64,
}

/// Static ZST tag used by the tag-only and mixed-bundle spawn benches.
#[derive(Component)]
#[derive(Clone, Copy)]
struct P22bTagA;

#[derive(Component)]
#[derive(Clone, Copy)]
struct P22bTagB;

#[derive(Bundle)]
struct P22bPosBundle {
    pos: P22bPos,
}

#[derive(Bundle)]
struct P22b2Data {
    pos: P22bPos,
    vel: P22bVel,
}

#[derive(Bundle)]
struct P22b2Data2Tags {
    pos: P22bPos,
    vel: P22bVel,
    a: P22bTagA,
    b: P22bTagB,
}

const N: usize = 10_000;
/// Direct `spawn_batch` is chunked at 5_000 (<= MAX_BATCH_HINT), mirroring
/// `phase12_5_spawn_batch.rs`. `u32`, not `u64`: `spawn_batch` requires an
/// `ExactSizeIterator` and `Range<u64>` does not implement it.
const CHUNK: u32 = 5_000;
const CHUNKS: u64 = (N as u64) / CHUNK as u64;

#[inline]
fn pos(i: u64) -> P22bPos {
    P22bPos {
        x: i,
        y: i.wrapping_mul(3),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// (a) spawn 10k tag-only — ZST-column-only archetype through the direct
//     bundle spawner (derive(Component) single-component Bundle emission)
// ════════════════════════════════════════════════════════════════════════════

fn bench_spawn_tag_only(c: &mut Criterion) {
    c.bench_function("p22_spawn_batch_10k_tag_only", |b| {
        b.iter_with_setup(EcsMaster::new, |mut ecs| {
            for _ in 0..CHUNKS {
                let _ = ecs
                    .spawn_batch((0..CHUNK).map(|_| P22bTagA))
                    .expect("5000 <= MAX_BATCH_HINT");
            }
            black_box(ecs.entity_count());
        });
    });
}

// ════════════════════════════════════════════════════════════════════════════
// (b) spawn 10k (2 data + 2 tags) vs (2 data) — gate: tags variant <= +10%
// ════════════════════════════════════════════════════════════════════════════

fn bench_spawn_2data(c: &mut Criterion) {
    c.bench_function("p22_spawn_batch_10k_2data", |b| {
        b.iter_with_setup(EcsMaster::new, |mut ecs| {
            for chunk in 0..CHUNKS {
                let base = chunk * u64::from(CHUNK);
                let _ = ecs
                    .spawn_batch((0..CHUNK).map(move |i| P22b2Data {
                        pos: pos(base + u64::from(i)),
                        vel: P22bVel {
                            x: base + u64::from(i),
                            y: 0,
                        },
                    }))
                    .expect("5000 <= MAX_BATCH_HINT");
            }
            black_box(ecs.entity_count());
        });
    });
}

fn bench_spawn_2data_2tags(c: &mut Criterion) {
    c.bench_function("p22_spawn_batch_10k_2data_2tags", |b| {
        b.iter_with_setup(EcsMaster::new, |mut ecs| {
            for chunk in 0..CHUNKS {
                let base = chunk * u64::from(CHUNK);
                let _ = ecs
                    .spawn_batch((0..CHUNK).map(move |i| P22b2Data2Tags {
                        pos: pos(base + u64::from(i)),
                        vel: P22bVel {
                            x: base + u64::from(i),
                            y: 0,
                        },
                        a: P22bTagA,
                        b: P22bTagB,
                    }))
                    .expect("5000 <= MAX_BATCH_HINT");
            }
            black_box(ecs.entity_count());
        });
    });
}

// ════════════════════════════════════════════════════════════════════════════
// (c) has_tag — gate <= 5 ns (hit AND miss)
// ════════════════════════════════════════════════════════════════════════════

fn bench_has_tag(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("p22b_hastag");
    let never_attached = ecs.register_tag("p22b_hastag_miss");
    let e = ecs.spawn_empty();
    ecs.add_tag(e, tag);

    c.bench_function("p22_has_tag_hit", |b| {
        b.iter(|| black_box(ecs.has_tag(black_box(e), black_box(tag))));
    });
    c.bench_function("p22_has_tag_miss", |b| {
        b.iter(|| black_box(ecs.has_tag(black_box(e), black_box(never_attached))));
    });
}

// ════════════════════════════════════════════════════════════════════════════
// (d) dynamic tag attach/detach — ATTRIBUTION split (see module docs)
// ════════════════════════════════════════════════════════════════════════════

/// First toggle: one `add_tag` in a fresh world — INCLUDES the
/// `get_or_create_archetype` miss (mask build + archetype construction +
/// pool reservation) and the hook/observer gate checks.
fn bench_tag_first_attach_cold(c: &mut Criterion) {
    c.bench_function("p22_tag_first_attach_cold", |b| {
        b.iter_batched(
            || {
                let mut ecs = EcsMaster::new();
                // Idempotent re-mint per iteration — no budget drain.
                let tag = ecs.register_tag("p22b_first_attach");
                let arch = ecs.create_archetype(&[P22bPos::component_id()]);
                let e = ecs
                    .spawn_one(arch, pos(7))
                    .expect("spawn_one(P22bPos) into its 1-component archetype");
                (ecs, tag, e)
            },
            |(mut ecs, tag, e)| {
                ecs.add_tag(e, tag);
                // Return the world so its teardown (VM release) drops OUTSIDE
                // the timed region.
                ecs
            },
            BatchSize::PerIteration,
        );
    });
}

/// Warm toggle: both archetypes exist, both pools sized — 10k attach + 10k
/// detach of pure row-move migrations per iteration. Per-toggle-pair figure
/// = criterion estimate / 10_000.
fn bench_tag_toggle_warm(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("p22b_toggle");
    let mut entities: Vec<Entity> = Vec::with_capacity(N);
    for chunk in 0..CHUNKS {
        let base = chunk * u64::from(CHUNK);
        entities.extend(
            ecs.spawn_batch((0..CHUNK).map(move |i| P22bPosBundle {
                pos: pos(base + u64::from(i)),
            }))
            .expect("5000 <= MAX_BATCH_HINT"),
        );
    }
    // Pre-warm: create the {P22bPos, tag} archetype and size both pools so
    // the timed region contains no archetype creation and no pool growth.
    for &e in &entities {
        ecs.add_tag(e, tag);
    }
    for &e in &entities {
        ecs.remove_tag(e, tag);
    }

    c.bench_function("p22_tag_toggle_warm_x10k", |b| {
        b.iter(|| {
            for &e in &entities {
                ecs.add_tag(e, tag);
            }
            for &e in &entities {
                ecs.remove_tag(e, tag);
            }
            black_box(ecs.entity_count());
        });
    });
}

// ════════════════════════════════════════════════════════════════════════════
// (e) query with 1 dynamic term vs none — iter AND for_each_chunk drivers.
//     Single 10k-row tagged archetype: identical per-row work in both
//     variants, so the delta IS the term overhead (archetype-level test +
//     per-cursor dispatch).
// ════════════════════════════════════════════════════════════════════════════

fn term_world() -> (EcsMaster, TagId) {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("p22b_queryterm");
    let mut entities: Vec<Entity> = Vec::with_capacity(N);
    for chunk in 0..CHUNKS {
        let base = chunk * u64::from(CHUNK);
        entities.extend(
            ecs.spawn_batch((0..CHUNK).map(move |i| P22bPosBundle {
                pos: pos(base + u64::from(i)),
            }))
            .expect("5000 <= MAX_BATCH_HINT"),
        );
    }
    // Migrate the whole population into {P22bPos, tag} via the public attach
    // surface — the plain archetype stays behind, EMPTY (a matched-but-empty
    // archetype the no-term variant also walks, keeping the archetype list
    // shape identical across both variants).
    for &e in &entities {
        ecs.add_tag(e, tag);
    }
    (ecs, tag)
}

fn bench_query_term_overhead(c: &mut Criterion) {
    let (mut ecs, tag) = term_world();

    c.bench_function("p22_query_iter_10k_no_term", |b| {
        b.iter(|| {
            let view = ecs.query::<&P22bPos, ()>();
            let mut sum = 0u64;
            for p in view.iter() {
                sum = sum.wrapping_add(p.x);
            }
            black_box(sum)
        });
    });

    c.bench_function("p22_query_iter_10k_one_term", |b| {
        b.iter(|| {
            let view = ecs.query::<&P22bPos, ()>().with_tag(tag);
            let mut sum = 0u64;
            for p in view.iter() {
                sum = sum.wrapping_add(p.x);
            }
            black_box(sum)
        });
    });

    c.bench_function("p22_for_each_chunk_10k_no_term", |b| {
        b.iter(|| {
            let mut view = ecs.query::<&P22bPos, ()>();
            let mut sum = 0u64;
            view.for_each_chunk(|slice: &[P22bPos]| {
                for p in slice {
                    sum = sum.wrapping_add(p.x);
                }
            });
            black_box(sum)
        });
    });

    c.bench_function("p22_for_each_chunk_10k_one_term", |b| {
        b.iter(|| {
            let mut view = ecs.query::<&P22bPos, ()>().with_tag(tag);
            let mut sum = 0u64;
            view.for_each_chunk(|slice: &[P22bPos]| {
                for p in slice {
                    sum = sum.wrapping_add(p.x);
                }
            });
            black_box(sum)
        });
    });
}

// ════════════════════════════════════════════════════════════════════════════
// (f) ZST pool grow (cold) — frontier-crossing adds through the PUBLIC
//     `ComponentPool::add(&[])` surface (`grow_rows` is not public), the
//     `pool_grow_event.rs` ladder pattern. A ZST pool is tick-only: each
//     event is 0 data commits + the lockstep tick commits of
//     `grow_rows_zst`, plus the first-touch fault of the fresh tick slab.
// ════════════════════════════════════════════════════════════════════════════

/// 8 M-row tick-only reservation; the ladder stops after the frontier passes
/// 2 M rows, so every doubling event below that rung is sampled.
const ZST_RESERVE_ROWS: usize = 8_000_000;
const ZST_STOP_ROWS: usize = 2_097_152;

/// `(rows_step, event_duration)` for every frontier-crossing add (warm-up
/// included — more samples, same population).
static ZST_EVENTS: Mutex<Vec<(usize, Duration)>> = Mutex::new(Vec::new());

fn bench_zst_pool_grow(c: &mut Criterion) {
    // Mint the ZST layout once through the public tag surface; the registry
    // is process-global, so the raw pool below sees the size-0 layout.
    let tag_cid = EcsMaster::new().register_tag("p22b_zst_grow").component_id();

    c.bench_function("p22_zst_pool_grow_ladder", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let mut pool = ComponentPool::new(tag_cid.0, ZST_RESERVE_ROWS);
                let mut events: Vec<(usize, Duration)> = Vec::with_capacity(16);

                let start = Instant::now();
                while pool.committed_rows() <= ZST_STOP_ROWS {
                    if pool.count() == pool.committed_rows() {
                        // This add crosses the frontier => grow_rows_zst fires.
                        let before = pool.committed_rows();
                        let t0 = Instant::now();
                        pool.add(&[])
                            .expect("ladder stays below the 8M-row reserve ceiling");
                        let dt = t0.elapsed();
                        events.push((pool.committed_rows() - before, dt));
                    } else {
                        pool.add(&[])
                            .expect("ladder stays below the 8M-row reserve ceiling");
                    }
                }
                total += start.elapsed();

                ZST_EVENTS
                    .lock()
                    .expect("zst grow-event sink poisoned")
                    .extend(events);
                drop(pool);
            }
            total
        });
    });

    report_zst_grow_events();
}

/// Per-rung aggregation — these printed lines are the report figures (the
/// criterion estimate for the group is the full ladder fill, an aggregate).
fn report_zst_grow_events() {
    let mut sink = ZST_EVENTS.lock().expect("zst grow-event sink poisoned");
    if sink.is_empty() {
        // Either the ladder bench was filtered out of this invocation, or the
        // count()==committed_rows() frontier predicate never fired.
        println!("p22_zst_pool_grow: no grow events collected (bench filtered out?)");
        return;
    }
    let mut classes: BTreeMap<usize, Vec<Duration>> = BTreeMap::new();
    for &(step_rows, d) in sink.iter() {
        classes.entry(step_rows).or_default().push(d);
    }
    for (step_rows, mut durs) in classes {
        durs.sort_unstable();
        let max = durs[durs.len() - 1];
        let median = durs[durs.len() / 2];
        println!(
            "p22_zst_pool_grow[{} rows]: events={} max={max:?} median={median:?}",
            step_rows,
            durs.len()
        );
    }
    sink.clear();
}

// ════════════════════════════════════════════════════════════════════════════

fn configure() -> Criterion {
    Criterion::default()
}

/// The ZST ladder iteration is ms-scale (~2 M adds); bound its wall time the
/// way `pool_grow_event.rs` does.
fn configure_grow() -> Criterion {
    Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(6))
        .warm_up_time(Duration::from_secs(2))
}

criterion_group! {
    name = phase22_tags;
    config = configure();
    targets =
        bench_spawn_tag_only,
        bench_spawn_2data,
        bench_spawn_2data_2tags,
        bench_has_tag,
        bench_tag_first_attach_cold,
        bench_tag_toggle_warm,
        bench_query_term_overhead,
}

criterion_group! {
    name = phase22_zst_grow;
    config = configure_grow();
    targets = bench_zst_pool_grow,
}

criterion_main!(phase22_tags, phase22_zst_grow);
