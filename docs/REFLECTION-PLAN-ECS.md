# `boyko_reflect` — the ECS glue plan (Wave 3)

> **Status:** PLAN — a rung ladder a developer executes. Not a discussion.
> **Design input:** [`docs/REFLECTION-ANALYSIS.md`](REFLECTION-ANALYSIS.md) (revision 2026-08-21).
> **Siblings, cross-referenced by filename and never duplicated:**
> [`docs/REFLECTION-PLAN-CORE.md`](REFLECTION-PLAN-CORE.md) (the value model, the registry table,
> the derive) · [`docs/REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md) (`Sink`/`Source`,
> the name-keyed save format) · [`docs/REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md) (the
> feature matrix, the ship-absence census, the Miri allowlist, the perf legs).
> **Worktree:** `D:/wt/reflect`, branch `feat/reflection`. `graphify` CLI is not installed on this
> machine; orientation was Grep/Read, and every fact below carries a `file:line` anchor.

---

## 0. What this plan owns, and what it does not

**Owns.** Everything that stands between a `ComponentId` and a byte: enumeration of an entity's
components across all three storage backends; `get_field` / `set_field` / by-name access;
`add_default` / `remove`; the presence view for bitset tags; the refusal taxonomy; the reuse of
`boyko_ecs`'s raw-by-id access; the **public by-id structural seam** the glue needs and
`boyko_ecs` does not have; every `unsafe` the glue writes and the `// SAFETY:` comment it carries;
the Miri-TB and proptest obligation per path.

**Does not own.** `TypeInfo` / `FieldInfo` / `Scalar` / `ValueKind` / `TypeKind` / `NestedCursor`
and the `#[derive(Reflect)]` that bakes them (CORE). The `REFLECT` registry table and
`type_info_of` (CORE). `Sink`/`Source` and the save key (BOUNDARY). The Cargo feature, the CI
matrix, the ship-absence census, the Miri package allowlist, and the perf legs (GATES).

**Two carried-forward decisions this plan does not re-open.**

* The §0/§2 central finding — *"reflection only in a Debug build" is not a compiler property; the
  mechanism is an optional crate behind a Cargo feature plus a CI symbol-absence gate.* Unchanged,
  not re-litigated. Its consequence here is only that every rung below is gated in the
  **feature-ON** leg, and that the ECS glue never appears in a ship build's dependency closure.
* The §6 scope fork — **TAKEN: option (B)**, POD + `String` + nested + `#[repr(Int)]` fieldless
  enum, collections deferred to v2, recorded with its reason and reversible by construction. The
  glue is **scope-agnostic**: it moves bytes at an offset and calls a fn-pointer, so narrowing
  (B) back toward (A) deletes accessors in CORE and changes nothing in this plan.

---

## 1. Verified in-tree facts

Every row was read in this worktree on 2026-08-21. The anchors are load-bearing: five of them
**contradict** the analysis, and each contradiction is what makes a rung exist.

| # | Fact | Anchor |
|---|---|---|
| F1 | `get_component_raw(e, id) -> Option<*const u8>` routes **Table** through a raw `columns` projection and **Dense** through `dense_get_raw`; returns `None` for stale generation, dead slot, null column. | `ecs_master/component_api.rs:176-247` |
| F2 | `get_component_raw_mut` is the write-capable twin, same three-way prologue, dense arm re-resolves `slot_of` + `row_ptr`. | `component_api.rs:253-300` |
| F3 | `dense_contains` / `dense_slot_of` / `dense_get_raw` are **`pub` on `EcsMaster`**. | `component_api.rs:49`, `:58`, `:76` |
| F4 | `DenseRegistry::dense_ids() -> &[ComponentId]` is **`pub`**, registration-ordered; `EcsMaster::dense_registry()` is **`pub`**. | `dense/dense_registry.rs:156`, `ecs_master.rs:590` |
| F5 | `is_signature_storage(kind)` is `matches!(kind, Table)` — **both `Bitset` and `Dense` are excluded from every archetype signature**. | `component_registry/mod.rs:354-356` |
| F6 | `EcsMaster::has_component` branches `Dense → dense_contains`, else column-null. **There is no `Bitset` branch**, so it returns `false` for every enabled bitset tag. | `component_api.rs:673-702` |
| F7 | `entity_archetype_id(e) -> Option<ArchetypeId>` is `pub` and generation-checked; `archetype_master()` is `pub`; `ArchetypeMaster::get_archetype(id) -> Option<&Archetype>` and `get_archetype_ptr(id) -> Option<*const Archetype>` are both `pub`. | `entity_query_api.rs:35`, `ecs_master.rs:575`, `archetype_master.rs:242`, `:271` |
| F8 | `Archetype::component_ids()` is `pub`, but the **field** `component_ids` (like `columns`, `id`, `signature`) is `pub(crate)` — an external crate **cannot** write `addr_of!((*p).component_ids)`. | `archetype/archetype.rs:1411`, `:127-160` |
| F9 | ⚠️ **`migrate_entity_attach_ids` `debug_assert!`s that every added id is a ZST.** *"D9: this path skips byte-writes for `added` — sound ONLY for size-0 columns. A data component routed through here would leave its bytes uninitialized."* | `commands/migration_helpers.rs:1387-1394~` |
| F10 | `migrate_entity_detach_ids` is **data-general** — it collects retained bytes, fires `on_replace`/`on_remove` on the dying row, and runs `drop_fn` exactly once per removed id. No ZST assertion. | `migration_helpers.rs:1635-1652~` |
| F11 | The only **data** insert helper is `migrate_entity_insert<B: Bundle>(…, bundle: B)` — **generic, taking the bundle by value**. There is no by-id data attach anywhere in the tree. | `migration_helpers.rs:332-338` |
| F12 | All five migration helpers are **`pub(crate)`**. | `migration_helpers.rs:1230, :1305, :1372, :1658, :1922` |
| F13 | `EcsMaster::add_tag` / `remove_tag` are `pub` and drive the by-id migration end to end for ZSTs: inland resolve → presence test → `retag_in_place` or `merged_archetype_id_dyn` + `migrate_entity_{attach,detach}_ids` → `DeferredScopeGuard` → `drain_deferred_hook_queue`. **This is the template.** | `ecs_master/tag_api.rs:130-183`, `:200-238` |
| F14 | `set_component_raw`'s **dense** arm bumps the slot's `changed` tick; its **table** arm does not (it memcpys through `get_component_raw_mut`). The asymmetry is documented at the fn but not resolved. | `component_api.rs:444-497` |
| F15 | The only change-detecting write path is `get_component_mut<T>() -> Mut<'_, T>` — **generic over `T`**. `get_component_changed_tick(e, id)` is a `pub` **read**; there is no `pub` by-id **write**. | `component_api.rs:553`, `:337-379` |
| F16 | `EnableTagId` and `TagId` are `#[repr(transparent)]` over `ComponentId` with `pub(crate)` fields and, in each case, an explicit doc line: *"The reverse direction has NO constructor."* `is_enabled_id` / `enable_id` / `disable_id` take `EnableTagId`. | `component_registry/tags.rs:49, :93`, `enable_tag_api.rs:113-129` |
| F17 | There is **no public high-water mark of minted `ComponentId`s**. `next_id_for_test()` is `pub(crate)` and test-only; `NEXT_ID` is a private `AtomicUsize`. | `component_registry/mod.rs:212, :1090` |
| F18 | `register_enable_tag(name)` / `register_tag(name)` are `pub`, `&mut self`, and **idempotent by name** — a second call for a live name returns the existing id and mints nothing. ⚠️ **Idempotent *within `TAG_NAMES`* — see F27, which is the half that makes this fact dangerous rather than useful for derived bitset components.** | `enable_tag_api.rs:60`, `tag_api.rs:65` |
| F19 | `storage_kind(usize) -> StorageKind`, `residency_class(usize) -> ResidencyKind`, `get_layout(usize) -> Option<&'static ComponentLayout>` and `MAX_COMPONENTS = 512` are all `pub`. | `component_registry/mod.rs:388`, `:577`, `:1123`, `:61` |
| F20 | `ComponentLayout { size, alignment, drop_fn, type_name, type_id }` is `pub`, pinned at 56 B. For a **dynamic tag** `type_name` is the interned user name and `type_id` is `DynamicTagMarker`'s — and `DynamicTagMarker` is **private and unnameable outside `boyko_ecs`**. | `mod.rs:107-120`, `:171-179`, `:192-194~` |
| F21 | `make_component_device_backed` sets `columns[cid] = Column::null()` and `assert!`s (release) that the id is `ResidencyKind::Gpu`. It is `#[cfg(not(miri))]` and needs a `DeviceColumnHandle`. | `archetype/archetype.rs:694-730~` |
| F22 | **`grep -rn 'residency = "gpu"' crates/*/src/` returns zero hits** — there is no GPU-resident component in the tree. `classify_component_residency` is the `pub` runtime classifier. | (measured) `component_registry/mod.rs:685-688~` |
| F23 | The fixture types the acceptance test needs all exist: `Transform` (`boyko_scene/src/transform.rs:46`), `Visibility` `#[repr(u8)]` (`render_caps.rs:226`), `GpuTransform3D` `#[component(storage="dense")]` (`boyko_render/src/gpu_transform3d.rs:84~`), `TrsPacked` (`:55`), `ParticleEmitter` (`boyko_render/src/particle.rs:127`), `ParticleEffectHandle` with `on_insert`/`on_replace` (`:185`), `EmitterActive` `#[component(storage="bitset")]` (`:164`). | (as cited) |
| F24 | ⚠️ **`force_alloc_panic` has ZERO `#[cfg(force_alloc_panic)]` sites in any `.rs` file in the tree.** It survives only in the root `Cargo.toml`'s `check-cfg` list and an **archived** doc, while CI still runs a whole job under it. That job asserts nothing. | [`Cargo.toml`](../Cargo.toml):26~, `docs/archive/PHASE-9-FORCE-ALLOC-PANIC.md`, `.github/workflows/ci.yml:178-191` |
| F25 | `proptest` is a workspace dependency ([`Cargo.toml`](../Cargo.toml):52~) already used by six crates. | `crates/boyko_ecs/Cargo.toml:93~` and five siblings |
| F26 | Package names use hyphens: `boyko-ecs`, `boyko-scene`. The new crate is therefore `boyko-reflect` in `crates/boyko_reflect/`. | `crates/boyko_ecs/Cargo.toml:2~` |
| F27 | 🔴 **`register_enable_tag(name)` MINTS A NEW ID for any name not already in `TAG_NAMES`, and a derived `#[component(storage = "bitset")]` type never interns its name there.** Traced end to end: `EcsMaster::register_enable_tag` → `try_register_enable_tag_by_name` → `try_register_tag_by_name`, whose table is `TAG_NAMES` and whose miss path calls `try_register_dynamic(ComponentLayout::new_dynamic_tag(leaked))`. A derived bitset component's id came from `register_new::<Self>()` (`component_registry/mod.rs:918`, a monotonic `NEXT_ID.fetch_add`) and its name was never interned. | `enable_tag_api.rs:60`, `component_registry/tags.rs:134`, `:155`, `:182-196`, `component_registry/mod.rs:918` |
| F28 | `NEXT_ID` is a monotonic `AtomicUsize` with `fetch_add`; **ids are never recycled**, and `component_id()` is a per-type `static ID: OnceLock<ComponentId>` resolved once per process. So *"the id re-registered to a different type"* is not a state this process can reach. | `component_registry/mod.rs:212, :918`; `boyko_macros/src/component.rs:434-436` |
| F29 | `try_register_tag_by_name` returns `None` when `NEXT_ID >= MAX_COMPONENTS` and `name` was never minted; `register_enable_tag` turns that `None` into `register_enable_tag_exhausted_panic`. Dynamic tags and typed components share the one 512-id budget. | `component_registry/tags.rs:189-192~`, `enable_tag_api.rs:59-65~` |

### The five contradictions, named

1. **F9 + F11 kill "`add_default` routes through the existing structural insert."** The five
   helpers §4 points at are the *tag* path; the one that attaches is ZST-only **by assertion**, and
   the one that attaches data is generic over a compile-time `Bundle`. A by-id data attach **does
   not exist**. → rungs EG2, EG6.
2. **F8 changes the enumeration route.** `Archetype`'s fields are `pub(crate)`, so B.7's raw
   projection form is *not expressible* from `boyko_reflect`. The glue must use the safe `&self`
   accessor — which puts it outside BUG-MIGRATE-TB-1's hazard instead of obliged to obey it. → D2.
3. **F16 + F17 kill "walk the registered enable tags."** There is no enumerator and no reverse
   bridge. → D3, D4, rung EG3.
4. **F14 + F15 add a failure the analysis never names**: a table-path `set_field` is invisible to
   every `Changed<T>`-gated system, while the same edit on a dense component is visible. An
   inspector edit to `Transform` would therefore silently not reach the GPU. → D10, D11, rung EG5.
5. **F24 removes the allocation instrument** the zero-alloc audit would naturally reach for. The
   glue brings its own counting allocator. → §7, rung EG1.
6. 🔴 **F27 kills "the bitset presence WRITE needs zero new API."** §4's substitute for the rejected
   `EnableTagId` constructor was `display_name(id) → register_enable_tag(name) → enable_id/disable_id`,
   *"all through existing public API. Zero additions."* It routes to a **different bit**:
   `set_presence(EmitterActive_id, false)` would mint a brand-new dynamic tag under the string
   `boyko_render::particle::EmitterActive`, clear **that** bit, and leave `EmitterActive`'s own bit
   set — while returning `Ok(())`. → **D4 is reversed**; rungs EG0, EG2, EG3.
   *The dynamic-tag half of the same trick survives untouched, and the asymmetry is the point:* a
   runtime-minted tag's id **came from** `try_register_tag_by_name`, so its name **is** in `TAG_NAMES`
   and `ComponentLayout::type_name` **is** that interned name (F20). `register_tag(name)` returns the
   same id, and `tag_by_name` (`tag_api.rs:76`) is public besides. **The runtime-minted citizen is
   better served by the public API than the compile-time one.**
7. **F28 kills EG4 gate 5 as specified** — *"the id re-registered to a different fixture type between
   the read and the enumeration"* is not constructible in one process. → EG4, which gets a
   constructible substitute rather than losing the gate.

---

## 2. The v1 public surface of the glue

```rust
//! boyko_reflect::ecs — every fn takes `&EcsMaster` or `&mut EcsMaster`.
//! Nothing here allocates. Nothing here is reachable from a ship build.

/// Why an access could not be served. `#[repr(u8)]`, `Copy`, one byte — never a
/// bare `Option`, because `None` conflates states the inspector must show apart.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    EntityDead,       // stale generation / never registered / despawned
    ComponentAbsent,  // the entity does not carry this id
    NotReflectable,   // no TypeInfo: opted out of #[component(reflect)], or a dynamic tag
    NoHostBytes,      // ResidencyKind::Gpu — enumerated, deliberately unreadable
    PresenceOnly,     // StorageKind::Bitset — no per-row bytes; use presence_of
    FieldOutOfRange,
    NoSuchField,      // by-name miss
    KindMismatch,     // field kind vs. Scalar tag (the load-bearing RELEASE check)
    BufferTooSmall,   // components_of_into's out slice could not hold the answer
    NoDefault,        // TypeInfo.default_in_place is None — the type has no `Default`, or it
                      // carries #[reflect(no_default)] (CORE D20). Inspectable, not synthesizable.
    Unsupported,      // v1 scope edge, named per site (see §8 Deferred)
}

/// How the id is stored, i.e. which of the five inspector row states applies.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdKind {
    Table,        // signature, host bytes, TypeInfo present  -> fields
    TableOpaque,  // signature, host bytes, no TypeInfo       -> name only
    TableGpu,     // signature, ResidencyKind::Gpu            -> "GPU-resident"
    Dense,        // no signature, host bytes, TypeInfo       -> fields
    Bitset,       // no signature, no bytes                   -> boolean toggle
}

#[derive(Clone, Copy, Debug)]
pub struct IdEntry { pub id: ComponentId, pub kind: IdKind }

// ── enumeration (rungs EG1, EG3) ────────────────────────────────────────────
pub fn components_of_into(ecs: &EcsMaster, e: Entity, out: &mut [IdEntry])
    -> Result<usize, Refusal>;
pub fn display_name(id: ComponentId) -> &'static str;   // ComponentLayout::type_name

// ── read (rung EG4) ─────────────────────────────────────────────────────────
pub fn fields_of(id: ComponentId) -> Result<&'static [FieldInfo], Refusal>;
pub fn get_field(ecs: &EcsMaster, e: Entity, id: ComponentId, f: usize)
    -> Result<Scalar, Refusal>;
pub fn field_value<'a>(ecs: &'a EcsMaster, e: Entity, id: ComponentId, f: usize)
    -> Result<FieldValue<'a>, Refusal>;
pub fn get_field_by_name<'a>(ecs: &'a EcsMaster, e: Entity, id: ComponentId, name: &str)
    -> Result<FieldValue<'a>, Refusal>;

// ── write (rung EG5) ────────────────────────────────────────────────────────
pub fn set_field(ecs: &mut EcsMaster, e: Entity, id: ComponentId, f: usize, v: Scalar)
    -> Result<(), Refusal>;
pub fn set_field_by_name(ecs: &mut EcsMaster, e: Entity, id: ComponentId, name: &str, v: Scalar)
    -> Result<(), Refusal>;

// ── presence, the bitset substitute for fields (rung EG3) ───────────────────
pub fn presence_of(ecs: &EcsMaster, e: Entity, id: ComponentId) -> Result<bool, Refusal>;
pub fn set_presence(ecs: &mut EcsMaster, e: Entity, id: ComponentId, on: bool)
    -> Result<(), Refusal>;

// ── structural (rung EG6) ───────────────────────────────────────────────────
pub fn add_default(ecs: &mut EcsMaster, e: Entity, id: ComponentId) -> Result<(), Refusal>;
pub fn remove(ecs: &mut EcsMaster, e: Entity, id: ComponentId) -> Result<(), Refusal>;
```

`FieldInfo`, `Scalar`, `FieldValue` and `type_info_of` come from CORE
([`docs/REFLECTION-PLAN-CORE.md`](REFLECTION-PLAN-CORE.md)); this plan only consumes them.

---

## 3. The storage-kind matrix, and where each refusal is MECHANICAL

The task the refusal answers is *"reading a field at an offset is meaningless for a bitset tag."*
That is true, and it is **two different questions asked at two different boundaries**. Neither
mechanism can serve the other's boundary, so v1 builds both — see **D5**.

| citizen | in signature | host bytes | `type_info_of` | `IdKind` | field read | field write | presence | inspector row |
|---|---|---|---|---|---|---|---|---|
| `Table` + `Cpu` + `#[component(reflect)]` | yes | yes | `Some` | `Table` | ✅ | ✅ | derived (`Ok(true)`) | fields |
| `Table` + `CpuPinned` + `#[component(reflect)]` | yes | yes | `Some` | `Table` | ✅ | ✅ | derived | fields (host memory — F21's assert forbids ever flipping it to device) |
| `Table` + `Cpu`, no `#[component(reflect)]` | yes | yes | `None` | `TableOpaque` | `NotReflectable` | `NotReflectable` | `Ok(true)` | name only |
| **dynamic tag** (`register_tag`) | yes | none (ZST) | `None` **always** | `TableOpaque` | `NotReflectable` | `NotReflectable` | `Ok(true)` | interned **name** (F20) |
| `Table` + `Gpu` | yes | **no** | `Some` | `TableGpu` | `NoHostBytes` | `NoHostBytes` | `Ok(true)` | "GPU-resident — no host bytes" |
| `Dense` | **no** (source 2) | yes | `Some` | `Dense` | ✅ | ✅ | `Ok(true)` | fields |
| `Bitset` | **no** (source 3) | **no** | derive-refused ⇒ `None` | `Bitset` | `PresenceOnly` | `PresenceOnly` | ✅ get **and** set | boolean toggle |

**`TableOpaque` deliberately merges two citizens.** The analysis lists "opted out" and "dynamic
tag" as separate rows, then observes that three of its rows are *"the same UI problem in different
clothes"*. They are. And distinguishing them is not merely unnecessary — it is **not possible**:
the only discriminator is `type_id == TypeId::of::<DynamicTagMarker>()`, and `DynamicTagMarker` is
private and unnameable outside `boyko_ecs` (F20). Both display `ComponentLayout::type_name`, which
is meaningful for both (the interned name, or the Rust path). Splitting them is **deferred** with a
named blocker: a `pub fn is_dynamic_tag(id) -> bool` in `boyko_ecs` — a third shipping-API addition
this plan refuses to make for a cosmetic gain.

**The `TypeId` cross-check is skipped whenever `type_info_of(id)` is `None`.** It must never be
pointed at a dynamic tag: every dynamic tag shares one `TypeId`, so the check would **pass for the
wrong tag**, which is worse than not checking (§5, B.4). Encoded structurally — the check lives
inside the arm that already holds a `&'static TypeInfo`, so there is no code path that can reach it
without one.

---

## 4. The shipping-crate seam

**This is the one place a dev-only feature widens a shipping crate's public API.** It is stated
here, at plan time, rather than discovered in the middle of rung EG6. **Four items in ONE owner call**
(analysis **B.13 #2**): three structural, and one that completes a bridge `boyko_ecs` already
advertises and already tests internally. Three of the four are `boyko_ecs`-internal-shaped; the
fourth was reached independently by [`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md)'s
D10 and filed as a *separate* owner question until F27 showed the two were the same decision.

*(F6's `has_component` bug is **not** on this list. The former S4 — `is_enabled_raw` — was justified
partly as "the correct fix shape for F6's bug class"; with S4′ in hand the glue no longer needs a new
`EcsMaster` method at all, so F6 goes back to being purely an incidental kernel finding, reported and
not fixed here. §9 records the disposition.)*

| # | New public item in `boyko_ecs` | Why the glue cannot proceed without it | Justified on its own merits? |
|---|---|---|---|
| S1 | `EcsMaster::add_component_by_id(&mut self, e: Entity, id: ComponentId, bytes: &[u8]) -> AddOutcome` | F9 + F11: the ZST attach helper asserts size-0; the data attach helper is generic over `Bundle`. Nothing in the tree attaches a data component by id. | **Yes** — scene loading, prefab instantiation, undo/redo and network apply all want a by-id data attach; today each would have to be generic over the component set. `add_tag` proves the signature can be pure `Entity` + `ComponentId` + bytes and name no reflection type. |
| S2 | `EcsMaster::remove_component_by_id(&mut self, e: Entity, id: ComponentId) -> bool` | F10 + F12: the data-general detach helper exists and is correct; it is `pub(crate)`. | **Yes** — the exact twin of the already-public `remove_tag`, generalised off the ZST restriction it never actually needed. |
| S3 | `EcsMaster::mark_component_changed(&mut self, e: Entity, id: ComponentId) -> bool` | F14 + F15: no by-id change-tick write exists, so a table-path `set_field` is invisible to `Changed<T>`. | **Yes** — the write twin of the already-public `get_component_changed_tick`; any by-id writer (scene apply, replication) has the same hole. |
| ~~S4~~ **S4′** | `EnableTagId::try_from_component_id(id: ComponentId) -> Option<Self>` — `None` unless `storage_kind(id) == Bitset` (lands in `component_registry::tags`, not on `EcsMaster`) | F16 + **F27**: `is_enabled_id` / `enable_id` / `disable_id` all need an `EnableTagId`, which has no reverse constructor, and the by-name re-mint route **writes the wrong bit**. Both the read half and the write half need this. | **Yes** — it is the inbound half of a bridge the crate already advertises outbound (`EnableTagId::component_id()`, *"bridges to the shared `ComponentId` space"*) and already tests internally (`enable_tag_id_bridges_to_component_id_round_trip`, `component_registry/mod.rs:1676`). It is total, safe, `storage_kind`-checked, and mints **no capability the crate does not already have** — every enable/disable it unlocks is reachable from inside `boyko_ecs` today. |

**S1's return type is an enum, not a `bool`.** `enum AddOutcome { Added, AlreadyPresent, Rejected(RejectReason) }` where `RejectReason` covers `{ EntityDead, WrongByteLen, NotTableOrDense, GpuResident }`. An editor's "Add Component" must **refuse** rather than clobber a present value; `add_tag`'s in-place-replace semantics are right for a tag (zero bytes to lose) and wrong for data.

**S1's implementation is a bytes-carrying sibling of `migrate_entity_attach_ids`, not a widening of it** — see **D9**. `add_component_by_id` follows `add_tag`'s choreography verbatim (F13): inland resolve → silent no-op on stale → `DeferredScopeGuard::enter()` → presence test on the source signature → `merged_archetype_id_dyn` → `migrate_entity_attach_ids_with_bytes` → `drop(scope)` → `drain_deferred_hook_queue()`. Dense ids route instead to the already-existing `dense_insert_and_fire` (`component_api.rs:94`), which already fires `on_add` + `on_insert` and already takes `&[u8]`.

**What the seam does NOT include, and why.**

* **A dynamic-tag presence *write*.** `TagId` has no reverse constructor either — but `register_tag(name)` is `pub` and **idempotent by name** (F18), and the name is available from `ComponentLayout::type_name` (F20). So `set_presence` on a dynamic tag routes `display_name(id) → register_tag(name) → add_tag/remove_tag`, all through existing public API. **Zero additions — and this bullet is SOUND**, for the reason F27's second half states: a dynamic tag's id *came from* `try_register_tag_by_name`, so its name **is** the key in `TAG_NAMES` and the re-mint is a genuine lookup. `tag_by_name` (`tag_api.rs:76`) is public besides, so even the mint can be avoided. *(Re-checked separately rather than inherited from the bullet below, because the two bullets looked identical and only one of them was true.)*
* **~~A bitset presence *write*. Same trick~~ — FALSE, struck 2026-08-21 (second pass).** `register_enable_tag(name)` is idempotent **within `TAG_NAMES`**, and a *derived* `#[component(storage = "bitset")]` type never interns its name there (**F27**). On a derived bitset id the call **mints a new dynamic tag**, and `set_presence(EmitterActive_id, false)` would clear the new tag's bit while `EmitterActive`'s own bit stayed set — returning `Ok(())`. *(This plan's own EG3 gate 4 would have caught it, which is the proof that EG3 was unimplementable as specified, not that the design was safe.)* **Both halves — the `&self` read and the `&mut self` write — now go through S4′.**
* **~~`EnableTagId::from_component_id`. Rejected~~** → **adopted as S4′; D4 is reversed.**
* **A hook-firing whole-value `replace_component_by_id`.** Deferred to v2 — see **D10**.
* **`registered_component_count()`.** Rejected — see **D3**.
* **A `Bitset` branch on `has_component` (F6).** Reported as a `boyko_ecs` finding, **not fixed under this campaign** — it changes the behaviour of a shipping API with existing callers for reasons that belong to reflection. Filed in `docs/OPEN-QUESTIONS.md`. The glue simply never calls it for a non-`Table` id, and rung EG3's RED mutation *is* that call.

---

## 5. Decisions

Numbered so a later reader cannot re-litigate them from scratch. Each carries the reason and the
rejected alternatives.

**D1 — enumeration fills a caller buffer and tags each id with its kind; it does not return a slice.**
`components_of_into(ecs, e, &mut [IdEntry]) -> Result<usize, Refusal>`.
*Reason:* three sources (F5) means there is no single contiguous slice to borrow, and the caller
must know which of §3's five row states applies before it can render a row.
*Rejected:* `Option<&[ComponentId]>` (§4 as written — structurally blind to `Dense` and `Bitset`,
so it would refuse to show `GpuTransform3D`, the one component the read path fully handles);
returning a `Vec` (one allocation per entity per inspector frame, principle 5); an iterator
borrowing `&EcsMaster` (sound, but it pins the shared borrow across the caller's own reads and
cannot be reused frame to frame — the buffer can).

**D2 — the table source uses the safe `&self` accessor, never `EntityInland::archetype_ptr()`.**
`entity_archetype_id(e)` → `archetype_master().get_archetype(id)` → `.component_ids()` (F7).
*Reason:* two, and the second is the interesting one. (a) F8: `Archetype`'s fields are
`pub(crate)`, so B.7's prescribed raw-projection form is **not expressible** from an external
crate — the prescription cannot be followed as written. (b) By never touching the cached slab
pointer, the glue is **outside** BUG-MIGRATE-TB-1's hazard rather than obliged to obey it: the rule
constrains references derived from the interior-mutable, `SharedReadWrite`-rooted `EntityInland`
pointer, and `get_archetype` derives from the bundle under `&self`. This is a **simplification** of
the analysis's Wave-3 obligation, not a violation of it — and it is Miri's to confirm, not the
argument's (rung EG1's gate).
*Rejected:* a new `pub fn component_ids_of(&self, e) -> &[ComponentId]` projection on `EcsMaster`
(a fifth shipping-API addition to buy nothing); re-implementing `get_component_raw`'s prologue
inside `boyko_reflect` (duplicates audited `unsafe` and goes stale the day a fourth storage kind
lands — the `StorageKind` discriminant space is explicitly extensible, `component_registry/mod.rs:320~`).

**D3 — the bitset source scans `0..MAX_COMPONENTS` on `storage_kind`, not a registry-held list.**
*Reason:* F17 — there is no public high-water mark, and `next_id_for_test` is `pub(crate)`
test-only. 512 `Relaxed` loads on a cold editor path is the cheapest thing that adds **zero**
shipping API. The scan is also self-correcting: a storage kind added tomorrow appears without
touching the glue.
*Measured, not assumed:* rung EG3 reports the wall-clock of one `components_of_into` on the
acceptance fixture, with the 512-scan isolated as its own leg.
*Rejected:* `pub fn registered_component_count() -> usize` (a fifth shipping-API addition to save a
cold sub-microsecond scan); caching the bitset id set in `boyko_reflect` (a second source of truth
about the registry — exactly B.2's drift class, in a layer that is supposed to have none).

**D4 — ~~the `&self` bitset probe is a typed answer on `EcsMaster`, not a reverse constructor on `EnableTagId`~~ → REVERSED 2026-08-21 (second pass): the seam IS the checked constructor, `EnableTagId::try_from_component_id(id) -> Option<Self>` (S4′), and `is_enabled_raw` is dropped.**

*The original decision, kept verbatim so the reversal argues with a reason rather than replacing
one:* `EnableTagId`'s documented invariant is *"a proof that the id was minted as a bitset enable
tag"* (F16). A checked `from_component_id` reading `STORAGE_KIND` would re-establish exactly that
proof and would be sound — but it mints a **transferable capability token** at runtime for a caller
that only ever wanted to ask one question. `is_enabled_raw` answers the question, returns the refusal
in its own `None` arm, and creates no token. ~~The write half needs nothing at all (F18).~~

**The load-bearing sentence is the struck one, and it is false.** F27: `register_enable_tag(name)` is
idempotent within `TAG_NAMES`, which **derived** bitset components never intern; on a derived id it
mints a new dynamic tag and toggles that tag's bit instead. So `is_enabled_raw` cannot be the whole
seam — it serves the `&self` read and leaves the write with no correct route at all. Once a write
route is needed, the choice is between the constructor and a **second** `EcsMaster` method
(`set_enabled_raw(e, id, on) -> Option<()>`), i.e. two ad-hoc methods versus one general one.

**Three things then decide it, and the token objection loses on its own terms:**

1. **The token grants nothing new.** It is `storage_kind`-checked and total; every enable/disable it
   unlocks is already reachable from inside `boyko_ecs`. The objection was about *inviting* by-id
   callers to mint rather than ask — but F27 shows the "ask" form does not exist for the write, so the
   invitation is to the only correct route.
2. **[`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md)'s D10 independently requires the
   same constructor**, calls it *"a required `boyko_ecs` seam"*, and files it as its own owner
   question B-1 — while this plan's EG0 landed a `trybuild` `compile_fail` fixture asserting the item
   **must not exist**. Two plans specified mutually exclusive seams for one view, one of them gated
   mechanically against the other, and the owner was about to be handed the same decision twice with
   opposite recommendations. **They are one decision** — analysis **B.13 #2**, which merges the
   ECS seam and BOUNDARY's B-1 into a single four-item owner call.
3. **It completes a bridge the crate already advertises and already tests.** `component_id()` is
   documented as *"bridges to the shared `ComponentId` space"*, and
   `enable_tag_id_bridges_to_component_id_round_trip` (`component_registry/mod.rs:1676`) asserts a round-trip the public
   API cannot perform. The inbound half is a completion, not a new capability.

*Now rejected:* `EcsMaster::is_enabled_raw` (it is `EnableTagId::try_from_component_id(id).map(|t|
ecs.is_enabled_id(e, t))` — a convenience over the seam, and by the plan's own
*"a datum lands in the same commit as its first reader"* discipline a convenience with a one-line
body and no second caller is not shipping API); a paired `set_enabled_raw` (two methods where one
constructor serves both halves **and** BOUNDARY); making `EnableTagId`'s tuple field `pub` (destroys
the invariant outright).

*Kept from the original:* if a `&self` typed answer is still wanted **after** the constructor exists,
it is a `boyko_reflect`-local free function over the seam — not a fifth item on a shipping crate.

**D5 — refusal is TWO mechanisms at TWO boundaries; neither substitutes for the other.**

* **Type-level → must not compile.** `#[derive(Reflect)]` on a `#[component(storage = "bitset")]`
  type is a hard error with a span on the user's own type name (CORE owns the derive; the span
  requirement is B.5's, because an Aether user wrote `tag Foo(bitset);` and must not be shown an
  error about a derive they never typed). Reinforced by a **release `assert!` inside
  `install_type_info`**: `storage_kind(id) != Bitset`. Two mechanisms for one rule because the
  compile-time half cannot see the Aether-expanded path or a future runtime reclassification, and
  the runtime half cannot give a good span.
* **Id-level → typed refusal value.** `get_field(ecs, e, id, f)` receives a **runtime**
  `ComponentId` that may name anything — a bitset tag, a dynamic tag, a GPU component, an id from a
  stale inspector row after a hot-reload. The call site cannot know statically. A compile error is
  not available, and `panic!` is wrong for an editor. `Refusal` is the answer.

*Why not one or the other:* the derive-time error asks *"is this **type**'s storage bitset?"*,
which is statically known at the derive site and unknown at the call site. `Refusal` asks *"is this
**id** bitset?"*, which is unknown at the derive site and known at the call site. They are different
questions about different things, and each boundary is blind to the other's answer.

**D6 — reads and writes return `Result<T, Refusal>`, never `Option<T>`.**
*Reason:* `None` conflates ten states (§2's enum) that an inspector must display apart. B.4 raises
this same *"the id is real, the value is not viewable here"* problem three separate times and A.5's
fallback raises it a fourth; one typed enum answers all four at once. Cost: one byte, zero
allocations, and `Result` is already `#[must_use]`.
*Rejected:* `Option<Scalar>` per §4 (an inspector cannot tell "component absent" from
"GPU-resident" from "you forgot to derive `Reflect`" — and reporting the wrong one is a
**correctness** failure of a tool whose entire job is to be trusted about what is there, per A.5);
panicking on refusal (an editor must not die on a stale row).

**D7 — the GPU refusal is a CLASSIFICATION test, not a null-column test.**
`residency_class(id) == ResidencyKind::Gpu` ⇒ `NoHostBytes`, checked **before** any pointer is
taken, regardless of whether the column happens to be non-null.
*Reason:* three. (a) A null column also means *absent*, so a null test conflates two rows of §3's
matrix and produces exactly the "lists a component and then shows nothing" confusion B.4 names.
(b) A `Gpu`-classed component's host bytes are stale by design **before** `make_component_device_backed`
ever runs, so showing them is worse than refusing. (c) The classification test is decidable with
**no `DeviceColumnHandle`, no Vulkan and no GPU** — so the gate runs in CI and under Miri, whereas
a null-column test could only be reddened on a machine with a device. F21 makes (c) decisive:
`make_component_device_backed` is `#[cfg(not(miri))]`.
*Rejected:* null-column detection (conflates; untestable under Miri); trusting the analysis's
"`get_component_raw` returns `None`" (true, but it returns the *wrong refusal*).

**D8 — `add_default` evaluates the default into an aligned stack scratch BEFORE any structural mutation, then hands `&[u8]` to the attach path and FORGETS the scratch.**
*Reason:* `default_in_place` may unwind — CORE bakes it as `ptr::write(p.cast::<T>(), T::default())`
and a hand-written `Default` impl can panic. In `migrate_entity_attach_ids`, Step 2 calls
`dst_pool.commit_units(row, 1)` **before** Step 3 advances `entity_ids` / `current_index`
(`migration_helpers.rs:1518-1544~`). An unwind between the commit and the write therefore leaves a
**committed, uninitialised row** that the pool's drop walk will later run `drop_fn` over. Evaluating
first moves the entire unwind window in front of every structural change: if `T::default()` panics,
the scratch is still uninitialised (`MaybeUninit` drops nothing) and the world is byte-for-byte
untouched.
The scratch is then **moved**, not dropped — the bytes were `copy_nonoverlapping`'d into the row,
which now owns the value; dropping the scratch would be the second drop of one value (M7/W3). With
`MaybeUninit<[u8; N]>` the forget is structural rather than a call: there is no `Drop` to suppress.
*Rejected:* (a) calling `default_in_place` on the committed target row (the unwind hole above);
(b) having the derive emit a field-wise, statically-panic-free default (removes the hole and the
`T: Default` bound, but **silently diverges from the user's own `Default`** — a trap that shows up
as "the editor added a component with different values than `spawn` would have"); (c) a
`Layout::from_size_align` heap allocation per call (breaks A.7's "0 bespoke" row for the one
structural path that claims it).
*The scratch's size and alignment are MEASURED, not guessed:* rung EG6 walks every registered
`ComponentLayout` at fixture time and asserts `size <= N && alignment <= A`, reporting the observed
maxima; the tail takes a `#[cold]` heap fallback whose hit counter is asserted **zero** on the
fixture and **non-zero** under the rung's RED mutation.

**D9 — `migrate_entity_attach_ids` gains a bytes-carrying sibling; it is not widened in place.**
*Reason:* its ZST `debug_assert!` (F9) is not decoration — it is the statement that makes the
byte-write-free fast path sound, and `add_tag` depends on that path staying byte-free. Widening it
with an `Option<&[&[u8]]>` parameter makes every tag attach carry a branch it does not need and
blurs the assertion into "…unless bytes were supplied," which is no longer checkable.
*Rejected:* widening in place (above); routing through `migrate_entity_insert<B: Bundle>` (F11 —
impossible from a `ComponentId`); building `add_default` on `Commands` (the deferred path is
generic over `Bundle` for the same reason, and an editor edit must be visible before the next
`drain`).

**D10 — `set_field` writes bytes and bumps the changed tick. It fires NO hooks.**
*Reason:* `on_replace` / `on_insert` in this tree carry **whole-value replace** semantics, and
`ParticleEffectHandle` is the proof: `on_replace` pushes `-1` for the OLD slot and `on_insert`
pushes `+1` for the NEW one (`boyko_render/src/particle.rs:171-185~`). A field-level poke is not a value replacement;
firing the pair per field would double-count the refcount on any multi-field edit, and firing it
once per field would fire it N times for an N-field edit. The tick bump, by contrast, is
**mandatory** — without it the table-path edit is invisible to every `Changed<T>`-gated system
while the identical dense-path edit is visible (F14), and that asymmetry is the *silent* failure:
an inspector edit to `Transform` would simply never reach the GPU.
*Consequence, recorded rather than hidden:* editing a hook-bearing component field-wise does not
run its hooks. The inspector queries `component_registry::get_hooks(id).is_some()` and says so on
the row.
*Deferred to v2:* `replace_component_by_id(e, id, bytes)` — a whole-value set that fires
`on_replace` pre-write and `on_insert` post-write. It is the honest home for hook-bearing edits and
it is a fifth shipping-API addition; it waits until an editor actually needs it.

**D11 — the change-tick bump is a NEW `boyko_ecs` fn (S3), not a behaviour change to `set_component_raw`.**
*Reason:* F14's asymmetry is a live defect in a **shipping** API with existing callers. Fixing it
under a reflection campaign would change shipping behaviour for reasons that belong to reflection —
exactly the inversion the directional rule exists to prevent. Report it (like F6's `has_component`
bug), add the explicit by-id bump, and let `boyko_ecs` decide the asymmetry on its own schedule.

**D12 — the row pointer always comes from `EcsMaster::get_component_raw` / `_mut`; the glue never re-implements it.**
*Consequence, and it is the point:* the glue's entire `unsafe` surface reduces to **two**
operations — `base.add(offset)` and the `(f.get)(p)` fn-pointer call — while the liveness and
generation checks, the three-way storage routing, the TB-safe column projection and the null-column
check are all inherited from an already-audited function (F1, F2).
*Rejected:* an inlined fast path in `boyko_reflect` (duplicates audited `unsafe`; would have missed
the dense branch, which landed four days after the analysis's snapshot).

**D13 — by-name lookup is a linear scan of `&'static [FieldInfo]`. No index, no cache.**
*Reason:* field counts are bounded (the shipped `BindAccessor` analogue caps at 255) and the path
is cold by construction. A per-type name index is a second allocation and a second source of truth
about a type's fields — B.2's drift class, in the one layer that must not have it.
*Measured, not assumed:* rung EG4 reports the scan cost at the widest type in the fixture.
*Rejected:* `HashMap<&str, u8>` per type (`clippy.toml` `disallowed-types`, plus the alloc, plus
the drift); a perfect-hash bake in the derive (CORE complexity for a cold path).

**D14 — dense components are in v1 scope and are enumerated by walking `dense_ids()`.**
*Reason:* F3 + F4 — both the id list and the O(1) membership probe are already `pub`. This is the
rung that decides whether the design "refuses to show the one component it is fully able to read."
*Measured:* `dense_ids().len()` is reported at EG1 — the fixture binary's own count; in the
production tree the population is today **1** (`GpuTransform3D`) — so the
per-entity source-2 cost is `len × O(1)` with a named `len`.

**D15 — enumeration is complete or it refuses; it never silently truncates.**
`components_of_into` returns `Err(BufferTooSmall)` rather than a short count.
*Reason:* a truncated component list is a *wrong answer that looks like an answer* — the inspector
would display an entity as not having a component it has. The upper bound is `MAX_COMPONENTS`
(`512`), so a caller can size a stack array once and never see the error; the error exists so that
undersizing is loud.

---

## 6. `unsafe` census — every block and the `// SAFETY:` it will carry

The glue writes **four** `unsafe` blocks (U1–U4) plus one in the `boyko_ecs` seam (U5). Everything
else is safe code over `pub` API. Each comment below is the text that lands verbatim.

**U1 — field pointer.** `boyko_reflect::ecs::field_ptr`
```rust
// SAFETY: `base` came from `EcsMaster::get_component_raw(entity, id)`, which returns
//   `Some` only for a live, generation-checked entity whose storage actually hosts
//   `id` — a non-null table column, or a live dense slot (component_api.rs:175-247).
//   It therefore points at ONE initialised value of the type registered at `id`,
//   valid for `get_layout(id).size` bytes and aligned to `get_layout(id).alignment`.
//   `field.offset` was baked by `core::mem::offset_of!` on that same type in the same
//   compile, so `offset + size_of::<F>() <= size_of::<T>()`: the sum stays inside the
//   one allocated object and the result is `align_of::<F>()`-aligned. Provenance is
//   inherited from `base` (arena-rooted, interior-mutable `SharedReadWrite`); NO
//   reference is formed here, so no retag occurs.
```

**U2 — typed read through the baked accessor.**
```rust
// SAFETY: `field.get` was monomorphised by the derive for the exact field type at
//   `field.offset` of the type registered at `id`, and installed only into this
//   `FieldInfo`. `type_info_of(id)` returned the `TypeInfo` the derive installed for
//   that same `ComponentId` through the write-once `OnceLock`, and the pre-flight
//   `get_layout(id).type_id == (info.type_id_fn)()` check re-established the type
//   identity in RELEASE — never a `debug_assert!`, because the legitimate
//   `--release --features reflect` editor build is exactly where a stale
//   (ComponentId, field) triple arrives after a hot-reload (§5). `p` satisfies the
//   accessor's contract by U1.
```

**U3 — typed write through the baked accessor.**
```rust
// SAFETY: pointer as U1/U2, with `get_component_raw_mut` supplying write-capable
//   provenance under `&mut EcsMaster` (whole-world exclusivity ⇒ no live reader,
//   component_api.rs:249-300). `field.set` performs the store THROUGH THE RAW POINTER
//   and never forms `&mut F`: a `Unique` retag through the arena's deliberately
//   `SharedReadWrite` provenance is the 14a-F2 / Phase-19 Tree-Borrows hazard class
//   (A.4). The kind check inside `set` is the load-bearing RELEASE guard; on mismatch
//   it returns `false` and writes nothing.
```

**U4 — default into scratch.**
```rust
// SAFETY: `scratch` is `>= layout.size` bytes at `>= layout.alignment` (asserted in
//   RELEASE immediately above; the `#[cold]` fallback covers the tail), currently
//   uninitialised, and unaliased — it is a local `MaybeUninit`. `info.default_in_place`
//   is the derive-generated `ptr::write(p.cast::<T>(), T::default())` for the type
//   registered at `id`, so it initialises exactly `layout.size` bytes over memory that
//   held no live value: no prior value is dropped, nothing leaks. If `T::default()`
//   unwinds it does so BEFORE the `ptr::write`, leaving `scratch` uninitialised and —
//   because this runs before any structural call — the world byte-for-byte untouched
//   (D8). The bytes are subsequently MOVED into the row by `add_component_by_id` and
//   must never be dropped here; `MaybeUninit` has no `Drop`, so the move is structural.
```

**U5 — the byte write in `migrate_entity_attach_ids_with_bytes` (lands in `boyko_ecs`).**
```rust
// SAFETY (mirrors `migrate_entity_attach_ids` Step 2, plus the byte write the tag path
//   does not need): `row == target.current_index == dst_pool.count()` (pools grow in
//   lockstep, debug-asserted) and `reserve_capacity(1)` guaranteed a committed slot, so
//   `write_at_unchecked_initialized(row, bytes)` targets a LOGICALLY-UNINIT slot and no
//   drop runs. `bytes.len() == get_layout(added_cid).size` is checked in RELEASE at the
//   `add_component_by_id` boundary (a debug-only check would let a short slice write a
//   partially-uninitialised row in the exact build an editor ships). `commit_units`'s
//   precondition `row == count` holds; both ticks land at `current_tick`, uniform with a
//   fresh insert.
```

---

## 7. Allocation and cost — how each number will be MEASURED

**The workspace's allocation instrument does not exist (F24).** `force_alloc_panic` survives in the
root `Cargo.toml`'s `check-cfg` list and an archived doc, but **no `.rs` file in the tree contains a
`#[cfg(force_alloc_panic)]` site** — so the CI job that runs the whole release suite under it
asserts nothing. This is reported to `docs/OPEN-QUESTIONS.md` and routed to
[`docs/REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md); this plan does not build on it.

**Instrument used instead:** a test-binary-local counting `#[global_allocator]` in
`crates/reflect_fixture/tests/ecs_alloc.rs` — **the fixture's, not `boyko_reflect`'s own**: the  <!-- doc-path-planned -->
harness drives the glue over `#[component(reflect)]` types, and a derived reflect fixture cannot
live in `crates/boyko_reflect/tests/` at all — the crate declares no `reflect` feature (GATES D4,
"now or ever"), so the derive's consumer-side `#[cfg(feature = "reflect")]` there is an
`unexpected_cfgs` red under the existing `-D warnings` gate (CORE D2) and a feature-off strip
besides. Per-binary, self-contained, exact, and it works under Miri. Every zero below is an **asserted equality**, not a claim.

| path | allocations | how measured |
|---|---|---|
| `components_of_into` (all three sources) | **0** | counting allocator, delta across the call, `assert_eq!(delta, 0)` |
| `fields_of` / `get_field` / `field_value` / nested descent | **0** | same |
| `get_field_by_name` / `set_field_by_name` | **0** | same |
| `set_field` (Prim / enum / array element) | **0** | same |
| `set_field` (`Str`) | 1 alloc + 1 free | same; the pair is asserted balanced |
| `presence_of` / `set_presence` (bitset) | **0** | same |
| `set_presence` (bitset, via `EnableTagId::try_from_component_id`) | **0** | same; the constructor is a `storage_kind` load and a newtype wrap — it mints nothing, which is the property F27 shows the by-name route lacks |
| `set_presence` (dynamic tag, by name) | **0** | same — `tag_by_name` is a lookup that **never mints** (`tag_api.rs:76`); the older formulation measured the idempotent path of `register_tag`, a call this plan no longer makes on either citizen |
| `add_default` | **0 bespoke** | same, plus the `#[cold]` heap-fallback counter asserted `0` on the fixture |
| `remove` | **0** | same |

| number | how measured, at which rung |
|---|---|
| `dense_ids().len()` on the fixture | printed by EG1; asserted `>= 1` and named in the rung report |
| the 512-id bitset scan's wall clock | EG3, isolated leg: `components_of_into` with source 3 compiled out vs. in, median of ≥ 21 iterations |
| whole `components_of_into` cost per entity | EG1 (sources 1+2), re-reported at EG3 (all three) |
| by-name scan cost at the widest fixture type | EG4; the field count of that type is reported beside it |
| `max(size)` and `max(alignment)` over all registered `ComponentLayout`s | EG6, walking `get_layout(0..MAX_COMPONENTS)`; the scratch constants are set from the observed maxima |
| `size_of::<IdEntry>()` | EG1, pinned by `const _: () = assert!(...)` |

No number above is stated as an expectation here. A plan that predicts its own measurements has
already decided what it will find.

---

## 8. Rung ladder

**Unconditional gate on every rung** (in addition to the rung's own): `cargo test -p boyko-reflect
--all-targets --no-fail-fast` **plain** (the crate has no `reflect` feature — GATES D4 — so there
is no "feature-ON leg of `-p boyko-reflect`"; that phrase was this plan's own instance of the B.9
error corrected in §10), plus **both** legs of the fixture:
`cargo test -p reflect-fixture --all-targets --no-fail-fast --features reflect-fixture/reflect`
and the same command with no `--features`;
`cargo clippy -p boyko-reflect -p reflect-fixture -p boyko-ecs --all-targets -- -D warnings`;
`cargo test -p boyko-ecs --all-targets --no-fail-fast` green (the seam rungs touch a shipping
crate); every new `unsafe` carries its `// SAFETY:`; author-only commit + push. `--workspace` is
**not** used — each worktree carries its own `target/` and disk is the binding constraint.

**Where the glue's tests live, stated once so no rung re-decides it.** The glue's *source*
(`boyko_reflect::ecs`) is un-`cfg`-gated and lives in `crates/boyko_reflect/src/`; every test that
constructs a `#[component(reflect)]` component lives in **`crates/reflect_fixture/`** (feature-ON,
Miri-allowlisted, FFI-free — local shapes: a reflect table component, a
`#[component(reflect, storage = "dense")]` struct of `[f32; 4]` arrays, a derived
`#[component(storage = "bitset")]` tag, a hook-bearing `#[repr(transparent)]` tuple struct). The
**real engine types** — `Transform`, `GpuTransform3D`, `EmitterActive`, `ParticleEffectHandle`,
`Visibility` — appear in exactly one rung, **EG8**, in `crates/reflect_dogfood/` (B.12/B.13 #1).
EG1–EG7's gates run on the fixture shapes; naming an engine type inside them is the blocker-1
defect (a gate that cannot be built in the package that runs it), and the engine names below are
kept only as *the production instance of the shape*, never as the gate's subject.

**Every rung names a RED MUTATION and the rung is not done until its red has been SEEN.** A gate
whose red nobody has run is not a gate; this campaign has paid for that lesson at L6, at L10 (12
benches in a gate table, none of which existed), and at B.6.

---

### EG0 — the seam census: compile the reachability claim before writing glue — **size S**

**Lands.** `crates/boyko_reflect/tests/seam_census.rs` (**plain** — the census compiles against
`boyko_ecs`'s public surface and constructs no reflect component, so it is the one glue test that
belongs in `boyko_reflect`'s own tests; there is no feature leg to name, GATES D4) that *compiles against*
`boyko_ecs`'s public surface and calls every accessor **on the glue's own route**: `entity_archetype_id`,
`archetype_master().get_archetype(_).component_ids()`, `get_component_raw`, `get_component_raw_mut`,
`dense_contains`, `dense_slot_of`, `dense_get_raw`, `dense_registry().dense_ids()`,
`get_component_changed_tick`, `storage_kind`, `residency_class`, `get_layout`, `register_tag`,
`tag_by_name`, ~~`register_enable_tag`~~, `add_tag`, `remove_tag`, **`is_enabled_id` / `enable_id` /
`disable_id`** — the list was stale against F27 in **both** directions and **D16** corrects it, along
with the *"intends to use"* framing that made two of its rows unfalsifiable. Plus a `trybuild`
`compile_fail` case per item on the **not-yet-reachable list**, which holds **two kinds** and is
load-bearing: the four things §4 must **add** (S1, S2, S3, **S4′**), whose fixtures are expected to
**flip to `pass` at EG2**, and the one thing this plan **refuses** to add
(`TagId::from_component_id`), whose fixture must stay red forever. A single undifferentiated
"negative list" is what let one item sit on it while a sibling plan declared the same item
mandatory.

> **`EnableTagId::from_component_id` moved from the negative list to the positive one, 2026-08-21
> (second pass).** D4 rejected it and EG0 landed a `compile_fail` fixture so *"the item reds the
> moment it becomes reachable"* — a good instrument pointed at the wrong item. D4 is reversed (F27),
> so the fixture would have fired at EG2 as a *success* signal misread as a regression. It is now a
> **positive**-list row, spelled `EnableTagId::try_from_component_id`, and — like the other three
> seam items — its EG0 row is a `compile_fail` **until EG2 lands**, then flips. The distinction the
> negative list still carries is real and is preserved for `TagId::from_component_id`: that one this
> plan genuinely refuses, because the dynamic-tag path needs no constructor at all (F27's second
> half, §4).

**Why this is a rung and not a paragraph.** §4 of the analysis asserts `add_default`/`remove`
"route through the existing structural insert/remove." F9 and F11 say they cannot. The difference
between those two sentences is three rungs of work, and the cheapest way to know which one is true
is to make the compiler say it — before EG6 discovers it halfway through.

**Gate.**
1. `seam_census.rs` compiles and runs green in the plain `-p boyko-reflect` leg. **The rung's
   central claim is MEASURED and holds** (EG0 audit, 2026-08-26): a throwaway probe in that exact
   package called every accessor above and reported `running 1 test … ok`. What follows are the
   gates that could **not** have detected it being wrong.
2. **FIVE** `compile_fail` fixtures with a blessed `.stderr` — one per item on the
   not-yet-reachable list, of **both** kinds: S1, S2, S3 and **S4′**, which flip to `pass` at EG2,
   and `TagId::from_component_id`, which never does. ~~*“Each **negative-list** item has a
   `trybuild` `compile_fail` fixture with a blessed `.stderr`.”*~~ **STRUCK (D18):** after the
   sidebar above moved S4′ to the positive list, “negative list” denotes exactly **one** item — so
   this gate demanded one fixture while the paragraph three lines above it demanded five, and
   both REDs below operate on fixtures the gate never required to exist. That is the C8 shape:
   the probe measures scaffolding the gate never pinned.
3. **The glob states a `>=` floor**, read from a `fixture_count(dir)` helper — the one
   `crates/reflect_fixture/tests/reflect_compile_fail.rs:91` already declares and which each of
   its four legs asserts against. **An empty glob is a VACUOUS PASS, and it was MEASURED in
   *this* package** (EG0 audit): `trybuild` printed *“There are no trybuild tests enabled yet”*,
   the harness reported `running 1 test … ok`, and the process exited **0**. `running N` does
   **not** catch it — the harness function runs and passes over zero fixtures. The nearest
   template, `crates/boyko_reflect/tests/c6_compile_fail.rs`, carries **no** floor, so the
   vacuous shape is the one an implementer inherits by proximity (D18).
4. The header cites §1 **by anchor**, never by copy, and the local assertion **parses
   `docs/REFLECTION-PLAN-ECS.md` at run time** — in-tree precedent
   `crates/boyko_app/tests/vg_r0d_census.rs:56`. Two constants declared in the same file and
   compared to each other is a gate that **cannot fail**, and a table copied into a `.rs` header
   is covered by **no** census (`GATED_DOCS` is `.md`-only), so the copy becomes a second carrier
   of the facts §1's anchors exist to keep single (D19). ~~*(The repo-wide anchor gate is a
   **separate** EG8 deliverable and cannot be added here — see the ordering note there.)*~~
   **STRUCK (D19): it LANDED at `eeb567be`, 2026-08-21, ahead of EG8 and wider than it** — all
   five reflection documents sit in `GATED_DOCS` (`tests/internal_docs_anchors.rs:280`) and the
   census runs green over eight tests. The ordering note's reason — *“this plan links three
   siblings that do not exist yet”* — is false: all five are on disk.
5. **Same commit, or this rung lands RED at the root census.** `seam_census.rs` now exists, so
   the `<!-- doc-path-planned -->` marker on **this rung's own Lands line** comes off and
   `PLANNED_EXACT`'s `("REFLECTION-PLAN-ECS.md", 2)` becomes `1`
   (`tests/internal_docs_anchors.rs:1716`). That pin asserts **equality**, not a ceiling,
   precisely so it cannot go quiet when the work finishes. No list on this rung carried the
   obligation until D19.
6. `#![cfg(not(miri))]` on the trybuild harness, and on `seam_census.rs` if gate 4 reads from
   disk — see **Miri** below.

**RED MUTATION.** Delete a fixture's blessed **`.stderr`** — *not* its `.rs` — and re-run:
`trybuild` reds with **`wip`** and *“writing the following output to `wip/…`”*. ~~*“Delete one
`compile_fail` expectation and re-run: `trybuild` must red on the now-unexpected success.”*~~
**STRUCK (D18): all three readings of “expectation” were MEASURED at the EG0 audit, and the
prescribed one does not fire.** Deleting the fixture `.rs` shrinks the glob and the target exits
**0**; deleting the `t.compile_fail(…)` call runs nothing; only deleting the `.stderr` reds, and
it reds with `wip`, **never** with *“unexpected success”* — that is the **next** red's message,
not this one's. A red whose artifact is ambiguous across three readings, two of which return
exit 0, is not a red.

Separately, add `EcsMaster::add_component_by_id` as a stub *before* EG2 and confirm its
`compile_fail` case flips — the fixture must red the moment the item becomes reachable, so the
not-yet-landed list cannot silently rot. **OBSERVED at the EG0 audit**: the harness reported
*“Expected test case to fail to compile, but it succeeded”*, exit **101**; the source was then
restored byte-identically under `cmp`.

*Second RED, and it is the one that would have caught this rung's own defect:* add
`EnableTagId::try_from_component_id` as a stub and confirm **exactly one** fixture flips. ~~If two
do — i.e. if a `from_component_id` refusal fixture still exists beside it — the negative and
positive lists disagree about the same item, which is the state EG0 and
[`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md)'s D10 were in until F27 forced the
question.~~ **STRUCK (D18): the discriminating half can no longer fire.** After the sidebar, the
only negative-list fixture names `TagId::from_component_id` — a **different type** and a
different function name — so stubbing `EnableTagId::try_from_component_id` cannot flip it, and
“two flipped” is unreachable by construction. What survives is the **exactly one** count; the
two-plans-disagree state this red was built to detect is now detected by gate 2's count of five.

**These mutations touch `boyko_ecs`, and that is NOT “starting EG2” (D20).** EG2's owner gate
binds that rung's *Lands* — a public item that survives the commit. A stub applied, its red
**OBSERVED**, and the source restored byte-identically under `cmp` leaves no public surface
behind; it is the rung protocol's own instrument, and it was exercised on this very rung at the
EG0 audit. Said here because EG2 reads *“the rung does not start before the answer”*, and an
implementer who reads only that either stalls at EG0 or breaches the owner gate to satisfy it.

**Inverted outcome is a result, not a failure.** If a negative-list item *does* compile, the seam
in §4 shrinks by one and EG2 gets smaller. Record it; do not re-argue it.

**Miri / proptest.** ~~None (no `unsafe`, no data).~~ **CORRECTED (D19): no proptest and no
`unsafe` — but TWO guards this rung must carry.** CI runs `cargo +nightly miri test
--all-targets … -p boyko-reflect …` (`.github/workflows/ci.yml:306`), so `--all-targets` picks
up every target this rung adds. The trybuild harness needs `#![cfg(not(miri))]` — it shells out
to `cargo`, which Miri cannot execute, and `crates/boyko_reflect/tests/c6_compile_fail.rs`
already carries the guard **in this package** for exactly that reason. If gate 4's assertion
parses the plan at run time, `seam_census.rs` needs the guard too: Miri refuses host file I/O
under isolation, measured at GATES G4's fifth red and recorded at
`crates/boyko_reflect/tests/c2_registry_source_census.rs:12~`. ⚠️ The CI comment block names
only *“the fixture's census/codegen harnesses”* — `reflect_fixture`'s, not this package's — so
the one place an implementer might have caught this points at the wrong crate.

---

### EG1 — `components_of_into`: table + dense, kind-tagged, zero-alloc — **size M**

**Lands.** `IdEntry`, `IdKind`, `Refusal`; `components_of_into` with **source 1** (archetype
signature via the safe accessor, D2) and **source 2** (`dense_ids()` × `dense_contains`, D14);
`display_name`; the `TableGpu` / `TableOpaque` classification (`residency_class` + `type_info_of`);
`Err(BufferTooSmall)` (D15). Source 3 is absent and its slot is a `// EG3` marker, not a stub that
returns something.

**Gate.**
1. **The B.3 assertion, and it is the one the design would otherwise ship wrong.** A fixture entity
   carrying the fixture's reflect **table** component **and** its
   `#[component(reflect, storage = "dense")]` component (the `GpuTransform3D` shape — `[f32; 4]`
   arrays, 96 B; the real `GpuTransform3D` lives in `boyko_render`, out of this package's reach,
   and is EG8 gate 2's subject) enumerates **both**, with kinds `Table` and `Dense`.
2. `IdKind` classification is exhaustive over the fixture: a `#[component(reflect)]` table component →
   `Table`; a plain `#[derive(Component)]` with no `#[component(reflect)]` → `TableOpaque`; an
   `EcsMaster::register_tag("editor_marker")` dynamic tag → `TableOpaque` with
   `display_name == "editor_marker"` (F20); a `classify_component_residency(_, Gpu)` fixture →
   `TableGpu`.
3. Zero allocations (counting allocator, `assert_eq!(delta, 0)`).
4. `Err(BufferTooSmall)` on a 1-slot buffer against a 3-component entity — and the buffer's
   contents are asserted **unmodified**, so a caller cannot mistake a partial fill for an answer.
5. `const _: () = assert!(size_of::<IdEntry>() == N)` with `N` set from the measurement.
6. **Miri-TB** (`-Zmiri-tree-borrows`) over the whole enumeration on a world holding table + dense
   components, with a sibling structural migration interleaved between two enumerations.

**RED MUTATION.** Three, and the second one is the interesting one.
* **R1** — delete source 2. `dense_component_is_enumerated` reds. *(This is §4 as literally
  written; the red is the proof that the analysis's spine was blind.)*
* **R2** — replace `dense_contains` with `has_component` in source 2. The test must stay **GREEN**
  (F6's bug does not affect dense ids). This is a *negative* control: it proves EG1's gate is not
  accidentally covering EG3's trap, so EG3's red is genuinely EG3's.
* **R3** — swap the safe accessor for a `&*inland.archetype_ptr()` deref. Under
  `-Zmiri-tree-borrows`, this must red. **If it does not red, that is a reportable finding**: it
  means BUG-MIGRATE-TB-1 is not enforceable by the instrument the tree relies on, and D2's second
  reason is unsupported (its first reason, F8, stands regardless). Report it either way — the rung
  tests the instrument, not only the code.

**Measured and reported.** `dense_ids().len()` on the fixture; `components_of_into` wall clock per
entity; `size_of::<IdEntry>()`.

---

### EG2 — the by-id structural seam in `boyko_ecs` — **size M** — **OWNER-GATED**

**Lands** (in `boyko_ecs`, the shipping crate — §4): `add_component_by_id` (S1) with `AddOutcome`;
`remove_component_by_id` (S2); `mark_component_changed` (S3); **`EnableTagId::try_from_component_id`
(S4′, in `component_registry::tags` — not on `EcsMaster`)**;
`migrate_entity_attach_ids_with_bytes` (`pub(crate)`, D9). Doc comments name `add_tag`/`remove_tag`
as the precedent and name **no reflection type**.

**Owner gate before the first edit.** This rung widens a shipping crate's public API for a
dev-only feature. Analysis **B.13 #2** routes it to the owner — **as ONE call over FOUR items**,
which is a change: S4′ was previously filed twice, as B.11 #2 (this plan's seam) and as
[`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md)'s B-1, with opposite recommendations,
because it was reached from two directions. §4's table states each item's independent merit; the
owner sees that table and the four signatures, and says yes or names a subset. **BOUNDARY's B4 blocks
on the same answer** and does not ask separately.

**Gate.**
1. `add_component_by_id` on a **table** component: the entity migrates, the bytes land, `on_add` +
   `on_insert` fire once each, the changed and added ticks are `current_tick`.
2. `add_component_by_id` on a **dense** component routes to `dense_insert_and_fire`, the archetype
   id is **unchanged** (the dense no-migration contract, `entity_archetype_id` before == after), and
   the hooks fire.
3. `AddOutcome::AlreadyPresent` on a second call — and the existing value is asserted **byte-identical**
   (an editor's "Add Component" must not clobber).
4. `AddOutcome::Rejected(WrongByteLen)` for `bytes.len() != get_layout(id).size`, asserted in a
   `--release` build (the release-editor gap, §5).
5. `AddOutcome::Rejected(NotTableOrDense)` for a bitset id; `Rejected(GpuResident)` for a `Gpu` id.
6. `remove_component_by_id` on a `Drop`-carrying fixture: `on_replace` + `on_remove` fire once each
   against the **dying** row, and the drop counter increments **exactly once**.
7. `mark_component_changed` on the table path makes a subsequent `Changed<T>` query match; on the
   dense path it is idempotent with the arm that already bumps (F14).
8. **`EnableTagId::try_from_component_id`** returns `Some` for a bitset id and `None` for
   `Table`/`Dense`, **and the round-trip closes**: `EnableTagId::try_from_component_id(t.component_id())
   == Some(t)` for a tag minted by `register_enable_tag` — the public counterpart of the internal
   `enable_tag_id_bridges_to_component_id_round_trip` (`component_registry/mod.rs:1676`).
9. **The F27 assertion, and it is the one this rung exists to make impossible to get wrong.** For a
   **derived** `#[component(storage = "bitset")]` type — a `boyko_ecs`-test-local fixture registered
   through `register_new` (that route, not the name mint, is what "derived" means here; the
   production instance `EmitterActive` lives in `boyko_render`, which this crate's tests cannot
   reach, and is exercised at EG8) — assert that `register_enable_tag(display_name(id))` returns an
   `EnableTagId` whose `component_id()` **differs** from `id` — i.e. the by-name route demonstrably
   mints a *different* tag. This is a test of the
   kernel's documented behaviour, not a regression guard; it exists so that the next person who
   reaches for the obvious substitute finds a test saying why it is wrong, in the crate where the
   behaviour lives.
10. **Miri-TB** over `migrate_entity_attach_ids_with_bytes` for a table component with a `Drop` field.

**RED MUTATION.**
* Feed `add_component_by_id` a slice one byte short with the length check deleted → Miri reds on
  the partially-uninitialised row (and the release test reds without Miri).
* Convert the length check to `debug_assert!` → the `--release` leg of gate 4 goes green-wrong →
  the rung reds. *(This is the release-editor gap made mechanical rather than documentary.)*
* Delete the tick write in `mark_component_changed` → gate 7 reds.
* Drop the `storage_kind(id) == Bitset` check from `try_from_component_id` → gate 8's `None` arm
  reds for a `Table` id, i.e. the constructor stops being the *proof* F16 says an `EnableTagId` is
  and becomes a cast.
* Point the glue's presence read at `has_component` instead of the constructor → EG3 gate 1 reds
  with a confident wrong `false` (F6). *(Kept here as a cross-rung note: the red lives at EG3, which
  is where the caller is.)*

---

### EG3 — bitset presence, source 3, and the `has_component` trap — **size S**

**Lands.** `components_of_into` source 3 (D3: scan `0..MAX_COMPONENTS` on
`storage_kind == Bitset`, keep those with the bit set); `presence_of`; `set_presence`. **The two
citizens take two different routes, and the split is F27's, not a style choice:**

| citizen | read | write |
|---|---|---|
| **derived** `#[component(storage = "bitset")]` (`EmitterActive`) | `EnableTagId::try_from_component_id(id)` (S4′) → `is_enabled_id` | the same `EnableTagId` → `enable_id` / `disable_id` |
| **dynamic tag** (`register_tag`) | enumerated from the signature; presence is *"it is in the signature"* | `tag_by_name(display_name(id))` (public, `tag_api.rs:76`) → `add_tag` / `remove_tag` — **no new API**, because a dynamic tag's name *is* its `TAG_NAMES` key (F27) |

~~bitset → `register_enable_tag(display_name)`~~ is **struck**: on a derived bitset id that call
**mints a different tag** and toggles its bit (F27). `register_tag` on a *dynamic* tag id does not,
and the plan uses `tag_by_name` there anyway so that the write path cannot mint at all.

**Gate.**
1. The fixture's derived bitset tag (`EmitterActive`'s shape — the real `EmitterActive`,
   `boyko_render/src/particle.rs:164`, is EG8's, in `reflect_dogfood`) enabled on the fixture entity appears in
   `components_of_into` with `kind == Bitset`, and `presence_of` returns `Ok(true)`.
2. `presence_of` on a `Bitset` id whose bit is clear returns `Ok(false)` — **not** `ComponentAbsent`.
   The two are different rows.
3. `get_field` / `fields_of` on a `Bitset` id return `Err(PresenceOnly)`, in release.
4. `set_presence(false)` then `presence_of` → `Ok(false)`; the archetype id is unchanged across
   both toggles (the bitset no-migration contract).
5. Dynamic-tag presence round-trips by name: `register_tag("x")` → `add_tag` → enumerate →
   `set_presence(id, false)` → gone from the enumeration.
6. Zero allocations on the presence write path — for the dynamic tag, `tag_by_name` **never mints**,
   so there is no allocation to account for at all; the old formulation (*"zero on the second
   `register_*_tag(name)` call"*) measured the idempotent path of a call this plan no longer makes on
   either citizen.
7. **The F27 gate, and it is the one that makes gate 4 mean something.** After
   `set_presence(<the derived bitset fixture's id>, false)`, assert
   **`register_enable_tag(display_name(id))` was never called** — a `#[cfg(test)]` counter, or,
   stronger and allocation-free, assert that
   `NEXT_ID`'s observable high-water (via one probe `register_tag`) is **unchanged across the toggle**.
   A presence write that mints an id has taken the wrong route even if the bit it cleared happens to
   read back correctly, and *"the bit reads back correctly"* is exactly what the wrong route
   produces on a freshly-minted tag.
8. **Miri-TB** over the full three-source enumeration.

**RED MUTATION.** Point `presence_of` at `EcsMaster::has_component` instead of the `EnableTagId`
route. The derived bitset fixture reports **absent** while it is demonstrably enabled — gate 1 and gate 2 both
red. This is F6 weaponised: the trap the analysis found is now a test that fires if anyone reaches for
the obvious probe.

**Second RED, and it is this rung's own defect made mechanical.** Route `set_presence`'s bitset arm
through `register_enable_tag(display_name(id))` — the plan's original §4 text. **Gate 4 goes green**
(the newly minted tag's bit is set and then cleared, so `presence_of` on *that* tag round-trips) while
**gate 1 and gate 7 red**: the fixture tag is still enumerated as enabled, and the id high-water moved.
Run it. It is the exact code the first revision specified, it fails in a way that looks like success
from inside its own toggle, and gate 7 exists because gate 4 alone could not tell the difference.

**Third RED:** delete source 3 → gate 1 reds.

**Measured and reported.** The 512-id scan's isolated wall clock, and the whole three-source
`components_of_into` per entity.

---

### EG4 — the read path and the refusal taxonomy — **size M**

**Lands.** `fields_of`, `get_field`, `field_value`, `get_field_by_name` (D13); U1 and U2; the
release `type_id` cross-check; D7's classification-first GPU refusal; the `TypeInfo`-present guard
that structurally prevents the `TypeId` check from ever seeing a dynamic tag (§3).

**Gate.**
1. **The full §3 matrix, one assertion per row**, on one fixture world (local shapes throughout —
   the engine-type run of this same matrix is EG8's): the reflect table fixture → fields;
   the dense fixture → fields; a no-`#[component(reflect)]` component → `NotReflectable`; a dynamic tag
   → `NotReflectable` with a non-blank `display_name`; a `Gpu`-classified fixture → `NoHostBytes`;
   the derived bitset fixture → `PresenceOnly`; a despawned entity → `EntityDead`; an absent id →
   `ComponentAbsent`; `f == fields.len()` → `FieldOutOfRange`; a `Nested` field through the scalar
   `get_field` → `KindMismatch`.
2. Nested descent reads a leaf of a local depth-2 nest (the `Transform → Vec3 → f32` shape — the
   real descent is EG8's) and an **array** element of the dense fixture's `[f32;4]` (the
   `TrsPacked.pos` shape, B.8).
3. Zero allocations across the whole matrix.
4. **Release leg**: the whole matrix re-run under `--release`, with the `type_id` cross-check
   proven live by gate 5.
5. **A `TypeInfo` whose `type_id` disagrees with the id's registered layout is refused, in release** —
   this is what proves U2's release `type_id` cross-check is live.

   > **The route is corrected, 2026-08-21 (second pass): the original one is not constructible.**
   > *"The id re-registered to a different fixture type between the read and the enumeration"* cannot
   > happen in this process — `NEXT_ID` is a monotonic `fetch_add` with **no recycling**, and
   > `component_id()` is a per-type `static ID: OnceLock` resolved once (**F28**). An implementer
   > would have either faked this gate or dropped it, and EG4's second RED mutation depends on it.
   >
   > **A constructible route exists and is cheap**, because `install_type_info` is `pub` and
   > **first-writer-wins** (CORE C2 gate 1). Take a fixture type `Plain` that is
   > `#[derive(Component)]` **without** `#[component(reflect)]` — nothing installs a `TypeInfo` for it,
   > so the test is the first writer — and install *another* type's static under its id:
   >
   > ```rust
   > let plain = Plain::component_id().0;                    // registered layout: TypeId::of::<Plain>()
   > boyko_reflect::install_type_info(plain, <Other as Reflect>::TYPE_INFO);   // claims TypeId::of::<Other>()
   > let e = ecs.spawn((Plain::default(),));
   > assert_eq!(get_field(&ecs, e, ComponentId(plain), 0), Err(Refusal::NotReflectable));
   > ```
   >
   > `get_layout(plain).type_id != (info.type_id_fn)()`, so the release cross-check fires **before any
   > pointer is taken** — which is also the ordering U2's SAFETY comment claims and which nothing else
   > in this rung asserts. No ordering games, no id prediction, no hand-written `impl Component`.
   > *(The refusal is `NotReflectable` rather than a new arm: from the caller's side the id has no
   > **usable** `TypeInfo`, and inventing a `TypeIdMismatch` arm would give the inspector a row state
   > that can only arise from a bug in the process that produced it.)*
6. **Miri-TB** over U1 + U2 for every `ValueKind` the fixture exercises.
7. **proptest**: for a fixture type with a randomly generated field-value vector, writing it with
   the typed API and reading every field back through `get_field` reproduces the vector exactly
   (`f32`/`f64` compared bitwise, not by `==`, so a NaN payload cannot be laundered).

**RED MUTATION.**
* Delete the `residency_class == Gpu` check → the `Gpu` fixture returns host garbage instead of
  `NoHostBytes` → gate 1 reds. *(And this red runs under Miri, which a null-column test could not —
  F21, D7(c).)*
* Convert the `type_id` cross-check to `debug_assert_eq!` → gate 5's release leg goes green-wrong →
  the rung reds.
* Replace `Result<_, Refusal>` with `Option<_>` → gates 1 and 5 **fail to compile**, which is the
  intended shape of that red: D6 is enforced by the test's own types.

**Measured and reported.** The by-name scan cost at the fixture's widest type, with that type's
field count printed beside it.

---

### EG5 — the write path, the change tick, and the release kind check — **size M**

**Lands.** `set_field`, `set_field_by_name`; U3; the mandatory `mark_component_changed` call after
every successful write (D10, D11); the release `-> Result<(), KindMismatch>` guard.

**Gate.**
1. `set_field` → `get_field` round-trips for every `ScalarKind` the fixture carries, on **table**
   and on **dense** storage.
2. **The F14 assertion.** After `set_field` on a **table** component, a `Changed<T>` query matches
   on the following frame. The same assertion on a **dense** component. Both, because the whole
   point is that they now behave alike.
3. Writing a `Scalar` whose tag disagrees with the field's `ValueKind` returns `Err(KindMismatch)`
   and leaves the bytes **byte-identical**, asserted in `--release`.
4. Writing a `Bitset` / `Gpu` / non-reflectable id returns the matching refusal, in release.
5. `set_field` on the fixture's hook-bearing tuple struct (a local `on_insert`/`on_replace` pair
   modelled on `ParticleEffectHandle`, `boyko_render/src/particle.rs:185` — the production type is EG8's) fires
   **no** hooks — the D10 limitation asserted, not assumed — while the changed tick **does**
   move. A hook-fire ledger counts zero.
6. Nested-leaf write through `NestedCursorMut` at composed offset (A.4's "no field handle escapes")
   round-trips.
7. **Miri-TB** over U3 for every write arm, including the `Str` arm's raw
   `drop_in_place` + `ptr::write` (CORE owns the accessor; this rung owns the ECS-rooted pointer it
   receives) — with `-Zmiri-tree-borrows`, and the alloc/free pair asserted balanced by the counting
   allocator.
8. **proptest**: random `(field, Scalar)` sequences against a shadow model; after each, every field
   reads back equal to the model, and every mismatch is a `KindMismatch` rather than a silent write.

**RED MUTATION.**
* Delete the `mark_component_changed` call → gate 2's **table** leg reds while its dense leg stays
  green. That asymmetry in the red is the F14 defect made visible.
* Convert the kind check to `debug_assert!` → gate 3's release leg goes green-wrong → reds.
* Form `&mut F` inside the `Str` setter instead of the raw store → Miri-TB reds (gate 7). If it does
  **not** red, report it: A.4's headline `unsafe` argument would then rest on an unenforced claim.

---

### EG6 — `add_default` / `remove`, drop-safety and unwind-safety — **size M**

**Lands.** `add_default` (D8: scratch-first, move-not-drop) and `remove`, over S1/S2; the
`MaybeUninit` scratch with its release size/align assert and `#[cold]` heap fallback; U4.

**Gate.**
1. `add_default` on an absent table component: it appears in `components_of_into`, its fields read
   back equal to `T::default()`'s bytes, `on_add`+`on_insert` fire once each.
2. `add_default` on an absent **dense** component: same, with the archetype id unchanged.
3. `add_default` on a present component → `Err`/`AlreadyPresent`, value untouched.
4. `add_default` on `Bitset` → `PresenceOnly`; on `Gpu` → `NoHostBytes`; on non-reflectable →
   `NotReflectable` (no `default_in_place` exists without a `TypeInfo`); **on a reflectable type
   whose `TypeInfo.default_in_place` is `None` → `NoDefault`** — CORE **D20**, which made that field
   `Option` so a type with no `Default` (or one carrying `#[reflect(no_default)]`) stays fully
   *inspectable* while refusing to be *synthesized*. Without this arm the same type would either be
   un-derivable or would reach `add_default` with nothing to call.
5. **Drop-count test (A.8).** A fixture `{ pod: u32, s: String, n: Nested { s: String } }`:
   `add_default` then `remove` leaves the global drop counter at exactly **2** (one per `String`),
   and the counting allocator's alloc/free deltas balance to zero.
6. **Unwind test.** A fixture whose `Default::default()` panics: `add_default` returns via unwind,
   and afterwards `components_of_into` is byte-identical to before, the entity's archetype id is
   unchanged, and a subsequent `EcsMaster` drop runs clean under Miri.
7. `remove` of the **last** component routes the entity into the EMPTY archetype and leaves it
   alive with zero components (the O3 contract, `migration_helpers.rs:1652-1658~`).
8. Zero bespoke allocations on `add_default`; the `#[cold]` fallback counter is `0` on the fixture.
9. **Miri-TB** over U4 + U5 + the whole attach/detach pair.
10. **proptest**: random `add_default` / `remove` / `set_field` sequences over a 5-component fixture;
    after every step, `components_of_into` equals the model set, every present component's fields
    equal the model, and at the end the drop counter equals the number of removals × the type's
    owning-field count.

**RED MUTATION.**
* Move `default_in_place` from the scratch onto the **committed target row** and run gate 6 →
  Miri reds on the drop walk over an uninitialised row. *(This is D8's entire reason, executed.)*
* Drop the scratch instead of forgetting it → gate 5's drop count reads 4, and Miri reds on the
  double free.
* Shrink the scratch to 32 bytes → the 96 B dense fixture (the `GpuTransform3D` shape) takes the `#[cold]` fallback and gate 8's
  `counter == 0` reds. *(This is how the measured constant is proven to be doing work rather than
  being large enough to never matter.)*

**Measured and reported.** `max(size)` and `max(alignment)` across every registered
`ComponentLayout` at fixture time; the scratch constants derived from them.

---

### EG7 — hook / observer / change-detection conformance — **size S**

**Lands.** No new API. A conformance ledger test that pins, for each of the four structural /
value operations, **exactly** which of `on_add` / `on_insert` / `on_replace` / `on_remove` fire, in
what order, how many times, and whether the changed tick moves.

**Why a separate rung.** §4 claims `add_default`/`remove` "inherit hooks/observers/change-detection
for free." Through the **new** seam that is a claim about code that did not exist when the claim was
written. And D10 makes `set_field` deliberately *not* inherit them. Both halves need to be written
down as a table and asserted, or the next reader will assume the wrong one.

**Gate.** One ledger table, asserted row by row on the fixture's hook-bearing tuple struct (a
local `on_insert`/`on_replace` refcount pair modelled on `ParticleEffectHandle` — the production
type's balance is EG8's, where `boyko_render` is reachable) and a synthetic observer-carrying
fixture:

| op | `on_add` | `on_insert` | `on_replace` | `on_remove` | changed tick |
|---|---|---|---|---|---|
| `add_default` (absent → present) | 1 | 1 | 0 | 0 | set to `current_tick` |
| `add_default` (already present) | 0 | 0 | 0 | 0 | unchanged |
| `remove` | 0 | 0 | 1 | 1 | n/a (row dies) |
| `set_field` | 0 | 0 | 0 | 0 | **bumped** |
| `set_presence` (bitset) | 0 | 0 | 0 | 0 | n/a (no tick on a bit) |

Plus: the local refcount is **balanced** across an `add_default` + `remove` pair (`+1` then `-1`)
— the proof that the seam's hook firing is not merely present but *correctly paired*. The same
balance over the **production** pair — `ParticleEffectHandle` / `ParticleEffectRefs` — is an EG8
line item, because only `reflect_dogfood` can link `boyko_render`.

**RED MUTATION.** Swap the `on_replace` / `on_insert` order in the seam → the ledger reds. Delete
the hook fire from `remove_component_by_id` → the refcount ends at `+1` and the balance assertion
reds. Add a hook fire to `set_field` → the `set_field` row reds (D10 is enforced in both directions,
so a future "helpful" addition is caught).

---

### EG8 — the dogfood acceptance fixture and the Miri certification — **size M**

**Lands.** A.9's end-to-end acceptance test, with its fixture corrected per §6.1 and the A.9
correction — **in `crates/reflect_dogfood/`, and split from the Miri row that used to share it:**

> **Two packages, 2026-08-21 (second pass), and neither constraint is negotiable.**
> `Transform` / `Name` / `Visibility` live in `boyko_scene` and `GpuTransform3D` / `EmitterActive` in
> `boyko_render`, so this fixture requires those crates' `reflect` features (analysis **B.12**, owner
> sheet **B.13 #1**) — and `boyko_render` reaches `boyko_rhi_vulkan`, which **Miri cannot execute**.
> So `reflect_dogfood` carries this test and is **not** on the Miri allowlist, while
> `reflect_fixture` — FFI-free, `boyko-ecs`/`boyko-macros`/`boyko-reflect` only — carries the Miri
> row and reproduces the same *shapes* locally. The fixture's shapes are **the primary gated
> subject**; this test is the additional claim that the layer works on the engine's own components.
> [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)'s **D15/G0** own both packages.

```text
{ Name(NameId(u32)),                                   // tuple struct + Nested, depth 2
  Transform { translation: Vec3, rotation: Quat, scale: Vec3 },   // Nested, depth 2
  Visibility (#[repr(u8)] fieldless),                  // top-level TypeKind::Enum
  GpuTransform3D { prev: TrsPacked, curr: TrsPacked }, // DENSE + [f32;4] arrays
  EmitterActive (bitset),                              // presence view
  ParticleEffectHandle(pub u32),                       // tuple struct + real hooks (on_insert/on_replace refcount pair)
  <a locally declared StrFixture { s: String }>,        // the Str arm, no production consumer
  <a locally declared GpuFixture, classify_component_residency(_, Gpu)>,
  <a dynamic tag minted by register_tag("editor_marker")> }
```

Enumerate; read one value of each kind; descend into `Transform` **and** into `Name → NameId`; read
an **array** element of `TrsPacked.pos`; set a nested leaf; set `Visibility`'s variant; toggle
`EmitterActive`; `add_default` + `remove` **`ParticleEffectHandle`** and assert its
`ParticleEffectRefs` refcount balanced (`+1` then `-1`) — EG7's ledger re-run against the
production hook pair, not only the fixture's local one; assert the refusal for the `Gpu` and
dynamic-tag rows; re-read everything.

**Gate.**
1. The fixture runs green in the feature-ON leg, debug **and** release.
2. `GpuTransform3D` is present in the enumeration — *the single highest-value assertion, because it
   is the failure the design would otherwise ship: refusing to show the one component it is fully
   able to read.*
3. Zero allocations across the whole acceptance path except the `StrFixture` set (1 + 1, balanced).
4. **The equivalent fixture in `reflect_fixture` — same shapes, local declarations — runs under
   Miri-TB.** *This gate is corrected: the acceptance fixture itself cannot run under Miri, because
   it reaches `boyko_rhi_vulkan` through `boyko_render`. The Miri claim and the dogfood claim are two
   tests, and saying so is what makes both of them runnable.*
5. **The Miri gate is proven live**, in **both** its rows — the allowlist is
   [`REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)'s deliverable (its **D4/G4**), and this
   rung's job is the **proof**: land a deliberate out-of-bounds read in a `#[cfg(test)]` fn, confirm
   the CI Miri leg reds, revert. Do it **twice**, once per row, because they cover different code:
   * `-p boyko-reflect` **PLAIN** (no `--features` — the crate has no `reflect` feature, and
     `--features reflect` on it is a hard cargo error) → covers the glue's arithmetic and the
     accessors;
   * `-p reflect-fixture --features reflect-fixture/reflect` → the only row that reaches
     derive-generated `unsafe`.
   Without both, "Miri-TB is mandatory" is a gate that cannot fail — the exact failure mode recorded
   at B.6, B.9 and L10. ⚠️ And the fixture's process-spawning tests and its bench carry
   `#[cfg(not(miri))]`, or this row reds for a reason unrelated to reflection and the likeliest "fix"
   deletes the row.
6. ~~**The four plan documents join the repo-wide anchor gate — and this rung is its single owner.**~~
   **LANDED 2026-08-21, ahead of this rung and wider than it.** Add `REFLECTION-PLAN-ECS.md` and its
   three siblings to `GATED_DOCS` (`tests/internal_docs_anchors.rs:280`, which held exactly four
   documents when this rung was written, none of them a reflection plan) and give each a
   `("REFLECTION-PLAN-*.md", 0)` row in `OVER_WAIVED_MAX` (`:1845-1880`). **What actually landed
   registers five documents, not four** — `REFLECTION-ANALYSIS.md` alongside the four plans — each
   with its `0` row, and it also carried the `GATES` Appendix GC caveat deletion this rung owed.
   Whether EG8 keeps a gate-6 line item is bookkeeping for the campaign owner. The gate then checks, on every `cargo test`, that every `crates/...` path these
   plans mention exists on disk and that every `file.rs:N` anchor lands on a definition-shaped line —
   which is what makes §1's twenty-nine rows worth citing rather than decorative. **Ordering note,
   and it is the reason this is not an EG0 item:** the gate's *path* check fails on a dead markdown
   link, and this plan links three siblings that do not exist yet. Registering before all four have
   landed would red the workspace for a reason that has nothing to do with reflection.

   **Same-commit obligation:** `docs/REFLECTION-PLAN-GATES.md` deferred this registration and called
   the deferral *"a choice, not an omission"*, and its Appendix GC says its anchors are
   *"hand-checked, not machine-checked"*. Both statements become false the moment this gate lands, so
   **this commit deletes them** — GATES' §4 row now points here, and GC's caveat carries an expiry
   naming this rung. Two plans holding opposite decisions about one gate is how a gate ends up owned
   by nobody; a caveat that outlives the gate it describes is the doc-rot class this campaign
   measured at 75 %.

**RED MUTATION.** Remove `-p boyko-reflect` from the Miri allowlist → gate 5's first deliberate-UB
run goes **green**, which is the red; then remove `-p reflect-fixture` and confirm the *second* one
does too. **Both, separately** — a single-row check would have certified the row that covers no
derive-generated `unsafe` and reported the obligation met. Repoint one §1 anchor by a single line →
gate 6 reds (that one-line drift is the exact defect the anchor gate was built to catch, measured at
75 % across the pre-repair navigation docs). Separately: build the acceptance test with the feature
OFF and assert the whole file is `#[cfg(feature = "reflect")]`-gated to nothing (it must compile to
an empty binary, not fail).

---

## 9. Deferred, explicitly, and to what

| deferred | to | reason |
|---|---|---|
| `FieldMut<'a>` — a borrowed `&mut` handle into a field | **v2**, behind a full Tree-Borrows analysis against concurrent query borrows | the "cached pointer + reborrow" class Miri caught in boyko after three critic rounds approved it (14a-F2). The value model (get/set `Scalar` by copy) has no aliasing obligation at all; a handle has the whole one. |
| `replace_component_by_id` — whole-value set that fires `on_replace` + `on_insert` | **v2** | a fifth shipping-API addition; D10's limitation is recorded and displayed instead. It waits until an editor needs it. |
| Splitting `TableOpaque` into "opted out" vs. "dynamic tag" | **v2**, blocked on a `pub fn is_dynamic_tag(id)` in `boyko_ecs` | `DynamicTagMarker` is private and unnameable outside the crate (F20). A cosmetic gain for a sixth shipping-API addition. |
| Fixing `has_component`'s missing `Bitset` branch (F6) | **`boyko_ecs`, not this campaign** | changes a shipping API's behaviour for existing callers; reported to `docs/OPEN-QUESTIONS.md`. The glue routes around it and EG3's RED enforces the routing. |
| `EcsMaster::is_enabled_raw` (the former S4) | **not built** | With S4′ in hand it is `EnableTagId::try_from_component_id(id).map(\|t\| ecs.is_enabled_id(e, t))` — a one-line convenience with no second caller. It lives as a private helper in `boyko_reflect::ecs`, not as a fifth public item on a shipping crate. D4 records why the choice went the other way and why F27 reversed it. |
| Resolving `set_component_raw`'s table/dense tick asymmetry (F14) | **`boyko_ecs`, not this campaign** (D11) | same reason. The explicit `mark_component_changed` (S3) is additive and breaks nothing. |
| Reviving or deleting the dead `force_alloc_panic` CI job (F24) | **GATES** | out of this plan's scope; the glue brings its own allocator instead. |
| `Vec` / `Map` / collections, data-carrying enums, `Option<T>`, generics, `repr(packed)` | **v2** (A.1) | scope (B). The glue is agnostic: it moves bytes at an offset. |
| Resources (`State<S>` and friends) | **not v1, possibly never by this route** | reflection is keyed by `ComponentId`; a Resource has a `ResourceId`. Out of scope twice over (§5). |
| `StorageKind` discriminant 3 (reserved for relationships) | **whenever it lands** | D2/D3 are written so a fourth kind appears in `components_of_into` without a glue change, but its row in §3's matrix will need a disposition. |

---

## 10. Dependencies on the sibling plans

**On [`docs/REFLECTION-PLAN-CORE.md`](REFLECTION-PLAN-CORE.md)** — hard, and ordered:

| needed | by rung | note |
|---|---|---|
| `TypeInfo`, `FieldInfo { offset, kind, get, set }`, `Scalar`, `ValueKind`, `TypeKind`, `FieldValue`, `NestedCursor` / ~~`NestedCursorMut`~~ | EG4, EG5 | the whole value model. ⚠️ **`NestedCursorMut` is scheduled by NO CORE rung, and EG5 gate 6 is written around it** — verified 2026-08-26: zero hits in `crates/`, zero in `docs/REFLECTION-PLAN-CORE.md`; CORE's C6 is *"the recursion contract, **read side**"* and its two remaining rungs are C10 (enums) and C11 (`Str`). The type is specified only in `docs/REFLECTION-ANALYSIS.md` A.4 (FIX W1), which also names the escape hatch — *"if TB proves troublesome, nested-leaf write slips to v2"*. This is the **C7 defect class inside the dependency table**: ECS must put it to CORE **before** EG5, not discover it halfway through EG5 (D17) |
| `type_info_of(id) -> Option<&'static TypeInfo>` | EG1 | EG1 needs it only to classify `Table` vs. `TableOpaque`; a stub returning `None` unblocks EG1 if CORE is behind |
| `install_type_info` carrying the **release `assert!(storage_kind(id) != Bitset)`** | ~~EG3~~ **CORE C9 — LANDED** | the runtime half of D5. ~~If CORE declines it, EG3 must add the check on its own read path and say so~~ **STRUCK 2026-08-26 (D17): CORE did not decline, so this conditional is DISCHARGED.** `crates/boyko_reflect/src/registry.rs:87` carries the plain release `assert!(storage_kind(component_id) != StorageKind::Bitset, …)` today — verified in tree at the EG0 audit. CORE recorded the obligation to strike it here (its §7.4 item 4: *"must be struck when ECS is next edited, or EG3 builds the same check twice"*), and this edit discharges it. The live edge is now **EG3 → CORE C9**, which §11 also lacked |
| `TypeInfo::default_in_place` and `drop_in_place` | EG6 | D8's scratch fill; `drop_in_place` is used only by the drop-count gate, never by the glue |
| `ValueKind::Array` (offset + stride + count) | EG4, EG8 | without it `GpuTransform3D` is a hard error and gate EG8-2 cannot exist (B.8) |
| top-level `TypeKind::Enum` (a component that *is* an enum has no fields) | EG4, EG8 | `Visibility` is the dogfood target (§6.1(c)) |
| derive-time refusal of `bitset` ~~/ generics / `repr(packed)`~~, ~~**spanned at the user's type name**~~ → **spanned at the `reflect` key** | EG3 | ~~B.5: an Aether user wrote `tag Foo(bitset);` and must not see an error about a derive they never typed~~ **STRUCK 2026-08-26 (D17), on two grounds CORE MEASURED.** **CORE D34** deleted generics and `repr(packed)` from `REFUSALS` — rustc and `#[derive(Component)]` already refuse both, so those fixtures' reds could not fire; they survive only as upstream regression pins in `crates/reflect_fixture/tests/reflect_compile_fail_upstream/`. **CORE D37** superseded the caret: the Aether user cannot exist, because `reflect` is a key on the `component` construct only while `storage = "bitset"` comes from `tag` — re-verified at this audit (`grep -rn reflect crates/aether_lang/src/` returns nothing but `reflectance`). Only the **bitset** row is ECS's, and its caret is on `reflect` |
| the **B.1 fork resolved** (one `BindAccessor`+`TypeInfo` table, or two) | before EG4 | the glue reads `type_info_of` either way and is agnostic to *which* — it depends only on the resolution **existing**, because Horn 1 changes what `FieldInfo` is |
| **CORE C8's install seam** (`type_info_of` returns `Some` only because of it) | **EG8** | **ADDED 2026-08-26 (D17)** — an edge `docs/REFLECTION-PLAN-CORE.md` §7.4 item 4 states explicitly and *"neither document carried"*. §11's EG8 row did not name it either |
| **the real engine types being *annotated*** with `#[component(reflect)]` — `Transform`, `GpuTransform3D`, `EmitterActive`, `ParticleEffectHandle`, `Visibility` | **EG8, and it is on NO rung's Lands** | **ADDED 2026-08-26 (D17)**, from CORE §7.4 item 4: *"no rung in any of the four documents schedules"* it. Verified at this audit: `grep -rn 'component(reflect' crates/boyko_scene/src crates/boyko_render/src` returns **zero**, while both crates already carry the `reflect` feature (`crates/boyko_scene/Cargo.toml:52~`, `crates/boyko_render/Cargo.toml:44~`). EG8 carries it or hands it to a rung of its own; it is recorded here so it stops being invisible from the ECS side too |

**On [`docs/REFLECTION-PLAN-GATES.md`](REFLECTION-PLAN-GATES.md)** — hard:

* **TWO Miri allowlist rows, with different shapes** — `-p boyko-reflect` **plain** and
  `-p reflect-fixture --features reflect-fixture/reflect`. ~~`boyko-reflect` … with the feature ON
  (otherwise Miri compiles an empty crate and reports green)~~ is **struck**: `boyko_reflect` carries
  no `reflect` feature (GATES D4), so `--features reflect` on it is a **hard cargo error**, and with
  the feature "off" the crate is **not empty** — nothing in its source is `cfg`-gated. That sentence
  came from analysis B.9's closing line, was inherited by four plans, and would have produced a CI
  line that fails before compiling anything; B.9 is corrected at the source. Every Miri obligation
  above is vacuous until **both** rows land; EG8 gate 5 is the proof they did.
* **`crates/reflect_dogfood/`**, and the `reflect` features on `boyko_scene` / `boyko_render` that
  make it compile (analysis B.12, owner sheet B.13 #1). EG8's acceptance fixture lives there and
  **cannot** live in `reflect_fixture`, which must stay FFI-free to be Miri-able.
* The feature matrix (ON / OFF legs) that every rung's unconditional gate rides.
* The `boyko_ecs` seam (EG2) must appear in the ship build's `cargo tree` **without** pulling
  `boyko_reflect` — the seam names no reflection type, but GATES owns the assertion.
* The dead `force_alloc_panic` job (F24) is GATES' to revive or retire.

**On [`docs/REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md)** — soft, and one-directional:

* BOUNDARY **consumes** `components_of_into`, `fields_of` and `field_value`; nothing in this plan
  waits on BOUNDARY.
* One shared constraint: BOUNDARY's save key comes from `get_serialize_info(id).stable_name`
  (B.2) — this plan therefore never puts a name on `TypeInfo` and never uses `ComponentId` as a
  persisted key. `display_name` is `ComponentLayout::type_name`, which is **diagnostics only** and
  must not reach a save file.

**On the owner** — the single list is analysis **B.13**; two of its rows reach this plan:

* **B.13 #2 — EG2's shipping-API widening, FOUR items in ONE call**: `add_component_by_id`,
  `remove_component_by_id`, `mark_component_changed`, `EnableTagId::try_from_component_id`. §4's
  table is the input. The rung does not start before the answer. *(The fourth item was previously
  filed twice — here as B.11 #2 and in [`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md)
  as B-1 — with opposite recommendations. It is one decision.)*
* **B.13 #1 — may engine crates carry a `reflect` feature?** Blocks **EG8** only, and only its
  dogfood half: a "no" deletes `crates/reflect_dogfood/` and EG8 asserts the same matrix over
  `reflect_fixture`'s local shapes. Every other rung in this ladder is unaffected.

---

## 11. Rung summary

| rung | lands | blocked on |
|---|---|---|
| **EG0** | seam census + trybuild not-yet-landed list (**five** fixtures, D18) + the `doc-path-planned` decrement (**D19**) | GATES G0–G4 (the packages and the legs) |
| **EG1** | `components_of_into` sources 1+2, `IdKind`, `Refusal`, `display_name` | EG0; CORE's `type_info_of` (stub-able) |
| **EG2** | the four-item public by-id seam in `boyko_ecs` (S1, S2, S3, **S4′**) | EG0; **owner (B.13 #2)** |
| **EG3** | bitset presence, source 3, the `has_component` trap gate, the F27 mint gate | EG2 (**S4′**); **CORE C9 — LANDED** (the release `assert!` in `install_type_info`; edge added 2026-08-26, D17) |
| **EG4** | read path, refusal matrix, GPU + dynamic-tag rows | EG1; CORE's value model + `Array` + `TypeKind::Enum` |
| **EG5** | write path, change tick, release kind check | EG2 (S3), EG4 |
| **EG6** | `add_default` / `remove`, drop + unwind safety, the `NoDefault` arm | EG2 (S1, S2), CORE's `default_in_place` (**`Option`**, D20) |
| **EG7** | hook / observer / tick conformance ledger | EG5, EG6 |
| **EG8** | dogfood acceptance fixture (**in `reflect_dogfood`**) + Miri certification (**both rows**) + ~~anchor-gate registration (**and deleting GATES' two now-false statements about it**)~~ **LANDED @`eeb567be`, five documents** + **annotating the real engine types** (D17 — on no rung's Lands before this edit) | all of the above; GATES' two Miri rows and `reflect_dogfood`; **owner (B.13 #1)** for the dogfood half; **CORE C8** (D17); ~~**all four plan documents landed** (the anchor gate's path check reds on a dead sibling link)~~ **STRUCK (D19): discharged — all five are on disk and registered** |

---

## 12. Decisions taken at the EG0 audit and its verification — D16–D22

**Why these live here and not in §5.** [`docs/REFLECTION-PLAN-CORE.md`](REFLECTION-PLAN-CORE.md)
cites this document *by line* at five places — twice into §5's **D5** and three times into §10's
`install_type_info` row — and MEASURED at this audit: **the anchors census does not check a
cross-document line citation between two `.md` files.** A one-line insertion above the §10 row was
applied and the census stayed green over eight tests.
`tests/internal_docs_anchors.rs:367~` says why, structurally: *"this census scans `.md` files for
citations into `.rs`, never the reverse."* Growing §5 would therefore rot the sibling's citations
**silently**, so the new decisions go after the last cited line instead and the §5 series continues
here. *(Confirming the instrument is not blind everywhere: this very appendix reddened it four times
while being written — an over-waiver, two `Cargo.toml` lines that are not definitions, and a
misbinding created by writing a bare `:NNN` fragment next to a sibling's link.)*

---

**D16 — the EG0 census enumerates the glue's ROUTE, and “the accessors it intends to use” was not a
falsifiable list.** Two rows were wrong in opposite directions, and both were invisible because a
census of `pub` items compiles either way.

* **`register_enable_tag` comes OFF.** §4's bitset-write bullet that justified it is struck
  (“*Same trick* — FALSE, struck 2026-08-21”), EG3's route table sends **both** halves through S4′,
  and **EG3 gate 7 asserts that the glue never called it**. A census in `boyko_reflect` certifying
  the reachability of a call the glue is gated against calling is a row that argues with a gate.
  It stays a legitimate call *inside `boyko_ecs`'s own tests* — EG2 gates 8 and 9 mint with it on
  purpose — which is precisely why it belongs there and not on this list.
* **`is_enabled_id` / `enable_id` / `disable_id` go ON**, because they *are* the glue's bitset route
  (EG3's table) and were absent. ⚠️ **And the census must call them the way the glue will, or the
  row is a gate that cannot fail.** MEASURED at this audit: all three **compile and run today** from
  `boyko_reflect`'s tests, given a tag from `register_enable_tag`. What is unreachable is not the
  function — it is the **`ComponentId` → `EnableTagId` direction**, because `EnableTagId` is
  `pub(crate)`-fielded with *“no constructor”* (F16), which is exactly what **S4′** adds. A census row
  that mints its tag proves nothing about the property EG0 exists to compile; the row must reach the
  tag **from an id**, and it therefore belongs on the `compile_fail` side until EG2, not on this list.
* `register_tag` **stays**, and §7's *“a call this plan no longer makes on either citizen”* is about
  the **write path** only — EG1 gate 2 and EG3 gate 5 both mint with it in fixture setup. No conflict.
* `dense_slot_of` and `dense_get_raw` **stay**, on the corrected framing: no rung consumes them (dense
  reads go through `get_component_raw`, whose dense arm calls `dense_get_raw` *inside* `boyko_ecs`),
  but §1's **F3** asserts all three are `pub` on `EcsMaster`, and this census is where that fact is
  compiled. They are rows of the ROUTE's *substrate*, not of its callers — which is what “intends to
  use” could not express.

**D17 — four §10 dependency rows were settled, wrong, or missing, and CORE had already recorded two
of them as debts against this document.** Where CORE's completion discharges an ECS conditional, the
conditional is **struck with its reason**, not left pending on one side and done on the other.

1. `install_type_info`'s release `assert!` — **landed** at CORE C9 (`crates/boyko_reflect/src/registry.rs:87`).
   ECS's fallback clause is struck; the edge **EG3 → CORE C9** is added to §11, which lacked it.
2. The derive-refusal row — generics and `repr(packed)` are **deleted** by CORE D34 (their reds could
   not fire), and the caret is **on `reflect`**, not on the user's type name, by CORE D37 (the Aether
   user that justified ECS's caret cannot exist).
3. **`NestedCursorMut` is scheduled by no CORE rung** and EG5 gate 6 is written around it — the C7
   defect class, sitting in the table that exists to prevent it. ECS escalates before EG5.
4. Two obligations CORE names against ECS rungs and neither document's list carried: **EG8 → CORE
   C8**, and **annotating the real engine types**, which is on no rung's *Lands* anywhere.

**D18 — EG0's `compile_fail` half had a gate that could not fail and a red that could not fire, and
both were MEASURED rather than argued.**

* **Gate 2's noun drifted under the 2026-08-21 sidebar.** After S4′ moved to the positive list,
  *“negative list”* denotes **one** item while the paragraph above demands **five**. The gate now
  states the count and both kinds.
* **The glob needs a `>=` floor.** MEASURED in `boyko_reflect`: an empty glob prints *“There are no
  trybuild tests enabled yet”*, reports `running 1 test … ok`, and exits **0**. `running N` is blind
  to it. `crates/reflect_fixture/tests/reflect_compile_fail.rs:91` already carries the helper and
  calls the shape *“a VACUOUS PASS, measured”*; `crates/boyko_reflect/tests/c6_compile_fail.rs` —
  the template an implementer copies — does not.
* **RED 1 named no artifact and predicted a message no reading produces.** All three readings were
  run: deleting the fixture `.rs` exits 0, deleting the `t.compile_fail(…)` call runs nothing, and
  deleting the `.stderr` reds with **`wip`**. *“Unexpected success”* is RED 2's message — observed,
  from stubbing `add_component_by_id`.
* **RED 2's discriminating half is unreachable** and is struck: the only negative-list fixture names
  `TagId::from_component_id`, a different type and a different function, so stubbing
  `EnableTagId::try_from_component_id` can never flip it. Gate 2's count of five now carries that job.
* ⚠️ **A blessed `.stderr` here embeds `boyko_ecs`'s own source text.** Measured: rustc's
  *“there is a method `get_component` with a similar name”* suggestion reproduces `get_component`'s
  four-line signature into the fixture's expected output, computed over `EcsMaster`'s **entire**
  inherent method set. A rename or a reformat in the kernel re-blesses all four seam fixtures for a
  reason unrelated to the seam. `tests/trybuild_corpus_compiler_witness.rs` freezes the compiler, not
  the method table. This fails **loud**, so it is a maintenance cost, not a false green — recorded so
  the re-bless is expected rather than investigated.

**D19 — two obligations this rung creates that its own list did not carry, and one deferral that
points at a landed gate.**

* **The `doc-path-planned` decrement — DISCHARGED at EG0's landing.** EG0's *Lands* line carried
  the marker; `PLANNED_EXACT` asserts **equality** (`tests/internal_docs_anchors.rs:1716`),
  documented *“When a deliverable lands, its marker comes off and this number is decremented in the
  same commit.”* Without gate 5, **EG0 lands red at the root census** — the pin was `2` when this
  was written and is `1` now, with EG1's `ecs_alloc.rs` the one marker left.
* **The Miri guards.** CI runs `cargo +nightly miri test --all-targets … -p boyko-reflect`, so a new
  trybuild harness in this package is picked up and Miri cannot shell out to `cargo`. *“Miri /
  proptest. None”* read as *no obligation*. The CI comment naming the hazard names
  `reflect_fixture`'s harnesses, not this package's.
* **Gate 3's deferral to EG8 is struck.** EG8 gate 6 is itself struck as **LANDED 2026-08-21**, and
  its ordering note's reason — *“this plan links three siblings that do not exist yet”* — is false:
  all five reflection documents are on disk and in `GATED_DOCS`. Left standing, gate 3 tells an
  implementer §1 is un-gated and invites a redundant local anchor checker — the duplication C9 paid
  for. Gate 4 also fixes the **form**: parse this document at run time (precedent
  `crates/boyko_app/tests/vg_r0d_census.rs:56`), never compare two constants declared in the same
  file, and cite §1 by anchor rather than copying its rows into a `.rs` header that no census scans.

**D20 — a RED MUTATION applied and restored byte-identically is not “starting” an owner-gated rung.**
Two of EG0's reds add public items to `boyko_ecs`, which is EG2's surface, and EG2 reads *“the rung
does not start before the answer”*. The rung protocol already resolves this — the red is **OBSERVED**
and the source restored under `cmp` — but the plan did not say so at either site, so an implementer
either stalls at EG0 or breaches the owner gate to satisfy its own gate. Said at both sites now.

---

**D21 — measured while LANDING EG0, 2026-08-26. Two facts the audit could not have had, because
they only appear once the corpus exists and its reds are run.**

* ⚠️ **The seam items contaminate EACH OTHER'S blessed `.stderr`.** D18 records that a fixture's
  expected output embeds `boyko_ecs`'s own source text and warns about *kernel renames*. The
  contaminating change does not have to come from outside the seam. **MEASURED**: RED 2's stub of
  `add_component_by_id` made S1's fixture flip **and** S2's fixture **mismatch** — rustc's *“there
  is a method … with a similar name”* line in S2's diagnostic switched from `get_component` to the
  newly-reachable `add_component_by_id`, with a machine-applicable suggestion rewriting the call.
  So when EG2 lands S1, S2 and S3 in one commit, **every surviving fixture in the corpus is
  re-blessed for a reason unrelated to its own item**. Expected, loud, not a false green — but an
  EG2 implementer who re-blesses one fixture and stops will leave the target red.
* **“Exactly one fixture flips” counts FLIPS, not REDS**, and the two are different trybuild
  outcomes. A **flip** is `error` — *“Expected test case to fail to compile, but it succeeded.”* A
  **mismatch** is a red whose fixture still failed to compile, only with different prose. RED 2
  produced one flip **and one mismatch**; RED 3 produced one flip and **zero** mismatches. Counting
  reds would have reported RED 2 as *“two fixtures reacted”* and sent an implementer looking for a
  second reachable item that does not exist.

**What EG0 actually landed, and the form gates 3 + 4 left unspecified.**
`crates/boyko_reflect/tests/seam_census.rs` is **one** target carrying all three tests plus the
trybuild harness, so gate 6's *“on the trybuild harness, and on `seam_census.rs`”* is one
`#![cfg(not(miri))]`. The corpus is `crates/boyko_reflect/tests/seam_compile_fail/`, five fixtures.
Three choices are worth not re-deriving at EG2:

1. ~~**The floor is READ FROM THIS DOCUMENT, not written as a literal** — §4's four `S`-rows plus
   EG0's one refused item, parsed at run time. There is no `5` anywhere in the `.rs`.~~
   **STRUCK (D22): true as written, and worthless as a guard, because the floor did not guard the
   glob.** The corpus directory was spelled **twice** — once in the `CORPUS` constant the floor
   counts, once as a literal inside `t.compile_fail("tests/seam_compile_fail/*.rs")` — so `CORPUS`
   had exactly one reader and it was not the glob. **MEASURED at the EG0 verification, 2026-08-26:**
   mutating **only** the glob by one character, directory and fixtures untouched, left the target at
   exit **0**, `3 passed`, with `trybuild` printing *"There are no trybuild tests enabled yet"* —
   zero fixtures compiled, the `.stderr` corpus never shown to a compiler, and the correspondence
   test still green because it reads files from disk rather than from `trybuild`. What is worth not
   re-deriving is the **repaired** form: the glob is `format!("tests/{CORPUS}/*.rs")`, so the
   directory has one spelling and one reader, and the same one-character mutation now reds twice
   (the floor's `read_dir` panics, and the class gate names the site).
2. ~~**The plan-row ↔ fixture binding is the blessed `.stderr`, never a filename and never a
   comment.** Each fixture spells its call path-qualified (`EcsMaster::add_component_by_id(&mut
   ecs, …)`, not `ecs.add_component_by_id(…)`), so rustc echoes the exact `Type::fn` into the
   diagnostic and `trybuild` byte-checks it. A comment cannot satisfy the assertion; only the
   compiler having looked and not found it can. This is the answer to *“the repaired clause was
   still satisfiable by a COMMENT naming the message”*.~~
   **STRUCK (D22): the binding was `err.contains(&item.path)`, and a substring of the path is not
   that property. BOTH failure directions were MEASURED at the EG0 verification, 2026-08-26**, and
   the clause was therefore still satisfiable by exactly the thing it claimed to exclude.
   * **A comment satisfies it.** rustc renders the *entire source line* of an error span, trailing
     comments included. Replacing S3's call with
     `let _ = EcsMaster::zqq_probe(&mut ecs, entity, id); // EcsMaster::mark_component_changed` and
     re-blessing gave exit **0**, `running 3 tests`, `3 passed`, five fixtures `ok` — while the
     compiler's actual finding was ``no associated function or constant named `zqq_probe` `` and
     **nothing had looked for `mark_component_changed` at all**.
   * **Reachability satisfies it, which is the sharper direction.** With a three-argument
     `add_component_by_id` stub in the kernel, S1's fixture stopped being `E0599` and became
     `error[E0061]: this function takes 3 arguments but 4 arguments were supplied`, whose
     machine-applicable suggestion prints
     `let _ = EcsMaster::add_component_by_id(&mut ecs, entity, id);` — path-qualified, and produced
     *precisely by* reachability. `contains` matched **three** times in that one file. So a green
     census was compatible with the item being reachable, the opposite of what it certifies — and
     that `.stderr` is one an EG2 implementer would bless on the way to landing S1.
   **The repaired binding matches the sentence only an ABSENT item can produce**:
   ``error[E0599]: no … named `<fn>` found for … `<Type>` … in the current scope``, assembled from
   the `Type::fn` the plan spells. Both mutations above red against it, OBSERVED.
3. **`is_enabled_id` / `enable_id` / `disable_id` are pinned as `fn`-pointer coercions, not called**
   (D16). MEASURED that this is not decoration: swapping `is_enabled_id`'s parameter order reds the
   census with `E0308`, printing both signatures. A call would have minted a tag with
   `register_enable_tag` — the very route EG3 gate 7 forbids the glue — and proved nothing about the
   `ComponentId` → `EnableTagId` direction, which is what S4′ adds and what the corpus holds.
   *(This one survived verification unchanged; 1 and 2 did not.)*

---

**D22 — measured at the EG0 VERIFICATION, 2026-08-26. Two of D21's three "choices worth not
re-deriving" were FALSE, and both were this campaign's own named defect classes wearing the shape of
a repair.** The struck text is above with its measurement; what follows is what the class costs and
what now gates it.

* **A `.stderr` substring is not a proof of absence in EITHER direction**, and the failing direction
  that matters most is the *reachable* one, because that is the state EG2 creates on purpose. The
  binding is now the `E0599` sentence, and the two reds are recorded at
  `crates/boyko_reflect/tests/seam_census.rs`'s header rather than only here.
* **A floor guards a glob only when both name the same directory — and the decoupling was
  INHERITED, not invented.** `crates/reflect_fixture/tests/reflect_compile_fail.rs:91`, the shape
  EG0 gate 3 tells an implementer to copy, spells each of its three directories twice; C9/G5 have
  been shipping that way. `crates/boyko_reflect/tests/c6_compile_fail.rs` had **no floor at all** —
  MEASURED: emptying its corpus leaves C6 gate 5 green at exit 0. Both are repaired, and the class
  is gated once rather than per harness: `tests/trybuild_corpus_compiler_witness.rs` now resolves
  **every** `trybuild` glob in the repository (**63 call sites across 30 harnesses**, measured) and
  reds on one that matches no fixture. Its red was OBSERVED on a landed `boyko_ecs` harness, whose
  own target reported `running 1 test … ok` at exit **0** with the mutated glob.
* **`SeamItem::flips_at_eg2` was a near-dead datum.** As landed, its only readers were the parse
  floor inside its own producer and a `println!` label — the header called the two-kind distinction
  load-bearing while nothing over the corpus treated the kinds differently. It now carries two
  assertions: the kinds must name **no common item** (the two kinds are parsed from two *different*
  sections, so a collision is reachable, and it is the state EG0 and BOUNDARY's D10 were in until
  F27), and the refused item's fixture must be a **different file** from every flipping item's — the
  per-item count cannot see a single `.stderr` naming both, and D21's first bullet is the proof that
  such contamination happens. Both reds OBSERVED.
* **The dense membership count was a tautology.** `dense_ids()` is **empty** in that binary, so
  `dense_ids.iter().filter(|&&d| ecs.dense_contains(entity, d)).count() == 0` never invokes
  `dense_contains` and holds for every possible implementation of it — `x || !x` spelled as an
  iterator, under a message asserting it was not one. It is replaced by the cardinality itself
  (falsifiable, and it reds on the day EG1's subject exists), a type pin of the pair's composition
  that holds with the slice empty, and the concrete-id call that already had a known answer.

---

### EG0-audit findings recorded for the OWNER, not acted on

1. ⚠️ **The owner call EG2 is gated on is missing from the Russian owner channel.**
   `docs/OPEN-QUESTIONS.md`'s newest section (2026-08-21, *second pass*) carries the five-row **B.13**
   table, the merge of the seam into **four items in one call**, and EG2's *“blocked, does not start”*.
   `docs/ru/OPEN-QUESTIONS.md`'s newest section is the **first** pass: it states the pre-F27
   three-item framing, blocks a *“Волна 3”* that the rung vocabulary has since replaced, and contains
   **zero** occurrences of `B.13` or `EG2`. Per CLAUDE.md the English side is source of truth and the
   pair is updated in the same commit *“because a diverged pair is worse than a missing one”*.
   ~~Not repaired here: writing the owner's own channel in his language is his edit, not an
   audit's.~~ **REPAIRED at the EG0 verification, 2026-08-26 (D22).** That refusal was over-cautious
   and it read the rule backwards: `docs/ru/` holds Russian versions of the documents the owner
   reads, and the project rule is that editing either side updates the other **in the same commit**
   — declining leaves the diverged pair the rule exists to forbid, on the one section that carries
   the call EG2 is blocked on. The 2026-08-21 *second pass* section is translated in full (the
   five-row **B.13** table, row 1's engine-crate `reflect` feature, row 2's merge of the seam into
   four items in one call with EG2 *“blocked, does not start”*, and the fixed-without-you
   paragraph); nothing was summarised and nothing was invented that the English side does not have.
2. **EG2's question, stated exactly as the owner must answer it.** *May `boyko_ecs`, a shipping
   crate, gain four public by-id items for a dev-only feature — `add_component_by_id` (S1),
   `remove_component_by_id` (S2), `mark_component_changed` (S3), and
   `EnableTagId::try_from_component_id` (S4′, in `component_registry`)? Yes, no, or a named subset.*
   It blocks EG2 and through it EG3, EG5, EG6 and BOUNDARY's B4. Verified live at this audit: all
   five not-yet-reachable items (those four plus `TagId::from_component_id`) and
   `migrate_entity_attach_ids_with_bytes` return **zero** grep hits in `crates/boyko_ecs/src/`.
3. **EG0's work makes one half of that question answerable, and the answer is recorded rather than
   acted on:** the S1 addition is **mechanically unobstructed**. The stub compiled clean in
   `boyko_ecs` on the first try and the `compile_fail` fixture flipped exactly as predicted. That is
   evidence about cost, not about the values call, which remains the owner's.
4. ⚠️ **A path correction EG0 must not rediscover the hard way.** `component_registry::tags` is a
   **private** module (`mod tags;`); `EnableTagId` is reachable only through `pub use tags::*` in the
   parent, i.e. at `component_registry::EnableTagId`. MEASURED: the census probe's first compile
   failed `E0603` on the spelling EG2's *Lands* uses. EG2's *“lands in `component_registry::tags`”* is
   true of the **source file** and false of the **public path** — harmless for an inherent associated
   fn, fatal for the census's `use` line.
5. ⚠️ **Doc-rot in a sibling, reported not repaired.**
   [`REFLECTION-PLAN-BOUNDARY.md`](REFLECTION-PLAN-BOUNDARY.md):401 states *“its EG0 **landed** a
   `trybuild` `compile_fail` fixture”* — past tense for a deliverable that does not exist; *specified*
   would be true. It is the passage an implementer reads while running EG0's second RED, which exists
   to detect exactly such a fixture.
6. ⚠️ **CORE's prose about ECS's `install_type_info` row is now stale in CONTENT, not only in line
   number.** [`docs/REFLECTION-PLAN-CORE.md`](REFLECTION-PLAN-CORE.md) says at line 3575 that ECS's
   fallback is one *“which C9 accepting the item does **not** retire”*, and at line 3981 that it
   *“must be struck when ECS is next edited”*. D17 strikes it, so both sentences are **discharged**
   and belong struck at CORE's next edit — reported rather than repaired, because rewriting a
   sibling's prose from inside an audit is how the measured 75 % doc-rot repair rate happens. The
   three line citations into this document (CORE lines 1047, 3575 and 3981) were **re-derived** with
   this amendment: the `install_type_info` row moved from line 1135 to line **1203**, and §5's D5,
   the other cited anchor, did **not** move — every insertion here is below it or after §11.

### Ownership sweep (2026-08-26): does any sibling already own EG0's work?

**No.** `EG0` appears outside this document exactly twice — `REFLECTION-PLAN-BOUNDARY.md:401`
(historical narration inside D10's RESOLVED block) and `REFLECTION-PLAN-GATES.md:461` (the
campaign-order diagram). Neither builds a corpus, so the **C9/G5 duplication shape does not recur
here**. `crates/boyko_reflect/tests/` is claimed by BOUNDARY's B1 and B4 and by this rung, with no
filename collision, and BOUNDARY B4 gate 7's source census scans `crates/boyko_reflect/src/**` only,
so this rung's test-side calls do not trip it. Across the whole EG ladder only **EG2, EG3 and EG8**
are named by siblings; **EG0, EG1, EG4, EG5, EG6 and EG7 are named by nobody outside this document**,
which is where an orphaned obligation would sit unseen — D17 item 4 is one that did.
