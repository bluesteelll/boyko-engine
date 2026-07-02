> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 20 — Fixed Timestep + Multi-Schedule: Research

Pre-design research for boyko-engine "direction #2". Branch `ecs`. All local file references verified against the working tree on 2026-06-11; all external references fetched from primary sources (Bevy `main` branch ≈ 0.18-dev, flecs `master`, Unity Entities 1.0 docs, Godot stable docs).

## Brief summary (TL;DR)

- **Bevy** stores the accumulator *in a resource* (`Time<Fixed>.overstep`), clamps dt *upstream* in `Time<Virtual>` (`max_delta` = 250 ms), and has **no step cap** in the fixed loop itself — the clamp implies a ceiling of 16 steps at the default 64 Hz. The fixed loop is a *system* (`run_fixed_main_schedule` in the `RunFixedMainLoop` schedule) that swaps the generic `Time` resource to fixed values for the duration of each `FixedMain` run.
- **flecs is the opposite model**: fixed rate is *per system* (`.interval(t)` timers on system entities), fires **at most once per frame**, and *discards* backlog > 2× interval — throttling, not catch-up. Unity DOTS is a per-group catch-up loop behind a swappable `IRateManager`.
- **boyko already has 80% of the per-run machinery**: `Schedule::run` is self-contained per invocation (frame-start tick bump + apply-window bump + state pass + apply windows), and the demo *already* calls it 0..=5 times per display frame from a hand-rolled Fiedler accumulator (`crates/boyko_demo/src/sim/runner.rs`). What does not exist: a `Time` resource, multi-schedule storage in `App`, event-update gating for fixed steps, and an interpolation alpha.
- **Events are the sharpest gap**: Bevy explicitly gates its per-frame buffer swap on "the fixed loop ran at least once" (`ShouldUpdateMessages` Ready/Waiting state machine) so `FixedUpdate` readers never miss a buffer generation. boyko's `update_events()` is app-driven with no such gate.
- **Bevy's label dispatch is a `HashMap<InternedScheduleLabel, Schedule>` with remove-run-reinsert ("hokey-pokey") per schedule run** — pointer-hashed, cheap, but it is exactly the structure boyko's house rules forbid on the hot path; the zero-overhead alternative is enum/const-indexed schedule slots.

---

## What boyko already has (anchor)

| Fact | Evidence |
|---|---|
| `App` drives exactly ONE schedule | `crates/boyko_ecs/src/ecs/core/app/app.rs` — field `schedule: Option<Schedule>` (line 57); `update()`/`run_n()`/`run()` each reduce to `schedule.run(world)` once per frame (lines 287–351); startup = `Vec<Box<dyn FnOnce(&mut EcsMaster)>>` drained once in `finish()` (line 272) |
| `Schedule::run` per-invocation tick contract | `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` — frame-start `world.bump_change_tick()` → `this_run` (line 209), published as `frame_this_run` for conditions (line 216); #56 apply-window second bump at the end (line 303) with `debug_assert_eq!(current, this_run + 1)` (lines 304–307). **Exactly two bumps per run, asserted.** |
| State transition pass is per-schedule, inside `run` | schedule.rs lines 286–291: gated on `state_entries.is_empty()`, runs before the executor loop, reuses the frame-start tick. `run_state_transitions` doc (≈ line 395): per-entry `pending_initial` ⇒ *"a state shared by two schedules fires its initial once per schedule"* — multi-schedule interaction already documented. |
| Apply windows are intra-run | schedule.rs `apply_window_drain` (line 595): deferred command applies happen inside each `Schedule::run`, gated on `pending == running` barrier. |
| Events flip is app-driven, not schedule-driven | `update_events` (swap path `swap_and_flatten`) requires `&mut EventDispatcher`; zero references to events in schedule.rs. Callers are `world.update_events()` in tests/benches only (`crates/boyko_ecs/tests/event_double_buffer.rs`: events sent frame N visible only after `update_events`, i.e. frame N+1). |
| No engine `Time` — explicitly deferred to the app | `crates/boyko_demo/src/sim/resources.rs` lines 15–18: `pub struct DeltaTime(pub f32);` with doc *"The engine has no built-in `Time`"* (demo plan §9 G8/G9). |
| The demo already runs a fixed-timestep accumulator over `Schedule::run` | `crates/boyko_demo/src/sim/runner.rs` — `FIXED_DT = 1.0/60.0` (line 95), `MAX_FRAME_DT = 0.25` clamp (line 100), `MAX_SUBSTEPS = 5` (line 103); `SimRunner.accumulator: f32` is a **struct field, not a resource** (line 114); `step()` writes `DeltaTime` then calls `self.schedule.run(world)` per substep (lines 329–351); on hitting the cap the backlog is **dropped** (`accumulator = 0.0`, line 347). The wasm runner duplicates the same rhythm sequentially without a `Schedule` (lines 453–472). |
| Display dt source | `crates/boyko_demo/src/app.rs` line 379: `let dt = ctx.input(|i| i.stable_dt)` — egui's smoothed, wasm-safe delta. |
| `Schedule` requires a pool | runner.rs lines 27–33: `ScheduleBuilder::new` requires `Arc<ThreadPool>`, `Schedule::run` enters `pool.install` — *"there is no sequential / no-pool execution path in the schedule"*; wasm cannot use `Schedule` at all. |

