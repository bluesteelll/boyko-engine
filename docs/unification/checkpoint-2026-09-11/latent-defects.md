# Two latent defects - row-keyed physics state, and rejected GPU uploads leaking device memory

- **Source:** workflow `runtime-data-ledger-rev2` (run `wf_7c3b8f78-6af`), defects agent (read-only), tree D:/wt/joltab
- **Status:** BOTH CONFIRMED BY READING THE CODE; NOT REPRODUCED AND NOT FIXED. The fix plan is in the file; it waits for the id-recycling change to land on the same tree.
- **Copied verbatim** on 2026-09-11 at the checkpoint the owner asked for; agent outputs are reproduced without edits.

---

## Verdicts

# Two latent defects on joltab: verdicts

The tree is `D:/wt/joltab`, HEAD `d11962a9`. Its `crates/*/src` is identical to `ca582e72`. The working copy has uncommitted edits in 15 `boyko_ecs` files, `entity_api.rs` among them, so I quote that file at HEAD. `archetype.rs`, all of physics src and all of render src are unmodified. Nothing was built or run. Every claim below comes from reading the code.

## A. Row-keyed physics state — CONFIRMED, and worse than F-3 and the ledger say

**The mechanism.**
- A body's row is its position in the gather's walk over matching archetypes: `joltab:crates/boyko_physics/src/systems.rs:244` "`for (body, mass, collider, sensor, simulated, kinematic) in query.iter() {`".
- A despawn swap-removes that row:
  - `joltab:crates/boyko_ecs/src/ecs/core/archetype/archetype.rs:1263` "`if self.component_pools.swap_remove_unit(removed_unit_index.0).is_err() {`"
  - `:1268` "`self.entity_ids.swap_remove(removed_unit_index.0);`"
  - reached from `delete_entity` via `HEAD:…/ecs_master/entity_api.rs:1084` "`let outcome = archetype.remove_entity(removed_unit_index);`"
- A spawn appends at the end: `archetype.rs:1172` "`self.entity_ids.push(entity_id);`".
- The latch is only resized, never re-keyed: `resources.rs:3187` "`self.asleep.resize(n_rows, false);`".
- Every dynamic row gets an island, even one with no contacts: `resources.rs:2725` "`self.island_of.build_view().as_mut_slice()[row as usize] = next_id;`", with the predicate at `systems.rs:1021` "`i < bodies.len() && is_dynamic_row(bodies[i].inv_mass)`".
- The docs at `resources.rs:3013-3014` ("Body ROWS … are STABLE across frames … rows never shift") and `:3056` ("A brand-new row defaults `false` (awake)") are false under any structural change.

**A1 — a new body inherits a latch and hangs mid-air indefinitely.** This is unbounded, and neither F-3 nor the ledger names it. Setup: the `sleeping_pipeline_o8.rs` harness (floor at row 0, pile P1..Pn, `sleeping = true`), with the pile latched.
1. Between two schedule runs, `world.delete_entity(P_j)` with j < n. Pn swap-moves into row j.
2. Spawn E at rest in mid-air, far from everything. It is appended at row n, so the row count is unchanged.
3. Next run, `begin_step` calls `sync_rows(n+1)`, which does nothing. `asleep[n]` is still Pn's `true`.
4. E's one-member island stays frozen: `:3238` "`self.frozen_islands.resize(n_islands, true);`" and `:3244` "`if !self.asleep[row] {`" is never true.
5. E's state is captured (`colored.rs:3197-3198` "`if !sleep.is_row_awake(row) { frozen.push((row as u32, *b));`") and gravity is undone on restore (`:3326` "`snapshot[r] = snap;`"). Write-back skips E (`:3079`).
6. `end_step` sees speed² = 0 < `DEFAULT_SLEEP_THRESHOLD` (1.0e-4, `resources.rs:414`). The latch holds: `:3347` "`self.asleep[row] = *c >= frames;`".
7. This repeats every step. E never moves until an awake body touches it or `wake_all` fires.

The reverse order also triggers it. Spawn E first (row n+1), then delete P_j: E moves into row j with P_j's latch, and `sync_rows` truncates E's own fresh latch.

