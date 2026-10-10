# Research: L9, contact reuse (the pair/manifold cache), for boyko_physics at `D:/wt/joltab`

Scope: read-only research. No files were built or written. "arith." marks numbers I calculated from P0b spans and counts; they were not measured.

## Brief summary (TL;DR)
- **Jolt (v5.3.0 and master have the same logic) reuses a pair's result when the relative pose has moved less than 1 mm and less than 2°.**
  - The pose is measured in body 1's frame: `inv_r1·(com2−com1)` and `inv_r1·q2`.
  - It is compared with the pose at the pair's **last full collision**. On a hit the whole `CachedBodyPair` is copied with `memcpy`, so the reference pose does not move.
  - "No contact" results are cached as well.
  - Points are stored in each body's local frame and the normal in body 2's frame. **Separation is never stored.** It is recomputed from the current transforms at constraint setup and on every position iteration.
  - On the parity scene Jolt hits about 100 % of pairs: 8,456 cached-manifold adds against 8,541 `ProcessBodyPair` calls and 0 new pairs at frames 200–400. Turning the cache off costs +43.9 % at W=8 (ANALYSIS §1, §3).
- **Box3D, Box2D v3 (main) and Rapier 0.35 use a looser tolerance that grows with body size.**
  - Box3D: 5 cm while touching, 2 cm while not touching. The test is a conservative-advancement arc bound, plus a 10° limit on each body's world rotation.
  - Rapier: 0.05 × length unit, plus an absolute rotation limit of cos Δθ > 0.98 (about 11.5°).
  - Both keep the world normal and lever arms frozen, which is why they need the absolute rotation limit. They refresh only the separation.
  - PhysX PCM instead refreshes the points it keeps and drops any point whose tangential drift is too large.
- **Every engine that reuses contacts relies on a speculative margin, and boyko has none.**
  - Jolt's speculative distance is 2 cm, far above its 1 mm tolerance. Box3D's non-touching tolerance is `min(recycle, speculative)`.
  - boyko keeps only points with `separation <= 0` (`box_box.rs:75-81`, `:940`). A cached boyko manifold therefore cannot contain a feature that is about to touch.
  - A cached boyko "no contact" carries no proof of clearance.
  - This is the central design fork. Adding a margin would also raise the manifold count, and so move the per-contact denominator.
- **Tree facts that shape L9:**
  - Manifold anchors are world points, and `separation` is fixed at gather time for the whole step (`manifold.rs:54-71`; `colored.rs:1778-1805`, `:2048-2095`; `soft_step.rs:115-117`). A reused manifold needs refreshed anchors, normal and separation.
  - The SAT evaluates all 15 axes before it tests separation (`box_box.rs:275-299`), so the 5,037 separated pairs (53 %) pay close to the full SAT cost.
  - `ColliderShape` is `Sphere | Box` only.
- **Gain (arith.): at most h·t_np, minus the cost of the hit path.**
  - W=1: 2.2–2.8 ms at h≈1, with a hit cost of 40–100 ns per pair (an estimate). That is 20–25 % of the derived with-simd T(1).
  - W=8 after L5: 0.40–0.51 ms (7–9 %).
  - L5 gives nothing at W=1, so **at W=1 L9 is the only lever on the narrowphase**.
  - Per-manifold parity on collision plus setup at W=1 cannot be reached while the broadphase is AllPairs at 2.10 ms, whatever L9 does.

## Approaches in current engines

### Jolt (v5.3.0 tag checked = master)
- **Entry point.** `ProcessBodyPair` tries `GetContactsFromCache` iff `mUseBodyPairContactCache` (default true) and neither body is `IsCollisionCacheInvalid()`. On a hit, collision detection is skipped.
- **On a miss.** `AddBodyPair` runs "regardless of whether collision is ultimately detected ('we want to remember that no collision was found too')".
- **Criterion.** Verbatim from v5.3.0:
  - `delta_position = inv_r1 * Vec3(com2 − com1)`, then `if ((delta_position − old).LengthSq() > mBodyPairCacheMaxDeltaPositionSq) return;`
  - `delta_rotation = inv_r1 * r2`, then `if (abs(delta_rotation.Dot(old)) < mBodyPairCacheCosMaxDeltaRotationDiv2) return;`
  - Defaults: `Square(0.001f)` (1 mm) and `0.99984769…` (cos 1°, so 2° in total).
  - The test is not scaled by body extent.
