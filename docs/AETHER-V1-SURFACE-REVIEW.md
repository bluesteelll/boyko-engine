# Aether v1 — the whole surface, on one page, for review

**Purpose.** Every feature and every syntactic form the shipped v1 language accepts, in one file, so
the owner can go row by row and mark what changes. The `Verdict` column is empty on purpose — fill it
with `keep` / `change` / `?` and the row id is the handle for the follow-up work.

**Provenance.** Read out of `crates/aether_lang/src/{parse,expand,ast,ctx,diag}.rs` via
[book/src/aether/reference.md](../book/src/aether/reference.md), which is itself pinned by the
`trybuild` goldens in `crates/aether_tests/tests/ui/`. Specimen code is copied verbatim from
[a6_scene.rs](../crates/aether_tests/tests/a6_scene.rs) and
[demo_arena.rs](../crates/aether_tests/tests/demo_arena.rs) — nothing here is illustrative fiction.
Where a claim is read off a code path with no test behind it, the row says so.

**What Aether is, in one line.** A single function-like macro, `aether! { … }`, that is a
**transpiler, not a runtime**: every construct expands at compile time to the canonical hand-written
engine surface — the same `#[derive(Component)]`, the same `Query<D, F>`, the same `impl Plugin`.
No interpreter, no registry, no reflection, no second codegen path.

---

## 0. The whole grammar

```ebnf
block     = ( "aether" "v1" ";" )? construct* ;          (* no separator between constructs *)

construct = component | tag | bundle | event | system | plugin | machine | material | scene ;

component = "component" UpperCamel "{" ( comp_item ("," comp_item)* ","? )? "}" ;
comp_item = IDENT ":" TYPE
          | "requires" PATH ("," PATH)*
          | ("on_add"|"on_insert"|"on_replace"|"on_remove") "=" PATH
          | "no_bundle" ;

tag       = "tag" UpperCamel ( "(" "bitset" ")" )? ";" ;

bundle    = "bundle" UpperCamel "{" ( IDENT ":" TYPE ("," IDENT ":" TYPE)* ","? )? "}" ;   (* <=16 *)

event     = "event" UpperCamel "{" ( ev_field ("," ev_field)* ","? )? "}" ;
ev_field  = IDENT ":" ( "entity" "(" IDENT ("," IDENT)* ","? ")" | TYPE ) ;

system    = "system" snake_case "(" ( param ("," param)* ","? )? ")" clause* "{" TOKENS "}" ;
param     = "mut"? IDENT ":" param_ty ;
param_ty  = "query" "<" TYPE ("," filter)* ","? ">" | "res" "<" TYPE ">" | "mut" "res" "<" TYPE ">"
          | "local" "<" TYPE ">" | "events" "<" TYPE ">" | "emit" "<" TYPE ">" | "commands"
          | TYPE ;                                        (* verbatim escape hatch *)
filter    = ("with"|"without"|"added"|"changed"|"enabled"|"disabled") PATH ;
clause    = "on" ("startup"|"update"|"fixed") | "in" PATH | "before" PATH | "after" PATH
          | "when" EXPR_NB ;

plugin    = "plugin" UpperCamel ";" ;

machine    = "machine" UpperCamel "{" "initial" IDENT ";" state* "}" ;
state      = "state" UpperCamel "{" state_item* "}" ;
state_item = "initial" IDENT ";" | "enter" params? "{" TOKENS "}" | "exit" params? "{" TOKENS "}"
           | "on" PATH params? ( "if" EXPR_NB )? "=>" state_path ( "{" TOKENS "}" | ";" )
           | state ;
state_path = IDENT ( "." IDENT )* ;                       (* ROOT-anchored *)

material  = "material" lowercase "{" ( mat_key ("," mat_key)* ","? )? "}" ;
mat_key   = "base" ":" color4 | "emissive" ":" color3
          | ("metallic"|"roughness"|"reflectance"|"flags"|"textures") ":" EXPR ;

scene      = "scene" lowercase "{" scene_item* "}" ;
scene_item = "let" IDENT "=" mesh_src ";" | node ;
mesh_src   = "plane" "(" EXPR ","? ")" | "cube" "(" EXPR ","? ")" | "mesh" "(" EXPR "," EXPR ","? ")" ;
node       = node_head ( "at" at_pose )? ( "{" ( prop ("," prop)* ","? )? "}" )? ";"? ;
node_head  = "mesh" IDENT | "sun" | "spot" | "point" | "sky" | "camera" | "sdf" EXPR | "entity" ;
prop       = "material" ":" IDENT | "children" ":" "[" node ("," node)* ","? "]"
           | head_key ":" key_value | "casts_shadow" | EXPR ;
```

