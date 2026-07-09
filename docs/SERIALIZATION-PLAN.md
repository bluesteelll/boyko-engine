# Serialization — Research & Plan (`boyko_serialize`)

> **Status: RESEARCH + PLAN ONLY. No implementation.** Branch `ecs`, 2026-06-16.
>
> Produced by a 5-agent research workflow (R1 fast-binary architectures, R2
> reflection-vs-codegen, B1 boyko-grounding → architect → critic). The critic
> returned **CHANGES REQUESTED** with 4 critical blockers (C1–C4) and 4 important
> remarks (W1–W4); the **reflection-vs-codegen decision was explicitly APPROVED**.
> This document records the research, the architect design, and **resolves every
> blocker** (§5). The resolved design should get **one more architect→critic pass
> at implementation kickoff** before any code is written (plan-only deliverable —
> the resolutions in §5 are orchestrator decisions, not yet adversarially
> re-reviewed).
>
> **Constraints honored:** no third-party crates (std + `boyko_utils` only);
> custom binary format; zero-copy where it is actually fast; 0%-gate (serialization
> adds nothing to spawn/iter/schedule).

---

## 0. TL;DR

- **Reflection? NO.** Save/load ships as **codegen** — a per-`ComponentId` fn-ptr
  table (mirror of the landed clone subsystem) + a raw-blit fast path for POD
  columns. `boyko_reflect` stays **dev-only** and is never a dependency of the
  shipping `boyko_serialize`. This upholds the `REFLECTION-ANALYSIS.md` directional
  invariant and is independently confirmed by external evidence (bevy_replicon
  dropped reflection for codegen for exactly this reason; EnTT's snapshot system
  uses monomorphized per-type callbacks with zero runtime registry).
- **Three component classes, three mechanisms**, routed by a serialization-specific
  classification computed at derive time:
  1. **Plain-old-bytes (POB)** — `#[repr(C)]`, all fields transitively integers /
     floats / raw pointers, no `Entity` → **blit the whole column** (one `memcpy`
     of `count * stride`). The maximally fast path.
  2. **Owning** (`String`/`Vec`/heap) and **bit-restricted** (`bool`/`char`/enum/
     niche) → per-element `serialize_fn`/`deserialize_fn` (length-prefixed,
     position-independent, validates on read).
  3. **Entity-bearing** → encode path **+** a remap pass that rewrites every
     `Entity` field through a saved→new table.
- **Load default = `CopyIntoWorld`**: one `memcpy` per POD column into a freshly
  committed pool. Already "maximally fast" for a *mutable* load (bandwidth-bound).
- **True zero-copy (mmap-cast-in-place) is DEFERRED to a later phase** — it is sound
  only for an immutable snapshot world, needs a bit-pattern-validity proof on
  untrusted bytes, and an mmap-ownership/lifetime story. v1 ships the copy path,
  which is one streaming memcpy per column.
- **Stable type key = explicit `stable_name` + 64-bit hash + per-component
  `format_version`**, resolved **once per file-type** into a dense
  `Vec<ComponentId>`. Never a name probe per entity.
- **New crate** `crates/boyko_serialize/` depending on `boyko_ecs` + `boyko_utils`
  + `std` only. Additive registry changes in `boyko_ecs` (one cold parallel table
  + a name→id index), additive derive emission in `boyko_macros`.

---

## 1. The reflection question (the explicit ask)

> *"Decide whether to use the reflection we have planned for serialization, or not."*

**Decision: do NOT use reflection for the shipping save/load path. Use codegen.**

### Why codegen wins here

| Axis | Reflection (`&dyn Reflect`, field-walk, `DynamicStruct`) | Codegen (fn-ptr table + POD blit) |
|---|---|---|
| **POD path** | per-field dynamic dispatch + a `DynamicStruct` allocation per component | **one `memcpy` per whole column** — no per-row, no per-field |
| **Dispatch** | `HashMap<TypeId, …>` probe + `dyn` virtual call per field | direct `fn`-ptr per component (no `dyn`, monomorphic cursor) |
| **Runtime registry** | a reflection type-registry the shipping binary must carry | **none** beyond the existing cold `OnceLock` tables |
| **Allocations on hot save** | `Box<dyn Reflect>` / `DynamicStruct` churn | zero in the driver loop (one setup grow) |
| **Precedent in-tree** | none shipping | the **landed `Cloneability` + `clone_fn` + `CloneProbe`** subsystem (release-shipping) — this mirrors it 1:1 |

- **External confirmation (R2):** `bevy_replicon` moved *off* reflection *to* codegen
  precisely because per-field reflective serialization is the cost class that kills
  throughput. **EnTT's snapshot** system — the closest "no reflection, per-type
  codegen" analog — drives serialization through **monomorphized `operator()`
  archive callbacks per component type with zero runtime registry**; the only
  runtime table is the entity-remap map. That is exactly the boyko shape:
  `serialize`/`deserialize` fn-ptr in the registry (mirror of the clone fn-ptr) +
  a per-load entity-remap table.
