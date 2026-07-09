> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 16 Plan — Run Conditions (`.run_if(predicate)`)

Lead architect deliverable. Save to `docs/PHASE-16-PLAN.md`. All claims grounded in the `ecs`-branch code (file:line). No implementation code — pseudo-code/signatures only.

---

## §0 — Round 2 Patches (resolve critic Round 1; READ FIRST, supersede on conflict)

> Critic R1 verdict: REVISE, **no CRITICAL**. The 0%-gate (`has_condition` bitset,
> `SystemBox` untouched) and race-freedom (`running == 0` ⟹ no live worker, via the
> dispatcher setting `running` before spawn `schedule.rs:484` and clearing after drain
> `:331`) were VERIFIED CORRECT. These patches close the must-fix precision/guard items.

- **P1 (HIGH, Proof-a precision).** The race-freedom proof must state the drain clears
  *exactly* the gate-counted `running` bits: the apply-window gate (`schedule.rs:268-270`)
  fires only when `pending == running.count_ones()` OR `running == 0`; `apply_window_drain`
  (`:316-384`) reads `target = pending` (`:317-323`) and pops exactly `target` completions,
  each clearing one distinct `running` bit (`:331`). ⟹ post-drain `running.count_ones() == 0`.
  So condition eval (gated on `running == 0`) provably runs with NO worker holding a cell.
- **P2 (HIGH, cascade).** Single-pass skip settlement holds ONLY for *contiguous* skip
  chains. A `should_run == true` conditioned system mid-chain is dispatched (not skipped);
  its successors' conditions are evaluated on the NEXT post-`apply_window_drain` iteration
  (the existing rhythm), not in the current pass. Add test `skip_run_skip_cascade`
  (`a.run_if(false) → b.run_if(true) → c.run_if(false)`: assert b runs once, c never, frame
  terminates) — the all-skip `skip_cascade_single_pass` does not exercise the mixed rhythm.
- **P3 (MEDIUM, sizing landmine).** `system_conditions` / `system_gating_sets` /
  `has_condition` are sized `n` today; `systems` is `n_final` (post-`insert_sync_points`).
  `insert_sync_points` (`schedule_builder.rs:547-552`) is currently the IDENTITY stub, so
  `n == n_final`. Add `debug_assert_eq!(n, n_final, "Phase 16 condition-array indexing
  assumes identity sync-insertion; revisit when insert_sync_points injects nodes")` at the
  take-loop, and a comment. (When Phase-9.1 sync-insertion lands, size these off `n_final`
  and index off the post-sync descriptor order.)
- **P4 (MEDIUM, tick footgun — NOT "deferred").** A read-only `Changed<T>`/`Added<T>`/`Ref<T>`
  condition COMPILES and passes the write-only `debug_assert` (it declares reads, no writes),
  but `run_condition` never calls `set_change_ticks`, so its meta stays at the `initialize`
  sentinel (`function_system.rs:199-208`) ⟹ it silently reports **all-changed (always true)**.
  This is an UNGUARDED FOOTGUN, not an enforced boundary. Document it as such in §1 + the risk
  register (symptom: "always-true gate"). Correct tick-aware conditions are a Phase-16.1
  follow-up (would need `evaluate_ready_conditions` to `set_change_ticks` with a cross-frame
  `last_run`). Do NOT claim tick-conditions are "rejected."
- **P5 (MEDIUM, citation).** `apply_window_drain` BODY is `schedule.rs:316-384` (the gate +
  `world_mut` reborrow are `:268-280`). Anchor the new `evaluate_ready_conditions` call
  between the drain's return (`~:280`) and the termination check (`~:283`).
- **P6 (LOW ×3).** (a) **`run_condition` does NOT call `apply`** (orchestrator decision):
  conditions are pure read-only predicates; a `Commands`/`EventWriter`-using condition is a
  logic error, and its deferred commands are DROPPED (safer than flushing structural
  mutations mid-eval-pass that later conditions in the same pass would observe). The runner
  calls `initialize` (once at build) + `run_unsafe` (per frame) ONLY. (b) Soften the
  determinism proof to "deterministic given conditions are read-only (enforced for declared
  access; `Commands`/`EventWriter` side-effects are a documented logic error)". (c)
  Acknowledge the doubled O(n) scan (gated to conditioned schedules; bounded by
  `n ≤ 1024`); add test `condition_eval_deferred_while_workers_live` (a condition records via
  atomic the `running.count_ones()` it observes — assert always 0; this is the regression net
  for the R2 race guard).

---

## §1 Scope

### In scope
1. A **condition** is `impl IntoSystem<(), bool, M>` — reuses the entire `SystemParamFunction`/`FunctionSystem` machinery (`into_system.rs:78-89`, `function_system.rs:166-299`). The single new erased type is `type BoolSystem = Box<dyn System<Out = bool>>`.
2. `SystemConfig::run_if<C, M>(self, cond) -> Self` (`system_config.rs:47`) and `ConfigureSet::run_if<C, M>(self, cond) -> Self` (`schedule_builder.rs:444`). Multiple `.run_if(a).run_if(b)` → AND (eager fold).
3. Build-time `conditions: Vec<BoolSystem>` on `SystemDescriptor` (`system_descriptor.rs:39`); set conditions keyed by `SystemSetId` on the builder.
4. Runtime parallel storage on `Schedule`: `system_conditions: Vec<Vec<BoolSystem>>` (permuted into topo order alongside `systems`), `set_conditions: Vec<(SystemSetId, BoolSystem)>`, `system_gating_sets: Vec<Box<[SystemSetId]>>` (the `system → gating-set-ids` map), and a `has_condition: FixedBitSet` for the 0%-gate.
5. Executor integration at the **apply-window boundary** (`schedule.rs:268-280`): a conditioned system that becomes ready has its (own + gating-set) conditions evaluated single-threaded; false eager-AND fold → mark completed + decrement successors WITHOUT running the body.
6. Set conditions evaluated ONCE per frame, cached in `ExecutorScratch` (reset in `reset_for_frame`).
7. Built-ins: `run_once` (uses `Local<bool>`, `local.rs:62`). Documented read-only requirement + `debug_assert!` on the condition's write-`Access` emptiness.

### Out of scope (deferred, with reasons)
| Deferred | Reason |
|---|---|
| `resource_exists::<R>` | `Option<Res<R>>` is **NOT** a `SystemParam` (grep: no `impl SystemParam for Option<...>`). `Res<R>::get_param` panics on absence (`res.rs:130` → `missing_resource_panic`). No SystemParam exposes `contains_resource` (`ecs_master.rs:1807`). Shipping it requires a new `Option<Res<R>>` or `Has<R>` SystemParam — out of Phase 16. See §8. |
| Typed combinators `.and`/`.or`/`.not(c)` | AND-via-chaining covers the common case; combinators are a thin ergonomic layer. |
| `on_event`/`on_message` | The missed-events footgun (research §1: a false-conditioned system's `EventReader` cursor doesn't advance) needs a deliberate semantic decision. |
| `in_state` | Depends on a state machine (Phase 17). |
| Condition access in the conflict graph | Unnecessary — conditions run single-threaded at the apply barrier (§3). |
| Auto sync-points for conditions | Conditions are read-only; no deferred commands to flush. |
| `Changed<T>`-based conditions | `run_cached_system` does not call `set_change_ticks` (`ecs_master.rs:1685-1702`); a condition's meta ticks stay at the `initialize` sentinel. Stateful tick-based conditions would mis-report. Documented as Open Question §12. |

---

## §2 `BoolSystem` + parallel storage

### Decision 2.1: `type BoolSystem = Box<dyn System<Out = bool>>`

**What**: A new erased system type whose `Out = bool`, stored entirely OUTSIDE `SystemBox`.

**Why**: `SystemBox::system` is pinned `Box<dyn System<Out = ()>>` (`system_box.rs:61`), 40 B / 1 cache line (`system_box.rs:50-55`). The executor's whole dispatch path — `add_system` bound `F::System: System<Out = ()>` (`schedule_builder.rs:125`), `run_unsafe` return ignored (`schedule.rs:643`), `apply` (`schedule.rs:350`) — assumes `()`. Widening `SystemBox` to a generic-or-enum `Out` would (a) break the 1-cache-line invariant asserted in the doc, (b) ripple through every `SystemBox::new` call site, (c) tax the no-condition hot path. `System::Out` is already generic (`system.rs:59`); `FunctionSystem<F,M>::Out = <F as SystemParamFunction<M>>::Out` (`function_system.rs:171`), so a `fn(...) -> bool` becomes a `BoolSystem` for free via `IntoSystem<(), bool, M>` (`into_system.rs:78-89`).

