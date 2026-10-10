# Physics ECS unification - research

- **Date:** 2026-09-10
- **Code tree:** `D:/wt/joltab` @ `ca582e72` (branch `merge/ke16-into-ecsnative`: the ECS-native physics lane through Stage 4, merged with the KE16 thread pool)
- **Companion design:** [PHYSICS-ECS-UNIFICATION-DESIGN.md](PHYSICS-ECS-UNIFICATION-DESIGN.md)
- **Status:** four read-only survey lenses (data, logic, plans, reference). Nothing was run: no cargo, no rustc, no git state change.

## The owner's orders (2026-09-10, verbatim in translation)

1. "There is not one reason to use an allocator other than ours. If something is missing, extend the memory library."
2. "Move ALL runtime data structures onto our system, and all arrays etc. onto the ECS."
3. "The point is not only to move everything onto our allocator, but to bring everything as close as possible to the ECS PARADIGM - in particular in PHYSICS - and to make ONE UNIFIED SYSTEM."

## Provenance tags

- **[S]** source read.
- **[D]** official documentation.
- **[B]** blog post or talk (recorded, not relied on).
- Anything that could not be verified is labelled "not verified", never paraphrased from memory.
- In-tree facts are cited as `path:line` with the quoted line.

## How this file is laid out

Each lens report below is reproduced verbatim under its lens heading, followed by that lens's open list (one item per entry, text verbatim). Line anchors in the reports refer to `D:/wt/joltab` @ `ca582e72` unless a report names another tree.

---

# Lens 1 of 4: data

# Physics data inventory: D:/wt/joltab @ ca582e72 (merge/ke16-into-ecsnative)

This was a read-only walk. I ran no cargo or rustc and changed no git state. Sizes marked "arith." are worked out from field types (Vec3 = 12 B, Quat = 16 B and Mat3 = 36 B, per `D:/wt/joltab/crates/boyko_math/src/mat.rs:30` `pub rows: [Vec3; 3],`). I did not check them with size_of or an LSP hover.

"Pyramid" means `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs:14` `//! * **1240 dynamic bodies** + 1 static floor;`. That is 1241 gathered rows, wired by `add_physics_colored_solve`, with `cfg.parallel_solve = workers > 1; cfg.parallel_broadphase = workers > 1; cfg.sleeping = false;` (bench lines 214-216).

## TL;DR

1. **The 34-Vec count is right.** The only non-test heap containers in `boyko_physics/src` are:
   - the 4 `IslandSleep` fields (resources.rs:3057/3062/3068/3073);
   - the 30 `SoftBody` fields;
   - SoftBody constructor locals;
   - one debug-only `vec!` (resources.rs:2965).

   Everything else is one of 90 kernel `ScratchColumn`s on reserved ids 511 down to 422, or a plain Resource/Component field. **The gate that pins the 34 is not in this tree.** `tests/physics_vec_side_store_census.rs` lands in ad0ebea4, which `git merge-base --is-ancestor` reports as NOT in ca582e72's history. It is the only commit on feat/ecs-native-storage that is missing here.
2. **The rigid pipeline uses ECS storage but is not ECS-shaped.**
   - Each step copies per-body state RigidBody + RigidBodyMass + Collider into `BodyState` (156 B, arith.), then into `BodyEffective` (64 B), then back through a touched mask.
   - Every cross-stage and cross-frame key uses the gather ROW: pairs, manifolds, warm-start, box-axis cache, sleep latch, soft reaction. The kernel swap-removes rows on despawn.
   - Physics uses no dense component, no relationship and no event.
3. **The walk surfaced bugs.**
   - F-1: the `Trigger` bundle, and any collider without a `RigidBody`, never enters physics.
   - F-2: `pub` Vec fields plus unchecked raw indexing in SoftBody let safe code cause UB in release.
   - F-3: row-keyed persistent state is not stable across a despawn, although the sleep doc says it is.
   - F-4: some authored bytes are never read.
   - F-5: **the pyramid runs the serial O(n²) AllPairs broadphase** (769,420 bound tests per step, arith.). `parallel_broadphase` does nothing on that path. This corrects serial-candidate list (c) in the brief.
4. **"ECS-native == cache-optimal" is only partly true.**
   - It holds for the storage primitive: `row_ptr` on a `ComponentPool` either way.
   - It held only after the per-pool stagger fix, which recovered a ~40% regression.
   - It depends on three things the kernel does not provide uniformly: cohort-distinct ids, a derived per-body record shaped for the solver, and untracked storage for components.
5. **The ledger disagrees with the tree in four places.**
   - Its "Resource+VmColumn" destination does not type-check.
   - 10 of SoftBody's 30 Vecs are per-substep scratch, not per-body data.
   - Its SoftBody soundness argument ignores the `pub` fields.
   - It repeats the row-stability claim.

## 1. Storage forms present

- **(T) Table component**, an archetype `ComponentPool` column: RigidBody, RigidBodyMass, Collider, Contact, SoftBody, Transform.
- **(B) Bitset EnableTag**, a per-archetype `EnableColumn` with no data column: Simulated, Kinematic (`components.rs:61` `#[component(storage = "bitset")]`).
- **(Z) Zero-size marker** (archetype membership): Sensor.
- **(S) Resource-owned `ScratchColumn<T>` on a reserved synthetic id.** It is backed by an untracked pool: `scratch_column.rs:95` `let column = ComponentPool::new_untracked(component_id.get(), reserve_rows);`. The ids are hand-placed downward from 511 (`scratch_ids.rs:18-19` "the scratch ids occupy a fixed region at the TOP of `[0, MAX_COMPONENTS)`").
- **(R) Plain Resource fields:** PhysicsConfig, IntegrationMode, PhysicsStats, SdfField.
- **(V) std `Vec` in a Resource or Component:** IslandSleep ×4, SoftBody ×30, TransformPropagationScratch ×3, RefcountDeltas, DeferredFree.
- **(D) Dense, non-fragmenting component:** exists in the kernel, unused by physics. `dense_store.rs:6-8` "deletion is tombstone + free-list, never swap-remove, so **live slots never move** — the determinism contract the colored physics solver depends on". `dense_store.rs:12` "the physics consumer (Stage P) land later".

## 2. Inventory by subsystem

Each entry gives: form, element, lifetime, granularity, writers → readers, whether a raw pointer is held across a grow, and the pyramid size.

### 2.1 Authored per-entity data

- **RigidBody.** Form T. `components.rs:29` `pub struct RigidBody {`, holding position, linear_velocity, rotation, angular_velocity. 52 B (arith.). Persistent, per entity.
  - Writers: `physics_apply` (`systems.rs:1141` `*body = RigidBody {`); `physics_integrate`, but only in Foundation mode (`systems.rs:144` `if *mode == IntegrationMode::SolverOwned {`); `sync_transform_to_body` (scene_sync.rs:100-104); `physics_soft_rigid_apply` (coupling.rs:551-554); gameplay.
  - Readers: `physics_gather` (systems.rs:244), `sync_body_to_transform`.
  - No physics pointer is held into it. Pyramid: 1241 × 52 B ≈ 64.5 KB.
- **RigidBodyMass.** Form T, 48 B (arith.): `inv_inertia` Mat3, inv_mass, restitution, friction. Persistent, per entity.
  - Readers: gather, integrate, scene_sync.
  - `inv_inertia` has no production reader (F-4). Pyramid: 59.6 KB, of which 44.7 KB is never read.
- **Collider.** Form T, 24 B (arith.): shape (16 B, repr(C) tag + union), layer, mask. Persistent, per entity.
  - Reader: gather, which reads `shape` only. `layer`/`mask` have no reader (F-4).
- **Simulated / Kinematic.** Form B. Toggled by gameplay (`bundles.rs:73` `e.enable::<Simulated>();`). Read order-preservingly via `IsEnabled` in gather, integrate and scene_sync. `components.rs:45-46` "NO archetype migration and NO row move — so flipping it NEVER reorders the physics gather".
- **Sensor.** Form Z. Read at gather as `Option<&Sensor>` (systems.rs:206).
- **Contact.** Form T (`components.rs:202` `pub struct Contact {`). **No producer exists anywhere in the tree.** A grep for a `Contact {` construction or insert outside components.rs finds none.

### 2.2 Singletons

- **PhysicsConfig.** Form R, global and persistent. It is also a per-step datum:
  - `physics_gather` writes `dt` every step: `systems.rs:217` `cfg.dt = fixed_time.delta_secs();`.
  - `select_broadphase` writes `broadphase` in Auto mode: `broadphase_policy.rs:192` `cfg.broadphase = if band { BroadphaseKind::Grid } else { BroadphaseKind::AllPairs };`.
  - Every stage reads it.
- **IntegrationMode.** Form R, set at wire-up (plugin.rs:527-532), read by integrate.
- **PhysicsStats.** Form R, `{active_body_count: u32, broadphase_band: bool}` (broadphase_policy.rs:111-116). Written and read only by `select_broadphase`.
- **SdfField.** Form R, a fixed inline edit array (`sdf_query.rs:46` `pub struct SdfField(SdfEditField);`). Inserted only when `with_sdf || soft` (plugin.rs:496). Absent in the pyramid.

### 2.3 Gather mirror (`SolverScratch`, resources.rs:3634)

- **`bodies`: ScratchColumn<BodyState>, id 511** (`scratch_ids.rs:110` `SCRATCH_ID_BODY_STATE: usize = MAX_COMPONENTS - 1;`).
  - Element: BodyState, an AoS of 156 B (arith.) holding two Mat3, position/velocity/angular velocity, rotation, 3 f32, 3 bool and the shape.
  - Lifetime: per step (refilled at systems.rs:242-253). Granularity: per body row.
  - Writers: the gather; the colored solver's per-substep `position_integrate` (colored.rs:3278-3284); `write_back` (3051-3062); the sleep restore (3320-3330).
  - Readers: select_broadphase, broadphase, narrowphase, narrowphase_sdf, build_graph, solver build/integrate/refresh, `IslandSleep::end_step`, apply, coupled soft step.
  - Pointers: `EmitPtrs.bodies` (resources.rs:1786) holds a raw pointer across a scope with no grow in the window.
  - Pyramid: ≈193.6 KB (arith.).
- **`touched`: TouchedMask = ScratchColumn<BitSet256>, id 476.** Per step. Reset at gather (systems.rs:257), set in `write_back`, read by apply (`systems.rs:1137` `if row < bodies.len() && scratch.touched.get(row) {`). Pyramid: 5 chunks × 32 B.
- **`vn_initial`: ScratchColumn<f32>, id 477.** Per step, per contact point. Cleared at gather (systems.rs:220). Used only by the serial `SoftStepSolver`. The colored solver has its own copy (`contact_column_id(25)`).

### 2.4 Broadphase

- **`ContactPairs.pairs`: ScratchColumn<(BodyIndex,BodyIndex)>, id 446.** Per step, per pair, keyed by row (`resources.rs:545` `pub(crate) pairs: ScratchColumn<(BodyIndex, BodyIndex)>,`). Written by broadphase (the AllPairs loop at systems.rs:299-308 or the grid), read by narrowphase (systems.rs:378). The pyramid pair count cannot be derived statically.
- **`BroadphaseGrid`: 13 ScratchColumns, ids 459..447, plus scalar geometry** (resources.rs:694-762). Per step, per cell or body.
  - Writer: broadphase, Grid arm only.
  - Readers: the coupled soft step (coupling.rs:313/340/348).
  - Pointers: `EmitPtrs.out_base` and `pair_count` are captured after `resize`, and the refill view is retaken only after the join (resources.rs:1832-1855, 1892-1895).
  - Pyramid: **never built**. `add_physics_pipeline` forces Grid only `if coupling` (plugin.rs:458-462), and `parallel_broadphase` "is a no-op for the shipped all-pairs loop" (resources.rs:222-223).

### 2.5 Narrowphase and its cross-frame cache

- **`Manifolds.manifolds`: ScratchColumn<Manifold>, id 445.** Element size pinned: `manifold.rs:121` `const _: () = assert!(size_of::<Manifold>() == 152);`. Per step, per pair. Writers: narrowphase (clear + push) and narrowphase_sdf (append). Readers: build_graph and the solve.
- **`Manifolds.sensor_overlaps`: id 444.** Per step, row-keyed. **No production reader.** The only consumer is `D:/wt/joltab/crates/boyko_physics/tests/scene_sync_s5.rs:451`.
- **`box_axis_cache`: BoxAxisCache (ScratchColumn<AxisEntry>, id 443).** **Persistent across frames**, per body pair, keyed by row pair (`axis_cache.rs:78-79` `fn pack(body_a: BodyIndex, body_b: BodyIndex) -> u64 { ((body_a.0 as u64) << 32) | (body_b.0 as u64)`). Read and written by narrowphase. Cleared on growth or when occupancy passes len/2 (axis_cache.rs:183-200).

### 2.6 Constraint graph (resources.rs:2428-2475)

- **8 ScratchColumns, ids 467..460**: uf_parent, uf_size, island_of, island CSR ×2, colour CSR ×2, and `color_occ` (u64). Plus the counts.
- Per step. Per body (`uf_*`, `island_of`), per island, per colour.
- Writer: `physics_build_graph` (systems.rs:1023). Readers: `build_columns` (colored.rs:1554-1555), `manifold_frozen` (colored.rs:1714-1720), and `IslandSleep`.
- Island ids are volatile: `resources.rs:3008` "Island ids are NOT stable: [`ConstraintGraph::build`] re-derives them every frame".
- Pyramid: 3 × 1241 × 4 B ≈ 14.9 KB for the per-body columns. `color_occ` is 20 words per colour × n_colors, and n_colors is unknown.

### 2.7 Colored solver (Resource `ColoredSoftStepSolver`, colored.rs:1408-1434)

- **`bodies`: ScratchColumn<BodyEffective>, id 510.** 64 B: inv_mass, world inverse inertia, linear and angular velocity (contact.rs:81-91). Rebuilt every solve (colored.rs:1475-1497). Per body row.
  - Written per substep by gravity, warm-start apply, solve (through `ScratchSolveView::row_ptr`) and `refresh_inertia`.
  - Pointers: the solve view base is captured after all growth: `colored.rs:1539` "BEFORE any `solve_view()` captures a base — so no worker can see a moving base". The base is also address-stable by construction (scratch_column.rs:33-37).
  - Pyramid: 79.4 KB.
- **`columns`: ContactColumns, 31 ScratchColumns, ids 508..478** (colored.rs:858-923). Per step, per contact-point slot. About 105 B per slot plus CSR (arith.). Built serially and solved through `ContactSolveView` raw bases (colored.rs:458-491).
- **`warm_read` / `warm_write`: WarmStartTable (ScratchColumn<WarmEntry>, ids 472/471, 24 B).**
  - **Persistent across frames**, double-buffered and swapped: `colored.rs:3046` `core::mem::swap(&mut self.warm_read, &mut self.warm_write);`.
  - Per contact point, keyed by row pair plus feature (warm_start.rs:133-144).
  - It knowingly loses state on a structural change: `warm_start.rs:37-39` "A structural change reshuffles the dense rows, so the matched keys differ for one frame: that is a warm-start MISS".
- **`frozen`: ScratchColumn<(u32, BodyState)>, id 468.** Per step, filled only when sleeping is on. It exists because `colored.rs:3183` "the O1 SIMD kernels are NOT per-lane masked".
- The serial `SoftStepSolver` (soft_step.rs:188-213) holds bodies (id 509), manifolds (470), points (469) and warm tables (474/473). It is not inserted in the pyramid.

### 2.8 Sleep (`IslandSleep`, resources.rs:3051-3084)

- **`asleep`** (Vec<bool>) and **`below_count`** (Vec<u16>): **persistent, per body ROW**. Resized by `sync_rows` (3186-3189) and written by `end_step` (3342-3351).
- **`frozen_islands`** (Vec<bool>) and **`energy`** (Vec<f32>): per step, per island.
- **`awake_rows`**: ScratchColumn, id 475, per step.
- **`wake_all`**: a bool one-shot.
- Pyramid: inserted (colored path), but `begin_step`/`end_step` are never called (sleeping is off), so these stay empty with 1024 capacity reserved at boot.

### 2.9 Soft body (none in the pyramid)

**`SoftBody`: form T holding 30 Vecs plus `particle_radius`** (component.rs:69-180).

| Group | Fields | Lifetime |
|---|---|---|
| Particle state | pos/prev/vel × xyz, inv_mass (10) | Persistent, rewritten every substep |
| Distance topology | c_a, c_b, c_rest, c_compliance (4) | Immutable |
| Tet topology | t0..t3, t_rest, t_compliance (6) | Immutable |
| Per-substep scratch | coupling_prev ×3, coupling_dv ×3, coupling_hit, sc_cell_start, sc_cell_items, sc_cursor (10) | Scratch (`component.rs:119-120` "coupling velocity-baseline position X (SP2 D6/D4, W1) — scratch"; `:155` "self-collision spatial-hash CSR bucket offsets — scratch") |

