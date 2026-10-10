# Advanced physics — the survey

> Status: architect's research record, 2026-09-10, **rev 2 — after one critique round; the log is
> §17**. Companion of
> [`ADVANCED-PHYSICS-DESIGN-SPACE.md`](ADVANCED-PHYSICS-DESIGN-SPACE.md), which holds the design
> and the ladder for THIS engine. Every claim carries a tag: **[S]** a source file that was opened,
> **[D]** vendor or project documentation, **[P]** a paper, **[B]** a blog or secondary source,
> **[T]** this tree (a `file:line` re-opened for this record). An untagged claim is
> **UNVERIFIED** and is listed as such in §16. Every `file:line` below was re-opened at
> `D:/wt/ecsnative` (branch `feat/ecs-native-storage`, HEAD `46c8e489`; rev 2 re-verified at
> `05fcbd1d`, which adds one docs-only commit and moves no code line) for the physics crate, and
> at `D:/claude/BoykoEngine` (branch `feat/multi-paradigm-render`, HEAD `128233be`) for the docs,
> unless the path says otherwise. No timing was taken for this record; every number quoted is a
> citation of a number measured elsewhere, with its source. Written in the register of
> [`docs/animation/ANIMATION-DESIGN-SPACE.md`](../animation/ANIMATION-DESIGN-SPACE.md).

## 0. What the tree holds today (the ground every rung stands on)

The physics crate ships ONE solver family in two implementations and exactly one constraint TYPE.

