# Hierarchies

Parent-child trees in boyko are not a bespoke scene-graph bolted onto the engine.
They are two ordinary components — `ChildOf` on the child and `Children` on the
parent — kept consistent by [lifecycle hooks](./hooks-and-observers.md). The
shape mirrors **Bevy 0.16**: one foreign-key component is the source of truth,
the reverse collection is derived. If you have used Bevy's hierarchy, this will
feel identical; the engine-specific parts are the *when* (deferred consistency
window) and the *why* (no separate graph data structure to keep in sync).

Hierarchies are a concrete instance of the generic [relations](./relations.md)
substrate. `ChildOf` is a `Relationship` and `Children` is its
`RelationshipTarget` — everything on this page is the same machinery you can
build your own relation pairs from.

## The two components

```rust,ignore
use boyko_ecs::prelude::*; // ChildOf, Children, Entity

// On the CHILD — the foreign key, and the single source of truth.
pub struct ChildOf(pub Entity);

// On the PARENT — the reverse collection, derived. Read-only to you.
pub struct Children { /* private Vec<Entity> */ }
```

`ChildOf` is what you write. `Children` is what the engine maintains for you: you
**never** construct or mutate `Children` directly — its constructors are
crate-internal. Inserting, overwriting, or removing `ChildOf` is the *only* way
the tree changes, and the hooks on `ChildOf` reactively patch the parent's
`Children` to match.

