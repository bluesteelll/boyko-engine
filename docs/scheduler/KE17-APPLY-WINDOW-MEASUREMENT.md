# KE17 — sizing the apply-window barrier

Measurement pass. This file records what the mechanism is at the current lines,
what instrument was built, and what it read. The design verdict is the sibling
file [KE17-APPLY-WINDOW-DESIGN.md](KE17-APPLY-WINDOW-DESIGN.md); it is allowed
to conclude "not worth building" on these numbers and it does not.

Worktree `D:/wt/threadpool`, branch `feat/threadpool-ke16`, HEAD `59009f8a`.
No production code was changed by this pass. It added one bench,
`crates/boyko_ecs/benches/ke17_apply_window.rs`, and six lines to
`crates/boyko_ecs/Cargo.toml`.

---

## 1. The mechanism, at the current lines

Re-read rather than trusted. `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`
is MODIFIED in the working tree by the concurrent KE16 workflow, so the KE16
probe's line numbers no longer land; its description does.

| Thing | KE16 probe said | Working tree today |
|---|---|---|
| the gate | `:602` | **`:621-623`** |
| `apply_window_drain` | `:722` | **`:719`**, the `running` clear at **`:745`** |
| `try_dispatch_ready` | `:967-1326` | **`:990`**, pred check `:1022`, conflict check `:1026` |

The drift is a doc comment: `git diff` on that file is a `PARK_TIMEOUT`
rationale block plus a reworded inline comment. **The gate's logic is
byte-identical to HEAD.** The probe's account of the mechanism is correct.

### When a finished system's `running` bit clears

`Schedule::executor_main_loop` (`:564`) runs one loop turn per dispatch round.
The turn opens with:

```rust
let pending  = completion.pending_load(Ordering::Acquire);          // :621
let running  = self.executor_scratch.running.count_ones(..);        // :622
if pending > 0 && (pending == running || running == 0) {            // :623
    let world_mut: &mut EcsMaster = unsafe { cell.world_mut() };
    self.apply_window_drain(world_mut, completion);                 // :632
}
```

`running` counts systems dispatched-and-not-yet-drained; `pending` counts those
that have finished and pushed a completion. `pending == running` is the
statement **"every system currently in flight has finished"** — quiescence.
Only then does `apply_window_drain` (`:719`) pop completions and, per system,
in this order: clear `running` (`:745`), call `system.apply(world)`, drain the
deferred hook queue, set `completed`, and decrement each successor's
`pred_remaining`.

The only other site that clears a `running` bit is the inline-exclusive path
(`:1177`), which runs solo on the dispatcher by construction. There is no third.

**Consequence.** With four systems in flight and one finished, `pending = 1`,
`running.count_ones() = 4`, the gate is false, and the finished system's
`running` bit stays set, its `completed` bit stays clear, and its successors'
`pred_remaining` stays above zero — until the LAST member of the in-flight set
finishes.

### What that blocks

Two distinct checks in `try_dispatch_ready`, and the distinction matters
because they catch different systems:

* **The predecessor path** (`:1022`) — `pred_remaining[i] != 0`. A successor of
  a finished-but-undrained system cannot be dispatched.
* **The conflict path** (`:1026`) — `bitset_intersects(conflict_bits[i], running)`.
  A system that conflicts with a finished-but-undrained system cannot be
  dispatched, *even if it has no predecessors at all and was ready from the
  first round*.

The second is the one the ticket's framing misses, and on the engine's own
Main schedule it is the dominant one (§5).

Dispatch is not fully blocked in that window: a system that neither conflicts
with nor succeeds anything still-marked can start, and doing so raises the
quiescence bar (`running` grows). System assignment remains dynamic; the round
STRUCTURE is what is static, exactly as KE16 recorded.

---

## 2. The instrument

`crates/boyko_ecs/benches/ke17_apply_window.rs` (criterion, `harness = false`).

Six schedules of spin-bodied systems with deliberately unequal costs. Each body
stamps `Instant`-derived start and end nanoseconds and its worker id into
statics; the frame's real timeline is then reconstructed exactly. Bodies never
sleep. Conflicts are real — a system in conflict class `k` takes
`Query<&mut Kk>` (component ids 496-499), so the kernel's own `ConflictGraph`
produces the conflicts, not a table in the bench.