The same trap catches any body moved into an asleep row while moving slower than 0.01 m/s. A block placed at rest on a sleeping pile merges into an all-asleep island and freezes, penetration included. This is exactly the wake-on-merge promise at `resources.rs:3023-3028`, broken.

**A2 — an unrelated despawn wakes a whole pile.** This costs performance and is bounded.
1. Let D be an awake faller at row j, and let the last row be a member of a sleeping pile.
2. Delete D. The pile member moves into row j and gets `asleep = false`, `below_count = 0`.
3. The whole pile island is solved and integrated for another `sleep_frames` steps (60 by default, `resources.rs:419`).

**A3 — smaller effects.** A moved body that is falling fast freezes for exactly one step; this is F-3's case. When bodies span several archetypes, any spawn, despawn or component insert/remove in an archetype that is not walked last shifts every later row by ±1. Each body then inherits its neighbour's latch.

**Exposure.** Sleeping is off by default: `resources.rs:480` "`sleeping: false,`". Only tests and benches turn it on (`sleeping_pipeline_o8.rs:116`, `benches/sleeping.rs:183,218`). The only pipeline gate already despawns a non-last sleeping body (`sleeping_pipeline_o8.rs:238`). It asserts only the body count and that positions are finite (`:245-247`), never which body holds which latch.

**Warm start — CONFIRMED on the default path.** The failure is a hit on the wrong contact, not the miss that `warm_start.rs:37-39` documents ("…the matched keys differ for one frame: that is a warm-start MISS").
- Keys are built from rows: `colored.rs:1664` "`warm_start::pack(m.body_a, m.body_b, cp.feature_id)`", then `:1667` "`match warm_read.get(warm_key) {`".
- Sphere contacts use one fixed feature id: `systems.rs:484` "`feature_id: 0,`" for sphere–sphere, and `feature_vertex_face(0)` for sphere–box (`sphere_box.rs:28`).
- Warm start is on by default: `colored.rs:1456` "`warm_start_enabled: true,`".

Sequence:
1. Static floor F; body A carrying a 10-sphere tower; lone body B resting on F as the last row.
2. Delete A. B moves into A's row.
3. Next step, B's contact with F computes the same key A's contact had and reads A's stored impulse, about 11× B's own load. That impulse is applied every substep.

The error lasts one step, because that step stores impulses under B's correct row (`colored.rs:3046` swap). It is bounded by the previous body's impulse and re-solved with clamping.

**BoxAxisCache — the key reuse is CONFIRMED, but it is REFUTED as a correctness defect.** The lookup is `systems.rs:415` "`let last_axis = axis_cache.get(a, b);`". An inherited axis is kept only if it is still a valid candidate within 5% of the best depth: `box_box.rs:246` "`Some(last) if last.index != best.index && best.depth >= last.depth / HYSTERESIS_RATIO => last,`". The worst case is one frame with a near-optimal reference face.

### Red-first tests for A
All are device-free and use the real schedule (`sleeping_pipeline_o8.rs` harness, `cfg(not(miri))`):
- **A1**, both orders, `sleep_frames = 8`, settle 120 steps, then delete plus spawn E at (5, 4, 0). Assert E's y < 3.0 after 30 steps. Today it stays exactly 4.0.
- **A2**: spawn an awake faller before a pile of N bodies, let the pile settle, delete the faller. Assert `world.resource::<IslandSleep>().is_row_awake(r)` is false for every pile row after the next step (`is_row_awake` is `pub`, `resources.rs:3145`). Today they are awake.
- **Warm start**: run the scene above twice. In the control run A is the last row, so B keeps its row. Assert B's velocity after the step matches the control within resting noise. Today B takes a kick.

### Does the physics design cover it?
- **Structurally, yes.** D1 (bodies addressed by stable slots, which never move) removes renaming; it lands at U5. D4 moves the latch into `BodyGate`, whose zero DEAD value on reuse means "awake"; that is U6. D15 `fresh_step` covers slot reuse (U7).
- **U6's F-3 red-first** ("despawn mid-archetype while islands sleep", design:797) catches A2 and the one-step freeze. It does not name A1 (despawn plus spawn in one window, with the row count unchanged) or the cross-archetype shift. The research labels F-3 "Minor … frozen for one frame" (RESEARCH.md:213-216, :313), which understates it. Widen U6's gate to A1 (both orders), A2 and the cross-archetype shift.
- **U7's reuse test** (design:798) covers reuse only. That is enough once U5 exists, because moved rows then no longer happen.

