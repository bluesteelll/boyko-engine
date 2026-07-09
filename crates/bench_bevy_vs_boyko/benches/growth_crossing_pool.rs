//! Phase X.I — g8 growth-crossing benchmark: boyko vs Bevy 0.18.1.
//!
//! THE X.I headline gate (plan §Metrics XI-B6) — **impossible pre-X.I**:
//! 1,000,000 entities into ONE archetype, 15x past the old fixed 65,536-row
//! pool ceiling. Boyko pools now grow in place (eager VA reserve + lazy slab
//! commit at the frontier — no data movement, ever); Bevy realloc+memcpy's
//! the table on every doubling.
//!
//! # Workload (binding)
//!
//! ONE archetype x 3 components x 192 B each (`[u64; 24]`), 1,000,000
//! entities spawned in **100 sub-batches x 10,000** through each engine's
//! batch-spawn path. Boyko's `spawn_batch` caps one call at
//! `MAX_BATCH_HINT = 8_192`, so each 10 k sub-batch is two pushes of 5,000
//! (timed together as one sub-batch). Cold world per iteration; Bevy is NOT
//! pre-reserved (no `reserve` calls — `spawn_batch` reserves only each
//! call's `size_hint`, forcing the doubling path).
//!
//! Payload = 576 MB per engine per iteration. Worlds DROP inside
//! `iter_custom` after the clock stops: boyko pools release their
//! VmReservations when the archetype drops, Bevy frees its tables — commit
//! charge does not accumulate across iterations.
//!
//! # Measurement
//!
//! `iter_custom`, sample_size = 10. Timed region = ALL spawning (growth
//! events included); world **construction excluded** (the clock starts after
//! `EcsMaster::new()` / `World::new()` — both are us-scale noise against a
//! ~150-300 ms iteration) and world Drop excluded (the `Instant` stops
//! first). Per-sub-batch durations are recorded per iteration; the spike is
//! `max - median` over the 100 sub-batches (cancels the common per-batch
//! payload floor, isolates the growth EVENT), aggregated as the MEDIAN
//! across iterations, with the raw max + argmax index alongside for
//! attribution. `XI_DUMP_PROFILE=<dir>` dumps the first iterations' full
//! per-sub-batch CSV profiles for offline attribution.
//!
//! # Targets (binding, XI-B6)
//!
//! * `g8_boyko_growth_total` vs `g8_bevy_growth_total`: boyko **>= 1.5x**
//!   faster (model 1.7-2.1x: payload ~96 ms both sides; Bevy adds ~600 MB of
//!   doubling memcpy ~100-150 ms; boyko growth is ~80 bounded commits
//!   <= 1 ms total).
//! * Worst-sub-batch spike ratio (boyko/Bevy) **<= 0.1x**: boyko's worst
//!   sub-batch = one 64 MiB pool commit (<= 50 us, XI-B4) + the
//!   `entity_ids` Vec realloc residual (~1-2 ms at 1 M rows — the plan D10
//!   known residual); Bevy's worst = the final ~288 MB table-doubling
//!   memcpy (~50-70 ms in one sub-batch).
//!
//! On a miss, decompose per the XI-B6 model FIRST (commit events are
//! XI-B4-bounded and CANNOT explain a total-time miss).
//!
//! # Component-slot range (boyko)
//!
//! 208-210 — verified free across every bench/test binary (g7 holds
//! 152-199; the drop_fn/legacy_query suites hold 200-207).

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc. See
// docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};

// ── boyko imports ──────────────────────────────────────────────────────────
use boyko_ecs::ecs::core::component::component::Component as BoykoComponent;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── bevy imports ───────────────────────────────────────────────────────────
use bevy_ecs::prelude::Component as BevyComponentDerive;
use bevy_ecs::prelude::*;

// ── Workload constants (XI-B6, binding) ────────────────────────────────────

const TOTAL_ENTITIES: usize = 1_000_000;
const SUB_BATCH: usize = 10_000;
const SUB_BATCHES: usize = TOTAL_ENTITIES / SUB_BATCH; // 100
/// Boyko `spawn_batch` is capped at `MAX_BATCH_HINT = 8_192` per call: one
/// 10 k sub-batch = two pushes of 5,000.
const HALF_BATCH: usize = SUB_BATCH / 2; // 5_000

// ── boyko side: one bundle x 3 components x 192 B ([u64; 24]) ──────────────

#[repr(C)]
#[derive(Clone, Copy)]
struct G8A([u64; 24]);
#[repr(C)]
#[derive(Clone, Copy)]
struct G8B([u64; 24]);
#[repr(C)]
#[derive(Clone, Copy)]
struct G8C([u64; 24]);

