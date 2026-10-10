VERDICT: CHANGES REQUESTED; BLOCKING=0; IMPORTANT=5

# Architecture review: Tree broadphase (`BroadphaseKind::Tree`)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED. The exactness and determinism core holds. The defects are in the C5 hint, the rebuild triggers, and gates that cannot fail.

## Remarks

### 🔴 Critical
None. The determinism argument checks out against the tree:
- The pair count sizes the axis cache (`D:/wt/joltab/crates/boyko_physics/src/narrowphase/axis_cache.rs:261-281`).
- `Vec3::dot` is scalar and left to right (`D:/wt/joltab/crates/boyko_math/src/vec.rs:177-178`).
- `fl(a−b) = −fl(b−a)` exactly, and f32 addition is commutative.
- The cull slack of 16u against the ~4u needed holds. I re-derived it.
- The output is fixed by the pair set alone: counts, a prefix over rows, and per-row sorted lists.

### 🟡 Important

#### W1. On any colored world with sleeping off, C5's hint marks every row as frozen
**Where**: D3, C5, algorithm step 1 (the hint is `is_row_awake`).
**Evidence**:
- `IslandSleep` is inserted on every colored world, whatever `cfg.sleeping` is (`plugin.rs:535-549`).
- The solve touches it only `if cfg.sleeping` (`systems.rs:1105`).
- `awake_rows` is written only by `begin_step` (`resources.rs:3558-3567`).
- A row past the live range reads NOT-awake (`resources.rs:3234-3244`).
- So with sleeping off the mask stays empty, and every dynamic row reads "frozen". With sleeping toggled off (`row_identity_remap.rs`, `row_keyed_state_defect_a.rs`), the mask is left over from the last sleeping-on step.

**Consequence**:
- On the J cfg-A headline (sleeping off) and today's default, all 1,240 moving boxes become pending sleepers. Pending ≥ 64 triggers the cold serial rebuild. Next step the verify fails on every one of them, because they moved. This churns every step.
- Cost: build plus 1,240 serial self-queries at 100–150 ns plus a sort of about 9.5k SL pairs, roughly 0.2–0.3 ms serial at every W. It replaces the 0.07–0.13 ms parallel path. That is +1–2 % of T(8), above J's SE bar of 0.75 %.
- The output stays exact, so G1–G3 stay green. C5's G5 runs only the sleeping-on rows (J-Son, R-S), so no gate sees it.

**Confidence**: CONFIRMED.
**Needed**:
- Gate the hint on `cfg.sleeping`, and on the mask actually covering the current N.
- Add a structural check: `sleeper_rebuilds == 0` on a sleeping-off J run, with the mutation "hint not gated" red.
- Add a sleeping-off J row to C5's G5.

#### W2. Rebuild triggers for the persistent sets: incomplete, too broad, and never bounded from above

**(a) The cursor is never stamped.**
- `RemapCursor::remap` only classifies. `stamp` is mandatory (`row_identity.rs:208-239`). Without it, classify returns `Reset` on every gather from the third on (`:467-491`).
- Step 0 of the plan never stamps. Without a stamp, both sets are rebuilt every step and the output is still exact.
- G2 asserts only the lower bounds `static_rebuilds ≥ 1` and `sleeper_rebuilds ≥ 1`. No gate can see that persistence has failed.
- **Confidence**: PLAUSIBLE (the plan omits the stamp); the cursor mechanics are CONFIRMED.

**(b) Churn dissolves both sets.**
- Any spawn or despawn changes the row count, so `finish_gather` reports the rows as unstable (`row_identity.rs:440`), which yields `Rows`.
- D3 then rebuilds both sets. Step 0 disables the hint, so every verified sleeper is reclassified active. The sleeper set only refills after max(64, n_sl/8) pending rows or 8 steps.
- **Consequence**:
  - A world with one rigid spawn every ≤ 8 steps never keeps a sleeper set, so C5's floor gain there is zero.
  - Every spawn step rebuilds the whole static set. At the plan's own 10k costs (170–220 ns per row), that is about 1.6–2.2 ms plus the SS sort. This is at least the entire projected 10k broadphase (1.9–2.4 ms).
  - The premise "cold" is false for spawn-heavy games.