---

## Approaches in state-of-the-art engines

### Bevy (main branch, 0.17/0.18 era)

- **Approach**: many small schedules stored by label in a `Schedules` resource; a top-level `Main` schedule runs the others in a `MainScheduleOrder` list; fixed timestep is a *nested schedule loop* (`FixedMain`) driven by an accumulator on `Time<Fixed>`.
- **Schedule inventory** ([main_schedule.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_app/src/main_schedule.rs)): startup `[PreStartup, Startup, PostStartup]` (run once, on `Main`'s first invocation); main loop `[First, PreUpdate, RunFixedMainLoop, Update, SpawnScene, PostUpdate, Last]`; fixed loop `[FixedFirst, FixedPreUpdate, FixedUpdate, FixedPostUpdate, FixedLast]` under `FixedMain`. `MainScheduleOrder { labels: Vec<InternedScheduleLabel>, startup_labels: Vec<InternedScheduleLabel> }`. Execution: `Main::run_main` does `world.resource_scope(|world, order| { for &label in &order.labels { let _ = world.try_run_schedule(label); } })`; `FixedMain::run_fixed_main` mirrors it over `FixedMainScheduleOrder`.
- **Time stack** ([fixed.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_time/src/fixed.rs), [virt.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_time/src/virt.rs), [lib.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_time/src/lib.rs)):
  - `Time<Real>` (wall clock) → `Time<Virtual>` (clamped + scaled) → `Time<Fixed>` (quantized). `time_system` runs in `First`; `run_fixed_main_schedule` runs in `RunFixedMainLoop` inside the `RunFixedMainLoopSystems::FixedMainLoop` set.
  - `Time<Virtual>`: `max_delta` default **`Duration::from_millis(250)`**, `paused`, `relative_speed`, `effective_speed`. Raw delta is clamped to `max_delta` before accumulation; documented rationale: OS suspend/debugger freezes ("reporting the full elapsed delta time is likely to cause bugs in game logic") and the death spiral ("computing each frame takes longer and longer and the game will appear to freeze").
  - `Time<Fixed>`: fields `timestep: Duration`, `overstep: Duration`. Default timestep **64 Hz** (15 625 µs), chosen because it "losslessly converts into `f32` and `f64`" and avoids pathological refresh-rate interactions. `overstep_fraction() = overstep / timestep` — **this is the interpolation alpha**.
  - The loop: `accumulate_overstep(virtual_delta)`, then `while expend() { *world.resource_mut::<Time>() = Time<Fixed>.as_generic(); world.run_schedule(FixedMain); }`, then `*world.resource_mut::<Time>() = Time<Virtual>.as_generic()`. **The generic-`Time` swap is how the same system code reads fixed dt inside `FixedMain` and virtual dt outside.** **No max-step cap in the loop** — protection is entirely the upstream 250 ms clamp, which at 64 Hz arithmetically bounds the loop at 16 steps/frame.
- **Storage / dispatch**: `Schedules` resource = `HashMap<InternedScheduleLabel, Schedule>` ([schedule.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/schedule/schedule.rs)); `Interned<T>(pub &'static T)` with **pointer-based** `eq`/`hash` (`ref_eq`/`ref_hash`), values leaked by the interner ([intern.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/intern.rs)). `run_schedule` → `schedule_scope`, which **temporarily removes the schedule from the map, runs it, and reinserts it** ("hokey pokey", [PR #8387](https://github.com/bevyengine/bevy/pull/8387)), warning if a same-label schedule was inserted during the scope.
- **Trade-offs**: maximal plugin composability (any plugin can target any label, insert new schedules into the order); costs are one pointer-hash map remove+insert per schedule per frame, `try_run_schedule` silently skipping absent labels, and `Box`/`dyn` label machinery.

### flecs (master)

- **Approach**: one pipeline per world; *phases are entities* tagged `EcsPhase`, ordered by a topological sort over the `DependsOn` relationship; builtin phases `EcsOnLoad, EcsPostLoad, EcsPreUpdate, EcsOnUpdate, EcsOnValidate, EcsPostUpdate, EcsPreStore, EcsOnStore`; custom phases/pipelines are first-class ([Quickstart](https://github.com/SanderMertens/flecs/blob/master/docs/Quickstart.md), [Pipeline addon API](https://www.flecs.dev/flecs/group__c__addons__pipeline.html)).
- **Fixed rate is PER SYSTEM, not per schedule** ([Systems doc](https://www.flecs.dev/flecs/md_docs_2Systems.html), [timer.c](https://github.com/SanderMertens/flecs/blob/master/src/addons/timer.c)): `.interval(t)` attaches a timer; in `ProgressTimers`: `time_elapsed = timer.time + delta_time_raw; if (time_elapsed >= timeout) { t = time_elapsed - timeout; if (t > timeout) t = 0; timer.time = t; tick = true; }` — the system fires **at most once per `ecs_progress`**, carries remainder ≤ 1 interval, and **discards** backlog beyond 2× interval. **No catch-up loop**, hence no determinism guarantee under lag. `.rate(n)` = `tick_count % rate` divisors; shared *tick source* entities synchronize multiple systems. Systems running on timers read `delta_system_time` (time since *their* last run) instead of frame `delta_time`.
- **Frame pacing**: `ecs_progress(world, 0)` auto-measures delta; `ecs_set_target_fps` makes `ecs_progress` *sleep* the remainder of the frame ([Frame functions](https://www.flecs.dev/flecs/group__world__frame.html), [target_fps example](https://github.com/SanderMertens/flecs/blob/master/examples/c/systems/target_fps/src/main.c)).
- **Trade-offs**: zero schedule-level machinery, per-system granularity (different systems at different rates in one pipeline pass), but unsuitable as-is for deterministic physics catch-up (a stall yields slow-motion, not extra steps).

### Unity DOTS / Entities 1.0

- **Approach**: nested system groups (no label map; groups are classes containing systems). `FixedStepSimulationSystemGroup` sits inside `SimulationSystemGroup`; its update rhythm is delegated to a swappable **`IRateManager`** ([FixedStepSimulationSystemGroup](https://docs.unity3d.com/Packages/com.unity.entities@1.0/api/Unity.Entities.FixedStepSimulationSystemGroup.html)).
- **Default = catch-up loop**: [`RateUtils.FixedRateCatchUpManager`](https://docs.unity3d.com/Packages/com.unity.entities@1.0/api/Unity.Entities.RateUtils.FixedRateCatchUpManager.html) — default timestep **1/60 s**, runtime-overridable via `Timestep`. "The group updates exactly once for each elapsed interval of the fixed timestep"; documented example: elapsed 1.0 → 1.05 s at 0.02 ⇒ two updates with simulation times 1.02 and 1.04; a frame may also produce **zero** updates. The manager pushes a fixed elapsed/delta time onto the World's time stack for each step (PushTime/PopTime). Documented caveat: "if the wall time needed to simulate a single group update exceeds the fixed timestep interval, the group can end up even further behind" — spiral risk acknowledged, mitigated by the world-level clamp.
- **Clamp location**: `World.MaximumDeltaTime` (the old `FixedStepSimulationSystemGroup.MaximumDeltaTime` is deprecated; the world-level field "applies to both the fixed-rate and variable-rate timesteps", [0.16 changelog](https://docs.unity3d.com/Packages/com.unity.entities@0.16/changelog/CHANGELOG.html)). A non-catch-up variant exists: [`FixedRateSimpleManager`](https://docs.unity3d.com/Packages/com.unity.entities@1.0/api/Unity.Entities.RateUtils.FixedRateSimpleManager.html) (at most one step/frame — the flecs-like semantics).
- **Trade-offs**: the rate-manager interface makes the policy (catch-up vs throttle vs custom netcode step) pluggable per group; cost is OOP group hierarchy, not applicable structurally to boyko, but the *policy-behind-interface* split is the transferable idea.

### Godot 4

- **Approach**: engine-level. `_physics_process` runs at `physics_ticks_per_second`; physics interpolation is a project setting that makes the engine **automatically interpolate node transforms between physics ticks** ([Using physics interpolation](https://docs.godotengine.org/en/stable/tutorials/physics/interpolation/using_physics_interpolation.html)).
- **Contract**: "Setting the transform of objects only within physics ticks allows the automatic interpolation to deal with transforms *between* physics ticks"; setting transforms outside the tick causes jitter. Teleports require `reset_physics_interpolation()` (set transform first, then reset). Cameras are recommended to be interpolated manually — the [advanced doc](https://docs.godotengine.org/en/stable/tutorials/physics/interpolation/advanced_physics_interpolation.html) uses `Node3D.get_global_transform_interpolated()` inside `_process()`.
- **Trade-offs**: the *alpha lives engine-side* and the user mostly never touches it — the strongest evidence that previous/current snapshotting + lerp can be an engine/renderer concern rather than per-game code.

### Minimal baselines (other Rust ECS)

- **hecs**: ships nothing — "It is a library, not a framework. In place of an explicit 'System' abstraction, a `World`'s entities are easily queried from regular code" ([README](https://github.com/Ralith/hecs)). Fixed timestep is entirely the caller's loop.
- **boyko's own demo** is itself a baseline: a complete 3-mode game loop was built from **one** schedule + an app-side accumulator + one `DeltaTime` resource (`crates/boyko_demo/src/sim/runner.rs`).

---

## Comparative table

| Aspect | Bevy | flecs | Unity DOTS | Godot 4 | boyko today |
|---|---|---|---|---|---|
| Fixed-rate unit | schedule (`FixedMain`) | **per system** (timer entity) | system group | engine loop (`_physics_process`) | per app-driven `Schedule::run` call (demo) |
| Accumulator lives in | `Time<Fixed>.overstep` **resource** | per-system `EcsTimer` component | rate-manager object on the group | engine internals | `SimRunner.accumulator` struct field (demo) |
| dt clamp | `Time<Virtual>.max_delta` = 250 ms, upstream | none global; backlog > 2× interval discarded per timer | `World.MaximumDeltaTime` | engine settings | `MAX_FRAME_DT = 0.25` (demo) |
| Max steps/frame | no explicit cap (≤16 implied by clamp at 64 Hz) | 1 by construction | implied by clamp; pluggable via `IRateManager` | engine setting | `MAX_SUBSTEPS = 5` + backlog drop (demo) |
| Catch-up semantics | yes (while-expend loop) | **no** (throttle) | yes (default manager); simple manager opt-out | yes | yes (demo) |
| Default Hz | **64** | n/a (per system) | 60 | 60 | 60 (demo) |
| Fixed dt exposure | generic `Time` swapped to fixed during `FixedMain` | `delta_system_time` | pushed World time per step | `_physics_process(delta)` | `DeltaTime` resource overwritten before each run (demo) |
| Multi-schedule storage | `HashMap<InternedScheduleLabel, Schedule>` + remove/reinsert per run | pipeline = entity query over phases | class hierarchy of groups | n/a | none (one `Option<Schedule>` field) |
| Interpolation alpha | `Time<Fixed>::overstep_fraction()`; pattern in official example; ecosystem crate for transforms | not built-in | not built-in (user/netcode) | **engine-automatic** transform interpolation | absent |
| Events × fixed steps | swap gated on "fixed ran ≥ once" (`ShouldUpdateMessages`) | n/a (observers immediate) | ECB playback per sync point | n/a | `update_events()` app-driven, ungated |
| State transitions | dedicated `StateTransition` schedule after `PreUpdate` (+ before `PreStartup` at startup) | n/a | n/a | n/a | pass inside each `Schedule::run`, per-schedule `pending_initial` |

---

## Answers to the architect's questions

### a. Where the accumulator lives; who clamps dt

- **Bevy**: accumulator = `Time<Fixed>.overstep`, a plain resource field; mutated by the `run_fixed_main_schedule` system. Clamping is **not** in the fixed loop: `Time<Virtual>::advance_with_raw_delta` clamps raw delta to `max_delta` (default 250 ms) *before* it ever reaches the accumulator; the fixed loop itself runs unbounded `while expend()`. Implied bound: 250 ms / 15.625 ms = 16 steps. Pause/`relative_speed` also act at the virtual layer, so a paused game accumulates zero overstep for free.
- **flecs**: accumulator state lives in the `EcsTimer` component on each system entity; "clamp" = discard remainder when `time_elapsed - timeout > timeout`; never more than one fire per `ecs_progress`.
- **Unity**: accumulator state lives in the `FixedRateCatchUpManager` instance owned by the group; clamp = `World.MaximumDeltaTime` (world-level, shared with variable rate).
- **boyko demo**: accumulator is a `SimRunner` field; clamp = `frame_dt.min(MAX_FRAME_DT)` (0.25 s) + hard cap `MAX_SUBSTEPS = 5` + **backlog drop on cap** (`accumulator = 0.0`) — stricter than Bevy (Bevy never drops accumulated overstep; it only clamps inflow).

### b. Change detection × running the SAME schedule 0 or N times per frame

- **Bevy**: change windows are per-system (`last_run` vs `this_run`); a `FixedUpdate` system compares against *its own* previous run, so N runs per frame and 0 runs per frame both produce correct `Changed<T>`/`Added<T>` windows — a change made in `Update` is seen by a `FixedUpdate` system on its next run whenever that happens. This per-system-window property is the load-bearing design, not anything schedule-level.
- **boyko**: the same per-system `last_run` model holds (Phase 10), and `Schedule::run` is *already* proven self-contained per invocation — the demo runs it 0..=5×/frame today. Concretely per the code:
  - **N runs/frame**: each run bumps the change tick exactly twice (frame-start + apply-window, `debug_assert` at schedule.rs:304–307). The #56 contract "deferred-added components are observed exactly once on the *next frame*" silently becomes "on the *next run of that schedule*" — for fixed substeps that is the next substep, which matches Bevy's per-schedule-run sync points. No math breaks; only the doc wording ("frame") becomes inaccurate.
  - **0 runs/frame**: a system's/condition's `last_run` freezes while *other* schedules keep bumping the world tick. This is exactly the dormancy class Phase 16.1 addressed (tick-aware conditions; clamp against `MAX_CHANGE_AGE`). The wraparound scan (`should_run_check_ticks`, schedule.rs:222) is world-level, so with two schedules it fires from whichever runs — **needs an audit that the per-schedule clamp pass covers ALL schedules' systems**, since each `Schedule` only clamps its own `systems`/conditions/state entries.
  - **Cross-schedule visibility**: a `Changed<T>` in `Update` whose data was mutated during 3 fixed substeps sees it once (window spans all three runs) — correct by the window math, same as Bevy.
  - **Known residual**: the #56 side-effect F1 (direct-API `get_component_mut` inside a scheduled system stamps at the apply-window tick) multiplies per run — with substeps, the "one frame delayed" becomes "one substep delayed"; behaviorally milder, but worth re-stating in the contract doc.

### c. Commands / apply windows across fixed sub-steps

- **Bevy**: deferred buffers are applied at sync points *within* each schedule run; since `FixedMain` (and each of its five sub-schedules) is run once per fixed step, commands queued in a substep are applied before the next substep. No cross-substep deferral.
- **boyko**: identical shape already — `apply_window_drain` runs inside each `Schedule::run` under the barrier gate (schedule.rs:595), so each fixed substep fully drains its own commands. The #56 apply-window bump per run keeps deferred `Added`/`Changed` stamps one tick ahead per substep. Nothing new is needed here for substeps of a *single* schedule; the new design question is only whether *multiple distinct schedules* per frame each keep their own two-bump contract (they do, as long as each is a plain `Schedule::run`).

### d. Events across fixed steps

- **Bevy** (events renamed to *messages* in current main): buffers double-buffer **per update, not per fixed step**. `message_update_system` (early in the frame) is gated by `message_update_condition` reading `MessageRegistry::should_update: ShouldUpdateMessages { Ready | Waiting | Always }`; `signal_message_update_system` — registered by **TimePlugin in `FixedPostUpdate`** — sets `Ready` only when the fixed loop actually ran. Net effect ([message/update.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/message/update.rs)): **if `FixedMain` ran zero times in a frame, the swap is held (`Waiting`), so fixed-step readers never lose a buffer generation**; if it ran N times, all N substeps read the same generation and reader cursors prevent re-reads.
- **boyko**: `update_events()` is application-driven, ungated, and not wired into `Schedule::run` at all. Consequences to design around: (1) calling it once per display frame reproduces Bevy's "stable set across substeps" behavior for free; (2) **a reader living only in a fixed-step schedule can miss a whole generation on a 0-substep frame** — boyko has no `needs_update` analogue; (3) the Phase-12 per-lane TLS routing and `EventIter` cursor checkpointing are orthogonal and unaffected by call frequency, but `update_events` requires `&mut` (no live workers), so its only legal call sites are between `Schedule::run` calls — same place Bevy puts it (`First`).

### e. The interpolation story

- **Canonical** ([Fix Your Timestep!](https://gafferongames.com/post/fix_your_timestep/)): keep `previous` and `current` sim states; render `state = currentState * alpha + previousState * (1 - alpha)` with `alpha = accumulator / dt`. Interpolation (one step of latency) is preferred; extrapolation is the alternative when latency is unacceptable.
- **Bevy**: ships the *alpha* (`Time<Fixed>::overstep_fraction()`) and an official example pattern, not a built-in. [examples/movement/physics_in_fixed_timestep.rs](https://github.com/bevyengine/bevy/blob/main/examples/movement/physics_in_fixed_timestep.rs): input accumulation in `RunFixedMainLoopSystems::BeforeFixedMainLoop` (doc: input in `FixedUpdate` would "sometimes not register player input, as that schedule may run zero times per frame"); physics writes `PhysicalTranslation` + `PreviousPhysicalTranslation` in `FixedUpdate`; rendering in `AfterFixedMainLoop` does `let alpha = fixed_time.overstep_fraction(); transform.translation = previous.lerp(current, alpha);`.
- **Ecosystem**: [bevy_transform_interpolation](https://github.com/Jondolf/bevy_transform_interpolation) is the de-facto built-in — drop-in interpolation/extrapolation of `Transform` for changes made in fixed schedules; powers [Avian 0.2's built-in interpolation](https://joonaa.dev/blog/07/avian-0-2). So *yes*, the Rust ECS world ships transform interpolation, but as a crate, not in Bevy core.
- **Godot** is the engine-side extreme: automatic transform interpolation behind a project setting, `reset_physics_interpolation()` for teleports, `get_global_transform_interpolated()` for manual consumers (cameras).
- **Minimal renderer API distilled from all four**: (1) read `alpha` after the fixed loop finishes for the frame; (2) a snapshot point at the *start* of each fixed step to copy current → previous; (3) a teleport/reset escape hatch. The demo's `sync_gpu_instance` (post-step GPU mirror) is exactly the consumer that would take `alpha`.

### f. States

- **Bevy**: `StateTransition` is a *dedicated schedule*, inserted by `StatesPlugin` via `order.insert_after(PreUpdate, StateTransition)` and `insert_startup_before(PreStartup, StateTransition)` ([bevy_state/app.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_state/src/app.rs)); **once per frame, never inside `FixedMain`**.
- **boyko**: the Phase-17 pass runs inside whichever `Schedule::run` carries `state_entries`. In a multi-schedule world this forces a choice Bevy never faces: if states are registered on the fixed schedule, transitions apply per substep (0..N×/frame — a queued transition on a 0-substep frame waits, exactly like the demo's documented "switch queued while paused applies after unpausing", runner.rs:324–328); if on the per-frame schedule, fixed systems gated by `in_state` see transitions at frame granularity. The existing code already documents the duplication hazard: registering one state into two schedules runs *two* passes and fires *two* initials (schedule.rs `run_state_transitions` doc). The demo's wasm runner additionally proves the pass can be hoisted out of `Schedule::run` and driven externally (runner.rs `apply_mode_transition`).

### g. Schedule labels: interned-dyn vs enum/const-index

- **Bevy**: `#[derive(ScheduleLabel)]` → `label.intern()` → `Interned<dyn ScheduleLabel>` = `&'static dyn` with **pointer** eq/hash (interner leaks one canonical value per label); `Schedules` = `HashMap<InternedScheduleLabel, Schedule>`; `run_schedule` = `schedule_scope` = **remove from map → run → reinsert** ([PR #8387](https://github.com/bevyengine/bevy/pull/8387)), with a warning if a same-label schedule appeared meanwhile. Per-frame cost at Bevy's defaults: ~13 map remove+insert pairs plus pointer hashes — negligible in wall time, but it is heap-map traffic on the frame path, `try_run_schedule` swallows missing labels, and the removed-while-running trick means a system cannot run its own schedule.
- **flecs**: no label map — the pipeline is an entity and phase matching is a cached query; **Unity**: no labels at all, groups nest directly as objects.
- **Zero-overhead alternative consistent with boyko's house style** (array indexed by `ComponentId`-like ids, no `HashMap` on the hot path — CLAUDE.md forbidden list): a closed `enum`/const-index label set mapping to a fixed array of schedule slots, or `App` holding named `Schedule` fields outright (the current `schedule: Option<Schedule>` generalized). The cost of "dispatch" then is an array index or nothing (direct field access); the price is that plugins cannot mint new top-level schedule labels at runtime (they can still own positions in a `Vec<index>` order list, which is Bevy's `MainScheduleOrder` minus interning).

### h. Minimal viable schedule set

- Bevy's 15 labels exist for a plugin *ecosystem*: `PreUpdate`/`PostUpdate` are explicitly "engine/plugin preparation work" / "engine/plugin response work" — slots for third-party code to sandwich user logic. flecs ships 8 phases for the same reason. Unity's root is just 3 groups (Initialization/Simulation/Presentation).
- Evidence for a lean floor: hecs ships zero and works; **boyko's demo shipped a full product on exactly one schedule** + app-driven accumulator + Phase-15 `.before/.after` doing the work of `PreUpdate`/`PostUpdate` *inside* one schedule. The structurally irreducible set implied by the mechanics above is: a startup slot (exists: `App::add_startup_system`), one per-frame schedule, one fixed schedule run 0..N×, plus *positions* (not necessarily schedules) for the three per-frame chores that today have no home: time update + event swap (Bevy: `First`), state transition (Bevy: after `PreUpdate`), and a post-fixed-loop interpolation/render-prep point (Bevy: `RunFixedMainLoopSystems::AfterFixedMainLoop` / `PostUpdate`). Whether those chores are schedules, built-in passes (like the Phase-17 state pass), or App-loop steps is the architect's call — boyko already has precedent for the "built-in pass" form.

---

## Pitfalls and mistakes (recurring across sources)

1. **Unbounded catch-up = spiral of death** (Fiedler; Unity's own docs admit the group "can end up even further behind"). Every shipping engine clamps *somewhere*: Bevy at the virtual-delta inflow, Unity at `World.MaximumDeltaTime`, the demo at inflow + step cap + backlog drop. Clamping in *two* places (inflow and step cap) double-protects but changes semantics (boyko's demo drops backlog; Bevy never does).
2. **Input read inside the fixed schedule** loses events/edges on 0-substep frames — Bevy's example dedicates a system set (`BeforeFixedMainLoop`) and a doc paragraph to this exact bug.
3. **Event/message buffer swap decoupled from the fixed loop** silently drops events for fixed-step readers; Bevy needed a dedicated state machine (`ShouldUpdateMessages`) to fix it after the fact.
4. **Writing render transforms from the fixed step (or sim transforms from the render rate)** — Godot's docs hammer this: interpolated objects must be written only on physics ticks; teleports need an explicit interpolation reset, or you smear.
5. **Tick/version bookkeeping that assumes "once per frame"**: boyko's #56 wording and the Phase 16.1 dormancy fixes show the contract must be stated per *run*, not per *frame*; Bevy's per-system `last_run` is the proven model both codebases already share.
6. **Re-entrant schedule runs**: Bevy's remove-reinsert makes "a system runs its own schedule" structurally impossible (and warns on label collisions); boyko's apply-window barrier + dispatcher-exclusive `&mut EcsMaster` impose the same constraint by different means — any nested-schedule design must respect the "no live worker" invariant at the nesting point.

## Relevant canonical works

- "Fix Your Timestep!", Glenn Fiedler, 2004 (rev. 2019) — accumulator, max-frame-time clamp, render interpolation `alpha = accumulator/dt`. https://gafferongames.com/post/fix_your_timestep/
- Bevy `Time<Fixed>`/`Time<Virtual>` module docs (the in-source essays on overstep and death-spiral protection are the best current engineering write-up of the resource-based formulation).
- flecs Systems manual + `timer.c` — the per-system-timer counter-model and its deliberate non-catch-up semantics.

---

## Shapes available to boyko (for the architect; no recommendation)

**Hard constraints from the existing code, common to all shapes:**
- `Schedule::run` = exactly two tick bumps with a `debug_assert` (schedule.rs:303–307); any design either calls it as an opaque unit (preserving the contract per run) or renegotiates the contract explicitly.
- The state pass, apply windows, and `update_events` all require dispatcher-exclusive `&mut EcsMaster` (no live workers) — legal only between/around `Schedule::run`s or at its established barrier points.
- No `HashMap` on the frame path (CLAUDE.md); `EcsMaster` is `!Send + !Sync` (Arena TLS discipline) — Bevy's `Schedules`-as-resource + remove/reinsert would need a const-indexed replacement.
- `Schedule` requires an `Arc<ThreadPool>`; wasm has no schedule path at all — the wasm sequential runner must be expressible in whatever shape is chosen.
- Precedents already in-tree: `DeltaTime` overwritten before each run (runner.rs:338), inflow clamp 0.25 s + 5-substep cap + backlog drop, per-schedule `pending_initial`, eframe `stable_dt` as the display-delta source, `App::run` checking `AppExit` once per frame after the run.

**Shape 1 — Bevy-style nested fixed loop, const-indexed.** `App` grows a fixed array (or named fields) of schedules ordered by a `Vec` of const label indices; a `Time` resource family (`Real`/`Virtual` clamp/`Fixed` overstep) lives in resources; the fixed loop is either a driver system in a per-frame schedule (Bevy-faithful; requires solving nested-run aliasing without a `Schedules` remove/reinsert resource) or an `App`-level loop between schedule runs (sidesteps nesting entirely). Carries over: `overstep_fraction()` alpha, generic-Time swap (or the demo's simpler `DeltaTime`-overwrite), `First`-equivalent home for `update_events` with a Bevy-style "fixed ran ≥ once" gate.

**Shape 2 — App-as-rate-manager (promote the demo).** Keep schedules as plain `App` fields (`startup` drained-FnOnce list already exists; add `main: Schedule`, `fixed: Schedule`); `App::update` becomes: measure/clamp dt → write `Time` → `while accumulator >= dt && steps < cap { fixed.run(world) }` → `main.run(world)` → `update_events()`. This is `SimRunner::step` lifted into the engine with Unity's `IRateManager` lesson applied (the catch-up policy as a small swappable strategy: catch-up / simple / custom). Zero dispatch cost, zero new hot-path structures, trivially mirrors the wasm sequential runner; plugins target the two fixed slots rather than arbitrary labels.

**Shape 3 — flecs-style single schedule + per-system rate conditions.** No second schedule: a fixed-tick source (accumulator in a resource, advanced by a built-in pass like the Phase-17 state pass) drives `.run_if(on_fixed_tick)` conditions on individual systems, with `delta_system_time` semantics for their `DeltaTime`. Cheapest structurally (Phase 16 conditions already exist and are barrier-evaluated), but it inherits flecs's semantics: at most one fire per frame, no catch-up loop, no determinism under lag — the substep `while` cannot be expressed as a run condition. Viable only if the catch-up requirement is explicitly waived, or as a complement (rate-divided systems *inside* a fixed schedule, which flecs tick sources and Bevy's `.run_if(on_timer)` both validate).

## Open questions for the architect

- Backlog policy on cap: drop (demo, loses sim time, no spiral) vs retain (Bevy, sim slow-motion under sustained overload)?
- Does `update_events` get a fixed-aware gate (Bevy's `ShouldUpdateMessages`) in the same phase, or is the 0-substep event-loss documented as a footgun first?
- Where do states live in a two-schedule world — per-frame schedule (Bevy parity), fixed schedule (per-substep transitions), or hoisted into the App driver (wasm runner precedent)?
- Does the per-schedule `check_change_tick` clamp pass cover all schedules' systems once several schedules share one world's tick stream (b. residual)?
- Is the alpha consumer (`overstep_fraction`) a resource read only, or does boyko also ship the previous/current snapshot machinery (bevy_transform_interpolation-class) for the demo's GPU mirror?

## Sources

[1] https://github.com/bevyengine/bevy/blob/main/crates/bevy_app/src/main_schedule.rs
[2] https://github.com/bevyengine/bevy/blob/main/crates/bevy_time/src/fixed.rs
[3] https://github.com/bevyengine/bevy/blob/main/crates/bevy_time/src/virt.rs
[4] https://github.com/bevyengine/bevy/blob/main/crates/bevy_time/src/lib.rs
[5] https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/message/update.rs
[6] https://github.com/bevyengine/bevy/blob/main/crates/bevy_state/src/app.rs
[7] https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/schedule/schedule.rs
[8] https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/intern.rs
[9] https://github.com/bevyengine/bevy/pull/8387
[10] https://github.com/bevyengine/bevy/blob/main/examples/movement/physics_in_fixed_timestep.rs
[11] https://gafferongames.com/post/fix_your_timestep/
[12] https://www.flecs.dev/flecs/md_docs_2Systems.html
[13] https://github.com/SanderMertens/flecs/blob/master/src/addons/timer.c
[14] https://github.com/SanderMertens/flecs/blob/master/docs/Quickstart.md ; https://www.flecs.dev/flecs/group__c__addons__pipeline.html
[15] https://www.flecs.dev/flecs/group__world__frame.html ; https://github.com/SanderMertens/flecs/blob/master/examples/c/systems/target_fps/src/main.c
[16] https://docs.unity3d.com/Packages/com.unity.entities@1.0/api/Unity.Entities.FixedStepSimulationSystemGroup.html
[17] https://docs.unity3d.com/Packages/com.unity.entities@1.0/api/Unity.Entities.RateUtils.FixedRateCatchUpManager.html ; FixedRateSimpleManager ; @0.16 changelog (World.MaximumDeltaTime)
[18] https://docs.godotengine.org/en/stable/tutorials/physics/interpolation/using_physics_interpolation.html ; advanced_physics_interpolation.html
[19] https://github.com/Jondolf/bevy_transform_interpolation ; https://joonaa.dev/blog/07/avian-0-2
[20] https://github.com/Ralith/hecs
[21] Local: app.rs, schedule.rs, runner.rs, demo app.rs, resources.rs, event_double_buffer.rs (paths in-text)