Four numbers per frame, medians over 61 frames after 8 warm-up frames:

* **`span`** — measured, `max(end)` relative to a base taken immediately before
  `Schedule::run`.
* **`ideal_sim`** — greedy index-order list schedule over the MEASURED per-system
  durations, same DAG, same conflicts, same worker count, with the barrier
  removed (each completion retires the instant it lands) and zero dispatch
  latency.
* **`barrier_sim`** — the same simulation with the real gate semantics of `:623`
  replayed (a completion retires only at quiescence), zero dispatch latency.
* **`idle_pred` / `idle_nopred`** — wall time during which at least one lane was
  idle AND a not-yet-started system was legally startable (predecessors finished
  by wall clock, no conflicting system executing), split by whether that system
  has a predecessor. Split by STRUCTURE, not by cause — see §4.

From these: `barrier/model = (barrier_sim - ideal_sim) / barrier_sim` is the
barrier's own share of a latency-free frame, and `residual = (span - barrier_sim) / span`
is everything the model does not explain (dispatcher scan, park/unpark, load).

### Why not just report idle time

Idleness alone is not a finding — a lane with nothing to do is not a defect.
`idle_*` counts only idleness *with legal work available*. Even that is a joint
counter, so the attribution is carried by the two simulations, which differ in
exactly one rule.

### Separation from KE16 defect A

Defect A is "a task spawned from inside a WORKER lands in that worker's local
injector and no sibling polls it". **No body in this harness spawns anything**:
each is one leaf task that busy-waits and touches no `Query` rows, issues no
`par_iter`, and queues no `Commands`. Defect A has no surface here; whatever the
instrument reports cannot be it.

### The control

`s4_wide_control` — 16 independent systems, unequal costs, no edges and no
conflicts. The barrier can hold nothing there. Across all ten readings
(2 worker counts × 5 runs) it read **`barrier/model = 0.00%` and
`idle_pred = 0.0 µs`, without exception.** A non-zero reading there would have
condemned the instrument, not the scheduler.

---

## 3. The numbers

⚠ **Every timing below was taken on the owner's workstation while it was
BUSY** — `\Processor(_Total)\% Processor Time` sampled 40-52% throughout, two
other agent workflows were compiling in the same worktree, and one of them was
mid-edit on `crates/boyko_threadpool/src/*` (1 800 lines added to `worker.rs`
alone). Absolute wall-clock is therefore not a usable datum here: the same
criterion row moved 2.9x between two runs an hour apart
(`s1/w4`: 845 µs run B, 2.48 ms run D).

Five full runs: **A** and **B** (guarded timer, full criterion settings),
**U** (unguarded timer, happened to catch the quietest window), **C** and **D**
(guarded, short criterion settings). `barrier/model` is reported for all five
because it is the load-independent column; the others are reported from the
quiet runs and flagged.

### 3.1 The barrier's own share — `(barrier_sim - ideal_sim) / barrier_sim`

Structural: computed from measured durations and the real DAG/conflict sets,
with dispatch latency and load removed by construction.

| Shape | W | A | B | U | C | D | **range** |
|---|---|---|---|---|---|---|---|
| `s1_few_long_uneven` | 4 | 47.46 | 47.49 | 47.49 | 47.48 | 47.34 | **47.3-47.5 %** |
| `s1_few_long_uneven` | 16 | 47.46 | 47.48 | 47.49 | 47.46 | 47.38 | **47.4-47.5 %** |
| `s2_many_short_uneven` | 4 | 13.37 | 13.40 | 13.38 | 13.38 | 13.53 | **13.4-13.5 %** |
| `s2_many_short_uneven` | 16 | 0.09 | 0.03 | 0.00 | 0.15 | 0.06 | **~0 %** |
| `s3_chain_beside_long` | 4 | 36.40 | 36.39 | 36.37 | 36.37 | 36.86 | **36.4-36.9 %** |
| `s3_chain_beside_long` | 16 | 36.51 | 36.61 | 36.39 | 36.47 | 36.53 | **36.4-36.6 %** |
| `s4_wide_control` | 4 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | **0.00 %** |
| `s4_wide_control` | 16 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | **0.00 %** |
| `s5_narrow_conflict` | 4 | 11.83 | 11.78 | 11.78 | 12.39 | 11.86 | **11.8-12.4 %** |
| `s5_narrow_conflict` | 16 | 12.55 | 12.69 | 12.52 | 12.50 | 12.73 | **12.5-12.7 %** |
| **`s6_engine_main_like`** | **4** | 8.14 | 8.12 | 8.12 | 8.16 | 8.62 | **8.1-8.6 %** |
| **`s6_engine_main_like`** | **16** | 8.12 | 8.12 | 8.12 | 8.14 | 8.12 | **8.1 %** |

