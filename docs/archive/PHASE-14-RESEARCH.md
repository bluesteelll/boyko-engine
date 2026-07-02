> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 14 Research — Observers / Component Lifecycle Hooks

Researcher's input to the architect. Sources: Bevy `archetype.rs` /
`lifecycle.rs` / `deferred_world.rs` (main + 0.14/0.16), flecs
`observable.h` / Observers Manual, Unity DOTS docs, and boyko's
`core/` tree. URLs + file:line in §Sources.

## TL;DR — what the architect must know

- **The zero-cost mechanism is proven, and boyko is missing the one
  piece it needs.** Bevy's trick: a `u32` `ArchetypeFlags` field on each
  `Archetype`, set once at archetype creation by OR-ing each component's
  hook flags. The hot-path dispatch site is `if archetype.has_add_hook()
  { ... }` — a single bit test that branch-predicts to "not taken" when
  nothing is hooked. **boyko's `Archetype` has NO flags field today.**
  Adding one ~4 B field + a one-time compute at archetype construction is
  the core of Phase 14. flecs does the identical thing with
  `EcsTableHasOnAdd = (1u<<16)` / `EcsTableHasOnRemove = (1u<<17)`.
- **The reentrancy/aliasing hazard is solved by forbidding structural
  changes inside a hook.** Bevy hooks receive a `DeferredWorld` (permits
  component/resource *mutation*, NOT add/remove/despawn). Structural
  changes a hook wants route through `Commands`, deferred to flush. flecs
  uses a defer counter (`ecs_defer_begin`/`end`). This sidesteps the
  structural-change-during-structural-change aliasing UB entirely.
- **Hook storage mirrors boyko's existing `drop_fn` precedent exactly.**
  `ComponentLayout` already stores `drop_fn: Option<DropFn>` per
  `ComponentId`. The hook table is the same pattern: more `Option<HookFn>`
  slots, `None` by default → zero cost when unused; "is this component
  hooked?" = `Option::is_some()`.
- **Scope recommendation: ship hooks-only (14a). Defer full Observers
  (14b) indefinitely.** Bevy Observers are a *second* system on top of
  hooks (entity-targeted, runtime add/remove, a `CachedObservers` index +
  event routing boyko lacks). Hooks alone satisfy the roadmap's
  "spawn/despawn/insert/remove callbacks" goal at a fraction of the risk.
  boyko's `EventDispatcher` is **not** a suitable observer backbone (§6).
- **Structural mismatch the architect must confront:** boyko has **no
  `World`/`DeferredWorld` split** — `Command::apply` takes raw
  `&mut EcsMaster`. A hook receiving `&mut EcsMaster` could call
  `world.create_entity(...)` mid-insert → exactly the reentrancy UB Bevy's
  `DeferredWorld` prevents. Decision #1: build a `DeferredEcsMaster`
  restricted view, or a `Commands`-only hook context.

## 1. Bevy `ComponentHooks` — API, signature, the zero-cost trick

### Hook signature + context (`lifecycle.rs`, main/0.16)

```rust
pub type ComponentHook = for<'w> fn(DeferredWorld<'w>, HookContext);
pub struct HookContext { pub entity: Entity, pub component_id: ComponentId, /* + caller, relationship_hook_mode */ }
pub struct ComponentHooks {
    on_add: Option<ComponentHook>, on_insert: Option<ComponentHook>,
    on_discard /*replace*/: Option<ComponentHook>, on_remove: Option<ComponentHook>,
    on_despawn: Option<ComponentHook>,
}
```

- Hook is a **plain `fn` pointer** — zero-alloc, inline. Receives a
  `DeferredWorld` (mutate components/resources, queue `Commands`; NO
  structural change) + `HookContext`.
- Hooks fire **immediately** during the structural op (synchronous); the
  *reactions* they queue via `Commands` are deferred.
- Ordering: `on_add` before `on_insert` (add only when genuinely new);
  `on_insert` also on replace; `on_replace`/`on_remove` on removal/despawn.

### Registration

