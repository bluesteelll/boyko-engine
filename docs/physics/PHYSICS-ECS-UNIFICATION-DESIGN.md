# Physics ECS unification - design

- **Date:** 2026-09-10
- **Code tree:** `D:/wt/joltab` @ `ca582e72` (branch `merge/ke16-into-ecsnative`)
- **Revision:** rev 2 after one critique pass
- **Research (surveys and the owner's orders):** [PHYSICS-ECS-UNIFICATION-RESEARCH.md](PHYSICS-ECS-UNIFICATION-RESEARCH.md)
- **Status:** design only. No gate was run and no timing was taken. The owner questions in section 14 (Q1-Q5) are open.

## Provenance tags

- **[S]** source read.
- **[D]** official documentation.
- **[B]** blog post or talk (recorded, not relied on).
- Anything that could not be verified is labelled "not verified", never paraphrased from memory.
- In-tree facts are cited as `path:line` with the quoted line.

## How this file is laid out

1. The rev 2 design, verbatim.
2. Critique log - pass 1: every critic finding verbatim, each followed by the action the architect took, quoted from rev 2's changelog table.
3. The critic's preserve list, verbatim.

---

# Architecture: ECS-native physics. One entity model, one system graph, and parallelism as a kernel feature (Rev 2)

Tree: `D:/wt/joltab` @ `ca582e72`. I ran nothing and edited nothing. "arith." means worked out from field types, not checked with `size_of` or an LSP hover.

**Format note.** My role definition asks for a patch from revision 2 on. The orchestrator asked for the full text. I give the full text. To keep the patch rule's guard against silent drops, the changelog table at the end quotes the removed rev-1 text verbatim for every change.

---

## 0. Corrections and new findings (read these first)

| ID | Premise in the brief or a lens | What the tree says | Consequence |
|---|---|---|---|
| X-1 | Fact (c): "broadphase serial below `MIN_PARALLEL_BODIES = 4096`" | `resources.rs:437` `broadphase: BroadphaseKind::AllPairs,` + `resources.rs:440` `broadphase_select: BroadphaseSelectMode::Manual,` → `systems.rs:299-300` `for i in 0..n {` / `for j in (i + 1)..n {` | The pyramid runs the O(n²) arm. The 4096 gate (`resources.rs:1720`) is never reached. |
| X-2 | Fact (d): a nested scope on a worker runs serially (1.01×) | `docs/threadpool/KE16-RESULTS.md:1728` `\| speed-up over \`seq\` \| **14.6×** \| 13.3× \|`; `:1730` "`par_iter` inside a scheduled system is now the SAME as from the dispatcher — 1.004 on gnu" | The pool is not the blocker; the kernel's parallel drivers are (§8). |
| X-3 | "`Entity` is not a `QueryData`", so no row→entity projection | `iters/query/query.rs:357-358` "Returns a read-only iterator yielding `(EntityId, D::Item<'_>)`"; `dense_store.rs:95` "`slot -> EntityId` (deterministic order + serde key)" | Entity identity is not a blocker. With dense slots, slot→entity is the kernel's `s2e`. |
| X-4 | (new) The cohort-stagger obligation only concerns physics | `boyko_render/src/mesh_draw.rs:429` `let u32_id = register_asset_layout::<u32>(None);`; `:446-448` `counts: ScratchColumn::new(u32_id, u32_rows),` …; `constants.rs:198-200` "**two pools whose ids are congruent mod 64 get the SAME stagger**" | Nine render lanes share one stagger. K1 is a kernel feature, not a physics chore. |
| X-5 | (new) Sleep could live on an `EnableColumn` toggled by the solver | `enable_store.rs:9` "The bit's home is `(archetype, row)`"; `:23-24` "In v1 a toggle requires `&mut EcsMaster`" | Drives D4. |
| X-6 | (new in rev 1) `Collider` can `#[require]` the dense group | `boyko_macros/src/lib.rs:120-122` "`storage = "dense"` is **supported**" | **Superseded in rev 2** by the anchored group (D13). `#[require]` expands only on command paths (W5), and has no removal path (W4). |
| X-7 | Dense `row_ptr` is untyped with a runtime stride | `dense/views.rs:251` `unsafe { self.base.add(slot * self.stride) }`; `OPEN-QUESTIONS.md:96-97` "no consistent direction, inside ±2 %" | Neutral. The typed view is for ergonomics and slices. |
| X-8 | The 34-Vec census gate pins the count | `tests/physics_vec_side_store_census.rs` lands in `ad0ebea4`, not an ancestor of `ca582e72` (lens data) | R0 restores it. |
| X-9 | (new) Dead slots need a new guard in every sweep | `solver/simd.rs:197` `if snap.simulated && is_dynamic_row(eff.inv_mass) {`; `:259-260` "blending only the `inv_mass != 0` lanes so a static lane is byte-untouched" | Gravity, integrate and refresh already skip `inv_mass == 0` lanes. Only the broadphase needs a new dead-slot guard (§10.8). |
| X-10 | (new) Hooks and dense routing fire on the raw spawn path | `ecs_master/entity_api.rs:241` "Step 6 (Phase 14a §3.2): fire on_add / on_insert hooks."; `:310-311` `for &(cid, bytes) in dense_components {` / `self.dense_insert_and_fire(entity, archetype_id, cid, bytes);`; `:318-319` `drop(scope);` / `self.drain_deferred_hook_queue();` | A kernel structural step at this site reaches `create_entity` (the bench path) and command spawns alike. Basis of D13. |
| X-11 | (new) An apply window opens only when every dispatched system is done | `schedule/schedule.rs:669` `if pending > 0 && (pending == running \|\| running == 0) {` | A command issued by the chain tail is applied only after every chain system has completed. Basis of the release point (D14). |
| X-12 | (new) Scheduled pool tasks run from two contexts | Top level: `worker.rs:96-97` `if let Some(t) = try_steal_random(…) {` / `run_task(t);`. Inside joins: `scope.rs:53` imports `run_task` for the join. `scope.rs:24-25` "a parked joiner a claimable lane for any other wave" | A task can be run from inside a blocking join. This is the root of C1. The fix keys on the run context (§10.1). |

---

## 1. Goal

**Functional:**
- Every physics datum has exactly one **authoritative** home in an ECS form, owned by the entity it describes wherever a per-entity form fits.
- A datum may also have one **derived** form. Derived forms are named as derived and carry an authority window (§4.1).
- The rev-1 phrasing "exactly one ECS form" is restated this way. Q5 puts the choice to the owner: this restatement, or the recorded design where `RigidBody` itself is the solver column.
- Every physics stage is an ECS system on the one scheduler, and all intra-system parallelism comes from kernel drivers that any crate can use.
- End state: `crates/boyko_physics/src` holds no `pool.scope` / `try_with_active_pool` call and no `Vec` field outside the owner-decided SoftBody rung.

**Performance:**
- Remove the serial fraction the pyramid measures (Amdahl at W=8: 0.43–0.53).
- Allocations go for unification, not speed (fact b).
- The unification costs nothing on the solver hot loop: argued by layout in §7, gated by G-jolt, and settled for D2 by the SP-1 spike (§12).

## 2. Context, constraints, target metrics

**Affected crates:**
- `boyko_ecs`: registry, dense storage, query drivers, `par` facade, commands.
- `boyko_threadpool`: gang, task run-context.
- `boyko_macros`: `group` / `ticks` / `anchor` keys.
- `boyko_physics`: all stages.
- `boyko_render`: K1 consumer.

**Invariants kept:**
- FMA do-not-fuse (`docs/physics/FMA-DETERMINISM.md:16` "**Recommendation: `do-not-fuse`.**").
- `{1,N}` bit-identity of the coloured solve.
- Run-to-run determinism for a fixed op sequence.
- SP4: per-element `row_ptr`, no whole-buffer reborrow.
- Dense is CPU-only (`DENSE-COMPONENTS-PLAN.md:61`).
- **New chain invariant (D14):** from `physics_prepare`'s dispatch to `physics_writeback`'s completion, no `PhysicsBody` slot changes owner or bytes through a kernel structural operation.

**Targets** (each is a gate threshold):

| Metric (Jolt pyramid, W=8) | Today | Target |
|---|---|---|
| Heap allocations per step | 331.6 | ≤ 16 after P2. The rest are scope boxes and chunks, owned by the allocator plan. |
| Physics data-path allocations per step | 0 | 0 |
| Dispatcher rounds in the physics chain | 8 | 6 (§6) |
| `pool.scope` sites in physics src | 4 (`resources.rs:1957`, `:2049`, `colored.rs:2867`, `soft/colored.rs:1122`) | 0 |
| Per-body persistent + derived bytes | 220 B (arith.) | 168 B (arith., §5) |
| Tick commit on solver-derived columns | none (scratch) | none (K2, compile-time enforced) |
| Broadphase `sqrt` per step (AllPairs, n=1241) | 2 × 769,420 | 1241 |
| T(1)/T(8) | 1.71 | ≥ 3.0 (Jolt 3.99). The §12 timing decides which rung moves it. |
| Gang empty-phase barrier, W=8 | n/a | < 1 µs |
| Group live count at step 10 in G-jolt / G-alloc | n/a | == spawned bodies (anti-vacuity, W5) |

---

## 3. Key decisions

### D1: Body identity is a stable dense-group slot, not the archetype row (preserved)

**What.** Every entity carrying `Collider` owns the dense group `PhysicsBody`. Its slot *is* `BodyIndex` for every cross-stage and cross-frame key.

**Why:**
- (a) Row keys break under swap-remove: `archetype.rs:1220` "[`RemoveOutcome::Swapped`]: swap-remove occurred", while `resources.rs:3187` `self.asleep.resize(n_rows, false);` only resizes (F-3).
- (b) Slots never move: `dense_store.rs:6-8` "deletion is tombstone + free-list, never swap-remove, so **live slots never move**".
- (c) One column spans all body archetypes.
- (d) Gather and apply no longer depend on row order (`systems.rs:1153-1155`), so both can be parallel.
- (e) Slot→entity comes free (X-3).

**Alternatives:**
- Row key: rejected (F-3, serial loops).
- Avian-style `SolverBodyIndex` component: rejected, because it duplicates `e2s`.
- ADVANCED's `row_entity` columns: superseded (§13).

**Trade-off:**
- Slot values depend on the spawn/despawn sequence (`DENSE-COMPONENTS-PLAN.md:56` "coloring DEPENDS on absolute body slot values"). They stay run-to-run bit-identical.
- Sweeps cover the high-water mark, including dying and free slots. Inertness is established per loop (§10.8), not asserted once.

### D2: Authored components stay TABLE; solver state is a derived dense group. **Recommendation, put to the owner as Q5**

**What.** `RigidBody`, `RigidBodyMass` and `Collider` stay table components. `physics_prepare` refreshes the group from them; `physics_writeback` writes it back.

**Why this is recommended:**
- The random-access solve record must pack velocity with the *derived* world inverse inertia on one line: `contact.rs:81` `pub struct BodyEffective {` = 64 B (arith.). No authored layout provides that.
- A `RigidBody`-as-group-column solve reads velocity from the 52 B `RigidBody` column and inverse mass plus inertia from another column: 2 random lines per body access instead of 1.
- The price of D2 is two sequential streams per step, both parallel under K4.

**What it replaces and what it keeps from the owner's recorded design.** Memory `project-feature-tracks.md:25` records the owner's chosen design: "the solver strides ONE dense column directly → the gather + the std::Vec BodyState/BodyEffective mirror are DELETED".
- D2 **keeps** "strides a dense column directly": `BodyVel` is a dense column, strided by `row_ptr`.
- D2 **deletes** both mirrors: id 511 `BodyState` and the `BodyEffective` rebuild.
- D2 **differs** in one point only: `RigidBody` stays table, and pose/velocity are copied in and out once per step.
- This is a reversal of a recorded owner resolution, and rev 1 made it on arithmetic alone. It is now owner question **Q5**, decided on the **SP-1 spike** (§12): coloured-solve kernel, 1-line vs 2-line body record, against the prepare + writeback cost, at W=1 and W=8, 1241 and 10k bodies, plus a gameplay `Query<&RigidBody>` dense-resolve microbench.

**Alternatives:** `RigidBody` dense in place = Q5 option (b). Split authored columns in the solve (3 lines per access): rejected.

**Trade-off:** two copies per step, and two pose homes during the chain, with the authority window in §4.1.

### D3: The dense group is SoA with co-slotted columns (preserved)

**Why:**
- The broadphase reads 16 B per body; a 168 B AoS record streams 10.5× the useful bytes.
- In-tree precedent: `resources.rs:176-178` "the SoA kernel MEASURED ~1.6× SLOWER on the AoS `BodyState`".

**Alternative:** independent dense components that happen to share slots. Rejected: a user can remove one of them.

### D4: Sleep and the per-slot gate live in the dense group (`BodyGate`), not on an `EnableColumn`

**Why:**
- The debounce counter is written every step, in slot order, for every awake body. `EnableColumn` is `(archetype,row)`-keyed and cannot be toggled by a system (X-5).
- The same 4-byte word carries the per-lane gate the integrate kernels need: SIMULATED | DYNAMIC | KINEMATIC | SEEN | AWAKE. The port of those kernels (U5a) moves the gate source from `BodyState.simulated` to this byte, and adding AWAKE lets U6 delete the frozen snapshot (`colored.rs:3183` "the O1 SIMD kernels are NOT per-lane masked").

**Alternative:** the recorded step-6 text (`ARCH-AUDIT-ECS-DATA-REMEDIATION.md:30` "sleep → `Sleeping` component + `EnableColumn` paged-bitset tag"). Surfaced as Q3.

**Trade-off:** gameplay reads sleep through `&BodyGate`.

### D5: Per-pair persistent state is one resource-owned, double-buffered `PairCache`, keyed by slots (preserved)

**Why:**
- A pair has two owners.
- Pair entities are rejected on determinism (`ARCH-AUDIT…:30`) and on spawn cost ("20k `Commands` spawns/frame = 0.6–2 ms").
- `systems.rs:422` `axis_cache.set(a, b, c.reference_axis);` mutates a shared table inside the pair loop. Read-old/write-new removes that.

**Key:** `body_a:24 | body_b:24 | feature:16`, so slot < 2^24 (debug_assert).

**Freshness:** the `fresh_step` rule (D15), not rev 1's SEEN rule.

### D6: Gameplay-visible contacts and sensor overlaps are kernel events (owner Q1)

Unchanged from rev 1, with two additions:
- S7 is ordered before S6 (§6).
- End events carry the full `Entity` stored the previous step, so a body that dies mid-chain still ends its contacts under its own handle.

### D7: Intra-system parallelism is a kernel feature: `par_range` (K5a) and the gang `par_phases` (K5b), with an enforced nesting rule

**What.** The gang has resident lanes, work-claiming, a completion-counter barrier, and bounded spin then park. `par_range` is a one-phase gang.

**New in rev 2 (C1).** A lane task runs **resident** only when a worker runs it from its top-level loop. Run from any join context, it **declines** and returns at once (§10.1). This is enforced by the task's run context, not by documentation.

**Why:**
- Physics hand-rolls 4 sites with 4 cut policies.
- Every wide colour pays `thread_pool.rs:328` `let shared = Box::new(ScopeShared::new(` plus a chunk, a join and a wake/park per wave.
- Box2D and Jolt keep lanes resident [S].

**Alternatives:**
- A join-style in-scope barrier: removes scope setup, not per-wave wake/park.
- Substeps and colours as systems: ~104 phases would become ~104 rounds.

**Trade-offs:**
- Resident lanes are unavailable to other systems while their gang runs (bounded by `lanes ≤ max useful chunks`).
- A declined lane is lost parallelism, never lost correctness: lane 0 alone can complete every phase.

### D8: Substeps and colours are phases inside `physics_solve` (preserved)

### D9: Cohort identity for storage is a kernel feature (K1, preserved)

Evidence:
- `constants.rs:202-205` "an obligation the kernel cannot discharge for it";
- `scratch_ids.rs:541` `const SCRATCH_REGION_MIN_ID: usize = MAX_COMPONENTS - 128;`;
- X-4.

### D10: Untracked dense storage with **compile-time** refusal of change detection (K2, rewritten for C3)

**What:**
- `#[component(storage = "dense", ticks = "none")]` emits `const TICKS_TRACKED: bool = false`.
- `Ref<T>`, `Mut<T>`, `Added<T>` and `Changed<T>` const-assert `T::TICKS_TRACKED`. The precedent is `iters/query/filter.rs:969-974` `pub const fn assert_storage_supports_change_detection() {` `assert!(` `!C::STORAGE_IS_BITSET,`, with the rationale at `component.rs:61-63` "compile-rejected rather than silently compiling to a lie".
- `&T` and `&mut T` stay legal: `write.rs:83-86` "`&mut T` writes the underlying value but does NOT consult per-row tick fields".
- Every runtime tick consumer of `DenseStore` is routed around untracked stores (§8 K2 table).

**Why:** derived solver columns never read ticks, and a tracked pool commits "8 B/row of change detection … a 192 KiB resident floor per column" (`scratch_column.rs:70-73`). Rev 1's debug-assert guard was unsound in release (C3).

### D11: Split `PhysicsConfig` into user config and per-step `PhysicsStep` (preserved)

### D12: Colliders without `RigidBody` enter physics as static bodies

This fixes F-1. It is now a direct consequence of D13 (`Collider` anchors the group). The pose source is `RigidBody` if present, else `GlobalTransform`.

### D13 (new, W4/W5): Group membership is **anchored** on `Collider` by the kernel's structural core

**What.**
- `#[component(anchor_group = PhysicsBody)]` on `Collider`.
- The kernel inserts the group, with every column at its `DEAD` value, wherever `Collider` is attached. It removes the group wherever `Collider` is detached or the entity despawns.
- The attach site is the one where dense routing already runs on every spawn path (X-10: `entity_api.rs:305-312`). The detach sites are the component-remove core and `dense_despawn_fire_and_tombstone` (`entity_api.rs:1051-1060`).
- The anchor is a new `ArchetypeFlags` bit, so a world with no anchored group pays the existing `flags.is_empty()` test (`entity_api.rs:256` `if !flags.is_empty() {`).

**Why:**
- `#[require]` has no removal path (`required.rs:32-38` holds constructors only) and expands only on command paths, not on `EcsMaster::create_entity` (`entity_api.rs:171-180`). The bench and 26 `create_entity(` calls in physics use that raw path (W5).
- A user hook on `Collider` is not usable either: hooks are "Derive XOR runtime … registering hooks for a derive-hooked type panics" (`hooks/mod.rs:48-54`). The anchor must not consume the user's hook slot.

**Alternative:** a fixture migration of 23 files to command spawns. Rejected: it leaves the raw path silently group-less for users too.

**Trade-off:** a new structural-core step (cold; structural ops only).

### D14 (new, C2/W3): Deferred slot release and `DEAD` values

**What.** A group may declare `RELEASE = Deferred`. Then:
- **Remove** clears `live`, sets `s2e = TOMBSTONE`, drops the `e2s` entry, and **keeps the bytes**. The slot enters a `dying` list, not the free list. Group columns are `Copy`, so there is no drop.
- **Release** happens only through a kernel command, `Commands::release_dense_group::<G>()`, applied in an apply window. It writes each column's `const DEAD: Self` into every dying slot and moves them to the free list.
- Fresh appends are also initialised to `DEAD`.
- `physics_writeback` (S6), the chain tail, issues the release command once per step. S6 is ordered after S7.
- Because an apply window opens only when every dispatched system has completed (X-11), release never lands inside [S1, S6].

**DEAD values:**

| Column | DEAD value | Effect |
|---|---|---|
| `BodyVel` | all zero | `inv_mass = 0` ⇒ existing kernel gates skip it (X-9) |
| `BodyPose` | `position = NaN`, `bound_radius = NaN`, `rotation = IDENTITY` | Fails the broadphase predicate (§10.8) |
| `BodyMaterial` | zero | Unreachable: no pair ever names a dead slot |
| `BodyInertia` | zero | Consistent with `colored.rs:1486-1488`'s static-row assert |
| `BodyGate` | zero | Not SEEN, not SIMULATED, not DYNAMIC |

**Why:**
- Rev 1's zero-fill made a tombstone a zero-radius sphere at the origin (`components.rs:125-129` `#[repr(C)]` … `pub enum ColliderShape {` `Sphere {`, tag 0). That is C2.
- Freeing mid-chain let S3 read a freshly freed slot, and let S7 map a reused slot to a new entity (W3).
- With deferred release, a body despawned mid-chain keeps its bytes, is solved to the end of that step, is not written back (its row is gone), and its slot is recycled only after the chain.

**Semantics:** a structural change mid-chain takes effect at the next step boundary. That is today's semantics as well; today it can also misapply rows in release builds, which `systems.rs:1153-1155` catches only in debug.

**Trade-off:** dying slots live until the end of the chain. Memory is bounded by despawns per step. A world that anchors the group but runs no physics must call the release itself (`EcsMaster::release_dense_group::<G>()`); a debug diagnostic fires when the dying count exceeds the live count.

### D15 (new, W2): Warm-start freshness is `fresh_step`, not SEEN

**What:**
- S1 sets `BodyGate.SEEN` and `BodyMaterial.fresh_step = step` on any row whose SEEN bit is 0. Those are fresh inserts, whose DEAD value has SEEN = 0.
- Every `PairCache` lookup (S3 axis read, S5 warm seed) is skipped when `fresh_step[a] == step || fresh_step[b] == step`.

**Proof it suffices, for any release timing.**
1. Let X be the previous tenant of slot s, and Y the next.
2. X's entries were stored at step i only if X held s at S5(i).
3. Y fills s first at S1(j). Y can hold s only after X's removal and release, so S5(i) < S1(j) and i ≤ j−1.
4. `PairCache` is double-buffered (`colored.rs:3046` `core::mem::swap(&mut self.warm_read, &mut self.warm_write);`), so step i's entries are readable only during step i+1.
5. If i+1 < j, they are gone. If i+1 = j, Y is fresh at step j and every lookup naming s is skipped. Step j's store holds only Y's entries.

**Why:** rev 1 set SEEN in S1 and looked it up in S3/S5 of the same step, so a reused slot always read as "seen" (W2).

**Cost:** one compare against a line the lookup already touches: `BodyMaterial` is read for shape (S3) and restitution/friction (S5).

**Wrap:** equality on a u32 step. A false fresh verdict after 2^32 steps costs one skipped warm start.

---

## 4. The entity model (MUST 1)

**Owning entity types:**
- Body = any entity with `Collider` (the anchor).
- Soft body = any entity with `SoftBody`.
- Joint: per ADVANCED.
- No pair, contact or island entities.

| Datum | Form | Owner | Evidence / why not a higher rung |
|---|---|---|---|
| `RigidBody` (pose, velocity) | table component, **authoritative** outside [S1, S6] | body | Authored API unchanged (D2; Q5 may change this) |
| `RigidBodyMass` | table component | body | Authored. `inv_inertia` stays dead data (F-4) |
| `Collider` | table component, **anchors** `PhysicsBody` (D13) | body | D12/D13 |
| `Simulated`, `Kinematic` | bitset EnableTag | body | `components.rs:61` `#[component(storage = "bitset")]`. Read per row at S1 in table order |
| `Sensor` | zero-size marker | body | Unchanged |
| `BodyVel` {inv_mass, world inv inertia, v, ω} | group column, untracked, **derived; authoritative within [S1, S6]** | body | The solver's random-access record |
| `BodyPose` {position, bound_radius, rotation} | group column, untracked, derived | body | Broadphase/narrowphase source; NaN sentinel when dead |
| `BodyMaterial` {shape, restitution, friction, fresh_step, mat_flags(SENSOR)} | group column, untracked | body | Refreshed on change or FRESH |
| `BodyInertia` {local inv inertia} | group column, untracked | body | Recomputed on `Changed<Collider>` / `Changed<RigidBodyMass>` / FRESH (F-4) |
| `BodyGate` {flags, asleep, below} | group column, untracked | body | D4. Replaces `resources.rs:3057` `asleep: Vec<bool>,` and `:3062` `below_count: Vec<u16>,` |
| `BodyImpulse` {dv_lin, dv_ang} | group column (soft rung) | body | Replaces row-keyed `SoftRigidReaction` |
| Contact pairs, manifolds, sensor overlaps | system scratch on K1 cohorts (per step) | — | Pair-keyed, rebuilt each step |
| `PairCache` (warm impulses + axis) | persistent resource columns, double-buffered | — | D5/D15 |
| `ConstraintGraph`, island `energy`, `frozen_islands` | system scratch | — | `resources.rs:3008` "Island ids are NOT stable" |
| `ContactColumns` (31) | system scratch (solver cohort) | — | The cache-optimal SoA (§7) |
| `BroadphaseGrid` | system scratch | — | Per step |
| Contact / sensor enter-exit, sleep / wake transitions | kernel events | bodies | D6 / D4 |
| `PhysicsConfig` / `PhysicsStep` | resources | — | D11 |
| `SoftBody` particle columns + topology | kernel segmented dense column (K7), recommended | soft body | Q2 |
| `SoftBody` per-substep scratch (10) | shared system scratch | — | Bodies are stepped serially (`soft/colored.rs:699`) |

**Vec count after U6:** 34 → 30. SoftBody remains, pending Q2 and S0.

### 4.1 Authority windows

- **Outside [S1 dispatch, S6 completion]:** `RigidBody` is authoritative and the group is stale.
- **Within:**
  - The group is authoritative for simulated dynamic bodies.
  - A `RigidBody` write by another system in that window is overwritten by S6 for rows S6 writes (`SIMULATED & inv_mass ≠ 0 & AWAKE`, the `colored.rs:3057` `if snapshot[row].simulated && is_dynamic_row(eff[row].inv_mass) {` rule). Today's gather→apply behaves the same way.
  - The supported path for gameplay writes is `.before(physics_prepare)`.
- `Transform` stays the scene's pose, synced by the existing scene systems. That is out of scope.

---

## 5. Data structures

```rust
// ── Dense group `PhysicsBody` (K3). ONE slot map (e2s/s2e/live/dying/free), N co-slotted columns.
// All columns: storage="dense", group=PhysicsBody, ticks="none" (K2), Copy, `const DEAD` (D14).
// Column ids minted contiguously at group registration (K1) ⇒ distinct stagger by construction.
// PhysicsBody: RELEASE = Deferred; anchored on Collider (D13).

#[repr(C)]                     // 64 B (arith.); one line per slot (base 64-aligned, stride 64)
pub struct BodyVel {
    inv_mass: f32,             // 0 ⇒ static/dead: solve no-op, kernels skip (X-9)
    inv_inertia_world: Mat3,   // refreshed per substep
    linear: Vec3,              // solve RMW (random, by contact body slot)
    angular: Vec3,
}                              // const _: assert!(size_of::<BodyVel>() == 64)

#[repr(C)]                     // 32 B (arith.); 2 slots per line
pub struct BodyPose {
    position: Vec3,            // DEAD = NaN
    bound_radius: f32,         // cached circumradius; DEAD = NaN; broadphase reads bytes 0..16 only
    rotation: Quat,            // DEAD = IDENTITY
}                              // const _: assert!(size_of::<BodyPose>() == 32)

#[repr(C)]                     // 32 B (arith.; ColliderShape 16 B not size_of-checked, Open Q5)
pub struct BodyMaterial {
    shape: ColliderShape,      // narrowphase dispatch (random, per pair)
    restitution: f32,          // column build (random, per manifold)
    friction: f32,
    fresh_step: u32,           // D15: step of the first S1 fill
    mat_flags: u8,             // SENSOR
    _pad: [u8; 3],
}

#[repr(C)] pub struct BodyInertia { local_inv: Mat3 }                    // 36 B; refresh, sequential
#[repr(C)] pub struct BodyGate { flags: u8, asleep: u8, below: u16 }     // 4 B
// flags: SIMULATED | DYNAMIC | KINEMATIC | SEEN | AWAKE. Read by gravity/integrate/refresh (lane mask),
// by writeback (filter) and by sleep. DEAD = 0.
// BodyImpulse { dv_lin: Vec3, dv_ang: Vec3 } 24 B — soft rung.
// Per body: 64+32+32+36+4 = 168 B (arith.) vs 220 B today. Fields private; read accessors only.
// Safe code can permute values through `&mut T` (legal: no ticks) → wrong physics, never UB:
// every solver index is bounds-derived from `len`.

pub struct ScratchCohort { first_line: u8, width: u8 }   // K1; width ≤ 64, asserted

#[repr(C)]
struct GangShared {                      // K5b; lives in lane 0's frame for the gang's duration
    claim: CachePadded<AtomicU64>,       // (phase:32 | next_chunk:32), CAS-claimed
    done: [CachePadded<AtomicU32>; 2],   // completion counter per phase parity
    completed: CachePadded<AtomicU32>,   // last fully completed phase (FINAL = u32::MAX)
    parked: CachePadded<AtomicU64>,      // bit per lane parked after spin budget (lanes ≤ 64)
    phase_n: AtomicU32,                  // n recorded by the opener; debug SPMD agreement check
    poisoned: AtomicBool,
    threads: [UnsafeCell<MaybeUninit<Thread>>; 64], // written by each lane before its first park,
                                         // published by the SeqCst `parked` RMW (§10.1)
}
```

**Drop order:**
- Group columns are POD; release writes `DEAD`, with no drop glue.
- Scratch cohorts drop with their resource.
- `GangShared` outlives every lane task because the gang's scope join precedes lane 0's frame return (the `scope.rs:1281` structure).
- `Thread` handles are dropped by lane 0 after the join.

---

## 6. The system graph (MUST 2)

All systems go in the caller's fixed schedule.

| # | System | Declared access | Internal parallelism | Replaces |
|---|---|---|---|---|
| S1 | `physics_prepare` | **Q-dyn** `Query<(&RigidBody, &RigidBodyMass, Ref<Collider>, IsEnabled<Simulated>, IsEnabled<Kinematic>, Option<&Sensor>, GroupSlot<PhysicsBody>)>`; **Q-static** `Query<(Ref<GlobalTransform>, Ref<Collider>, Option<&Sensor>, GroupSlot<PhysicsBody>), Without<RigidBody>>`; writes `DenseColumnMut<BodyVel/BodyPose/BodyMaterial/BodyInertia/BodyGate>`; `Res<PhysicsConfig>`, `ResMut<PhysicsStep>`, `Res<FixedTime>` | Two `par_iter` loops in sequence inside one system (K4). Each writes through typed views by the slot from `GroupSlot` | `physics_gather`, `select_broadphase`; `physics_integrate` in SolverOwned mode |
| S2 | `physics_broadphase` | `DenseColumn<BodyPose>`, `Res<PhysicsStep>`, `ResMut<BroadphaseGrid>`, `ResMut<ContactPairs>` | AllPairs: `par_range`, per-chunk output runs concatenated in chunk order. Grid: gang | `systems.rs:280` |
| S3 | `physics_narrowphase` | `DenseColumn<BodyPose>`, `DenseColumn<BodyMaterial>`, `Res<ContactPairs>`, `ResMut<Manifolds>`, `ResMut<PairCache>` | Gang: per-pair manifold, then prefix compaction | `systems.rs:357` |
| S4 | `physics_build_graph` | `Res<Manifolds>`, `DenseColumn<BodyVel>` (inv_mass for `is_dynamic`), `ResMut<ConstraintGraph>` | Serial | `systems.rs:1005` |
| S5 | `physics_solve` | `DenseColumnMut<BodyVel>`, `DenseColumnMut<BodyPose>`, `DenseColumn<BodyInertia>`, `DenseColumnMut<BodyGate>`, `DenseColumn<BodyMaterial>`, `Res<Manifolds>`, `Res<ConstraintGraph>`, `ResMut<ColoredSolver>`, `ResMut<PairCache>`, `Res<PhysicsConfig>`, `Res<PhysicsStep>` | One gang per step | `systems.rs:1061` + 4 scopes |
| S7 | `physics_events` | `Res<Manifolds>`, `Res<ContactPairs>`, `DenseSlots<PhysicsBody>` (s2e, read), `ResMut<ContactEventState>`, `EventWriter<…>` | Serial | new |
| S6 | `physics_writeback` | `Query<(Mut<RigidBody>, GroupSlot<PhysicsBody>)>`, `DenseColumn<BodyVel/BodyPose/BodyGate>`, `Commands` (issues `release_dense_group::<PhysicsBody>()`) | `par_iter_mut` (K4) | `systems.rs:1127` + `touched` |

**Order:**
- S1 → S2 → S3 → {S4 → S5} ∥ S7 → S6.
- S6 is `.after(S5).after(S7)`. That makes the release command land strictly after S7's last `s2e` read (D14).
- S7 is an O(p) merge walk. It is expected to be shorter than S4 + S5, so waiting for it costs nothing; §12's timing verifies this.

**Rounds:** S1, S2, S3, {S4 ∥ S7}, S5, S6 = **6**.

**Access check (W6):**
- The only intra-system write on a group node in S1 is each `DenseColumnMut<T>`, one per column id.
- `GroupSlot` declares a read of the group's slot-map node, a separate registry id (§8 K3).
- No two S1 params write the same id, so `filtered_access_set.rs` (indexed "0..512 component reads (by ComponentId.0) | 512..1024 component writes") passes.
- Rev 1's two queries both writing `PhysicsBodyMut` are gone.

**Variants (W9):**
- `physics_integrate` is registered **only** in `IntegrationMode::Foundation` (`plugin.rs:528-530`), before S1, and unchanged: it runs on authored `RigidBody`. This removes the dead round from the SolverOwned path.
- `physics_narrowphase_sdf` runs after S3 and appends to `Manifolds`, as today. It is ported to group reads in U5; it stays serial.
- The uncoupled soft step has disjoint access and may run ∥ S2…S6.
- The coupled soft step is ported to group reads in U5 (§12).

**End state:** no physics-local `pool.scope`. Every parallel construct is `par_iter` (K4), `par_range` (K5a) or `par_phases` (K5b).

---

## 7. Hot-loop layout costs (MUST 3)

**Kernel storage facts:**
- `row_ptr` = base + i·stride (`dense/views.rs:251`).
- The base is a 64 KiB reservation plus a stagger that is a 64 B multiple. Stride is 64 B for `BodyVel` and 32 B for `BodyPose`.
- Untracked commits no tick pages (`scratch_column.rs:74`).

| Loop | Today | ECS form | Verdict | Closed by |
|---|---|---|---|---|
| Broadphase AllPairs | `BodyState` 156 B stride; 193.6 KB swept ~n/2 times; `body_bounding_radius` twice per test (`systems.rs:302`) | `BodyPose` bytes 0..16. The liveness sentinel (NaN) lives **inside** those 16 B, so the byte count is unchanged: 39.7 KB (arith.) | Better: ~4.9× fewer bytes, 1.5M `sqrt` removed, same pair set on a churn-free world | K3 + D14 |
| Broadphase grid | same AoS source | same column; the count pass skips `!(r >= 0.0)` rows (one predictable branch) | Better | K3 + D14 |
| Narrowphase | 2 random bodies from a 156 B AoS | `BodyPose` + `BodyMaterial`, 1 line each; `fresh_step` on the line already read | Equal or better | K3 |
| Graph build | union-find `u32` per row | per slot; dead = static singleton | Equal | — |
| Coloured solve | `BodyEffective` 64 B, 1 line per random body | `BodyVel` 64 B, identical address arithmetic | Equal (Q5/SP-1 prices the alternative) | K1 |
| Gravity / integrate / refresh | 220 B per body streamed | `BodyVel` 64 + `BodyPose` 32 + `BodyInertia` 36 + `BodyGate` 4 = 136 B per body (arith.) | Better (−38 %) | K3 |
| Prepare | serial, AoS writes | parallel table reads + slot writes | Equal bytes, parallel | K4 |
| Writeback | serial, row-order | parallel, slot-addressed | Equal bytes, parallel | K4 |
| Soft step | per-body `Vec` | K7 segments | Equal | K7 (Q2) |

**Where the claim fails today, and what closes it:**
- the cohort stagger is a caller duty (K1);
- there is no SoA dense storage (K3);
- tracked dense pays tick pages (K2);
- dense storage has no parallel driver (K4);
- resource-column parallelism has no driver (K5);
- dead slots are not inert by default (D14 `DEAD` values).

Holes raise the working set by at most the dying count plus the free count. That is bounded by LIFO reuse (`dense_store.rs:124-125`).

---

## 8. Kernel features, ordered (MUST 4)

| # | Feature | What | Physics use | Other consumers | Prerequisite |
|---|---|---|---|---|---|
| K1 | Storage cohorts | `ScratchCohort::reserve(width)`: registry-free scratch pools, contiguous stagger run. `DenseGroup` registration mints column ids contiguously | Replaces `scratch_ids.rs` | render `MeshRenderScratch` (X-4), particles, lights; animation pose banks | — |
| K2 | Untracked dense, compile-time sound | See the table below | All group columns | render dense instance data whose ticks are unread (not verified) | — |
| K3 | Dense groups | See the list below | `PhysicsBody` | render instance families (candidate; not verified); animation skeleton instances | K1, K2 |
| K4 | KE15: `par_iter` / `par_for_each_chunk` over mixed table + dense / `GroupSlot` terms | Dense store pointers resolved once per chunk; lifts `par_iter.rs:305-311`; covers `IsEnabled` + dense data terms | S1, S6 | render dense sync, animation | K3 |
| K5a | `par_range` | A one-phase gang with a caller-supplied cut table | S2 AllPairs | render culling/batching, scene propagation | K5b |
| K5b | Gang (`par_phases`) + task run context | §10.1, including the nesting rule | S2 grid, S3, S5, soft solve | transform propagation, animation hierarchy, render multi-pass culling | KE16 (closed) |
| K6 | Group release command | `Commands::release_dense_group::<G>()` and `EcsMaster::release_dense_group::<G>()` | S6 | any deferred-release group | K3 |
| K7 | Segmented dense column | Per-entity variable-length ranges in a group bank | SoftBody (Q2) | animation bone arrays, UI text runs | K3 |

**K3, dense groups, in detail:**
- one slot map (`e2s`, `s2e`, `live`, `dying`, `free`) plus N co-slotted `Copy` columns, each with `const DEAD`;
- removal is by group only (a trybuild fixture rejects removing a member);
- `RELEASE = Immediate | Deferred` (D14);
- anchor component (D13);
- `DenseColumn<T>` / `DenseColumnMut<T>` params, one access node per column id;
- a `GroupSlot<G>` query datum that reads the slot-map node, a separate registry id;
- typed views with disjoint `range_mut`.

**Invariant, restated:**
- live ⟺ `live.test(s)`;
- dying ⟺ ¬live ∧ s ∈ dying ∧ bytes intact ∧ `s2e[s] = TOMBSTONE`;
- free ⟺ s ∈ free ∧ bytes == DEAD.

This replaces `dense_store.rs:70`'s "`s` is **live** ⟺ `live.test(s)` ⟺ `s ∉ free` ⟺ `s2e[s] != TOMBSTONE`" for deferred groups only. Immediate groups keep that invariant.

**K2 routing, one row per kernel tick consumer (C3):**

| Consumer | Site | Untracked route |
|---|---|---|
| `Ref<T>` / `Mut<T>` fetch | `dense_store.rs:738` `pub(crate) fn added_ticks_ptr(`, `:746` `pub(crate) fn changed_ticks_ptr(` | Compile-rejected by a const-assert in `Ref`/`Mut` `init_state`; the accessor debug-asserts tracked (unreachable) |
| `Added<T>` / `Changed<T>` | `filter.rs:969`, `:1300` | Second const-assert `T::TICKS_TRACKED` beside the bitset one |
| `DenseStore::insert` stamping | `dense_store.rs:257-259` `self.column.write_added_tick(…)` / `write_changed_tick(…)` | `if self.column.is_tracked()` (cold, structural) |
| `insert_with_ctor` | `dense_store.rs:372-373` | same |
| `insert_or_replace` | `dense_store.rs:418` `self.column.write_changed_tick(slot as usize, current_tick);` | same |
| Serde load stamping | `dense_store.rs:758-765` `pub(crate) unsafe fn stamp_slot_ticks(` | same. Group columns are also excluded from serde as derived data, rebuilt by S1 because a loaded slot has SEEN = 0. Not verified: whether the serde seam enumerates every dense type; if it does, K3 adds the skip flag |
| `check_ticks` dense arm | `check_ticks.rs:24-25` "every LIVE slot of every [`DenseStore`] in the world's [`DenseRegistry`]" | skip untracked stores |
| Table component with `ticks = "none"` | derive | rejected by the derive (K2 is dense-only) |

**Not needed:** an in-place `EnableMut` toggle. The dense `free: Vec<u32>` (`dense_store.rs:126`) and the new `dying` list are kernel-storage bookkeeping, settled by class under the allocator plan. `dying` goes on a `VmColumn`, the way `s2e` does (`dense_store.rs:118` `s2e: VmColumn<EntityId>,`).

---

## 9. Public API (signatures only)

```rust
// boyko_ecs — K1
impl ScratchCohort { pub fn reserve(width: usize) -> Self; }
impl<T: Copy> ScratchColumn<T> { pub fn new_in(cohort: &ScratchCohort, k: usize, reserve_rows: usize) -> Self; }

// boyko_ecs — K2 (derive-emitted; trait default true)
// trait Component { const TICKS_TRACKED: bool = true; ... }

// boyko_ecs — K3
pub enum GroupRelease { Immediate, Deferred }
pub trait DenseGroup: 'static { const WIDTH: usize; const RELEASE: GroupRelease; }
pub trait GroupColumn: Component + Copy { type Group: DenseGroup; const DEAD: Self; }
pub struct GroupSlot<G: DenseGroup>;                       // QueryData: Item = u32 slot; reads slot map
pub struct DenseColumn<'w, T: GroupColumn>;                // SystemParam, read
pub struct DenseColumnMut<'w, T: GroupColumn>;             // SystemParam, write
impl<T: GroupColumn> DenseColumnMut<'_, T> {
    pub fn solve_view(&self) -> TypedDenseView<'_, T>;     // Copy + Send + Sync
    pub fn len(&self) -> usize;                            // high-water (live + dying + free)
}
impl<T: Copy> TypedDenseView<'_, T> {
    pub unsafe fn row_ptr(&self, slot: usize) -> *mut T;
    pub unsafe fn range_mut(&self, r: core::ops::Range<usize>) -> &mut [T]; // ranges pairwise disjoint
}
pub struct DenseSlots<'w, G: DenseGroup>;                  // s2e / is_live, read
impl Commands<'_, '_> { pub fn release_dense_group<G: DenseGroup>(&mut self); }   // K6
impl EcsMaster { pub fn release_dense_group<G: DenseGroup>(&mut self); }

// boyko_ecs — K5
pub fn for_range<C: CutTable>(n_chunks: u32, cuts: &C, body: impl Fn(core::ops::Range<u32>) + Sync);
pub fn par_phases(max_useful_lanes: u32, program: impl Fn(&GangLane<'_>) + Send + Sync);

// boyko_threadpool — K5b
impl GangLane<'_> {
    pub fn for_each(&self, n: u32, body: impl Fn(u32) + Sync); // SPMD; exactly-once; phase barrier
    pub fn single(&self, body: impl Fn() + Sync);
    pub fn lane(&self) -> u32;
    pub fn lanes(&self) -> u32;
}
// internal: the Task trampoline gains a `RunCtx { Top, Nested }` argument (§10.1)

// boyko_physics
pub trait RigidSolver: Resource + 'static {                // W9: seam re-typed in U5b
    fn solve(&mut self, step: &PhysicsStep, config: &PhysicsConfig,
             manifolds: &[Manifold], bodies: SolverBodies<'_>);
}
pub struct SolverBodies<'a> { /* TypedDenseView over BodyVel, BodyPose, BodyInertia, BodyGate */ }
impl BodyGate { pub fn is_asleep(&self) -> bool; }
impl BodyPose { pub fn position(&self) -> Vec3; pub fn rotation(&self) -> Quat; }
```

---

## 10. Algorithms for critical paths

### 10.1 Gang protocol (K5b), with the nesting rule

**Run context (C1):**
- The pool's task trampoline receives `RunCtx`.
- `worker_main`'s top-level runs (`worker.rs:96-97`, `:115-116`, `:126-128`) pass `Top`.
- Every join route (`join_on_worker`, `join_external_helping`, `join_external`, which run tasks through `run_task` imported at `scope.rs:53`) passes `Nested`.
- Ordinary spawned closures ignore it.
- Cost: one register argument, no TLS. Fallback, if the trampoline change is rejected: a depth counter in `LaneDeposit`'s `_pad: u32` (`tls.rs:141`), costing two TLS writes per join, unmeasured.

**Lane task entry:**
- `Nested` → return immediately (declined).
- `Top` → run the program as a **resident** lane.

**Claim.** For phase p a lane loads `claim`:
- `phase == p && idx < n` → CAS to `idx + 1`, run chunk `idx`.
- `phase == p − 1` → CAS to `(p, 1)`, record `phase_n = n`, run chunk 0.
- Otherwise stop claiming.
- `n == 0`: the opener CASes to `(p, 0)` and performs the completer steps itself.

**Complete.** `done[p&1].fetch_add(1, AcqRel)`. The lane that reaches n is the last completer:
1. `done[(p+1)&1].store(0, Relaxed)`;
2. `completed.store(p, SeqCst)`;
3. `bits = parked.swap(0, SeqCst)`;
4. unpark each set bit's `threads[i]`.

**Wait (W7), resident lanes and lane 0 only:**
1. Spin with `pause` on `completed.load(Acquire) >= p` for the budget.
2. Write `threads[lane]` if not yet written.
3. `parked.fetch_or(bit, SeqCst)`.
4. `if completed.load(SeqCst) >= p { parked.fetch_and(!bit, Relaxed); continue }`.
5. `thread::park()`. Loop to 1 on return.

All four handshake operations are SeqCst, so they sit in one total order S. Either the waiter's `fetch_or` precedes the completer's `swap` (the completer sees the bit and unparks; park's token covers an unpark that arrives before the park), or the completer's `store` precedes the waiter's `load` (the waiter sees p). No lost wake.
- Cost: one SeqCst store and one swap per phase, plus one RMW per park.
- The count gate at `scope.rs:66-68` ("the count gate makes that joiner's check-then-park race-free against its own last completer") is the in-tree precedent.
- The timeout backstop is not used.

**Late lane.** `completed == FINAL` → return. Otherwise skip phases with `q < claim.phase`: `claim.phase = q+1` is opened only after `completed ≥ q`, so those phases are complete.

**SPMD contract.** Every value that decides control flow (each phase's n, loop counts) comes from data no phase writes after its first read, or from a completed `single`. A debug assert checks n against `phase_n`.

**Lanes.** `min(W, max_useful_lanes)`. W−1 lane tasks are spawned in one pool scope on lane 0's deque; lane 0 is the caller. Correctness needs no other lane, because lane 0 can complete every phase. W=1 spawns nothing.

**Deadlock freedom (C1):**
- Only two kinds of frame ever block in a gang wait:
  - (i) lane 0. It waits for chunks claimed by resident lanes.
  - (ii) resident lanes. Below the lane frame there is only `worker_main`, because it was run with `Top`.
- A chunk finishes using only frames above it on its own thread. A nested scope joins its own tasks (`scope.rs:28-30` "Nested scopes cannot deadlock: a joiner either runs a ready task or parks with a wake guaranteed by its own scope's last completer").
- A waiting lane runs no task, so it never stacks work above its own wait.
- The critic's trace (a nested scope's Drop pops an outer lane task) now declines in O(1).
- A B1 joiner of an unrelated system that steals a lane task declines it, so it does not stay until FINAL.
- A lane task stranded in a busy sibling's deque is reclaimed by lane 0's final scope join through sibling steal, and declined.

**Panic.** The panicking lane sets `poisoned` and counts its chunk done. Other lanes skip the remaining phases, and the scope re-raises (`scope.rs:1410-1411`).

**Cost per gang:** one scope (two heap acquisitions until the allocator plan's scope arena). Per phase: one CAS and one `fetch_add` per chunk, one SeqCst store, one swap. Spin and park are `#[cold]`.

### 10.2 `physics_prepare` (S1)

Two `par_iter` loops in one system. Per row:
- `slot` comes from `GroupSlot`.
- `BodyPose ← RigidBody` pose, plus `bound_radius = body_bounding_radius(shape)`. This is the same function as `systems.rs:302`, so the pair set is bit-identical.
- `BodyVel ← {inv_mass, R·I_local·Rᵀ, v, ω}`, the same expression as today's gather.
- `BodyGate.flags ←` SIMULATED | DYNAMIC | KINEMATIC | AWAKE from `IsEnabled` and `inv_mass`; the sleep bits are kept.
- If `!SEEN`: set SEEN, set `fresh_step = step`, refresh `BodyMaterial` and `BodyInertia`.
- Else, if `Collider` or `RigidBodyMass` changed: refresh those two.

The statics loop: pose from `GlobalTransform` when it changed or the row is fresh; `inv_mass = 0`.

**Batching.** The kernel default gives ≤ 2 chunks at 1241 rows (`par_iter.rs:73`). S1/S6 pass an explicit `BatchingStrategy`, measured in U5.

**`PhysicsStep`.** `dt` and the broadphase selection (the former `select_broadphase`) are written here once, and `step` is incremented.

### 10.3 Broadphase (S2)

**AllPairs:**
- `par_range` over `i`-blocks balanced by triangle area.
- Each chunk counts, prefix-sums, then writes its own run. Concatenating in chunk order reproduces today's `(min,max)`-sorted sequence (`systems.rs:327-329`).
- The outer loop skips dead `i` with `if r_i.is_nan()`. Inner dead `j` fail the predicate (§10.8).

**Grid:** a gang over build_csr / emit. Not verified: that `emit_passes` maps 1:1 onto phases.

### 10.4 Narrowphase (S3)

**Phase 1:**
- Pair k reads two slots.
- Its axis read is skipped if either body's `fresh_step == step`.
- It runs the shape dispatch (`systems.rs:391-426` unchanged) and writes `manifold_out[k]`, `valid[k]`, `axis_out[k]`.

**Phase 2:** prefix compaction in pair order into `Manifolds` / `sensor_overlaps`.

### 10.5 Solve program (S5)

One gang per step:
- `single`: build_columns plus per-colour cut tables. Today's policy stays: `MIN_SLOTS_PER_CHUNK = 64`, 6×W.
  - The warm seed is skipped for fresh bodies (D15).
- For each of the 4 substeps (`resources.rs:431` `substeps: 4`), as phases:
  1. `for_each(slot chunks)`: gravity;
  2. per colour: warm_apply;
  3. per colour: solve with bias;
  4. `for_each(slot chunks)`: position_integrate + refresh_inertia;
  5. for each of R relax sweeps, per colour: solve without bias.
- `single`: restitution, `store_and_swap` (canonical order, `colored.rs:3038-3046`), sleep `end_step` if enabled.

**Phase count:** 4 × (2 + (2+R)·C) = 104 at C≈6, R=2. At < 1 µs per phase that is ≈ 0.1 ms per step (arith.).

**Warm apply per colour.** Colours are body-disjoint for dynamic rows, and the writes are guarded (`colored.rs:1747-1753` "LOAD-BEARING THE MOMENT THIS RUNS IN PARALLEL"). Each body therefore gets the same ordered updates: bit-identical, gated by `{1,N}`.

**Narrow colours** are 1-chunk phases. That subsumes `colored.rs:2707` without changing bits (`colored.rs:2699-2701` "inline == 1-worker == N-worker").

### 10.6 Writeback (S6)

`par_iter_mut`. For each row whose `BodyGate.flags` has SIMULATED | AWAKE and `inv_mass ≠ 0`: `*body = RigidBody { … }` from `BodyPose` / `BodyVel`. This is the `colored.rs:3057` rule, so `Changed<RigidBody>` stays precise.

Rows without a group membership are filtered out by `GroupSlot`. That covers Collider removed, and Collider added mid-chain (DEAD ⇒ flags 0 ⇒ no write).

Finally, `commands.release_dense_group::<PhysicsBody>()`.

### 10.7 Events (S7)

- Merge-walk this step's sorted valid pair keys against `ContactEventState`'s previous keys and stored `Entity` handles.
- Start events need both `s2e` ≠ TOMBSTONE; a dying participant emits no start.
- End events use the stored handles.
- Then swap. Deterministic.

### 10.8 Dead-slot inertness, per loop (C2)

"Dead" = dying (bytes intact, simulated to the end of its step by design) or free (DEAD bytes).

| Loop | Touches free slots? | Guard | Result |
|---|---|---|---|
| Broadphase AllPairs | yes | `bound = r_i + r_j` = NaN, and `delta.length_squared() <= bound * bound` is false for NaN under an ordered compare | no pair. A future SIMD broadphase must use `_CMP_LE_OQ` and must not route NaN through `min`/`max` (memory: NaN inverts under NMin/NMax); test-pinned |
| Broadphase grid | yes | count pass skips `!(r >= 0.0)` | never binned (a saturating NaN→int cast would otherwise put it in cell 0) |
| Narrowphase / `narrowphase_sdf` | no / yes | pairs only name live slots; the SDF per-body sweep is gated on `BodyGate.flags & SIMULATED` | none |
| Graph build | yes (per-slot arrays) | `inv_mass == 0` ⇒ static singleton, like any static row today | none |
| Gravity / integrate / refresh | yes | `snap.simulated && is_dynamic_row(eff.inv_mass)` (`simd.rs:197`, `:245`); AVX2 blends only `inv_mass != 0` lanes (`simd.rs:259-260`); port keeps the gate, now from `BodyGate` | byte-untouched |
| Solve / warm apply | no | contacts only name live slots | none |
| Writeback | only rows with membership | flags 0 | no write |
| Events | no | TOMBSTONE check for dying | no start event |
| Sleep | yes | DYNAMIC gate | none |

**Dying slots** are real bodies until release: the chain simulated them from S1, so their bytes are consistent. They do not produce NaN.

---

## 11. Multithreading model

- **Model:** multi-reader/multi-writer, with ownership partitioned per phase. No `Mutex`/`RwLock`.
- **Shared read-only during a system:** slot maps, cut tables, graph CSR.
- **Structural changes** happen only in apply windows (X-11). By D14, group slots do not change owner or bytes between S1 dispatch and S6 completion, so view bases, lengths and slot ownership are stable for the whole chain, not just per system (W3).
- **Partitioning:**
  - slot ranges (slot phases);
  - body-disjoint colours;
  - pair indices;
  - rows of S1/S6 (`e2s` is injective, so slot writes are disjoint).
- **Atomics:** §10.1. The data chain is: all phase-p chunk writes → `done` `fetch_add` (AcqRel, one release sequence) → last completer → `completed.store(SeqCst)`, which also orders as release → waiter `load(Acquire/SeqCst)`. Phase p+1 reads happen-after phase p writes.
- **False sharing:** hot atomics are on separate lines. Chunk-boundary lines of 32 B columns are shared by two lanes on different bytes: no race, negligible contention.
- **`Send`/`Sync`:** views are `Copy + Send + Sync` (`dense/views.rs:144-149`). `GangLane` is `!Send`. `GangShared` is `Sync`; its `threads` cells are written before the SeqCst publish and read after the swap.
- **Race-freedom:** each element's writer in a phase is unique, and cross-phase accesses are ordered by the barrier chain. Dead slots are written only by kernel release (apply window, exclusive).
- **Deadlock-freedom:** §10.1.

---

## 12. Migration rungs, gates, and the one timing (MUST 5)

### Gates on every rung

**G-form** — `tests/physics_entity_model_census.rs`:
- the physics `Vec`/`Box`/`HashMap` field scanner, with a list that only shrinks;
- a `pool.scope(`/`try_with_active_pool(` list that only shrinks (4 → 0);
- registry assertions: each group column is Dense, in `PhysicsBody`, `TICKS_TRACKED == false`, with a `DEAD` value;
- removed ids are unregistered.

**G-alloc** — a counting `#[global_allocator]` in its own test crate (the uncommitted `joltab` bench edits are untouched):
- pyramid at 256 bodies, W=4, steps 10..20;
- per-step acquisitions ≤ the rung's recorded budget, and the data path == 0;
- **anti-vacuity:** `live_count(PhysicsBody) == spawned` and `manifolds > 0` at step 10.

**G-jolt:**
- `benches/jolt_parity_pyramid.rs`, W ∈ {1,2,4,8,16}, interleaved A/B against the rung base;
- configuration printed in the receipt;
- the same anti-vacuity assertion, run once before timing;
- a loop-level microbench for the touched loop.

### The one per-stage timing (R0, owner leave Q4)

**What:** per-stage wall time at W=1 and W=8, same binary, one window, interleaved.

**Stages:**
- gather, select, broadphase, narrowphase, build_graph;
- the solve split into:
  - build_columns;
  - the serial per-substep kernels (gravity, warm apply, integrate, refresh), 16 per step;
  - dispatched colour sweeps;
  - inline colour sweeps;
  - restitution + store + write_back;
- apply;
- R = T − Σ systems (round overhead).

**Instrument:** `SystemSpan` (`schedule.rs:1359-1361`) plus `register_scope` sub-zones (`profiling/ecs_control.rs:176`). Not verified: arming from criterion. Fallback: an uncommitted `Instant` fork.

**Decision rule (W8).** Each saving is credited only to the rung whose content removes it:
- **P1** (K5a; parallel broadphase + narrowphase): S_P1 = (bp₈ + np₈)·(1 − 1/8).
- **P2** (K5b gang solve): S_P2 = (serial_solve_kernels₈ + inline_colours₈)·(1 − 1/8) + (dispatched₈ − dispatched₁/8).
- **U5** (K4; parallel prepare/writeback, `BodyEffective` rebuild deleted): S_U5 = (gather₈ + apply₈)·(1 − 1/8) + build_bodies₈.
- **RR** (rounds 8 → 6, which lands with U5's graph): S_RR ≈ R₈ · 2/8.

**Order:**
- The first perf rung is the argmax of S over rungs whose prerequisites are met: P1 needs K5b (landed within P1); P2 needs K5b; U5 needs U1–U4.
- If S_U5 is the largest, the U track is scheduled first and P rungs interleave after U4.
- P1 and P2 do not depend on slot addressing: the gang runs on today's row-keyed scratch too. Only U5 does.

### The D2 spike (SP-1, owed for Q5)

A microbench, not a stage timing:
- coloured solve kernel over the pyramid contact set with (a) `BodyVel` 1-line records vs (b) velocity in the 52 B `RigidBody` column plus a separate inv-mass/inertia column;
- plus prepare + writeback at W=1/8 and 1241/10k;
- plus a gameplay `Query<&RigidBody>` table-vs-dense-resolve microbench.

**Rule:** if (b)'s solve loss per step is below (a)'s prepare + writeback per step at W=8, recommend (b). Within noise, it is the owner's values call.

### Rungs

| Rung | Content | Extra gate |
|---|---|---|
| R0 | Restore `physics_vec_side_store_census.rs` (34); take the timing; run SP-1 | census red if count ≠ 34 |
| U1 | K1 cohorts; migrate `scratch_ids.rs` and render lanes | staggers pairwise distinct mod 64; G-jolt; render microbench recorded |
| U2 | K2 + K3 + K6 (kernel only): groups, DEAD, deferred release, `GroupSlot`, anchor | trybuild: `Changed<T>`, `Added<T>`, `Ref<T>`, `Mut<T>` on a `ticks="none"` type fail to compile; removing a group member fails to compile. Miri Tree Borrows on views and `range_mut`. Proptest on slot-state invariants (live/dying/free) under random spawn/despawn/remove-anchor/re-add/release. Release-build test: insert + `check_ticks` + serde load on an untracked store do not fault |
| U3 | K4 | proptest: mixed dense `par_iter` ≡ `iter` multiset; `IsEnabled` + `GroupSlot` in one query |
| U4 | `PhysicsBody` group anchored on `Collider`; S1 fills it (the gather still drives the solve) | shadow equivalence against gathered rows (raw `create_entity` path, as in the bench). **F-1 red-first:** a `Trigger` overlap is reported. `Collider` removal and re-add. Anti-vacuity |
| U5a | Port `apply_gravity`, `position_integrate`, `refresh_inertia` (scalar oracles and AVX2) from `(&[BodyEffective], &[BodyState])` (`simd.rs:120-122`, `:169`, `:213-215`) to the SoA group views plus the `BodyGate` lane gate. Arithmetic op sequence unchanged | scalar ↔ AVX2 bit-oracle on a fixed corpus plus dead/static/non-simulated lanes; pre/post-port per-kernel bit-identity; FMA `do-not-fuse` asm check re-run |
| U5b | Re-type the `RigidSolver` seam (`solver/mod.rs:63-67` `fn solve(` … `scratch: &mut SolverScratch,`) to `SolverBodies`; port `SoftStepSolver` (ids 509/470/469/474/473, `vn_initial` 477); port `physics_narrowphase_sdf` and the coupled soft step's body reads; re-key `SoftRigidReaction` by slot | the existing reference-solver tests (23 files) green; per-scene bit-identity of the reference solver pre/post (addressing-only change) |
| U5 | Switch S2–S6 to slots; delete id 511, `touched`, the `BodyEffective` rebuild; parallel S1/S6; register `physics_integrate` only in Foundation mode; S6 after S7 | `{1,N}`; tolerance gate vs pre-rung (A5); pyramid bit-identity (one archetype, spawn order = slot order: critic-confirmed via `jolt_parity_pyramid.rs:113`); assert `touched`-set ≡ gate-filter set before deletion; **mid-chain despawn test** (despawn in S2's window ⇒ no NaN, no wrong-entity event); **dead-slot-at-origin test** (despawn a body at (0,0,0) resting on the floor ⇒ pair and manifold counts equal the survivors' only) |
| U6 | `IslandSleep` → `BodyGate` + island scratch; AWAKE in the kernel lane gate; delete the frozen snapshot (id 468); Vec 34 → 30 | **F-3 red-first** (despawn mid-archetype while islands sleep); frozen-body byte-identity under sleep; census 30 |
| U7 | `PairCache` merge + `fresh_step` | reuse test: despawn X, spawn Y into X's slot between two steps and within one apply window ⇒ no warm impulse or axis inherited |
| P1 | K5a + parallel S2 (AllPairs) + S3 phases | manifold multiset and order identical at W ∈ {1,8,16}; G-jolt |
| P2 | K5b gang; S5 on one gang; soft solve on a gang | `{1,N}`; nested-gang stress (§17); G-alloc ≤ 16 per step at W=8; `pool.scope` census 0 |
| E1 | Events (after Q1) | event stream identical at W=1 and W=8 |
| S0 | SoftBody (after Q2; sibling plan) | census 30 → 0 |

---

## 13. Respecting what exists (MUST 6)

**ADVANCED-PHYSICS-DESIGN-SPACE (main checkout):**
- Joints as designed (`:115` "`JointState` is **dense** because it is durable"). Body refs become slots, and the row map (`:166-167`) is superseded by D1.
- SoftBody: S0/D-8 (`:807`) owns that rung; Q2 frames it.
- Character and destruction are unchanged.
- ADVANCED's stale pre-KE16 premises (`:37-39`, fact 2) are flagged, not edited.

**Dense plan:**
- The dense kind stays the home of solver state.
- `DENSE-COMPONENTS-PLAN.md:26` "RigidBody*/velocity→dense" is the Q5 option (b), no longer silently superseded.
- `:64`'s "AVX re-gather … is the DEFAULT" becomes redundant.

**ARCH-AUDIT:**
- `:36` "C2 keep the gather": kept in substance, re-keyed by slot.
- `:30`'s sleep target is revised through Q3.

**Owner rulings of 2026-09-09:**
- barrier after KE16 (`OPEN-QUESTIONS.md:5212`): KE16 is closed on this tree;
- `Cuts` tuning deferred (`:5219-5222`): today's policy is kept;
- dense `par_iter` after Stage 4 (`:5223-5225`): Stage 4 is closed, so K4 is due.

**FMA:** no op-order change. U5a is a port of addressing and the gate source only, and is bit-oracle gated.

**GPU plan:** rigid stays CPU (`ADVANCED…:1069`); dense stays CPU.

**Allocator plan:** owns the scope boxes and chunks. This design cuts their count (≈ 331 → ≤ 16).

**Sub-granular packing plan:** `POOL-SUBGRANULAR-PACKING-PLAN.md:48` owns the resident floor. K1 reuses the `[pad | data | ticks]` layout (`constants.rs:218-229`) unchanged.

**Owner order (1):** marks `RENDER-PHYSICS-GPU-PLAN.md:218-220` and `PERF-DIRECTIONS.md:254`'s Rapier/Jolt-FFI allowances as superseded (orchestrator action).

---

## 14. Owner questions (values/scope) vs orchestrator decisions (MUST 7)

**Owner:**
- **Q1 Gameplay-visible contacts:** (a) events only [recommended]; (b) events plus a `Touching` relationship; (c) produce the existing `Contact` component.
- **Q2 SoftBody particle storage:** (a) K7 segmented dense column [recommended; per the owner's ladder, `feedback-no-allocator-but-ours.md:32-34`]; (b) ADVANCED D-8 resource row bank plus dense handle.
- **Q3 Sleep's gameplay face:** (a) `BodyGate` plus transition events [recommended]; (b) the recorded `Sleeping` EnableTag, which needs an in-place kernel toggle and a second home or per-step random lookups.
- **Q4:** permission to run the R0 timing, SP-1, and the per-rung G-jolt on the workstation.
- **Q5 (new) Where pose and velocity live** (W1):
  - (a) `RigidBody` stays table, with a derived group (D2), and the goal reads "one authoritative home plus one named derived form, with an authority window";
  - (b) the recorded design (`project-feature-tracks.md:25`, `DENSE…:26`): `RigidBody`'s pose and velocity *are* group columns, no copy, one form.
  - Decided on SP-1's numbers. The architect recommends (a) unless SP-1 shows (b)'s 2-line solve costs less than prepare + writeback at W=8. The value side (one form vs two) is the owner's.

**Orchestrator, with numbers:**
- first perf rung (the §12 rule);
- gang spin budget;
- `BatchingStrategy` for S1/S6;
- broadphase default `Auto` (the pair set is bit-identical between arms, `systems.rs:275-276`);
- fusing S2 + S3 into one gang;
- persistent incremental colouring;
- the RunCtx trampoline vs the TLS-depth fallback;
- retiring the Rapier/Jolt-FFI allowances (owner order (1)).

---

## 15. What this design does NOT change (MUST 8)

- **Solver math:** TGS-Soft, substep/relax counts, first-fit colouring, restitution, sequential-impulse arithmetic, the 31-column `ContactColumns` SoA.
- **Kernel arithmetic:** the integrate/gravity/refresh **op sequences** are unchanged. Their **signatures, lane gathers and gate source** are ported (U5a, gated). Rev 1 wrongly said the kernels were unchanged in full.
- **FMA contract and determinism class.** Colouring values may change where slot order ≠ row order.
- **Authored API:** `RigidBody`, `RigidBodyMass` (including the dead `inv_inertia`), `Collider` fields, the tags, the bundles. `Collider` only gains `anchor_group`. The `RigidSolver` seam signature changes (U5b); that is an internal seam.
- **Scene side, config defaults** (`simd_solve`/`parallel_solve` per the owner's half-ruling; `sleeping: false`), joints, destruction, character, the GPU plan, the asset queues.
- **Kernel bookkeeping `Vec`s** (dense `free`, `EntitySlotMap`, `LiveBitmap`).

---

## 16. Integration

**boyko_ecs:**
- `component_registry`: cohort ids, group ids, `TICKS_TRACKED`;
- `component/dense/*`: groups, DEAD, dying/release, typed views, untracked routing (§8 K2 table);
- `ecs_master/entity_api.rs`: the anchor step at `:305-312` and the remove/despawn sites;
- `iters/query/*`: `GroupSlot`, the K2 const-asserts in `Ref`/`Mut`/`filter.rs`, the par drivers;
- `change_detection/check_ticks.rs`;
- `system/params`: `DenseColumn*`, `DenseSlots`, `Commands::release_dense_group`;
- a new `par` facade.

**boyko_threadpool:**
- a `gang` module;
- the `RunCtx` trampoline argument across `worker.rs` / `scope.rs` run sites;
- a design doc in `docs/threadpool/` (none exists).

**boyko_macros:** `group`, `ticks = "none"`, `anchor_group`; `DEAD` via a `GroupColumn` impl.

**boyko_physics:**
- `systems.rs`, `plugin.rs` (§6 graph, Foundation-only integrate);
- `resources.rs` (delete id 511 / `touched`; `IslandSleep` → scratch);
- `solver/colored.rs` (gang program);
- `solver/simd.rs` (U5a);
- `solver/mod.rs`, `soft_step.rs` (U5b);
- `narrowphase/axis_cache.rs` + `solver/warm_start.rs` → `PairCache`;
- the SDF narrowphase and coupling ports;
- `scratch_ids.rs` deleted after U1.

**boyko_render:** `mesh_draw.rs`, `particle_system.rs`, `particle.rs`, `light_system.rs` → cohorts.

## 17. Validation

**Unit / integration:**
- gang: exactly-once, late-lane fast-forward, `n=0`, W=1, panic poisoning;
- **nested gang/scope stress:** at W ∈ {2,4,8}, a phase body calls `par_iter`, `for_range` and a nested `par_phases`, with a watchdog (fails on > 1 s). A B1-joiner steals a lane task and must decline (a counter asserts ≥ 1 decline in a forced scenario);
- F-1, F-3, FRESH reuse (U7), mid-chain despawn, dead-slot-at-origin, Collider remove/re-add;
- `{1,N}` at W ∈ {1,2,4,8,16};
- event determinism;
- G-form, G-alloc with anti-vacuity.

**trybuild:** change detection on `ticks="none"`; individual group-member removal.

**Loom:**
- the claim/complete/park protocol with 2–3 lanes × 3 phases, including a straggler racing a phase open and the SeqCst park handshake (the W7 case);
- an abstract model of the decline rule: a 2-thread deque where a nested join pops a lane task.

**Proptest:**
- group slot-state machine (live/dying/free, co-slot, DEAD bytes on free, `s2e`/`e2s` round-trip);
- mixed-dense `par_iter` ≡ `iter`;
- chunked AllPairs ≡ serial.

**Miri (Tree Borrows):** group views, `range_mut`, a gang with 2 lanes.

**Benchmarks:**
- G-jolt, the per-stage harness, SP-1;
- gang empty-phase latency (104 phases);
- broadphase AoS vs SoA;
- prepare/writeback at 1241 / 10k.

**`debug_assert!`:**
- SPMD n vs `phase_n`;
- every group column's `len` == group high-water;
- free-slot bytes == DEAD in debug sweeps;
- slot < 2^24 at key pack;
- writeback-visited count == live count minus the dying rows the chain began with;
- `size_of` const asserts;
- no `PairCache` hit with a fresh body (the D15 belt);
- the release command is issued exactly once per step.

## 18. Open questions (not decisions)

1. The pyramid's colour/slot histogram is unknown. It sets C and the useful-lane cap.
2. Not verified: whether `IsEnabled` and dense data terms compose in one query (U3 covers it).
3. Not verified: whether render's nine same-id lanes are swept at the same index (X-4).
4. Not verified: whether registry `LAYOUTS` consumers need scratch columns registered, and whether serde enumerates every dense type (§8 K2 serde row).
5. Not verified: `ColliderShape` = 16 B and `BodyEffective` = 64 B. `BodyEffective` is not `repr(C)` (`contact.rs:80`).
6. Not verified: whether today's AVX2 gravity/integrate lanes consult `simulated` or only `inv_mass`. Either way, U5a's oracle is the scalar kernel (`simd.rs:196-200`, `:244-249`).
7. Not verified: that `thread::current()` is allocation-free on std-spawned workers. The fallback is the pool's own per-worker handles.

## Checklist

- Structure, data, API, threading, correctness, integration, validation: addressed.
- N/A: tick wrap (all new columns are untracked; `fresh_step` wrap is handled in D15).
- Edge cases:
  - empty world;
  - W=1;
  - slot ≥ 2^24;
  - reuse within one window and across one step boundary (D15);
  - despawn or Collider removal mid-chain (D14);
  - Collider without `RigidBody` (D12);
  - a dead slot at the origin (§10.8);
  - a nested parallel call in a phase (§10.1);
  - a panicking phase;
  - no physics schedule running (D14 trade-off).

---

## Changelog: rev 1 → rev 2

| Finding | Action | Where in rev 2 | Removed rev-1 text (verbatim) |
|---|---|---|---|
| C1 gang/par_range nesting deadlock | FIX: run-context rule. Lane tasks run from any join decline; only top-level runs are resident. Deadlock-freedom argument; nested stress and loom tests | D7, §10.1, §11, §17, X-12 | "**Lanes.** `min(W, max_useful_lanes)`. W−1 lane tasks are spawned on the caller's deque; the caller is lane 0. Correctness does not depend on how many lanes start, because the caller alone can complete every phase." (kept, and extended with the nesting rule) |
| C2 dead slots not inert | FIX: `DEAD` values per column (NaN pose/radius), a per-loop inertness table, a grid skip, and a dead-slot-at-origin test. Byte count restated (sentinel inside the 16 B) | D14, §5, §7, §10.8, U5 gate | "Sweeps cover the high-water mark including tombstones (bounded by LIFO reuse), so dead slots must be inert (zero-fill, K3)." / "POD, zero-filled on free (K3) ⇒ dead slots inert." |
| C3 K2 unsound in release | FIX: `TICKS_TRACKED` const; compile-time rejection in `Ref`/`Mut`/`Added`/`Changed`; every runtime tick consumer routed; trybuild and a release fault test | D10, §8 K2 table, U2 | "the pool reserves tick sub-regions but never commits them (the form `ScratchColumn` already uses). Tick accessors debug-assert tracked." |
| W1 D2 overturns an owner resolution | FIX: Q5 with the SP-1 spike and rule; goal restated as authoritative + derived; authority window added | §1, D2, §4.1, §12 SP-1, §14 Q5 | "Every physics datum lives in exactly one ECS form, owned by the entity it describes wherever a per-entity form fits." / "Net saving: none measurable." |
| W2 FRESH/SEEN self-defeating | FIX: `fresh_step == step` lookup rule, with a proof valid for any release timing | D15, §10.2, §10.4, §10.5, U7 | "`BodyMaterial.flags.SEEN` is 0 after zero-fill on insert. `PairCache` lookups skip any key with an unseen body, and prepare sets `SEEN`." |
| W3 slot identity across the chain | FIX: deferred release (dying state) plus a release command from S6, with S6 after S7; chain invariant stated; mid-chain despawn test | §2 invariants, D14, §6, §11, U5 gate | "Structural changes happen only in apply windows (SCH7, `schedule.rs:1331-1332`), so every view's base and length are stable for the whole system." |
| W4 Collider removal unspecified | FIX: kernel anchor (D13) at attach/detach/despawn sites; remove/re-add test; S6 filters by `GroupSlot` | D13, §6 S6, U4 | "Removing the group on `Collider` removal zero-fills the slot and pushes it on the free list." |
| W5 raw spawn path skips `#[require]` | FIX: the anchor lives in the structural core that `create_entity` runs (X-10); anti-vacuity in G-jolt/G-alloc | D13, X-6 superseded, §12 gates | "`Collider` (shape, layer, mask) \| table component, gains `#[require(PhysicsBody)]`" |
| W6 S1 intra-system write conflict | FIX: queries read `GroupSlot` (slot-map node); writes go through one `DenseColumnMut` per column; two par loops in one system; round target re-costed (still 6) | §6 S1 + access check, §10.2 | "`Query<(&RigidBody, …, PhysicsBodyMut)>` + a statics query `(Ref<GlobalTransform>, Ref<Collider>, PhysicsBodyMut), Without<RigidBody>`" |
| W7 park/wake ordering | FIX: SeqCst handshake on `completed`/`parked`, park token, no timeout reliance; loom case | §10.1 Wait, §5, §17 | "**Wait.** A lane leaving the claim loop spins (`pause`, bounded budget) on `completed.load(Acquire) >= p`, then marks itself parked and parks (with a timeout backstop)." |
| W8 decision-rule attribution | FIX: savings defined per rung content (P1, P2, U5, RR); ordering by met prerequisites | §12 decision rule | "saving_P1 ≈ A₈·(1 − 1/8), where A = broadphase + narrowphase + inline colours + solve serial kernels + gather + apply (the stages that P1 parallelises)" |
| W9 unaccounted `BodyState` consumers | FIX: seam re-typed; `SoftStepSolver` ported (U5b); Foundation integrate kept and registered only in that mode; SDF and coupling ported; SIMD kernel port with a bit-oracle gate (U5a); §15 corrected | §6 variants, §9, §12 U5a/U5b, §15 | "**Solver math:** TGS-Soft, … and the 31-column `ContactColumns` SoA." (kernels-unchanged claim) and "`physics_integrate` (dead round, `plugin.rs:559`)" |
| Sleep word / gate (consequence of C2/W9) | D4's `BodySleep` becomes `BodyGate` (flags + sleep); SENSOR moves to `BodyMaterial` so S7 does not conflict with S5 | D4, §5 | "`#[repr(C)] pub struct BodySleep { below: u16, asleep: u8, _pad: u8 } // 4 B; sleep phases, sequential`" |
| Preserve list | Kept unchanged: D1, D3, parallel warm-apply with the movability guard, D5, K1 plus X-4, the §0 table (X-1, X-2), the CAS claim word and role-separated lines, the §12 single timing plus G-form plus red-first F-1/F-3, D11, the Q3 discipline (extended to Q5) | — | — |

**Sections whose invariants the changes depend on (for re-reading):**
- §10.1 depends on `scope.rs:20-30` and `worker.rs:96-128`.
- D13 depends on `entity_api.rs:241-319` and `:1051-1060`.
- D14 depends on `schedule.rs:669`.
- D10 depends on `filter.rs:969-974` and `dense_store.rs:257-259`, `:372-373`, `:418`, `:738-765`.
- §10.8 depends on `simd.rs:197` and `:259-260`.
- D15 depends on `colored.rs:3038-3046`.

**Primary files referenced:**
- `D:/wt/joltab/crates/boyko_threadpool/src/scope.rs`
- `D:/wt/joltab/crates/boyko_threadpool/src/worker.rs`
- `D:/wt/joltab/crates/boyko_threadpool/src/tls.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/component.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/iters/query/filter.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/iters/query/data/write.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/change_detection/check_ticks.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/hooks/mod.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/mod.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/joltab/crates/boyko_physics/src/components.rs`
- `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs`

---

# Critique log - pass 1

Critic: `architecture-critic`, one pass over rev 1. Findings and verdict are quoted verbatim. Each action is quoted verbatim from the rev 2 changelog table above (columns "Action" and "Where in rev 2"); the removed rev-1 text is in that table.

## Verdict

CHANGES REQUESTED. The plan is not ready to implement. It has 3 blocking findings: a deadlock in the gang/par_range protocol when a parallel call is nested inside a phase, a false claim that zero-filled dead slots are inert in the broadphase, and K2's untracked dense storage guarded only by debug asserts, which gives UB in release. It also has 9 important findings. I walked all 18 sections and checked every load-bearing claim against D:/wt/joltab @ ca582e72. These claims hold, with quoted lines: X-1 (systems.rs:299-300), X-2 (KE16-RESULTS.md:1728, :1730), X-3 (query.rs:357), X-4 (mesh_draw.rs:429-459), X-5 (enable_store.rs:8, :23), X-6 (boyko_macros/src/lib.rs:120-122), and the guarded warm-apply premise (colored.rs:1739-1765). Several design claims in the plan are true as written. The findings below are about what the plan asserts without checking, and about what it leaves out.

## Blocking findings

### C1

**Finding (verbatim):**

C1. Gang / par_range (K5a/K5b) deadlocks when any parallel call is nested inside a phase body. WHERE: §10.1 'W-1 lane tasks are spawned on the caller's deque; the caller is lane 0'; a waiting lane 'spins ... then parks'; §8 'par_range becomes a one-phase gang'; the listed consumers are render, animation and transform passes. CODE: crates/boyko_threadpool/src/scope.rs:20-21 `A joining WORKER of this pool reaches its own deque as the OWNER ... so it pops its own oldest chunk from the FIFO owner end`. The pool's deadlock-freedom argument (scope.rs:28-30 `Nested scopes cannot deadlock: a joiner either runs a ready task or parks with a wake guaranteed by its own scope's last completer`) assumes no task ever blocks except inside a stealing join. A gang lane that waits on a phase barrier breaks that assumption. par_iter itself opens a nested pool.scope from inside tasks (par_iter.rs:336-341 `par_iter may be invoked from inside a worker task body ... scope's Drop performs work-stealing so nested invocations cannot deadlock`). TRACE: lane 0 is the caller worker Wk. Its deque still holds unstolen gang lane tasks. A phase-p chunk body calls query.par_iter() or for_range(). The nested scope's Drop pops the OLDEST task on Wk's deque, which is an outer gang lane task. That lane enters the gang, finds phase p fully claimed, and waits for done[p]==n. That count needs the chunk sitting below it on the same stack, so it never completes. The park timeout only re-polls, so it never escapes. A worker that stole a batch of lane tasks hits the same path. CONSEQUENCE: any system that calls par_iter, for_range or par_phases from inside a par_phases or for_range body hangs its worker for good. This covers the plan's own advertised consumers (render multi-pass culling, animation hierarchy passes), and a par_iter inside a for_range body is natural code. The shipped rigid solve does not nest, so physics alone does not show the bug. A second, non-deadlock consequence: a B1 joiner of an unrelated system's scope can steal a lane task. That system's join then cannot return until the whole gang reaches FINAL, several ms on the pyramid. CONFIDENCE: CONFIRMED (the pool behaviour is traced in the code; the lane placement and the absence of any nesting rule are traced in the plan). WHAT IS NEEDED: K5b needs a nesting contract that is enforced, not just documented. Either gang lanes can never sit where a nested join pops or steals them, a lane started beneath a phase of its own gang detects this and returns, or opening a scope or gang inside a phase body is rejected at runtime, with a test. The loom/Miri plan in §17 must include a pool-level nested case, not only the bare protocol.

**Architect's action (rev 2 changelog, row "C1 gang/par_range nesting deadlock"):** FIX: run-context rule. Lane tasks run from any join decline; only top-level runs are resident. Deadlock-freedom argument; nested stress and loom tests

**Where in rev 2:** D7, §10.1, §11, §17, X-12

### C2

**Finding (verbatim):**

C2. 'Dead slots are inert by zero-fill' is false for the broadphase, the first loop that sweeps the high-water mark. WHERE: §3 D1 trade-off 'Sweeps cover the high-water mark including tombstones ... so dead slots must be inert (zero-fill, K3)'; §7 'reads 16 B (position + cached radius)'; §10.3 says nothing about liveness. CODE: the predicate is geometry only: crates/boyko_physics/src/systems.rs:302-304 `let bound = body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);` ... `if delta.length_squared() <= bound * bound {`. Today no filter is needed because every gathered row is live (systems.rs:244). The zero bit pattern of ColliderShape is a real shape: components.rs:125-129 `#[repr(C)] ... pub enum ColliderShape { Sphere {` has tag 0 = Sphere, so zero bytes read as Sphere{radius:0}. A tombstone is therefore a zero-radius static sphere at the world origin. The pyramid floor sits at `position: Vec3::new(0.0, -1.0, 0.0)` (jolt_parity_pyramid.rs:135) with half-extents (50,1,50), so the origin lies on its top face. CONSEQUENCE: after the first despawn that leaves a tombstone, AllPairs and the grid (every dead slot hashes to the origin cell) emit pairs between each dead slot and the floor, and with every body whose bounding sphere contains the origin. The narrowphase then runs sphere-box on radius 0, and any contact acts as a phantom immovable point collider at (0,0,0). S7 turns these contacts into events whose participant is `s2e[slot]` = TOMBSTONE (dense_store.rs:34 `pub(crate) const TOMBSTONE: EntityId = EntityId(usize::MAX);`). The pair and manifold counts also change, so G-jolt and any identity gate compare different work. CONFIDENCE: CONFIRMED. WHAT IS NEEDED: dead-slot inertness has to be established for each loop rather than asserted once. The broadphase and narrowphase need a liveness gate: a bit inside the 16-byte record, a sentinel radius that fails the predicate (NaN fails `<=`; note that -inf does not, because (-inf)^2 = +inf), or a LiveBitmap test. The §7 byte count must be restated to include it. Also add a test that despawns a body at the origin next to the floor.

**Architect's action (rev 2 changelog, row "C2 dead slots not inert"):** FIX: `DEAD` values per column (NaN pose/radius), a per-loop inertness table, a grid skip, and a dead-slot-at-origin test. Byte count restated (sentinel inside the 16 B)

**Where in rev 2:** D14, §5, §7, §10.8, U5 gate

### C3

**Finding (verbatim):**

C3. K2 ('untracked dense', ticks="none") is guarded only by debug asserts, so reachable safe code gets UB in release. WHERE: §8 K2 'the pool reserves tick sub-regions but never commits them ... Tick accessors debug-assert tracked'; D4/§9 make BodySleep and BodyPose readable by gameplay ('gameplay reads sleep through &BodySleep'). CODE: every tick accessor on an untracked pool is debug-only: component_pool.rs:1904-1915 `debug_assert!(self.is_tracked(), "...UNTRACKED pool...")` followed by `unsafe { *(*self.added_base.as_ptr().add(index)).get() }`, with the same pattern at :1809-1831 for the added/changed base pointers that `Changed<C>` and `Mut<T>::deref_mut` use. The dense kernel writes and walks ticks without going through anything K2 changes: DenseStore::insert stamps them (dense_store.rs:190-193 `a fresh dense component is Added this frame, so BOTH the slot's added and changed ticks are stamped`), and check_ticks sweeps them periodically (check_ticks.rs:24-25 `every LIVE slot of every [DenseStore] in the world's [DenseRegistry]`). CONSEQUENCE: in a release build with any ticks="none" dense component, the first insert or the first check_ticks pass dereferences reserved but uncommitted VA and faults. Even after those kernel sites are patched, gameplay code such as `Query<&BodySleep, Changed<BodySleep>>`, `Ref<BodyPose>` or a `Mut<BodyPose>` from a safe system reads or writes uncommitted memory in release. That is a soundness hole in a kernel API offered to every crate. CONFIDENCE: CONFIRMED. WHAT IS NEEDED: K2 must enumerate every kernel tick consumer of DenseStore (insert/remove stamping, check_ticks, Ref/Mut/Added/Changed fetch, serde if it reads ticks) and route each one around untracked stores. User-facing change detection on a ticks="none" type must be a compile-time rejection (a const on Component, the same as the existing STORAGE_IS_BITSET refusals), not a debug_assert. Add a trybuild fixture for it.

**Architect's action (rev 2 changelog, row "C3 K2 unsound in release"):** FIX: `TICKS_TRACKED` const; compile-time rejection in `Ref`/`Mut`/`Added`/`Changed`; every runtime tick consumer routed; trybuild and a release fault test

**Where in rev 2:** D10, §8 K2 table, U2

## Non-blocking findings

### W1

**Finding (verbatim):**

W1. D2 overturns a recorded owner resolution on arithmetic alone and does not raise it as an owner question. The owner's chosen dense design is recorded as `the solver strides ONE dense column directly -> the gather + the std::Vec BodyState/BodyEffective mirror are DELETED -> ... contiguity kept with NO copy (strictly better than the gather)` (memory project-feature-tracks.md:25, marked 'owner's idea, the chosen design'). D2 keeps a per-step copy both ways (prepare/writeback), backed only by 'Net saving: none measurable' (arith., never measured). It also contradicts the plan's own §1 goal ('every physics datum lives in exactly one ECS form'): velocity and pose live in RigidBody and in BodyVel/BodyPose, with a round trip every step. CONSEQUENCE: the lane builds the whole group on a premise the owner has recorded the opposite of, and gameplay sees two pose homes (three with Transform) with no stated authority during the chain. CONFIDENCE: CONFIRMED (quoted). NEEDED: either add an owner question with the measured cost of each option (an in-place RigidBody-as-dense-group spike against prepare/writeback, which lens plans R-1 records as never measured), or restate the goal as 'one authored form plus one derived solver form' and get that ratified.

**Architect's action (rev 2 changelog, row "W1 D2 overturns an owner resolution"):** FIX: Q5 with the SP-1 spike and rule; goal restated as authoritative + derived; authority window added

**Where in rev 2:** §1, D2, §4.1, §12 SP-1, §14 Q5

### W2

**Finding (verbatim):**

W2. The FRESH/SEEN scheme defeats itself through step ordering. §10.6: 'PairCache lookups skip any key with an unseen body, and prepare sets SEEN'. But prepare is S1, and it runs before the lookups in S3 (axis_read) and S5 (warm start). A slot that is freed and then reused in one apply window is zero-filled (SEEN=0), gets SEEN set by S1, and is then looked up as seen, so it inherits the dead body's warm impulse and reference axis. CONSEQUENCE: the plan's own U7 gate ('no stale warm impulse') is red by construction. A respawn-in-place pattern (projectile or prop pools under LIFO reuse, dense_store.rs:124-125) injects the previous tenant's accumulated impulse into a body with a different mass. CONFIDENCE: CONFIRMED (traced in plan text). NEEDED: a freshness signal that lasts past the step's last cache lookup (set after store_and_swap, or a per-step FRESH bit cleared at the end of the step), with the lookup rule stated against that bit.

**Architect's action (rev 2 changelog, row "W2 FRESH/SEEN self-defeating"):** FIX: `fresh_step == step` lookup rule, with a proof valid for any release timing

**Where in rev 2:** D15, §10.2, §10.4, §10.5, U7

### W3

**Finding (verbatim):**

W3. Slot identity must hold across the whole chain S1..S7, not only within one system, and the plan claims only the per-system version. §11: 'Structural changes happen only in apply windows (SCH7) ... stable for the whole system'. But the executor drains an apply window at the start of every dispatch round: schedule.rs:650 `// === Step 1: apply window drain` ... :678 `self.apply_window_drain(world_mut, completion);`. Physics shares 'the caller's fixed schedule' with gameplay. CONSEQUENCE: a gameplay system that despawns (or despawns then spawns) during the S2 round has its commands applied before S3 or S5. Pairs and manifolds then reference a slot that is zero-filled, and so solved as a static phantom (inv_mass 0), or reused by a new entity under LIFO. S7, running concurrently with S4-S6, then maps `s2e[slot]` to the NEW entity and emits a contact event naming an entity that never touched anything. Today's copy-based gather avoids the first effect; IM-1's debug_assert (systems.rs:1153-1155) at least flags the second in debug builds. CONFIDENCE: CONFIRMED that the drain runs every round; PLAUSIBLE for how often it fires in real schedules. NEEDED: a stated chain invariant with a mechanism behind it: physics-chain atomicity in the schedule, or group slot frees deferred to a physics-owned point, or a generation check at the s2e and event boundary.

**Architect's action (rev 2 changelog, row "W3 slot identity across the chain"):** FIX: deferred release (dying state) plus a release command from S6, with S6 after S7; chain invariant stated; mid-chain despawn test

**Where in rev 2:** §2 invariants, D14, §6, §11, U5 gate

### W4

**Finding (verbatim):**

W4. Removing Collider does not remove the group; the removal path is unspecified, and #[require] only acts on attach. §5 'Removing the group on Collider removal zero-fills the slot' names no mechanism. The required-components registry has only construction entries: required.rs:32-38 `Capture-free constructor for a required component ... writes one fully-initialized value` (no removal path; grepping 'remov' finds 0 hits in that file). CONSEQUENCE: `remove::<Collider>()` leaves a live group slot that S1 never refreshes, because both S1 queries need Collider. S2 keeps colliding against it at its last pose, and S6's query `(Mut<RigidBody>, PhysicsBodyRef)` has no Collider term, so if RigidBody remains the ghost's solver pose keeps being written back. CONFIDENCE: CONFIRMED. NEEDED: name the kernel mechanism (a Collider on_remove hook that removes the group, or group membership derived from Collider) and add a test for Collider-removal and re-add.

**Architect's action (rev 2 changelog, row "W4 Collider removal unspecified"):** FIX: kernel anchor (D13) at attach/detach/despawn sites; remove/re-add test; S6 filters by `GroupSlot`

**Where in rev 2:** D13, §6 S6, U4

### W5

**Finding (verbatim):**

W5. The raw spawn path does not expand #[require], so the Jolt bench and 22 physics test files would get no PhysicsBody group, and the design's gates would pass vacuously. The bench spawns through `world.create_entity(archetype, &[(RigidBody..),(RigidBodyMass..),(Collider..)])` (jolt_parity_pyramid.rs:113-123). EcsMaster::create_entity routes only the ids it is given: entity_api.rs:171-180 `partition the input into a TABLE subset ... and a DENSE subset`, with no required-component expansion. Grepping 'require' in entity_api.rs finds only a doc line at :677; expansion lives in commands (spawn_at_command.rs, spawn_batch_command.rs, migration_helpers.rs). Grep counts 26 `create_entity(` calls across 23 files in crates/boyko_physics. CONSEQUENCE: after U4/U5, G-jolt times a step with zero bodies and reads it as a large speed-up, and G-alloc measures an empty pipeline. This is the repository's catalogued failure mode, green from emptiness. CONFIDENCE: CONFIRMED. NEEDED: either require expansion on the raw path (a kernel change) or a fixture migration listed in U4. G-jolt and G-alloc also need an anti-vacuity assertion that the group's live count equals the spawned body count.

**Architect's action (rev 2 changelog, row "W5 raw spawn path skips `#[require]`"):** FIX: the anchor lives in the structural core that `create_entity` runs (X-10); anti-vacuity in G-jolt/G-alloc

**Where in rev 2:** D13, X-6 superseded, §12 gates

### W6

**Finding (verbatim):**

W6. S1 as declared fails the intra-system aliasing check. It holds two queries that both write `PhysicsBodyMut` (dynamic, plus the statics query with `Without<RigidBody>`). The detector is keyed by component id only, with no filter-based disjointness: filtered_access_set.rs:1-2 `accumulates per-bit ownership so sibling SystemParams reject conflicting access at registration time`, indexed `0..512 component reads (by ComponentId.0) | 512..1024 component writes` (:20-21). The kernel also documents a dense component as one conflict node: write.rs:280 `the conflict graph (Decision 6 — one dense node)`. CONSEQUENCE: registration panics with ComponentWriteVsWrite. Splitting S1 into two systems adds a round and breaks the '6 rounds' target; merging into one Option-heavy query adds per-row branches. CONFIDENCE: CONFIRMED. NEEDED: choose the S1 shape explicitly and re-cost the round target.

**Architect's action (rev 2 changelog, row "W6 S1 intra-system write conflict"):** FIX: queries read `GroupSlot` (slot-map node); writes go through one `DenseColumnMut` per column; two par loops in one system; round target re-costed (still 6)

**Where in rev 2:** §6 S1 + access check, §10.2

### W7

**Finding (verbatim):**

W7. The gang's park/wake handshake is a Dekker pattern whose memory ordering the plan does not specify. §10.1/§11 give only `completed.store(p, Release)` on the completer side and `load(Acquire)` on the waiter side. The completer does store completed, then read parked; the waiter does set parked, then re-check completed, then park. Under Release/Acquire each thread's store can pass its later load, so both sides can read stale values and the wake is lost. The only backstop is a timeout, and the pool measured its quantum at `~15.3 ms unguarded and ~1.0 ms while boyko_app's TimerResolutionGuard holds 1 ms` (scope.rs:59-61); the bench does not hold that guard. CONSEQUENCE: one lost wake on the final phase makes the gang's scope join wait out the timeout, stalling a step of about 12 ms at W8 by up to about 15 ms. The existing scope needed a dedicated 'count gate' to close the same race (scope.rs:66-69). CONFIDENCE: PLAUSIBLE (standard SC-reordering hazard; the plan is silent on it). NEEDED: specify the handshake ordering (SeqCst fences or an equivalent gate) and add it to the loom model.

**Architect's action (rev 2 changelog, row "W7 park/wake ordering"):** FIX: SeqCst handshake on `completed`/`parked`, park token, no timeout reliance; loom case

**Where in rev 2:** §10.1 Wait, §5, §17

### W8

**Finding (verbatim):**

W8. The §12 decision rule assigns savings to P1 that P1 does not deliver. 'A = broadphase + narrowphase + inline colours + solve serial kernels + gather + apply (the stages that P1 parallelises)'. But P1 contains only 'K5a + parallel S2 (AllPairs chunks) + S3 phases'. Inline colours and the per-substep serial kernels become parallel only under the gang in P2 (§10.5), and gather/apply only under K4 in U5. CONSEQUENCE: if the timing shows the 16 serial solve kernels per step dominate (lens logic §3), the rule credits P1 with that saving, schedules P1 first, and P1 then cannot move T(1)/T(8). This defeats the purpose of the single owed timing (MUST 5). CONFIDENCE: CONFIRMED (traced in plan text). NEEDED: define A and B from each rung's actual content, and state the order for when U5 lands relative to the P rungs.

**Architect's action (rev 2 changelog, row "W8 decision-rule attribution"):** FIX: savings defined per rung content (P1, P2, U5, RR); ordering by met prerequisites

**Where in rev 2:** §12 decision rule

### W9

**Finding (verbatim):**

W9. The entity model and system graph cover only the coloured rigid path, so U5 cannot delete id 511 as specified. Consumers of `SolverScratch.bodies` (BodyState) that the plan does not account for: (a) the public solver seam `pub trait RigidSolver ... fn solve(&mut self, config: &PhysicsConfig, manifolds: &[Manifold], scratch: &mut SolverScratch)` (solver/mod.rs:57-67) and the reference `SoftStepSolver` (soft_step.rs:826), which 23 test/bench files reference, together with its columns 509/470/469/474/473 and vn_initial 477; (b) Foundation integration, where physics_integrate is not a dead round (plugin.rs:528-530 `IntegrationMode::SolverOwned` / `IntegrationMode::Foundation`; :557-559); (c) physics_narrowphase_sdf and SdfField (plugin.rs:582-585); (d) the coupled soft step's reads of bodies and the grid. The default-on AVX2 kernels also take the AoS types directly: `pub fn position_integrate(bodies_eff: &[BodyEffective], snapshot: &mut [BodyState], ...)` (simd.rs:213-217), with `simd: true,` as the default (resources.rs:451). Their lane gathers and scalar bit-oracles must be rewritten for the SoA group, and those are FMA-sensitive kernels, yet §15 says kernels are unchanged. CONSEQUENCE: either a BodyState mirror survives for the seam, leaving two body models and contradicting the goal, or U5 silently expands into porting the reference oracle and the SIMD kernels, with no gate. CONFIDENCE: CONFIRMED. NEEDED: decide the fate of each variant (port, retire, or keep behind a named exception), and add the SIMD kernel port with its bit-oracle gate to a rung.

**Architect's action (rev 2 changelog, row "W9 unaccounted `BodyState` consumers"):** FIX: seam re-typed; `SoftStepSolver` ported (U5b); Foundation integrate kept and registered only in that mode; SDF and coupling ported; SIMD kernel port with a bit-oracle gate (U5a); §15 corrected

**Where in rev 2:** §6 variants, §9, §12 U5a/U5b, §15

## Other changelog rows (not tied to a single finding)

- **Sleep word / gate (consequence of C2/W9).**
  - Action: D4's `BodySleep` becomes `BodyGate` (flags + sleep); SENSOR moves to `BodyMaterial` so S7 does not conflict with S5
  - Where in rev 2: D4, §5
- **Preserve list.**
  - Action: Kept unchanged: D1, D3, parallel warm-apply with the movability guard, D5, K1 plus X-4, the §0 table (X-1, X-2), the CAS claim word and role-separated lines, the §12 single timing plus G-form plus red-first F-1/F-3, D11, the Q3 discipline (extended to Q5)
  - Where in rev 2: —

# Critic's preserve list

1. D1, stable dense slots as BodyIndex: grounded in dense_store.rs:6-8 `deletion is tombstone + free-list, never swap-remove, so **live slots never move**`. It closes F-3 and the row-shift warm-start misses, and removes IM-1's serial gather/apply. Keep it. On the pyramid every body, the floor included, is spawned into `bundle_archetype_id_for::<RigidBodyBundle>()` (jolt_parity_pyramid.rs:113), and Simulated is a bitset toggle (:125) that moves no row. So archetype-row order equals spawn order equals slot order, and U5's 'bit-identity on the pyramid' is expected, provided W5 is fixed.
2. D3, SoA group over one AoS dense record: it matches the in-tree measurement `resources.rs:176-178` (SoA kernel ~1.6x slower on AoS BodyState). BodyVel reproduces BodyEffective's one-line-per-random-body shape exactly, so the solve-side 'equal by identical address arithmetic' claim holds.
3. Parallel per-colour warm-apply (§10.5): it is race-free and bit-identical, because the existing kernel already guards writes on movability, and did so deliberately ahead of parallel dispatch: colored.rs:1747-1753 `LOAD-BEARING THE MOMENT THIS RUNS IN PARALLEL ... The guard is what makes each worker's writes disjoint`. Keep that guard.
4. D5, read-old/write-new PairCache: it correctly identifies the shared-table mutation in the pair loop that blocks a parallel narrowphase (systems.rs:415-422 `let last_axis = axis_cache.get(a, b);` ... `axis_cache.set(a, b, c.reference_axis);`).
5. K1 cohorts as a kernel feature, with X-4 as evidence that the cohort obligation is already broken outside physics: mesh_draw.rs:446-459 builds nine lanes on the single `u32_id`, and constants.rs:198-205 states the obligation 'the kernel cannot discharge'. This moves hand-placed physics ids (scratch_ids.rs:541) into a capability every crate uses.
6. The corrections table in §0: X-1 (the pyramid runs AllPairs, systems.rs:299-300) and X-2 (a nested scope from a worker is parallel, KE16-RESULTS.md:1728 `speed-up over seq | **14.6x**`) both check out and correct stale premises in the brief.
7. Gang claim word updated by CAS rather than fetch_add, so a straggler from phase p-1 cannot consume an index of phase p, and hot atomics placed on separate cache lines by role (claim, done, completed, parked).
8. §12's single owed timing with an explicit decision rule, the G-form census that only shrinks (restoring the 34-Vec pin missing from this tree), and the red-first F-1/F-3 tests. The mechanism is right; only the attribution needs fixing (W8).
9. D11, splitting PhysicsConfig into user config and per-step state: it removes the two per-step writers (systems.rs:217 `cfg.dt = fixed_time.delta_secs();` and broadphase_policy.rs:192) that make every reader conflict.
10. Raising Q3 (sleep's gameplay face) against the lane's recorded step-6 text, instead of silently overriding it. D2 should follow the same discipline (W1).

# Critique log - pass 2 (2026-09-11)

Pass 2 was scoped to the rev 1 → rev 2 delta. The critic's output is reproduced below verbatim: the verdict, every blocking finding, every non-blocking finding, and the fixes it confirmed. The architect's response is the Rev 3 patch that follows this section.

## Verdict

> CHANGES REQUESTED (pass 2, scoped to the delta). The C1 deadlock fix is correct. W1, W2, W6, W7 and W8 are resolved. C2, C3, W3, W4 and W5 are only partly fixed, and rev 2 introduces new defects. There are 5 blocking findings: (B1) K2 misses two tick consumers that safe code can reach, so C3's release-build UB is still there; (B2) a slot that is already dying when S1 runs is simulated as a live ghost for a whole step, and whether that happens depends on apply-window completion order; (B3) the rev-2 chain invariant ("lengths and slot ownership stable for the whole chain") is false, because a mid-chain insert appends a slot; (B4) the anchor has one attach site, but the tree has at least eight, and command spawns do not reach entity_api.rs:305-312 as X-10 claims; (B5) the clone path copies live group bytes (SEEN=1), which breaks the premise of the D15 proof and can double-insert the group. There are also 10 non-blocking items. Evidence is from the D:/wt/joltab working tree. I had no shell tool, so I could not run `git diff --stat ca582e72 d11962a9 -- crates`; that check is still owed. No cargo and no timing were run.

## Blocking findings

- B1. The C3 fix is incomplete: the K2 routing table ('one row per kernel tick consumer', section 8) misses two consumers that safe public API can reach. WHERE: section 8 K2 table, D10, U2 release-fault test. (a) The dense arm of `EcsMaster::get_component_mut<T>` reads the tick region directly and never goes through a Ref/Mut `init_state`, so the planned const-assert does not fire there: component_api.rs:657 `if component_registry::storage_kind(cid.0) == component_registry::StorageKind::Dense {`, :673 `let added: Tick = unsafe { *(*store.added_ticks_ptr().add(slot)).get() };`, :674-675 `store.changed_ticks_ptr().add(slot)`. The Mut it returns then writes that changed tick on DerefMut. (b) `DenseStore::compact` moves ticks: dense_store.rs:607 `pub fn compact(&mut self) {` and :640 `self.column.move_ticks(read, write);`. The only guard is component_pool.rs:1955-1958 `debug_assert!(self.is_tracked(), ...)`. Safe code reaches it through ecs_master.rs:599 `pub fn dense_registry_mut(&mut self)`, then dense_registry.rs:112 `pub fn store_mut(`, then compact, or through `build_view().compact()` (views.rs:93-94). CONSEQUENCE: in a release build, `world.get_component_mut::<BodyPose>(e)` (editor or debug tooling code) or a compact on a group column reads and writes reserved but uncommitted tick VA, which faults. That is exactly C3's failure. There is a second problem: compact on ONE column of a group relocates that column alone and rebuilds its own live/s2e/e2s. That desyncs the co-slotted columns K3 relies on, and discards the bytes of dying slots (compact skips `!live`, dense_store.rs:616). CONFIDENCE: CONFIRMED. NEEDED: add both rows to the table. Get_component_mut needs a compile-time refusal or an untracked route. Compact needs a group-level definition (group-wide, or refused for groups) with move_ticks routed. Also state whether a group's column stores stay reachable through the per-id raw `store_mut` insert/remove/compact surface, which bypasses the anchor and the dying state. Extend U2's trybuild and release-fault tests to cover both paths.
- B2. New defect in D14 / section 10.8 (the fix for C2 and W3): a slot that is already dying when S1 runs is a live ghost body for the whole step. TRACE: removal keeps the bytes (design :239 'keeps the bytes. The slot enters a dying list'), and release happens only in S6's apply window (:242). Now take a despawn, or a Collider removal, applied in any window after S6(i)'s release and before S1(i+1): a gameplay despawn in Update, or a direct `delete_entity` between steps (entity_api.rs:1059-1060 routes it to the dense tombstone). The slot then enters step i+1 dying, still holding its step-i bytes: SIMULATED|DYNAMIC flags, a finite pose, a real bound_radius and a real inv_mass. S1's queries never visit it, because its row is gone, so nothing neutralises it. S2 pairs it, S3 builds manifolds, S5 applies its impulses to live bodies, and gravity/integrate move it. Section 10.8 :705 says 'Dying slots are real bodies until release: the chain simulated them from S1'. That is false for this case. Today the gather drops such a body in the same step, so this is a semantics regression the section 10.8 table does not list. SECOND CONSEQUENCE: apply windows run commands in completion-pop order (schedule.rs:781 `let idx = match completion.pop() {` … :810 `self.systems[i].system.apply(world);`). So when a gameplay despawn completes in the same window as S6, whether the despawned body is a ghost next step, and which slot the next spawn receives, depends on thread timing. That breaks section 2's 'Run-to-run determinism for a fixed op sequence', and the slot numbering feeds the colouring (DENSE-COMPONENTS-PLAN.md:56). GATE CONSEQUENCES: U5's dead-slot-at-origin test is red by construction if the despawn happens between steps, which is the natural shape. U7's 'spawn Y into X's slot between two steps' cannot happen, because X is dying, not free, until the next release, so that test passes vacuously unless it asserts slot(Y) == slot(X). CONFIDENCE: CONFIRMED (plan text plus schedule.rs). NEEDED: slots that are dying at chain start must be inert for that step. The release point must be deterministically ordered against other structural commands. Restate D14 and section 10.8. Re-shape the U5 and U7 tests with explicit slot-equality and ghost-absence assertions.
- B3. The chain invariant introduced by the W3 fix is false for inserts. WHERE: section 11 'group slots do not change owner or bytes between S1 dispatch and S6 completion, so view bases, lengths and slot ownership are stable for the whole chain'; section 2 'no PhysicsBody slot changes owner or bytes'. CODE: a Collider attach applied in any mid-chain window inserts the group. By X-11, a window opens between rounds whenever every running system has completed (schedule.rs:669 `if pending > 0 && (pending == running || running == 0) {`). When the free list is empty, `DenseStore::insert` appends (dense_store.rs:226-242 `None => { // Fresh slot: append at the frontier` … `self.s2e.push(entity);`). An empty free list is the steady state in a world without despawns, since it is fed only by S6's release. So `len()` grows between S2/S4 and S5/S6, and a free slot changes owner from nobody to the new entity. The DEAD bytes make the new slot inert. The length change is the problem. CONSEQUENCE: per-slot scratch sized by one chain system and swept by a later one using its own `len()` goes out of bounds. The design itself sets this up: S4's union-find/island arrays are 'per slot' (section 7 graph-build row), and S5's slot phases and sleep `end_step` sweep slots. An implementer who trusts 'lengths stable for the whole chain' indexes len_S4 out of bounds: a panic, or UB under get_unchecked. CONFIDENCE: CONFIRMED that the invariant is false; PLAUSIBLE for which array trips first. NEEDED: one slot bound per step (for example captured at S1 and used by every chain system), or a per-system len with cross-system per-slot sizing forbidden. Correct the text in sections 2 and 11. Add a mid-chain spawn test with an empty free list to U5.
- B4. The D13 anchor site set is wrong, and the basis claim in X-10 is false. X-10 says the site at entity_api.rs:305-312 'reaches create_entity (the bench path) and command spawns alike'. It does not: `SpawnAtCommand::apply` runs its own sequence (spawn_at_command.rs:383-407 dense required inserts, :415 `world.apply_attach_flags_all(entity);`, :426-477 hook fires, :479-489 'Step 8b … the SOLE dense fire path for this spawn'). Other attach sites: `create_entity_at` (entity_api.rs:354), `create_entity_at_with_pool_ids` (:517), `migrate_entity_insert` (migration_helpers.rs:384) and `migrate_entity_attach_ids` (:1558) for a Collider inserted onto an existing entity, clone (materialize.rs:562/:597), prefab instantiate (prefab.rs:43-47: dense memberships are not captured), and serde `load_archetype` (load_writer.rs:315, which fires no hooks). The load case matters because the design excludes group columns from save (section 8 serde row): a loaded world then has no group at all, which contradicts 'rebuilt by S1 because a loaded slot has SEEN = 0'. Detach sites: `migrate_entity_remove` (:1166) and `migrate_entity_detach_ids` (:1846) besides `delete_entity_core`. A ready census is the existing flag checks: ON_ADD_ANY at entity_api.rs:262, :445, spawn_at_command.rs:434, migration_helpers.rs:1071, :1793, materialize.rs:597; ON_REMOVE_ANY at migration_helpers.rs:1306, :2013, entity_api.rs:791. On insert-migration the anchor must key on Collider being NEWLY attached, or adding any other component to a body inserts a second group. CONSEQUENCE: bodies spawned through Commands, inserted onto existing entities, loaded or instantiated from prefabs silently get no PhysicsBody, so S1 never sees them and they are not simulated. Every planned gate still passes (U4 shadow equivalence, G-jolt and G-alloc anti-vacuity all use the raw create_entity path). This is W5's green-from-emptiness failure again, on the paths gameplay actually uses. CONFIDENCE: CONFIRMED. NEEDED: enumerate every attach and detach site, and add one anti-vacuity test per spawn path (command, insert, clone, prefab, load).
- B5. The clone path contradicts the premise of D14/D15 and can double-insert the group. `materialize_dense_memberships` copies the source's LIVE group bytes into a new slot: materialize.rs:875 `let Some(slot) = store.slot_of(source_id) else {`, :900 `cloned.push((cid, src_bytes.to_vec()));`, :914 `store.insert(entity.id(), bytes, current_tick);`. It is reached from `clone_and_spawn` (entity_api.rs:863-887). D15's proof rests on design :267: 'S1 sets BodyGate.SEEN … on any row whose SEEN bit is 0. Those are fresh inserts, whose DEAD value has SEEN = 0'. A clone arrives with SEEN=1, so it is never marked fresh. If its (LIFO-popped) slot was released from body X at the end of step j−1, the clone's PairCache lookups in step j read X's warm impulse and reference axis, which is exactly W2's consequence, now reached through clone_and_spawn of a projectile. Two more problems. The loop inserts per store, so each column store pops its own free list, which is incompatible with K3's single slot map. And if the anchor also fires at the clone's Collider attach (materialize.rs:597), the group is inserted twice: `DenseStore::insert` only debug-asserts 'already present' (dense_store.rs:205-209). In release, `e2s` is overwritten, leaving an orphaned live slot with the source's pose that S1 never refreshes and nothing ever removes: a permanent ghost body. CONFIDENCE: CONFIRMED (code path); the double insert depends on where the anchor is placed. NEEDED: define what cloning a body does to its group (anchor-created DEAD, or copy with SEEN cleared and one group-aware slot), make the anchor insert idempotent, and add a clone-into-reused-slot case to U7.

## Non-blocking findings

- N1. Rev-2 C1 fix, performance side: declined lanes can quietly shrink the gang. A B1 joiner's sibling sweep batch-steals from lane 0's deque (worker.rs:347 `drain_one(|| stealer.steal_batch_and_pop(local))`). It then pops its own deque once per loop (scope.rs:1540 `let popped = lane.deque().pop();`) and declines each lane task in O(1) until its own scope drains. So one concurrent par_iter join in another system can decline most of a gang's W−1 lanes within microseconds. CONSEQUENCE: in real frames with concurrent par_iter systems, S5's gang degrades toward lane 0 alone. It stays correct but runs serially, and G-jolt (a physics-only schedule) cannot see this. CONFIDENCE: PLAUSIBLE. Options: record effective resident lanes per gang in the receipt; add a G-jolt variant with a concurrent par_iter system; place lane tasks where B1 joiners do not reach them first.
- N2. Section 10.1 lists the Top run sites as worker.rs:96-97, :115-116 and :126-128. It omits :84-85 (`if let Some(t) = deque.pop() { run_task(t);`) and :90-91 (the injector pop). These are the most common top-level runs. If an implementer passed Nested there, lanes would decline, costing parallelism but not correctness. The design should cite all five worker_main sites against the six join sites (scope.rs:1542, :1554, :1564, :1589, :1670, :1675).
- N3. Section 6 says '{S4 → S5} ∥ S7 … S7 is expected to be shorter than S4 + S5, so waiting for it costs nothing'. The executor decrements successors only in `apply_window_drain` (schedule.rs:824-832), which runs only when every running system has completed (:669). S5 therefore cannot dispatch while S7 is still running. The real cost is max(0, S7 − S4), not zero whenever S7 < S4 + S5. The 6-round count is correct; the parallelism claim and the cost rule are not. The section 12 timing should measure S7 against S4. CONFIDENCE: CONFIRMED.
- N4. The section 10.8 grid row omits the geometry pre-pass. `recompute_geometry` sweeps every body (resources.rs:1023-1033). With NaN DEAD radii, the median uses `select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))` (resources.rs:1047-1051), which is not a total order. The in-tree comment at :1041-1043 says 'every radius is finite … so no NaN reaches the compare'. The std documentation leaves the result unspecified and permits a panic for a non-total-order comparator. In addition, `n = bodies.len()` (:1084) counts dead slots. The coarse CSR build (:1356-1361) and `emit_oversized_candidates` (:1526) need the same skip that the count pass gets. This also contradicts X-9 ('only the broadphase needs a new guard'), as does the new SIMULATED gate on the SDF per-body sweep in section 10.8. CONFIDENCE: CONFIRMED that the precondition is violated; PLAUSIBLE for the panic.
- N5. The U5a/U5b addressing is incoherent. U5a ports the kernels to slot-addressed group views, and U5b re-types the RigidSolver seam to slot-addressed `SolverBodies`. But S2–S6, and so the manifold body indices, stay gather/row-addressed until U5 ('U4 … the gather still drives the solve'). So either the U5a/U5b code is dead until U5, or it indexes slot views with gather indices. In the second case, U5b's 'per-scene bit-identity (addressing-only change)' goes red on any scene where gather order ≠ slot order (multi-archetype scenes). State the intermediate addressing, or fold U5a/U5b into U5. CONFIDENCE: CONFIRMED (plan text).
- N6. The section 17 debug_assert 'writeback-visited count == live count minus the dying rows the chain began with' is wrong. S6's query requires RigidBody, so D12 static colliders (Collider without RigidBody) are live but never visited. Mid-chain inserts are live too. And dying slots are not live, so subtracting them is meaningless. As written, it fires in debug on U4's F-1 red-first Trigger test.
- N7. D13 does not say whether removing an ANCHORED group directly, while Collider stays, is rejected the way member removal is. If it is allowed, the Collider silently leaves physics, because S1's queries require GroupSlot.
- N8. Two open questions answered from the tree. Open question 4 (serde): boyko_serialize/src/save.rs:285 `for &cid in world.dense_registry().dense_ids() {` enumerates every dense type, so K3's serde skip flag is required, not conditional. Open question 6: the AVX2 gravity and integrate lanes do consult `simulated` (simd.rs:778 and :902 `active[lane] = if snap.simulated && is_dynamic_row(eff.inv_mass) {`); refresh gates on `inv_mass != 0.0` only (simd.rs:151).
- N9. X-11 in its strong form (successors are decremented only in an apply window) also means the S6-issued release lands in a window shared with any concurrently completing system. That is the root of B2's ordering race; record it next to X-11 so later rungs see it.
- N10. Provenance note: I had no shell, so the `git diff --stat ca582e72 d11962a9 -- crates` check the orchestrator asked for was NOT performed. Every citation above is from the D:/wt/joltab working tree, which the design notes carries uncommitted `joltab` bench edits.

## Fixes the critic confirmed

- C1 deadlock: FIXED. The Top/Nested run-context split covers all 11 `run_task` sites: worker_main top-level at worker.rs:85, :91, :97, :116, :128, and the joins at scope.rs:1542, :1554, :1564, :1589, :1670, :1675, which reach run_task through the import at scope.rs:53. A resident lane's frame sits directly on worker_main, and waiting lanes run no tasks, so a gang wait never has unfinished work beneath it on its own stack. The pass-1 trace (a nested scope's Drop pops an outer lane task) now declines. Nested gangs, lane 0 running inside a join, and lanes stranded in a sibling's deque (reclaimed by lane 0's scope join, then declined) all keep progress. The cost is lost parallelism only (N1).
- W7 park/wake: FIXED. With all four handshake operations SeqCst (`completed` store / `parked` swap against `parked` fetch_or / `completed` load), the waiter and the completer cannot both read stale values. Park's token covers an unpark that arrives before the park. The `threads[]` cells are published through the RMW chain before the swap reads them. GangShared lives until lane 0's scope join, so a completer's unpark loop never touches freed memory. Gang tokens that reach pool parkers (or the reverse) only cause spurious wakes, which every loop tolerates.
- C2, broadphase part: CORRECT. A NaN bound fails the ordered compare at systems.rs:304 (`<=`). The outer loop skips `r_i.is_nan()`. A DEAD BodyVel (inv_mass = 0) keeps the existing kernel gates skipping the slot: simd.rs:151, :197, :245, :778, :902. The remaining gaps are B2 (slots dying at S1) and N4 (grid geometry).
- C3, approach: CORRECT. A `TICKS_TRACKED` const with a const-assert follows the in-tree precedent filter.rs:1013 `const { Self::assert_storage_supports_change_detection() };`. All seven listed routing rows are real sites. The remaining gap is B1: get_component_mut and compact are missing from the table.
- W1: RESOLVED. Q5 is raised with the SP-1 spike and a decision rule, the goal is restated as one authoritative plus one derived form, and the authority window is stated in section 4.1.
- W2 / D15: RESOLVED for slots the anchor creates. I checked the proof against every release timing: despawn before S5 (the dying slot's entries are stored at S5(i), and Y is fresh at step i+1); release in the same window as a re-spawn; the double-buffer swap at colored.rs:3046 `core::mem::swap(&mut self.warm_read, &mut self.warm_write);`. u32 wrap only costs one skipped warm start. The remaining gap is the clone path (B5).
- W3: PARTIALLY RESOLVED. Deferring release is correct for removals during the chain. X-11 is confirmed: schedule.rs:669, with successors decremented only in apply_window_drain (:824-832). S6.after(S7) does order the release after S7's last s2e read. The remaining gaps are B2 and B3.
- W4/W5: the MECHANISM is correct. #[require] is unusable (it has no removal path and does not run on the raw path). User hooks are unusable (hooks/mod.rs:48-54, 'Derive XOR runtime … registering hooks for a derive-hooked type panics'). ArchetypeFlags has free bits: FLAGS_ON_ATTACH = 1 << 12 at archetype_flags.rs:98, so bits 13-15 are unused. The site set is not resolved (B4).
- W6: RESOLVED. GroupSlot reads the slot-map node and each column has one DenseColumnMut, so the two S1 queries write no shared id. S4 ∥ S7 and S5's access set do not conflict.
- W8: RESOLVED. Each saving is credited to the rung whose content removes it, and ordering requires met prerequisites.
- W9: dispositions RESOLVED (seam re-typed, SoftStepSolver/SDF/coupling ported, Foundation-only integrate, U5a bit-oracle gate); the intermediate rung addressing is still open (N5).
- X-10 (hooks and dense routing at entity_api.rs:241, :310-311, :318-319) and X-12 are accurate as quoted. X-10's claim that command spawns reach that site is false (B4).

# Rev 3 patch

## How to read rev 3

- **The rev-2 text above stays exactly as written.** Nothing earlier in this file was edited, including the header's `Revision:` line and the "How this file is laid out" list. The patch's own writer notes ask for both to change. They are not applied in place, because this file is append-only; this note stands in for them. The current revision is **rev 3 after two critique passes**, and the file's layout is: rev 2 → rev1→rev2 changelog → Critique log - pass 1 → preserve list → Critique log - pass 2 → Rev 3 patch.
- **The patch below supersedes every section it names.** Each `P-§…` block lists the rev-2 text it removes (quoted verbatim) and the text that replaces it. Read a named section as rev 2 with those removals and additions applied. A section the patch does not name stands as rev 2 wrote it.
- The patch's closing table ("Changelog: rev 2 → rev 3") is the rev 2 → rev 3 changelog.
- **N10 provenance, closed by the writer (2026-09-11).** `git diff --stat ca582e72 d11962a9 -- crates`, run in `D:/wt/joltab` with HEAD at `d11962a90adf04ae10718c8f9b81683a98e4192b`, reports only `crates/boyko_physics/Cargo.toml` (+21) and `crates/boyko_physics/benches/jolt_parity_pyramid.rs` (+16): 2 files, 37 insertions, no deletions. No physics or ECS source file changed between the commit the design cites and HEAD. The patch text below still records N10 as open, because the patch is reproduced verbatim.

# Rev 3 patch: PHYSICS-ECS-UNIFICATION-DESIGN.md (rev 2 → rev 3)

**Provenance.** I re-read every new citation below in the `D:/wt/joltab` working tree during this round. I have no shell, so `git diff --stat ca582e72 d11962a9 -- crates` has **not** been run by me either. It is still owed (N10). No cargo, no timing.

**Writer notes (for whoever applies this):**
- Set the header `Revision:` to "rev 3 after two critique passes".
- Add "Critique log - pass 2" to "How this file is laid out".
- Append the finding table at the end of this patch as "Changelog: rev 2 → rev 3".

Findings touch overlapping sections. Each section is therefore patched **once**, and the patch is labelled with every finding it serves. The order is: §0, §2, §3, §5, §6, §7, §8, §9, §10, §11, §12, §14, §16, §17, §18, Checklist. After those come the refutation / accept-as-open responses and the table.

---

## P-§0: corrections table (B2, B4, N4, N9, N1)

**Removed (X-9 row, verbatim):**
> | X-9 | (new) Dead slots need a new guard in every sweep | `solver/simd.rs:197` `if snap.simulated && is_dynamic_row(eff.inv_mass) {`; `:259-260` "blending only the `inv_mass != 0` lanes so a static lane is byte-untouched" | Gravity, integrate and refresh already skip `inv_mass == 0` lanes. Only the broadphase needs a new dead-slot guard (§10.8). |

**Added:**
> | X-9 | Dead slots need a guard in every sweep that reads geometry | Gravity/integrate gate on `simulated` and `inv_mass`: scalar `simd.rs:197`, `:245`; AVX2 `simd.rs:778`, `:902` `active[lane] = if snap.simulated && is_dynamic_row(eff.inv_mass) {`; refresh `simd.rs:151` `if eff.inv_mass != 0.0 {` | Those kernels need no new guard. New guards are needed in: broadphase AllPairs; **every** per-body loop of the grid (geometry fold `resources.rs:1023-1033`, median `:1044-1051`, count `:1253`, fill `:1304`); the SDF per-body sweep (§10.8). **Corrected in rev 3 (N4):** rev 2 said only the broadphase needed one. |

**Removed (X-10 row, verbatim):**
> | X-10 | (new) Hooks and dense routing fire on the raw spawn path | `ecs_master/entity_api.rs:241` "Step 6 (Phase 14a §3.2): fire on_add / on_insert hooks."; `:310-311` `for &(cid, bytes) in dense_components {` / `self.dense_insert_and_fire(entity, archetype_id, cid, bytes);`; `:318-319` `drop(scope);` / `self.drain_deferred_hook_queue();` | A kernel structural step at this site reaches `create_entity` (the bench path) and command spawns alike. Basis of D13. |

**Added:**
> | X-10 | Hooks and dense routing fire on `create_entity`'s path | (citations unchanged) | True for `create_entity` only. **Corrected in rev 3 (B4):** `SpawnAtCommand::apply` runs its own sequence (`spawn_at_command.rs:383-489`; it registers at `:352`). An entity's archetype pointer is written at 14 sites in 5 forms (X-13). D13 no longer rests on this row. |

**Removed (X-11 Consequence cell, verbatim):**
> A command issued by the chain tail is applied only after every chain system has completed. Basis of the release point (D14).

**Added:**
> Two consequences. (1) A successor is decremented only in `apply_window_drain` (`schedule.rs:824-832`), so a system dispatched in round r delays the next round until it completes (N3, §6). (2) A command that a chain system issues shares its window with every system completing in the same round, and it is ordered against them by completion pop (X-14). That is the root of B2's ordering race (N9). Rev 3 therefore issues **no** physics command. Release is a step inside S1's body (D14).

**Removed (X-12 Consequence cell, verbatim):**
> A task can be run from inside a blocking join. This is the root of C1. The fix keys on the run context (§10.1).

**Added:**
> A task can be run from inside a blocking join. This is the root of C1. Rev 3 puts lane work in no structure any join reaches: the lane ticket board is probed only by `worker_main`'s top level (§10.1). Rev 2's run-context split is dropped (N1, N2).

**Added (two new rows after X-12):**
> | X-13 | (pass 2, B4) Every site that writes an entity's archetype pointer | `entity_master.rs:343` in `register_entity_with_ptr`, callers `entity_api.rs:230`, `:425`, `:563`, `spawn_at_command.rs:352`, `materialize.rs:554`, `prefab.rs:786`. `entity_master.rs:303` in `register_batch`, callers `spawn_batch_command.rs:767`, `load_writer.rs:569`. Direct `EntityInland::new` assignments at `migration_helpers.rs:977-978` (`migrate_entity_insert`, fn at `:384`), `:1378-1379` (`migrate_entity_remove`, `:1166`), `:1767-1768` (`migrate_entity_attach_ids`, `:1558`), `:2088-2089` (`migrate_entity_detach_ids`, `:1846`). `entity_master.rs:360` `deallocate_entity`, callers `entity_api.rs:221`, `:1088`, `:1099`. `ecs_master.rs:1027` `self.entity_master.clear();` | This is the complete set of archetype transitions. Basis of the binder (D13). |
> | X-14 | (pass 2, B2/N9) Commands in one apply window are applied in completion-pop order | `schedule.rs:781` `let idx = match completion.pop() {` … `:810` `self.systems[i].system.apply(world);` | Entity ids and dense slots handed out in one window already depend on thread timing when two systems of one round both issue structural commands. This is true for every storage and predates this design. Rev 3 adds no physics command to any window. |

Depends on: D13, D14, §10.1, §10.8.

---

## P-§2: invariants (B2, B3)

**Removed (verbatim):**
> - Run-to-run determinism for a fixed op sequence.

**Added:**
> - Run-to-run determinism for a fixed **applied** op sequence. Commands from systems that complete in the same apply window are applied in completion-pop order (X-14). Their relative order is therefore timing-dependent today, for every storage. This design issues no physics command into any window (D14), so it adds no case. A deterministic in-window apply order is a separate scheduler item (§14).

**Removed (verbatim):**
> - **New chain invariant (D14):** from `physics_prepare`'s dispatch to `physics_writeback`'s completion, no `PhysicsBody` slot changes owner or bytes through a kernel structural operation.

**Added:**
> - **Chain invariant (D14; restated in rev 3 for B3).** Let `slot_bound` be the high-water mark of `PhysicsBody` that S1 records in `PhysicsStep` right after its release step. From S1's dispatch to S6's completion:
>   - (a) A slot `< slot_bound` that is live or dying at S1 changes bytes only through chain systems. Its owner changes only by live → dying, never to a new entity.
>   - (b) A slot that is free at S1, and every slot `≥ slot_bound`, holds `DEAD` bytes for the whole chain, even if a mid-chain window inserts it for a new entity.
>   - (c) `len()` **may grow** mid-chain: a spawn with an empty free list appends. So every per-slot sweep and every per-slot scratch array in the chain is bounded by `slot_bound`, never by a view's own `len()`.
>   - Rev 2's "no slot changes owner or bytes" was false under (c) and for free slots under (b).

Depends on: D14, §11, §10.2–§10.5.

---

## P-§3 D6 (N3)

**Removed (verbatim):**
> - S7 is ordered before S6 (§6).

**Added:**
> - S7 is ordered `.after(S4)` and is unordered with S5 and S6 (§6). It needs no ordering against S6: a slot it reads through `s2e` cannot be reused before the next S1 (D14).

---

## P-§3 D7 (N1, N2)

**Removed (verbatim):**
> **New in rev 2 (C1).** A lane task runs **resident** only when a worker runs it from its top-level loop. Run from any join context, it **declines** and returns at once (§10.1). This is enforced by the task's run context, not by documentation.

**Added:**
> **Lane placement (C1; rev 3 for N1).** Lane work is never a pool task. Lane 0 publishes W−1 **tickets** on a per-pool `LaneBoard` (§5, §10.1). Only `worker_main`'s top level takes tickets. No join route probes the board. So every lane has only `worker_main` beneath it on its stack, by construction. A B1 joiner of an unrelated system cannot consume a lane, so it cannot shrink the gang. This was N1's trace: the joiner batch-steals lane 0's deque (`worker.rs:347` `if let Some(t) = drain_one(|| stealer.steal_batch_and_pop(local)) {`), then declines each lane in O(1) at `scope.rs:1540-1542`.
> - Rev 2's `RunCtx { Top, Nested }` change to the 11 `run_task` sites is dropped: nothing a join can run is ever a lane.
> - The gang also stops opening a pool scope, which removes rev 2's two heap acquisitions per gang.

**Removed (verbatim):**
> - A declined lane is lost parallelism, never lost correctness: lane 0 alone can complete every phase.

**Added:**
> - An untaken ticket is lost parallelism, never lost correctness: lane 0 alone can complete every phase, and at FINAL it cancels untaken tickets. Tickets go untaken only when no worker is at top level: every worker is busy or in a join.
> - Lane entry costs one indirect call per lane per gang (a monomorphised trampoline). Chunk loops stay monomorphised.

**Alternatives (append to D7's list):**
> - Rev 2's run-context decline: correct but lossy. Any concurrent `par_iter` join could decline most of a gang's lanes (N1).
> - A receipt counter plus a G-jolt variant with a concurrent `par_iter` system: this detects the loss without removing it. Kept as telemetry (§12 P2).

---

## P-§3 D10 (B1)

**Removed (verbatim):**
> - Every runtime tick consumer of `DenseStore` is routed around untracked stores (§8 K2 table).

**Added:**
> - Every runtime tick consumer of `DenseStore` is routed around untracked stores (§8 K2 table). Rev 2 missed `EcsMaster::get_component_mut` and `DenseStore::compact`.
> - A G-form source census pins the table. It lists every non-test site that calls `added_ticks_ptr` / `changed_ticks_ptr` / `move_ticks` / `stamp_slot_ticks` / `write_added_tick` / `write_changed_tick` through a `DenseStore` or its column. A new site is red until it gets a row.
> - Group columns (K3) are not `DenseStore`s, so no row of the table can receive a group column id.

---

## P-§3 D13: whole section replaced (B4, B5, N7)

**Removed (entire rev-2 D13, verbatim):**
> ### D13 (new, W4/W5): Group membership is **anchored** on `Collider` by the kernel's structural core
>
> **What.**
> - `#[component(anchor_group = PhysicsBody)]` on `Collider`.
> - The kernel inserts the group, with every column at its `DEAD` value, wherever `Collider` is attached. It removes the group wherever `Collider` is detached or the entity despawns.
> - The attach site is the one where dense routing already runs on every spawn path (X-10: `entity_api.rs:305-312`). The detach sites are the component-remove core and `dense_despawn_fire_and_tombstone` (`entity_api.rs:1051-1060`).
> - The anchor is a new `ArchetypeFlags` bit, so a world with no anchored group pays the existing `flags.is_empty()` test (`entity_api.rs:256` `if !flags.is_empty() {`).
>
> **Why:**
> - `#[require]` has no removal path (`required.rs:32-38` holds constructors only) and expands only on command paths, not on `EcsMaster::create_entity` (`entity_api.rs:171-180`). The bench and 26 `create_entity(` calls in physics use that raw path (W5).
> - A user hook on `Collider` is not usable either: hooks are "Derive XOR runtime … registering hooks for a derive-hooked type panics" (`hooks/mod.rs:48-54`). The anchor must not consume the user's hook slot.
>
> **Alternative:** a fixture migration of 23 files to command spawns. Rejected: it leaves the raw path silently group-less for users too.
>
> **Trade-off:** a new structural-core step (cold; structural ops only).

**Added:**
> ### D13 (rev 3): Group membership is **anchored** on `Collider` and enforced at the **binder**, the one place an entity's archetype can change
>
> **What:**
> - `#[component(anchor_group = PhysicsBody)]` on `Collider`.
> - **Anchor mask.** Every `Archetype` carries `anchor_mask: u8`, computed once when the archetype is created. Bit g is set iff the archetype's component set contains group g's anchor. At most 8 anchored groups per process; registration asserts this.
> - **Binder.** An entity's `EntityInland` archetype pointer is written only by four `EcsMaster` functions (new file `ecs_master/binder.rs`):
>   - `bind_new`: single spawn, clone, prefab;
>   - `bind_batch`: batch spawn, serde load;
>   - `rebind`: the four migrations;
>   - `unbind`: despawn and create-rollback.
>
>   Together they replace the 14 sites of X-13. `EcsMaster::clear` resets every group store.
> - **Compile-time completeness.** The inland store's archetype-changing writers (today `register_entity_with_ptr`, `register_batch`, `deallocate_entity`, and `get_mut`/`IndexMut` handing out `&mut EntityInland`) require a `&BindToken`. Only `ecs_master::binder` can construct that token (private constructor).
>   - The swap-remove fixups use the tokenless `fix_unit_index(id, row)`, which cannot change the archetype pointer. Those fixups are at `migration_helpers.rs:963-964`, `:1369-1370`, `:1753-1754`, `:2079-2080` and `entity_api.rs:1094-1097`.
>   - Reads keep their current forms: `inland(id) -> Option<EntityInland>`, plus a raw-pointer projector for the `unsafe_ecs_cell.rs:329` `&raw const` read.
>   - Any future structural path that moves an entity between archetypes therefore cannot compile without going through the binder.
>   - Grep finds no caller of these writers outside `boyko_ecs/src` (only comments in benches/tests), so narrowing them is an internal API change.
> - **Transition.** Each binder reads `old = anchor_mask(old archetype)` (0 when there is none) before the write and `new = anchor_mask(new archetype)` (0 when there is none). If `old != new` it calls `#[cold] #[inline(never)] anchor_transition(entity, old, new)`:
>   - for each bit in `new & !old`: group insert (DEAD bytes, SEEN = 0);
>   - for each bit in `old & !new`: group remove (deferred for `PhysicsBody`, D14).
>
>   The read goes through a raw projection, following the `entity_api.rs:1046` `addr_of!((*archetype_ptr).flags).read()` discipline.
> - **Adding any other component to a body leaves `old == new`,** so no second group is inserted. This is the critic's "Collider NEWLY attached" condition, enforced by the mask XOR rather than by per-site reasoning.
> - **Where the transition runs relative to existing steps:**
>   - At each spawn/clone/prefab/load it runs at the existing commit point. Materialize, for example, maps the entity "only NOW — after full success" (`materialize.rs:548-554`). So a rolled-back spawn never inserts a group.
>   - At despawn, `unbind` runs where `deallocate_entity` runs today (`entity_api.rs:1088`, `:1099`). That is after `remove_entity`, and the inland still names the old archetype there.
>   - At spawn the transition runs before the hook fires (for example `entity_api.rs:230` precedes step 6 at `:241`), so a user `on_add` for `Collider` already sees the group.
> - **Idempotence (B5).** Group insert release-asserts `!e2s.contains(entity)` through a cold message fn. A kernel bug is then a panic, never an orphaned live slot. `DenseStore::insert` only debug-asserts (`dense_store.rs:205-209`).
> - **Group columns are not dense stores.** Their ids are a new `StorageKind::Group`, never `Dense`.
>   - They have no `DenseStore` and are not in `DenseRegistry::dense_ids()`. `create_archetype` asserts that no archetype's `component_ids` ever contains one.
>   - So every existing per-id dense walk skips them **by construction**: clone `materialize.rs:862` (`for &cid in world.dense_registry.dense_ids() {`), save `save.rs:285`, `check_ticks.rs:195-196`, and the despawn tombstone walk `entity_api.rs:1128`.
>   - This is the structural fix for B5. The clone path cannot copy live group bytes, cannot pop per-column free lists, and cannot double-insert. The clone gets its group from `bind_new`'s transition: DEAD bytes, SEEN = 0.
> - **Every `StorageKind` dispatch becomes an exhaustive `match`.** There are 57 occurrences in 19 files today, mostly `== StorageKind::Dense`. Adding `Group` is then a compile error at each site until it is routed (for example the direct read APIs at `component_api.rs:201`, `:274`) or refused (every id-based structural entry point; the loader returns `LoadWriteError` instead of panicking).
> - **Membership changes only through anchor transitions (N7).**
>   - `insert::<T>`, `remove::<T>`, `migrate_entity_remove::<C>` and the `Bundle` derive const-assert `!T::IS_GROUP_COLUMN`.
>   - Id-based paths refuse a `Group` id.
>   - No API removes a group directly.
>   - So "the group removed while `Collider` stays" is not a reachable state, and `Collider` cannot silently leave physics.
> - **Debug reconciliation (mechanical backstop).** In debug builds, at the end of every `apply_window_drain` (`schedule.rs:841`) and after every direct `EcsMaster` structural call, the kernel checks: for each anchored group g, Σ rows of archetypes with bit g == `live_count(g)`. The cost is O(#archetypes), debug only. A missed path is red in any test that exercises it.
>
> **Why:**
> - B4: the anchor site set was wrong. There are 14 inland-write sites in 5 forms (X-13), and SpawnAt, spawn batch, load, clone and prefab do not reach `entity_api.rs:305-312`.
> - The binder is the one choke point every path must already pass: an entity whose inland does not name its archetype is broken today.
> - `#[require]` and user hooks remain unusable (rev 2's reasons: `required.rs:32-38`; `hooks/mod.rs:48-54`).
>
> **Alternatives:**
> - Rev 2's single site: rejected (B4).
> - An anchor call at each hook-fire site (the ON_ADD_ANY / ON_REMOVE_ANY census): rejected. Load fires no hooks (`load_writer.rs:315`), clone gates its fire on `cloner.fire_hooks` (`materialize.rs:575`), and a per-site list goes stale with the next structural path.
> - A free `ArchetypeFlags` bit (13-15): rejected. It allows only 3 groups, and it couples to the hook-flag recompute (`archetype_flags.rs:457-469`).
> - Keeping group columns as `StorageKind::Dense` with per-column `DenseStore`s: rejected. That is B5 (per-store free lists, live-byte clone copy) and B1b (single-column compact).
>
> **Trade-off:**
> - A refactor of 14 call sites and a visibility change on the inland store.
> - Per structural op: two byte loads and one compare. Both archetype headers are already touched by the op.
> - A debug-only O(#archetypes) check per window.
> - `anchor_mask` may grow the `Archetype` header (open question 8). The header is read on structural paths only; queries read `columns` at offset 0.

Depends on: §5 group store, §8 K3, §12 U2/U4, §17.

---

## P-§3 D14: whole section replaced (B2, B3, N9)

**Removed (entire rev-2 D14, verbatim):**
> ### D14 (new, C2/W3): Deferred slot release and `DEAD` values
>
> **What.** A group may declare `RELEASE = Deferred`. Then:
> - **Remove** clears `live`, sets `s2e = TOMBSTONE`, drops the `e2s` entry, and **keeps the bytes**. The slot enters a `dying` list, not the free list. Group columns are `Copy`, so there is no drop.
> - **Release** happens only through a kernel command, `Commands::release_dense_group::<G>()`, applied in an apply window. It writes each column's `const DEAD: Self` into every dying slot and moves them to the free list.
> - Fresh appends are also initialised to `DEAD`.
> - `physics_writeback` (S6), the chain tail, issues the release command once per step. S6 is ordered after S7.
> - Because an apply window opens only when every dispatched system has completed (X-11), release never lands inside [S1, S6].
>
> **DEAD values:**
>
> (table)
>
> **Why:**
> - Rev 1's zero-fill made a tombstone a zero-radius sphere at the origin (`components.rs:125-129` `#[repr(C)]` … `pub enum ColliderShape {` `Sphere {`, tag 0). That is C2.
> - Freeing mid-chain let S3 read a freshly freed slot, and let S7 map a reused slot to a new entity (W3).
> - With deferred release, a body despawned mid-chain keeps its bytes, is solved to the end of that step, is not written back (its row is gone), and its slot is recycled only after the chain.
>
> **Semantics:** a structural change mid-chain takes effect at the next step boundary. That is today's semantics as well; today it can also misapply rows in release builds, which `systems.rs:1153-1155` catches only in debug.
>
> **Trade-off:** dying slots live until the end of the chain. Memory is bounded by despawns per step. A world that anchors the group but runs no physics must call the release itself (`EcsMaster::release_dense_group::<G>()`); a debug diagnostic fires when the dying count exceeds the live count.

(The DEAD-values table is kept verbatim. Only the surrounding text is replaced.)

**Added:**
> ### D14 (rev 3): Deferred slot release **at the chain head**, and `DEAD` values
>
> **What.** A group may declare `RELEASE = Deferred`. Then:
> - **Remove** (only through the binder, D13) clears `live`, sets `s2e = TOMBSTONE`, drops the `e2s` entry and **keeps the bytes**. The slot enters `dying`, not `free`. Group columns are `Copy`, so there is no drop.
> - **Release is a step inside the chain head's system body, not a command.** S1 holds `DenseGroupMut<PhysicsBody>` (§8 K3/K6). Serially, before its two `par_iter` loops, S1 calls `release_dying()`. That call:
>   - writes each column's `const DEAD: Self` into every dying slot;
>   - appends those slots to `free` in dying order, so LIFO reuse is deterministic;
>   - clears `dying`;
>   - returns `slot_bound = len()`, which S1 stores in `PhysicsStep.slot_bound` (B3).
> - **Insert** pops `free` (bytes already DEAD) or appends a slot and writes DEAD into every column. The anchor supplies no bytes; S1 fills them.
> - No physics system issues a command. `Commands::release_dense_group` is **deleted**.
>
> **Slot state by removal time:**
>
> | Removed | Released | Simulated in step i? | Simulated in step i+1? |
> |---|---|---|---|
> | before S1(i) | head of S1(i) | no | no |
> | in a mid-chain window, after S1(i) and before S6(i) completes | head of S1(i+1) | **yes**, with the bytes S1(i) wrote. S2(i)'s pairs may name it. It is not written back, because its row is gone | no |
> | between steps, after S6(i) and before S1(i+1) (the **B2 case**) | head of S1(i+1), before any sweep | yes (it was live for all of step i) | **no**: DEAD before S2(i+1) reads anything |
>
> **Why:**
> - **B2, the ghost.** Every slot that is dying when S1 runs is DEAD before the step's first geometry read. So a between-step despawn, the natural shape, is never simulated.
> - **B2 / N9, the ordering.** Release runs inside a system body, so the round structure fixes its position relative to every apply window: after every window that precedes S1's round, before every window that follows it. Completion-pop order (X-14) orders commands only, and physics issues none. Whether a despawned body is a ghost, and which slot a later spawn receives, no longer depends on which system popped first.
> - **What remains timing-dependent is the pre-existing class.** A structural system that is **unordered** with S1 lands before or after the gather by timing, today and after this change. So does the in-window order among gameplay commands (X-14, §14).
> - **C2 and W3 stay closed.** A mid-chain removal keeps the bytes that S3/S5/S7 may still read through S2's pairs. A slot freed at S1 is DEAD for the whole step, because S2 runs after the release and so no pair names it.
> - **Access.** `DenseGroupMut<G>` declares a write on every column id of G and on G's **recycle node**. The recycle node is a registry id distinct from the slot-map node that `GroupSlot`/`DenseSlots` read (§8 K3). Release touches only column bytes and the recycle lists; `e2s`/`s2e`/`live` were already updated at removal. A reader of `GroupSlot` or `DenseSlots` running in S1's round therefore reads nothing that release writes. Field projection rule: §5.
>
> **DEAD values:** (rev-2 table, unchanged)
>
> **Semantics:** unchanged. A structural change takes effect at the next step boundary.
>
> **Trade-off:**
> - O(dying × 168 B) serial writes at the head of S1 (arith.), bounded by removals since the last S1. Zero in a churn-free step.
> - A slot becomes reusable only after the next S1. So a despawn followed by a spawn within one step never reuses the slot, and `free` never holds more than one step's removals beyond steady state.
> - A world that anchors the group but runs no chain calls `EcsMaster::release_dense_group::<G>()`. That call panics (cold) if any `DenseGroupMut<G>` has ever been initialised in the world, because an exclusive release between S1 and S5 would DEAD-fill slots that S2's pairs still name. The flag `chain_head_registered` is set in `DenseGroupMut::init_state`. The debug diagnostic "dying count > live count" stays.
> - `dying` and `free` live on `VmColumn`s: address space only, no heap, like `s2e` (`dense_store.rs:118`).

Depends on: §2 chain invariant, §6 S1 row, §10.2, §10.8, §11, D15.

---

## P-§3 D15: proof restated for the rev-3 release point (B2 consequence)

**Removed (verbatim):**
> **Proof it suffices, for any release timing.**

**Added:**
> **Proof it suffices, under the rev-3 release point (D14).**

**Removed (verbatim):**
> 3. Y fills s first at S1(j). Y can hold s only after X's removal and release, so S5(i) < S1(j) and i ≤ j−1.

**Added:**
> 3. Y fills s first at S1(j). s became free only at the head of some S1(k), and X's bytes left s there. So X held s at S5(i) only for i ≤ k−1. Y was inserted in a window after S1(k), so j ≥ k+1, and therefore i ≤ j−2.

**Removed (verbatim):**
> 5. If i+1 < j, they are gone. If i+1 = j, Y is fresh at step j and every lookup naming s is skipped. Step j's store holds only Y's entries.

**Added:**
> 5. i+1 ≤ j−1 < j, so step i's entries are gone before Y's first lookup.
>
> The `fresh_step` skip is therefore not load-bearing under rev 3's release point. It is kept for two reasons. It costs one compare on a line the lookup already loads (Cost, below). And it keeps `PairCache` safe for any future group with `RELEASE = Immediate`. The debug belt "no `PairCache` hit with a fresh body" stays.
>
> Clones (B5) are covered by the same proof: a clone's group is anchor-created (D13), with SEEN = 0 and DEAD bytes.

---

## P-§5: data structures (B1, B3, B5, N1)

**Removed (verbatim):**
> #[repr(C)]
> struct GangShared {                      // K5b; lives in lane 0's frame for the gang's duration
> …
>     threads: [UnsafeCell<MaybeUninit<Thread>>; 64], // written by each lane before its first park,
>                                          // published by the SeqCst `parked` RMW (§10.1)
> }

**Added:**
```rust
#[repr(C)]
struct GangShared {                      // K5b; lives in lane 0's frame until lane 0's exit wait returns
    claim: CachePadded<AtomicU64>,       // (phase:32 | next_chunk:32), CAS-claimed
    done: [CachePadded<AtomicU32>; 2],   // completion counter per phase parity
    completed: CachePadded<AtomicU32>,   // last fully completed phase (FINAL = u32::MAX)
    parked: CachePadded<AtomicU64>,      // bit per lane parked after spin budget (lanes ≤ 64)
    exited: CachePadded<AtomicU32>,      // taken tickets whose lane has returned (SeqCst RMW)
    exit_waiting: AtomicBool,            // lane 0 parked in the exit wait (SeqCst; W7 Dekker pair)
    phase_n: AtomicU32,                  // debug SPMD agreement check
    poisoned: AtomicBool,
    panic: UnsafeCell<Option<Box<dyn Any + Send>>>, // written once by the `poisoned` CAS winner.
                                         // Box only on the panic path (resume_unwind needs it)
    entry: unsafe fn(*const (), &GangLane<'_>),     // monomorphised by par_phases; 1 call per lane per gang
    program: *const (),                  // &F in lane 0's frame
    threads: [UnsafeCell<MaybeUninit<Thread>>; 64], // unchanged from rev 2
}

// boyko_threadpool — one per pool, inside PoolInner (pool lifetime). Replaces the per-gang pool scope.
#[repr(C)]
struct LaneBoard {
    pending: CachePadded<AtomicU64>,      // bit b: ticket b published and neither taken nor cancelled
    in_use:  CachePadded<AtomicU64>,      // bit b: slot b owned by a live gang (CAS-claimed by lane 0)
    slots:   [CachePadded<LaneTicket>; 64], // 64 × 64 B = 4 KiB per pool
}
#[repr(C)]
struct LaneTicket { gang: AtomicPtr<GangShared>, lane: AtomicU32 } // written before the `pending` publish
```

**Added (after `ScratchCohort`):**
```rust
// K3 — one per registered DenseGroup, in DenseRegistry::groups[GroupId]. Not in `slots` / `dense_ids`.
pub(crate) struct DenseGroupStore {
    // slot-map node (its own registry id): read by GroupSlot / DenseSlots; written only in apply windows
    e2s: EntitySlotMap,
    s2e: VmColumn<EntityId>,             // TOMBSTONE for dying/free
    live: LiveBitmap,
    // recycle node (its own registry id): written by DenseGroupMut (the chain head) and in apply windows
    recycle: UnsafeCell<Recycle>,
    // one untracked ComponentPool per column id (K1 contiguous ids, K2 no tick pages); DEAD-initialised
    columns: [ComponentPool; MAX_GROUP_WIDTH],
    width: u8,
    anchor: ComponentId,                 // Collider for PhysicsBody
    chain_head_registered: bool,         // set by DenseGroupMut::init_state; refuses EcsMaster release (D14)
}
struct Recycle { dying: VmColumn<u32>, free: VmColumn<u32> }  // VM-backed, no heap
// Every system param projects its node with addr_of!((*store).<field>). No struct-wide
// &DenseGroupStore / &mut DenseGroupStore is formed while a round runs (BUG-MIGRATE-TB-1 discipline),
// so S1's recycle write and a concurrent GroupSlot read of e2s are disjoint borrows under Tree Borrows.

// PhysicsStep gains:
//     slot_bound: u32,   // written by S1 after release_dying(); every chain sweep uses it (B3)
```

**Removed (verbatim):**
> - `GangShared` outlives every lane task because the gang's scope join precedes lane 0's frame return (the `scope.rs:1281` structure).
> - `Thread` handles are dropped by lane 0 after the join.

**Added:**
> - `GangShared` outlives every lane that took a ticket. Lane 0 cancels untaken tickets and returns only after `exited == taken` (§10.1). On unwind a drop guard runs the same cancel-and-wait.
> - A `LaneTicket` slot is released (`in_use` bit cleared) only after that wait. A taker reads its ticket before it runs and never touches it after `exited.fetch_add`.
> - `Thread` handles are dropped by lane 0 after the exit wait.
> - A group store's columns are POD and drop with the registry.

---

## P-§6: system graph (B2, N3, W6 re-check)

**Removed (S1 row fragment, verbatim):**
> writes `DenseColumnMut<BodyVel/BodyPose/BodyMaterial/BodyInertia/BodyGate>`;

**Added:**
> writes `DenseGroupMut<PhysicsBody>` (all five column ids plus the recycle node; runs `release_dying()` first, D14);

**Removed (S6 row fragment, verbatim):**
> `DenseColumn<BodyVel/BodyPose/BodyGate>`, `Commands` (issues `release_dense_group::<PhysicsBody>()`)

**Added:**
> `DenseColumn<BodyVel/BodyPose/BodyGate>`, `Res<PhysicsStep>`

**Removed (Order block, verbatim):**
> - S1 → S2 → S3 → {S4 → S5} ∥ S7 → S6.
> - S6 is `.after(S5).after(S7)`. That makes the release command land strictly after S7's last `s2e` read (D14).
> - S7 is an O(p) merge walk. It is expected to be shorter than S4 + S5, so waiting for it costs nothing; §12's timing verifies this.
>
> **Rounds:** S1, S2, S3, {S4 ∥ S7}, S5, S6 = **6**.

**Added:**
> - S1 → S2 → S3 → S4 → S5 → S6. S7 is `.after(S4)` and unordered with S5 and S6.
> - S6 needs no edge to S7. There is no release command any more, and a slot S7 reads through `s2e` cannot be reused before the next S1 (D14).
> - **Why `.after(S4)` and not `.after(S3)` (N3).** Successors are released only in an apply window, and a window opens only when every running system has completed (X-11; `schedule.rs:669`, `:824-832`).
>   - Dispatched in S4's round, S7 would delay S5 by max(0, S7 − S4). Rev 2's "costs nothing whenever S7 < S4 + S5" was wrong.
>   - Dispatched in S5's round, S7 costs max(0, S7 − S5), and S5 is the longest stage. The S4 → S7 edge carries no data.
> - §12's timing measures S7 against S5.
>
> **Rounds:** S1, S2, S3, S4, {S5 ∥ S7}, S6 = **6**.

**Removed (verbatim):**
> - The only intra-system write on a group node in S1 is each `DenseColumnMut<T>`, one per column id.

**Added:**
> - S1's only group write param is `DenseGroupMut<PhysicsBody>`: one write per column id plus one on the recycle node. `GroupSlot` reads the slot-map node. The three node kinds are distinct registry ids (§8 K3).
> - S5 keeps its per-column `DenseColumn`/`DenseColumnMut` params. It holds no recycle write.

---

## P-§7: layout table (N4)

**Removed (grid row, Verdict cell's ECS-form text, verbatim):**
> same column; the count pass skips `!(r >= 0.0)` rows (one predictable branch)

**Added:**
> same column. Every per-body grid loop (geometry fold, median input, count, fill) skips `!(r >= 0.0)` rows, one predictable branch each. `n` = live-radius count.

---

## P-§8: kernel features (B1, B3, B4, B5, D14, N1)

**Removed (K5b row, verbatim):**
> | K5b | Gang (`par_phases`) + task run context | §10.1, including the nesting rule | S2 grid, S3, S5, soft solve | transform propagation, animation hierarchy, render multi-pass culling | KE16 (closed) |

**Added:**
> | K5b | Gang (`par_phases`) + lane ticket board | §10.1: lanes live on a per-pool board that only `worker_main`'s top level probes | S2 grid, S3, S5, soft solve | transform propagation, animation hierarchy, render multi-pass culling | KE16 (closed) |

**Removed (K6 row, verbatim):**
> | K6 | Group release command | `Commands::release_dense_group::<G>()` and `EcsMaster::release_dense_group::<G>()` | S6 | any deferred-release group | K3 |

**Added:**
> | K6 | Group release | `DenseGroupMut<G>::release_dying()`, a step in the chain head's body (D14). `EcsMaster::release_dense_group::<G>()` exists for chain-less worlds and is refused once a chain head is registered | S1 | any deferred-release group | K3 |

**Removed (K3 detail bullet, verbatim):**
> - removal is by group only (a trybuild fixture rejects removing a member);

**Added:**
> - Column ids are `StorageKind::Group`. There is no `DenseStore`, they are not in `dense_ids()`, and they never appear in an archetype's `component_ids` (D13).
> - Membership changes only through binder transitions (D13). Insert release-asserts absence (B5).
> - `insert::<T>`/`remove::<T>`/`Bundle` const-assert `!IS_GROUP_COLUMN`. Id-based structural entry points refuse `Group` ids. Trybuild fixtures cover insert and remove.
> - **No compact.** Slot stability is the group's contract (D1; `PairCache` keys are slots). `DenseRegistry::store_mut`'s kind check becomes a release `assert!` (`dense_registry.rs:113-120` is a `debug_assert!` today). So no per-id surface (`store_mut` → `insert`/`remove`/`compact`/`build_view`; `ecs_master.rs:599` → `dense_registry.rs:112`; `views.rs:73-95`) can reach one column of a group (B1b).
> - Query terms over group columns (`&T`, `&mut T`, `GroupSlot`) match archetypes by `anchor_mask`. The binder guarantees every row of an anchored archetype has a slot, so per-row membership is a `debug_assert!`, not the per-row `e2s.contains` filter that plain dense needs.
> - `DenseGroupMut<G>` param: writes every column id and the recycle node; `release_dying()`; typed views.
> - A group column read before the entity's first S1 returns DEAD (NaN pose). This is documented on `BodyPose`.

**Removed (invariant line, verbatim):**
> - free ⟺ s ∈ free ∧ bytes == DEAD.

**Added:**
> - free ⟺ s ∈ free ∧ bytes == DEAD;
> - `dying` is empty immediately after `release_dying()`;
> - during a chain, every slot ≥ `PhysicsStep.slot_bound` has DEAD bytes.

**Removed (the entire K2 routing table, verbatim):**
> | Consumer | Site | Untracked route |
> |---|---|---|
> | `Ref<T>` / `Mut<T>` fetch | `dense_store.rs:738` `pub(crate) fn added_ticks_ptr(`, `:746` `pub(crate) fn changed_ticks_ptr(` | Compile-rejected by a const-assert in `Ref`/`Mut` `init_state`; the accessor debug-asserts tracked (unreachable) |
> | `Added<T>` / `Changed<T>` | `filter.rs:969`, `:1300` | Second const-assert `T::TICKS_TRACKED` beside the bitset one |
> | `DenseStore::insert` stamping | `dense_store.rs:257-259` `self.column.write_added_tick(…)` / `write_changed_tick(…)` | `if self.column.is_tracked()` (cold, structural) |
> | `insert_with_ctor` | `dense_store.rs:372-373` | same |
> | `insert_or_replace` | `dense_store.rs:418` `self.column.write_changed_tick(slot as usize, current_tick);` | same |
> | Serde load stamping | `dense_store.rs:758-765` `pub(crate) unsafe fn stamp_slot_ticks(` | same. Group columns are also excluded from serde as derived data, rebuilt by S1 because a loaded slot has SEEN = 0. Not verified: whether the serde seam enumerates every dense type; if it does, K3 adds the skip flag |
> | `check_ticks` dense arm | `check_ticks.rs:24-25` "every LIVE slot of every [`DenseStore`] in the world's [`DenseRegistry`]" | skip untracked stores |
> | Table component with `ticks = "none"` | derive | rejected by the derive (K2 is dense-only) |

**Added:**
> | Consumer | Site | Untracked route |
> |---|---|---|
> | `Mut<T>` dense fetch | `iters/query/data/mut_.rs:441-442` `let added = *(*store.added_ticks_ptr().add(slot)).get();` / `let changed_tick = store.changed_ticks_ptr().add(slot);` | Compile-rejected: `const { assert!(T::TICKS_TRACKED) }` in `Mut`'s `init_state` |
> | `Ref<T>` | `ref_.rs:166-178` has a table arm only and no dense arm today | The same const-assert, so a future dense arm cannot bypass it |
> | `Added<T>` dense | `filter.rs:1204` | Const-assert `T::TICKS_TRACKED` beside `filter.rs:969-974` |
> | `Changed<T>` dense | `filter.rs:1492` | same |
> | **`EcsMaster::get_component_mut<T>` dense arm (B1a)** | `component_api.rs:657` `if component_registry::storage_kind(cid.0) == component_registry::StorageKind::Dense {`; `:673` `let added: Tick = unsafe { *(*store.added_ticks_ptr().add(slot)).get() };`; `:674-675` | Compile-rejected: `const { assert!(T::TICKS_TRACKED) }` at fn entry. The fn is generic over `T`, so the `filter.rs:1013` post-monomorphisation precedent applies. No untracked route is offered: S1 overwrites a direct write to a derived group column, and a plain untracked dense column is written through `get_component_raw_mut` (`component_api.rs:253`), whose dense arm resolves `slot_of` + `row_ptr` and touches no tick |
> | `DenseStore::insert` stamping | `dense_store.rs:257-259` | `if self.column.is_tracked()` (cold, structural) |
> | `insert_with_ctor` | `dense_store.rs:371-373` | same |
> | `insert_or_replace` | `dense_store.rs:418` | same |
> | Serde load stamping | `dense_store.rs:758-765` | same |
> | **`DenseStore::compact` (B1b)** | `dense_store.rs:607` `pub fn compact(&mut self) {`; `:640` `self.column.move_ticks(read, write);`. Reached through `ecs_master.rs:599` → `dense_registry.rs:112` → `compact`, and through `views.rs:93-94` | Ticks move only `if self.column.is_tracked()`; the data bytes still move. Groups cannot reach `compact` (next row) |
> | Any per-id `DenseRegistry` surface given a group column id | `dense_registry.rs:112-131` `store_mut`, `:136` `store`, `:146` `store_existing_mut` | Group ids never get a `DenseStore`. `store_mut` release-asserts `StorageKind::Dense`; `store`/`store_existing_mut` return `None` |
> | `check_ticks` dense arm | `check_ticks.rs:229`, `:236` | Skip untracked stores. Group columns are not in `dense_ids()` (`check_ticks.rs:195-196`) |
> | Serde save/load of group columns | `save.rs:285` `for &cid in world.dense_registry().dense_ids() {` | Excluded by construction (they are not in `dense_ids()`). Load rebuilds them: `bind_batch`'s transition (`load_writer.rs:569`) inserts DEAD groups with SEEN = 0 and S1 fills them. This answers rev 2's "not verified" and N8 |
> | Table component with `ticks = "none"` | derive | rejected by the derive |

**Removed (verbatim):**
> **Not needed:** an in-place `EnableMut` toggle. The dense `free: Vec<u32>` (`dense_store.rs:126`) and the new `dying` list are kernel-storage bookkeeping, settled by class under the allocator plan. `dying` goes on a `VmColumn`, the way `s2e` does (`dense_store.rs:118` `s2e: VmColumn<EntityId>,`).

**Added:**
> **Not needed:** an in-place `EnableMut` toggle. The group's `dying` and `free` both go on `VmColumn`s, the way `s2e` does (`dense_store.rs:118` `s2e: VmColumn<EntityId>,`). Plain dense's `free: Vec<u32>` (`dense_store.rs:126`) stays kernel bookkeeping under the allocator plan.

---

## P-§9: public API (B1, B3, B4, D14, N1)

**Removed (verbatim):**
> impl Commands<'_, '_> { pub fn release_dense_group<G: DenseGroup>(&mut self); }   // K6
> impl EcsMaster { pub fn release_dense_group<G: DenseGroup>(&mut self); }

**Added:**
```rust
pub struct DenseGroupMut<'w, G: DenseGroup>;   // SystemParam: write on every column id of G + G's recycle node
impl<G: DenseGroup> DenseGroupMut<'_, G> {
    pub fn release_dying(&mut self) -> u32;    // K6: DEAD-fill + dying→free; returns slot_bound (= len afterwards)
    pub fn view<T: GroupColumn<Group = G>>(&self) -> TypedDenseView<'_, T>;
}
impl EcsMaster { pub fn release_dense_group<G: DenseGroup>(&mut self); } // K6; panics if a DenseGroupMut<G> was initialised
// trait Component { const TICKS_TRACKED: bool = true; const IS_GROUP_COLUMN: bool = false; ... }
// get_component_mut<T> const-asserts TICKS_TRACKED; insert/remove/Bundle const-assert !IS_GROUP_COLUMN
// PhysicsStep { ..., pub slot_bound: u32 }
```

**Removed (verbatim):**
> // internal: the Task trampoline gains a `RunCtx { Top, Nested }` argument (§10.1)

**Added:**
> // internal (boyko_threadpool): `LaneBoard` in `PoolInner`, probed at `worker_main`'s three top-level points (§10.1).
> // internal (boyko_ecs): the binder (`bind_new` / `bind_batch` / `rebind` / `unbind`), `BindToken`, `Archetype::anchor_mask` (D13).

---

## P-§10.1: gang protocol (N1, N2, C1 re-argued)

**Removed (verbatim):**
> **Run context (C1):**
> - The pool's task trampoline receives `RunCtx`.
> - `worker_main`'s top-level runs (`worker.rs:96-97`, `:115-116`, `:126-128`) pass `Top`.
> - Every join route (`join_on_worker`, `join_external_helping`, `join_external`, which run tasks through `run_task` imported at `scope.rs:53`) passes `Nested`.
> - Ordinary spawned closures ignore it.
> - Cost: one register argument, no TLS. Fallback, if the trampoline change is rejected: a depth counter in `LaneDeposit`'s `_pad: u32` (`tls.rs:141`), costing two TLS writes per join, unmeasured.
>
> **Lane task entry:**
> - `Nested` → return immediately (declined).
> - `Top` → run the program as a **resident** lane.

**Added:**
> **Lane placement: the ticket board (C1 closed by construction; N1).**
> - **Lanes are not pool tasks.** Lane 0 publishes tickets on the pool's `LaneBoard` (§5). A ticket is taken only at `worker_main`'s three top-level probe points:
>   - after the own-deque pop (`worker.rs:84-87`), before the injector;
>   - in the pre-`mark_idle` re-poll (`:115-118`);
>   - in the post-`mark_idle` re-poll (`:126-130`).
>
>   No join route probes the board (`scope.rs:1540-1567`, `:1577`, `:1669-1678`). Neither do `pop_any`, the injector or any deque.
> - **Publish (lane 0).** For each lane l in 1..k: CAS a free bit in `in_use`, then write `(gang, l)` into that slot with Relaxed. Then `pending.fetch_or(bits, SeqCst)`. Then up to k × `unpark_one_idle` (`worker.rs:443`), whose first step is `publish_fence` (`worker.rs:434-436`, `:450-451`). This is the producer half of the pool's existing Race-C protocol. If `in_use` is saturated, lane 0 publishes fewer tickets. W=1 publishes none.
> - **Take (worker_main only).**
>   1. `p = pending.load(Relaxed)`. If zero, fall through: one load of a read-mostly line.
>   2. Otherwise, for the lowest set bit b: `old = pending.fetch_and(!(1<<b), SeqCst)`. If `old` had b, this worker owns ticket b.
>   3. Load `(gang, l)` with Acquire.
>   4. Run the lane under `catch_unwind` as a **resident** lane.
>   5. `gang.exited.fetch_add(1, SeqCst)`. Then, if `gang.exit_waiting.load(SeqCst)`, unpark lane 0.
>
>   After step 5 the taker touches neither the ticket nor the gang. The post-`mark_idle` probe is preceded by `publish_fence()`. That fence is the worker half of the Race-C pair (`worker.rs:418-429`): `mark_idle` itself is only `Release` (`worker.rs:386`).
> - **Cancel and exit wait (lane 0, after FINAL, and in a drop guard on unwind).**
>   1. `old = pending.fetch_and(!mine, SeqCst)`, then `taken = popcount(mine & !old)`.
>   2. Wait until `exited == taken`: spin; then `exit_waiting.store(true, SeqCst)`; re-check `exited` with SeqCst; then `park`. Both sides of this Dekker pair are SeqCst, the same argument as the W7 handshake.
>   3. `in_use.fetch_and(!mine, Release)`, then return.
> - **A lost wake costs a lane, never correctness.** Lane 0 completes every phase alone, and at FINAL it cancels untaken tickets.
> - Rev 2's `RunCtx` is dropped. No join can run a lane, so there is nothing to decline. The 11 `run_task` sites (`worker.rs:85`, `:91`, `:97`, `:116`, `:128`; `scope.rs:1542`, `:1554`, `:1564`, `:1589`, `:1670`, `:1675`) are unchanged.

**Removed (verbatim):**
> **Lanes.** `min(W, max_useful_lanes)`. W−1 lane tasks are spawned in one pool scope on lane 0's deque; lane 0 is the caller. Correctness needs no other lane, because lane 0 can complete every phase. W=1 spawns nothing.

**Added:**
> **Lanes.** `min(W, max_useful_lanes)`. Lane 0 is the caller and publishes the other lanes as tickets. Correctness needs no other lane. The receipt records `taken` per gang (N1 telemetry).

**Removed (verbatim):**
> **Deadlock freedom (C1):**
> - Only two kinds of frame ever block in a gang wait:
>   - (i) lane 0. It waits for chunks claimed by resident lanes.
>   - (ii) resident lanes. Below the lane frame there is only `worker_main`, because it was run with `Top`.
> - A chunk finishes using only frames above it on its own thread. A nested scope joins its own tasks (`scope.rs:28-30` "Nested scopes cannot deadlock: a joiner either runs a ready task or parks with a wake guaranteed by its own scope's last completer").
> - A waiting lane runs no task, so it never stacks work above its own wait.
> - The critic's trace (a nested scope's Drop pops an outer lane task) now declines in O(1).
> - A B1 joiner of an unrelated system that steals a lane task declines it, so it does not stay until FINAL.
> - A lane task stranded in a busy sibling's deque is reclaimed by lane 0's final scope join through sibling steal, and declined.

**Added:**
> **Deadlock freedom (rev 3):**
> - The frames that block: lane 0 (phase waits and the exit wait) and taken lanes (phase waits only).
> - A taken lane sits directly on `worker_main`, so no unfinished work lies beneath it on its stack.
> - A phase wait needs chunks claimed by lanes. A claimed chunk runs on its claimer's stack and blocks only in joins of its own nested scopes or gangs (`scope.rs:28-30`). Those joins never need a ticket, because tickets are not tasks and no join sees them.
> - A waiting lane runs no task.
> - The exit wait needs every taken lane to return. A taken lane either observes `completed == FINAL` at entry, or it runs to FINAL, which lane 0 publishes before it cancels.
> - **Nested gang:** a chunk that opens a gang is that gang's lane 0. Idle top-level workers take its tickets, or nobody does and it runs alone.
> - **Lane 0 above an unrelated join:** for example a system task picked up inside a KE16 join (`scope.rs:1547-1551`). Its waits depend only on its own lanes, never on the join beneath it.
> - The critic's pass-1 trace (a nested scope's Drop pops an outer lane task) cannot occur: no deque holds a lane.

**Removed (verbatim):**
> **Panic.** The panicking lane sets `poisoned` and counts its chunk done. Other lanes skip the remaining phases, and the scope re-raises (`scope.rs:1410-1411`).

**Added:**
> **Panic.**
> - A panicking lane wins or loses the `poisoned` CAS; the winner stores the payload. The lane counts its chunk done and exits, still counting `exited`.
> - Other lanes skip the remaining phases.
> - Lane 0 re-raises with `resume_unwind` after the exit wait.
> - A panic in lane 0 runs cancel and exit wait in its drop guard before unwinding out of the frame that owns `GangShared`.
> - A lane panic never reaches `worker_main`'s abort path (`worker.rs:178-182`), because the take path catches it.

**Removed (verbatim):**
> **Cost per gang:** one scope (two heap acquisitions until the allocator plan's scope arena). Per phase: one CAS and one `fetch_add` per chunk, one SeqCst store, one swap. Spin and park are `#[cold]`.

**Added:**
> **Cost per gang:**
> - No pool scope and no heap acquisition.
> - Open: k slot CASes, one `fetch_or`, ≤ k unparks.
> - Close: one `fetch_and`, the exit wait, one `fetch_and`.
> - Per phase, unchanged: one CAS and one `fetch_add` per chunk, one SeqCst store, one swap. Spin and park are `#[cold]`.
> - Every `worker_main` iteration pays one Relaxed load.

Kept verbatim: Claim, Complete, Wait (W7), Late lane, SPMD contract.

---

## P-§10.2 S1 (B2, B3)

**Removed:** none (insertion at the top of §10.2, before "Two `par_iter` loops in one system. Per row:").

**Added:**
> **Release first (D14, K6).** Serially, before either loop: `step.slot_bound = group.release_dying()`. Every slot dying when S1 starts receives each column's DEAD and moves to `free`. Cost: O(dying × 168 B). After this line no slot `< slot_bound` is dying.

**Removed (verbatim):**
> **`PhysicsStep`.** `dt` and the broadphase selection (the former `select_broadphase`) are written here once, and `step` is incremented.

**Added:**
> **`PhysicsStep`.** `dt`, the broadphase selection (the former `select_broadphase`) and `slot_bound` are written here once, and `step` is incremented. Every chain system reads `slot_bound` for its per-slot bounds (§2 (c)).

---

## P-§10.3 broadphase (B3, N4)

**Removed (verbatim):**
> - The outer loop skips dead `i` with `if r_i.is_nan()`. Inner dead `j` fail the predicate (§10.8).

**Added:**
> - Both loops run over `0..step.slot_bound`. The outer loop skips dead `i` with `if r_i.is_nan()`. Inner dead `j` fail the predicate (§10.8).

**Removed (verbatim):**
> **Grid:** a gang over build_csr / emit. Not verified: that `emit_passes` maps 1:1 onto phases.

**Added:**
> **Grid:** a gang over build_csr / emit, over slots `0..step.slot_bound`. Every per-body loop applies the live-radius predicate `r >= 0.0`, which is false for NaN:
> - the geometry fold and median input (`resources.rs:1023-1051`). `n` becomes the number of pushed radii, not `bodies.len()` at `:1084`;
> - the count pass (`:1253`);
> - the fill pass (`:1304`).
>
> The oversized legs (`:1340-1369`, `:1526`) consume only rows those passes produced, so they inherit the skip. A debug assert checks that every row in `oversized` and `cell_bodies` has a live radius.
>
> With the skip, the in-tree premise at `:1041-1043` ("every radius is finite … so no NaN reaches the compare") holds again. So the `select_nth_unstable_by` comparator at `:1049-1050` sees a total order, and the std-unspecified or panicking case N4 describes is unreachable.
>
> Not verified: that `emit_passes` maps 1:1 onto phases.

---

## P-§10.5 solve (B3)

**Removed:** none (insertion after the "Phase count" paragraph).

**Added:**
> **Bounds (B3).** Every slot phase (`for_each(slot chunks)`), every per-slot scratch array (S4's union-find and island arrays included) and the sleep `end_step` sweep range over `0..step.slot_bound`, never over a view's `len()`. Each chain system debug-asserts `view.len() >= step.slot_bound` on entry.

---

## P-§10.6 writeback (D14)

**Removed (verbatim):**
> Finally, `commands.release_dense_group::<PhysicsBody>()`.

**Added:**
> S6 issues no command. Release happens at the head of the next S1 (D14).

---

## P-§10.8: whole section replaced (B2, B3, N4)

**Removed (entire rev-2 §10.8, verbatim):**
> ### 10.8 Dead-slot inertness, per loop (C2)
>
> "Dead" = dying (bytes intact, simulated to the end of its step by design) or free (DEAD bytes).
>
> | Loop | Touches free slots? | Guard | Result |
> |---|---|---|---|
> | Broadphase AllPairs | yes | `bound = r_i + r_j` = NaN, and `delta.length_squared() <= bound * bound` is false for NaN under an ordered compare | no pair. A future SIMD broadphase must use `_CMP_LE_OQ` and must not route NaN through `min`/`max` (memory: NaN inverts under NMin/NMax); test-pinned |
> | Broadphase grid | yes | count pass skips `!(r >= 0.0)` | never binned (a saturating NaN→int cast would otherwise put it in cell 0) |
> | Narrowphase / `narrowphase_sdf` | no / yes | pairs only name live slots; the SDF per-body sweep is gated on `BodyGate.flags & SIMULATED` | none |
> | Graph build | yes (per-slot arrays) | `inv_mass == 0` ⇒ static singleton, like any static row today | none |
> | Gravity / integrate / refresh | yes | `snap.simulated && is_dynamic_row(eff.inv_mass)` (`simd.rs:197`, `:245`); AVX2 blends only `inv_mass != 0` lanes (`simd.rs:259-260`); port keeps the gate, now from `BodyGate` | byte-untouched |
> | Solve / warm apply | no | contacts only name live slots | none |
> | Writeback | only rows with membership | flags 0 | no write |
> | Events | no | TOMBSTONE check for dying | no start event |
> | Sleep | yes | DYNAMIC gate | none |
>
> **Dying slots** are real bodies until release: the chain simulated them from S1, so their bytes are consistent. They do not produce NaN.

**Added:**
> ### 10.8 Dead-slot inertness, per loop (C2; rev 3 for B2, B3, N4)
>
> "Dead" means free (DEAD bytes) or `≥ slot_bound` (DEAD bytes, appended mid-chain). A slot that is dying when S1 starts is released by S1 **before any sweep**, so it is free and DEAD for the whole step (D14). A slot that dies **after** S1 is not dead for its step. It keeps the bytes S1 wrote and is simulated to the end of the step. This is the only case rev 2's closing sentence covered.
>
> | Loop | Range | Touches dead slots? | Guard | Result |
> |---|---|---|---|---|
> | S1 release | dying list | writes them | — | DEAD before any sweep |
> | Broadphase AllPairs | `0..slot_bound` | yes | `bound = r_i + r_j` = NaN; `<=` is false for NaN under an ordered compare; outer loop skips `r_i.is_nan()` | no pair. A future SIMD broadphase must use `_CMP_LE_OQ` and must not route NaN through `min`/`max`; test-pinned |
> | Grid geometry fold + median | `0..slot_bound` | yes | skip `!(r >= 0.0)`; `n` = live-radius count | no NaN reaches `min`/`max`, the median comparator or `cbrt(n)` |
> | Grid count + fill | `0..slot_bound` | yes | same predicate in **both** passes | never binned; count and fill agree (a NaN→int saturating cast would otherwise put it in cell 0, `resources.rs:1360-1362`) |
> | Grid oversized legs | rows produced by the passes | no | inherited; debug assert | none |
> | Narrowphase | pairs | no | pairs name only slots that were live, or dying-with-bytes, at S2 | none |
> | `narrowphase_sdf` per-body sweep | `0..slot_bound` | yes | `BodyGate.flags & SIMULATED` (DEAD flags = 0) | none |
> | Graph build | per-slot arrays sized `slot_bound` | yes | `inv_mass == 0` ⇒ static singleton | none |
> | Gravity / integrate / refresh | `0..slot_bound` | yes | scalar `simd.rs:197`, `:245`; AVX2 `:778`, `:902`; refresh `:151`; the port keeps the gate, sourced from `BodyGate` | byte-untouched |
> | Solve / warm apply | contacts | no | contacts name only pair slots | none |
> | Writeback | query rows | only rows with membership | flags 0 for DEAD; per-row debug assert (§17) | no write |
> | Events | pairs | no | TOMBSTONE check for dying | no start event |
> | Sleep | `0..slot_bound` | yes | DYNAMIC gate | none |

---

## P-§11: multithreading (B3, D14)

**Removed (verbatim):**
> - **Structural changes** happen only in apply windows (X-11). By D14, group slots do not change owner or bytes between S1 dispatch and S6 completion, so view bases, lengths and slot ownership are stable for the whole chain, not just per system (W3).

**Added:**
> - **Structural changes** happen only in apply windows (X-11), and a window may open between any two chain rounds. The §2 chain invariant is what holds from S1 dispatch to S6 completion. View bases are address-stable (`VmReservation`), so a view taken in any round is valid. A view's `len()` is **not** the chain's bound; `PhysicsStep.slot_bound` is (B3).

**Removed (verbatim):**
> Dead slots are written only by kernel release (apply window, exclusive).

**Added:**
> Dead slots are written by exactly two writers:
> - `release_dying` at the head of S1. S1 holds the write on every column id and on the recycle node; no reader of those ids runs in S1's round; `GroupSlot` readers touch a disjoint field, per the §5 projection rule.
> - Group insert in apply windows, which are exclusive.

**Removed (verbatim):**
> `GangShared` is `Sync`; its `threads` cells are written before the SeqCst publish and read after the swap.

**Added:**
> `GangShared` is `Sync`; its `threads` cells are written before the SeqCst publish and read after the swap. `LaneBoard` is `Sync`. A ticket slot is written before the SeqCst `pending` publish and read after a winning SeqCst `fetch_and`.

---

## P-§12: gates, timing, rungs (B1–B5, N1, N3, N5)

**Removed (stage list bullet, verbatim):**
> - apply;

**Added:**
> - apply;
> - events (S7), compared against S5, which it now overlaps (N3);

**Removed (U2 row, verbatim):**
> | U2 | K2 + K3 + K6 (kernel only): groups, DEAD, deferred release, `GroupSlot`, anchor | trybuild: `Changed<T>`, `Added<T>`, `Ref<T>`, `Mut<T>` on a `ticks="none"` type fail to compile; removing a group member fails to compile. Miri Tree Borrows on views and `range_mut`. Proptest on slot-state invariants (live/dying/free) under random spawn/despawn/remove-anchor/re-add/release. Release-build test: insert + `check_ticks` + serde load on an untracked store do not fault |

**Added:**
> | U2 | K2 + K3 + K6 (kernel only): `StorageKind::Group` with every dispatch made an exhaustive `match`; group store; DEAD; `release_dying`; `DenseGroupMut`; `GroupSlot`; the binder (`BindToken`, `anchor_mask`, 14 sites converted) | **Compile gates:** the workspace builds only with every `StorageKind` match routed. **Trybuild:** `Changed<T>`, `Added<T>`, `Ref<T>`, `Mut<T>` and `EcsMaster::get_component_mut::<T>` on a `ticks="none"` type fail to compile; `insert::<G-column>`, `remove::<G-column>`, and a `Bundle` containing a group column fail; writing an inland without a `BindToken` from outside the binder fails. **Tick routing (B1):** a plain dense `ticks="none"` store under insert, remove ×k, `compact`, `check_ticks` and serde load. This runs in the default debug profile, where `component_pool.rs:1809-1831` and `:1955-1958` `debug_assert!` any unrouted tick access; a missing route is therefore red. `#[should_panic]` for `dense_registry_mut().store_mut(<group column id>)`, and for `EcsMaster::release_dense_group` after a `DenseGroupMut` init. **Proptest:** the slot-state machine (live/dying/free, DEAD on free, `slot_bound`, `s2e`/`e2s` round-trip) under random spawn/despawn/remove-anchor/re-add/non-anchor migration/clone and `release_dying` at arbitrary points. **Miri (Tree Borrows):** `release_dying` on one thread concurrent with a `GroupSlot` read of `e2s` on another; views and `range_mut` |

**Removed (U4 row, verbatim):**
> | U4 | `PhysicsBody` group anchored on `Collider`; S1 fills it (the gather still drives the solve) | shadow equivalence against gathered rows (raw `create_entity` path, as in the bench). **F-1 red-first:** a `Trigger` overlap is reported. `Collider` removal and re-add. Anti-vacuity |

**Added:**
> | U4 | `PhysicsBody` group anchored on `Collider`; S1 releases and fills it (the gather still drives the solve) | Shadow equivalence against gathered rows (raw `create_entity` path, as in the bench). **F-1 red-first:** a `Trigger` overlap is reported. **Anti-vacuity per path (B4).** Each of the following asserts group `live_count` and `slot_of(e)` after the op, and after one step that S1 filled the slot (SEEN set): `create_entity`; `create_entity_at`; `create_entity_at_with_pool_ids`; `Commands::spawn` (SpawnAt); spawn batch; clone with `fire_hooks` on and off; prefab instantiate; serde save → load; `Collider` inserted onto an existing entity (typed command and id-based); **a non-`Collider` component inserted onto a body** (live count and slot unchanged: no second group); `Collider` removed (typed and id-based) and re-added; despawn (direct, command, hierarchy cascade); `clear()`. The debug reconciliation check (D13) runs after every apply window in all of them |

**Removed (U5a row, verbatim):**
> | U5a | Port `apply_gravity`, `position_integrate`, `refresh_inertia` (scalar oracles and AVX2) from `(&[BodyEffective], &[BodyState])` (`simd.rs:120-122`, `:169`, `:213-215`) to the SoA group views plus the `BodyGate` lane gate. Arithmetic op sequence unchanged | scalar ↔ AVX2 bit-oracle on a fixed corpus plus dead/static/non-simulated lanes; pre/post-port per-kernel bit-identity; FMA `do-not-fuse` asm check re-run |

**Added:**
> | U5a | **Kernel-only (N5).** Add slot-addressed ports of `apply_gravity`, `position_integrate`, `refresh_inertia` (scalar oracles and AVX2) over SoA group views plus the `BodyGate` lane gate, beside the AoS kernels (`simd.rs:120-122`, `:169`, `:213-215`). The arithmetic op sequence is unchanged. Production keeps calling the AoS kernels until U5 wires the ports and deletes the AoS ones. The rung has the shape of U2/U3 | Scalar ↔ AVX2 bit-oracle on a fixed slot-addressed corpus, including dead/static/non-simulated lanes; AoS ↔ SoA per-kernel bit-identity on the same corpus (addressing-independent); FMA `do-not-fuse` asm check re-run |

**Removed (U5b row, verbatim):**
> | U5b | Re-type the `RigidSolver` seam (`solver/mod.rs:63-67` `fn solve(` … `scratch: &mut SolverScratch,`) to `SolverBodies`; port `SoftStepSolver` (ids 509/470/469/474/473, `vn_initial` 477); port `physics_narrowphase_sdf` and the coupled soft step's body reads; re-key `SoftRigidReaction` by slot | the existing reference-solver tests (23 files) green; per-scene bit-identity of the reference solver pre/post (addressing-only change) |

**Added:** (row deleted; folded into U5, below)

**Removed (U5 row, verbatim):**
> | U5 | Switch S2–S6 to slots; delete id 511, `touched`, the `BodyEffective` rebuild; parallel S1/S6; register `physics_integrate` only in Foundation mode; S6 after S7 | `{1,N}`; tolerance gate vs pre-rung (A5); pyramid bit-identity (one archetype, spawn order = slot order: critic-confirmed via `jolt_parity_pyramid.rs:113`); assert `touched`-set ≡ gate-filter set before deletion; **mid-chain despawn test** (despawn in S2's window ⇒ no NaN, no wrong-entity event); **dead-slot-at-origin test** (despawn a body at (0,0,0) resting on the floor ⇒ pair and manifold counts equal the survivors' only) |

**Added:**
> | U5 | **Addressing (N5):** through U4 every body index in S2–S6 is a gather row; from U5 every one is a slot, switched in this one rung. No rung mixes them. Content: switch S2–S6 to slots bounded by `slot_bound`; wire U5a's kernels and delete the AoS ones; delete id 511, `touched`, the `BodyEffective` rebuild; re-type the `RigidSolver` seam (`solver/mod.rs:63-67`) to `SolverBodies`; port `SoftStepSolver` (ids 509/470/469/474/473, `vn_initial` 477), `physics_narrowphase_sdf` and the coupled soft step's body reads; re-key `SoftRigidReaction` by slot (the former U5b); parallel S1/S6; register `physics_integrate` only in Foundation mode; S7 `.after(S4)` | `{1,N}`; tolerance gate vs pre-rung (A5) on every scene. Bit-identity on single-archetype scenes, where gather order = slot order (the pyramid, critic-confirmed via `jolt_parity_pyramid.rs:113`), for both the coloured and the reference solver. The 23 reference-solver files green. Assert `touched`-set ≡ gate-filter set before deletion. **Mid-chain despawn (S2's window):** the body is in step i's pairs (assert non-empty) and absent from step i+1's; no NaN; no wrong-entity event. **Dead-slot-at-origin, between steps (B2):** despawn a body at (0,0,0) resting on the floor after the schedule run returns; at step i+1, pair and manifold counts equal the survivors' only, and the slot's bytes == DEAD after S1. **The same test with the despawn in S2's window:** included at i, excluded at i+1. **Release-order determinism (B2):** a gameplay system `.after(physics_solve)`, so in S6's round, despawns one body and spawns one every step; 20 runs at W=8; the per-step `s2e` hash sequence is identical across runs. **Mid-chain spawn with an empty free list (B3):** a world with no despawns, a spawn in S3's window; assert `len` grew past `slot_bound`, no panic, the new body inert in step i (flags 0, in no pair) and fresh at i+1; the §10.5 bound debug asserts hold |

**Removed (U7 row, verbatim):**
> | U7 | `PairCache` merge + `fresh_step` | reuse test: despawn X, spawn Y into X's slot between two steps and within one apply window ⇒ no warm impulse or axis inherited |

**Added:**
> | U7 | `PairCache` merge + `fresh_step` | **Reuse (B2):** despawn X between steps i and i+1; spawn Y in a window after S1(i+1), with the spawner ordered `.after(physics_prepare)`. Assert `slot(Y) == slot(X)` (non-vacuity) and that Y's first simulated step inherits no warm impulse or axis. **Same-window despawn+spawn:** assert `slot(Y) != slot(X)`, since X is dying, not free (documents D14). **Clone into a reused slot (B5):** despawn X between steps; clone a live contacting body Z in a window after S1(i+1). Assert `slot(clone) == slot(X)`, the clone's `BodyGate` SEEN == 0 before its first S1, group `live_count` +1 exactly (no double insert), and no inherited warm start |

**Removed (P2 row, verbatim):**
> | P2 | K5b gang; S5 on one gang; soft solve on a gang | `{1,N}`; nested-gang stress (§17); G-alloc ≤ 16 per step at W=8; `pool.scope` census 0 |

**Added:**
> | P2 | K5b gang with the lane ticket board; S5 on one gang; soft solve on a gang | `{1,N}`; nested-gang stress and board loom model (§17); G-alloc ≤ 16 per step at W=8 (budget unchanged until measured; the gang no longer allocates); `pool.scope` census 0. **N1 telemetry:** the G-jolt receipt records `taken` per gang, and a G-jolt variant adds a concurrent `par_iter` system. Its `taken` distribution and T(8) are recorded, not gated: timing-dependent |

---

## P-§14: orchestrator decisions (N1, B2)

**Removed (verbatim):**
> - the RunCtx trampoline vs the TLS-depth fallback;

**Added:**
> - gang spin budget and the exit-wait spin budget;
> - **deterministic in-window apply order** (scheduler item, not part of any rung here). Drain the completion queue into the preallocated `executor_scratch`, then apply in ascending system index instead of pop order (`schedule.rs:781-810`). Cost is O(k log k), k ≤ systems per round. It removes X-14's timing dependence for every storage. It changes the order in which commands and hooks from one window run, so it goes to the orchestrator with numbers. Recommended.

---

## P-§16: integration (B1, B4, D14, N1)

**Removed (verbatim):**
> - `ecs_master/entity_api.rs`: the anchor step at `:305-312` and the remove/despawn sites;

**Added:**
> - `ecs_master/binder.rs` (new): `bind_new` / `bind_batch` / `rebind` / `unbind`, `BindToken`, `anchor_transition`. The 14 X-13 sites are converted:
>   - `entity_api.rs:221`, `:230`, `:425`, `:563`, `:1088`, `:1099`;
>   - `spawn_at_command.rs:352`; `spawn_batch_command.rs:767`; `load_writer.rs:569`;
>   - `materialize.rs:554`; `prefab.rs:786`;
>   - `migration_helpers.rs:977`, `:1378`, `:1767`, `:2088`.
>
>   `ecs_master.rs:1027` `clear()` resets the group stores.
> - `entity/entity_master.rs` + `entity/inland_store.rs`: token-gated archetype writers, `fix_unit_index`, the `inland()` accessor, and a raw projector for `system/unsafe_ecs_cell.rs:329`.
> - `archetype/archetype.rs`: `anchor_mask: u8`, computed at creation. `create_archetype` refuses `Group` ids.
> - `component_registry`: `StorageKind::Group`, and the 57 dispatch occurrences made exhaustive `match`es.
> - `component/dense/dense_registry.rs`: the group table, and the `store_mut` release assert. `dense_store.rs:640`: `move_ticks` routed.
> - `ecs_master/component_api.rs:635`: the `get_component_mut` const-assert.
> - `schedule/schedule.rs:841`: the debug reconciliation check.

**Removed (verbatim):**
> - `system/params`: `DenseColumn*`, `DenseSlots`, `Commands::release_dense_group`;

**Added:**
> - `system/params`: `DenseColumn*`, `DenseGroupMut`, `DenseSlots`, `GroupSlot`.

**Removed (verbatim):**
> - the `RunCtx` trampoline argument across `worker.rs` / `scope.rs` run sites;

**Added:**
> - `LaneBoard` in `PoolInner`; board probes at `worker.rs:84-87`, `:115-118`, `:126-130` (post-`mark_idle` probe preceded by `publish_fence`). No change to any `run_task` site or join route.

---

## P-§17: validation (B1–B5, N1, N6)

**Removed (verbatim):**
> - **nested gang/scope stress:** at W ∈ {2,4,8}, a phase body calls `par_iter`, `for_range` and a nested `par_phases`, with a watchdog (fails on > 1 s). A B1-joiner steals a lane task and must decline (a counter asserts ≥ 1 decline in a forced scenario);

**Added:**
> - **nested gang/scope stress:** at W ∈ {2,4,8}, a phase body calls `par_iter`, `for_range` and a nested `par_phases`, with a watchdog (fails on > 1 s). A variant runs a concurrent `par_iter` system whose B1 joins span the whole gang. The watchdog must pass, and `{1,N}` must hold on the solve;
> - **board:** exactly-once lane numbering across take/cancel races; a ticket taken after FINAL returns at once; lane 0 never returns before `exited == taken` (poisoned-payload test: a lane panics after taking; lane 0 re-raises after the wait);

**Removed (verbatim):**
> - F-1, F-3, FRESH reuse (U7), mid-chain despawn, dead-slot-at-origin, Collider remove/re-add;

**Added:**
> - F-1, F-3, FRESH reuse (U7, with slot equality), clone into a reused slot, mid-chain despawn, dead-slot-at-origin (between steps and mid-chain), release-order determinism, mid-chain spawn with an empty free list, Collider remove/re-add, non-anchor migration keeps the slot, per-path anchor anti-vacuity (U4);

**Removed (verbatim):**
> **trybuild:** change detection on `ticks="none"`; individual group-member removal.

**Added:**
> **trybuild:** change detection on `ticks="none"` (`Ref`, `Mut`, `Added`, `Changed`, `EcsMaster::get_component_mut`); group-member insert and removal (typed, `Bundle`); inland write without `BindToken`.

**Removed (verbatim):**
> - an abstract model of the decline rule: a 2-thread deque where a nested join pops a lane task.

**Added:**
> - the lane board: lane 0 plus 2 workers. It covers publish, take, cancel-before-take, cancel racing a take, take after FINAL, and the exit-wait park handshake (`exited` vs `exit_waiting`, both SeqCst), including a lost-wake schedule that must still terminate with lane 0 alone.

**Removed (verbatim):**
> - group slot-state machine (live/dying/free, co-slot, DEAD bytes on free, `s2e`/`e2s` round-trip);

**Added:**
> - group slot-state machine (live/dying/free, co-slot, DEAD bytes on free and ≥ `slot_bound`, `dying` empty after release, `s2e`/`e2s` round-trip) under random structural ops, including clone and non-anchor migration;

**Removed (verbatim):**
> - writeback-visited count == live count minus the dying rows the chain began with;

**Added:**
> - S6, per written row: `BodyGate.flags ⊇ SIMULATED | DYNAMIC | AWAKE` and `slot < slot_bound`. Write count ≤ |{s < `slot_bound` : flags ⊇ SIMULATED | DYNAMIC | AWAKE}|. Equality is **not** a debug assert, because D12 statics, dying rows, rows that lost `RigidBody` mid-chain and mid-chain inserts all make it strict (N6). Equality is asserted only by the churn-free U4/U5 test scenes;
> - every S2 pair slot `< slot_bound`; every chain view `len() >= slot_bound`;
> - Σ rows of anchored archetypes == group `live_count` after every apply window (D13);
> - every row in the grid's `oversized` and `cell_bodies` has a live radius;

**Removed (verbatim):**
> - the release command is issued exactly once per step.

**Added:**
> - `release_dying` runs exactly once per step, and `dying` is empty after it.

---

## P-§18: open questions (N8, B4, N1)

**Removed (verbatim):**
> 4. Not verified: whether registry `LAYOUTS` consumers need scratch columns registered, and whether serde enumerates every dense type (§8 K2 serde row).

**Added:**
> 4. Not verified: whether registry `LAYOUTS` consumers need scratch columns registered. **Answered (N8):** serde does enumerate every dense type (`save.rs:285`). Group columns are excluded by construction (they are not in `dense_ids()`), so no skip flag is needed.

**Removed (verbatim):**
> 6. Not verified: whether today's AVX2 gravity/integrate lanes consult `simulated` or only `inv_mass`. Either way, U5a's oracle is the scalar kernel (`simd.rs:196-200`, `:244-249`).

**Added:**
> 6. **Answered (N8):** the AVX2 gravity and integrate lanes consult `simulated` (`simd.rs:778`, `:902`). Refresh gates on `inv_mass != 0.0` only (`simd.rs:151`). U5a's oracle is still the scalar kernel.

**Added (new items):**
> 8. Not verified: whether the `Archetype` header has a free byte for `anchor_mask`. An LSP hover is owed at U2. Growth affects structural paths only.
> 9. Not verified: whether `EcsMaster::clear()` (`ecs_master.rs:1026-1028`) resets `dense_registry` today. The group-store reset is added regardless.
> 10. Not verified: that `unpark_one_idle` ×k from lane 0 wakes k distinct workers under contention. A shortfall costs lanes only (N1 telemetry).

---

## P-Checklist: edge cases (B2, B3, B5)

**Removed (verbatim):**
>   - despawn or Collider removal mid-chain (D14);

**Added:**
>   - despawn or Collider removal mid-chain (D14), and between steps (released at the next S1 head);
>   - mid-chain spawn with an empty free list (`len` > `slot_bound`);
>   - clone of a body into a reused slot;
>   - a non-anchor component added to a body (no second group);
>   - spawn through every binder path (U4);
>   - an exclusive `release_dense_group` in a world with a chain head (refused);

---

## Responses without a section edit

- **N2: superseded by P-§10.1.** No lane is a task, so no `run_task` site needs a context. The five top-level sites (`worker.rs:85`, `:91`, `:97`, `:116`, `:128`) and six join sites (`scope.rs:1542`, `:1554`, `:1564`, `:1589`, `:1670`, `:1675`) are recorded as the reason only `worker_main`'s three probe points change.
- **N7: resolved by construction** in P-§3 D13. No API removes a group while `Collider` stays.
- **N9: recorded next to X-11 (P-§0).** It is also moot: rev 3 issues no physics command.
- **N10: ACCEPT-AS-OPEN (provenance).** I also have no shell. Owed by the orchestrator: `git diff --stat ca582e72 d11962a9 -- crates`, attached to the doc, before any rung starts. All rev-3 citations are from the working tree. The critic notes that tree carries the uncommitted `joltab` bench edits; none of the physics or ECS source lines cited here are in `benches/`.

---

## Changelog: rev 2 → rev 3

| Finding | Action | Section |
|---|---|---|
| B1 K2 misses `get_component_mut` and `compact` | FIX: both rows added. `get_component_mut<T>` const-asserts `TICKS_TRACKED` (compile). `compact` routes `move_ticks` on `is_tracked()`. Group columns become `StorageKind::Group` with no `DenseStore`, so no per-id `store_mut`/`compact`/`build_view` surface reaches them; `store_mut` release-asserts. The K2 table is pinned by a source census. U2 trybuild and debug-profile routing tests are extended (debug asserts make a missing route red) | D10, §8 K2 table, §8 K3, U2 |
| B2 dying-at-S1 ghost; completion-order release race | FIX: release moves from an S6 command to `release_dying()` at the head of S1's body (via `DenseGroupMut`), so every slot dying at S1 is DEAD before any sweep. No physics command enters any window. The determinism invariant is qualified (X-14), and a deterministic in-window apply order is raised as a scheduler item. U5 tests reshaped (between-step dead-slot, release-order determinism). U7 asserts slot equality | D14, D15 proof, §2, §6, §10.2, §10.6, §10.8, §11, §14, U5, U7 |
| B3 chain invariant false for inserts | FIX: invariant restated with `PhysicsStep.slot_bound` captured by S1. All chain sweeps and per-slot scratch are bounded by it. Debug asserts added. Mid-chain spawn test with an empty free list | §2, §5, §9, §10.2, §10.3, §10.5, §10.8, §11, U5, §17 |
| B4 anchor site set wrong | FIX: binder, the 4 EcsMaster functions that alone can write an entity's archetype pointer (`BindToken`, compile-enforced). 14 sites converted (X-13, including spawn batch and load, which were missing). `anchor_mask` XOR, so only a newly attached Collider inserts. X-10 corrected. Per-path anti-vacuity in U4. Debug reconciliation after each window | D13, §0 X-10/X-13, §16, U4, §17 |
| B5 clone copies live group bytes / double insert | FIX: group columns are not in `dense_ids()`, so the clone walk (`materialize.rs:862`) cannot see them. The clone's group comes from the binder (DEAD, SEEN = 0). Group insert release-asserts absence. Clone-into-reused-slot case in U7 | D13, D15, §8 K3, U7 |
| N1 declined lanes shrink the gang | FIX: lanes move off the pool's task structures onto a per-pool `LaneBoard` that only `worker_main`'s top level probes. Joiners cannot consume lanes. The per-gang pool scope and its two allocations are gone. Deadlock-freedom re-argued. Loom model. `taken` telemetry and a concurrent-`par_iter` G-jolt variant (recorded, not gated) | D7, §5, §9, §10.1, §11, §16, P2, §17 |
| N2 Top run-site census incomplete | SUPERSEDED by the N1 board. `RunCtx` dropped; all 11 sites cited as rationale | §0 X-12, §10.1 |
| N3 S7 ∥ S4 cost rule wrong | FIX: S7 `.after(S4)` dispatches in S5's round, cost max(0, S7 − S5). S6 no longer after S7. Still 6 rounds. Timing measures S7 vs S5 | §6, D6, §12 timing |
| N4 grid geometry pre-pass and oversized legs | FIX: live-radius predicate on the geometry fold, median, count and fill; `n` = live count; oversized legs inherit the skip, with a debug assert. X-9 corrected | §0 X-9, §7, §10.3, §10.8, §17 |
| N5 U5a/U5b addressing incoherent | FIX: U5a stays kernel-only (ports exercised by oracle tests, wired at U5). U5b folded into U5. Row → slot addressing switches in U5 alone. Bit-identity only where gather order = slot order; tolerance elsewhere | §12 U5a, U5b (deleted), U5 |
| N6 writeback debug_assert wrong | FIX: per-row assert plus a `≤` bound. Equality only in churn-free test scenes | §17 |
| N7 direct group removal | RESOLVED by construction: no API removes a group; typed and id paths refuse group columns | D13 |
| N8 serde / AVX2 gates answered | ACCEPT: open questions 4 and 6 closed with the critic's citations. The serde skip flag is not needed (not in `dense_ids()`) | §8 K2 serde row, §18 |
| N9 release window shared with gameplay | ACCEPT: recorded at X-11. Moot under D14 rev 3 (no physics command) | §0 X-11/X-14 |
| N10 git diff not run | ACCEPT-AS-OPEN: owed by the orchestrator; I have no shell either | Provenance note |

**Sections whose invariants this patch depends on (critic re-read list):**
- D13 depends on X-13, and on the `BindToken` visibility claim (no external callers of the three writers; grep in this round).
- D14 depends on `schedule.rs:669`, `:781-832` and the §5 projection rule.
- §10.1 depends on `worker.rs:84-130`, `:383-387`, `:418-451` and `scope.rs:1540-1678`.
- §10.3 and §10.8 depend on `resources.rs:1023-1084`, `:1253`, `:1304`, `:1340-1369`, `:1526`.
- The K2 table depends on `mut_.rs:441-442`, `filter.rs:1204`, `:1492`, `component_api.rs:657-675`, `dense_store.rs:607-640` and `dense_registry.rs:112-150`.

**Primary files read this round:**
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/dense/dense_registry.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/dense/views.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/clone/materialize.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/clone/prefab.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/serialize/load_writer.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/change_detection/check_ticks.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/iters/query/data/ref_.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/iters/query/data/mut_.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`
- `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/entity/entity_master.rs`
- `D:/wt/joltab/crates/boyko_threadpool/src/worker.rs`
- `D:/wt/joltab/crates/boyko_threadpool/src/scope.rs`
- `D:/wt/joltab/crates/boyko_physics/src/resources.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs`
- `D:/wt/joltab/crates/boyko_serialize/src/save.rs`

The design document is `D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md`.

# Critique log - pass 3 (2026-09-11)

Pass 3 was scoped to the rev-3 patch. The critic's output is reproduced below verbatim: the verdict, every blocking finding, every non-blocking finding, and the fixes it confirmed. The architect's response is the Rev 4 patch that follows this section.

## Verdict

> CHANGES REQUESTED (pass 3, scoped to the rev-3 patch). All five pass-2 blockers are answered correctly in mechanism: B1's routing table is complete for plain dense stores, B2's ghost and its dependence on completion-pop order are gone, B3's slot_bound invariant holds under mid-chain insert and remove, B4's binder covers every archetype-pointer write in the tree, and B5's clone path is closed by construction. All ten N items are answered; N10 is closed by the writer's git diff note. The patch does add three new blocking defects: (NB1) the LaneBoard exit signal is a use-after-free, since a taker reads gang memory after the RMW that lets lane 0 free it; (NB2) the claim that making StorageKind matches exhaustive forces every site to route group columns is false for the query layer, which dispatches on the compile-time T::STORAGE_IS_DENSE, so `Query<&BodyGate>` silently returns zero rows; (NB3) the N4 grid fix makes the median input empty whenever every slot below slot_bound is dead, and select_nth_unstable_by panics on an empty slice. There are also 8 non-blocking items. Evidence is from D:/wt/joltab; no cargo was run and nothing was timed.

## Blocking findings

- NB1 - LaneBoard exit wait: use-after-free of GangShared (new mechanism, P-§10.1 / P-§5). WHERE: Take step 5, design :1771 "`gang.exited.fetch_add(1, SeqCst)`. Then, if `gang.exit_waiting.load(SeqCst)`, unpark lane 0. After step 5 the taker touches neither the ticket nor the gang."; Cancel step 2, :1776 "Wait until `exited == taken`: spin; then `exit_waiting.store(true, SeqCst)`; re-check `exited` with SeqCst; then `park`."; §5 :1529 GangShared "lives in lane 0's frame until lane 0's exit wait returns". PROBLEM: the taker's `fetch_add` is the operation that releases lane 0. Lane 0's spin, or its SeqCst re-check, can see `exited == taken` immediately after that RMW, run `in_use.fetch_and` and return, which pops the frame holding GangShared. The taker then runs `gang.exit_waiting.load` and, if it reads true, unparks a `Thread` taken from `gang.threads` (:1542); both reads are of a dead stack frame. The Dekker SeqCst argument proves that no wake is lost. It says nothing about lifetime. Step 5 contradicts its own sentence 'after step 5 the taker touches neither ... the gang', because the load and the unpark are part of step 5. The patch also never says that lane 0 writes `threads[0]` before `exit_waiting.store(true)`, so a lane 0 that never parked in a phase wait leaves that cell uninitialised. CONSEQUENCE: at W>=2, whenever the last taken lane returns while lane 0 is still in its exit spin (the common case, since phases end together), the taker reads freed stack memory. A garbage `true` feeds a garbage `Thread` to unpark: a crash or a silent memory smash in S5/S3/S2, reachable from safe `par_phases`. The planned loom model (§17 :2084) sees it only if GangShared sits in a loom-tracked allocation that is freed when lane 0 returns. A static placement makes that model vacuous. CONFIDENCE: CONFIRMED (the protocol text alone establishes it). NEEDED: the taker's last access to gang-owned memory must be the single RMW that can release lane 0. Anything the taker needs after it (the waiting flag, the wake target) must be folded into that RMW's word or read beforehand, or must live in pool-lifetime storage (the board, or the pool's per-worker parker). State where lane 0's wake handle lives. Make the loom model free GangShared when lane 0 returns.
- NB2 - Group columns under the query layer's compile-time dispatch: silent wrong answers, and the claimed compile gate misses them (new mechanism, P-§3 D13 / P-§8 K3 / U2). WHERE: :1392 "Every `StorageKind` dispatch becomes an exhaustive `match` ... Adding `Group` is then a compile error at each site until it is routed"; :1665 "Query terms over group columns (`&T`, `&mut T`, `GroupSlot`) match archetypes by `anchor_mask`"; U2 :1975 "the workspace builds only with every `StorageKind` match routed". PROBLEM: `&T`, `&mut T`, `Mut<T>`, `With`, `Without`, `Added`, `Changed`, `dense_iter` and chunked data do not dispatch on the runtime StorageKind. They dispatch on the const `T::STORAGE_IS_DENSE`: read.rs:116 `if const { T::STORAGE_IS_DENSE } {`, :161-164 `fetch.dense = match registry.store(state.id) { ... None => std::ptr::null(),`, :248-250 `if fetch.dense.is_null() { return false; }`; filter.rs:715 `const IS_ARCHETYPAL: bool = !C::STORAGE_IS_DENSE;`, :826 `return !unsafe { fetch.contains_row(row) };`; similar sites in write.rs:90-273, mut_.rs:220-417, dense_iter.rs:130-141 and query.rs:457. None of these is a StorageKind match, so adding `Group` produces no compile error at any of them. The patch never says what a group column's `STORAGE_IS_DENSE` is. Rev 2 §5 :333 declares the columns `storage="dense"`, and that derive emits `const STORAGE_IS_DENSE: bool = true;` (boyko_macros component.rs:575). Since `DenseRegistry::store` returns None for an id with no DenseStore (dense_registry.rs:136-140), every such term silently falls through. With the flag false instead, the table arm is taken and `mask.contains(id)` is false on every archetype: the same silence. CONSEQUENCE: the gameplay reads the design itself advertises compile and give wrong results with every gate green. D4 :157 "gameplay reads sleep through `&BodyGate`" (owner decision Q3) and §9's `BodyPose::position()`: `Query<(Entity, &BodyGate)>` returns 0 rows on the pyramid instead of 1241. `With<BodyGate>` matches nothing, and `Without<BodyGate>` passes every body. Only S1/S5/S6/S7 are safe, because they use the new params. This is the storage-kind-blindness class the repo already records (the `Without` flag-filter polarity and OR-filter dense cases). CONFIDENCE: CONFIRMED (code paths traced). NEEDED: define the compile-time kind of a group column. Put the const-dispatch family in the same completeness census as the runtime match, and route or compile-refuse each term, including the self-Bundle every `#[derive(Component)]` emits (self_bundle.rs:56), which is not the Bundle derive. Add behavioural gates: a `Query<&BodyGate>` row count equal to `live_count`, and `Without<BodyGate>` excluding every body.
- NB3 - Grid N4 fix: empty median input panics when every slot is dead (new defect in P-§10.3 / P-§10.8). WHERE: :1863 "the geometry fold and median input (`resources.rs:1023-1051`). `n` becomes the number of pushed radii"; :1856 and :1862 both loops over `0..step.slot_bound`. CODE: the only empty guard keys on the input length, resources.rs:1008 `if bodies.is_empty() {`, which becomes `slot_bound == 0` over slots. Then :1044 `let mid = self.scratch_radii.len() / 2;` and :1049 `.select_nth_unstable_by(mid, ...` run. std documents that select_nth_unstable_by panics when index >= len, so it always panics on an empty slice. With the live-radius skip, `scratch_radii` is empty whenever slot_bound > 0 and no slot below it has a live radius. CONSEQUENCE: every body despawned between steps (a level unload), then any further step with Grid selected, gives a release-build panic in S2. Grid is user-selectable (broadphase_policy.rs:185-186, Manual mode 'never touches broadphase'). Auto can also stay on Grid permanently, because the natural port of today's count (:182 `let count = scratch.bodies_len() as u32;`) is the slot high-water, which never shrinks. In rev 2 the median input was non-empty (NaN entries), so the panic is new with this fix. No planned test has an all-dead grid. CONFIDENCE: CONFIRMED (plan text, resources.rs and the std contract); which count Auto uses is PLAUSIBLE. NEEDED: key the degenerate-grid early return on the live-radius count, not on slot_bound, and add an all-dead-slots Grid step to U5 or §17.

## Non-blocking findings

- W1 - `EcsMaster::clear()` versus D14 and the chain invariant (new claim, D13 :1371 and §16 :2037 'clear() resets the group stores'). Open question 9 answered: clear() does NOT reset `dense_registry` today. ecs_master.rs:1026-1028 is `self.entity_master.clear(); self.archetype_master.clear();` plus the two bundle caches, so plain dense stores keep memberships for entities that no longer exist, a pre-existing bug. The patch leaves 'reset' undefined. If reset frees slots at once: an exclusive system unordered with physics can land in a round between S1 and S6, reuse slots below slot_bound for new entities, and violate §2 (a) "owner changes only by live → dying". S7 then maps S2's pair slots to new entities (W3's wrong-entity event again), and `fresh_step` becomes load-bearing, contrary to the D15 restatement :1510 'not load-bearing'. Consequence: level-reload code calling clear() mid-frame produces wrong-entity contact events for one step. CONFIDENCE: CONFIRMED that it is undefined; PLAUSIBLE for the scheduling. Direction: define reset as remove-all-to-dying (deferred, released at the next S1), or refuse it while a chain is in flight.
- W2 - Release authority is wrong in both directions (new API, P-§9 :1725 and :1728). (a) `pub fn release_dying(&mut self)` is on a public SystemParam. Any user system holding `DenseGroupMut<PhysicsBody>`, which is the only group-wide write view, and running between S1 and S5 can DEAD-fill slots that S2's pairs name. That brings back C2's NaN-pose narrowphase, and mid-chain reuse brings back W3. (b) The exclusive release is refused once a `DenseGroupMut` has EVER been initialised (:1483). So an editor session with physics paused and spawn/despawn churn can never release, the high-water grows without bound, and every sweep after resume covers it. CONFIDENCE: CONFIRMED (API text). Direction: gate release on 'no chain in flight' (set at S1, cleared at S6) rather than 'head ever initialised', and restrict `release_dying` to the registered chain head.
- W3 - When the anchor binding and the per-world group store come into existence (D13 :1364 'computed once when the archetype is created'; 'registration asserts'). The mask is correct only if group g's anchor binding exists before every archetype that contains Collider. If the binding is minted at plugin or group registration rather than inside Collider's own component registration, archetypes created earlier (colliders spawned before the plugin builds, or a scene loaded first) get bit 0 forever. Their bodies are never simulated, and the debug reconciliation (:1398) passes, because it keys on the same mask: green from emptiness. The patch also never says whether the transition may create the per-world `DenseGroupStore` lazily, or where `groups[GroupId]` lives so that pointers cached in system State stay valid. CONFIDENCE: PLAUSIBLE. Direction: bind anchor→group inside the anchor component's registration, and state the store's creation point and address stability.
- W4 - The BindToken gate names the wrong write surface. `InlandStore` implements `DerefMut<Target=[EntityInland]>` (inland_store.rs:285-287 `fn deref_mut(&mut self) -> &mut [EntityInland]`), and the field is `pub(crate) entities_inland: InlandStore` (entity_master.rs:55). The four migration sites write through exactly that path: migration_helpers.rs:977-978 `world.entity_master.entities_inland[entity.id().0] = EntityInland::new(...)`. A token cannot gate DerefMut. Unless DerefMut is removed and the field made private, the patch's 'cannot compile without going through the binder' (:1375) is false. CONFIDENCE: CONFIRMED. Direction: name the removal of DerefMut and the field privacy explicitly, and add the trybuild fixture on the index-assignment form.
- W5 - U7's warm-start assertions cannot fail. Under the patch's own D15 proof (:1502-1508, i ≤ j−2, and colored.rs:3037-3046 rebuilds `warm_write` every step), step-i entries are gone before Y's first lookup. So 'no inherited warm impulse or axis' in the reuse and clone cases stays green with the fresh_step skip deleted. Only the slot-equality and live_count assertions are real gates. CONFIDENCE: CONFIRMED. Direction: label those assertions as pins, and either add a case that makes the skip load-bearing (W1's immediate reset, or an Immediate group) or a mutation run that deletes the skip and must go red.
- W6 - LaneBoard details that the loom model and implementer need. (a) The `in_use` CAS ordering is unspecified. The new tenant's Relaxed slot writes must be ordered after the previous takers' reads, so the claim needs Acquire to pair with the `fetch_and(!mine, Release)` at :1777. (b) A ticket taken at the post-mark_idle probe must `unmark_idle` before running the lane, mirroring worker.rs:127. (c) Take step 4 is a lane-level `catch_unwind`, but the Panic rule says 'the lane counts its chunk done'. A lane-level catch does not know its in-flight chunk. If lane 0 or a taken lane unwinds mid-chunk without a per-chunk guard, the other lanes wait forever on that phase, and lane 0's exit wait waits on them: a hang. CONFIDENCE: PLAUSIBLE for (c). Direction: state a per-chunk unwind guard in `for_each`/`single` for every lane, including lane 0.
- W7 - Open question: tickets take priority over the injector (the probe sits before `pop_global_injector`, worker.rs:84-93), and a taken lane stays resident until FINAL. If S5's lane 0 publishes W−1 tickets before an idle worker has popped S7 from the injector, S7 waits for the whole gang. Its cost is then S7, not N3's max(0, S7 − S5). The rev-2 D7 trade-off ('resident lanes are unavailable to other systems') already implies this, but the N3 cost rule does not account for it. Direction: have the §12 S7-vs-S5 timing record whether S7 actually overlapped, and state the intended priority between tickets and system tasks.
- O1 - PHYSICS-ECS-UNIFICATION-DECISIONS.md:89 still records 'RunCtx trampoline vs TLS-depth fallback: Trampoline'. Rev 3 deletes RunCtx (P-§14 :2017), so that row now selects a mechanism that no longer exists. Mark it superseded so a later reader does not implement it.

## Fixes the critic confirmed

- B1 RESOLVED for plain dense. The full safe-reachable tick-consumer set is routed: Mut dense fetch (mut_.rs:441-442, const-assert); Ref (no dense arm, ref_.rs); Added (filter.rs:1204) and Changed (:1492) dense arms; get_component_mut dense arm (component_api.rs:673-675, fn-entry const-assert); DenseStore::insert (dense_store.rs:257-259), insert_with_ctor (:371-373), insert_or_replace (:418, also reached from set_component_raw at component_api.rs:487), stamp_slot_ticks (:758-765), compact (:640), check_ticks (check_ticks.rs:229, :236). The wrapped forms are covered because Option/AnyOf/tuples/Or delegate init_state (option.rs:85-86, anyof.rs:124-125, filter.rs:1586-1587, :1898-1899). Clone reaches ticks only through DenseStore::insert. get_component_changed_tick and any_changed_since are table-only (component_api.rs:369-379, :404-415). The serde save writes no ticks. No hook or observer API hands out a Mut (component_api.rs:635 is the only get_component_mut). chunked_data.rs:209/:370 const-refuse dense. The store_mut release assert (dense_registry.rs:113-120 is a debug_assert today) closes the per-id route to group ids (B1b).
- B2 RESOLVED. Release is a serial step at the head of S1's body, so every window before S1's round precedes it and every later window follows it (the gate at schedule.rs:669; commands are applied only in apply_window_drain, :781-832). A slot dying at S1 is DEAD before S2 reads anything. Release position and LIFO reuse no longer depend on completion-pop order. The remaining X-14 dependence, the order among gameplay commands inside one window, is correctly recorded as pre-existing, and the U5 release-order test would have gone red on rev 2 (S6's release command and the gameplay system's commands in one window).
- B3 RESOLVED (except clear(), W1). slot_bound = len after release. A mid-chain insert either pops a slot freed at this S1 (below slot_bound, DEAD) or appends at or above slot_bound. A mid-chain remove keeps the bytes and sets s2e to TOMBSTONE, and the slot stays not free until the next S1. So §2 (a)-(c), the §10.5 bounds and S7's s2e reads all hold.
- B4 RESOLVED in mechanism (W3/W4 caveats). X-13 checked complete by grep: register_entity_with_ptr at entity_api.rs:230/:425/:563, spawn_at_command.rs:352, materialize.rs:554, prefab.rs:786; register_batch at spawn_batch_command.rs:767 and load_writer.rs:569; EntityInland::new at migration_helpers.rs:977/:1378/:1767/:2088; deallocate_entity at entity_api.rs:221/:1088/:1099; clear at entity_master.rs:461. The mask XOR keys on a newly attached Collider, and required components ride the mask. Unbind at :1088/:1099 still sees the old archetype (entity_api.rs:1083-1099). The per-path U4 anti-vacuity list covers every spawn route.
- B5 RESOLVED. The clone walk materialize.rs:862 `for &cid in world.dense_registry.dense_ids()` cannot see group ids. bind_new at :554 inserts DEAD with SEEN=0, and group insert release-asserts absence. The D15 restated proof checks out: i ≤ k−1 and j ≥ k+1, and store_and_swap rebuilds warm_write every step (colored.rs:3037-3046), so no entry is carried forward.
- LaneBoard, correctness apart from NB1: C1 is closed by construction, since no join route or deque holds a lane. Publish-to-take visibility holds: Relaxed slot writes, then the SeqCst fetch_or, then the winning SeqCst fetch_and, all in one release sequence. There is no ABA on slots, because ownership is decided by an RMW on the current `pending`. The Race-C SB shape is fenced on both sides (publish_fence at worker.rs:434-436, before the post-mark_idle probe). The N1 shrink mechanism is removed. The per-gang pool scope and its two allocations are gone.
- N2 superseded (the 11 run_task sites stay unchanged). N3 fixed: S7 .after(S4) dispatches in S5's round, consistent with schedule.rs:669 and :824-832 (see W7). N4 fixed apart from NB3: the predicate is applied to the fold, median, count (resources.rs:1253) and fill (:1304), and the oversized legs consume only `oversized`/`cell_bodies` (:1344, :1558-1570). N5 fixed (U5a kernel-only, U5b folded, one addressing switch). N6 fixed (per-row assert plus a ≤ bound). N7 resolved by construction. N8 accepted. N9 moot. N10 closed by the writer's git diff note (2 bench files only).

# Rev 4 patch

## How to read rev 4

- **Everything above stays exactly as written.** Rev 2, the rev-3 patch and every critique log are unedited. This file is append-only, so the rev-4 patch is appended rather than applied in place. The current revision is **rev 4 after three critique passes**, and the file's layout is: rev 2 → rev1→rev2 changelog → Critique log - pass 1 → preserve list → Critique log - pass 2 → Rev 3 patch → Critique log - pass 3 → Rev 4 patch.
- **Reading order for any section.** Start from rev 2. Apply the rev-3 patch's `P-§…` block for that section, if one exists. Then apply the rev-4 patch's `P4-§…` block, if one exists. Each `P4-§…` block quotes verbatim the rev-2 or rev-3 text it removes, and gives the text that replaces it. **Rev 2 and the rev-3 patch stand except where rev 4 names a section.** A section rev 4 does not name reads exactly as it did after rev 3.
- The patch's closing table ("Changelog: rev 3 → rev 4") is the rev 3 → rev 4 changelog.
- **The O1 writer note is not applied in this round.** The patch asks for `PHYSICS-ECS-UNIFICATION-DECISIONS.md:89` to be replaced. That file was outside this writer's scope and is unchanged; its row still reads "RunCtx trampoline vs TLS-depth fallback (gang nesting rule) | **Trampoline**". The replacement row text is in the patch's writer notes below and is owed by the orchestrator.

# Rev 4 patch: PHYSICS-ECS-UNIFICATION-DESIGN.md (rev 3 → rev 4)

**Provenance.** Every new citation below was re-read in `D:/wt/joltab` during this round. No cargo was run and nothing was timed. I am read-only, so this patch is text for the writer to append.

**Writer notes.**
- Append this patch after "Changelog: rev 2 → rev 3".
- Add "Critique log - pass 3" (the orchestrator's pass-3 JSON, verbatim) before it.
- Append the closing table as "Changelog: rev 3 → rev 4".
- **Apply O1 to `PHYSICS-ECS-UNIFICATION-DECISIONS.md:89`.** Replace the row with: `| ~~RunCtx trampoline vs TLS-depth fallback (gang nesting rule)~~ | **Superseded by design rev 3** (P-§10.1): lanes are tickets on a per-pool LaneBoard that only worker_main's top level probes. No run_task site carries a run context, so neither mechanism is built | The TLS measurement (docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md) is still the reason the board adds no per-task TLS read |`

**Three new mechanisms, and the class each one removes.**

| Finding | Class | Mechanism that removes it | Replaces |
|---|---|---|---|
| NB2 | A new storage kind must be routed at every const site and every runtime site | Group columns are **not `Component`s**. Every `Component`-generic term and API refuses them with E0277. | Rev 3's claim that exhaustive `StorageKind` matches close the query layer |
| NB1 | Gang lifetime after the taker's releasing RMW | The exit wait **never parks**, so the taker's last access is the RMW itself | `exit_waiting` and the Dekker park |
| W1, W2 | Release authority | Chain state `chain_open` (opened by S1, closed by S6). A removal defers if the chain is open and releases at once if it is closed. | `release_dying` on a public param; the exclusive release refused "once ever initialised" |

Order of blocks: §0, §2, §3 (D4, D6, D7, D10, D13, D14, D15), §5, §6, §8, §9, §10.1, §10.2, §10.3, §10.6, §10.7, §10.8, §11, §12, §14, §16, §17, §18, Checklist, responses, table.

---

## P4-§0: corrections table (NB1, NB2, W1, W4)

**Removed:** none.

**Added (four rows after X-14):**
> | X-15 | (pass 3, NB2) Query terms dispatch on compile-time `Component` consts, not on the runtime `StorageKind` | Every leaf is `T: Component`-bounded: `read.rs:81` `unsafe impl<T: Component> QueryData for &T {`, `write.rs:78`, `mut_.rs:208`, `ref_.rs:118`, `data_is_enabled.rs:140`, `filter.rs:529` (`With`), `:709` (`Without`), `:986` (`Added`), `:1313` (`Changed`), `filter_enable.rs:151`, `:334`. Each branches on `if const { T::STORAGE_IS_DENSE }` (`read.rs:116`; `filter.rs:715` `const IS_ARCHETYPAL: bool = !C::STORAGE_IS_DENSE;`) | A storage kind reached through `Component` must be routed at every const site **and** every runtime site. No compiler check links the two families. Rev 4 takes group columns out of `Component` entirely (D13), so none of these impls can be instantiated for them. |
> | X-16 | (pass 3, NB1) The pool has already solved "the completer's last touch of a joiner-owned allocation" | `scope.rs:823-826` "Copied out BEFORE the decrement: after it the joiner may free this allocation, so `*shared` must not be touched again."; `:848-855` (the RMW's `&self` is all-`UnsafeCell`, so its protector is weak); `:909-917` (an external joiner is unparked BEFORE the decrement); gate `boyko_threadpool/tests/miri_scope_completion_protector.rs` | The gang exit has the same shape. Rev 4 takes the stronger form: nothing at all follows the RMW (§10.1). |
> | X-17 | (pass 3, W1) `EcsMaster::clear()` does not reset `dense_registry` | `ecs_master.rs:1026-1028` `pub fn clear(&mut self) {` `self.entity_master.clear();` `self.archetype_master.clear();` | Pre-existing bug: plain dense stores keep memberships of dead entities. Fixed in U2, independently of groups. |
> | X-18 | (pass 3, W4) The inland store's archetype-pointer write surface is `DerefMut` | `inland_store.rs:285-287` `impl DerefMut for InlandStore {` … `fn deref_mut(&mut self) -> &mut [EntityInland] {`; `entity_master.rs:55` `pub(crate) entities_inland: InlandStore,`; writes at `migration_helpers.rs:977` `world.entity_master.entities_inland[entity.id().0] =` and `:963` `…entities_inland.get_mut(moved_entity.0) {`. The only other `&mut self` methods are `ensure` (`inland_store.rs:162`) and `clear` (`:227`) | Basis of the rev-4 binder gate (D13) |

Depends on: D13, §10.1.

---

## P4-§2: invariants (W1, W2)

**Removed:** none. Inserted after the rev-3 bullet "Rev 2's "no slot changes owner or bytes" was false under (c) and for free slots under (b)."

**Added:**
>   - (d) **Outside the chain** (`chain_open == false`, D14 rev 4), a removed slot is released at once and may get a new tenant in the same window. The new tenant is marked fresh at its first S1. Every persistent slot-keyed state applies the fresh rule (D15): `PairCache` (S3/S5) and `ContactEventState` (S7). §17 lists them as a census.

Depends on: D14, D15, D6.

---

## P4-§3 D4 (NB2)

**Removed (rev 2, verbatim):**
> **Trade-off:** gameplay reads sleep through `&BodyGate`.

**Added:**
> **Trade-off:** gameplay reads sleep in two ways: through the `GroupRef<BodyGate>` query term (or `EcsMaster::group_get::<BodyGate>(e)`), and through the sleep/wake transition events. That is Q3's decision (the location and the events) and is unchanged. `&BodyGate`, `With<BodyGate>` and every other `Component`-generic form do not compile, because group columns are not `Component`s (D13 rev 4). A `GroupRef` row costs one `e2s` lookup plus one column load. This is gameplay-side only; the physics hot loops stay slot-addressed.

---

## P4-§3 D6 (W1/W2 consequence)

**Removed (rev 3, verbatim):**
> - S7 is ordered `.after(S4)` and is unordered with S5 and S6 (§6). It needs no ordering against S6: a slot it reads through `s2e` cannot be reused before the next S1 (D14).

**Added:**
> - S7 is ordered `.after(S4)` and is unordered with S5. **S6 is `.after(S7)` again.**
>   - S6 closes the chain (D14 rev 4). After that, a removal releases its slot at once, so every chain reader of `s2e` must already have finished.
>   - The edge costs no round. S7 is dispatched in S5's round, and S6 cannot dispatch before that round's window anyway (X-11).
> - **The merge applies the fresh rule (rev 4).**
>   - A previous-step record that names a slot with `BodyMaterial.fresh_step == step` is ended (End event, with the stored entities), never continued.
>   - A current pair that names a fresh slot is a Start.
>   - Without this rule, an out-of-chain despawn of X plus a spawn of Y into the same slot, touching the same partner Z, would suppress both End(X,Z) and Start(Y,Z).

Depends on: D14 rev 4, D15 case (B), §10.7.

---

## P4-§3 D7 (W7)

**Removed (rev 3, verbatim):**
> - An untaken ticket is lost parallelism, never lost correctness: lane 0 alone can complete every phase, and at FINAL it cancels untaken tickets. Tickets go untaken only when no worker is at top level: every worker is busy or in a join.

**Added:**
> - An untaken ticket is lost parallelism, never lost correctness: lane 0 alone can complete every phase, and at FINAL it cancels untaken tickets. Tickets go untaken only while every worker finds a task first or is inside a join.
> - **Priority (rev 4, W7): a ticket is the lowest-priority work.** `worker_main` probes the board only after its own deque, the injector and the sibling steal have all come up empty (§10.1).
>   - Why: a gang is elastic on entry, because a late lane joins the current phase ("Late lane"). It is inelastic on exit, because a taken lane stays until FINAL. A system task is inelastic.
>   - Tickets first: the task waits for the whole gang. With S7 in S5's round and every worker a lane, the round costs about S5 + S7.
>   - Tasks first: one lane joins late by S7, spread over S5's remaining phases. The round costs about max(S5 + S7/W, S7) (arith.).
>   - Both orders keep every core busy. Tasks-first has the shorter critical path and never starves an unrelated system, which serves the owner's throughput priority.
>   - This also makes N3's cost rule, max(0, S7 − S5), hold as rev 3 stated it.

---

## P4-§3 D10 (NB2 consequence)

**Removed (rev 3, verbatim):**
> - Group columns (K3) are not `DenseStore`s, so no row of the table can receive a group column id.

**Added:**
> - Group columns (K3) are neither `DenseStore`s nor `Component`s (D13 rev 4), so no row of the table can receive one.
> - **K2 is no longer a prerequisite of K3.** A group's column pools are untracked by construction. They are private and never handed out, the `scratch/scratch_column.rs:67-68` structure ("The pool is a private field and is never handed out, so no caller can reach a tick either"). They reserve tick VA and never commit it (`:73-74`).
> - K2 stays specified exactly as rev 3 wrote it, including the table B1 confirmed. It lands with its first plain-dense consumer. No physics rung waits for it, and C3/B1's hazard cannot reach a group column by construction.

---

## P4-§3 D13 (NB2, W3, W4, W1)

**Removed (rev 3, verbatim):**
> - **Anchor mask.** Every `Archetype` carries `anchor_mask: u8`, computed once when the archetype is created. Bit g is set iff the archetype's component set contains group g's anchor. At most 8 anchored groups per process; registration asserts this.

**Added:**
> - **Anchor binding (rev 4, W3).** The anchor → group binding is made inside the anchor's own id mint.
>   - `#[component(anchor_group = PhysicsBody)]` makes the derive emit `install_anchor::<Self, PhysicsBody>(raw)` inside `Collider::component_id()`'s `get_or_init`. It sits beside the existing install calls: `boyko_macros/src/component.rs:374-392`, "install this type's derive hooks into `HOOKS[raw]` atomically with ID assignment, before the component can appear in any archetype".
>   - `install_anchor` registers the group, minting its column ids as one contiguous block (K1). It writes the process-global, write-once `ANCHOR_GROUP[raw] = g`. It asserts at most 8 anchored groups per process.
>   - No archetype can contain `Collider` before `Collider::component_id()` has returned. So the binding exists before every such archetype, on every thread: the `OnceLock` read that yields the id acquires the write.
>   - **No nested `OnceLock` in the reverse order.** The group's registration is initialised only from inside `install_anchor`, and it receives `raw`; it never calls `Anchor::component_id()`. `G::group_id()` first forces `G::Anchor::component_id()`, then reads the group registration. So the order is always anchor → group.
> - **Anchor mask.** Every `Archetype` carries `anchor_mask: u8`, computed once at creation from `ANCHOR_GROUP`. Bit g is set iff the component set contains group g's anchor.
> - **Per-world store (W3).** `DenseRegistry` holds `groups: [Option<DenseGroupStore>; 8]` inline, with no heap.
>   - `ensure_group(g)` creates a store in place and reserves VM for each column (`VmReservation`, no commit).
>   - It is called by `anchor_transition`'s first insert and by every group param's `init_state`. Both hold `&mut EcsMaster`.
>   - No system `State` caches a pointer into the store. A State holds the `GroupId` and column ids and resolves the store once per run through the world cell. Column bases never move, so views taken in a run stay valid for that run.

**Removed (rev 3, verbatim):**
>   Together they replace the 14 sites of X-13. `EcsMaster::clear` resets every group store.

**Added:**
>   Together they replace the 14 sites of X-13. `EcsMaster::clear` goes through the binder's clear route, which applies D14 rev 4's clear rule to every group store.

**Removed (rev 3, verbatim):**
> - **Compile-time completeness.** The inland store's archetype-changing writers (today `register_entity_with_ptr`, `register_batch`, `deallocate_entity`, and `get_mut`/`IndexMut` handing out `&mut EntityInland`) require a `&BindToken`. Only `ecs_master::binder` can construct that token (private constructor).

**Added:**
> - **Compile-time completeness (rev 4, W4).** The real write surface is `impl DerefMut for InlandStore` (X-18). Rev 4:
>   - **deletes `impl DerefMut for InlandStore`**. `Deref` (`inland_store.rs:260`) stays, so the read sites (`[i]`, `.get(i)`, about 130 across 25 files) compile unchanged;
>   - adds `InlandStore::set(&mut self, _: &BindToken, id, EntityInland)`. It is the only method that writes an archetype pointer;
>   - adds a tokenless `fix_unit_index(&mut self, id, row)`, which writes the row only;
>   - keeps `ensure` (`:162`), which writes null sentinels past the end;
>   - reaches `clear` (`:227`) only through the binder's clear;
>   - makes `register_entity_with_ptr`, `register_batch` and `deallocate_entity` take `&BindToken` as well. Only `ecs_master::binder` can construct the token (private constructor).
>
>   With `DerefMut` gone, no `&mut [EntityInland]` or `&mut EntityInland` can be formed outside `InlandStore`'s own methods. So the assignment form (`migration_helpers.rs:977`) and the `get_mut` form (`:963`) fail to compile in any file, including in a future structural path. The field stays `pub(crate)`. The one remaining bypass is replacing the whole store (`entities_inland = …`); a source census catches it, and also pins that neither `DerefMut` nor `IndexMut` is re-added (§17). Trybuild cannot express this, because the surface is crate-private; the gate is the compiler plus the census.

**Removed (rev 3, verbatim):**
>   - for each bit in `old & !new`: group remove (deferred for `PhysicsBody`, D14).

**Added:**
>   - for each bit in `old & !new`: group remove (deferred iff `chain_open`, else released at once; D14 rev 4).

**Removed (rev 3, verbatim):**
> - **Group columns are not dense stores.** Their ids are a new `StorageKind::Group`, never `Dense`.

**Added:**
> - **Group columns are neither dense stores nor `Component`s (rev 4, NB2).**
>   - Their ids are a new `StorageKind::Group`, minted by the group registration, never by `Component::component_id()`.
>   - The column types implement only `GroupColumn` (§9), which `#[dense_group]` emits. They carry no `#[derive(Component)]`, so they also get no self-`Bundle`: that impl is emitted per `Component` (`bundle/self_bundle.rs:56`), and `Bundle` is sealed (`:54`).

**Removed (rev 3, verbatim):**
> - **Every `StorageKind` dispatch becomes an exhaustive `match`.** There are 57 occurrences in 19 files today, mostly `== StorageKind::Dense`. Adding `Group` is then a compile error at each site until it is routed (for example the direct read APIs at `component_api.rs:201`, `:274`) or refused (every id-based structural entry point; the loader returns `LoadWriteError` instead of panicking).

**Added:**
> - **Two dispatch families, each closed by its own mechanism (rev 4, NB2).**
>   - **Compile-time family.** This covers every query leaf; `With`, `Without`, `Added`, `Changed`, `IsEnabled`, `Enabled`, `Disabled`; `dense_iter`; chunked data; `get_component*`; `insert`/`remove`; and `Bundle`. All of them are `T: Component`-bounded and branch on `T::STORAGE_IS_DENSE` (X-15). A group column is not a `Component`, so none of them can be instantiated for it. The result is E0277 at the use site, with no per-site routing and nothing for a future term to forget. Rev 3 claimed the exhaustive runtime match closed this family. It closed none of it (NB2).
>   - **Runtime family** (id-based paths: id insert/remove, the loader, registry and reflection walks). Every `StorageKind` dispatch becomes an exhaustive `match`: 57 occurrences in 19 files today, mostly `== StorageKind::Dense`. Each is either routed (the direct read APIs at `component_api.rs:201`, `:274`) or refused (every id-based structural entry point; the loader returns `LoadWriteError`). A source census forbids `== StorageKind::`, `!= StorageKind::` and `matches!(…StorageKind::…)` outside `component_registry`. An unconverted comparison silently takes the table arm, and the census makes it red (§17).
>   - **Exclusivity.** A type cannot be both a `Component` and a `GroupColumn`. `#[dense_group]` evaluates an autoref probe per concrete column type at group registration (the derive's technique, `component/component.rs:144-150`). If the type implements `Component`, registration panics (cold, once per process, independent of mint order). Otherwise `Query<&BodyGate>` would read an empty `Component` store, which is the same silent zero NB2 describes.

**Removed (rev 3, verbatim):**
>   - `insert::<T>`, `remove::<T>`, `migrate_entity_remove::<C>` and the `Bundle` derive const-assert `!T::IS_GROUP_COLUMN`.

**Added:**
>   - `insert::<T>`, `remove::<T>`, `migrate_entity_remove::<C>` and every `Bundle` require `Component`, which a group column does not implement (E0277). `IS_GROUP_COLUMN` is deleted.

**Removed (rev 3, verbatim):**
> - **Debug reconciliation (mechanical backstop).** In debug builds, at the end of every `apply_window_drain` (`schedule.rs:841`) and after every direct `EcsMaster` structural call, the kernel checks: for each anchored group g, Σ rows of archetypes with bit g == `live_count(g)`. The cost is O(#archetypes), debug only. A missed path is red in any test that exercises it.

**Added:**
> - **Debug reconciliation (mechanical backstop).** In debug builds, at the end of every `apply_window_drain` (`schedule.rs:841`) and after every direct `EcsMaster` structural call, the kernel checks two things for each anchored group g:
>   - Σ rows of archetypes whose `component_ids` contain g's anchor id == `live_count(g)`. The anchor id is read from the component set, **not** from the cached mask;
>   - every archetype's cached `anchor_mask` equals the mask recomputed from its component set.
>
>   Keying on the cached mask would pass for a mask computed before the binding existed (W3, green from emptiness). The cost is O(#archetypes), debug only.

**Added (Alternatives, appended):**
> - (rev 4) Routing the compile-time family: a `STORAGE_IS_GROUP` const, a group arm at every `if const` site, and a census over them. Rejected. It needs about 60 const sites, and every future term, to cooperate. That is the storage-kind-blindness class this repo has already paid for (`tests/ab11_flag_filter_polarity.rs`; the OR-filter dense case). Taking group columns out of `Component` removes the class rather than patrolling it.

**Added (Trade-off, appended):**
> - Gameplay and tooling read group columns through `GroupRef<T>` / `group_get`, not `&T` / `get_component`. A generic inspector that walks `Component`s does not show them until it handles `StorageKind::Group`, and the exhaustive runtime match forces that decision at its site.

Depends on: X-15, X-18, §8 K3, §9, §12 U2, §17.

---

## P4-§3 D14 (W1, W2, W5)

**Removed (rev 3, verbatim):**
> - **Remove** (only through the binder, D13) clears `live`, sets `s2e = TOMBSTONE`, drops the `e2s` entry and **keeps the bytes**. The slot enters `dying`, not `free`. Group columns are `Copy`, so there is no drop.
> - **Release is a step inside the chain head's system body, not a command.** S1 holds `DenseGroupMut<PhysicsBody>` (§8 K3/K6). Serially, before its two `par_iter` loops, S1 calls `release_dying()`. That call:
>   - writes each column's `const DEAD: Self` into every dying slot;
>   - appends those slots to `free` in dying order, so LIFO reuse is deterministic;
>   - clears `dying`;
>   - returns `slot_bound = len()`, which S1 stores in `PhysicsStep.slot_bound` (B3).

**Added:**
> - **Chain state.** The group's recycle node carries `chain_open: bool`. Two unique params write it (one live claim per world each, §8 K3):
>   - `GroupHead<G>::open_chain()` runs in S1, serially, before its loops. It writes each column's `const DEAD` into every dying slot, appends those slots to `free` in dying order (deterministic LIFO), clears `dying`, sets `chain_open = true`, and returns `slot_bound = len()`. S1 stores that in `PhysicsStep.slot_bound` (B3).
>   - `GroupTail<G>::close_chain()` runs in S6, serially, as its last step. It sets `chain_open = false`. S6 is `.after(S7)`, so no chain reader is still running (§6).
> - **Remove** (only through the binder, D13) clears `live`, sets `s2e = TOMBSTONE` and drops the `e2s` entry. Then:
>   - if `chain_open`: the slot enters `dying` and **keeps its bytes**. S3, S5 and S7 may still read it through S2's pairs (C2, W3);
>   - if `!chain_open`: the slot is **released at once**, DEAD-filled and pushed to `free` (W1, W2b).
>
>   Group columns are `Copy`, so there is no drop.
> - **The first out-of-chain op flushes `dying`.** A group insert, remove or clear with `!chain_open` first releases every slot still in `dying`, which were removed mid-chain in the last step. So `dying` is non-empty only between a mid-chain removal and the next S1 head or out-of-chain op, whichever comes first.
> - **`EcsMaster::clear()` (W1).** For each group store:
>   - `!chain_open`: reset. `len = 0`, and `free`, `dying`, `live` and `e2s` are emptied. No fill pass is needed, because appends DEAD-fill;
>   - `chain_open` (an exclusive system unordered with physics landed inside the chain): every live slot goes to `dying` with its bytes, and `len` is kept. So §2 (a)–(c) hold, and the next S1 releases them.
>
>   New tenants after either form are covered by D15 case (B). The plain-dense part of `clear()` is X-17's fix (U2).
> - **Release authority (W2a).** Only `open_chain`, out-of-chain removal, the first-op flush and `clear` release. `release_dying`, `DenseGroupMut` and `EcsMaster::release_dense_group` are deleted. A user system cannot obtain `GroupHead<PhysicsBody>` while physics holds the claim.

**Removed (rev 3, verbatim, the slot-state table):**
> | Removed | Released | Simulated in step i? | Simulated in step i+1? |
> |---|---|---|---|
> | before S1(i) | head of S1(i) | no | no |
> | in a mid-chain window, after S1(i) and before S6(i) completes | head of S1(i+1) | **yes**, with the bytes S1(i) wrote. S2(i)'s pairs may name it. It is not written back, because its row is gone | no |
> | between steps, after S6(i) and before S1(i+1) (the **B2 case**) | head of S1(i+1), before any sweep | yes (it was live for all of step i) | **no**: DEAD before S2(i+1) reads anything |

**Added:**
> | Removed | Released | Simulated in step i? | Simulated in step i+1? |
> |---|---|---|---|
> | before S1(i) opens (outside the chain) | at removal | no | no. The slot may get a new tenant at once, which is fresh at its first S1 (D15 case B) |
> | mid-chain: after S1(i) opens, before S6(i) closes | head of S1(i+1), or the first out-of-chain op after S6(i) | **yes**, with S1(i)'s bytes; S2(i)'s pairs may name it; not written back | no |
> | between steps, after S6(i) closes (the **B2 case**) | at removal | yes (live for all of step i) | **no**: DEAD before S1(i+1) |

**Removed (rev 3, verbatim):**
> - **Access.** `DenseGroupMut<G>` declares a write on every column id of G and on G's **recycle node**. The recycle node is a registry id distinct from the slot-map node that `GroupSlot`/`DenseSlots` read (§8 K3). Release touches only column bytes and the recycle lists; `e2s`/`s2e`/`live` were already updated at removal. A reader of `GroupSlot` or `DenseSlots` running in S1's round therefore reads nothing that release writes. Field projection rule: §5.

**Added:**
> - **Access.**
>   - `GroupHead<G>` declares a write on every column id of G and on G's recycle node (`dying`, `free`, `chain_open`), plus a read on the slot-map node (for `live_count`).
>   - `GroupTail<G>` declares a write on the recycle node only.
>   - The recycle node is a registry id distinct from the slot-map node that `GroupSlot`, `GroupRef` and `DenseSlots` read (§8 K3). `open_chain` touches column bytes and the recycle node; `close_chain` touches `chain_open`.
>   - `e2s`/`s2e`/`live` change only in exclusive contexts, which are apply windows or `&mut EcsMaster`. Out-of-chain releases run only there.
>   - Field projection rule: §5.

**Removed (rev 3, verbatim):**
> - A slot becomes reusable only after the next S1. So a despawn followed by a spawn within one step never reuses the slot, and `free` never holds more than one step's removals beyond steady state.
> - A world that anchors the group but runs no chain calls `EcsMaster::release_dense_group::<G>()`. That call panics (cold) if any `DenseGroupMut<G>` has ever been initialised in the world, because an exclusive release between S1 and S5 would DEAD-fill slots that S2's pairs still name. The flag `chain_head_registered` is set in `DenseGroupMut::init_state`. The debug diagnostic "dying count > live count" stays.

**Added:**
> - A slot removed mid-chain becomes reusable only after the chain closes. A slot removed outside the chain is reusable at once. So between-step churn (a despawn plus a spawn in Update, or a paused editor session) keeps the high-water at the peak live count instead of growing it (W2b).
> - A world with no chain never opens one, so every removal releases at once: plain LIFO reuse, with no API to call.
> - A chain that stops after S1 (a panic mid-chain) leaves `chain_open` set. Removals then defer until the next S1, which releases them. `open_chain` finding `chain_open` already set raises a debug **diagnostic, not an assert**, because panic recovery reaches that state legitimately. The "dying count > live count" diagnostic stays.
> - The price of reuse between two consecutive steps is that D15's fresh rule becomes load-bearing, in `PairCache` and in S7's merge (D6).

**Added (Why, appended):**
> - **W1/W2 (rev 4).**
>   - Rev 3 released only at S1's head. A world whose chain did not run therefore never released.
>   - Its exclusive fallback was refused once a head had *ever* been initialised, so a paused editor grew without bound.
>   - Its release sat on a public param any user system could hold.
>   - Keying on `chain_open` gives every structural path one precise rule: defer inside [open, close], release outside it.

Depends on: §2 (a)–(d), §6 (S6 `.after(S7)`), D15, D6, §10.2, §10.6.

---

## P4-§3 D15 (W5; consequence of D14 rev 4)

**Removed (rev 3, verbatim):**
> 3. Y fills s first at S1(j). s became free only at the head of some S1(k), and X's bytes left s there. So X held s at S5(i) only for i ≤ k−1. Y was inserted in a window after S1(k), so j ≥ k+1, and therefore i ≤ j−2.

**Added:**
> 3. Y fills s first at S1(j). s was released in one of two ways (D14 rev 4):
>    - (A) at the head of some S1(k). X held s at S5(i) only for i ≤ k−1, and Y was inserted after S1(k), so j ≥ k+1 and i ≤ j−2;
>    - (B) out of chain, at X's removal in a window between S6(i') and S1(i'+1), or by a flush or `clear` there. X was last simulated at i ≤ i', and Y's first S1 is at j ≥ i'+1. So i ≤ j−1, and **i = j−1 is reachable**: a despawn and a spawn between the same two steps.

**Removed (rev 3, verbatim):**
> 5. i+1 ≤ j−1 < j, so step i's entries are gone before Y's first lookup.
>
> The `fresh_step` skip is therefore not load-bearing under rev 3's release point. It is kept for two reasons. It costs one compare on a line the lookup already loads (Cost, below). And it keeps `PairCache` safe for any future group with `RELEASE = Immediate`. The debug belt "no `PairCache` hit with a fresh body" stays.

**Added:**
> 5. If i ≤ j−2, step i's entries are gone before Y's first lookup (step 4). If i = j−1, which happens in case (B) only, Y is fresh at step j. Every lookup naming s is skipped, and step j's store holds only Y's entries.
>
> **The `fresh_step` skip is load-bearing in case (B).** Rev 3's "not load-bearing" held only while S1's head was the sole release point. S7 applies the same rule to `ContactEventState` (D6 rev 4). U7's out-of-chain reuse case is therefore a real gate: it goes red when either skip is deleted (§17 mutation runs). The debug belt "no `PairCache` hit with a fresh body" stays.

(The rev-3 sentence "Clones (B5) are covered by the same proof…" stays.)

---

## P4-§5: data structures (NB1, NB2, W2, W3, W6a)

**Removed (rev 2, verbatim, in the group header comment):**
> // All columns: storage="dense", group=PhysicsBody, ticks="none" (K2), Copy, `const DEAD` (D14).

**Added:**
> // All columns: declared by #[dense_group(PhysicsBody)], Copy, `const DEAD` (D14). NOT Components (D13 rev 4):
> // no storage attribute, no ticks — untracked private pools by construction (D10 rev 4).

**Removed (rev 3, verbatim, the `GangShared` struct):**
```rust
#[repr(C)]
struct GangShared {                      // K5b; lives in lane 0's frame until lane 0's exit wait returns
    claim: CachePadded<AtomicU64>,       // (phase:32 | next_chunk:32), CAS-claimed
    done: [CachePadded<AtomicU32>; 2],   // completion counter per phase parity
    completed: CachePadded<AtomicU32>,   // last fully completed phase (FINAL = u32::MAX)
    parked: CachePadded<AtomicU64>,      // bit per lane parked after spin budget (lanes ≤ 64)
    exited: CachePadded<AtomicU32>,      // taken tickets whose lane has returned (SeqCst RMW)
    exit_waiting: AtomicBool,            // lane 0 parked in the exit wait (SeqCst; W7 Dekker pair)
    phase_n: AtomicU32,                  // debug SPMD agreement check
    poisoned: AtomicBool,
    panic: UnsafeCell<Option<Box<dyn Any + Send>>>, // written once by the `poisoned` CAS winner.
                                         // Box only on the panic path (resume_unwind needs it)
    entry: unsafe fn(*const (), &GangLane<'_>),     // monomorphised by par_phases; 1 call per lane per gang
    program: *const (),                  // &F in lane 0's frame
    threads: [UnsafeCell<MaybeUninit<Thread>>; 64], // unchanged from rev 2
}
```

**Added:**
```rust
#[repr(C)]
struct GangShared {                      // K5b; lives in lane 0's frame until lane 0's exit wait returns
    claim: CachePadded<AtomicU64>,       // (phase:32 | next_chunk:32), CAS-claimed
    done: [CachePadded<AtomicU32>; 2],   // completion counter per phase parity
    completed: CachePadded<AtomicU32>,   // last fully completed phase (FINAL = u32::MAX)
    parked: CachePadded<AtomicU64>,      // bit per lane parked in a PHASE wait (lanes ≤ 64)
    exit_count: CachePadded<AtomicU32>,  // taken lanes that returned. The taker's Release fetch_add is its
                                         // LAST access to this struct; lane 0 spins on it (own line)
    threads_written: AtomicU64,          // bit l: threads[l] initialised; lane 0 drops exactly these
    phase_n: AtomicU32,                  // debug SPMD agreement check
    poisoned: AtomicBool,                // SeqCst; ends every claim and every wait (panic protocol)
    payload_claimed: AtomicBool,         // CAS-once owner of `panic`
    panic: UnsafeCell<Option<Box<dyn Any + Send>>>, // written by the payload_claimed winner; panic path only
    entry: unsafe fn(*const (), &GangLane<'_>),     // monomorphised by par_phases; 1 call per lane per gang
    program: *const (),                  // &F in lane 0's frame
    threads: [UnsafeCell<MaybeUninit<Thread>>; 64], // PHASE-wait wake targets only; read only by a
                                         // participant that has not yet done its exit RMW
}
// `exited` and `exit_waiting` are deleted: the exit wait never parks, so nothing wakes lane 0 (NB1).
```
(`LaneBoard` and `LaneTicket` are unchanged.)

**Removed (rev 3, verbatim, the `DenseGroupStore` block):**
```rust
// K3 — one per registered DenseGroup, in DenseRegistry::groups[GroupId]. Not in `slots` / `dense_ids`.
pub(crate) struct DenseGroupStore {
    // slot-map node (its own registry id): read by GroupSlot / DenseSlots; written only in apply windows
    e2s: EntitySlotMap,
    s2e: VmColumn<EntityId>,             // TOMBSTONE for dying/free
    live: LiveBitmap,
    // recycle node (its own registry id): written by DenseGroupMut (the chain head) and in apply windows
    recycle: UnsafeCell<Recycle>,
    // one untracked ComponentPool per column id (K1 contiguous ids, K2 no tick pages); DEAD-initialised
    columns: [ComponentPool; MAX_GROUP_WIDTH],
    width: u8,
    anchor: ComponentId,                 // Collider for PhysicsBody
    chain_head_registered: bool,         // set by DenseGroupMut::init_state; refuses EcsMaster release (D14)
}
struct Recycle { dying: VmColumn<u32>, free: VmColumn<u32> }  // VM-backed, no heap
```

**Added:**
```rust
// K3 — one per anchored group per world, inline in DenseRegistry::groups: [Option<_>; 8] (no heap),
// created in place by ensure_group (D13 rev 4). Not in `slots` / `dense_ids`.
pub(crate) struct DenseGroupStore {
    // slot-map node (own registry id): read by GroupSlot / GroupRef / DenseSlots / GroupHead;
    // written only in exclusive contexts (apply windows, &mut EcsMaster)
    e2s: EntitySlotMap,
    s2e: VmColumn<EntityId>,             // TOMBSTONE for dying/free
    live: LiveBitmap,
    live_count: u32,
    // recycle node (own registry id): written by GroupHead (S1), GroupTail (S6), exclusive contexts
    recycle: UnsafeCell<Recycle>,
    // one untracked ComponentPool per column (K1 contiguous ids); private, never handed out
    // (scratch_column.rs:67-68 structure) ⇒ no tick accessor reachable; DEAD-initialised on append
    columns: [ComponentPool; MAX_GROUP_WIDTH],
    width: u8,
    anchor: ComponentId,                 // Collider for PhysicsBody (from install_anchor's `raw`)
    head_claimed: bool,                  // one live GroupHead<G> State per world; cleared by the State's Drop
    tail_claimed: bool,                  // one live GroupTail<G> State per world
}
struct Recycle { dying: VmColumn<u32>, free: VmColumn<u32>, chain_open: bool }  // VM-backed, no heap
```

**Removed (rev 3, verbatim):**
> - `GangShared` outlives every lane that took a ticket. Lane 0 cancels untaken tickets and returns only after `exited == taken` (§10.1). On unwind a drop guard runs the same cancel-and-wait.
> - A `LaneTicket` slot is released (`in_use` bit cleared) only after that wait. A taker reads its ticket before it runs and never touches it after `exited.fetch_add`.
> - `Thread` handles are dropped by lane 0 after the exit wait.

**Added:**
> - `GangShared` outlives every access any taker makes to it. Lane 0 cancels untaken tickets and returns only after `exit_count == taken`. Each taker's `exit_count.fetch_add` is its last access to the gang, the ticket and anything the gang owns. On unwind, a drop guard runs the same cancel-and-wait (§10.1).
> - A `LaneTicket` slot is re-tenanted only after lane 0's `in_use.fetch_and(!mine, Release)`, which follows its exit wait. The next tenant claims the slot with an Acquire CAS (W6a).
> - Lane 0 drops the `Thread` cells named in `threads_written` after the exit wait. Every lane's `fetch_or` into that mask precedes its Release exit RMW, and lane 0 Acquire-reads the final `exit_count`.

Depends on: §10.1, D13, D14.

---

## P4-§6: system graph (W2, W1)

**Removed (rev 3, S1 row fragment, verbatim):**
> writes `DenseGroupMut<PhysicsBody>` (all five column ids plus the recycle node; runs `release_dying()` first, D14);

**Added:**
> writes `GroupHead<PhysicsBody>` (all five column ids plus the recycle node; runs `open_chain()` first, D14 rev 4);

**Removed (rev 3, S6 row fragment, verbatim):**
> `DenseColumn<BodyVel/BodyPose/BodyGate>`, `Res<PhysicsStep>`

**Added:**
> `DenseColumn<BodyVel/BodyPose/BodyGate>`, `Res<PhysicsStep>`, `GroupTail<PhysicsBody>` (recycle-node write; `close_chain()` last)

**Removed (rev 2, S7 row access cell fragment, verbatim):**
> `DenseSlots<PhysicsBody>` (s2e, read),

**Added:**
> `DenseSlots<PhysicsBody>` (s2e, read), `DenseColumn<BodyMaterial>` (`fresh_step`, read; D6 rev 4), `Res<PhysicsStep>`,

**Removed (rev 3, verbatim):**
> - S1 → S2 → S3 → S4 → S5 → S6. S7 is `.after(S4)` and unordered with S5 and S6.
> - S6 needs no edge to S7. There is no release command any more, and a slot S7 reads through `s2e` cannot be reused before the next S1 (D14).

**Added:**
> - S1 → S2 → S3 → S4 → S5 → S6. S7 is `.after(S4)` and unordered with S5. S6 is `.after(S5).after(S7)`.
> - **The S7 → S6 edge is restored (rev 4).** S6 closes the chain. Once it is closed, a removal releases its slot at once, so S7's `s2e` reads must have finished. It costs zero rounds: S7 dispatches in S5's round, and S6 cannot dispatch before that round's window (X-11).

**Removed (rev 3, verbatim):**
> - S1's only group write param is `DenseGroupMut<PhysicsBody>`: one write per column id plus one on the recycle node. `GroupSlot` reads the slot-map node. The three node kinds are distinct registry ids (§8 K3).

**Added:**
> - S1's only group write param is `GroupHead<PhysicsBody>`: one write per column id plus one on the recycle node, and a read on the slot-map node. `GroupSlot` reads the slot-map node. The three node kinds are distinct registry ids (§8 K3).
> - S6's `GroupTail` writes the recycle node, and its `GroupSlot` reads the slot-map node. The two are disjoint.

(Rounds: unchanged, **6**.)

---

## P4-§8: kernel features (NB2, W2, W7, NB1)

**Removed (rev 2, K2 row, verbatim):**
> | K2 | Untracked dense, compile-time sound | See the table below | All group columns | render dense instance data whose ticks are unread (not verified) | — |

**Added:**
> | K2 | Untracked dense, compile-time sound | The rev-3 table, unchanged | **None** (rev 4: group columns are untracked by construction, not through K2) | render dense instance data whose ticks are unread (not verified) | — ; lands with its first consumer |

**Removed (rev 2, K3 row prerequisite cell, verbatim):**
> K1, K2

**Added:**
> K1

**Removed (rev 3, K5b row, verbatim):**
> | K5b | Gang (`par_phases`) + lane ticket board | §10.1: lanes live on a per-pool board that only `worker_main`'s top level probes | S2 grid, S3, S5, soft solve | transform propagation, animation hierarchy, render multi-pass culling | KE16 (closed) |

**Added:**
> | K5b | Gang (`par_phases`) + lane ticket board | §10.1: lanes live on a per-pool board. `worker_main`'s top level probes it, and only as its lowest-priority work (W7). The exit wait never parks (NB1) | S2 grid, S3, S5, soft solve | transform propagation, animation hierarchy, render multi-pass culling | KE16 (closed) |

**Removed (rev 3, K6 row, verbatim):**
> | K6 | Group release | `DenseGroupMut<G>::release_dying()`, a step in the chain head's body (D14). `EcsMaster::release_dense_group::<G>()` exists for chain-less worlds and is refused once a chain head is registered | S1 | any deferred-release group | K3 |

**Added:**
> | K6 | Group chain state and release | `GroupHead<G>::open_chain()` in the head's body (release dying, open). `GroupTail<G>::close_chain()` in the tail's body. A removal defers iff `chain_open`, otherwise it releases at once. No release API (D14 rev 4) | S1, S6 | any deferred-release group | K3 |

**Removed (rev 3, verbatim):**
> - Column ids are `StorageKind::Group`. There is no `DenseStore`, they are not in `dense_ids()`, and they never appear in an archetype's `component_ids` (D13).

**Added:**
> - Column ids are `StorageKind::Group`. There is no `DenseStore`. They are not in `dense_ids()`, and they never appear in an archetype's `component_ids`. **The column types do not implement `Component`** (D13 rev 4).

**Removed (rev 3, verbatim):**
> - `insert::<T>`/`remove::<T>`/`Bundle` const-assert `!IS_GROUP_COLUMN`. Id-based structural entry points refuse `Group` ids. Trybuild fixtures cover insert and remove.

**Added:**
> - Every `Component`-generic API refuses a group column through its trait bound (E0277): `insert`, `remove`, `Bundle` (sealed, and there is no self-bundle), `get_component*`, and every query leaf and filter (X-15). Id-based entry points refuse `Group` ids through the exhaustive runtime `match`. There is one trybuild fixture per family (§12 U2).

**Removed (rev 3, verbatim):**
> - Query terms over group columns (`&T`, `&mut T`, `GroupSlot`) match archetypes by `anchor_mask`. The binder guarantees every row of an anchored archetype has a slot, so per-row membership is a `debug_assert!`, not the per-row `e2s.contains` filter that plain dense needs.
> - `DenseGroupMut<G>` param: writes every column id and the recycle node; `release_dying()`; typed views.

**Added:**
> - **Group-aware query terms:** `GroupSlot<G>` (the slot) and `GroupRef<T>` (a read of column T at the row's slot).
>   - Both are archetypal terms on the **anchor**. `matches_component_set` is `mask.contains(G::ANCHOR)`, and `aggregate_include` sets the anchor bit. That is the path every `&Collider` term already takes, so neither term contains a storage-kind branch. `HAS_DENSE = false`.
>   - The binder guarantees that every row of such an archetype has a slot, so per-row membership is a `debug_assert!`.
>   - Access: `GroupSlot` reads the slot-map node. `GroupRef<T>` reads the slot-map node and column id T.
> - `GroupHead<G>` param (one live claim per world): writes every column id and the recycle node, and reads the slot-map node. It provides `open_chain()`, `live_count()` and typed views.
> - `GroupTail<G>` param (one live claim per world): writes the recycle node. It provides `close_chain()`.
> - A claim is taken in `init_state` and released by the State's Drop. A second live claim panics (cold, at schedule build).

**Removed (rev 3, verbatim):**
> - `dying` is empty immediately after `release_dying()`;

**Added:**
> - s ∈ dying ⟹ `chain_open` was true when s was removed. `dying` is empty right after `open_chain()` and right after the first out-of-chain group op;

---

## P4-§9: public API (NB2, W2)

**Removed (rev 2, verbatim):**
> pub trait DenseGroup: 'static { const WIDTH: usize; const RELEASE: GroupRelease; }
> pub trait GroupColumn: Component + Copy { type Group: DenseGroup; const DEAD: Self; }

**Added:**
```rust
pub trait DenseGroup: 'static { const WIDTH: usize; const RELEASE: GroupRelease; type Anchor: Component; }
pub unsafe trait GroupColumn: Copy + Send + Sync + 'static {   // NOT Component (D13 rev 4); emitted by
    type Group: DenseGroup;                                     // #[dense_group]; hand impls are unsafe
    const DEAD: Self;                                           // (co-slotting and id-block invariants)
    fn column_id() -> ComponentId;                              // StorageKind::Group id
}
pub struct GroupRef<T: GroupColumn>;   // ReadOnlyQueryData, Item = &T; archetypal on T::Group::Anchor
impl EcsMaster { pub fn group_get<T: GroupColumn>(&self, e: Entity) -> Option<&T>; } // cold tooling read
```

**Removed (rev 3, verbatim):**
```rust
pub struct DenseGroupMut<'w, G: DenseGroup>;   // SystemParam: write on every column id of G + G's recycle node
impl<G: DenseGroup> DenseGroupMut<'_, G> {
    pub fn release_dying(&mut self) -> u32;    // K6: DEAD-fill + dying→free; returns slot_bound (= len afterwards)
    pub fn view<T: GroupColumn<Group = G>>(&self) -> TypedDenseView<'_, T>;
}
impl EcsMaster { pub fn release_dense_group<G: DenseGroup>(&mut self); } // K6; panics if a DenseGroupMut<G> was initialised
// trait Component { const TICKS_TRACKED: bool = true; const IS_GROUP_COLUMN: bool = false; ... }
// get_component_mut<T> const-asserts TICKS_TRACKED; insert/remove/Bundle const-assert !IS_GROUP_COLUMN
// PhysicsStep { ..., pub slot_bound: u32 }
```

**Added:**
```rust
pub struct GroupHead<'w, G: DenseGroup>;       // SystemParam; one live claim per world (W2a)
impl<G: DenseGroup> GroupHead<'_, G> {
    pub fn open_chain(&mut self) -> u32;       // K6: flush dying (DEAD + free), chain_open = true; returns slot_bound
    pub fn live_count(&self) -> u32;
    pub fn view<T: GroupColumn<Group = G>>(&self) -> TypedDenseView<'_, T>;
}
pub struct GroupTail<'w, G: DenseGroup>;       // SystemParam; one live claim per world
impl<G: DenseGroup> GroupTail<'_, G> { pub fn close_chain(&mut self); }
// deleted: DenseGroupMut, EcsMaster::release_dense_group, Component::IS_GROUP_COLUMN
// K2 (decoupled): trait Component { const TICKS_TRACKED: bool = true; ... } and the get_component_mut
//   const-assert land with K2's rung, not with physics
// PhysicsStep { ..., pub slot_bound: u32 }
```

---

## P4-§10.1: gang protocol (NB1, W6, W7)

**Removed (rev 3, verbatim):**
> - **Lanes are not pool tasks.** Lane 0 publishes tickets on the pool's `LaneBoard` (§5). A ticket is taken only at `worker_main`'s three top-level probe points:
>   - after the own-deque pop (`worker.rs:84-87`), before the injector;
>   - in the pre-`mark_idle` re-poll (`:115-118`);
>   - in the post-`mark_idle` re-poll (`:126-130`).

**Added:**
> - **Lanes are not pool tasks.** Lane 0 publishes tickets on the pool's `LaneBoard` (§5). A ticket is taken only at `worker_main`'s three top-level probe points, and each probe comes **after every task source has come up empty** (W7, D7):
>   - after the sibling steal (`worker.rs:96-99`), so after the own deque (`:84`) and the injector (`:90`);
>   - after the pre-`mark_idle` `pop_any` (`:115`), whose order is own deque → injector → steal (`:227-233`);
>   - after the post-`mark_idle` `pop_any` (`:126`).

**Removed (rev 3, verbatim):**
> - **Publish (lane 0).** For each lane l in 1..k: CAS a free bit in `in_use`, then write `(gang, l)` into that slot with Relaxed.

**Added:**
> - **Publish (lane 0).** For each lane l in 1..k, claim a free bit with `in_use.compare_exchange_weak(cur, cur | bit, Acquire, Relaxed)`, then write `(gang, l)` into that slot with Relaxed (W6a).
>   - The Acquire pairs with the previous tenant's `in_use.fetch_and(!mine, Release)`.
>   - That release follows the previous tenant's exit wait. The exit wait Acquire-read every taker's Release `exit_count` RMW, and each RMW follows that taker's read of its ticket.
>   - So the new tenant's slot writes happen-after every earlier read of the slot.

**Removed (rev 3, verbatim):**
> - **Take (worker_main only).**
>   1. `p = pending.load(Relaxed)`. If zero, fall through: one load of a read-mostly line.
>   2. Otherwise, for the lowest set bit b: `old = pending.fetch_and(!(1<<b), SeqCst)`. If `old` had b, this worker owns ticket b.
>   3. Load `(gang, l)` with Acquire.
>   4. Run the lane under `catch_unwind` as a **resident** lane.
>   5. `gang.exited.fetch_add(1, SeqCst)`. Then, if `gang.exit_waiting.load(SeqCst)`, unpark lane 0.
>
>   After step 5 the taker touches neither the ticket nor the gang. The post-`mark_idle` probe is preceded by `publish_fence()`. That fence is the worker half of the Race-C pair (`worker.rs:418-429`): `mark_idle` itself is only `Release` (`worker.rs:386`).
> - **Cancel and exit wait (lane 0, after FINAL, and in a drop guard on unwind).**
>   1. `old = pending.fetch_and(!mine, SeqCst)`, then `taken = popcount(mine & !old)`.
>   2. Wait until `exited == taken`: spin; then `exit_waiting.store(true, SeqCst)`; re-check `exited` with SeqCst; then `park`. Both sides of this Dekker pair are SeqCst, the same argument as the W7 handshake.
>   3. `in_use.fetch_and(!mine, Release)`, then return.

**Added:**
> - **Take (worker_main only).**
>   1. `p = pending.load(Relaxed)`. If zero, fall through: one load of a read-mostly line.
>   2. Otherwise, for the lowest set bit b: `old = pending.fetch_and(!(1<<b), SeqCst)`. If `old` had b, this worker owns ticket b.
>   3. If this is the post-`mark_idle` probe: `unmark_idle(&inner.idle, worker_id)` before anything else, as `worker.rs:127` does for a task (W6b).
>   4. Load `(gang, l)` with Acquire. The taker holds `gang: *const GangShared` **by value**. Every `&GangShared` or `&GangLane` it forms lives inside the lane frame of step 5 and has ended before step 6.
>   5. Run the lane under `catch_unwind` as a **resident** lane. Chunks run under the per-chunk guard (Panic, below).
>   6. **Exit, the taker's last access to gang memory:** `unsafe { (*gang).exit_count.fetch_add(1, Release) }`. Nothing follows it that touches the gang, the ticket, or anything the gang owns. There is no wake to issue, because lane 0 does not park in the exit wait.
>      - The receiver `&AtomicU32` covers only `UnsafeCell` bytes, so its protector is weak. The `CachePadded` deref frame, whose protector is strong, returns before the RMW commits. This is the `scope.rs:848-855` argument (X-16).
>
>   The post-`mark_idle` probe is preceded by `publish_fence()`, the worker half of the Race-C pair (`worker.rs:418-429`); `mark_idle` itself is only `Release` (`worker.rs:386`).
> - **Cancel and exit wait (lane 0, after FINAL, and in a drop guard on unwind).**
>   1. `old = pending.fetch_and(!mine, SeqCst)`, then `taken = popcount(mine & !old)`.
>   2. Spin with `pause` while `exit_count.load(Acquire) != taken`, up to the exit spin budget. After that, loop on `yield_now()`. **Lane 0 never parks here.**
>   3. `in_use.fetch_and(!mine, Release)`.
>   4. Drop the `Thread` cells named in `threads_written`, then return (or `resume_unwind`, see Panic).
> - **Why the exit wait never parks (NB1).**
>   - (i) **It removes the class.** No wake target exists, so a taker has nothing to read after the RMW that releases lane 0. The `exit_waiting` flag, the Dekker pair and the `threads[0]` initialisation question all go away.
>   - (ii) **It is faster on the system's critical path.** At the exit wait every phase is complete. Each taken lane is either spinning in a phase wait, where it sees completion within about 100 ns, or parked in one and already unparked by that phase's completer (W7). A parked lane 0 would add its own OS wake after the last taker's RMW. Spinning returns the instant that RMW lands.
>   - (iii) The price is lane 0's core for at most one OS wake latency of the slowest taken lane, per gang. There are about 4 gangs per step (S2 grid, S3, S5, soft). This is not measured; the P2 receipt records exit-wait time per gang (§18 Q11).
>   - **Rejected alternative:** the KE16 W-d′ count-gated park. The taker would read a `PoolInner`-owned target before the RMW and unpark on `prev == 1` (`scope.rs:541-560`). But that target exists only when lane 0 is a registered worker of this pool. A dispatcher or external lane 0 has none (`scope.rs:551-556`, "recorded for KE17"). It would need the second route, unpark-before-decrement, with its `park_timeout` backstop (`scope.rs:793-799`). That is two routes plus a timed backstop, for a wait whose expected length is one wake latency.
> - **Gates** (§17): a Miri gang-exit protector test, which is the lifetime gate, and the loom board model, which checks ordering and termination. Loom cannot see a use-after-free.

**Removed (rev 3, verbatim):**
> **Panic.**
> - A panicking lane wins or loses the `poisoned` CAS; the winner stores the payload. The lane counts its chunk done and exits, still counting `exited`.
> - Other lanes skip the remaining phases.
> - Lane 0 re-raises with `resume_unwind` after the exit wait.
> - A panic in lane 0 runs cancel and exit wait in its drop guard before unwinding out of the frame that owns `GangShared`.
> - A lane panic never reaches `worker_main`'s abort path (`worker.rs:178-182`), because the take path catches it.

**Added:**
> **Panic (rev 4, W6c).**
> - **A per-chunk guard on every lane, lane 0 included.** `for_each` and `single` arm a guard before each chunk body and disarm it after. It costs nothing on the normal path; its Drop is `#[cold] #[inline(never)]` and cannot panic. On unwind, the guard's Drop:
>   - `poisoned.store(true, SeqCst)`;
>   - runs the Complete step for its own chunk, so the phase count stays exact;
>   - `bits = parked.swap(0, SeqCst)` and unparks each lane in `bits`. The guard's thread is a participant that has not done its exit RMW, so `GangShared` is alive.
> - Every wait loop exits on `completed >= p || poisoned`. The SeqCst store-then-swap on the poisoner's side and the fetch_or-then-load on the waiter's side extend the W7 argument, so no wake is lost. `for_each` and `single` return at once when `poisoned` is set.
> - **Taken lane:** the unwind reaches the take path's `catch_unwind`. The lane CASes `payload_claimed` (false → true, AcqRel). If it wins, it stores the payload. Then it runs Take step 6.
> - **Lane 0:** its unwind reaches `par_phases`' drop guard. The guard publishes FINAL (`completed.store(FINAL, SeqCst)`, then swap and unpark), cancels, runs the exit wait, drops any stored payload, and continues unwinding with lane 0's own payload.
>   - If lane 0 did not panic but finds `poisoned` set after the exit wait, it calls `resume_unwind` with the stored payload.
> - Without the per-chunk guard, a lane that unwound mid-chunk would leave its phase one short forever, and lane 0's exit wait would wait on lanes stuck in that phase: a hang (W6c).
> - A lane panic never reaches `worker_main`'s abort path (`worker.rs:178-182`).

**Removed (rev 3, verbatim):**
> - Close: one `fetch_and`, the exit wait, one `fetch_and`.

**Added:**
> - Close: one `fetch_and`, the exit wait (spin then yield, no park), one `fetch_and`. Each taker does one Release RMW.

Depends on: X-16, §5, §11, §17.

---

## P4-§10.2 S1 (W2, NB3)

**Removed (rev 3, verbatim):**
> **Release first (D14, K6).** Serially, before either loop: `step.slot_bound = group.release_dying()`. Every slot dying when S1 starts receives each column's DEAD and moves to `free`. Cost: O(dying × 168 B). After this line no slot `< slot_bound` is dying.

**Added:**
> **Open the chain first (D14 rev 4, K6).** Serially, before either loop: `step.slot_bound = head.open_chain()`.
> - Every slot still dying receives each column's DEAD and moves to `free`, and `chain_open` becomes true. Those slots were removed mid-chain in the previous step; out-of-chain removals were already released.
> - Cost: O(dying × 168 B). After this line no slot `< slot_bound` is dying.
> - **The broadphase selection** (the former `select_broadphase`) reads `head.live_count()` here. It does not read `slot_bound`, and it does not port `broadphase_policy.rs:182` `let count = scratch.bodies_len() as u32;` as a slot count. The high-water never shrinks, so a slot count would pin Auto on Grid (NB3).

---

## P4-§10.3 broadphase (NB3)

**Removed (rev 3, verbatim):**
> - the geometry fold and median input (`resources.rs:1023-1051`). `n` becomes the number of pushed radii, not `bodies.len()` at `:1084`;

**Added:**
> - the geometry fold and median input (`resources.rs:1023-1051`). `n` becomes `n_live`, the number of pushed radii, not `bodies.len()` at `:1084`.
>   - **The degenerate-grid return keys on `n_live == 0`, measured after the fold (NB3).** Today's guards are `resources.rs:1008` `if bodies.is_empty() {` and `build`'s `:1173-1174` `let n = bodies.len();` / `if n == 0 {`. Ported to slots, they become `slot_bound == 0`. That is false when every slot below `slot_bound` is dead, and then `select_nth_unstable_by(mid, …)` (`:1049`) runs on an empty slice, where it always panics.
>   - So the fold runs first in `build_csr`. If it pushed nothing, `build_csr` sets the `1 × 1 × 1` geometry and clears `oversized`, and `build` / `build_parallel` return an empty pair set before the median.
>   - The `slot_bound == 0` test stays only as a fast path.
>   - `debug_assert!(mid < scratch_radii.len())` precedes the select;

---

## P4-§10.6 writeback (W2)

**Removed (rev 3, verbatim):**
> S6 issues no command. Release happens at the head of the next S1 (D14).

**Added:**
> S6 issues no command. Its last serial step is `tail.close_chain()` (D14 rev 4). From then on a removal releases its slot at once. Slots removed mid-chain stay dying until the next S1 head or the first out-of-chain op.

---

## P4-§10.7 events (W1/W2 consequence)

**Removed:** none (inserted after the rev-2 bullet "Merge-walk this step's sorted valid pair keys against `ContactEventState`'s previous keys and stored `Entity` handles.").

**Added:**
> - **Fresh rule (D6 rev 4).** A previous record, or a current pair, that names a slot with `BodyMaterial.fresh_step == step` never matches across steps. The previous record ends (End with its stored handles) and the current pair starts. Cost: two `fresh_step` loads per merged pair, from a 32 B-stride column of about 40 KB at pyramid scale (arith.; L1/L2-resident).

---

## P4-§10.8 (NB3, W2)

**Removed (rev 3, verbatim):**
> | S1 release | dying list | writes them | — | DEAD before any sweep |

**Added:**
> | Release | S1 `open_chain`: the dying list. Out of chain: the removed slot, the flush, `clear` | writes them | — | DEAD before any sweep of the next step |

**Removed (rev 3, verbatim):**
> | Grid geometry fold + median | `0..slot_bound` | yes | skip `!(r >= 0.0)`; `n` = live-radius count | no NaN reaches `min`/`max`, the median comparator or `cbrt(n)` |

**Added:**
> | Grid geometry fold + median | `0..slot_bound` | yes | skip `!(r >= 0.0)`; `n` = live-radius count; `n_live == 0` ⇒ degenerate grid and no pairs, before the median (NB3) | no NaN reaches `min`/`max`, the median comparator or `cbrt(n)`; the select never sees an empty slice |

---

## P4-§11: multithreading (NB1, W2, W6a)

**Removed (rev 3, verbatim):**
> Dead slots are written by exactly two writers:
> - `release_dying` at the head of S1. S1 holds the write on every column id and on the recycle node; no reader of those ids runs in S1's round; `GroupSlot` readers touch a disjoint field, per the §5 projection rule.
> - Group insert in apply windows, which are exclusive.

**Added:**
> Dead slots are written by exactly three writers:
> - `open_chain` at the head of S1. `GroupHead` holds the write on every column id and on the recycle node, and no reader of those ids runs in S1's round. `GroupSlot` readers touch the disjoint slot-map node, per the §5 projection rule.
> - Out-of-chain release: a removal, the first-op flush, `clear`. It runs only in exclusive contexts.
> - Group insert (DEAD on append), also exclusive.
>
> `chain_open` is written by `open_chain` (S1) and `close_chain` (S6), which the chain orders, and it is read only in exclusive contexts.

**Removed (rev 3, verbatim):**
> `GangShared` is `Sync`; its `threads` cells are written before the SeqCst publish and read after the swap. `LaneBoard` is `Sync`. A ticket slot is written before the SeqCst `pending` publish and read after a winning SeqCst `fetch_and`.

**Added:**
> `GangShared` is `Sync`. Its `threads` cells are written before the SeqCst publish and read after the swap, only by participants that have not done their exit RMW. `LaneBoard` is `Sync`. A ticket slot is written before the SeqCst `pending` publish and read after a winning SeqCst `fetch_and`. It gets a new tenant only through an Acquire `in_use` claim paired with the previous tenant's Release clear (W6a). After its `exit_count` RMW, a taker touches no gang memory (NB1).

---

## P4-§12: gates, timing, rungs

**Removed (rev 3, timing list, verbatim):**
> - events (S7), compared against S5, which it now overlaps (N3);

**Added:**
> - events (S7), compared against S5, with a flag recording whether S7 actually overlapped S5 (it started before S5's FINAL). Tickets are the lowest priority, so it should (W7);
> - gang exit wait, per gang (NB1: spin then yield, never park);

**Removed (rev 3, U2 row, verbatim):**
> | U2 | K2 + K3 + K6 (kernel only): `StorageKind::Group` with every dispatch made an exhaustive `match`; group store; DEAD; `release_dying`; `DenseGroupMut`; `GroupSlot`; the binder (`BindToken`, `anchor_mask`, 14 sites converted) | **Compile gates:** the workspace builds only with every `StorageKind` match routed. **Trybuild:** `Changed<T>`, `Added<T>`, `Ref<T>`, `Mut<T>` and `EcsMaster::get_component_mut::<T>` on a `ticks="none"` type fail to compile; `insert::<G-column>`, `remove::<G-column>`, and a `Bundle` containing a group column fail; writing an inland without a `BindToken` from outside the binder fails. **Tick routing (B1):** a plain dense `ticks="none"` store under insert, remove ×k, `compact`, `check_ticks` and serde load. This runs in the default debug profile, where `component_pool.rs:1809-1831` and `:1955-1958` `debug_assert!` any unrouted tick access; a missing route is therefore red. `#[should_panic]` for `dense_registry_mut().store_mut(<group column id>)`, and for `EcsMaster::release_dense_group` after a `DenseGroupMut` init. **Proptest:** the slot-state machine (live/dying/free, DEAD on free, `slot_bound`, `s2e`/`e2s` round-trip) under random spawn/despawn/remove-anchor/re-add/non-anchor migration/clone and `release_dying` at arbitrary points. **Miri (Tree Borrows):** `release_dying` on one thread concurrent with a `GroupSlot` read of `e2s` on another; views and `range_mut` |

**Added:**
> | U2 | K3 + K6, kernel only. **K2 is decoupled** (D10 rev 4): its trybuild and tick-routing gates move to K2's own rung, unchanged. Content: `StorageKind::Group` with exhaustive runtime matches plus the comparison census; `GroupColumn` (not `Component`), `#[dense_group]` and the exclusivity probe; `install_anchor` / `ANCHOR_GROUP`; the inline group store and `ensure_group`; DEAD; `open_chain` / `close_chain`; out-of-chain release, the flush and `clear`; `GroupHead` / `GroupTail` claims; `GroupSlot` / `GroupRef`; the binder (`BindToken`, `DerefMut` removed, `anchor_mask`, 14 sites converted). **X-17 bug fix:** `clear()` also clears plain dense stores and drops their values, with a red-first test | **Trybuild (NB2):** one fixture per family, each E0277 on a group column: `&T`, `&mut T`, `Mut<T>`, `Ref<T>`, `With<T>`, `Without<T>`, `Added<T>`, `Changed<T>`, `IsEnabled<T>`, `get_component`, `get_component_mut`, `insert`, `remove`, a tuple bundle containing one. **Behavioural (NB2):** on the pyramid, the row count of `Query<(Entity, GroupRef<BodyGate>)>` == `live_count` == the row count of `Query<Entity, With<Collider>>` (1241 per pass 3). After one `Collider` removal all three drop by one. **Census:** no `== StorageKind::` / `!= StorageKind::` / `matches!(…StorageKind::…)` outside `component_registry`; no `impl DerefMut` / `IndexMut for InlandStore`; no `entities_inland =` outside `entity_master.rs`. **`#[should_panic]`:** a second live `GroupHead<G>` or `GroupTail<G>` claim; a type that is both `Component` and `GroupColumn`; `dense_registry_mut().store_mut(<group column id>)`. **Anchor timing (W3):** a world spawns `Collider` entities before the physics plugin builds and before any group param is initialised. Assert `anchor_mask` is set and `live_count` == the collider count. The reconciliation check recomputes masks from component sets. **Proptest:** the slot-state machine (live/dying/free; DEAD on free and at ≥ `slot_bound`; `s2e`/`e2s` round-trip; removal defers iff `chain_open`; `dying` empty after `open_chain` and after the first out-of-chain op) under random spawn, despawn, remove-anchor, re-add, non-anchor migration, clone and clear, with `open_chain` / `close_chain` at arbitrary points. **Miri (Tree Borrows):** `open_chain` on one thread concurrent with a `GroupSlot` read of `e2s` on another; views and `range_mut` |

**Removed (rev 3, U4 fragment, verbatim):**
> despawn (direct, command, hierarchy cascade); `clear()`.

**Added:**
> despawn (direct, command, hierarchy cascade); `clear()` outside the chain (store reset, `len == 0`) and inside it (an exclusive system `.after(S3).before(S4)`: every slot dying, `len` kept, released at S1(i+1)).

**Removed (rev 3, U5 fragment, verbatim):**
> at step i+1, pair and manifold counts equal the survivors' only, and the slot's bytes == DEAD after S1.

**Added:**
> at step i+1, pair and manifold counts equal the survivors' only, and the slot's bytes == DEAD right after the despawn's window (out-of-chain release, D14 rev 4). **All-dead Grid step (NB3):** Grid selected in Manual mode; spawn k bodies; step; despawn all between steps; step twice more. Expect no panic, zero pairs and degenerate geometry. A variant makes every live slot a mid-chain insert (NaN radius) and expects the same. It must be red with the early return keyed on `slot_bound` (mutation, §17). **Auto count:** after a mass despawn, Auto returns to AllPairs once `live_count` < GRID_LO.

**Removed (rev 3, U7 row, verbatim):**
> | U7 | `PairCache` merge + `fresh_step` | **Reuse (B2):** despawn X between steps i and i+1; spawn Y in a window after S1(i+1), with the spawner ordered `.after(physics_prepare)`. Assert `slot(Y) == slot(X)` (non-vacuity) and that Y's first simulated step inherits no warm impulse or axis. **Same-window despawn+spawn:** assert `slot(Y) != slot(X)`, since X is dying, not free (documents D14). **Clone into a reused slot (B5):** despawn X between steps; clone a live contacting body Z in a window after S1(i+1). Assert `slot(clone) == slot(X)`, the clone's `BodyGate` SEEN == 0 before its first S1, group `live_count` +1 exactly (no double insert), and no inherited warm start |

**Added:**
> | U7 | `PairCache` merge + `fresh_step`; S7 fresh rule | **Out-of-chain reuse, a GATE (W5).** X rests on Z; assert the warm impulse at step i is non-zero. After step i returns, in one window: despawn X and spawn Y at X's pose. Assert `slot(Y) == slot(X)` (non-vacuity: LIFO). Assert that Y's first-step warm seed on (Y,Z) is zero and its axis is recomputed. Assert that S7(i+1) emits End(X,Z) and Start(Y,Z). **Mutation runs:** deleting the `PairCache` fresh skip turns the warm assertion red; deleting S7's fresh rule turns the event assertion red. **Clone into a reused slot, a GATE:** the same shape with Y = `clone_and_spawn` of a live contacting body. Also assert `BodyGate` SEEN == 0 before its first S1, and `live_count` +1 exactly (no double insert). **`clear()` outside the chain, a GATE:** new spawns reuse slots from 0 and inherit nothing (the same mutation run). **Head-release reuse, a PIN:** X despawned in S3's window of step i; Y spawned after S1(i+1). Assert `slot(Y) == slot(X)` and no inheritance. This stays green with the skip deleted (D15 case A, i ≤ j−2), so it is labelled a pin. **Mid-chain despawn + spawn in one window, a PIN:** `slot(Y) != slot(X)`, because X is dying. **`clear()` inside the chain (W1):** no S7 event in step i names a new entity |

**Removed (rev 3, P2 row, verbatim):**
> | P2 | K5b gang with the lane ticket board; S5 on one gang; soft solve on a gang | `{1,N}`; nested-gang stress and board loom model (§17); G-alloc ≤ 16 per step at W=8 (budget unchanged until measured; the gang no longer allocates); `pool.scope` census 0. **N1 telemetry:** the G-jolt receipt records `taken` per gang, and a G-jolt variant adds a concurrent `par_iter` system. Its `taken` distribution and T(8) are recorded, not gated: timing-dependent |

**Added:**
> | P2 | K5b gang with the lane ticket board; S5 on one gang; soft solve on a gang | `{1,N}`; nested-gang stress and the board loom model (§17); **the Miri gang-exit protector gate** (§17); **panic mid-chunk** on lane 0 and on a taken lane at W ∈ {2,4,8}: no hang (watchdog), the payload re-raised exactly once, and the pool still has W workers; G-alloc ≤ 16 per step at W=8 (budget unchanged until measured; the gang no longer allocates); `pool.scope` census 0. **N1/W7/NB1 telemetry:** the receipt records per gang `taken` and exit-wait time, and records whether S7 overlapped S5. A G-jolt variant adds a concurrent `par_iter` system. All recorded, not gated: timing-dependent |

---

## P4-§14 (NB1)

**Removed (rev 3, verbatim):**
> - gang spin budget and the exit-wait spin budget;

**Added:**
> - gang spin budget, and the exit wait's spin-then-yield budget (the exit wait never parks, §10.1);

---

## P4-§16: integration

**Removed (rev 3, verbatim):**
>   `ecs_master.rs:1027` `clear()` resets the group stores.

**Added:**
>   `ecs_master.rs:1026-1028` `clear()`: the binder's clear route applies D14 rev 4's clear rule to every group store and, for X-17, now also clears plain dense stores.

**Removed (rev 3, verbatim):**
> - `entity/entity_master.rs` + `entity/inland_store.rs`: token-gated archetype writers, `fix_unit_index`, the `inland()` accessor, and a raw projector for `system/unsafe_ecs_cell.rs:329`.

**Added:**
> - `entity/inland_store.rs`: `impl DerefMut` deleted; `set(&BindToken, …)`; `fix_unit_index`. `entity/entity_master.rs`: token-gated `register_entity_with_ptr` / `register_batch` / `deallocate_entity`, plus a raw projector for `system/unsafe_ecs_cell.rs:329`. Read sites are unchanged, since `Deref` is kept.

**Removed (rev 3, verbatim):**
> - `component_registry`: `StorageKind::Group`, and the 57 dispatch occurrences made exhaustive `match`es.
> - `component/dense/dense_registry.rs`: the group table, and the `store_mut` release assert. `dense_store.rs:640`: `move_ticks` routed.
> - `ecs_master/component_api.rs:635`: the `get_component_mut` const-assert.

**Added:**
> - `component_registry`: `StorageKind::Group`; group column registration, which does not go through `Component`; `install_anchor` / `ANCHOR_GROUP`; the 57 runtime dispatch occurrences made exhaustive `match`es.
> - `component/dense/dense_registry.rs`: the inline `groups` table, `ensure_group`, and the `store_mut` release assert.
> - `dense_store.rs:640` (`move_ticks` routing) and `component_api.rs:635` (the `get_component_mut` const-assert) move to K2's rung.
> - `boyko_macros`: `#[dense_group]` (the `GroupColumn` impls and the exclusivity probe); `#[component(anchor_group = …)]` emits `install_anchor` inside `component_id()`'s `get_or_init` (`component.rs:374-392`).
> - `boyko_physics`: the grid fold and early-return restructure (NB3); S1 reads `live_count` for Auto; S6 holds `GroupTail` and is `.after(S7)`; S7 applies the fresh rule.

**Removed (rev 3, verbatim):**
> - `system/params`: `DenseColumn*`, `DenseGroupMut`, `DenseSlots`, `GroupSlot`.

**Added:**
> - `system/params`: `DenseColumn*`, `DenseSlots`, `GroupHead`, `GroupTail`. Query terms: `GroupSlot`, `GroupRef`.

**Removed (rev 3, verbatim):**
> - `LaneBoard` in `PoolInner`; board probes at `worker.rs:84-87`, `:115-118`, `:126-130` (post-`mark_idle` probe preceded by `publish_fence`). No change to any `run_task` site or join route.

**Added:**
> - `LaneBoard` in `PoolInner`. Board probes after `worker.rs:96-99` (the steal), after `:115`, and after `:126` (`pop_any` came up empty; the post-`mark_idle` probe is preceded by `publish_fence` and followed by `unmark_idle` on a take). No change to any `run_task` site or join route.

---

## P4-§17: validation

**Removed (rev 3, verbatim):**
> - **board:** exactly-once lane numbering across take/cancel races; a ticket taken after FINAL returns at once; lane 0 never returns before `exited == taken` (poisoned-payload test: a lane panics after taking; lane 0 re-raises after the wait);

**Added:**
> - **board:** exactly-once lane numbering across take/cancel races; a ticket taken after FINAL returns at once; lane 0 never returns before `exit_count == taken`; a slot gets a new tenant only after its `in_use` bit is cleared.
> - **panic:** a lane panics mid-chunk, on lane 0 and on a taken lane. Expect no hang, the other lanes to exit, and lane 0 to re-raise once after the exit wait.

**Removed (rev 3, verbatim):**
> - the lane board: lane 0 plus 2 workers. It covers publish, take, cancel-before-take, cancel racing a take, take after FINAL, and the exit-wait park handshake (`exited` vs `exit_waiting`, both SeqCst), including a lost-wake schedule that must still terminate with lane 0 alone.

**Added:**
> - the lane board (loom): lane 0 plus 2 workers, with `GangShared` in a `Box` dropped when lane 0 returns. It covers publish (including the Acquire `in_use` claim of a slot the previous tenant has just released), take, cancel-before-take, cancel racing a take, take after FINAL, and the exit wait (Release `exit_count` RMW against an Acquire load; no park). **Loom cannot detect a use-after-free, so it is not the lifetime gate.**
> - **Miri gang-exit protector gate** (`boyko_threadpool/tests/miri_gang_exit_protector.rs`, modelled on `miri_scope_completion_protector.rs`, X-16). `GangShared` sits on lane 0's stack. A `#[cfg(miri)]` yield burst follows the taker's `exit_count` RMW. It runs under Tree Borrows with several seeds. **It must be red on a mutant that performs one gang load after the RMW** (rev 3's `exit_waiting.load`), and green on rev 4.

**Removed (rev 3, verbatim):**
> - F-1, F-3, FRESH reuse (U7, with slot equality), clone into a reused slot, mid-chain despawn, dead-slot-at-origin (between steps and mid-chain), release-order determinism, mid-chain spawn with an empty free list, Collider remove/re-add, non-anchor migration keeps the slot, per-path anchor anti-vacuity (U4);

**Added:**
> - F-1, F-3; out-of-chain reuse (U7 GATE: slot equality, warm and event assertions); clone into a reused slot (GATE); `clear()` inside and outside the chain; head-release reuse (PIN); mid-chain despawn; dead-slot-at-origin (between steps and mid-chain); release-order determinism; mid-chain spawn with an empty free list; **all-dead Grid step (NB3)**; **group query row count (NB2)**; **anchor before plugin (W3)**; Collider remove/re-add; non-anchor migration keeps the slot; per-path anchor anti-vacuity (U4);
> - **mutation runs (each must go red):** delete the `PairCache` fresh skip (U7 warm); delete S7's fresh rule (U7 events); re-add one gang load after the exit RMW (Miri gate); key the grid early return on `slot_bound` (all-dead Grid step); cache `anchor_mask` before the binding (the anchor-before-plugin reconciliation check);
> - **census of persistent slot-keyed state:** `PairCache` and `ContactEventState`. Any new resource keyed by body slot must be added to this list and must apply the fresh rule. This is enforced by review; it is listed next to §4's table.

**Removed (rev 3, verbatim):**
> **trybuild:** change detection on `ticks="none"` (`Ref`, `Mut`, `Added`, `Changed`, `EcsMaster::get_component_mut`); group-member insert and removal (typed, `Bundle`); inland write without `BindToken`.

**Added:**
> **trybuild:** every `Component`-generic term and API on a group column (E0277; the U2 list). Change detection on plain-dense `ticks="none"` moves to K2's rung. The inland write is gated inside the crate by the compiler (no `DerefMut`) and by the census, not by trybuild, because the surface is crate-private.

**Removed (rev 3, verbatim):**
> - `release_dying` runs exactly once per step, and `dying` is empty after it.

**Added:**
> - `open_chain` and `close_chain` each run exactly once per step, in that order. `dying` is empty after `open_chain` and after the first out-of-chain group op;
> - `mid < scratch_radii.len()` before the grid median select;
> - S7: no continued record names a fresh slot;
> - per gang: `popcount(threads_written)` ≤ the number of lanes that ran.

---

## P4-§18: open questions

**Removed (rev 3, verbatim):**
> 9. Not verified: whether `EcsMaster::clear()` (`ecs_master.rs:1026-1028`) resets `dense_registry` today. The group-store reset is added regardless.

**Added:**
> 9. **Answered (W1): it does not** (X-17). This is a pre-existing plain-dense bug, fixed in U2. Group stores follow D14 rev 4's clear rule.
> 11. Not measured: exit-wait time per gang (P2 receipt). If it is routinely about one OS wake latency because lanes park during a trailing `single` (S5 ends with one), there are two candidates, decided with P2's numbers:
>     - a `single_last` that lets non-owning lanes exit before it runs;
>     - the W-d′ count-gated park for a worker lane 0 only, keeping spin for a non-worker lane 0.
> 12. Not verified: the cost of the `e2s` lookup in `GroupRef` for gameplay queries (one random read per row). Physics hot loops are unaffected.

---

## P4-Checklist: edge cases

**Removed (rev 3, verbatim):**
>   - an exclusive `release_dense_group` in a world with a chain head (refused);

**Added:**
>   - a second `GroupHead` / `GroupTail` claim (refused);
>   - every slot below `slot_bound` dead with Grid selected (NB3);
>   - `clear()` inside and outside the chain (W1);
>   - a despawn and a spawn into the same slot between two consecutive steps (D15 case B);
>   - a lane panicking mid-chunk, on lane 0 and on a taken lane (W6c);
>   - `Collider` entities spawned before the physics plugin builds (W3);
>   - a type that is both `Component` and `GroupColumn` (refused);

---

## Responses without a section edit

- **Which pass-3 findings are fragile-mechanism classes, and what replaced them.**
  - NB2 is the storage-kind-blindness class. Patrolling it needs every const site and every future term to cooperate. It is **removed**: group columns are not `Component`s.
  - NB1 is a lifetime class, and a Dekker proof cannot discharge it. It is **removed**: there is no post-RMW access at all.
  - W2 (release authority) is **removed** by a single `chain_open` rule that every structural path consults, replacing ad hoc refusals.
  - Stated costs: gameplay syntax `GroupRef<T>` instead of `&T`; up to one wake latency of spinning per gang; D15's fresh rule becomes load-bearing and needs the S7 merge change.
- **W7:** answered in D7, §10.1 and §12 (tickets are the lowest priority, and S7's overlap is recorded).
- **O1:** the replacement row is given in the writer notes. The decisions file is outside the design and is applied by the writer.
- **Pass-2 fixes the critic confirmed:** B1's table is unchanged and moves with K2. B2, B3, B4 and B5 are unchanged in mechanism. B3's `slot_bound` also holds under out-of-chain release, because `slot_bound` is captured at S1. The rev-3 D15 case (A) proof is kept verbatim as case (A).

---

## Changelog: rev 3 → rev 4

| Finding | Action | Section |
|---|---|---|
| NB1 LaneBoard exit: use-after-free of `GangShared` | FIX (class removed). The exit wait never parks. The taker's last access is `exit_count.fetch_add(Release)` through a by-value raw pointer (X-16 protector argument). `exited` and `exit_waiting` are deleted; `threads_written` governs the drops. The lifetime gate is a Miri test modelled on `miri_scope_completion_protector.rs`, required red on rev 3's post-RMW load. Loom keeps ordering and termination, and is stated not to see UAF. W-d′ rejected, with reasons | §0 X-16, §5, §10.1, §11, §12 P2, §14, §17, §18 Q11 |
| NB2 group columns under compile-time query dispatch | FIX (class removed). Group columns are not `Component`s: `GroupColumn` only, from `#[dense_group]`, with an autoref exclusivity probe. Every `Component`-generic term and API gives E0277. Group-aware `GroupSlot`/`GroupRef` are archetypal on the anchor. The runtime family keeps exhaustive matches plus a comparison census. `IS_GROUP_COLUMN` deleted. K2 decoupled from K3. Behavioural gate: `GroupRef` row count == `live_count` == `With<Collider>` count | §0 X-15, D4, D10, D13, §5, §8, §9, §12 U2, §16, §17 |
| NB3 grid: empty median input panics | FIX. The degenerate return keys on `n_live == 0` after the fold, before the median. `debug_assert!(mid < len)`. Auto's count is `live_count`, never `slot_bound`. All-dead Grid step plus mutation | §10.2, §10.3, §10.8, §12 U5, §17 |
| W1 `clear()` undefined; pre-existing plain-dense bug | FIX. Outside the chain, reset (`len = 0`); inside the chain, everything goes to dying and `len` is kept. X-17 plain-dense clear fixed in U2 with a red-first test. §18 Q9 answered | §0 X-17, D13, D14, §12 U2/U4/U7, §16, §18 |
| W2 release authority wrong both ways | FIX. `chain_open` (open at S1, close at S6); defer iff open, else release at once; the first out-of-chain op flushes dying. `GroupHead`/`GroupTail` are unique per-world claims. `DenseGroupMut`, `release_dying` and `EcsMaster::release_dense_group` deleted. The S6 `.after(S7)` edge restored at zero rounds | D14, D6, §6, §8 K3/K6, §9, §10.6, §11 |
| W3 anchor binding and store creation time | FIX. `install_anchor` runs inside the anchor's `component_id()` `get_or_init` (`component.rs:374-392` precedent), and the group initialises only from there, so there is no reverse `OnceLock` nesting. Inline `groups[8]` with `ensure_group` in the transition and in `init_state`. No cached pointers. Reconciliation recomputes masks from component sets. Anchor-before-plugin test | D13, §5, §12 U2, §17 |
| W4 `BindToken` gated the wrong surface | FIX. `impl DerefMut for InlandStore` deleted; the token-gated `set` is the only archetype writer; `fix_unit_index` tokenless; census for `DerefMut`/`IndexMut`/whole-store assignment. Trybuild stated impossible (crate-private), with the compiler as the gate | §0 X-18, D13, §16, §17 |
| W5 U7 assertions could not fail | FIX. Out-of-chain reuse makes the fresh skip load-bearing (D15 case B), so U7 has GATE cases with mutation runs (`PairCache` skip, S7 rule). Head-release cases are labelled PIN. The S7 fresh rule is added, since it is required once reuse between consecutive steps exists | D15, D6, §10.7, §12 U7, §17 |
| W6 LaneBoard details | FIX. (a) `in_use` claim is `compare_exchange_weak(…, Acquire, Relaxed)`, with the happens-before chain stated. (b) `unmark_idle` on a take at the post-`mark_idle` probe. (c) Per-chunk unwind guard on every lane; waits end on `poisoned`; the poisoner wakes parked lanes | §10.1, §11, §17 |
| W7 ticket vs injector priority | FIX (decided). Tickets are the lowest-priority work (probes after own deque, injector and steal), because gangs are elastic on entry and tasks are not. S7 overlap is recorded in the timing | D7, §10.1, §12 timing, §16 |
| O1 decisions row names a deleted mechanism | ACCEPT. Replacement row text is given for the writer | Writer notes |

**Sections whose invariants this patch depends on (critic re-read list):**
- **§10.1** depends on `worker.rs:84-130`, `:227-233`, `:383-451` and `scope.rs:541-560`, `:823-855`, `:909-917`.
- **D13** depends on `boyko_macros/src/component.rs:369-394`, `inland_store.rs:162`, `:227`, `:260`, `:285-287`, `migration_helpers.rs:963`, `:977`, and `bundle/self_bundle.rs:54-56`.
- **NB2** depends on the eleven leaf impl headers listed in X-15.
- **D14 and D15** depend on `schedule.rs:669` and `:824-832` (X-11), and on the S6 `.after(S7)` edge.
- **§10.3** depends on `resources.rs:1007-1053`, `:1165-1180`, `:1218-1222` and `broadphase_policy.rs:182-192`.

**Primary files read this round** (all in `D:/wt/joltab/crates/`):
- `boyko_threadpool/src/worker.rs`
- `boyko_threadpool/src/scope.rs`
- `boyko_threadpool/tests/miri_scope_completion_protector.rs` (existence checked)
- `boyko_ecs/src/ecs/core/component/component.rs`
- `boyko_ecs/src/ecs/core/component/component_registry/mod.rs`
- `boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs`
- `boyko_ecs/src/ecs/core/iters/query/data.rs`, plus `data/{read,write,mut_,ref_}.rs` and `filter.rs` (the impl headers)
- `boyko_ecs/src/ecs/core/bundle/self_bundle.rs`
- `boyko_ecs/src/ecs/core/entity/inland_store.rs`
- `boyko_ecs/src/ecs/core/entity/entity_master.rs`
- `boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`
- `boyko_macros/src/component.rs`
- `boyko_physics/src/resources.rs`
- `boyko_physics/src/broadphase_policy.rs`

The design document is `D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md`; the decisions file is `D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md`.

# Critique log - pass 4 (2026-09-11)

Pass 4 was scoped to the rev-4 patch. The critic's output is reproduced below verbatim: the verdict, every blocking finding, every non-blocking finding, and the fixes it confirmed. Each line break inside the verdict is the critic's own; the empty quote lines between them are added only so the Markdown renders them as separate paragraphs. The architect's response is the Rev 5 patch that follows this section.

## Verdict

> CHANGES REQUESTED (pass 4, scoped to the rev-4 patch). There are two blocking defects, and each needs about one sentence of patch text. No mechanism has to change. The three new mechanisms are sound as designed.
>
> (1) NB1 is fixed. On every normal path, the taker's last access to gang memory is the Release `exit_count` RMW. That holds for a late lane at FINAL, for cancel racing a take, for a nested gang and for a worker that lost the ticket race.
>
> (2) NB2 is fixed. Every Component-generic surface I traced fails with E0277, and none gives a silent zero.
>
> (3) NB3 is fixed on every input: all slots dead, one live slot, everything below slot_bound dead, and slot_bound = 0.
>
> (4) The chain_open rule holds on every removal path. A panic, or an S6 that skips `close_chain`, leaves the chain open, which only defers releases (the safe direction).
>
> The two blockers are both in text rev 4 added.
>
> - PB1: the rev-4 panic protocol publishes FINAL from lane 0's drop guard without setting `poisoned`. After a lane-0 panic outside a chunk body, every phase barrier then passes at once, so the other lanes run consecutive phases concurrently.
> - PB2: the new GroupHead/GroupTail claim is "released by the State's Drop". That Drop has no way to reach the world, and the same patch forbids a State from caching a store pointer.
>
> There are also 8 non-blocking items. Evidence is from D:/wt/joltab. No cargo was run and nothing was timed.

## Blocking findings

- PB1 - Panic protocol: lane 0's drop guard publishes FINAL without setting `poisoned`, so after a panic outside a chunk every phase barrier passes at once (P4-§10.1 Panic; this is rev-4 text). WHERE: design :2868 "The guard publishes FINAL (`completed.store(FINAL, SeqCst)`, then swap and unpark), cancels, runs the exit wait, drops any stored payload". No `poisoned` store is listed. `poisoned` is set only by the per-chunk guard, which is armed "before each chunk body" (:2862). The waits are "Every wait loop exits on `completed >= p || poisoned`" and "`for_each` and `single` return at once when `poisoned` is set" (:2866). FINAL = u32::MAX (P-§5 :1532), and the rev-2 Claim still opens the next phase: "`phase == p − 1` → CAS to `(p, 1)`" (:579). PROBLEM: suppose lane 0 panics in SPMD code between two `for_each` calls, or in `for_each`'s own machinery such as the `phase_n` debug assert (:603). Then `poisoned` is never set. Every taken lane's wait for phase q is satisfied by FINAL, so the first lane to finish its chunks of q+1 opens q+2 while another lane is still running a q+1 chunk. The barrier is gone. Rev 3's guard stored no FINAL, so there the taken lanes finished the phases with real barriers; rev 4's FINAL store introduced this. Two related gaps sit on the taken-lane side. The take path's catch (:2867) stores the payload but never sets `poisoned`, and lane 0 re-raises only "if it finds `poisoned` set" (:2869), so a taken lane's panic outside a chunk is silently swallowed. And if that panic lands after a successful claim CAS but before the guard is armed, the claimed chunk is never completed and the phase stays one short forever: W6c's hang, reached from machinery code. CONSEQUENCE: in S5, consecutive phases are solver colors that share bodies. At W>=2, a lane-0 panic between colors lets two taken lanes write the same body's velocity concurrently through the unsafe column views, a data race (UB) on the recovery path that the P2 gate exercises. A debug SPMD-divergence assert in a taken lane vanishes, and the gang returns as if it had succeeded. CONFIDENCE: CONFIRMED from the patch text. NEEDED: every unwind catch point, meaning lane 0's drop guard and the take path's catch_unwind, must set `poisoned` (SeqCst) before anything else, with FINAL following it. Alternatively, `for_each`/`single` must also return on `completed == FINAL`; the patch must pick one. The per-chunk guard must cover the whole span from claim CAS to Complete, not only the body. Add 'lane 0 panics between phases' and 'a taken lane panics outside a chunk' to the P2 panic gate and to §17.
- PB2 - The GroupHead/GroupTail claim is 'released by the State's Drop', which has no world and is forbidden a store pointer (P4-§8 K3, P4-§5, P4-§3 D13; all rev-4 text). WHERE: design :2733 "A claim is taken in `init_state` and released by the State's Drop."; :2625 `head_claimed: bool, // one live GroupHead<G> State per world; cleared by the State's Drop`; contradicted by :2367 "No system State caches a pointer into the store. A State holds the `GroupId` and column ids and resolves the store once per run through the world cell." CODE: SystemParam gives a world only to `init_state` (system_param.rs:142 `fn init_state(world: &mut EcsMaster, ...)`) and `apply` (:198). There is no teardown hook. App owns the world by value as its FIRST field, so the world drops before the schedules. app.rs:126-132 reads "no field's Drop observes another", then `world: EcsMaster,`. PROBLEM: 'cleared by Drop' has no sound implementation as written. If the State caches a pointer to the claim flag, which the patch itself forbids, then every App drop writes into a freed EcsMaster: a use-after-free. It also dangles if the App is moved after `finish`, because EcsMaster is inline. If the flag is never cleared, any second `init_state` of the physics systems on the same world panics with 'second live claim', for example a schedule rebuilt for editor play/stop or a test that builds two schedules on one world. CONSEQUENCE: an implementer following P4-§5 writes either a UAF on every App shutdown with physics (CONFIRMED given app.rs:126-132, if the pointer route is taken) or an unreleasable claim (how often a rebuild happens is PLAUSIBLE). CONFIDENCE: CONFIRMED that the text contradicts itself and that the kernel has no teardown hook. NEEDED: define a release that has world access, or define the claim so that it needs no release, for example idempotent re-initialisation keyed on the system's identity. It must not depend on a pointer into a by-value EcsMaster. State it in §8 K3 and in the §5 field comments, and keep the U2 '#[should_panic] second live claim' gate consistent with it.

## Non-blocking findings

- NW1 - A take at the post-mark_idle probe spends a producer's claimed wake on a resident lane. `unmark_idle` returns the idle word precisely because "a caller that reads its own bit back as CLEAR owes that wake a lane which will actually scan" (worker.rs:392-410). worker_main may ignore that word only because "every one of its post-`mark_idle` exits either runs a task or `continue 'outer`s into a full re-poll" (:402-404). P4-§10.1 step 3 (:2833) copies the ignore. A ticket exit, however, runs until FINAL without scanning. CONSEQUENCE: a system task pushed while an idle worker sits between `pop_any` (:126) and the board probe waits up to one whole gang (S5 is the longest stage), even with other workers parked. That is W7's cost again through a narrow race; it costs time, never correctness. CONFIDENCE: the mechanism is CONFIRMED, the frequency PLAUSIBLE. Direction: on a take at that probe, if the returned word shows the bit already cleared, hand the wake on (as KE16 B1-P does) before running the lane.
- NW2 - Where `install_anchor` sits inside `Collider::component_id()`'s get_or_init matters. The binding is guaranteed to exist first only for threads that get the id through that OnceLock (:2361). `register_new` (boyko_macros component.rs:375) and `install_serialize_fn` (component_registry/serialize.rs:204, which publishes the stable name) make the raw id discoverable before the closure returns. If `install_anchor` is emitted after `#serialize_install` (component.rs:392), a load running concurrently on another thread and resolving 'Collider' by name could create an archetype before `ANCHOR_GROUP[raw]` is written. Its mask would then be 0 forever; the debug reconciliation catches this, release builds do not. CONFIDENCE: PLAUSIBLE, and it needs a concurrent first-mint plus load. Direction: emit `install_anchor` right after `register_new`, before any install that publishes `raw`.
- NW3 - The inland `clear` stays reachable without the binder. `EntityMaster::clear` is `pub` (entity_master.rs:481-483, which calls `self.entities_inland.clear()`), and `EcsMaster::entity_master_mut` is `pub` (ecs_master.rs:569). The rev-4 census only forbids `entities_inland =` outside entity_master.rs, so it does not see this call. The footgun is pre-existing: it already desynchronises archetypes. But it now also leaves group slots live with no anchor transition. Direction: token-gate `EntityMaster::clear` or make it pub(crate). Also, the X-18 citations have drifted in D:/wt/joltab: `impl Deref` is at inland_store.rs:282, `impl DerefMut`/`deref_mut` at :307-309 and `clear` at :236, not the cited :260, :285-287 and :227.
- NW4 - The runtime census exempts the module that holds the fallback. `storage_kind()` decodes with `_ => StorageKind::Table` (component_registry/mod.rs:409), inside `component_registry`, which the census exempts. If the Group decode arm is forgotten, every Group id reads back as Table, and every exhaustive match routes it to the table arm silently. Direction: add a Group round-trip unit test beside the Dense one (mod.rs:1595-1599), and make an unknown discriminant fail loudly instead of defaulting to Table.
- NW5 - Rev-4 reuse between steps widens the §17 slot-keyed census. `ContactPairs`, `Manifolds` and the sensor overlaps are slot-keyed and stay readable after S6. Under rev 3, a slot read through `DenseSlots` after the chain mapped to TOMBSTONE. Under rev 4 a despawn and a spawn in one window can re-tenant it, so a post-chain reader names the new entity. No in-tree consumer maps them today; only tests read counts or contents. Direction: list them in the §17 census as 'valid until the next structural window', or document that on the resources.
- NW6 - A losing lane's panic payload is dropped before Take step 6 (:2867, 'CASes payload_claimed ... if it wins, stores ... Then it runs Take step 6'). A payload whose Drop panics (via `panic_any`) would unwind past step 6: `exit_count` never reaches `taken`, and lane 0's never-parking exit wait spins forever. This is exotic. Direction: move the loser's payload drop after step 6 under an abort guard, or leak it.
- NW7 - With the poison fix, SPMD control flow can still hang. The SPMD contract allows a control-flow value 'from a completed `single`' (:603), but on poison `single` returns at once without running. A program whose loop exit comes from a `single` then spins forever in every lane, because each `for_each` also returns at once, and lane 0 never reaches its exit wait. The physics programs appear to use fixed counts, so this is PLAUSIBLE for future gang consumers (the K5b list names transform propagation and animation). Direction: have poisoned lanes leave the program, for example by unwinding a sentinel out of `for_each`/`single` that the lane catch recognises, rather than returning into the program. This can be folded into the PB1 fix.
- NW8 - Miri gate placement. In the mutant, the extra gang load must come after the `#[cfg(miri)]` yield burst, so that lane 0 can return and pop the frame between the RMW and the load. A load placed immediately after the RMW, with the burst after it, can stay green. The existing precedent learned that probe placement is not self-evidently decisive (scope.rs:882-901). State the order in §17.

## Fixes the critic confirmed

- NB1 RESOLVED on every normal path. The taker holds the gang as a by-value `*const` (:2834), and every `&GangShared`/`&GangLane` dies in the step-5 lane frame. Step 6's Release `exit_count.fetch_add` is the last access. Lane 0's Acquire load of the final count reads the tail of the RMW release sequence, so it happens-after every taker's reads, including its `threads[i]` unparks as a phase completer and its payload store. A late lane reads `completed` before its RMW. A worker that loses the `pending` race touches only the pool-lifetime board. Cancel versus take is decided by RMWs on `pending`, so `taken` is exact. A nested gang's GangShared lives in the chunk frame and its takers are top-level workers. The protector argument matches scope.rs:848-855 (the RMW's `&self` covers only UnsafeCell bytes; the CachePadded deref frame returns before the RMW). `exited`, `exit_waiting` and the `threads[0]` question are gone. The W6a Acquire `in_use` claim closes the re-tenancy happens-before chain. Loom is correctly scoped to ordering, and Miri is the lifetime gate.
- NB2 RESOLVED. Every leaf is `T: Component`-bounded (read.rs:81, write.rs:78, mut_.rs:208, ref_.rs:118, data_is_enabled.rs:140, filter.rs:529/709/986/1313, filter_enable.rs:151/334). Option, AnyOf, the tuples and Or require their inner leaf, and the relation terms require `Relationship`. The self-Bundle is emitted per Component (self_bundle.rs:54-56). insert, remove and get_component* are Component-bounded, so a group column gets E0277 everywhere, with no runtime zero. In the runtime family, the census catches every `==`/`!=`/`matches!` dispatch site I found: component_api.rs:201/274/467/657/765, insert_command.rs:135/256, remove_command.rs:87, migration_helpers.rs:469/660 and load.rs:509/676. The exhaustive matches at bundle_column_cache.rs:429-440 and migration_helpers.rs:777-836 become compile errors. `is_signature_storage` returning false for Group is the correct answer at its callers. The behavioural gate (GroupRef count == live_count == With<Collider> count) is the right oracle. DenseColumn<T: GroupColumn> and TypedDenseView<T: Copy> (:523-529) are consistent with columns that are not Components.
- NB3 RESOLVED on every input (resources.rs:1007-1053 and :1173-1177 re-read). Keying the degenerate return on `n_live == 0` after the fold covers all slots dead, a mid-chain insert into a DEAD slot, and live appends at or above slot_bound. One live slot gives mid = 0 on a length-1 slice. slot_bound == 0 stays a fast path. Auto reads `live_count`, not the high-water. The mutation run (keying the return on slot_bound) makes the all-dead test able to fail.
- chain_open (W1/W2) RESOLVED apart from PB2. Every removal passes the binder in an exclusive context and consults `chain_open`. `open_chain` and `close_chain` are serial in S1 and S6 and are ordered by the chain, and S6 `.after(S7)` costs zero rounds (X-11). A panic, or an S6 that skips `close_chain`, leaves the chain open, which only defers releases, the safe direction. clear() works both inside and outside the chain. D15 case (B), including flush and clear, gives i = j−1, and the fresh rule covers both PairCache and the S7 merge. The U7 GATE cases can now fail under their mutation runs, and the head-release cases are honestly labelled PINs.
- W3 RESOLVED apart from the NW2 ordering: the binding is minted inside the anchor's own mint (component.rs:374-392), the order is anchor → group with no reverse nesting, groups live in an inline store with no cached pointers, and the reconciliation recomputes masks from the component sets. W4 RESOLVED: deleting DerefMut (inland_store.rs:307-309) makes the forms at migration_helpers.rs:963/977/1369/1378/1753/1767/2079/2088 and entity_master.rs:308/350/380 fail to compile until they are routed through `set`/`fix_unit_index`. W5, W6a, W6b and W7 RESOLVED. D10's untracked-by-construction claim matches scratch_column.rs:67-74. O1 is applied at PHYSICS-ECS-UNIFICATION-DECISIONS.md:89.

# Rev 5 patch

## How to read rev 5

- **Everything above stays exactly as written.** Rev 2, the rev-3 and rev-4 patches and every critique log are unedited. This file is append-only, so the rev-5 patch is appended rather than applied in place. The current revision is **rev 5 after four critique passes**, and the file's layout is: rev 2 → rev1→rev2 changelog → Critique log - pass 1 → preserve list → Critique log - pass 2 → Rev 3 patch → Critique log - pass 3 → Rev 4 patch → Critique log - pass 4 → Rev 5 patch.
- **Reading order for any section.** Start from rev 2. Apply the rev-3 patch's `P-§…` block for that section, if one exists. Then the rev-4 patch's `P4-§…` block, if one exists. Then the rev-5 patch's `P5-§…` block, if one exists: rev 2 → `P-§…` → `P4-§…` → `P5-§…`. Each `P5-§…` block quotes verbatim the rev-2, rev-3 or rev-4 text it removes, and gives the text that replaces it. **Rev 2 and the rev-3 and rev-4 patches stand except where rev 5 names a section.** A section rev 5 does not name reads exactly as it did after rev 4.
- The patch's writer note asks for this fourth reading step to be added to "How to read rev 4". That section is not edited, because this file is append-only; this note carries the step instead.
- The patch's closing table ("Changelog: rev 4 → rev 5") is the rev 4 → rev 5 changelog.
- **No file other than this design is changed by this round.** The O1 row in `PHYSICS-ECS-UNIFICATION-DECISIONS.md:89` was applied before pass 4 (the critic confirms it). The P5-§16 list names code and code-doc changes (`boyko_macros`, `component_registry/mod.rs`, `inland_store.rs`, `entity_master.rs`, `ecs_master.rs`, `boyko_threadpool` including the `worker.rs:402-404` doc, and the `boyko_physics` docs on `ContactPairs`, `Manifolds` and `Sensor`). Those are implementation work for the rev-5 build, not edits made here.

# Rev 5 patch: PHYSICS-ECS-UNIFICATION-DESIGN.md (rev 4 → rev 5)

**Provenance.** I re-read every citation below in `D:/wt/joltab` during this round. No cargo was run and nothing was timed. I am read-only, so this patch is text for the writer to append.

**Writer notes.**
- Append "Critique log - pass 4" (the orchestrator's pass-4 JSON, verbatim) after "Changelog: rev 3 → rev 4". Append this patch after that log, and append the closing table as "Changelog: rev 4 → rev 5".
- In "How to read rev 4", the reading order gains a fourth step: rev 2 → `P-§…` → `P4-§…` → `P5-§…`. Each `P5-§…` block quotes, verbatim, the rev-2, rev-3 or rev-4 text it removes.

**Two class removals, plus two classes closed as a side effect.**

| Finding | Class | Mechanism that removes it | Replaces |
|---|---|---|---|
| PB1 (+ NW6, NW7) | Abort reached by two words (FINAL and `poisoned`) and several catch shapes, and poison answered by "return into the program" | **One signal.** `poisoned` is the only abort word, and FINAL is never stored on an abort path. **One catch point.** `run_lane` is one `catch_unwind` per lane frame, lane 0 included. **Leave, don't return.** A poisoned gang op unwinds a private sentinel to `run_lane`. **Barrier over actual completions.** A lane claims only after `completed ≥ p−1` came from a real Complete. | Rev 4's lane-0 drop guard, FINAL on unwind, "`for_each`/`single` return at once" |
| PB2 | A runtime claim with a lifecycle, and no teardown hook to end it | **Authority is a type.** `open_chain`/`close_chain` take `&G::ChainKey`, and only the crate that declares the group can construct that key. There is no runtime claim, so there is nothing to release. | `head_claimed`/`tail_claimed`, the `init_state` claim, "released by the State's Drop" |
| NW3 | An archetype-pointer writer that can be reached outside the binder | Every `InlandStore` method that writes an archetype pointer takes `&BindToken`, `clear` included. `entity_master_mut` becomes `pub(crate)`. | Census only |
| NW4 | A decoder that silently defaults | Explicit arms; an unknown discriminant is a cold panic; the round-trip test is tied to the variant list | `_ => Table` |

Order of blocks: §0, D13, D14, §5, §8, §9, §10.1, §11, §12, §16, §17, Checklist, responses, table.

---

## P5-§0: corrections table (PB2, NW3)

**Removed (rev 4, X-18 fragments, verbatim):**
> `inland_store.rs:285-287` `impl DerefMut for InlandStore {` … `fn deref_mut(&mut self) -> &mut [EntityInland] {`

and

> The only other `&mut self` methods are `ensure` (`inland_store.rs:162`) and `clear` (`:227`)

**Added (in their places):**
> `inland_store.rs:307-309` `impl DerefMut for InlandStore {` … `fn deref_mut(&mut self) -> &mut [EntityInland] {` (and `impl Deref` at `:282`)

and

> The only other `&mut self` methods are `ensure` (`inland_store.rs:162`) and `clear` (`:236`). `clear` is reachable from outside the crate: `entity_master.rs:481-483` `pub fn clear(&mut self) {` … `self.entities_inland.clear();`, through `ecs_master.rs:569` `pub fn entity_master_mut(&mut self) -> &mut EntityMaster {` (NW3)

**Added (two rows after X-18):**
> | X-19 | (pass 4, PB2) A system `State` has no teardown hook with world access | `system_param.rs:142` `fn init_state(world: &mut EcsMaster, system_meta: &mut SystemMeta) -> Self::State;` and `:198` `fn apply(_state: &mut Self::State, _system_meta: &SystemMeta, _world: &mut EcsMaster) {}` are the only world-bearing hooks. `app.rs:126` "no field's Drop observes another" and `:132` `world: EcsMaster,` (first field, by value) | Any per-world runtime state that a State "releases on Drop" either dangles (a pointer into a by-value `EcsMaster`, which drops first) or is never released. Rev 5 keeps no such state (D14, §8 K3). |
> | X-20 | (pass 4, NW1) A claimed wake is owed a scan | `worker.rs:392-395` "a caller that reads its own bit back as CLEAR owes that wake a lane which will actually scan"; `:402-404` `worker_main` may ignore it only because "every one of its post-`mark_idle` exits either runs a task or `continue 'outer`s into a full re-poll"; hand-on shape at `scope.rs:1607-1609` `if unmark_idle(&inner.idle, wid) & self_bit == 0 {` `unpark_one_idle_excluding(inner, self_bit);` | A ticket exit does not scan until FINAL, so it must hand the wake on (§10.1 Take step 3). |

Depends on: D13, D14, §8 K3, §10.1.

---

## P5-§3 D13 (NW2, NW3, NW4)

**Removed (rev 4, verbatim):**
> It sits beside the existing install calls: `boyko_macros/src/component.rs:374-392`, "install this type's derive hooks into `HOOKS[raw]` atomically with ID assignment, before the component can appear in any archetype".

**Added:**
> The derive emits it **immediately after `register_new`** (`component.rs:375` `let raw = …register_new::<Self>();`). It comes before `install_hooks` (`:385`), before `#storage_install` (`:387`), and before every other install, in particular before `#serialize_install` (`:392`).
> - Why: `install_serialize_fn` (`component_registry/serialize.rs:204`) inserts the stable name into the mutex-guarded index that `resolve_stable_name` searches (`serialize.rs:410-414`). That is the one path in the tree that yields `raw` to a thread which did not go through `component_id()`. Because the store precedes that insertion, the index mutex orders `ANCHOR_GROUP[raw]` (a Relaxed store) before every by-name resolution, as well as before every `OnceLock` read of the id.
> - **Standing rule for `component_id()`'s closure:** an install that archetype construction reads precedes every install that publishes `raw`. The rule covers `ANCHOR_GROUP` and the existing `STORAGE_KIND` (`mod.rs:369-373`). Order: `register_new` → `install_anchor` → `install_hooks` → `#storage_install` → … → `#serialize_install`.

**Removed (rev 4, verbatim):**
>   - **deletes `impl DerefMut for InlandStore`**. `Deref` (`inland_store.rs:260`) stays, so the read sites (`[i]`, `.get(i)`, about 130 across 25 files) compile unchanged;

**Added:**
>   - **deletes `impl DerefMut for InlandStore`** (`inland_store.rs:307-309`). `Deref` (`:282`) stays, so the read sites (`[i]`, `.get(i)`, about 130 across 25 files) compile unchanged;

**Removed (rev 4, verbatim):**
>   - reaches `clear` (`:227`) only through the binder's clear;

**Added:**
>   - **`InlandStore::clear` (`:236`) takes `&BindToken`** (NW3). With `set`, that makes every `InlandStore` method that writes an archetype pointer token-gated, so the compiler forces each caller chain through the binder.
>     - `EntityMaster::clear` (`entity_master.rs:481`) takes `&BindToken` and becomes `pub(crate)`.
>     - `EcsMaster::entity_master_mut` (`ecs_master.rs:569`) becomes `pub(crate)`. It has no caller outside `boyko_ecs`: the tree-wide grep finds only `ecs_master.rs:1528`, `load_writer.rs:545/596/604` and `spawn_batch_command.rs:445`.
>     - After this, no `&mut EntityMaster` leaves the crate. The census no longer has to find a `clear` call: it cannot compile.

**Removed:** none (inserted after the rev-4 bullet "**Runtime family** (id-based paths …) … the census makes it red (§17).").

**Added:**
>   - **The decoder does not default (NW4).** `storage_kind()` (`component_registry/mod.rs:390-411`) gets an explicit `Group` arm. Its `_ => StorageKind::Table` (`:409`) becomes a `#[cold] #[inline(never)]` panic, "undecodable storage kind".
>     - Discriminant 0 still decodes to `Table` through its own explicit arm, so `storage_kind_defaults_to_table` (`:1535`) is unchanged.
>     - The out-of-range-id default (`:396-398`) is not a discriminant and stays as it is.
>     - An `ALL_STORAGE_KINDS` array sits next to a private `const fn` that matches every variant exhaustively. Adding a variant is a compile error beside the array, and the round-trip test (§12 U2) iterates the array.
>     - The census exempts `component_registry`, so without this a forgotten decode arm would route every Group id to the table arm silently.

Depends on: X-18, §12 U2, §16, §17.

---

## P5-§3 D14 (PB2)

**Removed (rev 4, verbatim):**
> - **Chain state.** The group's recycle node carries `chain_open: bool`. Two unique params write it (one live claim per world each, §8 K3):

**Added:**
> - **Chain state.** The group's recycle node carries `chain_open: bool`. Two methods write it, and each requires the group's chain key (§8 K3, rev 5):

**Removed (rev 4, verbatim):**
> - **Release authority (W2a).** Only `open_chain`, out-of-chain removal, the first-op flush and `clear` release. `release_dying`, `DenseGroupMut` and `EcsMaster::release_dense_group` are deleted. A user system cannot obtain `GroupHead<PhysicsBody>` while physics holds the claim.

**Added:**
> - **Release authority (W2a, rev 5: a type, not a claim).** Only `open_chain`, out-of-chain removal, the first-op flush and `clear` release. `release_dying`, `DenseGroupMut` and `EcsMaster::release_dense_group` are deleted.
>   - `open_chain` and `close_chain` take `&G::ChainKey`. `#[dense_group(G)]` emits `pub struct <G>ChainKey(())` with a private field and a `pub(crate) const fn new()`, and no `Default`, `Clone` or `Copy`.
>   - The orphan rule fixes `PhysicsBody::ChainKey` to `boyko_physics`'s type, so safe code outside `boyko_physics` cannot open or close the physics chain.
>   - A user system may still hold `GroupHead<PhysicsBody>` for `live_count` and views. The scheduler orders it by its declared writes, and it cannot touch the chain.
>   - **Nothing is claimed at runtime, so nothing is released.** No State caches anything per world beyond the rev-4 rule (ids only). Rebuilding a schedule on the same world (editor play/stop), keeping two schedules on one world, and dropping the App are all ordinary.
>   - Two chains on one world across two schedules run serially, since a schedule run holds the world exclusively. Each opens and closes its own chain.
>   - Two `open_chain` calls inside one schedule would be a bug in `boyko_physics`. The §17 census pins exactly one call site each for `open_chain(` and `close_chain(` in that crate.
>   - **Rejected:** the critic's idempotent claim keyed on system identity. It keeps a per-world table whose entries outlive their systems. It keys on the TypeId of a system, which differs between closure and generic instantiations. It also still panics at schedule build. The key does the same job at compile time, with no state and no panic path.

Depends on: X-19, §8 K3, §9, §12 U2, §17.

---

## P5-§5: data structures (PB1, PB2)

**Removed (rev 4, verbatim, three lines of `GangShared`):**
```rust
    poisoned: AtomicBool,                // SeqCst; ends every claim and every wait (panic protocol)
    payload_claimed: AtomicBool,         // CAS-once owner of `panic`
```
and
```rust
    entry: unsafe fn(*const (), &GangLane<'_>),     // monomorphised by par_phases; 1 call per lane per gang
```

**Added:**
```rust
    poisoned: AtomicBool,                // SeqCst; THE ONLY abort signal. FINAL is never stored on an abort
                                         // path (rev 5, PB1); poison never satisfies a barrier
    payload_claimed: AtomicBool,         // CAS-once owner of `panic`; first real panic of any lane wins
```
and
```rust
    entry: unsafe fn(*const (), &mut GangLane<'_>), // monomorphised by par_phases; called only inside run_lane
```

**Removed (rev 4, verbatim, two fields of `DenseGroupStore`):**
```rust
    head_claimed: bool,                  // one live GroupHead<G> State per world; cleared by the State's Drop
    tail_claimed: bool,                  // one live GroupTail<G> State per world
```

**Added:**
```rust
    // (rev 5) no claim fields: chain authority is the type G::ChainKey (D14), so no per-world state
    // outlives a system State (X-19)
```

**Removed (rev 4, verbatim):**
> On unwind, a drop guard runs the same cancel-and-wait (§10.1).

**Added:**
> Lane 0 runs its program inside `run_lane`'s `catch_unwind`, so it never unwinds out of the frame that owns `GangShared` before the exit wait returns. An abort guard spans that frame from the first publish to the end of the exit wait. Only an unwind from the gang machinery itself (a kernel bug) can reach it, and it aborts the process (§10.1).

Depends on: §10.1, D14.

---

## P5-§8: kernel features (PB2)

**Removed (rev 4, verbatim):**
> - `GroupHead<G>` param (one live claim per world): writes every column id and the recycle node, and reads the slot-map node. It provides `open_chain()`, `live_count()` and typed views.
> - `GroupTail<G>` param (one live claim per world): writes the recycle node. It provides `close_chain()`.
> - A claim is taken in `init_state` and released by the State's Drop. A second live claim panics (cold, at schedule build).

**Added:**
> - `GroupHead<G>` param: writes every column id and the recycle node, and reads the slot-map node. It provides `open_chain(&G::ChainKey)`, `live_count()` and typed views.
> - `GroupTail<G>` param: writes the recycle node. It provides `close_chain(&G::ChainKey)`.
> - `init_state` of either param only calls `ensure_group` (it holds `&mut EcsMaster`). There is no claim, no teardown and no per-world flag (X-19). Authority is the key type, constructible only in the crate that declares the group (D14 rev 5).

---

## P5-§9: public API (PB1, PB2)

**Removed (rev 4, verbatim):**
```rust
pub trait DenseGroup: 'static { const WIDTH: usize; const RELEASE: GroupRelease; type Anchor: Component; }
```

**Added:**
```rust
pub trait DenseGroup: 'static {
    const WIDTH: usize; const RELEASE: GroupRelease;
    type Anchor: Component;
    type ChainKey: 'static;   // #[dense_group] emits `pub struct <G>ChainKey(())` + `pub(crate) const fn new()`
}
```

**Removed (rev 4, verbatim):**
```rust
    pub fn open_chain(&mut self) -> u32;       // K6: flush dying (DEAD + free), chain_open = true; returns slot_bound
```
and
```rust
impl<G: DenseGroup> GroupTail<'_, G> { pub fn close_chain(&mut self); }
```

**Added:**
```rust
    pub fn open_chain(&mut self, key: &G::ChainKey) -> u32; // K6: flush dying, chain_open = true; returns slot_bound
```
and
```rust
impl<G: DenseGroup> GroupTail<'_, G> { pub fn close_chain(&mut self, key: &G::ChainKey); }
```

**Removed (rev 2, verbatim):**
```rust
pub fn par_phases(max_useful_lanes: u32, program: impl Fn(&GangLane<'_>) + Send + Sync);
```
and
```rust
    pub fn for_each(&self, n: u32, body: impl Fn(u32) + Sync); // SPMD; exactly-once; phase barrier
    pub fn single(&self, body: impl Fn() + Sync);
```

**Added:**
```rust
pub fn par_phases(max_useful_lanes: u32, program: impl Fn(&mut GangLane<'_>) + Send + Sync);
```
and
```rust
    pub fn for_each(&mut self, n: u32, body: impl Fn(u32) + Sync); // SPMD; exactly-once; phase barrier.
                                                                  // &mut: a body cannot call a gang op (rev 5)
    pub fn single(&mut self, body: impl Fn() + Sync);
```
> `&mut self` makes "a gang operation inside a chunk body" a borrow error (E0500/E0502). So the sentinel is never raised while a chunk is held, and the per-chunk guard only ever sees a real panic (§10.1). An outer lane captured by a nested program cannot run an outer op either: the nested program is `Fn`, and an `Fn` closure cannot use a captured `&mut` mutably (E0596). The cost is zero, because it is a borrow and not a runtime check.

---

## P5-§10.1: gang protocol (PB1, NW1, NW6, NW7)

**Removed (rev 4, Take step 3, verbatim):**
>   3. If this is the post-`mark_idle` probe: `unmark_idle(&inner.idle, worker_id)` before anything else, as `worker.rs:127` does for a task (W6b).

**Added:**
>   3. If this is the post-`mark_idle` probe: `let was = unmark_idle(&inner.idle, worker_id)` before anything else (W6b). **If `was & self_bit == 0`, call `unpark_one_idle_excluding(inner, self_bit)` before running the lane (NW1).** A claim that cleared this bit spent the pool's wake on a thread that will not scan until FINAL, so the claim is handed on. This is rule B1-P's returning-exit shape (`scope.rs:1607-1609`; X-20).
>      - Cost: one test of a word the RMW already returns. A wake happens only when a claim actually landed.
>      - The other two probes run with the idle bit clear, so no claim can target them.

**Removed (rev 4, Take step 5, verbatim):**
>   5. Run the lane under `catch_unwind` as a **resident** lane. Chunks run under the per-chunk guard (Panic, below).

**Added:**
>   5. Run the lane through **`run_lane`** (Abort protocol, below) as a **resident** lane. `run_lane` is the same function lane 0 uses.

**Removed (rev 4, verbatim):**
> - **Cancel and exit wait (lane 0, after FINAL, and in a drop guard on unwind).**

**Added:**
> - **Cancel and exit wait (lane 0, after its `run_lane` returns).** On the normal path this comes after publishing FINAL. On the abort path it comes without FINAL. There is no drop-guard route.

**Removed (rev 2, verbatim):**
> **Late lane.** `completed == FINAL` → return. Otherwise skip phases with `q < claim.phase`: `claim.phase = q+1` is opened only after `completed ≥ q`, so those phases are complete.

**Added:**
> **Late lane.** `completed == FINAL` or `poisoned` → return, with no program code run. Otherwise skip phases with `q < claim.phase`. Such a phase was opened only by a lane that satisfied the barrier rule (Claim, below), so the phases skipped are complete and were not poisoned.

**Removed:** none (inserted after rev 2's Claim bullets, before "**Complete.**").

**Added:**
> **Barrier rule (rev 5, PB1).**
> - A lane opens phase p (the `phase == p − 1` CAS, or the `n == 0` self-complete) only after two things, in this order: (i) it observed `completed ≥ p−1` through the wait's Acquire load of a value that an actual Complete stored; (ii) it then read `poisoned == false` (Relaxed suffices, because (i) orders it).
> - A chunk of p is claimed only after p was opened. Poison never satisfies (i), because FINAL is never stored on an abort path.
> - Proof that a poisoned phase is never followed:
>   - A chunk guard's `poison()` is sequenced before its Complete `fetch_add(AcqRel)`.
>   - The last completer's `fetch_add` reads from that release sequence, so the poison store happens-before its `completed.store(p−1, SeqCst)`.
>   - A waiter that acquires that store therefore reads `poisoned == true` at (ii).
> - Result: no lane runs phase p over a phase p−1 that contained a panic, and no two lanes run consecutive phases concurrently, on any path.

**Removed (rev 2, verbatim):**
> **SPMD contract.** Every value that decides control flow (each phase's n, loop counts) comes from data no phase writes after its first read, or from a completed `single`. A debug assert checks n against `phase_n`.

**Added:**
> **SPMD contract.** Every value that decides control flow (each phase's n, loop counts) comes from data no phase writes after its first read, or from a completed `single`. A debug assert checks n against `phase_n`: the opener checks after arming its chunk guard, and any other lane checks before its claim CAS. A program must not catch an unwind across a gang operation. A caught `GangAbort` is resumed by the next gang op, because `poisoned` is sticky.

**Removed (rev 4, verbatim, the whole Panic block):**
> **Panic (rev 4, W6c).**
> - **A per-chunk guard on every lane, lane 0 included.** `for_each` and `single` arm a guard before each chunk body and disarm it after. It costs nothing on the normal path; its Drop is `#[cold] #[inline(never)]` and cannot panic. On unwind, the guard's Drop:
>   - `poisoned.store(true, SeqCst)`;
>   - runs the Complete step for its own chunk, so the phase count stays exact;
>   - `bits = parked.swap(0, SeqCst)` and unparks each lane in `bits`. The guard's thread is a participant that has not done its exit RMW, so `GangShared` is alive.
> - Every wait loop exits on `completed >= p || poisoned`. The SeqCst store-then-swap on the poisoner's side and the fetch_or-then-load on the waiter's side extend the W7 argument, so no wake is lost. `for_each` and `single` return at once when `poisoned` is set.
> - **Taken lane:** the unwind reaches the take path's `catch_unwind`. The lane CASes `payload_claimed` (false → true, AcqRel). If it wins, it stores the payload. Then it runs Take step 6.
> - **Lane 0:** its unwind reaches `par_phases`' drop guard. The guard publishes FINAL (`completed.store(FINAL, SeqCst)`, then swap and unpark), cancels, runs the exit wait, drops any stored payload, and continues unwinding with lane 0's own payload.
>   - If lane 0 did not panic but finds `poisoned` set after the exit wait, it calls `resume_unwind` with the stored payload.
> - Without the per-chunk guard, a lane that unwound mid-chunk would leave its phase one short forever, and lane 0's exit wait would wait on lanes stuck in that phase: a hang (W6c).
> - A lane panic never reaches `worker_main`'s abort path (`worker.rs:178-182`).

**Added:**
> **Abort protocol (rev 5; PB1, NW6, NW7).**
> - **One signal.** `poisoned` is the only abort word. FINAL means "lane 0's program returned normally", and only lane 0 on that path stores it. If lane 0 returns normally while `poisoned` is set, FINAL is still true: lane 0 passed every barrier, so every phase completed.
> - **`poison()`** is `poisoned.store(true, SeqCst)`; `bits = parked.swap(0, SeqCst)`; unpark each `threads[i]` in `bits`.
>   - It is idempotent. Its caller is always a participant that has not done its exit RMW, so `GangShared` is alive, and a bit in `parked` implies its `threads` cell is written.
>   - Against a waiter's `parked.fetch_or(SeqCst)` followed by a SeqCst load of `poisoned`, this is the W7 Dekker shape, so no wake is lost.
> - **One catch point per lane frame: `run_lane`.** Lane 0 and every taken lane run `catch_unwind(AssertUnwindSafe(|| entry(program, &mut lane)))`. `AssertUnwindSafe` is sound because the payload is re-raised to the caller unchanged. On `Err(p)`:
>   1. if `p.is::<GangAbort>()`: drop it (a ZST box, which never allocated and whose drop cannot panic);
>   2. otherwise: `poison()`, then `payload_claimed.compare_exchange(false, true, AcqRel, Acquire)`. The winner stores `p` in `panic`. A loser keeps `p` in a local, dropped later (NW6, below);
>   3. return: a taken lane goes to Take step 6, lane 0 to cancel and exit wait.
> - **Poison is observed by leaving, not by returning (NW7).**
>   - Every gang op (`for_each`, `single`) loads `poisoned` (Relaxed) at entry, and again after every wait exit, which is point (ii) of the barrier rule.
>   - If it is set, the op calls `std::panic::resume_unwind(Box::new(GangAbort))`. `GangAbort` is a private ZST, so `Box::new` does not allocate, and `resume_unwind` does not run the panic hook, so nothing is printed.
>   - The sentinel carries the lane straight to `run_lane`. No program code runs after the lane observes poison, so a loop whose exit comes from a `single` cannot spin.
>   - The ops take `&mut self` (§9), so no gang op runs inside a chunk body, and the sentinel never crosses an armed chunk guard.
> - **Per-chunk guard, claim to Complete (PB1).**
>   - The guard is a local constructed before the claim loop, with `held = None`. The successful claim CAS is followed immediately by `held = Some(idx)`, a plain local store; no call and no assertion sits between them.
>   - The opener's `phase_n` record and assert, the body, and every other step come after it. Complete clears `held` after its `fetch_add`.
>   - The Drop, reached only by unwinding, is `#[cold] #[inline(never)]` and cannot panic. If `held` is `Some`, it runs `poison()`, then Complete for that chunk. If `None`, it does nothing, because `run_lane` poisons.
>   - So a panic anywhere between a claim and its Complete, in the body or in machinery, completes the chunk exactly once.
>   - Cost on the normal path: one local store per chunk and one compare at scope end.
> - **Wait exit.** A wait loop exits on `completed >= p || poisoned`. This is followed by (ii); if that reads poisoned, the op raises `GangAbort`.
> - **Machinery outside `run_lane` does not unwind.** This covers lane 0's publish, cancel, exit wait and `Thread` drops, and the taker's steps 2–4 and 6. None of them contains user code or a panicking assertion.
>   - Backstop: an abort guard spans lane 0's frame from its first `in_use` claim to the end of step 4 of the exit wait, and spans the taker from its winning `fetch_and` to the end of step 6.
>   - An unwind that reaches either guard aborts the process with a new `boyko_log` code, in the pattern of `abort_on_task_panic` (`worker.rs:187-213`). Unwinding past it would pop `GangShared` under live takers.
>   - Zero cost on the normal path: `mem::forget` at the end of the span.
> - **Losing payloads (NW6).** A taken lane drops its losing payload **after** step 6, and lane 0 drops its own after the exit wait. The drop runs inside an abort-on-unwind guard: a payload whose `Drop` panics is a double panic and aborts, as std does. The payload is the lane's own box, not gang memory, so NB1's "nothing after the RMW touches the gang" still holds.
> - **Re-raise.** After the exit wait, lane 0 re-raises with `resume_unwind` if `payload_claimed` is set. Ordering:
>   - A taker's store to `panic` precedes its Release exit RMW, and lane 0 Acquire-read the final `exit_count`. Lane 0's own store is program-ordered.
>   - Exactly one payload reaches the caller: the first real panic of any lane.
>   - `debug_assert!(poisoned.load(Relaxed) == payload_claimed.load(Relaxed))` after the exit wait. Every `poison()` is on a real unwind, which reaches `run_lane` and claims or loses before its lane's exit RMW.
> - **Termination of the exit wait on the abort path.** Every taken lane returns, for one of four reasons:
>   - it was in a chunk: the chunk finishes and the next op raises;
>   - it was parked in a wait: `poison()`'s swap unparks it, or its SeqCst re-check sees the poison;
>   - it enters late: the entry check returns;
>   - its ticket was never taken: cancel removes it.
> - Nested gangs are unchanged: an inner gang's re-raised payload is a real panic in the outer chunk, so the outer chunk guard poisons the outer gang. A lane panic never reaches `worker_main`'s `run_task` abort (`worker.rs:178-182`).

Depends on: X-16, X-20, §5, §9, §11, §12 P2, §17.

---

## P5-§11: multithreading (PB1)

**Removed (rev 2, fragment, verbatim):**
> `GangLane` is `!Send`.

**Added:**
> `GangLane` is `!Send`, and every gang op takes `&mut self` (§9 rev 5). `poisoned` is written only by `poison()`: SeqCst store, then SeqCst swap on `parked`. It is read at op entry and at barrier point (ii). FINAL is stored only on lane 0's normal return. Neither the barrier nor the exit wait depends on FINAL on an abort path (§10.1 rev 5).

---

## P5-§12: gates (PB1, PB2, NW4)

**Removed (rev 4, U2 fragment, verbatim):**
> **`#[should_panic]`:** a second live `GroupHead<G>` or `GroupTail<G>` claim; a type that is both `Component` and `GroupColumn`; `dense_registry_mut().store_mut(<group column id>)`.

**Added:**
> **`#[should_panic]`:** a type that is both `Component` and `GroupColumn`; `dense_registry_mut().store_mut(<group column id>)`; `storage_kind` on an id whose `STORAGE_KIND` byte a test module has set to an undecodable value (NW4).
> **Trybuild (PB2):** from a foreign crate, `PhysicsBodyChainKey::new()` (E0624) and `PhysicsBodyChainKey(())` (E0603/E0423, private field) fail, so `open_chain`/`close_chain` for `PhysicsBody` do not compile outside `boyko_physics`.
> **Schedule rebuild (PB2), a GATE:** on one world, build a schedule containing the physics chain and run 3 steps. Drop it, build a second one and run 3 steps. Then keep two live schedules alternating for 3 steps each. Expect no panic, `open_chain`/`close_chain` strictly alternating (a debug counter), and the App dropped at the end. Mutation run: a per-world claim flag set in `init_state` and never cleared must turn it red.
> **Round-trip (NW4):** `set_storage_kind_round_trips_group` beside `set_storage_kind_round_trips_dense` (`component_registry/mod.rs:1591-1602`), and one test iterating `ALL_STORAGE_KINDS`.
> **Inland `clear` (NW3):** covered by the compiler, because `InlandStore::clear` and `EntityMaster::clear` take `&BindToken` and `entity_master_mut` is `pub(crate)`.

**Removed:** none (appended to the rev-4 P2 row's gate cell, after "the payload re-raised exactly once, and the pool still has W workers;").

**Added:**
> **abort paths (PB1, NW6, NW7)** at W ∈ {2,4,8}, each with a watchdog and each expecting the payload re-raised exactly once and W workers afterwards:
> - (a) lane 0 panics **between** two `for_each` calls;
> - (b) a taken lane panics **outside** a chunk;
> - (c) a panic injected by a `#[cfg(test)]` hook immediately after `held = Some(idx)`;
> - (d) a program whose loop exit comes only from a `single`, poisoned mid-loop: it must terminate;
> - (e) two lanes panic in one phase: the loser's payload is dropped, and exactly one payload is re-raised;
> - (f) a losing payload whose `Drop` panics: the process aborts. This is a child-process test that re-invokes the test binary with an env var and expects an abnormal exit.
>
> **Phase-overlap detector:** each chunk of phase p increments `active[p]`, asserts `active[p−1] == 0 && active[p+1] == 0`, then decrements; violations are counted and must be 0 on every schedule above.
> - Schedule (a) must be deterministic: latches ensure two taken lanes have entered, and one phase-(q+1) chunk body is held on a latch that releases only after a test-only probe observes the abort signal.
> - **Mutation run:** rev 4's text (lane 0's guard stores FINAL without `poison()`, and ops return on FINAL) must make the detector red under (a). Deleting `poison()` from `run_lane` must make (d) hang to the watchdog.

---

## P5-§16: integration (PB1, PB2, NW1–NW5)

**Removed (rev 4, verbatim):**
> - `boyko_macros`: `#[dense_group]` (the `GroupColumn` impls and the exclusivity probe); `#[component(anchor_group = …)]` emits `install_anchor` inside `component_id()`'s `get_or_init` (`component.rs:374-392`).

**Added:**
> - `boyko_macros`: `#[dense_group]` emits the `GroupColumn` impls, the exclusivity probe, and `<G>ChainKey` with a `pub(crate)` constructor. `#[component(anchor_group = …)]` emits `install_anchor` immediately after `register_new` (`component.rs:375`), before `install_hooks` (`:385`), `#storage_install` (`:387`) and `#serialize_install` (`:392`).
> - `component_registry/mod.rs`: the `storage_kind` decode gets a `Group` arm, a cold panic on an unknown discriminant (replacing `:409`), and `ALL_STORAGE_KINDS`.
> - `entity/inland_store.rs`: `clear(&mut self, &BindToken)` (`:236`). `entity/entity_master.rs`: `clear(&BindToken)` becomes `pub(crate)` (`:481`). `ecs_master.rs:569`: `entity_master_mut` becomes `pub(crate)`.
> - `boyko_threadpool`: `run_lane`, `poison()`, `GangAbort`, the claim-to-Complete chunk guard, the two abort guards, `&mut self` gang ops, and the NW1 hand-on at the post-`mark_idle` take. The doc at `worker.rs:402-404` ("`worker_main` ignores it and is right to") is amended: the ticket exit reads the word and hands the claim on.
> - `boyko_physics`: S1 calls `open_chain(&PhysicsBodyChainKey::new())` and S6 calls `close_chain(&…)`. Docs (NW5): `ContactPairs` (`resources.rs:537`) and `Manifolds` (`resources.rs:2244`) state "slot-keyed; a slot names its current tenant only until the next structural window after S6". The `Sensor` doc (`components.rs:179-185`), which calls `Manifolds::sensor_overlaps` "the per-step overlap signal", is re-pointed at D6's sensor events.

---

## P5-§17: validation (PB1, PB2, NW5, NW8)

**Removed (rev 4, verbatim):**
> - **panic:** a lane panics mid-chunk, on lane 0 and on a taken lane. Expect no hang, the other lanes to exit, and lane 0 to re-raise once after the exit wait.

**Added:**
> - **abort:** the §12 P2 list (a)–(f) plus mid-chunk panics on lane 0 and on a taken lane, all under the phase-overlap detector. Loom adds one schedule: a `poison()` caller racing a waiter's `fetch_or`/re-check, which must not lose the wake.

**Removed (rev 4, verbatim):**
> A `#[cfg(miri)]` yield burst follows the taker's `exit_count` RMW. It runs under Tree Borrows with several seeds. **It must be red on a mutant that performs one gang load after the RMW** (rev 3's `exit_waiting.load`), and green on rev 4.

**Added:**
> Inside the taker the order is fixed: the `exit_count` RMW, then the `#[cfg(miri)]` yield burst, then (mutant only) one gang load. The burst lets lane 0 finish its exit wait and pop its frame before the load; that is what makes the mutant's load a use of freed memory. A mutant with the load **before** the burst can stay green and does not count as the red receipt (NW8). The precedent's placement was not decisive (`scope.rs:882-901`: red at both placements there), so the receipt records the placement and the seeds on which it was red.
> - The gate runs under Tree Borrows with several seeds, and must be green on rev 5.
> - A second variant runs the abort path: one taken lane panics in a chunk and wins the payload, and a second loses and drops its payload after step 6. It must be green, which shows that the losing drop touches no gang memory.

**Removed (rev 4, verbatim):**
> - **census of persistent slot-keyed state:** `PairCache` and `ContactEventState`. Any new resource keyed by body slot must be added to this list and must apply the fresh rule. This is enforced by review; it is listed next to §4's table.

**Added:**
> - **census of slot-keyed state**, in two classes, listed next to §4's table and enforced by review:
>   - *persistent across steps; must apply the fresh rule:* `PairCache`, `ContactEventState`;
>   - *per-step scratch; valid until the next structural window after S6* (NW5): `ContactPairs`, `Manifolds` (including `sensor_overlaps`). After that window, an out-of-chain despawn plus spawn can re-tenant a slot (D15 case B). Gameplay reads contacts through D6's events. No in-tree consumer maps these slots today; only tests read counts or contents.
> - **census, chain calls (PB2):** `open_chain(` and `close_chain(` each appear at exactly one call site in `boyko_physics/src`.

**Removed:** none (appended to the rev-4 mutation-runs bullet).

**Added:**
> ; rev 4's abort text (FINAL without `poison()`, ops returning on FINAL) (phase-overlap detector, schedule (a)); `poison()` deleted from `run_lane` (schedule (d)); a never-cleared per-world claim (schedule-rebuild gate); the gang load placed after the burst (Miri; before the burst is not a receipt).

---

## P5-Checklist: edge cases

**Removed (rev 4, verbatim):**
>   - a second `GroupHead` / `GroupTail` claim (refused);

**Added:**
>   - `open_chain` / `close_chain` from a crate that does not declare the group (does not compile);
>   - a schedule rebuilt on the same world, two schedules on one world, and the App dropped with physics (no claim, nothing to release);

**Removed (rev 4, verbatim):**
>   - a lane panicking mid-chunk, on lane 0 and on a taken lane (W6c);

**Added:**
>   - a lane panicking mid-chunk, between phases, or between a claim CAS and its body, on lane 0 and on a taken lane (W6c, PB1);
>   - a poisoned program whose loop exit comes from a `single` (NW7);
>   - a losing panic payload whose `Drop` panics (NW6: abort);
>   - an undecodable `StorageKind` byte (NW4: cold panic);

---

## Responses without a section edit

- **PB1, which of the critic's two options.** Neither exactly. Both left FINAL meaning two things. Rev 5 removes FINAL from every abort path, so `completed ≥ p` again means only "p actually completed". It makes `poisoned` the sole abort word, set at one catch point that lane 0 shares with the taken lanes. This subsumes the critic's option 1 and makes option 2 unnecessary.
  - The critic's two taken-side gaps close by construction. A taken lane's out-of-chunk panic poisons in `run_lane`, and lane 0 re-raises on `payload_claimed`, not on `poisoned`. The claim-to-body gap closes because the guard is armed by the CAS itself.
  - Honest scope: poisoning on an out-of-chunk panic is for promptness, not for barrier correctness. Such a lane holds no chunk, and the barrier rule does not depend on it. The overlap detector and the mutation runs are what gate correctness.
- **PB2, why a key rather than a released claim.** X-19 shows the kernel has no world-bearing teardown, and adding one (a `SystemParam::teardown` run by `Schedule::drop` with a world it does not own) would reintroduce the by-value-App ordering hazard. Authority by type privacy is compile-time, costs zero bytes, and needs no release. The one property rev 4's claim gave beyond authority, "one chain per world", is not a kernel invariant: serial chains on one world are sound, and the physics crate's own single call site is pinned by census.
- **NW1:** accepted as specified by the critic, in the B1-P shape.
- **NW2:** accepted, and generalised into a standing ordering rule for the closure.
- **NW3:** accepted and taken further than the census. The token goes on the store's `clear`, and `entity_master_mut` loses `pub`.
- **NW4:** accepted; the class is removed (no silent default).
- **NW5:** ACCEPT-AS-DOCUMENTED. The class-removing alternative is making `ContactPairs`/`Manifolds` crate-private. That would break 10 external test files in `boyko_physics/tests` that read them (grep this round), for a hazard with no in-tree consumer. The census class and the resource docs are the proportionate fix.
- **NW6:** folded into PB1 (losing payloads are dropped after step 6, under an abort guard).
- **NW7:** folded into PB1 (leave-by-sentinel).
- **NW8:** accepted.
- Every fix the critic confirmed (NB1, NB2, NB3, chain_open, W3–W7, O1) is untouched by this patch. NB1's "nothing after the RMW touches the gang" is strengthened on the abort path: lane 0 no longer unwinds through the frame that owns `GangShared`.

---

## Changelog: rev 4 → rev 5

| Finding | Action | Section |
|---|---|---|
| PB1: FINAL published on unwind without `poisoned`, so barriers pass at once; taken-lane out-of-chunk panic swallowed; claim-to-guard gap | FIX, class removed. `poisoned` is the only abort word, and FINAL is only stored on normal return. `run_lane` is one `catch_unwind` per lane frame, lane 0 included; it replaces the lane-0 drop guard. The barrier rule is stated over actual completions, with a happens-before proof. The chunk guard is armed by the claim CAS. Gang ops take `&mut self`, so no op runs inside a chunk. Abort guards back the machinery. P2 gets abort schedules (a)–(f), a deterministic phase-overlap detector, and mutation runs against rev 4's text | §5, §9, §10.1, §11, §12 P2, §17, Checklist |
| PB2: claim "released by the State's Drop", with no world and a pointer ban | FIX, class removed. `open_chain`/`close_chain` take `&G::ChainKey`, a key constructible only in the declaring crate. `head_claimed`/`tail_claimed` are deleted, and there is no runtime claim or release. X-19 records the missing teardown. The `#[should_panic]` claim gate is replaced by trybuild, plus a schedule-rebuild GATE with a mutation run. Call-site census | §0 X-19, D14, §5, §8 K3, §9, §12 U2, §16, §17, Checklist |
| NW1: ticket take at the post-`mark_idle` probe spends a claimed wake | FIX. Read `unmark_idle`'s word and hand on with `unpark_one_idle_excluding` (B1-P shape); amend the `worker.rs:402-404` doc | §0 X-20, §10.1 Take 3, §16 |
| NW2: `install_anchor` position vs raw publication | FIX. Emitted right after `register_new`, before every install, including `#serialize_install`. Standing ordering rule for the closure; the index mutex orders it before by-name resolution | D13, §16 |
| NW3: `EntityMaster::clear` reachable without the binder; X-18 citations drifted | FIX, class removed. `InlandStore::clear` and `EntityMaster::clear` take `&BindToken`; `entity_master_mut` becomes `pub(crate)` (no external caller). Citations corrected to `:282`, `:307-309`, `:236` | §0 X-18, D13, §12 U2, §16 |
| NW4: `storage_kind` `_ => Table` fallback inside the census exemption | FIX, class removed. Explicit `Group` arm; cold panic on an unknown discriminant; `ALL_STORAGE_KINDS` with an exhaustive `const fn` guard; Group round-trip test | D13, §12 U2, §16, Checklist |
| NW5: per-step slot-keyed scratch readable after re-tenancy | ACCEPT-AS-DOCUMENTED. Second census class, docs on `ContactPairs`/`Manifolds`, and the `Sensor` doc re-pointed at D6 events. Making them crate-private was rejected (10 external test files) | §16, §17 |
| NW6: losing payload dropped before Take step 6 | FIX, folded into PB1. Dropped after step 6 (or after lane 0's exit wait) under an abort-on-unwind guard; child-process test | §10.1, §12 P2 (f), Checklist |
| NW7: SPMD loop driven by `single` spins under poison | FIX, folded into PB1. Poisoned ops unwind the `GangAbort` sentinel to `run_lane` and never return into the program; test (d) | §10.1, §12 P2 (d), Checklist |
| NW8: Miri mutant placement | FIX. Order fixed as RMW → burst → load; a before-burst mutant is not a receipt; the receipt records placement and seeds; abort-path variant added | §17 |

**Sections whose invariants this patch depends on (critic re-read list):**
- **§10.1** depends on `worker.rs:84-146`, `:178-213`, `:389-416`, `:494-518` and `scope.rs:1569-1611`, `:878-904`, plus rev 2's Claim, Complete and Wait (`:577-599`).
- **D14 and §8 K3** depend on `system_param.rs:142`, `:198` and `app.rs:125-132`.
- **D13** depends on `boyko_macros/src/component.rs:369-395`, `component_registry/mod.rs:369-411` and `:920-947`, `serialize.rs:204` and `:410-414`, `inland_store.rs:162`, `:236`, `:282`, `:307-309`, `entity_master.rs:477-485`, and `ecs_master.rs:567-571`.
- **§17 NW5** depends on `boyko_physics/src/resources.rs:537`, `:2244` and `components.rs:179-185`.

Primary files, all in `D:/wt/joltab/crates/`:
- `boyko_threadpool/src/worker.rs`
- `boyko_threadpool/src/scope.rs`
- `boyko_ecs/src/ecs/core/system/system_param.rs`
- `boyko_ecs/src/ecs/core/app/app.rs`
- `boyko_macros/src/component.rs`
- `boyko_ecs/src/ecs/core/component/component_registry/mod.rs`
- `boyko_ecs/src/ecs/core/component/component_registry/serialize.rs`
- `boyko_ecs/src/ecs/core/entity/inland_store.rs`
- `boyko_ecs/src/ecs/core/entity/entity_master.rs`
- `boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`
- `boyko_physics/src/resources.rs`
- `boyko_physics/src/components.rs`

The design document is `D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md`.

# Critique log - pass 5 (2026-09-11)

Pass 5 was scoped to the rev-5 patch. The critic's output is reproduced below verbatim: the verdict, every non-blocking finding, and the fixes it confirmed. There are no blocking findings. Each line break inside the verdict is the critic's own; the empty quote lines between paragraphs are added only so the Markdown renders them as separate paragraphs. No patch follows this section: the design closes at rev 5 (see the status section below).

## Verdict

> APPROVED (pass 5, scoped to the rev-5 patch). Nothing blocks, so the design closes at rev 5. Five small non-blocking items remain. They are errata and implementation notes, not a reason for a rev-6 round.
>
> (1) PB1 is closed on every path I traced; no two phases can run concurrently. Each path in turn:
> - Inside a chunk body, on lane 0 or a taken lane: the chunk guard runs poison() before its AcqRel Complete. The release-sequence proof (design :3501-3503) holds, including across the `done[(p+1)&1]` Relaxed reset, because that reset precedes every fetch_add of phase p+1 in modification order. So any lane that opens p+1 reads `poisoned == true` at its point (ii).
> - Between the claim CAS and the body: the guard is armed by a plain store right after the CAS (:3541). The n==0 opener holds no chunk, but a panic there leaves the phase uncompleted, and the poison exit ends every wait.
> - Outside a chunk (SPMD code between ops, the non-opener's SPMD assert, wait machinery): the lane holds no chunk, so nothing is orphaned. Point (i) can only be satisfied by a real completion, because FINAL is never stored on an abort path (:3499, :3527). Later phases can still run, but only one after another (the honest scope at :3676).
> - During a barrier: every wait exits on poison. poison()'s store-then-swap against the waiter's fetch_or-then-SeqCst-re-check is the W7 Dekker pair. The GangAbort sentinel is raised only at op entry and after a wait, where `held` is None, so it never crosses an armed chunk guard.
> - During FINAL publication: it happens only after run_lane returns Ok, when lane 0 has passed every barrier. It is machinery under the abort guard, so it cannot unwind past GangShared.
> - In a nested gang: the inner run_lane catches the inner sentinel, and the inner re-raise reaches the outer chunk guard as a real panic. An outer op cannot be reached from an inner program or body (E0596/E0500, and `&mut GangLane` is !Send).
> - `poisoned` implies `payload_claimed` on every path I traced. The one exception is a program that catches an unwind, which the contract forbids (NB3 below).
>
> (2) PB2 is closed. There is no runtime claim left, so there is nothing to release on a normal return, a panic, an early return or a schedule teardown:
> - A State holds only ids (design :2367, :3340).
> - `Schedule::run(&mut self, world: &mut EcsMaster)` (schedule.rs:286) serialises chains across schedules.
> - Coherence plus the orphan rule pin PhysicsBody::ChainKey to boyko_physics.
> - The only per-world lifecycle state left is `chain_open`, which the world owns. If it is stuck open after a panic or an early return, removals are deferred, which is the safe direction. The next open_chain releases them and raises a diagnostic, not an assert (:2497).
> - The schedule-rebuild GATE and its mutation run turn any claim implementation red.
>
> (3) Every other P5 block and all 8 NW responses hold against D:/wt/joltab, and every citation I re-read matches. Evidence is from D:/wt/joltab @ d11962a9; no cargo was run and nothing was timed.

## Blocking findings

None.

## Non-blocking findings

- NB1 - Stale claim text survives rev 5, because P5-§9 and P5-§12 did not remove it. WHERE: in §9, read in the prescribed order rev 2 -> P -> P4 -> P5, the struct declarations still read design :2776 `pub struct GroupHead<'w, G: DenseGroup>;       // SystemParam; one live claim per world (W2a)` and :2782 `pub struct GroupTail<'w, G: DenseGroup>;       // SystemParam; one live claim per world`. P5-§9 replaced only the open_chain/close_chain lines and the DenseGroup trait. The U2 content cell (:2981) still lists '`GroupHead` / `GroupTail` claims'; P5-§12 removed only the #[should_panic] fragment. CONSEQUENCE: someone implementing from §9 or the U2 row gets a rule that D14 rev 5 (:3340) and §8 (:3404, 'There is no claim, no teardown and no per-world flag') deny. It is not silent: the schedule-rebuild GATE and its mutation run (:3585) turn a claim implementation red. CONFIDENCE: CONFIRMED (grep 'claim' over the design). FIX: one erratum line, appended with this log, that deletes both comments and the U2 list item.
- NB2 - The chain key controls who can CALL open_chain/close_chain, not who can SCHEDULE the physics systems that call them. The rev-5 response 'Two open_chain calls inside one schedule would be a bug in boyko_physics' (:3342) is true only if S1 and S6 cannot be scheduled from outside the crate. boyko_physics exports its systems publicly: lib.rs:82 `pub mod systems;`, the re-export at :111, and systems.rs:136 `pub fn physics_integrate(`. A new S1 or S6 written in that house style would be pub. CONSEQUENCE: a user adds the S6 fn to the physics schedule a second time, unordered, and it lands between S1 and S5. close_chain then runs mid-chain, and a window in that round releases and LIFO-re-tenants a slot that S2's pairs still name. S6 then writes that slot's pose back to the new tenant (a one-frame wrong pose). A second S1 only raises the debug diagnostic (:2497). This is wrong, not unsound: DEAD slots are inert by design (§10.8), and the colouring is indexed by slot. Rev 4's claim refused this at build; rev 5 dropped that check without replacing it. CONFIDENCE: pub export CONFIRMED; S1/S6 visibility PLAUSIBLE, since the design never states it. DIRECTION: make the two call-site systems pub(crate) and register them only through the physics plugin, whose uniqueness is already enforced (app.rs:547, 'Panics (`boyko-B1801`) if a plugin of the same type was already added'). Also, the census pins call sites, not exits, so require every non-panicking S6 exit to reach close_chain (an early return that skips it leaves the chain open and fires the diagnostic every step).
- NB3 - `poisoned` can be set with no payload claimed, and release builds then report success. CONSEQUENCE: suppose a program wraps a gang op in catch_unwind(AssertUnwindSafe(...)). The contract forbids it (:3510), but it compiles. A chunk panic is then swallowed after its guard poisoned. `payload_claimed` stays false, lane 0 re-raises only on `payload_claimed` (:3552), and in release par_phases returns normally from an aborted gang. Only the debug_assert at :3555 catches it. CONFIDENCE: CONFIRMED from the patch text, on a path the contract forbids. DIRECTION: turn the check into a cold release assert, or re-raise a synthetic payload when `poisoned && !payload_claimed`. The cost is one branch on the abort path only.
- NB4 - The claim that a gang op inside a chunk body is a borrow error (:3462) assumes something the patch does not state: GangLane is neither Clone nor Copy, and it is !Sync. P5-§11 states only '!Send' and '&mut self'. CONSEQUENCE: if GangLane were Copy and Sync, a body could capture a copy and run `let mut l = copy; l.single(..)` inside an `Fn + Sync` closure. That runs a nested barrier wait while holding a chunk (a hang) and lets the sentinel cross an armed chunk guard. CONFIDENCE: PLAUSIBLE, because the natural implementation (a raw-pointer PhantomData, no derive) is already safe. DIRECTION: state the premise in §11, and add one trybuild fixture that calls a gang op inside a body.
- NB5 - Small implementation errata. (a) Two in-crate unit tests call `clear` without a token and are not listed in P5-§16: entity_master.rs:988 `em.clear();` (test `clear_resets_live_count`, :979) and inland_store.rs:484 `clear_keeps_commit`. They need a test-only way to get a token; the compiler will flag both. (b) `ALL_STORAGE_KINDS` (:3317): the exhaustive const fn forces an edit to that fn, not to the array, so 'adding a variant is a compile error beside the array' overstates it. This is harmless, because the `_` arm now panics, so a missing decode arm fails loudly on first decode. (c) The storage_kind doc, whose '# Panics' section says 'Never.' (mod.rs:385-388), must be updated with the cold panic.

## Fixes the critic confirmed

- PB1 RESOLVED. The abort paths now use one signal and one catch point per lane. FINAL is stored only on lane 0's normal return, when lane 0 has passed every barrier. Barrier point (i) accepts only real completions. A chunk panic's poison happens-before the store that ends its phase, because it travels in the `done` release sequence, which the parity reset does not break. So no successor of a phase that contained a panic is ever opened, and consecutive phases never overlap on any path: in a chunk, out of a chunk, between the claim CAS and the body, in a barrier, during FINAL, or in a nested gang. A taken lane's out-of-chunk panic now poisons in run_lane, and lane 0 re-raises on `payload_claimed`. The exit wait terminates for all four taken-lane states listed at :3556-3560. The phase-overlap detector's schedule (a) is realisable with latches, and the rev-4 mutant (FINAL without poison) opens q+2 while a q+1 chunk is still held, so the gate can fail.
- PB2 RESOLVED. Authority is a type: `&G::ChainKey`, whose constructor is pub(crate). Coherence and the orphan rule fix PhysicsBody's key to boyko_physics. The claim fields are deleted, and init_state only calls ensure_group. X-19 matches the code: system_param.rs:142 `fn init_state(world: &mut EcsMaster, ...)` and :198 `fn apply(...)` are the only hooks that receive the world, and app.rs:126/:132 are exactly as cited. Serial chains across schedules are guaranteed by schedule.rs:286 `pub fn run(&mut self, world: &mut EcsMaster)`. The trybuild fixture, the schedule-rebuild GATE and its mutation run replace the incoherent #[should_panic] test.
- NW1 RESOLVED. At the post-mark_idle take, the RMW's word is tested and the wake is handed on, in exactly the B1-P shape (scope.rs:1607-1609 `if unmark_idle(&inner.idle, wid) & self_bit == 0 {` / `unpark_one_idle_excluding(inner, self_bit);`). The other two probes run with the idle bit clear, as the code shows: the bit is set only between worker.rs:121 and :127/:135/:142. A hand-on that follows a gang's own publish wake is useful, not wasted, and it cannot cascade. The doc at worker.rs:402-404 is correctly scheduled for amendment.
- NW2 RESOLVED. install_anchor is emitted right after `register_new` (component.rs:375), before :385, :387 and :392. register_new publishes `raw` only through NEXT_ID/LAYOUTS (mod.rs:920-947), and the only reverse lookup returns a bool. So the one path that hands `raw` to a thread which did not call component_id() is resolve_stable_name, and it takes the index mutex (serialize.rs:410-414). The standing ordering rule is sound.
- NW3 RESOLVED. The citations now match: Deref at inland_store.rs:282, DerefMut/deref_mut at :307-309, clear at :236, ensure at :162, EntityMaster::clear at entity_master.rs:481-483, entity_master_mut at ecs_master.rs:569. A tree-wide grep finds entity_master_mut only in ecs_master.rs:1528, load_writer.rs:545/596/604 and spawn_batch_command.rs:445, all inside boyko_ecs, so pub(crate) breaks no external caller. Once the token is on clear, no &mut EntityMaster leaves the crate.
- NW4 RESOLVED. The silent `_ => StorageKind::Table` at mod.rs:409 becomes a cold panic, with an explicit Group arm. Discriminant 0 and the out-of-range default (:396-398) are kept, so `storage_kind_defaults_to_table` (:1535) is unaffected. A Group round-trip test is added next to :1591.
- NW5 ACCEPT-AS-DOCUMENTED is proportionate. No in-tree consumer maps these slots. Both resource sites and the Sensor doc match the code: resources.rs:537, :2244 and components.rs:179-185, where the doc calls `sensor_overlaps` 'the per-step overlap signal'. The census gets a second class for them.
- NW6 RESOLVED. A losing payload is dropped after step 6, or after lane 0's exit wait, under an abort-on-unwind guard. The payload is `'static` and heap-owned, so dropping it touches no gang memory, and NB1 still holds. Test (f) runs as a child process.
- NW7 RESOLVED. Poison is left by unwinding the private ZST sentinel to run_lane, never by returning into the program. `&mut self` ops keep the sentinel from ever crossing an armed guard, and test (d) plus the 'poison() deleted from run_lane' mutation gate it.
- NW8 RESOLVED. The order is fixed as RMW, then the yield burst, then the mutant's load. A mutant with the load before the burst does not count as a red receipt, the receipt records placement and seeds, and an abort-path Miri variant is added.

## Erratum E1 (NB1)

The critic's NB1 fix is "one erratum line, appended with this log". This file is append-only, so the erratum is stated here and the lines it names are left in place. **Read §9 and the §12 U2 row as if the following text were absent:** the trailing comments `// SystemParam; one live claim per world (W2a)` on the `GroupHead` declaration (:2776) and `// SystemParam; one live claim per world` on the `GroupTail` declaration (:2782) (both types remain `SystemParam`s); and the list item "`GroupHead` / `GroupTail` claims;" in the U2 content cell (:2981). D14 rev 5 (:3340) and P5-§8 K3 (:3404, "There is no claim, no teardown and no per-world flag") are authoritative. The quoted-and-removed rev-4 claim text inside the rev-5 patch (for example :3327, :3374-3375, :3397-3399, :3580) is historical, not live.

# Status: closed at rev 5 (2026-09-11)

- **The design is closed at rev 5, after five critique passes.** Pass 5 approved the rev-5 patch with no blocking findings, and no rev-6 round is planned.
- **Blocking findings per pass: 3 / 5 / 3 / 2 / 0.**
  - Pass 1 (rev 1): C1, C2, C3.
  - Pass 2 (rev 2): B1-B5.
  - Pass 3 (rev 3 patch): NB1-NB3.
  - Pass 4 (rev 4 patch): PB1, PB2.
  - Pass 5 (rev 5 patch): none.
- **Every earlier blocker was confirmed fixed by a later pass.**
  - Pass 2 confirmed C1 fixed and the C2 and C3 approaches correct. The remainders of C2 and C3 became pass-2 blockers (B2, from the C2 and W3 fix; B1, the rest of C3).
  - Pass 3 confirmed B1-B5 resolved in mechanism. Its caveats became pass-3 items: the group-column part of B1 became NB2, the `clear()` part of B3 became W1, and the B4 caveats became W3 and W4.
  - Pass 4 confirmed NB1, NB2 and NB3 resolved, along with chain_open (W1/W2, apart from PB2) and W3-W7.
  - Pass 5 confirmed PB1 and PB2 resolved, together with NW1-NW8 (NW5 as accept-as-documented).
- **Reading order for any section: rev 2 → `P-§…` → `P4-§…` → `P5-§…`.** Start from rev 2 and apply, in turn, the rev-3, rev-4 and rev-5 patch blocks for that section where they exist. A section a later patch does not name reads as it did after the earlier one. Erratum E1 above applies on top of that order.
- **Open non-blocking findings from pass 5.** None blocks, and each is to be handled during implementation:
  - NB1: stale claim comments in §9 (:2776, :2782) and the U2 content cell (:2981). They are handled textually by Erratum E1.
  - NB2: the chain key gates calls to `open_chain`/`close_chain` but not the scheduling of the systems that make those calls. Make S1 and S6 `pub(crate)`, register them only through the physics plugin, and require every non-panicking S6 exit to reach `close_chain`.
  - NB3: `poisoned` without `payload_claimed` returns success in release when a program catches an unwind, which the contract forbids. Use a cold release assert, or re-raise a synthetic payload.
  - NB4: state in §11 that `GangLane` is neither `Clone` nor `Copy` and is `!Sync`, and add a trybuild fixture for a gang op inside a body.
  - NB5: (a) test-only token access for `clear_resets_live_count` (entity_master.rs:979/988) and `clear_keeps_commit` (inland_store.rs:484); (b) the `ALL_STORAGE_KINDS` compile-error wording overstates what the check does; (c) the `storage_kind` doc's `# Panics` section (mod.rs:385-388) needs updating.
- **The code changes that the rev-5 patch lists in P5-§16 are implementation work, not edits to this document.** That list covers `boyko_macros`, `component_registry/mod.rs`, `inland_store.rs`, `entity_master.rs`, `ecs_master.rs`, `boyko_threadpool` including the `worker.rs:402-404` doc, and the `boyko_physics` docs on `ContactPairs`, `Manifolds` and `Sensor`. None of it was applied in this round. The evidence for every pass comes from `D:/wt/joltab` (merge/ke16-into-ecsnative @ d11962a9). No cargo was run and nothing was timed.
