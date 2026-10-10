VERDICT: CHANGES REQUESTED; BLOCKING=1; IMPORTANT=5

# Architecture review: L10, sleeping on by default and the frozen-pair skip

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED

I checked the plan against the tree at `D:/wt/joltab`. The exactness argument for Replay (D1–D3, E1–E4) holds everywhere I traced it. The problems are in four places: what Sets changes on the public surface, the load-bearing solver predicate, runtime transitions, and gaps in gate coverage.

## Remarks

### 🔴 Blocking

#### B1. Sets changes public observables that the required suites assert on, yet the plan says C3 moves no values and every commit is green
**Where:** D7/A5 (clean rows leave the graph); A2 (clean-owned pairs leave `ContactPairs`); A6 (`WarmSeedStats` carry fields "report stream values"); "What moves: C2 and C3 move no values"; C3b makes Sets the default.

**Problem:** Under Sets the clean islands disappear from five public views: `ContactPairs::pairs()`, `Manifolds::manifolds()`, `ConstraintGraph` (`n_islands`, `island_of`, colours), `IslandSleep::is_island_frozen` and `WarmSeedStats.carry_*`. The suites the brief says must stay unchanged read exactly these views:
- **`sleep_settles_box_piles.rs` G5/G6 (`shift_scene`).**
  - After the freeze and a 5-step hold, P1 asserts `n_islands()==1` and every pile row on island 0 (`:2354-2359`). Under D7 clean rows are not dynamic, so they have no island. Red.
  - P2 builds its set from `ContactPairs`/`Manifolds` (`:2273-2287`), gets an empty set, and panics "scene-fitness … escalate" (`:2366-2371`).
  - `islands_after == (2, 0)` also fails (`:2439-2443`).
- **`support_loss_wakes_sleepers.rs`.** `settle_until_latched` runs at least 120 steps with `SLEEP_FRAMES = 8` (`:84-86`). Then `contact_ids(UPPER)` reads `Manifolds::manifolds()` and must equal `[SUPPORT]` (`:489-494`). The island has long been clean, so the result is `[]`.
- **`frozen_island_warm_start.rs`.** It asserts `(points, carry_points) == (0, solved_before)` on every frozen step (`:696-713`, `:941-957`). The plan says the carry fields report stream values, which are 0 after move-in. The plan's own C3b gate compares "logical WarmSeedStats", which the public accessor would not return.
- **Receipts.** The runner's `step_shape` (`jolt_parity_pyramid.rs:869-876`) takes pair and manifold counts from the stream, so the per-manifold normalisations for sleeping-on R rows change meaning.

**Consequence:**
- C3b goes red on three named correctness suites the day it lands.
- On the new default path, any user system reading these public resources silently loses every sleeping contact and island.

**Confidence:** CONFIRMED. I traced the test code at the lines above and the plan text. The move-in happening inside G5's 5-step hold rests on D10 being met, which is near-certain for a pile at rest for 60 frames.

**What is needed:** a decision on what these public views mean when sleeping is on by default. Either:
- keep them logical (Box2D v3's `b2Body_GetContactData` and Rapier's contact graph still expose sleeping contacts), without paying the copy Sets exists to avoid; or
- make it an owner-visible API change: docs and book updated, and every affected suite re-expressed against logical accessors without loosening it.

Either way, list every site in "What moves", and make `WarmSeedStats` or the B1 assertions logical.

### 🟡 Important

#### W1. D7 breaks the load-bearing coloring/write-guard predicate pair
**Where:** D7/A5 change `systems.rs:1052-1056` to `is_dynamic_row ∧ !clean`. The solve's write guard stays `is_dynamic_row(eff.inv_mass)` (`colored.rs:2080-2081`, SIMD `:2422-2424`).

**Problem:** `contact.rs:20-40` states that both sites must be the identical predicate, "by construction". The plan backs the new split with nothing but a `debug_assert!` ("no stream manifold names a clean row"). If any manifold in the stream names a clean row, the coloring treats that row as ground, so two same-colour manifolds may share it, while the solve still writes it.