- **The critic explicitly approved the mechanism:** *"Codegen-not-reflection is
  correct and well-grounded — the POD blit + per-element fn-ptr mirrors the landed
  clone subsystem 1:1 and reflection buys nothing the registry substrate doesn't
  already provide; no dismissed case where reflection would win."*

### What `boyko_reflect` is still for

Reflection remains the **dev-only** path (editor/inspector JSON, schema dumps, the
"name a field at runtime" tooling). The directional invariant from
`REFLECTION-ANALYSIS.md` holds: **shipping crates must never depend on
`boyko_reflect`.** Save/load is a shipping concern → codegen.

---

## 2. Research findings (R1 / R2 / B1)

### 2.1 Three design tiers exist; boyko should use the first two, reject the third

| Tier | Exemplars | Mechanism | Fit for boyko |
|---|---|---|---|
| **Encode-per-field** | bincode, postcard | walk fields in declaration order, emit each scalar; reader walks identically; copy per field, **never** zero-copy | **the fallback / owning-type path** — sequential, cache-friendly, but cannot beat a blit |
| **Archive-is-the-format** | **rkyv** | the in-memory representation *is* the on-disk bytes; load = aligned `&[u8]` + cast, no deserialize pass | **the technique to steal** for the POD blit + position-independent owning encoding |
| **Access-without-parse** | FlatBuffers, Cap'n Proto | random-read a buffer via offset/vtable indirection, no deserialize pass | **rejected** — trades space + per-access indirection for schema-evolution flexibility boyko does not need internally |

### 2.2 What to steal from each

- **postcard/bincode** — LE-always (matches the vast majority of CPUs → no byteswap on
  the common path); varint (LEB128) for lengths/discriminants in the owning path;
  floats are `to_bits()` → fixed LE, never varint'd. The streaming
  `Serializer`/`Deserializer`-over-a-cursor shape is what `SaveCursor`/`LoadCursor`
  becomes.
- **rkyv** — the **relative-offset / position-independent pointer**:
  `ArchivedString { ptr: i32 /*relative offset*/, len: u32 }`,
  `ArchivedVec { offset, len }`. Never write an absolute pointer; write an offset
  computed against the write-head, so the blob is position-independent and
  mmap-castable at any base. This is the owning-type encoding. **Caveats rkyv pays
  that boyko must respect:** the buffer must be aligned to the archive's max align
  to cast soundly; the format is ABI/endianness-specific unless you pay for portable
  wrappers; **safe access requires validation (bounds/alignment/valid-bit-pattern)
  or you take `unsafe`** — this is the source of critical blocker **C3**.
- **FlatBuffers/Cap'n Proto** — confirms "data structured the same in-memory and on
  the wire ⇒ no encode/decode step." The vtable indirection is the part boyko does
  **not** want (it exists for evolution + random access). Their LE-always +
  enforced-alignment choices validate ours.

### 2.3 boyko's SoA is structurally closest to **Unity DOTS**

Unity serializes by **iterating chunks and writing the raw byte data of each
column**, storing a per-type **layout hash** + an archetype/type table, then
**remapping every `Entity` field on load**. This is almost exactly the boyko-native
shape — and **strictly better** here because boyko stores one *column per
component* (`buffer_ptr() + i*stride`, stable VM base, Phase X.B), so each column is
independently blittable with **no chunk interleave**. The layout-hash guard is
adopted as the per-component `layout_fingerprint` (and hardened — blocker **C2**).
The entity-remap-on-load is adopted directly (reusing the clone subsystem's
`map_entities_fn` mechanism — and scoped/hardened in blocker **C4**).

### 2.4 The codegen-vs-reflection verdict is reinforced (see §1)

EnTT snapshot = per-type monomorphized callbacks, zero registry. bevy_replicon =
dropped reflection for codegen for throughput. The parked `REFLECTION-ANALYSIS.md`
conclusion (save/load = codegen, reflection = dev-only) is **confirmed**, not
revisited.

---

## 3. Architecture (`boyko_serialize`)

### 3.1 Decision 1 — Hybrid: raw-blit (POB) + codegen fn-ptr (owning/entity), NO reflection

Routed by a serialization-specific `Serializability` discriminant computed by the
same autoref-probe machinery as the clone `Cloneability` (but **stricter** — see
**C3 resolution** in §5):

