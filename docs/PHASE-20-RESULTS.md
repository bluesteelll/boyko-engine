# Phase 20 — Results: Fixed Timestep + Multi-Schedule App

Branch `ecs`. Implementation `d8f5386` (W1-W3), test matrix + demo migration
`d1af331` (W4-W5), benches + this document (W6). Plan:
[PHASE-20-PLAN.md](PHASE-20-PLAN.md) (critic R1 REVISE → folded, ★-markers at
edit sites); research: [PHASE-20-RESEARCH.md](PHASE-20-RESEARCH.md).

## What landed

The engine has a built-in game-loop rhythm; `Schedule::run` and the executor
are byte-identical (gate P20-B1(a) below).

- **The frame driver** (`App::update_with_delta`, plan D1 Shape 2 — App-level
  composition AROUND opaque `Schedule::run`s, the demo's proven shape promoted):
  ① `Time::advance_with` (clamp → scale → pause; zero muls on the default
  speed-1.0 path, ★m5) → ② margin-aware check-ticks pass → ③ gated event swap →
  ④ fixed catch-up loop → ⑤ Main run. All inter-run work holds the dispatcher's
  own `&mut EcsMaster` with zero workers in flight; **zero new `unsafe`**
  (audited), zero new atomics, no RAII-guard-cached pointers (14a-F2/9.3c
  classes structurally absent).
- **`Time` + `FixedTime`** (D2 — two resources, NO Bevy-style generic-Time
  swap): `Time` = virtual clock (250 ms inflow clamp, `relative_speed`,
  `pause`) carrying the real (unclamped) fields; `FixedTime` = `timestep`
  (default exactly 64 Hz = 15 625 000 ns) + `overstep` (THE accumulator) +
  `overstep_fraction()` (THE interpolation alpha, D9) + `elapsed` +
  `steps_this_frame`. Integer-ns `Duration` math end to end — step counts are
  bit-deterministic (P20-B4).
- **`fixed_advance`** (D3 — ONE shared monomorphized driver): `App` calls it
  with `|w| fixed.run(w)`, the wasm demo runner with its sequential step
  closure, Miri tests with a counting closure — the identical
  accumulate/expend/clamp path on every target (the X.I D9 unified-path
  lesson). ★M3: the timestep is snapshotted once at loop entry, so a mid-loop
  `set_timestep` cannot void the substep bound (pinned by test).
- **D4 backlog policy**: inflow clamp ONLY (Bevy model) — exactly 16 substeps
  at the defaults; `overstep` is never silently discarded
  (`discard_overstep()` is the explicit escape hatch).
- **D5 multi-schedule, zero frame-path dispatch**: named `Option<Schedule>`
  fields + the closed `CoreSchedule { Main, Fixed }` enum, matched ONLY in
  config methods (`add_systems_in` / `add_systems_cfg_in` / `init_state_in` /
  `insert_state_in`); the frame driver reads direct fields — no `HashMap`, no
  label interning, no Bevy remove-run-reinsert dance. The one-arg `add_systems`
  family is unchanged (Main routing) — zero migration for existing callers.
- **D6 event-swap gate**: swap once per frame at frame start, held under
  `WaitForFixed` until the fixed schedule has run ≥ 1 substep since the last
  swap (Bevy's `ShouldUpdateMessages` lesson in boyko-native form: two App
  fields + one branch; `EventDispatcher`/`EventBuffer` code untouched). Policy
  auto-resolves at `finish()`: `WaitForFixed` iff a Fixed schedule exists. The
  ★M1 pause-hold hazard is documented on `EventUpdatePolicy` and pinned by the
  P20-B5 pause-spanning script.
- **D8/★C1 — the margin-aware all-schedule clamp pass** (the critic's central
  catch of the phase): the naive "App pass resets the counter first" design was
  FALSE — both paths read the same world counter, and the internal block of the
  first schedule to bump would win ~75% of threshold crossings, clamp only its
  own systems, reset the shared counter, and STARVE the sibling schedule's
  dormant systems past the `is_newer_than` wraparound (the Phase-16.1 class).
  Fix: `CHECK_TICK_PREEMPT_MARGIN = 4096` — the App pass fires strictly earlier
  (frame consumption ≤ 34 ticks/frame at defaults, debug_assert-pinned), then
  clamps the world scan + BOTH schedules under one tick snapshot.
  `Schedule::check_change_ticks` went private → `pub(crate)` (a `#[cold]` fn;
  codegen-neutral, verified by the asm gate). The W4 ★C1 tests target the RACE
  SHAPE itself (mid-frame crossing provenance), not just a near-threshold
  value.
- **D11 clock source**: `update()` self-clocks via `Instant` (first frame =
  ZERO delta, Bevy parity); `update_with_delta(Duration)` is the external-clock
  entry; `run_n_with_delta` is the deterministic loop every TIMED artifact
  routes through (Q7).
- **D10 demo migration (W5, committed `d1af331`)**: native + wasm rhythm onto
  `Time`/`FixedTime`/`fixed_advance`; `DeltaTime`, `FIXED_DT`, `MAX_FRAME_DT`,
  `MAX_SUBSTEPS`, and both hand-rolled accumulators DELETED; system bodies read
  `Res<FixedTime>::delta_secs()`; 3 demo integration tests migrated. Behavior
  deltas accepted per plan: 60 → 64 Hz, drop-on-cap → retain-with-clamp.
  **Recorded deviation**: `DemoApp` does NOT adopt the engine `App` — `App`
  lacks a with-world constructor (the demo needs `EcsMaster::with_capacity`);
  filed as **Phase 20.1** (`App::with_world` + the interpolated GPU mirror).
  The rhythm migration itself is complete on both targets.

## Gates

### P20-B1 — 0%-gate, three legs

- **(a) asm: PASS 3/3.** `Schedule::run` / `try_dispatch` / `apply_window`
  functions extracted from the `phase9_scheduler` bench asm are byte-identical
  (mnemonic + operand shape) to the W0 baseline
  (`D:\tmp\p20_baseline_phase9_scheduler.s`). The only schedule.rs delta this
  phase is a `pub(crate)` visibility keyword on a `#[cold]` fn + doc wording.
- **(b) empty-frame driver envelope: PASS — 14.18 ns/frame (gate ≤ 250 ns,
  17.6× headroom).** NEW `benches/app_overhead.rs`, `app_overhead/empty_main`:
  one finished App (1-thread pool), empty Main, no Fixed, no events, no states;
  `update_with_delta(16 ms)` per iteration:

  ```
  app_overhead/empty_main time:   [14.152 ns 14.175 ns 14.200 ns]
  ```

  The full declared envelope — Time advance + 3 predictable branches + the
  empty event swap + one empty `Schedule::run` — costs ~14 ns; the driver
  share net of the bare empty run (~6 ns, below) is ~8 ns.
- **(c) 50-system schedule bench vs the W0 `p20base` criterion baseline
  (untouched source, direct `Schedule::run`), gate ±2%: 4/5 legs within gate;
  the 50-exclusive leg is a MISS as written — +3.9/+4.4/+5.1% over three runs
  — attributed to binary-layout lottery, not a hot-path regression** (the
  XI-B4 protocol: measured-vs-model attribution beats gate-text literalism):

  | Bench | W6 (3 runs) | Δ vs `p20base` | Verdict |
  |---|---|---|---|
  | `phase9_schedule_run_empty` | 5.45 ns | −1.6% / −1.9% | PASS |
  | `phase9_schedule_run_50_exclusive_systems` | 4.43-4.48 µs | **+5.1% / +4.4% / +3.9%** | MISS as written — attributed below |
  | `phase9_par_iter_4096_entities` | 15.7 µs | +1.2% / +1.7% | PASS |
  | `phase9_schedule_run_two_disjoint` | 1.07-1.15 µs | −15.3% / −9.8% | improved (noisy bench) |
  | `phase9_schedule_run_one_exclusive` | 240 ns | +0.7% / +0.2% | PASS |

  **Attribution (decisive, from the same session):** the workspace carries a
  second bench of the IDENTICAL shape — `phase18_raw_schedule_run_50_systems`
  (50 trivial exclusive systems, 8-thread pool, direct `schedule.run`, source
  untouched this phase) in a different bench binary. At W0 it measured
  4.45 µs vs phase9's 4.27 µs; at W6 it measured **4.18 µs (−5.9%)** vs
  phase9's 4.43 µs (+4%). The two binaries SWAPPED order — the inter-binary
  spread of the same code shape (±4-6%) exceeds the entire gate width, and the
  asm leg (a) proves the executor instructions did not change. This is the
  documented function-alignment/binary-layout variance class (the X.B law:
  criterion lies at this scale; asm is the oracle). No code action.

### P20-B2 — substep overhead: **PASS — 5.2 ns/substep (gate ≤ 100 ns, ~19× headroom)**

`app_overhead/fixed_loop_1_substep` (empty Main + EMPTY Fixed schedule, exactly
one 15.625 ms timestep per frame, `overstep` returns to 0 every iteration):

```
app_overhead/fixed_loop_1_substep    time:   [25.243 ns 25.333 ns 25.442 ns]
app_overhead/bare_empty_schedule_run time:   [5.9696 ns 5.9940 ns 6.0199 ns]
```

Substep overhead = 25.33 − 5.99 (the extra bare run) − 14.18 (the (b) frame
envelope) = **5.2 ns** — one `resource_mut::<FixedTime>` re-borrow + integer-ns
Duration math + the swap-gate counter update, exactly the plan's model.
Equivalently: app-frame − bare-run = 19.3 ns ≤ 100 ns + the (b) envelope.

### ★m2 — `phase18_app` re-baselined

Group A moved from `run_n(1)` (self-clocked `Instant`) to
`run_n_with_delta(1, 16 ms)`; the header claim is re-worded from "NO per-frame
overhead ±3%" to the declared-envelope form (≤ 250 ns of P20-B1(b)). W6
numbers: A = 4.116 µs vs B (raw) = 4.178 µs ⇒ **A − B = −62 ns** — the full
Phase-20 frame driver is within noise of the raw loop at 50-system scale, far
inside the envelope. The criterion change column vs `p20base` (−5.9%) has **no
comparison validity** for Group A (the method changed — that is what
"re-baselined" means); the `app_plugin.rs` parity-test doc-comment got the
matching sweep (byte-identical modulo the two clock resource slots).

### P20-B3 — catch-up correctness: **PASS** (binding tests, `tests/app_fixed_timestep.rs`)

- `p20_b3_250ms_frame_is_exactly_16_steps` — 250 ms @ 64 Hz ⇒ exactly 16
  steps, zero remainder (the ★N4-corrected "exactly 16", not the ⌈…⌉ = 17
  upper bound).
- `p20_b3_huge_delta_clamps_to_16_steps_real_unclamped` — 1 s raw ⇒ clamped ⇒
  16 steps; `real_delta` carries the unclamped 1 s.
- `p20_b3_steady_60fps_expends_64_steps_per_second` — per-frame counts only
  1s and 2s; 60 frames ⇒ exactly 64 steps.
- `p20_b3_overstep_below_timestep_after_every_frame` — post-loop invariant
  over an irregular script.

### P20-B4 — determinism: **PASS**

`p20_b4_determinism_over_hitchy_script` — two runs over the same hitchy
1000-frame LCG script ⇒ identical per-frame step counts and bit-equal
`FixedTime::elapsed`.

### P20-B5 — event generations: **PASS** (`tests/app_multi_schedule.rs`)

- `wait_for_fixed_hold_loses_no_events` — a Fixed reader over a script with
  interleaved 0-substep frames observes every event exactly once.
- `pause_spanning_hold_delivers_backlog_once` — the ★M1 script: a pause
  spanning 5 frames holds the swap; on unpause the backlog arrives once,
  nothing lost, nothing doubled.
- `every_frame_policy_swaps_each_frame` — the `EveryFrame` override flows
  events through all-0-substep frames; a Main reader never double-reads.
- `cold_start_events_bounded_delay_no_loss` — the ★m4 trace: a frame-1 event
  is visible by frame 3 (bounded delay, no loss).

### P20-B6 — demo: **PASS with an honest LOC split**

- **Code LOC: negative.** The W5 diff is +114/−107 line-total, but the +114
  includes the new doc comments documenting the migrated rhythm; counting CODE
  lines the migration is net-negative (the deleted machinery: native
  `SimRunner` accumulator body, the wasm accumulator/clamp/cap loop,
  `DeltaTime`, `FIXED_DT`/`MAX_FRAME_DT`/`MAX_SUBSTEPS`). Reported honestly:
  **total +7, code negative** — the gate's intent (hand-rolled runner machinery
  net-deleted) holds.
