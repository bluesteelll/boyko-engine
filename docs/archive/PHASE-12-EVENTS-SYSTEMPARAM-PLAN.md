# Phase 12 — Events as SystemParam (`EventReader<E>` / `EventWriter<E>`)

> **Status**: Round 2 (post-critic, addresses C1+C2+C3+W1+W2+W3+O1+O2+all open questions).
> **Branch**: `ecs`.
> **Depends on**: Phase 6 EventDispatcher, Phase 8a SystemParam, Phase 8c IntoSystem/FunctionSystem, Phase 8d Commands::send_event, Phase 9 parallel scheduler + EVT1 TLS lane routing, Phase 10 Tick infrastructure.
> **Research input**: `docs/PHASE-12-RESEARCH.md`.

## §0 Round 2 changelog

Round 1 → Round 2 deltas. Every critic remark mapped to a concrete fix.

| Critic item | Severity | Resolution | Sections updated |
|---|---|---|---|
| **C1** Stacked-Borrows risk on cached `slot_ptr` (`&mut EventDispatcher` in `update_events` invalidates re-borrowed `NonNull<EventTypeSlot>`) | Critical | **Adopted Option (b)**: cache `NonNull<EventBuffer<E>>` directly (bypass slot on hot path). Buffer is heap-pinned in its own `Box`; its address is stable independent of dispatcher reborrows. Also cache `thread_count: u32` in State (immutable post-preregister). Slot is touched only at `init_state`; hot path never reborrows the slot. | §2.1, §2.2, §2.3 (EXT4 → `buffer_ptr<E>`), §4.3, §4.4, §5.1, §5.2, §6.1, §6.2, §6.4, §10.1, §10.2, §13.4 |
| **C2** Dead `frame_event_count.Acquire` load in `read()` | Critical | Removed from §6.2 `read()`. Slice math relies solely on `cursor`, `start_event_count`, `reader_len`. `is_empty()` keeps the `frame_event_count.Acquire` load (it is the only thing it consults). Updated §10.2 cost table. Added explicit clamp documentation: "reader observes only post-swap events; in-flight writes are invisible by ER5". | §2.2 (ER2 reword, ER9 reword), §6.2, §10.2, §10.3 |
| **C3** False sharing on `EventBuffer<E>` head between send-path `frame_event_count` and read-path `reader_buf`/`reader_len` | Critical | Pinned `EventBuffer<E>` layout explicitly with `#[repr(C)]` and `CachePadded<AtomicU64>` from `crossbeam_utils` (already a transitive dep via `boyko_threadpool` and `boyko_ecs/Cargo.toml`). Send-hot `frame_event_count` gets its own 64 B cache line; read-hot fields (`start_event_count`, `reader_len`, `reader_buf` header, `capacity`, `thread_count`) share one line; `lanes` Box header lives on yet another line. Added compile-time offset asserts. | §2.3 (EXT3, EXT7), §4.2, §4.4, §7.1, §7.2, §7.3, §10.5, §11.1, §13.1 |
| **W1** Per-call `slot.thread_count` lookup | Important | Solved by C1's `thread_count: u32` field in `EventWriterState<E>` / `EventReaderState<E>`. Per-call hot path now: `state.buffer_ptr.as_ref()` → `current_worker_id_or_dispatcher_lane(state.thread_count - 1)` → `buf.send_one(lane, event)`. No slot deref. Documented per-call lane resolution stays (worker migration safety — Phase 9 may reassign systems across frames). | §2.1 (EW2 reword + new EW8), §5.1, §6.1, §10.1 |
| **W2** Option A — unattached-thread/worker-0 lane collision | Important | New invariant **ER-NEW** in §2.1: `EventWriter::send` and `EventReader::read` `debug_assert!(boyko_threadpool::is_in_system_run())` to catch out-of-scheduler use. Main-thread tests must use `world.events().send_event(...)`. Documented in §9.3. The lane-0 collision is now diagnosed in debug, not silently UB-free-by-luck. | §2.1 (EW-NEW), §2.2 (ER-NEW), §9.2, §9.3, §13.1 |
| **W3** Test event-id range collision | Important | §13.0 reserves **EventId range 100-119** for Phase 12. All new tests use `#[event]` derive (not hardcoded ids) and the existing Phase 6 `register_event_new` helper allocates from the reserved pool. Documented in `docs/SYSTEMS.md` patch (§15). | §13.0, §13.1, §13.2, §13.4 |
| **O1** Drop `EventReaderState::_pad: u64` | Optional | Dropped. `EventReaderState<E>` shrinks from 32 B to 24 B (one half cache line); padding/align governed by `#[repr(C, align(8))]`. §11.1 size table updated. | §5.2, §11.1 |
| **O2** Wrong size claim for `EventReader<'s, 'w, E>` | Optional | Fixed: 8 B (single `&'s mut State`). §11.1 row corrected. | §11.1 |
| **OQ1** `'w` lifetime in `EventReader` | Open | **Dropped**. `'w` was not constraining anything (the read slice derives its lifetime from `'s` via the state-borrow chain). New shape: `EventReader<'s, E>` only. Updated signature, SystemParam impl, and tests. | §2.2 (ER1, ER2), §6.2 declaration, §6.4 impl, §11.1, §12 examples, §13.1 |
| **OQ2** `is_empty` vs `len` consistency under concurrent send | Open | Both clamp to `reader_len` consistently. `is_empty` now: `cursor.saturating_sub(start_event_count) >= reader_len`. `len` keeps the same formula. Race-free per ER5 (no in-flight swap inside the worker window). Documented. | §2.2 (ER2, ER9), §6.2 |
| **OQ3** `slot_ptr` Option vs panic | Open | Kept `Option<NonNull<EventBuffer<E>>>` at the `pub(crate)` boundary. Panic happens in `init_state` via the cold helper `event_not_preregistered_panic`. Cleaner: `buffer_ptr` is also useful for diagnostic tooling that wants `None` instead of a panic. | §4.3 |
| **OQ4** `&mut self` vs `&self` on `EventWriter::send` | Open | Kept `&mut self`. Matches Bevy's `EventWriter::send` and our `ResMut`/`Commands` writer-pattern. Mechanically the state mutation is "could-mutate" (writer carries `&'s mut State`); semantically signals "this is the producer side". | §6.1 |

**Plan-length impact**: +90 lines net (was 1418, projected 1500-1700). Net growth comes from §0 itself, the explicit `EventBuffer<E>` layout spec, the `CachePadded` integration notes, and three new invariants (EW-NEW, ER-NEW, EW8).

---

## §1 Summary, targets, scope

### 1.1 Goal

Add typed `EventReader<'s, E>` and `EventWriter<'s, E>` SystemParams over Phase 6's `EventDispatcher`. Eliminate the `OnceLock<EventId>` acquire-load and the `slots[id]` bounds-mask check on the per-call hot path by caching `NonNull<EventBuffer<E>>` and `thread_count: u32` in the SystemParam `State`. Preserve Phase 9's per-lane parallelism advantage (Option A — events outside the `Access` conflict graph).

### 1.2 Target metrics

| Operation | Target | Phase 6 baseline | Source of saving |
|---|---|---|---|
| `EventWriter::send` (single event, cached state) | ≤ 5 ns | ~10 ns (`OnceLock` + bounds + mask + slot deref + buffer cast) | Cached `*mut EventBuffer<E>` + cached `thread_count` |
| `EventReader::read` empty-case (cursor caught up) | ≤ 3 ns | n/a | One `start_event_count.Acquire` + one `reader_len.Acquire` + 32-bit compare |
| `EventReader::read` per-element | ≤ 2 ns | n/a | Slice deref + cursor `+1`, no atomic in inner loop |
| `EventWriter::send_many` per-event amortised | ≤ 1.5 ns | ~1.5 ns | Phase 6 bulk path |
| `EventReader::init_state` per `(system, E)` | ≤ 10 ns | n/a | `E::event_id()` `OnceLock` (paid once) |
| 10k events/frame send+read end-to-end | ≤ 50 µs | ~75 µs | `send_many` batching |

L1d footprint of the hot send loop: `EventWriterState<E>` = 24 B (cached buffer_ptr + thread_count + event_id), `EventBuffer<E>` head cache-line for `frame_event_count` (its own 64 B line via `CachePadded`), per-lane writer line (Phase 6 layout — disjoint from buffer head). Total touch per send: ~3 cache lines (state, frame_event_count line, lane writer line).

### 1.3 Scope (IN)

1. `EventWriter<'s, E>` SystemParam — `init_state` / `init_access` (zero declared access) / `get_param` returning a transparent wrapper around the cached buffer pointer.
2. `EventReader<'s, E>` SystemParam — same shape with `last_event_count: u64` cursor inlined.
3. `EventDispatcher` extension: `pub(crate) fn buffer_ptr<E>(&self) -> Option<NonNull<EventBuffer<E>>>` accessor used by `init_state` (replaces the slot-pointer accessor of Round 1).
4. `EventBuffer<E>` extension: `frame_event_count: CachePadded<AtomicU64>` (own cache line) + `start_event_count: AtomicU64` (co-located with `reader_len`). Layout explicitly pinned.
5. `EventBuffer::send_one` / `send_many` patch: `fetch_add` on `frame_event_count` Relaxed.
6. `EventReader::read()` returning `EventIter<'_, E>`.
7. `EventReader::is_empty()` / `len()` / `missed_events()` / `clear()` helpers.
8. `EventWriter::send` / `send_many` / `send_default` (when `E: Default`).
9. Coexistence with Phase 11 `Commands::send_event` and Phase 6/9 raw `EcsMaster::events()` / `send_event::<E>`.

### 1.4 Scope (OUT)

1. `Local<T>` separate SystemParam (deferred — Phase 12 Q5).
2. `EventMutator<E>` (Bevy 0.16 mutable read).
3. Push-based observers (flecs-style).
4. Structural event types (`OnAdd<T>`).
5. Multi-frame missed-event retention beyond one swap.

## §2 Invariants

### 2.1 EventWriter (EW)

