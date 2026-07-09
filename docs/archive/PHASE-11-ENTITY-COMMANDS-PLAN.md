> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 11 — EntityCommands Chaining + Despawn — Architectural Plan (Round 3)

## §0 Round 3 Changelog

This round resolves the two criticals (`C-N1`, `C-N2`) and three warnings (`W-N1`, `W-N2`, `W-N3`) raised in `D:\claude\BoykoEngine\docs\PHASE-11-CRITIC-ROUND-2.md`. Optional `O-N1` polished.

| Finding | Fix location | One-liner |
|---|---|---|
| **C-N1** `entity_master()` aliasing rule not type-enforced | §5.5 (new `EntityCounter<'s>` newtype), §5.6 (Commands carries `EntityCounter`, not raw `*const EntityMaster`), §11.7 (layout), §12.2, §12.7 (`UnsafeEcsCell::entity_counter`), §13.1, §13.3 (Miri test rename), §14 Wave B (Step 5 retitled), §15.2 | Added `EntityCounter<'s>` newtype that exposes ONLY `reserve_entity(&self)` over a `*const AtomicUsize`. `Commands<'s>` now holds `EntityCounter<'s>` (not `*const EntityMaster`); no path from `Commands` reaches any other `EntityMaster` field. Aliasing rule type-enforced. New invariant **EM6**. |
| **C-N2** §7.2 / §7.4 / §12.6 use fictitious flat-byte API | §7.2 (rewritten `swap_remove_index_no_drop` against `units[idx].ptr()` model), §7.4 (retained-bytes via `units[row].ptr()`), §7.5 (cost numbers adjusted), §12.6 (API surface updated), §13.1 (test names), §14 Wave E (Step 11 updated) | Option (b) from critic Q2: keep existing chunked + Unit-pointer storage. `swap_remove_index_no_drop` becomes a `copy_nonoverlapping(units[last].ptr(), units[idx].ptr(), layout.size())` plus `UnsafeCell<Tick>` row swap via `added_ticks[idx].get()`. Retained-bytes extraction via `from_raw_parts(units[row].ptr(), layout.size())`. No ComponentPool storage rework. |
| **W-N1** `apply_replace_in_place` superset proof | §7.4 (SAFETY block cites canonicalization invariant + adds debug-only defensive check) | Cited `ArchetypeRegistry::find_exact_match(ComponentMask)` canonicalization (`archetype_master.rs:99-133, 462-473`): same mask ⇒ same `ArchetypeId`. Therefore `merged_id == source_id ⇒ bundle ⊆ source`. Plus added a `debug_assert!(archetype.component_pools().has_pool(component_id))` defensive check in the inner loop. |
| **W-N2** `swap_remove_index_no_drop` contract tightened | §7.2 contract paragraph | Contract tightened to "MUST NOT invoke `drop_fn` on any slot (source or last) — caller owns byte ownership tracking." Removed the leakier "no drop on source-row bytes" phrasing. |
| **W-N3** `entity_counter` minted per system invocation | §5.6 (explicit statement about per-invocation `get_param`), §8.7 (new sub-section), §13.3 (Miri test for cross-frame staleness) | Stated explicitly that Phase 8c `IntoSystem` calls `SystemParam::get_param` **per system invocation each frame**. `Commands<'s>::Item<'w, 's>` is dropped at the end of each system body; the `EntityCounter`'s pointer never outlives `'w`. Cross-referenced Phase 8c contract. |
| **O-N1** §10.5 contention reference polish | §10.5 final paragraph | Polished wording to reference `bench_reserve_entity_parallel_8_threads` more tightly. |

Sections expanded vs Round 2:
- §2.3 — added **EM6** (EntityCounter field-restriction invariant).
- §5.5 — new `EntityCounter<'s>` newtype with safety contract.
- §5.6 — Commands carries `EntityCounter<'s>` instead of raw pointer; per-invocation `get_param` lifecycle stated.
- §7.2 — `swap_remove_index_no_drop` rewritten against `units[idx].ptr()` + `UnsafeCell<Tick>` model; W-N2 contract tightening.
- §7.4 — retained-bytes via Unit pointer; SAFETY block cites canonicalization invariant; defensive debug check added.
- §7.5 — cost numbers refined (Unit-pointer dereference + 1 cache miss in worst case).
- §8.7 — new sub-section "Per-invocation lifecycle of Commands" (W-N3).
- §10.5 — final paragraph polished (O-N1).
- §11.7 — Commands layout updated (8 + EntityCounter newtype = 16 B unchanged).
- §12 — API surface updated for EntityCounter + per-invocation get_param.
- §13.1, §13.3 — test names updated for EntityCounter aliasing.
- §14 Wave B — Step 5 retitled to "EntityCounter newtype + Commands wiring".
- §15.2 — internal API delta updated.

Open Round-2 questions resolved:
- **architect Q1** (W-N1): `get_or_create_archetype` canonicalization invariant **does exist** — confirmed by reading `archetype_master.rs:99-133, 462-473`. `find_exact_match(ComponentMask::from_components(...))` is exact-mask matching; same component set ⇒ same `ArchetypeId`. Cited in §7.4.
- **architect Q2** (C-N2): **Option (b)** — rewrite §7.2/§7.4/§12.6 against existing chunked + Unit-pointer storage. No ComponentPool rework. Cost analysis adjusted in §7.5.
- **architect Q3** (C-N1): **Yes** — `EntityCounter` newtype introduced. Type-enforces the aliasing rule.

Total Round 3: ~2310 lines (Round 2: 2194; growth from §5.5 newtype definition + §7.2 rewrite + §7.4 SAFETY block + §8.7 + §11.7 layout + §12.2/12.7 API delta + §13 test rename + §14 Step 5 retitle).

---

## §1 Summary, Target Metrics, Scope

### 1.1 Goal

Deliver Bevy-style chainable `commands.spawn(bundle).insert(extra).insert(more).id()` ergonomics on top of the Phase 8d `Commands<'s>` queue, with synchronous Entity ID return from the deferred-spawn path. Build `despawn` + `entity()` handle access + archetype-migrating `insert`/`remove` over the existing `Bundle` infrastructure.

### 1.2 Target metrics

| Path | Target | Justification |
|------|--------|---------------|
| `Commands::spawn(bundle)` enqueue + Entity return | ≤ 25 ns | 1 atomic fetch_add (5 ns) + 1 push (18 ns, Phase 8d D1) + EntityCommands construction (free) |
| `EntityCommands::insert(extra)` enqueue | ≤ 22 ns | 1 push (18 ns) + Entity carried in struct |
| `EntityCommands::remove::<C>()` enqueue | ≤ 22 ns | 1 push (18 ns) |
| `EntityCommands::despawn()` enqueue | ≤ 22 ns | 1 push (18 ns) |
| `EntityCommands::id()` | 0 ns (inlined struct field read) | Copy Entity field |
| `Commands::entity(id)` | ≤ 5 ns | Wrap existing Entity in handle |
| `SpawnAtCommand<B>::apply` | ≤ 500 ns warm / ≤ 1.2 µs cold | Phase 8.5 SpawnCommand baseline |
| `InsertCommand<B>::apply` (single migration) | ≤ 720 ns | Archetype migration via fresh `get_or_create_archetype` lookup + Unit-pointer-driven memcpy (Round 3: +20 ns vs Round 2 for the Unit dereference; see §7.5) |
| `RemoveCommand<C>::apply` (single migration) | ≤ 720 ns | Symmetric to insert |
| `DespawnCommand::apply` | ≤ 500 ns | `EcsMaster::delete_entity` baseline |
| `EntityCounter::reserve_entity()` | ≤ 10 ns single-thread; ≤ 60 ns under N=8 contention (§10.5) | `fetch_add(1, Relaxed)` + cache-line bounce model |
| Migration cache miss → first lookup of `(source, bundle)` | ≤ 200 ns | mask compute + archetype lookup, lock-free read |
| Full 10k mixed frame (spawn + insert + despawn) | ≤ 5.5 ms | Phase 8.5 already does 10k spawn in ~308 µs; +migration adds ~5 µs per insert (Round 3 unit-pointer factor + W5 contention) |

Cache budgets: hot `EntityCommands<'a, 's>` ≤ 16 B (one cache line); `SpawnAtCommand` / `InsertCommand` / `RemoveCommand` payloads ≤ 64 B (one cache line) per queue slot.

### 1.3 In-scope

- A. Pre-allocated Entity ID via atomic counter on `EntityMaster` (Path A).
- B. `EntityCommands<'a, 's>` struct + chainable `&mut self` methods (two-lifetime per C1).
- C. `Commands::spawn<B>` return type change `()` → `EntityCommands<'_, '_>`.
- D. `Commands::entity(Entity) -> EntityCommands<'_, '_>` access for existing entities.
- E. `Commands::despawn(Entity)` convenience wrapper.
- F. `EntityCommands::{id, insert, remove, despawn, reborrow, try_insert, try_remove, try_despawn}`.
- G. New `SpawnAtCommand<B>`, `InsertCommand<B>`, `RemoveCommand<C>`, `DespawnCommand` types.
- H. Archetype migration (insert/remove) via fresh `get_or_create_archetype` lookups using `migration_scratch` (no `Edges` graph cache in v1).
- I. Stale-entity handling: debug_assert + silent no-op for non-`try_*`; `Result` for `try_*`.
- J. Phase 10 integration: insert bumps `added_tick` + `changed_tick` to `current_tick`; retained components preserve original ticks via `create_entity_with_ticks`.
- K. Phase 9 parallel safety: workers call `EntityCounter::reserve_entity(&self)` lock-free; free-list pops only on dispatcher.

### 1.4 Out of scope (explicit non-goals)

