# Research: L10, sleeping on by default plus the frozen-pair skip

## TL;DR
- **Jolt, Box2D v3/Box3D, Bepu and Rapier all keep sleep state that already exists before the broadphase runs.** Jolt has an active-body list, Box2D/Box3D have solver sets, Bepu has inactive sets whose bodies live in the static tree, and Rapier has an awake mask. Pairs between two sleepers are never generated (Jolt, Bepu), never visited (Box2D) or skipped per pair (Rapier). boyko decides the freeze inside the solve (`colored.rs:3451-3454`), after narrowphase and graph build, from this step's islands. That timing is the root reason bp, np and graph cannot skip anything today.
- **The engines differ on whether a sleeping island keeps its contacts.** Box2D v3 moves touching contacts, impulses included, into the sleeping set and restores them into the graph on wake. Rapier leaves a skipped pair's manifolds untouched. Bepu moves pairs into "sleeping subcaches". **Jolt drops them**: every contact of a body that goes to sleep gets `OnContactRemoved`, and the pair cache is cleared, so a Jolt wake starts cold. boyko's B1 carry follows Box2D.
- **Waking on support removal differs too.** Box2D wakes both bodies whenever a touching contact is destroyed, which includes destroying a body. Rapier wakes every body interacting with a removed or user-modified collider. Jolt does not wake neighbours on removal (its docs tell you to call `ActivateBodiesInAABox`). Neither Box2D's `SetTransform` nor Jolt's `Body::SetLinearVelocity` wakes a body.
- **boyko's wake-on-contact-change (A4) relies on narrowphase re-running frozen pairs.** An island's manifold count changes only because the pairs are recomputed. Once frozen–frozen pairs are skipped and their manifolds kept, a frozen island's count is constant by construction. So the three A4 cases (delete, `RigidBody` removal, teleport; `support_loss_wakes_sleepers.rs:26-29`) would need another trigger.
- **The price, measured.** On the pyramid with everything asleep, boyko takes 5.334 / 5.407 ms per step (W=1/8: bp 1.93 + np 2.92 + graph 0.12 + solve 0.30); Jolt takes 2.17 / 2.29 µs. Flipping the default alone is worth −57.5 % on R at W=1 and −38.4 % at W=8, and leaves the parity row unchanged (`ANALYSIS.md` §6, §9).

## Approaches in reference engines

### Jolt v5.3.0 (read from the local source, `D:/tmp/jolt/JoltPhysics`)
- **Active list.** Per body type, `mActiveBodies[]` plus an atomic `mNumActiveBodies` (`BodyManager.h:345,348`).
  - Activation appends and publishes the count with a release store (`BodyManager.cpp:493-510`).
  - Deactivation swap-removes (`:512-542`). It also **zeroes linear and angular velocity** and clears the island index (`:600-611`).
  - `ActivateBodies` resets the sleep timer (`:544-579`).
- **Broadphase only starts from active bodies.**
  - `JobFindCollisions` claims batches of 16 active bodies with a CAS on `mActiveBodyReadIdx`, then calls `FindCollidingPairs(active_bodies, …)` (`PhysicsSystem.cpp:855-907`).
  - `QuadTree::FindCollidingPairs` walks each active body against the whole tree (`QuadTree.cpp:1411-1442`). Sleeping bodies stay in the tree and are found only as partners.
  - Each dynamic–dynamic pair is reported once, by comparing indices in the active list. An inactive body has index `0xffffffff` (`Body.inl:46-65`).
- **Wake on a new contact.** When a constraint is created, `ActivateBodies` runs for any inactive dynamic body in the pair, and the two bodies are linked by active index (`PhysicsSystem.cpp:1264-1279`). Islands are built each step over active bodies only (`IslandBuilder.h:24-43`).
- **Sleep test.**
  - It runs on the last collision step only. Every body in the island must pass (AND), and then the whole island is put to sleep (`PhysicsSystem.cpp:2319-2356`).
  - Per body: three test spheres around the COM and two extent points. The body can sleep once each sphere's radius stays ≤ `mPointVelocitySleepThreshold · mTimeBeforeSleep` for the accumulated time. Sensors never sleep (`Body.cpp:145-184`).
  - Defaults: 0.5 s, 0.03 m/s, `mAllowSleeping = true` (`PhysicsSettings.h:89,96,117`).
