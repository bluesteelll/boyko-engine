# L11 design: per-contact solve setup — warm impulses stored per manifold, cohort-shaped constraint blocks, SIMD warm apply

This is a read-only design against `D:/wt/joltab`. I could not check that the tree is at `f236ebdd` (no shell), and graphify could not run. Line numbers come from the working copy. "arith." means computed from P0 spans or counts, not measured.

**The short answer:** L11 can bring the solve setup from 0.425 µs to about 0.10–0.21 µs per manifold, and trim the solver kernel by 10–25 %, without changing a single output bit. That gets boyko to about 1.03–1.17× Jolt v5.3.0 per contact at W=1. It cannot reach v5.6.0: even with setup at zero, collision detection (broadphase + narrowphase) is still 1.16 µs per manifold at W=1, against Jolt v5.6.0's 1.14 µs for its whole step.

## Goal
- **Functional: nothing changes.** L11 does not change any value, for any W and for both the scalar and SIMD solve paths. That covers poses, velocities, accumulated impulses, `WarmSeedStats`, sleep latches, `contact_wakes`, and every pin and golden. Correctness bounds (A7-R1, A7-R2, the sleep suites, `support_loss_wakes_sleepers`, `frozen_island_warm_start`) are re-run unchanged.
- **Performance.** Four changes:
  - remove the per-point hash probe and insert, and the refill of a 1.5 MiB table;
  - remove the 26-column push build;
  - remove the AVX2 kernel's per-rank scalar gather (20 fields × 8 lanes);
  - make the warm apply 8 lanes wide.

**Targets, J with `simd_solve` on, per-stage spans from P0's J-B (arith.; stages normalised by 4,524.2 manifolds):**

| stage | P0 ms / µs per manifold | L11 ms / µs per manifold |
|---|---|---|
| solve_build | 1.038 / 0.229 | 0.25–0.55 / 0.055–0.122 |
| warm_apply | 0.614 / 0.136 | 0.15–0.30 / 0.033–0.066 |
| store | 0.270 / 0.060 | 0.04–0.08 / 0.009–0.018 |
| **setup total** | **1.922 / 0.425** | **0.44–0.93 / 0.097–0.206** |
| wide colours (kernel) | 3.576 / 0.790 | 2.68–3.22 / 0.59–0.71 |

**End to end (arith.). The "per step" ratio compares step times directly. The "per manifold" ratio divides each side by its own manifold count: boyko 4,519.3, Jolt 8,456.**

| row | today | after L11 | per step vs 5.3.0 / 5.6.0 | per manifold vs 5.3.0 / 5.6.0 |
|---|---|---|---|---|
| W=1 (derived simd T 11.04–11.14 ms) | 2.44 µs per manifold | T −1.35 … −2.37 ms → 8.67–9.79 ms; 1.92–2.17 µs per manifold | 0.55–0.62 / 0.90–1.01 | **1.03–1.17 / 1.68–1.90** |
| W=8 (base simd + L5, projected 5.5–5.7 ms) | 1.22–1.26 µs per manifold | T −1.01 … −1.57 → 3.93–4.69 ms | 1.11–1.33 / 1.58–1.88 | 2.08–2.48 / 2.95–3.52 |
| W=8 + C4 (conditional) | — | a further −0.15 … −0.36 ms | — | — |
| R-S (B1 carry) | store 0.32–0.37 ms | store 0.05–0.10 ms | — | — |

- **Allocations:** 0 heap per step on C1–C3. C4 adds one `pool.scope` per step, and only if it clears the build-if rule.
- **Committed memory at J:** about 3.8 MB of columns and tables today (arith.: 26 per-point columns ≈ 1.8 MB plus two 1.5 MiB tables) becomes about 1.7 MB.

## Context and constraints
- **Affected code:**
  - `solver/colored.rs` (the columns, build, warm apply, kernel, oracle, restitution, store);
  - `solver/warm_start.rs` (docs only; `WarmStartTable` stays for `SoftStepSolver` and as a test oracle);
  - a new file `solver/warm_records.rs`;
  - `scratch_ids.rs`; `colored_tests.rs`.
- **Invariants relied on, re-read in the tree:**
  - The body-body stream is sorted by `(a, b)`: `systems.rs:344-347` (non-strict assert; L5 ruling O6 makes it strict).
  - SDF manifolds follow, at most one per dynamic row, in row order (`systems.rs:606-634`).
  - The stream comes from `ContactPairs` via `Manifolds::manifolds_build`, which is `pub` (`resources.rs:2345`), and tests hand-build unsorted streams (`colored_tests.rs:789-815`). So sortedness is a fast-path property, never a correctness premise.
  - Feature ids are distinct within a manifold **only by test** (`sleep_settles_box_piles.rs:1654-1666`). The runtime must reproduce today's last-write-wins.
  - Today's cohorts are 8-group windows starting at each colour's first group, on every path: the serial loop (`colored.rs:2756-2769`), the inline path (`:2880-2897`), and parallel cuts stepped by `COHORT` from `g_lo` (`:3008-3017`). Cohort membership therefore does not depend on W.
  - Frozen status is per island of the dynamic side (`:1863-1871`), so manifolds with equal rows have equal frozen status.
  - `vn_initial` is read only by `apply_restitution`, after `restitution > 0` is tested (`:3146-3152`).
  - The effective mass is recomputed from `(dir, ra, rb, inv_mass, inv_inertia)` in every sweep (`contact.rs:131-148`).
  - `cross8` is op-for-op `Vec3::cross` (`simd.rs:546-561`).
