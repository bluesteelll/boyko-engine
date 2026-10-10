//! Phase X.F — g7/g7b growth-crossing benchmark: boyko vs Bevy 0.18.1.
//!
//! THE headline X.F gate (plan §B5/B6, R2 FINAL spec, R3-2 aggregation):
//! incremental multi-archetype population that forces BOTH engines through
//! their growth machinery — boyko commits arena slabs at the frontier (no
//! data movement, one syscall per event), Bevy realloc+memcpy's each table
//! on every doubling.
//!
//! # Workload (R2 FINAL, binding)
//!
//! 16 archetypes x 3 components x 192 B each (65-256 B pool class =>
//! 65,536 rows/pool => 12 MiB/pool arena alloc), 60,000 entities per
//! archetype (<= the 65,536 pool cap — pools are fixed, X.F scope), spawned
//! in 60 sub-batches of 1,000 per archetype via `spawn_batch`, round-robin
//! across archetypes. N = 960,000 entities; payload = 553 MB per side. Cold
//! worlds both sides; Bevy NOT pre-reserved (its `spawn_batch` reserves only
//! each batch's `size_hint` => the doubling path is forced).
//!
//! boyko arena demand: 48 pools x 12 MiB = 576 MiB => D4 commit trace
//! {12, 12, 24, 48, 64x8} MiB = 12 growth events (tick buffers are heap,
//! not arena). Bevy copied rows: ~1.008 M rows ~= 581 MB of doubling memcpy.
//!
//! # Measurement (R2 N4 + R3-2, binding)
//!
//! `iter_custom`, sample_size = 10, measurement_time >= 20 s, warm-up 3 s.
//! Timed region = world construction + all spawning (direct `spawn_batch`
//! path — no command queue on either side); world `Drop` OUTSIDE the timed
//! region (the `Instant` stops before drop). Per-sub-batch durations are
//! recorded per iteration; g7b reports the per-iteration spike
//! (max - median over the 960 sub-batches, which cancels the common
//! per-batch payload floor and isolates the growth EVENT) aggregated as the
//! MEDIAN across iterations, with the raw max alongside (untargeted). The
//! tester runs >= 3 bench runs and compares medians-of-medians (X.B
//! methodology).
//!
//! # Targets (binding)
//!
//! * g7 (total): boyko >= 1.5x faster than Bevy.
//! * g7b (worst event): boyko spike <= 0.1x Bevy's spike.
//!
//! If g7 misses, decompose per the R2 model table FIRST (boyko-side suspects
//! are the tick-memset + payload terms; the arena events are B4-bounded
//! <= 0.6 ms total and CANNOT explain a miss).
//!
//! # Component-slot range (boyko)
//!
//! 152..=199 (48 slots) — a free run disjoint from every other reserved
//! range in the codebase at the time of writing (`MAX_COMPONENTS = 512`).

// Benchmark-harness reporting only: the file-static `Mutex<Vec<…>>` is the spike sink the
// Criterion routines append to once per measured iteration BATCH and drain in the summary
// print. It is never inside a timed region and never in engine code -- benches are compiled
// out of every shipping build.
#![allow(clippy::disallowed_types)]

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
use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component as BoykoComponent;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── bevy imports ───────────────────────────────────────────────────────────
use bevy_ecs::prelude::Component as BevyComponentDerive;
use bevy_ecs::prelude::*;

// ── Workload constants (R2 FINAL spec) ─────────────────────────────────────

const N_ARCHETYPES: usize = 16;
const ENTITIES_PER_ARCHETYPE: usize = 60_000;
const SUB_BATCH: usize = 1_000;
const SUB_BATCHES: usize = ENTITIES_PER_ARCHETYPE / SUB_BATCH; // 60

// ── boyko side: 16 bundles x 3 components x 192 B ([u64; 24]) ──────────────

