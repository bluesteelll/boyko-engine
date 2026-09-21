# F1 diagnosis: which system order the G2 edge changes, and why six hwrt TAA pins move

**Verdict.** The pixels change through one pair of systems: **`visibility_sync` and `gather_mesh_draws`**. The two have no ordering path between them, and in this composition their relative order decides whether **frame 0 draws any meshes**.

- **Pinned hashes (`c6429c3e…` and the other five):** `gather_mesh_draws` runs before `visibility_sync`. On frame 0 the gather sees **0 rows**, because every mesh's `RenderEnabled` bit is still clear. Frame 0 is therefore rendered **without meshes**, and TAA carries that frame into its history.
- **With the G2 edge (`1dde137b…`):** `visibility_sync` and its apply window run before the gather. Frame 0 draws all **7 meshes**.
- **The data-flow-correct order is the G2 one.** The pinned hwrt images, and every software TAA pin (whose order the edge does not change), encode a stale, meshless frame 0.
- **Cause chain:** the G2 edge only reshuffles Kahn's FIFO topological order. The executor's rule for exclusive systems then turns that reshuffle into a different wave order.

Throughout, **"measured"** means a run with the instrument, and **"read"** means read in the code.

## Paths and receipts

**Paths:**
- `S` = `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/a5f69b98-b880-43f1-a079-e964dc80e637/scratchpad/lighttable` (a session scratch folder on the development machine; its logs and the instrument patch are not in the repository)
- `OD` = `S/order_diag`, with runs in `OD/runs/<label>/`: `golden.log`, `order.txt`, `mesh_probe.txt` and the copied BMP.

**Run conditions:**
- **Tree:** `D:/wt/lighttable`, HEAD `1c31aeac`, plus the lane's 34 uncommitted files.
- **Build:** msvc, `CARGO_TARGET_DIR=D:/wt/_targets/vkval-msvc`, `CARGO_INCREMENTAL=0`, 6 jobs, no RUSTFLAGS.
- **Every render** was `scripts/golden.ps1 -Pin <pin> [-Hwrt]`, check-only, **never `-Bless`**.
- **Receipts:** every log reads `[golden] repo root = D:\wt\lighttable`, every `Compiling` path is under `D:\wt\lighttable\crates\…`, and every copied BMP's sha256 equals the logged `actual`.

## 1. How the executor orders two conflicting systems with no ordering path

All citations below are from HEAD (the lane does not touch the scheduler).

**Step 1: topological index.**
- `ScheduleBuilder::try_build` sorts with Kahn's algorithm, using a **FIFO** ready queue (`schedule_builder.rs:1048-1075`).
- The queue is seeded with every in-degree-0 system in **insertion-key order** (`:1058-1062`), and children are appended as their in-degree reaches 0 (`:1070`).
- Systems are permuted into that order (`:572-625`), and `ConflictGraph::build` sets a conflict bit for:
  - each access conflict (`conflict_graph.rs:109-118`, via `Access::conflicts_with`, `access.rs:214`);
  - each ordering edge (`conflict_graph.rs:148`).
- **Any added edge changes a successor's in-degree, and therefore where it and everything behind it lands in the FIFO.** One edge can reshuffle the topological indices of unrelated systems.

**Step 2: the executor runs in waves** (`schedule.rs:662-841`).
- The apply-window drain fires only when `pending == running || running == 0`, i.e. once **every** dispatched system has completed (`:748`).
- `pred_remaining`, `running` and `completed` change only inside:
  - that drain (`:860-904`);
  - the inline exclusive path (`:1420-1432`);
  - `mark_skipped` (`:1136-1149`).
- `try_dispatch_ready` scans `0..n` in topological order (`:1258`) and dispatches every ready system whose conflict bits miss the live `running` set (`:1272`). Each dispatch is inserted into `running` at once, so later indices in the same scan see it (`:1310`).
- Scans between two drains therefore see the same state and cannot dispatch anything new. **The dispatch sequence is a pure function of** (topological order, edges, conflict bits, kinds, run-condition results).

