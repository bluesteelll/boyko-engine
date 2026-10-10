VERDICT: CHANGES REQUESTED; BLOCKING=0; IMPORTANT=3

# Architecture review: L10 rev 2 (sleeping on by default, plus the frozen-pair skip)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED

The plan is complete; it ends with its Sources list. I checked the logical-views mechanism against every assertion the three B1 suites make, and it holds. All three Important remarks are about steps where a held island is restored. None of them needs a redesign.

## Status of the rev-1 remarks

| rev 1 | status | evidence |
|---|---|---|
| B1 | ✅ | See the verification section below. There is no per-step copy: `order` changes only on move-in and restore steps, and on Rows steps it only has its rows translated. |
| W1 | ✅ | `effective_inv_mass` is one function over one flag column, feeding both the colouring (`systems.rs:1052-1056`) and the write guard. Held rows also drop out of gravity, integrate and refresh (`simd.rs:153,199,247`) and out of write-back (`colored.rs:3329,3354`). |
| W2 | ✅ | There is a transition table, and each stage reads `step_mode`. One gap remains for writes other than config (O6). |
| W3 | ✅ | Mirrors run after `begin_frame`. S3 has a Rows-step grow-clear. M10 is named. |
| W4 | ✅ | Tree, Grid with parallel emit, and AllPairs are all covered, plus S4 (SDF) and S5 (soft body). |
| W5 | ✅ | Census arm S8, with mutations in restore, move-in and the rehash. |
| O1–O5, OQ1–OQ3 | ✅ | |

## The B1 suites under the logical views (checked)

- **`sleep_settles_box_piles` G5/G6**
  - P1 (`:2346-2359`) and `islands_after == (2,0)` (`:2439-2443`) come out exact with pre-rooting:
    - union-by-size ties go to `ra` (`resources.rs:2720-2728`);
    - ids are assigned in ascending root order (`:2756-2781`);
    - the far sphere arrives as E1 (`NO_ROW`) and the pile rows are E3-aligned, so the pile stays held and its root is renamed monotonically.
  - P2 (`:2264-2301`) reads `pairs()` (stream ⊎ `withheld`) and `manifolds()` (stream ⊎ kept). It compiles against the view shapes the plan lists (`for &(a,b) in pairs`, `filter(|m| ..)`, `filter_map(|m| ..)` with `m` by value).
- **The view order key matches today's order.** The narrowphase keys every manifold `(min,max)` and pushes only `count > 0` (`systems.rs:400-466`). SDF manifolds are appended afterwards, one per row (`:606-634`). So `(a<<32)|b` followed by `(1<<63)|a` is exactly Off's order.
- **`support_loss_wakes_sleepers`.** `contact_ids` (`:356-384`) reconstructs the kept `(U,S)` manifold with global rows. Every change (teleport, `DisableSimulated`, delete, remove) fails R3 or removes the row, so D1 fires.
- **`frozen_island_warm_start`**
  - `carry_points` counts only when warm start is enabled and `count != 0` (`colored.rs:1651-1654`); `live_points` matches that.
  - Off's `carry_hits` stays constant after the first frozen step, because the carry copies values and a miss drops the entry permanently (`:3308-3316`). So `held_warm` is exact. Frozen step k=0 still goes down the stream path.
  - The crowd test's `widest_awake_color_slots` (`:1070-1098`) is unchanged after the C2a edit to `solver_manifolds()`. First-fit colour choice for awake manifolds does not depend on held manifolds (E5).

## Remarks

### 🔴 Critical
None.

### 🟡 Important

#### W1. Restores decided in the prologue stay in the Tree's `frozen` hint, and release covers only D2 and D3
**Where:** A1.3 (PRE_HELD taken from `of_row`), A1.4 (restore list), the kind row (`HeldHint { frozen: PRE_HELD, … }`), A2.3 ("Release … if D2/D3 hit PRE_HELD rows"), A2.5 ("HELD = PRE_HELD − restored + moved-in").