| Piece | Where [T] | State |
|---|---|---|
| Hot body column `RigidBody { position, linear_velocity, rotation, angular_velocity }` | `crates/boyko_physics/src/components.rs:29` | shipped; `#[repr(C)]` |
| Cold `RigidBodyMass { inv_inertia: Mat3 (WORLD), inv_mass, restitution, friction }` | `components.rs:85` | shipped |
| `Simulated` / `Kinematic` as `#[component(storage = "bitset")]` EnableTag bits | `components.rs:62`, `:74` | shipped; **kinematic MOTION is "an intentional deferral, not built yet"** (`components.rs:66-73`) — and ⚠ **the `Kinematic` bit is UNREAD by the solver.** The gathered `BodyState.kinematic: bool` is written at `systems.rs:244-251` / `resources.rs:3470-3488` and consumed by NOTHING: a grep for `kinematic` over `crates/boyko_physics/src` returns the field, the gather, `scene_sync.rs`'s doc prose and test constructors, full stop. The one-sided contact response people attribute to the bit is a property of `inv_mass == 0` (`BodyEffective::apply_impulse` is a branchless no-op there, `contact.rs:81-131`; `effective_mass` contributes zero), and `is_dynamic_row(inv_mass)` is the sole colouring/write predicate (`contact.rs:20-40`). A body with the bit and `inv_mass != 0` is coloured and written as **dynamic** today (design C-NB4a/C-NB9) |
| `ColliderShape::{Sphere, Box}` — the COMPLETE enum | `components.rs:127-145` | no capsule, no hull, no mesh |
| `Sensor` ZST marker; `Contact` gameplay component (type exists, no producer) | `components.rs:191`, `:202`; `resources.rs:3630-3640` | the row→entity map is deliberately absent |
| `Manifold` = 152 B POD, ≤4 `ContactPoint { anchor_a, anchor_b, separation, feature_id }`, `SDF_SENTINEL = BodyIndex(u32::MAX)` | `manifold.rs:52`, `:61-106`, `:131-132` | the one constraint currency |
| Reference `SoftStepSolver` (TGS-Soft): per substep gravity → warm-start apply → `solve_velocities(bias=true)` → position integrate → `refresh_inertia` → relax ×N; restitution ONCE after the loop; store+swap warm table | `solver/soft_step.rs:896-941`, `:946-952`, `:475-496` | byte-untouched 0%-gate reference |
| `SoftCoefficients::new(hertz, ζ, h)` = Box2D `b2MakeSoft` verbatim (`ω=2πf; a1=2ζ+ωh; a2=hω·a1; a3=1/(1+a2)`) | `soft_step.rs:808-819` | shared by both solvers (O2) |
| `MAX_BIAS_VELOCITY = 4.0`, `RESTITUTION_THRESHOLD = 1.0` | `soft_step.rs:81`, `:97` | `pub(crate)`, one source |
| The normal solve's bias: `bias = (bias_rate · separation).max(-MAX_BIAS_VELOCITY)` when `bias_active`, else `0` — **no `separation > 0` regime** | `soft_step.rs:569-575` | see §6 — this is why narrowphase never emits a positive separation |
| Warm start: double-buffered open-addressed `WarmStartTable` keyed `pack(body_a, body_b, feature_id)`, rebuilt each frame, no tombstones, dense-row keyed | `solver/warm_start.rs:1-60`, `:133`, `:167`, `:207` | joints do NOT fit this lifetime model (§2.6) |
| `is_dynamic_row(inv_mass) = inv_mass != 0` — THE dynamic predicate shared by coloring and the `*_movable` write guard | `solver/contact.rs:36-38` | load-bearing for MT soundness |
| `BodyEffective::apply_impulse` branchless for statics (`inv_mass == 0`, `inv_inertia == ZERO`) | `solver/contact.rs:81-131` | |
| `ConstraintGraph` (O4): union-find islands over DYN-DYN manifold edges only ("Box2D's ground rule"), greedy first-fit coloring **in manifold order** over a per-color `u64` body bitset, CSR `color_start`/`color_contacts` ascending within a color; **every array is manifold-keyed** | `resources.rs:2399-2490`, `:2625-2635`, `:2703-2713`, `:2850-2960` | there is no generic "constraint edge" |
| `LARGE_ISLAND_CONSTRAINTS` — RETIRED as the dispatch gate ("a guard that compares numbers across units is not a guard") | `resources.rs:2346-2397` | the gate is now `widest_color_slots() >= 256` (`colored.rs:3218-3236`) |
| `ColoredSoftStepSolver` (O5–O8): 31 SoA `ScratchColumn`s (`ContactColumns`), colors swept `0..n` sequentially (Gauss-Seidel ACROSS colors), each color dispatched as ONE `pool.scope` wave of `W × CHUNKS_PER_WORKER` chunks cut on whole manifold-GROUP boundaries | `solver/colored.rs:858-925`, `:2544-2600`, `:2925-2945` | run-to-run + `{1, N}`-worker bit-identical; value DIFFERS from the reference by design (`colored.rs:20-45`) |
| `MIN_PARALLEL_SLOTS_PER_COLOR = 256`, `CHUNKS_PER_WORKER = 6`, `MIN_SLOTS_PER_CHUNK = 64` (the 32/64/128/256 sweep table: 256 breaks the wide-colour gate), `COHORT = 8` | `colored.rs:227`, `:256`, `:270-309`, `:319` | "no single integer is right for both" scenes |
| O7 AVX2 cohorts: lane = one manifold-GROUP, cohort = 8 body-disjoint groups of one colour, gather-once / rank-loop / scatter-once, bit-identical to scalar | `colored.rs:2012-2064` | `simd_solve` ships OFF (`resources.rs:449`) |
| `ColorSolvePtrs` `Send + Sync` SAFETY: per-element `row_ptr` only, never a whole-buffer reborrow (the SP4 fix) | `colored.rs:348-366` | |
| O8 `IslandSleep`: per-ROW latch, per-frame derived island freeze, `max(|v|²+|ω|²)` exact; **four `std::Vec` fields** (`asleep`, `below_count`, `frozen_islands`, `energy`) | `resources.rs:3063-3096`, `:3133`, `:3232` | the un-migrated "step 6" (owner's call) |
| Broadphase: default `AllPairs` O(n²) (0%-gate), opt-in uniform-grid CSR with an oversized list; O3 parallel emit; P3 `Auto` policy constants labelled needs-calibration (measured ~6–15× too low, latent because `Manual` is the default) | `resources.rs:145-160`, `systems.rs:280-355`; KE16-RESULTS `:1287-1290` | |
| Narrowphase: sphere-sphere inline, sphere-box, box-box (15-axis SAT + clip + ≤4-point reduction + reference-axis hysteresis, feature-id classes disjoint by high bits) | `systems.rs:357-455`, `narrowphase/mod.rs:1-45` | nothing is swept; no positive-separation manifold is ever emitted |
| SDF narrowphase: sphere (centre sample) and box (8 corners; scalar oracle or the O9 AVX2 `sdf_edit_list_x8`, whose arm is KNOWN-DIVERGENT on ±0 and ships OFF) against the analytic ≤16-edit list, C1 sentinel body B | `systems.rs:598-660`, `resources.rs:85-121`, `:455-466`, `sdf_query.rs:1-131` | the physics knows the SDF ONLY as `SdfField(SdfEditField)`; the brick atlas is never sampled by physics |
| Pipeline order and the two invariants: **IM-1** "no structural change between gather and apply" (`physics_apply`'s `debug_assert!`), **IM-2** deterministic spawn order | `systems.rs:1-56`, `:1127-1170` | |
| Gather: `Query<(&RigidBody, &RigidBodyMass, &Collider, Option<&Sensor>, IsEnabled<Simulated>, IsEnabled<Kinematic>)>` in archetype-row order → row `i` == `BodyIndex(i)` (Encoding A) | `systems.rs:201-278` | no `EntityId` per row is gathered |
| Scene sync: `Transform → RigidBody` only for `!simulated && !is_dynamic_row(inv_mass)`; simulated dynamics must be hierarchy ROOTS | `scene_sync.rs:82-93`, `:192-212` | |
| Scratch-id band: 128 reserved ids, cohorts as contiguous runs ≤ `POOL_STAGGER_LINES` (compile-time stagger-distinctness proof), **38 free ids (421..384)** | `scratch_ids.rs:65-71`, `:126-200`, `:541-550`; memory `project-ecs-native-physics-lane` | a new cohort must fit or move the floor |
| Soft body (O11 SP1–SP4): XPBD distance + tet volume, same-body particle self-collision (Teschner hash, `next_pow2(2n)`), one-way SDF collide, two-way soft↔rigid coupling (deepest contact per particle, tie-break ascending `BodyIndex`, reaction applied after `physics_apply`), colored particle graph arity {2, 4} | `soft/mod.rs:1-46`, `soft/solver.rs:1-20`, `:41-67`, `:121-160`, `soft/self_collision.rs:1-35`, `:46-66`, `soft/coupling.rs:1-27`, `soft/colored.rs:86-137`, `:232-306` | **`SoftBody` holds 30 `pub … : Vec<…>` fields** (grep count on `soft/component.rs:69-180`) — the named Principle-0 exception |
| Soft rest-volume discipline: `V0` computed with the IDENTICAL op sequence as `project_volume` so `C = V − V0` is exactly 0 at rest | `soft/component.rs:339-361` | a design obligation every new constraint type inherits |
| Soft colored dispatch: `try_with_active_pool` from inside the physics stage, inline below 256 slots, else `W × 6` chunks; coupling is serial-only on the colored path | `soft/colored.rs:36`, `:56-70`, `:686-735` | |
| Determinism contract: reference solver single-threaded manifold order; colored run-to-run + `{1, N}` bit-identity; four source censuses ban FMA/`rsqrt`/`rcp`/`mul_add`/`algebraic_*`; the ISA baseline is `x86-64-v3` (+fma) since 2026-09-02 | `soft_step.rs:50-60`, `colored.rs:2634-2660`; `docs/physics/FMA-DETERMINISM.md:86-110` (ecsnative) | properties (i)/(ii) are product, (iii) SIMD==scalar is the 0%-gate instrument |
| Golden: `bodytype_determinism_golden` (64-bit hash of the full final state; same-binary, same-machine, serialized spawn) | `tests/bodytype_determinism_golden.rs:1-40` | |
| Threadpool `Scope` exposes `spawn` and `spawn_batch` and nothing else — no in-scope barrier | `crates/boyko_threadpool/src/scope.rs:944`, `:994` | the 72-waves/step fact (§9) |
| Dense storage kind exists in the kernel: `DenseStore`, `DenseBuildView` (`!Send`), `DenseSolveView` (`Copy`, `row_ptr` only) | `crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:83`, `dense/views.rs:29`, `:110` | the storage the design's per-instance state uses |
| `boyko_math::Quat` API: `from_mat3`, `normalize`, `mul`, `rotate`, `conjugate`, `inverse_rotate`, `integrate` — **no `dot`, no swing/twist decomposition, no delta-quat-to-rotation** | `crates/boyko_math/src/quat.rs:70-224` | the joint rung's math prerequisite (animation AK-1 asks for `dot`/`nlerp` too) |

**Confirmed ABSENT by census** (`grep -rniE 'joint|hinge|motor|ragdoll|speculative|ccd|capsule|convex hull|tunnel|character|vehicle'` over `crates/boyko_physics/src`, 2026-09-10): every hit is the word `disjoint`, a doc mention of "many-ragdoll" scenes in the retired-gate rationale (`resources.rs:2373`, `colored.rs:3249`), the box-box generator's "convex" wording, and a test message about tunnelling. **There are zero joints, zero CCD, zero capsules, zero characters, zero vehicles.** `grep 'fracture|destruct'` over `docs/*.md` finds nothing physics-related.

**No soft-body performance number exists in the tree.** `docs/threadpool/KE16-RESULTS.md:1280-1290` prices the RIGID switches only (`simd_solve` 1.96× shipped OFF; colored-vs-reference 1.059× @1k / 1.131× @10k).

## 1. The solver family, and what has moved since the engine chose it

Every shipping engine surveyed converged on ONE solver shape, and this engine already has it:
sub-stepping + soft constraints + relaxation + warm starting, graph-coloured for parallel width.

- **Box2D v3 "Soft Step"** [B] https://box2d.org/posts/2024/02/solver2d/ — eight solvers compared
  (PGS, PGS_NGS, PGS_NGS_Block, PGS_Soft, TGS_Sticky, TGS_Soft, TGS_NGS, XPBD) at 4 primary / 2
  secondary iterations, 60 Hz; Soft Step = "sub-stepping instead of iterations, warm starting, soft
  constraints, and relaxation"; loop accounting: PGS = 2 body + 6 constraint loops, sub-stepping
  variants = 9–11 body loops; `deltaPosition` (positions relative to a frame origin) is the
  far-from-origin precision fix.
- **Box3D (2026-06)** [B] https://box2d.org/posts/2026/06/announcing-box3d/ , [S]
  https://github.com/erincatto/box3d — a 3D fork of Box2D v3 by the lineage's author: "Robust Soft
  Step rigid body solver", sub-stepping, "Graph coloring for large islands", "Wide SIMD contact
  solver", continuous collision, island sleeping, cross-platform determinism, recording/replay.
  **This is boyko's architecture in 3D and the single highest-value source for the joint rung.**
- **Jolt** [D] `Docs/Architecture.md`: sequential impulses + warm start per island, jobs over islands;
  NOT sub-stepped by default — `mNumVelocitySteps = 10`, `mNumPositionSteps = 2`, `mBaumgarte = 0.2`
  [S] `Jolt/Physics/PhysicsSettings.h`. Its iteration counts are therefore NOT transferable to a
  sub-stepped budget (pitfall P-46).
- **PhysX 5** [D] `SoftBodies.html` 5.4.1: `PxSolverType::eTGS` recommended, explicitly for soft
  bodies over PGS.
- **Rapier (2026 staged island solver)** [S]
  `src/dynamics/solver/staged_island_solver/mod.rs`: "contacts are colored so same-color constraints
  touch pairwise-disjoint bodies (SIMD-packed per color)"; persistent workers "claim batches between
  spin-barrier stages"; "joints/multibody solve in worker-0-exclusive stages"; results "depend only
  on the coloring, not on batch distribution".
- **Avian 0.4** [B] https://joonaa.dev/blog/09/avian-0-4 — `SolverBody` (32 B) / `SolverBodyInertia`
  (16 B), delta pose, greedy colouring with incremental updates, persistent Box2D-style islands;
  "approximately 3x faster than 0.3"; SIMD after colouring.
- **bepuphysics2** [D] `Documentation/Substepping.md`: with PGS sub-stepping dominates; with TGS
  sub-steps and iterations are more comparable; "a single solver iteration per substep has
  significantly more convergence power than each solver iteration of a single substep"; floor ~2
  iterations.
- **Macklin et al., "Small Steps"** [P] SCA 2019, https://mmacklin.com/smallsteps.pdf — n sub-steps ×
  1 iteration beats 1 step × n iterations. The theoretical basis of the engine's substep loop, and
  of where joints belong in it (inside the substep, not as a separate iteration budget).

**The one genuinely new family since the engine's decisions**: **Vertex Block Descent** [P] Chen,
Liu, Yang, Yuksel, ToG 2024, https://arxiv.org/html/2403.06321v3 — per-VERTEX block coordinate
descent on the variational implicit-Euler form (a 3×3 Newton step per vertex, Gauss-Seidel over
vertices); vertex colourings of **3–8** colours against element colourings of **7–76**; Chebyshev
acceleration; dual-buffer partial-Jacobi where collision forces span same-coloured vertices; reports
230k verts / 700k elements at 15–17 ms/frame (120 iterations), 10× over XPBD on the squishy-ball
test (0.031 s vs 0.32 s/frame), stability at 1:2000 mass ratios where XPBD crushes the light body.
**Augmented VBD** [P] Giles, Diaz, Yuksel, ToG 2025, https://graphics.cs.utah.edu/research/projects/avbd/
— an augmented-Lagrangian layer for hard constraints of infinite stiffness, better convergence at
high stiffness ratios, "articulated bodies connected with hard constraints, including joints with
limited degrees of freedom", one extra dual/stiffness pass per iteration; demos and source
published. **No AVBD figure was extracted** (both PDFs exceeded the fetch limit) — see §16. Why it
matters here: colour count is dispatch-wave count, and dispatch is the engine's measured weakness
(§9). Why it does not transfer at face value: Chebyshev and partial-Jacobi are order-dependent
machinery the `{1, N}` no-atomics contract forbids (pitfall P-30).

## 2. Joints and constraints

### 2.1 Type catalogues

| Engine | Types | Source |
|---|---|---|
| Box3D | **9**: parallel, distance, filter, motor, prismatic, revolute, spherical, weld, wheel — every joint is (spring \| limit \| motor) over its DOFs; spherical carries cone + twist; no gear / rack / pulley / path / vehicle constraint | [S] `box3d/src/joint.c`, `include/box3d/types.h` |
| Jolt | **13**: Fixed, Distance, Point, Hinge, Cone, Slider, SwingTwist ("approximates the shoulder joint of a human"), SixDOF ("per translation axis and rotation axis what the limits are"), Path (Hermite), Gear, RackAndPinion, Pulley, Vehicle | [D] `Docs/Architecture.md` |
| Rapier | ONE generic 6-DOF `ImpulseJoint`; Revolute/Prismatic/Fixed/Spherical are constructors over it; separately `MultibodyJoint` (reduced coordinates) | [D] https://rapier.rs/docs/user_guides/rust/joints/ |
| Avian 0.4 | per-body anchor + basis frames (6-DOF), `JointDamping`, `JointForces` (readable), `JointCollisionDisabled`, `JointGraph` resource; **motors and articulations not yet supported**; XPBD joints behind `xpbd_joints` | [B] Avian 0.4 post |
| Unity DOTS | `PhysicsJoint` = constraints on a `PhysicsConstrainedBodyPair`; `PhysicsJointCompanion` for setups that "require more than one instance to stabilize" (ragdolls) | [D] com.unity.physics 1.4 API |
| PhysX 5 | D6 + reduced-coordinate articulations (Featherstone lineage; "the pose of a link is determined recursively through the pose of its parent") | [D] `Articulations.html` 5.5 |

### 2.2 The soft-step joint pipeline (Box3D)

- Three functions, the SAME shape as contacts: `b3PrepareJoint` ("Clamp joint hertz based on the
  time step to reduce jitter"; world anchor frames relative to COM; effective masses;
  `b3MakeSoft(hertz, ζ, h)`), `b3WarmStartJoint` (apply every accumulator), `b3SolveJoint(useBias)`
  — `useBias` is the ONLY difference between the main substep sweep and a relax sweep [S]
  `box3d/src/joint.c`. Boyko's `solve_velocities(.., bias_active)` is the same switch
  (`soft_step.rs:524-531` [T]).
- Explicit `_Overflow` prepare / warm-start / solve entry points: the overflow colour is a scalar
  serial lane [S] `joint.c`; "graph colors run wide, the overflow set stays scalar" [B]
  https://box2d.org/posts/2024/08/simd-matters/ .
- **Joints are scalar; only contacts go wide.** No SIMD widening in any Box3D joint solver, and
  `B3_SIMD_WIDTH` is **4 in every branch** (SSE2 / NEON / none / WASM — there is no AVX2 branch)
  [S] `box3d/src/core.h`. Box2D v3 widens contacts only. Boyko's 8-wide cohorts have no joint
  precedent anywhere in the survey (pitfall P-19).
- Reaction/breaking API: `b3GetJointConstraintForce` / `…Torque`, `b3GetJointReaction`
  (impulses × `invTimeStep`), `b3Joint_GetLinearSeparation` / `…AngularSeparation` [S] `joint.c`.
- Joints are island edges: a joint enters the awake graph when either body is awake
  (`b2CreateJointInGraph` gated on the awake set) [S] `box2d/src/joint.c`.

### 2.3 The spherical (ragdoll) joint, in full [S] `box3d/src/spherical_joint.c`

- Prepare: anchor frames relative to COM; swing effective mass
  `k = dot(swingAxis, invInertiaSum · swingAxis)`, `mass = k > 0 ? 1/k : 0` — a SCALAR per limit,
  not a matrix; same for twist; softness via `b3MakeSoft`.
- Warm start applies **five separate accumulators**: point-to-point (3-vector), spring, motor,
  swing/cone, twist lower AND twist upper (lower/upper are separate non-negative accumulators,
  never one signed one).
- Solve: spring via `b3DeltaQuatToRotation(relQ, targetRotation)` and the full 3×3 rotational mass;
  motor clamps to `maxMotorTorque · h`; twist limits with error `twistAngle − lowerTwistAngle`;
  cone limit error `coneAngle − swingAngle` with "sign flipped on Cdot"; point-to-point
  `K = diag(1/m1 + 1/m2) − r1_skew·invI1·r1_skew − r2_skew·invI2·r2_skew`.
- Rotation bookkeeping: `relQ = InvMulQuat(quatA, quatB)`, negate `quatB` when
  `dot(quatA, quatB) < 0` — "keeps the swing angle in the range [0, π]" / twist in `[−π, π]`;
  swing and twist extracted by dedicated decomposition functions.
- **Limits use a SPECULATIVE regime, and it sets THREE variables, not one** (⚠ corrected in the
  critique round — the first draft of this bullet gave only the bias, and the design transcribed the
  omission): `massScale` and `impulseScale` are loaded from the softness **only in the overlap
  branch**; the speculative branch leaves them neutral. Box2D's contact form, which is the same
  regime and is the one this survey can quote verbatim ([S]
  https://raw.githubusercontent.com/erincatto/box2d/main/src/contact_solver.c ,
  `b2SolveContacts_Overflow`, fetched 2026-09-10):

  ```c
  float velocityBias = 0.0f;  float massScale = 1.0f;  float impulseScale = 0.0f;
  if ( s > 0.0f )      { velocityBias = s * inv_h; }                        // speculative
  else if ( useBias )  { velocityBias = b2MaxFloat( softness.massScale * softness.biasRate * s,
                                                    -contactSpeed );
                         massScale    = softness.massScale;
                         impulseScale = softness.impulseScale; }
  float impulse = -cp->normalMass * ( massScale * vn + velocityBias )
                  - impulseScale * cp->normalImpulse;
  ```

  Leaving `massScale = 1`, `impulseScale = 0` in the speculative branch is what makes "arrive
  exactly on the surface" true; softening a speculative constraint (at boyko's defaults
  `massScale ≈ 0.95`, `impulseScale ≈ 0.05`) destroys the property the regime exists for. The same
  split in `revolute_joint.c`. Zero extra passes.
  Note also, for §6 and for the design's X0 rung: **Box2D multiplies the soft bias by `massScale`
  INSIDE the clamp**, which boyko's shipped kernel does not (`soft_step.rs:569-575` clamps the
  unscaled bias, then `soft_step.rs:577` multiplies) — a second, pre-existing divergence.

### 2.4 Coloring with joints in the graph

