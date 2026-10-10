# Architecture: L5, a parallel narrowphase, landing after L4 (`parallel_solve` on by default) and L2 (Grid thresholds) in the same lane

Nothing was built, run or written. Tree: `D:/wt/joltab` at e2bcbcb5.

## Goal
- **L5.** Take the serial narrowphase off the critical path. Today it is 2.946 ms at W=8, 32 % of T(8) = 9.162 ms (cfg-A).
  - The manifold stream, the sensor stream, the axis-cache bytes and the pose bytes must stay **bit-identical** to the serial loop for every W, every chunk partition and every steal order.
  - Zero new heap allocations beyond the existing cost of one `pool.scope`.
- **L4.** The default world solves colours in parallel at W ≥ 2. At W=1 it keeps exactly today's path.
- **L2.** `Auto` can no longer pick Grid below the measured size-disparity crossover. Parity is unaffected (`Manual` stays the default).

## Context and constraints
- **Measured input (P0b ANALYSIS §2, §9):**
  - t_np(1) = 3.142 ms, t_np(8) = 2.946 ms, T(8) = 9.162 ms, E(8) = 0.681, ω(8) = 6.54 µs.
  - Per step: 9,561 pairs, 4,524 manifolds (672 KiB), 16,888 points.
  - Per pair: 3.142 ms / 9,561 = **0.329 µs**. 53 % of pairs early-out in SAT.
- **Properties the design relies on, read from the code:**
  - The per-pair work in `systems.rs:413-448` is a pure function of (bodies[a], bodies[b], hint).
  - Only three sites mutate shared state: the `out` push, the `sensor_out` push, and `axis_cache.set` (`:444`).
  - `probe` (`axis_cache.rs:160-179`) reads only. `set` (`:319-354`) is the only writer. `begin_frame` (`:261-281`) is the only clear.
  - `ColliderShape` is `Sphere | Box` only. **There is no capsule path in this tree.**
- **Binding rules:** principle 0; no hot-path allocation; lock-free; every gate can fail; gains gated end to end under the P0 protocol. Refactoring comes last, so the unification "no physics-local `pool.scope`" end state is not pursued here.

## Key decisions

### D1 — Output: dense per-chunk runs in one ECS-owned staging column, joined in chunk order
- **What:**
  - Chunk c owns the pair range `[lo_c, hi_c)`.
  - It writes solver manifolds **upward from `stage[lo_c]`** and sensor manifolds **downward from `stage[hi_c − 1]`**. At most one manifold per pair gives `n_out + n_sen ≤ hi − lo`, so the two runs cannot meet.
  - After the join, the calling thread appends each chunk's out-run to `manifolds` and its sensor run (read in reverse) to `sensor_overlaps`, in ascending c.
- **Why:**
  - Pair order is output order, and contiguous chunks reproduce it for any partition.
  - Merging costs O(C) bookkeeping plus one O(M) copy. There is no key, no sort, no per-pair flag scan, and no route recomputation from bodies.
- **Alternatives rejected:**
  - Jolt-style atomic append plus a sort key: an O(M log M) serial sort and a hash key.
  - Avian-style per-thread buffers plus a sort by pair index: needs worker identity and a sort.
  - Unification §10.4's per-pair slots plus prefix compaction: 4.5k strided gathers plus a 9.5k flag scan, where D1 does 48 run copies. D1 is §10.4 at chunk granularity.
  - Writing straight into `manifolds` and compacting in place with memmove:
    - It needs a new public `unsafe set_len` in `boyko_ecs`.
    - Or it pays a `resize` fill of about 760 KiB every step, because that column's length must equal the manifold count.
    - It saves only 1/C of the copy.
  - Unity-style per-chunk blocks as the consumer format: every consumer of the `manifolds()` slice would have to change.
- **Trade-off:**
  - +1.39 MiB committed for J (9,561 × 152 B).
  - A serial 672 KiB copy on the calling thread. By my arithmetic, 27–45 µs at 15–25 GB/s; the `phys_np_compact` zone will measure it.

### D2 — Hints come from the live table; nothing writes it during the phase
- **What:**
  - On a non-prefetched frame, workers `probe` the live slots.
  - On a prefetched frame (rows changed), they read the `remapped` column that `begin_frame_synced` already filled serially.
