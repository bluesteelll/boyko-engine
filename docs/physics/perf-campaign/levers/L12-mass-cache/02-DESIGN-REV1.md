# L12 design (rev 1): cache the effective masses once per inertia epoch, bit-identical

This design was written read-only against `D:/wt/joltab`. I assumed the tree is at `47c5dabd` but could not check it: there is no shell, graphify did not run, and no cargo command ran. Line numbers are from today's working copy, before L11. Where L11 rewrites a site, the L11 name is given as well. "arith." marks a number computed from P0 spans, the in-tree opcode census or instruction tables; none of those is a measurement. The base this design builds on:
- L11's cohort layout (`L11-solve-setup/02-DESIGN-REV1.md` D5, closed);
- L8 (friction per patch) already landed, as the ruled order has it;
- L5's lane in the base: `parallel_solve` on, the `lanes < 2` inline term, GRID 2,700/3,000.

**The short answer**
- **The schedule.** An effective mass reads only `(dir, ra, rb, inv_mass, inv_inertia)` (`contact.rs:131-148`, `simd.rs:638-683`). Within a step, those inputs change only in `build_bodies` (`colored.rs:1542-1564`) and in each `refresh_inertia` (`:3597-3600`).
  - A one-bit schedule is cleared by those two writers and set by every sweep. So the first sweep of each epoch computes the masses and stores them, and every later sweep in the same epoch loads them.
  - At (S, R) = (4, 2) that is **5 sweeps that compute and 7 that load**.
  - No floating-point operation is added or reordered. A load returns the bits that were stored, so the lever is bit-identical by construction.
- **Where the masses live.** A solver-owned `ScratchColumn` of 32-B lane vectors. Each cohort owns a run that starts on a 64-B boundary; its start, `mass_base`, goes into the pad of L11's `CohortHead`.
  - Cost: +96 B per rank, plus at most 32 B per cohort. That is ≈ 232 KB at J, +31 % on the kernel's byte stream.