1. Via the `Component` trait — `register_component_hooks(&mut ComponentHooks)`
   (defaulted empty; derive can generate). 2. Runtime:
   `World::register_component_hooks::<C>().on_add(...)`. **Constraint:**
   hooks cannot be modified after the component exists in any archetype;
   each kind registered once.

### The `ArchetypeFlags` zero-cost trick (THE key finding)

`archetype.rs`:
```rust
struct ArchetypeFlags: u32 {
    ON_ADD_HOOK=1<<0; ON_INSERT_HOOK=1<<1; ON_DISCARD_HOOK=1<<2;
    ON_REMOVE_HOOK=1<<3; ON_DESPAWN_HOOK=1<<4;
    ON_ADD_OBSERVER=1<<5; /* ... observer bits 1<<6..1<<9 */
}
// one `flags: ArchetypeFlags` field on Archetype
```
Computed once at archetype creation by OR-ing each component's flags
(`ComponentInfo::update_archetype_flags` sets a bit iff
`hooks().on_X.is_some()`). Dispatch site (`deferred_world.rs`,
`trigger_on_add` etc.) is gated by a single `if archetype.has_add_hook()`
(= `flags.contains(ON_ADD_HOOK)`, one AND+compare) at the top; the
per-component `Option` loop is entirely skipped in the no-hook case.

Spawn call site (`bundle.rs`, `BundleSpawner::spawn_non_existent`) calls
`trigger_on_add`/`trigger_on_insert` unconditionally (the cheap flag check
is *inside*); only the separate *observer* dispatch gets an explicit outer
`if archetype.has_add_observer()` guard.

**No-hook branch cost:** one `u32` load + one `test`/`jz`, statically
predicted not-taken. Exactly the roadmap's `if const { HAS_HOOKS }` +
`#[cold]` discipline.

## 2. flecs observers

- Triggers: `OnAdd`, `OnRemove`, `OnSet`, `Monitor`. "Fires only when the
  component is *actually* added/removed."
- Observer = query + callback; multi-term decomposed into single-term.
- **Zero-cost mechanism (flecs equivalent of ArchetypeFlags):** per-table
  flags `EcsTableHasOnAdd=(1u<<16)` / `EcsTableHasOnRemove=(1u<<17)` so
  `flecs_emit` short-circuits when a table has no interested observers;
  per-id `observer_count` + `flecs_observers_exist(...) -> bool`.
- **Deferred/immediate:** explicit defer counter
  (`ecs_defer_begin`/`end`); ops inside route to a command queue, merged
  when the counter hits 0. `emit` (immediate) vs `enqueue` (queue if
  deferred). Nested defer uses dual command stacks → add/remove issued
  *inside* an observer queue and apply at outer-scope close. Caveat:
  OnAdd/OnRemove may fire in batched (non-emit) order.

## 3. Bevy Observers (0.14+) — brief + scope recommendation

- An `Observer` is itself a *component* on an entity holding a system run
  on a matching event (`On<E>`/`Trigger<E>` param); `World::add_observer`.
- Hooks vs Observers: hooks are innate to the component type, one per
  (component, kind), per-component-type, the 5 lifecycle events,
  deterministic, `ArchetypeFlags` bit test. Observers are separate
  entities, runtime add/remove, many-per-event, entity-targeted + global,
  lifecycle **+ arbitrary custom events**, less deterministic, need a
  `CachedObservers` reverse index.
- **→ ship 14a hooks-only.** Hooks cover the roadmap goal; need only
  `ArchetypeFlags` + `Option<HookFn>` (mirror `drop_fn`). Observers need an
  entity-as-observer machinery + per-event/target reverse index + event
  routing boyko lacks, and layer on top of hooks (hooks are a strict
  prerequisite). Defer 14b until a concrete need (e.g. Phase 19 hierarchy
  auto-despawn) forces it.

## 4. Unity DOTS — brief