**Problem:**
- Nothing in the plan takes the A1.4 restore list out of PRE_HELD before the kind arm runs. This covers D1, D1a, D8, jumpers, D5/D6 flushes and D7.
- Read literally, the Tree therefore keeps a restored island's rows in Z and withholds its internal SL pairs. `release` is never called for them.
- The plan also contradicts itself:
  - T4 justifies `release` by "D2 and D3 are decided after the Tree has already withheld those pairs", which implies earlier restores are excluded from the hint;
  - Invariant V ("both endpoints resting") fails for a non-resting D1 member that is still in Z.

**Consequence:**
- At C3c, on any restore decided in the prologue, the restored island's member–member and member–static pairs are missing from the stream for that step. Its manifold count drops, so A4 wakes it with no contacts.
- On a `wake_all` or epoch flush, every held contact in the world disappears for one step. `frozen_island_warm_start`'s exact-wake test and S3's teleport arm under the Tree would go red.

**Confidence:** CONFIRMED (it is in the plan text).

**What is needed:**
- State that the hint is PRE_HELD minus the prologue restore list, or that `release` runs for every restore on a Tree step.
- Add a debug assertion of Invariant V over `withheld` after the kind arm.
- Name a mutation: "prologue-restored rows left in the hint".

#### W2. A restored run is looked up by a translated ordinal, but on jumper steps that translation is not monotone; `order` is also merged before it is filtered
**Where:**
- A1.2 translates every record's rows through `inv`.
- D12 restores exactly those records that touch a jumper.
- A3(ii) finds the source in "the kept run … binary search by ordinal".
- A2.4 merges new ordinals into `order` before A2.5 filters out the restored records.

**Problem:**
- Rows resolved by the aligned walk (E3) have strictly increasing previous rows (`row_identity.rs:574-581`). A stage-2 row does not (`:598-602`, `:627-641`).
- After A1.2, a jumper-touching record's kept run is no longer sorted by current ordinal, so binary search for its other members' pairs (both still resting) can miss.
- Kept runs hold no no-manifold pairs, so a miss is read as "none", and a manifold is silently dropped.
- The same unsorted entries are still in `order` when A2.4 merges into it. A two-pointer merge against a misplaced entry can leave `order` unsorted even after the filter. Example: `[10, 50*, 20, 30]` ⊕ `[25]` gives `[10, 25, 20, 30]` once `50*` is removed.

**Consequence:**
- This is the default despawn case: a swap-remove moves the archetype's tail body into the freed row, which is E6 and therefore stage 2.
- `sleep_settles_box_piles` G3 and G4 do exactly this: the top box goes from the last row into row 1 (`:2199-2258`). The pile is restored, one of its resting pairs misses the lookup, the count changes, and A4 wakes the pile. The test's "nothing may wake" fails at C3b.
- In release builds, the unsorted `order` makes `manifolds()` emit an order different from Off's.

**Confidence:** CONFIRMED (the plan text, plus the monotonicity limits in `row_identity.rs`).

**What is needed:**
- Look up restored runs by a key that translation does not change: the t−1 ordinal (through `prev_row`, which is valid for resting endpoints), or local-index pairs. Alternatively, compute every pair of a record restored because of a jumper.
- Run the A2.5 filter before the A2.4 merge.
- Name a mutation: "restored run searched by translated ordinal".

#### W3. Under Sets, R3 runs on every static on every step; T2 makes that load-bearing, and no gate can see the cost
**Where:** A1.3 ("R2 first … R3 only if R2 holds"; statics always pass R2 because `inv_mass == 0`); T2 (`anchor_ok = RESTING ∧ !sensor` decides S-fit); the J-S0 and J gates.

**Problem:**
- R3 reads two `BodyState`s of about 156–160 B each (`resources.rs:3681-3735`) for every static, every step. It does so even when nothing is asleep.
- Off does none of this work.
- T2 now ties the Tree's static set to RESTING, so the per-static compare cannot simply be dropped.
- The "not claimed slower" arms (J, J-S0) contain one static.