**Alternatives rejected**:
- *Widen `SystemBox` to `enum { Unit(Box<dyn System<Out=()>>), Bool(...) }`* — bloats every system slot by a discriminant + the larger variant, taxes the no-condition path with a match per dispatch.
- *A `dyn FnMut(UnsafeEcsCell) -> bool` instead of `dyn System`* — loses `initialize`/`apply`/`access`/`meta`, so `Local` state (which lives in `FunctionSystem::state`) couldn't be initialized at build. `dyn System<Out=bool>` reuses the whole machinery.

**Trade-off**: A conditioned system pays one heap indirection (the `Box`) + one `Vec` indirection (`system_conditions[i]`) per condition per frame. Acceptable: only conditioned systems pay it; tens of ns per trivial condition (dominated by param fetch, research §5).

### Decision 2.2: Build-time storage on `SystemDescriptor`

```rust
// system_descriptor.rs — ADD one field (build-time only; never on the hot path).
pub(crate) struct SystemDescriptor {
    pub(crate) system_box: SystemBox,         // unchanged — moved into Schedule::systems
    pub(crate) ordering_hints: Vec<OrderingEdge>,
    pub(crate) sets: Vec<SystemSetId>,
    /// Phase 16: own run-conditions, in declaration order. Empty for the
    /// overwhelming majority of systems. Each is an initialized BoolSystem.
    pub(crate) conditions: Vec<BoolSystem>,    // NEW
}
```
`SystemDescriptor::new` (`system_descriptor.rs:60`) seeds `conditions: Vec::new()`. The descriptor is `pub(crate)`, dropped by `build` after the permutation — adding a `Vec` here costs nothing at runtime (the comment at `system_descriptor.rs:9-17` confirms these fields exist only during build).

### Decision 2.3: Builder-side set-condition map

```rust
// schedule_builder.rs — ScheduleBuilder gains:
/// Phase 16 — set-level run conditions, keyed by SystemSetId. A set may
/// accumulate multiple conditions (AND). Built into Schedule::set_conditions.
pub(crate) set_conditions: HashMap<SystemSetId, Vec<BoolSystem>>,   // NEW
```
Seeded `HashMap::new()` in `ScheduleBuilder::new` (`schedule_builder.rs:97`). `ConfigureSet::run_if` (§8) pushes into `set_conditions.entry(self.set_id).or_default()`. The intern path `set_id_of_value` (`schedule_builder.rs:146`) already guarantees the config id matches the membership id (Phase 15 crux), so the same `SystemSetId` keys both `set_members` and `set_conditions`.

### Decision 2.4: Runtime storage on `Schedule`

```rust
// schedule.rs — Schedule gains four fields AFTER executor_scratch (cold-ish:
// touched once per ready-transition, only when has_condition[i] is set).
pub struct Schedule {
    pub(crate) pool: Arc<ThreadPool>,
    pub(crate) systems: Vec<SystemBox>,
    pub(crate) conflict_graph: ConflictGraph,
    pub(crate) executor_scratch: ExecutorScratch,

    // ── Phase 16 ──────────────────────────────────────────────────────────
    /// `has_condition[i]` set iff system `i` has ANY own condition OR is a
    /// member of ANY set that has a condition. THE 0%-GATE (see §4). All-zero
    /// when no .run_if anywhere → the executor's branch is predicted-not-taken.
    pub(crate) has_condition: FixedBitSet,                       // NEW

    /// Per-system own conditions, indexed by post-topo SystemIndex. Permuted
    /// alongside `systems` (Step 6 of build). `system_conditions[i]` is empty
    /// unless system i carried `.run_if`. Outer Vec is len == systems.len().
    pub(crate) system_conditions: Vec<Vec<BoolSystem>>,         // NEW

    /// Gating sets per system, indexed by post-topo SystemIndex: the transitive
    /// sets that system i belongs to AND that carry at least one condition.
    /// Empty for systems in no conditioned set. Permuted alongside `systems`.
    pub(crate) system_gating_sets: Vec<Box<[SystemSetId]>>,     // NEW

    /// Flat set-condition table. The cached per-frame result lives in
    /// ExecutorScratch::set_cond_cache (indexed by the dense position here),
    /// NOT here. `(SystemSetId, BoolSystem)`; multiple rows per set = AND.
    pub(crate) set_conditions: Vec<SetConditionEntry>,          // NEW
}

/// One set-condition row. `slot` is the dense index into
/// ExecutorScratch::{set_cond_evaluated, set_cond_result} for memoization.
pub(crate) struct SetConditionEntry {
    pub(crate) set_id: SystemSetId,
    pub(crate) condition: BoolSystem,
    pub(crate) slot: u16,   // index into the per-frame cache bitsets
}
```

**Field order rationale**: the four new fields sit AFTER `executor_scratch`. The hot dispatcher loop (`schedule.rs:444-485`) reads `completed`/`running`/`pred_remaining`/`conflict_bits` — all in the prefix. `has_condition` is touched once per ready system (a single `contains(i)` bit test, §4). `system_conditions`/`system_gating_sets`/`set_conditions` are touched ONLY when `has_condition[i]` is set, i.e. never on the no-condition path. Keeping them at the tail preserves the dispatcher's L1d footprint (Schedule field-order doc `schedule.rs:65-72`).

### Decision 2.5: The build-time permutation (matches `systems` reordering)

`build` permutes `descriptors` into topo order via `reorder` and a `Vec<Option<SystemDescriptor>>` take-loop (`schedule_builder.rs:351-371`). The `conditions` Vec rides along inside each `SystemDescriptor`, so `system_conditions` is built by **moving** `conditions` out at the SAME `Step 10` where `system_box` is extracted (`schedule_builder.rs:410-415`):

```rust
// Step 10 (extended). descriptors_with_sync is ALREADY in topo order.
let mut systems: Vec<SystemBox> = Vec::with_capacity(n_final);
let mut system_conditions: Vec<Vec<BoolSystem>> = Vec::with_capacity(n_final);
for d in descriptors_with_sync {              // consumes by value
    system_conditions.push(d.conditions);     // move (no clone)
    systems.push(d.system_box);
}
```
Because `descriptors_with_sync[new_idx]` is the system at post-topo `SystemIndex(new_idx)`, `system_conditions[new_idx]` aligns with `systems[new_idx]` by construction — no separate permutation pass.

`system_gating_sets[new_idx]` is built from the Phase-15 transitive membership (§7) + the `reorder` map (`schedule_builder.rs:351-354`): for each system at post-topo index `j`, list the conditioned sets it transitively belongs to.

`has_condition[new_idx]` is set iff `!system_conditions[new_idx].is_empty() || !system_gating_sets[new_idx].is_empty()`.

**`initialize` of conditions**: conditions are `initialize`d in the SAME loop that initializes systems (`schedule_builder.rs:243-253`, Step 1), so their `Access`/`Local` state are ready before the first frame:
```rust
// Step 1 (extended).
for d in &mut descriptors {
    d.system_box.system.initialize(world);
    d.system_box.is_exclusive = d.system_box.system.access().is_universal();
    for cond in d.conditions.iter_mut() {
        cond.initialize(world);               // FS1-idempotent; builds Local state, declares Access
        debug_assert!(                        // read-only contract (§8)
            cond.access().component_writes.is_empty() && cond.access().resource_writes.is_empty(),
            "Phase 16 CR1: a run condition must declare NO writes (got writes from {})",
            cond.name(),
        );
    }
}
// Set conditions initialized when assembling Schedule::set_conditions (same world, build-time).
```
`Access::component_writes`/`resource_writes` expose `is_empty()` (`component_mask.rs:149`, `bit_set_256.rs:69`) — the read-only `debug_assert` is a 64 B + 32 B scan, build-time only.

---

## §3 Executor integration (the crux)

### Decision 3.1: Evaluate at the apply-window boundary, not in the ready scan

**What**: Condition evaluation is a new step **between** the apply-window drain (`schedule.rs:268-280`) and the dispatch loop (`schedule.rs:293`), gated on a fresh "newly-ready, conditioned" set. It runs on the dispatcher thread, single-threaded, using the `&mut EcsMaster` recovered exactly as the apply window does.

**Why (race-freedom for free)**: The apply-window gate `pending > 0 && (pending == running || running == 0)` (`schedule.rs:270`) is the SCH7 barrier — when it fires, every dispatched worker has executed past its Release `fetch_add` (`schedule.rs:665`) and the dispatcher's Acquire load (`schedule.rs:268`) synchronizes-with all of them. At that instant no worker holds the cell, so `cell.world_mut()` (`schedule.rs:278`) is a legal exclusive reborrow. A condition needs a `&mut EcsMaster` (via `run_cached_system`, §5) to read resources/components and advance its `Local` — recovering it at this exact barrier means the condition reads **committed** state (all prior systems' `apply` ran in `apply_window_drain`, `schedule.rs:350`) and races nothing.

