# `boyko_reflect` — deep design analysis

Snapshot 2026-06-15, branch `ecs`. Produced by a research → architect →
critic(×2 independent) pipeline (bevy_reflect / flecs / EnTT / Unity DOTS /
Unreal study + boyko source grounding). This document folds the two critique
rounds' CRITICAL/MAJOR findings back in as **resolved decisions**, and marks the
one genuinely-open scope fork for the owner.

Status: **DESIGN — not implemented.** ~~No code exists yet (`grep` confirms no
`Reflect` trait / serde in tree).~~ **Half struck 2026-08-21.** The *literal* half
still holds — there is no `trait Reflect`, no `derive(Reflect)`, and no `serde`
anywhere in `crates/` (verified: zero hits for either identifier; zero `serde` in
any `Cargo.toml`; `grep -rn boyko_reflect` returns only doc/comment mentions and no
manifest edge). The *implied* half — "nothing reflection-shaped exists" — is **false**.
A functionally overlapping accessor layer shipped **six days after this snapshot**:
`#[derive(Bindable)]` / `BIND_ACCESSORS` (commit `8a11f31b`, 2026-06-21), which is
roughly 80 % of §3.2's field model, already in the **release** binary. See **B.1**.

---

> ## Revision 2026-08-21 — re-grounded against `feat/reflection`
>
> This document was a snapshot of branch `ecs` at **2026-06-15**. Two months of
> engine work have landed since. This revision:
>
> 1. **Strikes** every claim the tree has falsified (`~~struck~~`, with the
>    correction and its evidence beside it — never a silent rewrite, so a reader of
>    an older review can still find what happened).
> 2. **Corrects citations in place** for claims that merely moved
>    (`component_registry.rs` → `component_registry/mod.rs`, and friends).
> 3. **Records the §6 scope fork as TAKEN — option (B)** — with its reason, and
>    with its *justification re-grounded*, because the tree's component population
>    is not the one §6 assumed. Reversible by construction.
> 4. **Adds Appendix B** for surface this document predates: the shipped
>    `BindAccessor` table, the three-way `StorageKind` × three-way `ResidencyKind`
>    × dynamic-tag refusal matrix, Aether-generated components, the measured
>    ship-gate instrument, and the fixed-size-array coverage gap.
>
> **New surface goes in Appendix B, never smuggled into §1–§8 or Appendix A.**
> The central finding of §0/§2 — that "reflection only in a Debug build" is not a
> compiler property, and that the mechanism is an optional crate behind a Cargo
> feature plus a CI absence gate — **stands unchanged and was not re-litigated.**

---

## 0. The one-paragraph verdict

A fast, separate-crate, build-gated reflection layer is feasible and the core
data model genuinely beats `bevy_reflect`. **But the naive "reflection only in a
Debug build" gate is not robustly achievable** — `cfg(debug_assertions)` cannot
control whether a crate is in the dependency graph, and is on for every plain
`cargo build`. The correct mechanism that delivers the *same outcome the owner
wants* ("on while developing, literally absent in the shipped game") is a **Cargo
feature on an `optional` crate**, enabled via `#[cfg_attr(feature = "reflect",
derive(Reflect))]`. "Debug-only" then means "the dev/editor build enables the
feature; the ship build does not," which is build hygiene + a CI symbol-absence
gate, not an inherent compiler property.

---

## 1. Scope: TWO crates, strict purpose split

| Crate | Purpose | Ships in release? | Depends on |
|---|---|---|---|
| **`boyko_reflect`** (this design) | Runtime field introspection **by name/index**: editor / inspector / entity browser / live-tuning / debug-dump / prefab authoring | **No** by default (feature-gated; absent from the dep graph) | `boyko_ecs` |
| **`boyko_serialize`** ~~(future, already deferred per `REMAINING-GAPS.md`)~~ — **SHIPPED** (corrected 2026-08-21) | Compile-time **codegen** (de)serialization for save/load, replication, baked prefabs | **Yes** | `boyko_ecs` only — **NEVER `boyko_reflect`** |

> **Tense correction (2026-08-21).** `boyko_serialize` is no longer future. It landed
> **2026-06-16** (commit `601e2247`, one day after this snapshot), ships
> `save.rs` / `load.rs` / `format.rs` / `error.rs`, carries its own `pob_throughput`
> bench, and is a workspace **default-member**
> (`crates/boyko_serialize/`, root `Cargo.toml` `default-members`,
> `docs/FEATURE_MAP.md:112`). The design consequence is **favourable**: the
> directional rule below is no longer a doc-only aspiration — it is asserted at the
> source, in the shipping crate's own manifest and lib header:
>
> * `crates/boyko_serialize/Cargo.toml:6~-10` — depends on `boyko_ecs`, the diagnostic
>   seam and `std` only, *"never `boyko_reflect` (the codegen-not-reflection invariant)"*.
> * `crates/boyko_serialize/src/lib.rs:6~` — *"This crate never depends on `boyko_reflect`."*
>
> A whole-tree `grep -rn boyko_reflect` returns **only** those two comments plus this
> document. No manifest edge to `boyko_reflect` exists anywhere.

**Why this makes "Debug-only" a *correct*, non-limiting choice (not a deferral
that hides the problem):** the research disproved the naive "reflection = dev
tooling" premise *as a monolith* — Unreal ships `UClass`/`FProperty` in shipping
builds for GC/replication/serialization. The rescue is the **scope split**: in
Rust you get via compile-time codegen (`serde`-style) what Unreal can only get
from runtime reflection. Decisive precedent: **`bevy_replicon` dropped reflection
and switched to plain serde** for replication. Of the seven reflection consumers
surveyed, **six survive Debug-only reflection** via codegen or bake-to-data; the
one irreducible exception is *shipped, name-keyed scripting* (a script VM
addressing component fields by string at runtime) — which is **out of scope for
v1**, and if ever needed, that one target enables `boyko_reflect` in its release
build via the same feature (the gate already supports "release WITH reflection"
for the legitimate editor/scripting case).

**The single enforceable invariant that keeps this honest:**
`boyko_serialize` and any shipping crate **must not depend on `boyko_reflect`.**
A CI/lint gate enforces it.

**What `boyko_reflect` IS:** by-name/by-index field get/set on *live, concrete,
known-type* component instances, reached through boyko's existing
raw-by-`ComponentId` access.
**What it is NOT:** (a) not a hot-path facility (forbidden by *compilation*, not
convention); (b) not the save/load engine; (c) not `bevy_reflect`'s
`DynamicStruct`/`FromReflect` machinery — no runtime-synthesized types, which is
the entire `Box<dyn>`-per-field allocation class we refuse; (d) not
reflection-as-components (rejecting the flecs co-located model).

---

## 2. The gating mechanism (resolved — was the #1 critical finding in BOTH critiques)

### Decision: feature-gated optional crate, applied at the derive site by the consumer. No `debug_assertions`. No marker feature on `boyko_ecs`.

The original draft proposed `#[cfg(any(feature = "reflect", debug_assertions))]`
inside the derive's expansion. **Both critics independently flagged this as
non-compiling for the default dev build:** `debug_assertions` is on for every
plain `cargo build`, so the derive would emit `boyko_reflect::…` paths into a
crate that does not have `boyko_reflect` in its deps (the crate is only pulled by
the feature) → hard `E0432`/`E0433` unresolved-crate error. You cannot
simultaneously have "works in a plain debug build with no opt-in" and "absent
from the dep graph unless a feature is on." They are contradictory.

**The clean, idiomatic resolution (serde-style optional derive):** the *consumer*
writes

```rust
#[derive(Component)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
struct Position { x: f32, y: f32 }
```

- `reflect` is a feature on the **consumer's own crate** that pulls the optional
  `boyko_reflect` dependency.
- Feature **off** → the `derive(Reflect)` is *never applied* → zero tokens, zero
  `boyko_reflect::` paths, nothing to resolve, the crate is absent. True
  zero-compile / zero-binary / zero-symbol. This is "the symbol does not exist,"
  not "the branch isn't taken."
- Feature **on** (dev/editor build) → full reflection, in an optimized binary if
  desired (the legitimate release-editor case).
- The `Reflect` derive itself is then **dead simple** — it always emits full
  reflection code and needs *no* internal `cfg`, because gating is hoisted to the
  `cfg_attr` at the use site.

> **Universality struck 2026-08-21 — "the *consumer* writes it" is not always
> possible.** The mechanism is sound; the claim that it covers every component is
> not. There is now a class of components for which ~~the consumer writes
> `#[cfg_attr(feature = "reflect", derive(Reflect))]`~~ **cannot be written at all,
> because the consumer never writes the struct**: components declared inside an
> `aether! { component Foo { … } }` block. `aether_lang`'s expander emits the
> `#[derive(Component)]` item itself, and its `ComponentDef` AST has **no attribute
> passthrough** — there is no syntactic slot for the user to put an attribute in.
> The macro must add it. Full analysis and the concrete three-file change: **B.5**.
> (`crates/aether_lang/src/expand.rs:166-195`, `crates/aether_lang/src/ast.rs:268-280`.)
> A.5's `#[reflect]` helper-attribute form inherits the same gap for the same reason.

This also resolves the **marker-feature footgun** (the draft put a `reflect`
marker feature on `boyko_ecs`; a critic showed workspace feature-unification could
then flip it on for a ship binary's `boyko_ecs`). With the gate on the consumer's
feature, `boyko_ecs` carries **no reflect feature at all** and the directional
rule holds unconditionally.

> **Narrowed 2026-08-21 (second pass): "no feature on a SHARED crate" is too strong, and taken
> literally it forbids the dogfood.** The footgun is real and `boyko_ecs` still carries no
> `reflect` feature — but the property that defeats unification is *"nothing enables it"*, not
> *"nobody declares it"*. Every dogfood target this document names (`Transform`, `Name`,
> `Visibility`, `GpuTransform3D`, `EmitterActive`) is defined in a shared engine crate, so the
> §2 sentence *"the consumer writes the opt-in"* has no consumer to write it — the third
> variant of the same gap **B.5** found for Aether. The mechanism, its five-clause census rule,
> and the in-tree precedent that shows the shape works (`hwrt`, three shared crates deep, in no
> ship build) are in **B.12**.

```toml
# the consumer (editor_app / game_components) Cargo.toml
[dependencies]
boyko_ecs     = { path = "../boyko_ecs" }
boyko_reflect = { path = "../boyko_reflect", optional = true }

[features]
default = []
reflect = ["dep:boyko_reflect"]
```

- **Ship build**: `cargo build -p game_app --release` (no `reflect`) → `boyko_reflect`
  not in the resolved closure, not compiled, no symbols.
- **Editor build**: `cargo build -p editor_app --release --features reflect`.
- **Dev convenience**: developers who want reflection always-on locally enable
  `reflect` in their normal dev invocation (or a dev-only wrapper crate). There is
  no *automatic* `cfg(debug_assertions)` switch — that was the unsound part.

### Honest caveat on "zero release cost" (was critical finding C2)

Zero cost is a property of *the shipped artifact's resolved feature closure*, not
an inherent compiler guarantee. Cargo **feature unification** is per-invocation
across the selected packages; ~~a workspace-wide `cargo build`/`cargo bench` where
any member (e.g. `bench_bevy_vs_boyko`, `boyko_demo`) enables `reflect` unifies it
ON for the ship crate too.~~ Resolver v2 (edition 2024) does **not** de-unify plain
workspace deps. Therefore:

- The 0%-cost claim is **demoted to**: *zero cost in a correctly-partitioned ship
  build, enforced by a CI gate as a deliverable* — ~~`cargo tree -p game_app
  --release` must show `boyko_reflect` absent, and a symbol-absence check must pass
  on the exact ship artifact.~~ (both halves corrected below and in **B.6**)
  `reflect` must never appear in any `default` features nor in any crate in the ship
  binary's dependency closure.
- The Wave-5 hot-path bench asserts "no delta on the **steady-state query/spawn
  inner loop**," not "no delta" (first-touch `TYPE_INFO` registration is real cold
  cost when the feature is on, and must not be mistaken for a hot-path regression).

> **Severity inverted 2026-08-21 — the unification trigger is now the DEFAULT, not
> an opt-in.** The mechanism described above is unchanged and still correct; what
> changed is *how easy it is to fire*. The 2026-07 build-gate audit added an explicit
> `default-members` list to the root `Cargo.toml` naming **every** member **plus the
> root package `"."`** — deliberately, because the root is also a package
> (`boyko-engine`), so a bare `cargo check --all-targets` used to compile a
> `println!` and report success in ~0.2 s while every engine crate went unchecked
> (measured: the bare form found 0 errors where `--workspace` found 4).
>
> Consequence for reflection: **a bare root-level `cargo build` now selects every
> crate.** So if any member ever enables `reflect`, unification turns it on for the
> ship crate too — with **no `--workspace` flag needed to cause it**. The wording
> *"a workspace-wide build"* understates the exposure: **there is no non-workspace-wide
> root build any more.** The mitigation ("a correctly-partitioned ship build") is
> therefore not one safe invocation among several — it is the **only** safe form.
>
> Partially offsetting: CI already de-selects the two crates this section names, on
> every leg — `--exclude boyko_demo --exclude bench-bevy-vs-boyko`
> (`.github/workflows/ci.yml:62, :87-89, :129, :167, :191`). And the hosts named here
> are hypothetical: **`game_app` / `editor_app` do not exist.** The real hosts are
> `boyko_app`, `boyko_demo`, and the root `boyko-engine` package. Wave 0 must pick
> the real ship target before it can write the gate.
>
> **The `cargo tree` half is the load-bearing half; the symbol check is
> corroboration, not proof** — and it has a link-configuration precondition this
> document never states. The tree has since *built and measured* this exact
> instrument for another subsystem, and recorded the ways its naive form cannot
> fail. See **B.6**.

---

## 3. The fast core (the part that genuinely beats bevy_reflect)

### 3.1 Registry: dense `ComponentId`-indexed table — the real win

bevy resolves a type via `HashMap<TypeId, TypeRegistration>` **plus** a second
`HashMap<TypeId, Box<dyn TypeData>>` = two hash probes + a downcast per access.
boyko has dense, 0-based `ComponentId`, so:

```rust
// boyko_reflect::registry — mirrors the proven LAYOUTS / HOOKS / SERIALIZE /
// BIND_ACCESSORS write-once OnceLock discipline (component_registry/), but lives
// in boyko_reflect.        [CORRECTED 2026-08-21 — was "HOOKS / STORAGE_KIND / LAYOUTS
//                           (component_registry.rs)"; see the two notes below]
use boyko_ecs::…::MAX_COMPONENTS;          // IMPORTED, never redeclared (W5)

static REFLECT: [OnceLock<&'static TypeInfo>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

#[inline]
pub fn type_info_of(id: ComponentId) -> Option<&'static TypeInfo> {
    REFLECT.get(id.0)?.get().copied()      // one bounds-checked acquire-load + branch
}

pub fn install_type_info(id: ComponentId, info: &'static TypeInfo) {
    debug_assert!(id.0 < MAX_COMPONENTS);   // same bounds discipline as sibling writers
    assert!(id.0 < MAX_COMPONENTS);         // release guard (W5)
    let _ = REFLECT[id.0].set(info);        // write-once, lock-free, idempotent
}
```

