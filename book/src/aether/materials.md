# Materials

The `material` construct declares a PBR material as a **builder function** over
the engine's own constructors. `material gold { … }` becomes an `#[inline] pub
fn gold() -> Material` and nothing else: no table, no registry, no lazy static,
no `Assets` call of its own.

That shape is forced by what materials *are* in this engine. A material is a
**runtime-minted asset** — you call `Assets::<Material>::add` and get a
`Handle<Material>` back — so a static table inside Aether would be exactly the
parallel data system [Principle 0](../architecture/principles.md) forbids. A fn
that returns the engine's `Material` costs nothing, composes with any minting
site, and leaves the asset system as the single authority.

## Grammar

```ebnf
material  := 'material' IDENT '{' mat_key* '}'
mat_key   := 'base'        ':' color ','?
           | 'metallic'    ':' EXPR  ','?    | 'roughness' ':' EXPR ','?
           | 'reflectance' ':' EXPR  ','?    | 'emissive'  ':' color ','?
           | 'flags'       ':' EXPR  ','?
           | 'textures'    ':' EXPR  ','?    (* escape: a MaterialTextures expression *)
color     := '(' EXPR ',' EXPR ',' EXPR (',' EXPR)? ')'
```

Material names are **lowercase** — they expand to fns, so an UpperCamelCase name
would read like a type at every call site. Every value is a verbatim Rust
expression with your own spans, so `metallic: BRASS_METALLIC` and
`roughness: cfg.rough` work exactly like literals.

## A material and what it becomes

```rust,ignore
aether! {
    material gold  { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
    material lamp  { base: (0.02, 0.02, 0.02), roughness: 0.6, emissive: (1.6, 0.9, 0.3) }
}
```

```rust,ignore
/// Aether material `gold`.
#[inline]
pub fn gold() -> ::boyko_render::Material {
    ::boyko_render::Material::new([1.0, 0.72, 0.30, 1.0], 1.0, 0.14, 0.5, [0.0; 3], 0)
}

/// Aether material `lamp`.
#[inline]
pub fn lamp() -> ::boyko_render::Material {
    ::boyko_render::Material::new([0.02, 0.02, 0.02, 1.0], 0.0, 0.6, 0.5, [1.6, 0.9, 0.3], 0)
}
```

That is the whole expansion, pinned token-for-token by
`the_section_3_6_before_after_pair_holds_verbatim`. Two details are visible in
it:

- **The alpha lane is synthesized.** A three-component `base` gets `1.0`
  appended, because `MaterialGpu::base_color` is a `vec4`. Write four components
  and yours passes through untouched.
- **`::boyko_render::Material` is the real path.** `boyko_render` re-exports
  `Material` and `MaterialGpu` at its crate root, so — unlike the deeply nested
  `Res` / `Entity` paths the other constructs emit — the obvious spelling
  resolves. Your crate needs `boyko_render` as a dependency to use `material`.

## The keys

Seven keys, in `Material::new`'s own parameter order, so the "expected one of"
list reads as the constructor you are filling in:

| Key | Takes | Default | Notes |
|-----|-------|---------|-------|
| `base` | `(r, g, b)` or `(r, g, b, a)` | **required** | linear base color; alpha `1.0` when you give three |
| `metallic` | expression | `0.0` | |
| `roughness` | expression | `0.5` | |
| `reflectance` | expression | `0.5` | the standard 4% F0 scale |
| `emissive` | `(r, g, b)` — exactly three | `[0.0; 3]` | linear emitted radiance |
| `flags` | expression | `0` | the `MaterialGpu` flag word |
| `textures` | expression | *absent* | a `MaterialTextures` value; see below |

Three rules the table does not show:

- **`base` has no default, and Aether refuses to invent one.** Every other key
  has a value the engine's shipped scenes agree on; a base color does not. The
  error names the whole default table so you can see what you *are* getting for
  free.
- **A repeated key is an error, not last-write-wins.** `base: … , base: …`
  reports on the second key. Silently dropping the first value is the kind of
  bug you find in a screenshot.
- **`emissive` takes exactly three components.** `Material::new` takes
  `emissive: [f32; 3]`; emitted radiance has no alpha. A fourth component would
  otherwise fail in rustc against a *synthesized* array that carries no span of
  yours.

### One table, not two lists

The key list the diagnostic prints and the key set the parser accepts are the
same rows of one `const MATERIAL_KEYS: &[(&str, MatKey)]`. That is a deliberate
repair of a shape used elsewhere in the crate — a `&[&str]` for the message
beside a `from_str` match for the dispatch. Two hand-maintained lists drift in
two directions: a key only in the message is advertised and then rejected as
unknown, and a key only in the match is accepted but never named in the
"expected one of". With one table, both failures are unrepresentable.

The other half of that guarantee is a test: `every_advertised_key_reaches_the_emission`
sets all seven keys to non-default values in one material and pins the emission.
A key the parser *accepts* but never threads through would otherwise ship a
material whose `reflectance` you believed you had set.

## The `textures:` escape

Naming `textures:` switches the emission to the engine's textured constructor:

```rust,ignore
aether! {
    material crate_box {
        base: (0.8, 0.8, 0.8, 0.5),
        metallic: BRASS_METALLIC,
        roughness: 0.3,
        reflectance: 0.35,
        flags: 0,
        textures: MaterialTextures { albedo: slot, ..MaterialTextures::NONE },
    }
}
```

```rust,ignore
/// Aether material `crate_box`.
#[inline]
pub fn crate_box() -> ::boyko_render::Material {
    ::boyko_render::Material::with_textures(
        ::boyko_render::MaterialGpu::new(
            [0.8, 0.8, 0.8, 0.5], BRASS_METALLIC, 0.3, 0.35, [0.0; 3], 0
        ),
        MaterialTextures { albedo: slot, ..MaterialTextures::NONE }
    )
}
```

`Material::with_textures` is the **only** constructor in the engine that can
produce a textured material, and it is the only place `MATERIAL_FLAG_TEXTURED`
is derived. Routing through it keeps that derivation in the engine's one
authority — **Aether never mints that bit itself**, and the shipped E2E asserts
the bit rather than the tokens, which is what makes the claim falsifiable.

## Minting a handle

A builder fn is a value, so minting is ordinary asset code — a startup system
that takes `ResMut<Assets<Material>>` and keeps the handles wherever your game
keeps them:

```rust,ignore
use aether::aether;
use boyko_ecs::App;
use boyko_ecs::ecs::core::asset::{Assets, Handle};
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;

aether! {
    material gold { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
    material lamp { base: (0.02, 0.02, 0.02), roughness: 0.6, emissive: (1.6, 0.9, 0.3) }
}

/// Aether materials are plain fns (no captures), so a resource carries the handles out.
#[derive(boyko_macros::Resource)]
struct Minted {
    gold: Option<Handle<Material>>,
    lamp: Option<Handle<Material>>,
}

fn main() {
    let mut app = App::new();
    // The windowed host inserts this at boot; a bare `App` does not, so insert it yourself.
    app.insert_resource(Assets::<Material>::with_reserved(8));
    app.insert_resource(Minted { gold: None, lamp: None });

    app.add_startup_system(|mut materials: ResMut<Assets<Material>>, mut out: ResMut<Minted>| {
        out.gold = Some(materials.add(gold()));
        out.lamp = Some(materials.add(lamp()));
    });

    app.update();
}
```

Under [the windowed host](../app/windowed-host.md) the `Assets<Material>`
resource already exists — the runner creates it at boot and mints slot 0 as the
engine default material, so a hit that carries no explicit material always
resolves to a valid row. Your startup systems mint on top of that.

What the handle points at is a 48-byte `MaterialGpu`, three `std430` `vec4`
lanes the shader reads directly:

| Lane | Field | Contents |
|------|-------|----------|
| 0 | `base_color` | `rgb` linear base color, `w` alpha |
| 1 | `mrr` | `[metallic, roughness, reflectance, bitcast(flags)]` |
| 2 | `emissive` | `rgb` linear emitted radiance, `w` unused |

The shipped E2E pins **every one of those lanes**, including the whole `mrr[3]`
bit pattern rather than just the textured bit — a drifted flags default (`0` →
`2`) would satisfy a mask check while every other assertion stayed green. It is
also why `gold` carries `roughness: 0.14` next to a defaulted `reflectance: 0.5`:
`Material::new` takes three `f32` scalars in a row, so a transposition
type-checks perfectly, and only two *different* numbers in adjacent slots can
catch it.

That test is the construct's anti-drift gate, and it is live: `aether-tests`
depends on `boyko_render` precisely so that the day `Material::new` gains a
seventh parameter, the A5 tests stop compiling **in this repo** instead of in
someone's game.

## What A5 deliberately does not do

- **No plugin registration.** A material carries no scheduling, so it needs no
  `plugin` header — and a block that has one is unaffected: the plugin collects
  sibling *systems*, never materials. Pinned by
  `a_material_needs_no_plugin_and_a_sibling_plugin_does_not_register_it`.
- **No handles on entities.** Aether gives you `gold()`; putting the resulting
  handle on an entity as a `MaterialHandle` is scene work, and `scene` is
  [rung A6](overview.md#status-and-what-is-next).
- **No shading math.** `material` parameterizes the frozen `MaterialGpu` that
  the one Cook-Torrance BRDF consumes. Custom surface math belongs to
  [the shader eDSL](../rendering/shader-edsl.md), which owns its own emission
  gates. A post-v1 `shader` construct is the designed seam between them.
- **No pre-check for cross-kind name collisions.** `material paint` beside
  `component Paint` expands untouched — see below.

## Refusals

| You write | Aether says |
|-----------|-------------|
| `material Gold { … }` | ``material names are lowercase — they expand to builder functions, not types (rename `Gold` to `gold`)`` |
| `base: (1.0, 0.72)` | ``` `base` color takes 3 (rgb, alpha=1.0) or 4 (rgba) components — found 2 ``` |
| `emissive: (1.0, 0.5, 0.2, 1.0)` | ``` `emissive` color takes exactly 3 components (rgb) — `Material::new` takes `emissive: [f32; 3]`, emitted radiance has no alpha — found 4 ``` |
| `base: 0.5` | ``` `base` takes a color tuple: `(r, g, b)` or `(r, g, b, a)` ``` |
| `roughnes: 0.14` | ``unknown material key `roughnes`; keys are: base, metallic, roughness, reflectance, emissive, flags, textures (did you mean `roughness`?)`` |
| `base: …, base: …` | ``duplicate material key `base` `` |
| no `base` key | ``` material `m` needs a `base:` color — every other key defaults (metallic 0.0, roughness 0.5, reflectance 0.5, emissive (0.0, 0.0, 0.0), flags 0), the base color does not ``` |
| two `material twice` | ``duplicate material `twice` — each material expands to a builder fn of its own name, and two of one name is one fn defined twice`` + *the first `material` of this name is here* |

The arity error lands on the **tuple**, because neither the key nor any single
component is the thing that is wrong:

```text
error: `base` color takes 3 (rgb, alpha=1.0) or 4 (rgba) components — found 2
 --> tests/ui/material_color_arity.rs:6:27
  |
6 |     material gold { base: (1.0, 0.72), metallic: 1.0 }
  |                           ^^^^^^^^^^^
```

### The one collision rustc could not place

Aether's rule is to [defer](diagnostics.md#what-aether-checks-and-what-it-leaves-alone)
whenever the downstream layer already produces a good error. `material` found
the one place in the whole DSL where that rule breaks down, and it took a
measurement to see it.

Two materials of one name expand to two `pub fn twice()`s — and to *nothing
else*. No derive, no trait bound, no second item. rustc has only `E0428` to
report, and measured against real rustc, it puts **both** of its labels on the
`aether!` token itself. Not one user token is named anywhere in the output. The
sibling cases are rescued by their extra items: `component` × `component` also
emits a derive, so a second, localized error points at real source.

So this one is Aether's, with both spans, in the shape the duplicate-`plugin`
diagnostic established:

```text
error: duplicate material `twice` — each material expands to a builder fn of its own name, and two of one name is one fn defined twice
  --> tests/ui/material_duplicate_name.rs:12:14
   |
12 |     material twice { base: (1.0, 1.0, 1.0) }
   |              ^^^^^

error: the first `material` of this name is here
  --> tests/ui/material_duplicate_name.rs:11:14
   |
11 |     material twice { base: (0.0, 0.0, 0.0) }
   |              ^^^^^
```

Cross-*kind* collisions stay with rustc, which genuinely lands them well — and
that is now stated by a test rather than by a comment:
`two_materials_of_one_name_are_refused_with_both_spans` ends by asserting that
`material paint` beside `component Paint` expands **untouched**.

## What this costs

Nothing at run time. The emitted fn is `#[inline]` and its body is the
constructor call you would have typed, so the material you author through Aether
and the one you write by hand are the same instructions. At compile time you pay
one small parse; there is no derive behind this construct at all.

## See also

- [Aether overview](overview.md) — the macro, the crates, and the shipped rungs.
- [Diagnostics](diagnostics.md) — the full error contract, including these.
- [Data constructs](data-constructs.md) — `component`, `tag`, `bundle`, `event`.
- [Rendering overview](../rendering/overview.md) and
  [Shader eDSL](../rendering/shader-edsl.md) — what consumes a `MaterialGpu`,
  and who owns the shading math.
- Source: `crates/aether_lang/src/parse.rs` (`MATERIAL_KEYS`, the color
  production), `crates/aether_lang/src/expand.rs` (`material_fn`,
  `validate_block`), `crates/aether_tests/tests/a5_material.rs`, goldens in
  `crates/aether_tests/tests/ui/material_*.rs`.