- **Archetype edge graph cache (Bevy `Edges`)** — Phase 12.
- **Intermediate archetype coalescing (Issue #5074)** — Phase 13.
- **`insert_if_new` mode** — Phase 12.
- **Recursive despawn / hierarchy** — N/A.
- **Spawn batch (`spawn_batch`)** — Phase 12.
- **Cross-world entity migration** — N/A.
- **`EntityCommands::clear`** — Phase 12.
- **Per-component `is_new` precision for `Added<T>` on replace** — Phase 12 (OQ5).
- **ComponentPool storage rework (flat-bytes flatten of chunked storage)** — out of scope; Round 3 keeps the existing chunked + Unit-pointer model (architect Q2 option b).

---

## §2 Invariants (EC1..EC15, EM1..EM6)

### 2.1 EC* (new this phase)

| ID | Statement |
|---|---|
| **EC1** | `EntityCommands<'a, 's>` is `!Send + !Sync` for any `'a`/`'s`. Carries `&'a mut Commands<'s>` which is `!Sync` via CQ-SEND2. Workers cannot share. |
| **EC2** | `EntityCommands::id() -> Entity` is infallible. Returns the Entity captured at construction. Always real — either freshly reserved via `EntityCounter::reserve_entity`, or user-supplied. |
| **EC3** | `EntityCommands::{insert, remove, despawn, try_insert, try_remove, try_despawn}` take `&mut self` and return `&mut Self`. Non-terminal. `despawn` does NOT consume `self` (Bevy PR #15523 revert lesson). |
| **EC4** | `EntityCommands::reborrow(&mut self) -> EntityCommands<'_, 's>` produces a shorter-lived clone borrowing `self.commands` mutably for the reborrow scope. The state lifetime `'s` is preserved. |
| **EC5** | `EntityCommands` operations enqueue lazily — no archetype mutation, no `EntityMaster` mutation (except the upstream `reserve_entity` atomic which is conflict-free per EM5/EM6), no resource access at the call site. Effects materialise at `CommandQueue::apply`. |
| **EC6** | Apply order within one queue is strict FIFO (Phase 8d C2'). |
| **EC7** | Pre-allocated Entity never written into an archetype + then `DespawnCommand`-ed: debug_assert warning + silent no-op in release. Reserved ID is leaked until Phase 12 reaper. |
| **EC8** | Stale Entity passed to `Commands::entity(e)` followed by `.insert(...)`: debug_asserts in `InsertCommand::apply` if `EntityMaster::is_entity_valid(e)` is false; silent no-op in release. |
| **EC9** | `EntityCommands::insert<B: Bundle>(bundle)`: target archetype = `source.signature ∪ B::component_ids()`. If a component of `B` already exists, **replace** + bump `changed_tick` (Phase 10 STORE3). `added_tick` overwritten too (OQ5 — Phase 11 limitation, ship documented). |
| **EC10** | `EntityCommands::remove<C: Component>()`: single-component remove. Bundle-typed remove deferred to Phase 12. |
| **EC11** | `EntityCommands::despawn()`: enqueues `DespawnCommand { entity }`. Apply calls `EcsMaster::delete_entity`. False return debug_asserts; silent no-op in release. |
| **EC12** | `try_*` variants v1: functionally identical to non-`try_*`. Names reserved for Phase 12 output-slot machinery. |
| **EC13** | The Entity returned by `EntityCommands::id()` is **invalid for query lookups until the spawn applies**. `world.get_component<T>(entity)` returns `None` between `spawn` and `apply`. |
| **EC14** | Two distinct `EntityCommands` instances from the same `Commands` cannot live concurrently — borrow checker rejects the double-borrow unless `reborrow` is used. |
| **EC15** | `EntityCommands<'a, 's>` carries only `(Entity, &'a mut Commands<'s>)` — 16 B handle. Per O1: `mem::size_of` static-asserted. |

### 2.2 Reused invariants

- **CQ1, CQ2, CQ-PACK1, CQ4, CQ5, CQ7, CQ-SEND1, CQ-SEND2** (Phase 8d).
- **B1..B4, SBC1..SBC10** (Phase 8.5).
- **SEND1, SEND5, SEND6, SCH7, ALLOC1..6** (Phase 9).
- **CD1..CD5** (Phase 10).
- **U1, U2, U11, U14** (Phase 7).

### 2.3 New Phase 11 EntityMaster invariants (EM1..EM6)

| ID | Statement |
|---|---|
| **EM1** | `EntityMaster::next_entity_id` becomes `AtomicUsize`. `EntityCounter::reserve_entity(&self) -> Entity` performs `fetch_add(1, Ordering::Relaxed)` on the same atomic via raw pointer. |
| **EM2** | `EntityCounter::reserve_entity` returns ONLY fresh IDs — never pops from `free_entity_ids`. Free-list popping remains exclusive to `EntityMaster::allocate_entity(&mut self)` (dispatcher only, `pub(crate)` per W2). |
| **EM3** | `entities_inland` Vec capacity sized to `MAX_ENTITIES_HINT = 64,000` at `EcsMaster::new`. `EntityCounter::reserve_entity` does NOT touch `entities_inland`. |
| **EM4** | The fresh-vs-recycled invariant: any ID returned by `EntityCounter::reserve_entity(&self)` is **strictly larger** than the max ID currently in `free_entity_ids` (proof: §4.6). |
| **EM5** | `EntityCounter::reserve_entity(&self)` may run from any thread holding an `EntityCounter`. Pre-sized Vec means dispatcher's `register_entity_with_ptr` does NOT realloc for `reserved_id < 64,000`. Workers never resize. |
| **EM6** (**NEW, C-N1**) | **EntityCounter field-restriction invariant**: code reachable from `Commands<'s>` (i.e. systems running on workers) MUST NOT obtain a shared `&EntityMaster` reference. The only window into `EntityMaster` from worker code is `EntityCounter<'s>`, which encapsulates a `*const AtomicUsize` aimed at `EntityMaster::next_entity_id`. This makes the aliasing rule type-enforced: workers cannot read `entities_inland.len()`, `free_entity_ids`, `sparse_to_active`, or `active_ids` via `Commands` — those fields are only reachable through `&mut EcsMaster` on the dispatcher. |

---

## §3 Decision Matrix Q1..Q11

| Q | Decision | Rationale | Trade-off |
|---|---|---|---|
| **Q1** Entity ID mechanism | Path A | Lowest complexity; ABA impossible | Free list drained dispatcher-only |
| **Q2** EntityCommands lifetime | Two lifetimes `<'a, 's>` where `'s: 'a` (C1) | Single `'a` collapses Commands<'s> generic and makes EntityCommands invariant in `'a`; reborrow fails. Two lifetimes match Bevy's shape | One extra lifetime parameter in signatures |
| **Q3** Despawn signature | `&mut self → &mut Self` | Bevy PR #15523 revert lesson | `despawn` not "terminal" by signature; documented |
| **Q4** Stale-entity policy | debug_assert + silent no-op for non-`try_*`; `try_*` functionally identical in v1 | Bevy Issue #10166 | `try_*` names exist but cannot yet report success |
| **Q5** Intermediate archetype | Punt; document limitation | Issue #5074 open 7 years | Chained `.insert(B).insert(C)` produces 2 migrations |
| **Q6** Insert overlap | Replace (overwrite bytes); `try_insert` reserved for "only if new" semantic in Phase 12 | Phase 8.5 SpawnCommand replace parity | `try_insert` is currently a no-op alias |
| **Q7** Bundle type for insert | Same Bundle trait as Phase 8.5 | Reuse OnceLock infra | Same bundle resolves to different archetypes per source |
| **Q8** Despawn-after-reserve race | Generation bump on free | Pre-existing mechanism | None |
| **Q9** SpawnCommand retire | Hard replace → SpawnAtCommand(entity, bundle) | `SpawnCommand` is `pub(crate)` | Zero-cost rename |
| **Q10** Phase 8d test migration | Return type change `() → EntityCommands<'_, '_>` is callsite-compatible (no-op Drop) | Existing tests verified via grep | None |
| **Q11** (**NEW, C-N1**) | EntityCounter newtype carrying ONLY `*const AtomicUsize` (no `*const EntityMaster`) | Type-enforces EM6 aliasing rule; future `Commands::reserve_n` cannot accidentally read `entities_inland.len()` because the pointer's type is `*const AtomicUsize`, not `*const EntityMaster` | Slightly more boilerplate in `UnsafeEcsCell::entity_counter` — projects to the atomic field rather than the whole master |

---

## §4 Pre-Allocated Entity ID Mechanism (Section A)

### 4.1 EntityMaster shape change

Current (Phase 7):
```rust
pub struct EntityMaster {
    free_entity_ids: Vec<EntityId>,
    next_entity_id: EntityId,
    pub(crate) entities_inland: Vec<EntityInland>,
    pub(crate) sparse_to_active: Vec<u32>,
    pub(crate) active_ids: Vec<EntityId>,
}
```

Phase 11:
```rust
pub struct EntityMaster {
    // Phase 11: dispatcher-only LIFO recycling. Workers do NOT pop (EM2).
    free_entity_ids: Vec<EntityId>,

    // Phase 11: atomic counter. Workers call via EntityCounter<'s>::reserve_entity
    // which holds a *const AtomicUsize pointer aimed at THIS field
    // (EM6: workers cannot reach any other field via this channel).
    // Dispatcher allocate_entity also uses fetch_add for fresh-path.
    next_entity_id: AtomicUsize,

    pub(crate) entities_inland: Vec<EntityInland>,
    pub(crate) sparse_to_active: Vec<u32>,
    pub(crate) active_ids: Vec<EntityId>,
}
```

Size impact: `AtomicUsize` is 8 B (same as `EntityId`), align 8. Net change: 0 bytes. Layout-stable.

### 4.2 New API: `EntityCounter<'s>::reserve_entity`; privacy of `allocate_entity` (W2)

`EntityMaster` no longer exposes a `reserve_entity(&self)` method — that responsibility migrates to the new `EntityCounter<'s>` newtype (§5.5). The dispatcher path remains:

```rust
impl EntityMaster {
    /// W2: restricted to `pub(crate)` in Phase 11. The previously-public
    /// `allocate_entity` is now dispatcher-only — public callers must go
    /// through `EcsMaster::create_entity` which is the canonical entry.
    /// This eliminates the W2 concern that mixed legacy + new paths could
    /// observe different generation semantics (recycled vs fresh = 0).
    #[inline]
    pub(crate) fn allocate_entity(&mut self) -> Entity {
        if let Some(id) = self.free_entity_ids.pop() {
            debug_assert!(id.0 < self.entities_inland.len(), "Free entity ID out of bounds");
            let current_gen = self.entities_inland[id.0].generation();
            Entity::new(id, current_gen)
        } else {
            let id = self.next_entity_id.fetch_add(1, Ordering::Relaxed);
            if id >= self.entities_inland.len() {
                self.entities_inland.resize(id + 1, EntityInland::NULL);
            }
            if id >= self.sparse_to_active.len() {
                self.sparse_to_active.resize(id + 1, u32::MAX);
            }
            Entity::new(EntityId(id), 0)
        }
    }

    /// Crate-internal accessor for EntityCounter construction inside UnsafeEcsCell.
    /// Not exposed to user code.
    #[inline]
    pub(crate) fn next_id_atomic(&self) -> &AtomicUsize {
        &self.next_entity_id
    }
}
```

### 4.3 Mutation discipline

`allocate_entity(&mut self)` is the only path that grows `entities_inland`. Workers never grow. `EntityCounter::reserve_entity` only touches the atomic counter — no Vec field reachable from the EntityCounter's pointer type (EM6).

### 4.4 Pre-sizing contract

`EcsMaster::new` pre-allocates `MAX_ENTITIES_HINT = 64,000`. Workers reading `entities_inland` during query iteration won't race the dispatcher's resize because the resize only happens during apply window when no workers run (SCH7).

### 4.5 Reservation leak scenario

Worker panic between `spawn` enqueue and apply: `SpawnAtCommand` is in queue; apply runs normally and registers the entity. **No leak.**

Queue dropped without apply (system cancelled): bundle Drop runs via `consume_and_drop_glue`; reserved Entity ID leaks. Counter marches forward monotonically. Worst case: 1 ID per panic. 2^64 ID space accommodates trillions of leaks. Phase 12 reaper if telemetry shows pressure (OQ4).

### 4.6 Four-interleaving proof (C4)

Unchanged from Round 2. The proof works against `EntityCounter::reserve_entity` exactly as it did against the inline method — both paths perform `fetch_add(Relaxed)` on the same `AtomicUsize`. The atomic counter's monotonicity + `is_entity_valid`'s generation+null check together close all collision windows.

### 4.7 Memory ordering analysis

| Operation | Order | Justification |
|---|---|---|
| `EntityCounter::reserve_entity::fetch_add(1, Relaxed)` | Relaxed | uniqueness only; happens-before for the returned id is established later by the apply-window barrier (every worker write is visible to dispatcher via SCH7's join) |
| `allocate_entity::fetch_add(1, Relaxed)` (fresh path) | Relaxed | dispatcher holds `&mut self`; relaxed is sufficient |
| `next_entity_id.load(Relaxed)` (observational getter) | Relaxed | no synchronization needed |

No `AcqRel` needed: the apply-window barrier provides cross-thread happens-before via `ArrayQueue` Release/Acquire on completion_queue.

---

## §5 EntityCommands<'a, 's> Shape + Chain Methods (Sections B, F)

### 5.1 Struct layout (C1 — two lifetimes)

```rust
/// Phase 11: chainable handle for issuing per-entity deferred commands.
///
/// # Lifetimes (C1)
///
/// - `'a` — the borrow scope of `&'a mut Commands<'s>`. Shorter or equal to `'s`.
/// - `'s` — the system's state scope (lifetime of the underlying `CommandQueue`).
///
/// The bound `'s: 'a` is implicit via `&'a mut Commands<'s>`.
///
/// # Layout (EC15)
///
/// 16 B total:
/// - `entity: Entity` (8 B: 4 B id + 4 B generation)
/// - `commands: &'a mut Commands<'s>` (8 B: pointer)
///
/// # Send / Sync (EC1)
///
/// `!Send + !Sync` — `Commands<'s>` is `!Sync` (CQ-SEND2 + EntityCounter `!Sync`).
pub struct EntityCommands<'a, 's> {
    pub(crate) entity: Entity,
    pub(crate) commands: &'a mut Commands<'s>,
}
```

Field order:
- `entity` first (8 B) — accessed by `id()` and embedded in every command payload.
- `commands` second (8 B) — accessed only by the chain methods.

No padding. `#[repr(C)]` for clarity (no unsafe layout dependency).

### 5.2 Construction

```rust
impl<'s> Commands<'s> {
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> EntityCommands<'_, 's>;
    pub fn entity(&mut self, entity: Entity) -> EntityCommands<'_, 's>;
}
```

### 5.3 Methods (with O4 TODO inline comments)

```rust
impl<'a, 's> EntityCommands<'a, 's> {
    /// Returns the entity targeted by this handle. EC2.
    #[inline]
    pub fn id(&self) -> Entity {
        self.entity
    }

    /// Enqueues an `InsertCommand<B>` for this entity. Chainable.
    #[inline]
    pub fn insert<B: Bundle>(&mut self, bundle: B) -> &mut Self {
        self.commands.queue.push(InsertCommand {
            entity: self.entity,
            bundle,
        });
        self
    }

    /// Enqueues a `RemoveCommand<C>` for a single component. Chainable.
    #[inline]
    pub fn remove<C: Component>(&mut self) -> &mut Self {
        self.commands.queue.push(RemoveCommand::<C> {
            entity: self.entity,
            _marker: PhantomData,
        });
        self
    }

    /// Enqueues a `DespawnCommand`. Chainable (returns `&mut Self`). Q3.
    #[inline]
    pub fn despawn(&mut self) -> &mut Self {
        self.commands.queue.push(DespawnCommand { entity: self.entity });
        self
    }

    #[inline]
    pub fn try_insert<B: Bundle>(&mut self, bundle: B) -> &mut Self {
        // TODO Phase 12: wire output-slot success indicator — do NOT alias non-try_* form.
        self.insert(bundle)
    }
    #[inline]
    pub fn try_remove<C: Component>(&mut self) -> &mut Self {
        // TODO Phase 12: wire output-slot success indicator — do NOT alias non-try_* form.
        self.remove::<C>()
    }
    #[inline]
    pub fn try_despawn(&mut self) -> &mut Self {
        // TODO Phase 12: wire output-slot success indicator — do NOT alias non-try_* form.
        self.despawn()
    }

    /// Reborrow with shorter lifetime. EC4.
    #[inline]
    pub fn reborrow(&mut self) -> EntityCommands<'_, 's> {
        EntityCommands {
            entity: self.entity,
            commands: &mut *self.commands,
        }
    }
}
```

### 5.4 `Commands::entity` and `Commands::despawn` (Sections D, E)

```rust
impl<'s> Commands<'s> {
    #[inline]
    pub fn entity(&mut self, entity: Entity) -> EntityCommands<'_, 's> {
        EntityCommands { entity, commands: self }
    }

    #[inline]
    pub fn despawn(&mut self, entity: Entity) {
        self.queue.push(DespawnCommand { entity });
    }
}
```

### 5.5 NEW: `EntityCounter<'s>` newtype (C-N1)

C-N1 critic note: returning a full `&EntityMaster` exposes more than necessary; future maintainers could add a method on `Commands` that reads other EntityMaster fields and the borrow checker would not catch it. Solution: an opaque newtype carrying a raw pointer to the atomic counter — and only that.

```rust
use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::ecs::core::entity::entity::{Entity, EntityId};

/// Phase 11 (C-N1, EM6): minimal projection of `EntityMaster` exposing only
/// the atomic counter for thread-safe Entity reservation from workers.
///
/// Used by [`Commands::spawn`] / [`Commands::entity`] to mint Entity IDs
/// from system bodies without exposing the full `&EntityMaster`. This
/// makes the EM6 aliasing rule type-enforced: a worker holding an
/// `EntityCounter` cannot reach `entities_inland`, `free_entity_ids`,
/// `sparse_to_active`, or `active_ids` — the only field reachable through
/// the carried pointer is `EntityMaster::next_entity_id`.
///
/// # Lifetime
///
/// `'s` is the system's state scope. The pointer is minted from
/// `UnsafeEcsCell<'w>` per system invocation and re-tagged to `'s` by
/// `PhantomData` (the SystemParam contract guarantees `'w >= 's`).
///
/// # Layout
///
/// 8 B: one `*const AtomicUsize` pointer. `PhantomData` is ZST.
///
/// # Send / Sync
///
/// `Send + Sync` via explicit impls — the carried pointer aims at a
/// single `AtomicUsize` whose access through this newtype is bounded to
/// `reserve_entity`, which is purely atomic RMW. No data race possible.
#[derive(Clone, Copy)]
pub struct EntityCounter<'s> {
    /// Raw pointer to `EntityMaster::next_entity_id`.
    /// Minted from `UnsafeEcsCell<'w>` in `Commands::get_param`.
    next_id_ptr: *const AtomicUsize,

    /// Variance marker: ties the pointer's apparent validity to `'s`.
    _marker: PhantomData<&'s AtomicUsize>,
}

// SAFETY: `EntityCounter` carries a `*const AtomicUsize`. The only
// dereference path is `reserve_entity` which performs an atomic RMW.
// `AtomicUsize::fetch_add(Relaxed)` is data-race-free from any thread.
unsafe impl<'s> Send for EntityCounter<'s> {}
unsafe impl<'s> Sync for EntityCounter<'s> {}

impl<'s> EntityCounter<'s> {
    /// Constructs an `EntityCounter` from a raw pointer to the atomic counter.
    ///
    /// # Safety
    ///
    /// * `ptr` must be a valid `*const AtomicUsize` for the entirety of `'s`.
    /// * The pointed-to atomic is the `EntityMaster::next_entity_id` of an
    ///   `EcsMaster` whose lifetime contains `'s`.
    /// * Caller asserts the EM6 invariant: no other code path reads
    ///   `EntityMaster` fields via this pointer (the type system makes this
    ///   impossible by construction — the pointer's type is `AtomicUsize`,
    ///   not `EntityMaster`).
    #[inline]
    pub(crate) unsafe fn from_ptr(ptr: *const AtomicUsize) -> Self {
        Self { next_id_ptr: ptr, _marker: PhantomData }
    }

    /// Atomically reserves a fresh Entity ID. Lock-free.
    ///
    /// Cost: ~10 ns single-thread, up to 60 ns under N=8 contention (§10.5).
    ///
    /// Returns an `Entity` with generation 0 (fresh path; EM1).
    ///
    /// # Safety contract (encapsulated)
    ///
    /// This method is safe to call because:
    /// * The pointer is valid for `'s` by construction (`from_ptr` contract).
    /// * `AtomicUsize::fetch_add(Relaxed)` is data-race-free.
    /// * Generation correctness follows from EM1..EM6 + §4.6 four-case proof.
    #[inline]
    pub fn reserve_entity(&self) -> Entity {
        // SAFETY (EM5, EM6, U_C2):
        //   - `next_id_ptr` was minted by `UnsafeEcsCell::entity_counter` (§12.7)
        //     from a live EntityMaster's `next_id_atomic()` projection.
        //   - The pointer's apparent lifetime `'s` is bounded by `'w >= 's`
        //     per the SystemParam contract (Phase 8c IntoSystem, §8.7).
        //   - Atomic RMW from any thread is data-race-free.
        let id = unsafe { (*self.next_id_ptr).fetch_add(1, Ordering::Relaxed) };
        debug_assert!(id < usize::MAX / 2, "EntityId counter near exhaustion");
        Entity::new(EntityId(id), 0)
    }
}
```

The boundary is hard: an `EntityCounter` value cannot be coerced into `&EntityMaster`, cannot project to any other EntityMaster field, and the only public method is `reserve_entity`. A future maintainer adding a method to `Commands` that wants to read, say, `entities_inland.len()` would need to add a brand-new field to `Commands<'s>` (which would visibly break the §11.7 layout assertion), not piggyback on the existing pointer.

### 5.6 `Commands<'s>` struct + `get_param` body (C2 + W-N3)

```rust
/// Phase 11: deferred world-mutation buffer.
///
/// # Lifetimes
///
/// - `'s` — the system's state scope. The underlying `CommandQueue` lives
///   in the system's cached state slot. The `EntityCounter<'s>` is
///   re-minted per system invocation by `SystemParam::get_param` (§8.7).
///
/// # Layout (§11.7)
///
/// 16 B total: `(&'s mut CommandQueue, EntityCounter<'s>)`.
pub struct Commands<'s> {
    pub(crate) queue: &'s mut CommandQueue,
    /// Phase 11 (C-N1, EM6): EntityCounter newtype — exposes ONLY the
    /// atomic counter, never the full `&EntityMaster`. See §5.5.
    pub(crate) entity_counter: EntityCounter<'s>,
}

// SAFETY (SP1, SP2, SP4 — augmented):
//   - SP1: `Commands::init_access` declares NO reads/writes.
//     `EntityCounter::reserve_entity` is conflict-free by EM6: the only
//     EntityMaster state touched is the atomic counter; atomic RMW from
//     `&self` is safe concurrent with any other reader/writer.
//   - SP2: per Phase 8c IntoSystem, `get_param` runs PER SYSTEM INVOCATION
//     each frame (§8.7). The EntityCounter pointer is re-minted fresh
//     every call; `Commands<'s>::Item<'w, 's>` is dropped at the end of
//     the system body, so the pointer never outlives `'w`.
//   - SP4: `init_state` constructs a fresh `CommandQueue` — no world
//     mutation, no archetype / resource registry change.
unsafe impl SystemParam for Commands<'_> {
    type State = CommandQueue;
    type Item<'w, 's> = Commands<'s>;

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        CommandQueue::new()
    }

    #[inline]
    fn init_access(
        _state: &Self::State,
        _system_meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // SP1: Commands declares NO component / resource access.
        // EntityCounter access is conflict-free per EM6 + EVT1 precedent.
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (SP2, EM5, EM6, U_C2, W-N3):
        //   - `state: &'s mut CommandQueue` is exclusive per APP1'.
        //   - `world.entity_counter()` mints a fresh `EntityCounter<'_>`
        //     whose internal pointer is valid for `'w`. We re-tag the
        //     lifetime to `'s` via PhantomData; sound because `'w >= 's`
        //     (Phase 8c IntoSystem contract — §8.7: get_param runs once
        //     per system invocation; `'s` never outlives `'w`).
        //   - The EntityCounter's contract (§5.5) restricts reachable
        //     state to the atomic counter only — EM6 is type-enforced.
        let counter = unsafe { world.entity_counter::<'s>() };
        Commands {
            queue: state,
            entity_counter: counter,
        }
    }

    #[inline]
    fn apply(state: &mut Self::State, _system_meta: &SystemMeta, world: &mut EcsMaster) {
        state.apply(world);
    }
}

impl<'s> Commands<'s> {
    /// Phase 11: returns `EntityCommands<'_, 's>` instead of `()`.
    #[inline]
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> EntityCommands<'_, 's> {
        let entity = self.entity_counter.reserve_entity();
        self.queue.push(SpawnAtCommand { entity, bundle });
        EntityCommands { entity, commands: self }
    }
}
```

Notice that `Commands::spawn` no longer needs a SAFETY block beyond the existing `unsafe impl SystemParam`: `EntityCounter::reserve_entity(&self)` is a safe method, with all the unsafety encapsulated inside the newtype's constructor (`from_ptr`).

### 5.7 Compilable 10-line sketch (C1 follow-up)

Unchanged from Round 2; the developer Step 6 verifies this exact shape with the real `Commands` + `Entity` types.

```rust
struct CommandQueue;
impl CommandQueue { fn push<T>(&mut self, _: T) {} }

#[derive(Clone, Copy)]
struct EntityCounter<'s> { _marker: std::marker::PhantomData<&'s u64> }
impl<'s> EntityCounter<'s> { fn reserve(&self) -> u64 { 42 } }

