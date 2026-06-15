# `boyko_reflect` — deep design analysis

Snapshot 2026-06-15, branch `ecs`. Produced by a research → architect →
critic(×2 independent) pipeline (bevy_reflect / flecs / EnTT / Unity DOTS /
Unreal study + boyko source grounding). This document folds the two critique
rounds' CRITICAL/MAJOR findings back in as **resolved decisions**, and marks the
one genuinely-open scope fork for the owner.

Status: **DESIGN — not implemented.** No code exists yet (`grep` confirms no
`Reflect` trait / serde in tree).

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
| **`boyko_serialize`** (future, already deferred per `REMAINING-GAPS.md`) | Compile-time **codegen** (de)serialization for save/load, replication, baked prefabs | **Yes** | `boyko_ecs` only — **NEVER `boyko_reflect`** |

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

This also resolves the **marker-feature footgun** (the draft put a `reflect`
marker feature on `boyko_ecs`; a critic showed workspace feature-unification could
then flip it on for a ship binary's `boyko_ecs`). With the gate on the consumer's
feature, `boyko_ecs` carries **no reflect feature at all** and the directional
rule holds unconditionally.

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
across the selected packages; a workspace-wide `cargo build`/`cargo bench` where
any member (e.g. `bench_bevy_vs_boyko`, `boyko_demo`) enables `reflect` unifies it
ON for the ship crate too. Resolver v2 (edition 2024) does **not** de-unify plain
workspace deps. Therefore:

- The 0%-cost claim is **demoted to**: *zero cost in a correctly-partitioned ship
  build, enforced by a CI gate as a deliverable* — `cargo tree -p game_app
  --release` must show `boyko_reflect` absent, and a symbol-absence check must pass
  on the exact ship artifact. `reflect` must never appear in any `default` features
  nor in any crate in the ship binary's dependency closure.
- The Wave-5 hot-path bench asserts "no delta on the **steady-state query/spawn
  inner loop**," not "no delta" (first-touch `TYPE_INFO` registration is real cold
  cost when the feature is on, and must not be mistaken for a hot-path regression).

---

## 3. The fast core (the part that genuinely beats bevy_reflect)

### 3.1 Registry: dense `ComponentId`-indexed table — the real win

bevy resolves a type via `HashMap<TypeId, TypeRegistration>` **plus** a second
`HashMap<TypeId, Box<dyn TypeData>>` = two hash probes + a downcast per access.
boyko has dense, 0-based `ComponentId`, so:

```rust
// boyko_reflect::registry — mirrors the proven HOOKS / STORAGE_KIND / LAYOUTS
// write-once OnceLock discipline (component_registry.rs), but lives in boyko_reflect.
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

```rust
#[repr(C)]
pub struct TypeInfo {
    pub type_name:   &'static str,        // std::any::type_name — diagnostics only
    pub stable_name: &'static str,        // module-qualified path or #[reflect(name=…)] — the SAVE key (O3)
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
type_info_of(id)` → `fp = base.add(f.offset)` → `(f.get)(fp)`. `add_default` /
`remove` route through the **existing** structural insert/remove (the same path
`Commands` uses), so they inherit hooks/observers/change-detection for free and
are already off the iteration hot path.

**Resolutions folded in:**
- **`name`/`TypeId` → `ComponentId` resolution lives in `boyko_ecs`** (alongside
  `LAYOUTS` / `type_name` / `TAG_NAMES`), not in the Debug-only crate — because the
  *shipping* save/load consumer needs it and is forbidden from depending on
  `boyko_reflect` (M2). Reflect *uses* it; it does not *own* it.
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
- **`repr(Rust)` is fine — no `repr(C)` requirement** (`offset_of!` reads the
  compiler's actual offset in the same compile). Cross-process stability is achieved
  by **serializing by field name, never by offset/index**.
- **`#[repr(packed)]` is rejected by the v1 derive** (taking `&field` on a packed
  type is UB; the `*_unaligned` ops are deferred).
- **Generics are rejected by the v1 derive** (a per-impl `static TYPE_INFO`
  collapses across monomorphizations — the documented Bundle / Phase-12.5
  `static SLOT` / Phase-17 `State<S>` trap). The diagnostic points at the future
  keyed-cell path. **Known exclusion:** the engine's own generic `State<S>` cannot
  be inspected in v1.

---

## 6. Open scope fork for the owner (the one genuine decision left)

Everything above is resolved. The remaining fork is **v1 field-type coverage**,
because it changes both effort and whether v1 can dogfood a real inspector:

- **(A) POD-only v1** — primitives + nested-`Reflect` structs. Fastest to ship,
  but cannot inspect `Name(String)`, `Children(Vec<Entity>)`, `Option<Parent>`, or
  `#[repr(u8)] enum State` — i.e. cannot fully inspect the engine's own components.
  If chosen, the v1 *advertised* purpose must drop "editor/inspector" and say
  "primitive + nested-struct field viewer."
- **(B) POD + `String` + nested + `#[repr(Int)] enum` v1** — can dogfood the
  inspector on `Name` / `Transform` / `State`. More work (specify the `Nested` /
  `Opaque` recursion contract + its allocation audit now), and the `Opaque` path is
  where any `Box`/`Vec` allocation pressure will live — must be specified, not
  hand-waved. Collections (`Vec`/`Map`) still deferred to v2.

Recommendation: **(B)** — a reflection layer that can only read `f32`/`u64` is not
an inspector, and the whole justification for reflection is tooling that handles
arbitrary components.

---

## 7. Phased plan (once the fork is decided)

- **Wave 0** — crate skeleton + feature wiring + import `MAX_COMPONENTS` +
  CI matrix (with/without feature) + the `cargo tree`/symbol-absence ship gate.
- **Wave 1** — registry (`REFLECT`, `install_type_info`, `type_info_of`) +
  `Scalar`/`ValueKind`/`TypeKind` + the `prim::` fn-ptr library.
- **Wave 2** — `#[derive(Reflect)]` (field walk, `offset_of!` baking, `Reflect`
  impl, `IS_REFLECT` const + `component_id()` install append, generics/packed/bitset
  rejection). trybuild for the rejections; `cfg_attr`-off compiles to nothing.
- **Wave 3** — ECS glue (`get/set_field`, by-name, `add_default`/`remove`). Miri-TB
  on every offset/raw/default path; proptest get/set roundtrip.
- **Wave 4** — boundary serialize (`Sink`/`Source`) + once-per-type name↔id
  resolution (in `boyko_ecs`) + name-keyed roundtrip incl. simulated id-reorder.
- **Wave 5** — perf validation: hot-loop 0%-gate (feature on vs off) + `get_field`
  vs bevy-shaped baseline + ship symbol-absence.

---

## 8. Preserved strengths (do not lose these in implementation)

- Dense-`ComponentId` array registry (one acquire-load + branch) — faster than
  bevy's double-hash; mirrors the proven `HOOKS`/`STORAGE_KIND`/`LAYOUTS` discipline.
- Lazy first-call registration via the existing `component_id()` `OnceLock` —
  avoids the `linkme` dead-strip / init-order hazard class entirely.
- Offset + flat fn-ptr field model — no per-field `&dyn`, no `Box`, no
  `DynamicStruct`/`FromReflect` value-tree allocation.
- `repr(Rust)` allowed + serialize-by-name — a real win over manual offset tables,
  and defuses the Unreal `WITH_EDITORONLY_DATA` layout-skew hazard.
- The directional rule (`boyko_serialize` / shipping crates never depend on
  `boyko_reflect`) — the load-bearing invariant that keeps "Debug-only" honest.
- The compile-boundary hot-path proof ("the symbol does not exist") — stronger than
  the 14a/14b runtime-branch discipline.