- **Why:** Lemma 1 (below) shows that the serial loop's read at step k equals a read of the table as it stood before the loop. So the "pre-step snapshot" of review item O3 costs nothing.
- **Alternatives rejected:**
  - Making `prefetch_remapped` unconditional, as the plan text says: a serial pass of 9.5k probes every step, which Lemma 1 makes unnecessary.
  - Jolt's `mCache[2]` double buffer: twice the memory, plus a copy or re-key every step.
  - Concurrent CAS inserts: slot layout would depend on timing. Lookups would be equal but the table bytes would not, which weakens the byte gate.

### D3 — The axis commit is serial and in pair order, after the join
- **What:** each chunk writes a per-pair `u8` `commit[k]`: the axis to set, or `AXIS_NONE`. After compaction, `commit_axes` walks k ascending and calls `set` for every non-`NONE` entry.
- **Skip rule, exact:** a worker writes `AXIS_NONE` when `!prefetched && hint == Some(new_axis)`. In the serial loop that `set` finds its own key and rewrites the same value, so it is a byte no-op (Lemma 2).
- **Why:** `set` is the only writer. Serial pair order reproduces today's table bytes and `occupied` exactly. Cost is one 9.5 KB scan plus `set`s for changed or new keys only.
- **Rejected:** any parallel commit, because linear-probe inserts race.

### D4 — Chunking and dispatch follow the solver's scope pattern
- **Chunk count:** `C = min(lanes × NP_CHUNKS_PER_LANE (6), n / NP_MIN_PAIRS_PER_CHUNK (128))`, clamped to ≤ `NP_MAX_CHUNKS` (256).
- **Inline instead of dispatching** when:
  - `C < 2`;
  - `lanes < 2`;
  - there is no pool;
  - the flag is off;
  - or `n > np_stage.capacity()` (a `#[cold]` fallback; the ceiling is 7.06M rows natively, 27.6k under Miri).
- **Cuts:** closed form `lo = i·n/C`, equal pair counts, spawned in a `scope.spawn` loop (`colored.rs:3040-3107` shape). The calling worker participates in the join.
- **Why:**
  - ×6 oversubscription is the solver's measured setting for stealing, and it absorbs the 53 % early-out imbalance.
  - 128 pairs × 0.329 µs = 42 µs, at Box2D's 40 µs task floor.
  - At W=8 this gives 48 chunks of about 199 pairs (about 65 µs each). At W=16 it gives 74 chunks (43 µs).
- **Rejected:**
  - Cost-weighted cuts: there is no per-pair cost model outside SAT.
  - An atomic block cursor: the Chase-Lev pool already steals, and L6's region primitive is not built.

### D5 — A new flag, `PhysicsConfig::parallel_narrowphase`
- It is a request; the dispatch still needs ≥ 2 lanes and ≥ 2 chunks.
- Default `false` in C3 (dormant) and `true` in C4.
- **Why:** it isolates L5 for census controls and gives a same-binary A/B.
- **Rejected:**
  - Reusing `parallel_solve`: it couples two census structures and cannot A/B L5 alone.
  - Always on: S1b needs an off switch.

### D6 — Today's serial loop stays as the oracle and the W=1 path
- Both paths call one `#[inline] fn collide_pair` (the extracted match at `:413-448`).
- The serial path keeps its interleaved `set`. So at W=1 the only new work is one thread-local pool probe per step.

### D7 — Chunk metadata lives on the stack
- A function-local `[AtomicU64; 256]`, written once per chunk as `(n_out << 32) | n_sen` with a `Relaxed` store, and read after the join. The scope join provides happens-before.
- No heap, no `unsafe`, no ECS column; the rules allow transient function-local scratch.
- Not padded: 48 single stores per step.

### D8 — The scope cost is accepted
- At W ≥ 2 each step adds 1 `Box<ScopeShared>` plus c 4 KiB blocks, with c measured (expected 1–2).
- This is the solver's existing per-dispatch cost class (133 scopes per step in S1c). The zero-allocation reusable scope is the already-filed follow-up.
- **This is the lane's one exception to "no hot-path allocation", and it is stated, not hidden.**

### D9 — Instruments sit on the calling thread only (review O1)
- Zones `phys_np_dispatch` (spawn plus join), `phys_np_compact` and `phys_np_axis_commit`.
- Counter `phys_np_chunks` (0 when inline). No zone inside a chunk task.

### D10 — The SDF stage is designed but not built (see §SDF)