### Decision on A (performance first, "bugs before features")
- **Latch: partial fix now.** Clear the latch on any row whose `RigidBody` was added since the solver last ran, before `begin_step`. Read it through `Ref<RigidBody>::is_added` (`ref_.rs:47`) in a pass that runs only when sleeping is on, walking in gather order. Gather and apply already rely on that same order (`systems.rs:223-225`).
  - This removes the only unbounded failure, A1 in both orders, because E is always "added".
  - It costs nothing with sleeping off (early return) and one tick read per row with it on.
  - What would overturn it: `benches/sleeping.rs` step time regressing beyond noise. If so, fold the read into the gather.
  - A2 and the one-step freeze stay until U6.
- **Warm start: defer to U5 + U7.** The wrong hit comes from a swap-moved old body. Nothing the gather can read today identifies it: `systems.rs:189-192` says "`Entity` is not a `QueryData`". Any interim identity would be thrown away at U5.
  - What would overturn it: the warm-start red-first test showing a survivor velocity jump more than 10× resting noise, or visible pops in a destruction scene. Then add an interim 16 B/row last-position column that marks a row "fresh", using the D15 lookup-skip form.
- **Land now:** all three red-first tests, and corrections to the false claims at `resources.rs:3013-3014,3055-3056,3173-3177`, `warm_start.rs:35-41` and `axis_cache.rs:28-34`.

## B. Rejected GPU uploads leak device memory — CONFIRMED in code, latent in practice

**What is not freed, and on which branch.** The leak is on the `Err` branch of `assets.fill` at `joltab:crates/boyko_render/src/gpu_upload.rs:120` "`let _ = assets.fill(staged.handle, gpu);`".
- For meshes, `build_mesh_gpu` has just created a vertex and an index `BoundBuffer`, plus a BLAS under `hwrt`. It also claims a geometry-table slot when that table is armed: `mesh_assets.rs:446` "`Some(table) if ctx.vb_geometry_table_armed() => table.register(`".
- For textures, `TextureGpu` holds a `VulkanTexture` and a bindless slot: `texture.rs:618` "`let bindless_slot = aux.register(ctx, texture.view());`".
- Dropping the value frees nothing:
  - `mesh.rs:131` "`MeshGpu` does NOT implement `Drop`"
  - `mesh.rs:203-204` "(`BoundBuffer`/`BuiltBlas`/`IndexType`/integers all have trivial drop), freeing NO device memory."
  - `memory.rs:39-40`: a buffer is destroyed only by value ("`destroy_bound_buffer` consumes it").
- `let _ =` silences the `#[must_use]` that `assets.rs:451-452` puts there for exactly this reason. The fill doc states the obligation at `assets.rs:449-450`: "…or the device buffers/BLAS it holds leak."

**Sequence.**
1. `AssetServer::load::<MeshGpu>` → `assets.reserve()` and `staging.push(Staged { handle, cpu })` (`server.rs:132-133`). The row is now Loading.
2. Before the boot drain, the row leaves Loading/Failed in one of three ways:
   - (a) `assets.remove(handle)` makes it Vacant and bumps the generation (`assets.rs:571`);
   - (b) a MeshRef that pointed at it is dropped, and `dec_ref` reaches zero on the Loading row, moving it to Retiring (`assets.rs:936-960`; `asset_refcount.rs:184`);
   - (c) the same handle is staged twice (`AssetStaging::push` is `pub`, `staging.rs:41`), so the second fill finds the row already Loaded.
3. `runner.rs:836` `run_system(upload_mesh_assets)` → `A::upload` allocates → `fill` fails at `assets.rs:454-455` "`let Some(idx) = self.resolve_reserved(handle) else { return Err((AssetError::StaleHandle, value));`" → the value is dropped.