`EXPR_NB` = `Expr::parse_without_eager_brace` (stops before a `{`). `TOKENS` = a verbatim,
unvalidated `proc_macro2::TokenStream`.

### The nine constructs at a glance

| id | Construct | Name case | Terminator | Emits | Needs `plugin`? | Verdict |
|---|---|---|---|---|---|---|
| C-1 | `component` | UpperCamel | `}` | `#[derive(Component)] pub struct` | no | |
| C-2 | `tag` | UpperCamel | `;` | unit `Component`, optionally `storage = "bitset"` | no | |
| C-3 | `bundle` | UpperCamel | `}` | `#[derive(Bundle)] pub struct` | no | |
| C-4 | `event` | UpperCamel | `}` | `#[event] pub struct` | no | |
| C-5 | `system` | snake_case | `}` | `pub fn` with the desugared signature | only if it has a clause | |
| C-6 | `plugin` | UpperCamel | `;` | `pub struct` + `impl Plugin` holding every sibling registration | — | |
| C-7 | `machine` | UpperCamel | `}` | flat `States` enum + one transition fn per (leaf, event) | **yes** | |
| C-8 | `material` | lowercase | `}` | `#[inline] pub fn` over `Material::new` / `with_textures` | no | |
| C-9 | `scene` | lowercase | `}` | `pub fn` spawning the declared world | no | |

---

## 1. One specimen block — every construct at once

Verbatim from [demo_arena.rs](../crates/aether_tests/tests/demo_arena.rs) (which is itself a gate:
building the plugin type-checks every emitted item against the real engine), with the mesh half of
[a6_scene.rs](../crates/aether_tests/tests/a6_scene.rs)'s `lab` and `annex` folded in to cover the
heads a headless test cannot run.

```rust
aether! {
    aether v1;                        // optional; absent == V1, byte-identical expansion
    plugin Arena;

    // ── Data ───────────────────────────────────────────────────────────────
    component Health { hp: f32, max: f32 }
    component Regenerating {
        rate: f32,
        requires Health,              // -> #[require(Health)], Default-constructed
        on_add = heal_full,           // on_add | on_insert | on_replace | on_remove
        no_bundle,                    // suppress the single-component Bundle
    }

    tag Enemy;                        // ZST component, real archetype bit
    tag Stunned(bitset);              // EnableTag backend: no bit, no pool, O(1) toggle

    bundle Pawn { health: Health, vel: Velocity }          // <= 16 fields

    event Damage {
        victim: entity(Health),       // participant lane, context components
        amount: f32,                  // parameter lane; must be Copy
    }

    // ── Content ────────────────────────────────────────────────────────────
    material gold  { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
    material lamp  { base: (0.02, 0.02, 0.02), roughness: 0.6, emissive: (1.6, 0.9, 0.3) }

    scene lab {
        let floor = plane(22.0);
        let block = cube(1.0);
        let custom = mesh(&verts(), &indices());

        mesh floor;                                              // no `at` -> Transform::IDENTITY
        mesh block at Transform { translation: Vec3::new(0.0, 3.0, -4.5),
                                  rotation: Quat::IDENTITY,
                                  scale: Vec3::new(14.0, 6.0, 0.4) };
        mesh block  at (-2.4, 0.5, -2.2) { material: gold, casts_shadow };
        mesh custom at ( 0.0, 0.0,  5.0) { material: lamp };

        entity at (0.0, 0.0, 0.0) {                              // the escape hatch head
            Health { hp: 10.0, max: 10.0 },                      // bare component expression
            children: [
                entity at (-0.5, 1.0, 0.0) { },
                entity at ( 0.5, 1.0, 0.0) { }
            ]
        };

        sun   { dir: (-0.42, 0.80, 0.42), color: (1.0, 0.97, 0.92), lux: 3.2 }
        sky   { sky: (0.28, 0.36, 0.50), ground: (0.15, 0.14, 0.13) }
        point { pos: (-1.8, 2.2, 2.4), color: (0.5, 0.7, 1.0), power: 240.0, range: 9.0 }
        spot  { pos: (3.6, 4.2, 3.2), dir: (-0.6, -0.7, -0.5), color: (1.0, 0.85, 0.6),
                power: 6000.0, range: 14.0, inner: 16.0, outer: 26.0, casts_shadow }
        camera at (0.0, 2.1, 8.4) { aspect: 1120.0 / 720.0, fov: 52.0, far: 120.0 }
        camera at (0.0, 2.1, -8.4) { aspect: 1120.0 / 720.0,
                                     Camera { order: 1, ..Camera::DEFAULT } }

        sdf SdfEdit::sphere([3.2, 0.85, 1.8], 0.85, sdf_op::UNION, 0.0);
    }

    // ── Behaviour ──────────────────────────────────────────────────────────
    system integrate(q: query<(&mut Transform, &Velocity)>, time: res<Clock>) on update {
        for (t, vel) in &mut q { t.translation = t.translation + vel.v * time.dt; }
    }

    system chase(
        q: query<&mut Velocity, with Enemy, without Stunned>,
        tally: mut res<Tally>,
    ) on update after integrate { }

    system apply_damage(
        dmg: events<Damage>,
        hurt: query<&mut Health>,
        out: emit<WaveCleared>,
        mut cmds: commands,
        scratch: local<u32>,
        dev: NonSendRes<GpuDevice>,             // verbatim escape hatch, untouched
    ) on update in CombatSet before chase when auditing { }

    // ── Flow ───────────────────────────────────────────────────────────────
    machine GameFlow {
        initial Boot;

        state Boot { on WaveCleared => Playing; }

        state Playing {
            initial Fighting;
            enter (tally: mut res<Tally>) { tally.entered_playing += 1; }
            exit  (mut cmds: commands)    { }

            state Fighting {
                on WaveCleared (s: res<Score>) if s.0 > 0 => Playing.Resting { /* action */ }
            }
            state Resting { on WaveCleared => Playing.Fighting; }
        }
    }
}
```

