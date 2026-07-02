> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

Both critical claims confirmed against the actual code:

1. **Baseline uses `ec.set_parent(p)`** inside the child's own spawn chain (common/mod.rs:127), NOT `add_child` post-children.
2. **Dangling-parent guard** (commands.rs:249-254): `if !view.has_parent(parent) { remove ChildOf; return }` — a `ChildOf` whose parent is not live at apply is **silently dropped, never retried**. This makes spawn-before-link ordering load-bearing for correctness, not just equivalence.

I now have full grounding for every critical finding. Producing the revised plan.

---

# Architecture: `ui!` authoring macro (boyko_ui GUI Phase P2)

## Goal

A function-like proc-macro `ui!` in `boyko_macros` that lets an author write a UI entity tree as a literal nested block, expanding to `Commands::spawn`/`.insert`/parent-link calls that are **state-equivalent** to the hand-written baseline: same final entities, same component set/values, same resolved archetypes (hitting the Phase-8.5 static bundle cache via `UiNodeBundle`), same `ChildOf`/`Children` hierarchy after one apply window. Zero runtime overhead beyond the equivalent hand-written code (Principle 1): the macro emits no allocation, no `dyn`, no per-node lookup the hand code would not also do. Compile-time key/field checking with spans pointing at the offending token, and **targeted DSL diagnostics for the high-frequency authoring mistakes** (so the "LLM-friendly" claim is real, not aspirational).

## Context and constraints

- **Affected subsystems:** `boyko_macros` (new `ui` proc-macro), `boyko_ui` (new `UiName` component + `UiNodeBundle` + prelude re-export + **extended equivalence baseline**). NO `boyko_ecs` core changes (locked).
- **Invariants preserved:**
  - Bundle cache: only named `#[derive(Bundle)]` structs get a per-world `OnceLock<ArchetypeId>` slot; tuple bundles are not a thing (sealed trait). The macro must emit a named bundle or chained single-component inserts — never a tuple.
  - `MAX_BUNDLE_ARITY = 16` per named bundle (structurally unreachable here — we only ever bundle 2 in `UiNodeBundle`; everything else is 1-component inserts).
  - Hierarchy: `Children` is hook-maintained; author code only ever inserts `ChildOf` (via `set_parent`/`add_child`). The Phase-19 consistency window means linkage materializes at the deferred-command drain.
  - **Dangling-parent guard (load-bearing, verified commands.rs:249-254):** `child_of_on_insert` removes a `ChildOf` whose parent is not live at apply time and **never retries**. Therefore, in a single apply window, **every parent's `SpawnAtCommand` MUST precede every descendant's `Insert(ChildOf)`** or the link is silently dropped. This is a **correctness invariant** of the lowering, not merely an equivalence nicety (see Decision 6 + Multithreading/ordering section).
  - Deferred spawn: `Commands::spawn` reserves an `Entity` via the atomic counter (live id immediately, live entity only after apply). `reserve_entity` (commands.rs:242) also returns a live id synchronously without enqueueing — used by the two-phase lowering (Decision 6). `#name` captures the reserved `Entity` — never reads a component.
