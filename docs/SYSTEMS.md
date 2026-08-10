# boyko-engine systems catalog (branch `ecs`)

Reference for every subsystem on the `ecs` branch — code locations,
key types, methods, and invariants. Agents read this for navigation.
For "where to find X", start in [FEATURE_MAP.md](FEATURE_MAP.md);
for cross-crate architecture, see [ARCHITECTURE.md](ARCHITECTURE.md).

**Status legend:**
- ✅ Implemented and tested
- ⚠️ Implemented with documented caveats
- 📋 Planned
- ❌ Not implemented (deliberate)

> This document reflects the cumulative state of Phases 2 → 18 + the 9.x
> executor-soundness series + the X.x perf series + Phases 14a/14b. The
> authoritative record for any single phase is its `docs/PHASE-*-RESULTS.md`.

> **Anchors are partly gated, and the boundary is stated because it moved four
> times.** `tests/internal_docs_anchors.rs` runs under the ordinary
> `cargo test --workspace` and checks **exactly two notations**: the suffix form
> `file.rs:N` (including `(:N)`) and the bare `(N)` member line. For those: the
> path must exist, line N must still hold a definition, and where a line's
> backticked symbols pair one-to-one with its numbers, line N must also name the
> symbol it stands beside. An anchor written `:N~` / `(N~)` deliberately points
> at a non-definition — a struct field, an enforcement site, a module-doc
> invariant — and waives **both** the shape and the identity check, keeping only
> the in-file bounds check.
>
> **NOT read by the gate:** line numbers spelled `line N` or `(line N)`. Nine
> such sites exist across these documents. That form is invisible — changing one
> to `(line 99999)` leaves the suite green and does not even move the anchor
> count. Write new citations in the `:N` form.
>
> **Binding rule, stated exactly because getting it wrong is what caused the
> worst rot found here:** an anchor binds to the nearest resolvable file-shaped
> path mention since the last heading — a **File:** header, **but also any inline
> markdown link or bare `crates/...` mention.** So an inline file link dropped
> into a member table silently rebinds every row after it, and a table whose
> members live in several files needs its header split.
>
> Do not read this box as a freshness guarantee for the whole document. It states
> which notations are machine-checked; a number in any other form is unverified.

> **Crates (19 members).** *Kernel:* `boyko_ecs` (core) · `boyko_macros`
> (proc-macros) · `boyko_utils` (collections) · `boyko_threadpool` (Chase-Lev
> work-stealing pool over crossbeam-deque). *Std-lib / sim:* `boyko_math` ·
> `boyko_scene` · `boyko_sdf_math` · `boyko_physics` · `boyko_input` ·
> `boyko_serialize`. *Render / UI / shaders:* `boyko_rhi` · `boyko_rhi_vulkan` ·
> `boyko_render` · `boyko_shaderdsl` · `boyko_fontbake` · `boyko_image` ·
> `boyko_ui`. *Host /
> apps / bench:* `boyko_app` (the windowed host — `EnginePlugins`, the
> device-singleton boot, the token-fenced G-buffer runner + `GpuSceneBundles`,
> ECS-owned lighting + CSM arming via the D5 light-table generation gate;
> host plan R2/R3/R4, [APP-HOST-PLAN.md](APP-HOST-PLAN.md)) · `boyko_demo`
> (wgpu+egui sandbox) · `bench_bevy_vs_boyko`. Sections 1–21 catalog the ECS
> kernel; sections 22–33 catalog the std-lib and render/UI crates.

---

## Table of contents

