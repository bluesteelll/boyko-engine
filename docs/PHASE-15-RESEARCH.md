The Bash tool is not available in this agent context. I do not have a Write tool either. My available tools are read-only (Read, Glob, Grep) plus web tools. 

The system-reminder explicitly states: "Do NOT Write report/summary/findings/analysis .md files. Return findings directly as your final assistant message — the parent agent reads your text output, not files you create." Combined with the absence of any write capability, the correct action is to **return the complete research findings directly as my final message**. The orchestrator will read this output and can persist it.

I have done exhaustive research. I'll now deliver the complete, comprehensive Phase 15 research report as my final response, formatted as the intended document content (so the orchestrator/architect can use or save it directly).

---

# Phase 15 Research — Explicit System Ordering & Schedule Sets

I was unable to write `docs/PHASE-15-RESEARCH.md` to disk — this agent context has no file-write tool (Bash/Write unavailable; only read-only Read/Glob/Grep + web tools). Per the harness guidance to return findings directly, the **complete report is below** for the orchestrator to persist verbatim to `D:\claude\BoykoEngine\docs\PHASE-15-RESEARCH.md`. All boyko file:line citations are verified against the current `ecs` branch.

> **Scope note.** RESEARCH ONLY. §4 (interaction analysis) and §5 (API shape) are flagged as INPUT TO THE ARCHITECT, not decisions.

## TL;DR — what the architect must know

- **boyko already has ~70% of the Phase 15 scaffold, dormant since Phase 9.** `OrderingEdge` (`Before`/`After`/`ChainConsecutive`/`InSet`), `SystemKey`, the `SystemSet` trait + `SystemSetId`, `SystemConfig` with working `.before()`/`.after()`/`.chain()`/`.in_set()`, and the full Tarjan-SCC + Kahn-topo + `ConflictGraph` build pipeline are all present and TESTED. The ONE missing piece is **set-membership expansion** (`InSet` → pairwise edges) plus **set-level ordering** (ordering a whole set before/after another). `schedule_builder.rs:269-273`'s `insert_sync_points` is a documented no-op; `system_config.rs:100-101` says in-set is "recorded but **not yet expanded**". Phase 15 is mostly *finishing* Phase 9's deferred "Wave 5 Step 14", not greenfield work.

