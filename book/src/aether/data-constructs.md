# Data Constructs

Four constructs cover the data half of an `aether!` block: `component`, `tag`,
`bundle` and `event`. All four are pure surface: each expands to the annotated
struct you would have written by hand, and the `boyko_macros` derive behind it
does every bit of the real work. Anything the derive supports flows through;
anything it rejects is still rejected, with the derive's own message.

Read [Components](../concepts/components.md), [Bundles](../concepts/bundles.md),
[Tags](../concepts/tags.md) and [Events](../concepts/events.md) for what these
types *are* — this page is only about the notation and what it becomes.

## `component`

```ebnf
component := 'component' IDENT '{' item* '}'
item      := IDENT ':' TYPE                                   (* a field *)
           | 'requires' PATH (',' PATH)*                       (* required components *)
           | ('on_add'|'on_insert'|'on_replace'|'on_remove') '=' PATH   (* a hook *)
           | 'no_bundle'                                       (* opt out of the 1-component Bundle *)
```

Items are comma-separated and a trailing comma is always allowed. Component
names are **UpperCamelCase** — they expand to types, and the parser says so with
a rename suggestion if you forget.

```rust,ignore
use aether::aether;

aether! {
    component Health {
        current: f32,
        max: f32,
        requires Regen,
        on_add = heal_full,
    }
}
```

expands to exactly this — nothing hidden, nothing extra:

```rust,ignore
#[derive(::boyko_macros::Component)]
#[require(Regen)]
#[component(on_add = heal_full)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}
```

Details that follow from that mapping:

- **Fields are emitted `pub`.** A construct declared in a block is meant to be
  used from the rest of the crate.
- **`requires` may repeat and accumulates.** `requires A, b::C` and two separate
  `requires` items produce the same single `#[require(...)]` list. Paths may be
  qualified.
- **Hooks are forwarded, not reimplemented.** The four keys map straight onto
  `#[component(on_add = …)]` and friends, so the derive's rules still hold —
  including its mutual exclusion with the runtime hook builder. A key may appear
  at most once; a duplicate is caught in the block, on the second key's span.
  See [Hooks & observers](../concepts/hooks-and-observers.md).
- **`no_bundle`** suppresses the automatic single-component `Bundle` impl, same
  as the hand-written attribute.
- **A fieldless `component Marker {}`** is a plain ZST struct — the derive's own
  auto-tag detection takes it from there. Prefer `tag` for that (below); the
  fieldless `component` form exists so a component you are still growing does not
  have to change keyword.

The hook forwarding is exercised end-to-end, not just parsed — the shipped test
`crates/aether_tests/tests/a0_component_tag.rs` binds a real Phase-14a hook fn
and asserts it fires exactly once per added `Health`.

## `tag`

```ebnf
tag := 'tag' IDENT ( '(' 'bitset' ')' )? ';'
```

A tag is a zero-data marker, and the declaration ends with `;` — tags have no
body at all. `(bitset)` is the only modifier.

```rust,ignore
aether! {
    tag Player;
    tag Stunned(bitset);
}
```

```rust,ignore
#[derive(::boyko_macros::Component)]
pub struct Player;                      // ZST => the derive's auto-tag path

#[derive(::boyko_macros::Component)]
#[component(storage = "bitset")]
pub struct Stunned;                     // EnableTag backend: O(1) toggle, no migration
```

> **Hazard — a `(bitset)` tag is not spawned.** The bitset backend keeps the tag
> in the [EnableTag](../concepts/enable-tags.md) store, not in archetype storage,
> so its API is `enable` / `is_enabled` / `disable`, never `spawn`. The first
> draft of the shipped A0 test spawned `Stunned` like a plain ZST and the kernel
> refused it. That refusal is the storage backend working correctly, and the test
> now pins the correct usage:
>
> ```rust,ignore
> world.enable::<Stunned>(entity);
> assert!(world.is_enabled::<Stunned>(entity));
> world.disable::<Stunned>(entity);   // no archetype migration either way
> ```

Any other modifier is refused by name — `tag T(dense);` reports *unknown tag
modifier `dense`; the only one is `bitset` (the EnableTag backend)*. That shape
also keeps the door open: when the kernel grows another storage backend, it
becomes one more accepted modifier and nothing else changes.

## `bundle`

```ebnf
bundle := 'bundle' IDENT '{' (IDENT ':' TYPE ','?)* '}'
```

```rust,ignore
aether! {
    bundle Projectile {
        pos: Position,
        vel: Velocity,
    }
}
```

