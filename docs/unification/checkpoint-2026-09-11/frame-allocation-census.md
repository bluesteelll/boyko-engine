# Frame-allocation census - survey, measurement, attribution and adjudication

- **Source:** workflow `frame-allocation-census` (run `wf_df9ef61b-d9b`); phases 1-2 on D:/wt/ecsnative @ ad0ebea4 (pre-Stage-3b pool), phases 3-4 on D:/wt/joltab (KE16 pool)
- **Status:** COMPLETE; adjudicated SOUND with two gate defects (fixed later, see census-gate-fix.md). The harness files are uncommitted in D:/wt/joltab.
- **Copied verbatim** on 2026-09-11 at the checkpoint the owner asked for; agent outputs are reproduced without edits.

---

## Phase 1 - survey

# Phase 1 survey — what the tree can and cannot say about allocations per frame

Tree: `D:/wt/ecsnative` @ `05fcbd1d` (`feat/ecs-native-storage`). Read-only; no code written, no build run.

## TL;DR

1. There are **20** counting test binaries, not 19 (the "nineteen" in `alloc_shim.rs` and the brief predates `profiling_residency.rs` / `profiling_overlay_alloc.rs`), plus 3 lib-test-module counters and the opt-in process shim. **None of the 24 measures a frame.** Every gate either bypasses `Schedule::run` (isolated helper / `run_system_once` / direct call) or drives `Schedule::run` and then **subtracts it as a "baseline"**.
2. That baseline is not zero and it is not noise — it is structural. `Schedule::run` allocates by construction on every run: one `Box<ScopeShared>` per `pool.install` (`D:/wt/ecsnative/crates/boyko_threadpool/src/thread_pool.rs:324`) plus **one heap cell per spawned concurrent system** (`D:/wt/ecsnative/crates/boyko_threadpool/src/task.rs:80-82`: "One `alloc` per spawn and one `dealloc` per task"). Every `par_iter` and every physics `pool.scope` adds another `Box<ScopeShared>` (`thread_pool.rs:374`) plus one cell per chunk/wave, because the shipped `spawn_batch` arm is "one `spawn` per body" (`scope.rs:1000-1006`; `default = []` in `boyko_threadpool/Cargo.toml:37`). The bump arena that would collapse this to one alloc per chunk exists but "is wired to NOTHING" (`block.rs:4-9`).
3. **The frame total is already computed in three gates and discarded.** `constraint_graph_o4_world.rs:294-330`, `colored_solve_zero_alloc_o5.rs:215-260` (`default_delta`, a warmed full physics `Schedule::run`), and `boyko_ui/tests/zero_alloc.rs:193` / `boyko_scene/tests/gates_zero_overhead_alloc.rs:183` (`base`) each hold a real per-run count of a real schedule and assert only a delta against it. The one absolute number the tree records is a prose remark — "measured: ~7 per frame for a 2-system schedule" (`boyko_ui/tests/zero_alloc.rs:8-9`) — with no date, no tree state, and no assertion.

---

## (a) The gates — window, driver path, and what each deliberately excludes

Counter kinds: **TL** = thread-local on the measuring thread; **PG** = process-global atomic (serialised by a lock); **bytes** = bytes not calls.

| # | Binary | Window measured | Goes through `Schedule::run`? | Counter | Explicitly excluded |
|---|---|---|---|---|---|
| 1 | `boyko_app/tests/zero_alloc.rs:100-215` | `gather_mixed_into` + 4 `upload_*` + `gbuffer_push_from_view`, 100 iterations over fake mapped slots | No | PG (single test) | Windowed loop, draw-list assembly (`DrawListScratch`, crate-private), any system, the executor — stated at `:20-23` |
| 2 | `boyko_ecs/tests/profiling_residency.rs:156-249` | `Profiler::arm` / re-arm | No | TL bytes | Not a frame gate at all — residency of the profiler store |
| 3 | `boyko_input/tests/zero_alloc.rs` | `process_actions` loop, raw queue push/pop, `PhysicalInput::begin_frame` | No | TL | The system fn, the scheduler ("which allocates on first init") |
| 4 | `boyko_input/tests/i4_zero_alloc.rs` | The exact ops of `update_action_state` body over preallocated resources | No (`:6-9`) | TL | The scheduler, the `ResMut`/`Res` fetch |
| 5 | `boyko_scene/tests/gates_zero_overhead_alloc.rs:248-267` | 4 warmed `sched.run` of `[propagate_transforms]` **minus** 4 warmed runs of `[noop_exclusive]` | **Yes, subtracted** | PG + lock | The executor's own per-run allocation, by design (`:22-27`); note the baseline shape is an *exclusive* system (no spawn cell) vs. a concurrent one — the shapes are not equal |
| 6 | `boyko_physics/tests/broadphase_grid.rs:390-420` | One `grid.build` after 4 warm builds, 300 bodies | No | TL | Everything but `BroadphaseGrid::build` |
| 7 | `boyko_physics/tests/colored_parallel_alloc_o6.rs:181-320` | One warmed `solve_colored` inside `pool.install`, 8 and 600/1200 bodies, 4 workers | No (direct solver call) | TL on dispatcher | Manifolds + graph build rebuilt *outside* the window (`:169-172`). The dense-world bound is `12 × 136 × 2 = 3264` allocations per step (`:262-266`) — a real number is printed via `eprintln!` (`:229`) and never recorded |
| 8 | `boyko_physics/tests/colored_solve_zero_alloc_o5.rs:144-212` | One warmed `solve_colored`, no pool | No | TL | Graph build outside window |
| 8b | same file `:215-260` | One warmed **full physics `Schedule::run`** (serial pool, 5 bodies), colored minus default | **Yes, subtracted** | TL | The absolute `default_delta` is discarded |
| 9 | `constraint_graph_o4_world.rs:230-291` | One warmed `ConstraintGraph::build`, 512 bodies | No | TL | 1 debug-only alloc tolerated |
| 9b | same file `:294-330` | Warmed full physics `Schedule::run`, colored minus default, bound `extra <= 4` release | **Yes, subtracted** | TL | "observed ~2 in release" per extra system (`:318-319`) — the only surviving per-system figure |
| 10 | `large_island_gate_p2.rs:383-410` (Gate B, `cfg(not(miri))`) | Warmed parallel `solve_colored`; asserts zero scopes below threshold, non-zero bounded above | No | TL | Same shape as #7 |
| 11 | `soft_body_sp1.rs:808-862` | One `run_system_once(physics_soft_step)` after 120 warm | No — explicitly to exclude `Schedule::run` (`:812-816`) | TL | The comment at `:813-815` names `exclusive_to_run`/`to_spawn` Vecs and `empty_intent` clones as the executor's per-step cost — **both are stale**: the Vecs are `mem::take`n scratch since `executor_scratch.rs:270-282` / `schedule.rs:1044-1055`, and `empty_intent` lives in the build-time `gpu_barrier_inputs` (`schedule.rs:1417-1421`, "never on the per-frame run path") |
| 12 | `soft_colored_sp4_alloc.rs:105-115` | One `run_system_once(physics_soft_step_colored)`, colored minus serial | No | TL | The `run_system_once` "harness floor" is subtracted, not reported |
| 13 | `boyko_render/tests/sdf_edit_gather.rs:203-233` | **4 × `app.update()`**, asserts exactly 0 | **Yes — the only App-level gate** | TL on calling thread | `App::new()` + `SdfPlugin` only; `SdfPlugin` registers no system (`:55-58`), so `Schedule::run` hits `if self.systems.is_empty() { return; }` (`schedule.rs:435`) **before** `pool.install`. The zero is a zero over an empty schedule. |
| 14 | `boyko_render/tests/ui_no_realloc.rs` | Pack + sort of `Vec<UiInstance>` at fixed N | No | PG armed | Everything but the pack lane |
| 15 | `boyko_ui/tests/zero_alloc.rs:212-300` | Layout pair schedule minus `[noop_normal, noop_exclusive]` | **Yes, subtracted** | PG + lock | Baseline asserted `> 0` at `:223-228` (so it is *known* nonzero) and then only used as subtrahend |
| 16 | `boyko_ui/tests/p3_watch_zero_alloc.rs` | `ui_hot_reload_system` called directly | No (`:7-9`: "NOT through the App scheduler, whose per-frame dispatch allocates a fixed amount") | PG + lock | Scheduler |
| 17 | `boyko_ui/tests/p4_bind_zero_alloc.rs:203-278` | Bind pair schedule minus noop pair | **Yes, subtracted** | PG + lock | Executor (`:5-8`) |
| 18 | `boyko_ui/tests/profiling_overlay_alloc.rs:204` | `write_row` 600 × 8 + positive control | No (`:21-25`) | TL | Query iteration, `Mut` deref, the scheduler |
| 19 | `boyko_ui/tests/text_emit_zero_alloc.rs` | `emit_glyphs` / `measure_one` at fixed glyph count | No | guard-armed | Everything else |
| 20 | `boyko_ui/tests/world_scratch_zero_alloc.rs:200-225` | `run_cached_system(ui_world_pick_system)` | No (`:18-21`) | TL | `into_system`, the scheduler |

