VERDICT: REVISE; BLOCKING=0; IMPORTANT=4

# Architecture review: L10 rev 2.3 (= `06-DESIGN-REV2.2.md` + `08-DESIGN-REV2.3.md`, read as one design)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED — no blocking remark; four Important ones, all in the gate set or in an unstated lookup rule, none in the exactness core.

Both texts are complete on disk: 08 opens with `# L10 rev 2.3` and ends with its `## Files` list (297 lines); 06 ends with its Sources (451 lines). Read in full, together with 04, 05, 07, `00-RULINGS.md`, the L9 / L11 / L5 designs, the L9 review, broadphase rev 2 §D3, the joltab tree and the l5np lane.

## 1. Status of 07's remarks

| item | closed? | how, and what I verified |
|---|---|---|
| **C1** (fill skips HELD rows that a sensor pair reads) | **closed** | D-F: `fill(r) ⇔ ¬HELD ∨ SENSOR_NBR`, set by the A2.2 scan on PRE_HELD ∪ CAND endpoints, HELD ⊆ PRE_HELD ∪ CAND, and `release` adds only non-sensor pairs (Invariant V). Reader enumeration below finds no unfilled row. E2′ restated; arm 7 + N19 can go red. |
| **W1** (no W ≥ 2 sleeping-on census arm) | closed in substance, **one gate defect → W1 below** | S8b at W=4 with `np_chunk::<true>` runs and N20 inside the Sets chunk prologue. The census is a `#[global_allocator]` with static atomics (`alloc_frame_census.rs:715-750`), so a worker-side allocation is counted: N20 can go red. The scope pin's formula is wrong (W1). |
| **W2** (sensor pairs in B2 and in RESTORED routing) | **closed** | D-G: one `sensor_pair` gates skip, B2, anchors, D2, D-F. Arm 8 (static + kinematic trigger, cached SEP pair, restore) with N21/N22. A moving trigger is no longer an anchor, so rev 2's no-churn returns. |
| **W3** (`dt` missing from the epoch) | **closed** | D-E′ adds `dt_bits`; `dt` is stamped at `systems.rs:219` (`cfg.dt = fixed_time.delta_secs()`) — verified. The config table enumerates every `collide_box_pair` input. Arm 4b's thresholds reproduce from L9 D8 (τ/(2√3·dt) = 0.0173 m/s at 60 Hz, 0.0087 at 30 Hz), and 0.012 sits between them. N23 can go red. |
| **W4** ("exactly 0" is false; REC always settled) | **closed** | `quat.rs:171-173` verified: for `conj(q)⊗q` the x term cancels exactly, y and z leave the rounding error of the first sum, s ≈ 1e-8. B5′ admits REC iff `tag & (HIT\|SETTLED) ≠ 0`; B3′/E4′ no longer rest on zero. The bound τ_eff ≲ 3·s·r_S follows from the criterion (2√2·s·(r_S+τ) ≤ τ). The scene witness (arm 9) is vacuous as written → W3 below. |
| **W5** (transitions into Off) | **closed** | D-H: `np_sets = step_mode == Sets ∨ restore set ≠ ∅` selects the arm; P-a takes `restore_rec` by stamp on both arms of `systems.rs:1097-1119` (`:1105` is `if cfg.sleeping`, verified). E7′. N30 can go red. |
| O1 | closed | Rule 5 in §10 Q2; N26 voids the runner closure (`reused` and `sep_hits` each overcount by `held_skipped`). |
| O2 | closed | Pre-C0 refutation, 0.6 × 0.59 + 0.23 = 0.584 ms (arith. reproduces). |
| O3 | closed, wording only (see O-1) | `l5np axis_cache.rs:98-103` is Lemma 2, verified. |
| O4 | closed | N27 + arm 10 (a stale commit byte makes `commit_axes` `set` a wrong axis under the held key; the mirror compare reds). |
| O5 | closed | N3′ is lookup-side; red via the flip at the restore step. |
| O6 | closed | Hit count = Σ `kept_rec[run].count`; `count == hits` holds because L11 D3 compacts hits into the new record. Solve keeps `Res<Manifolds>` (`systems.rs:1100`, verified). |
| O7 | closed | `SRC_RESTORE` = `Plan.flags` bit 2; `NONE` tested first; N28 covers the seed and the carry. |
| O8 | closed | B1′ defines bits 6–7 on HIT/SEPHIT; `HIT_INVARIANT = !(0x0400\|0x0800) = 0xF3FF` (checked against L9's bit layout); `steady()` deleted; N25 exhaustive. |
| O9 | closed | `l5np systems.rs:438-492` is `narrowphase_serial` with inline `set`/`push`, verified; D1′ keeps it and shares `np_route`/`collide_box_pair`. The probe grep over the lane reproduces exactly: `colored_tests.rs:1992, :2067, :2171, :3091`, nothing in `dispatch.rs`. |
| O10 | closed | Fast path runs P-a, A1 capture, D3, swap, stamp — capture before the swap, as A1 requires. N29. |
| O11 | closed | `PHYS_NP_REPLAYED` and `replayed` removed; `profiling.rs:77-131` verified as the counter site. |
| OQ1 | closed | `NONE` rows skipped (no current pair can name a vanished row); Reset flush writes both sources at length 0 — exact, since L9 misses every pair and `manifold_pair` is `None` on Reset. |
| OQ2 | closed | `restore_sort` (16 B, +1 id → 21), `sort_unstable_by_key`, gather. Uniqueness argument holds: a kept manifold unions its dynamic endpoints, so it lives in one record; K duplicates are pairs without manifolds and live in `restore_idx`. |
| OQ3 | closed | Debug `(slot, j)` list in `SleepSets::scratch` after the `order` merge, stamped; capture order = slot order because `graph(t−1).island(a)` indices are ascending stream indices. |

## 2. File:line and arithmetic verification

Every site 08 cites exists and says what 08 says: `quat.rs:171-173`; `systems.rs:219`, `:297-349`, `:600` (`let kernel = cfg.sdf_narrowphase`), `:1097-1119`/`:1100`/`:1105`; `resources.rs:130-134`, `:2269-2327`; `profiling.rs:77-131`; `scratch_ids.rs:889`, `:937-939`; l5np `dispatch.rs:115-124` (chunk_count), `:150-163`, `:170-190`, `:210-276`, `:253`; l5np `systems.rs:393-429`, `:438-492`, `:464`; l5np `axis_cache.rs:62-73`, `:98-103`.

"arith." items: D-F 1,241 × 10 ns = 12 µs (L9 D2's own figure); 12/230 = 5.2 %, 12/110 = 10.9 % → "5–11 %" ✓; 12/180 = 6.7 %, 12/80 = 15 % → "7–15 %" ✓. Pre-C0 0.584 ✓. Arm 4b bounds ✓. `HIT_INVARIANT` ✓. Ids 20 + 1 = 21 ✓. `HeldIsland` 12 × u32 = 48 B, `SleepSkipStats` 48 B, `RestoreSort` 16 B ✓. S8b: `chunk_count(n, 4) ≥ 2 ⇔ min(24, n/128) ≥ 2 ⇔ n ≥ 256` ✓ matches both phases.

## 3. New-defect walk over the delta

- **Determinism under W.** Every Sets-arm write is per slot (`tag[k]`, `commit[k]`, no stage write on Skip); `np_route` reads only `RowCls`, `restore_idx`, `kept_reuse`, all serial-written before the scope; the skip stats are per-chunk `Relaxed` stores summed after the join (an integer sum is order-free); `SENSOR_NBR`, `restore_sort`, `restore_idx`, the A1 capture, the D-H flush path are serial. The restore cursor's chunk start is a `lower_bound`, so `j(k)` is partition-free — *provided* jumper pairs are binary-searched (W2).
- **Fill predicate — readers of `row_frames`.** `Obb::from_frame` for both endpoints of every computed box pair; `criterion` (R_F) and `refresh_face` (R_inc) inside the same `collide_box_pair`; the SDF stage skips HELD rows. A computed pair with a HELD endpoint is, by `np_route`, a sensor pair (`!s && HELD → Skip` precedes everything), and that endpoint carries `SENSOR_NBR` because: (a) the pair is in the stream (T2 keeps sensors out of S-fit; Z rows are found by every Q query, bp rev 2 D3.1 "Q rows query all three trees"); (b) the scan ran (a HELD row was PRE_HELD or CAND); (c) nothing sensor enters after the scan. A `(RESTORED x, HELD y)` non-sensor pair is skipped — correct, because y's record is restored by D2 in the same step whenever x's pose changed, and otherwise the pair is still a fixed point. **No unfilled read found.**
- **Epoch inputs.** `collide_box_pair` reads `contact_reuse`, τ (and its clamp — body-side, R3), `dt` (via `is_fast`), sensor bits (R3), frames (R3), hints (D-C). The solve does not touch a held island; the warm carry depends on `warm` (rev 2 D5). The broadphase has no velocity expansion (`systems.rs:319-321`), so `dt` cannot move the pair set. **Complete.**
- **REC admission by HIT|SETTLED — can a record with a changed contact set be admitted?** No. At move-in the island is CAND (frozen at t−1, members bit-equal by R3), so the t−1 output is a fixed point of Off's map: HIT ⇒ `criterion(R, P)` holds again on unchanged (R, P); SETTLED miss ⇒ the hint at t equals the axis chosen at t−1, so `build(full(P))` is rebuilt bit for bit. "Contact set differs from the full compute" is L9b's own semantics and equally Off's. Cross pairs use Y's copy, admitted under the same rule at Y's move-in and watched by D1a since. The `HIT_INVARIANT` mask is exactly what makes a SETTLED-miss capture compare equal to Off's later HIT slot.
- **S8b can fail** (N20, global allocator), but its scope pin cannot pass on correct code under one arm (W1).
- **Principle 0.** `restore_sort`, `np_sets`, the debug `j` list all live in `SleepSets`/ECS columns; the per-chunk stats array is transient stack scratch (L5 D7 precedent). No side store.
- **Unsafe.** The delta names no new site; the Skip writes go through L9's tag write and L5's commit write. State that explicitly (O-4).
- **Gates that cannot fail.** Arm 9 (W3), N32 (O-2), the `& SENSOR` grep versus the design's own `debug_assert` (O-3).

## Remarks

### Critical
None.

### Important

#### W1. S8b's scope pin omits two scope sources that exist at W=4
**Where:** §7 S8b "scopes = S8's count + `[chunk_count(stream, 4) ≥ 2]`".
**Problem:** S8 is W=1, parallel off (`04:538`), where neither the solver nor the broadphase opens a scope. S8b runs W=4 with L4's `parallel_solve` default on and one arm on Grid-parallel:
- the colored solver opens a `pool.scope` per wide colour per pass (`colored.rs:215`, L5 D8's "133 per step in S1c"), and the stirred pile plus a projectile every 5th step changes the colour structure step to step;
- the Grid's parallel emit opens its own `pool.scope`s (`resources.rs:1982`, `:2074`, above the min-work threshold at `:669`).
**Consequence:** on the Grid-parallel arm the pin is red on correct code; on the Tree arm it holds only while every colour stays below the solver's dispatch width, which the scene does not constrain. Either way the arm cannot serve as the per-step pin W1 asked for.
**Confidence:** CONFIRMED (both scope sites cited; the formula has no term for them).
**What is needed:** pin in L5 C4's structural form (`scope_delta − Δnp_dispatches − Δbp_dispatches ≡ 0 mod passes`, or the measured envelope as G-L5-5 does), or assert `widest < MIN` on every step and use the Tree arm only for the exact count. Keep the 0-heap-beyond-scopes pin as is.

#### W2. The Restore route does not say how a jumper-endpoint pair is looked up
**Where:** 06 D1 ("restore_idx cursor", "the per-chunk start … is a `lower_bound` … as for L9's join"); 06 B4 ("does `lower_bound` in `restore_idx`"); 08 A3′ (keys via `prev_row` for every translated row).
**Problem:** D1 restores every record touching a jumper (rev 2 D12), so a restored record's pairs include jumper rows on every swap-remove despawn (the moved tail row). Their `key_prev` is non-monotone in stream order. L9's Stream join handles this with a binary search per jumper pair (L9 D9, `jumper_bits`); the Restore route is described once as a cursor and once as a per-pair `lower_bound`. If it is a monotone cursor without the jumper branch, a jumper pair misses.
**Consequence:** on the restore step of any held island whose member became a jumper, Sets computes that pair cold while Off's join hits its t−1 slot (REC or SEP): different manifold bits under L9b, different tag, pose divergence from that step. Arm 1 covers only the flipped case; N4/N14 do not name this.
**Confidence:** PLAUSIBLE (the text is ambiguous; the failure is certain if the cursor reading is the one built).
**What is needed:** state that the Restore route reuses L9's `PairJoin` (cursor plus `jumper_bits` binary search) over `restore_idx.keys`, and add the mutation "jumper pair merged on the restore cursor" (L9 M-c2's twin), red via a swap-remove despawn of a held member's tail-row neighbour.

#### W3. S3 arm 9 (τ = 0) holds no REC pair in a gravity pile, so B5′ has no scene witness
**Where:** §7 arm 9 "τ = 0: pile held, restore, wake. This is the witness for M3′"; N24's note that arm 9 "is only a witness".
**Problem:** with τ_eff = 0, L9 D8's `is_fast` is `3·dt²·(…) > 0`, true for any non-zero relative velocity, and rev 2 D13 keeps the at-rest velocity. A pile that settled under gravity carries residual velocities, so every box pair is fast, takes today's path and is never REC. The arm then exercises M3′ on zero REC pairs and passes from emptiness.
**Consequence:** the only scene-level check of W4's fix is void; N24 is an exhaustive test of the predicate against its own definition, which the design already concedes.
**Confidence:** CONFIRMED for the mechanism (formula); PLAUSIBLE that the arm is a gravity scene (unstated).
**What is needed:** the arm counts REC pairs admitted with HIT = 0 ∧ SETTLED = 1, ≥ 1, else void → red; to make that count reachable use exactly-zero relative velocity (gravity off, bodies placed at rest), where τ = 0 gives a slow pair that misses every step and is SETTLED.

#### W4. `RowCls` on a Sets step with nothing held: the routing reads flags that A1.3 may not have rewritten
**Where:** D-H "Later Off steps: `np_sets = false`, and `RowCls` is not read" (silent about Sets steps); rev 2 W3 ruling allows "skip the pass when nothing is held or CAND"; D1′ reads `RESTORED`, `HELD`, `SENSOR` per pair whenever `np_sets`.
**Problem:** under Sets, `np_sets` is true even when nothing is held, so `np_route` reads `RowCls` every step. If A1.3 is skipped on such a step and the flags are not zeroed, the step after a full wake still carries `RESTORED` (set at the wake step): pairs route to `restore_idx`, whose stamp is stale and is "treated as empty", so they compute cold. Off computes them from the t−1 slot, which was computed. Likewise a stale `SENSOR` fails D-G's `debug_assert_eq!`.
**Consequence:** a divergence on the step after every wake, for any scene where no other row is held or CAND — the common single-pile case. S3 would show it loudly, but the design must decide it, not the test.
**Confidence:** PLAUSIBLE (depends on which of the ruling's three options is built; the delta does not say).
**What is needed:** one sentence: A1.3 rewrites every row's flags on every step L10 runs (or a skipped pass zeroes `row_cls`), and the mutation "flags not rewritten on a nothing-held step" red via any restore arm's next step.

### Optional
- **O-1 (from 07 O3).** The Lemma 2 amendment says "the serial `set` is an identity". Under Sets the serial loop calls no `set` for a held pair (D1′), so both paths leave the slot to the mirror; equality with Off's lookups is D-C's argument, not Lemma 2's. Wording only.
- **O-2.** N32 ("restore sources not emptied on a Reset flush", red via the missed-gather arm) cannot go red as stated: on a Reset step `manifold_pair` and `key_prev` are `None` for every manifold and pair, so neither source is searched, and on the next step the stamp is stale. Replace with a spec assert (`len == 0` on Reset, debug) or drop it and record why.
- **O-3.** The grep gate "`& SENSOR` appears only inside `sensor_pair`" fires on the design's own `debug_assert_eq!(SENSOR, ba.is_sensor)` in `np_route`. Route that assert through `sensor_pair(fa, 0)` or exempt it.
- **O-4.** State the `unsafe` delta explicitly: the Sets arm adds 0 new sites if Skip reuses L9's tag write and L5's commit write, else list them with the same partition SAFETY.
- **O-5.** D-F prices "all rows" (12 µs) as the alternative, but L9's ruled O3 ("skip rows no pair will read") already fills ~0 rows at J-Son under Tree; D-F's real edge is under Grid, where a stream-touched mask would fill all 1,241 held rows and `¬HELD ∨ SENSOR_NBR` fills none. Say so, and add the fill predicate's composition with L9's mask to the Q2 cross-lane contract.

## Positive (preserve)
- **D-F's predicate** is the minimum that covers every reader, proved by the route order (`Skip` before `Restore`) plus HELD ⊆ PRE_HELD ∪ CAND plus Invariant V; it keeps the Sets floor at J.
- **One `sensor_pair`** across five decisions, tied to S5's routing predicate by a debug assert — the churn fix falls out for free.
- **B5′ + `HIT_INVARIANT`** together: the admission rule is the fixed-point condition, and the mask is exactly the bit that a SETTLED-miss capture and Off's later HIT slot differ in.
- **D-H's `np_sets`** decouples the narrowphase arm from the mode with one bool written where the mode is decided; the solve gates on the stamp only.
- **A3′'s Reset rule** (both sources at length 0) is exact for the right reason: both Off lookup functions miss by construction on Reset.
- **O6's count = hits** by D3's compaction retires the duplicated-fid gate cleanly; O7's flag bit removes the `NONE` collision.
- **Arm 4b** is a real value flip at a stated velocity between two derived bounds, with a void-is-red rule.

## Open questions for the architect
1. Is the L9 lane's `row_frames` fill a per-row predicate pass (D-F composes as AND) or lazy in the parallel phase (then D-F has no site)? The L9 C1 shape settles it.
2. On the D-H flush path with `m = Off`, does the `sleeping-off` solve arm (`solve_colored`) gain the `restore_rec` parameter, or does `SleepSets` join its access set? 08 says "access set unchanged" for the *sleeping* arm only.
3. Cross pairs are REC-without-manifold or SEP only (a manifold would have unioned the islands). Worth stating in B3, since it is what makes `restore_rec` keys unique (A3′) and `warm_hits` per record well-defined.

Key files:
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/08-DESIGN-REV2.3.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/06-DESIGN-REV2.2.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/07-REVIEW-OF-REV2.2.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`, `03-REVIEW-OF-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/02-DESIGN-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L5-narrowphase/02-DESIGN-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/broadphase/04-DESIGN-REV2.md`
- `D:/wt/joltab/crates/boyko_math/src/quat.rs`
- `D:/wt/joltab/crates/boyko_physics/src/systems.rs`, `resources.rs`, `profiling.rs`, `scratch_ids.rs`, `solver/colored.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_census.rs`
- `D:/wt/l5np/crates/boyko_physics/src/narrowphase/dispatch.rs`, `axis_cache.rs`, `D:/wt/l5np/crates/boyko_physics/src/systems.rs` (lane state)
