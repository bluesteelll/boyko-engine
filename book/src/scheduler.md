# Parallel Scheduler

The scheduler is the engine's top-level system runner. It takes a set of
registered systems plus their declared `Access` surfaces and a `ThreadPool`,
and runs them concurrently when their accesses don't conflict — fanning
work onto worker threads, auto-inserting synchronization points where
`Commands` deferral demands them, and re-raising the first panic seen on
any worker.

This page covers the user-facing scheduler API as introduced by **Phase 9**
of the engine. The internal contracts (apply window, conflict graph,
incremental ready-set) live in the architecture deep-dives — most users do
not need them.

## Why a parallel scheduler

Single-threaded ECS loops cap at the rate one CPU core can stream
component bytes — for an engine targeting AAA-scale entity counts (1M+
entities, 60 Hz tick), that's not enough. Phase 9 adds:

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
both living inside an out-of-line `CompletionChannel` (Phase 9.3c) the
workers reach through a `NonNull` rather than through the dispatcher's
`&mut self`.

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

A third type — [`SystemConfig`](#systemconfig) — is the value returned
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

// Build a thread pool. Worker count defaults to num_cpus::get() if not
// specified. Reuse the pool across schedules — building one is expensive.
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
`ExclusiveSystemMarker` (Phase 8c Q9). The blanket coexists with the
`SystemParamFunction` blanket for the same name without a coherence
conflict.

## `Commands` and the apply window

A system that enqueues structural mutations through `Commands`:

```rust,ignore
use boyko_ecs::ecs::core::system::Commands;

fn spawn_enemies(mut commands: Commands) {
    commands.spawn(EnemyBundle { hp: 100, pos: Position { x: 0.0, y: 0.0, z: 0.0 } });
}
```

…enqueues a `SpawnCommand<EnemyBundle>` into the system's per-system
`CommandQueue`. The queue is then flushed against `world` during the
**apply window** of the dispatch round — the serial phase between waves
where the dispatcher holds `&mut EcsMaster` exclusively.

The apply window contract (SCH7):

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
  (CQ-SEND2 — the failing fixture is
  `crates/boyko_ecs/tests/par_iter_compile_fail/capture_commands.rs`, run
  by the trybuild harness `tests/par_iter_captures_commands_fails.rs`).
- **Inline threshold** — archetypes with fewer than
  `MIN_ARCHETYPE_FOR_PARALLEL` (= 1024) rows run inline on the calling
  thread. The fork-join overhead would otherwise dominate.
- **Nested scopes** — `par_iter` calls `pool.scope`, which is re-entrant.
  Calling `par_iter` from inside a system body that is itself running on
  a worker works without deadlock (the rayon work-stealing pattern in
  `Scope::Drop`).

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
- `.run_if(cond)` (Phase 16) — attach a run condition.
- `.gpu()` (Phase 5) — mark a GPU-compute system (runs dispatcher-solo at the apply-window barrier).

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
- **`.run_if(cond)`** — attach a run condition (Phase 16).
- **`.gpu()`** — mark a GPU-compute system (Phase 5).

```rust,ignore
let physics = builder
    .add_system(physics_step)
    .in_set(PhysicsSet)
    .key();

builder
    .add_system(input_handler)
    .before(physics);
```

## Context discipline (ALLOC1)

There is no shared arena allocator to protect — the engine retired the
shared `Arena` in **Phase X.J**. Component storage now lives in per-pool
virtual-memory reservations: each `ComponentPool` reserves a fixed,
address-stable row ceiling up front (`ComponentPool::new(component_id,
reserve_rows)` on a `VmReservation`) and commits frontier pages lazily as
rows are added. There is no global `Arena::allocate_*` call on the hot
path.

Growth (`ComponentPool::grow_rows`) is plain `&mut self` field mutation —
it commits more pages on the pool's **own** reservation and never moves
the base pointer. Because it is reachable only through `&mut` paths (the
owner's direct API, or the apply window where the dispatcher holds
`&mut EcsMaster` and SCH7 guarantees zero workers in flight), the `&mut`
exclusivity **is** the guard. The commit syscalls are not global-allocator
calls, so they need no separate allocation flag
([`component_pool.rs:2023`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/memory/component_pool.rs#L2023)).

What survives from the old discipline is a thread-local **context flag**.
The dispatcher wraps every system body in
`boyko_threadpool::InSystemRunGuard::enter()`
([`schedule.rs:1239`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/schedule/schedule.rs#L1239)),
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
([`event_config.rs:66`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/events/event_config.rs#L66)) —
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
- **Workers** are the OS threads owned by `ThreadPool`. Each one has a
  local injector (Chase-Lev deque), a stealer pointing at every sibling
  deque, and TLS state (`current_worker_id`, `is_in_system_run`).
- `Schedule::run` enters `pool.install(|scope| ...)` once per frame. The
  install sets `ACTIVE_POOL` TLS for the calling thread so that ambient
  `par_iter` calls inside system bodies can discover the pool without an
  explicit argument.

`EcsMaster: Send + Sync` and `UnsafeEcsCell<'w>: Send + Sync` are the
two unsafe Send/Sync impls that enable workers to receive cell copies.
The aliasing discipline is enforced upstream:

- **Conflict graph (SCH3)** — at run time, no two concurrent systems
  hold overlapping `&/&mut` views through their cell copies. The graph
  is built at `ScheduleBuilder::build` from declared `Access` surfaces.
- **Apply-window barrier (SCH7)** — the dispatcher reborrows
  `&mut EcsMaster` only when all dispatched systems have reported
  completion (the gate `pending_apply == running.count_ones()`).

## Panic handling

The scheduler re-raises the first panic observed by any worker, on the
dispatcher thread, surfaced through `Scope::Drop`. Subsequent panics
within the same frame are dropped (logged in `scheduler-trace` feature
builds). The world is **not rolled back** — a panicking system may have
left partial mutation behind. Save/restore is the application's concern.

## Performance targets

Phase 9 binding targets (plan §1.2):

| Operation                                | Target          |
|------------------------------------------|-----------------|
| Schedule build (50 sys, no cycles)       | ≤ 50 µs         |
| Per-frame dispatch (50 sys, 16 threads)  | ≤ 20 µs         |
| Per-frame dispatch (1000 sys, 16 threads)| ≤ 200 µs        |
| Steal cost (worker idle → sibling deque) | ~100 ns         |
| Worker wake-up latency                   | ≤ 1 µs          |
| `par_iter` per-chunk dispatch            | ≤ 200 ns        |
| Steady-state worker idle                 | ≤ 1% CPU / core |

Measured numbers (8-core Windows reference box, `cargo bench --release`):

| Bench                               | Time      | Target  | Headroom |
|-------------------------------------|-----------|---------|----------|
| `phase9_schedule_run_empty`         | ~3.5 ns   | n/a     | n/a      |
| `phase9_schedule_run_50_exclusive_systems` | ~4.3 µs | ≤ 20 µs | 5× |
| `phase9_par_iter_4096_entities`     | ~25.0 µs  | n/a     | n/a      |
| `phase9_schedule_run_two_disjoint`  | ~1.6 µs   | n/a     | n/a      |
| `phase9_schedule_run_one_exclusive` | ~265 ns   | n/a     | n/a      |

See `crates/boyko_ecs/benches/phase9_scheduler.rs` for the measurement
harness.

## Migration from Phase 8.x

`EcsMaster::run_system`, `run_cached_system`, `run_system_once`, and
`run_closure_once` all continue to work unchanged. Phase 9 is an
**additive** layer — the existing single-system entry points are still
the correct choice for one-off invocations.

Phase 8.x users who want to upgrade can do so incrementally:

1. Build a `ThreadPool` once at world setup.
2. Build a `ScheduleBuilder`; move existing per-frame `run_system` calls
   to `builder.add_system(...)`.
3. Replace the frame loop's individual `run_system` calls with one
   `schedule.run(&mut world)` call.

No `Cargo.toml` change is required for `boyko_ecs` users — the public
re-exports (`Schedule`, `ScheduleBuilder`, `SystemConfig`, `SystemSet`)
flow through `boyko_ecs::ecs::core::schedule::*`.

## What layers on top

Phase 9 is the execution core. Later phases extend the **same**
`Schedule` / `ScheduleBuilder` without re-architecting it:

- **Schedule ordering & sets** (Phase 15) — `.before_set` / `.after_set`
  and `configure_set`, expanded into per-member edges at `build`.
- **Run conditions** (Phase 16) — `.run_if(cond)` gates a system's body
  per frame; conditions evaluate single-threaded at the apply-window
  barrier.
- **States** (Phase 17) — `State<S>` / `NextState<S>` with the
  `in_state` / `on_enter` / `on_exit` / `on_transition` conditions, built
  on the same run-condition mechanism.
- **App / Plugin facade** (Phase 18) — the `App` builder owns one or more
  `Schedule`s; plugins register systems through it.
- **Fixed timestep** (Phase 20) — `Time` / `FixedTime` and a `CoreSchedule`
  driving a fixed-step inner loop.

## Further reading

- `docs/PHASE-9-PARALLEL-SCHEDULER-PLAN.md` — the architectural plan
  with full §2 invariants (SCH1-15, SEND1-3, EVT1-4, ALLOC1-6,
  EXC1-2, PAR1-9, CQ-SEND1-2) and the §13 test matrix.
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
