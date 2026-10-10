# Research: order of two systems whose access conflicts but whose order is not declared ("ambiguities"). Bevy, flecs, Unity DOTS, EnTT, and the boyko executor

Provenance: a researcher agent (web + tree), 2026-09-19; the tree facts were read in the light-table lane (`D:/wt/lighttable` @ `1c31aeac`).

## TL;DR
- **Our executor runs in lock-step waves.** Each round starts with nothing running. It walks the systems in topological index order and dispatches every ready system that does not conflict with one already picked. It applies results only after **every** dispatched system has reported back (`schedule.rs:748`, `:1258-1311`). No new system is dispatched before that drain. So which systems share a wave, and which of two conflicting unordered systems goes first, is a fixed function of five things: the topological order, the conflict bits, the ordering edges, the exclusive flags and the run-condition results. **It does not depend on worker count or timing.** That fits F1 reproducing 3 out of 3 times.
- **The tie-break key is the global registration index, through a FIFO Kahn sort** (`schedule_builder.rs:1048-1075`). For two conflicting unordered systems: if both are ready in the same wave, the lower topological index runs first. Otherwise the one that became ready in an earlier wave runs first.
- **What the `seed.before(cull)` edge changes:**
  - `cull` goes from having no predecessors to having one, so it leaves the start of the queue and is re-queued at the tail when `seed` pops.
  - `cull` can no longer run until `seed`, which is exclusive, has run in its own solo wave.
  - The waves are packed again. `cull` declares `ResMut<LightingConfig>` and `ResMut<LightStats>` even in Manual mode (`light_policy.rs:202-207`). That write access stops blocking readers of `LightingConfig` in its old wave and starts blocking them in its new one.
  - Any conflicting unordered pair elsewhere whose wave numbers shift can flip order, even though no data flows through the new edge.
- **Timing-dependent parts that do exist:**
  - The `apply()` order inside a wave follows completion order through an MPSC `ArrayQueue` (`schedule.rs:860-905`, `:1712-1731`).
  - Event lanes are flattened in lane-index order, and a lane is chosen by which worker ran the sender (`access.rs:113-123`, `event_dispatcher.rs:630-637`).
  - The `may_defer` field doc says a planned KE17 change would relax the full barrier (`schedule.rs:141-169`).
- **The other engines:**
  - Bevy's multi-threaded executor order depends on timing. Its ambiguity detection is a static build-time check that is off by default. Its CI requires zero ambiguities across DefaultPlugins. The 0.20 prerelease adds `shuffle_seed` (behind the `debug` feature), which randomises the topological order.
  - flecs does not run systems concurrently with each other in its built-in pipeline. Its order is phase depth, then entity id.
  - Unity sorts the systems in a group by a hash of the type name and derives job dependencies from that update order.
  - EnTT's organizer turns every access conflict into an ordering edge in registration order.
  - In flecs, Unity and EnTT an unordered conflicting pair either cannot exist or is ordered by construction.

## Approaches in current engines

### Bevy (main branch, 0.20 prerelease, fetched 2026-09-19)
- **Executor.** `MultiThreadedExecutor::tick` first drains `system_completion` (a `ConcurrentQueue`) through `finish_system_and_handle_dependents` → `signal_dependents`, then calls `spawn_system_tasks`. That function iterates `ready_systems.ones()` in ascending index order. `can_run` rejects a system if `conflicting_systems` overlaps `running_systems`, or if it is exclusive while anything is running. [1]
  - Completions free up conflicts and dependents as soon as they arrive, so the order of an ambiguous pair depends on timing. Bevy's own doc on `shuffle_seed` says: "The MultiThreadedExecutor allows systems to run out-of-order if the 'next' system has a conflict with a currently-running system … can also produce orderings that are not possible in single-threaded execution." [2]
  - The example file states: "Unless the order is explicitly specified, their relative order is nondeterministic." [3]
  - The cheat book (0.13, outdated) says: "the order could even change every frame." [4]
