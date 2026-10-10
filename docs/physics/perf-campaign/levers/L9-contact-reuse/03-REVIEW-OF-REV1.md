VERDICT: CHANGES REQUESTED; BLOCKING=0; IMPORTANT=2

# Architecture review: L9, contact reuse (L9a exact fast path + L9b reuse within a tolerance)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED

I walked every section of the plan and checked it against `D:/wt/joltab`. I read `box_box.rs`, `systems.rs`, `colored.rs`, `axis_cache.rs`, `row_identity.rs`, `manifold.rs`, `plugin.rs`, `frozen_island_warm_start.rs`, `support_loss_wakes_sleepers.rs`, the parity runner, the P0b ANALYSIS, the L5 rev 1 design and the L10 rev 2 design. Every file:line the plan cites matches the tree.

Topics the plan leaves out, none of which causes a problem here:
- **New SIMD:** the refresh is about 30 ns.
- **Software prefetch:** the join and the record streams are monotone.
- **Non-temporal stores:** the records are read again the next step.
- **PGO.**

## Critical
None.

## Important

#### W1. The axis-hint table is not re-keyed for hit pairs on Rows, clear or grow steps
**Where**: D10; Interactions → Axis cache; the L10 rev 2.2 amendment list.

**Problem**: D10 says a hit leaves "the axis of the last full collision, which is exactly the record's". That holds only on Identity steps with no clear.
- On a prefetched (Rows) step, the old-key axis is read into `remapped[k]`, but only for that step (`axis_cache.rs:64-69`, `:367-388`).
- An entry moves to the new rows only when the pair calls `set(a, b)` (`systems.rs:437-446`), and a hit never calls `set`.
- After `begin_frame` clears the table on growth or high occupancy (`axis_cache.rs:261-281`), hit pairs never put their entries back.
- So a hit pair's first miss after a row move or clear reads `None`. After a swap-remove it can instead read a stale axis that belonged to another pair under the same key.

**Consequence**: This shows up in any scene with despawns, archetype migrations or pair-count growth.
- When such a resting near-parallel box pair next misses, it loses its hysteresis. If its face was being held by hysteresis (for example FaceB within `HYSTERESIS_RATIO`), the rebuilt record switches to the other reference face.
- That changes all four feature ids, so the manifold gets no warm start for one step. This is exactly the flicker the cache was built to prevent (`axis_cache.rs:4-13`).
- None of the J, R or G-TW rows moves rows, so no planned gate would see it.
- L10 rev 2's exactness step E5 needs the axis table to hold the same keys and `occupied` count as Off. To get that, it mirrors held box keys on Rows, clear and grow steps (`04-DESIGN-REV2.md:332`, `:387`). Under L9, Off stops inserting keys for hit pairs, so that mirror makes Sets ≠ Off. The plan's list of "required amendments" does not include this.

**Confidence**: CONFIRMED for the mechanism. How often it happens depends on the scene.

**What is needed**:
- Pick one of two rules:
  - keep the table keyed for hit pairs on every step where the key set changes (prefetched, cleared, grown); the tag already carries the axis through the row-translated join;
  - or state the loss and test for it.
- Then align the L10 amendment with the rule chosen.
- D10 rejected the tag fallback because it "changes values on flicker pairs". That reason only applies to the bit-identical commits (C1–C3 and reuse off), not to the reuse-on path.

#### W2. Whether a manifold exists now depends on history, and nothing gates A4 on box piles
**Where**: D6/D7, L9-L1, Interactions → A4, the list of correctness bounds at C4.

**Problem**:
- Today, whether a box pair emits a manifold depends only on the two poses (`axis_cache.rs:62-73`). That is why losing a hint cannot change a frozen island's manifold count and wake it through `IslandSleep::begin_step`.
- Under L9b it depends on the record as well as the poses. `refresh(R_old, P)` and `refresh(build(full(P)), P)` can emit different counts at knife-edge contacts:
  - all of `R_old`'s kept points have lifted, while the full clip keeps a point at `sep ≤ 0`;
  - or the reverse.
- L9-L1 only covers the case where the carry survives. A frozen pair loses its carry when a swap-remove flips its row order (`row_identity.rs:146-157`) or when the cursor resets. After that, its manifold count can change.

**Consequence**:
- A frozen box pile, with sleeping on as it is by default, can wake when an unrelated despawn swap-moves one of its knife-edge members. J and R create such members: they spawn lateral neighbours exactly one box pitch apart (`jolt_parity_pyramid.rs:678-683`). A4's documented invariant says this cannot happen today.
- The test the plan cites for A4 cannot fail under L9b. In `support_loss_wakes_sleepers` every dynamic body is a sphere (`:193-205`).
- So no test exercises D6's chain "all points lifted ⇒ no manifold ⇒ count changes ⇒ island wakes" on boxes. A hit that wrongly kept its manifold would leave an upper box frozen in mid-air, and the suite would stay green.

**Confidence**: CONFIRMED for the mechanism; PLAUSIBLE for how often it happens.

**What is needed**:
- Decide whether this spurious-wake path is accepted (state it and pin it with a test) or closed. One way to close it: records survive an order flip. The pose check is by the larger and smaller body, not by A and B, so only `REF_IS_B` would need flipping.
- Add a box-pile arm to the A4 tests that moves the support by less than τ_eff, so the D6 chain can go red.

## Optional

- **O1. Thin boxes.**
  - τ_eff is clamped by 5 % of the bounding radius. A 1 m plate with 0.5 mm half-thickness therefore gets τ_eff = 1 mm.
  - An unseen corner can then cross the plate's mid-plane before a miss happens, and the next full SAT may push toward the far side.
  - Clamping by the smallest half-extent would bound this.