- `D:/wt/joltab/crates/boyko_physics/benches/row_identity_churn.rs` already has the arms for this (swap_churn, first_archetype_spawn, burst_*, each with sleeping off and on). The plan does not use it.
- The bit+class verify is already the correctness authority: a persistent pair set depends only on each row's bits and class. So a rebuild triggered by the remap buys no correctness where the records still match.
- **Confidence**: CONFIRMED.

**(c) A change in N under direct drive.**
- `classify` returns `Identity` for a `RowIdentity` that never gathered (`row_identity.rs:470-472`). That covers benches, hand-filled snapshots, and possibly G1.
- The verify walks only rows 0..N. D3 lists no trigger for "N changed", "a member row ≥ N", or "a new row with a stale `RowRec`".
- A despawned static in the tail row stays in SS, so a `BodyIndex` ≥ N reaches the narrowphase. In world drive the cursor catches it.
- **Confidence**: PLAUSIBLE.

**Needed**:
- A complete, explicit list of triggers.
- Upper-bound gates, for example `static_rebuilds == 1` over J's 600 steps and a bound on `sleeper_rebuilds` after R-S's freeze at step 248. The mutation "stamp removed" must go red.
- The rebuild cost at 1,240 / 10k / 100k bodies, and the churn arms added to G4/G5.
- A decision on whether the remap trigger is needed at all. If it is not, M5 is an equivalent mutation (it cannot change the output) and must be re-specified.

#### W3. G5 never measures C2's parallel wave, and C2 alone falls short of the build-if rule on J
**Evidence**:
- G5 runs on C3's binary with `--cfg default`.
- `configure` leaves `parallel_broadphase` untouched under Default (`benches/jolt_parity_pyramid.rs:737-744`).
- The default stays `false` until C4 (`resources.rs:493`).
- The new `--broadphase` flag sets only Manual and the kind.
- So every G5 row times the **serial** Tree. The check "realized Δbp(8) ≥ 0.6 × 1.82 = 1.09 ms" is passed by the serial tree (≈ 1.75 ms), so it cannot fail for a broken or slow wave.

**Consequence**:
- By the plan's own figures, C2 alone is worth about 0.08–0.09 ms at W=8 (serial 0.15–0.22 against parallel 0.07–0.13 ms).
- That is about 1 % of T(8) today, and about 2.3 % of the projected 3.6–3.9 ms. Both are below the rule "build only if the gain is ≥ 5 % of T(W)" (ANALYSIS §9), and near J's SE bar.
- For that, C2 brings unsafe pointer waves, one `Box<ScopeShared>` plus one chunk per step, census re-pins, and a default flip. The flip also moves coupling worlds with ≥ 4096 bodies onto Grid's `build_parallel`, whose O3 Gate 7 read 1.92× against a 2.8× gate, not ruled on.
- Also, the gain table's before-values are cfg-A rows, while G5 runs `--cfg default` rows. P0b measured neither T nor the SE bars for those rows (only R).

**Confidence**: CONFIRMED.
**Needed**: Either
- gate C2 on its own: serial tree against parallel tree, same binary, W=8, under SE, with a threshold that a serial tree fails; or
- defer C2 by the same logic as Open question 3.

#### W4. Mutation M2 cannot go red
**Evidence**:
- G1's generator uses sizes 1e−3..1e3, so no radius is ever negative.
- G0 does include negative radii, but it tests only the lane kernel, not the cull bounds.
- `ColliderShape::Sphere { radius }` has no sign validation anywhere in `src/`, so negative radii are reachable and D8's `|r|` is load-bearing.

