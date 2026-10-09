# Cloning & Prefabs

> Copy a live entity, or a whole hierarchy, into new entities; or freeze one into a `Prefab` and
> stamp out copies later.

## What it is

The kernel can copy an entity's components into a new entity. Three tools share the same
machinery:

- **`clone_and_spawn`** copies one entity, now.
- **`clone_subtree`** copies an entity and its `ChildOf` descendants, and wires the copies into a
  matching hierarchy.
- **`Prefab`** captures an entity and its subtree into an owned template. You can despawn the
  source and instantiate the template any number of times.

There is no reflection involved. Each component type carries a clone function pointer that the
`#[derive(Component)]` macro picks, and a program that never clones pays only for registering
those pointers. The code lives in the module
[`clone`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_ecs/src/ecs/core/clone)
of `boyko_ecs`.

## Defining cloneable components

The derive decides how a component is cloned from the traits it implements:

| Component type | How it is cloned |
|----------------|------------------|
| `Copy`, with no `Entity` field | The whole column slice is copied as bytes. |
| `Clone` (including `Copy` with an `Entity` field) | Its `Clone` impl runs per value. |
| `#[component(clone = path)]` | Your function runs per value. |
| Neither `Copy` nor `Clone`, or `#[component(no_clone)]` | Not cloneable; see below. |

A custom clone function has the signature
`unsafe fn(src: *const u8, dst: *mut u8)` (`CloneFn`): it reads a live value at `src` and writes a
new one into uninitialised memory at `dst`.

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_macros::{Bundle, Component};

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Health(f32);                 // Copy: cloned as raw bytes

#[derive(Component, Clone)]
struct Label(String);               // Clone: cloned through `Clone::clone`

#[derive(Bundle)]
struct Goblin {
    hp: Health,
    label: Label,
}
```

### Components that must not be copied

Mark a component that must stay unique, such as a network id, with `no_clone`:

```rust,ignore
#[derive(Component, Clone, Copy)]
#[component(no_clone)]
struct NetId(u64);
```

The cloner then has to leave it out on purpose. Exclude it with `deny::<NetId>()` on an opt-out
cloner, or leave it out of an opt-in cloner's `allow` list (see
[`EntityCloner`](#configuring-the-clone-entitycloner)). If a non-cloneable component reaches the
clone anyway:

- with `strict(true)`, the clone panics and names the component;
- otherwise a release build skips the component and the copy lands in a smaller archetype, while a
  debug build stops at a `debug_assert!`, because the skip usually means a forgotten `deny`.

## Cloning one entity

From a system, `Commands::clone_and_spawn` reserves the new entity's id at once and runs the clone
in the next apply window. It returns `EntityCommands`, so you can add components to the copy:

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_macros::{Component, Resource};

#[derive(Resource)]
struct Template(Entity);

#[derive(Component, Clone, Copy)]
struct Wave(u32);

fn spawn_wave(mut commands: Commands, template: Res<Template>) {
    for _ in 0..3 {
        commands.clone_and_spawn(template.0).insert(Wave(1));
    }
}
```

With `&mut EcsMaster` (for example `App::world_mut()`), the direct calls run immediately and
return the new entity:

```rust,ignore
let copy: Entity = world.clone_and_spawn(goblin);
let tree: Entity = world.clone_subtree(squad_leader);
```

A shallow clone copies `ChildOf` as it is, so the copy becomes a sibling of the source under the
same parent. It never copies `Children`: the copy starts with no children.

## Configuring the clone: `EntityCloner`

`EntityCloner` is a reusable, `Copy` configuration. It holds no borrow of the world, so you can build
it once and keep it.

```rust,ignore
use boyko_ecs::prelude::*;

// Opt-out (the default): every cloneable component except the denied ones.
let without_net = EntityCloner::new().deny::<NetId>().build();

// Opt-in: only the allowed components.
let stats_only = EntityCloner::only().allow::<Health>().build();

// Deep: copy the ChildOf subtree as well.
let deep = EntityCloner::new().linked(true).build();

let a = world.clone_and_spawn_with(goblin, &without_net);
commands.clone_and_spawn_with(goblin, stats_only);
```