impl BoykoComponent for G8A {
    fn component_id() -> ComponentId {
        ComponentId(208)
    }
}
impl BoykoComponent for G8B {
    fn component_id() -> ComponentId {
        ComponentId(209)
    }
}
impl BoykoComponent for G8C {
    fn component_id() -> ComponentId {
        ComponentId(210)
    }
}

#[derive(Bundle)]
struct G8Bundle {
    a: G8A,
    b: G8B,
    c: G8C,
}

fn mk_g8(v: u64) -> G8Bundle {
    G8Bundle {
        a: G8A([v; 24]),
        b: G8B([v; 24]),
        c: G8C([v; 24]),
    }
}

fn register_boyko() {
    register_layout::<G8A>(208);
    register_layout::<G8B>(209);
    register_layout::<G8C>(210);
}

/// One 10,000-entity sub-batch into the single boyko archetype (direct
/// `spawn_batch` path; the archetype + pools are created lazily on the first
/// call — pool growth events fire at the committed frontier as the
/// population climbs to 1 M rows).
fn spawn_sub_batch_boyko(world: &mut EcsMaster, base: u64) {
    for half in 0..2u64 {
        let off = base + half * HALF_BATCH as u64;
        let entities = world
            .spawn_batch((0..HALF_BATCH).map(move |i| mk_g8(off + i as u64)))
            .expect("half sub-batch of 5000 is within MAX_BATCH_HINT (8192)");
        debug_assert_eq!(entities.len(), HALF_BATCH);
    }
}

// ── bevy side: one tuple bundle x 3 components x 192 B ─────────────────────

#[derive(BevyComponentDerive)]
struct G8VA(#[allow(dead_code)] [u64; 24]);
#[derive(BevyComponentDerive)]
struct G8VB(#[allow(dead_code)] [u64; 24]);
#[derive(BevyComponentDerive)]
struct G8VC(#[allow(dead_code)] [u64; 24]);

fn mk_g8_bevy(v: u64) -> (G8VA, G8VB, G8VC) {
    (G8VA([v; 24]), G8VB([v; 24]), G8VC([v; 24]))
}

/// One 10,000-entity sub-batch into the single Bevy archetype. `spawn_batch`
/// reserves only this batch's `size_hint` (NOT pre-reserved — binding
/// fairness rule), so the per-table doubling realloc+memcpy path is
/// exercised incrementally. Dropping the returned iterator spawns all rows.
fn spawn_sub_batch_bevy(world: &mut World, base: u64) {
    let _ = world.spawn_batch((0..SUB_BATCH).map(move |i| mk_g8_bevy(base + i as u64)));
}

// ── spike recording (the g7b/R3-2 machinery) ───────────────────────────────

/// Per-iteration `(spike, raw_max, argmax_index)` triples. `spike = max -
/// median` over the 100 sub-batch durations of that iteration;
/// `argmax_index` (0..100) attributes the worst sub-batch (index 0 hits
/// archetype/pool creation; late indices implicate the largest commit steps
/// on the boyko side and the largest table doubling on the Bevy side).
static BOYKO_SPIKES: Mutex<Vec<(Duration, Duration, usize)>> = Mutex::new(Vec::new());
static BEVY_SPIKES: Mutex<Vec<(Duration, Duration, usize)>> = Mutex::new(Vec::new());

fn record_iteration_spike(sink: &Mutex<Vec<(Duration, Duration, usize)>>, durs: &mut [Duration]) {
    let argmax = durs
        .iter()
        .enumerate()
        .max_by_key(|&(_, d)| d)
        .map(|(i, _)| i)
        .expect("non-empty sub-batch record");
    durs.sort_unstable();
    let median = durs[durs.len() / 2];
    let max = durs[durs.len() - 1];
    sink.lock()
        .expect("spike sink poisoned")
        .push((max.saturating_sub(median), max, argmax));
}

fn report_spikes(side: &str, sink: &Mutex<Vec<(Duration, Duration, usize)>>) {
    let mut data = sink.lock().expect("spike sink poisoned");
    if data.is_empty() {
        return;
    }
    // Argmax distribution BEFORE sorting by spike: mode + range attribute the
    // worst sub-batch structurally.
    let mut argmaxes: Vec<usize> = data.iter().map(|&(_, _, i)| i).collect();
    argmaxes.sort_unstable();
    let mode = {
        let (mut best, mut best_n, mut cur, mut cur_n) = (argmaxes[0], 0usize, argmaxes[0], 0usize);
        for &a in &argmaxes {
            if a == cur {
                cur_n += 1;
            } else {
                cur = a;
                cur_n = 1;
            }
            if cur_n > best_n {
                best = cur;
                best_n = cur_n;
            }
        }
        (best, best_n)
    };
    data.sort_unstable_by_key(|&(spike, _, _)| spike);
    let median_spike = data[data.len() / 2].0;
    let raw_max = data
        .iter()
        .map(|&(_, max, _)| max)
        .max()
        .expect("non-empty checked above");
    println!(
        "g8b[{side}]: median-of-iteration-spikes (max - median per iteration) = \
         {median_spike:?} over {} iterations; raw max sub-batch = {raw_max:?} (untargeted); \
         argmax mode = sub-batch #{} (x{}), range {}..{}",
        data.len(),
        mode.0,
        mode.1,
        argmaxes[0],
        argmaxes[argmaxes.len() - 1]
    );
    data.clear();
}

// ── g8 — total workload wall time (iter_custom) ────────────────────────────

fn bench_g8_boyko(c: &mut Criterion) {
    register_boyko();
    c.bench_function("g8_boyko_growth_total", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let mut sub = Vec::with_capacity(SUB_BATCHES);
                // World construction EXCLUDED from the timed region
                // (XI-B6 spec; ~us-scale either way).
                let mut world = EcsMaster::new();
                let start = Instant::now();
                for batch in 0..SUB_BATCHES {
                    let base = (batch * SUB_BATCH) as u64;
                    let t0 = Instant::now();
                    spawn_sub_batch_boyko(&mut world, base);
                    sub.push(t0.elapsed());
                }
                // Stop the clock BEFORE teardown (Drop outside the timed
                // region).
                total += start.elapsed();
                // Diagnostic (attribution only, off by default): dump the
                // first iterations' full sub-batch profiles for offline
                // spike attribution. Reading an env var here is outside the
                // timed region.
                if let Ok(dir) = std::env::var("XI_DUMP_PROFILE") {
                    use std::sync::atomic::{AtomicUsize, Ordering};
                    static DUMPED: AtomicUsize = AtomicUsize::new(0);
                    let n = DUMPED.fetch_add(1, Ordering::Relaxed);
                    if n < 3 {
                        let csv: String = sub
                            .iter()
                            .enumerate()
                            .map(|(i, d)| format!("{i},{}\n", d.as_nanos()))
                            .collect();
                        let _ = std::fs::write(format!("{dir}/xi_g8_boyko_profile_{n}.csv"), csv);
                    }
                }
                record_iteration_spike(&BOYKO_SPIKES, &mut sub);
                drop(world);
            }
            total
        });
    });
    report_spikes("boyko", &BOYKO_SPIKES);
}

