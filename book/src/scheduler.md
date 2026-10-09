# Parallel Scheduler

The scheduler is the engine's top-level system runner. It takes a set of
registered systems plus their declared `Access` surfaces and a `ThreadPool`,
and runs them concurrently when their accesses don't conflict — fanning
work onto worker threads, auto-inserting synchronization points where
`Commands` deferral demands them, and delivering a system's panic to the
caller exactly once.

This page covers the user-facing scheduler API. The internal contracts
(apply window, conflict graph, incremental ready-set) are summarized where
they explain behaviour you can observe; most users do not need more.

## Why a parallel scheduler

Single-threaded ECS loops cap at the rate one CPU core can stream
component bytes — for an engine targeting AAA-scale entity counts (1M+
entities, 60 Hz tick), that's not enough. The scheduler provides:

1. **Multi-system concurrency** — independent systems run in parallel
   when their declared `Access` doesn't overlap. A `fn(Query<&Position>)`
   and a `fn(Query<&Velocity, Without<Frozen>>)` run on different cores.
2. **Intra-system parallelism** — a single system can fan its query rows
   onto multiple workers via `query.par_iter()`. Large archetypes split
   into chunks; workers pick them up via work-stealing.
3. **Deterministic `Commands` flush** — commands enqueued during the
   parallel phase apply in a serial _apply window_, eliminating the
   race-on-write that a naive parallel apply would create.
