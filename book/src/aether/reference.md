# Aether v1 Reference

This page is the **lookup**: every construct's grammar line, its keys with types and defaults, what
it emits, and its refusals — plus the block-wide rules, the diagnostic catalogue, and the gates that
hold all of it in place. The [narrative pages](#see-also) are the explanation; this one is what you
open when you already know what you want and need the exact shape.

## What Aether is

**Aether** is one function-like macro, `aether! { … }`, over ordinary Rust. It is a **transpiler,
not a runtime**: every construct expands at compile time to the canonical hand-written engine
surface — the same `#[derive(Component)]` struct, the same `Query<D, F>` signature, the same
`impl Plugin`. There is no interpreter, no registry, no reflection, and no second codegen path.
`boyko_macros` stays the single codegen authority; Aether hands it the annotated items you would
have typed yourself.

Three crates, because the three can prove three different classes of claim:

| Crate | Role | Depends on the engine |
|---|---|---|
| `aether` | The user-facing `#[proc_macro]`. A shim: it forwards to `aether_lang`. | no |
| `aether_lang` | The whole language — `parse.rs`, `ctx.rs`, `expand.rs`, `ast.rs`, `diag.rs`. A plain library, so `expand_block` is unit-testable token-for-token without a compiler session. | **no** — every `::boyko_*` path it emits is a *token*, resolved in your crate |
| `aether_tests` | The integration crate, with the real engine in its dev-dependencies. Where emitted paths become real compilations, and where the `trybuild` goldens live. | yes |

`aether_lang` staying engine-free is the same no-cycle rule `boyko_macros` follows, and it is the
reason the [gates](#gates) come in three lanes: token pins in `aether_lang` cannot notice an engine
change *at all*. See [Tokens, not dependencies](#tokens-not-dependencies).

## How to read this page

**The source is the authority.** Everything here is read out of
`crates/aether_lang/src/{parse,expand,ast,ctx,diag}.rs` and pinned by the `.stderr` goldens in
`crates/aether_tests/tests/ui/`. `docs/AETHER-LANG-PLAN.md` is design *intent* and the narrative
book pages are *explanation*; where either disagrees with the code, the code wins and this page
says so — collected in [Where the plan and the source disagree](#where-the-plan-and-the-source-disagree).
Claims that are read off a code path with no test pinning them are collected in
[Stated limits](#stated-limits).

**Diagnostic text is verbatim.** Every message quoted here is either copied from a `.stderr` golden
(with that fixture's concrete idents) or is the `format!` string from the source with its
placeholders left in `{braces}`. The backticks inside a message are part of the message.

**Table conventions.** Refusal tables are uniform: `Trigger | Message | Span | Golden`.
`✓ <fixture>` names the `tests/ui/<fixture>.rs` + `.stderr` pair that pins the text *and* its
line:column; `—` means no golden — many of those are still pinned as text by a `fails_with(…)` unit
test, which asserts the message but never the span. The span shorthand is defined once, in
[How to read a span](#how-to-read-a-span).

---

## Cheat sheet

The whole v1 surface in one block. Precise productions live with each construct; `?`, `*`, `+` and
`|` are the usual, and [Notation](#notation) spells out the rest.

```ebnf
block     = ( "aether" "v1" ";" )? construct* ;          (* no separator between constructs *)

construct = component | tag | bundle | event | system | plugin | machine | material | scene ;

component = "component" UpperCamel "{" ( comp_item ("," comp_item)* ","? )? "}" ;
comp_item = IDENT ":" TYPE
          | "requires" PATH ("," PATH)*
          | ("on_add"|"on_insert"|"on_replace"|"on_remove") "=" PATH
          | "no_bundle" ;

tag       = "tag" UpperCamel ( "(" "bitset" ")" )? ";" ;

bundle    = "bundle" UpperCamel "{" ( IDENT ":" TYPE ("," IDENT ":" TYPE)* ","? )? "}" ;   (* ≤16 *)

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

| Construct | Name case | Terminator | Emits | Needs a `plugin`? | Detail |
|---|---|---|---|---|---|
| `component` | UpperCamel | `}` | `#[derive(::boyko_macros::Component)] pub struct` | no | [↓](#component) |
| `tag` | UpperCamel | `;` | unit `Component`, optionally `storage = "bitset"` | no | [↓](#tag) |
| `bundle` | UpperCamel | `}` | `#[derive(::boyko_macros::Bundle)] pub struct` | no | [↓](#bundle) |
| `event` | UpperCamel | `}` | `#[::boyko_macros::event] pub struct` | no | [↓](#event) |
| `system` | snake_case | `}` | `pub fn` with the desugared signature | only if it has a clause | [↓](#system) |
| `plugin` | UpperCamel | `;` | `pub struct` + `impl Plugin` holding every sibling registration | — | [↓](#plugin) |
| `machine` | UpperCamel | `}` | flat `States` enum + one transition fn per (leaf, event) | **yes** | [↓](#machine) |
| `material` | lowercase | `}` | `#[inline] pub fn` over `Material::new` / `with_textures` | no | [↓](#material) |
| `scene` | lowercase | `}` | `pub fn` spawning the declared world | no | [↓](#scene) |

---

## Notation

| Symbol | Meaning |
|---|---|
| `"x"` | literal token |
| `X?` `X*` `X+` | optional / zero-or-more / one-or-more |
| `A \| B` | alternation |
| `IDENT` | a `syn::Ident` (raw idents `r#foo` included) |
| `PATH` `TYPE` `EXPR` | `syn::Path` / `syn::Type` / `syn::Expr`, parsed verbatim with the user's spans |
| `EXPR_NB` | `Expr::parse_without_eager_brace` — stops before a `{` |
| `TOKENS` | an unvalidated `proc_macro2::TokenStream` (a verbatim body) |
| `UpperCamel` `snake_case` `lowercase` | an `IDENT` subject to the [case gate](#naming-and-case-gates) |

The grammar is **Rust-lexable**: it is parsed with `syn` off the `aether!` invocation's token
stream, so all delimiter nesting and balancing is the lexer's, not the grammar's.

---

## The block

The whole macro input **is** the block; the `aether! { … }` braces belong to the invocation, not to
the grammar. An empty block is legal and expands to nothing.

Constructs have **no separator token**. A stray `;` between two constructs is not accepted — the
block loop expects an ident head and reports
``expected a construct keyword (component, tag, …)``.

### The version header

```ebnf
version_header = "aether" version ";" ;
version        = "v1" ;                    (* the only spelling this crate speaks *)
```

| | Value |
|---|---|
| Accepted spellings | `v1` — the single row of `SYNTAX_VERSIONS` (`ast.rs`) |
| Absent header | read as `SyntaxVersion::CURRENT` = `V1` |
| Dispatch site | `expand_inner` matches on `block.version`; `SyntaxVersion::V1 => expand_v1(block)` |

The header is a **gate, never a dialect**. `aether v1; component Health { hp: f32 }` and
`component Health { hp: f32 }` expand byte-for-byte identically (pinned by
`the_version_header_is_parsed_accepted_and_gated`). A v2 grammar adds a second arm to that
exhaustive match, which is what makes the compiler enumerate every site that must grow one.
`SyntaxVersion` is one table carrying spelling *and* dispatch token together, so what the diagnostic
prints and what the parser accepts cannot diverge.

A header-only block (`aether v1;`) is legal and expands to **nothing, quietly** — asserted
separately from the garbage-input loop in `recovery_terminates_and_never_panics_on_garbage`, because
folding it in as a disjunct made every other input's failure indistinguishable from this one's
success.

**Header claiming rule.** If the block's *first* ident is `aether`, the header is claimed
unconditionally — no construct keyword spells it — so a malformed header gets the header's own
diagnostic rather than ``unknown construct `aether` ``.

**The header is the one non-recoverable position.** `AetherBlock::parse` contains exactly one `?`,
`parse_version_header(input)?`, so *all three* of its failure modes abort the entire block: no
construct expands and one `compile_error!` is emitted. Everything else in the language goes through
[recovery](#recovery). The reason is stated at the top of `parse.rs`: continuing past an unspoken
version means judging v2 source by v1 rules and reporting faults the author never committed.

| Trigger | Message | Span | Golden |
|---|---|---|---|
| `aether;` — no version ident | ``the syntax-version header names a version: `aether v1;` `` | **failed-parse** | — |
| `aether v2; …` | ``unknown aether syntax version `v2`; this aether speaks: v1 (did you mean `v1`?)`` | **name** (the version ident) | ✓ `version_header_unknown` |
| `aether v1 component …` — no `;` | ``the syntax-version header ends with `;` (`aether {vs};`)`` | **failed-parse** | — |
| `aether` at any later construct position | ``the `aether v1;` syntax-version header is the block's FIRST item — move it above every construct`` | **name** (`aether`) | ✓ `version_header_out_of_place` |

The misplaced header is *not* a block-level abort, and is deliberately **not** routed through the
unknown-construct diagnostic: `aether` is not one of the nine, so that path would tell the reader
the keyword does not exist, which is false and unactionable. It is dispatched as its own arm, fails,
and is recorded as a broken construct — one error, and the constructs around it still expand.
Because `aether` has no row in `CONSTRUCT_KEYWORDS`, it participates in no whole-block rule and
mints no stub.

> **Source-vs-doc.** `parse.rs`'s module doc says the block aborts for "exactly one
> thing, a syntax-version header naming a version this parser does not implement". The code aborts
> for a missing version ident and a missing `;` as well — all three are on the `?` path.

### Construct-keyword registry

`diag::CONSTRUCT_KEYWORDS`, in registry order. This is both the "this aether supports:" list and the
did-you-mean candidate set. The registry is **closed**: every keyword in it dispatches to a real
parser, and no keyword outside it is recognised — the "planned construct" arm that used to name
unshipped rungs was removed with `scene` at rung A6.

```text
component, tag, bundle, system, event, plugin, machine, material, scene
```

That is the order the *diagnostic* prints. The dispatch `match` in `AetherBlock::parse` lists them
in a different order (`component, tag, bundle, event, system, plugin, machine, material, scene`);
only the printed order is observable.

| Trigger | Message | Span | Golden |
|---|---|---|---|
| head ident is not one of the nine | ``unknown construct `compnent`; this aether supports: component, tag, bundle, system, event, plugin, machine, material, scene (did you mean `component`?)`` | **name** (the head ident) | ✓ `unknown_construct`, ✓ `no_planned_construct_remains` |
| construct position is not an ident at all (a literal, `#`, a stray `,`) | ``expected a construct keyword (component, tag, …)`` | **stream** | — |

### Separators and terminators

Trailing commas are allowed **everywhere** a comma-separated list exists.

| List | Empty allowed | Trailing `,` |
|---|---|---|
| component items | yes | yes |
| `requires` path list | no (≥1) | yes (surrendered to the item loop) |
| bundle fields | yes | yes |
| event fields | yes | yes |
| `entity( … )` context | no (≥1) | yes |
| system / handler / transition params | yes (`()`) | yes |
| `query<D, …>` filters | yes | yes (before `>`) |
| material keys | syntactically yes (then refused for missing `base`) | yes |
| colour tuples, `at (…)`, Tuple3 key values | arity-checked, so effectively no | yes |
| node props | yes | yes |
| `children: [ … ]` | parses, then refused | yes |
| `plane(…)` / `cube(…)` / `mesh(…, …)` | no | yes |

Lists **without** a separator, where a comma is an error: top-level constructs, machine/state items,
scene items.

| Position | `;` |
|---|---|
| `aether vN;` header | required |
| `tag NAME;` / `tag NAME(bitset);` | required |
| `plugin NAME;` | required |
| machine's opening `initial X;` | required |
| state's `initial X;` | required |
| transition with no action block | required (`;` *or* a `{ … }` action) |
| scene `let NAME = …;` | required |
| scene node | **optional** |
| after a brace-bodied construct's `}` (`component`, `bundle`, `event`, `system`, `machine`, `material`, `scene`) | not accepted |

### How expressions are bounded

| Position | Parser | Consequence |
|---|---|---|
| `when EXPR` (system clause) | `Expr::parse_without_eager_brace` | the system body `{` is never swallowed |
| `if EXPR` (transition guard) | `Expr::parse_without_eager_brace` | the `=>` / action brace is never swallowed |
| `at EXPR` (unparenthesized) | eager `Expr` | **`at BARE_PATH { … }` parses as one struct literal** and the node body is lost — see [the swallowed-body trap](#the-swallowed-body-trap) |
| `sdf EXPR` | eager `Expr` | the same trap, and `sdf` has no key table, so no Aether diagnostic can attach |
| material key values, colour components | eager `Expr` | bounded by `,` or the body `}` |
| node key values, `at` tuple components, bare-expression props | eager `Expr` | bounded by `,` / `)` / `}` |
| mesh source arguments | eager `Expr` | bounded by `,` / `)` |
| system body, `enter`/`exit` body, transition action | verbatim `TokenStream` | consumes the whole braced group unexamined |
| `requires`, `in`, `before`/`after`, hook `= …`, transition event, filter operand, `material:` target | `syn::Path` / `syn::Ident` | — |
| field types, param types | `syn::Type` | — |

A `let` scrutinee is refused in both `EXPR_NB` positions (`reject_let_binding`): `Expr::Let` is a
real expression node, legal only inside an `if`/`while` scrutinee, and Aether splices it into
`if !(…)` or `.run_if(…)` where a `let` is not valid Rust at all.

### Where the grammar is contextual

Every keyword below is a keyword in exactly one position; everywhere else it is an ordinary ident.
The only genuine Rust keyword tokens the grammar uses are `in`, `let`, `if`, `mut`.

| Word | Keyword only when | Otherwise |
|---|---|---|
| `aether` | the block's first ident | a hard "move it above every construct" error at any other construct position; usable as any *name* |
| the nine construct keywords | at construct-head position | usable as field names, path segments, etc. |
| `requires`, `no_bundle`, the four hook keys | bare, at a `component` item head | `no_bundle::C` is a path segment; the item loop's lookahead explicitly lets `ident ::` continue a `requires` path |
| `bitset` | inside `tag NAME( … )` | — |
| `on`, `before`, `after`, `when` | at a system clause head | (`in` is the Rust token, peeked first) |
| `startup`, `update`, `fixed` | directly after a system's `on` | — |
| `query`, `res`, `local`, `events`, `emit` | followed by `<` | fall through as a verbatim `TYPE` |
| `commands` | not followed by `::` or `<` | verbatim `TYPE` |
| `mut` (type position) | followed by `res` | hard error |
| the six filter keywords | after a `,` inside `query< … >` | — |
| `entity` (event field type) | bare — no `::`, no `<` after it | a user type named `entity`, reachable via a qualified path or generics |
| `initial`, `state`, `enter`, `exit`, `on` | at a machine/state item head | — |
| the seven material keys | at a material key head | — |
| `plane`, `cube`, `mesh` | as the source head of a scene `let` | — |
| the eight node heads | at a scene node head | — |
| `at` | directly after a node head, and **not** followed by `::` or `!` | a component expression named `at` reaches the prop fallback |
| `casts_shadow` | in prop position, not followed by `:`, `::` or `!` | `casts_shadow: X` is parsed as a keyed prop and fails the head's key lookup |
| `material`, `children` | as prop names followed by a **single** `:` | `ident ::` opens a path, `Ident { … }` a struct literal — both fall through as component expressions |

The scene prop dispatcher's keyed test is exactly `ident` + single `:` + not `::`; everything
failing it is tried as `casts_shadow` and then as a bare component expression.

---

## Naming and case gates

The case convention is enforced at **parse time, on the name's own span**, so a case fault never
travels into a derive's output or into a call site. The rule is mechanical: *a construct whose name
becomes a type is UpperCamelCase; a construct whose name becomes a value (a fn) is not.*

| Construct | Required case | Message base (verbatim) |
|---|---|---|
| `component` | UpperCamelCase | ``component names are UpperCamelCase — they expand to types`` |
| `tag` | UpperCamelCase | ``tag names are UpperCamelCase — they expand to types`` |
| `bundle` | UpperCamelCase | ``bundle names are UpperCamelCase — they expand to types`` |
| `event` | UpperCamelCase | ``event names are UpperCamelCase — they expand to types`` |
| `plugin` | UpperCamelCase | ``plugin names are UpperCamelCase — they expand to types`` |
| `machine` | UpperCamelCase | ``machine names are UpperCamelCase — they expand to enums`` |
| `state` (inside `machine`) | UpperCamelCase | ``state names are UpperCamelCase — leaves become enum variants`` |
| `material` | lowercase | ``material names are lowercase — they expand to builder functions, not types`` |
| `scene` | lowercase | ``scene names are lowercase — they expand to spawn fns, not types`` |
| `system` | snake_case | ``system names are snake_case — they expand to fns (rename `Foo`)`` |

Each message gains a parenthesized rename **only when the suggestion actually differs** from what
was typed — a self-identical rename explains nothing:

```text
error: component names are UpperCamelCase — they expand to types (rename `health` to `Health`)
 --> tests/ui/lowercase_component.rs:5:15
error: material names are lowercase — they expand to builder functions, not types (rename `Gold` to `gold`)
 --> tests/ui/material_name_is_lowercase.rs:7:14
```

Goldens: ✓ `lowercase_component`, ✓ `material_name_is_lowercase`, ✓ `scene_name_is_lowercase`.

**Unicode-correct.** `upper_camel_gate` / `lowercase_gate` classify with `char::is_uppercase`, not
an ASCII probe. `component Здоровье { hp: f32 }` expands normally, and `component здоровье` is
refused with a real, different suggestion — ``rename `здоровье` to `Здоровье` ``. An ASCII probe
would have refused the first and produced a self-identical rename on the second.

**Raw identifiers.** A raw ident prints *with* its escape, so both gates strip `r#` before
classifying and before suggesting: `component r#Foo { x: u32 }` is accepted, and `component r#health`
suggests `Health` (not `R#health`). Pinned by `the_case_gate_reads_through_a_raw_ident_escape`.

**`system` is the exception, on both counts.** Its check is not `lowercase_gate` — it is an inline
`name_str.starts_with(char::is_uppercase)` on the raw `to_string()` in `parse_system`. So it does
**not** strip `r#`, and it computes **no** rename suggestion; its message names the offending
spelling only. See [Stated limits](#stated-limits) for what that means for `system r#Foo`.

### Two snake_case implementations, one specification

The rename **suggestion** (`parse::snake_case`) and the **generated-name collapse**
(`expand::snake`) are separate code on purpose: coupling a diagnostic's wording to a codegen naming
rule makes either one hostage to the other. They implement one rule, and
`both_snake_case_implementations_agree_on_the_same_rule` pins them equal on the cases that
distinguish it.

The rule, at each uppercase char: open a new word when the previous char was lowercase or a digit
(`GameFlow` → `game_flow`), or when the previous was uppercase and the **next** is lowercase
(`UIState` → `ui_state`, the `S` opening `state`). A run of capitals is one word: `GOLD` → `gold`,
`HTTPProbe` → `http_probe`. A name that already spells the break (`A_B`) gets one separator, not two.

The collapse is **lossy and deliberately so**: `AB` and `Ab` both collapse to `ab`. That loss is
caught by Aether on the user's tokens rather than by rustc on generated ones — see
[machine flattening collisions](#refusals--machine).

---
## Data constructs

Four constructs carry data. All four are **surface only** — each parses to a shallow AST node and
re-emits the canonical `boyko_macros` item a person would have hand-written (Decision A3). Aether
owns the *grammar* and a small set of pre-checks where it has a strictly better span; every semantic
rule stays with the derive or the kernel. Narrative: [Data constructs](data-constructs.md).

| Construct | Emits | Downstream authority |
|---|---|---|
| `component` | `#[derive(::boyko_macros::Component)] pub struct` | `Component` derive |
| `tag` | `#[derive(::boyko_macros::Component)] pub struct NAME;` (unit) | `Component` derive + EnableTag store |
| `bundle` | `#[derive(::boyko_macros::Bundle)] pub struct` | `Bundle` derive |
| `event` | `#[::boyko_macros::event] pub struct` | `#[event]` attribute macro |

### Rules shared by all four

| Rule | Detail |
|---|---|
| Name case | UpperCamelCase, gated at parse on the name's own span — see [Naming and case gates](#naming-and-case-gates) |
| Visibility | Always `pub` — struct and every field. There is no visibility syntax in the grammar. |
| Generics / `where` | Not supported. `component A<T> {…}`, `bundle B<T> {…}`, `event E<T> {…}` and a `where` clause all fail with syn's `expected curly braces`; `tag A<T>;` fails with the tag-semicolon message. |
| Field attributes | Not supported. A doc comment or `#[allow(…)]` on a field is a parse error. Construct-level attributes are also refused — the block parser wants a construct keyword. |
| Trailing commas | Always permitted, in every list. |
| Emission order | Source order across the whole block; the item list is a flat, deterministic stream. |
| Duplicate names | **Not Aether's** for these four — they are type-producing, so rustc's E0428 reports on both user idents. See [Block rules](#duplicate-fn-names-across-kinds). |
| Plugin participation | None. `plugin_impl` collects only `system`, `scene` and `machine`. Notably, **an `event` is not registered for you.** |
| Recovery stub | `#[allow(dead_code, non_camel_case_types)] pub struct NAME;` beside the `compile_error!`, so the name keeps resolving. See [Recovery](#recovery). |

---

### `component`

```ebnf
component = "component" IDENT "{" ( comp_item ( "," comp_item )* ","? )? "}" ;
comp_item = field | requires | hook | "no_bundle" ;
field     = IDENT ":" TYPE ;
requires  = "requires" PATH ( "," PATH )* ;
hook      = ("on_add" | "on_insert" | "on_replace" | "on_remove") "=" PATH ;
```

Items are comma-separated, may interleave freely, and a trailing comma is allowed; an empty body
(`component Marker {}`) is legal. An item's head must be an ident: `requires`, `no_bundle` and the
four hook keys are matched first, and anything else is read as a field name that must be followed
by `:`.

| Item | Type | Default | Emits |
|---|---|---|---|
| `IDENT : TYPE` | verbatim `syn::Type` | — | `pub IDENT: TYPE` in the struct body |
| `requires` | one or more `syn::Path`, may repeat | none | one merged `#[require(P₁, P₂, …)]` |
| `on_add` / `on_insert` / `on_replace` / `on_remove` | `syn::Path` (a fn path) | none | a key inside `#[component(…)]` |
| `no_bundle` | flag | `false` | `no_bundle` key inside `#[component(…)]` |

**The `requires` list terminates on lookahead, not on a fixed count.** After a `,` the parser peeks
one ident and stops the list only if that ident is a **bare** `requires` / `no_bundle` / hook key, or
is followed by a single `:` (a field). An ident followed by `::` always *continues the path*, so
`requires A, no_bundle::C` is one two-path list — pinned by
`requires_list_accepts_keyword_headed_paths_at_every_position`.

#### Emission — the pinned §3.1 pair

```rust,ignore
component Health {
    current: f32,
    max: f32,
    requires Regen,
    on_add = heal_full,
}
```

```rust,ignore
#[derive(::boyko_macros::Component)]
#[require(Regen)]
#[component(on_add = heal_full)]
pub struct Health {
    pub current: f32,
    pub max: f32
}
```

Attribute order is fixed: `#[derive]`, then `#[require]` (omitted when empty), then
`#[component(…)]` (omitted when there are no hooks and no `no_bundle`). Hook keys are emitted in
**declaration order**, with `no_bundle` last. The item is `quote_spanned!` at the user's name — see
[Span discipline](#span-discipline).

#### Consequences and limits

- A fieldless `component Marker {}` emits `pub struct Marker {}` — a ZST the derive's auto-tag
  detection handles. [`tag`](#tag) is the dedicated spelling.
- `requires` takes **bare paths only**. The derive's `#[require(...)]` accepts three entry forms —
  `B` (⇒ `B::default()`), `C = expr`, and `D(args)` — but Aether parses a `syn::Path`, so **only the
  `B` form is reachable**: a required component through Aether is always constructed by `Default`.
  `requires Mass = Mass(1.0)` and `requires Mass(1.0)` both fail with
  ``expected `,` between component items``.
- `no_bundle` suppresses the Phase-22 single-component `Bundle` emission.
- **There is no storage key on `component`.** `storage = "bitset"` is reachable only through
  [`tag(bitset)`](#tag); `storage = "dense"` has **no Aether surface at all** —
  `component A { storage = "dense" }` is parsed as a malformed field.
- A field cannot be named `requires`, `no_bundle`, or any of the four hook keys — a bare keyword at
  item-head position opens that item.
- Duplicate hook keys and a duplicate `no_bundle` are Aether's own pre-checks (better span than the
  derive's).

#### Refusals — `component`

| Trigger | Message | Span | Golden |
|---|---|---|---|
| no ident after `component` | ``expected a component name after `component` `` | **failed-parse** | — |
| lowercase name | ``component names are UpperCamelCase — they expand to types (rename `health` to `Health`)`` | **name** | ✓ `lowercase_component` |
| body item head is not an ident (a `#[doc]`, a literal) | ``expected a field, `requires`, a hook key, or `no_bundle` `` | **stream** | — |
| `requires` with no path (`requires: u32`) | ``` `requires` takes one or more component paths ``` | **failed-parse** | — |
| second `no_bundle` | ``duplicate `no_bundle` `` | **flag** (the second) | — |
| hook key without `= path` (`on_add: u32`) | ``hook `on_add` takes `= path` (a fn path)`` | **failed-parse** | — |
| same hook key twice | ``duplicate hook `on_add` `` | **key** (the second) | ✓ `duplicate_hook` |
| a field name not followed by `:` — includes `on_ad = f`, `frobnicate,`, `storage = "dense"` | ``expected `:` after field `hp` (or a known item: requires / on_add / on_insert / on_replace / on_remove / no_bundle)`` | **name** of the field | ✓ `recovery_one_typo_costs_one_error`, ✓ `recovery_duplicate_type_name_with_a_broken_twin` |
| two items with no comma between | ``expected `,` between component items`` | **stream** | — |

> **Plan-vs-source.** §3.1 predicts ``on_ad = heal_full`` → ``unknown component key … (did you mean
> `on_add`?)``. **No such diagnostic exists**: an unknown bare item head falls into the field branch
> and reports the missing `:`. There is no did-you-mean for component item keys, unlike system
> clauses, query filters and material keys.

---

### `tag`

```ebnf
tag = "tag" IDENT ( "(" "bitset" ")" )? ";" ;
```

A tag has **no body** — the declaration ends with `;`. `(bitset)` is the only modifier the grammar
knows.

| Form | Emits | Storage |
|---|---|---|
| `tag Player;` | `#[derive(::boyko_macros::Component)] pub struct Player;` | ordinary signature (table) storage; the derive's ZST auto-tag path |
| `tag Stunned(bitset);` | `#[derive(::boyko_macros::Component)]`<br>`#[component(storage = "bitset")]`<br>`pub struct Stunned;` | the EnableTag bitset backend |

#### What `(bitset)` means

A bitset tag is not a component you spawn. From `boyko_ecs`'s `enable/mod.rs`: an `EnableTag` "never
enters an archetype signature mask and owns no `ComponentPool`; toggling a flag is a single atomic
read-modify-write at `(archetype, row)` with no migration and no structural-generation bump."

| Property | Bitset tag | Plain tag |
|---|---|---|
| Archetype signature bit | **no** | yes |
| `ComponentPool` column | **no** | yes (ZST) |
| Toggle API | `world.enable::<T>(e)` / `disable::<T>(e)` / `is_enabled::<T>(e)` | spawn / insert / remove |
| Cost of a toggle | O(1) warm; no migration, no structural-generation bump, **no hook or observer fires**, no deferred drain. Dead/stale entities are a silent no-op. | archetype migration |
| Single-component `Bundle` | suppressed — `storage = "bitset"` implies `no_bundle` | emitted |
| Clone / serialize metadata | suppressed (no pool to read) | emitted |
| Lifecycle hooks | the derive **rejects** combining them with `storage = "bitset"`; Aether's `tag` grammar has no hook keys, so this is unreachable from Aether | supported via `component` |
| `Added<T>` / `Changed<T>` | **compile-rejected.** `const STORAGE_IS_BITSET: bool = true` trips a const-assert: ``Added/Changed are not supported on bitset enable tags (no tick storage); use Enabled<T>/with_enabled or query the underlying data`` | supported |
| Query filter to use | `Enabled<T>` / `Disabled<T>` — in Aether's `system` sugar, `enabled T` / `disabled T` | `With<T>` / `Without<T>` |

`Enabled<T>` / `Disabled<T>` carry their own shape constraints: each **requires a positive
archetypal term to bound iteration, or must be the sole single term**; neither can appear in `Or<>`,
be combined with `Added`/`Changed` in the same query, or be used with `for_each_chunk`. Do **not**
pair `Enabled<T>` with `&T` — a bitset tag has no pool to read; pair it with the real data
components (`Query<&Pos, Enabled<Stunned>>`).

Note that `With<T>` / `Without<T>` on a bitset tag carries **no** compile-time refusal (`With`'s
consts gate on `STORAGE_IS_DENSE`, not `STORAGE_IS_BITSET`), while a bitset tag never enters a
signature. `enabled` / `disabled` are the filters for it. See [Stated limits](#stated-limits).

#### Refusals — `tag`

| Trigger | Message | Span | Golden |
|---|---|---|---|
| no ident after `tag` | ``expected a tag name after `tag` `` | **failed-parse** | — |
| lowercase name | ``tag names are UpperCamelCase — they expand to types (rename `player` to `Player`)`` | **name** | — |
| `(` with a non-ident inside — `tag T();` | ``the only tag modifier is `(bitset)` — the EnableTag backend`` | **stream** of the paren body | — |
| a modifier ident other than `bitset` | ``unknown tag modifier `dense`; the only one is `bitset` (the EnableTag backend)`` | **flag** | ✓ `bad_tag_modifier` |
| extra tokens after `bitset` | ``` `(bitset)` takes nothing else ``` | **stream** of the paren body | — |
| missing `;` (e.g. a `{ … }` body) | ``a tag declaration ends with `;` (tags have no body — a component with fields wants `component`)`` | **failed-parse** | ✓ `tag_missing_semicolon` |

The modifier refusal has **no** did-you-mean — the candidate set is one literal and the message names
it outright.

> **Plan-vs-source.** §3.1 predicts ``unknown tag storage `bitmap`; the only tag storage modifier is
> `bitset` ``. Shipped: ``unknown tag modifier `bitmap`; the only one is `bitset` (the EnableTag
> backend)``. The plan also says dense components "will surface as another `component` key" —
> not shipped; there is no storage key on `component` at all.

---

### `bundle`

```ebnf
bundle       = "bundle" IDENT "{" ( bundle_field ( "," bundle_field )* ","? )? "}" ;
bundle_field = IDENT ":" TYPE ;                (* at most 16 *)
```

The whole construct. No modifiers, no keys.

```rust,ignore
bundle Projectile { pos: Position, vel: Velocity }
```

```rust,ignore
#[derive(::boyko_macros::Bundle)]
pub struct Projectile {
    pub pos: Position,
    pub vel: Velocity
}
```

**Arity cap.** `MAX_BUNDLE_ARITY = 16`. This is Aether's **only** pre-check on `bundle`, kept because
it owns the friendlier span: the error lands on the **17th field's own name**, before the derive's
downstream refusal. The derive mirrors the same ceiling with its own message, ``Bundle supports at
most 16 components (MAX_BUNDLE_ARITY); split the bundle and insert the remainder with
EntityCommands::insert``, kept in lock-step with the runtime stack-collector ceilings in
`spawn_at_command.rs` / `insert_command.rs` / `migration_helpers.rs`.

**A zero-field bundle parses but does not compile.** Aether accepts `bundle B {}` and emits
`pub struct B {}`; the derive then refuses it: ``Bundle requires at least one field; to spawn an
entity with zero components use Commands::spawn_empty()``. Aether does not duplicate that check.
Everything else — the named-struct rule, the no-generics rule, the static-cache codegen — belongs to
the derive.

#### Refusals — `bundle`

| Trigger | Message | Span | Golden |
|---|---|---|---|
| no ident after `bundle` | ``expected a bundle name after `bundle` `` | **failed-parse** | — |
| lowercase name | ``bundle names are UpperCamelCase — they expand to types (rename `pawn` to `Pawn`)`` | **name** | — |
| body item is not an ident | ``expected a bundle field`` | **failed-parse** | — |
| field name not followed by `:` | ``expected `:` after bundle field `{name}` `` | **name** of the field | — |
| a **17th** field | ``bundle arity is capped at 16 (`MAX_BUNDLE_ARITY`) — split it`` | **name** of the 17th field | ✓ `bundle_arity_cap` |
| two fields with no comma between | ``expected `,` between bundle fields`` | **stream** | — |

---

### `event`

```ebnf
event       = "event" IDENT "{" ( ev_field ( "," ev_field )* ","? )? "}" ;
ev_field    = IDENT ":" ( participant | TYPE ) ;
participant = "entity" "(" IDENT ( "," IDENT )* ","? ")" ;
```

| Field kind | Written | Emits |
|---|---|---|
| Participant | `name: entity(A, B)` | `#[participant(components = "A, B")] pub name: ::boyko_ecs::ecs::core::entity::entity::Entity` |
| Parameter | `name: Type` | `#[parameter] pub name: Type` |

`entity` is a **contextual keyword** at the field-type position only (`input_is_bare_entity`): it is
the participant marker only as a *bare* ident with no `::` and no `<` after it. `thing: my::entity`
is an ordinary parameter of a type that happens to be named `entity`. The component context is
**never defaulted** — a bare `victim: entity` is an error by design.

Participant context components must be **bare, unqualified, non-generic idents**. Aether refuses
`entity(foo::Bar)`, `entity(::A)` and `entity(Slot<A, B>)` *on the user's tokens*, because the
derive's `components = "…"` channel is a comma-separated ident list that it splits on `,` and mints
an `Ident` from each piece — a qualified path would panic the downstream macro with no user span, and
a generic argument's own comma would corrupt the split. The idents must resolve to real `Component`
impls at the expansion site, because `participant_info()` emits `<Comp as Component>::component_id()`
for each.

```rust,ignore
event Damage {
    victim: entity(Position, Health),
    amount: f32,
}
```

```rust,ignore
#[::boyko_macros::event]
pub struct Damage {
    #[participant(components = "Position, Health")]
    pub victim: ::boyko_ecs::ecs::core::entity::entity::Entity,
    #[parameter]
    pub amount: f32
}
```

Field **source order is preserved** in what Aether emits — participants and parameters may interleave
freely in your block.

#### The two-lane rewrite

`#[event]` is the layout authority. It sorts the fields into two lanes by marker and **replaces the
struct** with a two-field outer struct plus two substructs. Interleaving in the source is flattened;
within each lane, declaration order is preserved. For the `Damage` above, the full generated surface
is:

| Item | Shape |
|---|---|
| `Damage` | `#[repr(C)] pub struct Damage { pub participants: DamageParticipants, pub parameters: DamageParameters }` |
| `DamageParticipants` | `#[repr(C)] #[derive(Clone, Copy)] pub struct DamageParticipants { pub victim: Entity }` |
| `DamageParameters` | `#[repr(C)] #[derive(Clone, Copy)] pub struct DamageParameters { pub amount: f32 }` |
| `impl Damage` | `pub const EVENT_NAME: &'static str = "Damage";` |
| `impl Event for Damage` | `type Participants` / `type Parameters`; `event_id()` (minted once via a per-type `OnceLock`), `event_name()`, `new(participants, parameters)`, `participants()` / `participants_mut()` / `parameters()` / `parameters_mut()` |
| `impl Participants for DamageParticipants` | `participant_count() -> 1`; `participant_info() -> &'static [ParticipantInfo]` with `{ name: "victim", required_components: &[Position::component_id(), Health::component_id()] }`, leaked once behind a `OnceLock` |
| `impl Parameters for DamageParameters` | empty marker impl |

Substruct names are `{Name}Participants` / `{Name}Parameters` — always, and they are ordinary
top-level items you reference by those names. Aether writes every field `pub`, so both substructs are
constructible from anywhere the event type is.

```rust,ignore
// construction — the outer struct has exactly two fields, both substructs
w.send(Damage {
    participants: DamageParticipants { victim },
    parameters:   DamageParameters { amount: 2.5 },
})
.expect("send within lane capacity");

// reading — EventReader::read() yields &E
for e in r.read() {
    let who = e.participants.victim;
    let how_much = e.parameters.amount;
}
```

There is no flat constructor: `Damage { victim, amount }` does not exist after the rewrite.
`<Damage as Event>::new(participants, parameters)` is the trait-shaped equivalent.

#### Hard constraints inherited from the traits

| Constraint | Source |
|---|---|
| Every field type must be `Copy + 'static` | both substructs `#[derive(Clone, Copy)]`, and `Participants: 'static + Sized + Copy` / `Parameters: 'static + Sized + Copy`. A `String` parameter will not compile. |
| The event type must be `Send + Sync + 'static` | `Event: 'static + Sized + Send + Sync` |
| **The event must not be a ZST** | `ZstCheck::<E>::NON_ZERO` is read in `EventDispatcher::preregister::<E>`: *"Event type is zero-sized; use a counter instead (add a non-ZST field)"*. An `event Ping {}` has empty participants **and** empty parameters, so the whole type is a ZST and this trips at monomorphisation. An event with at least one participant is non-ZST (`Entity` is `{ id, generation }`). |
| No generics | `#[event]` refuses them: `#[event] does not support generic structs (Q-001 scope)` — unreachable from Aether, whose grammar has no generics either |

**Declaring an event does not register it.** The `plugin` construct collects only systems, scenes and
machines, so nothing in an `aether!` block calls `preregister_event`. Lane registration is yours:
`world.preregister_event::<Damage>(EventConfig::default_for(lanes))` or
`preregister_event_default::<Damage>()`. There is no `App::add_event`. See
[Events](../concepts/events.md).

#### Refusals — `event`

| Trigger | Message | Span | Golden |
|---|---|---|---|
| no ident after `event` | ``expected an event name after `event` `` | **failed-parse** | — |
| lowercase name | ``event names are UpperCamelCase — they expand to types (rename `damage` to `Damage`)`` | **name** | — |
| body item is not an ident | ``expected an event field`` | **failed-parse** | — |
| field name not followed by `:` | ``expected `:` after event field `{name}` `` | **name** of the field | — |
| bare `entity` with no `( … )`, `v: entity()`, **or** a non-path inside the parens | ``participant fields name their component context: `entity(ComponentA, ComponentB)` `` | **name** (`entity`) / **failed-parse** | ✓ `participant_without_context` |
| a participant component that is qualified, multi-segment, or generic | ``participant context components are bare component idents (the `#[event]` channel is comma-separated identifiers) — found `X`; import the component and name it unqualified`` | first path segment's ident | — |
| two fields with no comma between | ``expected `,` between event fields`` | **stream** | — |

---
## Behaviour constructs

`system` sugars a system's **signature** and its **registration**; it never touches the body.
`plugin` is the block's single registration holder: it collects its siblings and emits one
`impl ::boyko_ecs::Plugin`. Narrative: [Systems & plugins](systems-and-plugins.md).

### `system`

```ebnf
system     = "system" IDENT param_list clause* "{" TOKENS "}" ;
param_list = "(" ( param ( "," param )* ","? )? ")" ;
param      = "mut"? IDENT ":" param_ty ;
clause     = "on" ("startup"|"update"|"fixed")
           | "in" PATH | "before" PATH | "after" PATH | "when" EXPR_NB ;
```

The parenthesised param list is **mandatory and unconditional** — `system tick {}` does not parse —
but it may be empty. `parenthesized!` is called with no `.map_err`, so a missing `(` produces syn's
own message, not one of Aether's.

Clauses run until the body brace, in **any order**, and `in`, `before`, `after`, `when` may repeat.
Only `on` is at-most-once. `in` is dispatched on the **real Rust `in` token** (`Token![in]`), checked
before the ident fork; `on` / `before` / `after` / `when` are contextual idents.

The body is a verbatim `TokenStream`, filled by `syn::braced!` and re-emitted as `{ #body }`. There
is no Aether expression syntax, no Aether control flow, no rewriting.

#### Parameter vocabulary

`expand.rs::param_ty_and_mut` is the whole table. Two path prefixes recur; both are emitted **fully
qualified as tokens** and both are the REAL nested engine paths (see
[Tokens, not dependencies](#tokens-not-dependencies)):

* `SYS` = `::boyko_ecs::ecs::core::system`
* `Q`   = `::boyko_ecs::ecs::core::iters::query`

| You write | Emitted type | Binding gets `mut` |
|---|---|---|
| `q: query<D>` | `Q::Query<D>` | inferred from `D` |
| `q: query<D, f>` | `Q::Query<D, Q::F<P>>` | inferred from `D` |
| `q: query<D, f1, f2, …>` | `Q::Query<D, (Q::F1<P1>, Q::F2<P2>, …)>` | inferred from `D` |
| `r: res<T>` | `SYS::Res<T>` | no |
| `r: mut res<T>` | `SYS::ResMut<T>` | **yes** |
| `l: local<T>` | `SYS::Local<T>` | no |
| `c: commands` | `SYS::Commands` | **yes** |
| `e: events<E>` | `SYS::EventReader<E>` | **yes** |
| `w: emit<E>` | `SYS::EventWriter<E>` | **yes** |
| `x: SomeType` | `SomeType`, unchanged | no |

**`mut` occupies two positions with different meanings.**

```text
system s( mut cmds : commands )      //  ^ binding-level `mut` — SysParam::explicit_mut
system s( r : mut res<T> )           //         ^ type-level `mut` — the ResMut sugar
```

The type-position `mut` pairs with `res` and **nothing else**. An explicit binding `mut` and an
inferred one produce a single `mut` — the emitter is
`(p.explicit_mut || inferred_mut).then(|| quote!(mut))`.

#### Query filters

| Sugar | Emitted filter |
|---|---|
| `with P` | `Q::With<P>` |
| `without P` | `Q::Without<P>` |
| `added P` | `Q::Added<P>` |
| `changed P` | `Q::Changed<P>` |
| `enabled P` | `Q::Enabled<P>` |
| `disabled P` | `Q::Disabled<P>` |

`query_type` shapes the filter position by count:

| Filters | Emission |
|---|---|
| 0 | `Q::Query<D>` — the `F` parameter is omitted entirely (kernel default `()`) |
| 1 | `Q::Query<D, Q::With<Alive>>` — **bare, no one-tuple** (the kernel implements `QueryFilter` for a bare filter) |
| ≥ 2 | `Q::Query<D, (F₁, F₂, …)>` |

The query **data** `D` is verbatim Rust with the user's spans and Aether never validates it;
`QueryData` is the authority.

#### Mutability inference

A parameter whose expansion needs `&mut self` access receives a `mut` binding automatically. Two
details are load-bearing:

* **The `query<D>` scan is token-exact, not textual.** `type_mentions_mut` / `stream_mentions_mut`
  walk the `TokenStream` of `D` and match the *identifiers* `mut` and `Mut` (recursing into groups).
  A type named `Mutation` or a segment `permutation` never false-positives — the pinned test
  `mutability_inference_follows_the_param_table` includes `f: query<&Mutation>` and asserts it emits
  **without** `mut`.
* **`events<E>` is in the inference set.** This engine's `EventReader::read` takes `&mut self`, so a
  non-`mut` reader binding could never be read. Recorded in the source as a deviation from the plan's
  inference list.

The verbatim escape is **never** inferred — Aether does not inspect types it does not own, so
`mut assets: NonSendResMut<…>` must be written with the binding `mut` by hand.

#### The verbatim escape and the contextual-keyword rule

A sugar keyword is a sugar **only when its own syntax follows it**; everything else falls through to
`SysParamTy::Verbatim` and is re-emitted untouched.

| Input | Route | Why |
|---|---|---|
| `query<…>` | sugar | `<` follows |
| `query(…)` | **refused** | parens where angles belong |
| `query::Thing` | verbatim | `::` follows, not `<` |
| `res<T>` / `local<T>` / `events<E>` / `emit<E>` | sugar | `<` follows |
| `res` (a bare user type named `res`) | verbatim | no `<` |
| `commands` | sugar | neither `::` nor `<` follows |
| `commands::Something` | verbatim | `::` follows |
| `&T`, `(A, B)`, any non-ident-led type | verbatim | the ident fork fails immediately |
| `NonSendRes<Gpu>`, `NonSendResMut<Assets<MeshGpu>>` | verbatim | not a sugar spelling (case-sensitive) |

`schedules_sets_and_the_escape_hatch_route_correctly` pins `draw(dev: NonSendRes<Gpu>)` expanding to
`pub fn draw(dev: NonSendRes<Gpu>)` — untouched, unqualified, no inferred `mut`.

Not something Aether checks: the engine implements `SystemParam` for tuples of arity
`0..=MAX_SYSTEM_PARAM_ARITY` (`= 12`); arities `13..=24` carry stub impls that `const { panic!(…) }`
at monomorphization. Aether emits no arity check of its own.

#### Clauses

| Clause | Repeatable | Stored as | Emits |
|---|---|---|---|
| `on startup` | no | `Schedule::Startup` | `app.add_startup_system(f);` at plugin `build` top level |
| `on update` | no | `Schedule::Update` | a statement inside `app.add_systems_cfg(\|b\| { … })` |
| `on fixed` | no | `Schedule::Fixed` | a statement inside `app.add_systems_cfg_in(::boyko_ecs::ecs::core::app::CoreSchedule::Fixed, \|b\| { … })` |
| *(no `on`)* | — | `None` → `bucket()` = `Update` | Main, unordered |
| `in PATH` | **yes** | `(Path, Span)` | `.in_set(PATH)`, appended in source order |
| `before PATH` / `after PATH` | **yes** | `(OrderKind, Path, Span)` | `.before(key)` / `.after(key)`, or `.before_set(PATH)` / `.after_set(PATH)` — see [Sibling ordering](#sibling-ordering) |
| `when EXPR` | **yes** | `(Expr, Span)` | `.run_if(EXPR)`, appended after every ordering call |

Call-chain assembly order in `bucket_stmts` is fixed: `b.add_system(f)` → every `.in_set(…)` in
source order → every ordering call in source order → every `.run_if(…)` in source order.

**`on startup` accepts no other clause.** The parser records the span of the first non-`on` clause
keyword and, after the clause loop closes, refuses the whole system at that span. Because the check
runs after the loop, clause order does not matter: `system s() in X on startup {}` is refused at `in`.

#### Refusals — `system`

Parse phase:

| Trigger | Message | Span | Golden |
|---|---|---|---|
| no ident after `system` | ``expected a system name after `system` `` | **failed-parse** | — |
| name starts with an uppercase char | ``system names are snake_case — they expand to fns (rename `Foo`)`` | **name** | — |
| two params with no comma between | ``expected `,` between params`` | **stream** of the paren body | — |
| param position holds no ident | ``expected a system param name`` | **failed-parse** | — |
| param name not followed by `:` | ``expected `:` after system param `q` `` | **name** of the param | — |
| `mut` in the type position with no ident after it | ``in the type position `mut` pairs only with `res`: `mut res<T>` `` | **failed-parse** | — |
| `mut <kw>` where `<kw>` is not `res` | ``in the type position `mut` pairs only with `res`: `mut res<T>` (found `local`)`` | **kw** | — |
| `query( … )` with parentheses | ``query takes angle brackets: `query<&mut Transform>` `` — a fixed literal; the example does not echo the user's type | **stream** at the `(` | ✓ `query_takes_angle_brackets` |
| `query<` with no data type | ``` `query<…>` opens with the query data (a type: `&T`, `&mut T`, a tuple, …) ``` | **failed-parse** | — |
| filter position holds no ident | ``expected a query filter: with, without, added, changed, enabled, disabled`` | **failed-parse** | — |
| an ident that is not a filter keyword | ``unknown query filter `wih`; filters are: with, without, added, changed, enabled, disabled (did you mean `with`?)`` | **kw** | — |
| a filter with no path | ``` `with` takes a component path ``` | **failed-parse** | — |
| `query<…` unterminated | ``expected `>` to close `query<…>` `` | **failed-parse** | — |
| `mut res` with no `<` | ``` `res` takes angle brackets: `res<T>` ``` | **failed-parse** | — |
| ~~bare `res`/`local`/`events`/`emit`~~ | **not refused** — the sugar arms are guarded by a `<` peek, so a bare keyword falls through to the verbatim-type escape and reaches rustc as an ordinary (unresolvable) type. Only `mut res` is committed to the sugar before the bracket check. *(An earlier revision of this row claimed a refusal the parser cannot produce; measured via `expand_block`.)* | — | — |
| `res`/`local`/`events`/`emit` unterminated | ``expected `>` to close `res<…>` `` | **failed-parse** | — |
| clause position holds no ident and no `{` | ``expected a clause (`on`, `in`, `before`, `after`, `when`) or the system body`` | **stream** | — |
| `in` with no path | ``` `in` takes a SystemSet path ``` | **failed-parse** | — |
| a second `on` | ``duplicate schedule clause; a system runs on exactly one schedule`` | **kw** of the second `on` | ✓ `duplicate_on_schedule` |
| `on` with no target ident | ``` `on` takes one of: startup, update, fixed ``` | **failed-parse** | — |
| `on` with an unknown target | ``unknown schedule `tick`; `on` takes one of: startup, update, fixed`` — exhaustive list, **no** did-you-mean | **name** of the target | — |
| `before`/`after` with no path | ``` `after` takes a SystemSet path or a sibling aether system name ``` | **failed-parse** | — |
| `when` with no expression | ``` `when` takes a condition expression (a fn implementing IntoSystem<(), bool, _>) ``` | **failed-parse** | — |
| `when let …` | ``` `let` bindings are not usable as a run condition — `when` takes a plain bool expression (bind with a `local<…>` param or match inside the body instead) ``` | the `let` token | — |
| any other clause head ident | ``unknown clause `afterr`; clauses are: on, in, before, after, when (did you mean `after`?)`` | **name** of the head | — |
| any clause other than `on` on a startup system | ``scheduling clauses other than `on` are rejected on startup systems — the engine runs them once, pre-loop`` | **kw** of the first non-`on` clause | — |

Whole-block phase — see [Block rules](#block-rules-aetherctx):

| Trigger | Message | Span | Golden |
|---|---|---|---|
| any scheduling clause with no `plugin` in the block | ``scheduling clauses (`on`, `after`, `when`, …) need a `plugin <Name>;` declaration in this block to hold the generated registration`` | **name** of the system | ✓ `clauses_need_a_plugin` |
| duplicate system name | ``duplicate system `tick` — each system expands to a fn of its own name, and two of one name is one fn defined twice`` + ``the first `system` of this name is here`` | second **name**, and the first | — |

---

### `plugin`

```ebnf
plugin = "plugin" IDENT ";" ;
```

`plugin` is a first-class member of `diag::CONSTRUCT_KEYWORDS`, so `pluging Movement;` gets both the
supported list and a did-you-mean.

#### When a plugin is required

`AetherCtx::build` refuses a block with **no plugin declared** if it contains either:

* a `system` for which `SystemDef::has_clauses()` is true — that is
  `schedule.is_some() || !in_sets.is_empty() || !orders.is_empty() || !whens.is_empty()`. Note this
  includes **`in`**, which the error message's own list abbreviates as ``(`on`, `after`, `when`, …)``;
* a `machine` (which needs somewhere to put `insert_state` and its transition registrations).

A **clause-free** `system` needs no plugin and expands to a bare `pub fn` and nothing else. A
`material` and a `scene` also require no plugin.

#### What the plugin collects

With a plugin present, **every sibling `system` is registered** — a clause-free one lands on Main
unordered. This is a recorded decision, not an accident: the plan's "register by hand" story applies
to plugin-free blocks.

`plugin_impl` emits exactly this shape, at the plugin construct's **own source position** among the
block's items (`expand_v1` walks `block.constructs` in order):

```rust,ignore
pub struct Movement;
impl ::boyko_ecs::Plugin for Movement {
    fn build(&self, app: &mut ::boyko_ecs::App) {
        // 1. inserts       — per sibling `machine`, in declaration order
        // 2. startup_calls — startup systems AND scene spawn fns, interleaved by declaration order
        // 3. main_block    — app.add_systems_cfg(|b| { … })                       (omitted if empty)
        // 4. fixed_block   — app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {…}) (omitted if empty)
    }
    fn name(&self) -> &'static str { "Movement" }
}
```

| # | Contents | Ordering rule |
|---|---|---|
| 1 | per `machine`: `app.insert_state(M::InitialLeaf);` then, **only if the initial-enter chain has an `enter` body**, `app.add_startup_system(__aether_<machine>_…);` | machine declaration order |
| 2 | `app.add_startup_system(f);` for every `on startup` system **and** for every `scene` spawn fn | one pass over `block.constructs` — the two kinds **interleave by declaration order**, so a scene declared before a startup system spawns before it runs |
| 3 | `b.add_system(f)…;` for every Update-bucket system, then every machine's transition registrations `b.add_system(__aether_…).run_if(in_state(M::Leaf));` | systems first, [topologically sorted](#topological-sort); machine registrations appended afterwards, in machine declaration order |
| 4 | `b.add_system(f)…;` for every Fixed-bucket system | topologically sorted |

Not collected by the plugin: `component`, `tag`, `bundle`, `event`, and `material`. A sibling
`material` emits its builder fn and the plugin's `build` stays empty — pinned by
`a_material_needs_no_plugin_and_a_sibling_plugin_does_not_register_it`, whose expected output is
literally `fn build(&self, app: &mut ::boyko_ecs::App) {}`. `app` is bound unconditionally in the
signature even when `build` is empty.

**At most one plugin per block.** `AetherCtx::build` scans `constructs ∪ broken` for the `plugin`
keyword; the second **named** one is refused, with both spans. A nameless broken `plugin` can only
*hold* the slot (there is nothing to print for it).

#### Refusals — `plugin`

| Trigger | Message | Span | Golden |
|---|---|---|---|
| no ident after `plugin` | ``expected a plugin name after `plugin` `` | **failed-parse** | — |
| lowercase name | ``plugin names are UpperCamelCase — they expand to types (rename `movement` to `Movement`)`` | **name** | — |
| missing `;` | ``a plugin declaration ends with `;` (the systems it registers are sibling `system` items)`` | **failed-parse** — in practice the **next construct's head** | ✓ `recovery_broken_plugin_keeps_the_block` |
| a second named `plugin` | ``one `plugin` per aether block — `A` already holds this block's registrations`` + ``the first `plugin` is here`` | second **name**, and the first | — |

---

### Sibling ordering

`before` / `after` take **either** a `SystemSet` path **or** the bare name of a sibling aether
`system`. `resolve_order` decides, and the decision is only ever attempted when the path is a
genuinely bare ident — `leading_colon.is_none() && segments.len() == 1 && segments[0].arguments.is_none()`.
Anything qualified or generic goes straight to the set path.

| Case | Result |
|---|---|
| bare ident naming a **broken** sibling `system` | `ResolvedOrder::Suppressed` — the edge is silently dropped, no diagnostic; it returns when the target parses |
| bare ident naming a sibling in the **same** bucket | `ResolvedOrder::Sibling` — target marked `needs_key`, `.before(__aether_k_target)` / `.after(__aether_k_target)` |
| bare ident naming a sibling on **`on startup`** | error — startup systems cannot be ordered against |
| bare ident naming a sibling in a **different** bucket | error — cross-schedule ordering is not expressible |
| bare ident within Levenshtein ≤ 2 of a sibling name | error naming the sibling |
| anything else | `ResolvedOrder::Set` — `.before_set(PATH)` / `.after_set(PATH)` verbatim |

A bare name that matches no sibling and is **not** within distance 2 is not an error: it becomes a
`SystemSet` path and rustc resolves it. That is the escape hatch the did-you-mean message points at.
The `SystemKey` local is `__aether_k_<system_name>`.

| Trigger | Message | Span | Golden |
|---|---|---|---|
| ordering against a sibling that runs on `startup` | ``ordering references `boot`, a startup system — startup systems run once, pre-loop, and cannot be ordered against`` | the bare path's ident | — |
| ordering against a sibling in a different bucket | ``sibling system `a` runs on a different schedule — cross-schedule ordering is not expressible`` | the bare path's ident | — |
| a near-miss bare name | ``` `read_inpt` is not a sibling aether system; a sibling `read_input` exists — system-to-system ordering uses the bare system name (a real SystemSet type this close in name must be referenced by a qualified path) ``` | the bare path's ident | — |
| a cycle of sibling edges inside one bucket | ``system ordering cycle among `a`, `c` — break one `before`/`after` edge`` + one ``…cycle member`` note per additional member | **name** of the first cyclic system, then each other | — |

#### Topological sort

Sibling ordering rides `SystemConfig::key()`, which means the **target must be registered before the
referrer** so its key exists. `bucket_stmts` therefore sorts emission inside each schedule bucket:

* indegree of member *i* = the number of its own `ResolvedOrder::Sibling` edges — `before` and
  `after` count identically, because both need the target's key;
* `Suppressed` and `Set` edges contribute nothing;
* selection is **stable Kahn**: `members.iter().find(|i| !emitted[i] && indeg[i] == 0)` over a
  source-index-ordered list, so the lowest source index wins every tie and the output is
  deterministic;
* a member that is the target of some edge is emitted as
  `let __aether_k_n = b.add_system(n)….key();`, otherwise as `b.add_system(n)…;`.

Because `resolve_order` already refuses cross-bucket sibling edges, every sibling target is
guaranteed to be a member of the same bucket.

```rust,ignore
aether! {
    plugin P;
    system a() on update before z {}
    system z() on update {}
}
```

```rust,ignore
app.add_systems_cfg(|b| {
    let __aether_k_z = b.add_system(z).key();   // emitted first: `a` needs its key
    b.add_system(a).before(__aether_k_z);
});
```

**Emission order is not execution order.** `z` is *registered* first; `.before(__aether_k_z)` is what
makes `a` *run* first. A cycle is a compile error naming every un-emitted member; the engine would
catch it too, at `build()`, as `ScheduleBuildError::OrderingCycle` — Aether reports it earlier and at
source.

---

### The arity allow

`expand.rs::arity_allow()` emits a bare `#[allow(clippy::too_many_arguments)]`, attached to exactly
**three** generated fn kinds — the ones whose arity the user controls:

| Generated fn | Carries the allow | Site |
|---|---|---|
| `system` fn | **yes** | `system_fn` |
| machine transition fn (`__aether_<machine>__<leaf>__<event>`) — merges the params of every handler it inlines | **yes** | `transition_fn` |
| machine initial-enter chain fn | **yes** | `initial_enter_fn` |
| `material` builder fn | no — nullary by construction | `material_fn` |
| `scene` spawn fn | no — demand-driven, at most four params | `scene_fn` |
| `plugin` struct / `Plugin` impl | n/a | `plugin_impl` |
| recovery stubs | no (they carry `#[allow(dead_code, …)]` instead) | `stub_item` |

The emission is **unconditional**, not gated on a parameter count: clippy's
`too-many-arguments-threshold` is *configuration* (default 7), so a count-gated emission would be
correct only for whoever kept the default and would go silently wrong in a crate that lowered it.

The measured motivation: under clippy 0.1.97 / rustc 1.97.1 an eight-param `system` produced
`warning: this function has too many arguments (8/7)` **spanned on the whole `aether!` token** — a
lint about a signature the author did not write, where no `#[allow]` of theirs can reach it. What the
gate does and does not cover is in [The clippy arity gate](#the-clippy-arity-gate).

---

### Worked example — pinned token-for-token

`expand.rs::the_section_3_3_before_after_pair_holds_verbatim`:

```rust,ignore
aether! {
    plugin Movement;

    system read_input(actions: res<ActionState>, mut cmds: commands)
        on update in InputSet
    { let _ = (&actions, &mut cmds); }

    system apply_velocity(q: query<(&mut Transform, &Velocity), with Player, without Frozen>,
                          time: res<Time>)
        on update
        after read_input
        when in_state(GameFlow::Playing)
    { for (t, v) in &mut q { t.translation += v.linear * time.delta_secs(); } }
}
```

```rust,ignore
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
#[allow(clippy::too_many_arguments)]
pub fn read_input(
    actions: ::boyko_ecs::ecs::core::system::Res<ActionState>,
    mut cmds: ::boyko_ecs::ecs::core::system::Commands
) { let _ = (&actions, &mut cmds); }
#[allow(clippy::too_many_arguments)]
pub fn apply_velocity(
    mut q: ::boyko_ecs::ecs::core::iters::query::Query<
        (&mut Transform, &Velocity),
        (::boyko_ecs::ecs::core::iters::query::With<Player>,
         ::boyko_ecs::ecs::core::iters::query::Without<Frozen>)
    >,
    time: ::boyko_ecs::ecs::core::system::Res<Time>
) { for (t, v) in &mut q { t.translation += v.linear * time.delta_secs(); } }
```

What the pin owns: `mut q` is **inferred** (the user wrote `q:`), the plugin item precedes the fns
because `plugin Movement;` is declared first, and the ordering edge put `read_input`'s `.key()`
capture ahead of its referrer.

---
## `machine`

A `machine` declares a Harel-lite chart: composite states, entry/exit actions, guarded event
transitions. **The hierarchy exists only inside the transpiler.** At expansion the chart is flattened
to one `enum` of leaf states, superstate handlers are copied down into the leaves, and each
`(leaf, event)` pair becomes one ordinary system gated by `run_if(in_state(leaf))`. Nothing walks a
hierarchy at run time and no new runtime type is introduced — the output is `States`, `State<S>`,
`NextState<S>`, `EventReader<E>` and `in_state`, all pre-existing kernel machinery. Narrative:
[State machines](state-machines.md).

```ebnf
machine     = "machine" IDENT "{" "initial" IDENT ";" state* "}" ;
state       = "state" IDENT "{" state_item* "}" ;
state_item  = "initial" IDENT ";"
            | "enter" param_list? "{" TOKENS "}"
            | "exit"  param_list? "{" TOKENS "}"
            | "on" PATH param_list? ( "if" EXPR_NB )? "=>" state_path ( "{" TOKENS "}" | ";" )
            | state ;
state_path  = IDENT ( "." IDENT )* ;        (* ROOT-anchored: `Playing.Paused` *)
```

No trailing `;` after the machine's closing brace. The machine-level `initial` is **required,
positional and a single ident** (not a path): it is parsed before the state loop, so it cannot appear
later. State items are **not** comma-separated; each self-terminates with `;` or a `{ … }`. Nesting
is unbounded, and the parser threads one declaration counter through the whole body so transitions
carry exact source order across the machine.

> **Shipped grammar is looser than the plan's EBNF.** §3.5 writes
> `state := 'state' IDENT '{' ('initial' IDENT ';')? handler* state* '}'` — `initial` first, then
> handlers, then nested states. `parse_state` is a flat `match` on each item head inside a
> `while !body.is_empty()` loop, so **the five state items may appear in any order and interleave
> freely**. The cardinality limits are enforced by occupancy checks, not by position.

### State items

| Item | Cardinality | Shape | Meaning |
|---|---|---|---|
| `initial C;` | ≤ 1 | child state name | Retarget: entering this composite lands in `C` (recursively). **Required** on any composite that is ever the resolved target of a transition or of an enclosing `initial`. Refused outright on a childless state. |
| `enter (p)? { … }` | ≤ 1 | params + verbatim block | Runs when the transition's enter chain includes this state (see [LCA chains](#lca-exitenter-chains)), and by the startup chain when this state is on the initial leaf's ancestor path. |
| `exit (p)? { … }` | ≤ 1 | params + verbatim block | Runs when the transition's exit chain includes this state. |
| `on E (p)? (if G)? => T (BLOCK\|;)` | any number, one per event type per state | event path, params, guard, root-anchored target, optional action | A transition. Declared on a composite ⇒ inherited by every descendant leaf that does not declare its own handler for the same event. |
| `state N { … }` | any number | nested state | Children non-empty ⇒ this state is a **composite**; empty ⇒ a **leaf**. |

There are no key/value options and therefore no defaults table: everything a machine takes is
positional syntax.

- **Params** use the [`system` param grammar](#parameter-vocabulary) verbatim, with the same
  mutability inference. `enter`/`exit` parens are optional (`enter { … }` is legal) and are only
  consumed when peeked.
- **Guard** is a verbatim `Expr` parsed with `parse_without_eager_brace`; a `let` scrutinee is
  refused.
- **Action** is the `BLOCK` form; `;` means "transition, no action".
- **Target** is root-anchored — resolution starts at the machine's top level for the *first* segment,
  then walks children. There is no relative/sibling form and no `..` form.

### Flattening

| Concept | Rule |
|---|---|
| Leaf | a state with no children |
| Variant name | **concatenation** of the state path: `Playing.Running` → `PlayingRunning` |
| Variant order | preorder over the declared tree, leaves only |
| Generated ident half | `snake()` collapse of the same concatenation: `PlayingRunning` → `playing_running` |

Both mappings are lossy (`A.BC` and `AB.C` both spell `ABC`; `AB` and `Ab` both collapse to `ab`),
and every lossy site is pre-checked so the collision is reported on the user's two states rather than
as a rustc "defined multiple times" on generated tokens. The `snake()` rule itself is specified once,
in [Two snake_case implementations](#two-snake_case-implementations-one-specification).

### Initial chains and handler inheritance

`initial` at the machine level resolves against the **top-level** states, then `resolve_to_leaf`
follows composite `initial` chains downward until it reaches a leaf. Same for a transition target: its
last segment is resolved, then chased to a leaf. A composite reached with no `initial` is a hard
fault.

**Every declared `initial` and every declared transition target is resolved eagerly**, whether or not
any leaf's walk reaches it. This is deliberate: the per-leaf inheritance walk skips handlers an inner
state shadows, and the retargeting walk only visits states something targets, so under lazy
resolution a typo in an unreachable name expanded clean.

**Innermost wins.** For each leaf, `MachineModel::build` walks the ancestor chain innermost-first
(leaf, parent, …, root). The first handler seen for a given event key wins; later (outer) handlers for
the same event are dropped for that leaf. The dedup key is `path_key` — the event path's whole token
spelling with whitespace removed — so `a::E` and `b::E` are *different* events for inheritance even
though they collapse to the same generated fn-name half (which is then its own refusal).

The resulting route list is then **re-sorted by `TransitionDef::decl_index`**, the parser's running
source-order counter across the whole machine body, nested states included. The innermost-first walk
order is what resolves inheritance; declaration order is what determines registration order, and the
two are not the same.

### Emitted items

Emitted in this order, spanned at the machine's own name (each generated fn keeps its leaf's spans):

| Item | Shape | Emitted when |
|---|---|---|
| flat enum | `#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum M { … }` | always |
| marker impl | `impl ::boyko_ecs::ecs::core::state::States for M {}` | always |
| predicates | `impl M { #[inline] pub const fn in_<snake(cat)>(self) -> bool { matches!(self, Self::A \| Self::B) } }` | one per **composite**; the `impl` block is omitted when the machine has no composites |
| initial-enter | `fn __aether_<snake(M)>__initial_enter(merged params) { … }` | only if some state on the initial leaf's ancestor path declares `enter` |
| transition fns | `fn __aether_<snake(M)>__<snake(leaf_cat)>__<snake(last event segment)>(…)` | one per (leaf, inherited event) |

The enum is `pub`; the generated fns are **private** (`fn`, not `pub fn`) — the sibling plugin's
`build` is emitted into the same module and references them there. All generated fns carry
`#[allow(clippy::too_many_arguments)]`. The composite predicate is emitted per composite even when two
composites cover the same leaf set (`in_world` and `in_world_field` are both emitted for a two-level
chain over the same two leaves).

### The transition system

```rust,ignore
#[allow(clippy::too_many_arguments)]
fn __aether_game_flow__playing_running__player_died(
    mut __aether_ev:   ::boyko_ecs::ecs::core::system::EventReader<PlayerDied>,
    mut __aether_next: ::boyko_ecs::ecs::core::system::ResMut<
                           ::boyko_ecs::ecs::core::state::NextState<GameFlow>>,
    score: ::boyko_ecs::ecs::core::system::Res<Score>,     // transition params
    mut cmds: ::boyko_ecs::ecs::core::system::Commands,    // exit/enter handler params
) {
    let mut __aether_fire = false;
    for _ in __aether_ev.read() {
        if !__aether_fire && (score.lives == 0) { __aether_fire = true; }   // guard
    }
    if __aether_fire {
        { /* exit bodies, innermost-first  */ }
        { /* the action block              */ }
        { /* enter bodies, outermost-first */ }
        *__aether_next = ::boyko_ecs::ecs::core::state::NextState::Pending(GameFlow::GameOver);
    }
}
```

Fixed prefix, always: `__aether_ev: EventReader<E>` then `__aether_next: ResMut<NextState<M>>`. Merged
params follow. Without a guard the loop body is bare `__aether_fire = true;`.

**Drain-then-act.** The loop always runs to completion; it only *remembers* that one event was
accepted. The exit/action/enter chain and the `NextState` write happen once, after the drain. This is
§5.1's "one transition per machine per frame — the remainder observed and discarded", and it is
load-bearing rather than stylistic: the kernel's `EventIter` advances the cursor only past events it
*yielded*, so a `return`-in-loop shape would leave the frame's remaining events unread, and the next
frame would re-read them and fire a second transition. Pinned by
`two_same_frame_events_produce_exactly_one_transition`, which reads `2` under the `return` shape.

**Guard evaluation.** `!__aether_fire &&` short-circuits, so a guard is evaluated once per event
*until one passes*, and never against the events being discarded. A failing guard consumes the event
but does not fire the transition and runs no action.

**The event payload is not bound.** The drain is `for _ in __aether_ev.read()` — guards and actions
can read resources, queries and locals, but **not the event's own fields**. There is no grammar for
naming the event value in v1.

### LCA exit/enter chains

For a transition `S_leaf --E--> T_leaf`, the LCA is the longest common prefix of the two root-first
lineages.

| Chain | Nodes | Order |
|---|---|---|
| exit | source lineage strictly below the LCA | innermost-first |
| action | — | between the two |
| enter | target lineage strictly below the LCA | outermost-first |
| `NextState::Pending(T_leaf)` | — | last |

Only states that actually declare the handler contribute a body; the rest are skipped, they do not
emit empty blocks. Consequences that fall out of the rule and are pinned by tests:

- A transition *inside* a composite never re-enters that composite. `Running ⇄ Paused` under `Playing`
  runs `Playing`'s `enter` exactly once across the whole run, not once per hop.
- A superstate's `exit` runs on a transition out of the composite, even though the current state is a
  leaf two levels below it.
- **Self-transition** (`on E => A` from leaf `A`): the lineages are identical, so both chains are
  empty. Only the action block runs, then `NextState::Pending(A)` is written. The kernel's
  `apply_state_transition` treats `requested == current` as an identity no-op — the record stays
  `None`, so no `on_enter` / `on_exit` / `on_transition` condition fires. `run_if(in_state(A))` keeps
  the system enabled, so it re-fires every frame an `E` arrives.
- Actions run **on the frame the transition is accepted**, inside the generated system, under the
  *pre-transition* `State<S>`; the flip happens in the engine's state pass. Aether adds no action
  queue.

### Parameter merging

The transition fn's signature is `merge_params` over, in order: the transition's own params, then each
exit handler's params (innermost-first), then each enter handler's params (outermost-first). The
initial-enter fn merges the `enter` params of the whole ancestor chain, outermost-first.

- Dedup is **by param name**.
- A name reused at a **different emitted type** is a hard fault, with both spans.
- The *first* occurrence is what gets emitted, so the first binding's explicit `mut` spelling wins for
  a name declared twice at one type.
- Because the merge is one flat signature, an **action block may reference a binding declared by an
  `enter`/`exit` handler on the transition's chain** — `a4_machine_hierarchy.rs` relies on this.

Site strings appear verbatim in the diagnostic: `this transition's merged enter/exit/action handlers`,
or ``the initial state's merged `enter` chain``.

### The initial-enter startup chain

`insert_state` seeds the *value*; nothing in the kernel runs an entry action for a state nobody
transitioned into. So the `enter` bodies along the initial leaf's whole ancestor path are emitted as
**one** startup system, outermost-first, with their params merged:

```rust,ignore
fn __aether_sim__initial_enter(
    mut cmds: ::boyko_ecs::ecs::core::system::Commands,
    mut log:  ::boyko_ecs::ecs::core::system::ResMut<Probe>,
) {
    { cmds.spawn(Ground); }   // World's enter
    { log.field += 1; }       // Field's enter
    { log.idle  += 1; }       // Idle's enter (the initial leaf)
}
```

If no state on that path declares `enter`, **neither the fn nor its registration is emitted** — pinned
by `an_enterless_initial_chain_emits_no_startup_system`.

### What the plugin emits

A `machine` **requires** a sibling `plugin` in the same block; the plugin holds every registration.
Order inside the emitted `Plugin::build` body — the general shape is in
[What the plugin collects](#what-the-plugin-collects):

```rust,ignore
impl ::boyko_ecs::Plugin for Flow {
    fn build(&self, app: &mut ::boyko_ecs::App) {
        // 1. per machine, in block order:
        app.insert_state(GameFlow::Boot);                            // the resolved INITIAL LEAF
        app.add_startup_system(__aether_game_flow__initial_enter);   // if the chain has a body
        // 2. sibling `system … on startup` and `scene` spawn fns, in block source order
        // 3. the Main bucket:
        app.add_systems_cfg(|b| {
            // sibling systems' own registrations first, then per machine:
            b.add_system(__aether_game_flow__boot__assets_ready)
                .run_if(::boyko_ecs::ecs::core::schedule::common_conditions::in_state(
                    GameFlow::Boot));
            // … one per (leaf, inherited event) …
        });
        // 4. the Fixed bucket
    }
    fn name(&self) -> &'static str { "Flow" }
}
```

**Registration order** of the transition systems: outer loop over leaves in **variant (preorder)**
order, inner loop over that leaf's routes in **`decl_index` (source) order**. This is §5.1's
deterministic mitigation for the case where several transition systems of the *same* leaf accept
different events on one frame: each writes `NextState`, and the last one wins. Multiple machines in
one block are all registered by that one plugin, in block order; their `insert_state` calls all
precede every startup registration in the body.

### Refusals — `machine`

Parse-time (`parse_machine` / `parse_state`):

| Trigger | Message | Span | Golden |
|---|---|---|---|
| no ident after `machine` | ``expected a machine name after `machine` `` | **failed-parse** | — |
| lowercase machine name | ``machine names are UpperCamelCase — they expand to enums`` (+ rename) | **name** | — |
| body does not open with `initial` | ``a machine opens with `initial <State>;` `` | **failed-parse** / **kw** | — |
| `initial` with no state ident | ``` `initial` names a state ``` | **failed-parse** | — |
| machine-level `initial` not terminated | ``` `initial <State>` ends with `;` ``` | **failed-parse** | — |
| a machine-body item that is not an ident | ``expected `state` (a machine body holds only states after `initial`)`` | **failed-parse** | — |
| a machine-body item ident other than `state` | ``expected `state`, found `foo` (a machine body holds only states after `initial`)`` | **kw** | — |
| no ident after `state` | ``expected a state name after `state` `` | **failed-parse** | — |
| lowercase state name | ``state names are UpperCamelCase — leaves become enum variants`` (+ rename) | **name** | — |
| a state-body item that is not an ident | ``expected `initial`, `enter`, `exit`, `on`, or a nested `state` `` | **stream** | — |
| any other state-item head ident | ``unknown state item `foo`; state items are: initial, enter, exit, on, state`` — exhaustive list, **no** did-you-mean | **name** of the head | — |
| second `initial` in one state | ``duplicate `initial` in this state`` | **kw** of the second | — |
| state-level `initial` with no child name | ``` `initial` names a child state ``` | **failed-parse** | — |
| second `enter` or second `exit` | ``duplicate `enter` in this state`` / ``duplicate `exit` in this state`` | **kw** of the second | — |
| `on` with no event path | ``` `on` takes an event type path ``` | **failed-parse** | — |
| `if` with no guard expression | ``` `if` takes a guard expression ``` | **failed-parse** | — |
| `on E if let …` | ``` `let` bindings are not usable as a transition guard — `if` takes a plain bool expression (bind with a `local<…>` param or match inside the body instead) ``` | the `let` token | — |
| no `=>` after the event / guard | ``a transition points at its target: `on Event => State.Path` `` | **failed-parse** | — |
| `=>` not followed by an ident | ``the transition target is a state path (`Playing.Paused`)`` | **failed-parse** | — |
| a `.` in the target path not followed by an ident | ``the state path continues with a state name after `.` `` | **failed-parse** | — |
| a transition with neither `{ … }` nor `;` | ``a transition ends with an action block or `;` `` | **failed-parse** | — |

Whole-block (`ctx.rs`):

| Trigger | Message | Span | Golden |
|---|---|---|---|
| `machine` with no sibling `plugin` | ``a `machine` needs a `plugin <Name>;` declaration in this block to hold its `insert_state` and transition registrations`` | **name** of the machine | — |

Expansion-time (`MachineModel::build`, `resolve_child`, `resolve_to_leaf`, `resolve_target`,
`merge_params`). Every one of these exists because the fault would otherwise surface as a rustc
duplicate-definition error on tokens the user never wrote. Rows marked *(2 spans)* attach a second
error as a note at the earlier declaration.

| Trigger | Message | Span | Golden |
|---|---|---|---|
| a machine with zero states | ``a machine declares at least one state`` | **name** of the machine | — |
| two **sibling** states of one name *(2 spans)* | ``duplicate state `Idle` — sibling states need distinct names`` + ``the first state flattening to this name is here`` | second **name** / first | ✓ `machine_duplicate_sibling_state` |
| two chart positions flattening to one name *(2 spans)* | ``states `A.BC` and `AB.C` both flatten to `ABC` — flattening concatenates the state path, so they would emit one name; rename one`` + same note | second **name** / first | ✓ `machine_flattened_name_collision` |
| two **composites** collapsing to one `in_*` predicate *(2 spans)* | ``composite states `{a}` and `{b}` flatten to `{ca}` and `{cb}`, which both collapse to the predicate `in_{snake}` — rename one`` + ``the first composite generating this predicate is here`` | second **name** / first | — |
| two `on E` in one state *(2 spans)* | ``duplicate handler for `E` in state `A` `` + ``the first handler is here`` | **kw** of the second `on` / first | ✓ `machine_duplicate_handler` |
| `initial` on a childless state | ``` `Idle` has no nested states, so `initial` has nothing to name — drop it, or nest `state Running { … }` inside `Idle` ``` | **name** of the `initial` target | ✓ `machine_initial_on_a_leaf` |
| a state name that does not exist in the scope being resolved | ``no state `Runing` in `Playing`; states declared here: `Running`, `Paused` (did you mean `Running`?)`` | **name** of the offending ident | ✓ `machine_unknown_initial_did_you_mean`, ✓ `machine_unreferenced_composite_initial`, ✓ `machine_shadowed_handler_target` |
| target is a composite with no `initial` | ``target `Playing` is a composite state with no `initial` — add `initial <leaf>;` or target a leaf (`Playing.Running`)`` | **name** of the last target segment (or the machine `initial` ident) | ✓ `machine_composite_target_without_initial` |
| two leaves minting one fn name *(2 spans)* | ``states `AB` and `Ab` both generate the system `__aether_m__ab__e` — generated names are the snake_case collapse of the flattened state path, and `AB` and `Ab` collapse alike; rename one`` + ``the first handler generating this name is here`` | **kw** of the second `on` / first | ✓ `machine_snake_collapse_collision` |
| two event paths on one leaf whose last segments collapse alike *(2 spans)* | ``events `a::E` and `b::E` both generate the system `__aether_m__a__e` for leaf `A` — the generated name keys on the event's last path segment; import one under an alias (`use … as …`)`` + same note | **kw** of the second `on` / first | — |
| a merged param name reused at a different type *(2 spans)* | ``param `cmds` is declared with conflicting types across this transition's merged enter/exit/action handlers`` (or ``… across the initial state's merged `enter` chain``) + ``the first binding of this name is here`` | later **name** / earlier | — |

The did-you-mean is Levenshtein ≤ 2 against the *declared sibling states of that scope*, and the
"states declared here" list is exhaustive and in declaration order.

**Recovery interacts asymmetrically with these two classes.** A machine that fails to *parse* recovers
normally: one `compile_error!`, a `pub struct M;` name-resolving stub, and every sibling construct
still expands. A machine that parses but fails one of the expansion-time checks above makes
`expand_inner` return `Err`, so **no** construct in the block emits its items — only the
`compile_error!` (plus any stubs).

### Known open semantics (v1.1)

Recorded in the shipped tests, not fixed in code.

- **The `run_if`-gated backlog bounce.** A system that does not run does not advance its `EventReader`
  cursor. If two leaves of one machine transition on the **same event type**, the leaf you just
  entered holds a stale cursor and a leftover event could send it straight back. Measured, the A4
  chart drives `Running ⇄ Paused` on one `PausePressed` without bouncing — the reader window is
  exactly one swap wide, so the driving event has left `reader_buf` by the frame the new leaf first
  runs. **That holds only while events of that type are at least two frames apart.** The v1 mitigation
  is to give opposing edges distinct event types (which is what `a3_machine.rs` does, deliberately).
- **Self-transition is an identity no-op at the kernel boundary.** No exit, no enter, the action runs,
  the `Pending` is discarded by `apply_state_transition`'s identity check, and no state-transition
  condition observes anything. Aether neither refuses the construct nor special-cases it; a chart that
  wants an observable re-entry has no v1 spelling for one.
- **Last-write-wins across event types.** Drain-then-act bounds each *generated system* to one
  transition per frame. Two systems of the *same current leaf* accepting different events on one frame
  both write `NextState`, and the later registration wins. Declaration-order registration is the
  documented determinism story; Aether adds no arbitration in v1.

---
## Content constructs

The two constructs that produce **values**, not types: `material` expands to a builder fn, `scene` to
a spawn fn. Both names are therefore lowercase, both occupy a `fn` item in the block's symbol table,
and both collide with each other and with `system` under the
[duplicate-fn-name rule](#duplicate-fn-names-across-kinds). Narrative:
[Materials](materials.md), [Scenes](scenes.md).

### `material`

```ebnf
material = "material" IDENT "{" ( mat_key ( "," mat_key )* ","? )? "}" ;
mat_key  = "base" ":" color | "emissive" ":" color
         | ("metallic" | "roughness" | "reflectance" | "flags" | "textures") ":" EXPR ;
color    = "(" EXPR "," EXPR "," EXPR ( "," EXPR )? ")" ;
```

Keys are comma-separated; a trailing comma is allowed, a missing separator is refused. Each key may
appear **once**; order is free (the parser fills named slots, not positions).

#### Keys

The accepted set and the set printed by the "expected one of" diagnostics are one table
(`parse.rs::MATERIAL_KEYS`), in `Material::new` parameter order.

| Key | Value shape | Default when absent | Reaches |
|---|---|---|---|
| `base` | `color`, 3 or 4 components | **none — required** | arg 1, `[r, g, b, a]` |
| `metallic` | `EXPR` | `0.0` | arg 2 |
| `roughness` | `EXPR` | `0.5` | arg 3 |
| `reflectance` | `EXPR` | `0.5` | arg 4 |
| `emissive` | `color`, exactly 3 components | `[0.0; 3]` | arg 5, `[r, g, b]` |
| `flags` | `EXPR` | `0` | arg 6 |
| `textures` | `EXPR` (a `MaterialTextures`) | absent ⇒ the untextured constructor | switches the whole emission |

`base` is the one key with no default: §3.6's default table names a value for every other key and
conspicuously omits this one, so Aether refuses rather than inventing a colour.

**Colour production and alpha synthesis.** Components are **verbatim expressions**, not literals —
`metallic: BRASS_METALLIC` and `base: (0.8, 0.8, 0.8, 0.5)` are both legal, and the tokens pass
through with the author's spans.

* `base` with **3** components → `[r, g, b, 1.0]`: the alpha lane is synthesized.
* `base` with **4** components → `[r, g, b, a]`, verbatim.
* `emissive` accepts **exactly 3** → `[r, g, b]`. There is no alpha lane to put a fourth component
  in: `Material::new` takes `emissive: [f32; 3]`.
* Arity is validated at parse, on the **parenthesized group's own span** — neither the key nor any
  single component is the thing that is wrong.

#### What it emits

```rust,ignore
#[doc = " Aether material `gold`."]
#[inline]
pub fn gold() -> ::boyko_render::Material {
    ::boyko_render::Material::new([1.0, 0.72, 0.30, 1.0], 1.0, 0.14, 0.5, [0.0; 3], 0)
}
```

An `#[inline] pub fn` of the material's own name returning `::boyko_render::Material`. Materials are
runtime-minted assets, so the target is a builder over the engine's own constructors — it composes
with any minting site (`materials.add(gold())`), and a static table would be the parallel data system
Principle 0 forbids. A material carries no scheduling: it needs **no `plugin`**, and a sibling
`plugin` does not register it.

**The `textures:` escape.** Present, the emission switches to the engine's only textured constructor,
over an explicit `MaterialGpu::new` carrying the same six scalars:

```rust,ignore
::boyko_render::Material::with_textures(
    ::boyko_render::MaterialGpu::new([0.8, 0.8, 0.8, 0.5], BRASS_METALLIC, 0.3, 0.35, [0.0; 3], 0),
    MaterialTextures { albedo: slot, ..MaterialTextures::NONE }
)
```

`MATERIAL_FLAG_TEXTURED` is derived by `with_textures`; Aether never mints that bit itself.

#### Refusals — `material`

| Trigger | Message | Span | Golden |
|---|---|---|---|
| no ident after `material` | ``expected a material name after `material` `` | **failed-parse** | — |
| uppercase name | ``material names are lowercase — they expand to builder functions, not types (rename `Gold` to `gold`)`` | **name** | ✓ `material_name_is_lowercase` |
| body item is not an ident | ``expected a material key: base, metallic, roughness, reflectance, emissive, flags, textures`` | **stream** | — |
| an ident that is not a key | ``unknown material key `roughnes`; keys are: base, metallic, roughness, reflectance, emissive, flags, textures (did you mean `roughness`?)`` | **key** | ✓ `material_unknown_key` |
| a key not followed by `:` | ``expected `:` after material key `metallic` `` | **key** | — |
| the same key twice | ``duplicate material key `roughness` `` + ``the first `roughness:` is here`` | **key** of the second, and the first | ✓ `material_duplicate_key` |
| two keys with no comma between | ``expected `,` between material keys`` | **stream** | — |
| no `base:` key at all | ``material `m` needs a `base:` color — every other key defaults (metallic 0.0, roughness 0.5, reflectance 0.5, emissive (0.0, 0.0, 0.0), flags 0), the base color does not`` | **name** of the material | — |
| a scalar key with no expression | ``` `metallic` takes an expression ``` (also `roughness`, `reflectance`, `flags`, `textures`) | **failed-parse** | ✓ `recovery_duplicate_name_with_a_broken_twin` |
| `base:` / `emissive:` not followed by `(` | ``` `base` takes a color tuple: `(r, g, b)` or `(r, g, b, a)` ``` / ``` `emissive` takes a color tuple: `(r, g, b)` ``` | **stream** | — |
| a non-expression inside a colour tuple | ``` `base` components are expressions ``` | **failed-parse** | — |
| two components with no comma between | ``expected `,` between `base` components`` | **stream** of the paren body | — |
| `base:` with any arity but 3 or 4 | ``` `base` color takes 3 (rgb, alpha=1.0) or 4 (rgba) components — found 2 ``` | **tuple** | ✓ `material_color_arity` |
| `emissive:` with any arity but 3 | ``` `emissive` color takes exactly 3 components (rgb) — `Material::new` takes `emissive: [f32; 3]`, emitted radiance has no alpha — found 4 ``` | **tuple** | — |
| two materials of one name | ``duplicate material `twice` — each material expands to a builder fn of its own name, and two of one name is one fn defined twice`` + ``the first `material` of this name is here`` | second **name**, and the first | ✓ `material_duplicate_name` |

---

### `scene`

```ebnf
scene      = "scene" IDENT "{" scene_item* "}" ;
scene_item = mesh_let | node ;

mesh_let   = "let" IDENT "=" mesh_src ";" ;
mesh_src   = "plane" "(" EXPR ","? ")" | "cube" "(" EXPR ","? ")"
           | "mesh"  "(" EXPR "," EXPR ","? ")" ;

node       = node_head ( "at" at_pose )? ( "{" node_body "}" )? ";"? ;
node_head  = "mesh" IDENT | "sun" | "spot" | "point" | "sky" | "camera" | "sdf" EXPR | "entity" ;

at_pose    = "(" EXPR "," EXPR "," EXPR ","? ")"     (* the translation sugar *)
           | "(" EXPR ","? ")"                       (* a parenthesised expression *)
           | EXPR ;                                  (* eager — see the swallowed-body trap *)

node_body  = ( prop ( "," prop )* ","? )? ;
prop       = "material" ":" IDENT
           | "children" ":" "[" node ( "," node )* ","? "]"
           | head_key ":" key_value
           | "casts_shadow"
           | EXPR ;                                  (* bare component expression *)
key_value  = "(" EXPR "," EXPR "," EXPR ","? ")"      (* KeyShape::Tuple3 *)
           | EXPR ;                                  (* KeyShape::Scalar *)
```

A `scene_item` is a `let` binding iff the next token is `let`; everything else is parsed as a node.
Scene items are **not** separated, and `let` bindings and nodes may interleave freely. The node
terminator `;` is optional, as are the `at` clause and the body brace.

#### `mesh_let` sources

| Source | Arity | Emits |
|---|---|---|
| `plane(SIZE)` | exactly one expression | `::boyko_render::MeshAssetsExt::plane(&mut *__aether_meshes, __aether_dev.get(), SIZE)` |
| `cube(SIZE)` | exactly one expression | `::boyko_render::MeshAssetsExt::cube(&mut *__aether_meshes, __aether_dev.get(), SIZE)` |
| `mesh(VERTICES, INDICES)` | exactly two (`&[Vertex]`, `&[u32]`); trailing comma allowed | `::boyko_render::MeshAssetsExt::register_mesh(&mut *__aether_meshes, __aether_dev.get(), VERTICES, INDICES)` |

Each binding emits `let NAME = <call>;`, with the author's own binding name. Bindings are
**scene-scoped**, not block-scoped — two scenes have two independent tables — and **scene-wide**, not
statement-scoped: every `let` is hoisted above every node, so a node may name a binding declared below
it. Because of that hoist, interleaving order between bindings and nodes is not observable.

#### Node heads

| Head | Bundle emitted | `at`? | `material:`? | `casts_shadow` ⇒ | Key table |
|---|---|---|---|---|---|
| `mesh IDENT` | `::boyko_render::MeshBundle::new(BINDING, <pose>)` | yes | yes | `::boyko_render::ShadowCaster` | — |
| `sun` | `::boyko_render::DirectionalLightObject { transform, global, light }` | **no** | no | — | `dir`, `color`, `lux` |
| `spot` | `::boyko_render::SpotLightObject { … }` | **no** | no | `::boyko_render::CastsPunctualShadow` | `pos`, `dir`, `color`, `power`, `range`, `inner`, `outer` |
| `point` | `::boyko_render::PointLightObject { … }` | **no** | no | `::boyko_render::CastsPunctualShadow` | `pos`, `color`, `power`, `range` |
| `sky` | `::boyko_render::SkyLight::new(sky, ground)` | **no** | no | — | `sky`, `ground` |
| `camera` | `::boyko_scene::CameraRig { transform, global, camera, projection }` | yes | no | — | `fov`, `aspect`, `near`, `far` |
| `sdf EXPR` | `::boyko_render::SdfPrimitive(EXPR)` | **no** | no | — | — |
| `entity` | `::boyko_scene::SpatialBundle { … }` with `at`; `spawn_empty()` without | yes | yes | `::boyko_render::ShadowCaster` | — |

`entity` is the universal fallback and is deliberately **not poorer** than the sugar heads: it takes
`material:` and `casts_shadow` so a hand-assembled drawable need not spell
`MaterialHandle(h.index() as u16)` itself.

Head-key values are shaped by their table row: `Tuple3` (`(x, y, z)`, exactly three verbatim component
exprs) or `Scalar` (one verbatim expr). A key may appear once per node.

#### Key tables — required vs defaulted

The rule (`ast.rs::NodeKeySpec::required`): a row is defaulted when Aether can *honestly synthesize* a
value; a row whose engine parameter has no neutral value is **required** and refused on the head's own
span rather than invented — the `base:` precedent.

| Head | Key | Shape | Required | Default / why required |
|---|---|---|---|---|
| `sun` | `dir` | tuple3 | **yes** | the light direction; the whole pose derives from it |
| | `color` | tuple3 | no | `[1.0, 1.0, 1.0]` — white is a neutral that *is* right |
| | `lux` | scalar | **yes** | `DirectionalLight::new`'s illuminance has no neutral value |
| `sky` | `sky` | tuple3 | **yes** | neither hemisphere has a neutral value — a black ground and a white ground light a scene differently |
| | `ground` | tuple3 | **yes** | same |
| `point` | `pos` | tuple3 | **yes** | position |
| | `color` | tuple3 | no | `[1.0, 1.0, 1.0]` |
| | `power` | scalar | **yes** | no neutral value |
| | `range` | scalar | **yes** | no neutral value |
| `spot` | `pos` | tuple3 | **yes** | eye of the look-at |
| | `dir` | tuple3 | **yes** | the **shine axis**; the pose is derived from `pos` + `dir` because `light_reconcile` overwrites the seeded direction from the transform's world `-Z` |
| | `color` | tuple3 | no | `[1.0, 1.0, 1.0]` |
| | `power`, `range`, `inner`, `outer` | scalar | **yes** | no neutral value (`inner` / `outer` in degrees) |
| `camera` | `fov` | scalar | no | `60.0`, **degrees** — emitted as `(fov) * (::core::f32::consts::PI / 180.0)`, a multiply rather than `.to_radians()` so the expression stays `f32` |
| | `aspect` | scalar | **yes** | it is the *target's* width/height and no default can be right about it |
| | `near` | scalar | no | `0.1` |
| | `far` | scalar | no | `1000.0` |
| `mesh`, `sdf`, `entity` | — | — | — | `NO_KEYS` |

Slot indices in the expander (`spawn_call`) are positional against these tables. Renaming a key is
free; **reordering** a table moves the matching arm with it.

Derived poses, for the heads that refuse `at`:

* `sun` — `Affine3A::look_at_rh(Vec3::ZERO, dir, +Y)`, rotation via `Quat::from_mat3`, translation
  `Vec3::ZERO`.
* `spot` — `look_at_rh(eye, eye + dir, +Y)` where `eye = Vec3::new(pos[0], pos[1], pos[2])`.
* `point` — `Transform::from_translation(pos)`.
* `sky` — no transform at all (a hemisphere fill).

**The required-key check runs on every node**, including children and one written with no body at all
(`camera;` fails for the missing `aspect:`). The article is chosen by first letter
(``an `aspect:` key``, ``a `dir:` key``), and the optional tail is printed only when the head has
optional keys (`" (these default: …)"`).

#### `at` poses

| Form | Lowers to |
|---|---|
| *(absent)* | `::boyko_scene::Transform::IDENTITY` |
| `at (x, y, z)` | `::boyko_scene::Transform::from_translation(::boyko_math::Vec3::new(x, y, z))` |
| `at (EXPR)` — one component | the expression, verbatim (an ordinary parenthesized expression) |
| `at EXPR` | the expression, verbatim, spans preserved (`at Transform { … }`) |
| `at (a, b)` / any other arity | refused on the tuple |

`at` is accepted only by `mesh`, `camera` and `entity`. On the other five it would be **silently
dropped**, so it is refused instead, each with its own reason (`at_refusal`):

| Head | Message |
|---|---|
| `sun` | ``the `sun` node derives its whole pose from `dir:` (look-at + `Quat::from_mat3`, exactly as the shipped scenes do) — an `at` here would be dropped`` |
| `spot`, `point` | ``the `{kw}` node derives its pose from `pos:` (and `dir:` for the aim) — an `at` here would be dropped`` |
| `sky` | ``the `sky` node is a hemisphere fill with no pose — an `at` here would be dropped`` |
| `sdf` | ``an `sdf` edit carries its WORLD-SPACE position inside the edit itself (v1 reads no `Transform`) — an `at` here would be dropped`` |

`at_refusal` also has a `mesh`/`camera`/`entity` arm (``the `{kw}` node takes `at` ``) that is
unreachable: those three heads pass the `takes_at` gate.

##### The swallowed-body trap

`at` takes its expression eagerly, so a `Transform { … }` pose needs no parentheses — and
consequently `camera at MY_POSE { aspect: 1.5 }` parses `MY_POSE { aspect: 1.5 }` as one **struct
literal**, exactly as it would in a Rust `if` scrutinee. The node body is gone, and the required-key
refusal fires at a reader who is looking straight at `aspect: 1.5`. `swallowed_body_hint` appends the
diagnosis:

```text
error: the `camera` node needs an `aspect:` key — it has no default (these default: fov, near, far)
       — note: the `{ … }` after `MY_POSE` was parsed as a STRUCT LITERAL (`MY_POSE { … }`), not as
       this node's body, so the node has no keys at all; parenthesize the pose to split them:
       `at (MY_POSE) { … }`
```

The hint is gated on the swallowed braces *looking like* this node's body — a field named for the
missing key, or for `material` / `casts_shadow` / `children` / any key of this head — so an honest
`at Transform { translation: … }` whose author merely forgot `aspect:` gets no hint. Pinned by
✓ `scene_at_bare_path_struct_literal`.

`sdf` has the identical hazard (`sdf MY_EDIT { … }`) and gets **no** hint, structurally: the hint
rides on the required-key refusal and `sdf` has `NO_KEYS`, so such a node parses cleanly and there is
no Aether diagnostic to attach anything to. The author gets rustc, on their own tokens, at the struct
literal.

#### Props

A node body is comma-separated props. A prop is keyed iff it reads `ident :` with a **single** colon —
`ident ::` opens a path and `Ident { … }` a struct literal, and both fall through to the component
expression arm untouched.

| Prop | Value | Effect |
|---|---|---|
| `material: IDENT` | a **sibling `material` construct's** name (block-scoped) | `.insert(::boyko_scene::MaterialHandle(__aether_mat_NAME.index() as u16))` |
| `casts_shadow` | bare flag | `.insert(ShadowCaster)` or `.insert(CastsPunctualShadow)`, per the head |
| `children: [ node, … ]` | one or more nodes | `Commands::add_child(parent, child)` per child |
| *bare `EXPR`* | any component expression | `.insert(EXPR)` |

Emission order on one node is fixed and independent of source order: the shadow marker first, then the
`MaterialHandle`, then the bare component exprs **in source order**. The bare-expr arm is the escape
hatch that makes sugar additive: a second camera gets its own draw order with
`camera at (…) { aspect: …, Camera { order: 1, ..Camera::DEFAULT } }`, because the expr is inserted
*after* the bundle and overwrites the field.

**`children:`** — a parent binds an `Entity` local, its children are emitted (recursively, each binding
its own), and `add_child` follows each child's whole subtree. A node binds an id only when someone
needs it — it has children, or it *is* one; every other node keeps the chained statement form.
Hierarchy rides on `ChildOf` insertion; Aether never writes `Children`.

```rust,ignore
let __aether_e0 = __aether_commands.spawn(/* parent */).insert(Root).id();
let __aether_e1 = __aether_commands.spawn_empty().insert(LeftArm).id();
__aether_commands.add_child(__aether_e0, __aether_e1);
```

#### Demand-driven params

The signature is computed from what the body actually uses, in this fixed order:

| Param | Present when | Type |
|---|---|---|
| `__aether_commands` | always | `::boyko_ecs::ecs::core::system::Commands` (`mut`) |
| `__aether_meshes` | the scene has **≥ 1 `let` binding** | `NonSendResMut<Assets<::boyko_render::MeshGpu>>` (`mut`) |
| `__aether_materials` | some node **references** a material | `ResMut<Assets<::boyko_render::Material>>` (`mut`) |
| `__aether_dev` | the scene has **≥ 1 `let` binding** | `NonSendRes<::boyko_app::GpuDevice>` |

A scene with neither compresses to `(commands)` alone — which is why a pure-`entity` scene drags
neither asset table nor the device into its signature, and can run headless. The `__aether_` prefixes
are not cosmetic; see [Span discipline](#span-discipline).

#### The asset-mint hoist

```rust,ignore
let __aether_mat_gold  = __aether_materials.add(gold());
let __aether_mat_chalk = __aether_materials.add(chalk());
```

* **Once per scene fn**, not once per node: one material placed on forty nodes mints **one** asset
  row, and every node inserts a handle to it.
* Only materials actually **referenced** are minted.
* Order is the **block's `material` declaration order**, not first-use order — collecting in use order
  would make the mint sequence a function of node order, so swapping two nodes would silently renumber
  every asset row the scene mints.
* Body layout is always: **every `let`, then every mint, then the nodes**.

#### How a scene registers

A `scene` is a plain `pub fn` with a doc comment:

```rust,ignore
#[doc = " Aether scene `lab` — the spawn fn."]
pub fn lab(/* demand-driven params */) { /* lets, mints, nodes */ }
```

When the block has a `plugin` header, that plugin's `build` emits `app.add_startup_system(<scene>);`
for **every** scene in the block, interleaved with `on startup` systems by block source order. A scene
needs **no** plugin: a plugin-free block emits the spawn fn and leaves registration to the author, the
same contract a clause-free `system` has.

```rust,ignore
// scene empty { }
#[doc = " Aether scene `empty` — the spawn fn."]
pub fn empty(mut __aether_commands: ::boyko_ecs::ecs::core::system::Commands) {}
```

**One suppression.** A scene whose `material:` names a material that is declared in the block but
failed to *parse* is skipped **silently** — no spawn fn, no diagnostic. See
[Silences](#silences--deliberate-non-diagnostics).

#### Refusals — `scene`

Bindings and heads:

| Trigger | Message | Span | Golden |
|---|---|---|---|
| no ident after `scene` | ``expected a scene name after `scene` `` | **failed-parse** | — |
| uppercase name | ``scene names are lowercase — they expand to spawn fns, not types (rename `Lab` to `lab`)`` | **name** | ✓ `scene_name_is_lowercase` |
| no ident after `let` | ``expected a mesh binding name after `let` `` | **failed-parse** | — |
| no `=` after the binding name | ``expected `=` after the mesh binding `floor` `` | **failed-parse** | — |
| source position is not an ident | ``a mesh binding is `plane(SIZE)`, `cube(SIZE)`, or `mesh(&VERTICES, &INDICES)` `` | **failed-parse** | — |
| an ident that is no source | ``unknown mesh source `plain`; sources are: plane, cube, mesh (did you mean `plane`?)`` | **kw** of the source | — |
| `plane`/`cube` with no expression / with a second argument | ``` `plane(…)` takes one size expression ``` / ``` `plane(…)` takes exactly one size expression ``` | **failed-parse** / **stream** of the arg list | — |
| `mesh( … )` missing either expression or the comma / with a third argument | ``` `mesh(…)` takes two expressions: `(&[Vertex], &[u32])` ``` / ``` `mesh(…)` takes exactly two expressions ``` | **failed-parse** / **stream** of the arg list | — |
| binding not terminated | ``a mesh binding ends with `;` (`let floor = …;`)`` | **failed-parse** | — |
| two `let` bindings of one name in one scene | ``duplicate mesh binding `a` in this scene`` + ``the first binding of this name is here`` | second **name**, and the first | — |
| node position holds no ident | ``expected a scene node; heads are: mesh, sun, spot, point, sky, camera, sdf, entity`` | **stream** | — |
| an ident that is no head | ``unknown scene node `sunn`; heads are: mesh, sun, spot, point, sky, camera, sdf, entity (did you mean `sun`?)`` | **head** | — |
| `mesh` with no binding ident | ``` `mesh` names a `let` binding of this scene: `mesh floor` ``` | **failed-parse** | — |
| `sdf` with no expression | ``` `sdf` takes an `SdfEdit` expression ``` | **failed-parse** | — |

`at`, props and keys:

| Trigger | Message | Span | Golden |
|---|---|---|---|
| `at` on a head with no pose slot | see the per-head table above | **kw** (`at`) | — |
| unparenthesized `at` with no expression | ``` `at` takes a `Transform` expression or the `(x, y, z)` translation sugar ``` | **failed-parse** | — |
| a non-expression inside `at ( … )` | ``` `at` components are expressions ``` | **failed-parse** | — |
| two `at` components with no comma | ``expected `,` between `at` components`` | **stream** of the paren body | — |
| `at ( … )` with any arity but 3 or 1 | ``` `at (…)` is the translation sugar and takes 3 components (x, y, z) — found 2; a full pose is written unparenthesized (`at Transform { … }`) ``` | **tuple** | — |
| two props with no comma between | ``expected `,` between node props`` | **stream** of the body | — |
| a bare prop that is not a parseable expression | ``expected a node prop (`material:`, `casts_shadow`, `children:`, a head key) or a component expression`` | **failed-parse** | — |
| `casts_shadow` on `sun`, `sky`, `camera` or `sdf` | ``the `sky` node has no shadow-caster form`` | **flag** | ✓ `scene_casts_shadow_on_sky` |
| second `casts_shadow` | ``duplicate `casts_shadow` `` | **flag** | — |
| `material:` on any head but `mesh` / `entity` | ``the `sky` node has no `material:` form — a head that draws nothing carries no `MaterialHandle` `` | **key** | — |
| `material:` not followed by an ident | ``` `material:` names a sibling `material` construct ``` | **failed-parse** | — |
| second `material:` | ``duplicate `material:` `` | **key** | — |
| second `children:` | ``duplicate `children:` `` | **key** | — |
| `children: []` | ``` `children:` takes at least one node ``` | **key** | — |
| two children with no comma between | ``expected `,` between child nodes`` | **stream** of the bracket list | — |
| a key on a head whose table is empty | ``the `mesh` node takes no keys; props here are: material, casts_shadow, children, or a component expression`` | **key** | — |
| a key not in that head's table | ``unknown `sun` key `dirr`; keys are: dir, color, lux (plus material, casts_shadow, children) (did you mean `dir`?)`` | **key** | — |
| the same head key twice | ``duplicate `sun` key `dir` `` | **key** of the second | — |
| a `Scalar` key with no expression | ``` `sun` key `lux` takes an expression ``` | **failed-parse** | — |
| a `Tuple3` key not followed by `(` | ``` `sun` key `dir` takes a 3-tuple: `(x, y, z)` ``` | **stream** | — |
| a non-expression inside a `Tuple3` | ``` `dir` components are expressions ``` | **failed-parse** | — |
| two tuple components with no comma | ``expected `,` between `dir` components`` | **stream** of the paren body | — |
| a `Tuple3` with any arity but 3 | ``` `sun` key `dir` takes exactly 3 components (x, y, z) — found 2 ``` | **tuple** | — |
| a required key absent | ``the `sun` node needs a `dir:` key — it has no default (these default: color)`` · ``the `camera` node needs an `aspect:` key — it has no default (these default: fov, near, far)`` (+ the struct-literal note when it applies) | **head** | ✓ `scene_at_bare_path_struct_literal` |

Symbol resolution, at expansion time (`unknown_symbol`). Both messages share one builder; the *scope*
clause is spelled by the caller because the two symbol tables have different extents. With an empty
table the parenthetical becomes `no materials are declared here` / `no bindings are declared here`.

| Trigger | Message | Span | Golden |
|---|---|---|---|
| `material: NAME` naming no sibling `material` | ``no material `gol` in this aether block (materials here: `gold`, `lamp`) (did you mean `gold`?)`` | **name** of the reference | ✓ `scene_unknown_material` |
| `mesh NAME` naming no `let` binding of **this** scene | ``no mesh binding `floor` in scene `props` (bindings here: `crate_box`)`` (+ did-you-mean) | **name** of the reference | ✓ `scene_unknown_mesh_binding` |
| the scene's name collides with a sibling fn-producing construct | ``` `lab` is declared twice in this aether block — the `material` and the `scene` both expand to a fn of that name ``` + ``the first `material` of this name is here`` | second **name**, and the first | ✓ `scene_collides_with_a_material_fn` |

---
## Block rules (`AetherCtx`)

`AetherCtx::build` (`ctx.rs`) is the whole-block validation pass. It runs between parse and expand, so
no expander ever re-derives a block-level fact. The rules, in the order they execute:

1. `duplicate_fn_names`
2. the one-plugin rule ([↑](#plugin))
3. the plugin requirement ([↑](#when-a-plugin-is-required))

Each returns on its **first** violation. Whole-block rules do not accumulate against each other: a
block gets at most one ctx error, and independent-fault accumulation happens across *constructs*
([recovery](#recovery)) rather than across rules. `Error::combine` is used only to attach a second span
to a single diagnostic.

Every rule runs over `constructs ∪ broken` — a construct that failed to parse still holds its **name**
and its **kind**. Ordering across the two lists is by `BrokenConstruct::after`, which is what makes
*"the first … is here"* point at the earlier declaration rather than at whichever list happened to
hold it.

`AetherCtx` itself is deliberately narrow — it carries the sibling `material` list (for `scene`'s
`material:` prop) and the names of materials that failed to parse, plus the validation. `system`'s
sibling ordering and `plugin`'s collection each walk `block.constructs` at their own site; a table row
nothing reads is a datum that rots.

> **Plan-vs-source.** §6.2 describes `AetherCtx` as carrying "declared symbols + kinds + spans +
> per-kind payloads". What shipped is the two fields above plus the validation.

### Duplicate fn names, across kinds

Constructs split by the Rust **item kind** their name occupies (`Construct::emits_fn`):

| Half | Constructs | Item emitted | Who owns a duplicate |
|---|---|---|---|
| fn-producing | `system`, `material`, `scene` | a bare `pub fn` | **Aether**, with both spans |
| type-producing | `component`, `tag`, `bundle`, `event`, `machine`, `plugin` | a type carrying a derive | **rustc** (E0428) |

The split is drawn by measurement, not preference. Rung A5 measured real rustc on the fn shape: E0428
over two macro-generated `pub fn`s puts **both** of its labels on the `aether!` token and names no
user token anywhere. A material emits no derive and no trait bound, so unlike `component`×`component`
there is no second, localized error to rescue it. The type half carries a derive, so rustc reports the
duplicate definition *and* a second localized error against the user's own item — and a duplicated
check would only drift.

The rule is one rule over the whole fn half, not three special cases: `scene lab` beside
`material lab` is the same two-fns-one-name fault across kinds. The error lands on the **second**
declaration and combines the first's span. The noun in the same-kind message comes from
`Construct::fn_noun`: `material` → "builder fn", `scene` → "spawn fn", `system` → "fn".

```text
error: duplicate material `twice` — each material expands to a builder fn of its own name, and two of one name is one fn defined twice
  --> tests/ui/material_duplicate_name.rs:12:14
error: the first `material` of this name is here
  --> tests/ui/material_duplicate_name.rs:11:14
```

```text
error: `lab` is declared twice in this aether block — the `material` and the `scene` both expand to a fn of that name
  --> tests/ui/scene_collides_with_a_material_fn.rs:10:11
error: the first `material` of this name is here
 --> tests/ui/scene_collides_with_a_material_fn.rs:8:14
```

### Which collisions Aether owns, and why

Aether pre-checks a downstream fault only where it produces a strictly better span or message.
Everything Aether owns has the same justification: **rustc would report it on generated tokens.**

| Collision | Owner | Reason |
|---|---|---|
| duplicate `system`/`material`/`scene` name (same or across kinds) | Aether, two spans | rustc's E0428 over two generated `pub fn`s labels only the `aether!` token |
| duplicate type name (`component`/`tag`/`bundle`/`event`/`machine`/`plugin`) | rustc | the derive gives a second, localized error on the user's own item; a second check would drift |
| two `plugin`s | Aether, two spans | a whole-block invariant rustc cannot see |
| duplicate `component` hook key | Aether | the derive would too, but Aether owns the better span |
| duplicate `material` key | Aether, two spans | — |
| second `on` clause on a system | Aether | a whole-construct invariant |
| duplicate `let` mesh binding in a scene | Aether, two spans | a second binding silently retargets every `mesh NAME` below it |
| duplicate sibling `state` | Aether, two spans | rustc would see one generated variant |
| duplicate handler for one event in one state | Aether, two spans | one generated fn |
| two chart positions flattening to one name | Aether, two spans | one generated variant |
| two states whose snake_case collapse coincides | Aether, two spans | one generated fn name |
| bundle arity > 16 | Aether | the derive owns the rule; Aether owns the 17th field's span |

The snake-collapse case is the sharpest example of the principle:

```text
error: states `AB` and `Ab` both generate the system `__aether_m__ab__e` — generated names are the
snake_case collapse of the flattened state path, and `AB` and `Ab` collapse alike; rename one
  --> tests/ui/machine_snake_collapse_collision.rs:21:13
error: the first handler generating this name is here
  --> tests/ui/machine_snake_collapse_collision.rs:18:13
```

---

## Recovery

A parse failure does **not** abort the block. The [version header](#the-version-header) is the one
exception in the whole language.

### Speculative parse and resync

Each construct is parsed on a **fork**. On success the real stream advances past it
(`input.advance_to(&fork)`); on failure the real stream is still parked at the construct's own head,
which is what makes the resync well-defined — a failed `ParseStream` parse otherwise leaves the cursor
wherever it stopped.

`skip_to_next_construct` scans for the next `<construct-keyword> <ident>` pair at **depth zero**. The
block grammar is keyword-led and LL(1) at the construct level, so that pair is the one resync point
independent of how far into the broken construct the failure happened. Because the resync looks for
the *successor's head* rather than for a terminator, a `;`-less construct costs its successor nothing:
`tag Player { }` followed by `component Health { … }` yields one error and a fully expanded `Health`.

The scan always consumes at least one token tree. That is the loop's liveness property, and it is
gated: `recovery_terminates_and_never_panics_on_garbage` drives `42`, `component`,
`component; component;`, `system system system`, `tag`, `machine M { state }` and `, , ,` through the
expander and requires each to terminate, emit at least one error, and never panic. A resync that
consumed nothing would *spin* — a hang in a proc-macro, which presents as an unresponsive editor
rather than an error, and is the one failure mode worse than the sea of errors recovery removes.

### One error per fault

```rust,ignore
aether! {
    component Health { hp: f32 }
    component Broken { hp f32 }      // ← the one fault
    tag Player;
    system tick(q: query<&Health>) { let _ = &q; }
}
```

```text
error: expected `:` after field `hp` (or a known item: requires / on_add / on_insert / on_replace / on_remove / no_bundle)
  --> tests/ui/recovery_one_typo_costs_one_error.rs:25:24
```

That is the entire `.stderr`. `Health` (declared before the fault), `Player` and `tick` (declared after
it — which an abort-at-first-error parser never reaches) and `Broken` itself all resolve in `main`. The
contract this golden pins is the **size** of the `.stderr`; a recovery regression shows up here as
extra E0422/E0425 entries.

### The name-carrying stub

`peek_stub` reads the failed construct's name off a fork of the real stream, and `Stub::for_keyword`
chooses the item kind:

| Keyword(s) | Stub | Emitted |
|---|---|---|
| `component`, `tag`, `bundle`, `event`, `machine` | `Stub::Type` | `#[allow(dead_code, non_camel_case_types)] pub struct NAME;` |
| `plugin` | `Stub::Plugin` | the unit struct **plus** `impl ::boyko_ecs::Plugin for NAME` with an empty `build` and a `name()` returning the literal |
| `system`, `material`, `scene` | `Stub::Fn` | `#[allow(dead_code, non_snake_case)] pub fn NAME() {}` |
| anything outside the registry | `None` | nothing — an unknown construct has no known item kind |
| name never parsed | `None` | nothing — a stub needs a name to declare |

Three properties are load-bearing:

* **It is diagnostic-silent.** The `#[allow]`s are there because the stub lives in a file that already
  has one error; a recovery item that added `dead_code` (the author has not written the use yet) or a
  case-convention warning (the case gate is often the very failure being recovered from) would turn one
  error into an error plus a paragraph of noise.
* **The item kind must match `Construct::emits_fn`.** A `material` stubbed as a struct resolves the
  name and then fails at every call site — the cascade recovery exists to prevent, re-introduced by the
  fix for it. `every_registry_keyword_stubs_in_the_item_kind_its_construct_emits` pins the two tables
  equal keyword by keyword.
* **`plugin` needs the trait, not the name.** Every reference to a plugin is `app.add_plugin(P)`. A
  bare `pub struct P;` would trade *"cannot find value `P`"* for *"the trait bound `P: Plugin` is not
  satisfied"* at the same line — a different error, not one fewer.

Stubs are spanned at the user's name and dedupe against **each other** (two broken `material gold`
declarations emit one `pub fn gold`, not two) but **never** against a surviving construct.

**Recorded exemption from the `__aether_` prefix rule:** stubs carry the **user's** name. The prefix
rule exists so generated names cannot collide with the author's; a stub's whole purpose is to occupy
the author's name, so the collision the prefix prevents is possible here and is exactly the diagnostic
that should fire.

### What participates in block rules when broken

* a broken `plugin` still **occupies the plugin slot**, so no sibling clause reports "needs a plugin";
* a broken `material gold` still **occupies the name `gold`**, so a real duplicate is still Aether's
  own two-span diagnostic rather than rustc's E0428 on the macro token.

Reading a broken construct as absent manufactures a second fault out of the first.

```text
// recovery_broken_plugin_keeps_the_block.rs — `plugin Arena` missing its `;`
// one error, and Health / boot / tick / Arena all still resolve in main.
```

```text
// recovery_duplicate_name_with_a_broken_twin.rs — the second `material gold` fails to parse
error: `metallic` takes an expression                                          ← the parse fault
error: duplicate material `gold` — … two of one name is one fn defined twice   ← still Aether's
error: the first `material` of this name is here
// E0428 is absent.
```

A broken construct whose **keyword** is outside the registry (`compnent Health { … }`) has
`keyword: None` and participates in **no** whole-block rule. `BrokenConstruct::keyword` is a
`&'static str` from `CONSTRUCT_KEYWORDS`, not the user's spelling, so a diagnostic printing it cannot
print a typo back at the reader as if it were a construct name.

### Emission order

`expand()` branches: any broken construct routes to `recovered()`, which emits, in order:

1. each failure's `compile_error!` at its own span,
2. each failure's stub (deduped by name),
3. **unconditionally**, the expansion of every construct that parsed — and if a whole-block rule still
   refuses, that error is appended beside the parse errors.

Part 3 running unconditionally is the correction of a shipped defect: the earlier shape dropped the
whole expansion whenever the survivors failed a rule. It read as conservative and was the opposite — a
half-typed `plugin ;`, the ordinary mid-edit state of every block that has one, erased every sibling
item and re-created the unresolved-name sea the mechanism was built to prevent.

---

## Tokens, not dependencies

`aether_lang` depends on `syn`, `quote` and `proc-macro2` — **nothing from the engine**. Every
`::boyko_*` path in the expander is an emitted **token**, resolved in the downstream crate.

The consequence is a hard constraint: **the emitted path must be the one that actually resolves in the
user's crate**, not the plan's idealized spelling. `boyko_ecs` re-exports only a handful of items at
its root, so the plan's `::boyko_ecs::Res` is emitted as `::boyko_ecs::ecs::core::system::Res`.

| Root-level (as written) | Nested (the real path) |
|---|---|
| `::boyko_ecs::App`, `::boyko_ecs::Plugin` | `::boyko_ecs::ecs::core::system::{Commands, Res, ResMut, NonSendRes, NonSendResMut, Local, EventReader, EventWriter}` |
| `::boyko_macros::{Component, Bundle, event}` | `::boyko_ecs::ecs::core::entity::entity::Entity` |
| `::boyko_render::{Material, MeshBundle, MeshGpu, …}` | `::boyko_ecs::ecs::core::iters::query::{Query, With, Without}` |
| `::boyko_scene::{Transform, GlobalTransform, …}` | `::boyko_ecs::ecs::core::state::{States, NextState}` |
| `::boyko_math::{Vec3, Quat, Affine3A}` | `::boyko_ecs::ecs::core::schedule::common_conditions::in_state` |
| `::boyko_app::GpuDevice` | `::boyko_ecs::ecs::core::asset::Assets`, `::boyko_ecs::ecs::core::app::CoreSchedule::Fixed` |

Engine types are named by their **defining** crate (`::boyko_scene::Transform`, `::boyko_math::Vec3` —
`boyko_render` re-exports neither). Trait methods are emitted trait-qualified
(`::boyko_render::MeshAssetsExt::cube(…)`, not `meshes.cube(…)`), because the method form would require
the user to have imported the trait into the module the macro expands into.

**What the token contract costs:** no assertion inside `aether_lang` can notice that an engine
constructor grew a parameter — the unit tests pin what Aether *meant to emit*, and would stay green
forever. The engine half of the gate is `aether_tests`' compiled surface; see
[R4 — the anti-drift gate is the dependency list](#r4--the-anti-drift-gate-is-the-dependency-list).

---

## Span discipline

Four rules, as shipped.

**(1) Verbatim, never stringified.** Field types, hook paths, condition expressions, system bodies and
scene expressions are carried as parsed `syn` nodes and re-emitted unchanged. A `stringify!` +
re-parse round trip loses spans, and every rule below depends on them.

**(2) Synthesized tokens** are emitted through plain `quote!`, i.e. at `Span::call_site()`.

**(3) Items that exist *because of* a user name are spanned at that name.** The complete list of
`quote_spanned!` sites in `expand.rs` is eight: the three recovery stubs, `component` → derive struct,
`tag` → ZST component, `bundle` → derive struct, `event` → `#[::boyko_macros::event]` struct, and
`machine` — whose flat enum, `impl States`, predicates and initial-enter chain are all emitted from
that one site.

Measured at rung A7: with a plain `quote!` on the `component` emission, rustc's *"previous definition
of the type `Foo` here"* pointed at `aether! {`. The fix is why
`recovery_duplicate_type_name_with_a_broken_twin` can defer to rustc at all and still get **both**
labels on user tokens.

The fn-producing emissions (`system_fn`, `material_fn`, `scene_fn`) and `plugin_impl` use plain
`quote!`; the name ident is interpolated and carries its own span. For those, Aether owns the duplicate
diagnostic itself, so no rustc label needs to find the user's declaration — see
[Stated limits](#stated-limits) for the one place that reasoning is a reading rather than a recorded
rationale.

**(4) Generated internal names are `__aether_`-prefixed.** `__aether_commands`, `__aether_meshes`,
`__aether_materials`, `__aether_dev`, `__aether_mat_<name>`, `__aether_e<N>`, `__aether_k_<system>`,
`__aether_ev`, `__aether_next`, `__aether_fire`, `__aether_<machine>__<leaf>__<event>`. **A name
without that prefix in an error message is one you wrote.**

The prefix is not cosmetic. Measured: `let dev = plane(1.0); mesh dev;` in a scene shadowed the device
param and produced E0599 (`no method get on MeshHandle`) with **both** labels on the whole `aether!`
token — the exact user-token-free shape that justifies Aether owning a diagnostic. Prefixing makes the
collision unrepresentable instead of diagnosable, and a scene may now bind all four of
`commands` / `meshes` / `materials` / `dev` itself. Param *names* are invisible to registration (only
types and order are), so `add_startup_system` is unaffected. The four scene param roles are spelled
once each as constants (`SCENE_PARAM_COMMANDS`, `…_MESHES`, `…_MATERIALS`, `…_DEV`) because signature
and body must agree and a typo in either would surface only as an unresolved name inside macro output.

**Never panic.** `expand_block` returns `compile_error!` for every failure and panics for none. A
panicking proc-macro erases the block from analysis entirely, which is strictly worse than any
diagnostic. The one message that user input cannot reach is the
[internal-error escape](#the-internal-error-escape).

The mechanical enforcement of all four rules is [the span sweep](#the-span-sweep).

---
## Diagnostics

Every Aether error is a `syn::Error` minted through one constructor, `diag::err(span, msg)`, and
delivered as a `compile_error!` carrying that span. The catalogue is distributed: each construct's
refusals live with the construct ([component](#refusals--component), [tag](#refusals--tag),
[bundle](#refusals--bundle), [event](#refusals--event), [system](#refusals--system),
[plugin](#refusals--plugin), [machine](#refusals--machine), [material](#refusals--material),
[scene](#refusals--scene)), plus the [version header](#the-version-header), the
[construct registry](#construct-keyword-registry), the [case gates](#naming-and-case-gates), the
[block rules](#block-rules-aetherctx) and [sibling ordering](#sibling-ordering). This section holds
what is common to all of them. Narrative: [Diagnostics](diagnostics.md).

Source counts: `parse.rs` has 133 `diag::err` sites, `expand.rs` 21, `ctx.rs` 6, against 37 `.stderr`
goldens in `crates/aether_tests/tests/ui/`.

### How to read a span

| Shorthand | What it means |
|---|---|
| **name** / **key** / **kw** / **flag** | that ident's own span (`ident.span()`) |
| **failed-parse** | syn's span for the `parse::<T>()` that failed — the token the parse stopped on. For a construct-terminating `;` this is the **first token of the next construct** |
| **stream** | `ParseStream::span()` — the next token in that buffer, or the enclosing group's delimiter when the buffer is exhausted |
| **tuple** | the parenthesis group's joined span (`paren.span.join()`) |
| **head** | `SceneNode::head_span`, the node's head keyword ident |

`recovery_broken_plugin_keeps_the_block.stderr` is the clearest published example of the
**failed-parse** rule: the fixture's fault is on the `plugin Arena` line, and the caret lands on the
`system` keyword two lines below it — that is where the `;` was expected.

### Did-you-mean

`diag::did_you_mean(found, candidates)` returns the nearest candidate within **Levenshtein distance
≤ 2**, comparing with a strict `<`, so ties go to the **first candidate in table order**. Below the
threshold nothing is appended and the exhaustive list stands alone. The suffix is always
`` (did you mean `x`?) ``.

| Site | Candidate set | Source |
|---|---|---|
| unknown construct head | `component, tag, bundle, system, event, plugin, machine, material, scene` | `diag::CONSTRUCT_KEYWORDS` |
| unknown syntax version | `v1` | `SyntaxVersion::spellings()` |
| unknown system clause | `on, in, before, after, when` | `parse::CLAUSE_KEYWORDS` |
| unknown query filter | `with, without, added, changed, enabled, disabled` | `parse::FILTER_KEYWORDS` |
| unknown material key | the seven `MATERIAL_KEYS` names | `parse::MATERIAL_KEYS` |
| unknown mesh source | `plane, cube, mesh` | inline literal in `parse_mesh_let` |
| unknown scene node head | `mesh, sun, spot, point, sky, camera, sdf, entity` | `ast::NODE_HEADS` |
| unknown head key | that head's own key table (empty table ⇒ no suggestion) | `NodeHead::keys()` |
| `before`/`after` naming a non-sibling | the block's sibling `system` names | `expand::resolve_order` |
| unknown state name | the states declared under that parent | `expand::resolve_child` |
| unknown `material:` / `mesh` binding | the declared materials / that scene's `let` bindings | `expand::unknown_symbol` |

Sites that list their alternatives but deliberately offer **no** suggestion: the unknown `on`
schedule, the unknown state item, the unknown tag modifier, and the component item head (which falls
through to the field parse and reports the missing `:` instead).

Two consequences worth knowing:

* For `MATERIAL_KEYS`, `SYNTAX_VERSIONS` and the node key tables, the candidate list a message prints
  and the table the parser dispatches on are **the same rows** (a `(&str, Enum)` pair table), so a key
  cannot be advertised and rejected, or accepted and unnamed. `CONSTRUCT_KEYWORDS`, `CLAUSE_KEYWORDS`,
  `FILTER_KEYWORDS` and `NODE_HEADS` are separate `&[&str]` lists beside their `match` arms;
  [`the_registry_list_covers_every_dispatched_keyword`](#diagnostic-registry-coverage) is the guard
  against drift.
* On a head with an empty key table (`mesh`, `sdf`, `entity` → `NO_KEYS`), the did-you-mean call runs
  against an empty candidate set and can never suggest anything.

The three did-you-mean sites in `expand.rs` are *name resolution*, not grammar: sibling-system
ordering targets, machine state names, and a scene's `material:` / `mesh` binding references.

### Two-span diagnostics

These combine a second `syn::Error`, so rustc prints two separate `error:` entries. The primary always
lands on the **second** (later) occurrence.

| Diagnostic | Second-span note text |
|---|---|
| duplicate `plugin` | ``the first `plugin` is here`` |
| duplicate fn-producing name | ``the first `{keyword}` of this name is here`` |
| duplicate material key | ``the first `{key}:` is here`` |
| duplicate mesh binding | ``the first binding of this name is here`` |
| duplicate / flatten-colliding state | ``the first state flattening to this name is here`` |
| composite predicate collision | ``the first composite generating this predicate is here`` |
| duplicate machine handler | ``the first handler is here`` |
| minted transition-fn collision | ``the first handler generating this name is here`` |
| merged-param type conflict | ``the first binding of this name is here`` |
| system ordering cycle | ``…cycle member``, once per additional member |

### The comma rule

``expected `,` between …`` is one mechanical family: after a successful item a `,` is eaten if present;
if absent and the buffer is not empty, this fires at the **stream** span. Sites and exact wording:

`component items` · `bundle fields` · `event fields` · `params` · `material keys` ·
`` `{key}` components `` (material colours) · `` `at` components `` · `node props` · `child nodes` ·
`` `{key}` components `` (node `Tuple3` keys)

Trailing commas are always permitted — see [Separators and terminators](#separators-and-terminators).

### Silences — deliberate non-diagnostics

Each is a rule whose failure could not exist without a parse break, so reporting it would manufacture
a second fault from the first. Each is suppressed at its own site rather than centrally, and all four
come back the moment the broken construct parses.

| Situation | What happens instead |
|---|---|
| a `before`/`after` naming a sibling `system` that did not parse | the ordering edge is dropped (`ResolvedOrder::Suppressed`); the system still registers. The fn item would otherwise be handed to `after_set`, which takes a `SystemSet` type |
| a `scene` naming a `material` that did not parse | the whole `scene` is skipped silently — ``no material `gold` `` would contradict the source, since `gold` is declared right there |
| a `plugin` that did not parse | it still occupies the plugin slot, so no sibling clause reports "needs a plugin" |
| a broken construct's name | it still participates in the duplicate-name rule by name and kind |

### The internal-error escape

One message is not reachable from user input and says so:

> internal aether error: the `{head}` node's required `{key}:` {shape} slot was not filled by the parser — please report this block

Span: **head**. `missing_slot` exists so an expander/parser table disagreement (a `*_KEYS` row reordered
without moving the matching arm in `spawn_call`) becomes a spanned `compile_error!` rather than a macro
panic, which would erase the whole block from rust-analyzer's view.

### Golden coverage

37 fixture/`.stderr` pairs, registered across `a0`/`a2`/`a3`/`a4`/`a5`/`a6`/`a7_diagnostics.rs`. They
pin roughly 33 distinct Aether messages (a few carry more than one fixture) plus one rustc `E0428` that
Aether deliberately does not own.

| Suite | Fixtures |
|---|---|
| `a0_diagnostics.rs` | `unknown_construct`, `lowercase_component`, `duplicate_hook`, `bad_tag_modifier`, `tag_missing_semicolon`, `bundle_arity_cap`, `participant_without_context` |
| `a2_diagnostics.rs` | `query_takes_angle_brackets`, `clauses_need_a_plugin`, `duplicate_on_schedule` |
| `a3_diagnostics.rs` | `machine_composite_target_without_initial`, `machine_duplicate_handler`, `machine_unknown_initial_did_you_mean` |
| `a4_diagnostics.rs` | `machine_flattened_name_collision`, `machine_duplicate_sibling_state`, `machine_snake_collapse_collision`, `machine_initial_on_a_leaf`, `machine_unreferenced_composite_initial`, `machine_shadowed_handler_target` |
| `a5_diagnostics.rs` | `material_color_arity`, `material_name_is_lowercase`, `material_unknown_key`, `material_duplicate_name` |
| `a6_diagnostics.rs` | `scene_unknown_material`, `scene_unknown_mesh_binding`, `scene_casts_shadow_on_sky`, `scene_name_is_lowercase`, `scene_collides_with_a_material_fn`, `no_planned_construct_remains` |
| `a7_diagnostics.rs` | `version_header_unknown`, `version_header_out_of_place`, `scene_at_bare_path_struct_literal`, `material_duplicate_key`, `recovery_one_typo_costs_one_error`, `recovery_broken_plugin_keeps_the_block`, `recovery_duplicate_name_with_a_broken_twin`, `recovery_duplicate_type_name_with_a_broken_twin` |

`a1`'s two goldens live in `a0_diagnostics.rs` — **there is no `a1_diagnostics.rs`**.

Everything else in the catalogue is pinned only by **text**, through `fails_with(…)` assertions in
`expand.rs`'s `mod tests`, or is not pinned at all. A message in that class can move its caret to
`Span::call_site()` without any test going red.

---

## Gates

Three lanes, because the three crates can prove three different classes of claim:

| Lane | Crate | Can prove | Cannot prove |
|---|---|---|---|
| **unit** | `aether_lang` (`#[cfg(test)] mod tests` in `expand.rs`, `diag.rs`) | the exact tokens `expand_block` emits; the exact TEXT of every diagnostic | anything about spans, and anything about the engine |
| **golden** | `aether_tests/tests/ui/*.rs` + `.stderr`, driven by `trybuild` | that an error surfaces THROUGH rustc, at the line and column of the user's own tokens, in a real downstream crate | anything about runtime behaviour |
| **behaviour / anti-drift** | `aether_tests/tests/a*_*.rs` (non-diagnostic targets) | that emitted tokens name real engine items with real signatures, and that the result runs on a real `App` | nothing about diagnostics |

`crates/aether_tests/src/lib.rs` is a deliberately **empty** library — the crate exists only to give
the integration tests a Cargo home whose dependency set includes the engine, while `aether_lang` and
`aether` stay engine-free.

### The unit lane

MEASURED 2026-08-21: `cargo test -p aether-lang --lib` → **56 passed** in 0.01 s (54 in
`expand.rs::tests`, 2 in `diag.rs::tests`). No unit tests exist in `parse.rs`, `ast.rs`, `ctx.rs` or
`lib.rs` — `parse.rs` carries a `#[cfg(test)]` item, but it is `pub(crate) fn snake_case_for_tests`, an
export for the parity check, not a test.

Three assertion helpers, and which one a test uses tells you what kind of claim it makes:

| Helper | Assertion |
|---|---|
| `expands_to(input, expected)` | `expand_block(input).to_string() == expected.to_string()` — **token-for-token**. `TokenStream::to_string` is canonical for identical streams, so a string compare *is* token equality |
| `fails_with(input, needle)` | output contains `compile_error` **and** contains `needle` — the message TEXT only, never the span |
| `emits_in_order(input, first, second)` | `out.find(first) < out.find(second)` — for ORDER claims, where a full token pin would bury the one claim being made |

**Why not `macrotest`.** The plan's rung table named `macrotest` snapshots. It was not taken: a new
third-party dev-dependency loses to the no-new-3rd-party rule, and `expand_block` is a plain function,
so unit tests pin its output token-for-token — *strictly more precise* than macrotest's
rustfmt-normalized snapshots.

### The behaviour lane

16 integration targets, **20 test fns**. MEASURED 2026-08-21
(`cargo test -p aether-tests --all-targets --no-fail-fast`, windows-gnu, rustc 1.97.1): all green.

| Target | Tests | Pins | Wall |
|---|---|---|---|
| `a0_component_tag.rs` | 1 | components/tags are REAL engine components: `create_archetype` + `spawn_two`, `get_component` reads the values back, the `on_add = …` hook fires **exactly once**, and a `tag T(bitset)` **refuses to spawn into an archetype** and toggles via `enable`/`is_enabled` | 0.00 s |
| `a1_bundle_event.rs` | 1 | a `bundle` spawns through `Commands::spawn`; an `event` rides the real kernel lanes, `EventWriter`→`EventReader`, across two `app.update()`s; the two-band construction | 0.01 s |
| `a2_system_plugin.rs` | 1 | a `plugin` registers real systems; startup one-shot spawn visible to frame-1 queries; the `after` edge holds every frame; `when` holds the gated system shut; `query<(&mut …)>` actually writes | 0.00 s |
| `a3_machine.rs` | 1 | `insert_state`, `run_if(in_state(leaf))`, `NextState`, composite retargeting through `initial`, LCA-inlined `enter` — asserted at the **mid** state as well as the end state | 0.01 s |
| `a4_machine_hierarchy.rs` | 3 | the §3.5 chart end-to-end (inherited superstate handler, per-event guard, `exit` two levels above the current leaf); the initial-enter chain (three ancestors once, then LCA bound on re-entry); two same-frame events → **exactly one** transition | 0.01 s |
| `a5_material.rs` | 1 | `Assets<Material>::add` on a startup system, handles resolved back, **every lane of the 48-byte `MaterialGpu`** including the whole `flags` lane; `MATERIAL_FLAG_TEXTURED` set only for the `textures:` material | 0.00 s |
| `a6_scene.rs` | 2 | the material seam end to end (handle → `MaterialHandle(u16)` → asset row → base colour); the hoist observable (two props sharing a material have the **same** handle); `children:` asserted on `Children`, not `ChildOf`, so the kernel's reactive half is proven; light heads' values in the right slots | 0.01 s |
| `a7_dx.rs` | 1 | the `aether v1;` header on a real block whose systems must still register and run; **and** the clippy arity gate | 0.00 s |
| `demo_arena.rs` | 1 | every v1 construct in one block; a specimen that is also a build gate | 0.00 s |
| `a0/a2/a3/a4/a5/a6/a7_diagnostics.rs` | 1,1,1,1,1,1,**2** | the trybuild corpus + [the span sweep](#the-span-sweep) | cache-state-dependent: trybuild rebuilds a scratch crate, so the first suite pays the cold build (tens of seconds) and the rest ride it (~1 s each); the split moves between runs and is not a pinned number |

`a0_diagnostics` pays for the whole shared trybuild scratch build; the other six ride it.

#### R4 — the anti-drift gate is the dependency list

`aether_tests/Cargo.toml` is the gate. `aether_lang`'s token pins have **no engine dependency and
therefore cannot notice an engine change at all**. Five engine dev-deps make Aether's emitted paths
real compilations, in-repo, the same day:

| Dep | Added at | What it makes checkable |
|---|---|---|
| `boyko-ecs`, `boyko-macros` | A0 | the derive surface, `App`, `Plugin`, state, events |
| `boyko-render` | A5 | `Material::new` / `with_textures`, `MaterialGpu` — "the day `Material::new` gains a parameter, the A5 tests stop compiling here […] instead of in a user's game" |
| `boyko-app`, `boyko-scene`, `boyko-math` | A6 | `Transform`, `Vec3`, `GpuDevice` — a `scene` names types from four crates because that is where the engine defines them |

**Blast radius, recorded in the manifest:** this pulls `boyko_render → boyko_rhi_vulkan → boyko_rhi`
and the `boyko_app` host layer behind the Aether gate. `cargo test -p aether-tests` goes red when THAT
lane is red, for reasons that have nothing to do with the DSL, and a cold `cargo check -p aether-tests`
went from ~1 s to **~31.7 s** (MEASURED). Nothing opens a window or touches a device: `a6_scene.rs`'s
`vb_lab` module is a **registration** gate — `add_startup_system` requires `IntoSystem`, which
type-checks the whole generated body — and the app is never updated. The `annex` scene exists purely
for coverage-by-construction ("an emission path absent from this file is a path NO compiler ever
sees"): `spot` (`SpotLight::new`'s seven arguments), `camera` (`CameraRig` +
`Projection::Perspective`), `mesh(&V, &I)` (`register_mesh`), and `casts_shadow` on a punctual light.

### The trybuild golden corpus

**37 fixtures, 37 `.stderr`, 37 registrations** (verified 2026-08-21). Registration is by explicit
`t.compile_fail("tests/ui/<name>.rs")` — never a glob. The suite breakdown is in
[Golden coverage](#golden-coverage).

Two fixtures are goldens whose **contract is the SIZE of the `.stderr`**, not its wording:
`recovery_one_typo_costs_one_error` (one fault, one error, four names still resolving in `fn main`)
and `recovery_broken_plugin_keeps_the_block` (the same contract at the whole-block rules). Two pin
**rustc's own** output rather than Aether's — `recovery_duplicate_type_name_with_a_broken_twin.stderr`
carries a full `error[E0428]` with its `= note:` line — and those are the toolchain-coupled ones.

#### Blessing procedure

```powershell
$env:TRYBUILD = "overwrite"
cargo test -p aether-tests --test a5_diagnostics    # the ONE suite you changed
$env:TRYBUILD = ""
cargo test -p aether-tests --test a5_diagnostics    # verifying re-run, no overwrite
```

Then **read every `.stderr` you touched**, then re-run. The suites state the discipline:

> "a `.stderr` is re-blessed ONLY after verifying the error KIND is unchanged — the
> `token_use_after_submit_rejected` lesson (87 commits red because a line moved and nobody
> re-blessed)."

and `a7_diagnostics.rs` tightens it: "the error KIND **and the caret's position** are what the case is
about." (The command form above is the repo standard, from `boyko_ecs`'s compile-fail headers; no file
in the aether crates spells it — see [Stated limits](#stated-limits).)

#### Blessing hazards

1. **The UTF-8 trap.** Fixture sources carry `§` and em-dashes in their comments — 32 of the 37 `.rs` files
   contain non-ASCII bytes (26 carry `§` specifically; only five are pure ASCII). A tool that writes them in the Windows ANSI codepage (Python's
   `open(path, "w")`; PowerShell's `Set-Content` without `-Encoding utf8`) makes the fixture **invalid
   UTF-8**, rustc reports an encoding error, and `TRYBUILD=overwrite` **blesses the encoding error as
   the golden**. It is green afterwards, and it pins nothing. Always write fixtures as UTF-8; always
   read the `.stderr` back.
2. **Bless, then READ.** Overwriting is not verification. A golden recording whatever the compiler said
   the day nobody looked is invisible to a green run by construction.
3. **A fixture whose input stopped being a fault passes for the wrong reason.** Real precedent:
   `machine_snake_collapse_collision` shipped with the pair `AB`/`A_b`, and A7's snake_case change
   (`GOLD` → `gold` instead of `g_o_l_d`) stopped them colliding. The fixture's **input was re-aimed**
   at a pair the current rule collapses (`AB`/`Ab`) — the golden was NOT re-blessed. Re-blessing would
   have produced a passing compile-fail test that tests nothing.
4. **`wip/` is where a mismatch lands.** On a mismatch trybuild writes the actual output to
   `crates/aether_tests/wip/<name>.stderr`. That directory is entirely untracked (its own `.gitignore`
   is `*`) and currently holds six stale files from rung A4. Diff `wip/x.stderr` against
   `tests/ui/x.stderr` before deciding anything; a file's presence there means nothing about the
   current run.
5. **Platform/toolchain.** Local dev is windows-gnu / rustc 1.97.1; CI is `ubuntu-latest` +
   `dtolnay/rust-toolchain@stable`. Only the two rustc-owned goldens are exposed to wording drift, but
   a toolchain bump is the standard reason for a whole-corpus re-bless, and that is exactly when
   hazards 1–3 bite hardest.
6. **The aether trybuild suites are NOT `#[cfg(not(miri))]`-gated**, unlike `boyko_ecs`'s.

### The mechanical gates

Five checks a human cannot forget to run, because they are tests.

#### Expansion-volume band

`expand.rs::tests::expansion_volume_stays_inside_its_measured_band` counts tokens in and out of
`expand_block` over the pinned §3.x before/after corpus — tokens, not lines, "because a token count is
what the two crates actually exchange and is invariant under formatting". Groups are counted
recursively.

**The band is two-sided on purpose**: "a ceiling alone is satisfied by emitting NOTHING, and this repo
has shipped that exact failure — a gate whose green state includes the empty one."

| Corpus | In | Out | Ratio | Band |
|---|---|---|---|---|
| component+tag (§3.1) | 26 | 70 | 2.69 | 63..=77 |
| system+plugin (§3.3) | 74 | 239 | 3.23 | 215..=263 |
| machine (§3.5) | 59 | 624 | 10.58 | 560..=690 |
| material (§3.6) | 19 | 52 | 2.74 | 46..=58 |
| scene (§3.7) | 55 | 493 | 8.96 | 443..=543 |

Re-measured live 2026-08-21 (`cargo test -p aether-lang --lib expansion_volume -- --nocapture`):
identical to the A7 numbers. The test `println!`s the measurement on every run, so a passing band still
tells a reader which way the number is drifting. Bands are the measured count ±10% **rounded outward**
— a band rounded inward excludes counts the stated tolerance admits, so the number and the rule it
claims to follow would disagree.

The two double-digit ratios are the constructs that *transpile* rather than sugar (one system per
(leaf, inherited event); one spawn statement per node). The sugar constructs sit near 3× — that is
Decision A3's claim expressed as a number. **If you change an emission deliberately: re-measure and
move the band in the same commit.**

#### The span sweep

`a7_diagnostics.rs::every_ui_golden_is_registered_and_pins_a_span_off_the_macro_token` is a mechanical
sweep over the whole corpus, asserting three properties, each of which has failed silently somewhere in
this repo's history:

1. **Every `tests/ui/*.rs` is registered** in some `a*_diagnostics.rs` `compile_fail` list. An
   unregistered fixture cannot go red, so its `.stderr` records whatever the compiler said the day it
   was written, forever.
2. **Every fixture's `.stderr` pins at least one `line:column`** of the form
   `--> tests/ui/<name>.rs:L:C`.
3. **No label sits on the `aether! {` line — primary OR secondary.** That is the signature of span
   degradation: a diagnostic that lost the user's span falls back to the macro token, "which is
   technically a location and practically useless in a forty-line block."

Property 3's secondary half is not decoration. As first written the sweep read only `-->` lines, and the
very next golden added — `recovery_duplicate_type_name_with_a_broken_twin` — carried
``17 | aether! {  | ------- previous definition of the type `Foo` here``. The cause was real
(type-producing items emitted with `quote!`, i.e. at `Span::call_site()`); the fix was in `expand.rs`.
The sweep now parses rustc's gutter (`NN | <source>`, tolerating the `/` rail of multi-line spans) so
**every** label is checked.

Constraints this imposes on a new fixture:

* it must contain a line that trims to exactly `aether! {` — otherwise the sweep panics with "every
  fixture opens its block on a line of its own" (with more than one block, the sweep takes the **first**
  such line);
* it must be registered in a file whose name ends `_diagnostics.rs` — registering it from, say,
  `a7_dx.rs` does not count;
* both floors are `>= 30` (`registered.len()`, `checked`), currently 37, so a truncated suite list or an
  emptied `ui/` directory cannot pass.

#### Diagnostic-registry coverage

Two tests hold `diag::CONSTRUCT_KEYWORDS` against the two other tables it must agree with, one per
drift direction:

| Test | Direction caught |
|---|---|
| `diag.rs::the_registry_list_covers_every_dispatched_keyword` | a keyword the **parser dispatches** but the list omits — `pluging P;` would then report unknown-construct with no did-you-mean and a supported-list that misstates the surface. Asserts `plugin`, `material`, `scene` near-misses all suggest. Also names the reverse: a keyword the list advertises and the parser does NOT route would advertise a construct that errors as unknown |
| `expand.rs::every_registry_keyword_stubs_in_the_item_kind_its_construct_emits` | a keyword whose recovery stub is the wrong item kind. The stub table is keyed on the keyword (the recovery path has no parsed construct to ask), so it is a *second* table; the test walks all nine and asserts `system`/`material`/`scene` → `Stub::Fn`, `plugin` → `Stub::Plugin`, everything else → `Stub::Type`, **and** that `stub.emits_fn()` agrees with the same predicate — "or the duplicate rule draws its line at two different places on the two paths". Plus `"shader"` → `None` |

This is the mechanical form of DX-checklist item 6, "one table, spelling and dispatch together".
`MATERIAL_KEYS` gets the same treatment structurally (one `(str, MatKey)` table) and behaviourally by
`every_advertised_key_reaches_the_emission`, which sets **all seven** keys at once with every value
non-default — it catches a key the parser accepts but never threads into the emission.

#### snake_case parity

`both_snake_case_implementations_agree_on_the_same_rule` pins `expand::snake` and `parse::snake_case`
equal on the ten cases that distinguish the current rule from the letter-by-letter one it replaced:
`GOLD→gold`, `Gold→gold`, `GameFlow→game_flow`, `UIState→ui_state`, `HTTPProbe→http_probe`,
`PlayingRunning→playing_running`, `A_b→a_b`, `AB→ab`, `Ab→ab`, `x→x`. Changing this rule changes which
state pairs collide — see blessing hazard 3.

#### The clippy arity gate

`expand::arity_allow()` emits `#[allow(clippy::too_many_arguments)]` unconditionally on the three
generated fn kinds listed in [The arity allow](#the-arity-allow). **The gate is `a7_dx.rs` compiling
clean**, and nothing in it asserts this in Rust — *the assertion IS the target compiling under the
linter*. Its `wide` system carries eight params, one past clippy's default
`too-many-arguments-threshold` of 7. Drop the attribute and
`cargo clippy --workspace --all-targets -- -D warnings` goes red.

**What the gate does NOT cover:** a downstream crate that *lowers* the threshold is covered by the fix
(the attribute is unconditional) but not by the gate. No cheap gate exists — `trybuild` drives rustc,
not clippy, so no fixture can carry a lint at all, and a second probe crate with its own `clippy.toml`
would have to shell out to cargo from a test, against a config-discovery walk this repo has already
measured as leaking from the parent checkout.

### Running the gates

```bash
# The toolchain recipe. RUSTUP_TOOLCHAIN alone is NOT enough on this machine —
# a chocolatey rustc 1.95.0 shadows rustup's 1.97.1 and ignores the variable.
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu

cargo test -p aether-lang --lib                            # 56 unit tests, ~0.01 s (no engine build)
cargo test -p aether-tests --all-targets --no-fail-fast    # 20 tests + 37 goldens
cargo test -p aether-lang --lib expansion_volume -- --nocapture   # the volume numbers
cargo clippy --workspace --all-targets -- -D warnings      # the arity gate lives here
```

`--no-fail-fast` is load-bearing: `cargo test` stops at the first failing target, so one red target
shadows every target ordered behind it — and `aether_tests` has sixteen.

CI (`.github/workflows/ci.yml`) runs `cargo test --workspace --all-targets` in both debug and release,
and `cargo clippy --workspace --all-targets … -- -D warnings`. All three aether crates are listed in the
root `Cargo.toml`'s `members` **and** `default-members`, so the bare form covers them. CI's `cargo test`
lines do **not** pass `--no-fail-fast`.

### What nothing pins

Stated so a contributor does not read a green suite as a stronger claim than it is.

* **Spans, in the unit lane.** `fails_with` asserts message text only. A diagnostic added with a unit
  test and no golden is pinned at half its contract — and [the span sweep](#the-span-sweep) will not
  catch it either, since it only sweeps fixtures that exist.
* **Anything a GPU device would answer.** The mesh half of `scene` (`plane`/`cube`/`mesh` bindings)
  needs a live `VulkanContext`, so `vb_lab::lab` and `annex` are compiled and registered, never run. No
  windowed / `BOYKO_HOST_DUMP` run of an Aether scene exists.
* **A lowered clippy arity threshold.**
* **Self-transition enter/exit semantics.** The only self-targeting handler in the suite is
  `a4_machine_hierarchy.rs`'s `Pulse` (`state A { on Tick => A }`), and that machine declares no
  `enter`/`exit` at all — so whether a self-transition runs them is not asserted anywhere.
* **`aether_lang` on its own proves nothing about the engine.** Worth repeating because it has already
  produced a defect of the "gate that cannot fail" class: at rung A6 a token pin in `aether_lang`
  described itself in a comment as the anti-drift gate for `SpotLight::new`. It gates the expander
  only. The real gate is the compiled `annex` scene, whose liveness was proven by corrupting the
  emitter (six arguments → `E0061`).

---
## Where the plan and the source disagree

`docs/AETHER-LANG-PLAN.md` is design intent. Where it and the shipped code differ, this is what
shipped. Construct-local divergences are called out inline — [`component`](#refusals--component),
[`tag`](#refusals--tag), [`machine`'s grammar](#machine), [`AetherCtx`](#block-rules-aetherctx),
[the block-abort set](#the-version-header). The rest:

### Implementation shape

| Plan | Shipped |
|---|---|
| `src/expand/component.rs`, `src/expand/bundle.rs`, `src/expand/event.rs`, `src/kw.rs` — one file per construct | The crate is flat: `parse.rs`, `expand.rs`, `ast.rs`, `ctx.rs`, `diag.rs`. There is no `syn::custom_keyword!` table. |
| `macrotest` snapshots | `aether_lang`'s own token-for-token unit tests — strictly more precise, and no new third-party dev-dependency. Recorded as a deviation in `lib.rs`, not an omission. |
| `::boyko_ecs::Res`, `::boyko_ecs::Query`, `::boyko_ecs::With` | the REAL nested paths. The root re-exports only `App`/`AppExit`/`Plugin`/`Plugins` and the error types, and tokens must resolve — see [Tokens, not dependencies](#tokens-not-dependencies). |

### `system` and `plugin` (§3.3)

| Plan | Shipped |
|---|---|
| `param := IDENT ':' param_ty` | `param := 'mut'? IDENT ':' param_ty` — a binding-level `mut` is accepted |
| `'(' param (',' param)* ')'` | an **empty** list and a **trailing comma** are both accepted |
| inference set = `query` with `&mut`/`Mut<`, `mut res`, `commands`, `emit` | **plus `events<E>`** — `EventReader::read` takes `&mut self` here |
| a near-miss `before`/`after` ident passes through, with a note attached to rustc's error | an **Aether error** carrying the note's text — stable proc-macros cannot attach notes to downstream rustc errors. Cost: a real `SystemSet` type within edit distance 2 of a sibling system must be referenced by a qualified path. |
| (unstated) | with a `plugin` present, **every** sibling system is registered; clause-free ones land on Main unordered |
| (unstated) | `Query<D, F>` with exactly one filter stays **bare**, no one-tuple |

### `machine` (§3.5 / §5.1)

| Plan §3.5 "After" block | Shipped |
|---|---|
| `for __e in &mut __ev { … if !(guard) { continue; } … return; }` | [drain-then-act](#the-transition-system) with `__aether_fire`. §5.1 is the semantic authority and the expander follows it; the deviation is recorded in `transition_fn`'s own comment. |
| `__ev` / `__next` | `__aether_ev` / `__aether_next` (the prefix rule) |
| `__aether_gameflow__playing_running__player_died` | `__aether_game_flow__…` — the machine-name half is also `snake()`-collapsed |
| `::boyko_ecs::States`, `::boyko_ecs::ResMut`, `in_state` | the real nested paths |
| a doc comment on the generated enum | no doc comment on the enum; only the `in_*` predicates carry one |
| — | `#[allow(clippy::too_many_arguments)]` on every generated fn |
| state body order fixed (`initial`, handlers, nested states) | any order, freely interleaved |

`priority N` on transitions (plan R7) and entity-scoped machines (`machine … on entity`, plan §5.5)
are **not** in the shipped grammar — `parse_state` has no arm for either.

### The narrative pages

Every mapping checked in `book/src/aether/data-constructs.md` matches the source; its gaps are
omissions this reference closes — that `requires` is bare-path-only (so required components are always
`Default`-constructed through Aether), that field attributes / doc comments / explicit visibility /
generics are unsupported, that a zero-field `bundle` is refused by the derive, that `Added`/`Changed`
on a bitset tag is a compile error while `With`/`Without` is not, and that every event field type must
be `Copy`.

---

## Stated limits

Claims in this reference that are read off a code path rather than pinned by a test or observed
running, plus the boundaries of what was checked. Each is here because "an incomplete section that is
true beats a complete one that is plausible".

### Behaviour read from code, not from a green test

| Claim | Basis |
|---|---|
| `system r#Tick() {}` passes the snake_case gate | `parse_system`'s inline check reads the raw `to_string()`, whose first char is `r`. No fixture and no unit test covers a raw-ident system name. |
| `With<T>` / `Without<T>` on a bitset tag matches nothing at runtime | Inference from two verified facts: `With`'s consts gate on `STORAGE_IS_DENSE`, not `STORAGE_IS_BITSET`, and an `EnableTag` never enters an archetype signature mask. No explicit source statement and no test pins the conclusion. |
| A zero-field `bundle B {}` is refused by the derive | Read from the `Bundle` derive's two zero-field guards. Aether's emission of `pub struct B {}` was confirmed; the struct was not compiled through the derive to see the message land. |
| A ZST `event` trips at preregistration | `ZstCheck::<E>::NON_ZERO` is read at exactly one site, `EventDispatcher::preregister::<E>`. Whether a ZST event declared but never preregistered compiles silently was not tested. |
| `component` has no `storage` key | A negative: `parse_component` has no such branch, and `component A { storage = "dense" }` parses as a malformed field. Not an exhaustive search for another spelling. |
| The `#[event]` generated-item order | Read from the emitting `quote!` in `boyko_macros/src/event.rs`, not from a `cargo expand` dump. |
| `system NAME` with no `(` reports syn's own message | `parenthesized!` is called with no `.map_err`; the case was not executed. |
| `MAX_SYSTEM_PARAM_ARITY = 12` and the `13..=24` panic stubs | Read from the constant and `tuple_impl.rs`'s module docs; no >12-param aether system was built. |
| Machine last-write-wins depends on registration order being run order | `expand.rs` and the plan both assert declaration-order registration as the deterministic mitigation, and both systems take `ResMut<NextState<M>>` so the scheduler cannot co-schedule them — but the scheduler was not read to confirm it dispatches conflicting systems in registration order. |
| The initial-enter chain runs before every sibling startup system | Only the **emission** order in `plugin_impl` was verified, not the `App`'s execution order. |
| `merge_params` first-binding-wins for the `mut` spelling | `merged` keeps the first `&SysParam`, so `x: Foo` in one handler and `mut x: Foo` in another emits without `mut`. No test pins it. |
| `resolve_child` can print an empty ``states declared here:`` list | Candidates are filtered by `parent == Some(leaf_idx)`; not demonstrated with a test case. |
| `at_refusal`'s `mesh`/`camera`/`entity` arm is unreachable | Those heads pass `takes_at()`. Read from the source, not proven by a coverage run. |
| An unused `let` mesh binding in a scene | `scene_fn` emits `let NAME = <call>;` with no `#[allow]`, and no suppression or test was found. Whether rustc's `unused_variables` fires (on the user's own span) was not verified by compiling. |
| A 4-param `scene` fn under a lowered `too-many-arguments-threshold` | `arity_allow()` is not applied to `material_fn` or `scene_fn`; the default threshold of 7 cannot be reached, a lowered one was not checked. |
| The per-key "no neutral value" reasons for `sun.lux`, `point.power`/`range`, `spot.power`/`range`/`inner`/`outer` | A restatement of the general rule in `NodeKeySpec::required`. Unlike `sky` (both hemispheres) and `camera.aspect` (the target's width/height), the source gives no per-key sentence for these. |
| How syn splits `>>` when a query's data type ends in a generic (`query<Option<&T>>`) | The parser calls `input.parse::<Token![>]>()` after the data type and relies on syn's handling. No test in the corpus exercises that shape. |
| `did_you_mean` compares whole strings including any `r#` prefix | So a raw-ident typo's edit distance includes the two escape characters. No test covers that combination. |

### Diagnostics not pinned by a golden

* The **composite-predicate collision** and the **two-events-one-fn-name** messages have no fixture in
  `tests/ui/`. Their wording here is quoted from the `format!` strings in `expand.rs`; the example
  values in the composite-predicate row are constructed from the format string, not copied from a
  fixture.
* Span semantics for **failed-parse** and **stream** are stated from syn's documented behaviour plus
  the one directly observed case (`recovery_broken_plugin_keeps_the_block`). Individual
  `map_err(|e| diag::err(e.span(), …))` sites could differ if the underlying parse consumed tokens
  before failing.
* `ParseStream::span()` on a fully exhausted **top-level** buffer returns the buffer's scope, which for
  `syn::parse2` is `Span::call_site()`. `diag.rs` claims no diagnostic can fall back to call-site; that
  holds for the traced paths (the block loop only calls `input.span()` while `!input.is_empty()`) but
  was not exhaustively proven for every nested-buffer `.span()` site.
* Messages produced by syn's own `braced!` / `bracketed!` / `parenthesized!` macros — a `children:`
  value that is not a bracketed list, a `material`/`scene` body that is not braced — are **not**
  Aether-authored, no golden covers them, and their text is not quoted here.
* `plugin_impl` emits `pub struct #pname;` with plain `quote!`, whereas the other five type-producing
  constructs use `quote_spanned!(name.span())`. No golden exercises a plugin-name collision, so no
  consequence is claimed here. The reading that Aether owning the fn-duplicate rule makes rustc's label
  placement moot does not cover `plugin`, which is type-producing — flagged as a possible latent span
  issue, not documented as behaviour.

### Provenance of the gate numbers

* The Aether-layer expansions and messages in this reference were produced by **running**
  `aether_lang::expand_block` from a throwaway crate (`aether_lang` is a plain lib with only
  syn/quote/proc-macro2 deps). Downstream `boyko_macros` / kernel facts are read from source only.
* `cargo test -p aether-lang --lib` was run green (56 passed, 0 failed). The `aether_tests` trybuild
  suite and the clippy `-D warnings` gate were **not** re-run while writing this page, so the
  `line:column` pins quoted here are the committed goldens, not freshly re-measured ones. The aether
  crates were clean in git, so what was read is the shipped state.
* The blessing command form is the repo standard, taken from `crates/boyko_ecs/tests/bundle_compile_fail.rs`.
  No file under `crates/aether{,_lang,_tests}` spells it — the suites state the blessing *discipline*
  but never the command. That is a real doc gap worth closing in the suite headers.
* The UTF-8 blessing hazard's **preconditions** are all verifiable in-tree (32 of 37 fixtures carry
  non-ASCII; the default-encoding behaviour of the local tooling is documented in the repo's working
  notes). The incident narrative itself is recorded in session memory, not under `crates/`.
* `a6_scene.rs`'s comment justifying the `probe_props` / `probe_lights` split says clippy's
  `too_many_arguments` fires on the generated system fn. That was written at rung A6, **before**
  `arity_allow` landed on `system_fn`, so it now over-states the constraint. Not re-tested here.
* The six `.stderr` files in `crates/aether_tests/wip/` are dated 2026-08-20 and correspond to A4
  machine fixtures. Nothing in-tree states they are leftovers from an A4 blessing round.
* Whether the two toolchain-coupled goldens have ever diverged between CI's Linux/stable and local
  windows-gnu 1.97.1 was not checked; only local green was observed.
* CI's `cargo test` lines lack `--no-fail-fast`, which `CLAUDE.md` calls load-bearing. The fact is
  reported; whether it is deliberate for CI is not something the source says.

### Reading coverage

`ctx.rs`, `diag.rs`, `ast.rs` and both `lib.rs` files were read in full. `parse.rs` (77 KB) and
`expand.rs` (192 KB) were read in targeted sections, so a cross-cutting rule could exist in an unread
region of either. `docs/AETHER-LANG-PLAN.md` was not read line by line — every section number cited in
this page is quoted from a source comment, not verified against the plan document.
`book/src/aether/materials.md` and `diagnostics.md` were not read in full; any divergence between those
pages and this one is unassessed.

---

## See also

- [Aether overview](overview.md) — the macro, the crates, and what a block becomes.
- [Data constructs](data-constructs.md) — the explanation behind
  [`component`](#component) / [`tag`](#tag) / [`bundle`](#bundle) / [`event`](#event).
- [Systems & plugins](systems-and-plugins.md) — the param sugar, clauses and sibling ordering, in
  prose.
- [State machines](state-machines.md) — the chart semantics behind [`machine`](#machine).
- [Materials](materials.md) and [Scenes](scenes.md) — the two content constructs, worked through.
- [Diagnostics](diagnostics.md) — the error contract as a narrative, including recovery.
- [Contributing](../contributing.md#changing-the-aether-dsl) — the checklist a DSL change is reviewed
  against.
- [Components](../concepts/components.md), [Enable tags](../concepts/enable-tags.md),
  [Bundles](../concepts/bundles.md), [Events](../concepts/events.md),
  [Systems](../concepts/systems.md), [Queries](../concepts/queries.md),
  [States](../scheduling/states.md), [App & Plugins](../app/plugins.md) — the hand-written surface
  Aether expands to.
- Source: `crates/aether_lang/src/{parse,expand,ast,ctx,diag,lib}.rs`,
  `crates/aether/src/lib.rs` (the shim and its DX checklist),
  `crates/aether_tests/tests/` (behaviour targets and the `ui/` goldens); design plan in
  `docs/AETHER-LANG-PLAN.md`.
