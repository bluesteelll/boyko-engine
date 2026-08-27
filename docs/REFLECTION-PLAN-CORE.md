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
| `Sink`/`Source` · the name-keyed wire · `stable_name` consumption at the wire · tuple-struct reorder caveat — ~~⚠️ **DEBT, opened 2026-08-21 (C7 audit): BOUNDARY does not state the caveat.** Its only tuple-struct text is [`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md):977 and it carries no "caveat" / "by-position" / "positional" sentence; B4 gate 4's reorder subjects are all named-field types, so nothing there depends on what C7 retracts. This is a delegated statement with no recipient text, not a contradiction. **BOUNDARY owes the paragraph before its first `Sink` rung lands**~~ **PAID 2026-08-27 at the B0 audit.** BOUNDARY's **D24** states the caveat in full — "caveat", "by-position" and "positional" all present — and it lands a tuple-struct fixture (`PosPair(u32, u32)`) on rung B0, because the plan's only previous tuple struct was B5's dogfood `Name` ([`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md):1292), which an owner “no” on B.13 #1 deletes outright | [`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md) |
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
| F5 | In-tree installer/getter convention is `(component_id: usize, …)`, because the derive calls them as `…::component_id().0` | `component_registry/serialize.rs:287`, `:308` |
| F6 | `install_bind_accessor` is **`pub`** specifically *"so the `#[derive(Bindable)]` expansion (which lives in downstream crates where `pub(crate)` is unreachable) can call it"*; write-once, *"a same-id re-install is a silent no-op (first writer wins)"* | `component_registry/serialize.rs:299~-318` |
| F7 | The `component_id()` funnel is `static ID: OnceLock<ComponentId>` → `get_or_init` → `register_new::<Self>()` → **six** install slots (`storage`, `require`, `clone`, `relationship`, `residency`, `serialize`) + the const-gated `if Self::HAS_HOOKS` | `boyko_macros/src/component.rs:432-459`, **re-derived 2026-08-26 after C9 landed.** The content of this fact is unchanged and still **six** slots — `#reflect_install` is the seventh and is cited separately wherever it matters. **The anchors have now moved three times for the same reason** (C7 inserted the descriptor emission into the same `quote!`; C8 inserted the D29 condition block, the `reflect_install` binding and the slot; C9 replaced that condition block with the refusal argument and the `reflect_refused` gate), which is why the former numbers are recorded here as *"it moved"* and no longer as digits: a struck anchor rots exactly like a live one and reds nothing extra. **Live, in the tree C9 leaves behind:** `fn component_id()` `:434`, `get_or_init` `:437~`, `register_new::<Self>()` `:438~`, the const-gated `install_hooks` `:447~-449`, the six slots `:450~-455`, the seventh `#reflect_install` `:456~`, `ComponentId(raw)` `:457~`. **Since HEAD `eeb567be` all five reflection documents ARE in `internal_docs_anchors.rs`'s `GATED_DOCS`, so this row now reds when it rots** — which is how C9's own landing found it |
| F8 | Install slots come in two flavours: **const-gated** (`if Self::HAS_HOOKS`, for the derive-XOR-runtime-builder contract) and **ungated + self-gating** (`install_residency_class::<Self>` short-circuits on the default const) | `boyko_macros/src/component.rs:323~-346` |
| F9 | `#[component(...)]` already parses a bare flag key (`no_bundle`) and a `storage = "bitset"|"dense"` key, with duplicate detection and a *"valid keys: …"* diagnostic | `boyko_macros/src/component.rs:742~-873` |
| F10 | `StorageKind` is 3-way (`Table=0`, `Bitset=1`, `Dense=2`); `Bitset` has **no `ComponentPool`**, `Dense` has a global `DenseStore` and is **always `ResidencyKind::Cpu`** | `component_registry/mod.rs:323-339` |
| F11 | The `Bindable` derive's trampoline takes `let this: &T = unsafe { &*(p as *const T) }` off arena-rooted provenance, and it has passed Miri | `boyko_macros/src/bindable.rs:113`, `:119` |
| F12 | `core::mem::offset_of!` is load-bearing engine-wide on this toolchain, including as `const _: () = assert!(offset_of!(…) == N)` layout pins | `boyko_render/src/gpu_transform3d.rs:108-115` (317 sites tree-wide) |
| F13 | `boyko_serialize`'s manifest and lib header assert the directional rule at the source: *"never `boyko_reflect` (the codegen-not-reflection invariant)"* | `crates/boyko_serialize/Cargo.toml:6~-10`, `boyko_serialize/src/lib.rs:6~` |
| F14 | Package names are dashed, directories underscored (`boyko-serialize` in `crates/boyko_serialize`), and every member carries `[lints] workspace = true` | `crates/boyko_serialize/Cargo.toml` |
| F15 | `clippy.toml` bans `HashMap`/`HashSet`/`Mutex`/`RwLock`/`Rc`/`RefCell` at **deny** via `[workspace.lints.clippy] disallowed_types = "deny"`; `OnceLock` is **not** banned; exceptions carry `#[allow(clippy::disallowed_types)]` + a rationale | `clippy.toml`, root `Cargo.toml` |
| F16 | The root is **also a package**, so `default-members` names every member **plus `"."`** — *"there is no non-workspace-wide root build any more"* | root [`Cargo.toml`](../Cargo.toml):1-40 |
| F17 | **`grep -c hwrt .github/workflows/ci.yml` = 0.** Every `#[cfg(feature = "hwrt")]` body in the tree is compiled by **no CI leg**. A feature-gated body is invisible to the default gate — measured, not feared | `.github/workflows/ci.yml` |
| F18 | CI's Miri step is a **hand-listed package allowlist** (`-p boyko-ecs -p boyko-utils -p boyko-threadpool -p boyko-serialize -p boyko-math -p boyko_sdf_math -p boyko_image`), required, not `continue-on-error`; `MIRIFLAGS=-Zmiri-tree-borrows` is workspace-wide | `.github/workflows/ci.yml:193-226`, `.cargo/config.toml` |
| F19 | Features unify **per package**, and the tree has recorded the consequence: *"a `#[cfg]`'d field on a struct `boyko_app` constructs appears or vanishes for that crate depending on a flag none of its own source names"* | `crates/boyko_rhi_vulkan/Cargo.toml:21~-25` |
| F20 | A counting-global-allocator **delta** harness with baseline subtraction is the tree's established zero-allocation instrument | `crates/boyko_ui/tests/p4_bind_zero_alloc.rs:1~-20` |
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
  dodged by declining to declare the feature: root [`Cargo.toml`](../Cargo.toml):25-26's
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
downstream crate"* (`crates/aether_lang/Cargo.toml:6~-11`). A.5's jurisdictional point was
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

  ⚠️ **This sketch is a DECISION, not a rung, and until 2026-08-21 no rung's `Lands` created it —
  nor `trait Reflect`, nor the registration of `reflect` as a derive helper attribute, without
  which `#[reflect(no_default)]` below does not even resolve. All three are now C7's, in
  `boyko_reflect` / `boyko_macros` (D22).** Recorded here because this block is where the next
  reader meets the trait, and reading a fenced sketch as a landed item is how it stayed missing
  through four documents.

  `#[diagnostic::on_unimplemented]` is stable since 1.78 and is **already load-bearing in this tree**
  — `crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs:67~` and
  `query/filter.rs:2507~`, the second with a `compile_fail` fixture
  (`crates/boyko_ecs/tests/compile_fail_chunk/changed_filter_rejected.rs:11~`) that pins the message.
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

### D22 — C7 lands in `boyko_reflect` too. `Reflect` and `ReflectDefault` are ITEMS, not references; `reflect` is a REGISTERED helper attribute; and the descriptor is a FREE `static`, because `static` is not an associated item and `const` is not an address.

*Architect's ruling on the C7 audit, 2026-08-21.* Amends **C7's `Lands`** and, by consequence,
**C9 gate 5**.

C7's `Lands` opened *"In `boyko_macros`: …"* and then emitted, from that crate, paths into two
items that exist nowhere:

* **`boyko_reflect::Reflect`.** ~~`crates/boyko_reflect/src/` declares **no trait at all** — six modules, and its re-export block carries only the `TypeInfo` family plus `install_type_info`/`type_info_of`/`Scalar`/`ScalarKind`/`NestedCursor`/`FieldValue`.~~ **STATE AT THE C7 AUDIT (2026-08-21). The finding STOOD and C7/D22 discharged it** — `crates/boyko_reflect/src/reflect.rs:51` declares `pub trait Reflect` today, and `lib.rs` re-exports it beside the `TypeInfo` family.
  ⚠️ **Re-derived 2026-08-27 (ECS D25) rather than re-anchored a third time.** The count is
  **eight** modules, not six: it was already seven when the EG1 rung last edited this sentence, and
  that edit moved it one further from true while re-deriving the clause's own range onto a span
  that now includes the `pub use reflect::{Reflect, ReflectDefault};` line — its own
  counter-example. A range containing what its sentence denies is worse than no range, so it is retired rather than widened. The identifier appears in the plan set at exactly two sites and **both are uses** (C7 `REFLECTION-PLAN-CORE.md:2381`, C8 `:2995`). `REFLECTION-ANALYSIS.md:11` even records *the absence of a `trait Reflect`* as a finding about the tree.
* **`boyko_reflect::ReflectDefault`.** Its only occurrence anywhere is inside **D20's prose**
  (:344-349), in a fenced sketch introduced by *"where `boyko_reflect` declares"*. D20 is a
  decision, not a rung. By C9 the trait is load-bearing twice — gate 5 blesses a `.stderr` against
  its `on_unimplemented` message, and C9's third red deletes that attribute *from* it — while
  created by nobody.

**The decision.** Both are **C7's, in `boyko_reflect`**, landed in the same commit as the derive
that emits them. `Reflect` carries `const TYPE_INFO: &'static TypeInfo`; `ReflectDefault` is D20's
sketch verbatim, attribute and blanket impl. The mechanism D20 leans on is sound and was verified,
not assumed: `#[diagnostic::on_unimplemented]` is stable on the pinned 1.97.1 and already
load-bearing at `boyko_ecs/src/ecs/core/iters/query/chunked_data.rs:67~` and `query/filter.rs:2507~`,
the second with a blessed `compile_fail` fixture.

**Second part: `reflect` is registered as a derive helper attribute.**
`#[proc_macro_derive(Component, attributes(component, require, entities, relationship,
relationship_target))]` (`boyko_macros/src/lib.rs:140~-142`) names five, and `reflect` is not one.
An unregistered helper attribute is a hard error at the **use** site — *"cannot find attribute
`reflect` in this scope"* — resolved before the derive runs, so `#[reflect(no_default)]` is not
something the derive can inspect, it is something the user's crate fails to compile. C7's `Lands`
named only *"the `reflect` flag key on `#[component(…)]`"*, which is a **different attribute
namespace**, and no rung in any of the four plans lands the registration. D14's `#[reflect(skip)]`
and C9's `no_default_accepted` inherit the same requirement.

**Third part: the descriptor is a free `static`, reached by `&`.** `impl T { static X: u32 = 7; }`
is *"error: associated `static` items are not allowed"* on the pinned toolchain (compiled), so
`&T::__REFLECT_TYPE_INFO` naming a `static` is not expressible. The two escapes are **not**
interchangeable: an associated `const` gives the type **one descriptor per crate that reads it**,
and **C6's Check B identifies types by address** (`ptr::from_ref` + `ptr::eq` over the `path` and
`done` arrays, `type_info.rs:626~,641-643,651`), so a `const` descriptor degrades both its cycle
test and its memoization while `validate` goes on returning `Ok`. One stable address per type is
a C6 obligation. The form is a free `static` inside a generated `const _: () = { … };` block —
which is what every hand-baked static in the campaign already is.

> **Mechanism corrected 2026-08-21 (C7 landing).** This paragraph and §C7's bullet below both said
> a `const` "would be const-promoted at each `&`-site", giving "a fresh address per reference".
> **Measured: false on the emission C7 actually ships.** The expansion contains exactly *one*
> `&__REFLECT_TYPE_INFO`, so within a crate a `const` descriptor's address is stable — and
> substituting `const` for both `static`s leaves all sixteen `c7_derive_bake` tests green, both
> `ptr::eq` clauses included. The divergence is at the **crate boundary**: `&` on a `const` in a
> const-initializer yields an anonymous const-evaluated allocation, and each crate that evaluates
> the associated const interns its own copy (`0x7ff739cff5c8` upstream vs `0x7ff739cf26a0`
> downstream, one type). A graph read entirely from one side stays self-consistent, which is why
> no single-crate check can see it. The falsifying gate is
> `reflect_dogfood/tests/c7_cross_crate_address.rs` — the workspace's only annotated type defined
> in a *library* and read from a consumer, `reflect_fixture` having no `src/lib.rs`.

**Why recorded rather than fixed silently.** All three are *compile* errors, so an implementer
would have hit them within an hour and invented a fix. That is precisely the danger: the fix for
the third one is a coin-flip between `const` and `static` unless the author knows C6 compares
addresses, and the wrong side of that flip produces a **descend that still works** and an
acyclicity check that has quietly stopped recognizing types it has seen.

### D23 — C7's gates run in `reflect_fixture`, under a named feature-bearing invocation, against `c6_nested_descend.rs`'s statics. A gate whose oracle lives in a package that cannot compile the derive is not a weak gate; it is not a gate.

*Architect's ruling on the C7 audit, 2026-08-21.* Amends **C7's gate table** and **§3.3's
`default_in_place` row**.

C7's headline gate named C3's hand-written statics as its oracle. That oracle is unreachable for
three independent, measured reasons — the C3 types are **private items of an integration-test
binary** (`c3_type_info.rs:43,52,63,70`); `boyko_reflect` has **no `boyko-macros` edge** and cannot
get one usefully, because `boyko-macros` is a **dev**-dependency of `boyko_ecs`
(`boyko_ecs/Cargo.toml:89-90`) and does not propagate; and `crates/boyko_reflect/Cargo.toml:12~-16`
forbids a `[features]` table *"now or ever"*, so the derive's consumer-side `#[cfg(feature =
"reflect")]` is permanently false there. Three of the four C3 types are also out of rung order at
C7. **The decision:** C7's gates live in `crates/reflect_fixture/tests/c7_derive_bake.rs` and take
`c6_nested_descend.rs`'s `Body → Placement → Point` and `Slot → Handle` as the comparison target —
the file that **nominates itself** for the role in its own header (`c6_nested_descend.rs:20~-23`).

**And the invocation is part of the gate.** Every C7 gate file is `#![cfg(feature = "reflect")]`,
so a plain `cargo test -p reflect-fixture` compiles it to nothing and exits 0 — C6's own record
measures exactly that (`REFLECTION-PLAN-CORE.md:2327-2328`). The rung records `cargo test -p reflect-fixture --features
reflect-fixture/reflect --test <name>` and the `running [1-9]` check, for the green side **and for
every red**. This campaign's standing rule is that `running 0 tests` is a vacuous pass, and a gate
table that names no invocation is the shortest path to one.

**Consequences the same ruling settles.** Gate 5's alloc arm cannot be an arm on
`c4_prim_zero_alloc.rs` — that file is in the package that cannot run the derive, so the arm would
measure a hand-written `default_in_place` wearing the derived one's verdict (the weaker-subject
substitution C11 already forbids by name at :3354-3356). It becomes a **second** instrument, in
`reflect_fixture`, with its own positive control; the c4 header's *"for no gain"* argument against a
second `#[global_allocator]` was written before a subject existed that its binary cannot reach, and
the gain is now twofold, because keeping the allocator out of `c7_derive_bake.rs` is also what keeps
C7's derived descriptors inside §7.2's Miri row instead of behind a `#![cfg(not(miri))]`.

### D24 — A drop-count gate needs a type that drops. Gate 4's subject set was drop-free, so its counter was structurally zero, and the red named a slot the gate does not read.

*Architect's ruling on the C7 audit, 2026-08-21.* Amends **C7 gate 4**, **C7's second RED**, and
**§3.3's `default_in_place` row**.

`{ pod, Nested{pod}, [f32;4] }` has no drop glue anywhere in it. `needs_drop` is false for all
three, so `drop_in_place` is `None` for all three **whether the derive is right or sabotaged**, the
count is identically zero, and *"no leak, no double-free"* is unfalsifiable. **A.8 prescribed a set
that would have worked** — `{ pod, String, Nested{String} }`
(`REFLECTION-ANALYSIS.md:1054-1055`) — and §3.3's row (`REFLECTION-PLAN-CORE.md:1220`) substituted the drop-free one. The
substitution is the defect; the harness C7 builds is not.

**The decision.** Gate 4's set gains `Owned { tag: u32 }` with an instrumented `impl Drop`, and one
type nesting it. This is **better than A.8's `String` at C7** for two reasons: a `String` field is
`ValueKind::Str`, structurally accessorless until C11, and an exact drop **count** separates a leak
(too low) from a double-free (too high) where an allocator delta sees only the heap. A.8's `String`
half is **C11's**, and C11 already carries it — gate 2's *"exactly 1 alloc + 1 free"* and its second
red's *"1 alloc, 0 frees — a leak"*. Nothing is deferred that C7 could have done; one half moves to
the rung that owns the type.

**And the gate CALLS the slots.** C7 emits no install (deliberately, and that stays), so a mutated
`default_in_place`/`drop_in_place` would be **written and never read** — the dead-datum class this
campaign has now found five times. The slots are `pub`; the gate invokes them on a
`MaybeUninit<T>` destination it owns, and asserts exact counts in both directions. The second RED
is respecified accordingly: it named `drop_in_place` while pointing at a `default_in_place` gate,
over a fixture set containing neither a `String` nor a `Drop` — three failures stacked, any one of
which alone would have produced a green.

### D25 — Machine-made descriptors are `validate`d at the rung that first makes them, and the walk's INDEX is gated separately from its names and offsets.

*Architect's ruling on the C7 audit, 2026-08-21.* Amends **C7's gate table**; binds **C8**, **C10**
and **C11** by the same rule.

C3 built `validate` as the coherence oracle and C6 added the two structural checks to it; every
**hand-baked** fixture in the campaign calls it. C7 is the first rung whose descriptors are
**machine-made** and it called it nowhere — so a derive emitting `Nested` with `nested: None`, or
`Prim` with `get: Some(..)` and `set: None`, was green across the whole table. **The rule, standing
from here:** every rung that generates descriptors runs `validate` over each of them, and the rung
after it inherits the clause rather than re-deciding it.

**The second half is about the index, and it is the one nothing could see.** D14 forbids omitting a
field because by-index access would then depend on which fields were skipped, and
`c3_type_info.rs:304~-306` records the consequence — but the natural walk shape, `fields.iter()
.filter(|f| /* classifies */ …)`, keeps every surviving field's **name and offset right** and
shifts only the index. No C7 fixture had an unclassifiable field, so no gate could see it. `struct
Padded { a: u32, _pd: PhantomData<u8>, b: u32 }` gives it one, at C7, without waiting for
`#[reflect(skip)]` (C9): an unclassifiable field bakes `Opaque` with no accessors, which `validate`
accepts and D15's refusal later upgrades to a spanned error.

### D26 — G0's linkage deviation is retired at **C8**, not C7, because needle B is `install_type_info` and C7 emits no install. C7 adds the annotation BESIDE the linkage.

*Architect's ruling on the C7 audit, 2026-08-21.* Amends **C7**, **C8's `Lands`**, and corrects
[`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md):504-507.

GATES schedules the swap at C7 on the ground that the annotation *"puts needle A and needle B in the
feature-ON image by the same mechanism the annotation will"*. **Needle B is the literal name
`install_type_info`** (~~`reflect_absence_census.rs:124~`~~ → ~~`:181~`, re-measured at the C8
audit~~ → **`:228`, re-derived after C8 landed — the landing rewrote that file's header and moved
its own re-measurement**; GATES D5 calls it the LTO-sensitivity
probe), and the only thing that will ever reference it from the derive is **C8's** install slot —
C7 says so itself: *"No install call is emitted at this rung. The static exists and is inert."* A
swap at C7 would leave L2, the **present control**, with no `install_type_info` reference at all,
and nothing would red: the calibration table has no L2-needle-B column and the gate asserts only
L2 needle A `> 0`. A probe losing its subject in silence is this campaign's gate-that-cannot-fail
in its purest form.

**The decision, in two halves.** At **C7** `reflect_on.rs` gains `#[component(reflect)]` **beside**
its existing `reflect_linkage()` — so the derive key is exercised in the fixture, the G3 calibration
re-run that C2 and GATES both scheduled at C7 has the annotation it was scheduled for, and
`OPT_IN_TOKENS`'s live assertion (~~`reflect_absence_census.rs:314~,339-349`~~ → ~~`:371~` for the
const, `:397~-402` for the assertion, re-measured at the C8 audit~~ → **`:571` for the const,
`:622~-630` for the L2 assertion and `:601~-610` for the L3 one, re-derived after the C8 landing and
its follow-up**) stays green. At **C8**
the linkage is deleted and `OPT_IN_TOKENS` is updated to the annotation form **in the same change**,
which is what that assertion's own failure text instructs.
~~Both bin headers (`reflect_on.rs:10~-15`, `reflect_never.rs:1~-3`,
`reflect_off_twin_plus.rs:8~-11`) name C7 as the retiring rung and are corrected to C8 at that
landing.~~

> **Struck 2026-08-21 (C8 audit) — the word "Both" carries three anchors, and the third names no
> rung.** `reflect_off_twin_plus.rs:8~-11` reads *"This file is a copy of src/bin/reflect_on.rs (the
> twin's source) plus the marker … G7a's harness is the drift gate"* — no rung at all. The file does
> name C7, but **at `:18~`, in the item doc** (*"Tracked at CORE C7"*), and it names it as the rung
> that ADDED the annotation, not as a retiring rung — so it needs no correction either. **C8's own
> sentence (*"`reflect_off_twin_plus.rs` names no rung and needed none"*) is right about the header
> and wrong about the file**, and is corrected at C8 for the same reason. Reading either line and
> going to look for a C7 reference to fix is how this tree has introduced fresh rot before.
> **Corrected:** two bin *headers* name C7 as the retiring rung — `reflect_on.rs:10~-20` and
> `reflect_never.rs:1~-11` — and both were corrected in place at the C7 ruling.
> `reflect_off_twin_plus.rs` **carried** the linkage fn itself; **C8 deleted it**, exactly as
> the rest of this sentence predicted, so no line can be cited for it any more. Its own C7
> tracking note (`reflect_off_twin_plus.rs:18~-25`) survived: the fn and the sentences that
> describe it were on C8's list, the tracking note was not.

