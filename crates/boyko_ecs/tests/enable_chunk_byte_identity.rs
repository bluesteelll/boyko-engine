//! Chunk-aware EnableTag filter — the load-bearing BYTE-IDENTITY gate (rev. 3).
//!
//! Proves the run-aware `for_each_chunk` / `par_for_each_chunk` /
//! `for_each_chunk_entities` drivers visit the IDENTICAL entity set in the
//! IDENTICAL order as the scalar per-row `iter()` + `with_enabled` /
//! `without_enabled` path, across the mandated bit patterns:
//!
//! - all-enabled / all-disabled / alternating
//! - single-enabled-first / single-enabled-last
//! - random-sparse (1% / 50% / 99% density, seeded)
//! - patterns spanning page boundaries (>4096 rows, 4095↔4096)
//! - patterns spanning word boundaries (multiples of 64, 63↔64)
//!
//! Both `with_enabled` (Enabled) and `without_enabled` (Disabled, incl. the
//! absent-EnableColumn archetype = one full run) polarities are covered.
//!
//! These are integration tests (out-of-crate) so they exercise ONLY the public
//! surface (`with_enabled` / `without_enabled` / `for_each_chunk` /
//! `for_each_chunk_entities` / `par_for_each_chunk`).

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::EnableTagId;
use boyko_ecs::ecs::core::iters::query::par_iter::BatchingStrategy;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, EntityId};
use boyko_ecs::prelude::{EcsMaster, Entity, Query};
use boyko_macros::Component;

/// Payload whose `v` field carries the entity's spawn index, so a visited
/// sequence of `v`s is directly comparable across the scalar and chunk paths.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct CbiPayload {
    v: u32,
}

fn arch(ecs: &mut EcsMaster) -> ArchetypeId {
    ecs.create_archetype(&[CbiPayload::component_id()])
}

/// Spawns `n` rows `v = 0..n` into one archetype (chunked, direct path).
fn spawn_rows(ecs: &mut EcsMaster, a: ArchetypeId, n: u32) -> Vec<Entity> {
    let mut out = Vec::with_capacity(n as usize);
    for v in 0..n {
        let bytes = v.to_ne_bytes();
        out.push(
            ecs.create_entity(a, &[(CbiPayload::component_id(), &bytes)])
                .expect("create_entity must succeed"),
        );
    }
    out
}

/// A tiny deterministic splitmix64 PRNG so the "random-sparse" patterns are
/// reproducible without a test-time `rand` seed dance.
struct SplitMix64(u64);
impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// `true` with probability `pct/100`.
    fn chance(&mut self, pct: u32) -> bool {
        (self.next_u64() % 100) < u64::from(pct)
    }
}

// ── Pattern generators (returns the set of enabled spawn-indices) ─────────────

fn pattern_all(n: u32) -> Vec<bool> {
    vec![true; n as usize]
}
fn pattern_none(n: u32) -> Vec<bool> {
    vec![false; n as usize]
}
fn pattern_alternating(n: u32) -> Vec<bool> {
    (0..n).map(|i| i % 2 == 0).collect()
}
fn pattern_single_first(n: u32) -> Vec<bool> {
    (0..n).map(|i| i == 0).collect()
}
fn pattern_single_last(n: u32) -> Vec<bool> {
    (0..n).map(|i| i == n - 1).collect()
}
fn pattern_random(n: u32, pct: u32, seed: u64) -> Vec<bool> {
    let mut r = SplitMix64(seed);
    (0..n).map(|_| r.chance(pct)).collect()
}
/// A run pattern straddling word boundaries (63↔64) and page boundaries
/// (4095↔4096): enabled in `[60, 70)`, `[4090, 4100)`, plus one whole page.
fn pattern_boundary_runs(n: u32) -> Vec<bool> {
    (0..n)
        .map(|i| {
            (60..70).contains(&i)
                || (4090..4100).contains(&i)
                || (4096..8192).contains(&i)
        })
        .collect()
}

// ── The oracle harness ───────────────────────────────────────────────────────