pub struct Commands<'s> {
    queue: &'s mut CommandQueue,
    counter: EntityCounter<'s>,
}

pub struct EntityCommands<'a, 's> {
    entity: u64,
    commands: &'a mut Commands<'s>,
}

impl<'s> Commands<'s> {
    pub fn spawn(&mut self) -> EntityCommands<'_, 's> {
        let e = self.counter.reserve();
        self.queue.push(e);
        EntityCommands { entity: e, commands: self }
    }
}

impl<'a, 's> EntityCommands<'a, 's> {
    pub fn id(&self) -> u64 { self.entity }
    pub fn insert(&mut self, _: u64) -> &mut Self {
        self.commands.queue.push(self.entity);
        self
    }
    pub fn reborrow(&mut self) -> EntityCommands<'_, 's> {
        EntityCommands { entity: self.entity, commands: &mut *self.commands }
    }
}
```

### 5.8 Drop semantics for EntityCommands

`EntityCommands` has no custom `Drop`. Default drop releases the `&'a mut Commands<'s>` reborrow. The reserved Entity ID is NOT freed on drop — SpawnAtCommand is already in the queue.

---

## §6 Command Bodies: SpawnAtCommand / InsertCommand / RemoveCommand / DespawnCommand (Section G)

### 6.1 SpawnAtCommand<B>

Unchanged from Round 2.

```rust
#[repr(C)]
pub(crate) struct SpawnAtCommand<B: Bundle> {
    pub(crate) entity: Entity,
    pub(crate) bundle: B,
}

unsafe impl<B: Bundle> Send for SpawnAtCommand<B> {}
unsafe impl<B: Bundle> Sync for SpawnAtCommand<B> {}

impl<B: Bundle> Command for SpawnAtCommand<B> {
    fn apply(self, world: &mut EcsMaster) {
        let archetype_id = B::cached_archetype_id(world);
        let arity = B::component_ids().len();
        debug_assert!(arity > 0 && arity <= MAX_BUNDLE_ARITY);

        let mut slots: [MaybeUninit<(ComponentId, &[u8])>; MAX_BUNDLE_ARITY] =
            [const { MaybeUninit::uninit() }; MAX_BUNDLE_ARITY];
        let slots_base = slots.as_mut_ptr() as *mut u8;
        let slot_stride = mem::size_of::<MaybeUninit<(ComponentId, &[u8])>>();
        let mut count = 0;
        let entity = self.entity;

        self.bundle.for_each_component_bytes(|id, bytes| {
            debug_assert!(count < MAX_BUNDLE_ARITY);
            unsafe {
                let slot_ptr = slots_base.add(count * slot_stride)
                    as *mut MaybeUninit<(ComponentId, &[u8])>;
                slot_ptr.write(MaybeUninit::new((id, bytes)));
            }
            count += 1;

            if count == arity {
                let initialized: &[(ComponentId, &[u8])] = unsafe {
                    std::slice::from_raw_parts(
                        slots_base as *const (ComponentId, &[u8]),
                        count,
                    )
                };
                debug_assert!(
                    world.entity_master().entities_inland
                        .get(entity.id().0)
                        .map_or(true, |i| i.is_null()),
                    "SpawnAtCommand applied to already-registered entity"
                );
                let _ = world
                    .create_entity_at(entity, archetype_id, initialized)
                    .expect("create_entity_at failed inside SpawnAtCommand::apply");
            }
        });
    }
}
```

### 6.2 New EcsMaster API: `create_entity_at`

Unchanged from Round 2 (see prior §6.2).

### 6.3 InsertCommand<B> + `merged_archetype_id` (W4 resolution)

Unchanged from Round 2. The W4 `migration_scratch` reuse remains valid.

### 6.4 RemoveCommand<C> + W1 absent-C policy

Unchanged from Round 2. Note: the absent-component check uses `source_arch.component_ids().contains(&component_id)` — a small linear scan over ≤ MAX_COMPONENTS-per-archetype (~10 typical). Hot-path acceptable.

### 6.5 DespawnCommand

Unchanged from Round 2.

---

## §7 Archetype Migration Algorithm (Section H)

### 7.1 High-level sequence

Unchanged from Round 2 in structure. Refined per C5 + Round 3 C-N2.

### 7.2 Insert migration — formal contract for `move_out_entity` (C5 + O2 + C-N2 + W-N2)

**Rename**: `forget_entity` → `move_out_entity`.

**ROUND 3 — W-N2 contract tightening**: the contract now explicitly states "MUST NOT invoke `drop_fn` on any slot (source or last)". This is stricter than Round 2's "no drop on source-row bytes" — removes ambiguity around the `last_row` slot.

```rust
impl Archetype {
    /// Phase 11 (C5, O2, W-N2 tightened): release a row WITHOUT invoking
    /// per-component Drop.
    ///
    /// # Caller contract — PRECONDITION
    ///
    /// For every component pool `P` in this archetype:
    /// - The bytes at `P.row[removed_unit_index]` MUST have been either:
    ///   (a) moved out via memcpy by the caller (typically into a target
    ///       archetype during migration), OR
    ///   (b) explicitly dropped via `ComponentPool::drop_at(removed_unit_index)`.
    ///
    /// Calling this with any pool whose source-row bytes were NOT moved or
    /// dropped is a leak — `Drop` will never run.
    ///
    /// # Behavior — POSTCONDITION
    ///
    /// For every pool `P` in this archetype:
    /// 1. Byte storage: `swap_remove_index_no_drop(removed_unit_index)`:
    ///    (a) copy bytes at `units[last].ptr()` → `units[removed].ptr()`
    ///        via `ptr::copy_nonoverlapping` (the existing chunked +
    ///        Unit-pointer storage model — see `swap_remove` in
    ///        `component_pool.rs:339` for the precedent),
    ///    (b) decrement `P.units.len()` via `units.pop()`,
    ///    (c) **W-N2 tightening**: MUST NOT invoke `P.drop_fn` on ANY slot
    ///        (neither `removed_unit_index` nor `last`). Caller assumes
    ///        full byte ownership tracking. This is stricter than
    ///        "no drop on source-row bytes" — both slots are bytewise
    ///        moved/copied; no destructor runs.
    /// 2. Tick storage: same swap-remove behavior, separately, for BOTH
    ///    `added_ticks` and `changed_ticks`:
    ///    `added_ticks[removed] = added_ticks[last]; changed_ticks[removed]
    ///    = changed_ticks[last]`. Ticks are `u32` POD, no drop concern.
    ///
    /// Archetype-level bookkeeping:
    /// - `entity_ids[removed] = entity_ids[last]; entity_ids.pop();`
    /// - `current_index -= 1`.
    /// - Return `RemoveOutcome::Swapped { moved_entity }` so the caller can
    ///   update the moved entity's `unit_index` in `EntityMaster`, or
    ///   `RemoveOutcome::Last` if `removed_unit_index == last_unit_index`.
    pub(crate) fn move_out_entity(
        &mut self,
        removed_unit_index: InlandPoolId,
    ) -> RemoveOutcome {
        let last_unit_index = InlandPoolId(self.current_index.saturating_sub(1));
        if removed_unit_index == last_unit_index {
            // Pop last: shrink each pool's units + tick rows by one, no drop.
            self.component_pools.pop_entity_no_drop();
            self.entity_ids.pop();
            self.current_index -= 1;
            return RemoveOutcome::Last;
        }
        let moved_entity = self.entity_ids[last_unit_index.0];
        // Per-pool swap-remove without drop, including both tick rows.
        self.component_pools.swap_remove_unit_no_drop(removed_unit_index.0);
        self.entity_ids.swap_remove(removed_unit_index.0);
        self.current_index -= 1;
        RemoveOutcome::Swapped { moved_entity }
    }
}
```

#### `ComponentPool::swap_remove_index_no_drop` — ROUND 3 REWRITE (C-N2)

The Round 2 sketch used `byte_ptr().add(idx * stride)` as if `ComponentPool` were a flat `Vec<u8>`. The actual storage (verified at `crates/boyko_ecs/src/ecs/memory/component_pool.rs:22-88`) is:
- `units: Vec<Unit>` — each `Unit` holds an absolute `*mut u8` into the arena (may be in different chunks for large pools).
- `added_ticks: Box<[UnsafeCell<Tick>]>` — accessed via `self.added_ticks[idx].get()`.
- `changed_ticks: Box<[UnsafeCell<Tick>]>` — same.

The existing `swap_remove` (line 339) follows this pattern:
```rust
let removed_ptr = self.units[index].ptr();
let last_ptr    = self.units[last_index].ptr();
unsafe {
    drop_fn(removed_ptr);  // ← we MUST NOT do this
    std::ptr::copy_nonoverlapping(last_ptr, removed_ptr, self.component_layout.size());
}
self.units[index] = Unit::new(removed_ptr);
// tick swap via *self.added_ticks[index].get() = *self.added_ticks[last_index].get()
self.units.pop();
```

The Round 3 `swap_remove_index_no_drop` mirrors this but skips the `drop_fn` invocation:

```rust
impl ComponentPool {
    /// Phase 11 (C5, C-N2): swap-remove row `idx` for byte storage + both
    /// tick storages. NO drop_fn invocation on either slot (W-N2 tightening).
    ///
    /// # Storage model
    ///
    /// This uses the existing chunked + Unit-pointer storage (`units: Vec<Unit>`
    /// where each Unit holds an absolute `*mut u8` into the arena, possibly
    /// in different chunks for large pools). Mirrors the existing
    /// `swap_remove` (line 339 of this file) but skips drop.
    ///
    /// # Safety
    ///
    /// * `idx < self.units.len()`.
    /// * Caller has ensured the source-row bytes were moved-out or dropped
    ///   per the `move_out_entity` contract (§7.2 PRECONDITION).
    /// * `&mut self` guarantees exclusive access (Phase 9 SCH3).
    pub(crate) unsafe fn swap_remove_index_no_drop(&mut self, idx: usize) {
        debug_assert!(idx < self.units.len(), "swap_remove_index_no_drop: idx out of bounds");
        let last_index = self.units.len() - 1;

        if idx != last_index {
            let removed_ptr = self.units[idx].ptr();
            let last_ptr = self.units[last_index].ptr();

            // SAFETY (mirrors existing `swap_remove` semantics):
            //   - removed_ptr and last_ptr are valid arena pointers obtained
            //     via prior `add`/`add_typed` calls.
            //   - They are non-overlapping: idx != last_index and each
            //     slot is `component_layout.size()` bytes. The pointers may
            //     live in different chunks (large pools span multiple
            //     chunks), but `copy_nonoverlapping` does not require them
            //     to be in the same allocation — only non-overlapping.
            //   - W-N2: NO drop_fn invocation on removed_ptr; caller has
            //     already moved or dropped those bytes per the contract.
            //   - NO drop_fn invocation on last_ptr either; the bytes are
            //     bitwise-copied into the removed slot, and last_ptr's
            //     slot becomes logically uninitialized (covered by the
            //     `units.pop()` below removing it from the active range).
            unsafe {
                core::ptr::copy_nonoverlapping(
                    last_ptr,
                    removed_ptr,
                    self.component_layout.size(),
                );
            }

            // Refresh the unit's pointer (preserves invariant that
            // self.units[idx].ptr() addresses the bytes for row idx).
            self.units[idx] = Unit::new(removed_ptr);

            // Tick swap — mirrors the existing swap_remove's tick block.
            // SAFETY: idx != last_index, both < self.units.len(). &mut self
            // gives exclusive access to the tick buffers; no concurrent
            // reader exists per Phase 9 SCH3.
            unsafe {
                let added_last = *self.added_ticks[last_index].get();
                let changed_last = *self.changed_ticks[last_index].get();
                *self.added_ticks[idx].get() = added_last;
                *self.changed_ticks[idx].get() = changed_last;
            }

            // Mark dirty (consistent with existing swap_remove).
            let chunk_idx = idx / self.components_per_chunk;
            if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
                chunk.mark_dirty();
            }
            let last_chunk_idx = last_index / self.components_per_chunk;
            if let Some(chunk) = self.chunks.get_mut(last_chunk_idx) {
                chunk.mark_dirty();
            }
        }
        // (idx == last_index): just pop. No byte/tick movement needed.
        // Tick rows for `last_index` become outside the active range; they
        // retain their POD `Tick` values until the next push overwrites.

