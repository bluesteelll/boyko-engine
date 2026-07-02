# Architecture: Phase 20 — Fixed Timestep + Multi-Schedule

Companion to `docs/PHASE-20-RESEARCH.md` (cited R-§…; background NOT repeated). Branch `ecs`. Format reference: `docs/PHASE-XI-PLAN.md`.

## Goal

Give the engine a built-in game-loop rhythm: a `Time` resource family, a deterministic fixed-timestep catch-up loop (default 64 Hz), and a multi-schedule `App` (Startup / Main / Fixed) — while keeping `Schedule::run` and the executor **byte-identical** and the existing single-schedule frame path at its current cost.

- **Functionality**: `Time` (virtual: clamped, pausable, scalable; real fields carried) + `FixedTime` (timestep, overstep, `overstep_fraction()` alpha); 0..N fixed `Schedule::run`s per frame with Bevy-class death-spiral protection (R-§a); `app.add_systems_in(CoreSchedule::Fixed, …)`; event-swap correctness for fixed-step readers (R-§d); the demo's hand-rolled runner machinery is **net-deleted** (dogfooding proof), and the wasm sequential runner is re-expressed over the same engine primitives.
- **Performance**: zero new structures on the frame path — no `HashMap`, no interned labels, no `Box<dyn>` schedule dispatch (R-§g); the fixed loop is App-level composition AROUND `Schedule::run` (no executor change); additive per-frame App cost is a declared, bench-bound budget (§Metrics P20-B1).
- **Correctness**: no fixed-reader event-generation loss on 0-substep frames (Bevy's `ShouldUpdateMessages` lesson, R-§d / R-Pitfall 3); change-tick clamp coverage for ALL schedules incl. dormant ones (the R-§b residual); determinism of the step count under a given dt sequence up to the inflow clamp.

## Context and constraints

- Affected: `core/app/app.rs` (multi-schedule + frame driver), NEW `core/time/` module, `core/schedule/schedule.rs` (ONE visibility change + doc wording sweep — zero executor logic edits), `core/ecs_master/ecs_master.rs` (NEW `pub(crate)` margin-aware check-ticks helper ★C1 + a `#[cfg(test)]` tick setter for W4; `contains_resource` already exists — no change), `core/change_detection/tick.rs` (NEW `CHECK_TICK_PREEMPT_MARGIN` const next to `CHECK_TICK_THRESHOLD` ★N2), `boyko_demo` (`sim/runner.rs`, `app.rs`, `sim/resources.rs`, system bodies' `DeltaTime` reads).
- NOT affected (binding): the executor main loop, `try_dispatch_ready`, apply-window drain, conflict graph, `boyko_threadpool`, events internals (`EventBuffer`/`EventDispatcher` code), Phase-17 state machinery (`StateEntry`/`apply_state_transition`), `ScheduleBuilder` (consumed as-is, one instance per schedule).
- Invariants preserved: `Schedule::run` = exactly two tick bumps, asserted (schedule.rs:303-307) — the design calls it as an **opaque unit** (R-hard-constraints); `update_events`/state pass/check-ticks all run under dispatcher-exclusive `&mut EcsMaster` between runs; `EcsMaster` stays `!Send + !Sync`; plain `&mut` window mutations only — no RAII-guard-cached-`NonNull` (14a-F2/9.3c classes structurally absent); zero new `unsafe` expected.
- Hard platform constraint: `Schedule` requires `Arc<ThreadPool>`; wasm32 has none — all new *rhythm logic* (clock math, accumulator, catch-up loop) must be expressible without a `Schedule` (R-§ anchor row "Schedule requires a pool") and must be the SAME code path on native, wasm, and Miri (the X.I D9 unified-path lesson).
- Binding targets: §Metrics P20-B1…P20-B7.

## Key decisions

### D1: The fixed loop is an App-level driver between `Schedule::run`s (Shape 2), not a nested driver system (Shape 1)
**What**: `App::update_with_delta` runs, in order: ① time advance → ② cold check-ticks pass → ③ gated event swap → ④ fixed catch-up loop (`while expend { fixed.run(world) }`) → ⑤ `main.run(world)`. Each `Schedule::run` is opaque; all inter-run work holds the dispatcher's own `&mut EcsMaster` with zero workers in flight.
**Why**: (a) Bevy's nested form exists to serve a plugin ecosystem and requires the remove-run-reinsert `HashMap` trick to solve schedule self-aliasing (R-§g, PR #8387) — the exact structure CLAUDE.md forbids; solving aliasing without it means a second ownership dance for zero benefit. (b) boyko's `update_events`, state pass, and tick contract all demand `&mut EcsMaster` with no live worker — the App-level slot between runs is the natural barrier (R-hard-constraints); a nested driver would have to re-prove that inside `pool.install`. (c) The demo ALREADY runs this exact shape in production (`SimRunner::step`, R-anchor) — Shape 2 is promotion of proven code, the cheapest-risk option. (d) Fixed-before-Main matches Bevy's `RunFixedMainLoop`-before-`Update` relative order (R-§ Bevy schedule inventory), so input-edge and transition-latency semantics match the reference engine.
**Alternatives rejected**: Shape 1 (nested driver system) — needs an aliasing solution Bevy solves with a forbidden structure, and puts `&mut`-requiring chores inside the install window; Shape 3 (per-system rate conditions) — cannot express the catch-up `while` (at most one fire/frame, no determinism under lag — R-Shape 3); it remains available LATER as a complement (rate-divided systems inside Fixed).
**Trade-off**: plugins cannot mint new top-level schedules at runtime (closed set, D5); a system cannot itself drive the fixed loop (it is not a system). Both are deliberate.

### D2: Two separate resources — `Time` + `FixedTime`; NO generic-Time swap
**What**: `Time` = virtual time (per-frame `delta` clamped to `max_delta` then scaled by `relative_speed`, `paused` ⇒ delta zero) carrying the real (unclamped) fields alongside; `FixedTime` = `timestep` + `overstep` (THE accumulator) + `overstep_fraction()` (THE alpha) + `elapsed` + `steps_this_frame`. Fixed-schedule systems read `Res<FixedTime>` (`delta()` ≡ timestep); frame systems read `Res<Time>`.
**Why**: Bevy's generic-`Time` swap costs one resource overwrite per substep plus a per-frame restore (R-§ Bevy time stack) and makes the same type read differently depending on which schedule observes it — runtime magic with measurable cost. Two resources cost **zero writes** beyond the intrinsic overstep mutation, are branch-free, and make the read site self-documenting. Folding real time into `Time` fields (instead of a third `Time<Real>` resource) avoids a third slab slot for data that has no independent consumer yet.
**Alternatives rejected**: generic swap (cost + magic, above); promoting the demo's `DeltaTime(f32)` (no clamp/pause/scale/alpha story, f32 accumulator drift); three resources (Real/Virtual/Fixed — Real has no consumer; add later without breakage if profiling/replay needs it).
**Trade-off**: a system body cannot be moved between Main and Fixed without changing its `Res<…>` parameter — accepted; the type IS the documentation of which clock the system marches to.

### D3: Accumulator = `FixedTime.overstep` (resource), driven by ONE shared `fixed_advance` function (the unified path)
**What**: `pub fn fixed_advance<F: FnMut(&mut EcsMaster)>(world: &mut EcsMaster, step: F) -> u32` — reads `Time.delta`, accumulates into `FixedTime.overstep`, loops `expend()` (subtract one timestep, return true) calling `step` per substep, records `steps_this_frame`, returns the count. `App` calls it with `|w| fixed.run(w)`; the wasm demo runner calls it with its sequential step closure; Miri tests call it with a counting closure (no pool).
**Why**: resource location makes the alpha and step count readable by systems (requirement 6) and serializable; an App-field accumulator would force the wasm runner to duplicate the math (the divergence that bred the demo's f32-drift and drop-on-cap idiosyncrasies). One generic function = monomorphized direct calls (no `dyn`), and Miri/wasm traverse the identical accumulate/expend/clamp logic they ship with (X.I D9). Duration (integer ns) arithmetic, not f32: 64 Hz = exactly 15 625 000 ns; step counts are bit-deterministic for a given dt sequence (requirement 2).
**★M3 (binding shape)**: `fixed_advance` SNAPSHOTS `timestep` ONCE at loop entry and threads it through every `expend(ts)` call and `elapsed += ts` increment — `expend` takes the snapshot as a parameter (`pub(crate) fn expend(&mut self, ts: Duration) -> bool`). Without the snapshot, a fixed-schedule system holding `ResMut<FixedTime>` calling `set_timestep` mid-loop would void the "next-frame effect" promise, the ⌈(max_delta+timestep)/timestep⌉ bound (a 1 ns timestep ⇒ ~250 M iterations), and the P20-B3/B4 gates. Residual exposure: `ResMut<FixedTime>` mid-loop mutation of `overstep` (e.g. `discard_overstep`) remains observable next iteration — identical to Bevy's exposure; doc note. **★m6**: `fixed_advance` is `pub` and must be called EXACTLY ONCE per `Time::advance_with` (a second call re-accumulates the same delta) — doc contract + T3 covers the missing-`Time`/missing-`FixedTime` panics.
**Alternatives rejected**: App-field accumulator (demo precedent — but it is precisely the duplicated-machinery class this phase deletes); f32 seconds accumulator (drift, non-exact timesteps); `&mut dyn FnMut` (needless indirect call on the substep path).
**Trade-off**: one `resource_mut::<FixedTime>()` re-borrow per substep (a slab index, single-digit ns vs the µs-scale `Schedule::run` it brackets — bounded by P20-B2).

### D4: Backlog policy — inflow clamp ONLY (Bevy model); no step cap, no backlog drop; default 64 Hz, `max_delta` = 250 ms
**What**: `Time` clamps the raw delta to `max_delta` (default 250 ms) BEFORE scaling/accumulation; the catch-up loop runs unbounded `while expend()`. Implied bound: ⌈(max_delta × relative_speed + timestep)/timestep⌉ steps/frame is the safe UPPER bound (evaluates to 17 at defaults); the exact count at defaults is **16** because `overstep_prev < timestep` strictly and 250 ms is an exact multiple of the 64 Hz timestep — P20-B3's binding "exactly 16" is the correct number (★N4). `overstep` is never discarded by the engine; `FixedTime::discard_overstep()` is the explicit escape hatch (teleport/long-pause resume).
**Why**: requirement 1 binds to R-§a (Bevy's inflow-clamp model); the clamp is the single spiral guard every shipping engine has somewhere (R-Pitfall 1), and clamping in ONE place keeps semantics analyzable. Retain-vs-drop: dropping (the demo's `accumulator = 0.0`) silently loses simulated time non-deterministically under load — it breaks requirement 2's "deterministic under load up to the clamp"; retaining yields bounded slow-motion under sustained overload (Bevy/Unity behavior), which is the predictable failure mode. 64 Hz: lossless f32/f64 conversion + avoids refresh-rate beat patterns (R-§ Bevy `Time<Fixed>`).
**Alternatives rejected**: demo's clamp+cap+drop (non-deterministic time loss; double clamping obscures which guard fired); configurable policy enum (Unity `IRateManager`) — YAGNI now, the seam exists (the loop is 6 lines inside `fixed_advance`; a policy enum can be added without API breakage when a netcode consumer appears).
**Trade-off**: a user who raises `max_delta` or `relative_speed` raises the worst-case substep count proportionally — documented invariant, their explicit configuration choice.

### D5: Multi-schedule storage — named `Option<Schedule>` fields + closed `CoreSchedule` enum, routing at CONFIG time only; existing `add_systems` stays as the Main alias (no break)
**What**: `App` grows `fixed_builder: Option<ScheduleBuilder>` (created lazily on first Fixed registration) and `fixed: Option<Schedule>`; the existing `builder`/`schedule` fields ARE Main. `pub enum CoreSchedule { Main, Fixed }` is matched only inside config methods (`add_systems_in`, `add_systems_cfg_in`, `init_state_in`, `insert_state_in`); the frame path is direct field access — **zero dispatch** (R-§g zero-overhead alternative). `add_systems` / `add_systems_cfg` / `init_state` / `insert_state` / `add_startup_system` keep their exact signatures, documented as Main-routing.
**Why**: requirement 3 forbids a frame-path schedule map; Bevy's `HashMap` + hokey-pokey exists for open plugin label minting we deliberately exclude (D1 trade-off). Named fields beat a `[Option<Schedule>; 2]` array: the frame driver's field accesses compile to fixed offsets and read in the order they execute; an array buys nothing for 2 elements. Keeping the one-arg `add_systems` avoids touching the existing call-site population (Phase-18 tests + benches + book examples) — migration cost zero.
**Alternatives rejected**: `HashMap<Label, Schedule>` (forbidden, R-§g); a `ScheduleLabel` trait with user-mintable labels (re-imports the dyn/interning machinery; the R-§h evidence is that a lean closed set suffices — the demo shipped a product on ONE schedule); breaking `add_systems` to take a label first-arg (gratuitous churn).
**Trade-off**: third-party plugins target `Main`/`Fixed` positions only, plus Phase-15 sets WITHIN a schedule (which is what `PreUpdate`/`PostUpdate` are for in Bevy anyway — R-§h). New top-level slots are an engine change by design.

### D6: Event swap — once per frame at frame start, gated on "fixed ran ≥ 1 substep since the last swap" (Bevy `ShouldUpdateMessages` semantics, boyko-native form)
**What**: `App` owns `fixed_steps_since_swap: u32` and `event_policy: EventUpdatePolicy { WaitForFixed, EveryFrame }`. Frame start (step ③, before the fixed loop): swap iff `policy == EveryFrame || fixed.is_none() || fixed_steps_since_swap > 0`; on swap, call `world.update_events()` and zero the counter. The fixed loop then adds its step count. Default policy resolved at `finish()`: `WaitForFixed` iff a fixed schedule was built, else `EveryFrame`. `EventDispatcher`/`EventBuffer` code: **zero changes**.
**Why**: requirement 4 + R-§d — a fixed-schedule `EventReader` must not lose a buffer generation on a 0-substep frame; Bevy needed a post-hoc state machine for exactly this (R-Pitfall 3). The boyko-native equivalent is two App fields and one branch — no registry, no signal system, because the swap site and the fixed loop live in the same function. Swap-at-frame-start (Bevy `First` parity) gives every substep and the main run of a frame one stable generation (R-§d consequence 1). The placement BEFORE the fixed loop means the gate reads the counter accumulated by previous frames — the hold composes correctly across consecutive 0-substep frames.
**Alternatives rejected**: ungated per-frame swap (silent fixed-reader loss — the documented Bevy bug class); swap per substep (re-introduces intra-frame generation churn and `&mut` choreography for zero benefit — Bevy deliberately swaps per UPDATE, not per fixed step); putting the gate inside `EventDispatcher` (the dispatcher has no knowledge of schedules; App is the only place that knows both).
**Trade-off / documented hazard (★M1 — expanded per critic)**: under `WaitForFixed`, a PAUSE holds the swap **indefinitely** (paused ⇒ 0 substeps ⇒ counter stays 0), starving ALL readers — including Main-only readers unrelated to the fixed schedule (paused menu sending UI events = the canonical case). After `capacity_per_lane` (1024) sends per lane, `send` returns `Err(EventBufferFull)` — and the caller may ignore that `Result` (silent drop); `overflow_count` surfaces only via debug-only `diagnostics()`. On unpause the held backlog floods readers in one generation (bounded by lanes × capacity, but semantically a stale-event burst). **Recorded rulings**: (a) the `EventUpdatePolicy` doc + book page pair the PAUSE workflow with the policy explicitly (held swap → eventual `Err` → unpause flood) and instruct pause-capable apps to check `send`'s `Result` or use `EveryFrame` when no fixed readers exist; (b) release-build observability of held-swap saturation is FILED with the events follow-up (debug `diagnostics()` is the only counter this phase); (c) the demo does NOT dogfood this hazard (it sends no events) — P20-B6 green does not cover it; **P20-B5 must include a pause-spanning hold script** (★M1) and the cold-start trace (★m4: startup events become visible at the first post-substep swap, worst nominal delay ≈ 2 frames at 60 FPS/64 Hz; no loss, only bounded delay — unbounded only under pause). A user-set `WaitForFixed` with NO fixed schedule silently degrades to `EveryFrame` via the `fixed.is_none()` arm — one doc sentence. Worlds driven by `App` must NOT also call `update_events()` manually (double-flip halves visibility windows) — App contract note; the existing manual call sites are App-less tests, audited in W3.

### D7: States stay per-schedule (Phase 17 machinery untouched); App routes state registration to Main by default; binding same-schedule contract for edge conditions
**What**: `App::init_state` / `insert_state` register into the **Main** builder (unchanged code path); new `init_state_in` / `insert_state_in` take a `CoreSchedule`. The Phase-17 pass keeps running inside whichever `Schedule::run` carries the entries. **Binding contract (doc + book)**: `on_enter`/`on_exit`/`on_transition` conditions are valid only on systems in the SAME schedule where the state is registered; `in_state` (plain value read) is valid anywhere at frame granularity; registering one state type into both schedules is rejected by documentation (double pass + double initial — already documented in `run_state_transitions`).
**Why**: `StateTransitionRecord<S>` is an `Option` cleared at the start of every pass, NOT a tick window (`transition_record.rs:59-71`) — a transition recorded by Main's pass is visible to Fixed's conditions only during the fragile interval before the next Main pass clears it, and a 0-substep frame **misses it entirely**. Cross-schedule edge conditions are therefore structurally unsound under the current record shape; the honest design is the same-schedule contract, which both Bevy (OnEnter schedules run inside StateTransition — never observable cross-loop) and the demo (Mode + all its edge-gated systems in ONE schedule) already obey. Hoisting the pass to App level (the wasm-runner precedent, R-§f) was rejected: it breaks same-frame visibility for the consuming schedule (the record must be set inside the run that evaluates the conditions — Phase 17's load-bearing placement) and would fork the `pending_initial`/tick-stamp contract for zero requirement.
**Alternatives rejected**: App-level pass (above); fixed-only registration default (0-substep frames delay every transition — wrong default for UI-driven states); a tick-windowed record enabling cross-schedule edges — REAL but out of scope, filed as the Phase 20.x follow-up (one `Option<Tick>` window per record + condition rewrite).
**Trade-off**: transition latency for `in_state`-gated fixed systems is frame-granular when the state lives on Main (queued frame N → applied in Main(N) → seen by Fixed's substeps in N+1) — identical to Bevy's `NextState`-in-`Update` latency (D1 ordering note). The demo keeps Mode on Fixed (per-substep transitions, today's exact behavior).

### D8: Change-tick clamp coverage — App-level all-schedule pass at frame start with a PREEMPT MARGIN (★C1); `Schedule::run`'s internal block stays as the standalone belt
**What**: frame step ②: `if world.should_run_check_ticks_with_margin(CHECK_TICK_PREEMPT_MARGIN)` (one u32 compare, predicted-not-taken; fires when `current − last_check ≥ CHECK_TICK_THRESHOLD − MARGIN`) → `#[cold] #[inline(never)] App::check_ticks_all_schedules`: `run_check_ticks_scan(world)` + `schedule.check_change_ticks(t)` + `fixed.check_change_ticks(t)` + `world.set_last_check_tick(t)`, with `t = world.current_tick()`. NEW `CHECK_TICK_PREEMPT_MARGIN = 4096` (`change_detection/tick.rs`, next to `CHECK_TICK_THRESHOLD` — ★N2) + a `pub(crate)` margin-aware world helper next to `should_run_check_ticks`. `Schedule::check_change_ticks` visibility: private → `pub(crate)` (a `#[cold]` fn; codegen-neutral, P20-B1a verifies). `Schedule::run`'s own threshold block is untouched and stays as the standalone-user belt.
**Why the margin (★C1 — the critic's re-derivation, accepted)**: the naive "App pass resets the counter first" claim is FALSE — both paths read the SAME world counter, the App checks BEFORE the frame's bumps and the internal blocks check AFTER their frame-start bump, so at a threshold-crossing frame the internal block of the first schedule to bump wins ~75% of crossings, clamps only ITS OWN systems, resets the shared counter, and STARVES the sibling schedule's clamp — two consecutive internal wins push a dormant sibling system past `MAX_CHANGE_AGE + 2×THRESHOLD`, which WRAPS `is_newer_than` (the tick.rs:59 inequality has exactly one threshold of slack). The margin makes the App pass fire strictly earlier: after a reset, elapsed grows ≤ 2×(1+substeps) ≤ 34 ticks/frame at defaults — far below 4096 — so the App check always crosses `T − 4096` at a frame start before any internal block can reach `T` mid-frame. Invariant: a frame must not consume more than MARGIN ticks (= 2047 substeps; debug_assert in `update_with_delta`). The App is the only owner that can enumerate all schedules; clamping at frame start with un-bumped `current` is sound (`check_tick` only pulls old values forward; 1-34 ticks of staleness is noise against ~518 M slack — critic-verified).
**Alternatives rejected**: modifying `Schedule::run` to take a sibling list (executor change — forbidden by requirement 8); making `check_change_ticks` `pub` for hand-rollers (multi-schedule support IS the App; hand-rolled multi-schedule without App is documented as unsupported for clamp coverage); margin-less App pass (the ★C1 race above).
**Trade-off**: the cold scan fires 4096 ticks early (negligible against the ~518 M threshold); +1 u32 compare per frame. Tick consumption is now 2×(1 + substeps) bumps per frame ⇒ at a sustained 16 substeps/60 FPS the cold O(world) scan fires every ~70 h of uptime instead of ~50 days — quantified, acceptable. **W4 must test the race shape itself**: place the crossing MID-FRAME (between the App check and a substep/Main bump) and assert the SIBLING schedule's dormant condition still gets clamped — a test that only forces `last_check_tick` near the threshold would pass even with the race present (★C1 test note).

### D9: Interpolation — alpha only this phase; snapshot machinery filed
**What**: `FixedTime::overstep_fraction() -> f32` (= overstep/timestep, ∈ [0, 1)) read after the fixed loop via `Res<FixedTime>` from Main systems (the driver order guarantees the loop has settled before Main runs). Previous/current snapshot components + lerp (bevy_transform_interpolation-class) are NOT shipped; the canonical pattern (snapshot at substep start, lerp in Main) is documented in the book page with the Fiedler/Bevy-example citation (R-§e).
**Why**: requirement 6 demands the alpha be readable — that is a resource read, free. Bevy ships exactly alpha + an example; the ecosystem crate proves snapshotting works as a layer ON TOP without engine hooks (R-§e). Shipping engine-side snapshots now would force a component-pair convention before the demo has even consumed the alpha once.
**Trade-off**: demo GPU mirror stays substep-fresh (not interpolated) this phase; filed as the Phase 20.1 demo follow-up ("interpolated `sync_gpu_instance`").

### D10: Demo migration — native onto `App` (SimRunner deleted); wasm keeps sequential dispatch but adopts `Time`/`FixedTime`/`fixed_advance`; demo adopts the 64 Hz default
**What** (the net-deletion proof, requirement 7):
- **Native**: `DemoApp` holds `App` (replaces `world` + `pool` + `SimRunner` fields). `SimRunner::new`'s builder body moves verbatim into `app.add_systems_cfg_in(Fixed, |b| { … b.insert_state(Mode::Particles); … })`. Per frame: sync `SimParams.paused` → `Time::pause/unpause` (2 lines), then `app.update_with_delta(Duration::from_secs_f32(dt))`; substep count for the stats panel = `world.resource::<FixedTime>().steps_this_frame()`. World access for click-burst/upload/panel via `app.world()/world_mut()`.
- **wasm**: `wasm_runner::SimRunner` keeps `pending_initial` + the sequential per-mode dispatch (a pool is impossible — R-hard-constraint), but its `step` becomes: `Time::advance_with(dt)` + `fixed_advance(world, |w| run_sim_step_sequential(w, pending))` — the hand-rolled accumulator/clamp/cap code is deleted.
- **Deleted**: native `SimRunner` struct + `step`, `FIXED_DT`/`MAX_FRAME_DT`/`MAX_SUBSTEPS`, the wasm accumulator fields/loop, and the `DeltaTime` resource (system bodies read `Res<FixedTime>` → `delta_secs()`; shared bodies, both targets, one edit per system signature).
- **★M2 — explicitly assigned responsibilities** (the critic's runner.rs audit): (1) **resource seeding** (`SpatialGrid`/`BoidSnapshot`/`BoidParams`/`PhysicsParams`/`BallSnapshot`, runner.rs:148-159 — world ops BEFORE the builder, not builder body) → `app.insert_resource(...)` calls during config; (2) **wasm `Time`/`FixedTime` insertion**: no App/finish() exists on wasm, so wasm `SimRunner::new` inserts both resources directly (`insert_resource` + the pub constructors — feasibility critic-confirmed); (3) **wasm pause**: the current pre-accumulate early-return on `SimParams.paused` is REPLACED by the same `Time::pause/unpause` sync as native (keeps the D3 same-code-path claim exact; a paused wasm frame = advance_with(dt) with paused ⇒ delta ZERO ⇒ 0 substeps).
- **Behavior deltas (accepted, documented in W5 notes)**: 60 Hz → 64 Hz (engine default; no sim constant depends on 1/60 — critic-verified all bodies scale by `delta_secs`); drop-on-cap → retain-with-clamp (D4 — under sustained overload the demo now slow-motions instead of losing time); max substeps 5 → 16 implied; **a paused demo now still runs Main every frame** (today: nothing runs — empty Main for the demo, harmless, but the next demo feature must know).
**Why**: this is the dogfooding gate — if the engine API cannot replace the demo's runner with strictly less code, D1-D4 are wrong. The wasm path proves D3's unified-path claim in shipping code.
**Trade-off**: wasm still duplicates the *dispatch* (sequential per-mode match) — that is the pool gap, explicitly out of Phase 20 scope (a no-pool `Schedule` execution mode is its own phase).

### D11: Clock source — `update()` self-clocks via `Instant`; `update_with_delta(Duration)` is the external-clock entry; first frame delta = ZERO
**What**: `App` gains `last_instant: Option<Instant>`; `update()` computes `raw = now − last` (ZERO on the first frame, Bevy parity) and delegates to `update_with_delta(raw)` — the single frame function. `run()`/`run_n()` loop `update()`; new `run_n_with_delta(frames, delta)` for deterministic tests/benches.
**Why**: eframe owns the demo's loop and supplies egui's wasm-safe `stable_dt` (R-anchor) — an engine that insists on owning the clock cannot be embedded; an engine without a default clock punts the 90% case. Both entries share one body; `Instant` appears only in the thin native shell (compiles on wasm — `App` construction **panics at runtime on wasm** (★m3: `ThreadPoolBuilder::build`'s worker-spawn `expect` — a runtime guarantee, not a compile-time one; the wasm demo never constructs an App); **Miri tests must use `update_with_delta`** — `Instant::now` requires `-Zmiri-disable-isolation`). **★m5**: `advance_with` bypasses `mul_f64` when `relative_speed == 1.0` — the default chain stays pure integer-ns end-to-end (mul_f64 is IEEE-deterministic but not bit-identical to the unscaled delta, and costs two float conversions).
**Trade-off**: none measurable; one `Option<Instant>` field.

## Data structures

```rust
// NEW: core/time/time.rs — plain resource; written once per frame by the driver,
// read by systems. No repr/alignment pinning: never on a per-entity hot loop.
pub struct Time {
    delta: Duration,        // virtual delta this frame: clamp(raw, max_delta) * speed; ZERO when paused
    delta_secs: f32,        // cached f32 of `delta` (the per-system read)
    elapsed: Duration,      // sum of virtual deltas
    real_delta: Duration,   // raw, unclamped, unscaled
    real_elapsed: Duration, // wall-clock sum
    max_delta: Duration,    // inflow clamp, default 250 ms (D4); setter-validated > 0
    relative_speed: f32,    // default 1.0; setter-validated >= 0, finite
    paused: bool,
}

// NEW: core/time/fixed_time.rs
pub struct FixedTime {
    timestep: Duration,         // default 15_625_000 ns (exactly 64 Hz); setter-validated > 0
    timestep_secs: f32,         // cached (delta_secs() read)
    overstep: Duration,         // THE accumulator (D3); invariant: < timestep after each fixed loop
    elapsed: Duration,          // sum of expended timesteps (determinism witness)
    steps_this_frame: u32,      // written by fixed_advance after the loop; read by Main systems / demo stats
}
// Both impl Resource via the standard non-generic mechanism (same as the demo's SimParams).

// core/app/app.rs — App field deltas (existing fields unchanged; drop order remains
// non-load-bearing: every `run*` takes &mut self ⇒ no frame in flight at drop).
pub struct App {
    // … existing: world, builder (= Main), schedule (= Main), startup,
    //   plugin_type_ids, pool, finished …
    fixed_builder: Option<ScheduleBuilder>, // lazily created on first *_in(Fixed, …)
    fixed: Option<Schedule>,                // None ⇒ the fixed branch is one predicted-not-taken check
    fixed_timestep: Duration,               // config staging; applied to FixedTime at finish()
    event_policy_cfg: Option<EventUpdatePolicy>, // user override; None ⇒ auto-resolve at finish (D6)
    event_policy: EventUpdatePolicy,        // resolved
    fixed_steps_since_swap: u32,            // D6 gate counter (saturating_add)
    last_instant: Option<Instant>,          // D11 self-clock
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoreSchedule { Main, Fixed }       // closed set (D5); matched in config methods only

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventUpdatePolicy { WaitForFixed, EveryFrame } // D6
```

## Public API (deltas only)

```rust
// core/time (module re-exported from core::time; added to the crate's public surface)
impl Time {
    pub fn delta(&self) -> Duration;        pub fn delta_secs(&self) -> f32;
    pub fn elapsed(&self) -> Duration;
    pub fn real_delta(&self) -> Duration;   pub fn real_elapsed(&self) -> Duration;
    pub fn pause(&mut self);                pub fn unpause(&mut self);
    pub fn is_paused(&self) -> bool;
    pub fn relative_speed(&self) -> f32;    pub fn set_relative_speed(&mut self, s: f32);
    pub fn max_delta(&self) -> Duration;    pub fn set_max_delta(&mut self, d: Duration);
    pub fn advance_with(&mut self, raw: Duration); // pub: the wasm runner's entry (D10)
}
impl Default for Time;

impl FixedTime {
    pub fn new(timestep: Duration) -> Self; pub fn from_hz(hz: f64) -> Self;
    pub fn timestep(&self) -> Duration;     pub fn set_timestep(&mut self, d: Duration); // takes effect next frame
    pub fn delta(&self) -> Duration;        pub fn delta_secs(&self) -> f32;  // == timestep
    pub fn overstep(&self) -> Duration;     pub fn overstep_fraction(&self) -> f32; // THE alpha, in [0, 1)
    pub fn discard_overstep(&mut self);     // teleport / pause-resume escape hatch (D4)
    pub fn elapsed(&self) -> Duration;      pub fn steps_this_frame(&self) -> u32;
    // accumulate()/expend(ts: Duration) are pub(crate): only fixed_advance drives
    // them; expend takes the loop-entry timestep SNAPSHOT (★M3).
}
impl Default for FixedTime; // 64 Hz

// core/time/fixed_loop.rs — the unified driver (D3). Monomorphized; no dyn.
pub fn fixed_advance<F: FnMut(&mut EcsMaster)>(world: &mut EcsMaster, step: F) -> u32;

// core/app/app.rs
impl App {
    pub fn add_systems_in<F, M>(&mut self, schedule: CoreSchedule, system: F) -> &mut Self
        where F: IntoSystem<(), (), M>, F::System: System<Out = ()> + 'static;
    pub fn add_systems_cfg_in(&mut self, schedule: CoreSchedule,
        f: impl FnOnce(&mut ScheduleBuilder)) -> &mut Self;
    pub fn init_state_in<S: States + Default>(&mut self, schedule: CoreSchedule) -> &mut Self;
    pub fn insert_state_in<S: States>(&mut self, schedule: CoreSchedule, state: S) -> &mut Self;
    pub fn set_fixed_timestep(&mut self, d: Duration) -> &mut Self; // + set_fixed_hz(f64) convenience
    pub fn set_event_update_policy(&mut self, p: EventUpdatePolicy) -> &mut Self;
    pub fn update_with_delta(&mut self, raw: Duration); // the frame function (D11)
    pub fn run_n_with_delta(&mut self, frames: u64, delta: Duration); // deterministic loop
    // UNCHANGED signatures: add_systems / add_systems_cfg / init_state / insert_state
    //   / add_startup_system / update / run / run_n / world / world_mut / pool / finish
    //   — the one-arg forms are documented as CoreSchedule::Main routing (D5).
}

// core/schedule/schedule.rs — visibility only:
pub(crate) fn check_change_ticks(&mut self, current: Tick); // was private (D8); #[cold] body unchanged

// core/ecs_master — ★m1: contains_resource already exists (ecs_master.rs:2221),
// finish() uses it as-is. The ONLY edits: NEW pub(crate) margin-aware
// check-ticks helper next to should_run_check_ticks (★C1, D8) + a
// #[cfg(test)] tick setter for the W4 race-shape test.
```

`finish()` additions (cold, once): resolve `event_policy`; insert `Time::default()` and `FixedTime::new(self.fixed_timestep)` **if absent** (a user-inserted value during config wins); build `fixed = fixed_builder.map(|b| b.build(&mut world))`.

## Algorithms for critical paths

| Path | Steps | Big-O | Cache | Branching |
|---|---|---|---|---|
| `App::update_with_delta` (frame driver) | ① `Time::advance_with` (≤ 12 field writes, 1 Duration min, 0 muls default path ★m5) → ② margin-aware check-ticks compare → ③ event-gate compare + (swap: `update_events`, O(k) registered types) → ④ fixed branch + `fixed_advance` → ⑤ `main.run` | O(1) + the runs | App struct + 2 resource slots; all dispatcher-L1 | 3 predictable branches added (check-ticks not-taken; swap-gate; `fixed.is_some()`); zero indirect calls |
| `fixed_advance` | read `Time.delta` → `accumulate` → loop {`expend` (1 cmp + 1 sub) → `step(world)`} → store `steps_this_frame` | O(steps), steps ≤ 16 @ defaults | 1 resource slot re-borrowed per substep | 1 well-predicted loop branch per substep |
| `Time::advance_with` | `real += raw`; `d = min(raw, max_delta)`; `if paused {ZERO} else if speed == 1.0 {d} else {d.mul_f64(speed)}` (★m5: zero muls on the default path); cache f32; `elapsed += d` | O(1) | single struct | 2 branches (paused; speed==1.0) |
| Event swap (held frame) | 1 compare, no call | O(1) | — | 1 not-taken branch |
| `check_ticks_all_schedules` | pool-tick scan + per-schedule system/condition clamp | O(world) | cold | `#[cold]` `#[inline(never)]`, fires ~every `CHECK_TICK_THRESHOLD` ticks |
| `Schedule::run` | **UNCHANGED — byte-identical (P20-B1a)** | — | — | — |

Determinism note: all accumulator math is integer-nanosecond `Duration`; for a given raw-dt sequence and config, the substep count sequence and `FixedTime::elapsed` are bit-exact across runs and platforms (P20-T3).

## Multithreading model

- **No new concurrency.** Every new mutation (`Time`, `FixedTime`, event swap, check-ticks clamp, the fixed loop's own bookkeeping) executes on the dispatcher thread holding `&mut EcsMaster`/`&mut App`, strictly OUTSIDE any `pool.install` window. Workers exist only inside `Schedule::run`, which this phase calls as an opaque unit — the SCH7 apply-window proof is inherited, not re-argued.
- **No new atomics, no ordering decisions, no Send/Sync changes.** `Time`/`FixedTime` are ordinary resources: systems read them via `Res<…>` under the existing conflict graph; a user `ResMut<Time>` system serializes against readers exactly like any other resource (the driver's own writes never overlap a run).
- **Data-race freedom**: by construction — there is no instant at which a worker thread is live while any Phase-20 code mutates shared state. The plain-`&mut`-window rule (house constraint) is satisfied with zero `unsafe`.

## Soundness

1. **Zero new `unsafe`** (expected and gated: W-audit greps the diff). No raw pointers, no cached `NonNull`, no RAII guards caching world pointers (14a-F2/9.3c classes structurally absent — all state is plain fields mutated under `&mut`).
2. **Tick contract**: each `Schedule::run` keeps its asserted two-bump contract; N substeps = N independent runs (the per-system `last_run` window model already proven by the demo's 0..=5 runs/frame, R-§b). The #56 wording "next frame" → "next run of that schedule" is a doc-only sweep (R-§b).
3. **Clamp coverage** (D8): every schedule's systems/conditions are clamped from the App pass; the world-level pool scan runs there too; `set_last_check_tick` written once per firing. Standalone single-schedule users keep the internal belt.
4. **Event generations** (D6): held swaps preserve the reader generation; `EventIter` cursor semantics (Phase 12) are call-frequency-independent (R-§d); lane overflow under long holds returns `Err` loudly — bounded memory, no UB, documented.
5. **Edge cases**: raw = 0 (first frame / paused host) ⇒ delta ZERO, 0 substeps, main still runs; huge raw ⇒ clamped (≤ 16 substeps @ defaults); `relative_speed = 0` ⇒ delta ZERO (legal pause-alias); `set_timestep` mid-frame ⇒ next-frame effect (single read per `fixed_advance` call); `timestep == 0` / `max_delta == 0` / non-finite speed ⇒ setter panics (cold); `overstep` after the loop is `< timestep` (debug_assert); `fixed_steps_since_swap` saturates; `steps_this_frame` bounded by the clamp arithmetic; empty-but-created Fixed schedule ⇒ each substep costs 2 bumps + early return (documented); Fixed never created ⇒ `FixedTime` stays inert (no accumulation — the loop is gated on `fixed.is_some()`).
6. **Drop order**: unchanged reasoning — `App` is never dropped with a frame in flight (`&mut self` on every `run*`); `fixed`'s `Schedule` drop is identical in kind to `schedule`'s.
7. **Miri/wasm**: `fixed_advance` + `Time`/`FixedTime` are pure safe code traversed identically on all targets (D3); Miri suites use `update_with_delta` (no `Instant`); existing executor Miri suites are unaffected (schedule.rs untouched).

## Integration

| File | Change |
|---|---|
| `core/time/{mod.rs, time.rs, fixed_time.rs, fixed_loop.rs}` | **NEW** — D2/D3/D4; registered in `core/mod.rs` |
| `core/app/app.rs` | multi-schedule fields + config routing (D5), frame driver `update_with_delta` (D1), event gate (D6), check-ticks pass (D8), self-clock (D11), `finish()` additions |
| `core/app/mod.rs` | re-export `CoreSchedule`, `EventUpdatePolicy` |
| `core/schedule/schedule.rs` | `check_change_ticks` private → `pub(crate)` (D8); doc wording sweep "#56 frame → run" (Soundness 2). **No logic edits** |
| `core/ecs_master/ecs_master.rs` | NEW `pub(crate)` margin-aware check-ticks helper (★C1) + `#[cfg(test)]` tick setter (W4); `contains_resource` exists — no change (★m1) |
| `core/change_detection/tick.rs` | NEW `CHECK_TICK_PREEMPT_MARGIN = 4096` const (★N2); no logic edits |
| `core/state/*`, `core/events/*`, `core/schedule/*` (rest), `boyko_threadpool` | **ZERO code changes** |
| `boyko_demo/src/sim/runner.rs` | native `SimRunner` DELETED; wasm runner rewritten over `Time`/`fixed_advance` (D10) |
| `boyko_demo/src/app.rs` | `DemoApp` holds `App`; paused-sync glue; stats read `steps_this_frame` |
| `boyko_demo/src/sim/resources.rs` | `DeltaTime` DELETED |
| `boyko_demo/src/sim/systems/*` | `Res<DeltaTime>` → `Res<FixedTime>` (mechanical) |
| tests | NEW `tests/app_fixed_timestep.rs`, `tests/app_multi_schedule.rs`, `tests/miri_fixed_loop.rs` |
| benches | NEW `benches/app_overhead.rs`; existing schedule benches untouched |
| docs/book | App/Time book pages (doc-writer); internal docs sync |

## Implementation plan (waves)

1. **W0 — baselines (orchestrator, BEFORE any edit)**: release asm of `Schedule::run` + executor fns (`D:\tmp\p20_baseline_*.s`); criterion baselines for the 50-system schedule bench and any existing App bench (multi-run protocol).
2. **W1 — time module (pure)**: `Time`, `FixedTime`, `fixed_advance`; full unit + proptest coverage (P20-T1/T2/T3) with a counting closure — no App, no pool; Miri-green standalone.
3. **W2 — App storage + config**: `CoreSchedule`, lazy `fixed_builder` (takes `Arc::clone(&self.pool)` at first registration), `*_in` methods, `set_fixed_timestep`/`set_event_update_policy`, `finish()` insert-if-absent (existing `contains_resource`) + policy resolution. No driver changes yet — existing tests stay green.
4. **W3 — frame driver**: `update_with_delta` (order ①-⑤), self-clock `update`, `run_n_with_delta`, event gate, `check_ticks_all_schedules` + the schedule.rs visibility change + doc sweep; audit grep for manual `update_events` call sites under App (D6 contract note).
5. **W4 — tests**: integration matrix below; Miri suite (`update_with_delta` + `fixed_advance` paths); ★C1 race-shape clamp test (Phase 10/16.1 technique: force `last_check_tick` so the threshold crossing lands MID-FRAME between the App check and a substep/Main bump, then assert the SIBLING schedule's dormant condition got clamped — a `#[cfg(test)]` setter on `EcsMaster` mirroring the `set_current_frame_for_test` precedent if no hook exists; a naive near-threshold test would pass with the race present and is NOT sufficient).
6. **W5 — demo migration** (D10): native onto App; wasm runner onto `Time`/`fixed_advance`; `DeltaTime` deletion; `cargo check --target wasm32-unknown-unknown -p boyko_demo` green; record the net-LOC delta + behavior-delta notes (60→64 Hz, retain-vs-drop).
7. **W6 — gates + docs**: asm diff vs W0 (P20-B1a); bench gates; `PHASE-20-RESULTS.md`; internal-doc sync (SYSTEMS/FEATURE_MAP/ARCHITECTURE App + time entries); book pages.

## Metrics and validation

### Binding gates
- **P20-B1 — 0%-gate, three legs**: (a) `Schedule::run` + executor fns **asm byte-identical** to W0 (the only schedule.rs delta is a `pub(crate)` on a `#[cold]` fn + comments — any instruction delta FAILS); (b) NEW `app_overhead/empty_main`: `update_with_delta(16ms)` on an App with an empty Main, no Fixed, no events, no states — **≤ 250 ns/frame** (the declared additive envelope: Time advance + 3 branches + empty `update_events` — the empty swap path is one `wrapping_add` + an empty bitmask walk, critic-verified single-digit ns); (c) the 50-system schedule bench (untouched source, direct `Schedule::run`) within ±2% of W0. **★m2**: `benches/phase18_app.rs` IS the App-driven variant — it is RE-BASELINED: its header claim ("adds NO per-frame overhead … ±3%") is re-worded to the (b)-budget form, and its Group A moves to `run_n_with_delta` so `Instant::now` jitter leaves the timed loop; the `app_plugin.rs` parity test's "byte-identical world state" doc-comment gets a sweep (true modulo the two new resource slots).
- **P20-B2 — substep overhead**: `app_overhead/fixed_loop_1_substep` minus a bare `Schedule::run` of the same schedule ≤ 100 ns/substep (one resource re-borrow + Duration math).
- **P20-B3 — catch-up correctness (test, binding)**: scripted dt sequences: 250 ms @ 64 Hz ⇒ exactly 16 steps; 1 s raw ⇒ clamped ⇒ 16 steps + `real_delta == 1 s`; steady 60 FPS ⇒ step pattern `1,1,1,1,2,…` summing to 64/s; post-loop `overstep < timestep` always.
- **P20-B4 — determinism**: two runs over the same 1000-frame dt script (with hitches) ⇒ identical per-frame step counts and bit-equal `FixedTime::elapsed`.
- **P20-B5 — event generations**: a Fixed-schedule `EventReader` over a script with 0-substep frames observes every event exactly once (hold verified); `EveryFrame` policy variant shows the swap each frame; a Main reader never double-reads; **★M1/★m4 scripts**: a pause-spanning hold (pause N frames mid-script → unpause → backlog arrives once, nothing lost) and the cold-start trace (startup events visible at the first post-substep swap, ≈ 2 frames at 60 FPS/64 Hz).
- **P20-B6 — demo**: net-LOC delta in `boyko_demo` **negative**; native demo runs all three modes; wasm target compiles warning-free.
- **P20-B7 — suites**: full debug + release green; clippy `-D warnings`; Miri: new fixed-loop suite + existing executor/8cd/14a/14b/19 churn suites unregressed.

### Test matrix
- **Unit (W1)**: T1 `Time::advance_with` table (zero/first-frame, clamp edge, scale, pause, pause+scale, real-vs-virtual divergence, setter panics); T2 `FixedTime` (`from_hz(64.0)` == 15 625 000 ns exactly; expend sequences; `overstep_fraction ∈ [0,1)`; `discard_overstep`; `set_timestep` next-frame effect; `steps_this_frame`); T3 `fixed_advance` with a counting closure (0/1/16-step cases; accumulate-across-frames; paused ⇒ 0; no-`FixedTime` ⇒ documented `expect` panic).
- **Proptest**: arbitrary dt sequences (0..400 ms) ⇒ invariants: `steps == floor((overstep_prev + delta)/timestep)`, post-loop `overstep < timestep`, `elapsed == steps_total × timestep`.
- **Integration (W4)**: multi-schedule change detection (Main `Changed<T>` sees N fixed-substep mutations once; Fixed `Added<T>` from deferred commands visible next substep — the per-run #56 contract); event-gate hold (P20-B5 scripts); states: Mode-on-Fixed demo parity (per-substep transitions, queued-while-paused applies on resume) + state-on-Main with `in_state`-gated fixed system (frame-granular visibility, no missed transition); D8 clamp test (dormant gated condition in Fixed survives a forced threshold crossing with correct `Changed` results); `AppExit` from a fixed system; startup sees `Time` present; double-`update_events`-under-App audit test.
- **Miri**: `tests/miri_fixed_loop.rs` — `fixed_advance` + `update_with_delta` (1-thread pool, the existing Miri-executor pattern); no `Instant` anywhere in Miri paths.
- **debug_assert! invariants**: post-loop `overstep < timestep`; `timestep > 0`; `max_delta > 0`; `speed.is_finite() && speed >= 0`; in `update_with_delta`: `finished` after the first-frame branch; App pass: `current_tick` monotone vs `last_check_tick`.

## Critic Round 1 — resolutions (verdict: REVISE → folded)

The critic verified against the code: D5's pool availability (pool exists from `App::with_pool` BEFORE config — lazy `fixed_builder` takes `Arc::clone(&self.pool)` at first registration, no hole); D3's borrow choreography (`resource_mut` returns a plain `&mut R` ending at the statement — the loop compiles with zero clones/unsafe); D7's record semantics (Option cleared per pass, letter-for-letter); the manual-`update_events` call-site audit (all App-less); the empty `update_events` path cost (single-digit ns); `Duration::mul_f64` determinism (IEEE; no f32 in the accumulator chain).

**Folded remarks** (★-markers at edit sites): **C1 CRITICAL** — D8's "internal block never fires" was false (the internal block wins ~75% of threshold crossings and starves the sibling schedule's clamp → `is_newer_than` wraparound, the Phase-16.1 class); fix = `CHECK_TICK_PREEMPT_MARGIN = 4096` margin-aware App check that strictly preempts both internal blocks + the W4 race-shape test (mid-frame crossing + sibling clamp assertion). **M1** — the D6 pause-hold hazard expanded and given recorded rulings (doc pairing, release observability filed, pause-spanning P20-B5 script, demo-doesn't-dogfood note). **M2** — three unassigned demo responsibilities assigned (resource seeding via insert_resource; wasm inserts Time/FixedTime directly; wasm pause syncs Time::pause, early-return deleted). **M3** — `expend(ts)` takes the loop-entry timestep snapshot (binding signature; mid-loop `set_timestep` cannot void the bound or the gates). m1 — `contains_resource` exists, zero ecs_master.rs changes (beyond the C1 helper). m2 — phase18_app.rs named as the re-baselined App bench + Group A on `run_n_with_delta` + parity-comment sweep. m3 — App on wasm panics at RUNTIME (wording). m4 — cold-start visibility trace documented + scripted. m5 — `speed == 1.0` mul_f64 bypass. m6 — `fixed_advance` exactly-once-per-advance contract + missing-resource panic tests.

**Open-question rulings (critic's answers, accepted)**: Q1 keep auto-resolve (`WaitForFixed` iff fixed exists — Bevy parity; the alternative converts bounded-eventually-loud into silent unconditional loss), conditional on the M1 documentation package. Q2 the 250 ns envelope redefinition accepted (asm leg + ±2% leg preserve the literal guarantees; config-gated branches would be worse). Q3 doc-only D7 enforcement accepted; the book page must show the 0-substep miss concretely; tick-windowed record stays the filed follow-up. Q4 demo adopts 64 Hz (default-dogfooding; `set_fixed_hz(60.0)` one-liner documented for preservation). Q5 `steps_this_frame` as a resource field accepted (stale window unobservable by construction; no-Fixed apps read a permanent 0 — doc line). Q6 `Time::advance_with` stays `pub` (a crate-external demo needs it; `pub(crate)`+shim is illusory) + a debug guard `debug_assert!(!is_in_system_run())` catching `ResMut<Time>`-driven misuse (the EventWriter precedent, inverted). Q7 `run_n` stays self-clocked; every TIMED artifact routes through `run_n_with_delta` (phase18 Group A re-pointed in W6).
