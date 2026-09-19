# Research: L8, friction per manifold patch (two tangent rows plus a twist row at the patch centre)

How this was gathered:
- **Tree.** Read with Read and Grep only, against `D:/wt/joltab`. I have no shell, so I could not confirm the tree is at `47c5dabd`, and graphify did not run. Line numbers are from the working copy.
- **Lane state.** L11, L9 and L10 are designed but not built, and L8 is designed against them. So each current code path below is named twice: its file:line today, and the L11 structure that replaces it (`levers/L11-solve-setup/02-DESIGN-REV1.md`).
- **Web.** Every web fact was fetched on 2026-09-19. Figures marked "arith." are my calculations from P0 spans, counts and op counts. They are not measurements.

## Brief summary (TL;DR)
- **Jolt switched to this exact model in v5.6.0.** It came in PR #2039, merged 2026-05-31, commit `0f58921`, and closed issue #983 ("Box can rotate while sliding due to friction being applied asymmetrically") [1][2][3][7].
  - Each manifold has 2 linear rows and 1 angular row, applied at the average contact point.
  - The limits are `max_linear = μ·Σλn_i` and `max_angular = μ·Σ d_i·λn_i`, where d_i is the in-plane distance from point i to the friction point. The two linear rows share a circular clamp; the twist row has its own interval clamp.
  - Friction is solved **before** the normal rows. The twist row exists only when the manifold has more than one point.
  - The lambdas are stored per cached manifold (`Vector<2> mFrictionLambda`, `float mAngularFrictionLambda`) and are not re-projected [4][5].
  - Jolt's figure: "15 % faster, 40% less memory for Pyramid test" [9].
  - **P0 already measures v5.6.0 on this machine:** 0.614× v5.3.0 at W=1. That figure covers every change from 5.3 to 5.6, and v5.6.0's own manifold count is not receipted, so the friction share of it is not isolated (`ANALYSIS.md` §1, §0 item 4).
- **Box3D, Bepu and Rapier's default "Simplified" model use the same limits:** a circular linear clamp at μΣλn and an independent twist clamp at μΣd_iλn_i. They differ on four points:
  - the centre rule: Jolt uses a plain average, Box3D weights by separation with a smooth decay, Bepu weights by depth ≥ 0, Rapier weights by count;
  - the tangent solve: two 1D rows and a joint clamp (Jolt), or one 2×2 block (Box3D, Rapier);
  - the order: friction first (Jolt), or normal first (Box3D, Bepu, Rapier);
  - warm-start storage: raw lambdas (Jolt), or a world-space vector re-projected onto the new tangents (Box3D; Rapier stores a world vector too).
  PhysX uses a different shape: up to 2 **anchors** per patch, 2 axes per anchor, and the patch's total normal force shared among the anchors. Its torsional friction is an optional patch radius [10][13][14][15][17][20].
- **Box2D main and Box3D apply no friction in bias sweeps.**
  - Box2D main split the wide solve into `b2PushContacts_Wide` (biased normal rows, no friction) and `b2SolveContacts_Wide` (friction plus rolling resistance). Its scalar overflow path wraps friction in `if ( useBias == false )`.
  - Box3D: "No friction when applying bias" [10][11].
  - boyko applies friction in all 12 sweeps per step (4 biased, 8 relax: `colored.rs:3567-3616`; `resources.rs:452-453`). This would be a separate value change, beyond D2.
- **Stability facts:**
  - The pure-translation stick/slip threshold is tan θ = μ in both models, because the aggregate limit is the same.
  - Under combined sliding and spinning, independent linear and twist clamps allow more friction than Coulomb. This is my derivation from the limit-surface definition [23], not a published measurement.
  - The twist capacity μΣd_iλn_i equals the per-point model's capacity for the same points.
  - The friction centre jumps when the point count changes. Box3D smooths it for exactly this reason ("Needed to prevent spinning top drift" [10]). boyko has no speculative points, and its resting pile changes support point counts 1,836 times over A7-R1's window (`sleep_settles_box_piles.rs:107`).