/// Sets enable bits per `pattern`, then asserts (a) scalar `iter()` and (b)
/// run-aware `for_each_chunk` yield the identical `v`-sequence, for `with_enabled`
/// (`invert=false`) and `without_enabled` (`invert=true`).
fn assert_byte_identity(label: &str, n: u32, pattern: &[bool]) {
    assert_eq!(pattern.len(), n as usize, "{label}: pattern length");
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag(label);
    let a = arch(&mut ecs);
    let ents = spawn_rows(&mut ecs, a, n);
    for (i, &on) in pattern.iter().enumerate() {
        if on {
            ecs.enable_id(ents[i], tag);
        }
    }

    for invert in [false, true] {
        let scalar = collect_scalar(&mut ecs, tag, invert);
        let chunk = collect_chunk(&mut ecs, tag, invert);
        assert_eq!(
            scalar, chunk,
            "{label} (invert={invert}): chunk sequence != scalar sequence \
             (n={n}, scalar.len={}, chunk.len={})",
            scalar.len(),
            chunk.len()
        );
        // Independent oracle: the visited multiset must equal the pattern's
        // matching indices in spawn order.
        let expected: Vec<u32> = (0..n)
            .filter(|&i| pattern[i as usize] != invert)
            .collect();
        assert_eq!(
            scalar, expected,
            "{label} (invert={invert}): scalar path disagrees with the pattern oracle"
        );
    }
}

fn collect_scalar(ecs: &mut EcsMaster, tag: EnableTagId, invert: bool) -> Vec<u32> {
    ecs.run_closure_once(move |q: Query<&CbiPayload>| {
        let mut out = Vec::new();
        if invert {
            for p in q.without_enabled(tag).iter() {
                out.push(p.v);
            }
        } else {
            for p in q.with_enabled(tag).iter() {
                out.push(p.v);
            }
        }
        out
    })
}

/// A hard cap on collected rows so a non-advancing run-extractor bug surfaces as
/// a clean test failure (panic with a clear message) instead of an OOM crash
/// that takes down the whole `cargo test` process. A correct run walk over `n`
/// rows yields at most `n` rows; any archetype here is well under 1<<20.
const COLLECT_CAP: usize = 1 << 20;

fn collect_chunk(ecs: &mut EcsMaster, tag: EnableTagId, invert: bool) -> Vec<u32> {
    ecs.run_closure_once(move |q: Query<&CbiPayload>| {
        let mut out = Vec::new();
        let mut q = if invert {
            q.without_enabled(tag)
        } else {
            q.with_enabled(tag)
        };
        q.for_each_chunk(|slice: &[CbiPayload]| {
            for p in slice {
                assert!(
                    out.len() < COLLECT_CAP,
                    "chunk driver emitted > {COLLECT_CAP} rows — non-advancing run \
                     extractor (infinite loop / overlapping runs)"
                );
                out.push(p.v);
            }
        });
        out
    })
}

// ── 1. Single-page patterns (n <= 4096) ──────────────────────────────────────

#[test]
fn byte_identity_small_all_enabled() {
    assert_byte_identity("cbi_small_all", 200, &pattern_all(200));
}
#[test]
fn byte_identity_small_all_disabled() {
    assert_byte_identity("cbi_small_none", 200, &pattern_none(200));
}
#[test]
fn byte_identity_small_alternating() {
    assert_byte_identity("cbi_small_alt", 200, &pattern_alternating(200));
}
#[test]
fn byte_identity_small_single_first() {
    assert_byte_identity("cbi_small_first", 200, &pattern_single_first(200));
}
#[test]
fn byte_identity_small_single_last() {
    assert_byte_identity("cbi_small_last", 200, &pattern_single_last(200));
}

// ── 2. Word-boundary patterns (multiples of 64, the 63↔64 seam) ───────────────

#[test]
fn byte_identity_word_boundary_exact_64() {
    assert_byte_identity("cbi_word_64", 64, &pattern_alternating(64));
}
#[test]
fn byte_identity_word_boundary_128_runs() {
    // Enabled exactly across a word seam: [62, 66) crosses 63↔64.
    let n = 128u32;
    let p: Vec<bool> = (0..n).map(|i| (62..66).contains(&i)).collect();
    assert_byte_identity("cbi_word_seam", n, &p);
}

// ── 3. Page-boundary patterns (>4096 rows, the 4095↔4096 seam) ────────────────

