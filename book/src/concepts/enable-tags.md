# Enable Tags (Bitset Storage)

> An enable tag is a per-row bit you flip in place — no archetype migration, no fragmentation, ideal for high-churn flags.

*(Branch: `ecs`, EnableTag phase.)*

## What an enable tag is

A [tag](tags.md) encodes presence in the archetype **signature**: adding or
removing one migrates the entity to a different archetype. That is the right
model for stable, query-defining markers. It is the wrong model for a flag that
flips every frame — each toggle moves the whole row.

An **enable tag** is the second tag backend. Presence is encoded in a
per-archetype **paged bitset** — one bit per row. Toggling is an O(1) atomic
bit read-modify-write at `(archetype, row)`:

- **no archetype migration** — the entity stays exactly where it is;
- **no fragmentation** — the archetype set does not grow with toggle subsets;
- **no spawn-time tick-pool floor** — a never-toggled tag allocates nothing.

You pay for this with a per-row bit test during queries (about one
predicted-not-taken branch per row, bench-flat for queries that do not name an
enable tag) and one hard restriction: **change detection is not available** for
enable tags — the bit carries no per-row tick, so `Added`/`Changed` over an
enable tag is a **compile error**, not a silent lie.

Use an enable tag for high-churn transient flags toggled every frame on many
entities: `Stunned`, `Frozen`, `Selected`, `Hidden`. Use an archetype tag for
low-churn, query-defining identity (`Player`, `Boss`). The
[Storage Trade-offs](../architecture/storage-tradeoffs.md) page is the full
decision model.

## Defining an enable tag

A bitset tag is a zero-sized `#[derive(Component)]` type with one attribute:

```rust,ignore
use boyko_macros::Component;

#[derive(Component)]
#[component(storage = "bitset")]
struct Stunned;

#[derive(Component)]
#[component(storage = "bitset")]
struct Selected;
```

The attribute does two things at registration:

- classifies the id as **bitset storage**, so it is filtered out of every
  archetype signature and gets no `ComponentPool`;
- **suppresses the single-component `Bundle`** that a plain `#[derive(Component)]`
  emits. A bitset tag has no pool, so it is deliberately **not spawnable as a
  one-component bundle** — `commands.spawn(Stunned)` does not compile. You spawn
  the entity normally, then toggle the bit.

A bitset tag must be zero-sized. Adding a field is a compile error — a flag with
data is a data component, not an enable bit.

## Toggling and probing

