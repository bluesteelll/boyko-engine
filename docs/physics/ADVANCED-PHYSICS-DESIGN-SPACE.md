# Advanced physics — the design for THIS engine

> Status: architect's design, 2026-09-10, **rev 2 — after one critique round; the log is §16**, and
> every finding there is answered in place. The survey it rests on
> is [`ADVANCED-PHYSICS-RESEARCH.md`](ADVANCED-PHYSICS-RESEARCH.md) (§0 there is the inventory of
> what the tree holds; §13 there is the pitfalls register this design answers by number, `P-n`).
> Every `file:line` below was re-opened at `D:/wt/ecsnative` HEAD `46c8e489`
> (rev 2 re-verified at `05fcbd1d`, which is `46c8e489` plus one docs-only commit — no code moved)
> (`feat/ecs-native-storage`) for `crates/boyko_physics`, `crates/boyko_threadpool`,
> `crates/boyko_ecs`, `crates/boyko_math` and `docs/threadpool`, and at `D:/claude/BoykoEngine`
> HEAD `128233be` (`feat/multi-paradigm-render`) for every other doc. Numbers are either measured
> elsewhere and cited, or **labelled estimates**; **no timing was taken for this document**. §14
> separates the PERF/ARCHITECTURE decisions taken here from the VALUES/SCOPE decisions that go to
> the owner. The register follows [`docs/animation/ANIMATION-DESIGN-SPACE.md`](../animation/ANIMATION-DESIGN-SPACE.md).

## 0. The shape in one paragraph

Everything on the ladder is **a second constraint KIND in the one constraint graph, or a second
ROW SPACE under the one colouring discipline, or a baked GRAPH the existing union-find already
knows how to cut** — nothing is a subsystem beside the solver. A **joint** is an ECS entity whose
definition is a table component and whose accumulated impulses are a *durable dense* component;
each step its two endpoints are resolved to dense rows, it is appended to the SAME edge stream
the colorer already first-fits over (contacts first, so a joint-free world is byte-identical —
the 0%-gate), it unions islands, and it is solved as scalar rows **inside the colour wave its
colour already owns** — so the step's wave count (72 today, `docs/OPEN-QUESTIONS.md:4996-5000`)
does not move by one. Ragdolls, vehicles and breakable welds are joint kinds plus a Gaia-baked
graph; the character controller is a query pass whose *state* is a component; CCD is a
speculative-separation regime the normal solve currently lacks plus a serial bullet lane. The
soft body's 30 `Vec` fields move into a resource-owned **row bank** with per-body ranges and a
Gaia-baked topology that carries its *own colouring* — after which faces, bending, tethers,
skinning and self-collision are additive constraint types on the same projector, and every soft
body in the world is one colour wave per constraint type instead of one per body. Destruction is
Blast's data model in ECS columns — a baked chunk tree + CSR bond graph, per-instance health
columns, chunks as ROWS promoted to entities under a count budget, bond breaks decided by a
bit-identical per-manifold impulse readout — with the SDF brick atlas as the *written* authority
for sculpted carving. The GPU is the last rung and only for the soft/continuum population, with the
projector written once as an eDSL leaf so the CPU oracle and the HLSL are one source. **Every
parallel claim below is conditional on the KE16 pool: on the shipped pool the worker-spawned
`pool.scope` route is the serial floor** (`docs/threadpool/KE16-RESULTS.md:286-300`).

## 1. The six facts every rung stands on

1. **One constraint TYPE exists.** `ConstraintGraph::build` consumes `&[Manifold]` and every array
   is manifold-keyed (`resources.rs:2399-2490`, `:2625-2635`); the colorer reads only `body_a` /
   `body_b` and the `is_dynamic_row` predicate (`resources.rs:2850-2960`, `contact.rs:36-38`).
   There are zero joints, capsules, sweeps, characters or vehicles (RESEARCH §0 census).
2. **The dispatch is the loss, not the solver.** 0.71× at W=16 against Jolt's 4.19× on the same
   pyramid, ≈1.1× single-threaded (`OPEN-QUESTIONS.md:4965-4996`); 72 waves/step; `Scope` has
   `spawn` and `spawn_batch` only (`crates/boyko_threadpool/src/scope.rs:944`, `:994`); the barrier
   belongs to the KE16 lane (`OPEN-QUESTIONS.md:5000-5010`). Corollary for this design: **no rung
   may add a wave per step**, and every "parallel" gate is stated as a value gate plus a
   per-worker touch count that is allowed to read one worker on this pool (the animation G2 shape,
   `ANIMATION-DESIGN-SPACE.md:582-587`).
3. **Bit-identity is a contract in three parts** (`docs/physics/FMA-DETERMINISM.md:86-110`,
   ecsnative): run-to-run, `{1, N}`-worker, and SIMD==scalar as the 0%-gate instrument; the colored
   value legitimately DIFFERS from the reference (`solver/colored.rs:20-45`). Every new constraint
   type inherits: exact ops only, the four source censuses, its OWN `{1, N}` oracle (never a golden
   against the serial path — `soft/colored.rs:14-20`), and a canonical-order store for anything
   that persists across frames (`colored.rs:67-80`). ⚠ **"Exact ops only" has a hole the censuses
   cannot see, and the first cone/twist kernel walks straight into it** — no transcendental exists
   in the production crate today and the censuses do not scan for one; §2.5's C-6 note settles it
   (KR-2b, in-house polynomials) rather than leaving the rule and the kernel contradicting each
   other.
4. **Durable per-element data lives in kernel storage.** `SoftBody`'s 30 `Vec` fields
   (`soft/component.rs:69-180`) and `IslandSleep`'s four (`resources.rs:3063-3096`) are the
   remaining exceptions, both the owner's call (`ARCH-AUDIT-ECS-DATA-REMEDIATION.md:16`, `:30`,
   `:43`; memory `project-ecs-native-physics-lane`). The kernel now has the dense kind —
   `DenseStore` / `DenseBuildView` (`!Send`) / `DenseSolveView` (`Copy`, `row_ptr` only)
   (`crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:83`, `dense/views.rs:29`, `:110`)
   — and `ScratchColumn` with its Build/Solve split (the SP4 fix, `colored.rs:348-366`).
5. **The gather is Encoding A**: row `i` == `BodyIndex(i)` in archetype-row order, no `EntityId`
   per row (`systems.rs:201-278`); IM-1 forbids structural change between gather and apply
   (`systems.rs:1127-1170`); the row→entity map for the `Contact` producer is deliberately absent
   (`resources.rs:3630-3640`). Joints, welds and chunk promotion all need that map.
6. **The scratch-id band has 38 free ids (421..384)** below the soft-graph cohort, each cohort a
   contiguous run ≤ `POOL_STAGGER_LINES` with a compile-time distinctness proof, and the floor
   moves with the lowest cohort (`scratch_ids.rs:126-200`, `:541-550`; memory). A new cohort either
   fits or moves `SCRATCH_REGION_MIN_ID` after re-running the census in that constant's doc.

## 2. The constraint kind — joints in the coloured graph (rungs J0–J4; closes AK-4)

### 2.1 Data model

Three components and one scratch cohort. A joint is an **entity** (so it has an id, can be
despawned, queried, serialized, and referenced by a ragdoll or fracture asset) carrying:

```
JointDef {                    // table component, #[repr(C)], ~248 B [ESTIMATE], authored / baked
    body_a: EntityId, body_b: EntityId,   // 2 × 8 = 16
    frame_a: (Vec3 anchor, Quat basis),   // 12 + 16 = 28; in body-local space, COM-relative
    frame_b: (Vec3 anchor, Quat basis),   // 28                        (Box3D shape)
    order:   u16,                         // 2  — THE authored joint key (see below); NOT the row
    kind: JointKind,                      // 1  #[repr(u8)]: Point | Weld | Distance | Hinge
                                          //    | Slider | ConeTwist | SixDof | Wheel | Motor
    flags: u16,                           // 2  collide_connected, kinematic_target, wheel bits …
    spring:  { hertz, damping_ratio }     // 8  0 hertz = maximum stiffness (Box3D weld semantics)
    limits:  [Limit; 6]                   // 6 × 12 = 72; per DOF: lower, upper, enabled bit
                                          //    (6DOF superset; a hinge reads one entry,
                                          //     a cone-twist reads 3)
    motor:   { target_velocity: [f32; 6], target_position: [f32; 6],
               max_force: [f32; 6], mode: u32 }        // 3 × 24 + 4 = 76
    break_force: f32, break_torque: f32,  // 8  +INF = unbreakable (J4)
}                                         // Σ = 241 B of fields; align 8 (EntityId = usize,
                                          // `primitives.rs:57`) ⇒ 248 B [ESTIMATE]
JointState {                  // #[component(storage = "dense")], #[repr(C)], 64 B [ESTIMATE]
    point: Vec3,              // point-to-point accumulator
    linear: [f32; 3],         // per-axis limit / spring accumulators (slider, 6DOF)
    angular_lo: [f32; 3], angular_hi: [f32; 3],   // lower/upper SEPARATE, never one signed (RESEARCH §2.3)
    spring: f32, motor: [f32; 2],
    broken: u32,              // 0 live, 1 broke this step, 2 disabled
}
JointReaction { force: Vec3, torque: Vec3 }   // table, written once per step (J4), read by gameplay
```

`JointState` is **dense** because it is durable and the solver must reach every instance across
archetypes through one `DenseSolveView` — the exact reason the dense kind was built
(`DENSE-COMPONENTS-PLAN.md:5-11`). It is AoS: joint kernels are scalar (§2.9) and read the whole
row; the 31-lane SoA of `ContactColumns` exists for the 8-wide contact cohort and is not repeated
here (P-19). Both sizes are pinned by `const _: () = assert!(size_of::<…>() == N)`; the `JointDef`
arithmetic above is the [ESTIMATE] the pin replaces with a fact, and **§2.10's cost model quotes
the pin, never the estimate**.

#### ⚠ `order`, and why the dense row index is NOT an authored order (CRITIQUE C-7)

Every ordering property in this design — "contacts then joints" (§2.4), the break readout (§2.7),
the stream append (§2.3), G-2/G-5 — was first written over "joint-row order", meaning the
`JointState` dense slot. **That is not a stable authored order.** A dense slot is assigned from a
LIFO free list ("insert pops free LIFO or pushes len; tombstone pushes free",
`DENSE-COMPONENTS-PLAN.md:57`), so after ANY joint despawn — a broken weld (§2.7), a despawned
ragdoll (§3), a fracture that retires a `jointed` bond (§9.4) — the next joints spawned fill the
freed slots in REVERSE. A second ragdoll spawned into a world where the first was destroyed has a
different slot permutation from the same ragdoll in a fresh world, hence a different colouring and
different floats.

So `JointDef.order: u16` is the key, authored at bake (root-first for a ragdoll, §3) and **sorted
once into the `JointColumns` prepare** (`(order, JointDef.body_a, JointDef.body_b)` as the total
key, so equal `order` is still total and stable). Everything downstream — the edge-stream append,
the colouring, the solve sweep, the break readout, the `JointReaction` write — walks that prepared
permutation, never the dense slot. The dense slot survives only as the `state_slot` back-index.

This does not remove the op-sequence caveat, it bounds it: the *colouring* still depends on absolute
BODY slot values (`DENSE-COMPONENTS-PLAN.md:56-58`), so §9.3's "bit-identical for a fixed,
deterministically-ordered op sequence" governs joints exactly as it governs chunks — **joint spawn,
joint despawn and `JointBroken`→despawn are ops in that recorded sequence**. What `order` buys is
that the joint sweep is not ALSO permuted, so two worlds with the same body layout and the same
joint set agree regardless of the joints' creation history. G-5 tests it (§13).

The per-step prepared form is a scratch cohort `JointColumns` (`ScratchColumn`s, ~13 columns
[ESTIMATE]: resolved `row_a`/`row_b`, world anchors ×2, world basis ×2, the per-DOF effective
masses, the soft coefficients per joint, the speculative-limit errors, a `state_slot` back-index,
the `order`-sorted permutation itself, plus the CSR `color_joint_start`). 13 fits the 38 free ids as
one contiguous run; it is its own cohort because it is swept in its own loop (the per-cohort rule,
`scratch_ids.rs:160-176`).

### 2.2 Endpoint resolution — the row map the gather lacks

`physics_gather` pushes one `BodyState` per matched row and nothing else (`systems.rs:242-262`).
Joints, welds, chunk promotion and the deferred `Contact` producer all need `EntityId ↔ row`. The
gather gains **two `ScratchColumn`s** on `SolverScratch`, refilled each gather: `row_entity:
ScratchColumn<EntityId>` (pushed beside each `BodyState`, so `row_entity[i]` is the entity of row
`i`), and the inverse `entity_row: ScratchColumn<u32>` — a DENSE array indexed by `EntityId.get()`,
`u32::MAX` for "not gathered this step". Joint prepare resolves `(body_a, body_b) → (row_a, row_b)`
in O(J), and a joint whose endpoint is not gathered (despawned, or not a `RigidBody`) is skipped for
the step and flagged.

