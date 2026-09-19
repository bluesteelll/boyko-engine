VERDICT: CHANGES REQUESTED; BLOCKING=0; IMPORTANT=3

# Architecture review: L5 parallel narrowphase, with L4 and L2 as the first commits of the lane

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED. There are no blockers. Three important items are open, and each fix is small.

I checked the plan against the tree at `D:/wt/joltab`. The core of the plan holds: the determinism argument, principle-0 placement, the allocation accounting and the gain arithmetic. I found no case where pair order, manifold order or axis-table state depends on W or on timing.

## Remarks

### 🔴 Critical
None.

### 🟡 Important

#### W1. The W=1 inline claim for L5 has no gate, and the algorithm section leaves out the `lanes < 2` test
- **Where:** "Algorithms", `try_parallel`: `c = chunk_count(n, pool.num_threads()); if c < 2 { return 0 }`. This contradicts D4's list of inline conditions, which includes `lanes < 2`. The claims affected are "W=1: 0 (inline path)" and "at W=1 the only new work is one thread-local pool probe per step".
- **Problem:** The formula gives C = min(1 × 6, 9561 / 128) = 6 at W=1, so the sketched algorithm dispatches at W=1.
  - Nothing in the gate list can see that:
    - G-L5-3 asserts `narrowphase_dispatches` +1 only on W ≥ 2 arms.
    - G-L4-1 covers the solver only.
    - The S1b arm (W=4) sets the flag off.
    - The runner "re-implements `chunk_count` from the exported constants". If both copies omit the `lanes` term they agree, so nothing is voided.
    - At W=1, G-TW's bars (1.4–2.0 % IQR/SE ≈ 280–400 µs) are far above the ~10 µs cost of a scope.
- **Consequence:** From C4 on, every default world at W=1 would open 1 `Box<ScopeShared>`, 1 or more `ScopeBlock` chunks and 6 spawns per step. That is exactly the W=1 cost L4 removes from the solver with a red-first gate, and every listed gate would stay green.
- **Confidence:** CONFIRMED. The plan text is inconsistent between D4 and the algorithm. The shape of the omission matches today's solver cut (`D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs:2927-2945`), which has no lanes test either; that is why L4 needs its gate term.
- **What is needed:**
  - Make the algorithm section match D4.
  - Add a narrowphase twin of G-L4-1: a 1-worker, flag-on arm asserting a `narrowphase_dispatches` delta of 0 (G-L5-3/G-L5-4 are the natural place). Name the mutation "drop the lanes term", which must turn it red.
  - The runner's copy of `chunk_count` must carry the same term.

#### W2. C1 silently turns the J-P1 runner row (`--parallel-solve` at W=1) into a vacuous measurement
- **Where:** C1 integration. It edits only the runner doc at `jolt_parity_pyramid.rs:61-62`. The flag doc at `:131` ("force parallel_solve on (J-P1 at W=1 …)") is left as is.
- **Problem:**
  - After C1, `parallel_solve` at W=1 always takes the inline path.
  - The runner's `waves` is the sample count of the `PHYS_COLOR_WIDE` zone (`jolt_parity_pyramid.rs:1300`). The solver opens that zone by colour class, not by whether it dispatched (`colored.rs:2731-2736`, before `if parallel`).
  - So a J-P1 row on a post-C1 binary still reports 109 waves while opening zero scopes, and gives ω₁ ≈ 0.
- **Consequence:** ω₁ is the input to the ruled L6 fork (00-RULINGS W2) and the basis of the plan's own "J-P1 +0.60 %" price. Any re-profile after C1 (the rulings say "re-profile after each" lever) gets a confident, wrong ω₁ with no void.
- **Confidence:** CONFIRMED (file lines above).
- **What is needed:** Retire J-P1 in C1 and state that ω₁ now comes from the zero-work spawn/join microbench the ruling allows. Alternatively, give the runner a colour-dispatch witness and void W=1 `--parallel-solve` rows.

#### W3. `fingerprint()` as specified ("FNV over slot bytes") reads uninitialised padding
- **Where:** Public API (`BoxAxisCache::fingerprint`); G-L5-1/G-L5-3 ("table bytes equal"); the wording of Lemma 2 and the Theorem.
- **Problem:** `AxisEntry` is `{ key: u64, axis: u32 }` with no `repr(C)` (`D:/wt/joltab/crates/boyko_physics/src/narrowphase/axis_cache.rs:111-117`). That is 16 bytes, 4 of them uninitialised padding.
- **Consequence:**
  - Hashing the raw bytes is UB. Under Miri it is an error that goes red for the wrong reason.
  - Natively, two tables with equal `(key, axis)` sequences can hash differently, depending on whether a slot came from a `resize` copy or a field write. That gives a spurious red in G-L5-3.
- **Confidence:** CONFIRMED.
- **What is needed:** Define the compared state as `(key, axis)` per slot plus `occupied`, and hash the fields. The lemmas then say "table state", not "bytes". The stale `axis` values left in EMPTY slots are equal on both paths, so they can stay in the hash.