**Consequence:** at W≥2 this is concurrent writes to one BodyEffective row (the O11-SP4 race class, which the {1,N} gate cannot see). At W=1 the solve order differs from Off. W2 names a concrete path that triggers it.

**Confidence:** CONFIRMED that the pair drifts. PLAUSIBLE that the invariant breaks.

**What is needed:** make the two sites agree by construction, or enforce the invariant in release.

#### W2. Runtime transitions out of and into the skip modes are unspecified
**Problem:** `sleeping` is toggled at runtime in a shipped test (`profiling_zone_counts.rs:190-191`).
- A2 sends sleeping-off steps down "today's loop verbatim". Nothing says the Sets store is flushed, or `clean_of_row` cleared, on that step. `SleepEpoch` (D5) holds the threshold, frames, warm-start flag and mode, but not `sleeping`. Clean rows' manifolds then reach the stream while D7 still marks those rows non-dynamic, which is the W1 hazard.
- R4 checks only the mask cursor. A2 invalidates the RR set when the stream cursor is Reset. It does not put every row back into N when that happens.

**Consequence:** switching mode from Off to Replay/Sets, if Off does not stamp the stream, drops every resting–resting pair from P_t for one step. Frozen islands' counts change, A4 fires, and values diverge from Off.

**Confidence:** PLAUSIBLE; the plan does not say.

**What is needed:**
- define every transition: sleeping on/off and each mode change;
- make "resting" require the stream cursor as well;
- add runtime toggles to S3.

#### W3. Where the mirrored axis `set`s for clean islands run, relative to `begin_frame`'s clear, is unspecified
**Problem:**
- A4 renames the store in the broadphase. The clear happens later, in the narrowphase's `begin_frame_synced`: prefetch, then clear on grow or occupancy, then stamp (`axis_cache.rs:263-280`, `:367-388`).
- D3 marks an island dirty only on an Identity-step clear.
- A spawn that pushes the pair count past `next_pow2` grows and clears the table on a Rows step, which is the common case. Clean islands stay clean across it.
- Off re-inserts their keys after the clear (`systems.rs:444`). If Sets mirrors them in the broadphase, the clear erases them.

**Consequence:** `occupied` diverges between modes. Later load-based clears then fire on different Identity steps, and awake box pairs read a hint in one mode and None in the other. Different manifolds follow, then different poses. S3's forced clear is an Identity-step clear, so it cannot catch this.

**Confidence:** CONFIRMED gap; the divergence path is traced.

**What is needed:** place the mirrors after `begin_frame` in the narrowphase, and add a Rows-step grow-clear while an island is clean to S3.

#### W4. The bit-identity gates miss three code paths the plan claims are exact
**Problem:** S1–S3 run AllPairs only. Not covered:
- **Grid,** with its separate drop-RR-then-merge code. Coupling forces Grid (`plugin.rs:513-517`), Auto can select it after L2, and cfg-B uses it. Also `parallel_broadphase`.
- **SDF-wired worlds,** which become Replay by default under D10. These are the owner's main target and have no Off-vs-Replay byte gate. The C1a SDF test compares against a twin within ε, not byte-for-byte.
- **Soft coupling,** where the plan claims R3 catches the reactions but no scene tests it.

**Consequence:** a divergence in any of these ships on a default path with every listed gate green.

**Confidence:** CONFIRMED absence.

**What is needed:** run S1–S3 under both broadphase kinds with parallel emit, plus an SDF pile with a mid-run field edit and a coupled soft body.

#### W5. The zero-allocation census cannot fail on the transition paths
**Problem:** The census arm measures a steady frozen world. Its mutation sits in `begin_step`, which runs every step. Restore, `WarmStartTable::reserve` (a "cold rehash" whose scratch storage is unspecified), compaction and the Rows-step rename/sort are never exercised after warm-up.