**Why this is a decision and not a scheduling detail.** Three landed file headers, one GATES
paragraph and one live assertion all point at C7, and C7's own `Lands` mentioned none of them. That
is the *"a landing that reds an existing gate, with the update in no list"* shape — and here the
update, applied as scheduled, would have **silently voided a probe** rather than reddening
anything.

### D27 — An install slot inside `component_id()` reaches no image until something CALLS `component_id()`. The three fixture bins must touch it, and D26's premise does not hold without that.

*Architect's ruling on the C8 audit, 2026-08-21.* Amends **C8's `Lands`** and **D26**.

D26 rests on one sentence: *"C8 is the rung whose install slot puts the literal name
`install_type_info` — G3's needle B — back into the L2 image, so C8 is where the linkage can be
deleted without voiding the probe."* **MEASURED FALSE.** The seventh slot lives inside
`component_id()`'s `get_or_init` closure (F7), and `reflect_on.rs`'s `main()` never calls
`FixturePod::component_id()` — it constructs the value and black-boxes one field
(`reflect_on.rs:43~-51`). An uncalled non-generic `#[inline]` method is dropped before the linker
sees it, exactly as C7's descriptor already is (*"at C7 the descriptor is referenced by nothing and
is dropped"*, ~~`reflect_absence_census.rs:127~-137`~~ → **`:171~-184`**, re-derived after the landing
rewrote that header).

`llvm-nm` over the three images the census itself builds
(`%TEMP%/boyko-reflect-census-reflect_on-on-{default,gc-sections,fat-lto}/release/reflect_on.exe`),
2026-08-21, `stable-x86_64-pc-windows-gnu` 1.97.1:

| needle | default | `--gc-sections` | fat LTO |
|---|---|---|---|
| `component_id` | **0** | **0** | **0** |
| `register_new` | **0** | **0** | **0** |
| `install_hooks` | **0** | **0** | **0** |
| `install_type_info` | 1 | 1 | 1 |
| `boyko_reflect` | 41 | 41 | 5 |

The whole funnel is absent in every link configuration; the **six existing slots are already in the
image's absence and the seventh would join them there**. The single `install_type_info` hit is
`_RNvNtCsd7WGKwjPoHP_13boyko_reflect8registry17install_type_info`, referenced only by
`reflect_on.rs:70~`'s `reflect_linkage()` — and by the pulled-object rule the census header states
(~~`reflect_absence_census.rs:78~-84`~~ → **`:86~-94`**) it is also the sole reason the other `boyko_reflect` symbols are there.
**Deleting the linkage as D26 schedules it therefore takes needle B to 0 AND needle A to 0,
reddening both `l2_b > 0` (~~`reflect_absence_census.rs:475~-484`~~ → **`:727~-738`**) and `l2_a > 0` (~~`:445~-451`~~ →
**`:673~-679`**)** — a census that was green at the audit (`cargo test -p reflect-fixture --features
reflect-fixture/reflect --test reflect_absence_census`, exit 0, ~~`running 2 tests`~~ →
**`running 3 tests`** since the C8 follow-up added `code_only_keeps_code_and_drops_prose`).

> *The `reflect_on.rs` anchors in this ruling (`reflect_on.rs:43~-51`, `:70~`) are the PRE-C8 tree's and are left
> as measured: they point at `reflect_linkage()`, which this rung deleted. The census anchors above
> are corrected instead of struck, because those clauses still exist. Re-derived by reading the
> files, 2026-08-21, after the landing and its follow-up.*

**The decision.** C8's `Lands` gains the obligation D26 did not carry: **the funnel is touched**.
`main()` gains `core::hint::black_box(<FixturePod as Component>::component_id());` in **all three**
fixture bins — `reflect_on.rs` (L1 and L2 share this source), `reflect_off_twin_plus.rs` (its own
contract is *"the twin's source plus exactly one fn"*, `reflect_off_twin_plus.rs:8~-11` — still lands) and `reflect_never.rs`
(*"nothing else about the two shapes may diverge, or L3 stops discriminating"*, ~~`reflect_never.rs:18~-21`~~ →
**`:24~-26`**). Un-touched, the
seventh slot is a **dead datum in the artifact** — this campaign's most-repeated class, and the
sixth instance was found one rung ago.

**What the touch buys beyond restoring the probe.** It is what makes L1's zero *mean* something for
the first time. Today the funnel is in neither image, so `l1_a == 0 && l1_b == 0` is earned by the
funnel's absence; after the touch the funnel is in **both** images and only the emitted
`#[cfg(feature = "reflect")]` separates them — which is precisely the property gate 2 claims. The
census's re-calibration, already scheduled at C8 by its own header (~~`reflect_absence_census.rs:134~-137`~~ → the schedule
line is now **`:180~-181`** and the table it produced is **`:137~-184`**), is where the new cells are
recorded; **they are not predicted here.**

### D28 — C8's gate 1 reads an ADDRESS, not `is_some()`. The install seam's two characteristic failures are wrong-descriptor and wrong-id, and `is_some()` sees neither.

*Architect's ruling on the C8 audit, 2026-08-21.* Amends **C8's `Gate`** and discharges an
obligation **C7's follow-up scheduled at C8 and C8's own list did not carry**.

`type_info_of` returns `Option<&'static TypeInfo>` (`boyko_reflect/src/registry.rs:54-64`) and the
table is write-once, **first writer wins** (`:87-128`). So `install_type_info(raw, <Sibling as
Reflect>::TYPE_INFO)` leaves an `is_some()` gate green *permanently* — the wrong descriptor is
never corrected — and `install_type_info(0, …)` is indistinguishable from `install_type_info(raw, …)`
for any subject that happens to hold id 0. That second shape is C7's red *"baked zeros into offsets
where every subject was one field wide"* wearing new clothes.

It is also an inherited obligation. C7's follow-up states the one-address-per-type property *"goes
live at **C8**'s install seam and **ECS EG8**, both of which read a descriptor from a crate other
than the one defining it"* (§C7 follow-up, and verbatim in
`reflect_dogfood/tests/c7_cross_crate_address.rs:36~-38`) — and C8's gate list read no address at all.

**The decision.** Gate 1 becomes an address identity over **at least two distinct subjects**, one of
them defined in a crate other than the one reading it. The rig exists and needs no new package:
`reflect_dogfood`'s `ProbeLeaf` / `ProbeRoot` (`src/address.rs:66~-84`) with
`probe_*_type_info_in_defining_crate`, already consumed by `c7_cross_crate_address.rs`. The gate
asserts, on the **first** touch of `component_id()`:

* `ptr::eq(type_info_of(T::component_id().0).expect(…), <T as Reflect>::TYPE_INFO)` for each subject;
* the two ids **differ** — an instrument precondition, stated before it is used, because a
  literal-`0` mutation is invisible to a one-subject gate whose subject holds id 0.

Two reds, both of which fire: **(i)** swap the descriptor argument to the sibling's `TYPE_INFO`
(gate 1 reds on `ptr::eq` for both subjects); **(ii)** replace `raw` with a literal `0` (both
install into slot 0 and first writer wins). Neither red is available to an `is_some()` gate.

> **Red (ii)'s DIAGNOSIS is interleaving-dependent, and this ruling's first draft named only one of
> its two halves. Measured 2026-08-21, five runs of the same mutated binary:** which subject reaches
> `install_type_info(0, …)` first decides which clause reports.
>
> * `--test-threads=1` (**2 of 2 runs, deterministic**): `a_second_component_id_touch_changes_nothing`
>   runs first and warms slot 0 with `ProbeLeaf`'s own descriptor, so `ProbeLeaf` (id 0) passes
>   `ptr::eq` and **`ProbeRoot` (id 1) reads `None`** — the `expect` reds, naming the literal-`0`
>   shape outright.
> * default parallel harness (**3 runs: wrong-address twice, `None` once**): when `ProbeRoot`
>   installs into slot 0 first, **`ProbeLeaf` (id 0) reads a WRONG ADDRESS** and the `ptr::eq` clause
>   reds instead, naming write-once.
>
> **The red itself is not race-dependent** — exit 101 in all five runs, with `the_install_seam_…` and
> `the_registry_route_…` red together and `assert_ne!(leaf_id, root_id)` never firing (the *ids* stay
> distinct; only the *table* collapses). So the gate's reliability is unaffected; what varies is
> which of two correct diagnoses the reader gets, and both messages point at the same mutation. It is
> recorded here because a ledger that names one outcome invites the next reader to treat the other as
> a new defect.

### D29 — `storage = "bitset"` + `reflect` is suppressed **at C8**, in the derive, because C8 is the rung that makes the emission DO something. C9 keeps the message; ECS D5's release assert moves to C9 with it.

*Architect's ruling on the C8 audit, 2026-08-21.* Amends **C8's `Lands`** and **C9's `Lands`**.

C8's gate 4 said the combination is *"a **derive error**, not a silent skip (C9 owns the message;
this gate owns \"the install is not emitted\")"* — two halves that exclude each other (if the derive
errors there is no expansion left in which to observe an absent install), and the strong half is
**C9's own table row** (`storage = "bitset"` | *no per-row bytes exist* | the `storage` key).

**MEASURED, 2026-08-21:** the combination compiles today and bakes a descriptor. A probe test
carrying `#[derive(Component, Default)] #[component(reflect, storage = "bitset")] pub struct
AuditBitsetTag;`, built `--features reflect-fixture/reflect`, ran `running 1 test`, exit 0, and
printed `bitset+reflect ACCEPTED: name=…::AuditBitsetTag size=0 align=1 fields=0 kind=Struct`.
Nothing rejects the pair: `reject_non_zst_bitset_tag` (`boyko_macros/src/component.rs:72~-76`) only requires
fieldlessness and `AuditBitsetTag` is fieldless; the `hooks.any()` rejection (`:84~-94`) covers only
lifecycle hooks; `Fields::Unit` maps to a zero-field `Struct` (`boyko_macros/src/reflect.rs:704~`). **And the
suppression does not exist either:** `boyko_macros/src/component.rs:181~` gates the reflect emission on
`hooks.reflect` alone, the one slot in its neighbourhood carrying no `storage_bitset` term — its six
neighbours all do (`:124~`, `:144~`, `:263~`, `:287~`, `:339~`, `:395~`).

> **The anchors in the paragraph above are the PRE-C8 tree's** (HEAD `bf7803d6`) and are left as
> measured, because the paragraph is a dated record of what the audit found. **Post-landing they are
> `:178~` for the condition — which now reads `hooks.reflect && !hooks.storage_bitset` — and `:123~`,
> `:144~`, `:254~`, `:278~`, `:330~`, `:386~` for the six neighbours** (`entities_items`,
> `serialize_items`, `clone_install`, `relationship_install`, `serialize_install`, `bundle_items`,
> in that order; only the first two did not move). Re-derived by reading the file, 2026-08-21, after
> the landing. Cite the neighbours **by binding name** in new text: names survive an insertion and
> these numbers do not, and no census covers this document (see F7).

**The decision, in two halves.**

* **At C8, the emission is suppressed for a bitset tag** — `hooks.reflect && !hooks.storage_bitset`,
  joining the six neighbours. C8 is the right rung and not C9 because C8 is where the emission stops
  being inert: it starts publishing into a `ComponentId`-keyed table whose id names a tag with **no
  per-row bytes**, which is the "coherent lie" shape C7's follow-up already caught in the non-struct
  arm. One term in one `if`; leaving it to C9 means shipping a rung whose install registers a
  descriptor for something that has nothing to describe. **Gate 4 is then runnable and its subject
  is one unit struct the rung creates**; its RED — drop the `storage_bitset` term — fires.
* **At C9, the message.** The spanned `compile_error!` is C9's table row and stays there. **ECS D5's
  second mechanism moves to C9 with it, and is written into C9's `Lands` so it stops being an
  obligation no rung carries:** `REFLECTION-PLAN-ECS.md:340-347` requires a *"release `assert!`
  inside `install_type_info`: `storage_kind(id) != Bitset`"*, and the landed installer has no such
  check — `boyko_reflect` names neither `storage_kind` nor `Bitset` anywhere, and this document
  mentioned `storage_kind` **zero** times before this decision. The getter is real and public
  (`boyko_ecs/src/ecs/core/component/component_registry/mod.rs:388`). **C8 does not need it:** after the suppression
  above, the derive — C8's only caller of `install_type_info` — can no longer pass a bitset id, and
  the runtime half exists for the callers the derive cannot see, which ECS D5 names as the
  Aether-expanded path and a future runtime reclassification. Neither exists before C9's refusal
  matrix.

### D30 — Horn-2 drift is a **generator**-agreement gate, not a source-drift gate: D14 makes source-level drift impossible by construction. Its host is `reflect_dogfood` plus a `boyko-ui` **dev** edge.

*Architect's ruling on the C8 audit, 2026-08-21.* Amends **C8's gate 5 and its second RED**, and
sharpens **D16**'s delivery clause.

Three things are wrong with the gate as written, and the third is the one the audit found last.

1. **The subject set is empty tree-wide.** Every `#[derive(…Bindable…)]` site
   (`boyko_ui/tests/p4_bind.rs:40~`, `p4_bind_zero_alloc.rs:94~`, `p4_miri.rs:63~`,
   `text_bind_emit.rs:50~`, `boyko_render/tests/ui_hud_screenshot.rs:455~`) carries no
   `#[component(reflect)]`, and every `#[component(reflect)]` site carries no `Bindable`. The only
   file holding both strings is `boyko_macros/src/lib.rs`, where they are two unrelated rustdoc
   examples (`boyko_macros/src/lib.rs:554~` and the reflect key's own docs).
2. **No listed package can host one.** The derive emits `impl ::boyko_ui::binding::Bindable`
   (`boyko_macros/src/bindable.rs:70`), so the subject needs a `boyko-ui` edge; D2 makes `#[cfg(feature = "reflect")]`
   evaluate in the **annotating** crate, so it also needs a feature literally named `reflect`.
   `reflect_fixture` forbids a third production dep — *"Deps are `boyko-ecs`, `boyko-macros`,
   `boyko-reflect` — and nothing else, ever"* (`reflect_fixture/Cargo.toml:23~-24`) — `reflect_dogfood` has no
   `boyko-ui` edge, and `boyko_ui` itself has neither a `reflect` feature nor a `boyko-reflect` dep
   (and is what D16 names as **Horn 1's** cost).
3. **The stated red is not expressible, and never becomes expressible.** *"Rename a field on one
   side only"* has no "one side": a single struct definition feeds both derives, both read the same
   `syn::Field::ident` (`boyko_macros/src/bindable.rs:36~-40,:54~`; `boyko_macros/src/reflect.rs:459~-462`), and neither admits a rename
   — `attributes(bind)` is declared at `boyko_macros/src/lib.rs:559~` and **never parsed** (`bindable.rs` never touches
   `f.attrs`). The audit's expectation that C9's `#[reflect(skip)]` would supply the divergence is
   **wrong, and D14 is why**: *"`#[reflect(skip)]` emits a `FieldInfo` with `kind: Opaque` … **it
   does not omit the field**"*, chosen precisely so *"the by-index API's indices"* cannot depend on
   which fields were skipped — *"the drift class again"*. So the reflect index equals the declaration
   index **always**, Bindable's id equals the declaration index **always**, and the two agree **by
   construction, permanently**. A gate whose red is a source edit is C7's descriptor-address gate
   again: green under the exact mutation its own doc names.

**The decision.** The gate is kept — D16 states plainly that this is *"what Horn 2 owes, and this
plan delivers at C8"* — but it is respecified as what it can see, and given a host.

* **What it asserts** is unchanged in content: for each subject, `<T as Bindable>::field_id(name)`
  equals the reflect field index, **for every field name**, plus `<T as Bindable>::FIELD_COUNT as
  usize == TYPE_INFO.fields.len()`. The iteration is driven by the **reflect descriptor's** names,
  because `Bindable` exposes no name enumeration at all — only `FIELD_COUNT` and `field_id(name)`
  (`boyko_ui/src/binding/bindable.rs:25-46`) — and the `FIELD_COUNT` clause is the half that catches
  a name reflect stopped emitting, which the name-driven loop cannot see.
* **What its RED is** — and this is the change: a mutation of one **generator**, not of a subject.
  Reverse `boyko_macros/src/bindable.rs:53~`'s `ids` (`(0..n).rev()`), or suffix `boyko_macros/src/reflect.rs:501~`'s baked `name:`.
  Either reds; a source rename does not and cannot.
* **Where the subject lives:** `reflect_dogfood`, which already has a `reflect` feature, a library
  target, an existing CI leg (`reflect-dogfood`, `cargo test -p reflect-dogfood --all-targets
  --features reflect-dogfood/reflect`) and the non-vacuity discipline the leg enforces — plus **one
  new line**, `boyko-ui` as a **`[dev-dependencies]`** edge. Dev, deliberately: it is test-only, so
  it enters no ship closure G2 censuses, and `boyko-ui` names no `boyko-reflect`, so it moves no
  reflect surface. None of G1's six manifest clauses touch it (C2/C3/C4 are about `boyko-reflect`
  edges and `features = […]` arrays; a plain dev edge with no features array is outside all of
  them). The alternative host — a test target in `boyko_render`, which already has both a
  non-default `reflect` feature (`boyko_render/Cargo.toml:44~`) and `boyko-ui` as a dev dep (`:125~`) at **zero**
  manifest cost — was **rejected**: no CI leg builds `-p boyko-render --features reflect`, so the
  test would be compiled by nothing (F17's measured class), and buying it back costs a CI job plus a
  `reflect_ci_coverage` row — more than the one line it saves.

### D31 — Two of C8's six gates measure something this toolchain cannot measure at C8. Both are re-homed rather than restated, and G6b's stated stable substitute is **measured blind**.

*Architect's ruling on the C8 audit, 2026-08-21.* Amends **C8's gates 3 and 6** and corrects
[`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)'s G6b note.

**Gate 3 (token absence, feature OFF).** Its deferral to G6b is not a route to an instrument:
G6b has **selected no form** (*"Which form lands is an implementation choice with a stated
criterion … Record the choice and the reason in the ledger"*, GATES `:1280-1282`), has **no ledger
row at all** (Appendix GB carries two `G6` rows and one `G6c`, no `G6b`), and has **no implementing
file** — `reflect_fixture/tests/` holds five files, none of them a token census. Every `G5`/`G6`/
`G6c`/`G7a`/`G7b`/`G8` row reads `— | —`, which that appendix's own preamble defines as *not
landed*. So *"this gate asserts whatever G6b selected"* resolves, at C8, to asserting nothing — while
C8 declares **both** its reds required.

Worse, the sentence that made the deferral look safe is false in **direction**. *"Under G6b's
`compile_fail` form the same mutation makes the fixture compile"* — it cannot. G6b's compile_fail
body (`GATES:1316-1321`) is `let _ = <MyComp as boyko_reflect::Reflect>::TYPE_INFO;` in
`reflect-fixture` compiled **feature-off**, where `boyko-reflect` is not in the resolved graph at
all (`reflect_fixture/Cargo.toml:32~,39`, `default = []`). The fixture's own `boyko_reflect::` path is therefore
`E0433` **regardless of what the derive emits** — the form is satisfied by the dep's absence and is
blind to the `cfg` it is meant to probe — and under the mutation it fails *harder*, never
"starts to compile". Measured at the workspace level in the same session:
`cargo tree --workspace -e normal` prints `boyko-reflect` exactly once, **at column 0** as its own
member root and at zero indented positions.

**The decision for gate 3: struck from C8, its content re-homed to the two GATES rungs that own
it.** The path half is **G6a**, which is C8's gate 2 already and is the strongest form there is —
GATES says so verbatim: *"The fixture compiling **is** the proof … it cannot be satisfied by
accident."* For a path token there is nothing between "zero occurrences" and "it compiles": a path
naming a crate absent from the graph is a hard error, so the two propositions are the same
proposition. The residual half — *a residue that compiles **and** names nothing*, e.g. an un-`cfg`'d
`#[used]` static or a branch that failed to const-fold — is **G6c's**, stated there in exactly those
words. C8 owns neither instrument and needs neither. **GATES is corrected in the same pass**: G6b's
note loses the false substitution claim, G6b's body records that its `compile_fail` candidate has
been measured blind (so its own criterion selects *neither* of its two candidates and a third form
must be chosen at its landing), and the ledger gains a `G6b` row so the rung has a slot to be
recorded in.

**Gate 6 (the six slots unperturbed).** This asks for a **byte comparison of an expansion**, which is
the exact measurement gate 3 spends nine lines deferring for being nightly-only — left uncorrected
one bullet later, with no deferral of its own. There is no in-tree substitute: `boyko_macros` is
`[lib] proc-macro = true`, so `component::expand` cannot be called from a test the way
`aether_lang`'s `expand_block` snapshot channel can. **Moved to G6c**, whose measurand this
literally is: *"`reflect_off_twin` (annotated, feature off) versus `reflect_never` (the same source
minus the `reflect` key) must emit the same symbol multiset and the same `.text` size"* — a
**concurrent** comparison on stable, needing no stored temporal baseline, and after D27 both legs
carry the funnel touch so the comparison is exactly *"does the seventh slot's existence perturb the
other six"*. **C8 does not need it:** for an un-annotated type the slot is `TokenStream2::new()`
interpolated into a `quote!`, which emits nothing as a language guarantee — the same guarantee the
six existing slots at ~~`boyko_macros/src/component.rs:389~-394`~~ → `:441~-446` → **`:450~-455`** (re-derived again after C9 landed; the
seventh, `#reflect_install`, is `:456~`) already rely on. This edge is recorded in §7 so it
cannot expire, which is how the last two rungs lost obligations.

### D32 — C9's trybuild corpus is **GATES G5's**, and two rungs were about to build one corpus because neither document could see the other.

[`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md):1196-1301 is a fully specified rung — *"G5 —
the derive's refusals: one trybuild corpus, two legs"* — that lands
`reflect_compile_fail.rs` + a `reflect_compile_fail/` directory in `crates/reflect_fixture/`, with
nine named fixtures, a feature-ON leg, a feature-OFF leg, the compiler-witness row, and four gates.
Its bitset paragraph ends with the sentence *"Named here so the two are not built twice."* C9 as
written re-specifies that corpus from scratch, in a different package, with a different row set and
no feature-off leg, and mentions neither `G5` nor `reflect_fixture` nor a feature.

**The duplication was invisible to the obvious search.** Grepping the four sibling documents for the
literal string `C9` returns **zero** hits in ECS, BOUNDARY, GATES and ANALYSIS — G5 refers to this
rung only as *"`docs/REFLECTION-PLAN-CORE.md`'s derive"*, and GATES' cross-plan table row reads
*"The derive's refusal set | `docs/REFLECTION-PLAN-CORE.md` | G5 — one trybuild fixture per
refusal"*. A cross-plan obligation addressed to a **document** rather than to a **rung** is
structurally unfindable from the rung that owes it, which is the same class as the C8 → EG8 edge
§7.4 records.

**The decision.** C9 builds the corpus, at G5's paths, under G5's fixture names, with G5's two legs.
G5 is thereby **discharged** and is to be struck to a cross-reference the next time GATES is
touched; the two defects this audit measured in G5 from the CORE side are recorded in §7.3d so they
travel with it. C9 owns the derive-side mechanism, `REFUSALS`, and the migration of the landed tests
its rows uncompile; nothing else about the corpus is re-invented here.

---

### D33 — The refusals live INSIDE the `#[cfg(feature = "reflect")]` emission, which forces the corpus's home. `boyko_ecs` cannot host it, and the reason is measured in both directions.

A refusal that fired with the feature **off** would refuse a program that compiles to nothing, which
contradicts D1's zero-cost-when-off premise and makes GATES G5's feature-off leg unsatisfiable by
construction. So each `compile_error!` is emitted inside the same `#[cfg(feature = "reflect")]` block
as the emission it guards — the `cfg` is evaluated in the **expanding** crate (D2), so the refusal
appears exactly where reflection was actually asked for.

That decides the corpus's home, because C9 gate 1 as written named
`crates/boyko_ecs/tests/compile_fail_*.rs`:

* `boyko_ecs`'s `[features]` table is `default`, `profiling-analysis`, `big_query_table`,
  `bench-alloc` — **no `reflect`**. trybuild copies the host manifest's `[features]` table into the
  generated crate and passes `--features` from the host's *enabled* set, so no fixture there can ever
  turn the `cfg` on. Every fixture would compile and the harness would red with *"expected compile
  failure"* — a gate that cannot go green, and whose repair by re-blessing would be vacuity.
* MEASURED on rustc 1.97.1: `#[derive(Component)] #[component(reflect)]` on a struct in
  `boyko_ecs`'s own test tree **compiles clean** — which is itself the proof that the `cfg` is dead
  there, since `boyko_ecs` has no `boyko-reflect` edge and a live `cfg` would be `E0433` — and emits
  `warning: unexpected cfg condition value: reflect`, promoted to an error by CI's clippy
  `-D warnings`.

The home is `reflect-fixture`: the only package that has `boyko-macros`, the `reflect` feature, and a
CI leg that builds it (`.github/workflows/ci.yml:174-194`). `boyko_reflect` is excluded for the same
`cfg` reason **plus** GATES D4's *"NO `[features]` table, now or ever"* and its lack of a
`boyko-macros` edge — the trap C7's gate 1 fell into, recorded at D23.

---

### D34 — Three of C9's seven rows refuse inputs that are ALREADY refused, and a row C9 does not author must not be in `REFUSALS`.

The census that keeps refusals honest has a mirror nobody had written down. *A refusal outside
`REFUSALS` is invisible to it* (D20 item 2). **A row in `REFUSALS` that C9 does not author is a
fixture whose red cannot fire**: the prescribed red is *"delete the refusal from the derive ⇒ its
fixture compiles ⇒ trybuild reds"*, and if rustc or `#[derive(Component)]` refuses the input anyway,
the fixture still does not compile and the red is unobservable. Measured on rustc 1.97.1, three rows
are in that state, and the third is a duplicate rather than an import:

* **Generic type parameters** — 15 errors with **no** `reflect` opt-in in the input, 20 with it. The
  seam is never reached, so the stated hazard is unreachable. **Deleted.**
* **`#[repr(packed)]`** — `E0793` from a plain `#[derive(Component)]` whenever a field needs align
  > 1; and with every field of align 1 the type **compiles, installs and is sound**. The reachable
  set is the harmless set. **Deleted**, with the condition under which the obligation returns
  recorded at the row.
* **The v2-kinds field row** — `Vec`, `Box`, `Option<T>`, `PhantomData<T>`, `&T` and raw pointers all
  bake `ValueKind::Opaque` from one fallthrough, so the `Opaque`-field row already refuses every one
  of them at the same span. **Merged**, not deleted: one rule, one message, one fixture.

Two `.stderr` fixtures pinning the *upstream* refusals are still worth keeping as regression pins,
but they live outside the census directory and are **not counted** — they pin rustc's diagnostic, not
C9's. Their feature-off behaviour is *does not compile* either way, which is what exposed GATES G5's
second leg (§7.3d).

**The same lens found the opposite defect too.** Two shapes the derive **accepts** today had no row
in any document's matrix — see D38.

---

### D35 — `#[reflect(skip)]` lands at C9, with the refusal it is the way out of. A refusal defined in terms of an escape hatch that does not exist is a refusal with no way out.

Five sites schedule it here and none of them is a **Lands** list: landed code
(`crates/boyko_macros/src/reflect.rs:288`’s `parse_reflect_skip` — the comment that said *"D14's field-level `#[reflect(skip)]` lands at C9 and is
parsed there"* has been replaced by the parser it promised), **D25**'s *"…without waiting for `#[reflect(skip)]` (C9)"*, §4's
*"`blob: OpaqueBlob` is a D15 hard error until C9's `#[reflect(skip)]`"*,
[`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md):1475 (which **blocks** rung B2 on it),
and GATES G5's `vec_field_skip_accepted` fixture. C9's Lands was a table of refusals only, and C9 is
the **last** CORE rung (§5), so nothing downstream could pick it up.

MEASURED 2026-08-26: `#[reflect(skip)]` on a field is accepted and **completely inert** —
`parse_reflect_no_default` scans `input.attrs` (type-level) exclusively, so a field attribute is
never read, and a `Vec<u32>` field carrying the skip bakes the same `Opaque` descriptor as one
without it. C9 therefore lands the field-attribute parse, D14's semantics (`fields.len()` unchanged,
`kind: Opaque`, all four accessor slots `None` — *never* an omission, because by-index access would
otherwise depend on which fields were skipped) and an ACCEPT fixture. It is also the migration for
`crates/reflect_fixture/tests/c7_derive_bake.rs:219`'s `Padded`.

---

### D36 — `REFUSALS` is a table of `(rule, message)` read AT the refusal sites, and its census is a source-text scan. A `const` a proc-macro crate "iterates" cannot be read by any test, and a list nothing emits from is a dead datum.

Gate 2 as written asks for *"a `const REFUSALS: &[&str]` the derive itself iterates"* and a test
comparing its length to a fixture count. Neither half is constructible:

* `boyko_macros` is `[lib] proc-macro = true`, so no test can import the const — D31 measured this
  same obstacle about this same crate for `component::expand`. The only available instrument is a
  **source-text scan**, the shape `tests/internal_docs_anchors.rs` uses.
* Nothing *iterates* a list of rule names to decide anything: each refusal is a distinct syntactic
  condition, evaluated at its own site. A `const` the derive merely **declares** is computed and
  never read — the class this campaign has now found five times.
* And an equality of counts is one-directional. It sees a rule added without a fixture. It cannot see
  the failure D20 item 2 exists to name — a refusal added to the derive and **not** to `REFUSALS` —
  because a `&[&str]` carries neither span nor message, so each refusal's text lives at its own site
  and a new site can simply not appear. That is C8 gate 5's shape: a drift test that cannot detect
  drift by construction.

**The shape that makes the datum live:** `const REFUSALS: &[(&str, &str)]` with one `IDX_*` per rule,
and every refusal site emitting `REFUSALS[IDX_X].1` inside its `quote_spanned!`, so the row's bytes
**are** the diagnostic's bytes. A refusal with no row then has nothing to say and does not compile.
The census scans for `REFUSALS` rows, counts them against `.rs` files in the compile_fail directory
only, and asserts each row's message literal appears at a `quote_spanned!` site in
`crates/boyko_macros/src/reflect.rs` — or, for the one **message-only** row (D20's, which is a trait
bound and cannot be a `compile_error!`), at `crates/boyko_reflect/src/reflect.rs:84`'s
`on_unimplemented`.

---

### D37 — The bitset refusal is spanned at the **`reflect` key**, and it REPLACES D29's derive-side suppression rather than sitting beside it.

**The span.** Three census-gated documents specified three different carets for one refusal:
this plan's C9 table said *"the `storage` key"*;
[`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md):340-347 and its §10 dependency row said *"the
user's own type name"*; [`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md):1386-1389 said
*"pins the span on `reflect`, not on the struct and not on `aether! {`"*. One emission has one span,
and gate 1's blessed `.stderr` freezes whichever is built first, so the choice had to be made before
the corpus. ECS's rationale is analysis B.5's Aether case, which
[`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md):520-529 **deleted by construction** and
which this audit verified in the tree: Aether emits `#[component(storage = "bitset")]` from
`tag Foo(bitset);`, and the string `reflect` does not appear in `crates/aether_lang/src/` at all
except as the PBR material key `reflectance`. The remaining user is hand-written Rust, where
`storage = "bitset"` is legitimate on its own and **`reflect` is the token that is wrong**. Span on
`reflect`; ECS's *"type name"* is superseded, and GATES' `aether_tests` twin for this combination is
a fixture for an input Aether cannot emit (§7.3d).

**The replacement.** D29 landed a *silent* suppression at C8 — the `!hooks.storage_bitset` term in `codegen`'s condition (`crates/boyko_macros/src/component.rs:181~`’s binding today) —
`let reflect_enabled = hooks.reflect && !hooks.storage_bitset;` — and handed the message here. With
the refusal in place the term is unreachable in its suppressing branch, and its only witness
(`reflect_fixture`’s `c8_bitset_suppression.rs`, deleted by C9) no longer compiles: a dead datum whose gate
has just been deleted, and D29's own RED (*"drop the `storage_bitset` term"*) becomes unobservable
because no subject can be constructed to observe it. The term is **deleted with the gate it served**,
and C8's clauses migrate into the corpus. Nothing is lost: feature off, the whole emission is
`cfg`-stripped and nothing installs; feature on, the refusal stops the compile. ECS D5's *"two
mechanisms at two boundaries"* is the compile-time refusal plus the release `assert!` — not three.

**And the release `assert!`'s ownership claim was wrong.** C9's *"It was on no rung's list in any of
the four documents"* is false: [`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md):1320 carries it
against rung **EG3** with a live fallback (*"If CORE declines it, EG3 must add the check on its own
read path and say so"*). C9 accepting the item creates a **C9 → EG3** edge and an obligation to
retire that conditional; both are recorded in §7.4.

---

### D38 — Two shapes the derive ACCEPTS had no row in any matrix: a union, and a data-carrying enum at the ITEM level. The audit's own lens found them by asking the reachability question in the opposite direction.

`crates/boyko_macros/src/reflect.rs` used to send every enum **and** every union down one arm — one arm,
two shapes — baking `TypeKind::Opaque` with an empty field list. C9 split it: `:712~` is the enum arm, `:729~` the union arm. C9's matrix named *"a fieldless enum
with no `#[repr(Int)]`"* (item span) and *"a data-carrying enum … | the field"* (field span), and
nothing at all about unions. MEASURED 2026-08-26: `#[component(reflect)]` on a **two-field union**
compiles and prints `kind=Opaque fields=0 size=4 align=4`, and on a `#[repr(u8)]` enum with payload
variants likewise — both published through C8's install seam. `reflect_fixture`’s `c7_derive_bake.rs` (its `the_non_struct_arm_bakes_an_opaque_fieldless_descriptor`, deleted by C9)
pins exactly that lie for the enum and calls it *"the claim this test pins"*; after C9 as written the
identical claim for a union would be unpinned **and** unrefused.

Two ITEM-level rows are therefore added — a data-carrying enum, and a union — matching GATES G5's
`union_rejected`, which this plan had no counterpart for. What remains accepted is a fieldless
`#[repr(Int)]` enum, for which `fields: &[]` is *true* rather than a lie; its `kind: Opaque` is still
a silent `Opaque` until **C10** replaces it with `TypeKind::Enum`, and §5 permits C9 to land first,
so the window is recorded at the row rather than left to be rediscovered.

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
| `default_in_place` | **0 bespoke** | C7 | delta harness (**in `reflect_fixture`, its own binary and its own `#[global_allocator]` — D23**) + a **drop-count** test (A.8) over ~~`{ pod, Nested{pod}, [f32;4] }`~~ → **`{ Pod, Nested{Pod}, ArrPack{[f32;4]}, Owned, Nested{Owned} }` where `Owned` has an instrumented `impl Drop` — D24: the struck set is drop-free, so the counter was identically zero and the gate could not fail. A.8 itself prescribed `{ pod, String, Nested{String} }` (`REFLECTION-ANALYSIS.md:1054-1055`) and this row substituted the drop-free one; the `String` half is C11's** |
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
> (at the appended canary's own `cfg` line in `crates/boyko_threadpool/src/lib.rs` — a
> coordinate in the MUTATED file state, which was then restored byte-identically, so there
> is deliberately no line to cite), with *"expected values for `feature` are: `default` and
> `scheduler-trace`"*
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
> governs the ship-visible table; the workspace's dev-only instrument, [`Cargo.toml`](../Cargo.toml):52~); the
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
> `registry.rs:19~`, exit 101. *The drift demonstration, with one execution finding:* the plan's
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
> inside the library** (`prim.rs:142~: assertion left == right failed; left: Bool, right: F32`)
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
> *"attempt to multiply with overflow"* raised at `array.rs:63~` — a **panic out of a
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
same way in **five** members already, e.g. `crates/boyko_ecs/Cargo.toml:91~` (`trybuild = "1"` under
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
> * `grep -rn "reflect" crates/boyko_macros/src/` → 2 hits, both unrelated prose (`boyko_macros/src/bundle.rs:274~`
>   *"the registry layout reflects `size_of`"*, `boyko_macros/src/lib.rs:534~` *"reflection-free"*). The derive key
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
>  --> src\lib.rs:2:25 <!-- doc-anchor-ignore -->
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
> (`transform.rs:33~` imports them from there). Plain, non-optional, no `features` array — the rule
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

**Lands.** ~~In `boyko_macros`: the `reflect` flag key on `#[component(…)]` (F9's mechanism, one
match arm), and the emission~~ → **corrected 2026-08-21 (architect's ruling on the C7 audit,
D22): C7 lands in TWO crates, and the `boyko_macros` half is four sites, not one.** The rung as
first written emitted impls of two traits that exist in no crate and in no rung's `Lands`, and
read a helper attribute the derive does not register — three hard-error-at-compile defects, each
verified against the tree rather than argued.

**In `boyko_reflect` (D22):**

* `pub trait Reflect { const TYPE_INFO: &'static TypeInfo; }` — C7's first emission bullet names
  it and C8's install (`<Self as boyko_reflect::Reflect>::TYPE_INFO`, `REFLECTION-PLAN-CORE.md:2995`) consumes it, but
  ~~`crates/boyko_reflect/src/` declares **no trait at all** (six modules)~~ — **state at the D22 audit; DISCHARGED, and the count was wrong in both directions (ECS D25, 2026-08-27): the crate is EIGHT modules, and `crates/boyko_reflect/src/reflect.rs:51` declares the trait today.** At the time no
  rung's `Lands` created one, and without it the emission did not compile.
* `ReflectDefault` **exactly as D20 sketches it** (`#[diagnostic::on_unimplemented]` with D20's
  two strings, blanket `impl<T: Default> ReflectDefault for T {}`). D20 is a *decision*, not a
  rung; its sketch was never assigned to anyone. C9 gate 5 already blesses a `.stderr` against
  this trait's message and C9's third RED already deletes the attribute *from* it, so by C9 it is
  load-bearing in two places while created in none.

**In `boyko_macros`:**

* the `reflect` **flag key** on `#[component(…)]` — F9's mechanism is accurately described
  (bare flag, duplicate detection, a "valid keys" diagnostic; `no_bundle` is the precedent at
  `boyko_macros/src/component.rs:749~-757`) but ~~one match arm~~ → **four sites, and there is no match**: the
  parser is a linear `if meta.path.is_ident(..)` chain inside one `parse_nested_meta` closure
  (`boyko_macros/src/component.rs:736~-908`), so the key needs a field on `ComponentHookPaths`, its own `if` block
  with its own duplicate-detection error, and **both** copies of the "valid keys: …" literal —
  `boyko_macros/src/component.rs:742~` (the `on_despawn` arm) and `:894~` (the unknown-key arm), two independent
  strings. Updating one and not the other ships a diagnostic that lies about which keys are valid;
* the **registration of `reflect` as a derive helper attribute.**
  `#[proc_macro_derive(Component, attributes(component, require, entities, relationship,
  relationship_target))]` (`boyko_macros/src/lib.rs:140~-142`) lists five names and `reflect` is not
  one, so `#[reflect(no_default)]` is a hard *"cannot find attribute `reflect` in this scope"*
  resolved **before** the derive runs — the derive cannot inspect what does not resolve. `reflect`
  is a different attribute namespace from `component`, and no rung in any of the four plans lands
  it (`grep "attributes("` over CORE/ECS/BOUNDARY/GATES returns zero hits). D14's
  `#[reflect(skip)]` and C9's `no_default_accepted` inherit the same requirement;

and the emission — behind `#[cfg(feature = "reflect")]` (D2) — of

* ~~`impl boyko_reflect::Reflect for T { const TYPE_INFO: &'static TypeInfo =
  &T::__REFLECT_TYPE_INFO; }`~~ with ~~a `static __REFLECT_TYPE_INFO: TypeInfo`~~ →
  **`const _: () = { static TYPE_INFO_FOR_T: ::boyko_reflect::TypeInfo = …; impl
  ::boyko_reflect::Reflect for T { const TYPE_INFO: &'static ::boyko_reflect::TypeInfo =
  &TYPE_INFO_FOR_T; } };`** — a **free** `static` in a generated block, reached by `&`, never an
  associated item. The two struck halves cannot both be true: `static` is not permitted as an
  associated item, so `T::__REFLECT_TYPE_INFO` cannot name one. *Compiled, not argued* — `impl T {
  static X: u32 = 7; }` under `--edition 2024` on the pinned 1.97.1 gives *"error: associated
  `static` items are not allowed"*, exit 1. **And the escape must be a `static`, not a `const`**:
  C6's Check B identifies types by ADDRESS (`ptr::from_ref` + `ptr::eq` over the `path` and `done`
  arrays, `type_info.rs:626~,641-643,651`), and ~~a `const` is const-promoted afresh at each
  `&`-site~~ → **corrected 2026-08-21: a `const` gives the type ONE DESCRIPTOR PER CRATE that
  evaluates it** (measurement in the boxed note at §D22's "Third part" — the expansion has exactly
  one `&`-site, so the *within-crate* address is stable and every same-crate `ptr::eq` in this
  campaign is blind to the substitution),
  so a `const` descriptor degrades the acyclicity walk's cycle test and its memoization into a walk
  that cannot recognize a type it has already seen — as soon as the graph spans two crates, which
  is exactly what C8's install seam and ECS EG8 hand it. One stable address per type is a C6
  obligation, not a style choice. Gated by `reflect_dogfood/tests/c7_cross_crate_address.rs`,
  landed at C7. It is also the shape every hand-baked static already uses (`nested:
  Some(&D_LEAF_TYPE_INFO)`, `c4_prim_zero_alloc.rs:722~`);
* that `static`'s `fields`, a baked `&'static [FieldInfo]`, every `offset` a
  `core::mem::offset_of!(T, f)` (F12 — ~~the exact idiom already load-bearing 317 times on this
  toolchain~~ → **317 lines under `crates/*/src` at the plan-authoring commit `e1b430f2`;
  re-measured 2026-08-21 the figure is 325 there and 452 repo-wide. The number was true when
  written and is not maintained, so it is dated here rather than re-stated.** F12's *anchor* is
  also the weaker of two available proofs: `gpu_transform3d.rs:108-115` is `offset_of!` in
  **expression** position inside a `const _: () = assert!(…)`, which witnesses const-evaluability;
  the position C7 bakes into is a **`static` initializer**, and the tree witnesses that one exactly
  — `boyko_reflect/tests/c3_type_info.rs:185~` (`offset: offset_of!(Inner, x)` inside `static
  INNER_FIELDS: [FieldInfo; 2]`) and `c4_prim_zero_alloc.rs:484~,680,717,754,765`. Cite those);
* `default_in_place` / `drop_in_place` from `Default` / `needs_drop` — **`default_in_place` is
  `Option`, and its `Some` arm carries a named bound assertion, D20.** The derive emits
  `Some(__default_in_place::<T>)` plus the `const _: fn() = || { fn __assert_reflect_default<T:
  ReflectDefault>() {} __assert_reflect_default::<T>(); };` witness spanned at the type name, so a
  type with no `Default` fails with `ReflectDefault`'s `#[diagnostic::on_unimplemented]` message
  rather than with an `E0277` pointing into generated code. `#[reflect(no_default)]` emits `None`
  instead, and emits no witness. *(Both the trait and the attribute are landed above, by D22 —
  neither existed when this bullet was written.)*

**The walk is index-faithful and total — D14 restated here because C7 is the rung that lands the
walk.** `fields.len()` equals the type's **declared** field count, unconditionally: no field is
filtered. A field the v1 kind table cannot classify — a ZST, a `PhantomData<T>`, an opaque handle —
bakes `kind: ValueKind::Opaque` with **every** accessor slot `None` (which `validate` accepts;
`Violation::OpaqueWithAccessor` fires only if one is carried), never a shorter list. C9 turns the
un-skipped `Opaque` into the spanned refusal D15 requires; C7 owes only that the index of every
field is its declaration position. The natural implementation — `fields.iter().filter(|f| /* has a
ValueKind */ …)` — keeps every surviving field's name and offset **right** and shifts only the
INDEX, which is the entire addressing model, and gate 8 exists because nothing else in this table
could see it.

**Tuple structs:** `FieldInfo.name` is `"0"`, `"1"`, … and `TypeInfo.kind` is
`TypeKind::TupleStruct` (`type_info.rs:109~` — the arm exists and the derive must set it, which no
gate said). **For tuple structs by-name == by-position**, so the reorder stability the design
advertises does **not** hold for them — ~~stated in the derive's diagnostic~~ → **stated in the
type's docs and in `#[component(reflect)]`'s own rustdoc, because a derive diagnostic for it is not
expressible**: a tuple struct is *accepted*, so there is no `compile_error!` to carry the text, and
a non-fatal proc-macro warning needs the nightly-only `proc_macro::Diagnostic` (`boyko_macros/src/`
contains no `Diagnostic` / `emit_warning` site to model one on, and this campaign's toolchain is
stable). (Named-field structs are recommended for anything serialized; ~~the wire consequence is
BOUNDARY's~~ → **the wire consequence is delegated to BOUNDARY by §0's owner table (`REFLECTION-PLAN-CORE.md:22`) and
BOUNDARY does not state it** — ~~its only tuple-struct text is `REFLECTION-PLAN-BOUNDARY.md:977`~~ **PAID 2026-08-27: BOUNDARY's D24 states it, and the `Name` row is now at `REFLECTION-PLAN-BOUNDARY.md:1365`** (re-derived by reading the site when rung B0 landed and moved it; the previous value `:1292` was correct on the day it was written). As written this said it carries no "caveat" /
"by-position" / "positional" sentence at all. A delegated statement with no recipient text is a
caveat that exists in one document's index and nowhere else; **BOUNDARY owes the paragraph, and
§0's row is annotated to say the debt is open** — see the §0 note.)

**No install call is emitted at this rung.** The static exists and is inert; C8 wires it. Splitting
them keeps "the derive computes the right offsets" separable from "the funnel appends correctly",
which are two different failures.

**Consequence for the G0 linkage deviation, and it moves a scheduled retirement (D26).**
[`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md):504-507 schedules *"CORE C7 swaps the
linkage for the real `#[component(reflect)]` annotation"* on the ground that the annotation *"puts
needle A and needle B in the feature-ON image by the same mechanism"*. **Needle B is the literal
name `install_type_info`** (`reflect_absence_census.rs:228`, GATES D5's LTO-sensitivity probe) and
the only thing that will ever reference it from the derive is **C8's** install slot. So at C7 the
sentence is false and at C8 it is true. **C7 therefore ADDS the annotation and KEEPS the linkage;
C8 deletes the linkage and updates `OPT_IN_TOKENS`.** Concretely, at C7 `src/bin/reflect_on.rs`
gains `#[component(reflect)]` on `FixturePod` **beside** its existing `reflect_linkage()`, so
`reflect_absence_census.rs:571`'s `OPT_IN_TOKENS = ["reflect_linkage", "boyko_reflect::"]` and its
live assertion at :622~-630 stay green — the assertion whose own failure text says *"if CORE C7 has
landed the `#[component(reflect)]` key, update `OPT_IN_TOKENS` … in the same change"*, an
instruction C7's `Lands` and regression list did not carry. **Implementer trap, named because the
compiler finds it and the plan should have:** `FixturePod` has **no `Default`**, so the annotation
alone reds it on D20's witness — `#[derive(Default)]` goes on `FixturePod` **and** on
`reflect_never.rs`'s twin shape in the same change, or L3 stops being "the same source minus the
opt-in".

**Gate.**

> **Where the gates run, and under what invocation — D23, and the table said neither.** Every gate
> below lives in **`crates/reflect_fixture/`**, in a new `#![cfg(feature = "reflect")]` test file
> `tests/c7_derive_bake.rs` (gate 5 excepted, see its own note), and runs as
> ```
> cargo test -p reflect-fixture --features reflect-fixture/reflect --test c7_derive_bake
> ```
> **whose output must read `running [1-9]`.** The derive's emission is `#[cfg(feature =
> "reflect")]` evaluated in the **expanding** crate (D2), and no package's `default` enables
> `reflect`; the established shape for such a file is a whole-file `#![cfg(feature = "reflect")]`
> (`c6_nested_descend.rs:31~`), which C6's own record confirms compiles to **0 tests feature-off by
> design** (`REFLECTION-PLAN-CORE.md:2327-2328`). A C7 gate or red run as a plain `cargo test -p reflect-fixture` therefore
> prints `running 0 tests` and exits 0 — a vacuous pass on the green side *and* on the red side.
> Every red below must be OBSERVED under this invocation.

1. **Derived == hand-baked.** ~~For the C3 fixture types, `T::TYPE_INFO` compares field-for-field
   equal to C3's hand-written static~~ → **for `reflect_fixture`'s own `Body → Placement → Point`
   and `Slot → Handle` (`tests/c6_nested_descend.rs:56-83`), `<T as Reflect>::TYPE_INFO` compares
   field-for-field equal to the hand-written static beside it** — names, offsets, kinds,
   `fields.len()`, accessor presence. **The C3 fixture types are not, and cannot be made into, this
   gate's oracle, for three independent measured reasons (D23):** (a) `Inner`, `Facing`,
   `OpaqueBlob` and `Everything` are **private items of an integration-test binary**
   (`c3_type_info.rs:43,52,63,70`) — a test binary exports nothing and there is no `src`-side or
   fixture-library copy; (b) `boyko_reflect` **cannot invoke the derive** — its manifest lists
   `boyko-ecs` as its only dependency and `proptest`/`trybuild` as its only dev-dependencies, and
   `boyko-macros` is a **dev**-dependency of `boyko_ecs` (`boyko_ecs/Cargo.toml:89-90`, under
   `[dev-dependencies]`), so it does not propagate, and `boyko_ecs/src` re-exports it nowhere;
   (c) even with the edge added, `crates/boyko_reflect/Cargo.toml:12~-16` states there is **NO
   `[features]` table, now or ever (GATES D4)**, so the derive's emitted `#[cfg(feature =
   "reflect")]` is permanently false there and `T::TYPE_INFO` would never exist. Three of the four
   C3 types are additionally out of rung order at C7 (`Facing` is C10; `label: String` is C11's
   `Str`; `blob: OpaqueBlob` is a D15 hard error until C9's `#[reflect(skip)]`), and `Everything`
   has no `Default`. **The reachable oracle already exists and nominates itself**:
   `c6_nested_descend.rs:20~-23` — *"C7 replaces these statics with generated ones and inherits this
   file as an independently-pinned comparison target"* — in the one package that has
   `boyko-macros`, the `reflect` feature, and locally-declared depth-2 named **and** tuple nests.
   ~~The hand-baked side is the oracle and it is independently pinned by C3 gate 4.~~ →
   **The hand-baked side is the oracle, and what pins it is stated per-attribute rather than
   blanket, because "independently pinned" covered less than the sentence implied.** C3 gate 4's
   two tests pin **names in declaration order** and **offsets**
   (`every_baked_offset_equals_offset_of`, `c3_type_info.rs:639-660`, which does assert
   `field.name` — the audit claim that names are pinned by nothing is **refuted**) and
   **size/align** (`baked_size_and_align_match_the_types`, :665-672). **Kinds** are pinned only by
   C3 gate 1's `validate` *coherence*, and **accessor identity is pinned by nothing on either
   side** — a descriptor carrying `get: Some(hand::get_u32)` on an `f32` field is coherent and
   green. So this gate adds the clause that closes it: for every `Prim` field, a **round-trip**
   through the derived slots (`set` a known `Scalar`, `get` it back, and compare against a direct
   read of the field) — which is what makes "the derive picked the accessor for *this* field's
   type" a check rather than a hope. The `c6_nested_descend.rs` statics carry their own
   `offset_of!` pin under this gate for the same reason C3's do.
2. `(TYPE_INFO.type_id_fn)() == TypeId::of::<T>()`; `size`/`align` == `size_of`/`align_of`; and
   `TypeInfo.kind` is `Struct` for `Body`/`Placement`/`Point` and `TupleStruct` for `Slot`/`Handle`.
3. ~~Reorder two fields in a fixture ⇒ the derived offsets move accordingly (a `#[repr(Rust)]`
   type, so the assertion is "offsets are a permutation consistent with `offset_of!`", not fixed
   numbers).~~ → **Two structurally identical `#[repr(Rust)]` types with SWAPPED declaration order,
   both resident in the file.** "Reorder two fields in a fixture" describes a source edit performed
   at gate time; a standing gate cannot be an edit. For each field of **both** orderings,
   `derived.offset == offset_of!(T, f)` — that is the clause the all-zero red dies on — **plus a
   non-vacuity clause `derived_offsets(AB) != derived_offsets(BA)`**, which is the half that
   actually shows the offsets move and the half "permutation" was reaching for. *(The audit read
   "offsets are a permutation" as satisfiable by `{0,0,…}` being a permutation of itself; the
   trailing "consistent with `offset_of!`" reds, so the claim as stated is **refuted** — but the
   sentence is ambiguous enough that two readers split on it, and an ambiguous gate is amended, not
   defended.)*
4. ~~`default_in_place` drop-count test over `{ pod, Nested{pod}, [f32;4] }` (A.8): writes into
   **uninitialized** bytes only; no leak, no double-free; empty struct (`fields: &[]`) is a no-op,
   not a panic.~~ → **A drop-count test over `{ Pod, Nested{Pod}, ArrPack{[f32;4]}, Owned,
   Nested{Owned} }`, where `Owned` carries an instrumented `impl Drop`, and the gate CALLS both
   slots** (D24). Three things were wrong and the first two make it a gate that cannot fail:
   * **Every subject was drop-free.** `Pod`, `Nested{Pod}` and `[f32;4]` have no drop glue by
     construction, so `needs_drop` is false for all three and `drop_in_place` is `None` for all
     three **whether the derive is correct or sabotaged**; the count is identically zero and "no
     leak, no double-free" is unfalsifiable over that set. A.8 — which this gate cites by name —
     prescribes `{ pod, String, Nested{String} }` (`REFLECTION-ANALYSIS.md:1054-1055`); §3.3's row
     (`REFLECTION-PLAN-CORE.md:1220`) silently substituted the drop-free set, and the substitution is what removed the
     instrument. **`Owned { tag: u32 }` with `impl Drop` bumping a thread-local restores it at C7
     without waiting for `Str`** (C11), and it is a *stronger* subject than `String` for this
     property: an exact count separates leak (too low) from double-free (too high), where an
     allocator delta only sees the heap. **A.8's `String` half is C11's**, and C11 already carries
     it — gate 2's *"exactly 1 alloc + 1 free"* and its second red (*"1 alloc, 0 frees — a leak"*);
     the cross-reference is recorded there rather than duplicated here.
   * **Nothing called the slot.** The rung's own *"no install call is emitted at this rung; the
     static exists and is inert"* means a mutated `drop_in_place` would be **written and never
     read** — the dead-datum class, five instances so far. The slots are `pub`
     (`type_info.rs:262,269`), so the gate calls them directly, on a `MaybeUninit<T>` destination
     it owns; it does not need the install seam and must not wait for it. **Asserted counts, both
     directions:** `default_in_place` into uninitialized bytes ⇒ **+0** drops (the double-free
     half), the test's own drop of the finished value ⇒ **exactly +1** (the leak half).
   * ~~empty struct (`fields: &[]`) is a no-op, not a panic~~ is **not a `default_in_place`
     property at this rung** and is retargeted, not deleted. C7 bakes `default_in_place` as
     `ptr::write(p as *mut T, T::default())` — it never reads `fields`, so the clause describes a
     field **walk** C7 does not implement. It is inherited verbatim from A.8, where it belongs
     (`REFLECTION-ANALYSIS.md:1051-1052`, describing a recursive walk). Retargeted: **a fieldless
     struct bakes `fields: &[]` and a working `default_in_place`, and gate 7's `validate` accepts
     it** — the walk's empty case, checked where the walk is. Also: `[f32;4]` is **not a derivable
     subject** — the derive applies to an item — so it is a struct wrapping the array, as
     `WithArray` already is at `c4_prim_zero_alloc.rs:466-468`.
5. Alloc-delta arm for `default_in_place` = 0 bespoke. **In `crates/reflect_fixture/tests/
   c7_alloc_delta.rs`, its own binary, with its own `#[global_allocator]`, arming protocol and
   permanent positive control — a deliberate second instrument, and the reason is load-bearing
   (D23).** The existing harness is `boyko_reflect/tests/c4_prim_zero_alloc.rs:156~`, and its own
   header argues in advance against a second copy (*"three things that would then be free to drift
   apart, for no gain"*, :7~-11) — but the subject here is **derive-generated**, and `boyko_reflect`
   can neither invoke the derive nor compile its output (gate 1's three reasons), so an arm added
   there would measure a **hand-written** `default_in_place` wearing the derived one's verdict:
   exactly the weaker-subject substitution C11 already forbids by name (`REFLECTION-PLAN-CORE.md:3835-3837`). The gain the
   c4 header could not see is therefore real, and it is not the only one: a `#[global_allocator]`
   is one per binary and c4's is `#![cfg(not(miri))]` (`c4_prim_zero_alloc.rs:95~`), so folding this arm into
   `c7_derive_bake.rs` would carry that `cfg` onto **all** of C7's gates and delete C7's derived
   descriptors from the Miri row §7.2 exists to give them (*"the only row that reaches
   derive-generated `unsafe`"*). Separate binary, `#![cfg(not(miri))]` on **this file only**.
6. Tuple-struct fixture: names are `"0"`,`"1"`, over a locally-declared `#[repr(transparent)]`
   tuple struct in `reflect_fixture`. **The subject already exists** — `Handle(pub u32)` /
   `Slot(pub Handle)`, both `#[repr(transparent)]`, `c6_nested_descend.rs:76~-83` — and both fields
   sit at **offset 0** (measured), so this is a naming-and-`TypeKind` gate, never an offset one.
   `ParticleEffectHandle(pub u32)` is the production instance of the shape (hook-bearing,
   `boyko_render`) — it cannot be this gate's subject, because the fixture package must stay
   FFI-free (§0.3) and `boyko_render` reaches `boyko_rhi_vulkan`; it is exercised at ECS EG8, in
   `reflect_dogfood`.
7. **NEW (D25) — `validate` over every derived descriptor.** `validate(<T as Reflect>::TYPE_INFO)`
   is `Ok` for every fixture type in this file. C7 is the **first** rung whose descriptors are
   machine-made, and the rung called the coherence oracle C3 built for exactly this **nowhere** —
   while every hand-baked fixture in the campaign calls it (`c3_type_info.rs:365-372`,
   `c6_nested_descend.rs:281~`, `c6_dogfood_descend.rs:213~`). Gate 1 catches incoherence only where
   the hand-baked oracle happens to cover the slot, and gate 6's tuple subject has no oracle at
   all, so without this a derive emitting `ValueKind::Nested` with `nested: None`, or `Prim` with
   `get: Some(..)` and `set: None`, is green across every other gate and is caught at C8 or later,
   if ever.
8. **NEW (D25) — the walk is index-faithful.** Over `struct Padded { a: u32, _pd:
   PhantomData<u8>, b: u32 }`: `fields.len() == 3`, `fields[1].kind == ValueKind::Opaque` with all
   accessors `None`, and `fields[2].name == "b"`. D14 fixes the never-omit rule (`REFLECTION-PLAN-CORE.md:253`) and
   `c3_type_info.rs:304~-306` records its consequence — *"a shorter list would make by-index access
   depend on which fields were skipped"* — but **no gate could see a violation**, because no C7
   fixture had an unclassifiable field: `Body`/`Placement`/`Point` are all-`Prim`, gate 4's set is
   POD structs, gate 6's is a one-field transparent tuple, and gates 2/3/5 never look at field
   counts. The plan mentions `PhantomData` nine times and every one is `NestedCursor`'s `_pd`,
   never a reflected field.
9. **NEW (D23) — the two obligations landed rungs deferred TO C7, which its table did not carry.**
   (a) **Re-run G3's link-configuration calibration with the annotation in place**, which C2's
   landing note schedules here (`REFLECTION-PLAN-CORE.md:1534`, *"next re-run scheduled at C7"*) and
   [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md):911-914 schedules with its reason (*"at
   CORE C7 the derive's expansion starts referencing some of the crate's symbols, and per-symbol
   decidability inside a pulled object … becomes the census's whole question"*). D26 keeps the
   annotation landing at C7, so the schedule stands. Invocation: `cargo test -p reflect-fixture
   --test reflect_absence_census -- --ignored --nocapture measure_link_configuration_table`
   (`reflect_absence_census.rs:784~-804`) — **nine `--release` builds into per-leg
   `CARGO_TARGET_DIR`s, and a missing `llvm-nm` is a hard RED, not a skip** (:299~-310); budget the
   disk. ~~**And the instrument must be extended in the same change**: it prints six aggregate
   columns, `| link configuration | L1 A | L2 A | L3 A | L1 B | L3 B |` (:787~), with **no L2
   needle-B column** — so "per-symbol sharpness", the property it is being re-run for, is not
   something the table can report today. Add the L2-B column.~~ **DISCHARGED — the column landed,
   and this sentence outlived it.** `reflect_absence_census.rs:787~` now prints **seven** columns,
   `| link configuration | L1 A | L2 A | L3 A | L1 B | L2 B | L3 B |`, with seven separators at
   `:788~` and seven values at `:793~-801`. *(Struck 2026-08-21 at the anchor sweep. The coordinate
   `(:787~)` was re-pointed to this line during the sweep **without the line being read** — so a
   repair whose whole purpose was to stop coordinates from lying moved this one onto a line that
   refutes the sentence around it. That is why the strike records the reason and not just the
   number: the anchor was right and the claim was wrong, which no bounds check can distinguish.)*
   (b) **Re-run G3's census itself**
   after the annotation lands (`OPT_IN_TOKENS` stays valid at C7 per D26 — this is a regression
   check, not a bless).

**RED MUTATION.** Make the derive bake `0` for every offset. Gate 1 reds on every non-first field,
and gate 3 reds. **This red could not fire as the rung first wrote it, and it is cured by the gate-1
and gate-3 amendments rather than by respecifying the mutation** (D23): with gate 1's oracle in a
package that cannot compile the derive, and every *other* named subject one field wide, **every
offset in C7's subject set was zero** — `FixturePod.value`, gate 4's single-field structs, gate 6's
transparent tuple (`Slot.0 = 0`, `Handle.0 = 0`, measured) — so what would have been observed is a
green table. Under gate 1's corrected oracle the mutation has live subjects: **`Body.placement = 4`
and `Placement.layer = 8`, measured on this toolchain**, and gate 3's swapped-order pair adds two
more.

~~*Second red:* make the derive emit `drop_in_place: None` for a type that owns a `String`. Gate 4
reds on the drop count — which is the mutation a naive "all POD" assumption would actually
produce.~~ → **respecified as TWO reds, one per slot (D24), because the original named the wrong
slot, had no subject, and mutated a datum nothing read.** Gate 4 is a **`default_in_place`** test
(§3.3's row, `REFLECTION-PLAN-CORE.md:1220`) and `drop_in_place` is a different `TypeInfo` field
(`type_info.rs:262` vs `:269`); its fixture set owned no `String` and no `Drop`, and a repo-wide grep finds **no `impl
Drop` in `reflect_fixture` or `reflect_dogfood` at all**; and with no install and no call site the
mutated slot was written and never read. The naive "all POD" assumption is still the mutation worth
making — it just needed a subject that can show it:

* *Second red:* emit `drop_in_place: None` for `Owned` (which owns an instrumented `Drop`). **Gate
  4's drop count goes 1 → 0** — a leak, seen as a count that is too low, which is only visible
  because the gate asserts an exact expected count rather than counting calls.
* *Third red:* make `default_in_place` run `drop_in_place` on the destination **before** writing.
  **Gate 4's count goes 1 → 2** on `Owned` — the double-free half, and the reason "writes into
  uninitialized bytes only" is a claim and not a comment.

*Fourth red (gate 7):* emit `nested: None` for a `Nested` field. `validate` reds with
`Violation::NestedWithoutTypeInfo` — the variant already exists (`type_info.rs:325~`), which is why
this gate costs one line.

*Fifth red (gate 8):* filter unclassifiable fields out of the walk. `Padded` bakes two fields
instead of three, every name and offset still **right**, and only `fields[2].name == "b"` and the
length red — which is the whole point: the defect moves the index and touches nothing else.

> **Executed 2026-08-21 (worktree `D:/wt/reflect`, toolchain `stable-x86_64-pc-windows-gnu`
> 1.97.1), on the amended rung. All nine gates green, all five REDs OBSERVED.**
>
> *Lands, file by file.* **`boyko_reflect`:** `src/reflect.rs` — `Reflect` (`const TYPE_INFO:
> &'static TypeInfo`) and `ReflectDefault` (D20's sketch verbatim: the two
> `#[diagnostic::on_unimplemented]` strings, the `Default` supertrait, the blanket impl);
> `src/lib.rs` — the module and its two re-exports, plus its Contents entry.
> **`boyko_macros`:** `src/lib.rs` — `reflect` added to `#[proc_macro_derive(Component,
> attributes(…))]`, and the key's rustdoc, which is where the tuple-struct by-position caveat
> is stated; `src/component.rs` — `ComponentHookPaths::reflect`, its `if
> meta.path.is_ident("reflect")` block with duplicate detection, **both** "valid keys"
> literals, and the two-line splice in `expand`; `src/reflect.rs` (new) — the walk, the kind
> table, and the emission. **`reflect_fixture`:** `tests/c7_derive_bake.rs` (gates 1, 2, 3, 4,
> 6, 7, 8 — 16 tests), `tests/c7_alloc_delta.rs` (gate 5, its own `#[global_allocator]`, 2
> tests), `tests/reflect_absence_census.rs` (gate 9a's **L2 needle-B column** + the
> re-calibrated table), `src/bin/reflect_on.rs` (the annotation **beside** the linkage, D26),
> `src/bin/reflect_never.rs` and `src/bin/reflect_off_twin_plus.rs` (`#[derive(Default)]`, and
> the plus-copy tracks the annotation — see the deviations). **`boyko_ecs`:**
> `tests/compile_fail_hooks/{on_despawn,unknown_key}_rejected.stderr` re-blessed.
>
> *Every gate ran under the invocation D23 makes part of the gate*, and the vacuity claim was
> checked rather than trusted: `cargo test -p reflect-fixture --test c7_derive_bake` **without**
> `--features reflect-fixture/reflect` prints `running 0 tests` and **exits 0**. Both C7 files
> would have reported a green table having compiled every gate away.
>
> **The observer ran before any code existed, and reported exactly the three absences the audit
> named** — `unknown #[component(...)] key`, `no Reflect in the root` / `no ReflectDefault in
> the root`, and `cannot find attribute reflect in this scope`, D22's three blocking findings,
> observed rather than taken on the ruling's word. **A fourth toolchain refusal turned up while
> building, and it is compiled rather than argued:**
>
> * **`std::any::type_name` is NOT const on 1.97.1** — *"error: `std::any::type_name` is not
>   yet stable as a const fn"*, compiled. `TypeInfo.type_name` is a baked `&'static str`, so
>   the derive emits `concat!(module_path!(), "::", stringify!(T))` — which is the shape every
>   hand-baked static in this campaign already writes (`"c6_nested_descend::Point"`), and the
>   slot is diagnostics-only, never a save key (D8). Gate 2 pins the result
>   (`"c7_derive_bake::Body"`) rather than leaving the substitution unstated.
>
> *Five decisions taken here and recorded rather than worked around:*
>
> 1. **`ScalarKind::EntityId` is deliberately NOT classified**, and it is the one kind where
>    that is a safety call rather than a scope call. Every other `Prim` name is a Rust
>    primitive; `EntityId` is an ordinary identifier a consumer crate may also define, so a
>    name match would install `prim::get_entity_id` on a same-named foreign type and read its
>    first four bytes as a slot index — the silent-garbage class (analysis FIX Mi2) the whole
>    model exists to refuse. No C7 gate covers it either. An `EntityId` field therefore falls
>    to the `Nested` arm and fails to compile with a trait bound naming the type. **A wrong
>    `Nested` is a compile error; a wrong `Prim` is a wild read** — the arm order is chosen for
>    which way each mistake fails.
> 2. **`Nested` is the fallback for a bare argument-free path, and `Opaque` is the fallback for
>    everything else**, for the same asymmetry. A generic argument is what separates the
>    taxonomy's standard indirections (`Vec<T>`, `Box<T>`, `Option<T>`, `PhantomData<T>`) from
>    a plain nested struct, and a non-path type is never nested-by-value. The proxy cannot be
>    exact — no syntactic list names `struct MyBox<T>(*mut T)` — which is exactly why
>    `validate`'s two structural checks, and not this table, are the soundness proof (D21).
>    Consequence, stated because it is visible to the first user: a `String` field is a bare
>    path, so at C7 it reads as `Nested` and fails the `Reflect` bound. `ValueKind::Str` is
>    **C11's** `Lands`, and C9's matrix is where the diagnostic becomes spanned.
> 3. **A non-struct shape bakes `TypeKind::Opaque` with `fields: &[]`.** `#[component(reflect)]`
>    on an enum is C10's (`TypeKind::Enum` + an `EnumInfo`), and the model already has a word
>    for *"a type the model cannot describe"*. The rejected alternative was inventing a spanned
>    refusal at C7: C9 gate 2's anti-rot census counts `.stderr` fixtures against a `const
>    REFUSALS` the derive iterates, so a refusal landed outside that table is structurally
>    invisible to the census that keeps refusals honest.
> 4. **`drop_in_place` is decided by `const fn needs_drop::<T>()` inside the static's
>    initializer, not by a syntactic guess.** `if needs_drop::<T>() { Some(..) } else { None }`
>    is const-evaluable, so the type system answers a question the macro cannot see. This is
>    what makes the second RED — the naive *"all POD"* assumption — a mutation of one arm
>    rather than a rewrite, and it is why no fixture had to declare its own drop-ness.
> 5. **The emitted `__reflect_default_in_place` is GENERIC over `ReflectDefault`, and that was
>    measured.** Written monomorphically it produced **two** errors for a type with no
>    `Default`, and the **first** was the bare *"the trait bound `T: Default` is not
>    satisfied"* from the helper's own body, with D20's named message second. D20's promise is
>    that such a type *"fails with `ReflectDefault`'s message"*; leading with the anonymous one
>    honours it only in the sense that the good message is somewhere in the output. Taking the
>    bound through `ReflectDefault` (whose supertrait supplies `Default` inside the body)
>    leaves one obligation per site and **both** errors then carry the named message, spanned
>    at the user's type name. D20's `const _: fn() = …` witness is kept verbatim; only the
>    helper's signature changed.
>
> *Gate 9a's re-calibration produced the rung's most useful measurement, and it refuted the
> note this record first carried.* The new **L2 B** column reads **1** in every row — the
> `install_type_info` reference `reflect_linkage()` puts there, i.e. exactly the number D26
> refused to let go silently to 0. L2 **A** moved **6 → 41** in the two non-LTO rows and
> **5 → 5** under fat LTO, and the obvious reading — *"the derive put ~35 symbols there"* — was
> written down, then checked, and is **false**: rebuilding the identical leg with the
> annotation commented out gives **41 as well** (same `CARGO_TARGET_DIR`; source restored
> byte-identically). Two facts, both worth more than the number:
>
> * the 6 → 41 growth is **C3–C6's**, not C7's — the calibration was last run at C2, and the
>   crate has since gained `prim`'s 24 accessors, `array_get`/`array_set`, `validate` with
>   `walk_nested`/`exhaust`, and the `Violation`/`Problem` fmt impls with their `Vec<Problem>`
>   `RawVec` instantiations. One referenced symbol still pulls the whole object; the object got
>   bigger;
> * **needle A structurally cannot see the derive's emission at all.**
>   `__REFLECT_TYPE_INFO` / `__REFLECT_FIELDS` / `__reflect_type_id_of::<T>` are defined in the
>   **consumer** crate, so v0 mangling names *`reflect_on`* as their defining crate. The image
>   in fact contains **zero** symbols matching those names in any row: at C7 the descriptor is
>   referenced by nothing and is dropped before the linker sees it — which is C7's own *"the
>   static exists and is inert"* with an artifact-level witness, and why the fat-LTO row did not
>   move. Whether the emission reached the image is **G6b's** question, and from C8 it becomes
>   needle B's.
>
> *RED ledger — every mutation applied to `crates/boyko_macros/src/reflect.rs`, its failure
> OBSERVED, the source restored byte-identically (`cmp`, and the SHA-256 re-checked after the
> last restore).*
>
> | red | mutation | OBSERVED |
> |---|---|---|
> | 1 | every `offset` baked `0usize` | 4 red / 12 green, exit 101. Gate 1: *"`c7_derive_bake::Body` field #1 `placement`: derived offset 0 != oracle offset 4"* — **the non-zero subject the amendment created**; gate 3, the round-trip clause and gate 8's `offset_of!` clause red with it |
> | 2 | `drop_in_place` always `None` | 2 red / 14 green, exit 101. Gate 4: *"the finished value cost 0 drop(s) through `drop_in_place`, not exactly 1"*, `left: 0 right: 1` — the **1 → 0 leak**, visible only because the count is exact |
> | 3 | `default_in_place` drops the destination first | 2 red / 14 green, exit 101. Gate 4: *"`default_in_place` ran drop glue on the DESTINATION"*, `left: 1 right: 0` — the double-free half, caught at the `+0` clause (earlier and more precisely than the 1 → 2 total) |
> | 4 | `nested: None` on a `Nested` field | 4 red / 12 green, exit 101. Gate 7: *"derived `c7_derive_bake::Body` is INCOHERENT: field #1 `placement`: NestedWithoutTypeInfo"* — the predicted variant, by name |
> | 5 | `fields.iter().filter(…)` — the natural walk shape | **1 red / 15 green**, exit 101. Gate 8 alone: *"the walk dropped a field it could not classify"*, `left: 2 right: 3`. Every other gate green, which is D25's argument measured: nothing else in the table can see an index shift |
>
> *A sixth red, self-imposed, because the rung's `Lands` carried `#[reflect(no_default)]` and
> its gate table carried no clause for it.* Deleting the opt-out from `NoDefaultAtAll` (a type
> with no `Default`) is exit 101 with **two** `E0277`s, both reading *"`#[component(reflect)]`
> bakes `default_in_place` from `Default`, and `NoDefaultAtAll` does not implement it"* with
> the label *"add `#[derive(Default)]`, write an impl, or opt out with
> `#[reflect(no_default)]`"*, both spanned at `pub struct NoDefaultAtAll` — D20's message,
> verbatim, at the user's own item. The green-side clause
> (`no_default_opts_out_of_the_default_slot`) is a gate only because this red fires: the
> subject compiles **at all** solely because the opt-out suppresses the witness.
>
> *Deviations from the amended rung, both consequences of the landing rather than choices:*
>
> 1. **`src/bin/reflect_off_twin_plus.rs` gained the annotation and `#[derive(Default)]`, and no
>    rung's list carried it.** D26 named `reflect_on.rs` and `reflect_never.rs`; the plus-copy's
>    own header states its whole contract — *"a copy of `src/bin/reflect_on.rs` (the twin's
>    source) plus the marker"* — and GATES G7a's table repeats it (*"`reflect_off_twin` plus one
>    `#[inline(never)] fn`"*). Left untracked, G7a's positive control would differ from its
>    baseline by the marker **plus a `Default` impl**: a delta that is still non-zero, so the
>    control would keep reporting a pass while no longer measuring what it says it measures.
>    G7a's harness does not exist yet, which is exactly why this had to be caught by reading
>    rather than by a red.
> 2. **Two `.stderr` fixtures in `boyko_ecs` re-blessed, and the update was in no list.** The
>    `reflect` key is appended to both "valid keys" literals (C7's `Lands` says so), and
>    `compile_fail_hooks/on_despawn_rejected.stderr` and `unknown_key_rejected.stderr` pin those
>    strings byte-for-byte. Observed red first (`cargo test -p boyko-ecs --test
>    compile_fail_hooks`, exit 101, two MISMATCHes whose whole delta is `, reflect`), then
>    re-blessed by a targeted textual edit rather than `TRYBUILD=overwrite`, so a second
>    unrelated drift could not have been swept in with it. This is the `OPT_IN_TOKENS` shape
>    one level down: **the rung changes a diagnostic, and a corpus somewhere pins that
>    diagnostic.**
>
> *Regression, all exit 0, all `running [1-9]`.* `c7_derive_bake` **16** · `c7_alloc_delta`
> **2** (positive control: `deliberate allocations observed = 1`) · `c6_nested_descend` **4** ·
> `reflect_absence_census` **1** (+1 ignored calibration, re-run separately, 9 release builds) ·
> `reflect_leg_nonvacuity` **1** · `boyko-reflect` whole suite **76** across 9 binaries ·
> `reflect-dogfood` feature-on **4** · all **ten** `boyko_ecs` trybuild corpora
> (`compile_fail_hooks` **1**, `bundle_compile_fail` **1**, `codegen_reject_relations_query`
> **2**, `compile_fail_chunk` **2**, `compile_fail_dispatcher_token` **1**, `compile_fail_local`
> **1**, `compile_fail_observers` **1**, `compile_fail_relations` **1**, `compile_fail_require`
> **1**, `compile_fail_zero_init` **1**) · root censuses `internal_docs_anchors` **5**,
> `engine_packages_census` **3**, `trybuild_corpus_compiler_witness` **2**,
> `reflect_manifest_census` **7**, `reflect_ship_closure` **2**, `reflect_ci_coverage` **6** ·
> `cargo check --all-targets` for `boyko-ecs`, `boyko-scene`, `boyko-ui`, `reflect-dogfood` ·
> `cargo clippy --all-targets -- -D warnings` (touch-first) for `boyko-macros`,
> `boyko-reflect`, and `reflect-fixture --features reflect-fixture/reflect`.
>
> **And §7.2's Miri row was exercised, which is the half D23 bought by keeping the allocator
> out of `c7_derive_bake.rs`.** `cargo miri test -p reflect-fixture --features
> reflect-fixture/reflect --test c7_derive_bake` is **16 passed**, exit 0 — the derive's
> generated `unsafe` (`ptr::write` into a `MaybeUninit`, `ptr::drop_in_place`, the raw
> `base.add(offset)` accessor calls) under Tree Borrows, on the shape a consumer actually gets.
> Folding gate 5's `#[global_allocator]` into that file would have carried
> `#![cfg(not(miri))]` onto all sixteen and left this row measuring nothing;
> `--test c7_alloc_delta` under Miri is `running 0 tests`, by design and as intended.
> `c6_nested_descend` under Miri: **4 passed**, unchanged.
>
> **Not run, and named rather than implied:** no `--workspace` build (disk budget), and no test
> suite for the render / physics / app crates. The blast-radius argument for that is not "it
> should be fine": the derive's new branch is gated on `hooks.reflect`, which no crate outside
> `reflect_fixture` sets, so for every other `#[derive(Component)]` in the tree the expansion is
> byte-identical — and the two things that *did* change unconditionally are the
> `attributes(…)` list (additive; `reflect` resolves where it did not before, and is inert) and
> the two diagnostic strings, whose only pins in the tree are the two `.stderr` fixtures above,
> found by `grep -rl "valid keys" --include=*.stderr` and re-blessed.

#### C7 follow-up (2026-08-21) — one gate that could not fail, and two arms nothing covered

> An adversarial verify of the landing above confirmed every gate and every RED reproduced. It
> also found **a gate that cannot fail** and two coverage gaps. All three are closed here, each
> under the observer-before-gate protocol; the derive source was restored byte-identically after
> every mutation (`cmp` + sha256 recorded in the session).
>
> **1. `a_types_descriptor_has_exactly_one_address` could not fail, and its stated reason was
> wrong.** Substituting `const` for both `static`s in `boyko_macros/src/reflect.rs` leaves
> `c7_derive_bake` at **16 passed, exit 0** — including that test and gate 6's two `ptr::eq`
> clauses. The property is real; the **subject set** was wrong, and the wrong reason is why the
> wrong subject looked sufficient. A `const` is *not* re-promoted per `&`-site here (the
> expansion has exactly one), so the within-crate address is stable and every same-crate check is
> structurally blind. The divergence is at the **crate boundary**, where each evaluating crate
> interns its own copy of the anonymous const-evaluated allocation.
>
> The falsifying subject needs an annotated type **defined in a library** and read from a
> consumer; `reflect_fixture` has no `src/lib.rs`, so `reflect_dogfood` is the only package in
> the workspace that can host it. **Lands:** `reflect_dogfood/src/address.rs` (new — `ProbeLeaf`,
> `ProbeRoot`, and two `#[inline(never)]` readers; the attribute is the instrument, since a
> cross-crate-inlined reader would report the *consumer's* materialization and compare one side
> against itself), `src/lib.rs` (the module, and its header no longer claims the crate is empty),
> and `tests/c7_cross_crate_address.rs` (new — **3 tests**). RED observed: `ProbeLeaf` at
> `0x7ff739cff5c8` in the defining crate against `0x7ff739cf26a0` in the consumer.
>
> **The second clause of that file was itself rewritten after a measurement refuted its first
> form.** Comparing `<ProbeRoot>::TYPE_INFO.fields[0].nested` against `<ProbeLeaf>::TYPE_INFO`,
> both read in the consumer, **cannot fail**: a `const` descriptor is re-materialized *whole*, so
> a graph read entirely from one side is internally self-consistent however many copies exist.
> The edge must come from the **defining** crate's reader — which is the pairing C8's install seam
> actually produces. Written the naive way first, observed green under the mutation, corrected.
>
> **The wrong reason was recorded at five sites, and all five are corrected in this change** —
> `boyko_macros/src/reflect.rs`, `boyko_reflect/src/reflect.rs`,
> `reflect_fixture/tests/c7_derive_bake.rs`, and this document twice (§D22's "Third part", and
> C7's emission bullet). The two the verifier did not name were found by `grep`, because a repair
> that fixes the sentences it was handed and not the paragraphs arguing for them is how this tree
> has introduced fresh rot before. The `c7_derive_bake` test is **renamed**
> `a_types_descriptor_has_exactly_one_address_within_this_crate` and keeps its same-crate clause,
> with its doc now stating what it cannot see.
>
> **Why this was urgent rather than tidy:** C7 is the first rung whose output can break a **C6**
> obligation. Check B identifies types by address, so two addresses per type silently stop its
> cycle detection and its memoization while `validate` goes on returning `Ok` — and it goes live
> at **C8**'s install seam and **ECS EG8**, both of which read a descriptor from a crate other
> than the one defining it. The failure would first appear two rungs downstream of its cause.
>
> **2. The `L2 B` datum was computed and never asserted.** The column C7 added lives only in the
> `#[ignore]`d `measure_link_configuration_table`, which prints and asserts nothing, while the
> asserting gate computed `l1_a, l1_b, l2_a, l3_a, l3_b` — no `l2_b`. Nothing here was a false
> claim, but D26's entire purpose was to stop *"a probe losing its subject in silence"*, and this
> is the campaign's **dead-datum class** (a sixth instance). `l2_b > 0` is now a clause of
> `reflect_absence_census_three_legs_under_fat_lto`, placed beside `l2_a > 0` because it plays the
> same role: it is the **present control for needle B**, without which L1 B = 0 and L3 B = 0 are
> indistinguishable from *"the needle matches nothing anywhere"*.
>
> `> 0` and not `== 1`, deliberately: the measurand is presence, and the header's own table shows
> a count moving 6 → 5 when fat LTO strips an `__imp_` thunk, so an equality would red on the
> linker's bookkeeping instead of on the property. **Stated honestly at the site: the clause is a
> CONSTANT until C8.** What puts `install_type_info` in the L2 image today is the temporary
> `reflect_linkage()`, whose presence the same test's `OPT_IN_TOKENS` clause already requires — so
> at C7 the two cannot disagree. Its worth is at C8, which deletes the linkage and must replace it
> with the derive's install call; this is the clause that reds if the replacement does not arrive.
> RED observed by repointing `reflect_linkage()` at `boyko_reflect::type_info_of` — which keeps
> both `OPT_IN_TOKENS` in the source and keeps `l2_a > 0` green, so the new clause is the only
> thing that fires: *"NOT RESOLVED (needle B inert) … L2 B = 0, measured 1 in every link
> configuration at C7"*.
>
> **3. The derive's non-struct arm shipped with zero coverage.** All fifteen entries of
> `every_derived_descriptor()` were structs, so `codegen`'s `Data::Enum(_) | Data::Union(_)` arm
> was constructed by nothing. Measured: an annotated enum **compiles**, bakes `TypeKind::Opaque`
> with `fields: &[]`, and `validate` returns `Ok`. The implementer's reason for accepting rather
> than refusing holds — a refusal outside C9's `const REFUSALS` would be invisible to the census
> that keeps refusals honest — but the arm's output is a *coherent descriptor asserting that a
> type with two payload variants has no fields*, and a coherent lie is what these gates exist to
> catch. **Lands:** the `NonStruct` subject (payload variants deliberately, so `fields: &[]` is a
> substantive claim), `the_non_struct_arm_bakes_an_opaque_fieldless_descriptor`, and `NonStruct`
> added to `every_derived_descriptor()` (now **16** entries) so gate 7's `validate` sweep reaches
> the arm. RED observed: `Opaque` → `Struct` in that arm reds the new test **and nothing else** —
> 16 of 17 still green, which is the measured proof that no other gate covered it.
>
> **4. Field-level `#[reflect(...)]` — verified, no code change.** `parse_reflect_no_default` is
> called from exactly one site, ~~`boyko_macros/src/component.rs:165~`~~ → `:180~` → **`:182~`** (re-derived again after C9 landed;
> still exactly one site), with `&input.attrs`; field attributes are
> never passed, so a field-level `#[reflect(anything)]` is inert while the same unknown key at
> type level errors. This **is** already documented at the site (`reflect.rs`'s
> `parse_reflect_no_default` rustdoc names D14's future `skip` and says a field-level
> `#[reflect(...)]` "is inert here rather than an error, because the registration is what makes it
> resolve and C9 owns what it means"). Left alone, as briefed. The one thing the doc leaves the
> reader to derive rather than saying outright is the *asymmetry* with the type-level error.
>
> *Follow-up regression, all exit 0, all `running [1-9]`:* `c7_derive_bake` **17** ·
> `c7_alloc_delta` **2** · `c7_cross_crate_address` **3** · `reflect_absence_census` **1**
> (+1 ignored calibration) · `reflect_leg_nonvacuity` **1** · `boyko-macros`, `boyko-reflect`,
> `reflect-fixture --features reflect-fixture/reflect` and `reflect-dogfood --features
> reflect-dogfood/reflect` full suites · root censuses `reflect_manifest_census`,
> `reflect_ship_closure`, `reflect_ci_coverage`, `internal_docs_anchors`,
> `engine_packages_census`, `trybuild_corpus_compiler_witness` · clippy `-D warnings`
> (touch-first) on every edited crate. **No manifest edit was needed:** `reflect_dogfood` does not
> set `autotests = false`, so the new integration test is auto-discovered, and no census keys on
> that package's target table — checked rather than assumed, this campaign having been bitten
> twice by a manifest edit no rung's list carried.

---

### C8 — The install seam: the seventh slot in `component_id()`

> **LANDED 2026-08-21**, under the rung protocol (observer before gate, gate before edit).
> Every gate ran green with an unpiped exit code and `running [1-9]`; every RED was applied,
> its failure OBSERVED, and every source restored byte-identically (`cmp`). What the
> execution ADDED to the rung as written, each recorded at its site below:
>
> * **Gate 4 needed a positive control and did not have one.** *"For a bitset tag,
>   `type_info_of` is `None`"* is green today, green before C8, and green if the whole
>   install seam were deleted — nothing is installed for anything in that world. The gate
>   now reads a non-bitset sibling in the same pass, and OBSERVED that clause red on the
>   pre-C8 tree. Without it this rung would have shipped a gate that could not fail.
> * **`OPT_IN_TOKENS` could not simply "become the annotation form".** The same list is
>   asserted **present in L2**, and post-retirement `reflect_on.rs` spells
>   `boyko_reflect::` nowhere — the literal reading reds the census on the change that
>   lands the rung. The list split: the annotation in `OPT_IN_TOKENS`, the crate path in a
>   new `L3_FORBIDDEN_TOKENS` (forbidden in L3, not required in L2). Edit-list item 3 named
>   the swap and not this.
> * **Item 9's red fired from prose written to prevent it.** The first draft of
>   `reflect_never.rs`'s new warning paragraph spelled the attribute it was warning against,
>   and the census failed on it. Recorded at both sites.
> * **Red 2 fires on `l2_a`, not `l2_b`.** Deleting the funnel touch takes needle A and
>   needle B to zero together (D27 says so; C8's RED text names only `l2_b`), and `l2_a > 0`
>   is asserted first, so it is the clause that reports. The red is real and stronger than
>   written.
> * **The calibration's numbers, recorded from the run:** L2 needle A `5 → 7` under fat LTO
>   (the two additions are `prim::get_f32` / `prim::set_f32`, read off `llvm-nm`), the
>   consumer-side `__REFLECT_*` symbols `0 → 2` in L2 and `0` in L1/L3, and `component_id`
>   `0 → 4` in **every** image — the last being what makes L1's zero mean the feature gate
>   rather than an uncalled funnel. Full table in
>   `reflect_fixture/tests/reflect_absence_census.rs`'s header and GATES §G3.
>
> **This note was written before the rung was audited, and the audit refuted four of its
> claims — one of them a gate this rung itself made unable to fail.** All four are closed
> in the follow-up below, which is part of C8 and not a later rung; read it before
> treating anything above as final.

**Lands.** In `boyko_macros`, beside the six existing install slots (F7):

```rust
#reflect_install     // emitted iff #[component(reflect)]; wrapped in #[cfg(feature = "reflect")]
```

~~expanding to `boyko_reflect::install_type_info(raw, <Self as boyko_reflect::Reflect>::TYPE_INFO);`.~~
→ **corrected 2026-08-21 (C8 audit): the paths are ABSOLUTE.**

```rust
::boyko_reflect::install_type_info(raw, <Self as ::boyko_reflect::Reflect>::TYPE_INFO);
```

*Why, and why it is a decision rather than an accident:* **every** other path the reflect emission
puts into a consumer crate is already absolute — `::boyko_reflect::FieldInfo` (`boyko_macros/src/reflect.rs:501~`),
`<#field_ty as ::boyko_reflect::Reflect>::TYPE_INFO` (`:560~`), `impl ::boyko_reflect::Reflect`
(`:863`), `::boyko_reflect::ReflectDefault` (`:778~`, `:804`); a grep for a non-absolute
`boyko_reflect::` in that file returns **only doc comments**. A bare first segment inside
`component_id()`'s body resolves through the consumer's own scope before the extern prelude, so a
consumer `mod boyko_reflect` or `use x as boyko_reflect` shadows it. The six existing funnel slots
use the non-absolute `boyko_ecs::…` form and matching them would be defensible — but this is the one
path whose **absence in a ship build is the campaign's central claim**, and the sibling emission has
already closed the hole. Absolute wins.

No `IS_REFLECT` const (D7). `boyko_macros` gains **no dependency** on `boyko_reflect` (D17). *(The
`Reflect` trait this consumes is landed at C7, in `boyko_reflect` — D22; it existed in no crate when
this line was written.)*

**Also lands: the bitset suppression (D29).** The emission's condition becomes
`hooks.reflect && !hooks.storage_bitset`, joining the six neighbours ~~at `boyko_macros/src/component.rs:123~`,
`:144~`, `:239~`, `:263~`, `:315~`, `:334~`. Today `:164~` gates on `hooks.reflect` alone~~ and
`#[component(reflect, storage = "bitset")]` compiles and bakes a zero-field descriptor —
**measured**, see D29. C9 adds the spanned message; C8 stops the install.

> **Anchors corrected 2026-08-21, after the landing that rotted them — every number re-derived by
> reading the file, not by trusting the previous text.** *"Today `:164~` gates on `hooks.reflect`
> alone"* was true of HEAD `bf7803d6` and is false of the tree this rung leaves: the condition is
> **`:178~`**, `let reflect_enabled = hooks.reflect && !hooks.storage_bitset;`, and it carries the
> term. The six neighbours are now **`:123~`, `:144~`, `:254~`, `:278~`, `:330~`, `:386~`** — only
> `entities_items` and `serialize_items` kept their numbers; `clone_install`, `relationship_install`,
> `serialize_install` and `bundle_items` each moved. **Cite them by binding name.** This is the same
> defect F7's row records, in the same rung, and it is invisible to every check the tree has: the
> `REFLECTION-PLAN-*.md` documents are not in `internal_docs_anchors.rs`'s `GATED_DOCS`.

**Also lands: the funnel is TOUCHED (D27), or the seventh slot reaches no image.** `main()` gains
`core::hint::black_box(<FixturePod as Component>::component_id());` — plus the `Component` import —
in **`reflect_on.rs`** (shared by L1 and L2 through the `[[bin]]` table), **`reflect_off_twin_plus.rs`**
(contract: *"the twin's source plus exactly one fn"*) and **`reflect_never.rs`** (contract: *"nothing
else about the two shapes may diverge"*). This is the obligation D26 assumed and did not carry;
without it, deleting `reflect_linkage()` reds **two** census clauses instead of none.

**And C8 retires G0's linkage deviation — moved here from C7 by D26.** `src/bin/reflect_on.rs`
carries `#[component(reflect)]` **beside** `reflect_linkage()` from C7 onward; C8 is the rung whose
install slot puts the literal name `install_type_info` — G3's **needle B** — back into the L2 image,
so C8 is where the linkage can be deleted without voiding the probe. ***Conditional on D27, added
2026-08-21:* that sentence is true only once the funnel is touched. Un-touched, `component_id()` is
never called, the slot is dropped before the linker sees it, and deleting the linkage reds `l2_a > 0`
and `l2_b > 0` both. Measured; see D27.**

**In the same change — the full edit list, re-derived at the C8 audit because the previous list was
short by four live sites:**

1. Delete `reflect_linkage()` from `reflect_on.rs` (`reflect_on.rs:49~-50` call, `:64~-72` fn) **and** from
   `reflect_off_twin_plus.rs` (`:40-41` call, `:59-65` fn incl. its doc line *"Same linkage as <!-- doc-anchor-ignore -->
   `reflect_on`"*).
2. Add the D27 funnel touch to `reflect_on.rs`, `reflect_off_twin_plus.rs` **and** `reflect_never.rs`.
3. `reflect_absence_census.rs:371~`'s `OPT_IN_TOKENS`: `["reflect_linkage", "boyko_reflect::"]` →
   the annotation form.
4. That file's `OPT_IN_TOKENS` doc comment (`:365~-370`) and the twin-source assertion's failure text
   (`:396~-402`).
5. The `l2_b > 0` clause — comment `:453~-474`, assertion `:475~-484` (*the earlier citation
   ":461~-483" was a sub-range of the block to be edited*). It names C7 and C8, and C8's landing is
   what converts it from a constant into a live check; its failure text already says so.
6. **NEW — the file's module header, `:17~-28`.** It is the census's most explicit present-tense
   description of the pre-C8 state (*"C7 landed the key and `reflect_on.rs` now carries **both**…
   `OPT_IN_TOKENS` below and its assertion's failure text still say 'C7' and are **deliberately
   left**: C8's `Lands` carries them"*). After C8 both sentences are false, and the second one
   describes a list it was not on.
7. **NEW — the header's C7 calibration block, `:90~-137`,** which ends *"Re-run this calibration at
   C8."* The re-run is C8's, and its new `L2 B` / `L2 A` cells are **recorded from the run, never
   predicted here**.
8. **NEW — `reflect_on.rs:30~-31`,** the `FixturePod` item doc: *"BESIDE the linkage below rather
   than instead of it (D26 …)"*, which names a linkage that will no longer exist. The previous list
   accounted only for the bin *headers*.
9. **NEW — `reflect_never.rs:19~`, and it reds the census if it is not touched.** The L3
   non-collision clause (`reflect_absence_census.rs:383~-393`) scans that file's **source text** for any `OPT_IN_TOKENS`
   entry, and `reflect_never.rs:19~` contains the literal `#[component(reflect)]` inside a rustdoc line (*"is the
   `#[component(reflect)]` key and the linkage fn, both of which this file lacks"*). The instant
   `OPT_IN_TOKENS` becomes the annotation form, that substring matches and the census fails claiming
   the linked-unused discriminator has collapsed into the present control — **a red caused by the
   instrument's own prose.** Reword the line to name the key without spelling it, and add a sentence
   at the assertion site saying so: **L3's source may not quote the opt-in tokens, in prose or in
   code.**

**The two bin headers were corrected in place at the C7 ruling** (`reflect_on.rs:10~-20`,
`reflect_never.rs:1~-11`). ~~`reflect_off_twin_plus.rs` names no rung and needed none.~~ →
**corrected at the C8 audit: that is true of its header (`reflect_off_twin_plus.rs:8~-11`) and false of the file** — `:18~-25`
reads *"Tracked at CORE C7"*, naming C7 as the rung that ADDED the annotation, which stays true and
needs no edit. Its **linkage** and the sentence describing it are item 1 above. See D26's strike.
The census's own failure text is the instruction; the earlier plan's omission was that no rung's
regression list carried it.

**Gate.**
1. ~~**Feature ON:** `type_info_of(T::component_id().0)` is `Some` on the **first** touch of
   `component_id()`, before `T` can enter any archetype.~~ → **respecified 2026-08-21 (D28):
   `is_some()` is blind to both of the seam's characteristic failures.** **Feature ON**, on the
   **first** touch of `component_id()` and before `T` can enter any archetype, over **two distinct
   subjects** defined in `reflect_dogfood`'s library and read from its test — `ProbeLeaf` /
   `ProbeRoot` (`src/address.rs:66~-84`), the rig `c7_cross_crate_address.rs` already uses, so no new
   package or fixture is needed:
   `ptr::eq(type_info_of(T::component_id().0).expect(…), <T as Reflect>::TYPE_INFO)`,
   with the two ids asserted **distinct** as a stated instrument precondition. This is also where
   C7's *"one stable address per type … goes live at **C8**'s install seam"* obligation is
   discharged — it was scheduled at this rung and this rung's own list did not carry it.
2. **Feature OFF, in a crate whose resolved graph does not have the dep at all:**
   `#[component(reflect)]` compiles. This is the whole D1/D2 mechanism and it is a compile, not an
   argument. The subject is `reflect_off_twin` — `reflect-fixture` built feature-off, where
   `boyko-reflect` is not in the graph (`reflect_fixture/Cargo.toml:32~,39`). *(Wording tightened at the C8 audit:
   no crate in the tree lacks the dep **unconditionally** while carrying the key; every annotating
   crate declares it `optional`. The resolved-graph form is what gate 2 tests, and it is a real
   subject.)* This is **G6a**, and GATES states its strength: *"The fixture compiling **is** the
   proof … it cannot be satisfied by accident."*
3. ~~**Token absence, feature OFF** — **the instrument is
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
   > GATES ledger; this gate asserts whatever G6b selected.~~
   >
   > **STRUCK 2026-08-21 (C8 audit, D31) — the deferral leads to no instrument, and its escape
   > clause is false in direction.** G6b has selected no form, has no ledger row, and has no
   > implementing file; every `G5`/`G6`/`G6c`/`G7a`/`G7b`/`G8` ledger row reads `— | —`, which that
   > appendix defines as *not landed*. So *"asserts whatever G6b selected"* resolves to asserting
   > nothing at C8 — while C8 declares both its reds required. And the `compile_fail` candidate
   > **cannot** start compiling under the mutation: with the feature off `boyko-reflect` is not in
   > `reflect-fixture`'s resolved graph, so the fixture's own `boyko_reflect::` path is `E0433`
   > whatever the derive emits. **Gate 3 is struck from C8** and its content re-homed: the path half
   > IS gate 2 (= G6a), and the residue half — *a residue that compiles and names nothing* — is
   > **G6c's**, in G6c's own words. C8 owns neither instrument and needs neither. GATES is corrected
   > in the same pass. *(Measured alongside: `cargo tree --workspace -e normal` prints
   > `boyko-reflect` once, at column 0 as its own member root, at zero dependency edges.)*
4. ~~**Bitset suppression:** `#[component(reflect, storage = "bitset")]` is a **derive error**, not
   a silent skip (C9 owns the message; this gate owns "the install is not emitted").~~ →
   **respecified 2026-08-21 (D29): the two halves excluded each other, and the strong one is C9's
   own table row.** **Bitset suppression:** for a type carrying `#[component(reflect, storage =
   "bitset")]`, **no install is emitted** and `type_info_of(T::component_id().0)` is `None`. Not a
   derive error at C8 — C9 owns the message and, with it, ECS D5's release `assert!`. The subject is
   one unit struct this rung creates. *(Measured at the audit: the combination compiles today and
   bakes `size=0 align=1 fields=0 kind=Struct`, and `boyko_macros/src/component.rs:164~` — the pre-landing anchor;
   **`:178~` after C8, where the term now is** — is the one slot in its neighbourhood with no
   `storage_bitset` term.)*
5. ~~**Horn-2 drift test (D16):** for every type carrying both `#[derive(Bindable)]` and
   `#[component(reflect)]`, `<T as Bindable>::field_id(name)` and the reflect field index
   **agree**, for every field name. Feature-on only.~~ → **respecified 2026-08-21 (D30): the subject
   set was empty tree-wide, no listed package could host one, and D14 makes source-level drift
   impossible by construction.** **Horn-2 generator-agreement test (D16).** Over a subject in
   `reflect_dogfood` (which gains `boyko-ui` as a **dev**-dependency — one line, test-only, in no
   ship closure) carrying both derives, feature-on only, under the existing `reflect-dogfood` CI
   leg: `<T as Bindable>::field_id(name)` equals the reflect field index **for every name in the
   reflect descriptor**, *and* `<T as Bindable>::FIELD_COUNT as usize == TYPE_INFO.fields.len()` —
   the second clause because `Bindable` exposes no name enumeration, so a name reflect stopped
   emitting is invisible to the first. Its RED is a **generator** mutation, not a source edit; see
   gate 5's red below.
6. ~~The `component_id()` funnel's existing six slots are unperturbed: a type **without**
   `#[component(reflect)]` expands byte-identically to its pre-rung expansion.~~ → **MOVED to
   GATES G6c, 2026-08-21 (D31).** A byte comparison of an *expansion* is the same nightly-only
   measurement gate 3 spent nine lines deferring, left uncorrected one bullet later with no deferral
   of its own; and `boyko_macros` is `[lib] proc-macro = true`, so `component::expand` cannot be
   called from a test the way `aether_lang`'s `expand_block` snapshot channel can. G6c's measurand
   IS this property in a form stable Rust can take — `reflect_off_twin` (annotated, feature off) vs
   `reflect_never` (un-annotated), same symbol multiset and same `.text`, **concurrent**, needing no
   stored temporal baseline; after D27 both legs carry the funnel touch, so the comparison is exactly
   *"does the seventh slot's existence perturb the other six"*. **C8 does not need it:** for an
   un-annotated type the slot is `TokenStream2::new()` interpolated into a `quote!`, which emits
   nothing as a language guarantee — the same guarantee the six existing slots already rely on.

**RED MUTATION — two, and neither is the pair the previous text named.**

*First red — the `cfg` (unchanged in substance):* remove the `#[cfg(feature = "reflect")]` wrapper
from the emitted install. **Gate 2 reds with `E0433`** on `reflect_off_twin`, originating in the
`Component` derive. ~~and gate 3 reds in whichever form G6b selected (the count going 0 → non-zero,
or the `compile_fail` fixture starting to compile). **Both reds are required**: gate 3 alone would
be satisfied by a build that happens to have the dep in the graph, which F16/F19 make the *default*
at the workspace root.~~ → **struck 2026-08-21 (C8 audit).** Gate 3 no longer exists here (D31), and
the stated reason was **measured false**: at the workspace root `boyko-reflect` is a dependency edge
of **no** package under default features (`cargo tree --workspace -e normal`: one line, column 0).
F16 makes the root *select* it as a member, which is not the same as putting it in another package's
graph, and F19's per-package unification enables an optional dep only when a selected package asks —
which no default does. The conclusion *"one red is not enough"* survives, on a different and real
ground, stated as the second red below.

*Second red — the funnel touch (NEW, D27):* delete
`core::hint::black_box(<FixturePod as Component>::component_id());` from `reflect_on.rs`'s `main()`.
**`l2_b > 0` reds** — *"NOT RESOLVED (needle B inert) … from C8 it means the derive's install call
did not reach the image after the linkage was retired"*, which is that clause's own text and the
exact defect this audit measured. This is the red that makes C8's landing non-vacuous: without it,
the seventh slot can be emitted, be correct, and reach no artifact, and every gate above still
passes.

**Both reds are required, and the reason is now the right one:** the first proves the emission is
*gated*, the second proves it is *there*. A rung that lands only the first ships an install that
compiles in every configuration and exists in none.

*Third red (gate 5, respecified — D30):* ~~rename a field in a `Bindable + reflect` fixture on one
side only.~~ → **struck: not expressible, and D14 guarantees it never becomes expressible.** One
struct definition feeds both derives, both read the same `syn::Field::ident`, neither parses a rename
(`attributes(bind)` is declared and never read), and D14 fixes `#[reflect(skip)]` to *emit* an
`Opaque` field rather than omit it — *precisely* so the by-index vocabulary cannot drift. The two
indices agree by construction, permanently. **The red that fires is a generator mutation:** reverse
`boyko_macros/src/bindable.rs:53~`'s `ids`, or suffix `boyko_macros/src/reflect.rs:501~`'s baked `name:`. That is the drift Horn 2 buys
and the reason the test is the price of taking it.

*Fourth red (gate 4 — D29):* drop the `!hooks.storage_bitset` term from the emission's condition.
Gate 4 reds: the bitset tag's descriptor is installed and `type_info_of` returns `Some`.

*Fifth and sixth reds (gate 1 — D28):* **(i)** swap the descriptor argument to a sibling's
`TYPE_INFO` — gate 1 reds on `ptr::eq`, and note the table is write-once so the wrong descriptor is
never corrected; **(ii)** replace `raw` with a literal `0` — both subjects install into slot 0 and
first writer wins, so **either** the subject holding the non-zero id reads `None` **or** the subject
holding id 0 reads the *other* type's descriptor, decided by which install lands first (see D28's
measured note: `--test-threads=1` gives the `None` half deterministically, the parallel harness gave
the wrong-address half twice in three runs). Red (ii) is why gate 1 requires **two** subjects with
distinct ids: a one-subject gate whose subject happens to hold id 0 cannot see it in either shape.

#### C8 follow-up (2026-08-21) — a clause its own rung made prose-satisfiable, a clause with no red of its own, a tautology, and six anchors this rung rotted

> **Under the rung protocol: observer before gate, gate before edit.** Each fix below repairs or
> writes the check FIRST, then applies the property-breaking mutation and **OBSERVES** the failure,
> then restores the source byte-identically (`cmp` + SHA-256). Nothing here is predicted.
>
> **1. The L2 opt-in clause was satisfied by `reflect_on.rs`'s own PROSE — a regression this rung
> introduced in one change.** `OPT_IN_TOKENS` is matched against source text, and the clause at the
> top of the gate requires L2 to contain **all** of it. Pre-C8 the token was `reflect_linkage`,
> which the pre-C8 `reflect_on.rs` spelled **only in code** (`git show HEAD:…/reflect_on.rs` —
> verified: `:50` call, `:69` fn, and nowhere else). C8 changed the token to `#[component(reflect)]`
> **and** rewrote that file's header to spell it twice. **MEASURED:** delete the real attribute at
> `:46`, leave the header — `grep -c` still reads **2**, the clause stayed GREEN, and the census only
> reported three assertions and ~16 s of fat-LTO builds later, at `l2_a`, with a message about the
> linker rather than about the retired annotation.
>
> The rung had already found the **mirror-image** defect on the L3 side (its own draft rustdoc
> reddened the census), stated the rule *"L3's source may not quote these tokens, in prose or in
> code"*, and reworded `reflect_never.rs` — **and never applied the symmetric rule to the L2 half.**
> A rule written for one direction of a two-directional list is half a fix.
>
> **Landed:** `code_only()` in `reflect_absence_census.rs` (`:431`) — a character scanner that strips
> line comments while honouring string literals, and **refuses** (panics, with instructions) on the
> three constructs it does not model: block comments, raw strings, and a `'"'` character literal.
> Each refusal is detected **in the normal state**, so a mention inside prose is invisible to it —
> which is why the sentence *"…asked for"* in that file's header does not trip the raw-string guard.
> **Both** clauses scan the derived text, deliberately from one function: the symmetry is now the
> mechanism, not a rule the next author must remember, and that is stated at `OPT_IN_TOKENS`. The
> stripper is itself pinned by `code_only_keeps_code_and_drops_prose` (four shapes: whole-line
> comment, **trailing** comment, real code, `//` inside a string) — an instrument exercised only by
> the gate is this campaign's dead-datum class, and it has now been found six times.
>
> **RED OBSERVED** (delete `#[component(reflect)]` at `reflect_on.rs:46~`, leave the header's two
> mentions): exit **101**, `running 3 tests`, 1 failed, **`finished in 0.00s`** — it reds *before the
> first build*, where the old shape reported after three assertions and three fat-LTO links:
>
> > `reflect_on.rs no longer carries the reflect opt-in ["#[component(reflect)]"] IN CODE -- the
> > present control has stopped being annotated … Comments are stripped before this scan
> > ([code_only]), so a mention of ["#[component(reflect)]"] in that file's header does NOT satisfy
> > this clause: the attribute must be on a type.`
>
> Restored byte-identically: `cmp` clean, SHA-256 `bc77912a2530faa2b49950762b53c9c213e2358df71fa807a8ac9640161da112`.
>
> **2. Two in-source statements asserted what this rung's own landing refuted.** The landing note
> records that RED 2 fires on `l2_a`, not `l2_b`; that correction was written into this document
> **only**, and both source sites still carried the refuted claim — which are the lines an
> implementer reads. Corrected in place: `reflect_on.rs`'s funnel-touch comment (*"which reds
> `l2_b > 0`"* → **`l2_a > 0`**, with why) and the `l2_b` clause's own comment block in
> `reflect_absence_census.rs` (*"which reds exactly here"* → it does not).
>
> **3. `l2_b > 0` was live but had no red of its own — and its one distinguishing case IS
> constructible at C8.** Every mutation that removes the install (delete the `#reflect_install`
> interpolation; delete the funnel touch; delete the annotation) also removes the pulled object,
> takes needle A to zero, and reports at `l2_a > 0` ~35 lines earlier. Both of `l2_b`'s documented
> reds belonged to another clause. Its distinguishing case is *`boyko_reflect` symbols present while
> `install_type_info` is inlined away* — the "probe loses its subject in silence" shape D26 exists
> to prevent.
>
> **RED OBSERVED**, one edit in `boyko_reflect/src/registry.rs`: `#[inline(never)]` →
> `#[inline(always)]` on `install_type_info`. Exit **101**, and **`l2_b` reds ALONE** — `l2_a > 0`
> stayed green:
>
> > `NOT RESOLVED (needle B inert): the present control (reflect_on, feature on) carries NO
> > install_type_info symbol … (L2 B = 0)`
>
> `llvm-nm` on the mutated fat-LTO L2 image confirms the shape rather than assuming it: needle B
> **0**, needle A **6** — the three std `OnceLock<&TypeInfo>` instantiations, `prim::get_f32`,
> `prim::set_f32`, and the `REFLECT` data symbol; the call was inlined into the emitted slot and the
> name left the image. **`#[inline(never)]` on the installer is therefore load-bearing for this
> census, not a codegen preference**, and that is now written at the clause. Restored byte-identically:
> `cmp` clean, SHA-256 `ef1990ac781376d5949f976b29b477904216784ef2aa3ea390b3548fbbf33f67`.
>
> **4. Gate 4's second test was a tautology.** `assert_ne!(tag_id, control_id)` cannot fail —
> `register_new::<Self>()` mints one id per type — and it stayed green under RED 4 and under an
> install-deletion, i.e. under every red this rung defines. Gate 4's `running 2` was one real clause
> and one compile. Replaced with the claim the doc line was already making in prose and no assertion
> carried: **the suppression suppressed the reflect emission and nothing else** —
> `storage_kind(tag_id) == Bitset` and `storage_kind(control_id) == Table`, renamed
> `the_suppressed_tag_is_still_installed_as_a_bitset_enable_tag`. The distinct-id property survives
> non-tautologically: two different `StorageKind`s cannot come out of one slot.
>
> **RED OBSERVED**, and it is the mis-edit D29 actually invites — the term landing in a *neighbouring*
> binding: `let storage_install = if hooks.reflect { TokenStream2::new() } else { … }`. Exit **101**,
> **1 red / 1 green** — the `type_info_of(tag).is_none()` clause stays green (nothing is installed for
> the tag either way), so this defect is visible **only** here:
>
> > ``assertion `left == right` failed: `C8BitsetTag` (id 1) carries `storage = "bitset"` and reads
> > back as Table. The reflect suppression has taken the storage install with it … left: Table
> > right: Bitset``
>
> Restored byte-identically: `cmp` clean, SHA-256 `b2d52348c8ba6dfdf9dc228cbe82adf421853d2370202a7747e2c0a6b522c438`.
>
> **5. Six plan anchors this rung's own edit rotted — and the enumeration was short.** Corrected in
> place, **each re-derived by reading the file** rather than by trusting the number reported to me
> (this tree has measured that repairing doc-rot introduces fresh rot when the repair trusts a
> report): the D29 condition `:164` → **`:178`** at three sites (§D29's measured paragraph, C8's  <!-- doc-anchor-ignore -->
> `Lands`, and C8 gate 4's respecification); the six neighbours `:239 :263 :315 :334` → **`:254 :278  <!-- doc-anchor-ignore -->
> :330 :386`** (`:123`, `:144` unmoved) at two sites; D31's *"six existing slots"* `:389-394` →  <!-- doc-anchor-ignore -->
> **`:441-446`** (the verifier's report said `:441-447`, which is **seven** — `:447` is  <!-- doc-anchor-ignore -->
> `#reflect_install`, the slot D31's sentence is explicitly *not* counting). **Beyond the six:** F7's  <!-- doc-anchor-ignore -->
> whole anchor block (`:371-397` → **`:423-450`** plus seven live anchors), §D14's  <!-- doc-anchor-ignore -->
> `parse_reflect_no_default` call site `:165` → **`:180`**, and five `reflect_absence_census.rs`  <!-- doc-anchor-ignore -->
> anchors that C8 re-measured **at the audit** and then rotted **at the landing** (`:124`→`:181`→  <!-- doc-anchor-ignore -->
> **`:228`**; the `OPT_IN_TOKENS` const and its two assertions; the pulled-object paragraph; the  <!-- doc-anchor-ignore -->
> `l2_a`/`l2_b` clauses; the C7-descriptor paragraph). ~~**Nothing reds on any of this:**
> `internal_docs_anchors.rs`'s `GATED_DOCS` is `FEATURE_MAP.md` / `SYSTEMS.md` / `ARCHITECTURE.md` /
> `MESHLET-VIRTUAL-GEOMETRY-PLAN.md` and covers **no** `REFLECTION-PLAN-*.md`.~~ **NO LONGER TRUE, 2026-08-21:** all five reflection documents are now in `GATED_DOCS`, so this rung's anchors DO red. The numbers quoted just above are a record of a past repair, not live citations, and carry `doc-anchor-ignore` for that reason. New text should cite
> `component.rs`'s slots **by binding name**; names survive an insertion and these numbers do not.
>
> **6. Two over-claims corrected in place.** *(a)* **Red (ii)** (`raw` → literal `0`). Its stated
> outcome — *"the subject with the non-zero id reads `None`, the `expect` reds"* — is one of **two**
> reachable diagnoses, and not the one a default run usually gives. Measured over five runs of the
> same mutated binary: `--test-threads=1` is deterministic `None` (2/2, because
> `a_second_component_id_touch_changes_nothing` runs first and warms slot 0 with `ProbeLeaf`'s own
> descriptor), while the **parallel** harness gave the *wrong-address* half **twice in three runs**.
> The red itself is not race-dependent — exit 101 every time, two tests red, and
> `assert_ne!(leaf_id, root_id)` never firing — so gate reliability is unaffected; what varies is
> which correct diagnosis the reader gets. Recorded at D28 and at C8's red list.
> *(b)* **`cargo test -p boyko-macros` is a vacuous green** and must not be credited as a run:
> verified on this box — `running 0 tests` for the lib, then **15 doctests, every one `ignored`**
> (14 + 1 across the two doctest binaries), `0 passed` overall, exit 0. It is a **compile**, and the
> regression list below says so. `boyko_macros`'s behaviour is measured through `reflect-fixture`
> and `reflect-dogfood`, which run its expansion.
>
> *Follow-up regression, all unpiped, all exit 0:* `boyko-reflect` **4 + 19 + 1 + 13 + 7 + 9 + 8 + 1
> + 14** (`c6_compile_fail`'s single test drives two trybuild fixtures) · `reflect-fixture --features
> reflect-fixture/reflect --all-targets`: `c6_nested_descend` **4** · `c7_alloc_delta` **2** ·
> `c7_derive_bake` **17** · `c8_bitset_suppression` **2** · `reflect_absence_census` **2** (+1 ignored
> calibration; `running 3 tests`, up from 2 — the stripper's pin) · `reflect_leg_nonvacuity` **1** ·
> `reflect-dogfood --features reflect --all-targets`: `c6_dogfood_descend` **4** ·
> `c7_cross_crate_address` **3** · `c8_horn2_agreement` **2** · `c8_install_seam` **3** · root
> censuses `reflect_manifest_census` **7**, `reflect_ship_closure` **2**, `reflect_ci_coverage` **6**,
> `internal_docs_anchors` **5**, `engine_packages_census` **3**,
> `trybuild_corpus_compiler_witness` **2** · `cargo clippy --all-targets -- -D warnings`
> (touch-first, every `.rs` in the four packages `touch`ed before the first run) for `boyko-macros`,
> `boyko-reflect`, and `reflect-fixture` / `reflect-dogfood` in **both** feature states · **`cargo
> test -p boyko-macros` — 0 passed, 15 ignored, exit 0: a COMPILE, listed as one.** And the feature-off
> vacuity is re-confirmed rather than assumed: `cargo test -p reflect-fixture --test
> c8_bitset_suppression` (no feature) is `running 0 tests`, exit 0, exactly as that file's header
> warns.

---

### C9 — The refusal matrix, spanned, with an anti-rot census

> **LANDED 2026-08-26**, under the rung protocol (observer before gate, gate before edit). Every
> gate ran green with an unpiped exit code and a non-vacuous `running [1-9]`; every RED was applied,
> its failure OBSERVED, and every source restored byte-identically (`cmp` + MD5). C9 is the LAST
> CORE rung, so what follows is also this plan's closing record.
>
> **What landed, file by file.** `crates/boyko_macros/src/reflect.rs`: `REFUSALS` (six rows),
> six `IDX_*` constants, a const-eval name pin over all six, `parse_reflect_skip`,
> `has_integer_repr`, `spanned_message`, `push_opaque_field_refusal`, five spanned refusal sites,
> and the refused-item early return. `crates/boyko_macros/src/component.rs`: `reflect_span` on
> `ComponentHookPaths`, D29's `!hooks.storage_bitset` term **deleted**, and `reflect_enabled`
> re-derived as `hooks.reflect && !reflect_refused`. `crates/boyko_reflect/src/registry.rs`: the
> release `assert!` plus a hand-written `Component` subject and two release gates.
> `crates/reflect_fixture/`: the `trybuild` dev-edge, `tests/reflect_compile_fail.rs` and its six
> census fixtures, five `reflect_pass/` fixtures, two `reflect_compile_fail_upstream/` pins, **eight**
> blessed `.stderr` (the four accepting fixtures have none — a `t.pass()` case pins a runtime
> assertion, not compiler prose). `tests/reflect_refusal_census.rs`: the census, eight clauses. Deleted:
> the whole of `reflect_fixture`'s `c8_bitset_suppression.rs`, whose two subjects both migrated.
>
> **The refusal matrix as BUILT — six rows, every one of them authored by C9.**
>
> | `REFUSALS` row | caret, measured | fixture |
> |---|---|---|
> | `bitset_storage_rejected` | the `reflect` key — `^^^^^^^` under `reflect` in `#[component(reflect, storage = "bitset")]` | compile_fail |
> | `vec_field_rejected` | the field's **type** — under `Vec`, not under the field's doc comment (see finding 1) | compile_fail |
> | `fieldless_enum_without_repr_rejected` | the `enum` keyword | compile_fail |
> | `data_carrying_enum_rejected` | the `enum` keyword | compile_fail |
> | `union_rejected` | the `union` keyword | compile_fail |
> | `missing_default_rejected` | the type NAME, via `ReflectDefault`'s `on_unimplemented`; **message-only**, no `compile_error!` | compile_fail |
>
> **1. The `Opaque`-field caret was wrong as first built, and only blessing it showed that.** The
> matrix says *"the field"*, so the first emission used `syn::Field::span()`. A `Field`'s span starts
> at its first **attribute**, and a documented field's first attribute is its `///` line — so the
> caret landed under the doc comment, pointing at prose. Corrected to `field.ty.span()`: the type is
> what the kind table declined, it identifies the field unambiguously, and it is the one span that
> reads the same for a named field and for a tuple field (which has no ident to point at). The row's
> span column now says *the field's type*, and the reason is at the emission site.
>
> **2. A refused item emits its refusals and NOTHING ELSE, which the rung did not say and the
> corpus forced.** `codegen` returns a third value, `refused`, and `component.rs` gates the install
> slot on it. Without that a union fixture pins **three** errors — the refusal, D20's `Default`
> witness (a union cannot derive `Default`), and `<Self as Reflect>::TYPE_INFO` on a type with no
> `impl Reflect` — and a `.stderr` that pins two derived errors freezes rustc's rendering of them
> too. One refusal, one error.
>
> **3. The two upstream pins cannot live in the census directory OR in the feature-off leg**, and
> that is a *third* consequence of D34 the decision did not reach. Their output DIFFERS between the
> legs — a generic fixture is 15 errors with the feature off and 20 with it on — and one fixture
> cannot carry two blessed files. They are in `tests/reflect_compile_fail_upstream/`, run in the
> feature-ON leg only, and a census clause asserts they never gain a `REFUSALS` row.
>
> **4. An empty trybuild glob is a VACUOUS PASS, measured.** With a directory emptied, `trybuild`
> prints *"There are no trybuild tests enabled yet"* and returns success — so every clause in
> `reflect_compile_fail.rs` would stay green with its subject gone. The census covers
> `reflect_compile_fail/` by bijection; the other two directories now state their own `>=` floor in
> the harness, because nothing outside that file knows how many fixtures they are supposed to have.
>
> **5. ⚠️ Gate 2's first form COULD NOT FIRE, and RED 5 is what found it.** The clause read *"every
> `IDX_` constant is mentioned at least twice — one declaration, one use"*. The const-eval name pin
> (`same_str(REFUSALS[IDX_X].0, …)`) is itself a second mention, so **deleting a whole
> `quote_spanned!` site left the census green**: MEASURED, `7 passed` while `bitset_storage_rejected`
> compiled. The distinguishing byte is the tuple element — the pin reads `.0`, and only a refusal
> SITE reads `.1`. The clause now asserts `REFUSALS[IDX_X].1` appears **exactly once** for each
> emitted row and **never** for the message-only one, which is D36's own sentence in a form a scan
> can check. Re-run under RED 5: red.
>
> **6. And its replacement's first draft reddened in the GREEN state**, for the reason this campaign
> has already met once: a clause counting `quote_spanned!` occurrences matched **this file's own
> rustdoc**, which spells the macro's name three times while explaining the mechanism. Deleted with
> the measurement recorded at its site. Whether a site uses `quote_spanned!` rather than `quote!` is
> not a text property at all — it is a CARET, and the second RED measures it where carets live.
>
> **7. `{Self}` is a TEMPLATE, and the `.stderr` clause reddened on it first.** D36's *"each row's
> message literal appears …"* is a plain `contains` for five rows and false for the sixth:
> `on_unimplemented` substitutes `{Self}` with the offending type's name, so the row's bytes and the
> printed bytes differ by exactly the placeholders. The clause matches literal SEGMENTS in order,
> which keeps the drift it exists to catch fully visible while allowing the one substitution rustc is
> entitled to make.
>
> **8. One deviation from D36, with its reason.** D36 asks for *"every refusal site emitting
> `REFUSALS[IDX_X].1` inside its `quote_spanned!`"*. Each site does spell `REFUSALS[IDX_X].1` — that
> is what the census keys on — but the value goes through `spanned_message`, which re-mints the
> literal with the site's span. It has to: `quote`'s `ToTokens for str` builds its `Literal` at
> `Span::call_site()`, which is the derive attribute, i.e. exactly the caret C9 forbids. Setting the
> span on the literal as well as on the block makes the caret independent of which of the two rustc
> reads.
>
> **THE RED LEDGER — seven mutations, seven observations, four sources restored `cmp`-clean.**
> Pre-mutation MD5: `reflect.rs` `cfdd3ea8ac08523d8233a623e8b696fc`, `component.rs`
> `044442bcb0c00af36088a487801d5873`, `registry.rs` `e970da516ff5ec3ef6b147c0d5a905fd`,
> `boyko_reflect/src/reflect.rs` `540fee01aeda0923792a5a6f25fb2523`; all four verified identical
> afterwards by `md5sum -c` **and** `cmp`. ⚠️ **The identity claim is AT THE MOMENT OF RESTORE, and
> `registry.rs` was edited twice AFTER the ledger closed** — a missing `///` separator before its
> `# Panics` heading, and `#[cfg(not(debug_assertions))]` on the hand-written subject and its id,
> which `-D warnings` demanded because in a DEBUG build the two release-only gates are their only
> consumers. Both were re-gated (`boyko-reflect` debug 4 / release 7, clippy clean). Recorded here
> because *"restored byte-identically"* and *"unchanged since"* are two claims, and only the first
> one is what a RED ledger earns.
>
> | RED | mutation | OBSERVED |
> |---|---|---|
> | 1 | a `REFUSALS` row with no fixture | census exit **101**, 3 clauses red, naming `red1_invented_rule_rejected` as *"rows with no fixture"* |
> | 2 | one refusal's `quote_spanned!` → plain `quote!` | trybuild **mismatch**: the caret moved off the `union` keyword and onto the line above the item — the `#[derive(Component, …)]` attribute |
> | 3 | delete `#[diagnostic::on_unimplemented]` | trybuild **mismatch**: the message became rustc's generic *"the trait bound `NoDefaultImpl: ReflectDefault` is not satisfied"*; the census's message-only clause red too |
> | 3b | … **and** delete the row AND the fixture (the corrected form) | the bijection clause goes **GREEN** — the pre-D20 blindness, exactly as the correction predicts — while two other clauses red, because the message-only row is pinned BY NAME. Stronger than the correction expected, and measured rather than argued |
> | 4 | a spanned refusal site with no row | `boyko-macros` does not compile: `error[E0425]: cannot find value 'IDX_RED4_NEW_RULE' in this scope` |
> | 5 | delete the bitset refusal | trybuild: *"Expected test case to fail to compile, but it succeeded."* — and see finding 5 above for what it also found. ⚠️ **Its SECOND stated observation is not observable, and the rung should not have offered it:** *"`bitset_storage_rejected`'s migrated positive-control twin reads `Some` where C8's landed clause read `None`"* needs a live subject carrying `reflect` **and** `storage = "bitset"`, and with the refusal in place no such subject compiles — which is why the twin (`reflect_pass/bitset_tag_without_reflect_accepted.rs`) carries the tag WITHOUT `reflect` and is unmoved by this mutation. The consequence is real; the reading of it is the trybuild line, not a second assertion |
> | 6 | mis-scope the bitset refusal to any `storage` key (gate 5(a)'s own) | `dense_storage_accepted` reds — the dense control now carries the bitset message on its own `reflect` key |
> | 7 | delete the release `assert!` | `cargo test -p boyko-reflect --release`: *"test did not panic as expected"*, 1 red / 6 green — its positive control (`…table_id_still_works`) stays green, which is what tells the two apart |
>
> **GATES G5 IS DISCHARGED (D32).** Its `<!-- doc-path-planned -->` marker on
> `crates/reflect_fixture/tests/reflect_compile_fail.rs` is removed and the document's
> planned-deliverable pin went **4 → 3** — the anchors census's *"DOWN means a deliverable landed"*
> direction, fired by this rung and satisfied in the same edit.
>
> *Gates, all unpiped, all exit 0:* corpus feature-ON `running 3` (6 refusals + 2 upstream pins + 4
> accepting fixtures) · corpus feature-OFF `running 1`, and all six census fixtures **compile**, which
> is D33 measured rather than argued · census `running 7` · `boyko-reflect --release` `running 7`
> (the bitset gate + its control) · `c7_derive_bake` `running 16`.
>
> *Regression, all unpiped, all exit 0:* `boyko-macros` (a COMPILE — 0 passed, 15 ignored doctests,
> as C8 recorded) · `boyko-reflect` debug **4+19+1+13+7+9+8+1+14** and release **7+19+1+13+7+9+8+1+14**
> · `reflect-fixture --features reflect-fixture/reflect --all-targets` (`c6_nested_descend` 4,
> `c7_alloc_delta` 2, `c7_derive_bake` 16, `reflect_absence_census` 2+1 ignored,
> `reflect_leg_nonvacuity` 1, `reflect_compile_fail` 3) · `reflect-dogfood --features reflect
> --all-targets` (4+3+2+3) · **all nine `boyko_ecs` trybuild corpora** · the root censuses
> `internal_docs_anchors` 8, `engine_packages_census` 3, `trybuild_corpus_compiler_witness` 2,
> `reflect_manifest_census` 7, `reflect_ship_closure` 2, `reflect_ci_coverage` 6,
> `reflect_refusal_census` 8 · clippy `-D warnings`, touch-first, on all four edited packages in both
> feature states.


**Lands.** Every rejection the derive must make, each a `compile_error!` **spanned at the offending
token** (never at the derive attribute).

⚠️ **RE-COUNTED, RE-SPANNED AND RE-HOMED by the C9 audit, 2026-08-26 — D32–D38.** The table below
is the corrected matrix; the rows it replaces are struck beneath it, each with the measurement that
struck it. Three of the original rows refused inputs that **`#[derive(Component)]` or rustc already
refuses**, one row's field half was a strict subset of another row's, and two shapes the derive
accepts today had **no row at all**. The census trap is why this had to be settled before a line of
code: *a refusal outside `REFUSALS` is invisible to the census that keeps refusals honest* — and its
mirror, **a row in `REFUSALS` that C9 does not author is a fixture whose red cannot fire**, because
deleting C9's refusal leaves the program non-compiling anyway.

| refused | reason | span | `REFUSALS` |
|---|---|---|---|
| `storage = "bitset"` together with `reflect` | no per-row bytes exist — the bit *is* the datum, so "read field at offset" is meaningless | **the `reflect` key** (D37) | yes |
| an `Opaque` field without `#[reflect(skip)]` | D15 — the wire is shared with the shipped `boyko_serialize`; silent omission is unacceptable. **This ONE rule refuses every standard indirection** (`Vec`, `Box`, `Option<T>`, `Map`, `&T`, raw pointers, `PhantomData<T>`, a data-carrying enum *in a field*): all of them fall through the same arm of the classifier to `ValueKind::Opaque` — D34, measured | **the field's TYPE** — corrected at the landing: a `syn::Field` span starts at its first ATTRIBUTE, so on a documented field the caret landed under the `///` line | yes |
| a fieldless enum with no `#[repr(Int)]` | no guaranteed discriminant width (FIX Mi3) | the enum item | yes |
| **a data-carrying enum as the component ITSELF** — D38 | C7's non-struct arm accepts it and bakes `TypeKind::Opaque` with `fields: &[]` — *a coherent descriptor asserting that a type with two payload variants has no fields* | the enum item | yes |
| **a union as the component ITSELF** — D38 | the same arm, the same lie, and **no document had a row for it**: GATES' G5 corpus carries `union_rejected` and this matrix carried nothing | the union item | yes |
| **a type with no `Default` and no `#[reflect(no_default)]`** — **D20** | `default_in_place`'s `Some` arm is baked from `Default`, and an inspector's "Add Component" needs it. **This row is not a `compile_error!` and cannot be**: a proc macro cannot see trait impls, so the refusal is a trait bound carrying `#[diagnostic::on_unimplemented]`, which is this tree's existing answer for that class (`query/chunked_data.rs:67~`, `query/filter.rs:2507~`, with a blessed fixture at `tests/compile_fail_chunk/changed_filter_rejected.rs:11~`) | the **type name**, via the `const _: fn() = …` witness the derive emits | yes — as a **message-only** row (D36) |

**And one ACCEPT, which is not decoration:** field-level **`#[reflect(skip)]`** lands here (D35),
because the `Opaque`-field row is *defined in terms of it*. Five sites schedule it at C9, one of them
landed code, and it was on no rung's **Lands** list
anywhere. MEASURED 2026-08-26: `#[reflect(skip)]` on a field is accepted today and **completely
inert** — a `Vec<u32>` field carrying it still bakes `Opaque`, byte-identical to the same field
without it.

~~| generic type parameters | a per-impl `static TYPE_INFO` collapses across monomorphizations — the documented Bundle / Phase-12.5 `static SLOT` / Phase-17 `State<S>` trap | the generic param |~~
**STRUCK — D34.** MEASURED on rustc 1.97.1 with **no `reflect` opt-in in the input at all**: a
generic `#[derive(Component)]` struct fails with **15 errors** (`E0107` *missing generics for
struct* plus `E0425` *cannot find type `T` in this scope*), because the derive emits `impl #name`
and `impl … Component for #name` from the bare ident and threads no generics —
`crates/boyko_macros/src/component.rs:416` and `crates/boyko_macros/src/component.rs:432`. With
`#[component(reflect)]` added it is
**20**. The row's stated hazard — a per-impl `static TYPE_INFO` collapsing across monomorphizations
— is therefore **not reachable**: the reflect seam is never entered. The only thing the row could
still buy is one spanned error instead of fifteen, and that needs the *whole* derive to
early-return, which is not `cfg`-able (D33) and would therefore fire with the feature **off** as
well. Deleted from `REFUSALS`; §6 already defers generics to v2, and rustc enforces the deferral
today.

~~| `#[repr(packed)]` | taking `&field` on a packed type is UB; the `*_unaligned` ops are v2 | the `repr` attribute |~~
**STRUCK — D34, and the reason survives its row.** MEASURED, two subjects: `#[repr(packed)]` with
fields `u8, u32` under a **plain** `#[derive(Component)]` (no `reflect` anywhere) →
`error[E0793]: reference to field of packed struct is unaligned`, one error, caret on `Component`.
`#[repr(packed)] #[component(reflect)]` with fields `u8, u8` → **compiles, installs, and is sound**:
with every field of align 1 the struct is align 1, so every `base + offset` read is aligned by
construction. The *reachable* set of this row is exactly its *harmless* set, and its unsound set is
refused by a diagnostic C9 neither authors nor controls. The reason stays true of the accessors
(`crates/boyko_reflect/src/prim.rs`'s scalar reads take a shared reborrow, which requires
alignment), so the obligation **returns** the day `#[derive(Component)]` stops taking a reference to
a field — recorded here rather than gated, because a refusal for a case rustc already refuses has no
observable red.

~~| a data-carrying enum, `Option<T>`, `Vec`, `Map`, `Box`, `&T`, raw pointers | v2 kinds; `Option<T>` is the smallest data-carrying enum … | the field |~~
**MERGED into the `Opaque`-field row — D34.** MEASURED in one run, at offsets 0, 24, 32, 40, 0 and
8 respectively: `Vec<u32>`, `Box<u32>`, `Option<u32>`, `PhantomData<u64>`, `&'static u32` and
`*const u8` **all** bake `ValueKind::Opaque`, from the single fallthrough at
`crates/boyko_macros/src/reflect.rs:568~` after `scalar_kind`, the array arm and `is_nested_path`
have all declined. The `Opaque`-field row therefore already refuses, **at the field span**, every
input this row enumerated: two rows, one verdict, one message, one fixture. *(Raw pointers are
additionally refused upstream by `Bundle: Send + Sync` unless `storage = "dense"` or `no_bundle`
suppresses the bundle impl — measured both ways.)* The **item**-level halves of this row are neither
merged nor lost: they are the two new D38 rows above.

**On the fieldless-enum row's own reason.** *"a silent `Opaque` would be worse"* indicts the branch
the row permits: a fieldless `#[repr(u8)]` enum is **accepted** and gets exactly that silent
`Opaque` (measured) until **C10** gives it `TypeKind::Enum`, and §5 lets C9 land first. The row is
kept because its stated ground — no guaranteed discriminant width, FIX Mi3 — is real and
C10-independent; the window is recorded so it is not rediscovered.

**Also lands at C9 — ECS D5's second mechanism, moved here by D29 (2026-08-21).** A release
`assert!` inside `install_type_info`: `storage_kind(id) != Bitset`
(`crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs:388` is the public getter;
[`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md):340-347 states the requirement as *"refusal is
TWO mechanisms at TWO boundaries; neither substitutes for the other"*). ~~**It was on no rung's list
in any of the four documents**~~ — **FALSE, corrected 2026-08-26 (D37).**
[`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md):1320 lists this exact item against rung **EG3**,
with a live fallback clause — *"If CORE declines it, EG3 must add the check on its own read path and
say so"* — which C9 accepting the item does **not** retire. The substantive half of the claim holds
(no rung's *Lands* carried it), and the wrongness mattered twice: it hid the obligation to retire
ECS's conditional, and it hid the new **C9 → EG3** edge, now recorded in §7.4. Re-verified at this
audit: the landed installer (`crates/boyko_reflect/src/registry.rs:87`) still carries only its
`MAX_COMPONENTS` `debug_assert!` and bounds guard, and `boyko_reflect` names neither `storage_kind`
nor `Bitset` anywhere. It belongs here and not at C8 because the runtime half exists for the callers
the derive cannot see. **Gate:** an out-of-band `install_type_info(bitset_id, info)` panics. The gate
is `#[cfg(not(debug_assertions))]` and **its invocation is part of it** —
`cargo test -p boyko-reflect --release`, output read for a non-vacuous `running [1-9]`, exactly the
discipline `crates/boyko_reflect/src/registry.rs:141~`'s own note already states for the release
halves beside it. **RED:** delete the assert; the gate reds. *(The CI leg exists: `reflect-on` runs a
`profile: [debug, release]` matrix, `.github/workflows/ci.yml:174-194`.)*

**`storage = "dense"` is NOT refused** — a dense component has real per-row bytes at a stable
address, and it is the one non-table kind that is fully readable. Its *enumeration* problem is
[`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md)'s (analysis B.3); the derive has no reason to
refuse it and refusing it would make the design decline the one flagship component it can fully
read.

**Diagnostics quality is a deliverable, not a nicety.** ~~Aether emits `#[derive(Component)]` items
the user never wrote and can already produce `#[component(storage = "bitset")]` from
`tag Foo(bitset);` — so a user who typed three words would otherwise get an error about a derive
they never typed.~~ **STRUCK 2026-08-26 (D37): that user cannot exist.**
[`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md):520-529 deletes the combination by
construction — `reflect` is a key on the **`component`** construct only, and only `tag` emits
`storage = "bitset"` — and it is verified in the tree: `crates/aether_lang/src/expand.rs` emits
`#[component(storage = "bitset")]`, while the string `reflect` appears in that crate **only** as the
PBR material key `reflectance`. The discipline is kept on its own, stronger ground: Aether solves
this class with `quote_spanned! { name.span() => … }` and has a **recorded measurement** of what
happens without it (`crates/aether_lang/src/expand.rs:187~` — *"rustc's 'previous definition of the
type `Foo` here' pointed at `aether! {`"*), with a blessed fixture that now shows the caret on the
user's own `tag Foo;`. The remaining user of the bitset refusal is **hand-written Rust**
([`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md):534-536), and D37 spans it at the
`reflect` key.
**MIGRATION — nineteen landed tests in two files stop compiling, and C9 owns every one of them.**
The rung as first written listed zero. MEASURED green at the audit and therefore deleted by this
rung: `cargo test -p reflect-fixture --features reflect-fixture/reflect --test c7_derive_bake --test
c8_bitset_suppression` → `running 17` and `running 2`, exit 0. One compile error in a file deletes
**every** test in it, which is why a subject collision is a whole-file loss.

| subject | refused by | migration |
|---|---|---|
| `c7_derive_bake.rs`'s `Padded` — an un-skipped `PhantomData<u8>` | the `Opaque`-field row | **DONE**: gained `#[reflect(skip)]` (`crates/reflect_fixture/tests/c7_derive_bake.rs:219`), which is why D35 lands the attribute here. `crates/reflect_fixture/tests/c7_derive_bake.rs:911`'s index-faithfulness assertions are **unchanged** by the skip and were re-run green: D14 keeps `fields.len()` at 3, keeps the name and the offset, and keeps all four accessor slots `None` |
| `c7_derive_bake.rs`'s `NonStruct` — a data-carrying enum | the new D38 item row | **DONE**: moved to the corpus as `crates/reflect_fixture/tests/reflect_compile_fail/data_carrying_enum_rejected.rs`, subject and variants verbatim. Its gate's doc said *"C10 replaces this test rather than deleting it"* — **C9 did, four rungs early**, and the replacement is a blessed `.stderr`, not a deletion. `every_derived_descriptor` went 16 → 15 and the file went `running 17` → `running 16` |
| `c8_bitset_suppression.rs`'s `C8BitsetTag` | the bitset row | **DONE**: the whole file is deleted. The tag became `crates/reflect_fixture/tests/reflect_compile_fail/bitset_storage_rejected.rs`; that file predicted it — *"If this file ever stops compiling, the message arrived early and C9's row is the place to record it"* — and D37 records it. Its positive control and its storage-kind clause moved together into `crates/reflect_fixture/tests/reflect_pass/bitset_tag_without_reflect_accepted.rs`, a `t.pass()` case, so **both** are still RUN |

*(The falsehood at that file's header — *"the four `REFLECTION-PLAN-*.md` documents are not in
`internal_docs_anchors.rs`'s `GATED_DOCS`"* — died with the file. HEAD `eeb567be` had put all four
in, at `tests/internal_docs_anchors.rs:283`, one commit before the comment was read. The by-name
citation practice it defended is still right; only its stated reason was doc-rot.)*

**Gate.**
1. A `trybuild` corpus, one case per `REFUSALS` row, with blessed `.stderr`. ~~the established
   harness (`crates/boyko_ecs/tests/compile_fail_*.rs`)~~ — **STRUCK, D33: that package cannot host
   it, and the measurement runs in both directions.** `boyko_ecs` declares no `reflect` feature,
   trybuild copies the host manifest's `[features]` table into the generated crate, and the derive's
   whole emission is `#[cfg(feature = "reflect")]` evaluated *there* — so every fixture would
   **compile** and the harness would red with *"expected compile failure"* on all of them. MEASURED
   directly: `#[component(reflect)]` on a struct in `boyko_ecs` compiles clean (it would be `E0433`
   if the `cfg` were live — that package has no `boyko-reflect` edge at all) and emits
   `warning: unexpected cfg condition value: reflect`, which CI's clippy promotes under
   `-D warnings`. **The home is `reflect-fixture`**, at the paths and fixture names
   [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md):1196-1301 already reserves (D32), and it
   needs a `trybuild` **dev**-dependency in `crates/reflect_fixture/Cargo.toml`, which has no
   `[dev-dependencies]` table today. `reflect_dogfood`'s recorded dev-edge argument applies verbatim:
   a dev edge enters no ship closure, so G2's censuses do not move, and it carries no
   `features = [...]` array, so none of G1's manifest clauses touch it. `#[cfg(not(miri))]` like its
   siblings.
2. **Anti-rot census, and it is BIDIRECTIONAL or it is nothing (D36).** ~~a test asserting that the
   number of `.stderr` fixtures equals the number of refusal rules enumerated in the derive source
   (a `const REFUSALS: &[&str]` the derive itself iterates)~~ — **STRUCK: no test can read it and
   nothing would emit from it.** `boyko_macros` is `[lib] proc-macro = true`, so a `const` there is
   unreachable from any test — D31 measured that obstacle about this same crate — and *"the derive
   itself iterates"* is not a construction: each refusal is a distinct syntactic condition, so
   nothing iterates a list of names to decide anything. A `const` the derive only **declares** is the
   dead-datum class this campaign has now found five times. **The shape that makes it live:**
   `const REFUSALS: &[(&str, &str)]` — rule name, message — with one `IDX_*` per rule, and every
   refusal site emitting `REFUSALS[IDX_X].1` inside its `quote_spanned!`. Then a refusal added
   without a row has literally **nothing to say**, which closes the direction §2's D20 item 2 names
   (*"a rule that is not in `REFUSALS` is structurally invisible to it"*) and which the
   equality-of-counts version could not see. The census itself is a **source-text scan** — the shape
   `tests/internal_docs_anchors.rs` uses, and the only one available across a proc-macro boundary:
   it counts `REFUSALS` rows against `.rs` files in the compile_fail directory **only** (the
   `t.pass()` cases live in a sibling directory), and it asserts each row's message literal appears
   either at a `quote_spanned!` site in `crates/boyko_macros/src/reflect.rs` or, for the row marked
   *message-only*, at `crates/boyko_reflect/src/reflect.rs:84`'s `on_unimplemented`.
   ~~**Re-scoped 2026-08-21 (architect's C6 ruling, D21)** … It asserts the derive names the five
   standard indirection kinds (`Box`, `Vec`, `&T`, `Option<Box<_>>`, raw pointers) …~~ — **the
   CONCLUSION stands, the INSTRUMENT is struck 2026-08-26 (D34).** D21's ruling is unchanged and
   restated: this census covers DIAGNOSTIC quality, not termination; it is **not** the acyclicity
   proof and never was — that is `validate`'s `NestedCycle` arm, with `NestedNotInline` alongside it
   for addressing-validity, both landing at **C6** (§3.1); the consequence of a missing kind is a
   worse diagnostic, not an unsound descend. What is struck is the name table: it is a **dead datum**
   (the `Opaque`-field row already reaches the same verdict at the same span — measured, D34) and it
   is not a sound detector (`crates/boyko_macros/src/reflect.rs:436`'s `is_nested_path` decides on
   *"has generic arguments anywhere"*, so a user's `MyArena<T>` is syntactically identical to
   `Vec<T>` — the very reason C6 ruled a runtime refusal **list** impossible, and the reason that
   file's header says the structural checks *"enumerate no type name at all and therefore cannot
   drift against this list"*). Adding the list back would re-create the drift surface D21 removed.
3. **Two legs, and the second is what makes the corpus test the derive rather than test rustc.**
   Feature ON: every fixture fails with its blessed `.stderr`. Feature OFF: every fixture
   **compiles**, because D33 puts the refusals inside the same `#[cfg(feature = "reflect")]` block as
   the emission they guard — a refusal that fired with the feature off would refuse a program that
   compiles to nothing. ⚠️ **This is also the second reason the two struck rows could not have
   stayed:** MEASURED with the feature OFF, a generic fixture fails with 15 errors and a packed
   fixture with `E0793`. GATES G5's *"Feature off: every fixture **compiles**"* and its second RED
   (*"the harness reds on all nine at once"*) are therefore **false today, for two of its nine
   fixtures, before C9 lands anything** — recorded in §7.3d as G5's to fix.
4. The corpus is run under `--no-fail-fast`, and so is every leg of this rung. **`cargo test` stops
   at the first failing target**, so one known-red target shadows every target behind it — this repo
   has measured a trybuild fixture staying red for **87 commits** because a line was added and its
   `.stderr` was never re-blessed, invisible until the flag was passed. *(This is an invocation, not
   an instrument: nothing in the tree can red if it is omitted. It is listed with the invocation and
   **not counted as a gate** — this campaign has twice shipped a gate table whose members could not
   move.)*
5. **Two positive CONTROLS, labelled as controls.** (a) `#[component(reflect, storage = "dense")]`
   compiles and installs. ⚠️ MEASURED green **today, before any C9 code exists**:
   `storage_kind(id) == Dense`, `type_info_of(id).is_some() == true`, two `Prim(F32)` fields at
   offsets 0 and 4. It excludes *"the refusals ate the dense case too"*, which is real value, but
   under the reds below it cannot move — so it carries its own red: **mis-scope the bitset refusal to
   match any `storage` key**; the control reds. (b) `#[reflect(skip)]` on a `Vec` field compiles and
   bakes D14's descriptor (`fields.len()` unchanged, `Opaque`, all four slots `None`) — the way out
   of the `Opaque`-field row, and a `t.pass()` case, so it is **run**, not merely compiled
   (trybuild's `check_pass` executes the fixture binary and requires success — verified in trybuild
   1.0.120).
6. **The two D20 fixtures, and the second is a `t.pass()`:** `missing_default_rejected` (a type with
   no `Default` — its `.stderr` pins `ReflectDefault`'s `on_unimplemented` *message*, not rustc's
   generic E0277 text, which is the whole point of the row) and `no_default_accepted`
   (`#[reflect(no_default)]` compiles, `TYPE_INFO.default_in_place.is_none()`). **`REFUSALS` counts
   the first**, as a *message-only* row (D36), so gate 2's census sees the rule — the defect D20
   exists to close was precisely that a hidden `T: Default` bound is structurally invisible to a
   census keyed on `REFUSALS`.
   *(Both fixtures depend on two items **C7** lands under D22 — the `ReflectDefault` trait whose
   message the first `.stderr` pins, and the `reflect` helper-attribute registration without which
   `no_default_accepted`'s `#[reflect(no_default)]` is a "cannot find attribute" error at the use
   site. Both now exist: `crates/boyko_reflect/src/reflect.rs:84`.)*

**RED MUTATION.** Add a refusal rule to `REFUSALS` without adding its fixture. Gate 2 reds.
*Second red:* change one refusal's `quote_spanned!` to plain `quote!`. The `.stderr` no longer
matches (the caret moves to the derive) and gate 1 reds — which is how span quality becomes
gate-visible instead of aspirational.

*Third red, and it is the one that shows the census could not have seen D20 before:* delete
`#[diagnostic::on_unimplemented]` from `ReflectDefault`. Gate 6's first fixture reds because its
`.stderr` now carries rustc's generic *"the trait bound `Foo: Default` is not satisfied"* pointing
into the expansion. ~~**Then delete the `REFUSALS` row too** and confirm gate 2 goes back to green
with the rule gone~~ — **CORRECTED 2026-08-26: as written, this predicts the opposite of what its own
gate prints.** With the row gone and `missing_default_rejected` still on disk the counts are N
fixtures against N−1 rules, so gate 2 **reds**. The state being demonstrated — the one this plan
shipped in before D20 — had neither the row **nor** the fixture, so the mutation must delete
**both**. A red whose predicted observation is inverted is how a red that cannot fire gets certified.

*Fourth red, D36's own, and it is the direction the struck census could not see at all:* add a
spanned refusal to the derive **without** adding its `REFUSALS` row. Under the struck `&[&str]` shape
this was silently fine, because each refusal's message lived at its own site. Under D36 the new site
has no message to emit and does not compile, so the drift is caught at the derive rather than at a
fixture count.

*Fifth red, D37's own:* delete the bitset refusal. Because D37 also deletes
the `!hooks.storage_bitset` term from `component.rs`'s condition, the tag then **installs a
descriptor**, and `bitset_storage_rejected`'s migrated positive-control twin reads `Some` where C8's
landed clause read `None`.

**D29's derive-side term is REPLACED, not joined (D37).** With the refusal in place,
`component.rs`'s `!hooks.storage_bitset` term is unreachable in its
suppressing branch and its only witness no longer compiles — a dead datum whose gate has just been
deleted. It is deleted together with the gate it served. Nothing is lost: with the feature **off**
the whole emission is `cfg`-stripped and nothing installs, and with the feature **on** the refusal
stops the compile. ECS D5's *"two mechanisms at two boundaries"* is satisfied by the compile-time
refusal and the release `assert!` — not by three.

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

> **This rung carries A.8's `String` half of the drop-count obligation — assigned here 2026-08-21
> (D24).** A.8 asks for a drop-count test over `{ pod, String, Nested{String} }`; §3.3's row put the
> whole thing on C7 and then substituted a drop-free subject set, so the `String` half had no
> rung that could reach it — `ValueKind::Str` is structurally accessorless until this one. C7 keeps
> the half it can build (an instrumented `impl Drop`, exact counts, both directions) and **this
> rung's gate 2 plus its second red already ARE the `String` half**, expressed as allocator
> accounting rather than a drop counter: *"exactly 1 alloc + 1 free"* is the double-free half and
> *"1 alloc, 0 frees"* is the leak half. Nothing new is owed here; the cross-reference is recorded
> so A.8's obligation is not read as unlanded.

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
3c. **GATES owes CORE two gates C8 re-homed to it (D31, 2026-08-21), and they are listed here
   because the last two rungs each lost an obligation that lived in prose and on no list.**
   * **G6b** — the token-census question in full. C8's gate 3 is struck, not deferred: C8 asserts
     nothing about token counts, and G6b's *"whichever form has a RED runnable on this toolchain
     without nightly"* criterion must select a **third** form, its two named candidates having been
     measured — `-Zunpretty=expanded` is nightly, and the `compile_fail` fixture is satisfied by the
     dep's absence and is blind to the derive's `cfg`. Until G6b lands, the campaign's token claim
     rests on **G6a**, which is C8's gate 2 and is the stronger statement anyway.
   * **G6c** — C8's gate 6 ("the six slots are unperturbed"), respecified as G6c's concurrent twin
     comparison (`reflect_off_twin` vs `reflect_never`: same symbol multiset, same `.text`). After
     C8's D27 touch both legs carry the funnel, which is what makes the comparison answer C8's
     question. G6c must also gain the twin-source identity guard its own note already demands.
3d. **GATES G5 is DISCHARGED by C9 (D32), and it carries two defects this audit measured from the
   CORE side.** G5 (*"the derive's refusals: one trybuild corpus, two legs"*,
   [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md):1196-1301) specifies the same corpus C9
   builds, in the same package, and closes with *"Named here so the two are not built twice"* — while
   naming CORE only by document, never by rung, so the collision was invisible to a grep for `C9`
   (**zero** hits in all four siblings). C9 builds it at G5's paths and fixture names; G5 becomes a
   cross-reference the next time GATES is edited. Travelling with it:
   * **G5's feature-off leg is false today, before C9 lands anything.** Gate 2 says *"Feature off:
     every fixture **compiles**"* and its second RED says the harness *"reds on all nine at once"*.
     MEASURED on rustc 1.97.1 with the feature OFF: `generic_component_rejected` fails with 15 errors
     and `repr_packed_rejected` with `E0793`, because both are refused by `#[derive(Component)]`
     itself rather than by the reflect emission (D34). Under D34 those two rows leave `REFUSALS`, so
     the leg becomes true again for the corpus that is actually built — but the *claim* has to be
     corrected where it is written, not left to be inherited.
   * **G5's `aether_tests` twin is a fixture for an input Aether cannot emit.** G5 assigns *"the
     spanned version of this refusal"* to an `aether_tests` trybuild fixture and calls it *"CORE's
     Aether item"*. [`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md):520-529 deleted that
     combination by construction and this audit verified it in the tree (D37). The fixture is retired
     with the motivation, not kept as decoration.
4. **ECS owes CORE nothing; CORE owes ECS the model.** CORE's accessors take a bare `*const u8` /
   `*mut u8` and never reach into `boyko_ecs`'s storage, so C1–C11 are buildable and gateable with
   no ECS glue at all. The seam is deliberate: it is what lets the value model be proven before the
   `get_component_raw` / enumeration work starts. ECS consumes `type_info_of`, `TypeInfo.fields`,
   and the `prim::` accessors, and owns everything about *reaching* the bytes — including
   `BUG-MIGRATE-TB-1` (raw-pointer projection, never `&Archetype`) and the three-source
   enumeration. **CORE also owes ECS the install that fills the table ECS reads:** `type_info_of`
   returns `Some` only because of **C8**'s seam, so **ECS EG8 depends on CORE C8** — an edge neither
   document carried before the C8 audit, and EG8's gate 2 (*"`GpuTransform3D` is present in the
   enumeration"*) cannot pass without it. EG8 additionally needs the real engine types to be
   *annotated*, which **no rung in any of the four documents schedules**; that is EG8's to carry or
   to hand to a rung of its own, and it is recorded here so it stops being invisible.
   **And CORE now owes ECS a second install-side item: `C9` carries ECS D5's release
   `assert!(storage_kind(id) != Bitset)` inside `install_type_info` (D29, re-confirmed at the C9 audit
   under D37), so **ECS EG3 depends on CORE C9** — the last rung of this plan.
   [`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md):1320 has carried that item against EG3 all
   along, with the fallback *"If CORE declines it, EG3 must add the check on its own read path and say
   so"*; CORE does **not** decline it, so that conditional is retired and must be struck when ECS is
   next edited, or EG3 builds the same check twice. C9's own text claimed the item *"was on no rung's
   list in any of the four documents"*, which was false and is struck at the rung; the sentence is
   what hid this edge, exactly as the C8 → EG8 edge above was hidden.
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