- **O2. Record memory scales with all pairs.**
  - Records are indexed by pair slot, so they commit 2 × P × 128 B however few pairs actually touch.
  - At broadphase rev 2's 100k-body / ~770k-pair figure (`04-DESIGN-REV2.md:165`, `:528`) that is about 0.2 GB.
  - State this in D9's trade-off.
- **O3. The `row_frames` fill is serial.**
  - The prologue costs O(N) however many box pairs exist.
  - Under L10 with everything held, the J step costs 0.11–0.23 ms; the fill adds about 12 µs, or 5–11 %. In sparse scenes it is a net loss.
  - Skipping rows no pair will read, or filling in the parallel phase, avoids it.
- **O4. Some gate specs can pass when they should fail.**
  - M-b2 ("F = A always") changes nothing unless the test puts the slab in body B. Every scene in the tree spawns the floor first (`jolt_parity_pyramid.rs:669`, `frozen_island_warm_start.rs:310`), so the floor is A and already the larger body.
  - M-c3 looks unobservable, because `remap` is already classified in the prologue.
  - G-L9b-1 allows 2e-6 m on corner separation. When the slab is the incident body its local points sit ~70 m out, which gives f32 noise of about 4–8e-6 m. The "≈1e-7 m" claim holds only for box-sized incident bodies.
- **O5. Tags for every pair kind.** Every pair kind, sensors and spheres included, must write its tag every step. Otherwise stale REC tags survive two buffer swaps. The W4 counter closure would void such steps, but the rule belongs in the algorithm.
- **O6. L9a has no realized-gain rule.**
  - Its G-TW row only checks for a regression.
  - On J-A at W=1 its prediction is 2.3–4.7 % of T; the "4–8 %" in the plan is J-D's figure. The SE bar there is 2.0 %.
  - As written, only the C0 bench can refute L9a's gain.
- **O7. Tag layout.**
  - The plan says its tag bits are "exactly L10 rev 2's layout". L10 lists `axis|SETTLED|BOX|SET|PUSHED` (`04-DESIGN-REV2.md:112`); L9 uses `axis|BOX|PUSHED|SET|SETTLED`.
  - An L10 replay that copies the tag (amendment 3) reproduces step t−1's REC bit without HIT, so the reuse counters would differ between Sets and Off.

## Positive (keep these)

- **D1.** It refuses to cache "no contact", and it refuses to add a speculative margin that would inflate the per-manifold denominator. Both are correct and honest.
- **L9a is exact.**
  - Its early exit and cached axis evaluate only a subset of the same `eval_axis` calls, so the result cannot change.
  - A cached separating axis stays exact even if the join lands on the wrong slot, because any negative axis proves separation.
- **D4.**
  - The pose check is made in the larger body's frame and scales rotation by extent; the bound needs no sqrt.
  - I checked the Jolt figures it rejects against `PhysicsSettings.h`: 1 mm, cos(2°/2) and a 0.02 m speculative distance.
- **D7.** A miss emits `refresh(build(full))`, so the output depends only on the record and the current poses. I traced `frozen_island_warm_start`'s twin-exactness test through this:
  - the narrowphase still runs on frozen pairs;
  - frozen velocities are preserved, so D8's fast/slow classification cannot differ;
  - so the wake-step output equals the twin's step-F output.
- **D9.** Records hold no row data, and the join goes through row translation. A record is also self-checking: the shape pin plus the pose check mean a wrong join can only cost hit rate, not correctness. The carry is shared with L10.
- **Refutation gates:** C0's refutation rule, G-L9b-6's minimum hit rate, reuse-off pinned to exactly 0.7180 mm and to C0's pose hash, and an honest "cannot claim" list.

## Open questions

1. **Build-if ruling.** ANALYSIS §9 row 6 says L9 fails its build-if rule once L5 lands. That needs a ruling (plan OQ1) before C1, not after C0.
2. **No research stage ran.** The owner's rule is that research always searches the web.
   - The load-bearing Jolt facts check out.
   - D4's description of Rapier is out of date. Current parry keeps normals in each shape's local frame (`local_n1`/`local_n2`), with a cos 1° threshold and a 1 mm per-point threshold (`DIST_SQ_THRESHOLD = 1e-6`).
   - This only affects a rejected alternative, but the rejection text should be accurate.
3. **Knife-edge pairs on J.**
   - J's 2,240 lateral and 2,030 diagonal pairs sit at a gap of about 0. The early exit alone (i) already returns for them at axis index 0–2, so the cached axis (ii) adds little on J.
   - C0 should report SEP_HITS against FULL for separated pairs, so L9a's gain can be split between (i) and (ii).
4. **The pair-list carry has no stamp.** `pairs_prev` is swapped in the broadphase, while `tag_prev` and `reuse_prev` are swapped in the narrowphase, and nothing ties the two swaps together. Because records check themselves, a mismatch costs only hit rate. Confirm this property is intended and document it, or stamp `pairs_prev` with the gather sequence number.

Files cited (all under `D:/wt/joltab/`):
- `crates/boyko_physics/src/narrowphase/axis_cache.rs`
- `crates/boyko_physics/src/systems.rs`
- `crates/boyko_physics/src/row_identity.rs`
- `crates/boyko_physics/tests/support_loss_wakes_sleepers.rs`
- `crates/boyko_physics/tests/frozen_island_warm_start.rs`
- `crates/boyko_physics/benches/jolt_parity_pyramid.rs`
- `docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`
- `docs/physics/perf-campaign/levers/broadphase/04-DESIGN-REV2.md`
- `docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`

Sources:
- [Jolt PhysicsSettings.h](https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSettings.h)
- [parry contact_manifold.rs](https://raw.githubusercontent.com/dimforge/parry/master/src/query/contact_manifolds/contact_manifold.rs)