1. [Identifiers](#1-identifiers-id-types-)
2. [Memory subsystem](#2-memory-subsystem-) — VmReservation (per-OS reserve/commit), ComponentPool (type-erased, self-growing, tick sub-regions); Arena + MemFreeBlockMaster DELETED (X.J)
3. [Component subsystem](#3-component-subsystem-) — trait, mask, registry, pool bundle, **§3.6 hooks & observers**, **§3.7 tags (static ZST + dynamic) + empty archetype**, **§3.8 EnableTag (enable-bit, non-fragmenting tag backend)**
4. [Entity subsystem](#4-entity-subsystem-) — Entity, EntityInland (slab ptr), EntityMaster
5. [Archetype subsystem](#5-archetype-subsystem-) — Archetype (inline columns), signature, registry, bundle slab, master
6. [EcsMaster facade](#6-ecsmaster-top-level-facade-) — incl. **§6.1 multi-world model** (WorldId, schedule binding, hooks-global/observers-per-world)
7. [Bundle subsystem](#7-bundle-subsystem-)
8. [Query subsystem](#8-query-subsystem-) — typed `Query<D, F>` DSL, filters, par_iter, for_each_chunk, LegacyQuery, **§8.6 dynamic tag terms + the `_pre_terms` funnel**
9. [SystemParam + Resources + IntoSystem](#9-systemparam--resources--intosystem-)
10. [Commands](#10-commands--deferred-mutation-)
11. [Schedule + parallel scheduler](#11-schedule--parallel-scheduler-) — incl. ordering, sets, run conditions
12. [Change detection](#12-change-detection-)
13. [States](#13-states-)
14. [Events](#14-events-)
15. [App + Plugin facade](#15-app--plugin-facade-)
16. [Error handling](#16-error-handling-)
17. [boyko_utils](#17-boyko_utils-)
18. [boyko_threadpool](#18-boyko_threadpool-)
19. [Macros](#19-derive--attribute-macros-)
20. [Constants](#20-constants-)
21. [boyko_demo](#21-boyko_demo-)

**Std-lib / simulation crates**

22. [boyko_math](#22-boyko_math-) — SIMD-aligned POD math vocabulary
23. [boyko_scene](#23-boyko_scene-) — Transform / GlobalTransform / Camera / propagation
24. [boyko_sdf_math](#24-boyko_sdf_math-) — analytic SDF edit-list field leaf
25. [boyko_physics](#25-boyko_physics-) — in-house 3D TGS-Soft solver
26. [boyko_input](#26-boyko_input-) — source-agnostic rebindable action mapping
27. [boyko_serialize](#27-boyko_serialize-) — custom binary world save/load

**Render / UI / shader crates**

28. [boyko_rhi](#28-boyko_rhi-) — backend-agnostic RHI trait surface
29. [boyko_rhi_vulkan](#29-boyko_rhi_vulkan-) — raw-FFI Vulkan backend + framegraph
30. [boyko_render](#30-boyko_render-) — GPU-resident columns, lighting, shadows, SDF
31. [boyko_shaderdsl](#31-boyko_shaderdsl-) — in-house Rust shader eDSL
32. [boyko_fontbake](#32-boyko_fontbake-) — load-time MTSDF font baker
32b. [boyko_image](#32b-boyko_image-) — in-house PNG/zlib/DEFLATE decoder (leaf, load-time)
33. [boyko_ui](#33-boyko_ui-) — ECS-native UI

---

## 1. Identifiers (ID types) ✅

**Files:**
- [crates/boyko_ecs/src/ecs/identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs)

Core ID types in `boyko_ecs` are strongly-typed `#[repr(transparent)]` newtypes
(zero runtime cost; the compiler refuses to mix them — audit C-017 closed the
historical "all-`usize`-aliases" hole), generated by one `define_id!` macro:
`EntityId`, `ArchetypeId`, `ComponentId`, `InlandPoolId`, and
siblings. Each derives `Debug + Default + Clone + Copy + PartialEq + Eq + Hash +
PartialOrd + Ord`, has `const fn new` / `const fn get`, `From<usize>` +
`From<Self> for usize`, and a hand-rolled `Display`.

`Generation = usize` stays an alias (only ever paired with `EntityId` inside
`Entity`).

Subsystem-local dense-table sizing newtypes live next to their owners, NOT in
`primitives.rs`:
- `ResourceId` — [resources/resource_registry.rs](../crates/boyko_ecs/src/ecs/core/resources/resource_registry.rs)
- `BundleTypeId` — [bundle/bundle_type_registry.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle_type_registry.rs)
- `QueryTypeId` — [iters/query/query_type_registry.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_type_registry.rs):76
- `ObserverId` — [component/observers/mod.rs](../crates/boyko_ecs/src/ecs/core/component/observers/mod.rs):59
- `SystemSetId` — [schedule/system_set.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_set.rs)

`Slot { index, generation }` (boyko_utils) is the shared key type for
sparse-set collections; `Entity` implements `From<Slot>` / `Into<Slot>`.

---

## 2. Memory subsystem ✅

### 2.1. VmReservation (per-OS reserve/commit primitive) ✅

**File:** [crates/boyko_ecs/src/ecs/memory/vm.rs](../crates/boyko_ecs/src/ecs/memory/vm.rs)

The single backing primitive of the memory subsystem (extracted Phase X.G as a
twin of the X.C/X.F arena arms; the sole survivor since X.J retired the Arena):
a write-once virtual-address reservation committed lazily at a frontier —
**bases never move across growth**.

- `reserve(len)` — reserve-only acquisition, granule-rounded
  (`os_len = align_up(len, COMMIT_GRANULE)`, 64 KiB):
  `VirtualAlloc(MEM_RESERVE, PAGE_NOACCESS)` on Windows (hand-declared
  externs, no `windows-sys` dep), `mmap(PROT_NONE)` on Unix
  (target-gated `libc`, overcommit-mode-2-proof). Zero commit charge.
- `commit(old, new)` — frontier commit via `VirtualAlloc(MEM_COMMIT)` /
  `mprotect(RW)`; pages are demand-zero (the J/J-XI never-written-reads-zero
  invariants build on this).
- The `cfg(any(miri, not(any(windows, unix))))` fallback arm eagerly
  `alloc_zeroed`s the full `os_len` (commit = no-op) so all growth
  bookkeeping runs under Miri / wasm32.
- `Drop` releases through the per-cfg-arm matching deallocator (the M-001
  discipline — cross-dealloc is statically impossible).
- API (`pub(crate)`): `reserve(len)`, `base()`, `os_len()`, `commit(old,
  new)`. (`reserve_unzeroed` was deleted with its sole client in X.J.)

Consumers: every `ComponentPool` (§2.3, Phase X.I) and the entity-metadata
`InlandStore` (§4.3, Phase X.G). Each owner carries its own slab-doubling
policy on top; vm.rs stays policy-free.

### 2.2. Chunk — DELETED (Phase X.I) ✅

`memory/chunk.rs` is gone. The `{ start_index, capacity, is_dirty }` metadata
was written by every mutation and read by NOBODY (the dirty flag predated
Phase 10's real per-row `Tick` columns); deleting it removed a per-mutation
runtime-divisor `udiv` + bounds-checked store from `add`/`swap_remove`/
`set_component`/`write_at`. Query-side "chunks" (`for_each_chunk`,
`chunk_iter`, `par_chunk`) are row-range batching — unrelated and untouched.

### 2.3. ComponentPool (type-erased + tick sub-regions, self-growing) ✅

**File:** [crates/boyko_ecs/src/ecs/memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

```rust
pub struct ComponentPool {
    buffer: NonNull<u8>,            // data sub-region base == vm.base(); WRITE-ONCE
    len: usize,                     // live rows; row i at buffer + i*stride (X.B)
    committed_rows: usize,          // warm-path capacity oracle (ONE cmp in add)
    reserve_rows: usize,            // the ceiling = capacity(); immutable
    component_layout: Layout,       // cached from registry
    data_committed: usize,          // byte frontier, granule-aligned (cold)
    ticks_committed: usize,         // byte frontier per tick region (cold)
    added_base: NonNull<UnsafeCell<Tick>>,   // vm.base()+data_len; WRITE-ONCE
    changed_base: NonNull<UnsafeCell<Tick>>, // +tick_len; WRITE-ONCE
    component_id: usize,
    drop_fn: Option<DropFn>,              // type-erased Drop (M-004)
    component_type_id: TypeId,            // typed-API validation (C-004)
    vm: VmReservation,              // declared LAST: Drop's drop_fn loop runs first
}
```

**Phase X.I**: each pool owns one `VmReservation` laid out
`[data | added_ticks | changed_ticks]` (granule-aligned sub-regions). Eager
reserve, ZERO initial commit; `#[cold] grow_rows` doubles committed slabs
`[64 KiB … 64 MiB]` with ticks in lockstep BY ROWS, an idempotent no-op arm,
and a sufficiency proof (GROW1-XI, plan D4) — growth is O(1) in live rows,
never copies, never moves a base. Sizing: `with_default_sizes` ⇒
`clamp(POOL_TARGET_DATA_BYTES/stride, POOL_MIN_ROWS, POOL_MAX_ROWS)`
(1 GiB / 2^16 / 2^24 syscall arms; 4 MiB / 256 / 2^18 fallback); the explicit
constructor `new(component_id, reserve_rows)` = exact ceiling, clamp-bypass by
design (★R1-9 — also the small-ceiling test knob). Phase X.J collapsed the
legacy `new(arena, id, n, m)` shape (`reserve_rows = n × m` EXACTLY, the D2
mapping) when it retired the Arena. See
[PHASE-XI-RESULTS.md](archive/PHASE-XI-RESULTS.md).

**API** ([component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)):
- Raw byte: `add(&[u8])` (739), `set_component(idx, &[u8])`, `get_raw(idx)`,
  `get_raw_mut(idx)`.
- Typed (TypeId-guarded, C-004): `add_typed::<T>`, `set_component_typed::<T>`,
  `get_typed::<T>`, `get_mut_typed::<T>`.
- Removal: `swap_remove(idx)` (937), `pop()` — both run `drop_fn`.
- Iteration: `buffer_ptr() -> *const u8` (1302) — the dense, SIMD-aligned base
  (`SIMD_BUFFER_ALIGN = 32`, Phase X.A); `count() -> usize` (1226) = the `len`
  field.
- Row addressing: the private `#[inline] unsafe fn row_ptr(&self, idx)` (710) =
  `buffer.as_ptr().add(idx * stride)`.

**Phase X.B** deleted the former parallel `units: Vec<Unit>` (each entry was
provably `buffer + i*stride`), replacing it with the computed `row_ptr` + an
explicit `len`. This net-removed `unsafe` (the `commit_units` raw-write loop is
gone) and shrank the Miri surface. The hot read/iter paths (`column.ptr.add`,
`fetch.base.add`) never used `units`, so iteration is unaffected. See
[PHASE-XB-RESULTS.md](archive/PHASE-XB-RESULTS.md).

**Phase 10** added the per-row `added` / `changed` tick columns; **Phase X.I**
moved them from heap `Box<[UnsafeCell<Tick>]>`es into the pool's own
reservation (write-once `added_base`/`changed_base`, valid for the committed
prefix; never-written slots read `Tick::ZERO` via demand-zero — the J-XI
never-written invariant; vacated slots may hold stale ticks, write-before-read
covers re-adds). `UnsafeCell` gives interior mutability through `&self` for
filter reads while the Phase 9 scheduler's per-`(archetype, component)`
exclusivity keeps writes sound; adjacent-row writes from sibling `par_iter`
chunks target distinct locations (Round 2 C3).

**ZST (tag) pools — Phase 22 D1/D6.** Size-0 components are first-class
(the old `debug_assert!(size > 0)` rejection is gone). A `stride == 0` pool is
**tick-only**: `pool_byte_layout` degrades to `data_len == 0`,
`added_off == 0`, `os_len == 2 * tick_len`
([constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):256+);
`pool_reserve_rows(0) == POOL_MAX_ROWS` (rows bounded by the tick regions
only — 2^24 × 4 B × 2 = 128 MiB address space per tag pool per hosting
archetype, 2 MiB under the cfg fallback `POOL_MAX_ROWS = 262_144`
([constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):85/:90), zero
resident until commit). Construction order is load-bearing (O1): tick bases
derive from `base = vm.base()`; `buffer` is set per-arm LAST — for ZSTs a
**dangling, provenance-free, non-null** pointer at
`SIMD_BUFFER_ALIGN.max(align)` (SIMD-A1 holds), valid only for zero-size
access. Growth routes through the `#[cold]` sibling `grow_rows_zst`
(tick-frontier-driven, GROW1-ZST proof chain Z1–Z6; `data_committed == 0`
invariant — the data region is never committed). `swap_remove`/`row_ptr`/the
drop loop are unchanged code with extended SAFETY arms (0-byte copies between
equal dangling pointers; `drop_in_place::<ZST>` reads no bytes). Storage cost:
exactly **8 B/row** (the tick pair) — kept so `Added<Tag>`/`Changed<Tag>` work
with zero filter changes (the signature-only alternative is a compile-but-lie).

### 2.4. Arena + MemFreeBlockMaster — DELETED (Phase X.J) ✅

`memory/arena.rs` (the shared growing arena — X.C lazy-commit, X.F huge
reserve + frontier slabs) and `memory/free_mem_block.rs` (its best-fit
free-block tracker) are gone: **client-less since Phase X.I** — every
`ComponentPool` owns its memory via a per-pool `VmReservation` (§2.1/§2.3),
so the shared-arena policy layer had zero production users. With them went
the `EcsMaster::arena` field + `with_arena_reserve()` / `arena()`, the
`&Arena` / `*const Arena` parameter-and-field chain (`Archetype`,
`ArchetypeMaster`, `ComponentPoolBundle`), the arena constants
(`DEFAULT_ARENA_RESERVE`, `ARENA_MIN_SLAB` / `ARENA_MAX_SLAB`, the
master-era compaction class), and the dead `ChunkId` / `InlandChunkId` ids;
`ARENA_COMMIT_GRANULE` was renamed `COMMIT_GRANULE`.
`ArchetypeMaster::new` / `with_capacity` are no longer `unsafe` (the
contract existed only for the arena pointer). Net −2,999 LOC. See
[PHASE-XJ-RESULTS.md](archive/PHASE-XJ-RESULTS.md).

### 2.5. Row addressing — no `Unit` (removed Phase X.B) ✅

Rows are computed arithmetic from the pool's stable, write-once reservation base. The
`Unit { ptr }` wrapper and `id_unit.rs` are gone. See §2.3 + [PHASE-XB-RESULTS.md](archive/PHASE-XB-RESULTS.md).

---

## 3. Component subsystem ✅

### 3.1. Component trait

**File:** [crates/boyko_ecs/src/ecs/core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs)

```rust
pub trait Component: 'static + Sized {
    fn component_id() -> ComponentId;
    fn debug_type_name() -> &'static str { type_name::<Self>() }
    fn type_id() -> TypeId { TypeId::of::<Self>() }
    fn mem_size() -> usize { size_of::<Self>() }
    fn alignment() -> usize { align_of::<Self>() }
}
```

`#[inline]` on default methods (cross-crate hint). No `#[inline(always)]`
(Phase 5a demoted every site per CLAUDE.md principle 7).

### 3.2. ComponentMask

**File:** [crates/boyko_ecs/src/ecs/core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs)

512-bit mask (`[BitSet<u64>; 8]`). Private `blocks`; access via `block(i)`
(C-023). `MAX_COMPONENTS = 512` enforced by `debug_assert!` (M-009 / C-011 fixed
the historical `% 8` wrap bug).

### 3.3. ComponentPoolBundle

**File:** [crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs)

One type-erased `ComponentPool` per `ComponentId` within an archetype. Two-phase
commit (C-009): `can_push_entity_components(&[(id, &[u8])]) -> bool`
(read-only) + `push_entity_components(...)` (lockstep append). `swap_remove_unit`
returns `EcsResult<()>` (C-019).

### 3.4. ComponentRegistry (global static)

**File:** [crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs)
(the tag id types split out into
[component_registry/tags.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs)
in the god-file refactor; `clone.rs` / `required.rs` / `serialize.rs` sit alongside)

Lock-free store of `ComponentLayout { layout, type_name, type_id, drop_fn }`,
backed by `static LAYOUTS: [OnceLock<ComponentLayout>; MAX_COMPONENTS]` (M-002 /
C-002 / Q-004 / Q-010: were `static mut`). `MAX_COMPONENTS = 512` ([component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):61).

**API:** `register_new::<T>()` (production — called from the derive's
`component_id()` via a per-type `OnceLock`; also installs hooks if the type
carries `#[component(...)]`), `register_layout::<T>(id)` (test escape hatch),
`get_layout(id)`, `get_layout_unchecked(id)`. IDs are first-call order, stable
per-process, NOT across processes; collisions panic in debug AND release.

### 3.5. `#[derive(Component)]` macro

**File:** [crates/boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs)

Emits `const SIZE/ALIGN/layout()` inherents + the `Component` impl (lazy
`component_id()`). The optional `#[component(on_add = path, …)]` attribute binds
Phase-14a hooks (mutually exclusive with the runtime builder; see §3.6).

---

### 3.6. Component lifecycle hooks (14a) & observers (14b) ✅

Two complementary reactive-callback mechanisms firing at the four
structural-op kinds (`add` / `insert` / `replace` / `remove`; a despawn
fires `replace` + `remove` per dying component — no separate despawn kind).
Both are gated by the per-archetype `ArchetypeFlags` `u16` bit-test so a
world with no callback pays one `test`/`jz` and zero allocation.

**Hooks (Phase 14a)** — one write-once fn-ptr per component *type*, stored
in the process-global `HOOKS` table; bound via the `#[component(on_add =
…)]` derive XOR the runtime `register_component_hooks` builder. The
per-archetype `ON_*_HOOK` bits are OR-seeded at construction from `HOOKS`.
Files: [core/component/hooks/](../crates/boyko_ecs/src/ecs/core/component/hooks/)
(`mod.rs` = `ComponentHooks` / `HookFn` / `HookContext`, `dispatch.rs` =
`trigger_on_*`, `deferred_master.rs` = the read-only `DeferredEcsMaster`
view, `builder.rs` = `ComponentHooksBuilder`, `scope.rs` = the deferred-drain
depth guard, `archetype_flags.rs` = the bit definitions). See
[PHASE-14-RESULTS.md](archive/PHASE-14-RESULTS.md).

**Observers (Phase 14b)** — the runtime-mutable sibling: an `add`/`remove`-able
*list* of fn-ptrs keyed by `(kind, component)`, stored **per-world** (not
global), with NO staleness panic (late registration runs a dynamic
archetype-bit walk). At every fire site hooks run first, then observers.

Key types — [core/component/observers/mod.rs](../crates/boyko_ecs/src/ecs/core/component/observers/mod.rs):

```rust
pub struct ObserverId(pub(crate) u64);              // mod.rs:59  — monotonic, never reused
pub enum   ObserverKind { Add, Insert, Replace, Remove } // mod.rs:70 (#[repr(u8)], the [kind] index)
pub type   ObserverFn =                             // mod.rs:98
    unsafe fn(DeferredEcsMaster<'_>, ObserverContext);
pub struct ObserverContext { entity, component_id, kind }; // mod.rs:107
pub(crate) struct ObserverEntry { id, runner };     // mod.rs:124 — 16 B POD, Copy (fire loop copies by value)
struct     ObserverLists {                          // mod.rs:137 — the lazily-boxed payload
    by_kind_component: [[Vec<ObserverEntry>; 512]; 4],   //   [kind][cid] dense 2-multiply index (mod.rs:139~)
}
pub struct ObserverRegistry {                       // mod.rs:159 — Send + Sync (fn-ptr-only, no unsafe impl)
    lists: Option<Box<ObserverLists>>,              //   None until the first add_observer (zero 48 KiB cost)
    next_id: u64,
}
```

`ObserverRegistry` lives as a `pub(crate)` field on **`ArchetypeMaster`**
([archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):65~),
NOT on `EcsMaster` (Phase 14b D3, the critic's C1 crux): co-locating it
there lets the single creation funnel `create_archetype`
([archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):141)
seed each new archetype's `ON_*_OBSERVER` bits at construction (borrow-split:
read `&self.observer_registry` into a `Copy` `ArchetypeFlags`, then OR into the
new slot), which a recipe taking only the component ids could never do from
`&EcsMaster`. Registry API ([observers/mod.rs](../crates/boyko_ecs/src/ecs/core/component/observers/mod.rs)): `add` (:189), `remove` (:211),
`has_observer` (:237), `fire_list` (:253).

**Per-archetype flag bits** —
[hooks/archetype_flags.rs](../crates/boyko_ecs/src/ecs/core/component/hooks/archetype_flags.rs).
`ArchetypeFlags(u16)`: hook bits `ON_*_HOOK` = `1<<0..1<<3` (bit 4 reserved
for a future `on_despawn` hook), observer bits `ON_*_OBSERVER` =
`1<<5..1<<8`, and the combined gate masks `ON_*_ANY = ON_*_HOOK |
ON_*_OBSERVER`. `insert_from_observers(cid, &reg)` OR-seeds the observer bits
at construction (symmetric to `insert_from_hooks`); `insert_observer_bits(other)`
merges them without disturbing the hook bits. Each structural-op fire site
widens its inner test from `ON_*_HOOK` to `ON_*_ANY` — a different immediate in
the same `test`/`jz`, so the no-callback hot path stays byte-identical (the
0%-gate, bench-verified).

**Dynamic bit maintenance** — `ArchetypeMaster::add_observer`
([archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):842)
runs an add-first walk (`iter_archetypes_mut`, raise the bit on archetypes
containing `cid`) only when the `(kind, cid)` list went empty → non-empty;
`remove_observer` ([archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):875) runs a remove-last recompute (`flags = (flags &
!bit) | (any_sibling_observes_kind ? bit : 0)`, preserving the hook bit) only
when the list became empty. Both seed sites (`create_archetype`,
`add_existing_archetype` [archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):477) and both walks are cross-checked by the
`#[cfg(debug_assertions)]` bit⇔registry tripwire
`debug_assert_observer_flags_consistent` ([archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):929).

**Dispatch (the 4 `#[cold] #[inline(never)]` fire fns)** —
[core/component/observers/dispatch.rs](../crates/boyko_ecs/src/ecs/core/component/observers/dispatch.rs):
`fire_on_add_observers` / `fire_on_insert_observers` /
`fire_on_replace_observers` / `fire_on_remove_observers` (emitted by the
`define_fire_observers!` macro at line 51; instantiated at lines
118/123/129/134). A fire fn is entered ONLY when the archetype's
`ON_*_OBSERVER` bit proved some component carries it; it reads the per-world
registry and fires every observer for `(kind, component_id)` in registration
order.

> **OBS-FIRE-LOOP invariant** (dispatch.rs:19-33~, the single most dangerous
> spot in 14b): no registry `&` — nor any `world`-derived `&` — may be live
> across the `DeferredEcsMaster::from_world` mint or the runner call. Each
> loop turn re-derives a fresh registry `&`, copies one 16 B `ObserverEntry`
> by value, and lets every borrow end **before** the view is minted. The
> registry lives *inside* the world the view reborrows, so a held `&` spanning
> the mint is the exact Tree-Borrows protected-tag conflict (the F2-class
> hazard) that produced UB in Phase 14a. Miri `-Zmiri-tree-borrows` is the
> soundness oracle here, not code review.

**The 10 fire sites** (each: outer gate unchanged, inner `ON_*_HOOK` widened to
`ON_*_ANY`, hooks fire before observers, observer set == hook set per site).
Phase 22 added the bottom three (the dynamic-tag attach/detach/re-tag
migration paths — counted against this ledger per the Phase-14b lesson):

| Site | File:line (observer calls) | Kinds |
|------|----------------------------|-------|
| `EcsMaster::create_entity` | [ecs_master/entity_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs):137, fires 270/290 | add, insert |
| `EcsMaster::create_entity_at` | [ecs_master/entity_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs):345, fires 448/468 | add, insert |
| `EcsMaster::fire_despawn_hooks` | [ecs_master/entity_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs):691, fires 774/786 | replace, remove |
| `SpawnAtCommand::apply` | [commands/spawn_at_command.rs](../crates/boyko_ecs/src/ecs/core/commands/spawn_at_command.rs):113, fires 374/394 | add, insert |
| `InsertCommand::apply_replace_in_place` | [commands/insert_command.rs](../crates/boyko_ecs/src/ecs/core/commands/insert_command.rs):113, fires 176/201 | replace, insert |
| `migrate_entity_insert` | [commands/migration_helpers.rs](../crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs):332, fires 899/918 | add, insert |
| `migrate_entity_remove` | [commands/migration_helpers.rs](../crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs):980, fires 1132/1138 | replace, remove |
| `migrate_entity_attach_ids` (Phase 22) | [commands/migration_helpers.rs](../crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs):1372, fires 1613/1625 | add, insert |
| `migrate_entity_detach_ids` (Phase 22) | [commands/migration_helpers.rs](../crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs):1658, fires 1838/1848 | replace, remove |
| `retag_in_place` (Phase 22) | [commands/migration_helpers.rs](../crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs):1922, fires 1952/1988 | replace, insert |

The plan's original "6 fire sites" undercounted: Phase 14a also fires at the 4
deferred-command apply sites (rows 4–7), so observers were silent for
the entire `Commands` API until the tester wrote tests against the user-facing
API. See [PHASE-14B-RESULTS.md](archive/PHASE-14B-RESULTS.md).

**Id-keyed hook registration (Phase 22 D8)** —
`register_hooks_by_id(component_id: ComponentId, hooks: ComponentHooks) ->
Result<(), HooksError>`
([component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):885)
is the entry point for ids with no Rust type to name (dynamic tags); the typed
runtime path delegates to it. The Phase-21 H1 staleness gate applies
identically: `Err(HooksError::AlreadyArchetyped)` once the id was ever
archetyped in ANY world (the flags of that archetype are frozen), else the
write-once `Err(AlreadyRegistered)` semantics. **Contract (documented on
`register_tag` and in the book): mint → register hooks → first attach.**
Observers need no gate (`add_observer` runs the dynamic archetype-bit walk).

**Public API on `EcsMaster`** —
[core/ecs_master/observer_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs):
`register_component_hooks::<C>() -> ComponentHooksBuilder` (91);
`observe_on_add::<C>(runner) -> ObserverId` (143) / `observe_on_insert` (151)
/ `observe_on_replace` (161) / `observe_on_remove` (170); the type-erased
`add_observer(kind, cid, runner)` (182); `remove_observer(id) -> bool` (199).
Phase 14b also changed `get_component_mut::<T>(entity)` ([component_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs):553) from
`Option<&mut T>` to `Option<Mut<'_, T>>` — the change-detection-correct
direct-API mutator (the `Mut` deref-guard bumps the row's change tick).

### 3.7. Tags — static ZST + dynamic runtime (Phase 22) ✅

**Static tags (D1/D2)** — any size-0 `#[derive(Component)]` type. Detection is
`ComponentLayout.size == 0` (`is_zst()`,
[component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):157)
— no attribute, no new trait. Storage = the tick-only pool (§2.3). The derive
also emits a **single-component `Bundle`** (so `commands.spawn(PlayerTag)`
works) — suppressed by `#[component(no_bundle)]` (§19, §7). `With`/`Without`/
`&Tag`/`Mut<Tag>`/`Added`/`Changed`/hooks/observers all work unmodified;
re-inserting a present tag = replace semantics (`on_replace` + `on_insert` +
changed-tick stamp). E2E suite:
[tests/phase22_static_tags.rs](../crates/boyko_ecs/tests/phase22_static_tags.rs).

**Dynamic tags (D3)** — runtime-minted, name-keyed ids in the shared 512-slot
registry. **Implementation deviation (recorded):** `TagId` lives inside the
registry module, NOT the planned `identifiers/tag_id.rs` — mint-protocol
locality + constructor privacy (`TagId(pub(crate) ComponentId)`). Key items, in
[component_registry/tags.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs):

- `TagId` (:49) + the one-way public bridge `component_id()` (:56, `const`)
  / `From<TagId> for ComponentId` (:61). No `ComponentId → TagId` constructor.
- `TAG_NAMES` intern (:155, `OnceLock<Mutex<HashMap<Box<str>, TagId>>>`,
  NAME-keyed idempotency — O2; bounded `Box::leak` ≤ 512);
  `try_register_tag_by_name` (:182); `tag_by_name` (:201).

…and in
[component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):

- `DynamicTagMarker` (uninhabited sentinel TypeId, :201);
  `ComponentLayout::new_dynamic_tag` (:171).
- `try_register_dynamic` (:965) — bounded CAS on `NEXT_ID`, `None` at the
  ceiling; slot-occupied ⇒ `#[cold]` panic (:996), NEVER the same-TypeId
  idempotent return (would alias two names).
- `register_hooks_by_id` (:885) with the H1 gate — see §3.6.

**World surface** —
[ecs_master/tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/tag_api.rs):
`try_register_tag` (:47) / `register_tag` (:65, panicking sugar) /
`tag_by_name` (:76) / `has_tag` (:89, O(1) inland → archetype ptr → signature
bit) / `add_tag` (:130) / `remove_tag` (:200). Deferred:
`EntityCommands::add_tag`/`remove_tag`
([params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs):182/:196)
via the POD `AddTagCommand`/`RemoveTagCommand`
([commands/tag_commands.rs](../crates/boyko_ecs/src/ecs/core/commands/tag_commands.rs):38/:54).

**Dynamic migration (D9)** — allocation-free id-keyed helpers in
[commands/migration_helpers.rs](../crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs):
`merged_archetype_id_dyn` (:1230) / `without_ids_archetype_id` (:1305, maps
`kept.is_empty()` → the EMPTY archetype — O3) / `migrate_entity_attach_ids`
(:1372, zero-retained attach-FROM-empty is first-class) /
`migrate_entity_detach_ids` (:1658) / `retag_in_place` (:1922, the present-tag
replace path). All three fire hooks + observers (ledger rows 8–10 in §3.6)
with Phase-14a §3.4 reborrow confinement. `MAX_BUNDLE_ARITY` raised 8 → 16
(:58, lock-step with the derive and `spawn_at_command.rs`).

**Empty archetype (D5)** — entities may hold zero components. Lazy: resolved
through `get_or_create_archetype(&[])` on first demand (no reserved constant,
preserves the Phase-12.6 lazy `EcsMaster::new` budget). `EcsMaster::spawn_empty`
([ecs_master/entity_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs):667);
`Commands::spawn_empty`
([params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):184)
= `spawn(EmptyBundle)` — the hand-written zero-component bundle
([bundle/self_bundle.rs](../crates/boyko_ecs/src/ecs/core/bundle/self_bundle.rs):135,
zero unsafe, own `BundleTypeId` so warm spawns hit the static bundle cache).
Remove-last-component routes INTO the empty archetype (an ordinary migration
edge); the empty signature matches only zero-required-component queries
(flecs-invariant subset matching). Suite:
[tests/phase22_empty_archetype.rs](../crates/boyko_ecs/tests/phase22_empty_archetype.rs).

**Query terms (D4)** — see §8.6.

Suites: [tests/phase22_tags.rs](../crates/boyko_ecs/tests/phase22_tags.rs)
(out-of-crate reachability — W3), `phase22_tags_exhaustion.rs` (own process,
drains the registry), `phase22_query_terms.rs` (per-driver),
`phase22_bundles.rs`, `phase22_static_tags.rs`, `phase22_empty_archetype.rs`.

---

### 3.8. EnableTag — enable-bit, non-fragmenting tag backend ✅

The **second tag storage backend**. §3.7 tags use the *signature/table* backend
(membership = an archetype-signature bit + a tick-only `ComponentPool`); an
**EnableTag** uses the *bitset* backend: its id is filtered out of every
archetype signature and owns no pool — presence is a single per-row bit in a
paged per-archetype bitset. Toggling is therefore **O(1): no migration, no
structural-generation bump, no hook/observer fire, no deferred drain** (flecs
`CanToggle`). The cost: no per-row tick storage, so `Added<T>`/`Changed<T>` are
compile-rejected on a bitset tag (the Phase-22 "compile-but-lie" lesson). Use it
for high-churn transient flags (`Stunned`, `Visible`, `Sleeping`). Authoritative
design: [ENABLE-TAG-PLAN.md](archive/ENABLE-TAG-PLAN.md) +
[ENABLE-TAG-PLAN-AMENDMENT-D7.md](archive/ENABLE-TAG-PLAN-AMENDMENT-D7.md).

**Storage model (D1)** — three layers in
[component/enable/enable_store.rs](../crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs):

```text
EnableStore   per-archetype; inline-4 SmallList4<(ComponentId, EnableColumn)>   (enable_store.rs:599)
  └ EnableColumn   one per (archetype, tag); lazily-paged page directory          (enable_store.rs:160)
      └ EnablePage   #[repr(C, align(64))] [AtomicU64; 64] = 512 B / 4096 rows    (enable_store.rs:60)
```

The bit's home is `(archetype, row)` — exactly like component data + Phase-10
tick columns — so it travels through the existing swap-remove / migration row
loops and never leaks across entity recycling. A page is allocated only on the
first toggle into its 4096-row range (`get_or_alloc_page`, `#[cold]`), capping
any single allocation at 512 B (a `const _` size+align pin enforces it,
enable_store.rs:65-66). Index arithmetic: `page = row >> 12`,
`word = (row >> 6) & 63`, `bit = row & 63`. `EnableColumn::test` (enable_store.rs:206)
reads a `None` page as `false` (all-disabled).
`swap_remove_bit` (enable_store.rs:299) is **READ-first**
(snapshot `last`'s bit before any write)
so an adjacent `removed == last - 1` / same-word pair cannot corrupt; the
store-level `swap_remove_row` (enable_store.rs:662) fires it across every
allocated column once per structural op. Migration uses a borrow-free owned
snapshot: `read_row_bits` (enable_store.rs:702, Phase-1 READ) → owned
`(ComponentId, bool)` `Copy` values that survive a later `swap_remove_row` of the
very columns read (structurally not the NEW-1 dangling-slice class) →
`write_row_bit` (enable_store.rs:714, Phase-2 WRITE; a clear never allocates a
column/page).

**StorageKind classifier (D5)** —
[component/component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):
`enum StorageKind { Table = 0, Bitset = 1, Dense = 2 }` (:323 — the discriminant
space is deliberately extensible, 3 is reserved for relationships), recorded in the cold
parallel `static STORAGE_KIND: [AtomicU8; MAX_COMPONENTS]` (:373) — kept parallel
to `LAYOUTS`/`HOOKS` rather than as a sixth `ComponentLayout` field so
`ComponentLayout` stays pinned at 56 B (TRIPWIRE 2). `storage_kind(id)` (:388,
one `Relaxed` load, out-of-range → `Table`); write-once
`set_storage_kind` (:433, debug-panics on re-classify to a *different* kind);
`install_storage_kind::<C>` (:729, const-gated on
`C::STORAGE_IS_BITSET`); the dynamic mint
`try_register_enable_tag_by_name`
([component_registry/tags.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs):134).
`EnableTagId` (:93) is a
`#[repr(transparent)]` proof-of-mint over `ComponentId` with a one-way public
bridge `component_id()` (:99) / `From<EnableTagId> for ComponentId` (:104) — no
reverse constructor. The compile-time discriminator is the new
`Component::STORAGE_IS_BITSET` const (default `false`,
[component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs):65),
which the `Added`/`Changed` per-monomorphization const-asserts read to
compile-reject change detection on a bitset tag.

**Signature filtering (Step 4)** — archetype construction skips any id with
`storage_kind == Bitset`. The signature mask is built through the single shared
`filtered_signature_mask` helper ([archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs):314 — every
non-signature-storage id is skipped there, so the registry-minted signature
matches bit-for-bit), and the pool bundle skips the same ids at
`Archetype::create_by_ids` (:326) and `register_component_inplace` (:466). The
`enable_store` field
([archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs):154~)
sits on every `Archetype` (`EnableStore::new()` at both construction sites);
`set_enable_bit` (archetype.rs:589) flips the paged bit and returns
`newly_allocated == true` only on the first column for the tag; `enable_column_ptr`
(archetype.rs:567) hands the query fetch a borrowed `*const EnableColumn` (or
NULL). `swap_remove_row` / remove paths fire `enable_store.swap_remove_row` only
when `!enable_store.is_empty()` (the 0%-gate for enable-free archetypes,
archetype.rs:1024~/:1056~/:1114~/:1137~).

**The cull oracle (D2)** —
[component/enable/enable_presence.rs](../crates/boyko_ecs/src/ecs/core/component/enable/enable_presence.rs)
`EnablePresence`: per-world, one lazily-`AtomicPtr`-published
`Box<[AtomicU64; 16]>` (128 B) per tag id, a bit per `ArchetypeId`
(`PRESENCE_WORDS = 16`, `PRESENCE_CAPACITY = 1024`). `contains(tag, arch)`
(:164) is O(1) (one pointer load + one word load + one bit test); a never-toggled
tag (null slot) → `false` = "no column ⇒ every row disabled ⇒ drop the
archetype". It is consulted ONLY as the `contains` oracle over an
already-bounded matched set — **never a driver** (deliberately no
`for_each_present`): a presence-driven enumeration would be the unbounded
sole-`Enabled` path the plan compile-rejects. `note_column_alloc` (:120, `&self`,
sets one bit + bumps a lock-free `epoch` with `Release`) and `snapshot_present`
(:219, a bounded 16-word `Acquire`-load loop into an `ArchetypeBitSet`) back the
D7 candidate-seeded scan. The atomic words are the forward seam for the D7
worker-marking toggle; v1's `Relaxed`/single-thread discipline needs no retry.
`clear_archetype` (:268) clears one arch's bit across every tag on archetype
removal/`clear()`. It lives on `ArchetypeMaster` next to `enable_generation`
([archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):102~/:86~)
so the presence-bit set and the generation bump pair atomically inside
`note_enable_column_alloc` (archetype_master.rs:630) — keyed by *this* world's
`ArchetypeId` (multi-world-correct). `enable_generation` (:612) is independent of
`structural_generation` (a toggle never bumps the latter) and is left monotonic
across `clear()` (a recycled id can never be a stale candidate).

**API (D3/D5)** —
[ecs_master/enable_tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs):
registration `register_enable_tag` (:60) / `try_register_enable_tag` (:72);
typed toggle `enable::<T>` (:87) / `disable::<T>` (:95) / `is_enabled::<T>` (:104);
dynamic `enable_id` / `disable_id` / `is_enabled_id` (:113/:119/:126). All
mutators take `&mut self` (the v1 soundness ground for the `Relaxed` atomics —
do NOT relax to `&self` without the D7 loom proof); dead/stale entities are
silent no-ops. The internal core `set_enable_bit` (:148) resolves the live
inland → current post-swap row → reborrows `&mut Archetype` (confined, dropped
before touching `archetype_master`) → flips the bit → fires
`note_enable_column_alloc` once per genuinely-new column. Deferred toggle:
`EntityCommands::{enable, disable, enable_id, disable_id}`
([params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs):220/:236/:249)
→ the POD `EnableTagCommand { entity, tag, value }`
([commands/enable_tag_commands.rs](../crates/boyko_ecs/src/ecs/core/commands/enable_tag_commands.rs):45)
whose `apply` calls `enable_id`/`disable_id` at the apply window. Cross-archetype
migration copies the enable bits via the borrow-free two-phase snapshot in
[commands/migration_helpers.rs](../crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs)
(`read_source_enable_bits` :104 PHASE-1 / `write_target_enable_bits` :129
PHASE-2 / `fire_enable_column_alloc_bookkeeping` :162 O2), each gated by
`EnableStore::is_empty` so an enable-free entity is byte-identical to before.

**Query integration (D2/D4/D7)** — three shapes, all archetype-granularity cull
+ per-row bit test:
- **Typed filters** —
  [query/filter_enable.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter_enable.rs):
  `Enabled<T>` / `Disabled<T>`, non-archetypal per-row `QueryFilter`s
  (`IS_ARCHETYPAL = false`, `NEEDS_CHANGE_DETECTION = false`,
  `CONTAINS_ENABLE_TERM = true`). The per-archetype `EnableFetch`
  (`*const EnableColumn` or NULL) is refreshed in `set_table_*` like the
  `Added`/`Changed` `tick_base`; a NULL column reads as disabled (`Disabled`
  inverts it). Both have a no-op `init_access` (the `Without<C>` precedent — they
  declare no component access) and implement NEITHER `OrComposable` (so
  `Or<(Enabled<A>, ..)>` is a compile error — M1) nor `ArchetypalQueryFilter`
  (so `for_each_chunk` rejects them).
- **Dynamic terms** —
  [query/enable_terms.rs](../crates/boyko_ecs/src/ecs/core/iters/query/enable_terms.rs):
  `EnableTerms` (per-view, ≤ `MAX_ENABLE_TERMS = 8`,
  [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):397) populated by
  `with_enabled` / `without_enabled` on `Query`
  ([query.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query.rs):170/:185)
  and `QueryView`
  ([query_view.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs):283/:298);
  NEVER stored in the shared interned `QueryState` (QS1 stays term-agnostic —
  like `TagTerms`). Polarity bit `i`: `1` = `with_enabled` (bit must be set),
  `0` = `without_enabled` (bit must be clear). **Gate caveat:** unlike the
  Phase-22.1 archetype-level `TagTerms` (which leaves ZERO term code in the row
  loop), an enable term is a genuine per-row predicate, so the `is_empty()` gate
  stays a RUNTIME branch *inside* the row loop — but it is loop-invariant
  (`enable_terms` is never written mid-iteration), so the compiler hoists it to
  one predicted-not-taken branch (bench-flat).
- **The `(D, F)` seam** —
  [query/state.rs](../crates/boyko_ecs/src/ecs/core/iters/query/state.rs):
  `assert_query_shape` const-asserts (state.rs:211) reject an enable tuple with no
  positive bound (must be paired with a `With<_>` or be a SOLE single leaf;
  `CONTAINS_ENABLE_TERM && CONTAINS_CHANGE_DETECTION` is also rejected,
  state.rs:211). `HAS_ENABLE_TERM = F::CONTAINS_ENABLE_TERM` (state.rs:103) gates
  the whole enable machinery off for non-enable `(D, F)`. The candidate-seeded
  branch `IS_CANDIDATE_SEEDED` (state.rs:119 — sole single-enable, no data
  component, no positive archetypal) seeds the matched set from the bounded
  `EnablePresence::snapshot_present` candidate snapshot
  (walked by `seed_from_candidates`, [query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs):299) instead of the full
  live-archetype set (the D7 bounded global scan — the answer to "data-less
  global scan of enabled/disabled entities"). On `update`, the
  `last_observed_enable_generation` slot ([state.rs](../crates/boyko_ecs/src/ecs/core/iters/query/state.rs):68~) is re-checked against
  `ArchetypeMaster::enable_generation`: a bump means "an archetype gained a
  column" ⇒ re-snapshot + re-cull, inside `update` (state.rs:429~).

**Derive (Wave 5)** — `#[component(storage = "bitset")]`
([boyko_macros/src/component.rs](../crates/boyko_macros/src/component.rs):71~/:84~/:178~/:315~):
must be a ZST (a fielded bitset tag has no pool to hold data — rejected at macro
time, component.rs:71~); cannot combine with lifecycle hooks (an enable-bit op fires no
hook — rejected, component.rs:84~); emits `const STORAGE_IS_BITSET = true` + an
`install_storage_kind::<Self>` call routing the minted id to `StorageKind::Bitset`;
suppresses the single-component `Bundle` emission (no pool ⇒ not spawnable as a
one-component bundle).

**Key invariants:**
1. **No change detection on a bitset tag** — `Added<T>`/`Changed<T>` over a
   `STORAGE_IS_BITSET` type is a compile error (no per-row ticks; compile-but-lie
   guard).
2. **A toggle is a structural-class `&mut EcsMaster` op** — O(1), but NOT a
   read: no `&self` toggle in v1 (the `Relaxed` atomics rely on `&mut`
   exclusivity; the `AtomicU64` words reserve the D7 `Acquire`/`Release` seam).
3. **0%-gate** — an enable-free archetype skips `swap_remove_row`
   (`is_empty()`); an enable-free `(D, F)` const-folds the entire machinery away
   (`HAS_ENABLE_TERM = false`); a query with no dynamic term takes the
   byte-identical no-term path. The runtime per-row enable-term branch is
   loop-invariant-hoisted (bench-flat), the one acknowledged non-const gate.
4. **`enable_generation ⊥ structural_generation`** — a toggle bumps only
   `enable_generation` (and only on a first column per archetype); a structural
   op never touches `enable_generation`.

Suites: [tests/loom_term_list.rs](../crates/boyko_ecs/tests/loom_term_list.rs)
(the lock-free publish/read protocol), [tests/miri_phase22_1.rs](../crates/boyko_ecs/tests/miri_phase22_1.rs),
and the per-file `#[cfg(test)]` units in `enable_store.rs` / `enable_presence.rs`
/ `enable_tag_api.rs` / `archetype.rs` / `archetype_master.rs` /
`component_registry.rs`.

---

## 4. Entity subsystem ✅

### 4.1. Entity

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    id: EntityId,            // newtype wrapping usize (C-017)
    generation: usize,       // bumped on every deallocate_entity
}
```

Fields private (C-023); access via `entity.id()` / `entity.generation()`.
`From<Slot>` / `Into<Slot>`. Equality compares BOTH fields — the load-bearing
ABA defence.

### 4.2. EntityInland (Phase 7 fast-store record)

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs)

```rust
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EntityInland {
    archetype_ptr: *mut Archetype,   // direct slab pointer; NULL ⇔ dead slot
    unit_index: u32,                 // row index into Archetype.entity_ids / columns
    generation: u32,
}
```

16 B, align 8 (layout pinned by `const _` asserts on the 64-bit target). Phase 7
replaced the legacy three-field `{ archetype_id, unit_index, generation }` with
this **direct `*mut Archetype` slab pointer** so the hot `get_component_raw` path
dereferences into the archetype without a `SparseMap` indirection.
`archetype_ptr.is_null()` (`is_null()`, line 97) is the single liveness +
generation source of truth.

### 4.3. EntityMaster (Phase 7 + X.D + X.G)

**File:** [crates/boyko_ecs/src/ecs/core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs)

```rust
#[repr(C)]                                          // X.G: hot cluster on cache line 0
pub struct EntityMaster {
    pub(crate) entities_inland: InlandStore,        // index = EntityId.0; is_null() ⇔ dead
    next_entity_id: AtomicUsize,                    // fresh-id minting (EM1/EM6)
    live_count: usize,                              // # live entities (Phase X.D)
    free_entity_ids: Vec<EntityId>,                 // LIFO recycle queue (dispatcher-only)
}
```

Phase 7 replaced the old `entities` + `SparseMap<EntityInland>` pair with the
single direct-indexed `entities_inland` fast store. **Phase X.D** removed
the EnTT-style `active_ids` + `sparse_to_active` acceleration vectors. **Phase
X.G** replaced the backing `Vec<EntityInland>` with an
[`InlandStore`](../crates/boyko_ecs/src/ecs/core/entity/inland_store.rs):
one lazy 1 GiB virtual reservation (`memory/vm.rs` `VmReservation`, §2.1)
committed in 256 KiB→×2→16 MiB frontier slabs —
**growth never reallocates, copies, or fills** (`EntityInland::NULL` is
all-zero 16 B; demand-zero pages ARE the NULL fill — invariant J). The
`Deref<Target=[EntityInland]>` keeps every read/indexed-write site and the
Phase-7 hot lookup codegen-identical; `ensure(n)` replaced every
`resize(n, NULL)`. Production spawn path got **15–54% faster** (the per-batch
resize-fill died); the g7b entity-store doubling spikes are GONE. See
[PHASE-XD-RESULTS.md](archive/PHASE-XD-RESULTS.md) +
[PHASE-XG-RESULTS.md](archive/PHASE-XG-RESULTS.md).

**API** ([entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs)):
- `allocate_entity() -> Entity` (124) — recycles from `free_entity_ids`, else
  `fetch_add` on `next_entity_id`. `pub(crate)`.
- `register_entity_with_ptr(entity, *mut Archetype, row)` (323) — writes the
  fast-store slot; `live_count += 1`.
- `register_batch(start, *mut Archetype, start_row, n)` (263, `pub(crate)`) —
  bulk fast-store write; `live_count += n`.
- `ensure_capacity(n)` (241, `pub(crate)`) — dispatcher-side lazy growth (Phase
  12.6; replaces the eager `EcsMaster::new` pre-extension).
- `deallocate_entity(entity) -> bool` (360) — bumps generation in place, nulls
  the slot, recycles the id; `live_count -= 1` on the success path only (the C1
  guard: a no-op on a stale/never-registered handle never decrements).
- `is_entity_valid(entity)` (394) / `get_entity(id)` (406) — gen-checked read
  straight from `entities_inland`.
- `entity_count()` (416) / `is_empty()` (468) — `live_count`-backed (O(1)).
- `iter_entities()` (447) — **O(capacity)** scan, skips `is_null()`, ascending
  `EntityId`. Cold inspection/test API only.
- `rewind_allocate(entity) -> bool` (520, `pub(crate)`) — C-007 guard plumbing
  for `EcsMaster::create_entity`'s failure rollback.

Workers reach `next_entity_id` only through the `EntityCounter<'s>` newtype
([system/params/entity_counter.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_counter.rs):75)
(atomic RMW only); all other mutation is dispatcher-`&mut self` inside the apply
window (SCH7).

---

## 5. Archetype subsystem ✅

### 5.1. Archetype (Phase 7 inline column table)

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs):127

```rust
#[repr(C)]
pub struct Archetype {
    columns: [Column; MAX_COMPONENTS],   // 8 KB inline table at offset 0 (Phase 7 D4)
    id: ArchetypeId,
    component_pools: ComponentPoolBundle,
    current_index: usize,                // = entity_ids.len()
    signature: ArchetypeSignature,
    flags: ArchetypeFlags,               // Phase 14a hook/observer gate bits (u16)
    component_ids: Vec<ComponentId>,
    entity_ids: Vec<EntityId>,           // parallel to pool dense indices
}
```

(Phase X.J deleted the vestigial `arena: *const Arena` field — the size stays
8480 B; tail padding under align 32 absorbed the 8 B.)

The size is pinned (`const _: () = assert!(size_of == 8480)` on the 64-bit
target). `columns` is at offset 0 so the Phase-7 fast read path issues a single
dependent load `*(arch + c*16)`:

```rust
#[repr(C)]
pub struct Column {        // archetype.rs:32 — 16 B, layout-pinned
    ptr: *mut u8,          // == ComponentPool::buffer_ptr(); NULL ⇔ absent column
    stride: u32,           // == component size; unit_index * stride = byte offset
    _reserved: u32,        // brings to 16 B so columns[c] lowers to c << 4
}
```

**API:**
- `create_entity(entity_id, &mut EntityInland, &[(ComponentId, &[u8])]) -> bool`
  (C-010 slice API; two-phase commit internally).
- `remove_entity(&EntityInland) -> RemoveOutcome`:
  ```rust
  pub enum RemoveOutcome {          // archetype.rs:89, pinned to 16 B
      Last,                         // removed was the tail
      Swapped { moved_entity: EntityId },  // tail swapped into the hole
      PoolFailure,                  // pool rejected; archetype unchanged
  }
  ```
- `pop(&mut EntityInland) -> bool` (C-008 fixed the release-mode `debug_assert!`
  bug), `has_component_id`, `has_components`, `component_mask`, `component_ids`.

### 5.2. ArchetypeSignature

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs)

`{ mask: ComponentMask, block_summary: BitSet<u8>, section_summary: BitSet<u32> }`
— hierarchical filter accelerator derived from `mask` via `Self::new(mask)`.
Fields private (C-023); the only mutation path is building a fresh signature.

### 5.3. ArchetypeRegistry

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs)

`block_groups: SparseMap<...>` keyed by 8-bit block pattern + `id_to_location`
reverse map. C-015: O(1) `len()` (debug-asserted), O(1) `unregister_archetype`,
O(1) `get_archetype_signature`. The discovery API
(`find_archetypes_with_components` / `find_matching_archetypes` /
`find_exact_match` / `find_with_filter`, each with an `_into(out: &mut Vec)`
zero-alloc variant) is the entrypoint for FEATURE_MAP's archetype-discovery
table. Small queries (≤3 components) use a stack-only relevant-block buffer.

### 5.4. ArchetypeBundle (stable-address slab)

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs)

Owning slab of `Archetype`s with stable heap addresses (so `EntityInland`'s
`archetype_ptr` and per-`(D,F)` caches stay valid) + a sparse id→slot map. The
`add_archetype` replace path uses the clear-bit-first protocol (Phase 8a
AB-R1).

### 5.5. ArchetypeMaster

**File:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):18

```rust
pub struct ArchetypeMaster {
    archetypes: ArchetypeBundle,
    registry: ArchetypeRegistry,
    next_archetype_id: ArchetypeId,
    generation: ArchetypeGeneration,              // bumps on every create_archetype
    structural_generation: ArchetypeGeneration,   // bumps on remove + clear (Phase 5c)
    pub(crate) observer_registry: ObserverRegistry,  // Phase 14b D3 (line 65)
}
```

`new()` / `with_capacity()` are safe `fn`s again (+ a `Default` impl): the
`unsafe` contract existed only for the `arena_ptr` parameter Phase X.J deleted.

The dual-generation design (Phase 5c) is the ArchetypeId-ABA fix:
- `generation` → "the set grew, reclassify deltas" (QueryState delta-add).
- `structural_generation` → "the set shrank, cached IDs may be dead" (QueryState
  full rebuild, drops the dedup bitset).

**API:** `create_archetype` (141) / `get_or_create_archetype`, `remove_archetype`
(bumps `structural_generation`), `get_archetype` / `get_archetype_mut`, the
discovery `find_*`, `archetype_generation()` / `structural_generation()`,
`add_existing_archetype` (477), `iter_archetypes` / `iter_archetypes_mut`,
`clear()`, plus the observer surface documented in §3.6:
`add_observer` (842) / `remove_observer` (875).

---

## 6. EcsMaster (top-level facade) ✅

**File:** [crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs):83

```rust
pub struct EcsMaster {
    resources: Resources,                  // dropped FIRST (Phase 8a C5)
    events: EventDispatcher,
    entity_master: EntityMaster,
    archetype_master: ArchetypeMaster,     // owns the pools (each owns its VmReservation)
    bundle_archetype_cache: OnceLock<Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>>,  // Phase 8.5, lazy (12.6)
    bundle_column_cache: OnceLock<BundleColumnCache>,   // Phase 12.5, lazy (12.6)
    change_tick: AtomicU32,                // Phase 10 per-frame counter
    last_check_tick: Tick,                 // Phase 10 wraparound scan bookkeeping
    deferred_hook_queue: CommandQueue,     // Phase 14a
    query_state_cache: OnceLock<QueryStateCache>,  // Phase 12.5, lazy; dropped LAST (C5)
}
```

Drop order is load-bearing: `resources` first (a misbehaving `Resource::drop`
still sees a valid world), then `events`, `entity_master`, `archetype_master`
(drops every `ComponentPool`, each running its `drop_fn` loop before releasing
its own `VmReservation`). The caches are `OnceLock`-lazy (Phase 12.6);
`query_state_cache` is declared LAST so any future `D::State` / `F::State`
carrying storage-derived raw pointers freed-before-storage trips Miri rather
than miscompiling (C5). The `arena: Box<Arena>` field, its two-phase
raw-provenance mint, and `with_arena_reserve()` were retired in Phase X.J
(client-less since X.I); the SEND1 `Send + Sync` justification was updated in
place.

**API** (full surface in
[FEATURE_MAP § EcsMaster](FEATURE_MAP.md#high-level-facade-ecsmaster)) — the
inherent `impl` blocks are split one file per surface under
[core/ecs_master/](../crates/boyko_ecs/src/ecs/core/ecs_master/), so every line below is quoted together with the file
that declares it:

- Construction — [ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs):
  `new()` (426) / `with_capacity(entity_cap, arch_cap)` (473).
- Archetypes / spawn / despawn — [entity_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs):
  `create_archetype` (48) / `get_or_create_archetype` (55);
  `create_entity(arch, &[(id, bytes)]) -> EcsResult<Entity>` (137);
  `spawn_one::<A>` (582) / `spawn_two::<A, B>` (618) / `spawn_empty` (667);
  `delete_entity` (798).
- Bulk spawn — `spawn_batch::<B, I>` ([ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs):1079).
- Component access — [component_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs):
  `get_component_raw` (176) / `set_component_raw` (444);
  `get_component_mut::<T> -> Option<Mut<T>>` (553).
- Queries — `query::<D, F>() -> QueryView` ([ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs):825).
- Cold scans — [entity_query_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_query_api.rs):
  `has_entity` (17) / `entity_count` (69) / `archetype_count` (75) /
  `iter_entities` (87) / `query_entities` (92).
- Systems — [system_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/system_api.rs):
  `run_system::<F, M, Out>` (111) / `run_cached_system::<S>` (142) — one-shot
  runners; see the §9.1 `Changed`-window note.
- Tags (Phase 22) — [tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/tag_api.rs):
  `try_register_tag` (47) / `register_tag` (65) / `tag_by_name` (76);
  `has_tag` (89) / `add_tag` (130) / `remove_tag` (200) — §3.7.
- Resources / hooks / observers / states / events: see §9, §3.6, §13, §14.
- Teardown — `clear()` ([ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs):1026).

Error type — [`core/error.rs`](../crates/boyko_ecs/src/ecs/error.rs) `enum
EcsError` (§16). The `anyhow` dependency is gone (C-019).

### 6.1. Multi-world model (Phase 21)

N `EcsMaster`s / `App`s coexist in one process; the Phase 21 audit verified
every process-global is **metadata-only** (no world-derived state): the
component / event / bundle-type / query-type / resource-type registries, the
`HOOKS` table, the H1 `EVER_ARCHETYPED` bitmask, and the `WorldId` counter.
Everything else (archetypes, pools, entities, caches, observers, resources,
event buffers, states) is world-owned. Rules:

- **`WorldId`** ([identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs))
  — `u64` minted per `EcsMaster::new`/`with_capacity` from a process-global
  atomic; accessor `EcsMaster::world_id()`.
- **Schedule-world binding (H2)** — a `Schedule` records the build world's
  `WorldId` at `ScheduleBuilder::try_build`; `Schedule::run` release-panics
  (`boyko-B9101`) on a mismatch (Bevy parity). This closes the cross-world UB
  surface of per-world cached pointers (`EventReaderState`'s
  `NonNull<EventBuffer<E>>`, `QueryState` generation collisions) at the single
  entry point.
- **Hooks are process-global per type; observers are per-world** (by design —
  hooks belong to the component type's definition, observers to a world). The
  H1 staleness gate in `register_component_hooks` is therefore process-global:
  it panics if the component was ever archetyped in ANY world (the
  `EVER_ARCHETYPED` bitmask, set at both archetype-mint funnels in
  [archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs)).
- **`Entity` is NOT world-tagged** (Bevy parity, 8-byte handle): an
  out-of-range foreign handle reads as absent; a colliding `(id, generation)`
  handle silently resolves to the local world's row — documented behavior,
  pinned in [tests/multi_world.rs](../crates/boyko_ecs/tests/multi_world.rs).
- **Shared-pool event-lane contract (H4)** — Apps sharing one `ThreadPool`
  must preregister event types with
  `EventConfig::default_for(worker_count + 1)` so every worker lane plus the
  dispatcher lane is in range in each world.
- SubApp / extract-style world-to-world transfer = future work.

Suite: [tests/multi_world.rs](../crates/boyko_ecs/tests/multi_world.rs);
results: [PHASE-21-RESULTS.md](archive/PHASE-21-RESULTS.md).

---

## 7. Bundle subsystem ✅

**Files:** [core/bundle/](../crates/boyko_ecs/src/ecs/core/bundle/).

The typed multi-component spawn payload (Phases 8d / 8.5 / 11).

```rust
// bundle/bundle.rs:417
pub trait Bundle: sealed::BundleSealed + Send + Sync + Unpin + 'static {
    fn component_ids() -> &'static [ComponentId];
    fn for_each_component_bytes(self, f: impl FnMut(ComponentId, &[u8]));
    // ...
}
```

- `#[derive(Bundle)]` over a **named struct** (tuple bundles were dropped in
  Phase 8.5 so the column cache has a stable per-type address).
- `BundleTypeId` (lazy) + `MAX_BUNDLE_TYPES = 1024`
  ([bundle_type_registry.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle_type_registry.rs):84).
- `BundleColumnCache`
  ([bundle_column_cache.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs))
  caches `(BundleTypeId → ArchetypeId, &'static [InlandPoolId])` per world —
  sub-nanosecond warm spawn lookups. `Unpin` + `Send` pins are asserted at
  compile time via `static_assertions` (SBO-UNPIN).

See [PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md](archive/PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md)
+ [PHASE-12.5-RESULTS.md](archive/PHASE-12.5-RESULTS.md).

---

## 8. Query subsystem ✅

The typed `Query<D, F>` DSL (Phase 8b) plus the chunked/parallel extensions
(Phases 9 + X.A). Module: [core/iters/query/](../crates/boyko_ecs/src/ecs/core/iters/query/)
(see [mod.rs](../crates/boyko_ecs/src/ecs/core/iters/query/mod.rs) for the
re-export surface). The Phase-5c `QueryState` archetype-match cache and
`ArchetypeBitSet` live one level up in [core/iters/](../crates/boyko_ecs/src/ecs/core/iters/).

### 8.1. Query / QueryView

```rust
// query/query.rs:62
pub struct Query<'w, 's, D: QueryData, F: QueryFilter = ()> { /* SystemParam */ }
// query/query_view.rs:83
pub struct QueryView<'w, D: QueryData, F: QueryFilter = ()> { /* direct API */ }
```

`Query<D, F>` is a `SystemParam`; `QueryView<D, F>` is the direct-API mirror via
`EcsMaster::query::<D, F>()`. Iteration:
- `for x in &q` / `for x in &mut q` (IntoIterator; `&q` gated by
  `ReadOnlyQueryData`) → `QueryIter` / `QueryIterMut`
  ([query/iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/iter.rs):95/:410).
- The hot row walk is one `column.ptr.add(row * stride)` + deref per element
  (byte-identical to Bevy in asm — see [PHASE-12.6-RESEARCH-QUERY-BEAT.md](archive/PHASE-12.6-RESEARCH-QUERY-BEAT.md)).

### 8.2. QueryData / QueryFilter

**Files:** [query/data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data.rs),
[query/filter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter.rs).

```rust
pub unsafe trait QueryData: Sized { /* data.rs:86 — GAT-based fetch */ }
pub unsafe trait ReadOnlyQueryData: QueryData {}  // data.rs:428
pub unsafe trait QueryFilter: Sized { /* filter.rs:75 */ }
```

- Data leaves: `&T`, `&mut T`, `Ref<T>` ([data/ref_.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data/ref_.rs):25), `Mut<T>` ([data/mut_.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data/mut_.rs):30),
  `()`, tuples 1..=12. `Ref`/`Mut` carry change-detection (§12); `Mut`'s
  deref-guard bumps the changed tick.
- Filters: `With<C>` ([filter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter.rs):513), `Without<C>` (693), `Added<C>` (863),
  `Changed<C>` (1253), `Or<F>` (1535), tuples.
  `set_table_readonly` /
  `set_table_mut` are split (Phase 8b M2 — no `*const → *mut` cast). The
  `*Fetch`/`*State` GAT structs accompany each leaf.
- `Tick` filters (`Added`/`Changed`) are non-archetypal; the iterator
  monomorphises against `IS_ARCHETYPAL` to const-fold the predicate.

### 8.3. Chunked + parallel iteration

| API | File | Notes |
|-----|------|-------|
| `for_each_chunk(\|slice\|)` (seq) | [query/chunk_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs) | one contiguous columnar slice per matched archetype (flecs shape, Phase X.A) |
| `par_for_each_chunk(\|slice\|, BatchingStrategy)` | [query/par_chunk.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs) | sub-archetype-range fan-out via `boyko_threadpool::scope` |
| `par_iter()` / `par_iter_mut()` (per-row) | [query/par_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs) | `ParQuery` (138) / `ParQueryMut` (206); `MIN_ARCHETYPE_FOR_PARALLEL` (73) |
| `ChunkedQueryData` bound | [query/chunked_data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs):72 | `&T`/`&mut T`/`()` + tuples; `Changed`/`Added`/`Ref`/`Mut` excluded at compile time |
| `ArchetypalQueryFilter` bound | [query/filter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter.rs):2512 | `With`/`Without`/`Or`/tuples |
| `BatchingStrategy` | [query/par_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs):83 | chunk-size policy |

`for_each_chunk` lands a credible multi-component SIMD win (boyko 1.28–1.34×
Bevy, native-SIMD) — the 5× headline is filed as Phase X.A.2. See
[PHASE-X.A-RESULTS.md](archive/PHASE-X.A-RESULTS.md).

### 8.4. QueryState cache + dedup bitset (Phase 5c)

**Files:** [core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs),
[core/iters/archetype_bit_set.rs](../crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs).

```rust
#[repr(C, align(64))]
pub struct QueryState {
    generation: ArchetypeGeneration,
    structural_generation: ArchetypeGeneration,
    matched_ids: Vec<ArchetypeId>,
    include: ComponentMask, exclude: ComponentMask, optional: ComponentMask,
    matched_archetypes: ArchetypeBitSet,         // 1024-bit dedup ([u64; 16], inline)
}
```

Warm path (both gens unchanged): pure slice walk over `matched_ids`. Cold paths:
structural-mismatch full rebuild (drop bitset, reclassify every live archetype)
or creation-only delta. Safe across `master.clear()` / `remove_archetype()` —
the structural counter triggers the rebuild automatically (the ArchetypeId-ABA
fix). `ArchetypeBitSet::insert`/`contains` panic (release-included) when
`id >= MAX_ARCHETYPES = 1024`. The per-system typed cache
`QueryDataState<D, F>` ([query/state.rs](../crates/boyko_ecs/src/ecs/core/iters/query/state.rs):47)
wraps this.

### 8.5. LegacyQuery — RETIRED

`core/iters/legacy_query.rs` no longer exists. The pre-Phase-8b
archetype-yielding query (`iter_one`/`iter_two`/`with_component_ids`/
`with_mask`/…) was kept for back-compat until `refactor(ecs): retire the legacy
query stack` (400693d) deleted it; every caller now uses the typed
`Query<D, F>` of §8.1. It was deliberately **term-free** — no `with_tag`
surface ever existed on it (Phase 22 D4).

What outlived it is the `ComponentSet` trait
([core/iters/component_set.rs](../crates/boyko_ecs/src/ecs/core/iters/component_set.rs)),
which returns `&'static [ComponentId]` (Q-012) for tuple types.

### 8.6. Dynamic tag terms + the `_pre_terms` accessor funnel (Phase 22 D4) ✅

**File:** [query/tag_terms.rs](../crates/boyko_ecs/src/ecs/core/iters/query/tag_terms.rs)

```rust
pub const MAX_DYN_TAG_TERMS: usize = 8;                  // tag_terms.rs:42
pub(crate) struct TagTerms {                             // tag_terms.rs:51
    ids: [TagId; MAX_DYN_TAG_TERMS],
    polarity: u8,    // bit i: 1 = with, 0 = without
    len: u8,
}
pub(crate) fn archetype_passes_tag_terms(&TagTerms, &Archetype) -> bool; // :150
```

`Query::with_tag`/`without_tag`
([query/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query.rs):138/:148)
and the `QueryView` mirrors
([query/query_view.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs):249/:259)
push into a per-view, stack-only, `Copy` `TagTerms`. The shared interned
`QueryState` is NEVER mutated by terms (QS1 stays term-agnostic). >8 terms =
loud release panic at term-add time (`tag_terms_overflow_panic`,
[tag_terms.rs](../crates/boyko_ecs/src/ecs/core/iters/query/tag_terms.rs):211).
`archetype_passes_tag_terms` is THE single term test, applied at each driver's
**archetype-transition point** (outside the row loop): ≤8 signature-bit tests;
`len == 0` = one predicted not-taken branch; inner row loop byte-identical
(asm-gated). Term-aware `len`/`is_empty` route through `count_term_matched` /
`any_term_matched` ([tag_terms.rs](../crates/boyko_ecs/src/ecs/core/iters/query/tag_terms.rs):195 — archetype-level
membership, no `entity_count`; the per-archetype gate is
`archetype_passes_tag_terms` :150).

**The `_pre_terms` funnel contract** —
[core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs):
every accessor exposing the raw matched list carries the `_pre_terms` suffix:
`iter_pre_terms` (:156), `len_pre_terms` (:380), `is_empty_pre_terms` (:391),
`matched_ids_pre_terms` (:400), `iter_cached_pre_terms` (:431),
`matched_ids_pre_terms_mut` (:483, the QS1 cache-maintenance writer). Outside
`query_state.rs`, every read of the matched list passes through a
`_pre_terms`-named symbol — a future driver cannot silently bypass terms
without consciously typing `_pre_terms` (the module-boundary comment at :59~
pins the inside: the private field is owned by QS1 cache maintenance, which is
pre-terms by definition). This converts the Phase-14b enumeration-by-memory
failure mode into a compile error at the crate-visible boundary.

Consumers (the D4 disposition table, all migrated): the
`QueryIter` / `QueryIterMut` constructors ([iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/iter.rs):95/:410), the
par distribution loops `for_each_impl` / `run_chunk_inline` ([par_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs):286/:602),
`for_each_chunk_impl` ([chunk_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs):97),
`par_for_each_chunk_impl` ([par_chunk.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs):104), and `Query`/`QueryView`
`len`/`is_empty`/`get`/`get_mut`/`single`. Per-driver behavioral suite:
[tests/phase22_query_terms.rs](../crates/boyko_ecs/tests/phase22_query_terms.rs).

---

## 9. SystemParam + Resources + IntoSystem ✅

Files: [core/system/](../crates/boyko_ecs/src/ecs/core/system/),
[core/resources/](../crates/boyko_ecs/src/ecs/core/resources/).

### 9.1. System + IntoSystem + FunctionSystem

```rust
// system/system.rs:57
pub unsafe trait System: Send + Sync + 'static {
    type Out;                                              // :60
    fn name(&self) -> &'static str;                        // :65
    fn access(&self) -> &Access;                           // :70
    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> Self::Out;  // :95
    fn apply(&mut self, _world: &mut EcsMaster) {}         // :165 (safe; flushes deferred state)
    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick);  // :240 (Phase 10 C1)
    fn check_change_tick(&mut self, current: Tick);        // Phase 16.1 Gap #2 (no default body)
}
```

- `IntoSystem<In, Out, Marker>`
  ([system/into_system.rs](../crates/boyko_ecs/src/ecs/core/system/into_system.rs):47)
  converts any `fn(P0..Pn)` (with `Pi: SystemParam`) into a runnable `System`
  without turbofish (Phase 8c). Markers: `IsFunctionSystem` (67),
  `ExclusiveSystemMarker` (154, for `fn(&mut EcsMaster)`), plus the Phase-17 IS2
  identity blanket (`impl<S: System> IntoSystem<.., S> for S`) so a pre-built
  `System` (run conditions) routes through `.run_if`.
- `FunctionSystem` + `SystemParamFunction`
  ([system/function_system.rs](../crates/boyko_ecs/src/ecs/core/system/function_system.rs):52,
  [system/function_system_impls.rs](../crates/boyko_ecs/src/ecs/core/system/function_system_impls.rs))
  cache `<F::Param as SystemParam>::State` + `SystemMeta` across invocations.
  `ExclusiveFunctionSystem`
  ([system/exclusive_function_system.rs](../crates/boyko_ecs/src/ecs/core/system/exclusive_function_system.rs))
  wraps `fn(&mut EcsMaster)`.

> **Known semantic (NOT a bug) — one-shot runners and the `Changed`/`Added`
> window** (pre-existing; surfaced by Phase 22 Wave 2B). `run_system` /
> `run_cached_system` never call `System::set_change_ticks` — the sequence is
> initialize → `run_unsafe` → `apply` → drain
> ([ecs_master/system_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/system_api.rs):111/:142).
> Both tick snapshots therefore stay at the `SystemMeta::new` pre-first-run
> sentinel `current_tick - MAX_CHANGE_AGE`
> ([system_meta.rs](../crates/boyko_ecs/src/ecs/core/system/system_meta.rs):172~-188).
> A one-shot system's `Added<T>`/`Changed<T>` observation window is the full
> clamped `MAX_CHANGE_AGE` span — "everything recently stamped" — NOT
> "since this system's previous run"; the narrowing `set_change_ticks`
> channel exists only in the `Schedule::run` dispatcher (Phase 10 C1).
> Tests that need real windows must drive a `Schedule`.

### 9.2. SystemParam + the worker cell

```rust
// system/system_param.rs:78
pub unsafe trait SystemParam: Sized {
    type State; type Item<'w, 's>;
    fn init_state(world: &mut EcsMaster, meta: &mut SystemMeta) -> Self::State;
    fn init_access(state: &Self::State, set: &mut FilteredAccessSet, ...);  // two-phase (Phase 8a C4)
    unsafe fn get_param<'w, 's>(...) -> Self::Item<'w, 's>;
    fn apply(state: &mut Self::State, world: &mut EcsMaster) {}
}
```

Tuple impls 0..=12 in
[system/params/tuple_impl.rs](../crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs)
(the tuple `apply` forwarder bug from Phase 8d was fixed — Commands no longer
silently no-op). Leaf params:

| Param | File | Notes |
|-------|------|-------|
| `Res<'w, R>` | [params/res.rs](../crates/boyko_ecs/src/ecs/core/system/params/res.rs):40 | shared resource read |
| `ResMut<'w, R>` | [params/resmut.rs](../crates/boyko_ecs/src/ecs/core/system/params/resmut.rs):42 | exclusive resource write |
| `Local<'s, T>` | [params/local.rs](../crates/boyko_ecs/src/ecs/core/system/params/local.rs):62 | per-system state (Phase 13) |
| `Commands<'s>` | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):97 | deferred mutation (§10) |
| `Query<'w, 's, D, F>` | [iters/query/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query.rs):62 | typed query (§8) |
| `EventReader<'s, E>` / `EventWriter<'s, E>` | [params/event_reader.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs):87 / [event_writer.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_writer.rs):89 | events (§14) |

- `UnsafeEcsCell<'w>`
  ([system/unsafe_ecs_cell.rs](../crates/boyko_ecs/src/ecs/core/system/unsafe_ecs_cell.rs))
  — `Copy` raw-pointer newtype with **by-value method receivers** (Phase 8a C1
  — eliminates the `&self` retag).
- `SystemMeta`
  ([system/system_meta.rs](../crates/boyko_ecs/src/ecs/core/system/system_meta.rs))
  carries `Access` + `name` + `last_run`/`this_run: Tick`. `Access` +
  `FilteredAccessSet`
  ([system/filtered_access_set.rs](../crates/boyko_ecs/src/ecs/core/system/filtered_access_set.rs))
  do cross-system (Phase 9) and intra-system (Phase 8a C4) conflict detection.

### 9.3. Resources storage

**Files:** [core/resources/](../crates/boyko_ecs/src/ecs/core/resources/).

```rust
// resources/resources.rs:100
pub struct Resources { /* MaybeUninit slab + BitSet256 registered_mask */ }
```

`insert::<R>` (154) / `remove::<R>` (252) / `contains::<R>` (370). The replace +
remove + Drop paths use the **clear-bit-first protocol** (Phase 8a C3 — clear
the registered bit BEFORE running `drop_fn`, so a panic-in-drop leaks rather than
UBs). `trait Resource: Send + Sync + 'static`
([resource.rs](../crates/boyko_ecs/src/ecs/core/resources/resource.rs)); lazy ids
via [resource_registry.rs](../crates/boyko_ecs/src/ecs/core/resources/resource_registry.rs)
(`RESOURCE_SLOT_COUNT = 256`, line 51). A type cannot be both Component and
Resource (audit M6). ZST resources are guarded.

---

## 10. Commands — deferred mutation ✅

**Files:** [core/system/params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs),
[core/commands/](../crates/boyko_ecs/src/ecs/core/commands/).

A per-system byte-arena queue flushed via `SystemParam::apply` after the body
returns. No `Box<dyn Command>`; commands memcpy into a `Vec<MaybeUninit<u8>>`.

```rust
pub struct Commands<'s> { /* commands.rs:97 */ }
```

| Method (line) | Effect |
|---------------|--------|
| `spawn(bundle) -> EntityCommands` (164) | reserve id, queue `SpawnAtCommand`; chainable |
| `entity(entity) -> EntityCommands` (228) | address an existing entity |
| `despawn(entity)` (251) | queue `DespawnCommand` |
| `spawn_batch(iter)` (313) | queue `SpawnBatchCommand` |
| `add::<C: Command>(cmd)` (125) | queue a custom `Command` |

- `EntityCommands<'a, 's>`
  ([params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs):80)
  — `.insert(..)`, `.remove::<C>()`, `.despawn()`, `.id()` (Phase 11 chaining).
- Entity-id reservation uses an atomic counter via `EntityCounter`
  ([params/entity_counter.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_counter.rs):75,
  Phase 11 Path A).
- Command structs in [core/commands/](../crates/boyko_ecs/src/ecs/core/commands/):
  `SpawnAtCommand`, `InsertCommand`, `RemoveCommand`, `DespawnCommand`,
  `SpawnBatchCommand`, `SendEventCommand`, plus `migration_helpers.rs`
  (archetype migration on insert/remove). `CommandQueue`
  ([commands/command_queue.rs](../crates/boyko_ecs/src/ecs/core/commands/command_queue.rs))
  uses a `CursorSync` RAII guard for Bevy-mirror panic recovery (Phase 12.5/12.6
  Opt-A1).
- The `migrate_entity_insert` path was rewritten in Phase 14b to consume bundle
  bytes INSIDE the `for_each_component_bytes` closure (fixing a dangling-slice
  UAF that Miri caught — see [PHASE-14B-RESULTS.md](archive/PHASE-14B-RESULTS.md)).

See [PHASE-8CD-INTOSYSTEM-COMMANDS-PLAN.md](archive/PHASE-8CD-INTOSYSTEM-COMMANDS-PLAN.md)
+ [PHASE-11-ENTITY-COMMANDS-PLAN.md](archive/PHASE-11-ENTITY-COMMANDS-PLAN.md).

---

## 11. Schedule + parallel scheduler ✅

A Bevy-class multi-system executor (Phase 9) on the custom
[`boyko_threadpool`](#18-boyko_threadpool-). Conflict graph + Tarjan SCC + Kahn
topo + apply-window barrier. Module: [core/schedule/](../crates/boyko_ecs/src/ecs/core/schedule/).

### 11.1. Schedule + ScheduleBuilder

```rust
// schedule/schedule.rs:92
pub struct Schedule {
    pool: Arc<ThreadPool>,
    systems: Vec<SystemBox>,                  // topo order, stable addresses
    conflict_graph: ConflictGraph,
    executor_scratch: ExecutorScratch,
    has_condition: FixedBitSet,               // Phase 16 0%-gate
    system_conditions: Vec<Vec<BoolSystem>>,  // Phase 16
    system_gating_sets: Vec<Box<[SystemSetId]>>,
    set_conditions: Vec<SetConditionEntry>,
    state_entries: Vec<StateEntry>,           // Phase 17 0%-gate (LAST field)
}
```

- `ScheduleBuilder::new(Arc<ThreadPool>)` ([schedule_builder.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs):149);
  `add_system(system) -> SystemConfig` (176);
  `build(&mut world) -> Schedule` (323) / `try_build(...)` (349, returns
  `Result<_, ScheduleBuildError>`).
- `Schedule::run(&mut world)`
  ([schedule.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule.rs):231) —
  bumps the change tick, runs the state-transition pass, then dispatches.
- Uses external dep `fixedbitset` for the conflict/condition bitsets.

### 11.2. Executor internals

| Type | File | Role |
|------|------|------|
| `ConflictGraph` | [conflict_graph.rs](../crates/boyko_ecs/src/ecs/core/schedule/conflict_graph.rs) | per-system conflict bitsets + ordering DAG (pred/succ) |
| `ExecutorScratch` | [executor_scratch.rs](../crates/boyko_ecs/src/ecs/core/schedule/executor_scratch.rs) | per-frame `pred_remaining` / `running` / `completed`; out-of-line completion channel behind `NonNull` (Phase 9.3c TB fix) |
| `SystemBox` / `BoolSystem` | [system_box.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_box.rs) | 1-cache-line `Out=()` hot slot + erased `bool` condition |
| `bitset_intersects` | [bitset_intersects.rs](../crates/boyko_ecs/src/ecs/core/schedule/bitset_intersects.rs) | conflict-bit intersection helper |

The dispatcher loop: drain pending applies under the barrier (gate proves
`running == 0` so `&mut world` aliases no worker cell) → find ready
(`pred_remaining == 0`, no conflict against running) → spawn on the pool's
`Scope` → park with a 100 µs backstop. **Soundness:** proven via loom + Miri
(Phase 9.1/9.2/9.3); structural allocation (frontier commits, container growth)
stays restricted to the dispatcher + build via the ALLOC1 TLS discipline. See
[PHASE-9-PARALLEL-SCHEDULER-PLAN.md](archive/PHASE-9-PARALLEL-SCHEDULER-PLAN.md),
[PHASE-9.2-RESULTS.md](archive/PHASE-9.2-RESULTS.md), [PHASE-9.3c-RESULTS.md](archive/PHASE-9.3c-RESULTS.md).

### 11.3. System ordering & sets (Phase 15)

**Files:** [schedule/system_config.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_config.rs),
[schedule/system_set.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_set.rs),
[schedule/ordering.rs](../crates/boyko_ecs/src/ecs/core/schedule/ordering.rs).

- `SystemConfig::{in_set, before, after, before_set, after_set}` (value-based) +
  `ScheduleBuilder::configure_set(set) -> ConfigureSet`
  ([schedule_builder.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs):216)
  (`.before`/`.after`/`.in_set` + set hierarchy).
- `SystemSetId` (`#[repr(transparent)] usize`) is interned from `(TypeId,
  discriminant())` via a single `set_id_of_value` path so config and membership
  resolve to the SAME id (the Phase 15 C1 crux). `#[derive(SystemSet)]` covers
  fieldless enums.
- Build expands set membership to transitive leaves, then to pairwise
  `(SystemKey, SystemKey)` edges feeding the EXISTING Tarjan-SCC + Kahn-topo
  pipeline (an ordering edge forces serial execution via a "false conflict" bit
  — Phase 9 scaffold; Phase 15 finished "Wave 5 Step 14").
- Diagnostics: `ScheduleBuildError` (`OrderingCycle` B9001, `SetHierarchyCycle`
  B9002, `SetsOrderedButIntersect` B9004, `UnknownSystemKey` B9005) + warning
  `boyko-W1501` for a never-joined ordered set.

See [PHASE-15-RESULTS.md](archive/PHASE-15-RESULTS.md).

### 11.4. Run conditions (Phase 16)

**File:** [schedule/common_conditions.rs](../crates/boyko_ecs/src/ecs/core/schedule/common_conditions.rs).

- `.run_if(cond)` on systems + sets, where `cond: impl IntoSystem<(), bool, M>`
  (erased as `BoolSystem = Box<dyn System<Out = bool>>`; the `Out=()`
  `SystemBox` hot slot is untouched).
- Built-ins: `run_once`, plus the state conditions `in_state` / `on_enter` /
  `on_exit` / `on_transition` (§13). Multiple `.run_if` AND-fold eagerly (no
  short-circuit, so stateful conditions advance).
- Executor integration: a separate `evaluate_ready_conditions` pass at the
  apply-window boundary (gated on `running.count_ones() == 0` → race-free). A
  false fold marks the system completed + decrements successors' `pred_remaining`
  WITHOUT running the body (skip-but-keep-dependents). The 0%-gate is the
  `has_condition` bitset `is_clear()` early-out (`try_dispatch_ready`
  byte-identical). Conditions are pure predicates (no `apply`).
- Tick-aware conditions (`Changed`/`Added`/`Ref`) are CORRECT since Phase 16.1:
  `run_condition(cond, this_run)` checkpoints the condition's
  `(last_run, this_run]` window at the eval site, only on a frame it is
  actually evaluated (Bevy "since-last-actual-run" parity — a dormant condition
  resumes seeing ALL changes accrued while dormant). There is NO frame-start
  condition bump. `Schedule.frame_this_run` (set once at the top of `run`) is
  the tick source — not `world.current_tick()`, which reads `this_run + 1`
  after the #56 apply-window bump.

See [PHASE-16-RESULTS.md](archive/PHASE-16-RESULTS.md) +
[PHASE-16.1-RESULTS.md](archive/PHASE-16.1-RESULTS.md).

---

## 12. Change detection ✅

Bevy-style per-row tick storage (Phase 10). Module:
[core/change_detection/](../crates/boyko_ecs/src/ecs/core/change_detection/).

- `Tick(u32)`
  ([change_detection/tick.rs](../crates/boyko_ecs/src/ecs/core/change_detection/tick.rs))
  — wrapping-safe `is_newer_than(last_run, this_run)` (Round-3 C-NEW-1 fixed the
  transposed-operand formula); `MAX_CHANGE_AGE = u32::MAX - (2 *
  CHECK_TICK_THRESHOLD - 1)`, `CHECK_TICK_THRESHOLD = 518_400_000`.
- Per-row storage: the pool's `[added | changed]` tick sub-regions
  (`added_base`/`changed_base: NonNull<UnsafeCell<Tick>>`, Phase X.I — §2.3).
- `EcsMaster::change_tick: AtomicU32` bumped once per `Schedule::run`; per-system
  `last_run`/`this_run` snapshot on `SystemMeta`, written via
  `System::set_change_ticks` (the C1 single dispatcher→system channel).
  **Phase 16.1 stamp contract:** UNGATED systems (`has_condition[i]` clear) are
  stamped in the frame-start loop (they run every frame, so it is equivalent);
  GATED systems are stamped at their DISPATCH site only on a frame they run
  (concurrent path = pre-pass before the `systems_ptr` raw lift; inline-exclusive
  path = before `run_unsafe`); a skipped frame freezes the ticks, so `Changed<T>`
  body queries observe the full dormant window on resume. Conditions checkpoint
  inside `run_condition` (eval site). `System::check_change_tick` (no default
  body) + `#[cold] Schedule::check_change_ticks` clamp system + own-condition +
  set-condition ticks on the `should_run_check_ticks` cold path (the dormancy
  wraparound guard). **Phase 20 D8/★C1:** `Schedule::check_change_ticks` is now
  `pub(crate)`; an App-level margin-aware pass
  (`CHECK_TICK_PREEMPT_MARGIN = 4096`, `App::check_ticks_all_schedules`) fires
  at frame start strictly BEFORE any schedule's internal block can cross the
  threshold, clamping the world scan + ALL schedules under one tick snapshot —
  without the margin the first internal block to fire would reset the shared
  counter and starve the sibling schedule's clamp. The internal block stays as
  the standalone single-schedule belt.
- Filters `Added<C>` / `Changed<C>` (§8.2); data `Ref<T>` (immutable + flags) /
  `Mut<T>` (deref-guard bumps `changed`). `set_if_neq` / `bypass_change_detection`
  escape hatches.
- Wraparound: `run_check_ticks_scan`
  ([change_detection/check_ticks.rs](../crates/boyko_ecs/src/ecs/core/change_detection/check_ticks.rs))
  clamps live-row ticks every `CHECK_TICK_THRESHOLD` ticks (scans only `count()`
  live rows, not the full buffer — W3).

0% measurable overhead on queries that use no change detection (`NEEDS_CHANGE_DETECTION`
const elision, Phase 12.5 NCD6). See
[PHASE-10-CHANGE-DETECTION-PLAN.md](archive/PHASE-10-CHANGE-DETECTION-PLAN.md).

---

## 13. States ✅

Application/game states layered on the single `Schedule` (Phase 17). Module:
[core/state/](../crates/boyko_ecs/src/ecs/core/state/).

- `trait States: Send + Sync + Clone + PartialEq + Eq + Hash + 'static`
  ([state/states.rs](../crates/boyko_ecs/src/ecs/core/state/states.rs)) —
  hand-impl, no derive.
- Resources `State<S>` (`#[repr(transparent)]` current value,
  [state/state.rs](../crates/boyko_ecs/src/ecs/core/state/state.rs)) and
  `NextState<S>` (`enum { Unchanged, Pending(S) }`,
  [state/next_state.rs](../crates/boyko_ecs/src/ecs/core/state/next_state.rs)).
- A `TypeId`-keyed resource-id registry
  ([resources/resource_type_registry.rs](../crates/boyko_ecs/src/ecs/core/resources/resource_type_registry.rs))
  — the D3 decision avoiding the rust#22991 trap where `#[derive(Resource)]` on a
  generic `State<S>` would alias every `S` onto one slot.
- `StateTransitionRecord<S>` + `apply_state_transition::<S>`
  ([state/transition_record.rs](../crates/boyko_ecs/src/ecs/core/state/transition_record.rs))
  — a per-`S` record of "exited/entered this frame", written by the built-in
  transition pass (`Schedule::run_state_transitions`, once per frame, gated by
  `state_entries.is_empty()` — the 0%-gate twin of `has_condition`).
- Run conditions `in_state` / `on_enter` / `on_exit` / `on_transition`
  ([schedule/common_conditions.rs](../crates/boyko_ecs/src/ecs/core/schedule/common_conditions.rs)),
  composing with Phase 16 `.run_if`. The initial `OnEnter(initial)` is
  synthesized once on frame 1.
- Builder entry — [schedule_builder.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs):
  `insert_state` (247) / `init_state` (283).
- World entry — [core/ecs_master/state_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/state_api.rs):
  `insert_state` (33) / `init_state` (51) / `state` (62) / `set_next_state` (85).
- `StateEntry` (type-erased) + `StateTransitionSet` (opt-in ordering hook,
  [state/state_set.rs](../crates/boyko_ecs/src/ecs/core/state/state_set.rs)).
- The Phase-17 IS2 identity `IntoSystem` blanket lets the conditions (returning
  `impl System<Out = bool>`) flow through `.run_if`.

Zero new `unsafe`. See [PHASE-17-RESULTS.md](archive/PHASE-17-RESULTS.md).

---

## 14. Events ✅ (full dispatcher + SystemParam readers/writers)

A double-buffered event dispatcher (Phase 6) plus `EventReader` / `EventWriter`
SystemParam wrappers (Phase 12). **Note:** older revisions of this catalog said
"no dispatcher" — that is stale; the dispatcher exists. Module:
[core/events/](../crates/boyko_ecs/src/ecs/core/events/).

### 14.1. Event trait + `#[event]` macro

```rust
// events/event.rs
pub trait Event: 'static + Sized {
    type Participants: Participants;
    type Parameters: Parameters;
    fn event_id() -> EventId; /* + new / participants[_mut] / parameters[_mut] / layout / type_id */
}
```

The `#[event]` macro (Q-001) rewrites a user struct with `#[participant(...)]` /
`#[parameter]` fields into a two-field `{ participants, parameters }` native
layout + the `Event` impl (safe typed-field accessors — no UB cast). ZST events
are rejected at compile time (`ZstCheck`).

### 14.2. EventDispatcher + EventBuffer

**Files:** [events/event_dispatcher.rs](../crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs),
[events/event_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/event_buffer.rs),
[events/event_config.rs](../crates/boyko_ecs/src/ecs/core/events/event_config.rs).

- `EventDispatcher` — one 64-byte-aligned `EventTypeSlot` per registered event
  type, each carrying a type-erased `EventVTable { swap_fn, drop_fn, type_id }`
  (no `dyn Trait`). API in
  [event_dispatcher.rs](../crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs):
  `send_event::<E>(event)` (274, reads the worker id from TLS);
  `send::<E>(thread_index, event)` (292, low-level);
  `update_events()` (436, per-frame swap of write lanes into the read buffer).
- `EventBuffer<E>` — split cache-line lanes (`#[repr(C)]` + `CachePadded`,
  Phase 12 false-sharing fix C3): `frame_event_count` on CL0; reader fields on
  CL1; per-thread write lanes on CL2+. `MAX_EVENT_THREADS = 64`,
  `MAX_EVENT_CAPACITY = 16384` (constants).
- Events sit OUTSIDE the conflict graph (Option A) — parallel writers of the
  same `E` are OK via per-lane TLS routing (strictly more permissive than Bevy).

### 14.3. EventReader / EventWriter (Phase 12 SystemParams)

**Files:** [system/params/event_reader.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs),
[system/params/event_writer.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_writer.rs).

- `EventReader<'s, E>` ([event_reader.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs):87) — caches
  `NonNull<EventBuffer<E>>` + thread_count + a per-system cursor; yields via
  `EventIter<'a, E>` (245) which checkpoints the cursor on partial iteration
  (drop). The cached pointer is anchored to a `Box::into_raw` buffer (stable
  across `&mut dispatcher` reborrows — Phase 12 C1).
- `EventWriter<'s, E>` ([event_writer.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_writer.rs):89) — caches the
  same buffer pointer; routes writes to the calling worker's lane.

### 14.4. Participants / Parameters

**Files:** [events/participants/](../crates/boyko_ecs/src/ecs/core/events/participants/),
[events/parameters/](../crates/boyko_ecs/src/ecs/core/events/parameters/).

Per-event-type typed buffers (`Vec<MaybeUninit<u8>>`) with a `TypeId` guard on
every typed `get`/`push` (Q-019). The split is retained (Q-020 deferred — no
participant-filtered dispatch use case yet).

### 14.5. EventRegistry (global)

**File:** [events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs)

`static [OnceLock<EventInfo>; MAX_EVENTS]` (mirror of `component_registry`);
lazy `event_id()`; `MAX_EVENTS = 256` (line 51). Same startup warm-up contract as
ComponentId.

See [PHASE-6-EVENT-DISPATCH-PLAN.md](archive/PHASE-6-EVENT-DISPATCH-PLAN.md) +
[PHASE-12-EVENTS-SYSTEMPARAM-PLAN.md](archive/PHASE-12-EVENTS-SYSTEMPARAM-PLAN.md).

---

## 15. App + Plugin facade ✅

The Phase-18 builder over `EcsMaster` + `ScheduleBuilder` + `Schedule` +
`ThreadPool`, extended in Phase 20 into a **multi-schedule frame driver**
(Main + Fixed) with a `Time`/`FixedTime` clock family. Modules:
[core/app/](../crates/boyko_ecs/src/ecs/core/app/) +
[core/time/](../crates/boyko_ecs/src/ecs/core/time/); re-exported at the crate
root (`boyko_ecs::{App, Plugin, Plugins, AppExit}`) and in the
[prelude](../crates/boyko_ecs/src/prelude.rs) (which adds `CoreSchedule`,
`EventUpdatePolicy`, `Time`, `FixedTime`, `fixed_advance`).

```rust
// app/plugin.rs
pub trait Plugin: 'static {                 // NOT Send + Sync — consumed at build
    fn build(&self, app: &mut App);
    fn name(&self) -> &'static str { type_name::<Self>() }
}
```

`App` ([app/app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs)) owns `world:
EcsMaster` + `pool: Arc<ThreadPool>` + staged `Option<ScheduleBuilder>` →
`Option<Schedule>` pairs for BOTH `CoreSchedule`s (Main + a lazily-created
Fixed — Phase 20 D5: a closed enum matched only in config methods; the frame
driver reads direct named fields, zero dispatch, no label map) + a one-shot
startup list + a `Vec<TypeId>` plugin-dedup set. Constructors `new` /
`with_threads` / `with_pool`. Config: `add_plugin` / `add_plugins((..))`
(sealed `Plugins` trait,
[app/plugins.rs](../crates/boyko_ecs/src/ecs/core/app/plugins.rs), 1..=12 +
nesting), `insert_resource`, `init_state` / `insert_state`, `add_systems_cfg` /
`add_systems`, each with a `*_in(CoreSchedule, …)` routing form (Phase 20),
`add_startup_system`, `set_fixed_timestep` / `set_fixed_hz`,
`set_event_update_policy`; every config method panics loudly (`boyko-B1802`)
after `finish()`. Runner: `update` (self-clocked via `Instant`, first frame =
ZERO delta), `update_with_delta(raw)` — THE Phase-20 frame driver (binding
order D1: ① `Time::advance_with` → ② margin-aware all-schedule check-ticks
pass → ③ gated event swap (D6: held under `EventUpdatePolicy::WaitForFixed`
until ≥ 1 substep since the last swap) → ④ `fixed_advance` catch-up loop →
⑤ Main run), `run_n`, `run_n_with_delta` (the deterministic loop every TIMED
artifact routes through), `run` (loops until a system sets
`ResMut<AppExit>(true)`). Driver cost: 14 ns/frame envelope + 5 ns/substep
(P20-B1(b)/B2). `App` is `!Send + !Sync`.

**Time module** ([core/time/](../crates/boyko_ecs/src/ecs/core/time/),
Phase 20 D2/D3/D4): `Time`
([time/time.rs](../crates/boyko_ecs/src/ecs/core/time/time.rs)) — the virtual
frame clock (250 ms inflow clamp, `relative_speed`, `pause`; real unclamped
fields carried alongside) — and `FixedTime`
([time/fixed_time.rs](../crates/boyko_ecs/src/ecs/core/time/fixed_time.rs)) —
`timestep` (default exactly 64 Hz), `overstep` (THE accumulator),
`overstep_fraction()` (THE interpolation alpha), `elapsed`,
`steps_this_frame`. NO Bevy-style generic-`Time` swap — two plain resources,
seeded by `finish()` if absent. `pub fn fixed_advance(world, step)`
([time/fixed_loop.rs](../crates/boyko_ecs/src/ecs/core/time/fixed_loop.rs)) is
the ONE shared catch-up driver: App, the wasm demo runner, and Miri tests
traverse the identical integer-ns accumulate/expend path (timestep snapshotted
at loop entry, ★M3; ≤ 16 substeps/frame at the defaults). Step counts are
bit-deterministic for a given dt script (P20-B4). See
[PHASE-20-RESULTS.md](archive/PHASE-20-RESULTS.md).

`AppExit(bool)` ([app/app_exit.rs](../crates/boyko_ecs/src/ecs/core/app/app_exit.rs))
**hand-impls `Resource`** — the derive is unusable inside `boyko-ecs` lib code
because `boyko-macros` is only a dev-dependency (the macro-cycle constraint;
the prelude likewise omits the derives). See [PHASE-18-RESULTS.md](archive/PHASE-18-RESULTS.md).

---

## 16. Error handling ✅

**File:** [crates/boyko_ecs/src/ecs/error.rs](../crates/boyko_ecs/src/ecs/error.rs)

```rust
#[non_exhaustive]
pub enum EcsError {
    ArchetypeNotFound(ArchetypeId),
    EntityNotFound(EntityId),
    ComponentPoolFull { component_id: ComponentId },
    UnknownComponentForArchetype { archetype_id: ArchetypeId, component_id: ComponentId },
    ArchetypeRejectedEntity { archetype_id: ArchetypeId },
    PoolSwapRemoveFailed,
    // + event-dispatch variants (Phase 6)
}
pub type EcsResult<T> = Result<T, EcsError>;
```

Hand-rolled `Display` + `std::error::Error` (no `thiserror`). The `anyhow`
dependency was dropped (C-019). Re-exported at the crate root as
`boyko_ecs::{EcsError, EcsResult}`. `#[non_exhaustive]` so new variants land
without a major-version bump.

---

## 17. boyko_utils ✅

Files under [crates/boyko_utils/src/](../crates/boyko_utils/src/).

### 17.1. BitSet + BitSet256

- `BitSet<T: BitInteger>`
  ([bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs)) — generic
  over the backing integer (u8 / u32 / u64); `set` / `unset` / `contains` / iter
  / bitwise combinators.
- `BitSet256` ([bit_mask/bit_set_256.rs](../crates/boyko_utils/src/bit_mask/bit_set_256.rs))
  — fixed 256-bit set + `pop_lowest_set_bit` (Phase 6); backs the resource
  `registered_mask` and event lane masks.

The historical `bit_mask.rs` / `bit_set512.rs` / `bit_storage.rs` (1080 LOC of
commented-out dead code) were deleted (M-010).

### 17.2. SparseMap / SparseSlotMap

- `SparseMap<U>`
  ([sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs))
  — `{ sparse, dense, indices }`; `active_indices()` / `iter_dense()` for
  O(active) iteration. Used by `ArchetypeBundle` + `ArchetypeRegistry`;
  `EntityMaster` moved off it in Phase 7.
- `SparseSlotMap<U>`
  ([sparse_map/sparse_slot_map.rs](../crates/boyko_utils/src/sparse_map/sparse_slot_map.rs))
  — generation-tracked, keyed by `Slot { index, generation }`. `remove` writes a
  tombstone with `generation.wrapping_add(1)` so stale slots are rejected by
  `contains`/`get`/`insert` (M-016 ABA fix). 9 dedicated tests.
- `SparseCollection<K, V>`
  ([sparse_map/sparse_collection.rs](../crates/boyko_utils/src/sparse_map/sparse_collection.rs))
  — the shared trait abstraction.

### 17.3. Slot

[identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs) — `{ index,
generation }`; the shared key for sparse-set / slot-map structures.

---

## 18. boyko_threadpool ✅

**Crate:** [crates/boyko_threadpool/](../crates/boyko_threadpool/) — files
`thread_pool.rs`, `scope.rs`, `worker.rs`, `tls.rs`, `sync.rs`,
[lib.rs](../crates/boyko_threadpool/src/lib.rs).

A custom Chase-Lev work-stealing pool built **directly on
`crossbeam_deque::{Worker, Stealer, Injector}`** primitives (NOT rayon);
everything above (worker threads, parking, scope, panic propagation, install
API) is hand-rolled to fit the scheduler's contracts. Exports:

- `ThreadPool` / `ThreadPoolBuilder` / `WorkerHandle` / `PoolInner` /
  `TaskHandle` / `MAX_WORKERS` (lib.rs:61).
- `Scope` (lib.rs:61) — `Scope::spawn` with `'scope` lifetime erasure;
  `Scope::Drop` blocks via *work-stealing* (rayon pattern) so nested scopes can't
  deadlock. `install` (dispatcher TLS bookkeeping) vs `scope` (worker-safe,
  lighter; used by `par_iter` / `par_for_each_chunk`).
- TLS (lib.rs:65): `current_worker_id`, `WORKER_ID_DISPATCHER` /
  `WORKER_ID_UNATTACHED`, `InSystemRunGuard` (the ALLOC1/ALLOC6 guard — the
  ECS crate's context-restricted paths `debug_assert!` it or its negation:
  event lane routing, the hook-drain SAFETY-7 gate), `try_with_active_pool`.

`ThreadPool::drop` joins workers (Phase 9.3b: split handle + `Arc<PoolInner>`
breaks the worker↔pool cycle). The whole pool + `Scope` fork/join + parallel
`Schedule::run` is proven sound and Tree-Borrows-clean (Phase 9.1/9.2/9.3 —
loom 4/4 + stress + Miri). Deps: `crossbeam-deque`, `crossbeam-utils` (+ `loom`
under `--cfg loom`). See [PHASE-9.1-RESULTS.md](archive/PHASE-9.1-RESULTS.md),
[PHASE-9.2-RESULTS.md](archive/PHASE-9.2-RESULTS.md).

---

## 19. Derive / attribute macros ✅

**File:** [crates/boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs)
(deps: `syn`, `quote`, `proc-macro2`, and `boyko-ecs` for the emitted paths).

| Macro | Generates |
|-------|-----------|
| `#[derive(Component)]` | `Component` impl (lazy `component_id()` via per-type `OnceLock`, also installs hooks) + inherent `SIZE` / `ALIGN` / `layout()`. Optional `#[component(on_add = path, …)]` binds Phase-14a hooks (XOR the runtime builder). |
| `#[derive(Resource)]` | `Resource` impl (lazy `resource_id()`); rejects a type already a `Component` (M6). |
| `#[derive(Bundle)]` | `Bundle` impl over a named struct (sealed; `Send + Sync + Unpin + 'static`). |
| `#[derive(SystemSet)]` | `SystemSet` impl for fieldless enums (variant → discriminant); data-carrying variants / unions / generics rejected (Phase 15). |
| `#[event]` | Rewrites a struct with `#[participant(...)]` / `#[parameter]` fields into `{ participants, parameters }` + the `Event` impl (typed-field accessors). Compile-fail UI tests in `tests/ui/event_attribute/`. |

**`boyko-macros` is a DEV-dependency of `boyko-ecs`** (a normal dependency would
cycle, since the macros emit `::boyko_ecs::…` paths). Consequences (Phase 18):
the prelude omits the derives, and lib-internal types like `AppExit` hand-impl
their traits. Import derives directly: `use boyko_macros::{Component, Resource,
Bundle, SystemSet};`.

---

## 20. Constants

| Name | Value | Where defined |
|------|-------|---------------|
| `COMMIT_GRANULE` | 64 KiB (renamed from `ARENA_COMMIT_GRANULE`, Phase X.J) | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):7 |
| `CACHE_LINE_SIZE` | 64 B | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):11 |
| `MIN_ALIGNMENT` | 8 B | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):16 |
| `SIMD_BUFFER_ALIGN` | 32 B (AVX2) | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):26 (Phase X.A) |
| `POOL_TARGET_DATA_BYTES` | 1 GiB (syscall arms) / 4 MiB (fallback) | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):60/:69 (syscall / fallback arm, Phase X.I) |
| `POOL_MIN_ROWS` / `POOL_MAX_ROWS` | 2^16 / 2^24 (syscall arms); 256 / 2^18 (fallback) | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):75/:85 (syscall arms, Phase X.I) |
| `POOL_MIN_SLAB` / `POOL_MAX_SLAB` | 64 KiB / 64 MiB | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):103/:110 (Phase X.I) |
| `DEFAULT_INLAND_RESERVE` | 1 GiB (syscall arms) / 16 MiB (fallback) | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):354/:361 (syscall / fallback arm, Phase X.G) |
| `INLAND_MIN_SLAB` / `INLAND_MAX_SLAB` | 256 KiB / 16 MiB | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):367/:372 (Phase X.G) |
| `MAX_EVENT_THREADS` / `MAX_EVENT_CAPACITY` | 64 / 16384 | [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):380/:384 |
| `MAX_COMPONENTS` | 512 | [component/component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):61 |
| `MAX_EVENTS` | 256 | [events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs):51 |
| `MAX_ARCHETYPES` | 1024 | [iters/archetype_bit_set.rs](../crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs):7 |
| `RESOURCE_SLOT_COUNT` | 256 | [resources/resource_registry.rs](../crates/boyko_ecs/src/ecs/core/resources/resource_registry.rs):51 |
| `MAX_QUERY_TYPES` | 1024 (4096 with `big_query_table`) | [iters/query/query_type_registry.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_type_registry.rs):84/:89 |
| `MAX_BUNDLE_TYPES` | 1024 | [bundle/bundle_type_registry.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle_type_registry.rs):84 |
| `MAX_CHANGE_AGE` / `CHECK_TICK_THRESHOLD` | ~3/4·u32::MAX / 518_400_000 | [change_detection/tick.rs](../crates/boyko_ecs/src/ecs/core/change_detection/tick.rs) |
| `MAX_WORKERS` | (pool cap) | [boyko_threadpool/thread_pool.rs](../crates/boyko_threadpool/src/thread_pool.rs) |

Note: the per-subsystem `MAX_*` capacities live next to their owning registry,
NOT in `constants.rs` (which holds only sizes / thresholds / SIMD-align).

---

## 21. boyko_demo ✅

**Crate:** [crates/boyko_demo/](../crates/boyko_demo/) — a wgpu+egui
GPU-instanced sandbox that dogfoods the public API. Particles / boids / physics
modes switched via Phase-17 states; drives a real `Schedule::run` + `par_iter` +
a **zero-AoS-copy `for_each_chunk` SoA → GPU upload**, **substep-gated** since
Phase 20.1 (`upload_due` in `app.rs`, D5): a 0-substep display frame with a
stable entity count uploads nothing, so uploads run at min(display, sim) rate
(−55 % upload events at 144 Hz display / 64 Hz sim). `GpuInstance`
(`render/instance.rs`) is the 24 B interpolated record
`{pos, scale, color, prev_pos}`; the vertex shader renders
`mix(prev_pos, pos, alpha)` with `alpha = FixedTime::overstep_fraction()`
delivered through the 80 B camera uniform. `sync_gpu_instance`
(`sim/systems/common.rs`) is the SINGLE load-bearing `prev_pos` maintainer
(D3) — including in Physics mode, where it looks redundant but gating it out
would kill prev maintenance. Layout: `app.rs`, `sim/`
(`systems/`, `grid.rs`, `modes.rs`, `runner.rs`, `bundles.rs`, `components.rs`,
`resources.rs`), `render/`, `ui/`. Compiles for wasm32 (webgl backend; 28
pointer-width const-asserts gated to 64-bit). See [DEMO-PLAN.md](archive/DEMO-PLAN.md) +
[DEMO-DOGFOODING.md](DEMO-DOGFOODING.md) +
[PHASE-20.1-RESULTS.md](archive/PHASE-20.1-RESULTS.md).

---

# Std-lib / simulation crates

These build on the public `boyko_ecs` API (components / resources / systems /
schedule) — every durable store is an ECS column or a `Resource`-owned buffer
(Principle 0). They are the standard-library layer a game composes on top of the
kernel.

## 22. boyko_math ✅

**Crate:** [crates/boyko_math/](../crates/boyko_math/) — the single SIMD-aligned
POD math vocabulary for the whole engine. Every type is `#[repr(C)]` plain-old-data
(`Copy`, no `Drop`, no interior pointers), SIMD-aligned where it matters.

**Modules:**
- [vec.rs](../crates/boyko_math/src/vec.rs) — `Vec2` / `Vec3` / `Vec4`.
- [quat.rs](../crates/boyko_math/src/quat.rs) — `Quat` (rotation).
- [mat.rs](../crates/boyko_math/src/mat.rs) — `Mat3` (row-major) / `Mat4` (column-major).
- [affine.rs](../crates/boyko_math/src/affine.rs) — `Affine3A`, the packed world-pose transform.
- [ray.rs](../crates/boyko_math/src/ray.rs) — `Ray` + `ray_aabb` / `ray_sphere` intersection.

**Bit-determinism (INVIOLABLE):** `Vec3` / `Quat` / `Mat3` are lifted *verbatim*
(instruction-identical) from the physics foundation so migrated physics stays
bit-for-bit unchanged — normalization is exact `sqrt().recip()` (NOT hardware
`rsqrt`), and there is **no** `mul_add` / FMA / fast-math anywhere. New ops for the
new types follow the same discipline (every multiply-then-add is separate
statements so codegen does not contract into an FMA). It is a workspace leaf (no
intra-workspace deps).

## 23. boyko_scene ✅

**Crate:** [crates/boyko_scene/](../crates/boyko_scene/) — the engine's spatial
vocabulary and transform propagation (std-lib Phase S2). Sits one layer above the
kernel and owns the spatial components every world-space subsystem builds on.

**Key types + entry points:**
- [transform.rs](../crates/boyko_scene/src/transform.rs) — `Transform` (LOCAL,
  decomposed, designer-facing pose) and `GlobalTransform` (cached WORLD pose, a
  packed `Affine3A`).
- [propagation.rs](../crates/boyko_scene/src/propagation.rs) —
  `propagate_transforms` composes each entity's `GlobalTransform` along the
  `ChildOf` / `Children` chain, alloc-free and dirty-gated (`TransformPropagationScratch`).
- [camera.rs](../crates/boyko_scene/src/camera.rs) / [camera_plugin.rs](../crates/boyko_scene/src/camera_plugin.rs) — camera components + the rig → on-screen `ViewUniform`.
- [visibility_sync.rs](../crates/boyko_scene/src/visibility_sync.rs) / [render_caps.rs](../crates/boyko_scene/src/render_caps.rs) — `Visibility` / `RenderEnabled` + `MeshHandle` / `MaterialHandle` render capabilities.
- [identity.rs](../crates/boyko_scene/src/identity.rs) — interned `Name` / `NameId`.

Principle 0: no parallel pose store — `Transform`/`GlobalTransform` are ordinary
ECS columns. Plugins: `TransformPlugin`, `CameraPlugin`.

## 24. boyko_sdf_math ✅

**Crate:** [crates/boyko_sdf_math/](../crates/boyko_sdf_math/) — the analytic SDF
edit-list field math + std430 data model, extracted as a `#![no_std]` leaf with
**zero dependencies** (only `core`). It is the SINGLE source of truth shared by two
consumers that must NOT depend on each other: `boyko_rhi_vulkan` (the GPU golden
mirror of the HLSL field) and `boyko_physics` (the CPU SDF-collision narrowphase),
guaranteeing both fold bit-identical arithmetic.

**Modules:**
- [lib.rs](../crates/boyko_sdf_math/src/lib.rs) — `SdfEdit`, `sdf_edit_list` (folds
  the ordered primitive/op list per point), `sdf_kind` / `sdf_op` constants.
- [brick.rs](../crates/boyko_sdf_math/src/brick.rs) — the brick-atlas data model.
- [mesh_sdf.rs](../crates/boyko_sdf_math/src/mesh_sdf.rs) — mesh-derived SDF queries.

It DELEGATES its f32 field bodies to `boyko_shaderdsl::field::*::<f32>` (the Eval
backend), so the CPU field and the HLSL emitter share one authored source.

## 25. boyko_physics ✅

**Crate:** [crates/boyko_physics/](../crates/boyko_physics/) — the in-house 3D
physics: the universal contact currency (`Manifold`), a swappable zero-`dyn`-on-the-
hot-path `RigidSolver` trait, and the real TGS-Soft solver. All bodies / colliders /
contacts are ordinary `#[derive(Component)]` columns with a Phase-10-ready hot/cold
split (no parallel data system — the SP4 race remediation put both solvers on kernel
`ScratchColumn`).

**Modules:**
- [components.rs](../crates/boyko_physics/src/components.rs) — `RigidBody` / `Collider` / `Contact` columns; `bundles.rs` — `DynamicBody` / `Trigger`.
- [manifold.rs](../crates/boyko_physics/src/manifold.rs) — `Manifold` / `ContactPoint` (the contact currency, `SDF_SENTINEL`).
- [narrowphase/](../crates/boyko_physics/src/narrowphase/) — convex contact generators (`sphere_box`, `box_box` with a feature-id-stable OBB cache in `axis_cache.rs`).
- [solver/](../crates/boyko_physics/src/solver/) — `SoftStepSolver` / `ColoredSoftStepSolver` / `NoopSolver` (`soft_step.rs`, `warm_start.rs`, `contact.rs`, `simd.rs`, `colored.rs`).
- [soft/](../crates/boyko_physics/src/soft/) — soft-body (`component.rs`, `collide.rs`, `self_collision.rs`, `coupling.rs`, `colored.rs`).
- [sdf_query.rs](../crates/boyko_physics/src/sdf_query.rs) — body-vs-SDF via `boyko_sdf_math` (zero readback, zero graphics deps).
- [scene_sync.rs](../crates/boyko_physics/src/scene_sync.rs) — `boyko_scene` `Transform` ↔ body sync.

**Entry point:** `add_physics_systems` (+ `_soft` / `_soft_colored` / `_sdf` /
`_with_scene_sync` variants) adds the fixed-step pipeline to a `ScheduleBuilder`.
Deterministic, Miri-clean; broadphase auto-selected (`select_broadphase`).

## 26. boyko_input ✅

**Crate:** [crates/boyko_input/](../crates/boyko_input/) — source-agnostic,
rebindable action mapping. Turns raw keyboard/mouse events from ANY source (native
raw-FFI Win32 window, egui demo, synthetic test stream) into typed rebindable
**actions** consumed by ECS systems. The engine path depends on NO windowing library
(winit/Win32/eframe live behind feature-gated edge adapters), so it compiles on every
target including wasm.

**Layers:**
- [raw/](../crates/boyko_input/src/raw/) — canonical physical enums (`keycode.rs`), the
  seam event (`event.rs::RawInputEvent`), the ring buffer + per-frame snapshot
  (`queue.rs::RawInputQueue` / `PhysicalInput`), scancode tables (`scancode.rs`).
- [action/](../crates/boyko_input/src/action/) — typed `Actionlike` (`actionlike.rs`),
  the binding map (`map.rs::InputMap`), the SoA `ActionState` (`state.rs`), per-frame
  aggregation (`process.rs`), rebind sessions (`rebind.rs`), clash resolution
  (`clash.rs`), name interning (`names.rs`).
- [win32.rs](../crates/boyko_input/src/win32.rs) — a PURE Win32 message → `RawInputEvent`
  edge adapter (no FFI, no windowing dep).
- [persist/](../crates/boyko_input/src/persist/) — keybind save/load.

**Entry point:** `InputPlugin` + the `GameplaySet`.

## 27. boyko_serialize ✅

**Crate:** [crates/boyko_serialize/](../crates/boyko_serialize/) — custom binary
world save/load. **Codegen, not reflection**: serialization is driven through the
per-`ComponentId` fn-ptr table in `boyko_ecs`'s cold registry (`SERIALIZE`) plus a
raw-blit fast path for `PlainOldBytes` columns. Never depends on any reflection crate.

**Modules:**
- [format.rs](../crates/boyko_serialize/src/format.rs) — the `#[repr(C)]` on-disk types (`SaveHeader`, `TypeTableEntry`, `ArchetypeBlock`, `ColumnRegion`, `VarRef`) with const-asserted layouts (the bytes ARE the wire contract).
- [save.rs](../crates/boyko_serialize/src/save.rs) — `save_world` / `save_world_to_file`, the two-pass save (Pass 1 sizes + lays out offsets + grows once; Pass 2 blits POB columns and encodes `SerializeViaFn`).
- [load.rs](../crates/boyko_serialize/src/load.rs) — `load_world` / `load_world_from_file`, the `CopyIntoWorld` + `Remap` loader (validate header, resolve the type table once, blit/decode per fresh archetype, remap saved→fresh entity ids incl. `ChildOf`).

**Status:** Phases S1–S3 shipped (save/load, per-component `format_version`, loader
fuzz: Err-or-valid-never-UB). S4 (mmap) / S5 (parallel) deferred. Spec:
[SERIALIZATION-PLAN.md](SERIALIZATION-PLAN.md).

---

# Render / UI / shader crates

The render stack: an FFI-free RHI trait surface, a raw hand-FFI Vulkan backend, a
single ECS↔RHI bridge crate, the shader eDSL that single-sources the SDF shader math,
the font baker, and the ECS-native UI. Per the HYBRID-perf principle, render choices
are decided by measurement (mesh vs SDF vs hybrid), not representation-consistency.

## 28. boyko_rhi ✅

**Crate:** [crates/boyko_rhi/](../crates/boyko_rhi/) — the backend-agnostic Render
Hardware Interface trait surface (wgpu-hal-shaped, **FFI-free**). An umbrella `RhiApi`
trait with associated owned-resource types, separate operational traits (`RhiDevice`,
`RhiQueue`, `RhiCommandEncoder`), thin enums/descriptors, and a generational handle
registry (`ResourceRegistry`). Backends implement these over their own resources via
**static dispatch** — every call monomorphizes to a direct non-virtual call, zero
abstraction overhead. `RhiApi` is intentionally NOT object-safe; there is no `dyn`,
`Box`, or `HashMap` anywhere.

**Modules:** [api.rs](../crates/boyko_rhi/src/api.rs) (`RhiApi`), [device.rs](../crates/boyko_rhi/src/device.rs), [queue.rs](../crates/boyko_rhi/src/queue.rs), [encoder.rs](../crates/boyko_rhi/src/encoder.rs), [descriptor.rs](../crates/boyko_rhi/src/descriptor.rs), [enums.rs](../crates/boyko_rhi/src/enums.rs), [handle.rs](../crates/boyko_rhi/src/handle.rs), [error.rs](../crates/boyko_rhi/src/error.rs). Depends only on `boyko_utils`.

## 29. boyko_rhi_vulkan ✅

**Crate:** [crates/boyko_rhi_vulkan/](../crates/boyko_rhi_vulkan/) — the raw
hand-FFI Vulkan backend (std-only, no third-party crates; the FFI mirrors
`boyko_ecs`'s `vm.rs` style). Implements the `boyko_rhi` traits over real Vulkan.

**Modules:**
- [ffi.rs](../crates/boyko_rhi_vulkan/src/ffi.rs) / [device.rs](../crates/boyko_rhi_vulkan/src/device.rs) / [debug.rs](../crates/boyko_rhi_vulkan/src/debug.rs) — hand loader, `VkInstance` / `VkDevice`, validation messenger.
- [memory.rs](../crates/boyko_rhi_vulkan/src/memory.rs) / [suballocator.rs](../crates/boyko_rhi_vulkan/src/suballocator.rs) — a `VkDeviceMemory` free-list sub-allocator with coalescing.
- [rhi_impl/](../crates/boyko_rhi_vulkan/src/rhi_impl/) — the `RhiApi` impl, split by the god-file refactor into [mod.rs](../crates/boyko_rhi_vulkan/src/rhi_impl/mod.rs) (the `Vulkan` marker + ownership/teardown discipline), [device.rs](../crates/boyko_rhi_vulkan/src/rhi_impl/device.rs) (device / queue / `ComputeLayouts`) and [encoder.rs](../crates/boyko_rhi_vulkan/src/rhi_impl/encoder.rs) (`VulkanCommandEncoder` with `pipeline_barrier` lowering).
- [compute.rs](../crates/boyko_rhi_vulkan/src/compute.rs) — compute dispatch + the `golden_*` CPU oracles.
- [swapchain.rs](../crates/boyko_rhi_vulkan/src/swapchain.rs) / [window.rs](../crates/boyko_rhi_vulkan/src/window.rs) / [texture.rs](../crates/boyko_rhi_vulkan/src/texture.rs) — the on-screen path (surface / swapchain / present / image barriers).
- [framegraph/](../crates/boyko_rhi_vulkan/src/framegraph/) — the Render Dependency Graph (declare → compile auto-barriers → execute), the single sync authority replacing the hand-barrier path.
- [brick_atlas.rs](../crates/boyko_rhi_vulkan/src/brick_atlas.rs) / [mesh_sdf_texture.rs](../crates/boyko_rhi_vulkan/src/mesh_sdf_texture.rs) — SDF brick-atlas + mesh-SDF textures.

Depends on `boyko_rhi` + `boyko_sdf_math` (the golden mirror folds the shared field).
The `shaders/` directory holds the frozen `.hlsl` + committed `.spv` (the eDSL emits
byte-identical SPIR-V; see §31).

## 30. boyko_render ✅

**Crate:** [crates/boyko_render/](../crates/boyko_render/) — the bridge between the
graphics-pure ECS core and the RHI. The ONLY crate that may name both `boyko_ecs` and
the RHI surface, so the orphan-rule impls (`RhiContext: NonSendResource`) and the
graphics-aware types live here, never in the kernel. GPU access is compiler-enforced
`!Send`.

**Areas:**
- GPU columns: [gpu_column.rs](../crates/boyko_render/src/gpu_column.rs) mints `DeviceLocal` (VRAM) component pools behind the RHI registry and drives the kernel A2 device-mint seam; [gpu_system.rs](../crates/boyko_render/src/gpu_system.rs) dispatches compute with zero per-frame readback.
- 3D instancing / meshes: [gpu3d_instance.rs](../crates/boyko_render/src/gpu3d_instance.rs) / [gpu3d_system.rs](../crates/boyko_render/src/gpu3d_system.rs) / [mesh_draw.rs](../crates/boyko_render/src/mesh_draw.rs) / [mesh_assets.rs](../crates/boyko_render/src/mesh_assets.rs) (asset-rung A2 folded the standalone `MeshRegistry` into `Assets<MeshGpu>`) / [instance_model.rs](../crates/boyko_render/src/instance_model.rs) / [material.rs](../crates/boyko_render/src/material.rs).
- Lighting: [light.rs](../crates/boyko_render/src/light.rs) / [light_system.rs](../crates/boyko_render/src/light_system.rs) / [light_reconcile.rs](../crates/boyko_render/src/light_reconcile.rs) / [light_policy.rs](../crates/boyko_render/src/light_policy.rs) (directional / point / spot + clustered froxel cull). Host plan R4: `LightTableGeneration` (the D5 writer-side staging generation `collect_lights` bumps per actual rewrite) + the light-header word-7 CSM gate (`LightingConfig::csm_shadows`, packed by `LightHeaderGpu::new`).
- Shadows: [csm_config.rs](../crates/boyko_render/src/csm_config.rs) / [csm_caster.rs](../crates/boyko_render/src/csm_caster.rs) / [shadow_atlas.rs](../crates/boyko_render/src/shadow_atlas.rs) (CSM cascades + punctual atlas) + `ssao_*`. Host plan R4: `sync_csm_light_gate` (csm_caster.rs) keeps the header gate in lock-step with the depth-pass arming predicate (fitted sun AND live casters).
- Occlusion culling (VG R3, two-phase HZB): [hzb_config.rs](../crates/boyko_render/src/hzb_config.rs) is the PRODUCER knob (`HzbMode::{Off, Build}` — does the engine maintain a depth pyramid) and [occlusion_config.rs](../crates/boyko_render/src/occlusion_config.rs) the CONSUMER knob (`OcclusionMode::{Off, TwoPhase}` — does the owner want the decision), two types because they are two questions; both are `Copy` Resources read live per frame, both default to `Off`, and enablement is structural (`mode != Off`), never a stored flag. [occlusion_marker.rs](../crates/boyko_render/src/occlusion_marker.rs)'s `OcclusionCulling` is the per-entity CAPABILITY — a table-storage ZST whose presence is the datum, read non-filteringly as `Option<&T>` so the gather's lock-step with the instance ring survives; `Off` suppresses the TEST, never the GATHER, so the marked-instance counter means one thing regardless of the knob. [hzb.rs](../crates/boyko_render/src/hzb.rs) is the host mirror of the shader's verdict, used as an ORACLE by the GPU gates rather than as production code. Plugins: `HzbPlugin`, `OcclusionPlugin` (composed unconditionally — `Off` is the 0 %-gate, so every golden pin stays byte-identical BY CONSTRUCTION). The host side (`boyko_app`) owns `hzb_plan_for`'s producer-or-consumer disjunct, `occlusion_arm_for`, and `OcclusionForce` — a DIAGNOSTIC verdict override (`KeepAll`/`DeferAll`) that is deliberately not owner surface and lives outside `boyko_render` for that reason. Spec: [VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md](VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md).
- Token-typed uploads: [upload.rs](../crates/boyko_render/src/upload.rs) — `upload_camera_ring` / `upload_instance_models` (R3) + `upload_light_table` (per-slot staging ring — the R4 host-write-vs-GPU-copy race pin) / `upload_csm_ring` (the 336 B `ResolvedCsm` mirror, unconditional per frame).
- View: [view.rs](../crates/boyko_render/src/view.rs) consumes the engine-derived `ViewUniform` (from `boyko_scene`) as the single view source.
- Asset loaders (in-house, zero third-party decode deps): [loaders/](../crates/boyko_render/src/loaders/) — `obj`, `png_texture`, `ron_material`, and `glb` (glTF 2.0 binary → `MeshData`, VG-R0 rung R0b: concatenates primitives, composes node hierarchies, bakes each placement into model space).
- VG-R0 density census: [vg_census.rs](../crates/boyko_render/src/vg_census.rs) — the host reducer that turns one `vb_id` readback into a census row (covered pixels, distinct visible triangles, the power-of-two histogram and its modal bucket), plus the workspace's streaming SHA-256. Pure CPU and device-free by design, which is why it is its own module; the armed GPU half lives in `boyko_app`. Distinct triangles are counted by sorting packed `(instance, primitive)` keys and counting runs — `HashMap` is banned here and a run's *length* is that triangle's covered-pixel count, so the histogram falls out of the same pass. See [FEATURE_MAP.md](FEATURE_MAP.md)'s "VG-R0 density census" section for the whole instrument.

Plugins: `Render3dPlugin` (+ `light_plugin`, `csm_plugin`, `shadow_plugin`,
`ssao_plugin`). Depends on `boyko_ecs` + `boyko_rhi` + `boyko_rhi_vulkan` +
`boyko_scene` + `boyko_math` + `boyko_fontbake` + `boyko_utils`.

## 31. boyko_shaderdsl ✅

**Crate:** [crates/boyko_shaderdsl/](../crates/boyko_shaderdsl/) — the in-house Rust
shader eDSL (zero third-party deps; NO rust-gpu / naga / spirv-builder). Author the
SDF shader math ONCE, generic over a `FieldScalar` backend, and instantiate it two
ways: `S = f32` is the **Eval** backend (each op is one `core` f32 instruction,
byte-identical to `boyko_sdf_math`) and `S = Emit` is the **HLSL SSA recorder** (each
op pushes one SSA node; the printer walks the arena into HLSL textually equivalent to
the frozen `.hlsli`). There is NO runtime AST and NO transpiler — the generic body is
ordinary monomorphized Rust; the byte-identity gate re-DXCs the emitted HLSL and
compares against the frozen `.spv`.

**Modules:** [field.rs](../crates/boyko_shaderdsl/src/field.rs) (the authored field math), [marcher.rs](../crates/boyko_shaderdsl/src/marcher.rs) (sphere-trace control flow), [brick.rs](../crates/boyko_shaderdsl/src/brick.rs) / [cubic_hit.rs](../crates/boyko_shaderdsl/src/cubic_hit.rs) / [levels.rs](../crates/boyko_shaderdsl/src/levels.rs) (brick atlas), [normal.rs](../crates/boyko_shaderdsl/src/normal.rs) / [oct.rs](../crates/boyko_shaderdsl/src/oct.rs) / [pack.rs](../crates/boyko_shaderdsl/src/pack.rs) (G-buffer pack), [shadow.rs](../crates/boyko_shaderdsl/src/shadow.rs) / [ssao.rs](../crates/boyko_shaderdsl/src/ssao.rs), [scalar.rs](../crates/boyko_shaderdsl/src/scalar.rs) (`FieldScalar`), [emit/](../crates/boyko_shaderdsl/src/emit/) (`feature = "emit"`, the SSA arena + printer). This killed ~5 field-drift bugs by single-sourcing the shader math.

## 32. boyko_fontbake ✅

**Crate:** [crates/boyko_fontbake/](../crates/boyko_fontbake/) — the load-time MTSDF
font baker for GUI text (P5b). A **build/setup tool**, NEVER on the render hot path:
ingests a font, extracts glyph outlines/metrics, generates a multi-channel SDF atlas
entirely in-house, packs the glyphs, and serializes to a `.bfont` the runtime loads
with a thin POD reader.

**Modules:**
- [face.rs](../crates/boyko_fontbake/src/face.rs) — the in-house `FontFace` / `OutlineSink` traits + a `ttf-parser` adapter (`TtfFace`). The engine depends on the trait, not the backend.
- [extract.rs](../crates/boyko_fontbake/src/extract.rs) — glyph outlines (line/quad/cubic, em-normalized) + metrics.
- [msdf/](../crates/boyko_fontbake/src/msdf/) — the in-house MSDF generator (edge coloring, per-channel signed pseudo-distance, scanline sign-correction, error-correction).
- [atlas.rs](../crates/boyko_fontbake/src/atlas.rs) — skyline atlas packing + `.bfont` serialization.

Depends on `boyko_math` + `boyko_threadpool` only (off the hot path).

## 32b. boyko_image ✅

**Crate:** [crates/boyko_image/](../crates/boyko_image/) — the in-house PNG decoder,
written from the spec text with **zero third-party dependencies** (`std` only): RFC 1950
(zlib) + RFC 1951 (DEFLATE) decompression plus the PNG container. A pure-CPU, `Send`
LEAF crate (no workspace dependencies at all — `boyko_utils`'s decoupled role, mirrored
for image data); a **load-time** path, never per-frame.

**Scope:** color types 0/2/4/6 (grayscale, RGB, grayscale+alpha, RGBA), bit depths 8 and
16, all five PNG filter types, non-interlaced. Single entry point: `decode_png`.

**Consumer:** `boyko_render`'s texture loader
([loaders/png_texture.rs](../crates/boyko_render/src/loaders/png_texture.rs)) — the
textured-PBR asset path.

## 33. boyko_ui ✅

**Crate:** [crates/boyko_ui/](../crates/boyko_ui/) — ECS-native UI. Widgets ARE
entities; layout inputs/outputs are components; the tree is `ChildOf`/`Children`
(Phase 19); layout is systems over the ECS. No parallel data system — props/outputs
are ECS columns, per-frame scratch is a `Resource`-owned buffer (reset every frame).

**Areas:**
- Layout: [layout.rs](../crates/boyko_ui/src/layout.rs) — `ui_layout_discovery` (a scheduled `FunctionSystem` where `Changed`/`Added` change detection lives, sets a `dirty` flag in `LayoutScratch`) + `ui_layout_apply` (an exclusive `&mut EcsMaster` system that re-lays-out root subtrees when dirty). [components.rs](../crates/boyko_ui/src/components.rs) / [units.rs](../crates/boyko_ui/src/units.rs) / [anchor.rs](../crates/boyko_ui/src/anchor.rs) — layout components.
- Text: [text/](../crates/boyko_ui/src/text/) — the `.ui` markup format (`parser.rs`, `ast.rs`, `lower.rs`, `emit.rs`), MSDF glyph measurement/emission (`measure.rs`, `font.rs`, `dispatch.rs`), `UI_FORMAT_VERSION`.
- Widgets: [widgets.rs](../crates/boyko_ui/src/widgets.rs) — the widget bundles/spawners.
- Interaction: [interaction/](../crates/boyko_ui/src/interaction/) — hit-testing, `focus.rs`, dispatch, and the `Interaction` → input `action.rs` edge.
- World-space HUD: [world/](../crates/boyko_ui/src/world/) — diegetic 3D UI (`pick.rs` cursor-ray pick, `project.rs`, `visibility.rs` CPU-proxy depth occlusion).
- Binding + reload: [binding/](../crates/boyko_ui/src/binding/) (`Bindable` data-binding to ECS state) + [reload/](../crates/boyko_ui/src/reload/) (`.ui` markup hot-reload).

Depends on `boyko_ecs` + `boyko_macros` + `boyko_utils` + `boyko_input` +
`boyko_scene` + `boyko_math` + `boyko_fontbake` (no render dependency — it emits
render-agnostic glyph-quad / instance descriptors). Plugin: `UiPlugin`; macro: `ui!`.

---

## Build / verification state

The cumulative suite is ~1,064 passing (`cargo test --workspace --all-targets`
debug, Phase X.J baseline) — in-module `#[cfg(test)]` units + integration files under
[crates/boyko_ecs/tests/](../crates/boyko_ecs/tests/). `cargo clippy
--all-targets -- -D warnings` is clean (modulo two known pre-existing trybuild
`.stderr` drifts on `bundle_compile_fail` / `compile_fail_chunk`, awaiting a
`TRYBUILD=overwrite` re-bless). Miri (`-Zmiri-tree-borrows`) is clean across the
change-detection / hooks / observers / states / executor-soundness suites.

For the exact per-phase gate (test counts, Miri scope, bench deltas), read the
relevant `docs/PHASE-*-RESULTS.md`; for the bench methodology (deterministic
`[profile.bench]` codegen, opt-in `bench-alloc` mimalloc, the median-of-N
`bench.ps1`), see [BENCHMARKING.md](BENCHMARKING.md).