**Step 3: the rule that decides this bug.** A dispatcher-solo system (`CpuExclusive` or `GpuCompute`, `system_kind.rs:71-73`):
- is **skipped with `continue`** if anything is already running (`schedule.rs:1296-1297`);
- once accepted, ends the scan with `break` (`:1304`).
- So two conflicting, unordered systems run in the order they become ready. Within a wave the lower topological index wins, **except** that a concurrent system with a higher index overtakes an exclusive system with a lower index whenever the wave already contains any concurrent system.
- `gather_mesh_draws` takes `NonSendRes<Assets<MeshGpu>>`, so its access is **universal** (`resource.rs:92`) and it is `CpuExclusive`.

**Determinism, measured (G2 tree, `taa_armed` hwrt, 34 Main frames):**

| Property | Result |
|---|---|
| Dispatch sequence (round counters stripped) across frames | Identical in every frame |
| Across 2 runs (`G2_a`, `G2_b`) | Identical |
| Across pool sizes 1, 3 and 16 (`W1_G2`, `W3_G2`, `W16_G2`; `POOL workers=` confirmed in the dump) | Identical |
| Wave partition in those same runs | Identical |
| Image in those same runs | Identical: `1dde137b` |

- **Golden worker count:** the golden host uses `App::new()` (`taa_jitter_eval.rs:493` → `app.rs:201-202`), which takes the default pool = `available_parallelism` (`thread_pool.rs:669,806`). That is **16 workers** on this box, and the dispatch order does not depend on it.
- **Across machines** (read, not measured): the order is determined by the binary, i.e. insertion order plus the feature set. hwrt adds a system and an edge, so the two legs have different orders.

**Two things that are NOT deterministic:**
1. **The order in which applies run within a wave.** The drain pops the completion queue in completion order (`schedule.rs:860`).
   - Measured: at 3 and 16 workers, **34 distinct apply sequences in 34 frames**. At 1 worker, 1 sequence.
   - `Commands` from two systems in the same wave are therefore applied in a timing-dependent order.
2. **The Fixed schedule's step count.** It depends on the wall clock: 14, 15 and 16 steps were logged across runs. Here Fixed holds only `pack_gpu_transforms`.

The `taa_armed` image is stable despite both. Neither is what moves F1.

## 2. The instrument

**Patch:** `S/order_instrument.patch`, 495 lines: 353 inserted lines, 284 of them the new `order_dump.rs`. It applies cleanly with `git apply` (re-applied once mid-session to prove it). It touches:
- `schedule/mod.rs`, `schedule.rs` (+11 lines), `schedule_builder.rs` (+37 lines);
- the new `schedule/order_dump.rs`;
- `boyko_threadpool/src/thread_pool.rs` (+4 lines);
- `boyko_render/src/mesh_draw.rs` (+16 lines).

It passes `cargo check` and `cargo clippy -p boyko-ecs -p boyko-threadpool -p boyko-render --features boyko-render/hwrt -- -D warnings` (exit 0; a forced re-lint showed `Checking` for all three crates).

**Env vars.** None is `BOYKO_*`, because `golden.ps1` wipes every `BOYKO_*` the pin does not name.

| Variable | Effect |
|---|---|
| `ECS_ORDER_DUMP=<file>` | Enables the dump |
| `ECS_ORDER_FRAMES=<n>` | Number of frames logged; default 40, 60 was used |
| `ECS_ORDER_FORCE=kA>kB,…` | Injects insertion-key edges before Tarjan/Kahn, in schedules with n>1. Each one is logged as a `FORCE` line with names |
| `ECS_ORDER_WORKERS=<n>` | Overrides the default pool size |
| `MESH_DRAW_PROBE=<file>` | Makes `gather_mesh_draws` log its row count on each frame. The count comes from the gather's own query, including the `Enabled<RenderEnabled>` filter |

**What it writes:**
- **At build:** one `POOL` line. Then, per schedule, `SYS idx key kind label [in: sets]`, `ACC` (the declared access with type names), `EDGE`, and `AMB i j class detail`.
  - A pair is ambiguous when the two systems cannot overlap (an access conflict, or one side is dispatcher-solo) **and** neither can reach the other through ordering edges. Reachability is the transitive closure; set edges are included, because the builder expands them into system edges.
- **At run:** `RUN sched frame` lines listing `D`/`X` (dispatch, concurrent or dispatcher), `A` (apply) and `S` (skipped).
- **Analysis scripts:** `OD/analyze.py` (summary, diff and show), `OD/separate.py` (the pair that separates PASS runs from FAIL runs) and `OD/amb_list.py`.

