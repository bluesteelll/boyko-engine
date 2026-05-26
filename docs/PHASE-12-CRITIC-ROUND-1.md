# Architecture review: Phase 12 — Events as SystemParam

## Verdict

[X] CHANGES REQUESTED — three blockers, three important items, two optional.

## Remarks

### Critical (blockers — implementation must not start)

#### C1. Stacked-Borrows risk on cached `slot_ptr` across `update_events`

**Where**: §2.3 EXT5 line 81, §4.3 line 280-281, §5.1 line 358, §6.4 lines 696-697; Test plan §13.4 `miri_slot_ptr_provenance`.

**Problem**: The plan derives `NonNull<EventTypeSlot>` from `&EventTypeSlot` (via `as *const _ as *mut _`), caches it in `Send + Sync + 'static` state, and later re-derives `&EventTypeSlot` from it inside `get_param`. **Between** these two operations, `EcsMaster::update_events` takes `&mut EventDispatcher` and (in debug builds) writes `last_swap_frame` on the very same `EventTypeSlot`. Under Stacked Borrows / Tree Borrows that `&mut self` pops all shared borrows derived from the parent `&EventDispatcher`, invalidating the cached pointer's provenance. A subsequent `as_ref()` is undefined behaviour that Miri will catch. This isn't a stale-data bug — it is an aliasing-model violation that the plan justifies by appealing to "the dispatcher is heap-pinned" (EXT5), which addresses address stability but not provenance.

**Why critical**: The cached-pointer optimisation is the centerpiece of Q3 and the ~5 ns hot-path target. The plan's `miri_phase12.rs::miri_slot_ptr_provenance` test "send 1000 events with intervening swaps" will almost certainly fail under Miri.

**What is needed**: Resolve the SB/TB model before implementation. Either (a) cache a pointer whose provenance survives `&mut EventDispatcher` (e.g. derive from a stable raw pointer planted at preregister time, not from `&` re-borrow), (b) cache the `*mut EventBuffer<E>` directly (bypasses the slot entirely on the hot path; the slot is only needed for `thread_count`, which can be cached as `u32` in state), or (c) document and prove that no `&mut EventDispatcher` borrow ever overlaps the cached pointer's use. State which path is chosen and update the SAFETY comments accordingly. The "audited; ~0 cycles" hand-wave in Q3 cost line is not sufficient.

#### C2. Wasted `frame_event_count` Acquire-load in `EventReader::read()`

**Where**: §6.2 line 515 vs lines 519-551; §10.2 cost projection.

**Problem**: `read()` loads `frame_count = buf.frame_event_count.load(Acquire)` on line 515 and never uses it for the slice math (which depends only on `cursor`, `start_count`, `reader_len`). On the hot read path that target ≤ 3 ns empty / ≤ 2 ns per-element, this is a wasted ~3-5 cycle Acquire-load on a counter that lives in a different cache line from `reader_len` and `start_event_count` (because `frame_event_count` is bumped on every send → invalidated frequently, while `start_event_count` / `reader_len` change only at swap → mostly clean). The empty-case projection at §10.2 charges `frame_event_count.Acquire` once, but the full-read path charges it AND `reader_len.Acquire` AND `start_event_count.Acquire` — three Acquire loads, the first of which is dead.

**Why critical**: Either the load is dead and should be removed (silent regression vs target), or the slice math is incomplete (the plan does not bound `end_offset` against `frame_event_count`, which under Option A could leave a reader observing **uncommitted in-flight writes** if `reader_len < frame_event_count - start_event_count` and the math ever shifts to use `frame_count` instead of `reader_len`). Specify which.

**What is needed**: Decide whether `read()` clamps to `reader_len` (current plan, then drop the `frame_count` load entirely) or to `min(frame_count - start_count, reader_len)` (then justify why the second min ever matters when ER5 forbids in-flight swaps). Update §6.2, §10.2, and the Acquire-ordering rationale in §4 accordingly.

#### C3. EventBuffer<E> two new AtomicU64 fields create false-sharing risk with the per-frame swap path

**Where**: §4.4 revised layout (lines 322-334), §7.1-7.3 patches, §11.1 row "EventBuffer<E> +16 B".