Spread across five runs on a loaded machine is under one percentage point on
every row. That is what a structural datum looks like.

### 3.2 The measured frame, in the quietest run (U)

`residual` here is 2.0-14.0 %, i.e. the barrier model accounts for 86-98 % of
the measured frame. This is the validation of §3.1, and the row to read for the
*measured* share of frame time.

| Shape | W | span µs | ideal µs | barrier_sim µs | barrier/span | residual | max in-flight |
|---|---|---|---|---|---|---|---|
| `s1_few_long_uneven` | 4 | 821.9 | 420.2 | 800.2 | **46.2 %** | 2.6 % | 4 |
| `s1_few_long_uneven` | 16 | 818.0 | 420.2 | 800.2 | **46.5 %** | 2.2 % | 4 |
| `s2_many_short_uneven` | 4 | 398.9 | 325.1 | 375.3 | **12.6 %** | 5.9 % | 4 |
| `s2_many_short_uneven` | 16 | 352.0 | 325.1 | 325.1 | **0.0 %** | 7.6 % | 12 |
| `s3_chain_beside_long` | 4 | 730.9 | 400.1 | 628.8 | **31.3 %** | 14.0 % | 2 |
| `s3_chain_beside_long` | 16 | 729.1 | 400.0 | 628.8 | **31.4 %** | 13.8 % | 2 |
| `s4_wide_control` | 4 | 234.7 | 205.2 | 205.2 | **0.0 %** | 12.6 % | 4 |
| `s4_wide_control` | 16 | 204.0 | 200.0 | 200.0 | **0.0 %** | 2.0 % | 14 |
| `s5_narrow_conflict` | 4 | 371.1 | 300.2 | 340.3 | **10.8 %** | 8.3 % | 4 |
| `s5_narrow_conflict` | 16 | 338.9 | 280.1 | 320.2 | **11.8 %** | 5.5 % | 6 |
| **`s6_engine_main_like`** | **4** | 1240.8 | 1100.1 | 1197.3 | **7.8 %** | 3.5 % | 3 |
| **`s6_engine_main_like`** | **16** | 1231.0 | 1100.1 | 1197.3 | **7.9 %** | 2.7 % | 3 |

Measured `barrier/span` lands within 1.3 points of the structural
`barrier/model` on every shape except `s3` (where the 20-round chain also pays
20 dispatcher round-trips, which the model prices at zero).

### 3.3 The mechanism, caught in a single frame

`s1_few_long_uneven`, W=16, one raw timeline (ns, relative to the first body
start; `lane` is the worker id the body ran on):

```
sys 0  lane 0  start      0   end 400200      head, 400 us
sys 2  lane 2  start      0   end  20100      head,  20 us
sys 3  lane 1  start   1000   end  21000      head,  20 us
sys 1  lane 3  start   2900   end  22900      head,  20 us
sys 4  lane 6  start 432600   end 452700      successor of sys 0
sys 5  lane 7  start 438900   end 839000      successor of sys 1
sys 7  lane 8  start 439500   end 839600      successor of sys 3
sys 6  lane 9  start 442100   end 842100      successor of sys 2
```

`sys 5`, `sys 6` and `sys 7` had their predecessors finished at 21-23 µs. Twelve
of sixteen lanes were idle. They started at 439-442 µs — the instant the 400 µs
head `sys 0` was drained. **Each was held ~417 µs on an 800 µs frame by a
`running` bit belonging to a system it has no relationship with.**

