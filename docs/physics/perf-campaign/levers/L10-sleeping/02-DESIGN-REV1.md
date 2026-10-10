# Architecture: L10, sleeping on by default and the frozen-pair skip

## Goal
- **Functional.** Two changes:
  - `PhysicsConfig::sleeping` defaults to `true`. This is a value change the owner approved (D3).
  - A frozen island stops paying for broadphase, narrowphase, graph build and the per-point solve bookkeeping.
- **Exactness.** Neither skip mode changes a single bit. `SleepSkip::Replay` and `SleepSkip::Sets` reproduce `SleepSkip::Off` byte for byte: poses, velocities, latches, island keys, `contact_wakes` and warm seeds. Off is today's sleeping-on path.
- **Wake triggers stay exact by construction.** These are A4 (count change), wake-on-merge, `wake_all` and the energy unlatch. Today's recorded gaps are also kept exactly (see "Cannot claim").
- **Performance, derived from P0 (not measured).**

| row (P0 window) | now | C1 flip | C2 Replay | C3 Sets |
|---|---|---|---|---|
| R, W=1 | 14.226 ms | 6.049 (= measured F) | ≈0.80 | ≈0.12 |
| R, W=8 | 9.933 | 6.123 | ≈0.85 | ≈0.13 |
| J-Son tail [264,1000), W=1 | 5.334 | 5.334 (sleeping already on) | ≈0.56 | ≈0.11 |
| J-Son tail, W=8 | 5.407 | 5.407 | ≈0.60 | ≈0.11 |
| J-S0 (sleeping on, nothing freezes) | — | 0 | ≤ +0.3 % | ≤ +0.3 % |
| J (parity, sleeping off on both sides) | 9.162 at W=8 | 0 | 0 | 0 |

## Context and constraints
- **Affected:** `systems.rs` (gather, bp, np, graph, solve), `solver/colored.rs`, `resources.rs`, `narrowphase/axis_cache.rs`, `solver/warm_start.rs`, `row_identity.rs`, `plugin.rs`, the parity runner and the tests.
- **Invariants kept:**
  - IM-1: gather and apply walk every row.
  - The canonical warm store.
  - The pair list stays sorted `(min,max)`.
  - The coloring invariant.
  - The `{1,N}` bit identity of the colored solve.
  - Zero heap allocation per step.
  - Principle 0: all new state lives in `ScratchColumn`/`TouchedMask` owned by resources.
- **Lever order.** The rulings put L10 after L5 and after the broadphase redesign. This plan is written against `e2bcbcb5`, so it states the contracts that let it slot into either (§Algorithms A2, A3).
- **Allocations:** 0 per step after warm-up. Columns commit pages and do not call the heap.

## Key decisions

**D1: Replay is a cache and recompute is the oracle. Exactness is derived from inputs, not assumed.**
- **What.** A pair is replayed only when today's recompute would read bit-identical inputs:
  - both BodyStates (pose, shape, flags);
  - the same body roles (`a<b` order kept);
  - the same hysteresis hint.

  Anything else is recomputed. Recompute is today's code on today's inputs.
- **Why.** The rulings require bit identity for every non-value lever. It also lets the flip (C1) be the only commit that moves values, with every skip commit gated against the Off oracle.
- **Rejected:**
  - Jolt drop-and-cold-wake. It changes values, and B1 measured a 4.54e-2 sag against 1e-2.
  - Rapier-style change ticks. They miss `get_component_raw_mut` writes and enable-state toggles, so bit identity could not be claimed.
- **Trade-off.** One extra condition per replay (R1–R4, settled, no clear), and a field-wise bit compare of candidate rows (~0.02 ms at 1240 rows).

**D2: "Resting" is decided at the step boundary from t−1's FROZEN decision, not from the latch.**
- **What.** Row r is resting at step t iff all four hold:
  - R1: r maps to a row p of gather t−1 through `RowIdentity`.
  - R2: at t−1, p was either immovable (`bodies_prev[p].inv_mass == 0`) or frozen (`!awake_rows[p]`), with the mask stamped at t−1.
  - R3: `BodyState(r)` at t is bit-equal to `bodies_prev[p]`.
  - R4: the mask cursor classifies as Identity or Rows, not Reset.
- **Why R2 and not the latch.** Only a frozen or immovable row's post-solve BodyState equals its gathered t−1 state, which is the pose t−1's narrowphase saw. The frozen row is restored bytewise (`colored.rs:3645-3651`). The solver never writes `inv_mass==0` rows (`simd.rs:247`, write-back guards). A row latched at `end_step(t−1)` for the first time was integrated at t−1, so its pose moved.
- **Why R3.** It catches every writer: user writes, raw writes, `sync_transform_to_body`, the soft→rigid apply, a mass or shape change, and a Sensor or Simulated toggle.
- **Rejected:** ticks (D1); an explicit per-step baseline copy, which is O(immovable) memcpy on static-heavy worlds.

