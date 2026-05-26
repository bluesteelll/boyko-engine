# Phase 9 — Migration / Legacy Compatibility Audit

**Branch:** `ecs`
**Step:** Wave 7 Step 23
**Date:** 2026-05-26

This document records the legacy-compatibility audit performed at the end
of Phase 9 implementation, per plan §15 and §14 Step 23. The audit
verifies that Phase 8.x users do not need source-level changes to upgrade
to Phase 9, and enumerates the small set of additive behavioural changes.

---

## §1 — Single-system entry points (Phase 8a + 8c)

All four legacy entry points on `EcsMaster` continue to work unchanged:

| Method                                   | Status       | File reference                                                                |
|------------------------------------------|--------------|-------------------------------------------------------------------------------|
| `EcsMaster::run_system_once<S>`          | Unchanged    | `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:919`                  |
| `EcsMaster::run_closure_once<F, M, Out>` | Unchanged    | `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:949`                  |
| `EcsMaster::run_system<F, M, Out>`       | Unchanged    | `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:987`                  |
| `EcsMaster::run_cached_system<S>`        | Unchanged    | `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:1019`                 |

Verification: `cargo test -p boyko-ecs --test into_system_closure_inference`
(3 tests), `cargo test -p boyko-ecs --test phase8cd_integration` (7 tests),
and `cargo test -p boyko-ecs --test system_param_smoke` all pass on the
Phase 9 branch.

Phase 9's `Schedule::run` uses `System::run_unsafe` directly via the
`SystemBox` indirection and does **not** call `EcsMaster::run_system`,
so the two paths are decoupled. Users may freely mix:

```rust,ignore
// Per-frame: scheduled systems run via the schedule.
schedule.run(&mut world);

// Out-of-band: one-shot systems still use the legacy API.
let result = world.run_system(my_diagnostic_query_system);
```

---

## §2 — Phase 8c+8d test files

The following Phase 8c/8d test files were inspected at the end of Phase
9; **none required modification**:

- `tests/derive_bundle_smoke.rs` — 7 tests, all green.
- `tests/phase8cd_integration.rs` — 7 tests, all green.
- `tests/into_system_exclusive_smoke.rs` — 4 tests; one pre-existing test
  (`into_system_exclusive_fn_compiles_and_runs`) intermittently fails due
  to a static-counter race when tests run in parallel within the file.
  This is a Phase 8c test design issue independent of Phase 9 and was
  observed on the same branch before Phase 9 changes landed.
- `tests/into_system_closure_inference.rs` — 3 tests, all green.
- `tests/command_queue_panic_recovery.rs` — green.
- `tests/bundle_panic_safety.rs` — green.

Phase 9 leaves the Phase 8c/8d API surface alone. The new `FunctionSystem<F, M>`
is consumed by the new `Schedule` infrastructure but neither requires nor
provides changes to the existing `EcsMaster::run_*` flow.

---

## §3 — Phase 8.5 static bundle cache

The Phase 8.5 `bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; 1024]>`
is **unaffected** by Phase 9.

Verification points (plan §15.4):

- `OnceLock<T>: Sync` when `T: Send + Sync`. `ArchetypeId: Copy + Send +
  Sync`. The cache is safely shareable.
- Phase 9 reads the cache only on the dispatcher (apply window). Worker
  bodies enqueue spawn commands via `Commands::spawn`; the lookup against
  `cached_archetype_id` happens during `SpawnCommand::apply`, which runs
  on the dispatcher under `&mut EcsMaster`.
- No worker-side `OnceLock::set` race exists. Two systems both calling
  `Commands::spawn(SameBundle)` enqueue independent `SpawnCommand`s;
  both flushes are serialised in the apply window. The first call to
  `OnceLock::set` succeeds; the second sees an already-populated cache.

`tests/derive_bundle_smoke.rs` confirms the cross-world isolation
invariant remains valid under Phase 9.