- **Deferred apply order.** `apply_deferred` walks `unapplied_systems.ones()`, which is ascending topological index, not completion order. [1] Issue #10122 (0.11.2) shows the consequence: two ambiguous systems' command buffers were applied in the opposite order to the one they ran in, in about 10% of runs. [5]
- **SingleThreadedExecutor** runs `for system_index in 0..n` in topological order. [6]
- **Topological sort.** `DiGraph::toposort` reverses the output of Tarjan's SCC algorithm. Nodes are stored in an `IndexMap`, which keeps insertion order. [7]
- **Ambiguity detection.** Configured through `ScheduleBuildSettings { ambiguity_detection: LogLevel, hierarchy_detection, auto_insert_apply_deferred, use_shortnames, report_sets, #[cfg(feature="debug")] shuffle_seed }`. The default is `ambiguity_detection: LogLevel::Ignore`, and `LogLevel` is `Ignore | Warn | Error`. [2]
  - Algorithm (`get_conflicting_systems`): take every pair in `flat_dependency_analysis.disconnected()`, i.e. no path between them in the transitive closure. Skip a pair if it is covered by `ambiguous_with` or if either system is in `ambiguous_with_all`. A pair where either side is exclusive is a conflict. A pair that `is_compatible` is not. Otherwise call `get_conflicts` and filter the result against `ignored_scheduling_ambiguities`, which `allow_ambiguous_component` / `allow_ambiguous_resource` fill. [2][8]
  - The result is `ConflictingSystems(Vec<(SystemKey, SystemKey, Box<[ComponentId]>)>)`, reported as `ScheduleBuildWarning::Ambiguity` (a warning or an error). [2][8]
  - `ambiguous_with(set)` and `ambiguous_with_all()` are on `IntoScheduleConfigs`. [9]
- **`shuffle_seed`.** Shuffles the node list with `Xoshiro128PlusPlus::seed_from_u64` and rebuilds the DAG with the same edges. Its purpose, quoted: "if you spot erroneous behavior when shuffling, that is an indication that the 'default' ordering is correct by chance." [2] An open issue from 2026-09-16 asks for docs comparing it with ambiguity detection. [10]
- **History.**
  - Discussion #1312 (Jan 2021) chose "warn, don't forbid". [11]
  - Discussion #2480 (2021) listed the sources of nondeterminism: floating point, system order ("need not be fully static, but must be specifiable"), entity iteration order, PRNG, and "Commands and EventReader evaluation order". [12]
  - PR #13950 (Jul 2024) added a CI ambiguity test. [13]
  - PR #15031 (Sep 2024) removed every ambiguity in DefaultPlugins. [14] `tests/ecs/ambiguity_detection.rs` now asserts a total of 0 for both the main app and the render app. [15]
- **Trade-offs.** Ambiguous pairs are allowed for throughput. Detection is opt-in and static. The checker produces false positives: #11796 reports that it ignores `With`/`Without` filters, and it was closed as not planned. [16]

### flecs (master)
- **Order.** The built-in pipeline query is `System, Phase(cascade(DependsOn)), …` with `order_by_callback = flecs_entity_compare`. The docs say: "A pipeline by default orders systems by their entity id, to ensure deterministic order … generally … the order they are declared." [17][18]
- **Threading.** "The scheduler runs each multithreaded system on all threads, and divides the number of matched entities across the threads … the same entity is always processed by the same thread, until the next sync point." [17] The docs describe no case of two systems running concurrently in the built-in pipeline, so an ambiguous pair does not arise.
- **Sync points.** Inserted automatically where "a read for a component for which commands could have been inserted" occurs (`flecs_pipeline_check_term`). [17][18] `flecs_stage_merge` merges stages in index order: `for (i = 0; i < count; i++)`. [19]

### Unity Entities (1.x)
- **Order.** Within a group: `OrderFirst`/`OrderLast` first, then `UpdateBefore`/`UpdateAfter`. [20] `ComponentSystemSorter` runs Kahn's algorithm with a **min-heap keyed on `TypeManager.GetSystemTypeHash`**, with ties broken by `unsortedIndex`. [21] That hash is `BurstRuntime.GetHashCode64(systemType)` [22], documented as a 64-bit hash of the type's `AssemblyQualifiedName`. [23] So the order among unconstrained systems does not depend on registration order.
- **Jobs.** "If a system that updates earlier in the frame reads data that a later system writes, or writes data that a later system reads, then the second system depends on the first." [24] Every conflicting pair is therefore chained in main-thread update order. Worker count does not affect that order.