> ⚠ **CRITIQUE C-4 — this was `SparseMap<u32>`, and that was a Principle-0 violation.**
> `SparseMap` is three `std::Vec`s (`sparse: Vec<Option<usize>>, dense: Vec<U>, indices: Vec<usize>`,
> `crates/boyko_utils/src/sparse_map/sparse_map.rs:5-13`). `ARCH-AUDIT-ECS-DATA-REMEDIATION.md:9`
> ratifies it as a *primitive inside `boyko_utils`*, in the same breath as declaring that **all**
> the violations are in `boyko_physics` and naming `SolverScratch.bodies: Vec<BodyState>` as the
> worst of them — the exact shape a `SparseMap` field on `SolverScratch` would restore, one lane
> after Stage 4 removed it (`resources.rs:3634-3645`: "`bodies` is a `ScratchColumn`, not a
> `std::Vec` (audit Stage P)"). A `ScratchColumn<u32>` is also the cheaper structure here: the
> question asked is a pure array lookup, `SparseMap`'s dense/indices arms buy an iteration order
> nothing on this path wants, and `Option<usize>` is 16 B per slot against 4.
>
> The cost this makes explicit: `entity_row` is sized by the largest live `EntityId`, not by the
> body count, so it is `O(max_entity_id)` of address space — a `scratch_reserve_rows(4)` column
> (§8.1's ceiling discussion, and the same `POOL_MAX_ROWS` cap). Commit is demand-driven, and the
> clear is a range clear over the rows the previous gather touched (tracked by `row_entity`'s live
> span), never a full-column memset.

**Kernel request KR-1**: an entity datum in `Query` (the archetype already exposes
`entity_ids_slice()` — `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs:1701`), or a chunk-walk
that hands the gather the slice. Until it lands the gather takes the second form. This also
discharges the "row→entity map… Phase 10 adds it back" deferral verbatim (`resources.rs:3633-3639`).

### 2.3 One edge stream, contacts first — the same-pair rule holds by construction

`ConstraintGraph::build` takes a second slice, `joints: &[JointEdge { row_a, row_b }]`, and
`build_islands` / `color_manifolds` walk **manifolds in manifold order, then joints in prepared
order** (§2.1's `order` permutation, NOT the dense slot), over the SAME occupancy bitset.
Consequences, each checked against the code:

- **0%-gate**: with zero joints the stream is today's stream; `color_start` / `color_contacts` and
  every float are byte-identical (`resources.rs:2850-2960` is unchanged up to the appended loop).
- **Same-pair rule (P23 / P-4) by construction**: the first-fit test is
  `(!a_dyn || !occ_get(a)) && (!b_dyn || !occ_get(b))` (`resources.rs:2890-2893`). A contact `(A,B)`
  placed in colour `c` sets `A` and `B`; the joint `(A,B)` cannot take `c`. Box2D needs the rule as
  a separate check only because it colours joints and contacts in separate passes over separate
  arrays; one pass over one bitset subsumes it. A `debug_assert_coloring` re-scan over the union of
  both edge kinds is added (the existing re-scan is at `resources.rs:2969-2999`).
- **Islands**: `build_islands` unions joint edges when both endpoints are dynamic (the same ground
  rule, `resources.rs:2703-2713`), so a jointed assembly is one island and `IslandSleep` freezes
  or wakes it as a unit (P-48's opposite failure: half a ragdoll asleep).
- **Kinematic rows** (P-5 / P22): today they are non-dynamic (`inv_mass == 0`) and impose no
  occupancy. A joint to a kinematic body writes NOTHING to it (the `*_movable` guard,
  `colored.rs:1873-2063`), so the scatter hazard Box3D describes does not arise as long as the
  kinematic row is read-only in the colour — which is exactly the shipped guard. No change.
- **Reserved high colours for static-touching constraints** (Box2D's anti-tunneling ordering) are
  NOT adopted in v1 — but ⚠ **CRITIQUE C-11: the reason first given here was wrong.** "Statics
  impose no occupancy here" is equally true of Box2D (`invMassA = 0` bodies are never entered in a
  bitset, RESEARCH §2.4), so it distinguishes nothing. Box2D's actual reason for searching
  static-touching constraints from the TOP of the palette is **Gauss-Seidel sweep order** — the
  last-solved constraint is the best-satisfied one, and a static contact solved last is a wall that
  holds ("to reduce tunneling", `box2d/src/constraint_graph.h`). The real obstacles here are two,
  and both are about THIS tree:
  1. **There is no palette.** `color_manifolds` grows colours by appending occupancy words with no
     upper bound (`resources.rs:2884-2891`); Box2D reserves the top of a FIXED 24/`B2_GRAPH_COLOR_COUNT`
     palette. "Reserved high colours" presupposes a constant that does not exist, so adopting it
     means introducing a palette cap AND an overflow lane for the constraints that exceed it — a
     structural change, not a reordering.
  2. Given a palette, the reordering is still a **value change** to the shipped colored path
     (different colours ⇒ different sweep order ⇒ different floats), so it needs its own gate.
  That gate is the **stacked-tunnelling fixture**, and it is named in the ladder at rung X1 rather
  than left floating in §15: a heavy stack over a thin static floor, asserting no body's position
  crosses the floor plane over N steps, red-first against today's ordering. Deferred, with the
  fixture named (§14.A D-14, §15).
- **Persistence (P-3)**: the per-step rebuild stays. The honest colored ratio already INCLUDES the
  graph build and still wins (`colored_solve_plus_graph` 1.059× @1k, 1.131× @10k,
  `KE16-RESULTS.md:1286`), while the loss is dispatch (fact 2). Persistence would change the
  determinism argument from "pure function of this step's input" to creation-order history
  (RESEARCH §9) for a prologue that is not the bottleneck. Revisit after the KE16 barrier lands
  (§14.B.1).

### 2.4 Where joints solve — inside the colour wave, plus one serial overflow lane

The substep loop keeps its exact order (`soft_step.rs:896-941`; mirrored in
`solve_colored_inner`, `colored.rs:3259-3316`): gravity → **one serial warm-start apply over every
contact slot** → solve(bias) across colours → position integrate → inertia refresh → relax ×N;
restitution once. The 72 waves are 4 substeps × (1 solve + 2 relax) × ~6 colours — the waves come
from `solve_all_colors` (`colored.rs:3276`, `:3307`) and from nothing else. Joints enter at the
same points as contacts, Box2D's stage list (`b2_stagePrepareJoints … b2_stageWarmStart …
b2_stageSolve … b2_stageRelax`, RESEARCH §9):

| Stage | Joint work | Wave cost |
|---|---|---|
| once per step, after `build_columns` | `prepare_joints`: sort by `order` (§2.1), resolve rows, rotate frames to world, effective masses, `SoftCoefficients::new(hertz.clamped, ζ, h)` (§2.5), speculative errors; serial in prepared order | 0 waves (a serial pass beside the contact column build) |
| per substep, **before colour 0** | `warm_start_joints()` — appended to the EXISTING serial `warm_start_apply` pass (`colored.rs:3273`), walking joints in prepared order after the contact slots | 0 waves |
| per substep, per colour, bias and relax sweeps | `solve_joints(c, use_bias)` in the same wave as `solve_color` chunks — the colour's joints ride its LAST contact chunk (see the floor below) | 0 extra waves |
| per substep, before colour 0 | the **overflow** joints (a colour with too few joints to earn their own task, or a joint whose endpoint failed to resolve) are solved INLINE on the calling worker, serially, in prepared order — Box3D's `_Overflow` lane | 0 waves |

> ⚠ **CRITIQUE C-2 — the warm-start row said "inside the colour's ONE wave", and that was both
> non-existent and value-wrong.** There is no per-colour warm-start wave in this solver: warm start
> is a *single serial pass over every contact slot* before colour 0
> (`Self::warm_start_apply(self.columns.solve_view(), self.bodies.solve_view())`,
> `colored.rs:3273`, whose own comment reads "single-threaded here, parallel in `solve_all_colors`").
> Putting joint warm start inside the SOLVE wave of colour `c` would also change the value: a joint
> in colour 3 would re-apply its accumulated impulse AFTER colours 0–2 had already solved against
> un-warmed body velocities — the opposite of Box2D's stage order, where `b2_stageWarmStart`
> precedes `b2_stageSolve` wholesale (RESEARCH §9). Joining the serial pass is both value-correct
> and free; the only other option — a per-row parallel warm-start pass — would ADD a wave and
> violate fact 2.

Inside one colour the joint rows and the contact groups touch pairwise-disjoint dynamic bodies
(the colouring invariant, now over both kinds), so the `{1, N}` argument of `colored.rs:2634-2660`
extends verbatim: disjoint writes, per-body order-independence, within-row order preserved,
barrier between colours. **The wave count per step is unchanged at 72.** The one new serial cost
is `prepare_joints`, O(J) per step, plus O(J) inside the existing warm-start pass.

#### The joint floor is in JOINTS, and a colour's joints ride its last contact chunk

⚠ **CRITIQUE C-NB1/C-NB2.** The overflow trigger was first written as "a colour whose joint count
is below the inline floor" with no floor named. The two floors that exist are in **slots** —
`MIN_PARALLEL_SLOTS_PER_COLOR = 256` (whether a colour dispatches at all) and `MIN_SLOTS_PER_CHUNK
= 64` (how finely a dispatched colour is cut), `colored.rs:227`/`:309` — and the whole-solve gate
reads `widest_color_slots()` (`colored.rs:3256-3257`), which counts **contact slots only**. A joint
is not a slot. Comparing a joint count against a slot floor is precisely the unit error this tree
already catalogues in the retired large-island gate: *"the two 256s are in different UNITS… A guard
that compares numbers across units is not a guard"* (`resources.rs:2379-2390`). So:

- **`MIN_JOINTS_PER_TASK: usize = 8` [ESTIMATE], stated in joints.** Its measurement plan is the
  same shape as `MIN_SLOTS_PER_CHUNK`'s: sweep `{2, 4, 8, 16, 32}` on a crowd-of-ragdolls fixture at
  W ∈ {8, 16} and take the largest value that keeps the wide-colour dispatch gate green. Until that
  sweep runs the constant is labelled `[ESTIMATE]` at its definition site, like `SAFETY` and
  `HERTZ_NYQUIST_FRACTION`.
- **Joints do NOT contribute to the whole-solve dispatch gate.** `widest_color_slots()` stays a
  contact-slot metric; a joint-only world therefore never flips `parallel` on, and solves its joints
  inline. This is the conservative arm (it can only under-dispatch, never race) and it keeps the
  gate's unit pure. Revisit only if a joint-dominated fixture measures a loss.
- **A colour's joints ride its LAST contact chunk rather than becoming their own task.** A ragdoll
  contributes ≤4 same-kind joints to a colour (§2.9) at ~60–250 flops each — a sub-microsecond task,
  exactly the `worker/1 µs` cell where per-task registration dominates (`KE16-RESULTS.md:286-300`).
  On the shipped pool such a task runs inline anyway (the serial floor), but under the KE16 barrier
  a wave of 96 contact chunks plus N tiny joint tasks re-creates the measured regression
  `MIN_SLOTS_PER_CHUNK` exists to prevent: *"about 2.7 slots of work per boxed closure… the
  Jolt-parity pyramid measured 0.82× at 8 workers and 0.56× at 16 — NEGATIVE scaling"*
  (`colored.rs:263-280`). Only a colour clearing `MIN_JOINTS_PER_TASK` gets its own task; everything
  below it appends to the last chunk, and a colour with no contact chunks at all goes to the
  overflow lane. Bit-identity is unaffected — the `{1, N}` property holds for ANY partition of a
  colour into tasks (`colored.rs:270-280`), so this is a placement knob, never a value knob.

#### Order within a colour, and where P-9 actually lives

Order WITHIN a colour is fixed: contacts (ascending manifold index, as today) then joints
(ascending prepared-order index).

> ⚠ **CRITIQUE C-3 — the P-9 claim attached to this order was inverted, and the order is the wrong
> place for it anyway.** The text read: "the joint-row order is the baked root-first order, which is
> how Jolt's `CalculateConstraintPriorities` (P-9) is honoured without a runtime sort." Two errors.
>
> **(a) Inverted.** Jolt increments priority *toward* the root — `// Calculate priority for each
> part. Start with the base priority and increment towards the root`, walking `j =
> mSkeleton->GetJoint(j).mParentJointIndex` and taking `max` ([S] `Jolt/Physics/Ragdoll/Ragdoll.cpp`,
> `RagdollSettings::CalculateConstraintPriorities`) — and sorts **ascending**, `return
> lhs->GetConstraintPriority() < rhs->GetConstraintPriority()` ([S]
> `Jolt/Physics/Constraints/ConstraintManager.cpp`, `sSortConstraints`, iterated forward). Root
> constraints therefore carry the HIGHEST priority number, sort LAST, and are solved **last** in the
> Gauss-Seidel sweep — the last-solved constraint being the best-satisfied one. Feeding a
> **root-first** row order to a first-fit colorer does the opposite: root joints take the LOWEST free
> colours and are solved FIRST every sweep.
>
> **(b) The wrong axis.** Order within a colour is *irrelevant to convergence* — the bodies in one
> colour are pairwise disjoint, which is the whole basis of the `{1, N}` argument, so no joint in a
> colour can see another's result. Sweep priority lives in the **colour index**, and nowhere else.
>
> **The v1 ruling: drop the P-9 claim; do not silently invert the bake.** `order` stays root-first
> because it is the readable authoring order and it is what the mass-ratio bake contract walks
> (§3) — it is a *stability* key for the prepared permutation, not a *priority* key. Honouring P-9
> would mean appending ragdoll joints **leaf-first** so root joints first-fit into the highest free
> colours, which is a value change to the colouring and needs its own evidence. It is therefore a
> **value-bearing follow-up with a named fixture**: a ragdoll-settling gate (drop a ragdoll, measure
> the root-joint separation at rest and the frames-to-sleep, root-first vs leaf-first append) —
> ladder rung R, §15. If the fixture shows no difference on a 15–20-body ragdoll at 4 substeps ×
> (1+2), P-9 is recorded as not transferring to a sub-stepped soft solver and the claim dies with a
> number behind it.

### 2.5 The kernel set and its rules

All kernels follow Box3D's soft-step triple (RESEARCH §2.2): `prepare` / `warm_start` /
`solve(use_bias)`, with `use_bias` the only difference between the main and relax sweeps — the
same switch as `solve_velocities(.., bias_active)` (`soft_step.rs:524-531`).

- **J1 Point, Weld, Distance.** Point: 3×3 `K = diag(1/mA + 1/mB) − rA× IA⁻¹ rA× − rB× IB⁻¹ rB×`
  solved by the explicit inverse (exact ops; no `mul_add`). Weld: point + 3×3 angular block;
  `linearHertz` / `angularHertz` = 0 means maximum stiffness. Distance: one scalar, with
  `min_length`/`max_length` limits and a spring — covers rope, stick and spring (Box3D `types.h`).
- **J2 Hinge, Slider (+ motors, limits).** Hinge: point block + 2×2 angular perpendicular block +
  axial scalar with lower/upper limit accumulators and a motor clamped to `max_torque · h`. Slider
  the translational mirror.
- **J3 ConeTwist (spherical), SixDof.** The ragdoll joint, transcribed from RESEARCH §2.3: five
  accumulators; `rel_q = conj(q_a) · q_b`, `q_b` negated when `dot(q_a, q_b) < 0`; swing/twist
  decomposition; per-limit scalar effective mass `k = axis · (IA⁻¹ + IB⁻¹) · axis`, `m = k > 0 ?
  1/k : 0`. SixDof = per-axis (limit | spring | motor) over 3 linear + 3 angular, the
  Jolt/Rapier generic form; ConeTwist is the fast path, not a constructor over SixDof (the
  cone/twist limits are hard and speculative — P-11 — and SixDof's per-axis angular limits are a
  different, box-shaped constraint).
- **Wheel** (§6) and **Motor** (a PD drive between two bodies: velocity targets with force/torque
  caps plus position control through hertz/ζ — the powered-ragdoll primitive).
- **Speculative limits, verbatim — all THREE variables, not just the bias** (⚠ CRITIQUE C-1):

  ```
  let mut bias = 0.0;  let mut mass_scale = 1.0;  let mut impulse_scale = 0.0;
  if C > 0.0        { bias = C * inv_h; }                       // speculative: arrive exactly
  else if use_bias  { bias = (soft.mass_coeff * soft.bias_rate * C).max(-MAX_BIAS_VELOCITY);
                      mass_scale    = soft.mass_coeff;
                      impulse_scale = soft.impulse_coeff; }
  d_lambda = -m_eff * (mass_scale * vn + bias) - impulse_scale * lambda;
  ```

  The earlier text gave only the bias switch. That is not the rule: **the soft mass and impulse
  scalings belong to the OVERLAP branch alone.** In the speculative branch Box2D leaves `massScale =
  1.0f, impulseScale = 0.0f` ([S] `box2d/src/contact_solver.c`, `b2SolveContacts_Overflow`, fetched
  2026-09-10). With the defaults `mass_coeff = a2·a3 ≈ 0.95` and `impulse_coeff = a3 ≈ 0.05`, a
  bias-only branch would still soften a speculative constraint by ~5 % of the required impulse plus
  a decay of the accumulator — and "arrive exactly on the surface", the entire point of the
  speculative regime, fails. Note also that Box2D multiplies the soft bias by `massScale` **inside**
  the clamp; §7.1 records that as a second, pre-existing divergence of the shipped kernel.
  No extra pass; this is what keeps a limit from being blown through at high angular velocity.
- **Hertz clamp in PREPARE, not authoring (P-12)**: `hertz_eff = hertz.min(HERTZ_NYQUIST_FRACTION
  / h)`; the fraction is `0.25` [ESTIMATE — Jolt's stated valid range is `(0, 0.5 × rate]`, Box3D
  "clamp[s] joint hertz based on the time step"; the exact constant is a fixture measurement]. A
  runtime change of `substeps` therefore cannot destabilise a previously valid joint.
- **No Baumgarte** (P-22); joints use the same `SoftCoefficients` as contacts (O2, one source).
- **Math prerequisite (KR-2, `boyko_math`)**: `Quat::dot`, `Quat::inv_mul`, hemisphere negation,
  `swing_twist(rel_q, axis) -> (swing, twist)`, `delta_quat_to_rotation`, and a 3×3 symmetric
  inverse — none exist (`crates/boyko_math/src/quat.rs:47-235`: the API is `new`, `from_mat3`,
  `normalize`, `mul`, `rotate`, `conjugate`, `inverse_rotate`, `integrate`, and nothing else); the
  animation campaign's AK-1 asks for `dot`/`nlerp` from the same crate.

#### ⚠ CRITIQUE C-6 — "exact ops only" and the ConeTwist kernel contradicted each other

The rule as written is "exact ops only, the four source censuses" (§1 fact 3) and, here, "exact
sqrt, no FMA". **The ConeTwist kernel this design transcribes cannot be written under that rule as
stated.** RESEARCH §2.3's spherical joint needs *angles* — "cone limit error `coneAngle −
swingAngle`", "twist limits with error `twistAngle − lowerTwistAngle`" — i.e. `acos`/`atan2`/`asin`;
motor and spring position targets need `sin`/`cos` to build a target rotation. Three facts make the
contradiction concrete:

1. **No transcendental call exists in the production physics crate today.** Every `.sin()`/`.cos()`
   hit under `crates/boyko_physics/src` is inside `#[cfg(test)]` code — `math.rs:84` (in
   `quat_rotate_known_angle`), `box_box.rs:851`/`:919`, `sphere_box.rs:164`, `resources_tests.rs`.
   There are zero `.acos()`/`.atan2()`/`.asin()` hits at all.
2. **The censuses would not see one.** They scan `_mm256_`/`_mm_` × `{fmadd, fmsub, fnmadd, fnmsub,
   fmaddsub, fmsubadd, rsqrt, rcp}` + `_ps(`, plus `mul_add(` and the `algebraic_` stem
   (`systems.rs:1560-1572`). `f32::sin` matches none of them, so a libm call would enter the kernel
   with no gate objecting.
3. **libm is deterministic within one binary, so `{1, N}` and run-to-run survive** — but the claim
   the design makes is stronger than `{1, N}`: it is *exact ops*, and a libm transcendental is
   neither exact nor censused. Box2D refuses to rely on it even for the weaker property, shipping
   `b2ComputeCosSin` (Bhāskara I) under the comment *"Approximate cosine and sine for determinism …
   However, I don't trust this result"* and `b2Atan2` as an approximation ([S]
   `box2d/src/math_functions.c`).

**Ruling: in-house polynomials, in `boyko_math`, under the same exact-op discipline** — Box2D's
precedent, and the arm that keeps one rule instead of two. KR-2 therefore also carries
`cos_sin(x) -> (f32, f32)`, `atan2(y, x)`, and `acos(x)`, each a fixed-degree polynomial /
rational form built from `+ - * /` and `sqrt` only, with:

- an **accuracy gate** against `f64` `libm` over a dense sweep of the argument range, stating a
  max-ULP figure that the ConeTwist limit tolerance of G-3 is then set from (not the other way
  round);
- the **census vocabulary extended** to the new file so the exact-op rule is mechanically enforced
  there too (G-11), since these are the functions most tempting to "fix" with a libm call later;
- a red fixture: replacing one polynomial with `f32::acos` must turn the accuracy gate — or the
  census — RED, so the gate is not vacuous.

Rejected alternative, recorded: *admit libm and name it a census exception.* Cheaper to write,
but it makes "exact ops only" a rule with an unenumerated hole, in a crate whose whole determinism
argument is that the holes are enumerated (`unsafe`, `disallowed_types`, `#[ignore]`). If the owner
prefers it, it is a §14.B call — the polynomial arm is the design's recommendation.

### 2.6 Warm start is per joint and durable; `reset_warm_start(row)` (P21)

`JointState` IS the warm start (Box3D `b3JointSim`, P-24); there is no hashed table and no
canonical-store step for joints — the accumulators are written back per joint row in the dense
column after the last substep (a `{1, N}`-identical value, since each joint is solved by exactly
one task). The animation design's `reset_warm_start(body)` becomes one mechanism for both kinds:
a per-row `warm_reset: TouchedMask` on `SolverScratch` set by the caller (a pose teleport, a
`DriveToPoseUsingKinematics` write); at seed time a contact point whose key touches a flagged row
seeds zero instead of probing the table (`soft_step.rs:475-496` is the store side; the probe side
gains one mask test), and `prepare_joints` zeroes the `JointState` of any joint touching a flagged
row. The mask is cleared at the end of the step.

### 2.7 Breakable joints and the reaction readout (J4)

Nothing breaks inside the solver (RESEARCH §2.7). After the substep loop, **in prepared order**
(§2.1's `order` permutation — not the dense slot),
`JointReaction { force = λ_point · inv_dt, torque = λ_angular · inv_dt }` is written; a joint whose
`|force| > break_force || |torque| > break_torque` sets `JointState.broken = 1`, is skipped by
`prepare_joints` from the next step on, and raises a `JointBroken { joint, force, torque }` event
(the kernel's `#[event]`). Because the accumulated impulses are `{1, N}` bit-identical and the walk
order is the authored `order` rather than a free-list artefact, **which joint breaks is
bit-deterministic** — the property no surveyed engine states (P-20). Removal of the entity is the
caller's `Commands` in the apply window (IM-1), and **that despawn is itself an op in the recorded
sequence** (§2.1's C-7 note, §9.3): a world that has broken a weld is not required to match a fresh
world spawned with that weld already absent, only to replay identically from the same op sequence.

### 2.8 The oracle, extended

- `joints_colored_is_bit_identical_across_worker_counts`: a fixture with ≥ `MIN_PARALLEL_SLOTS_PER_COLOR`
  slots per colour (so the dispatch is real, not inline — the `large_island_gate_p2` shape) and
  joints in every colour; full final state hash + every `JointState` equal at W ∈ {1, 2, 4, N};
  per-worker touch counts reported (allowed to read one worker on the shipped pool, fact 2).
- `joints_serial_reference`: the reference `SoftStepSolver` gains the same joint kernels in
  prepared order after the manifold sweep; the colored value differs by design (fact 3) and the
  gates are tolerance gates (a hanging chain's drift, a hinge's axis error, a limit's
  overshoot) plus `static_body_unmoved_under_tgs` bit-identical.
- `joint_free_world_is_byte_identical`: the golden hash of `tests/bodytype_determinism_golden.rs`
  is UNCHANGED with the joint stream compiled in and empty — the 0%-gate.
- `same_pair_joint_and_contact_never_share_a_color`: a proptest over random graphs asserting the
  rule through the re-scan (a red fixture: a colorer that skips the joint occupancy set).
- `joint_hertz_clamp_is_in_prepare`: a joint authored at 10× the substep rate, `substeps`
  changed at runtime, no NaN, no growth.
- `joint_sweep_order_is_the_authored_order_not_the_dense_slot` (⚠ C-7): build a world, despawn a
  joint, spawn two more (so the LIFO free list hands out slots in reverse), and assert the prepared
  permutation is ascending by `JointDef.order` — with a RED arm that walks dense slots instead and
  observes a different permutation. This is a property test on the *permutation*, deliberately not
  on the floats: the float claim is the op-sequence claim (§9.3), and asserting bit-identity across
  two different op sequences would be a gate that cannot pass.
- `speculative_branch_leaves_mass_and_impulse_scale_unscaled` (⚠ C-1): a limit at `C > 0` solved by
  the kernel reaches the limit surface to within one ULP of `C·inv_h · h`; RED arm = the earlier
  bias-only branch, which under-shoots by the `impulse_coeff` decay.
- `transcendental_polynomials_match_libm_to_N_ulp` and the census extension (⚠ C-6): the accuracy
  gate of §2.5, plus a RED arm replacing one polynomial with `f32::acos`.
- The FMA/approx source census gains the joint file(s) **and the new `boyko_math` transcendental
  file** (`systems.rs:1548-1611` is the pattern, including its non-vacuity witness).

### 2.9 SIMD — joints are scalar in v1, and the number that says so

A cohort needs **8 joints of ONE kind in ONE colour with pairwise-disjoint bodies**
(`COHORT = 8`, `colored.rs:319`). A ragdoll is ~15–20 bodies (`ANIMATION-RESEARCH.md:378`), a
tree with a torso hub of degree ~5, so its ~14–19 joints spread over ≥5 colours at ≤4 per colour
per kind [ESTIMATE from the graph shape] — a cohort of mostly-masked lanes, the same shape the
256-slot inline floor exists to refuse (`colored.rs:227`). No surveyed engine widens joints
(Box3D is 4-wide and scalar on joints, P-19). **Decision: scalar joint kernels in v1**; a joint
cohort rung is gated on a COUNT gate — a fixture whose per-colour per-kind joint histogram reaches
≥8 in ≥ half of its colours (a crowd of ragdolls) — before any kernel is written (counts, not
clocks: `docs/gaia/DECISIONS.md:282-289`). The 8-wide contact cohort and `simd_solve` (1.96×,
shipped OFF, `KE16-RESULTS.md:1285`) are untouched.

### 2.10 Cost model (all [ESTIMATE], no timing taken)

Per joint per sweep: point 3×3 (~60 flops), hinge ≈ point + 2×2 + axial (~100), cone-twist ≈ point
+ swing + twist×2 + motor + spring (~200), 6DOF ≈ 250. Per step a joint is swept `substeps × (1 +
relax)` = 12 times plus one prepare. A 19-joint ragdoll ≈ 230 joint solves per step ≈ 45k flops
[ESTIMATE]. Memory: `JointDef` **~248 B** (⚠ CRITIQUE C-NB5 — the earlier "~136 B" did not add up
from its own field list; the corrected arithmetic is in §2.1 and the `const _` pin is what the cost
model must quote once J0 lands), `JointState` 64 B, `JointColumns` ~13 × 4–12 B per joint. At 248 B
a `JointDef` is a cold, once-per-step-read table component — which is why the per-step hot form is
`JointColumns` and not the def itself; if the pin comes back materially larger, the fork is a
hot/cold split of `JointDef` (frames + kind + order hot, limits/motor/break cold), not a wider hot
row. No wave is added; the serial `prepare_joints` is O(J log J) for the `order` sort plus O(J).

### 2.11 Collision filtering (rung F) — the unlisted prerequisite of every ragdoll

⚠ **CRITIQUE C-5. The first draft named two filtering mechanisms and neither of them exists.**

- `JointDef.flags: collide_connected` (§2.1) was declared, but nothing in §2 or the ladder said how
  it is *honoured*: the broadphase and narrowphase have no joint access at all, so a flag on the
  joint reaches no decision. Box2D honours it in pair creation by walking the body's joint edge list
  ([S] `box2d/src/body.c`, `b2ShouldBodiesCollide`: `if ( joint->collideConnected == false &&
  joint->edges[otherEdgeIndex].bodyId == otherBodyId ) return false;`, called from
  `broad_phase.c`'s `// Does a joint override collision?`).
- §3 said parent–child collision is disabled "through the existing `Collider { layer, mask }`". Those
  fields are **not consumed anywhere in the physics crate**: a grep for `\.layer\b|\.mask\b` over
  `crates/boyko_physics/src` returns only hash-table masks (`narrowphase/axis_cache.rs:190-265`,
  `solver/warm_start.rs:266-353`), and `components.rs:154-157` says so in its own words — "the fields
  are wired for the Phase-10 broadphase". Even if they were live, `layer: u32` caps a
  one-bit-per-ragdoll scheme at 32 ragdolls, which a crowd fixture (§2.9's own count gate) exceeds by
  design.

So rung R has a prerequisite, and it is now on the ladder. **Two arms; the design picks the second,
with the first kept as the cheap complement:**

| Arm | What lands | Cost | Verdict |
|---|---|---|---|
| **F1 — layer/mask filtering** | make `Collider.{layer, mask}` live in `physics_broadphase` / the grid emit (`systems.rs:280-355`): a pair is a candidate only when `a.layer & b.mask != 0 && b.layer & a.mask != 0` | one `u32` AND per candidate pair, inside a loop that already touches both colliders; a world that leaves the default `layer`/`mask` (all-ones) is byte-identical — the 0%-gate | **adopt**: it is the general gameplay filter every surveyed engine ships, and it is ~10 lines. It does NOT solve ragdolls (the 32-bit cap). |
| **F2 — joint-aware pair filter** | a per-row joint CSR on `SolverScratch` (`joint_start[rows+1]`, `joint_other[]`, built in `prepare_joints` from the resolved rows — KR-1's map is what makes it possible), consulted by the narrowphase pair filter: skip the pair when a joint between those two rows has `collide_connected == false` | build is O(J) beside a pass that is already O(J); the query is a short CSR scan whose length is the body's joint DEGREE (≤ ~5 for a ragdoll torso hub, §2.9) | **adopt**: this is what actually disables parent–child collision, it scales to any number of ragdolls, and it is the mechanism `JointDef.flags.collide_connected` needs in order to mean anything |

F2's CSR is the same structure §9.4's welds and a future contact-event producer want, so it is built
once. Both arms are 0 waves (they run inside existing passes) and both are 0%-gated: F1 by the
all-ones default, F2 by an empty joint set. **Ladder: F sits between J0 and R**, and R's row names
it as a prerequisite.

Gate **G-12**: a two-body world jointed with `collide_connected = false` emits zero manifolds for
that pair and non-zero for an identical un-jointed pair (RED arm = the filter absent); a
`layer`/`mask` pair matrix fixture over 3 layers; and the 0%-gate golden unchanged with both
compiled in and unused.

## 3. Ragdoll (rung R) — AK-4 closed here, L7's blend stays in the animation campaign

- **Data (Gaia-baked, `RagdollAsset`)**: one row per ragdoll bone of the low-detail skeleton —
  `{ anim_bone: u16, parent: u16, shape: ColliderShape, mass, local_com, local_inertia }` — and one
  row per joint `{ parent_part, child_part, kind: ConeTwist | Hinge, limits, hertz, ζ, motor caps
  }` plus `additional_joints` for closed loops (Jolt `mAdditionalConstraints`). **Bake contracts
  (eager, closed — `docs/gaia/DECISIONS.md:56-79`)**: parts parent-first, so `JointDef.order`
  (§2.1) comes out root-first — a *stable authoring* key, **not** a P-9 priority claim (⚠ C-3: Jolt
  solves root constraints LAST, so root-first would be the inversion of P-9, and sweep priority
  lives in the colour index anyway; the leaf-first experiment is a named follow-up, §15); adjacent
  mass ratio ≤ 10:1 (Unity's number, P-8) is a bake REFUSAL, not a warning, walked in `order`;
  shapes are `Sphere | Box` until the capsule rung (§5.1) lands, and the asset carries the capsule
  so the bake refuses on a tree without it rather than silently boxing.
- **Runtime**: bodies are hierarchy ROOTS (`scene_sync.rs:192-212`) with `Simulated` ON; `N_joint ≥ 1`
  joint entities per bone pair are allowed (Unity's companion, P-10); parent–child collision
  disabled by **rung F** (§2.11) — see there for why the `Collider { layer, mask }` route named in
  the first draft is not available.
- **Drive**: `DriveToPoseUsingKinematics` = animation writes `RigidBody.{linear,angular}_velocity` so
  the body reaches the target in `dt` and sets the `warm_reset` mask (§2.6);
  `DriveToPoseUsingMotors` = the `Motor` targets of each joint set from the pose (`kind: Motor`
  position mode). Both are writes into columns the animation campaign's ⑤ already owns
  (`ANIMATION-DESIGN-SPACE.md:484-488`); the per-bone blend weight is L7's.
  ⚠ **CRITIQUE C-NB4b — the drive must clear the sleep latch, and the design did not say so.**
  `DriveToPoseUsingKinematics` toggles a ragdoll body between kinematic and dynamic, which is
  exactly the runtime mass-regime flip `IslandSleep` documents as unsafe: *"If a body's mass regime
  flips at RUNTIME (static ↔ dynamic …) while its latch reads `asleep = true`, the freshly-dynamic
  body would be a freeze candidate in `begin_step` carrying a stale latch with no fresh debounce…
  MUST be paired with `wake_all` (or clearing that row's latch)"* (`resources.rs:3300-3316`). So the
  drive pairs with a per-row latch clear (preferred — `wake_all` is a whole-world hammer that would
  cost every sleeping island in the scene once per ragdoll toggle), and the same call site already
  sets `warm_reset`, so it is one mask, two consumers.
- **Island shape**: one ragdoll = one island of ~15–20 bodies, well under any parallel floor;
  parallelism is across ragdolls in one colour (the joint-in-colour design makes a crowd of
  ragdolls a WIDE colour, which is the case the retired island gate got wrong,
  `resources.rs:2346-2397`).

## 4. Kinematic motion (rung K) — the one deferral that gates three rungs

Today a body with the `Kinematic` bit gets a one-sided contact response and its pose is not advanced
("an intentional deferral, not built yet", `components.rs:66-73`; the gate at
`soft_step.rs:913-919`). The animation design works around it by writing `Transform` and
`Δpose/dt` itself (`ANIMATION-DESIGN-SPACE.md:466-482`). Rung K makes the solver own it: the
position-integrate sub-pass advances kinematic rows by `v·h` and `ω·h` with no gravity, in both
solvers, guarded so a world with no `Kinematic` bit is byte-identical. Moving platforms for the
character (§5), kinematic vehicle targets, and `DriveToPoseUsingKinematics` (§3) sit on it.
Kinematic rows stay read-only in the colour scatter (P22): the integrate pass is per-row, not
per-constraint.

⚠ **CRITIQUE C-NB4a / C-NB9 — the `Kinematic` BIT IS UNREAD TODAY, and K must not add a second
dynamic predicate.** The one-sidedness above is not a property of the bit: it is a property of
`inv_mass == 0`, through `BodyEffective::apply_impulse`'s branchless no-op for statics
(`contact.rs:81-131`) and `effective_mass`'s zero contribution. The gathered
`BodyState.kinematic: bool` is written at gather (`systems.rs:244-251`, `resources.rs:3470-3488`)
and **read by nothing in the solver** — a grep for `kinematic` over `crates/boyko_physics/src`
returns the field declaration, the gather, `scene_sync.rs`'s doc prose, and test constructors, and
nothing else. Meanwhile every scatter guard and the colorer route through `is_dynamic_row(inv_mass)`,
whose doc comment states the rule that makes this dangerous:

> "the coloring's 'is this row dynamic?' decision and the solve's 'may I write this row?' guard
> [must be] the IDENTICAL predicate over the same `inv_mass` snapshot… If the two ever disagreed…
> a worker could write a row the coloring believed shared, **a cross-worker data race the
> `{1, N}`-worker bit-identity test cannot detect** (it runs the SAME predicate on both sides)."
> — `contact.rs:20-38`

A body carrying the `Kinematic` bit with `inv_mass != 0` is coloured and written as dynamic today.
The first draft's rung-K predicate `kinematic && !simulated` would therefore introduce a **second**
predicate beside the one that comment insists must be sole — and the `{1, N}` oracle is blind to
exactly that class of drift. **Two changes, both before K:**

1. **Define kinematic structurally, not by a flag pair**: a kinematic row is `Kinematic ∧ inv_mass
   == 0`. Enforce it at the gather with a `debug_assert!(!kinematic || inv_mass == 0.0, "invariant:
   a Kinematic body must have inv_mass == 0")`, so the convention becomes an invariant with a site
   that fires when it is broken. (This is the house rule "capability = presence of a component"
   applied to a *state*: the bit selects the integrate behaviour, `inv_mass` remains the sole
   write-permission predicate.)
2. **Rung K's integrate pass keys on the bit ONLY for the integrate decision** — never for a write
   permission, never for colouring. The scatter and colorer keep `is_dynamic_row` untouched, so no
   second predicate enters the MT argument. The pass is per-row, single-threaded, in the same
   position-integrate sub-pass that already exists.

**A third gap K opens, which the first draft did not name: sleeping.** `IslandSleep` skips
`inv_mass == 0` rows both from island membership and from the energy metric
(`resources.rs:3246-3260`, `:3319-3340`), and `build_islands` unions only dyn–dyn edges, so a
dyn–kinematic manifold never merges (`resources.rs:2703-2713`). Today that is harmless because
kinematic bodies do not move. **The moment K lets a platform move, a body asleep on it stays frozen
while the platform leaves under it** — Box2D keeps bodies touching a moving kinematic awake. So K
also carries: a kinematic row whose `|v|² + |ω|²` exceeds the sleep threshold **wakes every island
adjacent to it through a manifold** (a pass over the step's manifolds, O(manifolds), before the
freeze decision in `end_step`). Gate **G-13**: a box asleep on a platform that then starts moving is
awake on the next step and rides the platform; RED arm = the wake pass removed, the box hangs in
the air.

## 5. Character controller (rung C) — a query pass whose state is a component (P-17)

Both reference designs are geometric solvers outside the rigid world (RESEARCH §4). Under
Principle 0 the LOGIC may be a query pass, but the STATE must be columns and the queries must read
the engine's own snapshot — which is the shape the soft↔rigid coupling already uses
(`deepest_contact` over the frame-N `SolverScratch` + `BroadphaseGrid` CSR, `soft/coupling.rs:1-27`,
`soft/solver.rs:145-160`).

- **Data**: `CharacterMover { velocity, up, ground_normal, ground_row, flags: grounded | on_steep |
  on_platform, max_slope_cos, step_height, push_limit_default }` (table component) on an entity
  with `Transform` + `Collider { Capsule }` (+ optionally an inner `RigidBody` with `inv_mass: 0` +
  `Kinematic`, Jolt's `mInnerBodyShape`, so sensors and rigid bodies see it).
- **Per fixed step**, one system `character_move` after `physics_apply` (it reads the solved
  snapshot): (1) `cast_mover` — conservative advancement of the capsule along the desired
  translation against the snapshot's bodies (broadphase grid candidates) and the SDF (sphere
  tracing on `sample_sdf`, which is exact for a capsule of radius r: advance by `d − r`); (2)
  `collide_mover` — planes `{ normal, separation, push_limit, clip_velocity, row }` from the
  narrowphase generators at the new pose (the `Manifold` normal + separation is the plane; the
  body's `Kinematic`/`inv_mass` picks `push_limit`); (3) `solve_planes` — the Box3D projection
  "closest position delta satisfying all planes" as a fixed-iteration Gauss-Seidel over the plane
  list in row order (deterministic; Box3D's exact method is a small QP, and the GS form with a
  fixed count is the bit-stable choice); (4) `clip_vector`; (5) `stick_to_floor` (cast down by
  `step_height`), `walk_stairs` (cast up, forward, down). Contact normals within 2.5° merged
  (Jolt's value).
- **Prerequisites**: capsule collider (§5.1); a shape cast; rung K for platforms. **Rotation is not
  handled** (Box3D's stated limitation, P-18) — a rotating platform is a later rung using a local
  coordinate frame (Jolt's approach).
- **Waves**: 0 — the pass is per character and serial in v1; a crowd is a `par_iter` over
  characters that reads the snapshot read-only (fact 2 applies).

### 5.1 The capsule collider (rung H0) and the shape cast (rung H1)

`ColliderShape::Capsule { half_height, radius }` (`components.rs:127-145` is the complete enum;
`local_inv_inertia` at `resources.rs:3503-3540`, `body_bounding_radius` at `systems.rs:1171`, the
narrowphase `match` at `systems.rs:387-430` and the SDF `match` at `systems.rs:598-618` are the
five shape-aware sites — nothing else in the pipeline is). Generators: capsule-sphere
(segment-point), capsule-capsule (segment-segment), capsule-box (segment-OBB closest features,
feature ids from the box's clipped feature classes, `narrowphase/mod.rs:26-45`), capsule-SDF
(sample the two cap centres and the midpoint — the leaf already has `sd_capsule`,
`crates/boyko_sdf_math/src/lib.rs:78-81`). H1 = conservative-advancement cast for sphere / box /
capsule vs the snapshot and the SDF (shared by CCD bullets §7, the mover §5, cast-wheel vehicles
§6 and the animation AK-5 ground query, `ANIMATION-DESIGN-SPACE.md:572`).

## 6. Vehicles (rung V)

**Default: Box3D's shape — no vehicle subsystem.** A car is a chassis body + N wheel bodies +
N `Wheel` joints (§2.5): suspension spring (hertz/ζ) + suspension limits, spin motor
(`max_spin_torque`/`spin_speed`), steering spring + limits — all solved in the colour wave with
everything else, so a vehicle costs N joint rows and zero new machinery. Tyre friction is the
existing 2-DOF Coulomb cone at the wheel–ground contact (`soft_step.rs:596-650`), with a `Wheel`
flag selecting per-wheel friction/restitution from the joint rather than `RigidBodyMass`. The
raycast school (Bullet/Rapier) is the cheap variant for many vehicles: `RaycastWheel` rows on the
chassis, suspension as an analytic impulse from the H1 ground query — a follow-up, since it needs
no joint but does need AK-5 (§5.1). Jolt's engine/gearbox/differential model is gameplay data on
the chassis entity (an Aether `machine` over a table component, `docs/aether-v2/MACHINES.md:7-17`),
not a solver concern.

## 7. Continuous collision (rung X) — the regime the normal solve lacks, then a serial bullet lane

### 7.1 X0 — the speculative regime in the solve (prerequisite of everything speculative)

The shipped kernel applies, when `bias_active`:

```rust
let bias = (soft.bias_rate * pc.separation).max(-MAX_BIAS_VELOCITY);           // ANY sign
let d_lambda = -soft.mass_coeff * m_eff * (vn + bias) - soft.impulse_coeff * pc.normal_impulse;
```
[S] `soft_step.rs:569-581`; the colored kernel mirrors it. With the defaults (`contact_hertz = 30`,
`ζ = 10`, `h = 1/240`) `bias_rate = ω/a1 ≈ 188.5/20.79 ≈ 9.1 s⁻¹` against `inv_h = 240 s⁻¹` — a
positive-separation contact would be stopped ~26× too early (arithmetic from `resources.rs:141-146`
and `soft_step.rs:808-819`, not a measurement; P-25).

⚠ **CRITIQUE C-1 — X0 was transcribed as a bias switch alone, and that is not the regime.** The
first draft wrote `if separation > 0 { bias = separation · inv_h } else { soft }`. Read that against
the code above: `mass_coeff` and `impulse_coeff` are applied **unconditionally whenever
`bias_active`**, so a speculative contact under the draft's branch would still be scaled by
`mass_coeff = a2·a3 ≈ 0.95` and still decayed by `impulse_coeff = a3 ≈ 0.05` — and "arrive exactly
on the surface", the entire property the speculative regime exists to provide, fails. Box2D's
speculative branch leaves the scalings at their neutral values and loads them from `softness` only
in the overlap branch ([S] `box2d/src/contact_solver.c`, `b2SolveContacts_Overflow`, fetched
2026-09-10):

```c
float velocityBias = 0.0f;  float massScale = 1.0f;  float impulseScale = 0.0f;
if ( s > 0.0f )        { velocityBias = s * inv_h; }                       // speculative
else if ( useBias )    { velocityBias = b2MaxFloat( softness.massScale * softness.biasRate * s,
                                                    -contactSpeed );
                         massScale    = softness.massScale;
                         impulseScale = softness.impulseScale; }
float impulse = -cp->normalMass * ( massScale * vn + velocityBias ) - impulseScale * cp->normalImpulse;
```

**X0 is therefore a three-variable change, not a one-line branch**, in both the colored kernel and
(subject to §14.B.2) the reference:

```rust
let (bias, mass_scale, impulse_scale) = if pc.separation > 0.0 {
    (pc.separation * inv_h, 1.0, 0.0)
} else if bias_active {
    ((soft.mass_coeff * soft.bias_rate * pc.separation).max(-MAX_BIAS_VELOCITY),
     soft.mass_coeff, soft.impulse_coeff)
} else { (0.0, 1.0, 0.0) };
let d_lambda = -m_eff * (mass_scale * vn + bias) - impulse_scale * pc.normal_impulse;
```

Note that the relax sweep (`bias_active == false`) already reduces to `-m_eff * vn`
(`soft_step.rs:577-580`), so the third arm reproduces today's relax exactly — the branch is a
refactor there, not a change.

#### The second, PRE-EXISTING divergence the X0 rung must record

Box2D multiplies the soft bias by `massScale` **inside** the clamp — `b2MaxFloat( softness.massScale
* softness.biasRate * s, -contactSpeed )` — while the shipped kernel clamps the *unscaled* bias and
multiplies afterwards: `(bias_rate * separation).max(-MAX_BIAS_VELOCITY)`, then `-mass_coeff *
m_eff * (vn + bias)`. The two agree everywhere the clamp is inactive and differ once it binds: the
shipped form's effective maximum push-out is `mass_coeff × MAX_BIAS_VELOCITY ≈ 0.95 × 4.0 = 3.8 m/s`
rather than `4.0`, so `MAX_BIAS_VELOCITY` does not mean what its name says on deep overlaps. This is
**not** introduced by X0 — it ships today — but X0 is the rung that touches these three lines, so it
is the rung that records it. It is a **value change** to fix (deep-overlap trajectories move), hence
its own row: fix it *with* X0 under one gate, or pin the current behaviour with a comment. Both are
defensible; the design's recommendation is to fix it with X0, because shipping a speculative regime
transcribed "verbatim from Box2D" while silently keeping a Box2D divergence in the same three lines
is the doc-rot shape this repository catalogues. Added to §14.B.2's question.

X0 lands in the **colored** solver (the path CCD ships on) first. The reference `SoftStepSolver`
is byte-untouched by Decision 7 (`OPTIMIZATION-PLAN-PHYSICS.md:103-114`); adding the branch there
is value-neutral for every manifold the narrowphase emits today (`separation ≤ 0` always) but not
byte-neutral — **§14.B.2** asks the owner whether the reference's 0%-gate is bytes or values.

### 7.2 X1 — speculative contacts

`PhysicsConfig::speculative_distance: f32` (default `0.0` = the 0%-gate; Box2D/Jolt use 0.02 m,
RESEARCH §6). The broadphase adds it to `body_bounding_radius`; the narrowphase emits a manifold
when the closest-feature distance is `< speculative_distance` (sphere-sphere: trivial;
sphere-box: the closest point already exists, `narrowphase/sphere_box.rs`; box-box: the SAT
separating axis gives the distance and the clip at positive separation reuses the reference-face
clipper, `narrowphase/box_box.rs:1-25`); `feature_id` unchanged so warm start persists across the
touch. The solver clamps the approach velocity so the bodies land exactly on the surface (X0).
Ghost collisions on internal edges (P-15) are filtered by the existing reference-axis hysteresis
for box-box and are a known artefact for the SDF path (an SDF has no internal edges — the field
is C0 across a CSG seam, `systems.rs:626-660` documents the zero-gradient skip).

### 7.3 X2 — fast bodies and the serial bullet lane

A per-step derived flag `is_fast = max_motion > SAFETY · min_extent` (Box3D; `SAFETY` an
[ESTIMATE] to fix by fixture) and an authored `Bullet` marker component (capability = presence of
a component, the house rule). After the substep loop: fast non-bullet bodies are swept
(conservative advancement, H1) against statics, kinematics and the SDF, and clamped to the first
TOI — parallel-safe because each fast body writes only its own row (a `par_iter` shape, fact 2
applies); **bullets are swept serially afterwards in ascending row order, including against
dynamic bodies** — Box3D runs `b3BulletBodyTask` serially for exactly the determinism reason
(P-14), and under `{1, N}` this is mandatory. Time stealing is the accepted artefact (P-16);
sweeps are re-centred on the fast body (P-23). Bullet-vs-bullet is not resolved (Box2D's rule).

## 8. Soft bodies and cloth (rungs S0–S3)

### 8.1 S0 — the migration off the `Vec` side store: the fork, decided

`SoftBody` holds 30 `pub … : Vec<…>` fields (`soft/component.rs:69-180`); the audit records the
variable-length-payload question as "particle-as-entity, or component-owned sub-columns"
(`ARCH-AUDIT…:43`); the animation design says the cloth rung is "not buildable on `SoftBody` as
it stands" because a parallel skin-to-pose pass would be a parallel writer into a `Vec`-backed
column (`ANIMATION-DESIGN-SPACE.md:493-502`).

**Decision D-8 (§14.A): component-owned sub-columns, as a resource-owned ROW BANK with per-body
ranges** — the pose-bank shape of the animation design (`ANIMATION-DESIGN-SPACE.md:14-18`), not
particle-as-entity:

- **Particle-as-entity rejected by a number**: 20k `Commands` spawns/frame = 0.6–2 ms of CPU, the
  measured reason particles are not entities (`crates/boyko_render/src/particle.rs:5-6`); a 10k-particle
  cloth would spend its whole budget on bookkeeping, and per-particle entities carry no contiguity
  guarantee across archetypes, so every constraint endpoint becomes a map lookup (fact 5).
- **`SoftBank` (resource)**: one `ScratchColumn` per particle field (`pos_x..`, `prev_*`, `vel_*`,
  `inv_mass`, the coupling scratch, the self-collision CSR), each body a contiguous row range
  `[start, start + n)`; append-only for the process lifetime with a free-list of ranges (the P27
  "append-only" rule the render carriers already follow). Columns are reserved at
  `scratch_reserve_rows` budgets (`scratch_ids.rs:97-108`) — address space, zero commit until used.
  ⚠ **CRITIQUE C-NB8 — a process-lifetime bank has a BOOT-TIME CEILING, and it must be named.** A
  `ScratchColumn`'s capacity is a hard ceiling fixed at construction — the soft path already says so
  in its own words: *"every coloring buffer is a `ScratchColumn` whose capacity is a HARD ceiling
  fixed at construction, so the 'may grow on a denser substep' half of the C3b alloc contract is
  unreachable here — there is no allocator to reach"* (`soft/colored.rs:741-745`). That ceiling is
  `scratch_reserve_rows(stride)` = `POOL_TARGET_DATA_BYTES / stride` clamped into
  `[POOL_MIN_ROWS, POOL_MAX_ROWS]`, and `POOL_MAX_ROWS` is **16,777,216 on the syscall arms and
  262,144 on the fallback arm** (Miri / wasm32 / 32-bit, `constants.rs:81-90`). So: a 4-byte particle
  field caps at 2²⁴ ≈ 16.8 M particles natively, and at **262,144 under Miri/wasm** — which is the
  number that actually binds, because the soft `{1, N}` oracles are the tests most likely to be run
  under Miri. **The bank names its ceiling as a constant with both arms, and overflow is a loud
  panic at spawn, never a silent truncation.** Sizing beyond that is a re-reservation at boot, i.e.
  a config value, not a runtime growth.
- **The free-list of ranges is itself durable data and gets a home**: two `ScratchColumn<u32>`s
  (`free_start[]`, `free_len[]`) with a count, on the same resource — not a `Vec`, for the same
  reason the bank replaces the `Vec`s in the first place. It is small, but "small" has never been
  the principle's exemption (`ARCH-AUDIT…:9`), and a `Vec` here would be a side store inside the
  structure built to remove side stores.
- **`SoftBody` becomes a `#[component(storage = "dense")]` handle**: `{ asset: SoftAssetId,
  start: u32, n: u32, particle_radius, flags, material }` — durable, one global column, reachable
  by the solver through a `DenseSolveView` (fact 4).
- **Topology is a Gaia-baked asset (`SoftAsset`)**: edges + rest + compliance, tets + rest volume +
  compliance, faces, bend quads + rest angle, LRA pairs + max distance, skin weights + inverse
  binds, aero/pressure parameters — AND **the per-constraint-type colouring** (Jolt's `Optimize()`
  → `mUpdateGroups` precedent: the parallel partition is a bake product). Baked into append-only
  asset columns by blit (the AK-6 asset-blob region, `ANIMATION-DESIGN-SPACE.md:573`;
  `crates/boyko_serialize/src/format.rs:22-34` is v2 with the dense-store region). Bake contract:
  rest values computed by the SAME `no_std` leaf functions the solve calls (the rest-volume
  discipline of `soft/component.rs:339-361`, P-33, made structural: the leaf is one function
  linked into both the bake and the runtime).
- **Colouring across bodies**: bodies are disjoint row ranges, so colour `c` of every body is
  body-disjoint with colour `c` of every other; the runtime concatenates the baked per-body CSRs
  into one CSR per constraint type per step (O(bodies) offsets, no recolouring). **The soft path's
  per-substep `pool.scope` COUNT goes from "one per dispatching colour per body" to "one per
  dispatching colour"** — see the ladder's S0 row for why that is stated as a count gate and not as
  a formula (⚠ C-NB3). The position-dependent self-collision colouring stays a runtime colouring,
  now over the bank rows of all bodies with `self_collision_iters > 0`.
  ⚠ **CRITIQUE C-NB6 — the anchor for that recolour was wrong.** `soft/colored.rs:686-735` is
  `physics_soft_step_colored` plus the head of `step_body_colored`; it recolours nothing. The
  immutable distance/volume **topology** is coloured ONCE PER FRAME by `color_topology_once`
  (called at `:748`, defined at `:840-856`, with its own comment "Color the IMMUTABLE
  distance/volume topology ONCE per frame, reused across substeps (D3)"). What recolours inside the
  substep loop is **self-collision only**, whose `color_constraints_2` over the freshly emitted
  `pair_list` is at `:936-943`. The distinction matters to S0: the topology colouring is what the
  bake replaces, and the self-collision colouring is what stays at runtime.
- **Gate**: a pure backing swap — the serial and colored soft results are **byte-identical** to
  today's `Vec` path on the existing fixtures (the C1 rule for backing swaps, `ARCH-AUDIT…:35`),
  zero per-step allocation, and the `soft_colored` `{1, N}` oracles unchanged. **Kernel request
  KR-3** = the animation AK-2: ratify `ScratchColumn` (or a differently named alias) as the
  resource-owned column for DURABLE data; the bank is not scratch.
- The scratch-id budget: the bank's ~30 columns plus the soft-graph cohort exceed the 38 free ids
  → `SCRATCH_REGION_MIN_ID` moves down after the census (fact 6).

### 8.2 S1 — faces and the constraint catalogue (Jolt's set, RESEARCH §7.1)

Each is a projector on the same XPBD `Δλ` (`RESEARCH-SOFT-BODY.md:37-41`), each with its own
colouring in the asset and its own `{1, N}` oracle (P-27):

| Constraint | Arity | Colouring | Notes |
|---|---|---|---|
| `Face { v[3], material }` | — | — | the prerequisite of everything below; also the render-mesh binding seam (§8.3) |
| Dihedral bend `{ v[4], rest_angle, α }` | 4 | `color_constraints_4` exists (`soft/colored.rs:268-306`) | the correct bend; the diagonal-spring bend is an authoring option (Jolt `Distance`) |
| Isometric/quadratic bend | 4 | same | up to 3× cheaper (Bergou) — a measured A/B, not v1 |
| LRA `{ v, anchor, max_dist }` | 2, unilateral | `color_constraints_2` | the cape cure without more substeps |
| Skinned `{ v, joints[4], weights[4], max_dist, backstop }` + `InvBind` | 1 | trivially parallel | reads the animation pose bank (`ANIMATION-DESIGN-SPACE.md` §1.3/§1.7); the parallel skin-to-pose pass writes bank rows through Solve views — sound only after S0 |
| Pressure (per body) | — | serial per body | `Σ_faces` signed volume in FIXED face order, applied once per iteration before integration (Jolt) — a serial reduction, deterministic by order |
| Aerodynamics (per face) | 3 | needs `color_constraints_3` (or 4 with a repeated vertex) | drag/lift per face |
| Neo-Hookean pair `{ tet, μ, λ }` | 4 | tet colouring | replaces `project_volume` per material flag; inversion-robust (P-34); same op discipline |
| Cosserat rods | 2 + frames | 2 | hair/rope; later |

Solve order per substep (Jolt's, adapted to what ships): volume/neo-Hookean → bend → skinned →
distance → LRA → self-collision → coupling → SDF → velocity (today's order is distance → volume →
self → coupling → SDF, `soft/self_collision.rs:4-6`; the change is value-bearing and lands as its
own rung with its own gate, the O5 pattern).

### 8.3 S2 — cloth on characters

The rung the animation design deferred: skinned constraint + backstop + LRA (P-29) on top of S0/S1,
with the `Face` column doubling as the render binding (the skinned vertex stream of
`ANIMATION-DESIGN-SPACE.md` §1.8 reads the same rows). The per-step "skin to the current pose"
pass is a lane over bank rows, parallel across characters; order = drive before the step, read
back after (§7 of the animation design).

### 8.4 S3 — self-collision beyond particle–particle; soft↔soft

Vertex-face and edge-edge pairs change the candidate EMISSION order that the SP3 determinism proof
rests on (`self_collision.rs:27-35`, P-36); soft↔soft is unsolved in shipping engines (P-32).
Both are behind the S1 catalogue and are **§14.B.6** (scope). Air meshes are the robust
alternative (a baked tessellation) if the owner wants "cannot miss" over "cheap".

### 8.5 Tearing and plasticity — routed to the SDF

Tearing invalidates the colouring CSR, the `next_pow2(2n)` hash, the preallocated ranges and the
zero-allocation contract in one event (P-28). **Recommendation §14.B.5: out of the soft ladder;
the visual goal is served by SDF carving (§9.6).** Plasticity (rest-shape update past a threshold)
is a per-constraint write in fixed order and does not break the structures — it can follow S1 as
a small rung if wanted.

### 8.6 The solver family stays XPBD; VBD/AVBD is recorded, not adopted

The only new evidence since `RESEARCH-SOFT-BODY.md` (RESEARCH §1). Its wins — 3–8 colours, mass
ratios — come with Chebyshev acceleration and dual-buffer partial-Jacobi, order-dependent machinery
the no-atomics `{1, N}` contract forbids (P-30), and it is not a projector (it would not reuse
`project_distance` / `project_volume`). The wave-count problem it would address is being attacked
at the pool (fact 2). **Closed here** unless AVBD's published source shows an order-independent
form; §15 keeps the pointer.

## 9. Destruction (rungs D0–D2)

### 9.1 D0 — the data model: Blast's asset/instance split in ECS columns

```
FractureAsset (Gaia-baked, append-only asset columns):
  chunks:  { parent: u32, first_child: u32, child_stop: u32, centroid: Vec3, volume: f32,
             local_inertia: Vec3, shape: ColliderShape | HullId, mesh_range: (u32, u32), level: u8 }
  bonds:   { normal: Vec3, area: f32, centroid: Vec3, jointed: bool }
  graph:   CSR { adjacency_start[nodes+1], adjacent_node[], adjacent_bond[] }   // NvBlastSupportGraph
  support: { chunk_of_node[] }
  damage:  { threshold_by_level[], propagation, material_class: u8 }             // Chaos's per-level knobs
  debris:  { max_sleep_time, removal_duration, slow_moving_threshold }         // Chaos's lifecycle fields

FractureInstance   #[component(storage = "dense")] on the root entity:
  { asset, bank_start, n_nodes, root_row, promotions_this_step }

FractureBank (resource, row bank like SoftBank):
  bond_health: f32, chunk_health: f32, node_actor: u32 (which promoted entity owns the node),
  visible: bits, plus the union-find scratch per instance
```

Immutable topology in the asset, mutable health per instance in bank rows, chunks as ROWS — the
exact split Blast ships (RESEARCH §8.1) and the exact reason "every chunk a body from t=0" is
refused (P-39, the 0.6–2 ms spawn number).

### 9.2 What breaks a bond — the impulse export seam, then a monomorphic damage program

- **Seam (new, prerequisite)**: the colored solve already holds `normal_impulse` per slot
  (`colored.rs:886`); after the substep loop, walking `canonical` order (`colored.rs:67-80`,
  `:907`), it writes `ContactImpulse { manifold, max_normal_impulse, sum_normal_impulse }` into a
  `ScratchColumn` — canonical order makes it `{1, N}` bit-identical, so **bond breaks are
  deterministic under the same contract as joints** (P-20, P-42). The reference solver gets the
  same export from its `PointConstraint` rows.
- **Damage program**: `fn apply_impact<M: DamageModel>(asset, inst, impact: Impact { row, normal,
  impulse }, bank)` — a generic over a `DamageModel` trait with `#[repr(u8)]` material classes
  dispatched by a `match` at the instance (monomorphised; no `dyn`, principle 1). Blast's shader
  functions are C function pointers (RESEARCH §8.1); the Rust form is the baked blend-program
  precedent (`ANIMATION-DESIGN-SPACE.md:188`, `:215`, D10 at `:617`). Anisotropy = compare the
  impact direction with the bond
  normal; thresholds per level from the asset.
- **Stress propagation (optional D0b)**: Blast's `ExtStressSolver` as a fixed-iteration
  Gauss-Seidel over the bond CSR in bond order — serial per instance in v1, deterministic by
  order, cost linear in the iteration cap (Blast's own statement). Needs ≥1 anchored node.

### 9.3 Split — the union-find CODE, the three phases, and a budgeted promotion

- Island detection over unbroken bonds is connected components — the algorithm `ConstraintGraph`
  runs over manifolds (`resources.rs:2664-2713`). **Share the CODE, not the instance**
  (`RENDER-PHYSICS-GPU-RESEARCH.md:157-158`): `uf_find` / `uf_union` are extracted into a
  `UnionFind` over two `ScratchColumn<u32>` (KR-4), used by both.
- **Three phases in the apply window (P-41)**: `generate` (impacts → fracture commands, no
  mutation, runs after `physics_apply`) → `apply` (health writes, bond breaks) → `split`
  (components → promotion list). Structural work — spawning promoted chunk entities with
  `RigidBody` + `RigidBodyMass` (from baked centroid/volume/inertia) + `Collider` + `ChunkOf {
  instance, node }` + the render instance of `mesh_range` — goes through
  `Commands::spawn_batch` (`crates/boyko_ecs/src/ecs/core/system/params/commands.rs:313` — ⚠ C-NB6b:
  the first draft cited `ecs_master.rs:1079`, which is the *exclusive* `EcsMaster::spawn_batch`, not
  the deferred `Commands` route the text describes) and lands before the next gather, so IM-1 holds
  (`systems.rs:1127-1170`).
- ⚠ **CRITIQUE C-NB7 — "single-threaded" was assumed, never stated.** The recorded op sequence is
  deterministic only if the three phases and the promotion run in a **fixed instance order on ONE
  `Commands` buffer**. Blast's own guarantee is per-actor thread safety (RESEARCH §8.1), which says
  nothing about ordering across actors; and this tree records that parallel `Commands` workers make
  entity-id values timing-dependent — *"the entity-id counter is a `Relaxed` atomic shared by
  parallel `Commands` workers"* (`systems.rs:29-35`, IM-2). §9.3 said the stress solver is serial
  and left the rest open. **Stated now: `generate`, `apply` and `split` are ONE system, running the
  instances in ascending `FractureInstance` order, single-threaded, writing one `Commands` buffer,
  and the promotion `spawn_batch` is issued from that same system.** Parallelising `generate` is a
  later rung and would need a per-instance output buffer merged in instance order — the same shape
  the broadphase's O3 parallel emit uses — never a shared queue.
  Correspondingly, **G-9's replay fixture spawns through the parallel-capable path**, not through a
  hand-serialised test harness: a gate whose fixture cannot reach the non-deterministic route is a
  gate that cannot fail, and this repository's failure taxonomy is mostly that.
- **Budget**: `max_promotions_per_step` (count; Chaos's `PerAdvanceBreaksAllowed`); the
  overflow carries to the next step in instance order. Debris lifecycle (sleep → removal after
  `max_sleep_time` with `removal_duration` shrink) rides `IslandSleep` — **which is why step 6
  (its migration) precedes D0** (P-48).
- **Determinism**: coloring depends on absolute slot values (`DENSE-COMPONENTS-PLAN.md:56-58`), so
  the claim is "bit-identical for a fixed, deterministically-ordered op sequence INCLUDING the
  fracture ops"; the `FractureLog` (a resource column of `{ step, instance, bond | promotion }`)
  IS that op sequence, recorded like the animation's significance snapshot
  (`ANIMATION-DESIGN-SPACE.md:524-534`). It is also the wire form if destruction is ever
  server-authoritative (§14.B.8).

### 9.4 Welds are breakable joints (J4)

A bond flagged `jointed` between two nodes that a split puts into different promoted entities
becomes a `Weld` joint with `break_force`/`break_torque` from the asset — Blast's internal joints
that activate on split (RESEARCH §2.7). No new kind.

### 9.5 The hull collider (rung H2) — the fidelity gate

Voronoi shards are hulls; with `Sphere | Box` a fracture degrades to boxes (P-43). D0 ships on
OBB-fitted boxes (labelled degraded); H2 adds `ColliderShape::ConvexHull { hull: HullId }`
referencing baked hull vertices/face normals/edge directions in an asset column (CoACD/V-HACD-class
decomposition at bake — RESEARCH §8.2), hull-hull via SAT over face normals + edge cross products
(the box-box generator IS the 6-face/3-edge special case, `narrowphase/box_box.rs`), hull-sphere,
hull-box, hull-capsule, hull-SDF (vertex samples, 8-wide through `sdf_edit_list_x8` when the ±0
divergence is fixed — `resources.rs:455-466`), feature ids from clipped features, inertia baked.
Interior faces are a render contract: ≥2 material slots per fractured mesh (P-44).

### 9.6 D2 — the SDF-atlas route for sculpted destruction

The analytic list is capped at 16 and byte-frozen against the physics kernel
(`crates/boyko_sdf_math/src/lib.rs:105`, `:318-323`; P-46), so carving cannot be an edit list. The
written representation exists: bricks (`brick.rs:34-44`, `:195-207`), a CPU baker byte-parallel to
`fill_brick` (`mesh_sdf.rs:1-18`), and an incremental dirty-brick rebake that is bit-identical to
a full bake (`brick_atlas.rs:14-19`).

- **`CarvedField` authority**: a region whose field is BRICK-resident; carve edits are a STREAM
  applied to the bricks (dirty-brick rebake), then dropped — unbounded edit count, bounded state.
  The analytic list stays the near-field/dynamic authority; the atlas becomes authoritative where
  carving has happened.
- **Physics samples a CPU-resident brick proxy** (Decision 6's "later" arm,
  `OPTIMIZATION-PLAN-PHYSICS.md:95-101`, never per-frame readback): `boyko_sdf_math` gains
  `sample_brick(bricks, p) -> (d, ∇)` (trilinear over the same snorm8 samples the shader reads),
  written as an eDSL leaf instantiated over `f32` and `Emit` so the CPU proxy and the GPU sampler
  are one source (the SDF field's existing pattern, `sdf_query.rs:1-15`). `sample_sdf` folds
  `min(analytic, brick)` with a fixed op order.
- **Splitting**: after a carve, connected components over the dirty region's brick occupancy
  (`classify_brick` codes); a component with no anchor is a fragment. **Fragment geometry is NOT
  generated at runtime** (P-45; "runtime refracture = tens of ms",
  `sdf-engine-architecture.md:419`): a fragment is promoted as a baked pattern chunk (D0) or, in
  v1, as its brick-occupancy OBB — the "SDF decides WHERE, the solver decides HOW debris behaves"
  chain (`sdf-engine-architecture.md:446-455`). Marching-cubes hulls tied to the dirty-brick set
  are the GPU-side follow-up the render plan names (`RENDER-PHYSICS-GPU-PLAN.md:214-216`).
- **Determinism gate**: the CPU proxy bytes == the atlas staging bytes after every carve (the
  M3 "bit-identical to a full rebake" property extended to the proxy); the carve stream is part
  of the recorded op sequence.
- **Budget honesty (P-47)**: carving is GPU-bound (regen), promotion is CPU-bound (solver); every
  claim names which.

### 9.7 Rendering the pieces

Unpromoted chunks render as sub-ranges of the parent's mesh through the existing instance path;
a promoted chunk is one more instance row of its `mesh_range`. Interior faces carry the interior
material slot (P-44). Culling of many small pieces is the render campaign's; the debris budget
above bounds their count.

## 10. GPU — the last rung (G)

Rigid stays CPU (branchy, low-N, decision logic — `RENDER-PHYSICS-GPU-PLAN.md:82-95`). The
soft/continuum population is the GPU candidate (SP5, past a MEASURED break-even,
`OPTIMIZATION-PLAN-PHYSICS.md:312-315`). What this design adds: **the XPBD projectors of S0/S1 are
written as `boyko_shaderdsl` leaves** — one generic body instantiated over `f32` (the CPU oracle
the whole soft path already runs) and `Emit` (HLSL) — so when a `SoftBank` range is moved to a
`GpuColumn`, the bit-identity of CPU vs GPU is the same mechanism the SDF field uses today, not a
new burden (P-35). The GPU soft solve is per-colour dispatches over the baked colouring (no
atomics), collision against the atlas directly (no readback), coupling to rigids through a
per-step reaction buffer read ONCE (the `SoftRigidReaction` shape, `soft/coupling.rs:57-66`).
Destruction's GPU half (brick regen) is already GPU. Not started before S1 ships on the CPU.

## 11. The ladder

Prerequisites name the tree site each rung changes or depends on; "waves" is the change to the
72-per-step count; "oracle" is the new gate; costs are [ESTIMATE]s.

| Rung | Feature | Prerequisites in THIS tree | Data added (ECS / Gaia) | Waves | Cohort shape | Oracle | Cost model |
|---|---|---|---|---|---|---|---|
| **K** | kinematic motion | `soft_step.rs:913-919` gate; `components.rs:66-73`; the colored mirror `colored.rs:3293`; **the `Kinematic ∧ inv_mass == 0` gather invariant** and the adjacent-island wake pass (§4, C-NB4a/C-NB9: the bit is unread today and `is_dynamic_row` must stay the SOLE write predicate) | none | 0 | per-row | byte-identical without the bit; **G-13** asleep-on-a-moving-platform | O(rows) + O(manifolds) wake scan |
| **J0** | graph + storage for a second constraint kind | `resources.rs:2625-2635` (second slice), `:2703-2713` (island union), `:2850-2960` (stream append); KR-1 row map on `SolverScratch` (`systems.rs:242-262`, `resources.rs:3630-3640`) as **two `ScratchColumn`s, never a `SparseMap`** (C-4); `JointColumns` cohort in `scratch_ids.rs` (13 of 38 free ids); KR-2 quat math **+ in-house transcendentals** (C-6) | `JointDef` (table, with `order: u16` — the authored key, C-7), `JointState` (dense), `JointReaction`; `SolverScratch.{row_entity, entity_row, warm_reset}` | 0 | scalar rows inside the colour wave | 0%-gate golden unchanged; same-pair proptest; `joint_sweep_order_is_the_authored_order_not_the_dense_slot` | prepare O(J log J) sort + O(J), serial |
| **F** | collision filtering: `layer`/`mask` live (F1) + joint-aware pair filter (F2) | J0 (F2 needs the resolved rows); `physics_broadphase` / grid emit `systems.rs:280-355`; `components.rs:150-168` (the fields exist, **unread** — C-5) | per-row joint CSR on `SolverScratch` | 0 | — | **G-12** jointed pair emits no manifold; layer matrix; 0%-gate at the all-ones default | O(1)/pair + O(degree) |
| **J1** | point, weld, distance | J0 | — | 0 | scalar | `{1, N}` at ≥256-slot colours; tolerance gates | ~60–100 flops/joint/sweep |
| **J2** | hinge, slider, motors, limits (speculative) | J1 | — | 0 | scalar | limit-overshoot fixture; hertz-clamp fixture | ~100 |
| **J3** | cone-twist, 6DOF | J2; KR-2 swing/twist | — | 0 | scalar | ragdoll drop fixture; `{1, N}` | ~200–250 |
| **J4** | breakable + reaction readout + `JointBroken` event | J1 | `JointReaction` | 0 | — | bit-deterministic break set at W ∈ {1, N} | O(J) |
| **R** | ragdoll (AK-4 closed) | J3, K, **F** (parent–child collision — C-5); `scene_sync.rs:192-212` roots; the per-row sleep-latch clear on a kinematic drive (`resources.rs:3306-3316`, C-NB4b); H0 for fidelity | `RagdollAsset` (Gaia, bake contracts: parent-first `order`, ≤10:1 masses) | 0 | across ragdolls in a colour | pose-drive round trip; mass-ratio bake refusal fixture; **root-first vs leaf-first settling fixture** (the P-9 follow-up, C-3) | ~230 joint solves/step per ragdoll |
| **H0** | capsule collider | the five shape sites: `components.rs:127-145`, `resources.rs:3503-3540`, `systems.rs:1171`, `:387-430`, `:598-618`; `sd_capsule` (`boyko_sdf_math/src/lib.rs:78-81`) | `ColliderShape::Capsule` | 0 | — | generator proptests; feature-id stability | — |
| **H1** | shape cast (conservative advancement) vs snapshot + SDF | H0; `BroadphaseGrid` CSR (`resources.rs:594-700`); `sample_sdf` | none | 0 | `par_iter` over casters | TOI vs brute-force sweep on fixtures | O(candidates) |
| **C** | character controller | H0, H1, K | `CharacterMover` (table) | 0 | per character | slope / stair / platform fixtures; `{1, N}` over a crowd | O(planes) per character |
| **V** | vehicles (wheel joints) | J2, H0; raycast variant needs H1 + AK-5 | `Wheel` joint kind | 0 | scalar | drive/brake/steer fixtures | N joints per vehicle |
| **X0** | speculative regime in the solve — **three variables** (`bias`, `mass_scale`, `impulse_scale`), not a bias switch (C-1); plus the `massScale`-inside-the-clamp divergence (§7.1) | `soft_step.rs:569-581` (colored mirror); §14.B.2 for the reference AND for the clamp fix | none | 0 | — | byte-identical while no positive separation exists; `speculative_branch_leaves_mass_and_impulse_scale_unscaled` | — |
| **X1** | speculative contacts | X0; broadphase radius (`systems.rs:1171`); generators (`box_box.rs`, `sphere_box.rs`) | `PhysicsConfig::speculative_distance` (default 0) | 0 | contacts' cohort | 0%-gate at 0.0; tunnelling fixture; **the stacked-tunnelling fixture that decides D-14** (heavy stack over a thin static floor, no crossing over N steps — C-11) | +candidates |
| **X2** | fast-body sweep + serial bullets | H1, X0 | `Bullet` marker; per-step `is_fast` | 0 (serial lane) | — | `{1, N}` with bullets; TOI fixture | O(fast) |
| **S0** | soft storage migration (the first cloth rung) | `soft/component.rs:69-180` (30 `Vec`s); `color_topology_once` `soft/colored.rs:748`, `:840-856` (what the bake replaces) and the self-collision recolour `:936-943` (what stays — C-NB6); KR-3; `SCRATCH_REGION_MIN_ID` move (`scratch_ids.rs:541-550`); AK-6 blob region; the bank's named `POOL_MAX_ROWS` ceiling (C-NB8) | `SoftBank` (resource bank), `SoftBody` (dense handle), `SoftAsset` (Gaia, with baked colourings) | **a COUNT gate on `pool.scope` entries, in two regimes** (C-NB3) — see the note below the table | unchanged (SP4) | **byte-identical** to the `Vec` path; zero alloc; `{1, N}` unchanged; **G-7** as a count | O(bodies) CSR concat |
| **S1** | faces, dihedral bend, LRA, pressure, aero, neo-Hookean | S0; `color_constraints_{2,4}` (`soft/colored.rs:232-306`) + a 3-arity | asset columns per type | 0 (types add colours WITHIN the substep's existing wave budget only if folded — see §15) | particle rows | per-type `{1, N}`; rest-state creep fixture per type | per-constraint projector |
| **S2** | cloth on characters (skinned + backstop + LRA) | S1; animation pose bank (`ANIMATION-DESIGN-SPACE.md` §1.3/§1.7) | skin columns in `SoftAsset` | 0 | lane over bank rows | pose-drive fixture; `{1, N}` | O(particles) |
| **S3** | vertex-face self-collision; soft↔soft | S1; `self_collision.rs:27-35` proof re-done | — | +1 runtime colouring | — | emission-order determinism | — (§14.B.6) |
| **D0** | fracture data model + impulse seam + damage + split + budgeted promotion + welds | J4; step 6 (`IslandSleep`, `resources.rs:3063-3096`); KR-1; KR-4 union-find; `colored.rs:886`, `:907` (impulse export); `Commands::spawn_batch` (`commands.rs:313`, NOT `ecs_master.rs:1079` — C-NB6b); the three phases single-threaded in instance order (C-NB7) | `FractureAsset` (Gaia), `FractureInstance` (dense), `FractureBank`, `ChunkOf`, `FractureLog`, `ContactImpulse` | 0 | — | recorded-op-sequence bit-identity; count gates on promotion | O(impacts + nodes in split) |
| **H2** | convex hull collider + hull narrowphase | H0; `narrowphase/box_box.rs` as the special case; `sdf_simd.rs` ±0 fix for the 8-wide vertex sample | `ColliderShape::ConvexHull`, hull asset columns | 0 | contacts' cohort | SAT proptests vs a reference GJK | O(faces+edges²) per pair |
| **D1** | shards as hulls; stress solver (optional) | D0, H2 | — | 0 | — | as D0 | — |
| **D2** | SDF-atlas carving | brick proxy leaf (`brick.rs`, `mesh_sdf.rs`), `brick_atlas.rs` rebake, `sdf_query.rs` fold, eDSL sampler | `CarvedField`, carve stream, brick proxy | 0 CPU; GPU regen | — | proxy bytes == atlas bytes; carve stream in the record | GPU regen; CPU O(dirty bricks) |
| **G** | GPU soft / continuum | S1 on CPU; `GpuColumn`; eDSL projectors | — | GPU dispatches per colour | — | CPU oracle == GPU bit-for-bit via the eDSL | past a measured break-even |

> ⚠ **CRITIQUE C-NB3 — S0's wave cell was a FORMULA, and the formula overstates the win.** The cell
> read "**−(bodies−1) × Σ_types n_colors** per substep". That counts colours, but a colour is only a
> wave **if it dispatches**, and a per-body colour below `MIN_PARALLEL_SLOTS_PER_COLOR = 256` slots
> is solved INLINE with no `pool.scope` at all (`soft/colored.rs:53-63`, and the site
> `if span < MIN_PARALLEL_SLOTS_PER_COLOR { …inline… }` at `:1060`). Two regimes follow, and the
> formula is right in only one of them:
>
> - **Bodies whose individual colours already dispatch** (large cloths, ≥256 slots per colour): the
>   reduction is real, and it is the case S0 is for.
> - **A world of many SMALL soft bodies** (the case the "wave count becomes independent of the body
>   count" sentence sounds like it is about): today's wave count is already ≈ 0, because every
>   per-body colour is inline. Merging them into a bank can *cross* 256 where no individual body
>   did, so the merged bank may **ADD** waves — while doing strictly less total work and with far
>   better locality. That is a legitimate outcome, not a regression, but it is the opposite sign.
>
> So the claim is stated as **G-7's COUNT gate on `pool.scope` entries per substep**, with both
> regimes measured and named, and no formula in the cell. This is the same rule §2.9 applies to the
> joint cohort (counts, not clocks) and the same rule G-7 already carried; the ladder cell had
> simply outrun it.

Critical path: **K → J0 → F → J1 → J2 → J3 → R** (the ragdoll line, the whole of AK-4) ‖ **H0 → H1 →
C / X2 / V-raycast** ‖ **X0 → X1** ‖ **S0 → S1 → S2** ‖ **step 6 → D0 → (H2) → D1** ‖ **D2** ‖ **G**
last. J0 and S0 are independent and can run in two worktrees (the worktree-per-system rule). F can
land in parallel with J1–J3 provided it precedes R; F1 (layer/mask) has no J0 dependency at all.

## 12. Kernel and crate requests born from the design

| # | Item | Why | Zero when unused |
|---|---|---|---|
| KR-1 | `boyko_ecs`: an entity datum in `Query` (or a chunk-walk exposing `entity_ids_slice`, `archetype.rs:1701`) | §2.2 row map; also the deferred `Contact` producer and F2's joint CSR | yes |
| KR-2 | `boyko_math`: `Quat::{dot, inv_mul, swing_twist, delta_to_rotation}`, hemisphere negation, symmetric 3×3 inverse — exact ops | §2.5 | yes |
| KR-2b | `boyko_math`: in-house `cos_sin` / `atan2` / `acos` polynomials with an ULP gate and a census extension (Box2D `b2ComputeCosSin` / `b2Atan2` precedent) — **without them "exact ops only" and the ConeTwist kernel contradict each other** (C-6) | §2.5 | yes |
| KR-3 | kernel: ratify `ScratchColumn` (or an alias) for durable resource-owned banks | §8.1; = animation AK-2 | yes |
| KR-4 | `boyko_physics`/`boyko_utils`: `UnionFind` over two `ScratchColumn<u32>` extracted from `ConstraintGraph` | §9.3, shared CODE | yes |
| KR-5 | threadpool: the in-scope barrier (one `pool.scope` per step) — the KE16 lane's item, restated: every parallel gate here is red-by-construction on the shipped route until it lands | fact 2; = animation AK-9 | — |
| KR-6 | `boyko_sdf_math`: `sample_brick` as an eDSL leaf (f32 + Emit) | §9.6 | yes |
| KR-7 | `boyko_shaderdsl`: the XPBD projectors as leaves | §10 | yes |
| KR-8 | Gaia: asset kinds `RagdollAsset`, `SoftAsset` (with baked colourings), `FractureAsset`, hull columns; bake contracts as refusals | §3, §8.1, §9.1, §9.5 | v2 files unchanged |
| KR-9 | `boyko_physics`: the `ContactImpulse` canonical-order export | §9.2 | one column, empty when no consumer |

## 13. Gates (all red-first; every parallel gate reports per-worker touch counts)

- **G-0 0%-gate**: `bodytype_determinism_golden`'s `GOLDEN` unchanged with J0, X0 (colored), S0
  and D0 compiled in and unused; the reference solver's bytes unchanged unless §14.B.2 rules
  otherwise.
- **G-1 same-pair**: proptest over random contact+joint graphs → the re-scan never finds a
  dynamic body twice in a colour; red fixture = a colorer that ignores the joint stream.
- **G-2 `{1, N}` with joints**: at W ∈ {1, 2, 4, N}, full state hash + all `JointState` equal,
  on a fixture with ≥256-slot colours (non-vacuous dispatch, `large_island_gate_p2` shape) — on
  the SHIPPING worker route (the `ke16_app1` receipt, `tests/ke16_app1_solve_in_system_bit_identity.rs:1-45`).
- **G-3 limits**: a hinge driven at high ω never exceeds its limit by more than the speculative
  tolerance; a limit authored above the Nyquist clamp does not NaN after a runtime `substeps` change.
- **G-4 ragdoll**: dropped ragdoll settles, no joint separation above tolerance, sleeps as ONE
  island; a 10:1 adjacent-mass asset is a bake refusal (red fixture at 11:1).
- **G-5 breaks**: the set of broken joints/bonds at W=1 equals W=N bit-for-bit; a fixture with a
  near-threshold impulse. **Stated over `JointDef.order`, never over the dense slot** (C-7): the
  fixture despawns a joint and spawns two more before the near-threshold step, so the LIFO free list
  has permuted the slots, and the assertion is that the broken SET (by `order`) is unchanged at
  W ∈ {1, N}. It does NOT assert bit-identity against a differently-ordered op sequence — that would
  be a gate that cannot pass (§9.3, D-11).
- **G-6 CCD**: X0 byte-identical on today's fixtures; X1 a bullet-speed sphere against a thin box
  does not tunnel; X2 bullet order serial (touch counts show one worker for the bullet lane by
  design).
- **G-7 S0**: byte-identical serial and colored soft results vs the `Vec` path; zero per-step
  allocation (the counting allocator); and a **COUNT of `pool.scope` entries per substep, reported
  for BOTH regimes** (C-NB3) — a many-large-bodies fixture, where the count must fall, and a
  many-small-bodies fixture, where today's count is already ≈0 and the merged bank may legitimately
  raise it. The gate asserts the first and *records* the second; asserting "independent of body
  count" as a single inequality would fail on the second fixture for the right reason, which is not
  a failure.
- **G-8 rest-state creep** per S1 type: a body at rest under its new constraint stays at rest to
  exact zero for the constraint value (the `C = 0` discipline).
- **G-9 fracture**: the recorded op sequence replays to a bit-identical world; promotions per step
  == the budget on a saturating fixture (count); `IslandSleep` desync `debug_assert!` never fires.
  **The fixture spawns through the parallel-capable `Commands` path** (C-NB7), not through a
  hand-serialised harness — otherwise the gate cannot reach the route whose determinism it claims
  (`systems.rs:29-35`, the `Relaxed` id counter), and a gate that cannot fail is not a gate.
- **G-10 carve**: CPU proxy bytes == atlas staging bytes after each carve; `sample_sdf` fold order
  fixed.
- **G-11 census**: every new file with `_mm256_` sites joins the four-census pattern **and the new
  `boyko_math` transcendental file joins it too** (C-6), each with a non-vacuity witness of its own
  (`systems.rs:1598-1611` is the pattern); a bare `#[ignore]` carries its class prefix
  (`CLAUDE.md`'s ignore vocabulary).
- **G-12 filtering** (new, C-5): a pair joined by a `collide_connected = false` joint emits zero
  manifolds while an identical un-jointed pair emits one; a 3-layer `layer`/`mask` matrix fixture;
  the 0%-gate golden unchanged with F1+F2 compiled in and at their defaults. RED arms: the filter
  removed (both halves separately).
- **G-13 kinematic sleep** (new, C-NB4a/b): a box that has fallen asleep on a kinematic platform is
  awake on the step after the platform starts moving, and rides it; RED arm = the adjacent-island
  wake pass removed, the box hangs in the air. Plus: a `Kinematic` body spawned with `inv_mass != 0`
  trips the gather's `debug_assert!` — the invariant that keeps `is_dynamic_row` the sole write
  predicate.

## 14. Decisions

### 14.A PERF / ARCHITECTURE — taken here, with the numbers

| # | Decision | The number that decides it |
|---|---|---|
| D-1 | Joints enter the SAME `ConstraintGraph` as a second edge kind, contacts first, per-step rebuild kept (no persistence yet) | the graph build is already inside the winning colored ratio (1.059× / 1.131×, `KE16-RESULTS.md:1286`); the loss is dispatch (0.71× at W=16); persistence changes the determinism argument for a non-bottleneck |
| D-2 | Joints solve INSIDE the colour wave, scalar, with a serial overflow lane; zero added waves | 72 waves/step is the measured problem; the `{1, N}` proof of `colored.rs:2634-2660` extends verbatim to disjoint joint rows |
| D-3 | Joint state is a durable DENSE AoS component, not a hashed table and not a 31-lane SoA | Box3D's per-joint lifetime; the dense kind exists for exactly this (`dense_store.rs:83`); SoA serves the 8-wide contact cohort only |
| D-4 | Joint kernels scalar in v1; the cohort rung is count-gated | a ragdoll yields ≤4 same-kind joints per colour [ESTIMATE from a degree-5 tree]; `COHORT = 8`; no engine widens joints; `MIN_PARALLEL_SLOTS_PER_COLOR = 256` already refuses thin colours |
| D-5 | Speculative regime = `bias = C·inv_h` **AND `mass_scale = 1`, `impulse_scale = 0`**; the soft scalings belong to the overlap branch alone; hertz clamp in prepare; no Baumgarte | `bias_rate ≈ 9.1 s⁻¹` vs `inv_h = 240 s⁻¹` at the defaults — the soft rate is the wrong regime for `C > 0` by ~26×; and at `mass_coeff ≈ 0.95` / `impulse_coeff ≈ 0.05` a bias-only branch still softens the speculative constraint, so "arrive exactly" fails ([S] `contact_solver.c`; C-1) |
| D-5b | KR-2b: in-house transcendental polynomials rather than admitting libm as a census exception | the alternative makes "exact ops only" a rule with an unenumerated hole, in the one crate whose determinism argument is that its holes are enumerated; Box2D ships `b2ComputeCosSin`/`b2Atan2` for the weaker property (C-6) |
| D-5c | `JointDef.order: u16` is the authored joint key; the dense slot is only a back-index | a dense slot is a LIFO free-list artefact (`DENSE-COMPONENTS-PLAN.md:57`), so "joint-row order" was not an order at all after the first despawn (C-7) |
| D-5d | Collision filtering is a rung (F), with BOTH arms: `layer`/`mask` live, and a joint-aware pair filter | `layer`/`mask` are read by nothing today (`components.rs:154-157` says so); a `u32` layer bit caps a per-ragdoll scheme at 32 ragdolls; Box2D filters by walking the joint edge list ([S] `body.c`) (C-5) |
| D-6 | Breaking = post-step readout of the bit-identical accumulated impulse; never inside the solver | every surveyed engine; determinism of the break SET is the property only this contract can give |
| D-7 | Character controller = query pass + component state, Box3D's plane solve with a fixed-count GS; optional inner kinematic body | Principle 0 on STATE; the shape already shipped by `soft/coupling.rs` (snapshot + grid) |
| D-8 | Soft storage = resource-owned row bank + dense handle + Gaia asset carrying its colourings; particles are not entities | 20k spawns = 0.6–2 ms (`particle.rs:5-6`); waves go from `Σ_bodies Σ_types n_colors` to `Σ_types n_colors` per substep |
| D-9 | Solver family stays XPBD/TGS-Soft; VBD/AVBD closed unless an order-independent form appears | Chebyshev / partial-Jacobi are order-dependent; the contract forbids atomics in the reduction (`soft/colored.rs:12-13`) |
| D-10 | Destruction = Blast's asset/instance split in columns; chunks are rows; promotion budgeted by count | same 0.6–2 ms number; Chaos's whole optimisation page is countermeasures for the opposite choice |
| D-11 | Bond breaks read a canonical-order impulse export; fracture ops are a recorded column | `DENSE-COMPONENTS-PLAN.md:56-58`: bit-identity only for a fixed op sequence, so the sequence is made explicit |
| D-12 | Carving uses the brick atlas as WRITTEN authority with a CPU proxy; no runtime fragment geometry | `MAX_SDF_EDITS = 16`, layout frozen; "tens of ms" refracture; Decision 6 zero-readback |
| D-13 | CPU/GPU bit-identity for soft kernels via eDSL leaves; rigid stays CPU | the mechanism the SDF field already uses; the plan's population partition |
| D-14 | Reserved high colours for static-touching constraints NOT adopted in v1 — deferred behind a NAMED fixture, not behind "statics impose no occupancy here" | ⚠ C-11: the original reason was true of Box2D too (`invMassA = 0` never enters a bitset), so it distinguished nothing. The real obstacles are (a) **there is no palette** — `color_manifolds` appends occupancy words unboundedly (`resources.rs:2884-2891`) while Box2D reserves the top of a fixed 24-colour palette, so this needs a cap + overflow lane, not a reordering; (b) it is a value change to the shipped colored path. The stacked-tunnelling fixture that decides it is on the ladder at X1 |

### 14.B VALUES / SCOPE — to the owner

1. **Sequencing against KE16**: J0/S0 can land on the shipped pool (their gates are value gates;
   the parallel touch counts will read one worker). Whether to wait for the in-scope barrier
   before measuring any advanced rung, or accept red-by-construction parallel gates meanwhile —
   the same question `OPEN-QUESTIONS.md:5000-5010` already puts to you.
2. **The reference solver's 0%-gate: bytes or values? — and the `massScale` clamp divergence.**
   (a) X0's `separation > 0` branch is value-neutral for every manifold emitted today but not
   byte-neutral. Keep `SoftStepSolver` byte-untouched (CCD lives on the colored path only) or
   redefine the reference gate as value-identity and add the branch. (b) Separately (C-1): the
   shipped kernel clamps the soft bias BEFORE multiplying by `mass_coeff`, where Box2D clamps after
   (`b2MaxFloat( softness.massScale * softness.biasRate * s, -contactSpeed )`), so
   `MAX_BIAS_VELOCITY = 4.0` actually caps push-out at ≈3.8 m/s. This ships today and is a **value**
   change to fix. Fix it with X0 under one gate (the design's recommendation — the three lines are
   being rewritten anyway, and shipping a "verbatim from Box2D" regime beside a silent Box2D
   divergence in the same expression is doc rot by construction), or pin the current behaviour with
   a comment and leave it.
3. **Ragdoll ownership**: this campaign closes AK-4 (J0–J3 + R data); the per-bone blend (L7's
   ⑤) stays in animation. Confirm the split.
4. **Joint SIMD rung**: in scope now (behind the count gate) or shelved until a crowd fixture
   exists.
5. **Tearing**: out of the soft ladder, routed to SDF carving (recommended), or in as its own rung
   with the structure-invalidation cost accepted.
6. **Self-collision scope**: particle–particle same-body (shipped) only; vertex-face; soft↔soft;
   air meshes. Each is a separate rung with its own determinism proof.
7. **Hull collider before or after D0**: D0 on boxes (degraded, faster to a first fracture) vs
   H2 first (fidelity before feature).
8. **Destruction posture**: single-player recorded op sequence (as designed) vs server-authoritative
   from day one (The Finals' model) — changes what the `FractureLog` must carry.
9. **Speculative distance default** (0 keeps the 0%-gate; Box2D/Jolt use 0.02 m) and whether
   `Bullet` is per-body (marker) or a world default.
10. **Debris budgets**: `max_promotions_per_step`, `max_sleep_time`, `removal_duration` values.
11. **`ScratchColumn` for durable banks** (KR-3 = AK-2): ratify the name or alias it.
12. **Step 6 (`IslandSleep`) and step 9 (`SoftBody`)** were held as your calls
    (`project-ecs-native-physics-lane`): this design makes step 9 = S0 and needs step 6 before D0.
    Confirm both proceed.
13. **Character controller tier**: the geometric mover (recommended, D-7) vs a rotation-locked
    rigid body (Jolt `Character`) — the latter is simpler but inherits solver jitter on slopes.
14. **Vehicle default**: wheel joints (D-2's zero-machinery path) vs raycast wheels first.
15. **Transcendentals** (C-6, D-5b): in-house polynomials in `boyko_math` with an ULP gate and a
    census extension (recommended), or admit libm and name it as a census exception. The rule as
    written today ("exact ops only") and the ConeTwist kernel as designed cannot both stand.

### 14.C The omission register — what every surveyed engine ships and this design does NOT

⚠ **CRITIQUE C-NB10.** These are not defects; each is a VALUES/SCOPE call that the first draft
simply did not put to the owner. Recorded here so that "we will implement all of it" has an explicit
boundary rather than an implicit one.

| # | Omitted | What the survey shows | Where it would attach |
|---|---|---|---|
| O-1 | **Triangle-mesh, heightfield, compound and cylinder colliders** | Jolt, Box2D v3, Chaos and DOTS all ship mesh + heightfield statics; the collider ladder here is Capsule (H0) and ConvexHull (H2) only | the SDF-first thesis plausibly covers static level geometry, and that is a *good* answer — but it is not written down as a ruling. Decide and record it |
| O-2 | **A world ray / shape query API against BODIES** | every surveyed engine ships `CastRay` / `CastShape` as public gameplay API; H1 is a solver-side cast and AK-5's interim is the SDF gradient only | H1's machinery is the implementation; what is missing is the public surface and its determinism statement (Jolt explicitly declares query result ORDER non-deterministic, RESEARCH §10) |
| O-3 | **Contact begin/end and sensor EVENTS, plus the `Contact` producer** | Box2D v3's `b2World_GetContactEvents` and sensor events are the gameplay surface joints, fracture and characters all end up needing; `Contact` exists in `components.rs:191-205` with **no producer anywhere in the crate** (grep) | KR-1's row map makes it possible and D0's impulse export walks the same canonical order; no rung currently emits it |
| O-4 | **Runtime motion-type switching** (`SetMotionType`, DOTS `PhysicsMass` overrides) | every engine ships it; here it is exactly the mass-regime flip `resources.rs:3306-3316` warns about | rung K plus the per-row sleep-latch clear (C-NB4b) is most of it; the missing piece is the public API and its gate |
| O-5 | **Per-body sleep enable / threshold** (Box2D `enableSleep`, `sleepThreshold`) | per-body, not per-world | `IslandSleep` is per-world today; per-body needs a column, which is step 6's migration anyway |
| O-6 | **Gear, rack-and-pinion, pulley, path constraints** (Jolt) | listed in RESEARCH §2.1, absent from `JointKind` with no ruling | they are additional kinds in §2.5's catalogue and cost nothing structurally; the question is whether they are wanted |
| O-7 | **Jolt `SkeletonMapper`** (high→low LOD skeleton chains) | `RagdollAsset.anim_bone: u16` is a 1:1 map; the animation campaign named the mapper at `ANIMATION-RESEARCH.md:378` | `RagdollAsset` would carry a chain table; it is a Gaia bake question, and it is animation's or physics's by agreement |

## 15. Unverified / open

- **Every wave-shape claim is against a pool whose parallel physics route is not shipped**
  (RESEARCH §9, §16): the KE16 retake is owed and its first attempt found an 11× serial floor at
  1 µs bodies traced to rustc 1.98.1 (memory, 2026-09-10). Nothing here was timed.
- The joint-per-colour histogram for a ragdoll (≤4 per kind) is inferred from the graph shape, not
  counted on a fixture — the count gate of D-4 measures it.
- `HERTZ_NYQUIST_FRACTION = 0.25`, `MIN_JOINTS_PER_TASK = 8` and the fast-body `SAFETY` factor are
  estimates to fix by fixture, each labelled `[ESTIMATE]` at its definition site.
- **P-9 does not transfer as stated, and the replacement is a fixture, not an argument** (C-3):
  Jolt solves ROOT constraints last (priority increments toward the root, sorted ascending), so
  root-first colour assignment inverts it. Whether the distinction is even measurable on a 15–20-body
  ragdoll at 4 substeps × (1+2) is unknown; the root-first vs leaf-first settling fixture at rung R
  decides, and if it shows nothing, P-9 is recorded as not transferring to a sub-stepped soft solver.
- **The ULP accuracy of the KR-2b polynomials is unknown**, and G-3's cone/twist limit tolerance is
  derived FROM it, not assumed before it.
- **The `entity_row` column's residency** (a dense array over the largest live `EntityId`) is
  address-space cheap but its committed footprint depends on the id distribution, which no fixture
  here measures.
- Whether `Query` can yield an entity datum today (KR-1) was not verified beyond
  `entity_ids_slice()` existing at `archetype.rs:1701`; the `resources.rs:3633-3639` note may be
  stale.
- S1 adds constraint TYPES, each with its own colouring — within the existing per-substep soft
  waves only if the per-type colour sweeps are folded into one `pool.scope` per substep, which is
  the same barrier question as fact 2. Until then each type adds `n_colors(type)` waves per
  substep on the soft path (the rigid 72 is untouched).
- The Box2D anti-tunnelling colour ordering (D-14) is untested here; a stacked-tunnelling fixture
  would decide it.
- The `ContactImpulse` export's cost is unmeasured (one canonical walk per step, the same walk the
  warm store already does).
- `sdf_edit_list_x8`'s ±0 divergence is owner-deferred (`resources.rs:455-466`); H2's 8-wide hull
  vertex sampling waits on it or runs the scalar oracle.
- AVBD's source (published, per its project page) was not read; D-9 is revisited only if it
  exposes an order-independent variant.
- Havok's and Chaos's internals are [D]/[B] only; nothing in this design depends on them.

## 16. Critique log (round 1, 2026-09-10)

Every finding of the first critique round, with its disposition. Sources re-opened for this round:
`D:/wt/ecsnative` at HEAD `05fcbd1d` (`46c8e489` + one docs-only commit — `git diff --stat` shows
`docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md` alone, so every `file:line` in the header's provenance
note is unmoved), plus four upstream sources fetched 2026-09-10:
[S] `box2d/src/contact_solver.c`, [S] `box2d/src/math_functions.c`, [S] `box2d/src/body.c`,
[S] `JoltPhysics/Jolt/Physics/Ragdoll/Ragdoll.cpp`, [S]
`JoltPhysics/Jolt/Physics/Constraints/ConstraintManager.cpp`. **No timing was taken.**

| # | Finding | Disposition | Where |
|---|---|---|---|
| C-1 | X0 transcribed as a bias switch; the shipped kernel applies `mass_coeff`/`impulse_coeff` unconditionally under `bias_active`, so a speculative contact would still be softened | **FIXED** — X0 is now a three-variable change in both §7.1 and §2.5, with the Box2D block quoted; D-5 restated; new oracle in §2.8; ladder X0 row rewritten | §2.5, §7.1, §2.8, §11, D-5 |
| C-1b | Box2D multiplies the soft bias by `massScale` *before* the clamp; the shipped kernel does not — a second, pre-existing divergence | **FIXED (recorded)** — named as a pre-existing value bug with its consequence computed (≈3.8 m/s where `MAX_BIAS_VELOCITY` says 4.0) and routed to the owner as §14.B.2(b) | §7.1, §14.B.2 |
| C-2 | §2.4 placed `warm_start_joints(c)` "inside the colour's ONE wave"; no per-colour warm-start wave exists, and the placement is value-wrong against Box2D's stage order | **FIXED** — joint warm start joins the existing serial `warm_start_apply` pass at `colored.rs:3273`; the wave arithmetic (4 × 3 × ~6) is now written out | §2.4 |
| C-3 | "root-first row order honours Jolt's `CalculateConstraintPriorities`" is inverted, and order-within-a-colour is the wrong axis for it | **FIXED (claim withdrawn)** — Jolt increments priority TOWARD the root and sorts ascending, so root solves LAST; priority lives in the colour index, not in intra-colour order. `order` is kept as a *stability* key, the P-9 claim is dropped, and a root-first vs leaf-first settling fixture is put on rung R | §2.4, §3, §11, §15 |
| C-4 | `entity_row: SparseMap<u32>` on `SolverScratch` is three `std::Vec`s — the class Stage 4 just removed from that very resource | **FIXED** — `ScratchColumn<u32>` dense-by-`EntityId` instead, with the ceiling and the range-clear stated | §2.2, §11 |
| C-5 | Two collision-filter mechanisms named, neither exists (`collide_connected` unhonourable; `layer`/`mask` unread, and 32-ragdoll capped) | **FIXED** — new rung **F** with both arms (F1 layer/mask live, F2 joint-aware pair filter over a per-row joint CSR), gate G-12, ladder row, R's prerequisite updated | §2.11, §3, §11, §13, D-5d |
| C-6 | "exact ops only" contradicts the ConeTwist kernel, which needs `acos`/`atan2`/`sin`/`cos`; no transcendental exists in the crate today and the censuses would not see one | **FIXED** — KR-2b: in-house polynomials with an ULP gate and a census extension (Box2D precedent), the libm arm recorded as the rejected alternative and put to the owner as §14.B.15 | §2.5, §2.8, §12, §13 (G-11), §14.B.15, D-5b |
| C-7 | "joint-row order" is used as an authored order, but a dense slot is a LIFO free-list artefact | **FIXED** — `JointDef.order: u16` is the key, sorted once into `JointColumns`; every downstream ordering claim restated over it; the op-sequence caveat extended to joint spawn/despawn/break; new permutation fixture; G-5 restated | §2.1, §2.3, §2.4, §2.7, §2.8, §9.3, §13, D-5c |
| C-NB1 | The joint overflow floor was unnamed and compared a joint count against slot floors | **FIXED** — `MIN_JOINTS_PER_TASK` stated in joints with a measurement plan, and joints explicitly excluded from the whole-solve dispatch gate so `widest_color_slots()` stays unit-pure | §2.4 |
| C-NB2 | Tiny per-colour joint tasks re-create the "2.7 slots per boxed closure" regression | **FIXED** — a colour's joints ride its last contact chunk unless they clear the floor | §2.4 |
| C-NB3 | S0's "−(bodies−1) × Σ n_colors" overstates: sub-256-slot colours never dispatch, so a merged bank can ADD waves | **FIXED** — the cell is now a COUNT gate with both regimes named, and the formula is gone | §8.1, §11 |
| C-NB4a | `BodyState.kinematic` is unread; rung K's `kinematic && !simulated` would add a second dynamic predicate beside the one `contact.rs:20-38` insists must be sole | **FIXED** — kinematic defined as `Kinematic ∧ inv_mass == 0` with a gather `debug_assert!`; the bit gates the integrate decision only, never a write permission | §4, §11 |
| C-NB4b | Once K lets a platform move, a body asleep on it stays frozen; and the drive's mass-regime flip needs a latch clear | **FIXED** — an adjacent-island wake pass added to K with gate G-13; the per-row latch clear added to §3's drive | §3, §4, §13 |
| C-NB5 | `JointDef ~136 B` does not add up from its own field list | **FIXED** — field-by-field arithmetic gives 241 B of fields, 248 B at align 8 (`EntityId` is a `usize` newtype, `primitives.rs:57`); the cost model now defers to the `const _` pin | §2.1, §2.10 |
| C-NB6 | `soft/colored.rs:686-735` cited for a per-substep recolour; and `spawn_batch` cited at `ecs_master.rs:1079` while the text says `Commands` | **FIXED** — topology is `color_topology_once` (`:748`, `:840-856`, once per frame); the substep recolour is self-collision at `:936-943`; `Commands::spawn_batch` is `commands.rs:313` | §8.1, §9.3, §11 |
| C-NB7 | Destruction determinism assumes but never states single-threaded instance order; G-9's fixture could not reach the non-deterministic route | **FIXED** — the three phases are one single-threaded system in ascending instance order on one `Commands` buffer; G-9 spawns through the parallel-capable path | §9.3, §13 |
| C-NB8 | `SoftBank`/`FractureBank` on `ScratchColumn` have a boot-time ceiling the design did not name; the range free-list needs a home | **FIXED** — `POOL_MAX_ROWS` named on both arms (2²⁴ / 262,144), loud panic at spawn; the free-list moved into two `ScratchColumn<u32>`s | §8.1, §11 |
| C-NB9 | RESEARCH §0 row 3 should say the `Kinematic` bit is unread today | **FIXED, with the locator corrected** — the sentence "a `Kinematic` body's velocity feeds the one-sided contact response" is in the DESIGN at §4, not in RESEARCH §0 row 3; both sides are now explicit that the one-sidedness comes from `inv_mass == 0` (`contact.rs:81-131`) and the bit has no consumer | §4 here; RESEARCH §0 row 3 |
| C-NB10 | Seven omissions vs what Jolt / Box2D v3 / Chaos / DOTS ship, not recorded as scope decisions | **FIXED** — new §14.C omission register, O-1…O-7, each with where it would attach | §14.C |
| C-NB11 | D-14's stated reason ("statics impose no occupancy here") is equally true of Box2D and distinguishes nothing | **FIXED, and the finding is sharpened** — the reason is replaced by two obstacles specific to this tree, the first of which the critique did not name: **there is no palette at all** (`color_manifolds` grows colours unboundedly, `resources.rs:2884-2891`), so "reserved high colours" presupposes a constant that does not exist. The stacked-tunnelling fixture is named in the ladder at X1 | §2.3, D-14, §11 |

**Nothing was refuted outright.** The single partial refutation is C-NB9's locator (the quoted
sentence lives in the design, not the survey); its substance was accepted and fixed in both
documents. Two findings were *strengthened* rather than merely applied: C-NB11 (the missing palette
is a structural obstacle the critique did not name) and C-1b (the clamp divergence's numeric
consequence, ≈3.8 m/s against a constant named 4.0, was computed here).

**One thing this round did NOT do:** no gate was run and no timing was taken, so every "FIXED" above
is a fix to the *design*, and none of them is evidence about the *code*. The rungs still ship
red-first.