No per-structural-op lifecycle callback. Structural changes batched at
sync points via `EntityCommandBuffer`; reaction expressed via *queries*
with change filters (Burst/job-parallel-safe), not callbacks. **Takeaway:**
DOTS validates "callbacks are dangerous on the hot/parallel path" and
leans on change-detection — which boyko already has (Phase 10
`Added<T>`/`Changed<T>`). Some "on_add" needs are already served by
`Added<T>` at zero structural-op cost; the architect should scope which
needs genuinely require a synchronous callback.

## 5. boyko integration points (file:line)

- **`Component` trait** (`component.rs:32-59`): only `component_id()`
  required + defaulted helpers. Where `const HAS_HOOKS: bool = false`
  (compile-time elision) and/or defaulted `fn register_hooks(&mut ComponentHooks)`
  would go. Both backward-compatible widenings.
- **`component_registry`** (`component_registry.rs:67,91-125,141-226`):
  `ComponentLayout` `#[repr(C)]` 56 B (one cache line):
  `{ size, alignment, drop_fn: Option<DropFn>, type_name, type_id }`.
  `DropFn = unsafe fn(*mut u8)` (line 67) — **the fn-ptr-table precedent**.
  Hook table mirrors it: either inline (pushes past 64 B — spills the
  line) or a **parallel cold table** `static HOOKS: [OnceLock<ComponentHooks>;
  MAX_COMPONENTS]` (keeps `ComponentLayout` 56 B; recommended). `OnceLock`
  + collision-detection idiom at lines 141-226. `MAX_COMPONENTS = 512`.
- **Archetype** (`archetype.rs:109-170,203-310,480-507`): `#[repr(C)]`,
  `columns: [Column; MAX_COMPONENTS]` pinned at offset 0 (asserted lines
  165-170), then `id, component_pools, current_index, signature, arena,
  component_ids, entity_ids`. **NO flags field — the gap.** Add
  `flags: ArchetypeFlags`; compute once in `create_by_ids` (203-232) /
  `register_component_inplace` (281-284) where the `component_ids` loop
  already runs. Place `flags` adjacent to `signature`/`id` (NOT disturbing
  the offset-0 columns invariant).
- **Structural-op hot paths** (where dispatch slots in):
  | Lifecycle | Function | File:line | Guard site |
  |---|---|---|---|
  | spawn (deferred) | `SpawnAtCommand::apply` | `spawn_at_command.rs:106-248` | after write loop (~240): on_add+on_insert all |
  | spawn (direct) | `EcsMaster::create_entity[_at]` | `ecs_master.rs:534-700` | after `register_entity_with_ptr` (599/697) |
  | insert (in-place) | `InsertCommand::apply_replace_in_place` | `insert_command.rs:95-173` | on_replace before `drop_at` (165) + on_insert after `write_at` |
  | insert (migration) | `migrate_entity_insert` | `migration_helpers.rs` + `insert_command.rs:74-82` | after migration: on_add + on_insert |
  | remove | `RemoveCommand::apply`→`migrate_entity_remove` | `remove_command.rs:61-99` | on_replace+on_remove **before** row moved out |
  | despawn | `DespawnCommand::apply`→`delete_entity` | `despawn_command.rs:31-43`→`ecs_master.rs:886-926` | on_replace+on_remove all, **before** `remove_entity` (906) |
  Removal/despawn hooks must fire **before** bytes are dropped/moved (so
  the hook can still read the dying value) — Bevy fires on_replace/on_remove
  pre-drop.
- **CommandQueue / apply window** (`command_queue.rs:63-120`,
  `command.rs:53-56,111-167`): packed byte-arena; `apply` runs each command
  under exclusive `&mut EcsMaster` with hoisted `catch_unwind` + `CursorSync`
  RAII. **CQ7/APP4 (command.rs:42-46): `apply` forbids re-entry into
  `run_system_once`/`run_closure_once`.** A hook wanting to spawn/insert
  during apply must **enqueue into the same CommandQueue**, not call a
  structural method directly. The cursor-advance + `bytes.len()` re-read
  governs whether hook-issued commands drain in the same pass.