- `SoftCols::from_body` takes raw pointers into these Vec buffers per constraint call (solver.rs:547-548) or per colour (soft/colored.rs:1061/1098/1192). Nothing in the crate resizes them in that window: my grep for push, resize, clear or extend on `body.<field>` found none.
- **`SoftColorScratch`** (Resource, soft/colored.rs:514-537): 3 × 6 ScratchColumns (ids 440..423), a pair list (id 422), a `topology_colored` latch and a counter.
- **`SoftRigidReaction`** (Resource): `dv_lin` id 442, `dv_ang` id 441, per step, per rigid ROW (`coupling.rs:44-45` "keyed by dense BodyIndex (the snapshot row)").
- Soft bodies are stepped serially: `soft/colored.rs:699` and `soft/solver.rs:95` `for body in query.iter_mut() {`.

### 2.10 Scene side that physics writes into

- **Transform** (boyko_scene, form T): written by `sync_body_to_transform` for dynamic roots (scene_sync.rs:163-167) and read by `sync_transform_to_body`. `scene_sync.rs:8` "the world pose lives in TWO ECS columns ([`RigidBody`] and `Transform`)".
- **GlobalTransform**: written by the exclusive per-frame `propagate_transforms`.
- **TransformPropagationScratch** (propagation.rs:115-152): `stack: Vec<(Entity, u32)>`, `dirty: Vec<Entity>`, `detached: Vec<Entity>`, plus `last_run` and a `OnceLock`. It moves the Vecs out and back each run (`propagation.rs:250` `std::mem::take(&mut scratch.stack),`).
- The pyramid wires no scene sync.

### 2.11 Asset queues (not physics data)

- **RefcountDeltas**: `asset_refs.rs:99` `deltas: Vec<RefDelta>,`. Pushed from hooks (render_caps.rs:285-346).
- **DeferredFree**: `asset_refs.rs:149` `entries: Vec<FreeEntry>,`, drained with an order-preserving `self.entries.remove(i)` (asset_refs.rs:191).
- Its doc is out of date. `asset_refs.rs:143-144` says "nothing drains it until F6's fence-gated `retire_deferred_frees` lands", while `D:/wt/joltab/crates/boyko_app/src/runner.rs:280` says "Drained every frame by `retire_deferred_frees` alongside `DeferredFree` +".

### 2.12 Things that do not exist

- No joints: a grep for "joint" in `boyko_physics/src` finds nothing.
- No collision or sensor events: no `#[event]` or EventWriter in physics.
- No `Contact` producer.
- No row→entity map: resources.rs:3616-3621.
- `Entity` is not a query datum: `systems.rs:191` "`Entity` is not a `QueryData` in the engine". The kernel has no `QueryData` impl for Entity; the impls in `iters/query/data/*.rs` cover `&T`, `&mut T`, `Mut`, `Ref`, `Option`, `AnyOf`, `IsEnabled` and tuples.

## 3. Findings the walk surfaced

**F-1 (Important, logic): colliders without a RigidBody are invisible to physics, so the shipped Trigger never reports anything.**
- The gather requires all three columns: `systems.rs:202-205` `query: Query<( &RigidBody, &RigidBodyMass, &Collider,`.
- `D:/wt/joltab/crates/boyko_physics/src/bundles.rs:83-84` "No [`RigidBody`] / [`RigidBodyMass`]: a trigger is not integrated." So a `Trigger` never enters `bodies`, never pairs, and its `sensor_overlaps` stay empty.
- The same applies to `components.rs:56-57` "A permanent (collision-only) static body simply does not carry a [`RigidBody`]": such a body collides with nothing.
- The only sensor test spawns RigidBody + RigidBodyMass + Collider + Sensor (`tests/scene_sync_s5.rs:63-67`), so no test covers this.
- There is also a latent mirror image. An entity with a RigidBody but no Collider or RigidBodyMass is walked by `physics_apply`'s `Query<Mut<RigidBody>>` (systems.rs:1127) but not gathered. In release, apply rows would shift onto the wrong bodies; only the `debug_assert!` at systems.rs:1153 catches it.

**F-2 (Important, soundness): safe code can cause an out-of-bounds raw access in SoftBody.**
- The fields are `pub`: `component.rs:91` `pub c_a: Vec<u32>,`.
- The kernel reads without bounds checks: `soft/solver.rs:580-582` `let a = unsafe { *cols.c_a.add(c) } as usize; ... let wa = unsafe { *cols.inv_mass.add(a) };`.
- The only guard is debug-only (component.rs:66-67 "the solver `debug_assert!`s it on entry"; soft/colored.rs:730-735).
- So `soft.c_a[0] = u32::MAX`, or `soft.pos_x.truncate(0)`, from safe user code through `Query<&mut SoftBody>` gives an out-of-bounds read or write in release. I reasoned this from the code; I did not reproduce it.

**F-3 (Minor, logic): row-keyed persistent state does not follow the body across a despawn.**
- `resources.rs:3013-3014` claims "Body ROWS, by contrast, are STABLE across frames (the gather is FULL and dense, IM-1 — rows never shift)".
- The kernel swap-removes: `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/archetype/archetype.rs:1220` "[`RemoveOutcome::Swapped`]: swap-remove occurred".
- After a despawn the moved body inherits the removed row's `asleep`/`below_count` latch, and `sync_rows` truncates the moved body's own latch. A moving body can therefore be frozen for one frame if its island's other rows are latched. Warm-start admits this cost (warm_start.rs:37-39); the box-axis cache and `SoftRigidReaction` share it.

**F-4 (Minor, dead data): authored bytes that are never read.**
- `RigidBodyMass.inv_inertia` is overridden at gather: `resources.rs:3460` `let inv_inertia_local = local_inv_inertia(collider.shape, mass.inv_mass);`, and `resources.rs:3437-3439` "auto-overriding any value authored on ... (which is retained for custom authoring but recomputed here)". A grep finds no other production reader.
- `Collider.layer`/`mask` are never gathered: `components.rs:156-158` "foundation broadphase does an unfiltered all-pairs; the fields are wired for the Phase-10 broadphase".
- The derived local and world inertia is recomputed for every body every step, although it depends only on the shape, inverse mass and spawn rotation (resources.rs:3460-3464).

**F-5 (Performance, pyramid context): the pyramid runs the serial O(n²) broadphase.**
- No grid, and no parallel broadphase: `systems.rs:299-300` `for i in 0..n { for j in (i + 1)..n {`. That is 1241·1240/2 = 769,420 bound tests per step, each calling `body_bounding_radius` (a sqrt for boxes) twice (systems.rs:302).
- Its share of the step time is **not measured**. It belongs next to narrowphase in the serial-fraction accounting.
- Other serial per-substep work (colored.rs):
  - gravity: `:3246-3247` "Gravity integrate DYNAMIC bodies (shared O1 kernel). Single-threaded";
  - `warm_start_apply` over every slot: `:1732` `for i in 0..view.len() {`;
  - position integrate and inertia refresh (`:3277-3289`).
- Serial once per step: `build_bodies`, `build_columns` and `store_and_swap` (`:3038` loop), plus gather and apply.

**F-6 (Architecture): each step holds three copies of velocity and four of pose.**
- The chain is RigidBody → BodyState (gather) → BodyEffective (`build_bodies`) → BodyState (`write_back` / `position_integrate`) → RigidBody (apply) → Transform (sync).
- The AoS mirror has its own cost: `resources.rs:176-178` "the SoA kernel MEASURED ~1.6× SLOWER on the AoS `BodyState`".

**F-7 (Performance): immutable soft topology is re-coloured every frame for every body.**
- `soft/colored.rs:830` `scratch.topology_colored = false;` after each body, then `color_topology_once` recolours distance and volume on the next frame and the next body (836-857). One shared scratch serves all bodies, which is also why bodies cannot be stepped with `par_iter`.

**F-8 (Gap): gameplay cannot observe sensor overlaps or contacts.**
- `sensor_overlaps` holds `BodyIndex` rows with no entity mapping and no consumer. `Contact` has no producer. `Entity` is not a `QueryData`.

## 4. Testing "ECS-native == cache-optimal" against the solver hot loop

- **True for the primitive.** Solver body access is random by `body_a`/`body_b`: `row_ptr(i)` = base + i·stride on a `ComponentPool` for a ScratchColumn and equally for a DenseStore (`dense_store.rs:169` `column: ComponentPool::new(component_id.get(), reserve_rows),`). BodyEffective rows are one cache line each. The base is a 64 KiB reservation plus a stagger that is a multiple of 64 B (constants.rs:168-172), and the stride is 64 B (arith.).
- **It held only after a fix.** `constants.rs:187-189` "Staggering each pool's in-reservation base by `(component_id % 64) × CACHE_LINE_SIZE` returns the heap-`Vec` behavior (scattered offsets ⇒ spread across sets) measured to recover the ~40% rigid-solver regression from the P2 ScratchColumn migration".
- **The stagger comes with an obligation the kernel leaves to the caller.** `constants.rs:201-203` "A subsystem that sweeps many columns at index `i` in one hot loop (a "cohort") therefore has an obligation the kernel cannot discharge for it". Physics meets it today by hand-placing 90 ids (scratch_ids.rs:14-19). Production ids are minted upward in first-touch order (`scratch_ids.rs:14-16`), so if solver state moved into real (dense) components, whether a cohort's ids stay distinct mod 64 would depend on the order types were registered.
- **Layout: the solver's hot record is not the authored layout.** BodyEffective packs `inv_mass` + world inertia + both velocities into one line. RigidBody (hot, 52 B) and RigidBodyMass (cold, 48 B) are split, and world inertia is derived state refreshed every substep (colored.rs:3286-3289). So "ECS-native" cannot mean "solve on the authored columns"; a derived per-body solver record is needed either way. The open question is only whether it lives in per-step scratch or as a persistent, slot-stable dense component.
- **Tick cost.** Dense and table storage is tracked (dense_store.rs:169 uses `ComponentPool::new`). scratch_column.rs:70-74 records that tracked storage costs "8 B/row of change detection the scratch path never reads: a 192 KiB resident floor per column and 3.0x the commit charge". The component macro has no untracked storage option: a grep for "untracked" in `boyko_macros/src` finds nothing.
- **Conclusion.** The claim is true for the storage primitive. It is not true by default for layout or id assignment. It becomes true only with three capabilities: cohort-aware stagger or id assignment (today a physics-local band), a derived solver record, and untracked storage for components.

## 5. Ledger cross-check

C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/ledger/physics-scene-math.json has 94 rows. It inventories only containers (Vec, HashMap, Box), so component data, the ScratchColumn mirrors and the dead fields are out of its scope. Its row list matches the grep census. It disagrees with the tree here:

1. **The "Resource+VmColumn" destination** (RefcountDeltas, DeferredFree, InternerState::strings) does not type-check. `vm_column.rs:70` "NOT `Send`/`Sync` (the `NonNull` inside `VmReservation` and `base`)" conflicts with `resource.rs:42` `pub trait Resource: 'static + Send + Sync + Sized {`. It would need an `unsafe impl` per owner, which is a per-crate adapter. VmColumn also requires `size_of::<T>()` to divide the commit granule (vm_column.rs:33-35). `ScratchColumn` is Send+Sync (component_pool.rs:2384-2385), but it needs a registered id, and boyko_scene has no id band (the ledger itself notes this).
2. **SoftBody: all 30 fields are sent to the same "dense component / SegmentedColumn" destination.** 10 of them are per-substep scratch (component.rs:119-177). Because bodies are stepped serially, their natural home is shared per-step scratch, not a per-body segment.
3. **SoftBody soundness.** The ledger says the fields are "sound today only because no array is ever resized after construction". That holds only for in-crate code; the fields are `pub`, so safe code can resize them or write out-of-range indices (F-2).
4. **`SoftCols` pointer lifetime** is described as "for one dispatch/substep". It is actually per constraint call on the serial path (solver.rs:547-548) and per colour on the colored path. No window crosses a grow, so the conclusion stands; only the wording is imprecise.
5. **`asleep`/`below_count`.** The ledger repeats "the latch follows the body" without noting the swap-remove shift (F-3).
6. **TransformPropagationScratch → ScratchColumn** misses a blocker: the `std::mem::take` pattern (propagation.rs:247-255) cannot work because "`ScratchColumn` has no `Default`" (resources.rs:767-768). `remove_resource` exists (resource_api.rs:33); I did not check whether re-inserting allocates.
7. **Agreed:** `frozen_islands`/`energy` → ScratchColumn in the graph cohort (8 + 2 < 64); the debug-only `vec!` at resources.rs:2965; the construction-time locals are class B.

## 6. Final table

These are candidates only; the architect and owner decide.

| Datum | Today's form | Natural ECS form (candidate) | Why |
|---|---|---|---|
| RigidBody pose + velocity | Table component (components.rs:29) | Keep as table component; one pose datum for dynamic roots | Pose lives in RigidBody and Transform with two sync systems (scene_sync.rs:8) |
| RigidBodyMass | Table component, 36 of 48 B never read | Table component without `inv_inertia`; derived inertia cached per entity | Recomputed every gather (resources.rs:3460-3464) |
| Collider.shape / layer / mask | Table component; layer/mask never read | Keep shape; wire layer/mask into broadphase or drop them | components.rs:156-158 |
| Simulated / Kinematic | Bitset EnableTag | Keep | O(1), order-preserving IsEnabled (components.rs:45-46) |
| Sensor / RigidBody-less collider | Zero-size marker; not gathered | Gather must admit collider-only bodies (static / sensor class) | F-1 |
| Contact | Table component, no producer | Kernel event or relationship produced from manifolds | Unobservable today; needs Entity projection (systems.rs:191) |
| PhysicsConfig.dt / .broadphase | Resource, written per step | Split user config from per-step derived state | Two systems write the config (systems.rs:217, broadphase_policy.rs:192) |
| BodyState | ScratchColumn 511, per-step 156 B AoS mirror | Remove as a mirror: pose from components, derived inertia in a dense per-body record | 193.6 KB copy per step; AoS blocks SIMD (resources.rs:176-178); row invariant (systems.rs:1153) |
| BodyEffective | ScratchColumn 510/509, per step | Dense, untracked, slot-stable solver-body component (slot = BodyIndex) | Already one line per row; slots never move (dense_store.rs:6-8); needs untracked storage and cohort ids |
| touched | ScratchColumn<BitSet256> 476 | Goes away if the solver writes the dense record directly | Exists only to map rows back |
| vn_initial (serial) | ScratchColumn 477 | Keep as scratch | Per step, per point |
| ContactPairs / Manifolds | ScratchColumns 446 / 445 | Keep as scratch, keyed by stable slot | Per-step currency |
| BroadphaseGrid | 13 ScratchColumns | Keep as scratch (persistent proxies only if the broadphase becomes incremental) | Per step; unused in the pyramid (F-5) |
| sensor_overlaps | ScratchColumn 444, row-keyed, no reader | Kernel enter/exit events or an "overlaps" relationship on the sensor entity | Gameplay has no path to it (F-8) |
| Warm tables + box-axis cache | Two persistent row-pair hash tables | One persistent per-pair record keyed by stable ids | Duplicated job; both miss on swap-remove (warm_start.rs:37-39) |
| ConstraintGraph | 8 ScratchColumns | Keep as scratch | Island ids volatile (resources.rs:3008) |
| ContactColumns | 31 ScratchColumns | Keep (this is the cache-optimal SoA) | Per-step working set |
| frozen snapshot | ScratchColumn 468 | Remove via an integrate masked by a Sleeping tag | Exists because kernels are unmasked (colored.rs:3183) |
| IslandSleep.asleep | Vec<bool>, row-keyed | Sleeping EnableTag per entity | Follows the entity; queryable. Kernel has only a read datum (data_is_enabled.rs:1), so writing from a system needs commands or a new in-place toggle |
| IslandSleep.below_count | Vec<u16> | u16 in the dense solver-body record | Per-entity debounce |
| frozen_islands / energy | Vec, per island | ScratchColumn in the graph cohort | Per-step scratch |
| awake_rows | ScratchColumn 475 | Derived from the Sleeping tag | Per step |
| SoftBody particle state (10) | Vec in a component | Kernel 1:N segmented dense column | Principle 0 "one contiguous buffer"; closes F-2 if fields become private |
| SoftBody topology (10) | Vec in a component | Shared asset or segment, with its colouring stored alongside | Immutable, yet re-coloured every frame (F-7) |
| SoftBody scratch (10) | Vec in a component, per body | Shared per-step ScratchColumns | Scratch only; bodies are stepped serially |
| SoftColorScratch | 19 ScratchColumns | Self-pair graph stays scratch; distance/volume colouring moves with topology | F-7 |
| SoftRigidReaction | ScratchColumns 442/441, row-keyed | Per-entity external-impulse accumulator consumed at gather | Removes the second row walk (coupling.rs:544); no impulse API exists today |
| Transform / GlobalTransform | Table component / per-frame exclusive system | Keep | Canonical pose |
| TransformPropagationScratch | Vec ×3 | ScratchColumn on a scene id band plus a new borrow split | `mem::take` (propagation.rs:250) |
| RefcountDeltas | Vec | Kernel event (EventBuffer: preallocated per-thread lanes, event_buffer.rs:185-193) | Push-in-hook, FIFO drain = event shape |
| DeferredFree | Vec + `remove(i)` | Send+Sync kernel column with a stable-partition drain | Ledger destination does not compile (§5.1) |
| Joints | Absent | Entities or relationship edges plus dense constraint rows | Nothing exists yet |