**Alternatives rejected (research §3.1, "Option B")**: evaluating inside `try_dispatch_ready`'s ready scan (`schedule.rs:444`) while prior-round workers are still running. The cell is shared `Copy` into live worker closures there; reborrowing `&mut EcsMaster` would alias a worker's write-capable cell → data race / UB. Rejected.

### Decision 3.2: Where the new step slots in — precise control flow

The problem: a system only becomes "ready" when `pred_remaining[i] == 0`, which happens incrementally as predecessors complete in `apply_window_drain`. We must evaluate a conditioned system's gate at the moment it transitions to ready, BEFORE `try_dispatch_ready` would spawn it. Two transition points exist:
1. Frame start — systems with `pred_count == 0` are ready immediately (no apply window has fired yet; `running == 0`, `pending == 0`).
2. After each `apply_window_drain` — successors whose `pred_remaining` hit 0 become newly ready.

Both are points where `running == 0` OR the SCH7 barrier holds. The cleanest integration: a single `evaluate_ready_conditions` pass run at the TOP of each loop iteration, AFTER the apply-window drain and BEFORE termination/dispatch. It is gated so it does nothing when `has_condition` is empty (§4).

```rust
// schedule.rs executor_main_loop — REVISED loop body.
loop {
    // === Step 1: apply window drain (UNCHANGED — schedule.rs:268-280). ===
    let pending = self.executor_scratch.pending_apply.load(Ordering::Acquire);
    let running = self.executor_scratch.running.count_ones(..);
    if pending > 0 && (pending == running || running == 0) {
        // SAFETY (SCH7): unchanged — gate proves no worker holds the cell.
        let world_mut: &mut EcsMaster = unsafe { cell.world_mut() };
        self.apply_window_drain(world_mut);
    }

    // === Step 1.5 (PHASE 16): evaluate conditions for newly-ready systems. ===
    //
    // GATE (0%-regression, §4): skip the whole pass if no conditions exist.
    // `has_condition.is_clear()` is a single `count_ones(..) == 0` / cached
    // bool — predicted-not-taken when no .run_if anywhere.
    if !self.has_condition.is_clear() {
        // We may evaluate ONLY when the cell is recoverable as &mut: that is
        // true here iff no worker is live. The apply-window gate above either
        // already drained (running back to a quiescent count) OR running == 0.
        // We additionally require `running.count_ones(..) == 0` before touching
        // the cell, matching the exclusive-system precondition (EXC2,
        // schedule.rs:470). When workers ARE live, we defer condition eval to
        // a later iteration (the parked dispatcher wakes on completion).
        if self.executor_scratch.running.count_ones(..) == 0 {
            // SAFETY (SCH7 / Phase 16 CR2): running == 0 ⇒ every previously
            //   dispatched worker has completed AND been drained (apply_window
            //   above popped them). No worker holds a cell copy. The reborrow
            //   is the exclusive &mut for the duration of condition eval, which
            //   never spawns and never retains a cell-derived borrow.
            let world_mut: &mut EcsMaster = unsafe { cell.world_mut() };
            self.evaluate_ready_conditions(world_mut);
        }
    }

    // === Step 2: termination (UNCHANGED — schedule.rs:283). ===
    if self.executor_scratch.completed.count_ones(..) == n {
        return;
    }

    // === Step 3+4: dispatch (UNCHANGED — schedule.rs:293). ===
    let dispatched = self.try_dispatch_ready(scope, cell);

    // === Step 5: backoff (UNCHANGED — schedule.rs:302-304). ===
    if dispatched == 0 && self.executor_scratch.running.count_ones(..) > 0 {
        std::thread::park_timeout(PARK_TIMEOUT);
    }
}
```

**Why `running == 0` and not the looser SCH7 gate?** The looser gate `pending == running` can be true with `running > 0` only transiently (the gate fires after every dispatched worker reported, but `running` bits are cleared inside `apply_window_drain`, `schedule.rs:331`). After `apply_window_drain` returns, every drained system has `running` cleared, so the only `running` bits left belong to systems dispatched in an EARLIER round that haven't completed — but the gate `pending == running` proved they all completed, so `apply_window_drain` cleared them all. Net: after the drain, `running.count_ones() == 0` whenever the gate fired. When the gate did NOT fire (workers still in flight), we skip condition eval this iteration and park; a later iteration after their completion will have `running == 0`. So `running == 0` is the precise, always-safe predicate and it reuses the exact reasoning the inline-exclusive path already relies on (`schedule.rs:470, 506-521`).

### Decision 3.3: `evaluate_ready_conditions` — the pass

```rust
// schedule.rs — NEW method.
/// Evaluate conditions for every conditioned system that is newly ready
/// (pred_remaining == 0, not running, not completed) and whose conditions
/// have NOT yet been evaluated this frame. A system whose folded gate is
/// false is marked completed and its successors decremented — exactly as if
/// it had run and applied, but WITHOUT running the body.
///
/// Precondition: running.count_ones() == 0 (caller-checked). The dispatcher
/// holds &mut world exclusively; no worker is live.
fn evaluate_ready_conditions(&mut self, world: &mut EcsMaster) {
    let n = self.systems.len();

    // First, evaluate every set condition that gates a ready system, ONCE
    // per frame (memoized in scratch). Done lazily: a set condition is only
    // run the first time a ready system depends on it this frame.
    // (Loop body below pulls cached results via `set_gate(world, set_id)`.)

    for i in 0..n {
        // Reuse the EXACT ready predicate from try_dispatch_ready
        // (schedule.rs:445-453), minus the conflict check (conflicts gate
        // concurrent dispatch, not condition eval).
        if self.executor_scratch.completed.contains(i) { continue; }
        if self.executor_scratch.running.contains(i) { continue; }   // always false here
        if self.executor_scratch.pred_remaining[i] != 0 { continue; }
        if !self.has_condition.contains(i) { continue; }             // only conditioned systems
        if self.executor_scratch.cond_evaluated.contains(i) { continue; } // once per system per frame

        self.executor_scratch.cond_evaluated.insert(i);

        // EAGER FOLD (§6): run ALL own conditions + all gating-set conditions,
        // AND the results. Do NOT short-circuit (stateful conditions must
        // advance every frame).
        let mut should_run = true;

        // Own conditions.
        for cond in self.system_conditions[i].iter_mut() {
            // SAFETY (CR3): see §5 — run a BoolSystem on &mut world.
            let r = world.run_cached_system(cond);
            should_run = should_run && r;          // bitwise && is fine; NOT short-circuit over the LOOP
        }
        // Gating-set conditions (cached per frame).
        for &set_id in self.system_gating_sets[i].iter() {
            let r = self.set_gate(world, set_id);  // memoized; see §7
            should_run = should_run && r;
        }

        if !should_run {
            // SKIP: mark completed + decrement successors, WITHOUT body/apply.
            self.mark_skipped(i);
        }
        // If should_run == true, do nothing here — try_dispatch_ready will
        // pick the system up normally this same loop iteration (Step 3+4).
    }
}

/// Mark system `i` as skipped: completed + successor decrement. Mirrors the
/// apply-window completion tail (schedule.rs:359-372) MINUS run/apply/queue.
#[inline]
fn mark_skipped(&mut self, i: usize) {
    self.executor_scratch.completed.insert(i);
    // Decrement successors — IDENTICAL to schedule.rs:364-372.
    for &successor in self.conflict_graph.successors[i].iter() {
        let s = successor.0 as usize;
        debug_assert!(
            self.executor_scratch.pred_remaining[s] > 0,
            "invariant SCH13 (Phase 16): pred_remaining must not underflow on skip (system {})",
            s,
        );
        self.executor_scratch.pred_remaining[s] -= 1;
    }
}
```

**Cascade handling**: skipping system `i` decrements its successors' `pred_remaining`. A successor `s` that thereby reaches `pred_remaining[s] == 0` and is ALSO conditioned must have ITS conditions evaluated. Because `evaluate_ready_conditions` is a `for i in 0..n` pass in topo order, and skip-decrements only ever LOWER `pred_remaining`, a successor `s > i` (topo order guarantees successors come after predecessors) is reached later in the SAME pass with the updated `pred_remaining[s]`. A successor `s < i` cannot exist (would violate topological order — an ordering edge `i → s` forces `i` before `s`). Therefore one forward pass settles the entire skip cascade for systems that become ready purely via skips. Systems that become ready via a *real* completion are handled on the next loop iteration (after the next `apply_window_drain`). 

