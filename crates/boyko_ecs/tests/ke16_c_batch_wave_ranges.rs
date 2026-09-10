//! The parallel drivers' chunk RANGES, at the row counts where a `div_ceil`
//! conversion can lose or duplicate a row.
//!
//! # What this gate exists to catch
//!
//! `KE16-DESIGN-W.md` §4.2 converts both production `par_*` drivers from a
//! `while start < entity_count { spawn(...) }` walk to
//! `spawn_batch(entity_count.div_ceil(chunk_size), (0..n_chunks).map(...))`.
//! The conversion's whole correctness claim is that the closed form emits
//! **exactly** the ranges the walk emitted — `[i * chunk_size,
//! ((i + 1) * chunk_size).min(entity_count))` for every `i`. Two ways for that
//! to be wrong produce no compile error and no panic:
//!
//! * a count off by one drops the LAST chunk, so `entity_count % chunk_size`
//!   rows are silently never visited;
//! * a missing `.min(entity_count)` clamp makes the last chunk overrun, or an
//!   `n_chunks` that rounds down leaves a partial tail.
//!
//! Both are invisible at any count that divides evenly, which is exactly what
//! the existing coverage test uses (`par_chunk.rs`,
//! `parallel_disjoint_subrange_full_coverage_via_atomic_counter`: 10 000 rows
//! over 4 workers ⇒ `chunk_size = 2500`, `10000 % 2500 == 0`, the clamp never
//! fires). This file sweeps counts whose LAST chunk is short, so the clamp and
//! the round-up are both load-bearing in every assertion below.
//!
//! # Why per-row counters and not a sum
//!
//! A sum over visited rows cannot distinguish "row 4098 visited twice and row
//! 4097 not at all" from "each visited once". The oracle here is one counter
//! per row, asserted to be exactly 1 — it names the offending row index in its
//! message, which is what a `div_ceil` defect needs in order to be diagnosed
//! from the failure alone.
//!
//! Nothing here is `#[cfg]`-gated: the range arithmetic is a property the
//! drivers must have in every build, so the gate runs in every build.
//!
//! Nothing in this file needs a GPU, a window or a nightly toolchain, and it
//! carries no `#[ignore]`.

// Test oracle model: the `Vec<AtomicU32>` below is the REFERENCE visit ledger
// the engine's driver is verified against, never engine data itself. An
// integration-test target: compiled out of every shipping build.

use std::sync::atomic::{AtomicU32, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::BatchingStrategy;
use boyko_ecs::prelude::{EcsMaster, ThreadPoolBuilder};
use boyko_macros::Component;

/// One row's payload: its own index, so a visit can be attributed to a row.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Row {
    index: u32,
}

/// Worker count for every dispatch below.
///
/// Fixed rather than `available_parallelism`, because the chunk arithmetic
/// under test is a function of it: the counts in [`COUNTS`] were chosen against
/// `chunk_size = max(entity_count / 4, MIN_ARCHETYPE_FOR_PARALLEL)`, and a
/// machine-dependent divisor would silently turn a short-last-chunk case into
/// an evenly-dividing one — the very shape this file exists not to measure.
const WORKERS: usize = 4;

/// Row counts whose last chunk is SHORT under the default batching strategy at
/// [`WORKERS`] workers, plus the two boundary cases.
///
/// `BatchingStrategy::default()` gives `chunk_size = (n / 4).clamp(1024, ..)`,
/// so:
///
/// | rows | chunk | chunks | last chunk |
/// |------|-------|--------|------------|
/// | 1024 | 1024  | 1      | 1024 (the minimal wave: `n == 1`) |
/// | 1025 | 1024  | 2      | 1    (the shortest possible tail) |
/// | 4096 | 1024  | 4      | 1024 (the control: divides evenly) |
/// | 4099 | 1024  | 5      | 3    |
/// | 5001 | 1250  | 5      | 1    |
/// | 8191 | 2047  | 5      | 3    |
///
/// Rows below `MIN_ARCHETYPE_FOR_PARALLEL` (1024) take the inline path and
/// spawn no wave at all, so 1024 is the smallest count that exercises the
/// conversion.
const COUNTS: [usize; 6] = [1024, 1025, 4096, 4099, 5001, 8191];

