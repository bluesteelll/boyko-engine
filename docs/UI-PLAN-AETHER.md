# UI-PLAN — the Aether `ui` construct

**Campaign:** advanced UI/GUI for `boyko_ui` — sprites, animation, richer interactivity — and its
integration into the Aether DSL as a first-class construct.
**Branch:** `feat/ui-advanced` (worktree `D:/wt/ui`) · **Date:** 2026-08-21 · **Status:** plan, pre-implementation.

**Authority:** [`docs/UI-ADVANCED-ARCHITECTURE.md`](UI-ADVANCED-ARCHITECTURE.md) §7 (D25–D29), §3
(D7, D31, D32), §9, §11 item 8, §13 Q4.
**Evidence:** [`docs/UI-ADVANCED-RESEARCH-DSL.md`](UI-ADVANCED-RESEARCH-DSL.md) (the five axes, the
three models, §6's two arguments against) plus the three sibling research documents.
**Language rules this construct must obey:** [`docs/AETHER-LANG-PLAN.md`](AETHER-LANG-PLAN.md)
§6.1 (the closed keyword registry), §6.2 (`AetherCtx` is the only inter-construct channel), §7.1
(the error-message bar and the "defer what rustc reports better" rule), §7.2 (span policy, the
`__aether_` prefix), §7.3 (recovery stubs), §7.5 (the four-layer test pyramid), §8 R4 (the
anti-drift gate) and R8 (sugar never walls off the engine).
**Sibling plans:** [`docs/UI-PLAN-SPRITES.md`](UI-PLAN-SPRITES.md) ·
[`docs/UI-PLAN-ANIMATION.md`](UI-PLAN-ANIMATION.md) ·
[`docs/UI-PLAN-INTERACTION.md`](UI-PLAN-INTERACTION.md). This document does not restate their
component designs; §2 names exactly what it consumes from each and what it does *not*.

---

## 0 · How to read this

This is the ladder a developer walks. Each rung is independently landable and leaves the workspace
green. Each rung states **what lands**, **what gates it**, and a **red mutation** — a concrete edit
that must turn the gate red. A gate whose red nobody has seen is not a gate; this project has paid
for that lesson six times (`site.decode`, `LogSite.fields`, twelve unbuilt benches, `sample_shift=2`,
`intern_site`, and the UI render-generation gate verified dead in the architecture's §1).

Decisions are numbered **U<n>**, each with a reason and the alternatives rejected. Where this plan
departs from the architecture document it says so at the site and names the defect it replaces — §2
of that document sets the precedent, and a silent divergence between a plan and its authority is the
doc-rot this repo keeps recording.

Every claim about the tree carries a `path:line` and was established by reading in this worktree.
`graphify` CLI is not installed on this machine; orientation was Grep/Read.

---

## 1 · What this construct is, in one paragraph

`ui` becomes the **tenth Aether construct**. `ui hud { … }` expands to one `pub fn hud(mut
__aether_commands: Commands)` that spawns a UI entity tree — the same `spawn` / `.insert` /
`add_child` surface a person would type and the `ui!` macro emits — and the sibling `plugin`
registers it as a startup one-shot, exactly as `scene` is registered (`expand.rs:446`). It is a
**transpiler**: it introduces no runtime type, no runtime table, no `dyn`, no allocation, and no GPU
byte. Its whole runtime footprint is the component set it inserts, and that set is authored by the
user. **Nothing in this plan touches a shader** (§8).

---

## 2 · Dependencies — hard, soft, and none

The architecture sequences the construct **last** (§11 item 8) with a stated reason: *"Aether cannot
usefully name what does not exist, and … a construct whose vocabulary is still moving will churn its
own diagnostics corpus, every diagnostic of which is a trybuild golden."*

**That reason binds the sugar rungs and does not bind the core, and separating the two is this
plan's first finding.** An Aether node body accepts a **bare component expression** as its universal
fallback — the `entity { … }` shape §8 R8 requires and `SceneNode.extras` already documents as *"the
`ui!` fallback"* (`ast.rs:594`). A bare expression is a verbatim `syn::Expr`: Aether never learns the
component's name, its fields or its type. **So the construct can express every component this
campaign will ever add — including all 24 in D7a's two tables — on the day it lands, with zero
knowledge of any of them.** What the sugar rungs buy is a shorter spelling and a spanned diagnostic,
not a capability.

| Dependency | Kind | What is actually needed |
|---|---|---|
| **UI-PLAN-SPRITES** — `UiNineSlice`, `UiSpriteSheet`, `UiSpriteAnim` exist with final field lists | **hard, U4 only** | U4's `sprite:` / `nine_slice:` / `flipbook:` props name these types and their fields. Until they exist the author writes them as bare component expressions and loses nothing but characters. |
| **UI-PLAN-ANIMATION** — `UiStateTint` + the tween components, the easing spelling, the duration unit | **hard, U6 only** | U6's `states { hovered { tint: …, in: 120ms ease_out } }` (D28) lowers to inserts of these. |
| **UI-PLAN-INTERACTION** — `Draggable`, `DropTarget`, `Overflow`, `ScrollPosition`, `TextInput`, `FocusNeighbors`, `FocusGroup`, `Tooltip`, and `Focusable { tab_index }`'s final shape | **hard, U5 only** | U5's capability props. D28's rule — *"a prop that is absent emits nothing, so the archetype genuinely lacks the column"* — is a **property of the lowering**, not of the components, so it is designed here and enforced at U5. |
| **D7** — the `.ui` `#[derive(UiVocab)]` registration table (architecture rung 1) | **soft, and only for the equivalence gate** | The construct emits Rust, never `.ui` text, so D7 does not gate any rung here. It gates **how wide the three-way equivalence corpus can be**: a component `.ui` cannot spell has no third leg. See U11 and §13 Q2. |
| **D31** — `boyko-ui` promoted to a production dependency of `boyko_render` | **soft, cost only** | U0 adds `boyko-ui` to `aether-tests`' dev-dependencies (D27). If D31 has landed, `boyko_ui` is already in that crate's graph through `boyko-render` and the edge costs compile time only for the *names*. If it has not, the edge pays the whole ~25 k lines. Measured at §9 M2. |
| **D32** — the minimal `boyko_app` UI rung | **none** | The construct's gates are headless: an `App` that spawns and never presents, the a6_scene precedent (`a6_scene.rs:22-30`). |
| **D1 / D30** — the 80 B instance and the eDSL migration | **none** | Nothing here reaches the renderer. §8. |

**What the sugar rungs are hostage to is the *field spelling*, not the component.** A sugar prop
shipped against a component whose fields later change re-blesses its goldens — tracked as **RU3**.

---

## 3 · The measured starting position

| Fact | Anchor |
|---|---|
| Nine constructs; the registry is closed and every keyword dispatches | `parse.rs:68-88`, `diag.rs:14-15`, `ast.rs:181` |
| Exactly **two** `.stderr` goldens print the construct list verbatim | `tests/ui/unknown_construct.stderr:1`, `tests/ui/no_planned_construct_remains.stderr:1` |
| `scene` is the model: demand-driven params, `__aether_`-prefixed, `children:` → `add_child`, extras → `.insert` | `expand.rs:1383-1467`, `:1558-1604` |
| A `scene`'s spawn fn is registered by the sibling `plugin` as a startup one-shot, interleaved with startup systems in **block source order** | `expand.rs:437-451` |
| A `scene` with no `plugin` still emits its fn — unregistered, callable by hand. No error. | `expand.rs:437` (the filter never demands a plugin) |
| Node props are keyword-led; the `ident :` fork is a **single** colon, and everything else falls to a verbatim `syn::Expr` | `parse.rs:1492-1519` |
| `casts_shadow` is the precedent for a **non**-`ident :` contextual prop, peeked before the expr fallback | `parse.rs:1501-1514` |
| `ui!` requires a `UiLayout` per node and reports it at **`node.brace_span`** — the node's own brace | `boyko_macros/src/ui.rs:215`, `:318-322` |
| `UiNodeBundle` is **exactly** `{ layout: UiLayout, rect: ComputedRect }` | `bundles.rs:31-37` |
| `ui!`'s bundle fast path is a **spawn-shape** optimisation (the Phase-8.5 static archetype cache), not a different component set | `bundles.rs:18-26`, `ui.rs:447-492` |
| `UiName` inline cap is 60, mirrored in the macro as `UI_NAME_CAP` | `ui.rs:47` |
| `OnClick`/`OnHover`/`OnSubmit` are `#[repr(transparent)] u16` over `Actionlike::index()`; `NO_ACTION = u16::MAX` | `interaction/action.rs:14-35` |
| `Actionlike` is a root re-export of `boyko_input`; `fn index(self) -> usize` takes the variant by value | `boyko_input/src/lib.rs:45`, `actionlike.rs:50` |
| `::boyko_ui::OnClick` does **not** resolve — the canonical absolute path is `::boyko_ui::interaction::OnClick` | `boyko_ui/src/lib.rs:40-68` (root re-exports), `interaction/mod.rs:18` |
| `aether-tests` has **no** `boyko-ui` dependency today | `aether_tests/Cargo.toml:12-40` |
| The `.ui`≡`ui!` equivalence gate has **seven** cases and its comparator (`p3_common::assert_subtree_equiv`) is a test-local module | `boyko_ui/tests/p3_equivalence.rs:45-278` |
| `demo_arena.rs` claims *"every v1 construct in one block"* **in prose**, and is a gate | `aether_tests/tests/demo_arena.rs:1-5` |
| A unit test already pins `Stub::for_keyword` equal to `Construct::emits_fn`, keyword by keyword | `expand.rs:3984-3999` |

### One correction to the authority, at its source

**Architecture §7.1 D25 states that the construct's missing-`UiLayout` diagnostic is *"strictly
better than `ui!`'s error because the span points at the offending node rather than the macro"*.
That premise is false: `ui!` already reports at `node.brace_span`** (`ui.rs:215`, `:318`) — the
node's own brace, not the macro token. The construct's real advantage is different and is what U1
claims instead: the diagnostic fires **at parse, before expansion**, so it composes with §7.3
recovery (every sibling construct in the block still expands, one typo costs one error), and it can
name the node by its `#name` when it has one. Recorded here rather than quietly substituted, because
the two claims are checked by different tests.

---

## 4 · Decisions

### U1 — `ui` is the tenth construct and follows `scene`'s **shape**, not `ui!`'s **call**

`Construct` gains a tenth variant; `parse.rs`'s dispatch gains a tenth arm; `CONSTRUCT_KEYWORDS`
gains a tenth row (appended, so the printed order stays declaration order); `Stub::for_keyword` and
`Construct::emits_fn`/`fn_noun` gain `ui` as a **fn-producing** construct. The expander emits
`spawn` / `.insert` / `add_child` statements directly, mirroring `emit_node` (`expand.rs:1558`).

**Rejected — emit `::boyko_ui::ui! { … }` and let the macro do the lowering.** This is the
strongest rejected alternative in the plan and it is nearly right: it would give literal single-
expansion-authority (Decision A3), inherit the bundle fast path for free, and preserve spans because
Aether re-emits user tokens verbatim (§7.2(1)). It is refused for two reasons, both structural.
**(a)** A macro invocation inside macro output is opaque to Aether: the deferred-insert tail (U6),
the `UiRoot` synthesis (U3) and the per-node prop diagnostics all need Aether to *decide* what a node
emits, and a delegating expander can only append after the fact. **(b)** `ui!` binds `#name` to a
**user-visible `let`** (`lib.rs:469-473`); inside a generated fn that binding is unreachable, so the
delegation would carry a semantic that cannot mean anything here (see U4). The cost of the refusal
is one duplicated syntactic heuristic, bounded by U3 and pinned by U11.

**Rejected — a `ui` attribute macro or a separate `ui_aether!`.** Decision A1: cross-construct
references need one parse context, and a tenth surface with its own entry point is the drift
multiplication §6a of the research warns about, without the shared `AetherCtx` that makes B worth it.

### U2 — the grammar, and the one place it is not `scene`'s

```text
ui        := 'ui' NAME ( '(' 'actions' '=' PATH ')' )? '{' node* '}'
node      := ( '#' IDENT )? '{' body? '}' ';'?
body      := prop ( ',' prop )* ','?
prop      := 'children' ':' '[' node ( ',' node )* ','? ']'
           | 'on' handler ':' IDENT                                  (U5)
           | 'bind' bind_kind '{' key ':' value ( ',' … )* ','? '}'  (U6)
           | <sugar prop>                                            (U7 / U8 / U9)
           | EXPR                                                    -- the universal fallback
handler   := 'click' | 'hover' | 'submit'
bind_kind := 'text' | 'value'
```

Top-level nodes take an optional `;` terminator and `children:` lists are `,`-separated — **borrowed
verbatim from `scene`** (`parse.rs:1382-1385`, `:1550-1557`) rather than from `ui!`'s comma-separated
roots. Reason: intra-Aether consistency is what an author of a mixed block reads; a `ui` block that
punctuated differently from the `scene` two lines above it would be a rule to remember for no gain.

**The one place it is not `scene`:** a `ui` node has **no head keyword**. A scene node opens with
`mesh`/`sun`/`entity`; a `ui` node opens with `#name` or `{`. So there is no `NODE_HEADS` table and
no head-key tables — the prop set is uniform across every node, which is what a UI tree actually is.

**`on` and `bind` are contextual keywords peeked *before* the `ident :` fork**, following
`casts_shadow`'s precedent (`parse.rs:1501`). This is load-bearing: `on click: Confirm` is
`ident ident : expr`, which fails the single-colon fork (`parse.rs:1495-1498`), falls to the
expression fallback, parses `on` as a bare path expression and then reports *"expected `,` between
node props"* at `click` — a diagnostic that names neither the fault nor the feature. Verified by
reading the fork; the U5 golden pins the message the author actually gets.

**Reserved at their positions:** `children`, `on`, `bind`, and each sugar prop head. A component type
literally so named must be path-qualified — the same caveat `ui!` documents for `children`/`commands`
(`boyko_macros/src/lib.rs:475-477`).

**One table, spelling and dispatch together.** `UI_PROP_HEADS: &[&str]` is the *"expected one of"*
list, the did-you-mean candidate set, and the parser's dispatch — the `MATERIAL_KEYS` / `NODE_HEADS`
rule (`ast.rs:623-624`, `:797`). A row without an arm is a compile-time-visible fault under U11's
census test, not a runtime surprise.

### U3 — every node spawns `UiNodeBundle`; `ComputedRect` in a body is refused

Aether requires exactly one component expression per node whose **last path segment** is `UiLayout`
(the `head_ident_is` recognition, `ui.rs:352-364`, reproduced for one name only), and emits:

```rust
__aether_commands.spawn(::boyko_ui::bundles::UiNodeBundle {
    layout: <the UiLayout expression, verbatim>,
    rect:   ::boyko_ui::components::ComputedRect::default(),
})
```

**Reason, and it is a defect this decision prevents rather than a preference.** `UiNodeBundle` is
*exactly* `{ UiLayout, ComputedRect }` (`bundles.rs:31-37`), so the fast path and the slow path
produce the **same component set** — which means the equivalence gate, which compares component-id
sets and bytes, **cannot tell them apart**. An Aether that emitted `.spawn(UiLayout).insert(
ComputedRect::default())` would miss the Phase-8.5 static archetype cache on every node it ever
spawns, and every gate in this plan would stay green. Making the bundle base unconditional removes
the invisible fork entirely: there is nothing to get wrong, so nothing needs a gate that cannot fail.
The token shape is nonetheless pinned (U11), because "unconditional" is a property of the emitter.

**A `ComputedRect` expression in a node body is a spanned refusal** — *"`ComputedRect` is the layout
solver's output; the construct injects it"*. Reason: it preserves `ComputedRect`'s single-writer
invariant (`widgets.rs:6-9`) at the authoring surface, and it makes the bundle base unconditional
without a second heuristic. `.ui` keeps its `ComputedRect` arm (`text/dispatch.rs:106`) because a
`.ui` file round-trips a live world and must reproduce it; an Aether block has no round trip.

**Consequence, stated because it is a capability gap and not an oversight:** `ComputedRect` is one
component the three-way equivalence corpus cannot cover. §13 Q2 asks the owner to accept the class.

### U4 — top-level nodes get `UiRoot` synthesised; `#name` binds a **local**, never a user `let`

**`UiRoot` is inserted on every top-level node**, and an explicit `UiRoot` anywhere in a `ui` block
is a spanned error (*"a top-level `ui` node is a root; remove `UiRoot`"* at top level, *"a child node
is not a root"* inside `children:`).

*Reason:* a top-level node of a `ui` construct **is** a root by construction — there is no way to
express a non-root top-level node, because Aether cannot name a parent outside the block. A rule the
author can only get wrong is a rule the construct should carry, exactly as it carries `ChildOf`.
*Rejected:* diagnose-a-missing-`UiRoot` (matches `ui!`, but the diagnostic has no true negative — it
can only ever fire on an author who forgot); auto-insert *and* tolerate an explicit one (two spellings
of one fact, and a duplicate `.insert` of a ZST that no gate would notice).

**`#name` emits `UiName::new("name")` and binds `__aether_e<n>`** — an internal local, not a `let`
the user can see. This **differs from `ui!`**, where `#name` escapes the invocation
(`boyko_macros/src/lib.rs:469-473`). It cannot do otherwise: the construct expands to a `pub fn`
whose body is not the user's scope. The handle is reached at runtime the way `.ui` reaches it — by
querying `UiName`. Tracked as a migration trap, **RU2**, and stated in the generated fn's doc comment.

**Over-length and duplicate `#name`s are parse errors.** Over 60 bytes mirrors `UI_NAME_CAP`
(`ui.rs:47`); a duplicate mirrors `ui!`'s dup table and protects the reconciler's identity discipline
(`reload/reconcile.rs:107` matches survivors by `UiName`) — two nodes with one name make the keyed
diff non-deterministic, which is a *runtime* fault produced by an *authoring* mistake, exactly the
class §7.1 says Aether should pre-check.

**`UiViewport` is not touched and not diagnosed.** It is a host-supplied `Resource`; a proc macro
cannot see resources, and a diagnostic about one would be Aether pre-checking what it cannot know —
§7.1's rule. The generated fn's doc comment states the host requirement; U0's behavior test supplies
it.

### U5 — handlers lower to `OnClick(u16)` by token re-spelling, and a foreign path is refused

The construct names its action enum once, in its header, and each handler re-spells a bare variant:

```text
ui hud(actions = GameAction) {
    #start { UiLayout { .. }, UiBackground { .. }, on click: Confirm }
}
```

```rust
.insert(::boyko_ui::interaction::OnClick(
    <GameAction as ::boyko_input::Actionlike>::index(GameAction::Confirm) as u16
))
```

Both halves of that line are verified against the tree (§3): `::boyko_ui::OnClick` does **not**
resolve and `::boyko_ui::interaction::OnClick` does; `Actionlike` is a root re-export and
`index(self)` takes the variant by value.

* **The header is optional and required only if an `on` prop appears.** An `on` without it is a
  spanned error on the `on` keyword naming the header — not a `NO_ACTION` fallback.
* **The value is a bare `IDENT`, and a path (`Other::Thing`) is refused.** This is a correctness
  rule, not a style rule: `index()` is dense **within one enum**, so a foreign variant's index is a
  valid `u16` naming a *different* action. The failure would be a button that fires the wrong thing,
  silently, at runtime. The refusal names the header type.
* **Aether never emits `NO_ACTION`.** `NO_ACTION` exists for the `.ui` hot-reload path where a
  compile-time error is impossible (`text/dispatch.rs:721`'s `resolve_action_name` fallback). At a
  compile-time surface a typo must be a compile error — the interaction research's explicit demand.
  The gate is a trybuild golden whose *whole content* is that a misspelled variant fails to compile.
* **Duplicate `on click` in one node is a parse error**, following `duplicate `material:``
  (`parse.rs:1536`).

**Rejected — inline closure handlers** (`on click: |…| { … }`). A per-element `Box<dyn Fn>` on
durable data (Principles 0 and 1) that **cannot be serialised**, and hot reload is a shipped feature.
The near-unanimity of the field (SwiftUI, Compose, Dioxus, Leptos, QML, `bsn!`) is a real ergonomic
argument and is answered by the deferred codegen compromise, not by a `dyn` — D27, §13 Q1.

### U6 — `#name` references resolve through a **deferred-insert tail**, and only in `bind` props

`#name` cannot appear inside a component expression: a bare `#ident` is not valid Rust expression
syntax, which is precisely why `ui!` refuses value-position references
(`boyko_macros/src/lib.rs:471-476`). Aether has the same constraint for the same reason — a node's
component items are verbatim `syn::Expr`.

So the reference is a **prop**, not an embedded token, and it exists for the one field family whose
type is `Entity`:

```text
#hp_bar {
    UiLayout { .. },
    bind text { source: #player, comp: HEALTH_ID, field: 0, template: TemplateId::PERCENT }
}
```

`source:` takes `#IDENT`; every other key is a verbatim expression, validated positionally against a
`BIND_TEXT_KEYS` / `BIND_VALUE_KEYS` table — the `NodeKeySpec` discipline (`ast.rs:625-634`), reused
rather than reinvented. A reference to a `#name` declared **later in the tree** emits its `.insert`
after the whole tree instead of inline; an unresolved name is an `unknown_symbol`-shaped diagnostic
(`expand.rs:1503-1524`) over the block's declared `#name`s, with did-you-mean at edit distance ≤ 2.

**Reason:** this is what `.ui`'s two-pass runtime fixup already does (`text/dispatch.rs:47-62`'s
`BindParse::Deferred`), performed at compile time instead of at load time — one semantics, two
surfaces. **Rejected:** spawn-all-then-insert-then-link (makes forward references trivial but changes
the emission shape of *every* node rather than of the forward-referencing ones, and the equivalence
gate pins the resulting world, not the token shape, so the divergence would be unpinned); a fourth
name-resolution mechanism beside `ui!`'s dup table, `.ui`'s fixup list and `AetherCtx`.

### U7 — `ui` registers **no** `AetherCtx` symbol row, and participates in the duplicate-fn rule

`AetherCtx` is *"deliberately narrow: it carries the one symbol class a consumer actually resolves
against today"*, and its own doc states the rule — *"a table row nothing reads is a datum that rots"*
(`ctx.rs:44-49`). Nothing references a `ui` block by name, so `ui` adds no row.

It **does** join `Construct::emits_fn` (`ast.rs:215`) and `fn_noun`, because `ui hud` beside
`scene hud` or `material hud` is two `pub fn`s with one name, and §4's measurement is that rustc's
E0428 for that shape *puts both labels on the `aether!` token and names no user token anywhere*
(`ctx.rs:18-23`). Aether owns that diagnostic, with both spans. `ui` widens the class; it does not
add a case.

### U8 — sugar props emit a component **or nothing**, and absence is structural

D28's rule, made a property of the lowering: `draggable` emits `Draggable`; **an absent `draggable`
emits no token at all**, so the archetype genuinely lacks the column. No prop ever emits a
`bool`-carrying "off" component, and no prop ever emits a default-constructed marker "for symmetry".
This is capability-by-presence at the authoring surface, and the DSL is where it is easiest to get
wrong — a sugar table that mapped every prop to `Some(component)` with a `None` default would look
identical in the source and be wrong in the archetype.

**Gated by a per-node component-id-set assertion**, not by reading the emitted tokens: U7/U8/U9's
behavior tests assert the *absence* of the column on a node that omits the prop.

### U9 — styling and animation resolve at author time; no runtime cascade, ever

`states { hovered { tint: 0xFF3355FF, in: 120ms ease_out } }` lowers to `UiStateTint` + `TweenTint`
**inserts** — QML's `Behavior on <property>` shape, animation attached to the property *as data*,
with no new runtime. Refused, with the reasons the research gives: a USS-style selector cascade (a
parallel matcher over the tree — Principle 0) and Flecs's runtime `IsA` value inheritance (a pointer
chase per read against dense columns built to avoid one).

**The duration literal is an unverified spelling and is treated as one.** `120ms` relies on
`syn::LitInt` accepting a custom suffix. **There is no precedent for a custom literal suffix anywhere
in this repository** (grepped: the only `LitInt` use is `component.rs:732`, unsuffixed). U9 therefore
opens with a five-line `aether_lang` unit test that parses `120ms` and reads `LitInt::suffix()`; if it
does not hold, the fallback spelling is `in: ms(120)` — a call expression, unambiguously lexable —
and the rung continues. A rung that assumed the suffix and discovered it at implementation time would
have to re-cut its whole golden corpus.

### U10 — `UiDef` is `Box`ed in `Construct` from the first commit

`Construct::Material` is boxed because `MaterialDef` holds seven `syn::Expr` slots and `syn::Expr` is
a ~200-byte enum — inline, that one variant would have made every `Construct` 952 bytes
(`ast.rs:172-177`). `UiDef` carries a node **tree** of expressions and is strictly larger. Boxed from
the start, for the same reason `SceneNode` boxes `AtPose::Verbatim` (`ast.rs:605-608`). Build-time
allocation; the hot-path `Box` ban is a runtime rule.

### U11 — three gates, three questions, and the comparator is never duplicated

| Question | Gate | Where it lives |
|---|---|---|
| Does Aether emit the **tokens** the design specifies? | `aether_lang` unit snapshots (the `scene_fn` precedent — `expand.rs`'s `#[cfg(test)]` pins) | `aether_lang`, no engine dependency |
| Do the emitted **paths** resolve against the real engine? | compiling `aether-tests` (§8 R4) | `aether_tests`, needs the `boyko-ui` dev-dep |
| Does the emitted code produce the **same world** as the other two surfaces? | a **third leg** on the existing `.ui`≡`ui!` gate | `boyko_ui/tests/p3_equivalence.rs` |

**The third leg lives in `boyko_ui/tests/`, and `boyko_ui` gains `aether` as a dev-dependency.** The
comparator (`p3_common::assert_subtree_equiv` — entity count, per-node component-id set, per-node
component bytes, `ChildOf`/`Children` topology, child order, `UiName`) is a test-local module and
must not be duplicated into `aether-tests`: a comparator with two spellings is the "checking a list
against itself" shape this project has recorded five times. The edge is acyclic — `aether` →
`aether_lang` → no engine crate — and integration tests link their own crate by name, so the emitted
`::boyko_ui::…` paths resolve there. Cost measured at §9 M3.

**Rejected:** promoting the comparator to a `pub` `test-support` feature on `boyko_ui` (a workspace
feature with one consumer, and a public item whose only purpose is to be used by a test); duplicating
it (the drift shape); and giving Aether its own second equivalence gate (the research's §4.1 asks
explicitly for a third leg rather than a second gate).

### U12 — the corpus census replaces a prose claim with a check

`demo_arena.rs` opens with *"every v1 construct in one block"* and is a gate (`demo_arena.rs:1-5`).
That claim is **prose**, and this project has recorded prose-standing-in-for-a-test as a defect (the
L8c logging rung: a check whose satisfaction was a comment). U3 adds
`every_registry_keyword_appears_in_demo_arena` — a unit test that reads `CONSTRUCT_KEYWORDS` and
asserts each keyword occurs as a construct head in the demo source. Adding construct eleven without a
demo block then reds a test instead of silently falsifying a doc comment.

---

## 5 · Default OFF, and the one thing that is not

**The construct is off by structural absence.** There is no feature flag, no `cfg`, no runtime
switch: a workspace with no `ui` block emits nothing, spawns nothing and links nothing new. Every
image golden, every `.spv`, every `PINS.toml` hash is byte-identical **by construction** at every
rung of this plan, because no rung of this plan writes a byte that reaches a frame.

**The one global change, named because it is the only one:** registering the keyword changes the
canonical unknown-construct diagnostic (`diag.rs:25-33`) from *"… material, scene"* to *"… material,
scene, ui"*, which re-blesses exactly **two** `.stderr` goldens — `unknown_construct.stderr` and
`no_planned_construct_remains.stderr`. Both are re-blessed **at U0 and nowhere else**.

**Blessing discipline, quoted from the corpus that owns it:** *"a `.stderr` is re-blessed ONLY after
verifying the error KIND is unchanged — the `token_use_after_submit_rejected` lesson (87 commits red
because a line moved and nobody re-blessed)"* (`a6_diagnostics.rs:22-24`). For these two the kind is
`unknown construct`, the span is the same token, and the did-you-mean is unchanged; only the list
grows by one word. Anything else in the diff is a different failure wearing this one's clothes.

---

## 6 · The rung ladder

**Unconditional gate on every rung.** `cargo clippy -p aether-lang -p aether -p aether-tests -p
boyko-ui --all-targets -- -D warnings`; `cargo test -p aether-lang -p aether-tests -p boyko-ui
--all-targets --no-fail-fast`; the full existing trybuild corpus green with **no** un-named re-bless;
author-only commit. **Build with `-p <crate>`, never `--workspace`** — the worktree's disk budget;
an `os error 112` or a compiler ICE is the disk, not the code.

---

### U0 — the tenth construct: registry, grammar, universal fallback, roots, recovery — **size M**

**FIRST LANDABLE. Depends on nothing in this campaign** — it names only components that exist in the
tree today.

**Lands.** `Construct::Ui(Box<UiDef>)` + `UiDef`/`UiNode` in `ast.rs`; the `"ui"` row in
`CONSTRUCT_KEYWORDS` (appended); the parse arm and `parse_ui`/`parse_ui_node`/`parse_ui_prop`;
`UI_PROP_HEADS`; `Stub::for_keyword("ui") -> Stub::Fn`; `Construct::emits_fn`/`fn_noun`/`keyword`/
`name` arms; `ui_fn` in `expand.rs` (params `(mut __aether_commands: Commands)`, unconditional
`UiNodeBundle` base, `.insert(EXPR)` per extra, `add_child` per link); the startup registration in
the plugin's `startup_calls` filter (`expand.rs:437`); `boyko-ui` in `aether-tests`' dev-deps with
its blast-radius note beside the existing `boyko-render`/`boyko-app` ones; `aether` in `boyko_ui`'s
dev-deps; the two `.stderr` re-blesses.

**Gates.**
1. **`aether_lang` unit token pins** — the emitted statement sequence for: a leaf; a two-level nest;
   a three-level nest; two top-level roots; a node with four extras. The bundle base is pinned
   **literally** (`UiNodeBundle { layout: …, rect: ComputedRect::default() }`), because U3 says the
   fast path is invisible to every world-level gate.
2. **Registry parity — a gate that already exists and that the new keyword arms automatically:**
   `every_registry_keyword_stubs_in_the_item_kind_its_construct_emits` (`expand.rs:3984`).
3. **`UI_PROP_HEADS` census:** every row is fed to `parse_ui_prop` and must not take the unknown-prop
   path; and the unknown-prop message must print the table verbatim.
4. **trybuild goldens** (new): `ui_node_without_layout`, `ui_computed_rect_in_body`,
   `ui_root_in_a_child`, `ui_explicit_ui_root`, `ui_duplicate_name`, `ui_name_too_long`,
   `ui_unknown_prop_did_you_mean`, `ui_name_is_lowercase` (§2's case rule for a fn-producing
   construct), `ui_collides_with_a_scene_fn` (the §4 both-spans fault, widened across kinds).
5. **Recovery golden:** a block with a broken `ui` and a healthy sibling `component` — one error, the
   sibling still expands, the `ui` name still resolves as a fn stub (§7.3).
6. **Behavior test** (`aether_tests/tests/a8_ui.rs`, headless `App`, never presented): the tree
   spawns; **every top-level entity carries `UiRoot`**; child order matches declaration order; the
   parent's **`Children`** collection is asserted (not `ChildOf`) — the a6_scene rule, so the
   kernel's reactive half is proven; `UiName` present exactly on `#name`d nodes.
7. **Equivalence third leg** (`boyko_ui/tests/p3_equivalence.rs`): the five shapes of gate 1 built
   three ways in one world — `.ui`, `ui!`, `aether!` — compared pairwise with
   `assert_subtree_equiv`. The `ui!` leg spells `UiRoot` and `ComputedRect::default()` explicitly.

**Red mutations.**
* (a) Add `"ui"` to `CONSTRUCT_KEYWORDS` and **omit** the `Stub::for_keyword` arm ⇒ gate 2 reds. *This
  mutation is the reason U0 can be trusted: the gate predates the rung and cannot have been written
  to fit it.*
* (b) Omit `Construct::Ui` from `emits_fn` ⇒ `ui_collides_with_a_scene_fn` stops failing to compile ⇒
  trybuild reds ("expected compile failure, got success").
* (c) Swap the bundle base to `.spawn(<UiLayout>).insert(ComputedRect::default())` ⇒ gate 1 reds and
  **gates 6 and 7 stay green** — run it, and record that it stays green. That silence is the whole
  argument for pinning the token shape.
* (d) Drop the `UiRoot` synthesis ⇒ gate 6 reds; gate 7 reds only because the `ui!` leg spells
  `UiRoot` — which is why it does.
* (e) Emit `add_child(child, parent)` with the arguments swapped ⇒ gate 6's `Children` assertion reds
  (a `ChildOf` assertion would not distinguish the direction as cleanly).
* (f) Add a row to `UI_PROP_HEADS` with no parse arm ⇒ gate 3 reds.

---

### U1 — the diagnostic bar: spans, recovery, and the §7.1 line — **size S**

**Lands.** Span-column pinning across U0's goldens (§7.2, R2); the *"fires at parse, before
expansion"* property made observable; the §3 correction to D25 recorded in the construct's rustdoc so
the next reader does not re-derive it.

**Gate.** A golden in which **one** `ui` node is malformed inside a block that also declares a
`component`, a `material` and a `scene`: exactly one error, at the offending token, and all three
siblings expand (their items are nameable in the same file).

**Red mutation.** Make `parse_ui` return its error from the block level instead of through the
speculative-fork recovery path (`parse.rs:88-105`) ⇒ the sibling items vanish and the golden's
`.stderr` gains the "unresolved name" cascade §7.3 exists to prevent.

---

### U2 — the demo, the census, and the book page — **size S**

**Lands.** A `ui` block in `demo_arena.rs`; `every_registry_keyword_appears_in_demo_arena` (U12); a
`cargo expand` statement-count pin (§9 M1); the mdBook page — written by `doc-writer`, registered in
`book/src/SUMMARY.md`.

**Gate.** The census test; the expansion pin; `cargo test -p aether-tests` green.

**Red mutation.** Delete the `ui` block from `demo_arena.rs` ⇒ the census test reds. Before this rung
the same deletion falsifies only a doc comment and nothing notices — which is the defect class this
rung closes.

---

### U3 — handlers: the `actions` header and `on click|hover|submit` — **size S**

**Lands.** The optional `(actions = PATH)` header; the `on` contextual prop; `HANDLER_KINDS` (one
table: spelling, dispatch, and the did-you-mean set); the `<PATH as ::boyko_input::Actionlike>::
index(PATH::Variant) as u16` lowering into `::boyko_ui::interaction::{OnClick,OnHover,OnSubmit}`.

**Gates.**
1. Token pin per handler kind, with the **absolute path spelled out** in the expected tokens.
2. Behavior test: a node with `on click: Confirm` carries `OnClick` whose `.0` equals
   `GameAction::Confirm.index() as u16`; a node without carries **no** `OnClick` column.
3. trybuild: `ui_handler_without_actions_header`; `ui_handler_takes_a_bare_variant` (a path is
   refused); `ui_duplicate_on_click`; `ui_unknown_action_variant` — whose entire content is that a
   misspelled variant is a **rustc** error on the user's token.
4. `on click: Confirm` reports at `click`, not at `on` and not at the node.

**Red mutations.**
* (a) Emit `::boyko_ui::OnClick` (D27's own corrected defect) ⇒ `aether-tests` fails to compile.
  **Then remove `boyko-ui` from `aether-tests`' dev-deps and re-run: the wrong path goes green.** That
  is the demonstration that the dev-dependency is the gate and not paperwork — and it is the exact
  drift §8 R4 exists to prevent.
* (b) Fall back to `NO_ACTION` for an unknown variant ⇒ `ui_unknown_action_variant` stops failing to
  compile ⇒ trybuild reds.
* (c) Emit `OnHover` for `on click` ⇒ gate 2 reds on the component id, not on the value.
* (d) Peek `on` **after** the `ident :` fork instead of before ⇒ gate 4's message becomes *"expected
  `,` between node props"* and the golden reds.

---

### U4 — `bind text` / `bind value` and the deferred-insert tail — **size M**

**Lands.** The `bind` contextual prop; `BIND_TEXT_KEYS` / `BIND_VALUE_KEYS` positional tables;
`#IDENT` as the `source:` value shape; the block-local `#name` symbol table; the deferred tail for
forward references; the `unknown_symbol`-shaped diagnostic with did-you-mean.

**Gates.**
1. Token pin: a **backward** reference inserts inline; a **forward** reference inserts after the last
   node — two pins, and the difference between them is the rung.
2. Behavior test: a forward-referencing `BindText.source` equals the entity the later `#name` spawned
   (resolved by reading the component, not by counting statements).
3. trybuild: `ui_bind_unknown_name` (with did-you-mean at edit distance ≤ 2);
   `ui_bind_source_is_not_a_name` (a bare expression in `source:`); `ui_bind_unknown_key`.
4. Equivalence leg: the same bind tree authored in `.ui` (whose `#name` source is `BindParse::
   Deferred`, `text/dispatch.rs:47-62`) and in Aether produce identical worlds.

**Red mutations.**
* (a) Emit the forward reference inline ⇒ the emitted code fails to compile (E0425 on
  `__aether_e<n>`), in-repo, at `aether-tests`.
* (b) Emit the tail with the wrong node local (off-by-one on the counter) ⇒ gate 1 stays green and
  **gate 2 reds** — the reason gate 2 reads the component instead of the tokens.
* (c) Resolve `#missing` to a synthesised `Entity::PLACEHOLDER` instead of erroring ⇒
  `ui_bind_unknown_name` flips from compile-fail to compile-pass.

---

### U5 — sprite sugar — **size S** · **blocked on UI-PLAN-SPRITES**

**Precondition:** `UiNineSlice`, `UiSpriteSheet`, `UiSpriteAnim` exist with final field lists, and
the sprite plan has decided how an author names a texture (a `TextureGpu` bindless slot expression,
per D2/D3). Until then the same trees are authorable through the universal fallback — this rung buys
spelling, not capability.

**Lands.** `sprite:`, `nine_slice:`, `flipbook:` props with their positional key tables; if the
texture name requires a resource, the **demand-driven signature becomes real** for the first time
(the `scene_fn` param-set computation, `expand.rs:1411-1424`) and `ui_fn` grows its second param.

**Gates.** Token pins per prop; behavior test asserting the component's presence **and** the absence
of every prop's column on a node that omits it (U8); `ui_unknown_sprite_key` golden; the equivalence
leg extended **only for the components `.ui` can spell** — which is D7's reach, and the gate's doc
must list what it therefore does not cover.

**Red mutations.** (a) Map `nine_slice:` to `UiSpriteSheet` ⇒ the behavior test reds on the component
id. (b) Emit a default-constructed `UiSpriteSheet` when the prop is absent ⇒ the absence assertion
reds — the D28 rule made mechanical. (c) Add the prop head to `UI_PROP_HEADS` without an arm ⇒ U0's
census reds.

---

### U6 — animation sugar: `states { … }` — **size M** · **blocked on UI-PLAN-ANIMATION**

**Precondition:** `UiStateTint` + the tween components exist; the easing identifier set and the
duration unit are fixed. **This rung opens with the `120ms` suffix verification** (U9) before any
golden is cut.

**Lands.** The `states` prop; the state-name table (`hovered`, `pressed`, `focused`, `disabled` —
one table, spelling and dispatch together, and the set is exactly the set the animation plan's
`Interaction`-driven transition system reads); `in:`/`out:` durations and easing ids; the lowering to
`UiStateTint` + `TweenTint` inserts, **no new runtime**.

**Gates.** The suffix probe (its own unit test, landing first); token pins; behavior test — a node
with `states { hovered { … } }` carries `UiStateTint` with the authored bytes and **no** running
tween row at spawn (the tween is started by `Changed<Interaction>`, D14, not by the DSL);
`ui_unknown_state_name` with did-you-mean; `ui_states_on_a_node_without_interaction` if the animation
plan makes that a fault.

**Red mutations.** (a) Insert a `TweenTint` **row** at spawn instead of only the authored
`UiStateTint` ⇒ the "no running tween at spawn" assertion reds — the DSL emitting runtime state is
exactly D7a's excluded class, one layer up. (b) Accept a state name absent from the table ⇒ the
did-you-mean golden reds. (c) Skip the suffix probe and cut goldens against `120ms` ⇒ if the suffix
does not lex, every golden in the rung is re-cut; the probe exists so that discovery costs five lines.

---

### U7 — interaction sugar — **size S** · **blocked on UI-PLAN-INTERACTION**

**Precondition:** `Draggable`, `DropTarget`, `Overflow`, `ScrollPosition`, `TextInput`,
`FocusNeighbors`, `FocusGroup`, `Tooltip` exist with final shapes, and `Focusable`'s `tab_index` is
settled.

**Lands.** `draggable`, `drop_target`, `scroll x|y|both`, `focusable(N)`, `focus_group`,
`tooltip: "…"`, `text_input` — each a row in `UI_PROP_HEADS`, each emitting **a component or
nothing** (U8).

**Gates.** Per-prop token pin; per-prop presence/absence behavior assertion; `ui_scroll_axis_unknown`
golden; the equivalence leg for each component `.ui` can spell.

**Red mutations.** (a) Emit `Overflow { scroll_x: false, scroll_y: false }` for an absent `scroll` ⇒
the absence assertion reds. (b) Emit `Focusable::default()` for `focusable(3)` (dropping the index) ⇒
the byte comparison reds where a presence-only assertion would not — which is why the assertion
compares bytes.

---

### U8 — `with` factoring scopes — **size S** · **owner call, §13 Q3**

Author-time factoring with **zero runtime representation** — Flecs's `with` scope, the ECS-native
half of "styling". `with UiBackground { .. }, Focusable { .. } { … nodes … }` inserts the listed
component expressions into every node in the scope, resolved before the entity is live.

**Deferred by default.** It is the answer to §6b's forty-button tax and it is pure sugar over U0's
fallback, so it can land at any time or never. Recorded with its shape so the deferral is a decision.

**Gate if built.** Token pin showing the scope's components inserted **per node** with the node's own
props winning a collision (patch semantics, right-to-left, `bsn!`'s rule); a golden for a `with`
scope containing a `children:` prop (refused — `with` scopes nodes, not props).

---

## 7 · Deferred, each with its reason

| Deferred | Why the line is here |
|---|---|
| **Inline handler bodies** (D27) | The right long-term answer — `machine` already generates a flat enum + a drain-and-act fn + its registration from a declaration, so the pattern is proven. But it must synthesise an enum that coexists with the user's own action enum, and that interaction is a language-design question that must not ride in the construct's introducing rung. §13 Q1. |
| **`UiRepeat` dynamic lists** (D29) | The shape is fixed (a container carrying `UiRepeat { source, template, key_field }`, the template a disabled prefab **entity**, instances stamped with `UiListKey`). Its dependency is the reconciler's identity discipline extended to list keys — its own rung, in `boyko_ui`, not in the DSL. The DSL's job when it lands is three fields. |
| **A `.ui` ⇄ Aether transpiler** | Tempting and wrong: it would make the two surfaces' capability gap a *runtime* error instead of a design decision. The gap is settled by §13 Q2 instead. |
| **A relationship-general nesting head** (`Children [ … ]` as `bsn!` spells it) | `bsn!` and Flecs converged on relationship-as-nesting-head and this engine has generic Relations (`boyko_macros/src/relationship.rs`). But `children:` is what `ui!` and `.ui` both spell, and the three-way equivalence gate is worth more than the generality — which no UI tree in this campaign needs. Revisit when a second relationship wants a nesting head. |
| **`aetherfmt` / tree-sitter grammar for `ui` blocks** | §7.4's standing non-goal, unchanged by a tenth construct. |

---

## 8 · Shaders — none, and why that is worth stating

**No rung in this plan touches a shader.** There is no eDSL leaf, no `// === GENERATED … ===`
sentinel, no `*_edsl_sync` / `*_spv_sync` pin and no
[`docs/SHADER-VARIANT-MANIFEST.md`](SHADER-VARIANT-MANIFEST.md) row owed here. The construct emits
component inserts; the bytes that reach a frame are produced by the pack, which this plan never
names.

Stated explicitly because the campaign **does** carry those obligations — D30 migrates the UI leaves
into `boyko_shaderdsl` with both sync gates and a manifest row per variant, and D1 widens
`UiInstance` across seven lockstep sites including two HLSL mirrors and two `SpirvBlob<N>` pins. All
of that lands in **UI-PLAN-SPRITES**, sequenced *before* the widening it observes. A reader who looks
for a manifest row here should find this paragraph instead of silence.

---

## 9 · Measurement obligations

**This construct has no runtime term.** It emits the surface a person would type, so a criterion
bench over "Aether-spawned versus hand-spawned" would be an instrument with no subject — the two are
the same statements. The claim that they are the same is a **gate** (U11's third leg and the token
pins), not a number. Saying so here is deliberate: this campaign's recorded failure is a number that
was not measurable and a gate that could not fail, and the correct response to "no runtime term" is
to name the gate, not to invent a benchmark.

The numbers that do exist are compile-time, and each names its instrument and its discriminating
comparison.

| # | Claim under test | Instrument | Discriminating comparison |
|---|---|---|---|
| **M1** | Aether adds no statement the hand-written surface would not have | `cargo expand -p aether-tests --test a8_ui`, statements counted per node | The Aether leg and the `ui!` leg for the **same five trees** must emit the same statement count. A higher count means the transpiler added runtime work; a lower one means it dropped something the equivalence gate did not reach. Pinned as a test, re-blessed only with a named reason. |
| **M2** | `boyko-ui` in `aether-tests` is affordable, and D31 amortises it | cold `cargo check -p aether-tests` (the recorded precedent: `boyko-render` took it ~1 s → **~31.7 s**, `aether_tests/Cargo.toml:27-28`) | Before/after the edge, **twice**: with D31 landed and without. If D31 has landed the delta should be near zero, because `boyko_ui` is already in the graph — and if it is not near zero, D31 did not land the way it claims. |
| **M3** | `aether` in `boyko_ui`'s dev-deps is affordable | cold `cargo check -p boyko-ui --all-targets` | Before/after. `aether` → `aether_lang` names no engine crate, so this is `syn`-with-`full` plus ~7 k lines; if it is not, something took a dependency it should not have. |
| **M4** | The construct's own compile cost does not grow superlinearly with tree size (§8 R1) | `cargo check -p aether-tests` on a synthetic block | 8, 64 and 256 nodes in one `ui` block. Reported, not gated — R1's mitigation is the two-crate split, and this is the number that would say the split stopped working. |

---

## 10 · Risks

### RU1 — three surfaces, one comparator, and a corpus narrower than it looks

Today two lowerings are held together by one equivalence gate, and `lower.rs` opens by citing the
macro's line numbers so *"a future drift between the two paths is detectable by line"* — an admission
that drift is expected. This plan adds a third. The mitigation is U11's third leg on the **existing**
gate with the **existing** comparator.

*The residual, stated because the mitigation does not remove it:* the three-way corpus can only cover
components **all three surfaces can spell**. `ComputedRect` is outside it by U3's own decision; every
runtime-state component is outside it by D7a's safety property; everything needing a Rust expression
is outside `.ui` structurally. A gate whose coverage is the *intersection* of three vocabularies must
say so in its own header, or a future reader will read "equivalence" as "parity". U0's gate 7 carries
that sentence, and §13 Q2 asks the owner to accept the class.

### RU2 — `#name` means something different here, and the difference is silent

In `ui!`, `#name` escapes the invocation as a `let`. In an Aether `ui` block it cannot: the body is a
generated fn. An author migrating a `ui!` block will write `#panel` and then reach for `panel`, and
get an unresolved-name error inside macro output. *Mitigation:* the generated fn's doc comment states
it; the mdBook page (U2) states it beside the `ui!` grammar; and U0's goldens include the shape.
*Residual:* a doc comment is not a gate. There is no way to diagnose "the author expected a binding",
so this one is documentation and nothing more, and saying that is better than implying a gate.

### RU3 — the sugar rungs are hostage to field spellings that are not final

U5, U6 and U7 each name components the sibling plans have not written. A field renamed after a sugar
prop ships re-blesses that prop's goldens and its token pins. *Mitigation:* the three sugar rungs are
sequenced **after** their siblings' components land and are ordered independently of each other, so a
slipping sibling blocks one rung rather than the ladder; and the universal fallback means the
capability is available in the meantime, so no rung is on the critical path of anything a user can do.
*Residual:* U6's `states` table also encodes the *state name set*, which is a shared vocabulary with
the animation plan's transition system — that is a second spelling of one fact and the only sugar
rung where the drift is semantic rather than syntactic. It is the one to review against its sibling
before cutting goldens.

### RU4 — a re-bless is where a real regression hides

U0 re-blesses two `.stderr` files. This project's memory names the exact failure: a trybuild fixture
red for 87 commits because a line moved and nobody re-blessed, and its inverse — a golden re-blessed
past a *kind* change. *Mitigation:* §5 states the expected diff to the word: the list grows by `, ui`
and nothing else moves. Any other hunk in either file stops the rung.

### RU5 — the construct is the last rung of the campaign, and the last rung is where scope arrives

§11 sequences Aether eighth. By then the vocabulary is settled and every reviewer's remaining wish is
a DSL feature. *Mitigation:* §7 fixes the shape of the four deferred items so each is a decision with
a recorded answer rather than an open invitation; and U0 is landable **now**, independently, which
converts "the DSL rung" from one large terminal item into a small early one plus three sugar rungs
that follow their own subjects.

---

## 11 · Open questions for the owner (VALUES / SCOPE — also to be filed in [`docs/OPEN-QUESTIONS.md`](OPEN-QUESTIONS.md))

These are not perf or architecture forks; those are decided above.

1. **Call-site handler bodies.** D27 defers the codegen compromise (an inline handler body
   synthesising an `Actionlike` variant + the system + the registration + `OnClick(<index>)`). Is it
   wanted in this campaign, in a later one, or refused? The three-files-away action model is
   deliberate and Iced makes the same trade on purpose — but the field is near-unanimous the other
   way, and `machine` already proves the codegen pattern in this very DSL.
2. **`.ui` capability parity, and what the equivalence gate therefore means.** The Aether surface
   admits Rust expressions and refuses `ComputedRect`; `.ui` admits neither expressions nor runtime
   state. BSN accepted a permanent code-vs-asset gap. Accepting the same gap here is legitimate but
   must be a **decision**, because the gate is currently written as if the surfaces are equal.
   (This is the architecture's §13 Q4, sharpened by U3.)
3. **`with` scopes (U8).** Pure author-time factoring, zero runtime, and the answer to the
   forty-button tax — or one more grammar to learn. Owner call; the rung is written either way.
4. **The duration spelling.** `in: 120ms` (pending the suffix probe), `in: ms(120)`, or `in: 0.12`
   seconds? A syntax-values call, and the answer is cheap before U6 and expensive after.

---

## 12 · Sources

**In-tree, read for this document** (worktree `D:/wt/ui`, branch `feat/ui-advanced`):
`crates/aether_lang/src/{parse,ast,ctx,diag,expand}.rs` — the block parser and speculative recovery
(`parse.rs:34-108`), `registry_keyword` (`:158`), `parse_node`/`parse_prop`/`parse_node_body`
(`:1319-1562`), `Construct`/`Stub`/`NodeKeySpec`/`SceneNode` (`ast.rs:60-200`, `:550-644`, `:797`),
`AetherCtx` and the duplicate-fn rule (`ctx.rs:1-120`), `CONSTRUCT_KEYWORDS` (`diag.rs:14`),
`scene_fn`/`emit_node`/`spawn_call` (`expand.rs:1350-1640`), the plugin's startup registration
(`:437-451`), the stub/emits-fn parity test (`:3984`) ·
`crates/aether_tests/Cargo.toml` (the R4 anti-drift and blast-radius discipline) ·
`crates/aether_tests/tests/{a6_scene,a6_diagnostics,demo_arena}.rs` and `tests/ui/` (74 golden files;
the two that print the construct list) ·
`crates/boyko_macros/src/ui.rs` (the `ui!` parser, `head_ident_is`, `UI_NAME_CAP`, the missing-
`UiLayout` span) and `src/lib.rs:438-500` (the EBNF and the `#name` contract) ·
`crates/boyko_ui/src/lib.rs:40-100` (root re-exports vs `prelude`) ·
`crates/boyko_ui/src/interaction/action.rs` · `crates/boyko_ui/src/text/dispatch.rs:1-140`
(the closed vocabulary, the type-directed rule, `BindParse`) ·
`crates/boyko_ui/src/bundles.rs:1-37` · `crates/boyko_ui/tests/p3_equivalence.rs` ·
`crates/boyko_input/src/lib.rs:45` · `crates/boyko_input/src/action/actionlike.rs:45-60`

**Campaign documents:** [`UI-ADVANCED-ARCHITECTURE.md`](UI-ADVANCED-ARCHITECTURE.md) (the authority),
[`UI-ADVANCED-RESEARCH-DSL.md`](UI-ADVANCED-RESEARCH-DSL.md) (the evidence, and the external citation
list for `bsn!`, Flecs, QML, Iced, Xilem, Dioxus, Leptos, SwiftUI, Compose, UI Toolkit, React and
egui — not duplicated here), [`AETHER-LANG-PLAN.md`](AETHER-LANG-PLAN.md) (the language's own rules),
[`PARTICLES-PLAN.md`](PARTICLES-PLAN.md) (the plan-register shape this document follows).