**Consequence**: A regression to bounds built on `r` instead of `|r|` would ship green. A required-red mutation that cannot go red is a gate that cannot fail.
**Confidence**: CONFIRMED.
**Needed**: Negative radii in G1 (both radii negative, and mixed signs), and M2 shown red.

#### W5. The runner's per-step anti-vacuity check (ruling W4) does not cover the 7 new zones and counters
**Evidence**:
- `check_step` hard-codes 14 span expectations and 6 counter expectations (`benches/jolt_parity_pyramid.rs:954-984`).
- Growing `SPAN_ZONES` / `COUNTER_ZONES` (`profiling.rs:106-131`) compiles without touching it.
- The plan edits only `:723-749` and `:53-55` of the runner.

**Consequence**:
- An armed step where a `phys_bp_*` zone fires 0 times (brute path, fallback) is accepted.
- So is one where it fires W times, i.e. a zone placed inside a chunk task. Ruling O1 forbids that, and its lock-prefixed adds disturb the timed wave.
- The per-phase numbers the plan records under `docs/measurements/` would go unvalidated.
- The plan also never says what each zone records on the brute path, the fallback, or the AllPairs/Grid arms.

**Confidence**: CONFIRMED.
**Needed**: Per-path structural expectations for all 7, written into the plan and into `check_step`.

### 🟢 Optional

- **O1. Allocation wording.** The Goal says "allocates nothing on the hot path". But each query wave's `pool.scope` makes a 256 B `Box<ScopeShared>` plus a ≥ 4 KiB chunk per step (`tests/alloc_frame_census.rs:645-662`). The census section already accounts for this; the Goal should say the same.
- **O2. Fallback granularity.** A row with a ±inf position and a finite r can never satisfy the predicate (|d|² is inf or NaN). It is exactly as excludable as a NaN row. Only rows with an infinite or huge r need special handling. Today one runaway body forces the whole step into AllPairs: 2.1 ms at J, about 70 ms at 10k.
- **O3. Open question 1 settled from the code.**
  - `SCRATCH_ID_ROW_PREV` = `highest_id_clear_of(419, 511, 468)` = 403, so `ROW_IDENTITY_BOTTOM` = 400 (`scratch_ids.rs:596-609`).
  - That leaves 16 free ids (399..384).
  - The cohort would take 399..389, stagger slots 5..15. That is clear of `BODY_STATE` (slot 63) and `CONTACT_PAIRS` (id 446, slot 62).
  - The floor constant does not need to move; the floor assert moves to 389.

## Positive
- D1: exactness is required for the right reason (the axis cache is sized by P). The slack proof and the rejection of the AABB and layer predicates on value grounds are sound.
- The bit verify is the correctness authority and the latch is only a hint. A wrong hint costs a rebuild, never a pair.
- Row-owned 128 B blocks with a serial prefix give output that depends on the pair set alone: no global sort, no atomics, no false sharing. The pointer discipline matches the Grid's (`solve_base`, a view only after the join).
- The AllPairs arm stays verbatim as the oracle. M3 goes red at J step 0. The census is re-pinned with zero headroom, with a new structural formula and a mutation. The rebuild is kept `#[cold]` and out of line.
- The "what it cannot claim" list is honest.

## Open questions
1. How does the integration test `tests/broadphase_tree.rs` inject a random sleep hint for M6? `IslandSleep` has no pub setter for its mask, and `force_sleep_row` is `#[cfg(test)] pub(crate)` (`resources.rs:3249-3255`).
2. C4 turns `parallel_broadphase` on. Which counting-allocator tests run the full default pipeline at W ≥ 2 with at least PAR_QUERY_MIN rows, and pin a per-step count? Candidates: `constraint_graph_o4_world.rs`, `broadphase_grid.rs`, `soft_body_sp1.rs`, `soft_colored_sp4_alloc.rs`.
3. Once W2(c) is fixed, is the verify the sole correctness authority? If it is, M5 cannot change the output and has to be re-specified. If the cursor is still needed, name the case the verify misses.