- **Contacts while asleep.** "As soon as a body goes to sleep the contacts between that body and all other bodies will receive an OnContactRemoved callback" (`ContactListener.h:88-89`). The double-buffered cache is swapped and the old side cleared every step (`ContactConstraintManager.cpp:1463-1492`), so sleeping pairs fall out of it.
- **Documented rules** (`Docs/Architecture.md:320`): "removing a Body from the world doesn't wake up any surrounding bodies". `Body::SetLinearVelocity` does not wake; `BodyInterface::SetLinearVelocity` does. Static sensors lose contact with bodies that fall asleep (`:301`). Determinism requires the same API call order and consistent BodyIDs (`:636-638`, `:685`).

### Box2D v3 (main); Box3D uses the same design
- **Solver sets** (`solver_set.h`): `b2_staticSet=0`, `b2_disabledSet=1`, `b2_awakeSet=2`, `b2_firstSleepingSet=3`. Each `b2SolverSet` holds `bodySims, bodyStates, jointSims, contactSims, islandSims`. Box3D's `b3SolverSet` stores `contactIndices` (indices into the world's contact array) instead of copied contact sims.
- **Narrowphase.** `b2Collide` gathers contacts only from the constraint-graph colours and `awakeSet->contactSims`; contacts in sleeping sets are never visited (`physics_world.c`). Box3D's `b3Collide` does the same with `awakeSet->contactIndices`.
- **Sleeping an island** (`b2TrySleepIsland`, `solver_set.c`):
  - It refuses if a split is pending: `if (island->constraintRemoveCount > 0 && island->bodies.count > 1) return;`.
  - It moves the island's bodies and removes its touching contacts and joints from the graph colours.
  - A non-touching contact goes to the disabled set if the other body sleeps, and stays in the awake set if the other body is awake.
  - Islands are processed in reverse at the end of `b2Solve`: "This must be done last because putting islands to sleep invalidates the enlarged body bits" (`solver.c`).
- **Waking a set** (`b2WakeSolverSet`): bodies return with `*state = b2_identityBodyState`, which is zero velocity (`body.h`). Touching contacts are re-added to the graph with their impulses preserved (memcpy). Joints and islands are restored, then the sleeping set is destroyed.
- **Wake triggers:**
  - Begin-touch. Per-worker bitsets are OR-merged and processed serially in bit order. `b2LinkContact` calls `b2WakeSolverSet` on the sleeping side when the other side is awake (`island.c`).
  - `b2DestroyContact` wakes both bodies if the contact was touching (`contact.c`). `b2DestroyBody` destroys all of the body's contacts (`body.c`).
  - API calls that wake: `SetLinearVelocity`/`SetAngularVelocity` with a non-zero value, `ApplyForce`/`ApplyImpulse` with `wake`, `SetType`, `SetAwake`, `EnableSleep(false)`.
  - **`b2Body_SetTransform` does not wake.** It only moves the proxy.
  - `b2World_EnableSleeping(false)` wakes every sleeping set.
- **New contacts.** A new contact goes to the awake set if either body is awake, otherwise to the disabled set: "sleeping and non-touching contacts live in the disabled set" (`contact.c`).
- **Broadphase.** Only moved proxies (fat-AABB enlargement, the `B2_MOVED_NODE` flag) produce new pairs. New pairs are quicksorted by key, then contacts are created serially (`broad_phase.c`).
- **Sleep metric.**
  - Per body: `max(|v| + |ω|·maxExtent, 0.5·|Δx|/dt)` against `sleepThreshold`, with a per-body `sleepTime`. An island stays awake while any member's `sleepTime < B2_TIME_TO_SLEEP` (`solver.c`).
  - I did not verify the value of `B2_TIME_TO_SLEEP` from source. The 0.5 s figure comes from boyko's own doc (`resources.rs:3584`).