## Open list (lens data)

1. Owner call needed: should the Trigger bundle (Collider + Sensor, no RigidBody; bundles.rs:83-84) and 'collision-only static' bodies with no RigidBody (components.rs:56-57) take part in collision? Today physics_gather (systems.rs:202-205) never sees them. No test covers the Trigger bundle.
2. F-2 has not been reproduced: in a release build, safe code can write an out-of-range index into SoftBody (pub c_a, component.rs:91) and the next step does an unchecked raw read (soft/solver.rs:580-582). A reviewer or tester needs to confirm it with a test or Miri.
3. The time share of the serial AllPairs broadphase on the Jolt pyramid is not measured (769,420 bound tests per step, arith.). The bench never selects Grid, and parallel_broadphase does nothing on AllPairs (resources.rs:222-223). A Grid/Auto A/B is needed before the Jolt residual is attributed to the serial stages already listed. This needs owner permission for timings on the workstation.
4. The census gate tests/physics_vec_side_store_census.rs (ad0ebea4) is not in D:/wt/joltab's history (merge-base --is-ancestor says NOT an ancestor). The 34-Vec count is unguarded in this tree.
5. The kernel has three gaps before solver state can become dense components without losing performance: (a) cohort-aware id/stagger assignment for production ids (constants.rs:201-203; physics uses hand-placed ids, scratch_ids.rs); (b) an untracked storage option for components (DenseStore uses the tracked ComponentPool::new at dense_store.rs:169; no 'untracked' in boyko_macros); (c) a way to toggle an enable bit in place from a system (only the read datum IsEnabled exists).
6. Entity is not a QueryData (no impl in iters/query/data/*.rs; systems.rs:191). This blocks entity-keyed sensor/contact events and a row-to-entity map.
7. The ledger's 'Resource+VmColumn' destination does not compile: VmColumn is !Send/!Sync (vm_column.rs:70) and Resource requires Send+Sync (resource.rs:42). A Send+Sync kernel column, or a scratch-id minting API for crates other than physics, needs deciding.
8. Sizes are arithmetic, not size_of or LSP-hover checked: BodyState 156 B, RigidBody 52 B, RigidBodyMass 48 B, Collider 24 B, ContactColumns about 105 B per slot. Manifold 152 B is pinned by a const-assert. Counts of pairs, manifolds, contact slots and colours on the pyramid cannot be derived statically.
9. The DeferredFree doc (asset_refs.rs:143-144, 'nothing drains it until F6') contradicts D:/wt/joltab/crates/boyko_app/src/runner.rs:280 ('Drained every frame by retire_deferred_frees').
10. F-3 is reasoned from the code, not reproduced: after a despawn the moved body inherits the removed row's IslandSleep latch (the kernel swap-removes, archetype.rs:1220), and it can be frozen for one frame. The claim at resources.rs:3013-3014 is false under structural change.

---

# Lens 2 of 4: logic

# LOGIC lens: the physics system graph in D:/wt/joltab @ ca582e72, and where its parallelism comes from

## TL;DR
- On the Jolt pyramid, physics is **8 ordinary systems in one strict `.after` chain**. The scheduler runs **none** of them concurrently, and each one is a separate dispatcher round on a worker. **All** of the parallelism is one hand-written `pool.scope` per wide colour, inside `physics_solve_colored`. Every other stage is a serial loop over all bodies or all pairs.
- **Two of the brief's facts are wrong for this tree:**
  - **Fact (c), broadphase:** the pyramid bench never takes the grid path. It runs the **O(n²) `AllPairs` loop**, which is 769,420 bound tests per step. `parallel_broadphase = true` does nothing there. The `MIN_PARALLEL_BODIES = 4096` explanation describes code that does not run in this bench.
  - **Fact (d), nested scopes:** it is out of date. The shipped KE16 pool (A1f+B1) pushes a worker's spawns onto that worker's **own registered deque**, where siblings can steal them. `par_iter` inside a system now measures **14.6× over sequential** (1.004× the dispatcher route).
- So the kernel **can** run a parallel loop from inside a system today, but only over **archetype component rows**. It cannot do it over:
  - dense components (rejected at compile time);
  - resource-owned `ScratchColumn`s, which is where Stage 4 moved all of physics' bulk data;
  - CSR-cut work such as colours;
  - a chain of dependent phases (there is no barrier).
- The in-scope barrier has **no design document** in `docs/threadpool/`. It exists only as a one-paragraph proposal in `docs/OPEN-QUESTIONS.md`.

## 0. Corrections to the brief

**C-1: fact (c). The pyramid broadphase is `AllPairs`, not a grid below 4096.**
- The bench wires `add_physics_colored_solve` (`jolt_parity_pyramid.rs:207`: `let _keys = add_physics_colored_solve(&mut builder, &mut world);`). It sets only `cfg.parallel_solve = workers > 1; cfg.parallel_broadphase = workers > 1; cfg.sleeping = false;` (`:214-216`).
- The pipeline keeps the default kind off the coupling path (`plugin.rs:461`: `PhysicsConfig::default().broadphase`). That default is `broadphase: BroadphaseKind::AllPairs,` (`resources.rs:437`), and `broadphase_select: BroadphaseSelectMode::Manual,` (`resources.rs:440`) stops the policy from switching it (`broadphase_policy.rs:186-187`: `if cfg.broadphase_select != BroadphaseSelectMode::Auto { return;`).
- So the running arm is `for i in 0..n { for j in (i + 1)..n {` (`systems.rs:299-300`). The flag's own doc says it "is a no-op for the shipped all-pairs loop" (`resources.rs:222-223`).
- Measured cost: "at 1240 bodies the all-pairs pass is ≈0.9 ms of an 18.7 ms step" (`docs/OPEN-QUESTIONS.md:88`).
- `docs/OPEN-QUESTIONS.md:5519` ("serial at 1240 bodies (`MIN_PARALLEL_BODIES = 4096`)") gets the mechanism wrong.

**C-2: fact (d). A nested scope opened from a worker is parallel on this pool.**
- Placement: `Some(lane) => push_on_lane_no_wake::<P>(inner, lane, task),` (`worker.rs:752`) leads to `lane.deque().push(task);` (`worker.rs:713`). The doc: "`inner.stealers[lane.wid]` is the stealer of exactly this deque, so the wave is reachable by every sibling's `try_steal_random`" (`worker.rs:696-697`). The deques are `Worker::new_fifo()` (`thread_pool.rs:682`).
- The joiner is B1 (`scope.rs:20-24`: "A joining WORKER … pops its own oldest chunk … batch-steals into that same REGISTERED deque … parks idle-marked (rule B1-P)").
- Measured: `par_in_system/65536` **89 886 µs** vs `seq` 1 311 400 µs, "speed-up over `seq` | **14.6×**" (`docs/threadpool/KE16-RESULTS.md:1724-1728`). Also "`par_iter` inside a scheduled system is now the SAME as from the dispatcher — 1.004 on gnu" (`:1730`).
- The 1.01× figure came from the pre-KE16 pool (memory note of 2026-08-30, `D:/wt/aether`).

**C-3: arithmetic in the owed-measurement entry.**
- `docs/OPEN-QUESTIONS.md:5521` says "a ~500-slot colour yields 9 chunks".
- The code is `let by_work = ((span.1 - span.0) / MIN_SLOTS_PER_CHUNK).max(1);` (`colored.rs:2760`), which is integer division. 500/64 gives **7** chunks. 9 chunks needs 576–639 slots.
- The conclusion still holds: at W=8, 7 chunks leave one lane idle; at W=16 they leave 9 idle.

## 1. Wiring: which pipeline, and who calls it

- `add_physics_pipeline` (`plugin.rs:409`) is the only wiring function. The `add_physics_*` wrappers choose flags.
- **`boyko_app` does not wire physics in this tree.** Neither `crates/boyko_app/Cargo.toml` nor `crates/boyko_app/src` mentions physics. The only callers outside the crate are two `boyko_render` tests (`tests/bundles_s6_integration.rs`, `tests/interp_pair_b1.rs`).
- Integration ownership: `if S::default().owns_integration() { IntegrationMode::SolverOwned` (`plugin.rs:527-528`). `ColoredSoftStepSolver::owns_integration` returns `true` (`colored.rs:3374-3375`).

## 2. The system table (bench path in bold; other variants listed)

All systems go into the caller's single schedule. There are no sets, and the plugin makes no call to `builder.build`. None of them takes `&mut EcsMaster`, so none is dispatcher-only (`schedule.rs:1082`: `if self.systems[i].kind.runs_on_dispatcher() {`). Every one is `scope.spawn`ed onto a pool worker (`schedule.rs:1341`: `scope.spawn(move || {` → `:1362` `(*system_slot).system.run_unsafe(cell_copy);`).

| # | System | Ordering (`plugin.rs`) | Declared access | Internal work | Parallel? |
|---|---|---|---|---|---|
| **1** | `physics_integrate` | head, no `.after` (`:559`) | `Query<(&mut RigidBody, &RigidBodyMass, IsEnabled<Simulated>)>`, `Res<PhysicsConfig>`, `Res<FixedTime>`, `Res<IntegrationMode>` (`systems.rs:137-140`) | **Returns immediately on this path** (`systems.rs:144-145`: `if *mode == IntegrationMode::SolverOwned { return;`). Otherwise `query.par_iter_mut()` (`:149`) | Kernel `par_iter` exists but never runs here. Still costs one dispatch round per step. |
| **2** | `physics_gather` | `.after(integrate)` (`:561`) | 6-term read `Query`, `ResMut<SolverScratch>`, `ResMut<PhysicsConfig>`, `Res<FixedTime>` (`systems.rs:202-212`) | Serial `for (...) in query.iter()` copies 1241 rows into `SolverScratch.bodies` (`:244`) | serial |
| **3** | `select_broadphase` | `.after(gather)` (`:568`) | `Res<SolverScratch>`, `ResMut<PhysicsConfig>`, `ResMut<PhysicsStats>` (`broadphase_policy.rs:174-176`) | O(1). In Manual mode it only counts (`:182-188`) | serial, trivial |
| **4** | `physics_broadphase` | `.after(select)` (`:569`) | `Res<SolverScratch>`, `Res<PhysicsConfig>`, `ResMut<BroadphaseGrid>`, `ResMut<ContactPairs>` (`systems.rs:281-284`) | **AllPairs O(n²)**: 769,420 tests at n=1241 (C-1) | **serial**, about 0.9 ms (measured, `OPEN-QUESTIONS.md:88`) |
| **5** | `physics_narrowphase` | `.after(broadphase)` (`:572`) | `Res<SolverScratch>`, `Res<ContactPairs>`, `ResMut<Manifolds>` (`systems.rs:358-360`) | Serial `for &(a, b) in pairs.pairs() {` (`:378`) → `box_box_contact(` (`:416`), full SAT per pair. From the transcribed geometry I get **9,570 candidate pairs**, 1,240 of them box–floor: the floor's bounding radius is \|(50,1,50)\| = 70.7, so every box is a floor candidate. This is derived from the geometry, not measured. | **serial**, cost not measured |
| **6** | `physics_build_graph` | `.after(narrowphase)` (`:603`) | `Res<SolverScratch>`, `Res<Manifolds>`, `ResMut<ConstraintGraph>` (`systems.rs:1006-1008`) | `graph.build(...)` (`:1023`): reset, union-find, flatten, greedy colouring, all serial (`resources.rs:2619-2622`) | **serial** |
| **7** | `physics_solve_colored` | `.after(narrowphase)`, `.after(graph)` (`:621`, `:635`) | `ResMut<ColoredSoftStepSolver>`, `Res<PhysicsConfig>`, `Res<Manifolds>`, `Res<ConstraintGraph>`, `ResMut<SolverScratch>`, `ResMut<IslandSleep>` (`systems.rs:1062-1067`) | See §3 | **The only parallel stage.** Physics-local `pool.scope` per wide colour |
| **8** | `physics_apply` | `.after(solve)` (`:638`) | `Query<Mut<RigidBody>>`, `Res<SolverScratch>` (`systems.rs:1127`) | Serial `for mut body in query.iter_mut() {` (`:1131`) with row-order addressing | **serial** |

**Other variants (not on the pyramid path):**
- `physics_narrowphase_sdf`: serial per body (`systems.rs:574`).
- `physics_solve_step::<S>`: generic solver.
- `physics_soft_step`: serial `for body in query.iter_mut()` (`soft/solver.rs:95`).
- `physics_soft_step_coupled` (`soft/solver.rs:121-128`).
- `physics_soft_step_colored`: physics-local `pool.scope`, `soft/colored.rs:1071`/`:1122`.
- `physics_soft_rigid_apply`: serial, `soft/coupling.rs:544`.
- `sync_transform_to_body` (`scene_sync.rs:85`), `sync_body_to_transform` (`:149`), `debug_assert_dynamic_bodies_are_roots` (`:197`).

**Scheduler-level concurrency inside physics: none.**
- Every stage `.after`s its predecessor, and the resource conflicts (e.g. `ResMut<SolverScratch>` in gather vs `Res` downstream) would serialise them anyway.
- The dispatcher parks between rounds (`schedule.rs:741`/`:752`: `if dispatched == 0 && ... > 0 {` … `std::thread::park_timeout(PARK_TIMEOUT);`). That makes 8 round-trips per step, and each one wakes a worker and then the dispatcher.
- The per-round cost is **not measured in this tree**. The bench does not use the host's `timeBeginPeriod` guard, so a missed wake costs the machine's timer quantum (`schedule.rs:74-78`).

## 3. Inside `physics_solve_colored`: the only parallel stage, and its serial phases

It runs on a **worker** (see §2). It calls `pool.scope` through the ambient pool, which is set on workers (`tls.rs:516-517`: "On a worker thread it is set at `worker_main` entry").

- **Serial before the loop:**
  - `has_dynamic` scan (`colored.rs:3156-3159`);
  - `self.build_bodies(scratch.bodies());` and `self.build_columns(...)` (`:3178-3179`).
- **Per substep** (`for _ in 0..substeps {` `:3245`, `substeps: 4` `resources.rs:431`):
  - serial `simd::apply_gravity(...)` (`:3251`);
  - serial `Self::warm_start_apply(...)` (`:3259`), which walks every contact with `for i in 0..view.len() {` (`:1732`);
  - `solve_all_colors` with bias (`:3262`);
  - serial `simd::position_integrate` (`:3279`) and `simd::refresh_inertia` (`:3288`);
  - `relax_iterations` (2, `resources.rs:432`) more `solve_all_colors` (`:3292-3302`).
- That gives **12 colour sweeps and 16 serial kernels per step**.
- **Serial after the loop:** `apply_restitution` (`:3307`), `store_and_swap` (`:3310`), `write_back` (`:3339`).
- **Whole-solve gate:** `config.parallel_solve && self.columns.widest_color_slots() >= MIN_PARALLEL_SLOTS_PER_COLOR` (`:3242-3243`).
- **Per colour** (`solve_color_parallel`):
  1. Inline if `color_slots < MIN_PARALLEL_SLOTS_PER_COLOR` (`:2707`).
  2. Otherwise `n_chunks = by_lanes.min(by_work).clamp(1, n_groups)` (`:2759-2761`), and inline if `n_chunks < 2` (`:2770`).
  3. Otherwise `pool.scope(|scope| {` (`:2867`), one `scope.spawn(task(cut))` per cut (`:2931-2933`). The scope's Drop is the barrier between colours.
- **Scalar solve kernel:** the bench does not set `simd_solve`, and the default is `simd_solve: false,` (`resources.rs:456`).
- **Thread roles during the solve:** the calling worker is the B1 joiner. The spawns land on its own deque (C-2), and siblings steal them. The dispatcher thread is parked in the executor loop, not inside a join, so it is **not** a lane for the solve.

## 4. Gating constants and their effect at 1241 rows

| Constant | Value | Site | Effect on the pyramid |
|---|---|---|---|
| `MIN_PARALLEL_SLOTS_PER_COLOR` (rigid) | 256 | `colored.rs:227` | Colours under 256 slots solve inline. If even the widest colour is under 256, the whole solve runs serially. |
| `CHUNKS_PER_WORKER` (rigid) | 6 | `colored.rs:256` | `by_lanes` = 48 at W8 and 96 at W16. It only binds for colours of 3,072+ slots (W8), so it is **effectively dead** here. |
| `MIN_SLOTS_PER_CHUNK` | 64 | `colored.rs:309` | **This one sets the chunk count.** 256→4, 500→7, 576→9, 1024→16 chunks, the same at W8 and W16. Any colour under 1,024 slots leaves lanes idle at W=16, which is structural. |
| `COHORT` | 8 | `colored.rs:319` | Only applies with `simd_solve`; off in the bench. |
| `MIN_PARALLEL_BODIES` / grid `CHUNKS_PER_WORKER` | 4096 / 4 | `resources.rs:650` / `:642` | Not reached, because the grid arm is not taken (C-1). |
| `GRID_LO` / `GRID_HI` | 96 / 192 | `broadphase_policy.rs:79` / `:88` | Only in Auto mode; the bench is Manual. Auto would pick Grid at 1241 bodies, which is still serial because 1241 < 4096 (`resources.rs:1720`). |
| `MIN_ARCHETYPE_FOR_PARALLEL` | 1024 | `par_iter.rs:73` | Kernel default cut: `raw.clamp(self.min_batch_size, ...)` (`par_iter.rs:124`). At 1241 rows, `chunk_size` = 1024, so **at most 2 chunks for any W**. Irrelevant here (integrate returns early) but binding for any future `par_iter` over bodies. |
| soft `MIN_PARALLEL_SLOTS_PER_COLOR` / `CHUNKS_PER_WORKER` | 256 / 6 | `soft/colored.rs:63` / `:70` | Not on the pyramid path. `let n_chunks = (lanes * CHUNKS_PER_WORKER).clamp(1, total);` (`:1079`) has **no per-chunk work floor**. That is the over-chunking shape the rigid solver measured as negative scaling (`colored.rs:261-268`). |

Minor: the rigid site has no `lanes < 2` guard. At W=1 with `parallel_solve = true`, `by_lanes = 6` still dispatches. The bench avoids this with `workers > 1`.

**Amdahl budget check (arithmetic on recorded numbers):**
- With T1 = 20.311 ms and T8 = 11.931 ms (`OPEN-QUESTIONS.md` 2026-09-10 table), a perfectly parallel remainder implies about 10.7 ms of serial-equivalent time at W8.
- The only measured serial stage, broadphase at ~0.9 ms, explains **under 10 %** of that.
- The rest is somewhere in: narrowphase (9.5k SATs), graph build, the solve's 16 serial kernels plus build/store, inline narrow colours, chunk quantisation, and 8 dispatch rounds. **None of these is measured.**

## 5. Can a system express "parallel over this data" through the kernel today?

- **Archetype component rows: yes, and it works from a worker.**
  - `Query::par_iter` / `par_iter_mut` (`query.rs:542`, `:567`) and `par_for_each_chunk` (`query.rs:682`) open `pool.scope(|scope| {` (`par_iter.rs:341`) through `try_with_active_pool` (`:331`).
  - On the KE16 pool that is real parallelism (C-2).
  - The limit is the default cut (≤2 chunks at 1241 rows). `BatchingStrategy` can override it per call (`par_iter.rs:164`).
- **Dense components: no.** Rejected at compile time (`par_iter.rs:306-309`: `!D::HAS_DENSE && !F::HAS_DENSE, "a dense (storage = \"dense\") term is not supported on `par_iter` in D3`; same guard at `par_chunk.rs:122`). The owner ruled this "waits for Stage 4 to finish" (`OPEN-QUESTIONS.md:5223-5225`).
- **Resource-owned `ScratchColumn` / `DenseStore` (where Stage 4 put physics' bulk data): no kernel driver.**
  - The kernel supplies the safe *view*: `ScratchSolveView` is "`Copy + Send + Sync` … Exposes per-element `row_ptr(i)`" (`scratch/views.rs:227-232`).
  - The fork, join, cut and gates are written by the caller. That happens in three rigid sites plus one soft site with **four different cut policies**: 6×W with a 64-slot floor, 4×W with a 4096-body gate, 6×W with no floor, and the kernel's 1×W with a 1024-row floor.
  - This is what "a per-crate adapter" looks like in the parallelism dimension: the storage is the kernel's, but the parallel loop over it is physics'.
- **Dependent phases (colour sweeps, and substep kernels between sweeps): no primitive.**
  - `Scope` exposes `pub fn spawn` (`scope.rs:1120`) and `pub fn spawn_batch` (`:1156`), and the latter is just `self.spawn(f);` per body (`:1163`).
  - Each phase is a new scope: `let shared = Box::new(ScopeShared::new(` (`thread_pool.rs:328`), a block chunk on first spawn, a join on drop (`scope.rs:1281`), then `self.block.free_all()` (`:1333`).
- **Row-indexed parallel gather/apply: not expressible.**
  - gather/apply address `SolverScratch` by archetype-row order (`systems.rs:1130-1152`), and "`Entity` is not a `QueryData`" (`systems.rs:190-191`).
  - The `par_iter` body receives `D::Item` only (`par_iter.rs:245`: `Body: Fn(D::Item<'_>) + Send + Sync`).
  - Whether `par_for_each_chunk`'s `ChunkItem` exposes a row offset: **not verified**.

## 6. The in-scope barrier: what is on record, and what it would need

- **Design on record:** one paragraph. "The fix is one `pool.scope` per STEP instead of 72, which needs an in-scope BARRIER on `boyko_threadpool::Scope` … The wait logic already exists inside `Scope::drop` and is extractable" (`docs/OPEN-QUESTIONS.md:5145-5147`).
- **Ruling:** "The in-scope barrier waits for KE16 to CLOSE" (`:5212`). Order: "finish Stage 4 … → close KE16 → in-scope barrier" (`:5227`).
- A grep for "in-scope barrier" across `docs/` and `crates/` of this tree finds only `OPEN-QUESTIONS.md`, two archive files and `colored.rs`. **There is no design document in `docs/threadpool/`.**

What the code says it would need (derived by me, not a design):
1. **A mid-scope join separate from teardown.** `unsafe fn join_workers_until_drained(inner: &PoolInner, shared: *const ScopeShared)` (`scope.rs:1450`) already has the right shape. The free sites must stay at the final drop (`:1333` and the `Box::from_raw` after it).
2. **Reusing the block across phases.** The SAFETY argument at `scope.rs:1317-1331` (every cell dead once `pending == 0`) is exactly what a phase boundary establishes. Reusing it would take the ~330 allocations per step (fact b) down to about 2 per step. This is the allocation-unification lever, separate from speed.
3. **It would not remove the wake/park per phase by itself.**
   - Every push wakes: `wake_is_due` returns `true` (`worker.rs:612-614`).
   - An out-of-work worker or joiner parks idle-marked: `mark_idle(&inner.idle, wid);` (`scope.rs:1572`).
   - The 2026-09-09 entry names "waking and parking W workers" as the per-wave cost (`OPEN-QUESTIONS.md:5142-5143`).
   - A join-style barrier inside one scope saves scope setup/teardown. Only a resident-lane form (W tasks per step, each looping over phases behind a spin barrier) keeps lanes awake across colour boundaries. Jolt's `BarrierImpl::Wait` "executes the first executable job of THIS barrier only" is recorded in-tree as `[R:S]` (`KE16-VARIANTS-ADDENDA.md:156-157`); I did not verify it against Jolt's source.
   - Which of the two costs dominates is unmeasured, so the barrier's specification must state which one it targets.
4. **Bit-identity:** the cross-colour order is the barrier itself. The `{1,N}` property (`colored.rs:2634-2658`) is unaffected as long as every phase edge stays a full join.

## 7. The step as a sequence

```
bench thread = DISPATCHER (external lane)
Schedule::run -> pool.install (schedule.rs:455)
 R1 integrate     -> worker (scope.spawn, schedule.rs:1341) -> returns at systems.rs:145 (SolverOwned)  [dead round]
 R2 gather        -> worker -> SERIAL query.iter(), 1241 rows -> SolverScratch        (systems.rs:244)
 R3 select_bp     -> worker -> SERIAL O(1), Manual: count only                        (broadphase_policy.rs:182-188)
 R4 broadphase    -> worker -> SERIAL AllPairs, 769,420 tests (~0.9 ms measured)      (systems.rs:299-300)
 R5 narrowphase   -> worker -> SERIAL box_box SAT on ~9,570 pairs (derived)           (systems.rs:378,416)
 R6 build_graph   -> worker -> SERIAL union-find + greedy colouring                   (resources.rs:2619-2622)
 R7 solve_colored -> worker Wk (B1 joiner):
      SERIAL has_dynamic, build_bodies, build_columns                                 (colored.rs:3156-3179)
      x4 substeps:
        SERIAL apply_gravity, warm_start_apply                                        (:3251, :3259)
        x3 sweeps (1 bias + 2 relax), per colour c:
          slots<256 or chunks<2 -> SERIAL inline on Wk                                (:2707, :2770)
          else PARALLEL: pool.scope; spawn floor(slots/64) chunks onto Wk's deque;
               siblings steal (A1f); Wk pops/steals, parks idle-marked (B1-P);
               scope Drop = barrier                                                   (:2867-2934)
        SERIAL position_integrate, refresh_inertia                                    (:3279, :3288)
      SERIAL apply_restitution, store_and_swap, write_back                            (:3307-3339)
 R8 apply         -> worker -> SERIAL iter_mut, 1241 rows, Mut change ticks           (systems.rs:1131)
 (between rounds: dispatcher park_timeout; woken by the completer)                    (schedule.rs:752)
```

## 8. Verified-good, worth keeping
- The `{1,N}` bit-identity construction: body-disjoint colours, a per-element `row_ptr` view, a canonical warm store.
- The whole-solve gate fixed to use the colour-width metric (`colored.rs:3226-3243`).
- The `n_chunks < 2` refusal (`:2763-2772`).
- The KE16 pool now makes the kernel's `par_iter` usable from system bodies, which removes the old blocker to "one worker pool, all systems".

## Open list (lens logic)

1. OWED MEASUREMENT: per-stage wall time at W=1 vs W=8 (broadphase / narrowphase / build_graph / the solve split into its 16 serial kernels vs sweeps / inline-colour share / apply / 8 dispatch rounds). Only broadphase is measured (~0.9 ms, OPEN-QUESTIONS.md:88), which is under 10 % of the ~10.7 ms Amdahl serial term at W8. Possible no-edit instrument: the kernel's per-system SystemSpan opened at schedule.rs:1359 (`let _span = crate::ecs::core::profiling::zones::SystemSpan::open(`). Not verified that a criterion bench without the host can arm the profiler.
2. MISSING DATA: the colour/slot histogram of the 1240 pyramid. The '~6 colours / many NARROW colours' premise is inherited from the 10k scene (KE16-DESIGN.md:39: '10 k-body pyramid, 29751 contacts, 6 colors'). The fact-(b) allocation receipt contradicts it: 2.1 allocations/step at W1 matches only the install scope; each dispatched colour scope costs at least a Box<ScopeShared> (thread_pool.rs:328) plus one block chunk; so (331.6-2.1)/2/12 gives up to ~13.7 dispatched colour-waves per pass, which is more than 6 unless a scope takes 2+ chunks. Allocations at W8 (331.6) and W16 (339) are nearly equal, which fits chunk counts set by slots/64 rather than by W*6. Not verified.
3. CORRECT THE RECORD: docs/OPEN-QUESTIONS.md:5519 says the pyramid broadphase is 'serial at 1240 bodies (MIN_PARALLEL_BODIES = 4096)'. It is actually the O(n^2) AllPairs arm (resources.rs:437 plus Manual select, resources.rs:440), and the bench's parallel_broadphase=true is a no-op there (resources.rs:222-223). OPEN-QUESTIONS.md:5521 says '~500-slot colour yields 9 chunks'; integer division gives 7 (colored.rs:2760).
4. STALE FACT SPREADING: the brief's fact (d) and the memory note reference-nested-scope-serialises-on-worker describe the pre-KE16 pool. On this tree a nested scope from a worker is parallel (worker.rs:713, 752; scope.rs:20-24; KE16-RESULTS.md:1728 speed-up over seq 14.6x). D:/wt/merge docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:47-53 ('fact 2') still quotes 0.71x at W=16 and 'allowed to read one worker on this pool'. It needs re-reading against the 2026-09-10 retake (positive scaling 1.33/1.65/1.71/1.54).
5. IN-SCOPE BARRIER HAS NO DESIGN: only OPEN-QUESTIONS.md:5145-5147; nothing in docs/threadpool/. KE16 is closed on this tree, so its precondition (:5212) is met. The spec must say whether it targets scope setup/teardown (Box<ScopeShared> plus block chunk per colour: the allocation-unification lever) or the per-phase wake/park (every push wakes, worker.rs:612-614; idle joiners park, scope.rs:1572). A join-style barrier inside one scope only removes the first. Keeping lanes resident (Jolt-style own-barrier) is the direction that removes the second. Architect's call.
6. KERNEL GAPS behind 'one unified system' (owner/architect decisions, not physics fixes): (1) par_iter/par_chunk reject dense storage (par_iter.rs:306-309, par_chunk.rs:122), ruled deferred behind Stage P (OPEN-QUESTIONS.md:5223-5225); (2) no kernel parallel-for over Resource-owned ScratchColumn/DenseStore solve views, so physics hand-rolls 4 sites with 4 different cut policies; (3) no cut-policy object (deferred behind the barrier, OPEN-QUESTIONS.md:5219-5222); (4) no row index in par_iter items (par_iter.rs:245), which blocks parallel gather/apply; whether par_for_each_chunk exposes a row offset is not verified.
7. BENCH CONFIGURATION CAVEAT: the Jolt parity bench measures the scalar colored solve (simd_solve defaults false, resources.rs:456; the bench does not set it) and the O(n^2) broadphase. A game would likely choose Grid (Auto selects it at >=192 bodies) and simd_solve (recorded 1.96x). Quote the scaling numbers with that configuration named.
8. SMALLER ITEMS: (a) soft/colored.rs:1079 `let n_chunks = (lanes * CHUNKS_PER_WORKER).clamp(1, total);` has no per-chunk work floor, the shape measured as negative scaling on the rigid path (colored.rs:261-268); (b) no `lanes < 2` guard at the rigid site, so W=1 with parallel_solve=true still dispatches 6 chunks; (c) physics_integrate is registered unconditionally (plugin.rs:559) and costs a dead dispatch round per step on the SolverOwned path; (d) doc at colored.rs:205 ('a boxed closure per spawn') predates Stage 3b block emplacement; (e) kernel par_iter default gives at most 2 chunks at 1241 rows (par_iter.rs:73, 124), relevant to any future body-parallel pass.

---

# Lens 3 of 4: plans

# Lens report: what the repository has already decided about physics as ECS

## Scope and provenance

- **Code tree:** `D:/wt/joltab` @ `ca582e72` (`merge/ke16-into-ecsnative`). I read the docs listed in the brief, plus `docs/RESEARCH-SOFT-BODY.md`, `docs/PERF-DIRECTIONS.md`, `docs/aether-v2/KERNEL-BACKLOG.md`, `docs/scheduler/KE17-APPLY-WINDOW-DESIGN.md`, `docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md`, and the census gate at `ad0ebea4`, which is not in joltab (read with `git show`).
- **Main checkout:** `D:/claude/BoykoEngine`. There I read `docs/physics/ADVANCED-PHYSICS-{RESEARCH,DESIGN-SPACE}.md` and `docs/memory/ALLOCATOR-{RESEARCH,DESIGN-SPACE}.md`.
- **Owner-ruling notes:** `C:/Users/flint/.claude/projects/D--claude-BoykoEngine/memory/feedback-ecs-sdk-all-data.md`, `feedback-no-allocator-but-ours.md` and `project-feature-tracks.md`. Where these are in Russian, the quotes below are my translation.
- **Nothing was run.** No cargo, no git state change, no edits. graphify was not used, because this is a docs lens.
- ⚠ **Stale anchors.** ADVANCED-PHYSICS-* cites ecsnative @ `46c8e489`, and its line numbers do not match joltab. It cites `resources.rs:3063-3096` for `IslandSleep`; in joltab the struct is `resources.rs:3051` (`pub struct IslandSleep {`).

## 0. Corrections to the brief, measured on this tree

- **K-1: fact (d) is stale on joltab.** KE16 fixed the nested-scope serialisation.
  - `docs/threadpool/KE16-RESULTS.md:1726`: "| **`par_in_system/65536`** | **89 886 µs** | **98 249.5 µs** |".
  - `:1728`: "| speed-up over `seq` | **14.6×** | 13.3× |".
  - `:1731`: "the finding that opened this campaign recorded it at **1.01× of `seq`**".
  - `:1697`: "| **(1)** `in_scheduled_system < single_threaded_O5` | **PASS, 2.76×** |".
  - What still keeps intra-system parallelism physics-local is not the pool. It is:
    - (i) `par_iter` refusing dense storage: `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs:307`: "!D::HAS_DENSE && !F::HAS_DENSE,".
    - (ii) Physics cutting its work by CSR colour runs, which `BatchingStrategy` cannot express (see R-9).
    - (iii) Physics reaching the pool through TLS: `crates/boyko_physics/src/solver/colored.rs:2738` "let dispatched = try_with_active_pool(|pool| {".
- **K-2: "72 waves" is an upper bound, not a count.** `docs/OPEN-QUESTIONS.md:5505`: ""72 waves per step" assumed every colour"; `:5509`: "the **wave COUNT is W-independent**". ADVANCED-PHYSICS-DESIGN-SPACE fact 2 (`:47-53`) still builds on 72.
- **K-3: every KE16-conditional sentence in ADVANCED-PHYSICS predates the merge.** `ADVANCED-PHYSICS-DESIGN-SPACE.md:37-39`: "on the shipped pool the worker-spawned `pool.scope` route is the serial floor". On joltab that is no longer true (K-1).
- **K-4: the census counts are consistent.** `ad0ebea4:tests/physics_vec_side_store_census.rs:245`: "the scanner in this file measures **34 and 30**". Commit `57e0a512`'s "SoftBody's twenty-six" is the stale figure.
- **K-5: not verified here.** The brief's 2.1 / 331.6 / 331.6 alloc/step and the mimalloc A/B are not in any tracked file I read. The ecsnative-era "2671 allocations/step" still appears in `feedback-no-allocator-but-ours.md` (the "Замер того же вечера" paragraph) — that is the figure the brief corrected.

## 1. Decision register

### A. Kernel storage kinds physics is meant to use

| # | Decision | By | Status | Cite |
|---|---|---|---|---|
| A1 | A third storage kind, `Dense`, built for physics | owner's idea (memory `project-feature-tracks.md:25`: "DENSE (non-fragmenting) COMPONENTS (owner's idea, the chosen design)"); plan | **kernel D0–D4 SHIPPED; physics Stage P NOT** | `DENSE-COMPONENTS-PLAN.md:6` "the solver mirrors them in `std::Vec<BodyState>`… Goal: a third `StorageKind::Dense`" |
| A2 | Stage P: bodies become dense, the Vec mirrors are deleted, the solver goes through `DenseSolveView::row_ptr` — while keeping an AVX re-gather into a `ScratchColumn` as the default | plan | **NOT started**. No physics type is dense; the only production dense type is `GpuTransform3D` (`crates/boyko_render/src/gpu_transform3d.rs:2` "the FIRST production `#[component(storage = "dense")]` type") | `DENSE-COMPONENTS-PLAN.md:26` "RigidBody*/velocity→dense; … DELETE the Vec mirrors; … AVX re-gather into a contiguous `ScratchColumn` is the DEFAULT (W4)" |
| A3 | Dense is CPU-only, for ever | plan | shipped | `DENSE-COMPONENTS-PLAN.md:61` "dense ALWAYS `ResidencyKind::Cpu`" |
| A4 | Dense slot order = tombstone + LIFO free list, not swap-remove | plan | shipped | `DENSE-COMPONENTS-PLAN.md:19` "Deterministic order = tombstone + free-list (EnTT in-place deletion), NOT swap-remove" |
| A5 | Colouring reads absolute slot values, so bit-identity holds only for a fixed op sequence | plan (C3 finding) | standing contract | `DENSE-COMPONENTS-PLAN.md:56` "coloring DEPENDS on absolute body slot values"; `:57` "NOT claimed: identity across different op-orderings" |
| A6 | Keep the gather; move its buffer and all physics bulk onto `ComponentPool`; operate-in-place is deferred to a measured spike | architect ("mine, perf-driven") | **SHIPPED** (Stages 0, 1', 4) | `ARCH-AUDIT-ECS-DATA-REMEDIATION.md:24` "keep the gather, but move its buffer + all physics bulk onto `ComponentPool`… (it must beat the gather's cache win, which is not assumed)" |
| A7 | Use the real `ComponentPool`, not a bespoke primitive | **owner ruling** | shipped (`ScratchColumn` wraps `ComponentPool::new_untracked`) | `feedback-ecs-sdk-all-data.md:16` "the owner rejected it — use the REAL `ComponentPool`"; `crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:30-31` "`ComponentPool::new_untracked`… no bespoke `VmReservation` primitive" |
| A8 | A physics-side `ScratchColumn` wrapper is "glued on"; the bulk-column capability must live in the kernel | **owner ruling** | column type is now kernel; **id allocation is still physics-local** (C-2) | `feedback-ecs-sdk-all-data.md:20` "(A physics-side `ScratchColumn` wrapping `ComponentPool` is itself "glued on" — wrong…)" |
| A9 | Scratch ids come from a reserved band at the top of `MAX_COMPONENTS`, owned by physics | lane (Stage P/4) | shipped | `crates/boyko_physics/src/scratch_ids.rs:541` "const SCRATCH_REGION_MIN_ID: usize = MAX_COMPONENTS - 128;"; `:40` "use boyko_ecs::…::{MAX_COMPONENTS, register_layout};" |
| A10 | Per-cohort cache-set stagger invariant | lane, measured | shipped | `crates/boyko_ecs/src/ecs/constants.rs:188-189` "measured to recover the ~40% rigid-solver regression from the P2 ScratchColumn migration" |

### B. Per-body data and the gather

| # | Decision | By | Status | Cite |
|---|---|---|---|---|
| B1 | `RigidBody` / `Collider` / `Contact` are ordinary CPU-archetype components, split hot/cold | plan | shipped | `RENDER-PHYSICS-GPU-PLAN.md:212-213` "`RigidBody`/`Collider`/`Contact` are ordinary CPU-archetype components"; `crates/boyko_physics/src/components.rs:4-6` |
| B2 | IM-1: gather row = archetype-row order = `BodyIndex`; no structural change between gather and apply | plan | shipped, `debug_assert!` in `physics_apply` | `OPTIMIZATION-PLAN-PHYSICS.md:22` "**No phase may make the live set non-contiguous in the gathered array**" |
| B3 | `Simulated` / `Kinematic` are bitset EnableTags | plan (capability model) | shipped | `components.rs:61` / `:73` `#[component(storage = "bitset")]` |
| B4 | No row→entity map yet, because `Entity` is not a `QueryData` | lane | shipped as an absence; KR-1 open | `resources.rs:3617-3618` "intentionally NOT carried here… `Entity` is not yet a `QueryData`". My grep for a `QueryData` impl on `Entity`/`EntityId` under `iters/query` found none. |

### C. Per-step scratch (S3)

| # | Decision | By | Status | Cite |
|---|---|---|---|---|
| C1 | Stage 4: mechanically swap the S3 `Vec` fields to `ScratchColumn`, layout byte-identical | plan | **SHIPPED** (commit `57e0a512` "Stage 4 in this lane's scope is COMPLETE") | `ARCH-AUDIT…:29` |
| C2 | `ScratchBuildView` gains `resize`/`truncate` as kernel additions (principle 0) | owner-sanctioned, from the lane | shipped (`2d665c85`) | `OPEN-QUESTIONS.md:147-148` "the intended answer is `ScratchBuildView::resize(len, value)` and `::truncate(len)`" |
| C3 | Cached-frontier build view | lane, measured | shipped (`0a803cfc`) | `OPEN-QUESTIONS.md:120-121` "at n = 10 000 the column is **3.4 % FASTER than `Vec`**" |
| C4 | Stage 4 costs nothing end-to-end; the all-pairs loop was 7–17 % slower before C3 | measured | recorded | `OPEN-QUESTIONS.md:74` "**no cost detected, and none below ~2–4 % could have been.**"; `:83` "| 100 | 5.596 µs | 6.545 µs | **+17.0 %** |" |

### D. Cross-frame per-pair and per-row state (S2)

| # | Decision | By | Status | Cite |
|---|---|---|---|---|
| D1 | Warm-start and axis cache stay Resource-owned `ComponentPool`, keyed by pair key, NOT pair-entities | plan | shipped as `ScratchColumn`s (`scratch_ids.rs:308` `warm_table_id`) | `ARCH-AUDIT…:30` "(NOT pair-entities — W2: pair-entity archetype-row order depends on spawn/despawn/recycle order…)" |
| D2 | IM-2b: the warm store walks canonical order | plan | shipped | `OPTIMIZATION-PLAN-PHYSICS.md:24` |
| D3 | Contacts as gameplay-visible ECS data | **owner ballot, open** | `Contact` type exists with no producer | `ARCH-AUDIT…:42` "Contacts as gameplay-visible ECS data… default = pure scratch"; `ADVANCED-PHYSICS-DESIGN-SPACE.md:1282` "**no producer anywhere in the crate**" |

### E. Sleeping

| # | Decision | By | Status | Cite |
|---|---|---|---|---|
| E1 | O8: the gather stays full; only solve and integrate are skipped | plan | shipped (default `sleeping: false`, `resources.rs:480`) | `OPTIMIZATION-PLAN-PHYSICS.md:87` "`physics_gather` STILL walks every row" |
| E2 | Latch per ROW, not per island | lane | shipped, 4 `Vec`s | `resources.rs:3013-3014` "Body ROWS, by contrast, are STABLE across frames (the gather is FULL and dense, IM-1 — rows never shift)" |
| E3 | Target: `Sleeping` component + `EnableColumn` | plan; lane "step 6" | **owner's call, open** (`ADVANCED…:1262-1264` Q12) | `ARCH-AUDIT…:30` "sleep → `Sleeping` component + `EnableColumn` paged-bitset tag"; commit `57e0a512`: "(step 6, they become a Sleeping component plus an EnableColumn)" |
| E4 | Step 6 precedes destruction D0 | design | planned | `ADVANCED…:1003-1004` "**which is why step 6 (its migration) precedes D0** (P-48)" |
| E5 | Wake must not key on `Changed<RigidBody>` | plan | shipped | `OPTIMIZATION-PLAN-PHYSICS.md:91` "The `Changed<RigidBody>` route is REJECTED (W6)" |

### F. Soft body

| # | Decision | By | Status | Cite |
|---|---|---|---|---|
| F1 | The 30 `Vec` fields are an accepted L2 exception "for now" | plan | standing | `ARCH-AUDIT…:16` "accepted L2 exception for now" |
| F2 | D-8: a resource-owned ROW BANK on `ScratchColumn`s, `SoftBody` becomes a dense handle, particles are not entities | design (architect) | planned (S0); **owner call** Q12 | `ADVANCED…:807` "**Decision D-8 (§14.A): component-owned sub-columns, as a resource-owned ROW BANK with per-body ranges**"; `:838` "`SoftBody` becomes a `#[component(storage = "dense")]` handle" |
| F3 | Particle-as-entity rejected by a number | design | — | `ADVANCED…:811` "20k `Commands` spawns/frame = 0.6–2 ms of CPU" |
| F4 | KR-3: ratify `ScratchColumn` for DURABLE data | design | owner Q11 | `ADVANCED…:867-869` "ratify `ScratchColumn`… as the resource-owned column for DURABLE data; the bank is not scratch" |
| F5 | Physics fully in-house (no Rapier/Jolt/parry) | **owner decision** | standing | `docs/RESEARCH-SOFT-BODY.md:6` "Owner decision: **physics FULLY in-house (no Rapier/Jolt/parry FFI).**" |

### G. The advanced rungs — entity models already designed

- **Joints.**
  - The joint is an entity, `JointDef` is a table component, `JointState` is a dense component, and the prepared per-step form `JointColumns` is a scratch cohort (`ADVANCED…:84-121`; `:115-117` "`JointState` is **dense** because it is durable… the exact reason the dense kind was built").
  - The sort key is the authored `order: u16`, never the dense slot (`:130-133` "the next joints spawned fill the freed slots in REVERSE").
- **Row map.** `row_entity` / `entity_row` become `ScratchColumn`s, not `SparseMap` (`:166-167` "this was `SparseMap<u32>`, and that was a Principle-0 violation").
- **Destruction.** `FractureInstance` is dense, `FractureBank` is a resource row bank, chunks are ROWS promoted to entities under a budget (`:941-951`, "the exact reason "every chunk a body from t=0" is refused (P-39, the 0.6–2 ms spawn number)"). Split / generate / apply is ONE single-threaded system (`:993-997`).
- **Character.** A table component plus one system after `physics_apply` (`:651-656`).
- **GPU.** Rigid stays CPU (`:1069` "Rigid stays CPU (branchy, low-N, decision logic").
- **Status:** all design only, rev 2, "no gate was run and no timing was taken" (`:1363-1365`).

### H. Parallelism and scheduling

| # | Decision | By | Status | Cite |
|---|---|---|---|---|
| H1 | The colored solve's parallelism lives in a physics STAGE that owns the pool; no core edits | plan | shipped | `OPTIMIZATION-PLAN-PHYSICS.md:109` "a NEW dedicated stage `physics_solve_colored`… holds the pool handle"; `:265` "**No core ECS edits.** Consumes `pool.scope` as-is." |
| H2 | The in-scope barrier waits for KE16 to close | **owner ruling 2026-09-09** | KE16 now merged; barrier not started | `OPEN-QUESTIONS.md:5212` "**The in-scope barrier waits for KE16 to CLOSE.**" |
| H3 | `Cuts` policy object deferred until after the barrier; `MIN_SLOTS_PER_CHUNK = 64` ships as a compromise | **owner ruling** | standing | `OPEN-QUESTIONS.md:5219-5222` |
| H4 | `par_iter`'s dense rejection waits for Stage 4 to finish | **owner ruling** | **the trigger has fired** (Stage 4 closed at `57e0a512`); KE15 is still UNOWNED | `OPEN-QUESTIONS.md:5223-5225` "**`par_iter`'s dense rejection waits for Stage 4 to finish.**… The kernel fix is raised when Stage P's dense migration actually needs it."; `KERNEL-BACKLOG.md:54` "KE15 \| 🆕 **UNOWNED**… Give the parallel chunk runner a world cell" |
| H5 | Per-stage timing is owed before any next fix | adjudicator | **owed** | `OPEN-QUESTIONS.md:5524` "per-stage wall time at W=1 vs W=8 (narrowphase / broadphase / solve / inline-colour share)" |
| H6 | `parallel_solve` / `simd_solve` default OFF | **owner ruled in half** | standing | `FMA-DETERMINISM.md:655-656` "`simd` now defaults to `true`; `simd_solve` and `parallel_solve` do not"; `resources.rs:477` "parallel_solve: false," |
| H7 | The physics Fixed schedule is a serial chain | measured | fact | `docs/scheduler/KE17-APPLY-WINDOW-DESIGN.md:14` "0 % on a serial chain (which is what the physics Fixed schedule is)" |
| H8 | Parallelism inside a system is a kernel feature, not `pool.scope` in physics | **owner, 2026-09-10** | new, not reflected in any plan | `feedback-no-allocator-but-ours.md` §5 (translated): "parallelism within a system is a kernel feature (`par_iter`/barrier), not `pool.scope` in the physics crate" |

### I. Determinism contract (every migration inherits it)

- **Do not fuse.** `FMA-DETERMINISM.md:18` "Recommendation: `do-not-fuse`."
- **What a backing swap must satisfy.** Byte-identical for a pure backing swap; run-to-run + tolerance for anything that changes traversal (`ARCH-AUDIT…:35`).
- **Colored value vs reference.** The colored value legitimately differs from the reference (`OPTIMIZATION-PLAN-PHYSICS.md:63`).

### J. Allocator (main checkout, rev 1, CHANGES REQUESTED)

- **Target:** zero foreign allocations in steady state (`ALLOCATOR-DESIGN-SPACE.md:19`).
- **Threadpool is in scope:** `:32` "including the threadpool's `Box<ScopeShared>`, the scope block's `std::alloc` chunks, and crossbeam-deque's runtime buffers — is **in scope**". The code facts: `crates/boyko_threadpool/src/block.rs:101` "use std::alloc::{alloc, dealloc, handle_alloc_error};" and `thread_pool.rs:278` "let shared = Box::new(ScopeShared::new(".
- **Physics row in rung 2:** `:321` "`ScratchColumn` / dense components per the existing physics census rungs".
- **New relocating container:** `:70` "The new `HeapVec<T>` **relocates on growth** exactly like `Vec`".
- **Verdict:** `:413` "**CHANGES REQUESTED** — two blockers (C1, C2)".

### K. Resident-floor plan

- `POOL-SUBGRANULAR-PACKING-PLAN.md:48` "| 33 live solver `ScratchColumn`s | 4.13 MiB | **132 KiB** | 32× |". Status is DESIGN (`:3`).

## 2. Conflicts between plans

- **C-1: the owner-chosen dense design vs the architect's "keep the gather" vs what shipped.**
  - The memory note says dense won and the gather dies: `project-feature-tracks.md:25` "the gather + the std::Vec BodyState/BodyEffective mirror are DELETED… SUPERSEDES the ScratchColumn-side-wrapper idea".
  - ARCH-AUDIT decides the opposite: `:22` "the gather is a CACHE OPTIMIZATION… naively deleting it would REGRESS the solver".
  - Even the dense plan keeps a re-gather (`DENSE…:26` "AVX re-gather into a contiguous `ScratchColumn` is the DEFAULT").
  - What shipped is ARCH-AUDIT's option (c). Nothing in the tree measures gather vs in-place (grep of `crates/boyko_physics/benches/` shows no such bench). The regression claim is therefore an **unmeasured assertion** on both sides.
- **C-2: how scratch ids are minted — three incompatible answers.**
  - Dense plan: `DENSE…:87` "`ComponentPool::new` direct (no synthetic-id/new_scratch)".
  - ARCH-AUDIT: `:27` "`ComponentPool::new_scratch(layout, reserve_rows)` (synthetic-id, registry-free…)".
  - Shipped: neither. Ids are registered in the GLOBAL registry from a band physics owns (`scratch_ids.rs:541`, `:610-616`).
  - A second subsystem (animation pose bank, render) has no kernel API to reserve a band. The floor is guarded by a physics-local census (`scratch_ids.rs:527-540`). By A8 this is the "glued on" shape at the id level.
- **C-3: `IslandSleep`'s target form — three texts.**
  - ARCH-AUDIT `:30` and commit `57e0a512` both say "Sleeping component + EnableColumn".
  - The census rung (`ad0ebea4:tests/physics_vec_side_store_census.rs:253-256`) says "become kernel columns".
  - ALLOCATOR rung 2 (`:321`) says "`ScratchColumn` / dense components".
  - ADVANCED O-5 (`:1284`) adds per-body sleep enable/threshold, which "needs a column".
- **C-4: `SoftBody`'s target form.**
  - Census rung (`:260-263`): "the ten particle columns… become dense components".
  - ADVANCED D-8 (`:807`, `:815`): particle columns in a `SoftBank` resource of `ScratchColumn`s, with only the handle dense.
  - ARCH-AUDIT (`:16`, `:43`): accepted exception, with particle-as-entity vs component-owned sub-columns left open.
  - The owner's 2026-09-10 order ranks "resource column" below "dense component" (H8 note §5, translated: "component on an entity → dense component → relation → event → resource column → system scratch on a kernel column"). D-8 picks the lower form without arguing against the higher one for the particle columns.
- **C-5: dense bodies × enable bits do not compose on the fast path.**
  - ARCH-AUDIT puts sleep on an `EnableColumn` and Stage P puts bodies on `Dense`. But `DENSE-ENABLE-QUERY-PLAN.md:81-82` says "the `DenseStore` column is **archetype-agnostic**… while the enable bit is keyed by `(archetype, row)`", and `:85-86` says `dense_iter` "must **compile-reject** any `F` that carries an enable term".
  - `par_iter` also rejects dense (`par_iter.rs:307`).
  - So the two storage kinds prescribed for physics cannot be read together through either fast iteration path. I did not verify whether `IsEnabled<T>` as a data term (the gather's form, `systems.rs:207-208`) is admitted by `dense_iter`.
- **C-6: "physics makes zero core edits" vs principle 0.**
  - `OPTIMIZATION-PLAN-PHYSICS.md:20` "NO `boyko_ecs` core edits (the `plugin.rs` precedent — physics makes zero core edits)".
  - Principle 0 (CLAUDE.md) and H8 require capabilities to become kernel features. The lane already broke the old rule, with owner sanction (C2's `resize`/`truncate`).
  - H1's stage-owns-pool design is the O-plan's answer; H8 overturns it without a replacement plan.
- **C-7: external solver allowed vs forbidden.**
  - Allowed: `RENDER-PHYSICS-GPU-PLAN.md:218-220` "`boyko_physics` MAY later take a Rust dep (Rapier/parry…) **behind the seam**… Jolt-FFI is the other swap"; `PERF-DIRECTIONS.md:254` "CPU-D7 — Rapier/Jolt-FFI behind the `RigidSolver` seam — an option".
  - Forbidden: `RESEARCH-SOFT-BODY.md:6` (owner, fully in-house) and owner order (1).
  - The older two plans were never marked superseded.
- **C-8: the 72-waves and serial-floor premises vs joltab.** See K-1 and K-2. ADVANCED D-1, D-2, S0's count gate and KR-5 all price against the pre-KE16 pool.
- **C-9: `SYSTEMS.md` vs the census.** `docs/SYSTEMS.md:2011-2013` "(no parallel data system — the SP4 race remediation put both solvers on kernel `ScratchColumn`)". This contradicts the 34 `Vec` fields the census pins.
- **C-10: the allocator's `HeapVec`/`FrameVec` vs "every array onto the ECS".**
  - `ALLOCATOR-DESIGN-SPACE.md:93` adds a relocating heap container. `:91` adds `FrameVec` for per-frame lists.
  - The owner's order (2)/(3) and the ECS-form ladder put arrays in ECS storage first.
  - For physics per-step scratch there are now two candidate homes: `ScratchColumn` (kernel, address-stable) and `FrameVec` (memory library, dispatcher-only). No plan says which one wins.
- **C-11: the kernel's own dense store still holds a `Vec`.** `crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:126` "free: Vec<u32>,". The storage physics is told to migrate onto is itself on the ledger (`ALLOCATOR…` rung 2, "`DenseStore.free: Vec<u32>`").

## 3. Physics data kept OUTSIDE ECS forms on purpose, and the stated reason

Each row is a reason the design must answer or overturn with evidence.

- **R-1: the body gather mirror (`SolverScratch.bodies`, `BodyEffective`) instead of in-place dense rows.**
  - Reason: `ARCH-AUDIT…:22` "body access in the colored inner loop is SCATTERED… The gather turns scattered body reads into a dense AVX-friendly mirror"; `DENSE…:64` "scattered row_ptr risks regression vs contiguous gather".
  - Evidence: **none measured.** This is exactly where the "ECS-native = cache-optimal" claim is untested.
- **R-2: `BodyIndex` = archetype-row order (Encoding A), not an entity or slot key.**
  - Reason: `OPTIMIZATION-PLAN-PHYSICS.md:89` "Skipping gather rows would shift every subsequent `BodyIndex`, trip the assert, and re-map every warm key".
  - Moving to dense slots changes the colouring and the values (A5), so it needs a tolerance gate, not a byte gate.
- **R-3: `IslandSleep`'s per-row `Vec` latch instead of a per-entity tag.**
  - Reason: `resources.rs:3008-3014` island ids are unstable, "Body ROWS… are STABLE across frames".
  - ⚠ **The premise looks false across frames.** IM-1 is intra-step. Table removal swap-removes (`crates/boyko_ecs/src/ecs/core/archetype/archetype.rs:1933` test "remove_entity_non_last_returns_swapped_outcome"), and `sync_rows` only resizes (`resources.rs:3187` "self.asleep.resize(n_rows, false);").
  - So after a despawn, or a spawn into an earlier archetype, latches attach to different bodies.
  - I inferred this from the code; **no test or run was made.** It is the strongest argument FOR the entity-keyed `Sleeping` form.
- **R-4: warm-start tables and the box-axis cache stay pair-keyed resources, not pair entities.**
  - Reason: determinism of probe order (`ARCH-AUDIT…:30`, W2). Contacts as entities is an open owner ballot (D3).
  - The same row-shift issue as R-3 applies to dense-row warm keys, but only as a one-frame value artefact. `OPTIMIZATION-PLAN-PHYSICS.md:310` already names "warm-start re-key to a content-defined (Entity) key (OQ-2)".
- **R-5: manifolds, `ContactPairs`, graph, grid and contact columns are per-step scratch under synthetic ids, not components.**
  - Reason: `ARCH-AUDIT…:14` "Pattern (CSR/SoA, cleared-and-refilled, 0-alloc) is already correct; only the backing medium… changes".
- **R-6: soft particles are not entities.** Reason: `ADVANCED…:811` (0.6–2 ms per 20k spawns, and no contiguity across archetypes).
- **R-7: fracture chunks are rows, not bodies.** Reason: same number, `ADVANCED…:949-951`.
- **R-8: the character controller's logic lives outside the solver** (its state is a component). Reason: P-17, `ADVANCED-PHYSICS-RESEARCH.md:690`.
- **R-9: physics uses its own `pool.scope`, not kernel `par_iter`.**
  - Reasons:
    - (a) dense is refused (`par_iter.rs:300-301` "the chunk runner has no world cell");
    - (b) cuts are data-dependent CSR runs (`OPEN-QUESTIONS.md:5183-5186` "The distinguishing quantity between call sites is not a number but a FUNCTION");
    - (c) the O-plan's stage-owns-pool model (H1).
  - (a) is plumbing, not a design limit (`KERNEL-BACKLOG.md:54`). The owner's deferral trigger for (a) has fired (H4).
- **R-10: dense is never GPU-resident.** Reason: `DENSE…:61` (W1). A GPU soft rung must go through `GpuColumn`, not dense (`ADVANCED…:1073-1074`).
- **R-11: `SoftBody` as a `Vec` component in the meantime.** Reason: "variable-length-payload component-model gap" (`ARCH-AUDIT…:16`). The kernel still has no variable-length component primitive.

## 4. What is measured about "ECS-native = cache-optimal" on the solver

**Measured, in favour:**
- Cached-frontier column beats `Vec` by 3.4 % at n=10k (C3).
- The `BroadphaseGrid` migration was faster (`OPEN-QUESTIONS.md:42-43` "18.27 vs 19.20 ms on one worker").
- Stage 4 shows no end-to-end cost within ±2–4 % (C4).

**Measured, against, then fixed:**
- A ~40 % rigid-solver regression from cache-set aliasing (A10).
- A 7–17 % push regression on the all-pairs loop (C4).
- Both fixes were kernel work, not "free" properties of `ComponentPool`.

**Measured cost, not yet fixed:** 33 solver columns take 4.13 MiB resident (K). The packing plan is DESIGN only.

**Never measured:**
- The in-place (no gather) dense solve against the gather.
- Scattered `DenseSolveView::row_ptr` against a contiguous re-gather.
- ARCH-AUDIT itself concedes the win is "ADDRESS-STABILITY-on-grow… not "ComponentPool is faster on the hot loop"" (`:38`).

## Open list (lens plans)

1. OWNER (step 6): IslandSleep's target is written three ways - Sleeping component + EnableColumn (ARCH-AUDIT-ECS-DATA-REMEDIATION.md:30, commit 57e0a512), 'kernel columns' (census rung, ad0ebea4 tests/physics_vec_side_store_census.rs:253-256), 'ScratchColumn / dense components' (ALLOCATOR-DESIGN-SPACE.md:321). Pick one; ADVANCED O-5 (per-body sleep settings) also needs a column.
2. OWNER (step 9 / S0): SoftBody's particle columns go to dense components (census rung text) or to a resource ScratchColumn row bank plus a dense handle (ADVANCED D-8, :807/:838)? The owner's 2026-09-10 ladder ranks dense above resource column; D-8 does not argue against the higher form.
3. VERIFY BEFORE DESIGN: the IslandSleep per-row latch rests on 'rows never shift' (resources.rs:3013-3014), but table removal swap-removes (archetype.rs:1933) and sync_rows only resizes (resources.rs:3187). Needs a red-first test: a despawn mid-archetype while the islands are asleep. Not run.
4. KERNEL: KE15 (par_iter over dense, par_iter.rs:307) is UNOWNED, and the owner's deferral trigger ('waits for Stage 4 to finish', OPEN-QUESTIONS.md:5223) has fired. Without it, dense bodies stop compiling at physics_integrate's par_iter_mut (OPEN-QUESTIONS.md:5198-5201).
5. KERNEL: dense x enable-bit composition - dense_iter compile-rejects enable-bearing filters (DENSE-ENABLE-QUERY-PLAN.md:85-86) and dense is archetype-agnostic while enable bits are (archetype,row)-keyed (:81-82). Physics' prescribed kinds (dense bodies + EnableColumn sleep / Simulated bitset) do not compose on the fast path. Unverified whether the IsEnabled<T> data term is admitted by dense_iter.
6. KERNEL: scratch ComponentId allocation is physics-owned (scratch_ids.rs:541, band at the top of MAX_COMPONENTS=512); no kernel API reserves a band for another subsystem. The dense plan said 'no synthetic-id' (DENSE-COMPONENTS-PLAN.md:87), ARCH-AUDIT said 'new_scratch registry-free' (:27); neither shipped.
7. MEASUREMENT OWED: gather vs in-place dense solve (scattered DenseSolveView::row_ptr vs contiguous re-gather). This is the load-bearing reason for R-1 (ARCH-AUDIT :22-24) and was never measured; no bench exists in crates/boyko_physics/benches.
8. MEASUREMENT OWED: per-stage wall time at W=1 vs W=8 (OPEN-QUESTIONS.md:5524) before choosing barrier vs parallel narrowphase/broadphase.
9. STALE PREMISES: ADVANCED-PHYSICS-DESIGN-SPACE fact 2 / :37-39 and RESEARCH section 9 assume the pre-KE16 serial floor and 72 waves; joltab KE16-RESULTS.md:1726-1731 (par_in_system 14.6x over seq) and OPEN-QUESTIONS.md:5505-5509 (wave count W-independent, 72 an upper bound) supersede them. The brief's fact (d) is likewise stale.
10. OWNER: contacts as gameplay-visible ECS data (ARCH-AUDIT :42) is still an open ballot; the Contact component has no producer (ADVANCED :1282); KR-1 (Entity as QueryData) is not found in iters/query.
11. DOC CONFLICT: RENDER-PHYSICS-GPU-PLAN.md:218-220 and PERF-DIRECTIONS.md:254 (CPU-D7) still permit Rapier/Jolt-FFI behind the seam, against the owner's fully-in-house decision (RESEARCH-SOFT-BODY.md:6) and order (1); never marked superseded.
12. DOC CONFLICT: SYSTEMS.md:2011-2013 claims 'no parallel data system' while the census pins 34 Vec fields.
13. ALLOCATOR: rev 2 is blocked (C1 Heap back-pointer TB UB, C2 ScopeArena wid collision - ALLOCATOR-DESIGN-SPACE.md:439-440); the physics per-step home (ScratchColumn vs FrameVec/HeapVec) is not decided anywhere; DenseStore.free is itself a Vec (dense_store.rs:126).
14. OWNER (H8 vs H1): the 2026-09-10 instruction 'parallelism inside a system is a kernel feature, not pool.scope in physics' overturns OPTIMIZATION-PLAN-PHYSICS D7 (:109, :265) with no replacement plan; the Cuts policy object (CSR-shaped cuts) was deferred until after the barrier (OPEN-QUESTIONS.md:5219).

---

# Lens 4 of 4: reference

# Research: ECS-native physics elsewhere, and what it costs (lens for boyko_physics unification)

Tree read: `D:/wt/joltab` (merge/ke16-into-ecsnative @ ca582e72). Nothing in it was touched. External code was read on each project's default branch on 2026-09-10, and none of those versions is pinned: avian `main`, Jolt `master`, bevy_rapier `master`, Box2D `main`, Unity Physics through the unofficial `needle-mirror` of the UPM package (`master`) plus the 1.3 manual. Tags: [S] source read, [D] official doc, [B] blog or talk (recorded, not relied on). Source files were read through a summarising fetch tool. Constants and identifiers quoted from them are as returned, not re-checked against raw bytes.

## Brief summary (TL;DR)
- **No engine with a real solver runs its hot loop on the ECS component columns.** Unity Physics, avian and boyko all do the same three things each step: copy ECS components into a solver-owned dense layout, solve on that layout, write back. The ECS is the source of truth, not the solver's storage. Unity builds `PhysicsWorld` NativeArrays with `IJobChunk` [S]. Avian keeps a `SolverBodies` resource (`Vec`s) plus a `SolverBodyIndex` component [S]. boyko gathers into a `ScratchColumn<BodyState>` (`crates/boyko_physics/src/systems.rs:244` `for (body, mass, collider, sensor, simulated, kinematic) in query.iter() {`).
  - Avian measured what happens when the solver fetched through ECS queries instead. Its 0.3 post says "the ECS appears to be performing rather poorly for random access" (`Query::get` per constraint), and the dense solver-body rewrite gave about 3x [B].
- **So "ECS-native = cache-optimal" holds only for the gathered form, and only after stagger work.** In-tree, the first move of the contact columns onto `ComponentPool` cost about 40 %, from cache-set aliasing (`scratch_ids.rs:130` `// conflict-miss storm that cost the measured ~40 % rigid-solver regression when`). It was recovered by a per-id base stagger. Keeping it recovered now depends on physics assigning synthetic ids by hand, in cohorts (`scratch_ids.rs:9` `//! each scratch column needs a registered id even though its element type`).
- **Nobody makes a contact an entity** (Unity, avian, Jolt, rapier). Joints are entities in Unity and avian. Islands are not entities anywhere: avian keeps them in a resource plus a per-body `BodyIslandNode` component. The only "collision = entity" design found is an old flecs toy example with no solver.
- **The parallel structure differs most from boyko at the dispatch level, not at the layout level.**
  - Box2D v3 enqueues one task per worker for the whole solve and advances stages (including per-colour stages) through an atomic sync word with spin-wait [S].
  - Jolt creates `max_concurrency` solver jobs that pull 16-item batches from an atomic status word [S].
  - Jolt's narrow phase is parallel and fused with the broadphase query, with per-job pair queues and stealing [S].
  - boyko opens a `pool.scope` per wide colour per pass (`solver/colored.rs:2738`). Its narrow phase is a serial loop (`systems.rs:378` `for &(a, b) in pairs.pairs() {`).
  - Unity chains one scheduled job per phase per iteration, so it is closer to boyko here [S].
- **Persistent vs rebuilt state is where published numbers exist.** boyko rebuilds islands and colouring from scratch every step (`resources.rs:2620`–`:2622`).
  - Box2D measured persistent islands against per-step DFS: 0.01 ms vs 0.69 ms average on 10,010 resting bodies [B].
  - Havok's docs attribute ">2x" over stateless Unity Physics to sleeping and caching [D]. The rig is not named.

## Approaches in state-of-the-art engines

### Unity Physics (DOTS) and Havok for DOTS
- **Entity model:**
  - Bodies are entities with `LocalTransform`, `PhysicsVelocity`, mass and collider components.
  - Joints are entities: the `CreateJoints` job builds the `Joints` array "from constrained body pair and joint components" [S: PhysicsWorldBuilder.cs].
  - Contacts are not entities. They live in `NativeStream`s inside `Simulation`, valid only between the narrowphase and Jacobian-building groups [S: Simulation.cs].
  - No island structure appeared in the files read.
- **Where solver state lives:** in a `PhysicsWorld` (CollisionWorld + DynamicsWorld with `NativeArray<MotionData>`, `NativeArray<MotionVelocity>`, `NativeArray<Joint>`) [S: DynamicsWorld.cs].
  - It is rebuilt every step. The docs say it "does not cache anything frame-to-frame" [D], and that the engine "forgoes this caching in favor of simplicity and control" [D].
  - The build uses three `IJobChunk`s (`CreateRigidBodies`, `CreateMotions`, `CreateJoints`) plus a `NativeParallelHashMap<Entity,int>` from entity to body index [S].
  - The static BVH is rebuilt only when `chunk.DidChange(...)` or `chunk.DidOrderChange()` fires [S].
- **Addressing invariant:** export writes back with `motionIndex = entityStartIndex + i`, where `entityStartIndex = ChunkBaseEntityIndices[...]` [S: PhysicsWorldExporter.cs]. The changelog states that build and export "expect the chunk layouts for rigid bodies to be the same at both ends" [D]. This is the same invariant as boyko's IM-1 (`systems.rs:1155`).
- **Parallelism:** the pipeline is a chain of system groups (build, body pairs, contacts, Jacobians, solve and integrate, export) that schedule jobs with dependencies [D].
  - `DispatchPairSequencer` packs body pairs into up to `kMaxNumPhases = 64` phases, using a 64-bit phase mask per body. Phase 63 is the serial overflow [S: Scheduler.cs].
  - The solver schedules a separate `ParallelSolverJob` per phase per iteration, chained on the previous handle. Its batch size is `info.ContainsDuplicateIndices ? info.NumWorkItems : 1` [S: Solver.cs].
  - `StepImmediate` runs the whole step on one thread without jobs [S].
- **Events:** collision, trigger and impulse streams live in `Simulation`. They are valid "after the PhysicsSimulationGroup has finished, and up until it starts in the next frame" [D].
  - Stateful Enter/Stay/Exit events are, per search results, `DynamicBuffer`s in the official samples. Not verified: the sample source was not read.
- **Havok for DOTS:** closed-source binary, "stateful". It "shares the same input and output data formats as Unity Physics", and simulation is "over two times faster" in scenes with many rigid bodies, credited to "automatic sleeping of inactive rigid bodies and other advanced caching techniques" [D]. No rig or scene is named.
- **Trade-offs:** statelessness buys determinism, netcode-friendliness and simple rollback. It costs warm starting and caching, and by Havok's claim more than 2x. (Whether Unity Physics warm-starts at all: not verified.)

### Avian (Bevy, ECS-driven, avian `main`)
- **Entity model:**
  - Bodies and colliders are entities with `Position`, `Rotation`, `LinearVelocity`, `ComputedMass` and related components.
  - Joints are entities with joint components [B: 0.4 post].
  - Contacts are not entities: `ContactGraph` is a resource holding `ContactEdge` connectivity and `ContactPair` data, with active and sleeping pairs in separate lists [B: 0.4 post; S: the narrow phase iterates `contact_graph.active_pairs_mut()`].
  - Islands live in the `PhysicsIslands` resource (`islands: StableVec<PhysicsIsland>`, per-contact and per-joint `IslandNode` vectors). Each body carries a `BodyIslandNode` component whose hooks maintain linked lists [S: islands/mod.rs].
- **Where solver state lives:** `#[derive(Resource)] pub struct SolverBodies { entities: Vec<Entity>, bodies: Vec<SolverBody>, inertias: Vec<SolverBodyInertia> }`. Each awake dynamic or kinematic body holds `SolverBodyIndex(u32)`, a component "added, removed, and updated automatically by the solver plugin" [S: solver_body/mod.rs].
  - Static and sleeping bodies get no solver body. They use a "dummy state" with `SolverBody::default()` [S].
  - Sizes: SolverBody 32 B in 2D and 56 B in 3D; SolverBodyInertia 16 B and 32 B. It stores delta position and rotation rather than world pose [B: 0.4 post].
  - Indices are assigned by observers on `RigidBody` add, on `Remove<Sleeping>`, and on `Remove<Disabled>`/`Remove<RigidBodyDisabled>` [S: solver_body/plugin.rs].
  - `prepare_solver_bodies` and `writeback_solver_bodies` use `par_for_each` with `MIN_PAR_ITER_ENTITIES` [S].
- **Race-class handling:** `SolverBodiesAccess` is a struct of `*mut SolverBody`, `*mut SolverBodyInertia` and `len`, with `PhantomData<&'a mut SolverBodies>`. It carries `unsafe impl Send/Sync` with "SAFETY: The caller is responsible for only accessing disjoint indices concurrently", plus `body_unchecked_mut` and `get_pair_unchecked_mut` [S].
  - This is the same per-element pointer discipline as boyko's SolveView. The backing is a `std::Vec`, kept stable by the borrow rather than by address stability.
- **Parallelism:**
  - Constraint graph: `GRAPH_COLOR_COUNT = 24`, `COLOR_OVERFLOW_INDEX = 23`, `DYNAMIC_COLOR_COUNT = 20` (static-touching constraints get the tail colours). The colouring is persistent and incremental [S: constraint_graph.rs; B: "persisted across time steps, and updated incrementally"].
  - Per colour, `crate::utils::par_for_each(&mut color.contact_constraints, 64, ...)`. The overflow colour is solved serially first [S: solver/plugin.rs].
  - `par_for_each` goes serial if `thread_num() == 1 || slice.len() < min_len`. Otherwise it uses `chunk_size = len / thread_num`, i.e. one chunk per thread, via `par_chunk_map_mut` on `ComputeTaskPool` [S: utils.rs].
  - The narrow phase is parallel over active pairs (`par_for_each(..., 64, ...)`), fetching colliders with `collider_query.get_many([...])`. Thread-local `BitVec`s record status changes and are "combined ... serially using bit-wise OR" for determinism [S: narrow_phase/system_param.rs].
  - `PhysicsSchedule` uses `SingleThreadedExecutor` [S: schedule/mod.rs], so all physics parallelism is intra-system. Which thread `run_physics_schedule` runs on: not verified.
- **Sleeping:** `Sleeping` is a structural marker. It is inserted with `world.insert_batch(bodies_to_sleep)` and removed per body with `entity_mut(entity).remove::<Sleeping>()`; hooks fire `on_add` and `on_remove` [S: islands/sleeping.rs]. Persistent islands use union-find merges and deferred DFS splitting of "the sleepiest island" [S].
- **Events:** `OnCollisionStart` and `OnCollisionEnd` observers, plus message/event readers [B: 0.3 and 0.4 posts].
- **Measured costs [B]:**
  - 0.4 vs 0.3: about 3x, on an i7-13700F, `pyramid_2d` with a base of 75 boxes, 4 substeps, `parallel` feature. Graph colouring alone gave "over 3x" on the solver and "nearly 2x" total.
  - 0.3: narrow phase 4.5x; contact-constraint generation 1.26 → 0.55 ms. The rig for the 0.3 numbers is not verified.
  - 0.6: BVH broadphase 10x on 40,000 static colliders; "remaining overhead is primarily due to inefficient change detection for AABB updates".

### bevy_rapier (counter-example: a non-ECS engine glued onto an ECS)
- **Storage:** double. Rapier's own arenas live in ECS components on a context entity: `RapierRigidBodySet { bodies: RigidBodySet, entity2body: HashMap<Entity, RigidBodyHandle> }`, `RapierContextColliders { colliders: ColliderSet, entity2collider: HashMap<...> }`, and `RapierContextSimulation { islands: IslandManager, broad_phase, narrow_phase, ... }` [S: src/plugin/context/mod.rs]. The same bodies' `Transform`/`Velocity` are ECS components as well.
- **Schedule:** three sets.
  - `SyncBackend`, about 17 systems including `init_rigid_bodies`, `apply_rigid_body_user_changes` and `sync_removals`.
  - `StepSimulation`, which is one `step_simulation` system.
  - `Writeback`: `writeback_rigid_bodies`, `update_colliding_entities`, `writeback_mass_properties` [S: src/plugin/plugin.rs].
- **Published sync cost:** no numbers were found or verified. The ballpit devlog puts its numbers only in a video.

### Jolt (performance reference, not an ECS)
- **Body storage:** AoS behind pointers. `mBodies` is `Array<Body*>`, "reserved to the max bodies ... so that adding bodies will not reallocate". Bodies are individually allocated (`AllocateBody`). Active bodies are an array of `BodyID` per type with `atomic<uint32>` counts. Access is guarded by `MutexArray<SharedMutex> mBodyMutexes` [S: BodyManager.h].
- **Contacts:**
  - `ContactConstraintManager` holds `uint8* mConstraints` with an `atomic<uint64> mNumConstraintsAndNextConstraintOffset` for lock-free append, and a `ContactConstraint` holds `Body* mBody1/mBody2`.
  - Warm-start state lives in a double-buffered `ManifoldCache mCache[2]` built on lock-free hash maps [S: ContactConstraintManager.h].
- **Islands:** rebuilt every step. `LinkBodies` works on an `atomic<uint32> mLinkedTo` per body, so islands are built during collision detection, and `SortIslands` puts large islands first [S: IslandBuilder.h].
  - `LargeIslandSplitter` handles islands above `cLargeIslandTreshold = 128`. `SplitMask = uint32` gives 32 splits, with split 31 non-parallel. Splits under `cSplitCombineTreshold = 32` are folded into the non-parallel split. Batches are `cBatchSize = 16`. Status is an `atomic<uint64> mStatus` packing iteration, split and item [S: LargeIslandSplitter.h/.cpp].
- **Parallelism:** a job graph with dependency counters and barriers [S: JobSystem.h], `cMaxConcurrency = 32` [S: PhysicsUpdateContext.h].
  - `JobFindCollisions`: a job claims active-body batches through a compare-exchange on `mActiveBodyReadIdx`, queries the broadphase, and pushes pairs into its own padded `BodyPairQueue`. It steals from sibling queues round-robin, so broadphase query and narrow phase interleave in one job. There are `num_find_collisions_jobs = max(2, min(ceil(active/cActiveBodiesBatchSize), max_concurrency))` of them [S: PhysicsSystem.cpp]. The value of `cActiveBodiesBatchSize` is not verified.
  - `JobSolveVelocityConstraints` is a `for (;;)` loop over `FetchNextBatch` (`BatchRetrieved` / `WaitingForBatch` / `AllBatchesDone`) plus an atomic next-small-island counter. It calls `std::this_thread::yield()` while waiting. There are `step.mSolveVelocityConstraints.resize(max_concurrency)` of them [S].
  - The effect is that colour transitions inside a large island are an atomic status change, not a new job or scope.
- **Memory:**
  - `TempAllocator` "works as a stack"; `TempAllocatorImpl` "allocates a large block through malloc upfront" [S: TempAllocator.h].
  - Jobs come from a preallocated `FixedSizeFreeList<Job> mJobs` sized by `inMaxJobs`, with a lock-free `atomic<Job*> mQueue[1024]` and a semaphore. The header calls the thread pool "an example implementation" [S: JobSystemThreadPool.h].

### Box2D v3 (added because avian cites it as the model)
- **Colouring:** greedy per-colour bitsets, persistent. On the large pyramid (5,050 bodies, 14,950 pairs) the colours were 2524 / 2508 / 2107–2465 ×4 / 652 / 32 [B: "SIMD Matters"].
  - AMD 7950X, 4 workers: AVX2 0.90 ms, SSE2 1.02 ms, scalar 1.91 ms [B].
- **Islands:** persistent, with serial union-find on add and deferred splitting of one island per step [B].
  - 182 pyramids (10,010 bodies, resting): DFS 0.69 ms average / 0.98 ms max vs persistent 0.01 ms / 0.45 ms.
  - Tumbler with 2,000 boxes: 0.43 ms vs 0.08 ms. Rig not verified.
- **Dispatch:** `for (int i = 0; i < workerCount; ++i)` enqueues one solver task per worker. The caller races to claim the main role. The main thread "synchronizes the workers and does work itself" by storing `atomicSyncBits`; workers spin with `b2Pause()` until it changes.
  - Per-colour stages come from `b2InitColorStages`, with `maxBlockCount = 4 * workerCount` [S: src/solver.c].

### flecs
- The flecs core has no solver. `flecs-systems-physics` provides move and rotate systems plus an octree spatial query (`ecs_squery_new`), with no `multi_threaded` flag and no constraint solving [S: src/main.c, include header].
- `ecs_collisions` (an old example) creates "a new entity with the `EcsCollision2D` component" per collision, "automatically cleaned up" after the frame [S: README]. Its version is not verified.
- The ECS FAQ has no physics section [S].

### EnTT
- Not studied. No physics integration by the EnTT author was found.

## Comparative table

| Aspect | Unity Physics | Havok for DOTS | avian | bevy_rapier | Jolt | boyko (joltab) |
|---|---|---|---|---|---|---|
| Contact = entity? | no (NativeStream) | no | no (ContactGraph resource) | no (rapier NarrowPhase) | no (manager buffer) | no (`Manifolds` resource; `Contact` component declared, `components.rs:202` `pub struct Contact {`, no producer found by grep) |
| Joint = entity? | yes [S] | same data as Unity | yes [B] | yes (mapped by HashMap) | no (Constraint objects) | n/a |
| Island | none found | not verified | persistent, resource + `BodyIslandNode` component | rapier `IslandManager` | per-step, lock-free links | per-step union-find (`resources.rs:2620` `        self.build_islands(manifolds, &is_dynamic);`) |
| Colouring | 64 phases, rebuilt per step | not verified | 24 colours, persistent incremental | not verified | 32 splits for islands >128 | first-fit, rebuilt per step (`resources.rs:2622`) |
| Solver body store | NativeArrays rebuilt per step | same input data | `SolverBodies` Vec resource + index component | rapier arena | `Body*` AoS | `ScratchColumn<BodyState>` refilled per step |
| Body ↔ row mapping | chunk order (build = export layout) | same | persistent `SolverBodyIndex` via observers | `HashMap<Entity, handle>` | `BodyID` + sequence | archetype-row order, "no structural change" (`systems.rs:1155`) |
| Sleeping | not verified | yes [D] | structural `Sleeping` component, batch insert | rapier | per-island | `IslandSleep` Vec resource; `Simulated` is a bitset EnableTag |
| Narrow phase | parallel jobs | not verified | parallel, thread-local bitsets | rapier-internal | parallel, fused with broadphase, stealing | serial (`systems.rs:378`) |
| Solve dispatch | job per phase per iteration, chained | not verified | `par_for_each` per colour (min 64, len/threads) | rapier-internal | `max_concurrency` persistent jobs, atomic batches | `pool.scope` per wide colour per pass |
| Warm start | not verified | "caching" [D] | yes [B] | rapier | ManifoldCache ×2 | yes (`scratch_ids.rs:228` `pub(crate) const WARM_TABLE_COLUMN_COUNT: usize = 4;`) |
| Published perf | – | ">2x" vs Unity (no rig) | ~3x 0.3→0.4, i7-13700F | none verified | – (in-house (c) only) | brief (c) |

## Key algorithms and techniques
- **Gather → solve → writeback over a dense per-step layout.**
  - Unity keeps the invariant by chunk order [S]. Avian keeps it with a persistent index component [S]. boyko keeps it by row order: `systems.rs:1127` `pub fn physics_apply(mut query: Query<Mut<RigidBody>>, scratch: Res<SolverScratch>) {`.
  - Avian allocates solver bodies only for awake dynamic and kinematic bodies [S]. boyko's gather visits every row: `systems.rs:1053` `/// INTEGRATE — `physics_gather` still walks every row (IM-1 intact). When off, it`. Both gather and writeback are serial loops in boyko (`systems.rs:244`, `:1131`). Avian runs both with `par_for_each` [S].
- **Greedy bitset colouring with a serial overflow bucket.** Every engine has one: Unity phase 63, avian colour 23, Jolt split 31. Avian and Box2D make the colouring persistent and incremental. Unity and boyko rebuild it every step.
- **Persistent islands.** Union-find on edge add plus deferred split (Box2D, avian) vs per-step rebuild (Jolt, boyko). Box2D measured about 69x on a resting pyramid field [B].
- **Stage advance without re-dispatch.**
  - Box2D: persistent per-worker tasks and an atomic sync word with spin-wait [S].
  - Jolt: persistent jobs pulling atomic batches, with a `yield` while waiting [S].
  - Contrast, Unity: a scheduled job per phase per iteration [S].
  - Contrast, boyko: a scope per colour ≥ 256 slots per pass, which allocates. `solver/colored.rs:2730` `        // W1: a `pool.scope` allocates (a boxed shared frame + a boxed closure per`.
- **Deterministic parallel event and state collection:** per-thread bitsets OR'd serially after the parallel narrow phase (Box2D [B], avian [S]).
- **Preallocated dispatch memory:** Jolt's job free list and upfront temp block [S]. Relevant to brief fact (b), about 330 chunk allocations per step at W8.

## Pitfalls and mistakes
- **ECS random access in the constraint loop.** Avian 0.3 fetched 21 components per body through `Query::get` for each constraint and named it the bottleneck [B]; the fix gave about 3x [B].
- **Dispatch granularity against fixed work.** boyko measured negative scaling before the chunk floor: `solver/colored.rs:268` `/// pyramid measured 0.82x at 8 workers and 0.56x at 16 — NEGATIVE scaling.`
  - The floor itself cannot serve both scene shapes: `:300` `/// NARROW colours, so its cost is per-wave dispatch and it wants coarse chunks; a`, and `:303` `/// making the cut policy a per-call-site OBJECT rather than a global constant —`.
- **Kernel storage aliasing.** Columns whose bases are 64 KiB-aligned alias cache sets (`scratch_ids.rs:130`). The per-cohort stagger discipline now lives in physics, in synthetic ids at the top of the id space (`scratch_ids.rs:541` `const SCRATCH_REGION_MIN_ID: usize = MAX_COMPONENTS - 128;`). The count is heading to about 94 (`:169` `// brings the live column count to ~94, and 94 consecutive ids CANNOT have`).
- **Structural sleep combined with order-based addressing.** Avian's `Sleeping` insert and remove are archetype moves [S]; avian can afford them because it addresses through `SolverBodyIndex`. Unity and boyko address by chunk or row order, so a structural change mid-pipeline breaks export (`systems.rs:1155`). boyko already avoids moves for `Simulated`: `components.rs:44` `/// kernel `enable::<Simulated>` / `disable::<Simulated>` API, with NO archetype`.
- **Change-detection overhead on bulk writeback.** Avian 0.6 names "inefficient change detection for AABB updates" as its residual on a fully static scene [B]. boyko's writeback deref-writes the whole `RigidBody` through `Mut` for touched rows (`systems.rs:1141`).
- **Double storage and handle maps** (bevy_rapier: `HashMap<Entity, RigidBodyHandle>` [S]). This is the pattern principle 0 and the HashMap ban exclude.
- **Statelessness.** Havok's documented ">2x" over Unity Physics is attributed to sleeping and caching [D].

## Relevant academic works
- Erin Catto, "Modeling and Solving Constraints", GDC 2009. Jolt's Architecture.md cites it as the basis of its sequential-impulse solver with warm starting [D]. https://box2d.org/files/ErinCatto_ModelingAndSolvingConstraints_GDC2009.pdf
- Avian's `constraint_graph.rs` cites "Intel's paper on many-core physical simulations" and Catto's "SIMD Matters" [S]. The Intel paper's title and venue are not verified.

## Applicability to boyko-engine
These are patterns observed, with how each fits the stated constraints. Choosing among them is the architect's call.

**Already equivalent**
- Gather, solve on a dense layout, write back. This is the shared form of Unity, avian and boyko. The kernel form is `ScratchColumn`, backed by `ComponentPool::new_untracked` and address-stable (`scratch_column.rs:9` `//! `ComponentPool`'s base is ADDRESS-STABLE across growth (in-place commit) —`).
- Per-element pointer solve views, the SP4 class fix. Avian's `SolverBodiesAccess` is the same discipline on a `Vec` [S].

**Direct candidates (evidence exists)**
1. Persistent island and colour state (Box2D, avian) in place of per-step rebuild (`resources.rs:2619`–`:2622`). This work is serial and sits in the chain `gather → select → broadphase → narrowphase → build_graph → solve → apply` (`plugin.rs:561` `    let gather = builder.add_system(physics_gather).after(integrate).key();` … `:638` `    let apply = builder.add_system(physics_apply).after(solve).key();`). Its share of the brief's 0.43–0.53 serial fraction is **not measured**.
2. A parallel, deterministic narrow phase (Jolt's fused query with per-job queues and stealing; avian's `par_for_each` with thread-local bitsets). Brief fact (d) says `par_iter` inside a system measured 1.01x, so a working in-system parallel primitive is a precondition.
3. A persistent-gang stage protocol for multi-pass solves (Box2D atomic sync, Jolt `FetchNextBatch`). It answers the per-wave dispatch cost the in-tree sweep names (`colored.rs:299`–`:303`). Principle 0 would place such a primitive in the kernel scheduler or threadpool, not in physics.
4. Awake-only solver bodies with dummy static/sleeping state (avian). boyko already mirrors the dummy semantics (`components.rs:54` `/// "dummy SolverBody for a disabled dynamic" freeze semantics — Decision 4).`) but gathers every row.

**Needs adaptation**
- Avian's `SolverBodyIndex` component, maintained by observers, would decouple solver rows from archetype order. boyko's kernel has hooks/observers (CLAUDE.md layout), but no index-component pattern exists in physics today.
- Unity's `DynamicBuffer` (in-chunk up to `InternalBufferCapacity`, then heap [D]) is the closest analogue for `SoftBody`'s 30 per-entity `Vec`s (`soft/component.rs:71` `    pub pos_x: Vec<f32>,`). No per-entity buffer component type was found in `boyko_ecs` (grep `dynamic ?buffer|per-entity (array|buffer)`: no match). Physics does not use `DenseStore` (grep: no match), although the kernel ships it (`dense/mod.rs:3` `//! One [`DenseStore`] per dense `ComponentId` holds every instance of that type`).
- `IslandSleep`'s 4 `Vec`s (`resources.rs:3057` `    asleep: Vec<bool>,`) correspond to per-body sleep state, which avian stores as components (`TimeSleeping`, `Sleeping`) plus resource islands [B/S].

**Does not fit**
- bevy_rapier's double storage and handle maps (principle 0, HashMap ban).
- Unity's full statelessness: boyko already warm-starts, and Havok's claim is >2x for caching.
- Jolt's `Body*` AoS with per-body mutexes as a layout model. Jolt's lead is in dispatch structure; its layout is not SoA. This is an inference from the headers read.
- Avian's `Vec`-backed `SolverBodies` as a storage choice (principle 0). Its access discipline is compatible.

## Open questions for the architect
See the `open` field.

## Sources
[1] https://docs.unity3d.com/Packages/com.unity.physics@1.3/manual/concepts-simulation.html [D] stateless world, pipeline
[2] https://docs.unity3d.com/Packages/com.unity.physics@1.3/manual/design.html [D] caching trade-off
[3] https://docs.unity3d.com/Packages/com.unity.physics@1.3/manual/physics-pipeline.html [D] system groups
[4] https://docs.unity3d.com/Packages/com.unity.physics@1.3/manual/simulation-results.html [D] event validity window
[5] https://github.com/needle-mirror/com.unity.physics (Unity.Physics/Dynamics/Simulation/Scheduler.cs, Simulation.cs, Dynamics/Solver/Solver.cs, Dynamics/World/DynamicsWorld.cs, ECS/Base/Systems/PhysicsWorldBuilder.cs, PhysicsWorldExporter.cs) [S] unofficial mirror
[6] https://docs.unity3d.com/Packages/com.unity.physics@0.50/api/Unity.Physics.Systems.PhysicsWorldBuilder.html and changelogs [D] same-chunk-layout requirement
[7] https://docs.unity3d.com/Packages/com.havok.physics@1.3/manual/index.html [D] Havok ">2x", stateful, same data
[8] https://docs.unity3d.com/Packages/com.unity.entities@1.3/manual/components-buffer-introducing.html [D] DynamicBuffer
[9] https://raw.githubusercontent.com/avianphysics/avian/main/src/dynamics/solver/solver_body/mod.rs and solver_body/plugin.rs [S]
[10] https://raw.githubusercontent.com/avianphysics/avian/main/src/dynamics/solver/constraint_graph.rs, solver/plugin.rs, src/utils.rs [S]
[11] https://raw.githubusercontent.com/avianphysics/avian/main/src/dynamics/solver/islands/mod.rs and islands/sleeping.rs [S]
[12] https://raw.githubusercontent.com/avianphysics/avian/main/src/collision/narrow_phase/system_param.rs and narrow_phase/mod.rs [S]
[13] https://raw.githubusercontent.com/avianphysics/avian/main/src/schedule/mod.rs [S] SingleThreadedExecutor
[14] https://joonaa.dev/blog/09/avian-0-4 [B] 3x, i7-13700F, SolverBody sizes
[15] https://joonaa.dev/blog/08/avian-0-3 [B] ECS random-access cost
[16] https://joonaa.dev/blog/12/avian-0-6 [B] BVH 10x, change-detection residual
[17] https://github.com/Jondolf/avian/pull/771 [B] graph-colouring PR
[18] https://github.com/dimforge/bevy_rapier/blob/master/src/plugin/plugin.rs and src/plugin/context/mod.rs [S]
[19] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsUpdateContext.h, PhysicsSystem.cpp, LargeIslandSplitter.h/.cpp, IslandBuilder.h [S]
[20] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Body/BodyManager.h, Constraints/ContactConstraintManager.h [S]
[21] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Core/TempAllocator.h, JobSystem.h, JobSystemThreadPool.h [S]
[22] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Docs/Architecture.md [D] SI solver with warm starting (the detailed step section was truncated by the fetch and not read)
[23] https://box2d.org/posts/2024/08/simd-matters/ [B] colouring and SIMD numbers, AMD 7950X
[24] https://box2d.org/posts/2023/10/simulation-islands/ [B] persistent islands vs DFS
[25] https://raw.githubusercontent.com/erincatto/box2d/main/src/solver.c [S] stage sync protocol
[26] https://github.com/flecs-hub/flecs-systems-physics (src/main.c, include/flecs_systems_physics.h) [S]; https://github.com/SanderMertens/ecs_collisions [S README]; https://github.com/SanderMertens/ecs-faq [S]
[27] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_tasks/src/task_pool.rs [S] a scope thread ticks the executor (nested-scope behaviour not documented there)
[28] https://github.com/MolecularSadism/avian_vs_rapier and https://molecularsadism.itch.io/bevy-ballpit [B] numbers only in video, not used

## Open list (lens reference)

1. Body-to-solver-row mapping: keep archetype-row order (IM-1, the same invariant as Unity's chunk-order export, systems.rs:1155) or add a persistent index component kept by observers (avian SolverBodyIndex)? This decides whether sleep can be structural and whether the gather can skip sleeping and static rows.
2. What share of the brief's 0.43-0.53 serial fraction (W=8) comes from each serial stage: physics_gather (serial iter, systems.rs:244), physics_narrowphase (serial loop, systems.rs:378), physics_build_graph (per-step union-find plus colouring, resources.rs:2619-2622), and physics_apply? No per-stage timing exists in this report. Box2D's 0.69 ms vs 0.01 ms island numbers are [B], on another engine and rig.
3. Persistent incremental islands and colouring (Box2D, avian) vs the per-step rebuild (Jolt, Unity, boyko). Determinism has to survive either way. Avian and Box2D OR thread-local bitsets serially to keep it.
4. Dispatch primitive for multi-pass solves: scope per wave (boyko today, Unity's job-per-phase) or a persistent worker gang with an atomic stage/batch word (Box2D solver.c, Jolt FetchNextBatch)? Per principle 0 it would be a kernel/threadpool feature. colored.rs:303 already argues for a per-call-site cut-policy object.
5. Brief fact (d) (nested scope from a worker measured 1.01x) vs the threadpool's documented intent (boyko_threadpool/src/lib.rs:17-18, scope is 'designed to be called from inside a worker'). The mechanism behind the sequential result was not re-derived here. It gates any in-system parallel narrow phase or gather.
6. Should synthetic scratch ComponentId assignment and the per-cohort cache-stagger discipline (scratch_ids.rs:9, :130, :169, :541) move into the kernel? Today physics hand-assigns ids at the top of the id space for a property of kernel storage.
7. SoftBody's 30 per-entity Vec columns (soft/component.rs:69-71): no kernel per-entity buffer component exists (grep found none). Options seen elsewhere: Unity DynamicBuffer (in-chunk up to capacity, then heap), DenseStore rows, or Resource-owned columns. Physics does not use DenseStore today.
8. IslandSleep's 4 Vecs (resources.rs:3057-3073): per-row latch and counter vs per-island scratch. Avian splits these into components (Sleeping, TimeSleeping) plus a resource. The lane's recorded 'step 6' names Sleeping + EnableColumn.
9. Contact events: the Contact component is declared (components.rs:202) but no producer was found by grep. Unity uses per-step streams plus stateful DynamicBuffers in its samples (not verified), avian uses messages plus OnCollisionStart/End observers. Which model the kernel event and observer system should carry is open.
10. Awake-only gather (avian dummy bodies) vs gather-all (IM-1). Affects the solver working-set size on large resting scenes.
11. NOT VERIFIED: whether Unity Physics warm-starts or has sleeping; Havok's colouring and islands; the rig for the avian 0.3 numbers and the Box2D islands post; Jolt's cActiveBodiesBatchSize value; whether Jolt's island build/finalize jobs are single jobs; the title of the Intel many-core paper avian cites; the Unity stateful-events sample source; published bevy_rapier sync costs (none found); the exact sizes of BodyState and BodyEffective (needs an LSP hover).