**Problem**: Phase 6 took deliberate care to put `ThreadLaneWriter` and `ThreadLaneReader` on disjoint 64 B cache lines (`event_buffer.rs:42-47`) to keep send-path writes off the swap-path read line. The plan now adds `frame_event_count: AtomicU64` and `start_event_count: AtomicU64` directly on `EventBuffer<E>` after `reader_len`. Layout: `lanes: Box<[...]>` (16 B) + `reader_buf: Box<[...]>` (16 B) + `reader_len: AtomicU32` (4 B) + `frame_event_count: AtomicU64` (8 B aligned, +4 B pad → 8 B) + `start_event_count: AtomicU64` (8 B) + `capacity: u32` + `thread_count: u32` + PhantomData. Without explicit `repr(C, align(64))` and padding, the layout depends on field order. `frame_event_count` is hot on EVERY send (fetch_add Relaxed → cache line owned exclusively by the writer-of-the-moment), while `reader_buf` (a `Box` header) is read on every reader iteration. If they share a cache line, every send by **any** worker invalidates every reader's L1d copy of `reader_buf`'s box header → an MESI invalidation storm precisely on the hot read path. The plan does not specify the field layout, padding, or `repr` of the modified `EventBuffer<E>`.

**Why critical**: This is the same false-sharing class that Phase 6 ThreadLanePair was explicitly designed to avoid (per the doc-comment on `ThreadLanePair`). Re-introducing it on `EventBuffer<E>` head silently regresses the multi-writer scenario the plan claims as its main advantage over Bevy (§3 Q2).

**What is needed**: Specify the `EventBuffer<E>` field layout explicitly with `repr(C)`, lay `frame_event_count` and `start_event_count` out so that (a) `frame_event_count` does not share a cache line with `reader_buf` / `reader_len` / `start_event_count`, and (b) `start_event_count` and `reader_len` may share a line (both Release-stored at the same swap, both Acquire-loaded by readers — they want to share). Add a compile-time assert pinning the layout. Update §10.5 L1d footprint.

### Important (must be resolved, but options can be discussed)

#### W1. Per-call `EventWriter::send` reads `slot.thread_count` and re-routes the lane every call

**Where**: §6.1 lines 442-451, §10.1 line 916.

**Problem**: Every `send()` does: `slot.as_ref()` → `slot.data` → cast → `slot.thread_count` load → `current_worker_id_or_dispatcher_lane(thread_count - 1)`. The `slot.thread_count` is **immutable post-preregister**. The lane resolution likewise: for the same thread, the same `current_worker_id_or_dispatcher_lane(N)` returns the same value. Caching the lane index at `init_state` time (when the system is assigned to a worker by the Phase 9 scheduler) would reduce `send()` to one buffer-pointer deref + one `send_one`. But the cache cannot live in the SystemParam state across worker reassignment — Phase 9 lets systems migrate.

**Solution options**:
1. Cache `*mut EventBuffer<E>` AND `thread_count: u32` in state (bypasses slot entirely on hot path; closes C3 by accident); leave the lane TLS lookup unchanged.
2. Document explicitly that the lane is resolved per-call by design (worker migration safety) and accept the 3-5 cycle cost; then the target ≤ 5 ns is tight but feasible.

The plan currently does neither — it caches the slot pointer but still goes through `slot.data` indirection AND the TLS lookup on every call.

#### W2. Option A claim "parallel writers UB-free" does not address unattached-thread / worker-0 lane collision

**Where**: §9.1 line 875.

**Problem**: `current_worker_id_or_dispatcher_lane` returns lane 0 for unattached threads AND for worker id 0. If a test/main thread calls `EventWriter::send` while worker 0 is concurrently running a system that also calls `EventWriter::send` (same E), both write to lane 0's `write_buf`. The §6.1 plan inherits this from Phase 9's `send_event` — but the plan now markets parallel-writer-safety as a deliberate boyko advantage (§3 Q2, §9.2). The advantage holds **inside `Schedule::run`** (Phase 9 quiesces unattached callers) but **not** in mixed-call scenarios that the doc-comments under §9.3 do not exclude. The §13.2 `event_writer_parallel_safety` test "4 worker threads each system sends 1000 events of same type" works because all 4 are workers; it does not cover main-thread + worker-0 concurrency.

**What is needed**: Either (a) document explicitly that `EventWriter` may only be called from within a scheduled system body (debug-assert via `is_in_system_run()`), pushing main-thread tests through the raw `EcsMaster::events().send_event` path (which can choose a safe lane), or (b) prove that the unattached-thread case is impossible under Phase 9's invariants (no proof currently given), or (c) make the unattached-thread lane choice non-zero (e.g. dedicate the dispatcher lane to unattached callers too) and document the trade-off.

#### W3. Test event-id range conflicts with established Phase 6/9 ranges

**Where**: §13 test plan; no explicit event-id range chosen.

