# UI-ADVANCED — Research: declarative UI in a macro / DSL

**Campaign:** advanced UI (sprites, animation, richer interactivity) for `boyko_ui`, and its
integration into the Aether DSL as a first-class construct.
**Branch:** `feat/ui-advanced` (worktree `D:/wt/ui`).
**Question this document answers:** how do declarative UI systems express a tree, and which of
those models fits an ECS-native, data-oriented, **retained-mode** engine — and which fights it.
**Method:** read the shipped code in this repo first (nothing here is assumed), then the open
literature and the primary sources of eight systems. Every claim about this repo carries a
`file:line`; every claim about another system carries a link.

---

## 0. Executive summary

| | |
|---|---|
| **Three candidate models** | **A** — extend the existing `ui!` macro + `.ui` text format in place. **B** — make `ui` a tenth **Aether construct** that lowers to the *same* `ui!` codegen. **C** — a retained view-tree diffed against the world each change (Xilem / Dioxus shape). |
| **Recommendation** | **B, sequenced after a measured slice of A.** One authoring language, one expansion authority, no second tree. |
| **Model that fights this engine** | **C.** It re-derives, in a parallel non-ECS data structure, the two things this repo already has: a keyed reconciler (`reload/reconcile.rs`) and fine-grained property binding (`binding/`). Its dynamic cases are `Box<dyn AnyView>`-shaped. Both are Principle 0 / Principle 1 violations, and Bevy — the largest ECS UI project in the field — shipped BSN with **no diffing at all**. |
| **Handlers** | Stay **named actions** (`OnClick(u16)` over a dense `Actionlike::index()`), not inline closures, not per-node observers. This is Iced's Elm-message model, already implemented here, already reflection-free and POD, and already serializable into a text file. |
| **Strongest argument against the recommendation** | §6. Two of them, and the first has a *decidable measurement* attached. |

---

## 1. The starting position — this repo already runs TWO declarative UI front ends

This is a completion campaign. Before comparing anything external, here is what exists, measured.

### 1.1 `ui!` — an open-vocabulary Rust macro

`crates/boyko_macros/src/ui.rs` (528 lines). Grammar (from its own doc comment,
`crates/boyko_macros/src/lib.rs:446-457`):

```text
ui!         := preamble? node ( ',' node )* ','?
preamble    := 'commands' ':' IDENT ';'
node        := name? '{' body '}'
name        := '#' IDENT                       // declares a let-binding + UiName
body        := items? children?
items       := component_item ( ',' component_item )* ','?
component_item := EXPR                          // a real Rust component literal
children    := 'children' ':' '[' ( node ( ',' node )* ','? )? ']'
```

Properties that matter for the comparison:

* **Open vocabulary.** A component item is `syn::Expr` — *any* Rust expression. The macro never
  needs to know the component's type. It only performs a **syntactic** last-path-segment match to
  recognise `UiLayout`/`ComputedRect` for the bundle fast path, and says so
  (`ui.rs`, `head_ident_is` doc): *"Only an alias that renames the type away from `UiLayout` … is
  missed, which then correctly falls to the generic insert path."*
* **Spawn-only.** `lower_node` emits `cmds.spawn(<base>).id()` + chained `.insert(…)` + one
  `add_child` per link, pre-order. There is no diff, no identity beyond `#name → UiName`, no
  dynamic content, no handler syntax, no styling.
* **`#name` is a `let` binding**, exposed after the invocation. Value-position `#name` references
  inside a component field are *not* supported, and the doc says why: a bare `#ident` is not valid
  Rust expression syntax and each item is parsed as a `syn::Expr`.

### 1.2 `.ui` — a closed-vocabulary indentation text format