- **EventDispatcher** (`event_dispatcher.rs:34-90`): type-erased per-event
  broadcast lanes, frame-deferred/pull-based, no per-entity routing. **NOT
  an observer backbone** (§6) — observer dispatch must be the synchronous
  `ArchetypeFlags`-gated path.
- **`#[derive(Component)]`** (`boyko_macros/src/lib.rs:31-84`): emits the
  `OnceLock`-backed `component_id()`; a `#[component(on_add=...)]` attribute
  + `register_hooks` codegen attaches to that lazy-init.

## 6. EventDispatcher cannot back observers — why

Broadcast (type-keyed lanes, no per-`Entity` routing), frame-deferred
(readers drain next frame) vs hooks must fire synchronously during the op
for correct ordering/state, and no `(ComponentId, kind) → handlers` reverse
index. The `ArchetypeFlags` + `Option<HookFn>` design is a different,
purpose-built structure. Do not couple 14a to the event system.

## 7. Recommended design sketch + bounded 14a scope (option space)

### Zero-cost mechanism (Q1) — recommend BOTH layers
- **Runtime (primary):** `flags: ArchetypeFlags` on `Archetype`, OR'd at
  construction; dispatch = `if archetype.flags.contains(ON_ADD_HOOK)`.
  Works even when the component type isn't statically known (boyko's
  type-erased `Command::apply` situation). The load-bearing mechanism.
- **Compile-time (secondary):** `const HAS_HOOKS: bool` on `Component`
  enables `if const { C::HAS_HOOKS }` in *monomorphic* typed paths only.
  Most boyko ops go through type-erased apply, so the const layer helps
  only direct typed APIs; the runtime flag always applies.

### Hook storage + signature (Q2)
- Storage: parallel cold `static HOOKS: [OnceLock<ComponentHooks>; MAX_COMPONENTS]`
  (keeps `ComponentLayout` 56 B; mirrors `LAYOUTS`). `register_hooks::<C>()`
  populates during registration.
- `HookFn` signature options:
  - (a) `unsafe fn(DeferredEcsMaster<'_>, HookContext)` — a newtype over
    `&mut EcsMaster` that **withholds** structural-change methods (exposes
    component/resource mutation + a `Commands` handle). Bevy-faithful, sound.
  - (b) `unsafe fn(&mut CommandQueue, HookContext)` — hook can only enqueue
    deferred commands; simplest to make sound, least powerful.
  - (c) `unsafe fn(&mut EcsMaster, HookContext)` — **REJECTED**: reintroduces
    reentrancy UB (hook calling `create_entity` mid-insert aliases the
    `&mut Archetype`/`&mut ComponentPool` the apply loop is writing).
  - `HookContext = { entity, component_id }` (omit Bevy's `MaybeLocation`/
    `RelationshipHookMode` — boyko has no such machinery).

### Reentrancy / deferred-dispatch (Q3 — the soundness crux)
Hazard: a hook firing mid-`SpawnAtCommand::apply` that spawns/inserts
aliases the `&mut Archetype`/`&mut ComponentPool` the apply loop holds, and
could reallocate `entities_inland` while a raw `archetype_ptr` is held →
UB. **Proven answer (Bevy + flecs): structural changes from a hook are
forbidden in-place and deferred.** boyko: hook gets a `Commands`/
`CommandQueue` handle; spawn/insert/remove/despawn it requests is
**enqueued**, not executed inline. The architect must define whether
hook-enqueued commands drain in the **same** apply pass (Bevy front-jump)
or the next, and ensure cursor/`bytes.len()` re-read makes appended
commands visible. Component *mutation* by a hook is sound if scoped away
from what the apply path is concurrently writing — the `DeferredEcsMaster`
view must enforce that.

### Registration API (Q5)
- Primary: `#[derive(Component)]` attribute `#[component(on_add=path, on_remove=path, ...)]`
  → generates `const HAS_HOOKS = true` + a `register_hooks` impl installing
  fn-pointers into `HOOKS` on first `component_id()` (the macro already
  emits `OnceLock`-backed `component_id()`).
- Secondary: `EcsMaster::register_component_hooks::<C>().on_add(...)` with
  the "register before the component appears in any archetype" constraint
  (so `ArchetypeFlags` are correct).

