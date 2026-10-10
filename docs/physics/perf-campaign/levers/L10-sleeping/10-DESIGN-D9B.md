# L10 D9b: the step record (design rev 3)

**Status.** Rev 3: the rev-2 design with the rev-3 patch applied, as one document. Rev 1 was
critiqued in round 1 (C1, W1, W2, O1, O2); rev 2 answered it and was critiqued in round 2 (W1–W3,
O1, O2); rev 3 answered round 2. Both remark tables are at the end. Implemented as commit C3d
of the L10 lane (`u/phys-l10b`); the notes on how the code realises it are in
"Implementation (C3d)" at the end.

**Line numbers.** The design was read against `u/phys-l10` at `c2dcd529`; every `file:line` below
cites that tree, not the tree C3d landed on. They are evidence of what the design read, and this
document is not an anchor-gated one.

Amends 04 D9/D6, 06 E2′ and 08 E2′/E7′. D5b's note follows as `11-DESIGN-D5B.md`.

## Goal

- **Invariant D9b: one latch per input per step.** Each input that carries a value has exactly one latch point per step, and every stage after that point reads the latched value.

  | input | write paths inside the contract | latch point |
  |---|---|---|
  | `PhysicsConfig`, `SdfField` | a field write through `ResMut`; assigning a whole new value; `insert_resource` over the existing one; a `Commands` closure doing any of these | the broadphase (D9b) |
  | the wake request | `IslandSleep::wake_all()` | the broadphase (D9b, Decision 5) |
  | `SoftBody` | component writes | the soft step, its only reader |
  | `FixedTime` | `resource_mut` | the gather, which stamps it into `cfg.dt` |
  | `Transform` of static and kinematic bodies | component writes | the S5 head `sync_transform_to_body` |
  | `BroadphaseTree` knobs | `set_brute_max_rows`, `set_query_kernel` | the broadphase, their only reader |
  | body components | component value writes | the gather, as today (see consequence 3) |

- **Consequences.**
  1. **Deferral equivalence, for every input above except body components.** A write inside a step, after its input's latch point, is bit-identical to the same write made at the next step boundary. This holds in every mode, with sleeping on or off, for rigid and soft alike.
  2. **Off ≡ Sets** holds under any schedule whose writers stay inside §1.5's contract, including body-component writes.
     - Every decision that can differ between Off and Sets is made at or after the broadphase. That is at or after the latch point of every resource input.
     - The set of rows the step writes back is the awake set, which is the same in both modes (`Obs.awake`).
     - So the premise that D-E′, E2′ and E7′ rely on ("inputs are constant within a step") holds by construction.
  3. **Body components are not deferral-equivalent inside the step's write-back window. This is today's behaviour, and D9b does not change it.**
     - The gather reads the components. From then until the write-back, the step owns the `RigidBody` of every row it writes back:
       - `physics_apply` rewrites the whole `RigidBody` of every `touched` row (`systems.rs:2010-2019`);
       - `touched` is every awake, simulated, dynamic row (`colored.rs:3955-3963`);
       - on the coupled path, `physics_soft_rigid_apply` then adds `+= reaction` to every row that has a non-zero reaction (`soft/coupling.rs:549-560`).
     - A value write to such a row inside that window is overwritten or combined with the step's write. The same write at a boundary takes effect.
     - Writes to other rows (static, kinematic, frozen or held) and to the other body components are deferral-equivalent.
     - Structural changes between the gather and the apply are outside the contract, as today (`systems.rs:1983-1995`).
     - The remedy is to order the writer before `PhysicsGatherSet` (OQ3).
- **Boundary (§1.5).**
  - **Outside the contract after setup:**
    - writes to pipeline state;
    - replacing or removing a stateful physics resource;
    - removing a value input;
    - structural changes to physics bodies between the gather and the apply.
  - **Inside the contract:** assigning or inserting a new `PhysicsConfig` or `SdfField` value. The latch copies whatever value is present.
  - **Warm start:** today it can only be toggled by replacing the solver, so it is not covered until D5b (C3e), which is sequenced before C1b.
- **Cost to Off.** At most about 30 ns per step on SDF and soft worlds, and at most about 5 ns elsewhere. No heap use. No hot loop gains a read.

## Changes in rev 2
Each row quotes the removed rev 1 text verbatim; the added text is in the section named.

| # | section | removed (verbatim, rev 1) | added | depends on |
|---|---|---|---|---|
| 1 | Goal | "Every value-bearing input that the rigid step reads after its broadphase must be the value the broadphase's classification saw. Then Off ≡ Sets holds for any schedule, not only for schedules with no mid-step writer." · "The rigid stages after the broadphase stop reading `PhysicsConfig` and `SdfField` at all." | Goal (one latch per input, deferral equivalence, scoped Off ≡ Sets, boundary) | §1.5, Decisions 4–6 |
| 2 | §1.1 | row "`warm_start_enabled` … Only through that replacement, and a replacement is a gap even at a step boundary (open question 1)" | rows for the wake request (C1), `SoftBody`, `BroadphaseTree` knobs, `FixedTime`/`Transform`; the warm row points to Decision 6 | Decision 5, Decision 6 |
| 3 | §1.4 item 4 | "It is one system that reads its inputs once, so it is its own latch and nothing upstream of it decided anything from them." | item 4 now keeps only the mode-independence claim; new item 5 corrects the size of the soft tear window (W1) | Decision 4 |
| 4 | §1.5 | (new section) | contract boundary: inputs vs pipeline state | — |
| 5 | §2 | (a) row "Invariant — Holds for every schedule" | "Holds for every schedule whose writers use the input APIs (§1.5)"; new row for the wake request | §1.5 |
| 6 | Decision 4 | "**What.** The three soft steps keep their live `Res<PhysicsConfig>` and `Res<SdfField>`." · "**Trade-off.** Within one step, the rigid SDF stage and the soft pass can see different fields under a mid-step edit. That tear exists today, D9b does not widen it, and it is recorded." | Decision 4 replaced: the pipeline registers step-record forms of the soft steps | §1.4 item 5 |
| 7 | Decisions 5, 6 | (new) | the wake counter latched by the record; the contract boundary plus D5b | §1.1, §1.5 |
| 8 | Data / API | `pub(crate) fn latch(&mut self, cfg: &PhysicsConfig, field: Option<&SdfField>, seq: u64);` | `latch(.., seq, wake_requests)`; `StepInputs.wake_requests`; `IslandSleep` wake counters; `begin_step_upto` | Decision 5 |
| 9 | Multithreading | "Config and field writers now conflict only with integrate, gather, select, the broadphase and the soft stages." | "… only with integrate, gather, select and the broadphase" | Decision 4 |
| 10 | Code sites 9 | "**Untouched:** `box_box.rs`, `reuse.rs`, `carry.rs` (thin-box lane) and `soft/*`." | `soft/solver.rs`, `soft/colored.rs`, `solver/colored.rs`, `resources_tests.rs` join the touch set | Decisions 4, 5 |
| 11 | Tests | "a new test `tests/step_inputs_access.rs`" · "Completeness: the system names after the broadphase in the `add_physics_sdf::<DefaultRigidSolver>` schedule's `system_zones()` must be ⊆ the list ∪ {the soft stages}. An empty inventory is red." | G-ACCESS moves in-crate. Its primary completeness check is an exhaustive destructure; zones are a secondary check that runs only where compiled (O1). New suite A (deferral), new arm LB6 (C1), a witness for LB5 (O2) | Decisions 4, 5 |
| 12 | Open questions | OQ1 "Proposed follow-up (D5b): the epoch covers the solver store's identity stamp. It needs a boundary arm, not a `MidStep` arm." · OQ3 "Confirm the soft-pass exclusion (Decision 4), or schedule D9b-S once its harnesses run through the plugin." · OQ6 "HEAD not verified (see Context)." | D5b is decided and sequenced (Decision 6); OQ3 is closed by Decision 4; HEAD is read from the refs | Decision 6 |
| 13 | Context | "**Not checked.** I could not confirm that HEAD is c2dcd529, because I have no shell." | HEAD confirmed from the ref files | — |
| 14 | Order | reasons 1–4 | + D5b sequenced as C3e between D9b and C1b | Decision 6 |

## Changes in rev 3
| # | section | what changed | remark | depends on |
|---|---|---|---|---|
| 1 | Goal | Each input now lists its allowed write paths. Deferral equivalence no longer covers body components. New consequence 3 describes the write-back window. The boundary is restated. | W2 | §1.5 |
| 2 | §1.5 | Replaced. The table now separates value inputs, the request input, broadphase knobs, body components, pipeline state, and replaced or removed stateful resources. | W2 | Goal, Decision 6 |
| 3 | Decision 6 | Replaced. Warm start runs iff the solver's setup flag AND the latched `cfg.warm_start` are both true. Adds a table of setup sites and the D5b note's obligations. | W3 | §1.5 |
| 4 | Public API | The code is unchanged. The contract doc sentence is replaced by one sentence per item. | W2(a) | §1.5 |
| 5 | Code sites | Adds the plugin and soft-step docs that name the public soft forms as registered, and the contract docs for stateful types. | O2, self-found | Decision 4 |
| 6 | Tests A | Rig: both writers run in every world, and `seen` is recorded before each writer's own write. Adds three pipeline shapes. DA1 is Early only. DA5's anti-vacuity is per run. DA6 runs on four soft shapes and gains a field run. DA8 moves to `at+1`. DA9 runs one field at a time with classes. | W1, O1, critic OQ1 | Decisions 4, 5 |
| 7 | Tests B | LB5 becomes an Early write plus a Late restore. Its anti-vacuity is the flip itself, measured in a reach world. | W1 | §1.1 row 3 |
| 8 | Tests C | The red-first evidence for G-ACCESS is its mutations, because the test cannot compile on c2dcd529. | self-found | — |
| 9 | Tests D | Mutation table re-derived. | W1 | Tests A, B |
| 10 | Order | Reason 2 now covers only D9b. D5b's value-neutrality becomes an obligation of its note. | W3 | Decision 6 |
| 11 | Open questions | OQ1 lists what the D5b note owes. OQ3 widened. OQ7 added (critic OQ2). | W3 | Decision 6 |