**Reachability.** The drain runs only once, at boot (`runner.rs:835-837`), after `app.finish()` (`runner.rs:638`) has run every startup system. No in-tree scene calls `load` (`runner.rs:327-329`). So (a) and (c) are reachable from any user startup system. I did not establish whether (b) can occur at boot. All three become the normal streaming race once the drain runs every frame. A dedicated `OrphanedMeshGpu` queue exists for these values but nothing ever pushes to it; its doc is out of date (`mesh_assets.rs:729` "no `fill` caller exists in-tree yet").

### Red-first test for B — a device is unavoidable for the behaviour
`upload_assets` takes `&VulkanContext`, which "has no public/testable constructor outside a real device boot" (`asset_streaming_f6_churn_headless.rs:12-13`).
- **Device test** (`gpu:` class, `f6_churn` harness): in a startup system, reserve a handle, push `Staged { handle, cpu: cube MeshData }` (fields are `pub`, `staging.rs:14-19`), then `assets.remove(h)`. After boot, assert `OrphanedMeshGpu` is non-empty; with validation on, assert no undestroyed-VkBuffer report at device teardown. Today the queue is empty; the f6 test even asserts that as a regression net (`:333`).
- **Device-free tripwire only**: a source census, or `clippy::let_underscore_must_use`, flagging `gpu_upload.rs:120`. It fails today, but it checks text, not behaviour.

### Coverage and decision on B
ASSET-STREAMING-PLAN F6 (plan:327) designed the routing, but it was never wired at the only `fill` call site. The ledger's `KF-assets-adopt-retiring` is the end state.

**Fix now; the path is cold, so success costs nothing.**
- Route the `Err` value into the existing `OrphanedMeshGpu` / `OrphanedTextureGpu` queues. Both are already inserted (`runner.rs:247,315`), drained behind the fence gate (`asset_refcount.rs:633`) and force-drained at shutdown (`runner.rs:3498`).
- Also make `OrphanedMeshGpu::drain_ready` unregister a non-reserved `geometry_slot`. Today it destroys only the BLAS and buffers (`mesh_assets.rs:763-765`), which would leave a geometry-table row naming destroyed buffers.
- Later, `KF-assets-adopt-retiring` replaces both queues against the same test.
- What would overturn this: nothing on performance. Only an owner ruling that using the existing queues breaks the no-`Vec` side-store rule; in that case build `KF-assets-adopt-retiring` now (M) against the same test.

**A related gap I did not trace further:** `MeshGeometryTable::unregister` (`mesh_geometry_table.rs:758`) has no caller anywhere in the tree. So under a VisibilityBuffer boot, even a normally retired mesh may never release its geometry-table slot.

## Files
- D:/wt/joltab/crates/boyko_physics/src/resources.rs
- D:/wt/joltab/crates/boyko_physics/src/systems.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/warm_start.rs
- D:/wt/joltab/crates/boyko_physics/src/narrowphase/axis_cache.rs
- D:/wt/joltab/crates/boyko_physics/src/narrowphase/box_box.rs
- D:/wt/joltab/crates/boyko_physics/tests/sleeping_pipeline_o8.rs
- D:/wt/joltab/crates/boyko_ecs/src/ecs/core/archetype/archetype.rs
- D:/wt/joltab/crates/boyko_ecs/src/ecs/core/asset/assets.rs
- D:/wt/joltab/crates/boyko_ecs/src/ecs/core/asset/server.rs
- D:/wt/joltab/crates/boyko_ecs/src/ecs/core/asset/staging.rs
- D:/wt/joltab/crates/boyko_render/src/gpu_upload.rs
- D:/wt/joltab/crates/boyko_render/src/mesh.rs
- D:/wt/joltab/crates/boyko_render/src/mesh_assets.rs
- D:/wt/joltab/crates/boyko_render/src/texture.rs
- D:/wt/joltab/crates/boyko_render/src/asset_refcount.rs
- D:/wt/joltab/crates/boyko_rhi_vulkan/src/memory.rs
- D:/wt/joltab/crates/boyko_app/src/runner.rs
- D:/wt/joltab/crates/boyko_app/tests/asset_streaming_f6_churn_headless.rs
- D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md
- D:/claude/BoykoEngine/docs/physics/PHYSICS-ECS-UNIFICATION-RESEARCH.md

The ledger extracts I worked from are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/ledger/rev2/`: `_a_hits.txt` and `_b_hits.txt`.
