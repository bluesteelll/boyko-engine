# Phase 6 — Event dispatch + double buffer

**Status:** ✅ DONE.
**Branch:** `ecs`
**Detailed architectural plan:** [`docs/PHASE-6-EVENT-DISPATCH-PLAN.md`](../PHASE-6-EVENT-DISPATCH-PLAN.md)
(1315 lines, full SAFETY contracts and per-step migration).
**Tests:** 32 dedicated event tests + 1 proptest suite + benches.
**Commits on `ecs`:** `eefc9c0`, `a680223`, `3272700`, `4258247`,
`5f10ad3`, `4c8d7da`, `69c3948`, `fb21719`, `d4f6512`.

## Goal

Replace the dead `EventPool` / `EventPoolBundle` design with a
double-buffered, per-thread, lock-free event dispatch path that:

1. Is correct under multi-thread emit and single-thread drain.
2. Drops typed events via registered `drop_fn` on buffer clear.
3. Splits writer-side and reader-side cache lines to eliminate
   false sharing.
4. Uses a flat fn-pointer vtable rather than `dyn Trait` for type
   erasure — zero virtual-call overhead in hot path.
5. Provides a clean `EcsMaster::send_event` / `drain_events` API
   without exposing the per-thread lane internals.

## Why before Phase 7

Phase 7 (fast random access) reduces `get_component_raw` from
~40 ns to ~12 ns. The event drain path is the **only** other
candidate for sustained hot-loop hits per frame; it had to be
lock-free and double-buffered before any "scheduler hot path"
work could even be sketched.

## Design summary (full detail in linked plan)

### Core types

```text
EventDispatcher          — top-level, lives in EcsMaster
  ├── lanes: [ThreadLane; MAX_THREADS]
  └── buffers: [TypedBufferSet; MAX_EVENT_TYPES]

ThreadLane
  ├── writer_state (cache line 1)
  └── reader_state (cache line 2)
       — split to avoid false sharing between emitting thread
         and draining thread

EventBuffer<E>
  ├── front: UnsafeCell<Vec<E>>     — written by emitters
  └── back:  UnsafeCell<Vec<E>>     — read by drainers
       — swap on frame boundary
```

### Vtable shape

```rust
struct EventVtable {
    drop_fn:  unsafe fn(*mut u8),
    clear_fn: unsafe fn(*mut u8),
    swap_fn:  unsafe fn(*mut u8),
}
```

Flat `fn`-pointer table — no `Box<dyn Trait>`, no `Arc<RwLock<...>>`.

### Concurrency model

- **Emit phase:** each thread writes only to its own
  `ThreadLane.writer_state[front_buffer]`. No locks, no
  cross-thread coordination.
- **Drain phase:** runs on a single thread between frames.
  Swaps front / back, iterates back buffers in registered order,
  calls user closure with `&E`, invokes `drop_fn` for each event,
  resets length to 0.
- **No `Mutex` / `RwLock` / `RefCell` anywhere in the hot path.**

### Memory budget

- `MAX_THREADS = 64`, `MAX_EVENT_TYPES = 256`, cache-line aligned
  `ThreadLane` ≈ 128 bytes ⇒ `EventDispatcher` ≈ 8 KB metadata.
- Per-buffer capacity: configurable via `EventConfig`. Default
  is a small initial allocation that grows once on first emit
  per type, then never reallocates.

## Implementation steps — all landed

| Step | Commit | Description |
|------|--------|-------------|
| 0 | `a680223` | `BitSet256` + `pop_lowest_set_bit` in `boyko_utils` (prereq). |
| 1–2 | `3272700` | `EventConfig`, `ZstCheck`, `EcsError` variants. |
| 3 | `4258247` | `EventBuffer<E>` with split cache-line lanes. |
| 4 | `5f10ad3` | `EventDispatcher` with fn-ptr vtable. |
| 5 | `4c8d7da` | Wire dispatcher into `EcsMaster`. |
| 6 | `69c3948` | Full test suite — 32 tests + 1 proptest + benches. |
| 7 | `fb21719` | Clippy + test-isolation + frame-wrap fixes. |
| 8 | `d4f6512` | Code-review follow-ups: panic-safe drops, cleanup. |

## Tests landed

- `tests/event_double_buffer.rs` — front / back swap invariants.
- `tests/event_multi_type.rs` — independent types in same frame.
- `tests/event_proptest.rs` — randomised emit / drain sequences,
  invariant: every emitted event is delivered exactly once and
  exactly one drop per event.
- `benches/event_dispatch.rs` — single-thread emit, multi-thread
  emit, full frame drain.

## Bench numbers (Windows x86_64, criterion)

Captured at commit `d4f6512`:

| Operation | Time | Notes |
|-----------|------|-------|
| `EventDispatcher::send_event::<E>` (single emit) | ~6 ns | Hot path, single thread. |
| `drain_events_typed::<E>` (per event) | ~3 ns | Direct iteration. |
| Frame drain of 100 k events × 3 types | ~1 ms | Includes drop_fn. |
| Swap (front ⇄ back) cost | < 50 ns | Atomic flag flip. |

These numbers are the **baseline** Phase 7 / 8 / 9 must not regress.

## Exit criteria — all met

- [x] `cargo test --all-targets` includes all 32 event tests + proptest.
- [x] `cargo bench --bench event_dispatch` runs end-to-end.
- [x] `cargo +nightly miri test event_attribute drop_fn event_*`
      clean under tree-borrows.
- [x] `EventPool` / `EventPoolBundle` files remain commented out;
      Phase 6 does **not** revive them.
- [x] `EcsMaster` exposes only `send_event` / `drain_events` /
      `register_event`. Lane internals stay `pub(crate)`.
- [x] No `dyn Trait`, no `Mutex` / `RwLock`, no `Box<…>` per emit.

## What this phase did NOT do

- It did **not** add event filtering / subscriber model — that is
  the Q-020 reopening trigger (not currently planned).
- It did **not** integrate with a scheduler — Phase 9 will tie
  `drain_events` to scheduler frame boundaries.
- It did **not** address the legacy `EventPool` Drop semantics
  (Phase 3d, still blocked).

## Cross-phase dependencies (already satisfied)

- Phase 1b drop-discipline pattern (typed `drop_fn` on
  `ComponentLayout`) provided the template for `EventVtable`.
- Phase 2c criterion infrastructure was a precondition for the
  event benches.
- Phase 4a newtype `ComponentId` / `ArchetypeId` flow informed the
  newtype `EventId` design (kept as a thin `u16` wrapper).

## Cross-phase dependencies for downstream phases

- **Phase 9 scheduler** will call `drain_events` exactly once per
  frame between system batches.
- **Phase 8 system API** will expose `EventReader<E>` /
  `EventWriter<E>` as system-parameter newtypes wrapping the
  Phase 6 primitives.

## References

- Detailed plan: [`docs/PHASE-6-EVENT-DISPATCH-PLAN.md`](../PHASE-6-EVENT-DISPATCH-PLAN.md)
  (architect → critic three-round cycle, all SAFETY contracts).
- Source files:
  - `crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs`
  - `crates/boyko_ecs/src/ecs/core/events/event_buffer.rs`
  - `crates/boyko_ecs/src/ecs/core/events/event_config.rs`
  - `crates/boyko_utils/src/bit_mask/bit_set_256.rs`
- Audit context: Q-007, Q-018, Q-020, Q-023 (all addressed or
  documented as deferred).