Both types are re-exported from the prelude
([`prelude.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/prelude.rs#L33)):

```rust,ignore
use boyko_ecs::prelude::*;       // ChildOf, Children, Commands, Query, ...
use boyko_macros::{Component, Bundle}; // derives live in boyko_macros, NOT the prelude
```

> Remember the import split: traits and the `ChildOf` / `Children` *types* come
> from `boyko_ecs::prelude`, but `#[derive(Component)]` / `#[derive(Bundle)]` for
> your own components come from `boyko_macros`.

## Linking: spawning children and setting `ChildOf`

The ergonomic surface lives on [`Commands`](./commands.md) and `EntityCommands`.
All of it funnels through `ChildOf` insertion/removal — these are thin wrappers
that never touch `Children`.

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_macros::{Component, Bundle};

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Name(u32);

// A bare tuple is NOT a Bundle in boyko (the blanket tuple impl was removed).
// Wrap your components in a #[derive(Bundle)] struct.
#[derive(Bundle)]
struct NameBundle { name: Name }

fn build_tree(mut cmds: Commands) {
    // Spawn a parent and capture its (reserved) Entity id.
    let parent = cmds.spawn(NameBundle { name: Name(0) }).id();

    // Spawn a child and attach it in one chain.
    cmds.spawn(NameBundle { name: Name(1) })
        .set_parent(parent); // inserts ChildOf(parent) on this child

    // Or attach an already-spawned child from the parent's side.
    let other = cmds.spawn(NameBundle { name: Name(2) }).id();
    cmds.entity(parent).add_child(other);

    // Free function form: commands.add_child(parent, child).
    let yet_another = cmds.spawn(NameBundle { name: Name(3) }).id();
    cmds.add_child(parent, yet_another);
}
```

The relationship methods, all chainable on `EntityCommands`
([`entity_commands.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs#L317)):

| Method | Effect |
|--------|--------|
| `.entity(p).add_child(c)` | Inserts `ChildOf(p)` on `c` |
| `.entity(p).add_children(&[c1, c2])` | One `ChildOf(p)` insert per child |
| `.entity(c).set_parent(p)` | Inserts `ChildOf(p)` on `c` (reparents if already parented) |
| `.entity(c).remove_parent()` | Removes `ChildOf` from `c` |
| `.entity(p).remove_children(&[c1])` | Removes `ChildOf` from each listed child |
| `.entity(p).clear_children()` | Removes `ChildOf` from **all** current children (does not despawn them) |
| `commands.add_child(p, c)` | Free-function equivalent of `.entity(p).add_child(c)` |

**Reparenting is atomic.** Overwriting `ChildOf` on a child that already had a
parent unlinks it from the old parent before linking it into the new one — the
old parent's unlink (`on_replace`) is applied before the new parent's link
(`on_insert`), in FIFO order.

### The consistency window

This is the one place hierarchies differ from a naive in-memory tree, and it
matters. `ChildOf`'s hooks do not mutate `Children` immediately. They enqueue
deferred link/unlink commands that run at the **apply window** — the
deferred-command drain at the end of the current system (or at the next
`CommandQueue` apply for direct-API mutations). So:

```rust,ignore
use boyko_ecs::prelude::*;

fn link(mut cmds: Commands, parent: Entity, child: Entity) {
    cmds.entity(parent).add_child(child);
    // INSIDE this system, parent's `Children` does NOT yet contain `child`.
    // It becomes consistent after the system returns and commands drain.
}
```

This is the same same-frame staleness the engine already accepts for any
command-driven mutation. Read `Children` *after* the apply window (a later
system, or after the `run_system` / direct call returns), never within the same
command batch that mutated it.

## Iterating `Children`

`Children` is just a component, so you read it through a [query](./queries.md)
or with `get_component`. It exposes a slice plus the usual length helpers
([`hierarchy/mod.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs#L122)):
`as_slice() -> &[Entity]`, `len()`, `is_empty()`, `contains(Entity)`.

```rust,ignore
use boyko_ecs::prelude::*;

// Visit every parent and its direct children.
fn print_children(q_parents: Query<(Entity, &Children)>) {
    for (parent, children) in q_parents.iter() {
        for &child in children.as_slice() {
            let _ = (parent, child); // ... do work per (parent, child) edge
        }
    }
}
```

Two properties worth internalising, both consequences of the storage choice:

- **Sibling order is unspecified** and changes on removal. A child is dropped
  with `swap_remove` (O(1) — the last child fills the gap), so the order is not a
  stable contract. Sort at the consumer if you need determinism.
- **An emptied `Children` is retained, not removed.** When the last child leaves,
  the parent keeps an empty `Children` (a 24-byte header over a zero-capacity
  `Vec`, no heap allocation). This is deliberate: a `0 ↔ 1 ↔ 0` child-count
  oscillation under remove-on-empty would migrate the parent's archetype on every
  flip (a full byte-copy, ~590 ns class) versus an in-place `swap_remove`
  (~90 ns class). Archetype-gated iteration skips an empty `Children` row at zero
  cost, so retaining it is free.

### Walking the whole subtree

For transitive walks the engine provides relation accessors on `EcsMaster`, so
you do not hand-roll recursion over `Children`
([`relations_query_dsl.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/tests/relations_query_dsl.rs#L613)):

```rust,ignore
use boyko_ecs::prelude::*;

fn subtree(ecs: &EcsMaster, root: Entity) {
    // Depth-first over the reverse collections; excludes `root` itself.
    for descendant in ecs.descendants::<ChildOf>(root) {
        let _ = descendant;
    }
    // Walk the FK chain upward; excludes the start node.
    for ancestor in ecs.ancestors::<ChildOf>(root) {
        let _ = ancestor;
    }
}
```

These are the same `descendants` / `ancestors` walks any `Relationship` gets — see
[Relations](./relations.md) for the full DSL (`Related<R, D>` joins, relation
filters, wildcard traversal).

## Despawning: the recursive cascade

The default is **recursive**. Despawning a parent despawns its entire subtree.
This is driven by `Children`'s `on_replace` hook (`LINKED_DESPAWN`): when the
parent is removed, its children are recursively despawned, theirs in turn, and so
on. The cascade recurses through the engine's flat deferred queue rather than the
call stack, so depth is bounded by a guard, not by the native stack.

```rust,ignore
use boyko_ecs::prelude::*;

// Both of these cascade to every descendant of `root`:
fn nuke(mut cmds: Commands, root: Entity) {
    cmds.despawn(root);
}

fn nuke_direct(ecs: &mut EcsMaster, root: Entity) {
    ecs.delete_entity(root); // direct-API form, drains immediately
}
```

### Opting out: `despawn_without_children`

When you want to remove a parent but keep its children alive, use the opt-out.
Exactly the one cascade hook for that despawn is suppressed; unrelated despawns
queued by other hooks still cascade normally.

```rust,ignore
use boyko_ecs::prelude::*;

fn detach_then_remove(mut cmds: Commands, parent: Entity) {
    // Children survive — but each keeps a now-DANGLING ChildOf pointing at the
    // freed parent. Reparent or despawn them explicitly to avoid the footgun.
    cmds.entity(parent).despawn_without_children();
}

// Direct-API equivalent:
fn detach_direct(ecs: &mut EcsMaster, parent: Entity) {
    ecs.despawn_without_children(parent);
}
```

The trade-off is explicit: the surviving children's `ChildOf` is left pointing at
a dead entity. If you want them re-rooted, call `clear_children()` on the parent
first (which removes `ChildOf` from every child) and *then* despawn the parent —
that leaves the children parentless but with no dangling reference.

## Guards and footguns

- **Self-reference is rejected.** A `ChildOf(self)` is reactively removed by the
  link hook; the parent's collection is never touched.
- **Dangling parent is rejected.** A `ChildOf` pointing at a non-existent entity
  is reactively removed.
- **Deep cycles are NOT detected.** Only the one-compare self-reference guard
  exists. An `A → B → … → A` cycle is a documented footgun: a recursive despawn
  over a cycle would re-enter indefinitely. Do not build `ChildOf` cycles.
- **Read after the apply window.** `Children` is stale within the same command
  batch that mutated `ChildOf` (see the consistency window above).

## Zero cost when unused

A program that never mints a `ChildOf` / `Children` component id leaves the cold
hook slots unset. The per-archetype flag gate therefore raises no hierarchy bit,
and the hot iteration path pays nothing for hierarchies it does not use — the
same 0%-when-unused discipline the hooks substrate guarantees everywhere.

## See also

- [Relations](./relations.md) — the generic substrate `ChildOf` / `Children` are built on
- [Hooks and observers](./hooks-and-observers.md) — the reactive mechanism that keeps `Children` consistent
- [Commands](./commands.md) — the deferred-mutation API and apply window
- [Queries](./queries.md) — how to iterate `Children` and join across relations
- Source: [`core/hierarchy/mod.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs#L63), [`hierarchy/commands.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/hierarchy/commands.rs), [`ecs_master.rs` despawn cascade](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs#L1362)