4. **Context discipline** — system bodies run under a thread-local
   "in-system-run" flag. Context-restricted paths (event send/read, `Time`
   access, the hook-drain) `debug_assert!` against it. Storage growth never
   happens inside a system body: a `ComponentPool` only grows on a `&mut`
   path (the owner's direct API or the per-frame apply window), where the
   dispatcher holds `&mut EcsMaster` exclusively.

The scheduler does not introduce `Mutex` or `RwLock` on the hot path.
Cross-worker synchronization is one `AtomicUsize` per frame
(`pending_apply`) plus a lock-free MPSC `ArrayQueue` for completions,
both living inside an out-of-line `CompletionChannel` the workers reach
through a `NonNull` rather than through the dispatcher's `&mut self`.

## High-level overview

```text
+---------------------+      .add_system(...)
|   ScheduleBuilder   |  ──────────────────────► registers systems +
|  pool: Arc<Pool>    |                           ordering hints
+----------+----------+
           |  .build(world)
           |    - cycle detection
           |    - topological sort
           |    - conflict graph build
           v
+---------------------+      .run(&mut world)
|      Schedule       |  ──────────────────────► one frame:
|  pool / systems     |                           1. dispatch ready
|  conflict_graph     |                           2. workers run bodies
|  exec_scratch       |                           3. apply window
+---------------------+                           4. repeat until done
```

Two types form the public surface:

- [`ScheduleBuilder`](#schedulebuilder) — fluent registration of systems.
- [`Schedule`](#schedule) — the runnable artefact produced by
  `ScheduleBuilder::build`. Mutable; its internal scratch state advances
  per frame.

A third type — [`SystemConfig`](#systemconfig-fluent-api) — is the value returned
from `add_system(...)`. It carries the `.before`, `.after`, `.chain`,
`.in_set`, `.before_set`, `.after_set`, `.run_if`, and `.gpu` fluent
hints, plus `.key()` to capture the system's `SystemKey` for use in a
sibling ordering call.

## `ScheduleBuilder`

```rust,ignore
use std::sync::Arc;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_threadpool::ThreadPoolBuilder;

// Build a thread pool. Worker count defaults to
// std::thread::available_parallelism() if not specified. Reuse the pool
// across schedules — building one is expensive.
let pool: Arc<_> = ThreadPoolBuilder::new().num_threads(8).build();

let mut world = EcsMaster::new();
let mut builder = ScheduleBuilder::new(Arc::clone(&pool));

// Register systems. Each call returns a SystemConfig handle for fluent
// ordering / set membership. Ordering edges reference another system by
// its `SystemKey`, captured from the handle via `.key()`.
let physics = builder.add_system(physics_step).key();
builder.add_system(render_prepare).after(physics);

let mut schedule = builder.build(&mut world);

// One frame.
schedule.run(&mut world);
```

### Output bound

Only systems with `Out = ()` flow through the scheduler. The compile-time
bound on `add_system` is `F: IntoSystem<(), (), M>`. Systems with a
non-unit return type use `EcsMaster::run_system` directly, outside the
schedule.

## `Schedule`

```rust,ignore
impl Schedule {
    pub fn run(&mut self, world: &mut EcsMaster);
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```

`Schedule::run` advances the executor one frame. It:

1. Resets the per-frame scratch state.
2. Enters `pool.install(|scope| ...)` — the scope's lifetime gates the
   safety of cross-thread borrows.
3. Dispatches every ready system; workers pick them up via work-stealing.
4. Drains completions in an apply window when all dispatched systems have
   reported back.
5. Repeats until every system has run and applied.

The function returns only after every system has both **run** (its body
executed) and **applied** (its `Commands` queue, if any, has flushed
against `world`).

A `Schedule` is bound to the world it was built on. Calling `run` with a
different `EcsMaster` panics with `boyko-B9101` (one compare per run,
checked in release builds too), because the schedule caches per-world
pointers. Build one schedule per world.

Every system also gets a profiling zone at build time, so a profiler
capture shows one span per system run. `Schedule::system_zones()` lists
each system's name and zone id, in topological order, for joining profiler
rows back to systems; it yields nothing in builds that compile system zones
out.

## Exclusive systems

A system whose declared `Access` is universal — equivalent to "this
system needs `&mut EcsMaster`" — is called an **exclusive system**. The
scheduler recognises these via `Access::is_universal()` and runs them
inline on the dispatcher thread, gated on `running == 0` (every concurrent
system has completed and applied).

The canonical exclusive-system signature:

```rust,ignore
fn save_world(world: &mut EcsMaster) {
    // Full read/write access to the entire world.
}
```

`IntoSystem` has a blanket impl for `FnMut(&mut EcsMaster) -> ()` via the
`ExclusiveSystemMarker`. The blanket coexists with the
`SystemParamFunction` blanket for the same name without a coherence
conflict.

Two other kinds of system also run on the dispatcher thread with nothing
else in flight:

- a system that takes `NonSendRes<R>` / `NonSendResMut<R>` (its data cannot
  cross threads);
- a **GPU-compute system**, marked with `.gpu()` on its `SystemConfig`. It runs
  at the apply-window barrier, the sound place to record and submit through
  the single-threaded RHI. For every producer → consumer edge of the conflict
  graph whose consumer is a GPU-compute system,
  `Schedule::gpu_barrier_inputs()` yields a `GpuBarrierEdge` that
  `boyko_render` lowers into Vulkan buffer barriers.

## `Commands` and the apply window

A system that enqueues structural mutations through `Commands`:

```rust,ignore
use boyko_ecs::ecs::core::system::Commands;

fn spawn_enemies(mut commands: Commands) {
    commands.spawn(EnemyBundle { hp: 100, pos: Position { x: 0.0, y: 0.0, z: 0.0 } });
}
```

…enqueues a spawn command into the system's per-system `CommandQueue`.
The queue is then flushed against `world` during the **apply window** of
the dispatch round — the serial phase between waves where the dispatcher
holds `&mut EcsMaster` exclusively.

The apply window contract:

- The dispatcher does not reborrow `&mut EcsMaster` while any worker
  holds a cell copy. The gate `pending_apply == running.count_ones()`
  proves every dispatched system has reported back.
- Commands flush in deterministic order — the order systems completed
  within the apply window — which matches the topological order modulo
  parallel-completion timing.

**`Commands::send_event::<E>(event)`** also lives here — events emitted via
the user-facing `send_event` wrapper enqueue into the dispatcher's lane
(`worker_count` slot of the event dispatcher) during the apply window.
Direct emission from a worker body uses the worker's own lane via TLS
(`current_worker_id`) — see _Event lanes_ below.

## `Query::par_iter`

A single system can split its query into parallel chunks via `par_iter`:

```rust,ignore
use boyko_ecs::ecs::core::iters::query::Query;

fn integrate_velocities(query: Query<(&mut Position, &Velocity)>) {
    // The Fn body runs concurrently on disjoint row ranges. The Send + Sync
    // bound forbids capturing &mut state.
    query.par_iter_mut().for_each(|(pos, vel)| {
        pos.x += vel.vx;
        pos.y += vel.vy;
        pos.z += vel.vz;
    });
}
```

Bounds and behaviour:

- **`Fn`, not `FnMut`** — the closure cannot mutate captures. Per-row
  mutation flows through `D::Item<'_>` (e.g. `&mut Position`).
- **`Send + Sync`** — workers cross thread boundaries; the closure body
  is shared across them. This compile-fails any `&mut Commands` capture
  (the failing fixture is
  `crates/boyko_ecs/tests/par_iter_compile_fail/capture_commands.rs`, run
  by the trybuild harness `tests/par_iter_captures_commands_fails.rs`).
- **Inline threshold** — archetypes with fewer than
  `MIN_ARCHETYPE_FOR_PARALLEL` (= 1024) rows run inline on the calling
  thread. The fork-join overhead would otherwise dominate.
- **Nested scopes** — `par_iter` calls `pool.scope`, which is re-entrant.
  Calling `par_iter` from inside a system body that is itself running on
  a worker works without deadlock (the rayon work-stealing pattern in
  `Scope::Drop`), and its chunks really fan out: every spawned task, a
  worker's own included, lands where other workers can steal it.

`par_iter` is read-only (`D: ReadOnlyQueryData`); `par_iter_mut` accepts
any `D: QueryData`.

## `SystemSet` labels

Systems can be grouped under a `SystemSet` for ordering hints that span
multiple systems:

```rust,ignore
use boyko_macros::SystemSet;

#[derive(SystemSet)]
struct PhysicsSet;

#[derive(SystemSet)]
struct RenderSet;

let mut builder = ScheduleBuilder::new(pool);
builder.add_system(integrate).in_set(PhysicsSet);
builder.add_system(collide).in_set(PhysicsSet);
// Order this system relative to a *set* with `.after_set` / `.before_set`
// (a set-relative hint expands to per-member edges at build time).
builder.add_system(render).in_set(RenderSet).after_set(PhysicsSet);
```

Ordering relative to a set uses `.before_set(set)` / `.after_set(set)` —
**not** `.before` / `.after`, which take a `SystemKey` for ordering against
a single system. `.in_set(set)` records membership only (no edge on its
own). The full ordering vocabulary is:

- `.before(key)` / `.after(key)` — order against a single system by `SystemKey`.
- `.chain(key)` — strict serial order (this → `key`), a distinct edge variant for diagnostics.
- `.in_set(set)` — set membership.
- `.before_set(set)` / `.after_set(set)` — order against every (transitive) member of a set.
- `.run_if(cond)` — attach a run condition.
- `.gpu()` — mark a GPU-compute system (runs dispatcher-solo at the apply-window barrier).

These compose; conflicts between hints panic at `build` time with a cycle
diagnostic.

## `SystemConfig` fluent API

`SystemConfig` (returned by `add_system`) carries:

- **`.key()`** — returns this system's `SystemKey` so a sibling call can
  order against it.
- **`.before(other: SystemKey)`** — this system runs before `other`.
- **`.after(other: SystemKey)`** — this system runs after `other`.
- **`.chain(other: SystemKey)`** — strict serial order (this → `other`).
  Same DAG edge as `before` but a distinct variant for diagnostics. There
  is no no-arg `.chain()` — pass the target key explicitly.
- **`.in_set(set)`** — adds this system to `set` (a `SystemSet` value).
- **`.before_set(set)` / `.after_set(set)`** — order against a set's members.
- **`.run_if(cond)`** — attach a run condition (see
  [Run Conditions](scheduling/run-conditions.md)).
- **`.gpu()`** — mark a GPU-compute system (see
  [Exclusive systems](#exclusive-systems)).

```rust,ignore
let physics = builder
    .add_system(physics_step)
    .in_set(PhysicsSet)
    .key();

builder
    .add_system(input_handler)
    .before(physics);
```

## Context discipline

There is no shared arena allocator to protect — the engine retired the
shared `Arena`. Component storage lives in per-pool
virtual-memory reservations: each `ComponentPool` reserves a fixed,
address-stable row ceiling up front (`ComponentPool::new(component_id,
reserve_rows)` on a `VmReservation`) and commits frontier pages lazily as
rows are added. There is no global `Arena::allocate_*` call on the hot
path.

Growth (`ComponentPool::grow_rows`) is plain `&mut self` field mutation —
it commits more pages on the pool's **own** reservation and never moves
the base pointer. Because it is reachable only through `&mut` paths (the
owner's direct API, or the apply window where the dispatcher holds
`&mut EcsMaster` with zero workers in flight), the `&mut`
exclusivity **is** the guard. The commit syscalls are not global-allocator
calls, so they need no separate allocation flag
([`component_pool.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/memory/component_pool.rs)).

What survives from the old discipline is a thread-local **context flag**.
The dispatcher wraps every system body in
`boyko_threadpool::InSystemRunGuard::enter()`
([`executor_scratch.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/schedule/executor_scratch.rs)),
and context-restricted paths `debug_assert!(boyko_threadpool::is_in_system_run())`
(or its negation) to catch misuse:

- `EventReader` / `EventWriter` — must be used from inside a system body
  (the TLS `current_worker_id` router places the write on the correct
  event lane).
- `Time` access — `debug_assert!`s it is **not** called inside a system
  body (advance happens on the dispatcher).
- The deferred hook-drain — asserts the dispatcher context (not mid-body).

The structural mutation paths stay deferred regardless:

- Archetype / pool growth — happens on `&mut` paths only (apply window or
  the owner's direct API), never from a worker mid-body.
- `Commands` — a system enqueues into its own per-system `CommandQueue`
  (allocated before the body runs) and the queue flushes during the apply
  window.

## Event lanes

The event dispatcher reserves one lane per worker plus one lane for the
dispatcher. Worker bodies emit events to their own lane; the dispatcher
emits during the apply window. The lane count is the `thread_count`
passed to `EventConfig::default_for(thread_count: u32) -> EcsResult<Self>`
([`event_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/events/event_config.rs)) —
sized to cover every worker plus the dispatcher lane.

User code calls `EventDispatcher::send_event::<E>(event) -> EcsResult<()>`
(or `Commands::send_event::<E>(event)` for deferred-from-system emission);
the TLS `current_worker_id` router places the write on the correct lane.
Both `send_event` and `EventConfig::default_for` return an `EcsResult`,
so a caller may need to handle the `Err` (e.g. a full lane or an
out-of-range config).

## Threading model

- The **dispatcher** thread is the thread that calls `Schedule::run`.
  It owns `&mut EcsMaster` for the duration of the call and re-borrows
  `world_mut` during the apply window.
- **Workers** are the OS threads owned by `ThreadPool` (by default one per
  `std::thread::available_parallelism()`). Each one owns a Chase-Lev deque
  whose `Stealer` is published in a global registry, plus TLS state
  (`current_worker_id`, `is_in_system_run`). Every spawn — a worker's own
  included — lands either on a registered deque or in the pool's global
  `Injector`, so no task is reachable by one thread alone and an idle
  worker can always steal it.
- `Schedule::run` enters `pool.install(|scope| ...)` once per frame. The
  install sets `ACTIVE_POOL` TLS for the calling thread so that ambient
  `par_iter` calls inside system bodies can discover the pool without an
  explicit argument.

`EcsMaster: Send + Sync` and `UnsafeEcsCell<'w>: Send + Sync` are the
two unsafe Send/Sync impls that enable workers to receive cell copies.
The aliasing discipline is enforced upstream:

- **Conflict graph** — at run time, no two concurrent systems hold
  overlapping `&/&mut` views through their cell copies. The graph is
  built at `ScheduleBuilder::build` from declared `Access` surfaces.
- **Apply-window barrier** — the dispatcher reborrows `&mut EcsMaster`
  only when all dispatched systems have reported completion (the gate
  `pending_apply == running.count_ones()`).

## Panic handling

When a system body panics, `Schedule::run` promises the following:

- **The panic reaches the caller exactly once**, on the thread that
  called `run`, with the original `Box<dyn Any + Send>` payload. If
  several systems panic in one run, the first captured payload wins; each
  other one is discarded with the diagnostic `boyko-E0202`.
- **The run is cancelled at round granularity.** No system is dispatched
  after the panic is observed; systems already spawned run to completion.
  Delivery happens when the pool's scope has drained, never earlier.
- **The schedule stays reusable, and the world stays well-formed but
  semantically partial.** The per-frame scratch state is reset on the
  next `run`, and no storage invariant is broken. What is *not* promised
  is application-level consistency: the panicked system's half-written
  state stays. The world is not rolled back.
- **Commands queued during the aborted run stay queued** in the buffer of
  the system that queued them. They are applied the next time that system
  is applied — in a later run that dispatches it (a system whose run
  conditions are false is skipped without an apply, so its commands keep
  waiting).

Two consequences follow. A system that panicked has already advanced its
change-detection window, so on its next run it sees only changes since
the aborted run. And because `Commands` claims entity ids at enqueue
time, a cancelled run leaves those ids claimed until that system's next
apply.

## Performance

The scheduler has its own criterion harness,
`crates/boyko_ecs/benches/phase9_scheduler.rs` (an empty schedule, one
exclusive system, 50 exclusive systems, two disjoint systems, and
`par_iter` over 4096 entities).
Measured results, each with its date and commit, are collected on the
[Benchmarks](reference/benchmarks.md) page.

## What layers on top

The executor core is extended by the **same** `Schedule` /
`ScheduleBuilder`, without re-architecting it:

- **Schedule ordering & sets** — `.before_set` / `.after_set` and
  `configure_set`, expanded into per-member edges at `build`
  ([Ordering & Sets](scheduling/ordering-and-sets.md)).
- **Run conditions** — `.run_if(cond)` gates a system's body per frame;
  conditions evaluate single-threaded at the apply-window barrier
  ([Run Conditions](scheduling/run-conditions.md)).
- **States** — `State<S>` / `NextState<S>` with the `in_state` /
  `on_enter` / `on_exit` / `on_transition` conditions, built on the same
  run-condition mechanism ([States](scheduling/states.md)).
- **App / Plugin facade** — the `App` builder owns one or more
  `Schedule`s; plugins register systems through it
  ([App & Plugins](app/plugins.md)).
- **Fixed timestep** — `Time` / `FixedTime` and a `CoreSchedule` driving a
  fixed-step inner loop ([Time & Fixed Timestep](app/time.md)).

The one-shot runners (`EcsMaster::run_system`, `run_cached_system`,
`run_system_once`, `run_closure_once`) remain the right tool for a single
system outside a frame loop; see [Resources](concepts/resources.md#a-standalone-system-run).

## Further reading

- `docs/archive/PHASE-9-PARALLEL-SCHEDULER-PLAN.md` — the original design
  record, for contributors: the numbered invariants the source comments
  cite (SCH1-15 for the executor, including SCH7 for the apply window;
  SEND1-3; EVT1-4; ALLOC1-6; EXC1-2; PAR1-9; CQ-SEND1-2) and the test
  matrix.
- `crates/boyko_threadpool/` — the underlying work-stealing pool. Not
  intended for direct user consumption; `ScheduleBuilder::new` and
  `par_iter` are the right entry points.
- `crates/boyko_ecs/tests/scheduler_par_iter_concurrent_systems.rs` —
  end-to-end integration test exercising the full `par_iter` ×
  `Schedule::run` path.
- `crates/boyko_ecs/tests/par_iter_compile_fail/capture_commands.rs` —
  the trybuild compile-fail fixture proving `&mut Commands` cannot be
  captured inside a `par_iter` body (CQ-SEND2), driven by the harness
  `crates/boyko_ecs/tests/par_iter_captures_commands_fails.rs`.