`crates/boyko_ui/src/text/` — parser (off-side rule, zero lookahead, never fails at file level,
`parser.rs`), lowering (`lower.rs:209`, an explicit line-cited mirror of the macro's `lower_node`),
serializer, and a component dispatch that is a **hand-written `match` on the component's text
name**: `parse_and_insert`, `text/dispatch.rs:71` — ~20 component arms today; 83 string-keyed arms
across the file once field and enum-variant names are counted.

The two front ends are held together by an equivalence gate: `crates/boyko_ui/tests/p3_equivalence.rs`
builds the same tree twice in one world and asserts identical entity count, per-node component-id
**set**, per-node component **bytes**, `ChildOf`/`Children` topology and child order, and `UiName`.

### 1.3 A keyed reconciler already exists

`crates/boyko_ui/src/reload/reconcile.rs:107` (`reconcile_ui`) diffs a freshly parsed tree against
the live tree, matching a survivor by **`UiName` (explicit identity)** and falling back to
**`UiSourceOrder` (structural identity)**. Survivors are patched *set-if-changed*; new keys are
spawned through the full lowering; vanished keys are despawned through a two-phase apply with a
forced drain barrier (the soundness fix documented at the top of that file). Transient components
are preserved by omission.

That is, verbatim, the explicit-vs-structural identity split that SwiftUI and React describe —
built, shipped, and gated.

### 1.4 Fine-grained property binding already exists

`binding/components.rs`: `BindText { source: Entity, comp: ComponentId, field: u8, field2: u8,
template: TemplateId }` (16 B) and `BindValue` (16 B). `binding/bindable.rs` documents two call
paths: the `ui!` path calls `fmt_field` with a **concrete sink** (no vtable), the `.ui` path goes
through an installed fn-pointer + a `&mut dyn fmt::Write` trampoline, *"both only when the source
changed."* Reflection-free: *"no runtime string compare, no `HashMap`, no `TypeId` / `Any`,
no `Box<dyn Fn>`."*

### 1.5 Handlers are already named actions, not closures

`interaction/action.rs`:

```rust
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnClick(pub u16);
```

with the design note: *"Each carries a dense `Actionlike::index()` as a raw `u16`, resolved at
authoring/parse time — NOT a generic `OnClick<A>`. … so it never monomorphizes per action enum and
the components are authorable from `.ui` text (an integer is the reflection-free common
denominator)."* `NO_ACTION = u16::MAX` is the "fire nothing" sentinel.

### 1.6 What is missing — measured, not assumed

* **Sprites.** `components.rs:434-467`: `UiImage { texture: u32, uv_min, uv_max, tint }` exists as a
  24-byte component but its own doc says it does *"nothing until a P5a follow-up learns `UiImage`"*,
  and its `Default` is a transparent tint so an authored-but-untextured image is invisible. There is
  no atlas, no texture table, no sprite-sheet vocabulary.
* **Animation.** `grep -rn -i "animat|tween|easing" crates/boyko_ui/src` returns **prose only** —
  two doc comments that mention animation as a future consumer. There is no track, no curve, no
  tween system.
* **Styling.** No class, no theme, no style sheet, no `with` scope. Every property is authored per
  node.
* **Dynamic content.** Nothing in the repo produces *N* nodes from *N* data rows. `BindText`
  handles "the value changed"; nothing handles "the count changed."

### 1.7 Aether does not have a `ui` construct

`crates/aether_lang/src/parse.rs:68-76` — the whole v1 registry is `component`, `tag`, `bundle`,
`event`, `system`, `plugin`, `machine`, `material`, `scene`. Nine. `docs/AETHER-LANG-PLAN.md:81`
names `ui!` as the **in-repo precedent** that the `scene` construct generalises, and §3.7 states the
generalisation explicitly: *"The generalization of the proven `ui!` shape to render objects."*
Aether v1 is complete (`docs/AETHER-LANG-PLAN.md` A0..A7); `ui` would be the tenth construct.

---

## 2. The field, compared on the five axes that actually differ

Listing frameworks teaches nothing. These are the five decisions each system makes, and every
system in the survey makes each of them differently.

### 2.1 Axis A — how nesting is expressed

| System | Nesting | Notable |
|---|---|---|
| **Bevy `bsn!`** ([0.19](https://bevy.org/news/bevy-0-19/), [PR #23413](https://github.com/bevyengine/bevy/pull/23413)) | `Player Children [ Sword, Shield ]` | **The bracket list is the *relationship component*, not a keyword.** `Player Inventory [ Apple, Potion ]` works identically for any user relationship. |
| **Flecs script** ([docs](https://www.flecs.dev/flecs/md_docs_2FlecsScript.html)) | Scope braces: `my_parent { my_child {} }` | Plus a `with SpaceShip, HasWeapons { … }` scope that factors shared components out of a group of siblings. |
| **Dioxus RSX / Leptos `view!`** | Markup nesting inside the macro | Compiles to builder calls; nesting is syntactic only. |
| **SwiftUI / Compose** | Trailing-closure nesting via result builders / `@Composable` calls | `@ViewBuilder` turns control flow into *generic types* (`TupleView`, `_ConditionalContent`) — nesting is encoded in the **type**. |
| **QML / UXML** | Markup containment | Tree is the document. |
| **`ui!` (here)** | `children: [ … ]`, a reserved context keyword hard-wired to `add_child` | A special case of what `bsn!` generalised. |

**What this means here.** This engine already has a generic Relations API
(`crates/boyko_macros/src/relationship.rs`, 673 lines; `ChildOf`/`Children` is one instance of it).
`children:` being a keyword rather than a relationship head is an artefact of `ui!` predating that
generalisation. `bsn!` and Flecs both landed on relationship-as-nesting-head, from opposite
directions, which is the strongest form of corroboration available. Flecs's `with` scope is the
ECS-native answer to "styling" — see §2.3.

### 2.2 Axis B — where handlers live (the axis the campaign turns on)

Four distinct models exist. They are not variations; they have different runtime shapes.

**Model 1 — inline closure capturing app state.** Dioxus, Leptos, React, SwiftUI, Compose. The
handler is written at the call site and captures the surrounding state. Cost in Rust: it needs
`Box<dyn Fn>` (or an equivalent type-erased slot) *per element*, plus interior mutability to reach
the state. Raph Levien's architecture survey names this precisely — observable/callback patterns
*"require shared mutable access to that state, which is clunky at best in Rust"*
([Xilem: an architecture for UI in Rust](https://raphlinus.github.io/rust/gui/2022/05/07/ui-architecture.html)).
Xilem's own answer is not to remove the closure but to thread `&mut AppState` *through* event
dispatch along an **id path** — which requires the framework to own the dispatch spine.

**Model 2 — message enum (Elm / Iced).** `view()` returns a widget tree parameterised by a
user-defined `Message` enum; the widget maps an event to a variant; one central `update` handles
every variant ([iced](https://github.com/iced-rs/iced), [book](https://book.iced.rs/first-steps.html)).
No closure captures state; the "handler" is a **value**. The documented cost is verbosity — Levien's
survey lists exactly that: *"requiring explicit message type definitions and pattern matching."*

**Model 3 — structure in markup, behaviour registered out of band.** Unity UI Toolkit: UXML declares
the tree and USS styles it; handlers are attached in C# with `RegisterCallback` after a query
([UI Toolkit intro](https://docs.unity3d.com/Manual/ui-systems/introduction-ui-toolkit.html)). The
markup stays serializable *because* it contains no code.

**Model 4 — observer as a scene node.** Bevy `bsn!`:

```rust
fn button() -> impl Scene {
    bsn! {
        Node { width: px(100), height: px(50) }
        on(|press: On<Pointer<Press>>| { info!("button pressed!") })
    }
}
```

`on(...)` returns an `impl Scene` that spawns an `Observer` — an entity — as part of the tree
(PR #23413). It reads inline but is structurally Model 1 with an ECS entity as the box.

**What this means here.** **Model 2 is already implemented, and in its strongest possible form.**
`OnClick(u16)` is an Elm message where the message is a *dense index into a fixed action enum*
(`#[derive(Actionlike)]`, `boyko_macros/src/actionlike.rs`, fieldless enum → `index()`/`COUNT`), the
component is `#[repr(transparent)]` **two bytes**, and the "update" is an ordinary ECS system
reading `ActionState`. It costs no allocation, no vtable, no monomorphisation per action set, and —
the decisive property for this campaign — **it survives a round trip through a text file**, because
an integer is representable and a closure is not.

Adopting Model 1 or Model 4 here would put a type-erased callable on every interactive node. That is
a `Box`/`dyn` on a per-element durable datum: **Principle 1 violation, and Principle 0 if the
callable lives anywhere but an ECS column.** Name it as such.

Advanced interactivity (drag, scroll, long-press, value-change, hover-exit, key-repeat) therefore
does **not** need a new handler mechanism. Each is a **new action-emitting component of the same
2-byte shape** — `OnDragStart(u16)`, `OnValueChanged(u16)` — plus, where the event carries data, a
separate payload column written by the dispatch system. Capability = component presence; runtime
on/off = the EnableTag bit. The existing `RelativeCursorPosition` (opt-in, written set-if-changed
only on the hovered node) is the shipped precedent for the payload half.

### 2.3 Axis C — styling and reuse

| System | Mechanism | Resolution time |
|---|---|---|
| **`bsn!`** | Scenes **are patches**: `my_button() = button() + overrides`; inheritance applied right-to-left | **Author/spawn time.** PR #23413: *"`Scene::resolve` applies the Scene as a 'patch' on top of the final `ResolvedScene`."* No diffing. |
| **Flecs script** | `prefab` + `IsA`, `template` with `prop`s, `with` scopes | Mixed — `IsA` value inheritance is a **runtime chain walk**; `with` is pure scoping. |
| **QML** | Property bindings + `states`/`transitions` | Runtime, engine-monitored dependencies ([property binding](https://doc.qt.io/qt-6/qtqml-syntax-propertybinding.html)). |
| **UXML + USS** | Real stylesheets with selectors and cascade | Runtime selector matching over the element tree. |
| **SwiftUI / Compose** | Modifier chains + environment values | Compile-time types + runtime environment lookup. |
| **here** | *(none)* | — |

**What this means here.** Two of these are ECS-native and two are not.

* **Take:** the `bsn!` patch model and the Flecs `with` scope. A patch is exactly what an
  `.insert` overwrite already is; a `with` scope is a **pure author-time factoring with zero runtime
  representation**. Both resolve to concrete component values before the entity is live.
* **Refuse:** a runtime selector cascade (UXML/USS) and runtime `IsA` value inheritance (Flecs).
  A selector matcher is a parallel data structure plus a per-frame or per-mutation match over the
  tree — a subsystem data model glued on the side, Principle 0. Runtime `IsA` inheritance means a
  component read must walk a prefab chain: a pointer chase per read against dense columns that were
  built precisely to avoid one.

The rule to write down: **inheritance and styling resolve at author/load time into concrete
component bytes.** A style is not a thing the renderer consults; it is a thing the spawner applied.

### 2.4 Axis D — dynamic content (a list whose length changes)

This is the hard axis, and three families exist.

**Family 1 — re-describe and diff.** React, Dioxus, Xilem. Produce a fresh lightweight description,
diff it against the retained structure, apply the minimum mutation set. Xilem states the split
cleanly: the view tree *"is retained only long enough to assist in event dispatching and then be
diffed against the next version, at which point it is dropped, while the widget tree persists."*
Dioxus's optimisation is to split each `rsx!` call into a compile-time static `Template` plus a list
of dynamic nodes, so the diff visits only dynamic slots — *"diffing takes 90% less time"*
([Templates & diffing](https://dioxuslabs.com/blog/templates-diffing/)).
Identity comes from **keys**: React's [reconciliation](https://legacy.reactjs.org/docs/reconciliation.html),
SwiftUI's `ForEach(_, id:)` (which is why elements must be `Identifiable`), Compose's `key()` which
*"overrides positional identity with a value based identity"*
([positional memoization](https://newsletter.jorgecastillo.dev/p/positional-memoization-in-jetpack)).

**Family 2 — fine-grained reactivity, no tree diff.** Leptos, SolidJS, QML bindings. Each dynamic
binding gets its own effect that writes exactly one property; *"no virtual DOM overhead"*
([Leptos](https://docs.rs/leptos/latest/leptos/macro.view.html)). Lists still need a keyed `<For>`,
because cardinality is the one thing a property effect cannot express.

**Family 3 — immediate mode.** egui: the list is a Rust `for` loop, there is no identity problem at
all, and the price is that layout runs every frame plus the documented sizing paradox — to know a
window's size you must lay it out, but layout also answers interaction queries, so position must be
chosen before size is known ([egui README](https://github.com/emilk/egui)). Levien's survey adds the
structural cost: *"it is difficult to do sophisticated layout and other patterns that are easier in
retained widget systems."* This engine has a 2340-line retained layout solver
(`crates/boyko_ui/src/layout.rs`); Family 3 is not on the table.

**What this means here.** The engine already has Family 2 (`BindText`/`BindValue` — a per-widget
component naming source entity + `ComponentId` + field `u8`, written only when the source changed)
*and* the identity half of Family 1 (`reconcile_ui`'s `UiName` / `UiSourceOrder` split). What is
missing is **cardinality**, and none of the surveyed systems can hand it over directly, because none
of them has an ECS underneath.

The ECS-native shape follows from the two things that already exist:

> **A dynamic list is not a macro loop and not a virtual tree. It is a relationship plus a pool
> system, keyed by a component.**

Concretely: a container carries `UiRepeat { source, template, key_field }` — the template being a
**disabled prefab subtree that is itself an entity**, not a `Vec<Node>`. One system compares the
source column's row count against the container's `Children` length, spawns the delta by cloning the
prefab subtree, despawns the surplus, and stamps each instance with a `UiListKey(u64)` component
taken from the keyed field. Reorders match by `UiListKey`, exactly as `reconcile_ui` matches by
`UiName` — **one identity discipline in the crate, not two**. No `Vec` of descriptions, no `Box`, no
per-frame allocation: the retained tree *is* the state, which is the whole premise of an ECS UI.

The DSL's job is then small and honest: it declares `UiRepeat`'s three fields. It does **not**
expand a loop, because a macro cannot know a runtime length — and a `for` loop inside the macro
would produce a fixed node count at compile time, which is the wrong answer to the question.

### 2.5 Axis E — the fact that a macro cannot resolve names

State the problem exactly. A proc macro receives a `TokenStream` with spans. It does not see types,
does not see `use` statements, cannot know whether `Foo` is a component, cannot know a field's type,
cannot resolve an asset path. Procedural macros are additionally *unhygienic* — they behave *"as if
the output token stream was simply written inline"*
([Rust Reference](https://doc.rust-lang.org/reference/procedural-macros.html)) — so generated code
must spell absolute paths (`::std::option::Option`) or it fails to resolve in the caller's module.

Four mitigations exist in the literature. All four are already used in this repository.

**M1 — re-spell the tokens and let rustc resolve them.** `bsn!` never resolves `Player`; it emits
`<Player as GetTemplatePatch>::patch(|t| { t.image = "player.png".into(); })` and rustc resolves the
path (PR #23413). BSN's *implicit `.into()`* — `Node { border: px(2) }` where `px(2)` is a `Val` and
the field is a `UiRect` — is type conversion done by **inference**, not by macro knowledge. This
repo's `ui!` uses the same trick: `head_ident_is` matches syntactically, guesses the fast path, and
**falls through soundly** when the guess misses.

**M2 — a registry consulted at RUNTIME, keyed by a name the macro merely passes through.**
`text/dispatch.rs:721` (`parse_action_index`): `OnClick(Jump)` in a `.ui` file resolves against a
process-wide table filled by `InputPlugin::build`, and an unknown name becomes a **report entry plus
`NO_ACTION`** — not a crash, not a dropped node. `Bindable::field_id(name) -> Option<u8>` and
`register_bind_accessor()` (a `ComponentId`-keyed fn-pointer table) are the same mitigation for
component fields.

**M3 — a symbol table the DSL builds for names *it* declared.** Aether's `AetherCtx`
(`docs/AETHER-LANG-PLAN.md` §4): `material: gold` resolves against the sibling `material` construct
in the same block, with did-you-mean at edit distance ≤ 2 (§7 item 7). A macro cannot resolve *Rust*
names — but it can perfectly resolve names declared inside its own block. For a `ui` construct this
is the strongest tool available: styles, sprite atlases, animation curves and templates declared in
the same `aether!` block resolve by name with precise spans.

**M4 — give up and version the imports.** BSN's answer for `.bsn` files is `use bevy@0.14::prelude::*;`,
and the discussion is candid that this does not close the gap: *"Rust code in bsn files is out of
scope for MVP,"* leaving a persistent code-vs-asset feature divergence
([Discussion #14437](https://github.com/bevyengine/bevy/discussions/14437)).

**What this means here.** The `.ui` format's closed vocabulary is not an oversight — it *is* M2,
with the registry hand-written as a `match`. That has a cost that scales exactly with this campaign:
every sprite component, every animation track, every new interaction component costs an arm in
`parse_and_insert` plus a serializer arm plus a row in the equivalence gate. The in-repo precedent
for fixing this is already shipping one directory over: **the derive that installs `BindAccessor`
should also install a text-parse fn pointer**, converting the hand-written vocabulary into a
registration table. That is not an invention; it is `register_bind_accessor` applied to a second
table.

---

## 3. The three candidate models for this campaign

### Model A — extend `ui!` and `.ui` in place

Add sprite/animation/interaction components; add their arms to `parse_and_insert` and the
serializer; extend the equivalence corpus.

* **Principle 0:** clean. Everything is a component.
* **Cost:** linear in the new vocabulary, paid twice (macro path is free — open vocabulary; text path
  is a hand-written arm each).
* **Ceiling:** `.ui` cannot express a Rust expression, so anything computed must be authored in
  Rust, and the two surfaces diverge in capability the way BSN's code/asset gap does. No styling, no
  reuse, no in-block names.

### Model B — `ui` as an Aether construct, lowering to the `ui!` surface

`aether! { ui hud { … } }`, parsed by `aether_lang`, emitting **the canonical hand-written surface**
— i.e. the same `cmds.spawn(...)`/`.insert(...)`/`add_child` shape `ui!` emits (Decision A3: one
expansion authority, zero drift). The construct gets `AetherCtx`, so `style: panel_dark`,
`sprite: ui_atlas.button_hover` and `on_click: Jump` resolve **in block** (M3) with spanned
diagnostics and a trybuild golden each.

* **Principle 0:** clean — it is a transpiler; the emitted code is what a person would have typed.
* **Gains:** in-block name resolution; `with`-style factoring; the `machine` construct's proven
  pattern for generating an enum + a system + its registration from a declaration (§6b).
* **Cost:** a third authoring surface with its own parser, its own diagnostics corpus, and its own
  recovery stubs — even though it shares the codegen.

### Model C — a retained view tree diffed against the world

A `View`-trait tree (Xilem) or a `Template` + dynamic-node split (Dioxus) rebuilt on state change
and diffed into the ECS.

* **Principle 0: violated.** The view tree is durable per-element data in a non-ECS structure. Xilem
  is explicit that the view tree is a *separate* tree from the widget tree.
* **Principle 1: violated in the dynamic cases.** Type-erasure (`AnyView` / `Box<dyn View>`) is the
  documented escape hatch for conditional and dynamic subtrees.
* **Redundant here.** `reconcile_ui` already does keyed diff-and-patch, and `BindText`/`BindValue`
  already do fine-grained property updates without any diff at all.
* **Counter-evidence from the closest peer:** Bevy shipped BSN in 0.19 with patches and **no
  reconciliation** — *"`Scene::resolve` applies the Scene as a 'patch'"* — after years of design
  discussion that considered reactive approaches (#14437 lists reactivity as post-MVP).

**Model C is the one to name as fighting this engine.** Not because diffing is wrong — this repo
diffs, in `reconcile.rs` — but because the thing being diffed there is a *parsed file against the
live world*, both of which already exist, whereas Model C manufactures a third representation whose
only purpose is to be diffed.

---

## 4. Recommendation

**Model B, sequenced after a measured slice of Model A**, with six specifics.

1. **`ui` becomes the tenth Aether construct; `ui!` remains its lowering target.** One expansion
   authority. Aether emits the `ui!`-canonical surface, and the existing `p3_equivalence` gate is
   extended to a third leg rather than a second gate being invented.

2. **Nesting generalises from `children:` to a relationship head.** `bsn!` and Flecs converged here
   independently, and this engine already has generic Relations. `children: [ … ]` stays as the
   sugar spelling of `ChildOf`.

3. **Handlers stay named actions.** `OnClick(u16)` and its siblings. No inline closures, no
   per-node observers. Advanced interactivity = more action-emitting components of the same 2-byte
   shape + payload columns written set-if-changed, following `RelativeCursorPosition`.
   *The Principle-violating pattern to name explicitly:* an `on_click: |…| { … }` clause storing a
   `Box<dyn Fn>` per node — standard in SwiftUI/Compose/Dioxus/Leptos/QML and now in `bsn!` — is a
   per-element `dyn` on durable data. The ECS-native replacement is the action index plus an
   ordinary system.

4. **Dynamic lists get `UiRepeat` + a key component + a pool system**, reusing the reconciler's
   identity discipline. Not a macro `for` loop (wrong: a macro cannot know a runtime length), not a
   virtual tree (wrong: Principle 0). The template is a prefab **entity**, not a struct.
   *The Principle-violating pattern to name explicitly:* a widget holding `Vec<ChildDescriptor>` to
   diff against — the SP4-race shape, a parallel data system.

5. **Styling = author-time patches and `with` scopes; never a runtime cascade.** Take `bsn!`'s patch
   semantics and Flecs's `with`; refuse USS selectors and runtime `IsA` chain walks. A style
   resolves to concrete component bytes before the entity is live.

6. **Animation takes QML's shape, as a component.** QML's `Behavior on x { NumberAnimation { duration: 300 } }`
   attaches the animation **to the property, as data**
   ([Qt animations](https://doc.qt.io/qt-6/qtquick-statesanimations-animations.html)) — the single
   best declarative-animation precedent in the survey, and it maps one-to-one onto a dense component
   column plus one system.
   *The Principle-violating pattern to name explicitly:* an animation track owned by a widget struct
   or a `HashMap<Entity, Track>` side store. A track is per-element durable data; it is a **dense
   component** (`docs/DENSE-COMPONENTS-PLAN.md`) and the tween is one system over that column,
   SIMD-shaped. QML's `states`/`transitions` map onto the same machinery — and Aether already has a
   `machine` construct for the state half.

**Name resolution, per surface** (§2.5): the macro path uses M1 (re-spell, let rustc resolve) plus
M3 (`AetherCtx` for in-block names); the text path uses M2 (registration tables). The concrete
action item is to stop hand-writing the `.ui` vocabulary and install the parse fn from the same
derive that already installs `BindAccessor`.

**Sequencing.** Ship the *component vocabulary* first (sprites, animation tracks, the new
action-emitting components) under Model A, because Aether cannot usefully name what does not exist,
and Aether's rule §7 item 6 (*"one table, spelling and dispatch together"*) means a construct whose
vocabulary is still moving will churn its own diagnostics corpus. Open the `ui` construct once the
component set stops moving.

---

## 5. Quick reference — what to take and what to refuse

| Pattern | Source | Verdict here |
|---|---|---|
| Relationship-as-nesting-head | `bsn!`, Flecs | **Take.** Engine has generic Relations. |
| Scene-as-patch, resolved before spawn | `bsn!` | **Take.** Equals `.insert` overwrite. |
| `with` scope factoring | Flecs script | **Take.** Author-time only, zero runtime cost. |
| Static template + dynamic-slot split | Dioxus | **Take the idea, not the machinery.** `ui!` already emits a static spawn; `BindText` already is the dynamic slot. |
| Explicit vs structural identity | React, SwiftUI, Compose | **Already have it** — `UiName` / `UiSourceOrder`. Extend to list keys. |
| Message enum instead of closures | Iced / Elm | **Already have it** — `OnClick(u16)` over `Actionlike::index()`. |
| Structure serializable *because* it holds no code | UXML/USS | **Take as a rule.** It is why hot reload works here at all. |
| `Behavior on <property>` animation-as-data | QML | **Take**, as a dense component + one system. |
| Runtime property-binding engine with dependency tracking | QML | **Refuse.** Per-property observers; `BindText`'s changed-gate is the flat equivalent. |
| Selector/cascade stylesheet | UXML + USS | **Refuse.** Runtime matcher = parallel data system. |
| Runtime `IsA` value inheritance | Flecs | **Refuse.** Pointer chase per read against dense columns. |
| Inline closure handler | SwiftUI, Compose, Dioxus, Leptos, `bsn!` `on()` | **Refuse.** Per-element `dyn`; not serializable. |
| View tree diffed into the widget tree | Xilem, Dioxus, React | **Refuse.** Third representation; Principle 0. |
| Immediate mode | egui | **Refuse.** 2340-line retained layout solver already exists; sizing paradox. |

---

## 6. The strongest arguments AGAINST this recommendation

### 6a. Model B multiplies the drift surface instead of dividing it — and the campaign's real bottleneck may be elsewhere

Today there are **two** lowerings held together by one equivalence gate. `lower.rs` opens by citing
the macro's line numbers so that *"a future drift between the two paths is detectable by line"* —
which is an admission that drift is expected, not hypothetical. Model B adds a third surface. Even
though it shares codegen, it does **not** share: a parser (`aether_lang/src/parse.rs`), a diagnostics
corpus (every diagnostic is a trybuild golden — `AETHER-LANG-PLAN.md` §7 item 3, and
`crates/aether_tests/tests/ui/` already holds ~35 golden pairs), or recovery stubs (§7.3, which
memory records as *having been listed since A0 and not actually existing*).

Meanwhile the gap the campaign is most likely to hit is the one BSN names as persistent and
unresolved: **`.ui` cannot express a Rust expression.** Aether does not close that gap — Aether's
`ui` block is a *macro*, so it has exprs, and the `.ui` file still does not. A third surface widens
the capability spread rather than narrowing it.

The honest counter-plan is Model A alone: spend the campaign on the component vocabulary and on
converting `parse_and_insert` from a hand-written match into a registration table, and give Aether
the `ui` construct only later, if at all.

**The measurement that decides this, and it is decidable:** enumerate the advanced-UI features this
campaign will ship and count how many require a **Rust expression at the authoring site** versus
pure literal data. A sprite sub-rect, a duration, an easing id, a tab index, an action name — all
literal data, and `.ui` + a registration table is sufficient; Aether would be sugar. A computed
colour, a bound expression, a conditional subtree — these need exprs, `.ui` structurally cannot host
them, and Aether's macro path becomes load-bearing rather than ornamental. **Run that count before
committing to B.**

### 6b. "No inline closures" is a real ergonomic tax, and the field is close to unanimous against it

Every system surveyed except Iced puts the handler at the call site: SwiftUI, Compose, Dioxus,
Leptos, QML — and Bevy, having considered three strategies in #14437 (observer syntax,
component-driven handlers, `Construct` observer), shipped the inline one. That is not fashion; it is
locality. The action-index model requires, per button: an `Actionlike` variant, its registration, a
system that reads `ActionState`, and a name that resolves. The button's behaviour then lives three
files from the button. A forty-button HUD is forty enum variants and a `match` nobody wants to read.

There are two honest responses and both should be recorded:

* **The trade is deliberate and Iced makes it on purpose.** Levien's survey lists Elm's verbosity as
  its known cost, alongside its known benefit: no shared mutable state. Here the benefit is larger
  than in Iced, because it is what makes the handler *serializable* — and hot reload
  (`reload/system.rs`, `reconcile.rs`) is a shipped feature this campaign must not break. A `.ui`
  file cannot contain a closure. That is not a limitation of the design; it is the reason the design
  was chosen.
* **The tax should be paid down with codegen, not with a `dyn`.** Aether's `machine` construct
  already generates a flat enum plus one drain-and-act fn per (leaf, event) plus the registration,
  from a declaration. An Aether `ui` block that accepts an inline handler *body*, and emits the
  `Actionlike` variant + the system + the registration + `OnClick(<variant index>)`, gives call-site
  locality **with the same POD runtime**. That is the compromise to design toward — and it is an
  argument *for* Model B that lives inside the argument against it.

---

## 7. Open questions for the owner (VALUES / SCOPE — not perf forks)

1. **Does `.ui` stay capability-equal to the macro path?** BSN accepted a permanent code/asset gap.
   Accepting the same gap here is legitimate but must be a decision, because the equivalence gate is
   currently written as if the two are equal.
2. **Is the `.ui` component vocabulary allowed to become a registration table** (derive-installed
   parse fns) rather than a hand-written `match`? This changes a load-bearing file and is the
   difference between linear and constant cost per new component.
3. **Does the campaign want call-site handler bodies** (§6b's codegen compromise), or is the
   three-files-away action model accepted as-is?
4. **Sprite atlas ownership:** `UiImage.texture` is a `u32` into a *"(future) UI texture table"*
   (`components.rs:441`). Whether that table is an asset-system column or a UI-local resource is a
   scope call this campaign must settle before the DSL can name a sprite.

---

## 8. Sources

**In-repo (read for this document):**
`crates/boyko_macros/src/ui.rs` · `crates/boyko_macros/src/lib.rs:438-490` (the `ui!` EBNF) ·
`crates/boyko_macros/src/actionlike.rs` · `crates/boyko_macros/src/relationship.rs` ·
`crates/boyko_ui/src/text/parser.rs` · `crates/boyko_ui/src/text/lower.rs:209` ·
`crates/boyko_ui/src/text/dispatch.rs:71,721` · `crates/boyko_ui/src/reload/reconcile.rs:107,210` ·
`crates/boyko_ui/src/interaction/action.rs` · `crates/boyko_ui/src/interaction/components.rs` ·
`crates/boyko_ui/src/binding/components.rs` · `crates/boyko_ui/src/binding/bindable.rs` ·
`crates/boyko_ui/src/components.rs:434-467` · `crates/boyko_ui/tests/p3_equivalence.rs` ·
`crates/aether/src/lib.rs` · `crates/aether_lang/src/parse.rs:68-76` ·
`docs/AETHER-LANG-PLAN.md` §3.7, §4, §7

**External:**

- [Bevy 0.19 release notes — BSN](https://bevy.org/news/bevy-0-19/)
- [Bevy PR #23413 — Next Generation Scenes: core scene system, `bsn!` macro, Templates](https://github.com/bevyengine/bevy/pull/23413)
- [Bevy Discussion #14437 — Bevy's Next Generation Scene/UI System](https://github.com/bevyengine/bevy/discussions/14437)
- [Dioxus — Templates and diffing](https://dioxuslabs.com/blog/templates-diffing/)
- [Dioxus — the `rsx!` macro](https://dioxuslabs.com/learn/0.7/essentials/ui/rsx/)
- [Leptos — the `view!` macro](https://docs.rs/leptos/latest/leptos/macro.view.html)
- [Raph Levien — Xilem: an architecture for UI in Rust](https://raphlinus.github.io/rust/gui/2022/05/07/ui-architecture.html)
- [Xilem — `View` trait](https://docs.rs/xilem_core/latest/xilem_core/trait.View.html)
- [Iced — repository / Elm architecture](https://github.com/iced-rs/iced) · [Iced book — First Steps](https://book.iced.rs/first-steps.html)
- [egui — README (immediate-mode trade-offs, sizing paradox)](https://github.com/emilk/egui)
- [Apple WWDC21 — Demystify SwiftUI (structural vs explicit identity)](https://developer.apple.com/videos/play/wwdc2021/10022/)
- [Jorge Castillo — Positional memoization in Jetpack Compose](https://newsletter.jorgecastillo.dev/p/positional-memoization-in-jetpack)
- [React — Reconciliation (keys)](https://legacy.reactjs.org/docs/reconciliation.html)
- [Flecs Script](https://www.flecs.dev/flecs/md_docs_2FlecsScript.html) · [Flecs Prefabs Manual](https://github.com/SanderMertens/flecs/blob/master/docs/PrefabsManual.md)
- [Qt — QML property binding](https://doc.qt.io/qt-6/qtqml-syntax-propertybinding.html) · [Qt — QML animations, `Behavior on`, states and transitions](https://doc.qt.io/qt-6/qtquick-statesanimations-animations.html) · [Qt Quick Compiler](https://doc.qt.io/qt-6/qtqml-qtquick-compiler-tech.html)
- [Unity — Introduction to UI Toolkit (UXML/USS, retained mode)](https://docs.unity3d.com/Manual/ui-systems/introduction-ui-toolkit.html)
- [The Rust Reference — Procedural macros (hygiene, absolute paths)](https://doc.rust-lang.org/reference/procedural-macros.html) · [RFC 1566 — Procedural macros](https://rust-lang.github.io/rfcs/1566-proc-macros.html)