**Why a `cond_evaluated` bitset and not reusing `completed`**: a `should_run == true` system is NOT marked completed by this pass (it still needs to run). Without `cond_evaluated`, the next loop iteration would re-run its conditions (advancing `Local` twice per frame — wrong, violates the once-per-frame `run_once` semantic). `cond_evaluated[i]` records "conditions already folded this frame for system i". Reset in `reset_for_frame` (§7).

### Proof (a): race-freedom

Conditions are evaluated ONLY inside `evaluate_ready_conditions`, called ONLY when `running.count_ones(..) == 0` (Step 1.5 guard). `running` bit `i` is set when system `i` is dispatched (`schedule.rs:484, 504`) and cleared when its completion is drained (`schedule.rs:331`). `running == 0` therefore means every dispatched system has been drained — i.e. every worker that received a cell copy has executed past its Release `fetch_add` (`schedule.rs:665`) which the dispatcher's prior Acquire load synchronized-with (`schedule.rs:268`). No worker holds a live cell. The dispatcher's `cell.world_mut()` is the unique `&mut EcsMaster`. `run_cached_system` consumes a fresh `UnsafeEcsCell::new_mutable(self)` derived from that `&mut` (`ecs_master.rs:1692`) and the condition body runs to completion before `run_cached_system` returns, retaining no cell-derived borrow. No spawn happens inside the pass. ∴ no data race. This is the identical argument the inline-exclusive path already uses (`schedule.rs:506-521`), reused verbatim.

### Proof (b): skipped-successor semantic

`mark_skipped(i)` sets `completed[i]` and runs the EXACT successor-decrement loop from the apply-window completion path (`schedule.rs:364-372`). A skipped system's `before` successors thus get their `pred_remaining` decremented identically to a system that ran. They become ready and run (or are themselves evaluated, if conditioned). The skip is "don't run the body", NOT "remove from the DAG" — matching Bevy's `signal_dependents` (research §1). The skipped system never pushes to `completion_queue` and never bumps `pending_apply`, so the apply window's `target`/`drained` accounting (`schedule.rs:317-323`) is unaffected (a skip is invisible to it). Termination (`completed.count_ones() == n`, `schedule.rs:283`) counts skipped systems as completed, so a frame where every system is skipped still terminates.

### Proof (c): underflow guard holds

`mark_skipped` carries `debug_assert!(pred_remaining[s] > 0)` before each decrement (identical to `schedule.rs:366-370`). Each ordering edge `i → s` contributes exactly 1 to `pred_count[s]` (deduped at build, `schedule_builder.rs:379-388`). System `i` is marked completed/skipped AT MOST ONCE per frame: `cond_evaluated[i]` prevents a second condition eval (`evaluate_ready_conditions` checks `completed.contains(i)` and `cond_evaluated.contains(i)`), and `try_dispatch_ready` skips `completed` systems (`schedule.rs:445`) so a skipped system is never spawned/re-completed. ∴ each edge `i → s` decrements `pred_remaining[s]` exactly once, never below 0. The guard never trips. (A system that runs normally is NOT also skipped because `evaluate_ready_conditions` only calls `mark_skipped` when `should_run == false`, in which case `try_dispatch_ready` never spawns it — the two paths are mutually exclusive per system per frame.)

### Proof (d): determinism

`evaluate_ready_conditions` iterates `for i in 0..n` in the fixed post-topo order (the same order `try_dispatch_ready` scans, `schedule.rs:444`). The skip decision for system `i` depends only on (1) its conditions' results and (2) cached set-condition results, both pure functions of committed world state at the deterministic apply boundary. The skip cascade is resolved in one forward pass over a fixed order. The Kahn-FIFO tie-break (`schedule_builder.rs:680`) is intact — Phase 16 adds no edges and reorders nothing. Two runs with identical world state + identical condition bodies produce identical skip sets and identical execution. ∴ deterministic.

---

## §4 The 0%-regression gate

### Decision 4.1: `has_condition` is a `FixedBitSet`, sibling to `running`/`completed`

**What**: `Schedule::has_condition: FixedBitSet` of length `n`, all-zero iff no `.run_if` anywhere. The gate is a single `self.has_condition.is_clear()` test (a `FixedBitSet` block-OR; `count_ones(..) == 0`) at the top of Step 1.5.

**Where the gate branch lives**: in `executor_main_loop`, immediately after the apply-window drain, BEFORE termination and dispatch (Step 1.5 above). It is `if !self.has_condition.is_clear() { ... }`. When clear, the body — including the `running == 0` check and the entire `evaluate_ready_conditions` pass — is skipped. The branch is predicted-not-taken (the schedule was built with no conditions ⇒ the bitset is all-zero ⇒ the branch is never taken across the whole run).

**Why this is 0%**: 
- `SystemBox` is **not widened** — its layout (`system_box.rs:50-55`, 1 cache line) and the `Out = ()` pin are untouched. No per-system condition data sits on the dispatch path.
- The no-condition path adds exactly ONE branch per loop iteration (`is_clear()`), in the same cost class as the existing `pred_remaining[i] != 0` check (`schedule.rs:451`) and the `dispatched == 0 && running > 0` backoff (`schedule.rs:302`). `is_clear()` on a small bitset (n ≤ 1024 → ≤ 16 `u64` words) is a few ORs, hoistable/cached.
- `try_dispatch_ready` (`schedule.rs:425-673`) is **byte-identical** — no condition check added inside the dispatch loop. Conditions are resolved in the separate Step 1.5 pass, which is entirely skipped when the gate is clear.
- The 50-empty-systems bench (Phase 15 methodology, `PHASE-15-RESULTS.md:67-72`) builds with zero conditions ⇒ `has_condition` all-zero ⇒ the executor diff is one not-taken branch per round. Target: within ±2% (criterion "no change detected"), same as Phase 15.

**Micro-optimization (optional, deferred)**: cache `has_any_condition: bool` as a `Schedule` field to avoid the `is_clear()` scan per iteration. Deferred unless the A/B shows measurable cost — `is_clear()` on ≤16 words is sub-nanosecond.

### Decision 4.2: No `if const` elision (research §3.2)