**D3: `SolverScratch::bodies` is double-buffered by a swap in the gather.**
- The swap is O(1), unconditional, and changes no values. R3's baseline then costs no copy.
- **Rejected:** capturing immovable rows each step (150 B × statics per step).

**D4: C2 carries the previous step's stream for one step (pairs, per-pair output index, per-manifold tag).**
- The stream is translated through the existing `RowRemap`. There is no new persistent row-keyed store.
- **Rejected:** a persistent `PairCache` now. That is U7, slot-keyed; the owner ruled "refactor last". C2 is its minimal row-keyed form, and U7 subsumes it.

**D5: C3 keeps a clean island as a Box2D-style copied set (members, kept manifolds, no-manifold pairs, warm entries).**
- Each island is copied once, at its sleep transition.
- **Rejected:** Box3D-style indices. They would point into a stream that is rebuilt every step and would dangle.
- **Trade-off.** Row-keyed renaming is O(store) on structural-change steps only, and U7's slot keys delete it.

**D6: C3's dirty test is conservative and runs at the broadphase.**
- **What.** An island is dirty if any non-sensor bounding-sphere overlap exists between a member and a non-resting row, even without a contact.
- **Why.** Clean or dirty is known before the narrowphase, so one narrowphase pass suffices and no manifold-level circularity arises.
- **Cost.** An awake body hovering near a pile makes that pile pay C2's cost. This affects performance only.

**D7: The graph over awake islands only is `is_dynamic(row) && !clean(row)`.**
- `ConstraintGraph::build` is unchanged. Colors of awake manifolds are provably unchanged (§Exactness E5).

**D8: Wake semantics are unchanged. A4 remains the trigger. No new trigger is added.**
- A4 stays non-vacuous because every input change routes to recompute (C2) or to the full path (C3, D1–D10).
- **Rejected in L10:** Rapier's write-wake and Box2D's destroy-wake. Both are value changes, so they go to Open questions.

**D9: Sleep keeps the restored at-rest velocity; it does not zero it (Jolt and Box2D do).**
- Zeroing changes values on wake. The frozen velocity is below 0.01 m/s by the threshold, and B1's wake quality was measured with today's rule.

**D10: `Sets` is disabled at setup when the SDF stage is wired (the effective mode becomes Replay).**
- The SDF stage emits per row, not per pair. `SdfField` edits carry no stamp that physics can see, so a clean island's SDF contacts cannot be proven unchanged. Replay stays exact because the SDF stage runs unchanged.

**D11: `IslandSleep`'s four `std::Vec`s become kernel columns before the flip (C0).**
- Principle 0 is binding, and the flip puts them on the default path.

**D12: Colored-path variants of the bp and np systems.**
- The reference-solver path keeps `physics_broadphase`/`physics_narrowphase` byte-untouched. `IslandSleep` and `ConstraintGraph` do not exist there, and it is D1's value oracle.

## Data structures