/// The wave size the drivers must emit for `n` rows at [`WORKERS`] workers:
/// the closed form `KE16-DESIGN-W.md` §4.2 replaces the `while` walk with.
///
/// Spelled here a SECOND time, from the documented heuristic
/// (`BatchingStrategy::chunk_size`: `(n / (workers × batches_per_thread))`
/// clamped into `[min_batch_size, max_batch_size]`, `batches_per_thread = 1`,
/// `min_batch_size = MIN_ARCHETYPE_FOR_PARALLEL = 1024`,
/// `max_batch_size = usize::MAX`), rather than by calling the driver's own
/// helper — which is `pub(crate)` and, being the code under test, could not
/// disagree with itself.
fn expected_chunks(n: usize) -> usize {
    let chunk_size = (n / WORKERS).max(1024);
    n.div_ceil(chunk_size)
}

/// A world holding `n` rows of `Row { index: 0..n }` in one archetype.
fn world_with_rows(n: usize) -> EcsMaster {
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[Row::component_id()]);
    for i in 0..n {
        world
            .spawn_one(arch, Row { index: i as u32 })
            .expect("invariant: seeding the fixture archetype must succeed");
    }
    world
}

/// Asserts the ledger recorded exactly one visit for every row of `n`.
///
/// Names the first offending row rather than only the total, because the
/// diagnosis of a `div_ceil` defect is "which end of the range was lost".
fn assert_each_row_visited_once(ledger: &[AtomicU32], n: usize, driver: &str) {
    for (i, slot) in ledger.iter().enumerate().take(n) {
        let seen = slot.load(Ordering::Acquire);
        assert_eq!(
            seen, 1,
            "{driver} at {n} rows visited row {i} {seen} times, not once: the wave's chunk \
             ranges are not the ranges the per-task walk emitted"
        );
    }
}

/// `par_iter`'s wave (`par_iter.rs`, `for_each_impl`) covers every row exactly
/// once at counts whose last chunk is short.
#[test]
fn par_iter_visits_every_row_exactly_once_at_counts_whose_last_chunk_is_short() {
    for n in COUNTS {
        let mut world = world_with_rows(n);
        let ledger: Vec<AtomicU32> = (0..n).map(|_| AtomicU32::new(0)).collect();
        let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

        {
            let view = world.query::<&Row, ()>();
            pool.install(|_scope| {
                view.par_iter().for_each(|row: &Row| {
                    ledger[row.index as usize].fetch_add(1, Ordering::AcqRel);
                });
            });
        }

        assert_each_row_visited_once(&ledger, n, "par_iter");
    }
}

/// `par_for_each_chunk`'s wave (`par_chunk.rs`, `par_for_each_chunk_impl`)
/// covers every row exactly once at the same counts.
///
/// The chunk driver hands the body a SLICE rather than a row, so this also pins
/// that the slice lengths sum to `n`: an overrunning last chunk would index a
/// row that does not exist and an underrunning one would leave its tail at
/// zero.
///
/// It additionally carries this file's NON-VACUITY receipt, and it is the only
/// one of the two that can: the chunk driver hands the body one call per chunk,
/// so counting the calls reads the `n_chunks` the driver computed. `par_iter`'s
/// body is per-row and exposes no chunk identity, so its test above pins
/// coverage alone — the two drivers spell the SAME closed form (`par_iter.rs`,
/// `for_each_impl`; `par_chunk.rs`, `par_for_each_chunk_impl`) and this receipt
/// is what stops a silent sequential fallback (PAR7, "no pool attached") from
/// passing both as one inline chunk.
#[test]
fn par_for_each_chunk_visits_every_row_exactly_once_at_counts_whose_last_chunk_is_short() {
    for n in COUNTS {
        let mut world = world_with_rows(n);
        let ledger: Vec<AtomicU32> = (0..n).map(|_| AtomicU32::new(0)).collect();
        let chunks_seen = AtomicU32::new(0);
        let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

        {
            let mut view = world.query::<&Row, ()>();
            pool.install(|_scope| {
                view.par_for_each_chunk(
                    |slice: &[Row]| {
                        chunks_seen.fetch_add(1, Ordering::AcqRel);
                        for row in slice {
                            ledger[row.index as usize].fetch_add(1, Ordering::AcqRel);
                        }
                    },
                    BatchingStrategy::default(),
                );
            });
        }

        assert_each_row_visited_once(&ledger, n, "par_for_each_chunk");
        assert_eq!(
            chunks_seen.load(Ordering::Acquire) as usize,
            expected_chunks(n),
            "the dispatch at {n} rows was not the promised size: the driver must emit \
             `entity_count.div_ceil(chunk_size)` chunk bodies, and a sequential fallback \
             would report one"
        );
    }
}