| Category | Detection | Save | Load |
|---|---|---|---|
| **Plain-old-bytes** (`#[repr(C)]`, all fields ∈ {int,float,raw ptr}, no Entity) | autoref probe + field-type scan, `repr(C)` enforced | **blit whole column** `copy_nonoverlapping(buffer_ptr, file, count*stride)` | one bulk `memcpy` into a fresh pool (mmap-cast deferred) |
| **Owning / bit-restricted** (`String`/`Vec`; or `bool`/`char`/enum/niche) | autoref `Clone`-not-`Copy`, OR not provably all-bits-valid | per-element `serialize_fn` (length-prefixed) | per-element `deserialize_fn` into uninit row, **validates** |
| **Entity-bearing** (`ChildOf` / explicit `#[entities]`) | hand-written (`ChildOf`) or `#[entities]` opt-in | per-element `serialize_fn` (writes raw saved ids) | `deserialize_fn` + `map_entities_fn` remap pass |
| **Not serializable** (`#[component(no_serialize)]`) | attr | skip | default-construct via `REQUIRES` ctor, or reject in strict mode |

### 3.2 Decision 2 — `boyko_serialize` depends on `boyko_ecs` only, never `boyko_reflect`

The `stable_name` production + name→id resolution live in `boyko_ecs` (registry),
reachable by the shipping serializer. Pure codegen + raw-blit. (See §1.)

### 3.3 Decision 3 — Load default = bulk memcpy into a fresh pool; mmap-cast deferred

boyko pools own **mutable, VM-backed, demand-committed** storage with **co-located
ticks** (`[data | added_ticks | changed_ticks]` in one reservation). A live column
cannot point into a read-only mmap and still support `swap_remove`, structural
inserts, tick stamping, or `Mut<T>` writes. The bulk memcpy is still **one
streaming memcpy per column** (no per-row/per-field) — bandwidth-bound, within
~1.2× of an mmap-cast for cold loads, and composes with the parked fast-load memory
suite (`world.reserve(archetype, n)` pre-commits pages, then the load memcpy fills
them with no page-fault stalls). **mmap-cast-in-place is deferred** (see §5 C3 and §6).

### 3.4 Decision 4 — Fold serialization into `#[derive(Component)]`

No separate `#[derive(Serialize)]`. The classification (Copy / Clone / has-entity /
bit-validity) is computed once by the existing derive. Opt-out via
`#[component(no_serialize)]` (mirrors `#[component(no_clone)]`). One extra
`install_serialize_fn::<C>(id)` in the existing `component_id()` closure (ungated,
like `install_clone_fn`).

### 3.5 Decision 5 — Stable type key (name-hash + version), resolved once per load

The file keys each component by an explicit `stable_name` (default = fully-qualified
type name, overridable via `#[component(stable_name = "...")]`) + its 64-bit hash +
a per-component `format_version: u16`. On load, each file-local type resolves
**once** to the running world's `ComponentId` via a new name→id index, materialized
into a dense `Vec<ComponentId>`. The per-entity loop is then pure id-indexed byte
copies — never a name probe per component per entity. (`ComponentId` is
first-touch-order process-unstable → keying by id or positional enumeration breaks;
keying by stable name is the only stable option. The index does **not** exist today
— blocker **C1** specifies it from scratch.)

### 3.6 Decision 6 — `layout_fingerprint` (blit-validity gate)

A derive-computed `const LAYOUT_FINGERPRINT: u64` guards "the struct changed shape
since the save." A component is blittable only if `#[repr(C)]`/`#[repr(transparent)]`
(`#[repr(Rust)]` blittable types are a derive-time compile error). The precise
fingerprint formula is hardened in blocker **C2 resolution** (§5) —
`offset_of!`-based, no field-type-name dependency.

### 3.7 Data structures (registry additions, mirroring `CLONE`)

```rust
// boyko_ecs::component_registry (NEW, parallel to CloneFn/CloneInfo/Cloneability)

/// Serialize one instance: read live value at `src`, append position-independent
/// bytes into `sink`. Installed ONLY for the encode path; POB installs None.
/// # Safety: `src` = live aligned initialized C; `sink` append-only.
pub type SerializeFn = unsafe fn(src: *const u8, sink: &mut SaveCursor<'_>);

/// Deserialize one instance from `src` into UNINIT `dst` (one ptr::write, no drop
/// of prior). Returns Err on a malformed stream (caller rolls back; dst left uninit
/// — the W5 partial-row contract, mirrors CloneFn). Entity fields written as SAVED
/// ids; the separate map_entities pass remaps them.
/// # Safety: `dst` = writable uninit >= size_of, aligned.
pub type DeserializeFn = unsafe fn(src: &mut LoadCursor<'_>, dst: *mut u8)
    -> Result<(), DecodeError>;

/// Load-direction entity remap (saved id -> new Entity).
pub type LoadMapEntitiesFn = unsafe fn(dst: *mut u8, map: &LoadEntityMap);

#[repr(u8)]
pub enum Serializability { PlainOldBytes = 0, SerializeViaFn = 1, Ignore = 2 }

#[repr(C)] #[derive(Clone, Copy)]
pub struct SerializeInfo {
    pub serialize_fn:       Option<SerializeFn>,       // Some only for ViaFn
    pub deserialize_fn:     Option<DeserializeFn>,     // Some only for ViaFn
    pub map_entities_fn:    Option<LoadMapEntitiesFn>, // Some only for entity-bearing
    pub serializability:    Serializability,
    pub format_version:     u16,
    pub layout_fingerprint: u64,
    pub stable_name:        &'static str,
    pub stable_name_hash:   u64,
}

// Cold parallel table (mirror CLONE). Read ONLY from boyko_serialize.
static SERIALIZE: [OnceLock<SerializeInfo>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

#[inline] pub fn get_serialize_info(id: usize) -> Option<&'static SerializeInfo>;
#[inline] pub fn install_serialize_fn<C: Component>(id: usize); // ungated, from component_id()
```