- **EW1** — `EventWriterState<E>` is built at `SystemParam::init_state` carrying cached `event_id: EventId`, `buffer_ptr: NonNull<EventBuffer<E>>`, `thread_count: u32`. After construction all fields are read-only.
- **EW2** — `buffer_ptr` is derived from `EventDispatcher::buffer_ptr::<E>()`, which returns the address of a `Box<EventBuffer<E>>` heap-allocated at `preregister`. The box's address is stable independent of `&mut EventDispatcher` re-borrows. The cached `NonNull` is therefore Stacked-Borrows / Tree-Borrows clean across `update_events` (C1 resolution).
- **EW3** — `EventWriter::send(&mut self, event)` performs **zero** OnceLock acquire, **zero** slot bounds check, **zero** mask check on the hot path. Operations: load `state.buffer_ptr` → deref → `current_worker_id_or_dispatcher_lane(state.thread_count - 1)` → `EventBuffer::send_one`. The slot is not touched after `init_state`.
- **EW4** — Lane routing reuses Phase 9 EVT1: `current_worker_id_or_dispatcher_lane(thread_count - 1)`. Worker → own lane; dispatcher thread → reserved lane `thread_count - 1`; unattached thread → lane 0 (gated by EW-NEW in debug).
- **EW5** — `EventWriter::send` declares NO `Access`. Two `EventWriter<E>` systems for the same `E` are parallel-safe by per-lane EVT1 discipline.
- **EW6** — Returns `Result<(), EcsError>` on overflow (Phase 6 parity).
- **EW7** — Cannot send for unregistered `E`: `init_state` panics via cold helper. Hot path has no runtime check.
- **EW8 (new)** — `state.thread_count` is captured at `init_state` and frozen. Phase 6 SEND4 + the one-shot preregistration discipline guarantee `EventBuffer<E>::thread_count` never changes post-creation. If a future feature requires dynamic resize, it must invalidate every cached State via a generation counter — flagged as a future-phase concern in §16.
- **EW-NEW (W2 resolution)** — `EventWriter::send` `debug_assert!(boyko_threadpool::is_in_system_run())`. Out-of-scheduler use (main-thread tests, FFI callbacks) must use `EcsMaster::events().send_event::<E>(event)` directly. Documented in §9.3. In release this is a no-op (no production overhead).

### 2.2 EventReader (ER)

- **ER1** — `EventReaderState<E>` carries `event_id`, `buffer_ptr: NonNull<EventBuffer<E>>`, `thread_count: u32`, and `last_event_count: u64`. 24 B total (O1 — dropped reserved pad).
- **ER2** — `EventReader::read()` returns `EventIter<'_, E>` over `reader_buf[start_offset..start_offset + visible_len]`, where:
  - `start_count = buf.start_event_count.load(Acquire)`
  - `reader_len = buf.reader_len.load(Acquire) as u64`
  - `start_offset = cursor.saturating_sub(start_count) as usize`
  - `visible_len = reader_len.saturating_sub(start_offset as u64) as usize`
  - **No `frame_event_count` load in `read()`** (C2 resolution). Only `start_event_count` and `reader_len` are consulted.
- **ER3** — Iterator advances `cursor += consumed` on drop. `break`-mid-iteration leaves cursor at `start_count + consumed_so_far`. Cursor is checkpointed via the back-pointer `&'s mut last_event_count` in `EventIter`.
- **ER4** — `read()` runs concurrently with: any other `EventReader<E>` / `EventReader<F>` / `EventWriter<F>` (F≠E) / `EventWriter<E>` (same E — disjoint allocations: writer → write_buf; reader → reader_buf).
- **ER5** — `read()` MUST NOT run concurrently with `update_events`. Enforcement via Phase 9 frame-boundary call.
- **ER6** — Cursor `u64` rolls over after ~1.8e19 events → never in practice.
- **ER7** — Missed-events: if `cursor < start_count`, `start_offset = 0`, missed count = `start_count - cursor`. Cursor forwards past the gap on next iteration.
- **ER8** — Declares NO `Access` (Option A).
- **ER9** — `is_empty()` short-circuits via `cursor.saturating_sub(start_count) >= reader_len`, both loaded `Acquire`. **No `frame_event_count` load** (consistency with `read()` — OQ2 resolution). Returns true when there are no visible (post-swap) unread events.
- **ER-NEW (W2 resolution)** — `EventReader::read` `debug_assert!(boyko_threadpool::is_in_system_run())`. Same rationale as EW-NEW.

### 2.3 EventDispatcher extension (EXT)