- **Binding rules:** replay determinism, `{1, N}` bit identity, principle 0 (`ScratchColumn`s owned by the solver resource), no hot-path allocation, lock-free, no FMA (`FMA-DETERMINISM.md`; the census tests at `colored_tests.rs:3221` and `solver_simd_has_no_fma_or_approx_callsites` cover the new code automatically).

## Key decisions

### D1 — Warm identity: one record per manifold in stream order, found by a merge-join. No hash.
- **What:**
  - The store writes `recs[mi]`: 64 B holding the 4 points' `λn`, `λt1`, `λt2`, `fid & 0xFFFF`, and `count`.
  - It also writes `keys[mi] = ord(m)`: `(a<<32)|b` for body-body, `(1<<63)|a` for SDF. This is L10's D6 ordinal, which is monotone with the stream order.
  - The next step finds each manifold's source record by `ord(remap.manifold_pair(m))`, walking in manifold order with a cursor. Points then match by fid inside the record.
- **Why:**
  - Manifold order is sorted by pair, so on an Identity step (the steady state) the lookup is one compare per manifold against a sequential 36 KB key array.
  - Today it is 16.9k random probes into 1.5 MiB. The research's witness puts about 15–20 ns per point of the build, and about 10 ns per point of the store, in costs that grow with the working set.
  - The store becomes a write by index: its layout is a pure function of `mi`. IM-2b's canonical-order constraint, the `canonical` and `warm_key` columns, and the table refill all disappear. A per-manifold record is also the natural shape for L8's per-manifold friction impulses (Jolt master, Box3D, Bepu).
- **Rejected:**
  - Jolt-style hash per manifold: still random, still needs a refill, and the layout depends on insertion order.
  - Box2D persistent contact records (no lookup at all): need persistent pair identity across row moves. That is U7 `PairCache`, which is refactor-last.
  - History-independent hashing (Blelloch–Golovin): still random access over MiB.
  - A smaller entry or prefetching in today's table: at best halves the misses.
- **Trade-off:** 72 B per manifold per buffer, instead of 24 B per table slot (at load ≤ 0.5). A non-strict stream pays a cold sort (D2).

### D2 — The search: gallop forward, binary search backward. A cold sorted index for non-strict streams. Exact everywhere.
- **What:**
  - When `keys_r` is strictly increasing (`strict_r`, recorded by the writer):
    - if `keys_r[cur] ≤ k`, gallop forward from the cursor;
    - otherwise binary-search `[0, cur)`;
    - then set the cursor to the index after the result.
  - Otherwise (unsorted or duplicate-key streams): build `(key, mi)` pairs once in the lookup pass and `sort_unstable` them in a `ScratchColumn` (in place, no heap). A lookup takes the last equal key and scans equal-key records by descending `mi`.
  - Inside a record, scan points from last to first; the first fid match wins.
- **Why:**
  - Rows steps translate keys non-monotonically only around jumpers or order flips. Binary search handles those exactly, so there is no dependency on the tree lane's `stage2_rows`.
  - The scan order reproduces today's per-key last-write-wins (Lemma W).
- **Cost:** O(M) on Identity; O(M + j·log M) on Rows steps with j non-monotone keys; O(M log M) only on non-strict streams (hand-built tests).

### D3 — The store, and the B1 carry
- **What:**
  - The fill (D4) pre-writes each solved record's shape (count, fids). After restitution, the store walks cohorts and writes only the impulses into `recs_w[mi]`.
  - A second pass over the per-manifold plan, in manifold order:
    - frozen manifold (B1): copy per point, by fid, from `recs_r[src]`; hits compact into the new record, and a miss drops the point, as today;
    - any other unsolved manifold: `count = 0`.
  - Then swap, and stamp `warm_cursor`. With warm start disabled: no store and no stamp, as today.
- **Why:** the writes land on 11 ascending per-colour streams of 64 B lines, with no 1.5 MiB refill. The carry becomes O(frozen manifolds): R-S 0.32–0.37 → 0.05–0.10 ms (arith.).