## Context and constraints
- **HEAD confirmed.** `D:/wt/merge/.git` points to `D:/claude/BoykoEngine/.git/worktrees/merge`. Its `HEAD` is `ref: refs/heads/u/phys-l10`, and the loose ref `refs/heads/u/phys-l10` is `c2dcd5294afd8289b5937cef3cb442c2d514f3fa`. Without git I cannot tell whether the working tree has uncommitted changes. The orchestrator confirms that before the developer starts.
- **What I read.** The `D:/wt/merge` tree, `plan.md`, `fix.md`, and design docs 04, 05, 06 and 08. No cargo was run. Every red prediction below is a prediction until the tester records it.
- **Prior art in this campaign.** `05-REVIEW-OF-REV2.md:118` (O6) already named this class: "D5 and D6 are checked only in the bp prologue. A user system … could call `wake_all()` or write `SdfField` … Either state this as a contract or re-check at those stages." Rev 1 closed the D5 half and missed the D6 half. Rev 2 closes both.

---

## 1. Inventory

### 1.1 Inputs that are read after the broadphase and can be written from outside the physics chain

| input | read sites (after the broadphase) | public write paths | can a window write split Off from Sets? | D9b |
|---|---|---|---|---|
| `contact_reuse`, τ | `narrowphase_step` → `ReuseStep::new` (`systems.rs:745-750`); `is_fast` (`reuse.rs:401-412`) | `ResMut<PhysicsConfig>` (every field is `pub`, `resources.rs:176-551`), `resource_mut`, `Commands::add` (`commands.rs:126`) | **Yes.** Off recomputes frozen pairs; Sets skips them. | latched |
| `dt` (narrowphase) | `systems.rs:748` | same; the gather re-stamps it (`systems.rs:239`) | **Yes, permanently** (1.4 item 3) | latched |
| `dt`, `substeps`, `gravity`, `contact_hertz`, `contact_damping`, `relax_iterations`, `simd`, `simd_solve`, `parallel_solve` | solve: `colored.rs:4053-4054`, `:4180-4189`, `:4231`, `:4297` | same | No: same live value at the same stage. It does tear Off internally (1.3). | latched (Decision 2) |
| `sleep_threshold`, `sleep_frames` | `end_step`, `colored.rs:4389` | same | No | latched |
| `sleeping`, `sleep_skip` | the solve's arm | same | Closed by W1 | latched; W1's special case deleted |
| `parallel_narrowphase` | `systems.rs:756` | same | No (L5's theorem) | latched |
| `sdf_narrowphase` | SDF stage, `systems.rs:1369` | same | In principle (±0 ties, `resources.rs:133-145`) | latched |
| `SdfField` | SDF stage, `systems.rs:1358-1405`; soft steps `soft/solver.rs:86-87`, `:123-124`, `soft/colored.rs:688-689` | `ResMut<SdfField>` `push`/`clear` (`sdf_query.rs:70-82`), wholesale `from_edits` (`:56-63`), Commands | **Yes** (the review's example) | latched; soft reads it too (Decision 4) |
| **`IslandSleep` wake request (C1)** | prologue D6 (`sleep_sets.rs:978-979`); consumed by `begin_step` in the solve (`resources.rs:4322-4327`, via `systems.rs:1922`) | `pub fn wake_all(&mut self)` (`resources.rs:4056`), the documented remedy for a moved support (`:4286`) | **Yes.** On c2dcd529: Off clears every latch at t and solves the piles. Sets' prologue saw no request, so the held rows stay at effective inverse mass 0 (`colored.rs:4098-4104`) and are restored only at t+1 by D8. | latched (Decision 5) |
| `ColoredSoftStepSolver` warm flag | broadphase epoch (`systems.rs:472`), solve (`colored.rs:1971`) | only by replacing the resource (the field is private at `colored.rs:1520`; the sole knob is `with_warm_start`, `:1590`) | through replacement | Decision 6 → D5b |
| Body components (`RigidBody`, `RigidBodyMass`, `Collider`, `Sensor`, `Simulated`, `Kinematic`) | the gather only (`systems.rs:283-302`) | queries, `get_component_mut`, Commands | No: the gather is their latch | unchanged |
| `SoftBody` | the soft step only | queries | No: the pass does not depend on the mode | the soft step is its own latch |
| `Transform` | S5 head `sync_transform_to_body` (before integrate) | queries | No | the head is its latch |
| `FixedTime` | the gather only | `resource_mut` | No | the gather latches it into `cfg.dt` |
| `BroadphaseTree::set_brute_max_rows`, `set_query_kernel` (`broadphase_tree/mod.rs:570`, `:584`) | the broadphase only | `ResMut<BroadphaseTree>` | No: the broadphase *is* the latch point, and the pair set is exact | unchanged |

### 1.2 Can the shipped schedule run a writer inside the window? No.
*(Unchanged from rev 1.)*

**Shipped code has no such writer.**
- The only in-schedule writers of `PhysicsConfig` are `physics_gather` (`systems.rs:233`) and `select_broadphase` (`broadphase_policy.rs:193`). Edges order both before the broadphase (`plugin.rs:777-798`).
- No system anywhere takes `ResMut<SdfField>`.
- The shipped scenes write `PhysicsConfig` at setup only (`playground.rs:372-389`, `_hud_probe.rs:375`).
- Tests write between `schedule.run` calls.
- The one exception is the `MidStep` rig. It forges `SystemKey`s from `PhysicsStageKeys` (`sleep_skip_bit_identity.rs:534-542`).

**User schedules: placement is deterministic per graph today.**
- The executor runs in waves, and each wave drains before the next is dispatched (`pending == running`, `schedule.rs:777-795`).
- The dispatch scan runs in topological-index order (`:1288-1341`).
- Kahn's FIFO puts systems with no predecessor first (`schedule_builder.rs:1048-1075`).
- A conflict edge carries no order (`conflict_graph.rs:23-30`).

**An unordered writer `W` in `FixedSet::Gameplay`** runs in wave 1, ahead of the chain (sync w1, integrate w2, gather w3, select w4, bp w5, np w6, SDF w7). It lands after the broadphase only in three cases:
1. It is delayed to wave 6 or later by a serial run of lower-index conflicting roots.
2. It is ordered through forged keys.
3. It is a `Commands` closure queued in the broadphase's wave and applied in the drain before the narrowphase's wave.

Once KE17's split retire lands (built but not yet read, `schedule.rs:141-169`, `:612-634`), cases 1 and 3 depend on completion timing.

### 1.3 Off alone under such a write: order-dependent and torn
- The same user system takes effect at step t or at t+1 depending on unrelated insertion order.
- The step tears. A `dt` write between the narrowphase and the solve gives `is_fast` the old value and `h` the new one. On c2dcd529, a `gravity`, `dt` or field write between the solve and the soft step reaches the soft pass but not the rigid solve.
- After D9b, a write takes effect at its input's next latch point, whatever the topology.

### 1.4 Corrections to the premises
1. **`SdfField` is `Copy`** (`sdf_query.rs:45-46`), with fixed inline arrays (`MAX_SDF_EDITS = 16`). `size_of` is **1,568 B**.
2. **The solver constants do not split Off from Sets** (1.1). They are latched anyway (Decision 2).
3. **"One step of divergence" understates the problem.** A write undone before the next broadphase is never flushed, so the divergence is permanent. `dt` is the natural case (the gather re-stamps it), and so is an SDF edit reverted inside one step.
4. **The soft pass does not depend on the mode.** It reads `BodyState` and the grid, which are identical in both modes. So it cannot split Off from Sets.
5. **(W1, corrected.)** Keeping the soft pass on live reads would widen its tear with the rigid step. The window grows from (solve, soft) to (broadphase, soft). On a coupled world, the soft→rigid reaction would then be computed against a rigid state stepped with other parameters. Decision 4 closes this.

### 1.5 The contract boundary (rev 3)

| class | members | contract |
|---|---|---|
| **Value inputs** | `PhysicsConfig`, `SdfField` | Any write lands at the next broadphase. That includes a field write through `ResMut`, assigning a whole new value, `insert_resource` over the existing one, or a `Commands` closure doing any of these. Assignment is the only way to rebuild a field (`*ResMut<SdfField> = SdfField::from_edits(..)`, `sdf_query.rs:56-63`); the rig does this at `sleep_skip_bit_identity.rs:518` and so does `benches/sdf_narrowphase_o9.rs:164`. Deferral-equivalent; Off ≡ Sets. Carried over from today: a value must respect the pipeline's wiring (`broadphase == Grid` on the coupling path, `soft/solver.rs:152-157`). Removing either resource after setup is outside the contract. |
| **Request input** | `IslandSleep::wake_all()` | The requests are counted. The broadphase latches the count, and the next solve serves exactly that count (Decision 5). Deferral-equivalent; Off ≡ Sets. |
| **Broadphase knobs** | `BroadphaseTree::set_brute_max_rows`, `set_query_kernel` | Read only by the broadphase, and no writer can run inside the broadphase. Inside the contract. |
| **Body components** | `RigidBody`, `RigidBodyMass`, `Collider`, `Sensor`, `Simulated`, `Kinematic` | Value writes are inside the contract under the write-back window rule (Goal, consequence 3): Off ≡ Sets always holds, and writes outside the window are deferral-equivalent. Structural changes (spawn, despawn, or archetype migration of a physics body) between the gather and the apply are outside the contract, as today (`systems.rs:1983-1995`). |
| **Pipeline state**: the step's own intermediate data | `SolverScratch` (`bodies_build`/`bodies_mut`/`set_bodies`/`vn_initial_build`/`clear`, `resources.rs:4908-4968`, and `touched`); `ContactPairs::pairs_build` (`:898`); `Manifolds::manifolds_build`/`sensor_overlaps_build` (`:2878-2884`) and `box_axis_cache` (`axis_cache.rs:394`, `:464`); `ConstraintGraph::build`; `BroadphaseTree::step_direct` (`mod.rs:628`); `SoftRigidReaction::reset`/`clear_values`; the solver's `solve_*` entries; `SleepSets`; `StepInputs` itself | Outside the contract at any time after setup. |
| **Stateful resources, replaced or removed** | `*ResMut<X> = …`, `insert_resource` over an existing `X`, `remove_resource::<X>`, or `mem::swap` across worlds, for any `X` among `IslandSleep`, `ColoredSoftStepSolver`, `SoftStepSolver`, `Manifolds`, `SleepSets`, `SolverScratch`, `ContactPairs`, `ConstraintGraph`, `BroadphaseTree`, `BroadphaseGrid`, `SoftRigidReaction`, `SoftColorScratch`, `StepInputs` | Outside the contract after the first step. Before the first step it counts as setup (`sleep_skip_bit_identity.rs:2069`, `row_keyed_state_defect_a.rs:849`, `softstep.rs:1187`, `:1630`). |

**Why the line runs between value inputs and stateful resources:**
- **A value input has no state beyond its value.** The latch copies whatever value is present, so no reader can tell whether it arrived by field write, assignment or insert. Rev 2 put assignment outside the contract, even though §1.1 lists it as a write path and DA1, DA2, LB1 and LB2 perform it (critic W2a).
- **A write to pipeline state corrupts Off by itself**, so "Off ≡ Sets" is not the question for it. Examples:
  - `bodies_mut` between the gather and the apply rewrites what the apply writes back;
  - replacing `Manifolds` between the narrowphase and the solve drops the stream the solve reads.
- **Replacing a stateful resource has no exact answer, even as a latch.**
  - **`IslandSleep` replaced mid-step.** This destroys Off's per-row latch after Sets' narrowphase has already skipped the held contacts. Off solves the piles at t and Sets cannot. The latched value would be the destroyed state, so there is nothing to evaluate.
  - **Solver replaced.** Off loses the frozen islands' warm records, while Sets' `kept_rec` survives. A flush at the next broadphase restores from `kept_rec`, which Off no longer has. A mid-step replacement diverges at t for every island restored at t. Making either case exact would need per-record store identity.
- **At a step boundary, replacing `IslandSleep` happens to be exact through D7.**
  - The prologue's latch-cursor peek classifies the step as Reset, so `cursors_ok` is false and Sets flushes (`sleep_sets.rs:967-972`).
  - Both solves' `rekey_rows` then set `reset_wake` (`resources.rs:4158-4160`).

  This is not promised: it stays outside the contract, like every other stateful replacement.
- **Access-set gates cannot express this boundary**, because the solve must hold `ResMut<IslandSleep>` and `ResMut<SolverScratch>`. The boundary is a documented contract. The behavioural arms (§Tests) enforce it for inputs.

---

## 2. Options, priced
*(Unchanged except the rows marked "rev 2".)*

| | (a) Latch at the broadphase into a step record | (b) Make the write impossible: every writer ordered outside the chain | (c) Out of scope, plus a debug assert | (d1) Physics in its own sub-schedule | (d2) Latch at the gather into `SolverScratch` |
|---|---|---|---|---|---|
| Closes Commands, forged keys and exclusive systems | **Yes** | No (a command's `apply(&mut EcsMaster)` declares no access) | No | Yes | Yes |
| Closes the wake request (rev 2) | **Yes** (Decision 5) | No: `wake_all()` runs through the same writers | No | Yes | Only if the gather also latches the count |
| Runtime cost on Off | ≤ ~30 ns, no heap | ≥ 1 extra barrier round per Fixed run when a writer shared the head's wave (µs, unmeasured) | 0 | Physics becomes a barrier | same as (a) |
| Parallelism | **Improves**: post-broadphase stages, soft included, stop conflicting with config and field writers | Worse | Unchanged | Much worse | same as (a) |
| API / existing code | No public signature change. Shipped wave composition unchanged. | A new kernel set-fence check; breaks the rig | None | Rewires `FixedSet`, Snapshot ordering and `PhysicsStageKeys` | Breaks P3 (`select_broadphase` writes `cfg.broadphase` after the gather); needs a 100 B copy per solve call |
| Invariant (rev 2) | Holds for every schedule whose writers use the input APIs (§1.5) | Only for systems the check can see | Release diverges; Off tears | Holds | Holds |

**Details:**
- **(a), `SdfField`.** Copy the field. A tick-based pin is unavailable:
  - `gen` restarts at 1 (`sdf_query.rs:56-63`), and D10 rejected it.
  - The kernel has no resource change ticks (`resmut.rs:53-58`).
  - Even a sound tick leaves no exact response to a mismatch.
- **(c) is unacceptable:**
  - release builds diverge;
  - a transient write diverges permanently;
  - the assert fires only if a test happens to reproduce the placement;
  - Off stays torn.
- **(d3) "an SDF edit is a world edit".** The boundary half is (a). The localized half is D10b (OQ2).
- **Precedent.** Box2D asserts `!IsLocked()` in its setters during `Step`; Avian runs physics in its own schedule. (a) gets the same "no mid-step write is observed" property without Avian's barrier.

---

## 3. Recommendation: D9b = option (a), the step record

### Decision 1: one resource, `StepInputs`, latched first thing in every broadphase variant
**What.** `physics_broadphase`, `_colored` and `_colored_sdf` each begin with `inputs.latch(&cfg, field.as_deref(), scratch.rows.gather_seq(), wake)`.
- `wake` is `sleep.wake_requests()` on the colored variants and `0` on the reference variant, which has no `IslandSleep`.
- Everything after that reads the record: the kind arms, the prologue (`Prologue.inputs` replaces `.cfg` and `.field`), the epoch, and every later stage, soft included.

**Why.** Every decision that can split Off from Sets is made at or after the classification (D9). So the epoch and the D6 test are functions of exactly the values every later stage reads.

**Alternatives.**
- Store it in `SleepSets`: that resource exists only on the colored path.
- `ContactPairs`: misnamed, and not held by the SDF stage or the solve.
- `SolverScratch`: see (d2).

**Trade-off.** One resource id; the rk12/B2 census is re-copied.

### Decision 2: latch the whole `PhysicsConfig`
**What.** Copy about 100 B (`PhysicsConfig` is `Copy`, `resources.rs:176`). No stage after the broadphase names `PhysicsConfig`.

**Why.**
- The rule is mechanical and gateable.
- It closes Off's `dt` and `gravity` tears.
- It deletes W1's special case: `SleepSets::step_sleeping`, `step_seq` and `solve_sleeping` (`sleep_sets.rs:688-692`, `:761-762`, `:795-807`, `:941-943`). The solve arm reads `inputs.config().sleeping`.

**Trade-off.** About 2 ns per step.

### Decision 3: latch the field with a full `Copy`
**What.** `self.field = *f`, 1,568 B, whenever the world has an `SdfField`.

**Why.** Copying only `edits[..count]` saves ≤ 800 B (≈ 10 ns) and leaves the AABBs stale, a trap for any later `authority()` call on the record.

**Trade-off.** ≤ ~25 ns per step on SDF and soft worlds.

### Decision 4 (replaced in rev 2): the soft pass reads the record inside the pipeline
**What.**
- The plugin registers three crate-private systems, `physics_soft_step_latched`, `physics_soft_step_coupled_latched` and `physics_soft_step_colored_latched`, which read `Res<StepInputs>`.
- The public `physics_soft_step`, `physics_soft_step_coupled` and `physics_soft_step_colored` stay unchanged, as standalone forms reading `Res<PhysicsConfig>` and `Res<SdfField>`. A standalone call is its own latch: no broadphase runs before it.
- Both forms are thin wrappers over one `#[inline]` body per step that takes `(&PhysicsConfig, &SdfField, …)`.

**Why.**
- The soft pass is part of the step. On a coupled world, its reaction lands on the rigid state that the same step solved.
- Rev 1 left the pass on live reads. That widened the rigid/soft tear to (broadphase, soft), which the critic measured (W1).
- With the latched forms there is no exception to the invariant:
  - G-ACCESS covers the soft stages;
  - config and field writers stop conflicting with them (a sparser graph);
  - the 13+ standalone harnesses (`soft_body_sp1.rs:75`, `soft_body_sp2.rs:258`, `ke16_app1_soft_colored_from_a_worker.rs:144`, `soft_colored_sp4*.rs`, `benches/soft_step*.rs`, `benches/soft_colored_sp4.rs`) are untouched.

**Alternatives.**
- **One system with `Option<Res<StepInputs>>` and a live fallback chosen by seq.** Rejected:
  - it keeps both accesses, so writers still conflict;
  - it adds `Res<SolverScratch>` to two soft steps only for the seq test;
  - it carries a branch the pipeline never takes.
- **Accepting the tear** (the critic's option 2). Rejected: it is the measured widening, on the coupling path.
- **Every harness latches a record.** Rejected: it needs a public way to write the record, and it edits 13+ files.

**Trade-off.**
- Three 5-line functions.
- The plugin's soft system names change. The tester checks any zone-name pin, and `tools/arch-graph` is regenerated if it is gated.
- `soft/solver.rs` and `soft/colored.rs` enter the touch set.

### Decision 5 (new, closes C1): the wake request is a counter, latched by the record
**What.**
- `IslandSleep.wake_all: bool` (`resources.rs:3913`) is split in two:
  - `reset_wake: bool`: internal, set by `rekey_rows`' Reset arm (`:4158-4160`) and served by the same solve. Semantics unchanged.
  - A request counter pair: `wake_requests: u64` (bumped by `pub fn wake_all()`) and `wake_served: u64` (written by `begin_step_upto`).
- The broadphase latches `wake_requests` into the record.
- Prologue D6 becomes `sleep.wake_pending(inputs.wake_requests())`, where `wake_pending(upto) = reset_wake || upto != wake_served`. This keeps today's inclusion of a Reset wake that is still pending after an early-returning solve.
- The pipeline solve calls `begin_step_upto(graph, n, inputs.wake_requests())`:
  - it clears the latches iff `reset_wake || upto != wake_served`;
  - then it sets `wake_served = upto` and `reset_wake = false`.
- Direct drive serves the live count. `begin_step(graph, n)` is `begin_step_upto(graph, n, self.wake_requests)`, and `solve_colored_sleeping` stays public and unchanged. So the 21 direct `begin_step` calls in `colored_tests.rs` (`:3004`–`:4173`) and `benches/sleeping.rs:190`, `:225` are untouched.

**Why.**
- A request raised after the latch is served at the next step in *both* modes.
- The Sets decision (the D6 flush) and the Off action (the latch clear) read one number, taken at one instant.
- Sets already acted at t+1 through D8. The change moves Off to match.

**Alternatives.**
- **Consume the request at the broadphase.** Rejected:
  - the broadphase would need `ResMut<IslandSleep>`;
  - the Reset wake must still be served in the solve after `rekey_rows`, so one state machine would be consumed at two sites.
- **A bool latched in the record, cleared by the solve.** Rejected: it swallows a request raised in the window of a step whose latch already saw one. That is observable with `sleep_frames = 1`: `end_step(t)` re-latches, and the swallowed request would have cleared that latch at t+1. The counter is exact for 16 B and one compare.
- **Declaring `wake_all` outside the contract.** Rejected: it is the documented public remedy (`resources.rs:4286`), gameplay calls it (explosions, teleports), and piles on SDF worlds are the owner's main target.

**Trade-off.**
- +16 B in `IslandSleep` and +8 B in the record.
- `resources_tests.rs:858`, `:917`, `:965` rename the field to `reset_wake`.
- A request raised after the broadphase is served at t+1 instead of t, in both modes. That is the contract.
- A `u64` counter cannot wrap in practice: 2⁶⁴ requests at 10⁹/s takes 584 years.

### Decision 6 (rev 3): stateful resources are outside the contract after setup; D5b makes warm start a latched input, ANDed with the solver's setup flag

**What.**
- §1.5 is the contract. It goes into the rustdoc listed under Public API.
- **D5b's shape is decided here. Its proof and its value table are owed by `11-DESIGN-D5B.md` (commit C3e).**
  - Add `PhysicsConfig::warm_start: bool`, defaulting to `true`. D9b's whole-config latch (Decision 2) carries it without any D9b change.
  - Each shipped solver keeps its constructor flag as a **setup value**, and the pipeline never writes it. The flags are `ColoredSoftStepSolver::warm_start_enabled` (`colored.rs:1520`) and `SoftStepSolver::warm_start_enabled` (`soft_step.rs:213`).
  - A solve runs warm iff `solver flag && cfg.warm_start`. In the pipeline, `cfg` is the record; in direct drive it is the caller's config. With the default `true`, this AND equals today's solver flag.
  - The epoch's `warm` (`systems.rs:472`, `sleep_sets.rs:296-297`) becomes the same AND, taken from the record and the solver. The solver's half is constant after setup, because replacing the solver is outside the contract. So any change in the AND is a change in the latched field, which D5's existing flag compare already sees.
- **Setup choices the note must tabulate.** I grepped the tree for `with_warm_start` and `warm_start_enabled =`; the copies under `docs/measurements/` are not compiled.

  | site | world | solver flag | `cfg.warm_start` | effective, today → after C3e |
  |---|---|---|---|---|
  | `tests/row_keyed_state_defect_a.rs:849` | pipeline, colored (the warm-off attribution arm) | `false` | `true` (default) | cold → cold |
  | `tests/softstep.rs:1187` | pipeline, `SoftStepSolver` | `warm` | `true` | `warm` → `warm` |
  | `tests/softstep.rs:1630` | same | `warm` | `true` | `warm` → `warm` |
  | `src/solver/colored_tests.rs:5217` | direct drive (`G1`), writes the field every step | `step != 20` | `true` | unchanged |
- **C3e's mandatory arms:**
  - toggle `cfg.warm_start` while islands are held, at a boundary (Off ≡ Sets; the epoch flushes);
  - the same toggle inside the window (Off ≡ Sets, plus the deferral form M == B);
  - a setup arm: a pipeline world built with `with_warm_start(false)` stays cold for the whole run, which pins the attribution site's behaviour.

**Why.**
- Warm start is the only configuration that today can only be reached by replacing a resource. As a latched config field, it falls under D9b's latch and D5's existing compare, with no new mechanism.
- **The AND replaces rev 2's "the record sets the solver's flag".** Rev 2 cited `row_keyed_state_defect_a.rs:849` as legitimate setup and then overrode it every step (critic W3), so a user's setup choice would have been silently reverted. With the AND, every setup choice keeps its effect, no site has to migrate, and the runtime switch is latched.

**Alternatives.**
- **The record overrides the solver's flag** (rev 2). Rejected: it silently reverts setup choices.
- **Migrate the sites to `cfg.warm_start` and delete `with_warm_start`.** Rejected: it removes a public setup knob to save one `&&` per step, and it changes four sites and the meaning of an attribution arm, for no performance gain.
- **An identity stamp in the epoch.** Rejected: it is not exact without per-record identity (§1.5).

**Trade-off.**
- There are two knobs, governed by one rule: warm start runs only if both allow it. Each knob's rustdoc names the other.
- Toggling warm start at runtime is not covered until C3e lands.
- One `bool` is added to `PhysicsConfig`, and the record grows with it. The implementer pins `size_of` with an LSP hover.
- Replacing a solver after setup stays outside the contract. There is no identity stamp.

### Data structures
```rust
// crates/boyko_physics/src/step_inputs.rs (new), `pub use` from lib.rs
/// The inputs one physics step runs with, latched by its broadphase (L10 D9b).
#[derive(Resource, Debug)]            // no Clone, no Default, no pub constructor or mutator
#[repr(C, align(64))]                 // resources are Box-backed (resource_api.rs:21-35 → Box::from_raw), so align is honored
pub struct StepInputs {
    field: SdfField,        // 1568 B at offset 0: `edits` starts on a cache line; valid iff has_field
    cfg: PhysicsConfig,     // ~100 B (the implementer pins size_of via the LSP hover); one read per stage per step
    seq: u64,               // RowIdentity::gather_seq() of the latch; 0 = never latched
    wake_requests: u64,     // IslandSleep::wake_requests() at the latch; 0 without IslandSleep
    has_field: bool,
}                           // ≈1.7 KB resident; Send + Sync automatically (POD)

// resources.rs, IslandSleep (cold fields, read once per step)
reset_wake: bool,           // was `wake_all`: set by rekey_rows' Reset arm, served by the next begin_step that runs
wake_requests: u64,         // bumped by pub fn wake_all(); never reset
wake_served: u64,           // the request count the last begin_step_upto served
```
Removed from `SleepSets`: `step_sleeping`, `step_seq`, their initialisers, and `solve_sleeping`.

### Public API

```rust
impl StepInputs {
    pub fn config(&self) -> &PhysicsConfig;                // the config this step runs with
    pub fn sdf_field(&self) -> Option<&SdfField>;          // the field this step runs with
    pub(crate) fn new() -> Self;
    pub(crate) fn latch(&mut self, cfg: &PhysicsConfig, field: Option<&SdfField>, seq: u64, wake_requests: u64);
    pub(crate) fn wake_requests(&self) -> u64;
    pub(crate) fn for_gather(&self, rows: &RowIdentity) -> &Self; // debug_assert seq == rows.gather_seq()
}
impl IslandSleep {
    pub fn wake_all(&mut self);                                     // signature unchanged; takes effect at the next broadphase
    pub(crate) fn wake_requests(&self) -> u64;
    pub(crate) fn wake_pending(&self, upto: u64) -> bool;           // replaces wake_all_pending
    pub(crate) fn begin_step(&mut self, graph: &ConstraintGraph, n_rows: usize);                 // direct drive: serves the live count
    pub(crate) fn begin_step_upto(&mut self, graph: &ConstraintGraph, n_rows: usize, upto: u64); // pipeline
}
impl ColoredSoftStepSolver {
    pub fn solve_colored_sleeping(/* unchanged */);                 // direct drive: serves the live count
    pub(crate) fn solve_colored_held(/* …, */ wake_upto: u64);       // pipeline: serves the latched count
}
```
Rustdoc contract sentences:

| item | sentence |
|---|---|
| `PhysicsConfig` | "Read once per step, by the broadphase; every later stage of that step runs with the value read then ([`StepInputs`]). A write takes effect at the next broadphase, whether it writes a field, assigns a new value, or calls `insert_resource`. A value must keep the pipeline's wiring (`broadphase == Grid` on the coupling path). Removing this resource after setup is not supported." |
| `SdfField` | "Read once per step, by the broadphase; every later stage of that step runs with the field read then. Assigning a field built with [`SdfField::from_edits`] is the normal way to change it, and it takes effect at the next broadphase like any other write. Removing this resource after setup is not supported." |
| `SleepSkip` | "A write to `PhysicsConfig::sleep_skip` takes effect at the next broadphase." |
| `IslandSleep::wake_all` | "Wakes every island at the next broadphase. A call made after this step's broadphase is served by the next step, in every sleep-skip mode." |
| `IslandSleep`, `ColoredSoftStepSolver` (type docs) | "Step state: replacing or removing it after the first step is not supported, because the step keeps state in it that one sleep-skip mode carries and the other rebuilds." |
| `StepInputs` (type and module docs) | "The inputs this step runs with, written only by the broadphase." The module doc carries §1.5's table. |

The body-component sentence belongs to the doc task in OQ3, not to D9b.

### Hot paths
- **The latch.** O(1): about 100 B + 16 B of stores, plus 1,568 B on SDF and soft worlds. Sequential, from L1/L2. One `Option` test.
- **Readers.** Field loads, once per stage per step. The SDF kernel streams `edits` from the record: same type, same layout, no codegen change.
- **`begin_step_upto`.** One `u64` compare replaces one `bool` test. The latch-clear loop is unchanged.
- **Soft wrappers.** They forward to the same `#[inline]` body, so there is no codegen change in the particle loops.

### Multithreading
- The broadphase writes the record serially.
- The narrowphase, SDF stage, graph, solves and soft steps read it. They are edge-ordered after the broadphase and share-read among themselves. No atomics.
- The wake counters are written only by `wake_all` (which conflicts with the broadphase's `Res<IslandSleep>`) and by the solve. The broadphase reads them.

**Access-set changes:**

| stage | change |
|---|---|
| broadphase ×3 | adds `ResMut<StepInputs>`; the reference and plain colored variants add `Option<Res<SdfField>>` (it always declares a read, `res.rs:175-186`) |
| narrowphase ×2, SDF stage, solve ×2 | `Res<PhysicsConfig>` / `Res<SdfField>` replaced by `Res<StepInputs>` |
| soft steps (latched forms) | `Res<PhysicsConfig>` + `Res<SdfField>` replaced by `Res<StepInputs>` |

- **Effect on the schedule.** Config and field writers now conflict only with integrate, gather, select and the broadphase. Shipped wave composition does not change.

### Exactness (amends 04 D9, D6 and 06/08 E2′/E7′)
- **The rule.** "Every stage after the broadphase, rigid and soft, reads `StepInputs`, which the broadphase latched before its prologue. The epoch and the D6 test are computed from the record. The solve serves exactly the wake requests the record latched."
- **What follows:**
  - Off's recompute or wake of a frozen island and Sets' skip or flush of the held one are functions of the same values and the same request count.
  - A write after the latch is invisible to both modes until the next broadphase, where the epoch or D6 flushes (E7).
  - Transient writes are invisible to both modes.

### Code sites

1. **`step_inputs.rs` (new):** the type, its unit tests and G-ACCESS. **`lib.rs`:** `pub use`.
2. **`resources.rs`:**
   - `IslandSleep` fields and methods at `:3910-3913`, `:3984`, `:4019-4023`, `:4048-4058`, `:4158-4160`;
   - `begin_step` → `begin_step_upto` (`:4301-4328`, docs `:4227-4228`);
   - contract docs on `PhysicsConfig` (`:170-176`), `SleepSkip` (`:156-168`), `IslandSleep` (type) and `wake_all` (`:4048-4056`).
3. **`plugin.rs`:**
   - insert `StepInputs::new()` unconditionally (`:615-653`);
   - register the three latched soft forms (`:893-906`);
   - **docs that name the public soft forms as the registered systems:** `PhysicsStageKeys::soft_step` (`:131-138`), `add_physics_soft` (`:391-393`, `:404-415`), `add_physics_soft_colored` (`:436-447`). They now name the latched forms and point to the public ones as standalone forms (critic O2).
4. **`systems.rs` broadphase:**
   - latch at `:351-363`, `:381-402`, `:410-432`;
   - `broadphase_sets` and `broadphase_arms` take `&StepInputs`;
   - `Prologue.inputs` replaces `.cfg`/`.field`.
5. **Narrowphase** (`:659-694`): `narrowphase_step` gets `inputs.for_gather(..).config()`.
6. **SDF stage** (`:1356-1411`): `inputs.sdf_field().expect("invariant: …")` and `config().sdf_narrowphase`.
7. **`physics_solve_colored`** (`:1916-1945`):
   - the arm is `inputs.config().sleeping`;
   - pass `inputs.config()` and `inputs.wake_requests()`;
   - docs `:1896-1910`.
8. **`physics_solve_step`** (`:1959-1969`): `inputs.config()`.
9. **`sleep_sets.rs`:**
   - `Prologue` (`:557-581`);
   - `prologue` (`:936-979`): the epoch from the record, and D6 through `wake_pending`;
   - delete W1's fields;
   - docs (`:643-652`).
10. **`solver/colored.rs`:**
    - `solve_colored_held` and `solve_colored_inner` thread `wake_upto`;
    - `begin_step_upto` at `:4087`;
    - the `ColoredSoftStepSolver` type doc gets the stateful-resource sentence.
11. **`soft/solver.rs`** (`:84-101`, `:121-…`) and **`soft/colored.rs`** (`:686-707`):
    - extract the `#[inline]` bodies and add the latched forms;
    - fix the docs that say the public forms are registered by the plugin (`soft/solver.rs:76-79`, `:103-105`; `soft/colored.rs:672-674`). This is self-found, the same defect as O2.
12. **`sdf_query.rs`:** docs only.
13. **`resources_tests.rs`** `:858`, `:917`, `:965`: `wake_all` → `reset_wake`.
14. **Untouched:** `box_box.rs`, `reuse.rs`, `carry.rs`, `colored_tests.rs` and the benches.

### Tests

#### A. Deferral equivalence (D9b's contract), in `sleep_skip_bit_identity.rs`

**Rig changes.**
- **Pipeline shapes.** `Pipeline` gains three variants:
  - `Soft`: `add_physics_soft::<DefaultRigidSolver>(.., false)`, which runs the latched `physics_soft_step` form;
  - `SoftColored`: `add_physics_soft_colored::<DefaultRigidSolver>`, which runs the latched colored form;
  - `SoftCoupledRef`: `add_physics_soft::<SoftStepSolver>(.., true)`. This is the reference broadphase with `physics_solve_step::<SoftStepSolver>` and no `IslandSleep`, so it runs with sleeping off only (critic OQ1).

  Together with `SoftCoupled`, every latched soft form and all three broadphase variants now have a behavioural arm.
- **Two writers, registered in every world of every arm** (M, B, C, R and both lockstep worlds), so all worlds of an arm run the same schedule graph. Both are placed through forged keys, as the rig does today (`:534-542`):
  - `mid_early`: `.after(broadphase).before(narrowphase)`;
  - `mid_late`: `.after(narrowphase_sdf.unwrap_or(narrowphase)).before(solve)`. `physics_build_graph` is not ordered against it; the two share no resource.
- **Each writer drains its own resource**, `MidEarly(MidWrites)` or `MidLate(MidWrites)`, and takes `ResMut<PhysicsConfig>`, `Option<ResMut<SdfField>>` and `Option<ResMut<IslandSleep>>`:
  ```rust
  #[derive(Default)]
  struct MidWrites {
      cfg: Option<fn(&mut PhysicsConfig)>,              // taken by the first run
      field: Option<SdfField>,                          // assigned wholesale, then taken
      wake: bool,                                       // one IslandSleep::wake_all(), then cleared
      applied: u32,                                     // runs that performed a write
      seen: Option<(PhysicsConfig, Option<SdfField>)>, // what this writer read on its latest run, BEFORE its own write
  }
  ```
  On every run the writer first overwrites `seen`, then performs its queued writes.
- **Definition: "the Late writer saw X at step k"** means that after step k, `MidLate.seen` satisfies X. `seen` is recorded before the Late writer's own write, so it shows the resources exactly as a post-narrowphase stage reading them live would see them, including any Early write.
- Writes are queued only in M's writers (and, in suite B, in the lockstep worlds' writers). In every other world the writers only record `seen`.
- `MidStep` is folded into this. The W1 arm migrates with its meaning unchanged.
- **`Obs`** gains `soft: Vec<[u32; 6]>`: the position and velocity bits of each particle, from `SoftBody.pos_*`/`vel_*`, empty when there are no soft bodies. Its sleep and graph fields become `Option`s, read only where `IslandSleep` and `ConstraintGraph` exist. So `SoftCoupledRef` observes bodies, pairs, manifolds and soft bits.
- **Compiles on c2dcd529.** Suites A and B use only APIs present there (`PhysicsConfig`, `SdfField`, `IslandSleep::wake_all`, `Manifolds::pair_classes`, `FixedTime`, the stage keys). So the red-first pass runs on c2dcd529.

**Driver** `deferral(label, pipeline, variant, mode: Option<SleepSkip> /* None = sleeping off */, specs, steps, at, write)`. It runs three rigs:
- **M** takes the write through a placement during step `at`.
- **B** takes the same write directly before step `at+1`.
- **C**, the control, takes it before step `at`.

Checks:
- After every step, `observe(M) == observe(B)`. On failure, the first differing field is named.
- **Anti-vacuity, per run:**
  - the writer carrying M's write has `applied == 1` after step `at`;
  - C ≠ B at some step in [`at`, `at+10`], which shows that the written value reaches the trajectory. If not, the run is void, which counts as red.
- **Special cases:**
  - `dt`: B takes no write, because the gather re-stamps a boundary write by design. C sets `FixedTime` to the written value for step `at` only.
  - Transient writes (edit Early, revert Late): B takes no write. C edits before `at` and reverts before `at+1`.

**Scenes.**
- **Witness ball.** Every scene except DA7 and DA8 adds a ball rolling at 2 m/s on the floor, far from the towers. Its speed² of 4 keeps it awake in every mode.
- **Soft scene** (DA6, and DA9 on the soft shapes):
  - `Spec::floor()`, whose top face is at y = 0; one tower at x = 0; the ball;
  - the S5 braced cube (`:2027-2042`), spawned at step 0 centred at (3, 0.3, 0) so it rests on the floor;
  - `SdfField = sdf_floor(0.0)` and `soft_body = true` at step 0.

  So the particles collide with the field on every soft shape. On `Soft` and `SoftColored` the field is their only floor.

| arm | scene · pipeline · variants | write · placement | modes | red on c2dcd529 (prediction) | extra anti-vacuity |
|---|---|---|---|---|---|
| DA1 `d9b_defer_sdf_edit` | S4 tower + ball on the field · Sdf · default and Tree-brute-off × W1/8 | field → `sdf_floor(-1e-3)` · Early | off, Off, Sets | all modes at `at` (the SDF stage reads the live field) | the ball has a field manifold at `at-1` |
| DA2 `d9b_defer_sdf_edit_reverted` | same | `sdf_floor(-5.0)` Early, `sdf_floor(0.0)` Late | off, Off, Sets | all modes at `at` | the Late writer saw `sdf_floor(-5.0)` at `at` |
| DA3 `d9b_defer_reuse` | towers + ball · Default · `Variant::cell({Grid, Tree}, reuse = true, W1/8)` | flip `contact_reuse` · Early | off, Off, Sets | off and Off at `at`. Sets is green: held pairs are skipped, and the ball's pair is not a box pair. The control carries the arm there. | the Off world's `pair_classes().reused > 0` at `at-1` |
| DA4 `d9b_defer_tau` | same | τ = 0 · Early | same | same as DA3 | same |
| DA5 `d9b_defer_dt` | towers + ball · Default · W1/8 | `cfg.dt = 2·DT`: run 1 Early, run 2 Late | off, Off, Sets | all modes at `at`. Run 1 reaches the ball's `h` through the narrowphase and the solve; run 2 through the solve. | run 1: the Late writer saw `dt == 2·DT` at `at`; run 2: `MidLate.applied == 1` |
| DA6 `d9b_defer_soft` (W1's witness) | soft scene · `SoftCoupled`, `Soft`, `SoftColored`, `SoftCoupledRef` · W1/8 | run 1: `gravity.y *= 0.5` Late; run 2: field → `sdf_floor(0.05)` Late | off, Off, Sets; `SoftCoupledRef`: off | every cell at `at`. Run 1 goes through the rigid solve and the soft step; run 2 through the soft step alone. | C ≠ B holds in `Obs.soft` itself, not only in the rigid fields |
| DA7 `d9b_defer_wake_all` (C1) | towers · Default · `arm_variants()` | `wake_all()` · Early; again Late | Off, Sets | both modes at `at` (the solve consumes the live flag; the latch fingerprint differs) | frozen rows == dynamic rows at `at-1` |
| DA8 `d9b_defer_wake_all_twice` | towers, `sleep_frames = 1` · Default · `arm_variants()` | M and B both get a wake before `at`. Then M gets a second wake in `at`'s window (Early), and B gets its second wake before `at+1`. | Off, Sets | both modes at **`at+1`**, not at `at` (see the reason below the table) | in B, some row re-latches at `at` (visible in the latch fingerprint or `frozen` after `at`) |
| DA9 `d9b_defer_every_field` | below | | | | |

- **Why DA8 goes red at `at+1`.** On c2dcd529, M's window request sets a `bool` that the pre-`at` request already set, so M's solve at `at` serves both requests at once. M and B agree through `at`. At `at+1`, B's second request clears every latch, while M holds (Sets) or freezes (Off) the rows that `end_step(at)` re-latched.
- **Dropped: DA1's Late run.** On the Sdf pipeline, no stage after the SDF stage reads the field. A Late field write is therefore deferred on both trees and under every mutation, so the run could not fail (critic W1). The field's only post-SDF-stage reader is the soft step, which DA6 run 2 witnesses.

**DA9: every `PhysicsConfig` field, one at a time.**
- **Setup.**
  - Shapes: towers + ball on `Sdf`; the soft scene on `SoftCoupled` and `SoftColored`.
  - Modes: off and Sets. Off exercises every awake-path read; Sets exercises the prologue, the epoch and the held paths.
  - Placement: Early.
  - One `#[test]` per shape, so the harness runs them in parallel. Each (field, shape, mode) cell is one M/B/C triple of `at + 10` steps, with `at = HELD_BY`.
- **Classification.** An exhaustive destructure of `PhysicsConfig` assigns each field a class **per shape**. A new field fails to compile until it is classified. The classes below are predictions; the tester's first run confirms them.

| class | fields (prediction) | DA9 does | "no stage reads it live" is carried by |
|---|---|---|---|
| observable | `gravity`, `dt` (special case), `substeps`, `relax_iterations`, `contact_hertz`, `contact_damping`, `contact_reuse`, `contact_reuse_distance`, `sleeping`, `sleep_skip`, `sleep_threshold` (set to 0), `sleep_frames` (raised by 10, so saturated debounces fall below it), `soft_body`, `soft_damping`, `soft_rest_clamp`, `soft_rigid_coupling`, `self_collision_iters`, `soft_body_colored` (on `SoftColored` only) | perturbs the field. Requires M == B in every cell, and C ≠ B in at least one mode of the shape; otherwise red with "misclassified". | DA9 and G-ACCESS |
| not observable in these scenes | **By theorem:** `simd`, `simd_solve`, `parallel_broadphase`, `parallel_narrowphase`, `parallel_solve` (the bit-identity theorems); `broadphase`, `broadphase_select` on `Sdf`, `Soft` and `SoftColored` (every kind emits the same pair set, which `PairOracle` checks for the tree); `sdf_narrowphase` (differs only in ±0 ties, `resources.rs:133-145`). **Unread here:** `soft_self_collision_colored` (self-collision is off); `soft_body_colored` on `SoftCoupled`. | perturbs the field, which exercises the latch copy. Requires M == B. No C ≠ B requirement. | G-ACCESS alone: no stage after the broadphase can declare `Res<PhysicsConfig>` |
| wiring | `colored` on every shape (it records the schedule's shape and nothing reads it, `resources.rs:303-319`); `broadphase`, `broadphase_select` on `SoftCoupled` (coupling requires the grid, `soft/solver.rs:152-157`) | not perturbed; the reason is written at the site | — |

- If the first run finds a field unobservable, the field moves to the second class, with its reason written at the site.
- So DA9 does not prove per-field completeness and does not claim to (critic O1). DA9's claim is narrower: a live read of any observable field would be caught here. The completeness claim belongs to G-ACCESS.

#### B. Off ≡ Sets under window writes (L10's invariant), lockstep arms

| arm | scene / variants | script | red on c2dcd529 | anti-vacuity |
|---|---|---|---|---|
| LB1 `s4_mid_step_sdf_edit_lands_next_step` | S4 scenes (`:1956-2007`), default and Tree-brute-off × W1/8 | `sdf_floor(-1e-3)` Early at `HELD_BY` | at `HELD_BY`: Off's SDF manifold changes, Sets keeps it | `held_rows == 3` at `HELD_BY`; `flushes == 1` and `d5_epoch` +1 at `HELD_BY+1`; Off's pile manifold bytes at +1 ≠ −1 |
| LB2 `s4_mid_step_sdf_edit_reverted_is_invisible` | same | `sdf_floor(-5.0)` Early, `0.0` Late | at `HELD_BY`, permanently | the Late writer saw the edited field at `HELD_BY`; `d5_epoch` unchanged over [`HELD_BY−1`, +10]; Off's pile stays latched through +10 |
| LB3 `s3_mid_step_reuse_toggle_lands_next_step` | `towers()`, `Variant::cell({Tree, Grid}, reuse = true, W1/8)` | flip `contact_reuse` Early | at `HELD_BY` | Off's `reused > 0` at −1; Sets flushes at +1; Off's bytes at +1 ≠ −1 |
| LB4 `s3_mid_step_reuse_distance_lands_next_step` | same | τ = 0 Early | at `HELD_BY` | same three checks |
| **LB5 `s3_mid_step_dt_write_is_invisible`** | same | `cfg.dt = DT_W` (1.0e3 s) Early at `HELD_BY`, then `cfg.dt = DT` Late at `HELD_BY` (a restore). On both trees the solve runs with `DT`; only the narrowphase's `is_fast` can see `DT_W`. | at `HELD_BY`: Off's narrowphase classifies held-island box pairs as fast and runs their full collision, while Sets skips them, so `Obs.tags` differ | per run, checks (i)–(iii) below the table |
| LB6 `s3_mid_step_wake_all_lands_next_step` (C1) | `towers()`, `arm_variants()` | `wake_all()` at `HELD_BY`, Early and (a second run) Late | at `HELD_BY`: Off clears every latch and solves the piles; Sets' held rows stay at inverse mass 0 | `held_rows == dynamic_rows` at `HELD_BY`; `flushes == 1` and `d6_wake_all` +1 at `HELD_BY+1`; Off's frozen rows are 0 at `HELD_BY+1` |
| Control `s3_mid_step_sleep_threshold_is_mode_independent` | `towers()` | `sleep_threshold = 0.0` Early | green on both trees | a wake within `HELD_BY+1..+3` in both worlds |
| W1 regression `s3_mid_step_config_writes_take_effect_on_the_next_step` | unchanged, migrated to `MidWrites` | — | — | must stay green |

**LB5 anti-vacuity, per run:**
- **(i)** The Late writer saw `dt == DT_W` at `HELD_BY`.
- **(ii) The flip itself, independent of the tree.** A reach world R (Off, same variant, idle writers) runs step `HELD_BY` with `FixedTime = DT_W`, and steps from `HELD_BY+1` on with `DT`. Requirements:
  - R's frozen rows equal its dynamic rows at `HELD_BY−1`;
  - `R.pair_classes().pairs` at `HELD_BY` is at least its value at `HELD_BY−1`;
  - `R.pair_classes().reused` at `HELD_BY` is strictly below its value at `HELD_BY−1`.

  Every row is frozen, so the poses are unchanged, and the reuse criterion reads only shapes and poses (`reuse.rs:429-453`). A reuse lost among an unchanged or larger pair set is therefore an `is_fast` flip (`reuse.rs:401-412`, `systems.rs:1155-1160`).

  If (ii) fails, `DT_W` cannot flip this scene. The arm is then void, which counts as red, and the developer raises `DT_W`.
- **(iii)** `d5_epoch` is unchanged at `HELD_BY+1`.

**Transitivity.** LB3 and LB4 also follow from DA3 and DA4 in the Off and Sets modes, plus the boundary arm `s3_epoch_changes_flush`. They are kept as the direct statement of L10's invariant. LB5 is the only arm that isolates the `is_fast` channel.

#### C. G-ACCESS

- **Location.** `step_inputs.rs`, under `#[cfg(test)]`, so it can name the crate-private soft forms.
- **Probe.** `fn w(_: ResMut<PhysicsConfig>, _: ResMut<SdfField>)`, initialized on a world wired with `add_physics_soft::<DefaultRigidSolver>(.., true)` plus an `SdfField`.
- **Must not conflict with the probe:**
  - `physics_narrowphase`, `_colored`, `physics_narrowphase_sdf`, `physics_build_graph`, `physics_solve_colored`, `physics_solve_step::<SoftStepSolver>`;
  - `physics_apply`, the three `*_latched` soft forms, `physics_soft_rigid_apply`;
  - `sync_body_to_transform`, `debug_assert_dynamic_bodies_are_roots`.
- **Must conflict with the probe** (a positive control, so the probe is not vacuous): the three broadphase variants, and the three public standalone soft forms.
- **Primary completeness check (independent of the profile tier).**
  - An exhaustive destructure of `PhysicsStageKeys` and `SceneSyncKeys`, with each field mapped by comment to a row above. A new keyed stage fails to compile until it is classified.
  - The unkeyed `physics_soft_rigid_apply`, registered `.after(apply)` with no key, is listed explicitly.
- **Secondary check, only `if SYSTEM_ZONES_COMPILED`** (`profiling/mod.rs:142`).
  - Schedule shapes: `add_physics_sdf`, `add_physics_soft(.., true)`, `add_physics_soft_colored`, `add_physics_systems_with_scene_sync`.
  - Every system name must be in the classified union: the lists above ∪ {`sync_transform_to_body`, integrate, gather, select, broadphase ×3}.
  - The test prints which checks ran, and the tester records that line.
  - DA9 is the behavioural backstop for observable fields.
- **Not expressible by access sets:** the `IslandSleep` class, because the solve holds `ResMut<IslandSleep>`. DA7, DA8 and LB6 carry it.
- **Red-first (self-found correction).** G-ACCESS names crate-private `*_latched` forms and lives in a file that does not exist on c2dcd529, so it cannot compile there. Rev 2's "red on c2dcd529" could not be executed. Its red-first evidence is the mutation table (D): each "reads the live …" mutation re-declares `Res<PhysicsConfig>` or `Res<SdfField>` on the new tree, and G-ACCESS must go red on it.

#### D. Unit tests, debug asserts, mutations

**Unit tests:**
- `latch` round-trips every `PhysicsConfig` field (exhaustive destructure) and `wake_requests`.
- `latch(None)` gives `sdf_field() == None`.
- `for_gather` panics on a stale seq (`should_panic`, debug only).
- `wake_request_after_the_latch_stays_pending`: `wake_all` → `upto` = count → `wake_all` → `begin_step_upto(upto)`. Afterwards `wake_served == upto` and `wake_pending(count)` is true; the next `begin_step_upto(count)` clears.
- `reset_wake_is_served_whatever_upto`.
- `wake_all_wakes_every_row` (direct drive, `colored_tests.rs:2994`) stays green unchanged.

**`debug_assert!`s:**
- `for_gather` checks seq == gather_seq in every reader that holds `SolverScratch`: narrowphase ×2, SDF stage, solve ×2, and the coupled soft latched form.
- `begin_step_upto` checks `wake_served <= upto && upto <= wake_requests`.
- The SDF stage checks `has_field`.

**Mutations**, each recorded red before the commit:

| mutation | expected red |
|---|---|
| the narrowphase reads the live `cfg` | DA3, DA4, DA5 run 1, DA9 (`contact_reuse`, τ, `dt`); LB3, LB4, LB5; G-ACCESS |
| the SDF stage reads the live field | DA1, DA2, DA9 (field); LB1, LB2; G-ACCESS |
| a rigid solve reads the live `cfg` | DA5 runs 1 and 2, DA6 run 1, DA9 (`gravity`, `substeps`, `relax_iterations`, `contact_hertz`, `contact_damping`); G-ACCESS. LB5 stays green, because the restore precedes the solve. |
| a latched soft form reads the live `cfg`/field (W1) | DA6 runs 1 and 2 on the shape whose form was mutated; DA9 (the soft fields); G-ACCESS |
| the reference broadphase does not latch (the record keeps its `new()` value) | debug: the `for_gather` seq assert in `physics_solve_step`; release: DA6 on `SoftCoupledRef` is void (C == B), which counts as red |
| the pipeline solve serves `sleep.wake_requests()` live (C1) | DA7 and LB6 at `at` / `HELD_BY` |
| `begin_step_upto` sets `wake_served = wake_requests` | DA8 at `at+1` |
| the latch is moved behind the prologue's early return (`sleep_sets.rs:946-950`) | debug: the seq assert; release: DA1 in sleeping-off mode (the SDF stage's `expect` fires) and DA3/DA5 in off mode (void: C == B) |
| the solve's arm reads the live `sleeping` (W1, re-expressed) | the W1 regression arm and G-ACCESS |

- **No mutation arm for the prologue's D6 read.** Reading the live count instead of the record is equivalent by construction: the prologue runs inside the system that latched, with no writer in between. The record is read only so that there is one source.

### Cost to the Off path
- **Per step:** about 100 B + 16 B of stores; on SDF and soft worlds, +1,568 B (≈ 25 lines, memcpy from L1/L2). One `u64` compare replaces a `bool` test in `begin_step_upto`.
- **Total:** ≤ ~5 ns without a field and ≤ ~30 ns with one. That is < 0.006 % of a J step (0.5–3.6 ms).
- **No change** to the heap, the I-cache hot path or any loop body. The soft wrappers forward to the same inlined body.
- **Value-neutral.** No shipped test writes inside the window, so every G1–G10 pose, GOLDEN and pin is unchanged. A "not claimed slower" row can go into the quiet window beside C1b's rows, as confirmation rather than a gate.

### Order: C3d (D9b) → C3e (D5b) → C1b

1. C1b makes Sets the default on SDF worlds. After C1b, the D9b hole and the warm-toggle hole would both ship by default. Rev 1's argument applies to D5b word for word (W2).
2. **D9b moves no value** and needs no quiet window. No shipped test or scene writes inside the window (§1.2), so every G1–G10 pose, GOLDEN and pin is unchanged.
   - **D5b's value-neutrality is not claimed here.** The AND (Decision 6) keeps every tabulated site's effective flag by construction.
   - `11-DESIGN-D5B.md` must confirm this with its site table and a green G1–G10/GOLDEN run before C3e merges.
   - If that table finds a moved value, C3e takes a quiet window.
3. C1b's pins are then taken on the final contract.
4. D9b deletes W1's special case before C1b builds on it.

### Integration and E-items
- **Lane files:** `step_inputs.rs` (new), `lib.rs`, `resources.rs`, `plugin.rs`, `systems.rs`, `sleep_sets.rs`, `solver/colored.rs`, `soft/solver.rs`, `soft/colored.rs`, `sdf_query.rs` (docs), `resources_tests.rs`, `tests/sleep_skip_bit_identity.rs`.
- **Outside the lock set**, forced by the gates:
  - `docs/measurements/rk12-id-census.tsv` and the B2 `g_res_id_census_playground` pin: +1 resource id;
  - a UG-02 ledger row, if the scanner reports `StepInputs`;
  - `tools/arch-graph` regenerated if gated (the soft system names change);
  - any zone-count pin that names the plugin's soft systems (the tester checks `profiling_zone_counts.rs`).
- **Design doc:** `10-DESIGN-D9B.md` in the L10 folder, amending 04 D9/D6, 06 E2′ and 08 E2′/E7′. D5b's note follows as `11-DESIGN-D5B.md`.

---

## Open questions

1. **D5b's design note, `11-DESIGN-D5B.md`, is owed.** Decision 6 fixes its shape (the AND) and its sequencing. The note owes:
   - an exactness proof that the epoch's warm compare stays exact across a `cfg.warm_start` toggle while islands are held (Off keeps a frozen island's records only when warm, `colored.rs:1894`; Sets keeps `kept_rec`);
   - the site table with effective values, confirmed by a run;
   - the three arms listed in Decision 6.
2. **D10b, continuous digging.**
   - Any edit flushes every held island (`sleep_sets.rs:974-991`).
   - A localized restore needs a lemma: hard ops only, with sample points outside the swept AABB of every changed edit. Smooth ops ripple globally.
   - D9b does not depend on it.
3. **Doc task: order writers before the gather.**
   - The `FixedSet`/`PhysicsGatherSet` docs should tell writers of `PhysicsConfig`, `SdfField` and `wake_all` to use `.before_set(PhysicsGatherSet)`.
   - Once KE17 lands, the step a write lands in depends on timing unless the writer is ordered. D9b removes every downstream consequence of that choice, but not the choice itself.
   - The same ordering is the remedy for body-component writes in the write-back window (Goal, consequence 3). The sentence also goes on `RigidBody`.
4. **Forged keys** (`PhysicsStageKeys` plus the public `SystemKey.0`). D9b makes them harmless for inputs. Whether to close the loophole is a kernel decision.
5. **A kernel detector for replacing stateful resources.** It would let §1.5's edge be enforced in debug builds. Resources have no change ticks today (`resmut.rs:53-58`). This is a kernel decision.
6. **Profile tier of the test build.** Whether it compiles system zones decides whether G-ACCESS's secondary check runs. The primary check and DA9 do not depend on it.
7. **Working tree at `D:/wt/merge`** (critic OQ2). HEAD is `c2dcd529`, read from the refs. Whether the tree is clean is still unverified; the orchestrator checks `git status` before the developer starts.

## Readiness checklist
- **Structure:** goal, metrics (ns, bytes, zero heap), per-decision justification, rejections and trade-offs are all stated.
- **Data:** repr, align, sizes and hot/cold order are given. The `IslandSleep` delta is +16 B, cold.
- **API:** minimal, no internal types leak, no `dyn`. Public signatures unchanged; `wake_all` keeps its signature with a stated semantic shift.
- **Multithreading:** single writer for the record; readers are edge-ordered; no atomics; Send/Sync automatic.
- **Correctness edge cases:**
  - no field (`None`);
  - stale record (the seq assert);
  - transient writes;
  - requests raised after the latch (the counter);
  - an early-returning solve (`reset_wake` and the count stay pending);
  - resource removal (expect-panic, the same class of failure as today's missing-resource panic);
  - replacement (out of contract, §1.5).
- **Validation:** suites A–D, the mutation table and the debug asserts are listed.
- **Integration:** files, E-items, value-neutrality and order are stated.
- **N/A:** `Drop` (POD); `unsafe` (none added).

## Remark → resolution (rev 2, critique r1)
| remark | resolution |
|---|---|
| **C1** `IslandSleep` wake request / replacement missed | **Accepted.** 1.1 has a wake-request row. Decision 5: a `u64` request counter latched in `StepInputs`; D6 and the pipeline solve both read the latched count; direct drive serves the live count; the internal Reset wake is split out unchanged. Replacement is mapped in §1.5: at a boundary it is exact through D7 (latch-cursor Reset → flush; both solves set `reset_wake`); mid-step it is out of contract (no exact answer exists). New arms: LB6 (Early and Late, red on c2dcd529 at `HELD_BY`), DA7, DA8. G-ACCESS cannot carry this class; stated. Every public `&mut` API on post-broadphase resources is re-walked in 1.1 and §1.5: `BroadphaseTree` knobs are read only by the broadphase; the rest is pipeline state. |
| **W1** the claim that the soft tear is not widened is false | **Accepted, option 1 in a cleaner form.** Rev 1's claim is withdrawn (§1.4 item 5). Decision 4 is replaced: the pipeline registers step-record forms of the three soft steps; the public forms stay as standalone forms. DA6 is the witness (red on c2dcd529, and red under the mutation that reads live in the soft form). |
| **W2** solver replacement neither scoped nor sequenced | **Accepted, both.** Goal and §2 scope the invariant to input APIs, with replacement out of contract (§1.5). D5b is decided (warm start becomes a latched `PhysicsConfig` field) and sequenced as C3e before C1b. Rev 1's identity-stamp idea is rejected with reasons. |
| **O1** G-ACCESS completeness depends on the profile tier and covers only one shape | **Accepted.** Primary completeness is an exhaustive destructure of `PhysicsStageKeys`/`SceneSyncKeys` plus the unkeyed soft apply. The zones cross-check runs only if compiled, over four shapes, including the coupled and scene-sync ones. DA9 is the behavioral backstop. |
| **O2** D9b-5 lacks the `is_fast` witness | **Accepted.** LB5 gains a per-run precondition: `held_rec > 0` and non-zero held-row velocity bits at `HELD_BY`. Otherwise the arm is void, which is red. |
| Critic OQ1 (latch in the record, or consume at the broadphase) | Latch in the record (Decision 5); consuming at the broadphase is rejected with reasons. |
| Critic OQ2 (boundary for `SolverScratch`, `Manifolds`, `ConstraintGraph`) | §1.5: pipeline state, outside the contract at any time after setup. |
| Critic OQ3 (HEAD) | HEAD is c2dcd529, read from the loose ref through the worktree's gitdir. Whether the working tree is clean is still unverified. |

## Remark → resolution (rev 3, critique r2)
| remark | resolution |
|---|---|
| **W1** LB5 cannot fail; the DA1 Late run and the DA8 predictions are wrong; "saw X" is undefined | **Accepted.** Five fixes: <br>• LB5 is now an Early `dt = DT_W` write plus a Late restore, so on either tree only `is_fast` can see it. <br>• The O2 precondition is replaced by the flip itself, measured in a reach world R through `FixedTime`, which works the same on both trees. <br>• DA1's Late run is dropped because it could not fail; the field's post-SDF-stage reader is witnessed by DA6 run 2 on four soft shapes. <br>• DA8 now predicts red at `at+1`, with the reason. <br>• "Saw X" is defined: both writers run in every world, and `seen` is recorded before the writer's own write. <br>The mutation table is re-derived: LB5 is red only under the narrowphase mutation, and is explicitly green under the solve mutation. |
| **W2** the contract is wrong in both directions | **Accepted, both.** <br>(a) §1.5 separates value inputs from stateful resources. Assigning or inserting a new `PhysicsConfig`/`SdfField` is a latched write, and the rustdoc says so, with the wiring constraint. <br>(b) Deferral equivalence is limited to non-body inputs. Goal consequence 3 describes the write-back window: the apply rewrites the whole `RigidBody` of `touched` rows (`systems.rs:2010-2019`, `colored.rs:3955-3963`), and the coupled reaction adds `+=` (`soft/coupling.rs:549-560`). D9b neither creates nor changes this. Off ≡ Sets holds in that window because the write-back set is the awake set. Structural changes in the window are listed as outside the contract. |
| **W3** D5b overrides a setup choice; "D5b moves no value" is unproven | **Accepted, and the decision changed, not only the wording.** Effective warm start = the solver's setup flag AND the latched `cfg.warm_start`. So `row_keyed_state_defect_a.rs:849` and `softstep.rs:1187`, `:1630` keep their effect without migrating. The site table, a setup arm and a GOLDEN run are obligations of `11-DESIGN-D5B.md`. "D5b moves no value" is removed from the Order section. The order C3d → C3e → C1b stands. |
| **O1** DA9 overclaims | **Accepted.** DA9 runs one field at a time, with a class per (field, shape). Only the observable class must show C ≠ B. The per-field completeness claim is G-ACCESS's, and the text now says so. `broadphase`/`broadphase_select` are wiring on `SoftCoupled`, and `colored` is wiring everywhere. |
| **O2** plugin docs will go stale | **Accepted.** `plugin.rs:131-138`, `:391-393`, `:404-415`, `:436-447` are in Code sites 3. The same defect in `soft/solver.rs:76-79`, `:103-105` and `soft/colored.rs:672-674` is in Code sites 11 (self-found). |
| Critic OQ1 (reference shape) | **Covered.** `SoftCoupledRef` (`add_physics_soft::<SoftStepSolver>(.., true)`) runs in DA6 with sleeping off. A missing latch in the reference broadphase makes that run void (C == B), which counts as red. |
| Critic OQ2 (clean tree) | Open question 7: the orchestrator checks. |
| Self-found | G-ACCESS's "red on c2dcd529" could not be executed; its red-first evidence is the mutation rows (Tests C). |

Files referenced (all read-only):
- `D:/wt/merge/crates/boyko_physics/src/narrowphase/reuse.rs`
- `D:/wt/merge/crates/boyko_physics/src/systems.rs`
- `D:/wt/merge/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/merge/crates/boyko_physics/src/resources.rs`
- `D:/wt/merge/crates/boyko_physics/src/plugin.rs`
- `D:/wt/merge/crates/boyko_physics/src/soft/solver.rs`
- `D:/wt/merge/crates/boyko_physics/src/soft/colored.rs`
- `D:/wt/merge/crates/boyko_physics/src/soft/coupling.rs`
- `D:/wt/merge/crates/boyko_physics/src/solver/colored_tests.rs`
- `D:/wt/merge/crates/boyko_physics/tests/sleep_skip_bit_identity.rs`
- `D:/wt/merge/crates/boyko_physics/tests/row_keyed_state_defect_a.rs`
- `D:/wt/merge/crates/boyko_physics/tests/softstep.rs`

Sources (carried over from rev 1; no new search this round):
- [Box2D b2_world.h — IsLocked](https://github.com/X-Ray-Jin/Box2D/blob/master/include/box2d/b2_world.h)
- [Box2D forum — world locked during a time step](https://forum.starling-framework.org/d/11103-what-does-it-mean-that-a-box2d-world-is-locked-in-the-middle-of-a-time-step)
- [avian3d — Physics schedule](https://docs.rs/avian3d/latest/avian3d/schedule/struct.Physics.html)
- [avian2d — PhysicsPlugins](https://docs.rs/avian2d/latest/avian2d/struct.PhysicsPlugins.html)

Files referenced:
- `D:/wt/merge/crates/boyko_physics/src/resources.rs`
- `D:/wt/merge/crates/boyko_physics/src/sleep_sets.rs`
- `D:/wt/merge/crates/boyko_physics/src/systems.rs`
- `D:/wt/merge/crates/boyko_physics/src/plugin.rs`
- `D:/wt/merge/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/merge/crates/boyko_physics/src/soft/solver.rs`
- `D:/wt/merge/crates/boyko_physics/src/soft/colored.rs`
- `D:/wt/merge/crates/boyko_physics/src/resources_tests.rs`
- `D:/wt/merge/crates/boyko_physics/tests/sleep_skip_bit_identity.rs`
- `D:/wt/merge/crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`
- `D:/wt/merge/docs/physics/perf-campaign/levers/L10-sleeping/05-REVIEW-OF-REV2.md`
- `D:/claude/BoykoEngine/.git/refs/heads/u/phys-l10`

---

## Implementation (C3d)

Implemented on `u/phys-l10b` from the trunk merge `75bea42e` (L10 C0–C3c, and L9 C4's reuse-on
default). The design's line numbers were read on `c2dcd529`; what follows names functions, not lines.

**Where each decision landed:**
- `src/step_inputs.rs` (new): `StepInputs` with `new`, `config`, `sdf_field`, `latch`,
  `wake_requests` and `for_gather`; its unit tests; and G-ACCESS. `lib.rs` declares the module
  last and re-exports the type.
- Decision 1: the three broadphase systems latch first, before the rotation. `broadphase_sets`
  takes the record, and `Prologue.inputs` replaces `Prologue.cfg`.
- Decision 2: the kind arms, the narrowphase (both), the SDF stage, the solve (both), and the
  latched soft forms read `inputs.config()`. `SleepSets::step_sleeping`, `step_seq` and
  `solve_sleeping` are deleted, and the colored solve's arm is `inputs.config().sleeping`.
- Decision 3: `latch` copies the whole `SdfField` whenever the world has one.
- Decision 4: `physics_soft_step_latched`, `physics_soft_step_coupled_latched` and
  `physics_soft_step_colored_latched` are crate-private. The plugin registers them. Each public
  form and its latched form forward to one `#[inline]` body.
- Decision 5: `IslandSleep` has `reset_wake`, `wake_requests` and `wake_served`, with
  `wake_pending(upto)`, `wake_requests()` and `begin_step_upto`. `begin_step` is
  `begin_step_upto(.., wake_requests)`. `solve_colored_held` takes `wake_upto`, and
  `solve_colored_inner` threads it as `Option<u64>`: `Some` of the latched count from the
  pipeline, and `None` from direct drive, which calls `begin_step`.
- Decision 6 and the Public API sentences: the rustdoc of `PhysicsConfig`, `SdfField`,
  `SleepSkip`, `IslandSleep` (the type and `wake_all`), `ColoredSoftStepSolver` and `StepInputs`.

**Choices the design left open, and one deviation:**
- **The plain colored broadphase latches the field but keeps its epoch field-free.** On the soft
  pipelines, `physics_broadphase_colored` runs with an `SdfField` present, and its soft step reads
  the latched field. Its sleep epoch still covers no field, as design 04 D10 scopes it:
  `broadphase_sets(.., sdf_epoch)` passes the latched field to the prologue only from
  `physics_broadphase_colored_sdf`. Feeding it on the soft pipelines would add flushes that no
  observable needs, and would move no value, only `SleepSets`' counts.
- **`physics_solve_colored` keeps `ResMut<SleepSets>` on its plain arm** (unused there now), so its
  access set, and with it the schedule's waves, are unchanged.
- **The debug `for_gather` check** runs in the narrowphase (both), the SDF stage, the solve (both)
  and the coupled latched soft form. The two uncoupled latched forms hold no `SolverScratch`.
- **Measured layout** (a size probe on msvc, removed afterwards): `StepInputs` is 1 664 B at align
  64. The field is at offset 0 (1 568 B), the configuration at 1 568 (`PhysicsConfig` is 72 B,
  not the ~100 B the design estimated), `seq` at 1 640, `wake_requests` at 1 648 and `has_field`
  at 1 656. So the latch copies 72 + 16 B, plus 1 568 B on a world with a field.

**The tests, as run:**
- **Suite A.** The rig has `MidWrites`, `MidEarly`/`MidLate`, the `mid_early`/`mid_late` writers
  registered in every world, and three new pipelines (`Soft`, `SoftColored`, `SoftCoupledRef`).
  `Obs` gains `soft`, and its sleep, graph and warm fields become optional. The deferral driver is
  `deferral_run` / `deferral`.
- **Two scene changes**, each needed by an anti-vacuity check that failed without it:
  - The soft scene sets `soft_damping = 0.02` at step 0. Undamped, the stiff cube keeps bouncing
    on the field in the uncoupled pipelines, and DA6's field run is void there.
  - DA8 sets `sleep_threshold = 1e-2` beside `sleep_frames = 1`. With the default threshold, one
    variant's woken towers do not re-latch on the step a wake is served, and the arm is void.
  - DA5's "the Late writer saw `2 DT`" is run 1's check only, as the design words it. Run 2
    checks its writer's `applied`.
- **DA9's classes, confirmed by its first run.**
  - `sleep_skip` is unobservable: Off ≡ Sets, and the field is inert with sleeping off.
  - `self_collision_iters` is unobservable: the cube's particles are 0.6 apart, past twice their
    radius.
  - The soft fields are unobservable on the SDF shape, which has no soft body.
  - `soft_rigid_coupling` is observable only on the coupled shape, and `soft_body_colored` only on
    the colored one.
  - In `Sets` mode, a C ≠ B can come from a flush alone: the tags compare exactly, and a held slot
    differs from a restored one. So `sdf_narrowphase` and `sleep_skip`, which are unobservable in
    value, print a `Sets` difference. The class check requires C ≠ B only of observable rows.
- **Suites B, C and D** are as designed.