### EnTT (organizer / flow)
- Constness of a parameter decides read-only vs read-write access. "All functions are added in order of execution to the organizer." Tasks are never executed: "The actual scheduling of the tasks is the responsibility of the user." [25]
- In `flow`, "the order of registration on the resources also determines the order in which the tasks are processed", and `sync()` creates a sync-point vertex. [26]
- Every conflict becomes an edge in registration order, so ambiguous pairs cannot exist.

## Comparison table

| Aspect | Bevy (MT) | flecs | Unity | EnTT | boyko |
|---|---|---|---|---|---|
| Conflicting pair with no declared order | allowed; order depends on timing | cannot occur (systems run one at a time) | ordered by update order | edge added in registration order | allowed; ordered by wave, then topological index |
| Tie-break in the sort | reversed Tarjan over IndexMap insertion order | phase depth, then entity id | min-heap on type-name hash, then index | registration order | FIFO Kahn by global registration key |
| Depends on worker count or timing | yes | no (fixed entity-to-thread split) | no | left to the user | **no** for wave order; **yes** for apply order within a wave and event lane order |
| Deferred apply order | ascending index | stage index | n/a | n/a | completion order (MPSC FIFO) |
| Ambiguity report | `LogLevel`, default Ignore; CI requires 0 | n/a | n/a | n/a | **none** |
| Randomised-order testing | `shuffle_seed` (debug feature) | no | no | no | none |

## Our executor: how two conflicting unordered systems X and Y are ordered (lane `D:/wt/lighttable`)
1. **Build.**
   - Edges are collected in descriptor order, with set edges appended after (`schedule_builder.rs:518-525`).
   - Cycles are checked with Tarjan (`:552-567`).
   - The topological sort is `kahn_topological_sort` with a **FIFO `VecDeque`**, seeded with the in-degree-0 nodes in `SystemKey` order, i.e. global registration order across all plugins. A child is pushed when its **last** predecessor pops (`:1048-1075`).
   - Descriptors are then permuted into that order (`:575-625`).
   - `ConflictGraph::build` marks conflicting pairs from `Access::conflicts_with` (`conflict_graph.rs:108-118`, `access.rs:214-222`), and every ordering edge also sets a conflict bit and raises `pred_count` (`conflict_graph.rs:125-150`).
   - `insert_sync_points` does nothing (`schedule_builder.rs:922-927`).
2. **Run.** Each loop iteration does the following:
   - It applies only when `pending > 0 && (pending == running || running == 0)` (`schedule.rs:748`). This drain clears every `running` bit.
   - Run conditions are evaluated only when `running == 0` (`:783`).
   - `try_dispatch_ready` scans `i in 0..n` and dispatches every ready system that does not conflict with `running`, setting its `running` bit straight away (`:1258-1311`). This is greedy first-fit in topological index order.
   - An exclusive or GPU system is dispatched only if `running` is empty at the moment the scan reaches it. When one is dispatched, the scan stops (`:1296-1304`).
   - Between drains `running` only grows and `pred_remaining` only falls during a drain, so later rounds dispatch nothing new until the drain. Each wave is a bulk-synchronous step.
3. **Order of X and Y.** If both are ready in the same wave, the lower topological index runs first and the other is pushed to a later wave. If not, the one whose predecessors finished in an earlier wave runs first. Worker count and timing play no part. An exclusive system is delayed until no lower-index system is ready. Bevy's executor has a similar note: exclusive systems "might be significantly displaced". [1]
4. **What an added, apparently unrelated edge A→B can change:**
   - (a) B's position in the Kahn sort: it now enters the FIFO tail when A pops. Everything between B's old and new positions moves down one index but keeps its relative order. The exception is when B's pop is what releases a successor, in which case that successor's subtree also moves.
   - (b) B's wave, which can now be no earlier than A's wave plus one.
   - (c) The packing of every wave that B leaves or joins, through B's **declared** access. For `cull` that is `ResMut<LightingConfig>`, `ResMut<LightStats>` and reads of `LightEnabled`, `PointLight` and `SpotLight`.
   - (d) The waves in which exclusive systems get dispatched.
   - (e) Which systems' run conditions are evaluated at which point.
   - (f) The apply-order groupings.
   Any ambiguous pair whose wave numbers move can flip. The test comment at `tests/phase15_set_ordering.rs:14-25` already relies on this structure: it uses one worker and a distinct resource per system.