Enable, disable, and test are direct `EcsMaster` operations. Enable and disable
take `&mut EcsMaster` (they are structural-class operations — see
[the access contract](#the-access-contract)); the probe takes `&self`:

```rust,ignore
use boyko_ecs::prelude::*;

fn example(world: &mut EcsMaster, e: Entity) {
    world.enable::<Stunned>(e);            // set the bit — O(1), no migration
    assert!(world.is_enabled::<Stunned>(e));

    world.disable::<Stunned>(e);           // clear the bit
    assert!(!world.is_enabled::<Stunned>(e));
}
```

Two enable tags are fully independent — each is a separate bit. Toggling an
entity that has been despawned is a silent no-op (it never panics and never
leaks a set bit onto a recycled slot), matching the deferred-command contract.

## Querying: `Enabled<T>` / `Disabled<T>`

`Enabled<T>` and `Disabled<T>` are per-row query filters. They live in
`boyko_ecs::ecs::core::iters::query`:

- `Query<&D, Enabled<A>>` visits rows whose `A` bit is **set**.
- `Query<&D, Disabled<A>>` visits rows whose `A` bit is **clear**. A row in an
  archetype that never allocated an `A` column reads as disabled, so a
  positive-data `Disabled` query also visits no-`A`-column rows.

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_ecs::ecs::core::iters::query::{Enabled, Disabled};

# #[derive(Clone, Copy)] #[derive(boyko_macros::Component)] #[repr(C)]
# struct Position { x: f32, y: f32 }
// Sum a data column over only the stunned entities.
fn stunned_total(world: &mut EcsMaster) -> f32 {
    world
        .query::<&Position, Enabled<Stunned>>()
        .iter()
        .map(|p| p.x)
        .sum()
}

// The complement: every Position row whose Stunned bit is clear.
fn not_stunned_total(world: &mut EcsMaster) -> f32 {
    world
        .query::<&Position, Disabled<Stunned>>()
        .iter()
        .map(|p| p.x)
        .sum()
}
```

Do **not** pair `Enabled<T>` with `&T` to read data — a bitset tag has no
storage. The filter only gates rows; pair it with the real data components you
want to read (`Query<&Position, Enabled<Stunned>>`).

The same filters work as the system parameter `Query<D, F>` and the direct-API
`world.query::<D, F>()` view shown above, across every driver
(`iter`/`iter_mut`, `par_iter`, `get`/`single`).

## The data-less global scan

The sole forms `Query<(), Enabled<A>>` and `Query<(), Disabled<A>>` — no
positive data term — are supported. They are a **bounded global scan**: the
candidate archetypes are seeded from a per-world archetype-presence bitset, so
the walk visits only archetypes where `A` is a property (where its column was
ever allocated), never the whole world.

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_ecs::ecs::core::iters::query::Disabled;

// How many entities (across present-A archetypes) currently have A off?
fn count_disabled(world: &mut EcsMaster) -> usize {
    world.query::<(), Disabled<Stunned>>().iter().count()
}
```

A subtlety worth stating once: the sole `Query<(), Disabled<A>>` enumerates only
archetypes where `A` is a property (present-`A` archetypes), so it does **not**
visit a no-`A`-column archetype — whereas the positive-term
`Query<&D, Disabled<A>>` does. The two shapes answer different questions: the
positive-term query says "of my `D` entities, which have `A` off"; the sole
query says "which entities have `A` as a property and currently off."

## Dynamic enable tags

Like [dynamic tags](dynamic-tags.md), enable tags can be minted at runtime from
a name. `register_enable_tag` returns an `EnableTagId`; toggle with the `_id`
variants and filter with runtime `with_enabled` / `without_enabled` terms:

```rust,ignore
use boyko_ecs::prelude::*;

let mut world = EcsMaster::new();
let frozen = world.register_enable_tag("frozen");   // EnableTagId, mint once at setup

# let e = world.spawn_empty();
world.enable_id(e, frozen);
assert!(world.is_enabled_id(e, frozen));

// Runtime per-row terms on any query view:
let count = world
    .query::<&Position, ()>()
    .with_enabled(frozen)        // keep rows whose `frozen` bit is set
    .iter()
    .count();
```

`with_enabled(tag)` keeps rows whose bit is set; `without_enabled(tag)` keeps
rows whose bit is clear (a never-allocated column reads as clear). The dynamic
per-row term filters identically to the typed `Enabled<T>` / `Disabled<T>` path.

`register_enable_tag` shares the `MAX_COMPONENTS` (512) id budget with every
typed component and dynamic tag; it panics on exhaustion. Use
`try_register_enable_tag` for a fallible mint when names come from user data.
A query carries at most `MAX_ENABLE_TERMS` (8) dynamic terms — exceeding it is a
loud panic at term-add time (setup), never a silent truncation.

## What is rejected (and why it is a compile error, not a silent lie)

Enable tags carry no per-row tick and are per-row (not archetypal) predicates.
Two shapes are therefore **compile-rejected**:

- **No change detection.** `Added<BitsetTag>`, `Changed<BitsetTag>`, and mixing
  an enable term with change detection in one query
  (`Query<&D, (Changed<C>, Enabled<A>)>`) are compile errors. The bit has no
  tick, so any such filter could only compile-and-lie.
- **Not composable in `Or<…>`.** `Enabled` / `Disabled` are sealed against
  `Or`: folding a per-row test against an archetypal element's unconditional
  `true` would leak disabled rows, so `Or<(Enabled<A>, …)>` does not compile.

This is the project's recurring trade: an honest, named compile error over a
filter that runs and silently never matches.

## Internals

Each hosting archetype stores an enable column as **lazily allocated 512-byte
pages**: one page is `[AtomicU64; 64]` = 4096 bits, covering 4096 rows. A tag
with no toggles in an archetype allocates nothing there; a tag whose rows all
sit in the first page allocates one page. The bit's home is
`page = row >> 12`, `word = (row >> 6) & 63`, `bit = row & 63`. The per-row read
is a `Relaxed` atomic load plus a bit test.

The first toggle of a tag into an archetype allocates that archetype's column,
records the archetype in a per-world presence bitset (the candidate set behind
the data-less global scan), and bumps the world's `enable_generation` once.
Steady-state toggles touch only the bit.

### The access contract

Enable and disable are **structural-class** operations: they take
`&mut EcsMaster`, which is what makes the `Relaxed` atomics on the bit and on
`enable_generation` sound in v1 — no worker thread is live during a toggle.
Queries read the bit **shared** (`&self`). The discipline mirrors structural
operations: **do not toggle an enable bit during query iteration.**

## Performance characteristics

| Operation | Cost | Notes |
|-----------|------|-------|
| Toggle (`enable`/`disable`) | O(1) atomic bit RMW | no migration, no structural bump |
| First toggle into an archetype | O(1) + lazy page alloc | allocates the 512 B page, bumps `enable_generation` once |
| `is_enabled` | O(1), ≤ 5 ns | inland load → column scan (≤ 4) → paged bit test |
| Query per-row gate | ≈ 1 branch/row | bench-flat for queries with no enable term |
| Data-less global scan | O(present-`A` archetypes) | bounded by the presence bitset, never a full-world sweep |
| Carry an enable tag | 0 B/row resident until toggled | pages are demand-allocated |

Contrast with an archetype tag: carrying it costs 8 B/row (the tick pair) and
toggling it is an archetype migration, but `With`/`Without` filtering is free
(whole archetypes are included or excluded). Pick the backend by toggle
frequency — the [Storage Trade-offs](../architecture/storage-tradeoffs.md) page
is the decision matrix.

## See also

- [Tags](tags.md) — the archetype-signature tag backend and its 8 B/row tick model
- [Dynamic Tags](dynamic-tags.md) — runtime-minted, name-keyed archetype tags
- [Storage Trade-offs: Tags, Churn, and Fragmentation](../architecture/storage-tradeoffs.md) — Table vs Bitset decision matrix
- [Change Detection](../change_detection.md) — why it does not extend to enable tags
- Source: [`enable_tag_api.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs) (`register_enable_tag` / `enable` / `disable` / `is_enabled`), [`filter_enable.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/iters/query/filter_enable.rs) (`Enabled` / `Disabled`), [`enable_terms.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/iters/query/enable_terms.rs) (`with_enabled` / `without_enabled`), [`enable_store.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs) (paged bitset)