- **Gain (arith.), J at W=1 with simd:**
  - Rows fall 39.9 % (ρ = 0.399).
  - My AVX2 op count gives about −39 % of kernel vector ops per cohort.
  - **The warm apply does not shrink by ρ.** The per-point normal apply stays. Applying each part separately costs about +24 %; merging the impulses per body per manifold saves about −32 %.
  - Net: −1.18 to −1.59 ms at W=1, which is −0.26 to −0.35 µs per manifold against today's 1.24.
  - At W=8 the gain is −0.02 to −0.59 ms. The wide-colour span there is `P(1)/8 + L(8)` with L(8) = 0.510 ms of dispatch and imbalance that L8 does not touch.
  - The plan's bound ρ·(t_colors + t_warm) is the upper end of these ranges.

## Approaches in state-of-the-art engines

### Jolt v5.6.0+ (per-manifold friction; v5.3.0 = per-point, the P0 headline)
- **Approach.** Per manifold: `mFrictionConstraint1/2` (`ContactConstraintPart`) and `mAngularFrictionConstraint` (`AngularFrictionConstraintPart`). Per point: `mNonPenetrationConstraint` plus `mDistanceToFrictionCenter` (`ContactConstraintManager.h`) [5].
- **Setup** (`CalculateFrictionConstraintProperties`) [4]:
  - `friction_point = Σ inWorldSpaceContacts[i] / N`.
  - `d_i = |delta − (delta·n)·n|`, with `delta = p_i − friction_point`.
  - `r1`/`r2` = `friction_point − COM` of each body: one shared world point.
  - The tangent rows use `r1`, `r2` along `t1`/`t2`. The angular row is set up only `if (mNumContactPoints > 1)`.
  - Tangents come from `GetTangents`: `t1 = n.GetNormalizedPerpendicular()`, `t2 = n × t1` [5].
- **Solve** (`SolveVelocityConstraint`) [4]:
  - `max_linear = μ·Σ λn_i` and `max_angular = μ·Σ d_i·λn_i`, both summed from the current total lambdas.
  - Comment: "First apply friction constraint (non-penetration is more important)".
  - The linear pair is clamped as `if (λ1² + λ2² > max²) scale`, a circle.
  - The angular row is solved with bounds `[−max_angular, +max_angular]`.
  - The normal rows are solved afterwards.