#[test]
fn byte_identity_page_boundary_all_enabled_one_run() {
    // A fully-enabled multi-page archetype: INV-COALESCE means one run, but the
    // visible sequence must still equal the full 0..n in order.
    assert_byte_identity("cbi_page_all", 8192, &pattern_all(8192));
}
#[test]
fn byte_identity_page_boundary_all_disabled() {
    assert_byte_identity("cbi_page_none", 8192, &pattern_none(8192));
}
#[test]
fn byte_identity_page_boundary_alternating() {
    assert_byte_identity("cbi_page_alt", 8192, &pattern_alternating(8192));
}
#[test]
fn byte_identity_page_boundary_runs() {
    assert_byte_identity("cbi_page_runs", 8192, &pattern_boundary_runs(8192));
}
#[test]
fn byte_identity_page_boundary_single_first() {
    assert_byte_identity("cbi_page_first", 8192, &pattern_single_first(8192));
}
#[test]
fn byte_identity_page_boundary_single_last() {
    assert_byte_identity("cbi_page_last", 8192, &pattern_single_last(8192));
}

// ── 4. Random-sparse densities (1% / 50% / 99%) over >3 pages ────────────────

#[test]
fn byte_identity_random_1pct() {
    let n = 13_000u32; // > 3 pages
    assert_byte_identity("cbi_rand_1", n, &pattern_random(n, 1, 0xDEAD_BEEF));
}
#[test]
fn byte_identity_random_50pct() {
    let n = 13_000u32;
    assert_byte_identity("cbi_rand_50", n, &pattern_random(n, 50, 0x1234_5678));
}
#[test]
fn byte_identity_random_99pct() {
    let n = 13_000u32;
    assert_byte_identity("cbi_rand_99", n, &pattern_random(n, 99, 0xCAFE_F00D));
}

// ── 5. without_enabled over a NO-EnableColumn archetype = one full run ────────

#[test]
fn byte_identity_without_enabled_no_column_visits_all() {
    // The tag is registered but NEVER toggled on this archetype, so there is no
    // allocated EnableColumn — `without_enabled` must visit every row (absent
    // page = all-matching), and `with_enabled` must visit none.
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("cbi_no_column");
    let a = arch(&mut ecs);
    let n = 5_000u32; // multi-page
    let _ents = spawn_rows(&mut ecs, a, n);

    let without = collect_chunk(&mut ecs, tag, true);
    let with = collect_chunk(&mut ecs, tag, false);
    let all: Vec<u32> = (0..n).collect();
    assert_eq!(without, all, "without_enabled (no column) visits every row in order");
    assert!(with.is_empty(), "with_enabled (no column) visits no row");

    // Cross-check the scalar path agrees.
    let without_scalar = collect_scalar(&mut ecs, tag, true);
    assert_eq!(without_scalar, all, "scalar without_enabled (no column) also visits all");
}

// ── 6. for_each_chunk_entities row-alignment (C1) ────────────────────────────

#[test]
fn byte_identity_for_each_chunk_entities_row_aligned() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("cbi_entities");
    let a = arch(&mut ecs);
    let n = 8_192u32; // multi-page mixed pattern
    let pattern = pattern_random(n, 40, 0xABCD_1234);
    let ents = spawn_rows(&mut ecs, a, n);
    for (i, &on) in pattern.iter().enumerate() {
        if on {
            ecs.enable_id(ents[i], tag);
        }
    }

    // Collect (entity, v) pairs from the entities chunk driver. C1: ids[k] must
    // be row-aligned with slice[k]; the payload `v` IS the spawn index, so the
    // entity at ids[k] must be ents[v].
    let pairs: Vec<(EntityId, u32)> = ecs.run_closure_once(move |q: Query<&CbiPayload>| {
        let mut out: Vec<(EntityId, u32)> = Vec::new();
        let mut q = q.with_enabled(tag);
        q.for_each_chunk_entities(|ids: &[EntityId], slice: &[CbiPayload]| {
            assert_eq!(ids.len(), slice.len(), "C1: id slice and chunk must be equal length");
            for (id, p) in ids.iter().zip(slice.iter()) {
                assert!(
                    out.len() < COLLECT_CAP,
                    "for_each_chunk_entities emitted > {COLLECT_CAP} rows — non-advancing \
                     run extractor (infinite loop / overlapping runs)"
                );
                out.push((*id, p.v));
            }
        });
        out
    });

    // Each pair must be row-aligned: the entity carrying payload v is ents[v].
    for (id, v) in &pairs {
        assert_eq!(
            ents[*v as usize].id(),
            *id,
            "C1 row mis-pairing: payload v={v} not paired with its own entity id"
        );
    }
    // The visited v-sequence must equal the scalar with_enabled order.
    let vs: Vec<u32> = pairs.iter().map(|(_, v)| *v).collect();
    let scalar = collect_scalar(&mut ecs, tag, false);
    assert_eq!(vs, scalar, "for_each_chunk_entities sequence != scalar with_enabled");
}