Conditions are runtime registration on a type-erased `Schedule` — there is no compile-time type available to `if const`-elide the branch (unlike Phase 10's `NEEDS_CHANGE_DETECTION` const, which keyed off the typed `Query<D,F>`). The per-schedule all-zero bitset branch is the correct mechanism and is effectively free. This is explicitly accepted.

---

## §5 Running the condition (invocation)

### Decision 5.1: Reuse `run_cached_system`

**What**: `evaluate_ready_conditions` invokes each `BoolSystem` via `world.run_cached_system(cond)` (`ecs_master.rs:1685-1702`), which returns `S::Out = bool`.

**Why**: `run_cached_system` is the ready-made primitive: it calls `initialize` (FS1-idempotent, no-op after build, `function_system.rs:188`), mints `UnsafeEcsCell::new_mutable(self)` (`ecs_master.rs:1692`), calls `run_unsafe` (returns the `bool`, `ecs_master.rs:1697`), then `apply` (`ecs_master.rs:1700`). For a read-only condition `apply` is a no-op (no deferred `SystemParam`s like `Commands`), so the reuse is correct and incurs only the empty `apply` forward (`function_system.rs:276-280` short-circuits if state is `None`, else forwards to `SystemParam::apply` which is empty for `Res`/`Local`).

**The dispatcher's `&mut EcsMaster`**: `run_cached_system` takes `&mut self` on `EcsMaster`. The dispatcher passes the `world_mut` recovered from `cell.world_mut()` (Step 1.5). Signature in the pass:
```rust
let r: bool = world.run_cached_system(cond);   // cond: &mut BoolSystem; world: &mut EcsMaster
```
`cond` is `&mut self.system_conditions[i][k]`, i.e. `&mut Box<dyn System<Out = bool>>`. `Box<dyn System>` derefs to `dyn System`, and `run_cached_system<S: System>` accepts `&mut S` — but `S = dyn System<Out=bool>` is `?Sized`. **Subtlety**: `run_cached_system<S: System>` bounds `S: Sized` implicitly. A `&mut Box<dyn System<Out=bool>>` can be passed as `&mut S` with `S = Box<dyn System<Out=bool>>` IF `Box<dyn System>` itself implements `System`. It does NOT by default. Two options:

- **Option A (chosen)**: a thin condition-runner method that takes `&mut dyn System<Out=bool>`:
  ```rust
  // ecs_master.rs — NEW (mirrors run_cached_system but ?Sized + Out=bool).
  /// Run a type-erased read-only condition once on &mut self, returning its
  /// bool verdict. Reuses the run_cached_system sequence; `apply` is a no-op
  /// for read-only conditions.
  pub(crate) fn run_condition(&mut self, cond: &mut dyn System<Out = bool>) -> bool {
      cond.initialize(self);                                  // FS1 no-op after build
      // SAFETY (CR3): &mut self is exclusive for the call (the dispatcher holds
      //   the unique &mut EcsMaster recovered at the apply barrier; running == 0
      //   proven by the caller). No other System::run_unsafe is in flight ⇒ S1.
      let cell = unsafe { UnsafeEcsCell::new_mutable(self) };
      // SAFETY (S1): as above; the cell is consumed by run_unsafe and does not escape.
      let verdict = unsafe { cond.run_unsafe(cell) };
      cond.apply(self);                                       // APP1'; no-op for read-only
      verdict
  }
  ```
  Then the pass calls `world.run_condition(cond.as_mut())` where `cond: &mut Box<dyn System<Out=bool>>` and `as_mut(): &mut dyn System<Out=bool>`. This avoids requiring `Box<dyn System>: System` and keeps the call monomorphization-free.

**Why Option A over the generic `run_cached_system`**: `run_cached_system` is generic over a `Sized` `S`; passing an erased `Box<dyn System>` would need a `System for Box<dyn System>` blanket impl (extra surface, extra indirection). `run_condition(&mut dyn System<Out=bool>)` is a direct virtual call on the already-erased trait object — one vtable dispatch, no wrapper. It is the minimal extension.

**Trade-off**: one new `pub(crate)` method on `EcsMaster` (~6 lines, two `unsafe` blocks with `// SAFETY:`). It duplicates the `run_cached_system` body but with a `?Sized` receiver and `Out = bool`. Acceptable — the alternative (blanket `System for Box<dyn System>`) is more surface for less clarity.

### Decision 5.2: Conditions `initialize`d at build, ticks NOT advanced per frame

Conditions are `initialize`d once in build Step 1 (§2.5). The per-frame `run_condition` call's `initialize` is the FS1 no-op (`function_system.rs:188`). `run_condition` does NOT call `set_change_ticks` (mirroring `run_cached_system`, `ecs_master.rs:1685-1702`), and the dispatcher's per-frame `set_change_ticks` loop (`schedule.rs:158-161`) iterates `self.systems` only — NOT conditions. So a condition's meta ticks stay at the `initialize` sentinel (`function_system.rs:205-208`). This is correct for the shipped built-ins (`run_once` uses `Local<bool>`, no ticks). It is a documented limitation for `Changed<T>`-based conditions (Open Question §12).

---

## §6 Eager fold (NOT short-circuit)

### Decision 6.1: Fold ALL conditions every frame, AND the results

**What**: For a system with own conditions `[c0, c1, ...]` and gating-set conditions `[s0, s1, ...]`, the per-frame gate is:
```
should_run = (run c0) AND (run c1) AND ... AND (cached s0) AND (cached s1) AND ...
```
where EVERY `run cK` is invoked (the `Local` state advances), and the AND is computed by accumulating into a `bool` WITHOUT `?`-style early exit over the loop. Concretely, the fold in §3.3 uses `should_run = should_run && r;` per iteration but **does not `break`** — every condition runs.

**Why NOT short-circuit (the one non-obvious rule, research §1/§5)**: a condition like `run_once` mutates its own `Local<bool>` every time it runs (sets the flag, returns the previous value). If the fold short-circuited (e.g. `c0 == false` skips `c1`), then `c1`'s `Local` would not advance, and a later frame where `c0 == true` would observe `c1` in a stale state — wrong. Bevy's `evaluate_and_fold_conditions` uses `.fold(true, |acc, res| acc && res)` precisely so "short-circuiting would prevent conditions from mutating their own state" (research §1). We replicate: the LOOP always runs every condition; only the accumulated `bool` decides the gate.

**Subtlety — `&&` vs `&`**: `should_run && r` where both operands are already-evaluated `bool`s does NOT short-circuit anything (the loop already ran `r = world.run_condition(...)` before the `&&`). The `&&` is over two materialized `bool`s, so it is equivalent to `&`. Using `&&` is fine; the load-bearing property is that `r` is computed unconditionally inside the loop body, before the fold step. I will write it as `should_run &= r;` (bitwise, unambiguous) to make "no short-circuit" explicit and avoid any reader confusion.

```rust
let mut should_run = true;
for cond in self.system_conditions[i].iter_mut() {
    let r = world.run_condition(cond.as_mut());   // ALWAYS runs (advances Local)
    should_run &= r;                              // eager AND; no break
}
for &set_id in self.system_gating_sets[i].iter() {
    should_run &= self.set_gate(world, set_id);   // cached; set conditions also eager-folded
}
```

**Trade-off**: a system whose first condition is already `false` still pays the cost of running its remaining conditions. This is intentional (correctness > the micro-saving of skipping cheap read-only bodies). Acceptable — conditions are tens of ns each.

---

## §7 Set-level conditions

### Decision 7.1: Evaluate ONCE per frame, memoized in `ExecutorScratch`

**What**: Each `SetConditionEntry` carries a dense `slot: u16`. `ExecutorScratch` gains two parallel bitsets:
```rust
// executor_scratch.rs — ExecutorScratch gains (after pred_remaining, before queue):
/// Phase 16 — per-frame condition memoization.
/// `cond_evaluated[i]` set once system i's conditions have been folded this
/// frame (prevents re-folding stateful conditions; §3.3).
pub(crate) cond_evaluated: FixedBitSet,           // NEW, len == system_count

/// `set_cond_evaluated[slot]` set once set-condition row `slot` has run this
/// frame; `set_cond_result[slot]` caches its bool. Reset in reset_for_frame.
pub(crate) set_cond_evaluated: FixedBitSet,       // NEW, len == set_conditions.len()
pub(crate) set_cond_result: FixedBitSet,          // NEW, len == set_conditions.len()
```
`set_cond_result` is a bitset (1 bit per row) — a set condition returns `bool`, so a packed bitset is the densest cache. `set_gate(world, set_id)` lazily evaluates and memoizes:
```rust
// schedule.rs — set-condition memoized evaluator. Returns AND of every row
// for `set_id` (a set may have multiple conditions → AND).
fn set_gate(&mut self, world: &mut EcsMaster, set_id: SystemSetId) -> bool {
    let mut acc = true;
    for entry in self.set_conditions.iter_mut().filter(|e| e.set_id == set_id) {
        let slot = entry.slot as usize;
        let r = if self.executor_scratch.set_cond_evaluated.contains(slot) {
            self.executor_scratch.set_cond_result.contains(slot)
        } else {
            // EAGER (§6): run the set condition body once this frame.
            let v = world.run_condition(entry.condition.as_mut());
            self.executor_scratch.set_cond_evaluated.insert(slot);
            self.executor_scratch.set_cond_result.set(slot, v);
            v
        };
        acc &= r;   // eager AND across a set's own conditions
    }
    acc
}
```
**Borrow note**: `set_gate` takes `&mut self` and iterates `self.set_conditions` while mutating `self.executor_scratch` — these are disjoint fields, so a split borrow (or indexing rather than holding an iterator across the scratch write) satisfies the borrow checker. The developer step will index by position (`for k in 0..self.set_conditions.len()`) to avoid holding `&mut self.set_conditions` across the `&mut self.executor_scratch` write.

**Why once-per-frame (research §3.5)**: a set condition gates EVERY member of the set. Re-running it per member would (a) waste N condition runs for an N-member set, (b) advance any stateful set-condition `Local` N times per frame (wrong). Memoizing in `set_cond_evaluated`/`set_cond_result` (Bevy's `evaluated_sets` bitset) runs it exactly once. The first ready member to depend on the set triggers the run; subsequent members read the cache.

### Decision 7.2: Reuse Phase-15 transitive membership for `system_gating_sets`

**What**: `system_gating_sets[j]` (for post-topo system index `j`) is the list of conditioned sets that system `j` transitively belongs to. Built at build time from `flatten_set_membership`'s output (`transitive_members`, `schedule_builder.rs:281-282`) inverted: for each `(set_id, members)` in `transitive_members` WHERE `set_conditions` has a row for `set_id`, for each member `SystemKey`, append `set_id` to that system's gating list (then remap the `SystemKey` to its post-topo index via `reorder`, `schedule_builder.rs:351-354`).

```rust
// schedule_builder.rs build — after Step 6 (reorder built), before Step 10.
// Only sets that actually carry conditions become "gating".
let conditioned_sets: HashSet<SystemSetId> = set_conditions.keys().copied().collect();
let mut gating_by_new_idx: Vec<Vec<SystemSetId>> = vec![Vec::new(); n];
for (&set_id, members) in &transitive_members {
    if !conditioned_sets.contains(&set_id) { continue; }
    for &member_key in members {
        let new_idx = reorder[member_key.0] as usize;
        gating_by_new_idx[new_idx].push(set_id);
    }
}
// Dedupe + freeze to Box<[SystemSetId]> per system.
let system_gating_sets: Vec<Box<[SystemSetId]>> = gating_by_new_idx
    .into_iter().map(|mut v| { v.sort_unstable_by_key(|s| s.0); v.dedup(); v.into_boxed_slice() })
    .collect();
```
**Why reuse, not rebuild**: `flatten_set_membership` (`schedule_builder.rs:791-889`) already computes transitive leaf membership with cycle detection, sorted + deduped (`schedule_builder.rs:876-877`). Phase 16 inverts it once at build for the conditioned subset. Zero new graph algorithm.

**A system's effective gate** = `AND(own conditions)` AND `AND over gating sets of set_gate(set_id))` — implemented in §3.3 + §6.

### Decision 7.3: `reset_for_frame` resets the memo bitsets

```rust
// executor_scratch.rs reset_for_frame — ADD:
self.cond_evaluated.clear();
self.set_cond_evaluated.clear();
self.set_cond_result.clear();   // result bits must be re-derived each frame
```
These three `clear()` calls are O(words) and run once per frame in `reset_for_frame` (`executor_scratch.rs:161`), alongside the existing `running`/`completed`/`ready_scratch` clears (`executor_scratch.rs:162-164`). Negligible. When `set_conditions` is empty, `set_cond_*` are zero-length bitsets and `clear()` is a no-op.

---

## §8 API + built-ins + `resource_exists` feasibility

### Decision 8.1: `SystemConfig::run_if`

```rust
// system_config.rs — SystemConfig gains:
/// Attaches a run condition to this system. The system runs in a frame only
/// if every attached condition returns `true` (eager AND — all conditions
/// run every frame so stateful conditions like `run_once` advance correctly).
///
/// A condition is any `impl IntoSystem<(), bool, M>` — e.g. a `fn() -> bool`,
/// `fn(Res<R>) -> bool`, or the built-in `run_once`. It MUST be read-only
/// (declare no component/resource writes); this is `debug_assert!`ed at build.
///
/// Multiple `.run_if(a).run_if(b)` chain into an AND.
#[inline]
pub fn run_if<C, M>(self, condition: C) -> Self
where
    C: IntoSystem<(), bool, M>,
    C::System: System<Out = bool> + 'static,
{
    let sys = C::into_system(condition);
    let boxed: BoolSystem = Box::new(sys);
    self.builder.descriptors[self.key.0].conditions.push(boxed);
    self
}
```
Mirrors `add_system`'s `IntoSystem` shape (`schedule_builder.rs:122-128`) but with `Out = bool`. The `BoolSystem` is pushed onto the descriptor's new `conditions` Vec (§2.2). Build-time only; no hot-path cost.

### Decision 8.2: `ConfigureSet::run_if`

```rust
// schedule_builder.rs — ConfigureSet gains:
/// Attaches a run condition to this set. Every member of the set (transitive)
/// runs in a frame only if every set condition returns `true`. The set
/// condition is evaluated ONCE per frame (memoized), not per member.
#[inline]
pub fn run_if<C, M>(self, condition: C) -> Self
where
    C: IntoSystem<(), bool, M>,
    C::System: System<Out = bool> + 'static,
{
    let sys = C::into_system(condition);
    let boxed: BoolSystem = Box::new(sys);
    self.builder.set_conditions.entry(self.set_id).or_default().push(boxed);
    self
}
```
Keyed by the SAME `set_id` the handle already holds (`schedule_builder.rs:441`), which `set_id_of_value` (`schedule_builder.rs:146`) guarantees matches the membership id.

### Decision 8.3: `run_once` built-in

```rust
// NEW module: schedule/common_conditions.rs (or system/conditions.rs).
use crate::ecs::core::system::params::local::Local;

/// A condition that returns `true` exactly once (the first frame it is
/// evaluated) and `false` forever after. Backed by a `Local<bool>` that flips
/// on first run. Because conditions are eager-folded (never short-circuited),
/// the `Local` advances every frame it is reached.
#[inline]
pub fn run_once(mut has_run: Local<bool>) -> bool {
    if *has_run {
        false
    } else {
        *has_run = true;
        true
    }
}
```
`run_once` is a plain `fn(Local<bool>) -> bool`, which is `impl IntoSystem<(), bool, M>` via the `SystemParamFunction` blanket (`function_system.rs:52-75` → `into_system.rs:78-89`) because `Local<bool>: SystemParam` (`local.rs:98`) and `bool` is the `Out`. The `Local<bool>` state lives in the condition's own `FunctionSystem::state` (`function_system.rs:116`), initialized to `false` at build (`local.rs:103-107` → `Default`), persisted across frames. No engine plumbing — it falls out of the existing machinery.

### Decision 8.4: `resource_exists` — DEFER (feasibility verdict)

**Verdict: NOT FEASIBLE in Phase 16 without a new SystemParam.** Grounded:
- `resource_exists::<R>` in Bevy is `fn(res: Option<Res<R>>) -> bool { res.is_some() }` (research §1).
- boyko has **no** `impl SystemParam for Option<T>` (grep `impl.*SystemParam for Option` → no matches).
- `Res<R>::get_param` calls `missing_resource_panic::<R>()` when absent (`res.rs:128-130`) — so a `fn(Res<R>) -> bool` condition would PANIC instead of returning `false` when the resource is missing. Unusable for existence testing.
- `EcsMaster::contains_resource::<R>()` exists (`ecs_master.rs:1807`), but no `SystemParam` exposes it, and a condition body can only consume `SystemParam`s (the `SystemParamFunction` contract, `function_system.rs:64`).

**Conclusion**: ship `run_if` + `run_once` in Phase 16; defer `resource_exists` until an `Option<Res<R>>` (or a dedicated `Has<R>` predicate-param) SystemParam exists. File as a Phase 16.x follow-up: "add `impl SystemParam for Option<Res<R>>` returning `None` instead of panicking, then `resource_exists::<R>` becomes a 1-line built-in." This matches the brief's instruction ("ship `resource_exists` ONLY if `Option<Res<R>>` is supported — VERIFY; if not, defer").

### Decision 8.5: Read-only requirement — documented + `debug_assert!`, no new bound

**What**: NO `ReadOnlySystem` marker trait is added in Phase 16. The read-only requirement is (a) documented on `run_if`, (b) enforced by a build-time `debug_assert!` that the condition's `Access` declares no writes:
```rust
debug_assert!(
    cond.access().component_writes.is_empty() && cond.access().resource_writes.is_empty(),
    "Phase 16 CR1: run condition '{}' declares writes; conditions must be read-only", cond.name(),
);
```
(`Access::component_writes.is_empty()` — `component_mask.rs:149`; `resource_writes.is_empty()` — `bit_set_256.rs:69`. The `Access` is populated by `initialize` at build, `function_system.rs:222-230`.)

**Why no marker trait**: boyko has no `ReadOnlySystem` today (research §4). Adding it would require a new trait + impls across every read-only `SystemParam` — large surface for a guarantee the `debug_assert` already provides at build. The condition runs single-threaded at the apply barrier (§3), so even a buggy write-declaring condition would NOT race (it holds the exclusive `&mut`); the `debug_assert` catches the API misuse (a condition that mutates the world is a logic error — Bevy forbids it via the `ReadOnlySystem` bound; we forbid it via the assert + docs). A write-declaring condition in release would run and mutate — undesirable but sound (single-threaded). The assert makes it a debug-build panic, which is the right severity. A future phase can promote this to a `ReadOnlySystem` bound if needed.

**Trade-off**: release builds don't enforce read-only. Accepted — the soundness argument (single-threaded eval) does not depend on read-only-ness; only the "conditions shouldn't have side effects" API contract does, and that's a debug-checked logic invariant.

---

## §9 Wave / Step plan

Steps grouped into waves; independent steps within a wave can be developed in parallel (different files, no sequential dependency).

### Wave A — types + storage (no executor change yet)
1. **`BoolSystem` alias + `SystemDescriptor::conditions`** (`system_descriptor.rs`): add `type BoolSystem = Box<dyn System<Out = bool>>` (in `system_box.rs` or a new `schedule/mod.rs` export), add `conditions: Vec<BoolSystem>` field, seed in `new`.
2. **`ExecutorScratch` memo bitsets** (`executor_scratch.rs`): add `cond_evaluated`, `set_cond_evaluated`, `set_cond_result`; allocate in `new` (sized `system_count` / `set_condition_count`); clear in `reset_for_frame`. `new` gains a `set_condition_count: usize` param.
3. **`EcsMaster::run_condition`** (`ecs_master.rs`): the `&mut dyn System<Out=bool>` runner (§5), two `// SAFETY:` blocks.

### Wave B — builder API + build pipeline
4. **`SystemConfig::run_if`** (`system_config.rs`): §8.1.
5. **`ScheduleBuilder::set_conditions` + `ConfigureSet::run_if`** (`schedule_builder.rs`): add the `HashMap` field, seed in `new`, add the method (§8.2).
6. **Build: initialize conditions + read-only assert** (`schedule_builder.rs` Step 1): §2.5 init loop extension.
7. **Build: assemble `Schedule` Phase-16 fields** (`schedule_builder.rs` Step 10 + new sub-steps): move `conditions` out per-descriptor into `system_conditions`; build `system_gating_sets` from `transitive_members` + `reorder` (§7.2); build `set_conditions` (flatten the `set_conditions` HashMap into the `SetConditionEntry` Vec with dense `slot`s, initializing each `BoolSystem`); compute `has_condition`. Thread the four fields into the `Schedule { ... }` constructor (`schedule_builder.rs:422-427`) and pass `set_conditions.len()` to `ExecutorScratch::new`.

### Wave C — executor integration
8. **`Schedule` struct fields** (`schedule.rs`): add `has_condition`, `system_conditions`, `system_gating_sets`, `set_conditions` (§2.4).
9. **`evaluate_ready_conditions` + `mark_skipped` + `set_gate`** (`schedule.rs`): the pass + helpers (§3.3, §7.1).
10. **Wire Step 1.5 into `executor_main_loop`** (`schedule.rs`): the gated condition-eval step (§3.2). `try_dispatch_ready` UNCHANGED.

### Wave D — built-ins + exports
11. **`run_once`** (`schedule/common_conditions.rs`, new file): §8.3. Export via `schedule/mod.rs`.

### Wave E — validation
12. **Tests** (§10): unit + integration + 0%-regression A/B + Miri.

Dependency edges: Wave A → B → C (sequential within the build→executor chain); D independent of C; E last. Within Wave A, steps 1/2/3 are independent (different files). Within Wave B, step 4 independent of 5/6/7; 6 and 7 both touch `build` (sequence them).

---

## §10 Test surface

### Unit (in-module `#[cfg(test)]`)
- **`run_if_stores_condition`** (`system_config.rs`): `.run_if(|| true)` pushes one `BoolSystem` onto the descriptor's `conditions`; `.run_if(a).run_if(b)` pushes two.
- **`configure_set_run_if_stores`** (`schedule_builder.rs`): `configure_set(S).run_if(c)` populates `set_conditions[S]`.
- **`build_initializes_conditions`** (`schedule_builder.rs`): a condition with a `Res<R>` param has its `Access` populated post-build (init ran); `has_condition` bit set for the conditioned system.
- **`has_condition_clear_when_no_run_if`** (`schedule_builder.rs`): a schedule with no `.run_if` has `has_condition.is_clear() == true`.
- **`mark_skipped_decrements_successors`** (`schedule.rs`): construct a 3-system DAG `a → b`, skip `a`, assert `pred_remaining[b]` dropped by 1 and `completed[a]` set.
- **`run_condition_returns_bool`** (`ecs_master.rs`): `run_condition` on a `BoolSystem` wrapping `|| true` returns `true`; on `|res: Res<R>| res.0 == 5` returns the right verdict.
- **`evaluate_ready_conditions_reads_only_conditioned`** (`schedule.rs`): a probe condition increments a counter; after one pass, only conditioned ready systems' conditions ran.

### Integration (`tests/phase16_run_conditions.rs`)
- **`system_skipped_when_condition_false`**: a system whose `.run_if(|| false)` is attached never runs its body; a counter stays 0. The frame still terminates.
- **`system_runs_when_condition_true`**: `.run_if(|| true)` → body runs once per frame.
- **`run_once_runs_exactly_once`**: a system with `.run_if(run_once)` runs on frame 1, not on frames 2/3 (the `Local<bool>` state persists). **The `run_once`-state test.**
- **`skip_successor_still_runs`** (**the skip-successor test**): `a.run_if(|| false)`, `b.after(a)`. Assert `b` STILL runs (its `before` successor relationship is honored — `a`'s skip decremented `b`'s `pred_remaining`). Assert `a`'s body did NOT run.
- **`skip_cascade_single_pass`**: `a.run_if(|| false)`, `b.after(a)`, `c.after(b)`, all in one frame; assert `b` and `c` run (b's pred from a satisfied via skip; c's pred from b satisfied via b's real completion next iteration). Validates the cascade settles.
- **`set_condition_gates_all_members`**: `configure_set(S).run_if(|| false)`; three systems `in_set(S)`. Assert none run.
- **`set_condition_evaluated_once_per_frame`** (**the set-once-per-frame test**): a set condition increments a shared `AtomicUsize` each time its body runs; attach to a set with 5 members; run one frame; assert the counter is EXACTLY 1 (not 5). Run a second frame; assert 2.
- **`eager_fold_advances_all_locals`**: a system with `.run_if(|| false).run_if(run_once)`; assert that after 2 frames the `run_once` `Local` has advanced (i.e. the second condition ran both frames despite the first being false) — verify by then flipping the first condition to true and observing `run_once` already returns false. Validates NO short-circuit.
- **`multiple_conditions_and`**: `.run_if(cond_a).run_if(cond_b)`; body runs iff both true (test all four combinations).
- **`mixed_conditioned_unconditioned_parallel`**: a schedule with some conditioned and some unconditioned systems; assert unconditioned ones run unaffected and parallelism is preserved (the conditioned ones gate correctly).

### 0%-regression A/B (criterion, the high-stakes item)
- **`bench_50_systems_no_conditions`** (`benches/`): the Phase 15 50-empty-systems bench, run on (1) pre-Phase-16 baseline via `git stash`, (2) Phase 16 with zero `.run_if`. Target: within ±2% (criterion "no change detected"), same methodology as `PHASE-15-RESULTS.md:67-72`. Confirm the `try_dispatch_ready` asm is byte-identical (diff `schedule.rs` dispatch loop) and `SystemBox` layout unchanged (the `const _: assert!` size check if added).
- **`bench_50_systems_all_conditioned_true`**: 50 systems each with `.run_if(|| true)` — measures the per-frame condition-eval overhead (one `run_condition` per system). Informational, not a gate.

### Property-based (`proptest`)
- **`skip_set_is_deterministic`**: for a random DAG + random condition verdicts (fixed per run), two `Schedule::run` calls with the same world state produce the same set of executed systems. Validates determinism (Proof d).
- **`pred_remaining_never_underflows`**: random DAG + random skips; assert no `mark_skipped` decrement underflows (debug build, the `debug_assert` would panic). Validates Proof c.

### Miri (`tests/miri_phase16.rs`, `-Zmiri-tree-borrows`) — the race-freedom check
- **`miri_condition_eval_no_ub`**: a multi-threaded pool (2 workers), a schedule mixing parallel unconditioned systems + a conditioned system reading a `Res<R>`. Run several frames under Miri. The load-bearing check: the `cell.world_mut()` reborrow inside Step 1.5 (when `running == 0`) does not alias any worker cell → no Tree Borrows protected-tag violation. NB: per `project-phase-9-complete` memory, multi-thread Miri on `Scope::spawn` has a known protected-tag deferral; if it blocks, run the SINGLE-threaded executor path under Miri (force `num_threads(1)`) to exercise `evaluate_ready_conditions` + `run_condition` + `mark_skipped` for UB, and document the multi-thread deferral as in Phase 9.
- **`miri_run_once_local_state`**: under Miri, `run_once` across 3 frames — assert no UB in the `Local<bool>` state borrow through `run_condition` → `run_unsafe` → `get_param` (`local.rs:123-132`).

---

## §11 Risk register

| # | Risk | Severity | Mitigation |
|---|---|---|---|
| R1 | **0%-regression leak** — `is_clear()` per iteration measurably costs on the 50-systems bench | HIGH | §4 gate is one not-taken branch in the existing cost class; A/B bench is the gate; fallback = cache `has_any_condition: bool`. |
| R2 | **Race** — condition eval aliases a live worker's cell | CRITICAL | §3.2 evaluates ONLY when `running.count_ones() == 0`; Proof (a); Miri test. Identical reasoning to the inline-exclusive path already in production (`schedule.rs:506-521`). |
| R3 | **Underflow** — a system both skipped and run, or an edge decremented twice | HIGH | Proof (c); `cond_evaluated` bitset makes skip-at-most-once; `try_dispatch_ready` skips `completed`; `debug_assert!` guard retained in `mark_skipped`. |
| R4 | **Stateful condition runs twice/frame** (e.g. `run_once` advances twice) | HIGH | `cond_evaluated[i]` (system) + `set_cond_evaluated[slot]` (set) memoize once-per-frame; reset in `reset_for_frame`; the `eager_fold_advances_all_locals` + `set_condition_evaluated_once_per_frame` tests. |
| R5 | **Skip cascade not settled in one pass** — a chain of skipped-then-conditioned successors | MEDIUM | Topo-order forward pass + skip-only-lowers-`pred_remaining` ⇒ successors (always topo-later) seen with updated counts; `skip_cascade_single_pass` test. Systems made ready via REAL completions handled next loop iteration (existing rhythm). |
| R6 | **`Box<dyn System>: System` confusion** — `run_cached_system` can't take an erased box | MEDIUM | §5.1 Option A: dedicated `run_condition(&mut dyn System<Out=bool>)`, one vtable call, no blanket impl. |
| R7 | **`resource_exists` shipped accidentally** despite missing `Option<Res>` | LOW | §8.4 explicit defer + feasibility verdict; not in the Wave plan. |
| R8 | **Write-declaring condition mutates world in release** | LOW | Sound (single-threaded eval); `debug_assert!` catches in debug (§8.5); documented read-only contract. Future `ReadOnlySystem` bound. |
| R9 | **Set-condition memo borrow conflict** (`&mut self.set_conditions` vs `&mut self.executor_scratch`) | LOW | §7.1: index-by-position, not held iterator; disjoint fields. |
| R10 | **Tick staleness** — a `Changed<T>` condition mis-reports | MEDIUM | §5.2: `run_condition` doesn't advance ticks; shipped built-ins (`run_once`) don't use ticks; Open Question §12 documents the limitation; defer tick-aware conditions. |
| R11 | **Condition `apply` side effects** — a condition with `Commands` would defer a command at the barrier | LOW | Read-only `debug_assert` (§8.5) catches `Commands` (it declares no writes but... actually `Commands` declares no access — see note). The eval runs at the apply barrier where `apply` is legal; a deferred command would flush into `world` correctly. Documented: conditions should not use `Commands`. |

**Note on R11**: `Commands` declares ZERO access (like `Local`), so the read-only `debug_assert` (which checks `Access` write bits) would NOT catch a condition that uses `Commands`. This is acceptable for Phase 16 — a `Commands`-using condition is a logic error, and its `apply` (run via `run_condition`, `ecs_master.rs:1700`) would flush correctly at the barrier (`evaluate_ready_conditions` holds the exclusive `&mut world`, and the deferred-hook drain runs in `apply_window_drain` on subsequent iterations). Document "conditions must be pure read-only predicates; do not use `Commands`/`EventWriter` in a condition." A future `ReadOnlySystem` bound would forbid it at compile time.

---

## §12 Open questions

1. **Tick-aware conditions (`Changed<T>`, `resource_changed`)**: `run_condition` does not call `set_change_ticks` (§5.2), so a condition's meta ticks stay at the `initialize` sentinel and `Changed<T>`-based conditions would mis-report. Options: (a) defer all tick-based conditions to a later phase (chosen for Phase 16 — only `run_once` ships); (b) have `evaluate_ready_conditions` call `cond.set_change_ticks(world.last_check_tick?, this_run)` before each `run_condition` — but `this_run` was already bumped at frame start (`schedule.rs:134`) and the condition's `last_run` semantics across frames need design. **Recommendation**: defer; document that conditions are tick-agnostic in Phase 16.

2. **`resource_exists` follow-up scope**: should the `Option<Res<R>>` SystemParam (returning `None` instead of panicking) be its own micro-phase (Phase 16.1) or folded into a future "resource ergonomics" phase? It unblocks `resource_exists`, `resource_equals`, `resource_changed`. **Recommendation**: file as Phase 16.1, independent of this plan.

3. **`has_any_condition: bool` cache vs `is_clear()` scan**: keep the `is_clear()` scan (sub-ns on ≤16 words) or add the cached bool? **Recommendation**: ship with `is_clear()`; add the bool ONLY if the A/B bench (§10) shows measurable cost. Decided by measurement, per the inlining/measurement principle.

4. **Condition eval when `running > 0` but a conditioned system is ready**: §3.2 defers condition eval to a later iteration (parks until workers complete). This is slightly less eager than Bevy (which can evaluate while unrelated systems run, since conditions are read-only). For Phase 16 the simpler "eval only when `running == 0`" is chosen for an airtight race-freedom proof. **Open**: a future optimization could evaluate a condition concurrently with running systems IF the condition's read `Access` is disjoint from every running system's write `Access` (i.e. add conditions to the conflict graph). Deferred — the brief explicitly says conditions need NOT be in the conflict graph, and the once-per-barrier eval is correct. Possible parallelism left on the table; benchmark-gated.

5. **`set_cond_result` as `FixedBitSet` vs `Vec<bool>`**: a packed bitset is denser but `set(slot, v)` is a bit op vs a byte store. For the tiny set-condition counts expected (handful per schedule), either is fine. **Recommendation**: `FixedBitSet` for consistency with the other scratch bitsets; revisit if set-condition counts grow large.

---

## Readiness checklist

**Plan structure**: goal stated in perf+functional terms (§1); target metrics concrete (0% / ±2% bench, tens-of-ns per condition, §4/§5); every decision justified via race/cache/perf; alternatives rejected with reasons (§2.1, §3.1, §5.1, §8.5); trade-offs listed (each Decision).

**Data structures**: each field typed + commented (§2.2-2.4); `FixedBitSet` chosen for `has_condition`/memo (consistency + density); `SystemBox` explicitly NOT widened (§2.1, §4); field order rationale given (§2.4). N/A: cache-line padding — these fields are dispatcher-owned single-threaded scratch (no false sharing; `ExecutorScratch` doc `executor_scratch.rs:8-23` already documents the dispatcher-sole-mutator discipline).

**API**: minimal (`run_if` ×2 + `run_once`); no internal types leak (`BoolSystem` is `pub(crate)`); generics where specialization needed (`IntoSystem<(), bool, M>`); no `dyn Trait` in the hot path (the `dyn System<Out=bool>` is touched only in the gated Step 1.5, never on the no-condition path).

**Multithreading**: model explicit (single-threaded eval at the apply barrier, §3); the one new `unsafe` (cell reborrow) justified + Proof (a); no shared state added (memo bitsets are dispatcher-owned); `Send`/`Sync` unaffected (`BoolSystem` is `Box<dyn System>` which is `Send+Sync`, `system.rs:56`).

**Correctness**: edge cases — empty schedule (gate clear, no pass), all-systems-skipped (terminates via `completed == n`), skip cascade (§3.3 + Proof), underflow (Proof c); determinism (Proof d); drop order — `BoolSystem`s drop with `Schedule` (no special order); `unsafe` invariants stated (§5.1 CR3, §3.2 CR2).

**Integration**: affected modules listed (§9); API changes noted (`SystemDescriptor`/`ScheduleBuilder`/`ExecutorScratch`/`Schedule` gain fields; `EcsMaster` gains `run_condition`; `ExecutorScratch::new` signature gains a param); compatible with `Arena`/`ComponentPool`/`UnitId` (untouched — Phase 16 is schedule-layer only); implementation broken into steps (§9).

**Validation**: unit tests (§10); property tests (§10 determinism + underflow); benchmarks (§10 0%-regression A/B); `debug_assert!` invariants (read-only CR1 §8.5; underflow SCH13 in `mark_skipped` §3.3; the existing SCH15/SCH6 unchanged).

---

**Files this plan touches** (all absolute):
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\system_descriptor.rs` — add `conditions: Vec<BoolSystem>`.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\system_config.rs` — add `run_if`.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\schedule_builder.rs` — `set_conditions` field, `ConfigureSet::run_if`, build-pipeline extensions (Step 1 init+assert, Step 10 assembly, `system_gating_sets` from `transitive_members`).
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\executor_scratch.rs` — `cond_evaluated`/`set_cond_evaluated`/`set_cond_result`; `new` param; `reset_for_frame` clears.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\schedule.rs` — four new `Schedule` fields; `evaluate_ready_conditions`/`mark_skipped`/`set_gate`; Step 1.5 in `executor_main_loop`. `try_dispatch_ready` UNCHANGED.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\common_conditions.rs` — NEW, `run_once`.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\mod.rs` — export `BoolSystem`, `run_once`.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` — add `run_condition(&mut dyn System<Out=bool>) -> bool`.
- `D:\claude\BoykoEngine\crates\boyko_ecs\tests\phase16_run_conditions.rs`, `tests\miri_phase16.rs` — NEW test files.
- `D:\claude\BoykoEngine\crates\boyko_ecs\benches\` — extend the 50-systems bench with conditioned variants.

**Untouched (hot path, 0%-protected)**: `conflict_graph.rs`, `bitset_intersects.rs`, `system_box.rs` (layout + `Out=()` pin), and `try_dispatch_ready` (the dispatch loop body).