- **Hit path.**
  - `memcpy(output_cbp, &input_cbp, sizeof(CachedBodyPair))`: the reference pose stays that of the last full collision, so drift cannot accumulate.
  - If `mFirstCachedManifold == cInvalidHandle`, the pair is handled as "no contact".
  - Each manifold is copied (`memcpy … sGetRequiredTotalSize`).
  - Points go back to world space as `transform_body1 * ccp.mPosition1` and `transform_body2 * ccp.mPosition2`. The normal is `transform_body2.Multiply3x3(mContactNormal).Normalized()`.
  - `CalculateNonPenetrationConstraintProperties(... p1_ws, p2_ws, normal ...)` runs inside collision detection, so Jolt's FindCollisions includes the constraint setup.
  - Warm-start impulses come straight from the cached manifold through `SetTotalLambda`. `OnContactPersisted` is called.
- **Separation.** It is not stored. The position solve uses `p1 = transform1 * ccp.mPosition1`, `p2 = …` and `separation = max((p2−p1)·n + slop, −maxPen)`.
- **Storage.**
  - `CachedContactPoint`: 28 B (two local positions plus the normal impulse).
  - `CachedManifold`: 60 B for one point (normal in body 2's space, friction impulses, point count).
  - `CachedBodyPair`: 28 B (Δp and Δq as float3 each, plus the first-manifold handle).
  - `ManifoldCache mCache[2]` is a read/write double buffer, swapped with `mCacheWriteIdx ^= 1`. The hash map is lock-free.
- **Determinism and replay.** `SaveState` writes the read cache sorted by key (`GetAllBodyPairsSorted` / `GetAllManifoldsSorted`), so the cache is part of the saved state.
- **Other settings.**
  - `mSpeculativeContactDistance = 0.02`; it is 0 for sensors.
  - On a recomputed manifold, impulses are matched by local position within 1 cm (`mContactPointPreserveLambdaMaxDistSq`).
- **Failure modes on record.**
  - Issue #2125 (opened 2026-09-15, closed; no resolution shown on the page): after `SetShape` on a removed body, re-adding it with the same BodyID replays the stale manifold for the old geometry.
  - Unreleased release note: "Passing underestimate of penetration depth in OnContactPersisted when the contact comes from the contact cache."
  - Multicore scaling paper, as quoted in `04-PRACTICE-SURVEY.md:52`: beyond about 16 cores, "the lock free operations that are used to manage the contact cache dominate".

### Box3D (2026-06) and Box2D v3 main (the same design)
- **Where.** `b3CollideTask` (`physics_world.c`). It is gated by `!isFast`, `recycleDistance > 0`, `b3_relativeTransformValid`, and `b3_contactRecycleFlag`. That flag is set only if both bodies have `enableContactRecycling`.
- **Criterion.**
  - Each body's world rotation change: `min(dot(qA,qA0)², dot(qB,qB0)²) > B3_CONTACT_RECYCLE_ANGULAR_DISTANCE`, which is 0.99240388, i.e. cos²(θ/2) for 10°.
  - Translation: `‖xf.p − xfc.p‖ < tol` with `xf = inv(A)·B`, and `4·|modifiedCross(|qr.v|, maxExtent)|² < (tol − dist)²`. The code comment calls this "Variation of Conservative Advancement".
  - `tol` is `recycleDistance` = 10 × linear slop = 5 cm when the pair was touching, and `min(recycle, speculative 2 cm)` otherwise. A static body contributes an extent of zero.
- **Refresh.**
  - The world normal is kept. Anchors, stored relative to the body centres, are rotated by `dqA`/`dqB`.
  - Then `mp->separation = mp->baseSeparation + dot(dc + rB − rA, normal)`. The comment reads: "Keep anchors but update separation, same as sub-stepping. This eliminates jitter."
  - Recycling "also skips updating other aspects of the contact such as material parameters."
- **Anchoring.** The cached pose (`cachedRotationA/B`, `cachedRelativePose`) is written only on the path that does not recycle.
- **SAT cache (a separate technique).** The cached axis is re-evaluated with the current transforms, and "Cache hit, shapes are separated" when the separation is at least the speculative distance. Otherwise the pair is re-clipped on the cached axis, which is accepted if |Δsep| < linearSlop.
- **Documented costs.**
  - "improves performance but may lead to ghost collision that should be avoided on characters" (`types.h`).
  - Issue #155 (2026-09-05, closed): "Center of mass shift versus contact recycling — Need to invalidate contacts."

### Rapier 0.35 (2026-08-08)
- `contact_recycling` is on by default. The drift allowed is `normalized_contact_recycle_distance = 0.05` × `length_unit`, "ten times the linear slop".
- **Drift test.** `drift = |Δt| + 2·max_extent·sin(Δθ/2)` on `pos12 = co1⁻¹·co2`, compared with `max_drift`, plus a per-body `cos Δθ > 0.98`.
  - The reason is documented: "rotating rigidly together keeps the relative pose but invalidates world-frozen arms".
  - `max_drift` is chosen at the last full update and depends on whether the pair was touching.
- The reference pose is anchored to full updates only.
- **Disabled when:**
  - any collider change other than `IN_MODIFIED_SET | POSITION | LOCAL_MASS_PROPERTIES`;
  - user hooks are involved.
- On a recycled pair the solver rebuilds world points and separations from body-local anchors using the current poses.
- The changelog gives no speedup for recycling on its own.

### PhysX PCM (5.x)
- Contacts are persistent: points are stored in local frames, and `mRelativeTransform` is kept.
- **Invalidation thresholds.**
  - Position delta against `minMargin × {0.5, 0.125, 0.25, 0.375, 0.375}`, indexed by the number of contacts.
  - Quaternion thresholds `{0.9998, 0.9999, …}`; `0.99996` for primitive against plane ("about 0.5 degree").
- `refreshContactPoints` transforms each point by `aToB`, recomputes `dist` against the local normal, and drops points whose 2D projected drift exceeds a threshold.
- An invalidated manifold reruns GJK/EPA and adds one point.
- Documentation: PCM "might reduce stacking stability when simulating tall stacks with insufficient solver iterations".

### Bepu v2: no reuse
- Bepu v2 builds one-shot manifolds. Its author's reasoning: "Bepuphysics1 incrementally updated contact manifolds… I wanted the same level of quality and consistency that box-box face clipping offered" ("Seeking the Tootbird", 2022-03-10).

### Unity DOTS
- Not researched.

## Comparative table

| aspect | Jolt | Box3D / Box2D v3 | Rapier 0.35 | PhysX PCM | boyko (tree) |
|---|---|---|---|---|---|
| Criterion | Δp in body-1 frame ≤ 1 mm; Δq_rel ≤ 2° | arc-bounded drift < 5 cm touching / 2 cm not; per-body world rotation ≤ 10° | drift + arc ≤ 0.05·L; per-body rotation ≤ ~11.5° | margin × ratio(contact count); quat 0.9998–0.9999 | none (full recompute) |
| Scaled by extent | no | yes (`maxExtent`) | yes (`max_extent`) | margin-relative | — |
| Reference pose | last full collide | last full collide | last full collide | last full collide | — |
| Caches "no contact" | yes | recycled non-touching limited to the speculative distance | yes (`max_drift` depends on touching) | — | — |
| Refresh | local points and normal → world; separation implicit | separation = base + dp·n; world normal frozen | from local anchors; world normal and arms frozen | reproject; drop drifted points | — |
| Speculative margin | 2 cm | 2 cm (4 × slop) | 0.002 × L predictive | contact offset | **none** (`sep <= 0` only) |
| Cache storage | `mCache[2]`, lock-free hash | in the contact record | in the pair | in the manifold | — |
| Opt-out | global flag, per-body invalidate | per body; `isFast` | hooks; shape changes | — | — |

## Key techniques
- **Anchor the reference pose at the last full collision, not at the previous frame.** All four engines that reuse do this, and it is what keeps error from accumulating.
- **Relative pose in a body frame.** Jolt's local-frame storage stays exact for bodies that rotate together. A frozen world normal (Box3D, Rapier) requires an extra absolute rotation limit.
- **Refresh.** Either the rigid transport of local anchors, with separation recomputed as (pB−pA)·n (Jolt, Rapier, PhysX), or a base separation plus a first-order dp·n term (Box3D).
- **Caching negative results.** This is sound only with a clearance proof: a speculative margin above the tolerance, or a separating axis re-evaluated on the current poses (Box3D's SAT cache is exact for the "separated" outcome).
- **Invalidation triggers.** Shape change, mass or COM change, motion type, user hooks, fast or CCD bodies, removing and re-adding a body with the same id.

## Pitfalls and mistakes
1. **Ghost collisions from stale features.** Box3D documents this and advises turning recycling off for characters.
2. **New features missed inside the tolerance.** Only a speculative margin larger than the tolerance covers this. boyko has no such margin (`box_box.rs:75-81`).
3. **Stale geometry after identity reuse or a shape change.** Jolt #2125 and Box3D #155. In boyko this is hazard H-03: a recycled id carries a dead body's row state for one gather (`row_identity.rs:19-27`). Today that is a hint or an impulse; under L9 it would be a whole manifold.
4. **Fewer contacts or frozen points can weaken tall stacks** (PhysX docs). Box3D reports the opposite effect for its separation refresh: "eliminates jitter".
5. **Derived values reported from cached data can be wrong.** Jolt's unreleased fix of the `OnContactPersisted` penetration depth is an example.
6. **The lock-free cache itself limits scaling** at high core counts (Jolt).
7. **Material parameters are not refreshed on recycle** (Box3D). In boyko, friction and restitution are read from the bodies at solve build (`colored.rs:1760-1761`), so they are not part of the manifold.

## Tree facts (D:/wt/joltab @ f236ebdd)

**Narrowphase**
- `systems.rs:375-478` clears `manifolds` every step. For each pair it runs `match (shape, shape)` (`:413-448`).
- Box pairs read and write the axis hint (`:437-445`).
- Sensor routing is decided per pair (`:411`, `:460-464`).
- Counters are computed after the loop (`:472-477`).

**Manifold**
- 152 B, `#[repr(C)]`, at most 4 points. Anchors are "in world units", with a per-point `separation` and `feature_id` (`manifold.rs:54-122`). At most one manifold per pair.

**How the solver consumes it**
- `ra = anchor_a − pa` and `rb = anchor_b − pb` are taken at build (`colored.rs:1778-1779`).
- `separation` is a constant per step: "Signed separation at gather time" (`soft_step.rs:115-117`). It feeds `bias_rate·separation` in every substep (`colored.rs:2048`, `:2094-2098`).
- There is no per-substep separation update (a grep for `delta_position|base_separation` finds nothing).
- 4 substeps and 2 relax iterations (`resources.rs:135-140`).

**Separation equals (anchor_b − anchor_a)·n at emission**
- Edge contacts: exactly, by construction (`box_box.rs:1017`).
- Face contacts: `on_reference = pos − ref_normal·sep` (`:969`), so equal up to the difference between `ref_normal` and `sat.axis`. This is read from the code, not tested.
- Sphere-box: not verified (`sphere_box.rs:48-100`).

**SAT and speculative points**
- The SAT evaluates all 15 axes, then tests `depth < 0` (`box_box.rs:275-299`). Returning `None` early, or trying a cached separating axis first, gives the same `None` (inference from the code).
- Only `separation <= 0` points are kept (`:940`). There are no speculative points; Box3D keeps them, as the comment at `:75-81` notes.

**Axis cache**
- A single in-place open-addressed table, 16 B per slot, keyed `pack(a,b)` (`axis_cache.rs:97-117`).
- Cleared on grow, or when load exceeds 0.5 (`:261-281`).
- Written only when a pair produces a contact (`:319-354`).
- On steps where rows changed, it is pre-read through `RowIdentity` (`:367-422`).
- A dropped hint changes which contact is chosen, never whether there is one (`:62-73`).

**Warm table**
- Double-buffered, rebuilt every step in manifold order. Key: `a:24 | b:24 | feature:16`. Lookups are translated when rows move (`warm_start.rs:1-65`, `:114-140`).
- B1 carries entries of frozen islands (`colored.rs:1819-1843`).
- Reused manifolds would keep their feature ids, so warm-start keys would still hit (inference).

**Row identity**
- Four row-keyed stores exist today: the latch, two warm tables and the axis cache. U7's `PairCache` is planned to delete this file (`row_identity.rs:3-17`, `:45-51`).
- L10 would add a fifth store (kept manifolds). L9 would add a sixth.

**Resources and scratch ids**
- `Manifolds` is at `resources.rs:2269-2300`.
- `NARROWPHASE_COLUMN_COUNT = 3` (`scratch_ids.rs:889`); L5 raises it to 5, leaving 36 ids of headroom (per the L5 design).

**Counters**
- `phys_np_pairs`, `phys_np_manifolds`, `phys_np_points`, `phys_bp_pairs` (`profiling.rs:94-99`). **No counter or zone records per-pair relative drift, or splits the narrowphase cost into separated and touching pairs.**

**L5 (being built in D:/wt/l5np)**
- Per-pair work is a pure function of (bodies[a], bodies[b], hint). Hints come from the table as it stood before the phase (Lemma 1).
- Output is dense per-chunk runs joined in chunk order. The axis commit is serial, in pair order.
- Chunk floor: `NP_MIN_PAIRS_PER_CHUNK = 128`, derived from 0.329 µs per pair, which gives 42 µs chunks (`02-DESIGN-REV1.md` D1–D4).
- The design rejected concurrent CAS inserts because they "weaken the byte gate".
- At W=1 L5 takes the inline path, so its gain there is 0.

**Tree broadphase (rev 2)**
- Emits exactly AllPairs' pair set in `(min,max)` order (`04-DESIGN-REV2.md:36`). Predicted J broadphase at W=1: 0.16–0.24 ms (`:44`, `:317`).
- "A persistent active pair set would need pair identity across row changes, which does not exist before U5" (`:188`).

**L10 (rev 2)**
- Replay is an **exact** reuse: both `BodyState`s must be bit-equal (R3).
- State is carried one step as `pairs_prev`, `manifolds_prev` and a per-pair `pair_tag` (`axis:4|SETTLED|BOX|SET|PUSHED`), joined to the current stream by a monotone merge-join (D1, D2, D4, `:92-113`).
- Under Sets, held islands pay no narrowphase at all, so L9 gains nothing on frozen islands.

**Gates L9 must not loosen**
- **A7-R1:** reads 0.8583 mm against the 10 mm bound, over steps 600–3000. The drift rate is about 0.38 mm per 1000 steps (`tests/sleep_settles_box_piles.rs:64-69`, `:86-96`).
- **A7-R2:** the pile freezes at step 248 (`benches/sleeping_pipeline.rs:24`).
- **`support_loss_wakes_sleepers`:** sphere scenes. The A4 fix wakes an island when its manifold count changes. Case d is a 10 m teleport (`:22-33`).
- **`frozen_island_warm_start`:** the wake-step state must equal the twin's, bit for bit (`:37`).

**Replay contract KC-37**
- A replay starts from the startup world with no save loaded. The result must be bit-identical across machines, W and id assignment (`D:/claude/BoykoEngine/docs/unification/UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md:197-211`).
- A reuse cache is therefore internal state that must evolve as a pure, row-keyed function of the run.
- H-03 (generation-less ids) and H-16 (row order) apply to it.

## Expected gain and the per-contact metric

**Counts (P0b, J scene, per step).** 9,561 pairs; 4,519–4,524 manifolds; 16,888 points; 5,037 pairs (53 %) with no manifold.

**Hit-rate evidence**
- Jolt, same scene, 1 mm / 2°: about 100 % (8,541 pairs, 0 new, frames 200–400).
- boyko on J: **not measured**.
- boyko on R: the per-body drift is under 1 mm over 2,400 steps (A7-R1), so translation hits should be close to 100 % under a Jolt-style tolerance. Rotation drift is not measured (inference).
- R's pair count is not in ANALYSIS.

**Per stage, W=1, cfg-A armed (per manifold is ÷4,519; per pair is ÷9,561)**

| stage | ms | µs / manifold | µs / pair |
|---|---|---|---|
| broadphase | 2.100 | 0.465 | 0.220 |
| narrowphase | 3.142 | 0.695 | 0.329 |
| solve_build | 0.970 | 0.215 | — |
| warm_apply | 0.613 | 0.136 | — |
| store | 0.266 | 0.059 | — |
| wide colours (simd off / on) | 12.205 / 3.576 | 2.70 / 0.79 | — |

**Against Jolt**
- Jolt's FindCollisions includes its broadphase and the constraint setup (see above). At W=1 it is 3.0–4.6 ms: 0.355–0.544 µs per manifold, 0.351–0.539 µs per pair.
- boyko's comparable sum, broadphase + narrowphase + solve_build = 6.212 ms: **2.5–3.9× per manifold, 1.2–1.85× per pair** (arith.).

**Parity budget at W=1 (arith.)**
- Jolt's bracket × 4,519 manifolds = 1.60–2.46 ms for broadphase + narrowphase + setup.
- With today's broadphase (2.10) and setup (0.97), this is out of reach even with a narrowphase of 0.
- With the tree broadphase (0.16–0.24), the narrowphase must be ≤ 0.39–1.33 ms, i.e. **41–139 ns per pair**, against 329 ns today.

**L9 gain (arith.)**
- h is the time-weighted hit fraction; c is the hit-path cost per pair. c is not measured; 40–100 ns is an estimate.

| row | h = 1, c = 40 ns | h = 1, c = 100 ns | h = 0.5, c = 40 / 100 ns |
|---|---|---|---|
| W=1 (t_np 3.142) | −2.76 ms (≈25 % of with-simd T(1)) | −2.19 ms (≈20 %) | −1.38 / −1.09 ms |
| W=8 today (serial, 2.946) | −2.59 ms | −2.05 ms | — |
| W=8 after L5 (parallel part 0.577) | −0.51 ms (≈9 % of 5.5–5.7) | −0.40 ms (≈7 %) | — |

**How L9 and L5 overlap (arith.)**
- They attack the same 3.14 ms.
- After L9 at h≈1, t_np(1) ≈ 0.38–0.96 ms. A 128-pair chunk that is all hits takes 5–13 µs, comparable to ω(8) = 6.54 µs, so L5's chunk-floor arithmetic would need re-deriving.

## Relevant academic and industry works
- Gregorius, "Robust Contact Creation for Physics Simulations", GDC 2015. The tree's survey quotes it (`04-PRACTICE-SURVEY.md:6`, `:128`): "not to call any of those geometric algorithms at all if possible". I could not re-read the PDF; its rendering failed.
- Catto, "Contact Manifolds", GDC 2007. Not re-read.

## Applicability to boyko-engine (facts per design question; no choice made)
- **Can be taken directly:**
  - Anchoring to the last full collision (all four engines).
  - Pure per-pair decisions evaluated on the pre-phase snapshot. This composes with L5's Lemma 1 pattern, per-pair output slots and a serial commit.
  - Reused manifolds keep their feature ids, so the warm table and the B1 carry keep hitting (inference).
- **Needs adaptation:**
  - World anchors plus a fixed separation mean refreshed anchors, normal and separation are required. For a Jolt-style (pB−pA)·n refresh, the face-path equality at emission is exact only up to ulps.
  - Row-keyed state needs `RowIdentity` translation and the jumper rule. It can be merge-joined like L10's D4 carry.
  - Invalidation: shape (`BodyState.shape`), the sensor flag (routing at `systems.rs:411`), the one-gather recycled-id window (H-03), and SDF field edits (L10's epoch).
- **Does not fit as-is:**
  - A lock-free concurrent hash insert (Jolt's write cache) conflicts with the L5 byte gate and with principle 0's single-writer columns.
  - Jolt's "no contact" caching is unsound without a speculative margin or an exact separating-axis re-check.
- **Which pairs benefit:**
  - Sphere-sphere is a length and a compare, so a reuse check costs about as much as recomputing (inference).
  - Box-box is the heavy path.
  - The SDF stage is per body and its cost has never been measured (L5 design, SDF section).
- **Sleeping:** with L10 Sets, frozen islands skip the narrowphase, so L9's gain lies in awake contacts. That includes J parity, where both engines run with sleeping off.

## Open questions for the architect
1. Add a speculative margin (it changes values and the manifold count, and so the per-manifold metric) or a clearance proof (an exact cached separating axis or a stored gap)? Which denominator is the per-contact gate: per pair (invariant to the margin) or per manifold?
2. Tolerance form: Jolt's 1 mm / 2° relative, or an arc bound scaled by extent (Box3D, Rapier)? If the world normal is frozen, is an absolute rotation limit also needed?
3. The drift of J and R under each candidate tolerance has no counter. A diagnostic "would-hit" count per step is missing, as is a split of the narrowphase cost into separated and touching pairs.
4. Refresh scope: separation only (Box3D), or anchors, normal and separation (Jolt)? What bound on A7-R1 and A7-R2 does each give? It is unknown whether reuse raises or lowers the 0.38 mm per 1000 steps creep.
5. Storage: extend L10's D4 carry (merge-join), use a double-buffered rebuilt table like the warm table, or an in-place table like the axis cache? And what is its place in the row-keyed store family until U7?
6. `frozen_island_warm_start`'s bit-exact wake test and A4's wake-on-count-change both read state that reuse holds constant within the tolerance. Their exactness has to be re-argued (inference).
7. At W=1, L5 contributes 0. Should L9's build-if rule (currently "t_np(8) ≥ 20 % after L5", failed at 8.7 %) be restated for the owner's per-contact and W=1 goal? That is a values call.

## Sources
[1] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.cpp and …/v5.3.0/… — cache hit criterion, memcpy, world-space refresh, position-step separation, SaveState sorting
[2] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.h — CachedContactPoint / CachedManifold / CachedBodyPair layouts, `mCache[2]`
[3] https://raw.githubusercontent.com/jrouwe/JoltPhysics/v5.3.0/Jolt/Physics/PhysicsSettings.h (and master) — 1 mm, cos 1°, 0.02 m speculative distance, 1 cm lambda matching
[4] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSystem.cpp — ProcessBodyPair gating, caching of "no collision"
[5] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Body/Body.h — IsCollisionCacheInvalid, the InvalidateContactCache flag
[6] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Docs/ReleaseNotes.md ; https://jrouwe.github.io/JoltPhysics/md__docs_2_release_notes.html — the cache-penetration fix; the v5.6 friction model
[7] https://github.com/jrouwe/JoltPhysics/issues/2125 — stale manifold after SetShape and re-add with the same BodyID
[8] https://raw.githubusercontent.com/erincatto/box3d/main/src/physics_world.c — b3CollideTask recycling block, tolerances, separation refresh, anchoring
[9] https://raw.githubusercontent.com/erincatto/box3d/main/include/box3d/constants.h — recycle distance 10 × slop, angular 0.99240388 (10°), speculative 4 × slop
[10] https://raw.githubusercontent.com/erincatto/box3d/main/src/contact.h ; …/src/contact.c — flags, cached pose fields, per-body opt-in
[11] https://raw.githubusercontent.com/erincatto/box3d/main/src/convex_manifold.c — SAT cache: separated / contact-generated hits
[12] https://raw.githubusercontent.com/erincatto/box3d/main/include/box3d/types.h ; https://box2d.org/documentation3d/group__body.html ; https://box2d.org/documentation3d/group__world.html — ghost-collision caveat, recycle distance API
[13] https://github.com/erincatto/box3d/issues/155 — COM shift against recycling
[14] https://raw.githubusercontent.com/erincatto/box2d/main/src/physics_world.c — Box2D v3 main has the same recycling block
[15] https://raw.githubusercontent.com/dimforge/rapier/master/src/geometry/narrow_phase/pair_update.rs ; …/src/geometry/contact_pair.rs ; …/src/dynamics/integration_parameters.rs ; …/CHANGELOG.md — Rapier 0.35 recycling
[16] https://raw.githubusercontent.com/NVIDIA-Omniverse/PhysX/main/physx/source/geomutils/src/pcm/GuPersistentContactManifold.h / .cpp ; https://nvidia-omniverse.github.io/PhysX/physx/5.4.1/docs/AdvancedCollisionDetection.html — PCM thresholds, refresh, stacking caveat
[17] https://www.bepuentertainment.com/ ("Seeking the Tootbird", 2022-03-10) — Bepu v2's one-shot manifolds
[18] https://docs.rs/box3d-sys/latest/box3d_sys/constant.B3_CONTACT_RECYCLE_ANGULAR_DISTANCE.html — the constant's value
[19] Tree: `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md` §1–§9; `D:/wt/joltab/docs/physics/perf-campaign/levers/{L5-narrowphase/02-DESIGN-REV1.md, broadphase/04-DESIGN-REV2.md, L10-sleeping/04-DESIGN-REV2.md, L10-sleeping/01-RESEARCH.md, 00-RULINGS.md}`; `D:/wt/joltab/docs/physics/perf-campaign/{00-RULINGS.md, 04-PRACTICE-SURVEY.md}`; `D:/claude/BoykoEngine/docs/unification/UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md:197-211,351,364`
[20] Tree code: `D:/wt/joltab/crates/boyko_physics/src/{systems.rs, manifold.rs, narrowphase/box_box.rs, narrowphase/axis_cache.rs, solver/colored.rs, solver/soft_step.rs, solver/warm_start.rs, row_identity.rs, resources.rs, profiling.rs}`; tests `sleep_settles_box_piles.rs`, `support_loss_wakes_sleepers.rs`, `frozen_island_warm_start.rs`