Plus a **new** name→id index (blocker **C1**, specified in §5).

### 3.8 Cursors (the single `dyn`-free I/O boundary)

Concrete monomorphic `SaveCursor<'a>` (append-only over a preallocated `Vec<u8>`,
tracks `base_pos` for relative offsets) and `LoadCursor<'a>` (bounds-checked reads
over the file bytes). **Zero `dyn`** — the fn-ptr table is uniform
(`SerializeFn = unsafe fn(*const u8, &mut SaveCursor)`). This resolves the R2
"one `dyn` vs monomorphized writer" open question toward zero-dyn (the critic
endorsed this for a single-format engine). Every `LoadCursor` read validates `pos`
against `bytes.len()` — **validate, never transmute blindly.**

### 3.9 File format

`#[repr(C)]` fixed 64 B `SaveHeader { magic b"BOYKOSAV", format_version, endianness,
ptr_width, flags, *_off offsets, type_count, archetype_count, entity_count }`,
followed by a `TypeTableEntry[]` (per distinct component: `stable_name_hash`,
`layout_fingerprint`, `size`, `align`, `format_version`, `serializability`,
name-pool offset/len), `ArchetypeBlock[]` (component-count, entity-count, file-local
type indices, then per-column `ColumnRegion { data_off, byte_len }`, then entity
rows), a saved entity table, and a trailing `var_data` region (owning heap bytes
referenced by position-independent relative `i64` offsets — rkyv technique; `i64`
avoids the 2 GiB `i32` overflow). Blittable column regions are laid at
`max(SIMD_BUFFER_ALIGN, align)`-aligned offsets. **v1 rejects on endianness/ptr_width
mismatch** (no byteswap path — O2).

### 3.10 Public API

```rust
pub struct SaveOptions { pub persist_ticks: bool, pub include_filter: Option<fn(ComponentId)->bool> }
pub enum LoadEntityPolicy { Remap }   // PreserveIds deferred (W2)

pub fn save_world(world: &EcsMaster, opts: &SaveOptions, out: &mut Vec<u8>) -> Result<usize, SaveError>;
pub fn save_world_to_file(world: &EcsMaster, opts: &SaveOptions, path: &Path) -> Result<usize, SaveError>;
pub fn load_world(world: &mut EcsMaster, bytes: &[u8], policy: LoadEntityPolicy) -> Result<LoadReport, LoadError>;
pub fn load_world_from_file(world: &mut EcsMaster, path: &Path, policy: LoadEntityPolicy) -> Result<LoadReport, LoadError>;

pub struct LoadReport { pub entities_loaded: u64, pub archetypes_loaded: u32,
    pub columns_blitted: u32, pub columns_decoded: u32, pub types_skipped: u32, pub types_defaulted: u32 }
```

(`LoadMode::MmapInPlace` is **deferred** — see §6.)

### 3.11 Algorithms (critical paths)

**SAVE — two-pass (W3 resolution):**
1. **Pass 1 (size):** walk archetypes read-only, sum exact byte sizes per column
   (POB = `count*stride`; owning = run each `serialize_fn` against a sizing sink, or
   a size-estimate then exact in pass 2). Compute all file offsets. **One** grow of
   `out` to the exact total — no realloc mid-write.
2. **Pass 2 (fill):** for each column: POB → one `copy_nonoverlapping`; ViaFn → loop
   rows calling `serialize_fn(row_ptr(i), &mut SaveCursor)`. Emit type table + name
   pool. Backpatch header offsets. Exact offsets also enable the §3.12 parallel save.

Complexity: O(POD bytes) (one memcpy each) + O(owning heap bytes). Sequential
streaming write throughout; one `serializability` branch per *column*, not per row.

**LOAD (`CopyIntoWorld` + `Remap`) — the default:**
1. **Validate header** (release check, not debug_assert): magic, supported
   `format_version`, `endianness == native`, `ptr_width == 8`. Reject loudly.