### Bounded 14a deliverable
1. `ArchetypeFlags` (u32) on `Archetype` + one-time compute.
2. `ComponentHooks` cold table in `component_registry` + `register_hooks::<C>()`.
3. `Component::HAS_HOOKS` const + defaulted `register_hooks`.
4. A `DeferredEcsMaster` (or `Commands`-only) hook context forbidding
   structural change.
5. `#[cold] #[inline(never)]` dispatch fns gated by `if archetype.flags.contains(...)`,
   wired into the 6 structural-op sites.
6. Derive attribute for `on_add/on_insert/on_replace/on_remove`.
7. Bench gate: spawn_batch / query iter / insert / despawn show **0%
   measurable regression** with no hooks (mirror Phase 10's "0% when unused").

### Soundness hazards
- Reentrancy → defer all structural change from hooks (the (c) signature is
  the trap).
- Drop-order: fire on_replace/on_remove **before** `drop_at`/`remove_entity`.
- `ArchetypeFlags` staleness: if a hook registers after an archetype exists,
  its flags are wrong → silently skipped. Adopt "register before first use".
- Parallel scheduler: hooks fire only in the single-threaded apply window
  (CQ7/APP4) — never reachable from parallel `&self` query paths.
- `Added<T>` overlap: some on_add needs are served by Phase 10 `Added<T>` at
  zero structural-op cost (the DOTS lesson).

## 8. Risks / open questions the architect MUST decide

1. **DeferredWorld-equivalent or Commands-only?** The #1 decision. A
   `DeferredEcsMaster` (mutate-not-restructure) is Bevy-faithful, more
   powerful, but must statically withhold `create_entity`/`delete_entity`/
   insert/remove from `EcsMaster`. `Commands`-only is trivially sound, less
   ergonomic. boyko has no `World` type — design from scratch.
2. **Immediate vs deferred hook firing.** Recommend immediate *callback*
   (synchronous, for correct ordering/state) + deferred *reactions* (Commands).
3. **Hook-issued commands: same apply pass or next?** Define
   `CommandQueue::apply` behavior for commands appended mid-drain.
4. **Where do `ArchetypeFlags` live in the layout?** Not disturbing
   `offset_of!(Archetype, columns)==0`; co-locate with `signature`/`id`.
5. **Inline hook table (breaks 64 B) vs parallel cold table.** Recommend
   parallel.
6. **Scope lock: 14a hooks-only.** Confirm Observers (14b) deferred.
7. **`on_replace` on in-place insert?** Decide whether
   `apply_replace_in_place` fires on_replace+on_insert (Bevy) or only
   on_insert.
8. **Panic policy for hooks** during the `catch_unwind`/`CursorSync` apply
   window — abort the command, the frame, or caught?

## Sources

Bevy: archetype.rs (`ArchetypeFlags`), lifecycle.rs / docs.rs
`ComponentHooks` + `HookContext`, deferred_world.rs (`DeferredWorld` "no
structural changes"), bundle.rs (`BundleSpawner`), issue #16034 (hook
command front-jump), PR #14212 (on_replace), observer module docs, PR
#1525 (bitset Access origin), examples/ecs/component_hooks.rs.
flecs: Observers Manual, observable.h (`observer_count`,
`flecs_observers_exist`), flecs.h (`EcsTableHasOnAdd/OnRemove` table
flags), Commands group (emit vs enqueue, defer counter).
Unity: Entities manual, Job System (batched structural changes).
boyko: `component.rs:32-59`, `component_registry.rs:67,91-125,141-226`,
`archetype.rs:109-170,203-310,480-507`, `command.rs:42-56,111-167`,
`{spawn_at,insert,remove,despawn}_command.rs`, `migration_helpers.rs`,
`ecs_master.rs:534-700,886-926`, `command_queue.rs:63-120`,
`event_dispatcher.rs:34-90`, `boyko_macros/src/lib.rs:31-84`,
`docs/PHASE-13-ROADMAP.md:46-50,205-215`.
