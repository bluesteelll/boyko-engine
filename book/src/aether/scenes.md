# Scenes

The `scene` construct declares a **world**: meshes, lights, cameras, SDF edits
and hand-assembled entities, as a tree. It expands to one **spawn function**
whose `SystemParam` signature is exactly the one the engine's shipped scenes are
written with — and, when the block has a `plugin` header, a startup registration
for it.

This is the rung the rest of the language was built toward. A scene names its
sibling `material`s, binds its own meshes, and lowers every node to bundles that
already exist. Nothing here is a new runtime concept: after expansion, the file
is the `commands.spawn(…)` sequence a graphics programmer would have typed.

## Grammar

```ebnf
scene      := 'scene' IDENT '{' scene_item* '}'
scene_item := mesh_let | node
mesh_let   := 'let' IDENT '=' mesh_src ';'
mesh_src   := 'plane' '(' EXPR ')' | 'cube' '(' EXPR ')'
            | 'mesh' '(' EXPR ',' EXPR ')'          (* (&[Vertex], &[u32]) *)
node       := node_head ('at' EXPR)? ('{' node_body? '}')? ';'?
node_head  := 'mesh' IDENT                          (* a mesh_let binding *)
            | 'sun' | 'spot' | 'point' | 'sky' | 'camera'
            | 'sdf' EXPR                            (* an SdfEdit expression *)
            | 'entity'                              (* bare spawn, the escape hatch *)
node_body  := prop (',' prop)*
prop       := 'material' ':' IDENT                  (* a sibling `material` name *)
            | 'casts_shadow'
            | 'children' ':' '[' node (',' node)* ']'
            | EXPR                                  (* any extra component expression *)
```

Scene names are **lowercase** — they expand to spawn fns, so an UpperCamelCase
name would read like a type at the `add_startup_system` call site.

## The lab scene

This is the plan's flagship example, and it is the shipped test verbatim — a
compression of the engine's hand-written `vb_lab` setup:

```rust,ignore
aether! {
    plugin VbLab;

    material gold { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
    material lamp { base: (0.02, 0.02, 0.02), roughness: 0.6, emissive: (1.6, 0.9, 0.3) }

    scene lab {
        let floor = plane(22.0);
        let block = cube(1.0);

        mesh floor;
        mesh block at Transform { translation: Vec3::new(0.0, 3.0, -4.5),
                                  rotation: Quat::IDENTITY,
                                  scale: Vec3::new(14.0, 6.0, 0.4) };
        mesh block at (-2.4, 0.5, -2.2) { material: gold, casts_shadow };
        mesh block at (-4.4, 1.4, -1.0) { material: lamp };

        sdf SdfEdit::sphere([3.2, 0.85, 1.8], 0.85, sdf_op::UNION, 0.0);

        sun { dir: (-0.42, 0.80, 0.42), color: (1.0, 0.97, 0.92), lux: 3.2 }
        sky { sky: (0.28, 0.36, 0.50), ground: (0.15, 0.14, 0.13) }
    }
}
```

### What it becomes

```rust,ignore
/// Aether scene `lab` — the spawn fn.
pub fn lab(
    mut __aether_commands: Commands,
    mut __aether_meshes: NonSendResMut<Assets<MeshGpu>>,
    mut __aether_materials: ResMut<Assets<Material>>,
    __aether_dev: NonSendRes<GpuDevice>,
) {
    let floor = MeshAssetsExt::plane(&mut *__aether_meshes, __aether_dev.get(), 22.0);
    let block = MeshAssetsExt::cube(&mut *__aether_meshes, __aether_dev.get(), 1.0);
    let __aether_mat_gold = __aether_materials.add(gold());
    let __aether_mat_lamp = __aether_materials.add(lamp());

    __aether_commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    __aether_commands.spawn(MeshBundle::new(block, Transform { /* your tokens, verbatim */ }));
    __aether_commands.spawn(MeshBundle::new(
        block,
        Transform::from_translation(Vec3::new(-2.4, 0.5, -2.2)),
    ))
    .insert(ShadowCaster)
    .insert(MaterialHandle(__aether_mat_gold.index() as u16));
    // … the lamp block, the SDF edit, the sun, the sky …
}
```

