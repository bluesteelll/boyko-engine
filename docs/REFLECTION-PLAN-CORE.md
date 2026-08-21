# `boyko_reflect` — CORE plan: registry, data model, derive

> **This is a PLAN, not a discussion.** Its design input is
> [`docs/REFLECTION-ANALYSIS.md`](REFLECTION-ANALYSIS.md) (revision 2026-08-21, re-grounded
> against `feat/reflection`). Where the analysis debates, this document decides; where it
> decides, this document executes. A rung is a landable, gated commit.
>
> **Status:** PLAN — no code exists. Branch `feat/reflection`, worktree `D:/wt/reflect`.

## What this plan owns, and what it does not

| | Owner |
|---|---|
| ~~crate skeleton · Cargo feature wiring · workspace membership~~ — **corrected 2026-08-21: owned by GATES G0**, which lands all three packages, their manifests, their `members`/`default-members` rows and their census rows. This file owns what goes *inside* `boyko_reflect`, starting at C0's canary | [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md) |
| registry: `REFLECT`, `install_type_info`, `type_info_of` | **this file** |
| value model: `Scalar` / `ScalarKind` / `ValueKind` / `TypeKind` / `TypeInfo` / `FieldInfo` | **this file** |
| the `prim::` fn-ptr accessor library | **this file** |
| `#[component(reflect)]` — field walk, `offset_of!` baking, the `component_id()` install append | **this file** |
| every refusal the derive makes (generics, packed, bitset, `Opaque`, un-`repr`'d enum) | **this file** |
| the Nested / Opaque recursion contract and its allocation audit | **this file** |
| entity/component enumeration · `get_field`/`set_field` glue · `add_default`/`remove` · the public by-id structural seam on `EcsMaster` · the `StorageKind`×`ResidencyKind`×dynamic-tag runtime matrix · `BUG-MIGRATE-TB-1` in the enumeration glue | [`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md) |
| `Sink`/`Source` · the name-keyed wire · `stable_name` consumption at the wire · tuple-struct reorder caveat | [`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md) |
| CI legs (feature-on/off matrix) · the ship absence gate (`cargo tree` + symbol census + present control) · the Miri package allowlist · the hot-loop 0 % bench · the bevy-shaped `get_field` baseline | [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md) |

**Every rung below has a gate that a GATES-plan leg must actually run.** The dependency is
enumerated in §7 rather than assumed; a rung whose gate no CI leg executes is a rung this
campaign has already paid for twice (F17, F18).

---

## 0. Inherited, NOT re-litigated

Two findings arrive settled. They are restated here only so a reader of *this* file cannot
re-open them without going back to the analysis first.

1. **"Reflection only in a Debug build" is not a compiler property.** `cfg(debug_assertions)`
   cannot control whether a crate is in the dependency graph, and it is on for every plain
   `cargo build`. The mechanism that delivers the owner's actual intent — *on while developing,
   literally absent in the shipped game* — is an **optional crate behind a Cargo feature**, plus
   a CI symbol-absence gate on the ship build. (Analysis §0, §2.)
2. **The §6 scope fork is TAKEN: option (B)** — POD + `String` + nested + `#[repr(Int)]` enum in
   v1; collections (`Vec`/`Map`) deferred to v2. **Recorded as TAKEN-with-reason and
   REVERSIBLE**, not as pre-existing consensus: it was never ratified before 2026-08-21. The
   reason is the analysis's own — *a layer that can only read `f32`/`u64` is not an inspector,
   and the entire justification for reflection is tooling over arbitrary components*. Reversal
   is a scope edit (drop `Str` from `ValueKind`, delete two accessors), not a redesign.
3. **Engine-owned components may carry the opt-in, and the mechanism is decided before rung 0**
   (analysis **B.12**, owner sheet **B.13 #1**). §2's *"the consumer writes the opt-in"* assumed the
   consumer is the **game**; every dogfood target this campaign names is defined in a shared engine
   crate, so there is no consumer to write it. The plan proceeds on B.12's option (b): engine crates
   may declare a non-default `reflect` feature, contained by
   [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)'s **D3 C1–C6** rather than by the old
   leaf-only rule. **Consequence CORE must carry:** the real-engine dogfood lives in
   `crates/reflect_dogfood/`, not in `crates/reflect_fixture/` — the fixture must stay FFI-free
   because it is the package the Miri row names, and `boyko_render` reaches `boyko_rhi_vulkan`.
   Every gate below that says "real engine types" therefore names its package.

**Rung ordering across the four plans is stated once, in
[`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)'s G0 preamble, and cited here rather than
restated: `G0 → G1 → G2 → G3 → G4` precede `C0`.** That is a change from this file's first revision,
which opened with C0 creating the packages; see C0.

---

## 1. Verified in-tree facts

This document's own numbering. Each was read on `feat/reflection` at authoring time; a rung that
finds one false stops and escalates rather than working around it.

| # | Fact | Anchor |
|---|---|---|
| F1 | `MAX_COMPONENTS = 512`, `pub`, importable | `component_registry/mod.rs:61` |
| F2 | `LAYOUTS` and `HOOKS` are `[OnceLock<T>; MAX_COMPONENTS]`, *"a single acquire-load + branch — no `Mutex`, no `static mut`, no data race"* | `component_registry/mod.rs:206`, `:224` |
| F3 | `SERIALIZE` and `BIND_ACCESSORS` are the same shape and are **newer than the analysis snapshot** — the pattern is four instances deep, not two | `component_registry/serialize.rs:277` |
| F4 | `STORAGE_KIND` / `RESIDENCY_CLASS` are `[AtomicU8; MAX_COMPONENTS]` with `Relaxed`, deliberately, because *"the kind is a registration-time, write-once datum with **no payload published through it**"* | `component_registry/mod.rs:373`, `:501` |
| F5 | In-tree installer/getter convention is `(component_id: usize, …)`, because the derive calls them as `…::component_id().0` | `component_registry/serialize.rs:287`, `:299` |
| F6 | `install_bind_accessor` is **`pub`** specifically *"so the `#[derive(Bindable)]` expansion (which lives in downstream crates where `pub(crate)` is unreachable) can call it"*; write-once, *"a same-id re-install is a silent no-op (first writer wins)"* | `component_registry/serialize.rs:293-318` |
| F7 | The `component_id()` funnel is `static ID: OnceLock<ComponentId>` → `get_or_init` → `register_new::<Self>()` → **six** install slots (`storage`, `require`, `clone`, `relationship`, `residency`, `serialize`) + the const-gated `if Self::HAS_HOOKS` | `boyko_macros/src/component.rs:350-374` |
| F8 | Install slots come in two flavours: **const-gated** (`if Self::HAS_HOOKS`, for the derive-XOR-runtime-builder contract) and **ungated + self-gating** (`install_residency_class::<Self>` short-circuits on the default const) | `boyko_macros/src/component.rs:280-303` |
| F9 | `#[component(...)]` already parses a bare flag key (`no_bundle`) and a `storage = "bitset"|"dense"` key, with duplicate detection and a *"valid keys: …"* diagnostic | `boyko_macros/src/component.rs:643-760` |
| F10 | `StorageKind` is 3-way (`Table=0`, `Bitset=1`, `Dense=2`); `Bitset` has **no `ComponentPool`**, `Dense` has a global `DenseStore` and is **always `ResidencyKind::Cpu`** | `component_registry/mod.rs:323-339` |
| F11 | The `Bindable` derive's trampoline takes `let this: &T = unsafe { &*(p as *const T) }` off arena-rooted provenance, and it has passed Miri | `boyko_macros/src/bindable.rs:113`, `:119` |
| F12 | `core::mem::offset_of!` is load-bearing engine-wide on this toolchain, including as `const _: () = assert!(offset_of!(…) == N)` layout pins | `boyko_render/src/gpu_transform3d.rs:108-115` (317 sites tree-wide) |
| F13 | `boyko_serialize`'s manifest and lib header assert the directional rule at the source: *"never `boyko_reflect` (the codegen-not-reflection invariant)"* | `crates/boyko_serialize/Cargo.toml:6-10`, `src/lib.rs:6` |
| F14 | Package names are dashed, directories underscored (`boyko-serialize` in `crates/boyko_serialize`), and every member carries `[lints] workspace = true` | `crates/boyko_serialize/Cargo.toml` |
| F15 | `clippy.toml` bans `HashMap`/`HashSet`/`Mutex`/`RwLock`/`Rc`/`RefCell` at **deny** via `[workspace.lints.clippy] disallowed_types = "deny"`; `OnceLock` is **not** banned; exceptions carry `#[allow(clippy::disallowed_types)]` + a rationale | `clippy.toml`, root `Cargo.toml` |
| F16 | The root is **also a package**, so `default-members` names every member **plus `"."`** — *"there is no non-workspace-wide root build any more"* | root `Cargo.toml:1-40` |
| F17 | **`grep -c hwrt .github/workflows/ci.yml` = 0.** Every `#[cfg(feature = "hwrt")]` body in the tree is compiled by **no CI leg**. A feature-gated body is invisible to the default gate — measured, not feared | `.github/workflows/ci.yml` |
| F18 | CI's Miri step is a **hand-listed package allowlist** (`-p boyko-ecs -p boyko-utils -p boyko-threadpool -p boyko-serialize -p boyko-math -p boyko_sdf_math -p boyko_image`), required, not `continue-on-error`; `MIRIFLAGS=-Zmiri-tree-borrows` is workspace-wide | `.github/workflows/ci.yml:193-226`, `.cargo/config.toml` |
| F19 | Features unify **per package**, and the tree has recorded the consequence: *"a `#[cfg]`'d field on a struct `boyko_app` constructs appears or vanishes for that crate depending on a flag none of its own source names"* | `crates/boyko_rhi_vulkan/Cargo.toml:21-25` |
| F20 | A counting-global-allocator **delta** harness with baseline subtraction is the tree's established zero-allocation instrument | `crates/boyko_ui/tests/p4_bind_zero_alloc.rs:1-20` |
| F21 | 220 hand-written `impl Component for` sites exist; of the 40 files sampled, **every one is a bench, an integration test, or a `#[cfg(test)]` module** — none is production source | `crates/boyko_ecs/{benches,tests}/*`, `…/src/**/mod tests` |

---

## 2. Decisions

Numbered so a later reader argues with the reason, not from scratch. **Rejected alternatives are
recorded with each.**

### D1 — Optional crate behind the *consumer's* Cargo feature. No `debug_assertions`, no feature on `boyko_ecs`.

Inherited (§0.1). *Rejected:* `#[cfg(any(feature="reflect", debug_assertions))]` inside the
derive (non-compiling for a plain debug build — the derive emits `boyko_reflect::…` paths into a
crate that lacks the dep); a `reflect` marker feature on `boyko_ecs` (workspace feature
unification then flips it on for a ship binary — F16 makes that the *default*, not an opt-in).

### D2 — The reflect-enabling feature MUST be named `reflect`, in every consumer.

The gate lives in derive output as `#[cfg(feature = "reflect")]`, and a `cfg` in derive output is
evaluated in the **crate the derive expanded into** — which is the crate that *defines* the type, not
the crate that reads it. So the name is a cross-crate contract, not a local choice. Enforced by C8's
compile-both-ways PoC.

*Two consequences, both recorded because they are inherited silently:*

* every crate containing an `aether!` block inherits this requirement the moment Aether emits the
  opt-in (analysis B.5(a)) — that belongs in the Aether language docs, and it is named here so the
  Aether work knows it owes it;
* **every engine crate that opts a component in inherits it too** — `boyko_scene` for `Transform` /
  `Name` / `Visibility`, `boyko_render` for `GpuTransform3D`. That is not optional and it cannot be
  dodged by declining to declare the feature: root `Cargo.toml:25-26`'s
  `[workspace.lints.rust] unexpected_cfgs` check-cfg list **adds to** Cargo's per-manifest feature
  list, and every member opts in via `[lints] workspace = true`, so a `#[cfg(feature = "reflect")]`
  in a crate with no such feature **warns** and the existing `-D warnings` gate reds it. A missing
  feature is therefore a diagnostic rather than silence — which is the property
  [`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md)'s D16 relies on — and it is also why
  §0.3's decision had to be taken *before* rung 0 rather than discovered at C6.

### D3 — One derive, not two: an opt-in `#[component(reflect)]` key on the existing `Component` derive.

The analysis's §2 sketch (`#[cfg_attr(feature="reflect", derive(Reflect))]`) cannot hook the lazy
`component_id()` funnel, because that funnel is emitted by the `Component` derive and a separate
`Reflect` derive cannot inject into the same `impl Component` (A.5). `#[component(…)]` already
carries bare flag keys with duplicate detection and a good "valid keys" diagnostic (F9), so the
key costs one match arm. *Rejected:* a second `derive(Reflect)` (cannot reach the funnel);
`linkme`/`inventory` (the `--gc-sections` dead-strip / init-order hazard class, which the lazy
funnel sidesteps entirely).

### D4 — Registration is LAZY via the `component_id()` funnel, against a shipped in-tree precedent that chose explicit.

`#[derive(Bindable)]` faced this exact fork and took the **explicit** `register_bind_accessor()`
route — and its complete form (`register_bindable::<C>(app)`) has **zero callers anywhere**, its
partial form only five, all in tests, all registering the same fixture (analysis A.5 correction).
So "explicit works fine" has no evidence behind it in this tree; the opposite does.

The argument that decides it — and it is about **failure shape**, not ergonomics: a forgotten
`register_bindable::<C>` breaks *one authored binding*, a visible localized bug in a feature
someone deliberately wired. A forgotten reflection registration makes a component **silently
invisible in the inspector**, which is indistinguishable from *"this entity doesn't have that
component."* The inspector's entire job is to be trusted about what is there, so a silent
omission is a **correctness** failure of the tool. Reflection cannot tolerate it; binding can.

*Fallback, kept available as a mechanism escape hatch:* explicit `boyko_reflect::register::<T>()`
at startup. **If it is ever taken, the inspector must be able to distinguish "no `TypeInfo`
installed" from "component absent" and say so** — the same three-state row the ECS plan needs for
GPU-resident and dynamic-tag ids, so it is one shared row state, not four.

### D5 — `REFLECT` is `[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]`, mirroring `LAYOUTS`/`HOOKS`/`SERIALIZE`/`BIND_ACCESSORS` (F2, F3) — **never** the `AtomicU8` shape.

`STORAGE_KIND`'s `Relaxed` `AtomicU8` is right for a classification byte and **wrong** for a
`&'static TypeInfo` table, because that table *publishes a payload* and therefore needs the
release/acquire edge an `OnceLock` provides. The reason is stated at `STORAGE_KIND`'s own
declaration (F4) and this decision is its contrapositive. `MAX_COMPONENTS` is **IMPORTED, never
redeclared**.

### D6 — Installer signature is `install_type_info(component_id: usize, info: &'static TypeInfo)`.

Matches F5, because the derive calls it as `…component_id().0`. *Rejected:* a `ComponentId`
parameter — it would make `boyko_reflect` the only table in the tree with the other convention.

### D7 — No `IS_REFLECT` const on `Component`. **The `#[cfg]` is the gate; the derive's own knowledge is the other half.**

The analysis proposed *"an `install_type_info(…)` call gated by a new `const IS_REFLECT`"*
(§3.1). Working it against F7/F8 shows the const is a **second carrier of a value with no second
writer** — the "one value in two places" defect class:

* `HAS_HOOKS` exists because there is a genuine **derive-XOR-runtime-builder** contract: the slot
  may be filled by a runtime builder instead, so the const distinguishes them. **Reflection has
  no runtime `TypeInfo` builder**, so there is no XOR to express.
* The derive already knows, at expansion time, whether `#[component(reflect)]` was written. It
  can simply not emit the call — which is strictly stronger than emitting a call the optimizer
  folds away, and it is what the feature-off path needs anyway (zero tokens naming
  `boyko_reflect`).
* F21: the only writer of `Component` impls in production is the derive. A trait const would
  serve a hand-written production `impl Component`, and none exists.

*What would resurrect it:* a runtime `TypeInfo` builder (the hooks-builder analogue). If that is
ever built, the XOR contract returns and so does the const. Recorded so it is a re-decision, not a
rediscovery. *The question "is `T` reflectable?" has exactly one carrier:*
`type_info_of(id).is_some()`.

### D8 — `TypeInfo` carries **no** `stable_name`.

It already exists in `boyko_ecs` as `SerializeInfo.stable_name` + `stable_name_hash` +
`STABLE_NAME_INDEX`, documented cold, with `format_version` and `layout_fingerprint` beside it
(analysis B.2). A second declaration would be a drift pair with a *serialization wire format* on
the other end. The save key is read from `get_serialize_info(id)`. **The Wave-4 "once-per-type
name↔id resolution" is a consumption task, not a construction task** — see
[`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md).

### D9 — `FieldInfo` carries **no** `serialize` slot and **no** `debug_fmt` slot in CORE.

Both appear in analysis §3.2. Neither has a reader inside CORE's scope, and this campaign has
recorded **five** instances of the *dead datum* class — a field built, gated, and read by nothing
(`site.decode`, `LogSite.fields`, twelve unbuilt benches, a `sample_shift` that lay in the control
for two rungs, a silently-refusing `intern_site`). The rule this plan adopts: **a datum lands in
the same commit as its first reader.**

* `serialize` lands at BOUNDARY's first rung, together with the `Sink` trait and the wire — one
  commit that adds the field, the `prim::` serializers and the reader.
* `debug_fmt` lands with the debug-dump consumer, which is in none of the four plans. **DEFERRED,
  and named as deferred** (§6) rather than shipped inert.

*Cost, stated:* adding a field to `FieldInfo` later touches every derive expansion. That is a
same-commit mechanical change with one constructor (the derive), and it is cheaper than a slot
nobody can prove works.

### D10 — Accessors are `Option<fn>` **per kind**. There is no poison stub, and `get_field` returns `None` for any non-`Prim` field.

Analysis A.2's completeness-C2 / Mi2 fixes. A poison stub is a function that exists to be never
called; the `Option` makes "this kind has no such accessor" a value the type system carries.
*Rejected:* a stub that panics (a release-editor build then panics where it should refuse); a stub
that returns a zero `Scalar` (silent garbage — Mi2 exactly).

### D11 — The kind check on every setter is a **release** `-> bool`, never a `debug_assert!`.

The legitimate `--release --features reflect` editor build compiles `debug_assert!` out, and that
is precisely where an editor passes a stale `(ComponentId, field)` triple after a hot-reload
(analysis §5 / M5-W4). The `debug_assert_eq!` on `TypeId` stays as an *extra* dev guard; the
`bool` is load-bearing. **Its red must be shown in a release-profile test**, not a debug one
(C4).

### D12 — `ValueKind::Array` is in v1.

`[T; N]` of a `Prim` is offset + stride + count, all `const`, **zero alloc, no drop, no TB
exposure** — strictly easier than the `String` support the analysis spent its headline CRITICAL
on, and unlike `String` it has consumers: `GpuTransform3D`/`TrsPacked` (`[f32;4]`×3),
`csm_config` `[[f32;4];4]`, `ddgi_config` `[f32;3]`, `frustum` `[f32;4]`, and the engine's own
inline-string idiom `UiName { bytes: [u8; CAP], len: u8 }`. Without it the flagship dense
component is a **hard error**, not a partial view (analysis B.8). *Rejected:* leaving arrays in
`Opaque` (makes the design refuse the one component it is fully able to read).

### D13 — Build order inside scope (B): **arrays → nested → enum → `String` LAST.**

Ranked by in-tree consumer count (analysis §6.1(d)): nested structs (many), fieldless
`#[repr(Int)]` enums (**11**), arrays (pervasive), `String` (**zero** — no
`#[derive(Component)]` struct anywhere in `crates/*/src` has a `String`, `Box<str>` or `&str`
field). Deferring `String` to last also defers the whole `drop_in_place` + `ptr::write` TB
surface with it. **The decision (B) does not change; only what lands first.** Owner-visible: this
changes what the derive ships first (analysis B.11 #4).

### D14 — `#[reflect(skip)]` emits a `FieldInfo` with `kind: Opaque` and **no accessors**. It does not omit the field.

Omitting a field would make `fields.len()` disagree with the struct's field count and make the
by-index API's indices depend on which fields were skipped — the drift class again. **An
inspector that shows nothing for a field is honest; one that shows a shorter list is lying.**
This is also what gives `ValueKind::Opaque` a reader in v1 (see D15).

### D15 — An `Opaque` field with no `#[reflect(skip)]` is a **hard derive error, spanned at the field**.

Analysis A.6 / FIX O2: the wire format is shared with the *shipped* `boyko_serialize`, so silent
omission is unacceptable. Consequence made explicit here, because the analysis notes its blast
radius was never weighed: `SoftBody` (fourteen `Vec` columns) becomes a **compile error** if
anyone derives it — which is the right v1 outcome **provided the refusal is documented, spanned
and opt-out-able**, not a surprise at first `#[component(reflect)]`. So: `ValueKind::Opaque` is in
the taxonomy but **unreachable from the derive except via `#[reflect(skip)]`** (D14). That
asymmetry is deliberate and is stated in the type's doc comment.

### D16 — Two tables, not one: `BIND_ACCESSORS` is untouched; `REFLECT` is separate. **OWNER decision, and the plan proceeds on Horn 2 pending it.**

Analysis B.1's fork. Horn 2 changes nothing that ships and is the lower-risk default; Horn 1 pays
off only if the editor and the UI binding system are ever meant to share a field vocabulary, and
it lets a **shipping** crate (`boyko_ui`) consume reflection metadata, which strains the §1
directional rule. **This is a VALUES/SCOPE call about shipping API surface and goes to the owner
with both costs stated** (analysis B.11 #1).

**What Horn 2 owes, and this plan delivers at C8:** a feature-on-only test asserting that, for
every type carrying both `#[derive(Bindable)]` and `#[component(reflect)]`, `field_id(name)` and
the reflect field index **agree**. Cheap, and it is what makes the duplication honest rather than
merely tolerated. *If the owner takes Horn 1 instead*, C3/C8 change and the tuple-struct conflict
(A.6 accommodates them, `Bindable` rejects them) must be resolved rather than averaged — that
resolution is then a new rung, not an edit.

### D17 — `boyko_macros` emits tokens naming `boyko_reflect` and **does not depend on it**.

The directional rule is about crate deps, not emitted tokens — and this is already house doctrine
stated verbatim in a shipping manifest: *"this crate does NOT depend on `boyko-ecs`: every
`boyko_ecs::…`/`boyko_macros::…` occurrence in the expander is an emitted TOKEN resolved in the
downstream crate"* (`crates/aether_lang/Cargo.toml:6-11`). A.5's jurisdictional point was
independently reinvented by the tree; this decision cites it rather than re-arguing it.

### D18 — `boyko_reflect` depends on `boyko_ecs` and `std` only. No third-party crate. No `Mutex`/`HashMap`.

F15's ban applies to this crate like every other (`[lints] workspace = true`). Nothing in CORE
needs a map: the registry is `ComponentId`-indexed, name→field is a linear scan of the few
fields, and name→id already exists as `STABLE_NAME_INDEX` in `boyko_ecs` (D8).

### D19 — `[T; N]` is supported for `T: Prim` only; arrays of arrays are v2.

Stated in full at **rung C5**, where it governs, rather than here — it is the one decision whose
whole content is the shape of a single rung's arm. Its named exclusion is
`csm_config.rs:392`'s `view_proj: [[f32;4];4]`.

### D20 — `default_in_place` is `Option<unsafe fn(*mut u8)>` like every other accessor, and the `T: Default` requirement is a NAMED, spanned, census-visible refusal — not an `E0277` inside generated code.

The first revision made `default_in_place` the **one** non-`Option` accessor on `TypeInfo` while D10
made every other one `Option<fn>`, and C7 baked it from `Default`. Three defects follow from that
pair, and the third is the one that makes it a plan defect rather than a taste question:

1. **A silent `T: Default` requirement.** `#[component(reflect)]` on a type with no `Default` impl
   would fail — and fail as an `E0277` pointing into an expansion the user never wrote, which is the
   diagnostic class C9's whole "spanned at the offending token" discipline exists to refuse. It bites
   the first *arbitrary user component*, not the first test: every dogfood target happens to impl
   `Default` (`Transform` at `transform.rs:111`; `Visibility` derives it), so nothing in this
   campaign's own fixtures would have found it.
2. **It is not a row in C9's refusal matrix**, so **C9 gate 2's anti-rot census cannot see it** — that
   census counts `.stderr` fixtures against a `const REFUSALS` the derive iterates, and a rule that is
   not in `REFUSALS` is structurally invisible to it. A refusal the census cannot count is a refusal
   nothing keeps honest.
3. **A proc macro cannot detect a missing trait impl**, so a `compile_error!` spanned at the type is
   *not available* — which is precisely why this needs a decision rather than "add a C9 row".

**The decision, in three parts:**

* **`default_in_place: Option<unsafe fn(*mut u8)>`** — uniform with D10. `None` is a real state and
  it has a real consumer: [`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md)'s `add_default` answers
  `Err(Refusal::NoDefault)` instead of the type being un-inspectable. **An inspector that shows a
  type's fields and refuses to synthesize one is strictly better than a derive that refuses the type**
  — the same asymmetry D14 uses for `#[reflect(skip)]`.
* **The bound is asserted with a named diagnostic, and the diagnostic is house style.** The derive
  emits, spanned at the type name:

  ```rust
  const _: fn() = || {
      fn __assert_reflect_default<T: ::boyko_reflect::ReflectDefault>() {}
      __assert_reflect_default::<MyType>();
  };
  ```

  where `boyko_reflect` declares

  ```rust
  #[diagnostic::on_unimplemented(
      message = "`#[component(reflect)]` bakes `default_in_place` from `Default`, and `{Self}` does not implement it",
      label = "add `#[derive(Default)]`, write an impl, or opt out with `#[reflect(no_default)]`",
  )]
  pub trait ReflectDefault: Default {}
  impl<T: Default> ReflectDefault for T {}
  ```

  `#[diagnostic::on_unimplemented]` is stable since 1.78 and is **already load-bearing in this tree**
  — `crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs:67` and
  `.../query/filter.rs:2507`, the second with a `compile_fail` fixture
  (`crates/boyko_ecs/tests/compile_fail_chunk/changed_filter_rejected.rs:11`) that pins the message.
  So this is the tree's existing answer to "a trait bound needs a human-readable refusal", not a new
  mechanism.
* **`#[reflect(no_default)]` emits `None`** and is the documented way out, named in the label above.

**This makes the rule a C9 row with a fixture** (`missing_default_rejected`, plus the positive
`no_default_accepted`), so gate 2's census counts it like every other refusal. *Rejected:* keeping
`default_in_place` non-optional and adding the rule to C9 anyway (there is no spanned
`compile_error!` to bless a `.stderr` against — the error necessarily comes from trait resolution);
requiring an explicit `#[reflect(default)]` opt-in for the `Some` arm (it inverts the common case —
most components derive `Default` — and buys nothing the `on_unimplemented` label does not).

### D21 — Termination and addressing-validity are RUNTIME STRUCTURAL checks that enumerate NOTHING. A syntactic refusal list is a DIAGNOSTIC, and the two are not a drift pair.

*Architect's ruling on the C6 escalation, 2026-08-21.* Amends **§3.1**, **§5**, **C6** and **C9
gate 2**.

**The decision.** `validate` gains exactly two rules, and they are the proof:

* **Check A — inline containment**, per `Nested` edge, O(1): `offset + child.size <= parent.size`,
  `offset % child.align == 0`, `child.align <= parent.align` ⇒ `Violation::NestedNotInline`.
* **Check B — acyclicity**, per validated root: a DFS over `Nested` edges with a **fixed on-stack**
  path array and a linear scan ⇒ `Violation::NestedCycle`.

Both land at **C6**, both at validation time, neither on the descend path: no allocation, no
`HashMap` (D18), zero per-frame and per-level cost.

**Why the list cannot be the proof.**

1. **The defect is misnamed.** "Infinite descent" is the *second* step. Give a container field a
   `Nested` descriptor and the first descend reads the container's own words as the child's inline
   fields; when the child descriptor does not fit the field's extent that read runs past the end of
   the value. A **memory-safety** property is enforced next to its `unsafe`, in the crate that owns
   the pointer arithmetic — not in a proc macro four rungs downstream.
2. **A runtime refusal list is not even expressible.** `Vec<T>` is generic over unboundedly many
   `TypeId`s, so "refuse these types at registration" is not a finite check. A *syntactic* list is
   finite but cannot name a user-defined indirection (`struct MyBox<T>(*mut T)`).
3. **Sizedness proves acyclicity only for derive-generated statics.** Inline containment means the
   parent's size includes the child's and Rust forbids a `Sized` type of infinite size — so a
   derive-generated containment graph is acyclic **by the type system**. Hand-written statics are
   what C3's gates walk and what C6's red constructs, and a `size(A) == size(B)` newtype chain
   passes Check A while closing a cycle. **Both checks are required**; C6 gate 3(ii) measures that
   Check A stays green on a cycle rather than conceding it in prose.

**Why this is not a second list.** Checks A and B enumerate **no type name**. There is nothing in
them for C9's `REFUSALS` to agree or disagree with, so the drift surface the escalation correctly
feared does not exist. C9's list keeps its job — turning a registration-time `Problem` into a
**spanned compile error at the user's own token** for the five standard kinds — and that job is
*diagnostics*, so a missing kind is a worse message, not an unsound descend (C9 gate 2, re-scoped).

**Why this is recorded as a decision rather than fixed silently.** The first revision put the proof
**in the list**, said so in **three** places (§3.1's *"C9's refusal test IS the acyclicity proof"*,
C6 gate 3, §5's *"it is what makes C6's acyclicity proof true"*), and then scheduled that list
**four rungs after its consumer** — §5's own tree is `C6 → C7 → C8 → C9`. Three agreeing statements
of a false claim are not three checks; they are one claim with three readers. The escalation that
caught it was right to refuse the "second list" resolution and right to escalate rather than
amend. This entry exists so the next author meets the reasoning, not just the two `Violation` arms.

---

## 3. The value taxonomy (scope B + arrays)

```rust
#[repr(u8)]
pub enum ScalarKind { Bool, U8, U16, U32, U64, I8, I16, I32, I64, F32, F64, EntityId }

#[repr(C)]
pub struct Scalar { pub kind: ScalarKind, pub bits: u64 }   // POD, Copy, no heap

#[repr(u8)]
pub enum ValueKind {
    Prim(ScalarKind),
    Array,     // [T; N] of a Prim — D12
    Nested,    // a field whose type is itself reflectable
    Enum,      // fieldless #[repr(Int)]
    Str,       // built LAST — D13
    Opaque,    // reachable only via #[reflect(skip)] — D14/D15
}

#[repr(u8)]
pub enum TypeKind { Struct, TupleStruct, Enum, Opaque }
```

`Scalar`'s tag **doubles as** the `ValueKind` guard on `set` (D11), so the tag is not extra cost.
`EnumInfo { repr: EnumRepr, variants: &'static [VariantInfo] }` with `VariantInfo { name,
discr_bits: u64 }` storing the discriminant **already narrowed to the repr width** (analysis FIX
C2/O1 — not a lossy `i128 as u64` at the call site); reads sign-extend per `EnumRepr` for `Ix`.

**Top-level enums are a real case, not only a field case.** `Visibility`
(`boyko_scene/src/render_caps.rs:226`) is a fieldless `#[repr(u8)]` enum that **is** a Component,
with discriminants pinned *"so the byte is stable across serialization"* — so `fields_of` returns
`&'static []` and the whole value is reached through `TypeKind::Enum` plus type-level discriminant
accessors. C10 handles both; A.2's field-centric `FieldValue::Enum` alone does not.

### 3.1 The Nested recursion contract

Recursion is **pointer arithmetic over a `&'static` graph**. No value tree is ever materialized —
that is the entire `Box<dyn>`-per-field allocation class this design refuses.

* `FieldInfo.nested: Option<&'static TypeInfo>` points at the inner type's own `TYPE_INFO` static.
* **Descend = one `add` + one pointer copy per level.** `cursor.ptr.add(inner_off)`; the
  `&'static TypeInfo` is reused, never flattened into a path table.
* **Enumerate at depth = `info.fields`**, already `&'static`.
* **Derive-time recursion is depth 1.** The derive emits a *pointer* to the inner type's static;
  it does **not** walk the inner type's fields — the inner type's own derive did that. So there is
  no proc-macro recursion, no expansion blow-up, and no ordering requirement between the two
  types' expansions.
* ~~**Acyclicity is structural, not checked.** A `Sized` Rust value cannot contain itself by value,
  and *every* indirection (`Box`, `Vec`, `&T`, `Option<Box<…>>`) is a v2 kind that C9 refuses. So
  **no runtime cycle guard and no depth counter exist, and none is needed.** The property that
  keeps this true is C9's refusal list — **C9's refusal test IS the acyclicity proof**, and if v2
  ever admits an indirection, the guard becomes required in the same commit.~~
  → **Acyclicity and addressing-validity are CHECKED, by two rules in `validate`, and both land at
  C6** (architect's C6 ruling, 2026-08-21; decision **D21**). The struck sentence is false, and it
  is false in the direction that reads as safe.

  The *inline-containment* half of the old argument survives: a parent's `size` includes its
  child's, and Rust forbids a `Sized` type of infinite size, so a **derive-generated** containment
  graph is acyclic **by the type system** and every cycle must pass through an indirection. What
  that argument does not reach is the descriptor set this crate actually admits. `TYPE_INFO`
  statics are **hand-written** at C3 and C6 and in every fixture, and a hand-baked
  `size(A) == size(B)` newtype chain closes a cycle with inline containment intact. A proof that
  holds for the case the author imagined and not the case the code admits is this campaign's
  recurring shape, so both properties are enforced next to the `unsafe` that depends on them:

  * **Check A — inline containment.** Per `Nested` edge, O(1), no allocation:
    `offset + child.size <= parent.size`, `offset % child.align == 0`, `child.align <= parent.align`
    ⇒ `Violation::NestedNotInline`. This is the **addressing-validity** proof, and it is a
    memory-safety property rather than a tidiness one: a child descriptor that does not fit the
    field's extent makes the **very first** descend address bytes past the end of the value, before
    any recursion happens.
  * **Check B — acyclicity.** Per validated root, a DFS over `Nested` edges with a **fixed
    on-stack** path array and a linear scan ⇒ `Violation::NestedCycle`. Termination of the descend
    is this rule's corollary, not a separate device.

  **Both are required, and neither implies the other.** For derive-generated statics Check A
  implies acyclicity through sizedness; for hand-written statics it does not, which is exactly the
  fixture C6 gate 3(ii) builds. Both run at **validation time**, not descend time — the descend
  stays one `add` plus one pointer copy per level, and §3.3's *nested descend* row still claims 0.

  **C9's refusal list is the derive-side EARLY DIAGNOSTIC, not the proof — and it cannot be the
  proof.** A syntactic blacklist cannot name a user-defined indirection (`struct MyBox<T>(*mut T)`
  is not on any list of five kinds), and no enumerable *runtime* list exists either: `Vec<T>` is
  generic over unboundedly many `TypeId`s, so "refuse these types at registration" is not a
  finite check. What C9 buys is that a user who writes `Vec<u8>` meets a **spanned compile error at
  the field** instead of a registration-time `Problem` — a better diagnostic for the five standard
  kinds, and nothing more. Checks A and B enumerate **no type name**, so there is nothing for C9's
  list to drift against.

### 3.2 The Opaque contract

`Opaque` = not a `Prim`, not an array-of-`Prim`, not a reflectable nested type, not a fieldless
`#[repr(Int)]` enum. Disposition: **hard derive error** (D15) unless the field carries
`#[reflect(skip)]`, in which case a `FieldInfo { kind: Opaque, get: None, set: None, nested: None,
enum_info: None }` is baked (D14). **An `Opaque` field has no accessor to call, therefore no code
path, therefore no allocation** — that is the answer to "the `Opaque` path is where any
`Box`/`Vec` allocation pressure will live": in v1 there is no `Opaque` path at all, only an
`Opaque` *label*. If v2 gives `Opaque` a payload-producing accessor, the audit below must be
re-run against it, and that is written into the taxonomy's doc comment so the next author sees it
before the allocator does.

### 3.3 Allocation audit — SPECIFIED, and how each number is MEASURED

The analysis says the compound paths are where allocation pressure lives and that hand-waving them
is how it goes wrong. So the audit is not a table in a document; it is an **enforced test**, and
each row names the rung that adds its arm.

**Instrument (~~rung C5~~ → rung C4, corrected 2026-08-21):** a counting global allocator with
**baseline subtraction**, modelled on `crates/boyko_ui/tests/p4_bind_zero_alloc.rs` (F20) — the
delta between the measured path and an identically-shaped no-op, so the harness's own machinery
cancels. ~~`#[allow(clippy::disallowed_types)]` + rationale on the file, exactly as the precedent
does.~~

> **Corrected 2026-08-21, at C4's execution, and it closes a gate that could not run.** *"Instrument
> (rung C5)"* contradicted **C4 gate 5** — *"Alloc-delta harness arm: `Prim` get/set = 0 (§3.3)"* —
> which sits one rung **earlier**; the table below compounded it by assigning the `Prim get / set`
> row to C5 as well, so one arm had two owners and its gate had none. A gate whose instrument is
> specified to land after it is this campaign's most-repeated defect (twelve benches in a gate
> table, none of which existed). Resolved in the direction that keeps every gate runnable: the
> instrument lands at **C4** with the `Prim` arm; **C5 adds the `enumerate` and `array read` arms on
> top** (its Lands is amended to match). Two further corrections, both MEASURED at C4 rather than
> reasoned:
>
> * **The counter is THREAD-LOCAL, not the precedent's process-global `AtomicUsize` + `Mutex`.** Built
>   the precedent's way first, the harness reported `get Bool: baseline=1 measured=0 **delta=-1**` —
>   libtest's own machinery allocates on other threads while an armed window is open, and the mutex
>   can serialize this file's windows only against each other. A negative delta is the diagnostic
>   that settles it: a measured path cannot allocate less than nothing. A `const`-initialized,
>   `Drop`-free thread-local `Cell` is a plain TLS read (no lazy init, no destructor, therefore no
>   allocation inside the allocator), the deltas become exactly 0, and **no `Mutex` and no
>   `#[allow(clippy::disallowed_types)]` are needed** — hence the strike-through above.
> * **The harness binary is `#![cfg(not(miri))]`.** A `#[global_allocator]` forwarding to `System` is
>   not transparent under Miri + Tree Borrows on `x86_64-pc-windows-gnu`: `HeapFree` through the
>   pointer `System.alloc` returned is a foreign write to libtest's protected tags. **Measured, not
>   assumed:** with a filter selecting *nothing*, the binary prints `running 0 tests` and still
>   aborts inside `mpmc::Sender::drop`. Nothing is lost — C4 gate 4's subject is the `prim::` module,
>   and `c4_prim.rs` covers all twenty-four accessors under Miri. (`boyko_ui`'s F20 precedent is not
>   on the Miri allowlist, which is why no one has met this before.)

| Path | Claim | Rung | How the number is obtained |
|---|---|---|---|
| enumerate (`info.fields`) | **0** | C5 | delta harness; the slice is `&'static`. **Measured 2026-08-21: `enumerate (1 field(s)): baseline=0 measured=0 delta=0`** over 1000 walks |
| `Prim` get / set | **0** | **C4** (was C5 — see the correction above) | delta harness, per `ScalarKind`; **measured 2026-08-21: delta 0 for all 12 kinds, get, set and the refused-set path, over 1000 calls each** |
| array read (offset + stride + count) | **0** | C5 | delta harness; all three are `const`. **Measured 2026-08-21: `array read (len 4): baseline=0 measured=0 delta=0`** over 4000 calls; its red seen at 4000/4000 |
| **nested descend, depth ≥ 2** | **0 per level** | C6 | delta harness; **a depth-1 harness proves nothing about recursion and is not accepted**. **Measured 2026-08-21: `nested descend (depth 2): baseline=0 measured=0 delta=0`** over 1000 walks; its red seen at **2000/1000** — two per walk, i.e. one **per level**. ~~over `Transform → Vec3 → f32`~~ → over a local depth-2 nest: the harness is a `boyko-reflect` test binary (one `#[global_allocator]` per binary, C5's reasoning), and that crate cannot reach `boyko_scene`. The engine-types descend is C6 **gate 1's dogfood half**, in `reflect_dogfood`, where it belongs |
| enum discr read / write | **0** | C10 | delta harness |
| `Opaque` field (skipped) | **0** | C9 | no accessor exists to call — asserted as `get.is_none() && set.is_none()` |
| `default_in_place` | **0 bespoke** | C7 | delta harness + a **drop-count** test (A.8) over `{ pod, Nested{pod}, [f32;4] }` |
| `set_str` | **exactly 1 alloc + 1 free** | C11 | the harness asserts the **count**, not the adjective "cold" |
| the `REFLECT` static's own footprint | reported | C2 | `size_of::<[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]>()` printed by the rung and pinned by a `const _: () = assert!(…)`; it is a static in a dev-only crate, so the number is recorded, not budgeted |
| `size_of::<Scalar>()` / `align_of` | reported, then pinned | C1 | measured at the rung and pinned at the measured value; **the plan does not assert 16 in advance** |

**The one claim this audit cannot make yet:** nothing above exercises a `Formatter`/`Display`
path, because `debug_fmt` is deferred (D9). When it lands, it brings its own arm — a `Display`
impl that allocates would otherwise walk straight through a green audit.

---

## 4. Rung ladder

**Unconditional gate on every rung** (repo standard): `cargo clippy --workspace --all-targets -- -D
warnings`; `cargo test --workspace --all-targets --no-fail-fast`; `// SAFETY:` on every `unsafe`;
Miri where new `unsafe` lands; author-only commit + push. In this disk-constrained worktree the
developer runs `-p <crate>` legs while iterating and the `--workspace` form once before the
commit — **`--workspace` and `--no-fail-fast` are both load-bearing and neither may be dropped for
speed** (CLAUDE.md records the measurements that put them there).

**Toolchain, every session:** `export PATH="$HOME/.cargo/bin:$PATH"
RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu` — the env var **alone is insufficient**; a
chocolatey rustc 1.95.0 shadows rustup's 1.97.1 and ignores it.

---

### C0 — The red canary: prove the leg GATES installed can actually go red

> **This rung was rewritten, 2026-08-21 (second pass), and the rewrite is an ordering fix.** As first
> written it *created* `crates/boyko_reflect/` and `crates/reflect_fixture/` — and so did
> [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)'s **G0**, with different contents, with no
> arbitration between them, and with G0 additionally owning two census rows and the `default-members`
> entry this rung also claimed. Worse, the completion condition was **circular**: C0's gate 3 required
> a feature-ON CI leg, that leg is G4, G4 sits behind G0–G3, and G0–G3 sit behind the same two
> package directories. The first commit of the campaign was specified twice and landable never.
>
> **G0 owns package creation** (three packages now — G0's D15). **C0 consumes it** and adds the one
> thing G0 cannot: proof that the leg G4 installed *fails when it should*. The ownership row at the
> top of this file — *"crate skeleton · Cargo feature wiring · workspace membership | this file"* —
> is corrected to **GATES**; what this file owns from here on is what goes *inside* `boyko_reflect`.

**Prerequisite, not a gate:** `G0 → G1 → G2 → G3 → G4` have landed. C0 is the first CORE rung and the
sixth campaign rung.

**Lands.**

* In `crates/boyko_reflect/src/lib.rs` (created by G0, hollow): the crate-level doc stating the
  directional rule (`boyko_serialize` and every shipping crate must not depend on this crate — F13)
  and the D1 gating mechanism. **No `[features]` table is added, now or ever** — GATES D4, and the
  reason it matters is that four documents once specified a Miri row that could not run because they
  assumed one existed.
* In `crates/reflect_fixture/`: the **red canary** —
  `#[cfg(feature = "reflect")] compile_error!("red canary: the feature-ON leg is live");` — landed,
  observed, and reverted in the same rung.
* One manifest correction G0 inherits from this file's first revision:
  `[dependencies] boyko-ecs = { path = ".." }` is **wrong** — from `crates/boyko_reflect/` that
  resolves to `crates/`, which is not a package. It is `{ path = "../boyko_ecs" }` (F14: dashed
  package names, underscored directories).

**The ship target is NOT named here.** [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)'s **D2**
is its single owner, and this file's first revision was wrong about the list: it named `boyko_app`,
which is `[lib]`-only (`crates/boyko_app/Cargo.toml:13`) and has no artifact to census. The ship
targets are `boyko_demo` (`crates/boyko_demo/Cargo.toml:7`, the only game-shaped `[[bin]]` member) and
the root `boyko-engine` package.

**Gate.**
1. `cargo build -p reflect-fixture` (feature off) and `cargo build -p reflect-fixture --features
   reflect` both succeed. **A proc-macro/Cargo mechanism is settled only by a compile** — this is
   the A.5 PoC, and it runs here even though G0 already checked it, because *this* rung is the one
   that will mutate it.
2. `cargo tree -p reflect-fixture` shows `boyko-reflect` **absent**; with `--features reflect`,
   present. (The `cargo tree` half is the load-bearing half — analysis B.6.)
3. ~~A CI leg exists that compiles the crate with the feature ON.~~ → **GATES G4 has landed**, and
   this rung's RED is the evidence that it is not decorative. The dependency is now a *prerequisite*
   satisfied before the rung starts, not a condition the rung cannot discharge by itself.

**RED MUTATION.** Land the canary. The **default** gate (`cargo clippy --workspace --all-targets`)
must stay **green** — proving F17's hazard is live here too — and the `reflect-on` leg must go
**red**. If both stay green, the leg does not exist and C0 is not done. *This rung's whole purpose is
that F17 (zero `hwrt` legs in CI, measured) does not repeat.*

*Second RED, and it is new because §0.3 made engine crates eligible:* land the same canary inside
~~`crates/boyko_scene/src/transform.rs` under `#[cfg(feature = "reflect")]` **without** adding a
`reflect` feature to `boyko_scene`'s manifest~~ → **`crates/boyko_threadpool/src/lib.rs` — a crate
that declares no `reflect` feature and never will (re-specified 2026-08-21, third pass; the defect
and the choice are recorded below)**. The expected red is **not** the canary — it is
`unexpected_cfgs` under `-D warnings`, naming `feature = "reflect"` (D2). Run it: it is the compile
that settles whether a missing feature is a diagnostic or a silence, and
[`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md)'s D16 and B6 gate 6 both rest on the
answer. In this disk-constrained worktree the scoped form
`cargo clippy -p boyko-threadpool --all-targets -- -D warnings` carries the identical lint
configuration (`[lints] workspace = true`), per §4's preamble.

> **Re-specified 2026-08-21 (third pass) — closing Defect B of the honest C0 stop.** As first
> written, this RED could not fire: G0's landed set already declares `reflect` on `boyko_scene` —
> forced by G0's own gate 2 (`cargo check -p reflect-dogfood --all-targets --features reflect`
> cannot resolve without it) — and [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)'s G0
> Lands records the consequence where it is caused: *"`boyko_scene` declares `reflect` from G0
> onward, so a `#[cfg(feature = "reflect")]` canary in `boyko_scene` is a KNOWN cfg — the
> `unexpected_cfgs` red that mutation expects cannot fire unless the mutation also removes this
> feature. The re-specification belongs to CORE."* The cfg is known, no warning fires, the canary
> strips silently, and the "red" reads green — the F17 class, inside the rung that exists to
> refuse it.
>
> Of the stop's two options, this re-specification takes the second — **target a crate that
> declares no `reflect` feature** — and the crate is **`boyko_threadpool`**, because its NEVER is
> architectural rather than predicted: it sits *below* the kernel (`boyko_ecs` depends on it), so
> it cannot name a component, and a crate that can never define a component can never carry the
> opt-in this feature exists for. (`boyko_math` was considered and REJECTED: `Vec3` is C6's
> nested-dogfood inner type, so that crate plausibly bakes a `TypeInfo` behind `reflect` someday,
> and this red would rot back into the same defect the day it does.) The mutation also lands where
> the *real* residual hazard now lives: G0 gave `boyko_scene` and `boyko_render` their
> declarations, so the next crate to write `#[cfg(feature = "reflect")]` without the manifest half
> is by construction a crate that has not declared it.
>
> The stop's first option — the mutation ALSO removes `boyko_scene`'s declaration, restoring it
> after — is REJECTED on a measurement, not on taste (run 2026-08-21, this worktree): with the
> `reflect` feature and its optional `boyko-reflect` edge stripped from
> `crates/boyko_scene/Cargo.toml`, **every selection in the workspace dies at version
> resolution** — `cargo tree -p boyko-threadpool`, a selection that never reaches the dogfood,
> exits 101 with *"package `reflect-dogfood` depends on `boyko-scene` with feature `reflect` but
> `boyko-scene` does not have that feature"* — because `reflect-dogfood`'s leaf umbrella forwards
> `boyko-scene/reflect` unconditionally, and a forward to a feature no manifest declares is a hard
> resolver error (G0's Lands, measured again here) that fires **before and instead of** the
> `unexpected_cfgs` diagnostic this RED exists to observe. An expected red that cannot be the
> observed red is Defect B with one more moving part.

> **Executed 2026-08-21 (worktree `D:/wt/reflect`, toolchain `stable-x86_64-pc-windows-gnu`).**
> *Lands disposition:* bullets 1 and 3 were found already satisfied by G0's landed set —
> verified against the tree, not assumed (`crates/boyko_reflect/src/lib.rs` carries the
> directional-rule and D1 sections; both `boyko_reflect` and `reflect_fixture` manifests spell
> `path = "../boyko_ecs"`; `boyko_reflect` has no `[features]` table). The rung's durable delta
> is therefore this file's amendment plus the observed REDs below. *Prerequisite:* verified by
> listing — `tests/reflect_manifest_census.rs`, `tests/reflect_ship_closure.rs`,
> `tests/reflect_ci_coverage.rs`, `crates/reflect_fixture/tests/reflect_absence_census.rs`,
> `crates/reflect_fixture/tests/reflect_leg_nonvacuity.rs`, and ci.yml's `reflect-on` /
> `reflect-census` / `reflect-dogfood` jobs plus both Miri rows all exist. *Gate 1:*
> `cargo build -p reflect-fixture` exit 0, `--features reflect` exit 0. *Gate 2:*
> `cargo tree -p reflect-fixture` — `boyko-reflect` **0 hits**; with `--features reflect` —
> present; both exit 0. *Primary RED:* canary in `src/bin/reflect_on.rs`; the G4 leg's debug
> command run locally with the job's own `BOYKO_REFLECT_LEG=reflect-on` → **exit 101**,
> *"error: red canary: the feature-ON leg is live"*, failing all four compilations of the two
> `[[bin]]`s sharing the source; with the canary still in place, the default gate's scoped form
> (`cargo clippy -p reflect-fixture -p boyko-reflect --all-targets -- -D warnings`, feature off)
> → **exit 0, green** — F17's hazard shown live, and only the leg catches it. *Second RED (as
> re-specified):* canary appended to `crates/boyko_threadpool/src/lib.rs`;
> `cargo clippy -p boyko-threadpool --all-targets -- -D warnings` → **exit 101**,
> *"error: unexpected `cfg` condition value: `reflect`"* spanned at the cfg
> (`lib.rs:151:7`), with *"expected values for `feature` are: `default` and `scheduler-trace`"*
> and *"`-D unexpected-cfgs` implied by `-D warnings`"* — and the canary's own text appears
> **nowhere** in the output (0 hits): the diagnostic is the red, the `compile_error!` never
> fires. A missing feature is a diagnostic, not a silence — the answer D16 and B6 gate 6 rest
> on, now measured. *Restoration:* every mutated file restored byte-identically (sha256 equal
> to the pre-mutation hash). *Regression:* G0's census (3 tests), G1 (7), G2 (2), G3 (2, the
> self-building census), G4 (6 + the 1-test leg-nonvacuity) all re-run **exit 0** with
> non-vacuous `running N` lines after the restorations.

---

### C1 — `Scalar` + `ScalarKind`

**Lands.** The 16-byte-target `#[repr(C)]` POD tagged union, `Copy`, no heap — EnTT's
`meta_any`-with-SOO idea specialized to POD. Per-kind constructors and checked extractors
(`as_f32() -> Option<f32>`, …) with the sign-extension rule for the `Ix` kinds written once.

**Gate.**
1. A layout pin `const _: () = assert!(size_of::<Scalar>() == N && align_of::<Scalar>() == M)`
   where `N`/`M` are the **measured** values, printed by the rung and recorded in the commit
   message.
2. Per-`ScalarKind` round-trip: `Scalar::from(x).as_<kind>() == Some(x)`, over a proptest with
   the kind's full range, including `i*::MIN`, `u*::MAX`, `f32/f64` `NaN`, `±0.0`, subnormals.
3. Cross-kind extraction returns `None` for every one of the 11 wrong kinds (a 12×12 matrix, all
   off-diagonal `None`).

**RED MUTATION.** Change `bits: u64` to `bits: u32`. Gate 1 reds on the pin **and** gate 2 reds on
`u64`/`i64`/`f64` round-trip — two independent reds, which is the point: the pin alone could be
"fixed" by editing the number.

*Second red, for the sign rule:* make `I8`'s extractor zero-extend instead of sign-extend. Gate 2
reds on negative values only — so the proptest range must include them, and the test name says so.

> **Executed 2026-08-21 (worktree `D:/wt/reflect`, toolchain `stable-x86_64-pc-windows-gnu` 1.97.1).**
> *Lands:* `crates/boyko_reflect/src/scalar.rs` — `ScalarKind` (12 kinds), the `#[repr(C)]`
> `Scalar { pub kind, pub bits: u64 }`, `From` constructors per kind, checked extractors
> (`Option`, D10's shape; a hand-built non-canonical payload is `None`, never a truncated
> value), and the sign rule written ONCE (`ix_to_bits`/`ix_from_bits` — store side
> sign-extends to `i64`, read side reinterprets the whole `bits` two's-complement and narrows
> via `try_from`, which is exactly the shape whose zero-extending typo the second RED
> observes). `ScalarKind::EntityId` carries the kernel's `EntityId` **index** (a `usize`
> newtype — the full `Entity {id, generation}` handle is 96 bits of payload and does not fit
> the cell; it is a later rung's question, not a scalar). Float trips are bit-exact
> (`to_bits`/`from_bits`), so the gate asserts BITS — value equality would reject NaN wrongly
> and accept a `-0.0 → 0.0` swap wrongly. `proptest` added as a **dev**-dependency (D18
> governs the ship-visible table; the workspace's dev-only instrument, `Cargo.toml:52`); the
> full-range proptests are `#[cfg_attr(miri, ignore = …)]` with the deterministic edge tests
> (MIN/MAX/NaN/±0/subnormals) carrying the edges under Miri. No `unsafe` anywhere in the rung.
> *Gate 1:* **measured size 16, align 8**, printed by
> `tests/c1_scalar.rs::scalar_layout_measured_and_pinned` and pinned at those values by the
> `const _` in `scalar.rs`. *Gates 2–3:* 19 tests green (`cargo test -p boyko-reflect
> --test c1_scalar`, exit 0), including the 12×12 off-diagonal-`None` matrix over an
> exhaustive `sample()` match (a new kind fails to compile until classified).
> *First RED, two stages as the rung intends ("the pin alone could be 'fixed' by editing the
> number"):* stage A — `bits: u32` with minimal casts, pin intact → **compile-time red**,
> `error[E0080]` *"Scalar layout moved off its measured pin (16 bytes, align 8)"*, exit 101;
> stage B — the adversarial "fix" (pin edited to 8/4) → **13 runtime reds** including the
> named `u64`/`i64`/`f64` round-trips (`u64_roundtrip_full_range`,
> `i64_roundtrip_full_range_negatives_included`, `f64_roundtrip_all_bit_patterns_bitwise`),
> exit 101 — two independent reds, the second surviving the first's defeat. *Second RED:*
> `as_i8` reading the payload unsigned (`i8::try_from(self.payload(…)?)`, the
> `ix_from_bits` call dropped) → 3 reds, **all negative-driven**: proptest shrank to
> **minimal failing input `v = -1`**, the edge test failed at `i8 -128`, and every positive
> stayed green (16 passed) — the red the test names promise. *Restoration:* byte-identical
> (sha256 verified against the pre-mutation copy). *Regression:* the G0–G4 battery re-run
> green and non-vacuous after restoration — G0 census 3, G1 census 7, G2 closure 2, G3
> absence census 1 (+1 ignored calibration; the three fat-LTO legs rebuilt against the C1
> surface and all cells held), G4 coverage 6, leg-nonvacuity 1 — plus
> `cargo test -p boyko-reflect --all-targets` 19 and
> `cargo clippy -p boyko-reflect --all-targets -- -D warnings` clean, touch-first.

---

### C2 — The registry: `REFLECT`, `install_type_info`, `type_info_of`

**Lands.**

```rust
use boyko_ecs::…::component_registry::MAX_COMPONENTS;    // IMPORTED, never redeclared (D5)

static REFLECT: [OnceLock<&'static TypeInfo>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

#[inline]
pub fn type_info_of(component_id: usize) -> Option<&'static TypeInfo> { … }

pub fn install_type_info(component_id: usize, info: &'static TypeInfo) { … }
```

Bounds discipline copied verbatim from `install_bind_accessor` (F6): `debug_assert!` **plus** a
release `if component_id >= MAX_COMPONENTS { return; }` guard, and write-once via `OnceLock::set`
with first-writer-wins.

**Gate.**
1. **Write-once idempotence:** two `install_type_info` calls with different `info` for one id —
   `type_info_of` returns the **first**, and the second is a silent no-op (F6's stated contract).
2. **Release out-of-bounds:** `install_type_info(MAX_COMPONENTS, …)` and
   `type_info_of(MAX_COMPONENTS)` are a no-op / `None` **in a release-profile test**, not only
   under `debug_assert!`.
3. **`MAX_COMPONENTS` is imported, not redeclared:** a source test greps this crate for a local
   `const MAX_COMPONENTS` / a bare `512` literal in an array length and fails on a hit.
4. **Footprint reported:** the static's `size_of` printed and pinned (§3.3 last row).
5. Miri (TB) over install + read.

**RED MUTATION.** Replace the import with `const MAX_COMPONENTS: usize = 512;` locally. Gate 3
reds. Then **separately** change `boyko_ecs`'s `MAX_COMPONENTS` to `256`: with the import the
crate rebuilds and gate 2's boundary moves with it; with the local const, the array is 512 long
against a 256-id space and **nothing reds** — which is exactly the drift the rule exists to
prevent, and seeing it is how the rule is believed.

*Second red:* make `install_type_info` use `OnceLock::get_or_init`-style last-writer-wins. Gate 1
reds.

> **Executed 2026-08-21 (worktree `D:/wt/reflect`, toolchain `stable-x86_64-pc-windows-gnu` 1.97.1).**
> *Lands:* `crates/boyko_reflect/src/registry.rs` — the `REFLECT` table exactly as sketched
> (`[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]`, `MAX_COMPONENTS` imported from
> `boyko_ecs::ecs::core::component::component_registry`), `type_info_of` (`#[inline]`, one
> acquire-load + branch), and G0's stub REPLACED by the real `install_type_info` — same name,
> same signature, still a plain `#[inline(never)] pub fn`, so GATES needle B survived (verified
> in the recalibration below: the symbol persists at its new module path
> `…13boyko_reflect8registry17install_type_info`, and `reflect_fixture`'s linkage deviation
> compiles against the new body unchanged, feature on and off). Bounds discipline copied
> verbatim from `install_bind_accessor` (F6): `debug_assert!` + release guard, first-writer-wins
> `OnceLock::set`. The placeholder `TypeInfo` stays opaque — C3's subject, not this rung's.
> *Gate 1 (and the reason the registry gates are UNIT tests):* the `#[non_exhaustive]`
> placeholder is deliberately unconstructible outside the crate, so only an in-crate test can
> mint the two distinct `&'static TypeInfo` subjects — and since `TypeInfo` is a ZST with no
> guaranteed-distinct static addresses, the instrument is a `#[repr(C)] { a: TypeInfo, _pad:
> u64, b: TypeInfo }` static (offsets 0 and 8 guaranteed) with the distinctness precondition
> ASSERTED before use. First writer wins observed; second install a silent no-op. *Gate 2:*
> `cargo test -p boyko-reflect --release --lib` → `running 5` (non-vacuous), both
> `#[cfg(not(debug_assertions))]` OOB tests green (install at `MAX_COMPONENTS`/`usize::MAX`
> no-ops; reads `None`); the debug twin `#[should_panic(expected = "exceeds maximum allowed")]`
> pins the loud half. *Gate 3:* `tests/c2_registry_source_census.rs` — needles `const
> MAX_COMPONENTS` + array-length `512` + a NON-VACUITY clause (the `use boyko_ecs::…` import
> must be seen); `#[cfg(not(miri))]` because Miri refuses host file I/O (the `CreateFileW`
> class measured at GATES G4's fifth RED). *Gate 4:* footprint MEASURED **8192 bytes = 16 per
> slot × 512**, printed by `footprint_reported_and_pinned`, pinned as `16 * MAX_COMPONENTS` so
> the pin moves with the kernel. *Gate 5:* `cargo +nightly-x86_64-pc-windows-gnu miri test -p
> boyko-reflect --all-targets` (MIRIFLAGS `-Zmiri-tree-borrows` from `.cargo/config.toml`) —
> 4 lib + 8 scalar-edge tests green, proptests carried as reasoned Miri-ignores.
> *First RED:* the local `const MAX_COMPONENTS: usize = 512;` → gate 3 reds naming
> `registry.rs:19`, exit 101. *The drift demonstration, with one execution finding:* the plan's
> "change `boyko_ecs`'s `MAX_COMPONENTS` to `256`" does not rebuild as written — the kernel is
> SELF-PINNED and three `E0080`s fire first (`size_of::<Archetype>() == 8704` and the two
> `bit_owners` base-spacing asserts in `filtered_access_set.rs`), so the demo carried the
> bound-move through as a real one would (bases re-spaced 0/256/512/768, `OWNERSHIP_SLOT_COUNT`
> 1024, the Archetype pin relaxed for the experiment's duration only). **Leg A (import in
> place): everything moved with the kernel** — `MAX_COMPONENTS = 256`, footprint 4096, gate 2's
> release OOB tests probing at the NEW boundary, all green. **Leg B (local const): the 512-slot
> table over the 256-id space — all four lib tests green, footprint still 8192, NOTHING REDS**
> except the source census; the drift the rule exists to prevent, seen. All five mutated files
> restored byte-identically (sha256-verified). *Second RED, one recorded deviation:*
> `get_or_init`-style is not buildable as last-writer-wins (`get_or_init` IS first-wins, and
> `OnceLock` has no evict through `&self`), so the mutation that expresses last-writer-wins is
> the `[AtomicPtr<TypeInfo>; MAX_COMPONENTS]` store/load table — which is exactly D5's
> forbidden shape, so the mutation doubles as its demonstration. Gate 1 red observed with its
> named message (*"FIRST WRITER MUST WIN … last-writer-wins semantics, which is not OnceLock's
> contract and not F6's"*), exit 101. *G3 recalibration (the G0 measured note's standing
> instruction):* `measure_link_configuration_table --ignored` re-run against the real surface
> and the table re-pasted at GATES §G3 — gated cells unmoved (L1/L3 = 0 in every row, both
> needles), L2's magnitude 1 → 6/6/5 (the pulled-object rule visible; fat LTO strips the
> `__imp_` thunk); the drop-LTO RED re-run **stayed green a second time** (both gated zeros
> protected upstream of the linker), ledger row extended, next re-run scheduled at C7.
> *Regression:* the G0–G4 battery + both `boyko-reflect` profiles + clippy touch-first all
> green after restoration (tails in the rung report).

---

### C3 — `TypeInfo` / `FieldInfo`, hand-baked

The model must be shown expressible **before** a macro is asked to emit it. This rung writes
`TYPE_INFO` statics by hand for a fixture covering every `ValueKind` arm.

**Lands.**

```rust
#[repr(C)]
pub struct TypeInfo {
    pub type_name: &'static str,          // std::any::type_name — diagnostics only
    pub type_id_fn: fn() -> TypeId,       // TypeId::of is NOT const → fn-ptr, never a static field
    pub size: usize,
    pub align: usize,
    pub fields: &'static [FieldInfo],     // 'static slice, baked — no Vec
    pub kind: TypeKind,
    pub enum_info: Option<&'static EnumInfo>,   // Some iff kind == Enum (C10)
    pub default_in_place: Option<unsafe fn(*mut u8)>,  // None iff the type has no `Default` — D20
    pub drop_in_place: Option<unsafe fn(*mut u8)>,
}

#[repr(C)]
pub struct FieldInfo {
    pub name: &'static str,               // load-bearing: the deserialize key — NOT strippable
    pub offset: usize,                    // core::mem::offset_of! — const
    pub type_id_fn: fn() -> TypeId,
    pub kind: ValueKind,
    pub get: Option<unsafe fn(*const u8) -> Scalar>,
    pub set: Option<unsafe fn(*mut u8, Scalar) -> bool>,
    pub nested: Option<&'static TypeInfo>,
    pub enum_info: Option<&'static EnumInfo>,
    pub array: Option<ArrayInfo>,         // { elem: ScalarKind, stride: usize, len: usize }
}
```

No `stable_name` (D8). No `serialize`, no `debug_fmt` (D9). **Every** accessor `Option` — D10, and
`default_in_place` is no longer the one exception (D20).

**Gate.**
1. **Kind/accessor coherence, asserted:** a `validate(info) -> Result<(), Vec<Problem>>` walked
   over the fixture — `Prim` ⇒ `get.is_some() && set.is_some() && nested.is_none() &&
   array.is_none()`; `Nested` ⇒ `nested.is_some() && get.is_none()`; `Array` ⇒
   `array.is_some()`; `Enum` ⇒ `enum_info.is_some()`; `Opaque` ⇒ **all** accessors `None`. One
   arm per `ValueKind`, exhaustively matched so a new arm fails to compile until it is classified.
2. `get_field` (the scalar API) returns `None` for every non-`Prim` kind — no silent garbage
   `Scalar` (D10 / FIX Mi2).
3. `(info.type_id_fn)() == TypeId::of::<T>()` for each fixture type.
4. `offset` agrees with `core::mem::offset_of!` for every fixture field (the hand-baked statics
   *are* `offset_of!`, so this gate is trivially green here — it exists so C7 inherits a
   comparison target that is already independently pinned).

**RED MUTATION.** In the fixture, install a `Nested` field with `get: Some(prim::get_f32)`. Gate 1
reds with a named `Problem`. *Second red:* set a `Prim` field's `offset` to `0` when it is not the
first field — gate 4 reds.

> **Executed 2026-08-21 (worktree `D:/wt/reflect`, toolchain `stable-x86_64-pc-windows-gnu` 1.97.1).**
> *Lands:* `crates/boyko_reflect/src/type_info.rs` — `TypeInfo` / `FieldInfo` exactly as
> sketched, plus the four descriptors they point at (`ArrayInfo`, `EnumInfo`, `VariantInfo`
> and `EnumRepr`, which the sketch's slots name and which therefore cannot be deferred past
> the commit that declares those slots), `TypeInfo::get_field`, and `validate` with its
> `Violation`/`Problem` pair. C2's `#[non_exhaustive]` ZST placeholder is DELETED from
> `lib.rs`; `registry.rs`'s unit tests, which minted their two distinct subjects out of a
> `#[repr(C)]` container *because* the placeholder was a ZST, now build real descriptors, and
> the comment stating why those gates are unit tests is REWRITTEN rather than inherited — the
> old argument (*"not constructible outside this crate"*) stopped being true at this rung, and
> a stale rationale for a live test is the doc-rot class this campaign keeps paying for. The
> registry's own five tests and its `install_type_info` name/signature are untouched, so GATES
> needle B and G3's calibration are unaffected (verified: `reflect_fixture`'s two
> `fn(usize, &'static boyko_reflect::TypeInfo)` coercion sites still compile, feature on and
> off, and G3's census re-ran green).
>
> *Two deviations, both recorded rather than worked around:*
>
> 1. **`ValueKind::Str` is not in gate 1's rule list, and the gate's own text forbids leaving it
>    unclassified.** The rung enumerates five arms (`Prim`, `Nested`, `Array`, `Enum`,
>    `Opaque`) but the taxonomy has **six**, and the same sentence demands *"one arm per
>    `ValueKind`, exhaustively matched so a new arm fails to compile until it is classified"* —
>    a match with no wildcard cannot be written without deciding `Str`. **Decided here:** until
>    C11 lands the string accessor pair, a `Str` field is *structurally accessorless*
>    (`Violation::StrWithAccessor`), i.e. shaped like `Opaque` but labelled honestly. This is a
>    rule C11 **replaces in the same commit that gives `Str` something to call**, and it is
>    named in `Violation::StrWithAccessor`'s own doc comment so C11's author meets it there.
> 2. **The first RED mutation names `prim::get_f32`, which does not exist at C3** —
>    `prim::` is C4, the next rung (§5's order, and C4's own Lands). The mutation is executed
>    with the fixture's hand-written `hand::get_f32`, identical in signature and role; what the
>    mutation *tests* is a `Nested` field carrying a scalar getter, and the accessor's
>    provenance is incidental to that. Recorded because it is literally the "an API the
>    mutation assumes does not exist" class, and because the hand-written accessors are
>    deliberately **kept** after C4 lands, as the standing witness that the model is expressible
>    without the library.
>
> *Two rules added beyond the rung's list, both stated so they are not mistaken for the
> plan's:* the type-level *"`enum_info` is `Some` **iff** `kind == Enum`"* is checked in **both**
> directions (a one-sided check cannot see an `EnumInfo` hung on a struct), and `Nested`/`Array`
> reject a scalar **setter** as well as a getter — the rung lists only `get.is_none()` for
> `Nested`, and an accessor that *writes* a nested struct's first bytes is the same defect with
> worse consequences.
>
> *Gate 1:* `every_fixture_descriptor_is_coherent` over all three hand-baked statics, plus
> `the_fixture_covers_every_value_kind_arm` — a NON-VACUITY clause, because a green `validate`
> over a fixture missing three arms is a statement about the arms it happened to contain.
> *Gate 2:* `get_field_returns_none_for_every_non_prim_kind` walks all seven fields and asserts
> `Some` for `Prim` / `None` for the other five kinds. **`get_field` checks the KIND first and
> independently of the accessor slot** — a design decision this rung took and the plan does not
> state: keying on `get.is_none()` alone would make gate 2 a second reading of gate 1, and a
> malformed descriptor would then read `Inner`'s first four bytes as the whole field. The
> independence is asserted
> (`get_field_refuses_a_malformed_nested_descriptor_even_though_it_has_a_getter`) and was
> **observed live**: under the first RED, gate 1 went red while gate 2 stayed green. *Gate 3:*
> `(type_id_fn)()` checked on all three types and all nine fields. *Gate 4:*
> `every_baked_offset_equals_offset_of` plus the `size`/`align` and `ArrayInfo`-stride halves.
> 13 tests, `cargo test -p boyko-reflect --test c3_type_info` exit 0.
>
> *First RED:* `EVERYTHING_FIELDS[3]` (`inner`, `Nested`) given `get: Some(hand::get_f32)` →
> **exit 101**, gate 1 red with the named problem *"`c3_type_info::Everything` is INCOHERENT:
> field #3 `inner`: NestedWithScalarAccessor"*, 12 passed / 1 failed. *Second RED:* `level`'s
> `offset` hand-edited from `offset_of!(Everything, level)` to `0` → **exit 101, TWO
> independent reds**: gate 4 named it exactly (*"field #1 `level`: baked offset 0 != offset_of!
> 4"*) and `get_field_reads_prim_fields_back` printed the shipped consequence —
> `level` read back as **`Some(1095237632)`**, which is `12.5f32.to_bits() == 0x4148_0000`:
> the `u32` field reading the `f32` field's bits. The plan predicted one red; the *value* is
> what makes the defect legible, and it is measured here rather than described.
> *Restoration:* byte-identical, sha256 `e7e78a85…4799` before and after both mutations.
> *Miri:* `cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-reflect --all-targets`
> (TB, from `.cargo/config.toml`) — 4 lib + 8 scalar-edge + **13 C3** tests green, exit 0; the
> new `get_field` pointer arithmetic and the fixture's `&*(p as *const T)` reads are covered.
> *Regression:* G0 census 3, G1 manifest 7, G2 closure 2, G3 absence 1 (+1 ignored
> calibration), G4 coverage 6, leg-nonvacuity 1, `boyko-reflect` debug 4 + 19 + 1 + 13 = 37
> across four binaries, release `--lib` 5, clippy `-D warnings` touch-first clean, and
> `cargo check -p reflect-fixture/-p reflect-dogfood --all-targets --features reflect` both
> exit 0.

---

### C4 — The `prim::` accessor library + the release kind check

**Lands.** For each `ScalarKind`, a monomorphic pair

```rust
// SAFETY: `p` points at a live, initialized, `align_of::<T>()`-aligned instance of the
// field type this fn-ptr was installed for; `offset_of!` guarantees in-bounds and
// field-aligned; provenance is inherited from the arena-rooted base the caller derived.
pub unsafe fn get_f32(p: *const u8) -> Scalar { … }
pub unsafe fn set_f32(p: *mut u8, v: Scalar) -> bool { … }   // false on kind mismatch
```

The `set` half performs the **release** kind check and returns `false` (D11) **before** touching
memory. Reads use the `Bindable` pattern (`&*(p as *const T)`) — precedented and Miri-clean (F11);
**every writer stays raw** (`ptr::write`, never an intermediate `&mut T`), which is the asymmetry
analysis B.7 identifies and which was not written down anywhere before that revision. That
asymmetry is restated in this module's header comment, because it is the module where both halves
live side by side.

**Gate.**
1. Per-kind get/set round-trip through a `#[repr(C)]` fixture, reading back through the typed
   field.
2. **`set` with a mismatched kind returns `false` and leaves the bytes UNCHANGED** — asserted by
   byte comparison of the whole struct, not by reading the one field.
3. **Gate 2 runs in a RELEASE-profile test** (`cargo test --release`), because that is the build
   where `debug_assert!` vanishes and the editor case lives (D11).
4. Miri under `-Zmiri-tree-borrows` over the whole module. Requires
   [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md) to have added ~~`-p boyko-reflect` **with
   the feature ON**~~ **`-p boyko-reflect` PLAIN** to the Miri allowlist (F18) — *"a gate that cannot
   fail"* otherwise.

   > **Corrected 2026-08-21 (second pass).** *"With the feature ON"* was inherited from analysis B.9
   > and is unrunnable for this crate: `boyko_reflect` carries **no** `reflect` feature (GATES D4 —
   > the `#[cfg(feature = "reflect")]` in derive output is a *consumer-side* construct), so
   > `cargo +nightly miri test -p boyko-reflect --features reflect` is a **hard cargo error**, and
   > with the feature "off" the crate is **not empty** — nothing in its source is `cfg`-gated.
   > **This rung's subject is exactly what a plain `-p boyko-reflect` row covers**: the `prim::`
   > accessors run over hand-built pointers into a local `#[repr(C)]` fixture, with no derive
   > involved. The row that covers *derive-generated* `unsafe` is the fixture's
   > (`-p reflect-fixture --features reflect-fixture/reflect`), and it is what C11 gate 3 depends on
   > — see there. GATES **D4** is the single owner of this wording; this gate cites it.
5. Alloc-delta harness arm: `Prim` get/set = 0 (§3.3).

**RED MUTATION.** Replace the `-> bool` kind check with `debug_assert_eq!(v.kind, K); true`. In a
**debug** build the assertion fires, so the debug test reds — but it reds as a **panic**, not as a
refusal, which is a different failure and is the kind a maintainer "fixes" with `#[should_panic]`.
**Gate 3 is the one that catches the real defect:** in release the assertion vanishes, the setter
writes a `u32`'s bits into an `f32` field, and gate 2's byte comparison fails. *This is the
mutation this rung exists for* — the defect is invisible to a debug-only gate, and the
release-editor build is exactly where it would ship.

*Second red:* make `set_u32` write before checking. Gate 2 reds on the byte comparison even though
the return value is still `false`.

> **Executed 2026-08-21 (worktree `D:/wt/reflect`, toolchain `stable-x86_64-pc-windows-gnu` 1.97.1).**
> *Lands:* `crates/boyko_reflect/src/prim.rs` — twelve `get_*`/`set_*` pairs, one per
> `ScalarKind`, generated from a single `prim_accessors!` arm. **The macro is a decision, not
> a shortcut:** C1's second RED was one per-kind extractor typo (a zero-extending `as_i8`),
> and twenty-four hand-copied bodies are twenty-four chances to write it again — the macro
> removes the *drift*, the gates stay per-kind, so nothing is removed from the *coverage*.
> Reads use `&*(p as *const T)` (F11's precedented, Miri-clean trampoline pattern); **every
> writer stays raw** (`ptr::write`, never an intermediate `&mut T`), and the asymmetry is
> restated in the module header where both halves live, as the rung requires. The kind check
> is `Scalar`'s own checked extractor, which makes the release `-> bool` and the
> non-canonical-payload refusal **the same branch** — a `Scalar { kind: U8, bits: 300 }` never
> reaches the store either. **No `#[inline]`**, deliberately: every production call arrives
> through a `FieldInfo` fn-pointer slot where the attribute cannot help (§8 says so), and
> principle 7 forbids doctrine-driven inlining; the paragraph naming that decision also names
> what would reverse it.
>
> *One defect found in the rung as written, and it is gate 5's:* **C4 gate 5's instrument was
> specified to land at C5.** §3.3 said *"Instrument (rung C5)"* and its table gave the
> `Prim get / set` row to C5, while this rung's gate 5 demands that arm one rung earlier. The
> full record, the resolution (instrument lands here; C5 keeps `enumerate` + `array read`) and
> two measured corrections to the instrument's own design are at **§3.3**, where they govern.
>
> *Gate 1:* `every_kind_round_trips_through_its_accessor_pair` (12 kinds through a
> `#[repr(C)]` fixture) plus `every_kind_lands_in_its_own_typed_field`, which writes all twelve
> and compares the **whole struct** against a literal — a pair that consistently mis-addressed
> the same wrong bytes would agree with itself, so the `Scalar`-level round-trip alone is not
> the "read back through the typed field" the rung asks for. *Gate 2:* the full **12×12**
> mismatch matrix (132 off-diagonal cells), each asserting `false` **and** byte equality of the
> whole struct, plus a non-canonical-payload row. *Gate 3:* both profiles run, and they are
> distinguishable rather than asserted — `debug_leg_is_live_and_debug_assert_still_fires` /
> `release_leg_is_live_and_debug_assert_is_gone` observe at **runtime** whether a
> `debug_assert!` executed, so "the release leg ran" is a measurement and not a `cfg!` restated
> as an assertion. 7 tests, exit 0 in both profiles. *Gate 4:* `cargo
> +nightly-x86_64-pc-windows-gnu miri test -p boyko-reflect --all-targets` (TB) — 4 lib + 8
> scalar-edge + 13 C3 + **7 C4** green, exit 0. *Gate 5:* **measured, all deltas exactly 0** —
> 12 kinds × {get, set, refused set} against an identically-shaped no-op, 1000 calls per window
> (`get F32: baseline=0 measured=0 delta=0`, and so on for every row), with a permanent
> positive control (`deliberate allocations observed = 1`) in the same binary so a green can
> never mean "the counter never armed". The fixture's own precondition is asserted too:
> **`size_of::<AllPrims>() = 56`, sum of field sizes `= 56`** — zero padding, because gate 2's
> whole-struct byte comparison would otherwise be reading uninitialized memory, which is UB and
> which gate 4 reds on.
>
> *First RED (the mutation this rung exists for), two profiles, two DIFFERENT failures — which
> is the whole claim:* `set_f32`'s `-> bool` check replaced by
> `debug_assert_eq!(v.kind, ScalarKind::F32); true` with an unconditional
> `ptr::write(p, f32::from_bits(v.bits as u32))`. **Debug → exit 101, and it reds as a PANIC
> inside the library** (`prim.rs:142: assertion left == right failed; left: Bool, right: F32`)
> — not as a refusal, and exactly the shape a maintainer "fixes" with `#[should_panic]`.
> **Release → exit 101, and it reds as a CONTRACT VIOLATION in the test**
> (`set_F32 accepted a Bool scalar -- the kind check did not refuse`), the assertion vanished
> and the setter accepted. *Recorded precisely:* in release the mutation trips gate 2's
> **return-value** clause first, because the mutated setter returns `true` unconditionally; the
> rung's sentence *"gate 2's byte comparison fails"* describes the clause the **second** RED
> isolates, which is why the rung specifies two.
>
> *Second RED:* `set_u32` storing before checking (`ptr::write(…, v.bits as u32)` then
> `v.as_u32().is_some()`). The return value stayed **correct** — gate 2's `assert!(!wrote)`
> clause PASSED — and **only the byte comparison saw it**, in both profiles, exit 101:
> *"set_U32 REFUSED a Bool scalar and still changed the bytes -- the check runs after the store
> (field `u32_`, offset 32)"*, with the diff visible in the dumps at offset 32:
> `239,190,173,222` (`0xDEAD_BEEF`) → `1,0,0,0`. That is the rung's claim about why the gate
> compares bytes rather than return values, measured.
>
> *Third RED, added because the instrument moved here:* `let _ = Vec::<u8>::with_capacity(1);`
> inserted into the macro's reader → gate 5 exit 101,
> *"prim::get for Bool allocated **1000** time(s) over the no-op baseline in 1000 calls"* —
> one per call, no noise, and the three setter/refusal arms stayed green. A zero-allocation
> harness whose red nobody has seen is not a harness; this one's has been seen.
>
> *Restoration:* `prim.rs` restored byte-identically after each of the three mutations
> (sha256 `01c9959e…c86c`). *Regression:* G0 census 3, G1 manifest 7, G2 closure 2, G3 absence
> 1 (+1 ignored calibration), G4 coverage 6, leg-nonvacuity 1; `boyko-reflect` debug
> 4 + 19 + 1 + 13 + 7 + 4 = **48** across six binaries and release 5 + 19 + 1 + 13 + 7 + 4 =
> **49**; `cargo check -p boyko-reflect --all-targets` and `cargo clippy -p boyko-reflect
> --all-targets -- -D warnings` (touch-first, both profiles) exit 0.

---

### C5 — `ValueKind::Array` + the zero-allocation harness

**Lands.** `ArrayInfo { elem: ScalarKind, stride: usize, len: usize }` and the by-index element
accessor `array_get(p, &ArrayInfo, i) -> Option<Scalar>` / `array_set(…) -> bool`, with `i < len`
a **release** bounds check (same reasoning as D11). ~~Plus the counting-allocator delta harness
(§3.3), and its first three arms.~~ → **the harness itself and the `Prim` get/set arm landed at
C4** (§3.3's correction of 2026-08-21: C4 gate 5 named an arm whose instrument this rung was
specified to build). What C5 adds is the remaining two arms — **`enumerate` and `array read`** — on
top of the existing `tests/c4_prim_zero_alloc.rs` instrument, whose thread-local counter and
`#![cfg(not(miri))]` disposition are both measured facts C5 inherits rather than re-decides.

**Scope of the arm — D19, decided here rather than discovered mid-gate:** v1 supports `[T; N]`
where **`T` is a `Prim`, and only that**. `[[f32; 4]; 4]` is an array *of arrays*, so it needs a
2-D descriptor or a recursive `ArrayInfo`; both are v2. A matrix field therefore falls to `Opaque`
and is refused by D15 unless it carries `#[reflect(skip)]`. **The named victim is
`csm_config.rs:392`'s `view_proj: [[f32;4];4]`**, and it is named here so it is a recorded
exclusion rather than a surprise at first `#[component(reflect)]`. `GpuTransform3D`/`TrsPacked`
— the case D12 exists for — is `[f32;4]`×3 and is fully covered. *Rejected:* a recursive
`ArrayInfo { elem: ArrayInfo }` in v1 (it reintroduces an unbounded descend that §3.1's
acyclicity argument does not cover, since the recursion is now in the *descriptor* rather than in
the Rust type).

**Gate.**
1. Element round-trip over `[f32;4]`, `[u8;60]`, `[u32;1]`, `[i8;3]` — the sizes chosen so one
   arm is the flagship's shape, one is the inline-string shape (`UiName { bytes: [u8; CAP] }`),
   one is degenerate, and one is signed.
2. `i == len` and `i == usize::MAX` return `None`/`false` in a **release** test.
3. Alloc-delta = **0** for enumerate, `Prim` get/set, and array read.
4. Layout: `stride == size_of::<T>()` asserted against `offset_of!`-derived spacing for a fixture
   with a padded element type.

**RED MUTATION.** Set `stride` to `size_of::<T>() - 1` in the fixture. Gate 1 reds on element
1..n; **gate 4 reds on the stride identity**, which is the gate that would catch it in a real
derive bug where `stride` came from the wrong `size_of`.

*Second red (the harness's own):* insert `let _ = Vec::<u8>::with_capacity(1);` into `array_get`.
Gate 3 reds. **A zero-allocation harness whose red nobody has seen is not a harness** — this
mutation is run and its output recorded in the commit message. *(The instrument's red has already
been seen ONCE, at C4, where the harness landed: the same insertion into the `prim::` readers
produced* `prim::get for Bool allocated 1000 time(s) over the no-op baseline in 1000 calls` *— one
per call, no noise, and the setter arms stayed green. C5 still runs its own, because what is
untested here is the `array read` **arm**, not the counter.)*

> **Executed 2026-08-21 (worktree `D:/wt/reflect`, toolchain `stable-x86_64-pc-windows-gnu` 1.97.1).**
> *Lands:* `crates/boyko_reflect/src/array.rs` — `array_get` / `array_set` over an
> `ArrayInfo`, with the `index < len` refusal ordered **before** the `index * stride`
> multiply; and `crates/boyko_reflect/src/prim.rs` gains `getter_for` / `setter_for`,
> the by-`ScalarKind` dispatch the element accessors need. `ArrayInfo` itself already
> existed — C3 baked it — so this rung adds the *reader*, not the descriptor.
> **The dispatch is a wildcard-free `match` generated from the same `prim_accessors!`
> rows as the twenty-four accessors**, not the house-standard dense array indexed by
> `kind as usize`: this table is resolved at compile time and lowers to a jump table
> either way, and only the `match` makes a newly added `ScalarKind` a *compile error*
> until it has a pair. The discriminant-indexed array would answer a new kind with
> whatever sat at that index — the shape this campaign keeps paying for. The macro rows
> gained a leading `ScalarKind` ident and nothing else moved.
>
> *Gate 1:* `every_element_round_trips_through_the_by_index_accessors` over `[f32;4]`,
> `[u32;1]`, `[u8;60]`, `[i8;3]`, plus `every_element_lands_at_its_own_address`, which
> writes all four arrays into ONE fixture and compares the whole struct against a
> literal. **Both write every element before reading any**, deliberately: the rung's own
> RED collapses a byte-wide array's stride to **0**, and a per-element write-then-read
> pair would then agree with itself perfectly — C4 gate 1's self-consistency defect,
> one rung later and with a different cause. A third test pins that the by-index writer
> *adds* a bounds check rather than *replacing* D11's kind check. *Gate 2:*
> `an_out_of_bounds_index_is_refused_in_both_directions` over `len`, `len + 1` and
> `usize::MAX`, both accessors, with the fixture's bytes compared before/after; run in
> both profiles, each carrying the runtime liveness probe (`debug_assert! executed =
> true` / `= false`). *Gate 4:* three separate claims per shape — the descriptor's
> `stride == size_of::<T>()`, the **measured** spacing of real element addresses, and
> `stride * len == size_of::<[T; N]>()` — plus a **non-vacuity clause** proving the
> fixture actually pads after an array, so "spacing taken from the next field's offset"
> is a demonstrably different number: measured `offset_of!(tail) - offset_of!(i3) = 4`
> against `size_of::<[i8;3]>() = 3`, `size_of::<Arrays>() = 88`. Without that clause the
> gate is a tautology on a tightly packed struct. `u1` is len 1, so its measured-spacing
> claim is *not defined*; the test prints that rather than silently skipping it.
>
> *Gate 3, MEASURED, both new arms exactly 0:* `enumerate (1 field(s)): baseline=0
> measured=0 delta=0` and `array read (len 4): baseline=0 measured=0 delta=0`, on the
> C4 instrument, in the same binary as the permanent positive control
> (`deliberate allocations observed = 1`). The arms live in `tests/c4_prim_zero_alloc.rs`
> and not in a `c5_*` twin because **a `#[global_allocator]` is one per binary** — a
> second file would fork the allocator, the arming protocol and the positive control into
> two copies free to drift. The array arm carries its own non-vacuity test
> (`the_array_arm_actually_reads_the_array`), because a measured window that returned
> `None` for every call would report delta 0 while measuring nothing.
>
> *First RED (stride from the wrong `size_of`), one edit at the single `descriptor_stride`
> site, exit 101, **three** tests red:* gate 4 named the defect directly
> (`f4: descriptor stride is not size_of::<f32>(): left: 3 right: 4`) and both gate-1
> tests red on the data. *Recorded precisely, because the rung's prediction is off by
> one element:* the rung says gate 1 *"reds on element 1..n"*, and the observed red is at
> **element 0** — with `stride = 3` the writes of elements 1–3 overlap element 0's bytes
> and destroy it before it is ever read (`f4[0] ... left: bits 0, right: bits 1056964608`).
> The whole-struct comparison shows the same thing across all four arrays at once. *(The
> mutation is UB by construction — `stride = 3` makes three of four `f32` reads
> misaligned — so it was run natively and deliberately not under Miri.)*
>
> *Second RED (the arm's own):* `let _ = Vec::<u8>::with_capacity(1);` inserted into
> `array_get` → gate 3 exit 101, *"array_get allocated **4000** time(s) over the no-op
> baseline in 4000 calls"* — exactly one per call, no noise, and the `prim` and
> `enumerate` arms stayed green in the same run.
>
> *Third RED, not named by the rung, run because gate 2 otherwise has no observed red —
> and it REFUTED a sentence this rung's first draft had written into the module header.*
> Moving the bounds check below the multiply: **debug → exit 101**, and it reds as
> *"attempt to multiply with overflow"* raised at `array.rs:63` — a **panic out of a
> library whose entire contract is to refuse rather than fail**, which is the shape a
> maintainer "fixes" with `#[should_panic]`. **Release → exit 0, GREEN**: the wrapped
> product is simply discarded, because the check is still keyed on `index`. So the cost
> of the wrong order is a **debug-only panic, not a release wild read** — and the claim
> "wraps into a wild pointer in a release build" is false for this shape and was deleted
> from `array.rs`'s header and from the gate's doc comment. (`usize::MAX * stride` wraps
> to `2^64 - stride`, which is above any real extent, so even an offset-keyed check
> refuses it.) The rung is stronger for it: gate 2 is now documented as load-bearing in
> the profile where it actually bites.
>
> *Restoration:* `array.rs`, `prim.rs`, `c5_array.rs`, `c4_prim_zero_alloc.rs` restored
> from file copies after each mutation and verified by sha256 (`array.rs`
> `6f69b7e6…1c03`, `c5_array.rs` `3c92b997…5100`). *Gate:* `cargo test -p boyko-reflect
> --all-targets --no-fail-fast` — debug 4 + 19 + 1 + 13 + 7 + 7 + 8 = **59** across seven
> binaries, release 5 + 19 + 1 + 13 + 7 + 7 + 8 = **60**, exit 0 both; `cargo +nightly
> miri test -p boyko-reflect --all-targets` (TB) 4 + 8 + 0 + 13 + 7 + 0 + **8** green,
> exit 0 — C5's new `unsafe` is covered, and `c4_prim_zero_alloc` is the `running 0
> tests` its `#![cfg(not(miri))]` intends; `cargo check` / `cargo clippy -p boyko-reflect
> --all-targets -- -D warnings` (touch-first, both profiles) exit 0. *Regression:* G0
> census 3, G1 manifest 7, G2 closure 2, G3 absence 1 (+1 ignored calibration), G4
> coverage 6, leg-nonvacuity 1, `internal_docs_anchors` 5 — all exit 0.

---

### C6 — `Nested`: the recursion contract, read side

**Lands.** `NestedCursor<'a> { ptr: *const u8, info: &'static TypeInfo, _pd: PhantomData<&'a ()> }`
— `Copy`, re-rootable, with `type_info()` and `fields()` so enumeration works at depth ≥ 1 (the
gap the analysis's own completeness pass caught). `FieldValue<'a>` gains its `Nested` arm; the
bare `{ptr, info}` variant is **deleted** and never introduced (M2/O3). **The `'a` is the
validity guarantee** — compiler-enforced, not a documented contract: a `NestedCursor` cannot
coexist with a `&mut EcsMaster`.

**Also lands (architect's C6 ruling, 2026-08-21 — D21, §3.1):** the two rules that make the descend
addressing-valid and finite, as **two new `Violation` arms on `validate`** — the coherence checker
C3 shipped, *"one `Problem` per violation, over a match with no wildcard arm"*:

* `Violation::NestedNotInline` — **Check A**, in the existing `ValueKind::Nested` arm of that
  match, O(1) per edge: `offset + child.size <= parent.size`, `offset % child.align == 0`,
  `child.align <= parent.align`.
* `Violation::NestedCycle` — **Check B**, a DFS over `Nested` edges from the validated root, with a
  **fixed on-stack `[*const TypeInfo; N]` path array and a linear scan**. No `Vec` growth, no
  `HashMap` (D18), no allocation on the clean path.

Both are **validation-time**. The descend path is unchanged — still one `add` plus one pointer copy
per level, still 0 allocations (§3.3) — so this costs nothing per frame and nothing per level.
Neither check enumerates a type name, so neither is half of a drift pair with C9.

**And `trybuild = "1"` as a `[dev-dependencies]` entry of `boyko-reflect`** (gate 5's harness; see
gate 5 and the G1 clause below). **This does not violate D18.** D18 governs the crate's
*ship-visible* dependency surface — the manifest already states it in those words (*"D18 governs the
SHIP-VISIBLE table above; dev-dependencies enter no consumer's resolved closure"*, written when
`proptest` landed for C1). `trybuild` is the workspace's vetted compile-fail harness, declared the
same way in **five** members already, e.g. `crates/boyko_ecs/Cargo.toml:91` (`trybuild = "1"` under
`[dev-dependencies]`), and also `crates/boyko_ui/Cargo.toml`, `crates/boyko_log/Cargo.toml`,
`crates/aether_tests/Cargo.toml`, `crates/boyko_rhi_vulkan/Cargo.toml`. It is pinned as a bare
`"1"` rather than through `[workspace.dependencies]` because every existing site does — hoisting it
is a separate change and is not smuggled into this rung.

**Gate.**
1. **Two packages, two claims** — and they are not the same claim, which is why the first revision's
   single sentence could not be built:
   * **In `reflect_fixture`** (the primary subject, FFI-free, on the Miri allowlist): descend a
     locally-declared depth-2 nest and a locally-declared tuple struct, and read a leaf of each.
     This is the gate C6's RED mutations act on and the one gate 4 below runs under Miri.
   * **In `reflect_dogfood`** (the engine-types claim, off the Miri allowlist): the same descent over
     `Transform → Vec3 → f32` and `Name → NameId → u32` — real engine types, so this half is a
     dogfood. It lands with, and is gated by, `boyko_scene`'s `reflect` feature (§0.3).
   > **Corrected 2026-08-21 (second pass).** The first revision said *"Both are real engine types, so
   > this gate is a dogfood, not a fixture exercise"* — and `Transform`/`Name` live in `boyko_scene`,
   > which nothing could opt in under the old leaf-only feature rule, and which a `reflect_fixture`
   > restricted to `boyko-ecs`/`boyko-macros`/`boyko-reflect` (GATES G4's Miri constraint) cannot
   > reach at all. Both halves are recoverable; neither is recoverable in one package.
2. **Alloc-delta = 0 at depth ≥ 2** (§3.3) — a depth-1 harness is explicitly not accepted.
3. ~~**Acyclicity is asserted where it is actually enforced:** a test that the C9 refusal list
   covers every indirection kind (`Box`, `Vec`, `&T`, `Option<Box<_>>`, raw pointers). This is
   the acyclicity proof (§3.1); there is no runtime guard, so this test is the only thing
   standing between v1 and an infinite descend.~~
   → **`validate` refuses a hand-baked `Nested` edge two ways, on two separate fixtures** — the
   rewrite the architect's ruling prescribes, and unlike the struck text it names an instrument
   that exists at this rung:
   * **3(i) — addressing-validity.** A fixture whose `Nested` field points at a child descriptor
     **larger than the field's extent** (`offset + child.size > parent.size`) ⇒ a named
     `Violation::NestedNotInline`. This is the mis-described-container case — the one a refusal
     list is *supposed* to catch and cannot, since no list can name every indirection — and it is
     caught at **depth 1**, by arithmetic over two `usize`s, with nothing enumerated.
   * **3(ii) — acyclicity.** A hand-baked **cyclic** `TYPE_INFO` graph (`A.nested -> B`,
     `B.nested -> A`, both descriptors sized so Check A is *satisfied* at every edge) ⇒ a named
     `Violation::NestedCycle`. That fixture is also the proof that the two checks are **not**
     redundant: gate 3(ii) asserts the same graph produces **no** `NestedNotInline`, so Check A's
     green on a cycle is measured rather than conceded.

   Both fixtures are **hand-written statics**, which is exactly what C3's gates already walk
   (`tests/c3_type_info.rs`), so gate 3 is constructible **at C6** with nothing borrowed from C7's
   derive or C9's census.
4. Miri (TB) over the descend path.
5. `compile_fail`: a `NestedCursor` held across a `&mut` op does not compile. The harness is
   `trybuild`, newly a dev-dependency of `boyko-reflect` (see **Lands**). **Because that is a
   manifest change, this gate also re-runs G1's manifest census** (`tests/reflect_manifest_census.rs`,
   all six clauses plus its non-vacuity clause) and the reason for the new entry is stated **at the
   manifest**, next to the `proptest` rationale. Adding a dependency silently is precisely what G1
   exists to catch; a green census that nobody re-ran after a manifest edit is not a green census.

**RED MUTATION.** ~~Delete `Vec` from C9's refusal list and add a `Vec<u8>` field to a fixture with
a hand-baked `Nested` pointing back at the outer type. Gate 3 reds.~~ **STRUCK 2026-08-21: the
mutation edits a datum that does not exist until four rungs later.** `REFUSALS` is C9's, C9 is
downstream of C7's derive, and §5's ladder is `C6 → C7 → C8 → C9` — measured on this worktree, the
string `REFUSALS` occurs **only inside this document's own C9 prose** and in no source file. A red
that cannot be performed at the rung it belongs to is not a red.

→ **The rung's two reds, both acting on data and code this rung itself lands:**

* **First red (Check B).** Delete the cycle test from the DFS — i.e. keep the walk but stop
  comparing the child against the path — and run gate 3(ii)'s cyclic fixture. It reds by
  **stack/array overflow or by hanging**, so the leg is run under `--no-fail-fast` and the observed
  failure mode is recorded rather than predicted. *Without Check B the same graph produces a stack
  overflow at some later rung and looks like a bug in whoever descended.*
* **First red, second half (Check A).** Separately, delete the `offset + child.size <= parent.size`
  clause and run gate 3(i)'s over-sized-child fixture. It goes green, which is the whole point: the
  size clause is the load-bearing one, the alignment clauses do not stand in for it, and the fixture
  proves the wild read is refused by *that* comparison and not by something adjacent.

*Second red:* ~~change `NestedCursor`'s `PhantomData<&'a ()>` to `PhantomData<*const ()>`.~~
**STRUCK: that mutation does not compile**, so it reds the library rather than gate 5 — measured
(scratch crate, `stable-x86_64-pc-windows-gnu` 1.97.1): `error[E0392]: lifetime parameter 'a is
never used`. → **Delete the `'a` parameter and the `_pd` field entirely**, reintroducing the bare
`{ptr, info}` cursor M2/O3 says is *"deleted and never introduced"*. That is the **unique** mutation
with the effect the rung describes: any `PhantomData` still naming `'a` (including the variance-only
`PhantomData<*const &'a ()>`) keeps the region constraint, so the borrow is still held and the
fixture keeps failing correctly. With the lifetime gone the cursor outlives the borrow, the
`compile_fail` fixture **compiles**, and `trybuild` reports *"expected compile error but compiled
successfully"* — the shape this campaign has been bitten by (a trybuild fixture red for 87 commits
because nobody re-blessed it). So gate 5 is run under `--no-fail-fast` and its `.stderr` is
re-blessed only with a stated reason.

**Why the runtime check and not the list.** The defect a refusal list is supposed to prevent is
**misnamed** when it is called "infinite descent". Give a container field a `Nested` descriptor and
the *first* step reads the container's own words as the child's inline fields; if the child
descriptor is larger than the field's extent, that read runs **past the end of the value** — a
memory-safety failure at **depth 1, before any recursion**. The infinite descent is the *second*
step. A memory-safety property is enforced next to its `unsafe`, in the crate that owns the
pointer arithmetic, which is why Check A lives in `validate` and not in a proc macro four rungs
downstream; and Check B refuses the descent itself, directly, without enumerating anything. The
derive-side list remains worth having — as a *diagnostic* (C9 gate 2), so the common cases fail at
the user's own token instead of at registration.

> ## ~~⛔ STOPPED~~ → **RESOLVED 2026-08-21 by the architect's C6 ruling.** The escalation and its evidence are kept below; the disposition is at the end
>
> **Disposition (read this first).** Both defects are dissolved, and the resolution **inverts** the
> escalation's own option (a): `boyko_reflect` carries the **primary** proof, and it is **not a
> list**. The refusal list is demoted to the derive-side early diagnostic. Options (b) and (c) are
> refused — (b) would run two rungs of descending value model with nothing proving the descent is
> even addressing-valid, and (c) is *unnecessary*, because the proof now lives where C6 already
> works, so the ladder stays `C6 → C7 → C8 → C9` and §5's backwards dependency disappears rather
> than being re-ordered around. The amendments are in **§3.1**, **§5**, **C6 Lands / gate 3 / gate 5
> / RED MUTATION**, **C9 gate 2**, and the new decision **D21**. The escalation was **right to
> refuse (a) as recorded** — a second list is a drift surface — and the recorded framing was the
> defect: Checks A and B enumerate nothing, so there is no list to keep in sync.
>
> C5 landed and is fully gated. **C6 was not started** *when this was written*. The rung as written
> contained one blocking defect and one mutation defect; both are of the classes this campaign has
> already paid for, and the blocking one is **not** mechanically resolvable inside CORE the way C4
> gate 5's was — it changes where a datum lives and adds an obligation to another rung, so it is an
> escalation, not an amendment. **The resolution was the orchestrator's / architect's call; nothing
> below decided it.**
>
> ### Defect 1 (BLOCKING) — gate 3 asserts a property of "the C9 refusal list", and that list cannot exist at C6
>
> Gate 3: *"a test that the C9 refusal list covers every indirection kind (`Box`, `Vec`, `&T`,
> `Option<Box<_>>`, raw pointers)"*. Its RED: *"Delete `Vec` from C9's refusal list …"*.
>
> **Evidence, measured on this worktree at `793d8d3a` + C5:**
> * `grep -rn "REFUSALS" crates/ docs/ tests/` → **7 hits, every one inside C9's own prose in this
>   file** (lines 318–319, 1369, 1380, 1382, 1384, 1392). **Zero occurrences in any source file.**
> * `grep -rn "reflect" crates/boyko_macros/src/` → 2 hits, both unrelated prose (`bundle.rs:274`
>   *"the registry layout reflects `size_of`"*, `lib.rs:504` *"reflection-free"*). The derive key
>   `#[component(reflect)]` does not exist and does not land until **C7**, as the binding context
>   for this session states.
> * §5's own dependency tree is `C6 → C7 → C8 → C9`. C9's Lands is *"In `boyko_macros`: every
>   rejection the derive must make"*, so C9 is strictly downstream of the derive, which is strictly
>   downstream of this rung. **There is no ordering in which C9's list exists when C6 runs.**
>
> **The plan contradicts itself about where the proof lives**, and this is the same shape as the
> §3.3-vs-C4-gate-5 contradiction corrected above. §3.1 says *"**C9's refusal test IS the
> acyclicity proof**"* — i.e. at C9. C6 gate 3 says the proof is a C6 test. §5 then writes *"C9 may
> land before C10/C11 and should: it is what makes C6's acyclicity proof true"* — which concedes
> the dependency runs backwards and still schedules C9 four rungs later. One of the three has to
> move.
>
> **Why this is not the C4 fix repeated.** C4's defect was *"which rung owns this arm"* — the
> instrument moved one rung earlier, no API changed, no other rung gained an obligation. Here the
> only ways to make gate 3 runnable at C6 are:
> * **(a)** land the refusal set as a datum in `boyko_reflect` at C6 — e.g. a
>   `pub const INDIRECTION_KINDS_REFUSED: &[&str]` that C6's test asserts covers the five kinds —
>   and make C9's census assert `REFUSALS ⊇` it. This creates a **new cross-rung obligation** and a
>   second list that can drift from the first; the campaign's own "dead datum" ledger is five
>   entries long and this shape is how three of them started.
> * **(b)** move gate 3 (and its RED) to **C9**, where §3.1 already says the proof lives, and give
>   C6 an explicit note that its acyclicity rests on a gate that lands later — an honest but
>   *weaker* v1, since C7/C8 would then descend with the proof outstanding.
> * **(c)** re-order the ladder so a refusal census precedes the descend. This is a ladder change,
>   not a rung change.
>
> Landing C6 with gate 3 silently skipped is the one option that is **not** available: a rung whose
> gate nothing runs is this campaign's most repeated defect, and gate 3's own text says *"there is
> no runtime guard, so this test is the only thing standing between v1 and an infinite descend"*.
>
> ### Defect 2 (mutation) — the second RED does not compile, so it reds the library rather than gate 5
>
> *"change `NestedCursor`'s `PhantomData<&'a ()>` to `PhantomData<*const ()>`"* leaves `'a`
> referenced by nothing. **Measured** (scratch crate, `stable-x86_64-pc-windows-gnu` 1.97.1):
>
> ```text
> error[E0392]: lifetime parameter `'a` is never used
>  --> src\lib.rs:2:25
>   | pub struct NestedCursor<'a> {
>   |                         ^^ unused lifetime parameter
> ```
>
> So the mutation cannot be run as written: it is a compile error in `boyko_reflect`, not a
> `compile_fail` fixture that stopped failing. **The mutation whose *effect* the rung describes is
> deleting the lifetime entirely** — reintroducing the bare `{ptr, info}` cursor that M2/O3 says is
> *"deleted and never introduced"* — which is exactly the defect gate 5 exists to catch, and which
> does make trybuild report *"expected compile error but compiled successfully"*. A variance-only
> change (`PhantomData<*const &'a ()>`) is **not** a substitute: `'a` stays used, so the borrow is
> still held and gate 5 still fails correctly.
>
> ### What was verified as RUNNABLE, so the resolution does not have to re-check it
>
> * **Gate 1, fixture half** — buildable today: hand-baked `TypeInfo` statics are what C3 shipped
>   and `tests/c3_type_info.rs` already carries a depth-1 `Nested` fixture (`Inner { x: f32, y: f32 }`)
>   to extend to depth 2.
> * **Gate 1, dogfood half** — all four engine types exist and have the shapes the rung assumes:
>   `Transform { translation: Vec3, rotation: Quat, scale: Vec3 }`
>   (`crates/boyko_scene/src/transform.rs:46`, layout-pinned at 40 B),
>   `Vec3 { x: f32, y: f32, z: f32 }` (`crates/boyko_math/src/vec.rs:145`),
>   `Name(pub NameId)` / `NameId(pub u32)` (`crates/boyko_scene/src/identity.rs:56`, `:47`, both
>   `#[repr(transparent)]`, both layout-pinned). `Transform → Vec3 → f32` and `Name → NameId → u32`
>   are both genuine depth-2 descends, and the second is the tuple-struct case in the bargain.
>   *Note for whoever builds it:* with no derive until C7 these statics are hand-baked in
>   `reflect_dogfood`, so `boyko_scene`'s `reflect` feature gates the **dependency edge** and not
>   yet any `#[cfg]`'d body — the feature is real, the gating is not load-bearing until C7.
> * **Gate 2** — the C4/C5 instrument takes it: `tests/c4_prim_zero_alloc.rs` already measures
>   `enumerate` and `array read` at delta 0 and carries the positive control.
> * **Gate 4** — `-p boyko-reflect` is already on the Miri allowlist and green (§7.2's first row).
> * **Gate 5** — needs `trybuild = "1"` as a **dev**-dependency of whichever package hosts the
>   corpus; `boyko-reflect` has none today (`proptest` is its only dev-dep). D18 governs the
>   ship-visible table only, and the manifest already says so, so this is additive — but it is a
>   manifest change and **G1's census must be re-run**, which it is not today for a new dev-dep.

> **Executed 2026-08-21 (worktree `D:/wt/reflect`, toolchain `stable-x86_64-pc-windows-gnu` 1.97.1),
> on the amended rung.**
>
> *Lands.* `crates/boyko_reflect/src/cursor.rs` — `NestedCursor<'a>` (`Copy`, re-rootable,
> `type_info()` / `fields()` / `get()` / `descend()` / `value()`) and `FieldValue<'a>`;
> `crates/boyko_reflect/src/type_info.rs` — Check A in the existing `ValueKind::Nested` arm and
> Check B as a fixed-array DFS; `Cargo.toml` — `trybuild = "1"` as a dev-dependency with its reason
> at the manifest. Tests: `tests/c6_nested.rs` (gate 3 + the cursor), `tests/c6_compile_fail.rs` +
> two fixtures (gate 5), the `nested descend` arm appended to `tests/c4_prim_zero_alloc.rs` (gate 2),
> `crates/reflect_fixture/tests/c6_nested_descend.rs` and
> `crates/reflect_dogfood/tests/c6_dogfood_descend.rs` (gate 1's two halves).
>
> **The architect's caveat, checked first — he had read `type_info.rs`'s model but not `validate`'s
> body.** Three findings, and one of them corrects the ruling's own vocabulary:
>
> 1. **The enum is `Violation`, not `Problem`.** `Problem` is the *located* struct
>    `{ field_index, name, violation }`; `Violation` is the 14-arm rule enum. "Two new `Problem`
>    arms" is "two new `Violation` arms", and the plan text above now says so.
> 2. **The two arms land with zero call-site churn.** The *"no wildcard arm"* match is
>    `match field.kind` over `ValueKind` — adding `Violation` arms does not touch it — and **no
>    `match` over `Violation` exists anywhere in the tree** (`Display` prints it with `{:?}`;
>    `c3_type_info.rs` only compares arms for equality). Measured by grep before the first edit.
> 3. **`TypeInfo` carries `size` AND `align`** (`crates/boyko_reflect/src/type_info.rs:241`, `:243`
>    after this rung's edits), so Check A keeps all three clauses; the contingency the caveat named
>    — *"if align is absent, Check A drops its two alignment clauses"* — did not arise.
>
> *Five things decided here and recorded rather than worked around:*
>
> 1. **THREE new arms, not two — `NestedGraphTooLarge` is Check B's capacity refusal.** A *fixed
>    on-stack array* has a capacity, and a walk that runs out of it has produced a **partial**
>    acyclicity proof, which is not one. The third arm is where that is said out loud instead of
>    silently returning `Ok`. Bounds: `MAX_NESTED_DEPTH = 32` (the path) and `MAX_NESTED_TYPES = 256`
>    (the finished set, which is what keeps the walk linear in edges rather than exponential on a
>    diamond). Both branches have their own test, so the arm is not a sixth *dead datum*.
> 2. **The path array is the cycle test; the finished set is only a memo — and that distinction is
>    the whole check.** A plain global visited set would skip the re-entry into `A` in `A → B → A`
>    and report **no cycle at all**, terminating quietly while proving nothing. Written into
>    `NestedWalk`'s doc comment because it is the single most inviting way to "simplify" this code
>    into a vacuous green.
> 3. **Check A is checked on the validated descriptor's own edges, not on every node the walk
>    visits.** `Problem::field_index` names *this* descriptor's fields, and re-using it for a
>    grandchild's field index would quietly change what a `Problem` means. A cursor rooted at a
>    nested type therefore rests on that type's own `validate` — the same per-descriptor discipline
>    every other rule already uses — and `validate`'s doc comment states it as the precondition
>    `NestedCursor::new` names, rather than leaving it implied.
> 4. **`NestedCursor::new` takes `&'a T`, not a raw pointer**, for two independent reasons that only
>    became visible while building gate 5. A raw-pointer constructor leaves `'a` *unconstrained*, so
>    the caller may pick `'static` and both `compile_fail` fixtures compile — gate 5 would be
>    ornamental. And `&'a T` carries provenance over the **whole** value: a `&u8` to its first byte
>    would make the very first `add(offset)` a Tree-Borrows violation, which Miri would have caught
>    and which no amount of `unsafe` documentation would have fixed.
> 5. **`FieldValue` has exactly two arms** (`Prim`, `Nested`) — the kinds with a reader at this rung.
>    `Array`/`Enum`/`Str`/`Opaque` answer `None` and gain their arms in the commits that give them
>    something to call (D9's rule, applied to a variant rather than a field).
>
> *One refinement of the ruling's reasoning, found by building its fixture, and it makes the rule
> sharper rather than weaker.* The ruling says a `Vec<u8>` emitted as `Nested` means *"the descent
> reads the Vec's `{ptr,cap,len}` words AS the child's inline fields — a WILD READ at depth 1"*. The
> first half is exactly right; the second is only true **when the child descriptor does not fit the
> field's extent**. A `Vec` described as a 24-byte inline triple reads its own three words —
> in-bounds *garbage*, not a wild read. The out-of-bounds read appears the moment the mis-described
> child is **larger** than the space the field occupies, which is precisely
> `offset + child.size > parent.size` — the clause Check A leads with, and the one C6's second RED
> deletes. The gate-3(i) fixture is built on the real property rather than on the illustration: a
> `Leaky { data: Vec<f32> }` (24 B) whose `data` is described as a 40-byte `Wide`, and the test
> **prints the overrun** — *"field `data` at offset 0 claims a 40-byte child inside a 24-byte value
> — the first descend would read 16 bytes past the end"*. No check changed; the sentence that
> justifies it did.
>
> *Gate 1, fixture half:* `-p reflect-fixture --features reflect-fixture/reflect` — a depth-2
> named-field nest (`Body → Placement → Point → f32`, leaf read at depth 2) and a depth-2 tuple
> struct (`Slot → Handle → u32`), plus a coherence precondition test and a descriptor-vs-real-type
> pin, **4 tests**. *Gate 1, dogfood half:* `-p reflect-dogfood --features reflect-dogfood/reflect`
> over the real `Transform → Vec3 → f32` and `Name → NameId → u32`, **4 tests**. `Transform` turned
> out to be the first fixture in this campaign where the **finished set** earns its keep:
> `translation` and `scale` both point at one `VEC3_TYPE_INFO`, so the walk meets that node twice
> and must answer *"already proved"* rather than *"cycle"* — asserted, not assumed. **The dogfood
> half needed one manifest edge**: `boyko-math`, because it must bake `Vec3`/`Quat`'s **real**
> `offset_of!` / `size_of` / `align_of` and `boyko_scene` does not re-export them
> (`transform.rs:33` imports them from there). Plain, non-optional, no `features` array — the rule
> that manifest's own comment states.
>
> *Gate 2, MEASURED:* `nested descend (depth 2): baseline=0 measured=0 delta=0` over 1000 walks, on
> the C4 instrument, in the same binary as the permanent positive control
> (`deliberate allocations observed = 1`). Depth **2**, as §3.3's row demands.
>
> *Gate 3:* `a_nested_child_too_large_for_its_field_is_refused` (3(i)) asserts **exactly one**
> `Problem`, so the rung's RED can green it by deleting exactly one clause;
> `a_misaligned_or_over_aligned_nested_child_is_refused` isolates Check A's other two clauses on
> their own single-violation fixtures; `a_cyclic_type_info_graph_is_refused` (3(ii)) covers both a
> two-node ring and a self-naming descriptor and asserts the `Problem` names **the edge that closes
> the cycle**; and `check_a_is_satisfied_at_every_edge_of_the_cycle` **measures** the ruling's
> load-bearing claim — the cyclic fixtures produce **no** `NestedNotInline` at any node, so Check A's
> blindness to a hand-baked cycle is a recorded fact rather than a conceded argument. Both
> `A → B → A` and the direct self-reference compile as ordinary `static`s; rustc accepts the forward
> reference, which was checked rather than assumed.
>
> *Gate 4:* `cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-reflect --all-targets` (TB) —
> 4 + 8 + 0 + 13 + 7 + 0 + 8 + 0 + **12** green, exit 0; the new `descend`/`get` pointer arithmetic
> is covered. The two capacity tests carry `#[cfg_attr(miri, ignore = …)]` with a stated reason —
> they build their fixtures with `Box::leak` (a 33-deep chain of hand-written statics is 66
> declarations, and 257 leaves is not writable at all) and Miri's leak checker reports the
> deliberate leak. **No `unsafe` is in either**, and the Miri-relevant paths — the cursor and the
> static cyclic fixtures — do run. `-p reflect-fixture --features reflect-fixture/reflect` under
> Miri: 4 + 1 green, exit 0.
>
> *Gate 5:* two fixtures, both blessed against `rustc 1.97.1 (8bab26f4f 2026-07-14)` — the freeze
> `tests/trybuild_corpus_compiler_witness.rs` already pins. `cursor_held_across_mut` reds with
> **E0506** (*"cannot assign to `value.inner.x` because it is borrowed"*) and `cursor_outlives_value`
> with **E0515** (*"cannot return value referencing local variable"*): aliasing and lifetime, the two
> halves of what the `'a` buys. The first fixture reads the field again after the borrow ends, so its
> blessed bytes carry the borrow error and nothing else — an `unused_assignments` warning was in the
> first bless and was designed out rather than frozen in.
>
> **RED LEDGER — four mutations run, and one of them refuted this rung's own prediction.**
>
> * **RED 1b (Check A's size clause), one edit at `is_inline_contained`, exit 101, exactly one test
>   red:** *"a 40-byte child inside a 24-byte value is not inline-contained: ()"* — `validate`
>   returned `Ok`. The alignment fixtures stayed **green**, which is the measurement the split
>   fixtures exist for: `offset + child.size <= parent.size` is separately load-bearing, and the two
>   alignment clauses do not stand in for it.
> * **RED 1a (Check B's path comparison), exit 101, one test red — AND IT REFUTES THE RUNG'S
>   PREDICTED FAILURE MODE.** The rung says removing Check B reds *"by stack/array overflow or by
>   hanging"*. Observed instead: `expected a named NestedCycle, got: [Problem { field_index: None,
>   name: "c6_nested::RingA", violation: NestedGraphTooLarge }]`. The depth capacity is a **second**
>   bound, so the cycle is caught — by the **wrong rule**, with a `Problem` that says "this graph is
>   too big" about a two-node graph. That is a strictly worse diagnostic and a strictly weaker claim,
>   and it is exactly the shape a maintainer accepts as "still refused". Recorded because a
>   prediction that is off is worth more written down than quietly corrected.
> * **RED 1a-bis (Check B deleted in FULL — path array, depth guard and comparison), run to show the
>   failure the rung actually describes:** the test process dies with
>   `exit code: 0xc00000fd, STATUS_STACK_OVERFLOW`. Note what survives in this state: the `done`
>   memo. A finished-set-only walk does **not** terminate on `A → B → A`, because neither node is
>   finished while the other is being walked — the concrete form of the "a plain visited set is not
>   a cycle test" note in `NestedWalk`.
> * **RED 2 (delete the `'a` parameter and the `_pd` field entirely — the bare `{ptr, info}` cursor
>   M2/O3 forbids), exit 101:** `cursor_held_across_mut` → *"Expected test case to fail to compile,
>   but it succeeded."*; `cursor_outlives_value` → `.stderr` mismatch, **E0515 → E0107** (*"struct
>   takes 0 lifetime arguments but 1 lifetime argument was supplied"*). Run as
>   `--test c6_compile_fail`, because the mutation **also** stops `tests/c4_prim_zero_alloc.rs` from
>   compiling — that harness names `NestedCursor<'a>` in its fn-pointer types, so the deletion is
>   refuted in three places at once; only the targeted invocation isolates gate 5's own signal.
> * **The variance-only alternative was RUN, not argued.** `PhantomData<&'a ()>` →
>   `PhantomData<*const &'a ()>`: gate 5 **exit 0**, both fixtures still failing to compile with the
>   blessed bytes. `'a` stays used, the region constraint stays, and the borrow is still held — so
>   the full deletion really is the unique mutation with the described effect, measured rather than
>   asserted.
> * **RED for the new alloc arm** (`let _red = Vec::<u8>::with_capacity(1);` inside `descend`), exit
>   101: *"nested descend (depth 2): baseline=0 measured=**2000** delta=2000"* — exactly **two** per
>   walk, no noise, and the `prim`, `enumerate` and `array read` arms stayed green in the same run.
>   Two, not one, is the depth-≥2 claim made visible: §3.3's row says *per level*, and the counter
>   counts per level.
>
> *Restoration:* `type_info.rs` and `cursor.rs` restored from file copies after every mutation and
> verified byte-identical by sha256 (`type_info.rs` `53de5595…da4a`, `cursor.rs` `508859ad…5b9f`).
>
> *Gate.* `cargo check -p boyko-reflect --all-targets` exit 0; `cargo clippy -p boyko-reflect
> --all-targets -- -D warnings` **touch-first, both profiles**, exit 0 — it caught one real thing,
> `offset % child.align == 0` → `offset.is_multiple_of(child.align)`, which also strengthened the
> `align == 0` guard's rationale (`is_multiple_of(0)` answers `offset == 0`, so a descriptor claiming
> `align: 0` would have *passed* the alignment clause at offset 0). `cargo test -p boyko-reflect
> --all-targets --no-fail-fast`: debug 4 + 19 + 1 + 13 + 7 + 9 + 8 + 1 + 14 = **76** across nine
> binaries, release 5 + 19 + 1 + 13 + 7 + 9 + 8 + 1 + 14 = **77**, exit 0 both. `-p reflect-fixture`
> feature-**off** (the ship configuration) exit 0 with `c6_nested_descend` compiling to 0 tests **by
> design**, feature-**on** 4 + 1 + 1 exit 0; `-p reflect-dogfood --features reflect-dogfood/reflect`
> 4, exit 0. Clippy `-D warnings` on both consumer packages with the feature on: exit 0.
>
> *Regression, all exit 0:* G0 census 3, **G1 manifest 7 — re-run for the two manifest edits and
> green, which is gate 5's own clause**, G2 closure 2, G3 absence 1 (+1 ignored calibration), G4
> coverage 6, leg-nonvacuity 1, `internal_docs_anchors` 5, `trybuild_corpus_compiler_witness` 2,
> `engine_packages_census` 3, `goldens_pins_wellformed` 7, `gpu_blocking_reader_census` 2,
> `vg_symbol_reachability` 16.

---

### C7 — `#[component(reflect)]`: the field walk and `offset_of!` baking (no install yet)

**Lands.** In `boyko_macros`: the `reflect` flag key on `#[component(…)]` (F9's mechanism, one
match arm), and the emission — behind `#[cfg(feature = "reflect")]` (D2) — of

* `impl boyko_reflect::Reflect for T { const TYPE_INFO: &'static TypeInfo = &T::__REFLECT_TYPE_INFO; }`
* a `static __REFLECT_TYPE_INFO: TypeInfo` with `fields` a baked `&'static [FieldInfo]`, every
  `offset` a `core::mem::offset_of!(T, f)` (F12 — the exact idiom already load-bearing 317 times
  on this toolchain),
* `default_in_place` / `drop_in_place` from `Default` / `needs_drop` — **`default_in_place` is
  `Option`, and its `Some` arm carries a named bound assertion, D20.** The derive emits
  `Some(__default_in_place::<T>)` plus the `const _: fn() = || { fn __assert_reflect_default<T:
  ReflectDefault>() {} __assert_reflect_default::<T>(); };` witness spanned at the type name, so a
  type with no `Default` fails with `ReflectDefault`'s `#[diagnostic::on_unimplemented]` message
  rather than with an `E0277` pointing into generated code. `#[reflect(no_default)]` emits `None`
  instead, and emits no witness.

**Tuple structs:** `FieldInfo.name` is `"0"`, `"1"`, … . **For tuple structs by-name == by-position**,
so the reorder stability the design advertises does **not** hold for them — stated in the derive's
diagnostic and in the type's docs. (Named-field structs are recommended for anything serialized;
the wire consequence is BOUNDARY's.)

**No install call is emitted at this rung.** The static exists and is inert; C8 wires it. Splitting
them keeps "the derive computes the right offsets" separable from "the funnel appends correctly",
which are two different failures.

**Gate.**
1. **Derived == hand-baked.** For the C3 fixture types, `T::TYPE_INFO` compares field-for-field
   equal to C3's hand-written static — names, offsets, kinds, accessor presence. The hand-baked
   side is the oracle and it is independently pinned by C3 gate 4.
2. `(TYPE_INFO.type_id_fn)() == TypeId::of::<T>()`; `size`/`align` == `size_of`/`align_of`.
3. Reorder two fields in a fixture ⇒ the derived offsets move accordingly (a `#[repr(Rust)]` type,
   so the assertion is "offsets are a permutation consistent with `offset_of!`", not fixed
   numbers).
4. `default_in_place` drop-count test over `{ pod, Nested{pod}, [f32;4] }` (A.8): writes into
   **uninitialized** bytes only; no leak, no double-free; empty struct (`fields: &[]`) is a no-op,
   not a panic.
5. Alloc-delta arm for `default_in_place` = 0 bespoke.
6. Tuple-struct fixture: names are `"0"`,`"1"`, over a locally-declared `#[repr(transparent)]`
   tuple struct in `reflect_fixture`. `ParticleEffectHandle(pub u32)` is the production instance of
   the shape (hook-bearing, `boyko_render`) — it cannot be this gate's subject, because the fixture
   package must stay FFI-free (§0.3) and `boyko_render` reaches `boyko_rhi_vulkan`; it is exercised
   at ECS EG8, in `reflect_dogfood`.

**RED MUTATION.** Make the derive bake `0` for every offset. Gate 1 reds on every non-first field,
and gate 3 reds. *Second red:* make the derive emit `drop_in_place: None` for a type that owns a
`String`. Gate 4 reds on the drop count — which is the mutation a naive "all POD" assumption would
actually produce.

---

### C8 — The install seam: the seventh slot in `component_id()`

**Lands.** In `boyko_macros`, beside the six existing install slots (F7):

```rust
#reflect_install     // emitted iff #[component(reflect)]; wrapped in #[cfg(feature = "reflect")]
```

expanding to `boyko_reflect::install_type_info(raw, <Self as boyko_reflect::Reflect>::TYPE_INFO);`.
No `IS_REFLECT` const (D7). `boyko_macros` gains **no dependency** on `boyko_reflect` (D17).

**Gate.**
1. **Feature ON:** `type_info_of(T::component_id().0)` is `Some` on the **first** touch of
   `component_id()`, before `T` can enter any archetype.
2. **Feature OFF, in a crate that does not have the dep at all:** `#[component(reflect)]`
   compiles. This is the whole D1/D2 mechanism and it is a compile, not an argument.
3. **Token absence, feature OFF** — **the instrument is
   [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)'s G6b, and this gate defers to it rather
   than specifying its own.** The claim is unchanged: with the feature off, the expansion of a
   `#[component(reflect)]` type contains **zero** occurrences of the string `boyko_reflect`.
   > **Why the deferral, 2026-08-21 (second pass).** *"Measured, not asserted — the rung records the
   > count"* names a measurement of a macro **expansion**, and the only route to one is
   > `-Zunpretty=expanded`, which is **nightly** — while this campaign's mandated toolchain is
   > `stable-x86_64-pc-windows-gnu` 1.97.1 and its recorded hazard is a *shadowing* rustc, not a
   > missing one. G6b names this problem explicitly and offers a stable substitute with a stated
   > selection criterion — *whichever form has a RED a person can run on this toolchain without
   > nightly* — which is a criterion this gate had no business restating differently. **C8's RED
   > survives the substitution**: under G6b's `compile_fail` form the same mutation makes the fixture
   > *compile*, which is the same red through a different instrument. Record G6b's chosen form in the
   > GATES ledger; this gate asserts whatever G6b selected.
4. **Bitset suppression:** `#[component(reflect, storage = "bitset")]` is a **derive error**, not
   a silent skip (C9 owns the message; this gate owns "the install is not emitted").
5. **Horn-2 drift test (D16):** for every type carrying both `#[derive(Bindable)]` and
   `#[component(reflect)]`, `<T as Bindable>::field_id(name)` and the reflect field index
   **agree**, for every field name. Feature-on only.
6. The `component_id()` funnel's existing six slots are unperturbed: a type **without**
   `#[component(reflect)]` expands byte-identically to its pre-rung expansion.

**RED MUTATION.** Remove the `#[cfg(feature = "reflect")]` wrapper from the emitted install. Gate
2 reds with `E0433` (unresolved crate `boyko_reflect`) in the no-dep crate, and gate 3 reds in
whichever form G6b selected (the count going 0 → non-zero, or the `compile_fail` fixture starting to
compile). **Both reds are required**: gate 3 alone would be satisfied by a build that
happens to have the dep in the graph, which F16/F19 make the *default* at the workspace root.

*Second red:* rename a field in a `Bindable + reflect` fixture on one side only. Gate 5 reds —
which is the drift Horn 2 buys and the reason the test is the price of taking it.

---

### C9 — The refusal matrix, spanned, with an anti-rot census

**Lands.** Every rejection the derive must make, each a `compile_error!` **spanned at the offending
token** (never at the derive attribute):

| refused | reason | span |
|---|---|---|
| generic type parameters | a per-impl `static TYPE_INFO` collapses across monomorphizations — the documented Bundle / Phase-12.5 `static SLOT` / Phase-17 `State<S>` trap | the generic param |
| `#[repr(packed)]` | taking `&field` on a packed type is UB; the `*_unaligned` ops are v2 | the `repr` attribute |
| `storage = "bitset"` | no per-row bytes exist — the bit *is* the datum, so "read field at offset" is meaningless | the `storage` key |
| an `Opaque` field without `#[reflect(skip)]` | D15 — the wire is shared with the shipped `boyko_serialize`; silent omission is unacceptable | **the field** |
| a fieldless enum with no `#[repr(Int)]` | no guaranteed discriminant width (FIX Mi3) — a silent `Opaque` would be worse | the enum item |
| a data-carrying enum, `Option<T>`, `Vec`, `Map`, `Box`, `&T`, raw pointers | v2 kinds; `Option<T>` is the *smallest* data-carrying enum and niche optimization means **no** guaranteed discriminant location, so it inherits the full data-enum hazard and is **not** "cheap enough" | the field |
| **a type with no `Default` and no `#[reflect(no_default)]`** — **D20** | `default_in_place`'s `Some` arm is baked from `Default`, and an inspector's "Add Component" needs it. **This row is not a `compile_error!` and cannot be**: a proc macro cannot see trait impls, so the refusal is a trait bound carrying `#[diagnostic::on_unimplemented]`, which is this tree's existing answer for that class (`query/chunked_data.rs:67`, `query/filter.rs:2507`, with a blessed fixture at `tests/compile_fail_chunk/changed_filter_rejected.rs:11`) | the **type name**, via the `const _: fn() = …` witness the derive emits |

**`storage = "dense"` is NOT refused** — a dense component has real per-row bytes at a stable
address, and it is the one non-table kind that is fully readable. Its *enumeration* problem is
[`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md)'s (analysis B.3); the derive has no reason to
refuse it and refusing it would make the design decline the one flagship component it can fully
read.

**Diagnostics quality is a deliverable, not a nicety.** Aether emits `#[derive(Component)]` items
the user never wrote and can already produce `#[component(storage = "bitset")]` from
`tag Foo(bitset);` — so a user who typed three words would otherwise get an error about a derive
they never typed. Aether solves this class with `quote_spanned! { name.span() => … }` and has a
recorded measurement of what happens without it (*"rustc's 'previous definition of the type `Foo`
here' pointed at `aether! {`"*). Refusals stay spanned at the user's own token.

**Gate.**
1. A `trybuild` corpus, one case per row, with blessed `.stderr` — the established harness
   (`crates/boyko_ecs/tests/compile_fail_*.rs`), `#[cfg(not(miri))]` like its siblings.
2. **Anti-rot census:** a test asserting that the number of `.stderr` fixtures **equals** the
   number of refusal rules enumerated in the derive source (a `const REFUSALS: &[&str]` the derive
   itself iterates). A rule added without a fixture reds; a fixture deleted reds.
   **Re-scoped 2026-08-21 (architect's C6 ruling, D21): this census covers DIAGNOSTIC quality, not
   termination.** It asserts the derive names the five standard indirection kinds (`Box`, `Vec`,
   `&T`, `Option<Box<_>>`, raw pointers) so a user who writes one gets a **spanned compile error at
   the field** rather than a registration-time `Problem`. It is **not** the acyclicity proof and
   never was — that is `validate`'s `NestedCycle` arm, with `NestedNotInline` alongside it for
   addressing-validity, both landing at **C6** (§3.1). The consequence of a missing kind is
   therefore a **worse diagnostic**, not an unsound descend: the descend is refused by C6's checks
   either way. D20's item 2 (`REFUSALS` counts `missing_default_rejected`, so the hidden `T: Default`
   bound stays census-visible) is unaffected by this re-scoping.
3. The corpus is run under `--no-fail-fast`. **`cargo test` stops at the first failing target**,
   so one known-red target shadows every target behind it — this repo has measured a trybuild
   fixture staying red for **87 commits** because a line was added and its `.stderr` was never
   re-blessed, invisible until the flag was passed.
4. The dense case is a **positive** control: `#[component(reflect, storage = "dense")]` compiles
   and installs.
5. **The two D20 fixtures, and the second is a `t.pass()`:** `missing_default_rejected` (a type with
   no `Default` — its `.stderr` pins `ReflectDefault`'s `on_unimplemented` *message*, not rustc's
   generic E0277 text, which is the whole point of the row) and `no_default_accepted`
   (`#[reflect(no_default)]` compiles, `TYPE_INFO.default_in_place.is_none()`). **`REFUSALS` counts
   the first**, so gate 2's census sees the rule — the defect D20 exists to close was precisely that
   a hidden `T: Default` bound is structurally invisible to a census keyed on `REFUSALS`.

**RED MUTATION.** Add a refusal rule to `REFUSALS` without adding its fixture. Gate 2
reds. *Second red:* change one refusal's `quote_spanned!` to plain `quote!`. The `.stderr` no
longer matches (the caret moves to the derive) and gate 1 reds — which is how span quality becomes
gate-visible instead of aspirational.

*Third red, D20's own, and it is the one that shows the census could not have seen this before:*
delete `#[diagnostic::on_unimplemented]` from `ReflectDefault`. Gate 5's first fixture reds because
its `.stderr` now carries rustc's generic *"the trait bound `Foo: Default` is not satisfied"* pointing
into the expansion. **Then delete the `REFUSALS` row too** and confirm gate 2 goes back to green with
the rule gone — that green is the state this plan shipped in before D20, and seeing it is how the row
is believed.

---

### C10 — Enums: `TypeKind::Enum` at the top level **and** `ValueKind::Enum` in a field

**Lands.** `EnumInfo` / `VariantInfo` baking in the derive; `get_discr` / `set_enum_variant_index`;
`FieldValue::Enum { discr, info }`. Two shapes, both required:

* **top level** — a component that **is** an enum has no fields, so `fields_of` returns `&'static
  []` and the value is reached through `TypeKind::Enum` + type-level discriminant accessors.
  Dogfood: **`Visibility`** (`boyko_scene/src/render_caps.rs:226`) — a Component, fieldless
  `#[repr(u8)]`, discriminants pinned *"so the byte is stable across serialization"*, and "toggle
  a node's visibility" is the canonical inspector action.
* **field level** — 8 in-tree consumers (`UiAlign.main`, `UiAlign.cross`, `UiLayout.layout_type`,
  `UiLayout.position_type`, `UiAnchor.edge`, `UiText.align`, `UiWorldAnchor.scale_mode`,
  `BindText.template`), plus `Interaction` and `FocusPolicy` as top-level enum components.

`set_enum_variant_index` does a **release** bounds check (`idx < variants.len()` → else `false`,
**not** a `debug_assert!`), and only a **baked variant discriminant** is ever written — every
fieldless `#[repr(Int)]` variant value is a valid inhabitant, so there is no invalid-value UB.

**Gate.**
1. **Two packages again** (C6's split, same reason — `Visibility` is `boyko_scene`'s and the Miri row
   must stay FFI-free):
   * **In `reflect_fixture`:** a locally-declared fieldless `#[repr(u8)]` enum with *pinned*
     discriminants round-trips every variant through `get_discr`/`set_enum_variant_index`, and the
     gate asserts the **byte** written, not the variant.
   * **In `reflect_dogfood`:** the same over the real `Visibility`
     (`boyko_scene/src/render_caps.rs:226`), asserting the pinned `Inherited=0, Visible=1, Hidden=2`
     — the pin exists *for serialization stability*, so reading the byte is reading the thing the pin
     protects.
2. `set_enum_variant_index(len)` and `(usize::MAX)` return `false` and change no bytes, in a
   **release** test.
3. Sign extension: an `#[repr(i8)]` enum with a negative discriminant round-trips.
4. Alloc-delta arm = 0.
5. `VariantInfo.discr_bits` is narrowed to the repr width at bake time — asserted by comparing
   against `discr as <repr> as u64` for an `i8` enum with a negative variant, where a lossy
   `i128 as u64` at the call site would differ.

**RED MUTATION.** Make the derive bake `discr_bits` as `i128 as u64` at the *call site* instead of
narrowing at bake time. Gate 5 reds on the negative `i8` variant only — so the fixture **must**
contain one, and the test name says which variant carries the proof.

*Second red:* replace gate 2's release check with `debug_assert!`. The release test reds by
writing an out-of-range byte into the enum's storage — an invalid inhabitant, which is UB, and
therefore also a Miri red.

---

### C11 — `Str`: the `String` arm, built LAST

**Lands.** `ValueKind::Str`; `get_str` returning `&'a str` **borrowed** from the live buffer
(zero alloc); `set_str` performing **raw `ptr::drop_in_place(p as *mut String)` then `ptr::write(p
as *mut String, s.to_owned())`** on the original arena `*mut` provenance — it **never forms an
intermediate `&mut String`**. That sidesteps the `Unique` retag through the arena's deliberately
`SharedReadWrite` interior-mutable provenance, which is the 14a-F2 / Phase-19 TB-UB class. The
earlier `*slot = s.to_owned()` form is **rejected**, and its pre-declared "TB is avoided" verdict
was struck by the analysis's own critique round.

**Why last (D13):** there is **no `String` consumer in this tree** — every `#[derive(Component)]`
struct under `crates/*/src` was walked and **zero** have a `String`, `Box<str>` or `&str` field.
The engine deliberately went the other way (`Name` holds a `u32` into a leaking process-global
interner; "a component needs text" is `UiName { bytes: [u8; CAP], len: u8 }`). So this arm is a
**forward-looking obligation for future user components**, and its fixture is local
(`struct StrFixture { s: String }`) until a production consumer exists.

**Gate.**
1. Read: `get_str` borrows; alloc-delta = **0**. The soundness reason is the **lifetime**, not
   "the buffer lives as long as the component": the shared `&EcsMaster` borrow `'a` statically
   excludes any `&mut` op (including `set_str`) for `'a`. *(The wrong reason would also "justify"
   a UAF across a `set_str`, which is why it is written down.)*
2. Write: alloc-delta is **exactly 1 alloc + 1 free** — the count, not the adjective.
3. **Miri under `-Zmiri-tree-borrows` is the gate, not an argument.** This project's hard-won
   lesson is that critics approve TB-UB and only Miri is the oracle.
   > ⚠️ **And the row that carries it is the FIXTURE's, not this crate's — corrected 2026-08-21
   > (second pass).** `set_str` is reached through a derive-generated `FieldInfo`, so the Miri run
   > that exercises it is
   > `cargo +nightly miri test --all-targets -p reflect-fixture --features reflect-fixture/reflect`.
   > A plain `-p boyko-reflect` row covers this module's arithmetic and the accessor called on a
   > hand-built pointer — which is *most* of what this rung needs — but the `StrFixture { s: String }`
   > **component** can only exist in a consumer. GATES **D4/G4** own both rows;
   > `-p boyko-reflect --features reflect` is a cargo error and must not be written.
4. `compile_fail`: the returned `&str` cannot outlive the borrow.

**RED MUTATION.** Replace the raw `drop_in_place` + `ptr::write` with `*(p as *mut String) =
s.to_owned();`. Gates 1–2 stay **green** (the allocation accounting is identical) and **only Miri
reds**. That is the entire point of gate 3 and the reason it is mandatory rather than
recommended — and it is why the Miri-allowlist dependency (F18) must have landed by now, **in both
its rows**, or this rung's headline gate is a gate that cannot fail. *Run this mutation in the
fixture*: run in `boyko_reflect`'s own tests it would exercise a hand-built pointer rather than the
derive's, which is a weaker subject wearing the same verdict.

*Second red:* drop the `drop_in_place` and keep only the `ptr::write`. Gate 2 reds (1 alloc, **0**
frees — a leak), which is the accounting failure the raw form is specifically accused of and must
therefore be shown detectable.

---

## 5. Rung dependency order

```
(G0 → G1 → G2 → G3 → G4  — GATES: the three packages, the manifest census, the resolver
                            gate, the artifact census, and the CI legs. See GATES' G0
                            preamble, which owns the campaign-wide order.)
C0  the red canary: prove the leg G4 installed can fail
 ├─ C1  Scalar
 │   └─ C2  registry ────────────────┐
 │        └─ C3  TypeInfo/FieldInfo ─┼─ C4  prim:: + release kind check
 │                                   │       └─ C5  Array + alloc harness
 │                                   │            └─ C6  Nested (read)
 │                                   │                 └─ C7  derive: field walk + offset baking
 │                                   │                      └─ C8  install seam (7th slot)
 │                                   │                           ├─ C9  refusal matrix + census
 │                                   │                           ├─ C10 enums (both shapes)
 │                                   │                           └─ C11 Str  (LAST — D13)
```

~~C9 may land before C10/C11 and should: it is what makes C6's acyclicity proof true (§3.1).~~
**DELETED 2026-08-21** (architect's C6 ruling, D21). C9 makes nothing about C6 true: the acyclicity
proof is `validate`'s `NestedCycle` arm and the addressing-validity proof is its `NestedNotInline`
arm, both in `boyko_reflect`, both landing **at C6** (§3.1). C9's list is the derive-side early
diagnostic. The ladder above is therefore unchanged — `C6 → C7 → C8 → C9`, with no backwards
dependency — and C9 may still land before C10/C11 for its own reasons (it gates the derive's
refusals), which is a scheduling preference and not a proof obligation.

---

## 6. DEFERRED, and to what

| Item | Deferred to | Why |
|---|---|---|
| `FieldInfo.serialize` + the `Sink` trait | **BOUNDARY's first rung**, together with its first reader | D9 — the dead-datum rule |
| `FieldInfo.debug_fmt` | the rung that gains a debug-dump consumer; **no plan currently owns one** | D9. When it lands it brings its own allocation-audit arm (§3.3) |
| `Vec` / `Map` / any collection | **v2** | scope (B) as taken; `SoftBody`'s fourteen `Vec` columns stay a documented compile error (D15) |
| data-carrying enums, `Option<T>` | **v2** | no Reference-guaranteed variant-field layout; `Option` is the smallest data enum and niche optimization removes the discriminant's guaranteed location |
| generics (incl. the engine's `State<S>`) | **v2**, behind a keyed-cell registry | a per-impl `static` collapses across monomorphizations. *`State<S>` is out of scope twice over — it is also a `Resource`, not a Component, so it has a `ResourceId` and no `ComponentId`* |
| `#[repr(packed)]` | **v2**, with `*_unaligned` accessors | taking `&field` is UB |
| `FieldMut<'a>` (a borrowed `&mut` into a field) | **v2**, after a full TB analysis against concurrent query borrows | precisely the "cached pointer + reborrow" class Tree Borrows caught in boyko after three critic rounds approved it |
| 2-D arrays `[[T; N]; M]` | **v2** | **D19** (stated at C5); the named victim is `csm_config.rs:392`'s `view_proj: [[f32;4];4]`, which is refused (D15) rather than silently flattened. A recursive `ArrayInfo` was rejected because it moves the unbounded descend into the *descriptor*, where §3.1's acyclicity argument does not reach |
| `#[reflect(as_str)]` for inline `[u8; CAP] + len` string components | **open**, engineering, deferrable | a UX question, not a soundness one; it must not delay the taxonomy. Already recorded in `docs/OPEN-QUESTIONS.md` |
| a runtime `TypeInfo` builder | **not planned** | its absence is why D7 rejects `IS_REFLECT`. If it is ever built, D7 is re-decided |

---

## 7. Dependencies on the sibling plans

Stated as obligations, because a rung whose gate nothing runs is this campaign's most repeated
defect.

1. **GATES owes C0 a feature-ON CI leg, and it lands FIRST.** G0–G4 precede C0 (§0, and GATES' G0
   preamble owns the order). Without that leg, F17 repeats verbatim: `hwrt` has **zero**
   occurrences in `ci.yml` today, so every `#[cfg(feature = "hwrt")]` body in the tree is compiled
   by nothing. C0 does not land until its red canary has been **seen red on that leg**.
2. **GATES owes C4 (and therefore C6, C10, C11) TWO Miri rows, and they have different shapes.**
   F18: CI's Miri step is a hand-listed allowlist, deliberately, because Miri cannot execute FFI. A
   new package is not covered until it is named.
   * **`-p boyko-reflect` PLAIN** — the arithmetic, the registry, the `prim::` accessors. ~~with the
     feature ON~~ is **wrong and unrunnable**: this crate has no `reflect` feature (GATES D4), so
     `--features reflect` on it is a hard cargo error, and with the feature "off" the crate is not
     empty — nothing in its source is `cfg`-gated. That sentence came from analysis B.9's closing
     line and was inherited by four documents; B.9 is corrected at the source.
   * **`-p reflect-fixture --features reflect-fixture/reflect`** — the only row that reaches
     derive-generated `unsafe`, and therefore the row C11's headline gate (the `&mut String` retag)
     actually depends on. It also constrains the fixture's dependency table to
     `boyko-ecs`/`boyko-macros`/`boyko-reflect`, which is why the real-engine dogfood lives in
     `reflect_dogfood` instead (§0.3, GATES D15).
3. **GATES owns the ship absence gate, and owns the ship-target list.** CORE supplies the property
   (`boyko-reflect` absent from the resolved feature closure) and nothing else; the subjects are
   **`boyko_demo` and the root `boyko-engine`** per GATES **D2** — *not* `boyko_app`, which this
   file's first revision named and which is `[lib]`-only. CORE does **not** specify the instrument;
   the analysis has already measured that the naive form cannot fail (default release and
   `--gc-sections` both leave the symbol at 1 on both legs; only `lto = "fat"` +
   `codegen-units = 1` makes it decidable), that `cargo tree` is the load-bearing half, and that the
   census needs a **present control** beside the absent cell.
3b. **GATES owes CORE its packages.** G0 lands `crates/boyko_reflect/` (with the hollow
   `install_type_info` stub **C2 replaces**, keeping the name and signature — GATES D5 chose its
   census needle *because* the name survives), `crates/reflect_fixture/` with its four bins and one
   bench, and `crates/reflect_dogfood/`. C0 consumes all three.
4. **ECS owes CORE nothing; CORE owes ECS the model.** CORE's accessors take a bare `*const u8` /
   `*mut u8` and never reach into `boyko_ecs`'s storage, so C1–C11 are buildable and gateable with
   no ECS glue at all. The seam is deliberate: it is what lets the value model be proven before the
   `get_component_raw` / enumeration work starts. ECS consumes `type_info_of`, `TypeInfo.fields`,
   and the `prim::` accessors, and owns everything about *reaching* the bytes — including
   `BUG-MIGRATE-TB-1` (raw-pointer projection, never `&Archetype`) and the three-source
   enumeration.
5. **BOUNDARY owes CORE the `serialize` slot's first reader** (D9), and consumes
   `get_serialize_info(id).stable_name` rather than a `TypeInfo` field (D8).
6. **OWNER decisions that block rungs.** The single list is analysis **B.13** (it supersedes B.11 as
   a *list*; the individual rows are unchanged), mirrored in `docs/OPEN-QUESTIONS.md`. The ones that
   touch CORE:
   * **B.13 #1 — may engine crates carry a `reflect` feature?** Decided before rung 0 (§0.3).
     **Blocks the dogfood halves of C6 gate 1 and C10 gate 1**, and the existence of
     `crates/reflect_dogfood/`. This plan proceeds on yes; a no deletes those halves and nothing else.
   * **B.13 #3 — `BindAccessor` one table or two** — **blocks C3/C8** (this plan proceeds on Horn 2,
     D16).
   * **B.13 #4 — are Aether components reflectable by default or opt-in** — **blocks the Aether side
     of C7/C8**, not CORE itself.
   * **B.13 #5 — build order within (B), arrays before `String`** — **taken as D13**, and it changes
     what the derive ships first.
   * **B.13 #2** (the four-item `boyko_ecs` seam) does not block CORE at all — §7.4's seam is
     deliberate: CORE's accessors take a bare `*const u8` / `*mut u8`.

---

## 8. What this plan does not claim

* **Not "<5 ns, monomorphic, branchless".** Field get/set is one *indirect* `fn`-pointer call
  returning a `Scalar` — bevy's *vtable* indirection traded for a *fn-ptr-table* indirection, one
  word narrower, the same class. It is far cheaper than bevy (no `Box`, no `DynamicStruct`, no
  downcast, no double-hash), and the shipped in-tree analogue agrees: `BindAccessor` is two bare
  `fn` pointers dispatched through a `match` on a `u8`, documented as *"never on a still frame or
  the per-frame hot path."* Nobody in this tree pretends this shape is free.
* **Not a proof that reflection beats bevy.** The registry lookup (one array index + one
  acquire-load, versus two hash probes + a downcast) is the defensible advantage, and the
  measurement that establishes it is a `get_field`-vs-bevy-shaped-baseline bench owned by GATES.
  The feature-on/off delta proves *zero-hot-path*; it does **not** prove *beats-bevy*. **Both
  benches are required, and neither is CORE's.**
* **Not zero cost by construction.** Zero cost is a property of the shipped artifact's **resolved
  feature closure**, enforced by a CI gate as a deliverable — not an inherent compiler guarantee.
  F16 makes a bare root build select every member, so *"a correctly-partitioned ship build"* is not
  one safe invocation among several; it is the only safe form.
* **Not a claim that the first-touch install is free.** `install_type_info` is one cold
  `OnceLock::set` per type per process, behind the `component_id()` `OnceLock`. That is real cold
  cost when the feature is on and **must not be mistaken for a hot-path regression** by GATES'
  0 %-gate, which asserts no delta on the **steady-state query/spawn inner loop**.