- Box2D v3 / Box3D colour ONE graph holding contacts AND joints (`b2AssignJointColor` /
  `b3AssignJointColor`) with the rule, verbatim from `box2d/src/constraint_graph.c`: **"a joint
  cannot share the same color as a contact between the same two bodies"** [S]. Boyko's animation
  design already records it as P23 and AK-4 (`docs/animation/ANIMATION-RESEARCH.md:352`,
  `ANIMATION-DESIGN-SPACE.md:571` [T]).
- Palette: `B3_GRAPH_COLOR_COUNT` = 24, the last index is overflow (no body bitset on overflow),
  `B3_DYNAMIC_COLOR_COUNT` reserves the LOW colours for dynamic-dynamic constraints; constraints
  touching a static body are searched from the TOP downward (Box2D: `B2_DYNAMIC_COLOR_COUNT =
  COUNT − 4`, "to reduce tunneling") [S] `box3d/src/constraint_graph.c`,
  `box2d/src/constraint_graph.h`. Static bodies are never entered in a bitset (`invMassA = 0`).
  ⚠ **The reason for the TOP-down search is Gauss-Seidel ORDER, not occupancy** — the last-solved
  constraint is the best-satisfied one, so a static contact solved last is a wall that holds. The
  "statics impose no occupancy" fact is true of Box2D as well and distinguishes nothing. Two things
  must exist to adopt it here and only one of them does: boyko shares the no-static-occupancy
  property, but **has no palette at all** — `color_manifolds` appends occupancy words for a new
  colour without any upper bound (`resources.rs:2884-2891` [T]), where Box2D reserves the top of a
  fixed 24-entry palette with an overflow lane. "Reserved high colours" therefore presupposes a
  constant this engine does not have (design D-14, C-11).
- **Kinematic bodies must be coloured like DYNAMIC ones**, verbatim rationale: "we cannot access a
  kinematic body from multiple threads efficiently because the SIMD solver body scatter would write
  to the same kinematic body from multiple threads" [S] `box3d/src/constraint_graph.c`. Boyko's
  animation design records it as P22 (kinematic rows read-only in the scatter).
- Rapier does the opposite — joints and multibodies in a worker-0-exclusive stage [S]
  `staged_island_solver/mod.rs`. Both designs ship; the trade is parallel width vs a second kernel
  family inside the colour wave.
- Persistence: Box2D sets the two body bits on constraint creation and clears them on removal;
  sleep removes, wake re-adds — no per-step rebuild [B] https://box2d.org/posts/2023/10/simulation-islands/ ,
  [S] `constraint_graph.c`. Avian 0.4 the same. Rapier's `solver_contact_graph.rs`: "per-color flat
  arrays of solver contacts, maintained incrementally by the narrow phase over the contact
  lifecycle", `NUM_SOLVER_COLORS = 129` (128 + overflow), a `GENERIC_BUCKET` for multibody
  manifolds, `ContactRef::PADDING` for unused SIMD lanes [S].

### 2.5 Motors, springs, limits

- Jolt motor: `force = stiffness·(target_pos − pos) + damping·(target_vel − vel)`; states Off /
  Velocity (infinite damping) / Position (`target_vel = 0`) / PositionAndVelocity; force and torque
  limits [D] `Architecture.md`. Spring modes: stiffness+damping; mass-normalised; frequency+damping
  ratio "implemented as described in Soft Constraints: Reinventing The Spring — Erin Catto — GDC
  2011" [P/B] http://box2d.org/files/GDC2011/GDC2011_Catto_Erin_Soft_Constraints.pdf — the very
  derivation `SoftCoefficients::new` implements.
- Valid frequency range `(0, 0.5 × simulation frequency]` [D] Jolt SixDOF settings; Box3D clamps
  in prepare. A joint authored above half the substep rate is unstable BY CONSTRUCTION (pitfall
  P-21).
- **"Only soft translation limits are supported, soft rotation limits are not currently supported"**
  [D] Jolt `Architecture.md`. Box3D's cone/twist limits are hard with a speculative bias. No
  surveyed engine ships a soft rotation limit (pitfall P-22).
- Box3D motor joint: linear/angular velocity targets with `maxVelocityForce/Torque` PLUS position
  control via `linearHertz`/`angularHertz` and `maxSpringForce/Torque` — a PD drive between two
  bodies, the powered-ragdoll primitive [S] `types.h`.

### 2.6 Warm start and joint state lifetime

Box3D's `b3JointSim` holds frames, accumulated impulses and a per-type union, PER JOINT, across
frames [S] `joint.c`. This is a different lifetime model from boyko's contact `WarmStartTable`,
which is rebuilt-and-swapped every frame keyed by `pack(body_a, body_b, feature_id)`
(`warm_start.rs:1-60` [T]). Jolt's P21 (`ResetWarmStart` after `SetPose`) [D] Jolt `Ragdoll` is
the interaction point the animation design names (`ANIMATION-DESIGN-SPACE.md:571`).

### 2.7 Breakable joints — no engine breaks INSIDE the solver

- PhysX: `joint->setBreakForce(force, torque)` → `PxConstraintFlag::eBROKEN` +
  `onConstraintBreak()` [D] `Joints.html` 5.4.
- Jolt: read `GetTotalLambdaPosition` / `GetTotalLambdaRotation` after the step,
  `Constraint::SetEnabled(false)` over a threshold [D] Jolt discussion #141 + `Constraint.h`.
- Box3D: `b3GetJointConstraintForce/Torque`, `b3GetJointReaction`, `Get{Linear,Angular}Separation`
  [S] `joint.c`.
- Blast: a bond flagged `BondJointed` becomes an inactive internal joint that ACTIVATES when a split
  puts its two chunks in different actors [D] Blast high-level guide.
The break decision is frame-lagged and reads a value that depends on sweep order — under a colored
solver, two runs that are merely "equally valid" would break DIFFERENT joints (pitfall P-24). Under
boyko's `{1, N}` bit-identity the readout is a deterministic product, which is exactly why the
contract matters here.

### 2.8 The alternative: reduced-coordinate articulations

PhysX articulations and Rapier `MultibodyJoint` keep scalar per-DOF joint state and derive link
poses recursively (Featherstone; Bullet `btMultiBody` [S]) so the joint "can't violate their
positional constraint" — "more stable but slower and can be used for IK" [D] Rapier. A chain is a
serial recursion and does NOT graph-colour; every engine that has both puts articulations in a
separate scalar path (Rapier's `GENERIC_BUCKET`). Rapier states the full-coordinate failure plainly:
"the rigid-body will break the joint constraint if the constraint solver does not converge
completely" (pitfall P-20). The animation design already chose maximal coordinates (P24).

## 3. Ragdolls

- **Jolt** [S] `Jolt/Physics/Ragdoll/Ragdoll.h`: `RagdollSettings::mParts` 1-on-1 with
  `mSkeleton.GetJoints()`, each `Part` = `BodyCreationSettings` + `mToParent` constraint;
  `mAdditionalConstraints` for non-hierarchical pairs; `mBodyIndexToConstraintIndex` /
  `mConstraintIndexToBodyIdxPair`; `Stabilize()`; **`CalculateConstraintPriorities()` so
  "constraints near the leaves… have a lower priority than constraints near the root"**;
  `DriveToPoseUsingKinematics` (velocities so bodies reach the pose in `dt`) vs
  `DriveToPoseUsingMotors` ("the joints act like muscles"); `DisableParentChildCollisions()` via a
  group-filter table keyed on joint index.
- **Unity** [D] https://docs.unity3d.com/6000.0/Documentation/Manual/RagdollStability.html —
  adjacent mass ratios above ~10:1 jitter; keep the torso heaviest. An AUTHORING constraint
  (Gaia-shaped joint-graph data), not a solver one; AVBD is the only surveyed work that claims to
  address high stiffness ratios directly.
- **Unity DOTS** `PhysicsJointCompanion`: "complex setups like ragdoll joints require more than one
  instance to stabilize" [D].
- The animation design's L7 row expects a ~15–20-body island per character, parallelism ACROSS
  characters (`ANIMATION-RESEARCH.md:378` [T]).

## 4. Character controllers

Two tiers everywhere; both leading designs are deliberately NOT rigid bodies.

- **Box3D mover** [D] https://box2d.org/documentation3d/md_character.html — a GEOMETRIC solver
  "driven entirely by application code". Per frame: desired translation → `b3World_CastMover`
  (safe fraction) → move → `b3World_CollideMover` (gather PLANES, not manifolds) → filter →
  `b3SolvePlanes` ("finds the position delta closest to targetDelta that satisfies all planes") →
  `b3ClipVector`. Each plane carries `pushLimit` (`FLT_MAX` rigid; finite = soft, for yielding
  doors/enemies) and a `clipVelocity` flag. Capsule chosen because "its round profile slides
  smoothly along edges and corners without snagging". **Stated limitation: "The mover has no
  explicit rotation handling."** Box2D 3.1 introduced it as experimental [B]
  https://box2d.org/posts/2025/04/box2d-3.1/ .
- **Jolt** [D] `Architecture.md`, [S] `CharacterVirtual.cpp`: `Character` = a real body constrained
  against rotation, `PostSimulation` after each update; `CharacterVirtual` = "implemented using
  collision detection functionality only", "not added to the world", interacts by applying impulses
  during `Update`; wall sliding, moving platforms, steep-slope detection, `WalkStairs` (cast up,
  forward, down), `StickToFloor`, both in `ExtendedUpdate`; custom local coordinate systems (walking
  inside a flying ship); contact-normal merging ≈2.5°; optional `mInnerBodyShape` — a real body
  follows the virtual character so the world can see it (the documented escape hatch).
- Both require a **capsule** and a **shape cast** — neither exists in the tree (§0).

## 5. Vehicles — three schools

| School | Shape | Source |
|---|---|---|
| **Wheel joint** (Box3D) | no vehicle subsystem: a car = chassis body + 4 wheel joints, each with suspension spring (hertz/ζ) + limits, spin motor (`maxSpinTorque`/`spinSpeed`), steering spring (`steeringHertz`/ζ/`targetSteeringAngle`/`maxSteeringTorque`) + limits — solved in the same coloured graph | [S] `box3d/include/box3d/types.h` |
| **Raycast** (Bullet, Rapier) | "a rayCast vehicle, very special constraint that turns a rigidbody into a vehicle"; wheels are rays, suspension an analytic force on the chassis, no wheel bodies; tuning = suspensionStiffness / Compression / Damping / maxSuspensionTravel / frictionSlip / maxSuspensionForce | [S] `btRaycastVehicle.cpp`; [D] Rapier `DynamicRayCastVehicleController` |
| **Constraint + step listener** (Jolt) | `VehicleConstraint : Constraint, PhysicsStepListener`; virtual wheels with a pluggable `VehicleCollisionTester` (Ray / CastSphere / CastCylinder); engine, gearbox, differentials; wheel tests amortised round-robin (`mNumStepsBetweenCollisionTestActive/Inactive`, spread by `BodyID`); `mNumVelocityStepsOverride` | [S] `Jolt/Physics/Vehicle/VehicleConstraint.h` |
| **Component pipeline** (PhysX 5.1+) | the monolithic classes replaced by "flexible vehicle components… mixed, matched and customised", arranged in sequences | [D] `Vehicles.html` 5.3 |

## 6. Continuous collision detection

- **Speculative contacts** [B] Firth 2011 https://wildbunny.co.uk/blog/2011/03/25/speculative-contacts-an-continuous-collision-engine-approach-part-1/ ,
  [P/talk] Catto GDC 2013 https://box2d.org/files/ErinCatto_ContinuousCollision_GDC2013.pdf : contact
  constraints are created BEFORE bodies touch within a speculative distance (Box2D v3: 0.02 m = 4 ×
  linear slop 0.005 m; Jolt `mSpeculativeContactDistance = 0.02f` [S] `PhysicsSettings.h`); the
  constraint limits the approach velocity so the bodies land exactly on the surface. Known failure:
  ghost collisions on internal edges of a tiled floor or mesh (pitfall P-25).
- **The joint-limit form is the same regime** (§2.3), and — ⚠ corrected in the critique round — it
  is a **three-variable** regime: `C > 0 ⇒ bias = C·inv_h`, `massScale = 1`, `impulseScale = 0`.
  **Boyko's normal solve has no such regime.** The shipped kernel is, when `bias_active`
  (`soft_step.rs:569-581` [T]):

  ```rust
  let bias = (soft.bias_rate * pc.separation).max(-MAX_BIAS_VELOCITY);           // ANY sign
  let d_lambda = -soft.mass_coeff * m_eff * (vn + bias) - soft.impulse_coeff * pc.normal_impulse;
  ```

  so the soft mass and impulse scalings are applied **unconditionally** — there is no branch for
  them to be neutral in. Two consequences for the design's X0 rung, both of which its first draft
  missed:
  1. Adding only the bias branch is not enough: at the defaults `mass_coeff = a2·a3 ≈ 0.95` and
     `impulse_coeff = a3 ≈ 0.05`, a speculative contact would still be softened and would not land
     exactly on the surface.
  2. The bias magnitude itself: with `contact_hertz = 30`, `ζ = 10`, `h = 1/240`,
     `bias_rate = ω/a1 ≈ 188.5 / (20 + 0.785) ≈ 9.1 s⁻¹` against `inv_h = 240 s⁻¹`, so a
     positive-separation manifold fed to today's kernel is stopped ~26× too early (arithmetic from
     the constants, not a measurement).

  The narrowphase never emits a positive separation, which is why all of this is invisible today.