// ── 7. par_for_each_chunk — no double-cover, no skip (C5) ─────────────────────

/// A `BatchingStrategy` that forces a fixed small batch size (min==max==batch,
/// many batches per thread) so an archetype is split into ≥4 worker sub-ranges —
/// exercising the C5 batch terminator / no-double-cover path.
fn small_batches(batch: usize) -> BatchingStrategy {
    BatchingStrategy {
        batches_per_thread: 64,
        min_batch_size: batch,
        max_batch_size: batch,
    }
}

/// Collects the visited `v`s from `par_for_each_chunk` into a single sorted Vec
/// (parallel interleaving is legal, so we compare the MULTISET, not order). A
/// real `ThreadPool` is installed so the true parallel partitioning path runs,
/// not just the PAR7 inline fallback.
fn collect_par(ecs: &mut EcsMaster, tag: EnableTagId, invert: bool, batch: usize) -> Vec<u32> {
    use std::sync::Mutex;
    let sink = Mutex::new(Vec::<u32>::new());
    let pool = boyko_threadpool::ThreadPoolBuilder::new()
        .num_threads(4)
        .build();
    pool.install(|_scope| {
        let mut view = if invert {
            ecs.query::<&CbiPayload, ()>().without_enabled(tag)
        } else {
            ecs.query::<&CbiPayload, ()>().with_enabled(tag)
        };
        view.par_for_each_chunk(
            |slice: &[CbiPayload]| {
                let mut local: Vec<u32> = slice.iter().map(|p| p.v).collect();
                let mut guard = sink.lock().expect("sink lock");
                assert!(
                    guard.len() < COLLECT_CAP,
                    "par chunk driver emitted > {COLLECT_CAP} rows — non-advancing run \
                     extractor / overlapping batch runs"
                );
                guard.append(&mut local);
            },
            small_batches(batch),
        );
    });
    let mut v = sink.into_inner().expect("sink into_inner");
    v.sort_unstable();
    v
}

#[test]
fn byte_identity_par_for_each_chunk_matches_scalar_multiset() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("cbi_par");
    let a = arch(&mut ecs);
    let n = 10_000u32; // multi-page
    let pattern = pattern_random(n, 50, 0x9999_7777);
    let ents = spawn_rows(&mut ecs, a, n);
    for (i, &on) in pattern.iter().enumerate() {
        if on {
            ecs.enable_id(ents[i], tag);
        }
    }

    for invert in [false, true] {
        let mut scalar = collect_scalar(&mut ecs, tag, invert);
        scalar.sort_unstable();
        // Small batch forces many batches → exercises the C5 batch terminator.
        let par = collect_par(&mut ecs, tag, invert, 256);
        // No double-cover (no duplicates) and no skip (multiset equality).
        let mut dedup = par.clone();
        dedup.dedup();
        assert_eq!(dedup.len(), par.len(), "par invert={invert}: a row was double-covered");
        assert_eq!(par, scalar, "par invert={invert}: multiset != scalar (skip or double-cover)");
    }
}

#[test]
fn byte_identity_par_without_enabled_absent_page_no_double_cover() {
    // C5: without_enabled over a NO-column archetype synthesises an all-MAX span;
    // the batch end MUST clip each worker's run so a single absent-page archetype
    // yields disjoint per-batch runs (no overlap, no skip).
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("cbi_par_absent");
    let a = arch(&mut ecs);
    let n = 10_000u32;
    let _ents = spawn_rows(&mut ecs, a, n);

    let par = collect_par(&mut ecs, tag, true, 128); // sorted multiset
    let all: Vec<u32> = (0..n).collect();
    assert_eq!(par.len(), all.len(), "absent-page par: row count mismatch (skip or double-cover)");
    assert_eq!(par, all, "absent-page par must cover [0,n) exactly once");
}