5. **Timing-dependent parts** that bear on the rule that replays play on any machine:
   - The apply order within a wave is completion order (`executor_scratch.rs:17-18, 71-77`; `schedule.rs:1712-1731`), unlike Bevy's index order.
   - Events are not part of `Access`. Each worker writes to its own lane, chosen through thread-local storage, and the dispatcher writes to lane `worker_count` (`access.rs:113-123`). The per-frame flatten walks lanes in index order (`event_dispatcher.rs:630-637`). So event order depends on which worker ran the sender and on how many workers exist.
   - The `may_defer` field doc says the planned KE17 "split retire" will relax the full barrier (`schedule.rs:141-169`). Once that lands, the step-by-step wave property above would no longer hold.
   - There is no ambiguity detection or order dump. A grep for `ambigu` in `crates/boyko_ecs` finds nothing about scheduling.

## Key techniques
- **Ambiguity detection** (Bevy): take the pairs with no path in the transitive closure, test each for an access conflict, subtract an allow-list (per system/set, per component/resource), and report at a configurable level.
- **Tie-break keys:**
  - FIFO by registration index (ours). Changes whenever plugin add order changes or a system is inserted.
  - Reversed Tarjan over insertion order (Bevy).
  - Min-heap on a stable name hash (Unity). Unaffected by registration order, but an edge can still delay when nodes become ready.
  - Entity id, i.e. declaration order (flecs).
  - Kahn with a priority queue gives the lexicographically smallest topological order; this is the core of Coffman–Graham. [27][28]
- **Conflict as an edge in declared order** (Unity, EnTT): removes ambiguity by construction.
- **Randomised topological order** (Bevy `shuffle_seed`): finds constraints that are only correct by chance.
- **Dynamic topological order** (Pearce–Kelly, 2007): maintains an order under edge insertion. It is used in Abseil and TensorFlow. [29][30] I could not extract its locality guarantee verbatim from the PDF.