- **TOI + fast-body flag + motion clamping** (Box3D) [S] `box3d/src/solver.c`: `b3_isFast` when
  `maxMotion > safetyFactor · minExtent`; `b3SolveContinuous` sweeps AABBs against static,
  kinematic AND dynamic trees, "re-center[s] the sweep on the fast body so the TOI and the swept
  query stay in float precision"; `b3ShapeTimeOfImpact` = conservative advancement; the body is
  advanced to the fraction. **Bullet bodies run SERIALLY afterwards** (`b3BulletBodyTask`) because
  parallel bullet-vs-bullet resolution has "non-deterministic order" — under boyko's contract this
  is mandatory, not a quality choice. Box2D 3.1 measured "continuous collision performance is
  roughly doubled" and GJK 20% faster [B].
- **Linear cast with time stealing** (Jolt) [D]: `MotionQuality::LinearCast`, thresholds as
  fractions of the body's inner radius (`mLinearCastThreshold = 0.75`, `mLinearCastMaxPenetration
  = 0.25` [S]); "objects may appear to move slower for one frame then resume normal speed". Rapier:
  nonlinear (translation + rotation) TOI by motion clamping, per-body `ccd_enabled` [D]. There is no
  free CCD; the choice is which artefact (pitfall P-26).

## 7. Soft bodies and cloth

### 7.1 The reference constraint catalogue is stable across engines

- **Jolt** [S] `Jolt/Physics/SoftBody/SoftBodySharedSettings.h`: `Vertex { mPosition, mVelocity,
  mInvMass }`, `Face { mVertex[3], mMaterialIndex }`, `Edge { mVertex[2], mRestLength, mCompliance
  }`, `DihedralBend { mVertex[4], mCompliance, mInitialAngle }`, `Volume { mVertex[4],
  mSixRestVolume, mCompliance }`, `Skinned { mVertex, SkinWeight mWeights[4], mMaxDistance,
  mBackStopDistance, mBackStopRadius }`, `InvBind { mJointIndex, Mat44 mInvBind }`, `LRA {
  mVertex[2], mMaxDistance }` (Euclidean or geodesic), Cosserat rods `RodStretchShear { mLength,
  mBishop }` / `RodBendTwist { mOmega0 }`, and `UpdateGroup` — "the end indices for each group of
  constraints that can be updated in parallel". `Optimize()` groups by greedy SPATIAL partitioning
  (bbox → longest axis → seed → grow by connectivity, capped at `cVertexConstraintBatch`), with a
  final catch-all group for cross-group constraints [S] `SoftBodySharedSettings.cpp`. Per-iteration
  order: volume → dihedral bend → skinned → edge → rod stretch/shear → rod bend/twist → LRA;
  pressure once per iteration BEFORE integration [S] `SoftBodyMotionProperties.cpp`. Workers claim
  groups by atomic `fetch_add`, last group serial; `ParallelUpdate()` returns `EStatus{NoWork,
  DidWork, Done}` (cooperative claiming, not fork-join) [S]. `CreateConstraints(.., bend ∈ {None,
  Distance, Dihedral}, angle tolerance)` — the cheap diagonal-spring bend and the correct dihedral
  bend are an AUTHORING choice [D]. Limits: "Soft bodies can only collide with rigid bodies,
  collisions between soft bodies are not implemented yet"; "constraints cannot operate on soft
  bodies"; no determinism guarantee stated for soft bodies [D] https://jrouwe.github.io/JoltPhysics/ .
- **Chaos Cloth** [D] `ChaosClothAssetDataflowNodes`: `PBDEdgeSpring`, `PBDBendingSpring`,
  `PBDBendingElement`, `PBDAreaSpring` and the XPBD twins, `XPBDAnisoSpring/Stretch/Bending`,
  `LongRangeAttachment_v2`, `SelfCollision_v2`, `SelfCollisionSpheres`, `ClothVertexSpring`,
  `ClothVertexFaceSpring`, `Aerodynamics`, `Pressure`, `Backstop`, `AnimDrive`, `MaxDistance`,
  `Damping`, `Gravity`, `Mass`, `VelocityScale`, `ResolveExtremeDeformation`, `Solver`. The PBD/XPBD
  duplication is a COMPATIBILITY tax (assets tuned against iteration-dependent stiffness), not
  physics (pitfall P-38). Collision via Level Set Volumes (voxel SDF) [D].
- **PhysX 5** [D] `SoftBodies.html` 5.4.1: fixed-corotated FEM (Lamé, authored as E/ν); THREE
  meshes — voxelised simulation tets, conforming collision tets, render mesh — plus a cooked remap;
  "TGS is recommended"; **GPU/CUDA only**; kinematic-partial soft bodies for animated skins.
- **Havok Cloth** [D] vendor page (marketing register): constraint-based LAYERED cloth to avoid full
  cloth-cloth detection; LOD transition to plain skinning. Treat as a claim.
- **Unity**: the built-in `Cloth` is bound to `SkinnedMeshRenderer` (NvCloth lineage) [D]; no
  reliable DOTS soft-body package found — do not assume one.

### 7.2 Techniques

- **XPBD** [P] Macklin, Müller, Chentanez, MIG 2016, DOI 10.1145/2994258.2994272 —
  `Δλ = (−C − α̃λ) / (Σ wᵢ|∇Cᵢ|² + α̃)`, `α̃ = α/Δt²`; `α = 0 ⇒ PBD`. The engine's formulation
  (`soft/component.rs:25-48` documents the exact denominator [T]); one projector spans cloth → soft
  solid → rigid attachment (`docs/RESEARCH-SOFT-BODY.md:37-41` [T]).
- **Long-range attachments** [P] Kim, Chentanez, Müller-Fischer, SCA 2012,
  https://matthias-research.github.io/pages/publications/sca2012cloth.pdf — a UNILATERAL
  max-distance tether from a free particle to a kinematic attachment; geodesic distance leaves more
  room to unfold. The standard cure for stretchy capes without raising substeps.
- **Quadratic / isometric bending** [P] Bergou et al., SGP 2006,
  https://cims.nyu.edu/gcl/papers/bergou2006qbm.pdf — bending energy quadratic in positions, linear
  gradient, CONSTANT Hessian; up to 3× reduction in cloth time vs dihedral bending.
- **Neo-Hookean as two compliant constraints** [P] Macklin & Müller, MIG 2021, DOI
  10.1145/3487983.3488289 — deviatoric + hydrostatic; first-order gradients, no polar decomposition,
  conditioning unaffected by stiffness, recovers from INVERTED elements. The natural upgrade of the
  engine's distance+volume tets, which cannot recover from inversion (`DegenerateTet` is a
  construction-time guard only; a mid-sim collapse is a `DENOM_EPS` skip — `soft/component.rs:44-47`,
  `soft/solver.rs:58` [T]; pitfall P-36).
- **Shape matching** [P] Müller et al., SIGGRAPH 2005 (topology-free); *Physically Based Shape
  Matching*, SCA 2022 — POINTER ONLY (PDF not opened, §16).
- **Self-collision**: Teschner spatial hashing [P] VMV 2003 (the engine's hash and primes,
  `soft/self_collision.rs:46-66` [T]); Bridson, Fedkiw, Anderson [P] SIGGRAPH 2002, DOI
  10.1145/566570.566623 (repulsion + CCD + friction with true thickness); **Air Meshes** [P]
  Müller, Chentanez, Kim, Macklin, ToG 2015, DOI 10.1145/2766907 — tessellate the AIR once, one
  unilateral non-inversion constraint per air element, cannot miss an event, untangles
  automatically; the only surveyed technique that makes robust self-collision a CONSTRAINT SET
  rather than a detection problem (the tessellation is a baked artefact and must be regenerated on
  tearing).
- **Tearing / plasticity**: per-constraint deformation threshold breaks the constraint and rest
  conditions are recomputed; plasticity updates the rest shape past an elastic threshold — from the
  Bender et al. survey [P] CGF 2014, DOI 10.1111/cgf.12346 and secondary summaries [B]; re-derive
  from the survey before design. XPBI/PBI [P] arXiv 2405.11694 is the continuum-inelasticity
  extension. Tearing invalidates the colouring CSR, the `next_pow2(2n)` hash size, the preallocated
  columns and the zero-allocation contract at once (pitfall P-33).
- **SDF collision for particles** [P] Macklin et al., I3D 2020, `mmacklin.com/sdfcontact.pdf` —
  inherited pointer from `docs/RESEARCH-SOFT-BODY.md:85` [T], not re-opened this pass.
- **Parallel block neo-Hookean via graph clustering** [P] MIG 2022 — pointer inherited from
  `RESEARCH-SOFT-BODY.md` sources; not re-opened.

## 8. Destruction and fracture

### 8.1 Everybody precomputes the geometry and simulates only a graph

- **NVIDIA Blast** [S] `blast/include/lowlevel/NvBlastTypes.h`: `NvBlastChunk { centroid[3],
  volume, parentChunkIndex, firstChildIndex, childIndexStop, userData }` (a hierarchy as contiguous
  child ranges, no pointers); `NvBlastBond { normal[3], area, centroid[3], userData }` (`userData`
  carries "whether or not to create a joint"); `NvBlastSupportGraph { nodeCount, chunkIndices,
  adjacencyPartition, adjacentNodeIndices, adjacentBondIndices }` — **CSR**; per-family mutable
  `familyBondHealths` / `supportChunkHealths` separate from the immutable asset; fracture commands as
  flat records `NvBlastBondFractureData { userdata, nodeIndex0, nodeIndex1, health }` /
  `NvBlastChunkFractureData`. Support chunks "form an exact cover" of the root; damage = a user
  "damage program" (`graphShaderFunction` / `subgraphShaderFunction`) converting an impact into
  health loss using bond normal/area/distance (anisotropy = compare impact direction with the bond
  normal); "Blast performs island detection on the support graph to find all groups of support
  chunks that are connected by unbroken bonds" and creates new actors; visible chunks = highest
  ancestors fully owned [D] introduction. **Three phases**: `NvBlastActorGenerateFracture`
  (no mutation) → `NvBlastActorApplyFracture` → `NvBlastActorSplit`, each with a scratch query [D]
  low-level guide. "Blast guarantees that it is safe to operate on different actors from different
  threads"; `TkGroup::startProcess()` → workers `process(job)` → `endProcess()`, "every job must
  be processed exactly once" [D] high-level guide. No determinism statement. `ExtStressSolver`
  over the bond graph: tension / compression / shear limits, ≥1 fixed node for gravity,
  `getOverstressedBondCount()`, "computation time is linearly proportional to
  `maxSolverIterationsPerFrame`" [D] ext_stress. Authoring: Voronoi / slicing (noisy) / cutout;
  `BondGenerator` exact vs average; `ConvexMeshBuilder`; input must be closed, CCW, no
  self-intersection, no T-junctions [D] ext_authoring.
- **Chaos Destruction** [D] `chaos-destruction-overview` 4.27, [D] UE 5.4 Python API
  `GeometryCollection`: a `FManagedArrayCollection` columnar asset with a multi-level cluster
  hierarchy; "a connection graph is initialized based on each fractured rigid body's nearest
  neighbors… Each connection… is given initial strain values"; connections break when an impulse
  exceeds the limit, left-over damage propagates. Properties: `damage_model`, `damage_threshold:
  Array[float]` per level, `damage_propagation_data`, `per_cluster_only_damage_threshold`,
  `use_material_damage_modifiers`, `use_size_specific_damage_threshold`, `cluster_connection_type`
  ∈ {`CHAOS_POINT_IMPLICIT`, `…MINIMAL_SPANNING_SUBSET_DELAUNAY_TRIANGULATION`, `…AUGMENTED…`,
  `…BOUNDS_OVERLAP_FILTERED…`} (legacy in newer versions), `max_cluster_level`,
  `connection_graph_bounds_filtering_margin`; debris lifecycle as data: `remove_on_max_sleep`,
  `maximum_sleep_time`, `removal_duration`, `scale_on_removal`, `slow_moving_as_sleeping`,
  `slow_moving_velocity_threshold (cm/s)`; a per-frame break throttle
  `p.Chaos.Clustering.PerAdvanceBreaksAllowed` and Root Mesh Proxies [B] hzFishy notes. Fracture
  tools: Uniform/Cluster Voronoi, Radial, Planar, Slice, Brick (bond patterns), Mesh-cut, Custom
  sites; noise; Chance to Fracture; grout [D] fracturing guide. Fragment collision = Level Set
  Volumes (voxel SDF), "memory scales cubically" [D].
- **Havok Destruction**: no usable public technical documentation found — UNVERIFIED (§16).

### 8.2 Runtime fracture exists only where the representation is cheap to cut

- **Smash Hit** [B] https://blog.voxagon.se/2014/05/13/cracking-destruction.html — "slice all the
  convex shapes with five bounding planes, slightly randomized around the point of impact",
  half-edge vertex tracking, GJK separation test, ≤3 solves/frame INSIDE the physics loop;
  "objects always break where they get hit" (correctness traded for reliability).
- **Teardown** [B] https://blog.voxagon.se/ — voxel volumes on a regular grid with an 8-bit palette
  (≤255 materials), chosen because it "simplifies physics and graphics when breaking them up".
- **Dreams / Claybook** [B] Media Molecule SIGGRAPH 2015; [B/D] Aaltonen GDC 2018 — SDF/CSG as the
  representation; "SDF solves tunneling elegantly with negative inner distances, triangles are just
  a thin shell". The GDC PDF's numbers were NOT extracted (§16).
- **VACD + pattern insertion** [P] Müller, Chentanez, Kim, SIGGRAPH 2013 — volumetric approximate
  convex decomposition + an authored fracture pattern positioned at the impact, supporting PARTIAL
  and repeated fracture with no crack simulation (PDF too large; method from the abstract/listing).
- **Fracture modes** [P] Sellán et al., ToG 42(1) 2023, https://arxiv.org/abs/2111.05249 — sparsified
  eigenproblem → k lowest-energy modes; runtime = project the impact onto the k-dim subspace and
  threshold, O(k·ñ): **precompute 0.63–11.6 s per mode, 3 545–12 162 tets, runtime 1.06–1.96 ms**
  on a 2020 MacBook Pro; avoids Voronoi's "overly regular and convex fragments".
- **DMM / corotational FEM** [P] Parker & O'Brien, SCA 2009 (Force Unleashed) — timing table not
  extracted.
- **Convex decomposition for chunk colliders** [P] CoACD, SIGGRAPH 2022, https://arxiv.org/abs/2205.02961 —
  vs V-HACD on the V-HACD dataset: 192.1 s vs 201.9 s, 60.2 hulls @0.067 concavity vs 29.8 @0.044.
  Offline either way.
- **DeepFracture** [P] arXiv 2310.13344 — learned, research-stage.
- **The Finals** runs all destruction SERVER-side [D/B] Embark GDC page + press (paywalled talk);
  client-side nondeterministic fracture cannot be retrofitted into that model.
- Arbitrary-mesh booleans at runtime are what engines avoid; this tree's own §16 recorded "runtime
  refracture = tens of ms, not its profile", explicitly reasoned-not-measured
  (`docs/sdf-engine-architecture.md:419` [T]).

### 8.3 In-tree facts that bound the destruction rung [T]

- `ColliderShape` is `Sphere | Box` (`components.rs:127-145`); Voronoi shards are hulls — without a
  hull shape, "fracture" degrades to boxes.
- The analytic edit list is capped: `MAX_SDF_EDITS = 16` (`crates/boyko_sdf_math/src/lib.rs:105`),
  the `[SdfEdit; 16]` array is `#[repr(C)]`-frozen "byte-identical to the physics kernel's" and
  the AABB ledger is a COLD trailing member never interleaved into it (`lib.rs:275-278`, `:318-323`).
  The edit list cannot be a destruction log.