fn bench_g8_bevy(c: &mut Criterion) {
    c.bench_function("g8_bevy_growth_total", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let mut sub = Vec::with_capacity(SUB_BATCHES);
                let mut world = World::new();
                let start = Instant::now();
                for batch in 0..SUB_BATCHES {
                    let base = (batch * SUB_BATCH) as u64;
                    let t0 = Instant::now();
                    spawn_sub_batch_bevy(&mut world, base);
                    sub.push(t0.elapsed());
                }
                total += start.elapsed();
                if let Ok(dir) = std::env::var("XI_DUMP_PROFILE") {
                    use std::sync::atomic::{AtomicUsize, Ordering};
                    static DUMPED: AtomicUsize = AtomicUsize::new(0);
                    let n = DUMPED.fetch_add(1, Ordering::Relaxed);
                    if n < 3 {
                        let csv: String = sub
                            .iter()
                            .enumerate()
                            .map(|(i, d)| format!("{i},{}\n", d.as_nanos()))
                            .collect();
                        let _ = std::fs::write(format!("{dir}/xi_g8_bevy_profile_{n}.csv"), csv);
                    }
                }
                record_iteration_spike(&BEVY_SPIKES, &mut sub);
                drop(world);
            }
            total
        });
    });
    report_spikes("bevy", &BEVY_SPIKES);
}

// ── Criterion wiring (g7 harness conventions) ──────────────────────────────

fn configure() -> Criterion {
    Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(20))
        .warm_up_time(Duration::from_secs(3))
        .noise_threshold(0.05)
}

criterion_group! {
    name = growth_crossing_pool;
    config = configure();
    targets = bench_g8_boyko, bench_g8_bevy,
}

criterion_main!(growth_crossing_pool);