- **EXT1** — `EventBuffer<E>` adds `frame_event_count: CachePadded<AtomicU64>` (its own 64 B cache line — C3 resolution). Initialised at `EventBuffer::new`. Bumped `Relaxed` on every successful send (one fetch_add per send, or one fetch_add(n) per `send_many`). Never reset across frames.
- **EXT2** — `frame_event_count` is process-lifetime monotonic.
- **EXT3** — `EventBuffer<E>` adds `start_event_count: AtomicU64` co-located with `reader_len` (both Release-stored at the swap, both Acquire-loaded by readers — they belong on the same cache line for reader-side prefetch). Updated by `swap_and_flatten` after the per-lane copy: `start_event_count.store(frame_event_count - cursor, Release)`.
- **EXT4** — Buffer pointer accessor: `pub(crate) fn buffer_ptr<E: Event>(&self) -> Option<NonNull<EventBuffer<E>>>`. Returns `None` if `E` not registered. Pointer is stable for dispatcher lifetime (the box is owned by the dispatcher's `Box<[EventTypeSlotStorage; MAX_EVENTS]>` indirection → the heap address inside `slot.data` is independent of any `&mut EventDispatcher` reborrow). This is the key SB/TB-clean cache (C1 resolution).
- **EXT5** — Buffer pointer lifetime: `EventDispatcher` is owned by `EcsMaster`, the `Box<EventBuffer<E>>` is owned by the dispatcher's slot. Lifetimes: heap allocation outlives every `System`. **Stacked-Borrows reasoning**: the cached `NonNull<EventBuffer<E>>` carries provenance from `Box::into_raw(boxed_buffer)`. The dispatcher's `&mut self` in `update_events` operates on `EventDispatcher`'s fields (slot storage), not on `EventBuffer<E>` itself — so `&mut EventDispatcher` does NOT pop a borrow that would invalidate the buffer pointer. `update_events` reaches `EventBuffer<E>` via the type-erased `slot.data` raw pointer, which is provenance-equivalent to our cached pointer (both derive from the same `Box::into_raw`). **Miri-validatable**.
- **EXT6** — `frame_event_count.fetch_add` happens AFTER the `write_len.store(Release)` that publishes the event bytes — preserves the happens-before chain: reader sees new `start_event_count` (post-swap Release) ⇒ reader sees the corresponding event in `reader_buf`.
- **EXT7** — `EventTypeSlot` is **unchanged** vs Phase 6 — no field added (C1+C3 final placement keeps both counters on `EventBuffer<E>`).
- **EXT8 (C3 layout pin)** — `EventBuffer<E>` is explicitly laid out via `#[repr(C)]` with field order specified so `frame_event_count` does NOT share a cache line with `reader_buf` / `reader_len` / `start_event_count` / `lanes`. Compile-time offset asserts (§4.4) enforce this.

### 2.4 Send/Sync (SEND-EV)

- **SEND-EV1** — `EventWriterState<E>: Send + Sync + 'static`. Manual `unsafe impl` for the `NonNull<EventBuffer<E>>`. SAFETY: `EventBuffer<E>` is `Sync` per Phase 6 SEND4 (per-lane writer exclusivity + `&mut self` swap barrier). Pointer-aliasing-wise, sending the cached pointer across threads is safe because the heap address is stable.
- **SEND-EV2** — `EventReaderState<E>: Send + Sync + 'static`. Same as SEND-EV1.
- **SEND-EV3** — `EventWriter<'s, E>` / `EventReader<'s, E>` per-call wrappers: NOT `Send`/`Sync` (carry `&'s mut State`, mirror `Commands<'s>`).

## §3 Decision matrix (Q1–Q6)

### Q1. Cursor design

**Decision**: Bevy-style cursor (per-type `AtomicU64`, per-system `u64`). Unchanged from Round 1.

**Rationale**: Frame-only breaks "every event seen exactly once" for sub-schedules. Tick-keyed bloats per-event footprint. Bevy cursor: +1 Relaxed atomic per send, +24 B per `EventReaderState`.

### Q2. Conflict graph — Option A

**Decision**: Option A (events outside `Access`). Unchanged.

**Rationale**: Phase 9 EVT1 makes parallel writers UB-free; Option B would grow `Access` by 32 B + 32 B (event_read + event_write bitsets) and lose parallelism with no correctness benefit. EW-NEW / ER-NEW now diagnose out-of-scheduler use in debug (W2 fix).

### Q3. Buffer pointer caching (replaces "slot pointer caching")

**Decision**: Cache `NonNull<EventBuffer<E>>` (NOT `NonNull<EventTypeSlot>`) plus `thread_count: u32` in both states. **Changed from Round 1** per C1.

**Rationale**:
- Round 1 cached `NonNull<EventTypeSlot>` derived from `&EventTypeSlot`. Between `init_state` and `get_param`, `update_events` takes `&mut EventDispatcher` — under Stacked Borrows this pops every shared borrow derived from the parent `&EventDispatcher`, invalidating our cached pointer's provenance. The "audited; ~0 cycles" defense was insufficient. Miri test `miri_slot_ptr_provenance` would fail.
- Round 2 caches `NonNull<EventBuffer<E>>` derived from `Box::into_raw(boxed_buffer)` (the dispatcher's `preregister` path). The buffer's heap address is independent of `EventDispatcher` reborrows. `update_events` reaches the buffer via `slot.data` raw pointer with the same provenance — both pointers share the original allocation's tag. No SB violation.
- Caching `thread_count: u32` alongside removes the per-call `slot.thread_count` load (W1 fix). Lane routing becomes `current_worker_id_or_dispatcher_lane(state.thread_count - 1)`.

**Cost**: 24 B per state (vs 32 B in Round 1 — pad dropped per O1).

**Trade-off**: If a future feature requires `EventBuffer<E>` to be rebuilt (capacity change, reallocation), every cached State becomes stale. **Mitigation**: documented in EW8 as a future-phase concern; preregister is one-shot today.

### Q4. Migration of existing API

**Decision**: Both paths coexist. Unchanged.

### Q5. `Local<T>` SystemParam

**Decision**: No. Inline `last_event_count: u64` into `EventReaderState`. Unchanged.

### Q6. `EventWriter::send_many`

**Decision**: Expose. Forwards to `EventBuffer::send_many`. One `fetch_add(n, Relaxed)` per batch.

## §4 EventDispatcher changes

### 4.1 Counter placement (revised per C1+C3)

Both counters live on **`EventBuffer<E>`**, not on `EventTypeSlot`. The slot is touched only at `init_state` (one-time cost, paid via `buffer_ptr::<E>()`). Per-call hot path operates directly on the cached `NonNull<EventBuffer<E>>`.

`EventTypeSlot` is **unchanged** vs Phase 6. The slot still owns the type-erased `data: *mut u8` (which equals `Box::into_raw(buffer).cast::<u8>()`), the vtable, capacity, and thread_count. We do not modify it.

### 4.2 `EventBuffer<E>` layout (C3 resolution — explicit `#[repr(C)]` + `CachePadded`)

```rust
use crossbeam_utils::CachePadded;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use core::mem::MaybeUninit;
use core::marker::PhantomData;

/// Per-event-type buffer with explicit layout for cache-line discipline.
///
/// Field grouping (Phase 12 EXT8):
/// - Cache line 0: `frame_event_count` (send-path hot, written on every send).
///   Wrapped in `CachePadded` so it does NOT share a line with any reader-side
///   field. Single-writer contention is unavoidable (lanes write the same
///   counter), but reader-side cache lines stay clean.
/// - Cache line 1: read-path hot fields shared by all readers between swaps.
///   `reader_buf` (Box header, 16 B), `reader_len` (4 B), `start_event_count`
///   (8 B), `capacity_per_lane` (4 B), `thread_count` (4 B). Total <= 40 B
///   plus PhantomData ZST.
/// - Cache line 2+: per-thread `lanes: Box<[ThreadLanePair<E>]>` (16 B Box
///   header). Each `ThreadLanePair<E>` is 128 B (2 cache lines, see
///   ThreadLane{Writer,Reader} layout in event_buffer.rs:42-47).
#[repr(C)]
pub(crate) struct EventBuffer<E: Event> {
    // ── Cache line 0: send-path hot ──────────────────────────────────────────
    /// Monotonic per-type send counter. `Relaxed::fetch_add` on every send.
    /// Wrapped in `CachePadded` (64 B on x86_64) to isolate write traffic
    /// from reader-side fields.
    pub(crate) frame_event_count: CachePadded<AtomicU64>,

    // ── Cache line 1: read-path hot ──────────────────────────────────────────
    /// Snapshot of `frame_event_count` at the moment of the last swap.
    /// Release-stored by `swap_and_flatten`; Acquire-loaded by `read()`.
    pub(crate) start_event_count: AtomicU64,
    /// Number of initialised elements in `reader_buf`. Release-stored by
    /// `swap_and_flatten`; Acquire-loaded by `read()`.
    pub(crate) reader_len: AtomicU32,
    /// Mirror of EventConfig::capacity_per_lane.
    pub(crate) capacity_per_lane: u32,
    /// Mirror of EventConfig::thread_count.
    pub(crate) thread_count: u32,
    /// Padding for alignment of the next field (Box header is 16 B aligned to 8).
    _pad_line1: u32,
    /// Flat read buffer. Header is 16 B; contents live elsewhere in the heap.
    /// The header sits on cache line 1; the contents (events) are accessed
    /// sequentially by readers and prefetched by hardware.
    pub(crate) reader_buf: Box<[MaybeUninit<E>]>,

    // ── Cache line 2: per-thread lanes ──────────────────────────────────────
    /// Per-thread write lanes. Header 16 B; each lane is its own
    /// `ThreadLanePair<E>` (128 B aligned 64) in the heap.
    pub(crate) lanes: Box<[ThreadLanePair<E>]>,
    _marker: PhantomData<E>,
}

// ── Layout asserts (C3) ──────────────────────────────────────────────────────
// frame_event_count must sit before everything else; CachePadded ensures
// the next field starts on a fresh cache line.
const _: () = assert!(core::mem::offset_of!(EventBuffer<LayoutAssertEvent>, frame_event_count) == 0);
const _: () = assert!(core::mem::offset_of!(EventBuffer<LayoutAssertEvent>, start_event_count) >= 64,
    "start_event_count must live on a different cache line than frame_event_count");
const _: () = assert!(
    core::mem::offset_of!(EventBuffer<LayoutAssertEvent>, reader_len)
        - core::mem::offset_of!(EventBuffer<LayoutAssertEvent>, start_event_count)
        < 64,
    "start_event_count and reader_len must share a cache line (both swap-Release)"
);
```

`CachePadded<AtomicU64>` from `crossbeam_utils` is already a workspace dependency (`crates/boyko_ecs/Cargo.toml:12: crossbeam-utils = "0.8"`). The struct adds padding to make the wrapped type occupy at least one full cache line. Final `size_of::<CachePadded<AtomicU64>>() == 64` on x86_64.

Implementation note: `core::mem::offset_of!` is stable since Rust 1.77. boyko is on Rust 2024 — available.

### 4.3 Buffer pointer accessor (EXT4 — replaces Round 1 `slot_ptr`)

```rust
impl EventDispatcher {
    /// Returns a stable pointer to the heap-allocated `EventBuffer<E>` for `E`.
    ///
    /// The returned pointer is provenance-equivalent to the one stored inside
    /// `EventTypeSlot::data` (both derive from `Box::into_raw(boxed_buffer)`
    /// at preregister time). Callers may cache the pointer in long-lived
    /// state (e.g. `SystemParam::State`); it remains valid for the dispatcher's
    /// lifetime.
    ///
    /// Returns `None` if `E` was not preregistered.
    #[inline]
    pub(crate) fn buffer_ptr<E: Event>(&self) -> Option<NonNull<EventBuffer<E>>> {
        let id = E::event_id() as usize;
        if id >= MAX_EVENTS || !self.registered_mask.get(id) {
            return None;
        }
        // SAFETY: registered_mask bit set ⇒ slot is initialised (U3).
        let slot: &EventTypeSlot = unsafe { self.slots[id].slot.assume_init_ref() };
        // SAFETY: slot.vtable.type_id == TypeId::of::<E>() (checked at preregister);
        //   slot.data points at `Box<EventBuffer<E>>` from preregister.
        debug_assert_eq!(slot.vtable.type_id, core::any::TypeId::of::<E>());
        let buf_ptr: *mut EventBuffer<E> = slot.data as *mut EventBuffer<E>;
        // SAFETY: slot.data is a non-null pointer to the heap-allocated buffer.
        Some(unsafe { NonNull::new_unchecked(buf_ptr) })
    }
}
```

**Why this is SB/TB-clean** (C1 resolution): The pointer is derived from `slot.data`, which is a raw `*mut u8` populated by `Box::into_raw(boxed_buffer)` at preregister. Subsequent `&mut EventDispatcher` operations (e.g. `update_events`) borrow the dispatcher struct, but the buffer lives in its own heap allocation — the `&mut EventDispatcher` borrow does NOT cover the buffer's memory. The cached `NonNull<EventBuffer<E>>` retains the original allocation's tag from `Box::into_raw`. `update_events` reaches the buffer via the same raw pointer chain, so all reads/writes through the dispatcher pass through `slot.data as *mut EventBuffer<E>` with the same provenance — no SB violation.

### 4.4 `EventBuffer::new` initialisation

```rust
impl<E: Event> EventBuffer<E> {
    pub(crate) fn new(cfg: EventConfig) -> EcsResult<Box<Self>> {
        // ... existing lane + reader_buf allocation ...
        let buffer = Box::new(EventBuffer {
            frame_event_count: CachePadded::new(AtomicU64::new(0)),
            start_event_count: AtomicU64::new(0),
            reader_len: AtomicU32::new(0),
            capacity_per_lane: cfg.capacity_per_lane,
            thread_count: cfg.thread_count,
            _pad_line1: 0,
            reader_buf,
            lanes,
            _marker: PhantomData,
        });
        Ok(buffer)
    }
}
```

### 4.5 Backward compatibility

Public surface unchanged. Existing tests pass — counter bumps are internal. `EventTypeSlot` layout unchanged → existing 64 B / align(64) asserts retained.

## §5 EventReaderState / EventWriterState

### 5.1 `EventWriterState<E>` (24 B)

```rust
/// Per-system state for `EventWriter<E>`.
///
/// 24 B total. Fits in 3/8 of a cache line (typically co-located with other
/// states in the system's tuple of params).
///
/// Field order (Round 2 — C1+W1 resolution):
/// 1. `buffer_ptr` — cached `NonNull<EventBuffer<E>>` from EventDispatcher::buffer_ptr.
///    Stable for dispatcher lifetime (EXT4, EXT5).
/// 2. `thread_count` — cached from `EventBuffer<E>::thread_count` at init.
///    Immutable post-preregister (EW8). Used in lane routing per call.
/// 3. `event_id` — cached EventId (8 B). Diagnostic only on hot path.
#[repr(C)]
pub struct EventWriterState<E: Event> {
    /// Heap-stable buffer pointer. See SAFETY comment on Send/Sync impl.
    buffer_ptr: NonNull<EventBuffer<E>>,    // 8 B
    /// `EventBuffer<E>::thread_count` cached at init. Drives lane routing.
    thread_count: u32,                       // 4 B
    /// Padding to 8-byte boundary for the u64 below.
    _pad: u32,                               // 4 B
    /// Cached EventId. Read once at init; kept for diagnostics / future use.
    event_id: EventId,                       // 8 B (u64)
    /// Type binding without forcing `E: Send + Sync` on the state.
    _marker: PhantomData<fn(&E)>,            // 0 B
}

// SAFETY (SEND-EV1):
//   - NonNull<EventBuffer<E>>: the pointee is `Sync` per Phase 6 SEND4
//     (per-lane writer exclusivity + `&mut self` swap barrier). The raw
//     pointer itself crossing thread boundaries when the scheduler migrates
//     the system is safe because the heap address is stable and provenance
//     is preserved (derived from `Box::into_raw` at preregister).
//   - thread_count, event_id: u32 / u64; Send + Sync trivially.
//   - PhantomData<fn(&E)> is Send + Sync regardless of E.
unsafe impl<E: Event> Send for EventWriterState<E> {}
unsafe impl<E: Event> Sync for EventWriterState<E> {}

const _: () = assert!(core::mem::size_of::<EventWriterState<LayoutAssertEvent>>() == 24);
const _: () = assert!(core::mem::align_of::<EventWriterState<LayoutAssertEvent>>() == 8);
```

### 5.2 `EventReaderState<E>` (24 B — O1 pad dropped)

```rust
/// Per-system state for `EventReader<E>`.
///
/// 24 B total. Round 2 O1 fix: dropped Round 1's `_pad: u64` (no concrete
/// consumer; reintroduce later if a per-reader flag emerges).
#[repr(C)]
pub struct EventReaderState<E: Event> {
    /// Heap-stable buffer pointer.
    buffer_ptr: NonNull<EventBuffer<E>>,    // 8 B
    /// Per-(system, E) cursor. Persists across frames; never reset.
    last_event_count: u64,                   // 8 B
    /// Cached at init from EventBuffer<E>::thread_count.
    /// Used by future read-time facilities (currently unused on read path,
    /// but kept to keep state shape symmetric with EventWriterState and
    /// because read-side iteration may eventually need per-thread routing
    /// — e.g. read-from-own-lane debugging).
    thread_count: u32,                       // 4 B
    /// Padding to 8-byte boundary.
    _pad: u32,                               // 4 B
    /// Type binding.
    _marker: PhantomData<fn(&E)>,            // 0 B
}

// SAFETY (SEND-EV2): same as EventWriterState.
unsafe impl<E: Event> Send for EventReaderState<E> {}
unsafe impl<E: Event> Sync for EventReaderState<E> {}

const _: () = assert!(core::mem::size_of::<EventReaderState<LayoutAssertEvent>>() == 24);
const _: () = assert!(core::mem::align_of::<EventReaderState<LayoutAssertEvent>>() == 8);
```

**Note on `thread_count` in `EventReaderState`**: kept symmetric with `EventWriterState` even though `read()` does not use it. Rationale: future read-time facilities (per-thread snapshot for diagnostics, read-from-own-lane mode) may need it; the 4 B cost is already absorbed by the `u32` slot in the 24 B layout. If proven unused after 6 months in production, drop it and the state shrinks to 16 B.

`last_event_count` init: 0 (Bevy parity — late readers see historical events).

## §6 EventReader / EventWriter SystemParam impls

### 6.1 `EventWriter<'s, E>` wrapper

```rust
#[repr(transparent)]
pub struct EventWriter<'s, E: Event> {
    state: &'s mut EventWriterState<E>,
}

impl<'s, E: Event> EventWriter<'s, E> {
    /// Sends a single event to the per-thread lane (EVT1 routing).
    ///
    /// Hot path: ~5 ns (cached buffer pointer + cached thread_count +
    /// TLS lane resolve + per-lane atomic). Returns `Err(EventBufferFull)`
    /// on lane overflow.
    #[inline]
    pub fn send(&mut self, event: E) -> EcsResult<()> {
        // EW-NEW: out-of-scheduler use is debug-asserted.
        debug_assert!(boyko_threadpool::is_in_system_run(),
            "EventWriter::send called outside a scheduled system. \
             Main-thread / FFI callers must use EcsMaster::events().send_event::<E>(...).");

        // SAFETY (EW2, EXT5, C1 resolution):
        //   - buffer_ptr is heap-stable, provenance from Box::into_raw at preregister.
        //   - The pointee is `Sync` per SEND4; `&EventBuffer<E>` borrow is sound
        //     concurrently with per-lane writes (each writes a distinct lane —
        //     EVT1) and concurrent reader iteration of reader_buf (disjoint).
        //   - `&mut EventDispatcher` in update_events is NOT in flight (ER5/SCH7).
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };

        let lane = boyko_threadpool::current_worker_id_or_dispatcher_lane(
            self.state.thread_count.saturating_sub(1)
        );

        buf.send_one(lane, event)
        // EXT6: send_one bumps frame_event_count internally on success (Relaxed).
    }

    /// Bulk send — all-or-nothing (Phase 6 semantics).
    #[inline]
    pub fn send_many<I: ExactSizeIterator<Item = E>>(&mut self, iter: I) -> EcsResult<()> {
        debug_assert!(boyko_threadpool::is_in_system_run());
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        let lane = boyko_threadpool::current_worker_id_or_dispatcher_lane(
            self.state.thread_count.saturating_sub(1)
        );
        buf.send_many(lane, iter)
    }

    #[inline]
    pub fn send_default(&mut self) -> EcsResult<()> where E: Default {
        self.send(E::default())
    }
}

const _: () = assert!(core::mem::size_of::<EventWriter<'_, LayoutAssertEvent>>() == 8);
```

### 6.2 `EventReader<'s, E>` wrapper (OQ1 — dropped `'w`)

```rust
/// Typed event-read wrapper for one system invocation.
///
/// Lifetime `'s`: state scope. The Round 1 `'w` lifetime was dropped (OQ1
/// resolution) — the returned iterator's borrow lifetime is bound through
/// the `&'s mut last_event_count` cursor back-pointer, not via a separate
/// world lifetime.
pub struct EventReader<'s, E: Event> {
    state: &'s mut EventReaderState<E>,
}

impl<'s, E: Event> EventReader<'s, E> {
    /// Returns an iterator over unread events.
    ///
    /// On drop, the cursor advances by the number of events yielded. Mid-
    /// iteration `break` is supported.
    ///
    /// Cost (hot path):
    /// - Empty case: ~3 ns (one Acquire-load on `start_event_count` + one
    ///   on `reader_len` + 64-bit compare).
    /// - Per element: ~2 ns (`get_unchecked` + cursor +1).
    ///
    /// **C2 resolution**: `frame_event_count` is NOT loaded here. Slice math
    /// depends only on `cursor`, `start_event_count`, `reader_len`. Readers
    /// observe only post-swap events; in-flight writes are invisible by ER5.
    #[inline]
    pub fn read(&mut self) -> EventIter<'_, E> {
        // ER-NEW: scheduled-context check (debug only).
        debug_assert!(boyko_threadpool::is_in_system_run());

        // SAFETY (ER2, C1): buffer_ptr is heap-stable; the pointee is `Sync`;
        //   read-only access to reader_buf is sound concurrently with any
        //   number of writers (disjoint allocations) and other readers.
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };

        // ER2: two Acquire loads. Pair with the Release stores in swap_and_flatten.
        let start_count = buf.start_event_count.load(Ordering::Acquire);
        let reader_len = buf.reader_len.load(Ordering::Acquire) as u64;

        let cursor = self.state.last_event_count;
        let (start_offset, missed) = if cursor < start_count {
            // ER7: reader missed at least one frame's events.
            (0u64, start_count - cursor)
        } else {
            (cursor - start_count, 0)
        };

        // Clamp to reader_len.
        let visible_len = reader_len.saturating_sub(start_offset);

        // SAFETY: start_offset + visible_len <= reader_len (clamped above);
        //   reader_buf[..reader_len] is initialised (Phase 6 U11).
        let slice: &[E] = unsafe {
            let ptr: *const E = (*buf.reader_buf).as_ptr().cast::<E>();
            core::slice::from_raw_parts(
                ptr.add(start_offset as usize),
                visible_len as usize,
            )
        };

        EventIter {
            slice,
            consumed: 0,
            cursor_state: &mut self.state.last_event_count,
            start_count,
            missed,
        }
    }

    /// `true` if there are no unread post-swap events.
    ///
    /// **OQ2 resolution**: consistent with `len()` — both clamp to `reader_len`.
    /// Does NOT load `frame_event_count` (would over-report under in-flight
    /// writes that ER5 already excludes).
    #[inline]
    pub fn is_empty(&self) -> bool {
        debug_assert!(boyko_threadpool::is_in_system_run());
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        let start_count = buf.start_event_count.load(Ordering::Acquire);
        let reader_len = buf.reader_len.load(Ordering::Acquire) as u64;
        let cursor = self.state.last_event_count;
        let start_offset = cursor.saturating_sub(start_count);
        start_offset >= reader_len
    }

    /// Number of unread events. Clamps to reader_len (post-swap only).
    #[inline]
    pub fn len(&self) -> usize {
        debug_assert!(boyko_threadpool::is_in_system_run());
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        let start_count = buf.start_event_count.load(Ordering::Acquire);
        let reader_len = buf.reader_len.load(Ordering::Acquire) as u64;
        let cursor = self.state.last_event_count;
        let start_offset = cursor.saturating_sub(start_count);
        reader_len.saturating_sub(start_offset) as usize
    }

    /// Number of events the cursor skipped (ER7).
    #[inline]
    pub fn missed_events(&self) -> u64 {
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        let start_count = buf.start_event_count.load(Ordering::Acquire);
        start_count.saturating_sub(self.state.last_event_count)
    }

    /// Advances the cursor to the current `frame_event_count` without yielding.
    ///
    /// NOTE: this is the only place `frame_event_count` is consulted from the
    /// reader side. `clear()` is opt-in (not on the hot read path) — paying
    /// the extra Acquire load is acceptable.
    #[inline]
    pub fn clear(&mut self) {
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        self.state.last_event_count = buf.frame_event_count.load(Ordering::Acquire);
    }
}
```

### 6.3 `EventIter<'a, E>` — drop-finalised cursor