(Engine paths elided; the real emission is fully qualified, and the whole thing
is pinned token-for-token by `the_section_3_7_before_after_pair_holds_verbatim`.
Note what that pin owns and no behavior test could: the `at Transform { … }`
node keeps **your** bare `Transform` / `Vec3` / `Quat` spellings, while the node
that gave no pose receives Aether's own qualified `Transform::IDENTITY`.) The
plugin gets one line:

```rust,ignore
app.add_startup_system(lab);
```

Four things in that expansion are worth their own sections: where the parameter
list came from, why the mints sit at the top, why the params are
`__aether_`-prefixed, and what `MaterialHandle(… .index() as u16)` is doing.

## Mesh bindings are scene-scoped

`let floor = plane(22.0);` registers a mesh **once per scene run** and binds it
to a name that `mesh floor` nodes resolve against. Three sources ship:

| You write | It calls |
|-----------|----------|
| `plane(SIZE)` | `MeshAssetsExt::plane(meshes, dev.get(), SIZE)` |
| `cube(SIZE)` | `MeshAssetsExt::cube(meshes, dev.get(), SIZE)` |
| `mesh(VERTICES, INDICES)` | `MeshAssetsExt::register_mesh(meshes, dev.get(), VERTICES, INDICES)` |

The binding table belongs to **one scene**, not to the block — and the
diagnostic says so, because a message that misstated the scope would be a lie
the reader can only disprove by hunting. The shipped golden makes that concrete
with two scenes:

```rust,ignore
aether! {
    scene lab {
        let floor = plane(22.0);
        let block = cube(1.0);

        mesh floor;
        mesh block;
    }

    scene props {
        let crate_box = cube(1.0);

        mesh floor;                 // declared in the block — but not in THIS scene
    }
}
```

```text
error: no mesh binding `floor` in scene `props` (bindings here: `crate_box`)
  --> tests/ui/scene_unknown_mesh_binding.rs:21:14
   |
21 |         mesh floor;
   |              ^^^^^
```

Compare that with the `material:` prop, which *is* a block symbol: its message
reads ``no material `gol` in this aether block (materials here: `gold`, `lamp`)``.
Two symbol tables, two extents, two wordings — see
[AetherCtx](#aetherctx-the-blocks-symbol-table).

Two bindings of one name are refused (``duplicate mesh binding `a` in this
scene``); silently retargeting every `mesh a` below the second is not a fault
you would find by reading.

## The eight heads

| Head | Spawns | Takes `at` | `material:` | `casts_shadow` |
|------|--------|:----------:|:-----------:|----------------|
| `mesh NAME` | `MeshBundle::new(binding, pose)` | yes | yes | `ShadowCaster` |
| `sun` | `DirectionalLightObject` | no | no | — |
| `spot` | `SpotLightObject` | no | no | `CastsPunctualShadow` |
| `point` | `PointLightObject` | no | no | `CastsPunctualShadow` |
| `sky` | `SkyLight` | no | no | — |
| `camera` | `CameraRig` | yes | no | — |
| `sdf EXPR` | `SdfPrimitive(EXPR)` | no | no | — |
| `entity` | `SpatialBundle` with `at`, `spawn_empty()` without | yes | yes | `ShadowCaster` |

An unknown head answers with the whole registry:

```text
error: unknown scene node `sunn`; heads are: mesh, sun, spot, point, sky, camera, sdf, entity (did you mean `sun`?)
```

### Key tables

Each keyed head carries one table — spelling, value shape and requiredness
together, the same discipline [`material`](materials.md#one-table-not-two-lists)
uses, so the list a diagnostic prints and the set the parser accepts cannot
diverge:

| Head | Keys | Required | Defaults |
|------|------|----------|----------|
| `sun` | `dir`, `color`, `lux` | `dir`, `lux` | `color` → white |
| `sky` | `sky`, `ground` | **both** | — |
| `point` | `pos`, `color`, `power`, `range` | `pos`, `power`, `range` | `color` → white |
| `spot` | `pos`, `dir`, `color`, `power`, `range`, `inner`, `outer` | all but `color` | `color` → white |
| `camera` | `fov`, `aspect`, `near`, `far` | `aspect` | `fov` 60° · `near` 0.1 · `far` 1000.0 |

Requiredness follows the rule [`material`'s `base:`](materials.md#the-keys)
established: a key whose engine parameter has **no honest neutral value** is
refused rather than invented. `sky`'s two hemispheres are the clearest case — a
black ground and a white ground light a scene differently, so neither can
default. `camera`'s `aspect` is the render target's width over height, which
Aether cannot know. White *is* a right answer for `color`, so `color` defaults.

```text
error: the `sun` node needs a `dir:` key — it has no default (these default: color)
error: the `camera` node needs an `aspect:` key
```

Two details you can see in the emission: `fov` is authored in **degrees** and
converted by a multiply — `.to_radians()` on a bare float literal would infer
`f64` and fail against the `f32` field — and a tuple key is exactly three
components (``` `sun` key `dir` takes exactly 3 components (x, y, z) — found 2 ```).

An unknown key prints its head's table *and* the universal props:

```text
error: unknown `sun` key `dirr`; keys are: dir, color, lux (plus material, casts_shadow, children) (did you mean `dir`?)
```

## Poses, and where `at` is refused

`at` takes a `Transform` expression, with a translation sugar:

```rust,ignore
mesh block at (-2.4, 0.5, -2.2) { … }      // Transform::from_translation(Vec3::new(…))
mesh block at Transform { … }              // verbatim, your tokens
camera at (0.0, 2.1, 8.4) { aspect: … }
```

Only `mesh`, `camera` and `entity` take one. The light heads derive their whole
pose from their own keys — `sun` from `dir` (look-at plus `Quat::from_mat3`,
exactly as the shipped scenes do), `spot` and `point` from `pos` — and an `sdf`
edit carries its world-space position *inside the edit*. On those heads an `at`
would be **silently dropped**, which is the one failure mode a user cannot see
in the rendered frame without hunting for it. So it is refused, and the message
says where the pose really comes from:

```text
error: the `sun` node derives its whole pose from `dir:` (look-at + `Quat::from_mat3`, exactly as the shipped scenes do) — an `at` here would be dropped
error: the `sky` node is a hemisphere fill with no pose — an `at` here would be dropped
error: an `sdf` edit carries its WORLD-SPACE position inside the edit itself (v1 reads no `Transform`) — an `at` here would be dropped
```

> **Hazard — a bare path before a node body parses as a struct literal.**
> `at MY_POSE { material: gold }` reads as the struct literal `MY_POSE { … }`,
> exactly as it would in a Rust `if` scrutinee, and the node body disappears
> into it. Parenthesize to split them: `at (MY_POSE) { material: gold }`. A
> single parenthesized expression is passed through untouched (only a 3-tuple is
> the translation sugar), so this costs nothing else. The same applies to
> `sdf MY_EDIT { … }`. A call expression — `at pose_of(x)`, `sdf
> SdfEdit::sphere(…)` — cannot be continued by a brace and is never affected.
>
> **You do not have to recognize this yourself.** Since rung A7 the resulting
> error names it: a required-key refusal on a node whose pose is a bare path
> carries a note saying the braces were parsed as a struct literal, and gives
> you the parenthesized form. The note is *gated* on that shape, so an honest
> `at Transform { … }` node missing an unrelated key gets no false lead. See
> [Diagnostics](diagnostics.md#the-hint-for-a-pose-that-ate-its-own-body).

## Props

Every head accepts the same four props in its body:

- **`material: NAME`** — a sibling `material` construct. Aether mints the asset
  and narrows the row index into the render carrier for you.
- **`casts_shadow`** — `ShadowCaster` on `mesh`/`entity`, `CastsPunctualShadow`
  on `spot`/`point`. On a head with neither form it is refused: ``the `sky` node
  has no shadow-caster form``.
- **`children: [ node, … ]`** — nested nodes, parented through the kernel's own
  command.
- **a bare component expression** — inserted after the bundle. This is the
  escape hatch, and it is [never poorer than the sugar](#the-escape-hatch).

`children:` is the one shape that cannot use the chained statement form, because
a parent must hand its `Entity` to `add_child`. Only the nodes that need an id
bind one:

```rust,ignore
scene rig {
    entity at (0.0, 0.0, 0.0) {
        Root,
        children: [
            entity { LeftArm },
            entity at (1.0, 0.0, 0.0) { RightArm, children: [ entity { Hand } ] }
        ]
    };
}
```

```rust,ignore
let __aether_e0 = __aether_commands.spawn(SpatialBundle { … }).insert(Root).id();
let __aether_e1 = __aether_commands.spawn_empty().insert(LeftArm).id();
__aether_commands.add_child(__aether_e0, __aether_e1);
// … RightArm, Hand, then add_child(e0, e2)
```

`Commands::add_child` inserts `ChildOf`, and the reverse `Children` collection
is maintained by that component's own hooks — Aether writes `Children` no more
than your code does. See [Hierarchies](../concepts/hierarchies.md).

## Demand-driven parameters

The spawn fn's signature is computed from what the body **uses**:

| The scene contains | The fn gains |
|--------------------|--------------|
| always | `Commands` |
| any `let … = plane/cube/mesh(…)` | `NonSendResMut<Assets<MeshGpu>>` **and** `NonSendRes<GpuDevice>` |
| any `material:` prop | `ResMut<Assets<Material>>` |

A scene with neither compresses to `(commands)` alone, which is why a
pure-`entity` scene drags neither asset table nor device into its signature —
and why it runs headless:

```rust,ignore
scene props {
    entity at (1.0, 0.0, 2.0) { Health { hp: 10.0 } };
    entity { Marker, Tally(3) };
}
```

```rust,ignore
pub fn props(mut __aether_commands: Commands) {
    __aether_commands.spawn(SpatialBundle { /* placed anchor */ }).insert(Health { hp: 10.0 });
    __aether_commands.spawn_empty().insert(Marker).insert(Tally(3));
}
```

### Why the params are `__aether_`-prefixed

The design plan spells those four params `commands`, `meshes`, `materials` and
`dev`. A reviewer's probe showed what that costs: a scene containing
`let dev = plane(1.0);` shadows the device param, and rustc reports
`no method `get` on MeshHandle` **with both labels on the whole `aether!`
token** — no user token named anywhere.

Prefixing does not diagnose that fault. It deletes it: `__aether_dev` cannot
collide with a name you would write, so all four plan names are yours to bind.
The probe is now a compiled regression surface — the shipped `annex` scene
really does write `let dev = cube(0.5);`, and the pinned expansion is the whole
fn rather than a substring search, because the failure mode is a param and a
binding agreeing on a name.

## Material mints are hoisted, in declaration order

One material named on forty nodes mints **one** asset row. The mints sit at the
top of the fn, and their order is the **block's material declaration order** —
not the order the nodes happen to reference them:

```rust,ignore
material gold { base: (1.0, 0.72, 0.30) }
material chalk { base: (0.86, 0.86, 0.88) }

scene row {
    let cube_mesh = cube(1.0);
    mesh cube_mesh { material: chalk };      // chalk is referenced first…
    mesh cube_mesh { material: gold };
    mesh cube_mesh { material: chalk };
}
```

```rust,ignore
let __aether_mat_gold = __aether_materials.add(gold());     // …but gold is declared first
let __aether_mat_chalk = __aether_materials.add(chalk());
```

That ordering is a defect the pin caught before the code shipped. Collecting
references in **first-use** order makes the emitted mint sequence a function of
node order, so moving two nodes past each other silently renumbers every asset
row the scene mints — and asset row indices are what the render carrier stores.
Declaration order makes the emission stable under an edit that only moves nodes.

The narrowing at the end of each `.insert(…)` is the other half of the seam:

```rust,ignore
.insert(MaterialHandle(__aether_mat_gold.index() as u16));
```

An `Assets<Material>` row index is a `u32`; the render carrier
[`MaterialHandle`](../concepts/assets.md#the-render-carrier) is 16 bits. Writing
that narrowing by hand at every prop is exactly what the construct exists to
hide. The shipped E2E follows the whole path — sibling `material` resolves, its
builder fn is called, `Assets::add` mints, the index narrows — by resolving each
spawned handle **back** to its asset and comparing base colors, because a handle
pointing at the wrong row type-checks perfectly and lights the prop wrong.

## Registration

With a `plugin` header, each scene is registered as a startup one-shot, and
scenes and startup systems keep **block source order** across both kinds:

```rust,ignore
aether! {
    plugin Boot;
    system early() on startup { }
    scene arena { entity { Floor }; }
    system late() on startup { }
}
```

```rust,ignore
app.add_startup_system(early);
app.add_startup_system(arena);
app.add_startup_system(late);
```

Registering all systems first and all scenes after would type-check identically
and reorder your frame, which is why the order is pinned by a test rather than
left to the emission's shape. A block with no plugin emits the spawn fn and
leaves registration to you — the same contract a clause-free `system` has.

## The escape hatch

`entity` is the universal fallback, and the rule it lives by is that **the
escape hatch is never poorer than the sugar**. `entity` therefore accepts
`material:` and `casts_shadow` like a `mesh` node does — an `entity` carrying a
`MeshHandle` component expression is a drawable prop assembled by hand, and
refusing it the `material:` prop would force you to spell the `u16` narrowing
yourself.

Bare component expressions are inserted **after** the bundle, so they can also
overwrite a field the sugar filled. That is how a second camera gets its own
draw order without any new syntax:

```rust,ignore
camera at (0.0, 2.1, 8.4) { aspect: 1120.0 / 720.0, fov: 52.0, far: 120.0 }

camera at (0.0, 2.1, -8.4) {
    aspect: 1120.0 / 720.0,
    Camera { order: 1, ..Camera::DEFAULT }      // the head fills Camera::DEFAULT; this wins
}
```

An orthographic projection is reached the same way, through
`entity { CameraRig { … } }`. Sugar is additive over the escape hatch, never a
wall around it.

## AetherCtx: the block's symbol table

A6 is where §4's symbol table became a real module. `AetherCtx` is built between
parse and expand, carries every sibling `material` in declaration order, and
runs the whole-block rules: duplicate fn-producing names, one `plugin` per
block, and the requirement that scheduling clauses and machines have a plugin to
hold their registrations.

It is deliberately narrow — it holds the one symbol class something actually
resolves against today, because a table row nothing reads is a datum that rots.
`system` ordering and `plugin` collection each walk the construct list at their
own site and get no entry here.

The duplicate-name rule is where A6 widened an A5 measurement rather than adding
a case. Constructs that expand to a bare `pub fn` — `system`, `material`,
`scene` — collide in a way rustc reports with **no user token at all**, so
Aether owns those, across kinds as well as within one:

```text
error: `lab` is declared twice in this aether block — the `material` and the `scene` both expand to a fn of that name
  --> tests/ui/scene_collides_with_a_material_fn.rs:10:11
   |
10 |     scene lab {
   |           ^^^

error: the first `material` of this name is here
 --> tests/ui/scene_collides_with_a_material_fn.rs:8:14
  |
8 |     material lab { base: (0.0, 0.0, 0.0) }
  |              ^^^
```

Type-producing constructs still defer to rustc: they carry a derive, so rustc
reports the duplicate **and** a second, localized error against your own item. A
duplicated check there could only be worse, and duplicated checks drift.

## How the scene surface is gated

Two halves, and neither can do the other's job:

- **Token pins** in `aether_lang` say what Aether meant to emit — argument
  count, argument order, which key lands in which slot, the synthesized
  defaults. That crate has no engine dependency, so no assertion in it can
  notice that `SpotLight::new` grew a parameter. (An earlier comment in the
  tree claimed it could. It was wrong in the "gate that cannot fail"
  direction — the campaign's signature defect.)
- **A compiled scene** in `aether_tests` says the engine still accepts the
  emission. `add_startup_system(lab)` requires `IntoSystem`, which type-checks
  the entire generated body, so a changed constructor breaks in-repo the same
  day. A mesh binding needs a live device, so that app is never updated — the
  registration *is* the assertion. Alongside it, a device-free scene of
  `entity`, `sun`, `sky`, `point` and `sdf` nodes actually runs and reads the
  world back through ordinary queries.

An emission path absent from that file is a path no compiler ever sees, which is
why the shipped tests cover the surface by construction: the plan's `lab` scene
verbatim, an `annex` scene for `spot` / `camera` / `mesh(&V, &I)` /
`CastsPunctualShadow` / the second-camera escape, and a running `arena` for
`entity`, `children:` and the material seam.

## See also

- [Materials](materials.md) — the sibling construct a scene's `material:` prop
  resolves against.
- [Assets & handles](../concepts/assets.md) — `Assets<T>`, `Handle<T>`, and the
  16-bit render carrier the narrowing produces.
- [Hierarchies](../concepts/hierarchies.md) — what `children:` builds.
- [Diagnostics](diagnostics.md) — the scene error contract in full.
- [Rendering overview](../rendering/overview.md) — what consumes the components
  a scene spawns.
- Source: `crates/aether_lang/src/parse.rs` (the node grammar),
  `crates/aether_lang/src/expand.rs` (`scene_fn` and the per-head lowering),
  `crates/aether_lang/src/ctx.rs` (`AetherCtx`),
  `crates/aether_tests/tests/a6_scene.rs`, goldens in
  `crates/aether_tests/tests/ui/scene_*.rs`.