macro_rules! def_boyko_arch {
    ($bundle:ident, $mk:ident, $reg:ident, $a:ident, $b:ident, $c:ident,
     $sa:literal, $sb:literal, $sc:literal) => {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct $a([u64; 24]);
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct $b([u64; 24]);
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct $c([u64; 24]);
        impl BoykoComponent for $a {
            fn component_id() -> ComponentId {
                ComponentId($sa)
            }
        }
        impl BoykoComponent for $b {
            fn component_id() -> ComponentId {
                ComponentId($sb)
            }
        }
        impl BoykoComponent for $c {
            fn component_id() -> ComponentId {
                ComponentId($sc)
            }
        }
        #[derive(Bundle)]
        struct $bundle {
            a: $a,
            b: $b,
            c: $c,
        }
        fn $mk(v: u64) -> $bundle {
            $bundle {
                a: $a([v; 24]),
                b: $b([v; 24]),
                c: $c([v; 24]),
            }
        }
        fn $reg() {
            register_layout::<$a>($sa);
            register_layout::<$b>($sb);
            register_layout::<$c>($sc);
        }
    };
}

def_boyko_arch!(BArch0, mk_b0, reg_b0, BA0, BB0, BC0, 152, 153, 154);
def_boyko_arch!(BArch1, mk_b1, reg_b1, BA1, BB1, BC1, 155, 156, 157);
def_boyko_arch!(BArch2, mk_b2, reg_b2, BA2, BB2, BC2, 158, 159, 160);
def_boyko_arch!(BArch3, mk_b3, reg_b3, BA3, BB3, BC3, 161, 162, 163);
def_boyko_arch!(BArch4, mk_b4, reg_b4, BA4, BB4, BC4, 164, 165, 166);
def_boyko_arch!(BArch5, mk_b5, reg_b5, BA5, BB5, BC5, 167, 168, 169);
def_boyko_arch!(BArch6, mk_b6, reg_b6, BA6, BB6, BC6, 170, 171, 172);
def_boyko_arch!(BArch7, mk_b7, reg_b7, BA7, BB7, BC7, 173, 174, 175);
def_boyko_arch!(BArch8, mk_b8, reg_b8, BA8, BB8, BC8, 176, 177, 178);
def_boyko_arch!(BArch9, mk_b9, reg_b9, BA9, BB9, BC9, 179, 180, 181);
def_boyko_arch!(BArch10, mk_b10, reg_b10, BA10, BB10, BC10, 182, 183, 184);
def_boyko_arch!(BArch11, mk_b11, reg_b11, BA11, BB11, BC11, 185, 186, 187);
def_boyko_arch!(BArch12, mk_b12, reg_b12, BA12, BB12, BC12, 188, 189, 190);
def_boyko_arch!(BArch13, mk_b13, reg_b13, BA13, BB13, BC13, 191, 192, 193);
def_boyko_arch!(BArch14, mk_b14, reg_b14, BA14, BB14, BC14, 194, 195, 196);
def_boyko_arch!(BArch15, mk_b15, reg_b15, BA15, BB15, BC15, 197, 198, 199);

fn register_all_boyko() {
    reg_b0();
    reg_b1();
    reg_b2();
    reg_b3();
    reg_b4();
    reg_b5();
    reg_b6();
    reg_b7();
    reg_b8();
    reg_b9();
    reg_b10();
    reg_b11();
    reg_b12();
    reg_b13();
    reg_b14();
    reg_b15();
}

/// One 1,000-entity sub-batch into one boyko archetype (direct path — the
/// bundle's archetype + pools are created lazily on the first call, which is
/// where the arena growth events fire).
fn spawn_boyko<B: Bundle + Send + Sync + 'static>(
    world: &mut EcsMaster,
    base: u64,
    make: fn(u64) -> B,
) {
    let entities = world
        .spawn_batch((0..SUB_BATCH).map(move |i| make(base + i as u64)))
        .expect("sub-batch of 1000 is within MAX_BATCH_HINT");
    debug_assert_eq!(entities.len(), SUB_BATCH);
}

