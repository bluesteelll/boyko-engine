# Dense Components

> A dense component keeps every instance of its type in one global column, outside the archetype
> tables, so adding or removing it never moves an entity.

## What it is

By default a component is a **table** component: it is part of the archetype signature, and its
data lives in that archetype's column. Adding or removing a table component therefore moves the
entity to another archetype, and copies its row.

A **dense** component is opted out of the signature. Each dense type owns one
[`DenseStore`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs):
one contiguous column that holds every instance of the type, across all archetypes, keyed by
`EntityId`. Attaching or detaching it touches only that store. The entity keeps its archetype.

Use dense storage for data that:

- comes and goes often on entities that otherwise keep their shape;
- is processed as one flat array by its own system, regardless of the other components;
- needs stable slots: a live instance never moves inside its column.

The engine uses it for `GpuTransform3D`, the per-instance transform pair the renderer uploads for
GPU interpolation, and for the sprite cursor and tween components of `boyko_ui`.

| Storage | Declared with | Data lives in | Attach/detach moves the entity | Change detection |
|---------|---------------|---------------|--------------------------------|------------------|
| Table (default) | `#[derive(Component)]` | the archetype's column | yes | yes |
| Dense | `#[component(storage = "dense")]` | one global `DenseStore` per type | no | yes |
| Bitset | `#[component(storage = "bitset")]` | one bit per row | no | no |

Bitset storage is for zero-sized enable flags; see [Enable Tags](enable-tags.md).

## Defining a dense component

Add `storage = "dense"` to the `#[component]` attribute:

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_macros::{Bundle, Component};

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
}

/// Every `Velocity` in the world lives in one contiguous column.
#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct Velocity {
    x: f32,
    y: f32,
}

/// A dense component is not a bundle by itself; wrap it in one.
#[derive(Bundle)]
struct Mover {
    pos: Position,
    vel: Velocity,
}

fn setup(mut commands: Commands) {
    commands.spawn(Mover {
        pos: Position { x: 0.0, y: 0.0 },
        vel: Velocity { x: 1.0, y: 0.5 },
    });
}
```

A table component derives a one-component `Bundle` impl, so `commands.spawn(Position { .. })`
works. The derive **does not** emit that impl for a dense component. Spawn and insert it through a
`#[derive(Bundle)]` struct, as above. The same holds for `EntityCommands::insert`. Removal uses the
type directly: `commands.entity(e).remove::<Velocity>()`.

`#[require(...)]` accepts a dense component: it has bytes for the constructor to write. A bitset
flag is refused at compile time.

## How it's used

### Mixed queries

A dense component works in an ordinary query next to table components. The iterator walks the
matching archetypes and checks each row's membership in the dense store:

```rust,ignore
use boyko_ecs::prelude::*;

fn integrate(mut query: Query<(&mut Position, &Velocity)>) {
    for (pos, vel) in query.iter_mut() {
        pos.x += vel.x;
        pos.y += vel.y;
    }
}
```

A row whose entity has `Position` but no `Velocity` is skipped. `With<Velocity>` and
`Without<Velocity>` filter on dense membership the same way, including inside `Or<...>`.

### The pure-dense fast path

When a system needs only the dense column, `dense_iter` and `dense_iter_mut` stride it directly.
They skip freed slots and yield `(EntityId, &T)` or `(EntityId, &mut T)` in slot order:

```rust,ignore
use boyko_ecs::prelude::*;

fn damp(mut query: Query<&mut Velocity>) {
    for (_id, vel) in query.dense_iter_mut() {
        vel.x *= 0.99;
        vel.y *= 0.99;
    }
}
```

Both methods accept exactly one dense leaf, `&T` or `&mut T`. A table type, a tuple or an enable
filter (`Enabled<T>`, `Disabled<T>`) is a compile error. For an enable-filtered dense query, use
`iter` / `iter_mut`, which check the enable bit per row.

### Change detection, hooks and direct access

- `Added<T>` and `Changed<T>` work on dense components. The store keeps per-slot ticks, and a
  reused slot gets fresh ticks.
- Lifecycle hooks and observers (`on_add`, `on_insert`, `on_replace`, `on_remove`, `on_despawn`)
  fire for dense components on every structural path of `EcsMaster` and `Commands`, despawn
  included. The exception is a direct `DenseStore::remove` through the public
  `EcsMaster::dense_registry_mut()`: it fires no `on_replace` or `on_remove`.
- `get_component`, `get_component_mut` and `has_component` route to the dense store.
  [`EcsMaster::dense_contains`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs)
  and `dense_slot_of` answer membership by `ComponentId`.
- Binary save/load round-trips plain-old-bytes dense stores; see
  [Serialization](../persistence/serialization.md).

### Restrictions

| API | Dense term allowed? | Use instead |
|-----|---------------------|-------------|
| `iter`, `iter_mut`, `get`, `get_mut` | yes | — |
| `dense_iter`, `dense_iter_mut` | only a single dense leaf | — |
| `par_iter`, `par_iter_mut` | no (compile error) | `iter_mut` or `dense_iter_mut` |
| `for_each_chunk`, `par_for_each_chunk` | no (compile error) | `iter_mut` or `dense_iter_mut` |
| `Query::contains` | no (compile error) | `query.get(e).is_some()` |

A derived dense type is host-resident (`ResidencyKind::Cpu`): the derive has no residency key and
never sets one, so the type cannot be a [GPU-resident column](../rendering/gpu-columns.md). A
hand-written `impl Component` that pairs dense storage with a `Gpu` residency is caught only by a
`debug_assert!` when the type registers, so only debug builds catch it.

## Internals

A `DenseStore` holds one `ComponentPool` column plus its bookkeeping:

- an `EntityId → slot` map, the membership oracle;
- a `slot → EntityId` column, which gives iteration its order and the save format its key;
- a per-slot liveness bitmap;
- a LIFO free list of dead slots.

Removal tombstones the slot and pushes it onto the free list; it never swaps the last row in. A
later insert reuses the most recently freed slot before the column grows. Live slots therefore never
move, which gives `dense_iter` a deterministic order.

Each store also keeps a conservative set of the archetypes that ever hosted a member. A query with a
dense `With<T>` uses it to skip archetypes that never held one. The exact answer is still the
per-row membership test.

A store is created on the first insert of its type, so a world without dense components builds
none. The dense branches in the query code compile away for queries without a dense term.

## Performance characteristics

| Operation | Complexity | Notes |
|-----------|------------|-------|
| Insert | O(1) amortised | Pops the free list, or appends at the column's end. No archetype move. |
| Remove | O(1) | Tombstones the slot. No archetype move, no swap. |
| Membership test | O(1) | One lookup in the `EntityId → slot` map. |
| `dense_iter` | O(slots ever used) | One contiguous column; freed slots are skipped by the liveness bitmap. |
| Mixed `iter` | O(rows of matching archetypes) | Adds a per-row membership test. |

The trade-off: a table component is read straight from its archetype column, while a dense
component in a mixed query costs one extra lookup per row. Many freed slots also make
`dense_iter` stride over dead entries until inserts reuse them.

## See also

- [Components](components.md): table components and the `#[component]` attribute.
- [Enable Tags](enable-tags.md): the bitset storage kind.
- [Storage Trade-offs](../architecture/storage-tradeoffs.md): what each storage kind costs.
- [Queries](queries.md) and [Change Detection](../change_detection.md).
- Source: [`component/dense/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_ecs/src/ecs/core/component/dense)
  and [`iters/query/dense_iter.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/iters/query/dense_iter.rs).