        self.units.pop();
    }

    /// Phase 11 (C5): pop last row WITHOUT drop. Decrement units + tick rows
    /// are POD so no swap is required; they live in the shrunk range and
    /// will be overwritten on next push.
    pub(crate) fn pop_entity_no_drop(&mut self) {
        debug_assert!(!self.units.is_empty(), "pop_entity_no_drop on empty pool");

        // Mark dirty for the chunk that loses its last row.
        let last_index = self.units.len() - 1;
        let chunk_idx = last_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
            chunk.mark_dirty();
        }

        // W-N2: NO drop_fn invocation. Caller owns byte tracking.
        self.units.pop();
    }
}
```

`ComponentPoolBundle::swap_remove_unit_no_drop(idx) → forward to each pool`, `pop_entity_no_drop() → forward to each pool`.

#### Insert migration body (rewritten for C-N2)

```rust
fn migrate_entity_insert<B: Bundle>(
    world: &mut EcsMaster,
    entity: Entity,
    source_archetype_id: ArchetypeId,
    target_archetype_id: ArchetypeId,
    bundle: B,
) -> EcsResult<()> {
    let current_tick = world.current_tick();

    let source_ptr = world.archetype_master.archetype_ptr_for(source_archetype_id)
        .expect("invariant: source exists");
    let target_ptr = world.archetype_master.archetype_ptr_for(target_archetype_id)
        .expect("invariant: target just resolved");

    let inland = world.entity_master.entities_inland[entity.id().0];
    debug_assert!(!inland.is_null() && inland.generation() == entity.generation());
    let source_row = inland.unit_index() as usize;

    // SAFETY (U1, U2, U14, SCH7): exclusive &mut EcsMaster ⇒ no other Archetype refs alive.
    let source = unsafe { &mut *source_ptr };
    let target = unsafe { &mut *target_ptr };

    // Collect retained (source ∩ target) byte slices + their original ticks.
    // ROUND 3 C-N2: use `units[source_row].ptr()` (existing chunked storage),
    // NOT `byte_ptr().add(source_row * stride)` (which doesn't exist).
    let mut retained: [MaybeUninit<(ComponentId, &[u8], Tick, Tick)>; MAX_COLUMNS_PER_MIGRATION] =
        [const { MaybeUninit::uninit() }; MAX_COLUMNS_PER_MIGRATION];
    let mut retained_count = 0;

    for &target_cid in target.component_ids() {
        if source.component_ids().contains(&target_cid) {
            let pool = source.component_pools().get_pool(target_cid)
                .expect("invariant: retained component must exist in source");
            debug_assert!(source_row < pool.count(),
                "source_row out of bounds for retained component");
            let stride = pool.layout().size();

            // SAFETY (C-N2):
            //   - `pool.units[source_row].ptr()` is a valid arena pointer
            //     (initialized slot, < pool.count()).
            //   - The pointer is read-only valid for the lifetime of `source`
            //     (we hold &mut source through which we obtain &pool through
            //     get_pool — shared reborrow).
            //   - The bytes will be memcpy'd into target via
            //     `create_entity_with_ticks` BEFORE `source.move_out_entity`
            //     swaps them out — so the slice lifetime is bounded by
            //     the next mutating operation on source.
            //   - Slice is `stride` bytes; pool.layout().size() == stride.
            let bytes = unsafe {
                core::slice::from_raw_parts(pool.units[source_row].ptr(), stride)
            };
            let added = unsafe { pool.read_added_tick(source_row) };
            let changed = unsafe { pool.read_changed_tick(source_row) };
            retained[retained_count].write((target_cid, bytes, added, changed));
            retained_count += 1;
        }
    }

    // Collect bundle slices (target.component_ids() \ source.component_ids() ∪ overlapping replaces).
    // Bundle bytes WIN on overlap (Q6).
    let bundle_ids = B::component_ids();
    let mut bundle_slots: [MaybeUninit<(ComponentId, &[u8], Tick, Tick)>; MAX_BUNDLE_ARITY] =
        [const { MaybeUninit::uninit() }; MAX_BUNDLE_ARITY];
    let mut bundle_count = 0;
    bundle.for_each_component_bytes(|id, bytes| {
        bundle_slots[bundle_count].write((id, bytes, current_tick, current_tick));
        bundle_count += 1;
    });

    // Merge: bundle bytes override retained for any overlapping ComponentId.
    let mut combined: [MaybeUninit<(ComponentId, &[u8], Tick, Tick)>; MAX_COLUMNS_PER_MIGRATION] =
        [const { MaybeUninit::uninit() }; MAX_COLUMNS_PER_MIGRATION];
    let mut combined_count = 0;
    for i in 0..retained_count {
        combined[i] = retained[i];
    }
    combined_count = retained_count;
    for i in 0..bundle_count {
        let (b_id, b_bytes, b_added, b_changed) = unsafe { bundle_slots[i].assume_init() };
        let mut replaced = false;
        for j in 0..combined_count {
            let (c_id, _, _, _) = unsafe { combined[j].assume_init() };
            if c_id == b_id {
                combined[j].write((b_id, b_bytes, b_added, b_changed));
                replaced = true;
                break;
            }
        }
        if !replaced {
            combined[combined_count].write((b_id, b_bytes, b_added, b_changed));
            combined_count += 1;
        }
    }
    let combined_slice = unsafe {
        std::slice::from_raw_parts(
            combined.as_ptr() as *const (ComponentId, &[u8], Tick, Tick),
            combined_count,
        )
    };

    // Push into target with explicit ticks. This MEMCPYs every retained byte
    // slice INTO target's pools, completing the "move out" of the source row's
    // bytes — satisfies the `move_out_entity` PRECONDITION (a) from §7.2.
    let mut new_row: u32 = 0;
    let pushed = target.create_entity_with_ticks(
        entity.id(),
        &mut new_row,
        combined_slice,
        current_tick,
    );
    assert!(pushed, "target archetype rejected migration push");

    // SAFETY (C5, §7.2 PRECONDITION (a)): all source-row bytes for retained
    // components were memcpy'd into target's pools via the call above. The
    // source row's bytes are now redundant duplicates. `move_out_entity`
    // releases the storage WITHOUT running drop on either source or last
    // slot — W-N2 tightening — matching the contract.
    match source.move_out_entity(InlandPoolId(source_row)) {
        RemoveOutcome::Last => {}
        RemoveOutcome::Swapped { moved_entity } => {
            world.entity_master.entities_inland[moved_entity.0]
                .set_unit_index(source_row as u32);
        }
        RemoveOutcome::PoolFailure => {
            panic!("invariant: migration source removal must succeed");
        }
    }

    world.entity_master.entities_inland[entity.id().0] = EntityInland::new(
        target_ptr,
        new_row,
        entity.generation(),
    );

    Ok(())
}
```

### 7.3 Remove migration — drop discipline (C5)

Symmetric to insert with one explicit `drop_at` call before `move_out_entity`. ROUND 3 update: retained-byte extraction uses `units[source_row].ptr()`, same pattern as §7.2.

```rust
fn migrate_entity_remove<C: Component>(
    world: &mut EcsMaster,
    entity: Entity,
    source_archetype_id: ArchetypeId,
    target_archetype_id: ArchetypeId,
) -> EcsResult<()> {
    let current_tick = world.current_tick();
    let source_ptr = world.archetype_master.archetype_ptr_for(source_archetype_id).expect("invariant");
    let target_ptr = world.archetype_master.archetype_ptr_for(target_archetype_id).expect("invariant");
    let inland = world.entity_master.entities_inland[entity.id().0];
    let source_row = inland.unit_index() as usize;

    let source = unsafe { &mut *source_ptr };
    let target = unsafe { &mut *target_ptr };

    // Collect retained bytes + original ticks (target is strict subset of source).
    let mut retained: [MaybeUninit<(ComponentId, &[u8], Tick, Tick)>; MAX_COLUMNS_PER_MIGRATION] =
        [const { MaybeUninit::uninit() }; MAX_COLUMNS_PER_MIGRATION];
    let mut retained_count = 0;
    for &target_cid in target.component_ids() {
        let pool = source.component_pools().get_pool(target_cid).expect("invariant");
        let stride = pool.layout().size();
        // ROUND 3 C-N2: `units[source_row].ptr()` Unit-pointer (existing storage model).
        // SAFETY (same as §7.2): valid initialized slot; slice bounded by
        // the next mutating op on source (the drop_at + move_out_entity below).
        let bytes = unsafe {
            core::slice::from_raw_parts(pool.units[source_row].ptr(), stride)
        };
        let added = unsafe { pool.read_added_tick(source_row) };
        let changed = unsafe { pool.read_changed_tick(source_row) };
        retained[retained_count].write((target_cid, bytes, added, changed));
        retained_count += 1;
    }
    let combined_slice = unsafe {
        std::slice::from_raw_parts(
            retained.as_ptr() as *const (ComponentId, &[u8], Tick, Tick),
            retained_count,
        )
    };

    let mut new_row: u32 = 0;
    let pushed = target.create_entity_with_ticks(entity.id(), &mut new_row, combined_slice, current_tick);
    assert!(pushed);

    // C5 discipline: the removed component C's bytes are still owned by
    // source. We MUST explicitly drop them BEFORE move_out_entity, because
    // move_out_entity (W-N2 tightening) skips drop on ALL components.
    {
        let removed_pool = source.component_pools_mut().get_pool_mut(C::component_id())
            .expect("invariant: source has C (verified by RemoveCommand::apply)");
        // SAFETY: source_row < removed_pool.count(); &mut source ⇒ exclusive access.
        unsafe { removed_pool.drop_at(source_row) };
    }

    // Now move_out_entity swap-removes ALL pools (including the just-dropped C pool).
    // For the C pool: drop_at zeroed the logical state at source_row; the swap from
    // last_row brings live bytes back into source_row. Net effect: C@source_row is the
    // moved entity's bytes (no double-drop, no leak — see W-N2 tightening for proof).
    match source.move_out_entity(InlandPoolId(source_row)) {
        RemoveOutcome::Last => {}
        RemoveOutcome::Swapped { moved_entity } => {
            world.entity_master.entities_inland[moved_entity.0]
                .set_unit_index(source_row as u32);
        }
        RemoveOutcome::PoolFailure => panic!("invariant"),
    }
    world.entity_master.entities_inland[entity.id().0] = EntityInland::new(
        target_ptr,
        new_row,
        entity.generation(),
    );
    Ok(())
}
```

### 7.4 In-place replace path — single linear sequence (C3 + W-N1)

ROUND 3 — W-N1 fix: cite the canonicalization invariant in the SAFETY block and add a defensive debug check.

**Canonicalization invariant (cited from `archetype_master.rs:99-133, 462-473`)**:
> `ArchetypeMaster::get_or_create_archetype(component_ids)` computes `ComponentMask::from_components(component_ids)` and calls `ArchetypeRegistry::find_exact_match(&mask)`. This is an **exact-mask** match — two archetypes with the same `ComponentMask` are not created twice; the function returns the existing ID. Therefore: **same component set ⇒ same ArchetypeId** (modulo mask resolution).

Consequence for `apply_replace_in_place`: `merged_archetype_id == source_archetype_id` ⇒ `(source ∪ bundle).mask == source.mask` ⇒ `bundle ⊆ source` (every component in the bundle was already in source). Therefore the `get_pool_mut(component_id)` lookup for every bundle component MUST succeed.

```rust
impl<B: Bundle> InsertCommand<B> {
    fn apply_replace_in_place(self, world: &mut EcsMaster) {
        let current_tick = world.current_tick();
        let entity = self.entity;
        let entity_id = entity.id().0;

        // Step 1: resolve inland slot exactly once.
        let inland = world.entity_master.entities_inland[entity_id];
        if inland.is_null() || inland.generation() != entity.generation() {
            debug_assert!(false, "stale entity in InsertCommand::apply_replace_in_place");
            return;
        }
        let archetype_ptr = inland.archetype_ptr();
        let row = inland.unit_index() as usize;

        // Step 2: mint exclusive &mut Archetype from the inland's archetype_ptr.
        // SAFETY (U1, U2, U14, SCH7):
        //   - We hold `&mut EcsMaster` (dispatcher apply window).
        //   - `archetype_ptr` is write-capable provenance (minted by
        //     Phase 7 `ArchetypeMaster::archetype_ptr_for`).
        //   - The archetype slab is stable for the EcsMaster's lifetime
        //     (Phase 9 SEND6); the pointer is valid as long as `world`
        //     is borrowed exclusively (which is true here).
        let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };

        // Step 3: linear iteration over bundle bytes. For each:
        // (a) drop the existing component at `row` via the pool's drop_fn,
        // (b) memcpy the new bytes into the same slot,
        // (c) bump changed_tick to current_tick (Phase 10 STORE3).
        //
        // SAFETY (STORE3, SCH3, W-N1):
        //   - Canonicalization invariant (cited from `archetype_master.rs:99-133,
        //     462-473`): `ArchetypeMaster::get_or_create_archetype` uses
        //     `ComponentMask::from_components(...)` + `find_exact_match` —
        //     same component set ⇒ same ArchetypeId. The caller (§6.3)
        //     only enters this path when `merged_archetype_id ==
        //     source_archetype_id`, which by canonicalization implies
        //     `(source ∪ bundle).mask == source.mask` ⇒ `bundle ⊆ source`.
        //     Therefore the `get_pool_mut(component_id)` lookup below is
        //     guaranteed to succeed.
        //   - W-N1 defensive: in debug builds we ALSO assert
        //     `archetype.component_pools().has_pool(component_id)` before
        //     the `expect`, surfacing any future regression in
        //     canonicalization at the assertion site rather than at the
        //     less-helpful `expect` panic.
        //   - The pool is resolved from `archetype` (the exclusive &mut),
        //     so no aliasing.
        //   - `pool.drop_at(row)` runs the old value's destructor; the slot
        //     is then logically uninitialized until `pool.write_at(row, bytes)`
        //     re-initializes it.
        //   - `pool.write_changed_tick(row, current_tick)` is sound because
        //     we hold &mut Archetype ⇒ &mut Pool.
        //   - `added_tick` is NOT bumped (OQ5 / EC9: the replace-in-place
        //     fast path preserves the original add tick; the migration path
        //     over-bumps; Phase 12 unifies via `is_new` flag).
        self.bundle.for_each_component_bytes(|component_id, bytes| {
            debug_assert!(
                archetype.component_pools().has_pool(component_id),
                "W-N1: bundle component {:?} absent from source archetype despite \
                 canonicalization invariant — get_or_create_archetype regression?",
                component_id
            );
            let pool = archetype.component_pools_mut().get_pool_mut(component_id)
                .expect("invariant: target == source ⇒ bundle ⊆ source (canonicalization)");
            debug_assert!(row < pool.count(), "replace-in-place row out of bounds");
            unsafe {
                pool.drop_at(row);
                pool.write_at(row, bytes);
                pool.write_changed_tick(row, current_tick);
            }
        });
    }
}
```

Required new method: `ComponentPoolBundle::has_pool(component_id) -> bool` — already trivially derivable from existing `get_pool(component_id).is_some()`; we add the explicit alias for clarity at the debug-assert site.

### 7.5 Cost analysis (refined for Round 3)

ROUND 3 — Unit-pointer dereference cost adjustment: the retained-bytes extraction `units[source_row].ptr()` does one extra L1 load compared to a hypothetical flat `byte_ptr().add(stride * row)`. In the warm case, this load hits L1 (Unit is 8 B; `units: Vec<Unit>` is densely packed, sequential prefetch picks it up cheaply). In the cold case, large pools spanning multiple chunks may see a cache miss on the Unit array, costing ~10 ns extra. Adjusted target: ≤720 ns warm (was 700 ns), ≤900 ns cold.

| Step | Cost (warm) | Notes |
|---|---|---|
| `get_entity_archetype_id` | ~3 ns | Phase 7 fast inland read |
| `merged_archetype_id` (stack-merge, ≤32 components) | ~50 ns | union + dedup + sort + `get_or_create_archetype` cache hit |
| `archetype_ptr_for` × 2 | ~10 ns | bundle slab lookup |
| Retained-bytes collect (3 components) | ~40 ns | 3 × Unit deref + 3 × slice ctor; +10 ns vs Round 2 for unit indirection |
| Retained-tick read (3 × 2 reads) | ~30 ns | 6 tick loads via `*added_ticks[idx].get()` |
| Bundle walk (3 components) | ~30 ns | Phase 8.5 baseline |
| Merge retained + bundle (3 + 3) | ~20 ns | overlap dedup |
| `target.create_entity_with_ticks` (6 components) | ~310 ns | memcpy 6 components × ~50 ns each |
| `source.move_out_entity` (swap-remove, 3 cols + 2 tick rows each) | ~90 ns | byte + tick swap × 5 rows; +10 ns vs Round 2 (unit indirection at swap-time) |
| `EntityInland::new` write | ~5 ns | 16 B store |
| **Total insert migration (warm)** | **~590 ns** | Target ≤ 720 ns ✓ (Round 3 adjusted, +20 ns headroom) |
| Replace-in-place (3 components) | ~90 ns | drop + memcpy + tick × 3 |
| Remove migration | ~610 ns | + drop_at (~10 ns) - bundle steps; +20 ns vs Round 2 |

### 7.6 Wider archetypes (>32 components)

Stack-merge cap raised to 32 (W4). Wider archetypes fall back to `EcsMaster::migration_scratch` reuse — zero alloc after warmup.

---

## §8 Phase 9 Parallel Integration (Section K)

### 8.1 Cross-thread call graph

Unchanged from Round 2. The only new entity-minting path on workers is `EntityCounter::reserve_entity(&self)` (§5.5); the underlying mechanism is identical to the inline method's `fetch_add(Relaxed)` on `EntityMaster::next_entity_id`.

### 8.2 Aliasing safety proof

**Claim**: `EntityCounter::reserve_entity(&self)` from N parallel workers is data-race-free.

**Proof**:
1. `EntityCounter` carries a `*const AtomicUsize`. The only dereference is through `(*ptr).fetch_add(Relaxed)` — an atomic RMW. Atomic operations are data-race-free.
2. EM6 invariant: no other field of `EntityMaster` is reachable through the EntityCounter's pointer type. Future maintainers cannot accidentally read non-atomic state via this channel — the compiler rejects it (`*const AtomicUsize` has no `entities_inland` field).
3. Concurrent reads of `entities_inland` via query iteration are on a DIFFERENT field accessed through a DIFFERENT channel (`Query`'s `UnsafeEcsCell`) — no race.
4. Dispatcher `&mut self` paths run only in apply window, excluded from worker `&self` via SCH7.

### 8.3 Memory ordering soundness

Worker writes Entity to SpawnAtCommand → CommandQueue (per-system, single-writer). Dispatcher reads via Acquire on completion_queue. Apply path holds `&mut EcsMaster` exclusively. Happens-before edge established.

### 8.4 SystemParam access for Commands (post-Phase-11)

Commands::init_access still declares NO access. EntityCounter access is conflict-free (same policy as EventDispatcher per EVT1):

> EntityMaster's atomic counter for `reserve_entity` is conflict-free by construction: only field reachable via EntityCounter is the atomic counter; atomic RMW is thread-safe.

### 8.5 Worker spawn → same-frame query observability

A worker calls `spawn` at tick T. Same-frame query: Entity is NOT visible until apply. Register happens at apply time only.

### 8.6 Multi-worker spawn ID consistency

Workers W1 + W2 simultaneous spawn → atomic fetch_add ensures distinct IDs. Both SpawnAtCommands enqueued into distinct per-system queues. Apply registers at distinct slots.

### 8.7 NEW: Per-invocation lifecycle of Commands (W-N3)

The Round 2 SAFETY block in §5.6 hand-waved "the pointer is valid for 's per SystemParam contract" without explicitly stating when the pointer is minted, refreshed, or invalidated. W-N3 demands explicit treatment.

**Per Phase 8c IntoSystem contract** (`crates/boyko_ecs/src/ecs/core/system/function_system.rs`, `into_system.rs`):
- Each `FunctionSystem<F, M>` invocation calls `SystemParam::get_param` **once per system body execution** (each frame, each schedule pass).
- `get_param` receives a fresh `UnsafeEcsCell<'w>` whose `'w` is bounded by the **current** apply window (or the parallel-execution window pre-apply per Phase 9 SCH7).
- The returned `Self::Item<'w, 's>` (i.e. `Commands<'s>` in this case) is dropped at the **end of the system body** — never persists across frames.

**Phase 11-specific implications**:
1. The `EntityCounter`'s `*const AtomicUsize` is **re-minted every frame** by `get_param`. Across-frame staleness is impossible because:
   - At frame F end: `Commands<'s>` is dropped; the EntityCounter inside it ceases to exist.
   - At frame F+1 start: `get_param` is called anew with a fresh `UnsafeEcsCell<'w>`; a new EntityCounter is minted from `world.entity_counter()`.
2. The atomic counter's address itself (i.e. the address of `EntityMaster::next_entity_id`) is **stable across frames** — `EntityMaster` lives inside `Box<EcsMaster>` whose heap allocation does not move. Therefore the pointer values seen across frames are consistent (point to the same atomic). But the *provenance* / Stacked Borrows tag is refreshed each frame via `get_param`'s `UnsafeEcsCell` reborrow.
3. The `'s` (state) lifetime is the lifetime of the `CommandQueue` slot in `SystemMeta`. `'s` spans the system body but does NOT outlive `'w` because `Item<'w, 's>` is `Commands<'s>` and `Commands` itself only escapes its construction call by being the system body's argument (controlled by `IntoSystem::run`).

