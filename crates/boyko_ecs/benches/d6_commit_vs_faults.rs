//! D6 micro-experiment — `vm.commit` SYSCALL vs demand-zero PAGE FAULTS.
//!
//! # The question (architect prediction: faults dominate, D6 infeasible @75%)
//!
//! `g5` spawn_batch cold (fresh world) = 219 us; warm (pool pre-grown AND
//! pre-faulted) = 149 us → a 70 us delta. The warm bench removes BOTH the
//! commit syscall AND the page faults (its pages were faulted by the warm-up
//! spawn), so 70 us = syscalls + faults COMBINED. This bench splits them, to
//! decide whether Decision 6 (VM pre-commit at setup) can claw back the 70 us:
//!
//! * `vm.commit` = one `VirtualAlloc(MEM_COMMIT)` syscall over the grown range
//!   (demand-zero; the pages become readable/writable but are NOT yet resident
//!   — they fault on first WRITE). Source: `memory/vm.rs::commit`.
//! * The grown pages are faulted on first write: ~`160 KB / 4 KB = 40` minor
//!   faults for the 10k×16 B data slab + 2×`40 KB / 4 KB ≈ 2×10` for the two
//!   tick slabs ≈ 60 page faults × ~1-3 us each.
//!
//! # Decision rule (reported by `report()`)
//!
//! * `(i) ≈ (ii) − (iii)` (commit-only ≈ cold−warm delta) → SYSCALLS dominate
//!   → D6 (pre-commit at setup, removing the syscall from the hot spawn) is
//!   potentially FEASIBLE.
//! * `(i) ≪ (ii) − (iii)` (commit-only small; the cost appears only once (ii)
//!   WRITES) → FAULTS dominate → D6-as-pre-commit is INFEASIBLE (a setup-time
//!   `MEM_COMMIT` does NOT pre-fault pages; the first write still faults them
//!   on the hot path).
//!
//! # API constraint (why this is a public-API reconstruction, not the literal
//! (i)/(ii)/(iii) the brief sketched)
//!
//! `ComponentPool::grow_rows`, `write_at_unchecked_initialized`,
//! `commit_units`, `fill_ticks` and `Archetype::reserve_capacity` are ALL
//! `pub(crate)` — UNREACHABLE from a bench binary (an external crate). The
//! only public mutation path is `add` / `add_typed`, which FUSES commit +
//! write + fault per call (see the same constraint documented in
//! `benches/pool_grow_event.rs`). So the three variants are reconstructed from
//! the public surface (`new`, `add`, `count`, `committed_rows`, `capacity`,
//! `get_raw_mut`):
//!
//! * **(i) commit-isolated** — fresh pool; fill rows `0..N` via `add`, summing
//!   ONLY the frontier-crossing adds (`count() == committed_rows()` before the
//!   add). Each such add carries one `grow_rows` → `vm.commit` syscall over the
//!   freshly-committed slab; it WRITES exactly ONE row (one first-touch fault).
//!   So (i) = the commit syscalls + a bounded handful of slab-head faults (==
//!   the number of crossings, typically 1-3 for a 10k×16 B fill — the
//!   request-dominant first step commits ~the whole slab). This is an UPPER
//!   bound on the pure-syscall cost.
//! * **(ii) commit + write (the cold path)** — fresh pool; time the ENTIRE
//!   `add` fill of rows `0..N` (every crossing + every write). = syscalls +
//!   ALL demand-zero faults + N memcpys. The g5-cold analog.
//! * **(iii) warm (pre-committed + pre-faulted)** — a pool already grown to N
//!   AND already written once (pages resident); time OVERWRITING the same N
//!   live rows in place via `get_raw_mut` + `copy_nonoverlapping`. = neither
//!   commit nor fault — pure per-row store work. The g5-warm analog.
//!
//! Then `(ii) − (iii)` = commit + faults (the write-loop store cost cancels),
//! and `(i) ≈` the commit syscalls. Comparing the two settles the question.
//!
//! # Overshoot control
//!
//! (i) also records `committed_rows()` and the committed DATA bytes
//! (`committed_rows × stride`, granule-rounded) vs the `N × stride` written
//! bytes. If commit overshoots ≥2× it inflates the syscall side — reported as
//! the overshoot ratio.
//!
//! Run: `cargo bench -p boyko-ecs --bench d6_commit_vs_faults`.
//!
//! # Component-slot reservation
//!
//! Id **449** — adjacent to `pool_grow_event`'s 448, free across the workspace.

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use criterion::{Criterion, criterion_group, criterion_main};

const D6_ID: ComponentId = ComponentId(449);