### D4 — Build = plan (manifold order) → layout (colour order) → fill (per cohort), sized then written
- **What:**
  - **P-a**, serial, O(M), in manifold order. Per manifold:
    - write `keys_w[mi]`;
    - decide frozen;
    - search for the source record;
    - write `plan[mi] = {src, count, flags}` (8 B).
  - **P-b**, serial, O(M), over the graph's colour CSR:
    - assign groups;
    - write `group_start` (point prefix), `color_offsets`, `color_group_start`, `color_cohort_start`;
    - write each cohort's `rank_base`, `depth`, `nlanes`, per-lane `width` and `mi`.
  - **P-c (fill)**, per cohort: per-lane manifold constants go into the head, per-point data into rank blocks. Padding lanes and ranks are written as zero.
  - Columns are `resize`d to their exact length. This costs O(1) in the steady state because it fills only on growth; there is no `clear` and no `push`.
- **Why:**
  - Today's per-point `push` touches 26 build views: base/len/committed reloads, plus 26 constructions and drops per manifold (`colored.rs:1775`, `:1014-1044`). That is most of the 42 ns per point that does not grow with the working set.
  - A fill cohort writes 5–7 lines that stay in L1 while its 8 lanes are written.
  - The fill is a pure function per cohort, which makes C4 possible.
- **Rejected:**
  - Setup inside the narrowphase (Jolt): colour order does not exist until the graph is built.
  - Keeping one walk: it cannot size first.

### D5 — The AoSoA layout is today's cohorts, materialised
- **What:**
  - Per cohort, a 320 B `CohortHead`: n, t1, friction, body ids, widths, sentinel bits; t2 is derived in-kernel with `cross8(n, t1)` (the same op as `contact.rs:67`), and restitution plus `mi` live in a 64 B cold record.
  - Per (cohort, rank), a 320 B `RankBlock`: ra, rb, sep, λn, λt1, λt2, 8 lanes each.
  - Cold per rank: `vn0[8]`.
- **Why:**
  - The kernel does aligned vector loads instead of 160 scalar loads plus 160 stack stores per rank.
  - Per-manifold constants are stored once, not once per point: kernel bytes go from 1.35 MB to 0.92 MB at J (arith.), padding included (≈ 9 %, 2,320 blocks against 16,888 points).
  - The rank loop shrinks, which helps the I-cache.
  - It is Box2D's `b2ContactConstraintWide`, with adaptive depth (Box3D's is fixed at 4).
- **Rejected:**
  - Per-point SoA with per-group constants: still 10 gathered fields per rank, and 13 write streams.
  - Box3D's fixed 4 ranks: 4× padding for sphere piles.
  - Bepu's per-width type batches: more kernels, more I-cache.
  - Storing t2: +96 B per cohort to save 9 ops per cohort.
- **Trade-off:** every reader indexes by (cohort, lane, rank). Tests that build columns are ported (C2).

### D6 — The kernel, oracle, restitution and dispatch read the new layout; dispatch structure unchanged
- Chunk cuts stay as today: `COHORT` steps under SIMD, single groups for the scalar path, point quotas from `group_start`. Waves, scopes and chunk counts are therefore unchanged, and so are the census and the runner's structural expectations.
- The scalar oracle and restitution walk group-major (lane outer, rank inner), which is today's slot order.
- The worker path uses raw `addr_of!`/`addr_of_mut!` projections only; no `&`/`&mut` to a whole block. Scalar workers may share a cohort's blocks, but they write distinct lane elements.

### D7 — SIMD warm apply per cohort, serial, under `simd_solve`
- **Bit-identical:**
  - Each lane computes `((n·λn + t1·λt1) + t2·λt2)`, then `·(−1)` for A, then `apply_impulse_blend_x8` (already op-for-op with `apply_impulse`, `simd.rs:697-740`), with movable masks.
  - Each dynamic body appears in at most one lane per colour, and colours are walked in ascending order, so every body's add sequence equals today's slot walk.
- **Parallel per colour (L7) is not built:** 4 × 9.11 waves × ω(8) = 6.54 µs ≈ 0.24 ms of dispatch, which is at least the whole SIMD warm apply (0.15–0.30 ms). Recommend retiring L7 after C3.

### D8 — Rejected value-level shortcuts
- **Folding the warm apply into the first sweep is not bit-identical.** Take a body X in colours c0 < c1. Today: W(c0), W(c1), then S(c0) sees W(c1). Fused: W(c0), S(c0), W(c1), so S(c0) no longer sees W(c1). No reference engine does it (Box2D warm-starts every colour as its own stage).
- **Once-per-step effective masses (Box2D, Jolt, Rapier):** a value change, because inertia is refreshed every substep (`:3597-3600`).
- **Named follow-on, L12 (bit-identical, not part of L11):** cache the three masses per inertia epoch. There are **5** epochs per step, not the research's 8: build, then one after each substep's `refresh_inertia`, each serving relax(s) ×2 plus biased(s+1). The first sweep of each epoch computes and stores; the other 7 sweeps load. This needs its own measurement.

### D9 — `vn_initial` computed lazily
- Fill computes vn0 only where the group's restitution > 0, and writes 0 otherwise.
- The value is dead there (`:3146-3148`), so this is bit-identical in behaviour.
- The C0 digest masks vn0 to 0 when restitution ≤ 0.