## Pitfalls
- "The order is stable for fixed input" (the Kahn doc, `schedule_builder.rs:1037-1040`) is true, but any edit to the input changes the order: an edge, a newly registered system, or plugin order.
- An ambiguity checker built only on unfiltered access gives false positives (Bevy #11796). Without allow-lists it becomes noise, which is why Bevy added `ambiguous_with` and `allow_ambiguous_*`.
- Apply order can differ from run order (Bevy #10122).
- Declared access matters even when the code path does not write. `cull` takes `ResMut<LightingConfig>` but never writes it in Manual mode, and the scheduler still treats it as a writer.
- A checker based only on `Access` cannot see event-carried dependencies, because events are outside `Access` (`access.rs:113-123`).

## Academic and design sources
- Pearce & Kelly, "A Dynamic Topological Sort Algorithm for Directed Acyclic Graphs", ACM JEA 2007. [29][30]
- The Coffman–Graham scheduling algorithm (Kahn with lexicographic tie-breaks). [27]
- Bevy discussion #2480, a list of determinism sources for lockstep and replay. [12]

## Applicability to boyko-engine (facts only, no design)
- **Can be taken directly:** Bevy's disconnected-pair × `conflicts_with` enumeration. Our `ConflictGraph` already holds the pairwise conflict bits and the DAG, but not a transitive closure.
- **Needs adaptation:**
  - A checker would not see events, because they are outside `Access`.
  - The executor's wave determinism does not carry over to the within-wave apply order or to event lane order.
  - KE17 would change the barrier.
- **Contrast:** Bevy's order depends on timing and ours does not, so Bevy's argument that "ambiguities cost only nondeterminism" does not transfer as-is. Here an ambiguity is a deterministic outcome that changes when the build is edited.

## Open questions for the architect
- Which pair or pairs in `CoreSchedule::Main` flipped for the six TAA pins? That needs a dump of the waves and topological order with and without the edge.
- Should a conflicting unordered pair be a build error, a warning, or ordered implicitly (the Unity/EnTT approach)? And what tie-break key: registration index, or a stable name or hash?
- Does the replay contract require identical apply order within a wave and identical event order across worker counts?
- Should a detector include events?
- How does KE17's split retire interact with determinism?

## Sources
- [1] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/schedule/executor/multi_threaded.rs: `tick`, `spawn_system_tasks`, `can_run`, `signal_dependents`, `apply_deferred`
- [2] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/schedule/schedule.rs: `ScheduleBuildSettings`, `LogLevel`, `shuffle_seed`, the ambiguity build step
- [3] https://raw.githubusercontent.com/bevyengine/bevy/main/examples/ecs/nondeterministic_system_order.rs
- [4] https://bevy-cheatbook.github.io/programming/system-order.html (0.13, outdated)
- [5] https://github.com/bevyengine/bevy/issues/10122
- [6] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/schedule/executor/single_threaded.rs
- [7] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/schedule/graph/graph_map.rs: `toposort`
- [8] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/schedule/node.rs: `get_conflicting_systems`, `ConflictingSystems`
- [9] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/schedule/config.rs: `ambiguous_with`, `ambiguous_with_all`
- [10] https://github.com/bevyengine/bevy/issues/25812 and https://github.com/bevyengine/bevy-website/issues/2579
- [11] https://github.com/bevyengine/bevy/discussions/1312
- [12] https://github.com/bevyengine/bevy/discussions/2480
- [13] https://github.com/bevyengine/bevy/pull/13950
- [14] https://github.com/bevyengine/bevy/pull/15031
- [15] https://raw.githubusercontent.com/bevyengine/bevy/main/tests/ecs/ambiguity_detection.rs
- [16] https://github.com/bevyengine/bevy/issues/11796
- [17] https://raw.githubusercontent.com/SanderMertens/flecs/master/docs/Systems.md
- [18] https://raw.githubusercontent.com/SanderMertens/flecs/master/src/addons/pipeline/pipeline.c
- [19] https://raw.githubusercontent.com/SanderMertens/flecs/master/src/stage.c: `flecs_stage_merge`
- [20] https://docs.unity3d.com/Packages/com.unity.entities@1.3/manual/systems-update-order.html
- [21] https://raw.githubusercontent.com/needle-mirror/com.unity.entities/master/Unity.Entities/ComponentSystemSorter.cs
- [22] https://raw.githubusercontent.com/needle-mirror/com.unity.entities/master/Unity.Entities/Types/TypeManagerSystems.cs
- [23] https://docs.unity3d.com/Packages/com.unity.burst@1.6/api/Unity.Burst.BurstRuntime.GetHashCode64.html
- [24] https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/scheduling-jobs-dependencies.html
- [25] https://raw.githubusercontent.com/skypjack/entt/master/docs/md/entity.md: Organizer section
- [26] https://raw.githubusercontent.com/skypjack/entt/master/docs/md/graph.md: flow builder
- [27] https://en.wikipedia.org/wiki/Topological_sorting (overview only)
- [28] https://www.geeksforgeeks.org/dsa/lexicographically-smallest-topological-ordering/ (secondary)
- [29] https://whileydave.com/publications/pk07_jea/
- [30] https://www.doc.ic.ac.uk/~phjk/Publications/DynamicTopoSortAlg-JEA-07.pdf

**Tree files read (lane `D:/wt/lighttable`):**
- `crates/boyko_ecs/src/ecs/core/schedule/{schedule.rs, schedule_builder.rs, conflict_graph.rs, ordering.rs, executor_scratch.rs, mod.rs}`
- `crates/boyko_ecs/src/ecs/core/system/access.rs`
- `crates/boyko_ecs/src/ecs/core/events/{event_buffer.rs, event_dispatcher.rs}`
- `crates/boyko_ecs/tests/phase15_set_ordering.rs`
- `crates/boyko_render/src/{light_plugin.rs, light_policy.rs}`