### L4 — `parallel_solve` on by default, plus a `lanes ≥ 2` term in the whole-step gate
- **Why:**
  - At W=1 there is no shortcut today. With the flag on, every wide colour opens a scope (J-P1 +0.60 %; ω₁·waves predicts about 93 µs).
  - The new term makes W=1 literally today's path. It reads one thread-local per step.
- **Rejected:** a per-colour `lanes < 2` check (109 reads per step, where one per step suffices).

### L2 — `GRID_HI = 3_000`, `GRID_LO = 2_700`
- **HI:** the measured disparity crossover (2,978) to two significant figures, which is all the precision a K=1 measurement in the `bench` profile supports.
- **LO:** a 10 % dead band. A Grid-held disparity scene inside the band loses at most about 1.13× on broadphase (interpolated from the §8 slope).
- **Price, stated:** on uniform scenes between 1,109 and 3,000 bodies, Auto keeps AllPairs where Grid would be 1.8–2.7× faster.
- **Rejected:**
  - HI at the uniform crossover (~1,109): the pyramid scene would take Grid at +3.07 ms per step.
  - A disparity classifier: only Grid knows about oversized bodies, only from the previous build and only while it is running. Two measured families cannot calibrate a threshold, so it would reintroduce the `[ESTIMATE]` defect L2 exists to remove.
- The better policy belongs to the separately designed broadphase redesign.

## Data structures
```rust
// resources.rs — Manifolds (the same ResMut; no new resource, no scheduler change)
pub struct Manifolds {
    pub(crate) manifolds: ScratchColumn<Manifold>,       // unchanged, 152 B/row (const-asserted)
    pub(crate) sensor_overlaps: ScratchColumn<Manifold>, // unchanged
    pub box_axis_cache: BoxAxisCache,
    pub(crate) np_stage: ScratchColumn<Manifold>,        // NEW, id = narrowphase_column_id(3). Length only grows:
                                                         //   resized to max(len, n_pairs), filled only on growth.
                                                         //   Stale rows are never read (compaction reads written runs only).
    np_dispatches: u64,                                  // NEW, monotonic, calling thread only. Structural witness.
}
// narrowphase/axis_cache.rs — BoxAxisCache gains
    commit: ScratchColumn<u8>,  // NEW, id = narrowphase_column_id(4); per-pair axis or AXIS_NONE; length only grows

#[derive(Clone, Copy)]            // auto Send + Sync: shared slices of POD
pub(crate) struct AxisHints<'a> { slots: &'a [AxisEntry], mask: usize, shift: u32, remapped: &'a [u8], prefetched: bool }

#[derive(Clone, Copy)]            // auto Send + Sync (ScratchSolveView has existing unsafe impls; AtomicU64: Sync)
pub(crate) struct NpChunkCtx<'a> {
    bodies: &'a [BodyState], pairs: &'a [(BodyIndex, BodyIndex)], hints: AxisHints<'a>,
    stage: ScratchSolveView<'a, Manifold>, commit: ScratchSolveView<'a, u8>, meta: &'a [AtomicU64],
}
```
- **Scratch ids:** `NARROWPHASE_COLUMN_COUNT` goes from 3 to 5 (`scratch_ids.rs:889`).
  - The broadphase + narrowphase union grows from 17 to 19 wide, still ≤ 64 (`:929-932`).
  - 36 ids of headroom remain (`:726`).
  - The stage is swept beside `pairs`, `commit` and the axis slots in the chunk loop, and beside `manifolds` and `sensor_overlaps` in the compaction. All of these sit in the one cohort, so their stagger slots are distinct.
  - The compile-time assert at `:680-688` (axis carry versus the union) must still hold. If it fires, derive `SCRATCH_ID_AXIS_REMAP` with `highest_id_clear_of` against the union.
- No new `unsafe impl`. No new `Drop` type. Everything is POD.

