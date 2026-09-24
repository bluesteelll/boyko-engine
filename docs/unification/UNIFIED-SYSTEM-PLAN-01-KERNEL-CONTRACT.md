# Unified system plan — 01 Kernel contract (rev 6)

Tree tags (`[J]`, `[M]`, `[G]`, `[R]`) and the citation verification record are in
[00 Overview](UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md). Bare document names below are `[M]` documents.

Rev 5.1 (the modding reconciliation, 00 §10) changes I-1, I-4, KC-19b, the two P32/P40 rows of §3,
the modding line under §3, and §5.

Rev 6 (00 §10) adds KC-37 and §2.1 (replay determinism). It rewrites KC-04's arm text (portable arm only; the windows word arm moves to §8 as a revival form), narrows KC-36's "Deletes", and narrows I-4 and §5 to the exact-build family (Q-1).

Rev 6.1 (00 §10) answers critic pass 6. It adds rule B's `FixedTime` restriction and move clause; `TickActions` as the registered `ActionState<A>`; detection in place of kernel refusals; derived keys from an issue-time counter; canonical order for hooks of bundle-less ops; kernel surfaces D-E22 and D-E23; register rows H-20 and H-21.

## 1. Contract invariants (every KC obeys them)

- **I-1. Typed paths are unchanged.**
  - Engine component identity stays the derive's per-type `static ID: OnceLock<ComponentId>`
    (`[J]crates/boyko_macros/src/component.rs:372-375`).
  - Storage choice stays per-monomorphisation associated consts
    (`[J]crates/boyko_ecs/src/ecs/core/component/component.rs:51,65,79,95,103,126`).
  - No typed path gains a registry lookup or a `dyn` hop **to accommodate mods** (modding §7.8,
    `[M]docs/modding/MODDING-DESIGN-SPACE.md:1696-1700`).
  - The plan adds one typed-path lookup, and it is not for mods: KC-20 hashes the `TypeId` of a
    **generic** engine component on its uncached path (one hash plus one load per first touch per
    instantiation). Non-generic components are unchanged, and UG-15 leg (2) pins
    `Transform::component_id`.
  - System dispatch stays the existing one indirect call per system run
    (`[J]crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:1380`, `:1215`). No modding item adds
    a field to `SystemBox` or `Schedule`, or a branch or hop to the dispatch loop. Option A's mod
    systems pay their extra hop inside their own `System` impl (05 MS-12).
- **I-2. One storage family.**
  - Durable data lives in `ComponentPool` (tracked or untracked), `VmColumn` or
    `SegmentedColumn`, all on `boyko_memory` reservations.
  - No new column primitive stands beside `ComponentPool` (ledger net result,
    `RUNTIME-DATA-LEDGER.md:1715-1720`); the only admitted exception is KC-17.
- **I-3. One allocator.** Every commit goes through `boyko_memory::raw::commit_at` (UG-04).
  Process-heap calls exist only for the closed tags of allocator §0: `os-thread`, `panic`,
  `ffi-driver`, `harness`, `offline`, `diag` (`ALLOCATOR-DESIGN-SPACE.md:68-79`).