// ── 8. Multi-term composite (>= 2 enable terms) — for_each_run_composite ──────
//
// Every test in sections 1-7 uses a SINGLE enable term, so `for_each_run` takes
// the `len == 1` `enabled_runs` fast path. The multi-term composite walk
// (`EnableTermCols::for_each_run_composite`, including the whole-page skip) is
// reached ONLY with >= 2 enable terms — these tests close that coverage gap. A
// row matches `with_enabled(a).with_enabled(b)` iff (onA && onB), and
// `with_enabled(a).without_enabled(b)` iff (onA && !onB).

fn build_two_tag(
    ecs: &mut EcsMaster,
    name_a: &str,
    name_b: &str,
    pa: &[bool],
    pb: &[bool],
) -> (EnableTagId, EnableTagId) {
    let n = pa.len() as u32;
    assert_eq!(pb.len(), n as usize, "build_two_tag: pattern length mismatch");
    let ta = ecs.register_enable_tag(name_a);
    let tb = ecs.register_enable_tag(name_b);
    let a = arch(ecs);
    let ents = spawn_rows(ecs, a, n);
    for i in 0..n as usize {
        if pa[i] {
            ecs.enable_id(ents[i], ta);
        }
        if pb[i] {
            ecs.enable_id(ents[i], tb);
        }
    }
    (ta, tb)
}

/// `with_enabled(a).with_enabled(b)`: run-aware `for_each_chunk` == scalar `iter`
/// == the (onA && onB) boolean oracle.
fn assert_with_with(name_a: &str, name_b: &str, pa: &[bool], pb: &[bool]) {
    let n = pa.len() as u32;
    let mut ecs = EcsMaster::new();
    let (ta, tb) = build_two_tag(&mut ecs, name_a, name_b, pa, pb);

    let scalar = ecs.run_closure_once(move |q: Query<&CbiPayload>| {
        let mut out = Vec::new();
        for p in q.with_enabled(ta).with_enabled(tb).iter() {
            out.push(p.v);
        }
        out
    });
    let chunk = ecs.run_closure_once(move |q: Query<&CbiPayload>| {
        let mut out = Vec::new();
        let mut q = q.with_enabled(ta).with_enabled(tb);
        q.for_each_chunk(|slice: &[CbiPayload]| {
            for p in slice {
                assert!(out.len() < COLLECT_CAP, "chunk composite emitted > cap rows");
                out.push(p.v);
            }
        });
        out
    });
    let expected: Vec<u32> = (0..n)
        .filter(|&i| pa[i as usize] && pb[i as usize])
        .collect();
    assert_eq!(scalar, expected, "{name_a}+{name_b}: scalar with+with != oracle");
    assert_eq!(chunk, scalar, "{name_a}+{name_b}: chunk with+with != scalar");
}

/// `with_enabled(a).without_enabled(b)`: chunk == scalar == (onA && !onB) oracle.
fn assert_with_without(name_a: &str, name_b: &str, pa: &[bool], pb: &[bool]) {
    let n = pa.len() as u32;
    let mut ecs = EcsMaster::new();
    let (ta, tb) = build_two_tag(&mut ecs, name_a, name_b, pa, pb);

    let scalar = ecs.run_closure_once(move |q: Query<&CbiPayload>| {
        let mut out = Vec::new();
        for p in q.with_enabled(ta).without_enabled(tb).iter() {
            out.push(p.v);
        }
        out
    });
    let chunk = ecs.run_closure_once(move |q: Query<&CbiPayload>| {
        let mut out = Vec::new();
        let mut q = q.with_enabled(ta).without_enabled(tb);
        q.for_each_chunk(|slice: &[CbiPayload]| {
            for p in slice {
                assert!(out.len() < COLLECT_CAP, "chunk composite emitted > cap rows");
                out.push(p.v);
            }
        });
        out
    });
    let expected: Vec<u32> = (0..n)
        .filter(|&i| pa[i as usize] && !pb[i as usize])
        .collect();
    assert_eq!(scalar, expected, "{name_a}+{name_b}: scalar with+without != oracle");
    assert_eq!(chunk, scalar, "{name_a}+{name_b}: chunk with+without != scalar");
}