---

## 2. Per-construct surface

### 2.1 Rules shared by the four data constructs

| id | Rule | Shipped behaviour | Verdict |
|---|---|---|---|
| D-1 | Visibility | Always `pub` — struct and every field. **There is no visibility syntax in the grammar.** | |
| D-2 | Generics / `where` | Not supported. Fails with syn's `expected curly braces`. | |
| D-3 | Field attributes | Not supported. A doc comment or `#[allow(…)]` on a field is a parse error. Construct-level attributes too. | |
| D-4 | Trailing commas | Permitted in every list, everywhere. | |
| D-5 | Emission order | Source order across the whole block; a flat deterministic item stream. | |
| D-6 | Duplicate names | **Not Aether's** for these four — rustc's E0428 reports on both user idents (the derive gives a second localized error). | |
| D-7 | Plugin participation | None. `plugin_impl` collects only `system`, `scene`, `machine`. **An `event` is not registered for you.** | |
| D-8 | Recovery stub | A broken construct still mints `pub struct NAME;` beside its `compile_error!`, so the name keeps resolving. | |

### 2.2 `component`

| id | Item | Type | Default | Emits | Verdict |
|---|---|---|---|---|---|
| CM-1 | `IDENT : TYPE` | verbatim `syn::Type` | — | `pub IDENT: TYPE` | |
| CM-2 | `requires P1, P2` | one or more `syn::Path`, may repeat | none | one merged `#[require(P1, P2, …)]` | |
| CM-3 | `on_add` / `on_insert` / `on_replace` / `on_remove` | `syn::Path` (fn path) | none | a key inside `#[component(…)]` | |
| CM-4 | `no_bundle` | flag | `false` | `no_bundle` key inside `#[component(…)]` | |
| CM-5 | Item order | free interleaving, comma-separated, empty body legal | | | |
| CM-6 | Attribute order | fixed: `#[derive]`, `#[require]` (omitted when empty), `#[component(…)]` (omitted when no hooks and no `no_bundle`); hooks in declaration order, `no_bundle` last | | | |

Limits worth a verdict:

| id | Limit | Verdict |
|---|---|---|
| CM-7 | `requires` takes **bare paths only** — the derive's `C = expr` and `D(args)` entry forms are **unreachable**. A required component through Aether is always `Default`-constructed. | |
| CM-8 | **No storage key on `component`.** `storage = "bitset"` only via `tag(bitset)`; `storage = "dense"` has **no Aether surface at all**. | |
| CM-9 | A field cannot be named `requires`, `no_bundle`, or any hook key — a bare keyword at item-head position opens that item. | |
| CM-10 | `requires` list terminates on **lookahead**, not a count: `requires A, no_bundle::C` is one two-path list (an ident followed by `::` always continues the path). | |

### 2.3 `tag`

| id | Form | Emits | Verdict |
|---|---|---|---|
| TG-1 | `tag Player;` | `pub struct Player;` — ordinary signature storage, derive's ZST auto-tag path | |
| TG-2 | `tag Stunned(bitset);` | `#[component(storage = "bitset")]` — the EnableTag backend | |
| TG-3 | `(bitset)` is the **only** modifier the grammar knows; the refusal has no did-you-mean (candidate set is one literal) | | |

Consequences of TG-2 that the language does not restate at the site:

| id | Property | Verdict |
|---|---|---|
| TG-4 | No archetype bit, no `ComponentPool`, toggle is one atomic RMW at `(archetype, row)`; **no hook or observer fires**, dead entities are a silent no-op. | |
| TG-5 | `Added<T>` / `Changed<T>` on a bitset tag is a **compile error** (const-assert). `With` / `Without` is **not** refused but never matches. Filters to use: `enabled` / `disabled`. | |
| TG-6 | Lifecycle hooks + bitset is rejected by the derive; unreachable from Aether because `tag` has no hook keys. | |

### 2.4 `bundle`

| id | Rule | Verdict |
|---|---|---|
| BN-1 | Fields only. No modifiers, no keys. | |
| BN-2 | `MAX_BUNDLE_ARITY = 16`; the 17th field's own name carries the error (Aether owns the span, the derive owns the rule). | |
| BN-3 | A zero-field `bundle B {}` parses, then the derive refuses it. | |

### 2.5 `event`

| id | Field kind | Written | Emits | Verdict |
|---|---|---|---|---|
| EV-1 | Participant | `name: entity(A, B)` | `#[participant(components = "A, B")] pub name: Entity` | |
| EV-2 | Parameter | `name: Type` | `#[parameter] pub name: Type` | |
| EV-3 | `entity` is contextual **only** at field-type position as a bare ident with no `::` and no `<`. `thing: my::entity` is an ordinary parameter. | | | |
| EV-4 | Context components must be **bare, unqualified, non-generic idents** — `entity(foo::Bar)`, `entity(::A)`, `entity(Slot<A,B>)` are refused on the user's tokens (the derive's `components = "…"` channel splits on `,`). | | | |
| EV-5 | **The context is never defaulted** — a bare `victim: entity` is an error by design. | | | |