> **Two citation corrections, one cosmetic and one substantive (2026-08-21).**
>
> **(a) Cosmetic — the file moved.** ~~`component_registry.rs`~~ is no longer a file.
> Commit `9b84436c` (2026-07-09) *"split the component_registry god-module into a
> directory module"*: it is now `component_registry/` with `mod.rs`, `clone.rs`,
> `required.rs`, `serialize.rs`, `tags.rs`. Every item kept its exact
> `component_registry::…` path via `pub use` re-exports, so **no import in this
> design changes** — only the citation. `MAX_COMPONENTS` is still `pub` and still
> `512`, now at `component_registry/mod.rs:61`; W5's "IMPORTED, never redeclared"
> rule is unaffected and still importable.
>
> **(b) Substantive — `STORAGE_KIND` was never an `OnceLock` table.** It is
> `[AtomicU8; MAX_COMPONENTS]` with `Relaxed` ordering
> (`component_registry/mod.rs:373-374`), and so is its sibling
> `RESIDENCY_CLASS` (`:501-502`). That is deliberate, and the reason is stated at the declaration:
> *"`Relaxed` is sufficient: the kind is a registration-time, write-once datum with
> **no payload published through it**."* An `AtomicU8` is the right shape for a
> classification byte and the **wrong** shape to copy for a `&'static TypeInfo`
> table, which *does* publish a payload and therefore needs the release/acquire
> edge an `OnceLock` provides.
>
> The genuine `[OnceLock<T>; MAX_COMPONENTS]` payload tables are **`LAYOUTS`** (`:206`),
> **`HOOKS`** (`:224`), and — new since this snapshot — **`SERIALIZE`**
> and **`BIND_ACCESSORS`** (`component_registry/serialize.rs:277`). The last two are
> the *exact* shape §3.1 sketches, which **strengthens** the design rather than
> weakening it: the pattern is now four instances deep, not two.
>
> **(c) Signature convention.** In-tree installers take **`component_id: usize`**,
> not `ComponentId` — `install_bind_accessor(component_id: usize, acc: BindAccessor)`
> (`component_registry/serialize.rs:299~-308`), `get_bind_accessor(component_id: usize)` (`:287`). The
> reason is that the derive expansion calls them as `…::component_id().0`. Match the
> convention: `install_type_info(component_id: usize, info: &'static TypeInfo)`.

**Registration: lazy first-call via the existing `component_id()` `OnceLock`
closure — NOT `linkme`/`inventory`.** The derive appends, beside the existing
`if Self::HAS_HOOKS { install_hooks }`, a `boyko_reflect::install_type_info(…)`
call gated by a new `const IS_REFLECT`. This inherits boyko's deterministic,
zero-before-main, write-once registration and **sidesteps the entire
`linkme` + `--gc-sections` dead-strip / init-order hazard class**. A reflectable
component is always `component_id()`-touched before it can enter an archetype, so
no type is missed.

**Win vs bevy:** one array index + one acquire-load vs two SipHash/aHash probes +
downcast. This is the strongest, defensible advantage and both critics endorsed
it.

### 3.2 Field model: offset + flat fn-pointers (no per-field `&dyn`, no `Box`)

> ⚠️ **UNRESOLVED FORK, opened 2026-08-21 — this model is ~80 % built and SHIPPING.**
> `#[derive(Bindable)]` / `BIND_ACCESSORS` (commit `8a11f31b`, 2026-06-21) is a
> per-`ComponentId` `[OnceLock<T>; MAX_COMPONENTS]` table of flat `fn` pointers with
> by-`u8`-index field access and a `field_id(name) -> Option<u8>` resolver — i.e. the
> shape below, minus write, minus nesting, minus a kind tag. It lives in `boyko_ecs`
> and is read by `boyko_ui` **in the release binary**. The design must take a
> position — extend it into `TypeInfo`, or run a second parallel table and justify the
> duplication — **before Wave 1**. Full comparison and the fork's two horns: **B.1**.
> `TypeInfo.stable_name` below has a *separate* duplication problem: see **B.2**.

```rust
#[repr(C)]
pub struct TypeInfo {
    pub type_name:   &'static str,        // std::any::type_name — diagnostics only
    pub stable_name: &'static str,        // ⚠️ 2026-08-21: this datum ALREADY EXISTS in
                                          //    boyko_ecs (SerializeInfo.stable_name +
                                          //    STABLE_NAME_INDEX). Do not re-declare it. See B.2.
    pub type_id_fn:  fn() -> TypeId,      // TypeId::of is NOT const → fn-ptr, never a static field (W1)
    pub size:        usize,
    pub align:       usize,
    pub fields:      &'static [FieldInfo],// 'static slice baked by the derive — no Vec
    pub kind:        TypeKind,            // Struct | TupleStruct | Enum | Opaque
    pub default_in_place: unsafe fn(*mut u8),
    pub drop_in_place:    Option<unsafe fn(*mut u8)>,
}

#[repr(C)]
pub struct FieldInfo {
    pub name:        &'static str,        // load-bearing: the deserialize key (NOT strippable, O4)
    pub offset:      usize,               // core::mem::offset_of!(T, field) — const
    pub type_id_fn:  fn() -> TypeId,      // (W1) — fn-ptr, not a static TypeId
    pub kind:        ValueKind,           // Bool|U8..|F64|EntityId | Nested | Opaque
    pub get:       unsafe fn(*const u8) -> Scalar,
    pub set:       unsafe fn(*mut u8, Scalar) -> bool,   // returns false on kind mismatch (load-bearing, see §5)
    pub serialize: unsafe fn(*const u8, &mut dyn Sink),  // the ONE dyn, serialize-only
    pub debug_fmt: unsafe fn(*const u8, &mut core::fmt::Formatter) -> core::fmt::Result,
}
```

`Scalar` is a ~16 B `#[repr(C)]` POD tagged union (`bool/u8..u64/i8..i64/f32/f64/
EntityId`), plain `Copy`, **no heap** (EnTT `meta_any`-with-SOO idea, specialized
to POD). Its tag *doubles as* the `ValueKind` guard on `set`, so the tag is not
extra cost.

### 3.3 Performance honesty (was critical finding C2-arch)

- **The genuine win is the registry lookup** (array vs double-hash). Keep this
  claim.
- **The field get/set path is NOT "<5 ns monomorphic, no dispatch."** It is one
  *indirect* `fn`-pointer call (`(f.get)(fp)`) returning a `Scalar` — i.e. we
  traded bevy's *vtable* indirection for a *fn-ptr-table* indirection, one word
  narrower but the same class. It is **far cheaper than bevy** (no `Box`, no
  `DynamicStruct`, no downcast, no double-hash) but the original "monomorphic,
  branchless, <5 ns" wording is withdrawn.
  **Vindicated 2026-08-21 by the shipped analogue:** `BindAccessor` is literally two
  bare `fn` pointers dispatched through a `match` on a `u8` field index, and it is
  documented in-tree as a **cold, change-gated** path — *"read ONLY from the
  change-gated `boyko_ui` bind-apply path — never on a still frame or the per-frame
  hot path"* (`component_registry/serialize.rs:283~-287`). Nobody in this tree
  pretends this shape is free. The withdrawal was correct.
- **Two APIs:** the type-erased `get_field(...) -> Scalar` (what an editor needs —
  it does not know `T`), and an optional genuinely-monomorphic
  `get_field_typed::<T>()` (reuses the existing `get_component_typed`) for the rare
  case a caller knows `T` at compile time.
- **Mandatory bench (Wave 5):** `boyko get_field` vs a *hand-written
  bevy_reflect-shaped baseline* (`HashMap<TypeId>` + `&dyn Reflect` +
  `downcast_ref`). The "feature on vs off" delta proves *zero-hot-path / zero
  binary*; it does **not** prove *beats-bevy*. Both benches are required.