```rust
/// Iterator over unread events. Drop-finalised cursor checkpoint.
///
/// Lifetime `'a`: borrow of the dispatcher's reader_buf slice. Bound to
/// `'s` via the cursor back-pointer (the iterator must not outlive the
/// state borrow).
pub struct EventIter<'a, E: Event> {
    /// Borrowed slice into reader_buf.
    slice: &'a [E],
    /// Number of elements yielded so far.
    consumed: usize,
    /// Back-pointer to the state's cursor. Checkpointed on drop.
    cursor_state: &'a mut u64,
    /// start_event_count snapshot at read() time.
    start_count: u64,
    /// Missed events (ER7) — for user query via `EventReader::missed_events()`.
    missed: u64,
}

impl<'a, E: Event> Iterator for EventIter<'a, E> {
    type Item = &'a E;

    #[inline]
    fn next(&mut self) -> Option<&'a E> {
        if self.consumed >= self.slice.len() {
            return None;
        }
        // SAFETY: bounds checked above; slice elements initialised per ER2 + U11.
        let item = unsafe { self.slice.get_unchecked(self.consumed) };
        self.consumed += 1;
        Some(item)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.slice.len() - self.consumed;
        (remaining, Some(remaining))
    }
}

impl<'a, E: Event> ExactSizeIterator for EventIter<'a, E> {}