The 32-42 µs between `sys 0`'s end and the successors' start is the dispatcher
round-trip; that is the `residual`, and it is an order of magnitude smaller.

### 3.4 Criterion wall-clock, for the record

Median of 20 samples. Reported only to show that it is NOT the datum: the same
rows two runs apart.

| Bench | run A | run B |
|---|---|---|
| `s1_few_long_uneven/w4` | 1.90 ms | 845 µs |
| `s2_many_short_uneven/w4` | 404 µs | 550 µs |
| `s3_chain_beside_long/w4` | 758 µs | 748 µs |
| `s4_wide_control/w4` | 2.21 ms | 247 µs |
| `s5_narrow_conflict/w4` | 1.32 ms | 391 µs |
| `s6_engine_main_like/w4` | 4.86 ms | 1.29 ms |
| `s1_few_long_uneven/w16` | 2.02 ms | 2.48 ms |
| `s2_many_short_uneven/w16` | 1.12 ms | 1.57 ms |
| `s3_chain_beside_long/w16` | 2.65 ms | 3.04 ms |
| `s4_wide_control/w16` | 569 µs | 1.17 ms |
| `s5_narrow_conflict/w16` | 355 µs | 1.88 ms |
| `s6_engine_main_like/w16` | 1.25 ms | 1.42 ms |

Up to 8.9x apart on one row (`s4/w4`). Anyone quoting these as a barrier
measurement is quoting the other two workflows' compile load.

---

## 4. What the counter can and cannot attribute

`idle_pred` isolates the **predecessor path** and matches the model where the
shape is dominated by it: `s1` W=16 run U reads `idle_pred = 391.5 µs` against a
model prediction of `800.2 - 420.2 = 380.0 µs` — 3 % apart.

`idle_nopred` does NOT isolate the pool, and calling it that would have been a
silent wrong answer. A system with no predecessors can still be barrier-blocked
through the **conflict path**: its `conflict_bits` intersect a `running` bit
that a finished sibling has not had cleared. `s6` reads `idle_nopred = 1126.7 µs`
(90.8 % of a 1 240.8 µs frame) in the quiet run **while `residual` is 3.5 %** —
the model reproduces the frame, so that idleness is the barrier, not the pool.

The pool + dispatch floor is read off the control, which has neither edges nor
conflicts: `s4` reads `idle_nopred = 23.5 µs / 20.6 µs` (~10 % of a 200 µs
frame) in the quiet run. That is the number to subtract before reading
`idle_nopred` as a barrier cost anywhere else.

---

## 5. Do the synthetic shapes resemble the engine's own schedules?

The reference is what `boyko_app` and the plugins actually register, read at
`crates/boyko_app/src/plugins.rs:617-708` and the plugin `build` bodies.

**Counts.** 97 distinct `add_system` call sites outside `boyko_ecs`:
`boyko_render` 22, `boyko_demo` 20, `boyko_physics` 18, `boyko_app` 13,
`boyko_ui` 12, `aether_lang` 7, `boyko_scene` 5, `boyko_input` 2. A single frame
runs a subset across two schedules (Main and Fixed).

**Structure — Fixed / physics.** `crates/boyko_physics/src/plugin.rs:531-676`
registers a strict serial chain: `sync_transform_to_body` → `integrate` →
`gather` → `select_broadphase` → `broadphase` → `narrowphase` (→ `narrowphase_sdf`)
→ `build_graph` → `solve` → (`soft_step*`) → `apply` → `sync_body_to_transform`,
each pinned `.after` its predecessor. **The barrier costs nothing on a serial
chain**: one system in flight means `pending == running == 1` the moment it
finishes, so the gate fires immediately. Physics is not where this lever lives.

**Structure — Main / render.** `plugins.rs:617-708` is a shallow, wide DAG:
`sync_instance_model_cols` (the "pack") is a root; `gather_shadow_casters` is
`.after(pack)`; `sync_csm_light_gate`, `reduce_caster_bounds` and
`sync_punctual_light_gate` are `.after(casters)`; `sync_ssao_light_gate`,
`sync_sv0_light_gate`, `sync_cluster_light_gate` and `snap_apply` are roots; and
`gather_mesh_draws` is `.after(pack).after(snap)`. `LightingPlugin`'s
`collect_lights` joins by set (`LightCollectSet`) after the gate bridges.

