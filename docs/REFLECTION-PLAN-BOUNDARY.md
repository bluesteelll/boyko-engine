# `boyko_reflect` — the BOUNDARY: serialize, names, and the Aether seam

> **This is a plan, not a discussion.** Its input is `docs/REFLECTION-ANALYSIS.md`
> (revision 2026-08-21, re-grounded against `feat/reflection`). Everything below is a
> rung a developer executes, a gate that must be able to go red, or a numbered decision
> a later reader may not re-litigate from scratch.
>
> **Not re-litigated, inherited as settled** (`REFLECTION-ANALYSIS.md` §0/§2, §6):
> * "reflection only in a Debug build" is **not a compiler property**. The mechanism is
>   an **optional crate behind a Cargo feature** plus a **CI absence gate** on the ship
>   build. This plan assumes it and never revisits it.
> * The scope fork is **TAKEN: option (B)** — POD + `String` + nested + `#[repr(Int)]`
>   enum, collections deferred to v2 — **TAKEN-with-reason, and reversible**. Reason
>   (the analysis's own): *a layer that can only read `f32`/`u64` is not an inspector.*
>   It was never pre-existing consensus; it was recorded 2026-08-21 so implementation
>   could start. Within (B), the analysis's re-grounding (§6.1(d), B.8) orders the halves
>   **arrays → nested → enums → `String` last**, because `String` has **zero** in-tree
>   consumers and arrays have many. **This plan follows that order** and its `Str` rungs
>   are the last ones.

## What this file owns, and what it does not

| Plan | Owns |
|---|---|
| `docs/REFLECTION-PLAN-CORE.md` | `Scalar` / `ValueKind` / `TypeKind` / `TypeInfo` / `FieldInfo` / `EnumInfo`; the `REFLECT` registry; `#[derive]` + the `reflect` opt-in surface; `ValueKind::Array`; the `Opaque` refusal and `#[reflect(skip)]`; the `BindAccessor` one-table-or-two fork |
| `docs/REFLECTION-PLAN-ECS.md` | three-source enumeration; `get/set_field`; `add_default`/`remove`; the public by-id structural seam on `EcsMaster`; the B.4 citizen matrix as an ECS-side dispatch; `BUG-MIGRATE-TB-1` in the enumeration glue |
| **`docs/REFLECTION-PLAN-BOUNDARY.md` (this file)** | **the `Sink`/`Source` boundary and the one `dyn`; the dump grammar; type-key resolution (name↔id) and its once-per-dump materialization; the name-keyed roundtrip incl. the simulated id-reorder; the Aether seam** |
| `docs/REFLECTION-PLAN-GATES.md` | feature wiring; `cargo tree` + symbol-absence ship gate; the Miri package allowlist; the hot-path 0 %-gate and the bevy-shaped baseline bench; CI legs |

Where this plan needs something a sibling owns, it says so in **§Dependencies** and cites
the sibling by filename. It does not restate their content.

---

## Goal

A **name-keyed, zero-allocation, one-`dyn`** boundary that turns a live component into a
byte stream and back, such that the stream survives **everything a `ComponentId` does
not**: a different registration order, a different process, a reordered struct, a
renamed field, a component that is not reflectable at all.

The boundary exists for the editor: copy/paste a component, author a prefab, diff a
live-tuning session, dump an entity into a bug report. It is **not** the save/load
engine — that is `boyko_serialize`, it ships, and it is forbidden from depending on
`boyko_reflect` (`REFLECTION-ANALYSIS.md` §1; asserted at the source in
`crates/boyko_serialize/Cargo.toml:6-10` and `crates/boyko_serialize/src/lib.rs:6`).

| Property | v1 target | How it is established |
|---|---|---|
| `dyn` occurrences in `crates/boyko_reflect/src/**` | **exactly the boundary's** | source census with a committed allowlist (rung B1) |
| methods on `trait Sink` / `trait Source` | **1 each** | source census (rung B1) |
| allocations in `dump_entity` with a warmed sink | **0**, measured as a delta | counting global allocator + baseline subtraction, the `crates/boyko_ui/tests/p4_bind_zero_alloc.rs` methodology (rung B1) |
| allocations in `apply_entity` (no `Str` field) | **0**, measured as a delta | same harness (rung B3) |
| `resolve_stable_name` calls per dump | **once per distinct type**, never per entity | call counter on the fixture (rung B4) |
| dump bytes for the acceptance fixture | **pinned, and the pin must move deliberately** | committed golden blob (rung B5) |
| dump bytes across two processes whose ids differ | **byte-identical** | two test binaries, the id difference itself asserted (rung B5) |
| `aether_lang` new dependencies | **0** | its manifest is unchanged; gated by `cargo tree -p aether-lang` (rung B6) |

Every number above is *measured by a named instrument*, not asserted. Where a rung
claims a figure, the rung says which instrument produced it.

---

## Context and constraints

### Verified in-tree facts (this session, worktree `D:/wt/reflect`, branch `feat/reflection`)

Cited because each one changes a decision below. Symbol names are given beside line
numbers so a moved line does not invalidate the citation.

**The name index reflection is told to consume already exists — and is not total.**

* `SerializeInfo { serialize_fn, deserialize_fn, map_entities_fn, serializability,
  format_version: u16, layout_fingerprint: u64, stable_name: &'static str,
  stable_name_hash: u64 }` — `component_registry/serialize.rs`, `struct SerializeInfo`.
* `Component::stable_name()` defaults to `std::any::type_name::<Self>()` and is
  overridable by `#[component(stable_name = "...")]` —
  `crates/boyko_ecs/src/ecs/core/component/component.rs:185`. It is a **method, not a
  const**, deliberately (`type_name` is not a stable `const fn`).
* `register_stable_name::<C>` / `resolve_stable_name(hash, name)` / `fnv1a_64` /
  `STABLE_NAME_INDEX` — all `pub` in `component_registry/serialize.rs`. The index is
  documented **cold**: *"touched only at registration and once per file-local type at
  load — never on the per-frame hot path"*, which is W2's requirement, already met.
* **The index is installed UNGATED for every component — except bitset ones.** In
  `crates/boyko_macros/src/component.rs` the `serialize_install` token stream is
  `if hooks.storage_bitset { <empty> } else { install_serialize_fn::<Self>(raw);
  register_stable_name::<Self>(raw); }`. So **a `#[component(storage = "bitset")]` type
  has no `SerializeInfo` and no entry in `STABLE_NAME_INDEX`.** This is the fact behind
  **D9**.
* `resolve_stable_name` confirms a hash hit by comparing
  `get_serialize_info(candidate).stable_name` — so even if a bitset id were in a bucket,
  the confirmation step would reject it.

**The shipped wire format already solves the id-instability problem, and its choices are
precedent.**

* `boyko_serialize` `MAGIC = *b"BOYKOSAV"` (`format.rs:22`), `FORMAT_VERSION = 2` (`:34`).
* `TypeTableEntry` = 40 B, padding-free, `{ stable_name_hash@0, layout_fingerprint@8,
  size@16, align@20, name_off@24, name_len@28, format_version@32, serializability@34,
  _pad@35 }` with `const _: () = assert!` pins on every offset (`format.rs:210-258`).
* `load.rs::resolve_type_table` materializes a **dense `Vec<ResolvedType>` indexed by the
  file-local type index**, one `resolve_stable_name` per file type, never per entity
  (`load.rs:340-408`). This is the shape rung B4 copies.
* **A `format_version` mismatch is a hard `LoadError::VersionMismatch`** — *"never a
  silent blit of stale bytes, even when the `layout_fingerprint` still matches (a
  same-shape semantic reinterpretation)"* (`load.rs:198-200`). Precedent for **D7**.
* Leniency is always **counted**: `LoadReport { types_skipped, types_bitset_skipped,
  types_defaulted, … }` (`load.rs:81-111`). Precedent for **D8**/**D12**.
* `boyko_serialize/tests/load_roundtrip.rs:361 absent_file_type_is_skipped` performs
  in-place byte surgery on the saved stream (same-length name rewrite so no offsets
  shift) and asserts `types_skipped == 1`. That is the in-house technique rung B5 reuses.

**`EnableTagId` has only one public direction.**

* `pub struct EnableTagId(pub(crate) ComponentId)` — `component_registry/tags.rs:93`.
* `EnableTagId::component_id(self) -> ComponentId` (`:95-102`) and
  `impl From<EnableTagId> for ComponentId` (`:104-109`) exist. **The inverse does not.**
  A whole-crate grep for `EnableTagId(` finds six construction sites, **all inside
  `boyko_ecs`**; there is no `From<ComponentId>`, no `from_component_id`, and
  **`tags.rs` contains no `pub fn` at all**.
* The only public route to an `EnableTagId` from outside is
  `EcsMaster::register_enable_tag(name)` / `try_register_enable_tag(name)`
  (`enable_tag_api.rs:60`, `:72`) — a **name-keyed dynamic mint** whose name space is
  `TAG_NAMES` (`tags.rs:155`), which **derived** bitset components never intern.
* Therefore: **`REFLECTION-ANALYSIS.md` B.4's "public round-trip" is half-true.** The
  round-trip test it cites (`component_registry/mod.rs:1676`) runs *inside* the crate.
  An external `boyko_reflect` can enumerate a bitset `ComponentId` and **cannot call
  `is_enabled_id` / `enable_id` / `disable_id` with it**. See **D10**.
* `storage_kind(component_id) -> StorageKind` (`mod.rs:388`) and
  `get_layout(component_id) -> Option<&'static ComponentLayout>` (`mod.rs:1123`, with
  `pub type_name: &'static str` at `:117`) **are** public. That pair is what **D9** uses.

**The Aether seam is three files and no new dependency.**

* `crates/aether_lang/Cargo.toml:6-11` states the rule verbatim: *"this crate does NOT
  depend on `boyko-ecs`: every `boyko_ecs::…`/`boyko_macros::…` occurrence in the
  expander is an emitted TOKEN resolved in the downstream crate (the tokens-not-deps
  rule)."* Its only dependencies are `syn`, `quote`, `proc-macro2`.
* `expand.rs::component` builds a `component_attr` **keys vector** and emits
  `#[component( #(#keys),* )]` — that is already how `no_bundle` and all four `on_*`
  hooks get in. `expand.rs::tag` emits `#[component(storage = "bitset")]` for
  `tag NAME(bitset);`. Both are `quote_spanned!` at the user's name, with the reason
  recorded in a comment (*"MEASURED at rung A7: with `quote!` here, rustc's 'previous
  definition of the type `Foo` here' pointed at `aether! {`"*).
* `ast.rs::ComponentDef { name, fields, requires, hooks, no_bundle }` — **five fields, no
  `attrs`, no attribute-passthrough grammar anywhere**.
* `parse.rs::parse_component` (`:206-300`) is an item loop with a bare-flag precedent
  (`no_bundle`, `:273-278`) **and a lookahead list** at `:256-263` that decides whether an
  identifier after a comma continues a `requires` path or opens a new item. **That
  lookahead is the easily-missed third touch point** — see rung B6.
* `boyko_macros::component::parse_component_hooks` (`component.rs:605+`) already accepts
  bare flag keys (`no_bundle`, `no_clone`, `no_serialize`) and NameValue keys
  (`clone = <fn>`, `storage = "bitset"|"dense"`, `stable_name = ".."`,
  `format_version = N`). A `reflect` bare flag is **not a new shape**, it is the sixth
  instance of an existing one.
* The `component_id()` funnel (`component.rs:350-374`) already carries **six** install
  slots (`storage_install`, `require_install`, `clone_install`, `relationship_install`,
  `residency_install`, `serialize_install`), each an independently-emitted
  `TokenStream2`. Appending a seventh is a well-trodden pattern here.

**The missing-feature case is diagnosable, and only because of one existing table.**

* Root `Cargo.toml:25-26`: `[workspace.lints.rust] unexpected_cfgs = { level = "warn",
  check-cfg = ['cfg(loom)', 'cfg(force_alloc_panic)'] }`, and **every member opts in**
  via `[lints] workspace = true`. Cargo's own `--check-cfg` for `feature = "..."` values
  is generated from each manifest, and this table **adds to** it rather than replacing
  it. So `#[cfg(feature = "reflect")]` in a crate with no `reflect` feature **warns**,
  and the existing `cargo clippy --workspace --all-targets -- -D warnings` gate
  **promotes that warning to red**. See **D16**.
* Root `Cargo.toml` `default-members` names every member **plus `"."`**, so a bare
  root `cargo build` selects every crate. There is no non-workspace-wide root build any
  more (`REFLECTION-ANALYSIS.md` §2's severity inversion). Feature unification is the
  default, which is `REFLECTION-PLAN-GATES.md`'s problem, but it constrains **D14**.

**The zero-allocation instrument exists.** `crates/boyko_ui/tests/p4_bind_zero_alloc.rs`
— a counting global allocator plus **baseline subtraction** between two schedules of
identical shape, so the measured quantity is the subject's *own* allocations, not the
executor's. Rungs B1/B3 use this harness, not a fresh one.

### Invariants (violating any one is a red, not a discussion)

1. **The boundary is name-keyed at both levels.** A type is keyed by its stable name; a
   field is keyed by its field name. No `ComponentId`, no field index, and no byte offset
   ever enters the stream.
2. **`boyko_reflect` depends on `boyko_ecs` and nothing else in the workspace.** In
   particular it does **not** depend on `boyko_serialize` (D1), so the two formats cannot
   drift into each other by accident.
3. **Zero allocation in `boyko_reflect` itself.** Buffers are caller-owned and reused
   (`REFLECTION-ANALYSIS.md` A.7). The only irreducible allocation is a `Str` field's
   payload on apply, which is loader-owned, cold and explicit.
4. **Leniency is always counted.** No skip, no refusal and no mismatch is silent; every
   one lands in a report field that a test asserts by number.
5. **A refusal is a record, not an absence.** A component the boundary cannot read is
   written as a typed refusal, never omitted and never zero-filled.

---

## Key decisions

### D1 — The reflect dump is NOT a save format, and the crates cannot see each other

`boyko_reflect` produces a **dev artifact**: an editor clipboard payload, a prefab draft,
a bug-report dump. It has no backward-compatibility promise across engine versions and no
migration story.

* `boyko_reflect` **must not** depend on `boyko_serialize` (nothing forbids it — the
  directional rule only bans the other direction — but taking the dependency would import
  `TypeTableEntry`/`SaveHeader` and start a slow merge of two formats with opposite
  compatibility promises).
* Everything the boundary needs from the serialization world is in **`boyko_ecs`**
  (`get_serialize_info`, `resolve_stable_name`, `fnv1a_64`), which is already the one
  allowed dependency. This is not a workaround; it is why §4's placement call ("resolution
  lives in `boyko_ecs`; Reflect *uses* it, it does not *own* it") was right, and B.2
  confirms it was independently built there.

**Rejected:** *"share the `boyko_serialize` wire format."* It would put a **shipping**
crate's on-disk compatibility promise on the far side of a dev-only feature, and A.6's
`Opaque` hard error exists precisely because the two were assumed to share a format.

### D2 — One `dyn`, one method, one vtable slot

```rust
pub trait Sink {
    fn event(&mut self, ev: &SinkEvent<'_>) -> Result<(), SinkError>;
}
```

The `dyn` is **structurally forced**, not tolerated: `FieldInfo.serialize` is an
`unsafe fn(*const u8, &mut dyn Sink)` — a **function pointer cannot be generic**, so the
sink cannot be a type parameter. That is the whole reason the one `dyn` exists, and it is
the reason there is exactly one.

Making it **one method** rather than eleven is a separate, deliberate choice:

* the vtable is one slot, so the "one `dyn`" claim is literal rather than nominal;
* every `prim::emit_*` monomorphization ends in the *same* call shape, so the fn-ptr
  library's I-cache footprint is one target instead of eleven (principle 3, I-cache half);
* **a v2 kind (a `Vec`, a map) adds an `enum` arm, not a trait method** — it does not
  break every sink implementation in a downstream editor.

**Rejected:** a method-per-kind trait (`fn scalar`, `fn str`, `fn begin_array`, …). It
lets a sink specialize per kind, which measurement does not currently ask for, and it
makes every v2 kind a breaking change to the trait.

### D3 — `Source` is a lending pull iterator, and the borrow REPLACES A.6's documented contract

```rust
pub trait Source {
    fn next(&mut self) -> Result<Option<SourceEvent<'_>>, SourceError>;
}
```

`REFLECTION-ANALYSIS.md` A.6 states the deserialize-side rule as a **hard contract**:
*"`Source::str_field`'s returned `&str` must be consumed by `set_str` (copied) before the
next `&mut self` `Source` call."* With the signature above the compiler enforces it: the
returned event borrows from `&mut self`, so a second `next()` cannot be called while the
event is alive. **Delete the prose contract; keep the signature.**

This is the same move A.3 made for the read side (*"the `'a` (compiler-enforced) IS the
validity guarantee — there is no 'documented contract'"*), and it is applied here for the
same reason: a contract a reviewer must remember is a defect waiting for its first
forgetful caller.

### D4 — An unknown kind is a hard error at both ends, never a silent skip

`SinkEvent` and `SourceEvent` are `#[non_exhaustive]`. A `#[non_exhaustive]` enum forces
downstream sinks to write a `_ =>` arm — and **the default arm is specified to return
`Err(SinkError::UnsupportedKind { .. })`, never `Ok(())`.** A v1 sink meeting a v2 kind
fails loudly.

This is A.6's own stance (*"the derive refuses to serialize a type containing an `Opaque`
field (hard error) rather than silently dropping it … silent omission is unacceptable"*)
carried from compile time to run time, because `#[non_exhaustive]` otherwise reintroduces
exactly the silent drop A.6 outlawed.

### D5 — Binary only in v1, and the byte form is a 1:1 image of the event stream

One concrete pair ships: `VecSink` / `SliceSource` over a caller-provided `&mut Vec<u8>` /
`&[u8]`. The encoding is **one `u8` event tag plus that arm's payload**, strings as
`u16` length + bytes, scalars as their raw bits.

Consequences, all of them wanted:

* **f32/f64 round-trip is exact and NaN payloads survive** — no formatting, no parsing, no
  precision question. A text format would owe a round-trip proof per float; this owes none.
* the sink is a `match` + `write`, the source is a `read` + `match`; neither can be
  "smarter" than the other, so there is no encoder/decoder asymmetry to test for;
* **the event-sequence pin and the byte pin are the same statement**, so rung B5's golden
  blob gates both at once.

**Rejected:** a text (JSON/RON) sink in v1. It is a second wire format, it owes an
escaping story and a float round-trip story, and it has no v1 consumer. It is a v2 sink
implementation and costs nothing to add later — that is the point of D2's one-method
trait.

### D6 — The type key is `SerializeInfo.stable_name`, consumed, never re-declared

Per B.2: `TypeInfo` carries **no** `stable_name` field. The dump reads the key from
`get_serialize_info(id)`. This buys `format_version`, `layout_fingerprint` and
`stable_name_hash` for free and removes the drift class B.1 Horn 2 warns about — here with
a *serialization* wire format on the other end, which is the worst place to keep two
copies of a name.

**Known limitation, recorded not hidden:** the default `stable_name` is
`std::any::type_name::<Self>()`, which contains the module path and is therefore
**refactor-fragile** — moving `Position` between modules invalidates every dump that names
it. The fix exists and is the user's: `#[component(stable_name = "game::Position")]`. Rung
B4 surfaces this as a **counted** `types_unresolved`, and the acceptance fixture uses an
explicit `stable_name` so the test cannot be broken by moving the fixture file.

### D7 — `format_version` mismatch REFUSES; `layout_fingerprint` mismatch DOES NOT

Two guards travel in `SerializeInfo`; the boundary treats them differently, and the
asymmetry is the whole reason name-keying exists.

| guard | shipped loader (`boyko_serialize`) | reflect boundary | why |
|---|---|---|---|
| `layout_fingerprint` | hard error before a POB blit | **recorded, surfaced, not gated** | a field-wise apply does not blit. A changed layout is *precisely* the case name-keying is designed to survive; gating on it would refuse the only case reflection adds value in |
| `format_version` | **hard `LoadError::VersionMismatch`, even when the fingerprint matches** (`load.rs:198-200`) | **hard refusal**, counted (v1 is `Strict`-only — D19(b); a counts-and-continues mode is the v2 arm) | a `format_version` bump means a **semantic** reinterpretation of the same bytes. Neither a blit nor a field-wise apply can detect it; the loader's own doc says so. A name-keyed apply is not smarter here, only quieter, and quieter is worse |

### D8 — Patch semantics: a field absent from the dump is left untouched, and counted

`apply` never default-constructs. A field present in the target type but absent from the
stream is **untouched**, and lands in `ApplyReport.fields_missing`.

* it composes: applying two dumps over disjoint field sets is order-independent and
  idempotent, which is what live-tuning and "paste these two fields" need;
* it keeps `default_in_place` and A.8's drop-safety obligations out of the boundary —
  they belong to `add_default`, which is `REFLECTION-PLAN-ECS.md`'s;
* the difference between patch and replace is *visible* rather than assumed, because the
  count is asserted by number in rung B3.

**Rejected:** replace semantics (absent ⇒ default). It silently drags the whole
`add_default` drop-safety surface into every paste, and it makes a truncated stream
destructive.

### D9 — Bitset presence is resolved through `storage_kind` + `ComponentLayout::type_name`, NOT through `STABLE_NAME_INDEX`

The index B.2 tells reflection to consume is **structurally blind to exactly the citizens
B.4 wants a presence view for**: the derive skips `register_stable_name` for every
`#[component(storage = "bitset")]` type, and `resolve_stable_name`'s confirmation step
reads `get_serialize_info(candidate).stable_name`, which is `None` for them. A bitset tag
therefore cannot be found by name through that door, at all.

**v1 resolves a bitset presence record by scanning the registered bitset ids** — for
`id in 0..MAX_COMPONENTS` where `get_layout(id).is_some() && storage_kind(id) ==
StorageKind::Bitset`, compare `get_layout(id).type_name` to the record's key. Both
accessors are public today. The scan is ≤ 512 iterations, once per dump, on a cold path,
and it is **the same set `REFLECTION-PLAN-ECS.md`'s enumeration source 3 already walks** —
so the "index" is the enumeration itself, and there is no second name table to drift.

**Rejected (a):** *omit bitset tags from the dump in v1.* An editor "copy this entity"
would silently lose `EmitterActive` / `RenderEnabled` — a visible, wrong result, and
exactly invariant 5's failure.
**Rejected (c):** *make `register_stable_name` total in `boyko_ecs`.* It is the cleanest
long-term answer, but it is not one line: `resolve_stable_name` also has to stop
confirming through `SerializeInfo`, which means either installing a partial
`SerializeInfo` for bitset types (contradicting the comment that says the skip exists to
*keep the metadata table consistent*) or a second confirmation path. That is a shipping-
crate change made for a dev-only feature, with a `boyko_serialize` blast radius, and it
buys only refactor-robustness the type-name scan already approximates. **Filed as the v2
fix**; recorded in `docs/OPEN-QUESTIONS.md`.

**Residual, stated:** `ComponentLayout::type_name` is `std::any::type_name`, so a bitset
tag's presence key is refactor-fragile in the way D6 describes and cannot be overridden.
Counted as `types_unresolved`, never silently dropped.

### D10 — `EnableTagId::try_from_component_id` is a required `boyko_ecs` seam

B.4's presence view is **not implementable from an external crate today**: there is no
public `ComponentId → EnableTagId` constructor (see §Verified facts). The seam:

```rust
// boyko_ecs::…::component_registry::tags
impl EnableTagId {
    /// The inbound half of the documented round-trip. `None` unless `id` is
    /// classified `StorageKind::Bitset` — so a Table/Dense id can never be smuggled
    /// into the enable-bit surfaces.
    pub fn try_from_component_id(id: ComponentId) -> Option<Self>;
}
```

It is **justified on its own merits**, independently of reflection: `EnableTagId` already
advertises `component_id()` as *"bridges to the shared `ComponentId` space"* and the crate
has an internal round-trip test (`component_registry/mod.rs:1676`) asserting a property the
public API does not offer. It is total, safe, `storage_kind`-checked, and adds no new
capability — every enable/disable it enables is already reachable from inside the crate.

This is the **second** instance of `REFLECTION-ANALYSIS.md` §4's finding — *a dev-only
feature widening a SHIPPING crate's public surface* — and like the first it is an **OWNER
call on API surface**. It is much smaller than the first (one constructor versus a
structural migration seam), and unlike the first it is a *completion* of an existing
documented bridge rather than a new one.

> **RESOLVED 2026-08-21 (second pass): `REFLECTION-PLAN-ECS.md` claims it, it lands there as S4′, and
> it is NOT a separate owner question.** Two facts closed this:
>
> 1. **The sibling plan specified the opposite seam and gated against this one.** Its D4 *rejected*
>    `EnableTagId::from_component_id` — on the reasonable ground that it mints a transferable
>    capability token where `is_enabled_raw(e, id) -> Option<bool>` merely answers a question — and
>    its EG0 landed a `trybuild` `compile_fail` fixture asserting the item **must not exist**. So the
>    two plans specified mutually exclusive seams for one view, one of them mechanically gated against
>    the other, and the owner was about to receive the same decision twice with opposite
>    recommendations.
> 2. **The substitute does not work.** ECS §4 justified rejecting the constructor partly with *"the
>    write half needs nothing at all"* — the route `display_name(id) → register_enable_tag(name) →
>    enable_id`, on the strength of `register_enable_tag` being *"idempotent by name"*. It is
>    idempotent **within `TAG_NAMES`**, and a **derived** `#[component(storage = "bitset")]` type
>    never interns its name there — which is exactly this section's own D9 finding, arrived at from
>    the serialize side and not connected to the write side by either plan. On a derived bitset id
>    that call **mints a new dynamic tag** (`try_register_tag_by_name`'s miss path →
>    `try_register_dynamic(ComponentLayout::new_dynamic_tag(leaked))`), clears *that* bit, and leaves
>    the component's own bit set — returning success.
>
> So the constructor is required for **both** halves, not only the read, and it is **one item on one
> four-item owner call**: analysis **B.13 #2**, owned by `REFLECTION-PLAN-ECS.md`'s **EG2** as **S4′**.
> This plan's B4 blocks on that answer and **does not ask separately**; its own B-1 row is retired
> into B.13 #2 below.

The boundary blocks on it either way (rung B4).

### D11 — `Entity`-valued fields are written as raw bits and NOT remapped in v1

`ValueKind::Prim(EntityId)` is written verbatim. A dump's entity references are valid only
in the world that produced them. `ApplyReport.fields_entity` counts them so a tool can
warn, and the acceptance fixture asserts the count.

**Rejected:** reuse `SerializeInfo.map_entities_fn`. Remapping needs a load-time entity
map, which is a *save-format* concern (`LoadEntityPolicy::Remap` and the whole
`entity_remap.rs` surface). Importing it would breach D1 in spirit and in dependency.
Deferred to v2 with the collections work, where a prefab-scoped remap makes sense.

### D12 — The four citizens are typed refusal RECORDS, never absences or zeros

Following B.4's matrix, the dump writes one of:

| citizen | dump record | apply behaviour |
|---|---|---|
| `Table`/`Dense` + `Cpu`/`CpuPinned`, reflectable | `BeginType` … fields … `EndType` | full apply |
| `Table` + `Cpu`, no `reflect` opt-in | `Refused(NotReflectable)` | counted, skipped |
| `Table` + `Gpu` | `Refused(GpuResident)` | counted, skipped |
| `Bitset` (derived enable tag) | `Presence(bool)` keyed by D9's name | `enable_id`/`disable_id` via D10 |
| dynamic tag (`type_info_of == None` forever) | `Presence(true)` keyed by the interned tag name | `EcsMaster::tag_by_name` (`tag_api.rs:76`) — **public**, unlike D9's case |
| `#[reflect(skip)]` field | `Refused(Skipped)` inside the type block | counted, skipped |

The **asymmetry is worth naming**: a *dynamic* tag has a public name→id route
(`tag_by_name`); a *derived bitset* tag does not (D9/D10). The runtime-minted citizen is
better served by the public API than the compile-time one.

Zero-filling a GPU-resident component would produce a dump that looks complete and
applies garbage — invariant 5 exists for that case.

### D13 — The Aether seam emits a `reflect` key into the EXISTING `component_attr` vector

```
aether! { component Health { reflect, hp: f32 } }
    ↓  aether_lang::expand::component  — one push into the existing keys vector
#[derive(::boyko_macros::Component)]
#[component(reflect)]
pub struct Health { pub hp: f32 }
    ↓  boyko_macros::component  — the seventh install slot, cfg-wrapped
#[cfg(feature = "reflect")] impl boyko_reflect::Reflect for Health { … }
#[cfg(feature = "reflect")] static TYPE_INFO: boyko_reflect::TypeInfo = …;
// inside component_id():
#[cfg(feature = "reflect")]
boyko_reflect::install_type_info(raw, &TYPE_INFO);
```

**Why this and not `#[cfg_attr(feature = "reflect", derive(::boyko_reflect::Reflect))]`
emitted straight from the expander.** Both forms respect tokens-not-deps — the rule is
about *crate dependencies*, and `aether_lang` acquires none either way. The rule does not
decide it; **drift does**:

* `#[component(reflect)]` makes `aether_lang` emit **one key on an attribute it already
  emits**. It adds **zero new names** to the DSL's downstream vocabulary — the expander
  already emits `#[component(no_bundle)]`, `#[component(storage = "bitset")]` and
  `#[component(on_add = path)]` through the same vector.
* The `cfg_attr` form would make `aether_lang` emit **two new names it cannot check**: the
  literal crate path `::boyko_reflect::Reflect` and the literal feature string `"reflect"`.
  A rename on either side breaks the DSL silently — and `crates/aether_tests` exists
  precisely to catch that class (its manifest calls it the *"§8 R4 anti-drift gate"*, and
  records that pulling the render stack behind it cost a cold check ~1 s → ~31.7 s, bought
  deliberately). Adding names the gate must chase is moving in the wrong direction.
* **One owner for the reflect vocabulary: `boyko_macros`.** The DSL names a key; the derive
  owns what the key means.

**This is also an argument that feeds back into `REFLECTION-PLAN-CORE.md`:** the Aether
seam is only this cheap if the opt-in surface is A.5's **`#[component(reflect)]` helper
attribute**. Under §2's original two-derive `cfg_attr` sketch, the DSL must emit the crate
path and the feature name itself. **The Aether seam is therefore independent evidence for
A.5's form over §2's**, and CORE should record it as such.

### D14 — Aether reflection is OPT-IN, and the silent-omission objection is answered by the row, not by the default

Engineering default: **opt-in** (`reflect` written in the block). `REFLECTION-ANALYSIS.md`
B.11 #3 records this as an **OWNER** call on DSL ergonomics; the recommendation and its
cost are stated here so the owner can flip it in one line.

The argument against opt-in is A.5's, and it is strong: a forgotten reflection
registration makes a component *silently invisible in the inspector*, indistinguishable
from "this entity doesn't have that component" — a **correctness** failure of the tool.
That argument is what made registration lazy rather than explicit, and it applies verbatim
to a forgotten `reflect` key.

**It is answered without defaulting on, because B.4's matrix already contains the answer.**
Enumeration walks **ids**, not `TypeInfo`s: a component with no opt-in is still enumerated
and rendered as *"not reflectable"* (B.4 row 2). The failure mode of a forgotten key is a
**labelled row**, not an absence — so the intolerable failure A.5 names cannot occur. This
is the fifth independent use of the shared *"known-but-not-viewable"* row state (A.5's
correction and B.4's three refusal rows are the other four); it is a single UI affordance
that pays for itself repeatedly.

**The cost of flipping to default-on, stated precisely so the owner can weigh it:** every
crate containing an `aether!` block would then emit `#[cfg(feature = "reflect")]` tokens,
and under D16 every such crate must declare a `reflect` feature or go **red** under the
existing `-D warnings` gate — including `crates/aether_tests` and every future consumer
that does not care about reflection. Default-on trades a labelled row for a manifest
requirement on people who never asked for reflection. Opt-in does not.

### D15 — `tag` gains NO `reflect` key, which deletes B.5(b)'s span hazard by construction

B.5(b) warns that `tag Foo(bitset);` can produce exactly the type the derive refuses, so
*"the user wrote `tag Foo(bitset);` and would get an error about a derive they never
typed"*, and prescribes keeping the refusal `quote_spanned!` at the user's name.

Because `reflect` is a key on the **`component`** construct only, and Aether's `component`
never emits `storage = "bitset"` (only `tag` does), **`reflect` and `bitset` cannot
co-occur on the Aether path at all.** The hazard is structurally eliminated rather than
mitigated.

Two things follow and both are rungs, not remarks:
* a `tag`'s only datum is presence, which the inspector obtains from enumeration (D12)
  with no `TypeInfo` — so the key would buy nothing even if it existed;
* **hand-written Rust can still write `#[component(storage = "bitset", reflect)]`**, so
  `boyko_macros` still owes the spanned refusal. Rung B6 gates it with a trybuild fixture
  whose `.stderr` pins the span.

### D16 — A missing `reflect` feature is a diagnostic, not silence

`#[cfg(feature = "reflect")]` in a crate that declares no such feature is **not an error**
— the items simply vanish. That is a silent, feature-shaped instance of the exact failure
A.5 says reflection cannot tolerate, arriving through a different door.

It is caught mechanically, and only because the workspace already carries the instrument:
`[workspace.lints.rust] unexpected_cfgs` (root `Cargo.toml:25-26`) with every member
opted in, promoted to red by the existing `cargo clippy --workspace --all-targets --
-D warnings` gate.

Two things must be **verified by a compile, not assumed** (A.5's own lesson: *"this is a
proc-macro/Cargo mechanism that only a compile settles"*), and rung B6 verifies both:
1. that `unexpected_cfgs` actually fires for a `cfg` emitted from **derive output** (a
   macro-generated attribute), and
2. **where it points** — if the span lands on `aether! {` rather than on the user's
   `reflect` key, the diagnostic is useless and the expander must `quote_spanned!` the
   emitted key at the user's `reflect` ident, exactly as A7 did for the struct name.

### D17 — The dump has its own magic, and each reader rejects the other's file

`MAGIC = *b"BOYKORD1"` — eight bytes, deliberately distinct from `boyko_serialize`'s
`*b"BOYKOSAV"` (`format.rs:22`). A save file fed to `apply` is a clean
`SourceError::BadMagic`, and a dump fed to `load_world` is the shipped
`LoadError`'s bad-magic path (already tested: `load_roundtrip.rs::bad_magic_is_rejected`).

Cheap, and it makes D1's separation *checkable* rather than merely intended.

### D18 — The dump's type table is sorted by stable name, so a dump is process-order-independent

A multi-type dump carries a prologue of type declarations, and blocks thereafter reference
a **dump-local `u16` index** — the `boyko_serialize::resolve_type_table` shape, which is
what makes resolution once-per-type instead of once-per-entity (W2).

The prologue is **sorted by stable name**, never by `ComponentId` and never by enumeration
order. Without this, two processes with different registration orders would produce
different bytes for the same entity *even though nothing id-shaped leaked*, and rung B5's
byte-identity gate would be untestable. With it, the dump is deterministic and diffable,
matching the discipline `boyko_serialize/tests/save_determinism.rs` already holds the
saver to.

**The dump-local index is not an id.** It is defined by the dump's own prologue three
records earlier; invariant 1 is about *process* identifiers.

### D19 — The container is TRIMMED to what a v1 consumer names, and every shape borrowed from `boyko_serialize` gets an anti-drift gate

This plan is the largest single rung block in the campaign and the one whose value is least
established. It grew a full container format — an 8-byte magic (D17), a type-table prologue sorted by
stable name (D18), a dump-local `u16` index, a `format_version` refusal policy (D7), patch-vs-replace
semantics (D8), an 8-field `ApplyReport`, `ApplyPolicy { Strict, Lenient }`, and multi-entity dumps —
against an analysis scope of *"boundary serialize (Sink/Source) + name-keyed roundtrip incl. simulated
id-reorder"* (§7 Wave 4). Those shapes are hand-rebuilds of `boyko_serialize`'s `SaveHeader` /
`TypeTableEntry` / `resolve_type_table` / `LoadReport`, **adopted by D1 precisely so the two formats
cannot drift** — and with **no anti-drift gate of the kind D8/B.2 demand everywhere else in this
campaign. That omission is the real defect here**, and it is fixed first.

**(a) The anti-drift gate, which was missing.** D1 forbids the dependency, so the gate cannot be a
type-level one. It is `crates/boyko_reflect/tests/format_divergence_ledger.rs`, and it asserts a
**ledger**, not an equality:

| borrowed shape | `boyko_serialize`'s | the dump's | divergence |
|---|---|---|---|
| magic | `*b"BOYKOSAV"` (`format.rs:22`) | `*b"BOYKORD1"` | **intended** (D17) — and the test asserts they *differ*, so a copy-paste that unified them reds |
| type-table entry | `TypeTableEntry`, 40 B, offsets `const _: () = assert!`-pinned (`format.rs:210-258`) | `TypeDecl`, its own `const` offset pins | **intended** — the dump carries no `size`/`align`/`serializability`; the ledger names each omitted field so a *silent* omission is impossible |
| resolution shape | `resolve_type_table` → dense `Vec<ResolvedType>` by file-local index (`load.rs:340-408`) | the same shape | **adopted deliberately**; the ledger row is the statement that it was adopted, so a later divergence is a decision |
| version guard | `LoadError::VersionMismatch`, hard (`load.rs:198-200`) | hard by default (D7) | **adopted** |
| fingerprint guard | hard error before a POB blit | **recorded, not gated** (D7) | **intended** — the one row where the dump is deliberately more permissive, and B4 gate 4 asserts it |
| leniency accounting | `LoadReport { types_skipped, … }` (`load.rs:81-111`) | `ApplyReport` | **adopted** |

The test reads `boyko_serialize`'s pinned constants **through its own `const` assertions re-declared
locally** (there is no dependency to read them through), and reds when a re-declared value stops
matching the comment naming its source. That is weaker than a compile-time link and it is the
strongest form D1 permits; the ledger says so in its header rather than implying more.

**(b) Two items leave v1, because no v1 consumer names them.**

* **Multi-entity dumps.** `dump_entity` stays — *"copy/paste a component, author a prefab, dump an
  entity into a bug report"* are all single-entity, and B5's headline (byte identity across differing
  ids over a 4-component entity) needs the sorted prologue, so **the prologue stays too**. What goes
  is the *repetition*: `BeginEntity`/`EndEntity` remain in the grammar as the framing of the one
  entity, and `BeginDump` pins `entity_count == 1` in v1. A dump of N entities is a v2 arm, and it
  costs one relaxed assertion to add.
* **`ApplyPolicy::Lenient`.** `Strict` is the editor's behaviour and the one B3/B4's gates assert.
  Lenient counts-and-continues has no named v1 consumer, and it doubles the assertion surface of
  every counted-leniency gate. `ApplyPolicy` stays as the parameter (so adding the arm is not a
  signature change) with `Strict` its only variant in v1.

**What deliberately does NOT leave.** The prologue (B5's byte-identity gate is unbuildable without
it); `format_version` refusal (it has a stated precedent, a one-line implementation and a cheap gate
at B4 gate 3, and *"a name-keyed apply is not smarter here, only quieter, and quieter is worse"*);
the 8-field `ApplyReport` (invariant 4 — every count is asserted by number somewhere in the ladder,
and a field that is never asserted is the dead-datum class this campaign has five instances of).

---

## The event stream (normative)

```rust
#[non_exhaustive] #[derive(Clone, Copy, Debug)]
pub enum SinkEvent<'a> {
    BeginDump   { type_count: u16, entity_count: u16 },  // v1 pins entity_count == 1 — D19(b)
    TypeDecl    { index: u16, stable_name: &'a str, format_version: u16, layout_fingerprint: u64 },
    BeginEntity,                                   // the framing of the one entity in v1; the
                                                   // repetition (N entities) is a v2 arm — D19(b)
    BeginType   { index: u16 },                    // dump-local index, D18
    EndType,
    BeginField  { name: &'a str },                 // ALWAYS by name, D-invariant 1
    EndField,
    Scalar(Scalar),                                // Prim, by copy
    Str(&'a str),                                  // borrows the live buffer — zero alloc (A.3)
    BeginArray  { len: u32, elem: ValueKind },     // [T; N] of a Prim (B.8)
    EndArray,
    BeginNested { stable_name: &'a str },          // depth ≥ 1 (A.3's one `add` per level)
    EndNested,
    Variant     { name: &'a str, discr: Scalar },  // fieldless #[repr(Int)] — field-level AND top-level (§6.1(c))
    Presence(bool),                                // B.4 bitset / dynamic tag
    Refused(RefusalKind),                          // D12 — NotReflectable | GpuResident | Skipped | Opaque
    EndEntity,
    EndDump,
}

const _: () = assert!(core::mem::size_of::<SinkEvent<'_>>() <= 48);
```

`SourceEvent<'a>` mirrors it arm for arm; the byte encoding is one `u8` tag per arm plus
that arm's payload (D5), so **the event sequence and the byte image are the same
statement** and rung B5's golden pins both.

A **top-level enum component** emits `BeginType` → `Variant` → `EndType` with no
`BeginField` (§6.1(c) — a component that *is* an enum has no fields, and `fields_of`
returns `&'static []`). `FixVis` is the fixture-side case (B0/B2); `Visibility`
(`boyko_scene/src/render_caps.rs:226`, discriminants pinned *"so the byte is stable across
serialization"*) is the dogfood acceptance case — B5's dogfood half, in `reflect_dogfood`.

### Public API

```rust
pub fn dump_component(ecs: &EcsMaster, e: Entity, id: ComponentId, sink: &mut dyn Sink)
    -> Result<(), SinkError>;
pub fn dump_entity(ecs: &EcsMaster, e: Entity, sink: &mut dyn Sink)
    -> Result<DumpReport, SinkError>;

pub fn apply_component(ecs: &mut EcsMaster, e: Entity, id: ComponentId,
                       src: &mut dyn Source, policy: ApplyPolicy)
    -> Result<ApplyReport, ApplyError>;
pub fn apply_entity(ecs: &mut EcsMaster, e: Entity,
                    src: &mut dyn Source, policy: ApplyPolicy)
    -> Result<ApplyReport, ApplyError>;
```

`apply_component` takes an **explicit target `ComponentId`** — that is what lets an editor
paste `Foo`'s values onto a *different but name-compatible* type, and it is what makes
rung B5's field-permutation test expressible.

```rust
pub struct ApplyReport {
    pub fields_applied:   u32,
    pub fields_missing:   u32,  // in the type, absent from the stream — D8 patch semantics
    pub fields_unknown:   u32,  // in the stream, absent from the type
    pub fields_kind_mismatch: u32, // set(..) returned false — the release-present check (§5)
    pub fields_entity:    u32,  // written verbatim, NOT remapped — D11
    pub types_unresolved: u32,  // stable name resolved to nothing here — D6
    pub types_refused:    u32,  // D12
    pub versions_refused: u32,  // format_version mismatch — D7
}
```

Every field is asserted **by number** somewhere in the ladder. A count nobody asserts is
leniency nobody can see.

---

## Rung ladder

Unconditional gate on every rung: `cargo clippy -p boyko-reflect -p reflect-fixture
-p boyko-macros -p aether-lang -p aether-tests --all-targets -- -D warnings`; `cargo test
-p <crate> --all-targets --no-fail-fast` for every crate the rung touches (for
`reflect-fixture`, **both** feature legs — with `--features reflect-fixture/reflect` and
without); Miri-TB where the rung adds `unsafe` (**and only after
`REFLECTION-PLAN-GATES.md`'s G4 has added BOTH Miri rows — `-p boyko-reflect` **plain**
and `-p reflect-fixture --features reflect-fixture/reflect` — until then a Miri claim here
is a gate that cannot fail, B.9.** ~~`-p boyko-reflect` … with the feature ON~~ was this
preamble's own copy of the B.9 error corrected at B7 and owned by GATES **D4**: the crate
has no `reflect` feature, `--features reflect` on it is a hard cargo error, and with the
feature "off" it is not empty); author-only commit.

Per-worktree build discipline: `-p <crate>`, never `--workspace` (disk).

**Where each rung's tests live — stated once, because as first written every rung below
said `crates/boyko_reflect/tests/`, and most of them cannot.** A test that constructs a
`#[component(reflect)]` component cannot live in `boyko_reflect`'s own tests: the crate
declares no `reflect` feature (GATES D4, "now or ever"), so the derive's consumer-side
`#[cfg(feature = "reflect")]` there is an `unexpected_cfgs` red under the existing
`-D warnings` gate (D16's own mechanism, pointed at this plan) — and with no feature to
turn on, the derive's emission is stripped besides. And a test that names a real engine
type cannot live in `reflect_fixture` either: its dependency table is
`boyko-ecs`/`boyko-macros`/`boyko-reflect` **only** (GATES D15 — it is the Miri row's
package and must stay FFI-free). So:

* **`crates/boyko_reflect/tests/`** — only tests that construct no component: B1's `dyn`
  and one-method source censuses, B4's `format_divergence_ledger.rs`.
* **`crates/reflect_fixture/tests/`** — every fixture-driven rung: B0's fixtures and both
  id-harness binaries, B1–B4's behavioural gates, B5's instrument half, B7. Feature-ON.
* **`crates/reflect_dogfood/tests/`** — B5's dogfood half only (real engine types;
  B.12/B.13 #1).

> **Every rung states a RED MUTATION.** A gate whose red nobody has seen is not a gate.
> The mutation is applied, the red is *observed and reported*, the mutation is reverted.
> This campaign has paid for that lesson at least five times (`site.decode`, `LogSite.fields`,
> twelve unbuilt benches, a `sample_shift` that sat in the control for two rungs, an
> `intern_site` that failed silently) — the class is *"the dead datum"*, and it is a class,
> not a defect.

### B0 — The fixture crate and the id-difference harness — size S

**Lands.** `crates/reflect_fixture/tests/fixtures/` (a module, not a crate — **the
fixture package's**, per the placement rule above; every type here is
`#[component(reflect)]` and cannot live in `boyko_reflect`'s own tests): `Pod3 { a: u32,
b: f32, c: i16 }` with an explicit `#[component(stable_name = "reflect::fixture::Pod3")]`
(D6); `NestPair { inner: Pod3, tail: u8 }`; `ArrPack { m: [f32; 4] }`; `FixVis`, a
locally-declared fieldless `#[repr(u8)]` enum with **pinned** discriminants, as the
top-level-enum case — ~~a re-export of `Visibility`~~ is not expressible here
(`Visibility` is `boyko_scene`'s, out of this package's dependency table; the real
`Visibility` is B5's dogfood half, in `reflect_dogfood`); `StrFixture { s: String }`
(unused until the `Str` rungs — built last per §6.1(d)); `Decoy` (a distinct POD used only
to prove a mis-keyed apply corrupts something).

The **id-difference harness**: two test binaries over the same fixtures.
* `crates/reflect_fixture/tests/boundary_roundtrip.rs` — touches fixtures directly.
* `crates/reflect_fixture/tests/boundary_id_reorder.rs` — a `OnceLock` prelude that runs **before any fixture
  `component_id()` touch**, minting `K` spacer ids via `EcsMaster::register_tag`
  (`tag_api.rs:65`; tags draw from the same `NEXT_ID` counter as `register_new`,
  `component_registry/mod.rs::register_new`), then touching `Decoy`, then the fixtures.
  Every test in the file calls the prelude first, so thread order is irrelevant. Separate
  `tests/*.rs` files are separate processes, so the two binaries cannot contaminate each
  other.

**Gate.** `boundary_id_reorder.rs` asserts, as its first executable statement after the
prelude, that the ids genuinely moved:
`assert_ne!(Pod3::component_id().0, CAPTURED_POD3_ID)` where `CAPTURED_POD3_ID` is the
constant committed beside rung B5's golden blob.

**`K` is chosen against the remaining budget, and the budget is MEASURED, not assumed.** Dynamic tags
draw from the **same** 512-id `NEXT_ID` counter as every typed component
(`component_registry/tags.rs` → `try_register_dynamic`), and `try_register_tag_by_name` returns
`None` once `NEXT_ID >= MAX_COMPONENTS`, which `EcsMaster::register_tag` turns into
`register_enable_tag_exhausted_panic`. A `K` picked by eye works until a future test-binary link
order pushes this binary's registrations past 512 — and then it panics in a kernel function whose
name mentions neither reflection nor this test, and nobody connects the two.

So the prelude reads the high-water mark before it spends any of it (there is no public accessor —
`next_id_for_test` is `pub(crate)` — so it **probes**):

```rust
let probe = ecs.register_tag("__reflect_b0_probe").component_id().0;   // == the current NEXT_ID
let budget = MAX_COMPONENTS - probe - 1;
assert!(K + FIXTURE_TYPE_COUNT <= budget,
        "B0's spacer count K={K} plus {FIXTURE_TYPE_COUNT} fixture types exceeds the {budget} ids \
         left in this binary's shared 512-id budget (probe landed at {probe}). Lower K, or split \
         this test binary — do NOT let it reach register_enable_tag_exhausted_panic, whose message \
         names neither reflection nor this harness.");
```

`K` is then the smallest value that satisfies B5's needs, not the largest that fits: the assertion
exists to make exhaustion a **named** failure, and the message is the deliverable.

**RED MUTATION.** Set `K = 0` in the prelude. The `assert_ne!` reds. *This is the rung's
entire point*: without it, every later "survives an id reorder" claim is a gate that
passes because there was no reorder — the failure this campaign has recorded five times.

**Second RED, for the budget clause:** set `K = MAX_COMPONENTS`. The **budget assertion** must red,
with its own message — **not** `register_enable_tag_exhausted_panic`. If the panic wins the race, the
probe is in the wrong place and the clause is decorative.

### B1 — `Sink`: one trait, one method, one `dyn`, zero allocations — size M

**Lands.** `trait Sink` (D2); `SinkEvent` + the `size_of <= 48` pin; `SinkError`
(`UnsupportedKind`, `Truncated`, `Sink(u32)` for a caller code); `VecSink` over
`&mut Vec<u8>` with the D5 encoding; the `prim::emit_*` fn-ptr library for every
`ScalarKind`; `dump_component` for the `Prim`-only case.

**Gate.**
1. **`dyn` census** — a test walks `crates/boyko_reflect/src/**/*.rs` and asserts the set
   of lines containing `dyn ` equals a committed allowlist (by file + matched text, not
   line number). Modelled on the tree's existing source censuses
   (`scripts/check_hotpath_exceptions.py`, the particle artifact census).
2. **One-method census** — the same walker asserts the `trait Sink { … }` block contains
   exactly one `fn`. This is the only honest gate available: `size_of::<&dyn Sink>()` is
   two words regardless of method count, so **a size assertion here would be a gate that
   cannot fail** and is deliberately not written.
3. **Zero-allocation delta** — the `p4_bind_zero_alloc.rs` counting-allocator + baseline-
   subtraction harness. Baseline: a `VecSink` over a pre-reserved buffer driven with a
   hand-written event sequence. Subject: `dump_component` over `Pod3`. **Delta = 0 bytes,
   0 calls.** The figure is *measured and reported in the commit message*, not predicted.
4. **Unknown kind is an error** — a sink whose `_ =>` arm returns `Ok(())` is rejected by
   a test that feeds it a synthetic future-kind event and asserts `Err` (D4).

**RED MUTATION.** (1) add a `Box<dyn Fn…>` field to `VecSink` → census reds. (2) add a
second method to `Sink` → census reds. (3) make `VecSink::event` call
`self.buf.reserve(1)` unconditionally → the allocation delta goes non-zero. All three
observed before the rung lands.

### B2 — The dump grammar for the v1 kinds — size M *(prerequisite: CORE's `ValueKind::Array`)*

**Lands.** `dump_component` for `Nested` (any depth), `Array` of a `Prim`, field-level
`Enum`, **top-level `TypeKind::Enum`** (§6.1(c)); `Refused(Skipped)` for
`#[reflect(skip)]`; `DumpReport`.

Build order inside this rung follows §6.1(d): **arrays first** (`ArrPack`, then a local
reproduction of `GpuTransform3D`'s `TrsPacked` shape — the case that is a *hard error*
today, B.8), then nested (`NestPair`, the `Transform → Vec3/Quat` shape), then enums
(`FixVis`; the real `Visibility` is B5's dogfood half).

**Gate.**
1. **Event-sequence pin** — the exact `SinkEvent` discriminant sequence for each fixture,
   compared against a literal `&[&str]`. Because of D5 this is simultaneously a byte-image
   statement.
2. **Depth-2 descent** — `NestPair → Pod3 → b: f32` reaches the leaf and the leaf's bits
   equal the source bits exactly (raw bits, D5 — no float comparison, no epsilon).
3. **Array stride** — an `[f32; 4]` emits exactly 4 `Scalar` events and the 4 values are
   the 4 elements in order; a 3-element read would pass a naive "reads something" test and
   fails this one.
4. **Top-level enum** — `FixVis` emits `BeginType`/`Variant`/`EndType` with **zero**
   `BeginField` events, and the discriminant equals its pinned value (the fixture pins its
   discriminants for exactly the reason `Visibility` pins `Hidden = 2`; the real
   `Visibility` run of this grammar is B5's dogfood half).
5. **Zero-allocation delta** for every kind above (harness from B1). `Str` is **not** in
   this rung.

**RED MUTATION.** Change the array walk's stride from `size_of::<f32>()` to
`size_of::<f32>() + 1` → gate 3 reds on values 2..4 (gate 1 alone would not catch it,
which is why gate 3 exists separately). Change the top-level-enum emission to walk `fields_of`
→ gate 4 reds with 0 `Variant` events.

### B3 — `Source` + `apply`: name-keyed resolution, patch semantics, counted leniency — size M

**Lands.** `trait Source` (D3); `SliceSource`; `apply_component` with name-keyed field
resolution (linear scan over `&'static [FieldInfo]` — few fields, zero alloc);
`ApplyPolicy` — **`Strict` its only variant in v1** (D19(b): `Lenient` has no named v1 consumer and
doubles the assertion surface of every counted-leniency gate; it stays a *parameter* so adding the
arm later is not a signature change); `ApplyReport`; `ApplyError`.

**Gate.**
1. **Roundtrip equality** — dump → apply into a fresh entity → every field bit-equal.
2. **The lending borrow compiles the contract away** — a trybuild fixture that holds a
   `SourceEvent` across a second `next()` call and **fails to compile**, with the
   `.stderr` pinned. This is D3's whole claim; without the fixture the claim is prose.
3. **Counted leniency, by number** — a stream with one unknown field name gives
   `fields_unknown == 1` and `fields_applied == n-0`; a stream missing one field gives
   `fields_missing == 1` and **the target field is unchanged from its pre-apply value**
   (D8, asserted on the value, not only on the count).
4. **Kind mismatch is not a panic and not a silent write** — feed an `f32` event at a
   `u32` field: `set` returns `false`, `fields_kind_mismatch == 1`, the target byte is
   unchanged, and `Strict` turns it into an `Err`. This is §5's release-present check
   (`-> bool`, **never** a `debug_assert!`, because the legitimate
   `--release --features reflect` editor build compiles those out).
5. **Zero-allocation delta** on `apply_component` for a `Str`-free type.

**RED MUTATION.** Change the field resolution from `name ==` to "by index" → gate 1 still
passes for `Pod3` (same order!) and **gate 3 reds** — which is precisely why B5's
permutation test exists and why this rung's gate is not sufficient on its own. Change
the `Strict` arm to swallow the mismatch as `Ok` → gate 4's count reds.

### B4 — The type layer: stable-name keying, once-per-dump resolution, the citizen matrix — size M *(prerequisite: D10's `boyko_ecs` seam)*

**Lands.** The `BeginDump`/`TypeDecl` prologue sorted by stable name (D18); the
once-per-dump `Vec<Option<ComponentId>>` materialization (the
`boyko_serialize::resolve_type_table` shape, `load.rs:340-408`); `format_version` refusal
(D7); `layout_fingerprint` recorded-not-gated (D7); D9's bitset name scan; D12's refusal
records; `dump_entity` / `apply_entity` (**single-entity in v1**, D19(b)); **and the
`format_divergence_ledger` test D19(a) requires** — the anti-drift gate this plan owed for every
shape it borrowed from `boyko_serialize` and did not have.

**`EnableTagId::try_from_component_id` does NOT land here.** It is
[`REFLECTION-PLAN-ECS.md`](REFLECTION-PLAN-ECS.md)'s **S4′**, landing at its **EG2**, under the single
owner call `REFLECTION-ANALYSIS.md` **B.13 #2** (D10's resolution note). This rung **blocks on EG2**
and asks nothing separately. It cannot be skipped or worked around: without it the presence view is
unimplementable from an external crate, and the by-name substitute writes the wrong bit (D9/D10).

**Gate.**
1. **Once per type, measured** — a counter around `resolve_stable_name` (a test-only
   wrapper, or a `#[cfg(test)]` `AtomicUsize` in the boundary) asserts **exactly one call
   per distinct type** for a dump of 3 entities × 2 shared types → `2`, not `6`. This is
   W2's requirement turned into a number.
2. **Prologue determinism** — the prologue's name order equals the sorted order, asserted
   against a shuffled enumeration order (the test enumerates twice, in two different id
   orders, and compares prologue bytes).
3. **`format_version` refuses** — bump the fixture's `#[component(format_version = 1)]`
   between dump and apply (two fixture types, same stable name via the override) →
   `versions_refused == 1` and `Strict` errors. Mirrors
   `boyko_serialize/tests/version_mismatch.rs`.
4. **`layout_fingerprint` does NOT refuse** — the same pair with a *reordered* field set
   (different fingerprint, same `format_version`) **applies successfully**, by name. This
   is the positive statement of D7 and the negative statement of the loader's rule; both
   must be asserted or the asymmetry is folklore.
5. **The four citizens, one assertion each** — a `Gpu`-residency component yields
   `Refused(GpuResident)` and **not** a zero-filled field block; a derived bitset tag
   round-trips its bit through D9's scan and D10's constructor; a dynamic tag round-trips
   through `tag_by_name`; a non-opted-in `Table` component yields
   `Refused(NotReflectable)`.
6. **`has_component` is never the presence probe** — a source census asserting the string
   `has_component` does not appear in `crates/boyko_reflect/src/**`. It silently returns
   `false` for every bitset tag (`ecs_master/component_api.rs:673-702` has no `Bitset`
   branch), so an inspector reaching for it would **report the wrong answer rather than
   refuse** (B.4).
7. **`register_enable_tag` is never the presence *write*, either** — the same source census, second
   needle. On a **derived** bitset id that call mints a *different* tag and toggles its bit (D9/D10),
   which is a wrong answer that **round-trips correctly through its own toggle** and is therefore
   invisible to gate 5's bit assertion. The census is the only cheap instrument that sees it; the
   behavioural gate lives at `REFLECTION-PLAN-ECS.md` EG3 gate 7, where the caller is.
8. **The format-divergence ledger** (D19(a)) — every row present, every re-declared constant matching
   the `boyko_serialize` value its comment names, and the magic asserted **different**.

**RED MUTATION.** (1) hoist the resolve inside the per-entity loop → gate 1 reads `6`.
(2) sort the prologue by `ComponentId` → gate 2 reds. (3) make the fingerprint a hard
error (copy the loader's rule) → gate 4 reds, which is the correct signal that the two
formats have different obligations. (4) zero-fill the GPU component instead of refusing →
gate 5 reds. (5) set `MAGIC = *b"BOYKOSAV"` → the ledger reds on the row that exists to make D1's
separation checkable rather than intended, **and** gate 5 of B5 (cross-format rejection) reds — two
gates from one character, which is what "checkable rather than intended" buys.

### B5 — The name-keyed roundtrip and the SIMULATED ID REORDER — size M *(the headline)*

**Lands — in TWO packages**, the same split CORE C6/C10 and ECS EG8 carry, and for the same
reason: the byte-identity **instrument** must live where its ids are controllable and its
stable names explicit, and the **engine-types claim** must live where the engine crates are
reachable (the placement rule at the head of this ladder; B.12/B.13 #1).

* **Instrument half, `crates/reflect_fixture/`:** `tests/data/acceptance.rdmp` (the
  committed golden blob) + `acceptance_ids.rs` (`CAPTURED_POD3_ID` and friends, written by
  the capture, read by the reorder binary), over the **local** fixture entity —
  `{ Pod3, NestPair { inner: Pod3 }, ArrPack { m: [f32;4] }, FixVis }` plus `Decoy`. Every
  type carries an explicit `#[component(stable_name = …)]` (D6), which is what makes a
  *committed* golden blob refactor-stable — a blob pinned over engine types would move
  whenever an engine module moved, since the engine types' keys are bare `type_name`s.
* **Dogfood half, `crates/reflect_dogfood/`:** `tests/boundary_dogfood.rs` — dump → apply
  over the fixture entity ECS EG8 already builds, A.9's *corrected* shape:
  `{ Name(NameId(u32)), Transform { translation: Vec3, rotation: Quat, scale: Vec3 },
  Visibility (#[repr(u8)] fieldless), GpuTransform3D { prev: TrsPacked, curr: TrsPacked } }`.
  `Name` is the **tuple-struct + `Nested`** case (`"0"` field naming, A.6);
  `GpuTransform3D` is the **dense-storage + array** case and is *the single highest-value
  assertion in the ladder*, because a design built on the archetype signature alone would
  refuse to show the one component it is fully able to read (B.3). No golden blob here —
  the dogfood half asserts **roundtrip equality**, not byte pins, so an engine refactor
  moves no committed bytes.

**Gate — the third is the one that can fail for the right reason.**
1. **Roundtrip equality, both halves:**
   * `boundary_roundtrip.rs` (fixture): dump → apply → every field bit-equal, including a
     descent into `NestPair → Pod3`, an array element out of `ArrPack`, and a `FixVis`
     variant set + re-read;
   * `boundary_dogfood.rs` (dogfood): the same over the engine types — a descent into
     `Transform` **and** into `Name → NameId → u32`, an array element read out of
     `GpuTransform3D`, and a `Visibility` variant set + re-read.
2. **Byte identity across differing ids**, `boundary_id_reorder.rs` (fixture half, as are
   gates 3–5): the same local fixture entity dumped in the spacer-shifted process is
   **byte-identical** to the committed blob. Any `ComponentId`, field index or offset that leaked into the stream changes the
   bytes. Preconditioned on B0's `assert_ne!` — *the ids provably differ, so a pass means
   something*.
3. **Cross-process apply**, same binary: the committed blob (captured when `Pod3` had id
   `CAPTURED_POD3_ID`) applies cleanly, all values equal, **and `Decoy` — which now
   occupies a different id in this process — is unchanged**. An id-keyed implementation
   would either miss `Pod3` or write into whatever now holds that id; the `Decoy`
   assertion turns a silent corruption into a red.
4. **Field-permutation apply**: two fixture types `PermA { a: u32, b: f32 }` and
   `PermB { b: f32, a: u32 }`. Dump `PermA`, `apply_component(.., PermB::component_id(),
   ..)`, assert `b == b` and `a == a` **by value**. The test **first asserts the two types'
   field indices genuinely differ** (`fields_of(PermA)[0].name != fields_of(PermB)[0].name`)
   — without that precondition it is a gate that cannot fail.
5. **Cross-format rejection** (D17): `apply` on a `boyko_serialize` save blob →
   `SourceError::BadMagic`; `load_world` on a `.rdmp` → the shipped bad-magic error.

**RED MUTATION.** Add `component_id` to `TypeDecl` and write it. Gate 1 still passes
(same process). Gate 2 **reds** on byte identity. Gate 3 **reds** on values or on `Decoy`.
That divergence — one gate green, two red, from a single one-line change — is the proof
that gates 2 and 3 are measuring something gate 1 cannot see, and it is reported as such.
Second mutation: swap the field resolution to by-index → gate 4 reds with `a` and `b`
transposed while gates 1–3 stay green.

### B6 — The Aether seam — size M

**Lands.** Four files, no new dependency anywhere.

1. **`aether_lang/src/ast.rs`** — `ComponentDef` gains `pub reflect: bool` (the sixth
   field).
2. **`aether_lang/src/parse.rs::parse_component`** — `reflect` as a bare flag key,
   modelled exactly on `no_bundle` (`:273-278`), with **three touch points, and the third
   is the one a careless patch misses**:
   * the flag branch itself (duplicate `reflect` → a spanned error);
   * the item-head error string at `:230` and the field-fallback error string at `:295`
     (both enumerate the known items and would otherwise lie);
   * **the `requires`-continuation lookahead at `:256-263`** — `reflect` must join
     `"requires"` / `"no_bundle"` / `HookKind::from_str` in the `is_item_head` set, or
     `requires A, reflect` parses `reflect` as a *path* and the key silently vanishes.
3. **`aether_lang/src/expand.rs::component`** — `if def.reflect { keys.push(quote! {
   reflect }); }`, one line in the existing vector. The `#[component(...)]` attribute is
   already emitted only when the vector is non-empty, so a block without `reflect` emits
   byte-identical tokens to today.
4. **`boyko_macros/src/component.rs`** — `parse_component_hooks` gains the `reflect` bare
   flag (the sixth instance of that shape); a `reflect_install` `TokenStream2` appended as
   the **seventh** install slot in the `component_id()` funnel (`:350-374`), and the
   `#[cfg(feature = "reflect")]`-wrapped `Reflect` impl + `TYPE_INFO` static. The refusal
   for `storage = "bitset"` + `reflect` is spanned at the offending key.

**Gate.**
1. **`aether_lang` acquires no dependency** — `cargo tree -p aether-lang` output is pinned
   against a committed expectation. Its manifest's tokens-not-deps comment
   (`Cargo.toml:6-11`) is the invariant; the pin is the instrument.
2. **Zero-token identity for the unchanged path** — an expander unit test asserting that a
   `component` block **without** `reflect` produces byte-identical `TokenStream` output to
   the pre-change expander (the string form is compared). This is the DSL's 0 %-gate.
3. **The lookahead** — a parse test for `component Foo { requires A, reflect, hp: f32 }`
   asserting `def.reflect == true` **and** `def.requires == [A]`. Without the `:256-263`
   edit this test fails with `reflect` swallowed into `requires`.
4. **End-to-end against the real engine**, in `crates/aether_tests` (the §8 R4 anti-drift
   gate — its whole purpose): a `component Health { reflect, hp: f32 }` block that
   compiles with the crate's `reflect` feature **on** and produces a working `TypeInfo`,
   and compiles with it **off** to nothing. Both legs run in the same CI matrix cell,
   because the off-leg is the one that proves absence.
   ⚠️ **Wiring hazard, and it is a Cargo rule, not a preference — MEASURED 2026-08-21 on
   this toolchain**, not inferred: every engine edge in `crates/aether_tests/Cargo.toml`
   today lives in **`[dev-dependencies]`**, and Cargo **rejects `optional = true` on a
   dev-dependency** at manifest-parse time:
   `error: failed to parse manifest … Caused by: dev-dependencies are not allowed to be
   optional: 'libc'` (reproduced on a scratch crate; exit 101 before any compilation).
   So `reflect = ["dep:boyko-reflect"]`
   cannot be wired the way the rest of that manifest is wired. The optional
   `boyko-reflect` edge must go in **`[dependencies]`** (the crate's `lib` is empty, so
   this costs nothing), with `[features] reflect = ["dep:boyko-reflect"]` beside it.
   Verify by compiling both legs before writing the gate — this is another
   only-a-compile-settles-it item in the same family as D16.
5. **The bitset refusal is spanned at the user's key** — a trybuild fixture with
   `#[component(storage = "bitset", reflect)]` whose `.stderr` pins the span on `reflect`,
   not on the struct and not on `aether! {`. (Aether cannot produce this combination —
   D15 — but hand-written Rust can, and the `.stderr` is what proves the span.)
6. **D16, verified by a compile, not assumed** — a fixture crate that writes
   `#[component(reflect)]` and declares **no** `reflect` feature. Assert
   (a) it builds (the items vanish), and (b) `cargo clippy -- -D warnings` on it **reds**
   with `unexpected_cfgs` naming `feature = "reflect"`. Record **where the diagnostic
   points**; if it points at the derive call site rather than the user's key, add
   `quote_spanned!` at the emitted key and re-measure.

**RED MUTATION.** (1) Skip the `:256-263` lookahead edit → gate 3 reds (this is the
mutation worth doing *first*, because it is the one a reviewer's eye slides over).
(2) Emit the `reflect` key unconditionally in `expand.rs` → gate 2 reds, and gate 6's
fixture becomes every `aether!`-bearing crate — which is exactly D14's stated flip cost,
now observed rather than argued. (3) Drop the `#[cfg(feature = "reflect")]` from the
derive's emission → the feature-off leg of gate 4 fails to compile with an unresolved
`boyko_reflect` path, which is §2's original `E0432`/`E0433` finding reproduced on demand.

### B7 — `Str`, last — size S *(explicitly ordered last, §6.1(d))*

**Lands.** `SinkEvent::Str` emission (borrow, zero alloc — A.3); `set_str` on the apply
side with A.4's raw `ptr::drop_in_place` + `ptr::write` discipline, **never** an
intermediate `&mut String` (the `Unique`-retag-through-`SharedReadWrite` class that
Tree Borrows caught after three critic rounds approved it).

**Gate.** Miri-TB under `-Zmiri-tree-borrows` on the setter — **and only after
`REFLECTION-PLAN-GATES.md` has added BOTH Miri rows** (its D4/G4):

* `-p boyko-reflect` **PLAIN**, no `--features`;
* `-p reflect-fixture --features reflect-fixture/reflect`.

> ~~`-p boyko-reflect` … with the feature ON (run it with the feature off and it compiles an empty
> crate and reports green)~~ — **struck 2026-08-21 (second pass).** Inherited from
> `REFLECTION-ANALYSIS.md` B.9's closing line and false in both halves: `boyko_reflect` carries **no**
> `reflect` feature (GATES D4 — the `#[cfg]` in derive output is a *consumer-side* construct), so
> `--features reflect` on it is a **hard cargo error**, and with the feature "off" the crate is **not
> empty** — nothing in its source is `cfg`-gated. B.9 is corrected at the source.
>
> **Which row carries THIS rung matters:** `set_str` writes through a pointer into a **component**'s
> row, and a `StrFixture { s: String }` component can only exist in a **consumer** — so the row that
> can red on the `&mut String` retag is the **fixture's**, and a `boyko_reflect`-only run would
> exercise a hand-built pointer wearing the same verdict. B.9 remains the reason the row must exist;
> D4 is the reason it is spelled this way.

Alloc accounting: exactly **1 alloc + 1 free** per `set_str`, measured on the counting
allocator, no leak, no double-free.

**RED MUTATION.** Replace the raw store with `*slot = s.to_owned()` through a `&mut
String` and observe Miri-TB red **on the fixture row**. Land a deliberate Miri failure first to prove
the CI leg actually executes the package (B.9's explicit instruction) — and do it on **both** rows,
because they cover different code and a single proof would certify the row that never sees the
derive.

---

## Deferred, and to what

| Item | Deferred to | Why, in one line |
|---|---|---|
| `Vec<T>` / map / collection kinds in the stream | **v2** | §6's taken scope (B); `SoftBody`'s fourteen `Vec` columns stay a documented, opt-out-able refusal (B.8) |
| Data-carrying enums, `Option<T>` | **v2** | no Reference-guaranteed variant-field layout; `Option` is the smallest data enum and niche optimization means no guaranteed discriminant location (A.1) |
| A text (JSON/RON) sink | **v2** | D5. One `Sink` impl, no format change — that is what D2's one-method trait buys |
| Entity remapping on apply | **v2** | D11; needs a prefab-scoped entity map, which is save-format machinery |
| Making `register_stable_name` total over bitset types | **v2** | D9's rejected option (c) — a shipping-crate change with a `boyko_serialize` blast radius, for refactor-robustness the type-name scan approximates |
| `#[reflect(as_str)]` for inline `[u8; CAP]` + `len` string components | **open, engineering** | B.8's display note; a UX question, recorded in `docs/OPEN-QUESTIONS.md`, must not delay the taxonomy |
| `FieldMut<'a>` borrowed handles | **v2** | §5/W4 — the cached-pointer + reborrow class; needs a full TB analysis against concurrent query borrows |
| Cross-engine-version dump compatibility | **never (v1 scope statement)** | D1 — a dev artifact, not a save file |
| **Multi-entity dumps** (`BeginEntity` repeated) | **v2** | D19(b) — no named v1 consumer; every use case in this plan's Goal is single-entity. The grammar and the sorted prologue stay (B5's byte-identity gate needs them); v1 pins `entity_count == 1`, and relaxing that assertion is the whole of the v2 arm |
| **`ApplyPolicy::Lenient`** | **v2** | D19(b) — `Strict` is the editor's behaviour and the one the gates assert. The parameter stays, so the arm is additive |

---

## Dependencies on the sibling plans

| # | Needed from | What | Blocks |
|---|---|---|---|
| 1 | `REFLECTION-PLAN-CORE.md` | `Scalar`, `ValueKind`, `TypeKind`, `TypeInfo`, `FieldInfo`, `EnumInfo` — the boundary encodes them and adds none | B1 |
| 2 | `REFLECTION-PLAN-CORE.md` | **`ValueKind::Array`** (B.8). Without it `[T; N]` falls to `Opaque`, A.6 makes `Opaque` a hard error, and `GpuTransform3D` is **un-derivable** — the acceptance fixture cannot be built | B2, B5 |
| 3 | `REFLECTION-PLAN-CORE.md` | the opt-in surface. **This plan assumes A.5's `#[component(reflect)]` helper attribute and argues for it on Aether grounds (D13)** — under §2's two-derive `cfg_attr` form, `aether_lang` must emit a crate path and a feature string it cannot check | B6 |
| 4 | `REFLECTION-PLAN-CORE.md` | `#[reflect(skip)]` and the spanned, opt-out-able `Opaque` refusal (A.6 correction). The boundary encodes `Refused(Skipped)`; the syntax is CORE's | B2 |
| 5 | `REFLECTION-PLAN-ECS.md` | three-source enumeration **with each id kind-tagged** (B.3) — `dump_entity` needs `(ComponentId, StorageKind, ResidencyKind)`, not a bare slice | B4 |
| 6 | `REFLECTION-PLAN-ECS.md` | `get_component_raw` / `_mut` reached through the ECS glue, incl. the dense branch (`component_api.rs:176`, `:253`, `:76`) | B1, B3 |
| 7 | `REFLECTION-PLAN-ECS.md` — **it claims it; this is no longer "or here"** | **`EnableTagId::try_from_component_id`** = its **S4′**, landing at **EG2** under the single owner call B.13 #2 (D10's resolution note). Without it B.4's presence view is unimplementable from an external crate, and the by-name substitute writes the wrong bit | B4 |
| 8 | `REFLECTION-PLAN-GATES.md` | **TWO Miri rows**: `-p boyko-reflect` **plain** and `-p reflect-fixture --features reflect-fixture/reflect`, each proven by landing a deliberate red first (B.9, as corrected). ~~`-p boyko-reflect` with the feature ON~~ is a cargo error — the crate has no such feature (GATES D4) | B7 |
| 9 | `REFLECTION-PLAN-GATES.md` | the feature-off CI leg for `aether_tests`, and the `unexpected_cfgs`-under-`-D warnings` promotion this plan's D16 relies on | B6 |
| 10 | `REFLECTION-PLAN-GATES.md` | the ship absence gate. The boundary contributes one cell: **the dump entry points must be absent from a feature-off ship artifact**, with a feature-ON present control beside it (B.6's 2×2 discipline — *"a `shipping` binary with no emission symbol is ambiguous on its own"*) | — |

---

## Open questions this plan opens (mirrored into `docs/OPEN-QUESTIONS.md`)

> **The owner-facing list is `REFLECTION-ANALYSIS.md` B.13, not this table.** Four decisions were
> spread across three plan documents with no single list, and **two of them were the same decision
> described twice** — B-1 below and `REFLECTION-PLAN-ECS.md`'s seam, with opposite recommendations.
> The rows below are retained for provenance and each now points at its B.13 row; **nothing here is
> asked of the owner separately.**

| # | Question | Owner or engineering? | Blocks |
|---|---|---|---|
| ~~B-1~~ | **RETIRED into `REFLECTION-ANALYSIS.md` B.13 #2**, the four-item `boyko_ecs` seam owned by `REFLECTION-PLAN-ECS.md`'s EG2 (as **S4′**). It was filed here as a second, separate widening; it is the *same* decision that plan was already routing to the owner, and that plan had **rejected** the item while this one called it required (D10's resolution note). One call, four items | **OWNER**, via B.13 #2 | B4 |
| B-2 | **Are Aether components reflectable by default or opt-in?** → **B.13 #4.** This plan sets the engineering default to **opt-in** (D14) and states the flip cost precisely: default-on makes a `reflect` feature declaration mandatory in every `aether!`-bearing crate under the existing `-D warnings` gate | **OWNER** (DSL ergonomics) | B6 |
| B-6 | **May engine crates carry a `reflect` feature?** → **B.13 #1.** Not previously on any list, and the largest of the five: it decides whether v1 dogfoods the engine's own components at all. This plan is affected only indirectly — B5's **dogfood half** names `Transform` / `Visibility` / `GpuTransform3D`, so a "no" deletes that half; the instrument half (local fixtures, the golden blob, the id-reorder) already stands on its own (`REFLECTION-ANALYSIS.md` B.12, "Reversibility") | **OWNER** (shipping manifest surface) | B5 |
| B-3 | Should `register_stable_name` become total over bitset types, retiring D9's type-name scan? A `boyko_ecs` change with a `boyko_serialize` blast radius | engineering, v2 | — |
| B-4 | `has_component` has no `Bitset` branch and reports `false` for a tag the entity demonstrably has (`component_api.rs:673-702`). Reflection is how it was found; whether the kernel fn grows the third branch is a `boyko_ecs` question | engineering | — |
| B-5 | D16's diagnostic **span** — verified by rung B6 gate 6, not predicted here. If `unexpected_cfgs` points at the derive call site rather than the user's `reflect` key, the expander needs `quote_spanned!` on the emitted key | engineering, settled by a compile | B6 |

**Recorded as settled, not open:** §0/§2's central finding (optional crate + Cargo feature
+ CI absence gate, *not* `cfg(debug_assertions)`) — inherited unchanged. §6's scope fork —
**TAKEN: (B)**, with reason, reversible by construction, and built in the §6.1(d) order
(**arrays → nested → enums → `String` last**).
