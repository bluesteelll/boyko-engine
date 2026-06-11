//! Phase X.I W6 — XI-B4 pool-growth-event cost.
//!
//! Measures ONE `ComponentPool` growth event (the `#[cold] grow_rows` path:
//! 1 data `vm.commit` + 0-2 lockstep tick commits) at the three commit-step
//! classes of the D4 doubling policy.
//!
//! # Gates (binding, docs/PHASE-XI-PLAN.md §Metrics XI-B4)
//!
//! * `pool_grow_event[64KiB]` — gate <= 10 us per event
//! * `pool_grow_event[2MiB]`  — gate <= 10 us per event
//! * `pool_grow_event[64MiB]` — gate <= 50 us per event
//!
//! The gate figures are the per-class lines PRINTED after the bench (see
//! "Mechanism" below) — grep for `pool_grow_event[`. The criterion estimate
//! for the group `pool_grow_event_ladder` is the full ladder fill (an
//! aggregate, NOT the gate metric).
//!
//! # Mechanism — doubling LADDER (the plan's sanctioned alternative)
//!
//! `grow_rows` is `pub(crate)` and so is `Archetype::reserve_capacity` —
//! the ONLY public growth trigger reachable from a bench binary is
//! `ComponentPool::add` with `len` at the committed frontier. The
//! "fresh pool positioned at the target frontier per iteration" shape
//! (iter_batched + PerIteration) was REJECTED: criterion scales its
//! iteration count from the MEASURED duration (~us per event), so the
//! ms-scale setup fill (131 k adds for the 2 MiB frontier, 4.2 M adds for
//! the 64 MiB frontier) would be repeated ~10^5-10^6 times — hours of wall
//! time per class.
//!
//! Instead each `iter_custom` iteration runs ONE doubling ladder: a fresh
//! `ComponentPool::new(&arena, id, 1, 16_000_000)` (D2 mapping — explicit
//! 16 M-row ceiling, comfortably under the u32 construction assert; 16-B
//! stride => 256 MB data sub-region) is filled row by row through the public
//! `add` path. Before each add, `count() == committed_rows()` detects a
//! frontier crossing — exactly `add`'s own internal grow predicate — and
//! THAT single add is timed with `Instant`. The step class is recovered from
//! the `committed_rows()` delta (x16 B = the exact data-commit bytes; the
//! row count never reaches the 16 M ceiling, so no clamp distorts it).
//!
//! With a 16-B stride the D4 trace per ladder is: data commits of
//! 64 KiB x2, 128 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 4 MiB, 8 MiB,
//! 16 MiB, 32 MiB, then 64 MiB x2 (doubling clamps at `POOL_MAX_SLAB`).
//! The ladder stops right after the second full 64 MiB event (frontier
//! 128 MiB -> 192 MiB), i.e. after 8,388,609 adds (~128 MiB of 16-B row
//! writes, ~40-150 ms wall per iteration) — `sample_size(10)` +
//! `measurement_time(8 s)` keeps total wall time sane.
//!
//! # What one timed event contains (honesty note)
//!
//! grow_rows itself (1 data commit syscall + 0-2 tick commit syscalls — the
//! tick frontier saturates and skips on some events) + ONE 16-B `add`
//! (~1-2 ns) + the first-touch demand-zero fault of the fresh slab's first
//! page. That is the real first-write-after-growth cost the gate envelopes.
//!
//! # Component-slot reservation
//!
//! Id **448** — verified free across every bench/test binary in the
//! workspace.

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

use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::ecs::memory::arena::Arena;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use criterion::{Criterion, criterion_group, criterion_main};

const GROW_ID: ComponentId = ComponentId(448);

/// 16-byte POD row (matches the `component_pool_dense` payload class).
#[repr(C)]
#[derive(Clone, Copy)]
struct GrowRow {
    a: u64,
    b: u64,
}

const STRIDE: usize = std::mem::size_of::<GrowRow>(); // 16

/// Explicit D2-mapped reserve ceiling: 16 M rows x 16 B = 256 MB data
/// sub-region — room for the full doubling ladder up to a 192 MiB frontier.
const RESERVE_ROWS: usize = 16_000_000;

/// Stop the ladder once the frontier passes 128 MiB of data — the add that
/// crosses it is the second full 64 MiB commit step (128 MiB -> 192 MiB).
const STOP_ROWS: usize = 128 * 1024 * 1024 / STRIDE; // 8_388_608

