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
4. **Allocation discipline** — system bodies cannot allocate from the
   `Arena` (debug-asserted). All growth happens on the dispatcher during
   `ScheduleBuilder::build` or the per-frame apply window.

The scheduler does not introduce `Mutex` or `RwLock` on the hot path.
Cross-worker synchronization is one `AtomicUsize` per frame
(`pending_apply`) plus a lock-free `ArrayQueue` for completions.

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
from `add_system(...)`. It carries the `.before`, `.after`, `.chain`, and
`.in_set` fluent hints.

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
// ordering / set membership.
builder.add_system(physics_step);
builder.add_system(render_prepare).after(physics_step);

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

**Commands::send_event<E>(event)** also lives here — events emitted via
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
  (CQ-SEND2 — see `tests/par_iter_captures_commands_fails.rs`).
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
builder.add_system(render).in_set(RenderSet).after(PhysicsSet);
```

`.before(SystemSet)`, `.after(SystemSet)`, `.in_set(SystemSet)`, and
`.chain()` are the four ordering primitives. They compose; conflicts
between hints panic at `build` time with a cycle diagnostic.

## `SystemConfig` fluent API

`SystemConfig` (returned by `add_system`) carries:

- **`.before(other)`** — this system runs before `other`.
- **`.after(other)`** — this system runs after `other`.
- **`.chain()`** — chained with the previously-added system (shorthand
  for `.after(prev)`).
- **`.in_set(SET)`** — adds this system to the named `SystemSet`.

```rust,ignore
builder
    .add_system(physics_step)
    .in_set(PhysicsSet);

builder
    .add_system(input_handler)
    .before(physics_step);
```

## Allocation discipline (ALLOC1)

System bodies **must not** allocate from the `Arena`. The dispatcher
sets a thread-local flag (`IN_SYSTEM_RUN`) around every system body via
the `InSystemRunGuard`. `Arena::allocate_*` `debug_assert!`s the negation
of this flag.

All allocation happens during the apply window (dispatcher-only) or
during `ScheduleBuilder::build`. The disciplined paths:

- Archetype growth — deferred via `Commands::spawn` and resolved on
  apply.
- `CommandQueue::push` — uses its own `Vec<MaybeUninit<u8>>`, which IS
  on the heap, but is a per-system queue allocated before the system
  body runs.
- Event buffer growth — flagged in plan §11.5 as a pre-Phase 10 audit
  item; today only fires on first-write per buffer.

If you find a `debug_assert!` firing inside a system body, the rule is
not to relax the assert — it's to refactor the allocation to happen
outside the body. The release-mode safety net is the `force_alloc_panic`
cfg gate (see `docs/PHASE-9-FORCE-ALLOC-PANIC.md`).

## Event lanes

The event dispatcher reserves one lane per worker plus one lane for the
dispatcher. Worker bodies emit events to their own lane; the dispatcher
emits during the apply window. The number of lanes is set at
`EventConfig::default_for(worker_count + 1)` time.

User code calls `EventDispatcher::send_event<E>(event)` (or
`Commands::send_event<E>(event)` for deferred-from-system emission); the
TLS `current_worker_id` router places the write on the correct lane.

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
| `phase9_schedule_run_50_exclusive`  | ~4.3 µs   | ≤ 20 µs | 5×       |
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