fn spawn_sub_batch_boyko(world: &mut EcsMaster, arch: usize, base: u64) {
    match arch {
        0 => spawn_boyko(world, base, mk_b0),
        1 => spawn_boyko(world, base, mk_b1),
        2 => spawn_boyko(world, base, mk_b2),
        3 => spawn_boyko(world, base, mk_b3),
        4 => spawn_boyko(world, base, mk_b4),
        5 => spawn_boyko(world, base, mk_b5),
        6 => spawn_boyko(world, base, mk_b6),
        7 => spawn_boyko(world, base, mk_b7),
        8 => spawn_boyko(world, base, mk_b8),
        9 => spawn_boyko(world, base, mk_b9),
        10 => spawn_boyko(world, base, mk_b10),
        11 => spawn_boyko(world, base, mk_b11),
        12 => spawn_boyko(world, base, mk_b12),
        13 => spawn_boyko(world, base, mk_b13),
        14 => spawn_boyko(world, base, mk_b14),
        15 => spawn_boyko(world, base, mk_b15),
        _ => unreachable!("N_ARCHETYPES = 16"),
    }
}

// ── bevy side: 16 tuple bundles x 3 components x 192 B ─────────────────────

macro_rules! def_bevy_arch {
    ($mk:ident, $a:ident, $b:ident, $c:ident) => {
        #[derive(BevyComponentDerive)]
        struct $a(#[allow(dead_code)] [u64; 24]);
        #[derive(BevyComponentDerive)]
        struct $b(#[allow(dead_code)] [u64; 24]);
        #[derive(BevyComponentDerive)]
        struct $c(#[allow(dead_code)] [u64; 24]);
        fn $mk(v: u64) -> ($a, $b, $c) {
            ($a([v; 24]), $b([v; 24]), $c([v; 24]))
        }
    };
}

def_bevy_arch!(mk_v0, VA0, VB0, VC0);
def_bevy_arch!(mk_v1, VA1, VB1, VC1);
def_bevy_arch!(mk_v2, VA2, VB2, VC2);
def_bevy_arch!(mk_v3, VA3, VB3, VC3);
def_bevy_arch!(mk_v4, VA4, VB4, VC4);
def_bevy_arch!(mk_v5, VA5, VB5, VC5);
def_bevy_arch!(mk_v6, VA6, VB6, VC6);
def_bevy_arch!(mk_v7, VA7, VB7, VC7);
def_bevy_arch!(mk_v8, VA8, VB8, VC8);
def_bevy_arch!(mk_v9, VA9, VB9, VC9);
def_bevy_arch!(mk_v10, VA10, VB10, VC10);
def_bevy_arch!(mk_v11, VA11, VB11, VC11);
def_bevy_arch!(mk_v12, VA12, VB12, VC12);
def_bevy_arch!(mk_v13, VA13, VB13, VC13);
def_bevy_arch!(mk_v14, VA14, VB14, VC14);
def_bevy_arch!(mk_v15, VA15, VB15, VC15);

/// One 1,000-entity sub-batch into one Bevy archetype. `spawn_batch` reserves
/// only this batch's `size_hint` (NOT pre-reserved — binding fairness rule),
/// so the per-table doubling realloc+memcpy path is exercised incrementally.
/// Dropping the returned iterator spawns all rows.
fn spawn_bevy<B>(world: &mut World, base: u64, make: fn(u64) -> B)
where
    // Same bound shape as `World::spawn_batch` itself (`Effect` lives on the
    // `DynamicBundle` supertrait).
    B: bevy_ecs::bundle::Bundle<Effect: bevy_ecs::bundle::NoBundleEffect>,
{
    let _ = world.spawn_batch((0..SUB_BATCH).map(move |i| make(base + i as u64)));
}

fn spawn_sub_batch_bevy(world: &mut World, arch: usize, base: u64) {
    match arch {
        0 => spawn_bevy(world, base, mk_v0),
        1 => spawn_bevy(world, base, mk_v1),
        2 => spawn_bevy(world, base, mk_v2),
        3 => spawn_bevy(world, base, mk_v3),
        4 => spawn_bevy(world, base, mk_v4),
        5 => spawn_bevy(world, base, mk_v5),
        6 => spawn_bevy(world, base, mk_v6),
        7 => spawn_bevy(world, base, mk_v7),
        8 => spawn_bevy(world, base, mk_v8),
        9 => spawn_bevy(world, base, mk_v9),
        10 => spawn_bevy(world, base, mk_v10),
        11 => spawn_bevy(world, base, mk_v11),
        12 => spawn_bevy(world, base, mk_v12),
        13 => spawn_bevy(world, base, mk_v13),
        14 => spawn_bevy(world, base, mk_v14),
        15 => spawn_bevy(world, base, mk_v15),
        _ => unreachable!("N_ARCHETYPES = 16"),
    }
}

// ── g7b spike recording (R3-2) ──────────────────────────────────────────────

/// Per-iteration `(spike, raw_max, argmax_index)` triples, one entry per
/// workload iteration. `spike = max - median` over the 960 sub-batch durations
/// of that iteration; `argmax_index` is the global sub-batch index (0..960)
/// of the worst sub-batch — the spike-ATTRIBUTION signal (indices < 16 hit
/// archetype/pool creation in the round-robin; power-of-two entity-count
/// crossings implicate entity-metadata Vec doubling).
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
        "g7b[{side}]: median-of-iteration-spikes (max - median per iteration) = \
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

// ── g7 — total workload wall time (iter_custom) ────────────────────────────

fn bench_g7_boyko(c: &mut Criterion) {
    register_all_boyko();
    c.bench_function("g7_boyko_growth_total", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let mut sub = Vec::with_capacity(N_ARCHETYPES * SUB_BATCHES);
                let start = Instant::now();
                let mut world = EcsMaster::new();
                for batch in 0..SUB_BATCHES {
                    let base = (batch * SUB_BATCH) as u64;
                    for arch in 0..N_ARCHETYPES {
                        let t0 = Instant::now();
                        spawn_sub_batch_boyko(&mut world, arch, base);
                        sub.push(t0.elapsed());
                    }
                }
                // Stop the clock BEFORE teardown (N4: Drop outside the timed
                // region).
                total += start.elapsed();
                // Diagnostic (attribution only, off by default): dump the
                // first iterations' full sub-batch profiles for offline
                // spike attribution. Reading an env var here is outside the
                // timed region.
                if let Ok(dir) = std::env::var("XF_DUMP_PROFILE") {
                    use std::sync::atomic::{AtomicUsize, Ordering};
                    static DUMPED: AtomicUsize = AtomicUsize::new(0);
                    let n = DUMPED.fetch_add(1, Ordering::Relaxed);
                    if n < 3 {
                        let csv: String = sub
                            .iter()
                            .enumerate()
                            .map(|(i, d)| format!("{i},{}\n", d.as_nanos()))
                            .collect();
                        let _ = std::fs::write(format!("{dir}/xf_profile_{n}.csv"), csv);
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

fn bench_g7_bevy(c: &mut Criterion) {
    c.bench_function("g7_bevy_growth_total", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let mut sub = Vec::with_capacity(N_ARCHETYPES * SUB_BATCHES);
                let start = Instant::now();
                let mut world = World::new();
                for batch in 0..SUB_BATCHES {
                    let base = (batch * SUB_BATCH) as u64;
                    for arch in 0..N_ARCHETYPES {
                        let t0 = Instant::now();
                        spawn_sub_batch_bevy(&mut world, arch, base);
                        sub.push(t0.elapsed());
                    }
                }
                total += start.elapsed();
                record_iteration_spike(&BEVY_SPIKES, &mut sub);
                drop(world);
            }
            total
        });
    });
    report_spikes("bevy", &BEVY_SPIKES);
}

// ── Criterion wiring (R2 N4 harness, binding) ───────────────────────────────

fn configure() -> Criterion {
    Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(20))
        .warm_up_time(Duration::from_secs(3))
        .noise_threshold(0.05)
}

criterion_group! {
    name = growth_crossing;
    config = configure();
    targets = bench_g7_boyko, bench_g7_bevy,
}

criterion_main!(growth_crossing);