## Public API
```rust
pub struct PhysicsConfig { /* … */ pub parallel_narrowphase: bool }   // C3: false, C4: true
impl Manifolds { pub fn narrowphase_dispatches(&self) -> u64; }        // diagnostic, O(1)
impl BoxAxisCache { pub fn fingerprint(&self) -> u64; }                // cold: FNV over slot bytes + occupied, for gates
pub const NP_CHUNKS_PER_LANE: usize = 6;  pub const NP_MIN_PAIRS_PER_CHUNK: usize = 128;  pub const NP_MAX_CHUNKS: usize = 256;
// profiling: PHYS_NP_DISPATCH, PHYS_NP_COMPACT, PHYS_NP_AXIS_COMMIT (span); PHYS_NP_CHUNKS (counter)
pub const GRID_LO: u32 = 2_700;  pub const GRID_HI: u32 = 3_000;       // values change
```
Everything else is `pub(crate)`:
- `collide_pair`;
- `dispatch::{try_parallel, np_chunk, compact}`;
- `BoxAxisCache::{dispatch_views, commit_axes}`.

## Algorithms for critical paths
```text
physics_narrowphase(scratch, pairs, cfg, manifolds):        // + Res<PhysicsConfig>; ordered after broadphase already
  prefetched = axis_cache.begin_frame_synced(...)           // unchanged, serial
  c = cfg.parallel_narrowphase ? try_parallel(...) : 0
  if c == 0: serial loop (today's semantics, via collide_pair)
  counter PHYS_NP_CHUNKS = c; PHYS_NP_* as today, computed from manifolds after the merge

try_parallel:  meta = [AtomicU64; 256]
  zone DISPATCH: try_with_active_pool(|pool| {
      c = chunk_count(n, pool.num_threads()); if c < 2 { return 0 }
      grow np_stage and commit to at least n (fill only on growth, cold)
      (hints, commit) = axis_cache.dispatch_views(prefetched, n); stage = np_stage.solve_view()
      pool.scope(|s| for i in 0..c { s.spawn(move || np_chunk(ctx, i, i*n/c, (i+1)*n/c)) }); c })
  zone COMPACT: clear manifolds and sensor_overlaps; for i in 0..c: (o, s) = meta[i];
      out.extend_from_slice(stage[lo..lo+o]); for j in 0..s: sensor.push(stage[hi-1-j])
  zone AXIS_COMMIT: for k in 0..n where commit[k] != NONE: set(pairs[k], commit[k])
  np_dispatches += 1

np_chunk(ctx, i, lo, hi): wo = lo; ws = hi
  for k in lo..hi: h = hints.read(k, a, b); (m, ax) = collide_pair(...)
    commit[k] = match ax { Some(x) if !(!prefetched && h == Some(x)) => x, _ => NONE }
    if m.count > 0: if sensor(a) || sensor(b) { ws -= 1; stage[ws] = m } else { stage[wo] = m; wo += 1 }
  debug_assert!(wo <= ws); meta[i].store((wo-lo) << 32 | (hi-ws), Relaxed)
```

**Cost and cache behaviour per phase:**
- **Chunk phase.** O(n/W) per lane.
  - Pairs are sorted by (min, max), so `bodies[a]` stays hot, as in the serial loop.
  - Stage writes are dense and sequential per chunk. Commit bytes are sequential.
  - The axis table for J is 32,768 × 16 B = 512 KiB of shared random reads, which fits one core's L2.
  - Branch profile is as today. The sensor branch is always not-taken in a sensor-free world.
  - No new SIMD: SAT is scalar, as today.
- **Compaction.** O(M) copy, serial, streaming (672 KiB on J). Lines move from the workers' L2 to the caller.
  - A parallel scatter becomes worth building if `phys_np_compact ≥ 1 %` of T(8). That threshold is an open question.
- **Axis commit.** A 9.5 KB scan plus `set` for changed or new keys only. Upper bound: every `Some` result, about 4.5k × ~20 ns ≈ 90 µs (estimate). The zone measures it.

## Determinism argument
Notation: T₀ is the table after `begin_frame_synced`, and key_k is the pair's packed key. Keys are unique per frame because broadphase pairs are unique.

- **Lemma 1 (hint invariance).** On a non-prefetched frame, the serial loop's `probe(T_k, key_k)` equals `probe(T₀, key_k)`.
  - T_k differs from T₀ only by `set(key_j)` for j < k, with key_j ≠ key_k. Each such `set` either rewrites the axis field in key_j's own slot or fills an `EMPTY` slot with key_j. Entries never move. Only `begin_frame` clears, and it runs before the loop.
  - If key_k is in T₀ at slot s: every slot from home to s is non-`EMPTY` and keeps its key, and only `set(key_k)` writes slot s. So the probe finds key_k with the same axis.
  - If key_k is not in T₀: no slot ever receives key_k, so the probe still returns `None` (at an `EMPTY` slot, or after mask + 1 probes).
  - On a prefetched frame the hint is `remapped[k]`, a column the loop never writes.
  - Therefore every hint is a function of T₀ alone, and `collide_pair` can run per pair in any order on any thread.