The cost distribution is extremely unequal and structurally so, not by accident:
`sync_instance_model_cols`, `gather_shadow_casters` and `gather_mesh_draws` are
O(entities) per-row loops; the five `sync_*_light_gate` systems are O(1)
resource pokes. And the gates all write `LightingConfig`, so **they conflict
with each other and with `collect_lights`** — the kernel serialises them.

`s6_engine_main_like` is that graph, node for node and edge for edge, with the
gates in one shared write class. It is the closest of the six to the engine and
it is the one that reads **8.1 %**.

**What is NOT modelled.** `propagate_transforms` (`boyko_scene`) is
`fn(&mut EcsMaster)` — exclusive, so it runs inline on the dispatcher and is
outside this mechanism entirely. `boyko_ui`'s discovery/apply pairs and
`boyko_input`'s two systems were not modelled. Absolute costs in `s6` are
invented (600/300/500 µs for the three gathers, 1 µs for the gates, 80 µs for
`collect_lights`); the RATIOS are what the 8.1 % rests on, and a scene with
fewer entities moves the long systems down and the share up, not down.

---

## 6. Reproducing

```powershell
$env:PATH = "$HOME/.cargo/bin;$env:PATH"; $env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-gnu"
cargo bench -p boyko-ecs --bench ke17_apply_window
```

The `KE17 | ...` rows go to stderr, one per shape × worker count.
`BOYKO_KE17_TIMELINE=1` adds the raw per-system timeline of one frame per
configuration (that is where §3.3 comes from).
`BOYKO_KE17_NO_TIMER_GUARD=1` drops the `timeBeginPeriod(1)` guard the bench
otherwise holds to match the shipped host (`boyko_app::timer_resolution`).

⚠ The guarded/unguarded contrast in this pass is **confounded by machine load**
and no claim is made from it: the quietest of the five runs happened to be the
unguarded one. The guard is held by default because the shipped host holds it,
not because this pass measured a difference.

## 7. Manifest change

`crates/boyko_ecs/Cargo.toml` — appended six lines (comment + `[[bench]]` +
`name` + `harness`) at the end of the file. That manifest already carried an
uncommitted `[[bench]] ke16_par_iter_in_system` block from the concurrent KE16
workflow; this addition sits after it and touches nothing else.

## 8. KE17 D10 — what the SPLIT recovers (added after D3 landed)

§3.1's `barrier/model` is the UPPER BOUND: what the barrier costs, not what
removing it from the systems that provably carry nothing would recover. A
`Commands`-carrying system keeps its barrier under the split, so the
recoverable part is strictly smaller and had to be measured on its own. This
section is that measurement. It was allowed to cancel the feature; it does not.

### 8.1 The instrument change

`simulate`'s `barrier: bool` became a three-variant `RetirePolicy`
(`Immediate` / `Quiescence` / `Split(&[bool])`), so `Split` cannot be spelled
without the mask it is meaningless without. `Split` retires a completion the
instant it lands when its `may_defer` bit is CLEAR, and holds it to quiescence
when SET — with the early release running BEFORE the quiescence test on each
loop turn, because clearing a `running` bit early also lowers the quiescence
bar for whatever is still held, which is what the real bit-clear would do.

Two new columns per row: `split/model = (barrier_sim - split_sim) /
barrier_sim`, and `worst/model`, the same ratio under the WORST single-node
mask (the max over every one-hot mask the shape admits). The second needs no
mask to be believed.

### 8.2 Where the mask comes from — read, not invented

* **`s6` and `s3` stand for NAMED systems**, so their masks are read off those
  signatures one by one. On `s6`, all eleven were re-read this pass:
  `sync_instance_model_cols`, `sync_ssao_light_gate`, `sync_sv0_light_gate`,
  `sync_cluster_light_gate`, `gather_shadow_casters`, `sync_csm_light_gate`,
  `reduce_caster_bounds`, `sync_punctual_light_gate`, `collect_lights` and
  `gather_mesh_draws` take `Query` / `Res` / `ResMut` / `NonSendRes` only.
  **`snap_apply` (`snap_interpolation.rs:124`) is the only one with a
  `Commands` parameter** — the design's claim, confirmed rather than trusted.
  `s3`'s chain is the twelve links of `boyko_physics/src/plugin.rs:531-676`;
  none of them takes `Commands`.
