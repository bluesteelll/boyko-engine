# Phase 9 — Parallel scheduler

**Status:** ⚪ DRAFT — design exploration only. Implementation
depends on Phase 8 system API existing.
**Branch (when active):** `ecs`.

## Goal

Wire systems registered via Phase 8 `IntoSystem` into a runtime
that executes them across multiple OS threads with provable
data-race freedom, zero allocation per frame, and bounded latency
on common workloads (~100 k entities, 60-120 fps target).

## Why after Phase 8

Phase 9 needs `SystemParam::check_access` to know which `&` /
`&mut` accesses each system requires before scheduling. Without
Phase 8's typed-parameter discipline, the scheduler would either
need runtime aliasing checks (cost) or hand-rolled per-system
declarations (boilerplate).

## High-level design — load-bearing decisions to take

| D | Topic | Direction (subject to architect cycle) |
|---|-------|----------------------------------------|
| D9.1 | Graph topology source | Build at registration time from `SystemParam::check_access` returns. |
| D9.2 | Conflict detection | Component-mask + resource-id bit-overlap test. No `HashMap`. |
| D9.3 | Thread pool | Custom lock-free pool sized to `available_parallelism()` minus one (main thread participates). |
| D9.4 | Work-stealing | Per-thread mailbox + global atomic round-robin steal. Crossbeam-deque-style. |
| D9.5 | Frame boundary | Single atomic generation counter; systems read events for generation N, writers commit at end-of-frame swap. |
| D9.6 | `Commands` flush | Single-threaded apply between system stages; no cross-thread visibility issues. |
| D9.7 | Exclusive systems | `&mut World` systems run alone — stage boundary forces synchronisation. |

## Reference points

- **Bevy `Schedule` / `SystemSet`** — current state of the art for
  Rust ECS scheduling. Two-tier: stages (linear) + systems (DAG within
  stage). Stage barriers force `Commands` flush.
- **Unity DOTS `JobScheduler`** — different in that it integrates
  with the job system at a finer grain; less applicable.
- **TBB `flow_graph`** — general-purpose data-flow scheduling;
  inspiration for the lock-free queues.

## Scope sketch — what Phase 9 must deliver

### 9a — `Schedule` struct

A `Schedule` owns:

- `systems: Vec<SystemBox>` — boxed via thin wrapper (not `dyn`);
  could be `enum SystemBox { Parallel(Box<…>), Exclusive(Box<…>) }`.
- `access_table: Vec<Access>` — per-system component / resource
  mask.
- `conflict_graph: Vec<u64>` — bitset per system listing systems
  it conflicts with.
- `topo_order: Vec<Vec<SystemId>>` — pre-computed stages of systems
  that can run concurrently.

### 9b — Conflict detection

```rust
fn conflicts(a: &Access, b: &Access) -> bool {
    // Either side writes anything the other touches → conflict.
    (a.writes & (b.reads | b.writes)) != 0 ||
    (b.writes & (a.reads | a.writes)) != 0
}
```

`Access` is a packed bitset (component mask + resource mask + event
mask). No `HashMap`; everything indexable by `ComponentId.0`.

### 9c — Thread pool

- `spawn` once at `Schedule::new`; threads park on a per-thread
  channel.
- Work-stealing via lock-free deque. Each thread's local queue is
  drained first; on empty, steal from a victim thread chosen via
  atomic round-robin.
- No allocation per task — `SystemId` is a `u16`, fits in a packed
  enqueue word.

### 9d — Frame loop

```text
Schedule::run(&mut world) {
    for stage in &topo_order {
        // Fire all systems in this stage in parallel
        for sid in stage {
            thread_pool.schedule(sid);
        }
        thread_pool.wait_idle();
        // Flush Commands buffers in registration order
        apply_commands(&mut world);
        // Bump frame counter for change detection (if Phase 10 lands)
    }
}
```

Stage boundaries enforce all the synchronisation `Commands` /
change-detection need.

### 9e — Exclusive systems

Systems requesting `&mut World` run alone:

- They form their own stage.
- The scheduler pauses parallel execution, runs the exclusive system
  on the main thread, then resumes.

### 9f — Event boundary

Phase 6 `EventDispatcher::swap_buffers` is called once per frame,
between stages. Phase 9 picks the exact insertion point:

- After all stage-N writes, before stage-(N+1) reads.
- For now, the simplest model is: swap at end-of-frame. Multi-frame
  staging (Bevy's `swap_at_start_of_next_frame`) is a Phase 10
  refinement.

## Performance targets

| Metric | Target |
|--------|--------|
| Single-system frame overhead | ≤ 100 ns (dispatch + bookkeeping) |
| 100-system frame overhead, no work | ≤ 10 µs (scheduler bookkeeping alone) |
| Speedup at 8 cores, 100 systems, balanced | ≥ 6× over single-threaded |
| Wakeup latency on idle thread | ≤ 5 µs (lock-free park / unpark) |
| `Commands::flush` cost per command | ≤ 200 ns (parity with Phase 8) |
| Allocations per frame | 0 |

## Open questions (require architect cycle)

| Q | Decision needed |
|---|-----------------|
| Q-9.1 | Use `rayon` / `tokio` / custom thread pool? | Probably custom — `rayon`'s assumptions don't fit ECS dispatch. |
| Q-9.2 | How does the scheduler interact with `EventDispatcher` MAX_THREADS? | Threads must self-identify on first emit; scheduler enforces 1:1 mapping. |
| Q-9.3 | System sets / labels for ordering hints? | Bevy has a rich label system; we may want a smaller version. |
| Q-9.4 | Run-criteria / system conditions (`run_if(...)`)? | Phase 9 minimum or punt to Phase 10? Probably minimum: a single `fn() -> bool` per system. |
| Q-9.5 | Soft / hard ordering constraints (`before`, `after`)? | Likely Phase 9 minimum. |
| Q-9.6 | Multi-`Schedule` worlds (e.g. fixed-update + render)? | Defer to post-9. |
| Q-9.7 | `loom` testing strategy | Critical given lock-free design. Must integrate before Phase 9 lands. |

## Risks

| Risk | Mitigation |
|------|------------|
| Loom-detectable race in work-stealing | All shared mutation through carefully chosen atomics; `loom` test suite mandatory before commit. |
| ABBA deadlock between systems | The DAG construction must be acyclic; runtime check at `Schedule::build`. |
| Cache-line contention on the work queue | Per-thread queue + cache-line padding on shared atomics. |
| `Commands` ordering becomes non-deterministic | Flush in registration order; document determinism is preserved unless system bodies explicitly randomise. |
| Stage-boundary cost dominates small frames | Optimise stage transitions to a single atomic; reuse worker threads across stages. |

## Out of scope

- **Async / `Future` integration** — not a Phase 9 deliverable; the
  scheduler is sync-only.
- **GPU dispatch** — out of scope entirely.
- **Distributed / multi-process** — out of scope entirely.
- **Per-system change detection ticks** — Phase 10.

## Cross-phase dependencies

- **Phase 8 SystemParam** — `check_access` is the data source for
  the conflict graph.
- **Phase 6 EventDispatcher** — swap point determined by scheduler.
- **Phase 7 stable archetype pointers** — system threads dereference
  the same `*mut Archetype` simultaneously when accesses are
  read-only; that requires `Archetype: Sync`, which in turn requires
  `Arena: Sync` and `ComponentPool: Sync`. Phase 7 **documents** the
  requirement; Phase 9 **enforces** it.

## How to launch

When Phase 8 lands and the user gives explicit go-ahead:

1. Dispatch `researcher` for an in-depth comparison of Bevy's
   `Schedule`, Unity DOTS' `JobScheduler`, and lock-free work-stealing
   queue algorithms.
2. Dispatch `architect` for sub-phase 9a (`Schedule` struct + access
   detection — smallest cohesive piece).
3. Cycle `architecture-critic` — extra rigor here; this is the most
   complex phase to date.
4. Repeat per sub-phase 9b–9f.
5. **Tester must integrate `loom`** for the work-stealing pool
   before any commit lands. Phase 2c deferred `loom` "for Phase 4+";
   Phase 9 makes it mandatory.

## Estimated phasing

- **9a Schedule struct + access detection** — 1 architect, 2 developer.
- **9b Conflict detection + DAG** — 1 architect, 2 developer.
- **9c Thread pool + work-stealing** — 2 architect, 3 developer,
  1 dedicated `loom` session.
- **9d Frame loop + Commands flush** — 1 architect, 2 developer.
- **9e Exclusive systems** — 1 architect, 1 developer.
- **9f Event swap integration** — 1 architect, 1 developer.
- **9g Benchmarks + tests** — 1 developer, 2 tester, 1 results-analyst.

Total estimate: 10-12 sessions end-to-end. The longest-running
phase by a wide margin.

## References

- Bevy `Schedule`: <https://docs.rs/bevy_ecs/latest/bevy_ecs/schedule/>.
- Unity DOTS scheduler:
  <https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/scheduling-systems.html>.
- Chase-Lev work-stealing deque (canonical lock-free design):
  Chase, D. & Lev, Y. (2005), "Dynamic Circular Work-Stealing Deque".
- `loom` for Rust concurrency tests: <https://github.com/tokio-rs/loom>.