- The brick module exists in the leaf (`BRICK_INTERIOR = 8`, `APRON = 1`, `BRICK_LEVELS = 3`,
  `brick.rs:34-44`, `:195-207`) and the CPU mesh→SDF baker is byte-parallel to `fill_brick` /
  `classify_brick` (`mesh_sdf.rs:1-18`); the atlas does an incremental dirty-brick rebake with a
  sub-region upload, bit-identical to a full rebake (`crates/boyko_rhi_vulkan/src/brick_atlas.rs:14-19`).
  **The physics never samples a brick** (the `brick|atlas` grep over the physics crate hits only doc
  comments in `sdf_query.rs`).
- The spawn budget: "20k spawns/frame through `Commands` is 0.6–2 ms of CPU against ~2 µs of GPU
  emit, so a particle is never an `Entity`" (`crates/boyko_render/src/particle.rs:5-6`); `spawn_batch`
  exists (`crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:1079`).
- The colorer first-fits over ABSOLUTE body-slot occupancy, so free-list churn that changes slot
  values changes the colouring → different (valid) floats; determinism is claimed only "for a
  FIXED, deterministically-ordered op sequence" (`docs/DENSE-COMPONENTS-PLAN.md:56-58`).
- The routing rule "SDF decides WHERE the cut passes, the solver decides HOW debris behaves" is
  written and unmeasured (`docs/sdf-engine-architecture.md:402-455`; `RENDER-PHYSICS-GPU-RESEARCH.md:132-162`);
  the dirty-region regen estimate "~1 M brick regens/frame ≈ the 16 ms edge" is reasoned
  (`RENDER-PHYSICS-GPU-RESEARCH.md:111-119`).
- `docs/RENDER-PHYSICS-GPU-PLAN.md:210-218` names the in-house SDF-native paths as future edges:
  "collision-mesh-from-SDF via Marching Cubes… tied to the dirty-brick set; body-splitting via
  connected-components; XPBD/MPM continuum ('cut' = bulk-despawn particles/tets where
  `cutterSDF < 0`)".

## 9. Persistence, islands and dispatch — where the measured loss is

- **Box2D**: solver sets (`b2_staticSet`, `b2_disabledSet`, `b2_awakeSet`, one sleeping set per
  island — "The purpose of solver sets is to achieve high memory locality"); colouring persists;
  islands persist with merge-on-add union-find and DEFERRED split under a quota, "roughly an order
  of magnitude faster than DFS"; sleeping per island [S] `solver_set.h`, [B] simulation-islands.
  Stage pipeline verbatim: `b2_stagePrepareJoints, b2_stagePrepareContacts, b2_stageIntegrateVelocities,
  b2_stageWarmStart, b2_stageSolve, b2_stageIntegratePositions, b2_stageRelax, b2_stageStoreImpulses`
  [S] `solver.h`. **Resident workers** sweep a ring CAS-claiming `b2SyncBlock`s; the orchestrator
  publishes `atomicSyncBits` (stage + counter); workers spin with `b2Pause()` — N stages cost N
  atomic publishes, not N dispatches [S] `solver.c`. Box3D: "one stage per color for each
  iteration", each colour its own sync block, "workers receive at most M blocks of work" [S].
- **Jolt**: `LargeIslandSplitter` for islands over `cLargeIslandTreshold = 128`, `cNumSplits =
  sizeof(SplitMask)·8`, a non-parallel split for the residue, merge of splits under
  `cSplitCombineTreshold = 32`, atomic batch claim; cites Chen et al., "High-Performance Physical
  Simulations on Next-Generation Architecture with Many Cores" [S] `LargeIslandSplitter.h`. The
  ALTERNATIVE to colouring inside a big island.
- **Boyko today** [T]: islands + colours REBUILT every step into capacity-reused `ScratchColumn`s
  (`resources.rs:2625-2635`); the production route runs `physics_solve_colored` ON A WORKER, whose
  `pool.scope` tasks reach `injector_local[wid]` that no sibling polls — measured AS the serial
  floor: 1024 × 1 ms tasks = 1024.26 ms vs 1024.0 serial; `max_in_flight = 1` on every worker-route
  rep at W=4 and W=16 (`docs/threadpool/KE16-RESULTS.md:286-300`, ecsnative copy). The acceptance
  line FAILS at baseline: `in_scheduled_system` 29.98 ms vs `single_threaded_O5` 26.27 ms, 1.141×
  worse (`:301-316`); axis A closed on `a3` with 11.709 ms vs 28.096 ms (`:390-400`, `:546-552`).
  App-1: `lanes = pool.num_threads()` at the three physics sites ⇒ `W × {4, 6, 6}` chunks
  (`docs/threadpool/KE16-DESIGN-APP.md:14-32`). App-4: one wave per colour via `spawn_batch`
  (`colored.rs:2925-2945`).
- **Where boyko's waves actually come from** [T], since a design leaned on a stage that does not
  exist: the substep loop is gravity (single-threaded) → **`Self::warm_start_apply(...)`, ONE serial
  pass over every contact slot** (`colored.rs:3273`, whose comment reads "single-threaded here,
  parallel in `solve_all_colors`") → `solve_all_colors(bias = true)` (`:3276`) → position integrate
  → `refresh_inertia` → `solve_all_colors(bias = false)` × `relax_iterations` (`:3307`). **Only
  `solve_all_colors` dispatches.** There is no per-colour warm-start wave, and Box2D's stage list
  puts `b2_stageWarmStart` before `b2_stageSolve` wholesale for a value reason, not a scheduling one
  — warm-starting a constraint after other constraints have already solved against un-warmed
  velocities is a different simulation (design C-2).
- **The wave count**: 4 substeps × (1 + 2 relax) × ~6 colours = **72 `pool.scope` waves per
  step**, 1152 wake/park events at W=16; the fix is one scope per STEP with an in-scope BARRIER,
  which `Scope` lacks and which is owned by the KE16 lane (`docs/OPEN-QUESTIONS.md:4996-5010`).
  Jolt-parity pyramid (1240 boxes + floor, `dt = 1/60`, sleeping off, colored + parallel_solve ON,
  default AllPairs broadphase — the bench never sets `broadphase`, `OPEN-QUESTIONS.md:56-59`):
  boyko 18.551 / 19.348 / 19.397 / 21.218 / 26.091 ms at W = 1/2/4/8/16 (**0.71× at 16**) vs Jolt
  16.362 → 3.901 ms (**4.19×**); single-thread ≈1.1× with ±7% session drift on Jolt's own binary
  (`OPEN-QUESTIONS.md:4965-4996`). The bench's own caveat: iteration budgets differ (Jolt 10+2 vs
  4 × (1+2)), so only T(1)/T(N) compares (`benches/jolt_parity_pyramid.rs:22-33`).
- **KE16 status** [D] memory `project-ke16-and-editor-session-state` (2026-09-10): the tournament
  winner is `a1f+b1+wc+c0`; the feature arms were removed at `67563d3b`; the Step-App retake is
  OWED, and its first attempt exposed an 11× serial floor at 1 µs bodies on the shipped placement
  that was traced to rustc 1.98.1 on windows-gnu (under 1.97.1 the record reproduces). The ecsnative
  copy of `KE16-RESULTS.md` still reads "App: NOT RUN — OWED" (`:1-30`); the threadpool worktree's
  copy carries a "§App. TAKEN 2026-09-09 at HEAD `90a995e8`… the acceptance line is NOT EVIDENCE"
  section (`D:/wt/threadpool/docs/threadpool/KE16-RESULTS.md`, 2241 lines vs 1349). **Every wave
  shape in the design must be read against a pool whose parallel physics route is not yet shipped.**
- Memory `reference-nested-scope-serialises-on-worker`: `par_iter` inside a system measured 1.01× vs
  7.69× outside — the same defect from the ECS side. [D] project memory, not re-measured here.

## 10. Determinism across engines