2. **Resolve the type table once:** `resolve_stable_name(hash, name)` → running
   `ComponentId`; build a dense `Vec<ComponentId>`. Per component: compare
   `layout_fingerprint`; on mismatch for a POB column → **hard error** unless a
   `deserialize_fn` exists for that `format_version` (C2). Build-absent type → skip
   (or strict error).
3. **Pre-size entities:** `ensure_capacity(total)` + `reserve_batch(total)` once.
   Build the `LoadEntityMap` (saved `EntityId.0` → fresh `Entity`).
4. **Per archetype (always freshly created — start_row==0, W4):** resolve
   component-id set → `create_archetype`; `reserve_capacity(entity_count)`;
   `register_batch(...)`. Per column: POB+fingerprint-OK → one
   `copy_nonoverlapping(file_region, buffer_ptr_mut().add(0), entity_count*stride)`
   then `commit_units`; ViaFn → loop rows, `deserialize_fn(&mut LoadCursor,
   reserved_row_ptr(i))` into uninit, **rollback on first Err** (drop committed rows
   via the pool `drop_fn`, free the batch entities); skipped → default-construct via
   `REQUIRES` ctor. `fill_ticks(0, entity_count, current_tick)` (v1 resets ticks).
5. **Entity-remap pass:** for each column with `map_entities_fn.is_some()`, loop
   rows, `map_entities_fn(row_ptr(i), &LoadEntityMap)`. **Snapshot worklists by
   value; no cached `NonNull<EcsMaster>` across a structural op** (the 14a-F2 /
   Phase-19 TB discipline). An unmapped saved id → **release error** (C4).
6. Return `LoadReport`.

### 3.12 Multithreading

- **Save:** read-only `&EcsMaster`, naturally parallel per-archetype (disjoint output
  regions, offsets fixed by pass 1) via `boyko_threadpool` Scope — **v1
  single-threaded** (correctness first), parallelism is Phase S5.
- **Load:** `&mut EcsMaster`, single-threaded in v1 (entity alloc + archetype create
  touch shared state). Per-column memcpy *within* a reserved archetype is
  parallelizable later (disjoint regions).
- Registry tables (`SERIALIZE`, name index): write-once `OnceLock` / `Mutex`-guarded,
  touched only at registration (setup) + load resolve (once per type) — never a
  frame path. `SerializeInfo` is `Copy + Send + Sync` (fn-ptrs + POD).

### 3.13 Integration (minimal, additive)

- `component_registry.rs` — `SerializeFn`/`DeserializeFn`/`LoadMapEntitiesFn`/
  `Serializability`/`SerializeInfo`/`SERIALIZE`/`get_serialize_info`/
  `install_serialize_fn`, the `SerializeProbe` arms, **and the new name→id index**
  (C1). ~280 lines, all cold, mirrors the `CLONE` block.
- `boyko_macros/src/lib.rs` — emit `stable_name`, `layout_fingerprint`
  (`offset_of!`-based), `format_version`, the `SerializeProbe` invocation,
  `install_serialize_fn::<C>(id)`; enforce `#[repr(C/transparent)]` for blittable;
  honor `#[component(no_serialize)]` / `stable_name=".."` / `#[entities]`. ~180 lines.
- `component_pool.rs` — `blit_column_unchecked(start_row, &[u8])` (whole-slice
  memcpy variant of `write_at_unchecked_initialized`; W4 pins start_row==0). ~30 lines.
- `entity_master.rs` — none for v1 (`Remap` reuses existing
  `ensure_capacity`/`reserve_batch`/`register_batch`; the generation-preserving API
  is deferred with `PreserveIds`, W2).
- New crate `crates/boyko_serialize/`: `lib.rs`, `format.rs`, `save.rs`, `load.rs`,
  `cursor.rs`, `entity_map.rs`, `error.rs`. (`mmap.rs` deferred.) Reuses the clone
  subsystem's `materialize.rs` grow→write→commit→fill template and `map.rs`
  `EntityCloneMap` (`SparseMap<Entity>`) as the `LoadEntityMap` template.

---

## 4. The reflection decision, restated for the record

Already covered in §1. **Codegen. `boyko_reflect` stays dev-only and is never a
`boyko_serialize` dependency.** This is the headline answer to the user's question
and is the one design point the critic approved outright.

---

## 5. Critic blockers — RESOLUTIONS

> The critic returned **CHANGES REQUESTED**. These are the orchestrator-level
> resolutions of every blocker (the critic enumerated concrete option menus). They
> must be re-validated by one architect→critic pass before coding.

### C1 — The name→id index does NOT exist; specify it from scratch (not "mirror TAG_NAMES")

`TAG_NAMES` interns only *dynamic-tag* names; derived components are keyed by
`TypeId` inside the 56 B `ComponentLayout` hot record, with **no** name→id reverse
index. **Resolution — build a NEW dedicated index:**

```rust
// Keyed by stable-name HASH (never TypeId — two builds of "the same" component
// differ in TypeId but must resolve to the same stable name). Collisions
// disambiguated by comparing the full stable_name string.
static STABLE_NAME_INDEX: OnceLock<Mutex<HashMap<u64 /*hash*/, SmallVec<usize /*ComponentId*/>>>>
    = OnceLock::new();

/// Registered in the `component_id()` closure, UNGATED (like install_clone_fn),
/// once per component per process. Cold — never a frame path.
pub fn register_stable_name<C: Component>(id: usize);
/// Load-time, ONCE per file-local type. Returns the running ComponentId or None.
pub fn resolve_stable_name(hash: u64, name: &str) -> Option<usize>;
```

- Registration happens at `install_serialize_fn` time (same closure). The index maps
  `stable_name_hash → candidate ComponentIds`; on a hash hit it compares the full
  `stable_name` to confirm (collision → distinct types).
- **0%-gate proof obligation:** `STABLE_NAME_INDEX` is read only inside
  `resolve_stable_name`, called only from `boyko_serialize::load_world` — grep-proof
  that no spawn/iter/schedule path touches it.
- The `ComponentLayout` 56 B pin is untouched (TRIPWIRE 2) — the index is a separate
  static, the `stable_name` lives in `SerializeInfo` (the cold parallel table), not
  inline.

### C2 — `layout_fingerprint` must be derivable and not type-name-fragile

A proc-macro **cannot** see a field's *stable* type name (only token text — the same
brittle substring `struct_has_entity_field` uses). **Resolution — option (ii)+(iii):**

- `LAYOUT_FINGERPRINT = hash(size_of, align_of, repr_kind, [offset_of!(C, f) for each field f], field_count)`.
  - `offset_of!` is derivable, requires no type-name knowledge, and **catches a
    same-size field reorder/swap** (two `u32`s swapped change their offsets only if
    there is padding between — so also include `field_count` and the per-field
    `size_of` of each field type *as seen at the field*, which the macro gets via a
    helper const `size_of_val`-style trick on a zeroed instance... **NOTE:** if
    `offset_of!` + per-field size is not sufficient to catch a pure same-size swap of
    two identical-type fields, that swap is **semantically invisible to layout** and
    therefore safe to blit — the bytes are interchangeable. Document this explicitly.)
  - **`#[repr(C)]`/`#[repr(transparent)]` required for blittable** (derive-time
    compile error otherwise) — removes the "compiler reordered fields" risk entirely.
- **Fingerprint is a best-effort *guard*, not a correctness guarantee** — the
  human-facing version gate is `format_version` (the user bumps it on any
  intentional layout/semantic change).
