# Dynamic Tags

> A dynamic tag is a tag minted at runtime from a string name — no Rust type required.

*(Branch: `ecs`, Phase 22.)*

## What they are for

Static [tags](tags.md) are Rust types, fixed at compile time. Dynamic tags
cover the other half: tags whose set is **data-driven** — loaded from config
files, scripting layers, editors, or network protocols. You mint one by name
and get back a `TagId` handle:

```rust,ignore
use boyko_ecs::prelude::*;   // EcsMaster, TagId, ...

let mut world = EcsMaster::new();
let poisoned: TagId = world.register_tag("poisoned");
```

Under the hood a dynamic tag is an ordinary size-0 `ComponentId` in the same
registry as every typed component. Storage, hooks, observers, archetype
matching and migration are exactly the static-tag machinery — only the
*identity* (a name instead of a type) is new.

## Registering

| Method | Behavior |
|--------|----------|
| `world.register_tag(name) -> TagId` | mints or resolves; **panics** if the budget is exhausted |
| `world.try_register_tag(name) -> Option<TagId>` | fallible variant; `None` = budget exhausted |
| `world.tag_by_name(name) -> Option<TagId>` | lookup only, never mints |

Key properties:

- **Idempotent per name.** The same name always returns the same `TagId`, in
  any world — tags are process-global metadata, like every `ComponentId`. An
  already-interned name is always a success, even after the budget is gone.
- **The name is the stable key.** The numeric id depends on first-call order
  and is *not* stable across processes — serialize names, not ids.
- **Shared 512-slot budget.** Dynamic tags share `MAX_COMPONENTS = 512` with
  every typed component, and registry slots are write-once: a minted tag is
  never unregistered. Prefer `try_register_tag` when names come from user
  data — the 513th unique mint returns `None` instead of taking the process
  down; `register_tag` is the panicking sugar with a message naming the budget.
- **Cold path.** Minting takes a lock + hash-map insert. Mint once at setup
  and keep the `TagId`; never call `register_tag` per frame.

## The `TagId` → `ComponentId` bridge

`TagId` is a `#[repr(transparent)]` wrapper over `ComponentId` — a type-level
proof that the id was minted as a size-0 tag. The id-keyed engine surfaces
(hooks, observers) speak `ComponentId`, so the bridge is public:

```rust,ignore
let cid: ComponentId = poisoned.component_id(); // const fn
let cid: ComponentId = poisoned.into();         // From<TagId> for ComponentId
```

The bridge is deliberately **one-way**: there is no `ComponentId -> TagId`
constructor, and data-fetch APIs cannot accept a bare `TagId` — the
filter-only guarantee stays type-enforced.

## Attaching, detaching, checking

```rust,ignore
// Direct (structural, &mut world):
world.add_tag(entity, poisoned);
world.remove_tag(entity, poisoned);
let is_poisoned: bool = world.has_tag(entity, poisoned); // O(1), ~two loads + bit test

// Deferred (inside a system):
fn apply_poison(mut commands: Commands, target: Entity, poisoned: TagId) {
    commands.entity(target).add_tag(poisoned);
    // ... later:
    // commands.entity(target).remove_tag(poisoned);
}
```

Semantics:

- **`add_tag`, tag absent** — archetype migration into `source ∪ {tag}`;
  `on_add` + `on_insert` hooks/observers fire.
- **`add_tag`, tag present** — in-place replace semantics: `on_replace` +
  `on_insert` fire and the changed tick is stamped; no migration.
- **`remove_tag`, tag present** — migration into `source \ {tag}`;
  `on_replace` + `on_remove` fire. Removing the *last* component routes the
  entity into the empty archetype — it stays alive with zero components.
- **`remove_tag`, tag absent** — silent no-op (parity with `remove::<C>()`).
- **Dead or stale entity** — silent no-op, matching the deferred-command
  contract (a despawn may legitimately race an enqueued tag op within a frame).

## Querying: `with_tag` / `without_tag`

Typed `With<T>`/`Without<T>` cannot name a runtime id, so both `Query<D, F>`
(the system parameter) and `QueryView<D, F>` (the direct API) carry builder
methods that add **runtime tag terms**:

```rust,ignore
// Direct API:
let view = world.query::<&Position, ()>().with_tag(poisoned);
for pos in view.iter() { /* only poisoned entities */ }

// In a system:
fn tick_poison(q: Query<&mut Health>, poisoned: Res<PoisonTag>) {
    let mut q = q.with_tag(poisoned.0);
    for mut hp in &mut q {
        hp.current -= 1.0;
    }
}
```