**Problem**: Existing tests use event_id() = 20, 22, 50, 60-62, 70, 80, 90, 200, 201, 210, 215, 411-413 are NOT events (they are ComponentId slots — the user's brief had a fact error). Bench `event_dispatch.rs` uses 80. The plan §13 specifies no event-id range, and `event_attribute.rs` minted ids `>200` already. With `MAX_EVENTS = 256`, the available headroom is shrinking. Phase 12 tests need a documented reserved range to prevent collisions with future phases.

**What is needed**: §13 must designate a contiguous reserved range (e.g. 100-119 for Phase 12; or use `register_event_new` so the test does not hard-code an id). The current plan silently leaves this open and the developer will pick ad-hoc ids that may collide on full-test-suite runs (`#[event]` minted ids share the global `NEXT_EVENT_ID` counter).

### Optional (improvements, not blockers)

#### O1. `EventReaderState::_pad: u64` reserved with no concrete use case

**Where**: §5.2 line 397.

Allocating 8 B for unspecified "per-reader flags" (log-on-skip, drain-on-drop) before a real consumer exists contradicts YAGNI. Drop the field and reduce state to 24 B, or document the planned use. If the alignment-to-32 requirement is the real driver, justify it (the §11.2 claim that 32 B "fits one cache line" is true for 24 B too).

#### O2. Inconsistent size claim for `EventReader<'s, 'w, E>` (16 B vs actual 8 B)

**Where**: §11.1 row "EventReader<'s, 'w, E> | — | 16 B".

`&'s mut State` is 8 B; `PhantomData<&'w EventDispatcher>` is ZST. Total 8 B, not 16 B. Either the plan is hiding a `start_count_snapshot: u64` (cf. EventIter), in which case state it, or the size is wrong. Update §11.1 to match the concrete fields in §6.2 (lines 490-496).

## Positive

- **Q3 cached slot pointer concept is the right direction** — eliminating the `OnceLock` acquire-load + bounds check + mask check per send is exactly the kind of cache-resident-state optimisation principle #3 demands. The blocker is the SB/TB compliance, not the idea.
- **Counter placement moved from `EventTypeSlot` to `EventBuffer<E>` (§4.4 revision)** — keeps `EventTypeSlot` at the existing 64 B layout, avoids breaking Phase 6 compile-time asserts. Correct call.
- **Option A documented as a deliberate boyko advantage** (§3 Q2, §9.2) with the trade-off (within-frame order non-determinism for parallel writers) honestly listed — exactly the discipline principle #10 requires.
- **Drop-finalised cursor checkpoint with break-mid-iter support** (§6.3 EventIter::drop) is the correct Bevy-compatible shape and the panic-safety implication is acknowledged (§13.4 `miri_cursor_persistence_through_panic`).
- **`#[cold] + #[inline(never)]` on `event_not_preregistered_panic`** (§6.5) — correct application of principle #7 measured inlining for an error path.
- **§10 hot-path cycle budgets per instruction** are concrete and falsifiable. Better than most plans.
- **Q5 deferring `Local<T>` until a second consumer exists** is the right YAGNI call.

## Open questions for the architect

1. **`'w` lifetime in `EventReader<'s, 'w, E>`**: the plan introduces `_world: PhantomData<&'w EventDispatcher>` but `read()` returns `EventIter<'_, 's, E>` with no `'w` binding. What is `'w` actually constraining? If nothing, drop it (the SystemParam trait does not require the param to carry `'w`).

2. **`EventReader::is_empty()` vs `EventReader::len()` consistency under concurrent send**: `is_empty()` consults only `frame_event_count`; `len()` consults `frame_event_count + reader_len`. Under Option A, between two reads, a concurrent writer may bump `frame_event_count` without `reader_len` changing → `is_empty() == false` but `len() == 0` (clamp to `reader_len - start_offset = 0`). Document or fix.

3. **Step 4 `slot_ptr<E>` panics on unregistered** vs **§4.3 returns `Option`**: the plan §4.3 shows `Option<NonNull<...>>` but `init_state` (§6.4) calls `.unwrap_or_else(panic)`. Why expose `Option` at the `pub(crate)` boundary if every caller treats `None` as a panic? Either keep `Option` (and accept the branch in init_state) or panic inside `slot_ptr` (and drop `Option`). Pick one.

4. **`EventWriter::send` declared as `&mut self`** (§6.1 line 436) — what mutation does it actually perform? The state is read-only on the hot path; the slot deref is `&`. If `&self` works, prefer it (lets users call `send` through a shared borrow, useful for callbacks). If it must be `&mut self` for SystemParam aliasing, document why.

---

**Relevant file paths**:
- `D:\claude\BoykoEngine\docs\PHASE-12-EVENTS-SYSTEMPARAM-PLAN.md`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\events\event_buffer.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\events\event_dispatcher.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\system_param.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\tuple_impl.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\commands.rs`
- `D:\claude\BoykoEngine\crates\boyko_threadpool\src\tls.rs`