### D10 — Parallel fill is conditional (C4)
- Cohort ranges with point quotas; inline when `lanes < 2`, `n_chunks < 2`, `parallel_solve` is off, or there is no pool.
- **Build-if:** the measured `solve_build(8)` after C3 is ≥ 5 % of T(8) and above the SE bar.
- **Cost:** +1 scope per step. That needs a ruling of the kind L5 D8 got.

## Data structures
```rust
// solver/warm_records.rs (new): the colored solver's warm store. Owned by ColoredSoftStepSolver.
#[repr(C, align(64))] #[derive(Clone, Copy)]
pub(crate) struct WarmRecord {      // 64 B, one line; index = manifold index of the writing step
    n: [f32; 4], t1: [f32; 4], t2: [f32; 4], // accumulated impulses of stored points, point order
    fid: [u16; 4],                  // feature_id & 0xFFFF (pack's FEATURE_MASK semantics)
    count: u8,                      // stored points 0..=4 (carry: hits only, compacted)
    _pad: [u8; 7],
}
pub(crate) struct WarmRecords {     // one side of the double buffer
    keys: ScratchColumn<u64>,       // HOT: searched; ord(m) per manifold, stream order (36 KB at J)
    recs: ScratchColumn<WarmRecord>,// read on a hit / written by fill + store (290 KB at J)
    strict: bool,                   // keys strictly increasing, else the cold index is used
}
#[repr(C)] #[derive(Clone, Copy)]
struct Plan { src: u32 /* record index in read side | NONE */, count: u8, flags: u8 /* FROZEN|LOOKED_UP */, _p: u16 } // 8 B

#[repr(C, align(64))] #[derive(Clone, Copy)]
struct CohortHead {                 // 320 B, HOT: loaded once per cohort per sweep / warm apply
    n: [[f32; 8]; 3], t1: [[f32; 8]; 3],       // per-lane manifold normal, first tangent (t2 derived)
    friction: [f32; 8], body_a: [u32; 8], body_b: [u32; 8], // body_b == body_a on a sentinel lane
    width: [u8; 8],                 // points per lane; 0 = absent
    rank_base: u32, depth: u8, nlanes: u8, sentinel: u8 /* bit l */, _p: u8,
    _pad: [u8; 16],
}
#[repr(C, align(64))] #[derive(Clone, Copy)]
struct CohortCold { restitution: [f32; 8], mi: [u32; 8] } // 64 B: restitution pass, store
#[repr(C, align(64))] #[derive(Clone, Copy)]
struct RankBlock {                  // 320 B, HOT: one per (cohort, rank)
    ra: [[f32; 8]; 3], rb: [[f32; 8]; 3], sep: [f32; 8],
    ni: [f32; 8], ti1: [f32; 8], ti2: [f32; 8],  // written by the kernel (whole cohort = one owner under SIMD)
}
// ContactColumns → CohortColumns:
//   heads, cold, blocks, rank_cold: ScratchColumn<[f32; 8]> (vn0),
//   plan, color_offsets, group_start, color_group_start, color_cohort_start, warm_index (cold (u64, u32))
// ColoredSoftStepSolver: `warm_read`/`warm_write` → `warm: [WarmRecords; 2]`, `warm_cur: u8`; `frozen_points` deleted.
```
- **Sizes:** const-asserted 64 / 320 / 64 / 320 B. Bases are 64-aligned (debug-asserted at construction).
- **Reserve:** the point ceiling equals today's per-f32 column ceiling. The block reserve is ⌈ceiling / 8⌉ plus the cohort bound. This is address space only; without it, a push past the reserve panics.
- **Ids:**
  - The contact band keeps its top; `CONTACT_COLUMN_COUNT` 31 → 12 (freed ids are headroom).
  - Records use `warm_table_id(2)` and `(3)` with the `WarmRecord` layout; `(0)` and `(1)` keep `WarmEntry` for `SoftStepSolver`.
  - Co-swept pairs are asserted distinct mod `POOL_STAGGER_LINES`, as at `scratch_ids.rs:666-688`.
- **Drop:** none (all POD). **Send/Sync:** `CohortSolveView` and `CohortFillView` are `Copy + Send + Sync` via `unsafe impl`, with the disjointness SAFETY argument below.

## Public API
- No public change. `ColoredSoftStepSolver::{with_capacity, with_warm_start, warm_seed_stats, solved_steps, solve_colored*}`, `WarmSeedStats` (32 B) and `PhysicsConfig` are all unchanged.

## Algorithms for the critical paths