- **On-mismatch behavior for a POB column with NO decode fn → HARD ERROR** (loud,
  release). Never a silent default, never a silent garbage blit. (If a
  `deserialize_fn` exists for the file's `format_version`, demote to decode.)

### C3 — mmap-cast bit-pattern validity + lifetime → narrow the blit class AND defer mmap

The critic is right that serialize ingests *untrusted bytes* — a categorically
different obligation than clone (which copies a *live, already-valid* source). `Copy`
does NOT imply all-bit-patterns-valid (`bool`, `char`, fieldless enum, niche types
like `NonZeroU32`/`Option<NonZeroU32>` have invalid bit patterns → instantiating one
from a corrupt byte is instant UB). **Resolution:**

1. **Introduce a STRICTER blittable class `PlainOldBytes` (POB)** — distinct from the
   clone `TriviallyCopyable`. A type is POB iff `#[repr(C/transparent)]` **and** every
   field is transitively in `{integers, floats, raw pointers}` — **no `bool`, no
   `char`, no enum, no niche-optimized type, no `Entity`.** The derive's
   `SerializeProbe` proves this (a new probe arm; it is NOT a verbatim `Cloneability`
   mirror — clone never needed bit-validity). Any `Copy` type that is *not* provably
   POB (because it contains a `bool`/`char`/enum/niche) falls to the **`SerializeViaFn`
   decode path**, whose `deserialize_fn` validates each such field on read (e.g. a
   `bool` field reads a byte and checks `0|1`, an enum reads + matches the
   discriminant). This makes the copy path sound on untrusted bytes too — not just
   the mmap path.
2. **Defer mmap-cast-in-place (`MmapInPlace`) out of v1.** It additionally needs the
   `Mmap` ownership/lifetime story (who owns the mapping, where it is stored to
   outlive every adopting pool, unmap-vs-pool-teardown order — a Miri-TB obligation)
   and the read-only-world flag. v1 ships **only `CopyIntoWorld`** (one streaming
   memcpy per POB column — already bandwidth-bound and "maximally fast" for a mutable
   load). When mmap is built (a later phase, §6), the validity gate additionally
   requires: POB class, fingerprint match, endianness/ptr_width match, region aligned
   to `align_of::<C>()`, the mapping owned by a `'static`-lived holder on the world,
   and the world flagged read-only.

### C4 — Entity-remap scope must be decided and missed refs must be LOUD

`struct_has_entity_field` is a substring scan for `"Entity"`/`"ChildOf"` — under
save/load a missed field is **silent corruption** (written with its saved id, never
remapped → points at an arbitrary/never-created entity on load). **Resolution —
option (a), the clone-v1 boundary + loudness:**

- **v1 entity-remap scope = `ChildOf` only**, via a **hand-written** `map_entities_fn`
  (matches the clone v1 boundary exactly — `clone/map.rs`).
- **Any OTHER component the scan flags as entity-bearing is a HARD DERIVE ERROR**
  unless the user explicitly opts in with a **`#[entities]` field attribute** (so a
  user with `struct Foo { target: Entity }` is *forced* to write
  `#[entities] target: Entity` — never silently un-remapped). The `#[entities]`
  attribute is the sound, explicit replacement for the heuristic substring scan; the
  derive emits the remap for exactly the annotated fields. (Full auto-emit for
  `Option<Entity>`/`[Entity;N]`/`Vec<Entity>` is a post-v1 extension built on
  `#[entities]`, not the scan.)
- **Unmapped-saved-id policy → release ERROR** (not a debug-only assert): a saved id
  referenced by a remap with no entry in the `LoadEntityMap` fails the load loudly.
  (Cross-save / external refs are a documented non-goal for v1; a future "external
  ref" policy can remap-to-a-sentinel-dead-entity, explicitly opted into.)

### W1 — Load-time registration of build-present-but-never-touched components

`load_world` has bytes, not the type `C`, so it cannot `register_new::<C>()` a type
the process has never instantiated. **Resolution — v1 documents the contract +
provides a force-register helper:**

- v1 resolves only against **already-registered** ids. Document: *"every serializable
  component must be registered (touched once) before `load_world`."*
- Provide `boyko_serialize::register::<C>()` (calls `C::component_id()` to force
  registration without spawning) and a convenience `register_all!(A, B, C, …)` macro
  for the app to call once at startup. (A fully automatic startup sweep would need a
  linker-section registry like the `inventory` crate — **forbidden** by the
  no-third-party rule — so the explicit helper is the v1 answer.)
- A file type that resolves to no registered id → `types_skipped` (lenient default) or
  strict error.

### W2 — `PreserveIds` + free-list interaction → DEFER from v1

Restoring exact saved `(id, generation)` bypasses the allocator's free-list
invariants; sparse saved ids need slot-array sizing + intervening-slot free-marking
or a later `allocate_entity` collides with a restored id. **Resolution — ship only
`Remap` in v1** (the stated normal case). `PreserveIds` (replay/debug) is a later
phase that will specify free-list/sparse-id reconstruction and the
generation-preserving `EntityMaster` API + restrict to load-into-empty-world.

### W3 — Save realloc vs streaming → TWO-PASS save

Resolved in §3.11 (Pass 1 sizes exactly incl. owning, Pass 2 fills) — no realloc
mid-write, and the exact offsets enable the disjoint-region parallel save (S5).

### W4 — `blit_column_unchecked` stride-vs-size → load into fresh archetypes only

Resolved in §3.11 step 4: **load always targets a freshly-created archetype
(`start_row == 0`)**, so the whole-column blit writes `buffer_ptr_mut().add(0)` for
`entity_count*stride` bytes within the contiguous data sub-region, never overrunning
into the co-located tick sub-regions. `debug_assert!(bytes.len() == count*stride)` +
`debug_assert!(start_row == 0)` in `blit_column_unchecked`.

### O1 / O2 (non-blocking)

- **O1:** drop the "`SerializeInfo` is exactly 64 B / one cache line" claim (it is
  cold; exact size is not load-bearing). No `const_assert` needed (contrast
  `CloneInfo`'s asserted 16 B).
- **O2:** v1 **rejects** on endianness/ptr_width mismatch — **no byteswap path** (a
  per-field byteswap would need field-layout info the codegen path deliberately
  avoids). Same-target is the realistic v1 scope.

---

## 6. v1 scope vs deferred

| In v1 | Deferred (later phase) |
|---|---|
| POB blit (one memcpy/column), owning + bit-restricted decode path | **mmap-cast-in-place** (`MmapInPlace`) — needs bit-validity-on-cast + mmap lifetime (C3) |
| `CopyIntoWorld` load, `Remap` entity policy | **`PreserveIds`** entity policy (free-list reconstruction, W2) |
| `ChildOf` remap + `#[entities]` explicit opt-in (hard error otherwise) | full auto-emit for `Option<Entity>`/`[Entity;N]`/`Vec<Entity>` (built on `#[entities]`) |
| single-threaded save/load | **parallel** per-archetype save / per-column load (disjoint regions) |
| reject on endianness/ptr_width mismatch | cross-endian byteswap (likely never — same-target scope) |
| explicit `register::<C>()` / `register_all!` | automatic startup registration (blocked by no-third-party) |

---

## 7. Implementation phases (for the eventual developer)

- **S0 — registry + derive substrate (no I/O):** `Serializability` (POB/ViaFn/Ignore),
  `SerializeInfo`, the fn-ptr types, `SERIALIZE` table, `get/install_serialize_fn`,
  the **stricter `SerializeProbe`** (POB bit-validity arm — C3), the
  **`STABLE_NAME_INDEX` + `resolve_stable_name`** (C1), derive emission of
  `stable_name`/`layout_fingerprint`(`offset_of!`, C2)/`format_version`,
  `#[repr(C)]` enforcement, `#[component(no_serialize)]`/`stable_name`/`#[entities]`
  (C4). Tests: classification matrix (POB vs ViaFn vs Ignore, incl. bool/char/enum →
  ViaFn), fingerprint reorder-detection, `#[repr(Rust)]`-blittable compile-fail,
  name resolution + collision.
- **S1 — format + cursors + two-pass save:** `format.rs` (`#[repr(C)]` types,
  header const-asserts), `cursor.rs` (`SaveCursor`/`LoadCursor`, bounds-checked),
  `save.rs` (two-pass, W3). Tests: round-trip-inspect a POB-only world; an owning
  component.
- **S2 — load (`CopyIntoWorld` + `Remap`):** `entity_map.rs`, `blit_column_unchecked`
  (W4), `load.rs` (header validate, type resolve, fresh-archetype blit/decode,
  remap pass, partial-load rollback). Tests: POB; owning; `ChildOf` remap;
  skip/default tolerance; truncated-stream rollback; **unmapped-id → loud error**
  (C4); fingerprint-mismatch → hard error (C2).
- **S3 — versioning + robustness:** `format_version` tolerance (skip-absent /
  default-present / mismatch→decode), property tests (`save(load(save(w))) ==
  save(w)`), **fuzz the loader** (malformed/hostile bytes never UB — bounds-checked
  cursor + validate-not-transmute; this is where the C3 bit-validity decode path
  earns its keep).
- **S4 (deferred) — mmap + `MmapInPlace`:** platform mmap (`vm.rs`/Phase X.C style +
  `cfg(miri)` read-into-Vec fallback), the validity gate, mmap ownership/lifetime,
  read-only-world flag, all-or-nothing per-world fallback.
- **S5 (deferred) — parallel save/load.**

---

## 8. Validation (when implemented)

- **0%-gate:** `spawn_100k` / `query_iter` / `schedule_50_systems` byte-identical vs
  pre-feature; grep-proof every `get_serialize_info` / `resolve_stable_name` /
  `STABLE_NAME_INDEX` reader ⊆ `boyko_serialize`.
- **Benches (criterion):** `save_pob_100k` / `load_pob_100k` (assert one memcpy per
  column, not just time), `save_owning_10k` / `load_owning_10k`.
- **Unit tests:** classification + `#[repr(C)]` compile-fail; round-trip POB / owning
  / entity-bearing (`ChildOf`) / mixed / empty / MAX / ZST-tag; entity-remap
  correctness (a `ChildOf` chain survives with rewritten ids); versioning
  (skip/default/mismatch/endianness/ptr_width reject); rollback (truncated, malformed
  length prefix, fingerprint mismatch w/o decode → world unchanged, no leak, no
  double-free).
- **Property:** idempotent bytes; fuzz `load` on random bytes → never UB, always
  `Err` or a valid world (the C3 obligation).
- **Miri (Tree-Borrows) — the soundness gate:** deserialize-into-uninit, the remap
  pass mutating while walking (14a-F2 / Phase-19 class), the `cfg(miri)` fallback arm.

---

## 9. Sources

- **rkyv** — archive-is-the-format, relative pointers: David Koloski's rkyv
  architecture notes; `rkyv::rel_ptr` docs; rkyv GitHub.
- **postcard / bincode** — the Postcard wire spec (varint, LE-always, float-to_bits);
  postcard 1.0 notes.
- **FlatBuffers / Cap'n Proto** — vtable/offset access-without-parse; LE-always +
  enforced alignment.
- **Unity DOTS** — chunk byte-dump + per-type layout hash + entity-remap-on-load.
- **EnTT snapshot** — per-type monomorphized archive callbacks, zero runtime registry
  (the codegen analog).
- **bevy_replicon** — dropped reflection for codegen for throughput (the reflection
  verdict's external confirmation).
- **In-tree precedent** — the landed clone subsystem (`Cloneability` + `clone_fn` +
  `CloneProbe`, `crates/boyko_ecs/src/ecs/core/clone/`), `docs/REFLECTION-ANALYSIS.md`
  (the codegen-not-reflection directional invariant), `docs/CLONING-PLAN.md`.