impl<'a, E: Event> Drop for EventIter<'a, E> {
    #[inline]
    fn drop(&mut self) {
        // ER3: cursor advances by `consumed`; missed events absorbed by
        // setting cursor to start_count + consumed (skipping the gap).
        *self.cursor_state = self.start_count + self.consumed as u64;
    }
}
```

### 6.4 SystemParam impls

```rust
// SAFETY (SP1, SP2, SP4, EW1-EW8):
unsafe impl<'a, E: Event> SystemParam for EventWriter<'a, E> {
    type State = EventWriterState<E>;
    type Item<'w, 's> = EventWriter<'s, E>;

    fn init_state(world: &mut EcsMaster, _meta: &mut SystemMeta) -> Self::State {
        let event_id = E::event_id();
        let buffer_ptr = world.events()
            .buffer_ptr::<E>()
            .unwrap_or_else(|| event_not_preregistered_panic::<E>());

        // Cache thread_count by reading the buffer once. After this point
        // the buffer is reached only via the cached pointer.
        // SAFETY: buffer_ptr is heap-stable (EXT5); reading thread_count is
        //   a plain field load on `&EventBuffer<E>`.
        let thread_count = unsafe { buffer_ptr.as_ref() }.thread_count;

        EventWriterState {
            buffer_ptr,
            thread_count,
            _pad: 0,
            event_id,
            _marker: PhantomData,
        }
    }

    fn init_access(
        _state: &Self::State,
        _meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // EW5 / Q2 Option A: no access declared. Per-lane EVT1 discipline
        // makes parallel writers of the same E sound. Out-of-scheduler use
        // is debug-asserted in send() via EW-NEW.
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _meta: &SystemMeta,
        _world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        EventWriter { state }
    }
}

// SAFETY (SP1, SP2, SP4, ER1-ER9 + ER-NEW):
unsafe impl<'a, E: Event> SystemParam for EventReader<'a, E> {
    type State = EventReaderState<E>;
    type Item<'w, 's> = EventReader<'s, E>;

    fn init_state(world: &mut EcsMaster, _meta: &mut SystemMeta) -> Self::State {
        let buffer_ptr = world.events()
            .buffer_ptr::<E>()
            .unwrap_or_else(|| event_not_preregistered_panic::<E>());
        let thread_count = unsafe { buffer_ptr.as_ref() }.thread_count;
        EventReaderState {
            buffer_ptr,
            last_event_count: 0, // see-historical-events (Bevy parity)
            thread_count,
            _pad: 0,
            _marker: PhantomData,
        }
    }

    fn init_access(
        _state: &Self::State,
        _meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // ER8 / Q2 Option A: no access declared.
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _meta: &SystemMeta,
        _world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        EventReader { state }
    }
}
```

### 6.5 Diagnostic helper

```rust
#[cold]
#[inline(never)]
pub(crate) fn event_not_preregistered_panic<E: Event>() -> ! {
    panic!(
        "Event type {} (id={}) was not preregistered on this EcsMaster. \
         Call `world.preregister_event::<{0}>(EventConfig::default_for(N))` \
         during world setup before adding systems that use \
         EventReader<{0}> / EventWriter<{0}>.",
        core::any::type_name::<E>(),
        E::event_id(),
    );
}
```

## §7 EventDispatcher::send refactor

### 7.1 Patch `EventBuffer::send_one` (EXT6)

```rust
#[inline]
pub(crate) fn send_one(&self, thread_index: u32, event: E) -> EcsResult<()> {
    debug_assert!(thread_index < self.thread_count);
    let lane = &self.lanes[thread_index as usize].writer;
    let len = lane.write_len.load(Ordering::Relaxed);
    if len >= lane.capacity {
        lane.overflow_count.fetch_add(1, Ordering::Relaxed);
        return Err(EcsError::EventBufferFull { /* ... */ });
    }
    // SAFETY (Phase 6 per-lane discipline): only one worker writes this lane;
    //   write_buf contents are exclusive for the worker assigned thread_index.
    unsafe {
        let box_ptr: *mut Box<[MaybeUninit<E>]> = lane.write_buf.get();
        let buf_ptr: *mut MaybeUninit<E> = (*box_ptr).as_mut_ptr();
        let dst: *mut MaybeUninit<E> = buf_ptr.add(len as usize);
        (*dst).write(event);
    }
    lane.write_len.store(len + 1, Ordering::Release);
    // Phase 12 EXT6: bump per-type counter AFTER the Release-store on write_len.
    // CachePadded ensures this counter sits on its own cache line — no false
    // sharing with reader-side fields (C3 resolution).
    self.frame_event_count.fetch_add(1, Ordering::Relaxed);
    Ok(())
}
```

### 7.2 Patch `EventBuffer::send_many` (EXT6 batch path)

```rust
pub(crate) fn send_many<I>(&self, thread_index: u32, iter: I) -> EcsResult<()>
where I: ExactSizeIterator<Item = E>,
{
    // ... existing capacity check + bulk write ...
    lane.write_len.store(len + n, Ordering::Release);
    // EXT6: single atomic op for the batch.
    self.frame_event_count.fetch_add(n as u64, Ordering::Relaxed);
    Ok(())
}
```

### 7.3 Patch `swap_and_flatten` (EXT3)

```rust
unsafe fn swap_and_flatten<E: Event>(data: *mut u8, _frame: u64) {
    let buf: &mut EventBuffer<E> = unsafe { &mut *(data as *mut EventBuffer<E>) };
    use core::sync::atomic::Ordering;

    // ... existing drop-prev-reader_buf + per-lane copy loop ...
    // cursor = total events copied to reader_buf

    // Phase 12 EXT3: snapshot frame_event_count for readers.
    let total_count = buf.frame_event_count.load(Ordering::Relaxed);
    let new_start = total_count.wrapping_sub(cursor as u64);

    // Release-store start_event_count BEFORE reader_len so readers that see
    // the new reader_len necessarily see the new start_event_count.
    // Both fields share a cache line (C3 layout) — single store-buffer flush
    // commits both to the L1d.
    buf.start_event_count.store(new_start, Ordering::Release);
    buf.reader_len.store(cursor as u32, Ordering::Release);
}
```

Order rationale: `start_event_count` stored before `reader_len`. A reader that observes the new `reader_len` necessarily sees the new `start_event_count` (release-acquire chain).

## §8 Iteration semantics + cursor advancement

### 8.1 Lifetime hierarchy

```
'static : 's : '_ (iterator)
```

- `'static`: EcsMaster + EventDispatcher.
- `'s`: System state slot (lives across frames).
- `'_`: iterator (lives during `for ev in reader.read() { ... }`).