**Allocation audit (principle #5):** the only allocations are (a) the cold
`TypeId/name → ComponentId` intern (built once at editor/load setup, and it lives
in `boyko_ecs`, see §4), and (b) caller-provided, reused sinks. `add_default`
adds **no** new allocation site (§4). Zero allocation on get/set/enumerate; the
`fields` slice is `&'static`.

---

## 4. ECS glue (reuses existing raw-by-id access)

```rust
// ⚠️ STRUCTURALLY BLIND as specified — 2026-08-21. The archetype signature is no
//    longer the set of an entity's components: Bitset AND Dense ids are excluded
//    from every signature. Enumeration needs THREE sources, not one. See B.3.
pub fn components_of(ecs: &EcsMaster, e: Entity) -> Option<&[ComponentId]>;     // archetype signature
pub fn fields_of(id: ComponentId) -> Option<&'static [FieldInfo]>;             // baked 'static slice
pub fn get_field(ecs: &EcsMaster, e: Entity, id: ComponentId, f: usize) -> Option<Scalar>;
pub fn set_field(ecs: &mut EcsMaster, e: Entity, id: ComponentId, f: usize, v: Scalar) -> bool;
pub fn get_field_by_name(/* … */) -> Option<Scalar>;   // linear scan of the few fields
pub fn set_field_by_name(/* … */) -> bool;
pub fn add_default(ecs: &mut EcsMaster, e: Entity, id: ComponentId) -> bool;    // editor "add component"
pub fn remove(ecs: &mut EcsMaster, e: Entity, id: ComponentId) -> bool;
```

`get_field` builds on the confirmed `get_component_raw` (arena-rooted
`SharedReadWrite` provenance): `base = get_component_raw(e,id)` → `info =
type_info_of(id)` → `fp = base.add(f.offset)` → `(f.get)(fp)`. ~~`add_default` /
`remove` route through the **existing** structural insert/remove (the same path
`Commands` uses), so they inherit hooks/observers/change-detection for free and
are already off the iteration hot path.~~

**`get_component_raw` re-confirmed and WIDENED (2026-08-21).** It is still at
`ecs_master/component_api.rs:176`, with `get_component_raw_mut` at `:253` and
`set_component_raw` at `:444`, and it is still arena-rooted exactly as described.
Since the snapshot it grew (a) a **dense branch** routing to `dense_get_raw` (`:76`)
and (b) a null-column check that safely covers device-backed columns. It is a
**better** foundation than at snapshot, not a worse one.

> **`add_default` / `remove` — struck 2026-08-21: half-built, half-BLOCKED.** The
> claim was that these "route through the *existing* structural insert/remove."
> Today a genuine **by-id** structural migration path does exist — it did not in this
> form at snapshot — and it is already driven end-to-end by the public
> `EcsMaster::add_tag` (`ecs_master/tag_api.rs:130`) / `remove_tag` (`:200`):
>
> | helper | `commands/migration_helpers.rs` | visibility |
> |---|---|---|
> | `merged_archetype_id_dyn` | `commands/migration_helpers.rs:1230` | **`pub(crate)`** |
> | `without_ids_archetype_id` | `:1305` | **`pub(crate)`** |
> | `migrate_entity_attach_ids` | `:1372` | **`pub(crate)`** |
> | `migrate_entity_detach_ids` | `:1658` | **`pub(crate)`** |
> | `retag_in_place` | `:1922` | **`pub(crate)`** |
>
> That is exactly the seam `add_default` / `remove` need — and **every one of them is
> `pub(crate)` in `boyko_ecs`, so `boyko_reflect`, an external crate, cannot call any
> of them.** "Routes through the existing path" is therefore not a free inheritance;
> it is an **un-scoped, required Wave-3 deliverable: a public by-id structural seam
> on `EcsMaster`.**
>
> **This is the one place a dev-only feature widens a SHIPPING crate's public
> surface**, because the seam lands in `boyko_ecs`, not in the optional crate. That
> must be stated in the plan rather than discovered in Wave 3. It does not breach the
> directional rule (the seam names no reflection type — `add_tag`/`remove_tag` prove
> the signature can be pure `Entity` + `ComponentId`), but it is real permanent API
> surface, and the design should say whether it is justified on its own merits
> (a by-id structural API is independently useful to scene loading and the editor) or
> only as reflection scaffolding.

**Resolutions folded in:**
- **`name`/`TypeId` → `ComponentId` resolution lives in `boyko_ecs`** (alongside
  `LAYOUTS` / `type_name` / `TAG_NAMES`), not in the Debug-only crate — because the
  *shipping* save/load consumer needs it and is forbidden from depending on
  `boyko_reflect` (M2). Reflect *uses* it; it does not *own* it.
  **VINDICATED and ALREADY BUILT (2026-08-21)** — the placement call was right, and
  the shipping save/load consumer went and implemented it independently. Only the
  *tense* is stale: this is no longer a thing to design. See **B.2**.
- **Resolution happens once per distinct type per load**, materialized into a dense
  `Vec<ComponentId>` keyed by the save file's local type table; the per-entity loop
  is then pure id-indexed byte copies — **never** a `Mutex::lock()` + hash probe per
  component per entity (W2).
- **`add_default` adds no bespoke allocation:** it routes default-construction
  through the existing structural-insert storage (alignment discipline already
  proven there); if a scratch buffer is ever unavoidable it must be `align`-correct
  (`Layout::from_size_align(size, align)`) and the bytes **moved** (forgotten after
  the `copy_nonoverlapping`, not dropped) to avoid double-free (M7/W3).
- **ZST / bitset-storage components (M6):** the derive **refuses `Reflect` on
  `#[component(storage="bitset")]`** types (they have no data pointer and are
  filtered out of archetype signatures). ZST components → `fields_of` returns an
  empty `&'static []`, `get_field` returns `None` (presence-only).
  ~~*(exhaustive)*~~ — **NOT exhaustive, struck 2026-08-21.** Both individual rules
  are still correct; the *coverage* claim is not. The storage taxonomy has grown
  three ways since the snapshot, and M6 names one of four citizens. The v1 refusal
  matrix must be re-stated over **`StorageKind` × `ResidencyKind` × dynamic tags**,
  and two of the cases M6 lumps into "refuse" are actually *serviceable* with a
  different view. Full matrix: **B.3** (enumeration) and **B.4** (the four citizens).

---

## 5. Soundness (every `unsafe` + the release-editor gap)

- **`base.add(offset)`** — `base` is a live, init, `align_of::<T>()`-aligned
  instance of the type at `id`; `offset = offset_of!(T, field)` ⇒ in-bounds &
  field-aligned; provenance inherited from the arena-rooted `base`.
- **Typed reinterpret in the `get/set` fn-ptrs** — each fn-ptr is monomorphized for
  the exact field type the derive read via `offset_of!`, installed only into the
  matching `FieldInfo`.
- **The release-editor safety gap (was M5/W4):** the type/kind consistency check
  must be a **real runtime check on `set` (the `-> bool` return), present in
  release**, not a `debug_assert!` — because the legitimate `--release --features
  reflect` editor build compiles `debug_assert!` out, and that is exactly where an
  editor can pass a stale `(ComponentId, field)` triple after a hot-reload. The
  `debug_assert_eq!` on `TypeId` stays as an extra dev guard; the `bool` kind-check
  is the load-bearing one.
- **`FieldMut<'a>` (borrowed `&mut` into a field) is OUT OF SCOPE for v1 (W4).** The
  value model (get/set `Scalar` by copy) has no aliasing-proof obligation. A
  borrowed field handle is precisely the "cached pointer + reborrow" class that
  Tree Borrows caught in boyko after 3 critic rounds approved it (14a-F2, the
  Phase-19 `command_queue` twin). It requires a full TB analysis against concurrent
  query borrows before it is ever added.
- ⚠️ **A SECOND, differently-shaped TB rule landed after this snapshot and is not in
  the list above (added 2026-08-21): `BUG-MIGRATE-TB-1`.** Commit `43684a58`
  (2026-06-21), *"re-derive the structural-write archetype pointer under the live
  protector (Tree-Borrows UB)"*. It directly constrains reflection's **enumeration**
  glue, which is precisely the code that wants to walk an archetype's columns. Full
  statement, the in-tree precedent on both sides, and what the derive must adopt:
  **B.7**.
- **`repr(Rust)` is fine — no `repr(C)` requirement** (`offset_of!` reads the
  compiler's actual offset in the same compile). Cross-process stability is achieved
  by **serializing by field name, never by offset/index**.
  **Re-confirmed and strongly supported (2026-08-21):** `core::mem::offset_of!` is
  used **317 times** across `crates/*/src`, including as
  `const _: () = assert!(offset_of!(…) == N)` layout pins. The exact idiom the derive
  would bake is already load-bearing engine-wide, on this exact toolchain.
- **`#[repr(packed)]` is rejected by the v1 derive** (taking `&field` on a packed
  type is UB; the `*_unaligned` ops are deferred).
- **Generics are rejected by the v1 derive** (a per-impl `static TYPE_INFO`
  collapses across monomorphizations — the documented Bundle / Phase-12.5
  `static SLOT` / Phase-17 `State<S>` trap). The diagnostic points at the future
  keyed-cell path. **Known exclusion:** the engine's own generic `State<S>` cannot
  be inspected in v1.
  **Still correct, same victim, and there is a SECOND independent reason
  (2026-08-21):** `State<S: States>(S)` at
  `crates/boyko_ecs/src/ecs/core/state/state.rs:18` is still generic — but it is also
  **`impl<S: States> Resource for State<S>`** (`:43`), i.e. a **Resource**, not a
  Component. Reflection as designed is keyed by `ComponentId`; `State<S>` has a
  `ResourceId`. It is out of scope twice over. This matters because A.9's acceptance
  test names it — see the A.9 correction.

---

## 6. ~~Open scope fork for the owner~~ → **DECIDED: option (B), taken 2026-08-21**

> ### Decision record
>
> **Taken: (B) — POD + `String` + nested + `#[repr(Int)]` enum in v1.**
> Collections (`Vec` / `Map`) stay deferred to v2.
>
> **Reason** (this section's own argument, which stands): *a layer that can only
> read `f32`/`u64` is not an inspector, and the entire justification for reflection
> is tooling over arbitrary components.* The layer exists to serve **user** components
> it has never seen, not only the engine's own — so the coverage line is set by what
> an arbitrary component may contain, not by what today's engine happens to contain.
>
> **Status: TAKEN-with-reason, and REVERSIBLE.** This is not pre-existing consensus
> and was never ratified before today; it is a scope call recorded now so
> implementation can start. It is reversible **by construction**: dropping `Str` from
> `ValueKind` narrows the taxonomy and deletes accessors, breaking no other decision
> in this document. If Wave 2 finds the `String` setter's TB obligation
> (A.4) is not cheap, falling back to (A)+enums+arrays is a scope edit, not a redesign.
>
> **⚠️ But the JUSTIFICATION as written is stale, and the load-bearing half of (B)
> for THIS tree is not `String`.** See the re-grounding below. The decision does not
> change; what changes is which half of it Wave 2 should build first, and what the
> acceptance test can dogfood.

Everything above is resolved. The remaining fork ~~is~~ **was** **v1 field-type
coverage**, because it changes both effort and whether v1 can dogfood a real
inspector:

- **(A) POD-only v1** — primitives + nested-`Reflect` structs. Fastest to ship,
  but cannot inspect ~~`Name(String)`~~, `Children(Vec<Entity>)`, `Option<Parent>`,
  or ~~`#[repr(u8)] enum State`~~ — i.e. cannot fully inspect the engine's own
  components. If chosen, the v1 *advertised* purpose must drop "editor/inspector"
  and say "primitive + nested-struct field viewer."
- **(B) POD + `String` + nested + `#[repr(Int)] enum` v1** — can dogfood the
  inspector on ~~`Name`~~ / `Transform` / ~~`State`~~. More work (specify the
  `Nested` / `Opaque` recursion contract + its allocation audit now), and the
  `Opaque` path is where any `Box`/`Vec` allocation pressure will live — must be
  specified, not hand-waved. Collections (`Vec`/`Map`) still deferred to v2.

Recommendation: **(B)** — a reflection layer that can only read `f32`/`u64` is not
an inspector, and the whole justification for reflection is tooling that handles
arbitrary components.

### 6.1 Re-grounding (B) against today's component population — added 2026-08-21

The fork's dogfood examples were surveyed against `crates/*/src`. Three of the four
named types do not have the shape this section attributes to them.

| §6 claim | Verdict | Evidence |
|---|---|---|
| `Name(String)` | **FALSE** — `Name` carries no `String` | `boyko_scene/src/identity.rs:47,:56` |
| `Transform` is a nested-struct dogfood target | **TRUE**, and cleaner than assumed | `boyko_scene/src/transform.rs:46` |
| `#[repr(u8)] enum State` | **FALSE** as written — `State<S>` is a *generic* **Resource** | `boyko_ecs/src/ecs/core/state/state.rs:18,:43` |
| enums generally have no v1 consumer | **FALSE** — 11 in-tree consumers | see below |

**(a) `Name` is a `u32`, not a `String`.** `pub struct Name(pub NameId)` where
`pub struct NameId(pub u32)`, both `#[repr(transparent)]`, with a layout pin
`const _: () = assert!(size_of::<Name>() == 4 && align_of::<Name>() == 4);`
(`boyko_scene/src/identity.rs:59-61`). The string lives in a **setup-only process-global interner**
that hands back a **leaked `&'static str`** via `resolve()`. Two consequences:

* Inspecting `Name` in v1 needs a `u32` read plus a cold `resolve()` call — nothing
  more. It never needed scope (B) at all.
* The returned `&str` is **`'static`**, not `'a`-borrowed. So A.3's lifetime argument
  ("the shared `&EcsMaster` borrow `'a` statically excludes any `&mut` op") is
  *unnecessary for `Name`*. That argument is still correct and still load-bearing —
  for a genuine `String` field — but `Name` was never the case that motivated it.

**(b) There is no `String` consumer in the tree at all.** Every `#[derive(Component)]`
struct under `crates/*/src` was walked: **zero** have a `String`, `Box<str>`, or
`&str` field. Every `String` in production source is host / dump / profiling / error
plumbing (`boyko_app/src/host_dump.rs:46~`, `boyko_app/src/profiling/artifact.rs`,
`boyko_ecs/src/ecs/core/asset/error.rs:18~`) — none of them components. The engine deliberately
went the other way, and says so at the source: *"A `Name` component carries its
`NameId` inline"* (`boyko_scene/src/identity.rs:1~-27`). The idiom for "a component needs text" is a
**fixed-capacity inline byte array**: `UiName { bytes: [u8; CAP], len: u8 }`
(`boyko_ui/src/components.rs:274`), `UiTextBuffer` (`boyko_ui/src/binding/components.rs:67`)
— both `#[repr(C, align(64))]`. *That is an array case, not a `String` case.*

**(c) The enum half, by contrast, has ELEVEN consumers.** This is the one place the
audit that produced this revision was itself wrong, and it is corrected here rather
than propagated. `State<S>` is indeed unusable (generic, and a Resource) — but
fieldless `#[repr(u8)]` enums are pervasive in the component population:

*Enums that **are** components (so `TypeKind::Enum` at the top level):*

| type | file | note |
|---|---|---|
| `Visibility` | `boyko_scene/src/render_caps.rs:226` | discriminants pinned `Inherited=0, Visible=1, Hidden=2` **"so the byte is stable across serialization"** |
| `Interaction` | `boyko_ui/src/interaction/components.rs:17` | `None / Hovered / Pressed`, tick-bearing |
| `FocusPolicy` | `boyko_ui/src/interaction/components.rs:51` | `Block / Pass` |

*Fieldless `#[repr(u8)]` enums that are **fields** of components (the `ValueKind::Enum`
case):* `UiAlign.main: AlignMain`, `UiAlign.cross: AlignCross`,
`UiLayout.layout_type: LayoutType`, `UiLayout.position_type: PositionType`,
`UiAnchor.edge: AnchorEdge`, `UiText.align: TextAlign`,
`UiWorldAnchor.scale_mode: WorldScaleMode`, `BindText.template: TemplateId`.

`Visibility` is the **ideal** dogfood target for the enum half: it is a component, it
is fieldless `#[repr(u8)]`, its discriminants are pinned *for serialization
stability* — exactly A.4's `set_enum_variant_index` contract — and "toggle a node's
visibility" is the canonical inspector action. One design note it forces: a component
that **is** an enum has no fields, so `fields_of` returns `&'static []` and the whole
value is reached through `TypeKind::Enum` + the type-level discriminant accessors.
§3.2's `TypeKind` already has the `Enum` variant; A.2's *field*-centric
`FieldValue::Enum` does not cover the top-level case. Wave 2 must handle both.

**(d) Net — (B) stands, its priority order flips.** Ranked by in-tree consumer count:

| (B) component | consumers today | v1 priority |
|---|---|---|
| nested structs | `Transform`→`Vec3`/`Quat`, `GpuTransform3D`→`TrsPacked`, `UiAlign`, `UiLayout` | **1 — load-bearing** |
| fieldless `#[repr(Int)]` enums | 11 (above) | **2 — load-bearing** |
| fixed-size arrays `[T; N]` | pervasive (**not in the taxonomy at all** — see A.2 correction and **B.8**) | **hole, must be closed** |
| `String` | **0** | **4 — build last** |

So: build (B) as decided, but **build the `String` half last**, and treat A.4's
`String` setter — this document's headline CRITICAL — as a *forward-looking*
obligation for future user components rather than as the thing blocking the first
inspector. The genuinely urgent coverage question is `[T; N]`, which the taxonomy
omits entirely and which today would **hard-error** the flagship dense component
(**B.8**).

---

## 7. Phased plan ~~(once the fork is decided)~~ — the fork is decided (§6): **(B)**

The wave spine is unchanged. The **⊕ rows are deliverables added 2026-08-21** that
the snapshot could not have known about; each names the Appendix-B section that
justifies it. None of them is optional: three of them are the difference between a
gate that can fail and a gate that cannot.

- **Wave 0** — crate skeleton + feature wiring + import `MAX_COMPONENTS` +
  CI matrix (with/without feature) + the `cargo tree`/symbol-absence ship gate.
  - ⊕ **Name the real ship target.** `game_app` / `editor_app` do not exist; the
    hosts are `boyko_app`, `boyko_demo`, and the root `boyko-engine` package (§2).
  - ⊕ **Build the absence gate from the measured template, not from scratch** —
    `crates/profile_fixture/tests/profile_axis_census.rs`. Requires `lto = "fat"` +
    `codegen-units = 1` to be decidable at all, requires a **present control** beside
    the absent cell, and must state that `cargo tree` is the load-bearing half
    (**B.6**).
  - ⊕ **Add `boyko_reflect` to the CI Miri package allowlist.** Miri runs as a
    hand-listed sweep, not a workspace sweep; a new package is **not** covered by
    default, so A.9's "mandatory" Miri gate would be a gate that cannot fail
    (**B.9**). ⚠️ **Two rows, not one** — `-p boyko-reflect` *plain* plus
    `-p reflect-fixture --features reflect-fixture/reflect`; the crate itself has no
    `reflect` feature and `--features reflect` on it is a hard cargo error (**B.9**
    correction).
  - ⊕ **Decide whether engine crates may carry a `reflect` feature — BEFORE rung 0.**
    Every dogfood target in this document is defined in a shared engine crate, so §2's
    *"the consumer writes the opt-in"* has no consumer. It also decides the package
    layout (`reflect_fixture` for the gates, `reflect_dogfood` for the engine types)
    (**B.12**, owner sheet **B.13** row 1).
  - ⊕ **Resolve the `BindAccessor` fork before Wave 1** — one table or two (**B.1**).
- **Wave 1** — registry (`REFLECT`, `install_type_info`, `type_info_of`) +
  `Scalar`/`ValueKind`/`TypeKind` + the `prim::` fn-ptr library.
  - ⊕ **Consume `STABLE_NAME_INDEX`; do not re-declare `TypeInfo.stable_name`**
    (**B.2**).
  - ⊕ **Add `ValueKind::Array`** — `[T; N]` of a `Prim` (offset + stride + count,
    all `const`, zero alloc). Without it the flagship dense component is a hard
    error, not a partial view (**B.8**).
- **Wave 2** — `#[derive(Reflect)]` (field walk, `offset_of!` baking, `Reflect`
  impl, `IS_REFLECT` const + `component_id()` install append, generics/packed/bitset
  rejection). trybuild for the rejections; `cfg_attr`-off compiles to nothing.
  - ⊕ **Refusal matrix, not a single bitset rejection** — `StorageKind` (3) × <!-- doc-anchor-ignore -->
    `ResidencyKind` (3) × dynamic tags (**B.4**). Each refusal needs a **good span**, <!-- doc-anchor-ignore -->
    because an Aether user wrote `tag Foo(bitset);` and will otherwise get an error
    about a derive they never typed (**B.5**).
  - ⊕ **Argue A.5's install mechanism against the shipped precedent that chose the
    other way** (`register_bindable` is explicit, not lazy) — see the A.5 correction.
  - ⊕ **Top-level `TypeKind::Enum`**, not only `ValueKind::Enum` (§6.1(c)).
- **Wave 3** — ECS glue (`get/set_field`, by-name, `add_default`/`remove`). Miri-TB
  on every offset/raw/default path; proptest get/set roundtrip.
  - ⊕ **A public by-id structural seam on `EcsMaster`** — the five migration helpers
    are `pub(crate)` and unreachable from an external crate. This **widens a shipping
    crate's API for a dev-only feature** and must be justified on its own merits (§4).
  - ⊕ **Three-source enumeration** — signature + `DenseRegistry` + the enable store.
    The signature alone is structurally blind to `GpuTransform3D` (**B.3**).
  - ⊕ **Do not use `has_component` as the presence probe** — it silently returns
    `false` for every bitset tag (**B.4**).
  - ⊕ **Obey `BUG-MIGRATE-TB-1`** in the enumeration glue: raw-pointer projection,
    never `&Archetype` (**B.7**).
- **Wave 4** — boundary serialize (`Sink`/`Source`) + ~~once-per-type name↔id
  resolution (in `boyko_ecs`)~~ **← already built; consume it (B.2)** + name-keyed
  roundtrip incl. simulated id-reorder.
- **Wave 5** — perf validation: hot-loop 0%-gate (feature on vs off) + `get_field`
  vs bevy-shaped baseline + ship symbol-absence.
  - ⊕ The absence half is only meaningful with the Wave-0 link configuration and its
    present control (**B.6**).

---

## 8. Preserved strengths (do not lose these in implementation)

- Dense-`ComponentId` array registry (one acquire-load + branch) — faster than
  bevy's double-hash; mirrors the proven `LAYOUTS`/`HOOKS`/`SERIALIZE`/`BIND_ACCESSORS`
  discipline. *(Mirror list corrected 2026-08-21 — `STORAGE_KIND` is an `AtomicU8`
  table, not an `OnceLock` one; see §3.1. The core perf claim itself is **unchanged
  and unchallenged**: `ComponentId` is still a dense newtype over a 0-based counter
  bounded by `MAX_COMPONENTS = 512`, and every registry table in the tree is a
  direct-indexed array on it.)*
- Lazy first-call registration via the existing `component_id()` `OnceLock` —
  avoids the `linkme` dead-strip / init-order hazard class entirely.
  *(Re-confirmed 2026-08-21 and now **more** viable than when written: the funnel at
  `boyko_macros/src/component.rs:434-459` is intact — `static ID: OnceLock<ComponentId>`
  → `ID.get_or_init(|| { let raw = register_new::<Self>(); if Self::HAS_HOOKS { … } … })`
  — and it has grown from one install slot to **six**: `storage_install`,
  `require_install`, `clone_install`, `relationship_install`, `residency_install`,
  `serialize_install`. "Append one more, const-gated" is a well-trodden pattern here,
  not a novel one.)*
- Offset + flat fn-ptr field model — no per-field `&dyn`, no `Box`, no
  `DynamicStruct`/`FromReflect` value-tree allocation.
- `repr(Rust)` allowed + serialize-by-name — a real win over manual offset tables,
  and defuses the Unreal `WITH_EDITORONLY_DATA` layout-skew hazard.
- The directional rule (`boyko_serialize` / shipping crates never depend on
  `boyko_reflect`) — the load-bearing invariant that keeps "Debug-only" honest.
- The compile-boundary hot-path proof ("the symbol does not exist") — stronger than
  the 14a/14b runtime-branch discipline.

---

# Appendix A — v1 compound-value model (scope B: String + nested + `#[repr(Int)]` enum)

Owner decision 2026-06-15: v1 covers **POD + `String` + nested `#[derive(Reflect)]`
structs + `#[repr(Int)]` fieldless enums**, so the inspector can dogfood the engine's
own ~~`Name(String)`~~, a nested `Transform`-like struct, and ~~`#[repr(u8)] enum State`~~.
This appendix is a second architect→2-critic round; it folds the two critiques'
CRITICAL/MAJOR findings in as **resolved decisions**. It is purely additive to §1–§8.

> **Dogfood targets corrected 2026-08-21 (see §6.1 for the full survey).** `Name`
> carries **no `String`** — it is `Name(NameId)` over a `u32`
> (`boyko_scene/src/identity.rs:47,:56`). `State<S>` is a **generic Resource**, not an
> enum component (`boyko_ecs/src/ecs/core/state/state.rs:18,:43`). The surviving target is `Transform`, and it is
> cleaner than assumed. The correct enum dogfood target is **`Visibility`**
> (`boyko_scene/src/render_caps.rs:226`). The **scope decision (B) is unchanged and
> now formally TAKEN** — only its illustrations were wrong.

## A.1 Scope line (honest)

**v1:** primitives, `String` (read+write), nested `Reflect` structs (read+write at
any depth), fieldless `#[repr(Int)]` enums (read + set-variant). Tuple structs
supported (with the naming caveat A.4).
**v2 (explicitly excluded, with reasons):** `Vec`/`Map`/collections; **data-carrying
enums** (no Reference-guaranteed variant-field layout); **`Option<T>`** (it is the
smallest data-carrying enum — niche optimization means *no* guaranteed discriminant
location, so it inherits the full data-enum hazard; **not "cheap enough"**); generics;
`repr(packed)`; `FieldMut` borrowed handles.

> **The v1 and v2 lists are BOTH incomplete — fixed-size arrays `[T; N]` appear in
> neither (2026-08-21).** They therefore fall through to `Opaque`, and A.6's `Opaque`
> rule is a **hard error**, so today the flagship dense component would be
> **un-derivable** rather than partially inspectable. Arrays are the dominant compound
> in the render surface and are strictly *easier* than the `String` support this
> appendix spent its headline CRITICAL on. **`[T; N]` of a `Prim` must be pulled into
> v1** — see the A.2 correction and **B.8**.
>
> **Ordering note (§6.1(d)):** the `String` half of v1 has **zero** consumers in this
> tree; the nested, enum, and array halves have many. Build `String` **last**.

## A.2 Value taxonomy (with the two correctness fixes baked in)

```rust
#[repr(u8)] enum ScalarKind { Bool,U8,U16,U32,U64,I8,I16,I32,I64,F32,F64,EntityId }
#[repr(C)]  struct Scalar { kind: ScalarKind, bits: u64 }   // 16 B POD Copy (unchanged)

#[repr(u8)] enum ValueKind { Prim(ScalarKind), Str, Nested, Enum, Opaque }
//   ⚠️ INCOMPLETE (2026-08-21): no `Array` arm. `[T; N]` falls through to `Opaque`,
//      which A.6 makes a HARD ERROR — so `GpuTransform3D` is un-derivable today.
//      Add `Array { elem: &'static FieldInfo-ish, stride: usize, len: usize }`. See B.8.

#[non_exhaustive]
pub enum FieldValue<'a> {
    Scalar(Scalar),            // POD by copy
    Str(&'a str),              // borrows the live String buffer — ZERO alloc
    Nested(NestedCursor<'a>),  // the ONLY nested shape (the bare {ptr,info} variant is DELETED, was M2/O3)
    Enum { discr: Scalar, info: &'static EnumInfo },
}

/// Re-rootable READ cursor. The `'a` (compiler-enforced) IS the validity guarantee —
/// there is no "documented contract"; it cannot coexist with `&mut EcsMaster`.
#[derive(Clone, Copy)]
pub struct NestedCursor<'a> { ptr: *const u8, info: &'static TypeInfo, _pd: PhantomData<&'a ()> }
impl<'a> NestedCursor<'a> {
    pub fn type_info(&self) -> &'static TypeInfo { self.info }   // FIX (completeness-C1):
    pub fn fields(&self) -> &'static [FieldInfo] { self.info.fields } // enumeration at depth ≥ 1
}
pub fn fields_of_type(info: &'static TypeInfo) -> &'static [FieldInfo] { info.fields }
```

`FieldInfo` get/set are **`Option<fn>` for every kind** (FIX completeness-C2 / Mi2 —
**no "poison stub" exists**); `field_value` dispatches on `kind` and only calls the
accessor that kind installs. Per-kind payload: `nested: Option<&'static TypeInfo>`
(Nested), `enum_info: Option<&'static EnumInfo>` (Enum), `get_str`/`set_str` (Str),
`get_discr`/`set_discr` (Enum). The scalar API `get_field` **returns `None` for any
non-`Prim` field** (FIX Mi2 — no silent garbage `Scalar`).

`EnumInfo { repr: EnumRepr, variants: &'static [VariantInfo] }`, where
`VariantInfo { name: &'static str, discr_bits: u64 }` stores the discriminant
**already narrowed to the repr width** (FIX C2/O1 — *not* a lossy `i128 as u64` at the
call site); reads sign-extend per `EnumRepr` for `Ix` reprs.

## A.3 Read paths (all zero-allocation — the audit survived)

- `field_value(&ecs, e, id, f)`: `base.add(off)` → `match kind` → by-copy `Scalar` /
  by-borrow `&str` / `NestedCursor` / `Enum{discr,info}`. **0 alloc** in every arm.
- Nested descent: `cursor.fields()` to enumerate, `cursor.ptr.add(inner_off)` to
  descend — **one `add` per level**, `&'static TypeInfo` reused (no flattened path
  table). Acyclic by construction (a `Sized` value type cannot contain itself by value;
  all indirections are v2) → no runtime cycle guard needed.
- Enumerate: returns the baked `&'static [FieldInfo]`. **0 alloc.**
- `String` read justification (FIX M1): the returned `&str` is sound **because the
  shared `&EcsMaster` borrow `'a` statically excludes any `&mut` op (set_str /
  structural move) for `'a`** — *not* because "the buffer lives as long as the
  component" (that wrong reason would also "justify" a UAF across a `set_str`).

> **The argument is correct and stays — but its motivating example was wrong
> (2026-08-21).** This reasoning applies to a genuine `String` field, of which the
> tree has **none** (§6.1(b)). It specifically does **not** apply to `Name`, which
> A.1/A.4/A.6 use as its illustration: `Name` holds a `u32`, and the string comes
> from a cold `resolve()` against a **process-global interner that leaks** its
> allocations, so the `&str` is **`'static`** — not `'a`-borrowed
> (`boyko_scene/src/identity.rs:1~-27,:56`). Reading `Name` needs no lifetime argument
> at all: a `u32` read plus a cold call. Keep the `'a` reasoning for future user
> components; do not cite `Name` for it.

## A.4 Write paths (soundness-first)

- **`String` write — the highest-risk surface (FIX C1, the headline catch).** The
  setter does **raw `ptr::drop_in_place(p as *mut String)` then `ptr::write(p as *mut
  String, s.to_owned())`**, operating on the original arena `*mut` provenance — it
  **never forms an intermediate `&mut String`**. This sidesteps the `Unique` retag
  through the arena's deliberately-`SharedReadWrite` interior-mutable provenance, which
  is the 14a-F2 / Phase-19 TB-UB class. The earlier draft's `*slot = s.to_owned()` via
  `&mut String` is **rejected**; its pre-declared "TB is avoided" verdict is struck.
  **Miri-TB under `-Zmiri-tree-borrows` is the gate, not an argument** (the project's
  hard-won lesson: critics approve TB-UB; only Miri is the oracle). Alloc accounting:
  exactly 1 alloc (new) + 1 free (old), no leak, no double-free.

  > **Demoted from headline to forward-looking, 2026-08-21 — the motivating component
  > does not have the field.** The `unsafe` reasoning here is **sound and must be kept
  > verbatim**; what is struck is its *urgency ranking*. This was ranked the highest-risk
  > v1 surface because ~~`Name(String)`~~ was believed to exercise it. `Name` holds a
  > `u32` (`boyko_scene/src/identity.rs:47,:56`), and **no `#[derive(Component)]` struct in
  > `crates/*/src` has a `String` field at all** (§6.1(b)). So the raw
  > `drop_in_place` + `ptr::write` dance, its Miri-TB gate, and its 1-alloc/1-free
  > accounting are an obligation for *future user* components, not a blocker for the
  > first inspector. Build it last (§6.1(d)); keep the gate mandatory when it is built.
  >
  > Two things **replace** it at the top of the risk order, both because they have real
  > in-tree consumers: the **enumeration** glue's `BUG-MIGRATE-TB-1` obligation
  > (**B.7**) and the `[T; N]` hole (**B.8**).
  >
  > Miri-TB itself is confirmed available and enforceable exactly as this line assumes:
  > `MIRIFLAGS = "-Zmiri-tree-borrows"` is set workspace-wide in `.cargo/config.toml`,
  > and CI's Miri job is **required**, not `continue-on-error`
  > (`.github/workflows/ci.yml:193`). ⚠️ But it is an **allowlist**, not a sweep — a new
  > package is not covered until it is named. See **B.9**.
- **Enum write (FIX C2):** `set_enum_variant_index` does a **release** bounds check
  (`idx < variants.len()` → else `false`, not a `debug_assert!`); only a **baked
  variant discriminant** is ever written (every fieldless `#[repr(Int)]` variant value
  is a valid inhabitant) → no invalid-value UB; the kind check is the load-bearing
  release `-> bool`.
- **Nested-leaf write (FIX W1):** in v1 via a `&mut`-rooted `NestedCursorMut<'a>` whose
  setter performs the store **internally** through the same audited primitive/`str`
  glue at the composed offset — **no field handle escapes** (so it is *not* the
  `FieldMut` class). Same raw-store / `drop_in_place`+`write` discipline as A.4 String;
  Miri-TB gated. If TB proves troublesome, nested-leaf write slips to v2 (nested stays
  read-only) — but it is specified, not left implicit.
- All writers take `&mut EcsMaster` (exclusive); release kind-check on every setter.

## A.5 Install mechanism — the load-bearing unresolved item (was O-1 / W2)

The two-derive `#[cfg_attr(feature="reflect", derive(Reflect))]` from §2 cannot cleanly
hook the lazy `component_id()` install funnel, because that funnel is emitted by the
**`Component` derive**, and a *separate* `Reflect` derive cannot inject into the same
`impl Component`. Resolution:

**Recommended: a single `#[derive(Component)]` + an opt-in `#[reflect]` helper
attribute.** When `#[reflect]` is present, the Component derive emits the `Reflect`
impl, the `TYPE_INFO` static, and the `install_type_info` call **all wrapped in
`#[cfg(feature = "reflect")]`** (cfg in derive output is evaluated in the *consumer*
crate). Feature off → tokens stripped → zero, `boyko_reflect` absent. Feature on (the
consumer's `reflect` feature, which pulls the optional `boyko_reflect` dep) → full
reflection. `boyko_ecs`/`boyko_macros` **never depend on `boyko_reflect`** — they only
emit cfg-gated *tokens* that name it (the directional rule is about crate deps, not
emitted tokens). One naming convention: the consumer's reflect-enabling feature MUST be
named `reflect`. This supersedes the §2 `cfg_attr`-two-derive sketch.

**This is a proc-macro/Cargo mechanism that only a compile settles** (cf. the toolchain
lesson) → Wave 0 must include a PoC that compiles a consumer crate with the feature
both on and off. **Guaranteed fallback** if the lazy mechanism misbehaves: explicit
`boyko_reflect::register::<T>()` at startup (bevy-style) — slightly less ergonomic, but
zero mechanism risk.

> ### A.5 correction (2026-08-21): this is no longer an open question in a vacuum —
> ### the tree answered it for the analogous feature, and it chose the FALLBACK.
>
> `#[derive(Bindable)]` faced exactly this fork and did **not** hook `component_id()`.
> It emits a `register_bind_accessor()` associated fn (`boyko_macros/src/bindable.rs:100-127`;
> trait decl at `boyko_ui/src/binding/bindable.rs:45`) that a human must call at setup.
> Its own doc-comment spells out the failure mode when you forget: *"the entire
> `.ui`-dynamic data-bind path for `C` is unreachable"*
> (`boyko_ui/src/interaction/plugin.rs:183~-186`).
>
> **And the precedent is more damning than it first looks.** There are two registration
> entry points, and the tree uses the *weaker* one everywhere:
>
> * `register_bindable::<C>(app)` (`boyko_ui/src/interaction/plugin.rs:189-195`) — the **complete** form: it
>   installs the accessor **and** adds `C`'s `ComponentId` to the `UiBindScratch`
>   discovery gate. **It has ZERO callers anywhere in the tree.**
> * `C::register_bind_accessor()` — the raw half, which installs the accessor and
>   *not* the gate id. This is what all five call sites actually use:
>   `boyko_render/tests/ui_hud_screenshot.rs:490~`, `boyko_ui/tests/p4_bind.rs:87~`,
>   `p4_bind_zero_alloc.rs:148~`, `p4_miri.rs:116~`, `text_bind_emit.rs:115~`.
>
> All five are **tests**, and every one registers the same fixture type (`Health`) — so
> there is **no production `#[derive(Bindable)]` component at all** today. An explicit
> registration API whose complete form is called zero times, and whose partial form is
> called only from tests, is not a neutral precedent for "explicit works fine." It is a
> small, live demonstration of the exact failure mode the argument below rests on.
>
> Meanwhile the **lazy funnel this section recommends is healthier than when this was
> written** — it has grown from one install slot to six
> (`boyko_macros/src/component.rs:434-459`; §8). So the recommendation is *more*
> viable, not less. What changed is the burden of proof: **A.5 must now argue against
> a shipped in-tree precedent that went the other way**, and say why reflection goes
> lazy where binding went explicit.
>
> **The argument, which was never written down and belongs here:** the two features
> fail differently when registration is missed. A forgotten `register_bindable::<C>`
> breaks *one authored binding* — a visible, localized, reproducible bug in a feature
> someone deliberately wired up. A forgotten reflection registration makes a component
> **silently invisible in the inspector**, which is indistinguishable from "this entity
> doesn't have that component" — the inspector's entire job is to be trusted about
> what is there, so a silent omission is a *correctness* failure of the tool, not a
> missing feature. Reflection cannot tolerate "you forgot to register, so the
> inspector is quietly empty." That asymmetry — and not ergonomics — is why reflection
> takes the lazy funnel and binding did not.
>
> The explicit `register::<T>()` fallback therefore remains available as a *mechanism*
> escape hatch, but if it is ever taken, the inspector **must** be able to distinguish
> "no `TypeInfo` installed" from "component absent" and say so in the UI. Note this is
> the same three-state display problem the GPU-residency and dynamic-tag cases raise
> independently (**B.4**) — one shared "known-but-not-viewable" row state answers all
> three.

## A.6 Serialize/deserialize boundary (the one `dyn`)

`Sink`/`Source` traits, **by field name throughout** (stable across reorder — except
tuple structs, see below). Deserialize-side contract (FIX W3): `Source::str_field`'s
returned `&str` **must be consumed by `set_str` (copied) before the next `&mut self`
`Source` call** — stated as a hard contract (or use a `&mut dyn FnMut(&str)` callback
form). `Opaque` fields (FIX O2): the derive **refuses to serialize a type containing an
`Opaque` field** (hard error) rather than silently dropping it — the wire format is
shared with the ~~future~~ **shipped** (`boyko_serialize`, §1) serializer, so silent
omission is unacceptable.

> **The hard error is right; its BLAST RADIUS was never weighed (2026-08-21).** With
> `[T; N]` missing from the taxonomy (A.2 correction), "refuses" currently catches
> types the design intends to support:
>
> * **`GpuTransform3D`** — `{ prev: TrsPacked, curr: TrsPacked }`, each
>   `{ pos: [f32;4], rot: [f32;4], scale: [f32;4] }` (`boyko_render/src/gpu_transform3d.rs:55-62,:84~`).
>   Un-derivable today. This is the **first production `#[component(storage = "dense")]`
>   type** — see **B.3** for the second, worse problem it has.
> * **`SoftBody`** — an ordinary `#[derive(Component)]` with **fourteen**
>   `Vec<f32>`/`Vec<u32>` columns (`boyko_physics/src/soft/component.rs:69-87`): 100 %
>   v2-deferred kinds, so under A.6 it is not merely uninspectable but a **compile
>   error** if anyone derives `Reflect` on it.
>
> Adding `ValueKind::Array` (**B.8**) rescues the first class outright. `SoftBody`
> remains correctly out of v1 — the point is only that "refuses to serialize" must be
> a *documented, spanned, opt-out-able* refusal (`#[reflect(skip)]` on the field, or
> the type simply not opting in), not a surprise at first `#[reflect]`.

**Tuple structs (FIX completeness-C3):** `FieldInfo.name` for a tuple field is `"0"`,
`"1"`, … . **For tuple structs, by-name == by-position**, so the reorder-stability the
spine advertises does **not** hold for them — documented explicitly. ~~`Name(String)` is
a tuple struct; it works~~ — **struck 2026-08-21**: `Name` is a tuple struct
(`Name(pub NameId)`), so it is still a valid *illustration of the tuple-struct rule*,
but it holds a `u32`, not a `String` (`boyko_scene/src/identity.rs:56`). A `NameId` tuple field also
makes it a **`Nested`** case, not a `Str` one. Reordering a tuple struct's fields is
a breaking save change. Named-field structs are recommended for any serialized
reflectable type.

> **Note (2026-08-21): the shipped `Bindable` derive takes the OPPOSITE position on
> tuple structs** — it **rejects** them and requires named fields, because its whole
> API is `field_id(name) -> Option<u8>`. A.6 accommodates them. If the
> `BindAccessor`-merger horn of the **B.1** fork is taken, this is a direct conflict
> that must be resolved rather than averaged.

## A.7 Allocation audit (compound paths)

| Path | Alloc | Class |
|---|---|---|
| `field_value` (any kind read) / nested descent / enumerate / enum read / `get_field` / `set_field` (Prim) | **0** | none |
| `set_str_field` | 1 alloc + 1 free | caller-data-driven, cold, explicit |
| `set_enum_variant[_index]` | **0** | none |
| serialize (in `boyko_reflect`) | **0** | sink-owned, reused |
| deserialize `String` field | 1 / occurrence | loader-owned, cold, irreducible |
| `add_default` | **0 bespoke** | routes through existing structural insert — ⚠️ *the route is `pub(crate)`; see §4* |
| name/TypeId→ComponentId resolve | 1 dense `Vec` / load | cold setup, in `boyko_ecs`, once per type — ⚠️ *already built as `STABLE_NAME_INDEX`; see **B.2*** |
| array read `[T; N]` of `Prim` *(added 2026-08-21)* | **0** | offset + stride + count, all `const` — no alloc, no drop, no TB exposure (**B.8**) |

The **read / enumerate / nested-walk tree is provably zero-allocation** — a `String`
field is touched only by borrowing `&str` (ptr+len), never cloned. The only two alloc
sites materialize *payload from caller data* and are cold + explicit.

## A.8 `add_default` drop-safety for compounds (FIX W4)

`default_in_place` writes into **uninitialized arena bytes only** (the structural-insert
contract — never over a live value, so no leak/double-free of an existing owning field);
nested defaults recurse; empty struct (`fields: &'static []`) → walk/serialize is a
no-op, not a panic. If a scratch buffer is used it is `align`-correct and bytes are
**moved (forgotten after copy), not dropped**. Validation adds a **drop-count test** for
`add_default` of a `{ pod, String, Nested{String} }` type.

## A.9 Validation (incl. the dogfood acceptance test — FIX O4)

Beyond the §7 strategy: a single **end-to-end acceptance test** instantiating a real
~~`{ Name(String), Transform { translation: Vec3{f32,f32,f32}, … }, State(#[repr(u8)]) }`~~
entity — enumerate top-level fields, read each kind, **descend into `Transform` and read
a leaf**, set the ~~`String`~~, set the enum variant, set a nested leaf, re-read. (This test
would have surfaced the nested-enumeration gap immediately.) **Miri-TB is mandatory** on:
the String `drop_in_place`+`write`, nested-leaf write, enum discr read/write, nested
offset composition — these are the second TB-critical surface after the executor series.
Missing-`repr` fieldless enum → **compile error** (FIX Mi3), not a silent `Opaque`.

> ### A.9 correction (2026-08-21) — the fixture as literally written cannot compile
>
> Two of its three types are misdescribed, so the test would be rewritten at Wave 3 by
> whoever hit it. Rewriting it here keeps the *intent* (dogfood the engine's own
> components, one of each kind, with a nested descent) and fixes the instances:
>
> | as written | why it fails | replacement |
> |---|---|---|
> | `Name(String)` | `Name` is `Name(NameId)` over a `u32`; no `String` field exists anywhere (`boyko_scene/src/identity.rs:47,:56`) | keep `Name` as the **tuple-struct + `Nested`** case (`Name` → `NameId` → `u32`); add a **local fixture** type for the `Str` case |
> | `State(#[repr(u8)])` | `State<S: States>` is **generic** *and* a **`Resource`**, not a Component (`boyko_ecs/src/ecs/core/state/state.rs:18,:43`) — out of scope twice over (§5) | **`Visibility`** (`render_caps.rs:226`) — a fieldless `#[repr(u8)]` enum that **is** a Component, discriminants pinned for serialization |
> | `Transform { translation: Vec3, … }` | ✅ **correct, and cleaner than assumed** | keep verbatim |
>
> **`Transform` fully vindicates the nested half of scope (B):**
> `Transform { translation: Vec3, rotation: Quat, scale: Vec3 }` — 40 B, layout-pinned
> (`const _: () = assert!(size_of::<Transform>() == 40);`) — where
> `Vec3 { x, y, z: f32 }` (`boyko_math/src/vec.rs:145`) and `Quat { x, y, z, w: f32 }`
> (`boyko_math/src/quat.rs:23`) are plain named-field POD structs. **No `Opaque`, no
> arrays, no `String`** — an ideal `Nested` target at exactly depth 2.
>
> **Revised fixture:**
> `{ Name(NameId(u32)), Transform{ translation: Vec3{f32,f32,f32}, rotation: Quat, scale: Vec3 }, Visibility(#[repr(u8)] fieldless), GpuTransform3D{ prev: TrsPacked{[f32;4]×3}, curr } }`
> — enumerate, read each kind, descend into `Transform` **and** into `Name`→`NameId`,
> set a nested leaf, **set `Visibility`'s variant**, read an **array** element, re-read.
> A locally-defined `struct StrFixture { s: String }` covers the `Str` arm until a
> production consumer exists.
>
> **This test is now also the dense-storage regression** — `GpuTransform3D` is
> `#[component(storage = "dense")]`, so a fixture containing it fails immediately if
> enumeration is built on the archetype signature alone (**B.3**). That is the single
> highest-value assertion to add, because it is the failure the design would otherwise
> ship: *refusing to show the one component it is fully able to read.*
>
> ⚠️ **The "Miri-TB is mandatory" clause is not self-enforcing** — CI runs Miri as a
> hand-listed package allowlist, so `boyko_reflect` is not covered until Wave 0 adds
> it. See **B.9**.

## A.10 Net

The zero-alloc inspection core (crux a) survives the audit intact. Two CRITICAL
soundness items (String-replace TB retag; enum invalid-value write) and the install
mechanism were the real findings — all resolved above, with **Miri-TB as the gate for
every new `unsafe`, not an after-the-fact claim**. The v1/v2 line is honest:
read-everything + write-(scalar/String/enum/nested-leaf) in v1; collections, data-enums,
`Option`, generics, `FieldMut` in v2.

---

# Appendix B — surface this document predates (added 2026-08-21)

Everything here is **new material**, kept out of §1–§8 and Appendix A on purpose: a
reader comparing this revision against the 2026-06-15 original must be able to see
exactly what is old-and-corrected versus what is genuinely new.

The engine did not stand still between the snapshot and this revision. Two changes
land *directly* on this design — a shipped accessor table that is most of §3.2
(**B.1**), and a third storage kind that makes §4's enumeration spine structurally
blind to a production component (**B.3**). Both are **decisions**, not adjustments.

---

## B.1 `#[derive(Bindable)]` / `BIND_ACCESSORS` — a shipped, reflection-shaped, **ship-build-resident** field accessor table

> **The single largest thing this document does not know about.**
> `crates/boyko_macros/src/bindable.rs` · `crates/boyko_ecs/src/ecs/core/component/component_registry/serialize.rs:244~-315` · `crates/boyko_ui/src/binding/bindable.rs:45`
> Commit `8a11f31b` (2026-06-21) — *"P4 — action + data binding (… **reflection-free
> codegen binding**)"*, six days after the snapshot.

It is roughly **80 % of §3.2's field model, already in production**:

| §3.2 proposes | `BindAccessor` already does |
|---|---|
| per-`ComponentId` `[OnceLock<T>; MAX_COMPONENTS]` table | ✅ `BIND_ACCESSORS` (`component_registry/serialize.rs:277`) |
| flat `fn` pointers — no `dyn`, no `Box` | ✅ two bare `fn` ptrs (`fmt`, `value`) |
| by-index field access | ✅ by `u8` index, `match`-dispatched |
| `name → index` resolver | ✅ `field_id(name) -> Option<u8>` |
| documented cold-path discipline | ✅ *"never on a still frame, never on the per-frame hot path"* |
| bounded field count | ✅ ≤ 255 fields |
| `get_*(component_id: usize)` / `install_*(component_id: usize, …)` | ✅ `:287` / `:299~-308` |

**What it does NOT do** — and each gap is a place the two could diverge or merge:

* **read-only** — no setter of any kind;
* **no nesting** — flat, one level;
* **no kind tag** — `value_field` is literally `self.field as f32`, so it only
  compiles for numeric fields;
* **requires NAMED fields** — it *rejects* tuple structs, the exact opposite of A.6's
  tuple-struct accommodation (see the A.6 note);
* **it SHIPS** — the table lives in `boyko_ecs`, the read path lives in `boyko_ui`, and
  both are in the release binary unconditionally (no feature gates either).
  *(Nuance worth knowing before the fork below: the **mechanism** ships, but there is
  **no production `#[derive(Bindable)]` type yet** — all five registration call sites
  are tests registering one fixture (`Health`). So merging the tables today would
  disturb no shipped component data; the cost of Horn 1 is about the **API direction**,
  not about migrating existing users. See the A.5 correction.)*

### The fork, which must be resolved before Wave 1

**Horn 1 — extend `BindAccessor` into `TypeInfo` (one table).** No duplicated
registration, no two-sources-of-truth about what a component's fields are, and
`boyko_ui`'s binding path gets kinds, nesting and names for free. **Cost:** a
*shipping* crate (`boyko_ui`) then consumes reflection metadata, which strains — and
arguably inverts — the directional rule of §1. It does not *breach* it if the metadata
type stays in `boyko_ecs` and only the *derive-side richness* is gated, but that is a
subtle line and it must be drawn explicitly, not assumed. It also forces the
tuple-struct conflict to a decision.

**Horn 2 — two parallel tables, and justify the duplication.** `BIND_ACCESSORS` stays
exactly as it is (shipping, read-only, numeric, flat); `REFLECT` is a second
`[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]` that exists only when the feature is
on. **Cost:** a `#[derive(Component)] #[derive(Bindable)] #[reflect]` type installs two
descriptions of the same fields, which can drift — a renamed field updates one and not
the other, and nothing catches it. **Mitigation if this horn is taken:** a
feature-on-only test asserting that, for every type carrying both, `field_id(name)`
and the reflect field index agree. That test is cheap and it is what makes the
duplication honest rather than merely tolerated.

**Neither horn is obviously right.** Horn 2 is the lower-risk default (it changes
nothing that ships); Horn 1 is the one that pays off if the editor and the UI binding
system are ever meant to share a field vocabulary. This is a **VALUES/SCOPE call about
shipping API surface** and should go to the owner with these two costs stated, not be
decided silently in Wave 1.

---

## B.2 `stable_name` already exists — consume it, do not re-declare it

§4's placement decision — *"name/`TypeId` → `ComponentId` resolution lives in
`boyko_ecs` … Reflect **uses** it; it does not **own** it"* — was **correct**, and the
shipping save/load consumer implemented it independently while this document sat.
Already in `component_registry/serialize.rs`:

* `Component::stable_name()`, `register_stable_name`, `resolve_stable_name`;
* `STABLE_NAME_INDEX` (`:363`) — a hash-bucketed name → id index, explicitly documented
  as **cold**: *"touched only at registration and once per file-local type at load —
  never on the per-frame hot path"* (`:353-354`), which is exactly W2's
  no-`Mutex`-per-entity requirement, already satisfied;
* `SerializeInfo { stable_name, stable_name_hash (FNV-1a 64, `:321`), format_version
  (`:147`), layout_fingerprint (`:151`) }`.

**Consequence for §3.2:** `TypeInfo.stable_name` as proposed would be a **second,
independent declaration of an existing registry datum** — precisely the drift class
B.1 Horn 2 warns about, and here with a *serialization* wire format on the other end.
`layout_fingerprint` and `format_version` are further data the reflection layer would
otherwise have reinvented worse.

**Decision:** `TypeInfo` carries **no** `stable_name` field; the save key is read from
`get_serialize_info(id)`. Wave 1 wires the read; Wave 4's "once-per-type name↔id
resolution" is **already built** and is a consumption task, not a construction task.

---

## B.3 Enumeration is structurally blind — the signature is no longer the component set

> `component_registry/mod.rs:323-356` · `archetype/archetype.rs:1411` ·
> `ecs_master/component_api.rs:176,:76` · `boyko_render/src/gpu_transform3d.rs:84~`

§4 makes `components_of` — the archetype signature — the enumeration entry point.
**That premise is false today.** `is_signature_storage` (`component_registry/mod.rs:341~-356`) returns true
for **`StorageKind::Table` only**; both `Bitset` and `Dense` are excluded from every
archetype signature, so `Archetype::component_ids()` silently omits them.

**`StorageKind::Dense` landed 2026-06-19 (commit `3c14954d`) — four days after the
snapshot.** The resulting asymmetry decides real design:

| | in signature? | has per-row bytes at a stable address? | `get_component_raw` works? |
|---|---|---|---|
| `Table` | ✅ | ✅ | ✅ |
| `Dense` | ❌ | ✅ | ✅ (dense branch → `dense_get_raw`, `ecs_master/component_api.rs:76`) |
| `Bitset` | ❌ | ❌ (the bit *is* the datum) | ❌ (no column, by construction) |

So for a dense component **the read path already works and the enumeration path is
blind**. The concrete victim is not hypothetical: **`GpuTransform3D`** — the *first*
production `#[component(storage = "dense")]` type, a 96 B interpolation pose pair
(`gpu_transform3d.rs:84~`) — would **never appear** in an inspector built on
`components_of`. The design would **refuse to show the one component it is fully able
to read.**

### Fix: enumeration takes THREE sources

```rust
pub fn components_of(ecs: &EcsMaster, e: Entity) -> ComponentIter<'_>;   // NOT Option<&[ComponentId]>
//   1. Table  — the archetype signature (`Archetype::component_ids()`) — as today
//   2. Dense  — walk DenseRegistry's registration-order `dense_ids`, keep those with
//               `ecs.dense_contains(e, id)`                       (ecs_master/component_api.rs:49)
//   3. Bitset — walk the registered enable tags, keep those with
//               `ecs.is_enabled_id(e, EnableTagId)`            (enable_tag_api.rs:126)
```

Source 2 is cheap: `DenseRegistry` is a directly-indexed `Box<[Option<DenseStore>]>` of
`MAX_COMPONENTS` cells with a registration-order `dense_ids` list, and both
`dense_contains` (`:49`) and `dense_slot_of` (`:58`) are already **public** on
`EcsMaster`. Source 3 is B.4's presence view.

**Signature change:** the return type can no longer be `Option<&[ComponentId]>` — there
is no single contiguous slice to borrow. Either return an iterator, or fill a
caller-provided buffer (the allocation-free option, and the one consistent with §3.3's
zero-alloc audit). **`components_of` must also tag each id with its kind**, because the
caller has to know which of B.4's four display states applies.

---

## B.4 The four citizens: a refusal matrix, not a single bitset rejection

§4's M6 names one non-table case. There are **four**, and two of them are serviceable
rather than refusable.

### (1) `StorageKind::Bitset` — refuse `Reflect`, but **substitute a presence view**

"Read field at offset" is meaningless here — there are no per-row bytes — so the
compile-time refusal on `#[derive(Reflect)]` **stays exactly as M6 specifies**. But the
inspector is *not* helpless, which M6 implies it is. There is a real by-id boolean
surface: `EcsMaster::is_enabled_id(e, EnableTagId)` / `enable_id` / `disable_id`
(`enable_tag_api.rs:113,:119,:126`), and `EnableTagId` is `#[repr(transparent)]` over
`ComponentId` with ~~a public round-trip~~ (`component_registry/tags.rs:93,:104-106`; there is even a
round-trip test — `enable_tag_id_bridges_to_component_id_round_trip`,
`component_registry/mod.rs:1676`).

> **"Public round-trip" is HALF-TRUE and is struck 2026-08-21 (second pass) — the round-trip
> is public in ONE direction, and the test that proves the other one runs *inside* the crate.**
> `EnableTagId::component_id()` and `impl From<EnableTagId> for ComponentId` exist
> (`tags.rs:95-109`); **there is no inverse.** A whole-crate grep for `EnableTagId(` finds six
> construction sites, all inside `boyko_ecs`. So an external `boyko_reflect` can *enumerate* a
> bitset `ComponentId` and **cannot call `is_enabled_id` / `enable_id` / `disable_id` with it.**
>
> **And the obvious substitute does not work — it silently toggles the wrong bit.** The route
> "get the display name, then re-mint by name" (`display_name(id)` → `register_enable_tag(name)`
> → `enable_id`) is **idempotent by name only within `TAG_NAMES`**, and a *derived*
> `#[component(storage = "bitset")]` type never interns its name there. Traced:
> `EcsMaster::register_enable_tag` (`enable_tag_api.rs:60`) → `try_register_enable_tag_by_name` (`tags.rs:134`)
> → `try_register_tag_by_name` (`tags.rs:182`), whose name table is `TAG_NAMES`
> (`tags.rs:155`) and whose miss path calls
> `try_register_dynamic(ComponentLayout::new_dynamic_tag(leaked))` — **it MINTS A NEW ID.** A
> derived bitset component's id came from `register_new::<Self>()` and its name was never
> interned. So `set_presence(EmitterActive_id, false)` would mint a brand-new dynamic tag under
> the string `boyko_render::particle::EmitterActive`, clear *that* bit, and leave
> `EmitterActive`'s own bit set — while reporting success.
>
> **Therefore the presence view needs a real seam:** a checked
> `EnableTagId::try_from_component_id(id) -> Option<Self>`, `None` unless
> `storage_kind(id) == Bitset`. It mints no capability the crate does not already have
> internally and it completes the bridge `component_registry/mod.rs:1676` already tests. This is a **shipping-API
> owner call**, and it is **the same decision** as the ECS glue's bitset-probe seam — one item,
> not two. See the owner sheet, **B.13 row 2**.
>
> The *dynamic-tag* half of the same trick **is** sound, and the asymmetry is worth naming: a
> runtime-minted tag's id came from `try_register_tag_by_name`, so its name **is** in
> `TAG_NAMES` and `ComponentLayout::type_name` **is** that interned name — `register_tag(name)`
> returns the same id, and `tag_by_name` (`tag_api.rs:76`) is public besides. **The
> runtime-minted citizen is better served by the public API than the compile-time one.**

**v1 specifies a `PresenceKind` view:** `fields_of` → empty, `get_field` → `None`, and a
**distinct boolean get/set** answers instead. That is exact read *and* write on the one
datum the tag has. Real consumers exist today: `EmitterActive`
(`boyko_render/src/particle.rs`), `RenderEnabled` (`boyko_scene/src/render_caps.rs`).

> ⚠️ **A live trap for exactly the code a reflection layer would write:**
> **`EcsMaster::has_component(e, id)` silently returns `false` for every bitset tag.**
> (`ecs_master/component_api.rs:673-702`.) It branches on `StorageKind::Dense` → `dense_contains`,
> but has **no `Bitset` branch** — so a bitset id falls through to the archetype
> `columns` lookup, finds `column.ptr.is_null()` (a bitset tag has no column in any
> archetype, by construction), and reports `false` for a tag the entity demonstrably
> has enabled. An inspector reaching for the obvious type-erased presence probe would
> **report the wrong answer rather than refuse**. Reflection must never call it for a
> non-`Table` id. The correct dispatch is three-way: `Table` → column non-null;
> `Dense` → `dense_contains`; `Bitset` → `is_enabled_id`. *(Whether `has_component`
> itself should grow the third branch is a `boyko_ecs` question, not a reflection
> question — but reflection is how it was found, and it should be reported.)*

### (2) `StorageKind::Dense` — **support it**; it is the one non-table kind with real bytes

Readable via `get_component_raw` and writable via `get_component_raw_mut` (B.3). The
only gap is enumeration. **Dense components are in v1 scope.**

### (3) `ResidencyKind::Gpu` — enumerated but unreadable, a **third shape** of refusal

> `component_registry/mod.rs:475-483` · `archetype/archetype.rs:695-730`

Crossed with the storage axis is a three-way residency axis: `Cpu = 0`, `Gpu = 1`,
`CpuPinned = 2`. A `Gpu` component is **`Table`-kind**, so it **is** in the archetype
signature and **will** be enumerated by `component_ids()` — but once
`make_component_device_backed` runs, the inline `columns[cid]` is deliberately set to
`Column::null()` (never `refresh_column`, which would re-cache a dangling host base),
so `get_component_raw` returns `None`.

The failure mode is **safe but confusing**: the inspector lists a component and then
shows nothing for it, with **no way to distinguish "not present" from "lives on the
device."** v1 must read `residency_class(id)` and say **"GPU-resident — no host
bytes"** explicitly.

Note also: `CpuPinned` exists and must never be flipped to device backing (the archetype
mint `assert!`s on it at `:711~-720`), and **an archetype signature mixing `Gpu` with
non-`Gpu` is rejected outright** at mint — so "some fields on device" is not a state
that can occur.

### (4) Dynamic tags — signature-resident ids with **no Rust type at all**

> `component_registry/tags.rs:12~-45` · `ecs_master/tag_api.rs:47-200`

`EcsMaster::register_tag(name) -> TagId` (`tag_api.rs:65`) mints a **size-0,
`Table`-kind, name-keyed id at RUNTIME**. It **is** in the archetype signature,
`component_ids()` returns it, and `type_info_of(id)` will be `None` **forever**, because
no `#[derive]` ever ran for it. This breaks an unstated premise of §4 — *that every id
in a signature has a Rust type behind it.*

Worse for any `TypeId` cross-check: **all dynamic tags share `DynamicTagMarker`'s
`TypeId`** (`mod.rs`, and `component_registry/tags.rs:12~-16` states the reason — a generic-fn-body static
would collapse across monomorphisations, rust#22991). So **`ComponentId → TypeId` is no
longer injective**, and the premise `BindAccessor`'s own SAFETY comment leans on —
*"`ComponentId` IS the type's identity"* — **does not hold for this class.**

Reflection handles it *correctly by accident*: `type_info_of` returns `None`. Two
requirements follow:
* the inspector must display the **interned tag name** (`TAG_NAMES` has it,
  `tag_by_name` resolves it) rather than a blank row;
* **§5's `debug_assert_eq!` on `TypeId` must never be pointed at one** — it would pass
  for the wrong tag, which is worse than not checking.

### The matrix v1 must implement

| citizen | in signature? | `type_info_of` | v1 disposition | inspector row |
|---|---|---|---|---|
| `Table` + `Cpu` | ✅ | `Some` | **full read/write** | fields |
| `Table` + `Cpu` + no `#[reflect]` | ✅ | `None` | opt-out | *"not reflectable"* |
| `Table` + `Gpu` | ✅ | `Some` | **enumerate, refuse read** | *"GPU-resident — no host bytes"* |
| `Table` + `CpuPinned` | ✅ | `Some` | full read/write (host memory) | fields |
| `Dense` | ❌ (source 2) | `Some` | **full read/write** (B.3) | fields |
| `Bitset` | ❌ (source 3) | *derive refused* | **presence get/set** | boolean toggle |
| dynamic tag | ✅ | `None` **always** | presence only | interned tag **name** |

Three of these rows are the same UI problem in different clothes — *"the id is real,
the value is not viewable here"* — and one shared row state answers all three (see also
the A.5 correction, which raises it a fourth time).

---

## B.5 Aether-generated components: who attaches the opt-in to a struct the user never writes

> `crates/aether_lang/src/expand.rs:166-195` (`fn component`), `:202` (`fn tag`) ·
> `crates/aether_lang/src/ast.rs:268-288`

This is the **concrete answer** to §2/A.5's hardest unstated question. `fn component`
emits, verbatim:

```rust
#[derive(::boyko_macros::Component)]
#requires
#component_attr
pub struct #name { #(pub #fields),* }
```

and `ComponentDef` carries exactly five things — `name`, `fields`, `requires`,
`hooks: Vec<(HookKind, Path)>`, `no_bundle`. **There is no `attrs` field and no
attribute-passthrough grammar anywhere in the AST.** Therefore the §2/A.5 answer
*"the consumer writes it"* is **impossible** for an Aether component: the user has no
syntactic slot. **The macro must add it.**

### Three changes, and they cost `aether_lang` no new dependency

1. **`ast.rs`** — add `reflect: bool` to `ComponentDef`.
2. **`parse.rs`** — accept a `reflect` item/flag inside a `component` block.
   **Opt-in**, matching A.5's `#[reflect]` semantics — *unless* the owner decides
   Aether components are reflectable by default, which is a defensible call for a
   gameplay-authoring DSL and should be asked rather than assumed.
3. **`expand.rs`** — push a `reflect` key into the existing `component_attr` keys
   vector, so the emission becomes `#[component(reflect, …)]`. This is **free**:
   `component_attr` is already the assembled-keys mechanism (it is how `no_bundle` and
   every `on_*` hook get in).

Crucially, the emitted `#[component(reflect)]` is a **token resolved downstream** —
exactly the tokens-not-deps rule `aether_lang`'s own manifest already states: *"this
crate does NOT depend on `boyko-ecs`: every `boyko_ecs::…`/`boyko_macros::…` occurrence
in the expander is an emitted TOKEN resolved in the downstream crate"*
(`crates/aether_lang/Cargo.toml:6~-11`). A.5's subtle jurisdictional point — *"the
directional rule is about crate deps, not emitted tokens"* — has been **independently
reinvented and is now house doctrine, stated verbatim in a manifest.** That is
confirmation, not a conflict.

### Two second-order consequences

**(a) A.5's naming convention becomes a language-level requirement.** A.5 says *"the
consumer's reflect-enabling feature MUST be named `reflect`."* Once Aether emits the
opt-in, **every crate containing an `aether!` block inherits that requirement** —
silently, from a macro. That belongs in the Aether language documentation, not buried
in a reflection appendix.

**(b) The refusal needs a good span.** `fn tag` (`expand.rs:202`) emits ZST components,
and `tag NAME(bitset);` emits `#[component(storage = "bitset")]` — so **Aether can
already produce the exact types M6 refuses**. The user wrote `tag Foo(bitset);` and
would get an error about a derive they never typed. Aether already solves this class
(`quote_spanned! {name.span()=> …}`, with a recorded measurement of what happens
without it: *"rustc's 'previous definition of the type `Foo` here' pointed at
`aether! {`"*), so the fix is to keep the refusal spanned at the user's name.

**(c)** Aether has **no `dense` construct** today, so Aether components are always
`Table`-kind. B.4's dense row cannot arise from Aether — but it can arise from
hand-written Rust, so the matrix still needs it.

---

## B.6 The ship gate: the tree has already built this instrument and **measured how its naive form cannot fail**

> `crates/profile_fixture/tests/profile_axis_census.rs` · CI job *"cross-profile symbol
> census (G14/G16)"* (`.github/workflows/ci.yml:131-153`)

§2 and §7 present the ship gate — *"`cargo tree` must show `boyko_reflect` absent, and a
symbol-absence check must pass on the exact ship artifact"* — as straightforward
deliverable plumbing. It is not, and the tree has the receipts. Measured on **this exact
target** (`x86_64-pc-windows-gnu`, release):

| link configuration | `dev` leg | `shipping` leg | decidable? |
|---|---|---|---|
| default release | symbol = 1 | symbol = **1** | **no** |
| `-C link-arg=-Wl,--gc-sections` | 1 | **1** | **no — *no effect whatsoever*** |
| `lto = "fat"`, `codegen-units = 1` | 1 | **0** | ✅ yes |

The reason is in the same output: a default-release image contained
`core::ptr::drop_glue::<boyko_diag::telemetry::Block>` **in a binary whose source never
mentions telemetry**. A dependency rlib's plain functions are codegen'd and carried into
the image whether or not anything can reach them. So a whole-image census answers *"was
this symbol codegen'd into some rlib on the way here?"* — not *"can this program reach
it?"* Those are different questions, and only the second is the gate's.

Its stated method generalizes and should be **cited directly rather than
rediscovered**: *ask what kind of symbol it names, not which subsystem it belongs to.* A
**generic** function only exists if some site instantiated it (decidable without LTO); a
**plain function in a dependency's rlib** is codegen'd regardless (not decidable without
LTO).

### What this means for reflection specifically

**Reflection is the FAVOURABLE case, and that is exactly why the gate could pass for the
wrong reason.** With the feature off, `boyko_reflect` is **not in the resolved
dependency closure at all** ⇒ no rlib is built ⇒ there is nothing to carry into the
image. So:

* **`cargo tree` is the load-bearing half.** It tests the *resolver*, which is where the
  actual property lives (§2's whole argument is about the feature closure).
* **`nm`/`llvm-nm` is corroboration, not proof.** This document presents both as equally
  decisive and never mentions the link-configuration precondition — so **a Wave-0
  implementer following it literally would build a gate that passes for the wrong
  reason**, and would never see it fail.

**Two things the reflection gate must copy from the fixture:**

1. **A present control beside the absent cell** — the 2×2 discipline. *"A `shipping`
   binary with no emission symbol is ambiguous on its own: it is equally consistent with
   'the fold deleted this site' and with 'a shipping build contains no profiler at
   all'."* For reflection the natural control is a **feature-ON build of the same
   fixture**: same source, feature flipped, symbol present. Without it, "no symbol" is
   indistinguishable from "no fixture."
2. **`components: llvm-tools` is a RED, not a skip.** The census job treats a missing
   `llvm-nm` as failure, because *"a gate that passes on every machine lacking its tool
   is a gate that passes."*

And one discipline worth copying wholesale: that fixture **builds its own two legs**
rather than consuming other jobs' artifacts, explicitly so it is runnable locally —
*"a gate only CI can run is a gate whose RED nobody has seen."*

---

## B.7 `BUG-MIGRATE-TB-1` — a second, differently-shaped Tree-Borrows rule, and it lands on enumeration

> Commit `43684a58` (2026-06-21) *"re-derive the structural-write archetype pointer under
> the live protector (Tree-Borrows UB)"* · cited at every raw-column read site, e.g.
> `ecs_master/component_api.rs:~213-233` and `:~688-700` ·
> `system/dispatcher_token.rs:326-345`

§5's hazard list is anchored to 14a-F2 / the Phase-19 `command_queue` twin (cached
pointer + reborrow). That class is still real. **This is a different one**, and it is
not in the list:

**Do not form `&Archetype` or `&mut Archetype` at all.** Read the single `Column`
through a **raw-pointer projection**:

```rust
let columns_ptr = core::ptr::addr_of!((*archetype_ptr).columns).cast::<Column>();
let column = *columns_ptr.add(component_id.0);      // Column is Copy
```

The in-tree SAFETY comment states why: a `&Archetype` covers the **whole struct**
(including `current_index`); a sibling structural migration writes `current_index`
through a same-cell-derived pointer, transitioning the interior-mutable slab cell to
`Active`. A shared (foreign) read would then **FREEZE** that cell — and the
`Box`-of-slab deallocation on `EcsMaster` drop is **forbidden through a `Frozen` tag**.

**This lands squarely on reflection**, because enumeration is precisely the code that
wants to walk an archetype's columns (B.3). Wave 3's glue must use the projection form,
and Wave 3's Miri-TB run must cover it.

### The precedent cuts both ways — and that is now a decision, not a gap

The shipped `Bindable` trampolines **do** form a shared reference off the column
pointer:

```rust
let this: &#name = unsafe { &*(p as *const #name) };   // boyko_macros/src/bindable.rs:113, :119
```

— a shared retag off arena-rooted `SharedReadWrite` provenance, which has evidently
passed Miri. So the tree contains **in-tree precedent on both sides**: refuse the
struct-wide reference *for the archetype*, permit the typed shared reference *for the
component row*. They are not in conflict — the archetype struct has a concurrently
sibling-written field and the component row does not — but the distinction is subtle and
**the reflection design must state which pattern it adopts where**, rather than
inheriting whichever it happened to copy:

* **archetype / column-table access** → raw projection, **never** a reference
  (`BUG-MIGRATE-TB-1`);
* **component row access inside a monomorphized accessor** → the `Bindable` pattern is
  available and precedented, **but** A.4 already refuses it for the `String` *setter*
  (the `Unique` retag through interior-mutable provenance is the 14a-F2 class). So:
  **shared reads may take the `Bindable` form; every writer stays raw.**

That asymmetry is the actual rule, and it was not written down anywhere before this
revision.

---

## B.8 Fixed-size arrays `[T; N]` — the real v1 coverage question, and a hard error today

> `boyko_render/src/gpu_transform3d.rs:55-62,:84~-89` · `boyko_render/src/csm_config.rs:392`
> · `boyko_render/src/ddgi_config.rs:76` · `boyko_render/src/frustum.rs:33` ·
> `boyko_physics/src/soft/component.rs:69-87`

Arrays are **absent from A.2's taxonomy and from A.1's v2 exclusion list** — they appear
in neither. So they fall through to `Opaque`, and A.6's `Opaque` rule is a **hard
error**. The consequence is that today's design would make the flagship dense component
**un-derivable** rather than partially inspectable:

```rust
#[repr(C)] pub struct TrsPacked { pos: [f32;4], rot: [f32;4], scale: [f32;4] }   // 48 B
#[repr(C)] #[component(storage = "dense")]
pub struct GpuTransform3D { prev: TrsPacked, curr: TrsPacked }                    // 96 B
```

The wider render surface is the same shape: `csm_config.rs:392`
`view_proj: [[f32;4];4]`, `ddgi_config.rs:76` `origin: [f32;3]`, `frustum.rs:33`
`type Plane = [f32;4]`. And the engine's *string* idiom is an array too —
`UiName { bytes: [u8; CAP], len: u8 }`, `UiTextBuffer` (§6.1(b)).

**`[T; N]` of a `Prim` is cheap and mechanical**: offset + stride + count, all `const`
via `offset_of!` + `size_of::<T>()`, **zero alloc, no drop, no TB exposure**. It is
**strictly easier than the `String` support this document spent its headline CRITICAL
on** — and unlike `String`, it has consumers. It belongs in v1 (Wave 1).

> **Display note.** The inline-string components (`UiName`, `UiTextBuffer`) are
> `[u8; CAP]` + `len`, with **private** fields. `offset_of!` in the derive works (the
> expansion is in the defining module), but rendering 60 separate `u8` rows is useless
> to an inspector when the meaningful view is the decoded `&str`. Whether v1 gains a
> `#[reflect(as_str)]`-style field hint is **left open** — it is a UX question, not a
> soundness one, and it should not delay the taxonomy fix. Recorded in
> `docs/OPEN-QUESTIONS.md` rather than decided here.

**`Vec<T>` stays v2 — but its cost is now concrete.** `SoftBody` is an ordinary
`#[derive(Component)]` with **fourteen** `Vec<f32>`/`Vec<u32>` columns
(`soft/component.rs:69-87`): 100 % v2-deferred kinds, so under A.6 it is not merely
uninspectable but a **compile error** if anyone derives `Reflect` on it. That is the
right outcome for v1 — provided the refusal is documented and opt-out-able (A.6
correction), not a surprise.

**Re-shaped scope (B), for the owner, since the fork is reversible by construction:**
if v1 covers **arrays-of-`Prim`** and defers **`String`**, it inspects `GpuTransform3D`,
`Transform`, `Visibility`, `ParticleEmitter`, `ParticleEffectHandle` and `Name` — i.e.
**it actually dogfoods** — at *less* risk than currently-specified (B), because the
whole `drop_in_place` + `ptr::write` TB surface is deferred with it. **(B) as taken
remains the decision**; this is the recommended *build order* within it (§6.1(d)), and
it is worth putting to the owner explicitly because it changes what Wave 2 ships first.

---

## B.9 CI Miri is an **allowlist**, not a sweep — A.9's mandatory gate would not exist

> `.github/workflows/ci.yml:193-226`

A.9 declares Miri-TB **mandatory** on four surfaces (String replace, nested-leaf write,
enum discr, nested offset composition). The premise that this is self-enforcing is
false. CI runs Miri as **two named tests** plus a **hand-listed** package sweep:

```
-p boyko-ecs -p boyko-utils -p boyko-threadpool -p boyko-serialize
-p boyko-math -p boyko_sdf_math -p boyko_image
```

The list is deliberate and the reason is recorded in the workflow: **Miri cannot execute
FFI**, so a literal workspace sweep would fault the instant it reached
`boyko_rhi_vulkan`'s raw `vk*` calls. *(The same comment records that this step's
`--all-targets` was itself once the vacuous root-package form — the 2026-07 audit found
it interpreting nothing.)*

**A new `boyko_reflect` package is NOT covered by that list.** Unless Wave 0 adds it
explicitly, A.9's mandatory gate is **a gate that cannot fail** — which is the exact
failure mode this project has recorded repeatedly, and which the profiling census
(B.6) independently rediscovered.

**Wave-0 deliverable:** add `-p boyko-reflect` to the pure-compute sweep **and** verify
it runs by landing a deliberate RED first. `boyko_reflect` is pure-compute (no FFI), so
it belongs in that list on the merits. ~~Note the sweep must run with the **feature ON**,
or it compiles an empty crate and reports green.~~

> **The last sentence is WRONG and is struck 2026-08-21 (second pass).** It was inherited
> verbatim by four downstream documents, and following it literally produces a CI line that
> does not run at all. Two facts kill it:
>
> 1. **`boyko_reflect` carries no `reflect` feature and never will.** The
>    `#[cfg(feature = "reflect")]` in the derive's output is a **consumer-side** construct —
>    a `cfg` in derive output is evaluated in the crate the derive expanded into (§2, A.5).
>    A self-hosted feature on `boyko_reflect` would test a configuration no consumer ever
>    has, and would put a `reflect` feature on the one crate whose absence is the entire
>    claim. So `cargo +nightly miri test -p boyko-reflect --features reflect` is a **hard
>    cargo error** — *"none of the selected packages contains these features: reflect"* —
>    not a stricter run.
> 2. **With "the feature off" the crate is not empty.** Nothing in
>    `crates/boyko_reflect/src/**` is `cfg`-gated; its whole contents always compile. The
>    arithmetic, the registry and the `prim::` accessors over hand-built pointers are
>    covered by a **plain** `-p boyko-reflect` row.
>
> **The corrected obligation is two rows, not one:**
>
> ```
> cargo +nightly miri test --all-targets \
>   -p boyko-ecs … -p boyko-reflect \
>   -p reflect-fixture --features reflect-fixture/reflect
> ```
>
> * `-p boyko-reflect` **plain** — the arithmetic, the registry, the `prim::` accessors.
> * `-p reflect-fixture --features reflect-fixture/reflect` — the **only** row that reaches
>   any derive-generated `unsafe`, because only a consumer can carry a `#[component(reflect)]`
>   type. The multi-package `pkg/feature` spelling is load-bearing: a bare `--features reflect`
>   across a multi-package selection is ambiguous and is the form that silently selects nothing.
>
> Two further constraints on that second row, both of which turn it red for reasons unrelated
> to reflection if they are missed:
>
> * **Miri cannot spawn processes**, and the Miri sweep runs `--all-targets`. Any test in the
>   fixture that shells out — the absence census and the codegen-identity harness both run
>   `Command::new(env!("CARGO"))` — must carry `#[cfg(not(miri))]`. The template they copy
>   (`crates/profile_fixture/tests/profile_axis_census.rs`) has no such guard **because
>   `profile_fixture` is not on the allowlist**; copying it verbatim onto the allowlist
>   imports a failure whose likeliest "fix" is dropping `-p reflect-fixture`, which silently
>   reverts this whole section. The benches need the same guard for the same reason.
> * **Miri cannot execute FFI**, which is why the sweep is hand-listed at all. So the Miri
>   fixture must depend on `boyko-ecs` / `boyko-macros` / `boyko-reflect` **only** — which
>   means the Miri row and the *dogfood-against-real-engine-types* row are **two different
>   tests in two different packages**. See **B.12**.

---

## B.10 Particles are a clean, entirely-v1 consumer — not an obstacle

> `crates/boyko_render/src/particle.rs:125~-191`

Recorded because the campaign brief flagged particles as a possible obstacle. They are
not; they are a **model consumer**, and they exercise three of B.4's rows at once:

| type | shape | reflection disposition |
|---|---|---|
| `ParticleEmitter { rate: f32, accumulator: f32, burst: u32, speed_scale: f32 }` | 16 B, `#[repr(C)]`, layout-pinned, all POD | **full read/write, v1** |
| `ParticleEffectHandle(pub u32)` | `#[repr(transparent)]`, tuple struct, `on_insert`/`on_replace` hooks | **full read/write** — and a live test that `set` fires hooks |
| `EmitterActive` | ZST `#[component(storage = "bitset")]` | **B.4 presence view** |

`ParticleEffectHandle` is a particularly good acceptance case: it is a **tuple struct**
(A.6's `"0"` naming rule), it is `#[repr(transparent)]` over a `u32`, and it carries
lifecycle hooks — so writing it through reflection is the natural test that §4's
"inherit hooks/observers/change-detection for free" actually holds through the new
by-id structural seam (§4 correction).

The **GPU-resident particle state proper is not ECS component data**, so it poses no
reflection question at all.

---

## B.11 Open decisions this revision opens or leaves open

Recorded here (and mirrored into `docs/OPEN-QUESTIONS.md`) rather than buried, per the
project's surface-difficulties-immediately rule.

| # | Decision | Owner or engineering? | Blocks |
|---|---|---|---|
| 1 | **`BindAccessor`: one table or two** (B.1) — Horn 1 lets a shipping crate consume reflection metadata; Horn 2 duplicates field descriptions | **OWNER** (shipping API surface) | Wave 1 |
| 2 | **A public by-id structural seam on `EcsMaster`** (§4) — widens a *shipping* crate's API for a dev-only feature | **OWNER** (API surface); engineering owns the shape | Wave 3 |
| 3 | **Are Aether components reflectable by default or opt-in?** (B.5) | **OWNER** (DSL ergonomics) | Wave 2 |
| 4 | **Build order within (B): arrays before `String`** (§6.1(d), B.8) — recommended, changes what Wave 2 ships first | **OWNER** (scope) | Wave 2 |
| 5 | `#[reflect(as_str)]` for inline `[u8; CAP]` + `len` string components (B.8) | engineering, deferrable | — |
| 6 | Does `has_component` grow a `Bitset` branch? (B.4) — a `boyko_ecs` bug found *via* reflection | engineering | — |
| 7 | Tuple structs: A.6 accommodates, `Bindable` rejects (A.6 note) — only a conflict if fork #1 takes Horn 1 | engineering | follows #1 |

**Not open, recorded as settled:** the §0/§2 central finding (feature-gated optional
crate + CI absence gate, *not* `cfg(debug_assertions)`) — **unchanged, not
re-litigated**. The §6 scope fork — **TAKEN: option (B)**, with reason, reversible
(§6).

> **Superseded as a list, 2026-08-21 (second pass).** The seven rows above are still correct
> individually, but they are not the *whole* set and two of them are the same decision. The
> single owner-facing list is **B.13**; this table now feeds it rather than competing with it.

---

## B.12 Engine-owned components: who writes the opt-in when the consumer IS the engine

> The third leg beside **§2** ("the consumer writes the opt-in") and **B.5** ("the consumer
> never writes the struct"). §2's answer silently assumed the consumer is the **game**, owning
> its own components. B.5 caught the *Aether* variant. This is the **engine-crate** variant,
> and it is the largest of the three: it covers every component the engine itself ships, which
> is every dogfood target every plan names.

### The problem, stated mechanically

The opt-in is `#[component(reflect)]`, and the derive wraps its emission in
`#[cfg(feature = "reflect")]`, which is **evaluated in the defining crate**. Every dogfood
target lives in a shared engine crate:

| target | file | why it is named |
|---|---|---|
| `Transform { translation: Vec3, rotation: Quat, scale: Vec3 }` | `boyko_scene/src/transform.rs:46` | the `Nested` depth-2 case (A.9) |
| `Name(pub NameId)` / `NameId(pub u32)` | `boyko_scene/src/identity.rs:47` | tuple struct + `Nested` |
| `Visibility` `#[repr(u8)]` | `boyko_scene/src/render_caps.rs:226` | the top-level `TypeKind::Enum` case |
| `GpuTransform3D` / `TrsPacked` | `boyko_render/src/gpu_transform3d.rs:84~` | `storage = "dense"` **and** `[f32;4]` arrays — B.3's and B.8's joint case |
| `EmitterActive` | `boyko_render/src/particle.rs:164` | the bitset presence view (B.4) |

Writing `#[component(reflect)]` on any of them forces its **defining** crate to declare a
`reflect` feature and an optional `boyko-reflect` edge. **The declaration cannot be dodged by
omission**, because of an instrument the workspace already carries: root [`Cargo.toml`](../Cargo.toml)`:25-26`
sets `[workspace.lints.rust] unexpected_cfgs` with a `check-cfg` list that **adds to** Cargo's
per-manifest feature list rather than replacing it, and every member opts in via
`[lints] workspace = true` — so a `#[cfg(feature = "reflect")]` in a crate with no such feature
**warns**, and the existing `cargo clippy --workspace --all-targets -- -D warnings` gate
promotes the warning to red. (That is a *good* property — it is why a missing feature is a
diagnostic rather than silence — but it removes "just don't declare it" as an option.)

**And `boyko_scene` is not a leaf.** Measured: `boyko-scene` is depended on by **five**
workspace members — `aether_tests`, `boyko_app`, `boyko_physics`, `boyko_render`, `boyko_ui`.
So the rule "a member declaring a feature named `reflect` must be depended on by no other
workspace member" reds mechanically the moment the first dogfood target opts in.

### Three candidates, each with its cost

**(a) A leaf `reflect_engine_types` crate that hand-bakes `TypeInfo` statics** and calls
`install_type_info(Transform::component_id().0, &TRANSFORM_INFO)` explicitly. Works today —
every dogfood target has `pub` fields. Puts the feature only on a leaf. Costs nothing in a ship
build. **Rejected**, for three reasons, the third of which is decisive:

* it re-opens the lazy-vs-explicit registration argument (§A.5 / the CORE plan's D4) for
  **exactly** the components whose silent absence that decision says the inspector cannot
  tolerate — a forgotten row there is *"this entity doesn't have that component"*;
* it cannot reach private-field types at all (`UiName { bytes: [u8; CAP], len: u8 }` — B.8's
  display note), so those would have to be declared out of v1 explicitly;
* a hand-written `TypeInfo` beside the struct is **a second, independent description of the
  struct's fields** — the drift class **B.1 Horn 2** warns about and **B.2**/D8 refuse
  elsewhere in this same design. A field renamed in `boyko_scene` and not in the leaf produces
  a *confidently wrong inspector*, and nothing catches it.

**(c) Restrict v1's dogfood to fixture-local clones** of the engine shapes, and delete the
phrase "real engine types" from the plans. Cheapest. **Rejected:** it costs the campaign the
one claim that makes reflection worth building. A layer proven only against copies of the
engine's shapes has not been shown to work on the engine.

**(b) Let engine crates carry the `reflect` feature, and replace the leaf rule with one that
survives unification. — TAKEN.** The objection to (b) is that no such rule exists, on the
strength of `boyko_ecs`'s own recorded measurement. That measurement is real and is quoted
correctly — *"while it was default-on there was NO command line that could turn it off"*
(`crates/boyko_ecs/Cargo.toml`, the `profiling-analysis` block) — but it is **a measurement
about a DEFAULT-ON feature**, and its own conclusion was not "shared crates cannot carry
features"; it was *"opt-in is the only shape in which the flag means what it says. Enable it
the same way `hwrt` and `bench-alloc` are enabled: `--features boyko-ecs/profiling-analysis`."*

The tree then went and built the general case and it is three crates deep:

```
boyko_rhi_vulkan   hwrt = []                        (a shared crate, 
boyko_render       hwrt = ["boyko_rhi_vulkan/hwrt"]  five dependents on the chain,
boyko_app          hwrt = ["boyko-render/hwrt"]      default OFF at every level)
```

`hwrt` is a non-default feature on three **non-leaf** shipping crates, forwarded twice, and it
does **not** reach any ship build — because nothing enables it. `boyko_render`'s
`test-readback` is the same shape with a different enabler (a self-referential
dev-dependency, `crates/boyko_render/Cargo.toml:103~`). Both are load-bearing today.

The one thing wrong with `hwrt` is **F17**: `grep -c hwrt .github/workflows/ci.yml` = **0**, so
every `#[cfg(feature = "hwrt")]` body in the tree is compiled by no CI leg. That is a *coverage*
defect, not a *containment* defect — and it is the half this design must not inherit.

### The rule that replaces "leaf-only" — five clauses, all decidable from `cargo metadata --no-deps`

Each clause names the failure it catches; together they are strictly more capable than the leaf
rule, because they permit the dogfood and still forbid everything that can actually fire.

| # | clause | catches |
|---|---|---|
| **C1** | No member's `default` list transitively enables a `reflect` feature or names `boyko-reflect`. | the `profiling-analysis` failure verbatim — a default that unification restores through siblings that never wrote `default-features = false` |
| **C2** | Every dependency edge naming `boyko-reflect` has `optional == true`. | a non-optional edge, which is in the closure unconditionally |
| **C3** | A feature that pulls the crate is written exactly `["dep:boyko-reflect", …]`. | the bare `["boyko-reflect"]` form, which implicitly mints a *second* feature and is how a consumer silently gets an always-on optional dependency |
| **C4** | **No dependency edge of any kind carries `reflect` or `<pkg>/reflect` in its `features` array.** Enablement is by a `[features]` forward or by a command line — never by an edge. | the one form nothing can turn off: the same manifest records that *"an explicit `features = [...]` survives `--no-default-features` by design"* |
| **C5** | **No ship-target member (`boyko_demo`, the root `boyko-engine`) declares or forwards a `reflect` feature.** | the narrow, checkable remainder of the old leaf rule, applied where unification can actually reach a shipped artifact |
| **C6** | **Every command line in the repo that enables a `reflect` feature is a named row in `tests/reflect_ci_coverage.rs`.** | **F17** — the `hwrt` half. A feature-gated body compiled by no leg is invisible to the default gate; measured, not feared |

Non-vacuity is mandatory on the census (≥ 1 member declaring `reflect`, ≥ 1 optional edge, ≥ 1
named enabling invocation), for the reason the corpus-witness test states: *a check whose
subject can vanish while the check stays green is not a check.*

### Two consequences the plans must carry

**1. The dogfood and the Miri fixture are two different packages, and neither is optional.**
Miri cannot execute FFI, and `boyko_render` → `boyko_rhi_vulkan`. So:

| package | deps | role |
|---|---|---|
| `reflect_fixture` | `boyko-ecs`, `boyko-macros`, `boyko-reflect` **only** | the **primary** gated subject: the Miri row, the absence census, the codegen-identity twins, the trybuild corpus. Reproduces the engine's *shapes* locally (a `storage = "dense"` struct of `[f32; 4]` arrays, a fieldless `#[repr(u8)]` enum, a tuple struct). |
| `reflect_dogfood` | the above **plus** `boyko-scene`, `boyko-render`; `reflect = ["dep:boyko-reflect", "boyko-scene/reflect", "boyko-render/reflect"]` | the **real-engine-types** acceptance test. Not on the Miri allowlist, by construction. |

The fixture's local shapes are therefore **the primary subject, not a stand-in**, and the
dogfood is an additional claim proved separately. Nothing is lost: the two claims were never
the same claim, and pretending they were is what produced a Miri obligation nobody could run.

**2. The enabling invocation unifies workspace-wide, and that is fine as long as it is never
mistaken for the ship invocation.** `cargo test -p reflect-dogfood --features
reflect-dogfood/reflect` turns `boyko-scene/reflect` on for every selected member, including
`boyko_demo`. The ship gate is a *different* invocation — `cargo tree -p boyko_demo -e features
--edges normal,build`, **with no `--features`** — and its harness must assert that it passed
none. Per-leg `CARGO_TARGET_DIR` on the census legs keeps the artifacts apart.

**Reversibility.** If the owner declines (b), (c) is one commit away: delete the `reflect`
feature from `boyko_scene`/`boyko_render`, delete `reflect_dogfood`, and strike the phrase
"real engine types" from the four gates that use it. Nothing else in the design moves —
which is why this is recorded as a decision with a stated flip cost rather than a foundation.

---

## B.13 The owner decision sheet — ONE list

Four decisions were spread across three plan documents with no single list, two of them were
the same decision described twice, and the largest of them was not on any list at all. This is
the list. Each row states what is decided, who decides it, what it blocks, and what the plan
does **while it waits** — because a rung that stops dead on an unanswered question is how a
campaign stalls, and a rung that proceeds silently is how an owner is presented with a fait
accompli.

| # | Decision | Owner or eng.? | Blocks | Plan's position while it waits |
|---|---|---|---|---|
| **1** | **May engine crates carry a `reflect` feature?** (B.12) — the largest, because it decides whether v1 can inspect the engine's own components at all. Yes ⇒ `boyko_scene`/`boyko_render` gain a non-default `reflect` feature + an optional `boyko-reflect` edge, governed by B.12's C1–C6. No ⇒ v1's dogfood is fixture-local clones and the phrase "real engine types" leaves four gates. | **OWNER** (shipping manifest surface + the dogfood claim) | GATES G0/G1; CORE C6/C10; ECS EG8; BOUNDARY B5 | **proceeds on (b)**, with C1–C6 as the census rule and `reflect_dogfood` as a separate leaf package |
| **2** | **The by-id `boyko_ecs` seam — FOUR public items, one decision.** `add_component_by_id` (S1), `remove_component_by_id` (S2), `mark_component_changed` (S3), `EnableTagId::try_from_component_id` (S4′). *Previously filed twice — as B.11 #2 and as the BOUNDARY plan's B-1 — because the fourth item was reached from two directions.* It is one call: a dev-only feature widening a **shipping** crate's public API. Each item's independent merit is stated in the ECS plan's §4 table. | **OWNER** (API surface); engineering owns the shapes | ECS EG2, and through it EG3/EG5/EG6; BOUNDARY B4 | **blocked** — EG2 does not start before the answer. EG1/EG4 are built against the items that already exist |
| **3** | **`BindAccessor`: one table or two?** (B.1) — Horn 1 lets a *shipping* crate consume reflection metadata; Horn 2 duplicates the field description and buys a drift test. | **OWNER** (shipping API direction) | CORE C3/C8; the wording (not the construction) of the absence gate | **proceeds on Horn 2**, and pays its price at CORE C8 gate 5 (the `field_id(name)` ↔ reflect-index agreement test) |
| **4** | **Are Aether components reflectable by default, or opt-in?** (B.5) | **OWNER** (DSL ergonomics) | the Aether seam only | **proceeds on opt-in.** Flip cost, stated precisely: default-on makes a `reflect` feature declaration **mandatory** in every `aether!`-bearing crate under the existing `-D warnings` gate (B.12's `unexpected_cfgs` mechanism), including crates that never asked for reflection |
| **5** | **Build order inside (B): arrays → nested → enums → `String` last.** (§6.1(d), B.8) — changes what the first inspector can show. | **OWNER** (scope) | what the derive ships first | **taken**, and it is the order all four plans are written in |

**Engineering-owned, recorded here only so they are not mistaken for owner calls:**
`#[reflect(as_str)]` for inline `[u8; CAP] + len` (B.8); whether `has_component` grows a
`Bitset` branch (B.4 — a kernel question found *via* reflection); whether
`register_stable_name` becomes total over bitset types (the BOUNDARY plan's B-3); the
tuple-struct conflict (A.6 note), which only exists if row 3 takes Horn 1.

**Rows 1 and 2 are the two that block rung 0 and rung EG2 respectively, and they are the two
to put in front of the owner first.**