* **`s1`, `s2`, `s4`, `s5` stand for a KIND**, so their masks come from a
  census instead: of the 72 distinct function names registered through
  `add_system` across the seven system-registering crates, 68 resolve to a real
  `fn`, and exactly FOUR take `Commands` — `snap_apply`, `visibility_sync`,
  `apply_refcount_deltas`, `validate_asset_refs`. Every one of the four is a
  per-entity apply/reconcile step. No gather, no resolve, no
  `sync_*_light_gate`, no solver stage and no sync-out takes one, and those are
  the roles these four shapes model. Their masks are all-clear, which makes
  `split_sim == ideal_sim` on those rows BY CONSTRUCTION — the row states
  "these shapes model systems that carry no deferred payload" and is not
  independent evidence about the split. `worst/model` is what those rows are
  read for.

The mask is not merely tabulated. `build_schedule` now gives the marked nodes a
`Commands`-carrying body and asserts the table against the KERNEL's own
`Schedule::may_defer` (the D3 predicate) on every build, so the model's mask is
the engine's answer rather than a second opinion about it. It is asserted as a
population count, not per index: `kahn_topological_sort` breaks ties from a
FIFO ready queue, so any shape with an edge comes out permuted, and
`Schedule::may_defer` is indexed by post-topo position while every other array
in the bench uses the shape's own numbering.

### 8.3 The numbers

Two runs, both on the loaded box; the structural columns agree to within 0.05
points on every row, which is the same load-independence §3.1 reports — and run
1 makes the point unusually well: on `s6` W=4 its measured `span` was **4 375.9
µs** against run 2's **1 225.2 µs**, a 3.6x wall-clock move on the same row,
while `barrier/model` and `split/model` moved from 8.16 % to 8.11 %. Cells
with two figures are run 1 / run 2.

| Shape | W | `barrier/model` | **`split/model`** | `worst/model` | mask |
|---|---|---|---|---|---|
| `s1_few_long_uneven` | 4 | 47.48 / 47.49 | **47.48 / 47.49** | -2.52 | 0/8 |
| `s1_few_long_uneven` | 16 | 47.48 / 47.48 | **47.48 / 47.48** | -2.52 | 0/8 |
| `s2_many_short_uneven` | 4 | 13.38 / 13.35 | **13.38 / 13.35** | 13.35 | 0/24 |
| `s2_many_short_uneven` | 16 | 0.03 / 0.03 | **0.03 / 0.03** | 0.00 | 0/24 |
| `s3_chain_beside_long` | 4 | 36.37 / 36.38 | **36.37 / 36.38** | 0.00 | 0/21 |
| `s3_chain_beside_long` | 16 | 36.38 / 36.37 | **36.38 / 36.37** | 0.00 | 0/21 |
| `s4_wide_control` | 4 | 0.00 | **0.00** | 0.00 | 0/16 |
| `s4_wide_control` | 16 | 0.00 | **0.00** | 0.00 | 0/16 |
| `s5_narrow_conflict` | 4 | 11.76 / 11.78 | **11.76 / 11.78** | -5.88 | 0/12 |
| `s5_narrow_conflict` | 16 | 12.52 / 12.52 | **12.52 / 12.52** | -12.52 | 0/12 |
| **`s6_engine_main_like`** | **4** | 8.16 / 8.11 | **8.16 / 8.11** | 6.89 / 6.86 | **1/11** |
| **`s6_engine_main_like`** | **16** | 8.16 / 8.11 | **8.16 / 8.11** | 6.90 / 6.86 | **1/11** |

Absolute makespans on the deciding row (`s6`, W=4, run 2): `ideal_sim`
1100.1 µs, `split_sim` **1100.1 µs**, `barrier_sim` 1197.2 µs.

### 8.4 The verdict the decision rule produces