```rust,ignore
#[derive(::boyko_macros::Bundle)]
pub struct Projectile {
    pub pos: Position,
    pub vel: Velocity,
}
```

That is the whole construct. The derive owns the named-struct-only rule, the
no-generics rule, and the static-cache codegen; Aether adds uniformity of
surface and exactly **one** pre-check: bundle arity is capped at 16
(`MAX_BUNDLE_ARITY`), and Aether reports it on the **17th field's own name**
rather than letting the error surface downstream with a worse span.

## `event`

```ebnf
event    := 'event' IDENT '{' ev_field* '}'
ev_field := IDENT ':' 'entity' '(' PATH (',' PATH)* ')'   (* a participant *)
          | IDENT ':' TYPE                                (* a parameter *)
```

The engine's events are split into **participants** (entities the event is
about, each carrying a component context) and **parameters** (the payload).
Aether makes the participant marker type-shaped instead of a stringly attribute:

```rust,ignore
aether! {
    event Damage {
        victim: entity(Position, Health),
        amount: f32,
    }
}
```

```rust,ignore
#[::boyko_macros::event]
pub struct Damage {
    #[participant(components = "Position, Health")]
    pub victim: ::boyko_ecs::ecs::core::entity::entity::Entity,
    #[parameter]
    pub amount: f32,
}
```

The `#[event]` attribute macro remains the layout authority: it performs the
two-band rewrite that turns those fields into `DamageParticipants` and
`DamageParameters` substructs. Aether inherits that, which means it also
inherits the **construction shape**:

```rust,ignore
w.send(Damage {
    participants: DamageParticipants { victim },
    parameters: DamageParameters { amount: 2.5 },
})
.expect("send within lane capacity");
```

Three rules to know:

- **The empty participant context is never defaulted.** `victim: entity` without
  a component list is an error: participants exist to name their component
  context, and the engine's contract wants it explicit.
- **Participant components are bare idents.** The derive's `components = "…"`
  channel is a comma-separated list of identifiers, so `entity(foo::Bar)` and
  `entity(Slot<A, B>)` are refused *in your block, on your tokens* — importing
  the component and naming it unqualified is the fix. Forwarding either shape
  would have crashed the downstream macro with no span at all.
- **`entity` is contextual.** It is only the participant marker as a bare ident
  in the type position; `thing: my::entity` is an ordinary parameter field of a
  type that happens to be called `entity`.

### Registering event lanes

```rust,ignore
app.world_mut()
    .preregister_event::<Damage>(EventConfig::default_for(2).expect("config"))
    .expect("preregister");
```

Declaring an event is not registering its lanes; the engine has no
`App::add_event`, and an `EventWriter<E>` / `EventReader<E>` for an unregistered
type panics with a message naming it. See
[App & Plugins](../app/plugins.md#events-there-is-no-appadd_event).

> **Hazard — a zero-sized event does not compile.** The dispatcher carries a
> `const` guard: *"Event type is zero-sized; use a counter instead (add a
> non-ZST field)"*. An Aether `event Ping {}` has empty participants **and**
> empty parameters, so the whole type is a ZST and trips that guard at
> monomorphisation. Give every event at least one parameter field — the shipped
> A3 test's signal events all carry a `tick: u32` for exactly this reason.

## What this costs

Nothing at run time. Every item above is byte-for-byte the item a disciplined
engineer writes by hand, so the performance tables on
[Components](../concepts/components.md#performance-characteristics),
[Bundles](../concepts/bundles.md) and [Events](../concepts/events.md) apply
unchanged — there is no Aether-specific cost to look up. At compile time you pay
one small parse and an emission roughly the size of the source you typed; the
heavy codegen belongs to the derives, which the crate already pays for.

## See also

- [Aether overview](overview.md) — the macro, the crates, and the shipped rungs.
- [Systems & plugins](systems-and-plugins.md) — the constructs that *use* this
  data.
- [Diagnostics](diagnostics.md) — the full error contract for these constructs.
- [Components](../concepts/components.md), [Tags](../concepts/tags.md),
  [Enable tags](../concepts/enable-tags.md), [Bundles](../concepts/bundles.md),
  [Events](../concepts/events.md) — the hand-written surface.
- Source: `crates/aether_lang/src/parse.rs`, `crates/aether_lang/src/expand.rs`;
  runnable examples in `crates/aether_tests/tests/a0_component_tag.rs` and
  `crates/aether_tests/tests/a1_bundle_event.rs`.
