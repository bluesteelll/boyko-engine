# Diagnostics

A DSL is only as good as its errors. Aether's rule is narrow and absolute:
**every error carries the offending token's own span**, and the message names
what was expected. There is no fallback to "error in macro invocation", and no
diagnostic is allowed to land on the `aether!` call site when a real token
exists.

Two mechanisms do most of the work:

- **Exhaustive expected-one-of lists.** When a keyword is wrong, the message
  enumerates the legal set.
- **Did-you-mean at edit distance ≤ 2.** Against construct keywords, clause
  keywords, filter keywords, sibling system names, and sibling state names.

## Unknown construct

The canonical extensibility diagnostic. It names the whole v1 surface, not just
what happens to be implemented, so the list reads the same on every rung:

```text
error: unknown construct `compnent`; this aether supports: component, tag, bundle, system, event, plugin, machine, material, scene (did you mean `component`?)
 --> tests/ui/unknown_construct.rs:5:5
  |
5 |     compnent Health { hp: f32 }
  |     ^^^^^^^^
```

`plugin` is in that list because the block parser **dispatches** on it, and the
list must name every keyword the parser dispatches on or it misstates the
surface. It was missing until rung A4, which cost `pluging P;` both its
did-you-mean and an honest list; a unit test now ties the two together.

## Planned constructs name their rung

A misspelling and a not-yet-shipped construct are different failures, and they
get different messages. Writing a construct from a later rung tells you the
rung:

```text
error: `scene` is an Aether construct but lands at rung A6; this build carries rungs A0..A5 (component, tag, bundle, system, event, plugin, machine, material)
 --> tests/ui/planned_construct_names_its_rung.rs:9:5
  |
9 |     scene lab { }
  |     ^^^^^
```

`scene` is the only construct left with that answer. The message is also how a
rung's landing becomes visible from the outside: this golden's subject was
`material` until A5 shipped it, and the rung range moved with it. You never need
to consult a roadmap to find out whether something exists — try it, and read
which of the two errors you get.

## Case gates

Names that expand to **types** (`component`, `tag`, `bundle`, `event`,
`machine`, `state`, `plugin`) must be UpperCamelCase; names that expand to
**fns** (`system`, `material`) must not. Both directions are checked in the
block, with a concrete rename:

```text
error: component names are UpperCamelCase — they expand to types (rename `health` to `Health`)
error: system names are snake_case — they expand to fns (rename `Foo`)
error: material names are lowercase — they expand to builder functions, not types (rename `Gold` to `gold`)
```

The check is Unicode-correct: it asks `char::is_uppercase`, not an ASCII probe,
so `component Здоровье { … }` is accepted as titled in its own script. And the
rename is only attached when it actually differs from what you wrote — a
self-identical suggestion explains nothing.

It also **reads through a raw-ident escape**. `r#Foo` prints as `r#Foo`, whose
first character is the escape's `r`, so a naive gate refused it for being
lowercase and suggested `R#Foo` — which is not a legal identifier at all. The
gate classifies the escaped spelling and quotes the original: `component
r#Foo { … }` passes, and `component r#health { … }` says ``rename `r#health` to
`Health` ``.

## Data constructs

| You write | Aether says |
|-----------|-------------|
| `current f32` (missing colon) | ``expected `:` after field `current` (or a known item: requires / on_add / on_insert / on_replace / on_remove / no_bundle)`` |
| `on_add = f, on_add = g` | ``duplicate hook `on_add` `` |
| `no_bundle, no_bundle` | ``duplicate `no_bundle` `` |
| `tag Stunned(dense);` | ``unknown tag modifier `dense`; the only one is `bitset` (the EnableTag backend)`` |
| `tag Player` (no semicolon) | ``a tag declaration ends with `;` (tags have no body — a component with fields wants `component`)`` |
| a 17-field bundle | ``bundle arity is capped at 16 (`MAX_BUNDLE_ARITY`) — split it`` — spanned on the 17th field's name |
| `victim: entity,` | ``participant fields name their component context: `entity(ComponentA, ComponentB)` `` |
| `hit: entity(foo::Bar)` | ``participant context components are bare component idents … — found `foo::Bar`; import the component and name it unqualified`` |