| Engine | Contract | Mechanism | Source |
|---|---|---|---|
| Box2D v3 | "deterministic across thread counts and platforms… Multithreaded determinism is achieved by basing simulation order on creation order… Determinism is on by default and there is no explicit option to disable it" | creation order for bodies/shapes/joints; events included | [D] `docs/simulation.md` |
| Jolt | same-order API calls + same binary; `PhysicsSettings::mDeterministicSimulation` default on; `CROSS_PLATFORM_DETERMINISTIC` ~8% cost, validated MSVC/clang/gcc × x86/ARM/RISC-V/PowerPC/LoongArch/WASM; needs `/fp:precise`, `-ffp-contract=off`, own `Sin`/`Cos`/`Hash`, `QuickSort` not `std::sort`, fixed FPU state. Explicitly NON-deterministic: query result order, listener callback order, `GetActiveBodies` order | lock-free `ManifoldCache` (two `LockFreeHashMap`s, double-buffered) restored to order by `SortContacts` / `GetAllBodyPairsSorted` | [D] `Architecture.md`, [S] `ContactConstraintManager.h` |
| Rapier | local by default; `enhanced-determinism` for cross-platform, **incompatible with `simd8`** ("the latter affects SIMD lane width and creates its own determinism domain"); insertion order preserved; math via nalgebra `RealField` | | [D] https://rapier.rs/docs/user_guides/rust/determinism/ |
| Unity Physics | STATELESS by design ("forgoes this caching in favor of simplicity and control"), rollback-friendly; Havok for Unity is deterministic but stateful | no warm-start cache | [D] design.md; [D/B] Unity blog |
| Blast / Chaos / PhysX GPU | none stated | — | [D] |
| **boyko** [T] | run-to-run + `{1, N}`-worker bit-identity on one binary/machine with serialized spawn (IM-2); SIMD==scalar as the 0%-gate instrument; no cross-platform claim (`tests/softstep.rs:545-560`, `bodytype_determinism_golden.rs:37-40`) | canonical `(manifold, point)` warm store; disjoint writes; exact ops; four source censuses | `colored.rs:67-80`, `:2634-2660`; `FMA-DETERMINISM.md:86-110` |