- **Lemma 2 (commit equivalence).** The parallel path applies the serial `set` sequence in the same order, minus the skipped ones. For a skipped k, key_k is in T₀ at a slot s that holds h = x by Lemma 1, and only `set(key_k)` writes s. So the serial `set` at k writes the equal value and leaves `occupied` alone, which is a byte identity. Removing identity steps leaves the final bytes and `occupied` equal.
- **Lemma 3 (stream equivalence).**
  - The cuts partition [0, n) into ascending contiguous ranges.
  - Within a chunk, the out-run is in ascending k by address, and the sensor run is in ascending k by descending address, which the reverse read restores.
  - Runs are joined in ascending chunk index.
  - The routing predicate is the serial one, `is_sensor(a) || is_sensor(b)`, evaluated in the chunk.
  - So both streams equal the serial streams, and each manifold's bytes are `collide_pair`'s output.
- **Theorem.** For any W, any partition and any steal order, the manifold stream, the sensor stream, the table bytes and `occupied` all equal the serial path's. Everything downstream (graph, solve, sleep keys) sees the same inputs, so the pose bytes are equal.
- **Floating point.**
  - Same binary; IEEE operations only. rustc/LLVM never contract to FMA without flags.
  - The two inlined copies of `collide_pair` compute the same bits.
  - No crate sets MXCSR (grep for `setcsr|mxcsr|FLUSH_ZERO`: 0 sites), so every pool thread starts with the OS default. A threadpool test pins this (G-FP).

## Multithreading model
- **Shared and read-only during the scope:** bodies, pairs, axis slots, `remapped`.
  - Their writers are serial and on the calling thread, before or after the scope (`begin_frame_synced`, `commit_axes`).
  - `ResMut<Manifolds>` gives the system exclusive access through the scheduler.
- **Exclusive per chunk:**
  - stage rows `[lo_c, hi_c)` (writes land only at `[lo_c, wo) ∪ [ws, hi_c)`);
  - commit bytes `[lo_c, hi_c)`;
  - `meta[c]`.
  - The cuts are a partition, so no two tasks write the same element.
- **Synchronisation:** `spawn` publishes the captures, and the scope's `Drop` join acquires every task's effects. Both are covered by the pool's own loom models. `meta` uses `Relaxed` stores and loads ordered by that join. Loom is N/A: there is no new protocol.
- **Tree Borrows:**
  - Workers write through `ScratchSolveView::row_ptr`, whose provenance comes from `solve_base`.
  - No `&[T]` or `&mut [T]` over the stage or commit column exists until after the join, when `as_read_slice` and the build views are taken. This is the Grid emit's rule (`resources.rs:1917-1919`).
- **`unsafe`:** three sites in `np_chunk` (out write, sensor write, commit write), each with a `// SAFETY:` citing the cut partition, `n ≤ view.len()`, no concurrent reader, and write provenance from `solve_base`.
- **False sharing:** only at chunk boundaries (one 64 B line per boundary, touched by two writers), about 48 lines per step. Not padded.

## Expected gain (arithmetic from P0b, not measurement)

| row | W | prediction | basis |
|---|---|---|---|
| J-A | 8 | **Δt_np = −2.22 … −2.37 ms (24–26 % of T(8) = 9.162)** | parallel part 3.142 / (8 × 0.681) = 0.577 ms; serial residue 0–0.15 ms (dispatch ~6.5 µs + compaction 27–45 µs + commit ≤ 90 µs + scan); today's t_np(8) = 2.946 |
| J-A | 2 / 4 | −0.68…−1.53 / −1.84…−2.32 ms | E ∈ [0.681, 1]; not claimed |
| J-A | 16 | ≈ W=8 | SMT; T(16) = T(8) today; not claimed |
| J-A | 1 | 0 | inline path |
| R | 8 | ≈ −2.6…−2.9 ms | t_np,R(1) = 3.63 ms; I(8) unknown for R; not claimed |
| J-Son | 8 | ≈ −2.2 ms of the 5.4 ms sleeping floor | side effect; ungated |