Properties of tag terms:

- **Archetype granularity, resolved once per epoch.** Terms resolve **once
  per epoch at the driver entry** (Phase 22.1), not per archetype transition
  and never per row. The first driver call builds a memoised, term-filtered
  `&[ArchetypeId]` slice; the iteration cursors and the chunk/par drivers then
  walk that pre-resolved slice carrying **zero** term code. The bit-test (at
  most eight signature-bit tests per matched archetype) runs once, during that
  per-epoch prefilter build — plus on each point lookup in `QueryView::get` /
  `get_mut`, where a prefilter cannot help a single in-hand archetype. With no
  terms set, the driver takes the shared pre-terms slice through one predicted
  not-taken branch — the inner row loop is byte-identical to a term-free query.
- **Honored by every iteration driver** (both `Query` and `QueryView`):
  `iter`/`iter_mut`, `iter_entities`/`iter_entities_mut`,
  `par_iter`/`par_iter_mut`, `for_each_chunk`/`par_for_each_chunk`, and the
  `archetype_count`/`is_empty` accessors.
- **Point lookups are `QueryView`-only.** `single`/`single_mut` and
  `get`/`get_mut` exist on `QueryView` (the direct API) only — the `Query`
  SystemParam has no point-lookup methods. On the lookup path the term test
  runs per call against the in-hand archetype.
- **Ceiling: 8 terms** per query (`MAX_DYN_TAG_TERMS`), `with_tag` +
  `without_tag` combined. Exceeding it is a loud, release-active panic at
  term-add time (setup), never a silent truncation.
- Terms compose freely with typed filters (`With`, `Without`, `Added`,
  `Changed`, `Or`).
- Typed `Added`/`Changed` filters need a type to name, so they do not exist
  for dynamic tags yet — but ticks **are** maintained in dynamic-tag pools, so
  a future `DynAdded(TagId)` term needs no storage change.

## Hooks and observers: the registration contract

Observers were already id-keyed — `world.add_observer(kind, cid, runner)`
accepts `tag.component_id()` directly. Hooks gain the id-keyed entry point
`register_hooks_by_id`:

```rust,ignore
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::component::hooks::ComponentHooks;
use boyko_ecs::ecs::core::component::observers::ObserverKind;

let tag = world.register_tag("enemy");

// 1. mint  →  2. register hooks  →  3. first attach. THIS ORDER.
component_registry::register_hooks_by_id(
    tag.component_id(),
    ComponentHooks { on_add: Some(on_enemy_added), ..Default::default() },
)?;
world.add_observer(ObserverKind::Add, tag.component_id(), enemy_observer);

world.add_tag(entity, tag);   // hook AND observer fire
```

The contract — **mint → register hooks → first attach** — is enforced, not
advisory. An archetype's hook-flag bits are computed once, at archetype
creation. If you attach the tag first and register hooks afterwards, the
hosting archetype's flags are already frozen and the hook would silently never
fire there. `register_hooks_by_id` therefore returns
`Err(HooksError::AlreadyArchetyped { .. })` the moment the id has ever been
placed in an archetype of any world — turning a would-be silent lie into a
named, actionable error. (`HooksError::AlreadyRegistered` covers the ordinary
write-once collision.)

Observers have no such gate: registering an observer late triggers a dynamic
walk that raises the bits on existing archetypes.

## Limits and costs

| Aspect | Value |
|--------|-------|
| Id budget | 512 `ComponentId` slots, shared with typed components, write-once |
| Exhaustion | `try_register_tag` → `None`; `register_tag` → panic naming the budget |
| Storage per tag | 8 B/row (ticks) — identical to static tags |
| `has_tag` | O(1): inland load → archetype pointer → signature bit test |
| Query terms | ≤ 8 per query; archetype-level, resolved once per epoch into a memoised id slice; loud panic past the ceiling |
| Toggle cost | archetype migration (row move) — see the churn ladder |
| Stable identity | the **name** (ids are process-unstable) |

## See also

- [Tags](tags.md) — the static-tag model, the 8 B/row rationale, change detection
- [Storage Trade-offs: Tags, Churn, and Fragmentation](../architecture/storage-tradeoffs.md) — when a tag is the wrong tool
- Source: [`component_registry.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/component/component_registry.rs) (`TagId`, the mint protocol, `register_hooks_by_id`), [`tag_api.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/ecs_master/tag_api.rs) (`register_tag` / `add_tag` / `remove_tag` / `has_tag`), [`tag_terms.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/iters/query/tag_terms.rs) (query terms)
