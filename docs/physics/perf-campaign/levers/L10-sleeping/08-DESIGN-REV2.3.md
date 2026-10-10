# L10 rev 2.3: delta to rev 2.2

This revision is a delta to `06-DESIGN-REV2.2.md`. It answers `07-REVIEW-OF-REV2.2.md` under the orchestrator's rulings. Anything not named here stands.
- It was read against `D:/wt/joltab`. I could not confirm the tree is at 47c5dabd (no shell) and could not run graphify.
- Lane sites are from the L5 lane working copy `D:/wt/l5np`. L9 and L11 are cited by design section.
- "arith." means computed, not measured.

## 0. Remark → answer

| remark | answer | section |
|---|---|---|
| C1 | The fill covers every row a computed pair reads: `fill(r) ⇔ ¬HELD ∨ SENSOR_NBR`. "All rows" was priced and rejected: +12 µs per step is 5–11 % of Sets(1). E2′ restated; S3 arm 7; N19 | D-F, §3 |
| W1 | Census arm S8b: W=4, parallel np on, {Tree, Grid-parallel}. Pinned per step in both states. N20 sits in the Sets chunk path | §7 |
| W2 | One predicate, `sensor_pair`, gates the skip, B2's kept unit and anchors, D1's RESTORED routing, the D2 scan and D-F. S3 arm 8; N21, N22 | D-G |
| W3 | The epoch gains `dt_bits`. Every config input is enumerated. S3 arm 4b; N23 | D-E′ |
| W4 | REC is admitted iff `tag & (HIT\|SETTLED) ≠ 0` at t−1. B3 and E4′ no longer claim "exactly 0" | B3′, B5′, §3 |
| W5 | The np arm is Sets iff `step_mode == Sets` or the restore set is non-empty. P-a takes `restore_rec` by stamp on both solve arms. N30 | D-H |
| O1 | Contract rule 5 (`is_held_skip` tested first); N26 | B1′, §10 |
| O2 | The refutation (Off′(1) < 0.584 ms) moves to pre-C0 | §6 |
| O3 | l5np `axis_cache.rs:98-103` Lemma 2 is amended | §5 |
| O4 | N27 plus S3 arm 10 | §7 |
| O5 | N3 is reworded as a lookup-side mutation | §7 |
| O6 | The per-record hit count is `Σ kept_rec[run].count`, which already lives in `SleepSets`. `HeldIsland.warm_hits` is deleted. The solve keeps `Res<Manifolds>` | A1′ |
| O7 | `SRC_RESTORE` becomes a `Plan.flags` bit, and `src` keeps `NONE`. N28 | A3′ |
| O8 | The HIT and SEPHIT paths define bits 6–7 and carry the axis nibble. The held-slot compare masks only HIT\|SEPHIT. `steady()` is deleted | B1′ |
| O9 | The serial path keeps its inline shape and shares `np_route` and `collide_box_pair`. The probe is L5's to remove, and it is already gone | D1′ |
| O10 | The fast path runs the A1 capture. N29 | A1′ |
| O11 | `PHYS_NP_REPLAYED` and `SleepSkipStats::replayed` are removed | §2 |
| OQ1 | An entry with a vanished endpoint is skipped. A Reset flush writes both sources at length 0 | A3′ |
| OQ2 | A `(key, slot)` scratch, `restore_sort`, is sorted with `sort_unstable` (keys are unique), then gathered. +1 id | A3′ |
| OQ3 | In debug builds A2.4 writes `(slot, j)` into `SleepSets::scratch` after the merge | A1′ |

## 1. Decisions (changed)

### A1′ (replaces A1's "Store" and "Index space" sources; adds the fast path)
- **Hit count (O6).**
  - A1 sets `kept_rec[s] = rec` and `held_warm += rec.count`.
  - L11 D3 compacts hits, so `rec.count == hits`. This is `debug_assert!`ed.
  - A record's total is `Σ kept_rec[run].count`. A2.5 reads it at restore and subtracts it from `held_warm`.
  - `HeldIsland.warm_hits` goes back to `_p` (still 48 B).
  - The solve writes only `SleepSets`, which it already holds as `ResMut`. `Res<Manifolds>` stays, so user readers of `Manifolds` are not serialised against the solve.