const STEP_64K: usize = 64 * 1024;
const STEP_2M: usize = 2 * 1024 * 1024;
const STEP_64M: usize = 64 * 1024 * 1024;

fn register() {
    component_registry::register_layout::<GrowRow>(GROW_ID.0);
}

#[inline]
fn row_bytes(r: &GrowRow) -> &[u8] {
    // SAFETY: GrowRow is #[repr(C)] POD; the slice covers exactly
    // size_of::<GrowRow>() initialized bytes of `r`.
    unsafe { std::slice::from_raw_parts((r as *const GrowRow).cast::<u8>(), STRIDE) }
}

/// `(data_step_bytes, event_duration)` for every frontier-crossing add, from
/// every iteration (warm-up included — more samples, same population).
static EVENTS: Mutex<Vec<(usize, Duration)>> = Mutex::new(Vec::new());

fn bench_pool_grow_event(c: &mut Criterion) {
    register();

    c.bench_function("pool_grow_event_ladder", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                // The arena parameter is vestigial post-X.I D8 (the pool owns
                // its own VmReservation); `Arena::new` is reserve-only ~1 us.
                let arena = Arena::new();
                let mut pool = ComponentPool::new(&arena, GROW_ID.0, 1, RESERVE_ROWS);
                let row = GrowRow {
                    a: 0xDEAD_BEEF,
                    b: 0x5EED_F00D,
                };
                let bytes = row_bytes(&row);
                let mut events: Vec<(usize, Duration)> = Vec::with_capacity(16);

                let start = Instant::now();
                while pool.committed_rows() <= STOP_ROWS {
                    if pool.count() == pool.committed_rows() {
                        // This add crosses the frontier => grow_rows fires.
                        let before = pool.committed_rows();
                        let t0 = Instant::now();
                        pool.add(bytes)
                            .expect("ladder stays below the 16M-row reserve ceiling");
                        let dt = t0.elapsed();
                        events.push(((pool.committed_rows() - before) * STRIDE, dt));
                    } else {
                        pool.add(bytes)
                            .expect("ladder stays below the 16M-row reserve ceiling");
                    }
                }
                total += start.elapsed();

                EVENTS
                    .lock()
                    .expect("grow-event sink poisoned")
                    .extend(events);
                // Pool drop releases its reservation; per-iteration commit
                // charge (~288 MiB incl. ticks) does not accumulate.
                drop(pool);
                drop(arena);
            }
            total
        });
    });

    report_grow_events();
}

/// Per-class aggregation — these printed lines ARE the XI-B4 gate figures.
fn report_grow_events() {
    let mut sink = EVENTS.lock().expect("grow-event sink poisoned");
    if sink.is_empty() {
        return;
    }
    let mut classes: BTreeMap<usize, Vec<Duration>> = BTreeMap::new();
    for &(step, d) in sink.iter() {
        classes.entry(step).or_default().push(d);
    }
    for (step, mut durs) in classes {
        durs.sort_unstable();
        let max = durs[durs.len() - 1];
        let median = durs[durs.len() / 2];
        match step {
            STEP_64K => println!(
                "pool_grow_event[64KiB]: events={} max={max:?} median={median:?} (XI-B4 gate <= 10 us)",
                durs.len()
            ),
            STEP_2M => println!(
                "pool_grow_event[2MiB]: events={} max={max:?} median={median:?} (XI-B4 gate <= 10 us)",
                durs.len()
            ),
            STEP_64M => println!(
                "pool_grow_event[64MiB]: events={} max={max:?} median={median:?} (XI-B4 gate <= 50 us)",
                durs.len()
            ),
            other => println!(
                "pool_grow_event[{}KiB] (untargeted ladder rung): events={} max={max:?} median={median:?}",
                other / 1024,
                durs.len()
            ),
        }
    }
    sink.clear();
}

fn configure() -> Criterion {
    // ~40-150 ms per ladder iteration; 10 samples x 8 s keeps wall time
    // bounded while still collecting >= ~130 events per gate class run.
    Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(8))
        .warm_up_time(Duration::from_secs(2))
}

criterion_group! {
    name = pool_grow_event;
    config = configure();
    targets = bench_pool_grow_event,
}

criterion_main!(pool_grow_event);