**L4** (`--cfg default` rows, from ANALYSIS §9):
- J-D at W=8: −2.62 ms (P(1) = 3.576 with simd → 0.447 + L(8) 0.510).
- R at W=8: −3.87 ms (5.27 → 1.40).
- W=1: 0 on both.

**L2:** 0 on every row (`Manual` is the default).

## SDF stage (designed, not built in this lane)
- **Shape if built:** the same machinery over body rows.
  - `np_stage` grows to at least the row count, and the chunks cover rows.
  - Out and sensor runs as in D1. There is no axis commit.
  - The compaction appends after the body-body streams, preserving today's row order.
  - Bit-identical by Lemma 3, since each body's SDF manifold is a pure function of (body, field, kernel).
  - Prerequisites: check `SdfField: Sync`, and keep `sdf_narrowphase` read once per step. Its default `Scalar` does not change.
- **Why it is not built:**
  - Neither the runner nor any shipped app registers it, so t_sdf has never been measured and the build-if rule cannot be evaluated.
  - **Build-if:** a runner row with a field shows t_sdf(1) ≥ 5 % of T(8).

## Sleeping and counters
- **Sleeping.** Every pair is still visited, frozen islands included, and the stream is identical.
  - So the island contact key (`resources.rs:3491-3549`) and the B1 frozen warm carry are unchanged.
  - L10's later frozen-pair skip is a per-pair predicate that goes in front of `collide_pair` on both paths and keeps the order.
- **Counters.**
  - `PHYS_NP_*` are computed after the merge, on the calling thread, with unchanged values.
  - The `box_box.rs:746-756` test thread-locals stay thread-local. Their only reader calls `box_box_contact` on the test thread (`:2063-2070`).
  - C3 adds a doc note there: a system-level reader must run at W=1 or with the flag off.

## Integration (file:line)
**C1 — L4**
- `resources.rs:267` and `:281-287`: doc now says "default `true`"; W=1 takes the inline path.
- `resources.rs:498-501`: `parallel_solve: true`, with the comment.
- `colored.rs:3544-3545`: the gate becomes `config.parallel_solve && widest ≥ MIN && try_with_active_pool(|p| p.num_threads() >= 2) == Some(true)`.
- `benches/jolt_parity_pyramid.rs:61-62`: doc says `--cfg default` now has `parallel_solve` on.

**C2 — L2**
- `broadphase_policy.rs:66-90`: the constants; the docs change from `[ESTIMATE]` to `[MEASURED 2026-09-19, P0b §8, bench profile, K=1]`; plus `const _: () = assert!(GRID_HI >= 2_978)`, which cites the measurement.
- `tests/broadphase_select_p3.rs`: the sizes are derived from the constants, so the lattices grow to about 3k spheres. Five tests get `#[cfg_attr(miri, ignore = "miri-slow: …")]`, and one new test is added (G-L2-1).

**C3 — L5 code, dormant (flag `false`)**
- `scratch_ids.rs:887-952`: count 3 → 5; `np_stage_id()` and `axis_commit_id()`; layouts registered.
- `resources.rs:2268-2327`: new fields and constructor.
- `PhysicsConfig`: the flag and its doc, beside `:288`; the default beside `:501`.
- `narrowphase/axis_cache.rs`:
  - `:187-236`: the commit field and constructor;
  - new `AxisHints`, `dispatch_views`, `commit_axes`, `fingerprint`;
  - `:15-31`: module docs gain the parallel read/commit protocol and Lemmas 1–2.
- New `narrowphase/dispatch.rs`: constants, `NpChunkCtx`, `np_chunk`, `compact`, `try_parallel`, plus the unit test, proptest and Miri test. `narrowphase/mod.rs`: `pub(crate) mod dispatch;`.
- `systems.rs:375-478`: extract `collide_pair` (`:413-448`); serial path unchanged; add `Res<PhysicsConfig>`; the dispatch call; counters.
- `profiling.rs:75-131`: 3 zones, 1 counter; `SPAN_ZONES` 14 → 17, `COUNTER_ZONES` 6 → 7; the doc tables.
- `box_box.rs:746-756`: the doc note.
- Runner:
  - `configure` (`:723-749`): cfg-A sets `parallel_narrowphase = parallel`; new flag `--parallel-np on|off`.
  - Flags doc `:121-143`.
  - The W4 expectation for the new zones is 1 each when the recomputed C ≥ 2, else 0. The runner re-implements `chunk_count` from the exported constants; a mismatch voids the step.
  - The SUMMARY JSON (`:1160-1170`) gains the flag.