Lib-test-module counters (not integration binaries): `boyko_app/src/runner.rs:3716` (`ingest_captured_is_alloc_free_when_warm`, input bridge only), `boyko_scene/src/camera.rs:1029`, `boyko_threadpool/src/block.rs:711` (the unwired arena's own pins). The opt-in shim `boyko_app/src/profiling/alloc_shim.rs:145` is process-total from start-up and, per `:126-131`, **cannot be enabled in any test sweep** because it collides with every binary above.

### Union of the twenty against one frame

`App::update_with_delta` (`D:/wt/ecsnative/crates/boyko_ecs/src/ecs/core/app/app.rs:660-745`) is: ⓪ `fold_frame` → ① `Time::advance_with` → ② periodic cold `check_ticks` → ③ `update_events` → ④ `fixed_advance` × N `Schedule::run` → ⑤ Main `Schedule::run`.

| Frame step | Covered by a gate? |
|---|---|
| ⓪ fold, ① time, ② check-ticks | No gate. (Profiler store residency is #2, not its per-frame fold.) |
| ③ `update_events` (`event_dispatcher.rs:447-480`, `swap_and_flatten` `:594`) | No gate. |
| ④/⑤ **`Schedule::run` itself** — `pool.install` Box + one cell per concurrent system + apply window | **No gate asserts it; four gates subtract it; one gate (#13) asserts zero over an empty one.** |
| Physics stage bodies | Yes, each in isolation (#6-#12), with dispatch scopes bounded (#7, #10) |
| `par_iter` inside a system (1 Box + `n_chunks` cells per archetype, `par_iter.rs:341-403`) | No gate. Scene propagation (#5) and UI pairs (#15, #17) would fold it into the subtracted delta only if the *baseline* also par-iterated, which it does not — so if those systems par-iterate, the delta hides nothing only because they don't. |
| `Commands` apply (per-system `CommandQueue` state, capacity retained; `deferred_hook_queue` twin at `command_queue.rs:484-560`) | No gate. |
| Render frame helpers (gather + uploads) | Yes, #1, in isolation |
| Draw-list assembly, windowed loop, swapchain | No headless gate (device-needing, `#1:20-23`) |
| Input ingest | Yes, #3/#4 + `runner.rs` module, in isolation |
| UI layout / bind / pick / text / pack | Yes, #14-#20, each isolated or subtracted |

So the honest reading: the gates jointly prove that **system bodies** the engine ships are steady-state alloc-free in isolation, and jointly prove nothing about the **glue between them**, which is exactly where the allocations are known (not suspected) to be.

---

## (b) Headless drivers that can run a real scene with no GPU

| Driver | Path | Subsystems exercised | Not exercised |
|---|---|---|---|
| **Jolt-parity pyramid** | `D:/wt/ecsnative/crates/boyko_physics/benches/jolt_parity_pyramid.rs:188-235` — `EcsMaster::new` + `spawn_jolt_pyramid` (1240 dynamic + floor) + `add_physics_colored_solve` + `schedule.run` | Fixed-step physics through the real `Schedule`: 8 concurrent stage systems (`plugin.rs:557-638`, all param-based — none exclusive, so each is a spawn cell), colored solver `pool.scope` waves (`solver/colored.rs:2868-2942`), parallel broadphase scopes (`resources.rs:1958, 2050`), `SolverScratch`/`ScratchColumn` reuse | Events, commands, spawn/despawn, states, UI, render gather. Colored + parallel are opt-in and ship OFF (`:39-43`) — a default-config variant needs `add_physics_systems` (7 systems) |
| **KE16 solve-in-system** | `crates/boyko_physics/benches/ke16_solve_in_system.rs` (12 `schedule.run` sites) | Solver dispatched *from a worker* (the production route) | Same gaps as above |
| **Physics acceptance suites** | `colored_acceptance_o5.rs` (13 runs), `softstep.rs` (25), `sleeping_pipeline_o8.rs` (5, **spawn/despawn mid-sleep** = topology churn through the real pipeline), `bodytype_bits_toggle.rs`, `scene_sync_s5.rs` (physics ↔ `Transform` sync = scene crate) | Full rigid pipeline; `o8` adds structural change under physics | No events/UI/render |
| **Demo `SimRunner`** | `crates/boyko_demo/src/sim/runner.rs:153-382` — native path: real `ThreadPool` + one `Schedule` with `insert_state`, `on_enter`/`on_exit` spawn/despawn systems, `par_iter` sim systems, `fixed_advance`. Driven headless by `crates/boyko_demo/tests/{mode_switch,physics_smoke,state_exclusive_smoke,interpolation}.rs` | States + transition pass, **bulk spawn (`scatter_spawn`) and despawn-by-tag on mode switch**, `par_iter` fan-out (`:14-15`), grid build, GPU-column sync systems, demo ball physics | Kernel `Commands` (spawns are direct `create_entity`), `EventWriter/Reader`, boyko_physics, render gather |
| **ECS kernel suites** | `phase17_states.rs` (53 runs; header `:5-8` confirms "even on a `num_threads(1)` pool" every non-exclusive system goes through `Scope::spawn`), `phase12_events_systemparam.rs` (12; `EventWriter/EventReader` through the schedule), `phase16_run_conditions.rs`, `phase10_change_detection.rs`, `phase22_static_tags.rs`, `ke5_condition_combinators.rs` | Executor, conditions, states, events, change detection, tags | No physics/UI/render; small entity counts |
| **App-level, device-free** | `boyko_render/tests/sdf_edit_gather.rs` and `boyko_app/tests/sv0_adequacy.rs:305-616` — `App::new()` + `SdfPlugin` + `app.update()` | The `App` frame funnel (⓪-③) with an **empty** schedule | Everything else; `EnginePlugins` has only a `window(...)` constructor (`plugins.rs:124`) — no headless variant exists, so the shipped Main schedule cannot be run without a device |
| **Scene / UI** | `boyko_scene/tests/gates_zero_overhead_alloc.rs`, `visibility_sync_gates.rs` (17), `boyko_ui/tests/p6a_common`, `zero_alloc.rs`, `p4_bind_zero_alloc.rs` | Hierarchy propagation; UI layout/bind pairs through a 2-system schedule | Isolated pairs, not a frame |
| Benches with `#[global_allocator]` (`boyko_ecs/benches/*`, `bench_bevy_vs_boyko/benches/*`) | e.g. `phase9_scheduler.rs:33-36` | **Not counters** — an opt-in `mimalloc` swap under `bench-alloc`. Useful only as drivers (query iteration, event dispatch, spawn batch, scheduler) | — |

---

## (c) What "a scene" can honestly mean — a range, not an average

Derived from the code, the steady-state per-frame count has a **certain** part and a **candidate** part:

```
certain(frame) = Σ over Schedule::run    [ 1 (ScopeShared) + C_s (spawn cells, one per concurrent system) ]
               + Σ over par_iter calls   [ 1 + Σ_archetypes ceil(n / chunk_size) ]     (par_iter.rs:341-403; chunk_size = n / workers, clamped ≥ MIN_ARCHETYPE_FOR_PARALLEL, par_iter.rs:119-125)
               + Σ over physics scopes   [ 1 + n_waves ]                              (resources.rs:1958/2050; solver/colored.rs:2868 — passes × colors above threshold)
```

`C_s` for the default physics fixed step is 7 (8 colored); the O4 gate's "~2 per extra system in release" (`constraint_graph_o4_world.rs:318`) vs. the 1 cell the threadpool documents is an **unexplained factor of two** that Phase 2 must measure rather than assume.

Candidates the census fields contribute only under specific shapes (all verified to be capacity-reusing or high-water-only in the steady state):
- `EntityMaster::free_entity_ids` push on despawn (`entity_master.rs:378`), `DenseStore::free` (`dense_store.rs:450`), `EntitySlotMap::slots`, `LiveBitmap::words` (`live_bitmap.rs:53`) — grow only at a new high-water; **zero after warm-up in a stationary churn**.
- Observer arena / free list (`observers/entity_store.rs:190-194, 318, 357`) — only on attach/detach; zero unless a system attaches per frame.
- `Commands` per-system `CommandQueue` (`params/commands.rs:385-390`, capacity retained by `apply`) — zero after warm-up. **But** `deferred_hook_queue`'s twin drain (`command_queue.rs:491-527`) hands its capacity back *only if no re-entrant push happened*; a hook/observer that pushes during apply forces a fresh allocation **every frame**. Unverified candidate.
- `DescendantsIter::new` / `AncestorsIter::new` (`traverse_iter.rs:214, 295`) build a fresh `stack`/`VisitedSet` per call — one to two allocations per traversal per frame for any system that walks a relation per frame (`!ACYCLIC` seeds `insert_seen` at `:216-218`). Candidate.
- Schedule build structures — build-time only.

Three scenes with different shapes:

**S1 — Rigid-body pile** (driver: `jolt_parity_pyramid.rs`, two configs: default `add_physics_systems` + serial pool, and colored+parallel as the bench ships it). Exercises: N fixed steps × (1 + 7|8 cells) + broadphase/solver scope waves that scale with `passes × large colors` (bounded ≤ 3264 per step by gate #7, actual unknown). Does **not** exercise events, `Commands`, structural change, `par_iter` from user systems, UI. This is the *largest* plausible count and it is data-dependent.

**S2 — Spawn/despawn churn** (driver: demo `SimRunner` mode-switch every K frames with 4096-entity sets, or a synthetic `Commands`-based spawn/despawn schedule on `EcsMaster`). Exercises: transition pass, bulk `create_entity`/`delete_entity`, free-list high-water, `par_iter` fan-out (`n_chunks` cells per archetype per system per frame), states. Does **not** exercise the boyko_physics solver, events, render. Expect: a warm-up burst, then a *stationary* count dominated by `par_iter` chunk cells — the count is a function of `workers × systems`, not of entity count.

**S3 — Query-and-event loop** (driver: `phase12_events_systemparam.rs` shape scaled up — several `EventWriter`/`EventReader` systems + `Changed<T>` filters, run via `App::run_n_with_delta` so ③ `update_events` is in the window). Exercises: the App funnel, event swap/flatten, executor with no physics. Expect the *floor*: `1 + C_s` per schedule per frame plus whatever `swap_and_flatten` does (not read in this survey beyond `event_dispatcher.rs:594-616`; no `Vec` growth seen in the grep, unverified).

**Not available:** the shipped host frame (Main schedule from `EnginePlugins`, `plugins.rs:676-766`, ≥16 systems plus each plugin's own) without a device. Gate #1 covers its CPU helpers in isolation; nothing covers its `Schedule::run`.

### How Phase 2 could measure without a new instrument class

The pattern already exists and is sound for the executor: a **thread-local counter on the App/schedule-driving thread** (`sdf_edit_gather.rs:150-193`) sees every `ScopeShared` Box and every spawn cell, because both are allocated on the spawning (dispatcher) thread (`task.rs:80-82`, confirmed by `colored_parallel_alloc_o6.rs:135-139`). It is blind to allocations *inside* system bodies running on workers — which is exactly what gates #6-#12, #15-#20 already cover. Report per scene: warm-up frames' counts, then min/median/max over N steady frames, as counts only.

### Stale statements found on the way (not defects in code, but they will mislead a Phase 2 reader)

- `soft_body_sp1.rs:813-815` — names `exclusive_to_run`/`to_spawn` and `empty_intent` as `Schedule::run`'s per-step allocations; neither is (see row 11).
- `boyko_ui/tests/zero_alloc.rs:8-9` — "~7 per frame for a 2-system schedule" is undated and precedes the `executor_scratch` change (`schedule.rs:1048-1050` says the two `Vec::new()` per round were "the executor's only hot-path allocation" — which, given the `ScopeShared` Box and spawn cells, was also never true).
- `alloc_shim.rs:7` and the brief — "nineteen" gates; the tree has 20.

---

## Phase 2 - measurement (pre-Stage-3b tree)

**Phase 2 complete. Harness built, numbers taken, all green.**

Test binary: `D:/wt/ecsnative/crates/boyko_physics/tests/alloc_frame_census.rs` (new, untracked, nothing staged or committed).
Full release output saved at `D:/tmp/alloc_census_release.txt`.

```
cargo test -p boyko-physics --release --test alloc_frame_census -- --nocapture
```
Release: 1 passed, 18.7 s. Dev profile also passes (36 s, scenes scaled down). `cargo clippy -p boyko-physics --test alloc_frame_census -- -D warnings` clean; `cargo check --workspace --all-targets` exit 0, 0 errors.

## Where it lives, and why not where the brief said

Neither `boyko_ecs/tests/` nor `boyko_app/tests/` can reach `boyko_physics` (`boyko_app` does not depend on it; `boyko_ecs` cannot). `boyko_physics`'s dev graph is the only one spanning both the physics pile and the ECS kernel/`App`, so the binary sits there. Header states this.

**One `#[test]`, not eight.** `libtest` runs `#[test]` fns concurrently by default and the counter is process-global — two scenes at once would each measure the other. A `--test-threads=1` note is skippable; a single entry point cannot be misconfigured.

## THE ANSWER — heap acquisitions (alloc + realloc) per steady-state frame

| scene | K | mean | min | MAX | realloc max | bytes/frame | setup |
|---|---|---|---|---|---|---|---|
| S0 executor floor, 0 systems (**control**) | 0 | 0.000 | 0 | 0 | 0 | 0 | 65 |
| S0, 1 system | 0 | 5.016 | 5 | 6 | 0 | 1 752 | 107 |
| S0, 2 | 0 | 6.031 | 6 | 7 | 0 | 1 824 | 112 |
| S0, 4 | 0 | 8.062 | 8 | 9 | 0 | 1 967 | 124 |
| S0, 8 | 0 | 12.125 | 12 | 13 | 0 | 2 254 | 151 |
| S0, 16 | 0 | 20.254 | 20 | 21 | 0 | 2 834 | 202 |
| S0b, 4 Main + 4 Fixed, 1 substep/frame | 0 | 16.125 | 16 | 17 | 0 | 3 934 | 185 |
| S2 spawn/despawn churn + par_iter | 0 | 14.098 | 14 | 16 | 1 | 5 737 | 182 |
| S3 query + event loop | 0 | 8.066 | 8 | 9 | 0 | 1 976 | 145 |
| S1a rigid pile, DEFAULT pipeline, serial | 0 | 11.109 | 11 | 12 | 0 | 2 182 | 185 |
| S1b rigid pile, COLORED, parallel OFF | 0 | 12.125 | 12 | 13 | 0 | 2 254 | 228 |
| **S1c rigid pile, COLORED + parallel ON** | **63** | **2670.98** | **2601** | **2724** | 0 | **856 141** | 3 061 |

## Six findings

1. **The whole cost is dispatch, and it is a closed form: `n + 4` per `Schedule::run`, `0` when `n = 0`.** The empty schedule returns at its `systems.is_empty()` guard before `pool.install`. S0b measured `2 × (4+4) = 16`, which pins the constant as per-`Schedule::run`, not per frame. This settles the survey's open item: the O4 gate's "~2 per extra system in release" is wrong — it is **exactly 1 cell per concurrent system**, over a fixed 4.

2. **Steps ⓪–③ of `App::update_with_delta` cost nothing.** Profiling fold, `Time::advance_with`, check-ticks, `update_events` — zero. S3 sends and reads 32 events/frame, mutates 4096 rows through `Mut<T>` and runs a `Changed<T>` query, and still lands on `4 + 4 = 8`, the same figure as four systems that do nothing.

3. **1240 bodies of real physics add ZERO.** S1a = 11.109 = `7 + 4`. S1b (colored, 8 systems, parallel off) = 12.125 = `8 + 4`, and **byte-for-byte the same 2254 B/frame as S0's eight no-op systems**. The arena's premise holds: every buffer in the step is capacity-reused.

4. **The parallel dispatch is the entire allocation budget.** S1c differs from S1b by two config flags and measures **220×** — 2671 mean / 2724 max, 856 KB/step, ~2.15 acquisitions per contact. One heap cell per spawned task, exactly what `block.rs`'s own header says it was written to remove and is "wired to NOTHING". It sits inside the 3264/step bound `colored_parallel_alloc_o6.rs` already asserts — that gate was right, and nobody had recorded the number under it. **Note `parallel_broadphase` is inert here** (floor is 4096 bodies), so 2671 is the colored solve's `pool.scope` waves alone.

5. **`realloc` is ~0 everywhere.** One exception: the churn scene at 0.008/frame (one frame in 128, max 1). Vec growth is not a steady-state cost in any scene — the field census's `Vec`-typed kernel fields are all high-water-only, as suspected.

6. **The MAX is `mean + 1`, and the +1 is attributed.** Every scene shows a periodic extra acquisition of **exactly 1520 bytes, once per 64 spawned tasks**. `1520 = 8 + 63 × 24` — a `crossbeam_deque::Injector` block (`next` pointer + `BLOCK_CAP = 63` slots of `{Task(16), state(8)}`). One block per injector lap. That is the only hitch in the census, and it is bounded at +1.

## Anti-vacuity, and two places it caught the harness itself

- **Counter live**: all four axes proven (`alloc/realloc/dealloc/bytes`), plus a **process-global proof** — a system body allocating 1 MiB *on a worker thread* is counted (48 allocs / 8 MiB over 8 frames, off-thread confirmed). This is the property the tree's twenty thread-local gates lack.
- **Per-scene in-window probe**: every scene ends with a frame that makes a deliberate 1 MiB allocation inside the open window.
- **Scene doing something**: S0 asserts exactly `n` systems ran exactly once per frame (distinct `fn` items, each with its own counter — an array of fn pointers would be one type and could silently collapse to one system); S1 asserts >0 contacts (4611/4882) and that sampled bodies moved (3.5 m); S2 asserts 20 608 spawns / 20 608 despawns with population flat at 4160; S3 asserts 10 304 sent = 10 304 received and 1 318 912 `Changed` matches.
- **S1c engagement is asserted against a control arm, not a flag.** `parallel_solve = true` is a request, not an event. S1b/S1c differ only in those two flags; the run asserts S1c > S1b. In the dev-profile scene at pyramid height 6 the parallel path did **not** engage and S1c measured 13, indistinguishable from the serial arm — caught, and fixed by raising the debug scene to height 10 (now 926 vs 13, engaged).
- **The liveness probe was wrong twice and both failures are recorded in the file.** (a) `live.alloc > control.alloc` failed at `control=15, live=15` — the control frame happened to carry the periodic +1 injector block, so scene jitter exactly cancelled the probe. Fixed with a nested snapshot pair around the deliberate allocation, still inside the open window. (b) the window-total byte check `live ≥ control + 1 MiB` then failed at `control=5584, live=1052640` — the control had caught the churn scene's 262 KB spike. Fixed by flooring on the steady window's measured **minimum**. A liveness check a scene's own noise can cancel is not a liveness check.

## What is still unmeasured

- **The shipped host Main schedule** (`EnginePlugins`, ≥16 systems). `plugins.rs` has only a `window(...)` constructor — no headless variant — so it cannot run without a device. By finding 1 its floor is `n + 4`; its system bodies are not covered.
- **Draw-list assembly, swapchain, windowed loop** — device-needing.
- **S1's frame is one fixed step driven by `Schedule::run` directly**, because the physics `add_*` fns need `&mut ScheduleBuilder` and `&mut EcsMaster` simultaneously and `App` exposes only the builder. The App funnel around it is priced separately by S0/S0b, and that funnel is zero.
- **S1c's figure is data-dependent**: warm-up settled only at `K = 63` (one frame inside the 64-frame budget) and the steady window spreads over 25 distinct values as the pile's contact count drifts. Quote as a range, never as a figure.

No gate written, as instructed. No wall-clock number produced anywhere in the binary.

---

## Phase 3 - attribution and gate (KE16 tree)

{
 "coverage_map": "Tree: D:/wt/joltab, branch merge/ke16-into-ecsnative. The harness was measured at ca582e72. HEAD moved to d11962a9 at 04:24 today (a commit I did not make, authored Celtokisa). It committed the two diagnostic edits I was told to leave alone (the bench-alloc feature and the jolt_parity_pyramid global_allocator) plus docs/OPEN-QUESTIONS.md. None of that is on any path these binaries run, so the measurements hold on d11962a9 as well. Both headers now say so.\n\nWhere the work started: a previous run of this phase had already ported the harness into joltab and had nearly finished (outputs in D:/tmp/*joltab_FINAL*, 00:06-00:10), but never reported. I did not take its output on trust. I read both files in full, re-ran everything, checked the Commands id-leak claim against the ECS source, found and fixed one flaky assertion in the gate, named one of the two unexplained OTHER objects, and ran a mutation to show the gate can still go red. Nothing in ecsnative was touched.\n\nMEASURED, over the whole frame. One counting #[global_allocator], process-global. A system body allocating on a worker thread is proven to be counted: 8 MiB over 8 frames, confirmed off the driving thread.\n- The App frame: fold, Time, check-ticks, update_events and Main Schedule::run, with 0/1/2/4/8/16 systems (S0).\n- The same plus a Fixed schedule at one substep per frame (S0b).\n- Commands spawn/despawn churn plus a par_iter fan-out (S2).\n- The event lane with the EveryFrame swap, 4096 Mut<T> writes and a Changed<T> query (S3).\n- The 1240-body Jolt pyramid through the real physics Schedule::run: default pipeline serial (S1a), colored serial (S1b), colored with parallel_solve and parallel_broadphase at W=4 (S1c).\n\nFor each scene: three regimes (setup / warm-up with K derived / 256 steady frames), counted separately for alloc, realloc, dealloc and bytes. Every acquisition is also split by layout class (scope / chunk / injector / OTHER) inside the allocator, without allocating.\n\nNOT MEASURED:\n- The shipped host EnginePlugins Main schedule. There is no headless constructor for it. By the result below its dispatch floor is 2 acquisitions per Schedule::run, but its system bodies are not covered.\n- Draw-list assembly, swapchain and the windowed loop (they need a device).\n- Physics through App. It is driven by Schedule::run directly; S0/S0b price the App funnel around it, and that funnel adds nothing.\n- parallel_broadphase at or above its 4096-body floor. It is inert on this scene: +0.000 measured.\n- Sleeping ON, observers and hooks (not exercised).",
 "scenes": "Heap acquisitions (alloc + realloc) per steady-state frame, shown as mean / MAX over 256 frames, release.\n- BEFORE = ecsnative @ ad0ebea4 (the pool before KE16 Stage 3b; the phase-2 numbers).\n- AFTER = joltab @ ca582e72 / d11962a9 (the shipped KE16 pool).\n- AFTER is identical to three decimals across 9 release runs, except that a per-thread first-touch OTHER can land in an App scene's window and add 1-3 to its MAX. The value in brackets is the worst run.\n\n| scene | BEFORE | AFTER |\n|---|---|---|\n| S0, 0 systems (control) | 0 / 0 | 0 / 0 |\n| S0, 1 system | 5.016 / 6 | 2.016 / 3 |\n| S0, 2 systems | 6.031 / 7 | 2.031 / 3 (6) |\n| S0, 4 systems | 8.062 / 9 | 2.066 / 3 (4) |\n| S0, 8 systems | 12.125 / 13 | 2.125 / 3 |\n| S0, 16 systems | 20.254 / 21 | 2.254 / 3 |\n| S0b, 4 Main + 4 Fixed | 16.125 / 17 | 4.125 / 5 (6) |\n| S2, churn + par_iter | 14.098 / 16 | 4.039 / 5 |\n| S3, query + events | 8.066 / 9 | 2.062 / 3 (4) |\n| S1a, pile, default, serial | 11.109 / 12 | 2.109 / 3 |\n| S1b, pile, colored, parallel off | 12.125 / 13 | 2.125 / 3 |\n| S1c, pile, colored, parallel on, W=4 | 2670.98 / 2724 (range 2601..2724) | 331.797 / 339 (range 326..339; 1.26 MB per step; identical in all 9 runs) |\n\nWhere the difference comes from:\n- **Per scope frame** (a pool.install or a nested pool.scope):\n  - BEFORE: 4 plus one heap cell per spawn. The 4 are one Box<ScopeShared> and three allocations for a scratch crossbeam Worker deque built on every Scope::drop.\n  - AFTER: 1, plus one 4 KiB ScopeBlock chunk if the scope spawns anything, plus a doubling chunk each time the cells overflow. The scratch deque is gone: join_on_worker / join_external build none.\n  - So an App frame went from n + 4 to a flat 2, whatever n is.\n- **S1c is 8.05x lower** for the same two reasons over 121 scope frames a step, plus a third: a push made by a worker now lands in that worker's own Chase-Lev ring (place_task -> push_on_lane_no_wake) and allocates nothing. Before, it went through the injector and cost a 1520 B block per 63 pushes: 38 blocks a traced step before, 0.125 after.\n- **Unchanged between the trees:** the realloc from the Commands id-list growth (the pool is not involved), and zero from every physics buffer.\n- **Warm-up:** S1c settles at K=63 (warm-up spans 290..387 as the pile collapses). Every other scene settles at K between 0 and 62; K is reported, never asserted.",
 "table": "Steady-state acquisitions per frame on the KE16 tree, split by class. Each acquisition is classed from its Layout inside the allocator, so the frame is fully accounted for rather than sampled.\n- **scope** = one Box<ScopeShared> (256 B, align 128).\n- **chunk** = one ScopeBlock chunk (4 KiB, doubling, align 64).\n- **inj** = one crossbeam Injector block (1520 B).\n- **OTHER** = everything else.\n\n| scene | scope | chunk | inj (mean / max) | OTHER | realloc |\n|---|---|---|---|---|---|\n| S0, n = 1..16 systems | 1 exact | 1 exact | n/63 / 1 | 0 (a first-touch object can land) | 0 |\n| S0b | 2 exact | 2 exact | 8/63 / 1 | 0 (first touch) | 0 |\n| S2 | 2 exact | 2 exact | 0.031 / 1 | 0 | 2 in the 256-frame window |\n| S3 | 1 exact | 1 exact | 0.062 / 1 | 0 (first touch) | 0 |\n| S1a / S1b | 1 exact | 1 exact | 0.109-0.125 / 1 | 0 in release (S1b debug: 1 per step, debug_assert_coloring) | 0 |\n| S1c | 121 exact, every frame | 205..217, mean 210.7 (1.74 per scope) | 0.125 / 1 | 0 | 0 |\n\n**Per stage.** Traced with BOYKO_ALLOC_TRACE=1. Each capture is charged to the innermost FunctionSystem on its stack, because Schedule::run exposes no per-stage seam.\n- executor, Schedule::run: 1 scope + 1 chunk per run (the install frame).\n- physics_solve_colored: all the rest of S1c. That is 120 scopes (12 colour passes x 10 dispatched colours) and ~209.7 chunks, 99.5% of a traced step.\n- physics_build_graph: 0 in release. In debug, 1 debug_assert_coloring Vec per step.\n- Every other physics system, the event lane, change detection and every system body: 0.\n- Eight instrumented, conflict-serialised system bodies measured 0 over 160 frames. Their frame total (2.125) equals the plain-systems control.\n\n**Differential deltas** (section C: a 4-system App at W=2, baseline 2.070):\n- events: -0.008\n- change detection: -0.008\n- one par_iter fan-out: +1.992 (exactly +1 scope and +1 chunk)\n- Commands 64+64 per frame: +0.016 (+1 realloc max)\n\n**Physics differentials** (section D, one warmed pile, A/B/A):\n- parallel_solve ON: +328.2 per step over OFF (2.125 -> 330.4). OFF-window drift was 0.000.\n- parallel_broadphase alone: +0.000.\n- Colour-pass sweep, substeps x (1 + relax) = 12/4/2/1 passes: 335 / 113 / 57 / 32 per step, a flat 26.6..31.6 per pass.\n- Worker count 1/2/4: 254 / 254 / 346. The chunk count follows the number of spawns (lanes x 6, capped by work) against the 4 KiB chunk.\n\n**Dispatch primitives** (section A, predicted from the pool's published layout constants before comparing):\n- An empty install is exactly 1 scope.\n- An install with one spawn is 1 scope + 1 chunk.\n- Chunks are exactly 1 / 2 / 3 at 256 / 257 / 1024 spawns.\n- The cost does not change with pool width 1/2/4/8.",
 "attribution": "Five methods, each named at its site in D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_attribution.rs:\n0. Layout classes, applied inside the allocator.\n1. Direct pricing of the pool primitives (install / scope / spawn / spawn_batch), with no ECS involved.\n2. Per-system windows. Eight systems wrapped and serialised by a shared ResMut. This changes behaviour (they no longer run concurrently); the frame total is compared against plain systems and the two are equal.\n3. Differential toggles: event lane, change detection, par_iter, Commands; and on one warmed pile, parallel_solve, parallel_broadphase, a substeps/relax sweep and a worker sweep.\n4. Backtrace capture under BOYKO_ALLOC_TRACE=1, with ALL / realloc-only / OTHER-only / non-epoch-Local filters. It is diagnostic only and never armed for an assertion.\n5. NEW: section G. An allocation-free log of (frame, size, align) for every OTHER acquisition, so rare objects can be named without a backtrace changing the timing they depend on.\n\nEvery steady-state site, and what it is:\n\n**1. Box<ScopeShared>: should not allocate.**\n- One per Schedule::run install frame, and one per nested pool.scope (each par_iter fan-out; each dispatched colour in each solver pass).\n- Freed at the join and never reused.\n- A pool-owned free list of scope frames takes it to zero. The colored solver already names this as a filed follow-up (\"zero-alloc reusable-scope threadpool API\").\n\n**2. ScopeBlock chunk: should not allocate.**\n- One per spawning scope, 4 KiB, doubling on overflow.\n- Freed at every join (free_all) and allocated fresh by the next scope on the same thread.\n- Keeping the high-water chunk per joiner and resetting the cursor would take it to zero.\n- Items 1 and 2 are the entire App-frame cost (2) and the entire parallel physics cost (~330 per step). An App frame would go 2 -> 0.\n\n**3. crossbeam Injector block: transient, inside the queue.**\n- One per 63 pushes from outside the pool.\n- It is the whole of every scene's periodic MAX = mean + 1.\n- Not this engine's to remove short of replacing the queue.\n\n**4. First touch per pool thread (OTHER): a setup cost that lands in frames.** Two objects, both captured by stack:\n- crossbeam-epoch's Local, 2304 B, from Collector::register on a thread's first steal.\n- std's 30-byte UTF-16 copy of the worker name \"boyko-worker-N\", made for SetThreadDescription. The trace shows std::sys::pal::windows::to_u16s::inner under Thread::new::thread_start. It appears when a worker finishes BOOTING after setup, because ThreadPoolBuilder::build does not wait for its threads to start.\n\nSection G, over 832 fresh 2-worker Apps across 8 runs:\n- 660 of 1056 Locals landed at frame 64 or later, i.e. INSIDE the steady window. The gate's OTHER allowance is therefore load-bearing, not decorative.\n- Never more than 2 Locals per App, never 2 OTHER in one frame.\n- The thread-name object appeared in 13 Apps, always before frame 64.\n- Section F asserts the second half of a 4096-frame run carries none, so this is not a rate.\n- Pre-registering the threads with the epoch collector and waiting for boot in build() would move both objects into setup.\n\n**5. EntityMaster::free_entity_ids growth: Vec growth, and it is UNBOUNDED. A defect, not a cost.** Checked against the source:\n- Commands::spawn calls EntityCounter::reserve_entity, which is a fetch_add on next_entity_id and never pops the free list (EM2 forbids a worker to pop it).\n- DespawnCommand -> EcsMaster::delete_entity pushes onto the free list.\n- Only the dispatcher's allocate_entity pops it (entity_master.rs:124-148, 186).\n- Measured over 2048 frames at a flat population of 4160: free list +131072 and id-slot store +131072 against 131072 despawns. Not one id was reused.\n- Control: the same churn through create_entity / delete_entity recycles perfectly (32704 despawns leave the slot store at 64).\n- A column instead of a Vec would remove the realloc but NOT the growth. The fix is to recycle on the deferred route, since CommandQueue::apply runs on the dispatcher.\n\n**6. Free (measured zero):** the event lane, change detection, query iteration, and every physics buffer. The arena premise holds everywhere except the dispatch objects.\n\n**7. Before-tree only:** the scratch Worker<Task> deque, 3 per scope frame. Gone on the KE16 pool.",
 "gate": "Gate: D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_census.rs, fn gate(), inside one #[test] frame_allocation_census. It is a single test on purpose: libtest would otherwise run tests concurrently against a process-global counter. There is no #[ignore], and it runs in both profiles, with a separate pin set for debug.\n\nIt pins each scene's measured envelope per CLASS rather than as one total:\n- scope range and chunk range, checked on every frame;\n- the dispatch MAX;\n- injector blocks, at most 1 per frame;\n- OTHER summed over the window;\n- reallocs summed over the window;\n- plus a total MAX.\n\n**Pins:**\n- **App scenes (S0 n = 1..16, S3, S1a, S1b):** scope = chunk = 1 exactly, dispatch MAX 3.\n  - Headroom zero: one install frame, one chunk and at most one injector block is structural.\n- **S0b:** 2 / 2 / 5, also zero headroom.\n- **S0, 0 systems:** 0 across the board.\n- **S2:** scope 2, chunk 2, dispatch MAX 6 against a measured 5.\n  - Headroom +1, because the injector block and the free-list doubling are independent periodic events that have never yet coincided.\n  - realloc_sum pinned at EXACTLY 2 with zero headroom: the 8192 and 16384 doublings.\n  - Deterministic but one frame from the edge: the 4096 -> 8192 doubling fires in driven frame 63, the last warm-up frame. The rule is written at the pin: re-derive it from the frame arithmetic, never widen it.\n- **S1c release:** scope 109..=133 (measured 121), chunk 181..=241 (205..217), dispatch MAX 375 (339).\n  - Headroom = one dispatched colour either way (12 scopes, 24 chunks, 36 acquisitions), because the count moves in whole colours as the contact set drifts.\n- **S1c debug:** a height-10 pile, 85..=97 scopes, MAX 195 + 36.\n- **OTHER budget:** 2 x workers per window (plus 1 per step in debug for the colored pipeline).\n  - Two per thread = the epoch Local + the boot-time thread-name object.\n  - Worst measured: 3 of 4 on S0 n=2.\n\n**Positive controls, so no class can come back green from an emptied bucket:**\n- The chunk constants are asserted against boyko_threadpool::__layout_receipt.\n- Scope, chunk and realloc must be exact on each of 64 empty installs and 64 one-spawn installs.\n- The 16-system scene must show injector blocks.\n- Every S1 frame must satisfy (scope - 1) % passes == 0.\n- S1c must exceed S1b, or the parallel dispatch never engaged.\n- Scene invariants: contacts > 0 and bodies moved; exact spawn/despawn counts at a flat population; events sent equal events received; exactly n systems ran once per frame.\n\n**Flake found and fixed.** The first release run today went RED in class_predicates_match_the_threadpool_receipt with (scope, chunk, OTHER) = (1, 1, 1).\n- Cause: a worker's first steal (its epoch Local) landed on the single priced frame, after only 64 warm-up installs. Section G shows that is the common case on a fresh pool.\n- The control now warms with 400 installs x 8 spawns, prices 64 frames of each primitive, holds scope/chunk/realloc exact on every frame, and bounds OTHER over both windows by the thread count.\n- Green on every run since.\n\n**Mutation proof, re-run on the final file.** One Box<u64> per frame, in probe0 only:\n- 6 violations: every S0 row with n >= 1 plus S0b, 256-257 OTHER against a budget of 4.\n- Every MAX pin stayed green. That is exactly why the pins are per class and not one total.\n- Mutation reverted; the file was checked identical to its backup.\n\nMeasured numbers, date and tree are in both file headers, including the BEFORE/AFTER table.",
 "receipts": [
  "Files (both UNTRACKED in D:/wt/joltab, nothing staged, nothing committed): D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_census.rs (the census and gate; sha256 eea34f14...) and D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_attribution.rs (the attribution; sha256 fed942c3...). ecsnative's copies were not touched.",
  "Env for every call: PATH=$HOME/.cargo/bin:$PATH RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu (rustc 1.98.1 48a229cea) CARGO_TARGET_DIR=D:/wt/_targets/allocgate, --config profile.dev.debug=0, RUSTFLAGS never set.",
  "Final green run on the final bytes: cargo test -p boyko-physics --release --test alloc_frame_census --test alloc_frame_attribution -- --nocapture -> both ok, GATE: GREEN (12 scenes). Log: D:/tmp/alloc_joltab_resume_FINAL3_release.txt",
  "Debug (dev) profile on the final gate: D:/tmp/alloc_joltab_resume_FINAL2_dev.txt -> both ok, GATE: GREEN (debug pins)",
  "Release census runs today: rel_1, rel_3, rel_4, FINAL, FINAL2, FINAL3 all green. rel_2 went red in the old single-frame class control, which is now fixed. Logs: D:/tmp/alloc_census_joltab_resume_rel_{1,2,3,4}.txt",
  "Mutation (one Box per frame in probe0) -> red, 6 OTHER violations, MAX pins green: D:/tmp/alloc_census_joltab_resume_mutation.txt",
  "Trace mode that captured the 30-B thread-name object (FILTER_RARE, release): BOYKO_ALLOC_TRACE=1 cargo test -p boyko-physics --release --test alloc_frame_attribution -- --nocapture -> D:/tmp/alloc_attr_joltab_resume_trace_rel.txt. Debug trace runs: D:/tmp/alloc_attr_joltab_resume_trace_dev{,2}.txt",
  "Section G logs: D:/tmp/alloc_attr_joltab_resume_rel_{1,g1,g2,3}.txt, D:/tmp/alloc_attr_joltab_resume_dev_3.txt, plus the two trace logs",
  "cargo clippy -p boyko-physics --all-targets -- -D warnings: exit 0. Both test targets forced to re-check (touch plus -v shows two clippy-driver invocations), no warnings.",
  "cargo check --workspace --all-targets: exit 0, 0 warnings, 0 errors (D:/tmp/alloc_joltab_resume_check.txt)",
  "BEFORE numbers: the phase-2 run on D:/wt/ecsnative @ ad0ebea4, D:/tmp/alloc_census_release.txt (produced earlier, not re-run: the brief's rule is not to rebuild there)",
  "No wall-clock figure is produced or quoted anywhere. Counts only."
 ],
 "open": [
  "DEFECT, outside this phase's scope, for the orchestrator or owner: the Commands id leak. Commands::spawn never recycles ids, so EntityMaster::free_entity_ids and the entities_inland slot store grow by one entry per deferred despawn, forever, on a flat population (+131072 in 2048 frames at 64 churn a frame). The fix is to recycle on the deferred route in CommandQueue::apply (dispatcher-side, so EM2 allows it). Replacing the Vec with a column would NOT fix it. The S2 realloc pin (exactly 2) will need re-deriving when it lands.",
  "The two remaining per-frame dispatch objects are both in boyko_threadpool: Box<ScopeShared> (1 per scope frame) and a ScopeBlock chunk freed at every join. A pool-owned scope-frame free list plus a per-joiner high-water chunk would take an App frame from 2 to 0 and the parallel pile step from ~330 to ~0. It is a threadpool design change, not attempted here. The gate will go red on it by design and must then be re-pinned downward with the new numbers.",
  "One OTHER object is still unnamed. Once today, the S0 n=2 window carried one 6-acquisition frame with 5016 B of OTHER = two 2304 B epoch Locals plus 408 B. The 408 B object is not the 30 B thread-name object, and 832 fresh Apps under the layout log never produced a two-OTHER frame. It is inside the allowance (3 of 4) and is not a rate (section F), but it is unidentified. Recorded under 'Open' in the census header.",
  "The first-touch objects could be moved into setup: ThreadPoolBuilder::build could wait for its workers to boot and pre-register each with the epoch collector. That would let every App-scene OTHER budget drop to 0.",
  "Unmeasured: the EnginePlugins host Main schedule (no headless constructor), device-side frame work, physics through App rather than Schedule::run directly, parallel_broadphase at or above 4096 bodies, sleeping ON, observers and hooks.",
  "The joltab HEAD moved under this run: d11962a9 (Celtokisa, 04:24) committed the two bench diagnostic edits that were uncommitted at the start, plus docs/OPEN-QUESTIONS.md. Both headers record that measurements from ca582e72 hold on d11962a9. The two new test files remain untracked and nothing is committed, per the standing rules.",
  "The census reuses the 20-gate survey's diagnosis but does not change any of the existing twenty zero-in-my-window gates. Some of them (the O4 world gate, boyko_ui/tests/zero_alloc.rs) still subtract a baseline that is now known to be exactly 2 per Schedule::run on this pool."
 ]
}

---

## Phase 4 - adjudication

**Verdict: the census is sound and the gate is real, with two defects in the gate and three corrections to the report.** Nothing in the tree was left changed. Every scratch mutation was restored and checked byte-identical with `cmp`. The two harness files still have their stated hashes (census `eea34f14…`, attribution `fed942c3…`), both are untracked, and nothing is staged or committed.

**Is the counter live?** Yes, re-run on the final bytes. All four axes move (alloc 1, realloc 1, dealloc 1, 65,600 realloc bytes). A system body running off the driving thread allocated 8 MiB over 8 frames and all of it was counted. Every scene's 1 MiB probe was seen, and the layout-class control is exact on 64 + 64 installs. Release and debug both GREEN (`D:/tmp/adj_census_rel_1.txt`, `D:/tmp/adj_restored_release_both_2.txt`, `D:/tmp/adj_census_dev.txt`).

**Is each scene doing work?** Yes. The end-of-scene checks alone do not prove the steady window worked, so I ran 4,096 more steps per physics arm (`D:/tmp/adj_census_M1_series.txt`):
- Body positions changed bit-for-bit on every step (0 unchanged steps) and contacts were above zero on every step (4,481..4,911). The low 2.1 per step is a pile being solved, not an idle schedule.
- The App scenes pass their own checks: churn 20,608 spawns / 20,608 despawns, events 10,304 sent / 10,304 received, 1,318,912 Changed matches, and every probe system ran once per frame.

**Does the steady window exclude warm-up?** Yes for the window itself, but S1c's steady state keeps moving after it:
- Per-frame S1c: frames 0..18 are 290 (8 colours), 19..63 swing 314..387 while the pile collapses, and frames 64..319 are exactly 121 scopes every frame. The App scenes' two half-window means match each other.
- The longer run shows S1c still drifting down. Per-256-step block means go 331.5, 326.6, 314.8 … 307.5, the colour count mixes 9 and 10 (scopes 109..121, chunks 193..217), and contacts fall from 4,911 to 4,774.
- **Correction 1:** "331.797, identical in all 9 runs" is repeatable only because the window sits at the same point; it is not the steady state. Quote S1c as **302..339 per step, block means ~307..332**. The pins still hold over the long run: MAX stayed at 339, nothing went above 375, and the lower one-colour headroom was actually used.

**Is the MAX reported?** Yes: per-scene MAX, per-class MAX and a histogram.

**Does the attribution add up?** Yes, with no unexplained remainder in steady state.
- The four classes split every fresh allocation by construction. S1c = 121 scope + 210.67 chunk + 0.125 injector + 0 OTHER = 331.797.
- All 2 S2 reallocs trace to `EcsMaster::delete_entity` under `CommandQueue::apply`.
- The id leak matches the source: `reserve_entity` is a `fetch_add` and never pops the free list, and the free list and slot store each grew +131,072 against 131,072 despawns.
- The only unexplained item is the one 408 B object already listed as open.
- **Correction 2:** the traced physics step behind "99.5 % `physics_solve_colored`" is step ~50, which is still warm-up: 145 scopes and 386 allocations, 12 colours. The report pairs its percentage with the steady figures (120 scopes, 209.7 chunks). The shape is the same in both states; the numbers come from different ones.
- **Correction 3 (a coverage boundary):** the counter only sees the Rust heap. `VmReservation::commit` calls `VirtualAlloc(MEM_COMMIT)` directly, so arena growth is not counted. For example, the leak's slot store (16 B slots, 256 KiB minimum slab, doubling) commits pages without being seen. "0 from every physics buffer" means zero heap allocations, not zero memory growth.

**Does the gate catch a real regression?** It reds on the two usual shapes of per-frame allocation, and misses one kind of dispatch regression:

| mutation | result |
|---|---|
| a fresh `Vec` plus one `push` per step in `physics_apply` | **RED**: S1a, S1b and S1c each had 256 OTHER against a budget of 2 or 8. Every MAX pin stayed green, so the per-class pins are what caught it. |
| the same `Vec::push` per `Schedule::run` | **RED** in all 12 scenes |
| a `Local<Vec>` pushed every step and never cleared (a leak) | **RED**: 2 reallocs against a pin of 0 in every physics arm |
| one extra fan-out per colour pass in the colored solver | **GREEN**, which is a gate defect: +24 per step (331.8 → 355.8, MAX 363). 133 scopes falls inside the 109..=133 pin and still passes the "multiple of 12" check. The upward one-colour headroom was never seen in 4,352 steady steps. |

The fix for the last row: drop the upward headroom (scope ≤ 121, chunk ≤ 217, MAX ≤ 339), or pin scopes exactly against the solver's own count of dispatched colours.

**Other findings:**
- **A trap in the restore method.** `cp -p` keeps the old modification time, so cargo kept the mutated `boyko_ecs` library after a byte-identical restore. The attribution binary then went red (executor residual 1.000) on correct source. I ran `touch` on the four restored files to force a rebuild (their modification times are now new, content unchanged); after that both binaries are green. Future mutation proofs should restore with plain `cp`, or `cp` followed by `touch`.
- **A latent flake.** libtest's "running for over 60 seconds" notice allocates on the main thread and is counted: 2 OTHER in one frame in my long run. The census takes 17–24 s in release and 34 s in debug, so it is under the line today but could cross it on a loaded machine. Setting `harness = false` for these two test targets removes it.
- **Something that should not allocate at all.** With parallel_solve on a 1-worker pool the solver still dispatches: 127 scopes + 127 chunks = 254 allocations per step for no parallelism.
- **Side-by-side check holds.** Both trees' numbers are in the census header per scene. The scene code on the two trees is identical apart from formatting and the added per-frame check, so BEFORE vs AFTER compares the same scenes. The gate is on joltab but uncommitted, so it does not travel with the branch until someone commits it.

**Answer to the owner.** On the shipped KE16 pool (joltab), a frame makes **2 heap allocations**, no matter how many systems it runs: one 256-byte scope frame and one 4 KiB task chunk for `Schedule::run`. Every 63rd outside push adds a third, a crossbeam queue block. Each extra `Schedule::run` (a Fixed substep) or `par_iter` fan-out adds 2 more. A physics step is the same 2 when serial, and **302–339 per step (~1.2 MB)** on the 1,240-body pile with parallel_solve at 4 workers, because the colored solver opens one scope with ~1.7 chunks for every dispatched colour in each of its 12 passes. Everything else measured zero: events, change detection, queries, system bodies and every physics buffer, so the arena premise holds. The pool from before KE16 Stage 3b made n+4 per frame and ~2,670 per physics step. Four things should not be there:
1. The scope frame and the task chunk. Both are freed at the join and allocated again by the next scope; a pool-owned free list of scope frames plus a cached chunk per joiner would take a frame to 0 and the parallel step to about 0.
2. The unbounded `Commands` id leak: deferred despawns push ids that deferred spawns never reuse. Recycle them in `CommandQueue::apply`.
3. Parallel dispatch on a 1-worker pool.
4. The per-thread first-touch objects that land in early frames. `ThreadPoolBuilder::build` should wait for its threads to start and register them with the epoch collector.

Logs: `D:/tmp/adj_census_rel_1.txt`, `adj_census_M1_series.txt`, `adj_census_M2a_physics_apply_vec.txt`, `adj_census_M2b_physics_apply_leak.txt`, `adj_census_M2c_extra_fanout_per_pass.txt`, `adj_census_M2d_schedule_run_vec.txt`, `adj_restored_release_both.txt` (the stale red), `adj_restored_release_both_2.txt`, `adj_census_dev.txt`.