- **`j` in debug builds (OQ3).**
  - A2.4 reads each kept manifold at index `j` of `graph(t−1).island(a)` (the graph is still t−1's at the broadphase epilogue).
  - After the `order` merge, which is the last reader of `SleepSets::scratch` this step, debug builds write `(kept_slot << 32) | j` per moved-in manifold into it and stamp it with the gather seq.
  - The list is in capture order, so A1 zips it and asserts `run ∋ j`.
  - Release builds neither write nor read it. The column exists in both profiles, so the id layout does not depend on the profile.
- **Fast path (O10).** A lone pile's move-in step has no awake row, so it takes C3a's no-awake fast path. That path runs P-a, the A1 capture, D3, the swap and the stamp, in that order.

### A3′ (replaces A3's key and sort text; O7)
- **`SRC_RESTORE`.**
  - It is `Plan.flags` bit 2; FROZEN and LOOKED_UP hold bits 0–1.
  - `src` stays an index or `NONE`, and `NONE` is tested first.
  - P-c's seed and D3's carry read `restore_rec.recs[src]` iff the flag is set.
- **Keys (OQ1).**
  - Endpoint rows come from the record's row table after A1.2's translation.
  - If either row is `NONE` (a vanished member or anchor), the entry is skipped. No current pair or manifold can name a vanished row, so neither mode could hit it.
  - Otherwise the entry uses `p = prev_row[r]`, which is defined for every translated row.
  - On a D7 flush (a Reset cursor or a stale baseline) there is no `prev_row`. Both sources are written at length 0 and stamped, and no `prev_row` is read. Off also misses every L9 join (L9 D9) and every `manifold_pair` on Reset, so this is exact.
- **Sort (OQ2).**
  - Fill `restore_sort: ScratchColumn<RestoreSort { key: u64, slot: u32, _p: u32 }>` (16 B) from the restored records' kept manifolds.
  - `sort_unstable_by_key`, then gather `key` and `kept_rec[slot]` into `restore_rec` in sorted order. This costs O(r log r) + r × 64 B, on restore steps only.
  - Keys are unique. A manifold unions its dynamic endpoints, so it belongs to one island. Invariant K's duplicates exist only for pairs with no manifold, and those live in `restore_idx`. So the sort needs no stability.
  - **Rejected:** a k-way merge. It needs a K-entry heap column, and K is every record on a flush, for the same O(r log ·).
  - `restore_idx` is sorted in place: its 16 B entry is self-contained, and its equal keys are Invariant K duplicates, which are equal by K.

### B1′ (O8, O1; adds to B1)
- **L10 owns tag bits 6–7 on every path:**

| path | tag written |
|---|---|
| full compute | SET = contact; SETTLED = `hint == Some(chosen)` |
| HIT | `REC\|HIT\|SET\|SETTLED`; axis nibble and BOX copied from `tag_prev` (a hit reads no hint) |
| SEPHIT | `SEP\|SEPHIT`; sep_axis, axis nibble and BOX copied from `tag_prev`; SET = SETTLED = 0 |
| Separated / NoContact | SET = SETTLED = 0 |

- **`PairTag::HIT_INVARIANT = !(HIT|SEPHIT) = 0xF3FF`.** No reader of a kept tag consumes HIT or SEPHIT:
  - L9's join reads REC, SEP and sep_axis;
  - the mirror reads `rekeys()` and the axis nibble;
  - M3 on a cross pair's kept tag decides by SETTLED, and HIT implies SETTLED by the table above.
- **Rule 5.** `is_held_skip()` (`tag == HELD_SKIP`) is tested before any single-bit test in every classifier: counters, the closure, M3.

### B3′ (W4; replaces B3 "Steady state")
- `Quat::mul` (`quat.rs:171-173`) leaves `s ≈ 1e-8` for `q*⊗q`, not 0.
- What does hold: at bit-equal poses the criterion is a pure function of (R, P). So from the step after admission, Off either:
  - hits every step, with output `refresh(R, P)` and R unchanged; or
  - misses every step. The full compute then reads the hint it committed. M3′ makes that hint the axis chosen from the same inputs, so R is rebuilt bit for bit.
- An SEP axis stays negative.
- So the HIT-invariant tag, the record and the output are constant from move-in, and a copy taken at any held step equals Off's state.

### B5′ (replaces "REC: always settled")
- **REC is admitted iff `tag & (HIT|SETTLED) ≠ 0` at t−1** (one AND).
- For a cross pair, the tag tested is Y's kept tag.
- This restores rev 2's guard for τ_eff ≲ 3·s·r_S, τ = 0 included, where every step misses.

### D1′ (O9; replaces D1's placement text and pseudo-code)
```text
#[inline] fn np_route<const SETS: bool>(cls, a, b) -> Route    // one predicate, both paths
  if !SETS                                    → Stream
  s = sensor_pair(cls[a].flags, cls[b].flags)
  if !s && (cls[a].flags|cls[b].flags) & HELD      → Skip      // tag = HELD_SKIP; no stage write
  if !s && (cls[a].flags|cls[b].flags) & RESTORED  → Restore   // restore_idx cursor
  else                                        → Stream         // L9 pairs_prev join
```
- **On Skip:**
  - `np_chunk` writes `commit[k] = AXIS_NONE`;
  - `narrowphase_serial` (l5np `systems.rs:438-492`) skips `set`, which is the serial form of the same no-op, and keeps its inline `set`/`push`.
  - Both paths call the one `collide_box_pair`.
- **Rejected:** a context plus a commit array on the serial path. It adds 1 B per pair and a serial commit pass on W=1, which is the J-Son W=1 path, for no exactness gain. L5's theorem needs only the shared predicate and collide function.
- **The L5 debug probe is the L5 lane's to remove.** The lane working copy is already clean: `PROBE_MASK|eprintln!` over `l5np/crates/boyko_physics/src` hits only `solver/colored_tests.rs` (`:1992`, `:2067`, `:2171`, `:3091`). The pre-C0 base check re-runs this grep and escalates any hit to L5.

### D-E′ (W3; replaces D-E)
`SleepEpoch` gains `reuse: u8` (in the former `_p` byte), `tau_bits: u32` and `dt_bits: u32`. Any change flushes all.

| config input | read by | in epoch |
|---|---|---|
| `contact_reuse` | `collide_box_pair`, the slow gate | yes |
| `contact_reuse_distance` | τ_eff in the criterion | yes |
| `dt` | `is_fast` (L9 D8) | **yes (new)** |
| `sdf_narrowphase`, SDF edit list and kernel | SDF stage (`systems.rs:600`) | yes (rev 2 D5/D10) |
| `parallel_narrowphase` | dispatch only | no: L5's theorem |
| `broadphase`, `parallel_broadphase` | pair set | no: the set is exact; T6 |
| gravity, substeps, solver constants | integrate and solve | no: a frozen island is not solved, and its integrate is restored (rev 2 E4) |

- Body-side inputs (pose, velocity, shape, `is_sensor`) are `BodyState` fields, so R3 covers them.
- `dt` is stamped by the gather (`systems.rs:219`) and is constant under a fixed rate. A rate change flushes once.
- The cost is one u32 compare per step.

### D-F (C1; replaces Δ9's fill clause and §5's fill row)
- **The flag.** `RowCls` gains `SENSOR_NBR`.
  - A2.2's D2 scan sets it on every PRE_HELD or CAND endpoint of a stream pair with `sensor_pair`.
  - The scan runs whenever any row is PRE_HELD or CAND, and HELD ⊆ PRE_HELD ∪ CAND.
  - `release` (A2.3) adds only withheld pairs, which are non-sensor by Invariant V. So no sensor pair enters the stream after A2.2.
- **The fill rule.** L9 D2's fill, under the Sets arm, is `fill(r) ⇔ ¬HELD(r) ∨ SENSOR_NBR(r)`. Under the Off arm it fills all rows.
- **Assert.** A `debug_assert!` in `collide_box_pair` checks that each endpoint it reads satisfies `¬HELD ∨ SENSOR_NBR`. It reads the same flags as the fill.
- **Cost (arith.).**
  - All rows costs 1,241 × ~10 ns ≈ 12 µs per step at J (L9 D2).
  - That is 5–11 % of Sets(1) (0.11–0.23 ms) and 7–15 % of Sets(8) (0.08–0.18 ms). Both are above the ≈2 % SE bar and the campaign's 5 % build-if.
  - The flag costs one OR per sensor pair, in a scan that already runs, plus one predictable test per row.
  - So the numbers override the preference for all rows.

### D-G (W2)
- **One predicate.** `#[inline] fn sensor_pair(fa: u32, fb: u32) -> bool { (fa | fb) & SENSOR != 0 }` in `sleep_sets.rs` is the only predicate for:
  - `np_route` (skip and routing);
  - B2: a sensor pair is never kept, and its partner never joins the anchors;
  - the D2 scan;
  - D-F.
- `np_route` also runs `debug_assert_eq!(SENSOR, ba.is_sensor)` on computed pairs, which ties the flag to S5's overlap predicate (l5np `dispatch.rs:253`, `systems.rs:464`).
- **Why this is exact.** A sensor pair with a HELD endpoint is computed every step. Its t−1 state is therefore in `pairs_prev`, and at a restore it is read from there, which is Off's source.
- **No churn.** A moving trigger is no longer an anchor, so rev 2's no-churn behaviour returns.

### D-H (W5; replaces rev 2 D9's rows "sleeping on→off" and "Sets→Off")
- **The arm.** `SleepSets::np_sets: bool = step_mode == Sets ∨ restore set ≠ ∅`. The broadphase writes it, and it selects the narrowphase's const arm.
- **A1.0 with `m ≠ Sets` and a non-empty store:**
  1. A1.1 (cursors) and A1.2 (translation).
  2. Every record is restored through A2.5, with sources built as in A3′.
  3. `RowCls` is set to `RESTORED` on the flushed members and to `SENSOR` from `is_sensor` on every row (O(N), this step only). All other flags are 0.
  4. Stamp and return.
- **Effect.** No row is HELD, so the Sets arm only routes RESTORED pairs to `restore_idx`, and it fills every row.
- **The solve.** P-a takes `restore_rec` iff its stamp equals this gather, on both the sleeping-on and sleeping-off arms of `systems.rs:1097-1119` (`:1105`). The mode is never consulted.
- **Later Off steps.** `np_sets = false`, and `RowCls` is not read.

## 2. Data structures (delta)
```rust
// RowCls.flags: + SENSOR_NBR.  HeldIsland: warm_hits → _p (48 B unchanged)
// SleepSets:
restore_sort: ScratchColumn<RestoreSort>, // #[repr(C)] { key: u64, slot: u32, _p: u32 } = 16 B; restore steps only
np_sets: bool,
// SleepEpoch: + dt_bits: u32
// SleepSkipStats: replayed → restored_pairs (pairs computed from restore_idx); 48 B unchanged
// L11 Plan.flags: + SRC_RESTORE (bit 2).  L9 PairTag: + HIT_INVARIANT = 0xF3FF, is_held_skip()
```
- **Removed (O11).** `PHYS_NP_REPLAYED` is gone, so C2a adds only `PHYS_SLEEP_HELD`. `SleepSkipStats::replayed` is gone.
- **Scratch ids.** 20 + `restore_sort` = **21**.

## 3. Determinism (replaces E2′ and E4′; adds E7′)

**E2′.** Every non-skipped pair is computed from a source that holds Off's t−1 state.
- **`Stream`** (no RESTORED endpoint, or a sensor pair) reads L9's join. By induction its t−1 slot was computed:
  - a sensor pair is never skipped;
  - a non-sensor pair with a HELD endpoint at t−1 now has a HELD or a RESTORED endpoint, or it is gone.
- **`Restore`** reads `restore_idx`, which holds Off's steady state by K and E4′. A miss is stateless in both modes.
- **Shared by both routes:**
  - the key is L9's `key_prev`;
  - every row read is filled (D-F: a computed pair with a HELD endpoint is a sensor pair, and that endpoint carries `SENSOR_NBR`);
  - hints equal Off's (D-C);
  - config inputs equal Off's (D-E′).
- So the outputs equal Off's.

**E4′.** A held island is a fixed point of Off's map. This is rev 2's E4 plus:
- B3′: the HIT-invariant tag, the record and the output are constant from move-in, with no exact-zero premise;
- the D3 carry is idempotent;
- the epoch covers τ, `contact_reuse` and `dt`.

**E7′.** A step with a non-empty restore set computes restored pairs from `restore_idx` and seeds them from `restore_rec`, whatever the mode (D-H). So every flush lands in Off's state.

## 4. Expected gain
Unchanged.
- D-F keeps the Sets floor: J has no sensors, so no row gets `SENSOR_NBR`.
- `dt_bits` adds one compare per step.
- `restore_sort` runs on restore steps only.

## 5. Changed file:line list (added to rev 2.2 §5)

| site | change |
|---|---|
| `systems.rs:297-349` | the A1.0 flush path (D-H); A2.2 sets `SENSOR_NBR`; A2.5 skips `NONE` rows and writes empty sources on Reset; `restore_sort`; `np_sets`; the debug `j` list; `dt_bits` |
| `systems.rs:1097-1119` | P-a's second source by stamp on both arms; the fast path runs A1; access set unchanged |
| `resources.rs:130-134` | doc: a `dt` change flushes held islands |
| `resources.rs:2269-2327` | `HeldIsland._p` |
| l5np `narrowphase/dispatch.rs:150-163`, `:170-190`, `:210-276` | `NpChunkCtx` gains `cls`, `restore_idx`, `kept_reuse` and skip meta; `np_route` in `np_chunk`; `AXIS_NONE` on Skip |
| l5np `systems.rs:393-429`, `:438-492` | arm chosen by `np_sets`; the serial loop calls `np_route` and skips `set` on Skip |
| l5np `narrowphase/axis_cache.rs:98-103` | Lemma 2 also skips a held pair: the mirror keyed it serially before the scope with the axis Off's steady output sets, so the serial `set` is an identity (O3) |
| L9 `carry.rs` | B1′ bit rules, `HIT_INVARIANT`, `is_held_skip` |
| L9 `row_frames` fill | `¬HELD ∨ SENSOR_NBR` under the Sets arm |
| L11 `warm_records.rs` `Plan` | `SRC_RESTORE` moves to `flags` |
| `profiling.rs:77-131` | no `PHYS_NP_REPLAYED` |
| `scratch_ids.rs:889`, `:937-939` | L10 cohort 21 |

## 6. Commits (delta)
- **Pre-C0 (O2).**
  - Take the armed Off′ J-Son span at W=1, K=6 on the lane base. If Off′(1) < 0.584 ms (= 0.6 × 0.59 + 0.23), stop and escalate before C0.
  - The C3b rehearsal remains as the pre-claim check that Δ ≥ 0.35 ms.
  - The L5 probe grep (D1′) runs here.
- **C2a.** Adds `np_route<false>` and one counter.
- **C3b.** Adds D-F, D-G, D-H, A3′, B1′ and S8b.

## 7. Gates (delta)

**Comparator.**
- Held slots: `tag == HELD_SKIP`, and `kept & HIT_INVARIANT == off & HIT_INVARIANT`. This replaces `steady()`.
- Computed slots: still byte-equal.

**S3 arms added:**
- **4b.** `sleep_threshold` is raised so a box freezes while sliding on the floor at 0.012 m/s, which lies between the fast bounds (0.0087 m/s at 30 Hz, 0.017 m/s at 60 Hz, τ_eff = 1 mm). The rate then changes from 60 to 30 Hz while held. Anti-vacuity: that pair's Off class flips from slow to fast; a void is red.
- **7.** A box sensor over a held pile whose members alternate 0° and 45° yaw.
  - A Rows step moves a member into a row whose last fill holds a frame of the other yaw.
  - The sensor is placed so that a 0°-versus-45° frame flips its SAT verdict.
  - Compared: `sensor_overlaps()`, the sensor tags and the axis commits.
  - Anti-vacuity: the stale slot ≠ the member's frame.
- **8.** A static and a kinematic box trigger over a held pile, with a cached SEP pair, then a projectile restore.
  - Tags are compared at the restore step.
  - The moving trigger must show `moved_in ≤ 1` over 120 steps.
- **9.** τ = 0: pile held, restore, wake. This is the witness for M3′.
- **10.** Grid: a despawn shifts the stream so a held pair lands on a slot whose previous occupant committed a real axis.
- **11.** A held member is despawned.

**S8b census (W1).** W=4, `parallel_narrowphase` on, sleeping on, Sets, × {Tree, Grid-parallel}. After warm-up:
- **Phase A (dispatching).** A stirred awake pile with ≥ 256 stream pairs beside a held pile. A projectile restores the held pile every 5th step.
- **Phase B (inline).** The stirred pile is despawned, leaving < 256 stream pairs.
- **Pins per step:**
  - 0 heap allocations beyond the ruled scope allocation (L5 D8 ruling);
  - scopes = S8's count + `[chunk_count(stream, 4) ≥ 2]`.
- **Anti-vacuity:**
  - Phase A: runs of `np_chunk::<true>` > 0, restore lookups inside chunks > 0, and (Grid) held skips inside chunks > 0.
  - Phase B: chunks = 0.

**Mutations (new or reworded):**

| id | mutation | red via |
|---|---|---|
| N3′ | `restore_rec` searched with `key_prev` instead of `manifold_pair` | S3 arm 1, with the flip at the restore step (Off misses, the mutant hits) |
| N19 | the fill ignores `SENSOR_NBR` (it skips HELD rows that have a sensor partner) | S3 arm 7 |
| N20 | `Vec::with_capacity(1)` in `np_chunk`'s Sets prologue (the restore cursor start) | S8b phase A |
| N21 | B2 keeps sensor pairs | S3 arm 8 (tag, churn count) |
| N22 | RESTORED routing ignores `sensor_pair` | S3 arm 8 (tag) |
| N23 | the epoch ignores `dt` | S3 arm 4b |
| N24 | M3 admits REC without HIT or SETTLED | an exhaustive `admit()` test over u16. A scene red would need the hint to change an output beyond the chosen axis; arm 9 is only a witness |
| N25 | the hit path omits SET/SETTLED | exhaustive: `hit_tag(t) & HIT_INVARIANT == t & HIT_INVARIANT` for every settled REC miss tag; the same for SEPHIT over Separated tags |
| N26 | a counter tests HIT before `is_held_skip` | the runner closure voids: `reused` and `sep_hits` each overcount by `held_skipped` |
| N27 | a held skip leaves `commit[k]` unwritten | S3 arm 10, axis lookups |
| N28 | the D3 carry or the P-c seed ignores `SRC_RESTORE` | `frozen_island_warm_start` exact wake under Sets; S3 arm 3 |
| N29 | the fast path skips the A1 capture | S1 single-pile move-in: warm lookups, `carry_hits` |
| N30 | the np arm follows `step_mode` on a flush step, or P-a gates on the mode | S3 `sleeping` on→off and Sets→Off with a held REC pair: tag, seed and pose |
| N31 | restore keys built for `NONE` endpoints | S3 arm 11: bounds panic |
| N32 | restore sources not emptied on a Reset flush | S3 missed-gather arm |

**Unit tests.**
- Deleted: `steady()` exhaustive.
- Added:
  - the `restore_sort` build against a `Vec` sort oracle;
  - a grep gate: `& SENSOR` appears only inside `sensor_pair`.

## 10. Open questions (delta)
- The Q2 cross-lane contract gains:
  - B1′'s bit rules and the axis nibble copied on HIT and SEPHIT;
  - `HIT_INVARIANT`;
  - rule 5 (`is_held_skip` first).
- Q4 stands.
- There are no new questions.

## Files
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/06-DESIGN-REV2.2.md`, `07-REVIEW-OF-REV2.2.md`, `04-DESIGN-REV2.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/00-RULINGS.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`, `L11-solve-setup/02-DESIGN-REV1.md`
- `D:/wt/joltab/crates/boyko_physics/src/systems.rs`, `D:/wt/joltab/crates/boyko_physics/src/resources.rs`, `D:/wt/joltab/crates/boyko_math/src/quat.rs`
- `D:/wt/l5np/crates/boyko_physics/src/narrowphase/dispatch.rs`, `D:/wt/l5np/crates/boyko_physics/src/narrowphase/axis_cache.rs`, `D:/wt/l5np/crates/boyko_physics/src/systems.rs`