| id | Downstream constraint (inherited, not Aether's) | Verdict |
|---|---|---|
| EV-6 | Every field type must be `Copy + 'static`. A `String` parameter will not compile. | |
| EV-7 | The event must **not** be a ZST — `event Ping {}` trips `ZstCheck` at monomorphisation. | |
| EV-8 | `#[event]` **replaces** the struct with `E { participants: EParticipants, parameters: EParameters }`. There is no flat constructor. | |
| EV-9 | **Declaring an event does not register it.** Lane registration is yours (`preregister_event::<E>`); there is no `App::add_event`. | |

### 2.6 `system`

| id | Sugar | Emitted type | Binding gets `mut` | Verdict |
|---|---|---|---|---|
| SY-1 | `q: query<D>` | `Query<D>` | inferred from `D` | |
| SY-2 | `q: query<D, f, …>` | `Query<D, F>` (1 filter stays **bare**, no one-tuple; >=2 tuple) | inferred | |
| SY-3 | `r: res<T>` | `Res<T>` | no | |
| SY-4 | `r: mut res<T>` | `ResMut<T>` | yes | |
| SY-5 | `l: local<T>` | `Local<T>` | no | |
| SY-6 | `c: commands` | `Commands` | yes | |
| SY-7 | `e: events<E>` | `EventReader<E>` | yes | |
| SY-8 | `w: emit<E>` | `EventWriter<E>` | yes | |
| SY-9 | `x: SomeType` | verbatim, unchanged, **never inferred** — write the binding `mut` by hand | no | |

| id | Filter | Emits | Verdict |
|---|---|---|---|
| SF-1 | `with P` / `without P` | `With<P>` / `Without<P>` | |
| SF-2 | `added P` / `changed P` | `Added<P>` / `Changed<P>` | |
| SF-3 | `enabled P` / `disabled P` | `Enabled<P>` / `Disabled<P>` | |

| id | Clause | Repeatable | Emits | Verdict |
|---|---|---|---|---|
| SC-1 | `on startup` | no | `app.add_startup_system(f)` at `build` top level | |
| SC-2 | `on update` | no | a statement inside `add_systems_cfg` | |
| SC-3 | `on fixed` | no | a statement inside `add_systems_cfg_in(CoreSchedule::Fixed, …)` | |
| SC-4 | *(no `on`)* | — | Main, unordered (`bucket()` = Update) | |
| SC-5 | `in PATH` | **yes** | `.in_set(PATH)` in source order | |
| SC-6 | `before PATH` / `after PATH` | **yes** | `.before(key)` / `.before_set(PATH)` — see SO-* | |
| SC-7 | `when EXPR` | **yes** | `.run_if(EXPR)`, appended after every ordering call | |
| SC-8 | Clause order is free; only `on` is at-most-once. `in` is the real Rust `in` token; the rest are contextual idents. | | | |
| SC-9 | **`on startup` accepts no other clause** — refused at the first non-`on` clause's span regardless of order. | | | |

| id | Sibling ordering (`before` / `after` with a bare ident) | Verdict |
|---|---|---|
| SO-1 | Bare ident naming a sibling in the **same bucket** -> `SystemKey` edge (`__aether_k_<name>`); the bucket is topologically sorted (stable Kahn, lowest source index wins ties). | |
| SO-2 | Bare ident naming a **broken** sibling -> edge silently dropped, no diagnostic. | |
| SO-3 | Sibling on `on startup`, or in a **different bucket** -> error (cross-schedule ordering is not expressible). | |
| SO-4 | Bare ident within Levenshtein <= 2 of a sibling -> **error**, not a pass-through. Cost: a real `SystemSet` type that close in name must be referenced by a qualified path. | |
| SO-5 | Anything qualified/generic, or a bare name farther than distance 2 -> `.before_set(PATH)` verbatim, rustc resolves it. | |
| SO-6 | A cycle of sibling edges inside one bucket -> Aether error naming every member (the engine would also catch it at `build()`). | |

| id | Other system-level facts | Verdict |
|---|---|---|
| SM-1 | The paren list is **mandatory** (`system tick {}` does not parse) but may be empty. | |
| SM-2 | The body is a verbatim `TokenStream`. No Aether expression syntax, no control flow, no rewriting. | |
| SM-3 | Mutability inference scans `D` **token-exactly** — `query<&Mutation>` does not false-positive. | |
| SM-4 | Every generated `system` fn carries an unconditional `#[allow(clippy::too_many_arguments)]` (the lint would otherwise land on the whole `aether!` token). | |
| SM-5 | Aether emits **no** param-arity check; the engine's `MAX_SYSTEM_PARAM_ARITY = 12` and 13..=24 `const { panic! }` at monomorphisation. | |

### 2.7 `plugin`

| id | Rule | Verdict |
|---|---|---|
| PL-1 | One `plugin` per block, `UpperCamel`, `;`-terminated. | |
| PL-2 | **Required** iff the block holds a `system` with any clause (`on`/`in`/`before`/`after`/`when`) or a `machine`. | |
| PL-3 | With a plugin present, **every** sibling `system` is registered — a clause-free one lands on Main unordered. | |
| PL-4 | `material` needs no plugin and a sibling plugin does **not** register it. `scene` needs none but is registered if one is present. | |
| PL-5 | Registration order is **block source order** across systems and scenes alike (pinned by test, not left to emission shape). | |
| PL-6 | A block with no plugin emits the fns and leaves registration to you. | |

### 2.8 `machine`

| id | State item | Cardinality | Verdict |
|---|---|---|---|
| MC-1 | `initial C;` | <= 1 per state; **required** on any composite that is ever a resolved target; refused on a childless state | |
| MC-2 | `enter (p)? { … }` | <= 1 | |
| MC-3 | `exit (p)? { … }` | <= 1 | |
| MC-4 | `on E (p)? (if G)? => T (BLOCK\|;)` | any number, one per event type per state | |
| MC-5 | `state N { … }` | any number, unbounded nesting | |

| id | Semantics | Verdict |
|---|---|---|
| MS-1 | The hierarchy exists **only inside the transpiler**: flattened to one leaf enum, superstate handlers copied down, one `run_if(in_state(leaf))` system per (leaf, event). | |
| MS-2 | Machine-level `initial` is required, positional, a **single ident** (not a path), parsed before the state loop. | |
| MS-3 | Targets are **root-anchored** (`Playing.Paused`). No relative form, no sibling form, no `..`. | |
| MS-4 | **Innermost wins** for inheritance; the dedup key is the event path's whole token spelling (`a::E` and `b::E` are different events). Registration order is then re-sorted by declaration index. | |
| MS-5 | Variant name = **concatenation** of the state path (`Playing.Running` -> `PlayingRunning`); generated fn half = `snake()` of the same. Both mappings are lossy and every lossy site is pre-checked. | |
| MS-6 | Machines are **app-scoped**: one world-global `State<S>`. Per-entity machines are not shipped. | |
| MS-7 | **One transition per machine per frame**; the remainder of the frame's events is discarded. | |
| MS-8 | Guards and actions observe the **pre-transition** world; each leg costs **two frames**. | |
| MS-9 | The enum is `pub`; every generated fn is **private** and lives in the same module as the plugin's `build`. | |
| MS-10 | Params of every handler a transition inlines are **merged** into one fn signature. | |
| MS-11 | State body items may appear in **any order** and interleave freely (looser than the plan's EBNF). | |

### 2.9 `material`

| id | Key | Shape | Default | Verdict |
|---|---|---|---|---|
| MT-1 | `base` | 3 or 4 components | **required** (alpha synthesized as `1.0` when 3) | |
| MT-2 | `metallic` | EXPR | `0.0` | |
| MT-3 | `roughness` | EXPR | `0.5` | |
| MT-4 | `reflectance` | EXPR | `0.5` | |
| MT-5 | `emissive` | **exactly 3** components | `[0.0; 3]` | |
| MT-6 | `flags` | EXPR | `0` | |
| MT-7 | `textures` | EXPR (`MaterialTextures`) | absent -> untextured constructor; present -> switches the whole emission to `with_textures` over an explicit `MaterialGpu::new` | |
| MT-8 | Each key at most once, order free, values are **verbatim expressions** (`metallic: BRASS_METALLIC` works). | | | |
| MT-9 | Emits `#[inline] pub fn name() -> Material` and **nothing else** — no table, no registry, no lazy static, no `Assets` call. Minting stays yours: `materials.add(gold())`. | | | |

### 2.10 `scene`

| id | Mesh source | Arity | Verdict |
|---|---|---|---|
| SL-1 | `plane(SIZE)` | 1 | |
| SL-2 | `cube(SIZE)` | 1 | |
| SL-3 | `mesh(VERTICES, INDICES)` | 2 (`&[Vertex]`, `&[u32]`) | |
| SL-4 | Bindings are **scene-scoped** and **scene-wide**: every `let` is hoisted above every node, so a node may name a binding declared below it. | | |

| id | Head | Bundle | `at`? | `material:`? | `casts_shadow` => | Keys | Verdict |
|---|---|---|---|---|---|---|---|
| SN-1 | `mesh IDENT` | `MeshBundle::new` | yes | yes | `ShadowCaster` | — | |
| SN-2 | `sun` | `DirectionalLightObject` | **no** | no | — | `dir`\*, `color`, `lux`\* | |
| SN-3 | `spot` | `SpotLightObject` | **no** | no | `CastsPunctualShadow` | `pos`\*, `dir`\*, `color`, `power`\*, `range`\*, `inner`\*, `outer`\* | |
| SN-4 | `point` | `PointLightObject` | **no** | no | `CastsPunctualShadow` | `pos`\*, `color`, `power`\*, `range`\* | |
| SN-5 | `sky` | `SkyLight::new` | **no** | no | — | `sky`\*, `ground`\* | |
| SN-6 | `camera` | `CameraRig` | yes | no | — | `aspect`\*, `fov` (60.0, degrees), `near` (0.1), `far` (1000.0) | |
| SN-7 | `sdf EXPR` | `SdfPrimitive(EXPR)` | **no** | no | — | none | |
| SN-8 | `entity` | `SpatialBundle` with `at`, `spawn_empty()` without | yes | yes | `ShadowCaster` | none | |

`*` = required, refused on the head's own span rather than invented. The rule: a key is defaulted only
when Aether can *honestly synthesize* a value (white is a neutral that is right; an illuminance is not).

| id | `at` form | Lowers to | Verdict |
|---|---|---|---|
| AT-1 | *(absent)* | `Transform::IDENTITY` (Aether's own, fully qualified) | |
| AT-2 | `at (x, y, z)` | `Transform::from_translation(Vec3::new(x, y, z))` | |
| AT-3 | `at (EXPR)` | the expression verbatim | |
| AT-4 | `at EXPR` | verbatim, spans preserved (`at Transform { … }` keeps **your** bare spellings) | |
| AT-5 | any other arity | refused on the tuple | |
| AT-6 | On the five heads that derive their pose, `at` is **refused with a per-head reason**, never silently dropped. | | |

| id | Prop | Effect | Verdict |
|---|---|---|---|
| SP-1 | `material: IDENT` | a **sibling `material` construct's** name (block-scoped) -> `.insert(MaterialHandle(idx as u16))` | |
| SP-2 | `casts_shadow` | `.insert(ShadowCaster)` or `.insert(CastsPunctualShadow)`, per head | |
| SP-3 | `children: [ … ]` | `Commands::add_child` per child; the reverse `Children` collection comes from the kernel's own hooks | |
| SP-4 | bare `EXPR` | `.insert(EXPR)` — the escape hatch that makes sugar additive | |
| SP-5 | Emission order per node is **fixed and independent of source order**: shadow marker, then `MaterialHandle`, then bare exprs in source order. | | |
| SP-6 | A prop is keyed iff it reads `ident :` with a **single** colon; `ident ::` and `Ident { … }` fall through to the expression arm. | | |
| SP-7 | Scene params are **demand-driven** (at most four: commands, meshes, materials, device) — a scene with no `let` binding needs no GPU device and runs headless. | | |
| SP-8 | The material mint is **hoisted**: one `Assets::add` per material per scene run, shared by every node naming it. | | |

---

## 3. What v1 deliberately does NOT have

This is the highest-value review table: each row is a thing an author will reach for and not find.

| id | Absent | Today's workaround | Verdict |
|---|---|---|---|
| N-1 | **Visibility syntax** (`pub(crate)`, private) — everything is `pub` | none | |
| N-2 | **Generics / `where`** on any construct | none | |
| N-3 | **Field attributes and doc comments** inside a construct | none | |
| N-4 | **Construct-level attributes** (`#[cfg(…)]` on a construct) | put the whole block behind a `cfg` module | |
| N-5 | `storage = "dense"` — **no surface at all** | hand-written `#[component(storage = "dense")]` | |
| N-6 | `requires C = expr` / `requires D(args)` — only `Default`-constructed requires | hand-written derive | |
| N-7 | **Event registration** — `event` mints no lane | `preregister_event::<E>` by hand | |
| N-8 | `priority N` on machine transitions (plan R7) | none | |
| N-9 | **Entity-scoped machines** (`machine … on entity`, plan §5.5) | none | |
| N-10 | **Relative / sibling state paths** — targets are root-anchored only | spell the full path | |
| N-11 | **More than one transition per machine per frame** | none; the remainder is discarded | |
| N-12 | **Two plugins in one block** | split the block | |
| N-13 | **`at` on `sun`/`spot`/`point`/`sky`/`sdf`** | the head's own keys | |
| N-14 | **A `scene` node handle** — a node gets no name and returns no `Entity` | `entity` head + a marker component + a query | |
| N-15 | **Scene unload / a scene as a runtime object** — a scene is a spawn fn, nothing more | despawn by marker | |
| N-16 | **Mesh sources beyond `plane`/`cube`/`mesh(&V,&I)`** (no asset path, no glTF) | build the vertex/index slices in a fn | |
| N-17 | **A material asset handle in the language** — `material:` is the only consumer | call the builder fn yourself | |
| N-18 | **An `Or<>` filter, or nested query filters** | verbatim escape: write the `Query<…>` type by hand | |
| N-19 | **Run-condition combinators** (`when a && b` works as an EXPR, but there is no `unless` / set-level condition) | write the predicate fn | |
| N-20 | **`in`-set declaration** — you may reference a `SystemSet` but not declare one | declare the set in Rust | |
| N-21 | **Aether expression syntax / control flow** — every body is a verbatim `TokenStream` | by design | |
| N-22 | **A second syntax version** — `v1` is the only spelling; the header is a gate, not a dialect | | |

---

## 4. Sharp edges — the traps a reviewer should rule on

| id | Trap | Verdict |
|---|---|---|
| X-1 | **The swallowed-body trap.** `at` parses eagerly, so `camera at MY_POSE { aspect: 1.5 }` reads `MY_POSE { aspect: 1.5 }` as one struct literal and the node body is gone. Fix: `at (MY_POSE) { … }`. Aether attaches a diagnosis, but **only** when the swallowed braces look like this node's body. | |
| X-2 | `sdf MY_EDIT { … }` has the **identical hazard and no hint**, structurally: the hint rides on the required-key refusal and `sdf` has no keys. The author gets rustc on their own tokens. | |
| X-3 | **`mut` occupies two positions with different meanings**: `mut cmds: commands` (binding) vs `r: mut res<T>` (type sugar). The type-position `mut` pairs with `res` and nothing else. | |
| X-4 | **A sugar keyword is a sugar only when its own syntax follows it.** `res` with no `<` is a verbatim user type; `commands::Something` is verbatim; `query(…)` is refused. Case-sensitive: `NonSendRes<…>` is verbatim. | |
| X-5 | **The verbatim escape is never mutability-inferred** — `mut assets: NonSendResMut<…>` needs the binding `mut` by hand. | |
| X-6 | **Emission order is not execution order** in `bucket_stmts`: the ordering target is *registered* first so its key exists. | |
| X-7 | **A near-miss `before`/`after` ident is a hard error**, not a pass-through — a real `SystemSet` within edit distance 2 of a sibling system name must be qualified. | |
| X-8 | **The version header is the one non-recoverable position** — all three of its failure modes abort the whole block. Everything else recovers per-construct. | |
| X-9 | **A broken `plugin` still occupies the plugin slot**, so no sibling clause reports "needs a plugin". | |
| X-10 | **A scene `let` name can shadow a generated param** — `let dev = cube(0.5)` was a real failure until the params were `__aether_`-prefixed. The prefix is the whole defence. | |
| X-11 | **`system` is the exception in the case gate**: its check is an inline `starts_with(char::is_uppercase)` on the raw string, not the shared `lowercase_gate`. | |
| X-12 | Head-key **slot indices are positional** against the key tables: renaming a key is free, **reordering a table moves the matching arm with it**. | |

---

## 5. Where the shipped language diverges from `docs/AETHER-LANG-PLAN.md`

Each row is already a decision that was made once; a verdict here either ratifies or reopens it.

| id | Plan | Shipped | Verdict |
|---|---|---|---|
| P-1 | `param := IDENT ':' param_ty` | `'mut'? IDENT ':' param_ty` — binding-level `mut` accepted | |
| P-2 | non-empty param list, no trailing comma | empty list and trailing comma both accepted | |
| P-3 | inference set excludes `events<E>` | **includes it** — `EventReader::read` takes `&mut self` here | |
| P-4 | near-miss `before`/`after` passes through with a note on rustc's error | an **Aether error** carrying the note's text (stable proc-macros cannot attach notes downstream) | |
| P-5 | (unstated) | with a plugin present, **every** sibling system is registered | |
| P-6 | (unstated) | one-filter `Query<D, F>` stays **bare**, no one-tuple | |
| P-7 | machine state body order fixed (`initial`, handlers, states) | **any order, freely interleaved** | |
| P-8 | `for __e in &mut __ev { … }` | **drain-then-act** with `__aether_fire` (§5.1 is the semantic authority) | |
| P-9 | `__aether_gameflow__…` | `__aether_game_flow__…` — the machine-name half is `snake()`-collapsed too | |
| P-10 | a doc comment on the generated enum | none; only the `in_*` predicates carry one | |
| P-11 | `unknown tag storage \`bitmap\`; the only tag storage modifier is \`bitset\`` | `unknown tag modifier \`bitmap\`; the only one is \`bitset\` (the EnableTag backend)` | |
| P-12 | dense components "will surface as another `component` key" | **not shipped** — no storage key on `component` at all | |
| P-13 | `priority N` on transitions (R7) | not in the grammar | |
| P-14 | entity-scoped machines (§5.5) | not in the grammar | |
| P-15 | `AetherCtx` carries "declared symbols + kinds + spans + per-kind payloads" | two fields: the sibling material list + the broken-material names, plus the validation | |
| P-16 | one expander file per construct + `syn::custom_keyword!` | flat crate, no keyword table | |
| P-17 | `macrotest` snapshots | `aether_lang`'s own token-for-token unit tests (no new dev-dependency) | |

---

## 6. Cross-cutting policy rows

| id | Policy | Verdict |
|---|---|---|
| G-1 | **Name case is enforced at parse, on the name's own span.** UpperCamel for type-producing, lowercase/snake for fn-producing. A rename suggestion is appended only when it actually differs. | |
| G-2 | **Aether pre-checks a downstream fault only where it produces a strictly better span or message** — i.e. where rustc would report on generated tokens. Everything else is left to rustc on purpose. | |
| G-3 | **One error per fault**, and independent constructs still expand: a broken construct records a `compile_error!` plus a name-carrying stub. | |
| G-4 | **Whole-block rules do not accumulate** — a block gets at most one `AetherCtx` error (duplicate fn names -> one-plugin -> plugin requirement, each returning on its first violation). | |
| G-5 | **Every emitted path is a token, fully qualified, resolved in your crate.** `aether_lang` has no engine dependency, which is why its token pins cannot notice an engine change at all — that is `aether_tests`' job (the R4 anti-drift rule). | |
| G-6 | **Constructs have no separator.** A stray `;` between two constructs is refused. | |
| G-7 | The version header is **optional**; `aether v1; X` and `X` expand byte-for-byte identically. | |

---

## See also

- [book/src/aether/reference.md](../book/src/aether/reference.md) — the full lookup, with every refusal message verbatim and its golden
- [book/src/aether/overview.md](../book/src/aether/overview.md) · [data-constructs.md](../book/src/aether/data-constructs.md) · [systems-and-plugins.md](../book/src/aether/systems-and-plugins.md) · [state-machines.md](../book/src/aether/state-machines.md) · [materials.md](../book/src/aether/materials.md) · [scenes.md](../book/src/aether/scenes.md) · [diagnostics.md](../book/src/aether/diagnostics.md)
- [docs/AETHER-LANG-PLAN.md](AETHER-LANG-PLAN.md) — design intent (superseded by the source wherever they disagree)
- [crates/aether_tests/tests/demo_arena.rs](../crates/aether_tests/tests/demo_arena.rs) — every construct in one compiling block