- **Twist effective mass:** `1 / (n·(I1⁻¹ + I2⁻¹)·n)`, and the row deactivates below `FLT_MIN` [6].
- **Warm start:** friction lambdas load from `old_manifold` unconditionally once the manifold is found, with no re-projection when the tangent basis changes. `StoreAppliedImpulses` writes `mFrictionLambda[0..1]` and `mAngularFrictionLambda` [4].
- **Trade-offs (the author's claim):** 15 % faster and 40 % less memory on Pyramid. The v5.6.0 notes also say "Up to 40% performance improvement (scene dependent) and up to 70% memory reduction (scene dependent)"; I could not attribute that line to one change [9].
- **Known issue near this code.** #2121, filed against v5.3.0: friction applied at the midpoint `0.5·(p1 + p2)` adds `separation/2` to the lever arm on speculative contacts. The reported rolling-velocity error is +1.52 %. Fix PR #2123 derives a lever arm per body; it was open when fetched [8].
- **Sources:** [1]–[9].

### Box3D (Erin Catto, released 2026-06)
- **Approach:** central friction (a 2D block), a central twist row and rolling resistance per manifold. Normal rows use fixed per-point anchors [10].
- **Centre:** `weight = clamp(2 − s·invTau, B3_MIN_FRICTION_WEIGHT, 1)` with `invTau = 1 / B3_SPECULATIVE_DISTANCE`. `centerA` and `centerB` are separate weighted averages of `rA` and `rB`. Comment: "C0 friction center decay. Needed to prevent spinning top drift… Closer points may begin to touch on and off, so the friction center needs to move smoothly." The constant values were not retrieved (`constants.h` returned 404).
- **Masses:**
  - `tangentMass = inverse(K)`, a 2×2 matrix with off-diagonal coupling `rtA1·iA·rtA2 + rtB1·iB·rtB2`.
  - `twistMass = 1 / (n·(iA + iB)·n)`.
  - The lever per point is `leverArm = |rA − centerA|`.
  - Everything is computed once in prepare.
- **Solve order:** the normal points first. Then `if ( useBias == true ) continue;` ("No friction when applying bias"). Then the twist row (`maxImpulse = friction·Σ leverArm·λn`), then rolling resistance (`rollingResistance·Σλn`, a 3-vector clamp), then central friction (`maxImpulse = friction·totalNormalImpulse`, a circular clamp).
- **Warm start:** `frictionImpulse.x = warmStartScale·dot(manifold->frictionImpulse, tangent1)`, and the same for `.y`. The manifold keeps a **world-space** friction vector, so a basis change is re-projected.
- **Source:** [10].

### Box2D v2.4 and v3 (2D; for contrast)
- **v2.4:** friction is per point. "Solve tangent constraints first because non-penetration is more important than friction"; `maxFriction = friction·vcp->normalImpulse` [12].
- **v3 main:**
  - Friction is still per point: `maxFriction = friction·cp->normalImpulse`; the wide path uses `normalImpulse1`/`2`.
  - It runs only in the non-bias pass (`b2SolveContacts_Wide`, and `if ( useBias == false )` on the scalar overflow path). `b2PushContacts_Wide` has no friction.
  - Rolling resistance uses `Σ normalImpulse` [11].
- In 2D there is no twist, so Box2D gives no evidence on patch friction itself. It is evidence on the friction-in-bias question.

### Bepu v2 (Contact4, convex)
- **Accumulators:** `Contact4AccumulatedImpulses { Vector2Wide Tangent; Penetration0..3; Twist }` [13].
- **Centre:** `FrictionHelpers.ComputeFrictionCenter(OffsetA_i, Depth_i)`. Contacts with depth ≥ 0 are weighted, with equal weights if all depths are negative. `offsetToManifoldCenterB = centerA − OffsetB`.
- **Solve order:** 4× `PenetrationLimit.Solve`, then `maximumTangentImpulse = μ·ΣPenetration_i` and `TangentFriction.Solve`, then `maximumTwistImpulse = μ·Σ Penetration_i·|centerA − OffsetA_i|` and `TwistFriction.Solve`. The centre is recomputed every solve call.
- **Warm start:** the tangent row at the centre, the 4 penetration rows, then twist.
- **Source:** [13].

### Rapier
- **Two models.** Coulomb uses friction per point. "Simplified", the 3D default since v0.29.0 (5 Sept 2025, changelog [15]), "emulates coulomb with only one tangent constraint + one twist constraint per manifold" [14].
  - The Dimforge review cites "25% speedup on scenes involving many contacts (like large stacks)" and names v0.32; the changelog says v0.29.0 [16]. The discrepancy is unresolved.
- **Simplified model:**
  - The centre is a count-weighted average (`weight = 1/count`).
  - `tangent_limit = μ·Σ normal impulses`; `twist_limit = μ·Σ impulse_i·dist_i`.
  - The tangent part is one 2D constraint (`ContactConstraintTangentPartSlim`).
  - Order: normal, then twist ("if multi-contact"), then tangent, so that "the twist solve changes the angular velocities the central-friction constraint reads".
  - Warm data sits on the manifold's per-point `ContactData`: `warmstart_impulse`, `warmstart_tangent_world` (world vector), `warmstart_twist_impulse` [14].
- **Configuration:** the model is selectable through `IntegrationParameters::friction_model` [15].

### PhysX 5 (patch friction with anchors)
- **ePATCH:** "Up to two contact points lying in the contact patch area are selected as friction anchors", chosen to maximise their spread. Each anchor gets one 1D row along each of two perpendicular axes, so there are at most 4 friction rows per patch [17].
- **TGS vs PGS:** "The TGS solver type works with the combined impulse along the two axes and as such avoids this potential problem". The problem with PGS: "This can lead to asymmetries when transitioning from dynamic to static friction and vice versa in certain edge cases". TGS applies friction on all iterations [17].
- **Normal-force sharing:** "Without this flag, PhysX's friction model is stronger than analytical models" (`eIMPROVED_PATCH_FRICTION`) [19]. Unity's description: the flag "distributes the normal force between the friction anchors so that the total amount of friction applied does not exceed the analytical results" [20]. The flag was later removed, and PhysX now always behaves as if it were set (`04-PRACTICE-SURVEY.md:60`).
- **Strong friction:** "the strong friction feature remembers the 'friction error' between simulation steps" (`eDISABLE_STRONG_FRICTION`) [18].
- **Torsion:** friction around the normal comes only from the anchor lever arms, unless the optional `torsionalPatchRadius` / `minTorsionalPatchRadius` are set. Those "approximate rotational friction introduced by the compression of contacting surfaces" (search summary of the PxShape docs [21]).

## Comparative table

| Aspect | Jolt 5.3 | Jolt 5.6 | Box3D | Bepu | Rapier Simplified | PhysX ePATCH | boyko (tree) |
|---|---|---|---|---|---|---|---|
| Friction rows, 4-point manifold | 8 (2/point) | 2 + 1 twist | 2D block + twist (+rolling) | 2D + twist | 2D + twist | ≤2 anchors × 2 | 8 (2/point, coupled cone) |
| Centre | — | plain mean of world points | separation-weighted, C0 decay; A/B separate | depth ≥ 0 weighted | 1/count mean | anchors, spread max | — |
| Linear limit | μλn_i per point | μΣλn | μΣλn | μΣλn | μΣλn | Σ normal shared among anchors | μλn_i, circle |
| Twist limit | implicit (points) | μΣd_iλn_i; none if N = 1 | μΣleverArm·λn | μΣλn_i·d_i | μΣλn_i·d_i | anchor levers (+ optional radius) | implicit (points) |
| Tangent solve | two 1D + circle | two 1D + circle | 2×2 inverse | 2D | 2D slim part | 1D per axis (TGS combines) | two 1D + circle |
| Order | friction first | friction first | normal, twist, rolling, friction | normal, tangent, twist | normal, twist, tangent | — | per point: normal then its friction |
| Friction in bias sweeps | n/a | n/a | **no** | n/a | n/a | TGS: all iterations | **yes (12 of 12)** |
| Warm storage | per point | per manifold, raw λ | per manifold, world vector | in-place constraint | world vector | anchors persistent | per point, raw λ |
| Masses | once per step | once per step | once per step | on the fly | per step | — | every sweep (3 per point) |
| Speed claim | — | −15 % Pyramid | — | — | −25 % large stacks | — | — |

## Key algorithms and techniques
- **Patch limits.** Every twist-row engine uses the same pair: `|λ_t| ≤ μΣλn` as a circle, and `|λ_tw| ≤ μΣd_iλn_i` as an interval [4][10][13][14]. Summed over the same points, the linear limit equals the per-point model's aggregate capacity. The twist limit equals the per-point model's maximum torsion when every corner force is perpendicular to its radius (arith.).
- **Twist stop, arith. for a J box.**
  - Box: side 2 m, mass 8 kg (`BOX_INV_MASS = 0.125`, `BOX_INV_INERTIA = 0.1875`), μ = 0.2, 4 equally loaded corners, d = √2 m.
  - Torque: μ·m·g·d = 0.2·8·9.81·1.414 = 22.2 N·m. I_n = 5.333 kg·m², so α = 4.16 rad/s².
  - Starting from ω₀ = 2 rad/s, the box stops in 0.48 s, about 29 steps at 60 Hz.
  - The per-point model predicts the same number. For a continuous uniform square the mean distance is 0.383·side against the corners' 0.707·side, so both discrete models give 1.85× the continuum torque.
- **A sphere has no twist.** d = 0 in every model, so a sphere spinning about the normal never stops by friction in either one. Only PhysX's torsional radius [21] or Gazebo's `T = 3π/16·a·μ·N` model [22] address that.
- **A single-point manifold reduces exactly to the per-point model** if the twist row is skipped for count = 1 (Jolt's rule) and friction is solved after that point's normal row. My derivation: `centre = Σanchor/1` is exact in IEEE, and `μ·(0 + λn) = μ·λn`. This is the one subset that could stay bit-identical, so it needs a proof.
- **Centre smoothing.** Box3D's C0 decay needs a separation signal on points about to touch. boyko keeps only face points with `separation <= 0`; edge points are unfiltered, at most 1.96e-6 m (`OPEN-QUESTIONS.md:121-122`, `:241-242`). So boyko's centre moves in discrete jumps.
- **Warm impulse at a moved centre.** A per-manifold friction impulse carried across a point-count change is applied at the new centre. Jolt and Box3D do this. Today boyko drops a per-point tangent impulse whenever that point's feature id changes.
- **Friction only in relax sweeps** (Box2D main, Box3D) [10][11]. This is a separate lever that removes a further 4/12 of friction work, with its own value change.

## Pitfalls and mistakes
- **Summing the limit per anchor instead of once per patch.** PhysX's own docs: without the correction, the model "is stronger than analytical models" [19][20].
- **Using one shared lever point for separated contacts.** This is the error reported against Jolt in #2121 [8]. Box3D keeps `centerA` and `centerB` separate; Bepu expresses the same world centre relative to B.
- **Raw lambdas across a tangent-basis flip.**
  - boyko's `tangent_basis` switches seed at `|n.z| ≥ 0.999`, about 2.56° from ±z (`contact.rs:60-64`). Jolt's perpendicular switches on `|x|` against `|y|` [5].
  - Box3D and Rapier store a world vector to avoid this [10][14]. Today's per-point store has the same exposure (`colored.rs:3243`).
- **Independent clamps over-resist a body that slides and spins at once.** The set they allow is a cylinder in (F_t, M_n) space, which contains Coulomb's convex limit surface [23]. This is derived, not measured in any source.
- **Friction-first against normal-first.** Jolt reads the previous iteration's Σλn. boyko today reads the λn just written (`colored.rs:2127-2131`; kernel `:2538-2539`).
- **Asymmetric per-point application makes a sliding box yaw.** This is Jolt #983, the stated reason for the v5.6 change [3][7].
- **A7's residual drift (hypothesis only).** The drift RATE has a fixed (−x, −z) bias, is unexplained, and its named deciding test is "mirror the scene in x" (`OPEN-QUESTIONS.md:220-230`). Per-point Gauss-Seidel order is a candidate world-anchored bias of #983's kind. Nothing in the tree links the two.
- **Denominators move.** Because L8 changes values, contact sets and manifold counts change (R against R-ref: 6,662 against 5,949 manifolds, `ANALYSIS.md` §5). Per-contact figures before and after need each run's own count.

## Relevant academic works
- Goyal, Ruina, Papadopoulos, "Planar sliding with dry friction Part 1. Limit surface and moment function", *Wear* 143(2):307–330, 1991. The limit surface bounds every friction force and moment an interface can sustain [23]; only the abstract was read, via search.
- Catto, "Solver2D" (2024), author's opinion: TGS_Sticky "achieves stable stacking with no warm starting of the impulses. The only warm starting is the friction anchors. This shows how important strong friction is to stable stacking." [24]
- OSRF / Gazebo torsional friction: `T = 3π/16·a·μ·N` with `a = √(R·d)`, for a Hertzian patch [22].

## boyko tree facts this lever touches (today → L11's name)
- **Math:**
  - `tangent_basis`: `contact.rs:57-69`;
  - `effective_mass` is the same function for n, t1, t2: `:131-148`;
  - the manifold holds 4 points with `feature_id`: `manifold.rs:62-104`; `MAX_CONTACT_POINTS = 4` at `math.rs:34`.
- **Colored build:**
  - `push_manifold_points` at `colored.rs:1737-1817`: tangents `:1755`, `μ = max(μa, μb)` `:1760`, per-point seed `(λn, λt1, λt2)` `:1786-1797`;
  - `point_keys` / `pack(a:24|b:24|fid:16)` at `:1830-1843` and `warm_start.rs:148-159`;
  - `WarmEntry {key, normal_impulse, tangent_impulse:[2]}`, 24 B: `warm_start.rs:93-100`.
  - Under L11 these become P-a/P-b/fill, `CohortHead {n, t1, friction, …}`, `RankBlock {ra, rb, sep, ni, ti1, ti2}`, and a 64 B `WarmRecord {n[4], t1[4], t2[4], fid[4], count}`. L11 states: "the record's value fields become λn[4], λt1, λt2, twist; per-manifold friction moves into the head" (`02-DESIGN-REV1.md:161-196`, `:300`).
- **Solve:**
  - warm apply per point (`n·λn + t1·λt1 + t2·λt2`): `colored.rs:1881-1918`; L11 C3 widens it to `warm_apply_avx2`;
  - scalar oracle friction: `:2127-2172`;
  - AVX2 kernel: per-rank gather of 20 fields `:2441-2468`, normal `:2484-2535`, friction `:2537-2603`, per-rank impulse scatter `:2611-2621`;
  - restitution touches the normal row only: `:3141-3197`;
  - the store inserts `[λt1, λt2]` per point `:3238-3245`; the B1 carry copies them per point `:3282-3319`;
  - substep loop: gravity, warm, biased sweep, integrate + `refresh_inertia`, 2 relax sweeps (`:3547-3617`); defaults 4 substeps and 2 relax (`resources.rs:452-453`).
- **Reference `SoftStepSolver`:**
  - `PointConstraint.tangent_impulse1/2`: `soft_step.rs:109-130`;
  - build `:380`, `:450-460`; warm apply `:498-525`; store `:573`; solve `:606-732` (friction `:678-729`).
  - Documented as the "byte-untouched" reference oracle (`colored.rs:12-14`). It is paired with the colored solver under the same bounds in `colored_acceptance_o5.rs:5-15` and `sdf_collision.rs:22-25`, and is P0's R-ref row.
- **Graph, waves and census are unaffected.** Colouring is per manifold, so colours, waves (109.32 per step) and the scope/chunk pins do not change.
- **Op counts** (my count, `simd.rs:546-727`):
  - per rank: normal ≈ 209 vector ops, friction ≈ 311 (60 %);
  - a per-lane patch step (two centre masses, a twist mass, twist row, two centre applies, two angular applies) ≈ 418 per cohort;
  - with 4.06 ranks per cohort (2,320 blocks over about 571 cohorts): 2,111 ops per cohort become 1,279, **−39 %**.
- **Rows:** 3P = 50,663 becomes P + 3M = 30,460 on J; ρ = 0.399 (J), 0.376 (R).

## Red-set inventory (pins that consume friction values) and their rules
- `bodytype_determinism_golden.rs:81` `GOLDEN` and `golden_scalar_colored_equals_golden`. The file's contract forbids a change only "in the EnableTag refactor" (`:25-33`), so a value lever needs a re-pin ruling. Scalar = SIMD must still hold.
- `sleep_settles_box_piles.rs`:
  - A7-R1: bound `CREEP_BOUND_M = 0.01` (`:313`), reading 0.7180 mm. Its bands were pre-registered for S5 (`:90-94`); L8 has no bands yet.
  - A7-R2: freeze step 248.
  - G8: freeze step 185.
  - G2/G7 event budgets 3 and 6, settle limits 486 and 1016. These are re-sized only from a fresh `flicker_redraw_distribution` run and are never raised (`:33-37`).
  - the "6671 manifolds / 22 975 points" doc line (`:125`).
  - `simd_solve_on_off_bit_identical` is a gate, not a pin.
- `frozen_island_warm_start.rs`: per-point `point_hits` (`:22`, `:67`) and the bitwise wake-against-twin test. The carry must therefore carry the per-manifold friction record.
- Runner `--expect-pose` fixtures (`jolt_parity_pyramid.rs:140`, `:1326-1334`), P0's J-family pose `0x32d5e235342b4143`, and the H8 manifold count 4,519.3.
- O7 differential suite: `simd_solve_bits_match_scalar` (`colored_tests.rs:1454`), `cone_probe` (`:1755`), 1c (`:1915`), 1d (`:2009`), `cohort_shape_proptest` (`:2101`). These are relative (scalar against SIMD) and are ported, not re-pinned.
- **Inequality gates that should stay green:**
  - incline 0.3 rad with μ = 1.0 and μ = 0.05 (`colored_acceptance_o5.rs:512-524`);
  - sphere cone (`:565-580`);
  - `box_on_sdf_incline_on_the_default_solver` (`sdf_collision.rs:471-504`).
  None of them tests near the threshold.
- **Relative, must stay green:** `default_world_pyramid_determinism`, `default_world_worker_invariance`, `default_world_colored_simd`.
- **If the oracle adopts the patch model:** `softstep.rs` (friction tests `:741-831`, `:1514`), the reference arm of `sdf_collision`, and P0's R-ref row also move.
- **Not checked:** render goldens fed by physics, and soft-rigid coupling in `soft_body_sp1`/`sp2`.

## Gate material (from sources and the tree)
- **Stick/slip threshold at tan θ = μ.**
  - Today's incline gates have 3.2× and 6× margins (`colored_acceptance_o5.rs:517-523`), so they cannot detect a shifted threshold.
  - No measurement of how sharp the per-point model's threshold is exists in the tree.
- **Twist stop.** Analytic stop time (arith. above: 0.48 s for ω₀ = 2 rad/s on a J box). The per-point oracle predicts the same.
- **Yaw while sliding** (Jolt #983 [7]). A box sliding with ω₀ = 0 should keep n·ω ≈ 0. Whether today's per-point path fails it is **unmeasured**; if it does, this gate is red-first.
- **Rest and sleep.** A7-R1 bound, A7-R2 freeze, G2/G7 budgets under their file's triage, `support_loss_wakes_sleepers`, `frozen_island_warm_start`, and box-pile freeze steps before and after.

## Gain derived from P0 (arith.; J, `simd_solve` on, J-B spans)

| row | colours Δ | warm Δ (per-part / merged) | fill Δ | total | per manifold |
|---|---|---|---|---|---|
| W=1, today's base | −0.39 × 3.616 = −1.41 ms | +0.15 / −0.20 | +0.02…0.08 | **−1.18 … −1.59 ms** (−11 … −14 % of the derived 11.04–11.14) | −0.26 … −0.35 µs (1.24 → 0.89–0.98) |
| W=1, after L11 (colours 2.68–3.22, warm 0.15–0.30) | −1.06 … −1.27 | +0.04…0.07 / −0.05…−0.10 | +0.02…0.08 | −0.91 … −1.35 ms | −0.20 … −0.30 µs |
| W=8 (wide 0.957 = P(1)/8 0.447 + L(8) 0.510; warm serial 0.624) | −0.17 … −0.39 | +0.15 / −0.20 | +0.02…0.08 | **−0.02 … −0.59 ms** (0 … −11 % of 5.5–5.7) | — |
| cfg-A W=8 (P(1)/8 1.526, L(8) 0.715) | −0.60 … −0.87 | as above | | | |

- **Claimability at K=6.** The SE bars are 2.0 % (J, W=1) and 0.75 % (J, W=8). W=1 clears. At W=8 the lower end does not.
- **Against Jolt.** v5.3.0's solve is 1.30–1.49 µs per manifold. There is no per-stage v5.6.0 profile. If Jolt's −15 % landed entirely in its solve at its own manifold count, its solve would be about 1.02–1.21 µs per manifold. That is arithmetic under an assumption, not a measurement.

## Applicability to boyko-engine
- **Fits without adaptation:**
  - Lane = manifold is already boyko's AVX2 cohort shape (`colored.rs:2176-2187`) and Box3D's [10]. The friction step becomes one per lane, with no rank padding.
  - The per-manifold warm record fits L11's `WarmRecord`: 37 B of values within 64 B.
  - There are 4 points at most, the same as every reference engine.
- **Needs adaptation:**
  - Masses: the references compute them once per step, but boyko recomputes per sweep because inertia is refreshed every substep (L11 D8). The twist and centre masses join L12's per-epoch cache.
  - Box3D's centre decay depends on speculative points, which boyko does not keep.
  - Merging the warm apply per body changes the float order; the scalar and SIMD paths must still match each other.
- **Does not fit the binding rules as-is:**
  - raw lambdas across a basis flip, if the gates demand continuity;
  - PhysX's persistent anchors and strong friction, which would be new positional friction state.
  Both are value questions.

## Open questions for the architect
1. **The oracle.** Does SoftStepSolver adopt the patch model, or stay per-point as the physical reference?
   - Precedents: Jolt replaced its model outright; Rapier keeps both behind `friction_model`.
   - If it stays per-point, the paired same-bound gates are the comparison.
2. **Order:** friction-first (Jolt) or normal-first (Box3D, Bepu, Rapier, and boyko today).
3. **Twist for count = 1:** skip it (Jolt), which keeps sphere contacts bit-identical, or solve it at zero limit, which can flip −0.0 to +0.0.
4. **Tangent solve:** two 1D rows plus a circle clamp, or a 2×2 block.
5. **Centre rule:** a plain mean, or a weighted one with no separation signal available.
6. **Warm storage:** raw `(λt1, λt2)` or a world vector; per-part warm apply or a merge per body.
7. **Friction in bias sweeps:** Box2D and Box3D skip it. Is that inside D2, or its own value lever?
8. **Pre-registered A7-R1 bands for L8**, like S5's, and what a rise in D_max means.
9. **Measurements before claiming:** the kernel's normal/friction split, and v5.6.0's manifold count and stage shares, for like-for-like per-contact parity (H11 keeps v5.3.0 as the headline).
10. **The re-pin rule for `GOLDEN`** under a value lever.

## Sources
[1] https://jrouwe.github.io/JoltPhysics/md__docs_2_a_p_i_changes.html — the 20260531 entry: the friction model changed; `EstimateCollisionResponse` now returns 2 linear + 1 angular friction impulse.
[2] https://github.com/jrouwe/JoltPhysics/commit/0f58921ed9b42f3296d37163d7e1b69903175741 — the commit that introduced the model.
[3] https://github.com/jrouwe/JoltPhysics/pull/2039 — "New friction model"; merged 2026-05-31; fixes #983.
[4] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.cpp — friction point, limits, order, warm start and store.
[5] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.h — structs and `GetTangents`.
[6] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ConstraintPart/AngularFrictionConstraintPart.h — the twist mass.
[7] https://github.com/jrouwe/JoltPhysics/issues/983 — a box rotates while sliding (per-point asymmetry).
[8] https://github.com/jrouwe/JoltPhysics/issues/2121 — the midpoint lever-arm error.
[9] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Docs/ReleaseNotes.md — v5.6.0 performance lines.
[10] https://raw.githubusercontent.com/erincatto/box3d/main/src/contact_solver.c — central friction, twist, C0 decay, no friction under bias, world-vector warm start.
[11] https://raw.githubusercontent.com/erincatto/box2d/main/src/contact_solver.c — Push/Solve split; per-point friction only in the non-bias pass.
[12] https://raw.githubusercontent.com/erincatto/box2d/v2.4.1/src/dynamics/b2_contact_solver.cpp — v2.4 friction first, per point.
[13] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Constraints/Contact/ContactConvexTypes.cs — Contact4 centre, limits, order.
[14] https://raw.githubusercontent.com/dimforge/rapier/master/src/dynamics/solver/contact_constraint/contact_with_twist_friction.rs — Rapier's Simplified model.
[15] https://raw.githubusercontent.com/dimforge/rapier/master/CHANGELOG.md — v0.29.0 made Simplified the 3D default.
[16] https://dimforge.com/blog/2026/01/09/the-year-2025-in-dimforge/ — the "25% speedup" claim (names v0.32).
[17] https://nvidia-omniverse.github.io/PhysX/physx/5.4.0/_api_build/struct_px_friction_type.html — ePATCH anchors; TGS against PGS.
[18] https://nvidia-omniverse.github.io/PhysX/physx/5.3.1/_api_build/struct_px_material_flag.html — improved patch friction and strong friction flags.
[19] https://physics-playground.github.io/PhysX5/physx/5.3.1/docs/RigidBodyDynamics.html — "stronger than analytical models" (a mirror of the 5.3.1 docs).
[20] https://docs.unity3d.com/ScriptReference/Physics-improvedPatchFriction.html — normal force distributed among the anchors.
[21] https://nvidia-omniverse.github.io/PhysX/physx/5.4.0/_api_build/class_px_shape.html — torsional patch radius (search summary; not fetched).
[22] https://classic.gazebosim.org/tutorials?tut=torsional_friction&cat=physics — the torsional friction formula.
[23] https://www.sciencedirect.com/science/article/pii/0043164891901043 — Goyal, Ruina, Papadopoulos 1991 (abstract via search).
[24] https://box2d.org/posts/2024/02/solver2d/ — Catto, TGS_Sticky and friction anchors.

**Tree files read:**
- `D:/wt/joltab/docs/physics/perf-campaign/levers/00-RULINGS.md`
- `D:/wt/joltab/docs/physics/perf-campaign/00-RULINGS.md`
- `D:/wt/joltab/docs/physics/perf-campaign/01-PLAN-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/03-TREE-REPORT.md`
- `D:/wt/joltab/docs/physics/perf-campaign/04-PRACTICE-SURVEY.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/01-RESEARCH.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/02-DESIGN-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/03-REVIEW-OF-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`
- `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`
- `D:/wt/joltab/docs/OPEN-QUESTIONS.md`
- `D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/soft_step.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/warm_start.rs`
- `D:/wt/joltab/crates/boyko_physics/src/manifold.rs`
- `D:/wt/joltab/crates/boyko_physics/src/resources.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/colored_acceptance_o5.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/softstep.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/sdf_collision.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/sleep_settles_box_piles.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/bodytype_determinism_golden.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/frozen_island_warm_start.rs`
- `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs`
