# Aether — the boyko-engine authoring DSL (design plan)

## Goal

**Aether** is a simpler-syntax language for writing engine-level game code fast:
components, bundles, systems, events, reactive **state machines**, PBR materials,
and scene/prefab trees — all authored in one compact notation and **transpiled to
native Rust by a proc-macro at compile time**. There is no runtime, no
interpreter, no build step outside cargo: an `aether! { … }` block IS ordinary
Rust items after expansion, and those items are **exactly the code a disciplined
engineer would hand-write against today's engine APIs** — the same
`#[derive(Component)]`, the same `Query<D, F>` signatures, the same
`MeshBundle::new`, the same `Material::new`.

Owner's sketch (normative for the flavor of the surface syntax):

```text
component x {
    year: u32,
    day: u32
}

system f (...) { ... }
```

### Design goals

1. **Zero-cost transpilation (prime directive).** The emitted Rust must be
   byte-comparable to disciplined hand-written engine code. No hidden
   allocations, no `dyn`, no `HashMap`, no wrapper structs, no runtime registry,
   no reflection. If a construct cannot be expanded to zero-cost engine-native
   code, it does not ship.
2. **Builds ON `boyko_macros`, never bypasses it.** Aether emits *annotated
   Rust items* (`#[derive(Component)]`, `#[derive(Bundle)]`, `#[event]`, …) and
   lets the existing derives do the heavy lifting. One expansion authority per
   concern; Aether is a syntax layer, not a second codegen path for component
   registration.
3. **Great diagnostics.** Every error points at the offending token in the DSL
   source with a message naming what was expected — span-preserved through
   `proc_macro2`/`syn`, golden-tested with `trybuild`.
4. **rust-analyzer-friendly.** All user expressions, types, and statement bodies
   pass through as verbatim token streams (never re-lexed), so completions,
   go-to-definition, and hover work inside Aether blocks — the property Bevy's
   `bsn!` demonstrated is achievable for a function-like game DSL.
5. **Cleanly extensible.** Construct N+1 (e.g. `animation`, `shader`) is added
   by registering one parser + one expander module, without touching existing
   constructs (§6).

### Non-goals (v1)

- **No runtime or interpreter** — Aether does not exist at run time.
- **No build step outside cargo** — no codegen scripts, no `.aether` files on
  disk, no asset-pipeline compiler. (A `.aether` asset format could layer on
  later exactly as Bevy plans `.bsn` files on top of the `bsn!` macro.)
- **No parallel data system** — Aether introduces zero storage of its own;
  everything lands in `ComponentPool` columns, Resources, or existing asset
  tables (CLAUDE.md Principle 0).
- **No hand-editing of shader text** — the `material` construct parameterizes
  the frozen `MaterialGpu` PBR authority and *composes with* `boyko_shaderdsl`
  at a defined seam (§3.6); it never generates or edits HLSL.
- **No third-party construct plugins** — the construct registry is closed in v1
  (§6.4).
- **No editor tooling** — TextMate grammar / syntax highlighting is explicitly
  later; v1 rides on rust-analyzer through span preservation.
- **No entity-scoped (per-entity) state machines** — v1 machines are app-scoped
  (`States` resources); the per-entity variant has a designed v2 path (§5.5).

---

## Prior-art survey — what we take and what we avoid