| Builder method | Default | Effect |
|----------------|---------|--------|
| `deny::<C>()` / `deny_id(id)` | — | Opt-out builders only: skip `C`. |
| `allow::<C>()` / `allow_id(id)` | — | Opt-in builders only: clone `C`. |
| `linked(bool)` | `false` | `true` clones the `ChildOf` subtree. |
| `fire_hooks(bool)` | `true` | Run `on_add` / `on_insert` hooks and observers on the copy. |
| `strict(bool)` | `false` | Panic, in every build, on a component that cannot be cloned and is not excluded. |
| `preserve_ticks(bool)` | `false` | Keep the source's change ticks. By default the copy counts as added this frame, so `Added<T>` and `Changed<T>` see it. |

The builder is typestate: calling `allow` on an opt-out builder, or `deny` on an opt-in one, does
not compile.

## Deep clones

`clone_subtree(e)`, or a cloner with `linked(true)`, walks the `Children` index from the root. Each
node is cloned shallowly, then every copied `ChildOf` is pointed at the copied parent, and the
copies' `Children` lists are rebuilt through the normal hierarchy machinery. Relation links between
nodes of the subtree are remapped the same way. The result is an internally consistent copy of the
tree. See [Hierarchies](hierarchies.md).

## Prefabs

A `Prefab` is a frozen template of an entity and its `ChildOf` subtree. It owns its component
values, so it survives the source being despawned.

```rust,ignore
use boyko_ecs::prelude::*;

fn build_squads(world: &mut EcsMaster, leader: Entity) {
    let squad: Prefab = world.capture_prefab(leader);
    world.delete_entity(leader);          // the prefab does not need the source

    for _ in 0..4 {
        let root = world.instantiate(&squad);
        // `root` has no parent; attach it wherever it belongs.
    }
}
```

- `capture_prefab(e)` captures with the default cloner. `capture_prefab_with(e, &cloner)` applies
  the cloner's filter, strictness and hook setting. The subtree is always captured, so `linked` is
  ignored.
- Each `instantiate` re-runs every component's clone function, so instances never share values.
- The instance root is **detached**: it has no `ChildOf`. Inside the instance, `ChildOf` and
  in-subtree relation links point at the instance's own nodes, and `Children` is rebuilt.
- Instances count as added at instantiation: `preserve_ticks` is ignored.
- `Prefab` is neither `Send` nor `Sync`. Capture and instantiate it on the thread that owns the
  world.

## What a clone does not carry

- **Enable-tag state.** A bitset tag has no column, so the copy lands without it. See
  [Enable Tags](enable-tags.md).
- **Dense membership in a prefab.** `clone_and_spawn` and `clone_subtree` copy
  [dense components](dense-components.md). A `Prefab` does not capture them.
- **Remapping of other entity references.** In a deep clone or a prefab instance, `ChildOf` and
  [relation](relations.md) links whose target lies inside the copied subtree point at the copies.
  A relation link to an entity outside the subtree is not remapped, and any other `Entity`-typed
  field is copied unchanged.
- **Clone onto an existing entity.** Copying components onto an entity that already exists is not
  implemented.

## Performance characteristics

| Operation | Cost | Notes |
|-----------|------|-------|
| Clone, `Copy` component | One copy of the row's bytes per column | No per-value call. |
| Clone, `Clone` component | One `clone_fn` call per value | — |
| Component filter | One bit test per component | The filter is a fixed-size component mask, not a map. |
| Deep clone | One shallow clone per node, plus a relink pass | The walk is capped at 2<sup>20</sup> nodes as a cycle guard. |
| `instantiate` | One clone per captured component per node | Plus the `Children` rebuild. |

## See also

- [Entities](entities.md): spawning and the direct world API.
- [Commands](commands.md): the deferred `clone_and_spawn` path.
- [Hierarchies](hierarchies.md): `ChildOf` and `Children`.
- [Lifecycle Hooks, Observers & Triggers](hooks-and-observers.md): what fires on the copy.
- Source: [`clone/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_ecs/src/ecs/core/clone)
  and [`entity_api.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs).
