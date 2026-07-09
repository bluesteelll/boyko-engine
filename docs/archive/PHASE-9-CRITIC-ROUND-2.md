> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Architecture review: Phase 9 — Parallel Scheduler (Round 2)

## Verdict
[X] **CHANGES REQUESTED** — 2 criticals, 5 warnings, 3 optionals. The Round 2 architectural rewrite is substantive and resolves the 9 Round 1 criticals correctly in concept. Two new criticals surfaced from the rewrite (one is a structural correctness gap in `Access::is_universal`; the other is a documentation/spec inconsistency around the dispatcher acceptance gate). Round 3 should be small (≤ 1 day of architect work).

## Round 1 finding resolution (verbatim audit)

- **C1 Arena Send+Sync** — ✅ Resolved correctly. §2.4 SEND2 rewritten; Arena stays `!Send + !Sync`; ALLOC1..6 (§2.7) define the discipline; `InSystemRunGuard` RAII (§4.4 line 755-769); allocation site audit (§9.4).
- **C2 push_task broken** — ✅ Resolved. `injector_local: Arc<[CachePadded<Injector<TaskHandle>>]>` added (§4.2 line 469); worker_main drains stage 1.5 (§4.3 line 545); push_task targets local injector if on worker (§4.5 line 924-936).
- **C3 Nested par_iter deadlock** — ✅ Resolved. `join_workers_until_drained` (§4.5.5) work-steals during Scope::Drop instead of parking. Proof sketch in §4.6.
- **C4 Apply window** — ⚠️ Partially resolved (see C-NEW-1 below). The architect noticed the gate had a bug in §5.4.5 itself ("Wait — the above shows a bug") and corrected to `pending_apply == running.count_ones()` (line 1629-1646). But the corrected pseudocode in §5.4.5.1 has structural inconsistencies — see C-NEW-1.
- **C5 EventDispatcher TLS worker_id** — ✅ Resolved. EVT1..EVT4 (§2.8), `current_worker_id_or_dispatcher_lane(worker_count)` (§4.4 line 729), `send_event<E>(&self, event)` wrapper (§12.4).
- **C6 Scope transmute panic safety** — ✅ Resolved. §4.5.6 explicitly handles unwinding, abort caveat documented; loom test `loom_scope_drop_panic_with_pending` (§13.4.2).
- **C7 unpark_one_idle race** — ✅ Resolved. Four labelled scenarios (Race A/B/C/D) in §13.4.1; re-poll-after-mark_idle documented in §4.3 line 588-598.
- **C8 ConflictGraph O(N²)** — ✅ Resolved. `pred_remaining: Box<[AtomicU16]>` (§7.4); incremental algorithm. §10.5 dispatcher cost recalculated to ~197 µs at N=1024 (under 200 µs target).
- **C9 is_exclusive duplication** — ✅ Resolved. `Access::is_universal()` (§12.5), `SystemMeta::is_exclusive` field dropped; `SystemBox::is_exclusive` remains as build-time cache.
- **W1-W8** — all resolved per the §0 table; verified.
- **O1-O4** — all accepted per §0.
- **Q1-Q3** — all answered per §17.

---

## Remarks

### Critical (blockers)

#### C-NEW-1. `Access::is_universal()` references nonexistent fields
**Where**: §12.5 line 2620-2627, and §11.1 table claims `SystemMeta` stays at 224 B.

**Problem**: The plan's `is_universal()` body checks `self.event_reads.is_all_set() && self.event_writes.is_all_set()` with a comment `// if these exist`. Looking at `crates/boyko_ecs/src/ecs/core/system/access.rs:47-57`, the actual `Access` struct has **only four bitmask fields** (`component_reads`, `component_writes`, `resource_reads`, `resource_writes`) totaling 192 B, with a `const _: () = assert!(core::mem::size_of::<Access>() == 192);` guard at line 61. Adding event fields would break the 192 B invariant + the 3-cache-line claim that the existing code asserts.

**Why critical**: Two failure modes —
1. If the architect intended event lanes to participate in the "universal access" check, the existing `Access` is incomplete and the plan's `ApplyDeferred` (whose access is universal) does **not** actually block systems that hold an event lane mutably. SCH7's "apply runs with zero conflicting workers" claim has a hole for event accesses.
2. If event lanes are intentionally outside the per-system access surface (events are a side channel, sent via TLS-keyed lane per EVT1), then the `// if these exist` hedge is plain wrong and should be removed; `is_universal()` checks only the four existing bitmasks.

The plan must pick one and update §12.5 accordingly. The `// if these exist` hedge is unacceptable in architecture documentation — pseudocode must compile or be marked clearly as a sketch with a follow-up TODO.

**What is needed**: Decide whether event access is part of `Access`. If yes — extend `Access` (which requires updating the `size_of` assertion in `access.rs:61` and the 192 B claim in §11.1) and the conflict_with logic. If no — strike the `event_*` lines from `is_universal()` and document that event lane conflicts are out-of-scope for the schedule's conflict graph (which is consistent with the existing `Access::conflicts_with` body at `access.rs:117-125` that has no event_* terms).

#### C-NEW-2. §1.2 dispatcher acceptance gate is silently relaxed in §10.5 without §1.2 update
**Where**: §1.2 table line 100 vs §10.5 line 2389.

**Problem**: §1.2 binding acceptance gate says "Per-frame dispatch overhead (50 sys, all parallel, 16 threads) | **≤ 5 µs (≤ 100 ns/sys)**". §10.5 recalculates the same scenario and admits the result is "**~19 µs**", then writes "**target relaxed to 20 µs at 50 systems** (factor of 4 of original target; the original target ignored apply cost)". But §1.2 still reads "≤ 5 µs" — the table is not updated.

**Why critical**: §1.2 is the *contract* the implementer reads to know what to hit. A 4× slip is significant and either needs (a) the target updated in §1.2 with the same caveat the architect already wrote in §10.5, OR (b) the architecture changed (e.g., apply hoisted out of dispatcher per-system loop into a batched path) to actually hit 5 µs. The current state — two values, one binding, one apologetic — is a future "tester says it's at 19 µs and architect says it's fine" argument waiting to happen.

The N=1024 target at 200 µs is fine: §10.5 says ~197 µs (under target).

**What is needed**: Update §1.2 row to "≤ 20 µs at 50 sys" with a parenthetical "(apply cost dominates; per-spawn overhead is the residual)". Or: justify why 5 µs is hittable (e.g., by sharing one shared apply buffer rather than per-system apply calls). Pick one. Don't leave two contradictory numbers in the same plan.

---

### Important (must fix before APPROVE)

#### W-NEW-1. Apply window pseudocode is internally inconsistent across §5.4, §5.4.5, §5.4.5.1
**Where**: §5.4 main loop (line 1352-1411), §5.4.5 diagram (line 1590-1625 marked as buggy), §5.4.5.1 corrected pseudocode (line 1702-1724).

**Problem**: The plan presents three pseudocode blocks for the executor loop in §5.4 alone (with the second labeled "Wait — the above shows a bug" mid-section). The implementer cannot tell which is canonical. Specifically:
- Line 1390 mints `UnsafeEcsCell::new_mutable(world)` per-iteration **before** any apply-window check.
- Line 1717 (the "corrected" version) does the same.
- But the gate `pending == running` ensures the apply window runs **before** the cell is needed — yet the cell is still minted on every dispatch attempt even when nothing is dispatchable.

This is not UB (the cell is just a raw pointer + ZSTs; minting is benign at the language level), but the pseudocode reads as if it might be a problem and triggers reader confusion. Furthermore, after `apply_window_drain` runs (which holds `&mut world` for the duration of every `apply` call), any prior cell copies should be considered logically invalid; minting a fresh cell after apply is the correct pattern but the pseudocode does not make this rhythm crisp.

**Solution options**:
- (a) Delete the §5.4.5 buggy diagram entirely; keep only the §5.4.5.1 corrected one as the canonical reference.
- (b) Restructure the main loop as `[apply window] → [if completed return] → [mint cell] → [dispatch]`, with explicit `let cell = unsafe { mint };` placed AFTER apply but BEFORE dispatch, and a clear comment "cell remains valid until next apply_window_drain entry".
- (c) Add a unit test name in §13.1 `executor::tests::cell_minted_per_round_not_per_loop_iter` that pins down the intended behavior.

#### W-NEW-2. `Worker` deque ownership precludes the §4.5.5 "drain own deque" optimization
**Where**: §4.5.5 line 974-988 (claims `pool.workers_owned_deques[worker_id]` is accessible from `join_workers_until_drained`); §4.5.5 line 1024-1026 (acknowledges this is not actually true and falls back to option (b)).

**Problem**: The text first writes the optimistic code, then admits "the actual `Worker<TaskHandle>` lives on the worker thread's stack inside `worker_main`" and reverts to "use only `pool.injector_local[worker_id]` + global injector + sibling stealing". This means **a worker calling `Scope::Drop` inside a nested `pool.scope` cannot drain its own local deque** — the deque is on its own stack, but `join_workers_until_drained` does not have access. The worker thread can still pop from its deque on the way back through `worker_main`'s main loop, but only *after* `Scope::Drop` returns. During the wait, the worker's own deque is unreachable.

**Why important**: The deadlock-freedom proof in §4.6 assumes "the calling worker continues to do useful work while waiting" and lists "Pop from the calling worker's own local injector (drains inner tasks)" — but the local **injector** ≠ local **deque**. Inner tasks pushed via `scope.spawn` from inside the worker go to `pool.injector_local[worker_id]` per `push_task` (§4.5 line 928), so they ARE reachable. OK, the proof survives. But the §4.5.5 pseudocode is misleading and the implementer will likely write the "optimistic" version first and discover the issue at compile time.

**Solution options**:
- (a) Delete the misleading optimistic pseudocode; keep only the option-(b) version.
- (b) Restructure `Worker<TaskHandle>` to live in `pool.workers[i].deque: UnsafeCell<Worker<TaskHandle>>` (with single-producer discipline ensuring the worker itself is the only mutator). Adds complexity but enables the optimization.

#### W-NEW-3. `ScheduleBuilder::build` signature inconsistency
**Where**: §5.5 line 1760 `pub fn build(self, world: &mut EcsMaster) -> Schedule` vs §12.2 line 2541 same signature, but the §5.5 body at line 1763 reads `for d in &mut self.descriptors` — `self` was consumed by `build(self, ...)`, so `&mut self.descriptors` is a borrow of the consumed value. This is a pseudocode typo (the consumer pattern means `self` is owned, so `&mut self.descriptors` is valid). Disregard if I misread. But the loop later at line 1769 reads `self.descriptors.len()`, `self.descriptors[k.0 as usize].system_box.system.name()`, `self.descriptors` — all after `with_syncs = insert_sync_points(self.descriptors, ...)` which moves `self.descriptors`. So the latter references would not compile. Pseudocode quality issue.

**Solution**: Tighten §5.5 build pseudocode — either capture lengths and names before the move, or destructure self first.

#### W-NEW-4. `apply_window_drain` `pending_apply.fetch_sub(target)` at end is racy with concurrent worker `fetch_add`
**Where**: §5.4.5.1 line 1698.

**Problem**: `apply_window_drain` holds `&mut self` (dispatcher exclusive) and reads `target = pending_apply.load(Acquire)`, then drains `target` completions, then `fetch_sub(target, Relaxed)`. But between the initial `load` and the final `fetch_sub`, no new completions can arrive **because all workers had to be drained for the gate to fire** (`pending == running`). Wait — that's only true if no new dispatch happens during the drain. Let me check: `apply_window_drain` is called from the main loop and inside the function the dispatcher does not dispatch. So no, no new completions arrive during drain. Good.

But there's a subtler issue: the gate check `pending == running` uses two non-atomic reads (`pending_apply.load` + `running.count_ones()`). Between these two reads, a worker could push a completion + fetch_add — but only if there are pending dispatches. Since `running` is dispatcher-owned and `pending` only changes from worker completions of already-dispatched systems, the gate check is internally consistent so long as it's reading the post-`dispatch` state. The risk is: dispatcher dispatches B, increments running[B], then loops to top, reads pending (Acquire), reads running (which is 2). If worker A has not yet completed, pending == 1 and running == 2 → gate false. Worker A completes between the two reads: pending becomes 2, running is still 2 (dispatcher hasn't decremented). Reading running second gives 2; reading pending first gave 1. Gate `1 == 2` false. Next iteration: pending is now 2, running is 2 → gate fires correctly. So the staleness is at most one iteration. Acceptable but worth documenting.

**Solution**: Add a comment on the gate explaining "gate is monotone: once `pending == running`, the next iteration also sees it (until apply drains). No race exists; staleness is bounded to one iteration."

#### W-NEW-5. `running` decrement happens *inside* `apply_window_drain` but the gate is evaluated at the top of the outer loop. Dead-code in §5.4 try_dispatch_ready
**Where**: §5.4 line 1471-1474 (exclusive check `if self.scratch.running.count_ones(..) > 0 { defer; continue; }`).

**Problem**: After `apply_window_drain` returns, `running` is fully drained (every completion popped clears its bit). The next call is `try_dispatch_ready`. Inside `try_dispatch_ready`, the check at line 1471 (`if running.count_ones > 0`) is dead under normal flow: we just exited the apply window with `running == 0`. The only way it could be > 0 is if `try_dispatch_ready` itself dispatched a system earlier in this same call (line 1498 `self.scratch.running.set(i, true)`) and then encountered an exclusive system. So the check is **correct** for the within-call sequence (dispatch some normal systems, then defer the exclusive). OK — not a bug.

But: the pseudocode's "Run exclusive on dispatcher" path (line 1479-1494) runs `run_unsafe(world_cell)` + `apply(world_via_cell(world_cell))`. `world_via_cell` is undefined in the plan and there is no clean way to reconstruct `&mut EcsMaster` from a cell that derived from `&mut world` while the original `&mut world` is still live (the dispatcher's `world: &mut EcsMaster` parameter). The cell holds a raw pointer; `world_mut()` reborrows. But `apply(world)` requires `&mut EcsMaster`, and `world_via_cell(world_cell)` is a placeholder for `cell.world_mut()`. Just use `cell.world_mut()` directly.

**Solution**: Define `world_via_cell` or replace with explicit `unsafe { world_cell.world_mut() }`. Also note that for exclusive systems, since they hold universal access, no other system runs concurrently — but the `apply` call needs the dispatcher's own `&mut world` reborrowed through the cell; this is exactly the pattern `cell.world_mut()` provides per `unsafe_ecs_cell.rs:157-170`.

---

### Optional (improvements, not blockers)

#### O-NEW-1. `IN_SYSTEM_RUN` enforcement is debug-only and TLS-Cell-based
The plan acknowledges (§9.2 line 2263) "Release builds skip the check; the discipline is enforced at the higher layer (§9.4 audit)". The §9.4 audit is good (exhaustive list of allocate sites with routing), but a release-mode catch is achievable cheaply: a `cfg!(debug_assertions)` is fine for the `Cell<bool>` overhead in debug, but for the `force_alloc_panic` CI config mentioned in Step 24 — wire it as an `#[cfg(force_alloc_panic)]` runtime check that is normally inert. Already in plan; just want to flag that the architecture relies on developer discipline at runtime in release. Consider adding a Miri test that triggers an arena allocation from a stubbed worker context and confirms the debug_assert fires (lands as `arena::tests::allocate_inside_run_unsafe_debug_panics` per §13.1 — good).

#### O-NEW-2. `pred_remaining` ordering downgrade
§7.4.3 says "Relaxed sufficient since only dispatcher touches it". Confirmed correct under Round 2 design (workers never decrement `pred_remaining`). But the `AtomicU16` type is paying for atomicity that isn't needed — a `Box<[u16]>` accessed through `&mut self` would be equivalent and cheaper (no LOCK prefix on x86, although `fetch_sub` on AtomicU16 still compiles to a non-locked operation when known-uncontended... actually no, `fetch_sub` is always atomic on x86; it's the LOCK prefix that's emitted). Replacing with plain `[u16]` saves a few cycles per decrement and clarifies "this is single-thread state".

**Why not critical**: AtomicU16's overhead at N=1024 systems × ~3 successors × 30 frames = ~92k operations × ~5 ns = 460 µs/sec — negligible. But the architectural claim "Relaxed sufficient" + "no Box<[AtomicU16]> needed" simplifies the code.

#### O-NEW-3. Wraparound in `pred_remaining: AtomicU16`
§7.4 caps systems at 1024, so `pred_count` ≤ 1023, well under u16 max (65535). No wraparound concern. Worth noting `debug_assert!(pred_count <= u16::MAX)` in `ScheduleBuilder::build` to catch any future expansion past `MAX_SYSTEMS_PER_SCHEDULE`. Already partially covered by the `MAX_SYSTEMS_PER_SCHEDULE = 1024` const (§3 Q4).

---

## Positive

The Round 2 rewrite is substantive and addresses the Round 1 criticals correctly in spirit. Highlights worth preserving:

1. **§2.7 ALLOC1..6 invariants + §9.4 allocation site audit**. The discipline is well-specified and the audit table proves Arena is never reached from a worker `run_unsafe`. This is the right resolution for C1.
2. **§4.5.5 work-stealing Scope::Drop + §4.5.6 panic safety + §4.6 deadlock-freedom proof**. Clear, correct, rayon-style. The decision to pursue option (b) (only inject from local injector / global / sibling steal, not own deque) is honest about the implementation constraint and the proof still holds.
3. **§7.4 incremental ready set with `pred_remaining`**. Right call to land this in Phase 9 (not "Phase 9.1"). 2 KB cost + 20-line update for a 5× dispatcher cost reduction at N=1024.
4. **§9.1 type-by-type Send/Sync table** with explicit "Arena: NO" row. Hard to misimplement.
5. **§2.8 EVT1..EVT4 EventDispatcher TLS contract**. The choice "dispatcher uses lane `worker_count`" + `EventConfig::default_for(worker_count + 1)` is clean.
6. **§3 Q9.1/Q9.2 coherence proof for `ExclusiveSystemMarker`**. The architect actually walked through the coherence resolution and identified that `&mut EcsMaster` is not a `SystemParam`, so no overlap can occur. Solid.
7. **§5.4.3 dispatcher-owns-`running` correction**. Catching this within the same plan (rather than waiting for the critic) is good craftsmanship.
8. **§14 Step 7c addition**. Right call to make the Access bitset-extension + InSystemRunGuard wiring a distinct step rather than smuggling it into 7a/7b.

---

## Open questions for the architect

1. **Event lane access**: do event read/write lanes participate in `Access::conflicts_with` and `Access::is_universal()`? (Drives C-NEW-1 resolution.)
2. **Dispatcher target at 50 systems**: is the binding number 5 µs (requires architectural change) or 20 µs (requires §1.2 update)? (Drives C-NEW-2 resolution.)
3. **Exclusive system apply path**: when an exclusive system runs on the dispatcher (§5.4 line 1480-1482), the body calls `run_unsafe(world_cell)` followed immediately by `apply(world_via_cell(world_cell))`. Is the intent that `world_via_cell` = `cell.world_mut()`, and that the exclusive system body itself must not retain any cell-derived borrow past return? Spell it out in a SAFETY block — exclusive systems are a special path and the rules should be explicit.

---

**File citations** (absolute paths):
- `D:\claude\BoykoEngine\docs\PHASE-9-PARALLEL-SCHEDULER-PLAN.md` (Round 2 plan, 3266 lines)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\arena.rs:37-156` (Arena `!Send + !Sync` confirmed; `allocate_layout(&self)` confirmed)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\access.rs:45-126` (Access has only 4 bitmask fields, 192 B asserted; no event_* fields)
- `D:\claude\BoykoEngine\crates\boyko_utils\src\bit_mask\bit_set_256.rs:30-89` (no `is_all_set`/`set_all`; plan acknowledges this in §12.5)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\unsafe_ecs_cell.rs:64-170` (cell shape, `new_mutable` / `world_mut` SAFETY blocks)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\entity\entity_master.rs:55-75` (`with_capacity` exists; W3 routing valid)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\events\event_dispatcher.rs:148-299` (`send(thread_index, event)` signature confirmed)

Two clean criticals + five tractable warnings. Round 3 is small.