### 🟢 Optional
- **O1. DISPATCH zone versus the W4 expectation.** The sketch opens `phys_np_dispatch` around `try_with_active_pool`, and that is where the `c < 2` decision is made. The W4 rule says "1 when recomputed C ≥ 2, else 0". J-D at W=1 on C4 (flag on by default) would then void every step (exit 3) in the window. Open the zone only after the decision.
- **O2. G-FP tests the wrong property.**
  - Bit identity needs every thread that runs narrowphase work to share one MXCSR. All of those threads are pool workers, not the spawner.
  - On Linux a thread inherits its creator's MXCSR, so the "spawner sets FTZ" mutation reds only on Windows.
  - Setting MXCSR is outside Rust's FP model (`_mm_setcsr` is deprecated for that reason).
  - Either assert that the workers match each other and the default, or rely on the existing pose gates across W.
- **O3. Non-vacuity items in G-L5-3 have no observable.** "≥ 1 load-clear, ≥ 1 grow" cannot be seen from an integration test: `BoxAxisCache`'s fields are private and only `remap_resets()` is public. Add a diagnostic, or assert these in a crate-internal test.
- **O4. `--expect-pose` scope.** It lists C2, C4 and every W. Extend it to C0/C1, since L4 also claims bit identity.
- **O5. The scratch-id fallback is moot, and applying it would break another assert.**
  - I computed the new layout with MAX_COMPONENTS = 512 and stagger period 64:
    - the union becomes ids 441..459 (slots {57..63, 0..11});
    - `ROW_PREV = highest_id_clear_of(416, 511, 468)` stays 403, so `AXIS_REMAP` stays 400 (slot 16), and the assert at `:680-688` holds;
    - `SLEEP_ISLAND_KEY` moves 418 → 416, still strictly between 403 and 417;
    - the row-identity cohort is 20 wide and its bottom (400) is ≥ 384.
  - Deriving `AXIS_REMAP` independently would break the tiling assert at `scratch_ids.rs:654-658`. Say this in the plan so the developer does not reach for the fallback.
- **O6. Make the broadphase order assert strict.** `systems.rs:344-347` only checks `w[0] <= w[1]`. Lemma 1 depends on pairs being unique. Today they are: AllPairs because `i < j`, Grid through the `min_shared_cell` dedup (`resources.rs:1208,1939`). If the broadphase redesign, which comes next in the lane order, ever emitted a duplicate, only W ≥ 2 flag-on runs would diverge. A strict `<` makes that a debug red.
- **O7. Census citation.** The S1c pins are at `alloc_frame_census.rs:2317-2342`, not `:2269-2300`. The debug S1c pin (385 bodies, scope 97..109) also moves by +1 scope and +c chunks.

## Positive (preserve)
- **Lemma 1 is correct, and it deletes the unconditional pre-read that rev1 prescribed.**
  - I verified it against the axis cache (`axis_cache.rs`): `probe` only reads (`:160-179`), `set` is the only writer and never moves or evicts an entry (`:319-354`), and the only clear is in `begin_frame`, which runs before the pair loop (`:261-281`).
  - Probing continues to `EMPTY` with no tombstones, and keys are unique per frame.
- **D3's skip rule is exactly the identity-`set` condition.** Lemma 2 carries over to `occupied` and to the full-table decline path.
- **D1's two-ended per-chunk runs need no key, sort or flag scan.** Rejecting in-place compaction is right: `ScratchBuildView::truncate` exists (`boyko_ecs/.../scratch/views.rs:208`), but `resize` would refill rows [M, n), about 766 KB every step.
- **The Tree Borrows discipline matches the Grid emit** (`resources.rs:1917-1919`). G-L5-2's mutation is the exact hazard documented at `scratch_column.rs:149-151`.
- **Instrumentation and control:** `parallel_narrowphase` gives a same-binary A/B, the serial loop is kept as the oracle, and zones sit only on the calling thread (review O1).
- **L4's single per-step term and G-L4-1 are mechanically valid.** Today at W=1, `lanes = 1` makes `by_lanes = 6` and `n_chunks ≥ 2`, so G-L4-1 goes red on C0. The helper counts allocations on the calling thread.
- **L2 is honest:** it states its price and does not invent a disparity classifier.
- **"What it cannot claim" is complete.**

## Open questions for the architect
1. **D8 against the binding rule.** The rules say "no allocation on the hot path" with no written exception. The +1 scope per step needs an explicit orchestrator ruling, not only a statement in the plan.
2. **Parallel-compaction trigger (your open question 2): I contest the 1 % threshold.** Use the campaign's build-if rule instead (≥ 5 % of T(W) and > 2 × the combined spread, ANALYSIS §9). After L5, 1 % of T(8) is about 69 µs, which is above the ≤ 45 µs a parallel compaction could save (your own 27–45 µs estimate), and a lever that small cannot clear the ruled 5 % build-if.
3. **L2 and ruling W3.** W3 lists L2 among the levers gated end to end. State explicitly that L2 is exempt because no measured row runs `Auto`, so an A/B would be green from emptiness.

Files cited:
- `D:/wt/joltab/crates/boyko_physics/src/narrowphase/axis_cache.rs`
- `D:/wt/joltab/crates/boyko_physics/src/systems.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/joltab/crates/boyko_physics/src/scratch_ids.rs`
- `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_census.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/default_world_worker_invariance.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/scratch/views.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs`