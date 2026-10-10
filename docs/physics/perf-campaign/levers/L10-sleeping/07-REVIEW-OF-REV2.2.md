# L10 rev 2.2 - architecture review (on the complete plan)

VERDICT: REVISE; BLOCKING=1; IMPORTANT=5

# Architecture review: L10 rev 2.2 (the complete text: 451 lines, §0 to Sources)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED

I read the whole plan this time, together with rev 2, its review, the rulings, the L11, L9 and research documents, the earlier rev 2.2 review (`07-REVIEW-OF-REV2.2.md`), the L5 lane code (`D:/wt/l5np/.../narrowphase/dispatch.rs`, `axis_cache.rs`, `systems.rs`) and the joltab tree.

The delta does answer (a), (b) and (c):
- A1 captures at the t−1 stream index through L11's D3.
- B2 to B4 make the kept unit a pair and compute restored pairs from the kept source.
- D1 to D4 put the skip in L5's per-pair body, and the mirror and axis commit stay serial.

One blocking hole remains: `row_frames` against sensor pairs. There are five Important remarks. Four of them are places where sensors, `dt`, small τ or a transition into Off break the exactness argument. The fifth carries over the census gap from review 07, narrowed.

## Status of the previous review's remarks (07)

| item | status | evidence |
|---|---|---|
| C1 (plan cut off) | resolved | The full file is on disk. |
| W1 (census cannot see the scope term) | **open, narrowed → W1** | N13 now names the dispatch-input mutation, red on "Tree all-held step: dispatch count must be 0". S8 is still rev 2's "W=1, parallel off" (`04-DESIGN-REV2.md:538`). §7 and §8 do not change it. |
| W2 (`HELD_SKIP` = HIT\|SEPHIT) | resolved in intent → O1 | B1 says "It is its own class for the counters". The closure gains `held_skipped` (§5 runner row, §8). A single-bit counter now voids the step (loud), not silent. |
| O1 (refutation too late) | still holds → O2 | |
| O2 (A2.2 gives the counts for free) | still PLAUSIBLE, not re-raised | A2.2 is unchanged. `held_skipped` would need a fix-up for this step's restores and move-ins. |
| O3 (L5 Lemma 2 doc) | still holds → O3 | |
| OQ1 (REC pair with no manifold) | resolved | Δ4, B2, N6, S3 arm 3. |
| OQ2 (tags of no-manifold pairs on restore) | resolved for non-sensor pairs; **open for sensor pairs → W2** | B2 keeps SEP, and B4 computes from the kept tag, so `sep_axis` is Off's. |
| OQ3 (skip writes commit and tag) | resolved; one mutation still missing → O4 | D1 pseudo-code, N11, N12. |
| OQ4 (restore on L11 records) | resolved | A1: key search plus the `debug_assert` on `j`. A3: second source with disjoint keys. E6′: hits. The per-source cursor only affects cost, since D2 is exact from any cursor. |
| OQ5 (parity waiver) | resolved | B4, §10 Q2. |
| OQ6 (what Δ1 retires) | resolved | Δ1, A4, §7 (M2–M6), B5 keeps SETTLED for non-REC SET pairs. |

## Remarks

### Critical

#### C1. The `row_frames` fill skips HELD rows, but sensor pairs with a HELD endpoint are still computed
**Where:** Δ9; D1's `held_skip` ("HELD endpoint ∧ !sensor pair"); E2′ ("`row_frames` of non-HELD rows are filled"); §5 row "L9 `reuse.rs` / the `row_frames` fill: skip rows with `RowCls::HELD`".

**Problem:**
- `held_skip` exempts sensor pairs (rev 2 A3(i)), so the pair (held member x, box sensor s) is collided every step.
- T2 keeps s out of the Tree's S-fit set (`anchor_ok = RESTING ∧ !SENSOR`), so this pair is in the stream under Tree as well as Grid.
- L9 applies L9a to sensor box pairs (`L9 02-DESIGN-REV1.md` D3) and builds both OBBs from `row_frames` (D2; the `collide_box_pair` pseudo-code).
- With the fill skipping row x, `row_frames[x]` keeps whatever the last fill wrote into that slot:
  - On Identity steps that happens to be x's own frame (R3).
  - On any Rows step that shifts x's row (every swap-remove despawn below the pile's rows), the slot holds the frame of the body that sat in that row before.

**Consequence:** Take a box trigger volume over a held box pile, plus any despawn that shifts the pile's rows. The sensor SAT runs with another body's orientation, so `sensor_overlaps()` diverges from Off. So do the pair's SEP tag and axis commit. `sensor_overlaps()` is a public view that rev 2 D6 promised is "unchanged".
- Rev 2's S3 has both ingredients, but not together.
- N17 mutates in the opposite direction.

**Confidence:** CONFIRMED (plan text plus L9 D2/D3).

**Why critical:** E2′ and the logical-view exactness claim are false on the default path.

**What is needed:**
- The fill must cover every row that a computed pair reads: HELD rows with a sensor partner, or simply all rows. L9 puts the whole fill at about 12 µs at J, which is all Δ9 buys.
- E2′ must be restated to match.
- Add an S3 arm ("box sensor over a held pile across a Rows step") and the mutation "fill skips HELD rows that have a sensor partner".

### Important

#### W1. No census arm runs `np_pair<true>` inside `np_chunk` (carried from 07, narrowed)
**Where:** §7 "the sleeping-on arms of the census (S8) pin their own scope count"; §8 "L5 census… measured at C3b".

**Problem:**
- `chunk_count` returns 0 below 2 lanes (`l5np dispatch.rs:115-124`). So S8 (W=1, parallel off, `04:538`) never enters `np_chunk`.
- S1c runs with sleeping off (`04:582`), so it only executes `np_pair<false>`.
- The Sets-only chunk code (the per-chunk `restore_idx` cursor `lower_bound`, the skip-stats array, the `RowCls`/`restore_idx`/`kept_reuse` views in `NpChunkCtx`) runs under no census arm.
- All three census mutations in §7 sit on serial broadphase paths.

**Consequence:** A heap allocation per chunk or per dispatch in the Sets chunk path would ship green. That path is the default world at W=8.

**Confidence:** CONFIRMED.

**What is needed:**
- A sleeping-on census arm at W ≥ 2 with `parallel_narrowphase` on, pinned per step in both states: dispatching, and below 256 stream pairs.
- A mutation inside the Sets chunk path.

#### W2. Sensor pairs are not excluded from the kept unit (B2) or from the RESTORED routing (D1)
**Where:**
- B2: "A pair with an endpoint in a moving-in island and tag ∈ REC ∨ SEP ∨ PUSHED(manifold) becomes a `KeptPair` … Its non-member endpoint joins the record's anchors, so D1a watches it."
- D1: `src = if SETS && (cls[a]|cls[b]) & RESTORED { restore_idx } else { pairs_prev }`.

**Problem:**
- Sensor pairs carry SEP and PUSHED tags: L9a covers sensors (L9 D3), and sensor overlaps are PUSHED.
- They are computed on every held step, so their real t−1 state is in `pairs_prev`.

**Consequence:**
1. **At any restore step, a false red.** The sensor pair is computed from its move-in-time `KeptPair`, or cold if it was not kept. Off computes it from its t−1 slot.
   - Off gives SEP|SEPHIT with its cached axis.
   - Sets gives SEP with the canonical first negative axis, or a stale axis.
   - "Tags of computed slots are byte-equal to Off's" goes red with no value defect: L9-L2 keeps the manifold exact, and sensors are never REC.
2. **If sensor pairs are kept, a trigger volume becomes an anchor.**
   - A moving kinematic trigger fails R3, so D1a restores the pile.
   - M2 only checks manifold partners, and D2 excludes sensor pairs. So the pile is re-admitted on the next CAND step: move-in and restore churn on alternate steps, each paying the O(P_prev) scan (≈20 µs at J) plus the copies.
   - Rev 2 kept such a pile held.

**Confidence:** CONFIRMED (plan text).

**What is needed:**
- Carve sensor pairs out of B2 and out of D1's routing with the same predicate `held_skip` uses.
- Add a restore arm under a sensor to the tag compare.

#### W3. The epoch (D-E) leaves out `dt`, which L9 made a narrowphase input
**Where:** D-E ("`reuse: u8` and `tau_bits: u32`").

**Problem:**
- L9 D8: a pair is fast iff `3·dt²·(…) > (τ_eff/2)²`. A fast pair emits the raw full compute; a slow one emits `refresh(build(full))` (L9 D7). The bits differ.
- `dt` is stamped every step from `FixedTime` (`resources.rs:130-134`). It is not in `BodyState`, so R3 cannot see a change.
- A held pair's at-rest relative velocity is constant (rev 2 D13). The fast threshold on |Δv| at τ_eff = 1 mm is ≈0.017 m/s at 1/60 s and ≈0.0087 m/s at 1/30 s.

**Consequence:** A runtime change of the fixed rate while islands are held makes Off reclassify held pairs between slow and fast. Their manifolds change bit for bit, while Sets keeps the old ones. `manifolds()` and the post-wake poses diverge from the oracle. This violates rev 2.1 W2's "every runtime transition is defined".

**Confidence:** CONFIRMED for the mechanism. Whether a given scene sits near the threshold is PLAUSIBLE.

**What is needed:**
- Add `dt` bits (and any other config `collide_box_pair` reads) to `SleepEpoch`.
- Add it to S3 arm 4 and name the mutation "epoch ignores dt".

#### W4. "`q*⊗q` has a vec part of exactly 0 in IEEE" is false for `boyko_math::Quat::mul`, and B5's "REC: always settled" rests on it at small τ
**Where:** B3 "Steady state"; E4′; B5 "REC: always settled. L9-L1 covers both a hit and a miss at t−1".

**Problem:** `quat.rs:171-173` evaluates left to right. With `self = conj(q)`:
- `x = ((w·x − x·w) − y·z) + z·y`, which is exactly 0;
- `y = ((w·y + x·z) − y·w) − z·x` and `z = ((w·z − x·y) + y·x) − z·w`, which each return the rounding error of the first sum.

So s is about 1e-8 for a generic unit quaternion, not 0.
- At the default τ = 1 mm the criterion still holds by orders of magnitude, so the default path is safe.
- For τ_eff ≲ 3·s·r_S, including τ = 0 (which L9 D11 allows), every REC step is a miss. That means a full SAT that reads the hint, and B5 then admits the pair without the SETTLED check that rev 2 required for exactly this case.

**Consequence:** At τ ≈ 0, a REC pair whose chosen axis flipped on its island's first frozen step can be admitted. Off's next output then comes from a different hint.

**Confidence:** CONFIRMED for the false claim (formula checked). PLAUSIBLE for the divergence (τ ≈ 0 configurations only).

**What is needed:**
- Admit REC iff HIT or SETTLED at t−1. Both bits are in the tag, so this costs nothing.
- Restate B3 and E4′ without "exactly 0".
- Or have L9 make `s` exactly zero on bitwise-equal inputs, which is L9's decision.

#### W5. Transitions into Off do not say whether they read `restore_idx` / `restore_rec`
**Where:** Rev 2 D9's rows "sleeping on→off: flush all; `restore_warm` still inserted; sleeping-off solve" and "Sets→Off: flush all". A4 deletes `restore_warm`. §1–§3 do not restate these rows. D1 does not say how `SETS` is chosen.

**Problem:** On these flush steps `step_mode` is Off. If `np_pair<SETS>` follows `step_mode`:
- restored pairs go through the `pairs_prev` join and find `HELD_SKIP` (Grid) or nothing (Tree, withheld), so they are computed cold;
- the sleeping-off solve's P-a never searches `restore_rec`.