OQ1 fix: dropped `'w` — the iterator borrows through `'s` directly (via the cursor back-pointer's lifetime extending to the slice lifetime).

### 8.2 Cursor advancement

| Scenario | Cursor outcome |
|---|---|
| Full iteration | `cursor = start_count + slice.len()` (slice was clamped to reader_len). Next `read()` returns empty until next swap moves `start_event_count`. |
| Break mid-iteration | `cursor = start_count + consumed_so_far`. Remaining events visible on next `read()`. |
| Drop without `next()` | Cursor unchanged (`consumed = 0`). Idempotent. |
| Missed frame | `start_offset = 0`; iteration consumes current `reader_buf`; cursor advances past gap; `missed_events()` returns gap size. |

### 8.3 Re-read protection

Identical to Round 1. Bevy parity.

## §9 Phase 9 conflict graph integration

### 9.1 Why Option A is sound

1. **EventWriter parallelism**: Per-lane EVT1 — distinct lanes for distinct workers. UB-free by construction.
2. **EventReader parallelism**: `reader_buf` is read-only between swaps. Any number of readers concurrent.
3. **EventReader<E> + EventWriter<E>**: Disjoint allocations (reader_buf vs write_buf).
4. **EventReader<E> + update_events**: ER5 — swap runs outside the worker window.
5. **EventWriter<E> + update_events**: Same as 4.

### 9.2 Unattached-thread caveat (W2 resolution)

Old gap: `current_worker_id_or_dispatcher_lane` returns lane 0 for both unattached threads and worker 0. A main-thread caller writing concurrently with worker 0's send would collide on lane 0's `write_buf`.

**Round 2 fix**: `EventWriter::send` / `send_many` and `EventReader::read` / `is_empty` / `len` `debug_assert!(boyko_threadpool::is_in_system_run())`. Out-of-scheduler use (tests, FFI, debugger callbacks) must route through `EcsMaster::events().send_event::<E>(...)` (Phase 6 raw API), which can choose a safe lane via the dispatcher-lane reservation.

Phase 9's `is_in_system_run()` returns true only inside the `Schedule::run` execution window — the only window where worker quiescence is guaranteed for `update_events`.

### 9.3 Documentation (in doc-comments)

`EventWriter::send`:
> Must be called from within a scheduled system body. Main-thread / FFI callers use `EcsMaster::events().send_event::<E>(...)` instead. Two `EventWriter<E>` systems running in parallel produce events in unspecified order in the next frame's `reader_buf` iteration. For deterministic ordering, serialize producers via a `ResMut<Baton>`.

`EventReader::read`:
> Multiple `EventReader<E>` systems running in parallel observe the same `reader_buf` contents in the same order (read-only slice). Concurrent writes happen on `write_buf` (disjoint allocation) and are not visible until the next `update_events`.

### 9.4 Access struct unchanged

`Access` stays at 192 B. 192 B / 3 cache lines invariant preserved.

## §10 Hot-path performance projections

### 10.1 `EventWriter::send` (hot cache)

| Step | Cycles | Notes |
|---|---:|---|
| `debug_assert!(is_in_system_run())` | 0 (release) | TLS read in debug only |
| Load `state.buffer_ptr` | 0 | Register; state already in caller frame |
| `buffer_ptr.as_ref()` → `&EventBuffer<E>` | 0 | NonNull deref is identity |
| Load `state.thread_count` | 0 | Same cache line as buffer_ptr (state line) |
| `current_worker_id_or_dispatcher_lane` | ~3-5 | TLS read (fs/gs based) |
| Lane access: `&self.lanes[lane].writer` | ~3-5 | First touch of lanes Box — L1d miss possible cold |
| `write_len.load(Relaxed)` | ~3 | Lane writer line hit |
| Capacity branch (predicted-taken steady state) | 0 | |
| Write event to `write_buf` | ~5-10 | Single store (multi-line for large E) |
| `write_len.store(Release)` | ~3-5 | |
| `frame_event_count.fetch_add(Relaxed)` | ~3-5 | Own cache line (CachePadded) — no false sharing |
| **Total** | **~20-33 cycles ≈ 5-8 ns** | At 4 GHz |

Target: ≤ 5 ns achievable hot-cache; ≤ 8 ns p99 is realistic.

**Round 2 saving vs Round 1**: Eliminated `slot.as_ref()` (3-5 cycles) and `slot.data` load (no-op now — buffer pointer is direct). Net saving: ~5 cycles per call.

### 10.2 `EventReader::read` empty case (C2 fix applied)

| Step | Cycles | Notes |
|---|---:|---|
| `debug_assert!` | 0 (release) | |
| Load `state.buffer_ptr` → deref | 0 | |
| `start_event_count.load(Acquire)` | ~3-5 | Reader cache line hit (post-swap clean) |
| `reader_len.load(Acquire)` | ~1 | Same cache line as start_event_count |
| Compute start_offset, visible_len | ~2 | Arithmetic |
| Empty check (visible_len == 0) → empty iter | ~3 | Slice ptr is `ptr.add(0)` |
| **Total** | **~9-14 cycles ≈ 2-3.5 ns** |

**Round 1 had a wasted ~3-5 cycle `frame_event_count.Acquire` load** — Round 2 drops it (C2). Net saving: ~3-5 cycles per call.

### 10.3 `EventReader::read` per-element

Unchanged from Round 1: ~3-5 cycles ≈ 1-1.5 ns per element (`get_unchecked` + cursor +1 + loop branch). Target ≤ 2 ns achievable.

### 10.4 Aggregate frame budget

10k events × 6 ns send + 10k × 1.5 ns read = 60 + 15 = 75 µs nominal raw. With `send_many` (batching to one fetch_add per 100-event burst): ~30 µs. Within 50 µs target.

### 10.5 L1d footprint of hot loop

Send loop:
- `EventWriterState<E>`: 24 B (1 line, often shared with adjacent params in tuple).
- `EventBuffer<E>` `frame_event_count` line: 64 B (CachePadded — own line).
- Per-lane writer line: 64 B.
- Event payload (write target): `size_of::<E>()` B.

Total per-send touch: 3 distinct cache lines. Round 1 used 4 (slot line + buffer line + lane line + payload). Round 2 saves one line per send by skipping the slot.

Read loop:
- Reader state: 24 B (1 line).
- `EventBuffer<E>` reader line (start_event_count + reader_len + reader_buf header): 64 B.
- Slice contents: `n × size_of::<E>()` — sequential, prefetcher-friendly.

L1i: `read()` body fits within ~100 bytes x86_64; `next()` body fits in one line. Within L1i.

## §11 Memory layouts + sizes

### 11.1 Field sizes (revised)

| Type | Phase 6 size | Phase 12 R2 size | Delta | Notes |
|---|---:|---:|---:|---|
| `EventTypeSlot` (release) | 64 B | 64 B | 0 | Unchanged. |
| `EventTypeSlot` (debug) | 64 B | 64 B | 0 | Unchanged. |
| `EventBuffer<E>` | ~64 B | ~192 B | +128 | `CachePadded<AtomicU64>` adds 64 B; new fields + alignment fillers add ~64 B. **Per-event-type cost**, paid once at preregister — not per-instance. |
| `EventWriterState<E>` | — | 24 B | new | buffer_ptr + thread_count + event_id |
| `EventReaderState<E>` | — | 24 B | new | buffer_ptr + last_event_count + thread_count (O1: no pad) |
| `EventWriter<'s, E>` | — | 8 B | new | `&'s mut State` |
| `EventReader<'s, E>` | — | 8 B | new | `&'s mut State` (O2 fix: not 16 B) |
| `EventIter<'a, E>` | — | 48 B | new | slice (16) + consumed (8) + cursor ref (8) + start_count (8) + missed (8) |
| `Access` | 192 B | 192 B | 0 | Q2 Option A — no event bits |

Compile-time asserts:
```rust
const _: () = assert!(core::mem::size_of::<EventWriterState<LayoutAssertEvent>>() == 24);
const _: () = assert!(core::mem::size_of::<EventReaderState<LayoutAssertEvent>>() == 24);
const _: () = assert!(core::mem::size_of::<EventWriter<'_, LayoutAssertEvent>>() == 8);
const _: () = assert!(core::mem::size_of::<EventReader<'_, LayoutAssertEvent>>() == 8);
const _: () = assert!(core::mem::size_of::<EventIter<'_, LayoutAssertEvent>>() == 48);
```

### 11.2 Cache behaviour

- `EventWriterState<E>` + `EventReaderState<E>`: 24 B each. A tuple of (Reader<A>, Writer<B>, Reader<C>) is 24×3 = 72 B → 2 cache lines.
- `EventBuffer<E>` head: 1 cache line for `frame_event_count` (send-hot), 1 cache line for reader-side fields (read-hot). 128 B per buffer for layout discipline. Acceptable — per-event-type cost.
- `EventIter`: lives on the system body's stack frame; in L1d for the iteration duration.

## §12 Public API surface examples

### 12.1 Basic reader/writer pair

```rust
use boyko_ecs::prelude::*;

#[event]
#[derive(Clone, Copy)]
struct Explosion {
    pos: [f32; 3],
    radius: f32,
}

fn fire_explosion(
    mut events: EventWriter<Explosion>,
    query: Query<&Position, With<Bomb>>,
) {
    for pos in query.iter() {
        events.send(Explosion { pos: pos.0, radius: 5.0 })
            .expect("explosion buffer full");
    }
}

fn handle_explosions(
    mut events: EventReader<Explosion>,
    mut query: Query<&mut Health>,
) {
    for ev in events.read() {
        for mut hp in query.iter_mut() {
            hp.0 -= damage_falloff(ev.pos, ev.radius);
        }
    }
}

// Setup:
let mut ecs = EcsMaster::new();
ecs.preregister_event_default::<Explosion>().unwrap();
let mut sched = Schedule::new();
sched.add_system(fire_explosion);
sched.add_system(handle_explosions);
```

### 12.2 Multi-event router

```rust
fn router(
    mut explosions: EventReader<Explosion>,
    mut sounds: EventWriter<SoundCue>,
    mut score: EventWriter<ScoreDelta>,
) {
    for ev in explosions.read() {
        sounds.send(SoundCue::Boom(ev.pos)).ok();
        score.send(ScoreDelta(10)).ok();
    }
}
```

### 12.3 Batched writer

```rust
fn spawn_particles(mut events: EventWriter<Particle>) {
    events.send_many((0..100).map(|i| Particle::new(i))).ok();
}
```

### 12.4 Missed-events diagnostic

```rust
fn audio_system(mut events: EventReader<AudioCue>) {
    if events.missed_events() > 0 {
        eprintln!("audio: dropped {} cues", events.missed_events());
    }
    for cue in events.read() {
        play(cue);
    }
}
```

### 12.5 Coexistence with `Commands::send_event`

```rust
fn spawn_with_pickup(
    mut commands: Commands,
    mut events: EventWriter<Spawned>,
) {
    let id = commands.spawn(EnemyBundle::default()).id();
    events.send(Spawned { entity: id }).ok();
}
```

Distinction:
- `events.send()` (Phase 12): immediate, ~5 ns, worker's lane this frame, visible after next swap.
- `commands.send_event()` (Phase 11): queued, ~18 ns enqueue + apply-window cost, dispatcher's lane.

### 12.6 Main-thread test (post W2 fix)

```rust
#[test]
fn main_thread_send_via_dispatcher() {
    let mut ecs = EcsMaster::new();
    ecs.preregister_event_default::<TestEvent>().unwrap();
    // EventWriter::send would debug-panic here — use the raw API instead:
    ecs.events().send_event::<TestEvent>(TestEvent::default()).unwrap();
    ecs.update_events();
    assert_eq!(ecs.events_of::<TestEvent>().len(), 1);
}
```

## §13 Test plan

### 13.0 EventId range reservation (W3 resolution)

**Phase 12 reserves EventId range 100-119** (20 slots) for tests in this phase. Existing reservations:
- Phase 6: 10-19
- Phase 9: 20-29, 50-69, 80
- Phase 10 (Tick): 70-79 (verified via `Grep` for `event_id() = 7X` in tests)
- Reserved for future: 200-255

Tests in §13 use `#[event]` derive (which mints from `NEXT_EVENT_ID` atomically). Tests that need specific ids declare them explicitly in this 100-119 range to avoid collisions.

### 13.1 Unit tests (in-module)

`event_writer.rs`:
- `event_writer_init_state_caches_buffer_ptr_and_thread_count` — assert state fields post-init.
- `event_writer_send_routes_via_tls_lane_inside_system_run` — must be invoked from inside `Schedule::run` (or via test harness that sets the `is_in_system_run` flag).
- `event_writer_send_returns_buffer_full_on_overflow` — fill lane → `EventBufferFull`.
- `event_writer_send_many_atomic` — Phase 6 parity.
- `event_writer_init_state_panics_on_unregistered_event` — `#[should_panic]`.
- `event_writer_send_debug_panics_outside_system_run` — `#[cfg(debug_assertions)] #[should_panic]` (W2 fix).

`event_reader.rs`:
- `event_reader_init_state_starts_at_cursor_zero`.
- `event_reader_read_empty_when_cursor_caught_up` — `is_empty()` true initially after swap with no sends.
- `event_reader_read_advances_cursor` — send 3 (via raw API), swap, read 3, cursor = start_count + 3.
- `event_reader_read_break_advances_partially` — send 5, swap, break after 2, second read yields remaining 3.
- `event_reader_missed_events_after_skipped_frame`.
- `event_reader_clear_skips_to_now` — verify `clear()` advances cursor to current `frame_event_count`.
- `event_iter_drop_checkpoints_cursor_on_unwind` — panic mid-iter, verify drop ran, cursor reflects pre-panic count.
- `event_reader_is_empty_consistent_with_len` — across all states, `is_empty() == (len() == 0)` (OQ2 invariant).
- `event_reader_no_frame_event_count_load_on_read` — Miri-instrumented check that `read()` does not touch the CachePadded counter cache line (C2 verification — optional, fall back to manual `cargo asm` inspection if Miri tracking is infeasible).

`event_dispatcher.rs` extensions:
- `frame_event_count_bumps_on_send`.
- `frame_event_count_no_bump_on_overflow`.
- `start_event_count_updates_at_swap`.
- `buffer_ptr_returns_some_for_registered` — preregister, `Some`. Unregistered → `None`.
- `buffer_ptr_pointer_stability` — preregister, capture pointer, send 1000 events + swap, verify pointer unchanged.
- `event_buffer_layout_asserts` — runtime cross-check of compile-time offsets (sanity belt-and-braces).

### 13.2 Integration tests

`event_systemparam_smoke.rs`:
- `event_writer_reader_round_trip` — preregister, system A writes, swap, system B reads.
- `multi_event_type_systems_compile_and_run`.
- `event_writer_parallel_safety` — 4 worker systems each sending 1000 events of same type → total 4000 after swap. Verify under Miri.

`event_reader_cursor_behavior.rs`:
- `cursor_persists_across_frames`.
- `cursor_clears_on_explicit_call`.

### 13.3 Property tests (`proptest`)

In `event_systemparam_smoke.rs`:
- For random `(num_sends, num_reads_per_frame, num_frames)`, total events observed = total sent (modulo `missed_events`).

### 13.4 Miri tests (`crates/boyko_ecs/tests/miri_phase12.rs`)

- `miri_event_writer_send_does_not_violate_aliasing` — 100 sends through cached `buffer_ptr`. Miri clean.
- `miri_event_reader_read_does_not_alias_writer` — interleaved send/read (single-threaded). Miri clean.
- `miri_buffer_ptr_provenance_across_update_events` (renamed from Round 1's `miri_slot_ptr_provenance`) — cache buffer_ptr, run `update_events` (which takes `&mut EventDispatcher` and triggers swap), then re-deref the cached buffer_ptr in a subsequent system run. Verify NO Stacked-Borrows violation. **This is the central C1 validation test.**
- `miri_cursor_persistence_through_panic` — panic mid-iter; verify drop runs; verify cursor reflects pre-panic state.
- `miri_event_writer_state_send_across_workers` — simulate scheduler migration: build state on thread A, send across to thread B, use for 1000 sends. Verify SEND-EV1 invariants under Miri.

### 13.5 Criterion benches (`crates/boyko_ecs/benches/event_systemparam.rs`)

- `event_writer_send_1` — target ≤ 5 ns.
- `event_writer_send_many_1k`.
- `event_reader_read_empty` — target ≤ 3 ns.
- `event_reader_read_1k` — per-element ≤ 2 ns.
- `event_round_trip_10k_per_frame` — target ≤ 50 µs.
- `event_writer_send_vs_dispatcher_send_baseline` — A/B vs Phase 6 raw `send_event::<E>`. Expect Phase 12 SystemParam path ~30% faster (cached buffer pointer).

### 13.6 Compile-fail tests

- `event_reader_outlives_state.rs` — attempt to store `EventReader<E>` past the system body's state borrow. Lifetime error expected.
- `event_zst_event_rejected.rs` — existing Phase 6 ZstCheck mechanism. Verify still triggers.

## §14 Step-by-step implementation

| Step | Wave | File(s) | What | Lines | Depends on |
|---|---|---|---|---:|---|
| 1 | A | `event_buffer.rs` | Add `CachePadded<AtomicU64> frame_event_count` + `AtomicU64 start_event_count` to `EventBuffer<E>`. Patch `new()` to init both to 0. Pin field order with `#[repr(C)]`. Add offset asserts. | ~25 | — |
| 2 | A | `event_buffer.rs` | Patch `send_one` / `send_many` to `fetch_add` on `frame_event_count`. | ~6 | 1 |
| 3 | A | `event_dispatcher.rs` | Patch `swap_and_flatten` to compute and Release-store `start_event_count` before `reader_len`. | ~8 | 1, 2 |
| 4 | A | `event_dispatcher.rs` | Add `buffer_ptr::<E>` accessor (pub(crate)). | ~15 | — |
| 5 | A | tests | Unit tests for steps 1-4 (frame/start counter updates, buffer_ptr stability, layout asserts). | ~120 | 1-4 |
| 6 | B | `system/params/event_writer.rs` (new) | Define `EventWriterState<E>` + `EventWriter<'s, E>` + SystemParam impl. | ~180 | 4 |
| 7 | B | `system/params/event_reader.rs` (new) | Define `EventReaderState<E>` + `EventReader<'s, E>` + `EventIter<'a, E>` + SystemParam impl. | ~220 | 4 |
| 8 | B | `system/params/diagnostics.rs` | Add `event_not_preregistered_panic::<E>`. | ~15 | — |
| 9 | B | `system/params/mod.rs` | Re-export `EventReader`, `EventWriter`. | ~5 | 6, 7 |
| 10 | B | `boyko_threadpool/src/tls.rs` (verify) | Confirm `is_in_system_run() -> bool` exists (Phase 9 introduced it). If not, add it. | ~10 (if needed) | — |
| 11 | B | tests | Unit tests for steps 6-7 (init_state, get_param, send/read smoke, debug panic on out-of-scheduler use). | ~180 | 6-10 |
| 12 | C | `tests/event_systemparam_smoke.rs` (new) | Round-trip + multi-type + parallel-safety. | ~280 | 6-11 |
| 13 | C | `tests/event_reader_cursor_behavior.rs` (new) | Cross-frame cursor + clear + missed_events. | ~150 | 7, 11 |
| 14 | C | `tests/miri_phase12.rs` (new) | C1 provenance test + alias-freedom + panic-safety. | ~250 | 6-11 |
| 15 | C | `tests/event_systemparam_compile_fail/` | Lifetime + ZST compile-fail. | ~30 | 6-7 |
| 16 | D | `benches/event_systemparam.rs` (new) | Criterion benches for ≤5 ns / ≤3 ns / ≤2 ns / ≤50 µs targets. | ~270 | 6-11 |
| 17 | D | docs | `docs/SYSTEMS.md` + `docs/FEATURE_MAP.md` patches. | ~30 | 6-11 |
| 18 | D | book | `book/src/events_systemparam.md` (doc-writer agent). | ~150 | All |

### 14.1 Wave parallelism

- **Wave A** (steps 1-5): EventDispatcher / EventBuffer changes. Step 1 must precede steps 2-3. Steps 2 and 3 can run sequentially; step 4 is independent (single developer, sequential within wave).
- **Wave B** (steps 6-11): EventReader / EventWriter / mod / diagnostics / tests. Steps 6 and 7 are independent (separate files) — two parallel developer agents. Step 8 independent — third agent. Step 10 verification — fourth agent. Step 9 and 11 serialise after.
- **Wave C** (steps 12-15): All four file-independent — four parallel agents.
- **Wave D** (steps 16-18): Three file-independent — three parallel agents.

Estimated wall time with 4 parallel developer agents: Wave A 4-6 h sequential. Wave B 4-6 h with parallelism. Wave C 3-4 h parallel. Wave D 2-3 h parallel. Plus 2-3 critic rounds. Total 15-19 h.

### 14.2 Build-and-test gates per wave

- Wave A: `cargo check --all-targets`, `cargo test --test event_dispatch_*`, Phase 6 baseline benches unchanged.
- Wave B: full `cargo check`, `cargo clippy -- -D warnings`, SystemParam smoke tests.
- Wave C: `cargo test --all-targets`, `cargo +nightly miri test --test miri_phase12`. **C1 test (`miri_buffer_ptr_provenance_across_update_events`) must be Miri-clean — this is the merge gate.**
- Wave D: `cargo bench --bench event_systemparam` against agreed targets.

## §15 Migration impact

### 15.1 Existing API stays

- `EcsMaster::events()` / `events_of::<E>()` / `send_event::<E>` / `preregister_event::<E>` / `update_events`: unchanged.
- `EventDispatcher::send_event` / `send` / `send_many`: unchanged public surface. Internal: bumps `frame_event_count` on success.
- `Commands::send_event::<E>`: unchanged.
- `SendEventCommand<E>`: unchanged.

### 15.2 New API additions

- `EventDispatcher::buffer_ptr::<E>` (`pub(crate)`).
- `EventBuffer<E>` fields: `frame_event_count` (`pub(crate)`), `start_event_count` (`pub(crate)`).
- `EventReader<'s, E>`, `EventWriter<'s, E>`, `EventIter<'a, E>` (pub).
- `EventReaderState<E>`, `EventWriterState<E>` (pub for clarity).

### 15.3 Behavioural changes

- `frame_event_count` bumped on every successful send (process-lifetime monotonic). Tests reading the counter directly will observe accumulation across frames.
- `EventBuffer<E>` size grew by ~128 B (per-event-type cost, paid once at preregister).
- Out-of-scheduler `EventWriter::send` / `EventReader::read` debug-panics. Tests must either run inside a `Schedule` or use raw `EcsMaster::events()`.

### 15.4 Performance regression risk

- `fetch_add(1, Relaxed)` per send on a dedicated cache line: ~0.3 ns. Phase 6 raw path: ~10 → ~10.3 ns. **3% slowdown** on a path that was not on the critical hot loop. SystemParam path is **~5 ns** (faster than raw due to cached buffer pointer).

### 15.5 `docs/SYSTEMS.md` update

```
### EventReader<E> / EventWriter<E> — Phase 12

Typed SystemParam wrappers over the EventDispatcher. Cache
`NonNull<EventBuffer<E>>` + thread_count in state for sub-5-ns `send` and
sub-3-ns `is_empty`/`read` empty paths. SB/TB-clean across `update_events`
via heap-stable buffer addresses.

- Source: `crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs`
  + `event_writer.rs`
- Tests: `tests/event_systemparam_smoke.rs`, `tests/miri_phase12.rs`
  (key test: `miri_buffer_ptr_provenance_across_update_events`)
- Benches: `benches/event_systemparam.rs`
- Plan: `docs/PHASE-12-EVENTS-SYSTEMPARAM-PLAN.md`
- Reserved EventId range for Phase 12 tests: 100-119.
- Coexists with `Commands::send_event` (queued) and
  `EcsMaster::events_of::<E>` (raw).
```

## §16 Rejected alternatives

### 16.1 `Local<EventCursor<E>>` (Bevy-style composition)

Rejected (Q5). YAGNI.

### 16.2 `Res<Events<E>>` / `ResMut<Events<E>>` composition

Rejected (Q3 alternative). Forces serialisation; doesn't solve caching.

### 16.3 Tick-keyed events

Rejected (Q1). Per-event footprint bloat.

### 16.4 Adding event bits to `Access`

Rejected (Q2). Breaks 192 B invariant; loses parallelism.

### 16.5 Frame-only reader

Rejected (Q1). Breaks once-per-event for sub-schedules.

### 16.6 Push-based observers (flecs-style)

Rejected. `dyn` dispatch + pull > push for steady state.

### 16.7 Cache `NonNull<EventTypeSlot>` (Round 1 design)

**Rejected in Round 2 (C1 fix)**. Stacked-Borrows violation across `update_events`. Replaced by `NonNull<EventBuffer<E>>` whose provenance is anchored to `Box::into_raw` — SB-clean.

### 16.8 `EventMutator<E>` (Bevy 0.16)

Out of scope.

### 16.9 `EventReader::read` as `&self`

Rejected (OQ4). `&mut self` matches Bevy + our writer-pattern.

### 16.10 Drop-and-recreate `EventDispatcher`

Rejected (Q3 trade-off). Documented as future-phase concern (EW8).

### 16.11 Generation counter for cached buffer_ptr invalidation

Considered for safety against future dispatcher rebuild. **Deferred**: no rebuild is planned in Phase 12. If a future phase introduces rebuild, every cached State must be invalidated via a generation field — flagged as a forward-compat note in EW8.

## §17 Open questions — resolutions

### OQ1. `'w` lifetime in `EventReader`

**Resolved**: dropped. `EventReader<'s, E>` is the final shape. `EventIter<'a, E>` borrows through `'s` via the cursor back-pointer.

### OQ2. `is_empty()` vs `len()` consistency

**Resolved**: both clamp to `reader_len`. `is_empty()` formula: `cursor.saturating_sub(start_count) >= reader_len`. Race-free per ER5.

### OQ3. `buffer_ptr<E>` `Option` vs panic

**Resolved**: keep `Option` at `pub(crate)`. Panic in `init_state` via cold helper. Cleaner — useful for diagnostics that want a non-panicking `None`.

### OQ4. `&mut self` vs `&self` on `send`

**Resolved**: `&mut self`. Bevy parity + writer-pattern + reserves option to mutate state in future (e.g. local send-counter for diagnostics) without API break.

### O1. Drop `EventReaderState::_pad`

**Resolved**: dropped. State shrinks from 32 B to 24 B.

### O2. `EventReader<'s, 'w, E>` size

**Resolved**: 8 B (single `&'s mut State`). `'w` removed (OQ1). Round 1's 16 B claim was simply wrong.

### O3. Late-reader semantics

**Resolved**: keep cursor init at 0 (Bevy parity). Users wanting "skip historical" call `clear()`.

### O4. `EventReader::read` returning `&[E]` directly

**Resolved**: rejected. Iterator hides missed-events/start-offset math and advances cursor on drop. Future `peek() -> &[E]` may be added without breaking the iterator API.

### O5. `EventWriter::send` infallible

**Resolved**: keep `Result`. Silent drops obscure capacity bugs.

### O6. Re-add system semantics

**Resolved**: fresh state, cursor = 0. Matches Bevy.

### O7. Per-event-id `read_with_id`

**Resolved**: deferred. Compute from `start_event_count + slice_index` when needed (no per-event storage cost).

### O8. Debug check for `EventReader<E> + EventWriter<E>` same system

**Resolved**: rejected. Option A is deliberate. Users wanting Bevy semantics serialize via `ResMut<Baton>`.

---

## Plan readiness checklist (Round 2 final)

- [x] Goal stated in perf + functional terms (§1.1)
- [x] Target metrics concrete (§1.2)
- [x] Each architectural decision justified (Q1–Q6 in §3)
- [x] Alternatives rejected with reasons (§16)
- [x] Trade-offs honestly listed (§3 Q2, §3 Q3, §16.11 forward-compat)
- [x] Each field has type + role comment (§5.1, §5.2)
- [x] `#[repr(...)]` specified (§4.2 `repr(C)` with offset asserts, §5.1/5.2 `repr(C)`, §6.1 `repr(transparent)`)
- [x] Hot/cold split (§4.2 — send-line vs read-line vs lane-line on `EventBuffer<E>`)
- [x] Struct sizes known and justified (§11.1)
- [x] False-sharing padding specified (`CachePadded<AtomicU64>` on `frame_event_count` — §4.2)
- [x] Cache-line layout pinned with compile-time asserts (§4.2 offset_of asserts)
- [x] Public API minimal (§12)
- [x] No internal types leaked (states are pub for clarity, not named in user code)
- [x] Lifetimes explicit (`'s` state, `'a` iter; `'w` dropped per OQ1)
- [x] No `dyn Trait` in hot path
- [x] Generics where specialised (per-`E` monomorphisation)
- [x] Multi-threading model described (§9)
- [x] Atomic orderings explicit (Acquire/Release/Relaxed in §4, §7)
- [x] Sync points justified (§9.1: per-lane EVT1 + update_events &mut barrier; W2 fix: `is_in_system_run()` debug-gate)
- [x] Data partitioning described (per-lane writes preserved from Phase 6)
- [x] `Send`/`Sync` consistent (§2.4 SEND-EV1/2/3) + SAFETY comment with C1 reasoning
- [x] Edge cases (empty, MAX, overflow, wrap) (§2.2 ER6/ER7; §10.4 wrap)
- [x] Drop order — `EventIter::drop` advances cursor (§6.3)
- [x] Invariants for unsafe blocks stated (EW2/EW3/EXT5/C1-resolution refs)
- [x] Affected modules listed (§15)
- [x] Existing API change noted (none; additive only)
- [x] Compatibility with Arena/ComponentPool/UnitId verified (no interaction)
- [x] Implementation steps broken down (§14, 18 steps in 4 waves)
- [x] Unit tests specified (§13.1)
- [x] Integration tests specified (§13.2)
- [x] Property tests specified (§13.3)
- [x] Miri tests specified (§13.4, key test: C1 provenance)
- [x] Benches specified (§13.5)
- [x] `debug_assert!` invariants listed (`is_in_system_run`, lane bounds, ZstCheck retained)
- [x] EventId test range reserved (§13.0 — 100-119)
- [x] Critic round 1 resolutions enumerated (§0 changelog)
- [x] SB/TB-soundness explicitly argued for cached pointer (§2.3 EXT5, §3 Q3, §16.7)
- [x] C2 dead-load removal documented in cost table (§10.2)
- [x] C3 layout discipline pinned with compile-time asserts (§4.2)

---

End of Phase 12 plan (Round 2).

---

## Notes for the orchestrator

**File to overwrite**: `D:\BoykoEngine\docs\PHASE-12-EVENTS-SYSTEMPARAM-PLAN.md` (architect cannot Write — orchestrator's developer agent must persist this content).

**Round 2 net summary**:
- 3 criticals (C1/C2/C3) resolved with concrete code-shape changes.
- 3 importants (W1/W2/W3) resolved via cache extensions + debug-asserts + reserved id range.
- 2 optionals (O1/O2) applied.
- 4 open questions resolved (OQ1 drop `'w`, OQ2 clamp consistency, OQ3 keep `Option`, OQ4 keep `&mut self`).

**Key shape change**: cached pointer is now `NonNull<EventBuffer<E>>` (not `NonNull<EventTypeSlot>`). This is the central C1 fix and ripples through §2-§7, §10-§11, and §13.4. The Miri provenance test (`miri_buffer_ptr_provenance_across_update_events`) is the merge gate for Wave C.

**Round 3 expectation**: polish only / APPROVED — no further critical issues are anticipated.