- **The crux question is already answered — and boyko's existing code is correct.** Bevy's executor keeps TWO separate structures: a per-system dependency *count* (`num_dependencies_remaining`, gates readiness/order) and a per-system *conflict bitset* (`conflicting_systems`, gates concurrent dispatch). boyko has the exact same split: `pred_remaining` (readiness, `schedule.rs:451`) and `conflict_bits` (dispatch, `schedule.rs:458-463`). The subtlety: a Bevy `before`/`after` edge **forces non-overlapping (serial) execution of the two ordered systems even when they do NOT conflict on data** — confirmed verbatim by Bevy maintainers (discussion #2747). boyko's `ConflictGraph::build` ALREADY does this: `conflict_graph.rs:146-149` sets a "false conflict" bit for every ordering edge. So **boyko's current behavior is byte-for-byte Bevy's `before` semantics.** No redesign of the interaction model is required.

- **Deferred-command visibility across an ordering edge already works in boyko, for free.** boyko decrements a successor's `pred_remaining` only AFTER the predecessor's `apply()` (command flush) completes inside the apply-window drain (`schedule.rs:350-372`). So a downstream ordered system always sees the upstream's `Commands` effects — this is Bevy's `before()` (with-sync) semantics, not `before_ignore_deferred()`. boyko has no explicit `ApplyDeferred` node and does not strictly need one for correctness.

- **Ordering is resolved at BUILD time in every engine — never per-frame.** Bevy rebuilds only on `Schedule::initialize()` after a topology change; flecs caches the pipeline query and rebuilds on dirty; DOTS re-sorts a group only when membership changes. boyko's `ScheduleBuilder::build` produces an immutable `Schedule`; per-frame `reset_for_frame` is just a `clear()` + `[u16]` copy (`executor_scratch.rs:161-191`). **The 0%-regression constraint is structurally satisfiable** — all Phase-15 work lands in `build`, not in `run`.

- **Three identification strategies exist; boyko already chose the leanest.** Bevy targets ordering at a `SystemSet` (a bare function becomes `SystemTypeSet<F>`, ambiguous if the function is registered twice). specs uses string labels. boyko uses an opaque `SystemKey` handle returned by `add_system` — type-safe, zero-`dyn`, no TypeId-of-closure problem, no ambiguity. Sound divergence aligned with principle #1, but it costs ergonomics (you must hold the handle). The architect must decide: keep handles, add a `SystemSet`-targeted layer, or both.

- **flecs is the outlier: NO per-system before/after at all.** flecs orders strictly by coarse *phases* (8 built-ins, topologically sorted via `DependsOn`) + declaration order within a phase, and parallelizes *data-parallel* (one system at a time, entities split across threads). Bevy/boyko/DOTS are *task-parallel* (different systems concurrently). Because boyko is task-parallel, **Bevy is the correct reference model**, not flecs.

---

## §0 — What boyko already has (the Phase 9 dormant scaffold)

The single most important fact: **Phase 9 already built most of the Phase 15 surface and stubbed the hard part.** Phase 15 is the completion of "Wave 5 Step 14", referenced throughout the schedule module.

### Present and TESTED

| Piece | File:line | State |
|-------|-----------|-------|
| `OrderingEdge` enum (`Before`/`After`/`ChainConsecutive`/`InSet`) | `ordering.rs:54-72` | present, `as_dag_edge` tested |
| `SystemKey` opaque handle | `ordering.rs:33` | present |
| `SystemConfig::before/after/chain` | `system_config.rs:65-94` | present, push `OrderingEdge` |
| `SystemConfig::in_set` | `system_config.rs:103-114` | records membership, edge NOT expanded |
| `SystemSet` trait + `SystemSetId` | `system_set.rs:31-53` | present (TypeId-keyed, no derive) |
| `SystemDescriptor` (`ordering_hints`, `sets`) | `system_descriptor.rs:39-54` | present |
| `ScheduleBuilder` (`sets`, `set_members` maps) | `schedule_builder.rs:54-70` | present |
| Edge collection from hints | `schedule_builder.rs:200-204` | tested (`InSet` filtered → `None`) |
| Tarjan SCC cycle detection | `schedule_builder.rs:374-456` | tested (`cycle_in_before_after_panics`) |
| Kahn topo sort (FIFO, stable) | `schedule_builder.rs:471-498` | tested (`topological_sort_respects_before`) |
| `ConflictGraph::build` (conflict bits + DAG edges) | `conflict_graph.rs:97-176` | tested |
| Executor two-check dispatch | `schedule.rs:444-485` | tested |

### MISSING (the Phase 15 deliverable)

1. **Set-membership expansion** (`InSet(a, set)` → pairwise edges). Today `as_dag_edge` returns `None` for `InSet` (`ordering.rs:88`), so set membership contributes ZERO ordering. The builder destructure discards `_sets`/`_set_members` (`schedule_builder.rs:159-160`).
2. **Set-level ordering** — no API to say "set S before set T" or "system X before set S". `SystemConfig::before` takes a `SystemKey`, not a `SystemSetId` (`system_config.rs:65`). No `configure_sets` equivalent.
3. **Set hierarchy** (a set `in_set` another set) — no representation.
4. **Auto sync-point insertion** — `insert_sync_points` is a no-op pass-through (`schedule_builder.rs:355-360`). Correctness-neutral today (the per-system apply window is already a sync point — `schedule_builder.rs:307-353`), so Phase 15 may legitimately leave it stubbed and treat it as a future parallelism optimization.
5. **`#[derive(SystemSet)]` macro** — `boyko_macros` has `Component` + `event` but no `SystemSet`; users `impl SystemSet for MySet {}` by hand (`system_set.rs:44-51`).
6. **Missing-target / diagnostics** — no handling for a `SystemKey` from a different builder; today it silently indexes the wrong descriptor or panics on OOB in debug only (`conflict_graph.rs:128-139`).

### Two structures the executor already uses (critical for §4)

`ConflictGraph` (`conflict_graph.rs:65-78`) holds:
- `pred_count: Box<[u16]>` — in-degree in the **ordering DAG only** (NOT conflicts). Seeds `pred_remaining` each frame. Gates *readiness/order*.
- `successors: Box<[Box<[SystemIndex]>]>` — ordering-DAG out-edges.
- `conflict_bits: Box<[FixedBitSet]>` — bit `j` set iff `i`,`j` can't run concurrently. Set for BOTH access conflicts (`conflict_graph.rs:108-118`) AND ordering edges (`conflict_graph.rs:146-149`).

This maps 1:1 onto Bevy's `SystemSchedule`: `system_dependencies`=`pred_count`, `system_dependents`=`successors`, `conflicting_systems`=`conflict_bits`.

---

## §1 — Bevy's system ordering model (primary reference)

Bevy is the correct primary reference: like boyko it is **task-parallel**, uses a **single flat schedule graph** (since 0.10 "stageless"), and resolves ordering at **build time** into per-system dependency counters the executor decrements at runtime.

### 1.1 Why Bevy moved to a flat graph (historical lesson)
Pre-0.10 Bevy used hard **stages** (barriers). The 0.10 "stageless" release (March 2023, RFC #45) replaced them with one unified `Schedule` whose nodes are systems AND system sets in a single DAG. Verbatim motivation:
> "Have you ever wanted to specify that `system_a` runs before `system_b`, only to be met with confusing warnings that `system_b` isn't found because it's in a different stage?" — Bevy 0.10 announcement

Relevant to boyko: **boyko has a single flat schedule with no built-in stages/phases**, natively avoiding the "different stage, ordering silently ignored" class of bug.

### 1.2 Public API (verified in `config.rs`)
```rust
// all take `impl IntoSystemSet<M>` — a SET, or a bare system (auto-wrapped):
fn before<M>(self, set: impl IntoSystemSet<M>) -> ScheduleConfigs<T>;
fn after<M>(self, set: impl IntoSystemSet<M>) -> ScheduleConfigs<T>;
fn before_ignore_deferred<M>(self, set: impl IntoSystemSet<M>) -> ...;
fn after_ignore_deferred<M>(self, set: impl IntoSystemSet<M>) -> ...;
fn in_set(self, set: impl SystemSet) -> ScheduleConfigs<T>;       // join a set
fn chain(self) -> ScheduleConfigs<T>;             // a->b->c for tuple (a,b,c)
fn chain_ignore_deferred(self) -> ScheduleConfigs<T>;
fn run_if<M>(self, condition: impl Condition<M>) -> ...;          // Phase 16
fn ambiguous_with<M>(self, set: impl IntoSystemSet<M>) -> ...;    // escape hatch
fn ambiguous_with_all(self) -> ...;
```
Representative user code (Cheatbook / Tainted Coders):
```rust
app.add_systems(Update, (defend, attack, end_turn).chain());     // chain
app.add_systems(Update, (
    defend.before(end_turn),
    attack.after(defend),
    end_turn,
));
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
enum InputSet { Touch, Mouse, Gamepad }
app.configure_sets(Update, (EconomySet, PhysicsSet, InputSet::Touch).chain());
app.add_systems(Update, (defend, attack, end_turn).chain().in_set(EconomySet));
player_footsteps.in_set(MyAudioSet).in_set(MyGameplaySet::Player); // multi-set
```

### 1.3 Internal representation (`config.rs`, `graph/mod.rs`)
```rust
pub struct GraphInfo {
    pub(crate) hierarchy: Vec<InternedSystemSet>,  // sets this node is in_set of
    pub(crate) dependencies: Vec<Dependency>,      // before/after edges
    pub(crate) ambiguous_with: Ambiguity,          // escape hatch
}
pub(crate) struct Dependency {
    pub(crate) kind: DependencyKind,               // Before | After
    pub(crate) set: InternedSystemSet,             // the TARGET (always a set)
    pub(crate) options: TypeIdMap<Box<dyn Any>>,   // carries IgnoreDeferred
}
pub(crate) enum DependencyKind { Before, After }   // no-sync rides in `options`
```
`before_inner`/`after_inner` push `Dependency::new(DependencyKind::Before, set)` after `set.into_system_set().intern()`. So **the target of every ordering edge is a `SystemSet`**, never a system handle directly. `chain_inner` calls `metadata.set_chained()`, later emitting `Before` edges between consecutive tuple elements.

### 1.4 SystemSet identification — the `SystemTypeSet` trick
A bare function `F` impls `IntoSystemSet` producing `SystemTypeSet<F>`:
```rust
impl<Marker, F> IntoSystemSet<(IsFunctionSystem, Marker)> for F
where F: SystemParamFunction<Marker, ...>
{ type Set = SystemTypeSet<F>; fn into_system_set(self) -> Self::Set { SystemTypeSet::<F>::new() } }
```
`SystemTypeSet<T>` returns `Some(TypeId::of::<T>())`. Verbatim limitation:
> "You cannot order something relative to one if it has more than one member."

`#[derive(SystemSet)]` generates `dyn_clone`/`as_dyn_eq`/`dyn_hash` (via `define_label!`) so a `Box<dyn SystemSet>` compares/hashes type-erased — **the `dyn`-heavy machinery boyko's `SystemKey` approach avoids** (see §5).

### 1.5 Build pipeline — `ScheduleGraph::build_schedule` (crux for §4)
1. `hierarchy.analyze()` — topo-sort **set-membership** graph; cycle → `HierarchySort`.
2. `dependency.analyze()` — topo-sort **before/after** graph; cycle → `DependencySort`.
3. `hierarchy.group_by_key()` — compute set membership.
4. **Flatten** (`set_systems.flatten(...)`): REMOVE every set node from the dependency DAG and **replicate its edges to every member**. Verbatim: *"if `Set(A) → Node(Z)` and `System(X) ∈ A`, add `System(X) → Node(Z)`"*. Only systems remain as nodes afterward.
5. Build passes (`AutoInsertApplyDeferredPass`) modify the flattened graph.
6. **Ambiguity detection** — `systems.get_conflicting_systems(...)`: pairs with overlapping access but NO ordering edge (and not `ambiguous_with`).
7. `build_schedule_inner` — topo-sort flattened systems into `SystemSchedule`.

**Step 4 (flatten) is exactly the set-expansion pass boyko is missing.**

### 1.6 Sets nest; set-level before/after/run_if
`configure_sets` attaches `Dependency`/`run_if`/`hierarchy` to a SET node exactly like `add_systems` for systems — sets are first-class graph nodes. A set can be `in_set` another set; analyze + flatten handle arbitrary depth (cycles caught in step 1). A `run_if` on a set is evaluated ONCE per frame; false → whole set skipped (Phase 16 territory).

### 1.7 Ordering ↔ parallel executor — TWO SEPARATE mechanisms (decisive)
Bevy's `multi_threaded.rs` gates each system on two independent checks:
1. **Readiness** — `num_dependencies_remaining[sys] == 0` (flattened ordering in-degree). On completion, decrement dependents; push newly-zero into `ready_systems`.
2. **Concurrency** — `can_run`: `!system_meta.conflicting_systems.is_disjoint(&running_systems)` (data-access bitset).
> "A system can be 'ready' but unable to run if conflicts exist with active systems — proving these mechanisms are genuinely separate, not merged."

`SystemSchedule` (precomputed at build): `system_ids`, `system_dependencies` (in-degree), `system_dependents` (out-edges), `sets_with_conditions_of_systems`. Executor only decrements counters + tests bitsets per frame — NO per-frame topo sort.

**`before`-without-conflict, answered.** A `before` edge is HARD: the downstream's dep-count includes the upstream, so it cannot START until the upstream FINISHES, even with disjoint data. Verbatim (discussion #2747):
> "the current `before`/`after` edges force sequential execution regardless of actual data conflicts ... `after` cannot be implemented at all in terms of `as_if_after` without the ability to add some kind of false conflict"

So Bevy serializes two `before`-ordered non-conflicting systems w.r.t. each other (no time overlap), but either may still run parallel to unrelated third systems. An aspirational visibility-only `as_if_after` is NOT implemented.

### 1.8 ApplyDeferred / sync points relative to ordering
- `ApplyDeferred` (was `apply_system_buffers`): a special, effectively-exclusive system that flushes pending `Commands` of systems that ran. Runs alone between dependency levels.
- `AutoInsertApplyDeferredPass` (`auto_insert_apply_deferred.rs`, default-on via `ScheduleBuildSettings.auto_insert_apply_deferred=true`): topo-sorts, computes per-node `distance` (# sync points from start), checks `has_deferred()`, inserts an `ApplyDeferred` on edges where the upstream has deferred buffers and the edge is not `IgnoreDeferred` (`no_sync_edges`). **Coalesces** by distance — downstreams at equal distance reuse ONE node (`distance_to_explicit_sync_node`, `get_sync_point`). PR #16782 reuses explicit `ApplyDeferred` to avoid redundant syncs.
- `before()` (inserts sync, downstream sees flushed commands) vs `before_ignore_deferred()` (pure ordering, no flush). Same for `chain()` vs `chain_ignore_deferred()`.

**Mapping to boyko.** boyko has NO `ApplyDeferred` node / no auto-insert pass. Instead, the apply-window barrier flushes EVERY system's `Commands` (`SystemParam::apply`) inside the dispatcher's apply window, and a successor's `pred_remaining` is decremented only after the predecessor's `apply` returns (`schedule.rs:350-372`). So **boyko already provides Bevy's `before()` (with-sync) semantics for ordered systems** at the cost of "one extra dispatcher round per deferred system" (documented `schedule_builder.rs:336-347`) instead of coalesced sync points. boyko has no `_ignore_deferred` opt-out — fine for correctness, slightly less optimal for parallelism.

### 1.9 Cycle detection + error taxonomy + missing targets
- **When:** BUILD time, via `Dag::analyze()` on both hierarchy and dependency graphs.
- **Errors** (`ScheduleBuildError`): `HierarchySort` (membership cycle), `DependencySort`/`FlatDependencySort` (ordering cycle, incl. self-before-self), `CrossDependency` (ordered relative to a set it belongs to), `SetsHaveOrderButIntersect` (two ordered sets share a member), `SystemTypeSetAmbiguity` (ordered against a system type with >1 instance), `Uninitialized`. Redundant edges removed via `check_for_redundant_edges` (`hierarchy_detection`, default `Warn`).
- **Missing target:** `before(X)` where `X`'s set is never populated (wrong schedule) is **silently ignored** — an anonymous empty set node yields no edges after flatten. Documented footgun (Cheatbook; issue #7258): "compiles and runs, but doesn't work." boyko's `SystemKey` can do BETTER (a stale key is detectable; see §6).

---

## §2 — flecs pipeline / phases

flecs uses a fundamentally different model. **It has NO per-system before/after.** Ordering is by coarse **phases** plus declaration order within a phase.

### 2.1 Phases and DependsOn
- 8 built-in phases (entities tagged `EcsPhase`): `EcsOnLoad, EcsPostLoad, EcsPreUpdate, EcsOnUpdate, EcsOnValidate, EcsPostUpdate, EcsPreStore, EcsOnStore`.
- Systems join a phase via `kind(flecs::OnUpdate)` / `ECS_SYSTEM(world, Move, EcsOnUpdate, ...)`.
- Verbatim: *"Systems are ordered using a topology sort on the `DependsOn` relationship. Systems higher up in the topology are ran first."*
- Custom phases: `world.component<PreRender>().add(flecs::Phase).depends_on(flecs::PostUpdate);` — chain phases via `DependsOn`.

### 2.2 Within-phase order
Verbatim: *"Within a phase, they are ordered by declaration order."* / *"A pipeline by default orders systems by their entity id, to ensure deterministic order ... systems will be ran in the order they are declared, as entity ids are monotonically increasing."* Caveat: NOT guaranteed if entity recycling occurs before system creation — recommendation is to register systems before deleting entities, or supply a custom `order_by`.

### 2.3 Parallelism (the key contrast with Bevy/boyko)
flecs is **data-parallel, not task-parallel.** Verbatim: *"The scheduler runs each multithreaded system on all threads, and divides the number of matched entities across the threads"* (e.g. 1000 entities → thread 1 does 0–249, etc.). Systems within a phase run **sequentially** (one system at a time); each multi-threaded system splits its OWN entities across workers, then all sync before the next system. This is analogous to boyko's `Query::par_iter`, NOT to boyko's task-parallel scheduler.

### 2.4 Sync points / merge
Verbatim: *"Sync points are inserted automatically by analyzing which commands could have been inserted and which components are being read by systems ... When a pipeline sees a read for a component for which commands could have been inserted, a sync point is inserted before the system that reads."* Default world runs in **readonly mode** (ECS ops enqueued as commands per thread-local stage); at sync points (`ecs_readonly_end`) all stage queues merge sequentially for deterministic order. `immediate`/`no_readonly` systems run outside readonly mode and see structural changes directly.

### 2.5 Resolution timing
Pipeline (ordered system list + sync points) is a **cached query**, computed once and rebuilt only when the system set is dirty — NOT per `ecs_progress`.

### 2.6 What boyko can learn
- Mertens' rationale (verbatim, DesignWithFlecs): *"The granularity of control is at the module level, never at the individual system level. The reason for this is that modules may reimplement their features with different systems. If you have inter-system dependencies, those could break easily every time you update a module."* — A caution: fine-grained per-system edges create brittle cross-module coupling.
- Lesson for boyko: an OPTIONAL coarse phase/set sugar layer (Bevy's sets serve this role) is ergonomically valuable even atop a fine-grained edge engine. boyko's task-parallel core means it should NOT adopt flecs's phase-only model (which would serialize unrelated systems), but a small set of well-known sets (e.g. `First`/`Update`/`Last`) configured via set-ordering is the flecs ergonomic benefit without the parallelism cost.

---

## §3 — Unity DOTS / Entities (brief)

DOTS confirms the **two-orthogonal-layers** design that is exactly boyko's question.

- **Layer 1 — system update order (explicit, build-time):** `[UpdateInGroup(typeof(G))]` places a system in a `ComponentSystemGroup`; `[UpdateBefore(typeof(S))]` / `[UpdateAfter(typeof(S))]` order it relative to direct children of the same group. `OrderFirst`/`OrderLast` are implicit before/after-all. Cross-group order is **implied by group order** (if A∈GroupA, B∈GroupB, both in GroupC, then GroupA-vs-GroupB order determines A-vs-B). The group **topologically sorts** its children; verbatim: *"Every time you add a group to a system group, the group re-sorts the system update order for that group before updating again"* — i.e. at composition time, not per-frame.
- **Layer 2 — job dependency system (automatic, data-derived):** verbatim: *"When you schedule jobs, ECS keeps track of which jobs read and write which components. A job that reads a component is dependent on any prior scheduled job that writes to the same component and vice versa."* `AtomicSafetyHandle` enforces read/write safety. `ScheduleParallel()` auto-uses the system's `Dependency` property.
- **The composition:** system update order determines the ORDER jobs are *scheduled* (main thread); the job dependency system determines what runs *in parallel*. These are **separate** — exactly Bevy's `num_dependencies_remaining` vs `conflicting_systems` and boyko's `pred_remaining` vs `conflict_bits`.

---

## §4 — The interaction question (MOST IMPORTANT for boyko)

### 4.1 The two design options, against the evidence

**Option A** — one combined graph; topo-sort the union; run within topo-levels in parallel when conflict-free.
**Option B** — explicit ordering is a separate constraint layer the executor respects, distinct from the conflict graph that governs parallelism.

**What the engines actually do: Option B, unanimously.**
- Bevy keeps `num_dependencies_remaining` (ordering, readiness) SEPARATE from `conflicting_systems` (data, concurrency) — §1.7.
- DOTS keeps system-update-order SEPARATE from the job dependency system — §3.
- flecs keeps phase/DependsOn order SEPARATE from intra-system data parallelism — §2.

**boyko already implements Option B.** The executor's dispatch loop checks them as two distinct gates (`schedule.rs:444-463`):
```rust
if self.executor_scratch.pred_remaining[i] != 0 { continue; }            // gate 1: ORDERING (readiness)
if bitset_intersects(&self.conflict_graph.conflict_bits[i],
                     &self.executor_scratch.running) { continue; }       // gate 2: CONFLICT (concurrency)
```
`pred_remaining` is seeded from `pred_count` (ordering-DAG in-degree only — `conflict_graph.rs:68` doc: *"In-degree ... in the ordering DAG (not the conflict graph)"*). `conflict_bits` governs parallelism. **The executor is already correct and matches Bevy exactly. Phase 15 does not need to touch the executor.**

### 4.2 The "false conflict" subtlety — and why boyko is already right

The sharp sub-question: *does a `before` edge between two NON-conflicting systems force them onto different topo levels / serialize them?*

- **In Bevy: yes** (§1.7, discussion #2747 verbatim) — `before`/`after` is non-overlapping execution regardless of data conflict; Bevy explicitly needs "some kind of false conflict" to implement it.
- **In boyko: yes, and it already does it the same way.** `ConflictGraph::build` adds a conflict bit for EVERY ordering edge (`conflict_graph.rs:146-149`):
  ```rust
  // Ordered systems also share a conflict bit — the downstream
  // cannot run alongside the upstream regardless of access.
  conflict_bits[from_idx].insert(to_idx);
  conflict_bits[to_idx].insert(from_idx);
  ```
  So an ordering edge sets BOTH `pred_remaining` (forces direction: downstream waits) AND a conflict bit (forces non-overlap). This is precisely Bevy's `before` semantics. The module doc even states the rationale (`conflict_graph.rs:18-22`): *"Ordered systems share a conflict bit because the downstream cannot run alongside the upstream anyway — bundling the predicate into one bitset lets the executor answer 'can I dispatch sys i now?' with a single SIMD scan against the running bitset."*

**Is the conflict bit redundant given `pred_remaining` already serializes?** Subtle but worth the architect's attention:
- `pred_remaining[downstream] > 0` already prevents the downstream from dispatching until the upstream completes. So the *downstream-direction* serialization is guaranteed by `pred_remaining` alone.
- The conflict bit additionally makes the relationship **symmetric** in the `running` set: while the upstream is running, the conflict bit blocks the downstream from being co-dispatched (but `pred_remaining` already blocks it). And while the downstream's predecessors are incomplete it isn't a dispatch candidate anyway. **So for a pure A→B edge, the conflict bit is largely redundant with `pred_remaining`** — its presence is harmless (correctness preserved) but it is NOT free: `bitset_intersects` is a per-dispatch-round SIMD scan over `conflict_bits[i]` (`schedule.rs:458`, `bitset_intersects.rs`). For schedules with many ordering edges this slightly widens each conflict bitset.
- **Recommendation flag (architect to decide):** When Phase 15 adds many ordering edges (especially from set-expansion, which can create O(members²) edges), consider whether to KEEP the "ordering edge ⇒ conflict bit" rule. Two positions, both defensible: (a) **Keep it** — matches Bevy's "false conflict", trivially correct, and the SIMD scan is already AVX2-fast; (b) **Drop it for pure ordering edges** — rely on `pred_remaining` alone for direction, reserve `conflict_bits` for genuine data conflicts, shrinking the bitsets. This is a measurable micro-optimization, not a correctness issue; it should be benchmarked against the Phase 9 "50 systems" bench before changing. The conservative default is (a) — do not change a tested, correct invariant without a measured win.

### 4.3 Direction of a conflict between two CONFLICTING systems

The task asks: a `before` edge between conflicting systems is redundant w.r.t. the conflict edge (they can't run concurrently anyway), but the explicit edge fixes the DIRECTION. How does boyko decide direction today, and how does `before` override it?

- **Today (no explicit edge):** two conflicting systems get a conflict bit (`conflict_graph.rs:113-116`) but NO `pred_remaining` edge. So neither blocks the other on readiness; whichever has the lower `SystemIndex` is encountered first in the dispatch scan (`schedule.rs:444` iterates `0..n`) and dispatched first. `SystemIndex` is assigned by Kahn's topo sort, which for systems with no DAG edges falls back to **insertion order** (FIFO ready queue, `schedule_builder.rs:479-485`; documented `schedule_builder.rs:461-463`: *"ties break in insertion order"*). So the direction is **deterministic but implicit** = registration order. This is actually MORE deterministic than Bevy's default (Bevy's unordered conflicting pair is non-deterministic / "could change every frame"), but it is not user-controllable.
- **With an explicit `before`:** the edge adds a `pred_remaining` dependency that forces the direction regardless of insertion order, and re-orders `SystemIndex` via Kahn. `topological_sort_respects_before` (`schedule_builder.rs:626-651`) tests exactly this.

So boyko's answer to "how does `before` override direction": the explicit edge adds an ordering-DAG edge → Kahn places the predecessor first → `pred_remaining` enforces it at runtime. The pre-existing conflict bit (from the data conflict) is unaffected; the `before` edge just adds direction on top.

### 4.4 RECOMMENDATION for layering explicit edges onto Phase 9 (architect input)

Grounded in the actual code, the cleanest layering:

1. **Keep the executor untouched.** It already implements Option B correctly (`pred_remaining` + `conflict_bits`). Phase 15 is a BUILD-phase feature only.

2. **Add set-expansion BEFORE edge collection** in `ScheduleBuilder::build`, i.e. between Step 2 (capture names) and Step 3 (`schedule_builder.rs:198-204`). The expansion consumes the existing `set_members` map (`schedule_builder.rs:69`) and the new set-ordering edges, producing additional `(SystemKey, SystemKey)` pairs that flow into the SAME `dag_edges_keys` vec → SAME Tarjan/Kahn → SAME `ConflictGraph::build`. This mirrors Bevy's "flatten" (§1.5 step 4) exactly: a `Set(A).before(Set(B))` becomes pairwise edges `{a→b | a∈A, b∈B}`; a `System(X).before(Set(A))` becomes `{x→a | a∈A}`; `in_set` membership of nested sets resolves transitively.

3. **Reuse the existing dedup** (`schedule_builder.rs:258-267`) — set-expansion is the canonical source of duplicate edges (many members → many edges), and the `HashSet<(u16,u16)>` dedup already guards `pred_count` inflation.

4. **Cycle detection is already done** — Tarjan (`schedule_builder.rs:374-456`) runs on the post-expansion edge list; a `before`/`after`/membership cycle panics with `boyko-B9001` (`schedule_builder.rs:210-220`). Phase 15 only needs to enrich the diagnostic to distinguish hierarchy cycles from ordering cycles (Bevy's `HierarchySort` vs `DependencySort` distinction, §1.9) and to detect `SetsHaveOrderButIntersect` (two ordered sets sharing a member — a contradiction).

5. **Set-level ordering API** needs a target type that is either a `SystemKey` OR a `SystemSetId`. boyko's `OrderingEdge` already has the `InSet(SystemKey, SystemSetId)` variant; Phase 15 adds variants like `BeforeSet(SystemKey, SystemSetId)`, `SetBeforeSet(SystemSetId, SystemSetId)`, or generalizes the target to an enum `OrderTarget { System(SystemKey), Set(SystemSetId) }`.

This is the minimal, surgical completion of "Wave 5 Step 14" — no executor change, no new data structures in the hot `Schedule`, all cost in `build`.

---

## §5 — Proposed boyko API shape (INPUT TO THE ARCHITECT — NOT FINAL)

boyko's current builder (`schedule_builder.rs:97-111`):
```rust
let key_a = builder.add_system(system_a).key();   // returns SystemConfig, .key() => SystemKey
builder.add_system(system_b).after(key_a);          // order by handle
builder.add_system(system_c).in_set(MySet);         // join set (membership recorded, NOT expanded)
let schedule = builder.build(&mut world);
```

### 5.1 Identification: handle vs label vs SystemSet (the central API decision)

| Strategy | Used by | Pro | Con |
|----------|---------|-----|-----|
| **Opaque handle** (`SystemKey`) | **boyko (current)** | zero-`dyn`, type-safe, no ambiguity, no closure-TypeId problem | must thread the handle around; cross-`add_system` ordering is verbose |
| **System-as-set** (`SystemTypeSet<F>`) | Bevy | ergonomic `before(some_fn)` | ambiguous if `F` registered twice; pulls in `dyn SystemSet` + TypeId machinery |
| **String labels** | specs | simple, decoupled | stringly-typed, runtime panics on dup/missing |
| **Phases/groups only** | flecs, DOTS | brittleness-resistant (module-level) | coarse; no fine-grained control |

boyko's `SystemKey` is the principled choice for the engine core (principle #1: no `dyn`/`HashMap` on identification). The architect should decide whether to ALSO offer a `SystemSet`-targeted ergonomic layer (boyko's `SystemSet` trait is TypeId-keyed already — `system_set.rs:53` — so `before_set::<MySet>()` is cheap: a `TypeId` → `SystemSetId` intern via the existing `set_id_of`, `schedule_builder.rs:117-123`). A bare-system target (`before(system_b_fn)`) is the LEAST advisable for boyko because it reintroduces Bevy's "TypeId of a closure / >1 instance" ambiguity that `SystemKey` was explicitly chosen to avoid (`system_config.rs:11-22` documents this reasoning).

### 5.2 Minimal idiomatic surface (sketch, not final)
```rust
// systems (already exists):
cfg.before(key)            // SystemKey target
cfg.after(key)
cfg.chain(key)             // strict serial, distinct diagnostic
cfg.in_set(MySet)          // join a set (TypeId-keyed)

// NEW — order against a SET (the missing piece):
cfg.before_set::<MySet>()  // this system before all current+future members of MySet
cfg.after_set::<MySet>()

// NEW — configure a SET (Bevy configure_sets analogue):
builder.configure_set::<PhysicsSet>().before_set::<RenderSet>();
builder.configure_set::<MovementSet>().in_set::<PhysicsSet>();   // nesting

// chain a tuple at registration (Bevy .chain() analogue) — optional sugar:
builder.add_systems_chained((sys_a, sys_b, sys_c));   // emits a->b->c edges
```
Notes: keep methods consuming `self` returning `Self` (idiomatic Rust fluent builder; boyko already does — `system_config.rs:65-71`). Set targets resolve at `build` time, so "before all current+future members" is naturally satisfied (members are known by build).

### 5.3 The `#[derive(SystemSet)]` macro
boyko's `SystemSet` is methodless and TypeId-keyed (`system_set.rs:53`). A boyko derive is therefore TRIVIAL compared to Bevy's (which generates `dyn_clone`/`as_dyn_eq`/`dyn_hash`). boyko's derive needs only:
```rust
#[derive(SystemSet)] struct PhysicsSet;            // generates: impl SystemSet for PhysicsSet {}
#[derive(SystemSet)] enum CombatSet { Target, Damage, Cleanup }  // each variant a distinct set
```
i.e. emit `impl SystemSet for T {}` plus (if enum) one `SystemSetId` per variant. NO `dyn` vtable, NO `Hash`/`Eq` requirement on the user type (identity is by `TypeId` + variant discriminant). This is a strict win over Bevy's macro in line with principle #1. The macro mirrors the existing `#[derive(Component)]` pattern in `boyko_macros/src/lib.rs`. (Open question: enum-variant-as-distinct-set needs a `TypeId`+discriminant key, not just `TypeId::of::<CombatSet>()`; the current `set_id_of` keys on `TypeId` alone — `schedule_builder.rs:117` — so enum variants would collapse to one set unless the derive emits a distinct marker type per variant or the key gains a discriminant.)

---

## §6 — Performance + correctness constraints

### 6.1 0%-regression (the hard constraint)
All Phase-15 work is BUILD-time. The hot path (`Schedule::run` → `try_dispatch_ready`) is UNCHANGED: gate 1 reads `pred_remaining[i]` (already exists), gate 2 runs `bitset_intersects` (already exists). Per-frame reset is `clear()` + `[u16]` copy of `pred_count` (`executor_scratch.rs:161-191`). **No new per-frame work.** Confirmation that all leading engines resolve ordering once at build:
- Bevy: `Schedule::run` does NOT call `initialize`; rebuild only after topology change.
- flecs: pipeline is a cached query, rebuilt on dirty.
- DOTS: group re-sorts only when membership changes.
The Phase 9 "50 systems" bench (1.72× vs Bevy) is a `run` bench; since `run` is untouched, the bench is preserved by construction. **Caveat for the architect:** set-expansion can produce O(members²) edges per ordered set pair, inflating `conflict_bits` width and `ConflictGraph::build`'s O(N²/w) scan (`conflict_graph.rs:94`). This is a BUILD cost (one-shot), not a run cost — but for very large schedules the architect should bound it (Bevy's ambiguity check is similarly O(N²) and is opt-in for this reason). The `MAX_SYSTEMS_PER_SCHEDULE = 1024` cap (`schedule_builder.rs:49`) keeps the worst case ~128 KB of bitsets (within L2), so build remains cheap.

### 6.2 Determinism
Explicit ordering makes schedules deterministic where conflict-only might not. boyko's current conflict-only path is ALREADY deterministic (Kahn FIFO insertion-order tie-break — `schedule_builder.rs:461-463`, a documented best practice for stable topo sort), unlike Bevy's default (non-deterministic, "could change every frame"). Phase 15's value is letting users pin INTENTIONAL order so behavior is robust across machines/CPU-counts — the core motivation from Bevy discussion #10205: *"you miss one edge, and then spend hours debugging why it doesn't work as expected on another machine with different number of CPUs."*

### 6.3 Missing-target handling (boyko can beat Bevy)
Bevy silently ignores `before(X)` if `X` is unpopulated (§1.9) — a known footgun. boyko's `SystemKey` is an index into THIS builder's `descriptors` (`ordering.rs:33`), so:
- A key from a DIFFERENT builder, or a stale key past `descriptors.len()`, is DETECTABLE. Today it only trips a debug `debug_assert!` on OOB (`conflict_graph.rs:128-139`) and silently mis-indexes in release. **Recommendation:** Phase 15 should make foreign/OOB keys a BUILD-time error (panic or `Result`), not silent — a strict improvement over Bevy. For set targets, a `before_set::<MySet>()` where `MySet` has zero members should at minimum WARN (matching Bevy's behavior, but loudly).

### 6.4 Cycle detection
Already build-time via Tarjan (`schedule_builder.rs:209-220`), panicking `boyko-B9001` with the cycle's system names. Phase 15 should split diagnostics into hierarchy-cycle vs ordering-cycle (Bevy's `HierarchySort`/`DependencySort`) and add `SetsHaveOrderButIntersect` + `CrossDependency` (ordering a system relative to a set it's in) checks, since set-expansion makes these newly reachable.

---

## Comparative table

| Aspect | Bevy | flecs | EnTT | Unity DOTS | **boyko (current)** |
|--------|------|-------|------|------------|---------------------|
| Per-system before/after | Yes (`before`/`after`, target = SystemSet) | **No** (phases only) | `basic_organizer` resource-derived; no explicit before/after | `[UpdateBefore/After]` (within group) | **Yes** (`before`/`after`, target = SystemKey) |
| Sets / groups / phases | SystemSet (flat graph nodes) | 8 phases via `DependsOn` | none (user-driven) | `ComponentSystemGroup` | `SystemSet` trait (membership recorded, expansion **missing**) |
| Set/phase ordering | `configure_sets` | `DependsOn` between phases | n/a | group hierarchy | **missing** |
| chain() | Yes (tuple → before edges) | n/a (declaration order) | n/a | n/a | `.chain(key)` per-pair (no tuple sugar) |
| Identification of target | SystemSet (incl. `SystemTypeSet<F>`) | phase entity | resource types | system type | **SystemKey handle** |
| Ordering vs conflict | **2 separate** (`num_dependencies_remaining` + `conflicting_systems`) | separate (phase order + intra-system data parallel) | organizer derives from ro/rw resources | **2 separate** (update order + job deps) | **2 separate** (`pred_remaining` + `conflict_bits`) — identical to Bevy |
| `before` w/o data conflict | Serializes the pair (false conflict) | n/a | n/a | n/a (job deps independent of update order) | **Serializes the pair** (false conflict bit, `conflict_graph.rs:146-149`) — identical to Bevy |
| Parallelism model | Task-parallel | **Data-parallel** | user-driven | Task (jobs) + data (entities) | **Task-parallel** |
| Sync / command flush | Auto-inserted `ApplyDeferred`, coalesced | Auto sync points at phase reads | n/a | `EntityCommandBuffer` playback | Per-system apply window (every system flushes; no coalescing) |
| Resolution timing | Build (`initialize` on change) | Build (cached pipeline, dirty rebuild) | n/a (user) | Sort-on-membership-change | **Build** (`ScheduleBuilder::build`, immutable `Schedule`) |
| Cycle detection | Build (`Dag::analyze`) | Build (topo sort) | n/a | Build (group sort) | **Build** (Tarjan SCC, `boyko-B9001`) |
| Graph backend | own `DiGraph` (IndexMap+FixedBitSet, ex-petgraph) | DependsOn relations | adjacency-list `graph()` | reflection + sort | **own** `Vec<(SystemKey,SystemKey)>` + hand-rolled Tarjan/Kahn (no graph dep) |
| Determinism (unordered conflicting pair) | Non-deterministic by default | Deterministic (declaration order) | n/a | n/a | **Deterministic** (Kahn FIFO insertion order) |

---

## Recommendation summary (INPUT TO THE ARCHITECT)

1. **Treat Phase 15 as completing Phase 9's "Wave 5 Step 14", not greenfield.** The edge engine, cycle detection, topo sort, executor, and even the `OrderingEdge`/`SystemConfig`/`SystemSet` types already exist and are tested. The deliverable is: (a) set-membership expansion (`InSet` → pairwise edges = Bevy's "flatten"), (b) set-level ordering API + edges, (c) optional set hierarchy, (d) `#[derive(SystemSet)]`, (e) stricter diagnostics.

2. **Adopt Bevy's separated model — boyko already has it; do NOT touch the executor.** Ordering (`pred_remaining`) and parallelism (`conflict_bits`) are already two independent gates matching Bevy's `num_dependencies_remaining` / `conflicting_systems`. The interaction is solved.

3. **Hook set-expansion into `build` before edge collection** (`schedule_builder.rs:~198`), feeding the same `dag_edges_keys` pipeline. Reuse the existing dedup, Tarjan, Kahn, and `ConflictGraph::build`. All cost stays in `build`; `run` is untouched → 0% regression by construction.

4. **Keep `SystemKey` as the principled core identifier** (zero-`dyn`, no ambiguity) and ADD an optional `SystemSet`-targeted layer (TypeId-cheap via existing `set_id_of`). Avoid Bevy's bare-system-as-target (`SystemTypeSet`) ambiguity. boyko's `#[derive(SystemSet)]` can be far leaner than Bevy's (no `dyn_clone`/`as_dyn_eq`/`dyn_hash`).

5. **Decide the "ordering edge ⇒ conflict bit" question explicitly** (§4.2): default to KEEP (matches Bevy, trivially correct); only DROP for pure ordering edges if a benchmark shows the wider bitsets cost the "50 systems" win. Do not change a tested invariant without a measured gain.

6. **Beat Bevy on diagnostics:** make foreign/stale `SystemKey` and empty-set targets BUILD-time errors/warnings rather than silent no-ops, and split cycle diagnostics into hierarchy vs ordering.

7. **The auto-`ApplyDeferred` pass can stay stubbed for Phase 15.** boyko's per-system apply window already provides Bevy's with-sync `before` semantics. Coalesced sync points are a parallelism optimization (defer to a later phase), not a Phase 15 correctness requirement.

---

## Open questions for the architect

1. **Enum-variant sets:** `set_id_of` keys on `TypeId` alone (`schedule_builder.rs:117`). For `enum CombatSet { Target, Damage }` to yield distinct `SystemSetId`s, the derive must emit a distinct marker per variant OR the key must gain a discriminant. Which?
2. **Set target type:** generalize `OrderingEdge` to `OrderTarget { System(SystemKey), Set(SystemSetId) }`, or add discrete variants (`BeforeSet`, `SetBeforeSet`)?
3. **"future members":** Bevy sets capture members at build (all known by then). boyko's `set_members` is populated during `in_set` calls before `build`, so this is naturally satisfied — confirm no incremental/post-build system addition is planned (SCH1 says no, `schedule.rs` invariants).
4. **`_ignore_deferred` variants:** boyko has no command-flush opt-out (every system flushes in its apply window). Worth adding a `before`/`chain` no-sync variant for parallelism, or out of scope?
5. **Keep the redundant conflict bit for pure ordering edges?** (§4.2) — benchmark-gated.
6. **`configure_sets`-equivalent surface:** a `builder.configure_set::<S>()` returning a set-config handle, or fold into `add_system`'s `SystemConfig`?

---

## Sources

Bevy (source on `main` unless noted):
- [1] Bevy `schedule/config.rs` (IntoScheduleConfigs, before/after/in_set/chain, Dependency/DependencyKind) — https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/schedule/config.rs
- [2] Bevy `schedule/set.rs` (SystemSet trait, SystemTypeSet, IntoSystemSet, derive) — https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/schedule/set.rs
- [3] Bevy `schedule/graph/mod.rs` (GraphInfo, Dependency, DependencyKind, DiGraph) — https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/schedule/graph/mod.rs
- [4] Bevy `schedule/auto_insert_apply_deferred.rs` (sync-point insertion, no_sync_edges, coalescing) — https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/schedule/auto_insert_apply_deferred.rs
- [5] Bevy `schedule/executor/{mod,multi_threaded}.rs` (SystemSchedule, system_dependencies/dependents, can_run, conflicting_systems) — https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/schedule/executor/mod.rs
- [6] Bevy `schedule/schedule.rs` (ScheduleGraph::build_schedule, hierarchy/dependency analyze, flatten) — https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/schedule/schedule.rs
- [7] Bevy 0.10 "stageless" announcement (why stages → flat graph) — https://bevy.org/news/bevy-0-10/
- [8] Bevy RFC #45 Stageless — https://github.com/bevyengine/rfcs/blob/main/rfcs/45-stageless.md
- [9] Bevy Cheatbook — System Order — https://bevy-cheatbook.github.io/programming/system-order.html
- [10] Bevy Cheatbook — System Sets — https://bevy-cheatbook.github.io/programming/system-sets.html
- [11] Bevy `ApplyDeferred` docs (before vs before_ignore_deferred) — https://docs.rs/bevy/latest/bevy/ecs/schedule/struct.ApplyDeferred.html
- [12] Bevy `ScheduleBuildError` (error taxonomy) — https://docs.rs/bevy/latest/bevy/ecs/schedule/enum.ScheduleBuildError.html
- [13] Bevy `ScheduleBuildSettings` (ambiguity_detection/hierarchy_detection/auto_insert_apply_deferred defaults) — https://docs.rs/bevy/latest/bevy/ecs/schedule/struct.ScheduleBuildSettings.html
- [14] Bevy issue #1040 (explicit ordering, why implicit fails) — https://github.com/bevyengine/bevy/issues/1040
- [15] Bevy discussion #2747 (`before`/`after` force serial; `as_if_after` aspirational) — https://github.com/bevyengine/bevy/discussions/2747
- [16] Bevy discussion #10205 (ordering tension, determinism across CPU counts) — https://github.com/bevyengine/bevy/discussions/10205
- [17] Bevy issue #7258 (before/after pitfalls across schedules) — https://github.com/bevyengine/bevy/issues/7258
- [18] Bevy PR #16782 (reuse explicit ApplyDeferred for auto sync) — https://github.com/bevyengine/bevy/pull/16782
- [19] Tainted Coders — Bevy Systems (code examples) — https://taintedcoders.com/bevy/systems

flecs:
- [20] flecs Systems doc (phases, DependsOn topo sort, declaration order, sync points, multithreading) — https://www.flecs.dev/flecs/md_docs_2Systems.html
- [21] flecs DesignWithFlecs (module-level granularity rationale) — https://www.flecs.dev/flecs/md_docs_2DesignWithFlecs.html
- [22] flecs Quickstart (phase assignment code) — https://github.com/SanderMertens/flecs/blob/master/docs/Quickstart.md
- [23] flecs Pipeline addon — https://www.flecs.dev/flecs/group__c__addons__pipeline.html

EnTT:
- [24] EnTT `basic_organizer` (resource-derived task graph, NOT executed) — https://skypjack.github.io/entt/classentt_1_1basic__organizer.html
- [25] EnTT wiki — Entity Component System (no scheduler / main-loop philosophy) — https://github.com/skypjack/entt/wiki/Entity-Component-System

Unity DOTS:
- [26] DOTS Entities 1.0 — Systems update order (UpdateInGroup/Before/After, OrderFirst/Last, group sort) — https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/systems-update-order.html
- [27] DOTS Entities 1.0 — Job system & dependencies (auto read/write tracking) — https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/systems-scheduling-jobs.html

Other Rust ECS (comparative):
- [28] Ratys "What is a scheduler?" (implicit vs explicit order; explicit overrides; escape hatch) — https://ratysz.github.io/article/scheduling-1/
- [29] specs `DispatcherBuilder` (string-label dependencies) — https://docs.rs/specs/latest/specs/struct.DispatcherBuilder.html
- [30] legion README (Schedule auto-parallelize) — https://github.com/amethyst/legion
- [31] shipyard `Workload` (sequential + barriers) — https://docs.rs/shipyard/latest/shipyard/struct.Workload.html

boyko code (branch `ecs`, all paths under `D:\claude\BoykoEngine\`):
- `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` — executor, two-check dispatch (`:444-485`), apply-window successor decrement (`:350-372`).
- `crates/boyko_ecs/src/ecs/core/schedule/conflict_graph.rs` — `pred_count`/`successors`/`conflict_bits` (`:65-78`), ordering-edge ⇒ conflict bit (`:146-149`), build complexity (`:94`).
- `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` — build pipeline (`:151-304`), edge collection (`:200-204`), Tarjan (`:374-456`), Kahn FIFO (`:471-498`), `insert_sync_points` no-op (`:355-360`), set maps (`:64-69`), `set_id_of` (`:117-123`).
- `crates/boyko_ecs/src/ecs/core/schedule/ordering.rs` — `OrderingEdge` (`:54-72`), `SystemKey` (`:33`), `as_dag_edge` (`:83-90`).
- `crates/boyko_ecs/src/ecs/core/schedule/system_config.rs` — `before`/`after`/`chain` (`:65-94`), `in_set` (records-not-expands, `:103-114`), handle-vs-IntoSystem rationale (`:11-22`).
- `crates/boyko_ecs/src/ecs/core/schedule/system_set.rs` — `SystemSet` trait + `SystemSetId` (`:31-53`).
- `crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs` — `ordering_hints`/`sets` (`:39-54`).
- `crates/boyko_ecs/src/ecs/core/schedule/executor_scratch.rs` — `pred_remaining`/`conflict_bits` fields (`:44-111`), `reset_for_frame` (`:161-191`).
- `crates/boyko_ecs/src/ecs/core/system/access.rs` — `conflicts_with` (`:163-171`), `is_universal` (`:124-130`).
- `docs/PHASE-13-ROADMAP.md` — Phase 15 entry (`:79-82`).

A note for the orchestrator: please save the report above to `D:\claude\BoykoEngine\docs\PHASE-15-RESEARCH.md` — I could not write it myself (no file-write tool in this context). The most decision-critical sections for the Phase 15 architect are **§0** (the dormant scaffold inventory) and **§4** (the conflict-vs-explicit interaction — already correctly implemented in boyko, matching Bevy).