**Consequence:** Toggling `sleep_skip = Off` or `sleeping = false` while a REC pair is held gives a cold miss where the Off-throughout run hits (L9 W2's count and pose shape), and warm seeds are lost. The trajectory diverges from the oracle from that step on. S3's toggle arms would show it, but the design has to decide it.

**Confidence:** CONFIRMED (the text is silent where rev 2 was explicit).

**What is needed:**
- State that any step with a non-empty restore set routes RESTORED pairs to `restore_idx` and searches `restore_rec`, whatever the new mode.
- Name the mutation.

### Optional
- **O1 (from 07 W2).** Make "`is_held_skip()` is tested before any single-bit test" a stated contract rule in §10 Q2, with the mutation "a counter tests HIT before HELD_SKIP".
- **O2 (from 07 O1).** §6 already records the armed Off′ J-Son spans before C0. Apply the refutation test there (Off′ < 0.584 ms), not after C3b.
- **O3 (from 07 O3).** L5's Lemma 2 (`l5np axis_cache.rs:98-103`) says a pair is skipped "only when there was no pre-read and its hint equals the axis". Under Sets, held-skips commit `AXIS_NONE` on pre-read frames too. Add `:75-103` to §8's doc list.
- **O4 (from 07 OQ3).** Name the mutation "held skip leaves `commit[k]` unwritten". Its red needs a slot whose previous byte was a real axis, for example a Grid step where the stream shifts under a held island. N11 only covers the tag.
- **O5.** N3 as worded ("`restore_rec` keyed by the flip-surviving key") cannot fail on the entry side:
  - A3 already keys by min/max.
  - While an island is held no flip is possible, because a flip needs a jumper, and a jumper restores the record.

  Reword it as a lookup-side mutation: search `restore_rec` with `key_prev` instead of `manifold_pair`.
- **O6.** A1 writes `HeldIsland.warm_hits`, which lives in `Manifolds::held`. The solve holds `Res<Manifolds>` (`systems.rs:1100`). Either move the per-record hit count into `SleepSets`, or state the change of the solve's access set to `ResMut<Manifolds>`. That change would also serialise every user system that reads `Manifolds` against the solve.
- **O7.** `Plan.src` bit 31 (`SRC_RESTORE`) collides with a `u32::MAX` NONE unless NONE is tested first. `Plan.flags` has 6 free bits. No mutation covers "the D3 carry or the P-c seed ignores `SRC_RESTORE`"; M11′ only covers "not searched".
- **O8.** `steady()` normalises only HIT and SEPHIT. B1 defines SET and SETTLED "in the compute path", but a hit runs no SAT and reads no hint. If a kept tag captured at a REC-miss step carries SET or SETTLED that Off's steady REC|HIT does not, §7's held-slot compare goes red on correct code. Define both bits on the hit path, or compare only the bits the join reads.
- **O9.** D1's "one `np_pair(ctx, k)` called by `np_chunk` and the inline loop" does not match the L5 lane:
  - The serial path is `narrowphase_serial` (`l5np systems.rs:438-492`), which does inline `set` and `push` and has no ctx or commit array. Say which way it goes.
  - Separately, `np_chunk` in the lane still carries a debug probe: `PROBE_MASK.fetch_or` per chunk on a shared static (`dispatch.rs:210-216`) and an `eprintln!` per dispatch (`:377-380`). It must not reach the base that L10 builds on.
- **O10.** A single pile's move-in step has no awake dynamic row, so it takes the C3a fast path. State that the A1 capture runs there too. N16 covers only P-a and D3.
- **O11.** `PHYS_NP_REPLAYED` and `SleepSkipStats::replayed` (rev 2 C2a) are obsolete under Δ1. §2 and §5 do not list them.

## Positive (preserve)
- **Δ1 (compute from the kept source):**
  - It removes the replay join, `manifolds_prev`/`sensor_prev` and the per-chunk replay cursor.
  - It is exact by L9-L1 at the default τ.
  - It is honest about what it gives up (§9).
- **The kept unit is a pair (B2).** It keeps the value-bearing REC-without-manifold state from L9 W2.
- **Invariant K, with its fixed ordering:**
  - the restore filter runs before capture;
  - capture may read a tombstoned copy;
  - compaction moves to A1.0b.
- **Two indexes, one per Off lookup function.** `restore_idx` uses L9's flip-surviving `key_prev`, and `restore_rec` uses the warm key. These mirror Off's two lookup functions exactly, and N4 pins the difference.
- **The A1 capture:**
  - a key search in place of a passed index list;
  - `carry_by_fid` at the t−1 index, which is exact by idempotence;
  - the Reset and warm-disabled cases handled.
- **One `rekeys()` predicate** is shared with L9's commit. The mirror runs before the scope and the commit after it, which extends L5 Lemma 1 (checked against `l5np axis_cache.rs:88-103`).
- **The chunk count comes from the stream length.** Tree all-held opens no scope (N13), and `commit = AXIS_NONE` with no stage write matches `np_chunk`'s run discipline (`dispatch.rs:221-281`).
- **The skip-stats array is sound:** one `Relaxed` store per chunk, ordered by the join, the same class as L5's `meta`.
- **`held_len` counts kept manifolds only** (N8). The epoch gains `reuse` and τ.
- **§4's arithmetic reproduces:**
  - np 0.478–0.804 ms;
  - Off′(1) 0.82–1.59 ms;
  - Δ 0.59–1.48 ms;
  - rehearsal threshold 0.584 ms.
- The 3–6× shrink of rev 2's Δ is stated plainly.

## Open questions for the architect
1. **Keys for a vanished member or a Reset flush.** How are `restore_idx` and `restore_rec` keys built when a member vanished (its row table entry has no current row), or on a Reset flush (there is no `prev_row`)? Off misses both anyway, so skipping is exact. But a bounds-checked `prev_row[NONE]` panics on the default despawn path.
2. **Sorting `restore_rec`.** It is two parallel columns, and `sort_unstable` sorts one slice. Each record's run is already sorted in t−1 order. Is this a k-way merge, or a `(key, slot)` scratch? Either way, which scratch id does it use?
3. **Where A1's `debug_assert` gets `j`.** It checks `run ∋ j` taken from `graph(t−1).island(a)`, but A2.4 "hands over no list". Where does `j` come from in debug builds?

Key files:
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/06-DESIGN-REV2.2.md`
- `D:/wt/joltab/crates/boyko_math/src/quat.rs`
- `D:/wt/joltab/crates/boyko_physics/src/resources.rs`
- `D:/wt/joltab/crates/boyko_physics/src/systems.rs`
- `D:/wt/l5np/crates/boyko_physics/src/narrowphase/dispatch.rs`
- `D:/wt/l5np/crates/boyko_physics/src/narrowphase/axis_cache.rs`
- `D:/wt/l5np/crates/boyko_physics/src/systems.rs`