**Consequence:**
- A level with 10k static bodies reads about 3.2 MB more per step, roughly +0.16–0.32 ms. That is more than the whole claimed all-held floor (0.11–0.23 ms).
- At the 100k statics that broadphase rev 1 projects, it is about 32 MB per step, roughly +1.6–3.2 ms. This is a regression against Off on the new default path, and no gate would record it.

**Confidence:** CONFIRMED for the mechanism (plan text). The magnitude is arithmetic.

**What is needed:**
- Bound the classify cost. Directions: run R3 only for rows a record, a CAND island, the D2 scan or the SL list can reach; or skip the pass when nothing is held or CAND; or use a narrower compare for S rows.
- Add a static-heavy "not claimed slower" arm (J plus 10k statics).

### 🟢 Optional
- **O1.** The claim "everything else compiles unchanged" misses two sites:
  - `sdf_collision.rs:616` formats `{manifolds:?}`, so the view needs `Debug`;
  - `constraint_graph_o4.rs:242,480,497` call `island(..).is_empty()`, so `IslandManifolds` needs `is_empty`.
- **O2.** `ManifoldsView::get(pos: usize)` accepts a `color(c)` or `island(i)` value cast to `usize`. That silently reads the wrong manifold under Sets only. A handle newtype would make D6's "fails to compile" claim true.
- **O3.** The AVX2 R3 compare over "the f32 prefix" must be an integer compare. A `_ps` EQ compare treats ±0.0 as equal and NaN as unequal. Add a ±0.0 case and name the mutation.
- **O4.** Under T2, static sensors are never S-fit, so they are queried every step (n × 100–150 ns). In levels with many trigger volumes that costs about as much as the floor. Alternatives: a cached sensor–Z pair list, or a line in "cannot claim".
- **O5.** Every SDF edit flushes every held island in the world (D5). A field edited continuously makes Sets no better than Replay. Put this in "cannot claim".
- **O6.** D5 and D6 are checked only in the bp prologue. A user system explicitly ordered between the broadphase and the SDF stage or solve could call `wake_all()` or write `SdfField`; Off would see the write and Sets would not. Either state this as a contract or re-check at those stages.
- **O7.** Add a "concatenate instead of merge" mutation for `ManifoldsView`. The suites sort their results, so only the mixed held/stream scenes of the bit-identity suite can catch it.

## Positive (preserve)
- Logical views are built as stream ⊎ store, with an `order` index that only changes on transition steps. The ordinal key reproduces Off's order exactly, and the views cost nothing per step.
- Pre-rooting held islands in `reset_islands` gives exact island ids, `is_island_frozen` and A4 keys at the O(N) the build already pays.
- `effective_inv_mass`: one derived value feeds the colouring, the write guard and the integrate kernels, which retires W1 structurally.
- `restore_warm` is keyed in t−1 rows, the domain of `warm_read`. The move-in capture goes through the same `warm_remap` as `carry_frozen`.
- Jumpers are defined through `stage2_rows`, with a correct monotonicity argument for E3 rows.
- The SDF epoch compares edit bits rather than `gen` (`SdfField` is `Copy`). The physics code reads only `edits()`.
- The mirrors, the P_logical sizing and the per-path runner expectations.

## Open questions
1. `IslandInfo` has no `kept_start`, yet `island(i)` must produce `HELD_BASE|slot` handles without access to `Manifolds`. Is `kept_start` copied in at graph build?
2. M3's "SET ∧ ¬PUSHED" can only occur on box pairs with no manifold, and `graph(t−1).island(a)` does not enumerate those. Which pass reads their tags? If none does, exactness rests on INV-A7b after all.

Key files: `D:/wt/joltab/crates/boyko_physics/src/row_identity.rs`, `D:/wt/joltab/crates/boyko_physics/src/resources.rs`, `D:/wt/joltab/crates/boyko_physics/src/systems.rs`, `D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs`, `D:/wt/joltab/crates/boyko_physics/tests/sleep_settles_box_piles.rs`, `D:/wt/joltab/crates/boyko_physics/tests/frozen_island_warm_start.rs`, `D:/wt/joltab/crates/boyko_physics/tests/support_loss_wakes_sleepers.rs`