```rust
// resources.rs, PhysicsConfig (+1 field)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SleepSkip { Off, Replay, #[default] Sets }        // every mode yields Off's bits

// SolverScratch (+1)
pub(crate) bodies_prev: ScratchColumn<BodyState>,           // previous gather's buffer (post-solve), swapped at gather

// ContactPairs (+2)
pub(crate) pairs_prev: ScratchColumn<(BodyIndex, BodyIndex)>,  // P(t-1), swapped at bp start
pub(crate) src: ScratchColumn<u32>,   // per pair of P(t): u32::MAX = compute; bit31=0 → index into P(t-1);
                                      // bit31=1 → index into SleepSets.kept (C3 restore)
// Manifolds (+7, all parallel columns, swapped at np start)
pub(crate) manifolds_prev, sensor_prev: ScratchColumn<Manifold>,     // 152 B each
pub(crate) pair_out, pair_out_prev: ScratchColumn<u32>,  // per pair: u32::MAX none; idx; 0x8000_0000|idx = sensor
pub(crate) tag, tag_prev: ScratchColumn<u8>,             // per body-body manifold / sensor overlap:
                                                         // bits0..3 SAT axis (0xF = not box-box), bit7 SETTLED
pub(crate) stream_cursor: RemapCursor,                   // stamped by np when it wrote pair_out/tag

// IslandSleep: C0 Vec → ScratchColumn 1:1 (asleep u8, below_count u16, frozen_islands u8, energy f32)
//              C2 + mask_cursor: RemapCursor (stamped in begin_step after awake_rows is rebuilt)

// sleep_sets.rs (new resource, colored path only)
#[derive(Resource)] pub struct SleepSets {
  // C2 — hot, per step
  resting: TouchedMask,               // 1 bit/row (155 B at 1240 rows, L1-resident)
  non_resting: ScratchColumn<u32>,    // ascending rows N (fresh loop's outer set)
  inv_row: ScratchColumn<u32>,        // prev→cur, filled on Rows steps only (cold)
  rr: ScratchColumn<(u32, u32, u32)>, // resting-resting pairs + src, merge input
  stats: SleepSkipStats,
  // C3 — hot part (per step): 
  clean_of_row: ScratchColumn<u32>,   // cur row → store island, NONE; 4 B/row
  // C3 — cold part (touched on transitions / Rows steps only)
  islands: ScratchColumn<SleepIsland>,
  members: ScratchColumn<u32>, kept: ScratchColumn<Manifold>, kept_tag: ScratchColumn<u8>,
  nm_pairs: ScratchColumn<(u32, u32)>, warm: ScratchColumn<WarmEntry>,
  restore_warm: ScratchColumn<WarmEntry>, // this step's restored entries, prev-row keys, for the solve
  move_in: ScratchColumn<u32>,          // islands entering the store this step (solve fills warm)
  clean_pairs_total: u32,               // Σ kept + nm over live islands (axis begin_frame count)
  dead: u32, cursor: RemapCursor, epoch: SleepEpoch, sets_allowed: bool,
}
#[repr(C)] struct SleepIsland { members: R32, kept: R32, nm: R32, warm: R32, flags: u32 } // R32 = (start,len) u32×2; 36 B
#[repr(C)] struct SleepEpoch { threshold_bits: u32, frames: u16, warm_start: u8, mode: u8 }  // 8 B
#[repr(C)] #[derive(Clone, Copy, Debug, Default)]
pub struct SleepSkipStats { fresh_pairs: u32, rr_pairs: u32, replayed: u32, computed: u32,
  clean_islands: u32, clean_rows: u32, clean_manifolds: u32, restored: u32, moved_in: u32 } // 36 B
```

- **Hot/cold split.**
  - Per step: `resting`, `non_resting`, `clean_of_row` and the one-step stream buffers.
  - Transitions and remaps only: the store.
- **Memory at J scale.** About 0.27 MB of pair columns and 1.4 MB of manifold double buffers.
- **Threading.** No field is shared across threads, so there is no false sharing (§Multithreading).
- **Drop.** No `Drop` logic.
- **Unsafe.** None new. If R3 is vectorized with intrinsics, it gets a `// SAFETY:`.

## Public API

```rust
pub struct PhysicsConfig { /* … */ pub sleeping: bool /* default true */, pub sleep_skip: SleepSkip }
impl SleepSets { pub fn stats(&self) -> SleepSkipStats; pub fn is_row_clean(&self, row: usize) -> bool; }
```

- Everything else is `pub(crate)`.
- `solve_colored_sleeping` keeps its signature, so the direct-drive `colored_tests.rs` sites are untouched. `physics_solve_colored` calls a new `pub(crate) solve_colored_sets(…, &mut SleepSets)`.

## Algorithms for critical paths

**A1: Classification** (start of colored bp; O(rows); sequential on Identity, one gather through `prev_row` on Rows)
1. `mask = sleep.mask_cursor.peek(rows)` and `stream = stream_cursor.peek(rows)`. `peek` is a new `RemapCursor` method that classifies without counting.
2. For each row r:
   - Resting = R1 ∧ R2 ∧ R3. R3 is a field-wise bit compare of `BodyState` with exhaustive destructuring, so a new field fails to compile. `f32`s compare `to_bits`, and `shape` compares by variant plus payload bits.
   - Push non-resting rows to N.
   - A degenerate box (any half-extent == 0) is never resting. For it the hint can decide whether a contact exists (`axis_cache.rs:69-70`).
3. C3 only: if `clean_of_row[r] != NONE && !resting[r]`, mark that island dirty (D1-rule below).

**A2: Broadphase** (contract for any bp: *emit exactly the overlapping pairs with ≥1 non-resting endpoint, sorted*)
- **AllPairs at `e2bcbcb5`:**
  - Loop `for i in 0..n`:
    - if i ∈ N: today's inner loop over `j>i`;
    - else: only `j ∈ N ∩ (i,n)`, via a monotone cursor in N.
  - The output comes out sorted with no sort step. The expression is today's `bound = r(i)+r(j); (pos[j]-pos[i]).len²`. Cost is O(|N|·n).
  - N = all rows (and every `Off`/sleeping-off step) runs today's loop verbatim.
- **RR set.** Walk `pairs_prev` (valid iff `stream` is not Reset). Map both rows forward:
  - Identity: unchanged.
  - Rows: through `inv_row`.

  Keep a pair iff both endpoints are resting; set `src = k`. A pair whose mapped order reversed keeps `src = MAX` (compute). On Rows steps, sort RR (cold).