- **Cost of a load against a recompute.** One rank's three masses take 246 FP ops and 73.5 cycles on the multiply pipes. Loading them takes 3 aligned loads, 1.5 load-port cycles, and 96 B of sequential stream. The load is 14–49× cheaper in issue resources.
- **Whether that becomes time depends on the kernel's regime.**
  - If the kernel is issue-bound (L11's own kernel prediction, 320–530 cycles per rank against a chain of ≈155, implies it is): **0.145–0.36 ms per step at W=1, i.e. 0.032–0.080 µs per manifold** on L11's shape.
  - If the kernel is chain-bound: ≤ 0.05–0.07 ms.
  - Nothing measured so far tells the two apart.
- **Build-if.** An in-binary probe must clear the K=24 end-to-end SE bar, about 1 % of T(1). If it does not, L12 is closed without receipts.
- **W=8.** The gain divides by 8·E(8) = 3.74, giving ≤ 0.1 ms (≤ 2.4 % of T(8)). It is gated as "not slower" only.

## Goal
- **Function:** unchanged, bit for bit. Poses, velocities, impulses, warm records, `WarmSeedStats`, sleep latches, every pin and golden stay as they are, for every W, on both the scalar and the SIMD path. A7-R1 drift, freeze steps, the sleep suites and incline stick/slip are re-run unchanged. Any movement is a defect.
- **Performance:**
  - in the 7 load sweeps, remove the three per-rank `effective_mass_x8` evaluations (after L8, whichever masses L8's kernel computes);
  - claim the gain per contact (µs per manifold of the kernel stage) at W=1, and end to end on T(1);
  - W ∈ {2, 4, 8, 16} is "not slower".
- **Memory:** 0 heap allocations per step. The committed column is ≈ 232 KB at J (L11 shape) or ≈ 156 KB (post-L8 shape, below).

## Context and constraints
- **Affected code:** `solver/colored.rs` (the step loop, `build_bodies`, the dispatch chain, the AVX2 kernel, L11's `CohortColumns` and `CohortHead`), `solver/simd.rs` (a doc comment; `#[inline]` only if C0 calls for it), `scratch_ids.rs`, `solver/colored_tests.rs`, one new bench.
- **Invariants relied on, re-read in the tree:**
  - **Writers of `inv_inertia`:**
    - `build_bodies` `:1557-1562`;
    - `refresh_inertia_scalar` `simd.rs:151-157`;
    - `refresh_inertia_avx2` `simd.rs:346-377` (it blends only `inv_mass != 0` lanes);
    - both refresh functions are called only at `colored.rs:3597-3600`.
  - **Writers of `inv_mass`:** only `build_bodies`.
  - **Everything else inside the sweeps writes only velocities and impulses:**
    - `apply_impulse` `contact.rs:108-111`;
    - `apply_impulse_blend_x8` `simd.rs:697-740`;
    - the kernel scatter `colored.rs:2636-2649`;
    - gravity `simd.rs:171-187`;
    - the warm apply `:3562-3565` (L11 D7 is velocity-only);
    - the frozen restore, after the loop (`:3645-3651`).
  - **The substep order** (`:3547-3617`): gravity → warm apply → biased sweep → integrate + `refresh_inertia` → relax × R, then restitution (`:3619-3623`) and the store.
  - **`simd_solve` is fixed for the whole step** (`:3514`). Under it every colour, narrow or wide, runs `solve_color_avx2` through `solve_color_dispatch` (`:1986-2008`, `:2758-2769`, `:2885-2896`, `:3089-3100`, `:3119-3130`). Off, or on a non-AVX2 build, the scalar `solve_color` runs.
  - **Parallel SIMD chunks own whole cohorts:** cuts step by `COHORT` from `g_lo` (`:3008-3017`); L11 D6 keeps this.
  - **Ordering between sweeps** is only the per-colour scope join, or program order on the inline paths. L11's impulse lanes already rely on it across sweeps (`:2611-2621`).
  - **The scalar path is pinned op for op to the kernel** (`simd.rs:624-634`), and the scalar↔SIMD differentials (`colored_tests.rs:1454`, `:1626`, `:2009`, `:2101`) compare full outputs.
- **Binding rules:**
  - bit identity across W ∈ {1, 2, 4, 8, 16} and scalar/SIMD;
  - replay on any machine without id identity: L12 holds no state across steps and no ids;
  - principle 0: a `ScratchColumn` owned by the solver resource;
  - no allocation on the hot path; lock-free;
  - no FMA (`FMA-DETERMINISM.md`; the source census at `colored_tests.rs:3221` covers the new code automatically);
  - every gate can fail;
  - the P0 claim rule.

## Key decisions

### D1 — The epoch key is one bit, cleared by every writer of a mass input
- **What:**
  - `MassSchedule { valid: bool }`.
  - `writer()` sets `valid = false`. It is called by `build_bodies` and by a new wrapper `refresh_inertia_epoch`, which replaces the direct call at `:3597-3600`.
  - `sweep(cache) -> load` returns `cache && valid` and then sets `valid = true`.
  - Loads at (S, R): (4, 2) → 7; (4, 1) → 3; R = 0 → 0; in general S·R − 1 for R ≥ 1.
- **Why:**
  - Exact by construction. It is Jolt's rule — reuse exactly over the interval in which inertia is constant (`AxisConstraintPart::mEffectiveMass` over the velocity steps; recompute in the position phase) — keyed to boyko's refresh every substep.
  - It costs 12 bool operations per step, has no per-body test, and holds for any S and R.
- **Rejected:**
  - Once-per-step masses (Box2D, Box3D, Rapier, Avian) change values, because inertia is refreshed every substep (L11 D8).
  - A per-body change detector (compare the inertia bits at each refresh; skip cohorts whose bodies did not change) costs a 36-B read per body per refresh plus a data-dependent branch per cohort, and it gains only on non-rotating bodies.
  - A fixed table of load-sweep indices breaks as soon as S or R changes.
- **Trade-off:** correctness rests on "no other writer inside an epoch". D7 turns that premise into a test.

### D2 — The first sweep of an epoch computes and stores; there is no separate mass pass
- **Why:**
  - A mass pass after each refresh would re-read ra/rb/n/t1 (0.92 MB at J) and gather the bodies again: a sixth kernel-sized stream per epoch.
  - It would also be serial, or else add 5 `pool.scope`s per step (the L5 D8 exception class).
  - Computing epoch 0 in L11's fill moves 1/12 of the mass work into a pass that is serial at W>1 (without L11 C4). That is a loss at W=8.
- **Trade-off:** biased(0), and at R = 1 the last relax, store masses that nobody reads. The cost is ≤ 2 µs per step (2,320 × 3 stores, arith.). It is accepted so that there are only two kernel variants (D4).

### D3 — The masses live in a separate column of 32-B lanes, one 64-B-aligned run per cohort
- **What:**
  - Run of cohort k = `[C cohort-level lanes][R_g lanes per rank × depth(k)]`, rounded up to an even lane count.
  - Its start is `CohortHead.mass_base`, taken from L11's `_pad: [u8; 16]`.
  - L11 shape: C = 0, R_g = 3 (n, t1, t2).
  - L8 shape: R_g = the per-point masses L8's kernel computes (1 if friction is per patch); C = its per-manifold masses (a symmetric 2×2 tangent inverse = 3 lanes, plus a twist mass = 1).
- **Why:**
  - **Bytes:** +96 B per rank plus ≤ 32 B of pad per odd-depth cohort. At J that is 3 × 2,320 + ≈ 286 lanes = 7,246 × 32 B ≈ **232 KB**, +31 % on the kernel's 0.92 MB stream. Putting the masses inside `RankBlock` would need 320 → 448 B to stay line-aligned: +40 %, paid by every pass that walks blocks (the oracle, restitution, the store).
  - **False sharing:** chunk cuts fall on cohort boundaries and runs are line-aligned, so no 64-B line is written by two tasks. An unpadded 416-B in-block row would straddle lines, so the impulse lanes of adjacent chunks would share lines in every sweep.
  - **Isolation:** only the kernel touches the column. L11's `RankBlock`, its size assertions and G3 are unchanged apart from 4 B of head pad.
  - **Generality:** L8's per-cohort masses have a home at the front of the run. Per-rank 128-B rows would not give them one, and would cost +33 % of the mass bytes.
- **Rejected:**
  - in-block 416 B or 448 B (above);
  - 128-B rows (above);
  - masses in the head (the head would become mutable, and L11's G3 is written for a read-only head).
- **Trade-off:** one more sequential stream per worker, and one head field. If L8 uses the head pad, `mass_base` moves into a head extended to 384 B (+36 KB at J).

### D4 — Two monomorphised kernels via `const LOAD: bool`; the compute variant always stores
- **Why:**
  - Each variant gets its own register allocation. The kernel is spill-heavy (51 spill stores and 58 reloads per rank in the transcription, `FMA-DETERMINISM-MEASUREMENTS.md:322-324`), and a runtime flag inside the rank loop would force one allocation on both paths, with unswitching left to LLVM.
  - The variants run in disjoint sweeps. Both fit in the 32-KB L1i next to the substep loop's other kernels: post-L11 each is ≈ 4–5 KB, arith. from 1,710 instructions today minus the gather.
  - Worst case, a refill is ≈ 160 lines × ≈ 12 cycles ≈ 0.5 µs per variant switch; with ≈ 8 switches per step that is ≤ 4 µs.
- **Rejected:** a third, no-store compute variant (saves ≤ 2 µs for a third I-cache footprint).

### D5 — The mass stores are full-vector and unblended
- A padding rank's mass (1/(mA+mB)) and an absent lane's mass (0) are stored and loaded exactly as computed. They are consumed only by inactive-lane arithmetic, and the kernel's outputs already mask that: the impulse store is `blendv(old, new, active)` and the velocity blend uses `active ∧ movable` (`:2481-2482`).
- The column is transient: it is never persisted and never part of a pin. L11 G3's "padding bytes are zero" covers heads, blocks and records, not this column.
- A blend would put 3 more µops per rank on FP0/FP1, which is exactly the resource L12 frees.

### D6 — The scalar oracle and restitution keep recomputing
- With `solve_color` recomputing, every scalar↔SIMD differential compares a caching kernel against a recomputing reference. That is the strongest available detector of a stale or misindexed mass.
- Restitution's span is 0.008 ms per step (P0 J-A-a). Loading there would tie a scalar pass to a column that exists only under SIMD.
- The scalar path gains nothing. `simd_solve` is the default; cfg-A is a control.

### D7 — A census of epoch writers
Every write of `inv_mass` / `inv_inertia` in `solver/colored.rs` sits inside `build_bodies` or `refresh_inertia_epoch`, and both call `writer()`. A source census test enforces this (gate G4). A future lane that adds a writer in mid-epoch then fails a test instead of loading stale masses silently.

### D8 — Inlining is settled before the A/B (C0 and a conditional C0b)
- **The problem:**
  - The 2026-09-02 census (codegen-units 16, no LTO) shows `effective_mass_x8` as its own symbol, with one `vdivps` in the kernel (`FMA-DETERMINISM-MEASUREMENTS.md:124-125`). The masses were out-of-line calls there.
  - Such a call passes 29 `__m256` values by pointer (rust-lang #116558).
  - On Win64 the upper halves of the ymm registers are volatile, so the caller also spills every live ymm across the call.
  - L12 would remove that overhead in 7 of 12 sweeps; an `#[inline]` hint would remove it in all 12. So without C0b, L12's A/B would measure two levers at once.
- **The fix:**
  - C0 takes a disassembly census of the **parity** binary (fat LTO, which may already inline).
  - If the calls are still there, C0b adds `#[inline]` as its own measured commit (principle 7).
  - `#[inline(always)]` is not allowed together with `#[target_feature]` on stable Rust; `#[inline]` is.

### Rejected, because they change values
- Jolt's cached I⁻¹(r×axis): boyko applies I⁻¹·(r×(dir·λ)) (`contact.rs:110`), and the two round differently.
- The cheaper rn·(I⁻¹·rn) form: 29 ops instead of 38, but equal only mathematically.
- Caching across steps.

## Data structures
```rust
/// One 8-lane effective-mass vector, lane l = cohort lane l. A cohort's run starts on a 64-B line.
#[repr(C, align(32))] #[derive(Clone, Copy)]
struct MassLane([f32; 8]);                          // 32 B; const-asserted size and align

// L11 CohortHead (320 B, size unchanged):
//   _pad: [u8; 16]  →  mass_base: u32 /* first MassLane of this cohort's run; even */, _pad: [u8; 12]

// L11 CohortColumns gains:
//   mass: ScratchColumn<MassLane>   // HOT in the kernel only. len = Σ_k even_up(C + R_g·depth(k)).
//                                    // Sized in P-b, only under cfg(avx2) and simd_solve.
const MASS_LANES_PER_COHORT: usize = 0;             // L8: its per-manifold mass count (e.g. 4)
const MASS_LANES_PER_RANK: usize = 3;               // L8: 1 if friction is per patch

// ColoredSoftStepSolver gains:
//   mass: MassSchedule          // { valid: bool }. Cleared by build_bodies and refresh_inertia_epoch.
//   mass_cache: bool            // default true; false = every sweep computes (A/B, rollback)
//   #[cfg(test)] mass_changed_lanes: AtomicU64   // G2 witness: active lanes whose stored bits differ from the previous epoch's
//   #[cfg(test)] mass_loaded_ranks: AtomicU64    // G2 witness: ranks served by a load
```
- **Reserve:** `3 × L11's block reserve + the cohort bound` lanes. This is address space only.
- **Id:** one id from the contact-band headroom L11 freed (`CONTACT_COLUMN_COUNT` 12 → 13 in L11's numbering, `scratch_ids.rs:69`). It stays in the contiguous solver cohort, so the width assert at `:197-202` still proves the stagger.
- **Base:** 64-B aligned, as L11 debug-asserts at construction.
- **Drop:** none (POD).
- **Send/Sync:** L11's `CohortSolveView` gains a raw `mass` base, taken from the column's mutable raw base, never from `as_read_slice().as_ptr().cast_mut()`. Its `unsafe impl Send + Sync` keeps L11's argument and adds run disjointness (see the multithreading model).

## Public API
- `ColoredSoftStepSolver::with_epoch_mass_cache(enabled: bool) -> Self`: a test/bench hook that mirrors `with_warm_start` (`:1515-1517`). The default is on. There is no other public change: `PhysicsConfig` is untouched, so save and replay formats are untouched.

## Algorithms for the critical paths

| path | steps | cost | cache / branches / SIMD |
|---|---|---|---|
| schedule, `solve_colored` | `mass.writer()` in `build_bodies`; per substep: biased with `load = mass.sweep(cache)`; `refresh_inertia_epoch` → `writer()`; relax × R, each with `load = mass.sweep(cache)` | 12 bool operations per step | one branch per chunk call selects `::<LOAD>` |
| P-b (L11) | per cohort, in order: `mass_base = acc`; `acc += even_up(C + R_g·depth)`; then `mass.resize(acc)` (grows only on a new high-water) | O(cohorts), serial | one u32 per head |
| kernel, compute variant | exactly today's op order; after each `effective_mass_x8`, an `_mm256_store_ps` to lane `mass_base + C + R_g·r + d` (L8: cohort-level lanes at cohort entry) | +3 stores per rank | temporal stores: re-read 1–2 sweeps later; NT stores would push the lines to DRAM and turn L2/L3 hits into DRAM reads |
| kernel, load variant | 3 × `_mm256_load_ps` from the same lanes; the rest is unchanged | −246 FP ops, +3 loads per rank | sequential per chunk; hardware prefetch, no software prefetch; 32-B-aligned accesses never split a line |
| scalar oracle, restitution, store | unchanged | — | never touch the column |

## Expected gain (arith., shown)
**Per-rank budget on L11's kernel shape** (op mix from the research, `colored.rs:2427-2621` plus the helpers; Zen 3: 2 multiply pipes, 2 add pipes, divider throughput 5, 1 × 256-bit store per cycle):

| resource | compute sweep | load sweep |
|---|---|---|
| FP0/FP1: multiplies + blends | 277 + 29 → **153 c** | 133 + 26 → **79.5 c** |
| FP2/FP3: add + sub | 201 → 100.5 c | 108 → 54 c |
| divider | 5 × 5 → 25 c | 2 × 5 → 10 c |
| dispatch: ≈ 830 → ≈ 585 instructions (1,309 today minus the ≈ 480-instruction gather L11 deletes) | ≥ 138 c | ≥ 97 c |
| velocity chain (normal ≈ 62 + friction ≈ 93) | ≈ 155 c | ≈ 155 c |

**The load side.** 3 loads and 96 B per rank, against 246 FP ops, 73.5 mul-pipe cycles and ≥ 41 dispatch cycles. At W=1 the column streams from L3 (the kernel's 1.15 MB exceeds the 512 KB L2): 96 B / 32 B per cycle ≈ 3 c per rank. **The recompute is never cheaper than the load traffic.** It can only be *hidden*: post-L11, rank r+1's normal mass sits about 160 instructions after rank r's friction chain begins, which is within the 256-entry ROB. It is hidden if the kernel runs at its chain bound.

**The model.** Per load-sweep rank, Δ = min(M, max(0, t_rank − c_chain)), where:
- M = 41 c (dispatch floor, 246/6) to 73.5 c (mul pipes);
- c_chain ≈ 155 c;
- t_rank = the parent's measured cycles per rank-sweep.

Per step at W=1: N_L = 7 × 2,320 = 16,240 load-sweep ranks, converted at the P0 clock band of 3.3–4.6 GHz (P0 does not record the clock).

| parent t_rank | regime | Δ per load rank | Δ per step, W=1 | µs per manifold (÷ 4,524.2) |
|---|---|---|---|---|
| 320–530 c (L11's prediction: 2.68–3.22 ms / 12 / 2,320) | issue-bound | 41–73.5 c | **0.145–0.36 ms** | **0.032–0.080** |
| ≈ 200 c | mixed | ≤ 45 c | ≤ 0.16–0.22 ms | ≤ 0.035–0.049 |
| ≤ 170 c | chain-bound | ≤ 15 c | ≤ 0.05–0.07 ms | ≤ 0.012–0.016 |

- **Overheads to subtract:**
  - stores: 5 × 2,320 × 3 = 34.8k store slots, ≤ 8–11 µs, and only if store-bound (≈ 9 stores per rank against ≥ 155 c: it is not);
  - traffic: (5 + 7) × 232 KB = 2.78 MB per step, ≤ 19–26 µs if fully exposed (it is prefetched).
  - Net lower bound in the issue-bound row: 0.108 ms (0.024 µs per manifold).
- **The FMA null result does not discriminate between the regimes** (−145 add-pipe ops, −12.6 % instructions, 0.98–1.01×, `FMA-DETERMINISM-MEASUREMENTS.md:322-340`). FMA runs on the multiply pipes and lengthens the chain (latency 4 against 3), while L12 removes multiply-pipe work and adds no latency. It also measured today's kernel with the gather, which L11 removes.
- **Post-L8.** Mass work shrinks by ×0.56–0.69 (research, Box3D shape), and L8 reshapes the chain. So the issue-bound row becomes ≈ 0.06–0.25 ms (0.014–0.055 µs per manifold). C0 redoes this table on L8's kernel.
- **Relative to today:**
  - The kernel is 0.790 µs per manifold (J-B wide colours 3.576 ms / 4,524.2) and 0.59–0.71 after L11. The whole solve is 1.24 µs per manifold against Jolt's 1.30–1.49.
  - On T(1) (L11's projection is 8.67–9.79 ms), L12 is 1.5–4.2 % in the issue-bound regime.
- **T(W):**
  - W=8, with `parallel_solve` on and E(8) = 0.467: ÷ 3.74 → 0.039–0.096 ms, which is 0.8–2.4 % of the projected T(8) of 3.93–4.69 ms.
  - W=16 (SMT): the Zen 3 ROB is partitioned between the two hyperthreads and FP0/FP1 are shared, which pushes the kernel toward the issue-bound regime. Reported, not claimed.
- **Build-if (decided on T1, before any receipts):**
  - Condition: Δ_T1 − 11 µs ≥ the K=24 end-to-end SE bar on the parent's T(1).
    - With SD ≈ 1.38 % (P0 §9: a 2.0 % bar at K=6), SE = 1.2533 · 1.38 / √24 = 0.353 %, so the bar is 2√2 · 0.353 ≈ **1.0 % of T(1)**, about 0.07–0.09 ms.
    - The chain-bound row fails this condition; it is the case where the recompute costs nothing.
  - **Stop rule at C0:** if the post-L8 t_rank < c_chain(L8) + 20 c, stop. Δ could not reach the bar, and L12 is closed without writing C1.

## Determinism (bit identity)
1. **Constant inputs.** Within an epoch, `n`, `t1` and `ra`/`rb` (written by L11's fill before the loop) and `inv_mass`/`inv_inertia` (writers listed above, enforced by D7) are bitwise constant. `t2 = cross8(n, t1)` is the same op in every sweep. The body state is gathered at cohort entry, and within a sweep only velocities are written.
2. **Same operations.** The compute variant runs today's `effective_mass_x8` unchanged. Rust floats are IEEE with round-to-nearest, no contraction and no flush-to-zero (RFC 3514). `_mm256_mul_ps` and `_mm256_add_ps` lower to plain `fmul`/`fadd`. rustc on the MSVC host still generates code through LLVM, so `/fp:contract` does not apply. `x86-64-v3` enables the FMA ISA but not fusion. The source census covers the file.
3. **Storage is exact.** A 32-B aligned store and load moves bits. MXCSR's DAZ/FTZ affect arithmetic, not moves.
4. **A load happens only in the epoch of its store.** This is gate G1's oracle. The step start clears `valid`, so nothing is reused across steps.
5. **Across threads.** A mass computed on worker X is read on worker Y after a scope join. This relies on the pool sharing one MXCSR (L5 O2's assertion) to exactly the degree today's {1, N} identity already does.
6. **Scalar against SIMD.** `simd_solve` is fixed per step and the scalar path never reads the column, so the existing scalar↔SIMD differentials compare the cache against a recompute.
7. **No W dependence.** Runs are a pure function of the layout P-b computes; ownership per cohort does not depend on W.

## Multithreading model
- **Shared and read-only during a sweep:**
  - heads (including `mass_base`);
  - the blocks' geometry;
  - the bodies' `inv_mass` and `inv_inertia`;
  - in load sweeps, the cohort's own mass run.
- **Exclusive per task:** the runs of the task's cohorts, written only in compute sweeps. Runs are disjoint prefix-sum intervals, and a cohort belongs to exactly one task per sweep (the COHORT-stepped cuts).
- **Synchronisation:** no new mechanism. The existing per-colour scope join, or program order on the inline, `lanes < 2` and no-pool paths, orders a compute sweep's stores before the next sweep's loads, exactly as for L11's impulse lanes.
- **False sharing:** none. Runs start on a 64-B line and chunk boundaries are cohort boundaries.
- **Atomics:** none in release. The `#[cfg(test)]` witnesses do one `fetch_add(Relaxed)` per kernel call and are read after the join. Loom is N/A.
- **Data-race freedom:** disjoint runs + one owner per cohort per sweep + a join between sweeps. Miri gate G8b checks the cross-thread hand-off.

## Integration: files and lines

| file | lines (today; L11 name) | change |
|---|---|---|
| `solver/colored.rs` | `:1443-1481`, `:1495-1513`, after `:1515` | fields `mass`, `mass_cache` and the test witnesses; `with_epoch_mass_cache` |
| | `:1542-1564` `build_bodies` | calls `self.mass.writer()` |
| | `:1974-2024` `solve_color_dispatch` | new `load: bool` → `solve_color_avx2::<true/false>`; the scalar fallback ignores it |
| | `:2228-2653` kernel (L11's cohort kernel) | `const LOAD`; the mass sites at `:2485` and `:2540-2541` become compute+store or load; the SAFETY block is extended |
| | `:2708-2772`, `:2845-3132` | `load` is threaded through `solve_all_colors` and `solve_color_parallel` (`:2758`, `:2885`, `:3089`, `:3119`) |
| | `:3459-3472`, `:3568-3580`, `:3597-3600`, `:3604-3616` | the schedule; `refresh_inertia_epoch` wraps `simd::refresh_inertia` |
| | L11 `CohortHead`, `CohortColumns`, P-b | `mass_base`; the `mass` column; sizing |
| | `:3141-3197` restitution | a doc line only (it recomputes, D6) |
| `solver/simd.rs` | `:624-634` | doc: "evaluated in compute sweeps; loaded in the others" |
| | `:635-638` | C0b only: `#[inline]` |
| `scratch_ids.rs` | `:69`, `:1110-1125` | +1 contact column; id census |
| `solver/colored_tests.rs` | L11-ported `:1454-2181` | direct kernel calls get `::<false>`; new tests G1–G5 and G8 |
| `benches/l12_epoch_mass.rs` (new) + `Cargo.toml` after `:246` | — | the T1 probe, `harness = false` |
| `docs/SYSTEMS.md`, `docs/FEATURE_MAP.md` | colored solver entries | the epoch cache |

`profiling.rs` does not change. Per-mode spans are derived from the existing zones (T1/T2).

## Commit sequence (each commit green)

| commit | content | values |
|---|---|---|
| **C0** | Receipt only, on the post-L8 tip, parity profile, MSVC host. (a) Disassembly census of both future kernel sites: `vdivps` per symbol, calls to the mass helpers, spills per rank. (b) t_rank from L8's armed receipts: (wide + narrow) / sweeps / ranks. (c) The regime table redone on L8's kernel. (d) The stop rule. | none |
| **C0b** | Only if (a) finds calls: `#[inline]` on each out-of-line mass helper. Its own P0-protocol A/B: not claimed slower at any W; a gain claimed if seen. | none |
| **C1** | D1–D7 plus the T1 bench; G1–G5, G7 and G8 pass; M1–M11 recorded red | none |
| **decision** | T1 at K=12. Continue iff the build-if holds; otherwise the lane stops and the T1 receipt closes L12 | — |
| **C2** | T2 receipts, the MEASUREMENT-QUEUE result block, the per-contact table, docs | — |

## Gates
- **G1, schedule oracle** (unit test, C1).
  - For S ∈ 1..=6, R ∈ 0..=4 and cache ∈ {on, off}, the `MassSchedule` sequence must equal a brute-force oracle over the event list [writer, (biased, writer, relax × R) × S]. The oracle's rule: a sweep loads iff the cache is on and an earlier sweep happened after the last writer.
  - Counts are asserted: (4, 2) → 5/7, (4, 1) → 5/3, (4, 0) → 4/0; with the cache off, 0 loads.
- **G2, stale-epoch value gate** (unit test, C1).
  - Scene: L11's S-a (a 6-layer box pyramid) with an initial angular velocity on every box, so the inertia bits change every substep; 60 steps.
  - Compared body bits: the scalar oracle, SIMD with the cache on, and SIMD with the cache off, at W ∈ {1, 2, 4, 8, 16}. All must be equal.
  - **Anti-vacuity:** `mass_changed_lanes > 0` and `mass_loaded_ranks > 0`.
  - **Red-first:** the same scene with ω = 0 and identity rotations must turn the witness assertion red (M7).
- **G3, column layout** (C1).
  - Every `mass_base` is even; runs are disjoint and cover `[0, len)`.
  - The live lanes (pads excluded) hash equal across W ∈ {1, 2, 4, 8, 16} at the end of each S-a step.
- **G4, epoch-writer census** (C1). A source scan of `colored.rs`: `refresh_inertia(` calls and writes to `inv_inertia` / `inv_mass` appear only in `build_bodies` and `refresh_inertia_epoch`.
- **G5, kernel differential** (C1).
  - `load_variant_matches_compute_variant`: on the `cohort_shape_proptest` generator (200 cases; statics, sentinels, k ≤ 0 lanes, ragged widths), a compute sweep followed by a load sweep must be bit-equal to two compute sweeps (impulses and velocities).
  - Non-vacuity: the clamped and zero-mass lane counts are asserted > 0, as in the existing test.
- **G6, poses and pins:**
  - runner `--expect-pose` for every row at W ∈ {1, 8};
  - `default_world_pyramid_determinism`, `bodytype_determinism_golden`, the goldens;
  - `cargo test --workspace --all-targets --no-fail-fast` with **zero pin moves**;
  - A7-R1 drift, freeze steps, the sleep suites and incline stick/slip pass with their assertions byte-unchanged.
- **G7, census:** 0 heap allocations per step after warm-up; scope, chunk and zone-count pins unchanged (`alloc_frame_census`, `profiling_zone_counts`).
- **G8, Miri:**
  - (a) a single-thread compute sweep then a load sweep at small n;
  - (b) two `std::thread::scope` workers over two cohorts, compute sweep then load sweep, with the cohort→worker assignment swapped between them.
- **G9, FMA:** the source census covers the new code automatically. C0's binary census lists both monomorphs with 0 fused ops.

**Named mutations, each recorded red before C1 lands:**

| id | mutation | red in |
|---|---|---|
| **M1** (ruled: "load a stale epoch's mass") | `refresh_inertia_epoch` does not call `writer()`, so relax and later biased sweeps load epoch s−1 | G2, G6 goldens and poses |
| M1′ | `MassSchedule::writer` is a no-op | G1, G2 |
| M2 | the load reads rank r−1's lanes | G5, G2 |
| M3 | t1 and t2 lanes swapped on load | G5, G2 |
| M4 | the step start does not clear `valid` (biased(0) loads the previous step's epoch-4 masses) | G1, G2 |
| M5 | `mass_base` not rounded up to even | G3 (alignment) |
| M6 | runs overlap by one lane | G3 (hash across W), G8b (data race), G2 |
| M7 | the gate scene without rotation | G2 witness (anti-vacuity) |
| M8 | `simd::refresh_inertia` called directly in the loop | G4 |
| M9 | the compute variant skips the store | G2, G5 |
| M10 | the view's mass base taken from `as_read_slice().as_ptr().cast_mut()` | G8 (Tree Borrows) |
| M11 | P-b builds the prefix sum in a `Vec` | G7 |

**Timing:**
- **T1, the in-binary probe** (C1; bench `l12_epoch_mass`, `harness = false`, parity profile).
  - Setup: J at the tip's default configuration, run to step 200. Its `Manifolds`, `ConstraintGraph` and bodies are captured. Two solvers, `with_epoch_mass_cache(true)` and `(false)`, are interleaved per iteration on restored inputs; 200 iterations; at W=1 inline and at W=8 on a pool.
  - Every iteration asserts equal post-solve body bits between the two arms; a mismatch voids the process.
  - Statistic: the median over K=12 processes of the per-process mean Δ of `PHYS_PASS_BIASED + PHYS_PASS_RELAX`, with SE per the P0 rule. A canary arm (+20 µs spin after the solve) must be recovered within SE.
  - Per-mode spans: B_on = c′ + 3l and Rl_on = 4c′ + 4l, with c′ = B_off / 4. Two estimates of l act as a consistency check.
- **T2, the claim** (C2; P0 protocol).
  - Parent (post-L8 tip, with C0b if it landed) against the L12 tip, both parity profile on the MSVC host. K=24 at W=1 on J, K=12 elsewhere, interleaved, with per-process receipts and the canary seen.
  - Rows: J at the default configuration, W ∈ {1, 2, 4, 8, 16}; R at W ∈ {1, 8}; cfg-A (`simd_solve` off) at W=1 as a code-layout control; armed J at W ∈ {1, 8}.
  - **Claims:**
    - (a) **per contact, W=1:** (pass_biased + pass_relax) / manifolds is reduced, claimed, and **≥ 0.6 × Δ_T1**. The denominator is identical in both arms, because the lever is bit-identical;
    - (b) T(1) on J is reduced, claimed at K=24;
    - (c) W ∈ {2, 4, 8, 16} and R are not claimed slower; the W=8 gain is reported, and claimed only above the bar;
    - (d) cfg-A shows no claimed difference. A claimed difference there is a code-layout effect and is reported, not counted as a gain.
  - The per-contact table lists µs per manifold for broadphase, narrowphase, setup, kernel and other, before and after; only the kernel may move.

## What moves, and how it is re-measured
- **Values:** nothing. A red pin is a defect, never a re-bless.
- **Layout:** `CohortHead` gains `mass_base` and keeps 12 B of pad. L11's head padding-zero debug assertion now covers 12 B. L11 G3 compares across W rather than against pinned bytes, and C0's `setup_digest` hashes logical values, so neither moves.
- **Tests:** direct kernel callers gain `::<false>`; no assertion changes. The scratch-id census re-runs with +1 contact column.
- **Memory receipt** (`committed_rows`): ≈ +232 KB at J on L11's shape, ≈ +156 KB on the post-L8 shape.
- **Docs:** `SYSTEMS.md`, `FEATURE_MAP.md` and the result block. The book is doc-writer only.

## What it cannot claim
- Any change in stability or accuracy. None is possible by construction.
- A W=8 gain unless measured above the bar (arith. ≤ 0.1 ms).
- Per-contact parity with Jolt v5.6.0. Collision detection dominates that gap.
- How the gain splits between latency, pipe and dispatch effects. Only totals are measured.
- Any gain with `simd_solve` off, on non-AVX2 builds, in `SoftStepSolver` or in restitution.
- Any number on L8's kernel before C0. This document's arithmetic is on L11's shape; post-L8 it is a scaling estimate.

## Open questions (orchestrator)
1. **L8's mass shape.** L12 needs L8 to state which masses its kernel computes per rank and per cohort. I recommend that L8 computes them in every sweep from the per-epoch inertia, exact to the substep refresh. With L12, 7 of the 12 computations become loads. A Box3D-style fill-time friction mass (step-start inertia) would be a further value change and would take away what L12 caches.
2. **Cost of the K=24 row** at W=1 on J, which the 1 % end-to-end bar requires.
3. **The public hook `with_epoch_mass_cache`** (the precedent is `with_warm_start`): acceptable, or `#[doc(hidden)]`?
4. **C0b in this lane:** an `#[inline]` micro-lever with its own receipt, if C0 finds out-of-line mass calls.
5. **The stop rules:** confirm that failing C0's t_rank test or T1's build-if closes L12 with only the C0/T1 receipts and no T2.

## Files
- D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/colored_tests.rs
- D:/wt/joltab/crates/boyko_physics/src/scratch_ids.rs
- D:/wt/joltab/crates/boyko_physics/Cargo.toml
- D:/wt/joltab/crates/boyko_physics/benches/colored_solve.rs (precedent for driving the solver directly)
- D:/wt/joltab/docs/physics/FMA-DETERMINISM-MEASUREMENTS.md
- D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/00-RULINGS.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/02-DESIGN-REV1.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/03-REVIEW-OF-REV1.md

Sources:
- [Jolt AxisConstraintPart.h](https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ConstraintPart/AxisConstraintPart.h)
- [Jolt ContactConstraintManager.cpp](https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.cpp)
- [Box2D contact_solver.c](https://raw.githubusercontent.com/erincatto/box2d/main/src/contact_solver.c)
- [Box3D contact_solver.c](https://raw.githubusercontent.com/erincatto/box3d/main/src/contact_solver.c)
- [Rapier contact_with_coulomb_friction.rs](https://raw.githubusercontent.com/dimforge/rapier/master/src/dynamics/solver/contact_constraint/contact_with_coulomb_friction.rs)
- [Bepu changelog](https://raw.githubusercontent.com/bepu/bepuphysics2/master/Documentation/changelog.md)
- [uops.info VDIVPS ymm](https://www.uops.info/html-instr/VDIVPS_YMM_YMM_YMM.html)
- [uops.info VBLENDVPS ymm](https://www.uops.info/html-instr/VBLENDVPS_YMM_YMM_YMM_YMM.html)
- [Rust RFC 3514 float semantics](https://rust-lang.github.io/rfcs/3514-float-semantics.html)
- [Rust reference, codegen attributes](https://doc.rust-lang.org/stable/reference/attributes/codegen.html)
- [rust-lang/rust #116558](https://github.com/rust-lang/rust/issues/116558)
- [MSVC x64 calling convention](https://learn.microsoft.com/en-us/cpp/build/x64-calling-convention?view=msvc-170)