**Restore proof** (`OD/snap/` holds pre-edit copies; `OD/pre_*.sha256` and `OD/pre_gitdiff.patch` were taken before the first edit):
- **Final state:**
  - lane files 34/34 sha256 OK;
  - the 5 touched tracked files 5/5 OK and equal to HEAD;
  - `order_dump.rs` absent;
  - `git diff` byte-equal to the pre-snapshot (`b8247c53…`, the tester's `final_gitdiff.patch`);
  - `git status` identical (34 entries); HEAD `1c31aeac`; stash empty.
- **The tester's `tester_close/verify.sh`** printed `VERIFY OK` as the last command.
- **Rebuild:** files were restored by plain copy, which gives fresh mtimes so cargo cannot treat the instrumented artifacts as fresh. The target dir was then rebuilt from the restored tree: `Compiling boyko-threadpool, boyko-ecs, … boyko-render, boyko-app (D:\wt\lighttable\…)`.
  - Receipt, `OD/final2/`: `taa_armed` hwrt **FAIL `1dde137b…`** (F1, unchanged); software **PASS `31ce5178…`**.
- **`light_plugin.rs`:** the NOG2 mutation (`OD/mutate.py`: exact bytes, CRLF, exactly one match) was restored by copy after each use and re-verified at 34/34.

## 3. Executed order with and without the G2 edge (`taa_armed` hwrt)

**The schedule:**
- **Main:** 36 systems (29 `CpuConcurrent` + 7 `CpuExclusive`). 26 edges with G2, 25 without.
- **Fixed:** 1 system.
- **Hashes:** `G2_a` = `1dde137b` (identical to the tester's), and `NOG2_a` = `c6429c3e` (PASS). The instrument does not move the image.
- **Strict relative-order flips** between the arms: **11 pairs, in 34 of 34 frames each.**

| # | Pair | Pinned order (no G2) | G2 order |
|---|---|---|---|
| 1 | `select_lighting_cull` / seed | cull < seed | seed < cull (the G2 edge itself) |
| 2 | `collect_lights` / `propagate_transforms` | collect < propagate | propagate < collect |
| 3 | `gather_shadow_casters` / `resolve_active_camera` | casters < camera | camera < casters |
| 4 | `gather_mesh_draws` / `resolve_active_camera` | mesh_draws < camera | camera < mesh_draws |
| 5 | `gather_shadow_casters` / `visibility_sync` | casters < visibility | visibility < casters |
| **6** | **`gather_mesh_draws` / `visibility_sync`** | **mesh_draws < visibility** | **visibility < mesh_draws** |
| 7–10 | the seed / `resolve_shadow_atlas`, `particle_pack_effects`, `sync_instance_model_cols`, `drive_camera_motion` | other < seed | seed < other |
| 11 | `particle_tick_emitters` / `reduce_caster_bounds` | reduce < tick | tick < reduce |

- Pairs 2–11 are **ambiguous in both arms** (the `EXCL` class).

**Why pairs 2–6 flip (the dispatch sequences, frame 0):**
- **Pinned:** `… D24 collect_lights · X26 propagate_transforms · X27 gather_shadow_casters · X28 gather_mesh_draws · D29 {resolve_active_camera, visibility_sync} …`
- **G2:** `… X16 propagate_transforms · D17 {collect_lights, resolve_active_camera, visibility_sync} · X20 gather_shadow_casters · X21 gather_mesh_draws …`

The steps from the edge to the flip:
1. The edge gives `select_lighting_cull` in-degree 1.
2. It therefore leaves Kahn's initial FIFO and re-enters after the seed.
3. That swaps two topological indices: `collect_lights` 25 → 26 and `propagate_transforms` 26 → 25.
4. With G2, `propagate` (exclusive) runs alone first.
5. The next scan dispatches the concurrent `collect_lights`, then skips (`continue`) the exclusive gathers at indices 27 and 28, because something is now running.
6. The same scan dispatches `resolve_active_camera` and `visibility_sync`, so both overtake the gathers.
7. Without G2, `collect_lights` had already run before `propagate`. The scan after `propagate` therefore found `running == 0`, and the gathers ran first.

## 4. Bisection: the one responsible pair

**Method:** the G2 edge was kept, and `ECS_ORDER_FORCE` injected edges; every forced run's executed order was diffed. Pinned = `c6429c3e`, G2 value = `1dde137b`.

| Run | Forced edges (`→` = runs before) | Result | `gather_mesh_draws` rows on frame 0 |
|---|---|---|---|
| B1 | casters→{camera, vis}, mesh_draws→{camera, vis} | **PASS** | – |
| B2 | casters→{camera, vis} | **PASS** (forcing the casters first also moved the mesh gather ahead of camera and visibility) | – |
| B3 | mesh_draws→{camera, vis} | **PASS** | – |
| T1 | casters→{camera, vis}; {camera, vis}→mesh_draws | **FAIL** `1dde137b` | – |
| T2 | mesh_draws→{camera, vis}; {camera, vis}→casters | **PASS** | – |
| T3 | mesh_draws→camera, vis→mesh_draws; {camera, vis}→casters | **FAIL** `1dde137b` | – |
| T4 | mesh_draws→vis, camera→mesh_draws; {camera, vis}→casters | **PASS** | – |
| T5 | T4 + mesh_draws→particle_tick (removes a confound) | **PASS** | – |
| T6 | camera→casters→mesh_draws→{vis, particle_tick} | **PASS** | – |
| **T7** | **one edge only: mesh_draws→vis.** Strict flips vs G2: only vis/mesh_draws and vis/casters | **PASS** | **0** |
| P_G2 | none (the lane tree) | FAIL `1dde137b` | **7** |
| P_NOG2 | none, without the G2 edge | PASS | **0** |
| **P_NOG2_vis_first** | **without G2, plus the one edge vis→mesh_draws** | **FAIL `1dde137b`** (the converse direction) | **7** |
| W1/W3/W16 | none, 1, 3 and 16 workers | FAIL `1dde137b` ×3 | – |
| VB_G2 (`vb_taa_rcas`) | none | FAIL `3e873c4b` (F1's value) | 7 |
| **VB_G2_md_first** | **mesh_draws→vis** | **PASS `6b59bcee`** (pinned) | **0** |

**`OD/separate.py` over all 19 `taa_armed` hwrt runs** (10 with the image equal to `c6429c3e`, 9 equal to `1dde137b`):
- It searched all 630 pairs for a relation that is constant within the PASS runs, constant within the FAIL runs, and different between them.
- **Exactly one pair qualifies:** `visibility_sync` (k3) vs `gather_mesh_draws` (k33).
  - PASS ⟺ the gather runs first.
  - FAIL ⟺ `visibility_sync` runs first.
- **Casters, camera, collect_lights and seed ordering all proved irrelevant:** T1 and T3 FAIL, T2 and T4 PASS.
- **Two-way proof on the same pair:**
  - adding `vis → mesh_draws` to the pinned tree produces the G2 hash;
  - adding `mesh_draws → vis` to the G2 tree restores the pinned hash;
  - this holds on `taa_armed` and on `vb_taa_rcas`.

## 5. Mechanism

**Data flow, from reading the code:**
- `MeshBundle` has no `RenderEnabled` (`bundles.rs:43-58`).
- `RenderEnabled` is an `EnableTag` bitset whose bit is **clear until set**: "No directory slot or no page yet ⇒ the bit is clear" (`enable_store.rs:209`).
- `visibility_sync` (`visibility_sync.rs:143`) is `Changed<Visibility>`-gated. On its first run after spawn it enqueues a `SetRenderEnabledById` command per mesh, which **enables** the bit in its apply window. ⚠ **Superseded 2026-09-21 (rungs A9 / A9b)**: `SetRenderEnabledById` no longer exists — it is `SetRenderEnabled`, keyed by the full `Entity` resolved at enqueue through the `Entities` param. The by-id form re-resolved the row at apply and was hazard H-06: a toggle pending for a despawned `E` landed on the `F` spawned on `E`'s recycled id in the same frame (`boyko_scene/tests/visibility_sync_gates.rs` gates 5–6). `boyko_render`'s two copies of the pattern (`asset_refcount.rs` `DisableStaleMeshCommand`, `snap_interpolation.rs` `DisableSnap`) were fixed the same way (`boyko_render/tests/deferred_toggles_recycled_id.rs`). The mechanism this bullet describes — the bit is enabled in the apply window — is unchanged.
- `gather_mesh_draws` filters on `Enabled<RenderEnabled>` (`mesh_draw.rs:1253` / hwrt `:1368`).
- So a gather that runs before `visibility_sync`'s apply sees the **previous frame's** bits, which on frame 0 are none.

**Measured row counts:**
- Pinned order: frame 0 = **0**, frames 1–33 = 7.
- G2 order: 7 on every frame.
- Same split in the forced runs above.

**The data-flow-correct order** is `visibility_sync` (plus its apply) before `gather_mesh_draws`, which is the G2 arm.
- `visibility_sync`'s own contract (`visibility_sync.rs:127`) says it and its apply window "must run BEFORE" the pack that filters on `Enabled<RenderEnabled>`, so that the authoring intent of frame N reaches the draw list of frame N.
- **No edge encodes this for the gather:**
  - `visibility_sync` is only `.after(propagate)` (`camera_plugin.rs:73`);
  - `gather_mesh_draws` is only `.after(pack).after(snap)` (`plugins.rs:777`).
- **So the six pinned hwrt hashes contain a meshless frame 0**, which is one frame of lag carried into TAA history. The G2 values are the correct frame.

**Why only hwrt:**
- The software leg's `gather_mesh_draws` runs before `propagate_transforms` and `visibility_sync` **in both arms**:
  - `sw_G2_a` and `sw_NOG2_a`: `X20 gather_mesh_draws · X21 propagate · D22 visibility_sync`;
  - measured on the lane tree: frame 0 = **0 rows** (`sw_P_G2`);
  - software stays `31ce5178` in both arms.
- **The software pins therefore carry the same meshless frame 0**, and the edge does not change that.
- **The cause is a topology difference between the legs:**
  - hwrt registers `sync_prev_instance_model_cols.before(pack)` behind `#[cfg(feature = "hwrt")]` (`plugins.rs:682-683`);
  - that raises `sync_instance_model_cols`' in-degree, so it and its successors (both gathers) land later in Kahn's FIFO;
  - `gather_mesh_draws` moves from index 25, before `propagate` at 26 (software), to index 28, after `propagate` at 25/26 and next to camera/visibility at 29/30 (hwrt);
  - only there does the wave interaction from §3 decide its order relative to `visibility_sync`.
- **This also explains the lost VB software/hwrt parity:** both legs used to have a meshless frame 0; now only software does.

**Why only mesh outlines and the sphere's shadow edge** (read, not measured per frame):
- The whole frame-0 difference is "meshes absent". Floor, wall and cubes are all meshes; the SDF sphere is not.
- TAA's neighbourhood clamp should discard stale history in flat regions. At silhouettes and shadow edges the clamp box spans both sides, so part of the stale frame-0 value can survive.
- By the capture frame that value has been blended down by ~30 frames of feedback (`default_blend` 0.1, `min_blend` 0.015, `taa_config.rs:398-403`). That is consistent with residuals of 7–12 levels on edges only.
- `vb_sdf_taa` (`GEOMETRY_LEGS=sdf`, no meshes) cannot differ, and it does not.
- **Scope:** the other four moved pins use the same test binary (`taa_jitter_eval`) and the same Main schedule. Measured for `vb_taa_rcas`: the executed order is identical to `taa_armed`'s, and the single-edge restore brings back its pin. `taa_armed_basis`, `taa_rcas`, `vb_taa` and `vb_both_taa` were not re-run individually.
- **Cross-check:** PINS.toml's SG4 note says frame 0 has "no caster batch yet" (`goldens/PINS.toml:864`). `gather_shadow_casters` also filters on `Enabled<RenderEnabled>` and runs before `visibility_sync` in software, so that note is the same lag seen through the caster gather.

## 6. Full ambiguous-pair list (hwrt `taa_armed` host, Main)

**Counts:**
- **With G2:** 236 ambiguous pairs = 203 `EXCL` (at least one side universal) + 33 `DATA` (both concurrent, declared access conflict) + 0 `SOLO`.
- **Without G2:** 237 (seed/cull becomes ambiguous).

**Full lists** (DATA pairs with the conflicting item and R/W on each side; EXCL pairs grouped per exclusive system, so EXCL–EXCL pairs appear under both):
- `OD/ambiguous_pairs_hwrt_taa_armed_G2.txt`
- `OD/ambiguous_pairs_hwrt_taa_armed_NOG2.txt`
- raw `AMB` lines in `OD/runs/G2_a/order.txt`

**The 33 DATA pairs:**
- **The `LightingConfig`/`LightTableDirty` W/W cluster:** 23 pairs among `sync_ssao/sv0/cluster/csm/punctual_light_gate`, `select_lighting_cull`, `collect_lights` and `resolve_shadow_atlas`.
- `light_reconcile` (writes Point/Spot/`DirectionalLight`) vs:
  - `resolve_shadow_atlas`, `select_lighting_cull` (read Point/Spot);
  - `resolve_csm_cascades` (reads `DirectionalLight`).
- `resolve_active_camera` (writes `ViewUniform`) vs `resolve_shadow_atlas` and `resolve_csm_cascades` (read it).
- `sync_csm_light_gate` vs `resolve_csm_cascades` (`ResolvedCsm` R/W).
- `snap_apply` vs `drive_camera_motion` and `drive_moving_caster_motion`, and those two against each other (`Transform`).
- `particle_pack_effects` vs `particle_tick_emitters` (`ParticleClock`).

**The EXCL class:** every exclusive system — `propagate_transforms`, `apply_refcount_deltas`, `validate_asset_refs`, the seed, `gather_shadow_casters`, `reduce_caster_bounds`, `gather_mesh_draws` — is unordered against 29–34 systems.

**Other pairs that look image-relevant** (not measured; read from the dump and code):
1. **`visibility_sync` → `sync_gpu_3d_instances` and `sync_instance_model_cols`.** Both filter on `Enabled<RenderEnabled>`, and both run before `visibility_sync` in every arm and on both legs (waves 1 and 6, against 17, in the G2 hwrt arm). So they carry the same frame-0 lag. For `InstanceModelCol` this is benign in a static scene, because `MeshBundle` seeds it.
   - `visibility_sync.rs:127` names `sync_gpu_3d_instances` as the system that must run after it.
   - **These pairs are not in the AMB list at all:** `visibility_sync` declares only `CR Visibility`, and its real effect is a deferred `Commands` write. `sync_gpu_3d_instances` declares no read of `RenderEnabled` (its `ACC`: `CR GlobalTransform`, `CR MaterialHandle`, `CW Gpu3dInstance`). **Neither the `Commands` write nor the `Enabled<T>` filter read is visible to `Access`.** The pair that caused F1 was flagged only because the gather happens to be exclusive.
2. **`propagate_transforms` vs `sync_instance_model_cols` and `sync_gpu_3d_instances`** (EXCL). Both packs run before propagation in both arms (waves 1 and 6 vs 16), so `InstanceModelCol` and `Gpu3dInstance` trail `GlobalTransform` by one frame. That is harmless for this static pin and relevant to the moving-object modes (`BOYKO_TAA_MOTION=object`).
3. **`resolve_active_camera` → `resolve_csm_cascades`, and `light_reconcile` → `resolve_csm_cascades`.** These are genuine DATA dependencies (`ViewUniform` and `DirectionalLight`). They run in the correct order today only by accident of wave placement: `resolve_csm_cascades` is last in both arms.
4. **`resolve_active_camera` vs `resolve_shadow_atlas`** (`ViewUniform`). The atlas runs in wave 6, before the camera (wave 17), so punctual shadow ranking uses the previous frame's view. Not relevant here, since the scene has no punctual lights.

## Not done / limits

- **No fix was made, and nothing was blessed or committed.**
- The data-flow-correct edge (`visibility_sync` before the gathers) and whether to re-bless are for the architect and the owner.
  - An edge there would move the software TAA pins as well, since their frame 0 would gain meshes. That was not measured.
- Worker-count determinism was measured on one machine only; cross-machine determinism is read from the code.
- The TAA-clamp explanation of the edge-only residue is read, not traced per frame.

## Process incident (nothing changed)

- While I was editing this report, an unquoted bash heredoc expanded backticked text as commands. One of them ran a bare `git apply` in the main checkout (`D:/claude/BoykoEngine`), which waited on stdin.
- It was killed and ended with `error: No valid patches in input`, so **nothing was applied**.
- Afterwards the main checkout's `git status` shows only the pre-existing `M .claude/settings.local.json`, and the lane's `verify.sh` printed `VERIFY OK`.