Two consequences the design carries: (a) the 8-wide cohort is PART of the contract (Rapier's rule);
AVX-512 would be a separate determinism domain, never an opportunistic upgrade (pitfall P-45); (b)
Unity's statelessness would discard the warm starting TGS-Soft convergence depends on, for a
rollback property nothing here asks for (pitfall P-44).

## 11. GPU physics

- **PhysX 5** [D] `GPURigidBodies.html` 5.4.1: GPU broadphase, contact generation (PCM, convex ≤64
  verts), body management and the constraint solver; **D6 joints have full GPU shaders, other joint
  types run on CPU; joint projection, CCD and triggers are NOT GPU-accelerated; buffers cannot grow
  dynamically** (`PxSceneDesc::gpuDynamicsConfig`); direct GPU API `copySoftBodyData` /
  `applySoftBodyData`. Dynamic triangle meshes are supported ONLY with a cooked SDF and only on the
  GPU pipeline; sparse SDF `subgridSize` 4–8 recommended; **static actors with SDFs are not
  supported**; too-low resolution misses thin features [D] `RigidBodyCollision.html` 5.4.0 (pitfall
  P-41).
- **Boyko plan** [T]: CPU/GPU is a population partition — "where the DECISION runs vs where the
  BYTES live"; CPU MUST own branchy/low-N/decision logic; a CPU system touching GPU-resident bytes is
  forbidden (`RENDER-PHYSICS-GPU-PLAN.md:82-95`); rigid stays CPU; SP5 GPU soft past a MEASURED
  break-even (`OPTIMIZATION-PLAN-PHYSICS.md:312-315`, `RESEARCH-SOFT-BODY.md:64-68`); the SDF
  hand-off is "CPU-authoritative analytic field now, CPU-resident brick collision proxy later, never
  per-frame readback" (`OPTIMIZATION-PLAN-PHYSICS.md:95-101`). `boyko_physics` contains no
  `GpuColumn` reference (grep, 2026-09-10). The shader eDSL already instantiates one generic body
  over `f32` (host oracle) and `Emit` (HLSL) (`CLAUDE.md` layout table) — the engine's own mechanism
  for a CPU/GPU bit-identical kernel, which no surveyed engine has for soft bodies (pitfall P-40).

## 12. Comparative table

| Aspect | Box2D v3 / Box3D | Jolt | Rapier 2026 | Avian 0.4 | PhysX 5 | Chaos | **boyko today** [T] |
|---|---|---|---|---|---|---|---|
| Solver | Soft Step (substep + soft + relax + warm) | SI + warm start, 10+2 iterations, islands | staged island, coloured, persistent workers | substepped, coloured, solver bodies | TGS (recommended) | PBD-family evolution | TGS-Soft, 4 × (1+2) |
| Joints in the coloured graph | **yes**, same-pair rule, scalar | islands/splits | **no** — worker-0 stage | yes | GPU D6 only | per island | **none** |
| Colouring / islands | persistent, incremental | island splitter (>128) | persistent per-colour buckets | persistent | internal | internal | **rebuilt every step** |
| Dispatch | resident workers, CAS ring, atomic stage bits | jobs + barriers | persistent workers + spin barriers | Bevy task pool | GPU | task graph | `pool.scope` waves from a worker (serial floor) |
| SIMD | AoSoA 4-wide (Box3D) / 8-wide AVX2 (Box2D), contacts only | per-constraint | per-colour packed, padding lanes | planned | GPU | ISPC | AVX2 8-wide cohorts, contacts only |
| Determinism | thread-count + platform, creation order | same-order + binary; opt-in cross-platform | local; `enhanced-determinism` excludes `simd8` | not stated | not stated (GPU) | not stated | `{1, N}` bit-identity, same binary |
| CCD | speculative + TOI hybrid, bullet flag, serial bullets | LinearCast + time stealing | nonlinear TOI, motion clamping | — | eENABLE_CCD (CPU) | — | **none** |
| Character | geometric mover (planes), capsule, no rotation | `Character` / `CharacterVirtual` + inner body | — | — | — | — | **none** |
| Vehicle | wheel joint (no subsystem) | constraint + step listener, cast testers | raycast controller | — | component sequences | — | **none** |
| Soft / cloth | — | XPBD shared settings + baked groups, cooperative claiming | — | — | FEM, GPU-only, dual tet mesh | XPBD cloth + level sets | XPBD SP1–SP4, **30 `Vec` fields** |
| Destruction | — | — | — | — | Blast: chunk tree + CSR bonds + damage programs + stress solver | connection graph + strain + propagation, level sets | **none**; SDF analytic list capped at 16 |
| SDF collision | — | — | `Voxels` collider | — | cooked SDF, GPU-only, sparse | level sets for fragments | first-class analytic narrowphase, zero readback |
| Collider shapes | sphere/capsule/box/polygon/segment/chain | sphere, box, capsule, cylinder, convex hull, mesh, heightfield, compound | ball/cuboid/capsule/hull/trimesh/heightfield/voxels | Parry set | sphere/capsule/box/convex/mesh/heightfield/SDF | analytic + level set | **`Sphere \| Box` — the COMPLETE enum** (`components.rs:127-145`) |
| Ray / shape queries against bodies | `b2World_CastRay*`, `b2World_CastShape` | `CastRay` / `CastShape` / `CollideShape` | `QueryPipeline` | Parry | `PxScene::raycast`/`sweep` | trace API | **none** — SDF gradient sampling only |
| Contact / sensor EVENTS | `b2World_GetContactEvents`, sensor events | contact listener + `CollectCollidingBodies` | event handler | Bevy events | simulation event callbacks | notify | **`Contact` component exists with NO producer** (`components.rs:191-205`, grep) |
| Filtering | category/mask + joint `collideConnected` (edge-list walk, `body.c`) | `ObjectLayerFilter` + ragdoll group filter | groups + solver flags | layers | filter shader | channels | **`Collider.{layer, mask}` declared and read by NOTHING** (`components.rs:150-168`) |
| Motion-type switching / per-body sleep | `b2Body_SetType`, `enableSleep`, `sleepThreshold` | `SetMotionType`, per-body `AllowSleeping` | per-body `ccd`, sleeping | — | `setRigidBodyFlag` | — | **neither**; `IslandSleep` is per-world, and a runtime mass-regime flip is documented as unsafe (`resources.rs:3306-3316`) |

⚠ The last five rows were added in the critique round: the design's §14.C omission register needs a
survey line to point at, and "boyko today = none" is the fact each of those scope calls turns on.

## 13. Pitfalls register

Each entry: the pitfall, the source, and the consequence it imposes on THIS engine.

| # | Pitfall | Source | Consequence here |
|---|---|---|---|
| P-1 | Parallelising a solve that is DISPATCH-bound | KE16-RESULTS `:286-316` [T]; Box2D/Rapier resident workers [S] | wave COUNT is the variable; no rung may add waves per step |
| P-2 | A nested `pool.scope` issued from a worker serialises on the shipped pool | KE16 `:286-300` [T]; memory | every "parallel" claim below is conditional on the KE16 barrier |
| P-3 | Rebuilding islands and colours every step is a serial prologue that grows with the scene | Box2D islands post [B] | a persistence fork precedes joints (design D-1) |
| P-4 | A colorer that only sees contacts creates a same-pair race when joints arrive | `box2d/src/constraint_graph.c` [S]; P23 [T] | joints and contacts must be coloured in ONE occupancy pass |
| P-5 | Kinematic bodies coloured like statics corrupt/stall the SIMD scatter | `box3d/src/constraint_graph.c` [S]; P22 [T] | kinematic rows occupy colours once a joint can touch them |
| P-6 | Reduced-coordinate articulations do not colour | PhysX, Rapier [D] | maximal coordinates only (P24) |
| P-7 | Full-coordinate joints visibly break when the solver does not converge | Rapier [D] | stretch is an artefact, worst on long chains + mass ratios; ragdoll authoring rule |
| P-8 | Adjacent mass ratios >~10:1 jitter | Unity [D] | a Gaia bake contract on joint graphs |
| P-9 | Ragdolls converge better when ROOT constraints are solved LAST in the sweep | Jolt [S] `Jolt/Physics/Ragdoll/Ragdoll.cpp` `RagdollSettings::CalculateConstraintPriorities`: `// Calculate priority for each part. Start with the base priority and increment towards the root`; [S] `Jolt/Physics/Constraints/ConstraintManager.cpp` `sSortConstraints`: `return lhs->GetConstraintPriority() < rhs->GetConstraintPriority()`, iterated forward ⇒ root = highest priority = sorted LAST = solved last | ⚠ **restated in the critique round; the first version said "joint order within a colour = root-first by baked depth" and was doubly wrong.** (a) It is inverted: root-first assignment makes root joints take the LOWEST free colours in a first-fit colorer, i.e. solved FIRST. (b) Order WITHIN a colour cannot carry it at all — the bodies in one colour are pairwise disjoint (that is the `{1, N}` argument), so no joint there sees another's result; sweep priority lives in the COLOUR INDEX. Honouring P-9 here would mean appending ragdoll joints **leaf-first** so root joints first-fit into the highest colours — a value change to the colouring, deferred behind the ragdoll-settling fixture at design rung R |
| P-10 | One joint per logical ragdoll joint may not stabilise | Unity `PhysicsJointCompanion` [D] | allow N joint rows per bone pair |
| P-11 | Soft rotation limits are not shipped anywhere | Jolt [D]; Box3D hard limits [S] | cone/twist limits are hard + speculative, never "soft by reuse" |
| P-12 | A soft joint above half the substep rate is unstable by construction | Jolt range; Box3D clamp [S] | clamp in PREPARE, not in authoring, so a `substeps` change cannot destabilise |
| P-13 | The single bias-free restitution sweep under-resolves coupled constraints | `soft_step.rs:664-668` [T] | joints add a second coupling channel through the same bodies |
| P-14 | Parallel bullet-vs-bullet CCD is order-non-deterministic | `box3d/src/solver.c` [S] | bullets serial, mandatory under `{1, N}` |
| P-15 | Speculative contacts ghost-collide on internal edges | Firth 2011 [B] | any SDF/mesh path inherits it; tiled floors need edge-normal filtering |
| P-16 | Time stealing / motion clamping is visible | Jolt [D]; Rapier [D] | no free CCD — choose the artefact |
| P-17 | The character mover sits OUTSIDE the solver by design | Box3D doc, Jolt [D] | Principle 0 forces its STATE into components even though its LOGIC is a query pass |
| P-18 | The mover has no rotation handling | Box3D doc [D] | rotating platforms need Jolt's local coordinate system |
| P-19 | Joint SIMD widening has no precedent and a thin payoff | `box3d/src/core.h` [S]; `MIN_PARALLEL_SLOTS_PER_COLOR` [T] | joints scalar in v1; cohort rung measured-gated |
| P-20 | Breaking joints from accumulated impulse is frame-lagged and order-sensitive | Jolt #141, Box3D [S] | only a bit-identical impulse readout makes breaking deterministic |
| P-21 | Jolt's iteration numbers are not transferable | `PhysicsSettings.h` [S]; Solver2D [B] | never copy 10+2 into a substep budget |
| P-22 | Baumgarte vs soft is settled | Solver2D [B] | no Baumgarte term for joints |
| P-23 | Float precision far from the origin | Solver2D [B]; Box3D re-centred sweeps [S] | anchor lever arms as differences, sweeps re-centred |
| P-24 | Joint state has a per-joint lifetime, unlike the per-frame contact table | Box3D `b3JointSim` [S]; `warm_start.rs` [T] | durable joint columns, no hash table |
| P-25 | Positive separation through today's kernel is stopped ~26× too early — **and the soft mass/impulse scalings are applied unconditionally, so a bias-only fix does not restore "arrive exactly"** | `soft_step.rs:569-581` [T] (arithmetic); [S] `box2d/src/contact_solver.c` | the `C·inv_h` regime precedes speculative contacts, and it is a THREE-variable change (`bias`, `mass_scale`, `impulse_scale`) |
| P-25b | `MAX_BIAS_VELOCITY` does not cap what its name says: the shipped kernel clamps the bias BEFORE the `massScale` multiply, Box2D clamps after | `soft_step.rs:569-577` [T]; [S] `box2d/src/contact_solver.c` (`b2MaxFloat( softness.massScale * softness.biasRate * s, -contactSpeed )`) | effective cap ≈ `mass_coeff × 4.0 ≈ 3.8` m/s; a pre-existing value bug the X0 rung must either fix under its gate or pin with a comment |
| P-25c | A "verbatim from engine X" transcription that keeps a divergence from engine X in the same three lines | this round | any kernel claiming a verbatim source states the divergences it retains |
| P-26 | Element colouring costs dispatch waves (7–76 vs 3–8) | VBD [P] | bend (4-arity), LRA, skinned each add a colouring |
| P-27 | A colored path can never be bit-compared to the serial one | `soft/colored.rs:14-20` [T] | every new constraint type gets its own oracle, not a golden |
| P-28 | Tearing invalidates every precomputed structure at once | `soft/component.rs:492-585`, `:596-603` [T] | tearing is out of the ladder; SDF carving covers the visual |
| P-29 | Pinning by `inv_mass == 0` is not a skinned constraint | Jolt `Skinned` [S]; `ANIMATION-DESIGN-SPACE.md:493-502` [T] | skinned + backstop + LRA are the cloth primitive |
| P-30 | VBD's speed levers are order-dependent | VBD [P]; `soft/colored.rs:12-13` [T] | family stays XPBD; VBD numbers not transferable |
| P-31 | Mass ratios break position-based coupling first | VBD [P]; `soft/coupling.rs:6-9` [T] | single-deepest-contact coupling is an accuracy ceiling |
| P-32 | Soft↔soft collision is unsolved in shipping engines | Jolt [D]; `self_collision.rs:11-16` [T] | not a small extension of the hash |
| P-33 | The rest-state op-sequence discipline is load-bearing | `soft/component.rs:339-361` [T] | every new constraint computes its rest value with the solve's op sequence |
| P-34 | A distance+volume tet cannot recover from inversion | `soft/component.rs:44-47` [T]; MIG 2021 [P] | neo-Hookean pair as the second projector |
| P-35 | No precedent for a CPU/GPU bit-identical soft body | PhysX GPU-only [D] | the eDSL is the mechanism; the burden is boyko's |
| P-36 | Self-collision determinism lives in candidate EMISSION order | `self_collision.rs:27-35` [T] | vertex-face re-opens the proof |
| P-37 | Jolt's soft `Optimize()` dumps cross-group constraints into a final catch-all | `SoftBodySharedSettings.cpp` [S] | "colour count ≈ parallel width" is false; the overflow dominates the tail |
| P-38 | PBD/XPBD duplication is a compatibility tax | Chaos [D] | do not import |
| P-39 | Simulating every chunk as a body from t=0 | Chaos optimisation page [D]; `particle.rs:5-6` [T] | chunks are ROWS; promotion is budgeted |
| P-40 | Churn changes slot values → colouring → floats | `DENSE-COMPONENTS-PLAN.md:56-58` [T] | fracture ops are part of the recorded op sequence |
| P-40b | **A dense slot index is not an authored order.** Slots come from a LIFO free list ("insert pops free LIFO or pushes len; tombstone pushes free"), so after any despawn the next inserts fill freed slots in REVERSE — a second ragdoll spawned where the first was destroyed has its joints permuted against a fresh world | `DENSE-COMPONENTS-PLAN.md:56-58` [T] | any ordering PROPERTY over dense-stored constraints (sweep order, break readout, "contacts then joints") must be stated over an explicit authored key sorted once into the prepared form — never over the slot. Joint spawn / despawn / break are themselves ops in the recorded sequence |
| P-40c | **A transcendental slips past an "exact ops" rule because the census does not know its name.** The four censuses scan `_mm256_`/`_mm_` × `{fmadd…rsqrt, rcp}` + `mul_add(` + `algebraic_` (`systems.rs:1560-1572` [T]); `f32::acos` matches none. No transcendental exists in the production physics crate today (every `.sin()`/`.cos()` hit is `#[cfg(test)]`), so the first cone/twist kernel would be the first — silently | `systems.rs:1548-1611` [T]; grep [T]; [S] `box2d/src/math_functions.c` (`b2ComputeCosSin`: *"Approximate cosine and sine for determinism… However, I don't trust this result"*, `b2Atan2`) | either in-house polynomials under the same discipline (Box2D's answer, and the design's KR-2b) or an explicitly enumerated census exception. libm is deterministic within one binary, so `{1, N}` survives either way — the rule that breaks is "exact ops", which is the stronger one this crate actually claims |
| P-40d | **Fields that exist and are read by nothing read as capabilities.** `Collider.{layer, mask}` are declared with a filter semantics in their doc comment and consumed nowhere in the crate (`components.rs:150-168` says "wired for the Phase-10 broadphase"; the only `.mask` hits are hash-table masks in `axis_cache.rs`/`warm_start.rs`); a design leaned on them for ragdoll self-collision filtering | `components.rs:150-168` [T]; grep [T]; [S] `box2d/src/body.c` `b2ShouldBodiesCollide` (Box2D filters by walking the body's joint edge list) | a filtering rung must land before ragdolls; and a `u32` layer bit caps a per-ragdoll scheme at 32 ragdolls, which a crowd fixture exceeds by construction |
| P-41 | Mutating topology mid-solve | Blast three phases [D]; IM-1 [T] | generate/apply/split in the apply window via `Commands` |
| P-42 | The solver does not expose strain | `colored.rs:858-925` [T] | a per-manifold impulse export is a new seam |
| P-43 | `Sphere \| Box` cannot represent shards | `components.rs:127-145` [T] | hull collider precedes real fracture |
| P-44 | Interior faces are a rendering contract | Blast, Chaos [D] | ≥2 material slots per fractured asset |
| P-45 | Runtime booleans on artist meshes | Blast authoring rules [D]; §16 [T] | cut only convex / voxel / SDF |
| P-46 | The SDF edit list is not a destruction log | `lib.rs:103-105`, `:275-278` [T] | carving needs a WRITTEN (brick) representation |
| P-47 | Mixing budgets | `sdf-engine-architecture.md:408-426` [T] | every destruction claim names CPU or GPU |
| P-48 | Sleeping must absorb debris | `resources.rs:3063-3096` [T] | `IslandSleep` migration before debris |
| P-49 | SIMD lane width is a determinism domain | Rapier [D] | AVX-512 is a separate domain |
| P-50 | Statelessness discards warm starting | Unity [D] | rejected |
| P-51 | Lock-free caches need a canonical order | Jolt `SortContacts` [S]; `colored.rs:67-80` [T] | any new table follows the canonical-store rule |
| P-52 | Two measurement sessions do not make a ratio | `OPEN-QUESTIONS.md:4977-4996` [T] | every A/B in one window |
| P-53 | Numbers that could not be verified | §16 | not entered as measured |

## 14. Academic works (with what each is cited for)

- Macklin, Müller, Chentanez — XPBD, MIG 2016 — compliance α; α=0 ⇒ PBD.
- Macklin, Storey, Lu, Terdiman, Chentanez, Jeschke, Müller — Small Steps, SCA 2019 — substeps beat
  iterations (cloth timings UNVERIFIED, §16).
- Macklin, Müller — Stable Neo-Hookean constraints, MIG 2021 — inversion-robust pair.
- Müller, Macklin, Chentanez, Jeschke, Kim — Detailed Rigid Body Simulation with XPBD, CGF 2020,
  DOI 10.1111/cgf.14105 — the positional joint pole (competitor formulation to soft-step).
- Bender, Müller, Otaduy, Teschner, Macklin — PBD survey, CGF 2014 — bending, strain limiting,
  plasticity, tearing formulations.
- Müller, Heidelberger, Teschner, Gross — Shape matching, SIGGRAPH 2005; Müller et al. — Physically
  Based Shape Matching, SCA 2022 (pointer only).
- Kim, Chentanez, Müller-Fischer — Long Range Attachments, SCA 2012.
- Bergou, Wardetzky, Harmon, Zorin, Grinspun — Quadratic Bending, SGP 2006.
- Bridson, Fedkiw, Anderson — Robust cloth collisions, SIGGRAPH 2002.
- Müller, Chentanez, Kim, Macklin — Air Meshes, ToG 2015.
- Teschner et al. — Optimized Spatial Hashing, VMV 2003.
- Chen, Liu, Yang, Yuksel — Vertex Block Descent, ToG 2024; Giles, Diaz, Yuksel — Augmented VBD, ToG
  2025, DOI 10.1145/3731195.
- Tonge, Benevolenski, Voroshilov — Mass Splitting, ToG 31(4) 2012, DOI 10.1145/2185520.2185601 —
  jitter-free parallel Jacobi if colouring ever runs out.
- Chen et al. — many-core physical simulation (cited by Jolt's `LargeIslandSplitter.h`).
- Catto — Soft Constraints (GDC 2011), Continuous Collision (GDC 2013), Solver2D (2024), SIMD
  Matters (2024), Simulation Islands (2023), Box3D announcement (2026).
- Kugelstadt & Schömer — Cosserat rods, SCA 2016 — UNVERIFIED (cited from Jolt's naming).
- Müller, Chentanez, Kim — VACD fracture, SIGGRAPH 2013.
- Sellán et al. — Breaking Good (fracture modes), ToG 2023.
- Parker & O'Brien — Real-Time Deformation and Fracture, SCA 2009.
- Wei, Liu, Ling, Su — CoACD, SIGGRAPH 2022.
- Rong & Tan — JFA, I3D 2006 (mesh→SDF baking; cited by `sdf-engine-architecture.md`).
- Macklin et al. — Local Optimization for Robust SDF Collision, I3D 2020 (pointer).
- Parallel Block Neo-Hookean XPBD via Graph Clustering, MIG 2022 (pointer).

## 15. Sources

**External** — Box2D/Box3D: https://github.com/erincatto/box3d (`src/joint.c`,
`spherical_joint.c`, `revolute_joint.c`, `constraint_graph.c`, `solver.c`, `core.h`,
`include/box3d/types.h`), https://box2d.org/posts/2026/06/announcing-box3d/ ,
https://box2d.org/documentation3d/ , https://box2d.org/documentation3d/md_character.html ,
https://raw.githubusercontent.com/erincatto/box2d/main/src/{constraint_graph.c,constraint_graph.h,solver_set.h,solver.c,solver.h,joint.c} ,
https://raw.githubusercontent.com/erincatto/box2d/main/docs/simulation.md ,
https://box2d.org/posts/2024/02/solver2d/ , https://box2d.org/posts/2024/08/simd-matters/ ,
https://box2d.org/posts/2023/10/simulation-islands/ , https://box2d.org/posts/2024/08/releasing-box2d-3.0/ ,
https://box2d.org/posts/2025/04/box2d-3.1/ , GDC 2011 / GDC 2013 PDFs above. Jolt:
https://github.com/jrouwe/JoltPhysics (`Docs/Architecture.md`, `Docs/PerformanceTest.md`,
`Jolt/Physics/PhysicsSettings.h`, `LargeIslandSplitter.h`, `SoftBody/SoftBodySharedSettings.{h,cpp}`,
`SoftBody/SoftBodyMotionProperties.{h,cpp}`, `Constraints/ContactConstraintManager.h`,
`Ragdoll/Ragdoll.h`, `Vehicle/VehicleConstraint.h`, `Character/CharacterVirtual.cpp`,
`Core/JobSystem.h`), https://jrouwe.github.io/JoltPhysics/ , JoltPhysicsDocs 5.0.0 / 5.1.0 class
pages (`SoftBodySharedSettings`, `SixDOFConstraintSettings`, `Ragdoll`, `VehicleCollisionTester`),
discussion #141, https://jrouwe.nl/jolt/JoltPhysicsMulticoreScaling.pdf (chart-based, no numbers
extracted). Rapier: https://rapier.rs/docs/user_guides/rust/{joints,rigid_bodies,determinism}/ ,
https://raw.githubusercontent.com/dimforge/rapier/master/src/dynamics/solver/{solver_contact_graph.rs,staged_island_solver/mod.rs,staged_island_solver/worker.rs} ,
`impulse_joint/impulse_joint_set.rs`. Avian: https://joonaa.dev/blog/09/avian-0-4 . Unity:
com.unity.physics 1.4 `design.md`, `PhysicsJoint` API, 0.6 getting_started; Unity 6 RagdollStability
manual; Unity blog on Havok. PhysX 5: `SoftBodies.html` 5.4.1, `GPURigidBodies.html` 5.4.1,
`RigidBodyCollision.html` 5.4.0, `Joints.html` 5.4.0, `Vehicles.html` 5.3.0, `Articulations.html`
5.5.0 under https://nvidia-omniverse.github.io/PhysX/physx/ . Blast:
https://nvidia-omniverse.github.io/PhysX/blast/ (index, api/introduction, api_ll_users_guide,
api_hl_users_guide, extensions/ext_stress, extensions/ext_authoring),
https://raw.githubusercontent.com/NVIDIA-Omniverse/PhysX/main/blast/include/lowlevel/NvBlastTypes.h .
Chaos: chaos-destruction-overview (4.27), python-api `GeometryCollection` / `ClusterConnectionTypeEnum`
(5.4), fracturing-geometry-collections-user-guide, chaos-destruction-optimization,
panel-cloth-editor-overview, `ChaosClothAssetDataflowNodes` API index;
https://notes.hzfishy.fr/Unreal-Engine/Physics/Types/Geometry-Collection . Bullet:
`src/BulletDynamics/Vehicle/btRaycastVehicle.cpp`. bepuphysics2 `Documentation/Substepping.md`.
Havok Cloth vendor page. Smash Hit / Teardown: https://blog.voxagon.se/ . Media Molecule SIGGRAPH
2015; GDC 2018 Aaltonen PDF; GDC Vault talks (The Finals, Rainbow Six Siege, UE5 dynamic
destruction). Papers: the DOIs and arXiv ids in §14.