- **Target metrics:**
  - For a tree of `N` nodes with `K` total optional components and `L` parent-links: expansion issues exactly `N` `SpawnAtCommand`s + `K` author-`InsertCommand`s + `N_inject` injected-default `InsertCommand`s + `L` `ChildOf` `InsertCommand`s — **identical command counts** to the (extended) hand baseline node-for-node. Asserted by a command-count probe, not inferred (Decision 7, Test #2).
  - No `Vec`/`HashMap`/`format!`/`Box` in the *emitted* code. (Host-side macro code may use `Vec`/`HashMap` — that is expansion-time, not runtime.)
  - Each distinct resolved node-component-set maps to one cached archetype after first spawn; the canonical shape takes the **`UiNodeBundle` cache path** (asserted directly, not inferred from archetype equality — Decision 7, Test #3).

## Key decisions

### Decision 1: Grammar = component-list + trailing `children:` block, `#name` ONLY a node prefix

**What:** A node is `#name? { component-literal, component-literal, ..., children: [ <node>, ... ] }`. The body is a comma-separated list of **component literals** (real Rust struct/enum literal paths), with an optional trailing `children:` clause that **must be last**. `#name` is **exclusively a node-prefix declaration** in P2 (see Decision 3); it is **not** a top-level body item. Value-position references (`SomeComp { target: #title }`) are parse-supported only *nested inside a component literal's field expression*, as a forward seam for P4 (see Decision 3 + grammar).

**Why:** Component literals are real type paths → full rust-analyzer autocomplete/go-to-def and, critically, **the compiler type-checks them**, so unknown fields/types become ordinary E0560/E0412 errors at the user's token (span-forwarded) with zero allow-list maintenance. The block form reads as a literal tree (LLM-friendly, the project's stated authoring goal) and lowers 1:1 onto `spawn(bundle).insert(..)` + linkage. The `children:` keyed clause is explicit and greppable, and is the single canonical form that the P3 `.ui` text parser will mirror (structurally — see "P3 seam caveats").

**Alternatives rejected:**
- Closure-callback DSL (bevy_ui_dsl): no compile-time tree validation, awkward `#name` capture, closure nesting noise. Rejected.
- XML/JSX (belly): string-typed attributes defer validation to runtime, weak tooling, implies `dyn Fn`. Violates Principle 1. Rejected.
- Bare `Children [...]` (BSN): implicit; the keyed `children:` matches "explicit over implicit". Rejected as canonical.
- **`#name` overloaded as a top-level body item (original plan):** ambiguous and a trap for LLM authors — a bare `#other` in a component list is either a no-op or a baffling `Entity: Bundle not satisfied` error (critic-3 finding). **Removed entirely** (see Changes from review).

**Trade-off:** Arbitrary component literals (no closed widget vocabulary) means a typo'd component type produces a type error, not a friendly "unknown widget" — acceptable: spans point at the user token, and a widget vocabulary is P6.

### Decision 2: Per-node lowering = ONE canonical named bundle (set-based recognition) + chained single-component `.insert()`

**What:** The macro recognizes the canonical `UiNodeBundle` by **set membership, not position**: a node maps to `spawn(UiNodeBundle { layout, rect })` iff its component-literal head-path **set** contains *both* `UiLayout` and `ComputedRect` (in any order, any position), pulling those two literals out by head-path match and emitting the remaining literals as chained `.insert()`. Otherwise it emits `spawn(<UiLayout-literal>)` (+ injected `ComputedRect::default()`), or — if no `UiLayout` literal is present — a **macro error** (see Decision 8). Every remaining component literal is `.insert(<literal>)` (each is a single-component `Bundle` from the derive emission).

Recognition is **syntactic on the head path token** (a proc-macro runs before type resolution). The accepted spellings are **explicitly enumerated** (Decision 9): the bare idents `UiLayout` / `ComputedRect` only. A re-pathed/aliased spelling (`crate::components::UiLayout`, `use UiLayout as L`) is **not** recognized as the fast path → the node falls to `spawn(UiLayout-by-head-path)+insert(ComputedRect)`. Because that silent fast-path loss is a Principle-1/cache regression invisible to an archetype-equality gate, the gate asserts the **wanted path directly** (Decision 7) and the canonical spelling is documented as load-bearing.

**Why:** The Phase-8.5 static archetype cache only fires for named `#[derive(Bundle)]` structs as a unit. A canonical `UiNodeBundle` containing the always-present components lets the common node hit the cache in one spawn. Single-component inserts are themselves cache-backed bundles, so each insert's archetype migration is also cached. Set-based (not positional) recognition makes two semantically-identical author orderings lower **identically** — removing the ordering footgun (critic-3 finding).

**Alternatives rejected:**
- **Positional recognition ("leading UiLayout + ComputedRect")** (original plan): author order changes the spawn shape; two identical trees lower differently. Rejected for set-based.
- Per-node generated anonymous bundle struct: pollutes the user's namespace, complicates hygiene, fragments the cache per dynamic shape. Rejected.
- Always single-component inserts (no canonical bundle): the first node spawn would not hit the multi-component cache. Rejected.

**Trade-off:** Distinct optional-component combinations form distinct archetypes (correct, identical to the extended baseline). A re-pathed `UiLayout`+`ComputedRect` silently loses the bundle fast path — mitigated by (a) documenting the canonical spelling and (b) the gate testing the path directly so any future regression is caught.

### Decision 3: `#name` = a real Rust `let` binding capturing the reserved `Entity`, plus a `UiName` component. Value-position refs = BACKWARD-only by construction (Decision 6 makes all refs resolvable)

**What:** `#root { ... }` (a) binds the reserved `Entity` to the **user's ident `root`** (call-site span), AND (b) appends `.insert(::boyko_ui::components::UiName::new("root"))`. A `#title` used in value position **inside a component field expression** resolves to that node's `Entity` local. Because the lowering reserves **all** entity ids up front (Decision 6 two-phase), **every `#name` binding is in scope before any spawn/insert/field-expression is emitted** — so both backward and forward references resolve. An undeclared `#name` reference is a macro error with an exact span.

**Why:** A real `let` binding gives compiler-enforced uniqueness-in-scope and lets the captured `Entity` flow into cross-references with zero runtime cost (no string hashing — clay hashes strings; we do not). The literal string in `UiName` is the P3 hot-reload diff key. Two-phase reservation (Decision 6) eliminates the original plan's fatal forward-reference defect (critic-1 C2): in pure pre-order DFS, a parent referencing a not-yet-spawned child produced E0425. With all ids reserved first, **the reference order is irrelevant** — every `#name` local exists before any value position reads it.

**Alternatives rejected:**
- String-hashed id (clay `CLAY_ID`): collisions, no compiler uniqueness check. Rejected.
- **Pre-order DFS spawn-and-capture, backward-only refs (original plan):** forward refs (parent→child, the headline cross-ref use case) fail to compile. Rejected for two-phase reservation.

**Trade-off:** Two `#name` with the same name in one invocation is a macro error (exact span) — stricter than Rust shadowing, correct for a diff key. A `#named` node whose handle is never referenced would emit an `unused_variables` warning *in the user crate*; suppressed by emitting a trailing `let _ = &name;` binding-use (Decision 10).

### Decision 4: `UiName` payload = inline fixed-capacity small string (`UiNameStr`), no heap, no interner

**What:** `UiName` holds an inline `[u8; 60]` UTF-8 buffer + a `u8` length (64 B total with repr/align padding; see Data structures). `UiName::new(&str)` is a `const fn` that copies the literal bytes; over-length names are a compile error in the macro path (length checked at macro time on the string literal).

**Why:** Principle 1/5 forbid `String`/heap in components and on the hot path. An inline small string is allocation-free for both faces (the `ui!` literal path AND the P3 `.ui` text path), is `Copy`/POD/`Send+Sync` (matches every other boyko_ui component), and stores in a SoA column with no indirection. P3 diffing compares two `UiName` columns with a fixed-size `memcmp` — no interner resource, no `RwLock`, no string-table contention across threads. 60 bytes covers realistic node names; the macro rejects longer ones at compile time with a clear span.

**Alternatives rejected:**
- `&'static str` payload: free for the `ui!` literal path, but P3 `.ui` text-loaded names are not `'static` → divergent representation across faces. Uniformity wins. Rejected.
- Interned `u32` + string-interner `Resource`: adds a global mutable table needing synchronization (intern) and a back-map for diff display; cross-thread interning is a contention/lock surface this engine bans. The UI string key is cold (set at author/load time). Rejected.

**Trade-off:** 64 B per node for `UiName`. `UiName` is OPT-IN — present only on `#named` nodes — so unnamed nodes pay nothing. 60-char ceiling enforced at compile time, never a runtime surprise.

### Decision 5: Canonical bundle = `UiNodeBundle { layout: UiLayout, rect: ComputedRect }`; post-link archetype model documented

**What:** Introduce `#[derive(Bundle)] pub struct UiNodeBundle { pub layout: UiLayout, pub rect: ComputedRect }` in `boyko_ui` (new `bundles.rs`). Every `ui!` node spawns this base when its component set contains both `UiLayout` and `ComputedRect` (Decision 2); otherwise it spawns `UiLayout` and injects `ComputedRect::default()`.

**Post-link archetype model (made explicit — critic-2 finding):** the canonical bundle is the node's *base*, **not** its final archetype. The hierarchy hooks migrate archetypes on linking:
- **Leaf, no children, not a child:** `UiNodeBundle(+opts)`.
- **Child (has a parent):** `UiNodeBundle(+opts) + ChildOf` (the child's `ChildOf` insert migrates it).
- **Parent (has ≥1 child):** `UiNodeBundle(+opts) + Children` (the first `LinkChildCommand` migrates the parent into a `Children`-bearing archetype, commands.rs:129-136).
- **Mid-tree node (parent and child):** `UiNodeBundle(+opts) + ChildOf + Children`.

The equivalence gate compares **POST-apply-window** archetypes; `Children`/`ChildOf` membership is part of the equivalence contract. The developer asserts against these final shapes, not the pre-link base.

**Why:** `UiLayout` is the always-read primary input; `ComputedRect` is written for every laid-out node. Bundling exactly these two = the minimal always-present set → the common node hits the cache as a 2-component unit in one spawn. Everything else is opt-in via `.insert()`. Keeping the base minimal avoids bloating leaf archetypes / L1d footprint.

**Alternatives rejected:**
- Larger canonical bundle (include `UiSpacing`/`UiAlign`): forces those onto leaves that do not need them. Rejected.
- No `ComputedRect` in the bundle: every node would do an extra archetype migration. The baseline always has it; bundling is strictly better. Kept.

**Trade-off:** A node written without `ComputedRect` still gets `ComputedRect::default()` injected (every node carries a rect — the renderer/layout require it). Documented, matches the baseline contract.

### Decision 6: Two-phase lowering — reserve ALL entity ids first, then spawn/insert/link

**What:** The macro lowers in two passes over the parsed tree:
- **Phase A (reserve):** for **every** node (named or anonymous), emit `let <binding> = #cmds.reserve_entity();` (commands.rs:242 — returns a live `Entity` id synchronously, enqueues nothing). `#named` nodes use the user's ident; anonymous nodes use a hidden `__ui_n{counter}` ident. After Phase A, **every** node's `Entity` is in scope.
- **Phase B (materialize):** pre-order DFS. For each node, emit `#cmds.entity(<binding>).insert(<base-bundle-or-layout>)...` — using `entity(reserved_id)` (not `spawn`) so the spawn targets the pre-reserved id. Then the injected rect, the author inserts, and the `UiName` insert. Then, **after** a node's own materialization, emit its children's link edges `#cmds.entity(<parent>).add_child(<child>);` — but only after **both** parent and child have been materialized in Phase B (guaranteed by pre-order: parent materialized before descending into children).

**Why this is correct (resolves critic-1 C2 + critic-2 #2):**
- **Forward references:** every `#name` local exists after Phase A, so a value-position `#title` inside a parent's component field resolves regardless of declaration order. The original pre-order-DFS spawn-and-capture could not do this.
- **Dangling-parent guard:** Phase B is strict pre-order, so a node's spawn/insert (the `SpawnAtCommand` equivalent — see note below) is enqueued before its children's `add_child` (`Insert(ChildOf)`). At apply, FIFO drain guarantees every parent is live before its child's `ChildOf` insert is processed → `has_parent(parent)` is true → the link is **not** silently dropped (verified commands.rs:250). This is the load-bearing correctness invariant.

**Note on `entity(id).insert(base)` vs `spawn(base)`:** `reserve_entity` does not enqueue a spawn; the entity is materialized when the **first** structural command for that id is applied. The macro's Phase B emits `#cmds.entity(<reserved>).insert(<base-bundle>)` — the first insert on a reserved-but-not-spawned id is the materialization point. **Open item O1** flags verifying that `EntityCommands::insert` on a reserved id materializes the entity (vs requiring an explicit `spawn`); if `insert`-on-reserved is not a spawn, Phase B instead emits `#cmds.spawn(<base-bundle>)` capturing into the **pre-reserved binding via shadowing is NOT used** — see the fallback in Open questions O1. The plan's primary design assumes the well-established Phase-11 pattern (reserve id, then build it) works; the developer verifies O1 before coding Phase B and picks the confirmed primitive.

**Alternatives rejected:**
- **Single-pass pre-order DFS with spawn-and-capture (original plan):** cannot resolve forward refs; presented the spawn-before-link ordering as a "nicety" rather than a guarded correctness invariant. Rejected.
- Restrict value-position refs to backward-only + macro-error on forward refs: viable but strictly less capable, and the two-phase reservation costs nothing extra (one `reserve_entity` call per node — the spawn would reserve an id anyway). Rejected in favor of full forward-ref support.

**Trade-off:** One extra `reserve_entity()` call per node vs letting `spawn` reserve implicitly. `reserve_entity` is a single atomic fetch-add (commands.rs:242) — negligible, and the id would be reserved by `spawn` regardless, so net cost is ~one atomic per node, no allocation. Acceptable for the forward-ref + ordering-correctness guarantee.

### Decision 7: Cache-path + command-count gates assert the WANTED PATH, not just the resulting archetype

**What:** Two gate mechanisms make the Principle-1/cache claims **directly tested** (resolving critic-2 #1 and critic-1 minor):
- **Cache-path assertion (Test #3):** spawn the canonical shape via `ui!`, then assert the `UiNodeBundle` `BundleTypeId`'s per-world `OnceLock<ArchetypeId>` cache slot **is populated** (proving the `spawn(UiNodeBundle{..})` path executed, not a `spawn(UiLayout)+insert(ComputedRect)` migration that converges to the same archetype). The probe reads the cache slot for `UiNodeBundle`'s `BundleTypeId` (Phase-8.5 mechanism; the test obtains the slot via a small `#[cfg(test)]` accessor on `boyko_ui` that calls the existing `bundle_archetype_id_for` query path, or asserts the archetype was reached as a *bundle* spawn — see Test plan #3 for the exact probe).
- **Command-count assertion (Test #2):** drive both the `ui!` path and the extended hand baseline through a `CommandQueue` whose length is probed before/after, asserting the macro issues **exactly** `N` spawns + `K` author inserts + `N_inject` injected inserts + `L` `ChildOf` inserts — matching the baseline node-for-node. This catches a double-emitted `ComputedRect` (an extra migration = a Principle-1 regression) that an archetype-equality-only gate is blind to.

**Why:** The original gate relied solely on `entity_archetype_id` equality, which is **structurally blind** to (a) a missed `UiNodeBundle` fast path and (b) a redundant insert command — both real Principle-1 regressions. Asserting the path and the command count makes the zero-overhead claim a tested invariant.

**Trade-off:** Requires a small `#[cfg(test)]`-only cache-slot accessor and a command-count probe. Test-only surface, no production cost.

### Decision 8 & 9 & 10: see "Compile-time validation", "Path / hygiene strategy", and "Emitted-code lint cleanliness" below (each addresses a specific critic finding and is detailed in its dedicated section to avoid duplication).

## Data structures

### New component: `UiName` (in `boyko_ui/src/components.rs`)

```rust
/// Stable author-assigned name for a node. OPT-IN: present only on `#named`
/// nodes. The diff key for P3 hot-reload (compare two columns with a fixed-size
/// memcmp). Inline small-string — no heap, no interner: Principle 1/5.
#[repr(C, align(64))]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiName {
    /// UTF-8 bytes of the name; only `len` are meaningful, the rest are zero.
    bytes: [u8; Self::CAP],   // 60 B — covers realistic node names
    /// Valid byte count in `bytes` (<= CAP).
    len: u8,                  // 1 B
    _pad: [u8; 3],            // pad to 64 B (one cache line, clean column stride)
}

impl UiName {
    pub const CAP: usize = 60;
    /// Copies `s` into the inline buffer. `s.len()` MUST be <= CAP; the `ui!`
    /// macro enforces this at compile time, so the runtime path debug_asserts.
    pub const fn new(s: &str) -> Self;     // const so the literal path is fold-friendly
    pub fn as_str(&self) -> &str;          // cold (debug/diff display only)
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```
- Size = 64 B, exactly one cache line, one aligned column store target. `Copy`/`Eq` → `PartialEq` column compare in P3 is a single fixed-width `memcmp`. `derive(Component)` auto-emits the single-component `Bundle`, so `.insert(UiName::new("x"))` works.

### New canonical bundle (in new `boyko_ui/src/bundles.rs`)

```rust
use boyko_macros::Bundle;
use crate::components::{ComputedRect, UiLayout};

/// The always-present node base. Hits the Phase-8.5 static archetype cache as a
/// single 2-component unit. A `ui!` node spawns this base when its component set
/// contains BOTH UiLayout and ComputedRect (set-based recognition, Decision 2);
/// otherwise the macro injects `ComputedRect::default()` alongside the author's
/// UiLayout. NOTE: this is the node BASE, not its final archetype — linking adds
/// ChildOf (on a child) and/or Children (on a parent). See Decision 5.
#[derive(Bundle)]
pub struct UiNodeBundle {
    pub layout: UiLayout,
    pub rect: ComputedRect,
}
```

### Macro-internal AST (in `boyko_macros`, NOT emitted)

```rust
// Parsed form of one node. Lives only inside the proc-macro.
struct UiNode {
    name: Option<syn::Ident>,        // from `#name`; span preserved
    name_span: Span,
    components: Vec<syn::Expr>,       // each is a real component literal expr; span preserved
    children: Vec<UiNode>,           // recursive
    brace_span: Span,                // for nesting/arity diagnostics
}
// Plus a host-side name table: HashMap<String, Span> for duplicate detection +
// a HashSet<String> of all declared names for unknown-ref validation.
// (Vec/HashMap here are expansion-time host code, not emitted runtime code.)
```

## Public API

### Macro (in `boyko_macros/src/lib.rs`)

```rust
/// Function-like macro: author a UI entity tree. Expands to a block expression
/// that runs against a `Commands` binding in scope (default name `cmds`; override
/// with `commands: <ident>;` as the first clause). Returns the root `Entity`
/// (or a tuple of roots if multiple top-level nodes).
#[proc_macro]
pub fn ui(input: TokenStream) -> TokenStream;
```

### boyko_ui additions

```rust
// components.rs
pub struct UiName { /* see above */ }
impl UiName { pub const CAP: usize; pub const fn new(s: &str) -> Self; /* ... */ }

// bundles.rs
pub struct UiNodeBundle { pub layout: UiLayout, pub rect: ComputedRect }

// lib.rs prelude
pub use boyko_macros::ui;
pub use crate::components::UiName;
pub use crate::bundles::UiNodeBundle;
```

### Extended equivalence baseline (test-support, `boyko_ui/tests/common/mod.rs`)

To make the `#named` and "every optional component" gate rows constructible (critic-2 #3, critic-3 minor), `NodeSpec` is extended:

```rust
pub struct NodeSpec {
    pub layout: UiLayout,
    pub spacing: Option<UiSpacing>,
    pub align: Option<UiAlign>,
    pub absolute: Option<UiAbsolute>,
    pub content: Option<ContentSize>,
    pub root: bool,
    pub name: Option<&'static str>,   // NEW: hand-side UiName, inserted in the
                                      // SAME relative position the macro emits it
                                      // (after base + injected rect, per Phase B order)
}
```
The hand `spawn` path inserts `UiName::new(name)` at the matching position so a `#named` DSL node and its position-matched hand node have **identical** component sets (same archetype). Additionally, a **bundle-path baseline variant** `spawn_via_bundle(NodeSpec)` that hand-writes `cmds.spawn(UiNodeBundle { layout, rect })` is added so Test #1 can compare the macro's bundle fast path against an identical hand bundle shape (critic-3 #2).

## Grammar (final, canonical)

```
ui!         := preamble? node ( ',' node )* ','?            // one or more top-level (root) nodes
preamble    := 'commands' ':' IDENT ';'                     // optional Commands binding name (default: cmds)
node        := name? '{' body '}'
name        := '#' IDENT                                     // node-prefix ONLY (declares a let-binding + UiName)
body        := items? children?
items       := component_item ( ',' component_item )* ','?
component_item := EXPR                                       // a Rust component literal: Comp{..} | Comp(..) | Comp | path::Comp{..}
                                                            //   (a value-position `#name` may appear INSIDE a field EXPR,
                                                            //    e.g. `SomeComp { target: #title }` — never as a bare item)
children    := 'children' ':' '[' ( node ( ',' node )* ','? )? ']'
```

**Reserved context keywords (critic-3 minor):** `children` and `commands` are reserved at their positions (body-leading-clause / preamble-leading). A component type literally named `children`/`commands` must be path-qualified (`my_mod::children { .. }`). Collision is vanishingly unlikely — component types are `CamelCase` by convention (CLAUDE.md).

**`#name` is never a body item.** A bare `#IDENT` in the body position is a macro error: "a node reference must appear inside a component field; a bare `#name` is not a component". Value-position `#name` is only valid nested inside a component literal's field expression (Decision 3); the parser substitutes it with the corresponding `Entity` local during field-expr token rewriting.

**Parser = explicit recursive descent over `ParseStream` (critic-1 #5, critic-3 #4)**, NOT comma-split-then-`syn::Expr`. Per node:
1. Peek for `#` → parse the optional name (`input.parse::<Token![#]>()` then `Ident`).
2. Parse the braced body group.
3. Inside the body, loop: at each item boundary, **peek** for the `children` keyword (`input.peek(Ident) && input.fork().parse::<Ident>()? == "children"`); if found, parse the `children:` clause and require it to be the last clause (any further token before `}` → error "children must be the last clause"). Otherwise parse one `component_item` as a `syn::Expr` (after first rejecting a bare `#IDENT` body item and a bare `[...]` array item with targeted diagnostics — see Compile-time validation).
4. Recurse into the bracketed child list.

This explicit descent is what makes every named error (children-not-last, empty-node, trailing comma, inline-brace-node, `=`-instead-of-`:`) producible with `syn::Error::new(span, ...)` at the offending token, instead of an opaque `syn::Expr` parse failure.

### Example 1 — a leaf (one component)

```rust
let label = ui! {
    UiLayout { width: Unit::Px(120.0), height: Unit::Px(24.0), ..Default::default() }
};
```
1 node, `name=None`, `components=[UiLayout{...}]`, `children=[]`. No `ComputedRect` literal → macro injects `ComputedRect::default()`.

### Example 2 — a node with multiple components (canonical bundle, set-based + inserts)

```rust
let panel = ui! {
    UiSpacing { padding_left: Unit::Px(8.0), ..Default::default() },   // order-independent
    UiLayout { width: Unit::Px(300.0), ..Default::default() },
    ComputedRect::default(),
    UiAlign { main: AlignMain::Center, ..Default::default() }
};
```
Set contains `{UiLayout, ComputedRect}` (regardless of position) → `spawn(UiNodeBundle { layout: <UiLayout-lit>, rect: <ComputedRect-lit> })`; `UiSpacing`, `UiAlign` → `.insert()`. Author order of the remaining inserts is preserved (deterministic archetype).

### Example 3 — nested children

```rust
let root = ui! {
    UiLayout { layout_type: LayoutType::Column, ..Default::default() },
    UiRoot,
    children: [
        { UiLayout { height: Unit::Px(48.0), ..Default::default() } },
        { UiLayout { height: Unit::Px(48.0), ..Default::default() } }
    ]
};
```
Root: `components=[UiLayout{..}, UiRoot]`, `children=[childA, childB]`. No `ComputedRect` literal → inject. `UiRoot` (ZST tag) → `.insert(UiRoot)`.

### Example 4 — a `#named` node with a forward cross-reference (now compiles via two-phase)

```rust
let header = ui! {
    #shell {
        UiLayout::default(),
        UiRoot,
        // forward ref into a child declared BELOW — resolves because Phase A
        // reserves `title` before any Phase-B materialization:
        SomeLink { target: #title },
        children: [
            #title { UiLayout { height: Unit::Px(32.0), ..Default::default() } }
        ]
    }
};
```
Phase A: `let shell = cmds.reserve_entity(); let title = cmds.reserve_entity();`. Phase B materializes `shell` (with `SomeLink { target: title }` — the `#title` token rewritten to the `title` local), then `title`, then `cmds.entity(shell).add_child(title);`. The forward ref compiles because `title` is in scope from Phase A.

### Example 5 — explicit base + spacing + root, deep nest, `commands:` override

```rust
ui! {
    commands: my_cmds;
    #hud {
        UiLayout { layout_type: LayoutType::Column, width: Unit::Percent(100.0), ..Default::default() },
        ComputedRect::default(),
        UiSpacing { padding_top: Unit::Px(12.0), row_gap: Unit::Px(6.0), ..Default::default() },
        UiRoot,
        children: [
            #bar { UiLayout { height: Unit::Px(40.0), ..Default::default() }, UiSpacing { column_gap: Unit::Px(4.0), ..Default::default() } },
            { UiLayout::default(), ContentSize { width: 200.0, height: 18.0 } }
        ]
    }
}
```
`commands:` binds `my_cmds`; `#hud` set contains `{UiLayout, ComputedRect}` → `UiNodeBundle` base + `UiSpacing`/`UiRoot` inserts; `#bar` child; an anonymous leaf with `ContentSize`.

## Expansion (lowering) — two-phase

The macro emits, in this order:

**Phase A (reserve, breadth- or depth-first over the whole tree):** for every node, `let <binding> = #cmds.reserve_entity();` (`#named` → user ident; anonymous → `__ui_n{counter}`).

**Phase B (materialize, strict pre-order DFS):** for each node:
1. **Resolve the base.** If the node's component set contains both a `UiLayout` head-path literal and a `ComputedRect` head-path literal, emit `#cmds.entity(<binding>).insert(::boyko_ui::bundles::UiNodeBundle { layout: <UiLayout-lit>, rect: <ComputedRect-lit> })` (both literals via `quote_spanned!(lit.span()=> #lit)` at the bundle-field site — see span strategy), and drop those two from the insert list. Else if the set contains a `UiLayout` literal, emit `#cmds.entity(<binding>).insert(<UiLayout-lit>)` and **inject** `.insert(::boyko_ui::components::ComputedRect::default())`. Else (no `UiLayout`) → **macro error** (Decision 8). Remaining components → `.insert(<lit>)` in author declaration order (deterministic archetype). Field expressions are token-rewritten so any nested value-position `#name` becomes the corresponding `Entity` local.
2. **`UiName` for `#named` nodes:** `.insert(::boyko_ui::components::UiName::new(<name-as-str-literal>))`.
3. **Recurse into children** (Phase B), materializing each child after the parent.
4. **Link:** for each child, after both parent and child are materialized, emit a standalone statement `#cmds.entity(<parent-binding>).add_child(<child-binding>);` — **one `add_child` per statement, never chained across nodes** (Decision: borrow-shape contract, critic-1 #4). The parent is materialized before this in Phase B, so at apply the parent's spawn precedes this `ChildOf` insert → dangling guard passes.
5. **Unused-name suppression:** for each `#named` node, emit `let _ = &<binding>;` so an unreferenced handle does not warn under `-D warnings` (Decision 10).
6. The whole macro body is a block evaluating to the root binding(s) (single root → `Entity`; multiple → tuple).

### Generated TokenStream for Example 3 (representative, two-phase)

```rust
{
    // Phase A — reserve every id (so any #name forward-ref resolves):
    let __ui_n0 = cmds.reserve_entity();   // root
    let __ui_n1 = cmds.reserve_entity();   // childA
    let __ui_n2 = cmds.reserve_entity();   // childB

    // Phase B — materialize (strict pre-order), no typed reborrow (see hygiene):
    // root: set lacks ComputedRect -> spawn UiLayout + inject rect
    cmds.entity(__ui_n0).insert(UiLayout { layout_type: LayoutType::Column, ..Default::default() });
    cmds.entity(__ui_n0).insert(::boyko_ui::components::ComputedRect::default());  // injected
    cmds.entity(__ui_n0).insert(UiRoot);                                          // verbatim

    // childA
    cmds.entity(__ui_n1).insert(UiLayout { height: Unit::Px(48.0), ..Default::default() });
    cmds.entity(__ui_n1).insert(::boyko_ui::components::ComputedRect::default());

    // childB
    cmds.entity(__ui_n2).insert(UiLayout { height: Unit::Px(48.0), ..Default::default() });
    cmds.entity(__ui_n2).insert(::boyko_ui::components::ComputedRect::default());

    // Link — one standalone add_child per statement (no cross-node chaining).
    // Parent materialized before this -> dangling-parent guard passes at apply.
    cmds.entity(__ui_n0).add_child(__ui_n1);
    cmds.entity(__ui_n0).add_child(__ui_n2);

    __ui_n0   // evaluates to the root Entity
}
```

Notes vs the original plan's emission:
- **No `let __ui_cmds: &mut Commands = &mut cmds;` typed reborrow.** Methods are called directly on the user's `cmds` (or the `commands:` override). This drops the `elided_lifetimes_in_paths`/needless-borrow lint surface (critic-1 C1) and the gratuitous binding entirely.
- Each `cmds.entity(<id>).insert(..)` is a standalone statement → no `EntityCommands` borrow is held across another `cmds` use → no E0499 even when a field expression reads a sibling `#name` local (Decision: borrow-shape contract, critic-1 #4). `Entity` locals are plain `Copy` ids, never `EntityCommands`.

For a node whose set contains both `UiLayout` + `ComputedRect` (Example 2), step 1 emits the `UiNodeBundle` spawn — the Phase-8.5 cache hit:
```rust
cmds.entity(__ui_n0).insert(::boyko_ui::bundles::UiNodeBundle {
    layout: UiLayout { width: Unit::Px(300.0), ..Default::default() },   // quote_spanned! at field site
    rect: ComputedRect::default(),                                        // quote_spanned! at field site
});
cmds.entity(__ui_n0).insert(UiSpacing { .. });   // verbatim
cmds.entity(__ui_n0).insert(UiAlign { .. });     // verbatim
```

### Command-order / state equivalence to the (extended) hand baseline (the GATE)

**Honest claim (corrected per critic-1 C3 and critic-3 #3):** the expansion is **STATE-equivalent**, not byte-for-byte command-order-equivalent. The baseline (common/mod.rs:127) links via `ec.set_parent(p)` *inside the child's own spawn chain*; the macro batches `add_child` after the subtree. Both enqueue exactly one `Insert(ChildOf(parent))` per child, so the **final** archetypes, component values, and `ChildOf`/`Children` membership are identical after one apply window. The gate asserts that final state — it does **not** assert queue order. The two paths are made byte-closer by the bundle-path baseline variant (Test #1), but order parity is explicitly **not** claimed.

**Why batching `add_child` at the end is correct and chosen over interleaving:** in a single apply window the only ordering that matters for correctness is "every parent's spawn precedes its descendants' `ChildOf` inserts" (the dangling guard). Phase-B pre-order materializes the parent before any child link, so the guard always passes. Batching all of a subtree's links after the subtree's spawns is the simplest emission that provably satisfies this. The developer must **not** "fix" this to interleave `set_parent` mid-spawn without re-proving the guard holds for that order. (Interleaving also happens to satisfy the guard since the parent is spawned first in pre-order, but the batched form is the documented contract.)

**Computed-value assertion handled correctly (critic-1 C3):** the gate does **not** assert `ComputedRect` *layout-computed* values after a single window as a structural-identity proof (a single-window snapshot can depend on drain order). Instead: (a) it asserts the **authored** component values (the `UiLayout`/`UiSpacing`/etc. the author wrote, which are drain-order-independent), archetype id, presence, and `ChildOf`/`Children` membership; (b) if a `ComputedRect`-value comparison is wanted, the gate **runs layout to a fixed point** (repeated apply+layout until `ComputedRect` columns stop changing) on **both** trees before comparing, so the comparison is order-independent by construction. This is stated explicitly in Test #1.

### `#named` node expansion (Example 4 `#title`)
```rust
// Phase A:
let title = cmds.reserve_entity();
// Phase B:
cmds.entity(title).insert(UiLayout { height: Unit::Px(32.0), ..Default::default() });
cmds.entity(title).insert(::boyko_ui::components::ComputedRect::default());
cmds.entity(title).insert(::boyko_ui::components::UiName::new("title"));   // diff key
// unused-name suppression:
let _ = &title;
```
The binding is the user's ident `title`, so a value-position `#title` (forward or backward) resolves to it. `let _ = &title;` suppresses `unused_variables` if the author never references the handle (Decision 10).

## Compile-time validation

### Checked IN the proc-macro (macro errors, exact spans via `syn::Error::new[_spanned]`, all surfaced at once via `.combine()`)
- **Duplicate `#name`** within one invocation → error at the second occurrence's span: "duplicate ui name `foo`; names must be unique within a `ui!` invocation".
- **Malformed nesting** (missing brace, `children:` not followed by `[`, node with no body) → explicit recursive-descent errors with spans.
- **`children:` not last** → error at the offending trailing token: "`children:` must be the last clause in a node body".
- **`children:` appearing twice** → error at the second occurrence.
- **`children:` before any items** is allowed (a node may be link-only with a base) **only if** the node still has a base — but a node with *only* `children:` and no components is the empty-node case → error (a node needs at least its `UiLayout`). Stated: a node body must contain at least one component literal (which must yield a `UiLayout`, Decision 8).
- **Empty node** (`{ }`) → error: "a ui node needs at least one component (a `UiLayout`)".
- **Node without a `UiLayout` literal** (Decision 8) → error at the node's brace span: "a ui node requires a `UiLayout` component". (Resolves critic-2 minor: a rect-only dead node is rejected, not silently injected. Symmetric choice over "inject `UiLayout::default()`" — chosen because a layout-less node is almost always an authoring mistake, and an explicit error is friendlier than a silently-default-laid-out node.)
- **`#name` over `UiName::CAP` bytes** → error at the name ident span: "ui name `…` exceeds 60 bytes".
- **`#name` reference to an undeclared name** (value position inside a field) → error at the reference span: "unknown ui name `bar`". (With two-phase reservation, *declared* names always resolve, so this fires only for genuinely-undeclared refs.)
- **Bare `#IDENT` as a body item** (not inside a field expr) → error: "a node reference must appear inside a component field; a bare `#name` is not a component".
- **Targeted high-frequency-mistake diagnostics (critic-3 #4):**
  - **Inline brace-node among items** (`{ UiLayout::default(), { ... } }`) → detect a `{`-led item and error: "a child node must appear inside a `children: [ ... ]` clause".
  - **`=` where `:` expected** (`children = [...]`, `commands = x;`) → detect `=` after the `children`/`commands` keyword and error: "expected `:` after `{keyword}`, found `=`".
  - **Bracket-array as a body item** (`{ UiLayout::default(), [ ... ] }`) → detect a `[`-led item and error: "expected a `children: [ ... ]` clause; a bare `[ ... ]` is not a component".
  - **Trailing-comma rules:** a trailing comma after the last item and after `children: [...]` is accepted; a comma *between* `children: [...]` and a following item is unreachable because `children:` must be last (any token after it → "children must be last").
- **Component count > 16 in a single bundle:** structurally unreachable (only `UiNodeBundle`'s 2 are bundled; the rest are 1-component inserts). Documented, no check.

**Error message style (critic-1 minor):** match the house `syn::Error` casing/format in `boyko_macros` (lib.rs existing messages). Sentence-style, lowercase-leading where the existing macros do, backticked identifiers.

### Deferred to type-check (with span forwarding so the error points at the user's token)
- **Unknown component type / wrong field name / wrong field type:** the component literal is forwarded verbatim inside `quote!`/`quote_spanned!` (preserving its `Span`), so `UiLayout { widht: ... }` → rustc E0560 "no field `widht`" at `widht`. A non-`Component` type in the list fails the `insert::<B: Bundle>` bound (E0277) at the user token.
- **Span strategy at synthesized coercion sites (critic-1 #6):** **mandate** `quote_spanned!(expr.span()=> #expr)` at every synthesized coercion site — specifically the `UiNodeBundle { layout: <lit>, rect: <lit> }` field positions (the highest-risk blur site: a non-`UiLayout` literal placed in the `layout:` field reports a type mismatch). The injected `ComputedRect::default()` uses `call_site` (it has no user token). A compile-fail case asserts that a wrong-type literal in the bundle-field position reports against the **user's type**, not against `UiNodeBundle`.

## Path / hygiene strategy

- **Absolute paths only.** Emit `::boyko_ecs::ecs::core::system::Commands` (only where a path is unavoidable — see below), `::boyko_ecs::ecs::core::entity::entity::Entity`, `::boyko_ui::components::{ComputedRect, UiName}`, `::boyko_ui::bundles::UiNodeBundle`. Mirrors the existing `bundle_macro` emit (lib.rs:2792+). `$crate` does not exist for `#[proc_macro]` — leading-`::` paths are the only correct choice.
- **No typed reborrow of `Commands` (critic-1 C1).** The original `let __ui_cmds: &mut ::...::Commands = &mut cmds;` is **removed**: (a) it spelled `Commands` without its `'s` lifetime → `elided_lifetimes_in_paths` lint in the user crate under `-D warnings`; (b) it was gratuitous. Methods are called directly on `cmds` (or the `commands:` override ident). Per-node block-scoped reborrow is handled by the borrow checker with no annotation. **A `-D warnings` compile-pass test** (Test #6) runs a representative expansion to lock this in (not just `trybuild` compile-fail).
- **Dependency direction is sound:** the emitted code compiles in the user's crate, which depends on `boyko_ui` (and transitively `boyko_ecs`), so `::boyko_ui::...` / `::boyko_ecs::...` resolve. `boyko_macros` does NOT need a path-dep on `boyko_ui` (it only emits textual paths). No dev-dep cycle: `boyko_ui` → `boyko_macros` (normal dep, confirmed Cargo.toml:13), `boyko_macros` does not depend on `boyko_ui`.
- **No identifier capture:** all macro-synthesized locals use the reserved prefix `__ui_` (`__ui_n0`, `__ui_n1`, …) at `Span::call_site()` (proc-macros have weak hygiene; the prefix is the guard — same convention as `__bundle_field_{}`, lib.rs:2525). `#named` bindings deliberately use the **user's** ident (intentional exposure for cross-references), at call-site span.
- **`#name` colliding with the commands binding (critic-2 minor):** a `#cmds` (or `#my_cmds` under override) would bind a local `cmds`/`my_cmds` shadowing the user's `Commands` binding *after* the macro reads it — but Phase B calls `cmds.entity(..)` *before* any such shadow could matter only if the shadow is emitted first. To eliminate the hazard entirely, the macro **errors** if a `#name` equals the active commands binding ident: "ui name `cmds` collides with the commands binding; choose another name". A compile-fail case (`name_collides_commands.rs`) locks this in.
- **Commands binding:** default name `cmds` (the established convention); `commands: <ident>;` preamble overrides.

## Emitted-code lint cleanliness (Decision 10)

The engine gate is `clippy --all-targets -D warnings` *in the user crate*. The emitted code must be clean:
- **Conditional `mut`:** never emitted — Phase B uses `cmds.entity(id).insert(..)` standalone statements, so no `let mut __ec` exists. (The original `let mut __ec` pattern is gone.)
- **No needless binding for trivial nodes (critic-1 minor):** a leaf with one component and no children emits `cmds.entity(<id>).insert(<base>); cmds.entity(<id>).insert(ComputedRect::default());` then the id flows out — no `{ let __ec = ...; __ec.id() }` block. The root value is the Phase-A binding (an `Entity`), returned directly.
- **Unused `#named` handle:** suppressed via `let _ = &<binding>;` (Decision 3 trade-off / critic-3 minor). Tested.
- **Needless-borrow / unused-mut on the trivial path:** Test #6 (`-D warnings` compile-pass) covers a single unnamed leaf, a single unreferenced `#named` leaf, and a zero-insert-extra node.

## Re-export path

- `ui!` lives in `boyko_macros` as `#[proc_macro] pub fn ui`.
- `boyko_ui` re-exports it: `pub use boyko_macros::ui;` in `boyko_ui::prelude` (lib.rs:29-37) AND a top-level `pub use boyko_macros::ui;` so both `boyko_ui::ui!` and `boyko_ui::prelude::ui!` work.
- Safe (no cycle): `boyko_ui` lists `boyko-macros` as a normal dependency (Cargo.toml:13); `boyko_macros` does not depend on `boyko_ui`. (The `boyko_ecs::prelude` constraint that bars re-exporting its own derives is internal to `boyko_ecs` and does not touch `boyko_ui`.)
- Also re-export `UiName` and `UiNodeBundle` from `boyko_ui::prelude`.

## Multithreading model / ordering correctness

- The macro emits **single-threaded** authoring code (one `Commands` queue). No shared state, no atomics introduced by the macro itself (`reserve_entity` uses the world's existing atomic counter, commands.rs:242 — one fetch-add per node).
- **The one load-bearing ordering invariant:** within the single apply window, every parent's spawn (its first structural command) is enqueued before any descendant's `Insert(ChildOf)`. Guaranteed by Phase-B strict pre-order + per-subtree-trailing links. At FIFO drain, `has_parent(parent)` is true when each `child_of_on_insert` runs → no link is silently dropped (commands.rs:250). **This is a correctness invariant, gated by Test #4 (deep single-window tree).**
- No data-race surface: the macro neither shares data across threads nor introduces interior mutability. `UiName` is `Copy`/`Send`/`Sync` like every other component.

## Test / gate plan

All in `crates/boyko_ui/tests/`. The **equivalence gate** + **cache-path gate** + **single-window link gate** are load-bearing.

### 0. Extend the equivalence baseline FIRST (`tests/common/mod.rs`)
- Add `name: Option<&'static str>` to `NodeSpec`; hand `spawn` inserts `UiName::new(name)` in the macro's emission position (after base + injected rect).
- Add `spawn_via_bundle(NodeSpec)` that hand-writes `cmds.spawn(UiNodeBundle { layout, rect })` + the same optional inserts, for the bundle-path comparison.
- (Without this, the `#named` and bundle-path gate rows are not constructible — critic-2 #3, critic-3 minor.)

### 1. DSL ≡ hand-spawn STATE equivalence (`tests/ui_macro_equiv.rs`)
For ~7 tree shapes (leaf; multi-component node hitting `UiNodeBundle`; multi-component node with **shuffled order** to prove set-based recognition; 2-level nest; 3-level nest; `#named` nodes; node with every optional component incl. `UiName`), build the SAME tree via `ui!`, via `common::Ui::spawn` (insert path), AND via `spawn_via_bundle` (bundle path) in one world. After one apply window, for position-matched entity pairs assert:
- **Archetype identity (POST-link):** `world.entity_archetype_id(dsl_e) == world.entity_archetype_id(hand_e)` (ecs_master.rs:2020), against the **post-link** archetype (Decision 5 model: leaf / +ChildOf / +Children / +both). All three paths (DSL, insert-baseline, bundle-baseline) resolve the **same** `ArchetypeId` — asserted three-way (critic-3 #2).
- **Component presence:** `world.has_component(dsl_e, id) == world.has_component(hand_e, id)` for every relevant `ComponentId` (ecs_master.rs:2036), including `UiName` for named nodes, `ChildOf`/`Children` per the post-link model.
- **Authored component values:** `world.get_component::<T>(dsl_e) == world.get_component::<T>(hand_e)` for each authored `T` (`UiLayout`, `UiSpacing`, `UiAlign`, `UiAbsolute`, `ContentSize`, `UiRoot` presence, `UiName` value). These are drain-order-independent.
- **Computed values (order-safe):** if `ComputedRect` *computed* values are compared, run layout to a fixed point on both trees first (Decision/critic-1 C3), then compare. Otherwise only presence of `ComputedRect` is asserted.
- **Hierarchy:** `Children::as_slice()` set-equality parent-by-parent (order unspecified per hierarchy/mod.rs; a P5a sort test owns order). `ChildOf` of each child points at the right parent.
- **Liveness:** `world.has_entity(e)` for every node after apply (ecs_master.rs:2002).

### 2. Command-count equivalence (`tests/ui_macro_cmd_count.rs`) — critic-2 #1 / critic-1 minor
- Drive the `ui!` path and the extended insert-baseline through queues whose lengths are probed; assert the macro issues **exactly** the same total of spawns + author inserts + injected inserts + `ChildOf` inserts node-for-node (no double-emitted `ComputedRect`, no extra migration). This is the direct Principle-1 regression guard.

### 3. Bundle-cache-PATH verification (`tests/ui_macro_cache.rs`) — critic-2 #1
- Spawn the canonical shape (set contains `{UiLayout, ComputedRect}`) via `ui!`. Assert (a) the `UiNodeBundle` `BundleTypeId`'s per-world cache slot **is populated** (the bundle fast path executed — via a `#[cfg(test)]` accessor on `boyko_ui` that queries the slot through the existing `bundle_archetype_id_for` path), and (b) all N same-shape `ui!` nodes share one `ArchetypeId`, equal to `spawn_via_bundle`'s archetype. This proves the *path*, not just the resulting archetype.
- **Negative control:** spawn a **re-pathed** `crate::components::UiLayout`+`ComputedRect` node via `ui!`; assert it resolves to the **same** archetype but document (and, if a probe exists, assert) it did **not** take the `UiNodeBundle` slot path — locking in the documented fast-path-loss behavior of Decision 2/9.

### 4. Single-window deep-tree link integrity (`tests/ui_macro_link_window.rs`) — critic-2 #2
- Build a 3+ level tree (grandparent/parent/child) in ONE `ui!` invocation → ONE apply window. Assert every `ChildOf` link materialized (no link silently dropped by the dangling-parent guard, commands.rs:250) and every `Children` collection has the right members after one apply. This gates the load-bearing ordering invariant.

### 5. Compile-FAIL (`tests/ui_macro_compile_fail.rs` + `tests/ui_macro_compile_fail/*.rs` + `.stderr`), via the existing `trybuild` harness (pattern from `bundle_compile_fail.rs`)
- `dup_name.rs` — two `#foo` → "duplicate ui name `foo`".
- `bad_field.rs` — `UiLayout { widht: ... }` → E0560 at `widht` (span forwarding).
- `bad_field_in_bundle_slot.rs` — a wrong-type literal in leading position so it lands in `UiNodeBundle.layout` → assert `.stderr` points at the **user's type**, not `UiNodeBundle` (critic-1 #6).
- `non_component.rs` — a non-`Component` literal → `T: Bundle` not satisfied at the user token.
- `unknown_name_ref.rs` — value-position `#bar` with no `#bar` declared → "unknown ui name `bar`".
- `bare_name_item.rs` — a bare `#title` as a body item → "a node reference must appear inside a component field".
- `name_too_long.rs` — a 61-char `#name` → "exceeds 60 bytes".
- `name_collides_commands.rs` — `#cmds` (default binding) → "collides with the commands binding" (critic-2 minor).
- `children_not_last.rs` — `children: [...]` followed by another component → "children must be the last clause".
- `children_twice.rs` — two `children:` clauses → error at the second.
- `children_eq.rs` — `children = [...]` → "expected `:` after `children`, found `=`".
- `inline_brace_node.rs` — a `{...}` node among items → "a child node must appear inside a `children: [ ... ]` clause".
- `bracket_array_item.rs` — a bare `[...]` among items → "expected a `children: [ ... ]` clause".
- `no_layout.rs` — a node with components but no `UiLayout` (e.g. `{ ContentSize { .. } }`) → "a ui node requires a `UiLayout` component" (Decision 8).
- `empty_node.rs` — `ui! { { } }` → "a ui node needs at least one component".
- `deep_nest_6.rs` (compile-PASS smoke) — a 6+-level nest compiles (recursion-depth smoke, critic-1 #5).
- README: regenerate `.stderr` via `$env:TRYBUILD="overwrite"`; gate `#[cfg(not(miri))]` (matches `bundle_compile_fail.rs:43`).

### 6. `-D warnings` compile-PASS (`tests/ui_macro_warnings.rs`) — critic-1 C1 / critic-3 minor
- A small module with `#![deny(warnings)]` (or built under the workspace `-D warnings`) containing: a single unnamed leaf; a single `#named` leaf whose handle is **never referenced**; a node with zero extra inserts; a multi-component bundle node; a 2-level nest. Must compile clean — proving no elided-lifetime, needless-borrow, unused-mut, or unused-variable lint escapes the expansion.

### 7. `UiName` unit tests (`tests/ui_name.rs` or in-crate `#[cfg(test)]`)
- `new`/`as_str` round-trip ASCII + multi-byte UTF-8 up to CAP; `len`/`is_empty`; `PartialEq`; `size_of == 64`; `const fn new` usable in const context.

### 8. Miri (`tests/miri_ui_macro.rs`, curated subset of #1 + #4)
- One small `ui!` tree + one apply window under Miri (the macro emits safe code, but the spawn/insert/`ChildOf` path it drives exercises `Bundle::for_each_component_bytes`'s `unsafe`; confirms the macro feeds well-formed inputs). One single-window 3-level tree (link-integrity under Miri). Mirrors the curated-Miri convention.

### Mandatory `debug_assert!`
- `UiName::new`: `debug_assert!(s.len() <= Self::CAP, "ui name exceeds CAP")` (the macro rejects over-length literals; this guards the P3 text path).
- `UiName::as_str`: `debug_assert!(self.len as usize <= Self::CAP)` before the slice.

## Integration

- **Affected modules:**
  - `boyko_macros/src/lib.rs` — add `#[proc_macro] pub fn ui` + a `ui` parser module (explicit recursive descent). No changes to existing macros.
  - `boyko_ui/src/components.rs` — add `UiName`.
  - `boyko_ui/src/bundles.rs` — NEW file, `UiNodeBundle`. `pub mod bundles;` in lib.rs.
  - `boyko_ui/src/lib.rs` — `pub use boyko_macros::ui;`, extend prelude with `ui`, `UiName`, `UiNodeBundle`.
  - `boyko_ui/tests/common/mod.rs` — extend `NodeSpec` with `name` + add `spawn_via_bundle` (Test #0).
  - `boyko_ui/Cargo.toml` — add `trybuild` as a dev-dependency (mirror `boyko_ecs`'s dev-deps). `boyko-macros` already a normal dep (Cargo.toml:13).
  - **Optional `#[cfg(test)]` accessor** on `boyko_ui` exposing the `UiNodeBundle` cache-slot query for Test #3 (test-only, no production surface).
- **No `boyko_ecs` changes** (locked — the macro only emits calls to existing `Commands`/`EntityCommands` API: `reserve_entity`, `entity`, `insert`, `add_child`).
- **Compatibility with Phase-8.5 cache:** `UiNodeBundle` is a standard `#[derive(Bundle)]` → gets a `BundleTypeId` + per-world `OnceLock<ArchetypeId>` slot automatically.

### Implementation plan (for the developer)
1. **Verify O1** (insert-on-reserved-id materializes the entity) by reading the Phase-11 `reserve_entity` + spawn/apply path; pick the confirmed Phase-B primitive (`entity(id).insert(base)` vs `spawn(base)` capturing into a binding). This gates Phase-B emission shape.
2. `boyko_ui/src/components.rs`: add `UiName` (struct + impl + `debug_assert`s). In-crate size/round-trip tests.
3. `boyko_ui/src/bundles.rs`: add `UiNodeBundle`; `pub mod bundles;` + prelude re-exports.
4. `boyko_ui/src/lib.rs`: `pub use boyko_macros::ui;`, extend prelude.
5. `boyko_ui/tests/common/mod.rs`: extend `NodeSpec` (`name`) + `spawn_via_bundle` (Test #0).
6. `boyko_macros/src/lib.rs`: implement `ui` — (a) explicit recursive-descent parser over `ParseStream` building `UiNode` with span capture + reserved-keyword handling; (b) macro-time validation pass (dup names, children-last/twice/=, empty/no-layout node, name length/collision, bare-name item, inline-brace/bracket-array items, unknown refs) accumulating `syn::Error` via `.combine()`; (c) two-phase codegen (Phase A reserve, Phase B materialize + set-based `UiNodeBundle` recognition + `quote_spanned!` at bundle-field sites + per-subtree-trailing standalone `add_child` + unused-name suppressors). Match house style (lib.rs:2792 absolute paths, lib.rs:2112-2126 keyed-clause parse for `commands:`).
7. `boyko_ui/Cargo.toml`: add `trybuild` dev-dep + optional `#[cfg(test)]` cache-slot accessor.
8. Tests #0–#8.

## Metrics and validation

- **Benchmarks:** `cargo bench` add `ui_macro_spawn_tree` (N-node tree via `ui!`) vs `ui_macro_spawn_tree_hand` (extended baseline). Target: macro adds zero overhead vs hand spawns (command-count-identical per Test #2). Warm-cache bench (same shape 10k times) confirms the `UiNodeBundle` cache amortizes archetype lookup (matches the `spawn_via_bundle` warm path).
- **Unit/integration tests:** equivalence (#1) + command-count (#2) + cache-path (#3) + single-window links (#4) are the gates; compile-fail (#5), `-D warnings` pass (#6), `UiName` (#7), Miri (#8) are mandatory.
- **Clippy:** `cargo clippy --all-targets -- -D warnings` clean — enforced by Test #6 plus the standard workspace gate. Emitted code: no elided-lifetime path annotation, no needless `mut`, no needless binding, no unused `#named` handle warning.

## Out of scope for P2 (seams left)

- **`.ui` text format → P3.** The grammar is the surface P3 mirrors. **P3 seam caveats (critic-3 minor):** the SHARED artifact is the `UiNode` TREE SHAPE + `UiName`, **not** the value grammar — the text face parses a restricted literal grammar (numbers, enum idents, named units) into the same component values (it cannot evaluate arbitrary Rust exprs like `..Default::default()` / `Unit::Px(..)`), and resolves `#name` refs via a name→`Entity` table (not Rust `let`-bindings). Stating this now prevents a P3 surprise.
- **Hot-reload diffing → P3.** `UiName` is the diff key now; the diff algorithm (compare `UiName` columns, patch changed components) is P3. `UiName` is `Copy`/`Eq`, column-comparable.
- **Bindings / typed actions / event handlers → P4.** No `on:press`, no `bind:`, no closures (Principle 1 / no `dyn Fn`). `#name` value-position refs (parse-supported now) give the `Entity` handles P4 will attach actions to.
- **Widget vocabulary (`Panel`/`Button`/`Text`) → P6.** P2 uses arbitrary component literals. The set-based canonical-bundle recognition (Decision 2) is the hook where named widgets would slot in.
- **`StackIndex` auto-assignment by declaration order → P5a.** Not in P2 (sibling order unspecified). Additive later (`.insert(StackIndex(i))` per child), no grammar change.
- **Non-tree links (`#name` value-position into a component field):** parsing/resolution specified and supported now (two-phase makes forward refs work); no canonical UI component consumes an `Entity` ref yet — forward seam for P4/relations.

## Open questions

1. **O1 — `entity(id).insert(base)` on a reserved-but-unspawned id materializes the entity?** The two-phase lowering assumes the Phase-11 "reserve id, then build it via the first structural command" pattern materializes the entity. If the first command on a reserved id must be a `spawn` (not `insert`), Phase B emits `cmds.spawn(<base>)` and the reserved id is threaded differently. **Resolution:** the developer verifies this in step 1 before coding Phase B. (Recommendation: this is the established Phase-11 escape-hatch pattern — `reserve_entity` exists precisely so an id can be threaded before spawn — so `insert`-as-first-command is expected to materialize; verify to be certain.)
2. **`UiName::CAP` = 60 (→ 64 B struct).** Covers realistic names; longer = compile error. Bump to 124 (→ 128 B, two cache lines) only with evidence (doubles the opt-in column footprint). (Recommendation: 60.)
3. **Multiple top-level nodes → tuple of `Entity`.** Free and useful for sibling roots. (Recommendation: allow.)
4. **No-`UiLayout` node = error (Decision 8) vs inject `UiLayout::default()`.** Plan chooses error (a layout-less node is almost always a mistake). If a designer use-case for a rect-only/marker node emerges, switch to injection (additive, no grammar change). (Recommendation: error now.)

## Changes from review

**Critical fixes:**
- **C1 (typed `&mut Commands` reborrow trips `-D warnings`):** removed the `let __ui_cmds: &mut ::...::Commands = &mut cmds;` line entirely; Phase B calls methods directly on the user's `cmds`/override ident (no elided-lifetime path annotation, no needless borrow). Added **Test #6** (`-D warnings` compile-PASS) to lock it in beyond `trybuild`.
- **C2 (forward `#name` refs failed to compile under pre-order DFS):** introduced **Decision 6 — two-phase lowering** (Phase A reserves **all** entity ids via `reserve_entity` (commands.rs:242, verified) before Phase B materializes), so every `#name` local is in scope before any value-position read → forward and backward refs both resolve. `#name` reduced to a node-prefix only (value refs only inside field exprs).
- **C3 (single-window `ComputedRect`-value equivalence depends on drain order; linkage primitive mismatch):** corrected the equivalence claim to **STATE** equivalence (not command-order). The gate asserts **authored** values + archetype + presence + `ChildOf`/`Children` membership; `ComputedRect` *computed* values are only compared after running layout **to a fixed point** on both trees (order-independent). Documented that the baseline uses `set_parent` (in-chain) and the macro batches `add_child` — same final state, order not claimed.
- **Cache-path blindness (critic-2 #1):** **Decision 7** + **Test #3** assert the `UiNodeBundle` cache **slot is populated** (the fast path executed) and three-way archetype equality across DSL / insert-baseline / bundle-baseline, plus a re-pathed **negative control**. The gate no longer relies on archetype-equality alone.
- **Single-window link dropping (critic-2 #2):** elevated "every parent spawns before its descendants' `ChildOf` insert" to a documented **correctness invariant** (the dangling-parent guard at commands.rs:250 silently drops otherwise — verified). Guaranteed by Phase-B strict pre-order + per-subtree-trailing links; gated by **Test #4** (deep single-window tree).

**Major fixes:**
- **Borrow-shape (critic-1 #4):** specified exact emission granularity — one standalone `add_child` per statement, no cross-node `EntityCommands` chaining; `Entity` ids captured as plain `Copy` locals. Added an equivalence shape with a node reading a sibling `#name`.
- **Parser enforceability (critic-1 #5, critic-3 #4):** parser is now explicit recursive descent over `ParseStream` (peek for `#`/`children`/`commands`), NOT comma-split-then-`syn::Expr`; every named error (children-not-last/twice, empty, `=`-vs-`:`, inline-brace-node, bracket-array item, bare-name item) has a targeted diagnostic. Added the corresponding compile-fail cases + a 6-level nest smoke.
- **Span fidelity at bundle-field sites (critic-1 #6):** **mandated** `quote_spanned!(expr.span()=> #expr)` at the `UiNodeBundle` field positions; added `bad_field_in_bundle_slot.rs` asserting the error points at the user's type.
- **Token-overloading of `#name` (critic-3 #1):** dropped the standalone `#IDENT` body-item production; `#name` is a node prefix only; value refs only inside field exprs; bare-`#name`-item is a macro error.
- **Positional bundle recognition (critic-3 #2):** changed to **set-based** (`{UiLayout, ComputedRect}` in any order); added a shuffled-order equivalence shape + three-way (DSL/insert/bundle) archetype assertion.
- **Equivalence baseline lacked `UiName`/bundle path (critic-2 #3, critic-3 minor):** **Test #0** extends `NodeSpec` with `name` and adds `spawn_via_bundle`, making the `#named` and bundle-path rows constructible and named-vs-named.
- **Post-link archetype model (critic-2 #4):** documented in Decision 5 — the canonical bundle is the *base*, not the final archetype; linking adds `ChildOf`/`Children`; the gate compares post-link archetypes.
- **Linkage-primitive prose (critic-3 #3):** corrected "exactly equivalent" to STATE-equivalent; justified batching `add_child` over interleaving `set_parent`.

**Minor fixes:**
- **No-`UiLayout` dead node (critic-2 minor):** **Decision 8** — a node without a `UiLayout` literal is a macro error (`no_layout.rs`), instead of silently injecting only a rect.
- **Emitted-binding lint hazards (critic-1 minor, critic-3 minor):** no needless binding for trivial nodes; `let _ = &name;` suppresses unused `#named` handles; `#name` colliding with the commands binding is a macro error (`name_collides_commands.rs`). All covered by Test #6.
- **Command-count assertion (critic-1 minor):** **Test #2** asserts exact command counts (catches a double-`ComputedRect` insert), not just archetype equality.
- **Reserved keywords (critic-3 minor):** `children`/`commands` documented as reserved context keywords; path-qualify a same-named type.
- **P3 seam caveats (critic-3 minor):** added — shared artifact is the tree shape + `UiName`, not the value grammar; text face uses a name→`Entity` table.
- **Error-message style (critic-1 minor):** align casing/format with existing `boyko_macros` `syn::Error` messages.

**New open question:** O1 (does `entity(id).insert(base)` materialize a reserved id, or must Phase B use `spawn`) — flagged for developer verification in implementation step 1, with a recommended resolution.

Relevant files: `D:\claude\BoykoEngine\crates\boyko_macros\src\lib.rs` (proc-macro house style: absolute-path emit ~2792, keyed-clause parse ~2112, `__bundle_field_` hygiene ~2525), `D:\claude\BoykoEngine\crates\boyko_ui\src\components.rs` (`UiName`), `D:\claude\BoykoEngine\crates\boyko_ui\src\lib.rs` (prelude re-export), `D:\claude\BoykoEngine\crates\boyko_ui\tests\common\mod.rs` (baseline `spawn` uses `ec.set_parent(p)` at :127; extend per Test #0), `D:\claude\BoykoEngine\crates\boyko_ecs\tests\bundle_compile_fail.rs` (trybuild harness), `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\commands.rs` (`reserve_entity`:242, `add_child`:262, `Commands<'s>`:97), `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\hierarchy\commands.rs` (dangling-parent guard `child_of_on_insert`:234-254, link migration :129-136), `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` (gate APIs: `entity_archetype_id`:2020, `has_component`:2036, `get_component`:1891, `has_entity`:2002).