- Native demo runs all three modes (W5); 3 demo integration tests migrated.
- **wasm32 compiles warning-free — and the gate caught a pre-existing break:**
  `boyko-ecs` had NOT compiled on wasm32 since Phase X.G — two ungated
  pointer-width const asserts in `inland_store.rs` (`SLOT_SIZE == 16` and
  granule divisibility) fail on 32-bit targets; both are now
  `#[cfg(target_pointer_width = "64")]`-gated with a why-comment (wasm uses
  the fallback arm; no X.G commit boundary compiles for wasm, so there is no
  clean bisect point — the gate matrices of X.G/X.H/X.I simply never included
  a wasm check. Coverage-gap lesson recorded below.)

### P20-B7 — suites: **PASS**

- W4 matrix: `app_fixed_timestep` 9/9, `app_multi_schedule` 8/8, ★C1 in-file
  3/3 (provenance / boundary-walk / behavioral — the race-shape tests the plan
  demanded, not a naive near-threshold value).
- Full workspace: debug suite green at W4 close and re-verified green at W6
  before the close-out commit (`cargo test --workspace --all-targets`);
  clippy `-D warnings` clean; `cargo check --workspace --all-targets` clean.
- **Miri (Tree Borrows)**: `miri_fixed_loop` M-P20-1 (pure
  `fixed_advance`/clock math, no pool) TB-clean in 0.63 s. The App-driver
  variant is `#[ignore]`d: it constructs a real executor frame under the
  interpreter, the windows-gnu Miri executor wall-time class (> 40 CPU-min;
  the `miri_phase_bugfix_56` precedent). The driver's own new code is pure
  safe Rust traversed identically by the fast test; the executor itself is
  covered by the Phase-9.x Miri/loom suites, unchanged this phase.

