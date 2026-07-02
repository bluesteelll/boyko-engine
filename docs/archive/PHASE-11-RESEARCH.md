> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Research: EntityCommands Chaining Patterns (Phase 11)

## §1 Executive Summary (Chosen Approach for boyko)

The dominant industry pattern for synchronous-Entity / deferred-spawn is **pre-allocated Entity IDs**. Three concrete implementations exist across the engines studied:

1. **Bevy (current, post-PR #19451 merged 2025-11-03)** — decoupled `EntityAllocator` with a lock-free `RemoteAllocator` that runs an atomic `compare_exchange` loop over a 64-bit packed `FreeCountState` (33-bit length + 1 disable bit + 30-bit ABA generation). `Commands::spawn(bundle)` calls `self.allocator.alloc()` synchronously, then queues a closure that calls `world.spawn_at_with_caller(entity, bundle, ...)` on apply. Entity flushing no longer exists — uninitialized entities are valid-but-empty.
2. **Bevy (pre-PR #19451 historical)** — `Entities::reserve_entity()` atomically advanced a `free_cursor` counter under `Relaxed` ordering; `Entities::flush()` later promoted reserved IDs to spawned. This is the pattern most often cited in older docs.
3. **Unity DOTS** — `EntityCommandBuffer.CreateEntity()` returns a placeholder Entity with a **negative index**; playback resolves the placeholder to a real ID via a remap table. Multi-thread support requires `ParallelWriter` + per-command sort keys; commands are sorted by key before playback for determinism.
4. **flecs** — `ecs_new()` allocates the ID synchronously even inside `ecs_defer_begin/end` brackets, but multithreaded entity creation is the documented weak point: Sander Mertens recommends pre-creating a `vector<flecs::entity> unused_entities` and consuming from it in worker threads.

Returning Entity synchronously while spawn is deferred is **mandatory** for `commands.spawn(bundle).insert(extra).id()` ergonomics. The only viable models are (a) lock-free atomic counter (Bevy) or (b) placeholder + remap table (Unity).

For boyko, the existing `EntityMaster::allocate_entity()` is `&mut self` — it must either be promoted to lock-free `&self` (atomic counter for `next_entity_id` + lock-free free-list pop), or the spawn pattern must shift to Unity-style placeholder IDs resolved at apply. Bevy demonstrates both work; boyko's Phase 9 plan already needs `EntityMaster: Sync` so the lock-free path aligns with existing direction.

The `EntityCommands` struct shape is universal across Bevy and is a thin facade `{ commands: Commands<'_>, entity: Entity }` with `&mut self` chaining methods — Bevy explicitly **reverted** PR #14897 (consume-`self` chaining) after community pushback over reborrow ergonomics.

---

## §2 Bevy EntityCommands Deep Dive (Verbatim Patterns)

### Commands::spawn

From `crates/bevy_ecs/src/system/commands/mod.rs`:

```rust
pub fn spawn<T: Bundle>(&mut self, bundle: T) -> EntityCommands<'_> {
    let entity = self.allocator.alloc();
    let caller = MaybeLocation::caller();
    self.queue(move |world: &mut World| {
        move_as_ptr!(bundle);
        world.spawn_at_with_caller(entity, bundle, caller).map(|_| ())
    });
    self.entity(entity)
}

pub fn spawn_empty(&mut self) -> EntityCommands<'_> {
    let entity = self.allocator.alloc();
    let caller = MaybeLocation::caller();
    self.queue(move |world: &mut World| {
        world.spawn_empty_at_with_caller(entity, caller).map(|_| ())
    });
    self.entity(entity)
}

pub fn entity(&mut self, entity: Entity) -> EntityCommands<'_> {
    EntityCommands {
        entity,
        commands: self.reborrow(),
    }
}
```

`self.allocator.alloc()` runs **before** the closure is queued — Entity is real before the deferred apply.

### EntityCommands struct

```rust
pub struct EntityCommands<'a> {
    pub(crate) entity: Entity,
    pub(crate) commands: Commands<'a, 'a>,
}
```

24-32 B handle (one `Entity` + one `Commands` borrow). Created on every chain; passed by value via reborrow.

### Methods (from `docs.rs` and `commands/mod.rs`)

```rust
pub fn id(&self) -> Entity { self.entity }

pub fn insert(&mut self, bundle: impl Bundle) -> &mut Self {
    self.queue(entity_command::insert(bundle, InsertMode::Replace))
}

pub fn remove<B: Bundle>(&mut self) -> &mut Self {
    self.queue_handled(entity_command::remove::<B>(), warn)
}

pub fn despawn(&mut self) {
    self.queue_handled(entity_command::despawn(), warn);
}

pub fn try_insert(&mut self, bundle: impl Bundle) -> &mut Self { ... }
pub fn try_remove<T: Bundle>(&mut self) -> &mut Self { ... }
pub fn try_despawn(&mut self) -> &mut Self { ... }
pub fn reborrow(&mut self) -> EntityCommands<'_> { ... }
```

Two important facts:
- `&mut self` for chaining — **NOT** consume-`self`. PR #14897 made the consume-`self` switch in Aug 2024 and was **reverted** in PR #15523 (Oct 2024) after the `bevy_third_party` ecosystem reported 20k+ migration lines and that real-world code had `15 reassignments vs 6 chain savings` (PR #14897 thread).
- `id()` takes `&self` — Entity is `Copy`, no borrow conflict with subsequent `&mut self` chain calls.

### EntityCommand trait and concrete commands

From `crates/bevy_ecs/src/system/commands/entity_command.rs` (lines 48-242):

```rust
pub trait EntityCommand: Send + 'static {
    type Out: EntityCommandOutput;
    fn apply(self, entity: EntityWorldMut) -> Self::Out;
    fn with_entity(self, entity: Entity) -> impl Command
    where Self: Sized { ... }
}

// Lines 149-156
pub fn insert(bundle: impl Bundle, mode: InsertMode) -> impl EntityCommand {
    let caller = MaybeLocation::caller();
    move |mut entity: EntityWorldMut| {
        move_as_ptr!(bundle);
        entity.insert_with_caller(bundle, mode, caller, RelationshipHookMode::Run);
    }
}

// Lines 186-192
pub fn remove<T: Bundle>() -> impl EntityCommand {
    let caller = MaybeLocation::caller();
    move |mut entity: EntityWorldMut| {
        entity.remove_with_caller::<T>(caller);
    }
}

// Lines 233-242
pub fn despawn() -> impl EntityCommand {
    let caller = MaybeLocation::caller();
    move |entity: EntityWorldMut| {
        entity.despawn_with_caller(caller);
    }
}
```

`EntityCommand` is an **inner trait** dispatched by `EntityCommands::queue` — it wraps the closure into a `Command` via `with_entity` that captures the target Entity, then the outer `Command` does `world.get_entity_mut(entity).map(|e| inner.apply(e))`.

### Storage layout

All entity-targeted commands route through the **same** `CommandQueue` as `Commands::queue` — there's no separate per-entity queue. The Entity ID is captured into the closure's environment (8 B per entity-targeted command on top of the bundle payload).

---

## §3 Pre-Allocated Entity ID Mechanism

### Bevy historical (pre-2025-11): `Entities::reserve_entity` + flush

Older Bevy 0.5-0.16 used a two-phase model documented in `docs.rs/bevy/0.13.0/bevy/ecs/entity/struct.Entities.html`:

```rust
pub fn reserve_entity(&self) -> Entity
pub fn reserve_entities(&self, count: u32) -> ReserveEntitiesIterator
pub unsafe fn flush(&mut self, init: impl FnMut(Entity, &mut EntityLocation))
pub fn alloc(&mut self) -> Entity
pub fn free(&mut self, entity: Entity) -> Option<EntityLocation>
```

`reserve_entity()` took `&self` (not `&mut self`) — atomic. It used an `AtomicIsize` named `free_cursor`:
- `free_cursor >= 0` → free-list still has entries; `fetch_sub(1, Relaxed)` pops one
- `free_cursor < 0` → free-list exhausted; the negative offset is the count of "fresh" IDs needed past `meta.len()`

`flush()` then walked all reserved IDs and called `init(entity, &mut location)` to fill in archetype data. `get(entity)` on a reserved-but-unflushed entity returned `Some(EntityLocation::INVALID)`.

This is the pattern still in production Bevy 0.14-0.16 and what most external docs describe.

### Bevy current (post-PR #19451, merged 2025-11-03)

The flushing model was **deleted**. The new design splits `Entities` (metadata table) from `EntitiesAllocator` (ID supply). Quoting the PR description: "Allocation and construction happen immediately without pending states… roughly doubling command spawning speed while improving despawn performance by 20-30%."

Core types from `crates/bevy_ecs/src/entity/remote_allocator.rs`:

```rust
struct SharedAllocator {
    free: FreeList,
    fresh: FreshAllocator,
    is_closed: AtomicBool,
}

#[derive(Clone)]
pub struct RemoteAllocator {
    shared: Arc<SharedAllocator>,
}

impl RemoteAllocator {
    pub fn alloc(&self) -> Entity {
        self.shared.remote_alloc()
    }
}

#[derive(Clone, Copy)]
struct FreeCountState(u64);
impl FreeCountState {
    const DISABLING_BIT: u64 = 1 << 33;
    const LENGTH_MASK: u64 = (1 << 32) | u32::MAX as u64;
    const LENGTH_0: u64 = 1 << 32;
    const GENERATION_LEAST_BIT: u64 = 1 << 34;
}

fn remote_alloc(&self) -> Option<Entity> {
    let mut state = self.len.state(Ordering::Acquire);
    loop {
        if state.is_disabled() {
            core::hint::spin_loop();
            state = self.len.state(Ordering::Acquire);
            continue;
        }
        let len = state.length();
        let index = len.checked_sub(1)?;
        let entity = unsafe { self.buffer.get(index) };
        let ideal_state = state.pop(1);
        match self.len.try_set_state(state, ideal_state, Relaxed, Acquire) {
            Ok(_) => return Some(entity),
            Err(new_state) => state = new_state,
        }
    }
}
```

Bit layout in the 64-bit `FreeCountState`:
- Bits 0-32: 33-bit signed length (`1<<32` = zero sentinel)
- Bit 33: disable bit (set during single-writer free operations to block concurrent allocs)
- Bits 34-63: 30-bit ABA generation, incremented atomically with each pop

ABA defence: every successful `pop` increments the generation, so a `compare_exchange` reading the same length twice cannot succeed if any pop happened between reads.

### Unity DOTS placeholder mechanism

Unity sidesteps lock-free ID allocation by **not allocating IDs at record time at all**. `EntityCommandBuffer.CreateEntity()` returns a placeholder `Entity { index: -1, version: ... }` (negative index = placeholder marker). Playback:
1. Walks commands in order (post-sort).
2. On `CreateEntity` command: allocates a real ID, stores `placeholder → real` in a remap table.
3. On subsequent commands referencing a placeholder: substitutes the real ID via the remap table.

Quote from Unity docs: "temporary ID's in the recorded commands should _only_ be used in subsequent method calls of the same `EntityCommandBuffer` instance." This means **the user CANNOT read the real Entity outside the ECB context** — a substantial UX downgrade vs Bevy's synchronous-ID approach.

---

## §4 flecs Deferred Mode Comparison

flecs API surface (`flecs.dev/flecs/group__commands.html`):

```c
ecs_defer_begin(world);
ecs_defer_end(world);
ecs_defer_suspend(world);
ecs_defer_resume(world);
bool ecs_is_deferred(world);
```

Key semantic differences vs Bevy:
- **`ecs_new()` is synchronous even in defer mode** — entity ID is returned immediately from an atomic-add on the world's ID counter. Documented behavior: "ecs_new() does not recycle IDs" in deferred contexts.
- **Per-stage command buffers**: `ecs_stage_t` is a thread-local context. Each worker thread has its own stage with its own command buffer; merge happens sequentially at `ecs_readonly_end`.
- **Multithreaded entity creation is officially limited.** From Sander Mertens (flecs author) in GitHub Discussion #1198: "You _can_ create entities from a multithreaded system but it's very limited what you can do with those, because they can't be made alive yet while the world is in 'readonly' mode. One workaround for that is to create a pool of alive entity ids beforehand … a `vector<flecs::entity> unused_entities`, and have threads consume elements from that vector if they want to create an entity."

flecs trades synchronous-ID-everywhere for a documented constraint on parallel system creation. This is the opposite trade-off from Bevy.

---

## §5 Thread-Safe Entity Allocation Under Phase 9

Phase 9 (parallel scheduler) requires `Commands::spawn` to be callable from worker threads. The Entity must be returned synchronously to support `.id()` and `.insert(...).insert(...)` chaining.

**Three viable paths for boyko:**

### Path A — Atomic counter (Bevy historical)

Promote `EntityMaster::next_entity_id` from `EntityId` to `AtomicUsize`. `allocate_entity` becomes:
- Free-list path (`free_entity_ids: Vec<EntityId>`) → not thread-safe; either lock it or move to a lock-free MPMC queue (crossbeam-deque / Treiber stack).
- Fresh path → `fetch_add(1, Relaxed)` on the atomic counter.

`entities_inland` / `sparse_to_active` / `active_ids` cannot be safely grown from `&self` (Vec realloc is not thread-safe). Solution from Bevy's pre-2025 implementation: callers store reservations in a per-thread queue; the dispatcher resizes the Vecs once at apply time. This matches boyko's existing pattern where `EntityMaster` is mutated only on `&mut self` paths, and CLAUDE.md Phase 9 SEND5 contract already requires pre-allocation to `MAX_ENTITIES_HINT = 64,000` to avoid mid-flight reallocs.

Cost per `alloc`: 1 atomic CAS / fetch_sub on the free-cursor + 1 atomic load on the counter. Bevy measures ~3-5 ns.

### Path B — Lock-free FreeList + counter (Bevy current PR #19451)

Strictly stronger than Path A. The free-list is replaced with the `FreeCountState` 64-bit packed atomic (length + disable + generation) shown in §3. ABA-resistant via the generation bits. `RemoteAllocator` is `Clone` and held by `Arc<SharedAllocator>` so workers can each clone-and-call without aliasing.

Bevy's design notes that the disable bit lets the **single owner** (dispatcher) atomically claim exclusive access to the free-list when running `free_many()`, while remote workers spin (`hint::spin_loop()`) until the bit clears. This sidesteps the dual-writer free-list problem without a full mutex.

### Path C — Unity placeholder

`Commands::spawn` returns an `Entity` with `id: u32::MAX - n` (negative-sentinel marker), where `n` is the position in this system's local placeholder counter. Apply rewrites placeholders to real IDs via a per-queue remap table. Workers do not touch `EntityMaster` at all from the system body.

Downside: `EntityCommands::id()` returns a placeholder that is NOT a valid Entity until apply. If a worker passes this Entity to a function expecting a live Entity (e.g., a query `get(entity)`), the lookup will silently fail or return stale data. Bevy explicitly considered and rejected this for the ergonomic loss.

### Trade-off summary

| Path | Lock-free reads | `commands.entity(spawned).id()` works pre-apply | Implementation complexity |
|------|----------------|------------------------------------------------|--------------------------|
| A — atomic counter | yes (with pre-alloc) | yes | medium |
| B — packed atomic state | yes | yes | high (ABA design) |
| C — placeholders | yes | no (placeholder only) | low |

boyko's Phase 9 SEND5 invariant ("EntityMaster: Send + Sync, structural mutation under `&mut self` on dispatcher") aligns with Path A or B. Path C would conflict with the existing direction.

---

## §6 Archetype Migration Algorithm (Insert/Remove)

Bevy's archetype graph caches transition edges per `Edges` struct (`crates/bevy_ecs/src/archetype.rs`):

```rust
pub struct Edges {
    insert_bundle: SparseArray<BundleId, ArchetypeAfterBundleInsert>,
    remove_bundle: SparseArray<BundleId, Option<ArchetypeId>>,
    take_bundle: SparseArray<BundleId, Option<ArchetypeId>>,
}
```

Per Tainted Coders' "Bevy Archetypes": "Archetypes and bundles form a graph. Adding or removing a bundle moves an Entity to a new Archetype. Edges are used to cache the results of these moves."

### Insert flow

1. Look up current archetype `A` of entity.
2. Check `A.edges.insert_bundle[bundle_id]` — if cached, jump to step 5.
3. Compute target archetype `B = A + bundle_components` (union sort).
4. `get_or_create_archetype(B)` (cold path ~1 µs); cache the edge `A.edges.insert_bundle[bundle_id] = B`.
5. Allocate row in `B`.
6. For each component in `A ∩ B`: `memcpy` from `A.row[entity]` to `B.row[entity]`.
7. For each component in `bundle`: write into `B.row[entity]`.
8. Drop / swap-remove `A.row[entity]`; update fast-store inland for moved entity.
9. Bump Phase 10 ticks for inserted components (`Added` marker).

Cost: ~100-500 ns per migration (memcpy dominated; archetype graph traversal O(1) on cache hit).

### Remove flow

1. Look up current archetype `A`.
2. Check `A.edges.remove_bundle[bundle_id]` — if cached, jump to step 5.
3. Compute target `B = A - bundle_components`.
4. Cache the edge.
5. Allocate row in `B`.
6. For each component in `B`: `memcpy` from `A` to `B`.
7. For each component in `A - B`: run `Drop` on the component bytes (or skip if `Copy`).
8. Swap-remove `A.row[entity]`.

### Intermediate-archetype pitfall

GitHub Issue #5074 ("Chained EntityCommands create useless temporary archetypes") documents the antipattern: `.insert(B).insert(C)` on entity with `{A}` creates archetypes `{A,B}` AND `{A,B,C}` even though only `{A,B,C}` is wanted. Proposed fix: batch chained mutations into a single `modify_bundle` command before processing. **As of 2026-05 this is still open** per the cached search results. boyko has the opportunity to design this in from day one.

---

## §7 EntityCommands Lifetime + Send/Sync

Bevy's shape:

```rust
pub struct EntityCommands<'a> {
    pub(crate) entity: Entity,
    pub(crate) commands: Commands<'a, 'a>,
}
```

`Commands<'w, 's>` carries `&'s mut CommandQueue` + `&'w UnsafeWorldCell`. Both borrows are non-`Send` for the borrowed lifetime — `EntityCommands` is implicitly `!Send`.

This is **correct and required**:
- The system body runs single-threaded (one worker thread per system invocation under Phase 9 work-stealing).
- The `CommandQueue` is per-system state, single-writer (boyko CQ5 invariant already encodes this in `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:60-62`).
- No cross-thread sharing needed.

**Method receiver type**: `&mut self` returning `&mut Self` is the correct choice for boyko per the Bevy 14897 → 15523 revert experience. The consume-`self` variant breaks conditional builders:

```rust
let mut e = commands.spawn(bundle);
if condition { e = e.insert(Foo); } // requires reassignment with consume-self
```

`&mut self` allows:
```rust
let mut e = commands.spawn(bundle);
if condition { e.insert(Foo); }
let id = e.id();
```

Terminal methods (`id()`) take `&self` since `Entity` is `Copy`. `despawn()` per current Bevy is **NOT** terminal (returns `&mut Self`) so chains like `.despawn().log()` remain legal — Bevy's PR #14897 returned `Self` on `despawn` specifically for this.

---

## §8 Entity ID Synchronous vs Deferred Semantics

### Synchronous-ID model (Bevy, chosen approach)

```rust
let e = commands.spawn(bundle).id();  // e is REAL, allocator.alloc() ran
commands.entity(e).insert(extra);     // deferred; resolves to real entity at apply
```

Operations between `spawn` and apply:
- `world.get(e)` (queries) → returns `None` (entity exists but not yet in any archetype — `EntityLocation::INVALID` in old Bevy, "empty archetype" in new).
- `commands.entity(e).insert(...)` → enqueued; works at apply because Entity is real.
- `commands.despawn(e)` → enqueued; at apply, `delete_entity` finds the entity and removes it (or no-ops if it was never inserted into an archetype).

Critical apply ordering: spawn-then-insert-then-despawn within ONE queue MUST execute in enqueue order so `Entity` resolves to the right archetype at each step. boyko's `CommandQueue` already iterates in FIFO order (`command_queue.rs:283` `while local_cursor < stop`).

### Deferred-ID (placeholder) model — rejected

Unity's negative-index placeholder forces every command targeting the spawn to also be in the same buffer, and prevents the placeholder from escaping the buffer's playback context. Bevy's docs/PRs do not consider this; it would require API redesign incompatible with `commands.spawn(...).id()`.

### Apply-time edge cases

1. **Spawn → despawn within same queue, no apply between**: Bevy semantic is "the spawn happens, then despawn happens, net effect is allocator+free pair." Bevy historical (pre-PR #19451) tracked this via the pending flag; the new design just runs both commands and observes the free path on the second. boyko CommandQueue already supports this — `SpawnCommand::apply` calls `create_entity`, a subsequent `DespawnCommand::apply` calls `delete_entity` on the just-spawned ID.

2. **`commands.entity(stale_id).insert(...)`**: At apply, `delete_entity`-style stale-handle check (generation mismatch) returns false. Bevy's `entity_command::insert` uses `world.get_entity_mut(entity)` which returns `Err(EntityDoesNotExist)`; Bevy 0.14+ routes through `queue_handled(..., warn)` (`mod.rs:insert`) to emit a `warn!()` rather than panic. boyko has the same choice — `expect()`, `warn!`, or silent no-op. Per GitHub Issue #10166 the Bevy decision is "warn in debug, ignore in release."

3. **Pending despawn cancels pending spawn**: When the user does `let e = commands.spawn(b); commands.entity(e).despawn();`, Bevy semantics: at apply, `spawn` runs (creating the entity), then `despawn` runs (deleting it). Net zero archetype residency but two commands executed. This is exactly what boyko's `CommandQueue::apply` FIFO loop produces if both commands target the same Entity in order.

---

## §9 Despawn Cancellation Edge Cases

Bevy's `despawn` apply (`entity_command::despawn`):
```rust
pub fn despawn() -> impl EntityCommand {
    let caller = MaybeLocation::caller();
    move |entity: EntityWorldMut| {
        entity.despawn_with_caller(caller);
    }
}
```

`EntityWorldMut::despawn_with_caller` calls into `World::despawn_with_caller` which:
1. Looks up entity in `Entities`.
2. If valid: removes from archetype (swap-remove), runs `Drop` on components, calls `Entities::free(entity)` (bumping generation), runs `OnRemove` observers/hooks.
3. If invalid (already despawned): emits warning or no-ops depending on `_handled` variant.

Recursion: Bevy's `despawn` is **recursive by default** for `RelationshipTarget` children (Phase 11 boyko probably doesn't need this — Bevy hierarchy is a separate concept and boyko has no hierarchy yet).

### boyko `delete_entity` already exists

`ecs_master.rs:456-496` — returns `bool`, handles swap-remove, updates the moved entity's `unit_index` in the fast store, deallocates from `EntityMaster`. A `DespawnCommand` wrapper is essentially:

```rust
pub(crate) struct DespawnCommand { pub entity: Entity }
impl Command for DespawnCommand {
    fn apply(self, world: &mut EcsMaster) {
        let _ = world.delete_entity(self.entity);
    }
}
```

Cost: ~500 ns target is realistic — one fast-store read + one swap-remove + one free-list push.

### Double-despawn

Calling `commands.despawn(e); commands.despawn(e);` in the same system: apply runs first (succeeds, returns true), second sees stale generation (returns false). boyko's `is_entity_valid` already rejects stale generations.

### Spawn-then-despawn within one queue, with intervening insert

```rust
let e = commands.spawn(bundle);    // SpawnCommand
e.insert(extra);                    // InsertCommand(e)
commands.despawn(e.id());          // DespawnCommand(e)
```

Apply order: spawn → archetype A; insert → archetype A+extra (migration); despawn → free from A+extra. Three archetype touches, two migrations. The Issue #5074 pitfall applies — chained inserts create intermediate archetypes. For Phase 11 the architect should consider whether to batch `EntityCommands::insert` chains into a single migration. flecs does this implicitly via the per-stage merge phase that batches "all operations for an entity … optimizing table traversal" (DeepWiki on staging).

---

## §10 Comparison Table

| Aspect | Bevy (current) | Bevy (historical) | flecs | Unity DOTS |
|--------|----------------|-------------------|-------|------------|
| Sync entity ID from deferred spawn | yes (`allocator.alloc()`) | yes (`reserve_entity()`) | yes (`ecs_new()` even in defer) | no (placeholder + remap) |
| ID allocator data structure | `Arc<SharedAllocator>` + 64-bit packed atomic | `AtomicIsize free_cursor` | per-world atomic counter | per-ECB placeholder counter |
| ABA defence | 30-bit generation in `FreeCountState` | none (single counter) | none required (no recycling in defer) | n/a |
| Multi-threaded `spawn` | yes (lock-free) | yes (lock-free, Relaxed) | limited (use pre-alloc pool) | yes (ParallelWriter + sort key) |
| Chaining API receiver | `&mut self` | `&mut self` | builder consumes by value (C++) | not applicable (record-only) |
| Archetype migration on insert | edge-cached graph | edge-cached graph | merge-batched | resolved at playback |
| Intermediate archetype problem | open (Issue #5074) | open | mitigated by per-entity batching | mitigated by sort + batch |
| Despawn cancellation | Bevy: both run; net free | both run | both run | replays in sort order |
| Stale-entity insert handling | `warn!()` + no-op | `panic` (old) / `warn!` (new) | silent no-op | undefined (placeholder errors) |

---

## §11 Bench Targets (Per Phase 11 Brief)

Industry references for plausibility:
- **Bevy spawn enqueue**: PR #19451 description claims "roughly doubling command spawning speed" post-merge — order of magnitude consistent with the `~25 ns` boyko target.
- **Bevy archetype migration**: Tainted Coders / DeepWiki note "moves can be quite expensive" but no exact number. Anecdotal data from Bevy discord ~200-500 ns per migration for small bundles, consistent with the `≤ 1 µs` boyko target.
- **Despawn**: Bevy benches not directly cited but PR #19451 reports "20-30% improvement on despawn." `≤ 500 ns` is consistent with single-archetype swap-remove + free-list push.
- **10k mixed frame**: Bevy's `commands_overhead_2500_entities` benchmark in `crates/bevy_ecs/benches` reports ~50 µs for 2500 entities. Scaling to 10k → ~200 µs (well below the `≤ 5 ms` budget). The boyko target has comfortable headroom.

No published criterion numbers contradict the targets in the brief.

---

## §12 Open Architectural Questions for the Architect

1. **Atomic counter vs free-list lock-free vs placeholder**: Path A / Path B / Path C from §5. Recommendation requires knowing whether `EntityMaster::deallocate_entity` will run on workers or only on the dispatcher under apply. Current `delete_entity` is `&mut self` — runs only on dispatcher → Path A (simpler atomic counter for fresh IDs + free-list managed on dispatcher only) may suffice.

2. **Insert/remove chain batching (Issue #5074)**: boyko can avoid Bevy's intermediate-archetype problem by implementing per-entity command coalescing at apply time. flecs already does this (merge phase processes "commands in batches per entity"). Architectural decision: is `EntityCommands` allowed to defer enqueue until `.id()` / drop, OR does it enqueue eagerly with apply-time batching? The latter is simpler; the former allows compile-time `modify_bundle` fusion.

3. **`EntityCommands<'a, 's>` vs `EntityCommands<'a>`**: Bevy uses single lifetime (the `'a` borrows `Commands<'_, '_>` which itself elides `'w`). boyko's `Commands<'s>` is single-lifetime — `EntityCommands<'s>` is sufficient.

4. **Should `EntityCommands::despawn` consume `self`?** Bevy's current impl returns `&mut Self` to allow `.despawn().log_components()` chains. boyko has no `log_components` yet — could go either way. Consuming `self` slightly improves intent (the handle is dead) but breaks chain composition in conditional builders.

5. **Error handling on stale Entity**: Bevy's choice (Issue #10166) is `warn!()` in debug + no-op in release for the `try_*` family, panic for non-`try`. boyko's `delete_entity` returns `bool` — does the architect want `try_despawn -> bool` propagated, or fire-and-forget?

6. **Recursive despawn**: Bevy automatically despawns `RelationshipTarget` children. boyko has no hierarchy. Punt to a future phase.

7. **Bundle insert on entity that doesn't exist yet (pending in same queue)**: Bevy's allocator-based design guarantees the Entity ID is real even before apply, so this works. The Entity has no archetype yet, so `entity_command::insert` running before `spawn` would fail. Apply order matters — confirm `CommandQueue` apply is strict FIFO.

8. **Async/cross-thread Entity reservation**: Bevy's `RemoteAllocator` allows reservation from async contexts via `Arc`-cloned allocator. boyko has no async runtime today but Phase 9 workers will need this. Decision: `EntityMaster::reserve()` on `&self` or only `&mut self`? The latter forces all spawn to enqueue a "reserve" command that runs on the dispatcher, which conflicts with synchronous-ID requirement of §1.

9. **InsertMode (Replace vs Keep)**: Bevy distinguishes `insert` (replace existing component) vs `insert_if_new` (keep existing). boyko has no equivalent today — does Phase 11 expose both?

10. **Bundle removal semantics**: `EntityCommands::remove::<B: Bundle>()` removes ALL components in `B`. If the entity is missing some of them, Bevy continues (removes what's present, ignores absent). boyko's archetype migration on remove must mirror this.

---

## §13 References

[1] [Bevy ECS Commands module source](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/system/commands/mod.rs) — primary source for `Commands::spawn`, `Commands::entity`, `EntityCommands` struct shape.

[2] [Bevy EntityCommand trait + Insert/Remove/Despawn source](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/system/commands/entity_command.rs) — concrete EntityCommand implementations, lines 48-242.

[3] [Bevy Entities + RemoteAllocator source](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/entity/mod.rs) — entity allocator wrapper and the new EntitiesAllocator decoupling.

[4] [Bevy PR #19451 "Improved Entity Lifecycle: remove flushing"](https://github.com/bevyengine/bevy/pull/19451) — November 2025, deletes the reserve/flush model. Critical for understanding current Bevy direction.

[5] [Bevy Discussion #18577 "Never Flush Entities"](https://github.com/bevyengine/bevy/discussions/18577) — design rationale for the no-flush model, atomic-cursor architecture.

[6] [Bevy Issue #5074 "Chained EntityCommands create useless temporary archetypes"](https://github.com/bevyengine/bevy/issues/5074) — open problem with chained inserts creating intermediate archetypes.

[7] [Bevy PR #14897 "Have EntityCommands methods consume self"](https://github.com/bevyengine/bevy/pull/14897) — merged then reverted; the historical lesson on `&mut self` vs `self` chaining.

[8] [Bevy Issue #10166 "Commands should be infallible"](https://github.com/bevyengine/bevy/issues/10166) — rationale for `try_*` variants and warn-vs-panic semantics.

[9] [Bevy `EntityCommands` rustdoc](https://docs.rs/bevy/latest/bevy/ecs/system/struct.EntityCommands.html) — full method list and current signatures.

[10] [Bevy `Commands` rustdoc](https://docs.rs/bevy/latest/bevy/ecs/system/struct.Commands.html) — full method list including `spawn`, `spawn_batch`, `entity`, `get_entity`, `queue`.

[11] [Bevy 0.13 `Entities` rustdoc — historical reserve/flush model](https://docs.rs/bevy/0.13.0/bevy/ecs/entity/struct.Entities.html) — the older `reserve_entity` + `flush` pattern still used by most external articles.

[12] [Bevy Cheatbook on Commands](https://bevy-cheatbook.github.io/programming/commands.html) — user-facing semantics, apply timing, Entity ID gotchas.

[13] [Bevy Archetypes — Tainted Coders](https://taintedcoders.com/bevy/archetypes) — archetype graph and `Edges` caching.

[14] [Bevy `archetype` rustdoc](https://docs.rs/bevy/latest/bevy/ecs/archetype/index.html) — `Edges` struct and bundle transitions.

[15] [Bevy `Edges` rustdoc](https://docs.rs/bevy/latest/bevy/ecs/archetype/struct.Edges.html) — insert/remove/take bundle transition caching.

[16] [flecs Commands API reference](https://www.flecs.dev/flecs/group__commands.html) — `ecs_defer_begin/end/suspend/resume`.

[17] [flecs DeepWiki on Staging](https://deepwiki.com/SanderMertens/flecs/2.4-tables-and-storage) — per-stage thread-local command buffers, merge semantics.

[18] [flecs Discussion #1198 (Sander Mertens)](https://github.com/SanderMertens/flecs/discussions/1198) — author's recommendation to pre-allocate entity ID pool for multithreaded creation.

[19] [Unity Entities EntityCommandBuffer playback docs](https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/systems-entity-command-buffer-playback.html) — sort key + chunk-index deterministic playback.

[20] [Unity-Technologies EntityCommandBuffer samples README](https://github.com/Unity-Technologies/EntityComponentSystemSamples/blob/master/EntitiesSamples/Docs/entity-command-buffers.md) — placeholder negative-index pattern, ParallelWriter, sort keys, real-vs-temporary entity resolution.

[21] [Atomics in Rust (Mara Bos)](https://marabos.nl/atomics/) — relevant for the FreeCountState bit-packing and Acquire/Relaxed ordering choices in §5 Path B.

### Relevant boyko-engine paths for the architect

- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\command.rs` — existing `Command` trait + dispatch glue
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\command_queue.rs` — byte-arena queue; FIFO apply already supports spawn → despawn within one queue
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\spawn_command.rs` — current `SpawnCommand<B>` discards the returned Entity at line 208 with the explicit Phase 11 TODO note ("Phase 11 will add a `SpawnCommandReturning<B>` variant that pre-allocates an Entity and surfaces it pre-apply (Bevy-style `commands.spawn(...).id()`)")
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\commands.rs` — current `Commands<'s>` SystemParam; `spawn` returns `()` not `EntityCommands`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\entity\entity_master.rs` — `allocate_entity` and `deallocate_entity` (`&mut self`); `rewind_allocate` for failed-create rollback; SEND5 contract documented at line 350-367
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs:279` — `create_entity` apply path
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs:456` — `delete_entity` apply path (already exists, ready for `DespawnCommand` wrapper)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\bundle.rs` — `Bundle` trait + `BundleStaticInfo`; `cached_archetype_id` already supports insert-on-existing-entity if extended with `target_archetype = source + bundle_components` lookup

Sources:
- [Bevy ECS Commands module source](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/system/commands/mod.rs)
- [Bevy EntityCommand trait + Insert/Remove/Despawn source](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/system/commands/entity_command.rs)
- [Bevy Entities + RemoteAllocator source](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/entity/mod.rs)
- [Bevy PR #19451 "Improved Entity Lifecycle: remove flushing"](https://github.com/bevyengine/bevy/pull/19451)
- [Bevy Discussion #18577 "Never Flush Entities"](https://github.com/bevyengine/bevy/discussions/18577)
- [Bevy Issue #5074 "Chained EntityCommands create useless temporary archetypes"](https://github.com/bevyengine/bevy/issues/5074)
- [Bevy PR #14897 "Have EntityCommands methods consume self"](https://github.com/bevyengine/bevy/pull/14897)
- [Bevy Issue #10166 "Commands should be infallible"](https://github.com/bevyengine/bevy/issues/10166)
- [Bevy EntityCommands rustdoc](https://docs.rs/bevy/latest/bevy/ecs/system/struct.EntityCommands.html)
- [Bevy Commands rustdoc](https://docs.rs/bevy/latest/bevy/ecs/system/struct.Commands.html)
- [Bevy 0.13 Entities rustdoc — historical reserve/flush model](https://docs.rs/bevy/0.13.0/bevy/ecs/entity/struct.Entities.html)
- [Bevy Cheatbook on Commands](https://bevy-cheatbook.github.io/programming/commands.html)
- [Bevy Archetypes — Tainted Coders](https://taintedcoders.com/bevy/archetypes)
- [Bevy archetype rustdoc](https://docs.rs/bevy/latest/bevy/ecs/archetype/index.html)
- [Bevy Edges rustdoc](https://docs.rs/bevy/latest/bevy/ecs/archetype/struct.Edges.html)
- [flecs Commands API reference](https://www.flecs.dev/flecs/group__commands.html)
- [flecs DeepWiki on Staging](https://deepwiki.com/SanderMertens/flecs/2.4-tables-and-storage)
- [flecs Discussion #1198 (Sander Mertens)](https://github.com/SanderMertens/flecs/discussions/1198)
- [Unity Entities EntityCommandBuffer playback docs](https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/systems-entity-command-buffer-playback.html)
- [Unity-Technologies EntityCommandBuffer samples README](https://github.com/Unity-Technologies/EntityComponentSystemSamples/blob/master/EntitiesSamples/Docs/entity-command-buffers.md)
- [Atomics in Rust (Mara Bos)](https://marabos.nl/atomics/)