- **Persistent islands** ([blog](https://box2d.org/posts/2023/10/simulation-islands/)): islands are unioned when a constraint is added and split lazily ("one island per time step can split"). Cost, DFS against persistent: 0.69 → 0.01 ms (Pyramids, 10,010 bodies) and 0.43 → 0.08 ms (Tumbler).

### Rapier (master)
- **`IslandManager`** has a single `awake_island` holding every awake body, an incremental `persistent: PersistentIslands` graph, and `active_set_epoch` (`island_manager/manager.rs`). Waking any body wakes its whole island (`sleep.rs`).
- **Narrowphase skip.** `pair_update::process_pair` returns `OUTCOME_SKIPPED` when "Neither collider was changed by the user nor possibly moved by the simulation (its parent body is asleep or fixed)". The pair's manifolds are left untouched. The awake mask is rebuilt from `active_bodies()` at every narrowphase update (`narrow_phase/mod.rs`). I did not determine whether the outer loop still visits every pair.
- **Wakes** (`pair_management.rs`):
  - a removed collider wakes every body it interacted with;
  - a user-modified collider wakes its own body and every interacting body;
  - removing a pair that had an active contact wakes both bodies.
- **Broadphase BVH.** Only changed leaves are re-checked: "A pair can only stop overlapping if one of its colliders changed in the tree" (`broad_phase_bvh/update.rs`).

### Bepu v2
- **Sleeping bodies move to the static tree.** Their broad-phase leaves go in via `broadPhase.AddStatic` (`IslandSleeper.cs`). "Inactive bodies and statics exist within the same static/inactive broad phase tree and are not tested against each other"; "The body of a body-static pair must be active" (`NarrowPhase.cs`).
- **Pair cache.** The sleeper moves an island's pairs "into sleeping subcaches rather than keeping them in the active pair cache". I did not verify whether accumulated impulses are copied with them.
- **Budgeted traversal.** The number of bodies traversed and slept per frame is a target fraction of the active set. "The multithreaded island search is currently nondeterministic", so a `deterministic` flag forces a single thread (`IslandSleeper.cs`). The awakener must wake source sets in order for determinism (`IslandAwakener.cs`).
- **Metric.** `dot(v,v) + dot(ω,ω) < SleepThreshold` for `MinimumTimestepCountUnderThreshold` steps (default 32) (`BodyDescription.cs`, `BodyProperties.cs`). This is the same speed² family as boyko's metric.

## Comparison table

| Aspect | Jolt 5.3 | Box2D v3 / Box3D | Rapier | Bepu v2 | boyko (tree) |
|---|---|---|---|---|---|
| Sleep state lives in | active list (structural) | solver-set membership | awake mask plus persistent islands | inactive sets plus static tree | per-row latch `Vec<bool>`, decided inside the solve |
| Broadphase for sleepers | only active bodies query | only moved proxies query | only changed leaves | sleepers in the static tree; static–static never tested | AllPairs visits every i<j pair |
| Narrowphase for sleeper–sleeper pairs | never generated | never visited | per-pair skip, manifolds kept | never generated | fully recomputed |
| Contacts and impulses while asleep | dropped (cold wake) | kept in the set, restored to the graph | kept | moved to sleeping subcaches | warm entries carried (B1); manifolds recomputed |
| Island build | per step, active only | persistent, split lazily | persistent, one awake island | incremental | per step over all rows and all manifolds |
| Wake on support removal | no (manual AABB wake) | yes (touching contact destroyed) | yes (collider removal) | not checked | via manifold-count change (A4) |
| Wake on teleport / user write | through `BodyInterface` only | `SetTransform`: no | yes (change flags) | through the awakener API | via manifold-count change |
| Velocity when asleep | zeroed on sleep | identity (zero) on wake | not checked | not checked | restored to the pre-solve snapshot each step |
| All-asleep step (pyramid) | 2.17 / 2.29 µs | — | — | — | 5.33 / 5.41 ms |

## boyko tree facts (D:/wt/joltab @ `e2bcbcb5`, confirmed from `.git/refs`)

**Config and state**
- `sleeping: false` is at `resources.rs:504`; its rationale is the "0%-gate" at `:502-503` and `:328-329`.
- `DEFAULT_SLEEP_THRESHOLD = 1.0e-4` (speed²) at `:435`; `DEFAULT_SLEEP_FRAMES = 60` at `:440`.
- `IslandSleep` is at `:3108-3159`. Its per-row and per-island data are **`std::Vec`**: `asleep` `:3116`, `below_count` `:3121`, `frozen_islands` `:3127`, `energy` `:3132`. Flipping the default puts these on the default path (principle 0).
- The same struct has one `TouchedMask` (`:3138`) and two `ScratchColumn`s (`:3146`, `:3153`).
- `IslandSleep` is inserted only on the colored path (`plugin.rs:541-549`).

**The per-step sleeping path** (`colored.rs:3412-3674`)
- `rekey_rows` runs at the start of the solve (`:3426-3428`), then the no-dynamic-body early return (`:3435-3441`), then `begin_step(graph, n_rows)` (`:3451-3454`).
- `begin_step` (`resources.rs:3480-3568`):
  - compares each latched row's island manifold count with its stored key, which is A4 (`:3542-3548`);
  - does the wake-on-merge AND fold (`:3550-3554`);
  - rebuilds the awake mask with a per-row `island_of` call (`:3560-3567`).
- The frozen skip happens only in `build_columns` (`:1648-1656`, predicate `:1863-1871`).
- The freeze snapshot is captured at `:3487-3503`. Gravity, integrate and refresh then stream **all** rows (`:3554`, `:3590-3595`, `:3599`) and are undone at `:3639-3652`. Frozen poses are therefore restored byte-exactly each step.
- The store sizes the table for solved plus frozen points (`:3237`); `carry_frozen` is at `:3282-3319`.
- `write_back_awake` (`:3345-3360`) never flags a frozen row as touched, so `physics_apply` (`systems.rs:1184`) never writes it.
- `end_step` is at `resources.rs:3615-3670`.

**Stages that ignore the sleep state**
- Gather: `systems.rs:210-275`.
- Broadphase: AllPairs `:309-327`, Grid `:335-341`.
- Narrowphase: `:375-478`. It clears `manifolds` and `sensor_overlaps` every step (`:382`, `:385`).
- SDF narrowphase: a per-body loop over every simulated dynamic row (`:606-634`).
- Graph build: `systems.rs:1038-1057`.
- `ConstraintGraph::build` (`resources.rs:2655-2665`):
  - resets union-find over all `n_dynamic = bodies.len()` rows (`:2670-2689`);
  - colours first-fit over **all** manifolds, frozen ones included (`:2880-2993`).

**Change detection is already available at gather.** The gather reads `Ref<RigidBody>` (`body_set.rs:106-113`), and `Ref::is_changed()` exists (`boyko_ecs/.../ref_.rs:58-61`). Frozen rows are not written by apply. `Changed<RigidBody>` was ruled out as a wake key only because the solver writes awake rows every step (`resources.rs:3409-3411`). Two other stages write `RigidBody`: `sync_transform_to_body` (static and kinematic bodies; `plugin.rs:222-225`) and the soft→rigid coupling apply.

**Row-keyed persistent state.** Four consumers carry state across row moves through `RowIdentity`: the latch, the two warm tables and the axis cache (`row_identity.rs:6-17`). A recycled id can carry a dead body's state for one gather (`:19-27`). Kept manifolds would be keyed by `BodyIndex` rows in the same way.

**Axis cache.** `begin_frame(pairs)` sizes the table from **this frame's pair count**. It clears everything when the table must grow or when occupancy exceeds `len/2` (`axis_cache.rs:261-281`). A pair's hint is rewritten only when the pair is visited (`:319-354`).

**Recorded gaps that become default-path.** SDF + sleeping has no gate, and a soft→rigid reaction does not wake a sleeper (`resources.rs:3469-3475`, `plugin.rs:347-349`, `:405-407`). Also "Not covered": a user write that changes no contact, and a support that moves but keeps its island's count (`resources.rs:3463-3468`).

**Measured**
- Sleeping on with threshold 0 gives the same pose hash as sleeping off over J's 500 steps (`0x32d5e235342b4143`, `ANALYSIS.md:261`). The flip therefore moves only scenes where an island actually freezes inside the horizon.
- R-S first freezes at step 248 in 12 of 12 processes.
- F = 6.049 / 6.123 ms, of which collision plus graph is 92 % (§6).

### What a flip moves

**Parity runner.** Cfg A/B set `cfg.sleeping = args.sleeping` (`jolt_parity_pyramid.rs:735`), so J is unaffected. `CfgKind::Default` only ever sets `true` (`:741-743`), so **the R row inherits the new default** unless the runner changes. There is also a `sleeping: false` at `:1118`.

**Tests that pin the flag:**
- Off: `apply_row_alignment.rs:344`, `alloc_frame_census.rs:1769`, `alloc_frame_attribution.rs:1936`, `sleep_settles_box_piles.rs:1702` (A7-R1) and `:1896`, `support/profiling_harness.rs:294`.
- On: `sleep_settles_box_piles.rs:597`, `support_loss_wakes_sleepers.rs:241`, `sleeping_pipeline_o8.rs:116`, `frozen_island_warm_start.rs:290`, and six sites in `colored_tests.rs` (2209…3072).
- Toggled: `row_identity_remap.rs:357/495`, `row_keyed_state_defect_a.rs:235`, and benches `row_identity_churn.rs:241`, `sleeping_pipeline.rs:124`, `sleeping.rs:183/218`.

**Colored-path tests that inherit the default:**
- `default_world_pyramid_determinism.rs` (its doc says "sleeping off", `:8`; 120 frames);
- `default_world_worker_invariance.rs` (60 frames, `:56`);
- `default_world_colored_simd.rs` (10);
- `bodytype_determinism_golden.rs` (90);
- `colored_acceptance_o5.rs`, `colored_acceptance_simd_o7.rs`, `colored_solve_zero_alloc_o5.rs`, `body_set_selection.rs`, `scene_sync_s5.rs`, `sdf_collision.rs`, `soft_body_sp2.rs`.
- A test moves only if some island freezes within its horizon. The freeze needs at least 60 consecutive sub-threshold steps.

**Docs that say "off":** `book/src/simulation/physics.md:354`, `docs/SYSTEMS.md:2065`, `docs/MEASUREMENT-QUEUE.md:520`, `resources.rs:328-329`, `:3473`, `plugin.rs:349`, `:407`, `:543`.

### Planned refactor context
`D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md` is not in joltab's `docs/`. It plans:
- D4 `BodyGate` with an AWAKE lane gate that deletes the frozen snapshot (`:149-157`); U6 at `:797`;
- D5 / U7, a slot-keyed double-buffered `PairCache` (`:159`, `:798`);
- X-5: "In v1 a toggle requires `&mut EcsMaster`", which is why an `EnableColumn` sleep tag was set aside (`:41`).

The owner's "refactor last" ruling (`5aef7b39`) puts these after the levers.

## Pitfalls
1. **Deciding sleep after the stages you want to skip.** boyko's freeze depends on this step's islands and counts. Every reference decides from persistent state carried over from the previous step.
2. **A4 goes vacuous under a narrowphase skip.**
   - With kept manifolds, a frozen island's count cannot change, and `support_loss_wakes_sleepers` cases a, b and d would go red.
   - Case d is a raw `get_component_mut` pose write while frozen (`:504-510`). Only Rapier-style change flags catch that class; Box2D and Jolt do not wake on a teleport.
3. **Axis-cache clears drop hints the skip no longer rewrites.** A clear-on-grow or clear-on-occupancy computed from an awake-only pair count erases frozen pairs' hints. On wake, a different hint can change feature ids, which means warm misses. `frozen_island_warm_start.rs:738` asserts that the wake step is bit-exact. (Inference from `axis_cache.rs:261-281`; not run.)
4. **Kept manifolds are a fifth row-keyed store.** They need the `RowIdentity` remap and inherit its one-gather recycled-id bound.
5. **The drop policy changes wake quality.** Jolt drops contacts on sleep, which B1 measured as a cold wake (4.54e-2 m/s sag against 1e-2).
6. **Sensor semantics.** Jolt's static sensors stop seeing sleeping bodies. boyko rebuilds `sensor_overlaps` from all pairs every step (`systems.rs:385`).
7. **Determinism.** Bepu's multithreaded island search is nondeterministic without its flag. Box2D processes touch transitions serially in bit order. Jolt's active-list order depends on the order of API calls and on the ids.

## Inferences from the code (not tested)
- **Colours.** A dynamic body's occupancy bits are set only by manifolds of its own island, and a frozen island has no awake manifolds. So first-fit colours and the in-colour order of awake manifolds should not change when frozen manifolds are removed, as long as relative manifold order is kept (`resources.rs:2880-2993`).
- **Replayed manifolds.** `box_box_contact` depends only on the two poses and the hint (`03-TREE-REPORT.md` §4), and frozen poses are restored byte-exactly (`colored.rs:3645-3651`). So replaying a frozen–frozen pair's kept manifold should equal recomputing it, provided the hint is unchanged and no static or kinematic partner moved. Kinematic and static partners can move through `sync_transform_to_body`.
- **Wake-on-merge.** It survives if awake–frozen pairs keep going through narrowphase and the graph, as in all four references.

## Open questions for the architect
- Where does the frozen state live so broadphase can read it, and at what step boundary (the latch after `end_step` or after `rekey_rows`)? It must stay bit-identical across W.
- What stores the kept manifolds (a Box2D-style per-set copy or Box3D-style indices; a principle-0 column), and how are they remapped?
- Which trigger replaces A4 for skipped pairs? Candidates are: removed rows known to `RowIdentity`, `Ref::is_changed` on frozen rows at gather, and static or kinematic partner writes.
- The AllPairs skip still iterates n²/2 unless the loop becomes awake×all. What about the Grid CSR?
- Should the SDF stage skip frozen rows?
- Should sleep keep the restored at-rest velocity (today) or zero it (Jolt, Box2D)?
- Should the skip claim bit-identity against today's sleeping-on path?

## Sources
Line-numbered Jolt, boyko and unification-design references in the body were read directly. The Box2D, Box3D, Rapier and Bepu quotes came through WebFetch's summarizer, which is why they carry no line numbers. The prior survey only supplied engine defaults.

[1] D:/tmp/jolt/JoltPhysics (v5.3.0): `Jolt/Physics/Body/BodyManager.{h,cpp}`, `PhysicsSystem.cpp`, `Collision/BroadPhase/QuadTree.cpp`, `Body/Body.{cpp,inl}`, `IslandBuilder.h`, `PhysicsSettings.h`, `Constraints/ContactConstraintManager.{h,cpp}`, `Collision/ContactListener.h`, `Docs/Architecture.md`. Upstream: https://github.com/jrouwe/JoltPhysics/tree/v5.3.0
[2] https://raw.githubusercontent.com/erincatto/box2d/main/src/solver_set.h and `solver_set.c`: solver sets, sleep and wake
[3] https://raw.githubusercontent.com/erincatto/box2d/main/src/physics_world.c: collide gathering, touch processing, `EnableSleeping`
[4] https://raw.githubusercontent.com/erincatto/box2d/main/src/island.c: `b2LinkContact` wake
[5] https://raw.githubusercontent.com/erincatto/box2d/main/src/contact.c: contact set placement, destroy-wakes
[6] https://raw.githubusercontent.com/erincatto/box2d/main/src/body.c and `body.h`: API wakes, `SetTransform`, identity state
[7] https://raw.githubusercontent.com/erincatto/box2d/main/src/broad_phase.c: moved-proxy pairing
[8] https://raw.githubusercontent.com/erincatto/box2d/main/src/solver.c: sleep metric, island sleeping
[9] https://box2d.org/posts/2023/10/simulation-islands/: persistent islands, DFS numbers
[10] https://box2d.org/documentation/md_simulation.html: documented wake rules
[11] https://raw.githubusercontent.com/erincatto/box3d/main/src/solver_set.h and `physics_world.c`
[12] https://raw.githubusercontent.com/dimforge/rapier/master/src/dynamics/island_manager/manager.rs and `sleep.rs`
[13] https://raw.githubusercontent.com/dimforge/rapier/master/src/geometry/narrow_phase/pair_update.rs, `mod.rs`, `pair_management.rs`
[14] https://raw.githubusercontent.com/dimforge/rapier/master/src/geometry/broad_phase_bvh/update.rs
[15] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/IslandSleeper.cs, `IslandAwakener.cs`, `BodyProperties.cs`, `BodyDescription.cs`, `CollisionDetection/NarrowPhase.cs`
[16] D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md; D:/wt/joltab/docs/physics/perf-campaign/{00-RULINGS,01-PLAN-REV1,03-TREE-REPORT,04-PRACTICE-SURVEY}.md
[17] https://box2d.org/posts/2026/06/announcing-box3d/ (search pointer only)