- New `boyko_threadpool/tests/mxcsr_uniform.rs` (G-FP).

**C4 — L5 on by default**
- `PhysicsConfig` default `true`.
- `alloc_frame_census.rs`:
  - arm `:1767-1768` sets `parallel_narrowphase = parallel`, so S1b stays the control;
  - the structural assert `:1813-1821` becomes `(scope − 1 − Δnp_dispatches) % passes == 0`;
  - pins and docs `:295-334` and the pin table (`:2269-2300`, checks `:2380-2430`).
- `default_world_worker_invariance.rs:191-268`: add flag arms.
- New `tests/narrowphase_parallel_equivalence.rs`.
- The rest of C4's red set (see "What moves").

## Commit sequence (each commit green under `cargo test --workspace --all-targets --no-fail-fast`, clippy `-D warnings`, Miri leg)
1. **C1 — L4.** Flip plus the gate term; red-first test G-L4-1; the red set handled.
2. **C2 — L2.** Constants, test resizing, G-L2-1.
3. **C3 — L5 dormant.**
   - All code, the unit tests, proptest and Miri test, and `narrowphase_parallel_equivalence.rs` with the flag forced on.
   - Zones and runner.
   - No default world changes: census and pins are untouched.
4. **C4 — L5 on.** Default flip; the census and allocation re-pins; docs.
5. **Timing window** (owner confirms the quiet window first): L4 = C0 vs C1; L5 = C2 vs C4.

## Gates (each can fail)