/// 16-byte POD row — matches the `pool_grow_event` / `component_pool_dense`
/// payload class and the brief's "pos-like POD, stride ~12-16 B".
#[repr(C)]
#[derive(Clone, Copy)]
struct PosLike {
    a: u64,
    b: u64,
}

const STRIDE: usize = std::mem::size_of::<PosLike>(); // 16

/// The fixed workload: 10_000 rows (the g5 batch size).
const N: usize = 10_000;

/// Reserve ceiling: the data sub-region must hold N rows with room for the
/// commit-step doubling. 1 M rows × 16 B = 16 MB — comfortably above the
/// 160 KB the N-row fill commits, so a single doubling step covers it.
const RESERVE_ROWS: usize = 1_000_000;

const COMMIT_GRANULE: usize = 64 * 1024;

#[inline]
fn pos_bytes(p: &PosLike) -> &[u8] {
    // SAFETY: PosLike is #[repr(C)] POD; the slice covers exactly STRIDE
    // initialized bytes of `p`.
    unsafe { std::slice::from_raw_parts((p as *const PosLike).cast::<u8>(), STRIDE) }
}

fn register() {
    component_registry::register_layout::<PosLike>(D6_ID.0);
}

// ── per-variant timing sinks (sample-population, mirrors pool_grow_event) ────

/// (i) commit-isolated: sum of frontier-crossing-add durations per fill.
static COMMIT_ONLY: Mutex<Vec<Duration>> = Mutex::new(Vec::new());
/// (ii) commit + write (cold path): whole-fill duration per fill.
static COLD_FILL: Mutex<Vec<Duration>> = Mutex::new(Vec::new());
/// (iii) warm: in-place overwrite of N resident rows per pass.
static WARM_OVERWRITE: Mutex<Vec<Duration>> = Mutex::new(Vec::new());
/// Overshoot: (committed_rows after fill-to-N, committed data bytes).
static OVERSHOOT: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// Fills a fresh pool to N rows via the public `add` path, timing the WHOLE
/// loop (variant ii) and, separately, the SUM of just the frontier-crossing
/// adds (variant i). Records the post-fill commit overshoot.
fn fill_fresh_pool(row: &PosLike) {
    let bytes = pos_bytes(row);
    let mut pool = ComponentPool::new(D6_ID.0, RESERVE_ROWS);

    let mut commit_sum = Duration::ZERO;
    let whole_start = Instant::now();
    for _ in 0..N {
        if pool.count() == pool.committed_rows() {
            // This add crosses the frontier → grow_rows → vm.commit fires,
            // and it writes exactly ONE row (one first-touch fault).
            let t0 = Instant::now();
            pool.add(bytes).expect("fill stays below the reserve ceiling");
            commit_sum += t0.elapsed();
        } else {
            // No syscall: this add only writes a row (faults a fresh page
            // roughly every COMMIT_GRANULE/STRIDE adds within the slab).
            pool.add(bytes).expect("fill stays below the reserve ceiling");
        }
    }
    let whole = whole_start.elapsed();

    let committed_rows = pool.committed_rows();
    let committed_data_bytes = (committed_rows * STRIDE).next_multiple_of(COMMIT_GRANULE);

    COMMIT_ONLY.lock().expect("sink poisoned").push(commit_sum);
    COLD_FILL.lock().expect("sink poisoned").push(whole);
    OVERSHOOT
        .lock()
        .expect("sink poisoned")
        .push((committed_rows, committed_data_bytes));

    // Pool drop releases the reservation; per-iteration commit charge does
    // not accumulate across iterations.
    drop(pool);
}

/// Times the warm path (variant iii): a pool pre-grown to N AND pre-faulted
/// (the build-fill below touched every page), then OVERWRITE all N live rows
/// in place — neither a commit syscall nor a page fault occurs.
fn warm_overwrite_pool(pool: &mut ComponentPool, row: &PosLike) {
    let bytes = pos_bytes(row);
    let start = Instant::now();
    for i in 0..N {
        // SAFETY: `i < N == pool.count()`, so `get_raw_mut` returns a live,
        // resident, STRIDE-aligned-for-PosLike slot; source and dest are
        // disjoint (caller stack vs pool reservation); STRIDE == layout size.
        let dst = pool.get_raw_mut(i).expect("row i < N is live");
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, STRIDE);
        }
    }
    WARM_OVERWRITE
        .lock()
        .expect("sink poisoned")
        .push(start.elapsed());
}