## Unplanned findings

1. **The wasm32 build of `boyko-ecs` was broken since Phase X.G** (found and
   fixed by the P20-B6 wasm leg; details above). Lesson: a target that ships
   (the web demo) must sit in EVERY phase's gate matrix, not only in the phase
   that introduced it — X.G/X.H/X.I all ran their gates without a
   `--target wasm32-unknown-unknown` check and the regression sailed through
   three phases.
2. **Plain `&mut T` query access elides tick stamping** (the Phase-12.5 NCD
   const-elision working as designed): a Fixed-substep mutation through
   `&mut T` is INVISIBLE to a Main `Changed<T>` reader; mutation through
   `Mut<T>` stamps. Pinned by `main_changed_sees_substep_mutations_once`
   (which had to use `Mut<Wobble>` — the test comment documents the semantics
   for the next reader).
3. **The Phase-17 transition pass runs at the START of a schedule run**, so a
   `NextState` set during Main frame N applies at Main run N+1, and a
   Fixed-substep `in_state` reader sees it at frame N+2 (Fixed runs before
   Main, D1 order). Frame-granular, Bevy-parity latency — pinned by
   `state_on_main_gates_fixed_system_frame_granular` so the contract is a
   test, not folklore.

## Honest residuals

1. **Phase 20.1 (filed)**: `App::with_world` (the demo's `DemoApp` still
   hand-drives `EcsMaster` + pool because `App` cannot adopt a pre-built
   world); the interpolated GPU mirror (`sync_gpu_instance` consuming
   `overstep_fraction()` — D9 ships alpha only); release-build observability
   for event-hold saturation (★M1 ruling (b): debug-only `diagnostics()` is
   the only counter today).