- **I-4. Modding is additive, and its kernel half is not compiled when unused**
  (`MODDING-DESIGN-SPACE.md:1810-1823`; the owner's requirement H-1, 05 §1).
  - **Rule S-1 (05 §1).** For modding, the kernel may gain only the `ModSeam` trait (KC-19b) and
    code that is generic over it: function bodies, types with methods, trait impls. No kernel
    crate implements `ModSeam`, so a game that links no modding crate instantiates none of that
    code, and it has no object code in any kernel rlib or in the game binary, at any profile.
  - A non-generic body, a `static`, a field on an existing type or a visibility change, when made
    for modding, is admitted only where 05 §3.2 names it and UG-15 admits it. The live options at the head of the ordering (A′, C) need none of them; option A needs the items 05 §3.2 names; B, D, E and F are out (Q-1; 05 §3.1).
  - **Scope.** This contract carries only what every live modding option needs: `ModSeam` and four
    occupancy readers (KC-19b). The option-specific items of 05 §3.2 are built at modding Stage 3,
    behind the modding crates, for the chosen option only (§5).
  - Never, for any reason, in any mode:
    - an export or retention attribute (`no_mangle`, `export_name`, `link_section`, `used`,
      `linkage`), in the plain or the Rust 2024 `#[unsafe(...)]` form;
    - a function **definition** with any explicit non-Rust ABI (`extern "C"`, `"system"`,
      `"C-unwind"`, `"sysv64"`, `"win64"`, …) or a bare `extern fn`;
    - `global_asm!`, `naked_asm!` or `#[naked]`.

    Import blocks (`unsafe extern "system" { … }`) and fn-pointer types are not definitions and are
    allowed. UG-15 leg (1) enforces the rule. Its only exception mechanism is an owner-signed
    allowlist entry naming the file, the item and the reason (03 §6). The list is empty, and no rung
    in this plan adds to it.
  - Never for modding: a `cfg`, a cargo feature, or a constant whose value depends on the
    configuration.
  - Outside the rule: ordinary attributes (`#[cold]`, `#[inline]`, `#[doc(hidden)]`, `#[repr]`) and
    the tree's existing build-mode cfgs (`test`, `loom`, `miri`, `debug_assertions`).
  - Admission: a delta is admitted only while UG-15 is green in strict mode against its parent
    (modding R1, `:1818-1823`), including legs (7) and (7b) on a seam commit (05 §6).
- **I-5. Protector-lifetime soundness.**
  - TB-1..TB-3 as corrected by P11/P42 (`ALLOCATOR-DESIGN-SPACE.md:1175-1179, 3753`).
  - Every `unsafe` block carries a `// SAFETY:` that argues the protector, not the last use.

## 2. The unified kernel features

"Default-path cost" means the cost to a game that does not use the feature.

### Layer 0 — memory library (`boyko_memory`)

| KC | Capability | API shape | Storage / backing | Threading | Default-path cost | Deletes |
|---|---|---|---|---|---|---|
| **KC-01** | Crate split: memory primitives below the pool | `VmReservation`, `VmColumn<T: ZeroInit, O: CommitOwner = ColumnOwner>`; `#[doc(hidden)] pub mod raw { fn reserve(bytes); unsafe fn commit_at<O: CommitOwner>(&VmReservation, off, len) }`; `COMMITTED_BYTES: [AtomicUsize; 3]` indexed by the `O::INDEX` const (`Column`, `Chunk`, `Table`). **Producers per owner:** `Column` = every `ComponentPool` and every default `VmColumn`; `Chunk` = `ChunkArena` (KC-05); `Table` = the KC-18 kernel tables, declared `VmColumn<T, TableOwner>`. The owner is a marker type with an associated `INDEX` const, so the choice is monomorphised (0 runtime instructions) | OS reservations; old `boyko_ecs::ecs::memory` paths re-exported for one rung | reserve/commit: owner thread; counters `Relaxed` (bound, not publication) | none on hot paths; one relaxed RMW per commit syscall (P15/P31, `:2703`; the allocator's form there has five owners, `Column \| Heap \| Chunk \| Frame \| Table` — Heap leaves with U-1 and Frame with P34) | the `boyko_threadpool` → `std::alloc` necessity (KF-32, 9 rows) |
| **KC-02** | Commit quantum = `COMMIT_PAGE` (4 KiB); granule reservations kept | constants and `commit_page_region` (packing D1–D7) | unchanged reservations | unchanged | **0 hot-path instructions** (`[J]…/POOL-SUBGRANULAR-PACKING-PLAN.md:60-66`); +12 cold syscalls per column that grows past 4 KiB | 384→12 KiB per tracked column; not landed at `[J]` (`[J]crates/boyko_ecs/src/ecs/constants.rs:103` `POOL_MIN_SLAB = 64 KiB`) |
| **KC-03** | VmColumn contract widening | `unsafe trait ZeroInit` (P5); any non-ZST size (floor division, §2.2); `ensure_len_zeroed(n)`; `push/pop` stack helpers; `ByteColumn = VmColumn<MaybeUninit<u8>>` + `spare_ptr(n)`, `unsafe set_len`; `AtomicU64::from_ptr` views (KF-42) | VmColumn | single writer under `&mut`; atomic views for readers (KF-42) | none | `Vec::resize` fill loops (`slot+1` encoding); Vec-backed stacks; KF-35 (1 row), KF-42 (4 rows) |

### Layer 1 — pool substrate (`boyko_threadpool`)

| KC | Capability | API shape | Storage / backing | Threading | Default-path cost | Deletes |
|---|---|---|---|---|---|---|
| **KC-05** | Scope chunk source | `ScopeBlock` unchanged inline in `Scope`; `ScopeShared` emplaced as the block's first cell (P2); `ChunkArena` { 1 GiB VA, one `frontier: AtomicUsize`, `carve(exp)` `#[cold]` }; `SlotChunks` { intrusive per-exponent LIFO heads, `EXP_MAX = 51` } (P16/P28); `LaneDeposit._pad` → `cache_slot`, stored + 1 so that an all-zero deposit decodes to `NO_CACHE_SLOT` (§6 layout); `PoolInner.dispatcher_claim: AtomicU64` (P17) | one reservation per pool | SC-1 and SC-2 ownership (`:1820-1834`); claim with Acquire/Release; `W + D ≤ 64` asserted | 0 extra TLS reads (`:875`); empty scope = one list pop after warm-up | `Box::new(ScopeShared)` (`[J]crates/boyko_threadpool/src/thread_pool.rs:278,328`); chunk `alloc`/`dealloc` (`[J]crates/boyko_threadpool/src/block.rs:482,341`); defect 4 |
| **KC-06** | (a) boot-wait + epoch pre-registration (1f); (b) bounded MPMC injector ring | `ThreadPoolBuilder::build` waits for its workers; injector `W × LANE_CAP` of 16-B tasks; on overflow, `#[cold]` spin + `unpark_one_idle` per round (P10/P20) | pool reservation | push-side wake protocol (P20, `:2014-2020`) | overflow counter pinned at 0 | crossbeam `Injector` blocks; defect 6; together with KC-05, defect 5's allocation share (UG-03 scene S1c's W = 1 arm, pinned at 0 in the steady window); KF-34 (4 rows, narrowed per U-5) |
| **KC-07** | Pool ownership | `App` owns `ThreadPool` by value; `Schedule` holds no `Arc` (today `[J]…/schedule.rs:116`); `PoolInner` lives in one reservation with inline `MAX_WORKERS` arrays | reservation | unchanged | none | KF-31 (19 rows) |
| **KC-08** | Gang `par_phases` + `par_range` | physics §9/§10.1 rev 5 as written (`LaneBoard`, `run_lane`, `poison()`, `GangAbort`, `&mut GangLane`) | lane 0 frame; board in `PoolInner` | rev-5 protocol; NB3 cold release assert; NB4 `GangLane` is `!Clone + !Copy + !Sync` | one board probe at each of three idle-loop sites per worker iteration: after the steal, after `[J]crates/boyko_threadpool/src/worker.rs:115`, after `:126` (`PHYSICS-ECS-UNIFICATION-DESIGN.md:3055`); lowest priority | the 4 hand-rolled physics `pool.scope` sites; defect 5's serial share: `par_range` and `par_phases` run the body inline on the caller when the pool has one worker (one load of the worker count per call, no scope, no task). Red-first: a W = 1 physics step constructs 0 scopes (UG-03) |
| **KC-09** | Panic contract: a task panic is re-raised exactly once at its join, and the pool keeps W workers | fix of checkpoint defect CK-A6, landed by rung A2 | — | — | none | CK-A6 |
| **KC-04** | Engine thread context (replaces KF-45's `thread_local!` rows) | `boyko_threadpool::thread_ctx`: `#[inline] current() -> Option<&'static ThreadRecord>`; `claim_for_pool(requested) -> SlotBatch` and `adopt(slot) -> AdoptedRecord`, which only `ThreadPoolBuilder::build` and `worker_main` call; `unsafe fn release_current()`; `#[doc(hidden)] ThreadRecord::ext_ptr()` for the one upper-layer region; `#[cold] refused_write_panic()`; the UG-20 counters. Protocol, capacity and layout are in §6 | two all-zero `.bss` statics: `THREAD_BUSY: [AtomicU64; 128]` and `THREAD_RECORDS: [ThreadRecord; 8192]` (64 B each, 524,288 B virtual; resident per claimed page). A per-thread OS word holds the record's index + 1 | records are slot-private; the word is written only by its own thread; bits are CAS-claimed (§6) | one arm on every host (U-19): +1 load and +1 branch against a native `thread_local!` field, no lock, no call. On windows-gnu, now a comparison host, the native access is std's OS-key read. Per non-worker thread: one cold claim, and on gnu one `os-thread` System allocation plus one std `enable()` for `EXIT_GUARD`. Per worker: the adopt write, no `EXIT_GUARD`. MQ-13 prices msvc (deciding), and gnu and Linux (record-only). The not-built windows word arm is §8 | KF-45: 12 rows → 4 key/guard cells (§6, item 10) |

### Layer 2 — storage forms (`boyko_ecs`)

| KC | Capability | API shape | Storage / backing | Threading | Default-path cost | Deletes |
|---|---|---|---|---|---|---|
| **KC-10** | Registry-free scratch cohorts; ScratchColumn built from a type alone; world scratch frames (KF-05) | **Columns (D-S2):** `ScratchColumn::<T>::for_type(rows)`; `Default` = an unreserved column: the stagger is taken at construction, the reservation at the first push (lazy state below); `ScratchCohort::reserve(width)` + `in_cohort(&c, k, rows)`; `pub(crate) ComponentPool::new_untracked_raw(layout: Layout, type_id: TypeId, drop_fn: Option<DropFn>, stagger: u32, rows, reserve: Reserve)` with `Reserve::{Now, Lazy}`; the typed wrappers monomorphise `drop_fn` from `T` (`ScratchColumn` passes `None`, KC-16 passes `T`'s glue). **Identity without an id:** see the paragraph below the table. **Stagger:** one process-global `NEXT_STAGGER: AtomicU32`; `for_type` takes `fetch_add(1)` and `reserve(width)` takes `fetch_add(width)`; stagger = `(n % POOL_STAGGER_LINES) × CACHE_LINE_SIZE` (`[J]…/ecs/constants.rs:214-215`). **World scratch frames (D-R2a):** `pub(crate) struct WorldScratch`, a field of `EcsMaster`, with one `FrameStack<T>` per element type of the 15 KF-05 rows: `Entity`, `ComponentId`, `u32`, `u64`, `(Entity, u32)`, `(ComponentId, u32, u32)`, and `A64` (a `#[repr(align(64))] [u8; 64]` block). `FrameStack<T> { warm: [Option<ScratchColumn<T>>; 2], len: u8 }`; `take()` returns a warm column **by value** (a fresh `for_type` on the `#[cold]` empty path); `give(col)` clears the column and pushes it back, or drops it when the stack is full | untracked `ComponentPool` | columns: system-private (`Local`) or resource-owned; `WorldScratch`: `&mut EcsMaster` only | none on hot paths. `ScratchColumn::new(id, rows)` (`[J]…/scratch/scratch_column.rs:88`) stays for pinned-id users until the migration ends. `WorldScratch` adds 7 × (2 × 128 + 8) B ≈ 1.8 KiB to `EcsMaster`, zero-initialised, no syscall (UG-15 leg (3) re-blessed in D-R2a) | the `scratch_ids.rs` band (U-2); `register_asset_layout::<u32>` borrowing (`[J]crates/boyko_render/src/mesh_draw.rs:429`); unblocks KF-01 (324 rows) and KF-05 (15 rows) |
| **KC-11** | Untracked plain dense (`ticks = "none"`), compile-time refused | physics D10 / K2 table (`PHYSICS-ECS-UNIFICATION-DESIGN.md:1690-1705`) | DenseStore, untracked | unchanged | const-asserts only | **conditional**: built only with a verified consumer (K2 was decoupled in rev 4, `:2690`) |
| **KC-12** | Dense groups + anchor/binder + chain | physics §8/§9 read rev 2 → P → P4 → P5 plus Erratum E1 (`:3796, 3778-3780`): `StorageKind::Group`, `GroupColumn` (not `Component`), `#[dense_group]` emitting `<G>ChainKey` (pub(crate) constructor), `GroupHead<G>` / `GroupTail<G>`, `open_chain(&key)` / `close_chain(&key)`, `GroupSlot<G>`, `GroupRef<T>`, `BindToken`, `anchor_mask`, `DEAD` constants; **new:** `type Release: ReleasePolicy`, with marker types `Immediate`, `Chained` and `Stamped`, each carrying `const KIND: Release` (`enum Release { Immediate, Chained, Stamped }`, U-3). A Stamped-only API is bounded `G: DenseGroup<Release = Stamped>`, so misuse is a type error before monomorphisation (E0271), and no fixture depends on whether trybuild checks or builds. The erased paths cannot see `G`: `anchor_transition` iterates `DenseRegistry::groups: [Option<DenseGroupStore>; 8]` (`PHYSICS-ECS-UNIFICATION-DESIGN.md:2363-2366`, `:2393`). So `DenseGroupStore` carries `release: Release` (1 B), written once at `ensure_group` with the value `<G::Release as ReleasePolicy>::KIND`, carried by the group registration's erased descriptor, and read by `anchor_transition` and the first-op flush. Removal: `Immediate` releases at once; `Chained` defers iff `chain_open` (rev-5 D14); `Stamped` always defers and stamps `died`. `clear()`: `Chained` keeps rev 4's rule; `Stamped` moves every live slot to `dying` with a stamp | inline group store: untracked pools with contiguous ids (K1 cohort); `dying`/`free` as VmColumn stacks | rev-5 D14: removal defers iff `chain_open` (Chained) | two byte loads and one compare per structural op (the anchor masks, `PHYSICS-ECS-UNIFICATION-DESIGN.md:1413`); one compare of the `release` byte, only inside the `#[cold]` `anchor_transition` (`:1377`) and the first-op flush, so 0 on every op whose anchor mask is unchanged; query terms archetypal on the anchor, no storage-kind branch | row-keyed physics state (defect A class); KF-19 (14), KF-22 subsumed; `clear()` fix X-17 |
| **KC-13** | Stamped deferred release | **(a) Horizon form, D-S3(iii).** `GroupHead<G>::release_dying_before(&mut self, key: &G::ChainKey, horizon: Tick, visit: impl FnMut(u32)) -> u32`. It is bounded `G: DenseGroup<Release = Stamped>` and debug-asserts that `horizon` is not newer than the caller's `this_run` (wrap-aware). It releases exactly {dying : `died` strictly older than `horizon`}. Per slot: `visit(slot)`, then the kernel frees the slot's span-typed group columns, then the DEAD fill. Released slots are appended to `free` in dying order; retained entries are compacted in order; it returns `len()`. **(b) Teardown form, D-E9.** `EcsMaster::release_dense_group_at_teardown::<G>(&mut self, key: &G::ChainKey, token: &TeardownToken, visit: impl FnMut(u32)) -> u32`. Same bound, so it is refused for `Immediate` (no dying entries) and for `Chained` (no non-kernel per-slot resource to visit; its dying slots are released by `open_chain` or dropped with the store). It releases every dying entry in the same per-slot order. **The token.** `pub struct TeardownToken(PhantomData<*const ()>)`: private field, no public constructor, not `Clone`/`Copy`/`Default`, `!Send + !Sync`. It is minted only by KC-27's teardown driver, which runs when no schedule runs and passes `&TeardownToken` to rank callbacks of type `for<'t> fn(&mut EcsMaster, &'t TeardownToken)`. So no system can obtain one, and no callback can keep one. `release_dying_with` is **not built**: it has no consumer once the kernel frees spans (U-3) | `died: VmColumn<Tick>` for Stamped groups only | removal stamps `current_tick()` in apply windows / state transitions (X-25); `clear()` stamps and never releases a Stamped slot | Immediate/Chained: KC-12's cold `release` compare only; `GroupHead` methods are monomorphised on `G::Release` | `Assets<T>` retire rows (KF-49, 4 rows; KF-37 superseded) |
| **KC-14** | `par_iter` / `par_for_each_chunk` over mixed table + dense + `GroupSlot` terms | physics K4 | dense pointers resolved once per chunk | existing access model | none | compile-reject of dense `par_iter`; KF-20 (6 rows) |
| **KC-15** | `SegmentedColumn<T: Copy>` (K7 = EK14) | ED15 + N5 (`ENGINE-RUNTIME-ECS-DESIGN.md:415-451, 2039-2046`): pow2 classes, `SpanRef<T>{ptr, len, class}` held by the owner, `SpanRef::DEAD`, relocate on grow, LIFO class free lists (VmColumn<u32>), no compaction, one column per owning type / group column; **new:** span-typed group columns own their column inside the group store, and every release point frees spans before the DEAD fill | untracked `ComponentPool` + VmColumn free lists | writes only under `&mut` of the owner (apply windows, owner write param) | none | `RaggedColumn` / `CsrColumn` / `VmJagged` aliases; KF-03 (40); Heap `HeapVec` clients (U-1) |
| **KC-16** | Owning column (non-`Copy`, drop glue) | `OwnedColumn<T>`: `new(rows)` (lazy), `push`, `take_at`, `retain_ready(epoch)`, `drain_where`, `clear` (drops) | untracked `ComponentPool` built by KC-10's `new_untracked_raw`, with `drop_fn` = `T`'s glue passed by value: **0 `ComponentId`s** (U-8) | single writer | none | KF-02: the 7 rows in `boyko_ecs` and `boyko_render` (`RUNTIME-DATA-LEDGER.md:898`); six retire-lane types share one shape (`:912`). The `boyko_rhi_vulkan/src/memory.rs:728` row retires in D-E15, because it needs the rhi → `boyko_ecs` edge (KF-36). The `boyko_utils/src/sparse_map/sparse_map.rs:10` row retires in D-R2d, after D-M1 moves `SparseMap` |
| **KC-17** | Erased record column | `RecordColumn`: Layout-aligned records + `&'static` vtable (`drop_in_place`, `LAYOUT` const); handle `ErasedSystem { data: NonNull<u8>, vtable: &'static V }` (16 B); per-`ResourceId` record reuse on re-insert | address-stable `ByteColumn` records | dispatcher/`&mut` writer; readers through handles | one indirection, as today (`[J]…/schedule.rs:1380/1215` vtable call unchanged) | `Box<dyn System>` (`[J]…/schedule/system_box.rs:83`), `Box<dyn FnOnce>`, `Resources/NonSendResources::insert` boxes (ledger TSV rows 749, 751), `TermList` boxes (rows 729-735), `CommandQueue.bytes`, `ErasedKindBuffer.data`; KF-07 (6), KF-06 byte users (12) |
| **KC-18** | Kernel-internal tables and lists | fixed-capacity `VmColumn<T, TableOwner>` tables (U-7; they are UG-04's `Table` producers); intrusive dead-slot LIFO where the dead slot has a free word; VmColumn stack where the slot must hold DEAD (KF-41); `slot+1` zero encoding (`EntitySlotMap`); `SparseMap` moved into `boyko_ecs` on VmColumns | VmColumn / reservation | `&mut` writer | none | `DenseStore.free` (`[J]…/dense/dense_store.rs:126`), `EntitySlotMap.slots`, `LiveBitmap.words`, `ArchetypeBundle.{free_slots, id_to_slot, slots}` (TSV 557-559), `QueryStateCache.slots` (689), `bundle_archetype_cache` (685); KF-41 (4) |

TSV row numbers are line numbers of `[M]docs/memory/runtime-data-ledger.tsv` (line 1 is the header).

**KC-10: identity of a registry-free pool (critic open question 2).**
- The pool's `component_id: usize` field (`[J]…/memory/component_pool.rs:233`) becomes
  `component_id: u32` + `stagger: u32`. That is the same 8 B, so the 128 / 144 B pins hold (`:57`, `:62`).
- A registry-free pool stores `NO_COMPONENT_ID = u32::MAX`.
- `grow_rows` reads `self.stagger` instead of re-deriving it from the id (`:627`, `:754`).
- `component_type_id` (`:240`) comes from `TypeId::of::<T>()`.
- The layout is already stored by value (`:195`), so no layout table is needed.
- The drop glue is a constructor argument, so KC-16 needs no id either (U-8).
- **Lazy state (critic pass 2, O3).**
  - A `Reserve::Lazy` pool copies `VmColumn`'s lazy form (`[J]…/memory/vm_column.rs:83-89, 103-105`):
    its three base pointers are `NonNull::dangling()`, the reservation is absent, and
    `committed_rows = 0`.
  - The first `add` therefore fails the one warm compare it already has and enters the cold
    `grow_rows`. `grow_rows` reserves the address space and writes the bases once, under
    `&mut self`; that is `VmColumn`'s write-once-at-materialisation rule. A dangling base with
    `len = 0` is never dereferenced.
  - The absent reservation is `VmReservation::UNRESERVED` (dangling base, length 0; its `Drop` skips
    a zero length). *Writer check (00 §9 V-33): neither exists at `d552be05`. `VmReservation`
    (`[J]…/memory/vm.rs:85-97`) has no such constant, and its `Drop` (`:263-297`) releases on every
    arm with no length test, so D-S2 adds both the constant and the zero-length skip, on all three
    arms.* It is **not** a new `PoolBacking` variant: `Host` already spends
    `VmReservation`'s only niche on `Device` (`[J]…/component_pool.rs:46-54, 80-94`), so a third
    variant would grow the enum. The 128 / 144 B pins (`:55-64`) stay, and they are the gate.
- **The stagger is taken at construction, including for `Default`.**
  - It is therefore deterministic whenever construction is (schedule build and plugin setup).
  - A `Default` built inside a system body on a worker gets a timing-dependent cache line. That
    costs performance only, and MQ-16 measures it.
- `component_id()` (`:1465`) debug-asserts that it is not the sentinel. A registry-free pool is a
  private field of `ScratchColumn` and never enters an archetype bundle, so the debug-only caller at
  `[J]…/component/component_pool_bundle.rs:431` (under `#[cfg(debug_assertions)]`, the only
  pool-receiver caller in `[J]crates`) cannot reach one.

**KC-10: the stagger policy (question 3).**
- Same-type singles and cohort members get consecutive cache lines.
- The seed is deterministic whenever construction is single-threaded, which schedule build and
  plugin setup are.
- MQ-16 measures both the cohort case and the singles case.

**KF-05 world scratch frames: borrow model and re-entrancy.**
- **Why by value.** The 15 KF-05 rows sit on `&mut EcsMaster` paths that migrate entities while the
  scratch is live (clone subtree, prefab instantiate, trigger DFS). P27's `FrameGuard` handed out
  `&'w EcsMaster`, which forbids exactly those calls, and P34 deleted it. A frame taken **by value**
  borrows nothing from the world, so nested structural calls and user hooks re-enter
  `&mut EcsMaster` freely, with no `unsafe`.
- **Re-entrancy.** A nested `take` gets the second warm column, or a cold fresh one. Re-entrancy
  depth is unbounded and correct; warmth is bounded at depth 2.
- **Panics.** A panic between `take` and `give` drops the column, which releases its reservation,
  and leaves `len` consistent. The stack heals on the next `give`.
- **Cost.** Two 128-B moves per frame, against today's `Vec::new()` plus growth per call.

### Layer 3 — registry and identity

| KC | Capability | API shape | Storage / backing | Threading | Default-path cost | Deletes |
|---|---|---|---|---|---|---|
| **KC-19a** | Registry bug fixes (rung D-S1(i)) | **P39:** the mint **skips** occupied slots in `try_register_dynamic` and `register_new`; `dynamic_slot_occupied_panic` deleted (`[J]…/component_registry/mod.rs:998`); `#[cold] pub fn id_space_census()`, printed by `register_layout`'s different-type panic. **P40, tag path:** `intern_or_mint_tag(name, kind)` runs lookup → capacity check → mint → `set_storage_kind` → name insert under the one mutex, and the mint-then-reclassify at `[J]…/tags.rs:134-141` is deleted. **32.2:** `tag_by_name` is kind-exact. **NW4:** no silent `_ => Table` decode; `ALL_STORAGE_KINDS`. **Pre-existing defect:** `install_dense_storage_kind::<C>(id)` (`[J]…/mod.rs:774`), a safe `pub fn` taking any id, gains a release check `LAYOUTS[id].type_id == TypeId::of::<C>()` on its once-per-type path | process statics (unchanged) | `NEXT_ID` CAS `Relaxed` (bound); mutex-guarded intern (blessed exception); P40's publication rule (`ALLOCATOR-DESIGN-SPACE.md:3701`) | **0 instructions on the success path** (`:3662`); one `TypeId` compare per dense type, once per process | shipped defect 32.2 (`:3342`); the release reclassification a foreign id could trigger (`mod.rs:445-459`) |
| **KC-19b** | Modding seam common to every live option (rung D-S1(ii); 05 MS-03) | **`ModSeam`:** a `#[doc(hidden)] pub unsafe trait` with no methods, in a `#[doc(hidden)]` seam module, never re-exported at a crate root, with no implementor in any kernel crate (implementors: `boyko_mod_host`, `boyko_mod_api`, `boyko_mod_registry`, census test crates). **Four occupancy value readers**, `pub fn`s generic over `ModSeam`, returning by value the query, bundle, resource and event counters (`[J]…/iters/query/query_type_registry.rs:98`, `[J]…/bundle/bundle_type_registry.rs:93`, `[J]…/resources/resource_registry.rs:127`, `[J]…/events/event_registry.rs:93`). Component occupancy is KC-19a's `id_space_census()`, an engine item. **Moved out in rev 5.1:** P32 R1's rename (`TAG_NAMES` → `DYN_NAMES`), P40's separate sized body `intern_or_mint_sized` and the class-A entries (`ComponentLayout::new_dynamic`, `try_register_dynamic_by_name`, `dynamic_by_name`) are option-A items, 05 MS-02b, built at modding Stage 3 under S-1. Their rev-5 constraint stands: the tag body is not generalised | process statics (unchanged) | `Relaxed` loads, values only | **nothing compiled** (I-4, rule S-1): with no implementor, the readers are never instantiated and have no object code, at any profile. **Residual (predicted):** the readers make `QUERY_NEXT_ID` and `BUNDLE_NEXT_ID` reachable from other crates (05 §1 (b)); the fat-LTO game binary internalises them. **Proven** by UG-15 strict with legs (7) and (7b) against D-S1(ii)'s cut commit, not assumed; an item that moves a leg leaves the kernel for `boyko_mod_host` | — (capability) |
| **KC-20** | Generic component id mint (EK22) | `component_id_for::<T>(install: fn(u32))` over `TypeIntern`; the derive accepts generics | process intern | as `resource_id_for` | generic types only: one hash + one load on uncached paths; non-generic unchanged | rust#22991 id collapse (`ActionState<A>`) |
| **KC-21** | By-id structural seam (KF-47) | `add_component_by_id(entity, id, bytes) -> AddOutcome`, `remove_component_by_id`, `mark_component_changed`, `EnableTagId::try_from_component_id` (`[R]` via `ALLOCATOR-DESIGN-SPACE.md:3537-3542`) | existing migration core + sibling helper | `&mut EcsMaster` | 0 (shipped with engine clients) | per-crate by-id paths; allocator K-MOD-11 (P38) |

### Layer 4 — schedule and world services

| KC | Capability | Source | Default-path cost | Deletes / unblocks |
|---|---|---|---|---|
| **KC-22** | Exclusive system context: `(last_run, this_run)` + `Local<S>` | EK2 / KF-44 | none | KF-44 (21 rows) |
| **KC-23** | Change surfaces: structural log (EK3, 2-frame horizon, exact overflow), dense tick API (EK6, refuses group/untracked ids), group edit log (EK6g, opt-in `EDIT_LOG`), KF-25 accessors | engine ED17 + P-ED17, P-§9 | 0 when unread (opt-in per type) | hook queues (KF5 dropped); KF-25 (3) |
| **KC-24** | Resource change ticks `#[resource(ticks)]` + `TickEpoch` | EK5 | opt-in | FIF tick re-basing (O2) |
| **KC-25** | Enable initial polarity | EK4 / KF-17 | none | light seed (8 rows) |
| **KC-26** | Event lanes: per-type overflow policy (reject / drop-oldest / grow / coalesce), `#[event(swap)]`, `OsEventSink<E>`, `Commands::trigger` | EK7, EK8, KF-23 | per-type const policy | `InputRing`; KF-23 (5); KF-24 stays rejected |
| **KC-27** | Schedules: startup `ScheduleBuilder`, `CoreSchedule::Render`, runner `fn(&mut App) -> AppExit`, rank-ordered NonSend teardown. The teardown driver runs after the last `Schedule::run` has returned. It is the **only** mint site of `TeardownToken`, and it passes `&TeardownToken` to each rank callback (`for<'t> fn(&mut EcsMaster, &'t TeardownToken)`). The asset rank idles the device, calls KC-13(b) per `Stamped` group, then drops the lanes (`ENGINE-RUNTIME-ECS-DESIGN.md:2454`). KC-13(b) lands in this rung | EK9, EK10, EK11, KF-29, KF-30; ED16 | none | boxed startup closures; KF-29 (2), KF-30 (1) |
| **KC-28** | Hidden (default-excluded) entities; system, set and observer entities; prefab templates as entities | EK18, KF-13/14/15 | archetype-flag test at cached archetype match, 0 per row (U-16); executor tables stay compiled (KF-14 decision) | KF-13 (8), KF-14 (26), KF-15 (5); KF-12's 18 rows become edge entities (U-15) |
| **KC-29** | Relation collections: `Ordered` (EK15a), `CountOnly` + `COUNTED_TARGET` despawn redirect in `delete_entity_core` (EK15b), `Segmented` on KC-15 with `type Ctx` + derive `release_fn` (EK15c) | engine P-§9 | cold paths; `CountOnly` is `const ITERABLE = false` | `Children(Vec<Entity>)` (`[J]crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:120`); KF-11 (4), KF-48 (1) |
| **KC-30a** | Seams: enable bits in serialize/prefab/reflect (EK19); `SaveCursor` over `ScratchColumn<u8>` (KF-08); durable resource column (KF-04) | EK19, KF-04, KF-08 | cold | KF-04 (10), KF-08 (9) |
| **KC-30b** | Structured asset error: `AssetError::{Decode, Io}` carry a `boyko_log` code, a `&'static str` and POD args, and formatting happens at emission (`RUNTIME-DATA-LEDGER.md:1064-1080`). The type change is compiler-forced, so every construction site migrates in the same rung | KF-10 | cold (error paths) | KF-10 (77) |
| **KC-30c** | Loader decode context: `AssetLoader::decode` receives an ECS-owned context of reusable `ScratchColumn` lanes plus per-type staging payload lanes, and `Asset::Cpu` becomes a Copy `{start, len}` range record (`:1041-1062`). Scope: the kernel half and the loader signatures. The codec half (`boyko_image`, `boyko_fontbake`) stays in F1 | KF-09 | cold (load path) | KF-09: the kernel rows plus the loader rows the signature forces (counted at the cut); the rest unblocked for F1 |
| **KC-31** | One kernel name table / StrInterner; allocation-free static descriptors | EK20, KF-26, KF-43 | cold | per-crate interners; KF-26 (4), KF-43 (5) |
| **KC-32** | Kernel asset hot reload | EK21 | cold | `UiTreeView` sinks |
| **KC-33** | Query into kernel column; world read beside Commands; `ComponentPool` inert row | KF-27/28/39 | none | KF-27 (12), KF-28 (6), KF-39 (1) |
| **KC-34** | Device side: device-column meta fold (EK16/KF-40); RHI owns kernel columns via an rhi → `boyko_ecs` edge (KF-36) | EK16, KF-36, KF-40 | none | KF-36 (44), KF-40 (2) |
| **KC-35** | Split apply window | KE17 (`[J]docs/MEASUREMENT-QUEUE.md:83-109`) | **conditional**: built only if MQ-05 shows `split_sim` more than 2 pp from `barrier_sim`; must keep EM2′-K | — |
| **KC-36** | Deterministic in-window apply order. `apply_window_drain` (`[J]…/schedule/schedule.rs:783-860`) first drains the completion queue into a preallocated window bitset in `executor_scratch`, then applies in ascending system index by word scan (`tzcnt`), instead of in pop order (`:799-830`). The window fires only when every running system has completed (the gate at `:678-680`, restated in the SAFETY comment at `:811-814`), so the window's set is fixed by the dispatch history and only the pop order depends on timing; ordering the set removes that dependence. The set is also independent of W: the scan at `:1079-1132` has no worker-count term, and every system it marks runs before the next window. If KC-35 is ever built it must keep this property | physics P-§14 (`PHYSICS-ECS-UNIFICATION-DESIGN.md:2021`), X-14 | O(k + ⌈n/64⌉) per window instead of O(k): at most 16 extra word loads for n ≤ 1024 systems; no sort | X-14's thread-timing dependence of apply order, command order, the order of hooks **across systems**, table-row order and dense/group slot assignment within one window, for every storage. The order of hooks **within one structural op** follows `ComponentId` values and is KC-37 (c), rung D-E21. **Not entity ids:** they are claimed on the worker during the phase (`[J]crates/boyko_ecs/src/ecs/core/system/params/commands.rs:169-173` → `entity_counter.rs:200-219` → `entity_reservoir.rs:160-190`), before any window (U-20) |
| **KC-37** | **Simulation replay determinism (owner Q-9).** The kernel's ordering guarantees make a Fixed tick a function of (world, tick inputs), plus one generic hash trait. The contract, the boundary rule and the hazard register are in §2.1 | owner Q-9; hazard inventory of 2026-09-17 (H-01..H-15) and the architect's H-16..H-19 | (a) is KC-36. (b) writer lanes: one TLS read fewer per `send`. (c) declaration-order hooks: cold path only. (d) `every_tick` swap: one per-type swap per Fixed substep, only for opted types. A game with no `every_tick` type runs today's substep closure; the cost is one `bool` branch per frame, outside the loop (D-E8). (e) and (f): no code. (g) the trait: no object code until instantiated (UG-15 legs (7), (7b) on RP-1). (i) D-E22's access visitor: no object code until instantiated (legs (7), (7b) on D-E22). (j) D-E23: no added lookup (U-25). Keys, recording and hashing: 0, because they live in `boyko_replay` | X-14's remaining halves (event order; within-op hook order); W-dependent event refusals (H-02); pacing-dependent Fixed→Fixed event latency (H-19); pacing-dependent `FixedTime` reads (H-20); `ComponentId`-ordered hooks on bundle-less ops (H-04, despawn half) |

**KF-46 (window entities) has no kernel delta.** The ledger files it as a host capability provided
by `boyko_app` on the kernel's existing dense address stability (`RUNTIME-DATA-LEDGER.md:1643-1646`).
Its 13 rows retire in engine HO3 (Phase E), which is where the plan's exit criterion counts them.

**Render, not kernel:** RF1, the FIF-mirror protocol (engine ED3), is owned by `boyko_render`.

### 2.1 KC-37: the replay determinism contract (owner Q-9)

**Requirement (owner, 2026-09-17).** A replay loads and plays on any machine, and entity ids need not match. The same binary reproduces the same simulation on any machine and any worker count from the same recorded inputs. Replay files refer to entities by stable replay keys, never by `Entity` ids (00 §7).

**C, the contract.** A *replay session* is an `App` whose `CoreSchedule::Fixed` schedule is the simulation (`[Jw]crates/boyko_ecs/src/ecs/core/app/app.rs:64-72`). Suppose three things are the same:
1. the binary (the header's build identity, below);
2. the initial world: the startup path, run with the same startup inputs (seed, startup asset content, mod set) and no save loaded (Q-11);
3. the per-tick replay inputs.

Then the replay-hashed state after every Fixed tick is bit-identical across:
- machines: any CPU the binary runs on, i.e. any x86-64-v3 CPU (`[W].cargo/config.toml:99-113`);
- worker count W;
- entity-id assignment;
- frame pacing, i.e. how many substeps each frame runs;
- Main-schedule activity that obeys rule B.

Entity ids are outside the contract (U-20). GPU-resident archetypes are outside it too, because CPU queries skip them (`[Jw]…/iters/query_state.rs:249-251`).

**B, the boundary rule** (inside a replay session).
- **What a Fixed system may read:**
  - replay-hashed state; it exists only on keyed entities (a hashed component on an unkeyed entity is red, below);
  - `FixedTime`, which after D-E23 holds only tick-level values: `timestep`, `delta`, `delta_secs`, `elapsed` (U-25);
  - **the tick inputs:** `ActionState<A>` for every `A` registered with `ReplayPlugin::actions::<A>()`, which the replay's tick-start system captures (record) or restores (play) before any Fixed reader (RP-2, below), and `SimInputs<E>`;
  - resources marked `#[replay(constant)]`, written before tick 0 and never after.

  A Fixed read of `Time` is refused. `Time` carries the frame's `delta`, and after D-E23 also the frame-level fixed-loop values `fixed_steps`, `fixed_overstep` and `fixed_overstep_fraction`. Today those three are `FixedTime` getters that depend on pacing. `steps_this_frame` is written only after the loop, so Fixed code sees the previous frame's count (`[Jw]crates/boyko_ecs/src/ecs/core/time/fixed_time.rs:125-171`; `[Jw]…/time/fixed_loop.rs:82-83`; H-20).
- **Who may write replay-hashed state:** nothing outside Fixed. That excludes Main systems, exclusive systems outside Fixed, host code, and observers triggered from Main. The only exceptions are startup and the replay tick-start system's restore.
- **Move clause (U-28).** Nothing outside Fixed may spawn or despawn a keyed entity, or insert or remove a table (signature) component on one.
  - Every keyed entity carries `ReplayKey`, so a keyed entity's table holds only keyed entities, and every Fixed query iterates rows.
  - A Main-side move would reorder those rows for every order-dependent Fixed system, not only for physics (H-16).
  - Exempt: non-fragmenting components (dense, bitset) that no Fixed system reads (`[Jw]crates/boyko_ecs/src/ecs/core/component/component.rs:67-78`).
- **Events:** every event type that Fixed reads swaps every tick (item (d)).
- **Timestep:** unchanged within a session. It is recorded in the header and checked at each tick start.
- **Archetype ids (H-17): detected, not refused. No kernel field or branch is added.**
  - `clear()` resets the id counter (`[J]…/archetype_master.rs:968-971`).
  - A removed id returns only through `add_existing_archetype` (`:477`, `:569-572`), which has no caller in `[Jw]crates`.
  - Both `clear()` and `remove_archetype` bump `ArchetypeMaster::structural_generation()`, a public read (`[Jw]crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:37-52`, `:594`), reached through `EcsMaster::archetype_master()` (`[Jw]…/ecs_master/ecs_master.rs:575`).
  - The replay's tick-boundary systems compare it with its value at session start. Any change raises the coded H-17 panic, since the session's keys and archetype order are then void.

**Kernel obligations.** These hold for every game, whether or not it records.
- **(a) Apply order:** KC-36 (D-E0).
- **(b) Event order.** Lanes are keyed by writer (D-E20).
  - Today the lane is the running thread's worker id (`[Jw]…/system/params/event_writer.rs:131-133`). So two systems on one worker interleave, and which events a full lane refuses depends on W (H-02).
  - `EventWriter` holds `&'s mut` state (`:89-91`, `:110`), and the scheduler never runs one system instance on two threads at once. So a per-writer lane has one writer at a time, with no lock.
- **(c) Hook order within one op does not depend on `ComponentId` values** (D-E21).
  - **Inserts:** hooks fire in the bundle's declaration order, with required components in plan order.
  - **Ops with no declaring bundle** (despawn, clone/materialize, an archetype-driven remove): hooks fire in canonical type order, i.e. `ComponentLayout::type_name` bytes, ties broken by `TypeId` (U-26; `[Jw]crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs:109-121`, `:149`, `:173-178`).
  - **Today** both follow ids: `[J]…/commands/migration_helpers.rs:1022-1107` for inserts, and `[Jw]…/ecs_master/entity_api.rs:733-805` for despawn, which iterates `component_ids()` for its on_despawn, on_replace and on_remove passes. Ids are minted on first use, so the order depends on when Main first touched a type (H-04).
- **(d) Fixed→Fixed event latency** is fixed by `#[event(swap = "every_tick")]` (D-E8). Today readers read only the flat buffer filled at the swap (`[Jw]…/events/event_buffer.rs:185-193`, `:234-238`), and the swap happens once per frame, gated on substeps (`[Jw]…/app/app.rs:713-720`), before the substep loop (`:725-735`). An event sent at tick k is readable at k+1 only when a frame boundary falls between the two ticks (H-19).
- **(e) Order sources.** Queries iterate archetypes in ascending `ArchetypeId` (`[Jw]…/iters/query_state.rs:236-256`), which is creation order.
  - Among sim archetypes, that order is decided by the simulation, as long as rule B holds and no id is recycled. No production caller of `remove_archetype` exists: ripgrep over `[Jw]crates` finds only its definitions and in-file tests (`query_state.rs:585` onward). *Writer check (00 §9 V-62): one further test caller, `iters/query/state.rs:1272`, is inside `#[cfg(test)]`; the conclusion holds.*
  - No kernel order may depend on an `EntityId`, `ComponentId` or `NameId` value. `TypeId` enters only as U-26's tie-break under equal `type_name`, where it is fixed within one binary. D-E21's cut lists every other site.
- **(f) Parallel regions.** `par_iter` chunking depends on W (`[J]…/iters/query/par_iter.rs:119-125`), and stays so. The kernel merges no per-chunk output. `Commands` and `EventWriter` are `&mut` and never cross a chunk (`[Jw]…/params/commands.rs:169`; `event_writer.rs:89-91`). A game's per-chunk accumulation must be commutative and associative, as the hash below is. The hash merges its per-chunk sums through per-worker slots, because `par_for_each_chunk` has no reduce (`[Jw]…/iters/query/query.rs:682`).
- **(g) One generic trait:** `boyko_utils::replay_hash::{ReplayHash, HashSink}` (RP-1). Its methods are generic over the sink, so no object code exists until `boyko_replay` instantiates one. `Tick` and `NameId` have no impl, so hashing a `Tick` field is a compile error unless the field is `#[replay(skip)]`. `boyko_utils` is chosen because it is the leaf that `boyko_math` can depend on: `boyko_ecs` has no `boyko-*` dependency on `boyko_math` (`[Jw]crates/boyko_ecs/Cargo.toml:6-20`).
- **(h) Nothing else at run time.** No kernel field, counter, branch, hook or static exists for keys, recording or hashing.
- **(i) One generic read surface (D-E22).**
  - `Schedule::for_each_system` and `App::for_each_system_access` visit every built system with its schedule, post-topological index, name, exclusivity and `&Access`.
  - Today no route from a built schedule reaches that data: `Schedule`'s public surface is `run`, `len`, `is_empty`, `may_defer` and `gpu_barrier_inputs` (`[Jw]…/schedule/schedule.rs:286`, `:561`, `:567`, `:582`, `:1436`), and its `systems` are `pub(crate)` (`:122`).
  - The methods are generic over the visitor, so they have no object code until `boyko_replay` instantiates them.
- **(j) Frame-level fixed values in `Time` (D-E23, U-25).** No lookup is added.

**Library obligations (RP-0).** `boyko_math::det` and `boyko_math::rng`, a counter-based stream `(seed, key, counter) → u64` (U-23). The census `tests/sim_math_census.rs`. The census also covers `boyko_scene`, with its five Main-only camera calls on the owner-signed allowlist (`[J]crates/boyko_scene/src/camera.rs:483, 727, 728, 926, 927`). Its walker is a public function of the gate crate, so a game's own test can run it over the game's crates.

**The crate `boyko_replay` (RP-2)** is optional and additive, like the modding crates (05 §1 (a)). A game that does not depend on it compiles none of it.

- **Session.**
  - `ReplayPlugin { mode: Record | Verify | Play, … }` must be the first plugin added that registers Fixed or Main systems.
  - The topological sort is FIFO in registration order (`[Jw]…/schedule/schedule_builder.rs:1035-1075`). So the plugin's tick-start system is Fixed index 0, and its frame-hash system is Main index 0.
  - In every mode, the tick-start system declares a write of every tick input. Every Fixed reader of a tick input therefore conflicts with it, and the dispatch scan runs it first.
  - The boundary report checks both indices.
  - The plugin inserts `Time` paused. `App::finish` keeps a `Time` inserted at config time (`[Jw]…/app/app.rs:597-605`).
- **Recorded, per Fixed tick:**
  - **`TickActions`:** for every registered `A`, the value of `ActionState<A>` as Fixed sees it at tick start: the live sets, the consumed set and the frozen fixed snapshot, delta-coded against the previous tick.
    - **Capture point.** The value is captured after Main's freeze (`[Jw]crates/boyko_input/src/action/process.rs:72-76`) and after any Fixed `consume` in the previous tick (`[Jw]…/action/state.rs:186-193`). A press that a frame's N substeps all see (the sticky-edge model, `state.rs:10-23`) is therefore recorded as N ticks that see it.
    - **Play mode.** The tick-start system restores the value. Main still sees live input, because Main re-derives the live sets from scratch (`process.rs:121`) before any gameplay reader (`:27-28`).
    - **`boyko_input` addition.** One pair of methods on `ActionState<A>`, `replay_capture` and `replay_restore`. They are methods of a generic type, so they have no object code until instantiated (RP-2's lock set).
  - **`SimInputs<E>`:** the tick's payloads, each in its `Repr` form. `#[derive(SimInput)]` generates `Repr` with every entity reference as a `ReplayKey`. A field whose type can hold an `Entity` without a mapping fails to compile.
  - **Hash:** every `hash_interval` ticks, the tier-1 hash.
- **Not recorded:** frame deltas, W, entity ids, Main-schedule state.
- **Header.**
  - Format version.
  - **Build fingerprint:** rustc version and commit hash, target triple, profile keys, `target-cpu`, `BOYKO_*` axes, `Cargo.lock` hash and engine commit. It comes from the `boyko_build_id` generator shared with 05 §3.3.
  - **Executable image hash**, read once when recording or playing starts.
  - **Determinism-relevant switches:** `FixedTime`'s timestep; `SdfNarrowphaseKernel`, which degrades to scalar on non-AVX2 builds (H-10); the physics default-off switches of MQ-06; the MXCSR control bits at session start (H-21).
  - Seed; content hashes of the startup assets; the mod set's fingerprints in load order; origin = startup.
  - The player refuses a mismatched fingerprint or image hash (Q-10 default).
- **Keys (U-24).** `ReplayKey(u64)` is a component. The key's `tick` is the **replay tick**: the number of Fixed ticks since the startup gate opened. It is not the kernel `Tick`, which advances per system run in every schedule.
  - **Root spawns** go through `ReplaySpawner`, a `SystemParam` over `Commands` plus a per-tick ordinal.
    - Key layout: `0 | tick (31) | slot (12) | ordinal (20)`.
    - The slot is the order in which systems holding a `ReplaySpawner` ran `init_state`. `App::finish` builds Main's schedule, then Fixed's, then runs the startup systems in registration order, all on one thread (`[Jw]…/app/app.rs:614-629`).
    - A `ReplaySpawner` in Main is a boundary-report red.
    - Startup spawns use tick `2³¹ − 1`, so their slots cannot collide with Fixed keys.
  - **Derived spawns** use `boyko_replay::spawn_derived(ctx, parent, bundle)`. This covers children, prefab and scene instances, clones, and entities spawned by hooks or observers.
    - **From a system:** through `ReplayCommands`, as one command that inserts the bundle and the key in one migration.
    - **From a hook or observer:** through its deferred world.
    - **Key:** `1 | mix63(parent_key, tick, c)`, where `c` is the number of derived keys already issued under `parent_key` in this tick.
    - **When `c` is read:** it is read and incremented when the key is issued, inside the apply window. Commands apply in KC-36's ascending system order and then in queue order, and hooks fire in D-E21's order. So `c` depends only on the simulation, and two spawn sites under one parent in one op or tick get distinct keys. The caller passes no index.
    - **Storage:** counters live in `DerivedKeyCounters`, an open-addressed table in a resource-owned kernel column, keyed by `parent_key`. An entry stamped with a different tick reads as 0, so nothing is cleared per tick.
    - An unkeyed parent is a coded panic.
  - `ReplayKey` opts out of cloning, so a clone never duplicates a key.
  - **`ReplayKeyIndex`** maps key → `Entity` for playback.
    - It is maintained by `ReplayKey`'s `on_add`/`on_remove` hooks.
    - It is an open-addressed table in a resource-owned kernel column: power-of-two capacity, load ≤ ½, backward-shift deletion, cold grow.
    - It is only probed, never iterated, so its layout never reaches results.
    - A duplicate key, or overflow of a key field, is a coded cold panic.
  - **Cost:** 8 B per keyed entity, one increment per root spawn, and one probe per derived spawn and per index insert or remove. These costs apply only in games that link the crate.
- **The state hash.**
  - **Scope:** the component types and resources the game registers with `app.replay_hash::<T>()`, plus those an engine plugin registers for its own simulation state.
  - **Tier 1:** `H = Σ mix(key, h(hashed components of that entity))` (wrapping add), plus the keyed-entity count, plus the resources' hashes in registration order.
    - The sum is commutative and associative, so `par_for_each_chunk` computes it per chunk.
    - **Merge form (critic pass 6, O2).** Each chunk adds its wrapping sum into a per-worker slot (`[CachePadded<UnsafeCell<u64>>; MAX_WORKERS + 1]`, the dispatcher's included), indexed by the worker id, and the slots are summed after the join. Each slot has one writer, the slots are padded, and there is no shared atomic.
    - The result depends on none of W, chunking, row order or archetype order.
    - A hashed component on an unkeyed entity is red.
    - **Floats** hash `to_bits`, with every NaN mapped to the canonical quiet NaN (H-13). +0 and −0 hash differently, because within one binary the sign of zero is fixed (H-09).
    - **`Entity`-typed fields** hash the referent's `ReplayKey`. A dead referent hashes the sentinel `u64::MAX`; liveness is simulation state. A live unkeyed referent also hashes the sentinel, is counted, and is red in gate sessions (critic pass 6, O10).
    - Fields are hashed, not bytes, so padding never enters.
    - The mixer is in-house integer code with known-answer vectors.
  - **Tier 2** (on a mismatch only): a table of per-component hashes per key, sorted by key and written to a dump file. The diff names the first differing key and component.
- **Cadence.**
  - When recording: every `hash_interval` ticks (default 64 for shipped recordings, 1 in gate sessions).
  - When verifying: at the recorded ticks. At the first mismatch, the verifier re-runs from tick 0 with interval 1 and dumps tier 2 at the first differing tick. A deterministic re-run needs no snapshot.
- **Tick-boundary checks (verify sessions only).**
  - **What is taken:** the tier-1 hash, a **move digest** `Σ mix(key, archetype id, row)`, `structural_generation()`, and the MXCSR control bits.
  - **When:** at Fixed index 0, before each tick. Between two ticks in one frame, the pre-tick value is the post-tick value of the previous tick. At Main index 0, after the frame's last tick.
  - **Comparison:** the Main-index-0 values against the next frame's first pre-tick values. Each difference is red and names the frame:
    - the hash, for a Main or host write (H-18);
    - the digest, for a move-clause breach (U-28);
    - the generation, for H-17;
    - the bits, for H-21.
  - **MXCSR.** Bits 6–15 are read with an inline-assembly `stmxcsr`, at tick start and in every hash chunk, so on every worker. They are compared with the session-start value. Only reads happen: Rust makes writing the register UB (00 §11).
  - **Cost:** one hash and one digest per tick and one per frame.
- **Boundary report.**
  - **Entry point:** `boyko_replay::boundary_report(&App)`, called after `App::finish`:
    - by `ReplaySession::begin` when the game drives the `App` itself;
    - by every UG-22 scene;
    - by a game's own test when the game records under the windowed host. That host calls `finish` itself (`[Jw]crates/boyko_app/src/runner.rs:638`) and gains no replay call, so a non-replay game pays nothing.
  - **Data:** it reads D-E22's visitor. `Access` exposes no set reads, so for each id it decides "reads" and "writes" with two `Access::conflicts_with` probes, a pure-write probe and a pure-read probe (`[Jw]…/system/access.rs:81-99`, `:214`).
  - **Checks:**
    1. The Fixed reads that intersect writes by other schedules or by exclusive systems are a subset of {tick inputs, `SimInputs<E>`, constants}.
    2. No Fixed read of `Time`; this covers H-20.
    3. Every Fixed-read event type is `every_tick`.
    4. No `ReplaySpawner` outside Fixed and startup.
    5. The tick-start system is Fixed index 0, and the frame-hash system is Main index 0.
  - Anything else is listed, and is red in the gate scenes.
- **Startup gate.**
  - The replay's Main system unpauses `Time` once the startup asset set has loaded. No Fixed tick therefore runs before tick 0, and `FixedTime::elapsed()` at tick k is k × timestep.
  - An asset that completes after tick 0 and changes hashed state must arrive as a `SimInput`.

**Hazard register.** Classes: X-M = differs by machine, X-W = by worker count, X-R = run to run, ID = depends on id values, X-B = by build.

| H | Class | Hazard (tree) | Rung | UG-22 arm that catches it |
|---|---|---|---|---|
| H-01 | X-W, X-R | The apply window applies in completion-pop order (`[J]…/schedule/schedule.rs:799`, `:828`) | D-E0 (KC-36) | W; pacing |
| H-02 | X-W, X-R | The event lane is the worker id; refusals depend on W (`[J]…/params/event_writer.rs:131`, `:162`; `[J]…/events/event_dispatcher.rs:274-283`, `:632`; `[J]…/events/event_buffer.rs:340-360`; `[J]…/events/event_config.rs:25`) | D-E20 | W (S-R2) |
| H-03 | ID | Physics row identity matches generation-less ids (`[J]crates/boyko_physics/src/row_identity.rs:18-26`, `:519`, `:580`) | A1b; U5/U7 delete the code | id perturbation (S-R1) |
| H-04 | X-R | Hooks fire in `ComponentId` order, and ids are minted on first use (`[J]…/component_registry/mod.rs:920-921`; `[J]crates/boyko_macros/src/bundle.rs:345`). Inserts fire at `[J]…/commands/migration_helpers.rs:1022-1107`; despawn fires in `component_ids()` order at `[Jw]…/ecs_master/entity_api.rs:733-805` | D-E21 (declaration order for inserts; canonical type order for bundle-less ops, U-26) | Main churn, minting the r3 pair in reverse (r3) |
| H-05 | X-R | `NameId` values follow intern order (`[J]crates/boyko_scene/src/identity.rs:106-124`) | Rule: never an order key; no `ReplayHash` impl (RP-1). D-E21's cut audits | Main churn (interns) |
| H-06 | ID | `visibility_sync` resolves a bare `EntityId` at apply (`[Jw]crates/boyko_scene/src/visibility_sync.rs:84-105`) | A9 (a render-only bug) | not simulation |
| H-07 | ID (latent) | `Contact.other` is an `EntityId` (`[J]crates/boyko_physics/src/components.rs:202-210`) | Rule: hashed and recorded through `ReplayKey`. The rung that adds its first producer tests it | id perturbation |
| H-08 | X-M | std transcendental calls in engine code (non-test: scene 5, math 1, physics 2 `cbrt`) | RP-0. Scene's camera calls run in Main and stay | census; UCRT FMA3 |
| H-09 | X-B | The `f32::max`/`min` tie on ±0 is unspecified (`[J]…/resources.rs:105`) | Fixed within one binary | profile; OS |
| H-10 | X-B | The AVX2 SDF narrowphase differs on ±0 and degrades on non-AVX2 builds (`[J]…/resources.rs:79-116`; `[J]…/systems.rs:745-752`) | Header switch; the player refuses a degraded build | OS |
| H-11 | X-M, X-R | Steps per frame come from the wall clock (`[J]crates/boyko_app/src/runner.rs:1230-1238`; `[J]…/app/app.rs:769-777`; `[J]…/time/fixed_loop.rs:51-89`); one input snapshot is shared by all substeps of a frame (`[J]crates/boyko_input/src/action/process.rs:51-77`) | Tick-level recording of the registered `ActionState<A>` (RP-2) | pacing; round trip |
| H-12 | benign | `cbrt` in grid sizing (`[J]…/resources.rs:1094`, `:1121`) | RP-0 (`det::cbrt`) | census; the physics golden stays unchanged |
| H-13 | gate | NaN payloads are not deterministic | NaN canonicalised (RP-1) | NaN pair test |
| H-14 | X-R | Save/load omits solver state, and reload order differs (`[J]crates/boyko_serialize/src/save.rs:170`; `…/load.rs:579`) | v1 replays start at startup (Q-11) | — |
| H-15 | amplifier | Default W is `available_parallelism` (`[Jw]…/app/app.rs:201-203`) | W is not recorded; the W arm proves independence | W |
| H-16 | X-R | A move of a keyed entity from outside Fixed (such as a Main system adding a marker) reorders the rows that every Fixed query iterates. Physics keys on archetype-row order until U7 (`[J]…/systems.rs:28-35`, `:247-267`), and order-dependent gameplay systems do so always | Rule B's move clause (U-28), detected by the move digest (RP-2). U4–U7 move physics to group-slot order for their own reasons | Main churn with a marker insert (r11) |
| H-17 | X-R | `ArchetypeId` recycling after `clear()`/`remove_archetype` | Detected through `structural_generation()`, with no kernel refusal (RP-2) | RP-2's `clear()` test |
| H-18 | X-R | Main→Fixed crossings outside declared access (`Commands` spawns from Main) | Main-write check (RP-2) | churn; red control (r6) |
| H-19 | X-R | Fixed→Fixed event latency depends on pacing | D-E8 (`every_tick`) | pacing (S-R2) |
| H-20 | X-M, X-R | `FixedTime`'s frame-level getters (`overstep`, `overstep_fraction`, `steps_this_frame`) depend on pacing. `steps_this_frame` is written after the loop, so Fixed code sees the previous frame's count (`[Jw]…/time/fixed_time.rs:125-171`; `[Jw]…/time/fixed_loop.rs:82-83`) | D-E23 moves them to `Time` (U-25), whose Fixed read is refused | boundary report; pacing (r10) |
| H-21 | X-M | MXCSR control bits (DAZ, FTZ, rounding) changed by foreign code on an engine thread (00 §11, Dawson) | Verify-session read of the control bits on every worker (RP-2) | RP-2's decoder unit test (a write would be UB, so no live control exists) |

*Writer check (00 §9 V-59): H-11's `fixed_loop.rs` range was `:50-87` in the patch; `fixed_advance` is `:51-89`.*

⚠ *2026-09-24, the Phase B document step: **recorded and not located: window 4's W=1 pose divergence. It is not an engine hazard row.** By orchestrator ruling, no register row is added for it and it holds no rung. The diagnosis's verdict is "NOT LOCATED". It excludes X-14 / KC-36 and physics on the event's path, ranks a platform memory or compute fault first as "plausible, not confirmed", and leaves one in-process class open (`w1det/diagnosis.md:1`, §5).*
- ***The event.** In window 4, one process of the shipped default at W=1 (`raw/pass-01/075_r2_T-D-allpairs_W1`, AllPairs, exit 0, clean receipts) hashed `0xf6e397d168e0e8b8` against its row's `0x32d5e235342b4143`. Its per-step CSV matched its twins through step 438 and diverged at step 439 in `top_y`, and the pair count never differed. The protocol re-ran it, and the re-run matched (`docs/measurements/2026-09-22-broadphase-tree/README.md`, "The one invalid process"; `docs/MEASUREMENT-QUEUE.md:1139-1145`).*
- ***Why it is not X-14 / KC-36 (H-01) and not physics-local** (the diagnosis `w1det/diagnosis.md` §1–§5, on `6dd1f916`):*
  - *At W=1 all 4000 pool tasks of a run (8 systems × 500 steps) ran on one OS thread, with no nested task, and the dispatcher ran none of them. Each apply window holds one system, and no physics system issues a command, so the completion-pop order H-01 names cannot vary.*
  - *Seeded jitter at W 1/2/8 (60 processes) and FTZ/DAZ forced on every pool thread (5 processes) left the pose hash unchanged, and the scene never sets the denormal or underflow flag.*
  - *The event did not recur in 480 processes (`w1det/summary_all.txt`: 240 + 80 + 80 + 80, trunk binaries and the very binary that fired, idle and saturated; 481 with a smoke run, `denominator.txt`). One non-reference hash in 1031 processes gives a per-process rate between 2.5e-5 and 5.4e-3 (Poisson, 95 %).*
  - *Its signature is a single-bit one: one 1-ULP flip of one body's `position.y`, injected 10 to 40 steps before step 439 at W=1, reproduces the first differing step, the counts of steps where `top_y` and manifolds differ, and the final spread (`w1det/diagnosis.md` §4).*
- ***What the host shows.** 18 bugchecks since 2026-06-01, among them `0x1A` (one with parameter `0x3F`, an in-page CRC mismatch), `0x4E PFN_LIST_CORRUPT` ×3 and `0x50` ×2 (`w1det/bugchecks.txt`), on one non-ECC 16 GB SO-DIMM. That is the owner's workstation's memory-fault signature: a hardware fault is plausible, not confirmed. It is 00 RK-3's risk. Telling a RAM fault from a driver that corrupts pages needs owner actions that take a reboot or admin rights: a multi-pass memory test (Q-6), and `!analyze -v` over the minidumps (diagnosis §7, R4). Neither has run.*
- ***What stays open: class (g), a stray write from another thread of the process** (diagnosis §3(g), ranked second in its §5). A memory-safety defect outside physics could produce this event, and it is the class this repository has met before (the O11-SP4 colored-solve data race, CLAUDE.md principle 0). No such path was found, but the diagnosis records it as "not excluded by measurement". The only evidence against it is the perturbation's size, since a stray write more often lands as garbage (diagnosis §3(g), §4). A re-run cannot separate (g) from a hardware fault, because both give one non-reproducing, ULP-scale mismatch.*
- ***The instruments that would separate them are not built.** **R2** is a set of per-step FNV-1a digests: the pose, the manifolds, the colour graph, the warm records and the axis table. The pyramid runner reads them after `Schedule::run`, outside the timed pair, and an `--inject step,body,field,bit` probe proves the recorder can go red. **R3** is a between-system canary in a diagnostic build: it digests the physics resources at the end of each system and again at the start of the next (diagnosis §7). No rung, lever or queue item tracks either. Both belong to the physics campaign's runner, `benches/jolt_parity_pyramid.rs`, which also gains `--bp-kernel` (`docs/physics/perf-campaign/levers/00-RULINGS.md`).*
- ***The consequence for the protocol.** A single unreproduced mismatch on this host is re-run on a clean build before it is believed; that is 00 RK-3's re-run, with the build made clean. A matching re-run does not close the event. It is recorded with its binary, cell and hashes, as window 4's was (`docs/MEASUREMENT-QUEUE.md:1139-1145`), and it stays open under (g). It becomes a defect, a hazard row or a pin change when it reproduces, or when R2 places two or more events at the same stage on identical carried state (diagnosis §7, R2). **While R2 is unbuilt, the trigger is the second recorded unreproduced mismatch on any engine binary.** The physics campaign then builds R2 and R3, as the runner's owner, before the next timed window, and that window runs with R2's digests on. No D-E0 test, no physics fix and no default flip waits on this event.*

**Rulings this builds on and adds.**
- **Builds on:**
  - U-20, kept with its rationale corrected;
  - KC-36, kept with its "Deletes" narrowed;
  - KC-26/EK7, extended with `every_tick`;
  - U-13.
- **Adds:** U-21 (writer lanes), U-23 (in-house simulation math), U-24 (keys), U-25 (`FixedTime` split), U-26 (canonical hook order), U-27 (machine arm) and U-28 (move clause). All are in 00 §3.
- **Changes nothing else** in the kernel contract.

## 3. Mapping table: every source id to its unified id

**Physics** (`PHYSICS-ECS-UNIFICATION-DESIGN.md:464-473`, rev 3–5 patches):

| Source | Unified |
|---|---|
| K1 | KC-10 |
| K2 | KC-11 |
| K3 | KC-12 |
| K4 | KC-14 |
| K5a, K5b | KC-08 |
| K6 (rev 5 chain) | KC-12 (Chained) |
| K7 | KC-15 |
| D13 binder / anchor | KC-12 |
| D14 | KC-12, plus KC-13 for Stamped |
| D15 `fresh_step` | physics-owned (U7), no kernel item |
| P-§14 deterministic in-window apply order | KC-36 |

**Engine** (`ENGINE-RUNTIME-ECS-DESIGN.md:891-917`, P-§9 `:2529-2577`):

| Source | Unified |
|---|---|
| EK1 | KC-10 |
| EK2 | KC-22 |
| EK3 | KC-23 |
| EK4 | KC-25 |
| EK5 | KC-24 |
| EK6, EK6g | KC-23 |
| EK7, EK8 | KC-26 |
| EK9, EK10, EK11 | KC-27 |
| EK12 | KC-16 |
| EK13 | deleted (X-17) |
| EK14 | KC-15 |
| EK15a, EK15b, EK15c | KC-29 |
| EK16 | KC-34 |
| EK17 | deleted (Q1a) |
| EK18 | KC-28 |
| EK19 | KC-30a |
| EK20 | KC-31 |
| EK21 | KC-32 |
| EK22 | KC-20 |
| K6′ | KC-13 (re-filed, U-3) |
| RF1 | render, not kernel |

**Ledger** (`RUNTIME-DATA-LEDGER.md:775-825`):

| Source | Unified | Source | Unified |
|---|---|---|---|
| KF-01 | KC-10 | KF-26 | KC-31 |
| KF-02 | KC-16 | KF-27 | KC-33 |
| KF-03 | KC-15 (mode a); KC-10 CSR pair (mode b) | KF-28 | KC-33 |
| KF-04 | KC-30a | KF-29 | KC-27 |
| KF-05 | KC-10 (`WorldScratch`, D-R2a) | KF-30 | KC-27 |
| KF-06 | KC-03 (kernel); `fmt::Write` on `ScratchBuildView<u8>` in KC-10 | KF-31 | KC-07 |
| KF-07 | KC-17 | KF-32 | KC-01 |
| KF-08 | KC-30a | KF-33 | KC-05 (U-4) |
| KF-09 | KC-30c | KF-34 | KC-06 (U-5) |
| KF-10 | KC-30b | KF-35 | KC-03 |
| KF-11 | KC-29 | KF-36 | KC-34 |
| KF-12 | not built (U-15) | KF-37 | superseded by KC-13 |
| KF-13 | KC-28 | KF-38 | withdrawn |
| KF-14 | KC-28 | KF-39 | KC-33 |
| KF-15 | KC-28 | KF-40 | KC-34 |
| KF-16 | withdrawn | KF-41 | KC-18 |
| KF-17 | KC-25 | KF-42 | KC-03 |
| KF-18 | withdrawn | KF-43 | KC-31 |
| KF-19 | KC-12 | KF-44 | KC-22 |
| KF-20 | KC-14 | KF-45 | KC-04 |
| KF-21 | KC-08 | KF-46 | engine HO3; no kernel delta |
| KF-22 | KC-12 (subsumed) | KF-47 | KC-21 |
| KF-23 | KC-26 | KF-48 | KC-29 |
| KF-24 | rejected | KF-49 | KC-13 |
| KF-25 | KC-23 | | |

**Allocator rev 2.4:**

| Source | Unified |
|---|---|
| §2.0 crate split | KC-01 |
| §2.2 size pin relaxed; P5 `ZeroInit`; `ByteColumn`; `ensure_len_zeroed` | KC-03 |
| `DropColumn` | KC-16 (U-8) |
| `Heap`, `HeapRef`, `HeapVec`, `HeapBox`, `HeapDyn`, `SortedMap` | deferred (U-1) → KC-15 / KC-17 / KC-18 / KC-10 |
| `TableSet` | deferred (U-7) → KC-18 |
| P2, P16, P17, P28 | KC-05 |
| P3 poison write | UG-08 row for KC-05 |
| 1d (P10, P20); 1f | KC-06 |
| 1c | KC-17 + KC-03 |
| 1e `TermList` | KC-17 / KC-10 |
| P15 `commit_at`; P31 owner counter | KC-01 / UG-04 |
| rung 2 stacks and encodings | KC-18 |
| rung 3 pool ownership | KC-07 |
| `ErasedSystem` | KC-17 |
| P39; P40 (tag path); P32 32.2 and R2–R5; NW4 | KC-19a |
| P32 R1 (rename only); P40 (sized body); class-A seam entries | 05 MS-02b (option A, modding Stage 3; rev 5.1) |
| five value readers | KC-19b: four readers generic over `ModSeam`; components through KC-19a's `id_space_census()` |
| P38 | KC-21 |
| G1 | UG-02 |
| G2 | UG-03 |
| G2b | UG-04 |
| G3 | UG-05 |
| G4 | UG-06 |
| G5 | UG-07 |
| G6 (a)–(f) | UG-15 |
| §7 K-MOD-1..10 | 05 §4 |
| Frame class | deleted (P34) |
| `HeapString` | deleted (P35) |
| `ScopeArena` | deleted (P2) |
| `MOD_ID_BASE` / `register_at` | deleted (P26) |
| `DYN_ID_CEILING` | deleted (P39) |

**Modding §7 items 1–10** (`MODDING-DESIGN-SPACE.md:1458-1833`) map to 05 §3.2 MS-01..MS-15; 05 §3.1
says which options need each.

**Owner Q-9 and the replay hazard register** map to KC-37 (§2.1). Each hazard's rung is in §2.1's register.

## 4. Conflict resolutions, with numbers

- **R-A, Heap class (U-1).**
  - *Conflict.* The ledger's plan conflict 1 (`RUNTIME-DATA-LEDGER.md:1726`) against allocator
    rev 2.4's Heap. After HV-4 and P31/3 the Heap's clients are exactly the owner classes
    `schedule`, `schedule-builder`, `registry` and `master-table` (`ALLOCATOR-DESIGN-SPACE.md:2707`).
  - *What the ledger already assigns to every such row:*
    - schedule rows → KF-14, KF-07, relation and kernel-internal forms (TSV 784-794; the rows'
      `destination` column reads `NEW:VmBitSet`, `VmReservation-raw`, relation `NEW:VmJagged<T>`
      and `NEW:DynArena`);
    - builder rows → KF-01 scratch;
    - registry and master-table rows → kernel-internal VM forms (TSV 557-559 are `ArchetypeBundle`
      rows → `NEW:BoundedArray`, `NEW:VmColumn::grow_filled`, `VmReservation-raw`; TSV 2338-2340
      are `SparseMap` rows → `NEW:VmSparseMap<U>`);
    - resource boxes → kernel-internal (TSV 749, 751).
  - *Cost comparison.*
    - Per-frame: equal, because all clients are build-time.
    - Heap: eager Miri reserve 1.26 MiB per `EcsMaster` (every Miri test builds masters), 2 GiB
      VA per master, worst resident 204 KiB.
    - Heap safety surface: `HeapHeader`, `HeapRef`, class free lists, and a 448-entry run table.
    - ECS forms: 4 KiB floor per column after D-M1, ComponentPool code already hot in L1i, no new
      unsafe.
  - *What survives.* K7's pow2-class free lists are the `HeapVec` algorithm inside ECS storage
    (ED15 `ENGINE-RUNTIME-ECS-DESIGN.md:427-433`). The Heap design as closed for C1 (P1/P15) stays
    the revival form. Overturn gates: 00 §3 U-1.
- **R-B, scratch identity (U-2).**
  - *Conflict.* Three routes: KF-01 "band keyed by Layout", K1 "registry-free pools with a
    contiguous stagger run", and allocator P43 "one registered id per element type".
  - *Fact from P43.1.* The id is only a layout token plus a stagger seed
    (`ALLOCATOR-DESIGN-SPACE.md:3777-3779`), so a constructor taking `(size, align, stagger)`
    needs no id.
  - *Result.* The cheapest route that meets all three is chosen.
- **R-C, K6′ against physics rev 5 (U-3).**
  - *Conflict.* Engine rev 3 re-filed K6′ on `DenseGroupMut::release_dying_before` and the
    `chain_head_registered` refusal (`ENGINE-RUNTIME-ECS-DESIGN.md:2080-2092`). Physics rev 4/5
    deleted both, released out-of-chain removals at once, and flushed `dying` on the first
    out-of-chain op (`PHYSICS-ECS-UNIFICATION-DESIGN.md:2452-2463, 3336-3343`). Under rev 5, an
    asset slot would be freed while a submitted frame still names it.
  - *Resolution.*
    - A per-group associated type `Release` decides the policy. Its markers are `Immediate`,
      `Chained` and `Stamped`, each carrying `const KIND`. The erased paths read the policy from
      one byte of the store (KC-12).
    - `Stamped` skips immediate release and the flush, and `clear()` stamps rather than releases.
    - **Who** may release is decided by the ChainKey (rev 5). Only the declaring crate can build it.
      For the asset groups that crate is `boyko_render`, whose group markers are private
      (`ENGINE-RUNTIME-ECS-DESIGN.md:1883`), so `render_retire::<K>` can construct the key.
      No runtime claim is needed.
    - **When** is a separate question, and the key cannot answer it.
      - The horizon form is safe from any system holding `GroupHead<G>`: its horizon is
        `completed_gather`, which ED16's proof covers (`:2106-2110`).
      - The teardown form's horizon is "all", and ED16 does not cover it. Called from an exclusive
        system, it would free slots that submitted frames still name (a device-lost or GPU
        use-after-free).
      - So the teardown form also takes `&TeardownToken`, as rev 3 required (`:2090-2092`, `:2617`).
        Only KC-27's driver mints the token, when no schedule runs, and the rank callback idles
        the device first (`:2454`).
    - The horizon's provenance is render's obligation, gated by AS2's FIF-slot-reuse proptest. The
      kernel checks only that a horizon is not in the future.
  - *N5 against rev 5.* Rev 5 has no `release_dying` visitor. Span-typed group columns are freed
    by the kernel at every release point before the DEAD fill; the user visitor exists only for
    non-kernel per-slot resources (device lanes).
- **R-D, scope arena (U-4, U-6).**
  - *Conflict.* Ledger KF-33 wants a per-thread arena with mark/rewind
    (`RUNTIME-DATA-LEDGER.md:1449`); the ledger decision wants a frame-local `ScopeShared`
    (`:1866`); allocator P2/P16/P17 keeps the block inline and claims slot identity.
  - *Resolution.* The allocator form. It is the only one reviewed against C2 and C3, and the
    allocator's later passes confirmed the protocol closed (`ALLOCATOR-DESIGN-SPACE.md:3485-3493`,
    the table of earlier-pass findings that stand closed).
- **R-E, lanes (U-5).**
  - *Conflict.* KF-34 wants in-house rings everywhere (`RUNTIME-DATA-LEDGER.md:1462`).
  - *Resolution.* P10 measured the worker path at zero allocations
    (`ALLOCATOR-DESIGN-SPACE.md:1159`). The epoch `Local` is answered by 1f. Full rings become the
    revival form.
- **R-F, pool tables (U-7).** KF-31's inline arrays and the deferred TableSet produce the same
  memory.
- **R-G, drop-aware column (U-8), erased objects (U-9).** See 00 §3.
- **R-H, `Children`.** The ledger (KF-11 → spans), the engine (EK15c) and the allocator (P19.2
  "a relation") agree. KC-29 `Segmented` is the form.
- **R-I, modding** (U-10, U-11). See 05 §4.

## 5. Not in the contract

Each is either deleted or has a revival gate.
- **Deferred:** Heap family and TableSet (U-1, U-7); full in-house lanes (U-5); KC-11 until a
  verified consumer exists; KC-35 until MQ-05 says build.
- **Deleted:** Frame class (revival rule FR-1, `ALLOCATOR-DESIGN-SPACE.md:3152-3154`);
  `HeapString` (`:3210`); `ScopeArena`; `DYN_ID_CEILING`; `MOD_ID_BASE`; K-MOD-10 and K-MOD-11;
  EK13; EK17; `release_dying_with` (engine N5; no consumer once the kernel frees spans, U-3).
- **Withdrawn / rejected:** KF-12 (not built, U-15), KF-16, KF-18, KF-24, KF-37, KF-38.
- **Modding Stage 3, option-specific (rev 5.1):** 05 §3.2's MS-01, MS-02b and MS-09..MS-13 (option
  A), MS-14 (A′) and MS-15 (C); options B, D, E and F are out (Q-1). Each is built behind the modding crates, only for the option
  Stage 0 and MQ-09 choose, and every kernel half obeys I-4's rule S-1. Rev 5's `pub` on
  `try_register_dynamic`, `set_residency_class` and `install_map_entities_fn` is withdrawn (05
  MS-01).

## 6. Threading model: new shared state only

| State | Writers | Readers | Ordering | Argument |
|---|---|---|---|---|
| `COMMITTED_BYTES[i]` | any thread that commits | the gate | `Relaxed` | counter only; no control flow reads it |
| `ChunkArena.frontier` | any carving thread | carving threads | `fetch_add`, then a commit of its own disjoint range | one atomic, no second counter (P17, `ALLOCATOR-DESIGN-SPACE.md:1695-1697`) |
| `dispatcher_claim` | installing threads | installing threads | Acquire/Release RMW | release-sequence argument (P17.2) |
| `SlotChunks.heads` | the slot owner only (SC-2) | the slot owner | none (`UnsafeCell`) | exclusivity is external; `unsafe impl Sync` bullet plus a debug owner witness |
| Injector ring | pushers and stealers | workers | loom-modelled | P20 push-side wake proof |
| `LaneBoard` | lanes | worker top level | physics §10.1 rev 5 | PB1 proof (`PHYSICS-ECS-UNIFICATION-DESIGN.md:3735-3743`) |
| `NEXT_ID`, intern | registrars | lookups | CAS `Relaxed` + mutex | P40: the name insert is the publication point (`ALLOCATOR-DESIGN-SPACE.md:3701`) |
| `died` column (Stamped) | apply windows (`&mut EcsMaster`) | R1 system with `GroupHead` | exclusive, via the scheduler | declared write on the recycle node |
| `DenseGroupStore.release` | `ensure_group` (`&mut EcsMaster`), once | erased group paths | write-once before the store is reachable | no reader can precede the write |
| `NEXT_STAGGER` (KC-10) | column constructors (setup) | the same | `fetch_add`, `Relaxed` | a round-robin seed, not a publication. `fetch_add` hands out distinct values. The only nondeterminism is the order of construction when columns are built concurrently, which costs performance (MQ-16) and never correctness |
| `THREAD_BUSY[w]` (KC-04) | lazy claimants, pool builds (batch claims), releasing threads | the same | claim `compare_exchange(w, w \| take, AcqRel, Acquire)`, where `take` is one bit (lazy) or the lowest clear bits (batch); release `fetch_and(!bit, Release)`; scan `load(Acquire)`; batch sizing `load(Relaxed)` | a bit is set only by a CAS whose precondition is "these bits clear", so no bit goes to two claimants; the `Relaxed` free-count estimate only sizes a batch and never decides exclusivity |
| `THREAD_RECORDS[i]` (KC-04) | the slot owner only | the slot owner only | none (`Cell`s) | slot-private (the SC-2 shape): reachable only through the owner's word, which only the owner writes. The releasing owner's last write, the zeroing, happens-before the next claimant's first read, through `Release` on the bit and the claimant's `Acquire` CAS. A pool build's batch claim writes no record, and the worker's first read comes after its spawn |
| the per-thread word (KC-04), and `LANE`'s word | its own thread only | its own thread only | none | OS thread-local by construction |
| `THREAD_CTX_CLAIMS`, `THREAD_CTX_REFUSED`, `THREAD_CTX_PEAK`, `POOL_WORKERS_CLAMPED`, `POOL_RESERVE_DIPS` | cold paths only | UG-20, D-M6's tests | `Relaxed` (`fetch_max` for the peak) | counters; no control flow reads them |
| `TeardownToken` | — | — | not shared: `!Send + !Sync` | minted on the teardown driver's stack |
| KC-36 window bitset | the dispatcher | the dispatcher | none | inside `executor_scratch`, dispatcher-only |
| Event writer lane (D-E20) | the one system instance that owns the lane | `update_events` under `&mut EventDispatcher` | none | One writer at a time: `EventWriter` is `&'s mut` (`[Jw]…/event_writer.rs:89-91`), and the scheduler never runs one system instance on two threads. The swap barrier is unchanged (`:126-128`). |
| `EcsMaster::events().send_event` (D-E20) | the dispatcher, on the apply path and in exclusive systems | `update_events` | none | A call on a worker returns an error and writes nothing (U-21), so the dispatcher lane keeps one writer. |
| `SimInputs<E>` staging and tick columns (RP-2) | Main (staging, declared write); the tick-start system (moves staging → tick) | Fixed (tick slice, declared read) | none | Main and Fixed schedules never overlap: the frame driver runs them in sequence (`[Jw]…/app/app.rs:725-744`). |
| `ReplayKeyIndex` (RP-2), an open-addressed key → `Entity` table | `ReplayKey`'s hooks, under `&mut EcsMaster` | playback | none | Reached only through apply windows and the replay systems' declared access. The entity → key direction is the entity's own `ReplayKey` component. |
| `DerivedKeyCounters` (RP-2) | `spawn_derived`, at key issue inside an apply window or a hook | the same | none | Written only under `&mut EcsMaster` (apply) or a hook's deferred world, which runs inside an apply window. |
| Tier-1 hash accumulator (RP-2) | one per-worker slot per chunk (`[CachePadded<UnsafeCell<u64>>; MAX_WORKERS + 1]`) | the verifier, after the join | none | Each slot is written only by the worker whose id indexes it. The `unsafe impl Sync` argument is that exclusivity. Slots are read only after `par_for_each_chunk` returns. The wrapping sum is commutative and associative, so the result does not depend on W or order. |

**Data-race freedom.**
- Every new structure is either atomic with a stated ordering, slot-private under SC-1/SC-2, or
  reached only through `&mut EcsMaster` or a declared scheduler write.
- Loom models are listed in 03 §3.

**KC-04 thread-context protocol** (lands in D-M6; the loom model, the lifecycle binary and the
plan-build test are D-M6's red-first tests).

Each thread stores, in one OS word that only it writes, the index + 1 of the kernel record it owns
(0 = none). Only that thread's own claim, adopt and release write the word, so a thread's record
cannot change while the thread lives.

1. **Records (static, `.bss`).**
   - **The statics.** `boyko_threadpool::thread_ctx` declares two statics:
     - `THREAD_BUSY: [AtomicU64; THREAD_WORDS]`;
     - `THREAD_RECORDS: RecordTable`, a wrapper around `[ThreadRecord; THREAD_SLOTS]` that carries
       `unsafe impl Sync`; its `// SAFETY:` is the slot-private argument of the table above.

     `THREAD_WORDS = 128` and `THREAD_SLOTS = THREAD_WORDS × 64 = 8192`, const-asserted as
     `THREAD_SLOTS == 128 × MAX_WORKERS`.
   - **Placement.** Both statics have all-zero const initialisers
     (`[const { ThreadRecord::ZERO }; THREAD_SLOTS]`), so they are placed in `.bss`: 524,288 B of
     records plus 1,024 B of busy words, virtual size only.
     - No initialiser runs and nothing is materialised.
     - A record address is a link-time constant plus an offset, valid for the whole process.
     - D-M6 checks the placement structurally, with leg (7)'s tool.
   - **Resident cost.** It follows the peak number of claimed slots, not the capacity. A claim takes
     the lowest clear bit, so records fill from the front, and 64 records share a 4 KiB page.
   - **Ledger form.** The statics are not a `VmColumn`. KF-45's ledger form
     (`RUNTIME-DATA-LEDGER.md:1628`) is restated per U-19 (d) and 00 §5. KF-43 already accepts
     static tables as the allocation-free form (`:1593-1605`), and a zero static involves no
     allocator.
   - **Encoding.** The layout block below gives the per-field encoding under which all-zero means
     detached.
2. **Capacity (critic pass 4, W1).**
   - **Rule.** Workers claim at pool build, in one batch, and are clamped so they can never exhaust
     the table. The remainder is a foreign reserve, `FOREIGN_RESERVE = 2048` (const-asserted
     `== 32 × MAX_WORKERS`).
   - **A pool build sizes its batch as follows.**
     - `k = min(requested, free − FOREIGN_RESERVE)`, when that is ≥ 1.
     - Otherwise, one slot taken from the reserve if any slot is free; `POOL_RESERVE_DIPS` += 1.
     - With no free slot, `build` raises a coded cold panic on the building thread (code added to
       `boyko_log/src/codes.rs` at D-M6's cut).
     - The pool is built with `worker_count = k`. When `k < requested`, `POOL_WORKERS_CLAMPED` += 1;
       a smaller pool is a correct pool.
   - **A worker never claims and is never refused.**
   - **Every other thread** (host, test harness, user threads) claims lazily on first touch, and is
     refused only when all 8192 bits are set.
   - **`free` is an estimate.** It is the sum of `popcount(!load(Relaxed))` over the words, read
     once per build. It only sizes the batch; exclusivity comes from the CAS.
   - **Sizing basis.**
     - The pool share (6144 slots) holds 96 full 64-worker pools.
     - The reserve covers reserve dips plus foreign threads up to 2048. Example: a test binary on a
       1024-logical host, running 1024 concurrent tests that each build a default `App`, needs 96
       full pools + 928 dips + 1024 foreign threads = 1952.
     - The gate host is 16 logical (`[J]docs/threadpool/KE16-RESULTS.md:75`). There, the 10
       `boyko_render` lib tests that build a default `App` need 10 × 16 + 10 = 170 slots, i.e. 3
       record pages. *Writer check (00 §9 V-54): the 10 tests reach `App::new()` through 8 call
       sites, because `run_sv0_gate` (`[J]crates/boyko_render/src/light.rs:2214-2227`) is shared by
       three tests; the architect's figure was 8 tests and 136 slots.*
     - Critic pass 4's 32-logical case (264 slots as the critic counted it; 330 with all 10 tests)
       and its 64-core four-`App` case (260 slots) fit unclamped.
3. **The word** holds `index + 1`; 0 means no record.
   - **Why an index rather than the address.**
     - The record's address is always derived from `&THREAD_RECORDS`, so strict provenance holds and
       no integer-to-pointer cast exists.
     - The price is one `shl` and one RIP-relative `lea`, both register operations.
   - **One arm, every host (rev 6, U-19).** The word is
     `thread_local! { static WORD: Cell<usize> = const { Cell::new(0) } }`.
     - It has no `Drop`, so where `target_thread_local` is set (msvc, Linux) it compiles to a direct
       native TLS access; on windows-gnu under rustc ≥ 1.98 it is std's OS-key read
       (`[J]docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:87-95`).
     - Miri runs this arm; nothing here uses inline assembly.
     - **`EXIT_GUARD`'s destructor reads `WORD`.** On native-TLS hosts a const-initialised,
       `Drop`-free `WORD` has no destroyed state, so the read is always valid. On an OS-key host
       (windows-gnu) the read relies on std running key destructors in reverse registration order,
       with `WORD` registered first (critic pass 5, open question 2; stable-gnu std
       `sys/thread_local/key/windows.rs:150-191`, as the critic read it). A red lifecycle or Miri
       test on gnu is diagnosed against that dependence first.
     - Writes happen only at claim, adopt and release.
   - **loom.** The word is `loom::thread_local!` of the same cell.
   - **`boyko_diag::lane` is not changed by D-M6.** `LANE` stays diag's own const-initialised,
     `Drop`-free `thread_local!` (`[J]crates/boyko_diag/src/lane.rs:136-140`, `:34-40`); its reads
     never allocate (`:142-149`).
   - **The windows word arm is not built** (a `TlsAlloc` slot read at `gs:[0x1480 + 8·index]` after
     a canary, `CTX_IDX`/`CTX_TLS_OFF`, the arm constants and diag's copy). §8 keeps its text as the
     revival form D-M6w, with the conditions that build it (00 U-19 (a), (b)).
4. **`prepare()`: withdrawn in rev 6.** It existed only for the word arm (§8.4).
   `ThreadPoolBuilder::build`'s first KC-04 statement is `claim_for_pool` (item 6).
5. **Lookup (hot).**
   - **Shape.**
     ```rust
     #[inline]
     pub fn current() -> Option<&'static ThreadRecord> {
         let w = WORD.get();                  // item 3
         if w != 0 { Some(record(w - 1)) }    // `record`: &THREAD_RECORDS.0[i], unchecked; debug_assert!(i < THREAD_SLOTS)
         else { current_slow() }              // item 6
     }
     ```
   - **Cost:** 1 native TLS load (on windows-gnu, std's OS-key read), 1 null test, then `shl` +
     `lea`. The field access adds 1 load.
   - **The reference cannot leave the thread.** `current()` returns `&'static ThreadRecord`, and
     `ThreadRecord` is `!Sync` (its fields are `Cell`s), so the reference is `!Send`.
   - **`'static` holds until this thread's release,** which runs only from `AdoptedRecord`'s drop at
     the end of `worker_main`, from `EXIT_GUARD`'s destructor, or from
     `unsafe fn release_current()`, whose contract forbids a live reference.
   - **Engine callers never store the reference.** The only upper-layer accessor is `boyko_ecs`'s
     `pub(crate) fn ecs_fields() -> Option<&'static EcsThreadFields>`, in the new
     `crates/boyko_ecs/src/ecs/core/thread_fields.rs`. It is the single caller of `ext_ptr()`,
     pinned by `tests/thread_ctx_census.rs`.
   - **Reads.** Every read claims on first touch (item 6). A refused thread's reads return `None`,
     which every reader decodes as `DETACHED`:
     - `active_pool` null;
     - `wid == WORKER_ID_UNATTACHED`;
     - `cache_slot == NO_CACHE_SLOT`;
     - `in_system_run == 0`;
     - every ECS field zero, with `plan_build` null.
   - **`install`** reads `current()` once, before `active_scopes.fetch_add` (`thread_pool.rs:243`).
     `InstallGuard` keeps the `&ThreadRecord` for its restore, so the frame costs one lookup instead
     of today's five pool-part TLS accesses (`:245`, `:253`, `:254`, `:372`, `:376`). The three
     `LANE` accesses of the frame (`:253`, `:255`, `:373`) stay on diag's `LANE` thread-local (00 §9 V-53).
6. **Claim (cold).**
   - **Lazy claim:** `current_slow()`, `#[cold] #[inline(never)]`, in this order:
     1. *(Withdrawn in rev 6 together with the word arm; the numbering is kept, because step 2 is cited by the refusal rule.)*
     2. `EXIT_GUARD.try_with(|_| ())`. `EXIT_GUARD` is a `thread_local!` ZST whose `Drop` runs
        item 7. `Err` means this thread's TLS is being destroyed (00 §11), so the claim is refused.
     3. Scan `THREAD_BUSY` from word 0:
        - `w = load(Acquire)`; skip a full word;
        - `b = (!w).trailing_zeros()`;
        - `compare_exchange(w, w | 1 << b, AcqRel, Acquire)`;
        - on failure, reload the same word.
     4. `debug_assert!` that record `i` decodes as idle, field by field through its `Cell`s, pool
        fields first, so the failure text is `thread record not idle at claim`. **This is a check,
        not a write.** Item 7 is the only zeroing site, and a first claim reads `.bss` zeros.
     5. Write the word (`i + 1`).
     6. `THREAD_CTX_CLAIMS += 1`; `THREAD_CTX_PEAK.fetch_max(busy estimate)`.

     **Refusal:** if all bits are set, or step 2 returned `Err`, then `THREAD_CTX_REFUSED += 1` and
     the call returns `None`. A refused thread retries on its next read, because a slot may have
     freed since; each retry pays the scan, which is a capacity failure that UG-20 reports.
   - **Batch claim:** `claim_for_pool(requested) -> SlotBatch`, called by `build` as its first KC-04 statement.
     - Item 2 sizes `k`.
     - Per word, one CAS takes the lowest `min(remaining, popcount(!w))` clear bits (`compare_exchange(w, w | take, AcqRel, Acquire)`, as above; critic pass 5, O9).
     - `SlotBatch` is an inline `[u16; MAX_WORKERS]` plus a `len`; it makes no allocation.
     - Each record is debug-checked idle. The builder writes no record.
     - `THREAD_CTX_CLAIMS += k`.
   - **Adopt.**
     - `worker_main`'s first statement becomes `let _ctx = thread_ctx::adopt(slot);`. It writes the
       word and returns `AdoptedRecord`, whose `Drop` runs item 7.
     - Because it is declared first, the guard drops after every later-declared guard in
       `worker_main`, including `WorkerDequeDeposit` (`worker.rs:69`), so no engine write follows
       the release on that path.
     - A worker never touches `EXIT_GUARD`, so on windows-gnu it makes no `os-thread` allocation for
       the context.
     - **Happens-before:** the builder's `Acquire` CAS precedes the spawn, which precedes the
       worker's first read.
     - **Spawn failure:** a failed spawn panics `build` as it does today (`:747`), and the unspawned
       workers' slots stay claimed: leaked, never inherited.
7. **Release.**
   - **Where it runs:** from `AdoptedRecord::drop`, from `EXIT_GUARD`'s `Drop`, or from
     `unsafe fn release_current()` (fiber hosts).
   - **Steps:**
     1. Read the word; if it is null, return.
     2. Clear the word.
     3. **Zero the record**, field by field through its `Cell`s. `_pad` is a `Cell<u32>` and `ext`
        an `UnsafeCell`, so no byte is written through a pointer that the shared reference does not
        permit (SB/TB).
        - **This is the only zeroing site, and it is necessary.** `worker_main` sets `active_pool`
          and `wid` and never restores them (`[J]crates/boyko_threadpool/src/worker.rs:46, 59`), and
          a thread may exit with frame fields raised.
        - An unzeroed successor would inherit a worker identity and a pool pointer that dangles once
          the pool drops.
     4. `THREAD_BUSY[i / 64].fetch_and(!(1 << i % 64), Release)`.
   - **Order.** Clearing the word before the bit keeps a new claimant's record unreachable from the
     old owner.
   - **No callback.** No engine callback is defined, so no census crate defines an `extern` fn;
     leg (1) stays at 0 with no allowlist entry.
   - **Runner.** `EXIT_GUARD`'s `Drop` runs inside std's TLS-destructor runner: FLS on windows-gnu
     under rustc ≥ 1.98 (`[J]docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:79-82`), the native runner
     elsewhere. It takes no lock, allocates nothing, and makes no OS call.
8. **Not in the contract; restrictions.**
   - **Abnormal termination** (`TerminateThread`, or a thread killed before its TLS destructors run)
     leaves the slot busy for the rest of the process: leaked, never inherited, never zeroed. std
     also documents that, at process exit on Windows, other threads' destructors may not run
     (00 §11); their slots die with the process.
   - **Fibers.** The engine creates none. A host that deletes fibers on engine threads must call
     `release_current()` first.
   - **A claim during TLS teardown** is refused once `EXIT_GUARD` is destroyed (item 6 step 2).
   - **New restriction (critic pass 4, O7).**
     - **Today:** the eight engine rows are const-initialised `Cell`s with no `Drop`
       (`hooks/scope.rs:31`, `observers/propagate.rs:30`, `relationship/mod.rs:90, 152`,
       `hierarchy/commands.rs:127`, `tls.rs:169, 196, 205`). On msvc and Linux, a structural op or
       an `install` inside another thread-local's destructor therefore works.
     - **After D-M6:**
       - Such an op running after `EXIT_GUARD`'s destructor is refused: its reads answer `DETACHED`,
         and its first write raises the coded panic. A panic inside a TLS destructor aborts the
         process.
       - Such an op running before `EXIT_GUARD`'s destructor may claim a fresh record, which
         `EXIT_GUARD` releases.
     - **Engine exposure.** No engine thread-local's destructor calls engine code: after D-M6 the
       engine's only `thread_local!` statics are item 10's four key and guard cells.
     - **User exposure.** A user thread-local whose destructor does call engine code is outside the
       contract, like fibers. `thread_ctx`'s docs state this, and the book gains the note at D-M6
       (through `doc-writer`).
   - **A write with no record** raises `refused_write_panic()`, whose code is added at D-M6. Reads
     never panic. Because `install` checks the record before `active_scopes.fetch_add`, a refused
     `install` leaves no partial state.
9. **Loom model (D-M6, UG-09).**
   - **Setup.**
     - The claim, batch and release functions are generic over a `&ThreadTable` view; the statics
       are one instance.
     - Under `cfg(loom)`, each iteration builds a fresh all-zero local table of 2 slots (the
       statics' state at process start) plus a loom-only witness array.
     - Three threads run. Threads 1 and 2 claim lazily; thread 3 takes its slot through
       `claim_for_pool(1)` + `adopt` on a spawned child.
     - Every thread resolves several times and then releases explicitly.
   - **Properties.**
     - **A1, exclusivity:** a record's witness is CAS'd from 0 to the thread's id on first resolve,
       and back at release; the CAS never fails.
     - **A2, stability:** every `current()` in a thread's life returns the reference from its first
       resolve, checked after each peer claim and release.
     - **A3, exact refusal:** `None` is returned only when both bits were set at the scan.
   - **Thread count and bound.** loom 0.7.2 allows at most 5 threads (`rt/mod.rs:62`). The positive
     model uses exactly 5 (main, threads 1–3, and thread 3's spawned child); every arm runs under
     `LOOM_MAX_PREEMPTIONS=3`, the bound CI already uses (`[Jw].github/workflows/ci.yml:315`).
   - **Red-first arms.** Each is `#[should_panic(expected = "<oracle>")]`, where the oracle text is
     produced only by the model's own assertion. loom's own thread-limit and branch-limit panics
     (`scheduler.rs:99`, `path.rs:118`) cannot satisfy them.
     - **M1** (4 threads): a load-then-store in place of the CAS (lazy and batch).
       `expected = "A1: record claimed twice"`.
     - **M2** (3 threads): a release that publishes the bit before clearing the word.
       `expected = "A1: record claimed twice"`.
     - **M3** (3 threads): the rev-2 protocol as a test-only model. It uses a hashed home-bucket
       lookup, a linear-probe claim and a zero-on-release key. Two threads share a home bucket; the
       home-bucket owner exits while the displaced neighbour lives; the neighbour then resolves again.
       `expected = "A2: record changed within a thread's life"`.
     - **M4** (3 threads): rev 3's lazily materialised table (check, then write).
       `expected = "materialised once: count 2"`. It documents why the table is a static.
   - **Count.** The UG-09 leg first runs `-- --list`, which must list exactly the five names; then it
     must print `running 5 tests` and `0 filtered out`, using the recipe of 03 UG-09, for
     `boyko_threadpool/tests/loom_thread_ctx.rs`.
10. **Route per row.** The 12 KF-45 rows (`RUNTIME-DATA-LEDGER.md:1632`), decided:

    | Row | Route | Why |
    |---|---|---|
    | `component/hooks/scope.rs:31` `HOOK_DRAIN_DEPTH` | record (`EcsThreadFields`) | read by every structural op; a per-world field caused the F2 Tree Borrows UB (`[J]…/hooks/scope.rs:4-17`) |
    | `observers/propagate.rs:30`, `hierarchy/commands.rs:127`, `relationship/mod.rs:90`, `:152` | record (`EcsThreadFields`) | the same F2 reason, stated at each site (`[J]…/observers/propagate.rs:8-13`, in its module header; `hierarchy/commands.rs:122-126`; `relationship/mod.rs:86-89`, `:149-151`) |
    | `component_registry/required.rs:153` `BUILDING` | record (`EcsThreadFields.plan_build`, item 11) | an `id_fn` can re-enter the build (item 11), so the set must be reachable without a parameter; this also retires a `RefCell<Vec>`, its allocation, and two `disallowed_types` allows (`required.rs:15-16`, `:152`) |
    | `boyko_threadpool/src/tls.rs:169`, `:196`, `:205` | record (pool part) | read on every spawn; the ledger's measured case |
    | `boyko_diag/src/lane.rs:139` `LANE` | unchanged: diag's own `thread_local!` (rev 6, U-19; the word arm that would have replaced it is §8) | Read on every emit and written twice per `install`. Diag cannot depend on the record's crate. The cell is const-initialised and `Drop`-free (`lane.rs:34-40`), so on msvc and Linux it is one native TLS access. Ledger rev 5 rows it out of scope with this reason (00 §5). |
    | `boyko_log/src/drain_owner.rs:42`, `sync_out.rs:75` `TOKEN_ANCHOR` | the OS thread id (`GetCurrentThreadId` / `pthread_self`, both import declarations), called on the cold claim path; under `cfg(miri)`, one shared TLS-anchor address | only uniqueness among live threads is needed (`sync_out.rs:69-75`); no TLS read and no per-thread allocation |

    **Split.** 9 record, 1 unchanged `thread_local!` (`LANE`), 2 OS thread id. The 12
    `thread_local!` rows become 4 key or guard cells, none holding engine data:
    - `EXIT_GUARD`, touched once per non-worker thread;
    - threadpool's `WORD` (item 3);
    - diag's `LANE`, unchanged (`[J]crates/boyko_diag/src/lane.rs:136-140`);
    - the log's `TOKEN_ANCHOR`, under `cfg(miri)` only.

    Ledger rev 5 rows the four as out of scope, with this reason (00 §5).

    **Census scope** (`tests/thread_ctx_census.rs`): production `thread_local!` statics of the four
    KF-45 crates. Harness rows are outside it: `#[cfg(test)]` statics such as
    `boyko_threadpool/src/block.rs:677` (inside `mod tests`, `:579-580`), and the test-feature
    `boyko_log/src/probe.rs:77`.
11. **The plan-build set** (`BUILDING`'s record route; critic pass 4, open question 3).
    - **The re-entry path.**
      - `RequiredBuilder::require` is a `pub fn` that takes any `fn() -> ComponentId`
        (`[J]crates/boyko_ecs/src/ecs/core/component/component.rs:323-327`), and `Component` is a
        safe trait (`:37`).
      - A hand-written impl can therefore register an `id_fn` that builds a world and inserts the
        component under construction.
      - That insert's bundle resolution calls `get_required_plan`
        (`[J]…/bundle/bundle_column_cache.rs:419`) while the outer `build_required_plan` for the
        same component sits between its push and its `id_fn` call (`required.rs:301`, `:312`).
      - The derive never emits such an `id_fn` (it passes `B::component_id`), but the path is
        reachable from safe code, and today `BUILDING` turns it into a `Cycle` panic.
    - **Why not `&mut`.**
      - A set passed down by `&mut` cannot see this re-entry: the nested call is a fresh outermost
        build, pushes onto a fresh set, and the re-entry recurses without bound.
      - Publishing a raw pointer beside that `&mut` is UB, because the nested access is foreign to
        the outer frame's protected `&mut` (03 §3, Miri test 2's mutation).
    - **Form.**
      - `struct PlanBuildSet([Cell<u64>; 8])`: 512 bits, with `MAX_COMPONENTS <= 512` const-asserted
        (MD:M-K1's 1024 would make it 16 words). It is a stack local of the outermost build.
      - `EcsThreadFields.plan_build: Cell<*const PlanBuildSet>` publishes its address.
      - `build_required_plan` first reads `plan_build`:
        - if it is null, this is the outermost build: it creates the set and a `PlanBuildPublish`
          guard that writes the address and restores null on drop and on unwind;
        - otherwise, it uses the published set.
      - Per id, a `PlanBit` guard tests the bit (set → today's `required_cycle_panic`), sets it, and
        clears it on drop.
      - The set is reached only through `&PlanBuildSet`, so nested and re-entrant builds are sound
        under SB and TB.
    - **Cost.** Cold (once per component type per process): one `current()` per build.
    - **Invariant.** `plan_build` is null whenever no plan build is on the thread's stack (§7).

**`ThreadRecord` layout** (`boyko_threadpool`; 64 B, align 64; all-zero means detached)
```rust
#[repr(C, align(64))]
pub struct ThreadRecord {
    active_pool: Cell<*const PoolInner>, // +0   was ACTIVE_POOL   (tls.rs:169); null = none
    lane: Cell<LaneDeposit>,             // +8   was LANE_DEPOSIT  (tls.rs:196); 24 B (tls.rs:159)
    in_system_run: Cell<u32>,            // +32  was IN_SYSTEM_RUN (tls.rs:205); 0 = outside a system
    _pad: Cell<u32>,                     // +36  a Cell so the release can zero it through `&self`
    ext: UnsafeCell<[u8; 24]>,           // +40  opaque here; typed by exactly one upper crate
}

// LaneDeposit: layout pins unchanged (tls.rs:159-161)
#[repr(C)]
pub(crate) struct LaneDeposit {
    pool: *const PoolInner,       // +0   null = none
    deque: *const Worker<Task>,   // +8   null = none (null iff pool is null, D1)
    wid_enc: u32,                 // +16  wid.wrapping_add(1): 0 = WORKER_ID_UNATTACHED (u32::MAX),
                                  //      u32::MAX = WORKER_ID_DISPATCHER, 1..=64 = workers 0..63 (D-M6)
    cache_slot_enc: u32,          // +20  was `_pad` (tls.rs:141); cache_slot.wrapping_add(1):
                                  //      0 = NO_CACHE_SLOT (u32::MAX) (D-M2, KC-05)
}
// DETACHED (tls.rs:147-152) is all-zero; const-asserted:
//   DETACHED.wid_enc == 0 && DETACHED.cache_slot_enc == 0; the pointer fields are ptr::null().
// wid() and cache_slot() decode with wrapping_sub(1). The predicate (tls.rs:331) becomes
// `wid_enc.wrapping_sub(1) < worker_count`: one more register op, inside the named dispatch body.

// boyko_ecs, crates/boyko_ecs/src/ecs/core/thread_fields.rs (new), through ThreadRecord::ext_ptr();
// ecs_fields() is the single caller (census-pinned)
#[repr(C)]
struct EcsThreadFields {                     // 24 B, align 8, const-asserted; all-zero = idle
    hook_drain_depth: Cell<u32>,             // +0   was HOOK_DRAIN_DEPTH (hooks/scope.rs:31)
    propagate: Cell<bool>,                   // +4   was PROPAGATE (observers/propagate.rs:30)
    cascade_suppress: Cell<bool>,            // +5   was CASCADE_SUPPRESS (hierarchy/commands.rs:127)
    link_suppress: Cell<bool>,               // +6   was RELATIONSHIP_LINK_SUPPRESS (relationship/mod.rs:90)
    eviction_suppress: Cell<bool>,           // +7   was RELATIONSHIP_EVICTION_SUPPRESS (relationship/mod.rs:152)
    plan_build: Cell<*const PlanBuildSet>,   // +8   was BUILDING (required.rs:153); null = no build
    _pad: [Cell<u8>; 8],                     // +16
}
```

**Why one record rather than one column per crate:**
- Each hot reader touches one field.
- One fixed row per thread keeps all of a thread's fields on one cache line that only that thread
  writes, so there is no false sharing.
- Per-crate columns are not link-time constants, so they would add one base load per crate.

## 7. Invariants and edge cases (`debug_assert!` unless noted)

| Feature | Checks |
|---|---|
| KC-02 | `len <= committed`; D2 page-floor asserts ×10 (packing S1) |
| KC-03 | `ensure_len_zeroed` requires the absent value to be 0 (`slot+1`); `set_len <= committed` |
| KC-05 | `W + D ≤ 64` (release assert at build); `exp < EXP_MAX` (release assert, P28); claim released on unwind (`InstallGuard`); `NO_CACHE_SLOT` degrades correctly (counter > 0 in its test) |
| KC-06 | overflow counter == 0 in every census scene |
| KC-12 / KC-13 | live ⟺ `live.test(s)`; dying ⟹ bytes intact ∧ `s2e = TOMBSTONE`; free ⟹ bytes == DEAD; for Stamped, dying slots are never released before `horizon`, and never by removal, the first-op flush or `clear()`; the store's `release` byte equals `<G::Release as ReleasePolicy>::KIND` at every typed entry; `horizon` is not newer than `this_run` (wrap-aware); **an entity whose despawn returned `Deferred` keeps its row, its `e2s` entry and its live slot** (D-E2's test, every despawn path); `open_chain` with `chain_open` already set → diagnostic, not assert; anchor count ≤ 8 (4 used: `PhysicsBody` + Mesh/Material/Texture, `ENGINE-RUNTIME-ECS-DESIGN.md:2742`) |
| KC-15 | total copies ≤ 2n for n appends; `free(DEAD)` is a no-op; span freed exactly once |
| KC-19a | the mint never returns an occupied id; `register_layout` different-type panic prints `id_space_census()` |
| KC-17 | record vtable `LAYOUT` equals the handle's layout; drop count equals insert count at world drop |
| Overflow and wrap | `u32` step wrap (D15: one skipped warm start per 2³² steps); `Tick` wrap-aware compares (ED17/X-30); `MAX_COMPONENTS = 512` unchanged |
| Drop order | pools before reservations (PoolBacking-last precedent); EK11 ranks for NonSend residents; for each `Stamped` group: device idle → `release_dense_group_at_teardown` with the driver's token → its device lane drops → the group store drops. From D-E9 on, a debug assert at store drop checks that a `Stamped` store with dying entries was released by the driver, unless `std::thread::panicking()` is true (a failing test's unwind must not abort the test binary) |
| `ArchetypeFlags` (`u16`) | Bits 0–12 are used (`[J]…/hooks/archetype_flags.rs:29-98`). Bit 13 = `COUNTED_TARGET` (D-E2), bit 14 = `HIDDEN` (D-E11, U-16), bit 15 free; a const assert pins the three pairwise disjoint from every existing bit. Both new bits are computed once at archetype construction from the component set, **above** the `is_signature_storage` screen, as `insert_from_flag_declarations` is (`:188-213`). The observer recompute (`[J]…/archetype/archetype_master.rs:875-904`, through `ArchetypeFlags::clear`, `archetype_flags.rs:134-143`) never clears them; it clears only `ON_*_OBSERVER` bits. The `flags.is_empty()` gates (e.g. `[J]…/entity_api.rs:1050`) keep their test, so the table-only, hook-free path executes no added instruction. The bytes of the enclosing body still change where a rung edits it. D-E2 puts its redirect inside the branch at `:1050` and changes `delete_entity_core`'s return type, so D-E2 runs UG-15 attributed, with the `swap_remove/10k` body named (02 §2). D-E11 adds no code to that body; its bit is read by the cold helpers. An archetype carrying only a new bit takes the cold hook path, whose helpers test their own masks. Cost: one cold call per structural op on hidden or counted archetypes; 0 elsewhere |
| `delete_entity_core` step order | Fixed in 02 §4.4: validity checks → flags read → **redirect** (inside the existing `!flags.is_empty()` branch, before any hook, tombstone, observer retire, row move or `unbind`; the `Pinned` removal is enqueued, not performed) → hooks → dense tombstones → observer retire → `remove_entity` → `unbind` |
| `try_despawn` / `delete_entity` | `try_despawn` returns `Despawned`, `Deferred` or `NotAlive`; `delete_entity` and `despawn_without_children` return `outcome != NotAlive` |
| KC-10 world scratch | a frame is held by value; `len ≤ 2`; `give` clears before it stores; a panic between `take` and `give` leaves the stack consistent (Miri row, D-R2a) |
| KC-04 | **Word and bit:** a non-zero word `w` satisfies `w − 1 < THREAD_SLOTS`, and its bit is set. **Claim:** the record decodes as idle before the word is written (a debug check; the release is the only zeroing site): `active_pool` null; `wid() == WORKER_ID_UNATTACHED`; `cache_slot() == NO_CACHE_SLOT`; `in_system_run == 0`; `EcsThreadFields` all-zero. **Release:** the word is null before the record is zeroed, and the record is zeroed before the bit is cleared. **Stability:** within a thread's life `current()` changes only none → r at claim or adopt, and r → none at release. **Pool builds:** a pool's `worker_count()` equals its batch length, and no worker takes the lazy claim path. **Plan builds:** `EcsThreadFields.plan_build` is null whenever no plan build is on the thread's stack. **Const asserts:** `THREAD_SLOTS == 128 × MAX_WORKERS`, `FOREIGN_RESERVE == 32 × MAX_WORKERS`, `size_of::<ThreadRecord>() == 64`, `size_of::<EcsThreadFields>() <= 24`, `LaneDeposit::DETACHED`'s encoded fields are 0. **Structural (D-M6):** `THREAD_BUSY` and `THREAD_RECORDS` are in `.bss`. **UG-20 reports:** `THREAD_CTX_CLAIMS`, `THREAD_CTX_REFUSED`, `THREAD_CTX_PEAK`, `POOL_WORKERS_CLAMPED` and `POOL_RESERVE_DIPS`. **Refusal:** a write with no record raises the coded cold panic; reads answer `DETACHED`; a build with no free slot panics on the building thread (§6 items 2, 8) |
| KC-36 | the window bitset holds exactly the drained indices; apply order is ascending; `pending` returns to 0 |
| KC-37 | A writer lane has one writer at a time (debug: the lane records its owner system and asserts it on `send`). `send_event` writes only on the dispatcher. Within one op, insert hooks fire in declaration order, and bundle-less ops fire in canonical type order. An `every_tick` type has no Main-schedule reader (`App::finish` refuses one with a code). In a replay session: `structural_generation()` never changes (H-17); no Fixed read of `Time`; the timestep equals the header's; the tick-start system is Fixed index 0; no keyed entity changes its (archetype, row) between ticks; the MXCSR control bits equal their session-start value. `ReplayKeyIndex` holds no duplicate key. No hashed component sits on an unkeyed entity. Hashed floats hold no NaN in gate scenes. |

## 8. Revival form: the windows word arm (D-M6w, not built)

Rev 6 does not build this arm (00 U-19). The text is kept so that an overturn of U-19 (a) or (b) builds a reviewed design rather than a new one.
- **D-M6w's touch set would be:** `boyko_threadpool/src/{thread_ctx, thread_pool}.rs`, `boyko_diag/src/lane.rs` and `boyko_log/src/codes.rs`.
- **Its gates would be:** D-M6's, plus D-M6's withdrawn phases (a) and (g) (§8.5).
- RK-14 applies again (00 §6), and erratum D0-1 is re-opened.

### 8.1 The word arm
*Moved unchanged from rev 5.1 01 §6 item 3, lines 490–510.*

   - **Word arm (`WORD_ARM`; default on `x86_64-pc-windows-gnu`, never under Miri or loom).**
     - `CTX_IDX: AtomicU32` holds 0 when no index is allocated, `u32::MAX` when `TlsAlloc` returned
       `TLS_OUT_OF_INDEXES` (this process then uses the portable word), and otherwise the index + 1.
     - `CTX_TLS_OFF: AtomicUsize` holds 0, or `0x1480 + 8·index` once the canary has validated the
       direct mapping.
     - **The canary.** It is run once, by the thread whose CAS published `CTX_IDX`:
       1. `TlsSetValue(idx, CANARY_A)`, then read `gs:[0x1480 + 8·idx]` and compare;
       2. write `CANARY_B` to that address, then compare with `TlsGetValue(idx)`;
       3. clear the slot.

       Only if `idx < 64` and both comparisons hold is `CTX_TLS_OFF` stored (`Release`). Fresh slots
       read 0 in every thread (00 §11).
     - **Reads.**
       - When `DIRECT_ARM && off != 0`, a read is `gs:[off]`: a two-instruction `core::arch::asm!`
         read inside an `#[inline]` fn. UG-15 leg (1) counts only `global_asm!`, `naked_asm!` and
         `#[naked]`.
       - Otherwise the read goes to `#[inline(never)] word_by_call()`. This is not `#[cold]`,
         because a process whose canary failed uses it on every read. It returns `TlsGetValue(idx)`
         for a valid index, reads the portable word for `u32::MAX`, and returns 0 for 0.
     - **Writes** happen only at claim, adopt and release. They use `TlsSetValue`, or the portable
       word when `CTX_IDX == u32::MAX`.

### 8.2 `boyko_diag::lane`'s word
*Moved unchanged from rev 5.1 01 §6 item 3, lines 518–532.*

   - **`boyko_diag::lane`.**
     - `LANE` keeps the same two-arm shape, with its own `LANE_IDX` and `LANE_TLS_OFF`, as a private
       copy of about 40 lines. Diag may depend on nothing (`[J]crates/boyko_diag/Cargo.toml:6-16`),
       and its growth rule keeps a non-diagnostics primitive out of it
       (`[J]docs/diagnostics/substrate/00-GOAL.md:220-228`).
     - The word holds `lane.wrapping_add(1)` as a `u16`, so 0 means `LANE_UNCLAIMED` (`u16::MAX`).
     - **Reads never allocate:** `lane()` returns `LANE_UNCLAIMED` while no index exists, as
       `lane.rs:142-149` does today.
     - **Writes.**
       - `set_lane` runs twice per `install`. It writes directly to `gs:[LANE_TLS_OFF]` when
         `LANE_DIRECT_ARM` is set and the canary validated the offset; otherwise it calls
         `TlsSetValue`.
       - `set_lane` and `claim_lane` allocate the index lazily, with the same CAS, when
         `prepare_lane()` has not run.
     - **No `Drop`.** The word keeps `LANE` free of `Drop` (`lane.rs:34-40`).

### 8.3 Arm constants
*Moved unchanged from rev 5.1 01 §6 item 3, lines 533–543 (text and table).*

   - **Arm constants.** These are `const bool`s with `cfg!` values, in the `STEAL_EMPTY_GATE` shape
     (`[J]crates/boyko_threadpool/src/worker.rs:36-39`). They are declared only where the word-arm
     module compiles, `cfg(all(windows, target_arch = "x86_64", not(miri), not(loom)))`. Everywhere
     else, the portable arm is the only arm compiled.

     | Constant | File | Default | Meaning | MQ-13's one-line patch |
     |---|---|---|---|---|
     | `WORD_ARM` | `boyko_threadpool/src/thread_ctx.rs` | `cfg!(target_env = "gnu")` | the word is an OS TLS slot; `false` → the portable word | msvc "direct" arm: `true` |
     | `DIRECT_ARM` | `thread_ctx.rs` | `true` | hot reads use `gs:[CTX_TLS_OFF]` once the canary has validated it; `false` → `TlsGetValue` on every read | gnu "call" arm: `false` |
     | `LANE_WORD_ARM` | `boyko_diag/src/lane.rs` | `cfg!(target_env = "gnu")` | as `WORD_ARM`, for `LANE` | msvc "direct": `true` |
     | `LANE_DIRECT_ARM` | `lane.rs` | `true` | as `DIRECT_ARM`, for `LANE` reads and `set_lane` writes | gnu "call": `false` |

### 8.4 `prepare()`
*Moved unchanged from rev 5.1 01 §6 item 4, lines 544–564.*

4. **`prepare()`.**
   - **What it is.** `pub fn prepare()` is `#[cold] #[inline(never)]` and idempotent.
     - On the word arm, if `CTX_IDX == 0`, it allocates the index and runs the canary. It then calls
       `boyko_diag::lane::prepare_lane()`, which does the same for `LANE_IDX`.
     - On the portable arm, both calls are no-ops.
     - Each allocation increments `CTX_INDEX_ALLOCS` or `LANE_INDEX_ALLOCS`.
   - **Where it runs.** `ThreadPoolBuilder::build` calls `prepare()` as its first statement, before
     it claims slots or spawns a worker. At `d552be05` no such call exists
     (`[J]crates/boyko_threadpool/src/thread_pool.rs:664-796`), and the only `fn prepare` in the
     crate is a task constructor (`scope.rs:1194`); D-M6 adds the call.
   - **Red-first check.** A `debug_assert!` right before the first spawn (`:721-751`) checks that
     both indices are published. Deleting the `prepare()` call trips it.
   - **Why (performance).** Without `prepare()`, W workers booting together each allocate an index,
     and all but one free it again. Besides the wasted `TlsAlloc`/`TlsFree` pairs, a loser can hold
     a low index while the winner receives one ≥ 64; the canary then fails, and the process is
     pinned to the call arm for life (RK-14).
   - **The lazy path** (a thread that touches either word before any pool is built) allocates with
     the same CAS, and the loser frees its own index.
   - **D0 erratum.** `prepare_lane()` writes two `boyko_diag` shared statics once per process, at
     the first pool build, even with diagnostics off. 00 §5 records the erratum to D0 and DG12
     (`[J]docs/diagnostics/substrate/05-LADDER-GATES.md:56-61, :133`).

### 8.5 The rest of the arm
- *Moved unchanged:* rev 5.1 01 §6 item 5, lines 578–579 (`Relaxed` on `CTX_TLS_OFF`).

   - **`Relaxed` on `CTX_TLS_OFF` is exact.** The value is written once; a stale 0 routes to the
     call arm, which re-reads `CTX_IDX` with `Acquire`.

- *Moved unchanged:* rev 5.1 01 §6's shared-state row for `CTX_IDX`, `CTX_TLS_OFF`, `LANE_IDX`, `LANE_TLS_OFF` (line 418), together with the counters `CTX_INDEX_ALLOCS` and `LANE_INDEX_ALLOCS`. *(The table header is §6's, added so the row renders; 00 §9 V-66.)*

| State | Writers | Readers | Ordering | Argument |
|---|---|---|---|---|
| `CTX_IDX`, `CTX_TLS_OFF` (threadpool); `LANE_IDX`, `LANE_TLS_OFF` (diag) | `prepare()` / `prepare_lane()`, or the first lazy writer, once each | every reader | `*_IDX`: CAS `AcqRel` / `Acquire` on the cold path, `Acquire` load in the call arm. `*_TLS_OFF`: one `store(Release)` after the canary; hot read `Relaxed` | write-once. A reader sees either 0, which routes to the call arm (which re-reads `*_IDX` with `Acquire`), or the validated offset; both yield a correct read |

- *Moved unchanged:* erratum D0-1, the rev 5.1 00 §5 row beginning `` | `[J]docs/diagnostics/substrate/05-LADDER-GATES.md` (the D0 line item ``. Critic pass 5's O3 remains its open remark (00 §10). *(The header is 00 §5's, added so the row renders; 00 §9 V-66.)*

| Document | Patch |
|---|---|
| `[J]docs/diagnostics/substrate/05-LADDER-GATES.md` (the D0 line item `:56-61`; DG12 `:133`), and the DG12 comment at `[J]crates/boyko_diag/src/lane.rs:105-107` | **Erratum D0-1.** D-M6 writes the code comment; DOC-2 writes the document. On a host where `LANE` lives in a word-arm OS slot (windows-gnu by default, 01 §6 item 3), the first `ThreadPoolBuilder::build` calls `boyko_diag::lane::prepare_lane()` once per process, even with diagnostics off. That call makes one `TlsAlloc`, runs one canary, and writes once to each of two shared statics, `LANE_IDX` and `LANE_TLS_OFF`. D0's "every one-time cost runs on the enable path" and DG12 (b)'s "no `boyko_diag` shared static" each gain one named exception. The reason is the one DG12 already gives for the TLS `LANE` cell: the two values identify the per-thread lane slot rather than holding diagnostic state, and they are written at pool build, not at process start. Nothing else in D0 changes: no calibration, no spare claim, no lane-buffer write, no session id. The DG12 leg that D-M6 adds (02 §2, D-M6 phase (g)) makes a document–code drift go red (critic pass 4, O1). *Writer check (§9 V-55): DG12's own reason for excluding `LANE` is that D1 mandates that write and that it costs 2 B of per-thread TLS and no `.bss` (`05-LADDER-GATES.md:133`). `LANE_IDX` and `LANE_TLS_OFF` are neither D1-mandated nor TLS; they are shared statics in `.bss`. The exception therefore rests on 01 §6 item 4's performance argument, not on DG12's existing reason.* |

- *Moved unchanged:* D-M6's withdrawn tests, phase (a) "Prepare" and phase (g) "DG12 leg" (rev 5.1 02 lines 281–287 and 320–323).

     - **(a) Prepare.**
       - The first pool build publishes both indices before its first spawn (the `debug_assert!` of
         01 §6 item 4).
       - On the word arm, after booting W = 64, `CTX_INDEX_ALLOCS == 1` and
         `LANE_INDEX_ALLOCS == 1`; on a portable host both read 0, and the phase asserts that
         instead.
       - Red-first: delete `prepare()` from `build` → the debug assert trips.
     - **(g) DG12 leg (00 §5, erratum D0-1).** With diagnostics off and after (a):
       - `boyko_diag::lane::lanes_leaked() == 0`;
       - no spare is claimed;
       - `LANE_IDX` and `LANE_TLS_OFF` are the only `boyko_diag` statics that hold a non-zero value.

- The UG-20 fields for the arm in use and for both indices (rev 5.1 03 line 29 and 01 line 834). Rev 5.1 03 UG-20 read: "For KC-04: the arm in use (direct / call / portable), `CTX_IDX`, `THREAD_CTX_CLAIMS`, `THREAD_CTX_REFUSED`, `THREAD_CTX_PEAK`, `POOL_WORKERS_CLAMPED`, `POOL_RESERVE_DIPS` and `CTX_INDEX_ALLOCS`. For `boyko_diag`: the lane arm, `LANE_IDX` and `LANE_INDEX_ALLOCS` (01 §6)". Rev 5.1 01 §7's KC-04 row reported "the arm in use (direct / call / portable) and the index for KC-04 and for `LANE`, plus `THREAD_CTX_CLAIMS`, `THREAD_CTX_REFUSED`, `THREAD_CTX_PEAK`, `POOL_WORKERS_CLAMPED`, `POOL_RESERVE_DIPS` and both index-allocation counters".