| pass | steps | complexity | cache / branching / SIMD |
|---|---|---|---|
| P-a | per mi: read line 3 of `Manifold` (a, b, count); `keys_w[mi]`; frozen = `manifold_frozen`; `lp = remap.manifold_pair(m)`; `src = find(keys_r, cur, ord(lp))`; `plan[mi]` | O(M), plus j·log M on Rows steps | sequential; one predictable compare on Identity |
| P-b | per colour, per mi of `graph.color(c)`: skip frozen/empty; group g; lane = (g − base_g) mod 8; close a cohort at 8 lanes or colour end; `rank_base += depth` | O(M) | 8 B `plan` reads, ascending within a colour |
| P-c | per cohort, per lane: `tangent_basis`; `max` materials; `pa`/`pb`; head lane writes; record shape; per rank: ra, rb, sep, seed (fid scan of `recs_r[src]`, last match), vn0 if restitution > 0; zero padding (10 vector stores per block) | O(P) | writes land in the cohort's L1-resident lines; reads Manifold (3 lines), bodies (L2), record (1 line) |
| warm apply (SIMD) | per cohort: gather 16 bodies (`stage_body_state`); load n, t1; t2 = `cross8`; per rank: 9 aligned loads; impulse; two blended applies; scatter movable lanes < nlanes | O(P/8) vector | ≈ 12 vector ops per point plus cohort gather |
| kernel | as today, but per-rank data = 10 aligned loads; `active = cmpgt_epi32(cvtepu8(width), r)`; impulse store = `blendv(old, new, active)` (padding stays 0) | unchanged | the 20-field gather loop is removed |
| scalar oracle / restitution | group-major over the cohort: lane constants from the head, per rank from the block | unchanged | strided 32 B within a block |
| store | walk cohorts: impulses → `recs_w[mi]`; then walk `plan`: carry (frozen), count 0 (others) | O(M) | 11 ascending write streams |