2. The `#[ignore]`d Miri App-driver test (wall-time class, above) — runnable
   on a Linux Miri host where the executor interpretation cost is known to be
   ~40× lower; not a soundness gap (zero new unsafe, executor unchanged).
3. **Tick-windowed `StateTransitionRecord`** (the D7 follow-up): cross-schedule
   `on_enter`/`on_exit` conditions remain structurally unsound under the
   Option-cleared-per-pass record; the same-schedule contract is documented and
   the tick-window redesign (one `Option<Tick>` per record + condition rewrite)
   is the filed Phase 20.x enhancement.
4. P20-B1(c)'s 50-exclusive leg MISS-as-written (binary-layout attribution
   above) — if it ever needs to be retired conclusively, the probe is a
   PGO/section-alignment A/B or `-C link-arg=/ALIGN` sweep; not worth the
   machine time against an asm-identical hot path.

## Process notes

- The architecture-critic's ★C1 re-derivation (the App pass does NOT
  automatically preempt the internal threshold blocks; a margin is required)
  was the catch of the phase — the naive design would have starved sibling
  schedules' dormant-system clamps at ~75% of threshold crossings, a
  wraparound-class correctness bug surfacing after ~50 days of uptime.
  The W4 tests encode the race SHAPE (mid-frame crossing + provenance
  assertion), per the critic's test note that a naive near-threshold test
  would pass even with the race present.
- The W5 dogfooding gate did exactly what it exists for: it measured the API
  against the demo's real runner (machinery net-deleted — D1-D4 validated) AND
  surfaced the missing `App::with_world` constructor honestly instead of
  forcing the migration through a worse shape.
- Criterion at the ns scale was again the unreliable narrator (the +4% vs −6%
  swap of two identical-shape benches across binaries); the asm gate carried
  the verdict. Multi-run + cross-binary control is now the cheap standard
  protocol for any ±2% gate.