**Documented contract (added to `Commands<'s>` rustdoc)**:
> `Commands<'s>` is constructed by `SystemParam::get_param` once per system invocation. The contained `EntityCounter<'s>` carries a raw pointer minted from the current `UnsafeEcsCell<'w>`. Per the Phase 8c IntoSystem contract, `'w >= 's` for the duration of the system body, and the `Commands` value is dropped at body end. The pointer's apparent validity for `'s` is sound because: (a) it never escapes the body, (b) the underlying `AtomicUsize` address is stable across frames, and (c) the provenance is refreshed each frame via the fresh `UnsafeEcsCell` reborrow.

### 8.8 Soundness restated end-to-end

End-to-end soundness of the worker spawn path (per system invocation, per frame):

1. **Frame F start**: scheduler begins. Per Phase 9 SCH7, workers run in a window where dispatcher does not hold `&mut EcsMaster`.
2. **`FunctionSystem::run` for system S**: calls `SystemParam::get_param(state, meta, world: UnsafeEcsCell<'w>)`.
3. **`Commands::get_param`** mints `EntityCounter<'s>` via `world.entity_counter::<'s>()` — encapsulates `*const AtomicUsize` aimed at `EntityMaster::next_entity_id`.
4. **System body runs**: calls `commands.spawn(bundle)`. `EntityCounter::reserve_entity` does `fetch_add(Relaxed)` on the atomic — distinct ID guaranteed by atomic RMW (§4.6 Case 3).
5. **SpawnAtCommand pushed** into `state: &'s mut CommandQueue` (per-system, single-writer, no contention).
6. **System body returns**. `Commands<'s>` is dropped — EntityCounter inside ceases. The raw pointer's apparent validity ends with `'s`.
7. **Dispatcher apply window**: `CommandQueue::apply(world: &mut EcsMaster)` runs each command. `SpawnAtCommand::apply` calls `world.create_entity_at(entity, ...)` which registers the entity in `entities_inland` (slot at `entity.id().0` was previously NULL because `reserve_entity` only minted the ID counter; the slot is populated now).
8. **Frame F+1**: get_param re-mints the EntityCounter from a fresh `UnsafeEcsCell<'w>`. Validity bounded by frame F+1's `'w`. No cross-frame staleness.

---

## §9 Phase 10 Tick Integration (Section J)

### 9.1 Spawn ticks (unchanged)

`Archetype::create_entity` writes `added = changed = current_tick`.

### 9.2 Insert ticks + `current_tick` role clarification (W3)

Unchanged from Round 2.

### 9.3 Remove ticks

Symmetric. Retained components preserve original ticks via `create_entity_with_ticks`. Removed component's bytes are explicitly dropped via `drop_at` before `move_out_entity`. Tick rows for the removed pool are swap-removed (no drop, POD).

### 9.4 Despawn ticks

`EcsMaster::delete_entity` unchanged.

### 9.5 OQ5 decision recorded — Added<T> over-reporting on replace

Unchanged from Round 2.

---

## §10 Hot-Path Perf Projections (Section §1.2 expanded)

### 10.1 Enqueue costs

| Operation | Steps | ns |
|---|---|---|
| `Commands::spawn(b)` | `entity_counter.reserve_entity()` (10 ns single-thread, up to 60 ns under N=8 contention per §10.5) + `push(SpawnAtCommand)` (18 ns) | 28-78 ns |
| `EntityCommands::id()` | inline field read | 0 ns |
| `EntityCommands::insert(b)` | `push(InsertCommand)` (18 ns) | ~18 ns |
| `EntityCommands::remove::<C>()` | `push(RemoveCommand)` (18 ns) | ~18 ns |
| `EntityCommands::despawn()` | `push(DespawnCommand)` (18 ns) | ~18 ns |
| `Commands::entity(id)` | `EntityCommands` ctor | 0 ns |
| `Commands::despawn(id)` | `push(DespawnCommand)` (18 ns) | ~18 ns |
| Full chain: `spawn(b).insert(c).insert(d).id()` (single-thread) | 28 + 18 + 18 + 0 | ~64 ns |

### 10.2 Apply costs (warm path) — Round 3 refined

| Operation | Steps | ns |
|---|---|---|
| `SpawnAtCommand::apply` | `cached_archetype_id` (3 ns) + bundle walk (30 ns) + `create_entity_at` (~300 ns) | ~330 ns |
| `InsertCommand::apply` (migration, 3 retained + 3 bundle) | source lookup (3 ns) + `merged_archetype_id` (50 ns) + migration (590 ns Round 3) | ~640 ns |
| `InsertCommand::apply` (in-place replace, 3 components) | source lookup (3 ns) + replace path (90 ns) | ~95 ns |
| `RemoveCommand::apply` | source lookup (3 ns) + `without_component_archetype_id` (50 ns) + migration (610 ns Round 3) | ~660 ns |
| `DespawnCommand::apply` | `delete_entity` (~500 ns baseline) | ~500 ns |

### 10.3 10k mixed frame budget (updated for Round 3 unit-pointer adjustment)

Plan: 5,000 spawns + 2,000 inserts + 1,000 removes + 2,000 despawns. 4 worker threads.

| Phase | Cost | Total |
|---|---|---|
| Enqueue (parallel, 4 workers, contention factor ~3x — §10.5) | 10k × ~30 ns/spawn-equiv | ~300 µs |
| Apply: 5k spawns | 5k × 330 ns | 1.65 ms |
| Apply: 2k inserts | 2k × 640 ns | 1.28 ms |
| Apply: 1k removes | 1k × 660 ns | 660 µs |
| Apply: 2k despawns | 2k × 500 ns | 1.0 ms |
| **Total** | | **~4.9 ms** ✓ |

Updated target ≤ 5.5 ms. Round 3 adjustment +~100 µs vs Round 2 from Unit-pointer dereference cost is absorbed in the 600 µs of headroom.

### 10.4 Cache footprint per system

- `Commands<'s>` size: 16 B (`&'s mut CommandQueue` 8 B + `EntityCounter<'s>` 8 B). One cache line.
- `EntityCommands<'a, 's>` size: 16 B (8 + 8). Static-asserted in tests (§13.1).
- `EntityCounter<'s>` size: 8 B (`*const AtomicUsize` 8 + ZST PhantomData). `#[derive(Clone, Copy)]`.
- `CommandQueue` size (Phase 8d O2): 56 B. Stack-resident in `SystemMeta`.
- `SpawnAtCommand<3-component-bundle>` queue slot: 8 + 8 + ~48 = 64 B — one cache line.
- `InsertCommand<3-component-bundle>` queue slot: same 64 B.
- `RemoveCommand<C>` queue slot: 8 + 8 = 16 B.
- `DespawnCommand` queue slot: 8 + 8 = 16 B.

100 commands × 64 B = 6.4 KB — fits L1d (32 KB).

### 10.5 Parallel contention sensitivity (W5 + O-N1 polish)

**Cost model for `EntityCounter::reserve_entity` under N concurrent threads**:

| N | Cost/op | Notes |
|---|---|---|
| 1 | ~10 ns | `fetch_add(Relaxed)` cache hit on local core |
| 2 | ~20 ns | one cache-line bounce per op on average |
| 4 | ~30 ns | linear contention growth (RFO + cache-line transfer) |
| 8 | ~60 ns | saturating regime — cache coherence protocol traffic dominates |
| 16 | ~120 ns | NUMA effects on multi-socket; not target hardware |

These numbers are derived from the well-known x86_64 `lock xadd` cost model:
- Uncontended `lock xadd` (Relaxed atomic RMW): ~6-10 ns (L1 hit).
- Contended: each waiting thread incurs ~15-25 ns cache-line transfer (MESI invalidation + line refetch).
- N=8 saturation: ~60 ns is the steady state at which RFO traffic equals the protocol's bus bandwidth.

**Reconciliation with §10.3 budget**:
- 10k spawns × ~30 ns (N=4 average) = 300 µs in the enqueue phase. Matches §10.3.
- 10k spawns × ~100 ns (N=8 worst case) = 1 ms. Still within the 5.5 ms frame budget.

**Mitigation if telemetry shows contention as a hotspot**:
- Cache-line pad `next_entity_id` to its own 64 B cache line via `#[repr(C, align(64))]` wrapper. Adds 56 B of padding to EntityMaster but eliminates false-sharing with neighboring `free_entity_ids` Vec header.
- Even more aggressive: per-thread reservation pools (each thread fetches a batch of K IDs at once via `fetch_add(K)`, then hands out from the batch locally). Reduces atomic frequency by K×. Considered in §16.6 (rejected for v1 — atomic is already <1% of frame budget).

**Telemetry trigger** (O-N1 polish): if `bench_reserve_entity_parallel_8_threads` (§13.4) shows steady-state >100 ns/op, the cache-line padding mitigation lands in Phase 12. Until then, the cost model in this table is taken as the authoritative estimate; the bench validates it on first land. Decision: ship v1 without padding.

---

## §11 Memory Layouts + Sizes

### 11.1 EntityCommands (O1 re-verified)

```
+0  : entity: Entity              (8 B: id u32 + gen u32)
+8  : commands: &'a mut Commands  (8 B: pointer)
+16 : end
```

Size: 16 B. Align: 8. `!Send + !Sync`.

### 11.2 SpawnAtCommand<B>

Unchanged from Round 2.

### 11.3 InsertCommand<B>

Identical to SpawnAtCommand<B>.

### 11.4 RemoveCommand<C>

```
+0  : entity: Entity        (8 B)
+8  : _marker: PhantomData  (0 B)
+8  : end
```

Size: 8 B. Align: 8.

### 11.5 DespawnCommand

```
+0  : entity: Entity (8 B)
+8  : end
```

Size: 8 B. Align: 8.

### 11.6 EntityMaster after Phase 11

```
+0   : free_entity_ids: Vec<EntityId>            (24 B)
+24  : next_entity_id: AtomicUsize               (8 B)
+32  : entities_inland: Vec<EntityInland>        (24 B)
+56  : sparse_to_active: Vec<u32>                (24 B)
+80  : active_ids: Vec<EntityId>                 (24 B)
+104 : end
```

Net 104 B. Two cache lines.

### 11.7 Commands<'s> after Phase 11 (Round 3 — EntityCounter)

```
+0  : queue: &'s mut CommandQueue       (8 B)
+8  : entity_counter: EntityCounter<'s> (8 B = *const AtomicUsize + ZST PhantomData)
+16 : end
```

Size: 16 B. One cache line. `!Send + !Sync` enforced by `&'s mut CommandQueue` (CQ-SEND2). The `EntityCounter<'s>` is `Send + Sync` (its only operation is atomic-RMW), but `Commands<'s>` inherits `!Sync` from the `&mut CommandQueue` field.

### 11.8 Atomic counter contention model (refined W5)

Unchanged from Round 2. The EntityCounter's pointer aims at the SAME atomic field — contention characteristics identical.

### 11.9 EcsMaster::migration_scratch (W4)

Unchanged from Round 2.

### 11.10 NEW: EntityCounter<'s> layout (Round 3)

```
+0  : next_id_ptr: *const AtomicUsize  (8 B)
+8  : _marker: PhantomData             (0 B ZST)
+8  : end
```

Size: 8 B. Align: 8. `Send + Sync` via explicit impls. `#[derive(Clone, Copy)]` — trivial copy.

---

## §12 Public API Surface

### 12.1 New types

```rust
// crates/boyko_ecs/src/ecs/core/commands/entity_commands.rs (NEW FILE)
pub struct EntityCommands<'a, 's> { /* fields per §5.1 */ }

impl<'a, 's> EntityCommands<'a, 's> {
    pub fn id(&self) -> Entity;
    pub fn insert<B: Bundle>(&mut self, bundle: B) -> &mut Self;
    pub fn remove<C: Component>(&mut self) -> &mut Self;
    pub fn despawn(&mut self) -> &mut Self;
    pub fn try_insert<B: Bundle>(&mut self, bundle: B) -> &mut Self;
    pub fn try_remove<C: Component>(&mut self) -> &mut Self;
    pub fn try_despawn(&mut self) -> &mut Self;
    pub fn reborrow(&mut self) -> EntityCommands<'_, 's>;
}

// crates/boyko_ecs/src/ecs/core/system/params/entity_counter.rs (NEW FILE — Round 3 C-N1)
pub struct EntityCounter<'s> { /* private fields per §5.5 */ }

impl<'s> EntityCounter<'s> {
    pub fn reserve_entity(&self) -> Entity;     // public — only API surface
    // pub(crate) unsafe fn from_ptr(ptr: *const AtomicUsize) -> Self;  // crate-only ctor
}
```

### 12.2 Modified Commands<'s> (Round 3 — EntityCounter)

```rust
pub struct Commands<'s> {
    pub(crate) queue: &'s mut CommandQueue,
    pub(crate) entity_counter: EntityCounter<'s>,    // Round 3: typed newtype, not raw ptr
}

impl<'s> Commands<'s> {
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> EntityCommands<'_, 's>;  // RETURN CHANGED
    pub fn entity(&mut self, entity: Entity) -> EntityCommands<'_, 's>;        // NEW
    pub fn despawn(&mut self, entity: Entity);                                  // NEW

    pub fn add<C: Command>(&mut self, cmd: C);                                  // unchanged
    pub fn send_event<E: Event>(&mut self, event: E);                           // unchanged
}
```

Removed from Round 2: `pub(crate) fn entity_master(&self) -> &EntityMaster` accessor. The new shape never produces a `&EntityMaster` — only `&EntityCounter`. Aliasing rule type-enforced.

### 12.3 Modified EntityMaster (W2 privacy + C-N1)

```rust
impl EntityMaster {
    // Round 3 C-N1: REMOVED `pub fn reserve_entity(&self) -> Entity`.
    // Replaced by EntityCounter::reserve_entity. The atomic counter is
    // exposed crate-internally via `next_id_atomic` (§4.2) for
    // UnsafeEcsCell::entity_counter to project from.

    pub(crate) fn next_id_atomic(&self) -> &AtomicUsize;       // NEW (Round 3, crate-only)
    pub(crate) fn allocate_entity(&mut self) -> Entity;         // PRIVACY CHANGED (W2)
    // Other methods unchanged.
}
```

### 12.4 Modified EcsMaster

```rust
impl EcsMaster {
    pub fn create_entity_at(
        &mut self,
        entity: Entity,
        archetype_id: ArchetypeId,
        components: &[(ComponentId, &[u8])],
    ) -> EcsResult<Entity>;                    // NEW (§6.2)

    pub(crate) fn migration_scratch_mut(&mut self) -> &mut Vec<ComponentId>; // NEW (W4)
}
```

### 12.5 New Archetype API

```rust
impl Archetype {
    pub(crate) fn create_entity_with_ticks(
        &mut self,
        entity_id: EntityId,
        new_unit_index: &mut u32,
        components: &[(ComponentId, &[u8], Tick, Tick)],
        current_tick: Tick,
    ) -> bool;                                // NEW (§9.2)

    pub(crate) fn move_out_entity(             // RENAMED from forget_entity (O2)
        &mut self,
        removed_unit_index: InlandPoolId,
    ) -> RemoveOutcome;                       // NEW (§7.2)
}
```

### 12.6 New ComponentPool API (Round 3 rewrite — C-N2)

The API operates on `units[idx].ptr()` + `added_ticks[idx]` / `changed_ticks[idx]` — the existing chunked + Unit-pointer storage. No `byte_ptr()` / stride arithmetic.

```rust
impl ComponentPool {
    /// Run drop_fn on the slot at `index`. Bytes are logically uninitialized
    /// after; the next write_at or swap_remove_index_no_drop refreshes.
    pub(crate) unsafe fn drop_at(&mut self, index: usize);                       // NEW (§7.3)

    /// Swap-remove the row at `idx` without invoking drop_fn on EITHER source
    /// or last slot (W-N2 tightening). Mirrors the existing `swap_remove`
    /// flow (line 339 of this file) over the chunked + Unit-pointer storage,
    /// minus the drop_fn call.
    ///
    /// Steps (per §7.2):
    ///   1. If idx != last: `copy_nonoverlapping(units[last].ptr(),
    ///      units[idx].ptr(), layout.size())`. Refresh `units[idx]` to the
    ///      destination pointer. Mark dirty for both chunks.
    ///   2. Tick swap: `*added_ticks[idx].get() = *added_ticks[last].get()`
    ///      and same for `changed_ticks`.
    ///   3. `units.pop()`.
    pub(crate) unsafe fn swap_remove_index_no_drop(&mut self, idx: usize);       // NEW (§7.2, C5, C-N2)

    /// Pop last row. No drop_fn invocation. Marks dirty for the affected chunk.
    /// Tick rows are POD; no swap needed for pop.
    pub(crate) fn pop_entity_no_drop(&mut self);                                 // NEW (§7.2, C5)

    /// Write bytes into the slot at `index`. Slot MUST be logically uninitialized
    /// (just after `drop_at`) — caller responsible. Uses `units[index].ptr()` as
    /// destination.
    pub(crate) unsafe fn write_at(&mut self, index: usize, bytes: &[u8]);        // NEW (§7.4)

    /// Read `added_tick` at `index`. Used in retained-byte extraction (§7.2/§7.3).
    pub(crate) unsafe fn read_added_tick(&self, index: usize) -> Tick;           // NEW (§7.2)

    /// Read `changed_tick` at `index`. Used in retained-byte extraction.
    pub(crate) unsafe fn read_changed_tick(&self, index: usize) -> Tick;         // NEW

    /// Write `changed_tick` at `index`. Used in replace-in-place (§7.4).
    pub(crate) unsafe fn write_changed_tick(&mut self, index: usize, tick: Tick); // NEW (§7.4)
}

impl ComponentPoolBundle {
    /// Forwarder: `swap_remove_index_no_drop` on every pool.
    pub(crate) unsafe fn swap_remove_unit_no_drop(&mut self, idx: usize);        // NEW
    /// Forwarder: `pop_entity_no_drop` on every pool.
    pub(crate) fn pop_entity_no_drop(&mut self);                                 // NEW
    /// Returns true if a pool for `component_id` exists. W-N1 defensive check.
    pub(crate) fn has_pool(&self, component_id: ComponentId) -> bool;            // NEW (§7.4, W-N1)
}
```

### 12.7 Modified UnsafeEcsCell (Round 3 — EntityCounter projection)

```rust
impl<'w> UnsafeEcsCell<'w> {
    /// Phase 11 Round 3 (C-N1): mints an `EntityCounter<'s>` projecting only
    /// the atomic counter from EntityMaster — never exposes the full master.
    ///
    /// The returned EntityCounter carries a `*const AtomicUsize` aimed at
    /// `EntityMaster::next_entity_id`. The lifetime `'s` may be shorter than
    /// `'w`; the caller (typically `Commands::get_param`) ties `'s` via
    /// PhantomData re-tag.
    ///
    /// # Safety (U_C2)
    /// * The caller asserts that the active `SystemParam::init_access`
    ///   permits the conflict-free atomic-counter access (Commands declares
    ///   no access in the conflict graph — EVT1 precedent + EM6).
    /// * The by-value receiver keeps the raw pointer's provenance intact.
    /// * `'s <= 'w` per the SystemParam contract (§8.7).
    #[inline]
    pub(crate) unsafe fn entity_counter<'s>(self) -> EntityCounter<'s> {
        // SAFETY (U_C2, EM6): by-value receiver — no &self retag. The
        //   underlying *mut EcsMaster is valid for 'w; projecting
        //   `&(*ptr).entity_master.next_id_atomic()` and re-casting to
        //   *const AtomicUsize produces a raw pointer whose validity is
        //   bounded by 'w (>= 's). EntityCounter::from_ptr re-tags to 's
        //   via PhantomData.
        let em = unsafe { &(*self.ptr).entity_master };
        let atomic_ptr = em.next_id_atomic() as *const AtomicUsize;
        unsafe { EntityCounter::from_ptr(atomic_ptr) }
    }
}
```

(Assumes `EcsMaster` has a field `entity_master: EntityMaster` — verified in `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`.)

### 12.8 Usage examples

```rust
// Basic spawn + chain.
fn spawn_player(mut commands: Commands) {
    let player = commands.spawn(PlayerBundle {
        pos: Position(0.0, 0.0, 0.0),
        vel: Velocity::ZERO,
    })
    .insert(HealthBundle { hp: 100, max_hp: 100 })
    .insert(InventoryBundle::default())
    .id();
    info!("spawned player {:?}", player);
}

// Conditional builder.
fn spawn_enemy(mut commands: Commands, is_boss: bool) {
    let mut e = commands.spawn(EnemyBundle { /* ... */ });
    if is_boss {
        e.insert(BossBundle { phase: 0 });
    }
    let id = e.id();
    info!("spawned enemy {:?}", id);
}

// Existing entity.
fn damage_player(mut commands: Commands, player: Entity, dmg: u32) {
    commands.entity(player).insert(DamageBundle { dmg });
}

// Convenience despawn.
fn kill_player(mut commands: Commands, player: Entity) {
    commands.despawn(player);
}

// Reborrow pattern (EC4).
fn complex_helper(mut commands: EntityCommands<'_, '_>) {
    helper_subroutine(commands.reborrow());  // pass shorter-lifetime copy
    commands.insert(MoreComponents);          // original still usable
}
```

---

## §13 Test Plan

### 13.1 Unit tests (Round 3 updates)

| File | Test name | Asserts |
|---|---|---|
| `entity_master.rs` | `next_id_atomic_lock_free_under_parallel_load` | 8 threads × 1000 atomic fetch_add = 8000 unique |
| `entity_master.rs` | `allocate_entity_is_pub_crate_only` | trybuild fail: external `allocate_entity` call fails to compile |
| `entity_counter.rs` | `entity_counter_size_is_8_bytes` | `mem::size_of::<EntityCounter<'_>>() == 8` (Round 3) |
| `entity_counter.rs` | `entity_counter_is_send_and_sync` | compile-test `is_send::<EntityCounter<'static>>()` + `is_sync` |
| `entity_counter.rs` | `entity_counter_reserve_distinct_ids` | 1000 calls yield 1000 unique Entity IDs |
| `entity_counter.rs` | `entity_counter_reserve_lock_free_8_threads` | 8 threads × 1000 reserves = 8000 unique |
| `entity_counter.rs` | `entity_counter_does_not_pop_free_list` | reserve after deallocate skips recycled ID (EM2) |
| `entity_counter.rs` | `entity_counter_no_other_em_field_reachable` | compile-fail trybuild: `counter.next_id_ptr.entities_inland` does not type-check (EM6 type-enforcement smoke) |
| `entity_commands.rs` | `entity_commands_size_is_16_bytes` | `mem::size_of::<EntityCommands<'_, '_>>() == 16` (O1) |
| `entity_commands.rs` | `id_returns_pre_allocated_entity` | spawn then `.id()` returns reserved Entity |
| `entity_commands.rs` | `chain_insert_returns_mut_self` | `.insert().insert().id()` compiles + applies in order |
| `entity_commands.rs` | `reborrow_does_not_consume` | reborrow yields handle with shorter lifetime |
| `entity_commands.rs` | `c1_compilable_sketch_compiles` | inline test mirroring §5.7 sketch |
| `commands.rs` | `commands_spawn_returns_entity_commands` | type check at compile time |
| `commands.rs` | `commands_entity_handle_for_arbitrary_id` | no validation; `commands.entity(stale)` constructs |
| `commands.rs` | `commands_despawn_convenience_wrapper` | equivalent to `commands.entity(e).despawn()` |
| `commands.rs` | `commands_struct_size_is_16_bytes` | `mem::size_of::<Commands<'_>>() == 16` (Round 3 — same size, different field) |
| `commands.rs` | `commands_does_not_expose_entity_master_ref` | trybuild fail: `commands.entity_master()` from outside crate does not type-check (Round 3 — accessor removed) |
| `spawn_at_command.rs` | `apply_creates_entity_at_reserved_id` | reserved entity becomes alive |
| `insert_command.rs` | `apply_migrates_to_target_archetype` | source row gone; target row populated |
| `insert_command.rs` | `apply_replaces_existing_component_bytes_in_place` | overlap case writes new bytes; changed_tick bumped; added_tick PRESERVED |
| `insert_command.rs` | `apply_replace_in_place_canonicalization_w_n1` | verify the W-N1 defensive debug_assert fires if get_or_create_archetype regression happens (mock injection) |
| `insert_command.rs` | `apply_on_stale_entity_debug_asserts_release_noops` | release-build: no panic, no state change |
| `remove_command.rs` | `apply_migrates_to_smaller_archetype` | entity moves from {A,B,C} to {A,B} when C removed |
| `remove_command.rs` | `apply_on_missing_component_is_idempotent_silent` | W1: NO debug_assert; just no-op |
| `despawn_command.rs` | `apply_calls_delete_entity` | entity no longer alive after apply |
| `archetype.rs` | `move_out_entity_skips_drop_for_all_pools_w_n2` | drop_fn NOT invoked for ANY slot (source or last) per W-N2 tightening |
| `archetype.rs` | `move_out_entity_swap_remove_updates_moved_entity` | RemoveOutcome::Swapped carries moved_entity correctly |
| `component_pool.rs` | `swap_remove_index_no_drop_uses_unit_pointers` | (C-N2 sanity) verify implementation uses `units[idx].ptr()` not stride arithmetic — inspection test asserting no `byte_ptr` symbol referenced |
| `component_pool.rs` | `swap_remove_index_no_drop_swaps_bytes_and_ticks` | byte swap (via Unit pointers) + 2 tick swaps; no drop_fn |
| `component_pool.rs` | `drop_at_invokes_drop_fn_only` | drop_fn runs; bytes-after-drop_at logically uninitialized but slot remains |
| `component_pool.rs` | `swap_remove_index_no_drop_does_not_drop_last_slot_w_n2` | regression test: last_row's drop_fn must NOT be invoked (W-N2) |

### 13.2 Integration tests

Unchanged additions vs Round 2 plus:

| File | Test name | Scenario |
|---|---|---|
| `tests/entity_commands_smoke.rs` | `c_n1_entity_counter_aliasing` | parallel spawn from 4 systems via worker threads; verify EntityCounter::reserve_entity is sound and no EntityMaster access leaks |

(Rest unchanged.)

### 13.3 Miri tests (Round 3 updates)

| Test | Scenario | UB check |
|---|---|---|
| `miri_entity_counter_no_ub` | 100 EntityCounter::reserve_entity calls (Round 3 rename) | atomic ordering |
| `miri_spawn_apply_no_ub` | Commands::spawn + apply cycle | no provenance violations |
| `miri_insert_migration_no_ub` | Migration end-to-end | no double-drop; Unit-pointer derefs are sound (C-N2) |
| `miri_remove_migration_no_ub` | Symmetric remove | no leak; drop_at runs once |
| `miri_despawn_no_ub` | Despawn apply | no UAF |
| `miri_chained_commands_no_ub` | spawn → insert → remove → despawn chain | no aliasing; no double-drop |
| `miri_stale_entity_noop_no_ub` | Stale entity passed to insert | silent no-op; no read-uninit |
| `miri_c_n1_commands_get_param_no_ub` | EntityCounter projection in Commands<'s>::get_param | no provenance violations on entity_counter pointer mint/deref (Round 3 rename) |
| `miri_w_n3_cross_frame_get_param_no_ub` | Run system over 2 frames; verify get_param re-mints EntityCounter and no stale pointer is dereferenced (W-N3) |
| `miri_c_n2_unit_pointer_migration_no_ub` | Migration with multi-chunk pool (force `source_row` to be in a non-first chunk) — verify `units[source_row].ptr()` access is sound |
| `miri_c5_move_out_entity_no_double_drop` | move_out_entity on archetype with custom Drop component | drop_fn NOT invoked on ANY slot (W-N2) |

### 13.4 Criterion benchmarks

| Bench | Target | Justification |
|---|---|---|
| `bench_commands_spawn_enqueue` | ≤ 30 ns single-thread | §1.2 + §10.1 |
| `bench_entity_commands_insert_enqueue` | ≤ 22 ns | §1.2 |
| `bench_entity_commands_despawn_enqueue` | ≤ 22 ns | §1.2 |
| `bench_spawn_at_command_apply_warm` | ≤ 500 ns | §1.2 |
| `bench_insert_command_apply_migration` | ≤ 720 ns | §10.2 Round 3 |
| `bench_insert_command_apply_replace_in_place` | ≤ 100 ns | §10.2 fast-path |
| `bench_remove_command_apply` | ≤ 720 ns | §10.2 Round 3 |
| `bench_despawn_command_apply` | ≤ 500 ns | §10.2 |
| `bench_10k_mixed_frame` | ≤ 5.5 ms | §10.3 |
| `bench_reserve_entity_parallel_4_threads` | ≤ 35 ns/op average | §10.5 |
| `bench_reserve_entity_parallel_8_threads` | ≤ 80 ns/op average | §10.5 saturation; O-N1 telemetry trigger >100 ns |
| `bench_chained_spawn_3_inserts` | ≤ 200 ns enqueue | §10.1 |
| `bench_migration_scratch_reuse` | ≤ 60 ns warm (no alloc) | W4 |
| `bench_migration_multichunk_source_row` | ≤ 750 ns | C-N2 — measures Unit-pointer cost in multi-chunk archetypes |

### 13.5 Property-based tests

Unchanged from Round 2.

### 13.6 Debug assertions (Round 3 updates)

| Site | Assertion |
|---|---|
| `EntityCommands::insert` | `B::component_ids().len() >= 1` |
| `SpawnAtCommand::apply` | `entities_inland[entity.id().0].is_null()` |
| `InsertCommand::apply` | `is_entity_valid(entity)` (debug_assert + early return) |
| `RemoveCommand::apply` (stale entity) | `is_entity_valid(entity)` |
| `RemoveCommand::apply` (absent component) | **NO debug_assert** (W1) |
| `DespawnCommand::apply` | `delete_entity` returns true |
| `EntityCounter::reserve_entity` | `id < usize::MAX / 2` |
| `apply_replace_in_place` (W-N1) | `archetype.component_pools().has_pool(component_id)` (defensive — should always pass under canonicalization) |
| `migrate_entity_insert` | `source.create_entity_with_ticks` push succeeds |
| `migrate_entity_insert` | `move_out_entity` returns Last or Swapped |
| `merged_archetype_id` (stack path) | merge count `<= 32` |
| `move_out_entity` precondition | (debug build only) source-row bytes were moved or dropped — documented contract only; not auto-verifiable |
| `swap_remove_index_no_drop` | `idx < self.units.len()` |
| `drop_at` | `idx < self.units.len()` |
| `pop_entity_no_drop` | `!self.units.is_empty()` |

### 13.7 Loom tests (O3)

| Test | Scenario | Interleaving coverage |
|---|---|---|
| `loom_parallel_reserve_no_collision` | 2 threads × 100 EntityCounter::reserve_entity with full interleaving | all returned IDs distinct under every thread schedule |
| `loom_reserve_vs_dispatcher_recycle` | 1 worker reserve + 1 dispatcher allocate (simulated via mutex around `allocate_entity`) | no collision |
| `loom_commands_get_param_aliasing` | 2 systems with Commands SystemParam built in parallel | no aliasing on EntityCounter pointer derefs |

File: `crates/boyko_ecs/tests/loom_phase11.rs` (new). Cargo feature `loom` gates compilation.

---

## §14 Step-by-Step Implementation Plan (updated)

### Wave A — EntityMaster (1-2 PRs)

1. **Step 1**: Convert `next_entity_id: EntityId` → `AtomicUsize`. Update `allocate_entity` to `fetch_add(Relaxed)` on fresh path. File: `entity_master.rs`. Verify `cargo check`.

2. **Step 2 (Round 3 — C-N1)**: Add `EntityMaster::next_id_atomic(&self) -> &AtomicUsize` crate-internal accessor. NO `pub fn reserve_entity` on EntityMaster (Round 3 change: reserve_entity migrates to EntityCounter newtype). File: same.

3. **Step 3 (W2)**: Restrict `allocate_entity` privacy `pub` → `pub(crate)`. Audit in-crate callers via grep. Add trybuild test for external visibility. File: same.

4. **Step 4**: Update Send/Sync SAFETY block. File: same.

### Wave B — EntityCounter newtype + Commands wiring (1-2 PRs)

5. **Step 5 (Round 3 — C-N1)**: Create `crates/boyko_ecs/src/ecs/core/system/params/entity_counter.rs`. Define `EntityCounter<'s>` per §5.5. Implement `reserve_entity`, `from_ptr` (crate-only), `Send`/`Sync` impls, `Clone`/`Copy` derives. Add size assertion test (`mem::size_of::<EntityCounter<'_>>() == 8`). Add trybuild test asserting EntityCounter only exposes `reserve_entity` (no field access). Files: new + edit `params/mod.rs` to register.

6. **Step 6 (Round 3 — C-N1)**: Modify `Commands<'s>` to carry `entity_counter: EntityCounter<'s>` (replaces the Round 2 `*const EntityMaster` + PhantomData). Update `SystemParam::get_param` per §5.6. Add `UnsafeEcsCell::entity_counter<'s>` helper per §12.7. REMOVE the Round-2-planned `Commands::entity_master()` accessor — no public path to `&EntityMaster` from Commands. Files: `system/params/commands.rs`, `system/unsafe_ecs_cell.rs`. Existing tests pass.

### Wave C — EntityCommands struct (2-3 PRs)

7. **Step 7 (C1)**: Run §5.7 compilable sketch as standalone `tests/c1_lifetime_sketch.rs`. Verify `cargo check`. Create `crates/boyko_ecs/src/ecs/core/commands/entity_commands.rs`. Define `EntityCommands<'a, 's>`, `id()`, `reborrow()`. Add 16-byte size assertion. Add Send/Sync compile_fail tests.

8. **Step 8**: Wire `Commands::entity` + `Commands::spawn` return-type change. File: `system/params/commands.rs`.

9. **Step 9**: Create `commands/spawn_at_command.rs`. Define `SpawnAtCommand<B>` (Q9 — replaces SpawnCommand). Migrate apply logic from Phase 8.5 `SpawnCommand`. Delete `spawn_command.rs`. Wire `Commands::spawn`.

10. **Step 10**: Add `EcsMaster::create_entity_at` + `EcsMaster::migration_scratch_mut` + `migration_scratch` field. File: `ecs_master.rs`. Unit tests.

### Wave D — DespawnCommand (1 PR)

11. **Step 11**: Create `commands/despawn_command.rs`. Wire `EntityCommands::despawn`, `Commands::despawn`. Tests.

### Wave E — Migration scaffold (3 PRs) — Round 3 C-N2 rewrite

12. **Step 12 (C5 + C-N2 + W-N2)**: Add `ComponentPool::swap_remove_index_no_drop` per §7.2 — **uses `units[idx].ptr()` + `copy_nonoverlapping` + `*added_ticks[idx].get()`** mirroring the existing `swap_remove` flow (NOT flat-byte stride). Add `pop_entity_no_drop`, `drop_at`, `write_at`, `read_added_tick`, `read_changed_tick`, `write_changed_tick`. Add `ComponentPoolBundle::swap_remove_unit_no_drop`, `pop_entity_no_drop`, `has_pool` (W-N1). Tests for each. Files: `component_pool.rs`, `component_pool_bundle.rs`.

13. **Step 13 (C5, O2)**: Add `Archetype::move_out_entity` (renamed from `forget_entity`). Tests. File: `archetype.rs`.

14. **Step 14**: Add `Archetype::create_entity_with_ticks` (§9.2 + W3 doc). Tests for tick preservation. File: `archetype.rs`.

### Wave F — InsertCommand + RemoveCommand (3 PRs)

15. **Step 15 (C3 + C-N2 + W-N1)**: Create `commands/insert_command.rs`. Define `InsertCommand<B>`. Wire `EntityCommands::insert`. Implement `apply_replace_in_place` (§7.4 linear sequence + W-N1 defensive debug_assert + canonicalization SAFETY citation). Implement `migrate_entity_insert` (§7.2 with `move_out_entity` + Unit-pointer retained extraction per C-N2). Unit + integration tests.

16. **Step 16 (W1 + C-N2)**: Create `commands/remove_command.rs`. Define `RemoveCommand<C>`. Implement absent-C silent no-op (W1). Implement `migrate_entity_remove` (§7.3 with explicit `drop_at` before `move_out_entity` + Unit-pointer extraction).

17. **Step 17 (W4)**: Add `merged_archetype_id` + `without_component_archetype_id` helpers using `migration_scratch`. File: `commands/migration_helpers.rs`.

### Wave G — try_* polish (1 PR)

18. **Step 18 (O4)**: Add `try_insert`, `try_remove`, `try_despawn` aliases with TODO comments.

### Wave H — Test sweep + benches (2-3 PRs)

19. **Step 19**: Write all unit + integration tests from §13.1, §13.2 — including the new Round 3 Miri tests (C-N1 trybuild, C-N2 multi-chunk Miri, W-N1 canonicalization regression mock, W-N2 last-slot drop regression, W-N3 cross-frame).

20. **Step 20**: Write Miri tests from §13.3.

21. **Step 21 (O3)**: Write loom tests from §13.7.

22. **Step 22 (W5 + Round 3)**: Write criterion benches from §13.4. Validate against targets, especially `bench_reserve_entity_parallel_*_threads` and `bench_migration_multichunk_source_row` (new in Round 3).

### Wave I — Migration to existing tests (1 PR)

23. **Step 23**: Update any Phase 8d / 8.5 tests that depend on the old API. Grep confirms no callsite-incompatible changes.

### Wave J — Documentation + book chapter (1 PR)

24. **Step 24**: Update `docs/FEATURE_MAP.md`, `docs/SYSTEMS.md`. Public mdBook chapter — `doc-writer` agent.

Total: 24 steps, ~12-14 PRs. Estimated 2.5-3 weeks of focused work (Round 3 adds Step 5 EntityCounter standalone + Step 6 Commands wiring split, vs Round 2's single Step 5).

---

## §15 Migration Impact

### 15.1 Public API breaking changes

- **`Commands::spawn<B>` return type**: `()` → `EntityCommands<'_, '_>`. Callsite-compatible (no-op Drop).
- **`EntityMaster::allocate_entity` privacy** (W2): `pub` → `pub(crate)`.
- **Round 3 (C-N1)**: NO new public `EntityMaster::reserve_entity` — that API never existed publicly. Round 2 planned it; Round 3 routes through `EntityCounter::reserve_entity` instead.

### 15.2 Internal API changes (Round 3 updates)

- `SpawnCommand<B>` deleted; `SpawnAtCommand<B>` replaces.
- `EntityMaster::next_entity_id`: `EntityId` → `AtomicUsize`.
- `EntityMaster::next_id_atomic(&self) -> &AtomicUsize`: NEW crate-internal accessor.
- New `EntityCounter<'s>` newtype + module `system/params/entity_counter.rs`.
- New `EcsMaster::create_entity_at`, `EcsMaster::migration_scratch_mut`.
- New `Archetype::create_entity_with_ticks`, `Archetype::move_out_entity`.
- New `ComponentPool` no-drop APIs operating on `units[idx].ptr()` + `added_ticks[idx]` / `changed_ticks[idx]` (Round 3 — existing chunked + Unit-pointer storage).
- `Commands<'s>` field change: Round 2 planned `entity_master_ptr: *const EntityMaster`; Round 3 ships `entity_counter: EntityCounter<'s>`.
- `UnsafeEcsCell::entity_counter<'s>(self) -> EntityCounter<'s>`: NEW (Round 3 — replaces Round 2's `entity_master_ptr`).

### 15.3 Test migration

Grep confirms no existing tests rely on `Commands::spawn` returning `()`.

### 15.4 Phase 8.5 cache invalidation

Unaffected — `bundle_archetype_cache` reused for `SpawnAtCommand`.

### 15.5 Phase 9 invariant preservation

- SEND5: tightened by EM1-EM6.
- SEND6: unchanged.
- ALLOC1-6: `create_entity_at` inherits dispatcher-only contract.
- SCH7: unchanged.
- **NEW EM6 (Round 3)**: EntityCounter field-restriction invariant — workers cannot reach non-atomic EntityMaster state.

### 15.6 Phase 10 invariant preservation

Unchanged from Round 2.

---

## §16 Rejected Alternatives

(§16.1-§16.10 from Round 1 + §16.11 from Round 2 unchanged. §16.12 added below.)

### 16.12 Round 2 raw-pointer `*const EntityMaster` in Commands (Round 3 C-N1)

**Rejected (C-N1)**: Round 2 stored `entity_master_ptr: *const EntityMaster + PhantomData<&'s EntityMaster>` in `Commands<'s>` and exposed a `pub(crate) fn entity_master(&self) -> &EntityMaster` accessor that returned a shared reference to the full master. This works under the documented SystemParam contract but the EM6 aliasing rule ("only the atomic counter field is touchable via this channel") is encoded only as prose — a future maintainer adding `Commands::reserve_n` that reads `entities_inland.len()` would silently violate it without compiler help.

Round 3 adopts the `EntityCounter<'s>` newtype that carries `*const AtomicUsize` (not `*const EntityMaster`). The type system now enforces the aliasing rule: the pointer's destination type is `AtomicUsize`, not `EntityMaster`, so projecting to any other EntityMaster field is a compile error. This is strictly better than the Round 2 prose contract.

Cost: one additional struct definition (~30 lines), one extra `UnsafeEcsCell` projection method (~10 lines), one extra Wave B implementation step (Step 5 vs Step 5/6 split). Worth it for the type-enforced soundness boundary.

### 16.13 Round 2 flat-byte ComponentPool storage (Round 3 C-N2)

**Rejected (C-N2)**: Round 2's `swap_remove_index_no_drop` sketch used `self.byte_ptr().add(idx * stride)` as if ComponentPool were a flat `Vec<u8>`. The actual storage (verified at `crates/boyko_ecs/src/ecs/memory/component_pool.rs:22-88, 339-424`) is **chunked**: `chunks: Vec<Chunk>` + `units: Vec<Unit>` where each `Unit` holds an absolute `*mut u8` (possibly into different chunks for large pools).

Round 3 adopts the existing storage model unchanged (architect Q2 option b): `swap_remove_index_no_drop` uses `units[last].ptr()` + `units[idx].ptr()` + `copy_nonoverlapping(layout.size())`, mirroring the existing `swap_remove` flow (`component_pool.rs:339`) but skipping drop. Retained-byte extraction in `migrate_entity_insert` / `migrate_entity_remove` uses `from_raw_parts(units[row].ptr(), layout.size())` to get a `&[u8]` slice over the source row's bytes.

Alternative (rejected): flatten ComponentPool to single contiguous `Vec<u8>` per pool. This would enable stride arithmetic but breaks the existing chunked storage that supports arena-allocated multi-chunk pools (large components). Out of scope for Phase 11 — would be its own multi-week refactor.

Cost: +20 ns per migration warm path (Unit-pointer dereference cost) — absorbed within the 720 ns target (was 700 ns). Bench `bench_migration_multichunk_source_row` validates.

---

## §17 Open Questions

### OQ1: EntityCommands::insert semantics for Bundle::component_ids overlap

Closed (resolved by B1 inheritance).

### OQ2: EntityCommands::despawn followed by .id()

Closed (documented behavior).

### OQ3: Intermediate archetype problem

Phase 13 follow-up.

### OQ4: Reservation-reaper for leaked IDs

Phase 12 follow-up.

### OQ5: Added<T> over-reports replaced components

**RESOLVED (Round 2 decision recorded in §9.5)**: ship v1 with over-reporting documented as known limitation. Phase 12 fix via per-component `is_new` flag.

### OQ6: try_* output-slot wiring

Phase 12.

### OQ7: Cache-line false sharing on next_entity_id

Phase 12 (telemetry-driven). §10.5 cost model in v1.

### OQ8: Bundle parameter for EntityCommands::remove

Phase 12.

### OQ9: Loom test scheduling budget

Loom's exhaustive interleaving budget for `loom_parallel_reserve_no_collision` is bounded at 2 threads × 100 ops. If insufficient, Phase 12 may need coarser-grained tests (e.g., via `shuttle`). Decision deferred.

### OQ10 (NEW, Round 3): ComponentPool storage flatten

Out of scope for Phase 11. Round 3 keeps the existing chunked + Unit-pointer storage (architect Q2 option b). A future phase may flatten ComponentPool to a single contiguous `Vec<u8>` per pool to enable stride arithmetic and reduce indirection cost — would shave ~10 ns per migration row but requires reworking the arena allocator. Decision deferred until profiling shows Unit-pointer indirection as a hotspot.

---

## §18 Plan-Readiness Checklist (Round 3 updates)

### Plan structure
- [x] Goal stated in terms of perf + functionality (§1.1)
- [x] Target metrics concrete (§1.2 — Round 3 ≤ 720 ns migration)
- [x] Every decision justified via perf/cache/parallelism (§3 + §4-9)
- [x] Alternatives rejected with reasoning (§16, including new §16.12, §16.13)
- [x] Trade-offs honestly listed

### Data structures
- [x] Field types + access role comments (§11 — Round 3 EntityCounter layout §11.10)
- [x] `#[repr(C)]` where layout matters
- [x] Hot/cold split (entity hot, marker cold)
- [x] Sizes known + justified (O1 + EntityCounter 8 B + Commands 16 B re-verified)
- [x] False sharing analysis (§11.8 + §10.5)

### API
- [x] Public API minimal (§12 — Round 3 EntityCounter surface)
- [x] No leaked internal types (Round 3: `Commands::entity_master()` accessor REMOVED)
- [x] Lifetimes explicit (two lifetimes on EntityCommands per C1; `'s` on EntityCounter)
- [x] No `dyn Trait` in hot path
- [x] Generics where needed
- [x] C1 compilable sketch verified (§5.7 — Round 3 includes EntityCounter)

### Multithreading
- [x] Model explicit: workers `&self` via EntityCounter, dispatcher `&mut self` (§8)
- [x] Atomic ordering specified (§4.7, §11.8)
- [x] Sync points justified (apply-window SCH7 reuse)
- [x] Partitioning described (per-system CommandQueue, single-writer)
- [x] Send/Sync consistent (EC1, EntityCounter Send + Sync explicit impls)
- [x] C4 four-case proof (§4.6)
- [x] W5 contention model (§10.5)
- [x] **NEW EM6 invariant** (Round 3 C-N1) — field-restriction type-enforced

### Correctness
- [x] Edge cases enumerated (stale entity, spawn-then-despawn, absent-C, multi-chunk source row)
- [x] Generation check described (Q8, C4)
- [x] Drop order discussed (§7.2 move_out_entity W-N2 tightening)
- [x] Unsafe invariants stated (every SAFETY block planned, including W-N1 canonicalization citation in §7.4)
- [x] **NEW W-N1** canonicalization invariant cited from `archetype_master.rs:99-133, 462-473`

### Integration
- [x] Affected modules listed (§14 per-step file paths — Round 3 Wave B split)
- [x] Existing API changes noted (§15)
- [x] Phase 8.5 Bundle compatibility verified (§15.4)
- [x] Phase 9 SEND/SCH/ALLOC verified (§15.5 — EM6 added)
- [x] Phase 10 CD/STORE preserved (§15.6, OQ5 decision recorded)
- [x] Implementation plan stepwise (§14, 24 steps — Round 3 +1 step)

### Validation
- [x] Unit tests specified (§13.1 — Round 3 EntityCounter + W-N1/W-N2 tests added)
- [x] Integration tests specified (§13.2)
- [x] Property tests specified (§13.5)
- [x] Benchmarks specified with targets (§13.4 — Round 3 multichunk bench added)
- [x] debug_assert! sites listed (§13.6 — W-N1 defensive check added)
- [x] Miri tests specified (§13.3 — Round 3 C-N1, W-N3, C-N2 multichunk added)
- [x] Loom tests specified (§13.7, O3)

---

**End of Phase 11 Plan, Round 3.**

All 2 Round 2 criticals resolved with concrete code/proof:
- **C-N1**: `EntityCounter<'s>` newtype encapsulates `*const AtomicUsize` (not `*const EntityMaster`) — EM6 aliasing rule type-enforced. §5.5, §11.10, §12.1, §12.7.
- **C-N2**: §7.2 / §7.4 / §12.6 rewritten against existing chunked + Unit-pointer storage (`units[idx].ptr()` + `copy_nonoverlapping` + `*added_ticks[idx].get()`). No ComponentPool storage rework. Cost: +20 ns warm migration vs Round 2; absorbed in §10.3 budget.

All 3 Round 2 warnings resolved:
- **W-N1**: §7.4 SAFETY block cites canonicalization invariant from `archetype_master.rs:99-133, 462-473` (`ComponentMask::from_components` + `find_exact_match`); defensive `has_pool` debug_assert added.
- **W-N2**: §7.2 contract tightened to "MUST NOT invoke `drop_fn` on ANY slot (source or last)".
- **W-N3**: §8.7 new sub-section makes per-invocation `get_param` lifecycle explicit, citing Phase 8c IntoSystem contract. Cross-frame staleness impossible.

Round 2 optional polished:
- **O-N1**: §10.5 final paragraph references `bench_reserve_entity_parallel_8_threads` as the >100 ns/op telemetry trigger more tightly.

Round 4 expected to be polish only / APPROVED.

---

Relevant file paths (absolute):
- `D:\claude\BoykoEngine\docs\PHASE-11-ENTITY-COMMANDS-PLAN.md` — plan v2 (to be replaced with this Round 3 content)
- `D:\claude\BoykoEngine\docs\PHASE-11-CRITIC-ROUND-2.md` — critic input addressed
- `D:\claude\BoykoEngine\docs\PHASE-11-CRITIC-ROUND-1.md` — prior critic input (Round 1)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\entity\entity_master.rs` — EntityMaster (Wave A)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\commands.rs` — Commands<'s> (Wave B Step 6)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\entity_counter.rs` — NEW: EntityCounter<'s> newtype (Wave B Step 5)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\unsafe_ecs_cell.rs` — entity_counter<'s> projection
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` — create_entity_at, migration_scratch
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs` — create_entity_with_ticks, move_out_entity
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype_master.rs` — get_or_create_archetype canonicalization (cited in §7.4 W-N1)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs` — drop_at, swap_remove_index_no_drop (Unit-pointer model), write_at, pop_entity_no_drop, read/write tick accessors
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\bundle.rs` — Bundle trait (reused unchanged)
- `D:\claude\BoykoEngine\docs\PHASE-9-PARALLEL-SCHEDULER-PLAN.md` — Phase 9 invariants reused
- `D:\claude\BoykoEngine\docs\PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md` — Phase 8.5 bundle cache reused

Sources:
- [Bevy EntityCommands docs (single-'a, modern API)](https://docs.rs/bevy/latest/bevy/ecs/system/struct.EntityCommands.html)
- [Bevy Commands docs](https://docs.rs/bevy/latest/bevy/ecs/system/struct.Commands.html)
- [Bevy PR #15523 — revert consume-self despawn (Q3 lesson)](https://github.com/bevyengine/bevy/pull/15523)
- [Bevy Issue #10166 — silent no-op for stale entity commands (W1 reference)](https://github.com/bevyengine/bevy/issues/10166)
- [Bevy Issue #5074 — intermediate archetype problem (Q5 reference)](https://github.com/bevyengine/bevy/issues/5074)