fn bench_d6(c: &mut Criterion) {
    register();
    let row = PosLike {
        a: 0xDEAD_BEEF_CAFE_F00D,
        b: 0x5EED_1234_9ABC_DEF0,
    };

    // (i) + (ii): fresh pool every iteration — the cold path.
    c.bench_function("d6_cold_fill_10k", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let t0 = Instant::now();
                fill_fresh_pool(&row);
                total += t0.elapsed();
            }
            total
        });
    });

    // (iii): one pool grown + faulted ONCE in setup, overwritten in the timed
    // region — the warm path. The build-fill is NOT part of the timed region.
    c.bench_function("d6_warm_overwrite_10k", |b| {
        let mut pool = ComponentPool::new(D6_ID.0, RESERVE_ROWS);
        let bytes = pos_bytes(&row);
        for _ in 0..N {
            pool.add(bytes).expect("setup fill stays below the ceiling");
        }
        // Every one of the N rows' pages is now committed AND resident.
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let t0 = Instant::now();
                warm_overwrite_pool(&mut pool, &row);
                total += t0.elapsed();
            }
            total
        });
    });

    report();
}

/// Prints the absolute timings + the (i) vs (ii)−(iii) comparison + the
/// overshoot ratio + the verdict. These printed lines ARE the experiment
/// output (grep for `d6_`).
fn report() {
    let commit_only = drain_median(&COMMIT_ONLY);
    let cold = drain_median(&COLD_FILL);
    let warm = drain_median(&WARM_OVERWRITE);

    let (overshoot_rows, overshoot_bytes) = {
        let mut sink = OVERSHOOT.lock().expect("sink poisoned");
        let v = if sink.is_empty() {
            (0, 0)
        } else {
            // median committed_rows / bytes (all iterations are identical N).
            sink.sort_unstable();
            sink[sink.len() / 2]
        };
        sink.clear();
        v
    };

    let (Some(commit_only), Some(cold), Some(warm)) = (commit_only, cold, warm) else {
        println!("d6: insufficient samples — at least one sink was empty");
        return;
    };

    // (ii) − (iii) = commit + faults (the per-row store cost cancels).
    let cold_minus_warm = cold.saturating_sub(warm);

    let written_bytes = N * STRIDE;
    let overshoot_ratio = if written_bytes == 0 {
        0.0
    } else {
        overshoot_bytes as f64 / written_bytes as f64
    };

    println!("\n========== D6: commit-syscall vs page-fault split (10k × {STRIDE} B) ==========");
    println!("(i)   commit-isolated (Σ frontier-crossing adds) = {commit_only:?}");
    println!("(ii)  cold fill (commit + write + fault)         = {cold:?}");
    println!("(iii) warm overwrite (neither)                   = {warm:?}");
    println!("(ii) − (iii) = commit + faults                   = {cold_minus_warm:?}");
    println!(
        "overshoot: committed_rows={overshoot_rows} (data {overshoot_bytes} B) vs \
         written {written_bytes} B → {overshoot_ratio:.2}× {}",
        if overshoot_ratio >= 2.0 {
            "[≥2× — inflates the syscall side]"
        } else {
            "[modest]"
        }
    );

    // Decision rule. "≈" within 1.5×; "≪" when (i) is < ~40% of (ii)−(iii).
    let i = commit_only.as_secs_f64();
    let delta = cold_minus_warm.as_secs_f64();
    let ratio = if delta > 0.0 { i / delta } else { f64::INFINITY };
    println!("\n(i) / ((ii)−(iii)) = {ratio:.3}");
    if ratio >= 0.67 {
        println!(
            "VERDICT: SYSCALLS DOMINATE — (i) ≈ (ii)−(iii). \
             D6 (pre-commit at setup) is POTENTIALLY FEASIBLE."
        );
    } else if ratio <= 0.40 {
        println!(
            "VERDICT: FAULTS DOMINATE — (i) ≪ (ii)−(iii). \
             D6-as-pre-commit is INFEASIBLE (a setup MEM_COMMIT does NOT \
             pre-fault pages; the first write still faults on the hot path)."
        );
    } else {
        println!(
            "VERDICT: MIXED — (i) is {ratio:.0}% of (ii)−(iii); neither source \
             clearly dominates. Inspect the absolutes above."
        );
    }
    println!("==============================================================================\n");
}

/// Drains a sink and returns its median (the iteration population is i.i.d.).
fn drain_median(sink: &Mutex<Vec<Duration>>) -> Option<Duration> {
    let mut v = sink.lock().expect("sink poisoned");
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let m = v[v.len() / 2];
    v.clear();
    Some(m)
}

fn configure() -> Criterion {
    // Each cold iteration = one fresh-pool 10k fill (~150-250 us) + drop;
    // 50 samples keeps wall time small while giving a stable median per sink.
    Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(5))
        .warm_up_time(Duration::from_secs(1))
}

criterion_group! {
    name = d6;
    config = configure();
    targets = bench_d6,
}

criterion_main!(d6);