| Source | Lesson taken |
|---|---|
| **Dioxus `rsx!`** — the DSL lives in a dedicated `dioxus-rsx` library crate, *separate from* the proc-macro crate, "to enable tooling like autoformat, translation, and AST manipulation" | **Two-crate split**: `aether_lang` (parser/AST/expander, a normal library on `proc-macro2`) + `aether` (the thin `proc-macro = true` facade). The parser is unit-testable without a compiler session, and future tooling (formatter, migration scripts) reuses it. |
| **Bevy BSN (`bsn!`, PR #23413)** — an ergonomic Rust-ey scene notation that "plays nicely with Rust Analyzer and supports autocomplete, go-to definition, semantic highlighting" | A function-like macro CAN have first-class IDE behavior **if** every user-authored fragment stays a verbatim, span-preserved Rust token tree. Aether adopts the same rule: sugar the *structure*, pass the *code* through untouched. Also validates the "macro now, asset file later" sequencing for scenes. |
| **Leptos `view!` / Yew `html!`** — JSX-ish macros; DX friction concentrates where the macro re-interprets tokens (formatting requires `leptosfmt`; autocomplete degrades inside macro-invented syntax) | Minimize macro-invented syntax in *expression* positions. Aether keywords lead each clause; everything after a `:` or inside `{ }` bodies is plain Rust. |
| **`statig`** — hierarchical state machines: superstates, entry/exit actions, `no_std`, "state machines defined in ROM and no heap memory allocations"; its docs argue the **typestate pattern is wrong for dynamic systems** (events arrive at run time, so compile-time state types buy boilerplate, not safety) | Harel-lite semantics (hierarchy, guards, entry/exit) map to a **flat enum + static dispatch**, resolved at expansion time. Aether rejects typestates for game states and rejects `statig`'s trait-object-free but still *runtime-walked* hierarchy in favor of **compile-time flattening** (§3.5): the transpiler knows the whole chart, so superstate handler inheritance and LCA entry/exit sequences are computed during expansion — zero runtime hierarchy walk. |
| **Ferrous Systems, "Testing proc macros"** — `trybuild` for diagnostics-goldens ("avoid regressions … that mistakenly make an error no longer trigger or be less helpful"), `cargo expand` for debugging, `syn::Error::new_spanned` for placement | The Aether test pyramid (§7): trybuild diagnostic goldens + `macrotest` expansion snapshots + runtime behavior tests, all in a dedicated integration crate that depends on the real engine. |
| **nnethercote, "How much code does that proc macro generate?"** — expansion volume is invisible and is the real compile-time cost driver | Expansion-size discipline: Aether emits the *minimal* hand-written form (it reuses `boyko_macros` derives instead of inlining their output), and rung A7 pins an expansion-size measurement into CI (§8 R1). |
| **`syn`** — `Parse` trait + `syn::custom_keyword!` for non-Rust grammars over Rust-lexable tokens | The whole Aether grammar is deliberately **Rust-lexable** (identifiers, `:`, `=>`, braces), so `syn`'s tokenizer, spans, and error machinery apply unchanged. No custom lexer. |
| **Engine-internal precedent: `ui!`** (`boyko_macros::ui`) — a function-like entity-tree macro with an EBNF grammar in its doc comment, expanding to `cmds.spawn(...)`+`add_child` chains, with `#name` let-bindings | The scene construct (§3.7) is `ui!`'s proven shape generalized to 3D render objects. The `ui!` macro also proves the repo already accepts function-like tree DSLs. |
| **Engine-internal precedent: `boyko_shaderdsl`** — "NO runtime AST and NO transpiler" *for shader math*: dual-instantiation via a `FieldScalar` generic | Different problem, same principle: the artifact that ships is ordinary monomorphized Rust. Aether's transpiler runs at compile time only; like shaderdsl's `emit` feature (which freely uses `Vec`/`String`), the transpiler's internals are build-time tooling and exempt from hot-path rules — the INVIOLABLE principles govern the **emitted** code. |

---

## 1. The macro architecture (Decision A1 — the core decision)

### Decision A1: ONE umbrella function-like macro, `aether! { … }`, in item position

```rust
use aether::aether;

aether! {
    plugin Movement;

    component Velocity { linear: Vec3 }

    system apply_velocity(q: query<(&mut Transform, &Velocity)>, time: res<Time>)
        on update
    {
        for (t, v) in &mut q { t.translation += v.linear * time.delta_secs(); }
    }
}
```

One `aether!` block expands to a flat list of top-level Rust items (structs,
enums, fns, one `Plugin` impl). Multiple blocks per crate/module are the normal
granularity — one block per feature, exactly like one module per feature.

**Why one umbrella macro and not the alternatives:**

| Alternative | Verdict | Reason |
|---|---|---|
| **Per-construct function-like macros** (`component! {}`, `system! {}`) | **Rejected** | Kills the load-bearing feature: **cross-construct references**. A `system` names a sibling system for `after` ordering; a `scene` names a sibling `material`; a `plugin` collects sibling systems and machines. That requires one shared parse context (`AetherCtx`, §6.2) over one token stream. Per-construct macros would need global mutable state between macro invocations — expansion-order-dependent and forbidden. Also N imports, N parser entry points, N places to version. |
| **Item-level attribute macros** (`#[aether] component …`) | **Rejected** | Attribute macros can only decorate **syntactically valid Rust items** — the compiler must find the item's boundaries before the macro runs. The owner's surface (`system f (...) { ... }`, `machine G { state A { on E => B; } }`, `material gold { base: … }`) is *not* valid Rust item syntax, and making it valid would surrender exactly the simplification the owner asked for. Attribute macros remain the right tool where the item IS Rust (that niche is already served by `boyko_macros`). |
| **A `macro_rules!` token-muncher** | **Rejected** | The grammar has real nesting (hierarchical states, scene trees), needs real diagnostics (arbitrary-span errors, "did you mean"), and needs a symbol table. Munchers give none of that and their recursion cost explodes on large blocks. |

**Trade-offs accepted, with mitigations:**

- *Parsing complexity* concentrates in one crate — mitigated by the
  per-construct parser registry (§6.1): the umbrella parser only dispatches on
  the leading keyword; each construct owns its own `Parse` impl and expander
  module, mirroring `boyko_macros`' one-module-per-derive layout.
- *Incremental compile cost*: editing anything in a block re-expands the whole
  block. Mitigated by (a) blocks being feature-sized by convention, (b) the
  parser being allocation-light and single-pass, (c) the expansion being small
  because the heavy codegen stays in `boyko_macros` (§8 R1 measures this).
- *rust-analyzer*: RA expands function-like proc-macros and analyzes the
  result. Two rules make this good in practice: **verbatim token passthrough**
  for every user fragment (types, exprs, bodies — never `to_string()`+re-lex),
  and **error-recovery stub emission** (§7.3) so one typo in one construct does
  not erase the whole block from name resolution.
- *Span quality*: a function-like macro receives the full token stream with
  original spans; nothing about the umbrella shape degrades spans. The span
  policy is §7.2.

### Decision A2: two-crate split + one integration-test crate

```text
crates/aether_lang/            # THE language: parser + AST + expander. NOT a proc-macro crate.
  Cargo.toml                   #   deps: proc-macro2, syn (full), quote. Nothing from the engine.
  src/lib.rs                   #   pub fn expand(TokenStream) -> TokenStream  (the single entry)
  src/kw.rs                    #   syn::custom_keyword! table (component, system, machine, …)
  src/ast.rs                   #   the construct AST types (one enum Construct + per-construct structs)
  src/parse.rs                 #   block parser: keyword dispatch -> per-construct Parse impls
  src/ctx.rs                   #   AetherCtx: per-block symbol table + cross-construct resolution
  src/diag.rs                  #   error helpers: expected-one-of, did-you-mean, multi-error combine
  src/expand/mod.rs            #   Expand trait + item assembly + plugin synthesis
  src/expand/component.rs      #   one file per construct — the boyko_macros layout convention
  src/expand/bundle.rs
  src/expand/system.rs
  src/expand/event.rs
  src/expand/machine.rs
  src/expand/material.rs
  src/expand/scene.rs

crates/aether/                 # The user-facing macro. proc-macro = true. ~20 lines.
  src/lib.rs                   #   #[proc_macro] pub fn aether(ts: TokenStream) -> TokenStream
                               #       { aether_lang::expand(ts.into()).into() }

crates/aether_tests/           # Integration crate (a normal test crate, NOT part of aether's deps).
  Cargo.toml                   #   deps: aether, boyko_ecs, boyko_macros, boyko_render, boyko_scene,
                               #         boyko_app (dev), trybuild, macrotest
  tests/ui/                    #   trybuild diagnostic goldens (*.rs + *.stderr pairs)
  tests/expand/                #   macrotest expansion snapshots (*.rs + *.expanded.rs pairs)
  tests/behavior/              #   runtime tests: expanded systems/machines actually run in an App
```

**How this mirrors `boyko_macros` conventions** (deliberate, so the same
maintainers feel at home):

- **Thin entry delegator**: `aether`'s single `#[proc_macro]` is a one-liner
  delegating to a sibling implementation module — exactly like every
  `#[proc_macro_derive]` in `boyko_macros/src/lib.rs` delegating to
  `component::expand` / `bundle::expand` / `ui::expand`.
- **One module per construct**, shared helpers in a `common`-style module
  (`diag.rs`/`ctx.rs`).
- **No engine dependency — paths are emitted as tokens.** `boyko_macros` "has NO
  dependency on `boyko_ecs`: every `boyko_ecs::…` path a derive produces is
  emitted as a TOKEN inside `quote!` and resolved in the downstream consumer
  crate." Aether follows the identical rule: `aether_lang`/`aether` depend on
  **nothing** from the engine; all emitted paths are absolute tokens
  (`::boyko_ecs::…`, `::boyko_render::…`, `::boyko_macros::…`) resolved at the
  user's expansion site. No cycles, and the engine can evolve without lockstep
  Aether releases (the drift *tests* live in `aether_tests`, which does depend
  on the engine — that is exactly where drift should be caught, §8 R4).
- **Grammar documented as EBNF in the doc comment** of the `aether!` entry
  (the `ui!` precedent), plus per-construct EBNF below.
- English-only artifacts, `///` on every public item, `expect("invariant: …")`,
  imports grouped std → external → crate.

### Decision A3: Aether emits the *canonical hand-written surface*, and the derives do the rest

Every construct expands to the annotated-Rust form a disciplined engineer
writes today, and **the existing `boyko_macros` derives expand those**. Aether
never re-implements ID assignment, hook wiring, bundle codegen, or event layout
— single expansion authority, zero drift between "Aether components" and
"hand-written components". The compiler expands macros outside-in, so an
`aether!`-emitted `#[derive(Component)] struct …` is subsequently processed by
`boyko_macros` exactly as if the user had typed it.

This is also the compile-time-cost containment strategy: Aether's own emission
is roughly the size of the source the user would have typed; the heavy
generated code is attributed to the derives, which the crate pays for already.

---

## 2. Shared grammar conventions

- The token stream must be Rust-lexable (guaranteed: proc-macros only receive
  Rust-lexed tokens). All Aether keywords are contextual `syn::custom_keyword!`s
  — they are only keywords in construct-head / clause-head position, so a
  component may still be named `material` if path-qualified in expressions
  (the `ui!` reserved-context-keyword rule).
- `IDENT`, `TYPE`, `EXPR`, `BLOCK`, `PATH` denote verbatim Rust fragments parsed
  with `syn` and passed through with original spans.
- Trailing commas are always permitted.
- Uppercase construct names map to types (`component Health` → `struct Health`);
  the construct decides the case convention and diagnoses violations early
  (`material Gold` → error: "material names are lowercase — they expand to
  builder functions, not types" — with a rename suggestion).

Top level:

```ebnf
aether_block  := header* construct*
header        := 'plugin' IDENT ';'                      (* at most one; enables scheduling *)
construct     := component | tag | bundle | system | event
               | machine | material | scene
```

---

## 3. Construct specs v1

Each spec gives: surface syntax (EBNF), a concrete **before/after** pair using
the real engine APIs (verified against source during this design), and the
diagnostics story.

### 3.1 `component`

```ebnf
component     := 'component' IDENT '{' item* '}'
item          := field | requires | hook | flag
field         := IDENT ':' TYPE ','?
requires      := 'requires' PATH (',' PATH)* ','?        (* Required-Components *)
hook          := ('on_add'|'on_insert'|'on_replace'|'on_remove') '=' PATH ','?
flag          := 'no_bundle' ','?
tag           := 'tag' IDENT ('(' 'bitset' ')')? ';'     (* zero-data marker *)
```

**Before (Aether):**

```text
component Health {
    current: f32,
    max: f32,
    requires Regen,
    on_add = heal_full,
}

tag Player;
tag Stunned(bitset);
```

**After (emitted Rust — then expanded by `boyko_macros` as usual):**

```rust
#[derive(::boyko_macros::Component)]
#[require(Regen)]
#[component(on_add = heal_full)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

#[derive(::boyko_macros::Component)]
pub struct Player;                       // ZST => auto-detected static tag

#[derive(::boyko_macros::Component)]
#[component(storage = "bitset")]
pub struct Stunned;                      // EnableTag backend: O(1) toggle, no migration
```

Everything the derive already supports flows through: ZST auto-tag detection,
the Phase-22 single-component `Bundle` emission (and `no_bundle` opt-out), the
Phase-14a hook keys (mutually exclusive with the runtime builder — the derive
enforces it; Aether just forwards), `#[require(...)]` ctor wiring, and the
`storage = "bitset"` EnableTag backend (Aether's parser pre-checks the
"bitset ⇒ fieldless" rule to give the error *its own* better span, then emits;
the derive's check remains the authority).

*Storage attrs note:* the engine's planned dense (non-fragmenting) components
(DENSE-COMPONENTS-PLAN.md) will surface as another `component` key when the
derive grows one — Aether adds a `dense` flag token then, one line in this
parser, zero changes elsewhere (the §6 extensibility claim, exercised).

**Diagnostics:**

- `current f32` (missing `:`) → error spanned on `f32`: ``expected `:` between
  field name and type``.
- `on_ad = heal_full` → error spanned on `on_ad`: ``unknown component key
  `on_ad`; expected one of `on_add`, `on_insert`, `on_replace`, `on_remove`,
  `no_bundle`, `requires` (did you mean `on_add`?)``.
- `tag Stunned(bitmap);` → error on `bitmap`: ``unknown tag storage `bitmap`;
  the only tag storage modifier is `bitset` ``.
- A trait-bound failure (e.g. a `!Send` field) surfaces from the derive's named
  const-assert, landing on the struct name because Aether emits the struct
  ident with the user's span (`quote_spanned!`, §7.2).

### 3.2 `bundle`

```ebnf
bundle        := 'bundle' IDENT '{' (IDENT ':' TYPE ','?)* '}'
```

**Before / after:**

```text
bundle Projectile {
    pos: Position,
    vel: Velocity,
}
```

```rust
#[derive(::boyko_macros::Bundle)]
pub struct Projectile {
    pub pos: Position,
    pub vel: Velocity,
}
```

Nothing more — the derive owns arity ≤ 16, the named-struct-only rule, the
no-generics rule, and the static-cache codegen. Aether's added value is only
uniformity of surface. Diagnostics: a >16-field bundle is pre-checked by Aether
("bundle arity is capped at 16 (`MAX_BUNDLE_ARITY`) — split it") with the span
on the 17th field, which is friendlier than the downstream error.

### 3.3 `system` (+ the `plugin` header)

```ebnf
system        := 'system' IDENT '(' param (',' param)* ')' clause* BLOCK
param         := IDENT ':' param_ty
param_ty      := 'query' '<' QUERY_DATA (',' filter_list)? '>'
               | 'res' '<' TYPE '>'      | 'mut' 'res' '<' TYPE '>'
               | 'local' '<' TYPE '>'    | 'commands'
               | 'events' '<' TYPE '>'   | 'emit' '<' TYPE '>'
               | TYPE                                        (* escape: any real SystemParam *)
filter_list   := filter (',' filter)*
filter        := 'with' PATH | 'without' PATH | 'added' PATH | 'changed' PATH
               | 'enabled' PATH | 'disabled' PATH
clause        := 'on' ('startup' | 'update' | 'fixed')
               | 'in' PATH               (* in_set *)
               | 'before' PATH | 'after' PATH   (* a set type, or a sibling aether system ident *)
               | 'when' EXPR             (* run_if; EXPR is a real IntoSystem<(), bool, _> *)
```

**Before (Aether):**

```text
plugin Movement;

system read_input(actions: res<ActionState>, mut cmds: commands) on update in InputSet { … }

system apply_velocity(q: query<(&mut Transform, &Velocity), with Player, without Frozen>,
                      time: res<Time>)
    on update
    after read_input
    when in_state(GameFlow::Playing)
{
    for (t, v) in &mut q {
        t.translation += v.linear * time.delta_secs();
    }
}
```

**After (emitted Rust):**

```rust
pub fn read_input(actions: ::boyko_ecs::Res<ActionState>, mut cmds: ::boyko_ecs::Commands) { … }

pub fn apply_velocity(
    mut q: ::boyko_ecs::Query<
        (&mut Transform, &Velocity),
        (::boyko_ecs::With<Player>, ::boyko_ecs::Without<Frozen>),
    >,
    time: ::boyko_ecs::Res<Time>,
) {
    for (t, v) in &mut q {
        t.translation += v.linear * time.delta_secs();
    }
}

pub struct Movement;

impl ::boyko_ecs::Plugin for Movement {
    fn build(&self, app: &mut ::boyko_ecs::App) {
        app.add_systems_cfg(|b| {
            let __aether_k_read_input = b.add_system(read_input).in_set(InputSet).key();
            b.add_system(apply_velocity)
                .after(__aether_k_read_input)
                .run_if(in_state(GameFlow::Playing));
        });
    }
    fn name(&self) -> &'static str { "Movement" }
}
```

Mapping rules (each one is a mechanical, documented table row):

- `query<D>` / `query<D, filters>` → `Query<D, (F₁, …)>`; sugar filters map to
  `With`/`Without`/`Added`/`Changed`/`Enabled`/`Disabled`. `D` is verbatim Rust
  (`&T`, `&mut T`, `Ref<T>`, `Mut<T>`, `Option<&T>`, tuples — whatever the
  engine's `QueryData` accepts, unvalidated by Aether: the engine's trait
  system is the authority and its errors land on the user's `D` tokens).
- **Mutability inference**: a param whose expansion needs `&mut self` access
  (`query` containing `&mut`/`Mut<`, `mut res`, `commands`, `emit`) gets a
  `mut` binding pattern automatically — removing the most common Rust
  boilerplate error without changing semantics.
- `events<E>` → `EventReader<E>`; `emit<E>` → `EventWriter<E>`.
- The escape hatch `name: SomeRealParamType` passes through verbatim, so
  `NonSendRes<GpuDevice>`, `NonSendResMut<Assets<MeshGpu>>`, `Local<T>` and any
  future `SystemParam` work day one without Aether releases.
- The **body is untouched Rust** — verbatim tokens, full rust-analyzer service.
  Aether sugars the signature and the registration, never the code.
- Ordering: `before`/`after` with a **path that names a sibling Aether system**
  resolves through `AetherCtx` to the captured `SystemKey` (`SystemConfig::key()`
  — the engine's documented handle-forwarding API, system_config.rs:55);
  a path that is *not* a sibling system is emitted as `before_set`/`after_set`
  (a `SystemSet` type). `in` always emits `.in_set(...)`.
  Emission order inside the closure is topologically sorted over sibling
  `after`/`before` edges so every needed key exists before use; a cycle among
  siblings is a compile error with both spans named (the engine would also
  catch it at `build()` as `ScheduleBuildError::OrderingCycle`, but Aether can
  say it earlier and point at source).
- Schedules: `on update` → `add_systems_cfg`; `on fixed` →
  `add_systems_cfg_in(CoreSchedule::Fixed, …)`; `on startup` →
  `add_startup_system` (clauses other than `on` are rejected on startup
  systems — the engine runs them once, pre-loop).
- **Scheduling clauses require the `plugin` header.** A block with clauses but
  no `plugin Name;` is a compile error: "scheduling clauses (`on`, `after`,
  `when`, …) need a `plugin <Name>;` declaration in this block to hold the
  generated registration". A clause-free `system` is legal without a plugin —
  it is just a plain fn the user registers by hand.
- `when EXPR` passes `EXPR` verbatim into `.run_if(EXPR)` — `in_state`,
  `on_enter`, `run_once`, or any user condition fn are ordinary Rust names,
  fully RA-visible.

**Diagnostics:**

- `q: query(&mut Transform)` → error on `(`: ``query takes angle brackets:
  `query<&mut Transform>` ``.
- `after read_inpt` (typo, no such sibling or set in scope) → Aether cannot know
  every `SystemSet` type, so an unknown ident that is ALSO not a sibling system
  emits `after_set(read_inpt)` and rustc reports the unresolved name **on that
  ident's span** — plus Aether attaches a note when the Levenshtein distance to
  a sibling system is ≤ 2: ``note: a sibling aether system `read_input` exists —
  system-to-system ordering uses the bare system name``.
- Duplicate clause (`on update on fixed`) → error on the second `on`:
  ``duplicate schedule clause; a system runs on exactly one schedule``.

### 3.4 `event`

```ebnf
event         := 'event' IDENT '{' ev_field* '}'
ev_field      := IDENT ':' 'entity' '(' PATH (',' PATH)* ')' ','?   (* participant *)
               | IDENT ':' TYPE ','?                                (* parameter *)
```

**Before / after:**

```text
event Damage {
    victim: entity(Position, Health),
    amount: f32,
}
```

```rust
#[::boyko_macros::event]
pub struct Damage {
    #[participant(components = "Position, Health")]
    pub victim: ::boyko_ecs::Entity,
    #[parameter]
    pub amount: f32,
}
```

The `#[event]` attribute macro remains the layout authority (the two-field
participants/parameters rewrite). Aether's win: the participant marker becomes
type-shaped (`entity(A, B)`) instead of a stringly attribute, and the
`#[parameter]` noise disappears. Diagnostics: `victim: entity` without a
component list → error on `entity`: ``participant fields name their component
context: `entity(ComponentA, ComponentB)` `` (the empty context is deliberately
not defaulted — the engine's participant contract wants it explicit).

### 3.5 `machine` — reactive state machines (Harel-lite)

The centerpiece reactive construct. Semantics in §5; here the syntax and
expansion.

```ebnf
machine       := 'machine' IDENT '{' 'initial' IDENT ';' state* '}'
state         := 'state' IDENT '{' ('initial' IDENT ';')? handler* state* '}'
handler       := entry | exit | transition
entry         := 'enter' params? BLOCK
exit          := 'exit'  params? BLOCK
transition    := 'on' PATH params? guard? '=>' state_path (BLOCK | ';')
guard         := 'if' EXPR
state_path    := IDENT ('.' IDENT)*          (* Playing.Paused targets a nested state *)
params        := '(' param (',' param)* ')'  (* same param grammar as system *)
```

**Before (Aether):**

```text
plugin Flow;

machine GameFlow {
    initial Boot;

    state Boot {
        on AssetsReady => Playing;
    }

    state Playing {
        initial Running;
        enter (mut cmds: commands) { cmds.spawn(Hud); }
        exit  (mut cmds: commands, huds: query<&HudRoot>) { /* tear down */ }

        state Running {
            on PausePressed => Playing.Paused;
        }
        state Paused {
            on PausePressed => Playing.Running;
        }

        on PlayerDied (score: res<Score>) if score.lives == 0 => GameOver {
            // action block: runs on the accepting frame
        }
    }

    state GameOver {
        on RestartPressed => Boot;
    }
}
```

**After (emitted Rust — flattened at expansion time):**

```rust
/// Aether machine `GameFlow` — leaf states flattened (Harel hierarchy is
/// resolved by the transpiler; the runtime sees a flat enum).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameFlow {
    Boot,
    PlayingRunning,
    PlayingPaused,
    GameOver,
}

// The engine's `States` is a hand-impl marker trait (no derive exists) —
// Aether writes the impl the engineer would write.
impl ::boyko_ecs::States for GameFlow {}

impl GameFlow {
    /// Zero-cost superstate predicate (compile-time group membership).
    #[inline]
    pub const fn in_playing(self) -> bool {
        matches!(self, Self::PlayingRunning | Self::PlayingPaused)
    }
}

/// One generated transition system per (leaf state, event) pair, gated so the
/// dormant cost is the engine's condition bit-test. Example: Playing.* + PlayerDied.
/// Superstate handlers were copied into each leaf that lacks its own handler
/// for the same event (innermost-wins), so this fn exists for BOTH Playing leaves.
fn __aether_gameflow__playing_running__player_died(
    mut __ev: ::boyko_ecs::EventReader<PlayerDied>,
    mut __next: ::boyko_ecs::ResMut<::boyko_ecs::NextState<GameFlow>>,
    score: ::boyko_ecs::Res<Score>,          // guard/action params, merged + deduped
    mut cmds: ::boyko_ecs::Commands,         // exit-action params (LCA-inlined, §5.3)
) {
    for __e in &mut __ev {
        if !(score.lives == 0) { continue; }                  // guard, verbatim expr
        // -- LCA-computed exit actions, innermost-first (leaving Playing) --
        { /* exit Playing body, verbatim */ }
        // -- transition action block, verbatim --
        { }
        // -- LCA-computed enter actions, outermost-first (GameOver has none) --
        *__next = ::boyko_ecs::NextState::Pending(GameFlow::GameOver);
        return;                                               // first accepted event wins
    }
}

impl ::boyko_ecs::Plugin for Flow {
    fn build(&self, app: &mut ::boyko_ecs::App) {
        app.insert_state(GameFlow::Boot);                     // `initial` = the inserted value
        app.add_systems_cfg(|b| {
            b.add_system(__aether_gameflow__playing_running__player_died)
                .run_if(in_state(GameFlow::PlayingRunning));
            // … one registration per generated transition system …
        });
    }
    fn name(&self) -> &'static str { "Flow" }
}
```

Everything above is existing kernel machinery by name: `States` (Phase 17),
`State<S>`/`NextState<S>` (`Unchanged`/`Pending(S)`), `insert_state`,
`apply_state_transition` (runs in `Schedule::run`'s state pass),
`in_state`/`on_enter`/`on_exit` conditions (Phase 16 `common_conditions.rs`),
`EventReader` (Phase 12). **No new runtime types.** No `dyn`, no queues beyond
the engine's double-buffered events, no hierarchy walk at run time — the
hierarchy exists only inside the transpiler.

**Diagnostics:**

- `initial Runing;` → error on `Runing`: ``no state `Runing` in `Playing`;
  states declared here: `Running`, `Paused` (did you mean `Running`?)``.
- A transition targeting a composite state without an `initial` → error on the
  target: ``target `Playing` is a composite state with no `initial` — add
  `initial <leaf>;` or target a leaf (`Playing.Running`)``.
- Two handlers for the same event in one state → error on the second `on` with
  a note pointing at the first.
- Unreachable state (no inbound transition and not initial) → **warning**-class
  diagnostic (emitted as a `#[deprecated]`-free doc note? No — v1 emits a
  compile error only for hard faults; reachability is a `trybuild`-pinned
  *note* via `Diagnostic` when stable, else omitted — see §8 R6).

### 3.6 `material` — PBR materials

Ground truth (verified): the CPU authority is
`boyko_render::Material { gpu: MaterialGpu, textures: MaterialTextures }`;
`Material::new(base_color: [f32;4], metallic, roughness, reflectance,
emissive: [f32;3], flags) -> Self` (non-textured);
`Material::with_textures(MaterialGpu, MaterialTextures)` is "the ONLY
constructor that can produce a TEXTURED material"; handles are minted at run
time via `Assets<Material>::add` and carried on entities as
`MaterialHandle(handle.index() as u16)` (the vb_lab pattern).

Materials are therefore **runtime-minted assets**, so the zero-cost expansion
target is a **builder function**, not a static table:

```ebnf
material      := 'material' IDENT '{' mat_key* '}'
mat_key       := 'base' ':' color ','?
               | 'metallic' ':' EXPR ','?    | 'roughness' ':' EXPR ','?
               | 'reflectance' ':' EXPR ','? | 'emissive' ':' color ','?
               | 'flags' ':' EXPR ','?
               | 'textures' ':' EXPR ','?    (* escape: a MaterialTextures expression *)
color         := '(' EXPR ',' EXPR ',' EXPR (',' EXPR)? ')'
```

**Before / after** (values from the real vb_lab spread):

```text
material gold  { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
material lamp  { base: (0.02, 0.02, 0.02), roughness: 0.6, emissive: (1.6, 0.9, 0.3) }
```

```rust
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

Defaults match the engine's conventions observed in shipped scenes:
`metallic 0.0`, `roughness 0.5`, `reflectance 0.5`, `emissive [0;3]`,
`flags 0`, alpha `1.0` when `base` has three components. A `textures:` key
switches the emission to `Material::with_textures(MaterialGpu::new(…), <expr>)`
so the `MATERIAL_FLAG_TEXTURED` derivation stays in the engine's one authority.

**Composition with `boyko_shaderdsl` — the seam, not a duplication.** The
material construct parameterizes the *frozen 48-byte `MaterialGpu`* consumed by
the one Cook-Torrance BRDF (`pbr_lighting.hlsli`); it deliberately cannot alter
shading math. Custom surface math is `boyko_shaderdsl`'s jurisdiction (Rust
eDSL body fns → HLSL, byte-identity-gated), per the standing "shaders authored
via boyko_shaderdsl" rule. The designed future seam: a `shader` **construct**
(post-v1) whose value is a *reference to a shaderdsl body fn*
(`surface: my_surface_body`) — Aether would wire the reference, shaderdsl would
own the math and its emission gates. Nothing in v1's `material` grammar blocks
that key from being added (§6).

**Diagnostics:** unknown key → expected-one-of list with did-you-mean;
`base: (1.0, 0.72)` → error on the tuple: ``color takes 3 (rgb, alpha=1.0) or 4
(rgba) components``; `material Gold` → the case-convention error from §2.

### 3.7 `scene` — entity trees (the vb_lab compression)

The generalization of the proven `ui!` shape to render objects. A `scene`
expands to a **spawn function with exactly the SystemParam signature the
engine's shipped scenes use**, registered as a startup system when a `plugin`
header is present.

```ebnf
scene         := 'scene' IDENT '{' scene_item* '}'
scene_item    := mesh_let | node
mesh_let      := 'let' IDENT '=' mesh_src ';'
mesh_src      := 'plane' '(' EXPR ')' | 'cube' '(' EXPR ')'
               | 'mesh' '(' EXPR ',' EXPR ')'              (* (&[Vertex], &[u32]) exprs *)
node          := node_head ('at' EXPR)? ('{' node_body? '}')? ';'?
node_head     := 'mesh' IDENT                              (* a mesh_let binding *)
               | 'sun' | 'spot' | 'point' | 'sky' | 'camera'
               | 'sdf' EXPR                                (* an SdfEdit expression *)
               | 'entity'                                  (* bare spawn, ui!-style *)
node_body     := prop (',' prop)*
prop          := 'material' ':' IDENT                      (* a sibling `material` name *)
               | 'casts_shadow'                            (* mesh: ShadowCaster; spot/point: CastsPunctualShadow *)
               | 'children' ':' '[' node (',' node)* ']'
               | EXPR                                      (* any extra component literal, ui!-style *)
```

`at EXPR` accepts a `Transform` expression, with two sugars: a 3-tuple
`(x, y, z)` → `Transform::from_translation(Vec3::new(x, y, z))`, and a full
struct-ish `Transform { … }` passing through verbatim.

**Before (Aether — compressing the shipped `vb_lab.rs` setup):**

```text
plugin VbLab;

material gold { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
material lamp { base: (0.02, 0.02, 0.02), roughness: 0.6, emissive: (1.6, 0.9, 0.3) }

scene lab {
    let floor  = plane(22.0);
    let block  = cube(1.0);

    mesh floor;
    mesh block at Transform { translation: Vec3::new(0.0, 3.0, -4.5),
                              rotation: Quat::IDENTITY,
                              scale: Vec3::new(14.0, 6.0, 0.4) };
    mesh block at (-2.4, 0.5, -2.2) { material: gold, casts_shadow };
    mesh block at (-4.4, 1.4, -1.0) { material: lamp };

    sdf SdfEdit::sphere([3.2, 0.85, 1.8], 0.85, sdf_op::UNION, 0.0);

    sun  { dir: (-0.42, 0.80, 0.42), color: (1.0, 0.97, 0.92), lux: 3.2 }
    sky  { sky: (0.28, 0.36, 0.50), ground: (0.15, 0.14, 0.13) }
}
```

**After (emitted Rust — the exact shipped idioms, per-node):**

```rust
/// Aether scene `lab` — spawn function. Param set is DEMAND-DRIVEN: `meshes`/`dev`
/// appear because `let … = plane/cube/mesh(…)` is used; `materials` because a
/// `material:` prop is used. A scene with neither compresses to (commands) alone.
pub fn lab(
    mut commands: ::boyko_ecs::Commands,
    mut meshes: ::boyko_ecs::NonSendResMut<::boyko_ecs::ecs::core::asset::Assets<MeshGpu>>,
    mut materials: ::boyko_ecs::ResMut<::boyko_ecs::ecs::core::asset::Assets<::boyko_render::Material>>,
    dev: ::boyko_ecs::NonSendRes<GpuDevice>,
) {
    let floor = meshes.plane(dev.get(), 22.0);
    let block = meshes.cube(dev.get(), 1.0);

    // material handles: hoisted ONCE per scene fn, minted through the asset system
    let __aether_mat_gold = materials.add(gold());
    let __aether_mat_lamp = materials.add(lamp());

    commands.spawn(::boyko_render::MeshBundle::new(floor, Transform::IDENTITY));

    commands.spawn(::boyko_render::MeshBundle::new(block, Transform {
        translation: Vec3::new(0.0, 3.0, -4.5),
        rotation: Quat::IDENTITY,
        scale: Vec3::new(14.0, 6.0, 0.4),
    }));

    commands
        .spawn(::boyko_render::MeshBundle::new(
            block,
            Transform::from_translation(Vec3::new(-2.4, 0.5, -2.2)),
        ))
        .insert(ShadowCaster)
        .insert(MaterialHandle(__aether_mat_gold.index() as u16));

    // … lamp cube, sdf spawn (commands.spawn(SdfPrimitive(<expr>))) …

    // sun => DirectionalLightObject with the pose derived from `dir` exactly as
    // vb_lab does (look_at_rh + Quat::from_mat3), light = DirectionalLight::new(...).
    // sky => commands.spawn(SkyLight::new([..], [..]));
}

impl ::boyko_ecs::Plugin for VbLab {
    fn build(&self, app: &mut ::boyko_ecs::App) {
        app.add_startup_system(lab);
    }
    fn name(&self) -> &'static str { "VbLab" }
}
```

The scene construct is the **`AetherCtx` showcase**: `material: gold` resolves
against the sibling `material` construct (hoisting the `materials.add` mint),
`mesh floor` resolves against the `let` bindings, and the parameter list is
computed from what the body actually uses. It is also honest about scope:
anything not covered by a sugar head is an `entity { <component exprs> }` node
(the `ui!` fallback), so no engine feature is walled off.

**Diagnostics:** `material: gol` → error on `gol`: ``no material `gol` in this
aether block (materials here: `gold`, `lamp`)``; `mesh floot` → same shape
against the `let` bindings; a `casts_shadow` on `sky` → ``the `sky` node has no
shadow-caster form``.

---

## 4. Cross-construct architecture: `AetherCtx`

One per `aether!` block, built between parse and expand:

```rust
/// Per-block symbol table. Build-time only — this is transpiler state, never runtime state.
pub struct AetherCtx {
    /// Declared constructs by name, with kind + declaration span (for duplicate/
    /// did-you-mean diagnostics). Keyed collections here are compile-time tooling —
    /// exempt from runtime hot-path rules, same as boyko_shaderdsl's emit arena.
    symbols: Vec<Symbol>,            // (name, kind: Component|System|Machine|Material|Scene|…, span)
    plugin: Option<PluginDecl>,      // the block's plugin header, if any
}
```

Resolution rules v1:

- Scope = the enclosing `aether!` block. Cross-block references are ordinary
  Rust name resolution on the *expanded* items (a system in block A can name a
  set/type from block B because both are real items in the module) — Aether
  itself never resolves across blocks (no global macro state, ever).
- Consumers: `system`'s `after`/`before` (sibling systems → `SystemKey`
  captures), `scene`'s `material:`/`mesh` props, `plugin` synthesis (collects
  all schedulable constructs), `machine` (its own state namespace is
  block-internal).
- Duplicate names across kinds are an error at ctx-build time with both spans.

Pipeline (the single `aether_lang::expand`):

```text
TokenStream ─parse.rs─▶ Vec<Construct> ─ctx.rs─▶ AetherCtx ─expand/*.rs─▶ Vec<syn::Item> ─▶ TokenStream
                 │  (per-construct Parse, keyword-dispatched)      │ (per-construct Expand, ctx-aware)
                 └────────── errors accumulate; recovery stubs per §7.3 ──────────┘
```

---

## 5. Reactivity model — precise semantics (state machines × kernel features)

All semantics are defined **in terms of named, existing kernel features** — no
new runtime concepts.

### 5.1 What re-runs when

- Each `(leaf state, event)` transition compiles to ONE system, registered
  `.run_if(in_state(Leaf))`. Dormant cost is the engine's condition machinery
  (the `has_condition` bitset evaluated at the apply-window barrier — Phase 16),
  i.e. effectively zero when the machine is idle in another state.
- The system drains its `EventReader<E>` (double-buffered dispatcher, Phase 12).
  **First accepted event wins** per frame per system: after the first event that
  passes the guard, the system writes `NextState::Pending(target)` and returns.
  Remaining events of that type this frame are intentionally left unconsumed by
  *this* machine's cursor semantics? No — the cursor advances for all read
  events (the reader iterates); they are *observed and discarded*. This is the
  documented v1 policy: one transition per machine per frame.
- If several transition systems fire on the same frame (different events),
  **last write to `NextState<S>` wins** — the engine's `NextState` is a plain
  resource. Aether does not add arbitration in v1; the docs state it, and the
  deterministic mitigation is ordering the generated systems in declaration
  order within the plugin's `add_systems_cfg` closure (stable, documented).
- The actual state flip happens in the engine's transition pass
  (`apply_state_transition::<S>`, once per `Schedule::run`) — transitions
  requested this frame are visible via `State<S>` next frame. Guards and
  actions therefore **see the pre-transition world** (§5.3).

### 5.2 How guards read state

Guard params use the same param grammar as systems and become real
`SystemParam`s on the generated system (`res<Score>` → `Res<Score>`, a
`query<…>` guard param is legal). The guard expression is verbatim Rust over
those bindings. Change-detection-aware guards come for free: a guard param
`q: query<&Health, changed Health>` uses the kernel's `Changed<T>` filter with
the Phase-16.1 "since-last-actual-run" window semantics — no Aether-side tick
bookkeeping.

### 5.3 Entry/exit actions — LCA inlining (the zero-cost Harel mapping)

At expansion time, for every transition `S_leaf --E--> T_leaf`, the transpiler
computes the lowest common ancestor of source and target in the state tree and
emits, **inlined into the transition system body, in this order**:

1. exit actions from `S_leaf` up to (excluding) the LCA — innermost first;
2. the transition's action block;
3. enter actions from (excluding) the LCA down to `T_leaf` — outermost first;
4. the `NextState::Pending(T_leaf)` write.

Consequences, stated plainly:

- Actions run **on the frame the transition is accepted**, inside the
  generated system, under the pre-transition `State<S>` (the engine flips state
  at the next transition pass). This is the deterministic, allocation-free
  mapping; it trades the "actions run exactly when the state flips" purism for
  zero new kernel machinery. The alternative — engine-side enqueued action
  callbacks — is a `dyn`/queue design and is rejected.
- Action params are merged into the transition system's param list (deduped by
  type+name; a conflict like two different bindings of `mut res<X>` in exit and
  action blocks is a compile error naming both spans).
- The machine's **initial enter chain** (enter actions of the initial leaf's
  ancestor path) is emitted as one startup system, after `insert_state` seeds
  the value.
- Users who want leaf enter/exit behavior *outside* the machine (e.g. another
  block's system) use the kernel's own `on_enter(S::Leaf)` / `on_exit(S::Leaf)`
  / `on_transition(a, b)` conditions on ordinary systems — the generated flat
  enum is a first-class `States` type, so the whole existing condition surface
  applies to it. Aether adds nothing and hides nothing.

### 5.4 Hierarchy = compile-time flattening

- Leaves become the flat enum's variants (`Playing.Running` → `PlayingRunning`).
- A superstate handler is **copied into every descendant leaf** that does not
  declare its own handler for the same event (innermost-wins, classical Harel
  conflict resolution) — done by the transpiler, so "bubbling" costs nothing at
  run time.
- A transition targeting a composite state retargets to its `initial` leaf
  (recursively); a composite without `initial` is a compile error (§3.5).
- Superstate membership tests are emitted `const fn in_<group>()` predicates
  (a `matches!` over the leaf set) — usable in guards and in any user code.

### 5.5 Relation to hooks/observers + the v2 per-entity path

App-scoped machines (v1) ride on States + events and never touch
hooks/observers. The designed v2 extension — **entity-scoped machines**
(`machine … on entity` grammar reserved): the state becomes a generated
component (a plain enum field), transitions become systems over
`query<(&mut MachineState, …)>` driven by the same event grammar, and
enter/exit map to the kernel's component **hooks** (`on_add`/`on_remove` keys,
Phase 14a) when the state is modeled presence-style, or to `Changed<T>`-gated
systems when value-style. Nothing in the v1 grammar or expander needs to change
— `on entity` selects a different expander for the same AST (the §6 test).

---

## 6. Extensibility architecture

### 6.1 The construct registry (keyword-keyed)

`parse.rs` owns a single dispatch table from the leading keyword to a parse
function:

```rust
/// One row per construct. A new construct = one new row + one new module pair
/// (parse impl in ast.rs/parse.rs, expander in expand/<name>.rs). Existing
/// constructs are untouched — their parsers never see the new keyword.
const CONSTRUCTS: &[(&str, fn(ParseStream) -> syn::Result<Construct>)] = &[
    ("component", component::parse),
    ("tag",       component::parse_tag),
    ("bundle",    bundle::parse),
    ("system",    system::parse),
    ("event",     event::parse),
    ("machine",   machine::parse),
    ("material",  material::parse),
    ("scene",     scene::parse),
];
```

An unknown leading ident is THE canonical extensibility diagnostic:
``unknown construct `foo`; this aether supports: component, tag, bundle,
system, event, machine, material, scene`` (+ did-you-mean). The expander side
mirrors this with an `Expand` impl per construct; `expand/mod.rs` iterates
constructs, never matching on kinds itself.

Adding construct N+1 therefore touches: `kw.rs` (one keyword), the table (one
row), one new `ast.rs` node + `parse` fn, one new `expand/` module, tests. The
§3.1 `dense` flag, the §3.6 `shader` key, and the §5.5 `on entity` machines
were each checked against this claim during design.

### 6.2 `AetherCtx` as the stable inter-construct contract

Constructs communicate ONLY through `AetherCtx` (declared symbols + kinds +
spans + per-kind payloads such as a material's builder-fn name). A new
construct that wants to be referenceable registers a symbol kind; consumers
opt in by name. No construct imports another construct's AST.

### 6.3 Versioned syntax gating

The block accepts an optional first header `aether v1;`. Absent = the crate's
current default. When v2 syntax ever breaks v1, blocks pin their version and
the parser dispatches per-version at the construct table level (a second table,
not per-construct branching). This is cheap insurance; v1 ships with the header
parsed and only `v1` accepted.

### 6.4 Third-party construct plugins: explicitly NOT in v1

Proc-macros cannot soundly load external parser plugins (no dynamic loading in
the macro sandbox; a Cargo-feature-mesh of optional constructs fragments the
grammar and the diagnostics). Third parties extend Aether the same way they
extend the engine: PR a construct into `aether_lang`. Revisit only if a real
external demand appears.

---

## 7. Tooling & DX plan

### 7.1 The error-message quality bar

- Every Aether-originated error is a `syn::Error` with the **narrowest
  applicable span** (the offending token, not the construct, not the block).
- Errors **accumulate**: one bad field does not hide the next bad field;
  `Error::combine` collects per-construct, and independent constructs always
  all report (see 7.3).
- "Expected one of …" lists are exhaustive and sorted; "did you mean" fires at
  Levenshtein ≤ 2 against the legal keyword set / sibling symbol set.
- Errors that the *downstream* layer would catch anyway (derive const-asserts,
  trait bounds, `ScheduleBuildError`) are pre-checked by Aether ONLY when
  Aether can produce a strictly better span/message (bundle arity, bitset
  fieldlessness, ordering cycles among siblings); otherwise Aether defers —
  duplicated checks drift.
- **Every diagnostic in this plan is a `trybuild` golden** (`tests/ui/*.rs` +
  pinned `*.stderr`). A wording improvement is a deliberate re-bless, never an
  accident (the Ferrous Systems regression argument).

### 7.2 Span policy (the good-errors mechanism)

1. User fragments (idents, types, exprs, blocks) are carried as parsed `syn`
   nodes / token trees and re-emitted **verbatim** — never stringified,
   never re-lexed. This preserves spans end-to-end, which is what makes both
   diagnostics and rust-analyzer navigation land on DSL source lines.
2. Synthesized tokens use `Span::call_site()`.
3. Generated items that exist "because of" a user name are emitted under
   `quote_spanned!(name.span() => …)` so downstream errors (trait bounds,
   duplicate impls) point at the user's declaration — the same trick
   `boyko_macros` uses to make its named const-asserts readable.
4. Generated internal names are `__aether_`-prefixed and never collide with
   user names (double-underscore + construct + owner name).

### 7.3 Error recovery (rust-analyzer resilience)

On a parse error inside construct K, the expander still emits: (a) K's
`compile_error!` at the precise span, (b) a **best-effort stub** for K when its
name was parsed (e.g. the struct with the fields parsed so far, or a unit
struct) so downstream references to K's name keep resolving, and (c) the FULL
expansion of every other construct in the block. One typo therefore costs one
error, not a module-wide sea of "unresolved name" — the concentrated failure
mode users report in `view!`-style macros.

### 7.4 Workflows

- **`cargo expand`** is the debugging front door (documented in the crate
  README section of `aether`'s rustdoc): `cargo expand -p aether_tests
  --test <case>` shows the exact emitted Rust; expansion snapshots in
  `tests/expand/` are the same content pinned (`macrotest`), so "what does this
  expand to" has a versioned answer.
- **rust-analyzer expectations, stated honestly**: completions/hover/goto work
  inside `EXPR`/`TYPE`/`BLOCK` positions (verbatim tokens) once the block
  parses; they do NOT work *mid-keyword* (typing a new clause head) — that is
  inherent to macro DSLs; the §7.3 recovery keeps the rest of the file healthy
  while typing. Formatting inside bodies is preserved as written; an
  `aetherfmt` is a non-goal v1 (the `leptosfmt` lesson: plan for it, don't
  block on it).
- **Syntax highlighting**: TextMate grammar + tree-sitter are explicitly
  post-v1; v1 reads acceptably because the grammar is Rust-lexable and bodies
  ARE Rust.

### 7.5 The test pyramid (per rung, all in `aether_tests`)

1. `aether_lang` unit tests — parser accepts/rejects, AST shapes, ctx
   resolution (no compiler session needed — the two-crate split's payoff).
2. `trybuild` diagnostics goldens — every error message in this document.
3. `macrotest` expansion snapshots — pinned `*.expanded.rs` **compiled against
   the real engine crates** (the anti-drift gate: if `Material::new` gains a
   parameter, the snapshot build breaks in CI, not in a user's game).
4. Behavior tests — expanded output runs: an `App` with an Aether plugin
   executes systems in the declared order; a machine transitions under events
   with guards/entry/exit observed via probe resources.

---

## 8. Risks + mitigations

| # | Risk | Mitigation |
|---|---|---|
| R1 | **Compile-time blowup** — big blocks, syn `full` parsing, expansion volume invisible (nnethercote) | Two-crate split (parser compiles once); emission ≈ hand-written size because heavy codegen stays in `boyko_macros`; a CI measurement of expanded-LOC per snapshot (macrotest output is exactly that corpus); block-per-feature convention documented. `watt`-style precompiled macros: **considered, rejected v1** (toolchain friction on windows-gnu; measure first, optimize second). |
| R2 | **Span degradation** — the classic failure is stringify/re-parse round-trips | Hard rule §7.2(1) (verbatim tokens only), enforced by a clippy-style internal review checklist + trybuild goldens whose *span columns* are pinned in the `.stderr` files. |
| R3 | **rust-analyzer breakage** — a panicking or erroring macro erases the block from analysis | §7.3 recovery stubs (never panic: top-level `catch`-style guard converts internal invariant failures into a spanned error + stubs); behavior verified by a dedicated trybuild case ("one broken construct, sibling still resolvable"). |
| R4 | **DSL drifts from engine API** — the engine evolves, Aether emits yesterday's calls | The decisive gate: expansion snapshots **compile against real engine types in CI** (`aether_tests` deps). Any breaking engine change fails the Aether build the same day, in-repo. Plus the tokens-not-deps rule keeps the fix a one-file expander edit. |
| R5 | **Grammar ambiguity as constructs accumulate** | Constructs are keyword-led (LL(1) at the block level); clauses are keyword-led inside constructs; the versioned-syntax header (§6.3) is the escape valve for any future breaking regrammar. |
| R6 | **Warning-class diagnostics** (unreachable state, unused material) have no stable emission path from proc-macros | v1 emits hard errors only for hard faults; soft findings are deferred until `proc_macro::Diagnostic` stabilizes or are surfaced by a later `aether-lint` dev-tool built on `aether_lang` (the two-crate split makes that a free-standing binary, no macro needed). |
| R7 | **Last-write-wins on same-frame multi-machine/multi-event transitions** surprises users | Documented semantics (§5.1), deterministic declaration-order registration, and a behavior test pinning it. If real projects need arbitration, a `priority N` clause slots into the transition grammar without breaking v1. |
| R8 | **Scene sugar ossifies** — heads like `sun`/`spot` hardcode today's bundles | Every sugar head lowers to the `entity { <component exprs> }` general form; sugar is additive convenience over a universal fallback, so engine additions are usable before Aether learns their sugar. |

---

## 9. Implementation roadmap — rungs A0..A7

Each rung is independently shippable, gated by the §7.5 pyramid at its scope.
Sizes: S ≈ a focused session, M ≈ a few sessions, L ≈ a small campaign.

| Rung | Contents | Test gate | Size |
|---|---|---|---|
| **A0** | Crates skeleton (`aether_lang`, `aether`, `aether_tests`); block parser + keyword registry + `diag.rs` + recovery stubs; **`component` + `tag` end-to-end** (fields, requires, hooks, no_bundle, bitset) | trybuild goldens for every §3.1 diagnostic; macrotest snapshots compiling against `boyko_ecs`; unit tests; clippy `-D warnings` | **M** |
| **A1** | `bundle` + `event` | goldens (arity cap, participant syntax) + snapshots + a behavior test sending/reading an Aether event through an `App` | **S** |
| **A2** | `system` + `plugin` header: param sugar table, mutability inference, clauses, sibling-`SystemKey` ordering (topo-sorted emission), startup/update/fixed | behavior tests: ordering respected, `when` gating works, fixed-schedule system sees `FixedTime`; goldens for clause errors | **M** |
| **A3** | `machine` — FLAT machines only: enum + `States` impl + transitions + guards + leaf enter/exit + initial-enter startup system | behavior test: full transition graph exercised via events, guard blocks a transition, enter/exit probes fire in order; goldens (§3.5 list) | **M** |
| **A4** | `machine` hierarchy: composite states, `initial` retargeting, handler copy-down (innermost-wins), LCA entry/exit inlining, `in_group` predicates | behavior test: the §3.5 GameFlow chart verbatim; snapshot pins the flattened enum + one copied-down handler | **M** |
| **A5** | `material` (incl. `textures:` escape) | snapshots pinned against `boyko_render::Material::new`/`with_textures`; goldens (color arity, case rule) | **S** |
| **A6** | `scene` + full `AetherCtx` resolution (materials, mesh lets, demand-driven params) + `plugin` startup registration | the vb_lab-compression case as a behavior test under `BOYKO_HOST_DUMP`-style headless run where feasible, else spawn-count/component assertions on the world; goldens (unknown material/mesh) | **L** |
| **A7** | DX hardening: expansion-size CI measurement (R1), span-column-pinned goldens sweep (R2), RA resilience case (R3), `cargo expand` docs, `aether v1;` header | the full pyramid green across all constructs; a written DX checklist in the crate docs | **S** |

Dependencies: A2 needs A0 (ctx, plugin header); A3 needs A2 (param grammar,
plugin); A4 needs A3; A6 needs A5 + A2. A1/A5 can proceed in parallel with A2.

---

## 10. Decision index

| # | Decision | One-line rationale |
|---|---|---|
| A1 | One umbrella `aether!` function-like item macro | Cross-construct references need one parse context; attribute macros can't host non-Rust item syntax |
| A2 | `aether_lang` (lib) + `aether` (proc-macro) + `aether_tests` (integration) | Dioxus-proven tooling split; parser unit-testable; drift gate lives with the engine deps |
| A3 | Emit the canonical hand-written surface; `boyko_macros` derives do the codegen | Single expansion authority, zero drift, minimal expansion volume |
| — | Tokens-not-deps for all engine paths | The `boyko_macros` no-cycle rule, applied unchanged |
| — | Bodies/exprs/types are verbatim span-preserved Rust | BSN-proven RA friendliness; leptos-lesson avoidance |
| — | State machines: flat enum + compile-time Harel flattening + LCA-inlined actions | statig's semantics, zero-cost'd: no runtime hierarchy, no dyn, no queues beyond kernel events |
| — | Machines ride on `States`/`NextState`/conditions/`EventReader` — no new runtime types | Reactivity through named kernel features only |
| — | Materials expand to `#[inline]` builder fns over `Material::new`/`with_textures` | Materials are runtime-minted assets; a static table would be a parallel data system |
| — | Scenes generalize `ui!`: sugar heads over a universal `entity { … }` fallback | Proven in-repo shape; sugar can't wall off engine features |
| — | Closed construct registry, keyword-keyed; versioned header; no 3rd-party plugins v1 | LL(1) grammar stability + honest proc-macro constraints |

## Sources (prior-art research)

- Dioxus `rsx!` DSL-crate separation — https://lib.rs/crates/dioxus-rsx
- Bevy next-gen scenes / `bsn!` (rust-analyzer-friendly scene macro) — https://github.com/bevyengine/bevy/pull/23413 and https://github.com/bevyengine/bevy/discussions/14437
- Leptos DX chapter (view-macro IDE/formatting reality) — https://book.leptos.dev/getting_started/leptos_dx.html
- `statig` hierarchical state machines (superstates, entry/exit, no-heap; typestate critique) — https://github.com/mdeloof/statig
- Ferrous Systems, "Structuring, testing and debugging procedural macro crates" — https://ferrous-systems.com/blog/testing-proc-macros/
- N. Nethercote, "How much code does that proc macro generate?" — https://nnethercote.github.io/2025/06/26/how-much-code-does-that-proc-macro-generate.html
- `syn` (Parse trait, custom keywords, spanned errors) — https://docs.rs/syn/latest/syn/