| gate | what | how it can fail |
|---|---|---|
| G-L4-1 | a warmed colored step on a **1-worker** pool with `parallel_solve` on allocates **0** (the `dispatch::warmed_parallel_step_allocs(…, 1)` helper) | **red-first on C0** (a scope opens at W=1) |
| G-L4-2 | `assert!(PhysicsConfig::default().parallel_solve)` in G3; G3's existing {1,2,4,8} × {off,on} hash equality | mutation: revert the default → red |
| G-L2-1 | `auto_keeps_all_pairs_on_the_jolt_pyramid`: 1240 boxes + slab, `Auto`, one step ⇒ kind is AllPairs, band off | **red-first at 96/192** (selects Grid) |
| G-L5-1 | proptest (native): random box/sphere/sensor scenes, random pre-filled table (small table ⇒ home collisions), random `prefetched` with a `remapped` column, **random contiguous partitions** ⇒ out stream, sensor stream, table bytes and `occupied` equal to the serial path | named mutations, each compiles and must red: **M1** join chunk runs in descending c; **M2** drop `!prefetched` from the skip rule; **M3** commit in descending k (table bytes); **M4** read the sensor run forward; **M5** skip `commit_axes`; **M6** write the stage at `k` instead of `wo` (reads stale rows) |
| G-L5-2 | Miri, live: `np_chunk` on `std::thread::scope` with 2–3 chunks over about 30 pairs, then compact and commit, equal to serial | mutation: derive the stage base from `as_read_slice().as_ptr().cast_mut()` (the SP4 shape) ⇒ Miri reports a Tree Borrows error |
| G-L5-3 | `narrowphase_parallel_equivalence.rs`, `cfg(not(miri))`: about 300 boxes (≥ 2k pairs) with spawn/despawn churn every 7 steps, mixed shapes, sensors; W ∈ {1,2,4,8,16} × flag; per step, stream / sensor / `fingerprint()` / pose hashes equal the W=1 flag-off oracle. Non-vacuity asserted: ≥ 1 prefetched frame, ≥ 1 load-clear, ≥ 1 grow, box-sphere flips present, sensors present, `narrowphase_dispatches` +1 per step on W ≥ 2 flag-on arms. A `slow:`-ignored arm runs the J pyramid for 600 steps at W ∈ {1,8,16} | M1 at system level; churn non-vacuity reds if the scene stops moving rows |
| G-L5-4 | G3 with flag arms plus the dispatch-counter non-vacuity | M1 |
| G-L5-5 | census: S1c scope 133 → 134 exact, chunks 229 → 229 + c (**measured**, pinned to the long run's envelope under the file's rule); S1b unchanged 1/1 | **red-first:** C4 without the S1b arm edit reds S1b's "1 exact" |
| G-FP | every pool worker's MXCSR equals the spawner's | mutation: the spawner sets FTZ before `build` ⇒ red |
| G-TW | P0 protocol (median of K process means, SE claim rule, receipts, canary). L4 rows: J-D, R; C0 vs C1. L5 rows: J-A, J-D, R; C2 vs C4. W ∈ {1,2,4,8,16}; K=6 (K=12 at W=1). Armed J-A at all five W on C2 and C4 (spans, I(W), E(W); fills P0's missing W=2/4/16). Attribution: C4 `--parallel-np off` vs on, J-A, W=8. J-C canary at W=1 and 8 on C4. `--expect-pose`: **one pose hash per row across C2, C4 and every W** (exit 4 otherwise). About 64 cells, about 460 processes | L5's rule: realized ΔT(8) on J-A ≥ 0.6 × 2.22 = **1.33 ms**, claimed, else the prediction is refuted and investigated before merge. A claimed regression at any W blocks. The canary must be seen. I(W) reported before and after. W4 voids |

**Debug assertions:**
- `wo ≤ ws`;
- the cuts partition [0, n);
- `C ≤ NP_MAX_CHUNKS`;
- `stage.len() ≥ n`;
- after compaction, `manifolds.len() + sensor.len() == Σ meta`;
- `commit[k] < 15 || commit[k] == AXIS_NONE`.

## What moves, and how each is re-measured
- **C1:**
  - Tests that build default-config worlds on pools of 2 or more workers and count allocations or scopes. Candidates: `colored_parallel_alloc_o6.rs`, `large_island_gate_p2.rs`, `alloc_frame_attribution.rs`, `constraint_graph_o4_world.rs`, `broadphase_grid.rs`, `soft_*_alloc.rs`. The authoritative list is C1's red set.
    - A serial-path control sets `parallel_solve = false` explicitly.
    - A default-world pin is re-measured under its own file's rule.
  - A test asserting a dispatch at W=1 loses its subject and is re-stated.
  - **No pose, golden or hash pin may move.** A red there is a defect.
- **C2:** the p3 scene sizes (about 3k spheres) and five Miri ignores. `tests/ignore_reasons_census.rs` is re-blessed if it pins counts. No values move.
- **C4:**
  - S1c scope and chunk pins, the structural assert, and the census doc sentence "every other physics system … ZERO heap acquisitions" (`:333-334`): the narrowphase now owns 1 scope + c chunks when it dispatches.
  - `alloc_frame_attribution.rs` class and site attribution.
  - Other default-world allocation pins in the red set.
  - Again, no value pin may move.
- **Runner:** cfg-A now also parallelises the narrowphase at W > 1. P0's cfg-A rows are comparable only as the "before" of this lane's pairing, never quoted against post-L5 rows directly.

## What it cannot claim
- Any change to the serial AllPairs broadphase (1.95 ms at W=8), which is now the largest serial stage. It remains 21 % of T(8) and belongs to the redesign.
- Any E(W) or gain at W=2, 4 or 16 before G-TW measures it. Any W=16 gain beyond W=8. Any W=1 improvement.
- The compaction and commit residue values, until the zones read them. Whether a parallel compaction pays off.
- The SDF stage's cost or benefit.
- Anything about the sleeping floor, beyond an ungated prediction.
- Bit-identity under a non-default MXCSR or across different binaries. This is the same scope as every existing bit-identity claim.
- Zero allocation: one scope per step is added (D8).
- System-level `FALLBACKS` / `HELD_FACE_YIELDS` counts at W > 1.
- Any Jolt-parity ratio other than the one G-TW measures.
- The unification end state: this lane adds one physics-local `pool.scope`, to be folded into the K5 gangs at refactor time.

## Open questions
1. **The quiet window** (about 460 processes, roughly 80 min at P0b's rate). This is the owner's call.
2. **The parallel-compaction trigger**: a second scope phase if `phys_np_compact ≥ 1 %` of T(8) at W=8. I propose this threshold; the critic may contest it.
3. **Record hygiene, for the orchestrator:** `D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:90` ("Broadphase default Auto", rung P1) is contradicted by P0b §8. It needs a supersession note at merge time. §10.4's per-pair slots are implemented here at chunk granularity, and the doc should say so.
4. **The uniform-scene price of L2** (keeping AllPairs between 1,109 and 3,000 bodies where Grid would win) is accepted until the broadphase redesign. That redesign owns any disparity signal.