**Consequence:** a heap allocation on every wake or restore step ships undetected, against principle 5.

**Confidence:** PLAUSIBLE.

**What is needed:** a census scene that wakes, re-freezes and churns rows after warm-up, plus a mutation placed in the restore path.

### 🟢 Optional

- **O1. Rows steps can be every frame.** Any spawn or despawn makes `stable=false` (`row_identity.rs:440`), so every consumer classifies as Rows. The "cold" RR sort (~9.5k pairs) and the O(store) rename would then run each frame in a live game with spawns, which is the workload sleeping serves. Skip the sort when the mapped order is already monotone, and state the floor for Rows steps. PLAUSIBLE.
- **O2. The absolute floors are likely understated.**
  - The C3 floor uses a remainder of 0.064 ms. That is a disarmed T minus armed spans rounded to two decimals.
  - P0's own J-A-a terms (ANALYSIS §2) give gather 0.027 + apply 0.018 + g 0.089 = 0.134 ms at W=1, which is already above the ≈0.11 claimed.
  - The Replay model counts memory bandwidth only; 9.5k pairs through a branchy per-pair loop likely puts C2 at ~0.6–0.7 ms.
  - Quote ranges, including "≈50× Jolt". The ≥0.6× gain gate still passes.
- **O3. C3a voids its own timing rows.** The no-awake fast path breaks the runner's W4 anti-vacuity: `check_step` expects gravity, integrate and freeze spans on every step (`jolt_parity_pyramid.rs:954-969`). Add the runner to C3a's integration sites. CONFIRMED.
- **O4. The unconditional `bodies` swap (D3) costs J something.** On J and other sleeping-off rows it buys nothing. It moves the gather's ~200 KB of writes into a buffer last touched two steps ago. Swap only when a skip mode is active. PLAUSIBLE, below the SE bar.
- **O5. Add two mutations that encode D2's own argument:**
  - R2 replaced by the latch (a row latched for the first time was still integrated at t−1);
  - R4 treating Reset as Identity.

## Positive
Preserve all of these:
- Replay-as-cache with recompute as the oracle, and the Off/Replay/Sets ladder with Off as the byte oracle.
- R3 as an exhaustive field-wise bit compare that fails to compile when BodyState gains a field. It closes user writes, raw writes, soft reactions and recycled ids by value.
- R2 decided from the frozen mask rather than the latch, which is correct and non-obvious.
- SETTLED, the prefetch/clear conditions and the degenerate-box exclusion, which together make E2 sound.
- The E5 colouring argument.
- D10 (no Sets with SDF), and D6's conservative dirty test.
- C1b as the single value mover, and the pre-step hash reproduction.
- The red-first W-invariance check (`sleep_frames = u16::MAX`).
- An honest "cannot claim" list.
- Everything new lives in `ScratchColumn`/`TouchedMask`, and C0 removes the `IslandSleep` Vecs. Principle 0 holds.

## Open questions
1. Does Off mode write `pair_out`/`tag` and stamp `stream_cursor`? This is what W2 turns on.
2. What statistical resolution does the P0 protocol give on steps of 0.1–0.8 ms? No P0 row measured one. A K=6 rehearsal of the J-Son tail at C2 would settle it.
3. After C3b, does `SleepSkip::Off` stay a public oracle setting for users and the bit-identity gates?

Key files: `D:/wt/joltab/crates/boyko_physics/tests/sleep_settles_box_piles.rs`, `D:/wt/joltab/crates/boyko_physics/tests/support_loss_wakes_sleepers.rs`, `D:/wt/joltab/crates/boyko_physics/tests/frozen_island_warm_start.rs`, `D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs`, `D:/wt/joltab/crates/boyko_physics/src/narrowphase/axis_cache.rs`, `D:/wt/joltab/crates/boyko_physics/src/plugin.rs`, `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs`