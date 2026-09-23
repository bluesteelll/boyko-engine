# Commands id recycling (EM2-prime) - design, critique, implementation, review

- **Source:** workflow `census-gate-and-commands-id-leak` (run `wf_396905fa-b76`), tree `D:/wt/joltab` (merge/ke16-into-ecsnative @ d11962a9)
- **Status:** IMPLEMENTED AND REVIEWED (APPROVED), NOT YET TESTED BY THE TESTER, NOT COMMITTED. The code is uncommitted in D:/wt/joltab. Resume the workflow to run the tester step, then commit.
- **Copied verbatim** on 2026-09-11 at the checkpoint the owner asked for; agent outputs are reproduced without edits.

---

## Design rev 1 (architect)

# Architecture: recycling entity ids on the deferred route (EM2′, the free list becomes a claimable stack)

**Result:** The free list becomes a stack that workers can claim from with one `fetch_sub`. It is recycled on the worker side, not in `CommandQueue::apply`. The census's suggestion (recycle in `apply`, where the dispatcher may pop) cannot work on its own: when `Commands::spawn` returns, `.id()` has already given user code the id, so the id must be recycled at reserve time.

**Evidence and tooling:** Everything below was read in `D:/wt/joltab` at d11962a9. I had no shell, so graphify was not run, and I had no Agent tool, so the `researcher` could not be launched. As a substitute I checked Bevy 0.15 `entity/mod.rs` myself: `Entities::reserve_entity` does `free_cursor.fetch_sub(1, Relaxed)` over a `pending` list and reconciles in `flush()`. The design below comes from that pattern and is adapted to this kernel's constraints.

## Goal
- **Functional:** a flat population churned through `Commands::spawn` / `despawn` keeps `EntityMaster`'s free list and the `entities_inland` slot store bounded. The free list stays at or below the despawns of one window at rest. The slot store stays at or below the peak of (live + reserved-not-yet-applied) + leaked. Handles already returned to user code stay valid. The generation discipline stays in place.
- **Performance:**
  - Steady state costs 0 heap events and 0 VM commits per frame on the entity path. Today it is unbounded: the census measured +131,072 over 2048 frames.
  - A worker claim costs one locked RMW plus two L1 loads when a recycled id is available, and two locked RMWs on one line it already owns when minting fresh.
  - Workers never read `entities_inland` (EM3 is kept word for word).
  - Dispatcher push and pop use plain instructions with no `lock` prefix.

## Context and constraints (verified)
- **The defect path:**
  - `Commands::spawn` calls `EntityCounter::reserve_entity`, which only does `fetch_add` (`commands.rs:164-168`, `entity_counter.rs:148-161`). `EntityMaster::reserve_entity` does the same (`entity_master.rs:186-190`).
  - A despawn goes `deallocate_entity` → `free_entity_ids.push` (`entity_master.rs:378`).
  - The only pop is the dispatcher's `allocate_entity` (`entity_master.rs:125`).
  - `reserve_entity_skips_free_list` (`entity_master.rs:785-798`) pins the defect's cause.
- **A second leak route of the same class:** `DeferredCommands::spawn` (`deferred_master.rs:193-203`) mints fresh through `next_id_atomic()` even though it holds the world exclusively.
- **Adjacent defect, same class:** in `create_entity`'s rejection path (`entity_api.rs:216-222`), a recycled id is leaked. The comment says it "returns to the free list", but `deallocate_entity` returns `false` for a slot that was never registered (`entity_master.rs:361-363`), so it does not. The comment is false.
- **Load-bearing premise, SCH7:** `apply_window_drain` runs only when `pending == running || running == 0` (`schedule.rs:667-679`). The exclusive path runs only when `running == 0` (`schedule.rs:1099`). So `&mut EntityMaster` never overlaps a worker phase.
  - KE17's future split window keeps the barrier for systems that carry `Commands` (`docs/MEASUREMENT-QUEUE.md:88-89`), so this design is compatible with it.
- **Precedent for giving workers a plain, non-atomic store:** `UnsafeEcsCell::entities` already hands workers `*const InlandStore` under SCH7 (`unsafe_ecs_cell.rs:291-313`).
- **Invariants that must hold:**
  - Generations: the bump happens in `deallocate_entity` (`entity_master.rs:370-376`), and registration writes `entity.generation()` (`entity_master.rs:343-347`).
  - EM1–EM6 (`docs/archive/PHASE-11-ENTITY-COMMANDS-PLAN.md:128-135`).
  - The `Commands` layout is 16 B (`commands.rs:117`) and `EntityCounter` is 8 B (`entity_counter.rs:107`).
  - The X.G hot pair sits at offset 0 of `EntityMaster` (`entity_master.rs:37-55`).
- **Owner constraints:** the new structure goes on `VmColumn` (`vm_column.rs:80`), not on `Vec`; lock-free.

## Key decisions

### D1 — The free list *is* the claimable stack
**What:** `free: VmColumn<Entity>` is a LIFO stack whose logical length is an `AtomicIsize free_top`.
- Workers claim with `r = free_top.fetch_sub(1, Relaxed)`. If `r > 0`, the handle is `free[r-1]`. Otherwise they mint fresh.
- The dispatcher mutates the stack's structure only under `&mut`.

**Why:**
- It adds no refill step and no second structure. The dispatcher does zero work per window beyond one clamp.
- Any system can consume any free id, so no demand prediction is needed.
- There is no ABA problem. During a phase the stack is pop-only: nothing pushes concurrently, and entries are immutable. `fetch_sub` returns each positive value at most once.
- One RMW per claim is the same cost as today's `fetch_add`.

**Alternatives rejected:**
- **(a) as proposed, a separate ring refilled each apply window:**
  - The refill copy is O(k) per window, and it needs a policy for how many ids to move. If the answer is "all", the ring *is* the free list.
  - The dispatcher's `allocate_entity` would have to pop from two places.
- **(b) a per-worker or per-system pre-reserved batch:**
  - Per worker: a system runs on whatever worker steals it, so every claim would need a TLS lookup. On this toolchain (rustc ≥1.98 windows-gnu) `thread_local!` costs 2 locked RMWs plus `FlsSetValue`, which is more than the single `fetch_sub` it would save.
  - Per system (a stash on the `CommandQueue`):
    - The stash hoards ids. When demand moves between systems, the idle system's stash sits full while the other mints fresh ids, and repeated shifts grow the slot store without bound.
    - `CommandQueue::drop` has no world access, so a stash is leaked on drop. `run_system_once` builds a fresh queue on every call.
    - It adds a field to the one-line, 56 B `CommandQueue` (O2).
- **(c) remapping reserved ids at apply time:**
  - `.id()` returns before apply. The handle then escapes into other queues' commands (`add_child`, `ChildOf(e)`), resources, events, and `Local`s kept across frames. There is no registry of those copies.
  - The only way to "remap" is an indirection table consulted on every `is_entity_valid` / `get_component_raw`, which adds a dependent load to the hottest random-access path. The table is also unbounded, because escaped copies live forever.
  - It breaks the rule that the handle a worker got stays valid. Dead.
- **(d) an intrusive free list through the inland slots (EnTT style):**
  - Popping is a CAS retry loop that storms under contention.
  - Each claim is a pointer chase with a dependent random miss.
  - Workers would read and write `entities_inland`, which breaks EM3.
  - It needs ids that fit in `u32`.
- **(e) Bevy's single-atomic negative cursor (fresh id = base − r):** fresh singles would then come from `free_top`'s negative side, but `spawn_batch` needs a contiguous fresh range from `next_entity_id` (`spawn_batch_command.rs:694-699`, `register_batch` at `entity_master.rs:263-307`). The two would collide unless batch also goes through the cursor, which couples this change to Stage B (D7). The gain is one locked op on an owned line for fresh singles only. Rejected.

**Trade-off:**
- The fresh-mint path costs 2 RMWs instead of 1. The second RMW hits a line this core already owns exclusively.
- `free_top` can go transiently negative, and each `&mut` path pays one clamp.

### D2 — Entries carry the generation (`Entity`, 16 B), not a bare `EntityId`
**Why:**
- The worker's claim reads the entry sequentially (LIFO-adjacent, 4 per line) instead of doing a random `inland[id].generation()` load. That saves roughly one miss per claim.
- EM3 ("reserve does not touch `entities_inland`") keeps holding word for word.
- The dispatcher already has `next_gen` in a register at `entity_master.rs:371`.

**Alternative rejected:** a packed 8 B `(u32 id, u32 gen)`. It adds an id < 2³² invariant that contradicts `InlandStore::with_capacity`'s "never refuse a satisfiable request" (`inland_store.rs:129-143`).

**Trade-off:** 2× bytes per free entry. The free list is bounded by one window's despawns, so this is negligible.

### D3 — Settle lazily inside every `&mut` method; no end-of-window hook
**What:** `settle()` sets `top = max(free_top, 0)`, then `free.truncate(top)`, then `free_top = top`. It is plain code via `get_mut`, branch-free (`max`). Every `&mut` method that touches the stack calls it first.

**Why:**
- It is correct no matter how many apply sites exist: `apply_window_drain`, the exclusive inline path, `run_system_once`, the deferred-hook drain, and exclusive systems that mix `Commands` claims with `&mut` world calls.
- Claimed entries above `top` were already copied into their handles, so overwriting them is harmless.

**Alternative rejected:** Bevy's mandatory `flush()` points (`verify_flushed`). They need a hook at every site that mutates the world, which is fragile: a single missed site leads to double issue.

### D4 — The claim is ungated: `fetch_sub`, then `fetch_add` as the fallback
**Why:**
- The target steady state (flat churn) always takes the recycled path, where a gate would add a load.
- On the fresh path the second RMW hits the same line the first one made exclusive (D5), so there is no extra coherence transaction.
- The loop body has one branch (principle 7).

The measurement is listed under validation. It is not an open fork.

### D5 — The reservoir sits on its own 64 B line, holding both atomics
**What:** `EntityMaster { entities_inland @0, live_count @48, reservoir @64 (align 64) }`.

**Why:**
- Today `next_entity_id` shares line 0 with `entities_inland.base/len` (`entity_master.rs:37-42`). Every worker-side spawn's `lock xadd` therefore invalidates the header that every other worker's `get_component_raw` and `Entities::get` read, which is false sharing on the parallel read path. Moving both RMW'd atomics off line 0 removes it.
- The hot pair keeps its offset 0, so the XG-B1 displacement is unchanged.

**Trade-off:**
- A dispatcher fresh `allocate_entity` now touches 2 L1-hot lines instead of 1. X.G measured this family as sensitive (+6–10% from a field shuffle, `entity_master.rs:41-42`).
- The recycled and delete paths already touched two lines plus the entry line, so they are unchanged.
- `EntityMaster` goes from align 8 to align 64 (size 192), and `EcsMaster` inherits align 64.

### D6 — The free list lives on `VmColumn<Entity>`, reserved to the inland ceiling
**Why:**
- It follows the owner's allocator rule. The base address is write-once, so a worker's cached `base` can never dangle mid-phase.
- Free entries are distinct ids (F3), each below `inland.len()`, so the free list's length can never exceed the inland ceiling. Pushing past the reservation is therefore structurally impossible.
- The first commit is lazy (`POOL_MIN_SLAB` = 64 KiB, 4096 entries), so `EcsMaster::new` makes no syscall (XG-B4 stays green).
- `with_capacity` precommits `capacity/4` entries, the same as today's `Vec::with_capacity(capacity/4)` (`entity_master.rs:101`).

**Trade-off:** under Miri and wasm, the fallback eagerly zero-allocates 16 MiB per world on its first despawn, the same as `InlandStore` (`constants.rs:378-379`).

### D7 — `spawn_batch` stays fresh-and-contiguous in Stage A; Stage B is designed now
**Why:** batch contiguity is load-bearing across `register_batch`, the `EntityId(start_id + i)` arithmetic (`spawn_batch_command.rs:697-699`, `:722`), and the `SpawnBatchIter` range.

No per-frame path in the workspace calls `Commands::spawn_batch`; the grep found only benches, setup, and `app.rs:1199`. So Stage A closes the measured defect, and a pure-batch churn stays a stated residual.

**Stage B sketch, compatible with this reservoir:**
- A batch does `r = free_top.fetch_sub(n)`. Its recycled part is `free[max(r−n,0) .. max(r,0))`, which is immutable during the phase and is copied *at claim time* as a trailing payload behind the `SpawnBatchCommand` in the same byte arena. The command's glue advances the cursor by `size_of::<C>() + k·16`.
- The deficit `n−k` comes from `next_entity_id.fetch_add(n−k)`, which keeps the fresh part contiguous.
- Apply splits into a scatter-register of k entries plus the existing contiguous `register_batch` of `n−k`.
- No new storage structure is needed.

### D8 — Every deferred single-spawn route goes through one claim path
`EntityMaster::reserve_entity(&self)` becomes the claim. It serves `EntityCounter` (workers) and `DeferredCommands::spawn` (hooks, on the dispatcher). `rewind_allocate` gains a recycled-id arm (`unpop`), which closes the `entity_api.rs:216-222` leak.

## Data structures
```rust
/// Worker-reachable id source (EM6′). Line 0 = all worker-touched state.
#[repr(C, align(64))]
pub(crate) struct EntityReservoir {
    /// Claimable prefix length of `free`. Workers: fetch_sub(1, Relaxed).
    /// Can go negative within a phase (one step per claim that found it <= 0).
    /// Dispatcher: plain get_mut; clamped to >= 0 by `settle`. AUTHORITATIVE.
    free_top: AtomicIsize,                 // +0
    /// Fresh-id counter, moved verbatim from EntityMaster (EM1). Monotone
    /// except `rewind_allocate` under &mut.
    next_entity_id: AtomicUsize,           // +8
    /// Recycled entities, LIFO, each carrying the generation deallocate wrote.
    /// `free.len()` = physical top: == free_top at rest, >= free_top in a phase.
    /// Only `free.base` (+16) is read by workers; len/committed/... are
    /// dispatcher-only and written only in windows.
    free: VmColumn<Entity>,                // +16 .. +88
}   // size 128, align 64; const-assert offset_of!(free) == 16

#[repr(C)]
pub struct EntityMaster {
    pub(crate) entities_inland: InlandStore, // +0  (48 B) unchanged: XG-B1 hot pair
    live_count: usize,                       // +48 dispatcher-only
    pub(crate) reservoir: EntityReservoir,   // +64 own line (D5)
}   // size 192, align 64

pub struct EntityCounter<'s> {               // stays 8 B, Commands stays 16 B
    reservoir: *const EntityReservoir,
    _marker: PhantomData<&'s EntityReservoir>,
}
```
- `unsafe impl Send + Sync for EntityReservoir`. The argument: the atomics are RMW'd from any thread. `free.base` and the entries are written only under `&mut` (SCH7), and workers read only indices below the claim value they received.
- Under `cfg(loom)`, the atomics are aliased to `loom::sync::atomic`, following the `term_list.rs:114-121` precedent. `get_mut` becomes `with_mut`.

## Invariants (restated)

| ID | Statement |
|---|---|
| EM1′ | Fresh ids come from `reservoir.next_entity_id.fetch_add`, generation 0. Recycled claims carry the entry's generation. |
| EM2′ | The free list's structure (push, truncate/settle, sort, clear) changes only under `&mut EntityMaster` (dispatcher, SCH7). In a phase it is an immutable array. Workers remove entries only through `fetch_sub` on `free_top`. The generation bump still happens only on the dispatcher, in `deallocate_entity`; workers only copy it. |
| EM3 | Unchanged: a claim never touches `entities_inland`. |
| EM4′ | Every id handed out is in exactly one of {free prefix, live, claimed-pending, leaked}. The fresh counter is monotone. The old "fresh > max(free)" property is **dropped** (`PHASE-11-ENTITY-COMMANDS-PLAN.md:133`). |
| EM5 | Unchanged: claims never resize anything. |
| EM6′ | Workers reach exactly `*const EntityReservoir`, meaning two atomics (RMW) and `free.base` plus entries (read-only). No `EntityMaster` field is reachable. This is type-enforced as before. |
| F1 | After settle, `free_top == free.len()`. |
| F2 | Always `free_top <= free.len()`. In a phase `free.len()` is constant and `free_top` only decreases. |
| F3 | For each `e` in `free[0..top)`: `inland[e.id].is_null()` and `inland[e.id].generation() == e.generation()`. All ids are distinct. |

**Collision cases:**
- Worker claim vs worker claim: distinct `fetch_sub` values give distinct indices, and F3 makes those distinct ids.
- Recycled vs fresh: a recycled id is below the `next_entity_id` it was minted from, and a fresh id is at least that.
- Claim vs a later dispatcher pop: settle drops claimed indices from the prefix before any pop.
- Stale vs new handle: the generation compare.
- Claim vs dispatcher push: impossible under SCH7.

## Public API delta (no user-visible signature change)
```rust
impl EntityReservoir {                         // crate-internal
    pub(crate) fn new(free_reserve_elems: usize) -> Self;
    #[inline] pub(crate) fn claim(&self) -> Entity;                               // worker-safe
    #[inline] pub(crate) fn mint_fresh_batch(&self, n: usize) -> EcsResult<Range<usize>>;
    #[inline] pub(crate) fn claimable(&self) -> usize;                            // max(load,0)
    #[inline] pub(crate) fn next_fresh(&self) -> usize;
    #[inline] pub(crate) fn settle(&mut self) -> usize;
    #[inline] pub(crate) fn push_free(&mut self, e: Entity);
    #[inline] pub(crate) fn pop_free(&mut self) -> Option<Entity>;
    #[cold]   pub(crate) fn unpop(&mut self, e: Entity);                          // rewind arm
    pub(crate) fn clear(&mut self);
    pub(crate) fn sort_free_low_ids_first(&mut self);                             // compact()
}
unsafe fn EntityCounter::from_ptr(p: *const EntityReservoir) -> Self;           // was *const AtomicUsize
fn VmColumn::precommit(&mut self, n: usize);   // #[cold]; grows the commit frontier, len unchanged
```
- `EntityMaster::recycled_entity_count(&self)` returns `claimable()`, which equals the list length at rest.
- `Commands::spawn`, `spawn_empty`, `clone_and_spawn*` and `reserve_entity` docs change to: "the returned `Entity` may reuse an id; its generation may be > 0."

## Algorithms for the critical paths
| Op | Steps | O | Cache | Branches |
|---|---|---|---|---|
| `claim` (worker) | `r=fetch_sub(1)`; `r>0` → read `free.base[r-1]`; otherwise `fetch_add(1)` → `Entity(id,0)` | O(1) | reservoir line (RFO if contended) plus the entry line (1 miss per 4 claims) | 1 |
| `settle` | `top=max(t,0)`; `truncate`; store | O(1) | reservoir line, L1 | 0 (`max`) |
| `deallocate_entity` | as today, then `push_free(Entity(id, g+1))` (settle, write at `top`, len++, top++) | O(1) | inland slot, reservoir line, entry line | 1 (commit frontier, cold) |
| `allocate_entity` | `pop_free()` → debug-check F3 → return; otherwise fresh plus `ensure` (as today) | O(1) | inland line and reservoir line; the entry is L1 (just pushed) | 1 |
| `rewind_allocate` | fresh: as today (`entity_master.rs:532-543`). Recycled: `unpop(e)`, where a release `assert!` checks the slot is null and gen matches (cold) and a debug check confirms `e` is the entry physically at `top` | O(1) | — | cold |

SIMD: N/A (scalar id bookkeeping).

## Generations, and what a stale handle sees
- **Rule:** `dealloc` writes `slot = {null, 0, g+1}` and pushes `Entity(X, g+1)`. A claim or pop returns `Entity(X, g+1)` verbatim. `SpawnAtCommand` apply registers `g+1` (`spawn_at_command.rs:350-352` → `entity_master.rs:343-347`).
- **Stale `(X, g)` before the successor applies:** the slot is null, so `is_entity_valid` is false and Insert/Remove/Despawn are no-ops (EC8).
- **Stale `(X, g)` after the successor applies:** the generation mismatch rejects it everywhere. The bare-id `Entities::get(X)` resolves to the current occupant, as designed (`tests/ke2_entities_param.rs:206`).
- **The new handle `(X, g+1)` between reserve and apply:** invalid, exactly like a fresh reserved id today. After apply it is valid. If the queue panics and the command is re-absorbed and applied a window later, X is still in no list, so no other claimant can get it.
- **Wrap:** under per-frame LIFO churn one slot wraps after 2³² reuses (about 2.3 years at 60 Hz). This is pre-existing: the dispatcher path already reuses LIFO, as the census C7 control shows.

## Ordering with the apply window
1. **Phase P (workers):** claims return entries from `free[0..top_P)` or fresh ids. Entries freed in this phase cannot be claimed, because they are still live.
2. **Gate:** Acquire on `pending` (`schedule.rs:667`) synchronizes with every worker's completion Release, so the dispatcher sees the final `free_top`.
3. **Window W (dispatcher, `&mut`):** the first `EntityMaster` `&mut` op settles. Commands then apply in completion order: despawns push, spawns register the claimed handles. Hook spawns (`DeferredCommands`) claim through the same atomics, and the next `&mut` op absorbs them.
4. **Phase P+1:** ids freed in W are claimable. Reuse latency is one round.

## Determinism
- **Deterministic case:** within one system invocation, the k-th claim returns `free[top_P − k]`, and after exhaustion `next_entity_id` +0, +1, and so on. With one claiming system per round and a fixed apply order, the same op sequence gives the same ids. Test T3 pins this.
- **Non-deterministic cases:**
  - Concurrent claimers interleave. This is the same as today's concurrent `fetch_add`.
  - **New dependency:** the free-list order depends on the apply order of despawning systems. That order is completion order within a round (`schedule.rs:781`), which is non-deterministic when two despawners share a round. Today, a lone spawner gets deterministic fresh ids even next to concurrent despawners; after this change it does not.
  - This is a narrow regression. Bevy has the same property.
- **The fix, if the owner wants replay determinism:** drain a round's completions in system-index order. That is a scheduler change and outside this scope (open question 1).

## Multithreading model and data-race freedom
- **Model:**
  - Multi-writer on the atomics.
  - Single writer on `free` / `base` (dispatcher) and many readers (workers), separated in time by SCH7.
- **Happens-before chain for the plain memory `free.base` and the entries:** dispatcher writes → the pool's task publication (Release/Acquire in the KE16 deque) → worker reads → the worker's completion `fetch_add(Release)` → the dispatcher's Acquire at `schedule.rs:667` → dispatcher writes. Every write→read and read→write pair is ordered.
- **No read of uninitialized or out-of-range memory:** a worker reads index `r−1 < r ≤ top_P ≤ free.len()` (F2), so it never reads above `len`.
- **No double issue:** `fetch_sub` is an RMW on a single location. Its total modification order gives each positive value at most once while `free_top` is only decreasing, which it is during a phase.
- **Relaxed is enough everywhere:** uniqueness comes from the RMW, and visibility comes from the SCH7 edges above. This is the same argument as `PHASE-11-ENTITY-COMMANDS-PLAN.md:250-256`.
- **Provenance:** derive the reservoir pointer with `&raw const (*self.ptr).entity_master.reservoir` inside `UnsafeEcsCell::entity_counter` (`unsafe_ecs_cell.rs:280-281`). Today's code goes through `&EntityMaster`; the raw derivation keeps the worker's pointer a raw child of the write-capable root under Tree Borrows. The dispatcher's `&mut` in the window invalidates it only after the phase.
- **Pre-existing aliasing class (not new):** an exclusive system that holds both `Commands` and `&mut EcsMaster` already has it with today's `*const AtomicUsize`.

## Miri and loom
- **Miri:**
  - Unit tests on a reservoir built with a small `free_reserve_elems`, so the fallback allocates KiB, not 16 MiB. Three `std::thread::scope` threads × 8 claims over 12 pre-filled entries. The data-race detector checks the plain entry reads against the dispatcher writes across spawn/join.
  - Run with the repo's full `MIRIFLAGS` on `+nightly-x86_64-pc-windows-gnu`.
  - The existing `tests/miri_entity_store.rs`, `miri_phase19.rs`, `miri_phase22.rs` and `miri_feature2_observers.rs` must stay green. They now exercise deferred recycling.
- **loom:** add `tests/loom_entity_reservoir.rs` (`#![cfg(loom)]`), reaching the reservoir through a `#[doc(hidden)] pub` hook (precedent: `term_list.rs:438-444`).
  - Model 1: 2 claimers over 1 entry. Exactly one gets the recycled id, one gets fresh, and after settle `free_top == 0 == free.len()` and `next == base+1`.
  - Model 2: 3 claimers over 2 entries.
  - loom checks the atomic protocol (uniqueness, counts, settle arithmetic). The plain entry memory is not `loom::cell`-tracked, so its ordering is covered by Miri.

## Integration and implementation plan (in order; RED first)
0. **RED-first, on the unfixed tree (d11962a9, before any step below):** add `crates/boyko_ecs/tests/em_deferred_recycle.rs` with T-RED-1 (spec below). Run it and record the failure message. Verify ancestry with `merge-base --is-ancestor` before quoting the result.
1. `memory/vm_column.rs`:
   - add `precommit(n)`;
   - make `committed_elems` non-test (diagnostics);
   - drop `#[allow(dead_code)]` on `as_ptr` (`:331`).
2. **New** `core/entity/entity_reservoir.rs`: the struct, methods, Send/Sync SAFETY, `cfg(loom)` aliases, unit tests, and the doc-hidden hook. Register it in `core/entity/mod.rs`.
3. `core/entity/entity_master.rs`:
   - replace `next_entity_id` and `free_entity_ids` (`:64`, `:73`, `:85`, `:101`);
   - rewrite `allocate_entity` (`:124-148`), `reserve_entity` (`:186-190`, the claim; remove `dead_code`), `reserve_batch` (`:211-224`), `deallocate_entity` push (`:378`), `recycled_entity_count` (`:428-430`), `next_entity_id` (`:437-439`), `clear` (`:459-464`), `memory_usage` (`:476-479`), `compact` (`:489-500`, sort only, no shrink), and `rewind_allocate` (`:520-548`, plus the recycled arm);
   - update the SEND5 comment (`:558-587`);
   - invert `reserve_entity_skips_free_list` (`:785-798`).
4. `system/params/entity_counter.rs`: pointer type, `reserve_entity` → `claim`, `reserve_batch` → `mint_fresh_batch`; restate EM1′/EM2′/EM4′/EM6′ in the docs (`:1-30`, `:129-161`); update the tests.
5. `system/unsafe_ecs_cell.rs:268-289`: project with `&raw const … .reservoir`.
6. `component/hooks/deferred_master.rs:193-203`: replace the inline `fetch_add` with `world.entity_master.reserve_entity()`.
7. `ecs_master/entity_api.rs:216-222`: the rewind now succeeds for recycled ids. Correct the false comment.
8. `commands/spawn_at_command.rs:129-137`: add `debug_assert` that `slot.generation() == entity.generation()` when the slot exists (this catches D2 or F3 corruption).
9. `commands/command_queue.rs`:
   - KE7 docs (`:226-242`, `:1381-1388`): the reason ids are not reclaimed is *escape*, not EM2.
   - Add a sibling test with a pre-populated free list: the claimed recycled id is not re-issued after `rewind`.
10. Doc strings: `commands.rs:129-168` / `:232-244`, `system/params/entities.rs:20`.
11. Census re-derivation, counts only (step 13 of the checklist below).
12. `docs/SYSTEMS.md`, `FEATURE_MAP.md` and `ARCHITECTURE.md` mention EM2 / `free_entity_ids`. The book page `book/src/architecture/entities-and-generations.md` must go to `doc-writer`.

## Metrics and validation

**Tests on the RED-first harness:**
- **T-RED-1 `commands_churn_recycles_ids_on_the_deferred_route`.** Harness: `App::with_pool(pool(2))` in the census's shape (`alloc_frame_census.rs:1525-1560`), 1024 static rows, C = 64, 4 warm-up frames, 256 measured frames. Per-frame samples go into a preallocated test resource.
  - (a) Anti-vacuity: spawned = (W+F)·C, despawned ≥ spawned − 2C, live population within ±C.
  - (b) `max(recycled_entity_count()) ≤ C`.
  - (c) `capacity()` does not change across the window, and ≤ live + 2C.
  - (d) `committed_slots()` does not change.
  - (e) Anti-vacuity for the fix: the id set spawned in frame k equals the id set despawned in window k−1, and at least one handle has generation ≥ 1.
  - (f) Every `.id()` from frame k is valid after update k and reads `wave == k`; wave k−1 handles are invalid after frame k.
  - (g) A stale handle to a recycled id stays invalid after its successor registers.
  - On d11962a9, (b), (c) and (e) fail.
- **T2:** two unordered concurrent spawners plus one despawner, `pool(4)`. Bounded, no duplicate live ids (test-only `HashSet`), every handle valid.
- **T3:** determinism. Two fresh apps, one claimer, 64 frames: identical `Entity` sequences.
- **T4:** `DeferredCommands::spawn` recycles. An `on_despawn` hook spawns a replacement over 64 frames, and `capacity()` stays flat.
- **T5:** a `reserve_entity` escape without spawning leaks exactly that id, and the id is never re-issued.
- **T6:** `spawn_batch_churn_is_bounded`, marked `#[ignore = "deferred: Stage B batch claims (D7)"]`.

**Unit tests (reservoir and master):**
- `reserve_entity_claims_free_list_before_minting`
- `overshoot_clamps_at_settle`
- `claim_then_dispatcher_push_does_not_reissue_claimed`
- `rewind_allocate_returns_recycled_id`
- 8 threads × 1000 claims over 4000 pre-filled entries: exactly 4000 recycled (with their generations) plus 4000 fresh, 8000 unique.
- A proptest over random {claim, allocate, dealloc(live), register(claimed)} against a model, checking F1–F4 after every op (`#[cfg_attr(miri, ignore = "miri-slow: …")]`).

**Existing EM tests:**
- Kept: `test_entity_allocation_fresh`, `deallocate_then_allocate_recycles_id_with_bumped_generation`, `reserve_entity_shares_counter_with_allocate_entity` (the free list is empty in it), `atomic_counter_advances_on_fresh_allocation`, `deallocate_entity_rejects_stale_generation_handle`, `deallocate_unregistered_recycled_id_is_noop_and_preserves_live_count`, `live_count_equals_non_null_inland_count_after_churn`, `xg_b6_slot_address_stable_across_growth`.
- Adapted: `entity_counter_*` (the constructor changes).

**Gates that must be able to fail (mutations):**
- M1: `claim` always mints (the pre-fix behaviour) → T-RED-1 (b), (c), (e) red.
- M2: skip the clamp → proptest or a debug assert red.
- M3: settle without `truncate` → proptest red (a claimed id is re-issued).
- M4: entries store generation 0 → the new SpawnAt `debug_assert` and T-RED-1 (g) red.
- Restore each mutation with plain `cp`, then `touch` the file.

**`debug_assert!` sites:** F2 in `settle` and in `claim` when `r > 0`; F3 in `push_free`, `pop_free` and `unpop`; the generation check in `SpawnAtCommand::apply`; a test-only `EntityMaster::check_invariants()` that walks F1–F4 in O(n).

**Census re-derivation (counts only; derive, never widen):**
- `alloc_frame_census.rs:1874-1885`: `realloc_sum: 2` → **0**. It is a structural zero: at rest the free list holds ≤ 64 entries on a `VmColumn`, not a `Vec`.
- Re-measure `dispatch_max: 6` (`:1872`). Its +1 headroom was justified by the free-list doubling (`:1864-1866`).
- Update the census module docs at `:81`, `:93`, `:118`, `:136`, `:177`.
- `alloc_frame_attribution.rs`:
  - Invert C6 (`:1575-1581`) to `recycled_after ≤ recycled_before + CHURN_PER_FRAME` and `slots_after == slots_before`.
  - Reword C7's message premise (`:1621-1627`).
  - Change the verdict row (`:2691-2701`) to FREE.
  - Update the module doc (`:107-114`).
- Re-run the setup pins of the scenes that despawn, since one heap `Vec` allocation disappears from setup.

**Benchmarks — to run only when the owner says the machine is quiet; no verdict until then:**
- `create/delete_entity_10k` (the X.G layout sensitivity, D5);
- worker `Commands::spawn` (`profile_spawn_single`, `comparison_v2`);
- parallel `get_component_raw` with a concurrent spawner (the false sharing D5 removes);
- a gated vs ungated claim run (D4).

## Open questions
1. **Owner values call:** must entity ids be replay-deterministic when despawning systems run concurrently? If yes, the fix is draining completions in system-index order in `apply_window_drain` (`schedule.rs:769-835`), which is outside this scope.
2. Schedule Stage B (D7) now, for kernel completeness, or after this lands? No per-frame caller exists today.
3. `Commands::reserve_entity` without a spawn still leaks one id per call (the current contract, `commands.rs:238-240`). Bevy instead flushes claimed-but-unregistered ids into live empty entities. Adopting that would change `reserve_entity`'s semantics, so it is the owner's call.
4. Generation wrap under LIFO (about 2.3 years of per-frame reuse of one slot) is pre-existing and flagged here, not changed.

## Checklist
- **Goal, metrics, justifications, alternatives, trade-offs:** ✓
- **Layouts:** sizes and offsets given, `repr` and alignment stated.
- **Hot/cold split:** worker line vs dispatcher fields ✓.
- **API:** minimal, crate-internal, and the user-facing signatures are unchanged.
- **Multithreading model, orderings, happens-before proof:** ✓
- **Send/Sync:** stated.
- **Edge cases:**
  - an empty free list gives `r ≤ 0` and a fresh id;
  - overshoot is clamped;
  - `isize` cannot underflow (it would take 2⁶³ claims per phase);
  - `clear()` with pending queues is pre-existing and unchanged.
- **Generations and drop:** the generation rule is ✓. Drop order: `VmColumn<Entity>` is `Copy` with no element drop, and the reservation is released on drop.
- **Integration steps:** ✓
- **Tests, property tests, loom, Miri, debug asserts, benches (deferred):** ✓

---

## Critique (architecture-critic)

{
 "verdict": "CHANGES REQUESTED: one blocker. The core protocol holds up against the code in D:/wt/joltab. The free list becomes a claimable stack: workers claim with `fetch_sub`, it is pop-only during a phase, and the dispatcher settles lazily under `&mut`. I traced it against SCH7 (schedule.rs:667-679 drain gate, :1099 exclusive gate), the hook path (DeferredCommands::spawn, deferred_master.rs:193-203), and delete_entity, whose hooks fire before deallocate_entity (entity_api.rs:1047-1099). All other hooks fire only after allocate/register. It is sound on all three: no double issue, no ABA, and the Relaxed orderings are fine because SCH7's Acquire/Release edges supply the happens-before. Generations are carried verbatim into registration: every single-row path registers `entity.generation()` (spawn_at_command.rs:352, materialize.rs:554, prefab.rs:786, migration_helpers.rs). EM3 and EM5 still hold. A handle a worker gets from `Commands::spawn` stays valid. T-RED-1 can fail for the right reason: on d11962a9, (b) fails within two frames because `free_entity_ids.len()` grows by 64 a frame. The blocker is in the `rewind_allocate` redesign, where the plan's own wording keeps a check that produces generation ABA. Positives to keep:\n- D2: generation-carrying entries keep EM3 word-for-word and avoid Bevy's random `meta[index].generation` load.\n- D3: lazy settle is correct at every `&mut` site. I checked that no hook fires between pop and rewind (entity_api.rs:183-216).\n- D5: moves both RMW'd atomics off the inland header line.\n- D6: VmColumn with its write-once base.\n- It catches the false 'returns to the free list' comment at entity_api.rs:216-222, and the census setup-pin fallout.\n- The mutation gates M1 and M3 are well chosen.\n\nSections walked: goal, D1-D8, data structures, invariants, API, algorithms, generations, ordering, determinism, MT model, Miri/loom, integration, validation. Topics absent without consequence: SIMD (scalar bookkeeping), non-temporal stores, prefetch (LIFO entries are sequential), PGO.",
 "blocking": [
  "C1 - `rewind_allocate`'s 'fresh: as today' arm swallows the new recycled arm and causes generation ABA. WHERE: Algorithms table row `rewind_allocate` ('fresh: as today (entity_master.rs:532-543). Recycled: unpop(e)'), D8, plan step 3/7. PROBLEM: today's fresh-arm check is `id.0 + 1 == next_entity_id && id.0 < len` (entity_master.rs:533). A RECYCLED id satisfies it whenever it is the most recently minted fresh id. Traced on the public API: `create_entity` mints E=(N-1,0); `delete_entity(E)` pushes (N-1,1); `create_entity(arch, components missing a signature id)` makes allocate_entity pop (N-1,1). Archetype::create_entity then returns false (archetype.rs:1117-1118), and rewind_allocate takes the FRESH arm (entity_api.rs:216) and rolls next_entity_id back to N-1. The id is now in no list. The next create_entity mints (N-1,0) fresh, and register_entity_with_ptr overwrites the slot's generation 1 with 0 (entity_master.rs:343-347). The stale handle E=(N-1,0) is VALID again and aliases the new entity. CONSEQUENCE: generation ABA in release builds, reachable from safe public API (a bad-component create_entity right after despawning the newest entity), in any build of this plan. The new `unpop` arm is unreachable in exactly that case. The step-8 SpawnAt debug_assert does not cover the create_entity path, and the planned `rewind_allocate_returns_recycled_id` unit test passes with the bug unless it builds the id == next-1 case. CONFIDENCE: CONFIRMED (entity_master.rs:520-548, entity_api.rs:183/216-222, archetype.rs:1117). This is pre-existing, but the plan takes the function into scope ('closes the entity_api.rs:216-222 leak') and the literal instruction reproduces it. WHAT IS NEEDED: rewind must choose the fresh or recycled arm from what allocate_entity actually did, not from id arithmetic. The plan should name the discriminator. The recycled-arm test must include the id == next_entity_id-1 case, and a mutation that restores the arithmetic check must turn it red."
 ],
 "non_blocking": [
  "W1 - D4's reasoning understates the cost of the fresh path. The plan says the second RMW 'hits a line this core already owns, so there is no extra coherence transaction'. On x86 a locked RMW costs roughly 18-25 cycles on an L1-owned line anyway, because of the store-buffer drain, and under 8-worker contention another core can take the line between the two RMWs, so the fresh path costs 2 RFOs. Settle only runs in `&mut` ops that touch the stack. A world that only spawns through Commands (setup, streaming, the profile_spawn_single / comparison_v2 Beat-Bevy rows) never runs one, so free_top drifts negative forever and every spawn pays both RMWs. CONSEQUENCE: about +1 serialising locked op per fresh Commands::spawn, and more under contention. CONFIDENCE: CONFIRMED for the structure, PLAUSIBLE for the magnitude. Direction: correct the D4 reasoning, and make the gated-vs-ungated bench on a pure-spawn row a merge precondition once the machine is quiet, not just a listed validation item.",
  "O1 - Record EM2' as a named precondition that KE17 must keep, not just 'compatible'. settle writes free_top with a plain `get_mut` store, which would erase a concurrent worker `fetch_sub` and double-issue an id. So no apply window may ever overlap a running system that has HAS_DEFERRED=true. `may_defer` is built but not yet read (schedule.rs:139-156). State the constraint where the KE17 split will be built, at the SCH7 comment in apply_window_drain, so the split's author sees it.",
  "O2 - The layout numbers are only true on the native build. Under Miri, VmReservation gains a `layout` field (vm.rs:95-96): InlandStore becomes 64 B, VmColumn 88 B, and live_count/reservoir move to @64/@128. Under loom the atomics are not 8 B, so `offset_of!(free) == 16` fails. Gate every new size/offset const-assert on `all(not(miri), not(loom), target_pointer_width = \"64\")`. Separately, the existing layout comment at entity_master.rs:37-42 says InlandStore is 32 B; it is 48 B (5 fields, inland_store.rs:84-108). The plan uses 48 correctly, and D5 should fix the comment when it rewrites it.",
  "O3 - Size the free-list reservation from the inland's ROUNDED ceiling (vm.os_len()/16, inland_store.rs:198), not from reserve_request/16. For `with_capacity(c)` above the default where c*16 is not a multiple of the commit granule, the inland ceiling can be up to 4095 slots larger than the free list's reserve_elems. That makes D6's 'push past the reservation is structurally impossible' false at the extreme; it would only fire as a VmColumn exhaustion panic at more than 67M entities.",
  "O4 - D3 and the MT section describe cases that do not exist. 'Exclusive systems that mix Commands claims with &mut world calls' and 'an exclusive system that holds both Commands and &mut EcsMaster' are not reachable: ExclusiveFunctionSystem is `FnMut(&mut EcsMaster)` with no params (exclusive_function_system.rs:149-152), and Commands is only minted in get_param (commands.rs:425). The real sites where claims mix with `&mut` are DeferredCommands::spawn in hooks (claim, then the enclosing delete_entity's push) and run_system* called inside a Command::apply. Correct the text, and point the Miri/proptest coverage at those sites.",
  "O5 - The Miri unit test (std::thread::scope, 3x8 claims) checks the protocol against spawn/join happens-before edges. The proof in the plan relies on different edges: the KE16 deque publication plus the completion Release/Acquire. Add one short Miri run that drives App::with_pool(pool(2)) Commands churn for a few frames, so the data-race detector sees the dispatcher's entry/base writes against worker entry reads through the real edges. CONFIDENCE: PLAUSIBLE; inland reads may already cover the same edge.",
  "O6 - The doc sweep misses three places. (a) Test headers that give EM2 as the reason to avoid Commands, which become false: ke3_query_random_access.rs:293-300 and ke2_entities_param.rs:162-169. (b) The EntityCounter Send/Sync SAFETY comments (entity_counter.rs:87-100) say 'no plain memory access is possible through this type'; that is now false, and the Sync argument must cite SCH7. (c) The census/attribution line anchors are stale in the current tree: realloc_sum: 2 is at alloc_frame_census.rs:2039, not :1874-1885, and the C6 asserts are at alloc_frame_attribution.rs:1687-1698, C7 at :1700-1739. Locate them by name.",
  "OPEN Q1 - Does any cross-frame side state outside boyko_ecs key per-entity history by bare EntityId (render prev-transform / motion vectors, GPU instance maps, UI focus)? Such state would silently have assumed Commands-spawned ids are never reused. After this change an id is reused one round after its despawn. entity_observers is already retired before the push (entity_api.rs:1067), and the physics warm-start keys on dense BodyIndex, not EntityId (warm_start.rs:35-41). Settle it with a grep for `.id().0`-keyed maps in boyko_render / boyko_ui / boyko_physics.",
  "OPEN Q2 - The plan's own owner question on replay determinism stands. The analysis is accurate: a lone spawner loses deterministic ids only when two despawners share a round, and the fix belongs in apply_window_drain's completion order."
 ]
}

---

## Design rev 2 (architect)

# Rev 2: recycling entity ids on the deferred route (EM2′)

The critique has one blocker, and it is confirmed. `rewind_allocate`'s arithmetic check (`entity_master.rs:533`) accepts a recycled id whenever it is also the newest minted id. When that happens, a stale handle becomes valid again and aliases the new entity. The bug is already on d11962a9 and is reachable through the public API. The rejection path has a second failure: when the recycled id is not the newest, it leaks. D9 (new) fixes both by making the rewind follow a ticket that records which branch the allocation took. The fix has a RED-first test and a mutation that must turn it red.

W1 is correct on the facts. I did not follow its suggested remedy: the gated-vs-ungated choice is removed by design (D4), so no benchmark is needed to decide it. Every other finding is accepted.

I had no shell and no Agent tool, so graphify and the `researcher` did not run. Everything below was checked by reading files in `D:/wt/joltab` at d11962a9.

## Disposition of the critique

| Finding | Verdict | What changed |
|---|---|---|
| **C1** (blocking) | **Accepted and confirmed.** I re-traced it:<br>• `allocate_entity` pops the id and reads the generation from the slot (`entity_master.rs:125-134`).<br>• The rejection happens at `archetype.rs:1117-1118`.<br>• `entity_api.rs:216` then hits `id+1 == next` (`entity_master.rs:533`) and rolls the counter back.<br>• The next fresh mint re-registers the id with gen 0 (`:343-347`).<br>Second failure: when the recycled id is not the newest, `rewind` returns false and the `deallocate_entity` fallback does nothing (`:361-363`), so the id leaks. Both are on d11962a9. | New D9 (rewind by ticket) and invariant R1; the Context bullet is corrected; D8, the API, the Algorithms table and steps 0/3/7 are updated; new T-RED-2 and mutation M5. |
| **W1** | **Facts accepted; remedy differs.** A locked RMW always drains the store buffer, and in a world that never settles, ungated claims pay two RMWs forever. Instead of benchmarking gated against ungated, both costs are removed: each `EntityCounter` carries an EXHAUSTED bit. It is set when the counter is created, from one plain load, and again when a `fetch_sub` fails.<br>Steady state is one locked RMW on both the recycled and the fresh path. A second RMW happens only on the one claim per system call that runs the stack dry. The benchmark stays as a non-regression check but is **not** a merge precondition (reason under D4). | Goal, D1 trade-off, D4 rewritten, Data structures, Algorithms, new invariant X1, T7, mutations M6/M6b/M7, benchmarks. |
| **O1** | Accepted. The requirement is named EM2′-K and is written at the SCH7 site in this change (comments only). | Context, Invariants, MT, step 10b. |
| **O2** | Accepted. Size/offset asserts are gated to native 64-bit (not Miri, not loom), and the Miri layout is stated. The 32 B comment is fixed (it is 48 B). | Data structures, step 3. |
| **O3** | Accepted. The free list is sized from the inland's rounded ceiling via `InlandStore::ceiling_slots()`. | D6, step 1b. |
| **O4** | Accepted. The impossible case is removed from the text, and the two real sites where claims and `&mut` mix get coverage. | D3, MT, T9, Miri test 2. |
| **O5** | Accepted. A Miri run goes through the real pool (precedent: `miri_schedule_parallel.rs:116-117`). | Miri. |
| **O6** | Accepted (a), (b) and (c). Census lines are now found by name, not line number. | Steps 4, 11, 12. |
| OPEN Q1 | Settled by a grep. Six sites were classified; one (`PunctualSlotAssignment`) goes to the render owner. | Step 13, Open Q5. |
| OPEN Q2 | Same as my own Q1. No change. | — |

## Patch log (rev 1 → rev 2)

Each entry quotes the removed rev-1 text word for word. The added text is the rev-2 version of the named section below.

**P1 — Goal → Performance, second bullet.**
Removed: "A worker claim costs one locked RMW plus two L1 loads when a recycled id is available, and two locked RMWs on one line it already owns when minting fresh."
Added: rev-2 bullet. Depends on: D4, X1.

**P2 — Context → "Adjacent defect".**
Removed: "**Adjacent defect, same class:** in `create_entity`'s rejection path (`entity_api.rs:216-222`), a recycled id is leaked. The comment says it "returns to the free list", but `deallocate_entity` returns `false` for a slot that was never registered (`entity_master.rs:361-363`), so it does not. The comment is false."
Added: rev-2 bullet naming both failures. Depends on: D9.

**P3 — Context → SCH7 sub-bullet.**
Removed: "KE17's future split window keeps the barrier for systems that carry `Commands` (`docs/MEASUREMENT-QUEUE.md:88-89`), so this design is compatible with it."
Added: the EM2′-K sub-bullet. Depends on: Invariants, MT.

**P4 — D1 Trade-off.**
Removed: "- The fresh-mint path costs 2 RMWs instead of 1. The second RMW hits a line this core already owns exclusively."
Added: rev-2 bullet. Depends on: D4.

**P5 — D3 first Why bullet, tail.**
Removed: "and exclusive systems that mix `Commands` claims with `&mut` world calls."
Added: the hook and `run_system`-in-apply sites. Depends on: MT aliasing bullet.

**P6 — D4, whole section replaced.**
Removed:
"### D4 — The claim is ungated: `fetch_sub`, then `fetch_add` as the fallback
**Why:**
- The target steady state (flat churn) always takes the recycled path, where a gate would add a load.
- On the fresh path the second RMW hits the same line the first one made exclusive (D5), so there is no extra coherence transaction.
- The loop body has one branch (principle 7).

The measurement is listed under validation. It is not an open fork."
Added: rev-2 D4. Depends on: EM2′, X1, data structures (EntityCounter).

**P7 — D6, second bullet.**
Removed: "- Free entries are distinct ids (F3), each below `inland.len()`, so the free list's length can never exceed the inland ceiling. Pushing past the reservation is therefore structurally impossible."
Added: rev-2 bullet with the sizing formula. Depends on: F3, step 1b.

**P8 — D8 body.**
Removed: "`EntityMaster::reserve_entity(&self)` becomes the claim. It serves `EntityCounter` (workers) and `DeferredCommands::spawn` (hooks, on the dispatcher). `rewind_allocate` gains a recycled-id arm (`unpop`), which closes the `entity_api.rs:216-222` leak."
Added: rev-2 D8. Depends on: D4, D9.

**P9 — D9 (new, whole section).** Depends on: R1, Algorithms.

**P10 — Data structures.**
Removed: "}   // size 128, align 64; const-assert offset_of!(free) == 16"
Removed: "}   // size 192, align 64"
Removed:
"pub struct EntityCounter<'s> {               // stays 8 B, Commands stays 16 B
    reservoir: *const EntityReservoir,
    _marker: PhantomData<&'s EntityReservoir>,
}"
Added: rev-2 block and bullets. Depends on: D4, O2.

**P11 — Invariants.** Rows EM2′-K, R1 and X1 added; nothing removed.

**P12 — Public API delta.** Added: `try_claim_recycled`, `mint_fresh`, `AllocTicket`, `allocate_entity_ticketed`, the ticket form of `rewind_allocate`, `InlandStore::ceiling_slots`, and the `free_top_raw` probe. Nothing removed.

**P13 — Algorithms table.**
Removed: "| `claim` (worker) | `r=fetch_sub(1)`; `r>0` → read `free.base[r-1]`; otherwise `fetch_add(1)` → `Entity(id,0)` | O(1) | reservoir line (RFO if contended) plus the entry line (1 miss per 4 claims) | 1 |"
Removed: "| `allocate_entity` | `pop_free()` → debug-check F3 → return; otherwise fresh plus `ensure` (as today) | O(1) | inland line and reservoir line; the entry is L1 (just pushed) | 1 |"
Removed: "| `rewind_allocate` | fresh: as today (`entity_master.rs:532-543`). Recycled: `unpop(e)`, where a release `assert!` checks the slot is null and gen matches (cold) and a debug check confirms `e` is the entry physically at `top` | O(1) | — | cold |"
Added: rev-2 rows. Depends on: D4, D9.

**P14 — MT section.**
Removed: "- **Pre-existing aliasing class (not new):** an exclusive system that holds both `Commands` and `&mut EcsMaster` already has it with today's `*const AtomicUsize`."
Added: the aliasing, EM2′-K and EXHAUSTED-bit bullets.

**P15 — Miri and loom.** Added: `tests/miri_em_deferred_recycle.rs`. Nothing removed.

**P16 — Integration steps.**
Removed (step 0): "add `crates/boyko_ecs/tests/em_deferred_recycle.rs` with T-RED-1 (spec below)."
Removed (step 3 fragment): "and `rewind_allocate` (`:520-548`, plus the recycled arm);"
Removed (step 4): "4. `system/params/entity_counter.rs`: pointer type, `reserve_entity` → `claim`, `reserve_batch` → `mint_fresh_batch`; restate EM1′/EM2′/EM4′/EM6′ in the docs (`:1-30`, `:129-161`); update the tests."
Removed (step 7): "7. `ecs_master/entity_api.rs:216-222`: the rewind now succeeds for recycled ids. Correct the false comment."
Removed (step 11): "11. Census re-derivation, counts only (step 13 of the checklist below)."
Added: rev-2 steps 0, 1b, 3, 4, 7, 10b, 11, 12 and 13.

**P17 — Validation.**
Removed: "- `rewind_allocate_returns_recycled_id`"
Removed: "- `alloc_frame_census.rs:1874-1885`: `realloc_sum: 2` → **0**. It is a structural zero: at rest the free list holds ≤ 64 entries on a `VmColumn`, not a `Vec`."
Removed: "- Re-measure `dispatch_max: 6` (`:1872`). Its +1 headroom was justified by the free-list doubling (`:1864-1866`)."
Removed: "- Update the census module docs at `:81`, `:93`, `:118`, `:136`, `:177`."
Removed: "  - Invert C6 (`:1575-1581`) to `recycled_after ≤ recycled_before + CHURN_PER_FRAME` and `slots_after == slots_before`."
Removed: "  - Reword C7's message premise (`:1621-1627`)."
Removed: "  - Change the verdict row (`:2691-2701`) to FREE."
Removed: "  - Update the module doc (`:107-114`)."
Removed: "- a gated vs ungated claim run (D4)."
Added: T-RED-2, T7, T9, the recycled-rewind unit cases, M5–M7, census entries located by name, and the benchmark list with its not-a-precondition reason.

**P18 — Open questions.** Q5 added.

**P19 — Checklist.** Edge-case and correctness lines added.

---

# Architecture: recycling entity ids on the deferred route (EM2′, the free list becomes a claimable stack) — Rev 2

**Result:** The free list becomes a stack that workers claim from with one `fetch_sub`. It is recycled on the worker side, not in `CommandQueue::apply`. The census suggested recycling in `apply`, where the dispatcher may pop. That cannot work on its own: when `Commands::spawn` returns, `.id()` has already given user code the id, so the id must be recycled at reserve time.

**Evidence and tooling:** Everything below was read in `D:/wt/joltab` at d11962a9. I had no shell, so graphify was not run, and no Agent tool, so the `researcher` could not be launched. As a substitute I checked Bevy 0.15 `entity/mod.rs` myself: `Entities::reserve_entity` does `free_cursor.fetch_sub(1, Relaxed)` over a `pending` list and reconciles in `flush()`. The design is adapted from that pattern.

Rev 2 additionally checked:
- `entity_master.rs:124-148`, `:520-548`, `:703-723`
- `entity_api.rs:183-224`
- `archetype.rs:1117-1125`
- `ecs_master.rs:1436-1540`
- `entity_counter.rs` (whole file)
- `commands.rs:97-117`, `:164-244`, `:425-426`
- `vm_column.rs` (whole file)
- `vm.rs:111`
- `inland_store.rs:84-217`
- `schedule.rs:139-156`, `:667-679`
- `deferred_master.rs:176-219`
- `profile_spawn_single.rs:567-583`
- `miri_schedule_parallel.rs:72-117`
- the census and attribution anchors
- a workspace grep for side state keyed by bare `EntityId`

## Goal
- **Functional:** a flat population churned through `Commands::spawn` / `despawn` keeps `EntityMaster`'s free list and the `entities_inland` slot store bounded.
  - The free list stays at or below one window's despawns at rest.
  - The slot store stays at or below the peak of (live + reserved-not-yet-applied) + leaked.
  - Handles already returned to user code stay valid, and the generation discipline stays in place.
  - The rejected-create path never brings a stale handle back to life (D9).
- **Performance:**
  - Steady state costs 0 heap events and 0 VM commits per frame on the entity path. Today it is unbounded: the census measured +131,072 entries over 2048 frames.
  - Workers never read `entities_inland` (EM3 is kept word for word).
  - Dispatcher push and pop are plain instructions with no `lock` prefix.
  - **Worker claim cost:**
    - Recycled: one locked RMW plus two loads (`free.base` and the entry). The gate is a bit test on a register, with no memory access.
    - Fresh: one locked RMW, the same as today.
    - The only claim that pays two locked RMWs is the one per system call that runs the stack dry.
    - Each system call pays one extra plain load when its `Commands` is created (D4).

## Context and constraints (verified)
- **The defect path:**
  - `Commands::spawn` calls `EntityCounter::reserve_entity`, which only does `fetch_add` (`commands.rs:164-168`, `entity_counter.rs:148-161`). `EntityMaster::reserve_entity` does the same (`entity_master.rs:186-190`).
  - A despawn goes `deallocate_entity` → `free_entity_ids.push` (`entity_master.rs:378`).
  - The only pop is the dispatcher's `allocate_entity` (`entity_master.rs:125`).
  - `reserve_entity_skips_free_list` (`entity_master.rs:785-798`) pins the defect's cause.
- **A second leak route of the same class:** `DeferredCommands::spawn` (`deferred_master.rs:193-203`) mints fresh ids through `next_id_atomic()`.
- **Adjacent defects on the same path, both on d11962a9.** `create_entity`'s rejection path (`entity_api.rs:209-224`) chooses how to undo `allocate_entity` by arithmetic (`entity_master.rs:533`: `id.0 + 1 == next_entity_id`), not by what `allocate_entity` actually did.
  - **(i) The recycled id is not the newest:** `rewind` returns false. The fallback `deallocate_entity` returns `false` for a never-registered slot (`:361-363`), so the id **leaks**. The comment "returns to the free list" is false.
  - **(ii) The recycled id is also the newest minted id:** the fresh branch is taken and `next_entity_id` rolls back to that id. The id is then in no list. The next fresh mint re-registers it with generation 0 (`:343-347`), which **revives the stale handle**: generation ABA, in release builds, reachable from safe public API. The trigger is a `create_entity` with a missing component right after despawning the newest entity (trace in D9).
- **Load-bearing premise, SCH7:**
  - `apply_window_drain` runs only when `pending == running || running == 0` (`schedule.rs:667-679`). The exclusive path runs only when `running == 0` (`schedule.rs:1099`). So `&mut EntityMaster` never overlaps a worker phase.
  - **EM2′-K (KE17 requirement, named here):** a future split window must not run any `&mut EntityMaster` operation while a system with `may_defer[i] == true` is dispatched.
    - `may_defer` is built but not yet read (`schedule.rs:139-156`).
    - This design makes the requirement load-bearing: `settle` stores `free_top` plainly, and `push_free` writes an entry a worker may still be reading.
    - Step 10b writes the requirement at the SCH7 site.
- **Precedent for giving workers a plain, non-atomic store:** `UnsafeEcsCell::entities` already hands workers `*const InlandStore` under SCH7 (`unsafe_ecs_cell.rs:291-313`).
- **Invariants that must hold:**
  - Generations: the bump happens in `deallocate_entity` (`entity_master.rs:370-376`), and registration writes `entity.generation()` (`:343-347`).
  - EM1–EM6 (`docs/archive/PHASE-11-ENTITY-COMMANDS-PLAN.md:128-135`).
  - `Commands` is 16 B (`commands.rs:117`) and `EntityCounter` is 8 B (`entity_counter.rs:107`).
  - The X.G hot pair sits at offset 0 of `EntityMaster` (`entity_master.rs:37-55`).
- **Owner constraints:** the new structure goes on `VmColumn` (`vm_column.rs:80`), not on `Vec`; lock-free.

## Key decisions

### D1 — The free list *is* the claimable stack
**What:** `free: VmColumn<Entity>` is a LIFO stack whose logical length is an `AtomicIsize free_top`.
- Workers claim with `r = free_top.fetch_sub(1, Relaxed)`. If `r > 0`, the handle is `free[r-1]`; otherwise they mint fresh.
- The dispatcher changes the stack's structure only under `&mut`.

**Why:**
- It adds no refill step and no second structure. The dispatcher's only extra work per window is one clamp.
- Any system can consume any free id, so no demand prediction is needed.
- There is no ABA problem. During a phase the stack is pop-only and entries are immutable, so `fetch_sub` returns each positive value at most once.
- One RMW per claim is the same cost as today's `fetch_add`.

**Alternatives rejected:**
- **(a) A separate ring, refilled each apply window:**
  - The refill copy is O(k) per window and needs a policy for how many ids to move. If the answer is "all", the ring *is* the free list.
  - The dispatcher's `allocate_entity` would have to pop from two places.
- **(b) A per-worker or per-system pre-reserved batch:**
  - Per worker: a system runs on whatever worker steals it, so every claim needs a TLS lookup. On rustc ≥1.98 windows-gnu, `thread_local!` costs two locked RMWs plus `FlsSetValue`.
  - Per system: the stash hoards ids and leaks on `CommandQueue::drop`, which has no world access. `run_system_once` builds a fresh queue on every call. It also adds a field to the one-line, 56 B `CommandQueue`.
- **(c) Remapping reserved ids at apply time:**
  - `.id()` has already escaped into other commands, resources, events and `Local`s, and there is no registry of those copies.
  - The only way to remap is an unbounded indirection table consulted on every `is_entity_valid` / `get_component_raw`. That breaks the rule that a returned handle stays valid.
- **(d) An intrusive free list through the inland slots (EnTT style):**
  - The pop is a CAS retry loop, and each claim is a dependent random miss.
  - It breaks EM3 and requires ids to fit in `u32`.
- **(e) Bevy's single signed cursor:** `spawn_batch` needs a contiguous fresh range from `next_entity_id` (`spawn_batch_command.rs:694-699`, `register_batch` `entity_master.rs:263-307`). The two schemes would collide unless batch also moves onto the cursor, which couples this change to Stage B (D7).

**Trade-off:**
- The claim that runs the stack dry pays two RMWs, once per `EntityCounter` (D4).
- `free_top` can go transiently negative, and every `&mut` path that touches the stack pays one clamp.

### D2 — Entries carry the generation (`Entity`, 16 B), not a bare `EntityId`
**Why:**
- The claim reads the entry sequentially (LIFO-adjacent, 4 per line) instead of doing a random `inland[id].generation()` load, which saves about one miss per claim.
- EM3 keeps holding word for word.
- The dispatcher already has `next_gen` in a register at `entity_master.rs:371`.

**Alternative rejected:** a packed 8 B `(u32, u32)`. It adds an id < 2³² invariant that contradicts `InlandStore::with_capacity` (`inland_store.rs:129-143`).

**Trade-off:** 2× bytes per free entry. The free list is bounded by one window's despawns, so this is negligible.

### D3 — Settle lazily inside every `&mut` method that touches the stack; no end-of-window hook
**What:** `settle()` sets `top = max(free_top, 0)`, then `free.truncate(top)`, then `free_top = top`. It is plain code via `get_mut` and branch-free (`max`). Every `&mut` method that touches the stack calls it first.

**Why:**
- It is correct no matter how many apply sites exist: `apply_window_drain`, the exclusive inline path, `run_system_once`, and the deferred-hook drain.
- It is also correct at the two real sites where claims mix with `&mut` on one thread:
  - A hook's `DeferredCommands::spawn` claims inside an enclosing `delete_entity`, whose hooks fire before `deallocate_entity`'s push (`entity_api.rs:1047-1099`). The push settles first.
  - `run_system*` called inside a `Command::apply`: the nested system's claims interleave with pushes the enclosing apply has already made.
- Exclusive systems cannot hold `Commands`. They are `FnMut(&mut EcsMaster)` with no params (`exclusive_function_system.rs:149-152`), and `EntityCounter` is minted only in `Commands::get_param` (`commands.rs:425`).
- Claimed entries above `top` were already copied into their handles, so overwriting them is harmless.

**Alternative rejected:** Bevy's mandatory `flush()` points. They need a hook at every site that mutates the world; missing one site means double issue.

### D4 — The claim is gated by a per-counter EXHAUSTED bit, not by a load and not left ungated
**What:**
- `EntityCounter` holds `Cell<*const EntityReservoir>`. Bit 0 of the address is `EXHAUSTED`; the reservoir is `align(64)`, so bits 0–5 are always zero.
- The bit is set in two places:
  - **At creation** (`UnsafeEcsCell::entity_counter`, once per system call): one Relaxed `free_top.load() <= 0` sets it.
  - **On the first failed claim:** the bit is set after `fetch_sub` returns `r <= 0`, via `map_addr`, which keeps pointer provenance.
- **Claim:** if the bit is clear, `r = fetch_sub(1)`; `r > 0` returns `free[r-1]`, otherwise the bit is set. If the bit is set or `r <= 0`, `fetch_add(1)` mints fresh.

**Why the bit is always true (invariant X1):**
- During an `EntityCounter`'s lifetime nothing can be pushed. A push needs `&mut EntityMaster`.
- The counter lives inside a `Commands` held by a system body. System bodies hold no `&mut EcsMaster`, and SCH7 keeps the dispatcher out of phases.
- So `free_top` only decreases while the counter exists. Once it has been seen `<= 0`, it stays `<= 0`.
- A wrong bit could only cost a missed recycle, never a double issue: the bit only ever suppresses `fetch_sub`.

**Cost:**
- Recycled claim: one locked RMW plus an AND/TEST on the pointer, which is loaded for the RMW anyway. No added memory access.
- Fresh claim, steady state: one locked RMW, which is today's instruction.
- One extra RMW on the single claim in a call that runs the stack dry, plus one plain load per call at creation.
- Under contention neither path adds a coherence transaction, because no plain load of the shared line comes before the RMW.

**Correction of rev 1 (W1):**
- Rev 1 said the second RMW on an owned line adds no coherence transaction. That is wrong in two ways:
  - A locked RMW drains the store buffer even on an L1-owned line: a serialising op of roughly 20 cycles on current x86 cores.
  - Under contention another core can take the line between the two RMWs.
- Settle runs only in `&mut` ops that touch the stack, so a world that spawns only through `Commands` would pay two RMWs forever.
- That is exactly the shape of `p12_boyko_reserve_entity_only`: 10k `reserve_entity` calls in one system on an empty world (`profile_spawn_single.rs:567-583`).

**Proof by integer, not timing:**
- `free_top`'s negative drift equals the number of `fetch_sub`s that found `<= 0`. So T7 pins the RMW count as a number: a pure-spawn run leaves `free_top == 0`, and a run that drains k entries leaves `−1`.
- This meets the owner's no-timing rule and decides the question the critic wanted a benchmark for.

**Alternatives rejected:**
- **(a) Ungated (rev 1):** see the correction above.
- **(b) A load gate before every claim** (`if free_top.load() > 0`): under contention the load brings the line in Shared, and the RMW must then upgrade it. That is two coherence transactions on the recycled path, which is the target steady state.
- **(c) A separate `bool` field:** `EntityCounter` would become 16 B and `Commands` 24 B, breaking the pin at `commands.rs:117`.
- **(d) Bevy's signed cursor:** rejected in D1(e).

**Trade-off:**
- `EntityCounter` loses `Copy` and `Sync` because of the `Cell`.
  - Nothing copies it: `Commands` holds the only instance (`commands.rs:107`, built only at `:426`; the reborrow at `entity_commands.rs:314` borrows `Commands`).
  - `Commands` is already `!Sync` because of its `&mut CommandQueue`.
  - Code outside the crate cannot obtain an `EntityCounter`: `from_ptr` is `pub(crate)` and the field is `pub(crate)`.
- Worst case, one extra locked op per system call that spawns into an empty stack. A system that spawns once per frame pays it once per frame.

### D5 — The reservoir sits on its own 64 B line, holding both atomics
**What:** `EntityMaster { entities_inland @0, live_count @48, reservoir @64 (align 64) }` on the native build.

**Why:**
- Today `next_entity_id` shares line 0 with `entities_inland.base/len`. Every worker spawn's `lock xadd` therefore invalidates the header that every other worker's `get_component_raw` and `Entities::get` read. Moving both RMW'd atomics off line 0 removes that false sharing.
- The hot pair keeps offset 0, so the XG-B1 displacement is unchanged.

**Trade-off:**
- A fresh `allocate_entity` on the dispatcher now touches two L1-hot lines instead of one. X.G measured this family as sensitive: +6–10% from a field shuffle (`entity_master.rs:41-42`).
- `EntityMaster` goes from align 8 to align 64, and `EcsMaster` inherits align 64.

### D6 — The free list lives on `VmColumn<Entity>`, reserved to the inland's rounded ceiling
**Why:**
- It follows the owner's allocator rule. The base address is write-once, so a worker's cached `base` can never dangle mid-phase.
- **Sizing:** `free_reserve_elems = InlandStore::ceiling_slots()`.
  - That is `checked_align_up(inland.reserve_request, COMMIT_GRANULE) / SLOT_SIZE`: the exact value `grow_to` later derives as `vm.os_len() / SLOT_SIZE` (`inland_store.rs:198`), since `VmReservation::reserve` rounds with the same granule (`vm.rs:111`).
  - It is computed from `reserve_request` with no syscall and no materialization.
  - A `debug_assert!` in `InlandStore::grow_to` pins that the two values are equal.
  - Free entries are distinct ids (F3), each below `inland.len()`, which is at most the ceiling. So `free.len()` can never exceed `free.reserve_elems`, and `VmColumn`'s exhaustion assert is unreachable for every `with_capacity(c)`, including a `c * 16` that is not a granule multiple.
  - Because `size_of::<Entity>() == SLOT_SIZE == 16`, the free list reserves exactly the inland's address span.
- The first commit is lazy (`POOL_MIN_SLAB` = 64 KiB, 4096 entries), so `EcsMaster::new` makes no syscall and XG-B4 stays green.
- `with_capacity` precommits `capacity/4` entries, the same as today's `Vec::with_capacity(capacity/4)`.

**Trade-off:** under Miri and wasm, the fallback eagerly zero-allocates the inland's span (16 MiB by default) per world on its first despawn, the same as `InlandStore`.

### D7 — `spawn_batch` stays fresh-and-contiguous in Stage A; Stage B is designed now
- Batch contiguity is load-bearing across `register_batch`, the `EntityId(start_id + i)` arithmetic (`spawn_batch_command.rs:697-699`, `:722`) and the `SpawnBatchIter` range.
- No per-frame path in the workspace calls `Commands::spawn_batch`; the grep found only benches, setup and `app.rs:1199`.

**Stage B sketch:**
- A batch does `r = free_top.fetch_sub(n)`. Its recycled part, `free[max(r−n,0) .. max(r,0))`, is copied at claim time as a trailing payload behind the `SpawnBatchCommand`.
- The deficit comes from `next_entity_id.fetch_add(n−k)`, which keeps the fresh part contiguous.
- Apply splits into a scatter-register of k entries plus the existing contiguous `register_batch`.
- Stage B's batch claim must also honour the EXHAUSTED bit (skip `fetch_sub` when it is set).

### D8 — Every deferred single-spawn route goes through the reservoir
- **Workers:** `EntityCounter::reserve_entity` is the gated claim (D4).
- **Hooks:** `DeferredCommands::spawn` and `EntityMaster::reserve_entity(&self)` use the ungated `EntityReservoir::claim`.
  - A hook's `DeferredCommands` is short-lived, and each spawn stands alone.
  - The claim/push mixing at that site is settled by D3.
- Rewind is D9.

### D9 — Rewind follows an allocation ticket, never id arithmetic
**What:**
- `allocate_entity_ticketed(&mut self) -> AllocTicket { entity, source: Fresh | Recycled }`.
- `rewind_allocate(&mut self, t: AllocTicket) -> bool` (`#[cold]`):
  - **Fresh:** `id + 1 == next` → `next -= 1` (plain, `&mut`). Otherwise leak, with a `debug_assert!`.
  - **Recycled:** the slot is null and `slot.generation() == e.generation()` (release check, cold) → `unpop(e)`. Otherwise leak, with a `debug_assert!`.
- `allocate_entity()` becomes `allocate_entity_ticketed().entity` for callers that never rewind (`prefab.rs:693`, `materialize.rs:386`, tests).

**Why:**
- **The C1 trace:**
  1. `create_entity` mints E = (N−1, 0).
  2. `delete_entity(E)` pushes (N−1, 1).
  3. `create_entity(arch, [missing])` pops (N−1, 1). `Archetype::create_entity` returns false (`archetype.rs:1117-1118`).
  4. The arithmetic arm at `entity_master.rs:533` matches and sets `next = N−1`.
  5. The next create mints (N−1, 0), and `register_entity_with_ptr` writes generation 0, so E is valid again.

  `allocate_entity` already knows which branch it took. The ticket carries that fact instead of guessing it afterwards.
- **Cost:** the tag is the branch outcome, held in a register. Callers that drop the ticket let it die after inlining. `create_entity` pays one byte in its return slot: `Entity` is 16 B and is already returned through a hidden pointer on Win64.
- **Type safety:** the ticket is not `Copy` and not `Clone`, so a second rewind of the same allocation does not compile. This replaces the "a stale rewind returns false" heuristic tested at `entity_master.rs:717-722`.
- **Every refusal is a leak, never a re-issue (R1).** A leaked id is bounded and safe; a wrong restore is generation ABA.

**Alternatives rejected:**
- **(a) "Generation 0 means fresh":** false after a u32 wrap.
- **(b) Comparing `free[physical top] == e`:** reads bytes above `len` and depends on stale memory. It is kept only as a debug check inside `unpop`.
- **(c) An RAII guard that rewinds on drop:** adds a `Drop` to the success path and a borrow held across the scoped archetype reborrow (`entity_api.rs:185-207`).

**Trade-off:** `rewind_allocate`'s crate-internal signature changes, and two tests are adapted: `entity_master.rs:703-723` and `ecs_master.rs:1523-1540`.

## Data structures
```rust
/// Worker-reachable id source (EM6′). Line 0 = all worker-touched state.
#[repr(C, align(64))]
pub(crate) struct EntityReservoir {
    /// Claimable prefix length of `free`. Workers: fetch_sub(1, Relaxed),
    /// skipped once the claiming counter's EXHAUSTED bit is set (D4).
    /// Can go negative within a phase: one step per fetch_sub that found <= 0.
    /// Dispatcher: plain get_mut; clamped to >= 0 by `settle`. AUTHORITATIVE.
    free_top: AtomicIsize,                 // +0
    /// Fresh-id counter, moved verbatim from EntityMaster (EM1). Monotone
    /// except `rewind_allocate` (Fresh ticket) under &mut.
    next_entity_id: AtomicUsize,           // +8
    /// Recycled entities, LIFO, each carrying the generation deallocate wrote.
    /// `free.len()` = physical top: == free_top at rest, >= free_top in a phase.
    /// Only `free.base` (+16) is read by workers; len/committed/... are
    /// dispatcher-only and written only in windows.
    free: VmColumn<Entity>,                // +16 .. +88 native (72 B); +16 .. +104 under Miri (88 B)
}   // native: size 128, align 64. Miri: size 128, align 64.

#[repr(C)]
pub struct EntityMaster {
    pub(crate) entities_inland: InlandStore, // +0  native 48 B (5 fields) / Miri 64 B: XG-B1 hot pair
    live_count: usize,                       // native +48 / Miri +64; dispatcher-only
    pub(crate) reservoir: EntityReservoir,   // native +64 / Miri +128; own line (D5)
}   // native: size 192, align 64. Miri: size 256, align 64.

pub struct EntityCounter<'s> {               // 8 B (Cell is repr(transparent)); Commands stays 16 B
    /// Reservoir pointer; bit 0 = EXHAUSTED (D4, X1). !Copy, !Sync.
    tagged: Cell<*const EntityReservoir>,
    _marker: PhantomData<&'s EntityReservoir>,
}

#[must_use]
pub(crate) struct AllocTicket { entity: Entity, source: AllocSource }  // not Copy/Clone (D9)
#[repr(u8)] #[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AllocSource { Fresh, Recycled }
```

**Layout pins (O2):**
- Offset and size asserts are gated with `#[cfg(all(not(miri), not(loom), target_pointer_width = "64"))]`:
  - `offset_of!(EntityReservoir, free) == 16`, `size_of::<EntityReservoir>() == 128`
  - `offset_of!(EntityMaster, reservoir) == 64`, `size_of::<EntityMaster>() == 192`
  - `size_of::<EntityCounter>() == 8`, `size_of::<Commands>() == 16`
- Under Miri, `VmReservation` gains a `layout` field (`vm.rs:95-96`). Under loom, the atomics are not 8 B.
- Ungated, true on every 64-bit build: `align_of::<EntityReservoir>() == 64` (the EXHAUSTED bit relies on it) and `size_of::<Entity>() == 16`.

**Send / Sync:**
- `unsafe impl Send + Sync for EntityReservoir`. The atomics are RMW'd from any thread. `free.base` and the entries are written only under `&mut` (SCH7, EM2′-K), and workers read only indices below the claim value they received.
- `unsafe impl Send for EntityCounter`. The `Cell` is state owned by the counter and moves with it. Through the pointer a worker does only atomic RMWs, a Relaxed load, and reads of entries that are immutable for the phase (SCH7).
- No `Sync` for `EntityCounter`: the `Cell` removes it, and nothing needs it.

**loom:** under `cfg(loom)` the atomics are aliased to `loom::sync::atomic` (precedent: `term_list.rs:114-121`), and `get_mut` becomes `with_mut`.

## Invariants (restated)

| ID | Statement |
|---|---|
| EM1′ | Fresh ids come from `reservoir.next_entity_id.fetch_add`, generation 0. Recycled claims carry the entry's generation. |
| EM2′ | The free list's structure (push, truncate/settle, sort, clear, unpop) changes only under `&mut EntityMaster` (dispatcher, SCH7). In a phase it is an immutable array. Workers remove entries only through `fetch_sub` on `free_top`. The generation bump happens only on the dispatcher, in `deallocate_entity`. |
| EM2′-K | (KE17 requirement.) No `&mut EntityMaster` operation runs while a system with `may_defer[i] == true` is dispatched. Claims are reachable only through `Commands`, which has one mint site (`commands.rs:425`), and a system holding `Commands` reports `has_deferred()`. The KE17 split barrier is therefore the guard, as long as it keeps every `may_defer` system out of any window that mutates `EntityMaster`. Breaking it double-issues ids (settle's plain store erases a concurrent `fetch_sub`) and races (a push writes an index a worker may be reading). |
| EM3 | Unchanged: a claim never touches `entities_inland`. |
| EM4′ | Every id handed out is in exactly one of {free prefix, live, claimed-pending, leaked}. The fresh counter is monotone except for a Fresh-ticket rewind. The old "fresh > max(free)" property is dropped (`PHASE-11-ENTITY-COMMANDS-PLAN.md:133`). |
| EM5 | Unchanged: claims never resize anything. |
| EM6′ | Workers reach exactly `*const EntityReservoir`: two atomics (RMW, plus one Relaxed load at creation) and `free.base` plus entries (read-only). No `EntityMaster` field is reachable. |
| F1 | After settle, `free_top == free.len()`. |
| F2 | Always `free_top <= free.len()`. In a phase, `free.len()` is constant and `free_top` only decreases. |
| F3 | For each `e` in `free[0..top)`: `inland[e.id].is_null()` and `inland[e.id].generation() == e.generation()`. All ids are distinct. |
| R1 | `rewind_allocate` undoes exactly the branch the ticket records, and never infers it. A refused rewind leaks the id instead of re-issuing it. |
| X1 | A counter's EXHAUSTED bit is set only after observing `free_top <= 0` (at creation or through its own failed `fetch_sub`), and no push can happen during the counter's lifetime. A set bit is therefore always true. |

**Collision cases:**
- Worker claim vs worker claim: distinct `fetch_sub` values give distinct indices, and F3 makes those distinct ids.
- Recycled vs fresh: a recycled id is below the `next_entity_id` it was minted from, and a fresh id is at least that value.
- Claim vs a later dispatcher pop: settle drops claimed indices from the prefix before any pop.
- Stale vs new handle: the generation comparison.
- Claim vs dispatcher push: impossible under SCH7 / EM2′-K.
- Rewind vs everything else: R1.

## Public API delta (no user-visible signature change)
```rust
impl EntityReservoir {                         // crate-internal
    pub(crate) fn new(free_reserve_elems: usize) -> Self;
    #[inline] pub(crate) fn try_claim_recycled(&self) -> Option<Entity>;           // fetch_sub; worker-safe
    #[inline] pub(crate) fn mint_fresh(&self) -> Entity;                           // fetch_add; worker-safe
    #[inline] pub(crate) fn claim(&self) -> Entity;                                // ungated: try_claim_recycled or mint_fresh
    #[inline] pub(crate) fn is_exhausted_hint(&self) -> bool;                      // Relaxed load <= 0 (counter creation)
    #[inline] pub(crate) fn mint_fresh_batch(&self, n: usize) -> EcsResult<Range<usize>>;
    #[inline] pub(crate) fn claimable(&self) -> usize;                             // max(load,0)
    #[inline] pub(crate) fn next_fresh(&self) -> usize;
    #[inline] pub(crate) fn settle(&mut self) -> usize;
    #[inline] pub(crate) fn push_free(&mut self, e: Entity);
    #[inline] pub(crate) fn pop_free(&mut self) -> Option<Entity>;
    #[cold]   pub(crate) fn unpop(&mut self, e: Entity);                           // Recycled-ticket rewind
    pub(crate) fn clear(&mut self);
    pub(crate) fn sort_free_low_ids_first(&mut self);                              // compact()
}
impl EntityMaster {
    #[inline] pub(crate) fn allocate_entity_ticketed(&mut self) -> AllocTicket;
    #[inline] pub(crate) fn allocate_entity(&mut self) -> Entity;                  // = ticketed().entity
    #[cold]   pub(crate) fn rewind_allocate(&mut self, t: AllocTicket) -> bool;    // true = restored, false = leaked
    #[doc(hidden)] pub fn free_top_raw(&self) -> isize;                            // test/loom probe (precedent term_list.rs:438-444)
}
impl AllocTicket { #[inline] pub(crate) fn entity(&self) -> Entity; }
unsafe fn EntityCounter::from_ptr(p: *const EntityReservoir) -> Self;  // was *const AtomicUsize; sets EXHAUSTED from is_exhausted_hint()
fn InlandStore::ceiling_slots(&self) -> usize;  // align_up(reserve_request, COMMIT_GRANULE) / SLOT_SIZE, no syscall
fn VmColumn::precommit(&mut self, n: usize);    // #[cold]; grows the commit frontier, len unchanged
```
- `EntityMaster::recycled_entity_count(&self)` returns `claimable()`, which equals the list length at rest.
- `Commands::spawn`, `spawn_empty`, `clone_and_spawn*` and `reserve_entity` docs change to: "the returned `Entity` may reuse an id; its generation may be > 0."

## Algorithms for the critical paths

| Op | Steps | O | Cache | Branches |
|---|---|---|---|---|
| `EntityCounter::reserve_entity` (worker, gated) | Bit clear: `r=fetch_sub(1)`; `r>0` → read `free.base[r-1]`; otherwise set the bit. Bit set or `r<=0`: `fetch_add(1)` → `Entity(id,0)` | O(1) | Reservoir line (RFO if contended) plus the entry line (1 miss per 4 claims). The bit lives in the caller's `Commands` (register/L1). | 2, both stable within one call |
| `EntityReservoir::claim` (hooks, ungated) | `try_claim_recycled` or `mint_fresh` | O(1) | as above | 1 |
| `from_ptr` (once per system call) | one Relaxed `free_top.load()`; `<= 0` → preset the bit | O(1) | reservoir line, shared read | 0 (a `select`) |
| `settle` | `top=max(t,0)`; `truncate`; store | O(1) | reservoir line, L1 | 0 (`max`) |
| `deallocate_entity` | as today, then `push_free(Entity(id, g+1))` (settle, write at `top`, len++, top++) | O(1) | inland slot, reservoir line, entry line | 1 (commit frontier, cold) |
| `allocate_entity_ticketed` | settle; `pop_free()` → debug-check F3 → `(e, Recycled)`; otherwise fresh plus `ensure` (as today) → `(Entity(id,0), Fresh)` | O(1) | inland line and reservoir line; the entry is L1 (just pushed) | 1 |
| `rewind_allocate(ticket)` | `Fresh`: `id+1 == next` → `next -= 1` (plain, `&mut`), otherwise leak plus `debug_assert!`. `Recycled`: slot null and gen matches (release check) → `unpop(e)` (settle, write at top, len++, top++; debug check that `e` is the entry physically at `top`), otherwise leak plus `debug_assert!` | O(1) | — | cold |

SIMD: not applicable (scalar id bookkeeping).

## Generations, and what a stale handle sees
- **Rule:** `dealloc` writes `slot = {null, 0, g+1}` and pushes `Entity(X, g+1)`. A claim, pop or unpop returns or restores `Entity(X, g+1)` verbatim. `SpawnAtCommand` apply registers `g+1` (`spawn_at_command.rs:350-352` → `entity_master.rs:343-347`).
- **Stale `(X, g)` before the successor applies:** the slot is null, so `is_entity_valid` is false and Insert/Remove/Despawn are no-ops (EC8).
- **Stale `(X, g)` after the successor applies:** the generation mismatch rejects it everywhere. The bare-id `Entities::get(X)` resolves to the current occupant, as designed (`tests/ke2_entities_param.rs:206`).
- **Rejected create:** the id returns to exactly where it came from (R1). The C1 revival cannot happen (T-RED-2).
- **The new handle `(X, g+1)` between reserve and apply:** invalid, exactly like a fresh reserved id today, and valid after apply. If the queue panics and the command is re-absorbed a window later, X is still in no list, so no other claimant can get it.
- **Wrap:** under per-frame LIFO churn one slot wraps after 2³² reuses, about 2.3 years at 60 Hz. This is pre-existing: the dispatcher path already reuses LIFO (census control C7).

## Ordering with the apply window
1. **Phase P (workers):** claims return entries from `free[0..top_P)` or fresh ids. An invocation that starts with `top_P <= 0` never does a `fetch_sub`. Ids freed in P cannot be claimed in P, because they are still live.
2. **Gate:** Acquire on `pending` (`schedule.rs:667`) synchronizes with every worker's completion Release, so the dispatcher sees the final `free_top`.
3. **Window W (dispatcher, `&mut`):** the first `&mut` op that touches the stack settles. Commands then apply in completion order: despawns push, spawns register the claimed handles. Hook spawns (`DeferredCommands`) claim through the ungated `claim`, and the next `&mut` op absorbs them.
4. **Phase P+1:** ids freed in W can be claimed. Reuse latency is one round.

## Determinism
- **Deterministic case:** within one system invocation, the k-th claim returns `free[top_P − k]`; after exhaustion, `next_entity_id` +0, +1, and so on. With one claiming system per round and a fixed apply order, the same op sequence gives the same ids (T3). The EXHAUSTED bit does not change which ids are returned, only how many RMWs it takes.
- **Non-deterministic cases:**
  - Concurrent claimers interleave. This is the same as today's concurrent `fetch_add`.
  - **New dependency:** the free-list order depends on the apply order of despawning systems. That order is completion order within a round (`schedule.rs:781`), which is non-deterministic when two despawners share a round. Today a lone spawner gets deterministic fresh ids even next to concurrent despawners; after this change it does not. Bevy has the same property.
- **Fix, if the owner wants replay determinism:** drain completions in system-index order. That is outside this scope (open question 1).

## Multithreading model and data-race freedom
- **Model:**
  - Many writers on the atomics.
  - One writer on `free` / `base` (the dispatcher) and many readers (workers), separated in time by SCH7. Future windows must keep EM2′-K.
- **Happens-before chain for the plain `free.base` and entries:** dispatcher writes → the pool's task publication (Release/Acquire in the KE16 deque) → worker reads → worker completion `fetch_add(Release)` → dispatcher Acquire (`schedule.rs:667`) → dispatcher writes.
- **No read of uninitialized or out-of-range memory:** a worker reads index `r−1 < r <= top_P <= free.len()` (F2).
- **No double issue:** `fetch_sub` is an RMW on a single location. Its total modification order gives each positive value at most once while `free_top` is only decreasing.
- **Relaxed is enough everywhere:**
  - Uniqueness comes from the RMW, and visibility comes from the SCH7 edges.
  - The Relaxed load at counter creation reads the settled value or a later one: the window's settle happens-before the task publication, which happens-before the load, so coherence forbids an older value. A read of `<= 0` is therefore true (X1).
- **The EXHAUSTED bit** is per-counter state inside the worker's own `Commands`. It is never shared, so it adds nothing to the race argument. Its truth rests on EM2′ and X1.
- **Provenance:** the reservoir pointer is derived with `&raw const (*self.ptr).entity_master.reservoir` inside `UnsafeEcsCell::entity_counter` (`unsafe_ecs_cell.rs:280-281`). The bit is set and cleared with `map_addr`, which keeps provenance.
- **Aliasing (corrected):**
  - Exclusive systems take no params, so no body holds both a `Commands` and a `&mut EcsMaster`.
  - The two single-thread sites where a claim meets `&mut EntityMaster`:
    - **(i) A hook's `DeferredCommands::spawn`.** It claims through the world pointer the hook already uses for `deferred_hook_queue` (`deferred_master.rs:195-201`, the same SAFETY argument), and no reservoir pointer outlives the call.
    - **(ii) `run_system*` inside `Command::apply`.** The nested `Commands` pointer comes from the nested cell and is dead before the nested queue's apply takes `&mut`.

## Miri and loom
- **Miri:**
  - Unit tests on a reservoir with a small `free_reserve_elems`: three `std::thread::scope` threads × 8 claims over 12 pre-filled entries.
  - **New: `crates/boyko_ecs/tests/miri_em_deferred_recycle.rs`** (`#![cfg(miri)]`), with the pool built through `ThreadPoolBuilder::new().num_threads(2).build()` (precedent: `miri_schedule_parallel.rs:116-117`) and the header's `MIRIFLAGS` copied whole from `miri_schedule_parallel.rs:72`.
    - **Test 1:** a despawner system and a spawner system in unordered sets, one pre-populating frame of 8, then 3 churn frames with C = 4. The data-race detector then sees the dispatcher's push (entry write plus `len`) in window k against the worker's entry read in phase k+1 through the real KE16 publication and completion edges, which the spawn/join unit test cannot (O5).
    - **Test 2:** an `on_despawn` hook spawns a replacement through `DeferredCommands`, over 3 frames. This is the claim-then-push site (i).
    - Acceptance: the output reads `running 2 tests`. `running 0 tests` is a vacuous pass.
  - Run on `+nightly-x86_64-pc-windows-gnu` with the full `MIRIFLAGS`.
  - The existing `tests/miri_entity_store.rs`, `miri_phase19.rs`, `miri_phase22.rs` and `miri_feature2_observers.rs` must stay green.
- **loom:** add `tests/loom_entity_reservoir.rs` (`#![cfg(loom)]`) through the `#[doc(hidden)]` probe.
  - Model 1: 2 claimers over 1 entry. Exactly one gets the recycled id and one gets a fresh id. After settle, `free_top == 0 == free.len()` and `next == base+1`.
  - Model 2: 3 claimers over 2 entries.
  - Model 3: 2 gated counters over 1 entry. Each counter's bit is set only after it observes `<= 0`, and the result is the same as Model 1.
  - loom checks the atomic protocol. The ordering of the plain entry memory is covered by Miri.

## Integration and implementation plan (in order; RED first)
0. **RED-first, on the unfixed tree (d11962a9, before any step below):**
   - Add `crates/boyko_ecs/tests/em_deferred_recycle.rs` with **T-RED-1 and T-RED-2** (specs below).
   - Run it and record both failure messages. Verify ancestry with `merge-base --is-ancestor` before quoting the result.
1. **`memory/vm_column.rs`:**
   - add `precommit(n)`;
   - make `committed_elems` non-test (diagnostics);
   - drop `#[allow(dead_code)]` on `as_ptr` (`:331`).
   
   **1b. `entity/inland_store.rs`:**
   - add `ceiling_slots(&self)`;
   - in `grow_to`, add `debug_assert_eq!(vm.os_len() / SLOT_SIZE, self.ceiling_slots())`.
2. **New `core/entity/entity_reservoir.rs`:**
   - the struct and methods;
   - the Send/Sync SAFETY comments;
   - the `cfg(loom)` aliases;
   - the gated layout asserts;
   - unit tests and the doc-hidden probe.
   
   Register it in `core/entity/mod.rs`.
3. **`core/entity/entity_master.rs`:**
   - Replace `next_entity_id` and `free_entity_ids` (`:64`, `:73`, `:85`, `:101`). Size the free list with `entities_inland.ceiling_slots()` (D6).
   - Rewrite:
     - `allocate_entity` (`:124-148`), as the ticketed allocator plus the plain wrapper;
     - `reserve_entity` (`:186-190`), now `reservoir.claim()`; remove the `dead_code` allow;
     - `reserve_batch` (`:211-224`);
     - the push in `deallocate_entity` (`:378`);
     - `recycled_entity_count` (`:428-430`) and `next_entity_id` (`:437-439`);
     - `clear` (`:459-464`) and `memory_usage` (`:476-479`);
     - `compact` (`:489-500`), sort only, no shrink;
     - `rewind_allocate` (`:502-548`), replaced by the ticket form (D9); delete the arithmetic discriminator and its "heuristic" doc.
   - Fix the layout comment at `:37-42`: `InlandStore` is 48 B (5 fields), not 32 B. Give the rev-2 line map (line 0 = inland + `live_count`, line 1 = reservoir).
   - Update the SEND5 comment (`:558-587`).
   - Invert `reserve_entity_skips_free_list` (`:785-798`).
   - Adapt `rewind_allocate_decrements_next_id_on_fresh_path` (`:703-723`): the "second rewind" leg becomes a compile-time property of the non-`Copy` ticket, so it is deleted with a note.
4. **`system/params/entity_counter.rs`:**
   - Change the field to `Cell<*const EntityReservoir>` with the EXHAUSTED bit. `from_ptr` presets the bit, and `reserve_entity` becomes the gated claim.
   - `reserve_batch` → `mint_fresh_batch`.
   - Remove `Copy`/`Clone` and the `Sync` impl. Rewrite the `Send` SAFETY comment (`:87-100`) to cite SCH7 for the entry reads; "no plain memory access is possible" is now false (O6b).
   - Restate EM1′/EM2′/EM4′/EM6′/X1 in the docs (`:1-30`, `:129-161`).
   - Rebuild the tests at `:221-287` on a small reservoir. `entity_counter_is_send_and_sync` becomes send-only.
5. **`system/unsafe_ecs_cell.rs:268-289`:** project with `&raw const … .reservoir`.
6. **`component/hooks/deferred_master.rs:193-203`:** replace the inline `fetch_add` with `world.entity_master.reserve_entity()` (the ungated claim).
7. **`ecs_master/entity_api.rs:183, :209-224`:**
   - Use `allocate_entity_ticketed`, and call `rewind_allocate(ticket)` on rejection.
   - Delete the `deallocate_entity` fallback: it was a no-op for never-registered slots (`entity_master.rs:361-363`). Delete its false comment with it.
   - Update the C-007 comment block at `ecs_master.rs:1436-1442` and adapt `test_rewind_allocate_restores_fresh_id` (`:1523-1540`).
8. **`commands/spawn_at_command.rs:129-137`:** add a `debug_assert` that `slot.generation() == entity.generation()` when the slot exists.
9. **`commands/command_queue.rs`:**
   - KE7 docs (`:226-242`, `:1381-1388`): ids are not reclaimed because of *escape*, not EM2.
   - Add a sibling test with a pre-populated free list: after `rewind`, the claimed recycled id is not re-issued.
10. **Doc strings:** `commands.rs:129-168` / `:232-244`, `system/params/entities.rs:20`.

    **10b. EM2′-K at the site (O1), comments only:**
    - the SCH7 SAFETY comment in `apply_window_drain` (`schedule.rs:670-676`);
    - the `may_defer` doc (`:139-156`);
    - the KE17 entry in `docs/MEASUREMENT-QUEUE.md` (`:88-89`).
11. **Census re-derivation, counts only** (see "Metrics and validation → Census").
12. **Documentation sweep:**
    - `docs/SYSTEMS.md`, `FEATURE_MAP.md` and `ARCHITECTURE.md` mention EM2 / `free_entity_ids`.
    - Test headers that give EM2 as the reason to avoid `Commands` (O6a): `ke3_query_random_access.rs:296` and `ke2_entities_param.rs:165`. Only the stated reason changes; the tests stay valid.
    - The book page `book/src/architecture/entities-and-generations.md` goes to `doc-writer`.
13. **Side-state audit (critic OPEN Q1).** Grep `\.id\(\)\.0|EntityId\b` over `boyko_render|boyko_ui|boyko_physics|boyko_scene|boyko_app|boyko_input` `src/`: 94 hits in 15 files. Classification:

| Site | Keyed by | Crosses a frame? | Verdict |
|---|---|---|---|
| `boyko_render/src/shadow_atlas.rs:357-366` `PunctualSlotAssignment` | bare `EntityId` | yes (Resource) | **Open Q5.** Safe iff `resolve_shadow_atlas` republishes before `collect_lights` in every frame where the light set can change (`light_plugin.rs:107` orders them within a frame). Reuse already happens on the direct route; this change extends it to `Commands`-spawned lights. |
| `snap_interpolation.rs:142-156`, `visibility_sync.rs:68-86` | deferred commands keyed by bare id, resolved at apply | no | Safe. A successor registers at the earliest one window after its predecessor's despawn window. The only exception is panic re-absorption, which is pre-existing. |
| `particle_system.rs:490` `emitter_seed` | RNG seed input | no | Cosmetic: a reused id reuses its predecessor's seed, the same as the direct route today. |
| `boyko_ui/src/interaction/focus.rs:214, :331, :540` | sort tie-break | no | Safe. |
| `boyko_physics/src/manifold.rs` | doc only (dense `BodyIndex`) | no | Safe (`warm_start.rs:35-41`). |

   Re-run the grep at implementation time; the tree moves.

## Metrics and validation

**Tests on the RED-first harness (`em_deferred_recycle.rs`):**
- **T-RED-1 `commands_churn_recycles_ids_on_the_deferred_route`.** Harness: `App::with_pool(pool(2))` in the census's shape (`alloc_frame_census.rs`, the S2 churn scene, located by name), 1024 static rows, C = 64, 4 warm-up frames, 256 measured frames. Per-frame samples go into a preallocated test resource.
  - (a) Anti-vacuity: spawned = (W+F)·C, despawned ≥ spawned − 2C, live population within ±C.
  - (b) `max(recycled_entity_count()) <= C`.
  - (c) `capacity()` does not change across the window, and is ≤ live + 2C.
  - (d) `committed_slots()` does not change.
  - (e) Anti-vacuity for the fix: the ids spawned in frame k are the ids despawned in window k−1, and at least one handle has generation ≥ 1.
  - (f) Every `.id()` from frame k is valid after update k and reads `wave == k`; wave k−1 handles are invalid after frame k.
  - (g) A stale handle to a recycled id stays invalid after its successor registers.
  - On d11962a9, (b), (c) and (e) fail.
- **T-RED-2 `rejected_create_after_despawning_the_newest_entity_does_not_revive_its_handle`** (C1, direct API, no pool):
  - **Case A, the newest id:**
    - Setup: `arch = create_archetype(&[A, B])`, `e0 = create_entity(arch, [A,B])`, `e1 = create_entity(arch, [A,B])` (newest, id N−1, gen 0), `delete_entity(e1)`.
    - `create_entity(arch, [A])` must return `Err(ArchetypeRejectedEntity)`. Asserting the variant (not `ArchetypeNotFound`) is the anti-vacuity check that `allocate_entity` really ran.
    - Then (a) `next_entity_id() == N`, and (b) `recycled_entity_count() == 1`.
    - Then `e2 = create_entity(arch, [A,B])`: (c) `e2 == Entity(N−1, 1)`, and (d) `!is_entity_valid(e1)`.
  - **Case B, the leak arm:** despawn `e0` (not the newest), then run the same rejected create. (e) `recycled_entity_count() == 1` afterwards.
  - On d11962a9: (a) reads N−1, (c) returns `(N−1, 0) == e1`, (d) fails, and (e) reads 0.
- **T2:** two unordered concurrent spawners plus one despawner, `pool(4)`. Bounded, no duplicate live ids (test-only `HashSet`), every handle valid.
- **T3:** determinism. Two fresh apps, one claimer, 64 frames: identical `Entity` sequences.
- **T4:** `DeferredCommands::spawn` recycles. An `on_despawn` hook spawns a replacement over 64 frames, and `capacity()` stays flat. Its Miri twin is Miri test 2.
- **T5:** a `reserve_entity` escape without spawning leaks exactly that id, and the id is never re-issued.
- **T6:** `spawn_batch_churn_is_bounded`, `#[ignore = "deferred: Stage B batch claims (D7)"]`.
- **T7 (D4, counts not timing):**
  - Unit, in `entity_reservoir.rs`: 8 pre-pushed entries and one counter, 10 000 claims. The first 8 return the entries in LIFO order with their generations, the rest are fresh and contiguous, and `free_top_raw() == -1`.
  - Unit: an empty stack and 10 000 claims give `free_top_raw() == 0` (the bit is preset at creation).
  - Integration: pure-spawn App, C = 64, 16 frames, no despawner. `free_top_raw() == 0` at the end.
- **T9 (O4 site ii) `run_system_inside_command_apply_recycles_ids_despawned_earlier_in_the_same_apply`:** a `Command` whose apply despawns k entities and then `world.run_system(spawner_k)`.
  - The k spawned ids are the despawned ids with generation +1.
  - The stale handles are invalid.
  - `recycled_entity_count()` returns to 0 and `capacity()` does not change.

**Unit tests (reservoir and master):**
- `reserve_entity_claims_free_list_before_minting`
- `overshoot_clamps_at_settle`
- `claim_then_dispatcher_push_does_not_reissue_claimed`
- `rewind_recycled_newest_id_restores_the_free_entry`: the id == `next−1` case from the C1 trace.
- `rewind_recycled_older_id_restores_the_free_entry`
- `rewind_fresh_restores_the_counter`
- `rewind_fresh_after_an_intervening_mint_leaks_instead_of_reissuing`: the R1 refusal arm. Debug builds see a panic from the `debug_assert!`; the test uses `catch_unwind` and branches on `cfg!(debug_assertions)`, as `vm_column.rs:837-843` does.
- 8 threads × 1000 claims over 4000 pre-filled entries: exactly 4000 recycled (with their generations) plus 4000 fresh, 8000 unique.
- A proptest over random {claim (gated, per counter), claim (ungated), allocate_ticketed, rewind(ticket), dealloc(live), register(claimed)} against a model, checking F1–F3, R1 and X1 after every op (`#[cfg_attr(miri, ignore = "miri-slow: …")]`).

**Existing EM tests:**
- Kept: `test_entity_allocation_fresh`, `deallocate_then_allocate_recycles_id_with_bumped_generation`, `reserve_entity_shares_counter_with_allocate_entity` (its free list is empty), `atomic_counter_advances_on_fresh_allocation`, `deallocate_entity_rejects_stale_generation_handle`, `deallocate_unregistered_recycled_id_is_noop_and_preserves_live_count`, `live_count_equals_non_null_inland_count_after_churn`, `xg_b6_slot_address_stable_across_growth`, `test_guard_does_not_consume_recycled_slot`, `commands_reserve_entity_yields_distinct_ids` (`phase11_entity_commands.rs:353`).
- Adapted: the `entity_counter_*` tests, `rewind_allocate_decrements_next_id_on_fresh_path`, `test_rewind_allocate_restores_fresh_id`.

**Mutations that must turn a gate red** (restore each with plain `cp`, then `touch` the file):
- M1: `claim` always mints (the pre-fix behaviour) → T-RED-1 (b), (c) and (e) red.
- M2: skip the clamp → proptest or a debug assert red.
- M3: settle without `truncate` → proptest red (a claimed id is re-issued).
- M4: entries store generation 0 → the SpawnAt `debug_assert` and T-RED-1 (g) red.
- **M5:** `rewind_allocate` picks its arm by `id+1 == next` and ignores `ticket.source` → T-RED-2 (a), (c) and (d) and `rewind_recycled_newest_id_restores_the_free_entry` red.
- **M6:** `from_ptr` does not preset the bit → T7 integration red (reads −16).
- **M6b:** the bit is never set on a failed `fetch_sub` → T7 unit red (reads −9 992).
- **M7:** the bit is set after a *successful* recycled claim → T-RED-1 (b), (c) and (e) red.

**`debug_assert!` sites:**
- F2 in `settle`, and in `claim` when `r > 0`.
- F3 in `push_free`, `pop_free` and `unpop`.
- The generation check in `SpawnAtCommand::apply`.
- The refusal arms of `rewind_allocate`.
- `ceiling_slots` equality in `InlandStore::grow_to`.
- Pointer alignment in `EntityCounter::from_ptr` (bit 0 clear on input).
- A test-only `EntityMaster::check_invariants()` that walks F1–F3 in O(n).

**Census re-derivation (counts only; derive, never widen; locate by name, since line numbers drift, O6c):**
- `alloc_frame_census.rs`, S2 churn row: `realloc_sum: 2` (currently `:2039`) → **0**. This is a structural zero: at rest the free list holds ≤ 64 entries on a `VmColumn`, not a `Vec`.
- Re-measure `dispatch_max: 6` (currently `:2026`). Its +1 headroom was justified by the free-list doubling (the comment block just above it).
- Update the census module-doc passages that name the free-list realloc.
- `alloc_frame_attribution.rs`:
  - Invert C6 (the `// ── C6:` block, currently `:1640-1698`) to `recycled_after <= recycled_before + CHURN_PER_FRAME` and `slots_after == slots_before`.
  - Reword C7's message premise (the `// ── C7:` block, currently `:1700-1739`).
  - Change the verdict row that attributes the churn realloc to FREE.
  - Update the module doc.
- Re-run the setup pins of the scenes that despawn, since one heap `Vec` allocation disappears from setup.

**Benchmarks — only when the owner says the machine is quiet; no verdict until then; none is a merge precondition:**
- `create/delete_entity_10k`: D5's X.G layout sensitivity. This is the only row whose result could reverse a decision in this plan.
- Worker `Commands::spawn`: `profile_spawn_single` including `p12_boyko_reserve_entity_only`, and `comparison_v2`. This is a non-regression check for D4.
- Parallel `get_component_raw` with a concurrent spawner: the false sharing D5 removes.
- **Why D4 needs no pre-merge benchmark (W1):** the choice the critic wanted measured was two RMWs ungated against a load plus one RMW gated. Rev 2's claim has neither cost, and T7 proves the RMW count as an integer. What is left per claim is one AND/TEST on a register and one branch that is predictable within a call. There is no memory operation left to remove, so no timing could argue for a different design.

## Open questions
1. **Owner values call:** must entity ids be replay-deterministic when despawning systems run concurrently? If yes, the fix is draining completions in system-index order in `apply_window_drain` (`schedule.rs:769-835`), which is outside this scope.
2. Should Stage B (D7) be scheduled now, for kernel completeness, or after this lands? No per-frame caller exists today.
3. `Commands::reserve_entity` without a spawn still leaks one id per call (`commands.rs:238-240`). Bevy instead flushes claimed-but-unregistered ids into live empty entities. That would change `reserve_entity`'s semantics, so it is the owner's call.
4. Generation wrap under LIFO (about 2.3 years of per-frame reuse of one slot) is pre-existing and only flagged here.
5. **Render owner:** does `resolve_shadow_atlas` republish `PunctualSlotAssignment` before `collect_lights` in every frame where a punctual light can be spawned or despawned? If it is gated on change detection, a `Commands`-spawned light that reuses a despawned light's id would read the old slot until the gate fires. The same exposure already exists on the direct route.

## Checklist
- **Goal, metrics, justifications, alternatives, trade-offs:** ✓. The W1 correction is stated in D4.
- **Layouts:** native and Miri sizes and offsets are given; asserts are gated (O2); `repr` and alignment are stated; the 48 B comment is fixed.
- **Hot/cold split:** worker line vs dispatcher fields ✓. The EXHAUSTED bit lives in the caller's `Commands`, with no shared state.
- **API:** crate-internal. User-facing signatures are unchanged. `EntityCounter` loses `Copy`/`Sync`, and code outside the crate cannot observe that.
- **Multithreading:** the model, orderings, happens-before proof, X1 and EM2′-K are stated ✓. Send/Sync are stated per type.
- **Edge cases:**
  - An empty free list gives an EXHAUSTED preset and fresh ids with no `fetch_sub`.
  - Overshoot is clamped.
  - `isize` cannot underflow: drift per phase is at most one per counter plus one per ungated hook claim.
  - `with_capacity(c)` for any `c` is covered by `ceiling_slots` (O3).
  - A rejected create restores or leaks, never re-issues (R1).
  - `clear()` with pending queues is pre-existing and unchanged.
- **Generations and drop:** the generation rule ✓. Drop: `VmColumn<Entity>` elements are `Copy` (no element drop), the reservation is released on drop, and `AllocTicket` has no `Drop`.
- **Integration steps:** ✓, including 1b, 10b and 13.
- **Tests, property tests, loom, Miri (including the real-pool file), mutations M1–M7, debug asserts, benchmarks (deferred, not preconditions):** ✓

Files: `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/entity/entity_master.rs`, `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs`, `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/system/params/entity_counter.rs`, `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/system/params/commands.rs`, `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/entity/inland_store.rs`, `D:/wt/joltab/crates/boyko_ecs/src/ecs/memory/vm_column.rs`, `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`, `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/hooks/deferred_master.rs`, `D:/wt/joltab/crates/boyko_ecs/tests/miri_schedule_parallel.rs`, `D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_census.rs`, `D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_attribution.rs`, `D:/wt/joltab/crates/boyko_render/src/shadow_atlas.rs`

---

## Implementation report (developer)

# Implementation: EM2′, recycling entity ids on the deferred route (rev 2)

The RED-first tests failed on the unfixed tree for the stated reasons, the design is implemented, and the tests now pass. The census S2 realloc pin is re-derived to **0**. Nothing is committed or staged.

Two things need your attention first:
- **The machine returned corrupted file reads.** One read of `crates/boyko_rhi_vulkan/tests/sdf_mesh_hybrid_depth.rs:392` gave `8[c]` instead of `x[c]`. That is a single-bit flip (0x78 → 0x38), seen by both `sed` and rustc, so `cargo check --workspace` went red. Minutes later the file read back correctly with the same mtime, and it hashes to its index blob `6b918698…`. That fits page-cache or RAM corruption, which calls into question any build or test result from this machine. The owner should know.
- **Other environment trouble.** C: has 256 MB free and commit headroom is about 1 GB. That caused intermittent rustc failures ("only metadata stub found for `core`", STATUS_STACK_BUFFER_OVERRUN). I had to point `TMP`/`TEMP` at `D:/wt/_targets/tmp` and build with `-j 1`/`-j 2`.

## (1) RED on the unfixed tree
Confirmed with `merge-base --is-ancestor d11962a9 HEAD`; HEAD is d11962a9 with `crates/boyko_ecs/src` clean. Log: `D:/tmp/emrecycle/red_before_fix.log`.
```
T-RED-1: max free list 16576, capacity 1280 -> 17664, committed 16384 -> 32768, live 1088 -> 1088, max generation 0
  (b) x256: frame 0: the free list holds 256 ids after the apply window, more than one frame's 64 despawns
  (c) x256: frame 0: slot store capacity 1344 (window opened at 1280, live 1088); the id-slot store must not grow on a flat population
  (e) x257: frame 0: the 64 ids spawned are not the 64 ids despawned one window earlier (first spawned Some(EntityId(1280)), first despawned Some(EntityId(1152))); the deferred route is not recycling
  (d) x20: frame 236: committed slots 32768 (window opened at 16384)
T-RED-2: 5 clause(s) violated
  (a) case A: next_entity_id is 1 after the rejected create, expected 2
  (b) case A: 0 recycled ids after the rejected create, expected 1
  (c) case A: the next create returned Entity { id: EntityId(1), generation: 0 }, expected ... generation: 1
  (d) case A: the despawned handle Entity { id: EntityId(1), generation: 0 } is VALID again ... generation ABA
  (e) case B: 0 recycled ids after the rejected create of a recycled, not-newest id, expected 1 — the id leaked
```

## (3) GREEN after the fix (dev and release)
```
T-RED-1: max free list 64, capacity 1152 -> 1152, committed 16384 -> 16384, live 1088 -> 1088, max generation 129
test result: ok. 2 passed
```

## (4) Census S2 re-derived
- **Pin:** `realloc_sum` goes from 2 to **0**, and this is structural: the recycled ids live on a `VmColumn` and each frame's 64 spawns claim the previous window's 64 despawns. `dispatch_max` goes from 6 to **5**; the +1 covered the free-list doubling, which no longer exists. The S2 pin is now exactly `app("S2", 2, 4)`.
- **Measured:** release and debug both read 4.031 / 5 with `realloc` 0. GATE GREEN in both profiles.
- **Header updated:** the realloc bullet, the class table, the S2 note, and the "Open" leak bullet. The header now says the census **cannot see** a regression of the leak: the stack's storage is `VirtualAlloc`-backed, so a leak would grow in uncounted commits. I verified this with mutation M1: census stays GREEN, attribution C6 goes RED (`64 -> 141376`), T-RED-1 goes RED.
- **Attribution binary:**
  - C6 is inverted: `recycled_after <= recycled_before + 64`, and the slot store must not change.
  - C7's wording is updated.
  - The churn row and the classification row now read FREE, with a new gating assert |churn − baseline| < 0.6.
  - `verdict::VEC_GROWTH` is removed.
  - Result: 64 → 64 and 12288 → 12288 over 131072 despawns.

## Modified files (all in `D:/wt/joltab`)
- **`crates/boyko_ecs/src/ecs/core/entity/entity_master.rs`:**
  - 3-field layout with the reservoir on line 1 (layout asserts gated as specified).
  - `AllocTicket` / `AllocSource`, `allocate_entity_ticketed`, and a ticket-based `rewind_allocate` (D9 / R1).
  - `reserve_entity` is now `reservoir.claim()`; `deallocate_entity` pushes `Entity(id, g+1)`.
  - `free_top_raw` and `check_invariants` (both `#[doc(hidden)] pub`).
  - SEND5 comment rewritten; the 48 B layout comment fixed; tests adapted, and `reserve_entity_skips_free_list` inverted.
- **`.../entity/inland_store.rs`:** added `ceiling_slots()`, plus the `grow_to` `debug_assert_eq!`.
- **`.../memory/vm_column.rs`:** added `precommit`; `committed_elems` is no longer test-only; the dead-code allows on `as_ptr` and `clear` are dropped.
- **`.../system/params/entity_counter.rs`:** `Cell<*const EntityReservoir>` with the EXHAUSTED bit (preset in `from_ptr`, set on a failed `fetch_sub`); no longer `Copy` or `Sync`; docs, SAFETY comments and tests rebuilt.
- **`.../system/unsafe_ecs_cell.rs`:** `&raw const (*ptr).entity_master.reservoir` projection.
- **`.../component/hooks/deferred_master.rs`:** `DeferredCommands::spawn` now uses the ungated claim.
- **`.../ecs_master/entity_api.rs`:** the ticketed allocate/rewind; the no-op `deallocate_entity` fallback and its false comment are removed.
- **`.../ecs_master/ecs_master.rs`:** C-007 comments and the rewind test.
- **`.../commands/spawn_at_command.rs`:** the EM1′ generation `debug_assert`.
- **`.../commands/command_queue.rs`:** KE7 docs now give "escape" as the reason ids are not reclaimed; new test `rewind_does_not_reissue_a_claimed_recycled_entity`.
- **Docs only:**
  - `.../system/params/commands.rs` and `entities.rs`: doc strings.
  - `.../schedule/schedule.rs`: EM2′-K written at the SCH7 gate and on `may_defer`.
  - `docs/MEASUREMENT-QUEUE.md`: EM2′-K added to the KE17 entry.
- **Test headers:** `crates/boyko_ecs/tests/ke2_entities_param.rs` and `ke3_query_random_access.rs`.
- **Trybuild:** `tests/par_iter_captures_commands_fails.rs`, `tests/par_iter_compile_fail/capture_commands.rs` (docs), and the re-blessed `capture_commands.stderr`.
- **Internal docs:** `docs/SYSTEMS.md`, `docs/FEATURE_MAP.md`, `docs/ARCHITECTURE.md` updated for EM2′. I also re-derived 46 line anchors my edits had shifted; `internal_docs_anchors` is green.
- **Census and attribution:** `crates/boyko_physics/tests/alloc_frame_census.rs` (sha256 `d446be05…`) and `alloc_frame_attribution.rs` (`8d10c14b…`).
- **Line endings:** my Python rewrites turned files into LF; I converted them all back to CRLF.

## New files
- `crates/boyko_ecs/src/ecs/core/entity/entity_reservoir.rs`: `EntityReservoir` (`repr(C, align(64))`), the claim, settle, push, pop and unpop methods, the `cfg(loom)` aliases, the layout asserts, and the Send/Sync SAFETY comments.
- `crates/boyko_ecs/tests/em_deferred_recycle.rs`: T-RED-1 and T-RED-2. Each test collects every violated clause before failing.

## Conformance to plan
- ✅ D1–D9, EM2′-K (step 10b), steps 0–12, and the step-13 grep: 94 hits in 15 files, the same count as the plan.
- ⚠️ **T-RED-1 fixture change.** Rows are seeded with `spawn_one`, not `spawn_batch`. `SpawnBatchCommand` grows capacity to `end + MAX_BATCH_HINT`, so clause (c)'s `<= live + 2C` would have been false even on the fixed tree.
- ⚠️ **`Commands` was actually `Sync` before this change.** `CommandQueue` is auto-`Sync`, so its comment "Sync is NOT implemented" is false. The plan's premise "Commands is already !Sync" was therefore wrong, and this change makes `Commands` `!Sync` for real. The trybuild baseline moved from E0596 to E0277 and I re-blessed it. No code in the workspace relied on `Commands: Sync`.
- ⚠️ **One debug check left out.** `unpop`'s "entry physically at top" check needs a read above `len`, which `VmColumn` does not expose. The release check in `rewind_allocate` (slot null and generation matches) is the guard.
- ⚠️ **Smaller departures:**
  - `EntityCounter::reserve_batch` keeps its name and delegates to `mint_fresh_batch`.
  - A refused rewind panics in debug through `cfg!(debug_assertions)` rather than `debug_assert!(false)`.
  - The fresh allocate path keeps `fetch_add` ("as today").
- ✅ **Mutations run:**
  - M1 (claims never recycle): T-RED-1 RED on (b), (c), (d), (e); attribution C6 RED; census GREEN, as the header now documents.
  - M5 (rewind arm chosen by arithmetic): T-RED-2 RED on (a)–(d).
  - Both restored with `cp` + `touch`; sha256 matches the backups.

## Unsafe blocks
- `entity_reservoir.rs:109` / `:121`: `unsafe impl Send/Sync for EntityReservoir`. Atomics are RMW'd from any thread; plain writes happen only under `&mut` (SCH7 / EM2′-K); happens-before comes from pool publication and completion Acquire.
- `entity_reservoir.rs:178`: read of `free[r-1]`. Since `idx < r <= free_top <= len` (F2), the slot is initialized, immutable for the phase, and handed out at most once by `fetch_sub`.
- `entity_counter.rs:129`: `unsafe impl Send`. The `Cell` is owned by the counter; the plain reads rely on SCH7, not on the pointer type.
- `entity_counter.rs:157` / `:163`: `unsafe fn from_ptr` and the `is_exhausted_hint` load.
- `entity_counter.rs:211`: `&*ptr` in `reserve_entity`.
- `entity_counter.rs:242`: `reserve_batch`.
- `unsafe_ecs_cell.rs:281`: `&raw const` projection with no intermediate reference.
- `unsafe_ecs_cell.rs:290`: the `from_ptr` call.
- Test-only: `entity_counter.rs:280` and `:305`.

## Checks
- ✅ `cargo clippy -j 1 --keep-going --workspace --all-targets -- -D warnings`: exit 0 on the final file contents.
- ✅ `cargo check --workspace --all-targets`: its only error was the corrupted-read target above; clippy covers all targets and passed.
- ✅ `cargo test -p boyko-ecs --tests`: everything passes after the trybuild re-bless (lib: 964 passed).
- ✅ Census and attribution: release and dev both GREEN.
- ✅ `internal_docs_anchors`: green.
- Logs are in `D:/tmp/emrecycle/`.

## Left for the tester
- Not written: T2–T7, T9, the reservoir unit tests, the proptest, `tests/loom_entity_reservoir.rs`, and `tests/miri_em_deferred_recycle.rs`.
- Not run: mutations M2–M4, M6, M6b, M7.
- **The `#[cfg(loom)]` code (`with_mut`) has never been compiled.** Neither Miri nor loom was run.

## Stale items outside my scope
- The comment at `bench_bevy_vs_boyko/benches/profile_spawn.rs:426` still names `next_id_atomic`.
- `book/src/architecture/entities-and-generations.md` still describes `free_entity_ids`; that is for `doc-writer`.
- In `SYSTEMS.md:470`, "fires 386/406" was already stale before this change.
- The false "Sync is NOT implemented" comment in `command_queue.rs`.
- Open Q5 (render owner, `PunctualSlotAssignment`) is still open.

## Ready for code review

---

## Code review (code-reviewer)

{
 "verdict": "APPROVED. I found no blocking defects. I read every changed kernel file in D:/wt/joltab (the working tree on top of d11962a9) and traced the protocol against its call sites. The EXHAUSTED bit, loom and Miri are still unverified and must be covered before merge; that is the tester's gate (see the non-blocking items).\n\nBuild checks, both run by me on the current contents with the owner's environment:\n- `cargo check -j 2 --workspace --all-targets`: GREEN (Finished in 1m18s). The E0608 `8[c]` error is only in the developer's older log. Today the file compiles, which fits their bit-flip read diagnosis.\n- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0. It finished in 1.3 s because it hit the developer's cache from 07:22. No source file has changed since then: entity_master.rs and entity_reservoir.rs are byte-identical to the developer's `.good` backups (checked with `cmp`).\n\nDid the RED-first tests fail for the right reason? Yes.\n- `D:/tmp/emrecycle/red_before_fix.log` is timestamped 06:24, six minutes before the first `.orig` source backup (06:30).\n- T-RED-2 on the old tree gives `next_entity_id 1` and `Entity(1,0)` VALID again. Only the old `id+1 == next` arithmetic arm can produce that: it is the C1 generation ABA.\n- T-RED-1 gives max generation 0, the free list at 16576 and capacity 1280 -> 17664. That is the deferred-route leak, with anti-vacuity clause (e) green only on the fix.\n- M5 turns Case A red; Case B stays green, which is expected because a not-newest id takes the Recycled arm under arithmetic too. M1 turns T-RED-1 and attribution C6 red, and the census stays green; the census header now says so.\n- The census S2 pin is now `app(\"S2\", 2, 4)`: `dispatch_max` 2*2+1 = 5 (tightened from 6) and `realloc_sum` 0. It was re-derived, not widened.\n\nUnsafe blocks, each traced:\n- `try_claim_recycled` (the read of `free[r-1]`): `r > 0` implies `len >= r > 0`, so the column is materialized. The index satisfies `idx < r <= top_phase_start <= len` (F2). The entry was written in an earlier window, and that window happens-before this phase through task publication. `fetch_sub` hands each positive value out once, because `free_top` only decreases within a phase. Sound.\n- EntityCounter `from_ptr`, `&*ptr` and `split`: `map_addr` keeps provenance, and bit 0 is free because `align_of == 64`, which is const-asserted on every build. `!Sync` is load-bearing: the `Cell` write in `reserve_entity(&self)` would race without it, and the re-blessed trybuild (E0277) shows the compiler now enforces it.\n- The `&raw const (*ptr).entity_master.reservoir` projection creates no intermediate reference.\n- `unsafe impl Send/Sync` for EntityReservoir: every plain write takes `&mut`.\n\nProtocol walk:\n- Settle-before-push/pop is correct across unsettled phases: a claimed entry above `free_top` is truncated by the next push/pop, and entries below it keep F3.\n- A negative `free_top` is harmless: `claimable()` clamps it and the preset reads `<= 0`.\n- The hook claim inside `delete_entity` is settled by the push that follows it.\n- Rewind: `create_entity` has no step between allocate and rewind that touches the entity store, so the debug refusal arm cannot be reached. A Recycled restore needs a null slot with a matching generation, so the id cannot be re-issued (R1).\n- The free stack is capped: its entries are distinct and each id is below `inland.len()`, which is at most `ceiling_slots` = `reserve_elems`, so the VmColumn exhaustion assert cannot fire.\n- Scheduling: the SCH7 gate (`schedule.rs:680`) is the only `&mut` window during a phase. Condition evaluation, `run_cached_system` and the nested `run_system`-in-apply all run with no worker in flight.\n- EM3 still holds: a claim never touches `entities_inland`.\n- Layout: the pins compile natively (InlandStore 48 B, `live_count` at +48, reservoir at +64, 192 B), and the worker RMW line no longer shares a line with the inland header.\n\nThe plan is followed: D1–D9, EM2′-K written at the SCH7 site and on `may_defer`, and D6 `ceiling_slots` with its `grow_to` assert. The deviations are reported and reasonable: T-RED-1 seeds with `spawn_one` so clause (c) can fail, the unpop debug check is dropped because VmColumn exposes no read above `len`, and the plan's premise that `Commands` was already `!Sync` was wrong.\n\nWhat to keep:\n- The ticket makes a second rewind fail to compile, instead of relying on a runtime heuristic.\n- The integer RMW-count design of T7, so no timing is needed.\n- `debug_check_free_entry` at pop.\n- The header's admission that the census cannot see the leak.\n- CRLF line endings preserved.\n\nOpen questions:\n- (1) `EcsMaster::clear()` applied in the same window before a pending `SpawnAt(X, g+1)` now fails the new EM1′ `debug_assert` (the cleared slot has generation 0). The plan files this under the pre-existing \"clear with pending queues\" class. Does the owner want it documented?\n- (2) Plan Open Q5 (`PunctualSlotAssignment`, keyed by a bare `EntityId`, `crates/boyko_render/src/shadow_atlas.rs:363`) is still with the render owner. This change extends id reuse to lights spawned through `Commands`.\n- (3) `EcsMaster::create_entity_at` is `pub` and takes an arbitrary `Entity`. It can register an id that is currently on the recycled stack. This is pre-existing and out of scope, but it breaks F3, which EM2′ now relies on for worker claims.",
 "blocking": [],
 "non_blocking": [
  "Tester, before merge: nothing covers the D4 EXHAUSTED bit. The 'set on a failed fetch_sub' branch (`entity_counter.rs:212-217`) is reached by no test. T-RED-1 claims exactly 64 of 64 entries each frame. The `entity_counter` unit tests start on an empty reservoir, so the bit is preset and `fetch_sub` never runs. Still missing: T7 (`free_top_raw()` equal to 0 or -1), mutations M6, M6b and M7, the reservoir unit tests, T2-T5 and T9, the proptest, `tests/loom_entity_reservoir.rs` and `tests/miri_em_deferred_recycle.rs`. Failure if skipped: a regression in the bit (for example M7, setting it after a successful claim) would ship with every current test green. Confidence: CONFIRMED by reading the tests.",
  "The `cfg(loom)` arm of `crates/boyko_ecs/src/ecs/core/entity/entity_reservoir.rs` (`with_mut` on `AtomicIsize`/`AtomicUsize`) has never been compiled. It is now on the path of the EXISTING leg `crates/boyko_ecs/tests/loom_term_list.rs`, because a `--cfg loom` build compiles the whole library. Runtime is unaffected: that test builds no world, so no loom atomic is created outside a model. Failure: if the arm does not compile, the existing loom leg breaks. Confidence: PLAUSIBLE only; loom 0.7 does have `AtomicIsize` and `with_mut`. Settle it by building `loom_term_list` under `--cfg loom`.",
  "Only `cargo test -p boyko-ecs --tests` was run. Commands-spawned entities now reuse ids and carry generations above 0, while downstream crates (physics, render, ui, app, scene) may assume monotone or generation-0 ids from `Commands`. The workspace suite (`cargo test --workspace --all-targets --no-fail-fast`) is still owed by the tester. Confidence: PLAUSIBLE; I found no such assumption, but I did not audit every test.",
  "`crates/boyko_ecs/tests/em_deferred_recycle.rs:27` carries a file-level `#![allow(clippy::disallowed_types)]`, but the file uses none of the disallowed types (HashMap, HashSet, Mutex, RwLock, Rc); grep confirmed. It adds a false entry to the one-grep census of exceptions and would silently accept a future HashMap. Remove it. Confidence: CONFIRMED.",
  "`Commands<'_>` changed from `Sync` (auto: `&mut CommandQueue` plus the old `unsafe impl Sync for EntityCounter`) to `!Sync`. This is a public API change: `&Commands` can no longer be captured by `Fn + Sync` closures to call `reserve_entity(&self)`. No workspace user and no doc example depends on it (grep), but the change should be recorded in the changelog and handed to `doc-writer` together with the stale `book/src/architecture/entities-and-generations.md`. Confidence: CONFIRMED by the re-blessed trybuild stderr.",
  "Stale comments in files this diff touches. `command_queue.rs` still says `CommandQueue` 'Sync is NOT implemented', but it is auto-Sync (the developer flagged this). `entity_counter.rs:60-65` justifies `#![allow(dead_code)]` with 'until Wave C wires the Commands::spawn return path', which is long since wired. Both are documentation only, but the allow now covers code that is live. Confidence: CONFIRMED."
 ]
}