## Determinism argument
- **Lemma W (seeds).** For rows < 2²⁴ − 1 (today's `pack` domain, debug-asserted), `get(pack(la, lb, f))` in today's table equals `lookup(ord(la, lb), f & 0xFFFF)` over the records. Five facts give it:
  1. Today's table value per key is the last insert of that key, in the order: solved manifolds ascending, then frozen ones ascending.
  2. `recs[i]` holds exactly manifold i's inserts, in point order.
  3. Manifolds with equal rows share an island, so they are both frozen or both solved. Within one class, ascending `mi` equals insert order, so "descending `mi` among equal keys, last point first" selects the last insert.
  4. Carried entries: by induction on steps, the table and the records returned the same lookup values last step, so the copied values are equal.
  5. The remap is today's function, `manifold_pair` at `row_identity.rs:163-169`.

  Hit counts are equal as well, so `point_hits` and `carry_hits` are equal.
- **Values.** Every per-point input (ra, rb, n, t1, t2, sep, friction, restitution, ids, sentinel, seeds, and vn0 where it is read) comes from the same function of the same inputs. t2 = `n.cross(t1)` in both places.
- **Order.**
  - The kernel's cohorts, and lanes within cohorts, are today's (colour-start 8-windows).
  - The scalar oracle and restitution follow slot order.
  - The warm apply adds to each body in the same sequence (D7).
  - Masked lanes compute on zeros and never reach memory; FP exceptions are masked.
- **W invariance.**
  - P-a and P-b are serial.
  - The fill is a pure function per cohort.
  - Records are indexed by `mi`.
  - Nothing depends on thread count or steal order.
  - Across machines and binaries: integer merges plus op-for-op no-FMA SIMD. The {scalar, simd} oracle covers both in one binary.

## Multithreading model
- **C1–C3:** P-a, P-b, the fill, the warm apply and the store run on the calling thread. The kernel's dispatch is unchanged.
- **Parallel kernel:**
  - SIMD chunks own whole cohorts, i.e. their heads and blocks. A foreign read is impossible, so the absent-lane `chunk_start` trick (`:2371-2383`) is deleted.
  - Scalar chunks own groups: each reads its lanes' head constants (never written during the solve) and reads and writes its lanes' elements through raw projections.
- **C4 fill:**
  - Shared and read-only during the scope: `manifolds`, `bodies`, `bodies_eff`, `recs_r`, `plan`, `cold.mi`, all written serially before the scope.
  - Exclusive per task: `heads[k_lo..k_hi)`, `blocks[rank_base(k_lo)..rank_base(k_hi))`, `rank_cold` over the same range, and `recs_w[mi]` for its lanes' `mi`.
  - Hit counts go to a function-local `[AtomicU32; 256]` with `Relaxed` stores (L5 D7); the scope join provides happens-before.
  - No new atomics protocol, so loom is N/A.

## Integration: files and lines

| file | lines | change |
|---|---|---|
| `solver/colored.rs` | `:69-80` module doc (IM-2b) | rewritten: records by manifold index |
| | `:97`, `:110-114` | imports and ids |
| | `:116-206` | add aligned load/store helpers |
| | `:343-380`, `:470-778` | `ColorSolvePtrs`, `ContactSolveView` → `CohortSolveView` |
| | `:788-816`, `:1352-1432` | `ContactBuildView` and `push_point` deleted |
| | `:818-1350` | `ContactColumns` → `CohortColumns` |
| | `:1443-1481`, `:1495-1513` | solver fields, `with_capacity` |
| | `:1587-1729`, `:1737-1817`, `:1830-1843` | build → P-a / P-b / fill; `point_keys` → `ord` |
| | `:1881-1918` | warm apply: scalar group-major + `warm_apply_avx2` |
| | `:1974-2024`, `:2028-2174` | dispatch fork and scalar oracle take a colour context plus a group range |
| | `:2228-2653` | kernel: cohort range, aligned loads, blended impulse store |
| | `:2708-2772`, `:2845-3132` | colour loop and cuts: same quotas, via `group_start`, cohort index from `color_cohort_start` |
| | `:3141-3197` | restitution group-major |
| | `:3229-3319` | `store_and_swap`, `carry_frozen` → D3 |
| | `:3459-3472`, `:3562-3565`, `:3627-3630` | call sites; zones unchanged |
| `solver/warm_records.rs` (new) + `solver/mod.rs` | — | `WarmRecord`, `WarmRecords`, `find`, cold index |
| `solver/warm_start.rs` | `:1-64` | docs: the table serves `SoftStepSolver` only |
| `scratch_ids.rs` | `:56-84`, `:229-311`, `:826-875`, `:1110-1125` | band count, layouts, id census |
| `solver/colored_tests.rs` | `:264`, `:443`, `:736-984`, `:1420-1454`, `:1454-2181` | ports (below) |
| `profiling.rs` | — | C4 only: counter `PHYS_FILL_CHUNKS` |
| `benches/jolt_parity_pyramid.rs` | — | a `J-As` row (AllPairs, `simd_solve` on, `parallel_solve = W>1`) if the flag is absent |
| docs | `docs/SYSTEMS.md`, `docs/FEATURE_MAP.md` | colored solver entries |

**Interactions with other lanes:**
- **L5, and L4/L2 (in progress in `D:/wt/l5np`):**
  - L5 keeps the stream byte-identical and makes the order assert strict, which is L11's fast path.
  - L4 edits the `parallel` gate at `:3544-3545`: a textual conflict only.
  - `scratch_ids.rs` conflicts are in different sections.
  - Base L11 on the L5 merge.
- **Tree broadphase:**
  - It emits the exact sorted set.
  - L11 needs neither `stage2_rows` nor the jumper list: D2 handles non-monotone translations.
- **L10 rev 2 (closed; its C3 edits the same sites).** Mapping:

  | L10 piece | under L11 |
  |---|---|
  | C3a need-sized rebuild + rehash | not needed: records are need-sized |
  | A6 `restore_warm` insert into `warm_read` | a sorted `restore_rec` column that P-a searches after `keys_r`; keys are disjoint (L10 E5), so there is no merge copy |
  | `kept_warm` (24 B per point) | kept records: `recs_r[src[mi]]` at move-in |
  | carry and move-in capture through the `carry_frozen` logic | `plan.src` |
  | "table bytes under Replay" | record bytes |

  **Recommendation (orchestrator):** L11 C1 lands before L10 C3a.
- **L8:** the record's value fields become `λn[4]`, `λt1`, `λt2`, twist; per-manifold friction moves into the head. **L11 before L8:** L8 then edits two structs instead of adding a per-point key class.
- **L6:** unaffected (waves unchanged).
- **L7:** retire it after C3 (D7).
- **U7 `PairCache`:** would later replace the stream search with persistent ids.

## Commit sequence (each green)

| commit | content | value change |
|---|---|---|
| **C0** | Test-only `setup_digest`: FNV over the logical per-point values in slot order (seeds, masked vn0, group/colour CSRs) + `WarmSeedStats`, plus post-step body bits, per step. Scenes S-a…S-g (below) × W {1, 8} × simd {off, on}, pinned as constants. Record pose hashes (J-A, J-As, J-B, R, R-S, S16 at W {1, 8}) and check them against P0's. | none |
| **C1** | D1–D3 on today's SoA: P-a + `plan`, seeds from records, store by `mi` through `manifold_base`, carry by fid. Deleted: `canonical`, `warm_key`, `frozen_points`, and the colored use of `WarmStartTable`. | none |
| **C2** | D4–D6, D9: `CohortColumns`, P-b, fill, kernel, oracle, restitution, store on AoSoA; the O7 suite ported; `setup_digest` reimplemented over the new layout | none |
| **C3** | D7: `warm_apply_avx2` under `simd_solve`; the scalar path is the oracle | none |
| **C4** | Conditional (D10): parallel fill, `PHYS_FILL_CHUNKS`, census +1 scope | none |
| receipts | P0-protocol A/B, MEASUREMENT-QUEUE result block (tester/analyst) | — |

## Gates (each shown able to fail)
- **G1, the identity gate (C1–C4)** in `colored_tests.rs`. The C0 pins must reproduce.
  - **Scenes:**
    - S-a: 6-layer box pyramid, 300 steps;
    - S-b: sleeping pile freezes, then `wake_all`;
    - S-c: spawn/despawn/migrate with order flips and jumpers (Rows and Reset);
    - S-d: SDF box pile;
    - S-e: hand-built unsorted stream with duplicate keys and duplicate fids;
    - S-f: warm start toggled off → on;
    - S-g: restitution > 0.
  - Body bits are layout-free and authoritative; the digest localises a failure.
  - **Anti-vacuity** (crate-internal counters): S-b `carry_hits > 0`; S-c backward searches > 0 and misses > 0; S-e cold-index steps > 0 and a duplicate fid met; S-f one disabled step; S-g restitution applied > 0. **C0 red-first:** friction + 1 ULP in the build turns the digest red.
- **G2, differential proptest (C1).** Random ≤ 64-manifold streams over 3 chained steps: remap in {Identity, random `Rows` with flips and `NO_ROW`, Reset}; SDF; duplicate fids and keys; frozen subsets. Seeds and hit counts from the records must equal a `WarmStartTable` canonical-insert oracle. Non-vacuity counts are asserted.
- **G3, layout bytes (C2).** Heads, blocks and records are byte-equal across W ∈ {1, 2, 4, 8, 16} and across runs; padding lanes are zero (also a debug assertion). This replaces `:904` and retires `:966` (skipped when its `D:/tmp` file is absent, so it can pass from emptiness).
- **G4, the O7 suite ported:** `simd_solve_bits_match_scalar`, `simd_solve_width_only_matches_scalar_step`, 1c/1d differentials, `cohort_shape_proptest_bit_exact_and_non_vacuous`, built through P-b and the fill from `GroupSpec`s. Plus a new {scalar, simd} warm-apply differential (C3).
- **G5, poses:**
  - runner `--expect-pose` for every C0 row at W {1, 8};
  - `default_world_pyramid_determinism`, `bodytype_determinism_golden`, goldens;
  - `cargo test --workspace --all-targets --no-fail-fast` with **zero pin moves**.
- **G6, correctness bounds:** A7-R1, A7-R2, the sleep suites, `support_loss_wakes_sleepers`, `frozen_island_warm_start`, `row_keyed_state_defect_a` all pass with assertions byte-unchanged.
- **G7, census:** 0 heap per step after warm-up, and scope/chunk pins unchanged (C1–C3). C4: +1 scope, pinned, plus a 1-worker twin of G-L4-1 that must allocate 0.
  - **Correction (2026-09-21, after C2's census adjudication):** "scope/chunk pins unchanged (C1–C3)" was a prediction, and it did not survive C2's capture shrink. Scope did not move (134 on every frame), but the colour task's closure shrank with the view it captures — `ContactSolveView` 200 B → `CohortSolveView` 40 B, the spawned closure 264 → 104 B, its `ScopedCell` 280 → 120 B, so 34 cells fit a 4 KiB `ScopeBlock` chunk where 14 did — and the 96 colour scopes a step that spawned 15..=23 tasks at W=4 no longer take a second chunk. The release S1c pins moved DOWN: chunk 230 → 134, dispatch MAX 365 → 269, on every one of 4,352 long-run steps (−96 per step, frame for frame, against a C1 control rebuilt from `git archive`; the 17 × 256-step protocol of `alloc_frame_census.rs`, header "S1c after L11 C2"); the debug pins did not move. This is the census's own re-pin case — a narrowing with no headroom either way — not a widening, and "0 heap per step after warm-up" holds as written. The attribution binary's row D lost its `four > two` witness for the same reason (W=2 == W=4 == W=8 on the shipped kernel: a colour spawns at most 32 tasks at W=8, since `n_chunks` is a ceiling the cohort-snapped cut walk rounds down, and 32 × 120 B fits one block); it now pins `chunk == scope` per frame at W = 2/4/8 and keeps the strict lane-growth assertion where the model predicts growth — on the scalar cut walk (`simd_solve = false`), W=8 against W=4.
- **G8, Miri:** kernel, fill and store at small n. C4: scalar-parallel and fill on `std::thread::scope`.
- **Named mutations, each recorded red before its commit lands:**

| id | mutation | red in |
|---|---|---|
| M1 | first match inside a record | G2, G1 S-e |
| M2 | gallop only, no backward search | G2, G1 S-c |
| M3 | lookup keyed by current rows | G1 S-c, `row_keyed_state_defect_a` |
| M4 | carry writes `count = 0` | G1 S-b, `frozen_island_warm_start` |
| M5 | cold index returns the first equal key | G2 |
| M6 | fill skips padding zeroing | G3 |
| M7 | warm apply associates `n·λn + (t1·λt1 + t2·λt2)` | G1 S-a, G4 |
| M8 | kernel derives t2 = `t1 × n` | G1, G4 |
| M9 | scalar oracle reads lane l+1's head | G1 |
| M10 | store takes impulses from the neighbouring lane | G1 |
| M11 | lazy vn0 rule inverted | G1 S-g |
| M12 | warm-apply scatter to all 8 lanes (absent lane → row 0) | G1 |
| M13 (C4) | fill base derived via `as_read_slice().as_ptr().cast_mut()` | Miri Tree Borrows error |
| M14 (C4) | the `lanes < 2` inline term dropped | 1-worker alloc twin |
| M15 | `Vec::with_capacity(1)` in P-a | G7 |

- **G9, timing (P0 protocol):**
  - Parent against tip binaries, K=12 interleaved, receipts per process, the canary seen. Claim iff |effect| > 2·√(SE_A² + SE_B²).
  - Rows: J-As and J-A at W ∈ {1, 2, 4, 8, 16}; R and R-S at W {1, 8}; S16.
  - Armed J-As-a at W {1, 8} for per-stage µs per manifold (same 4,524.2 denominator before and after, because the lever is bit-identical).
  - **Realized-gain gates, ≥ 0.6 × the lower predicted Δ:**
    - solve_build −0.29 ms;
    - warm_apply −0.19 ms (C3, J-As only);
    - store −0.11 ms;
    - T(1) J-As −0.81 ms;
    - T(8) −0.61 ms.
  - Wide colours: **not claimed slower** is the gate; the −10…−25 % is claimed only if measured above SE.
  - cfg-A (scalar colours): not claimed slower at any W.
  - C4: its own in-binary serial/parallel A/B at W=8, built only under D10's rule.

## What moves, and how it is re-measured
- **Values:** nothing. A red pin is a defect, not a re-bless.
- **Layout tests ported in C2:** `colored_tests.rs:736-772`, `:855-984`, `:1420-1454`, `:1577-1626`, `:1708-1900`, `:2101`. `:264` and `:443` stay (`group_start` remains).
- **Scratch-id and component-id census:** re-run, with `CONTACT_COLUMN_COUNT` 31 → 12.
- **Census pins:** zero diff (C1–C3); +1 scope (C4). *(Corrected 2026-09-21: C2 moved the release S1c chunk and dispatch pins DOWN — see the G7 correction above.)*
- **Runner:**
  - the zone set is unchanged, so per-run structural expectations hold;
  - `J-As` row added;
  - C4 adds the `PHYS_FILL_CHUNKS` expectation (0 when inline or at W=1).
- **Memory receipts:** committed pages shrink by about 2 MB (C0 against C2, `committed_rows` summed).
- **Docs:**
  - the MEASUREMENT-QUEUE result block and a per-contact table;
  - the L10 lane gets the mapping above;
  - `SYSTEMS.md` / `FEATURE_MAP.md`;
  - the book is doc-writer only (the physics page's warm-start paragraph).

## What it cannot claim
- **Per-contact parity with v5.6.0.** Even with setup at zero the ratio is 1.77×, and after L11 it is 1.68–1.90× at W=1. The remainder is collision detection (bp 0.465 + np 0.695 µs per manifold), owned by L5, the tree broadphase and L9.
- **A Jolt per-stage setup comparison.** Jolt's setup sits inside FindCollisions; P0 cannot separate it.
- **Setup scaling at W=8** without C4: the setup stays serial at 0.44–0.93 ms.
- **The kernel gain is an instruction-count estimate.** It is gated only as "not slower", and claimed only if measured. There is no gain claim for the scalar cfg-A colours.
- **Old table bytes:** only values, seeds, stats and poses are claimed.
- **The effective-mass epoch cache (L12)** and a SIMD body transpose for the cohort gather: not in L11.

## Open questions (orchestrator)
1. **Lane order:** L11 C1–C2 before L10 C3a (so L10's warm pieces map onto records rather than a rehash), and L11 before L8.
2. **A ruling on C4's +1 scope per step** (the L5 D8 class), applicable only if C4 clears D10's build-if rule.
3. **Whether to commission L12** (mass cache per inertia epoch: 5 of 12 sweeps compute the masses, 7 load them; bit-identical).

## Files
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/warm_start.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored_tests.rs`
- `D:/wt/joltab/crates/boyko_physics/src/scratch_ids.rs`
- `D:/wt/joltab/crates/boyko_physics/src/row_identity.rs`
- `D:/wt/joltab/crates/boyko_physics/src/systems.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/scratch/views.rs`
- `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L5-narrowphase/02-DESIGN-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/00-RULINGS.md`

Sources (from the research brief):
- https://raw.githubusercontent.com/erincatto/box2d/main/src/contact_solver.c
- https://raw.githubusercontent.com/erincatto/box2d/main/src/solver.c
- https://raw.githubusercontent.com/erincatto/box3d/main/src/contact_solver.c
- https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.cpp
- https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Solver_Solve.cs
- https://raw.githubusercontent.com/bepu/bepuphysics2/master/Documentation/changelog.md
- https://raw.githubusercontent.com/dimforge/rapier/master/src/dynamics/solver/contact_constraint/contact_with_coulomb_friction.rs