The unknown-key case is worth a second look: because a component item that is
not a known keyword is parsed as a *field*, mistyping `on_ad = heal_full` gives
you the missing-colon error — whose message lists every legal item. One error,
and it tells you the whole item vocabulary.

## Systems and clauses

| You write | Aether says |
|-----------|-------------|
| `q: query(&mut T)` | ``query takes angle brackets: `query<&mut Transform>` `` |
| `q: query<&T, wih P>` | ``unknown query filter `wih`; filters are: with, without, added, changed, enabled, disabled (did you mean `with`?)`` |
| `system s() afterr X {}` | ``unknown clause `afterr`; clauses are: on, in, before, after, when (did you mean `after`?)`` |
| `on update on fixed` | ``duplicate schedule clause; a system runs on exactly one schedule`` |
| `on tick` | ``unknown schedule `tick`; `on` takes one of: startup, update, fixed`` |
| `p: mut query<…>` | ``in the type position `mut` pairs only with `res`: `mut res<T>` `` |
| `when let Some(x) = f()` | ``` `let` bindings are not usable as a run condition — `when` takes a plain bool expression (bind with a `local<…>` param or match inside the body instead) ``` |
| any clause, no `plugin` header | ``scheduling clauses (`on`, `after`, `when`, …) need a `plugin <Name>;` declaration in this block to hold the generated registration`` |
| `on startup in SomeSet` | ``scheduling clauses other than `on` are rejected on startup systems — the engine runs them once, pre-loop`` |
| `after a` where `a` is on another schedule | ``sibling system `a` runs on a different schedule — cross-schedule ordering is not expressible`` |
| `after a` where `a` is a startup system | ``ordering references `a`, a startup system — startup systems run once, pre-loop, and cannot be ordered against`` |
| mutually `after` siblings | ``system ordering cycle among `a`, `c` — break one `before`/`after` edge`` (every member's span reported) |
| `after read_inpt` next to a sibling `read_input` | ``…is not a sibling aether system; a sibling `read_input` exists — system-to-system ordering uses the bare system name (a real SystemSet type this close in name must be referenced by a qualified path)`` |
| two `plugin` headers | ``one `plugin` per aether block — `A` already holds this block's registrations`` (with a second span on the first header) |

The plugin-header error is a good example of the span policy — it lands on the
**system's name**, the thing that needs the plugin, not on the block:

```text
error: scheduling clauses (`on`, `after`, `when`, …) need a `plugin <Name>;` declaration in this block to hold the generated registration
 --> tests/ui/clauses_need_a_plugin.rs:5:12
  |
5 |     system tick() on update {}
  |            ^^^^
```

### One recorded deviation

The near-miss ordering case (`after read_inpt`) was designed to pass through and
have Aether attach a *note* to rustc's unresolved-name error. Stable
proc-macros cannot attach notes to downstream errors, so the close call became
an **Aether error** carrying the note's text. The cost is stated in the message
itself: a genuine `SystemSet` type whose name is that close to a sibling system
must be referenced by a qualified path.

## Machines

| You write | Aether says |
|-----------|-------------|
| `initial Runing;` inside `Playing` | ``no state `Runing` in `Playing`; states declared here: `Running`, `Paused` (did you mean `Running`?)`` |
| `=> Playing` where `Playing` is composite with no `initial` | ``target `Playing` is a composite state with no `initial` — add `initial <leaf>;` or target a leaf (`Playing.Running`)`` |
| two `on E` in one state | ``duplicate handler for `E` in state `A` `` + a second span: *the first handler is here* |
| `machine` with no `plugin` header | ``a `machine` needs a `plugin <Name>;` declaration in this block to hold its `insert_state` and transition registrations`` |
| the same param name with different types across merged handlers | ``param `cmds` is declared with conflicting types across this transition's merged enter/exit/action handlers`` |
| the same, across the initial leaf's ancestor `enter` bodies | ``param `x` is declared with conflicting types across the initial state's merged `enter` chain`` |
| `on E if let Some(_) = q => A;` | ``` `let` bindings are not usable as a transition guard — `if` takes a plain bool expression (bind with a `local<…>` param or match inside the body instead) ``` |
| an unknown item inside a state | ``unknown state item `foo`; state items are: initial, enter, exit, on, state`` |
| a non-state item in the machine body | ``expected `state`, found `foo` (a machine body holds only states after `initial`)`` |

```text
error: no state `Runing` in `Playing`; states declared here: `Running`, `Paused` (did you mean `Running`?)
  --> tests/ui/machine_unknown_initial_did_you_mean.rs:10:21
   |
10 |             initial Runing;
   |                     ^^^^^^
```

### Flattening collisions

Flattening concatenates the state path, and the generated fn and predicate names
are its snake_case collapse. Both steps are lossy, so two legal chart positions
can mint one name. rustc would report these as "defined multiple times" against
generated tokens; Aether reports both chart positions instead — every one of
these carries a second span, *the first … is here*:

| You write | Aether says |
|-----------|-------------|
| `state A { state BC {} }` next to `state AB { state C {} }` | ``states `A.BC` and `AB.C` both flatten to `ABC` — flattening concatenates the state path, so they would emit one name; rename one`` |
| two sibling `state Idle {}` | ``duplicate state `Idle` — sibling states need distinct names`` |
| leaves `AB` and `A_b`, each with `on E` | ``states `AB` and `A_b` both generate the system `__aether_m__a_b__e` — generated names are the snake_case collapse of the flattened state path, and `AB` and `A_b` collapse alike; rename one`` |
| composites `AB` and `A_b` | ``composite states `AB` and `A_b` flatten to `AB` and `A_b`, which both collapse to the predicate `in_a_b` — rename one`` |
| `on a::E` and `on b::E` in one state | ``events `a::E` and `b::E` both generate the system `__aether_m__a__e` for leaf `A` — the generated name keys on the event's last path segment; import one under an alias (`use … as …`)`` |

```text
error: states `A.BC` and `AB.C` both flatten to `ABC` — flattening concatenates the state path, so they would emit one name; rename one
  --> tests/ui/machine_flattened_name_collision.rs:17:19
   |
17 |             state C {}
   |                   ^

error: the first state flattening to this name is here
  --> tests/ui/machine_flattened_name_collision.rs:13:19
   |
13 |             state BC {}
   |                   ^^
```

### Reachability does not decide what is checked

Retargeting and handler inheritance are lazy walks. A name no leaf happens to
reach was never resolved — so until rung A4 a typo in one expanded clean. Every
declared name is now resolved eagerly:

| You write | Aether says |
|-----------|-------------|
| `initial Running;` inside a **childless** state `Idle` | ``` `Idle` has no nested states, so `initial` has nothing to name — drop it, or nest `state Running { … }` inside `Idle` ``` |
| a typo'd `initial` inside a composite nothing targets | ``no state `Runing` in `Lonely`; states declared here: `Running` (did you mean `Running`?)`` |
| `on E => Nowhere;` on a state whose inner state shadows `on E` | ``no state `Nowhere` in `M`; states declared here: `P0`, `Top` `` |

```text
error: `Idle` has no nested states, so `initial` has nothing to name — drop it, or nest `state Running { … }` inside `Idle`
  --> tests/ui/machine_initial_on_a_leaf.rs:11:21
   |
11 |             initial Running;
   |                     ^^^^^^^
```

The shadowed-target case is the sharpest of the three: `P0`'s `on E` is shadowed
for every leaf by `A`'s own `on E`, so no inheritance walk ever reaches it — and
the target it names would never have been looked up. A chart that names a state
which does not exist is broken whether or not anything reaches it.

## Materials

The seven material keys are one table in the parser — the same rows the
diagnostic prints and the parser dispatches on — so the "expected one of" list
cannot advertise a key the parser lacks, or omit one it accepts:

```text
error: unknown material key `roughnes`; keys are: base, metallic, roughness, reflectance, emissive, flags, textures (did you mean `roughness`?)
 --> tests/ui/material_unknown_key.rs:7:46
  |
7 |     material gold { base: (1.0, 0.72, 0.30), roughnes: 0.14 }
  |                                              ^^^^^^^^
```

Where each fault lands is the whole point:

| You write | Where the error lands |
|-----------|-----------------------|
| `roughnes: 0.14` | the **key**, with the exhaustive list and a did-you-mean |
| `base: (1.0, 0.72)` | the **tuple** — neither the key nor any one component is what is wrong |
| `emissive: (r, g, b, a)` | the tuple again; `Material::new` takes `[f32; 3]`, and a synthesized array carries no span of yours |
| `base: …, base: …` | the **second** key, refusing last-write-wins |
| no `base:` | the material's **name**, with the default table for every key that does have one |
| two materials of one name | **both** names — the one collision rustc could not place on a user token |

The full message texts, and the story behind that last row, are on
[Materials](materials.md#refusals).

## What Aether checks, and what it leaves alone

Duplicated checks drift, so Aether pre-checks a downstream rule **only** when it
can produce a strictly better span or message. The whole pre-check list:

| Pre-check | Why Aether owns it |
|-----------|--------------------|
| bundle arity ≤ 16 | the span lands on the 17th field, not on the struct |
| participant components are bare idents | the alternative is a downstream proc-macro panic with no span at all |
| tag modifier is `bitset` | the surface is Aether's, so the vocabulary error is Aether's |
| duplicate hook keys | Aether has both spans |
| sibling ordering cycles, cross-schedule and startup ordering | said at expansion, before `ScheduleBuildError::OrderingCycle` at `build()` |
| machine state resolution | the state namespace exists only inside the transpiler |
| flattened-name and snake-collapse collisions | Aether has both chart positions; rustc sees only a duplicate definition on tokens the user never wrote |
| every declared machine name, reachable or not | the lazy retargeting and inheritance walks skip what nothing targets, so a typo used to expand clean |
| `let` in a guard or a run condition | Aether splices that expression into `if !(…)` or `.run_if(…)`, where a `let` is not valid Rust; caught here, the error lands on your own `let` instead of on a synthesized `if` you never wrote |
| material keys, color arities, and the required `base:` | the key surface is Aether's; and a wrong arity would otherwise fail against an array Aether synthesized, which carries no span of yours |
| two materials of one name | measured: rustc's `E0428` puts **both** labels on the `aether!` token, because a material emits no derive and no trait bound to carry a second, localized error |

Everything else defers. Query data is handed to the engine's `QueryData` trait
unvalidated; trait-bound failures come from the derive's own const-asserts;
unresolved component or resource names are ordinary rustc errors. They still
land on your tokens, because every fragment is re-emitted verbatim with its
original span — that is the same mechanism that keeps rust-analyzer working
inside a block.

## How the contract is held

Every message above is pinned twice:

- **Unit tests** in `crates/aether_lang/src/expand.rs` assert the message text —
  the tighter pin, and it needs no compiler session.
- **`trybuild` goldens** in `crates/aether_tests/tests/ui/` assert that the error
  surfaces *through rustc*, in a real downstream crate, at the user's own tokens.
  A message that is right in a unit test but anchored at the call site passes the
  first and fails the second.

A `.stderr` golden is re-blessed only after verifying the error *kind* is
unchanged. Wording improvements are deliberate; a silently weakened diagnostic
is a regression.

## See also

- [Aether overview](overview.md) — the constructs these errors talk about.
- [Data constructs](data-constructs.md) and
  [Systems & plugins](systems-and-plugins.md) — the rules behind the messages.
- [State machines](state-machines.md) — the chart semantics the machine errors
  protect.
- [Materials](materials.md) — the key table these messages advertise.
- Source: `crates/aether_lang/src/diag.rs`, `crates/aether_lang/src/parse.rs`,
  goldens in `crates/aether_tests/tests/ui/`.