---

## §4 — Phase 6 events: semantic shift in `EventConfig::default_for`

Phase 9 introduces a new lane discipline for events (plan §2.8 EVT1):
every worker writes to its own lane (indexed by `current_worker_id`); the
dispatcher writes to an additional lane indexed at `worker_count`. The
total number of lanes is therefore `worker_count + 1`.

### Existing API (unchanged)

- `EventConfig::default_for(thread_count: u32)` — constructs the default
  per-lane buffer config for a dispatcher with `thread_count` lanes.
- `EventDispatcher::new(config: EventConfig)` — takes the config.

### Migration step for new code

Pre-Phase 9 callers wrote:

```rust,ignore
let cfg = EventConfig::default_for(num_workers).unwrap();
```

Post-Phase 9 schedule users should write:

```rust,ignore
// Reserve a lane for the dispatcher (EVT1).
let cfg = EventConfig::default_for(num_workers + 1).unwrap();
```

The `+ 1` is the dispatcher lane. Failure to add it does not crash —
`send_event` from the dispatcher would land on lane `worker_count` and
`thread_count == worker_count` would silently skip the write (per
`tid < thread_count` bounds check in `EventDispatcher::send`).

### Existing callsites in tests

`crates/boyko_ecs/tests/event_send_from_worker.rs` is the only existing
test that uses `EventConfig::default_for` with a multi-worker
configuration. It already accounts for the `+ 1` lane.

`crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:865` —
`EcsMaster::preregister_event_default` uses
`self.events.default_thread_count()`, which was set at `EventDispatcher`
construction. The user is responsible for picking the correct
`thread_count` when calling `EventDispatcher::new`.

The conservative recommendation in the Phase 9 plan is to standardise on
`EventConfig::default_for(thread_count + 1)` everywhere — Phase 9 users
get the dispatcher lane for free; pre-Phase 9 users get one extra unused
lane (≤ 16 KB overhead per buffer).

---

## §5 — Send/Sync impl additions

The following types gained `unsafe impl Send + Sync` impls in Phase 9
Wave 2 (plan §15.1):

- `EcsMaster` — `ecs_master/ecs_master.rs:1286`
- `UnsafeEcsCell<'w>` — `system/unsafe_ecs_cell.rs:294-295`
- `ComponentPool` — `memory/component_pool.rs:715-716`
- `Archetype`, `Column` — `archetype/archetype.rs:540-541, 551-552`
- `ArchetypeMaster` — `archetype/archetype_master.rs:638-639`
- `EntityMaster` — `entity/entity_master.rs:367-368`
- `EventDispatcher` — `events/event_dispatcher.rs:459-460`

These are **additions**. No existing Send/Sync impl was removed or
relaxed. Code that did not previously rely on `Send + Sync` is unaffected.

The notable **non-addition**: `Arena` retains `!Send + !Sync` per the
Round 2 C1 resolution. This is enforced by the absence of an impl block;
no source-level change is visible to users.

---

## §6 — `Access::is_universal()` and `Access::universal()`

Two new methods on `Access`:

- `fn is_universal(&self) -> bool` — returns `true` iff all 4 bitmask
  fields are fully set.
- `fn universal() -> Self` — constructs an `Access` with every bit set.

These are net-new public methods. Users typically interact with `Access`
indirectly through `SystemMeta`, so the additions do not break existing
code.

---

## §7 — Conclusion

**No source-level migration is required** for existing Phase 8.x users.
Phase 9 is an additive layer:

1. `EcsMaster::run_*` entry points unchanged.
2. Phase 8.5 cache untouched.
3. Phase 8c/8d API unchanged.
4. Only behavioural delta: `EventConfig::default_for(thread_count + 1)`
   for new schedule-driven event flow (and a no-op for single-thread or
   pre-Phase 9 users — the extra lane is harmless overhead).

The audit found no API regression and no semantic change to existing
single-system entry points.