#[test]
fn byte_identity_multi_with_with_small() {
    // Single page (n < 4096), 2 terms: fast under Miri, exercises the composite
    // match-word + Phase-1 permit / Phase-2 extend unsafe derefs.
    let n = 200u32;
    let pa = pattern_random(n, 50, 0xDEAD_BEEF_0000_1111);
    let pb = pattern_random(n, 50, 0x1111_0000_BEEF_DEAD);
    assert_with_with("cbi_multi_small_a", "cbi_multi_small_b", &pa, &pb);
}

#[test]
fn byte_identity_multi_with_with_page_skip() {
    // 3 pages. The middle page (rows 4096..8192) has tag A set on NO row, so A's
    // per-page summary is 0 -> the composite WHOLE-PAGE skip (page_and == 0)
    // fires. The result must stay byte-identical (zero matches in the middle
    // page), and pages 0 and 2 carry real (A && B) matches -- proving the skip
    // drops no later page. Direct validator for the for_each_run_composite
    // page-skip.
    let n = 12_288u32; // 3 full pages
    let pa: Vec<bool> = (0..n)
        .map(|i| !(4096..8192).contains(&i) && i % 3 == 0)
        .collect();
    let pb: Vec<bool> = (0..n).map(|i| i % 2 == 0).collect();
    assert_with_with("cbi_multi_pageskip_a", "cbi_multi_pageskip_b", &pa, &pb);
}

#[test]
fn byte_identity_multi_with_with_random() {
    let n = 10_000u32; // multi-page sparse composite
    let pa = pattern_random(n, 30, 0xA1A1_B2B2_C3C3_D4D4);
    let pb = pattern_random(n, 40, 0x0F0F_1E1E_2D2D_3C3C);
    assert_with_with("cbi_multi_rand_a", "cbi_multi_rand_b", &pa, &pb);
}

#[test]
fn byte_identity_multi_with_without_random() {
    let n = 10_000u32;
    let pa = pattern_random(n, 50, 0x1111_2222_3333_4444);
    let pb = pattern_random(n, 35, 0x5555_6666_7777_8888);
    assert_with_without("cbi_multi_ww_a", "cbi_multi_ww_b", &pa, &pb);
}

/// `par_for_each_chunk` over a 2-term composite split into many small batches —
/// validates the composite run walk under the parallel batch-clamping path
/// (`run_chunk_owned` -> `for_each_run` -> composite), which the single-term par
/// tests never reach.
fn collect_par_with_with(
    ecs: &mut EcsMaster,
    ta: EnableTagId,
    tb: EnableTagId,
    batch: usize,
) -> Vec<u32> {
    use std::sync::Mutex;
    let sink = Mutex::new(Vec::<u32>::new());
    let pool = boyko_threadpool::ThreadPoolBuilder::new()
        .num_threads(4)
        .build();
    pool.install(|_scope| {
        let mut view = ecs
            .query::<&CbiPayload, ()>()
            .with_enabled(ta)
            .with_enabled(tb);
        view.par_for_each_chunk(
            |slice: &[CbiPayload]| {
                let mut local: Vec<u32> = slice.iter().map(|p| p.v).collect();
                let mut guard = sink.lock().expect("sink lock");
                assert!(
                    guard.len() < COLLECT_CAP,
                    "par composite emitted > cap rows -- overlapping batch runs"
                );
                guard.append(&mut local);
            },
            small_batches(batch),
        );
    });
    let mut v = sink.into_inner().expect("sink into_inner");
    v.sort_unstable();
    v
}

#[test]
fn byte_identity_par_multi_with_with() {
    let mut ecs = EcsMaster::new();
    let n = 10_000u32;
    let pa = pattern_random(n, 35, 0x1234_5678_9ABC_DEF0);
    let pb = pattern_random(n, 55, 0x0FED_CBA9_8765_4321);
    let (ta, tb) = build_two_tag(&mut ecs, "cbi_par_multi_a", "cbi_par_multi_b", &pa, &pb);

    let mut expected: Vec<u32> = (0..n)
        .filter(|&i| pa[i as usize] && pb[i as usize])
        .collect();
    expected.sort_unstable();

    let par = collect_par_with_with(&mut ecs, ta, tb, 256);
    let mut dedup = par.clone();
    dedup.dedup();
    assert_eq!(dedup.len(), par.len(), "par composite: a row was double-covered");
    assert_eq!(par, expected, "par composite: multiset != oracle (skip or double-cover)");
}