The rule was: if `split_sim` lands within 2 percentage points of `barrier_sim`
on the engine-like shape, the recommendation flips to close-KE17-with-a-number
and the design is frozen rather than built.

**It does not. `split_sim` sits 8.1 % BELOW `barrier_sim` on `s6` — the split
recovers 100.0 % of the measured barrier cost, at both worker counts, in both
runs. The lever is real and the ticket stays open.**

The reason it recovers everything is visible in the shape: `s6`'s one deferring
node is `snap_apply`, which is 30 µs long, sits in no conflict class, and has no
successor in the modelled graph. Holding it to quiescence costs nothing, while
the five O(1) `sync_*_light_gate` bridges — which are what the 8.1 % is made of
(§2.1) — retire immediately. §3 of the design file predicted exactly this
("the predicate that makes the fix sound is the predicate that makes it
profitable"); this is the number behind that sentence.

### 8.5 Two findings the split column produced

**(a) The worst single node on `s6` still recovers 6.9 %.** If any ONE of the
eleven were to gain a `Commands` parameter tomorrow, the split's return falls
from 8.1 % to 6.9 %, not to zero. The reading is not balanced on the mask.

**(b) The split can make a HELD system LATER than the barrier does, and three
rows measured it.** `worst/model` is NEGATIVE on `s1` (-2.5 %) and `s5`
(-5.9 % at W=4, -12.5 % at W=16). The mechanism is not a modelling artefact and
is worth the step-2 designer's attention:

> Under today's barrier, quiescence arrives promptly because a blocked schedule
> stops dispatching. Under the split, the early releases keep finding work, new
> systems join the `running` set, and the quiescence the HELD system is waiting
> for recedes. On `s1` with a one-hot mask on a 20 µs head, the held system's
> drain moves from 400 µs (barrier) to 420 µs, and its 400 µs successor
> therefore ends at 820 µs instead of 800 µs.

So the split is not a Pareto improvement per system: it trades earlier
retirement for the non-deferring majority against a possibly later apply for
the deferring minority. On the engine's own shape today that trade is free
(1 of 11 nodes deferring, and that node on nobody's critical path), but a
schedule that grew a `Commands` system onto a critical path would want the
second half of the design — `ApplyDeferred` insertion, Phase 9.1's other
unlanded prerequisite — rather than the split alone.

This is also why `simulate` does NOT assert `ideal <= split <= barrier`: these
are greedy list schedules, releasing work earlier can change what the scheduler
picks first (Graham's anomaly), and an assert would have turned a real,
reportable property into a crash.

### 8.6 What is NOT claimed here

No wall clock. Every number above is structural — the same measured per-system
durations replayed through models differing in exactly one rule — for the
reason §3.4 gives: the criterion column on these rows moved up to 8.9× between
runs on this box, and the box was loaded again for this pass.

### 8.7 A divergence in `s6`'s edge table, reported and deliberately NOT fixed

While re-reading the eleven systems, two ordering edges in `S6_EDGES` were found
to disagree with the bench's own node comment and with `plugins.rs`:

* `gather_mesh_draws` is registered `.after(pack).after(snap)`, and the comment
  says "after 0 and 1", but the table carries only `(0, 10)`.
* `collect_lights` is joined by `sync_sv0_light_gate` and
  `sync_cluster_light_gate` through `LightCollectSet` (`sync_ssao_light_gate`
  carries no edge, by its own comment), so the edges into node 9 should be
  `(3, 9)` and `(4, 9)`; the table carries `(2, 9)` and `(3, 9)`.

The table was left alone on purpose: changing the shape would move
`barrier/model` and break comparability with §3.1's recorded 8.1 %, which is
the baseline this section's split column is read against. Neither edge can
flip the verdict, and the reason is arithmetic rather than judgement: node 10's
other predecessor (node 0, 600 µs) finishes long after node 1 (30 µs), so
`(1, 10)` cannot move node 10's start; and nodes 2, 3 and 4 are all in the same
write class, run one per round in index order, and complete within 3 µs of each
other, so swapping which two of them gate node 9 moves nothing. Fixing the table
belongs to whoever next revises the shape, with a fresh baseline.