- **Merge** fresh and RR into `pairs`/`src` (two sorted, disjoint runs).
- **Grid** keeps the full CSR build (so `SoftRigid` coupling still reads a complete grid), then drops resting-resting candidates and merges RR. The saving there is np and graph only.
- **C3 additions:**
  - D2 marking inside the fresh loop, for non-sensor fresh pairs only.
  - RR pairs owned by a clean island are excluded. Owner = the island of the min row that is a clean member. Sensor pairs are never owned.
  - Restore and move-in (A4).
- **Complexity.** All asleep: O(rows + |P_flow|). All awake: today's cost plus a mask test per row.

**A3: Narrowphase** (contract: *per-pair kernel ∈ {replay, compute}, emitted in pair order*; this maps onto L5's count/prefix/emit)
- Swap the stream buffers. Then call `begin_frame_synced(pairs, extra = clean_pairs_total, …)`, which now returns `(prefetched, cleared)`.
- Per pair k with `src ≠ MAX`, read `rec = pair_out_prev[src]` and `t = tag_prev[..]`:
  - **no manifold:** emit nothing.
  - **non-box manifold:** copy with `body_a/body_b ← (a',b')`.
  - **box-box:**
    - Replay iff `t.SETTLED ∧ ¬(cleared ∧ ¬prefetched)`.
    - If `prefetched` (Rows step), mirror `axis_cache.set(a',b',t.axis)`, which is today's insert under the new key.
    - Otherwise compute.
- Compute is today's code. It also writes `SETTLED = (hint == Some(reference_axis))` and the axis.
- Sensor pairs go through the same path into `sensor_overlaps`.
- Cache behaviour:
  - Replay reads `manifolds_prev` sequentially (src is monotone on Identity) and writes `manifolds` sequentially.
  - 0.69 MB at J scale.
  - No non-temporal stores: graph and solve read the output immediately.

**A4: C3 transitions** (bp, serial)
- **Dirty rules for a stored island:**
  - D1: a member is not resting or was removed.
  - D2: a fresh non-sensor overlap.
  - D3: `would_clear(total) ∧ Identity`, and the island holds ≥1 box-box kept manifold. `would_clear` is a pure function asserted equal to `begin_frame`'s decision.
  - D4: on a Rows step, a kept manifold pair's endpoint vanished or its order reversed.
  - D5: `SleepEpoch` changed (threshold, frames, `warm_start_enabled`, mode).
  - D6: `wake_all` is pending.
  - D7: Reset.
- **Restore (a dirty island):**
  - Its kept and no-manifold pairs, renamed to current rows, enter the merge with `src = 0x8000_0000|idx`.
  - Pairs with a non-resting endpoint are dropped; the fresh loop covers them.
  - Reversed manifold pairs get `src = MAX`.
  - Warm entries are copied to `restore_warm` in the store's pre-rename (t−1) keys.
  - The island is tombstoned. Compaction runs when `dead ≥ live`.
- **Move-in (an island frozen at t−1, still in the stream).** It moves in iff:
  - every member is resting;
  - D9: every member's latch is `asleep` after `end_step(t−1)`;
  - D10: every one of its box manifold pairs is SETTLED;
  - none of D2–D7 applies.

  Then its manifolds (via `graph(t−1).island(i)` into `manifolds`, before np swaps them), its tags, its no-manifold pairs (owner rule) and its members are copied into the store, and the island is queued in `move_in`.
- **Rename** the surviving store to the current rows (Rows steps), then stamp `cursor`.

**A5: Graph.** The predicate becomes `is_dynamic_row ∧ !clean(row)`. Debug assert: no manifold in the stream names a clean row.

**A6: Solve** (`solve_colored_inner`, colored.rs)
- Before `build_columns`: insert `restore_warm` into `warm_read` (the new `WarmStartTable::reserve(extra)` is a cold rehash; lookups do not change).
- `begin_step` and `end_step` skip clean rows. Keys, latches and `below_count` stay untouched, and `awake_rows[clean] = false`.
- Before `store_and_swap`: for each move-in island, look its points up in `warm_read` through `remap.manifold_pair` exactly as `carry_frozen` does, and push the hits with current-row keys into `store.warm`.
- **C3a, all modes:**
  - **No-awake fast path.** If no dynamic row is awake, skip gravity, warm apply, colors, integrate, restitution, capture and restore. `BodyEffective` is rebuilt every step and frozen rows would be restored anyway, so this is exact.
  - **Need-sized `rebuild`.** Zero and mask only `next_pow2(2·contacts)` slots; the reservation stays and nothing reallocates.
- `WarmSeedStats.carry_points/carry_hits` and the `PHYS_NP_*` counters report stream values. New counters `PHYS_BP_FRESH`, `PHYS_NP_REPLAYED` and `PHYS_SLEEP_CLEAN_{ISLANDS,MANIFOLDS,PAIRS}` report the store, so logical total = stream + store.

## Exactness (determinism) argument
- **E1: Pair set.** `fresh ∪ RR` equals today's `P_t`, and the two are disjoint.
  - The bounding test is a pure function of (pose, shape).
  - A resting-resting pair has unchanged inputs (R2+R3), so it overlapped at t−1 and is in `P_{t−1}`.
  - The test is symmetric under a row swap: `(−x)²=x²`, and f32 `+` is commutative.
- **E2: Replay equals recompute.** Take a pair `(a',b')` whose source is `(a,b)`:
  - Both BodyStates are bit-equal to what t−1's narrowphase read (R2, R3).
  - The roles are kept (reversed pairs are computed).
  - The hint is the same:
    - Identity with no clear: only pair `(a,b)` writes key `(a,b)`, and a settled replay would write the same value.
    - Rows: the prefetch reads the old key before any write or clear.
    - Identity with a clear: excluded (computed).
  - SETTLED means the recorded compute returned the same axis from the same inputs, and `box_box_contact` is pure (`03-TREE-REPORT.md` §4).
  - No-manifold pairs are pose-only (A7b) for non-degenerate boxes, and degenerate boxes never rest.
  - **Therefore** the manifold and sensor streams are byte-equal. The axis table is byte-equal too: Identity steps skip same-value overwrites, and Rows steps mirror the inserts in pair order.
- **E3: Downstream stages.** Graph, solve and store read only the stream, `bodies` and the warm tables, so they are identical. The whole claim follows by induction over steps.
- **E4: A clean island is a fixed point of today's per-step map.**
  - Its members are unchanged (D1, R3).
  - No new edges or contacts reach it (D2).
  - Its manifolds replay-equal (E2, D3, D4, D10).
  - Its key is unchanged, so A4 does not fire.
  - Its latches are asleep (D9) and its energy is unchanged (R3 includes velocity), so `end_step` is idempotent.
  - Config is unchanged (D5).
  - **Therefore** skipping the map on it is exact.
- **E5: What the reduced problem shares with clean islands, and why each is safe.**
  - Axis table: the same key set and values, so the same lookups, `occupied` counts and clear decisions. Only the layout differs.
  - Warm table: keys are disjoint by row, and restored entries are present before the first read. A lookup depends only on the key set.
  - Colors: occupancy bits are per dynamic body, and islands are body-disjoint, so awake colors and in-color order are unchanged. Only trailing colors that held nothing but frozen manifolds are dropped.
  - Island ids: never persisted.
  - Counters: logical totals.
- **E6: Worker-count invariance.** No L10 code reads W or runs in parallel. The colored solve's `{1,N}` identity carries over.

## Multithreading model
- All L10 code runs inside serial stages: gather, bp, np, graph, and the solve's serial prologue and epilogue.
- No new shared mutable state, atomics or synchronization.
- `SleepSets` has the same `Send`/`Sync` as the other resources. Build views are `!Send`.
- Data-race freedom follows from the executor's resource access sets. The colored bp system gains `Res<IslandSleep>`, `Res<ConstraintGraph>`, `Res<Manifolds>` and `ResMut<SleepSets>`; np gains `ResMut<SleepSets>`; graph gains `Res<SleepSets>`. Physics stages are already chained.
- Side effect to note: user systems that read `Manifolds` or `ConstraintGraph` now conflict with bp as well.

## Expected gain, derived from P0 (`ANALYSIS.md` §6, §9)

**Remainder.** T − (bp + np + graph + solve) is gather + apply + executor gap:
- J-Son: 5.334 − 5.27 = 0.064 ms.
- R: 6.049 − 5.98 = 0.069 ms.

**C2 adds** (bandwidth bounds at ≥ 20 GB/s, not measured):
- R3: ≤ 1240 × 2 × ~160 B, so ≤ 0.02 ms.
- RR walk: ≤ 0.01 ms.
- Replay copy: 4,524 × 152 B = 0.69 MB, ≤ 0.05 ms. R: 6,662 manifolds = 1.01 MB, ≤ 0.07 ms.

**C2 totals:**
- J-Son: T ≈ 0.064 + 0.03 + 0.05 + graph 0.12 + solve 0.30 = 0.56. Gain 4.77 ms (−89 %).
- R: T ≈ 0.069 + 0.035 + 0.07 + 0.18 + 0.45 = 0.80. Gain 5.25 ms (−87 %).

**C3a** takes out the substep loop, capture/restore and the 1.5 MB zeroing of a grown warm table: 0.07–0.15 ms.

**C3b** takes out the replay copy, the graph build (down to an O(rows) reset) and build/carry. Floor ≈ remainder + classify + O(rows) sleep bookkeeping ≈ 0.11–0.13 ms. That is ≈50× Jolt's 2.17 µs, down from 2,450×.

**Mixed worlds.** Replay leaves ≈ (0.12 + 0.30 + 0.05)/4,524 ≈ 0.10 µs per sleeping manifold per step. Sets leaves O(members) bit work.

**Realized-gain gate:** ≥ 0.6 × predicted.

## Integration: file:line per commit

| commit | change | sites (`D:/wt/joltab`) |
|---|---|---|
| C0 | `IslandSleep` Vec → columns | `resources.rs:3108-3200` (fields, ctor), `:3250-3263`, `:3288-3292`, `:3346-3382`, `:3480-3568`, `:3615-3670`; `scratch_ids.rs` (4 ids) |
| C1a | new gates and runner flags | new `tests/sdf_sleep_settles_and_wakes.rs`, `tests/default_world_sleep_worker_invariance.rs`; sleeping-on arm in `alloc_frame_census.rs` (next to `:1769`); `benches/jolt_parity_pyramid.rs:132-134` (doc), `:405`, `:465-506`, `:565-578`, `:735-743`, `:1118`, `:1156-1178`: `--sleeping on\|off`, `--sleep-skip off\|replay\|sets` |
| C1b | the flip | `resources.rs:502-504` → `true`; doc `:328-330`, `:3463-3475`; `plugin.rs:347-349`, `:405-407`, `:541-545`; explicit `sleeping: false` in reds (rule under "What moves"); `docs/SYSTEMS.md:2065`, `docs/MEASUREMENT-QUEUE.md:520` (R row annotated: now sleeping on, R-off added); `book/src/simulation/physics.md:354` goes to **doc-writer** |
| C2a | plumbing, no behaviour change (`sleep_skip` default Off) | `resources.rs:3949-3976` (+`bodies_prev`), `:562-600` (ContactPairs), `:2269-2300` (Manifolds); `systems.rs:250-252` (swap before `clear`); `row_identity.rs:222-245` (`peek`, inverse fill); `IslandSleep` + `mask_cursor` (stamp at end of `begin_step`, `:3560-3567`); new `src/sleep_sets.rs`; `plugin.rs:546-549` (insert `SleepSets`), `:629-633` (colored bp/np variants); `profiling.rs` (+counters); runner anti-vacuity expectations |
| C2b | Replay (default Replay) | `systems.rs:297-349` (A1–A2 in the colored variant), `:375-478` (A3: `:382-398` swap and begin, `:437-446` replay/tag/settled, `:450-466` pair_out, `:472-477` counters); `axis_cache.rs:261-281` (`begin_frame` → `cleared`), `:367-388` |
| C3a | solver floor (all modes) | `warm_start.rs:274-286` (need-sized), new `reserve`; `colored.rs:3487-3503`, `:3547-3623`, `:3639-3652` (no-awake fast path) |
| C3b | Sets (default Sets; Replay if SDF wired) | `sleep_sets.rs` (A4); `systems.rs:297-349`, `:375-478`, `:1052-1056` (predicate), `:1097-1119`; `axis_cache.rs` `would_clear`, `begin_frame_synced(extra)`; `colored.rs:3451-3454` (+restore), `:3627-3630` (+move-in warm); `resources.rs:3524-3566`, `:3649-3669` (skip clean rows); `plugin.rs:551` (`sets_allowed = !with_sdf`) |

## Commit sequence (each commit green; C1b is the only one that moves values)
C0 → C1a → C1b → C2a → C2b → C3a → C3b → receipts (MEASUREMENT-QUEUE result blocks, by the tester/analyst).

## Gates
Rules:
- Every mutation below is recorded red before its commit lands.
- If a named mutation turns out to be green, the scene is adjusted until it goes red, or the mutation is replaced and the reason recorded. A gate that cannot fail is not kept.

**Pre-step (before C0):** on the base, reproduce P0's pose hashes for R-S and J-Son (`p0b/raw/window/runs.jsonl`). If `dbd85977..e2bcbcb5` moved values, record the base's hashes as the fixtures instead.

**C0**
- The sleep suites stay green with identical numbers: freeze steps and drifts.
- Mutation M0 (energy max → sum) turns `sleeping_pipeline_o8` red.
- New census arm: the default world with sleeping on makes 0 heap allocations per frame after warm-up. Mutation: a `Vec::with_capacity(1)` in `begin_step` turns it red.

**C1a**
- `sdf_sleep_settles_and_wakes`, three checks:
  - the pile freezes by a step K that is printed;
  - the rest pose matches the sleeping-off twin within the box-pile ε;
  - editing the field away wakes the pile on the next step.

  Mutation: SDF-sentinel manifolds filed under no island makes the wake check red.
- `default_world_sleep_worker_invariance` compares 400 steps of the rest pile at W ∈ {1,2,4,8,16}, per-step pose bytes. Its red-first check: a run with `sleep_frames = u16::MAX` must fail the "a frozen step was observed" assertion.

**C1b**
- Full `cargo test --workspace --all-targets --no-fail-fast`. The red set is the list of moves.
- R with the new default reproduces P0's R-S pose hash at W=1 and W=8. J's hash is unchanged.
- T(W): in-binary R-off vs R, W ∈ {1,8}, K=6. This is a consistency check against P0's −57.5 %/−38.4 %, not a new claim. J parent vs commit, K=12, no regression.

**C2a**
- Every runner row's pose hash equals C1b's at W ∈ {1,8}.
- J and J-S0 parent vs commit, K=12: not claimed slower.

**C2b: `tests/sleep_skip_bit_identity.rs` (new), Off vs Replay, compared per step**
- **What is compared:**
  - every BodyState's bits;
  - latch, `below_count` and key per row;
  - `contact_wakes`;
  - manifold and sensor stream bytes;
  - axis table and warm table bytes;
  - `WarmSeedStats`.
- **Worker counts:** W ∈ {1,8}; also {2,4,16} in release.
- **Scenes:**
  - S1: the rest pile, 400 steps.
  - S2: J, 1000 steps (300 in debug).
  - S3: adversarial. It contains:
    - spawns and despawns, including order-reversing moves and component insert/remove;
    - an awake projectile merging into a pile;
    - a member teleport and a member velocity write;
    - a static floor teleport and a kinematic platform under a pile;
    - a sensor volume;
    - `wake_all` and a mid-run `sleep_threshold` change;
    - a forced Identity-step axis clear through pair churn;
    - a degenerate box;
    - a parked member.
- **Anti-vacuity:** `computed == 0` on steps where N is empty; `replayed > 0` on ≥ 90 % of S1's frozen steps.
- **Named mutations (each must be red):**
  - M1: R3 off.
  - M2: no mirror `set` on Rows steps.
  - M3: no clear invalidation.
  - M4: reversed pairs replayed.
  - M5: SETTLED ignored.
  - M6: RR pairs with an unmapped endpoint kept.
- **Also under both modes, with identical outputs:** `support_loss_wakes_sleepers`, `sleep_settles_box_piles` (freeze step), `frozen_island_warm_start` (the bit-exact wake at `:738`), `sleeping_pipeline_o8`, `row_identity_remap`, `row_keyed_state_defect_a`, the W-invariance gate and the census arm.
- **Runner:** `--expect-pose` from Off at W=1 for R, R-S and J-Son, at every (W, mode). Exit 4 is red.
- **T(W), P0 protocol (K processes, median of window means, SE rule, receipts, canary seen, interleaved, H6 receipts):**
  - in-binary Off vs Replay: R at W ∈ {1,2,4,8,16}, K=6; J-Son tail at W ∈ {1,8}, K=6;
  - armed spans (bp, np, graph, solve) for R and J-Son at W ∈ {1,8}, with I(W) reported;
  - J-S0 in-binary and J parent vs commit, W ∈ {1,8}, K=12: not claimed slower.

**C3a**
- Pose bytes and `WarmSeedStats` equal parent vs commit, for every row and W. Table bytes are excluded (the layout changes).
- Proptest: `reserve` and the need-sized `rebuild` preserve every lookup, and load stays ≤ 0.5.
- Fast-path counter > 0 on all-asleep steps.
- Mutation: taking the fast path while a parked member is awake turns S3 red.
- T(W): parent vs commit on R and J-Son.

**C3b**
- `sleep_skip_bit_identity`, Off vs Sets:
  - Compared: BodyState bits, latch, key, `contact_wakes`, logical `WarmSeedStats`, the seed triple of each solved point in canonical order, and sensor bytes.
  - Not compared: table bytes.
- **Mutations (each must be red):**
  - M7: D2 off.
  - M8: D9 off.
  - M9: D10 off.
  - M10: no mirror sets for clean islands (S3 must force a clear whose timing depends on them).
  - M11: no warm restore (`frozen_island_warm_start` must show 0 seeded).
  - M12: `begin_step` writes `NO_ISLAND_KEY` for clean rows (spurious A4).
- **Structural anti-vacuity on S1 after the freeze:** `clean_rows` == dynamic rows; the stream is empty; the graph holds 0 manifolds; np processed 0 pairs; logical totals equal Off's.
- `would_clear` equals `begin_frame`'s decision (debug assert and proptest).
- An SDF-wired default world reports effective Replay.
- T(W): in-binary Replay vs Sets and Off vs Sets on R (W ∈ {1,2,4,8,16}) and J-Son (W ∈ {1,8}), K=6. J-S0 and J at K=12: no regression.

**debug_assert!s:**
- The merged `P_t` is strictly increasing.
- Every RR pair has both endpoints resting.
- A replayed manifold's rows equal its pair.
- `|P_t| + clean_pairs_total` equals the brute-force count. This is O(n²), so it runs only in the test-only `sleep_skip_audit` feature.
- No stream manifold names a clean row.
- Store ranges stay in bounds, and `Σ` of the ranges equals `clean_pairs_total`.
- Clean rows are not awake.

## What moves, and how each is re-measured
- **C1b is the only value mover, and it re-blesses no golden.**
  - Rule: a red test whose subject is not sleeping gets an explicit `sleeping: false`. Its pin then stays byte-identical, and the step at which its first island froze is recorded as the reason.
  - Sleeping-on default behaviour is covered by the new gates (the SDF gate, sleep W-invariance, and R's hash equal to P0's R-S hash).
- **Expected:**
  - Cannot move, because the horizon is shorter than 61 steps: `default_world_colored_simd` (10), `default_world_worker_invariance` (60), `scene_sync_s5` (5–16).
  - Moves only if an island freezes within its horizon: `bodytype_determinism_golden` (90), `default_world_pyramid_determinism` (120; its doc already says "sleeping off"), `colored_acceptance_o5`/`_simd_o7`, `body_set_selection`, `sdf_collision`, `soft_body_sp2`.
  - Unaffected, because they already pin the flag: `apply_row_alignment:344`, `alloc_frame_census:1769`, `alloc_frame_attribution:1936`, A7-R1 `sleep_settles_box_piles:1702/:1896`, and every sleeping-on suite.
- **Runner.** The R row changes meaning: it now runs with sleeping on, equal to R-S. R-off keeps the old series. MEASUREMENT-QUEUE §10 is annotated.
- **Benches** `sleeping.rs:183/218`, `sleeping_pipeline.rs:124` and `row_identity_churn.rs:241` are re-recorded after C2b and C3b. They are not gates.
- **C2 and C3 move no values.** Only diagnostic counters gain logical and stream variants.

## What it cannot claim
- Any change to the J parity headline: it is exactly 0, because sleeping is off on both sides.
- A mixed awake/asleep number. No P0 row measures one; the per-sleeping-manifold residual above is derived.
- A floor near Jolt's. Gather, apply and executor dispatch over all rows (~0.06–0.09 ms) remain. Removing them needs archetype-level sleep (U6 AWAKE lane gate, or X-5), which is refactor-last.
- Speed on SDF worlds beyond Replay. The SDF stage is not skipped.
- A broadphase saving on Grid (np and graph only).
- A smooth step on an Identity step with an axis-cache clear. Every box replay is invalidated for that step, which costs about today's price, plus one settle step. Clear frequency in churning worlds is unmeasured.
- A saving for an awake body whose bounding sphere overlaps a sleeping island: that island pays the Replay cost (D6).
- Any fix to today's recorded wake gaps. They persist bit for bit:
  - a write that changes no contact count;
  - a moving support that keeps the count;
  - soft reactions below the threshold.
- Equal table bytes under Sets. Only lookups, seeds and poses are claimed.
- A skip for degenerate (zero-extent) boxes. They never rest.

## Open questions
1. **Owner, values.** Should a user write to a frozen body always wake it (Rapier's rule), closing the "no count change" gap? This is not in L10, because L10 is exact.
2. **Owner, values.** The soft→rigid gap is now reachable on the default path. Take up a red-first test and a wake on reactions as a separate lever?
3. **Axis-cache eviction.** Compacting live keys instead of the wholesale clear would remove D3's spike and keep live hints, which is a value change and would be a separate lever. First measure clear frequency on a churn scene.
4. **Sequencing.** If L5 or the broadphase redesign lands first, the replay becomes L5's second per-pair kernel. The mirror sets join L5's serial axis commit. The redesign then implements the A2 contract: query from the non-resting rows.
5. **Unification plan.** C0 pre-empts U6's `IslandSleep` Vec removal (census 34 → 30). The unification design must be told. U7's `PairCache` subsumes C2's stream carry and C3's kept manifolds; U6 subsumes capture/restore.

Key files:
- D:/wt/joltab/crates/boyko_physics/src/systems.rs
- D:/wt/joltab/crates/boyko_physics/src/resources.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/warm_start.rs
- D:/wt/joltab/crates/boyko_physics/src/narrowphase/axis_cache.rs
- D:/wt/joltab/crates/boyko_physics/src/row_identity.rs
- D:/wt/joltab/crates/boyko_physics/src/plugin.rs
- D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs
- D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md