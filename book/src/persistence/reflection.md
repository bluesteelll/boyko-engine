# Reflection (Editor Builds)

> **Experimental.** `boyko_reflect` describes a component's fields at run time, for editors and
> inspectors. It exists only in builds that turn on a `reflect` feature; a shipped game does not
> contain it.

## What it is

Runtime reflection answers questions such as "which fields does this component have, at which
byte offsets, and what is the value of field 2?". An editor needs those answers; a game loop does
not. [`boyko_reflect`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_reflect)
therefore lives behind a Cargo feature that every consumer names `reflect`. With the feature off,
the crate is not in the dependency graph at all: no code, no symbols, no tables.

Reflection is not how the engine saves worlds. [Serialization](serialization.md) is compile-time
code generation, and `boyko_serialize` must not depend on `boyko_reflect`.

The engine's own components (`Transform`, `Visibility` and so on) are not annotated yet.
`boyko_scene` and `boyko_render` already declare the non-default `reflect` feature that their
annotations will sit behind.

## Opting in

Make the dependency optional and name the feature `reflect`:

```toml
[dependencies]
boyko-ecs = { git = "https://github.com/bluesteelll/boyko-engine" }
boyko-macros = { git = "https://github.com/bluesteelll/boyko-engine" }
boyko-reflect = { git = "https://github.com/bluesteelll/boyko-engine", optional = true }

[features]
reflect = ["dep:boyko-reflect"]
```

Then annotate the components an inspector should see:

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_macros::Component;

#[derive(Component, Default, Clone, Copy)]
#[component(reflect)]
#[repr(C)]
struct Health {
    current: f32,
    max: f32,
}

#[cfg(feature = "reflect")]
fn inspect() {
    let info = boyko_reflect::type_info_of(Health::component_id().0)
        .expect("Health is annotated and the `reflect` feature is on");
    for field in info.fields {
        // Prints "current: Prim(F32) at byte 0", then "max: Prim(F32) at byte 4".
        println!("{}: {:?} at byte {}", field.name, field.kind, field.offset);
    }
}
```

What `#[component(reflect)]` does:

- It emits a `static TypeInfo` and an `impl boyko_reflect::Reflect` behind
  `#[cfg(feature = "reflect")]`. The `cfg` is evaluated in **your** crate, so the feature must be
  declared there; `boyko_macros` itself never depends on `boyko_reflect`.
- Every offset is computed with `core::mem::offset_of!`.
- Registering the component installs its `TypeInfo` under the component's id, so
  `type_info_of(id)` finds it.
- The type must implement `Default`, which supplies the inspector's "add with defaults" value. Opt
  out with `#[reflect(no_default)]`.
- A field must have a type the model can describe: a primitive, an array `[T; N]` of primitives,
  or a nested reflected struct. Any other field type (a `Vec`, a handle) is a compile error unless
  the field carries `#[reflect(skip)]`. A skipped field keeps its entry in the field list, with
  no accessors, so field indices stay stable.
- Fieldless enums need an integer `#[repr]`. Data-carrying enums, unions and bitset enable tags
  cannot be reflected: each is a compile error with a message that says why.
- Tuple structs work, but their field names are `"0"`, `"1"`, ..., so reordering their fields
  re-binds the names silently. Prefer named fields.

## The data model

| Item | What it holds |
|------|---------------|
| `TypeInfo` | Type name, size, alignment, kind, the field list and, for an enum, its variants. |
| `FieldInfo` | Field name, byte offset, `ValueKind`, and optional `get` / `set` accessors. |
| `Scalar` | A 16-byte value cell that carries a primitive value and its `ScalarKind`. |
| `NestedCursor` | Steps into a nested struct field without copying it. |

Every accessor is an `Option`: a kind without a given accessor has `None` there, never a stub.
`boyko_reflect::validate` checks a descriptor's coherence, for example that a nested field is stored
inline and that nesting never forms a cycle.

## Inspecting an entity

The module `boyko_reflect::ecs` connects the model to the world:

- `components_of_into(&ecs, entity, &mut buffer)` lists an entity's component ids into a buffer
  you provide, without allocating. It reports `BufferTooSmall` instead of truncating.
- `display_name(id)` returns a component's type name, or a fixed placeholder for an unregistered
  id.

## Status

The crate is experimental and not published. The workspace keeps it out of every shipping build:
nothing enables a `reflect` feature except the `reflect-dogfood` test package and explicit
command lines.

## See also

- [Components](../concepts/components.md): the `#[component(...)]` attribute.
- [Serialization](serialization.md): the compile-time save/load path.
- Source: [`crates/boyko_reflect`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_reflect)
  and the derive half in [`boyko_macros/src/reflect.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_macros/src/reflect.rs).