**In-tree** — every `file:line` in §0, §6, §8.3, §9, §10, §13 (ecsnative for `crates/boyko_physics`,
`crates/boyko_threadpool`, `crates/boyko_ecs/.../dense`, `crates/boyko_math`,
`docs/threadpool/*`, `docs/OPEN-QUESTIONS.md`, `docs/physics/FMA-DETERMINISM.md`; main checkout for
`docs/animation/*`, `docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md`, `docs/DENSE-COMPONENTS-PLAN.md`,
`docs/OPTIMIZATION-PLAN-PHYSICS.md`, `docs/RENDER-PHYSICS-GPU-{PLAN,RESEARCH}.md`,
`docs/RESEARCH-SOFT-BODY.md`, `docs/sdf-engine-architecture.md`, `docs/gaia/*`,
`docs/aether-v2/MACHINES.md`, `crates/boyko_sdf_math/src/*`, `crates/boyko_rhi_vulkan/src/brick_atlas.rs`,
`crates/boyko_render/src/particle.rs`, `crates/boyko_serialize/src/format.rs`).

## 16. Unverified — do not enter as measured

> Rev 2 (2026-09-10): §17 is the critique log for this document. Five entries below are new to it.

- Small Steps cloth timings (150k particles / 896k springs; 1×30 at 12.4 ms vs 30×1 at 13.5 ms):
  from a search summary; `mmacklin.com/smallsteps.pdf` would not extract.
- Every AVBD figure: both PDFs exceeded the fetch limit. The claims quoted are the project page's
  prose.
- Physically Based Shape Matching (SCA 2022), the VACD fracture PDF, the Claybook GDC 2018 slides,
  the Parker & O'Brien timing table, the Jolt multicore-scaling PDF: pointers only.
- Havok Destruction: no technical documentation found at all.
- Kugelstadt & Schömer 2016: cited from Jolt's constraint naming, not opened.
- Box2D's exact `B2_GRAPH_COLOR_COUNT` value: not in the fetched excerpt (Box3D's 24 was).
- UE ISM add/remove regressions in 5.4/5.5: community-reported.
- `KE16-RESULTS.md:1668` "Step App taken", as cited by `ANIMATION-DESIGN-SPACE.md` AK-9: the
  ecsnative copy is 1349 lines and reads "App: NOT RUN — OWED"; the threadpool worktree's copy
  carries a "§App. TAKEN 2026-09-09" heading whose numbers are declared NOT EVIDENCE; the memory
  note of 2026-09-10 records the retake as still owed pending a rustc bracket. Read the status from
  the threadpool worktree at the time of use.
- The "1–10 µs bodies" phrasing in the campaign brief maps onto KE16's `worker/1us_*` and
  `worker/10us_*` grid cells; no physics-specific per-body timing was read for this record.
- The memory note `reference-nested-scope-serialises-on-worker` (1.01× vs 7.69×) was not
  re-measured in this pass.
- **Box3D's own `spherical_joint.c` speculative block was quoted from the first pass's reading, not
  re-fetched in the critique round.** What WAS re-fetched (2026-09-10) is Box2D's contact form,
  `b2SolveContacts_Overflow` in `box2d/src/contact_solver.c`, which is the same regime and is the
  version §2.3 and §6 now quote verbatim. If a Box3D joint kernel is ever transcribed line-for-line,
  re-open `spherical_joint.c` rather than trusting the Box2D analogue for the joint-specific parts.
- **Box3D's `B3_GRAPH_COLOR_COUNT = 24` is from the first pass's excerpt; Box2D's
  `B2_GRAPH_COLOR_COUNT` was never in a fetched excerpt** (already noted above). The design's D-14
  discussion depends only on the fact that a fixed palette EXISTS upstream and does not exist here,
  which both excerpts support.
- **The ULP behaviour of any in-house transcendental (P-40c, design KR-2b) is unmeasured.** Box2D's
  own comment records distrust of platform `cosf`/`sinf` without publishing an error bound, and
  Bhāskara I's approximation has a known but unquoted-here maximum error.
- **No claim in this survey about boyko's per-joint or per-transcendental COST is measured.** The
  design's flop counts are [ESTIMATE]s and this document does not corroborate them.

## 17. Critique log (round 1, 2026-09-10)

The critique round addressed to this survey, with dispositions. Four upstream files were fetched
fresh for it (2026-09-10) — [S] `box2d/src/contact_solver.c`, [S] `box2d/src/math_functions.c`,
[S] `box2d/src/body.c`, [S] `JoltPhysics/Jolt/Physics/Ragdoll/Ragdoll.cpp`, [S]
`JoltPhysics/Jolt/Physics/Constraints/ConstraintManager.cpp` — and the [T] claims were re-checked
against `D:/wt/ecsnative` at `05fcbd1d` (= the header's `46c8e489` plus one docs-only commit; `git
diff --stat` names a single Markdown file, so no line number moved). **No timing was taken.**

| # | Finding against this document | Disposition |
|---|---|---|
| C-1 / §6, §2.3 | "the joint-limit form is the same regime: `C > 0 ⇒ bias = C·inv_h`" omits `massScale = 1` / `impulseScale = 0`, and the shipped kernel applies the soft scalings unconditionally — so a reader (and the design) takes a bias-only branch to be the whole regime | **FIXED** in §2.3 and §6: the Box2D block is quoted verbatim, the three variables are named, and the consequence at boyko's own defaults (`mass_coeff ≈ 0.95`, `impulse_coeff ≈ 0.05`) is stated. P-25 restated |
| C-1b | Box2D multiplies the soft bias by `massScale` inside the clamp; boyko does not — a second, pre-existing divergence | **FIXED** — recorded in §2.3 and as new pitfall **P-25b**, with the numeric consequence (`MAX_BIAS_VELOCITY = 4.0` caps at ≈3.8 m/s). New **P-25c** generalises the class |
| C-2 / §9 | the survey's stage list is right, but it did not state what boyko's OWN loop does, which is how the design came to invent a per-colour warm-start wave | **FIXED** — a new §9 bullet walks `colored.rs:3259-3316` and says in terms that only `solve_all_colors` dispatches, and why Box2D's stage order is a value constraint |
| C-3 / P-9 | "joint order within a colour = root-first by baked depth" inverts Jolt (priority increments TOWARD the root, sorted ascending ⇒ root solved LAST) and puts the property on an axis that cannot carry it (bodies in a colour are disjoint) | **FIXED** — P-9 rewritten with both source quotations and both errors named; the consequence is now a leaf-first colour-index question deferred behind a fixture |
| C-5 / §0, §12 | `Collider.{layer, mask}` are declared and unread; a design leaned on them | **FIXED** — new pitfall **P-40d** with the grep result and Box2D's `b2ShouldBodiesCollide` edge-list walk; new §12 "Filtering" row |
| C-6 / §2.3 | the ConeTwist kernel this survey transcribes needs `acos`/`atan2`/`sin`/`cos`, which no censused rule covers and which do not exist in the crate | **FIXED** — new pitfall **P-40c** with the census stems, the test-only grep result, and Box2D's `b2ComputeCosSin` / `b2Atan2` comment. §16 records that the ULP bound is unmeasured |
| C-7 / P-40 | P-40 covers "churn changes slot values", but not the sharper fact that a dense slot is not an ORDER at all | **FIXED** — new pitfall **P-40b** |
| C-NB9 / §0 row 3 | "the survey should say the `Kinematic` bit is unread today" | **FIXED, locator corrected** — the sentence the critique quoted ("velocity feeds the one-sided contact response") is in the DESIGN at §4, not in this document's §0 row 3; §0 row 3 said only that kinematic MOTION is deferred. The stronger fact — the bit has no consumer, and the one-sidedness belongs to `inv_mass == 0` — is now written into that row with the grep and `contact.rs:81-131` |
| C-NB10 / §12 | seven capabilities every surveyed engine ships that the design did not rule on | **FIXED** — five new §12 rows (collider shapes, ray/shape queries, contact & sensor events, filtering, motion-type switching / per-body sleep), each ending in the "boyko today" fact the design's new §14.C register turns on |
| C-11 / §2.4 | the design's D-14 reason does not distinguish this engine from Box2D | **FIXED here too, and sharpened** — §2.4 now says the top-down search is about Gauss-Seidel ORDER, and adds the obstacle the critique did not name: **boyko has no palette at all**, `color_manifolds` growing colours unboundedly, so "reserved high colours" presupposes a constant that does not exist |

**Refuted: none outright. One partial** — C-NB9's locator, above; its substance was accepted.
**Strengthened beyond the finding: two** — C-11 (the missing palette) and C-1b (the numeric
consequence of the clamp divergence).

Everything in this log is a change to the RECORD. Nothing here was measured, no gate was run, and
the survey's standing caveat is unchanged: **every wave-shape claim is read against a pool whose
parallel physics route is not shipped** (§9, §16).
