# Allocator design space - rev 1, its first critique, and rev 2 — current: ~~rev 2.5~~ ~~rev 2.6 (2026-09-23)~~ rev 2.7 (2026-09-24), with its closure (P58)

Date: 2026-09-10 (Parts I-II), 2026-09-16 (Part III). Trees: **ecsnative** = `D:/wt/ecsnative` (`feat/ecs-native-storage` @ `ad0ebea4`), **merge** = `D:/wt/merge` (`merge/ke16-into-render` @ `d2c8c646`), **joltab** = `D:/wt/joltab` (`merge/ke16-into-ecsnative` @ `d552be05`, the tree Part III reads), **main** = `D:/claude/BoykoEngine` (read-only). Companion: [ALLOCATOR-RESEARCH.md](ALLOCATOR-RESEARCH.md) (the four lens reports this design is built on).

> ⚠ **STATUS (2026-09-24, rev 2.7 closure): critique pass 9 (AP9), a closure pass on rev 2.7, returned CHANGES_REQUESTED with 0 Critical and 4 Important remarks (W1–W4) and two questions. [The rev-2.7 closure](#rev-27-closure-2026-09-24-p58-which-answers-critique-pass-9), now the last Part, resolves all four and answers both questions (P58); the pass-9 log precedes it. No owner ruling is needed. AP9 held only the first modding-crate rung, never C1. Open for the plan, not for this file: adopting P56 with P58's changes as the ban's mechanism, at that rung. P58's plan side is applied in the same step.** *Rev 2.7's status, superseded:* ~~**STATUS (2026-09-24, rev 2.7): critique pass 8 (AP8), a closure pass on rev 2.6, returned CHANGES_REQUESTED with 0 Critical, 1 Important (N-W1) and 3 Optional remarks (O1–O3). It found every AP7 remark and answer resolved, and C1's prerequisite met. [Rev 2.7](#rev-27-2026-09-24), now the last Part, resolves N-W1 and adopts O1–O3 (P56, P57); the pass-8 log precedes it. No owner ruling is needed. N-W1 held only the first modding-crate rung, never C1. Open for the plan, not for this file: adopting P56's candidate mechanism for the modding-crate ban on runtime-invoked code, at that rung, and applying the same-line plan patches that P57 proposes. Declaring C1's prerequisite met is the orchestrator's.**~~ *Rev 2.6's status, superseded:* ~~**STATUS (2026-09-23, rev 2.6): critique pass 7 (AP7) reviewed rev 2.5 and returned CHANGES_REQUESTED with 0 Critical, 2 Important and 5 Optional remarks. [Rev 2.6](#rev-26-2026-09-23), now the last Part, resolves both Important remarks and adopts all five Optional ones (P52–P55); the pass-7 log precedes it. No owner ruling is needed. One item stays open for the plan, not for this file: a mechanism for the modding-crate `#[used]`/ctor ban (P53.3). C1's prerequisite, "AP7 has closed rev 2.5" (plan 02 §2), is the orchestrator's to declare.**~~ *Rev 2.5's status, superseded:* ~~**STATUS (2026-09-23): rev 2.5 is written — step DOC-1 of the unified system plan — and awaits critique pass 7 (AP7), whose scope is the rev-2.5 delta and the pass-6 dispositions only; C1 waits for AP7.**~~ [Rev 2.5](#rev-25-2026-09-23) is ~~the last Part of this file~~ the Part before the pass-7 log. It moves the Heap class (with `HeapDyn`), `TableSet` and `DropColumn` to revival forms (P46), re-points §7 to the plan's file 05 (P47), keeps the thread context out of `boyko_memory` (P48), strikes `HeapRef::alloc_cold` from P29 (P49), gives every pass-6 remark a disposition (P50) and fixes pass 6's stale passages in place (P51). Its in-place markers begin `⚠ Rev 2.5`, and no line above it moved. *Superseded status:* ~~**STATUS (2026-09-17): closed at rev 2.4 by orchestrator ruling.** Critique pass 6 found no~~
> ~~Critical remark; its five Important remarks (W1-W5) are recorded as OPEN at the end of this file and~~
> ~~are resolved in the rungs that implement the affected items.~~ The unified system plan
> (`docs/unification/UNIFIED-SYSTEM-PLAN-*.md`) is the binding reading of this design: its rulings
> U-1 (the Heap class is deferred, not built), U-4, U-6, U-7 and U-8 supersede the corresponding
> parts here. *(Rev 2.5: so do U-2, U-9, U-10, U-11 and U-19 where they touch this file; P46–P51 write them in.)*

> ⚠ **REV 2 EXISTS, and it is Part III at the end of this file ([Rev 2 (2026-09-16)](#rev-2-2026-09-16)).**
> It is a PATCH against rev 1, not a replacement: Part I stays verbatim, and every change in Part III
> names the Part I text it replaces. Read Part III before acting on any §-numbered statement below,
> because a rev-1 sentence that Part III removed still reads as current where it stands.
>
> **What rev 2 closes.** Both blocking findings — **C1** (Heap bookkeeping moves INSIDE the
> reservation; handles name the reservation base, never the `Heap` struct) and **C2** (the
> `ScopeArena` is DELETED rather than repaired, so the role-sentinel collision class ceases to
> exist) — each with a red-first Miri test specified. Every `FIX IN REV 2` row is discharged
> (**W1-W5, O1-O4, O6-O8, Q1-Q6**); **O5 is RETIRED with its reason** because the mechanism that
> raised it is removed. Three things rev 1 did not have are added: the **throughput justification is
> WITHDRAWN** (the mimalloc A/B was null) and replaced by three that survive the null; **TB-1 keeps
> its rule and corrects its rationale**; and **§7, the modding seam** — nine kernel entries and gate
> G6, answering the owner's hard requirement that a game not using modding pay nothing.
>
> **Not re-opened:** the central fork of §1 (per-type replacement by lifetime class; `allocator_api`
> rejected; `#[global_allocator]` kept only as the deny gate). Pass 1 did not contest it. One clause
> of its *justification* is corrected in P0 because a measurement forces it.

> ⚠ **Read first (added 2026-09-11): this rev 1 was built on a thread pool WITHOUT Stage 3b.**
> The `ecsnative` tree it cites (`ad0ebea4`) does not contain `d51b4ced` ("stage 3b lands the
> per-scope block"). The shipped KE16 pool (`feat/threadpool-ke16`, merged into
> `merge/ke16-into-ecsnative` and `merge/ke16-into-render`) does. There, scoped task cells are
> already emplaced in the per-scope `ScopeBlock`.
>
> Measured on that tree (`crates/boyko_physics/tests/alloc_frame_census.rs` on
> `merge/ke16-into-ecsnative`):
>
> | scene | pre-3b (cited below) | KE16 pool |
> |---|---|---|
> | App frame | `n + 4` allocations | a flat 2 (one scope frame and one 4 KiB chunk per `Schedule::run`) |
> | parallel physics step | ~2,670 | 302–339 |
>
> A system-heap vs mimalloc A/B on the KE16 tree was null, so these allocations are not a speed
> lever; they are removed for unification.
>
> Rung 1's "one heap cell per task" is therefore already done on the shipped pool. What remains is
> the scope frame and the chunk, both freed at the join and allocated again by the next scope.
> Rev 2 starts from these facts. The design and the critique below are unchanged.

Structure of this file: Part I is the architect's rev-1 design reproduced VERBATIM (including its opening orientation note); it has NOT been edited to answer the critique. Part II is the critique log of pass 1, one row per finding with its evidence and the action it takes into rev 2. **Part III is rev 2** — it carries the heading `Rev 2 (2026-09-16)` rather than the word "Part", and it is a delta against Part I: P0-P14, each naming the Part I text it removes and the text that replaces it, then a change log mapping every pass-1 row to its disposition, then the open questions for pass 2. Provenance tags as in the research file: [S] source read, [D] official doc / paper, [B] blog or talk (recorded, not relied on); in-tree claims carry `[tree] path:line`. **Rev 2.5 (2026-09-23)** follows critique pass 6's review at the end of the file. It is the first revision that also marks superseded text where it stands, Part I included: a marker is a suffix on the existing line that begins `⚠ Rev 2.5`, or stale words struck through beside their correction. No sentence is deleted, and no line is inserted before rev 2.5, because 24 other documents cite this file by line number.

---

# Part I - Design, rev 1 (architect, verbatim)

The facts I need are confirmed: `VmReservation` is reserve/commit/release with no reset (`vm.rs:85-97, 199` ecsnative), `VmColumn` rejects non-dividing sizes (`vm_column.rs:144-149`), the scope block takes chunks from `std::alloc` (`block.rs:101`, merge), every `install` boxes a `ScopeShared` (`thread_pool.rs:278`, merge), and the packing plan makes the first commit exactly one 4 KiB page (`POOL-SUBGRANULAR-PACKING-PLAN.md:136-141`). graphify is not installed on this machine (project memory) and no shell tool is available, so orientation came from the four lenses plus these targeted reads.

# Allocator strategy - the design space for THIS engine

Rev 1. Architect. Trees: **ecsnative** = `D:/wt/ecsnative` (`feat/ecs-native-storage` @ `ad0ebea4`), **merge** = `D:/wt/merge` (`merge/ke16-into-render` @ `d2c8c646`). Provenance: `[tree] path:line`; external `[S]`/`[D]`/`[B]` as in the lenses.

## 0. The ruling, restated as a falsifiable end state

**Target.** In steady state (every frame after boot), no code path of the engine's own crates makes any allocation through any allocator other than `boyko_memory`. "Engine's own crates" = every workspace crate that is linked into a shipped binary. Structurally: no struct field, local, or temporary of type `Vec`/`Box`/`String`/`Arc`/`Rc`/`VecDeque`/`BTree*`/`HashMap`, no `vec!`/`format!`/`collect()`/`to_string()`/`Box::new` in those crates, except where a line-local `#[allow]` carries a written rationale from the closed vocabulary below.

**What stays foreign, by name and by phase** (the exception list is closed; the ledger in §3 classifies each site with one of these tags):

| Tag | What | Phase | Why accepted |
|---|---|---|---|
| `os-thread` | `std::thread::spawn` per worker: `Arc<Packet>`, `Thread`, 3 `Box`es `[S std lifecycle.rs]` | boot only | replacing std thread creation buys nothing per frame |
| `panic` | panic payload `Box<dyn Any + Send>` | failure path | a panic already fails the frame gate |
| `ffi-driver` | Vulkan driver host allocations (`VkAllocationCallbacks` left null) | any | driver threads call back; Principle 0 FFI exception; routing them would force a thread-safe heap for a foreign caller |
| `harness` | libtest / criterion | outside the measured window | not linked into shipped binaries |
| `offline` | `boyko_macros` (proc-macro), `aether_lang`, `boyko_shaderdsl` printer, `boyko_fontbake` | compile/bake time | not in the runtime binary or run once at bake |
| `diag` | `BOYKO_HOST_DUMP`, profiling artifact writer, error `String`s in `asset/error.rs` | cold, feature-gated, outside steady state | rung 4 migrates them to the `LogRing` pattern; until then they are outside the window by construction |

Everything else — including the threadpool's `Box<ScopeShared>`, the scope block's `std::alloc` chunks, and crossbeam-deque's runtime buffers — is **in scope**. Principle 0's "lock-free threadpool internals" exception covers the *shape* (ring buffers, atomics), not the allocator they come from; ruling 3 is explicit.

**The gates that prove it** (full ladder in §3):

1. *Runtime*: `alloc_frame_census.rs` (ecsnative, untracked) turned into a gate whose per-scene `STEADY_MAX` constants only decrease per rung and reach **0** at the end state. Lower bound only (the optimizer may elide allocations `[D GlobalAlloc docs]`), hence:
2. *Structural*: `DenyAfterSteady` — a `#[global_allocator]` in the gate binaries that delegates to `System` during boot and **aborts** (no unwind allowed inside `GlobalAlloc`) on any call once `enter_steady()` has been called, printing a fixed message through the OS write call. This is the "memory library is the global allocator" gate the brief names, in its only sound form: the library is *not* the process allocator (§1), so the process allocator is made to *refuse*.
3. *Static*: the field ledger (immediately), then clippy `disallowed-types` over the std paths once the ledger is small.

## 1. The central fork — decided

**Decision: (i) per-type replacement by lifetime class, with the memory library gaining four classes and their primitives. (ii) `allocator_api` is rejected as a mechanism and not implemented. (iii) the library as `#[global_allocator]` is rejected as an allocator and kept only as the *deny* gate.**

### Costs and reach of each option, priced on Lens C's counts

| | (i) per-type replacement | (ii) `Vec<T, Mem>` via `allocator_api` | (iii) library as `#[global_allocator]` |
|---|---|---|---|
| Site churn, kernel fields | 88 alloc-typed non-test fields (`[ecsnative]` boyko_ecs, Lens C §3.2) — every one changes type | the same 88 change type (`Vec<T>`→`Vec<T, Mem>`), plus a newtype anyway (clippy cannot tell `Vec<T>` from `Vec<T, A>`: both are `alloc::vec::Vec`) | 0 |
| Site churn, kernel locals | builder-concentrated: `schedule_builder.rs` 68 sites, `state.rs` 1, `schedule.rs` 3 in production (Lens C §3.2) | identical set, each becomes `Vec::new_in(mem)`; `vec!`/`collect()`/`format!` have no `_in` form → rewritten anyway | 0 |
| Workspace fields | 229 runtime-crate fields (Lens C §3.1) | 229 | 0 |
| `String`/`format!`/`HashMap`/`Arc` | replaced by library types | **no `A` parameter exists** for `String`/`format!`/std `HashMap` `[D]`; `Arc<T, A>` nightly only | routed, unchanged |
| Third-party runtime allocations (crossbeam, std thread) | replaced (deque) / accepted (thread) | not reached | routed, not removed |
| Data-shape change | **yes**: address-stable columns, zero-fill semantics, 16-24 B handles | none — `Vec<T, A>` still `grow`s by allocate+copy+free; `[D Allocator docs]` | none |
| Toolchain | stable | nightly until ≥ early 2027 `[D PR #156882 FCP 2026-09-09]`; the tree records nightlies breaking `component_pool.rs` size pins (`[main] rust-toolchain.toml:24-25`) | stable |
| Library work | 4 classes, ~2.5k LOC (§2) | same allocator surface **plus** an `Allocator` impl that must serve arbitrary `Layout`s and `grow`/`shrink` | a general-purpose, thread-safe, non-unwinding malloc — the `MemFreeBlockMaster` class deleted in X.J (`[ecsnative]` git `1ee2c461`) plus concurrency it never had |
| Existing gate family | untouched | untouched | **conflicts**: one `#[global_allocator]` per binary; 22 test binaries + 30 benches + `profiling-alloc` shim each install their own (`[ecsnative] boyko_app/src/profiling/alloc_shim.rs:139-144`) |
| Miri | fallback arm per class (one `alloc_zeroed`) | same | fallback arm calls the global allocator (`vm.rs:168-179`) → recursion; must special-case |
| Codegen | direct | `Box<T, A≠Global>` loses `noalias` (decided in #156882) | none |

### Why (i)

- **Ruling 1 and 2 are about shape, not routing.** The library was built to *minimise* allocations, and the owner's objection is to `Vec` *inside the ECS*. (iii) satisfies the letter of ruling 3 while leaving every `Vec` in place, still reallocating and moving; the SP4 root cause class (relocation under a held pointer, `[ecsnative] docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:19`) is untouched by it. (ii) leaves the same relocation.
- **The kernel's hot bookkeeping is served better by bespoke shapes than by any generic container.** `EntitySlotMap.slots` and `LiveBitmap.words` are index-addressed and can read zero as "absent" — on a `VmColumn` the zero-fill contract (`[ecsnative] vm.rs:19-37`) makes growth a commit with **no fill loop**; a `Vec<T, A>` must `resize` and write every element. `free_entity_ids` / `DenseStore.free` are LIFO stacks whose pop order is determinism data; a column stack preserves the order bit-for-bit and never moves.
- **Berger/Zorn/McKinley `[D OOPSLA 2002]`**: general-purpose custom allocators beat a good malloc in 2 of 8 programs, and the two winners were *regions*. (iii) spends the effort to be at best equal to mimalloc; (i) spends it on the shape that wins — regions (frame, scope) and typed columns.
- **The tree is already on route (i)** and every step measured a win: X.G (`Vec<EntityInland>` → `InlandStore`, spawn 15-54 % faster), F1/F3 (`entity_ids`, `s2e` → `VmColumn`), L5 (`LogRing`), 2026-09-09 (`ScratchColumn` "beats std::Vec", commit `0a803cfc`). The design below finishes that migration rather than starting a different one.
- **Enforceability.** Under (i) the end-state gate is a path ban on `alloc::vec::Vec` etc. Under (ii) the ban must be on a newtype the engine defines anyway — so (ii) collapses into (i) with std doing the buffer arithmetic. Under (iii) nothing static can be enforced at all.

### What (i) cannot do, and what stays where

- **Address stability under growth is a property of `VmColumn`/`ComponentPool`/`InlandStore` only.** Every site where a pointer is held across a push keeps those primitives: pool rows (`row_ptr`, `[ecsnative] component_pool.rs:816-834`), `ScratchSolveView` copies held by workers (`scratch_column.rs:28-38`), `s2e`, `entity_ids`, the inland store. The new `HeapVec<T>` **relocates on growth** exactly like `Vec` and is therefore allowed only where no pointer into it outlives a push — a `debug_assert!`-free rule enforced by review and by the ledger's `class` column (`heap` sites must state "no pointer held"). ⚠ *Rev 2.5 (P46): `HeapVec` is a revival form, not built (U-1); its relocate-on-grow clients take KC-15 spans, which relocate on grow too (plan 01 KC-15).*
- **Third-party runtime allocations are not reached by (i).** The one such site on the frame path is crossbeam-deque (`Injector::push` allocates a 31-slot block; `Worker` doubles; crossbeam-epoch allocates a node per sealed bag `[S deque.rs, internal.rs]`) — replaced by an in-house bounded deque in rung 1d. `std::thread::spawn` is accepted (tag `os-thread`).
- **`String` ergonomics** — `format!` is gone from engine crates; `core::fmt::Write` into a `HeapString`/`LogRing` replaces it.

### Rejected alternatives, closed

- **(ii) as a bridge for droppable/setup-once structures only.** Rejected: the same sites are served by `DropColumn<T>` and `HeapVec<T>` without nightly, and a second container vocabulary would split the codebase in two styles. No `Allocator` impl ships in v1 — it would be client-less, the exact X.J retirement criterion. ⚠ *Rev 2.5 (P46): the rejection stands; those sites are now served by KC-16's owning column and the ledger forms, because `DropColumn` and `HeapVec` are revival forms (U-8, U-1).*
- **`allocator-api2` shim on stable.** Rejected: a third-party container as the engine's canonical vector contradicts the in-house rule for engine libraries, and it still needs the newtype for the gate.
- **A process-singleton heap (`&'static`).** Rejected: tests build many `EcsMaster`s concurrently; a single-writer heap shared across them is a data race. Heaps are per owner.

## 2. What the memory library gains

### 2.0 The crate split (architecture decision)

`boyko_memory` — new crate containing `vm.rs`, `vm_column.rs`, `utils.rs`, the granule/page constants, and the four classes below. `boyko_ecs` keeps `component_pool.rs`, `device_column.rs`, `inland_store.rs`, `scratch_column.rs` (they know ticks, ids, and the stagger). Reason: `boyko_threadpool` and `boyko_utils` need reservations and cannot depend on `boyko_ecs` (cycle: `boyko_ecs` → `boyko_threadpool`). `boyko_memory` has no dependencies beyond the OS crates already used by `vm.rs`. ⚠ *Rev 2.5 (P48, U-19): the engine thread context is not placed here — it lives in `boyko_threadpool`, which keeps G5's `#![no_std]` for this crate; and of the classes below this crate holds none (Heap and Table are revival forms, P46; Frame is deleted, P34; Scope's chunk structures are threadpool-internal, P2/P16). The current §2.0 is P48.*

### 2.1 Lifetime classes — the table

| Class | Primitive(s) | Lifetime | Owner / thread rule | Free semantics | Zero-fill | Reset | Address-stable | Resident floor per instance (pre-plan → post-plan) |
|---|---|---|---|---|---|---|---|---|
| **Column** (exists) ⚠ *Rev 2.5 (P46): `DropColumn` is a revival form, U-8 → KC-16* | `ComponentPool`, `VmColumn<T>`, `InlandStore`, `ScratchColumn` + new `DropColumn<T>`, `ByteColumn` | persistent | single writer (`&mut`), readers anywhere | `len` rollback / `swap_remove` | yes, fresh commit only | none | **yes** | 64 KiB → 4 KiB (lazy: 0 until first push) |
| **Frame** | `FrameArena` + `FrameVec<T>`, `FrameSlice<T>` | one frame between two `reset()` calls at a fixed schedule position | **dispatcher thread only** | none; `mark()/rewind(mark)` LIFO | **no** | `reset()` at frame start | within a frame | 0 until used → high-water mark, page-rounded |
| **Scope** | `ScopeArena` (one per thread slot, `W+1`), replaces `ScopeBlock`'s `std::alloc` chunks and `Box<ScopeShared>` | one `Scope` (nested LIFO) | the thread that opened the scope; indexed by `wid` from the existing single TLS read | `rewind(mark)` at join | **no** | at join | within a scope | 0 until used → HWM; typical ≤ 64 KiB |
| **Heap** ⚠ *Rev 2.5 (P46): revival form, not built (U-1)* | `Heap` + `HeapVec<T>`, `HeapBox<T>`, `HeapDyn<V>`, `HeapString`, `SortedMap<K,V>` | persistent, individually freed | **single writer** (structural mutation = dispatcher-only, EM2 `[ecsnative] entity_master.rs:16`); readers anywhere | size-class intrusive LIFO free lists | **no** (reused) | none | **no** (relocates on grow) | 64 KiB for the whole heap (self-packed at 4 KiB pages) — independent of the packing plan |
| **Table** ⚠ *Rev 2.5 (P46): revival form, not built (U-7 → KC-18)* | `TableSet` — one reservation, N fixed-length typed tables laid out at build | build-once | single writer at build; readers anywhere | none | yes | none | yes | total bytes granule-rounded: 64 KiB → 4 KiB |

Five rows because `TableSet` is a *shape* of the Column class (fixed length, many tables, one reservation) that kills ~20 `Box<[..]>` fields with one granule; it is listed separately because its API differs. ⚠ *Rev 2.5 (P46): of the five, Column and Scope are built; Frame is deleted (P34); Heap and Table are revival forms.*

### 2.2 The library's own contract additions (to `VmReservation`)

- `commit` alignment relaxes to `COMMIT_PAGE` — this is packing-plan D1 (`[ecsnative] docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md:90-101`), not a new decision; the Frame/Scope/Heap classes use the page quantum from day one because they pack their own granules.
- **No decommit/reset primitive is added.** Frame and Scope floors are the high-water mark. Reason: Dawson's numbers `[B randomascii 2014]` — ~175 µs/MB first-touch faults + ~150 µs/MB kernel zeroing on release; a per-frame trim would pay that every frame; a periodic trim is a policy with no measured need. Revisit only with a measured resident number.
- `VmColumn::new` drops the `COMMIT_GRANULE % size_of::<T>() == 0` pin (`[ecsnative] vm_column.rs:144-149`). New rule: `committed_elems = committed_bytes / size_of::<T>()` (floor). The pin existed to keep `committed_elems * SIZE` exact for the commit range; computing the range in bytes and the capacity by floor division keeps every commit page-aligned for any element size. Admits 12/24/40 B elements (`traverse_iter.rs:284`, `serialize/mod.rs:288`, `bindless.rs:74`).

### 2.3 Data structures

```rust
// ---------- Column class additions (boyko_memory) ----------  // ⚠ Rev 2.5 (P46): DropColumn is a revival form (U-8 → KC-16); ByteColumn stays (KC-03)

/// VmColumn without the Copy bound: runs destructors on truncate/swap_remove/Drop.
/// Same reservation shape, same growth ladder, same address stability.
pub struct DropColumn<T> {
    base: NonNull<T>,          // write-once after first reserve (lazy)
    len: u32,                  // element count
    committed: u32,            // elements the committed frontier covers
    reserve_elems: u32,        // VA ceiling
    res: Option<VmReservation>,// None until first push
}
// size 32 B. `Drop`: drop_in_place over [0, len), then release.

/// Byte column with raw reserve/set_len for record arenas (CommandQueue, ErasedKindBuffer).
pub type ByteColumn = VmColumn<MaybeUninit<u8>>;   // + spare_ptr(additional) -> NonNull<u8>, unsafe set_len(n)

// ---------- Frame class ----------

/// Single-writer bump arena. NOT Sync; owned by EcsMaster; reset once per frame
/// by `frame_reset_system` (first system of the schedule, exclusive).
#[repr(C)]
pub struct FrameArena {
    cur: usize,                 // HOT: bump cursor (byte offset)
    end: usize,                 // HOT: committed frontier (byte offset)
    base: NonNull<u8>,          // write-once
    hwm: usize,                 // high-water mark (diagnostics + the floor number)
    res: Option<VmReservation>, // lazy; VA = FRAME_ARENA_RESERVE (256 MiB, Miri: 4 MiB)
}
pub struct FrameMark(usize);    // LIFO rewind token; `#[must_use]`

/// Fixed-capacity vector on the frame arena. Capacity chosen at creation;
/// overflow is a #[cold] re-bump + copy (bumpalo semantics) - callers size it.
pub struct FrameVec<'f, T> { ptr: NonNull<T>, len: u32, cap: u32, _f: PhantomData<&'f FrameArena> }
// 16 B. No Drop for T (T: Copy bound in v1 - same rule as VmColumn; DropColumn covers droppable persistent data,
// droppable per-frame data has no client in the inventory).

// ---------- Scope class ----------

/// One per thread slot in the pool (W workers + the dispatcher slot), padded
/// against false sharing between neighbouring workers' cursors.
#[repr(C, align(64))]
pub struct ScopeArena {
    cur: usize,      // HOT
    end: usize,      // HOT
    base: NonNull<u8>,
    hwm: usize,
    res: Option<VmReservation>,   // lazy; VA = SCOPE_ARENA_RESERVE (64 MiB per slot, Miri 1 MiB)
    _pad: [u8; 24],
}
// Accessed only by the owning thread through `UnsafeCell` in the pool's per-slot table.

// ---------- Heap class ----------  // ⚠ Rev 2.5 (P46): revival form, not built (U-1, U-9)

/// Single-writer size-class heap on two reservations.
pub struct Heap {
    small: VmReservation,                 // 4 GiB VA (Miri 8 MiB); 4 KiB pages assigned to classes on demand
    small_frontier: usize,                // next unassigned page offset (page-aligned)
    small_committed: usize,               // granule-rounded commit frontier
    free: [u32; CLASS_COUNT],             // per-class intrusive LIFO head: offset/16 from `small.base`, 0 = empty
    large: VmReservation,                 // 16 GiB VA (Miri 8 MiB); granule-rounded runs
    large_frontier: usize,
    large_free: VmColumn<RunFree>,        // (offset_granules: u32, len_granules: u32) LIFO per exact size, first-fit
    #[cfg(debug_assertions)] owner: ThreadId,
    #[cfg(any(miri, feature = "mem-sanitize"))] live: VmColumn<u8>,   // 1 bit per 16-B unit: the FLECS_SANITIZE analogue
}
// CLASS_COUNT = 24: 16*k for k in 1..=16 (16..256 B), then 512, 1 KiB, ... 64 KiB.
// class(layout) = branchless: size <= 256 ? (size+15)>>4 - 1 : 16 + (ceil_log2(size) - 9).
// align <= 16 served by class granularity; align 32/64 rounds size up to a class that is a multiple
// of the alignment and pages are 4 KiB-aligned, so every chunk of a class whose size is a multiple
// of 64 is 64-aligned. align > 64 -> large tier (granule-aligned).

pub struct HeapVec<T> {
    ptr: NonNull<T>,   // dangling when cap == 0
    len: u32,
    cap: u32,          // elements; derived from the class at allocation, stored to avoid the class lookup on push
    heap: NonNull<Heap>,   // the owner; Drop frees through it
}
// 24 B (same as Vec). Deref<Target=[T]> gives sort/binary_search/iter for free.
// Invariant HV-1: `heap` outlives self (owner drop order, §5).
// Invariant HV-2: no pointer into the buffer is held across a push (review rule; ledger column).

pub struct HeapBox<T> { ptr: NonNull<T>, heap: NonNull<Heap> }      // 16 B, sized T only

/// Type-erased object with a static vtable - the threadpool `Task` pattern (16 B thin),
/// replacing Box<dyn System>, Box<dyn FnOnce(&mut EcsMaster)>, Box<TermList>.
pub struct HeapDyn<V: 'static> { data: NonNull<u8>, vtable: &'static V, layout: Layout, heap: NonNull<Heap> }
pub trait DynVTable { unsafe fn drop_in_place(&self, p: NonNull<u8>); }
// V is a per-trait struct of fn pointers built by a generic `const fn of::<T>() -> &'static V`.

pub struct HeapString(HeapVec<u8>);   // utf8 invariant; impl core::fmt::Write; Deref<str>

/// Sorted-vector map for build-time lookups that were HashMaps. O(log n) get, O(n) insert.
pub struct SortedMap<K: Ord + Copy, V> { keys: HeapVec<K>, vals: HeapVec<V> }

// ---------- Table class ----------  // ⚠ Rev 2.5 (P46): revival form, not built (U-7 → KC-18)

/// One reservation, N typed fixed-length tables, laid out at build.
pub struct TableSet { res: VmReservation, used: usize }
pub struct TableLayout { size: usize }                  // builder: accumulates aligned field sizes
pub struct Table<T> { ptr: NonNull<T>, len: u32 }       // 16 B handle into a TableSet; Deref<[T]>
```

### 2.4 Public API (signatures)

```rust
// Column  // ⚠ Rev 2.5 (P46): the DropColumn block is a revival form (U-8)
impl<T> DropColumn<T> {
    pub fn new(label: &'static str, reserve_elems: usize) -> Self;
    pub fn push(&mut self, v: T) -> u32;                 // warm compare len < committed, #[cold] grow
    pub fn swap_remove(&mut self, i: u32) -> T;
    pub fn truncate(&mut self, n: u32);                  // drops [n, len)
    pub fn as_slice(&self) -> &[T]; pub fn as_mut_slice(&mut self) -> &mut [T];
}
impl VmColumn<MaybeUninit<u8>> {
    pub fn spare_ptr(&mut self, additional: usize) -> NonNull<u8>;   // commits if needed
    pub unsafe fn set_len(&mut self, n: usize);
}
// VmColumn<T: Copy> gains: stack helpers `pop() -> Option<T>`, `ensure_len_zeroed(n)` (commit-only growth,
// relies on the zero-fill contract; no write loop).

// Frame
impl FrameArena {
    pub fn new(label: &'static str) -> Self;                          // no reservation yet
    pub fn alloc(&mut self, layout: Layout) -> NonNull<u8>;           // bump; #[cold] commit path
    pub fn alloc_slice_uninit<T>(&mut self, n: usize) -> NonNull<T>;
    pub fn vec<T: Copy>(&mut self, cap: u32) -> FrameVec<'_, T>;
    pub fn mark(&self) -> FrameMark;
    pub fn rewind(&mut self, m: FrameMark);                            // debug_assert LIFO
    pub fn reset(&mut self);                                           // cur = 0; hwm updated
    pub fn high_water(&self) -> usize;
}

// Scope (threadpool-facing)
impl ScopeArena {
    pub fn mark(&self) -> usize;
    pub fn bump(&mut self, size: usize, align: usize) -> NonNull<u8>; // the ScopeBlock::bump shape: 2 loads, mask, 2 cmp, 1 store
    pub fn rewind(&mut self, mark: usize);
}

// Heap  // ⚠ Rev 2.5 (P46): revival form (U-1)
impl Heap {
    pub fn new(label: &'static str) -> Self;
    pub fn alloc(&mut self, layout: Layout) -> NonNull<u8>;          // never zeroed
    pub unsafe fn free(&mut self, p: NonNull<u8>, layout: Layout);   // class from layout; no header
    pub fn resident_bytes(&self) -> usize;
}
impl<T> HeapVec<T> {
    pub fn new_in(heap: &mut Heap) -> Self;                          // cap 0, no allocation
    pub fn with_capacity_in(heap: &mut Heap, cap: u32) -> Self;
    pub fn push(&mut self, v: T);                                    // #[cold] grow: alloc new class, memcpy, free old
    pub fn pop(&mut self) -> Option<T>;
    pub fn insert / remove / swap_remove / retain / truncate / clear / extend_from_slice / drain(..)
    pub fn as_slice / as_mut_slice; // + Deref/DerefMut<[T]>, IntoIterator for &/&mut
}
impl HeapDyn<V> { pub fn new<T>(heap: &mut Heap, value: T, vtable: &'static V) -> Self; pub fn data(&self) -> NonNull<u8>; pub fn vtable(&self) -> &'static V; }

// Table  // ⚠ Rev 2.5 (P46): revival form (U-7)
impl TableLayout { pub fn new() -> Self; pub fn field<T>(&mut self, n: usize) -> TableSlot<T>; }
impl TableSet { pub fn build(layout: TableLayout) -> Self; pub fn table<T>(&self, s: TableSlot<T>) -> Table<T>; }
```

### 2.5 Algorithms on the critical paths

| Op | Steps | Big-O | Cache | Branches | Notes |
|---|---|---|---|---|---|
| `FrameArena::alloc` | `p = align_up(cur, a); n = p+size; if n > end {cold commit}; cur = n` | O(1) | one line (`cur`,`end`), sequential writes into the arena | 1 predictable | identical to `ScopeBlock::bump` (`[merge] block.rs:369-398`) |
| `ScopeArena::bump` | same | O(1) | per-thread line, no sharing | 1 | replaces `alloc()`+`dealloc()` per chunk and `Box::new(ScopeShared)` per scope |
| `Heap::alloc` small | `c = class(size)`; `h = free[c]`; if `h != 0` {pop: `free[c] = *(base+h*16) as u32`} else {cold: assign page, thread chunks} | O(1) | touches the head chunk (one line) — the LIFO keeps it warm | 1 + cold | intrusive link is a `u32` at chunk offset 0, written/read via raw pointers only on **freed** chunks |
| `Heap::free` small | `*(p) = free[c]; free[c] = (p-base)/16` | O(1) | one line | 0 | — |
| `HeapVec::push` | `if len == cap {cold grow}; write; len+=1` | O(1) amortised | sequential | 1 | grow = next class ≥ 2×cap bytes (class ladder doubles above 256 B) |
| `VmColumn::ensure_len_zeroed(n)` | `if n > committed {cold commit}; len = max(len, n)` | O(1) | none | 1 | replaces `Vec::resize(n, ABSENT)` loops; requires the absent value to be 0 (`slot+1` encoding for `EntitySlotMap`) |
| `Heap::alloc` large | exact-size run from `large_free` LIFO (first match by `len_granules`), else frontier | O(k) over free runs, cold | — | cold | clients: `HeapVec` above 64 KiB only; coalescing deferred until a measured fragmentation number |

SIMD: none of these are vectorisable and none need to be — all are cold-or-O(1) bookkeeping; the data they hold is what gets vectorised. ⚠ *Rev 2.5 (P46): the three `Heap`/`HeapVec` rows above belong to the Heap's revival form.*

## 3. The gate ladder

| Rung | Gate | Lands | Mechanism | Red-first canary |
|---|---|---|---|---|
| **G1** | **Field ledger** (workspace) | immediately, before any migration | `tests/alloc_field_ledger.rs`, generalised from `[ecsnative] tests/physics_vec_side_store_census.rs` (`SCANNED_ROOT` → every runtime crate's `src/`, `:218`): scans struct fields typed `Vec<|Box<|String|Arc<|Rc<|VecDeque<|BTreeMap<|BTreeSet<|HashMap<|HashSet<` outside `#[cfg(test)]`; requires the set to equal `const LEDGER: &[Site { crate, file, struct, field, class, rung, tag }]` **exactly** (a missing site = red, an extra site = red — so the list can only shrink in the same commit that deletes the field); `MIN_SITES` anti-vacuity floor as in the physics test (`:277`) | (a) add a `Vec` field to production → red; (b) delete a ledgered field without its entry → red; (c) point the scanner at an empty dir → red on the floor |
| **G2** | **Frame allocation census as a gate** | with rung 1 | `crates/boyko_physics/tests/alloc_frame_census.rs` (untracked on ecsnative) becomes `tests/alloc_frame_gate.rs` at the workspace root: per scene `STEADY_MAX_S0..S3, S1a, S1b` constants; the assertion is `measured_max <= STEADY_MAX_X`; each rung lowers its constants; end state = 0 everywhere. Process-global counter (the `boyko_app/tests/zero_alloc.rs` shape, worker-visible, `:47`), not the `thread_local!` shape of `colored_solve_zero_alloc_o5.rs:281-284` | run with a deliberate `Vec::new()`+push in a system → red |
| **G3** | **`DenyAfterSteady`** | with rung 1, opt-in per gate binary | `#[global_allocator]` that delegates to `System` until `enter_steady()`; afterwards `alloc` writes a fixed 64-byte message with `WriteFile`/`write(2)` and `abort()`s. Not unwinding (required by `GlobalAlloc`); Miri arm: counts instead of aborting (the tree measured Tree-Borrows UB delegating to Windows `System` for over-aligned blocks under Miri, `[merge] block.rs:759-782` — the deny gate is `cfg(not(miri))`) | same as G2's canary, must abort |
| **G4** | **clippy `disallowed-types`** on `alloc::vec::Vec`, `alloc::boxed::Box`, `alloc::string::String`, `alloc::sync::Arc`, `alloc::collections::*` (plus the existing `HashMap` etc.) | end state, when the ledger for a crate is ≤ 10 sites (allows are then exceptions with rationale, not noise) | per crate: `#![cfg_attr(test, allow(clippy::disallowed_types))]` at the crate root covers unit tests in one line; integration tests in `tests/` get the same line each; the type ban fires on annotated locals and `collect::<Vec<_>>()` turbofish too, so production locals are covered here. The tree's L8c rejection (`[ecsnative] clippy.toml:32-63`, ~1051 sites) was of `disallowed-macros` without a crate-level test allow; this rung uses one | (a) `Vec` field in production → red; (b) `Vec` local in a `#[cfg(test)]` module → **must not** red (a canary that does not fire is a finding — verify both directions) |
| **G5** ⚠ *Rev 2.5 (P48): stands; the thread context stays out of `boyko_memory` so that it can* ⚠ *Rev 2.6 (P55, AP7 answer 1): per target — `#![no_std]` on every target; `extern crate alloc` only under `vm.rs`'s fallback `cfg(any(miri, not(any(windows, unix))))`; the compile check runs on the windows and unix targets; lands with UG-07 at F2, not C1* | **`#![no_std]` without `extern crate alloc`** | end state | for `boyko_memory` and `boyko_utils` only — structurally no `Vec`/`Box`/`String`/`format!` can be named. Not for `boyko_ecs` (needs `std::sync::OnceLock`, `std::thread` via the pool) | compile |

The counting gates remain **lower bounds** (`[D GlobalAlloc docs]`: the optimizer may elide); G1+G4 are the upper bound on written sites; G3 is the process-level proof.

## 4. Migration order — by measured per-frame cost, then Principle 0, then the ruling

Numbers for rung 1 are structural (the census has not been run: cargo beyond `check` was forbidden); the first act of rung 1 is running G2 to replace "structural" with measured counts and reorder 1a-1e by them.

### Rung 0 — prerequisite already designed: packing plan S0-S2 (`COMMIT_PAGE` quantum, page floors)
Not part of this campaign's code, but rung 2 is uneconomic without it (F4's own argument, `[ecsnative] dense_store.rs:36-45`: a 64 KiB floor per bookkeeping column × 3 per dense store). Rung 1 does **not** wait for it (Frame/Scope/Heap self-pack).

### Rung 1 — per-frame allocators (G2 goes to 0 for S0/S0b here, except 1d)
| Sub-rung | Site | Today | After | Per-frame allocations removed |
|---|---|---|---|---|
| 1a | `Box::new(ScopeShared)` per `install`/`scope` (`[merge] thread_pool.rs:278, 328`) | 1 alloc+free per `Schedule::run`, 1 per `par_iter` | emplaced in the owner thread's `ScopeArena` at `mark`, rewound at join | 1 + (#par_iter) per frame |
| 1b | `ScopeBlock` chunks via `std::alloc` (`[merge] block.rs:101, 244-290`) | ≥1 alloc+free per scope that spawns | chunks = sub-ranges of the same `ScopeArena`; the D1/D2 Tree-Borrows rules carry over verbatim (`block.rs:35-81`) | ≥1 per scope |
| 1c | `CommandQueue.bytes`, `panic_recovery` (`[ecsnative] command_queue.rs:83-85`), `ErasedKindBuffer.data` (`erased_buffer.rs:109`) | burst realloc when a frame exceeds retained capacity | `ByteColumn` (`spare_ptr`+`set_len`), `clear` = `len = 0` | burst only |
| 1d | crossbeam `Injector`/`Worker`/epoch (`[merge] worker.rs:646, 671`) | 1 block per 31 injector pushes + epoch node per sealed bag + deque doubling | **in-house bounded Chase-Lev** per lane on a `Table<Task>` ring in the pool's `TableSet` (capacity `LANE_CAP = 4096` × 16 B = 64 KiB per lane, `[merge] task/mod.rs:191-203` thin task); bounded MPMC injector ring (`W × LANE_CAP`); **overflow policy: the pushing thread executes the task inline** (`#[cold]`; helping at push, no unbounded growth). Lê et al. 2013 formulation `[D]`; loom model mandatory | all |
| 1e | `TermList` `Box::new` per (tag-terms, generation) epoch (`term_list.rs:141-170`); `traverse_iter.rs:51, 284` scratch (flagged per-call by Lens C, unverified) | conditional | `HeapDyn`/`HeapVec` (epoch-rate) / `FrameVec` (per call) | conditional |

### Rung 2 — per-entity side stores (Principle 0), after rung 0
| Site | After ⚠ *Rev 2.5 (P46.3): the `DropColumn`, `HeapVec` and `SparseMap` destinations below are re-pointed to KC-15/KC-16/KC-18* | Determinism note |
|---|---|---|
| `EntityMaster.free_entity_ids: Vec<EntityId>` (`entity_master.rs:73`) | `VmColumn<EntityId>` used as a stack; `as_mut_slice()` for the `sort_unstable_by` at `:499` | push/pop order identical → reuse order identical |
| `DenseStore.free: Vec<u32>` (`dense_store.rs:126`) | `VmColumn<u32>` stack | same |
| `EntitySlotMap.slots: Vec<u32>` (`entity_slot_map.rs:40`) | `VmColumn<u32>` with **`slot+1` encoding, 0 = absent**; `resize(id+1, ABSENT)` → `ensure_len_zeroed(id+1)` (no fill loop) | index-addressed; no order |
| `LiveBitmap.words: Vec<u64>` (`live_bitmap.rs:31`) | `VmColumn<u64>`, zero = not live, `ensure_len_zeroed` | — |
| `ArchetypeBundle.{free_slots, id_to_slot}` (`archetype_bundle.rs:135, 568, 689`) | `VmColumn<u16>` stack / `VmColumn<u32>` zero-encoded | LIFO preserved |
| `Children(Vec<Entity>)` (`hierarchy/mod.rs:120`) | `Children(HeapVec<Entity>)`; mutation only through `EcsMaster` hierarchy APIs (dispatcher-only, heap single-writer) | — |
| Observer arenas (`entity_store.rs:111-136`), `TriggerLists.by_trigger` (`trigger.rs:170-176`) | `DropColumn<EntityObserverList>` + `VmColumn<u32>` free list; `HeapVec<HeapVec<TriggerEntry>>` | register-time only |
| `Assets.free` + slot columns (`assets.rs:205, 239`), `Staging.queue` | `VmColumn` stacks | LIFO preserved |
| physics `IslandSleep.{asleep, below_count, frozen_islands, energy}` (`resources.rs:3069-3085`), `SoftBody` 30 columns (`soft/component.rs:71-177`) | `ScratchColumn` / dense components per the existing physics census rungs (`physics_vec_side_store_census.rs:249-269`) | `{1,N}` oracle re-run |
| UI frame lanes (`boyko_ui/resources.rs:216-254`, `pick.rs`, `focus.rs`, `bind_system.rs`), render `ui/pack.rs:143-150` | `FrameVec` on the `FrameArena` (clear+refill per frame is exactly the frame class); `Vec<Vec<Entity>>` pools → CSR `FrameVec<u32>` offsets + flat `FrameVec<Entity>` two-pass | dispatcher-only systems (UI runs exclusive) — verify per system in the rung |
| `boyko_utils::SparseMap`/`SparseSlotMap` (3 `Vec`s each) | `HeapVec` fields, `new_in(&mut Heap)` | — |

### Rung 3 — setup-once and build products (zero per-frame cost; on the list because of the ruling)
| Site | After ⚠ *Rev 2.5 (P46.3): the `TableSet`, `HeapVec`, `HeapDyn`, `SortedMap` and `DropColumn` destinations below are re-pointed to the ledger forms* |
|---|---|
| `Schedule.{systems: Vec<SystemBox>, system_conditions: Vec<Vec<BoolSystem>>, system_gating_sets, set_conditions, state_entries}` (`[merge] schedule.rs:116-187`), `ConflictGraph.{pred_count, successors, conflict_bits}` (`conflict_graph.rs:68-77`), `ExecutorScratch.{exclusive_to_run, to_spawn, pred_remaining}` + `Box<CompletionChannel>` (`executor_scratch.rs:219-330`) | one `TableSet` per `Schedule` ("ScheduleTables"): systems as `Table<HeapDyn<SystemVTable>>`… no — systems as `Table<ErasedSystem>` where the erased object bytes live in the heap; conditions as CSR (`Table<u32>` offsets + `Table<BoolSystem>`); successors CSR; bitsets as `Table<u64>`; scratch as `Table<SystemIndex>`; `CompletionChannel` in-table (already `NonNull`-addressed) |
| `SystemBox(Box<dyn System>)` (`system_box.rs:83`) | `ErasedSystem { data: NonNull<u8>, vtable: &'static SystemVTable }` (the `Task` pattern) |
| `ScheduleBuilder` 7 `Vec` + 5 `HashMap` + `Vec<String>` + `Box<dyn FnOnce>` (`schedule_builder.rs:92-144, 1090`) | `HeapVec`, `SortedMap<TypeId/SetId, u32>` (n ≤ hundreds, build-time), names become `&'static str` (they are `type_name` literals), `HeapDyn<FnOnceVTable>` |
| `Arc<ThreadPool>` in `Schedule`, `ScheduleBuilder`, `App` (`schedule.rs:122`, `app.rs:191`) | **removed**: `Schedule::run(&mut self, master, pool: &ThreadPool)`; `ScheduleBuilder::build(self, pool: &ThreadPool)`. `App` owns the pool by value |
| `Resources.slots`, `NonSendResources.slots`, `EventDispatcher.{slots, per_lane_overflow_count}`, `EcsMaster` caches (`resources.rs:103`, `event_dispatcher.rs:117-136`, `ecs_master.rs:127-340`), `BundleColumnCache.slots` | one `TableSet` per `EcsMaster` ("MasterTables") |
| `ComponentPoolBundle.pools: Vec<ComponentPool>`, `DenseRegistry.slots`, `ArchetypeRegistry` groups, `Archetype.component_ids` | `DropColumn<ComponentPool>` (address-stable — pools are pointed at), `DropColumn<Option<DenseStore>>`, `HeapVec` |
| `EnableStore.pages: Box<[Option<Box<EnablePage>>]>` (`enable_store.rs:75, 163-233`) | `VmColumn<EnablePage>` indexed by page (512 B pages, zero = all-disabled or all-enabled by the existing polarity — verify), `summary` as `Table<AtomicU64>` |
| ThreadPool `Arc<[..]>` tables, `Arc<PoolInner>` (`[merge] thread_pool.rs:128-135, 401`) | one `TableSet` per pool ("PoolTables"): workers, stealers, lane rings, `ScopeArena`s, `PoolInner`; workers hold `NonNull<PoolInner>` valid until `Drop` joins them (the `CompletionChannel` pattern) |
| `EventBuffer` lanes `Box<[MaybeUninit<E>]>` (`event_buffer.rs:119-243`) | `Table<MaybeUninit<E>>` in MasterTables (sizes known at preregister) |
| render/rhi persistent lists (`bindless.rs`, `retired_gpu_buffers.rs`, `gpu_column.rs:532`, FrameGraph SoA lanes) | `HeapVec` / `VmColumn` (12-24 B tuples now admitted) |

### Rung 4 — load, clone, serialize, diagnostics, strings
`prefab.rs`, `serialize/mod.rs:288`, `load_writer.rs`, `mesh_data.rs`, `texture_data.rs`, UI text AST/lower/report, `boyko_app` titles/profiling, `boyko_log` `String`s, asset `error.rs` → `HeapVec`, `HeapString` with `core::fmt::Write`, `InlineStr<N>` for titles, error types carry codes (`boyko_log/src/codes.rs` already exists) + `&'static str`. Tag `diag` sites are the last to go; G4 lands per crate as each drops under 10.

## 5. Public-signature changes, determinism proofs, Miri/loom

### 5.1 Signature changes and their consumers
| Change | Consumers |
|---|---|
| `Schedule::run(&mut self, &mut EcsMaster, &ThreadPool)`; `ScheduleBuilder::build(self, &ThreadPool)` | `boyko_app` runner/loop, every schedule test and bench, `bench_bevy_vs_boyko`, `boyko_demo` |
| `App` owns `ThreadPool` by value; `App::pool() -> &ThreadPool` | plugins that cloned the `Arc` |
| `Children(HeapVec<Entity>)`; `Children::iter()/as_slice()` unchanged; push/remove only via `EcsMaster::{set_parent, …}` | `boyko_scene`, `boyko_ui` hierarchy users; any code that did `children.0.push` |
| `EcsMaster` gains `frame: FrameArena`, `heap: Heap`, `tables: TableSet`; `EcsMaster::frame() -> &mut FrameArena` for exclusive systems | UI/render systems migrating frame lists |
| `VmColumn::new` accepts any non-ZST element size | none breaking |
| `boyko_memory` crate; `boyko_ecs::ecs::memory::{VmReservation, VmColumn}` re-exported at their old paths for one rung, then removed | internal |
| System names `&'static str` only in the builder | plugins passing `String` names (none found; verify in rung 3) |
| `boyko_utils::SparseMap::new_in(&mut Heap)` | archetype registry, any external user |
| Threadpool: `ThreadPool::new(workers)` unchanged; `Scope` API unchanged; lane capacity `LANE_CAP` becomes a build parameter with a default | none |

### 5.2 Determinism proofs owed per rung
| Rung | Obligation | Proof ⚠ *Rev 2.5 (P46): the `Heap` rows (reuse order here; P8/O8's capacity classes) belong to the Heap's revival form* |
|---|---|---|
| 1a/1b | `ScopeArena` addresses depend on which worker ran the spawner | **no computation reads a task cell's address** — cells are executed and discarded. Canary: `cfg(feature = "mem-shake")` offsets each per-worker arena base by `wid × 4096 × prime` so any address dependence changes results; the `{1,N}` physics oracle (`[ecsnative] plugin.rs:386`) must stay green with and without it |
| 1d | task execution order under the bounded deque + inline-overflow | the oracle is index-addressed and order-independent by construction (colored solve); loom model of push/pop/steal + a test that forces overflow (`LANE_CAP = 2` under test) with the oracle |
| 2 | LIFO stacks | column stack `push`/`pop` = identical sequence; existing entity-id reuse tests + a new property test: interleaved spawn/despawn sequence yields the same id sequence on `Vec` (test oracle) and `VmColumn` |
| 2 | `slot+1` encoding | property test: `EntitySlotMap` old vs new over random insert/remove sequences |
| 2, 3 | Heap reuse order | single writer, program order → same across thread counts; addresses differ run-to-run only by ASLR, as today |
| 2 | Frame arena | dispatcher-only; `debug_assert!(owner == current)` under `cfg(debug_assertions)` |

### 5.3 Miri / loom under Tree Borrows
- **Provenance granularity is the reservation.** Miri cannot see a use-after-free or overlap *inside* a `Heap`/arena `[D std::ptr; D arXiv 2206.11728]`. Compensation: under `cfg(any(miri, feature = "mem-sanitize"))` the Heap keeps a live-bit column (1 bit per 16-B unit), `free` asserts the bit is set and clears it, `alloc` asserts clear and sets it, freed chunks are filled with `0xDD` beyond the link word; `FrameArena::rewind`/`reset` and `ScopeArena::rewind` fill the released range with `0xDD`. This is `FLECS_SANITIZE` `[S flecs.h]`, and it is the only use-after-free detector the arenas will ever have. ⚠ *Rev 2.5 (P46): every `Heap` clause of §5.3, as amended by P3, P11 and P15/15.5, belongs to the Heap's revival form; the chunk, column and scope clauses stand.*
- **Tree Borrows rules inherited from `ScopeBlock` (`[merge] block.rs:35-81`) become library-wide invariants**: TB-1 no bookkeeping word inside a *live* chunk (the intrusive link exists only while the chunk is on a free list, and is read/written through raw pointers derived from the reservation base, never through a reference to the payload type); TB-2 allocate-and-initialise (`alloc` returns `NonNull`, the caller writes before minting any reference); TB-3 no `Deref` from an arena handle that outlives the frame/scope (`FrameVec<'f, T>` borrows the arena; `Scope` cells are consumed at execution).
- **Miri reserve sizes**: every class's reservation has a Miri arm constant (Heap small 8 MiB, large 8 MiB, Frame 4 MiB, Scope 1 MiB/slot) because the fallback arm eagerly `alloc_zeroed`s the full `os_len` (`[ecsnative] vm.rs:168-179`); exhaustion under Miri is a loud panic, as `reserve` already is.
- **`-Zmiri-ignore-leaks`**: a `HeapVec` never freed before its `Heap` drops is not a leak to Miri (the reservation is released). The sanitize live-bit column reports it instead: `Heap::drop` asserts zero live bits under `mem-sanitize`.
- **loom**: `Heap`, `FrameArena`, `ScopeArena` have no atomics — nothing to model. The in-house deque (1d) uses the threadpool's existing `cfg(loom)` atomic shim (`[merge] sync.rs:33-34`); mandatory loom tests: push/steal race, steal/steal race, overflow-inline path.
- **Drop order becomes a soundness invariant** (HV-1): `EcsMaster` fields are ordered so pools and every `HeapVec`/`HeapDyn` owner drop before `heap`, and `heap` before nothing (it owns its reservations); the `PoolBacking`-last precedent (`[ecsnative] component_pool.rs:81-84`) is the pattern. `debug_assert!` in `Heap::drop` under sanitize.
- **`unsafe` census**: every `HeapDyn::new`, `Table<T>` handout, `ByteColumn::set_len`, and free-list link access is an `unsafe` site with a `// SAFETY:` naming TB-1..3 and the ownership rule; the SAFETY comment must argue the **protector lifetime**, not the last access (the tree's recorded failure class).

## 6. VALUES/SCOPE questions for the owner only

1. **Third-party coverage.** Defaults chosen here: crossbeam-deque replaced (rung 1d, per-frame); `std::thread::spawn` accepted at boot (`os-thread`); Vulkan driver host allocations accepted (`ffi-driver`). Confirm or widen — widening `ffi-driver` means a thread-safe heap for a foreign caller, which is the one thing this design deliberately does not build.
2. **`diag` sites.** Default: they migrate last (rung 4) and are outside the measured window until then. Alternative is to declare them permanently accepted; the design assumes not.
3. **Rung order between 2 and 3.** Rung 3 has zero per-frame cost and exists because of ruling 3; rung 2 is Principle 0 and needs the packing plan (rung 0) first. If rung 0 slips, rung 3 can go ahead of rung 2 without any design change — say which.
4. **Resident-memory budget.** Frame/Scope floors are high-water marks with no trim (Dawson's per-MB costs). If a resident ceiling matters for a target (editor with many worlds), a trim policy becomes a measured rung; none is planned.

## Implementation plan (developer rungs, in order)

1. `boyko_memory` crate: move `vm.rs`, `vm_column.rs`, `utils.rs`, constants; re-export from the old paths; relax the `VmColumn` size pin; add `ensure_len_zeroed`, `pop`, `ByteColumn::{spare_ptr, set_len}`. `cargo check --workspace --all-targets` green.
2. G1 ledger test at the workspace root with the full roster (every runtime crate), red-first canaries pasted into its header as the physics census does.
3. `FrameArena`, `FrameVec`, `ScopeArena`, `Heap`, `HeapVec`, `HeapBox`, `HeapDyn`, `HeapString`, `SortedMap`, `DropColumn`, `TableSet` in `boyko_memory`, each with unit tests, the sanitize arm, and Miri runs.
4. G2 gate (frame census → thresholds) and G3 deny allocator; **run G2 first** to replace structural counts with measured ones and fix the 1a-1e order.
5. Rung 1a-1c (threadpool scope arena, byte columns); re-run G2; lower constants.
6. Rung 1d: in-house bounded deque + injector; loom tests; `{1,N}` oracle with `mem-shake`.
7. Rung 2 (after packing plan S0-S2); property tests for LIFO and `slot+1`.
8. Rung 3 (`TableSet`s for Schedule/Master/Pool; `ErasedSystem`; `Arc` removal; signature changes). ⚠ *Rev 2.5 (P46.3): KC-18 tables and KC-07's inline arrays, not `TableSet`s; `ErasedSystem` (KC-17) and the `Arc` removal (KC-07) stand.*
9. Rung 4; G4 per crate as each drops below 10 sites; G5 for `boyko_memory`/`boyko_utils`.

## Metrics and validation

- **Benchmarks**: `Schedule::run` floor for 0/1/2/4/8/16 systems (S0) before/after 1a-1b — expected delta = two allocator calls per run, i.e. tens of ns; the point is the count, not the time. `par_iter` over 1M entities in 4096-row chunks: scope arena vs `std::alloc` chunks. `Heap::alloc/free` 16-B and 256-B classes vs `System` (expect ≤ 5 ns; mimalloc's median < 10 ns `[B Forrest Smith]` is the bar). `HeapVec::push` vs `Vec::push` at 1k/1M elements (must be within noise — same algorithm). `EntitySlotMap` growth to 1M ids: `ensure_len_zeroed` vs `resize` (the fill loop is the measured difference).
- **Mandatory unit tests**: each primitive's growth across a commit boundary; `DropColumn` destructor count on truncate/swap_remove/drop; `Heap` class boundaries (16, 256, 257, 512, 64 KiB, 64 KiB+1); free-list LIFO order; `FrameArena` mark/rewind LIFO `debug_assert`; `TableSet` alignment of every table; sanitize double-free detection (red-first).
- **Property tests**: LIFO stack equivalence with `Vec`; `EntitySlotMap` `slot+1` equivalence; `HeapVec` vs `Vec` operation sequences; bounded deque vs a sequential model (loom + proptest).
- **`debug_assert!` invariants**: owner thread on `Heap`/`FrameArena` mutation; `len <= committed`; LIFO mark order; class ↔ layout on `free`; `TableSet` bounds; HV-1 at `Heap::drop` (sanitize).

## Open questions (for the critic)

1. `FrameVec<T: Copy>` in v1 — the inventory found no droppable per-frame data; if the critic finds one, the answer is `DropColumn` on the Heap, not a finalizer chain.
2. The Heap's large tier without coalescing: acceptable until a measured fragmentation number, or must v1 coalesce? My position: defer — clients are `HeapVec` above 64 KiB, of which the inventory names none after rung 2 moves the entity-scaled maps to columns.
3. Inline execution on lane overflow (1d) changes *when* a task runs relative to its spawner; any consumer relying on spawn-then-continue ordering (none known — scopes join) would be affected. To be confirmed by reading `scope.rs` spawn semantics in the rung.
4. `EnableStore` page polarity (zero = which state) decides whether `VmColumn<EnablePage>` gets its pages for free; unverified here.

---

# Part II - Critique log - pass 1 (2026-09-10)

**Verdict of pass 1: CHANGES REQUESTED** - two blockers (C1, C2), five important remarks (W1-W5), nine optional/provenance notes (O1-O9), six open questions (Q1-Q6). The central fork of §1 (per-type replacement by lifetime class; `allocator_api` rejected; `#[global_allocator]` kept only as the deny gate) was walked against the critic's eight-item bar and was **not contested**.

Rules of this log:

- Part I above is **unchanged**. No finding is answered by silently editing the design; every answer lands in rev 2 and is traceable to a row here.
- **Action** vocabulary: **FIX IN REV 2** - the rev-2 design text must change (and say how the finding was closed); **REFUTE (why)** - the finding is rejected, with the reason; **ACCEPT** - the finding is correct and adopted as a rung-time obligation or a preserved property, with no design-text rework beyond recording it.
- Actions were assigned by the Write phase on the evidence below; the architect may overturn one in rev 2 only by stating the counter-evidence in the rev-2 change log.
- Evidence is the critic's, with tree and line. Where the Write phase re-checked a citation itself, the row says **[verified by Write phase]** and names what was read.

## Positive findings - preserve in rev 2

| ID | What the critic confirmed | Evidence | Action |
|---|---|---|---|
| P1 | Bar 1 (address stability) answered correctly: every cached-pointer site stays on `VmColumn`/`ComponentPool`/`InlandStore`; `HeapVec` confined by HV-2 | `DenseQueryIter`'s `s2e` already a `VmColumn` [ecsnative `dense_store.rs:98-104`] | ACCEPT (preserve) |
| P2 | `ensure_len_zeroed` + `slot+1`/zero encoding for `EntitySlotMap`/`LiveBitmap` turns growth into a commit with no fill loop - a win `Vec<T, A>` cannot have | design §2.4, §2.5, rung 2 | ACCEPT (preserve) |
| P3 | LIFO stacks on `VmColumn` preserve reuse order bit-for-bit; `as_mut_slice` already exists for the `sort_unstable_by` | [ecsnative `entity_master.rs:499`], [ecsnative `vm_column.rs:368`] | ACCEPT (preserve) |
| P4 | Bar 8: the Heap is a segregated size-class free list on a reservation, not the X.J best-fit BTreeMap tracker, and has ~88 clients - not a re-creation of the retired thing | X.J retirement [ecsnative git `1ee2c461`] | ACCEPT (preserve) |
| P5 | Closed exception vocabulary and deny-gate-not-allocator reading of ruling 3 are the right shape; opt-in deny gate avoids the one-`#[global_allocator]`-per-binary conflict | [ecsnative `boyko_app/src/profiling/alloc_shim.rs:139-144`] | ACCEPT (preserve) |
| P6 | Miri sanitize arm (live-bit column, `0xDD` fill) is the right compensation for reservation-granular provenance | design §5.3 | ACCEPT (preserve) |
| P7 | Rejecting decommit on Dawson's numbers and a periodic trim as "policy without a measured need" is correct discipline | design §2.2 | ACCEPT (preserve) |
| P8 | Open question 4 of the design (EnablePage polarity) is answered: an absent page reads false, so zero-filled pages are free - but see W3 for the `Copy` bound | [ecsnative `enable_store.rs:203-212`] | ACCEPT (record the answer in rev 2; W3 carries the type fix) |

## Blocking findings

| ID | Finding | Evidence | Confidence | Action |
|---|---|---|---|---|
| C1 | **Heap back-pointer aliasing is UB under Tree Borrows as designed.** `HeapVec { heap: NonNull<Heap> }` (and `HeapBox`/`HeapDyn`/`HeapString`/`SortedMap`) is minted from `&mut Heap` in `new_in`; every later `EcsMaster` method reborrows `&mut self.heap` and writes `free[c]`/`small_frontier` through a sibling tag, which disables the older tag; the next `HeapVec::drop` write through it is "write access through <TAG> is forbidden". First despawn of an entity carrying `Children(HeapVec<Entity>)` reds every Miri run of the hierarchy/observer/trigger tests; natively it is silent UB in the allocator owning ~88 kernel fields. | Design §2.3 (`HeapVec.heap`), §2.4 (`Heap::alloc(&mut self)`, `Heap::free(&mut self)`), §5.1 (`EcsMaster` gains `heap: Heap` as a plain field). The tree's own precedents for exactly this rule: `block.rs` D1 keeps bookkeeping out of memory the owner's `&mut` protector could reach [merge `crates/boyko_threadpool/src/block.rs:35-42`]; `CompletionChannel` lives in its own allocation behind a bare `NonNull` so it is never reached through a reborrow under `&mut self` [merge `crates/boyko_ecs/src/ecs/core/schedule/executor_scratch.rs:221-233`]; the TB rule measured red [merge `scope.rs:203-214`]. | CONFIRMED | **FIX IN REV 2** - choose one shape and state it for all five handle types: (a) `&self` API with `UnsafeCell` bookkeeping under the single-writer (EM2) argument; (b) bookkeeping inside the reservation, handles pointing at the reservation base (the `ComponentPool`/`InlandStore` shape); or (c) explicit-heap API (`free(self, &mut Heap)`, no `Drop`, leaks caught by the sanitize live-bit column). Must carry a Miri red-first test: a `HeapVec` dropped after an intervening `Heap::alloc` through a fresh `&mut`. |
| C2 | **`ScopeArena` slots keyed by `wid` collide.** `WORKER_ID_DISPATCHER` is a ROLE shared by every installing thread, not a thread identity. Two installing threads (main in `Schedule::run` plus any other installer, or two Apps sharing one pool in a test) bump the same `ScopeArena.cur` unsynchronised: two `ScopeShared`s and two scopes' task cells overlap, `pending` counters alias - memory corruption at W >= 1, and the unwinding `InstallGuard` path has no slot to release. | Design §2.1 Scope row ("one per thread slot, W+1 ... indexed by wid"), rung 1a. `install` rewrites TLS wid to `WORKER_ID_DISPATCHER` on any thread [merge `crates/boyko_threadpool/src/tls.rs:136-137`; `thread_pool.rs:254`]; an install on a same-pool worker is labelled the same [merge `thread_pool.rs:274-277`]; concurrent install/scope frames are supported - `active_scopes` is a counter [merge `thread_pool.rs:148-150`]; tests install from spawned threads [merge `tls.rs:648`; `scope.rs:1811`]; production installer outside the dispatcher: fontbake [merge `crates/boyko_fontbake/src/msdf/distance.rs:433`]; `Schedule::run` install [merge `schedule.rs:455`]. | CONFIRMED | **FIX IN REV 2** - the arena must be owned by a THREAD identity: either a TLS-owned per-thread arena (the `LANE_DEPOSIT` single-read shape) with its lifetime relative to the pool stated, or a claim/release pool of dispatcher arenas (the `boyko_diag::lane::claim_lane` spare mechanism [merge `tls.rs:463-464`]) claimed at `install` and released by `InstallGuard` on return and unwind. Nested LIFO on one thread (an exclusive system calling `pool.scope` [merge `crates/boyko_physics/src/resources.rs:1780, 1872`]) must stay correct. Note: fontbake is tagged `offline` in §0, which removes it from the steady-state window but not from the correctness argument - the collision is a soundness defect whenever two installers share a pool. |

## Important findings (non-blocking)

| ID | Finding | Evidence | Confidence | Action |
|---|---|---|---|---|
| W1 | The KE16 completion-protector Miri gate keys on a DEALLOCATION that rung 1a removes: today it reds on the `complete_task(&self)` regression as "deallocation through <TAG> is forbidden ... at `Scope::drop`'s `Box::from_raw`"; after 1a the `ScopeShared` is rewound, not freed, so Tree Borrows judges nothing and the gate goes silently green (the "gate that could not fail" class). The sanitize `0xDD` fill can replace it only if it runs under `cfg(miri)` inside the release window and both `miri_note_*` sites are re-homed to the rewind. | [merge `scope.rs:203-214, 1351-1406`]; `miri_note_*` sites [merge `scope.rs:1308-1311, 1403-1404`]; RED-first precedent [merge `scope.rs:207-214`] | CONFIRMED | **FIX IN REV 2** - §5.3 states the replacement mechanism; rung 1a must re-establish RED-first with the `&Self` receiver regression before it lands. |
| W2 | `FrameArena::vec(&mut self, cap) -> FrameVec<'_, T>` ties each `FrameVec` to an exclusive borrow, so only one can be live per arena; rung 2's CSR two-pass (`FrameVec<u32>` offsets + `FrameVec<Entity>` flat) and any UI system holding two lists do not compile. | Design §2.4; `ScopeBlock` already solved this with `&self` + `Cell` cursors [merge `block.rs:83-89, 146-153`] | CONFIRMED (from the API text) | **FIX IN REV 2** - make the Frame class `&self` + interior mutability (single-thread by construction), or detach `FrameVec` from the borrow and guard `reset()` with a debug generation counter. |
| W3 | `VmColumn<EnablePage>` cannot type-check: `EnablePage` is `[AtomicU64; 64]` and `VmColumn` requires `T: Copy`; `ensure_len_zeroed` is also defined on `T: Copy`. The wanted zero-read semantics need a `Zeroable`-style bound (valid-when-all-zero, no `Drop`), not `Copy`; `DropColumn` is not it. | [ecsnative `enable_store.rs:60`] (`EnablePage`), [ecsnative `vm_column.rs:80`] (`T: Copy`), polarity [ecsnative `enable_store.rs:203-212`] | CONFIRMED | **FIX IN REV 2** - state which column flavour gets the relaxed bound and its exact contract. |
| W4 | G1 "landable immediately" rests on the physics census scanner, whose blind spots were measured empty for one crate only: tuple enum variants unread, type aliases invisible, `#[cfg(not(test))]` read as a test region, hand-wrapped generics read to the comma, text match on `Vec<` only. Generalising to 19 crates and to the full type list turns each "zero today" into an unmeasured claim in the false-green direction (`Box<dyn ..>`, `Box<[..]>`, enum payloads are exactly the shapes physics lacks). | [ecsnative `tests/physics_vec_side_store_census.rs:54-57`] (enum variants), `:45-48` (aliases), `:69-76` (`cfg(not(test))`), `:77-83` (generics), `:45, :52-53` (text `Vec<` only); two-sided exactness `:85-92`, tests at `:887`, `:957` | PLAUSIBLE on the count, CONFIRMED on the holes | **FIX IN REV 2** - G1's rung re-measures every listed hole over the full roster before the ledger is trusted, extends the matcher to the full type list, and proves the only-shrinks property two-sidedly. |
| W5 | G3 `DenyAfterSteady` must not abort while a panic is in progress: `panic!`/`assert!` formatting allocates, so any genuine assertion failure in the steady window aborts with the deny message before the real message exists, and every failing gate reads as "allocation after steady". The abort message must also name the thread role (G2 is process-global). | Design §3 G3; `GlobalAlloc` no-unwind contract [D std `GlobalAlloc` docs] | CONFIRMED | **FIX IN REV 2** - exempt `std::thread::panicking()` (a TLS read, no allocation) inside `alloc`; keep the count-instead-of-abort Miri arm; message names worker/dispatcher role. |

## Optional and provenance notes

| ID | Finding | Evidence | Action |
|---|---|---|---|
| O1 | Heap class formula underflows at size 0: `(0+15)>>4 - 1 = -1`. `HeapDyn`/`HeapBox` of a capture-less closure (the `Box<dyn FnOnce>` replacements) are ZSTs. | ZST handling precedent [merge `block.rs:253-261`]; builder site [merge `schedule_builder.rs:1090`] | **FIX IN REV 2** - special-case ZST to a dangling pointer, as `ScopeBlock::emplace` does. |
| O2 | `ScopeArena` padding arithmetic: 4 x 8 B + `Option<VmReservation>` (16 B, NonNull niche) = 48 B; `_pad: [u8; 24]` makes 72 B, which `align(64)` rounds to 128 B - two lines per slot. | Design §2.3 `ScopeArena` | **FIX IN REV 2** - drop `_pad` (`align(64)` already pads to 64) or size it to 16. |
| O3 | Miri reserve sizes: Heap `small`/`large` reservations are eager (not `Option`) at 8 MiB + 8 MiB per `EcsMaster` on the fallback arm, which `alloc_zeroed`s the full length; today's worst per-pool eager footprint is 6 MiB. Suites that build many masters under Miri multiply RSS. | [ecsnative `vm.rs:168-179`]; [ecsnative `constants.rs:61-67`] | **FIX IN REV 2** - make the Heap lazy like Frame/Scope (0 until first alloc); measure Miri RSS of the existing suite before fixing the 8 MiB constants. |
| O4 | G4 canaries: `disallowed_types` walks `hir::Ty`, so `Vec::new()` and turbofish `collect::<Vec<_>>()` fire, but `vec![]` (macro expansion) and an inferred-type `.collect()` may not. | L8c precedent [ecsnative `clippy.toml:32-63`] | **FIX IN REV 2** - G4's red-first set includes `vec![]`, inferred `collect()`, `format!`, `.to_string()`, `Box::new` in expansion, and records which the lint sees. |
| O5 | Rung 1d inline overflow also applies to `Scope::spawn` from a WORKER whose lane ring is full; running a task body inside `spawn` must not span the worker's own deque `&Worker` protector (D5). | [merge `worker.rs:663-666`] | **ACCEPT** - rung-1d obligation: the inline path releases the lane borrow before running the body; verified by reading `worker.rs` in the rung and by a loom/Miri test of the overflow path. Rev 2 records it under 1d. |
| O6 | Crate split: `VmReservation`, `commit`, `base` are `pub(crate)` today; `ComponentPool` staying in `boyko_ecs` forces them `pub`, making reserve/commit public API that plugins could use as a side allocator. | [ecsnative `vm.rs:85, 184, 199`] | **FIX IN REV 2** - mark them `#[doc(hidden)]` or seal them; state which. |
| O7 | Bar 4 (Drop/unwind) half answered: the `CommandQueue` `panic_recovery` tail re-absorption inside `catch_unwind` is preserved by `ByteColumn` only if the re-absorb appends without reallocating. | [ecsnative `command_queue.rs:64-68, 385-393`]; panic inside a scope is answered [merge `scope.rs:626-639`] | **FIX IN REV 2** - state that `panic_recovery` is a second `ByteColumn` and the re-absorb is `spare_ptr` + memcpy. |
| O8 | Determinism table should add a row: `HeapVec` capacity classes change WHEN `Children` reallocates versus `Vec`'s doubling; no order dependence, but any test pinning `Vec::capacity()` growth must be re-baselined rather than silently green. | [ecsnative `entity_master.rs:477`] (`capacity() * size_of`) | **FIX IN REV 2** - add the row to §5.2. |
| O9 | §1 says `disallowed-types` cannot tell `Vec<T>` from `Vec<T, A>` - correct; the end-state G4 ban must therefore list `alloc::vec::Vec` (not only `std::vec::Vec`); verify the path form fires with a canary. | Design §1 table, §3 G4 | **REFUTE (first half) / ACCEPT (second half)** - Part I §3 G4 already lists `alloc::vec::Vec`, `alloc::boxed::Box`, `alloc::string::String`, `alloc::sync::Arc`, `alloc::collections::*`, so the path-form request is already met **[verified by Write phase against Part I §3 G4]**. The canary obligation (prove the `alloc::` path form fires on a `Vec` written through the prelude) is accepted and folds into O4's red-first set. |

## Open questions from the critic

| ID | Question | Evidence | Action |
|---|---|---|---|
| Q1 | The census measured `n + 4` acquisitions per `Schedule::run` with `n` concurrent systems; rung 1 attributes one of the four (`Box<ScopeShared>`) and the `n` per-task cells. What are the other three, and does "G2 goes to 0 for S0/S0b at rung 1" survive their attribution? | [ecsnative `crates/boyko_physics/tests/alloc_frame_census.rs:58-63`] - **[verified by Write phase]**: lines 59-63 read "acquires exactly `n + 4`; with `n = 0` it acquires **0**, because the run returns at its `systems.is_empty()` guard before `pool.install`", and S0b measured `2 * (4 + 4) = 16`, "per `Schedule::run`, not per frame". | **FIX IN REV 2** - attribute all four constants and restate rung 1's G2 target per scene accordingly. |
| Q2 | Rung 1d inline overflow: the dispatcher spawns from inside `try_dispatch_ready`; executing a concurrent SYSTEM inline on the dispatcher inside that loop - do SCH invariants (apply window, dispatcher-owned `ExecutorScratch` fields) tolerate re-entrancy there? | [merge `schedule.rs:455`] (install); [merge `executor_scratch.rs:216-219`] | **FIX IN REV 2** - answer from a reading of `try_dispatch_ready`; if re-entrancy is not tolerated, the dispatcher's overflow policy differs from the worker's and rev 2 must say how. |
| Q3 | `TableSet` for `EventBuffer` lanes: lanes are sized at `preregister_event`, which is incremental before `App::finish()`; what is the build point of MasterTables, and what happens to an event registered after it? Same question for `EnableColumn::ensure_directory` regrow versus a fixed-length `Table`. | [ecsnative `event_buffer.rs:119-127`]; [ecsnative `enable_store.rs:218-233`] | **FIX IN REV 2** - name the build point and the late-registration path (or move those two sites off `Table` to a growable column). |
| Q4 | Bar 2: per-instance floors are given but no total. State reservations per `EcsMaster` / per pool / per `Schedule`, and the boot resident total on `boyko_demo` before/after. | design §2.1 | **FIX IN REV 2** - add the accounting table (numbers are small by construction; the accounting is what the brief asks for). Measurement of the boyko_demo resident total is a rung-time action, not a design-time one. |
| Q5 | `HeapDyn` is described as "16 B thin (the Task pattern)" but carries `layout: Layout` and a `heap` pointer - 40 B. | design §2.3 `HeapDyn` | **FIX IN REV 2** - move `layout` into the vtable (and resolve the `heap` pointer under C1), or drop the "16 B thin" claim. |
| Q6 | Crossbeam `Injector` block: the design says 31 slots [S deque.rs]; the tree's census measured `1520 B = 8 + 63 x 24`, i.e. `BLOCK_CAP = 63`. The [S] tag was not read against the linked version. | [ecsnative `alloc_frame_census.rs:85-89`] - **[verified by Write phase]**: lines 84-89 read "exactly 1520 bytes, once per 64 spawned tasks ... `1520 = 8 + 63 * 24` - a `crossbeam_deque::Injector` block (a `next` pointer plus `BLOCK_CAP = 63` slots ...)". Note the "31-slot" figure also appears in Lens B of [ALLOCATOR-RESEARCH.md](ALLOCATOR-RESEARCH.md) (TL;DR and the Mechanism (5) table); the research file is verbatim and is not corrected - this row is the correction of record. | **FIX IN REV 2** - correct to 63 slots / one block per 64 pushes (per the linked `crossbeam-deque 0.8.8`), and re-tag the provenance: the in-tree measurement is the source, the upstream `master` read was not of the linked version. |

## Summary of dispositions

| Action | Rows |
|---|---|
| FIX IN REV 2 | C1, C2, W1, W2, W3, W4, W5, O1, O2, O3, O4, O6, O7, O8, Q1, Q2, Q3, Q4, Q5, Q6 |
| ACCEPT | P1-P8 (preserve), O5 (rung-1d obligation), O9 second half (canary folds into O4) |
| REFUTE | O9 first half (`alloc::vec::Vec` already listed in Part I §3 G4) |

Rev 2 is not complete until C1 and C2 are closed with a stated shape and a red-first Miri test each; the critic's pass 2 scope is the delta against this log.

---

# Rev 2 (2026-09-16)

**Tree note.** graphify **is** installed and working on this machine (`graphify 0.9.55`; the project-memory record saying it is absent was measured 2026-09-07 and is stale — corrected by the writer, who ran one query against `graphify-out/graph.json`, 35,064 nodes, before falling back to Grep/Read). All code citations below were read in `D:/wt/joltab` (`merge/ke16-into-ecsnative`, working copy, HEAD `d552be05`, which has `d11962a9` as an ancestor — verified with `git merge-base --is-ancestor`; read-only) unless tagged `[main]` = `D:/claude/BoykoEngine`, `[ecsnative]` = `D:/wt/ecsnative`, or `[merge]` = `D:/wt/merge`.

⚠ **The runtime data ledger this rev 2 was derived from is rev 3 (2350 active rows, 673 in ECS data forms, 0 class-U). The copy now on disk at `[main] docs/memory/RUNTIME-DATA-LEDGER.md` is REV 4** — dated 2026-09-16, **2357 active rows, 690 (29.3 %) in an ECS data form, 0 class-U**, built over the joltab delta `d11962a9..d552be05`. Rev 4 landed while this patch was being written, so the header's "rev 4 is in flight and is not relied on" is no longer true of the file a reader will open. **Verified by the writer: the four per-group counts P6 quotes (ecs-storage 109, ecs-schedule 166, app-demo 256, codec-tools 527) are UNCHANGED in rev 4**, so P6's anti-vacuity floors stand as written; the totals in this paragraph are the only rev-3 numbers rev 2 carries, and a rung that trusts them re-reads the ledger first.

---

# Allocator design — rev 2, as a patch against rev 1

Target file: this file. Rev 1 (Part I) stays verbatim; this patch is **Part III — Design, rev 2 (delta)**, and each change below names the Part I text it replaces.

**Scope of rev 2:** close C1 and C2 with a stated shape and a red-first Miri test each; discharge every `FIX IN REV 2` row (W1-W5, O1-O4, O6-O8, Q1-Q6); retire O5 with a reason; re-derive every rev-1 statement the KE16 Stage 3b facts falsified; add the modding seam. The central fork of §1 (per-type replacement by lifetime class; `allocator_api` rejected; `#[global_allocator]` kept only as the deny gate) was not contested in pass 1 and is **not re-opened** — one clause of its *justification* is corrected in P0 because a measurement forces it.

---

## P0 — §1 "Why (i)", third bullet: the speed argument is withdrawn and replaced

**Depends on:** §0 (target), §3 (gate ladder), §4 (migration order).

**Removed** (§1 "Why (i)", bullet 3, verbatim):

> - **Berger/Zorn/McKinley `[D OOPSLA 2002]`**: general-purpose custom allocators beat a good malloc in 2 of 8 programs, and the two winners were *regions*. (iii) spends the effort to be at best equal to mimalloc; (i) spends it on the shape that wins — regions (frame, scope) and typed columns.

**Added:**

> - **Berger/Zorn/McKinley `[D OOPSLA 2002]`**: general-purpose custom allocators beat a good malloc in 2 of 8 programs, and the two winners were *regions*. (iii) spends the effort to be at best equal to mimalloc; (i) spends it on the shape that wins — regions (frame, scope) and typed columns.
> - **Throughput is NOT the justification for this campaign, and rev 2 says so where rev 1 implied otherwise.** MEASURED (`[main] docs/unification/checkpoint-2026-09-11/allocator-ab.md`, 4 clean passes, 1240-body pile): mimalloc/system = 0.998 / 1.023 / 0.992 at W = 1 / 8 / 16, band ±5 % at W8, sign-flipping when the arm order is reversed (1.005). The adjudication prices the remaining 332 same-thread acquisitions per step at **0.3-0.6 % of an 11.15 ms step, below resolution**. Three justifications survive the null, and they are the ones every rung is argued from here on:
>   1. **The ruling** (owner, 2026-09-10): one allocator, ours. Not negotiable, not a perf claim.
>   2. **Tail, not mean.** `[B Forrest Smith 2022]` measures a shipped game's allocator at < 10 ns median, ~25 ns p95, **1-50 µs at the worst 0.1 % and ~500 µs absolute worst**. A frame that makes 2 acquisitions can eat a 500 µs stall; a frame that makes 0 cannot. The census gate counts, it does not time, so this is an argument for the *count*, which is what the campaign removes.
>   3. **G3 is unreachable otherwise.** `DenyAfterSteady` (§3) can only abort on a process-heap call if the steady frame makes none. Every rung is a precondition for the one structural proof the brief asks for.
> - **Overturn gate for the whole ordering:** the per-stage pyramid timing (`wf_cc3f9944-c7a`, owed on the owner's quiet word; the lane is `D:/wt/stagetime`, `timing/per-stage-pyramid`). If it attributes **> 1 % of the W8 step** to allocator calls, rung 1 moves ahead of everything; if it attributes < 0.3 %, rung 1 keeps its place and is justified by 1-3 above and by nothing else.

---

## P1 (closes C1) — §2.1 Heap row, §2.3 Heap/HeapVec/HeapBox/HeapDyn, §2.4 Heap API: bookkeeping moves INSIDE the reservation

**Depends on:** §5.3 TB-1..3 (as amended by P11), §5.1 drop order, the `row_ptr` provenance argument at `crates/boyko_ecs/src/ecs/memory/component_pool.rs:817-829`, and `ScopeBlock`'s D1 at `crates/boyko_threadpool/src/block.rs:35-42`. ⚠ *Rev 2.5 (P46): P1 is the Heap's revival form, not built (U-1); its `DynVTable` shape survives as KC-17's record vtable.*

**The shape, chosen from the critic's three:** (b) — *bookkeeping inside the reservation, handles pointing at the reservation base*. (a) `&self` + `UnsafeCell` is rejected because it does not close the hole: the disabling retag is `&mut EcsMaster`, not `&mut Heap`, so an `UnsafeCell` inside the `Heap` struct would have to be argued from TB's byte-precise interior-mutability rule (`ReservedIM` tolerates foreign writes, `[B ralfj 2023]`, `[S miri tree_borrows/perms.rs]`), a rule that is **model-version-dependent and opt-in** — Miri's default is still Stacked Borrows (`[S miri README]`). (c) explicit-heap `free(self, &mut Heap)` is rejected because `Children(HeapVec<Entity>)` is dropped by `ComponentPool`'s type-erased `drop_fn(*mut u8)` (`crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs:83, 117`), which has no `&mut Heap` to pass and cannot get one without a thread-local or a global — both forbidden. (b) needs no aliasing-model subtlety at all: it is the argument the tree already ships twice.

**Removed** (§2.1, the Heap row of the lifetime-class table, verbatim):

> | **Heap** | `Heap` + `HeapVec<T>`, `HeapBox<T>`, `HeapDyn<V>`, `HeapString`, `SortedMap<K,V>` | persistent, individually freed | **single writer** (structural mutation = dispatcher-only, EM2 `[ecsnative] entity_master.rs:16`); readers anywhere | size-class intrusive LIFO free lists | **no** (reused) | none | **no** (relocates on grow) | 64 KiB for the whole heap (self-packed at 4 KiB pages) — independent of the packing plan |

**Added:**

> | **Heap** | `Heap` (owner) + `HeapRef` (8-B handle) + `HeapVec<T>`, `HeapBox<T>`, `HeapDyn<V>`, `HeapString`, `SortedMap<K,V>` | persistent, individually freed | **single writer** (structural mutation = dispatcher-only, EM2 — `entity_master.rs:642` "race-free as long as no concurrent structural mutation runs", `:660` "`live_count: usize` is dispatcher-only (`&mut self`); no worker reaches it"); readers anywhere; **no handle ever names the `Heap` struct** | size-class intrusive LIFO free lists, heads resident in the reservation | **no** (reused) | none | **no** (relocates on grow) | **0 until first alloc** → 4 KiB header page + one 4 KiB page per class touched (O3: lazy, not eager) |

**Removed** (§2.3, the whole Heap-class block, verbatim):

> ```rust
> /// Single-writer size-class heap on two reservations.
> pub struct Heap {
>     small: VmReservation,                 // 4 GiB VA (Miri 8 MiB); 4 KiB pages assigned to classes on demand
>     small_frontier: usize,                // next unassigned page offset (page-aligned)
>     small_committed: usize,               // granule-rounded commit frontier
>     free: [u32; CLASS_COUNT],             // per-class intrusive LIFO head: offset/16 from `small.base`, 0 = empty
>     large: VmReservation,                 // 16 GiB VA (Miri 8 MiB); granule-rounded runs
>     large_frontier: usize,
>     large_free: VmColumn<RunFree>,        // (offset_granules: u32, len_granules: u32) LIFO per exact size, first-fit
>     #[cfg(debug_assertions)] owner: ThreadId,
>     #[cfg(any(miri, feature = "mem-sanitize"))] live: VmColumn<u8>,   // 1 bit per 16-B unit: the FLECS_SANITIZE analogue
> }
> // CLASS_COUNT = 24: 16*k for k in 1..=16 (16..256 B), then 512, 1 KiB, ... 64 KiB.
> // class(layout) = branchless: size <= 256 ? (size+15)>>4 - 1 : 16 + (ceil_log2(size) - 9).
> // align <= 16 served by class granularity; align 32/64 rounds size up to a class that is a multiple
> // of the alignment and pages are 4 KiB-aligned, so every chunk of a class whose size is a multiple
> // of 64 is 64-aligned. align > 64 -> large tier (granule-aligned).
>
> pub struct HeapVec<T> {
>     ptr: NonNull<T>,   // dangling when cap == 0
>     len: u32,
>     cap: u32,          // elements; derived from the class at allocation, stored to avoid the class lookup on push
>     heap: NonNull<Heap>,   // the owner; Drop frees through it
> }
> // 24 B (same as Vec). Deref<Target=[T]> gives sort/binary_search/iter for free.
> // Invariant HV-1: `heap` outlives self (owner drop order, §5).
> // Invariant HV-2: no pointer into the buffer is held across a push (review rule; ledger column).
>
> pub struct HeapBox<T> { ptr: NonNull<T>, heap: NonNull<Heap> }      // 16 B, sized T only
>
> /// Type-erased object with a static vtable - the threadpool `Task` pattern (16 B thin),
> /// replacing Box<dyn System>, Box<dyn FnOnce(&mut EcsMaster)>, Box<TermList>.
> pub struct HeapDyn<V: 'static> { data: NonNull<u8>, vtable: &'static V, layout: Layout, heap: NonNull<Heap> }
> pub trait DynVTable { unsafe fn drop_in_place(&self, p: NonNull<u8>); }
> // V is a per-trait struct of fn pointers built by a generic `const fn of::<T>() -> &'static V`.
>
> pub struct HeapString(HeapVec<u8>);   // utf8 invariant; impl core::fmt::Write; Deref<str>
> ```

**Added:**

> ```rust
> // ---------- Heap class, rev 2: C1's shape ----------
> //
> // THE RULE (HEAP-1): every byte of mutable heap bookkeeping lives INSIDE the
> // reservation it describes. The `Heap` struct holds only write-once reservation
> // owners, so a `&mut EcsMaster` retag over it can disable nothing that anyone
> // later writes through. Every handle names the RESERVATION BASE, never the
> // `Heap` struct. This is `ComponentPool::row_ptr`'s argument verbatim
> // (`component_pool.rs:819-828`: provenance derives from one write-once base,
> // one allocated object, never remapped) and `ScopeBlock`'s D1 argument verbatim
> // (`block.rs:35-42`: copying a raw pointer out of a table does not retag its
> // pointee).
>
> /// Reservation-resident bookkeeping. Page 0 of the small reservation. NOTHING
> /// holds a reference to this type — it is reached only as
> /// `base.cast::<HeapHeader>()` through a raw pointer read out of `HeapRef`.
> #[repr(C, align(64))]
> struct HeapHeader {
>     large_base: *mut u8,      // the large reservation's base, so a free needs no `Heap`
>     small_frontier: u32,      // next unassigned 4 KiB page, in pages
>     small_committed: u32,     // commit frontier, in pages
>     large_frontier: u32,      // in granules
>     large_committed: u32,
>     large_free_len: u32,      // entries in the run table that follows
>     _pad: u32,
>     free: [u32; CLASS_COUNT], // per-class intrusive LIFO head, unit = 16 B from `small_base`; 0 = empty
>     // 100 B of heads at CLASS_COUNT = 25; the hot classes (16..64 B) share one line.
>     #[cfg(debug_assertions)] owner: u64,   // owning thread, debug only
>     // followed in page 0 by: large_free: [RunFree; MAX_RUNS]  (offset_granules: u32, len_granules: u32)
>     // followed at page 1.. under cfg(any(miri, feature = "mem-sanitize")) by the live-bit map,
>     //   1 bit per 16-B unit of the small reservation: the FLECS_SANITIZE analogue.
> }
>
> /// The heap's OWNER. 32 B, every byte write-once after `new`. Held as a field by
> /// `EcsMaster` / by a mod. Its only job is `Drop` (release both reservations).
> pub struct Heap {
>     small: VmReservation,   // 1 GiB VA (Miri 4 MiB), LAZY: 0 committed until the first alloc
>     large: VmReservation,   // 1 GiB VA (Miri 4 MiB), LAZY
> }
>
> /// What every handle carries: the small reservation's base, i.e. the heap's
> /// identity AND the address of its header in one word. `Copy`, 8 B.
> #[repr(transparent)]
> #[derive(Clone, Copy)]
> pub struct HeapRef(NonNull<u8>);
> // Send + Sync: the value is inert. USING it is gated by HV-3 below, not by the type.
>
> // CLASS_COUNT = 25 (was 24; O1 adds the ZST class 0).
> // class(size) = branchless, no underflow at 0:
> //   size == 0                 -> 0            (ZST, never allocates; see O1)
> //   0 < size <= 256           -> (size + 15) >> 4          (1..=16)
> //   size > 256                -> 17 + (ceil_log2(size) - 9)
> // Alignment rules unchanged from rev 1 (align <= 16 free; 32/64 by class-size multiple;
> // align > 64 -> large tier, granule-aligned).
>
> pub struct HeapVec<T> {
>     ptr: NonNull<T>,   // dangling when cap == 0; INSIDE the reservation when cap > 0
>     len: u32,
>     cap: u32,          // elements; from the class at allocation, stored to avoid the class lookup on push
>     heap: HeapRef,     // the RESERVATION BASE. Never a `&Heap`, never a `NonNull<Heap>`.
> }
> // 24 B — `Vec` parity, so no column stride regresses. Deref<Target=[T]>.
> // HV-1 (rev 1, kept): the `Heap` outlives self (owner drop order, §5.1).
> // HV-2 (rev 1, kept): no pointer into the buffer is held across a push.
> // HV-3 (NEW): push / pop / grow / Drop run ONLY on the heap's owner thread.
> //   Reads (`Deref`, `as_slice`, `iter`) run anywhere. Enforced by the scheduler
> //   (structural mutation is dispatcher-only, EM2) and checked by
> //   `debug_assert_eq!(header.owner, current_thread())` on every mutating entry.
>
> pub struct HeapBox<T> { ptr: NonNull<T>, heap: HeapRef }      // 16 B, sized T only
>
> /// Type-erased object with a static vtable — the threadpool `Task` pattern —
> /// replacing Box<dyn System>, Box<dyn FnOnce(&mut EcsMaster)>, Box<TermList>.
> /// 24 B, NOT the "16 B thin" rev 1 claimed (Q5): `layout` moved into the vtable,
> /// and the third word is the heap base that C1 requires.
> pub struct HeapDyn<V: 'static> { data: NonNull<u8>, vtable: &'static V, heap: HeapRef }
> pub trait DynVTable {
>     const LAYOUT: Layout;                                   // Q5: was a runtime field
>     unsafe fn drop_in_place(&self, p: NonNull<u8>);
> }
> // V is a per-trait struct of fn pointers built by `const fn of::<T>() -> &'static V`.
>
> pub struct HeapString(HeapVec<u8>);   // utf8 invariant; impl core::fmt::Write; Deref<str>
> ```

**Removed** (§2.4, the Heap API block, verbatim):

> ```rust
> // Heap
> impl Heap {
>     pub fn new(label: &'static str) -> Self;
>     pub fn alloc(&mut self, layout: Layout) -> NonNull<u8>;          // never zeroed
>     pub unsafe fn free(&mut self, p: NonNull<u8>, layout: Layout);   // class from layout; no header
>     pub fn resident_bytes(&self) -> usize;
> }
> impl<T> HeapVec<T> {
>     pub fn new_in(heap: &mut Heap) -> Self;                          // cap 0, no allocation
>     pub fn with_capacity_in(heap: &mut Heap, cap: u32) -> Self;
> ```

**Added:**

> ```rust
> // Heap — every mutating method takes `&self`, because nothing in the struct is mutated.
> impl Heap {
>     pub fn new(label: &'static str) -> Self;        // no reservation yet (O3)
>     pub fn as_ref(&self) -> HeapRef;                // the one way to get a handle
>     pub fn resident_bytes(&self) -> usize;
> }
> impl HeapRef {
>     /// # Safety: caller is the owner thread (HV-3); `layout` is the layout the
>     /// matching `free` will pass.
>     pub unsafe fn alloc(self, layout: Layout) -> NonNull<u8>;   // never zeroed; ZST -> dangling (O1)
>     /// # Safety: HV-3; `p` came from `alloc` on THIS heap with THIS layout.
>     pub unsafe fn free(self, p: NonNull<u8>, layout: Layout);
>     /// # Safety: HV-3. In-place when the class does not change, else alloc+memcpy+free.
>     pub unsafe fn grow(self, p: NonNull<u8>, old: Layout, new: Layout) -> NonNull<u8>;
> }
> impl<T> HeapVec<T> {
>     pub fn new_in(heap: HeapRef) -> Self;                            // cap 0, no allocation
>     pub fn with_capacity_in(heap: HeapRef, cap: u32) -> Self;
> ```

*(the remaining `HeapVec` method lines of §2.4 are unchanged; `HeapDyn::new` changes its first parameter from `&mut Heap` to `HeapRef` and drops the `vtable` argument's `layout` companion per Q5.)*

**Why this is not slower than rev 1.** Rev-1 free path: load `self.heap` (1), index `(*heap).free[c]` (1 load, 1 store). Rev-2 free path: load `self.heap.0` (1), `hdr = base` (0 — the header is AT the base), index `(*hdr).free[c]` (1 load, 1 store). **Identical instruction count.** The alloc path adds one `sub`+`shr` on the *free* path only (`offset16(p, base)`), which rev 1 also had (`(p-base)/16`, §2.5 row `Heap::free`). Net: zero.

**Rejected sub-variant, with its overturn gate.** `HeapVec` could be 16 B instead of 24 B by deriving the base from `ptr` with a mask, if the reservation were aligned to its own size (`VirtualAlloc2` + `MEM_ADDRESS_REQUIREMENTS` on Windows; `mmap` over-reserve + trim on Linux). Rejected for v1: the Windows API is version-gated and the over-reserve/trim dance has no in-tree precedent, against an 8-byte saving on one component column. **Overturn gate:** if a `Children`-heavy scene shows the `Children` column's bytes-touched-per-frame in the top 3 of the frame's D-cache profile, the aligned-reservation variant lands and `HeapVec` becomes 16 B.

### C1's red-first Miri test (mandatory, lands with the primitive)

`crates/boyko_memory/tests/miri_heap_handle_aliasing.rs`, run under **both** legs of P11.

| | |
|---|---|
| Body | Build an `EcsMaster`-shaped owner struct `{ heap: Heap, other: u64 }`. `let v = HeapVec::<u32>::with_capacity_in(owner.heap.as_ref());` push 8. Then take `&mut owner` afresh, call `owner.heap.as_ref().alloc(..)` **and** `free(..)` through it (so the sibling tag performs both a read and a write of the class head), then drop `v` — whose `Drop` writes the same class head through the handle minted before the sibling retag. Repeat with `HeapBox`, `HeapDyn`, `HeapString`, `SortedMap`. |
| Must be GREEN | on the shipped shape, on SB and on TB. |
| RED-first mutation (pasted in the file header, as the census gate does) | **one line**: move `free: [u32; CLASS_COUNT]` out of `HeapHeader` and into `struct Heap`, and make `HeapVec.heap` a `NonNull<Heap>`. Expected: `error: Undefined Behavior: write access through <TAG> ... is forbidden`, protected/parent tag born at the `&mut owner` retag, accessed tag at `HeapVec::drop`. The test file records the exact diagnosis text of the mutated run, per the tree's gate convention (`scope.rs:206-214` is the precedent). |
| Anti-vacuity | the green run asserts the mutation site is reachable: a counter incremented in `HeapVec::drop`'s free must read ≥ 5 (one per handle type). A run that reports 0 drops is a vacuous pass. |

---

## P2 (closes C2) — §2.1 Scope row, §2.3 `ScopeArena`, §2.4 Scope API, §4 rungs 1a/1b: the arena is DELETED; the defect class is removed rather than fixed

**Depends on:** `block.rs:1-22, 35-42, 133-176, 193-209, 292-350, 400-419`; `tls.rs:106, 126-161, 163-181, 304-309`; `scope.rs:756-908`; `thread_pool.rs:278, 328`; §5.3 (as amended by P11).

**Re-derivation first — what Stage 3b already did, which rev 1 did not know.** Rev 1 designed a per-thread-slot `ScopeArena` because the pre-3b pool put task cells on `std::alloc` one per spawn. On the shipped pool:

- every `Scope` owns a `ScopeBlock` **INLINE — a field, not a pointer** (`block.rs:6-7`), i.e. in the **joiner's own stack frame**. A stack frame is a thread identity that cannot collide, so **there is nothing left to key by `wid`**, and C2's collision class ceases to exist rather than being repaired;
- nested LIFO is structural: an inner scope's block is an inner frame's field;
- what remains per scope is exactly two acquisitions: `Box::new(ScopeShared)` (`thread_pool.rs:278` for `install`, `:328` for `scope`) and one `std::alloc` chunk in `ScopeBlock::grow` (the import at `block.rs:101`, the call at `:482`), freed at `free_all` (`block.rs:328-350`, the `dealloc` at `:341`) and allocated again by the next scope.

`WORKER_ID_DISPATCHER` is still a ROLE, not an identity, on this tree — the constant itself at `tls.rs:106` (its sibling `WORKER_ID_UNATTACHED` is the one the doc at `:108` calls a "Sentinel"), `:136-137` ("inside an `install` frame (on ANY thread, worker or not)"), `:307` ("`install` rewrites the slot's `wid` to `WORKER_ID_DISPATCHER`"). So C2's evidence holds; rev 2 simply does not build the thing that would have suffered from it.

**Removed** (§2.1, the Scope row, verbatim):

> | **Scope** | `ScopeArena` (one per thread slot, `W+1`), replaces `ScopeBlock`'s `std::alloc` chunks and `Box<ScopeShared>` | one `Scope` (nested LIFO) | the thread that opened the scope; indexed by `wid` from the existing single TLS read | `rewind(mark)` at join | **no** | at join | within a scope | 0 until used → HWM; typical ≤ 64 KiB |

**Added:**

> | **Scope** | `ScopeBlock` (**unchanged, inline in `Scope`**) + `ChunkArena` (one `VmReservation` per `ThreadPool`) + `ChunkCache` (one per CLAIMED slot) | one `Scope` (nested LIFO, structural — an inner scope is an inner stack frame) | **the thread that opened the scope, identified by a slot it CLAIMED**, never by a role sentinel | `free_all` returns chunks to the claimed slot's cache | **no** | at join, unchanged | within a scope, unchanged | pool-wide: `(W + D) × 60 KiB` cached + peak live; at W=16, D=8 → **1440 KiB (1.41 MiB) per pool** |

**Removed** (§2.3, the whole Scope-class block, verbatim):

> ```rust
> // ---------- Scope class ----------
>
> /// One per thread slot in the pool (W workers + the dispatcher slot), padded
> /// against false sharing between neighbouring workers' cursors.
> #[repr(C, align(64))]
> pub struct ScopeArena {
>     cur: usize,      // HOT
>     end: usize,      // HOT
>     base: NonNull<u8>,
>     hwm: usize,
>     res: Option<VmReservation>,   // lazy; VA = SCOPE_ARENA_RESERVE (64 MiB per slot, Miri 1 MiB)
>     _pad: [u8; 24],
> }
> // Accessed only by the owning thread through `UnsafeCell` in the pool's per-slot table.
> ```

**Added:**

> ```rust
> // ---------- Scope class, rev 2: no arena, a chunk SOURCE plus a chunk CACHE ----------
> //
> // `ScopeBlock` does not change: same `cur`/`end`/`bases`/`exps`, same `bump`,
> // same `emplace`, same D1/D2/D3, same 312-byte size pin, same
> // `offset_of!(ScopeBlock, cur) == 0` / `end == 8` pins (block.rs:176, 190-191).
> // Only two of its lines change: where `grow` gets a chunk and where `free_all`
> // puts one back.
>
> /// The pool's chunk source. ONE reservation per `ThreadPool`, replacing
> /// `std::alloc::alloc` at block.rs:482 and `dealloc` at block.rs:341.
> /// Bump-only; chunks come back to a per-slot cache, never to the frontier.
> pub(crate) struct ChunkArena {
>     res: VmReservation,        // 64 MiB VA (Miri 1 MiB), LAZY
>     frontier: AtomicUsize,     // bytes carved; fetch_add, ONE per chunk that was never cached
>     committed: AtomicUsize,    // commit frontier; advanced under a #[cold] path
> }
>
> /// One per CLAIMED slot. Depth 1 per exponent class.
> /// EXP_CACHED = 4 covers 4 / 8 / 16 / 32 KiB; a bigger chunk is returned to the
> /// arena frontier's free list (#[cold], and the census shows 1.74 chunks/scope,
> /// i.e. exps 0 and 1 dominate).
> #[repr(C, align(64))]
> pub(crate) struct ChunkCache {
>     slots: [Cell<*mut u8>; EXP_CACHED],   // 32 B
>     // 32 B of implicit tail padding to the 64-byte alignment. O2: NO explicit
>     // `_pad` field — `align(64)` already pads, and rev 1's `_pad: [u8; 24]` on a
>     // 48-byte body would have produced 72 B rounded to 128 B, i.e. two lines.
> }
> const _: () = assert!(size_of::<ChunkCache>() == 64);   // one line per slot, exactly
> ```

**Removed** (§2.4, the Scope API block, verbatim):

> ```rust
> // Scope (threadpool-facing)
> impl ScopeArena {
>     pub fn mark(&self) -> usize;
>     pub fn bump(&mut self, size: usize, align: usize) -> NonNull<u8>; // the ScopeBlock::bump shape: 2 loads, mask, 2 cmp, 1 store
>     pub fn rewind(&mut self, mark: usize);
> }
> ```

**Added:**

> ```rust
> // Scope (threadpool-internal; NOT public API, and NOT reachable by a mod — see §7)
> impl ChunkArena {
>     pub(crate) fn new() -> Self;                                  // no reservation yet
>     pub(crate) fn carve(&self, exp: u8) -> *mut u8;               // #[cold]; fetch_add + maybe commit
>     pub(crate) unsafe fn release_all(&self, caches: &[ChunkCache]);// ThreadPool::drop, after the join
> }
> impl ChunkCache {
>     #[inline] pub(crate) fn take(&self, exp: u8) -> *mut u8;      // 1 load, 1 store, 1 branch; null = miss
>     #[inline] pub(crate) fn give(&self, exp: u8, p: *mut u8) -> bool; // false = cache full, caller keeps it
> }
> ```

### The identity fix, in one word of existing padding

`LaneDeposit` (`tls.rs:126-142`) ends in `_pad: u32` at offset 20, documented as "Explicit tail padding, so `size_of` is readable ... and so `Cell::set` writes no indeterminate byte". **`_pad` becomes `cache_slot: u32`.**

- All three layout pins survive **unchanged**: `size_of == 2 * size_of::<*const ()>() + 8` (`tls.rs:159`), `align_of == align_of::<*const ()>()` (`:160`), `offset_of!(LaneDeposit, pool) == 0` (`:161`). No new `thread_local!` static (checkpoint defect 7: each one costs a System cell per thread on windows-gnu).
- **Zero extra TLS accesses on the spawn path.** The slot is already read once per lane query (D7, `tls.rs:113-115`); `Scope::new` already resolves the lane to compute `joiner_wake` (`scope.rs:585-589`). The claimed index rides in the same read.
- `DETACHED` seeds `cache_slot: NO_CACHE_SLOT` (`u32::MAX`).

| Thread | Slot | Written when |
|---|---|---|
| a pool worker | its own `wid` | `worker_main` entry, once |
| an installing thread (any thread, worker or not) | a slot claimed from `PoolInner.dispatcher_claim: AtomicU64` bitset by `fetch_or` of the lowest clear bit | `ThreadPool::install` entry; released by `InstallGuard` on return **and on unwind** |
| a thread that could claim none (bitset full) | `NO_CACHE_SLOT` | — |

**Exhaustion degrades, it does not corrupt** — the property C2 found missing. With `NO_CACHE_SLOT` the block calls `ChunkArena::carve` on every grow and returns chunks to the arena's cold free list: today's cost, correct behaviour, no collision. `D = 8` dispatcher slots (one `u64` word holds `W ≤ 56` + 8); a scene that exhausts them is `#[cold]` and is pinned at 0 occurrences by the census gate.

**Nested scopes on one thread are correct by construction**, and this is the property C2 demanded: two nested scopes on the same thread share one `ChunkCache`, but the outer scope takes its chunk first and returns it last, so the depth-1-per-exponent cache serves depth-1 nesting at a 100 % hit rate and deeper nesting at today's cost. There is no cursor to alias because there is no arena cursor — each block keeps its own `cur`/`end` in its own frame.

### `Box<ScopeShared>` is emplaced in the block

`ScopeShared` becomes the **first `emplace` into the scope's own `ScopeBlock`**, replacing both `Box::new` sites (`thread_pool.rs:278, 328`). Justification, in the order the soundness argument must be read:

1. **It is not reachable through `&mut Scope`.** Chunk memory is a separate allocation reached only by a raw pointer copied out of the block's table — `block.rs:35-42`'s D1, which the tree already gates under Miri.
2. **No completer touches `*shared` after its decrement**, on either route, so the block's free at `Scope::drop` is exactly as safe as today's `Box::from_raw`: route (b) reads `joiner_wake` **before** the `fetch_sub` and wakes through a `PoolInner`-owned target (`scope.rs:823-832, 855-876`); route (a) unparks **before** the `fetch_sub` because `waker` lives in the allocation (`scope.rs:767-771, 909-911`).
3. **Construction order.** `ScopeBlock` is built as a local, `ScopeShared` is emplaced into it, then the block is **moved once** into the `Scope` before any task can observe it. Moving the block moves only the table (`cur`/`end`/`bases` are raw pointers into chunks); a raw pointer's provenance is carried in its value and is unaffected by moving the word that stores it. `offset_of!(Scope, block) == 16` and the `Scope` cache-line claim (`block.rs:185-191`) are unaffected.
4. An empty scope now takes one chunk where it took none — and that chunk is a **cache hit**, not an allocation. Measured target: `install` goes from 1 acquisition (`block.rs:196-199`'s "a scope that spawns no task makes no allocator call at all" plus the `Box`) to **0**.

### C2's red-first Miri test (mandatory, lands with rung 1a)

`crates/boyko_threadpool/tests/miri_scope_slot_identity.rs`:

- **Body:** two threads `install` on the SAME pool concurrently and each runs a scope that spawns `N` tasks writing a thread-distinct pattern into their cells; a third arm installs from a pool worker (the `tls.rs:648` / `scope.rs:1811` shapes). Assert every cell reads back its own pattern and every chunk is returned exactly once.
- **RED-first mutation, one line:** replace the claimed slot with `current_worker_id_or_dispatcher_lane(worker_count)` (`tls.rs:406-411`), i.e. rev 1's keying. Expected: Miri data race on `ChunkCache.slots`, or a chunk double-return that the `mem-sanitize` live-bit column reports. The file records the exact diagnosis.
- **Plus a non-Miri arm:** `dispatcher_claim` exhaustion (`D` forced to 1 under `cfg(test)`) must degrade to `NO_CACHE_SLOT` and stay correct, with a counter proving the cold path ran ≥ 1 time (anti-vacuity).

---

## P3 (closes W1) — §5.3: the protector gates key on a POISON WRITE, not on a deallocation

**Depends on:** P2 (chunk retention removes the deallocation), `scope.rs:77-89, 206-214, 327-341, 448-487`, `lib.rs:104, 118, 169`.

W1 predicted this and rev 2 makes it sharper: with retention, `free_all` performs **no `dealloc` at all**, so *both* Miri counters — `MIRI_FREES_INSIDE_A_RELEASE_WINDOW` (which observes the `ScopeShared` `Box::from_raw`, `scope.rs:333-334`) and `MIRI_BLOCK_FREES_INSIDE_A_RELEASE_WINDOW` (which observes `free_all`, `:335-338`) — would go **silently green**, and the `ScopeShared` box they were built around no longer exists (P2). That is the "gate that could not fail" class, twice over.

**Removed** (§5.3, first sub-bullet of the sanitize bullet, verbatim):

> Compensation: under `cfg(any(miri, feature = "mem-sanitize"))` the Heap keeps a live-bit column (1 bit per 16-B unit), `free` asserts the bit is set and clears it, `alloc` asserts clear and sets it, freed chunks are filled with `0xDD` beyond the link word; `FrameArena::rewind`/`reset` and `ScopeArena::rewind` fill the released range with `0xDD`. This is `FLECS_SANITIZE` `[S flecs.h]`, and it is the only use-after-free detector the arenas will ever have.

**Added:**

> Compensation, and it is now TWO mechanisms with different jobs:
>
> **(a) The live-bit column, for the Heap.** Under `cfg(any(miri, feature = "mem-sanitize"))` the header's live-bit map (1 bit per 16-B unit, resident in the small reservation per P1) is asserted set and cleared by `free`, asserted clear and set by `alloc`; a freed chunk is filled `0xDD` beyond its link word. This is `FLECS_SANITIZE` `[S flecs.h]` and it is the only use-after-free detector inside a reservation, because provenance is per-allocation `[D std::ptr]`.
>
> **(b) The POISON WRITE, for the scope class — and it REPLACES the two release-window gates' trigger.** `ScopeBlock::free_all` no longer deallocates (P2), so the event the gates keyed on is gone. Under `cfg(miri)` only, `free_all` writes `0xDD` over `[bases[i], bases[i] + cap(i))` for every chunk **before** returning it to the cache, at exactly the point the `dealloc` used to be, and `miri_note_block_free_against_open_windows(key)` is re-homed to that write. Three facts make this a STRICTLY STRONGER detector than the `dealloc` it replaces, and the strength is the reason the replacement is acceptable:
>   1. a `dealloc` is UB only under a **strong** protector (`StrongProtector` ⇒ `ProtectedDealloc`, `[S miri tree_borrows/tree.rs]`), whereas a **foreign write** invalidates a location protected by *either* kind — the weak `noalias`-not-`dereferenceable` protector a `&mut F` argument installs is included `[S miri borrow_tracker/mod.rs]`;
>   2. the protector this gate exists to catch is exactly that one: the drop glue of an unrun scoped task hands the user's `Drop::drop(&mut self)` a `&mut F` whose referent is chunk memory (`block.rs:55-62`);
>   3. it therefore also covers the `ScopeShared`, which now lives in chunk memory (P2) — one mechanism where rev 1's tree had two, and the two counters MERGE into one with a written note that they did, because keeping a second counter over an event that no longer exists is the vacuity this row is about.
>
> **This is a deliberate, `cfg`-gated violation of TB-1** (see P11): TB-1 forbids the owner writing inside a chunk precisely so that no foreign write can happen, and the poison write is that forbidden write, performed on purpose in a configuration where a violation is a diagnosis rather than a corruption.
>
> **RED-first, re-established before rung 1a lands** (W1's requirement): change `ScopeShared::complete_task`'s receiver from `*const Self` back to `&Self` and its two `task/scoped.rs` call sites to `&*shared` — the mutation `scope.rs:206-214` already publishes as the pre-fix shape. Note that the published mutation reds with `deallocation through <TAG> ... is forbidden` at `Scope::drop`'s `Box::from_raw`; on the new trigger the expected diagnosis is `error: Undefined Behavior: write access through <TAG> ... is forbidden`, protected tag born at the mutated `complete_task` argument, accessed tag at `free_all`'s poison write. **Re-blessing that text is part of the rung, not an afterthought.** Recipe and seeds as published (`scope.rs:79-89`); the run must show **zero** `KE16-PROTECTOR-GATE-ARMED` lines, i.e. it reds on the property, not on its own armedness.
>
> `FrameArena::rewind`/`reset` fill the released range with `0xDD` under the same gate.

---

## P4 (closes W2, and the `Sync` question it exposes) — §2.3/§2.4 Frame class: `&self` + `Cell`

**Depends on:** `block.rs:83-89` (the `&self` + `Cell` precedent), `ecs_master.rs:122-127, 1283-1292` (the manual `unsafe impl Sync` and its `!Send` interior precedent).

**Removed** (§2.3, the Frame-class block, verbatim):

> ```rust
> /// Single-writer bump arena. NOT Sync; owned by EcsMaster; reset once per frame
> /// by `frame_reset_system` (first system of the schedule, exclusive).
> #[repr(C)]
> pub struct FrameArena {
>     cur: usize,                 // HOT: bump cursor (byte offset)
>     end: usize,                 // HOT: committed frontier (byte offset)
>     base: NonNull<u8>,          // write-once
>     hwm: usize,                 // high-water mark (diagnostics + the floor number)
>     res: Option<VmReservation>, // lazy; VA = FRAME_ARENA_RESERVE (256 MiB, Miri: 4 MiB)
> }
> pub struct FrameMark(usize);    // LIFO rewind token; `#[must_use]`
>
> /// Fixed-capacity vector on the frame arena. Capacity chosen at creation;
> /// overflow is a #[cold] re-bump + copy (bumpalo semantics) - callers size it.
> pub struct FrameVec<'f, T> { ptr: NonNull<T>, len: u32, cap: u32, _f: PhantomData<&'f FrameArena> }
> // 16 B. No Drop for T (T: Copy bound in v1 - same rule as VmColumn; DropColumn covers droppable persistent data,
> // droppable per-frame data has no client in the inventory).
> ```

**Added:**

> ```rust
> /// Single-writer bump arena, `&self` + interior mutability — the `ScopeBlock`
> /// shape (`block.rs:83-89`), and for the same compiler-enforced reason: a
> /// `&mut self` allocator ties every handle to an exclusive borrow, so rung 2's
> /// CSR two-pass (a `FrameVec<u32>` of offsets alive beside a flat
> /// `FrameVec<Entity>`) does not compile. W2.
> #[repr(C)]
> pub struct FrameArena {
>     cur: Cell<usize>,           // HOT: bump cursor (byte offset)
>     end: Cell<usize>,           // HOT: committed frontier (byte offset)
>     base: Cell<*mut u8>,        // write-once after the first commit
>     hwm: Cell<usize>,           // high-water mark (diagnostics + the floor number)
>     res: UnsafeCell<Option<VmReservation>>, // lazy; VA = 256 MiB (Miri 4 MiB)
>     #[cfg(debug_assertions)] owner: Cell<u64>,
> }
> pub struct FrameMark(usize);    // LIFO rewind token; `#[must_use]`
>
> /// Fixed-capacity vector on the frame arena. Capacity chosen at creation;
> /// overflow is a #[cold] re-bump + copy (bumpalo semantics) — callers size it.
> pub struct FrameVec<'f, T> { ptr: NonNull<T>, len: u32, cap: u32, _f: PhantomData<&'f FrameArena> }
> // 16 B. `T: Copy` in v1 (unchanged rationale: the inventory has no droppable
> // per-frame datum; if one appears the answer is `DropColumn`, not a finalizer chain).
> //
> // SYNC. `Cell` makes `FrameArena` `!Sync`, and `EcsMaster` holds one. That does
> // NOT change `EcsMaster`'s advertised auto-traits, because they are already a
> // MANUAL `unsafe impl Send` / `unsafe impl Sync` (`ecs_master.rs:1287-1288`)
> // over an interior that already contains a `!Send` slab routed by discipline
> // rather than by types (`nonsend_resources`, `:127`, guarded by the
> // behavioural test `nonsend_system_runs_on_dispatcher_and_observes_resource`,
> // `:125`). The arena takes the SAME contract and the SAME guard shape, and the
> // `unsafe impl Sync` justification block gains a bullet SEND11 naming it —
> // an explicit required edit, not a silent inheritance. (SEND10 is the highest
> // number in use today, `ecs_master.rs:1250`, so SEND11 is free.)
> ```

**API change** in §2.4: `FrameArena::{alloc, alloc_slice_uninit, vec, rewind}` take `&self`; `reset(&mut self)` keeps `&mut` (bumpalo's rule, `[S bumpalo lib.rs]`: "requires `&mut` to guarantee no active borrows"), which is what makes a live `FrameVec<'f>` across a reset a **compile error** rather than a rule. The one way to obtain `&FrameArena` is `EcsMaster::frame(&mut self) -> &FrameArena`, so access is gated by the exclusive world borrow that only a dispatcher-run exclusive system holds; N `FrameVec`s may be live from that one shared reference.

**Non-exclusive systems do not get the frame arena.** A param-based system's per-frame scratch stays `Local<ScratchColumn>` — the tree's existing shape and already alloc-free after warm-up (census: every system body measured 0).

---

## P5 (closes W3, and records P8's answer) — §2.2: the `ZeroInit` bound

**Depends on:** `vm_column.rs:24-28, 81, 116, 135-140`; `enable_store.rs:60, 203-212`.

**Removed** (§2.2, third bullet, first sentence, verbatim):

> - `VmColumn::new` drops the `COMMIT_GRANULE % size_of::<T>() == 0` pin (`[ecsnative] vm_column.rs:144-149`).

**Added:**

> - `VmColumn::new` drops the `COMMIT_GRANULE % size_of::<T>() == 0` pin (`vm_column.rs:135-140` on the KE16 tree; the rev-1 citation `:144-149` is the `[ecsnative]` numbering of the same `# Panics` list).
> - **The bound splits in two (W3).** `VmColumn<T: Copy>` (`vm_column.rs:81`) cannot hold `EnablePage([AtomicU64; WORDS_PER_PAGE])` (`enable_store.rs:60`, `WORDS_PER_PAGE = 64`), and `Copy` is the wrong property anyway: what the zero-fill contract needs is *all-zero is a valid value* and *no destructor*.
>   ```rust
>   /// # Safety: every all-zero bit pattern of `Self` is a valid value, and
>   /// `Self` has no `Drop` glue (`needs_drop::<Self>() == false`).
>   pub unsafe trait ZeroInit {}
>   // Blanket-implemented for the primitives and for AtomicU8..AtomicU64,
>   // [T; N] where T: ZeroInit, and #[repr(C)] structs of ZeroInit fields via a derive.
>   ```
>   `VmColumn<T: ZeroInit>` is the struct bound (a relaxation — every `T: Copy` site in the tree keeps compiling once its type is declared `ZeroInit`). Methods that hand a `T` OUT by value (`swap_remove`, `pop`, `get`) keep `where T: Copy`; `ensure_len_zeroed`, `as_slice`, `as_mut_slice`, `len`/`set_len` need only `ZeroInit`. `AtomicU64` is not `Copy` and is `ZeroInit`, so `VmColumn<EnablePage>` type-checks and its growth stays a commit with no fill loop.
> - **P8 recorded:** an absent `EnablePage` reads *false* (`enable_store.rs:203-212`), so zero-filled pages are free and the polarity question rev 1 left open (its Open question 4) is **answered yes** — with W3's type fix as the price.

---

## P6 (closes W4) — §3 G1: the ledger gate scans syntax, not text

**Depends on:** the runtime data ledger on disk (`[main] docs/memory/runtime-data-ledger.tsv`), `[ecsnative] tests/physics_vec_side_store_census.rs:45-92, 218, 277`.

**Removed** (§3, the G1 row's Mechanism cell, verbatim):

> `tests/alloc_field_ledger.rs`, generalised from `[ecsnative] tests/physics_vec_side_store_census.rs` (`SCANNED_ROOT` → every runtime crate's `src/`, `:218`): scans struct fields typed `Vec<|Box<|String|Arc<|Rc<|VecDeque<|BTreeMap<|BTreeSet<|HashMap<|HashSet<` outside `#[cfg(test)]`; requires the set to equal `const LEDGER: &[Site { crate, file, struct, field, class, rung, tag }]` **exactly** (a missing site = red, an extra site = red — so the list can only shrink in the same commit that deletes the field); `MIN_SITES` anti-vacuity floor as in the physics test (`:277`)

**Added:**

> `tests/alloc_field_ledger.rs` at the workspace root, scanning **the parsed syntax tree, not the text**: `syn` + `proc-macro2` as **dev-dependencies of the test target only** (tag `harness` — not linked into any shipped binary; this is the one place the in-house rule yields, because the alternative is a text matcher whose blind spots are measured below). It walks every runtime crate's `src/`, visiting `ItemStruct`, `ItemEnum` (incl. **tuple-variant payloads**), `ItemUnion` and `ItemType`, resolving **type aliases** within the crate, honouring `#[cfg(test)]` / `#[cfg(not(test))]` **as parsed attributes**, and matching on the resolved path segments (`alloc::vec::Vec`, `Vec`, `std::vec::Vec` all collapse to one key). Compared for **exact set equality** against the ledger's `runtime-data-ledger.tsv` rows, per crate; a missing site is red and an extra site is red, so the list can only shrink in the same commit that deletes the field.
>
> **Why the text scanner is not generalised (W4's measured holes, all in the false-green direction):** tuple enum variants unread (`:54-57`), aliases invisible (`:45-48`), `#[cfg(not(test))]` read as a test region (`:69-76`), hand-wrapped generics read to the comma (`:77-83`), text match on `Vec<` only (`:45, :52-53`). The shapes those holes hide — `Box<dyn ..>`, `Box<[..]>`, enum payloads — are exactly the shapes physics lacked and the other 19 crates have. The physics scanner's own floor constant is `MIN_CONTAINERS` (`:277`), with a second floor on field declarations parsed below it.
>
> **Anti-vacuity, three-sided:** (i) a per-crate `MIN_SITES` floor from the ledger's own per-group counts (ecs-storage 109, ecs-schedule 166, app-demo 256, codec-tools 527 … — these four are identical in ledger rev 3 and rev 4, verified, so the floors do not move with the revision); (ii) the scanner must report a nonzero count for **every** crate the ledger has rows in (a crate that silently stops parsing reads as "clean"); (iii) a fixture directory with one file per known hole, each asserted **detected** — the test that proves the new scanner sees what the old one could not.

---

## P7 (closes W5, O4) — §3 G3 and G4

**Removed** (§3, the G3 row's Mechanism cell, verbatim):

> `#[global_allocator]` that delegates to `System` until `enter_steady()`; afterwards `alloc` writes a fixed 64-byte message with `WriteFile`/`write(2)` and `abort()`s.

**Added:**

> `#[global_allocator]` that delegates to `System` until `enter_steady()`; afterwards, **unless `std::thread::panicking()`** (a TLS bool read, no allocation), `alloc` writes a fixed message with `WriteFile`/`write(2)` and `abort()`s. **The exemption is load-bearing, not politeness (W5):** `panic!`/`assert!` formatting allocates, so without it every genuine assertion failure inside the steady window aborts with the deny message *before the real message exists*, and every failing gate in the suite would read as "allocation after steady". The message names the thread role (`dispatcher` / `worker N` / `external`) because G2's counter is process-global and the role is the first thing a reader needs. Miri arm counts instead of aborting, `cfg(not(miri))` for the abort (the tree measured Tree-Borrows UB delegating to Windows `System` for over-aligned blocks under Miri, `[merge] block.rs:759-782`).

**Removed** (§3, the G4 row's Red-first canary cell, verbatim):

> (a) `Vec` field in production → red; (b) `Vec` local in a `#[cfg(test)]` module → **must not** red (a canary that does not fire is a finding — verify both directions)

**Added:**

> The red-first set is **seven shapes and their verdicts are recorded, not assumed** (O4: `disallowed_types` walks `hir::Ty`, so an annotated local and a turbofish `collect::<Vec<_>>()` fire, while `vec![]` and an inferred-type `.collect()` may not): (a) a `Vec` field in production; (b) `Vec::new()` local; (c) `collect::<Vec<_>>()` turbofish; (d) `vec![]`; (e) inferred `let v = xs.iter().collect();`; (f) `format!` / `.to_string()`; (g) `Box::new` inside a macro expansion. Each row of the file records **fires / does not fire**, and the ones that do not are what the ledger gate (G1) is responsible for instead — an honest split beats a lint believed to cover more than it does. Plus the negative canary, which is a finding either way: (h) a `Vec` local in a `#[cfg(test)]` module → **must not** red. O9's second half folds in here: prove the `alloc::vec::Vec` path form fires on a `Vec` written through the prelude.

---

## P8 (closes O1, O2, O3, O6, O7, O8, Q3, Q4, Q5, Q6; retires O5) — the small rows

**O1 — ZST.** `class(0) = 0`, a class that never allocates: `HeapRef::alloc` returns `NonNull::dangling()` for `size == 0` and `free` returns immediately, the shape `ScopeBlock::emplace` already ships (`block.rs:253-261`). The rev-1 formula `(0+15)>>4 - 1` underflowed; `CLASS_COUNT` becomes 25 and the formula is stated in P1. Affected clients: `HeapDyn`/`HeapBox` of a capture-less closure — the `Box<dyn FnOnce(&mut EcsMaster)>` field at `[merge] schedule_builder.rs:92` and its construction at `:263`. (Rev 1's critique row cited `[merge] schedule_builder.rs:1090`; that line is `systems: Vec<String>` in `OrderingCycle` and is not a closure site — corrected here.) ⚠ *Rev 2.5 (P46): this O1, O3 (as ruled by P15), O8, Q4's Heap and `TableSet` rows (as replaced by P30/O1), Q5 and Q3's `TableSet` build point belong to the revival forms; O2, O6, O7, Q3's growable-column half and Q6 stand.*

**O2 — padding.** The `ScopeArena` whose `_pad: [u8; 24]` made 72 B round to 128 B is deleted (P2); `ChunkCache` states `size_of == 64` as a build-failing assert and carries **no explicit `_pad`**.

**O3 — Miri residency.** Both `Heap` reservations are **lazy** (0 committed and 0 reserved until the first alloc), like Frame and Scope, because the Miri fallback arm eagerly `alloc_zeroed`s the full `os_len` (`vm.rs:168-179`). Miri arm constants: Heap small 4 MiB, large 4 MiB, Frame 4 MiB, ChunkArena 1 MiB per pool. **Rung-time obligation before the constants are trusted:** measure the existing suite's Miri RSS with `cargo +nightly miri test -p boyko-ecs` and record it; today's worst eager per-pool footprint is 6 MiB (`[ecsnative] crates/boyko_ecs/src/ecs/constants.rs:61-67`, "4 MiB data + 2 × 1 MiB ticks = 6 MiB").

⚠ **WRITER'S FLAG, for the critic (not an architect's decision): O3 and P1 do not agree, and the disagreement is load-bearing.** P1's `Heap` struct declares `small: VmReservation` / `large: VmReservation` — plain, not `Option` — while O3 requires "0 **reserved** until the first alloc", which needs an `Option` filled on the first allocation. But the first allocation runs through `HeapRef::alloc(self, ..)`, and a `HeapRef` carries only the reservation base: it cannot create the reservation it is the base of, and `Heap::as_ref(&self)` has nothing to hand out before one exists. The three exits are (i) reserve eagerly in `Heap::new` and commit lazily — address space is free on the release arm, and the Miri arm's cost is then exactly what the 4 MiB constants bound; (ii) keep the `Option` and make `as_ref` `&mut self`, which re-opens C1's retag question at the one site C1 exists to close; (iii) keep the `Option` behind interior mutability in the `Heap` struct, which is variant (a) that P1 rejected. **The writer did not choose** — the text above is as the architect wrote it, and this paragraph is the record that it contains a contradiction rather than a decision.

**O6 — sealing.** Rust has no crate-group privacy, so this is a gate, not a type, and rev 2 says so instead of implying a language mechanism. `VmReservation::{reserve, commit, base}` (`vm.rs:109, 199, 184` respectively) become `#[doc(hidden)] pub` inside `boyko_memory::raw`, and the G1 test grows a second assertion: **the set of files importing `boyko_memory::raw` equals a named allowlist** (`boyko_ecs`, `boyko_threadpool`, and `boyko_memory`'s own tests). Red-first: add an import elsewhere. This is also §7's K-MOD-7: a mod cannot reach `commit`.

**O7 — panic recovery.** `CommandQueue.panic_recovery` is a **second `ByteColumn`**, and the re-absorb inside `catch_unwind` (`command_queue.rs:64-68, 385-393`) is `spare_ptr(n)` + `copy_nonoverlapping` + `set_len` — an append that commits if needed and **never reallocates**, so the tail re-absorption's no-move property is preserved by construction rather than by luck.

**O8 — determinism row added to §5.2:**

> | 2, 3 | `HeapVec` capacity classes change WHEN a buffer reallocates, versus `Vec`'s doubling | no order dependence (single writer, program order). **Obligation:** any test pinning `Vec::capacity()` growth — e.g. `[ecsnative] entity_master.rs:477`'s `self.free_entity_ids.capacity() * size_of::<EntityId>()` inside `memory_usage()` — is re-baselined in the same commit, never left to pass silently on a different number |

**Q3 — the two incremental sites leave the `Table` class.** `EventBuffer` lanes are sized at `preregister_event` (`event_api.rs:22`; the buffer's own doc states it at `event_buffer.rs:192`, "All allocations happen once at `preregister_event` time; no allocation"), which is incremental before `App::finish()`; and `EnableColumn::ensure_directory` regrows (`enable_store.rs:218-233`). Both take a **growable column**, not a fixed-length `Table`: `EventBuffer` lanes → `VmColumn<MaybeUninit<E>>` per lane; `EnableStore.pages` → `VmColumn<EnablePage>` (P5 makes it type-check). `TableSet`'s build point is then unambiguous — `MasterTables` is built at `App::finish()` and contains only sites whose count is final there — and late registration has no path to a sealed table because no such site remains.

**Q4 — reservation accounting (VA reserved; resident is high-water and starts at 0):**

| Owner | Reservations | VA (release) | VA (Miri) | Resident floor at boot |
|---|---|---|---|---|
| `EcsMaster` | Heap small, Heap large, FrameArena | 1 GiB + 1 GiB + 256 MiB = **2.25 GiB** | 4 + 4 + 4 MiB | **0** (all lazy) |
| `EcsMaster` | MasterTables (`TableSet`) | granule-rounded total, ≤ 4 MiB | same | one granule, 4 KiB after packing-plan D1 |
| `ThreadPool` | ChunkArena | **64 MiB** | 1 MiB | 0 → `(W+D) × 60 KiB` cached + peak live; **1440 KiB (1.41 MiB) at W=16, D=8** |
| `Schedule` | ScheduleTables | ≤ 1 MiB | same | 4 KiB |
| per `ComponentPool` | unchanged | unchanged | unchanged | 64 KiB → 4 KiB after packing plan S0-S2 |
| per mod (§7) | Heap small + large | 2 GiB | 8 MiB | 0 |

Rev 1's 4 GiB + 16 GiB is cut to 1 GiB + 1 GiB: `InlandStore` already reserves 1 GiB on 64-bit (`inland_store.rs:5`, `DEFAULT_INLAND_RESERVE`) and is the tree's precedent, and the large tier's client list is empty after rung 2 moves the entity-scaled maps to columns. **Overturn gate:** a `Heap` exhausting 1 GiB of VA in any suite reds its own `#[cold]` exhaustion panic, which is the signal to raise the constant.

**Q5 — `HeapDyn` is 24 B, and the "16 B thin" claim is withdrawn.** `layout` moves into the vtable as `DynVTable::LAYOUT` (an associated const, so it costs no word and no load at the free site — the free reads it from a `&'static V` it already holds); the third word is the `HeapRef` that C1's shape requires. Stated in P1's code block.

**Q6 — the injector block is 63 slots, one block per 64 pushes.** The in-tree measurement is the source (`1520 = 8 + 63 × 24`, `alloc_frame_census.rs`; the census report states it as "exactly 1520 bytes, once per 64 spawned tasks"). Note the same report also writes "per 63 pushes" in its BEFORE/AFTER prose; **the 64 form is rev 2's, because that is the form the measured sentence takes**, and the two phrasings differ only in whether the block allocated on the 64th push is charged to it or to the 63 that filled the previous one. The "31-slot" figure in rev 1 §4 rung 1d and in Lens B of ALLOCATOR-RESEARCH.md came from an upstream `master` read that was not of the linked `crossbeam-deque 0.8.8`. The research file is verbatim and is not edited; this row and the pass-1 log's Q6 are the correction of record.

**O5 — RETIRED, with its reason, because its mechanism is removed (P10).** O5 was accepted as a rung-1d obligation: *"the inline path releases the lane borrow before running the body"*. Rung 1d no longer executes anything inline (P10 replaces inline overflow with a bounded spin), so there is no inline path and no borrow to release. **If the spin policy is ever replaced by inline execution, O5's obligation returns verbatim** — this sentence is the record that it was dropped on purpose, not lost.

**O9 — settled in pass 1** (first half refuted, second half folded into P7). Not re-litigated.

---

## P9 (closes Q1) — §3 G2 and §4's rung-1 targets, re-derived on the KE16 pool

**Depends on:** `[main] docs/unification/checkpoint-2026-09-11/frame-allocation-census.md`; the committed gate `crates/boyko_physics/tests/alloc_frame_census.rs` (landed `d5782d43`, 2026-09-13).

**Q1 is answered by re-derivation, not by attribution of rev 1's four.** The four constants of `n + 4` were: **one** `Box<ScopeShared>` and **three** allocations for a scratch crossbeam `Worker` deque built on every `Scope::drop` — the census states it in those words. On the shipped pool the scratch deque is gone (`join_on_worker` / `join_external` build none) and the cells are emplaced in the `ScopeBlock`, so the closed form is **2 per `Schedule::run`, whatever `n` is**: one `Box<ScopeShared>` (256 B, align 128) and one 4 KiB chunk.

**Removed** (§3, the G2 row's Mechanism cell, verbatim):

> `crates/boyko_physics/tests/alloc_frame_census.rs` (untracked on ecsnative) becomes `tests/alloc_frame_gate.rs` at the workspace root: per scene `STEADY_MAX_S0..S3, S1a, S1b` constants; the assertion is `measured_max <= STEADY_MAX_X`; each rung lowers its constants; end state = 0 everywhere. Process-global counter (the `boyko_app/tests/zero_alloc.rs` shape, worker-visible, `:47`), not the `thread_local!` shape of `colored_solve_zero_alloc_o5.rs:281-284`

**Added:**

> **G2 already exists, is committed and is green** — `crates/boyko_physics/tests/alloc_frame_census.rs` (`d5782d43`, `harness = false`, both profiles), with a process-global counting `#[global_allocator]` proven to see worker-thread allocations, and pins **per allocation CLASS** (scope / chunk / injector / OTHER / realloc) rather than one total. Rev 1 described building it; rev 2's job is to **re-pin it downward, rung by rung**, and it inherits three properties rev 1 did not know it had:
> - the per-class pins are what catch a regression the MAX pin misses: the mutation "a fresh `Vec` plus one `push` per step in `physics_apply`" reds on `OTHER` (256 against a budget of 2 or 8, in each of S1a, S1b and S1c) with **every MAX pin green**;
> - the **upward-headroom defect the adjudication found is already fixed** in the committed file: S1c pins are the 4,352-step envelope with no upward room — `scope: (121 - 12, 121)` i.e. 109..=121, `chunk: (193, 217)`, `dispatch_max: 339` (`alloc_frame_census.rs:2079-2081`, with the header stating it at `:209`) — because the first form (133 / 241 / 375) let one extra fan-out per colour pass (+24/step, 331.8 → 355.8, MAX 363) pass green;
> - its coverage boundary is written at the gate: **the counter sees only the Rust heap**; `VmReservation::commit` calls `VirtualAlloc(MEM_COMMIT)` directly and is not counted. So "0 acquisitions" after this campaign means *zero process-heap calls*, not zero memory growth — and that is the honest reading of the end state, stated here so no later reader mistakes one for the other.
>
> **Per-scene targets, restated from the measured KE16 numbers:**
>
> | scene | pre-3b (rev 1's basis) | KE16 today | after rung 1a+1b | after rung 1d |
> |---|---|---|---|---|
> | S0, n = 1..16 systems | `n + 4` (5.016 … 20.254) | **2.0x** (1 scope + 1 chunk) | **0** | 0 |
> | S0b, 4 Main + 4 Fixed | 16.125 | 4.125 | 0 | 0 |
> | S2 churn + `par_iter` | 14.098 | 4.039 | 0 | 0 |
> | S3 query + events | 8.066 | 2.062 | 0 | 0 |
> | S1a/S1b pile, serial | 11.109 / 12.125 | 2.109 / 2.125 | 0 | 0 |
> | S1c pile, colored + parallel, W=4 | 2601..2724 | **302..339** (~1.2 MB/step) | ≤ 1 per new exponent class, then 0 | 0 |
> | injector blocks | 1 per 64 outside pushes | unchanged | unchanged | **0** |
> | OTHER (per-thread first touch) | 2 × workers | unchanged | unchanged | **0** (see 1f) |
>
> **Two pins that must be re-derived rather than widened when they move:** S2's `realloc` pin (the id-list doubling, which EM2′ `0afcbd7d` already changed), and the S1c colour-count arithmetic. The gate's own header states the rule; rev 2 repeats it because a rung that widens a pin instead of re-deriving it is how this gate stops being one.

---

## P10 (closes Q2, re-scopes rung 1d) — §4's rung-1 table

**Removed** (§4, rung 1 rows 1a, 1b and 1d, verbatim):

> | 1a | `Box::new(ScopeShared)` per `install`/`scope` (`[merge] thread_pool.rs:278, 328`) | 1 alloc+free per `Schedule::run`, 1 per `par_iter` | emplaced in the owner thread's `ScopeArena` at `mark`, rewound at join | 1 + (#par_iter) per frame |
> | 1b | `ScopeBlock` chunks via `std::alloc` (`[merge] block.rs:101, 244-290`) | ≥1 alloc+free per scope that spawns | chunks = sub-ranges of the same `ScopeArena`; the D1/D2 Tree-Borrows rules carry over verbatim (`block.rs:35-81`) | ≥1 per scope |

> | 1d | crossbeam `Injector`/`Worker`/epoch (`[merge] worker.rs:646, 671`) | 1 block per 31 injector pushes + epoch node per sealed bag + deque doubling | **in-house bounded Chase-Lev** per lane on a `Table<Task>` ring in the pool's `TableSet` (capacity `LANE_CAP = 4096` × 16 B = 64 KiB per lane, `[merge] task/mod.rs:191-203` thin task); bounded MPMC injector ring (`W × LANE_CAP`); **overflow policy: the pushing thread executes the task inline** (`#[cold]`; helping at push, no unbounded growth). Lê et al. 2013 formulation `[D]`; loom model mandatory | all |

**Added:**

> | 1a | `Box::new(ScopeShared)` per `install`/`scope` (`thread_pool.rs:278, 328`) | 1 alloc+free per `Schedule::run`, 1 per `par_iter`, 1 per dispatched colour (121/step on the pile) | **emplaced as the first cell of the scope's own `ScopeBlock`** (P2), freed with the chunks at `free_all` | 2 per frame (App), ~121 per parallel physics step |
> | 1b | `ScopeBlock` chunks via `std::alloc` (import `block.rs:101`, `alloc` at `:482` inside `grow`, `dealloc` at `:341` inside `free_all`) | 1 alloc+free per spawning scope, 4 KiB doubling (1.74/scope measured) | `ChunkArena` (one pool reservation) + per-claimed-slot `ChunkCache`; **`ScopeBlock` itself is unchanged** — only `grow`'s source and `free_all`'s sink move (P2) | ~210 per parallel physics step |
> | 1f **(NEW)** | per-thread first touch landing in early frames (checkpoint defect 6): crossbeam-epoch `Local` (2304 B) on a thread's first steal, and std's 30-B UTF-16 thread-name copy for `SetThreadDescription` | 660 of 1056 `Local`s landed at frame ≥ 64, i.e. **inside** the steady window, so G2's OTHER allowance is load-bearing rather than decorative | `ThreadPoolBuilder::build` waits for its workers to boot and pre-registers each with the epoch collector | moves 2 × W acquisitions out of the frame window; lets every App scene's OTHER budget drop to 0 |

> | 1d | crossbeam `Injector` — owned at `thread_pool.rs:113` (`injector_global: CachePadded<Injector<Task>>`), pushed at `worker.rs:687-688` — and with it crossbeam-epoch | 1 block per **64** outside pushes (`1520 = 8 + 63 × 24`, Q6); measured 0.125/frame on S1c and `n/63` on S0. **The worker path already allocates nothing** — a worker's push lands in its own Chase-Lev ring (`place_task`, `worker.rs:750` → `push_on_lane_no_wake`, `:700`), which is why rev 1's "replace the per-lane deque" half is withdrawn: the lanes are not a per-frame cost, they double once and stop | bounded MPMC **injector** ring (`W × LANE_CAP`, `LANE_CAP = 4096`, 16 B thin task → 64 KiB per lane) in the pool's `TableSet` ⚠ *Rev 2.5 (P46.3): in the pool's own reservation, KC-06 — `TableSet` is a revival form (U-7)*; the per-lane Chase-Lev deques stay crossbeam's in v1 and are re-examined only if 1f fails to move the epoch `Local` out of the window. **Overflow policy: the pushing thread spins with `crossbeam_utils::Backoff` (already a dependency), `#[cold]`, and NEVER executes the task inline** | the injector blocks; the epoch `Local` follows 1f |

**Q2 is answered by removing the mechanism that raised it.** Rev 1's inline-overflow policy would have executed a concurrent SYSTEM on the dispatcher inside `try_dispatch_ready`, which is a re-entrancy question against the apply window and the dispatcher-owned `ExecutorScratch` (`[merge] executor_scratch.rs:216-219`, "**Dispatcher-owned** — workers never touch this"). Rev 2 does not execute anything inline, so the question does not arise. **Liveness proof for the spin, which is what replaces it:** a full ring means ≥ `cap` tasks are enqueued; every idle worker steals from the injector, and a worker blocked in a nested join also *executes* tasks from it (`join_on_worker`). The only state in which nothing drains is "every worker is parked", and a worker parks only when the queues are empty — which contradicts full. Modelled in loom (push/steal, steal/steal, and a forced-overflow arm with `LANE_CAP = 2` under `cfg(test)`), and the overflow counter is pinned at **0 occurrences** in the census gate, so the cold path is measured to be cold rather than assumed to be.

---

## P11 (closes the research's TB-1 challenge; amends §5.3) — TB-1 keeps its rule and changes its reason; two Miri legs

**Depends on:** P3 (the poison write), `block.rs:35-42, 55-73`.

**Removed** (§5.3, second bullet, verbatim):

> - **Tree Borrows rules inherited from `ScopeBlock` (`[merge] block.rs:35-81`) become library-wide invariants**: TB-1 no bookkeeping word inside a *live* chunk (the intrusive link exists only while the chunk is on a free list, and is read/written through raw pointers derived from the reservation base, never through a reference to the payload type); TB-2 allocate-and-initialise (`alloc` returns `NonNull`, the caller writes before minting any reference); TB-3 no `Deref` from an arena handle that outlives the frame/scope (`FrameVec<'f, T>` borrows the arena; `Scope` cells are consumed at execution).

**Added:**

> - **Tree Borrows rules inherited from `ScopeBlock` (`block.rs:35-81`) become library-wide invariants, and TB-1's RATIONALE is corrected here because rev 1 stated the wrong one.**
>   - **TB-1 — no bookkeeping word inside a chunk that any thread can hold a protector over.** The rule stands; the reason is **not** that the aliasing model forbids a footer inside a managed block. It does not: bumpalo keeps a mutable `ChunkFooter` with `Cell` fields inside every chunk and ships Miri-clean under `-Zmiri-strict-provenance` `[S bumpalo lib.rs, rust.yaml]`, and TB permits it because footer and payload are disjoint ranges reached from the same parent tag `[B ralfj 2023]`. **The real reason is CONCURRENT PROTECTORS, which bumpalo does not have and we do:** after publication the owner must write *nothing* inside a chunk, because a worker can hold a protector over a cell in it — the drop glue of an unrun scoped task hands the user's `Drop::drop(&mut self)` a `&mut F` whose referent is chunk memory, as a function argument (`block.rs:55-62`), and a foreign write to a location under a protector is UB for **both** protector kinds, where a deallocation is UB only under a strong one. So TB-1 is a *concurrency* rule wearing an aliasing-model costume, and it is restated as: **no thread writes inside a chunk another thread may hold a protector over; the intrusive free-list link exists only while a chunk is off every scope and on a free list.** The `Heap`'s own class heads are exempt by placement, not by permission: they live at page 0, a region no handle ever points into (P1).
>   - **TB-2 — allocate-and-initialise.** `alloc` returns `NonNull`, the caller writes before minting any reference. `ScopeBlock::emplace` is this by construction (`block.rs:244-326`, the ZST arm at `:253-261`) and bumpalo's `alloc = self.emplace().write(val)` is the same rule in the wild `[S bumpalo lib.rs]`.
>   - **TB-3 — no `Deref` from an arena handle that outlives the frame/scope.** `FrameVec<'f, T>` borrows the arena (and `reset(&mut self)` makes an outliving handle a compile error, P4); `Scope` cells are consumed at execution.
>   - **TB-4 (NEW) — the poison write is TB-1's one deliberate violation**, `cfg(miri)` / `mem-sanitize` only, and its whole value is that it converts a protector violation into a diagnosis (P3).
> - **Two Miri legs, not one, and the reason is that they check different things.** Default **Stacked Borrows** (what Bevy gates on, `[S bevy ci.yml]`) *and* `-Zmiri-tree-borrows`, the model that formally blesses deriving `base.add(offset)` over a whole reservation from one write-once base `[B ralfj 2023]` — which is what every column, the Heap and the ChunkArena do. Code can be SB-clean and TB-dirty (TB adds the `conflicted` bit on a protected `ReservedFrz` and a protector-end implicit access, `[S miri perms.rs, tree.rs]`), so neither leg subsumes the other. Both legs add `-Zmiri-strict-provenance` — free here, because the handle arithmetic is pointer-based and never casts through an integer, and the APIs are stable since 1.84.0 `[D]`. `RUSTFLAGS: -Zrandomize-layout` on both, Bevy's guard, which is worth more here than there because this tree pins struct sizes.
> - **Do not write `-Zmiri-retag-fields` or `-Zmiri-unique-is-unique` into any recipe.** The first is parsed and warns "is a NOP"; the second no longer exists `[S miri src/bin/miri.rs]`. A `MIRIFLAGS` line carrying them is decoration, and the tree's own lesson is that a flag believed to protect you is worse than no flag.
> - **loom** — `Heap`, `FrameArena` and `ChunkCache` have no atomics (single writer / single slot owner); `ChunkArena.frontier` has one `fetch_add` and one `#[cold]` commit, modelled. The injector ring (1d) uses the pool's existing `cfg(loom)` shim (`[merge] sync.rs:1-15` for the contract, `:63-89` for the two re-export arms — note `:31-36` is the "deliberately **not** shimmed" list and is not the shim). Mandatory models: push/steal, steal/steal, overflow-spin.

**Determinism, re-derived (§5.2, the 1a/1b row).**

**Removed** (§5.2, row 1, verbatim):

> | 1a/1b | `ScopeArena` addresses depend on which worker ran the spawner | **no computation reads a task cell's address** — cells are executed and discarded. Canary: `cfg(feature = "mem-shake")` offsets each per-worker arena base by `wid × 4096 × prime` so any address dependence changes results; the `{1,N}` physics oracle (`[ecsnative] plugin.rs:386`) must stay green with and without it |

**Added:**

> | 1a/1b | chunk addresses now depend on the claimed slot and on whether the cache hit | **no computation reads a task cell's or a `ScopeShared`'s address** — cells are executed and discarded. Canary, re-derived because there is no per-worker arena base to offset: `cfg(feature = "mem-shake")` makes `ChunkCache::take` **miss with probability 1/2** from a per-slot xorshift, so every scope's chunk address changes run to run. The `{1,N}` physics oracle (`plugin.rs:384-388`, "run-to-run bit-deterministic and `{1, N}`-worker bit-identical") must stay bit-identical with and without it, and the shake must be *proven to fire* (a counter of forced misses asserted > 0) — a shake that never shakes is the same vacuity as a gate that cannot fail. |

---

## P12 (NEW section §7) — the modding seam: additive, and compiled out means absent

**Depends on:** P1 (`HeapRef` is the whole allocator ABI), P8/O6 (`raw` is allowlisted), §3 (G6 joins the ladder). ⚠ *Rev 2.5 (P47): §7 is re-pointed to the plan's file 05 (`docs/unification/UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md`), which is authoritative for its content; G6 = UG-15.*

**The requirement, restated as a falsifiable end state.** A build whose dependency graph does not contain `boyko_modding` must be **byte-identical in the engine's own code** to a build of a tree where that crate does not exist: no indirection, no lookup, no branch, no registry, no lock, no allocation, no exported symbol, no binary-size delta, no startup work.

**The mechanism that makes it true by construction, rather than by care: modding is a CRATE, not a feature.** A cargo feature still compiles code into the kernel crate and can perturb inlining decisions and layout; a crate that is not in the graph contributes nothing to compile. `boyko_modding` depends on `boyko_ecs` and `boyko_memory`; nothing depends on it except an application that wants mods. ⚠ *Rev 2.5 (P47): the one crate `boyko_modding` is retired in favour of `boyko_mod_host`, `boyko_mod_api` and `boyko_mod_registry`; the kernel half of any modding item is generic over `ModSeam` and is not compiled when unused (05 §1).*

**What the kernel must expose (the complete additive list, and each entry's cost when unused):**

| # | Seam | Shape | Cost when modding is absent |
|---|---|---|---|
| K-MOD-1 ⚠ *Rev 2.5 (P47): withdrawn with U-1 (05 §4)* | allocation | `HeapRef::{alloc, free, grow}` take a **runtime `Layout`** and are non-generic | none — `HeapDyn` already needs them, so they exist either way; an unreachable `pub fn` is dropped at link |
| K-MOD-2 ⚠ *Rev 2.5 (P47): withdrawn with U-1 (05 §4)* | heap identity | `HeapRef` is `Copy + Send + Sync`, 8 B, `#[repr(transparent)]` over `NonNull<u8>` | none — it is the handle every engine collection already carries (P1) |
| K-MOD-3 ⚠ *Rev 2.5 (P47): removed (U-1, U-10; 05 §4)* | mod memory lifetime | **one `Heap` per mod**: `Heap::new()` → 2 lazy reservations. Unloading a mod is `Heap::drop` = 2 `VirtualFree`, which bounds a mod's leak at its own reservation and never touches the engine's free lists | none — no `Heap` is constructed |
| K-MOD-4 ⚠ *Rev 2.5 (P47): kept (05 §2 row 3)* | dynamic components | **the pool constructor is ALREADY layout-erased and non-generic**: `ComponentPool::new(component_id: usize, reserve_rows: usize)` (`component_pool.rs:279`) resolves `component_layout`, `drop_fn` and `type_id` out of `component_registry::get_layout_unchecked(component_id)` (`:284-288`). There is no `ComponentPool::new::<T>()` and therefore no generic path a mod would have to duplicate — the mod's pool and the engine's pool are the same call | **none, and this is the key to "identical codegen"** — there is no second path and no branch to take; the erased entry point already IS the only one |
| K-MOD-5 ⚠ *Rev 2.5 (P47): kept as MS-03 and MS-02b (05 §3.2, §4)* | dynamic ids | the id counter is `component_registry::register_new::<T>()` (`component_registry/mod.rs:920-921`, `NEXT_ID.fetch_add(1, Ordering::Relaxed)`) — the **same counter** the derive macro uses. It is generic today only to key the `TypeId` mint, so the additive seam is a non-generic sibling taking `(Layout, Option<DropFn>)`; the kernel never distinguishes the resulting ids, so it never branches on origin | none — the counter and its `fetch_add` exist for the derive |
| K-MOD-6 ⚠ *Rev 2.5 (P47): kept as MS-09 (05 §4)* | dynamic drop glue | the `drop_fn` a pool already stores: `pub type DropFn = unsafe fn(*mut u8)` (`component_registry/mod.rs:83`) in `pub drop_fn: Option<DropFn>` (`:117`) — the Bevy `Option<unsafe fn(OwningPtr)>` shape `[S blob_array.rs]`, already in-tree. A mod supplies an `extern "C"` thunk | none |
| K-MOD-7 ⚠ *Rev 2.5 (P47, P51): kept per 05 §4; its gate is UG-15 leg (1) plus a `raw` import census* | what is NOT exposed | ~~`boyko_memory::raw::{reserve, commit, base}`~~ `boyko_memory::raw::{reserve, commit_at}` ⚠ *Rev 2.6 (P54/O4): KC-01's names*, ~~`FrameArena`,~~ `ChunkArena`/~~`ChunkCache`~~ `SlotChunks`, `ScopeBlock`. A mod gets ~~`Heap`/`HeapVec`/~~columns and nothing that could commit pages or reach the pool's scope memory | enforced by O6's allowlist gate |
| K-MOD-8 ⚠ *Rev 2.5 (P47): kept (05 §4)* | registry | the mod registry is a **`Resource` inserted by `boyko_modding`** at first load (Principle 0: it is ECS data, not a side store). **No kernel field, no enum variant, no `Option<ModRegistry>` anywhere in `EcsMaster`** | none — an un-inserted resource slot is an index that is never read; `size_of::<EcsMaster>()` is unchanged, and G6 asserts it |
| K-MOD-9 ⚠ *Rev 2.5 (P47): kept = UG-15 leg (1) (05 §4)* ⚠ *Rev 2.7 (P56.2, AP8 N-W1): its thunks are called by a mod or by the host after `main`, so they are outside the modding-crate ban on runtime-invoked code; the kernel's leg-(1) allowlist stays empty* | ABI | every `#[no_mangle] extern "C"` thunk lives in `boyko_modding`; the kernel exports no C symbol | none — no symbol exists to export |

**Why a mod gets its own `Heap` rather than the world's.** Three reasons, all performance or soundness: (i) HV-3's single-writer invariant would otherwise be shared with untrusted code; (ii) a mod's free-list corruption would be a kernel corruption; (iii) unload becomes `O(1)` and leak-bounded instead of a walk. Cost to the engine: zero, because no `Heap` is constructed when no mod is loaded. **Overturn gate:** if a scene loads > 64 mods and the per-mod 4 KiB header pages show up in the resident profile, mods share one `Heap` partitioned by class range. ⚠ *Rev 2.5 (P47): withdrawn with K-MOD-3 — under load-only (U-10) reason (iii) is void, and reasons (i) and (ii) apply to any engine structure a mod writes (05 §4).*

**G6 — the zero-cost gate (joins the ladder in §3):** ⚠ *Rev 2.5 (P47): G6 = UG-15 (the plan's 03 §6, U-11); the row-by-row map is P47.3.*

| Check | Mechanism | Red-first canary |
|---|---|---|
| (a) no exported symbol | `objdump -t` on `boyko_demo` built **without** `boyko_modding`: zero symbols matching `boyko_mod_*` / `^mod_api_` | add a `#[no_mangle] pub extern "C" fn` to `boyko_ecs` → red |
| (b) identical codegen | build the same commit twice, once with the crate present but unused by the binary and once with it removed from the workspace; `.text` size of the kernel's named hot functions (`Schedule::run`, `ComponentPool::push`, `HeapRef::alloc`, `ScopeBlock::bump`) compared symbol-by-symbol | add a `cfg!(feature = "modding")` branch to any of the four → red |
| (c) no kernel growth | `size_of::<EcsMaster>()`, `size_of::<ComponentPool>()`, `size_of::<Scope>()` pinned equal across both arms | add an `Option<ModRegistry>` field → red |
| (d) no startup work | the frame-allocation census (G2) reports **identical** setup and steady numbers in both arms | construct anything at boot in the off arm → red |

---

## P13 — §6: the owner questions become decisions (the delegation of 2026-09-11)

**Removed** (§6 heading and preamble, verbatim):

> ## 6. VALUES/SCOPE questions for the owner only
>
> 1. **Third-party coverage.** Defaults chosen here: crossbeam-deque replaced (rung 1d, per-frame); `std::thread::spawn` accepted at boot (`os-thread`); Vulkan driver host allocations accepted (`ffi-driver`). Confirm or widen — widening `ffi-driver` means a thread-safe heap for a foreign caller, which is the one thing this design deliberately does not build.

**Added:**

> ## 6. Decisions taken under the owner's delegation ("decide all the questions yourself, whichever is best for performance", 2026-09-11), each with the gate that would overturn it
>
> | # | Decision | Reason | Overturn gate |
> |---|---|---|---|
> | 1 | Third-party: the crossbeam **`Injector` is replaced** (1d); the per-lane **Chase-Lev deques stay** crossbeam's in v1; `std::thread::spawn` accepted (`os-thread`); Vulkan driver host allocations accepted (`ffi-driver`) | the deques are not a per-frame cost (they double once); the injector is 1 block per 64 outside pushes and is the whole of every scene's periodic MAX. Widening `ffi-driver` would force a thread-safe heap for a foreign caller — the one thing this design does not build | a census showing lane-deque growth inside a steady window, or 1f failing to move the epoch `Local` out of the frame |
> | 2 | `diag` sites migrate **last** (rung 4) and are outside the measured window until then | they are cold and feature-gated; moving them early buys nothing and touches ~291 ledger rows (rev 4's diagnostics count; rev 3 read 301) | a `diag` site appearing in a steady-state census window |
> | 3 | Rung 3 may go **ahead of** rung 2 if the packing plan slips; no design change either way | rung 3 has zero per-frame cost and is pure ruling-3 work; rung 2 needs the 4 KiB page floor to be economic | — |
> | 4 | **No trim policy.** Frame/Scope/Chunk floors are high-water marks | Dawson's ~175 µs/MB first-touch + ~150 µs/MB kernel zeroing `[B randomascii 2014]` would be paid every frame by a per-frame trim; a periodic trim is policy without a measured need | a measured resident ceiling on a target (editor with many worlds) |
> | 5 | **Ordering against open defects.** Rung 1 starts after checkpoint defects 2 (A1 sleep latch) and 3 (rejected GPU upload leak) land — owner's standing rule, bugs before features. Rungs 1a/1b touch only `boyko_threadpool` and do not conflict with either | — | the owner reversing the order |

---

## P14 — implementation plan and measurement queue (replaces §"Implementation plan" steps 3-6 and §"Metrics and validation" bullet 1)

**Removed** (implementation plan, steps 3-6, verbatim):

> 3. `FrameArena`, `FrameVec`, `ScopeArena`, `Heap`, `HeapVec`, `HeapBox`, `HeapDyn`, `HeapString`, `SortedMap`, `DropColumn`, `TableSet` in `boyko_memory`, each with unit tests, the sanitize arm, and Miri runs.
> 4. G2 gate (frame census → thresholds) and G3 deny allocator; **run G2 first** to replace structural counts with measured ones and fix the 1a-1e order.
> 5. Rung 1a-1c (threadpool scope arena, byte columns); re-run G2; lower constants.
> 6. Rung 1d: in-house bounded deque + injector; loom tests; `{1,N}` oracle with `mem-shake`.

**Added:**

> 3. `boyko_memory` primitives: `ZeroInit` + the relaxed `VmColumn` bound (P5), `DropColumn`, `ByteColumn`, `FrameArena`/`FrameVec` (`&self`, P4), `Heap`/`HeapHeader`/`HeapRef`/`HeapVec`/`HeapBox`/`HeapDyn`/`HeapString`/`SortedMap` (P1), `TableSet`. Each with unit tests, the sanitize arm, and **both** Miri legs (P11). **C1's red-first test lands in this step, before any client**, and the O3/P1 laziness contradiction flagged in P8 is resolved before the `Heap` is written.
> 4. G3 deny allocator with the `panicking()` exemption (P7). G2 already exists and is green — it is **re-pinned**, never widened (P9).
> 5. Rung 1a+1b as **one commit** (they are one change: `ScopeShared` into the block and the block's chunks onto the `ChunkArena`), with C2's red-first test and W1's re-established protector gate on the poison write (P3). Then re-pin G2: App scenes to 0.
> 6. Rung 1f (boot-wait + epoch pre-registration), then re-pin the OTHER budgets to 0.
> 7. Rung 1c (byte columns, O7's second `ByteColumn`), 1d (injector ring + loom + spin-counter pin at 0), 1e.
>
> *(steps 7-9 of rev 1 renumber to 8-10, unchanged in content.)*

**Removed** (Metrics and validation, first bullet, first sentence, verbatim):

> - **Benchmarks**: `Schedule::run` floor for 0/1/2/4/8/16 systems (S0) before/after 1a-1b — expected delta = two allocator calls per run, i.e. tens of ns; the point is the count, not the time.

**Added:**

> - **Measurement queue (entries, not runs — benchmarks execute only on the owner's quiet word):**
>
> | ID | Entry | Arms | What decides |
> |---|---|---|---|
> | M-A1 | `Schedule::run` floor, 0/1/2/4/8/16 systems | before / after 1a+1b | the **count** goes 2 → 0 (G2, no machine needed); the time delta is expected to be tens of ns and is recorded, not asserted |
> | M-A2 | `par_iter` over 1M entities in 4096-row chunks | chunk cache hit vs `std::alloc` | per-scope cost of the chunk source |
> | M-A3 ⚠ *Rev 2.5 (P46): struck with U-1 (03 MQ-08)* | `HeapRef::alloc/free` at 16 B and 256 B | vs `System` | bar: mimalloc's < 10 ns median `[B Forrest Smith]` |
> | M-A4 ⚠ *Rev 2.5 (P46): struck with U-1 (03 MQ-08)* | `HeapVec::push` vs `Vec::push`, 1k / 1M | — | must be within noise (same algorithm); a difference means the class ladder is wrong |
> | M-A5 | `EntitySlotMap` growth to 1M ids | `ensure_len_zeroed` vs `resize` | the fill loop is the measured difference |
> | M-A6 | 1240-body pile, W = 1/8/16 | before / after rung 1 | **this is the entry that would overturn P0**: > 1 % attributable to the removed acquisitions raises rung 1's priority; < 0.3 % confirms the unification-only justification |
> | M-A7 ⚠ *Rev 2.5 (P46): kept as AL:M-A7 (03 MQ-08); its Heap half is the revival form's* | Miri RSS of the existing suite | before setting O3's constants | the Heap's Miri arm sizes |

---

## Change log (rev 1 → rev 2)

| Row | Disposition | Where |
|---|---|---|
| C1 | **CLOSED** — shape (b): bookkeeping inside the reservation, `HeapRef` = reservation base; red-first Miri test specified with a one-line mutation | P1 |
| C2 | **CLOSED** — the `ScopeArena` is deleted; identity moves from a role sentinel to a claimed slot in `LaneDeposit`'s existing padding word; exhaustion degrades; red-first Miri test specified | P2 |
| W1 | CLOSED — the release-window gates key on a `cfg(miri)` poison write, which is strictly stronger than the `dealloc` it replaces; the two counters merge with a note | P3 |
| W2 | CLOSED — Frame class is `&self` + `Cell`; the `Sync` consequence is answered from the tree's own `nonsend_resources` precedent and adds a SEND11 bullet | P4 |
| W3 | CLOSED — `unsafe trait ZeroInit`, split from `Copy` per method; P8's polarity answer recorded | P5 |
| W4 | CLOSED — G1 scans `syn`, not text; the holes become a fixture suite; anti-vacuity is three-sided | P6 |
| W5 | CLOSED — `std::thread::panicking()` exemption, role in the message | P7 |
| O1, O2, O3, O6, O7, O8 | CLOSED (O3 with a writer's flag: its laziness clause contradicts P1's `Heap` struct) | P8 |
| O4 | CLOSED — seven-shape red-first set with recorded verdicts | P7 |
| O5 | **RETIRED with its reason stated** — the inline-overflow mechanism is removed, so the obligation has no subject; it returns verbatim if inline execution ever returns | P8, P10 |
| O9 | settled in pass 1; not re-opened | — |
| Q1 | CLOSED by re-derivation — the four were 1 `ScopeShared` + 3 scratch-deque; on KE16 the closed form is 2 per `Schedule::run` | P9 |
| Q2 | CLOSED by removing the mechanism — no inline execution; bounded spin with a liveness proof and a pinned-at-0 counter | P10 |
| Q3 | CLOSED — `EventBuffer` lanes and `EnableStore.pages` leave the `Table` class for growable columns | P8 |
| Q4 | CLOSED — reservation accounting table; VA cut from 20.25 GiB to 2.25 GiB per master | P8 |
| Q5 | CLOSED — `HeapDyn` is 24 B; `layout` becomes `DynVTable::LAYOUT`; the "16 B thin" claim is withdrawn | P8 |
| Q6 | CLOSED — 63 slots, one block per 64 pushes; the in-tree measurement is the source of record | P8 |
| P1-P8 (pass-1 positives) | preserved; P8 additionally answered by W3's type fix | P5 |
| — | **NEW:** the throughput justification is withdrawn and replaced by three that survive the null A/B, with an overturn gate | P0 |
| — | **NEW:** TB-1 keeps its rule and corrects its rationale against the research's bumpalo counter-example; two Miri legs; two dead flags named | P11 |
| — | **NEW:** §7 modding seam, nine kernel entries, G6 with four checks and four red-first canaries | P12 |
| — | **NEW:** rung 1f (per-thread first touch), G2 re-pinning discipline, measurement queue M-A1..M-A7 | P9, P10, P14 |

## Open questions for the critic (rev 2)

1. **`ScopeShared` in chunk memory vs. the `Scope` cache-line claim.** P2 keeps `offset_of!(Scope, block) == 16` and the two hot words at bytes 16..32, but the first chunk now begins with a 256-B `ScopeShared` at align 128, so the first *task cell* starts ~256-384 B into the chunk instead of at byte 0. I judge this free (the cells are written once and read once, on another thread), but it is a layout change stated without a measurement, and M-A2 is where it would show.
2. **`D = 8` dispatcher claim slots** is a guess constrained only by "`W + D` must fit one `u64`". The census can tell us the real concurrent-installer count; until it does, the number is defended only by the graceful-degradation path.
3. **`syn` in a test target** is the one place this plan admits a third-party parser into the repository. The alternative is a hand-written Rust-subset parser in `boyko_utils`, which is more in-house and more likely to have its own blind spots. I chose the measured-blind-spot risk over the unmeasured one; the critic may weigh it differently.
4. **`Children(HeapVec<Entity>)` may be the wrong destination entirely** — the ledger makes relations a first-class kernel form, and a `Children` that is a relation needs no `HeapVec` at all. Rev 2 keeps rev 1's assignment so the two documents do not silently diverge, and flags it as the one rung-2 row whose destination another plan may own.
5. **(Writer-raised, not the architect's.) O3's "0 reserved until the first alloc" cannot be satisfied by P1's `Heap` struct as written**, because the first allocation goes through a `HeapRef` that carries only a reservation base and so cannot create the reservation. The three exits, and why none is free, are set out in P8's O3 flag. This needs an architect's ruling before step 3 of P14, and it is the one place where two parts of rev 2 contradict each other rather than merely being incomplete.

---

# Part IV - Critique log - pass 2 (2026-09-16)

**Verdict of pass 2: CHANGES REQUESTED** - four blockers (C1-C4), six important remarks (W1-W6), three optional notes (O1-O3), eight preserved positives, seven open questions for the architect. Three of the four blockers are in a mechanism rev 2 **introduced** (`ChunkArena` / `ChunkCache`) and the fourth is in §7, which pass 1 never saw; none is a re-litigation of pass 1.

Rules of this log, as in Part II:

- Part III above is **unchanged**. No finding is answered by silently editing rev 2; every answer lands in Rev 2.1 below and names the rev-2 text it removes.
- The log is reproduced **verbatim as the critic wrote it**, including its own headings, its confidence tags, its "preserve these" list and its open questions. The dispositions are in Rev 2.1's change log.
- Trees as the critic states them: code read in `D:/wt/joltab` (`merge/ke16-into-ecsnative`) unless marked; documents in `D:/claude/BoykoEngine`.

---

# Architecture review: allocator design space — rev 2 (pass 2)

## Verdict

**CHANGES REQUESTED** — 4 blockers, 6 important, 3 optional.

Rev 2 is a large and mostly high-quality patch. C2's defect *class* is genuinely removed rather than patched; C1's shape (b) is the right one and I verified its mechanism holds in this tree; P0's withdrawal of the throughput justification against a measured null is the kind of self-refutation that must be preserved verbatim. Every `FIX IN REV 2` row is addressed and I checked each one — only O3 is left unresolved, and the writer flags it. The blockers below are **not** re-litigations of pass 1: three are in the *new* mechanism P2 introduces (`ChunkArena` / `ChunkCache`), one is in §7, which pass 1 never saw.

All code read in `D:/wt/joltab` (`merge/ke16-into-ecsnative`) unless marked; documents in `D:/claude/BoykoEngine`. The runtime data ledger on disk is **rev 4** (`docs/memory/RUNTIME-DATA-LEDGER.md:1`), and P6's four anti-vacuity counts are confirmed unchanged from rev 3 (`:47-48`).

---

## 🔴 Critical (blockers)

### C1. C1 is not closed: `HeapRef` can neither create nor commit the reservation it names
**Where**: P1 (`Heap`, `HeapRef`, `impl HeapRef`), P8 O3 + WRITER'S FLAG, rev-2 Open question 5, P14 step 3.

Two parts, one root cause.

1. The flagged contradiction is real and unresolved. P1 declares `small: VmReservation` / `large: VmReservation` (plain); O3 requires "0 **reserved** until the first alloc"; the first alloc runs through `HeapRef::alloc(self, ..)`, which carries only a base pointer. Three exits are offered and none is chosen — and exit (ii) (`as_ref(&mut self)`) **re-opens C1 at the one site C1 exists to close**, while exit (iii) is variant (a) that P1 already rejected.
2. A second instance rev 2 does not mention: **`HeapHeader` has no route to `commit`.** It holds `large_base`, `small_frontier`, `small_committed`, `large_frontier`, `large_committed` — but not `os_len` and not the reservation. In this tree `VmReservation::commit(&self, old, new)` is a method on the reservation (`crates/boyko_ecs/src/ecs/memory/vm.rs:199`; `base()` at `:184`). So `HeapRef::alloc`'s cold "assign page, thread chunks" path cannot make the page writable, and cannot detect VA exhaustion — which P8/Q4 promises ("a `Heap` exhausting 1 GiB of VA reds its own `#[cold]` exhaustion panic").

**Consequence**: P14 step 3 ("C1's red-first test lands in this step, before any client") cannot be written. A developer choosing at the keyboard picks the exit that compiles — `as_ref(&mut self)` — putting `HeapVec.heap` back on a tag that `&mut EcsMaster` disables, with a green C1 test because its mutation (moving `free[]` into `struct Heap`) no longer describes the defect. The other exit's cost is bounded and measurable: 8 MiB of eager `alloc_zeroed` per `EcsMaster` on the Miri fallback arm (`vm.rs:168-179`, confirmed eager over the full `os_len`) against today's worst per-pool 6 MiB.

**Confidence**: CONFIRMED (plan text vs plan text; `vm.rs:168-179, 184, 199`).

**What is needed**: an architect's ruling with its overturn gate that keeps "no handle ever names the `Heap` struct" **and** gives the reservation-resident header a self-contained route to reserve/commit and to its own VA bound. Note the constraint forcing the fork is one line of `vm.rs`'s Miri arm, not the `Heap`'s shape.

### C2. Returned chunks have nowhere to go, and `ChunkArena` is a bump-only lifetime budget
**Where**: P2 (`ChunkArena` doc, `ChunkCache`, `give() -> bool`, "Nested scopes … correct by construction"), P8 Q4.

Three statements do not compose: `ChunkArena` is "Bump-only; chunks come back to a per-slot cache, **never to the frontier**"; `ChunkCache` is depth 1 per exponent with `give(&self, exp, p) -> bool; // false = cache full, caller keeps it` — **"keeps it" is never defined and `ChunkArena` has no field that could receive it**; yet two other sentences refer to "the arena frontier's free list", a structure in no struct.

The depth-1 cache does not serve the shipped nesting. `Schedule::run` installs a scope (`crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:466`) and every `par_iter`/`par_chunk` opens a **nested** one (`iters/query/par_iter.rs:341`, `iters/query/par_chunk.rs:139`). Under P2 the install scope emplaces `ScopeShared` immediately and so **holds** an exp-0 chunk for the whole frame — where today `ScopeBlock::new` allocates nothing and "a scope that spawns no task makes no allocator call at all" (`crates/boyko_threadpool/src/block.rs:196-199`). Per frame: outer takes exp0 (hit) → first inner misses → carve → inner returns, slot filled → later inners hit → **outer returns last into a full slot → orphan**. The ladder guarantees a second orphan class: `e = min_e.max(i)`, `cap = CHUNK0 << e`, `CHUNK0 = 4096`, `MAX_CHUNKS = 32` (`block.rs:105, 116, 455-456`), so chunk index 4 is 64 KiB and `EXP_CACHED = 4` can never hold it.

**Consequence**: ~1 orphan × 4 KiB per `install` frame at 60 fps (two schedules per frame in the App scenes) consumes a **bump-only, never-reclaimed** 64 MiB pool reservation in roughly four minutes, after which `carve` walks off the reservation. Faster on the physics path (census: 121 scopes + 210.67 chunks per pile step, `docs/unification/checkpoint-2026-09-11/frame-allocation-census.md:252`). Separately, because the arena is a *process-lifetime* budget, 64 MiB becomes a new hard ceiling on scope fan-out that `std::alloc` never imposed (≈1M 32-byte cells, reachable by a `par_iter` with a small chunk size). It also invalidates P2's "that chunk is a cache hit, not an allocation" and P8/Q4's `(W+D) × 60 KiB` residency figure.

**Confidence**: CONFIRMED.

**What is needed**: the chunk *return* path must be a named structure with a stated capacity and a stated full-behaviour, for both the over-exponent and the full-slot case, with residency re-derived. If the answer is a free list on the arena, it is pool-wide and therefore concurrent — price it against C3 rather than leaving it a sentence.

### C3. The chunk source and cache are a new cross-thread protocol with no ordering, no `Sync` argument, and an explicit statement that there is nothing to model
**Where**: P2 (`ChunkArena.frontier/committed: AtomicUsize`, `ChunkCache { slots: [Cell<*mut u8>; EXP_CACHED] }`, the claim/release table), P11 final bullet.

1. **The cache is handed between threads**: a dispatcher slot is claimed by `fetch_or` on `dispatcher_claim` and released by `InstallGuard`, so a `ChunkCache` written by A is read by B. No memory ordering is given for claim or release; no `unsafe impl Sync` (required for an array of `Cell` inside `PoolInner`) is named or justified. Relaxed claim/release leaves A's `give` and B's `take` with no happens-before edge — and the 4 KiB chunks those pointers name carry task-cell data across the handoff.
2. **`ChunkArena::carve(&self)`** advances `frontier` and `committed` as two independent atomics with no protocol. `VmReservation::commit` is `&self` and debug-asserts a monotonic granule-aligned frontier (`vm.rs:199-209`) — not a concurrent API; a thread using a page before another's `VirtualAlloc(MEM_COMMIT)` lands takes an access violation.
3. P11 says "`ChunkCache` … no atomics — nothing to model". The **claim protocol is the synchronisation**, and it is exactly what loom would have to cover. C2's own red-first test is a Miri data-race test and will not exercise a claim/release ordering bug.

**Consequence**: at W=16 with two concurrent installers — a shape the tree exercises (`tls.rs:648`, `scope.rs:1811`, pass-1 evidence) and reaches in production via `Schedule::run` plus any other installer — B can `take()` a pointer whose chunk is not visible, or two threads carve into an uncommitted page. Pass 1's C2 blocked because it corrupted memory across installing threads; the replacement moves the same hazard to a shared cache and frontier, and this time the plan says there is nothing to verify.

**Confidence**: CONFIRMED (absence traced in plan text; `thread_pool.rs:243-298, 347`).

**What is needed**: state the ordering of every `dispatcher_claim` operation and of `frontier`/`committed`, with the argument for each; state `ChunkCache`'s `Sync` contract and who may touch a slot claimed by another thread; put the claim/release handoff **in** the loom model list.

### C4. G6 proves the modding requirement by comparing two builds that are identical by construction
**Where**: P12, G6 checks (b), (c), (d).

The arms are "the crate present but unused by the binary" and "the crate removed from the workspace". A crate outside the binary's dependency closure is not compiled into it, so **the kernel's code is the same code in both arms**. (b) `.text` size of four hot functions, (c) `size_of::<EcsMaster/ComponentPool/Scope>()` "pinned equal across both arms", and (d) identical census numbers therefore compare a build to itself. The cost the requirement bounds is the **seam the kernel carries in both arms** — K-MOD-1's runtime-`Layout` entry points, K-MOD-5's non-generic `register_new` sibling, and whatever a later rung adds to keep them reachable. A branch, field or indirection added for mods stays green in three of four checks. Only (a), the absolute no-`boyko_mod_*`-symbol check, survives.

**Consequence**: the owner's newest hard requirement ships with a proof that cannot observe a violation of it — this repository's own catalogued "gate that could not fail" / "green from emptiness" class, applied to the requirement that motivated §7.

**Confidence**: CONFIRMED (read off G6's mechanism column).

**What is needed**: compare against a **pre-seam baseline pinned in the commit that lands the seam** — the four `.text` sizes and three `size_of`s as constants, re-blessed only with a written reason. Keep the two-arm build if you want it to catch cargo feature unification, but say that is what it catches.

---

## 🟡 Important

**W1. W2's defect recurs one level up: the only route to the `FrameArena` re-serialises it.** P4's "The one way to obtain `&FrameArena` is `EcsMaster::frame(&mut self) -> &FrameArena`" reborrows `&mut self` for the returned reference's lifetime, so while any `FrameVec<'f>` lives, **no other method of `EcsMaster` may be called — not even `&self` ones**. The Frame class went `&self` + `Cell` precisely so rung 2's CSR two-pass could hold two `FrameVec`s; that pass reads the hierarchy while it writes offsets, and that is now what does not compile. Options: `frame(&self)` with the owner `debug_assert` doing the work (consistent with the existing manual `unsafe impl Sync`, `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:1287-1288`); a split-borrow accessor; or show the rung-2 CSR pass written against the snapshot-then-allocate rule.

**W2. HV-3 is asserted over ~88 sites with no per-site proof, and rung 1e already contradicts P4.** HV-3 is enforced by "the scheduler (EM2)" plus a `debug_assert`, while `HeapRef` is declared `Send + Sync` so `HeapVec` inherits `Send` — the type system permits what HV-3 forbids. One destination already fails: rung 1e sends `traverse_iter.rs:51, 284` scratch to `FrameVec`, but `DescendantsIter::new(world: &'w EcsMaster, root: Entity)` is `pub` and takes `&EcsMaster` (`crates/boyko_ecs/src/ecs/core/iters/query/relation/traverse_iter.rs:281-294`; `VisitedSet.words: Vec<u64>` at `:49-51`), i.e. reachable from any non-exclusive system on a worker — which P4 explicitly excludes. Consequence: two parallel systems walking relations bump the same `FrameArena.cur` (`Cell`, `!Sync`) and get overlapping allocations at W ≥ 2; the `Heap` variant tears the class heads; in release the `debug_assert` is gone. Add an HV-3 ledger column discharged per site, and name which class serves **worker-thread per-call scratch**.

**W3. The overflow-spin liveness proof assumes the park decision is atomic**, in the subsystem whose sleep latch is an open defect. "A worker parks only when the queues are empty — which contradicts full" is the lost-wakeup shape: a worker that has decided the queues are empty can park after the pusher fills the ring. Consequence: at low W this is a hang, not a slowdown, and since the overflow counter is pinned at 0 in the census the path is never exercised outside the loom arm. Name the park protocol's linearisation point and model park/unpark in the forced-overflow arm.

**W4. `LaneDeposit._pad → cache_slot` needs a save/restore that `install` does not have.** The layout pins do survive (`crates/boyko_threadpool/src/tls.rs:139-141, 147-152, 159-161`), but `install` saves only two fields today: `prev_labels = (current_worker_id(), lane())` (`thread_pool.rs:252-257`). A nested `install`, or an `install` on a same-pool worker, rewrites `wid` and would now also rewrite `cache_slot`, with nothing said about restoring it or about the outer frame's held chunks — a second route into C3's handoff hazard.

**W5. A heap handle inside a component value collides with the tree's clone/prefab path.** `HeapVec` carries the identity of one `Heap`, and §1 rules heaps are per owner. This tree has a per-component clone path with `CopyBytes` (memcpy blob→row) and `CloneFnBytes` (`clone_fn(src_row, blob_slot)`) at `crates/boyko_ecs/src/ecs/core/clone/prefab.rs:75-80`, plus `tests/clone_entity.rs`; rev 2 mentions neither. Consequence: a prefab captured in world A and instantiated in world B yields `Children` rows naming A's reservation — dropping A releases the VA while B's rows still point into it; a `CopyBytes` classification is a plain double free within one world. State the clone/serialize contract, or take rev-2 Open question 4's hint and keep heap identity out of component values.

**W6. The modding seam states neither the component-id budget nor the unload contract.** The id space is hard-capped at `MAX_COMPONENTS = 512` (`core/iters/component_set.rs:11`), materialised as inline `columns: [Column; MAX_COMPONENTS]` per archetype (`core/archetype/archetype.rs:140`) and debug-asserted at pool construction (`memory/component_pool.rs:280`), while K-MOD-5's counter is a monotonic `fetch_add` — so a load/unload/reload cycle burns ids permanently out of the engine's own budget, with no overflow or reload policy. And K-MOD-3's "unloading a mod is `Heap::drop` = 2 `VirtualFree`" states no precondition: entities still carrying that mod's components will run its `drop_fn` against released VA.

---

## 🟢 Optional

- **O1.** `large_free: [RunFree; MAX_RUNS]` lives in page 0; `MAX_RUNS` is never given, nor the behaviour when the run table is full (~480 entries at 4 KiB). Low impact after rung 2 empties the large tier, but a constant and a policy cost one line.
- **O2.** The sanitize live-bit map is "1 bit per 16-B unit of the small reservation" — at the release VA of 1 GiB that is **8 MiB of committed bitmap per heap** in a native `mem-sanitize` build, and its "page 1.." placement collides with `small_frontier`'s page-1 start. Size it from the committed frontier, or state that `mem-sanitize` implies the Miri reserve constants.
- **O3.** G6(a) prescribes `objdump -t`. The toolchain here is windows-gnu (binutils present); on a msvc leg the check needs a different tool, and a check that silently does not run is C4's class again.

---

## Positive — preserve these

1. **P0 is the best thing in rev 2.** Withdrawing the throughput justification against a measured null and replacing it with three that survive it (the ruling; the *tail* rather than the mean; G3's unreachability) is exactly right. Verified: mi/sys 0.998 / 1.023 / 0.992 at W1/8/16, ±5 % band at W8, sign flip (1.005) on reversed arm order (`docs/unification/checkpoint-2026-09-11/allocator-ab.md:117-123`). The M-A6 overturn gate with explicit >1 % / <0.3 % thresholds is the right shape.
2. **C2 closed by deletion, not repair.** The re-derivation checks out: `ScopeBlock` is inline in `Scope`, stated in the file itself (`block.rs:6-7`), so there is nothing left to key by `wid`.
3. **The class ladder is now correct — I checked the arithmetic rather than taking it.** `class(0)=0`; `1..=256 → (size+15)>>4` giving 1..=16; `>256 → 17 + ceil_log2(size) − 9` giving 17 at 512 and 24 at 64 KiB, so `CLASS_COUNT = 25` is exact with no gap and no underflow. The `0 = empty` sentinel is safe by placement because the header occupies page 0.
4. **P11's TB-1 correction.** Finding that bumpalo keeps a mutable `Cell` footer *inside* its chunks and ships Miri-clean — and therefore that TB-1 is a concurrency rule, not an aliasing-model rule — is a genuine, non-obvious refutation of rev 1's own stated reason. Keep rule and reason together; the reason is what tells a later reader when the rule does not apply.
5. **P3's protector-kind argument.** That a foreign *write* invalidates under both protector kinds while a *deallocation* is UB only under a strong one is the correct reason the poison write is strictly stronger than the `dealloc` it replaces, and merging the two counters with a written note is the right handling of a gate whose event no longer exists.
6. **P6's ledger gate.** Syntax over text, the measured holes turned into a fixture suite that must be *detected*, and three-sided anti-vacuity including "a crate that silently stops parsing reads as clean". The floors verified against ledger rev 4 on disk.
7. **K-MOD-4 is true, and it is the load-bearing fact of §7.** `ComponentPool::new(component_id: usize, reserve_rows: usize)` is already non-generic and resolves layout, `drop_fn` and `type_id` from the registry (`crates/boyko_ecs/src/ecs/memory/component_pool.rs:279-288`). There really is no second path for a mod to duplicate — it is the *proof* (C4) that is wrong, not the claim.
8. **The writer's flag on O3, and O5's retirement with its reason.** Both apply the repository's own discipline to itself: an unresolved contradiction recorded as one, and a dropped obligation recording that it was dropped on purpose and what brings it back.

---

## Open questions for the architect

1. **(Rev-2 Q1, `ScopeShared` at the head of the first chunk.)** Free, and provable rather than judged: `required = 256 + (128 − 64) = 320`, so `min_e = 0` and the first chunk stays 4 KiB (`block.rs:405-407, 455-456`). It costs ~320 bytes of a 4 KiB chunk and no exponent. M-A2 is the right home.
2. **(Rev-2 Q2, `D = 8`.)** Agreed it is a guess. The quantity to count is "threads concurrently inside an `install` frame on one pool"; the census can report it. Note `W + D ≤ 64` for one `u64`, so `W ≤ 56` is a pool-size cap that should be asserted at `ThreadPool::new`, not discovered at `fetch_or`.
3. **(Rev-2 Q3, `syn` in a test target.)** I concur, for the reason given — a measured blind-spot risk beats an unmeasured one, and `harness` keeps it out of shipped binaries. Residual hole to name in the fixture suite: `syn` sees `#[cfg]` as text and cannot resolve feature-gated fields or cross-crate aliases, so that must be an explicit decision rather than an accident.
4. **(Rev-2 Q4, `Children` as a relation.)** Well founded, and W5 is an independent second reason: a heap handle in a component value must survive clone, prefab and serialize, and the tree has all three. The relation form removes W5 with it.
5. **(Rev-2 Q5.)** Confirmed as blocker C1, with a second instance you did not list (no route from the header to `commit`).
6. **Mine — what is the `Heap`'s story for a `HeapVec` that outlives its world in a test harness?** §1 rejects a singleton heap because tests build many `EcsMaster`s. Under shape (b) a `HeapVec` that survives its `Heap` writes into released VA; `Heap::drop`'s sanitize assert catches it only under `mem-sanitize`, and HV-1 is otherwise enforced by struct field declaration order alone. Is a field-order pin owed, and where?
7. **Mine — does G2 keep its meaning after rung 1a+1b?** P9 is careful that the counter sees only the Rust heap and that `VirtualAlloc(MEM_COMMIT)` is invisible. After C2 is resolved, both the `ChunkArena` and the `Heap` frontier are invisible growth. Should the census gain a second pin over *committed bytes*, so that "0 acquisitions" cannot be reached by moving growth where the counter cannot see it?

---

# Rev 2.1 (2026-09-16)

**Scope.** A patch against rev 2 (Part III), closing the four pass-2 blockers (C1-C4), every important and optional row of Part IV, and the critic's two new open questions. Part I and Part III stay as written; each change below names the rev-2 text it removes. The critic's "preserve these" list is not touched — P0, P3, P6, P7, P9, P11's TB-1 rule *and* its corrected rationale, and P12/K-MOD-4 stand as rev 2 wrote them.

---

# Allocator design — Rev 2.1 (patch against Rev 2)

**Convention.** Per change: the section by heading, the text **removed** quoted verbatim, the text **added**, and the sections whose invariants the change depends on. Rev 2's untouched text stands. Tree for every citation: `D:/wt/joltab` (`merge/ke16-into-ecsnative`, HEAD `d552be05`, read-only) unless tagged `[main]` = `D:/claude/BoykoEngine`. Ledger on disk read as **rev 4** (`[main] docs/memory/RUNTIME-DATA-LEDGER.md`), as the critic states.

**What changed, one line each.** C1 ruled: eager *reserve*, lazy *commit*, header self-sufficient via a `raw::commit_at` free function — the `as_ref(&mut self)` exit is rejected by name (P15). C2's defect class deleted, not patched: the depth-1 cache becomes an **unbounded per-slot intrusive free list**, and `give()` disappears (P16). C3: full ordering, `Sync` contract and loom list; `carve` drops to **one** atomic with a self-contained per-carve commit (P17). C4: G6 re-founded on **absolute pre-seam pins** (P18). Plus W1/W2/W5/Q4 collapsed into one structural ruling (P19), W3 (P20), W6 (P21), O1-O3 (P22), critic's questions 6-7 (P23).

---

## P15 (closes C1) — the O3/P1 contradiction is RULED, and the header gains a self-contained reserve/commit route

**Depends on:** P1 (HEAP-1, `HeapRef` = reservation base), P11 TB-1 (as amended), §2.2 (the D1 commit-page relaxation), §5.1 drop order, `vm.rs:109, 168-179, 184, 199-209`, `constants.rs:7` (`COMMIT_GRANULE = 64 * 1024`). ⚠ *Rev 2.5 (P46): P15's Heap half (15.1–15.6) is the Heap's revival form (U-1); `raw::commit_at` survives as KC-01's single commit route.*

### 15.1 The ruling

**Exit (i): `Heap::new` RESERVES both regions eagerly (VA only, no commit charge) and commits page 0; every later commit is lazy and is driven from inside the reservation.** Exit (ii) (`as_ref(&mut self)`) is rejected by name: it reinstates a `&mut`-derived tag at the exact site C1 exists to close, and the critic's prediction — that a developer picks the exit that compiles and lands a green test whose mutation no longer describes the defect — is the whole reason the choice is made here rather than at the keyboard. Exit (iii) is variant (a), already rejected in P1.

The constraint that made exit (i) look expensive is **one line of `vm.rs`'s Miri arm** (`vm.rs:176`, `alloc_zeroed` over the full `os_len`), not the `Heap`'s shape. It is priced and cut in 15.4, not paid.

**Removed** (P8, the WRITER'S FLAG paragraph, verbatim):

> ⚠ **WRITER'S FLAG, for the critic (not an architect's decision): O3 and P1 do not agree, and the disagreement is load-bearing.** P1's `Heap` struct declares `small: VmReservation` / `large: VmReservation` — plain, not `Option` — while O3 requires "0 **reserved** until the first alloc", which needs an `Option` filled on the first allocation. But the first allocation runs through `HeapRef::alloc(self, ..)`, and a `HeapRef` carries only the reservation base: it cannot create the reservation it is the base of, and `Heap::as_ref(&self)` has nothing to hand out before one exists. The three exits are (i) reserve eagerly in `Heap::new` and commit lazily — address space is free on the release arm, and the Miri arm's cost is then exactly what the 4 MiB constants bound; (ii) keep the `Option` and make `as_ref` `&mut self`, which re-opens C1's retag question at the one site C1 exists to close; (iii) keep the `Option` behind interior mutability in the `Heap` struct, which is variant (a) that P1 rejected. **The writer did not choose** — the text above is as the architect wrote it, and this paragraph is the record that it contains a contradiction rather than a decision.

**Added:**

> **RULED (rev 2.1): exit (i).** `Heap::new` performs exactly **two `MEM_RESERVE` / `PROT_NONE` reservations and one `COMMIT_PAGE` commit** (page 0, the header). `VmReservation` stays plain — no `Option`, no interior mutability, no `&mut self` accessor. The `Heap` struct is therefore still write-once after `new`, so HEAP-1 holds unchanged: a `&mut EcsMaster` retag over it disables nothing anyone later writes through.
>
> Why this is the cheapest of the three on every metric that differs:
>
> | | (i) eager reserve, lazy commit (RULED) | (ii) `Option` + `as_ref(&mut self)` | (iii) `Option` behind interior mutability |
> |---|---|---|---|
> | C1 closed | yes — no handle derives from a `&mut` | **no** — reopened at the one site | only under TB's byte-precise `ReservedIM` rule, which is model-version-dependent and opt-in (P1's own argument) |
> | release-arm cost | 2 × `VirtualAlloc(MEM_RESERVE)` + 1 page commit = **4 KiB resident, 2 GiB VA, ~3 µs, once per world** | same minus 4 KiB | same |
> | Miri/fallback cost | `os_len` eagerly `alloc_zeroed`ed (`vm.rs:168-179`) — **bounded by 15.4's constants** | deferred, then identical | identical |
> | branch on the alloc path | **none** — the reservation always exists | `if res.is_none()` on every alloc | same |
> | `HeapRef` self-sufficiency | complete (15.2) | not needed, because the handle is not the allocator | incomplete |
>
> O3's "0 **reserved** until the first alloc" is **withdrawn for the Heap class only** and replaced by "0 **committed** beyond the header page". It stands unchanged for Frame (P19 keeps it lazy) and is restated for `ChunkArena` in P17. **Overturn gate:** M-A7 (Miri RSS of the existing suite) measuring a per-`EcsMaster` delta above **2 MiB** against the pre-campaign baseline — that is the number at which eager reservation costs more than the branch it removes, and the answer then is a smaller Miri constant, not exit (ii).

### 15.2 The header can commit, and can see its own VA bound

**Removed** (P1, the `HeapHeader` struct in the Added code block, verbatim):

> ```rust
> #[repr(C, align(64))]
> struct HeapHeader {
>     large_base: *mut u8,      // the large reservation's base, so a free needs no `Heap`
>     small_frontier: u32,      // next unassigned 4 KiB page, in pages
>     small_committed: u32,     // commit frontier, in pages
>     large_frontier: u32,      // in granules
>     large_committed: u32,
>     large_free_len: u32,      // entries in the run table that follows
>     _pad: u32,
>     free: [u32; CLASS_COUNT], // per-class intrusive LIFO head, unit = 16 B from `small_base`; 0 = empty
>     // 100 B of heads at CLASS_COUNT = 25; the hot classes (16..64 B) share one line.
>     #[cfg(debug_assertions)] owner: u64,   // owning thread, debug only
>     // followed in page 0 by: large_free: [RunFree; MAX_RUNS]  (offset_granules: u32, len_granules: u32)
>     // followed at page 1.. under cfg(any(miri, feature = "mem-sanitize")) by the live-bit map,
>     //   1 bit per 16-B unit of the small reservation: the FLECS_SANITIZE analogue.
> }
> ```

**Added:**

> ```rust
> // COMMIT_PAGE is this design's D1 constant (§2.2), NOT COMMIT_GRANULE.
> // `constants.rs:7` pins COMMIT_GRANULE = 64 KiB, which is the Windows
> // *reservation* granularity; MEM_COMMIT inside an existing reservation is
> // PAGE-granular [D VirtualAlloc], and mprotect likewise. That fact is what
> // makes D1 sound, and it is stated here because both C1 and C2 rest on it.
> pub const COMMIT_PAGE: usize = 4096;
>
> /// Reservation-resident bookkeeping. Page 0 of the small reservation, committed
> /// by `Heap::new` and never uncommitted. NOTHING holds a reference to this type
> /// — it is reached only as `base.cast::<HeapHeader>()` through a raw pointer
> /// read out of `HeapRef`.
> ///
> /// Line 0 is everything the COLD paths need (limits, frontiers, the large base)
> /// and is touched once per page assignment; line 1.. is `free[]`, which the
> /// WARM path touches on every alloc and free. The split is the hot/cold rule
> /// applied inside one 4 KiB page: an alloc that hits a free list reads exactly
> /// one line.
> #[repr(C, align(64))]
> struct HeapHeader {
>     // ---- line 0: COLD. Read on page assignment, on a large alloc, and on drop.
>     large_base: *mut u8,      // the large reservation's base: a free needs no `Heap`
>     small_pages: u32,         // VA BOUND of the small reservation, in COMMIT_PAGEs.
>                               //   THIS is what C1 was missing: the header's own
>                               //   `os_len`, so `alloc` can detect VA exhaustion
>                               //   (Q4's promise) without naming the reservation.
>     large_granules: u32,      // VA bound of the large reservation
>     small_frontier: u32,      // next unassigned page. committed == frontier by
>                               //   construction (15.3), so rev 2's `small_committed`
>                               //   field is DELETED, not renamed.
>     large_frontier: u32,      // in granules; likewise committed == frontier
>     large_free_len: u32,      // live entries in the run table that follows
>     #[cfg(debug_assertions)] owner: u64,        // owning thread id (HV-3)
>     #[cfg(debug_assertions)] live_allocs: u32,  // 15.5: Heap::drop asserts 0
>     #[cfg(any(miri, feature = "mem-sanitize"))] live_map: *mut u8, // O2, P22
>     // ---- line 1..: WARM
>     free: [u32; CLASS_COUNT], // per-class intrusive LIFO head, unit = 16 B from
>                               //   the small base; 0 = empty (safe by placement:
>                               //   offset 0 is the header itself)
>     // followed in page 0 by: large_free: [RunFree; MAX_RUNS]   (P22/O1)
> }
> ```
>
> **The commit route, and the one `vm.rs` change it needs.** `VmReservation::commit(&self, old, new)` is a method on the reservation (`vm.rs:199`), which the header does not hold. The whole of that method's body is a function of `(base, os_len, old, new)`. So `boyko_memory::raw` gains
>
> ```rust
> /// # Safety: `base`/`os_len` name a live reservation this caller owns;
> /// `[old, new)` is non-empty, COMMIT_PAGE-aligned and within `os_len`.
> pub unsafe fn commit_at(base: NonNull<u8>, os_len: usize, old: usize, new: usize);
> ```
>
> and `VmReservation::commit` becomes a two-line wrapper over it, **keeping its own monotonic-frontier `debug_assert` at the wrapper** (`vm.rs:200-209`) because that assert is a property of its *column* callers, not of the syscall. `commit_at` asserts non-empty, in-bounds, and `COMMIT_PAGE`-aligned — the granule assert at `vm.rs:201-204` relaxes to the page per D1, which §2.2 already commits this design to. The three per-OS SAFETY blocks (V-CMT-W / V-CMT-U / fallback no-op) move verbatim with the body; no new `unsafe` argument is minted.
>
> This is also the choke point G2b needs (P23), and the reason it is a *function* rather than a duplicated body.

### 15.3 `HeapRef::alloc`, cold path — the steps C1 says cannot be written

**Added** (new sub-section under P1's "Why this is not slower than rev 1"):

> **Cold path (class `c` has an empty free list), in full:**
>
> 1. `hdr = self.0.cast::<HeapHeader>()` — no load; the header IS the base.
> 2. `p = hdr.small_frontier` (1 load, line 0).
> 3. `if p == hdr.small_pages { heap_small_va_exhausted(label) }` — `#[cold] #[inline(never)]`, a loud panic naming the heap; **this is Q4's promise, now implementable.**
> 4. `raw::commit_at(self.0, hdr.small_pages as usize * COMMIT_PAGE, p * COMMIT_PAGE, (p+1) * COMMIT_PAGE)` — one page, one syscall.
> 5. `hdr.small_frontier = p + 1` (1 store). **Committed == frontier by construction**, which is why the second counter is deleted.
> 6. Thread the page into `free[c]`: `COMMIT_PAGE / size_class(c)` links, a forward sequential write over one page (4 KiB streaming, no reads, prefetcher-friendly).
> 7. Return the head.
>
> Complexity O(page/class); frequency: once per 4 KiB of that class's high-water. Branches: 2, both predicted-not-taken after the first frame. Page-at-a-time rather than granule-at-a-time is deliberate: it makes step 5 trivially monotone, and it keeps the **resident floor at 4 KiB per class touched**, which is the number the packing plan exists to get down to. A heap growing 1 MiB pays 256 setup-time syscalls once, ~20-40 µs, outside the steady window.
>
> `large` is the same shape at granule units, with `large_base` read from line 0 and `large_granules` as the bound.

### 15.4 Miri/fallback constants, re-derived against the eager reserve

**Removed** (P8, O3, first two sentences, verbatim):

> **O3 — Miri residency.** Both `Heap` reservations are **lazy** (0 committed and 0 reserved until the first alloc), like Frame and Scope, because the Miri fallback arm eagerly `alloc_zeroed`s the full `os_len` (`vm.rs:168-179`). Miri arm constants: Heap small 4 MiB, large 4 MiB, Frame 4 MiB, ChunkArena 1 MiB per pool.

**Added:**

> **O3 — Miri residency, re-derived under 15.1's ruling.** The Heap's two reservations are **eagerly reserved and lazily committed**; on the fallback arm reserve == resident (`vm.rs:168-179`), so the Miri constants ARE the per-`EcsMaster` cost and are cut to match what Miri tests actually touch:
>
> | Reservation | Release VA | Miri / fallback | Derivation of the Miri number |
> |---|---|---|---|
> | Heap small | 1 GiB | **1 MiB** | 256 pages: 1 header + up to 25 class pages + growth. A Miri test allocating past 255 pages is not a Miri test. |
> | Heap large | 1 GiB | **256 KiB** | 4 granules. The large tier's client list is a `HeapVec` above 64 KiB; under Miri there are none today. |
> | sanitize live map (own reservation, P22/O2) | small_VA / 128 | **8 KiB** | 1 bit per 16 B of 1 MiB. |
> | FrameArena, per slot | 64 MiB | **0** (stays LAZY — P19) | it has no reservation-resident header, so laziness costs it no branch on the handle path |
> | ChunkArena, per pool | 1 GiB | **2 MiB** | eager at `ThreadPoolBuilder::build`; P17 explains why lazy is a race here |
>
> **Per-`EcsMaster` eager Miri cost: 1.26 MiB** (against the critic's feared 8 MiB, and against the 6 MiB per-pool eager footprint already shipped at `[ecsnative] constants.rs:61-67`). **Rung-time obligation, unchanged:** M-A7 measures the existing suite's Miri RSS before these constants are trusted, and a constant is lowered — never raised — to fit it.

### 15.5 HV-1 gets a mechanism (critic's open question 6)

**Added** (new bullet in §5.3, after the drop-order bullet):

> - **HV-1 is enforced twice, because field declaration order is not checkable from a layout.** `-Zmiri-randomize-layout` (P11) makes `offset_of!` say nothing about drop order, so an offset pin would be a gate that cannot fail.
>   1. **Source-level (the real gate).** G1's `syn` scanner (P6) already parses every struct's field list *in declaration order*. It gains one assertion: in any struct declaring a `Heap` field, that field is declared **after** every field whose type is in the heap-client set (`HeapVec`/`HeapBox`/`HeapDyn`/`HeapString`/`SortedMap`, plus any struct transitively containing one — resolvable because the scanner already indexes every struct in the workspace). Red-first: move `heap` up one position in `EcsMaster` → red. Cost: zero, the parse already happened.
>   2. **Runtime backstop, for the test-harness case §1 names** (many `EcsMaster`s, a `HeapVec` outliving its world). `#[cfg(debug_assertions)]` `live_allocs` in the header: `+1` per `alloc`, `-1` per `free`, and `Heap::drop` `debug_assert!(live_allocs == 0, ...)`. This fires on the leak *at the point of release*, in every debug build, not only under `mem-sanitize`. Release builds pay nothing (the field is `cfg`'d out).

### 15.6 C1's red-first test, amended

**Removed** (P1, "C1's red-first Miri test", the RED-first mutation cell, verbatim):

> **one line**: move `free: [u32; CLASS_COUNT]` out of `HeapHeader` and into `struct Heap`, and make `HeapVec.heap` a `NonNull<Heap>`. Expected: `error: Undefined Behavior: write access through <TAG> ... is forbidden`, protected/parent tag born at the `&mut owner` retag, accessed tag at `HeapVec::drop`. The test file records the exact diagnosis text of the mutated run, per the tree's gate convention (`scope.rs:206-214` is the precedent).

**Added:**

> **Two mutations, because 15.1 created a second way to lose the property.**
> - **M1 (rev 2's, kept):** move `free: [u32; CLASS_COUNT]` out of `HeapHeader` into `struct Heap` and make `HeapVec.heap` a `NonNull<Heap>`. Expected: `error: Undefined Behavior: write access through <TAG> ... is forbidden`, protected/parent tag born at the `&mut owner` retag, accessed tag at `HeapVec::drop`.
> - **M2 (new):** change `Heap::as_ref(&self)` to `as_ref(&mut self)` and take the handle from a fresh `&mut owner.heap` between the two sibling operations. Expected: the same diagnosis with the protected tag born at the `as_ref` argument. **M2 is the mutation that describes the defect a developer would actually introduce** — it is exit (ii) — and without it the file's green would survive the substitution the critic predicted.
> Both record the exact diagnosis text of the mutated run (`scope.rs:206-214` is the precedent). Anti-vacuity unchanged (≥ 5 drops counted).

---

## P16 (closes C2) — the chunk return path is a named unbounded structure; `give()` and the depth-1 cache are deleted

**Depends on:** `block.rs:105, 109, 116, 176, 190-191, 292-350, 400-499` (the ladder `e = min_e.max(i)` at `:455`, `cap = CHUNK0 << e` at `:456`, `dealloc` at `:341`, `alloc` at `:482`), P11 TB-1 (the free-list-link exemption), P17 (the concurrency protocol), §2.2/P15 (`COMMIT_PAGE`).

### 16.1 The defect, restated so the fix is checkable

The critic is right on all three counts and the third is the decisive one: `give() -> bool` had no defined "keeps it", `ChunkArena` had no field a returned chunk could land in, and "the arena frontier's free list" named nothing. On top of that, the depth-1-per-exponent cache **cannot** serve the shipped nesting — `schedule.rs:466` installs and `par_iter.rs:341` / `par_chunk.rs:139` open a nested scope inside it (verified in joltab) — so the outer scope's return always meets a full slot, and `e = min_e.max(i)` (`block.rs:455`) puts every chunk from index 4 upward outside `EXP_CACHED = 4` permanently.

**The answer is not a deeper cache.** A free chunk can hold its own link word in its own first 8 bytes at zero cost — TB-1, as P11 restates it, permits exactly this ("the intrusive free-list link exists only while a chunk is off every scope and on a free list"). An intrusive list has **no capacity**, so "full" ceases to be a state and the orphan class ceases to exist, the same way C2's original class was removed rather than repaired.

### 16.2 The structures

**Removed** (P2's Added code block, verbatim):

> ```rust
> /// The pool's chunk source. ONE reservation per `ThreadPool`, replacing
> /// `std::alloc::alloc` at block.rs:482 and `dealloc` at block.rs:341.
> /// Bump-only; chunks come back to a per-slot cache, never to the frontier.
> pub(crate) struct ChunkArena {
>     res: VmReservation,        // 64 MiB VA (Miri 1 MiB), LAZY
>     frontier: AtomicUsize,     // bytes carved; fetch_add, ONE per chunk that was never cached
>     committed: AtomicUsize,    // commit frontier; advanced under a #[cold] path
> }
>
> /// One per CLAIMED slot. Depth 1 per exponent class.
> /// EXP_CACHED = 4 covers 4 / 8 / 16 / 32 KiB; a bigger chunk is returned to the
> /// arena frontier's free list (#[cold], and the census shows 1.74 chunks/scope,
> /// i.e. exps 0 and 1 dominate).
> #[repr(C, align(64))]
> pub(crate) struct ChunkCache {
>     slots: [Cell<*mut u8>; EXP_CACHED],   // 32 B
>     // 32 B of implicit tail padding to the 64-byte alignment. O2: NO explicit
>     // `_pad` field — `align(64)` already pads, and rev 1's `_pad: [u8; 24]` on a
>     // 48-byte body would have produced 72 B rounded to 128 B, i.e. two lines.
> }
> const _: () = assert!(size_of::<ChunkCache>() == 64);   // one line per slot, exactly
> ```

**Added:**

> ```rust
> /// The pool's chunk SOURCE. One reservation per `ThreadPool`, replacing
> /// `std::alloc::alloc` (block.rs:482). CARVE-ONLY BY DESIGN: a carved chunk
> /// belongs to one slot forever and is recycled through that slot's free list,
> /// so the frontier advances only while the pool's PEAK CONCURRENTLY-LIVE chunk
> /// bytes grow — a process-lifetime high-water, not a per-frame rate. That is
> /// the difference between rev 2's four-minute exhaustion and this.
> ///
> /// Eagerly reserved at `ThreadPoolBuilder::build` (VA only). NOT lazy: `carve`
> /// takes `&self` from several threads, so a lazy first-touch would be a race
> /// with no owner to serialise it (P17).
> pub(crate) struct ChunkArena {
>     res: VmReservation,        // 1 GiB VA (Miri 2 MiB), eager reserve, 0 committed
>     frontier: AtomicUsize,     // bytes carved. ONE atomic: each carve commits its
>                                //   own disjoint range, so there is no second
>                                //   counter and no protocol between two (P17).
> }
>
> /// Every chunk this pool ever carved belongs to exactly one slot, and a slot's
> /// chunks live on ONE INTRUSIVE LIFO PER EXPONENT. The link word is the chunk's
> /// own first 8 bytes and exists only while the chunk is off every scope
> /// (TB-1's exemption, P11), so the list costs no memory and no allocation and
> /// has NO CAPACITY — "cache full" is not a state, and rev 2's `give() -> bool`
> /// is deleted rather than defined.
> ///
> /// EXP_MAX is `block.rs`'s MAX_CHUNKS, not a number of its own: the ladder
> /// `e = min_e.max(i)` with `i < MAX_CHUNKS` (block.rs:455) can produce any
> /// exponent below MAX_CHUNKS, so a shorter table would orphan exactly the
> /// chunks rev 2 orphaned. Drift is a build failure.
> #[repr(C, align(64))]
> pub(crate) struct SlotChunks {
>     heads: UnsafeCell<[*mut u8; EXP_MAX]>,   // 256 B = 4 whole cache lines
>     #[cfg(debug_assertions)] owner: AtomicU64, // P17's claim protocol, checked
> }
> const EXP_MAX: usize = 32;
> const _: () = assert!(EXP_MAX == boyko_threadpool::block::MAX_CHUNKS_RECEIPT);
> const _: () = assert!(size_of::<SlotChunks>() % 64 == 0); // whole lines: no false
>                                                           // sharing between slots
> // UnsafeCell, not Cell: the table lives in `PoolInner`, which is `Sync`, and the
> // exclusivity is external (P17 states it and justifies the `unsafe impl Sync`).
> ```
>
> The type is **renamed** `ChunkCache` → `SlotChunks` everywhere in the plan, because "cache" names a structure with a capacity and a miss policy and this has neither.

### 16.3 The API

**Removed** (P2's Added API block, verbatim):

> ```rust
> // Scope (threadpool-internal; NOT public API, and NOT reachable by a mod — see §7)
> impl ChunkArena {
>     pub(crate) fn new() -> Self;                                  // no reservation yet
>     pub(crate) fn carve(&self, exp: u8) -> *mut u8;               // #[cold]; fetch_add + maybe commit
>     pub(crate) unsafe fn release_all(&self, caches: &[ChunkCache]);// ThreadPool::drop, after the join
> }
> impl ChunkCache {
>     #[inline] pub(crate) fn take(&self, exp: u8) -> *mut u8;      // 1 load, 1 store, 1 branch; null = miss
>     #[inline] pub(crate) fn give(&self, exp: u8, p: *mut u8) -> bool; // false = cache full, caller keeps it
> }
> ```

**Added:**

> ```rust
> // Scope (threadpool-internal; NOT public API, NOT reachable by a mod — §7/K-MOD-7)
> impl ChunkArena {
>     pub(crate) fn new(va: usize) -> Self;          // reserves; commits nothing
>     /// #[cold] #[inline(never)]. Runs ONCE per chunk that has never existed.
>     /// `fetch_add(cap, Relaxed)` then `commit_at` over its OWN disjoint range.
>     /// Panics loudly (named, #[cold]) on VA exhaustion — the `chunk_table_
>     /// exhausted()` precedent already in block.rs:429.
>     pub(crate) fn carve(&self, exp: u8) -> *mut u8;
> }
> impl SlotChunks {
>     /// # Safety: the caller owns this slot (P17). 2 loads, 1 store, 1 branch.
>     #[inline] pub(crate) unsafe fn take(&self, exp: u8) -> *mut u8;  // null = empty
>     /// # Safety: the caller owns this slot AND the chunk, which is off every
>     /// scope. INFALLIBLE — no return value, because there is no full state.
>     #[inline] pub(crate) unsafe fn give(&self, exp: u8, p: *mut u8);
> }
> ```
>
> `take`: `h = heads[exp]; if h.is_null() { return null } heads[exp] = *(h as *mut *mut u8); h`. `give`: `*(p as *mut *mut u8) = heads[exp]; heads[exp] = p`. Both are 64-bit loads/stores on one of four lines of a slot-private structure; `block.rs`'s `grow` calls `take` and falls through to `carve` on null (`#[cold]`), `free_all` calls `give` in place of `dealloc` (`:341`).
>
> **Nothing is ever returned to the arena**, so `release_all` disappears: `ThreadPool::drop` releases the reservation as a whole after the join, which frees every chunk in one `VirtualFree`. That is strictly simpler than the per-chunk walk rev 2 specified, and it removes a "free every chunk exactly once" question from a path that runs during teardown.

### 16.4 The claims rev 2 made that this invalidates, corrected

**Removed** (P2, "Nested scopes on one thread are correct by construction", verbatim):

> **Nested scopes on one thread are correct by construction**, and this is the property C2 demanded: two nested scopes on the same thread share one `ChunkCache`, but the outer scope takes its chunk first and returns it last, so the depth-1-per-exponent cache serves depth-1 nesting at a 100 % hit rate and deeper nesting at today's cost. There is no cursor to alias because there is no arena cursor — each block keeps its own `cur`/`end` in its own frame.

**Added:**

> **Nested scopes on one thread are correct by construction, and the argument no longer depends on depth.** The shipped nesting is `install` (`schedule.rs:466`) with a nested `scope` inside it (`par_iter.rs:341`, `par_chunk.rs:139`), and both run on the same thread and therefore the same slot. With an intrusive list the outer scope's return **is** a push onto a list that already holds the inner's chunk; order is LIFO and irrelevant. Depth-`k` nesting warms the list to `k` entries at the first frame and hits every frame after. There is no cursor to alias because there is no arena cursor — each block keeps its own `cur`/`end` in its own frame (`block.rs:190-191`).

**Removed** (P2, "`Box<ScopeShared>` is emplaced in the block", bullet 4, verbatim):

> 4. An empty scope now takes one chunk where it took none — and that chunk is a **cache hit**, not an allocation. Measured target: `install` goes from 1 acquisition (`block.rs:196-199`'s "a scope that spawns no task makes no allocator call at all" plus the `Box`) to **0**.

**Added:**

> 4. An empty scope now takes one chunk where it took none (`block.rs:196-199`: "a scope that spawns no task makes no allocator call at all"). After the first frame that chunk comes off the slot's exp-0 list — **a list pop, not an allocation and not an OS call**; on the very first frame of a slot's life it is one `carve` (one `fetch_add` + one page commit). Measured target: `install` goes from 2 process-heap acquisitions to **0**, with the first-frame carve landing in the census's setup window, not its steady window. The claim rev 2 made — "a cache hit, not an allocation" — was true only under a cache that could not be full; it is restated here as a property of the structure that replaced it.

### 16.5 Residency and the VA ceiling, re-derived

**Removed** (P2's Added Scope row of §2.1, the last two cells, verbatim):

> | `free_all` returns chunks to the claimed slot's cache | **no** | at join, unchanged | within a scope, unchanged | pool-wide: `(W + D) × 60 KiB` cached + peak live; at W=16, D=8 → **1440 KiB (1.41 MiB) per pool** |

**Added:**

> | `free_all` returns chunks to the owning slot's per-exponent intrusive list | **no** | at join, unchanged | within a scope, unchanged | pool-wide = **peak concurrently-live chunk bytes**, a process high-water. Derived: a slot's live set is its deepest nesting × chunks per scope; the shipped shape is install (1 chunk, exp 0) + one nested `par_iter` scope (1-2 chunks, exps 0-1), so ≤ 4 chunks ≈ 16 KiB per slot, plus 256 B of head table. **At W=16, D=8: ≤ 390 KiB per pool.** PINNED, not asserted: the census gate gains `chunk_bytes_resident` per scene (P23/G2b), and this row is re-derived from it rather than defended by this paragraph. |

**Added** (new paragraph under P2):

> **The VA ceiling is real, is 256× larger than rev 2's, and fails loudly.** Rev 2's 64 MiB was a *rate* budget consumed by orphans; here it is a *peak* budget, so the constant is set against the worst legitimate peak rather than against a leak: **1 GiB**, matching `InlandStore`'s in-tree precedent (`inland_store.rs:5`). At 4 KiB chunks that is 262,144 concurrently-live chunks — a state in which `std::alloc` would be holding the same gigabyte. Exhaustion is a `#[cold]` named panic, the treatment `block.rs:429`'s `chunk_table_exhausted()` already gives the neighbouring unrecoverable case. **Overturn gate:** any suite or scene reaching the panic; the answer is then to raise the constant with the measured peak written beside it, never to fall back to `std::alloc` (a fallback would make `free_all`'s sink ambiguous, which is C2's defect in a new costume).

### 16.6 `mem-shake` is re-derived (it would otherwise exhaust the arena)

**Removed** (P11's Added §5.2 row, the canary clause, verbatim):

> Canary, re-derived because there is no per-worker arena base to offset: `cfg(feature = "mem-shake")` makes `ChunkCache::take` **miss with probability 1/2** from a per-slot xorshift, so every scope's chunk address changes run to run.

**Added:**

> Canary, re-derived twice — once because there is no per-worker arena base to offset, and once because a forced *miss* would now carve a chunk that is never reclaimed, growing the arena by one chunk per shake and exhausting the Miri constant in a long test: `cfg(feature = "mem-shake")` makes `SlotChunks::take` **return the list's SECOND entry** (rotating the head) with probability 1/2 from a per-slot xorshift, carving only when the list holds fewer than two. The address handed out changes run to run; the retained set grows by at most one chunk per exponent per slot.

---

## P17 (closes C3) — the cross-thread protocol, stated: ordering, `Sync`, ownership, and the loom model that covers it

**Depends on:** P16 (`SlotChunks`, single-atomic `carve`), `thread_pool.rs:100-159` (`PoolInner`), `:243-298` (`install`), `:347-358` (`InstallGuard`), `tls.rs:126-142, 147-152, 159-161` (`LaneDeposit`, its three layout pins, `DETACHED`), `worker.rs:139, 383-386, 494-513` (the park/wake protocol).

### 17.1 Ownership: who may touch slot `s`, and how a thread learns `s`

**Added** (replaces nothing; new sub-section, and it is what rev 2 left unstated):

> **Rule SC-1 — a slot index is meaningful only relative to a pool.** `SlotChunks` tables live in `PoolInner`, so a worker of pool P that installs on pool Q must NOT index Q's table with P's worker id. Rev 2's `cache_slot: u32` alone cannot express this; the pair `(pool, cache_slot)` can, and `LaneDeposit` already carries `pool` at offset 0 (`tls.rs:130`, the field the three layout pins are stated over). The rule is therefore: **`cache_slot` is valid iff `LaneDeposit.pool == the pool being used`.**
>
> **Rule SC-2 — who owns what.**
>
> | Thread | Slot for pool P | Written when | Released |
> |---|---|---|---|
> | a worker of P | its own `wid` ∈ `[0, W)` | `worker_main` entry, once | never (thread lifetime) |
> | any thread inside `install` on P **that does not already own a slot of P** | a bit claimed from `PoolInner.dispatcher_claim: CachePadded<AtomicU64>`, searched in `[W, W+D)` | `install` entry | `InstallGuard::drop`, on return **and on unwind** |
> | a thread already owning a slot of P (a worker of P installing on P; a nested `install`) | **keeps it, claims nothing** | — | — |
> | a thread that could claim none (all `D` bits set) | `NO_CACHE_SLOT` | — | — |
> | `pool.scope(..)` (`par_iter.rs:341`, `par_chunk.rs:139`) | the caller's existing slot | never — `scope` touches no TLS (`thread_pool.rs:309-322`) | — |
>
> The third row is what makes nested `install` safe **and** is W4's fix: the write is conditional on not already owning, so there is no overwrite to save. The one thing that must still be saved is the case where the frame *did* claim — handled in 17.3.
>
> **`W + D ≤ 64`** for one `u64`. With `D = 8` that is `W ≤ 56`, and it is asserted at `ThreadPoolBuilder::build` with a named panic rather than discovered at `fetch_or` (the critic's open question 2; `MAX_WORKERS` already exists at `thread_pool.rs`, so the assert is one line against it).

### 17.2 Memory ordering, every operation, with its argument

**Added:**

> | Operation | Ordering | Why exactly this |
> |---|---|---|
> | claim: `dispatcher_claim.fetch_or(bit, Acquire)` (loop over clear bits, ≤ D iterations, then `NO_CACHE_SLOT`) | **Acquire** | It must synchronise-with the previous owner's release of the SAME slot, because what the new owner then reads — `heads[]`, and the chunk bytes the previous owner wrote through them — was written before that release. |
> | release: `dispatcher_claim.fetch_and(!bit, Release)` | **Release** | Publishes every write the frame made to its slot's heads and chunks. |
> | the edge itself | release sequence | Both operations are **RMWs on one atomic**. A later RMW always reads the latest value in modification order and is part of the release sequence headed by the `Release` fetch_and, so the `Acquire` fetch_or synchronises-with it `[D C++/Rust atomics: release sequence includes RMWs by any thread]`. This is the *entire* cross-thread argument for the chunk handoff, and it is why the claim word must not be split into per-slot words. |
> | `ChunkArena.frontier.fetch_add(cap, Relaxed)` | **Relaxed** | It establishes *uniqueness of the range*, nothing else. The carving thread then commits and first-writes its own range; no other thread can reach that chunk except through the slot handoff, which already carries the edge above. A thread whose `fetch_add` passes the VA bound does not use the range and takes the `#[cold]` panic — the over-advance is harmless because the frontier is monotone. |
> | commit | none needed | **There is no `committed` atomic.** Each carve commits exactly `[p, p+cap)`, disjoint from every other carve's range by the `fetch_add`. `raw::commit_at` (P15) is called by the same thread that will first write the page. This deletes C3(2) entirely: no two-counter protocol, no monotonic-frontier assert violated (that assert stays on `VmReservation::commit`, which this path does not call). |
> | `SlotChunks::{take, give}` | **non-atomic** | Correct *because* SC-1/SC-2 give the slot one owner at a time and the claim pair orders the handoff. This is the same discipline as `ScopeBlock`'s `Cell` fields, which are sound for the same reason. |
>
> **`unsafe impl Sync for SlotChunks`, with the invariant it rests on:** *at any instant at most one thread may index a given `SlotChunks`; workers own `[0, W)` for their thread's lifetime and installers hold an exclusive bit of `dispatcher_claim` for `[W, W+D)`; the claim's Release/Acquire pair orders the previous owner's writes before the next owner's reads.* A `#[cfg(debug_assertions)] owner: AtomicU64` in the struct is written on claim and checked on every `take`/`give` — the runtime witness for the invariant the `unsafe impl` asserts.

### 17.3 `install` saves and restores one more word (closes W4)

**Removed** (P2, "The identity fix, in one word of existing padding", second bullet, verbatim):

> - **Zero extra TLS accesses on the spawn path.** The slot is already read once per lane query (D7, `tls.rs:113-115`); `Scope::new` already resolves the lane to compute `joiner_wake` (`scope.rs:585-589`). The claimed index rides in the same read.

**Added:**

> - **Zero extra TLS accesses on the spawn path.** The slot is already read once per lane query (D7, `tls.rs:113-115`); `Scope::new` already resolves the lane to compute `joiner_wake` (`scope.rs:585-589`). The claimed index rides in the same read, and SC-1's pool check reads `LaneDeposit.pool`, which that same read already returned.
> - **The frame saves and restores it (W4).** `install` today saves exactly `(current_worker_id(), lane())` (`thread_pool.rs:252-257`) and `InstallGuard` restores the pair on return and on unwind (`:347-358`). That tuple becomes a **triple** — `prev_labels: Option<(u32, u16, u32)>` = `(wid, lane, cache_slot)` — and `InstallGuard` gains `claimed_bit: u8` (`NO_CLAIM` sentinel). The guard's `Drop` releases the bit **iff this frame claimed it**, then restores the triple. The existing rationale at `:350-357` (one `Option` over a tuple, so "restored the id but not the lane" is inexpressible) extends verbatim to the third member, which is why the triple and not a fourth field.
> - **The outer frame's held chunks need nothing.** A nested `install` claims no slot (SC-2 row 3), so the outer frame's `heads[]` are untouched; and a chunk *in use* by an outer scope is not on any list, because `free_all` is what puts it there.

### 17.4 `LaneDeposit`'s padding word, and the pins

**Added** (amends P2's identity-fix section):

> `_pad: u32` at offset 20 becomes `cache_slot: u32`; `DETACHED` seeds `NO_CACHE_SLOT = u32::MAX` (`tls.rs:147-152`). All three layout pins survive unchanged and must be re-run, not re-derived: `size_of == 2 * size_of::<*const ()>() + 8` (`:159`), `align_of == align_of::<*const ()>()` (`:160`), `offset_of!(LaneDeposit, pool) == 0` (`:161`). No new `thread_local!` (checkpoint defect 7).

### 17.5 P11's "nothing to model" is withdrawn, and the loom list is stated

**Removed** (P11's Added block, final bullet, verbatim):

> - **loom** — `Heap`, `FrameArena` and `ChunkCache` have no atomics (single writer / single slot owner); `ChunkArena.frontier` has one `fetch_add` and one `#[cold]` commit, modelled. The injector ring (1d) uses the pool's existing `cfg(loom)` shim (`[merge] sync.rs:1-15` for the contract, `:63-89` for the two re-export arms — note `:31-36` is the "deliberately **not** shimmed" list and is not the shim). Mandatory models: push/steal, steal/steal, overflow-spin.

**Added:**

> - **loom — and the sentence rev 2 had here was the defect, not a summary of it.** Saying "`ChunkCache` … no atomics — nothing to model" pointed at the wrong object: the atomics that make `SlotChunks` sound are in the **claim protocol**, and that is exactly what a model must cover. ~~`Heap` and `FrameArena` genuinely have no cross-thread protocol (single owner, P15/P19) and stay unmodelled, with that stated as a property of their ownership rather than of their field types.~~ ⚠ *Rev 2.5 (P51, pass-6 O1): no class outside the claim protocol, the carve and the injector is left with a cross-thread protocol to model — the Frame class is deleted (P34) and the Heap is not built (U-1); the Heap's revival form keeps its single-owner argument (HV-3, P15/P19), stated as a property of its ownership rather than of its field types.* Mandatory models, using the pool's existing `cfg(loom)` shim (`[merge] sync.rs:1-15` for the contract, `:63-89` for the two re-export arms — `:31-36` is the "deliberately **not** shimmed" list and is not the shim):
>   1. **claim/handoff (the C3(1) shape):** thread A claims slot s, `give`s a chunk, releases; thread B claims s, `take`s it, writes it. Assert B observes A's chunk contents and the head chain. **Red-first: weaken either side to `Relaxed` → loom must report the missing edge.** This is the model that decides whether the Acquire/Release argument in 17.2 is real.
>   2. **claim exclusivity:** `D+1` threads racing `fetch_or`; at most one holds each bit; the `(D+1)`-th degrades to `NO_CACHE_SLOT` and stays correct.
>   3. **carve:** two threads `fetch_add` concurrently; ranges disjoint; each commits its own; the VA-bound check fires exactly once per over-advance.
>   4. injector ring (1d): push/steal, steal/steal, **and overflow-spin with park/unpark** (P20).
> - **Miri does not subsume this and must not be asked to.** C2's red-first test is a data-race test; a claim/release ordering bug is a *missing synchronisation edge* that Miri detects only if the schedule happens to expose it. The loom model is the gate; the Miri test is the witness that the shapes it models are the shapes the tree runs (`tls.rs:648`, `scope.rs:1811`, `schedule.rs:466`).

---

## P18 (closes C4) — G6 compares against a pre-seam baseline pinned as constants, not against a build of itself

**Depends on:** P12 (the nine seam entries), P6 (the `syn` scanner, reused), G2's re-pinning discipline (P9).

### 18.1 Why the two-arm build cannot see a violation

The critic's mechanism is exact: a crate outside the binary's dependency closure is not compiled into it, so checks (b), (c) and (d) compared a build to itself. The quantity the owner's requirement bounds is **the seam the kernel carries in both arms** — K-MOD-1's runtime-`Layout` entry points, K-MOD-5's non-generic registration sibling, K-MOD-10 (new, P21), and whatever a later rung adds. That is a *longitudinal* quantity, so the baseline must be longitudinal.

**Removed** (P12, G6 rows (b), (c), (d), verbatim):

> | (b) identical codegen | build the same commit twice, once with the crate present but unused by the binary and once with it removed from the workspace; `.text` size of the kernel's named hot functions (`Schedule::run`, `ComponentPool::push`, `HeapRef::alloc`, `ScopeBlock::bump`) compared symbol-by-symbol | add a `cfg!(feature = "modding")` branch to any of the four → red |
> | (c) no kernel growth | `size_of::<EcsMaster>()`, `size_of::<ComponentPool>()`, `size_of::<Scope>()` pinned equal across both arms | add an `Option<ModRegistry>` field → red |
> | (d) no startup work | the frame-allocation census (G2) reports **identical** setup and steady numbers in both arms | construct anything at boot in the off arm → red |

**Added:**

> | Check | Mechanism | Red-first canary |
> |---|---|---|
> | (b) **no kernel code growth across the seam** | `.text` size of four kernel symbols, pinned as **absolute constants measured on the PARENT of the seam-landing commit** and written into `tests/mod_seam_pins.rs` in that same commit. ~~The four are chosen so the pin has a guaranteed subject — **every pinned symbol is `#[inline(never)]` or `#[cold]` at its definition**, because an `#[inline]` function may leave no symbol at all and an absent symbol would read as a skip: `Schedule::run`, `ComponentPool::new` (`component_pool.rs:279`, non-generic), `ScopeBlock::grow` (`block.rs:425`, `#[inline(never)]` — this replaces rev 2's `ScopeBlock::bump`, which is `#[inline]` at `block.rs:368` and may not exist), `HeapRef::alloc_cold` (P15's `#[cold]` page-assign path).~~ ⚠ *Rev 2.6 (P52, AP7 W1): superseded by P29 (`:2609-2625`) and P49 (`:4561`). The pinned set is `ComponentPool::grow_rows`, `ComponentPool::commit_subregion`, `run_check_ticks_scan`, `ScopeBlock::grow`; `Schedule::run` and `ComponentPool::new` carry no inline attribute and are not pinned, and `HeapRef::alloc_cold` is never built (U-1)* **A missing symbol is RED, never a skip.** | add a `cfg!(feature = "modding")` branch, a `Layout` parameter, or a registry lookup to ~~any of the four~~ any symbol of P29's pinned set ⚠ *Rev 2.5 (P51, pass-6 O1): named by P29's rule, not by a count — four after P49* → red |
> | (c) **no kernel growth** | `size_of::<EcsMaster>()`, `size_of::<ComponentPool>()`, `size_of::<Scope>()` pinned as **absolute numbers** from the same parent commit (the tree already pins struct sizes and already has a re-bless process for them — `rust-toolchain.toml:24-25`) | add an `Option<ModRegistry>` field → red |
> | (d) **no startup work** | the census (G2) setup+steady numbers of the modding-absent build pinned as **absolute constants** from the parent commit | construct anything at boot in the off arm → red |
> | (e) **the seam cannot grow silently** (NEW) ⚠ *Rev 2.5 (P47): superseded by P33/33.2 and P38.3; now UG-15 leg (5)* | G1's `syn` scanner (P6 — the parse already happens) asserts that the set of kernel items carrying `#[doc(hidden)] pub` + the `mod-seam` marker attribute equals the named list K-MOD-1..K-MOD-10, **exactly** (an extra entry is red, a missing entry is red) | add a tenth erased entry point without a ledger row → red |
> | (a) no exported symbol | **unchanged**, with its tool decided in 18.2 | add a `#[no_mangle] pub extern "C" fn` to `boyko_ecs` → red |
> | (f) **cargo feature unification / graph leakage** (the two-arm build, RELABELLED) ⚠ *Rev 2.5 (P47): = UG-15 leg (6), its only role* | the two-arm build is KEPT — crate-present-but-unused vs crate-absent — and its true job is written down: it catches a mod crate that enables a feature of a shared dependency, or that pulls the kernel into a different codegen unit set. It **cannot** see a seam the kernel carries in both arms, and the table says so where a reader will look | enable a non-default feature of a shared dep from `boyko_modding` → red ⚠ *Rev 2.7 (P57, AP8 O2): proposed as UG-15 leg (6)'s red control (xiii) in 03 §6. It goes through a normal, host-matching dependency that `boyko_demo` shares, and the `cargo tree -e features` clause is the one that must go red. `boyko_modding` reads as each of the three modding crates (`05:60`)* ⚠ *Rev 2.7 closure (P58.5, AP9 W3): leg (6)'s feature clause is a per-package difference between the arms, and (xiii) goes red on that difference, not on the tree naming the modding crate* |
>
> **Re-bless discipline, borrowed verbatim from G2.** A pin moves only with a written reason in the same commit. Two legitimate reasons exist and are named: a toolchain bump (all rows re-derived at once, with the version in the header — the tree already lives with this, `rust-toolchain.toml:24-25`) and a deliberate kernel change unrelated to the seam. "It moved and I widened it" is the failure the G2 header already forbids and this file repeats the sentence.
>
> **Honest cost.** `.text` pins are brittle across codegen changes and will produce re-bless churn. That is accepted *because* the alternative measured zero: a check that compares a build to itself is this repository's catalogued "gate that could not fail", and the requirement it guards is the owner's newest.

### 18.2 The tool (closes optional O3)

**Removed** (P12, G6 row (a), mechanism cell, verbatim):

> `objdump -t` on `boyko_demo` built **without** `boyko_modding`: zero symbols matching `boyko_mod_*` / `^mod_api_`

**Added:**

> `llvm-nm` / `llvm-objdump` from the **`llvm-tools` rustup component** (toolchain-provided, present on windows-gnu and windows-msvc alike, so the check does not change tool per host) on `boyko_demo` built **without** `boyko_modding`: zero symbols matching `boyko_mod_*` / `^mod_api_`. The same tool serves (b)'s `.text` sizes. **If the tool is absent the check FAILS, it does not skip** — a check that silently does not run is (b)'s own defect class, and the census's `running 0 tests` precedent is why this sentence is here.

---

## P19 (closes W1, W2, W5, and rev-2 open question 4) — one structural ruling instead of three discipline rules

**Depends on:** P4 (`&self` + `Cell`), P1 (HV-1..HV-3), `ecs_master.rs:122-127, 1283-1292`, `traverse_iter.rs:49-51, 281-294`, `prefab.rs:75-80`, the ledger's relation form.

### 19.1 The frame arena is PER SLOT, and `frame` takes `&self`

W1 is correct and its fix cannot be `frame(&mut self)`. But `frame(&self)` on a single global arena makes W2 worse, because `DescendantsIter::new(world: &'w EcsMaster, root: Entity)` is `pub` and takes `&EcsMaster` (`traverse_iter.rs:281-294`), i.e. reachable from a worker. One change answers both.

**Removed** (P4, the "API change" paragraph, last sentence, and the following paragraph, verbatim):

> The one way to obtain `&FrameArena` is `EcsMaster::frame(&mut self) -> &FrameArena`, so access is gated by the exclusive world borrow that only a dispatcher-run exclusive system holds; N `FrameVec`s may be live from that one shared reference.
>
> **Non-exclusive systems do not get the frame arena.** A param-based system's per-frame scratch stays `Local<ScratchColumn>` — the tree's existing shape and already alloc-free after warm-up (census: every system body measured 0).

**Added:**

> **The `FrameArena` is per thread slot — one per `(W + D)` slot, exactly as `SlotChunks` is — and `EcsMaster::frame(&self) -> &FrameArena` resolves the caller's slot from the single `LaneDeposit` read the tree already pays (D7, `tls.rs:113-115`).** Three problems collapse into one structure:
>
> 1. **W1 dissolves.** `frame(&self)` reborrows `&self`, so N `FrameVec<'f>`s may be live beside any other `&self` method — rung 2's CSR two-pass (offsets in one `FrameVec`, flat data in another, while the hierarchy is read) compiles.
> 2. **The compile-error property W1 wanted is kept.** `reset` is `EcsMaster::frame_reset_all(&mut self)`; `'f` borrows `*self`, so a `FrameVec` alive across a reset is a **borrowck error**, not a rule. This is bumpalo's `&mut`-for-reset argument `[S bumpalo lib.rs]` applied one level up.
> 3. **W2's missing destination appears.** Worker-thread per-call scratch — `traverse_iter.rs:51, 284`'s `VisitedSet.words: Vec<u64>` reached through a `pub fn` taking `&EcsMaster` — allocates from *its own slot's* arena. No cross-thread bump, no `Cell` shared between threads, nothing for HV-3 to forbid. `FrameVec` holds a `NonNull<T>` and is therefore `!Send` by construction, so it cannot escape to another thread.
>
> Safety argument, in the shape the tree already uses: each slot's arena has exactly one live user at a time by the SAME rule that gives `SlotChunks` one owner (P17/SC-1, SC-2), so `Cell` interior mutability is sound per slot; `EcsMaster`'s manual `unsafe impl Sync` (`ecs_master.rs:1287-1288`) gains bullet **SEND11** naming the slot rule, exactly as P4 specified for the single-arena form. The `debug_assert` moves from "owner == dispatcher" to "the resolved slot is this thread's slot", which is a *checkable* statement rather than a scheduling claim.
>
> Cost: one TLS read per `frame()` call (callers hoist `&FrameArena` and allocate N times from it), 24 × 64 MiB = 1.5 GiB of VA at W=16 (VA is free; residency stays per-slot high-water, 0 until used — the Frame class stays **lazy**, because it has no reservation-resident header and therefore no branch to remove). `reset_all` writes `W+D` cursors at the frame boundary, where no worker is running.
>
> **Overturn gate:** M-A8 (new) — if per-slot arenas show a resident total above **4 MiB per pool** in the App scenes, or if the TLS read shows in a `frame()`-heavy profile, the Frame class reverts to one dispatcher arena and `traverse_iter`'s scratch goes to `Local<ScratchColumn>` instead.

### 19.2 HV-4: no heap handle inside a component value — which removes W5 and answers open question 4

**Removed** (P1's Added code block, the HV-3 comment lines, verbatim): ⚠ *Rev 2.5 (P46): HV-3 and HV-4, with G1's HV-4 assertion and the `hv3-owner` column, belong to the Heap's revival form (U-1); `Children` as a relation (below) stands.*

> // HV-3 (NEW): push / pop / grow / Drop run ONLY on the heap's owner thread.
> //   Reads (`Deref`, `as_slice`, `iter`) run anywhere. Enforced by the scheduler
> //   (structural mutation is dispatcher-only, EM2) and checked by
> //   `debug_assert_eq!(header.owner, current_thread())` on every mutating entry.

**Added:**

> // HV-3: push / pop / grow / Drop run ONLY on the heap's owner thread. Reads
> //   (`Deref`, `as_slice`, `iter`) run anywhere. In DEBUG this is checked by
> //   `debug_assert_eq!(header.owner, current_thread())` on every mutating entry.
> //   In RELEASE it is held by HV-4, which is structural, NOT by the scheduler:
> //   "the scheduler enforces it" is a claim over ~88 sites that cannot be
> //   discharged one site at a time, and `HeapRef: Send + Sync` means the type
> //   system permits what HV-3 forbids.
> // HV-4 (NEW, and it is what makes HV-3 provable): NO heap handle — `HeapVec`,
> //   `HeapBox`, `HeapDyn`, `HeapString`, `SortedMap`, or any struct transitively
> //   containing one — may appear in a COMPONENT VALUE, in a `Resource` reachable
> //   from a non-exclusive system, or in any type a worker can obtain `&mut` to.
> //   The owner set is then enumerable: dispatcher-owned structures only
> //   (Schedule, ScheduleBuilder, the registries, the EcsMaster tables). The
> //   ledger carries an `hv3-owner` column naming the owner of each heap row, and
> //   G1 refuses a row without one.
>
> **HV-4 is mechanically enforced by the gate that already parses these structs.** G1's `syn` scanner (P6) resolves derives; it asserts that no struct carrying `#[derive(Component)]` (or registered via `register_layout`) has a field whose resolved type is in the heap-handle set. Red-first: `#[derive(Component)] struct X { v: HeapVec<u8> }` → red.
>
> **W5 is removed with it, not answered.** The tree's per-component clone path (`prefab.rs:75-80`: `CopyBytes` memcpys blob→row; `CloneFnBytes` runs `clone_fn(src_row, blob_slot)`) would otherwise reproduce a handle naming world A's reservation into world B — a `CopyBytes` classification being a plain double free within one world. Under HV-4 there is no handle in a component value for either path to copy.

**Removed** (Part I §4 rung 2, the `Children` row, verbatim):

> | `Children(Vec<Entity>)` (`hierarchy/mod.rs:120`) | `Children(HeapVec<Entity>)`; mutation only through `EcsMaster` hierarchy APIs (dispatcher-only, heap single-writer) | — |

**Added:**

> | `Children(Vec<Entity>)` (`hierarchy/mod.rs:120`) | **a relation, not a component value** — the ledger's first-class kernel form, which stores the edge set in kernel storage and needs no per-entity handle. Forced by HV-4 (a heap handle in a component value is forbidden) and independently by W5 (clone/prefab/serialize would copy the handle). Rev 2's `Children(HeapVec<Entity>)` is **withdrawn**. | edge order is the relation's, pinned by the existing hierarchy tests |

**Removed** (rev-2 "Open questions for the critic", item 4, verbatim):

> 4. **`Children(HeapVec<Entity>)` may be the wrong destination entirely** — the ledger makes relations a first-class kernel form, and a `Children` that is a relation needs no `HeapVec` at all. Rev 2 keeps rev 1's assignment so the two documents do not silently diverge, and flags it as the one rung-2 row whose destination another plan may own.

**Added:**

> 4. **DECIDED (rev 2.1): `Children` is a relation.** Two independent reasons force it — HV-4 (19.2) and W5's clone/prefab path — so it is no longer a question and no longer a row another plan may re-decide. The rung-2 table carries the relation form; if the relations plan changes the edge storage, this row follows it rather than owning it.

---

## P20 (closes W3) — the liveness proof is re-derived from the push side, where the tree's protocol actually lives

**Depends on:** P10 (the bounded spin), `worker.rs:139` (`park()`), `:383-386` (`mark_idle`, `fetch_or(Release)`), `:494-513` (`unpark_one_idle_excluding`: `publish_fence()` then `idle.load(Acquire)`), `:594-596` ("the wake is unconditional … the StoreLoad barrier lives one level down").

**Removed** (P10, the liveness clause, verbatim):

> **Liveness proof for the spin, which is what replaces it:** a full ring means ≥ `cap` tasks are enqueued; every idle worker steals from the injector, and a worker blocked in a nested join also *executes* tasks from it (`join_on_worker`). The only state in which nothing drains is "every worker is parked", and a worker parks only when the queues are empty — which contradicts full.

**Added:**

> **Liveness proof for the spin, re-derived — the rev-2 form assumed the park decision was atomic, which is the lost-wakeup shape and would be a HANG, not a slowdown.** The correct argument runs on the push side, where the tree's protocol is:
>
> 1. A worker does not park on a decision; it runs **mark → re-poll → park**: `mark_idle` publishes its bit with `fetch_or(Release)` (`worker.rs:383-386`), *then* polls again, *then* `park()`s (`:139`).
> 2. Every successful push issues an unconditional wake, and that wake is `publish_fence()` (StoreLoad) followed by `idle.load(Acquire)` (`:494-496`, with the design note at `:594-596`).
> 3. Therefore a task published before a worker's final poll is seen by that poll, and a task published after it is seen by the pusher's `idle.load`, which observes the bit `mark_idle` released. This is the standard two-sided protocol and it has no lost-wakeup window — **that** is the fact the proof needs, not "a worker parks only when the queues are empty".
> 4. The ring being **full** means ≥ `cap` tasks were pushed successfully, each of which issued (2). So "ring full and every worker parked" requires a wake to have been lost, which (3) excludes. A worker blocked in a nested join additionally *executes* from the injector (`join_on_worker`), so it is a drainer, not a waiter.
> 5. **The one gap is the spinner itself**, which by definition published nothing and therefore issued no wake. Closed cheaply: **the `#[cold]` spin calls `unpark_one_idle` once per `Backoff` round**, so even a hypothetical missed wake is repaired within one round. The cost is one Acquire load per round on a path pinned at 0 occurrences.
>
> **Modelled in loom, in the forced-overflow arm, WITH park/unpark** (`LANE_CAP = 2` under `cfg(test)`): P17's list item 4. The census keeps the overflow counter pinned at **0**, so the path is measured cold rather than assumed cold — and because it is never exercised outside the loom arm, the loom arm is the only place this proof is checkable. Rung 1d lands **after** checkpoint defect 2 (the A1 sleep latch), per §6 decision 5; a spin proof written against a park protocol with an open defect would be proving the wrong protocol.

---

## P21 (closes W6) — the mod id budget and the unload contract

**Depends on:** P12 (K-MOD-3, K-MOD-5), `component_registry/mod.rs:63` (`MAX_COMPONENTS = 512`), `:214` (`NEXT_ID`), `:920-921` (`register_new::<T>`), `enable_tag_api.rs:55, 69` ("the shared `MAX_COMPONENTS` (512) ComponentId budget — shared with…"), `archetype.rs:140` (inline `columns`), `component_pool.rs:280`.

**Removed** (P12, K-MOD-5's Shape cell, verbatim):

> the id counter is `component_registry::register_new::<T>()` (`component_registry/mod.rs:920-921`, `NEXT_ID.fetch_add(1, Ordering::Relaxed)`) — the **same counter** the derive macro uses. It is generic today only to key the `TypeId` mint, so the additive seam is a non-generic sibling taking `(Layout, Option<DropFn>)`; the kernel never distinguishes the resulting ids, so it never branches on origin

**Added:**

> the id counter is `component_registry::register_new::<T>()` (`:920-921`, `NEXT_ID.fetch_add(1, Relaxed)`), generic only to key the `TypeId` mint. **The additive seam does NOT mint** — it registers at a caller-supplied id: `register_at(id: usize, layout: Layout, drop_fn: Option<DropFn>)`, `#[cold]`, non-generic, asserting `MOD_ID_BASE <= id < MAX_COMPONENTS` and that the slot is unoccupied (the collision detection already at `:918-919`). Reason, and it is a budget fact rather than a style preference: the id space is hard-capped at **512** (`:63`), materialised as an inline `columns: [Column; MAX_COMPONENTS]` per archetype (`archetype.rs:140`) and shared with enable-tags (`enable_tag_api.rs:55`), so a monotonic `fetch_add` would let a load/unload/reload cycle **burn the engine's own budget permanently**. Instead:
> - `MOD_ID_BASE = 384`. Ids `[0, 384)` are the engine's, minted by `NEXT_ID` as today with its exhaustion assert re-pointed at `MOD_ID_BASE` (one constant, a setup-time assert, no hot-path change); ids `[384, 512)` are the mod range, **owned and recycled by `boyko_modding`'s registry Resource** (K-MOD-8), not by the kernel.
> - The kernel therefore has **no** mod id policy, no free list, and no branch on id origin — the property K-MOD-4 buys, preserved.
> - **Cost when modding is absent: one changed constant in a cold assert.** G6(c)'s `size_of` pins are unaffected; (b)'s four symbols do not contain it.
> - **Overturn gate:** a scene needing more than 384 engine ids or more than 128 mod ids; the answer is then to raise `MAX_COMPONENTS`, which is an archetype-layout change and its own campaign.

**Removed** (P12, K-MOD-3's Shape cell, verbatim):

> **one `Heap` per mod**: `Heap::new()` → 2 lazy reservations. Unloading a mod is `Heap::drop` = 2 `VirtualFree`, which bounds a mod's leak at its own reservation and never touches the engine's free lists

**Added:**

> **one `Heap` per mod**: `Heap::new()` → 2 eager reservations, 1 committed header page (P15). Unloading is **four ordered steps, and `Heap::drop` is the LAST**, because a `Heap::drop` with entities still carrying that mod's components would run the mod's `drop_fn` against released VA: ⚠ *Rev 2.5 (P47): K-MOD-3 is removed (U-1; 05 §4) and K-MOD-10 is deleted (U-10, load-only); this unload sequence has no subject.*
> 1. remove the mod's component types from every archetype and drop their rows — **K-MOD-10 (NEW seam entry):** `EcsMaster::remove_component_type(id: ComponentId)`, `#[cold]`, non-generic, cost-free when absent (no caller ⇒ dropped at link; it is in G6(e)'s pinned inventory). **There is no such operation in the tree today** (searched `ecs_master/`: `despawn_without_children` at `entity_api.rs:837` is per entity, and nothing unregisters a type), so this is a genuinely new kernel entry and is on the critical path of "a mod can be unloaded at all"; ⚠ *Rev 2.5 (P47): deleted (U-10).*
> 2. drop the mod's `ComponentPool`s (releases their reservations);
> 3. `Heap::drop` — 2 `VirtualFree`, and its debug `live_allocs == 0` assert (P15/15.5) is what catches a missed step 1;
> 4. return the ids to `boyko_modding`'s free list.
> Steps 1-2 are kernel operations the engine already needs for world teardown; steps 3-4 are the mod's. The unload path is entirely `#[cold]` and entirely absent from a binary with no mod.

---

## P22 (closes optional O1, O2) — the run table's bound and policy; the sanitize map's home

**Depends on:** P15 (header layout, the third reservation), P1 (large tier). ⚠ *Rev 2.5 (P46): P22 is the Heap's revival form (U-1).*

**Removed** (P1's Added `HeapHeader` trailing comments — already quoted and removed in P15/15.2 — the clauses concerning `MAX_RUNS` and the live map placement, restated here for the record):

> // followed in page 0 by: large_free: [RunFree; MAX_RUNS]  (offset_granules: u32, len_granules: u32)
> // followed at page 1.. under cfg(any(miri, feature = "mem-sanitize")) by the live-bit map,
> //   1 bit per 16-B unit of the small reservation: the FLECS_SANITIZE analogue.

**Added:**

> **O1 — `MAX_RUNS = 448`, and the full-table policy is stated.** Page 0 holds a 64-B line 0, `free[25]` = 100 B, rounded to 192 B; the remaining 3904 B at 8 B per `RunFree` gives 488 entries, so **448** leaves headroom for a header line to grow without a re-layout. Behaviour when full, in order: (1) `#[cold]` single coalescing pass over the table, O(n) with n ≤ 448, merging adjacent runs — the large tier is granule-quantised so adjacency is an integer compare; (2) if still full, the freed run is **not recorded** (it is reclaimed at `Heap::drop` with the reservation) and a `#[cfg(debug_assertions)]` counter increments; (3) that counter is **pinned at 0** by the heap's unit suite. Justification for accepting (2): after rung 2 the large tier's client list is a `HeapVec` above 64 KiB, of which the inventory has none; a policy that leaks bounded VA in a state the gate says is unreachable is cheaper than a general free-space structure with no client.
>
> **O2 — the sanitize live map gets its OWN cfg-gated reservation.** Rev 2 put it "at page 1.." of the small reservation, which is where `small_frontier` starts assigning class pages — a collision, and at the release VA of 1 GiB the map would have been 8 MiB of committed bitmap per heap. Instead: a third `VmReservation`, created only under `cfg(any(miri, feature = "mem-sanitize"))`, sized `small_VA / 128` (1 bit per 16 B) and committed page-wise as `small_frontier` advances, its base in the header's `live_map` field (P15/15.2). Under Miri that is **8 KiB** (15.4). No collision is possible because no page frontier runs through it.

*(Optional O3 — the `objdump` toolchain hole — is closed in P18/18.2, not here.)*

---

## P23 (closes the critic's open questions 6 and 7) — the two mechanisms the campaign was missing

**Q6 — a `HeapVec` outliving its `Heap`.** Answered in **P15/15.5**: the field-order pin lives in G1's `syn` scanner (declaration order, which is what drop order actually follows and what `-Zmiri-randomize-layout` makes un-checkable from offsets), plus the debug `live_allocs == 0` assert in `Heap::drop` for the test-harness case §1 names.

**Q7 — does G2 keep its meaning after rung 1a+1b?** No, not on its own, and the critic's diagnosis is exact: after C1 and C2 both the `ChunkArena` frontier and the `Heap` frontier are growth the process-heap counter cannot see (`alloc_frame_census.rs` counts the Rust heap; `VirtualAlloc(MEM_COMMIT)` is invisible — G2's own coverage-boundary bullet, P9). "0 acquisitions" would then be reachable by moving growth out of the counter's sight, which is this campaign's own failure class applied to its own gate.

**Added** (new gate row in §3, after G2):

> | **G2b** | **Committed-bytes census** | with rung 1a | `raw::commit_at` (P15 — the single choke point every commit in the engine now passes through, which is *why* it is a function and not a duplicated body) increments a process-global `COMMITTED_BYTES: AtomicUsize` with a `Relaxed` `fetch_add`. The census gate gains a per-scene `commit_delta` measured over the steady window and pinned at **0**, beside the existing per-class acquisition pins, plus `chunk_bytes_resident` per pool (P16/16.5's residency number, pinned rather than argued). Cost: one relaxed RMW on a path that is already making a syscall — unmeasurable. | (a) commit a page from a system body → red on `commit_delta`; (b) **the anti-vacuity direction, which is the one that matters here**: the setup window must report a NONZERO `commit_delta`, or the counter is not wired to the commits it claims to count |

---

## Change log (rev 2 → rev 2.1)

| Row | Disposition | Where |
|---|---|---|
| **C1** | **CLOSED** — exit (i) ruled by name with a priced table; `HeapHeader` gains `small_pages`/`large_granules` (its own VA bound) and reaches the OS through the new `raw::commit_at`; `small_committed`/`large_committed` deleted because page-at-a-time assignment makes committed == frontier; Miri cost cut to 1.26 MiB/`EcsMaster`; HV-1 gets a source-order gate and a debug backstop; a second red-first mutation (M2) added that describes the exit a developer would take | P15 |
| **C2** | **CLOSED by deletion of the mechanism** — the depth-1 cache and `give() -> bool` are removed; the return path is an unbounded intrusive per-exponent LIFO per slot (`SlotChunks`), so "full" and "orphan" are not states; the arena is a peak budget at 1 GiB with a loud `#[cold]` exhaustion; residency re-derived to ≤ 390 KiB at W=16 and **pinned** by G2b; `mem-shake` re-derived so it cannot exhaust the arena | P16 |
| **C3** | **CLOSED** — SC-1/SC-2 ownership (slot is per-pool; a thread that already owns one claims nothing); Acquire/Release on `dispatcher_claim` with the release-sequence argument; `carve` drops to ONE atomic with a self-contained per-carve commit; `unsafe impl Sync for SlotChunks` with its invariant and a debug owner witness; P11's "nothing to model" **withdrawn** and replaced by four loom models, the first with its own red-first weakening | P17 |
| **C4** | **CLOSED** — G6 (b)(c)(d) re-founded on absolute pins taken at the seam-landing commit's parent; pinned symbols restricted to `#[inline(never)]`/`#[cold]` definitions so the pin cannot lose its subject (`ScopeBlock::bump` → `grow`); new check (e) pins the seam inventory via G1's scanner; the two-arm build kept and **relabelled** to the defect it can see; tool decided (`llvm-tools`, absent ⇒ red) | P18 |
| W1, W2, W5, Q4 | CLOSED by one structural ruling — the `FrameArena` is per slot with `frame(&self)`; **HV-4** forbids heap handles in component values, making HV-3 provable by enumeration instead of by ~88 site claims; `Children` becomes a relation, which removes the clone/prefab hazard with it | P19 |
| W3 | CLOSED — liveness re-derived from the push side's unconditional wake + `publish_fence`/Acquire pair; the spinner wakes once per `Backoff` round; park/unpark modelled in the forced-overflow loom arm | P20 |
| W6 | CLOSED — `MOD_ID_BASE = 384` with mod ids owned and recycled by `boyko_modding`, kernel keeps no policy and no branch; unload is four ordered steps and **K-MOD-10** (`remove_component_type`) is named as a new seam entry because the tree has no such operation | P21 |
| O1, O2 | CLOSED — `MAX_RUNS = 448` with a three-step full policy pinned at 0; the sanitize map moves to its own cfg-gated reservation, ending the page-1 collision and the 8 MiB release-VA figure | P22 |
| O3 (optional) | CLOSED — folded into G6's tool decision | P18 |
| Critic Q6, Q7 | CLOSED — declaration-order pin + debug `live_allocs`; **G2b** pins committed bytes at the `commit_at` choke point, with the anti-vacuity direction stated | P15, P23 |
| Critic Q1, Q2, Q3, Q5 | Q1 and Q3 accepted as the critic states them (no change owed); Q2's `W + D ≤ 64` becomes an asserted build-time cap (`W ≤ 56`) in P17/SC-2; Q5 is C1 and is ruled | P17, P15 |
| rev-2 P0, P3, P6, P7, P9, P11 (TB-1), P12/K-MOD-4 | **unchanged** — the critic's "preserve these" list is not touched by this patch; P11 loses only its final loom bullet, and its TB-1 rule *and* rationale stand as written | — |

## Open questions for pass 3

1. **`FrameArena` per slot multiplies a reservation by `W + D`** (P19). VA is free and residency is lazy, but it is `24 × 64 MiB` of VA per world at W=16 and a second per-slot table in the pool. M-A8 is the entry that would overturn it; I judge the W2 hole it closes worth more than the VA, and say so rather than leaving it implicit.
2. **`MOD_ID_BASE = 384` is a split of a fixed 512 budget, and I chose the split without a count of engine component types in the largest shipped scene** (P21). The number the census can report is "distinct `ComponentId`s registered in `boyko_demo` at steady state"; until it does, 384/128 is defended only by the loud assert on each side.
3. **`.text` pins will churn** (P18). I accept the churn because the alternative measured zero, but a reader should know the first toolchain bump after the seam lands will re-bless four rows at once, and that the re-bless is where the gate is weakest.

**Files read for this patch** (all read-only): `D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/memory/vm.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/constants.rs`; `D:/wt/joltab/crates/boyko_threadpool/src/block.rs`; `D:/wt/joltab/crates/boyko_threadpool/src/thread_pool.rs`; `D:/wt/joltab/crates/boyko_threadpool/src/tls.rs`; `D:/wt/joltab/crates/boyko_threadpool/src/worker.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/clone/prefab.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs`.

---

# Part V - Critique log - pass 3 (2026-09-16)

**Verdict of pass 3: CHANGES REQUESTED** - three blockers (C1-C3), four important remarks (W1-W4), two optional notes (O1-O2), eight preserved positives, four open questions for the architect. None is a re-litigation: two blockers are in text **rev 2.1 introduced** (P15's cold path, P21's recycling) and the third is in a rev-2 fix (P4's `&self`) that rev 2.1 built further on in P19 without revisiting its API surface.

Rules of this log, as in Parts II and IV:

- Parts III and Rev 2.1 above are **unchanged**. No finding is answered by silently editing rev 2.1; every answer lands in Rev 2.2 below and names the rev-2.1 text it removes.
- The log is reproduced **verbatim as the critic wrote it**, including its own headings, its confidence tags, its "preserve these" list and its open questions. The dispositions are in Rev 2.2's change log.
- Trees as the critic states them: code read in `D:/wt/joltab` (`merge/ke16-into-ecsnative`) unless marked; documents in `D:/claude/BoykoEngine`. Ledger on disk read as **rev 4**.

---

# Architecture review: allocator design space — Rev 2.1 (pass 3)

## Verdict

**[X] CHANGES REQUESTED** — 3 blockers, 4 important, 2 optional.

Rev 2.1 is a genuine close of pass 2. I traced each of C1–C4 rather than taking the change log: the exit-(i) ruling removes the contradiction *and* names the exit a developer would otherwise take (M2 mutation); `SlotChunks` deletes the orphan class instead of deepening the cache; P17's release-sequence argument is correct as stated (an acquire RMW reading a value in the release sequence headed by the `Release` RMW synchronises-with it — both are RMWs on one word, which is exactly why the claim word must not be split); P18 re-founds G6 on longitudinal pins. P20's liveness re-derivation checks out against the tree (`worker.rs:383` `mark_idle`, `:434` `publish_fence`, `:494-496` `idle.load(Acquire)`) — the mark → re-poll → park shape is really there.

The three blockers below are **not** re-litigations: two are in text rev 2.1 introduced (P15's cold path, P21's recycling), one is in a rev-2 fix (P4's `&self`) that rev 2.1 built further on in P19 without revisiting its API surface. All code read in `D:/wt/joltab` (`merge/ke16-into-ecsnative`); documents in `D:/claude/BoykoEngine`. Ledger on disk read as **rev 4**.

---

## 🔴 Critical (blockers)

### C1. `FrameArena::rewind` takes `&self`, so two live `FrameVec`s over the same bytes are reachable from safe code

**Where**: P4, "API change" paragraph (`ALLOCATOR-DESIGN-SPACE.md:992`): "`FrameArena::{alloc, alloc_slice_uninit, vec, rewind}` take `&self`; `reset(&mut self)` keeps `&mut`". `FrameMark` survives in P4's struct block (`:972`); P3 still poisons on `rewind` (`:925`); P19 does not touch it.

**Problem**: rev 1 had `rewind(&mut self)`, which made "rewind while a `FrameVec` is live" a borrowck error. W2's fix flipped the whole allocation surface to `&self` and carried `rewind` across with it. `FrameVec<'f, T>` borrows the arena **shared**, so this is safe, compiles, and is UB:

```
let a = arena.vec::<u32>(64);   // FrameVec borrows &arena
let m = arena.mark();
let b = arena.vec::<u32>(64);
arena.rewind(m);                // &self — a and b still alive
let c = arena.vec::<u32>(64);   // c aliases b
```

The `debug_assert!` LIFO check does not fire: the rewind *is* LIFO-legal; what is illegal is the live handle. bumpalo, which P4 cites as the precedent, deliberately exposes **no** `rewind` — only `reset(&mut self)` — for precisely this reason, so the citation supports the `&self` *alloc* and argues against the `&self` *rewind*.

**Consequence**: two concrete clients, and they are the only two that need `mark`/`rewind` at all. (a) Rung 2's UI frame lanes (`boyko_ui/resources.rs`, `pick.rs`, `focus.rs`) that clear-and-refill per system: a system that rewinds its own scratch while a CSR offsets vector is live silently writes two logical buffers into one range — release-build data corruption, no `debug_assert`, no Miri diagnosis (single thread, in-bounds writes, one allocated object). (b) If instead `rewind` is dropped to keep the API sound, P19's routing of worker per-call scratch to the slot arena has no reclamation inside a frame: `DescendantsIter`'s `VisitedSet` is `words: Vec<u64>` sized `entity_id/64` (`traverse_iter.rs:49-51`), so a 1M-entity non-`ACYCLIC` walk is 16 KiB per call, N calls per frame accumulate, and there is no trim policy (§6 decision 4) — the per-slot high-water becomes per-frame *total* traversal scratch × 24 slots, resident forever.

**Confidence**: CONFIRMED (plan text vs plan text; `traverse_iter.rs:49-51, 294` for the client).

**Why critical**: it is safe-API memory unsoundness in the primitive P14 step 3 writes first, in the one class whose handles are `!Send` and therefore look safe by construction.

**What is needed**: a ruling on how a released range is made unreachable rather than merely marked — the shape the tree already has for this is a *scoped* sub-region whose handles cannot outlive it (the `Scope`/`ScopeBlock` relationship), not a token. If `rewind` survives as an `unsafe fn`, say what discharges it per call site; if it does not survive, P19 point 3 owes a stated per-frame bound for worker scratch and a number for where that bound comes from.

### C2. The Heap's small tier has four classes (8/16/32/64 KiB) that its page-at-a-time cold path cannot serve

**Where**: P15/15.3 ("Cold path … in full", `:1580-1590`), steps 4-6: "`commit_at(… p*COMMIT_PAGE, (p+1)*COMMIT_PAGE)` — one page, one syscall", then "Thread the page into `free[c]`: `COMMIT_PAGE / size_class(c)` links". Class ladder from P1 (`:669-675`): `size > 256 → 17 + (ceil_log2(size) − 9)`, `CLASS_COUNT = 25`.

**Problem**: the ladder's top four classes are 8 KiB (c=21), 16 KiB (22), 32 KiB (23), 64 KiB (24) — every one larger than `COMMIT_PAGE = 4096`. Step 6 computes `4096 / 8192 = 0` links, so the free list stays empty and the cold path either loops or returns the head of an empty list. The large tier does not cover them: P1 puts `align > 64` and `size > 64 KiB` there, and P8/Q4 states the large tier's client list is empty after rung 2. There is no third path.

**Consequence**: `HeapVec` growth past 4 KiB has no defined allocation. That is not an exotic input — rung 3 sends `ScheduleBuilder`'s seven `Vec`s, `ConflictGraph.conflict_bits`, `HeapVec<HeapVec<TriggerEntry>>` and the `SortedMap` key/value pairs to this heap; a `HeapVec<u64>` of 1024 elements is class 21. A developer at the keyboard invents a multi-page assignment, and the numbers derived from the one-page form go stale with it: "resident floor 4 KiB per class touched" (15.3), 15.4's Miri derivation ("256 pages: 1 header + up to 25 class pages + growth"), and P8/Q4's per-`EcsMaster` floor.

**Confidence**: CONFIRMED (arithmetic on P1's stated formula against P15's stated step; the critic of pass 2 verified the same ladder at `:1429` and it is correct — the defect is in the *assignment*, not the ladder).

**Why critical**: P14 step 3 writes the Heap before any client, and C1's red-first Miri test lands with it; the primitive cannot be written as specified for a quarter of its class range.

**What is needed**: state the page-assignment quantum as a function of the class (and re-derive the two residency numbers and the Miri page count that were computed from "one page"), or state that classes above `COMMIT_PAGE` leave the small tier — in which case the large tier acquires the clients P8/Q4 says it does not have, and O1's `MAX_RUNS = 448` / no-coalescing policy must be re-argued against a non-empty client list.

### C3. P21's mod-id recycling is refused by the shipped registry, and unload leaves a permanently poisoned slot

**Where**: P21 (`:2029-2033`): "ids `[384, 512)` are the mod range, **owned and recycled** by `boyko_modding`'s registry Resource … The kernel therefore has **no** mod id policy"; K-MOD-6 (`drop_fn` thunk); the four-step unload at `:2041-2046`.

**Problem**: the component registry is **process-global and write-once**. `LAYOUTS: [OnceLock<ComponentLayout>; MAX_COMPONENTS]` (`component_registry/mod.rs:208-209`), `HOOKS` likewise (`:226-227`), and `register_new` panics when a slot is occupied by a different type (`:929-939`). `OnceLock` offers no reset reachable from a `static`. So:

1. **Recycling is not implementable.** An id returned to `boyko_modding`'s free list cannot be re-registered with a different mod's `Layout`/`drop_fn`; `register_at` must either refuse (so the id is burned anyway — W6's defect, moved one level down, not closed) or the kernel's `LAYOUTS` must become resettable, which changes the read `get_layout_unchecked(component_id)` that `ComponentPool::new` performs at `:284-288` — the exact function K-MOD-4 rests on, and the exact thing the owner's requirement says must not change for a no-modding build.
2. **The stale entry holds a code pointer.** After unload, `LAYOUTS[id]` retains the mod's `drop_fn` — a pointer into a module that is gone — forever, for an id that can never be overwritten. `was_ever_archetyped(id)` (`:899`) additionally freezes the id's hook state, so even a same-mod reload does not restore a clean slot.
3. The registry being process-global while `boyko_modding`'s registry is a per-world `Resource` (K-MOD-8) leaves the `[384, 512)` range with no arbiter across two worlds in one process — the shape §1 itself says the tests exercise ("tests build many `EcsMaster`s concurrently").

**Consequence**: a session is capped at 128 *distinct mod component types ever registered*, not 128 concurrently — the budget W6 asked to be protected is protected only on the engine's side. And any post-unload read of a mod id (a diagnostic dump over archetype metadata, a serialize pass, a `debug_assert` in `get_layout`) takes a code pointer into unloaded memory; I have confirmed the pointer is retained and unclearable, not that a reader is reachable today.

**Confidence**: CONFIRMED for (1) and the retention in (2) (`:208-209, 899, 929-939`); PLAUSIBLE for a reachable post-unload reader.

**Why critical**: this is the row that closed W6, and it closes it with a kernel capability the kernel refuses. It blocks §7/P21 and the owner's hard requirement, not rungs 1-4 — but the repair direction decides whether §7 stays additive, so it must be ruled before any seam entry is written.

**What is needed**: a ruling on whether the kernel gains an id-release capability at all. If not, say the mod id space is monotonic within a process and give the cap its own loud assert. If yes, price the change to `LAYOUTS`/`get_layout_unchecked` against G6(b) — a resettable slot on the read path is exactly the overhead the requirement forbids, and "identical codegen" then has to be demonstrated rather than asserted.

---

## 🟡 Important

### W1. "The `FrameArena` is per thread slot" is not expressible by the type that owns the arenas

**Where**: P19/19.1 (`:1936-1946`): "`EcsMaster::frame(&self) -> &FrameArena` resolves the caller's slot from the single `LaneDeposit` read"; P17/SC-1: "`cache_slot` is valid iff `LaneDeposit.pool == the pool being used`".

`EcsMaster::new()` takes no arguments (`ecs_master.rs:426`) and the type holds no pool — grep for `ThreadPool` in `ecs_master.rs` returns nothing, and §5.1 deliberately *removes* the `Arc<ThreadPool>` from `Schedule` in favour of `Schedule::run(&mut self, master, pool: &ThreadPool)`. Three consequences follow, and none is stated:

- **Sizing**: `W + D` is unknown at `EcsMaster::new()`, so the arena table cannot be inline; it becomes an indirection on `frame()` that G6(c)'s `size_of::<EcsMaster>()` pin will also notice.
- **SC-1 cannot be evaluated**: `EcsMaster` has no pool to compare `LaneDeposit.pool` against, so the one rule that makes a slot index meaningful is unavailable at the site that uses it.
- **A slot-less caller has no arena, and one is reachable through a `pub` API**: the main thread outside `install` carries `DETACHED` (`tls.rs:147-152`), i.e. `NO_CACHE_SLOT`, and `DescendantsIter::new(world: &'w EcsMaster, root: Entity)` is `pub` (`traverse_iter.rs:294`). P19 point 3 routes that call's scratch to "its own slot's arena". Consequence: either a panic on a shipped public path, or a shared fallback arena — which reintroduces exactly the cross-thread bump P19 exists to remove.

**Solution options**: give `EcsMaster` the pool identity it now needs (it already needs the worker count); or keep one dispatcher arena and route worker per-call scratch to `Local<ScratchColumn>` as P4 originally did, taking the M-A8 overturn gate in the other direction; or make `frame()` fallible and name what a slot-less caller gets.

### W2. `EXP_MAX == MAX_CHUNKS` pins a quantity that does not bound the index, and the stated justification is contradicted by the file it cites

**Where**: P16/16.2 (`:1700-1710`): "EXP_MAX is `block.rs`'s MAX_CHUNKS, not a number of its own: the ladder `e = min_e.max(i)` with `i < MAX_CHUNKS` (block.rs:455) **can produce any exponent below MAX_CHUNKS**", with `const _: () = assert!(EXP_MAX == …MAX_CHUNKS_RECEIPT);`.

`block.rs:439-455` says the opposite in its own comment: "`i < MAX_CHUNKS`, so `CHUNK0 << i <= 2^43`) … **nothing in this module bounds `min_e`**, because what bounds it is the COMPILER … giving `min_e <= 50`". The exponent is `max(min_e, i)`, so its bound is **50**, not 31. Today an index above 31 needs an emplaced type of ≥ 2^43 bytes, so `heads[exp]` is not out of bounds in practice — but the assert pins the wrong invariant, and `block.rs:533` explicitly contemplates "a lower `MAX_CHUNKS`" as a future remedy, at which point `EXP_MAX` shrinks with it while `min_e` does not, and a scope emplacing a >`CHUNK0 << EXP_MAX` task body writes a chunk pointer past the end of a `SlotChunks` table that sits inside `PoolInner` next to other slots' tables. The block's own test suite already exercises an over-`CHUNK0` type (`Huge8200`, `block.rs:1839`).

**Solution options**: pin `EXP_MAX` against the real bound (the compiler fact documented at `block.rs:439-455`) and pay 51 × 8 B of head table, or keep 32 and route `exp >= EXP_MAX` to a stated `#[cold]` path — but then "a shorter table would orphan exactly the chunks rev 2 orphaned" is back and needs its own answer.

### W3. G6(b)'s "every pinned symbol is `#[inline(never)]` or `#[cold]` at its definition" is false for two of the four

**Where**: P18/18.1 (`:1897`).

Verified in joltab: `ScopeBlock::grow` is `#[inline(never)]` (`block.rs:425`) ✓; `HeapRef::alloc_cold` is `#[cold]` by construction ✓; **`ComponentPool::new` carries no inline attribute** (`component_pool.rs:279`; nearest attributes `:139`, `:527`); **`Schedule::run` carries none** (`schedule.rs:286`; the `#[cold] #[inline(never)]` at `:510-511` belongs to a different function). The gate's rule is "a missing symbol is RED, never a skip", so the gate is red-by-construction on two rows under LTO, and the obvious repair is to add `#[inline(never)]` to `Schedule::run` and `ComponentPool::new` — a codegen change to the engine's own hot path made *for the modding gate*, which is the overhead class the owner's requirement forbids and which principle 7 requires a measurement to justify.

**Solution options**: choose four symbols that already carry the attribute for their own reasons (the tree has many `#[inline(never)]`/`#[cold]` sites on the pool and block paths), or state that the two additions are part of the seam's price and measure them.

### W4. `MOD_ID_BASE = 384` charges the no-modding build a quarter of its component budget

**Where**: P21 (`:2030`): "ids `[0, 384)` are the engine's, minted by `NEXT_ID` as today with its exhaustion assert re-pointed at `MOD_ID_BASE`". Cost claimed: "one changed constant in a cold assert".

The runtime cost is indeed zero, but the *capability* cost is not: `MAX_COMPONENTS = 512` is a hard cap shared with enable-tags (`component_registry/mod.rs:63`; `enable_tag_api.rs:55`), and every build — including one with no `boyko_modding` in the graph — now exhausts at 384. The owner's requirement is "no overhead of any kind… when a game does not use modding"; a 25 % budget cut is paid by exactly those games. The split is also the architect's own open question 2 (chosen without a count).

**Solution options**: a zero-width reserved range when the modding crate is absent — mods mint *downward* from `MAX_COMPONENTS` and the engine keeps minting upward to 512, with the collision detection that already exists at `:918-919, 929-939` as the meeting point; or make the split a `boyko_modding`-set value the kernel reads only through the `#[cold]` `register_at` path.

---

## 🟢 Optional

**O1. Three cells of P8/Q4's reservation accounting now contradict rev 2.1 and were not removed.** Q4 (`:1082-1090`) still reads `ThreadPool | ChunkArena | 64 MiB | 1 MiB` (P16/16.5 makes it 1 GiB VA, 15.4 makes it 2 MiB Miri), `EcsMaster | … + FrameArena | 256 MiB … = 2.25 GiB` (P19 makes it 64 MiB × (W+D) ≈ 1.5 GiB at W=16), and `per mod | 2 GiB | 8 MiB` (15.4 makes it 1 MiB + 256 KiB). The file's own header warns that a superseded sentence "still reads as current where it stands", and this is the table a reader consults for a budget. One `**Removed**` block fixes it.

**O2. `HeapHeader`'s stated hot/cold line split does not follow from its field order.** 15.2 says "Line 0 is everything the COLD paths need … line 1.. is `free[]`". `#[repr(C, align(64))]` pads only the tail: the cold fields are 8+4×5 = 28 B in release, so `free[0..]` begins at offset 28-32 and the first eight heads share line 0 with the cold fields — and the boundary *moves* between debug and release as the two `cfg(debug_assertions)` fields appear. Harmless (one line touched either way), but the comment asserts a layout the declaration does not produce; an explicit pad to 64 makes the claim true and costs one line.

---

## Positive — preserve these

1. **The exit-(i) ruling and its M2 mutation** (P15/15.1, 15.6). Ruling the fork by name *and* adding the red-first mutation that describes the exit a developer would actually take is the correct response to "a green test whose mutation no longer describes the defect". Keep both mutations.
2. **C1's shape is sound, and I checked the mechanism rather than the argument.** `Heap::as_ref(&self)` copies the base *value* out of `VmReservation`; a pointer's provenance travels in its value, so the handle is parent-tagged from the `VirtualAlloc`, not derived from the `&self` retag — the `block.rs:35-42` D1 argument, correctly applied.
3. **`SlotChunks` deletes the state instead of enlarging it.** An intrusive list has no capacity, so "full" and "orphan" cease to be states — the same move that closed pass-1 C2. The rename away from "cache" is right: the word implied a miss policy this does not have.
4. **P17/17.2's release-sequence argument is correct and is the right level of rigour.** Both operations being RMWs on one word is what makes the edge exist; the note that the claim word must therefore not be split into per-slot words is the non-obvious half and must survive any later "optimisation" of `dispatcher_claim`.
5. **P20's liveness re-derivation.** Verified in the tree: `mark_idle`'s `fetch_or(Release)` (`worker.rs:383`) precedes the re-poll, and every push does `publish_fence()` then `idle.load(Acquire)` (`:494-496`). Moving the proof to the push side is correct, and "the spinner published nothing, so it wakes once per `Backoff` round" is the right treatment of the one remaining gap.
6. **P18's honest cost paragraph.** "`.text` pins are brittle and will churn… accepted *because* the alternative measured zero" is the correct trade and the correct way to record it. Likewise (f): keeping the two-arm build and relabelling it to the defect it *can* see.
7. **HV-4, and `Children` decided rather than deferred.** Turning a ~88-site discipline claim into a structural rule with an enumerable owner set, enforced by a parser that already runs, is worth more than the rule it replaces — and it removes W5's clone/prefab hazard rather than answering it.
8. **P0, P3, P6, P11 stand unchanged and should keep standing.** The withdrawn throughput justification, the protector-kind argument, the syntax-over-text ledger and the TB-1 rationale correction are the load-bearing intellectual content of this file.

---

## Open questions for the architect

1. **Can G2b's `commit_delta` actually be 0 in the churn scenes?** `raw::commit_at` counts every commit in the engine, including legitimate `ComponentPool`/`VmColumn` growth. If S2 (churn + `par_iter`) or S1c grows a column inside the steady window, the pin is red-by-construction and the pressure will be to widen it — the failure G2's own header forbids. The measurement that settles it: `commit_delta` over the existing steady windows on today's tree, before the pin is written. If it is nonzero, the counter needs attribution by owner, not a flat pin.
2. **What is the measured peak concurrently-live chunk bytes on the pile?** 16.5's "≤ 4 chunks ≈ 16 KiB per slot, ≤ 390 KiB per pool" is derived from install + one nested `par_iter`. The colored solve dispatches 121 scopes/step across W=4 (`frame-allocation-census.md:252`); the entry is G2b's `chunk_bytes_resident`, and 16.5 says the row is re-derived from it — say so in the rung's exit criteria, not only in the paragraph.
3. **Where is HV-4's boundary for `Local<T>` and for Resources?** HV-4 forbids a heap handle "in a `Resource` reachable from a non-exclusive system, or in any type a worker can obtain `&mut` to" — `ResMut<T>` in a parallel system is that by definition, so the permitted owner set may be smaller than the four structures named. Rung 2 sends `boyko_utils::SparseMap` to `HeapVec` fields; naming which of its users are dispatcher-only would settle whether the ledger's `hv3-owner` column has an answer for every heap row before G1 starts refusing rows without one.
4. **Open question 3 of rev 2.1 (`.text` churn) — accepted as you state it.** The one thing I would add to the gate's header: record the toolchain version beside each of the four numbers, so a re-bless after a bump is distinguishable from a re-bless after a seam change by reading the file rather than the git log.

---

# Rev 2.2 (2026-09-16)

**Scope of this patch:** blockers C1, C2, C3; important W1, W2, W3, W4; optional O1, O2; and the four open questions of the pass-3 review. **Nothing else in the file is touched** — P0, P3, P6, P7, P9, P11, P17, P20, P22, P23 and the "preserve these" list stand as written.

**Two of the closures are entangled and must be read as one ruling:** C1 (the `&self` `rewind`) and W1 (the per-slot arena that no type can size) are both consequences of routing *worker* scratch to the Frame class. P24 removes the unsound primitive; P27 removes the client that forced it. Reading either alone gives the wrong picture.

**Trees:** documents `D:/claude/BoykoEngine` (`feat/multi-paradigm-render`); code read read-only in `D:/wt/joltab` (`merge/ke16-into-ecsnative`). Ledger on disk at rev 4 per the pass-3 review's own read; **no claim in this patch depends on a per-row number in it**, and the only ledger change it requires is a *destination* column edit that rung 2 owns (P27).

---

## P24 (closes C1) — `rewind`/`FrameMark` are deleted; reclamation is a property of the handle's `Drop`, not of a token

**Depends on:** P4 (`&self` + `Cell`), P3's sanitize bullet (`:925`), P27 (which removes the worker client), `traverse_iter.rs:49-51, 294` (the client), `[S bumpalo lib.rs]`.

### 24.1 The ruling

The critic is right and the diagnosis is exact: a token marks a range, it does not make it unreachable, and `&self` puts the token's use and the handle's life in the same borrow region. bumpalo exposes no `rewind` for this reason; rev 2 kept one because rev 1's `&mut self` had been hiding the defect behind borrowck.

**A released range is made unreachable by the only thing that can prove reachability — the type system's own ownership of the handle.** `FrameVec`/`FrameBox` reclaim their own bytes in `Drop`, and **only when they are the topmost block**. There is no token, no LIFO assertion, and no safe-code sequence that invalidates a live handle, because the only allocation a `Drop` can release is one the language has just proved nobody holds.

**Rejected alternative, with its specific failure** (the critic's own suggested shape — a scoped sub-region on the `Scope`/`ScopeBlock` model): `arena.scope(|s| …)` does not close the hole, it moves it up one level. A handle taken from the **parent** while a child scope is open is still live when the child's `Drop` restores the cursor:

```
outer.scope(|inner| { let a = outer.vec::<u32>(64); /* a's bytes are below inner's mark */ });
// inner's Drop rewinds over `a`, which is still live — the same defect, one frame up.
```

Preventing that requires the child to take the parent by `&mut`, which refuses the pattern every actual client has (accumulate into an outer buffer, scratch in an inner loop). Drop-reclaim has no such hole because it never restores a cursor past anything that is still owned.

**Removed** (P4, the "API change" paragraph as it stands after P19 removed its final sentence, verbatim):

> **API change** in §2.4: `FrameArena::{alloc, alloc_slice_uninit, vec, rewind}` take `&self`; `reset(&mut self)` keeps `&mut` (bumpalo's rule, `[S bumpalo lib.rs]`: "requires `&mut` to guarantee no active borrows"), which is what makes a live `FrameVec<'f>` across a reset a **compile error** rather than a rule.

**Added:**

> **API change** in §2.4: `FrameArena::{alloc, alloc_slice_uninit, vec}` take `&self`; **`rewind` and `FrameMark` do not exist** (bumpalo exposes none either, and for this reason: a mark is not a proof of non-reachability, and under `&self` the mark's use and a live handle sit in the same borrow region — safe code could then hold two `FrameVec`s over one range, in release, with no `debug_assert` and nothing for Miri to see). `reset` keeps `&mut` (bumpalo's rule, `[S bumpalo lib.rs]`: "requires `&mut` to guarantee no active borrows"), which is what makes a live handle across a reset a **compile error** rather than a rule.
>
> **Reclamation inside a frame is FR-1, a property of `Drop`:**
>
> **FR-1 (top-block reclaim).** `FrameVec::drop` / `FrameBox::drop` compute `self_end = offset_of(self.ptr) + self.cap * size_of::<T>()` and reclaim **iff** `arena.cur == self_end`, by storing `cur = offset_of(self.ptr)`. If any block was bumped after this one, `cur != self_end` and the drop is a no-op. Cost: 1 load, 1 compare, 1 predicted branch, 1 store on the taken arm — on a path that was previously a no-op, so the price is ~3 instructions per scratch handle, paid where a `Vec` would have paid a `free`.
> **Why this cannot be forged:** the only range a `Drop` releases is the one the compiler has just proved is unowned; a range under a live handle is never topmost, because that handle's own block sits at or above it.
> **FR-2 (`!Send`, now load-bearing).** `FrameVec` holds `NonNull<T>` *and* `&'w FrameArena`, and `FrameArena` is `!Sync` (`Cell`), so the handle is `!Send` twice over. This is what stops a `Drop` running on a thread other than the arena's writer. A compile-fail test pins it (`static_assert_not_send`), because it is now an invariant rather than an incident.
> **FR-3 (growth in place, the reason FR-1 pays for itself).** A topmost `FrameVec` grows by extending the bump — no re-bump, no copy, O(1) — so "callers size it" becomes a hint instead of a correctness burden, and the `#[cold]` re-bump+copy path survives only for the non-topmost case. A grown vec that re-bumped leaves its old block stranded until `reset`; FR-1 then reclaims only the new block, which is correct and is stated so a reader does not expect otherwise.
> **FR-4 (what does NOT reclaim).** `alloc` / `alloc_slice_uninit` return raw references with no owner type; they are frame-lifetime by construction and never reclaim. Only handle types participate in FR-1.

**Removed** (P4's Added code block, the `FrameMark` and `FrameVec` lines, verbatim):

> ```rust
> pub struct FrameMark(usize);    // LIFO rewind token; `#[must_use]`
>
> /// Fixed-capacity vector on the frame arena. Capacity chosen at creation;
> /// overflow is a #[cold] re-bump + copy (bumpalo semantics) — callers size it.
> pub struct FrameVec<'f, T> { ptr: NonNull<T>, len: u32, cap: u32, _f: PhantomData<&'f FrameArena> }
> // 16 B. `T: Copy` in v1 (unchanged rationale: the inventory has no droppable
> // per-frame datum; if one appears the answer is `DropColumn`, not a finalizer chain).
> ```

**Added:**

> ```rust
> // `FrameMark` is DELETED (FR-1 replaces it; a token cannot prove non-reachability).
>
> /// Vector on the frame arena. Grows IN PLACE while it is the arena's top block
> /// (FR-3); a non-topmost growth is a #[cold] re-bump + copy (bumpalo semantics).
> /// `Drop` reclaims its bytes iff it is the top block (FR-1) — which is the whole
> /// of this class's intra-frame reclamation policy.
> pub struct FrameVec<'w, T> { ptr: NonNull<T>, len: u32, cap: u32, arena: &'w FrameArena }
> // 24 B, was 16 B: FR-1 needs the cursor, so the arena reference is a FIELD, not
> // PhantomData. This is a STACK-LOCAL handle, never a column element, so the 8 B
> // buys reclamation at no stride cost anywhere — and it matches `HeapVec`'s 24 B,
> // so a reader meets one handle width in this design, not two.
> // `T: Copy` in v1 (unchanged rationale: the inventory has no droppable per-frame
> // datum; if one appears the answer is `DropColumn`, not a finalizer chain).
> // !Send by FR-2, twice: NonNull<T>, and &FrameArena where FrameArena is !Sync.
>
> pub struct FrameBox<'w, T> { ptr: NonNull<T>, arena: &'w FrameArena }   // 16 B, same rules
> ```

**Removed** (P3, the sanitize bullet's last line, verbatim):

> `FrameArena::rewind`/`reset` fill the released range with `0xDD` under the same gate.

**Added:**

> `FrameArena::reset` and **every FR-1 reclaim** fill the released range with `0xDD` under the same gate. The reclaim is the more valuable of the two: `reset` releases at a frame boundary where nothing is live by construction, whereas an FR-1 reclaim releases *during* a frame, so a stale read of a reclaimed scratch buffer is exactly the bug this poison exists to catch. Under `cfg(miri)` the poison write is what makes the round-trip test of 24.2 a use-after-free detector rather than a bookkeeping check.

### 24.2 The gate (mandatory, lands with the primitive)

`crates/boyko_memory/tests/frame_reclaim.rs`, plus one `trybuild` case:

| | |
|---|---|
| **Round-trip (the anti-vacuity direction)** | `let before = arena.used(); { let g = …; let v = g.vec::<u64>(1024); let w = g.vec::<u32>(16); drop(w); drop(v); } assert_eq!(arena.used(), before);` — a *direct* assertion that the LIFO client returns to its entry cursor. |
| **Non-topmost is a no-op** | allocate `a` then `b`, drop `a` first, assert `arena.used()` unchanged, then allocate `c` and assert `c` does not overlap `b`'s range (byte-level, via `used()` deltas). This is the assertion whose failure would be rev 2's corruption. |
| **Client round-trip** | the same before/after around a full `DescendantsIter` walk (its scratch destination is P27's, so this row moves with it) and around one UI lane refill. |
| **RED-first mutation** | change FR-1's predicate from `arena.cur == self_end` to unconditional `arena.cur = offset_of(self.ptr)` — i.e. re-introduce "rewind to my mark regardless". Expected: the non-topmost test reds on overlap, and under `cfg(miri)` the poison makes it `read access through <TAG> … is forbidden`. Recorded verbatim in the file header, per the tree's gate convention. |
| **Compile-fail (`trybuild`)** | a `FrameVec` returned out of the borrow that produced it → `error[E0515]`/`E0502`. This is the replacement for the deleted `rewind`'s "compile error rather than a rule" property. |

---

## P25 (closes C2) — the page-assignment quantum is a function of the class, stated as a table, and the three derived numbers are re-derived

**Depends on:** P1's ladder (`:669-675`), P15/15.2 (`COMMIT_PAGE`, header), P15/15.3, P15/15.4, P8/Q4, P22/O1. ⚠ *Rev 2.5 (P46): P25 is the Heap's revival form (U-1); its 204 KiB worst case is one of U-1's reasons.*

### 25.1 The defect and the ruling

The arithmetic is as the critic states it. `size_class(c)` is `16c` for `c ∈ 1..=16` and `1 << (c-8)` for `c ∈ 17..=24`, i.e. 512 B … 64 KiB; four classes (21/22/23/24 = 8/16/32/64 KiB) exceed `COMMIT_PAGE = 4096`, so `COMMIT_PAGE / size_class(c) = 0` and the free list is threaded with zero links. The large tier does not cover them (`size > 64 KiB || align > 64`), so there is no third path and `HeapVec` growth past 4 KiB has no defined allocation.

**Ruling: the quantum scales with the class; the classes stay in the small tier.** Moving 8-64 KiB to the large tier is rejected with a number: the large tier is granule-quantised at 64 KiB, so a `HeapVec<u64>` of 1024 elements (8 KiB, class 21 — the critic's own example, and a `ConflictGraph` row) would consume a 64 KiB granule, an **8× resident inflation** on the exact clients rung 3 routes here, and it would hand the large tier a hot client list that O1's no-coalescing policy was argued against.

**Removed** (P15/15.3, cold-path steps 3-7 and the paragraph that follows them, verbatim):

> 3. `if p == hdr.small_pages { heap_small_va_exhausted(label) }` — `#[cold] #[inline(never)]`, a loud panic naming the heap; **this is Q4's promise, now implementable.**
> 4. `raw::commit_at(self.0, hdr.small_pages as usize * COMMIT_PAGE, p * COMMIT_PAGE, (p+1) * COMMIT_PAGE)` — one page, one syscall.
> 5. `hdr.small_frontier = p + 1` (1 store). **Committed == frontier by construction**, which is why the second counter is deleted.
> 6. Thread the page into `free[c]`: `COMMIT_PAGE / size_class(c)` links, a forward sequential write over one page (4 KiB streaming, no reads, prefetcher-friendly).
> 7. Return the head.
>
> Complexity O(page/class); frequency: once per 4 KiB of that class's high-water. Branches: 2, both predicted-not-taken after the first frame. Page-at-a-time rather than granule-at-a-time is deliberate: it makes step 5 trivially monotone, and it keeps the **resident floor at 4 KiB per class touched**, which is the number the packing plan exists to get down to. A heap growing 1 MiB pays 256 setup-time syscalls once, ~20-40 µs, outside the steady window.

**Added:**

> 3. `pages = PAGES[c]` and `links = LINKS[c]`, two `const` tables indexed by the class — **not** a division. `PAGES[c] = max(1, size_class(c) / COMMIT_PAGE)`; `LINKS[c] = (PAGES[c] * COMMIT_PAGE) / size_class(c)`. Both are compile-time values; the run-time cost is two loads from one 75-byte `const` block (`[u8; 25]` + `[u16; 25]`) that shares a line with itself. A table rather than a shift because `size_class(c) = 16c` is not a power of two for `c ∈ 1..=16` (a divide by 48, 80, 112 … would otherwise appear on the cold path), and the table also records the exact link count including the ≤ 240 B page remainder those classes leave (≤ 5.9 %, worst at c = 15).
>
> | c | 1 | 2 | 3 | … | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 |
> |---|---|---|---|---|---|---|---|---|---|---|---|---|---|
> | `size_class` | 16 | 32 | 48 | … | 256 | 512 | 1 K | 2 K | 4 K | 8 K | 16 K | 32 K | 64 K |
> | `PAGES` | 1 | 1 | 1 | … | 1 | 1 | 1 | 1 | 1 | **2** | **4** | **8** | **16** |
> | `LINKS` | 256 | 128 | 85 | … | 16 | 8 | 4 | 2 | 1 | **1** | **1** | **1** | **1** |
>
> 4. `if p + pages > hdr.small_pages { heap_small_va_exhausted(label) }` — `#[cold] #[inline(never)]`, a loud panic naming the heap; **this is Q4's promise, now implementable.** The bound is `p + pages`, not `p`: with a 16-page assignment the old form would have committed past the reservation.
> 5. `raw::commit_at(self.0, hdr.small_pages as usize * COMMIT_PAGE, p * COMMIT_PAGE, (p + pages) * COMMIT_PAGE)` — **one range, still one syscall**; `pages` changes the range length, not the call count.
> 6. `hdr.small_frontier = p + pages` (1 store). **Committed == frontier by construction**, which is why the second counter is deleted; the invariant survives verbatim because the assignment is still a single monotone advance.
> 7. Thread the assignment into `free[c]`: `LINKS[c]` links, a forward sequential write (streaming, no reads, prefetcher-friendly). For `c ≥ 21` this is a single link and the "write" is one store.
> 8. Return the head.
>
> Complexity O(`LINKS[c]`); frequency: once per `PAGES[c] × 4 KiB` of that class's high-water. Branches: 2, both predicted-not-taken after the first frame. **Resident floor is `max(COMMIT_PAGE, size_class(c))` per class touched** (was, wrongly, a flat 4 KiB), i.e. 4 KiB for `c ≤ 20` and 8/16/32/64 KiB for `c = 21..24`. A heap whose every class is touched is **51 pages = 204 KiB** (1 header + 20 single-page classes + 2 + 4 + 8 + 16), and that is the number the packing plan is measured against. A heap growing 1 MiB pays ≤ 256 setup-time syscalls once, ~20-40 µs, outside the steady window.
>
> `large` is the same shape at granule units, with `large_base` read from line 0 and `large_granules` as the bound.

### 25.2 The large tier's client list stops being an assumption

**Added** (new paragraph in P22/O1, after its justification sentence):

> **The "empty client list" is now a pin, not a claim.** It was true at rung 2 and it is *conditionally false* at rung 3: any `HeapVec` whose high-water exceeds 64 KiB lands here, and two rung-3 rows can reach it — `ConflictGraph.conflict_bits` at more than **724 systems** (N²/8 B) and `HeapVec<HeapVec<TriggerEntry>>` at more than **2730** outer entries (24 B each). So: the ledger's heap rows gain a `max-bytes` column, G1 refuses a heap row without one, and the header's `#[cfg(debug_assertions)] large_allocs` counter is **pinned at 0 across the App scenes**. If it fires, O1's three-step full-table policy is re-argued *with a named client and its measured live-run count*, which is the only condition under which "a policy that leaks bounded VA in a state the gate says is unreachable" stops being sound reasoning.

### 25.3 The two numbers derived from the one-page form

**Removed** (P15/15.4, the Miri table's first row, verbatim):

> | Heap small | 1 GiB | **1 MiB** | 256 pages: 1 header + up to 25 class pages + growth. A Miri test allocating past 255 pages is not a Miri test. |

**Added:**

> | Heap small | 1 GiB | **1 MiB** | 256 pages, re-derived under 25.1's quantum: 1 header + a worst case of **50** class pages if every one of the 24 non-ZST classes is touched (20 × 1 + 2 + 4 + 8 + 16) = **51 pages = 204 KiB**, leaving 205 pages of growth. The constant is unchanged; only its derivation was wrong. A Miri test allocating past 255 pages is not a Miri test. |

*(P8/Q4's per-`EcsMaster` floor is re-derived in P30/O1, where the whole table is replaced — the floor is **4 KiB**, the header page, not 0.)*

---

## P26 (closes C3 **and** W4) — the kernel gains no id-release capability; mod types reuse the registry path the tree already ships

**Depends on:** `component_registry/mod.rs:163-181` (`new_dynamic_tag`), `:194-203` (`DynamicTagMarker` and its NAME-keyed idempotency rule), `:208-209` / `:226-227` (`LAYOUTS` / `HOOKS`), `:852-861` (`was_ever_archetyped`), `:920-947` (`register_new`), `:949-992` (`try_register_dynamic`), `:1206-1214` (`get_layout_unchecked`), `tags.rs:1-21` (the process-global `TAG_NAMES` intern and its blessed `disallowed_types` exception), `component_pool.rs:279-288`.

### 26.1 The ruling, in one sentence

**The kernel gets no id release, no free list, no resettable slot and no partition — and it needs none, because the shipped registry already contains a non-generic, name-idempotent, CAS-bounded mint for ids that have no Rust type** (`try_register_dynamic`, `:967`, built for dynamic tags), and the three defects the critic found are all consequences of *storing a mod's pointers in a process-global static*, which this design now simply forbids.

`get_layout_unchecked` is **unchanged**, so K-MOD-4 and G6(b) are untouched and the read-path overhead the owner's requirement forbids is never introduced. I checked the blast radius rather than assuming it: `get_layout_unchecked` has exactly **one** caller outside the registry — `ComponentPool::new` (`component_pool.rs:285`) — and every other layout read in the workspace goes through the checked `get_layout` on cold paths (`hooks/builder.rs:170, 189`, `prefab.rs:535`, `materialize.rs:940`, `migration_helpers.rs:1579`, `save.rs:749`). A resettable slot would have been priced against that one hot-ish site; it is not needed, so it is not priced — it is refused.

**Removed** (P21, the four bullets under "Added", verbatim):

> - `MOD_ID_BASE = 384`. Ids `[0, 384)` are the engine's, minted by `NEXT_ID` as today with its exhaustion assert re-pointed at `MOD_ID_BASE` (one constant, a setup-time assert, no hot-path change); ids `[384, 512)` are the mod range, **owned and recycled by `boyko_modding`'s registry Resource** (K-MOD-8), not by the kernel.
> - The kernel therefore has **no** mod id policy, no free list, and no branch on id origin — the property K-MOD-4 buys, preserved.
> - **Cost when modding is absent: one changed constant in a cold assert.** G6(c)'s `size_of` pins are unaffected; (b)'s four symbols do not contain it.
> - **Overturn gate:** a scene needing more than 384 engine ids or more than 128 mod ids; the answer is then to raise `MAX_COMPONENTS`, which is an archetype-layout change and its own campaign.

**Added:**

> - **No range split. `MOD_ID_BASE` is deleted, `register_at` is deleted, and `NEXT_ID`'s exhaustion assert is untouched.** A build with no modding crate keeps **all 512** ids and changes not one constant — which is what the owner's requirement asks for, and which the 384/128 split did not deliver (W4: the runtime cost was zero, the *capability* cost was a quarter of the budget, charged to exactly the games that never load a mod).
> - **The seam entry is a generalisation of a path that already ships, not a new mechanism.** `ComponentLayout::new_dynamic_tag(name)` (`:173-181`) gains a sibling `new_dynamic(name, size, align)` with the **same** `DynamicTagMarker` sentinel `TypeId` (renamed `DynamicMarker` in docs only) and **`drop_fn: None`**; `try_register_dynamic` (`:967` — a bounded CAS on the shared `NEXT_ID`, returning `None` at the ceiling rather than panicking) is re-exported as the `#[doc(hidden)] pub`, non-generic, `#[cold]` seam entry. **K-MOD-5 therefore costs the kernel one constructor and one re-export.** Everything the critic asked the kernel to grow — a free list, a resettable `LAYOUTS`, an id-origin branch — does not exist.
> - **Idempotency is NAME-keyed, which is why recycling is unnecessary rather than merely unavailable.** The sentinel `TypeId` is shared by every dynamic mint, so the registry's own rule already forbids the `TypeId`-keyed idempotent arm and mandates a name intern (`:199-202`); the shipped intern is process-global (`tags.rs:7`, `TAG_NAMES`, behind the blessed `#[allow(clippy::disallowed_types)]` at `tags.rs:17-21`). Keyed on `"<mod>::<Type>"`, a **load → unload → reload of the same mod re-resolves to the same id and consumes nothing**. The budget is spent by *distinct mod type names ever seen in the process* — precisely the rule the engine's own Rust types already follow. W6's cap becomes "128 distinct mod type names", which is the honest statement of the same budget the engine lives under, not the "128 registrations ever" trap.
> - **Three hard constraints on a mod component type, each closing one dangling pointer:**
>   1. **`drop_fn: None`, asserted by the seam entry.** Nothing in `LAYOUTS` ever holds a code pointer into a mod image, so C3's defect (2) cannot exist — there is nothing to dangle after unload. Justification is not convenience: mod component data crosses a dynamic-library boundary, where a destructor is also an allocator-identity hazard, and P5's `ZeroInit` already requires "no destructor" of anything a column holds. **Mod component types are POD.**
>   2. **`type_name` is the kernel-interned leaked string**, not the mod's `&'static str` (the shipped tag path already leaks once per unique name, `:171-172`). This closes a second dangling pointer the critique did not name: a raw `&'static str` from the mod's image would outlive the image in every diagnostic that prints a component name.
>   3. **`HOOKS[id]` is never written for a mod id.** The seam entry does not touch the hook table, so `was_ever_archetyped`'s freeze (`:852-861`) never becomes a reload hazard. A mod's reactive behaviour is per-world observers and systems — world-owned data, torn down with the world or the unload — not process-global function pointers.
> - **The cross-world arbiter (C3's defect 3) exists by construction.** Both the mint (`NEXT_ID`) and the name intern (`TAG_NAMES`) are process-global, so two `EcsMaster`s in one process resolve one mod type name to one id. **K-MOD-8 is amended:** `boyko_modding`'s per-world `Resource` is no longer an id owner; it holds only per-world state (which mods are loaded *here*, which ids are live *here*).
> - **The starvation guard is a counted cap in the modding crate, not a carved range in the kernel.** `boyko_modding` refuses to mint past `MAX_MOD_TYPES = 128` of its own; the kernel's ceiling behaviour is what already ships (`try_register_dynamic` → `None`; engine `register_new` → the loud exhaustion panic at `:922-927`). Cost to a no-modding build: **zero instructions, zero constants, zero symbols** — the counter is in a crate that is not compiled.
> - **Reload with a changed layout is the one hazard left, and it is refused loudly.** The idempotent arm compares the stored `{size, align}` against the request and returns `DynamicLayoutMismatch` when they differ; a mod that changes a component's size across a hot reload fails at load time with a named error, not at first row write. This is the honest cap on hot-reload and it belongs in `boyko_modding`'s documentation, where the person who hits it is standing.

**Removed** (P21, the unload steps, the fourth item, verbatim):

> 4. return the ids to `boyko_modding`'s free list.

**Added:**

> *(Step 4 is deleted: there is no free list and no id return. Unload is three steps — `remove_component_type` per world, drop the mod's pools, `Heap::drop` — and **K-MOD-10 stands unchanged**, still the genuinely new kernel entry on the critical path of "a mod can be unloaded at all".)* ⚠ *Rev 2.5 (P47): K-MOD-10 is deleted (U-10, load-only).*

### 26.2 The gate

New rows in `tests/mod_seam_pins.rs`: (i) two registrations of one `"<mod>::<Type>"` name return the **same** id and mint nothing (`NEXT_ID` unchanged) — red-first: key the intern on the pointer instead of the string; (ii) a registration with `drop_fn: Some(_)` is refused at the seam — red-first: drop the assert; (iii) a second registration of a name with a different size returns `DynamicLayoutMismatch`; (iv) `HOOKS[id]` is `None` for every mod id after a load/unload/reload cycle.

---

## P27 (closes W1; completes C1) — one `FrameArena` per world, reached through a guard; worker per-call scratch leaves the Frame class

**Depends on:** P24 (FR-1..FR-4), `ecs_master.rs:426` (`EcsMaster::new()` takes no arguments), `thread_pool.rs:158, 665-773` (`worker_count` is known only at `ThreadPoolBuilder::build`), `tls.rs:126-152` (`LaneDeposit`, `DETACHED`), `traverse_iter.rs:33-52, 281-308`, `component/scratch.rs` (`ScratchColumn`, the shipped shape), `docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md` (the TLS cost note referenced at `tls.rs:113-115`).

### 27.1 The ruling

W1 is correct on all three counts and I am taking the exit the critic named second, not the first. `EcsMaster::new()` takes no arguments and holds no pool (`ecs_master.rs:426`); §5.1 deliberately removed `Arc<ThreadPool>` from `Schedule`. Giving the world a pool identity to size a slot table is a workspace-wide constructor change **bought for one client**, and that client should not be here at all:

- The only worker-thread client of the Frame class was `DescendantsIter`'s scratch (P19 point 3).
- Resolving a slot per call means a TLS read per `DescendantsIter::new`, and on this host that is not free: the tree pays *one* `thread_local!` access per spawn deliberately, and the measured windows-gnu cost of a `thread_local!` access on rustc ≥ 1.98 is 2 lock-RMWs plus `FlsSetValue`.
- CLAUDE.md principle 0 names "truly transient function-local scratch" as a legitimate exception, and the tree's shipped answer for per-system persistent scratch is `ScratchColumn` (`ComponentPool`-backed, address-stable, reused across frames — `boyko_render` uses ~20 of them).

**So: the Frame class reverts to ONE arena per `EcsMaster`, and rung 1e's worker scratch goes to `Local<ScratchColumn>` — M-A8 taken in the direction the critic offered.** Everything W1 listed dissolves: no `W + D` at construction time, no SC-1 evaluation at a site with no pool, no `DETACHED` caller, no per-slot VA, no TLS read on the scratch path, and `24 × 64 MiB` of VA per world goes back to 256 MiB.

**A numeric correction to the critique, in the direction that strengthens the ruling.** The review states `VisitedSet` is "~16 KiB per call at 1M entities". It is **~122 KiB**: `words: Vec<u64>` is indexed `id >> 6` (`traverse_iter.rs:60-63`), so id 1 000 000 gives 15 626 *words* = **125 008 bytes**; 16 KiB is the word *count* read as bytes. The corrected figure makes the case for a persistent column stronger, not weaker — a frame arena would re-bump 122 KiB on every call and reclaim it on drop, while one `ScratchColumn` is warm after the first walk and never bumps again.

**Removed** (P19/19.1, the ruling sentence and points 1-3, verbatim):

> **The `FrameArena` is per thread slot — one per `(W + D)` slot, exactly as `SlotChunks` is — and `EcsMaster::frame(&self) -> &FrameArena` resolves the caller's slot from the single `LaneDeposit` read the tree already pays (D7, `tls.rs:113-115`).** Three problems collapse into one structure:
>
> 1. **W1 dissolves.** `frame(&self)` reborrows `&self`, so N `FrameVec<'f>`s may be live beside any other `&self` method — rung 2's CSR two-pass (offsets in one `FrameVec`, flat data in another, while the hierarchy is read) compiles.
> 2. **The compile-error property W1 wanted is kept.** `reset` is `EcsMaster::frame_reset_all(&mut self)`; `'f` borrows `*self`, so a `FrameVec` alive across a reset is a **borrowck error**, not a rule. This is bumpalo's `&mut`-for-reset argument `[S bumpalo lib.rs]` applied one level up.
> 3. **W2's missing destination appears.** Worker-thread per-call scratch — `traverse_iter.rs:51, 284`'s `VisitedSet.words: Vec<u64>` reached through a `pub fn` taking `&EcsMaster` — allocates from *its own slot's* arena. No cross-thread bump, no `Cell` shared between threads, nothing for HV-3 to forbid. `FrameVec` holds a `NonNull<T>` and is therefore `!Send` by construction, so it cannot escape to another thread.

**Added:**

> **There is ONE `FrameArena`, owned by `EcsMaster`, and it is reached only through a guard obtained from `&mut self`:**
>
> ```rust
> /// The only handle to the frame arena. Obtained from `&mut EcsMaster`, so a
> /// worker thread cannot reach it AT ALL — the exclusivity is the borrow, not a
> /// rule, not a TLS check, not a `debug_assert` over ~88 sites.
> /// Carries the world's SHARED reborrow too, which is what lets rung 2's CSR
> /// two-pass read the hierarchy while N handles are live.
> pub struct FrameGuard<'w> { arena: &'w FrameArena, world: &'w EcsMaster }
> impl EcsMaster { pub fn frame(&mut self) -> FrameGuard<'_>; }
> impl<'w> FrameGuard<'w> {
>     pub fn world(&self) -> &'w EcsMaster;
>     pub fn vec<T: Copy>(&self, cap: u32) -> FrameVec<'w, T>;
>     pub fn boxed<T: Copy>(&self, v: T) -> FrameBox<'w, T>;
>     pub fn alloc_slice_uninit<T>(&self, n: usize) -> &'w mut [MaybeUninit<T>];
> }
> ```
>
> 1. **W1 dissolves without a pool identity.** `EcsMaster::new()` keeps its signature; there is no slot table to size, no `LaneDeposit.pool` to compare against a pool the world does not hold (SC-1 is never evaluated at a site that cannot evaluate it), and no `NO_CACHE_SLOT` caller to panic at.
> 2. **The exclusivity is a borrow, and the CSR two-pass still compiles.** A bare `frame(&mut self) -> &FrameArena` would have been *too* strong — the `&'a mut self` is consumed for `'a`, so the world could not be read while a handle lived. The guard hands out both reborrows from one `&mut`, which is trivially sound (two shared reborrows of disjoint-or-shared fields) and is the whole reason the type exists.
> 3. **`reset` stays a borrowck error to misuse.** `EcsMaster::frame_reset(&mut self)`; `'w` borrows `*self`, so a live handle across a reset does not compile — bumpalo's `&mut`-for-reset argument `[S bumpalo lib.rs]`, one level up, unchanged.
> 4. **Worker per-call scratch leaves this class.** `DescendantsIter` / `AncestorsIter` / `trigger_broadcast_down` take their scratch as a parameter: `DescendantsIter::new_in(world: &'w EcsMaster, root: Entity, scratch: &'w mut TraversalScratch)`, where `TraversalScratch { visited: ScratchColumn<u64>, stack: ScratchColumn<(Entity, u32)> }`. A parallel system supplies it from `Local<TraversalScratch>` (per-system state, exclusively borrowed by that system instance, so a `&EcsMaster` caller on a worker thread is served with **no** TLS read, no arena, no slot and no panic); the kernel's own broadcast walk supplies the world-owned one it already reaches through `&mut EcsMaster`. **`DescendantsIter::new(world, root)` is removed, not kept as a panicking wrapper** — a `pub` function that panics on a thread is exactly the shipped-path failure W1 objected to.
> 5. **Per-system lanes are `ScratchColumn`, not `FrameVec`.** Rung 2's UI frame lanes (`boyko_ui/resources.rs`, `pick.rs`, `focus.rs`) clear and refill per system, per frame, from non-exclusive systems, which cannot obtain a `FrameGuard`. Their destination is the tree's shipped shape — a `Resource`/`Local`-owned `ScratchColumn`, warm after the first frame instead of re-bumped every frame. **This is a ledger destination edit rung 2 owns**, and it is named here so the two documents do not diverge silently.
>
> **The Frame class's client list is therefore exactly: dispatcher-side scratch inside an exclusive system** (command staging, hierarchy rebuilds, prefab materialisation, schedule-build buffers). **Exit criterion, stated so it can fail:** if rung 2 ends with that list empty, the class is **deleted**, not shipped unused — a bump allocator with no client is 256 MiB of VA and a maintenance surface bought for nothing.
>
> Cost: `size_of::<EcsMaster>()` grows by `size_of::<FrameArena>()` ≈ 64 B (one line). G6(c)'s absolute pin is re-blessed **once**, in the commit that lands the field, under P18's already-named legitimate reason "a deliberate kernel change unrelated to the seam" — and that is the entire multi-threading cost of this ruling, against the 1.5 GiB of VA, the per-call TLS read and the `DETACHED` panic it removes.
>
> **Overturn gate (M-A8, re-pointed):** if a profile shows an exclusive system's frame scratch dominated by the arena's own bump, or if a *new* worker-thread client appears that cannot take a `Local`, the per-slot form returns — and it returns with the pool identity problem solved first, not deferred.

**Removed** (P19/19.1, the cost paragraph and the overturn gate, verbatim):

> Cost: one TLS read per `frame()` call (callers hoist `&FrameArena` and allocate N times from it), 24 × 64 MiB = 1.5 GiB of VA at W=16 (VA is free; residency stays per-slot high-water, 0 until used — the Frame class stays **lazy**, because it has no reservation-resident header and therefore no branch to remove). `reset_all` writes `W+D` cursors at the frame boundary, where no worker is running.
>
> **Overturn gate:** M-A8 (new) — if per-slot arenas show a resident total above **4 MiB per pool** in the App scenes, or if the TLS read shows in a `frame()`-heavy profile, the Frame class reverts to one dispatcher arena and `traverse_iter`'s scratch goes to `Local<ScratchColumn>` instead.

**Added:**

> *(Superseded by the ruling above: one arena, `FrameGuard`, 256 MiB of VA, lazy, one cursor written at `frame_reset`. The M-A8 overturn gate is restated there in its new direction.)*

**Measurement entries** (specified, not run — the machine is the owner's):
- **M-A11** — the single arena's per-frame high-water across S1c/S2, and the count of FR-1 reclaims vs non-topmost drops. Pass condition for the class: high-water < 1 MiB and the named client round-trips to its entry cursor (24.2).
- **M-A12** — `TraversalScratch` high-water per system at the 1M-entity scene, to confirm the **122 KiB** figure and to size the `ScratchColumn`'s reservation. This number is the one the ledger row carries.

---

## P28 (closes W2) — `EXP_MAX` is pinned against the bound that actually bounds it

**Depends on:** `block.rs:105` (`CHUNK0 = 4096`), `:116` (`MAX_CHUNKS = 32`), `:425-455` (`grow`, `e = min_e.max(i)`, and the comment that derives `min_e ≤ 50` from rustc's layout caps), `:533`, `:1839` (`Huge8200`).

The critic is right and the file it cites says so in its own words: `block.rs:439-455` states that **nothing in that module bounds `min_e`** — the compiler does, at 50 — while `MAX_CHUNKS` bounds only the `i` term. `EXP_MAX == MAX_CHUNKS` therefore pins a quantity that does not bound the index, and `block.rs:533` already contemplates a lower `MAX_CHUNKS`, at which point the table would shrink while `min_e` would not.

**Removed** (P16/16.2, the `SlotChunks` doc lines and its two constants, verbatim):

> /// EXP_MAX is `block.rs`'s MAX_CHUNKS, not a number of its own: the ladder
> /// `e = min_e.max(i)` with `i < MAX_CHUNKS` (block.rs:455) can produce any
> /// exponent below MAX_CHUNKS, so a shorter table would orphan exactly the
> /// chunks rev 2 orphaned. Drift is a build failure.

and

> const EXP_MAX: usize = 32;
> const _: () = assert!(EXP_MAX == boyko_threadpool::block::MAX_CHUNKS_RECEIPT);

**Added:**

> /// EXP_MAX is 51 because the exponent is `e = min_e.max(i)` (block.rs:455) and
> /// the two terms have DIFFERENT bounds: `i < MAX_CHUNKS` (32) is a module fact,
> /// while `min_e <= 50` is a COMPILER fact — rustc refuses a type of size
> /// `1 << 61` and caps `repr(align)` at `1 << 29`, so `required < 2^61 + 2^29`
> /// (block.rs:439-455 states the derivation at the site that depends on it).
> /// Pinning EXP_MAX to MAX_CHUNKS pinned the term that does NOT bound the index;
> /// a lower MAX_CHUNKS (block.rs:533 contemplates one) would then shrink this
> /// table while `min_e` stayed put, and a scope emplacing a `> CHUNK0 << 31`
> /// body would write a chunk pointer past the end of a table that sits in
> /// `PoolInner` next to other slots'. The block's own suite already exercises an
> /// over-CHUNK0 type (`Huge8200`, block.rs:1839).

and

> const EXP_MAX: usize = 51;                                   // exponents 0..=50
> const _: () = assert!(EXP_MAX >= boyko_threadpool::block::MAX_CHUNKS_RECEIPT);
> // ^ catches a RAISED MAX_CHUNKS. The `min_e <= 50` half cannot be asserted in
> // code (it is a property of rustc's layout rules, not of a constant), so it is
> // carried by the comment above PLUS a release-active bound check on the return
> // path: `assert!(exp < EXP_MAX)` in the per-slot push, one compare on a path
> // that runs once per scope end. Unfireable per the derivation, and loud rather
> // than silently out of bounds if the derivation ever stops holding.

**Removed** (P16/16.2, the `heads` field line, verbatim):

> heads: UnsafeCell<[*mut u8; EXP_MAX]>,   // 256 B = 4 whole cache lines

**Added:**

> heads: UnsafeCell<[*mut u8; EXP_MAX]>,   // 408 B; with align(64) the struct is
>                                          // 448 B = 7 whole lines in both cfgs
>                                          // (the debug `owner` fits the tail pad)

Cost of the correction: **152 B per slot**, 3.6 KiB per pool at W+D = 24, against a table that could otherwise be indexed out of bounds. The existing `assert!(size_of::<SlotChunks>() % 64 == 0)` stands and still means "whole lines, no false sharing between slots".

---

## P29 (closes W3) — the pinned symbols are chosen by a rule, and the rule makes "red by construction" impossible

**Depends on:** verified in joltab — `ScopeBlock::grow` `#[inline(never)]` (`block.rs:425`) ✓; `ComponentPool::new` **carries no inline attribute** (`component_pool.rs:279`) ✗; `Schedule::run` **carries none** (`schedule.rs:286`) ✗; `ComponentPool::grow_rows` `#[inline(never)]` (`component_pool.rs:600`) ✓; `ComponentPool::commit_subregion` `#[inline(never)]` (`:559`) ✓; `run_check_ticks_scan` `#[inline(never)]` (`check_ticks.rs:107`) ✓.

The critic's mechanism is right: the gate's own rule ("a missing symbol is RED, never a skip") makes two of the four rows red on the shipped tree, and the obvious repair — adding `#[inline(never)]` to `Schedule::run` and `ComponentPool::new` — is a codegen change to the engine's own path made *for the modding gate*, which is the class the owner's requirement forbids and which principle 7 requires a measurement to justify. **I am not paying that.** The four symbols are re-chosen from those that already carry the attribute for their own reasons.

**Removed** (P18/18.1, the (b) row, the parenthetical listing the four symbols, verbatim):

> The four are chosen so the pin has a guaranteed subject — **every pinned symbol is `#[inline(never)]` or `#[cold]` at its definition**, because an `#[inline]` function may leave no symbol at all and an absent symbol would read as a skip: `Schedule::run`, `ComponentPool::new` (`component_pool.rs:279`, non-generic), `ScopeBlock::grow` (`block.rs:425`, `#[inline(never)]` — this replaces rev 2's `ScopeBlock::bump`, which is `#[inline]` at `block.rs:368` and may not exist), `HeapRef::alloc_cold` (P15's `#[cold]` page-assign path). **A missing symbol is RED, never a skip.**

**Added:**

> The symbols are chosen by a **rule with three conditions, each checkable before the pin is written**, because rev 2.1 chose two that fail it and would have made the gate red on the shipped tree: a pinned symbol must (i) already carry `#[inline(never)]` or `#[cold]` **at its definition, for its own stated reason, cited file:line** — never an attribute added for this gate, which would be a codegen change to the engine bought for the modding build; (ii) be **non-generic**, so it has one mangled name rather than one per instantiation; (iii) lie on a path the seam could plausibly perturb. The five: ⚠ *Rev 2.5 (P49): four — `HeapRef::alloc_cold` is struck (U-1).*
>
> | symbol | attribute, cited | why the seam would move it |
> |---|---|---|
> | `ComponentPool::grow_rows` | `#[inline(never)]`, `component_pool.rs:600` | the growth path every mod component column takes; a `Layout` indirection in the pool shows here |
> | `ComponentPool::commit_subregion` | `#[inline(never)]`, `component_pool.rs:559` | the commit route; K-MOD-7's `raw` seal and P15's `commit_at` both land on it |
> | `run_check_ticks_scan` | `#[inline(never)]`, `check_ticks.rs:107` | walks every registered component id per tick check; an id-origin branch shows here |
> | `ScopeBlock::grow` | `#[inline(never)]`, `block.rs:425` | the threadpool allocation path (replaces rev 2's `ScopeBlock::bump`, which is `#[inline]` at `block.rs:368` and may not exist) |
> | ~~`HeapRef::alloc_cold`~~ ⚠ *Rev 2.5 (P49): struck — U-1 never builds it; four symbols remain* | ~~`#[cold]` by construction, P15/15.3~~ | ~~the heap's page-assign path, written by this campaign~~ |
>
> **`Schedule::run` and `ComponentPool::new` are NOT pinned**, and the reason is recorded rather than left as an omission: neither carries an inline attribute today (`schedule.rs:286`, `component_pool.rs:279` — the `#[cold] #[inline(never)]` at `schedule.rs:510-511` belongs to a different function), so pinning them would have forced condition (i) to be bought with an attribute the engine does not otherwise want. **A missing symbol is RED, never a skip** — and under this rule a missing symbol now means "somebody removed an attribute", which is a real finding, instead of "the rule was written against symbols that never had one".
>
> **Stated limit, because the gate should not be believed past it:** a `.text` pin cannot see a change *inside* an `#[inline]` function, so it cannot cover the whole kernel. That is what check (e)'s seam-inventory gate is for, and (e) is the one of the five that scales.

---

## P30 (closes optional O1, O2) — the stale reservation table, and a layout comment that was not true of its declaration

### O1 — P8/Q4's table is replaced, not annotated

**Removed** (P8/Q4, the table body and the sentence that follows it, verbatim):

> | `EcsMaster` | Heap small, Heap large, FrameArena | 1 GiB + 1 GiB + 256 MiB = **2.25 GiB** | 4 + 4 + 4 MiB | **0** (all lazy) |
> | `EcsMaster` | MasterTables (`TableSet`) | granule-rounded total, ≤ 4 MiB | same | one granule, 4 KiB after packing-plan D1 |
> | `ThreadPool` | ChunkArena | **64 MiB** | 1 MiB | 0 → `(W+D) × 60 KiB` cached + peak live; **1440 KiB (1.41 MiB) at W=16, D=8** |
> | `Schedule` | ScheduleTables | ≤ 1 MiB | same | 4 KiB |
> | per `ComponentPool` | unchanged | unchanged | unchanged | 64 KiB → 4 KiB after packing plan S0-S2 |
> | per mod (§7) | Heap small + large | 2 GiB | 8 MiB | 0 |
>
> Rev 1's 4 GiB + 16 GiB is cut to 1 GiB + 1 GiB: `InlandStore` already reserves 1 GiB on 64-bit (`inland_store.rs:5`, `DEFAULT_INLAND_RESERVE`) and is the tree's precedent, and the large tier's client list is empty after rung 2 moves the entity-scaled maps to columns. **Overturn gate:** a `Heap` exhausting 1 GiB of VA in any suite reds its own `#[cold]` exhaustion panic, which is the signal to raise the constant.

**Added:**

> *(Re-derived at rev 2.2. Every cell below is the value after P15/15.1's eager-reserve ruling, P16/16.5, P19-as-amended-by-P27 and P25's quantum; the file's header warning that a superseded sentence "still reads as current where it stands" is why this table is replaced rather than footnoted.)*
>
> | Owner | Reservations | VA (release) | VA (Miri) | Resident floor at boot |
> |---|---|---|---|---|
> | `EcsMaster` ⚠ *Rev 2.5 (P46): Heap revival form; Frame deleted (P34)* | Heap small, Heap large, FrameArena (**ONE**, P27) | 1 GiB + 1 GiB + 256 MiB = **2.25 GiB** | 1 MiB + 256 KiB + 4 MiB | **4 KiB** — the heap's header page, committed by `Heap::new` (15.2); Frame is lazy, so 0 |
> | `EcsMaster` ⚠ *Rev 2.5 (P46): Heap revival form* | sanitize live map (P22/O2) | — (cfg'd out) | 8 KiB | 0 |
> | `EcsMaster` ⚠ *Rev 2.5 (P46): `TableSet` revival form (U-7)* | MasterTables (`TableSet`) | granule-rounded total, ≤ 4 MiB | same | one granule, 4 KiB after packing-plan D1 |
> | `ThreadPool` | ChunkArena | **1 GiB** (16.5) | 2 MiB | 0 → peak concurrently-live chunk bytes; **≤ 390 KiB at W=16**, pinned by G2b (M-A10) |
> | `ThreadPool` | `SlotChunks` (inline in `PoolInner`, not a reservation) | — | — | 448 B × (W+D) = **10.5 KiB at W=16, D=8** (P28) |
> | `Schedule` ⚠ *Rev 2.5 (P46): `TableSet` revival form (U-7)* | ScheduleTables | ≤ 1 MiB | same | 4 KiB |
> | per `ComponentPool` | unchanged | unchanged | unchanged | 64 KiB → 4 KiB after packing plan S0-S2 |
> | per mod (§7) ⚠ *Rev 2.5 (P47): withdrawn with K-MOD-3* | Heap small + large | 2 GiB | 1 MiB + 256 KiB | **4 KiB** (its own header page) |
>
> **Per-`EcsMaster` eager Miri cost: 1.26 MiB** (15.4, unchanged). Rev 1's 4 GiB + 16 GiB is cut to 1 GiB + 1 GiB: `InlandStore` already reserves 1 GiB on 64-bit (`inland_store.rs:5`, `DEFAULT_INLAND_RESERVE`) and is the tree's precedent. **The large tier's client list is empty after rung 2 and CONDITIONALLY non-empty at rung 3** — see P22/O1 as amended by P25/25.2, where the condition is a pinned counter rather than a claim. **Overturn gate:** a `Heap` exhausting 1 GiB of VA in any suite reds its own `#[cold]` exhaustion panic, which is the signal to raise the constant.

### O2 — `HeapHeader`'s line split is made true by a pad

**Removed** (P15/15.2, the doc sentence, verbatim): ⚠ *Rev 2.5 (P46): O2 (`LINE0_PAD`) is the Heap's revival form.*

> /// Line 0 is everything the COLD paths need (limits, frontiers, the large base)
> /// and is touched once per page assignment; line 1.. is `free[]`, which the
> /// WARM path touches on every alloc and free. The split is the hot/cold rule
> /// applied inside one 4 KiB page: an alloc that hits a free list reads exactly
> /// one line.

**Added:**

> /// Line 0 is everything the COLD paths need (limits, frontiers, the large base)
> /// and is touched once per page assignment; line 1.. is `free[]`, which the
> /// WARM path touches on every alloc and free. The split is the hot/cold rule
> /// applied inside one 4 KiB page: an alloc that hits a free list reads exactly
> /// one line.
> ///
> /// The split is produced by an EXPLICIT pad, not by `align(64)`: `repr(C,
> /// align(64))` pads only the TAIL, so without the pad `free[0..]` would start
> /// at offset 28 (release) or 44/56 (debug / sanitize) and the first eight heads
> /// would share line 0 with the cold fields — and the boundary would MOVE
> /// between cfgs, so the comment would be true in no build and false in a
> /// different way in each. Cold-field census: 8 + 4×5 = 28 B release, +8 owner
> /// +4 live_allocs = 44 B debug, +8 live_map = 56 B sanitize — every cfg fits in
> /// 64 B, so one pad constant serves all three.

and, in the struct body, between the cold group and `free`:

> ```rust
>     _pad_to_line1: [u8; LINE0_PAD],   // LINE0_PAD = 64 - size of the cold group in THIS cfg
>     const _: () = assert!(offset_of!(HeapHeader, free) == 64);   // the comment, mechanised
> ```

Cost: ≤ 36 B inside a page that is already committed. The `assert!` is what keeps the comment honest after the next cfg-gated field is added — which is the failure mode O2 actually describes.

---

## P31 — the pass-3 open questions, answered

**1. Can G2b's `commit_delta` be 0 in the churn scenes?** Almost certainly not, and a flat pin would be red by construction — the critic's own catalogued failure. **Ruling: the counter is attributed by owner at the choke point, at zero cost.** `raw::commit_at` takes a `const OWNER: CommitOwner` parameter (`Column | Heap | Chunk | Frame | Table`) and increments `COMMITTED_BYTES: [AtomicUsize; 5]` at a **compile-time** index — same one relaxed RMW, no branch, no extra load. G2b then pins `Heap + Chunk + Frame + Table` at **0** over the steady window (these are the campaign's own structures; a commit there in a steady frame is the defect the gate exists for) and **reports** `Column`, whose legitimate growth is bounded separately by the scene's entity high-water. **M-A9** measures `commit_delta` by owner over S1c/S2 on today's tree *before* the pin is written; if `Column` is nonzero in a steady window that is a finding about column growth, not a licence to widen the pin. The anti-vacuity direction of G2b is unchanged and now applies per owner: the setup window must report nonzero for **each** owner class that the scene uses. ⚠ *Rev 2.5 (P46.3): three owners — `Column`, `Chunk`, `Table` (the `Heap` owner leaves with U-1, `Frame` with P34; KC-01); G2b = UG-04 pins `Chunk` and `Table` at 0 and reports `Column`.*

**2. Peak concurrently-live chunk bytes.** Accepted as stated: **M-A10** measures `chunk_bytes_resident` on the pile (the colored solve's 121 scopes/step across W=4, `frame-allocation-census.md:252`), and **the row moves into rung 1a's exit criteria**, not only 16.5's paragraph. 16.5's "≤ 4 chunks ≈ 16 KiB per slot, ≤ 390 KiB per pool" is labelled *derived from install + one nested `par_iter`* and is superseded by M-A10's number when it exists.

**3. HV-4's boundary for `Local<T>` and Resources.** The critic is right that `ResMut<T>` in a parallel system is "a type a worker can obtain `&mut` to" by definition, so the permitted owner set is smaller than "the four structures". **Ruling: `hv3-owner` takes a value from a CLOSED vocabulary — `schedule`, `schedule-builder`, `registry`, `master-table` — and G1 refuses any other value, including an empty one.** A `Resource` is not an owner class; a `Local` is not an owner class. Consequence for the `SparseMap` rows rung 2 routes to `HeapVec` fields: a `SparseMap` user that is not one of the four **cannot become a heap row** and takes `VmColumn` instead. That decides the question by gate rather than by enumeration, so it does not need the enumeration now — which is the point, because the enumeration would go stale. ⚠ *Rev 2.5 (P46): the `hv3-owner` vocabulary belongs to the Heap's revival form (U-1); the `SparseMap` rows take `VmColumn`s (KC-18).*

**4. Toolchain version beside each `.text` number.** Accepted verbatim. `tests/mod_seam_pins.rs`'s header records `rustc -Vv` output beside each of the ~~five~~ four pinned numbers (P29 makes it ~~five~~ four ⚠ *Rev 2.5 (P49)*), so a re-bless after a toolchain bump is distinguishable from a re-bless after a seam change **by reading the file**, not the git log. P18's re-bless discipline already names the two legitimate reasons; this makes the first one self-evidencing.

---

## Change log (rev 2.1 → rev 2.2)

| Row | Disposition | Where |
|---|---|---|
| **C1** | **CLOSED by deleting the primitive** — `rewind` and `FrameMark` do not exist; reclamation is FR-1 (drop-reclaim iff topmost), which cannot be forged from safe code because the only range a `Drop` releases is one the compiler proved unowned. `FrameVec` 16 B → 24 B (carries the arena, stack-local so no stride cost), `!Send` promoted to a pinned invariant, FR-3 growth-in-place added. The closure-`FrameScope` alternative is rejected **with its specific hole** (a parent handle taken while a child is open). Gate: round-trip + non-topmost no-op + a `trybuild` escape case + a named red-first mutation. | P24 |
| **C2** | **CLOSED** — the assignment quantum is `PAGES[c] = max(1, size_class(c)/4096)` with `LINKS[c]` from a `const` table (no divide); the exhaustion check becomes `p + pages > small_pages`; **three derived numbers re-derived**: the resident floor is `max(COMMIT_PAGE, size_class(c))` per class (not a flat 4 KiB), the all-classes worst case is **51 pages = 204 KiB**, and 15.4's 1 MiB survives with a corrected derivation. Moving the four classes to the large tier is rejected with the 8× inflation number. The large tier's "empty client list" becomes a pinned counter with two named rung-3 thresholds (724 systems; 2730 entries). | P25 |
| **C3** | **CLOSED with NO kernel id-release capability and NO read-path change** — mod types reuse the shipped `try_register_dynamic` (`:967`) + the process-global `TAG_NAMES` intern; ids are monotonic but **name-idempotent**, so reload consumes nothing. Three hard constraints kill every retained pointer: `drop_fn: None` (defect 2 cannot exist), kernel-interned `type_name` (a second dangler the critique did not name), no `HOOKS` write (so `was_ever_archetyped` never bites). Cross-world arbitration is by construction (both statics are process-global); K-MOD-8 amended; unload loses its fourth step. `get_layout_unchecked` **unchanged** — blast radius verified as one caller. | P26 |
| **W1** | **CLOSED by withdrawing the per-slot form** — one `FrameArena` per world behind `FrameGuard<'w>` (obtained from `&mut self`, carries the world's shared reborrow so the CSR two-pass compiles). No pool identity, no slot table, no SC-1 evaluation, no TLS read, no `DETACHED` panic; 1.5 GiB of VA → 256 MiB. Worker scratch goes to `Local<TraversalScratch>`; `DescendantsIter::new` is **removed**, not wrapped. **Numeric correction: `VisitedSet` at 1M entities is ~122 KiB, not the review's 16 KiB** (word count read as bytes). Cost: `size_of::<EcsMaster>()` +64 B, one G6(c) re-bless. | P27 |
| **W2** | **CLOSED** — `EXP_MAX = 51`, pinned by `>= MAX_CHUNKS_RECEIPT` (catches a raise) plus a release-active `assert!(exp < EXP_MAX)` on the cold return path for the half that is a compiler fact and cannot be asserted. Cost 152 B/slot, 3.6 KiB/pool. | P28 |
| **W3** | **CLOSED without adding an attribute to any engine symbol** — a three-condition selection rule (already-attributed, non-generic, seam-relevant), and five symbols that satisfy it, each cited file:line. `Schedule::run` and `ComponentPool::new` are dropped with the reason recorded. | P29 |
| **W4** | **CLOSED by deleting the split** — `MOD_ID_BASE`, `register_at` and the re-pointed exhaustion assert are gone; a no-modding build keeps all 512 ids and changes no constant. The starvation guard is a counted cap **inside `boyko_modding`** (zero kernel instructions, zero symbols when absent). | P26 |
| **O1** | CLOSED — Q4's table replaced, not annotated; three stale cells corrected and two rows added (sanitize map, `SlotChunks`); the boot floor is **4 KiB**, not 0. | P30 |
| **O2** | CLOSED — an explicit `LINE0_PAD` plus `assert!(offset_of!(HeapHeader, free) == 64)`, so the comment is true in all three cfgs and stays true. | P30 |
| Critic Q1-Q4 | ANSWERED — G2b gains compile-time owner attribution with `Heap+Chunk+Frame+Table` pinned at 0 and `Column` reported (M-A9); M-A10 moves into rung 1a's exit criteria; `hv3-owner` becomes a closed four-value vocabulary G1 enforces; toolchain version recorded beside each pin. | P31 |
| rev-2.1 P0, P3 (except its last line), P6, P7, P9, P11, P17, P20, P22, P23 | **unchanged** — the "preserve these" list is untouched by this patch. | — |

## Open questions for pass 4

1. **P27 shrinks the Frame class to one client set, and that set may be empty.** I have written the exit criterion (delete the class rather than ship it unused), but I cannot enumerate its clients until rung 2 inventories the exclusive systems. If the list comes back empty, 256 MiB of VA and a whole primitive leave the design — and the FR-1 mechanism leaves with them, which would be the cheapest outcome and I would take it.
2. **P26 caps hot-reload at "same name, same layout".** A mod that changes a component's size across a reload is refused at load time. I judge that correct (the alternative is a resettable slot on a read path, which the requirement forbids), but it is a *product* limit, not only a technical one, and the owner may want it stated in the modding documentation before the seam is written rather than after a modder hits it.
3. **`.text` pins will churn** (P18, unchanged from rev 2.1's item 3, now with P31/4's toolchain stamp). Settled; not re-litigated.

**Files read for this patch** (all read-only): `D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/memory/component_pool.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/iters/query/relation/traverse_iter.rs`; `D:/wt/joltab/crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`; `D:/wt/joltab/crates/boyko_threadpool/src/block.rs`; `D:/wt/joltab/crates/boyko_threadpool/src/scope.rs`; `D:/wt/joltab/crates/boyko_threadpool/src/tls.rs`; `D:/wt/joltab/crates/boyko_threadpool/src/thread_pool.rs`; plus workspace-wide greps for `get_layout*`, `ScratchColumn`, `#[inline(never)]`, `MAX_CHUNKS`/`CHUNK0`.

---

# Part VI - Critique log - pass 4 (2026-09-17)

**Verdict of pass 4: CHANGES REQUESTED** - one blocker (C1), two important remarks (W1-W2), three optional notes (O1-O3), eight preserved positives, four open questions for the architect. The blocker is in text **rev 2.2 introduced** (P26's name intern), in the row that closed pass 3's C3; W1 is the file's own replace-don't-annotate rule (P30/O1) left unapplied at four sites; W2 is a consequence of P31/3's closed `hv3-owner` vocabulary that rev 2.2 did not carry back to rung 4.

Rules of this log, as in Parts II, IV and V:

- Rev 2.2 above is **unchanged**. No finding is answered by silently editing rev 2.2; every answer lands in Rev 2.3 below and names the rev-2.2 text it removes.
- The log is reproduced **verbatim as the critic wrote it**, including its own headings, its confidence tags, its "preserve these" list and its open questions. The dispositions are in Rev 2.3's change log.
- Trees as the critic states them: code read in `D:/wt/joltab` (`merge/ke16-into-ecsnative`, read-only); documents in `D:/claude/BoykoEngine`. Ledger on disk read as **rev 4**.

---

# Architecture review: allocator design space — Rev 2.2 (pass 4)

## Verdict

```
[ ] APPROVED
[X] CHANGES REQUESTED — 1 blocker, 2 important, 3 optional
```

Rev 2.2 is a real close of pass 3. I traced each closure rather than taking the change log, and three of them are better than what they replace:

- **C1 → P24** is sound as specified. I worked the safe-code sequences: with `rewind`/`FrameMark` gone, the only range a `Drop` can release is the topmost one, and a range under a live handle is never topmost because that handle's own block sits at or above it. FR-2 is correct for the reason stated twice over — `&T: Send` requires `T: Sync`, and `Cell` makes `FrameArena` `!Sync`, so `FrameVec` cannot carry a `Drop` to another thread. The rejection of the closure-scope alternative *with its specific hole* (a parent handle taken while a child scope is open) is the right way to close an alternative.
- **C2 → P25** is arithmetically correct and I checked it rather than reading it: `PAGES[c] = max(1, size_class(c)/4096)` gives 2/4/8/16 for c = 21..24, `LINKS` = 1 for those, and the all-classes worst case is `1 + 20 + (2+4+8+16) = 51` pages = 204 KiB exactly as written. The exhaustion check correctly becomes `p + pages > small_pages`. Refusing to move 8–64 KiB into the large tier **with the 8× inflation number** is the right call, and re-deriving all three dependent numbers instead of leaving them is the discipline this file keeps asking of itself.
- **W1 → P27** takes the second exit and is stronger than the first. `EcsMaster::new()` really does take no arguments (`ecs_master.rs:426`), so buying a pool identity for one client was the wrong trade. The numeric self-correction is right and I verified it: `VisitedSet.words: Vec<u64>` indexed `id >> 6` (`traverse_iter.rs:51, 60-63`) is 15,626 words = **125,008 B ≈ 122 KiB** at 1M ids, not 16 KiB. `FrameGuard` handing out two shared reborrows from one `&mut` is trivially sound and is the minimum that makes the CSR pass compile.

The one blocker is in text **rev 2.2 introduced** (P26), and it is the row that closes pass 3's C3.

All code read in `D:/wt/joltab` (`merge/ke16-into-ecsnative`, read-only); documents in `D:/claude/BoykoEngine`. Ledger on disk read as **rev 4** (no claim below depends on a per-row number in it).

---

## Remarks

### 🔴 Critical (blockers — implementation must not start on §7)

#### C1. P26's idempotency and cross-world claims rest on an intern the seam does not expose, and the intern it names cannot hold a sized component

**Where**: P26/26.1, the bullets "Idempotency is NAME-keyed…" and "The cross-world arbiter (C3's defect 3) exists by construction" (`ALLOCATOR-DESIGN-SPACE.md:2444, 2449`), against the seam's stated cost ("K-MOD-5 therefore costs the kernel **one constructor and one re-export**", `:2443`).

**Problem**: the two mechanisms do not compose, and the design names both without reconciling them.

1. The exposed entry is `try_register_dynamic` (`component_registry/mod.rs:967`). I read it: it is a bounded CAS on `NEXT_ID` plus a `LAYOUTS[..].set` and **consults no name table at all**. Called twice with the same name it mints twice.
2. The intern that supplies idempotency is `try_register_tag_by_name` (`tags.rs:182-197`), which is `pub(crate)`, is not in the seam list, and mints through `ComponentLayout::new_dynamic_tag(leaked)` — **size 0, alignment 1** (`mod.rs:173-181`). It is the only writer of `TAG_NAMES`.
3. `TAG_NAMES` is `OnceLock<Mutex<HashMap<Box<str>, TagId>>>` (`tags.rs:155`). Its value type is `TagId`, whose own documentation is the invariant that breaks: "a `TagId` is **a proof that the id was minted as a size-0 dynamic tag**, and only the `TAG_NAMES` mint path can issue one" (`tags.rs:38-40`). `TagId(pub(crate) ComponentId)` is constructible inside `boyko_ecs`, so a developer wiring the mod intern into this map compiles on the first attempt.

**Consequence** (both exits, named):

- **Exit A — the mod crate keeps its own map.** P26 amends K-MOD-8 so `boyko_modding`'s registry is a per-world `Resource` holding "only per-world state" (`:2449`), so two `EcsMaster`s in one process — the shape §1 says the tests exercise — mint two ids for `"mymod::Position"`, and every world teardown/reload re-mints. The budget is then spent by *registrations*, not by *distinct names*: this is pass-2 **W6 returning verbatim**, and C3's defect 3 is not closed but relocated.
- **Exit B — mod names go into `TAG_NAMES`, as `:2444` and `:2449` literally say.** Then `EcsMaster::tag_by_name` (`pub`, `tag_api.rs:76`) returns a `TagId` naming a 12-byte component, and `EcsMaster::add_tag` (`pub`, `tag_api.rs:130`) migrates the entity into an archetype hosting that id — its own doc: "**tag columns themselves copy zero bytes**" (`tag_api.rs:129`). The row's component data is never initialised, and the next read of it by the mod or by a query reads whatever the pool holds. A documented invariant of a shipped public type is falsified from safe code on a `pub` path.

**Confidence**: CONFIRMED (traced: `mod.rs:967` has no intern; `tags.rs:155` value type; `tags.rs:38-40` the ZST proof; `tags.rs:182-197` the only mint that inserts, at size 0; `tag_api.rs:76, 129-130`).

**Why critical**: this is the row that closed pass 3's C3, and pass 3's standard was "it must be ruled before any seam entry is written" because the repair direction decides whether §7 stays additive. It also carries the owner's newest hard requirement: exit B's repair (a second value type, or a kind check in the map) is a change to a shipped kernel path, and its cost against G6 has to be priced rather than asserted.

**What is needed**: rule which table holds a mod type name, what handle type it yields, and where it lives — such that (a) the cross-world claim survives without demoting it back to per-world state, (b) `TagId`'s size-0 proof is either preserved or explicitly restated with every consumer of `tag_by_name`/`add_tag` re-argued, and (c) the "one constructor and one re-export" cost line is corrected to whatever the answer actually costs. Note the constraint that forces the fork is one field type in `tags.rs:155`, not the mint.

---

### 🟡 Important

#### W1. Four superseded passages were left standing, and one of them is the only written justification for an `unsafe impl Sync`

**Where**: `:1942` (P19's safety argument), `:389-390` (§5.1's consumer table), `:1206` (K-MOD-6), `:1607` (P15/15.4's Miri table row).

**Problem**: P30/O1 established the rule for exactly this — a superseded table is **replaced, not annotated**, because the file's own header warns that a removed sentence still reads as current where it stands. Rev 2.2 applied the rule once and left four instances:

1. **`:1942`** — P27 removed P19/19.1's ruling sentence and points 1-3, but not the paragraph under them: "each slot's arena has exactly one live user at a time by the SAME rule that gives `SlotChunks` one owner (P17/SC-1, SC-2) … `unsafe impl Sync` gains bullet **SEND11** naming the slot rule … The `debug_assert` moves to 'the resolved slot is this thread's slot'". There are no slots now, and there is no such `debug_assert` to write. *Consequence*: SEND11 is a new `unsafe impl Sync` bullet over a `Cell`-bearing field, and the only rationale text in the design cites a rule the design deleted — the tree's own recorded failure class ("a SAFETY comment answering the wrong rule"). The correct argument does exist, in P27's `FrameGuard` doc comment ("the exclusivity is the borrow"); it is the wrong one that sits under the SEND11 heading.
2. **`:389-390`** — §5.1 still instructs consumers on `Children(HeapVec<Entity>)` and on `EcsMaster::frame() -> &mut FrameArena`. P19/19.2 withdrew the first (HV-4 forbids it; `Children` is a relation) and P27 replaced the second with `frame(&mut self) -> FrameGuard<'_>`. §5.1 also does not carry P27's **removal of `DescendantsIter::new`**, which is a breaking public change: the type is in `prelude.rs` and is used by `boyko_scene/tests/gates_composition_structural.rs`, `boyko_ecs/tests/relations_query_{scaling,dsl}.rs` and `relationship_api.rs`.
3. **`:1206`** — K-MOD-6 still reads "A mod supplies an `extern \"C\"` thunk" for `drop_fn`, which P26 constraint 1 forbids outright (`drop_fn: None`, asserted at the seam). G6(e) pins "the set of kernel items … equals the named list K-MOD-1..K-MOD-10, **exactly**", so the inventory the gate pins contains a contradicted entry.
4. **`:1607`** — 15.4's Miri table still reads "FrameArena, **per slot** | 64 MiB", superseded by P27's single 256 MiB arena (P30's replacement table has it right, which is what makes the two disagree).

**Confidence**: CONFIRMED (plan text vs plan text at the four cited lines; the `DescendantsIter` blast radius from a workspace grep).

**Solution options**: apply P30/O1's own rule — one `**Removed**`/`**Added**` block each; for (1) the replacement text already exists in P27 and only needs to be where SEND11 is derived; for (3) either delete K-MOD-6 from the inventory or restate it as "the kernel's existing `drop_fn` field, never written for a mod id".

#### W2. After P31/3's closed `hv3-owner` vocabulary, `HeapString` has no legal owner and rung 4's stated destinations are unreachable

**Where**: P31/3 (`:2700`) — "`hv3-owner` takes a value from a CLOSED vocabulary — `schedule`, `schedule-builder`, `registry`, `master-table` — and G1 refuses any other value… A `Resource` is not an owner class; a `Local` is not an owner class" — against Part I rung 4 (`:380`) and §2.3's `HeapString`.

**Problem**: rung 4 sends `boyko_log` strings, asset `error.rs` strings, UI text AST/lower/report, `boyko_app` titles/profiling and the serialize/prefab paths to `HeapVec`/`HeapString`. None of those owners is one of the four permitted values; the log ring in particular is written from worker threads. P31/3 already worked this out for one case ("a `SparseMap` user that is not one of the four **cannot become a heap row** and takes `VmColumn` instead") but did not carry the consequence back to the rung table or to the primitive list.

**Consequence**: two, and they are different. (i) Rung 4 cannot land as written: G1 refuses each of those heap rows, and the pressure at the keyboard is to add a fifth owner value, which dissolves the closed vocabulary that makes HV-3 provable. (ii) `HeapString` is then a primitive whose client list is empty — and this design's own retirement criterion is that a client-less primitive does not ship ("No `Allocator` impl ships in v1 — it would be client-less, the exact X.J retirement criterion", §1). P27 applied that criterion honestly to the Frame class; the same test is owed here.

**Confidence**: CONFIRMED (plan vs plan: `:2700` vocabulary against `:380` destinations).

**Solution options**: re-point rung 4's string rows at `ByteColumn`/`VmColumn<u8>`/`InlineStr<N>` and give `HeapString` the Frame class's exit criterion (delete if rung 4 ends with no legal owner); or state the fifth owner class explicitly with the HV-3 argument for it, rather than letting it be invented per row.

---

### 🟢 Optional

#### O1. `ChunkArena::carve` never states the alignment it returns, and `ScopeBlock` depends on one

`block.rs:410-411` rests its whole padding argument on "`bases[i]` is `CHUNK_ALIGN`-aligned", and `grow` allocates at `Layout::from_size_align(cap, CHUNK_ALIGN)` — **always 64, never the emplaced type's alignment** (`block.rs:109, 474`). The carve satisfies it (frontier starts at 0, every `cap` is a power of two ≥ 4096, so every carve is 4096-aligned), but P16/P17 never say so, and this is precisely the premise that changes when `std::alloc` — which honours a `Layout` — is replaced by a bump. One line in `carve`'s contract pins it.

#### O2. G6(b) measures a per-symbol `.text` size, which the host's object format does not carry

P18/18.2 chose `llvm-nm`/`llvm-objdump` so the tool does not change per host. The *quantity* still does: ELF symbols carry `st_size`, COFF symbols do not, so on the shipped windows-gnu/msvc legs a per-symbol size has to be inferred from the next symbol's address — which moves when an unrelated function is placed between them, producing re-bless churn that is not a seam change and is indistinguishable from one. Two of the five pinned symbols are additionally private/`pub(crate)` (`commit_subregion` is a private `fn` at `component_pool.rs:560`, `grow_rows` is `pub(crate)` at `:601`), so they carry local linkage. State how the number is obtained on PE/COFF, or pin something the format guarantees.

#### O3. G2b's owner attribution may not reach the `Table` and `Column` arms separately

P31/1 puts `const OWNER: CommitOwner` on `raw::commit_at`, but P15 makes `VmReservation::commit` a two-line wrapper over it. If the wrapper hard-codes one owner, every `VmColumn`, `ComponentPool` and `TableSet` commit lands in one bucket and the `Table` arm is pinned at 0 vacuously. G2b's own anti-vacuity direction catches this at setup (it requires nonzero per used owner class), so it is self-correcting rather than dangerous — but one sentence saying whether the wrapper is owner-parameterised saves a rung-time discovery.

---

## Status of earlier findings

| Round | Row | Status |
|---|---|---|
| pass 3 | C1 `FrameArena::rewind` under `&self` | ✅ closed by deletion (P24); FR-1/FR-2 traced sound |
| pass 3 | C2 small-tier classes above `COMMIT_PAGE` | ✅ closed (P25); arithmetic re-checked, all three derived numbers re-derived |
| pass 3 | C3 mod-id recycling | ⛔ **partially closed** — the id-release refusal and the POD/`drop_fn: None` constraints are right and close defects (1) and (2); defect (3) and the idempotency mechanism are reopened as pass-4 C1 |
| pass 3 | W1 per-slot arena not expressible | ✅ closed (P27), modulo the stale paragraph in W1 above |
| pass 3 | W2 `EXP_MAX` | ✅ closed (P28) — 51, `>=` assert catches a raised `MAX_CHUNKS`, release-active bound check for the half that is a compiler fact |
| pass 3 | W3 pinned symbols | ✅ closed (P29) — I verified `commit_subregion` `#[cold] #[inline(never)]` at `component_pool.rs:558-560` and `grow_rows` at `:599-601` |
| pass 3 | W4 `MOD_ID_BASE = 384` | ✅ closed by deletion (P26) — a no-modding build keeps all 512 ids |
| pass 3 | O1, O2 | ✅ closed (P30) |
| pass 3 | open questions 1-4 | ✅ answered (P31) |
| pass 2 | C1-C4, W1-W6, O1-O3 | ✅ stand closed; nothing in rev 2.2 reopens them (P27 re-derives W1's fix without weakening HV-4) |
| pass 1 | C1, C2, W1-W5, O1-O9, Q1-Q6 | ✅ stand closed |

---

## Positive — preserve these

1. **P24's FR-1.** Making reclamation a property of `Drop` rather than of a token is the correct structural answer, and "the only range a `Drop` releases is one the compiler has just proved unowned" is the sentence that makes it checkable. Keep the rejected closure-scope alternative **with its code example** — an alternative closed with its specific hole cannot be re-proposed by accident.
2. **P25's refusal to move the four classes to the large tier, priced.** The 8× resident inflation on the exact clients rung 3 routes there is the right kind of number to refuse with, and 25.2 turning "the large tier has no clients" from a claim into a pinned counter with two named thresholds (724 systems, 2730 entries) is the better half of that patch.
3. **P26's verified blast radius and its refusal.** I re-ran it: `get_layout_unchecked` has exactly one caller outside the registry (`component_pool.rs:285`; the other two are in-module wrappers). Refusing a resettable `LAYOUTS` — and refusing to *price* what it refuses — is right, and `drop_fn: None` + the kernel-interned `type_name` kill the dangling-code-pointer and dangling-`&'static str` classes outright rather than managing them. This part of P26 survives C1 intact.
4. **P27's numeric correction of its own critic.** 122 KiB, not 16 KiB, verified at `traverse_iter.rs:51, 60-63` — and the correction is used to *strengthen* the ruling against the arena rather than to score a point.
5. **P29's refusal to add an attribute for the gate's benefit.** "I am not paying that" is the correct answer to a gate that would have bought its own subject with a codegen change to the engine's hot path, and the three-condition selection rule with five file:line-cited symbols makes the replacement checkable. The stated limit ("a `.text` pin cannot see a change *inside* an `#[inline]` function — that is what (e) is for") is what stops the gate being believed past its reach.
6. **P28's honesty about what cannot be asserted.** "The `min_e <= 50` half cannot be asserted in code (it is a property of rustc's layout rules)" plus a release-active bound check is exactly right; I verified the derivation is in `block.rs:439-455` at the site that depends on it.
7. **P30/O1's replace-don't-annotate.** It is the right rule; W1 above is only a request to finish applying it.
8. Everything pass 2 and pass 3 asked to be preserved still stands untouched: P0's withdrawn throughput justification, P3's protector-kind argument, P6's syntax-over-text ledger, P11's TB-1 rationale correction, P17's release-sequence argument, P20's push-side liveness proof.

---

## Open questions for the architect

1. **Does the Frame class have a client at all?** Your own pass-4 question 1, and after P27 it is sharper: the class's client list is "dispatcher-side scratch inside an exclusive system", and W2 above shows `HeapString` may be in the same position. If rung 2's inventory comes back empty for both, two primitives and the FR-1 mechanism leave together — which you say you would take, and I agree. The measurement that settles it is the rung-2 inventory of exclusive-system scratch, not a benchmark.
2. **Hot-reload capped at "same name, same layout"** (your question 2): I concur it is correct technically, and I agree it is a product limit that belongs in `boyko_modding`'s documentation. It is also downstream of C1 — the mismatch check lives in whichever intern C1 rules on, so write it after that ruling, not before.
3. **Is `ScratchColumn` able to hold `(Entity, u32)` and `u64`?** P27's `TraversalScratch` uses both. `VmColumn`'s divisibility pin is relaxed by §2.2/P5, but `ScratchColumn` is a separate `ComponentPool`-backed shape in `boyko_ecs`; one line confirming the element-size rule applies there too would keep rung 1e from discovering it.
4. **`ensure_len_zeroed` after a shrink.** The zero-fill contract is "fresh commit only" (`vm.rs:19-37`), so a column that truncated and regrew reads stale, non-zero bytes where the `slot+1`/zero encoding expects absent. No named client shrinks today (`EntitySlotMap`, `LiveBitmap`, `EnableStore.pages` only grow), so this is a contract line rather than a defect — but it is the kind of thing that is true until the first client that truncates.

---

# Rev 2.3 (2026-09-17)

# Allocator design — Rev 2.3 (patch against Rev 2.2)

**Scope of this round.** Pass-4 C1 (blocker), W1, W2, O1-O3, and pass-4 open questions 1-4. Six patches: **P32** (C1), **P33** (W1 items 2-3), **P34** (W1 items 1 and 4, by deleting the Frame class — pass-4 question 1), **P35** (W2), **P36** (O1-O3), **P37** (questions 2-4).

**Trees.** Documents in `D:/claude/BoykoEngine` (`feat/multi-paradigm-render`). All code read in `D:/wt/joltab` (`merge/ke16-into-ecsnative`, read-only). Ledger on disk read as **rev 4** at `d552be05` (`RUNTIME-DATA-LEDGER.md:1, :9`) — P34 depends on three of its rows and cites them.

**Two stale passages this round found that pass 4 did not name** (same class as W1, flagged so the count is not silently three): K-MOD-5's Shape cell still describes `register_at` and `MOD_ID_BASE` (`:2029`) although P26's change log says both are deleted; the "Mandatory unit tests" bullet still requires a `FrameArena` mark/rewind LIFO `debug_assert` (`:438`) although P24 deleted `rewind` and `FrameMark`. Both are removed below.

---

## P32 (closes pass-4 C1) — the dynamic-name intern is ruled: one process-global table, value type narrowed to `ComponentId`, kind re-derived from the layout

**Depends on:** P26 (which this amends in five places), P12 (K-MOD-4/5/7, G6), P21 (K-MOD-5's Shape cell), P31/4 (the toolchain stamp on pins).

**Code the ruling rests on, each read in `D:/wt/joltab`:**

| fact | site |
|---|---|
| `TAG_NAMES: OnceLock<Mutex<HashMap<Box<str>, TagId>>>` — value type is the constraint | `component_registry/tags.rs:155` |
| `TagId` = "a proof that the id was minted as a size-0 dynamic tag, and only the `TAG_NAMES` mint path can issue one" | `tags.rs:38-40` |
| `TagId(pub(crate) ComponentId)`, `#[repr(transparent)]` | `tags.rs:47-49` |
| the only inserter mints at size 0 / align 1 | `tags.rs:182-197` via `mod.rs:173-181` |
| `try_register_dynamic` consults no name table and mints unconditionally | `mod.rs:967-992` |
| `EcsMaster::tag_by_name` → `Option<TagId>` for any interned name, no kind test | `tag_api.rs:76-78` |
| **the kernel already uses the layout as the tag-ness oracle**: "A data component routed through here would leave its bytes uninitialized; the registry layout is the oracle", plus a `debug_assert!` that every added id `is_zst()` | `commands/migration_helpers.rs:1573-1581` |
| the enable-tag mint goes through the *same* map and then reclassifies to `Bitset` | `tags.rs:134-141` |
| `get_or_create_archetype` filters `Bitset` ids out of the signature mask | `archetype_master.rs:694-702`, test at `:1455-1466` |
| `storage_kind` / `get_layout` are the shipped, `pub`, id-keyed accessors | `mod.rs:390`, `mod.rs:1125` |
| the physics scratch band owns the **top 128 ids** with a compile-time floor | `boyko_physics/src/scratch_ids.rs:685`, assert at `:690-695`, census at `:676-684` |

### 32.1 The ruling

**R1. One table, and its value type is `ComponentId`, not `TagId`.** `TAG_NAMES` is renamed `DYN_NAMES` and becomes `OnceLock<Mutex<HashMap<Box<str>, ComponentId>>>`. This is the one field the critique identified, and it is the whole fork: `TagId` is `#[repr(transparent)]` over `ComponentId` (`tags.rs:47-49`), so **the map's value is the same word and the same bytes** — the change costs no memory and no instruction, it removes a claim the map was making and could not keep. ⚠ *Rev 2.5 (P47): the rename is option A's, modding Stage 3 (05 MS-02b); D-S1(i)'s rename list is empty and the map keeps its name until then (05 RM-3).*

**R2. Kind is never stored; it is re-derived from the tables that are already authoritative.** The lookup path computes `is_zst = get_layout(id).is_some_and(|l| l.is_zst())` and `kind = storage_kind(id)` — two cold loads from `LAYOUTS`/`STORAGE_KIND` — and hands back a handle only when they agree with the handle's own invariant:

| accessor | returns `Some` iff | handle |
|---|---|---|
| `tag_by_name` | `size == 0 && kind == Table` | `TagId` |
| (enable-tag accessor, **not added until a client exists**) | `size == 0 && kind == Bitset` | `EnableTagId` |
| `dynamic_by_name` (seam) | interned at all | `ComponentId` |

`TagId`'s documented invariant (`tags.rs:38-40`) is preserved **verbatim** and strengthened in mechanism: today its proof is a social fact ("only this path inserts"), afterwards it is a re-read of the layout that *defines* tag-ness. That is not an invention — it is the rule the kernel already applies at the one site where being wrong corrupts a row (`migration_helpers.rs:1573-1581`).

**R3. The namespace is flat: one name, at most one id, whatever its kind.** Justified, not preferred: the name is the stable serialization key (`tags.rs:42-46`, `:88-90`), so two ids under one name in one process makes a save-file key ambiguous. Cross-world arbitration (C3's defect 3) is then the map itself, honestly this time — the map is process-global, the seam entry is its only writer for sized ids, and no per-world state participates.

**R4. A kind clash is loud, and its loudness differs by trust level.** Engine-authored input panics; mod-supplied input returns an error.
- `try_register_tag_by_name("x")` where `"x"` is interned as a sized type → `#[cold] #[inline(never)] dynamic_name_kind_clash_panic(name, existing_size)`, modelled on `dynamic_slot_occupied_panic` (`mod.rs:996-1010`). It must not return `None`: `None` on that API means *budget exhausted* (`tag_api.rs:39-41`), and returning it for a name clash would be a confident wrong answer on a shipped contract.
- the seam returns `Err(DynMintError::KindMismatch)`. A mod's manifest is untrusted input; untrusted input must not panic the host.

**R5. Size-0 mod types are dynamic tags, and the kernel never learns the word "mod".** A mod component of size 0 goes down the existing tag path and *is* a dynamic tag (`add_tag` copies zero bytes, which is correct for a ZST). A mod component of size > 0 goes down the sized path, and `add_tag` cannot receive it, **because the type system refuses to build the handle** under R2 — not because a rule says so. Classification is by layout, never by origin, which is exactly the property K-MOD-4 buys and G6(b) pins.

### 32.2 What R2 also fixes in the shipped kernel (found while tracing C1)

The kind check is not a modding cost; it repairs a defect reachable today from three `pub` methods with no `unsafe`:

1. `register_enable_tag("x")` mints through the tag path and reclassifies the id to `Bitset` (`tags.rs:134-141`).
2. `tag_by_name("x")` returns `Some(TagId)` for that bitset id — no kind test (`tag_api.rs:76-78`, `tags.rs:201-207`).
3. `add_tag(e, t)`: `has_component_id` is false (bitset ids are not in signatures), so `merged_archetype_id_dyn` runs, `get_or_create_archetype` filters the bitset id out of the mask (`archetype_master.rs:694-702`), and the union **collapses to the source archetype**.
4. `debug_assert_ne!(target, source)` (`tag_api.rs:166-169`) fires in debug; in **release** both that assert and `migrate_entity_attach_ids`'s own (`migration_helpers.rs:1565-1568`) are compiled out, and the entity is migrated from an archetype into itself with `on_add`/`on_insert` fired for a bitset id.

Under R2 step 2 returns `None` and the chain cannot start. This is stated as **CONFIRMED by reading, not by running** — the four sites are cited; no test was executed (the machine is the owner's).

### 32.3 The seam entries, and the corrected cost line

**Removed** (P26/26.1, the second "Added" bullet, `:2443`, verbatim):

> - **The seam entry is a generalisation of a path that already ships, not a new mechanism.** `ComponentLayout::new_dynamic_tag(name)` (`:173-181`) gains a sibling `new_dynamic(name, size, align)` with the **same** `DynamicTagMarker` sentinel `TypeId` (renamed `DynamicMarker` in docs only) and **`drop_fn: None`**; `try_register_dynamic` (`:967` — a bounded CAS on the shared `NEXT_ID`, returning `None` at the ceiling rather than panicking) is re-exported as the `#[doc(hidden)] pub`, non-generic, `#[cold]` seam entry. **K-MOD-5 therefore costs the kernel one constructor and one re-export.** Everything the critic asked the kernel to grow — a free list, a resettable `LAYOUTS`, an id-origin branch — does not exist.

**Added:**

> - **`try_register_dynamic` is NOT the seam entry, and re-exporting it was the defect.** It is a bounded CAS plus a `LAYOUTS[..].set` that consults no name table (`mod.rs:967-992`): called twice with one name it mints twice, which is pass-2 W6 returning verbatim. It stays `pub(crate)` and un-exported. The seam entry is the **name-keyed mint**, which the kernel does not have today for sized layouts and now gains:
>   ```rust
>   // component_registry/tags.rs — all #[cold]; DYN_NAMES is the renamed TAG_NAMES.
>   /// Lookup, capacity check, layout compare, mint and intern under ONE lock —
>   /// the discipline try_register_tag_by_name already documents (tags.rs:169-171).
>   fn intern_or_mint(name: &str, size: usize, align: usize) -> Result<ComponentId, DynMintError>;
>
>   /// MOD-SEAM K-MOD-5. Non-generic, #[cold], #[doc(hidden)] pub.
>   pub fn try_register_dynamic_by_name(name: &str, size: usize, align: usize)
>       -> Result<ComponentId, DynMintError>;
>   /// MOD-SEAM K-MOD-5. Never mints.
>   pub fn dynamic_by_name(name: &str) -> Option<ComponentId>;
>
>   pub enum DynMintError {
>       IdSpaceExhausted,                                   // the ceiling of 32.4
>       LayoutMismatch { size: usize, align: usize },        // the hot-reload cap
>       KindMismatch,                                        // R4, mod side
>       UnsupportedAlign,                                    // ComponentPool::new's ceiling, checked up front
>   }
>   ```
>   `ComponentLayout::new_dynamic(name, size, align)` is still added, still with the `DynamicTagMarker` sentinel and **`drop_fn: None` asserted at the seam**; `try_register_tag_by_name` and `try_register_enable_tag_by_name` are rewritten as two-line callers of `intern_or_mint(name, 0, 1)` plus their kind filter, so there is **one** mint body, not three.
> - **`UnsupportedAlign` is checked before the pool sees the layout**, because `ComponentPool::new` panics on an alignment above its ceiling (`component_pool.rs`, `new`'s contract) and a mod manifest must not be able to panic the host. Rung-time obligation: read that ceiling at the site and cite it in the seam's doc comment rather than restating a number here.
> - **Corrected cost line. K-MOD-5 costs the kernel one constructor (`new_dynamic`), one narrowed field type (`DYN_NAMES`'s value), one shared mint body replacing three, two `#[doc(hidden)] pub` entries, one error enum, and two kind filters inside functions that are already `#[cold]`.** No new static, no new table, no new map, no read-path change: `get_layout_unchecked` is untouched, and P26's verified blast radius (one caller, `component_pool.rs:285`) stands.

**Removed** (P26/26.1, the third "Added" bullet, `:2444`, verbatim):

> - **Idempotency is NAME-keyed, which is why recycling is unnecessary rather than merely unavailable.** The sentinel `TypeId` is shared by every dynamic mint, so the registry's own rule already forbids the `TypeId`-keyed idempotent arm and mandates a name intern (`:199-202`); the shipped intern is process-global (`tags.rs:7`, `TAG_NAMES`, behind the blessed `#[allow(clippy::disallowed_types)]` at `tags.rs:17-21`). Keyed on `"<mod>::<Type>"`, a **load → unload → reload of the same mod re-resolves to the same id and consumes nothing**. The budget is spent by *distinct mod type names ever seen in the process* — precisely the rule the engine's own Rust types already follow. W6's cap becomes "128 distinct mod type names", which is the honest statement of the same budget the engine lives under, not the "128 registrations ever" trap.

**Added:**

> - **Idempotency is NAME-keyed and it is now the seam's own mechanism, not an intern it merely names.** `intern_or_mint` does lookup → layout compare → capacity check → mint → intern under the `DYN_NAMES` lock, so two threads racing one name cannot double-mint (the property `tags.rs:169-171` already documents for tags) and a **load → unload → reload of the same mod re-resolves to the same id and consumes nothing**. The budget is spent by *distinct dynamic names ever seen in the process*, which is the rule the engine's own Rust types already follow. W6's cap is "128 distinct mod type names" **as a self-restraint inside `boyko_modding`**; the number that actually bounds it is the id ceiling of 32.4, and the two must be checked against each other at load time, not assumed equal.

**Removed** (P26/26.1, the "cross-world arbiter" bullet, `:2449`, verbatim):

> - **The cross-world arbiter (C3's defect 3) exists by construction.** Both the mint (`NEXT_ID`) and the name intern (`TAG_NAMES`) are process-global, so two `EcsMaster`s in one process resolve one mod type name to one id. **K-MOD-8 is amended:** `boyko_modding`'s per-world `Resource` is no longer an id owner; it holds only per-world state (which mods are loaded *here*, which ids are live *here*).

**Added:**

> - **The cross-world arbiter (C3's defect 3) is `DYN_NAMES`, and the seam is its only writer for sized ids.** Both the counter (`NEXT_ID`) and the intern are process-global and the seam entry cannot bypass the intern (`try_register_dynamic` is not exported), so two `EcsMaster`s in one process resolve one mod type name to one id **and the property is enforced by the API surface rather than asserted about it**. **K-MOD-8 is amended:** `boyko_modding`'s per-world `Resource` is not an id owner; it holds per-world state only (which mods are loaded *here*, which ids are live *here*).

**Removed** (P26/26.1, the first "Added" bullet, `:2442`, the clause that is false against `scratch_ids.rs:685`, verbatim):

> - **No range split. `MOD_ID_BASE` is deleted, `register_at` is deleted, and `NEXT_ID`'s exhaustion assert is untouched.** A build with no modding crate keeps **all 512** ids and changes not one constant — which is what the owner's requirement asks for, and which the 384/128 split did not deliver (W4: the runtime cost was zero, the *capability* cost was a quarter of the budget, charged to exactly the games that never load a mod).

**Added:**

> - **No range split. `MOD_ID_BASE` is deleted, `register_at` is deleted, and `NEXT_ID`'s exhaustion assert is untouched.** A build with no modding crate **changes not one constant and loses not one id it has today** — which is what the owner's requirement asks for. The stronger claim rev 2.2 made ("keeps all 512") was wrong and the correction makes W4's case *harder*, not softer: `boyko_physics` already pins the **top 128 ids** as its synthetic scratch band (`scratch_ids.rs:685`, `SCRATCH_REGION_MIN_ID = MAX_COMPONENTS - 128`, with a compile-time floor assert at `:690-695` and a 142-type census at `:676-684`), so the production range is **384**, not 512 — and P21's mod range `[384, 512)` was **byte-for-byte the physics band**. The split was therefore not merely expensive, it was unimplementable: `register_at` in that range would have hit `register_layout`'s different-type panic (`mod.rs:1026-1029`) on a physics scratch slot. Deleting it was right for a second, harder reason than W4 gave.

**Removed** (P21, K-MOD-5's Shape cell, the leading sentence of its "Added" block, `:2029`, verbatim — pass 4 did not name this one; it contradicts P26's own change log):

> the id counter is `component_registry::register_new::<T>()` (`:920-921`, `NEXT_ID.fetch_add(1, Relaxed)`), generic only to key the `TypeId` mint. **The additive seam does NOT mint** — it registers at a caller-supplied id: `register_at(id: usize, layout: Layout, drop_fn: Option<DropFn>)`, `#[cold]`, non-generic, asserting `MOD_ID_BASE <= id < MAX_COMPONENTS` and that the slot is unoccupied (the collision detection already at `:918-919`). Reason, and it is a budget fact rather than a style preference: the id space is hard-capped at **512** (`:63`), materialised as an inline `columns: [Column; MAX_COMPONENTS]` per archetype (`archetype.rs:140`) and shared with enable-tags (`enable_tag_api.rs:55`), so a monotonic `fetch_add` would let a load/unload/reload cycle **burn the engine's own budget permanently**. Instead:

**Added:**

> the id counter is `component_registry::register_new::<T>()` (`:920-921`, `NEXT_ID.fetch_add(1, Relaxed)`), generic only to key the `TypeId` mint. **The additive seam mints through the shared counter, and is safe to do so because it is name-idempotent** (P32/R1-R3): a load → unload → reload consumes nothing, so the "burn the budget permanently" hazard that motivated `register_at` does not arise. The id space is hard-capped at **512** (`:63`), materialised as an inline `columns: [Column; MAX_COMPONENTS]` per archetype (`archetype.rs:140`) and shared with enable tags (`enable_tag_api.rs:55`) and with the synthetic scratch band (`boyko_physics/src/scratch_ids.rs:685`); what bounds the mint is the ceiling of P32/32.4, not `MAX_COMPONENTS`.

### 32.4 The ceiling is a defect in the shipped tree, and it is fixed additively

`try_register_dynamic` stops at `MAX_COMPONENTS` (`mod.rs:970`) and `register_new` asserts against it (`:922-927`). Neither knows about the reserved band. The band's own protection is a *margin argument* — "reaching that floor from the production counter takes 384 distinct component types against a measured ~142" (`scratch_ids.rs:18-24`) — and a compile-time assert that the band does not grow **downward** (`:690-695`). Nothing stops the counter climbing **upward** into it. At 384 distinct types the engine does not report a budget exhaustion; it panics inside `LAYOUTS[raw].set` with "ComponentId N occupied by type X" (`mod.rs:938-943`), or, on the dynamic path, in `dynamic_slot_occupied_panic` with a message that blames a "test-only escape hatch" (`:1004-1009`). Mods make this worse by construction, because mod names draw from the same upward counter.

**Ruling: the ceiling is derived from the pins that exist, not declared as a constant.**

```rust
// component_registry/mod.rs
/// The lowest id any out-of-band pin has claimed. Lowered by `register_layout`,
/// never raised. A build that pins nothing keeps the full MAX_COMPONENTS.
static DYN_ID_CEILING: AtomicUsize = AtomicUsize::new(MAX_COMPONENTS);
```

- `register_layout::<T>(id)` adds `DYN_ID_CEILING.fetch_min(id, Relaxed)` — one relaxed RMW on a setup-only path that already does a `OnceLock::set`.
- `try_register_dynamic` and `register_new` compare against `DYN_ID_CEILING.load(Relaxed)` instead of the constant. `register_new` is reached once per type behind the derive's per-monomorphisation `OnceLock`; the load is not on any per-frame path.
- `Relaxed` is sufficient and the argument is the same one the module already makes for `NEXT_ID` (`mod.rs:34-39`): the atomic provides a *bound*, not a publication; the payload is published by `OnceLock::set`. A stale-low read refuses a mint that would have succeeded (fail-closed); a stale-high read cannot occur before the pin, because a pin that has not happened cannot be collided with.

**Cost when modding is absent:** one `AtomicUsize` in `.bss`, one relaxed RMW per `register_layout` call (≈ 90 calls at boot in a physics build, per the `scratch_ids.rs` census), one relaxed load per *first* touch of a component type. Zero per-frame instructions, no `size_of` change, no new symbol in the G6(b) set, no startup allocation. **It is a bug fix the engine wants on its own terms** — the owner's standing rule is bugs before features — and the modding seam is its second beneficiary, not its justification.

**Gate (new row in `tests/registry_id_ceiling.rs`):** pin 400; assert the next `try_register_dynamic_by_name` returns `Err(IdSpaceExhausted)` rather than panicking, and that the error names the ceiling and the pinning site's id. Red-first: delete the `fetch_min` → the mint walks into the pinned slot and the test observes the occupancy panic instead of the error.

**Measurement entry (specified, not run): M-A13** — the distinct-id census at steady state for `boyko_demo` and the playground scene: `NEXT_ID` high-water, the count of pinned ids, and `DYN_ID_CEILING`'s final value. This is the number `boyko_modding`'s `MAX_MOD_TYPES` must be checked against; until it exists, 128 is a self-restraint with no headroom proof.

### 32.5 K-MOD-11 — the sized attach, priced and refused a refactor

`add_tag` is not the attach path for a sized dynamic component, and the kernel says so itself: `migrate_entity_attach_ids` `debug_assert!`s that every added id is size-0 and states the consequence — "A data component routed through here would leave its bytes uninitialized" (`migration_helpers.rs:1573-1581`). The typed path that does write bytes is `migrate_entity_insert<B: Bundle>` (`:384`), **generic over the bundle**, so it is not a re-export. The pair "`add_tag` then `set_component_raw` (`component_api.rs:461`)" is refused as well: `set_component_raw` never creates membership (`:434-437`), and even if it did, `on_add`/`on_insert` observers fire inside the migration, i.e. *before* the bytes exist.

> **K-MOD-11 (NEW seam entry): `EcsMaster::add_component_raw(entity: Entity, id: ComponentId, bytes: &[u8]) -> bool`** — `#[cold] #[inline(never)]`, non-generic, MOD-SEAM-marked. Migrates `source → source ∪ {id}`, memcpys `bytes` into the new row (length checked against `get_layout(id).size`, refused on mismatch), stamps the added tick, and only then fires `on_add`/`on_insert`. It duplicates the migration skeleton of `migrate_entity_insert` with the bundle write replaced by one erased memcpy.
>
> **The refactor that would remove the duplication is priced and refused.** Sharing one erased core would replace a monomorphised, constant-size memcpy on the engine's own structural-insert path with a runtime-length one, paid by every game on every `insert`, to save duplicated code inside a `#[cold]` function. Cost when modding is absent: **zero** — no caller, so the function is dropped at link, and none of P29's five pinned symbols contains it.

### 32.6 The gate

**Removed** (26.2, verbatim):

> New rows in `tests/mod_seam_pins.rs`: (i) two registrations of one `"<mod>::<Type>"` name return the **same** id and mint nothing (`NEXT_ID` unchanged) — red-first: key the intern on the pointer instead of the string; (ii) a registration with `drop_fn: Some(_)` is refused at the seam — red-first: drop the assert; (iii) a second registration of a name with a different size returns `DynamicLayoutMismatch`; (iv) `HOOKS[id]` is `None` for every mod id after a load/unload/reload cycle.

**Added:**

> New rows in `tests/mod_seam_pins.rs`, each with its red-first mutation:
> 1. two `try_register_dynamic_by_name` calls with one name return the **same** id and leave `NEXT_ID` unchanged — RED: key the intern on the pointer instead of the string.
> 2. `drop_fn: Some(_)` is refused at the seam — RED: drop the assert.
> 3. a second registration of a name with a different `{size, align}` returns `Err(LayoutMismatch)` — RED: drop the compare.
> 4. `HOOKS[id]` is `None` for every dynamic id after load → unload → reload.
> 5. **`tag_by_name` is kind-exact**: a name minted through `try_register_dynamic_by_name` with `size > 0` yields `None` from `tag_by_name`, and a name minted by `register_enable_tag` also yields `None` — RED: delete either kind filter, and the test must then observe the 32.2 chain (in debug, the `tag_api.rs:166-169` assert fires; the test asserts *that* diagnosis text, per the tree's gate convention).
> 6. **the seam does not panic on untrusted input**: kind clash, layout mismatch, unsupported align and ceiling exhaustion each return their `DynMintError` variant — RED: replace any one with an `expect`.
> 7. two `EcsMaster`s in one process resolve one name to one id (the cross-world claim, as a test rather than as prose).
>
> Test rows 1-7 sit beside the `.text` pins, whose toolchain stamp discipline (P31/4) is unchanged.
>
> **Migration obligation, because row 5 changes shipped behaviour:** the kind filter makes `tag_by_name` return `None` where it returns `Some` today for an enable tag. The rung sweeps the workspace for `tag_by_name` callers before the filter lands and records the count in the test header; a caller that depended on the old answer was depending on the defect of 32.2.

### 32.7 What a no-modding build pays for all of P32

| item | cost when `boyko_modding` is absent |
|---|---|
| `DYN_NAMES` value narrowed `TagId` → `ComponentId` | **0 bytes** — `repr(transparent)`, same word (`tags.rs:47-49`) |
| two kind filters | two loads + two compares inside functions already `#[cold]` (`tag_api.rs:46, 64, 75`) |
| one shared mint body replacing three | code **shrinks** |
| `DYN_ID_CEILING` | 8 B `.bss`; one relaxed RMW per pin at boot; one relaxed load per type's first touch |
| `new_dynamic`, `try_register_dynamic_by_name`, `dynamic_by_name`, `add_component_raw`, `remove_component_type` | no caller ⇒ dropped at link; G6(a) still asserts no exported symbol |
| G6(b)'s ~~five~~ four pinned symbols ⚠ *Rev 2.5 (P49)* | none of them is touched (`component_pool.rs:559, 600`, `check_ticks.rs:107`, `block.rs:425`, ~~`HeapRef::alloc_cold`~~) |
| G6(c)'s `size_of` pins | unchanged — every addition is a static or a free function |
| G6(d) startup work | unchanged — `OnceLock`, lazy, and `DYN_ID_CEILING` is const-initialised |

---

## P33 (closes W1 items 2 and 3) — §5.1's consumer table and K-MOD-6

*(W1 items 1 and 4 are closed by P34, which deletes their subject rather than rewriting it. All four are accounted for; none is dropped.)*

**Depends on:** P27 (the removal of `DescendantsIter::new`), P19/19.2 (`Children` is a relation), P26 constraint 1 (`drop_fn: None`), P18/18.2 (G6's rows).

### 33.1 §5.1 — two rows replaced, one added, with a blast radius I re-measured

**Removed** (§5.1, the `Children` row, `:389`, verbatim):

> | `Children(HeapVec<Entity>)`; `Children::iter()/as_slice()` unchanged; push/remove only via `EcsMaster::{set_parent, …}` | `boyko_scene`, `boyko_ui` hierarchy users; any code that did `children.0.push` |

**Removed** (§5.1, the `frame` row, `:390`, verbatim):

> | `EcsMaster` gains `frame: FrameArena`, `heap: Heap`, `tables: TableSet`; `EcsMaster::frame() -> &mut FrameArena` for exclusive systems | UI/render systems migrating frame lists |

**Added** (three rows):

> | `Children` is a **relation**, not a component value (P19/19.2); no public `Children(HeapVec<..>)` type ever exists | `boyko_scene`, `boyko_ui` hierarchy users read through the relation API; nothing migrates a `HeapVec` |
> | ⚠ *Rev 2.5 (P46.3): neither field — U-1, U-7.* `EcsMaster` gains ~~`heap: Heap`, `tables: TableSet`~~. **No `frame` field and no `frame()` accessor** (P34) | none |
> | **`DescendantsIter::new`, `AncestorsIter::new`, `EcsMaster::descendants`, `EcsMaster::ancestors` are removed and replaced by `*_in` forms taking `&mut TraversalScratch`** | `prelude.rs:1` (the type is re-exported), `relationship_api.rs:45-50, 57-62` (the two `pub &self` constructors), `observer_api.rs` (3 sites), `relation/mod.rs` (2), `query/mod.rs` (1), `relationship/mod.rs` (1), and the tests `boyko_ecs/tests/relations_query_scaling.rs` (13 uses), `relations_query_dsl.rs` (2), `boyko_scene/tests/gates_composition_structural.rs` (1) — 34 occurrences across 10 files, counted in joltab |

**Two corrections to the critique's own blast radius, in the direction that makes the row bigger.** (i) The test files do not name `DescendantsIter`; they call `EcsMaster::descendants` (`relationship_api.rs:57-62`), a `pub` `&self` method P27 did not mention. Removing only the iterator's constructor would leave a `pub` method calling a deleted function. (ii) `AncestorsIter` carries the **same** `VisitedSet` (`traverse_iter.rs:206, 213-214`) and `DescendantsIter` carries a second `Vec` besides it — `stack: Vec<(Entity, usize)>` (`:284`) — which the `ACYCLIC` const-fold does *not* remove. Both walks, both buffers, and both `EcsMaster` accessors move together or the row is a half-migration.

### 33.2 K-MOD-6, and G6(e)'s subject

**Removed** (P12, the K-MOD-6 row, `:1206`, verbatim):

> | K-MOD-6 | dynamic drop glue | the `drop_fn` a pool already stores: `pub type DropFn = unsafe fn(*mut u8)` (`component_registry/mod.rs:83`) in `pub drop_fn: Option<DropFn>` (`:117`) — the Bevy `Option<unsafe fn(OwningPtr)>` shape `[S blob_array.rs]`, already in-tree. A mod supplies an `extern "C"` thunk | none |

**Added:**

> | K-MOD-6 | dynamic drop glue | **not a seam entry — a constraint.** `drop_fn` (`component_registry/mod.rs:83, :117`) is **never written for a dynamic id**: the seam asserts `drop_fn: None` (P26 constraint 1), so no code pointer into a mod image is ever stored in `LAYOUTS` and there is nothing to dangle after unload. Mod component types are POD. The row is retained by number so the other rows' citations do not shift, and it is **excluded from G6(e)'s item list** | none — there is nothing to expose |

**Removed** (P18/18.1, the G6 (e) row, `:1900`, verbatim):

> | (e) **the seam cannot grow silently** (NEW) | G1's `syn` scanner (P6 — the parse already happens) asserts that the set of kernel items carrying `#[doc(hidden)] pub` + the `mod-seam` marker attribute equals the named list K-MOD-1..K-MOD-10, **exactly** (an extra entry is red, a missing entry is red) | add a tenth erased entry point without a ledger row → red |

**Added:**

> | (e) **the seam cannot grow silently** (NEW) | G1's `syn` scanner (P6 — the parse already happens) asserts that the set of kernel **function items** carrying the `MOD-SEAM` marker equals **this list, exactly** (an extra entry is red, a missing entry is red): `HeapRef::{alloc, free, grow}`, `Heap::new`, `ComponentPool::new`, `ComponentLayout::new_dynamic`, `component_registry::{try_register_dynamic_by_name, dynamic_by_name}`, `EcsMaster::{remove_component_type, add_component_raw}` — **ten items**. The marker is a doc line `/// MOD-SEAM: K-MOD-n` (syn parses doc attributes; an inert attribute would need a proc macro and would cost the kernel a dependency for a gate's benefit — P29's refusal applied again). The five that exist *only* for the seam additionally carry `#[doc(hidden)]`; `HeapRef::*`, `Heap::new` and `ComponentPool::new` are ordinary `pub` because the engine itself calls them, and pinning them by doc-visibility would have made the rule unstatable. K-MOD-2/3/7/8/9 are **not** function items and are pinned by (a), (c) and O6's allowlist instead | add an eleventh erased entry point without a ledger row → red |

---

## P34 (closes W1 items 1 and 4, and pass-4 open question 1) — the Frame class is DELETED; its exit criterion has fired

**Depends on:** P27's own exit criterion ("if rung 2 ends with that list empty, the class is **deleted**, not shipped unused", `:2517`), P31/3's closed `hv3-owner` vocabulary, the ledger rev 4.

### 34.1 The evidence, and why it is the inventory P27 was waiting for

P27 made the class conditional on a rung-2 inventory of exclusive-system scratch. **That inventory exists on disk and it is not empty of rows — it is empty of frame-arena rows**, and it says so in three independent places:

1. `RUNTIME-DATA-LEDGER.md:126` — lifetime class 7, *system-scratch*: "Per-system transient data rebuilt on every run, held on a kernel ScratchColumn owned by the system (its Local, or KF-44 for an exclusive system). Rev 2 decides this backing; **the frame arena of the allocator design space is not built.**"
2. `RUNTIME-DATA-LEDGER.md:820, :2226` — **KF-44, "Per-system state for exclusive systems", `Local<T>`, 21 rows** (UI 17, scene 3/4). That is exactly the client set P27 reserved for the Frame class ("dispatcher-side scratch inside an exclusive system"), and the census has already routed every row of it to a `ScratchColumn`.
3. Every ledger row that mentions `FrameVec` mentions it as a **conflict with this design, resolved against it**: `ledger/ui-lane.md:139` ("PLAN CONFLICTS: … sends it to FrameVec on a FrameArena" → destination `ScratchColumn`), `:140` ("PLAN CONFLICT: the plan_ref chooses FrameVec" → `ScratchColumn`), `:143`, and `ledger/ecs-schedule.md:101-102`, which independently re-derives pass-3 W1: "FrameArena is dispatcher-thread-only by the allocator design's own Frame row; DescendantsIter takes `&EcsMaster`. That all holders are on the dispatcher thread is not verified; if any is not, FrameVec is unsound there."

Add P27's own two re-pointings (UI lanes → `ScratchColumn`, worker scratch → `Local<TraversalScratch>`) and the class has **no client in a census of every runtime heap site in the engine**. A bump allocator with no client is 256 MiB of VA, a primitive, a guard type, a reclamation rule, a `Sync` argument and a maintenance surface bought for nothing.

**Ruling: the Frame class is deleted.** With it go `FrameArena`, `FrameVec`, `FrameBox`, `FrameSlice`, `FrameMark`, `FrameGuard`, FR-1..FR-4, the 256 MiB reservation, `EcsMaster::frame`/`frame_reset`, the SEND11 bullet, P24's gate, and M-A11/M-A12's arena half. **W1 item 1 (the stale SEND11 rationale over a rule the design deleted) and item 4 (15.4's per-slot row) are closed by deletion, which is the file's own preferred closure and the one the critic's positive list rewards.**

**Destinations, stated so nothing is left homeless** — the mistake W2 found for `HeapString` and which this patch must not repeat:

| former Frame client | destination | why it is not worse |
|---|---|---|
| rung 2's CSR two-pass (hierarchy rebuild) | two `ScratchColumn`s owned by the rebuild system (KF-44 `Local`) | warm after frame 1 instead of re-bumped every frame; the borrow problem `FrameGuard` existed to solve does not arise, because the system takes `&EcsMaster` and `&mut Local<..>` as separate params |
| UI frame lanes (`resources.rs`, `pick.rs`, `focus.rs`, `bind_system.rs`, render `ui/pack.rs`) | `ScratchColumn` — already P27 point 5, already the ledger's decision (`ledger/ui-lane.md:139-143`) | the two documents stop diverging |
| `DescendantsIter`/`AncestorsIter` scratch | `Local<TraversalScratch>` (P27 point 4, extended by P33/33.1 to `AncestorsIter` and the two `EcsMaster` accessors) | no TLS read, no slot, no `DETACHED` panic |
| command staging, prefab materialisation | `ByteColumn` (the ledger's KF-erased-record-column, shared with `CommandQueue`) | one record column instead of a second arena |
| schedule-build buffers | rung 3's `ScheduleTables` / `SortedMap` | unchanged |

**What is lost, named rather than glossed:** P24's FR-1 ("the only range a `Drop` releases is one the compiler has just proved unowned"), which the critique's positive list asked to preserve. It is preserved **as the revival rule**, not as shipped code.

**Revival gate:** the Frame class returns if and only if a client appears that is (i) dispatcher-only, (ii) needs many differently-typed transient buffers within one frame whose element types are chosen at runtime, so a per-type `ScratchColumn` cannot be pre-registered, and (iii) whose peak is unknown at setup. If it returns, it returns with FR-1 as its reclamation rule and with P24's rejected closure-scope alternative still rejected, for the hole P24 named.

### 34.2 The sites, each quoted

| # | Site | **Removed**, verbatim | **Added** |
|---|---|---|---|
| 1 | §2.1, the Frame row (`:131`) | `\| **Frame** \| `FrameArena` + `FrameVec<T>`, `FrameSlice<T>` \| one frame between two `reset()` calls at a fixed schedule position \| **dispatcher thread only** \| none; `mark()/rewind(mark)` LIFO \| **no** \| `reset()` at frame start \| within a frame \| 0 until used → high-water mark, page-rounded \|` | *(row deleted; the table has four classes — Column, Scope, Heap, Table — and the sentence under it that reads "Five rows because…" becomes "Four rows because…")* |
| 2 | **P4 in its entirety** (`:929-:994`, heading "## P4 (closes W2, and the `Sync` question it exposes) — §2.3/§2.4 Frame class: `&self` + `Cell`"), including its Added `FrameArena`/`FrameVec` block, its "API change" paragraph and its "Non-exclusive systems do not get the frame arena" paragraph | the whole section | *(deleted — its subject no longer exists. The `!Sync`/SEND11 argument goes with it; `EcsMaster` gains no `Cell`-bearing field and its `unsafe impl Sync` (`ecs_master.rs:1287-1288`) is untouched, so **no new SEND bullet is owed**)* |
| 3 | **P24 in its entirety** (`:2273-:2361`, heading "## P24 (closes C1) — `rewind`/`FrameMark` are deleted; reclamation is a property of the handle's `Drop`, not of a token"), including 24.1's FR-1..FR-4 and 24.2's gate | the whole section | *(deleted with its class; FR-1 survives as P34's revival rule and nowhere else)* |
| 4 | **P27's arena half** (`:2467-:2535`): 27.1's ruling block (the `FrameGuard` code block and points 1-5), the client-list/exit-criterion paragraph, the cost paragraph, the M-A8 overturn gate, and **M-A11** | those blocks | *(deleted. **P27's `TraversalScratch` ruling, its `Local` routing, its `DescendantsIter::new` removal, its 122 KiB numeric correction and M-A12 are KEPT** — they are the part that survives the class and they are now the whole of P27; P33/33.1 extends them to `AncestorsIter` and the two `EcsMaster` accessors)* |
| 5 | §2.3, the Frame-class block | already quoted and replaced by P4; deleted with P4 | — |
| 6 | §2.4, the Frame API (`:267-277`) | ``// Frame`` / ``impl FrameArena { pub fn new(label: &'static str) -> Self; pub fn alloc(&mut self, layout: Layout) -> NonNull<u8>; pub fn alloc_slice_uninit<T>(&mut self, n: usize) -> NonNull<T>; pub fn vec<T: Copy>(&mut self, cap: u32) -> FrameVec<'_, T>; pub fn mark(&self) -> FrameMark; pub fn rewind(&mut self, m: FrameMark); pub fn reset(&mut self); pub fn high_water(&self) -> usize; }`` | *(block deleted)* |
| 7 | §2.5, the algorithm row (`:312`) | `\| `FrameArena::alloc` \| `p = align_up(cur, a); n = p+size; if n > end {cold commit}; cur = n` \| O(1) \| one line (`cur`,`end`), sequential writes into the arena \| 1 predictable \| identical to `ScopeBlock::bump` (`[merge] block.rs:369-398`) \|` | *(row deleted)* |
| 8 ⚠ *Rev 2.5 (P46.3): `TermList` → KC-17 / KC-10, not `HeapDyn`/`HeapVec`* | §4 rung 1e (`:348`) | `\| 1e \| `TermList` `Box::new` per (tag-terms, generation) epoch (`term_list.rs:141-170`); `traverse_iter.rs:51, 284` scratch (flagged per-call by Lens C, unverified) \| conditional \| `HeapDyn`/`HeapVec` (epoch-rate) / `FrameVec` (per call) \| conditional \|` | `\| 1e \| `TermList` `Box::new` per (tag-terms, generation) epoch (`term_list.rs:141-170`); `traverse_iter.rs:51, 284` scratch \| conditional \| `HeapDyn`/`HeapVec` (epoch-rate, `registry`-owned per P31/3) / **`Local<TraversalScratch>` of two `ScratchColumn`s** (per call) \| conditional \|` |
| 9 | §4 rung 2, the UI-lanes row (`:362`) | `\| UI frame lanes (`boyko_ui/resources.rs:216-254`, `pick.rs`, `focus.rs`, `bind_system.rs`), render `ui/pack.rs:143-150` \| `FrameVec` on the `FrameArena` (clear+refill per frame is exactly the frame class); `Vec<Vec<Entity>>` pools → CSR `FrameVec<u32>` offsets + flat `FrameVec<Entity>` two-pass \| dispatcher-only systems (UI runs exclusive) — verify per system in the rung \|` | `\| UI frame lanes (`boyko_ui/resources.rs:216-254`, `pick.rs`, `focus.rs`, `bind_system.rs`), render `ui/pack.rs:143-150` \| **`ScratchColumn` owned by the system** (`Local`, or KF-44 for an exclusive system) — the destination ledger rev 4 already records for these exact rows (`ledger/ui-lane.md:139-143`); `Vec<Vec<Entity>>` pools → CSR `ScratchColumn<u32>` offsets + flat `ScratchColumn<Entity>` two-pass \| no verification owed: a `ScratchColumn` is per-system state, so the dispatcher-only question does not arise \|` |
| 10 ⚠ *Rev 2.6 (P54/O1, AP7 O1): this site edited P11's bullet (`:1182`), which P17 had already removed (17.5, `:1873`). The live loom text is P17's Added block (`:1877`, with P51's fix), and this row's "`Heap` and `ChunkCache` have no atomics" is the sentence P17 called the defect* | §5.3, the loom bullet as replaced by P17 (`:1866`) | `- **loom** — `Heap`, `FrameArena` and `ChunkCache` have no atomics (single writer / single slot owner); …` | `- **loom** — `Heap` and `ChunkCache` have no atomics (single writer / single slot owner); …` *(remainder unchanged)* |
| 11 | P3's sanitize line (`:925`) | `> `FrameArena::rewind`/`reset` fill the released range with `0xDD` under the same gate.` | *(line deleted — `rewind` was deleted by P24 and the arena by this patch; a third instance of the W1 class, not named in pass 4)* |
| 12 | 15.4's Miri table row (`:1607`) | `\| FrameArena, per slot \| 64 MiB \| **0** (stays LAZY — P19) \| it has no reservation-resident header, so laziness costs it no branch on the handle path \|` | *(row deleted. **Per-`EcsMaster` eager Miri cost stays 1.26 MiB** — the Frame arena contributed 0 to it, so no derived number moves)* |
| 13 ⚠ *Rev 2.5 (P46): the Heap rows are the revival form's* | P30/O1's table, the `EcsMaster` row (`:2645`) | `\| `EcsMaster` \| Heap small, Heap large, FrameArena (**ONE**, P27) \| 1 GiB + 1 GiB + 256 MiB = **2.25 GiB** \| 1 MiB + 256 KiB + 4 MiB \| **4 KiB** — the heap's header page, committed by `Heap::new` (15.2); Frame is lazy, so 0 \|` | `\| `EcsMaster` \| Heap small, Heap large \| 1 GiB + 1 GiB = **2 GiB** \| 1 MiB + 256 KiB \| **4 KiB** — the heap's header page, committed by `Heap::new` (15.2) \|` |
| 14 ⚠ *Rev 2.5 (P51, pass-6 O1): read `SlotChunks` for `ChunkCache` (P16's rename); the Heap also leaves the list (U-1, P47)* | K-MOD-7's not-exposed list (`:1207`) | `…`boyko_memory::raw::{reserve, commit, base}`, `FrameArena`, `ChunkArena`/`ChunkCache`, `ScopeBlock`.…` | `…`boyko_memory::raw::{reserve, commit, base}`, `ChunkArena`/`ChunkCache`, `ScopeBlock`.…` |
| 15 ⚠ *Rev 2.5 (P46.3): no `DropColumn`, no Heap class, no `TableSet` — step 3 is `ZeroInit` + the relaxed bound and `ByteColumn`; C1's Miri test is not written (03 §3)* | P14's implementation-plan step 3 (`:1257`) | `> 3. `boyko_memory` primitives: `ZeroInit` + the relaxed `VmColumn` bound (P5), `DropColumn`, `ByteColumn`, `FrameArena`/`FrameVec` (`&self`, P4), `Heap`/`HeapHeader`/`HeapRef`/`HeapVec`/`HeapBox`/`HeapDyn`/`HeapString`/`SortedMap` (P1), `TableSet`. Each with unit tests, the sanitize arm, and **both** Miri legs (P11). **C1's red-first test lands in this step, before any client**, and the O3/P1 laziness contradiction flagged in P8 is resolved before the `Heap` is written.` | `> 3. `boyko_memory` primitives: `ZeroInit` + the relaxed `VmColumn` bound (P5), `DropColumn`, `ByteColumn`, `Heap`/`HeapHeader`/`HeapRef`/`HeapVec`/`HeapBox`/`HeapDyn`/`SortedMap` (P1), `TableSet`. **No Frame class (P34) and no `HeapString` (P35).** Each with unit tests, the sanitize arm, and **both** Miri legs (P11). **C1's red-first Miri test lands in this step, before any client**, and the O3/P1 laziness contradiction flagged in P8 is resolved before the `Heap` is written.` *(this is the single authoritative edit of the line; P35 does not re-edit it)* |
| 16 ⚠ *Rev 2.5 (P46.2): the `DropColumn`, `Heap` and `TableSet` tests are the revival forms'* | Metrics, the unit-test bullet (`:438`) | `- **Mandatory unit tests**: each primitive's growth across a commit boundary; `DropColumn` destructor count on truncate/swap_remove/drop; `Heap` class boundaries (16, 256, 257, 512, 64 KiB, 64 KiB+1); free-list LIFO order; `FrameArena` mark/rewind LIFO `debug_assert`; `TableSet` alignment of every table; sanitize double-free detection (red-first).` | `- **Mandatory unit tests**: each primitive's growth across a commit boundary; `DropColumn` destructor count on truncate/swap_remove/drop; `Heap` class boundaries (16, 256, 257, 512, 64 KiB, 64 KiB+1); free-list LIFO order; `TableSet` alignment of every table; sanitize double-free detection (red-first).` *(the `mark/rewind` clause was already dead after P24 — the second unnamed W1-class instance)* |
| 17 ⚠ *Rev 2.5 (P46.2): the `Heap` and `TableSet` asserts are the revival forms'* | Metrics, the `debug_assert!` bullet (`:440`) | `- **`debug_assert!` invariants**: owner thread on `Heap`/`FrameArena` mutation; `len <= committed`; LIFO mark order; class ↔ layout on `free`; `TableSet` bounds; HV-1 at `Heap::drop` (sanitize).` | `- **`debug_assert!` invariants**: owner thread on `Heap` mutation; `len <= committed`; class ↔ layout on `free`; `TableSet` bounds; HV-1 at `Heap::drop` (sanitize).` |
| 18 | Open questions, item 1 (`:444`) | `1. `FrameVec<T: Copy>` in v1 — the inventory found no droppable per-frame data; if the critic finds one, the answer is `DropColumn` on the Heap, not a finalizer chain.` | *(item deleted — the inventory found no per-frame data of any kind that wants an arena)* |

### 34.3 What this buys

256 MiB of VA per world; one primitive, one handle type, one guard type and one reclamation rule out of `boyko_memory`; the `size_of::<EcsMaster>()` re-bless P27 budgeted for G6(c) is **not needed**; SEND11 is never written, so the `unsafe impl Sync` at `ecs_master.rs:1287-1288` keeps exactly the ten bullets it has; and the design and the ledger agree on every one of the 21 KF-44 rows instead of disagreeing on all of them.

---

## P35 (closes W2) — `HeapString` is retired, and rung 4's heap destinations are re-pointed to columns

**Depends on:** P31/3 (the closed `hv3-owner` vocabulary), P1/HV-3 (single-writer), §1's retirement criterion ("a client-less primitive does not ship"), P34 (the same criterion, applied one section earlier). ⚠ *Rev 2.5 (P46): rung 4's column destinations stand; `HeapString`'s revival gate now sits inside the Heap's revival form (U-1).*

**The critic's argument is correct and it reaches further than `HeapString`.** Under P31/3 an `hv3-owner` must be `schedule`, `schedule-builder`, `registry` or `master-table`. Rung 4's owners are a log ring written from worker threads, an asset loader, a UI text pipeline, an app title and a serializer — **none** of them is one of the four, so rung 4 cannot land as written for `HeapVec` either, not only for `HeapString`. Adding a fifth value would dissolve the vocabulary that makes HV-3 provable, so it is refused.

**Removed** (§4 rung 4, `:379-380`, verbatim):

> ### Rung 4 — load, clone, serialize, diagnostics, strings
> `prefab.rs`, `serialize/mod.rs:288`, `load_writer.rs`, `mesh_data.rs`, `texture_data.rs`, UI text AST/lower/report, `boyko_app` titles/profiling, `boyko_log` `String`s, asset `error.rs` → `HeapVec`, `HeapString` with `core::fmt::Write`, `InlineStr<N>` for titles, error types carry codes (`boyko_log/src/codes.rs` already exists) + `&'static str`. Tag `diag` sites are the last to go; G4 lands per crate as each drops under 10.

**Added:**

> ### Rung 4 — load, clone, serialize, diagnostics, strings
> **No rung-4 row takes a heap handle**, because no rung-4 owner is one of P31/3's four `hv3-owner` values. Destinations:
>
> | rows | destination | why |
> |---|---|---|
> | `boyko_log` `String`s | **per-lane `ByteColumn`** (the log ring's own column) + `boyko_log/src/codes.rs` codes; formatting appends through a `core::fmt::Write` adapter over the column | the ring is written from worker threads, which HV-3 forbids for a heap; a per-lane column is single-writer by partition, not by discipline |
> | asset `error.rs` | error types carry a code + `&'static str`; **no string storage at all** | an error that allocates on the error path is the allocation this campaign exists to remove |
> | UI text AST / lower / report | `ScratchColumn` / `ByteColumn` owned by the UI system (KF-44 `Local`) | the ledger's own destination for every UI text row |
> | `boyko_app` titles | `InlineStr<N>` (N pinned at the OS title limit) | fixed, small, no allocation |
> | profiling labels | `&'static str` | they are `type_name` literals already |
> | `prefab.rs`, `serialize/mod.rs:288`, `load_writer.rs`, `mesh_data.rs`, `texture_data.rs` | `ByteColumn` / `VmColumn<T>` | these are byte buffers, not strings; a column is the Column class and needs no owner class |
>
> **`HeapString` does not ship.** Its client list is empty after this table, which is §1's own retirement criterion ("No `Allocator` impl ships in v1 — it would be client-less, the exact X.J retirement criterion"), applied here as P34 applied it to the Frame class. **Revival gate:** a string that (i) is owned by `schedule`, `schedule-builder`, `registry` or `master-table`, (ii) is individually freed, and (iii) is not a `&'static str` — `ScheduleBuilder`'s names fail (iii) by §5.1's own row, which is why none exists today.
>
> Tag `diag` sites are the last to go; G4 lands per crate as each drops under 10.

**Removed** (§2.1, the Heap row's primitive cell, `:133`, verbatim — cell only, the row's other cells are unchanged):

> `Heap` + `HeapVec<T>`, `HeapBox<T>`, `HeapDyn<V>`, `HeapString`, `SortedMap<K,V>`

**Added:**

> `Heap` + `HeapVec<T>`, `HeapBox<T>`, `HeapDyn<V>`, `SortedMap<K,V>`

**Removed** (§2.3, `:236`, verbatim):

> ```rust
> pub struct HeapString(HeapVec<u8>);   // utf8 invariant; impl core::fmt::Write; Deref<str>
> ```

**Added:** *(nothing — the primitive is not defined)*

**The gate must not name a type that does not exist.** 15.5's HV-1 scanner and 19.2's HV-4 scanner each hard-code the heap-client set, and one of them names `HeapString`.

**Removed** (15.5, point 1, the parenthetical, `:1617`, verbatim):

> (`HeapVec`/`HeapBox`/`HeapDyn`/`HeapString`/`SortedMap`, plus any struct transitively containing one — resolvable because the scanner already indexes every struct in the workspace)

**Added:**

> (**the heap-client set is derived, not listed: it is exactly the primitives named in §2.1's Heap row, plus any struct transitively containing one** — resolvable because the scanner already indexes every struct in the workspace. Deriving it is the difference between a gate that follows a retirement and a gate that asserts over a deleted type, which is a check that cannot fail)

The same substitution applies to 19.2's HV-4 comment (`:1966-1968`, "`HeapVec`, `HeapBox`, `HeapDyn`, `HeapString`, `SortedMap`, or any struct transitively containing one"): it becomes "any heap handle — the §2.1 Heap-row set — or any struct transitively containing one".

---

## P36 (closes optional O1, O2, O3)

### O1 — `carve`'s alignment becomes a contract line, and a `debug_assert`

Accepted. `ScopeBlock`'s padding argument rests on "`bases[i]` is `CHUNK_ALIGN`-aligned" (`block.rs:410-411`) while `grow` requests `Layout::from_size_align(cap, CHUNK_ALIGN)` (`:109, :474`) — always 64, never the emplaced type's alignment. P16/P17 never stated what `carve` returns.

**Added** (P16/16.3, `ChunkArena::carve`'s contract):

> **Alignment contract (O1):** `carve` returns a base aligned to **4096**, hence to `CHUNK_ALIGN` (64) and to every alignment `ScopeBlock::emplace` can demand. Derivation: the frontier starts at 0, every `cap` is a power of two ≥ 4096 (`block.rs:105`, `CHUNK0 = 4096`, doubling ladder), so every `fetch_add` leaves the frontier 4096-aligned. `debug_assert_eq!(base as usize % 4096, 0)` at the return. This is stated because the premise **changes** when `std::alloc` — which honours the `Layout` it is given — is replaced by a bump that honours only its own arithmetic; that substitution is precisely what rung 1a does.

### O2 — the `.text` number is read from the rlib's object members, and the churn case cannot arise

The critique's premise is right (COFF symbols carry no `st_size`) and its churn case is not: a function inserted between two symbols **is itself a symbol**, so the delta is taken to *it*, not across it. What the delta actually costs is quantisation to the 16-byte function alignment, which is stable run to run.

**Added** (P18/18.2, after the tool choice):

> **How the per-symbol number is obtained, per object format.** The measurement runs on the **rlib's object members**, not on the linked binary, for two reasons: local and `pub(crate)` symbols survive there (two of the ~~five~~ four pinned symbols ⚠ *Rev 2.5 (P49)* carry local linkage — `commit_subregion` is a private `fn` at `component_pool.rs:560`, `grow_rows` is `pub(crate)` at `:601`), and the quantity P18 wants is the kernel's codegen, not the linker's placement.
> - **ELF leg**: `llvm-nm --print-size` — `st_size` is exact.
> - **PE/COFF leg**: `llvm-objdump --disassemble-symbols=<mangled>`, whose listing ends at the next symbol in the section; the number is therefore the function's size **rounded up to the 16-byte function alignment**. It is not perturbed by an unrelated function being placed after it, because that function contributes its own symbol and becomes the new boundary.
> - The two legs measure slightly different quantities (exact vs 16-B-quantised), so **the pins are per leg**, alongside the `rustc -Vv` stamp P31/4 already requires. A leg whose pins were blessed on the other leg's numbers is a re-bless error, and the header's stamp is what makes it visible.

### O3 — the commit wrapper is owner-parameterised, and the parameter is a marker type, not an enum const

Accepted, with one self-correction to P31/1's shape: `const OWNER: CommitOwner` requires `adt_const_params`, which is unstable; a feature gate in `boyko_memory` for a gate's benefit is the class P29 refused.

**Added** (P31/1, after the ruling sentence):

> **The parameter is a marker type with an associated index, not an enum const param:** `raw::commit_at<O: CommitOwner>(..)` where `trait CommitOwner { const INDEX: usize; }` and `Column`/`Heap`/`Chunk`/`Frame`/`Table`(minus `Frame`, deleted by P34 — **four** owners) are unit structs. Monomorphisation folds `COMMITTED_BYTES[O::INDEX]` to a constant address: same one relaxed RMW, no branch, no extra load, and no unstable feature. **`VmReservation::commit` carries the parameter through** — `commit<O: CommitOwner>(&mut self, ..)` — so `VmColumn`, `ComponentPool` and `TableSet` each name their own owner and the `Table` arm is not pinned at 0 vacuously by a wrapper that hard-codes one value. G2b's anti-vacuity direction (nonzero per used owner class at setup) would have caught this at rung time; naming it here saves the discovery. ⚠ *Rev 2.5 (P46.3): three owners — `Heap` leaves with U-1; `TableSet` is a revival form, so the `Table` owner is KC-18's `VmColumn<T, TableOwner>`.*

---

## P37 — pass-4 open questions 2, 3, 4

*(Question 1 is answered by P34: the class is deleted, on the ledger's inventory rather than on a benchmark.)*

**2. Hot-reload capped at "same name, same layout".** Settled, and now sited: the `{size, align}` compare lives in `intern_or_mint` (P32/32.3) and surfaces as `DynMintError::LayoutMismatch`. It is a product limit and belongs in `boyko_modding`'s documentation, where the person who hits it is standing. No design change.

**3. Can `ScratchColumn` hold `(Entity, u32)` and `u64`? Yes — but the rule that applies is not the one the question assumes, and it has a price.** `ScratchColumn::new(component_id, reserve_rows)` requires a **registered `ComponentId` whose layout matches `T`** (`component/scratch/scratch_column.rs:50-104`; it asserts `!needs_drop::<T>()`, then `debug_assert`s `layout.size() == size_of::<T>()` and `layout.align() >= align_of::<T>()`). `VmColumn`'s divisibility pin, relaxed by §2.2/P5, is not the constraint; **two ids out of the shared 512 are**. So:

> **Added** (rung 1e, as a note on the `TraversalScratch` row): `TraversalScratch { visited: ScratchColumn<u64>, stack: ScratchColumn<(Entity, u32)> }` consumes **two `ComponentId`s**, process-wide and once, registered through `register_layout` (the doc-sanctioned synthetic use, `component_registry/mod.rs:1012-1024`) from two file-local `OnceLock`s inside the non-generic `TraversalScratch::new` (non-generic, so rust#22991's monomorphisation-collapse trap does not apply). Each registration lowers `DYN_ID_CEILING` (P32/32.4), so the budget accounting is automatic rather than a comment. `(Entity, u32)` also narrows the shipped `(Entity, usize)` frontier entry (`traverse_iter.rs:284`) — depth is capped at `MAX_PROPAGATION_DEPTH` (1024), so `u32` is sound and the entry shrinks 16 B → 12 B. ⚠ *Rev 2.5 (P50, U-2): superseded — `TraversalScratch` builds both columns with `ScratchColumn::for_type` and consumes no `ComponentId` (KC-10); `DYN_ID_CEILING` was deleted by P39.*

**4. `ensure_len_zeroed` after a shrink.** Accepted as a contract line with a debug mechanism, because the critic is right that it is true until the first client that truncates:

> **Added** (§2.2, third bullet): `ensure_len_zeroed(n)` relies on the **fresh-commit** zero-fill contract (`vm.rs:19-37`), so a column that truncated and regrew within its already-committed range reads its own stale bytes, where the `slot+1` / zero encoding expects "absent". No named client shrinks today (`EntitySlotMap`, `LiveBitmap`, `EnableStore.pages` only grow), so this is a contract, not a defect. It is held by a mechanism rather than by the sentence: ~~`VmColumn<T: Copy>`~~ `VmColumn<T: ZeroInit>` ⚠ *Rev 2.5 (P51, pass-6 O1): P5 made `ZeroInit` the struct bound* carries `#[cfg(debug_assertions)] ever_truncated: bool`, set by `truncate`/`set_len` on any shrink, and `ensure_len_zeroed` `debug_assert!`s it is clear. Release cost: zero (the field is `cfg`'d out). A client that legitimately needs both writes the zeros itself and says so at the call site.

---

## Change log (rev 2.2 → rev 2.3)

| Row | Disposition | Where |
|---|---|---|
| **pass-4 C1** | **CLOSED by narrowing one field type and deriving the kind from the layout.** `TAG_NAMES` → `DYN_NAMES`, value `TagId` → `ComponentId` (same word, `repr(transparent)`); `tag_by_name`/`try_register_tag_by_name` become kind-exact against `LAYOUTS`/`STORAGE_KIND`; flat namespace, justified by the name being the serialization key. `TagId`'s size-0 proof is preserved and its *mechanism* strengthened — the oracle is the one the kernel already trusts at `migration_helpers.rs:1573-1581`. `try_register_dynamic` is **not** the seam entry (it mints unconditionally — that was the defect); the entry is a new name-keyed `intern_or_mint` pair. Cost line corrected from "one constructor and one re-export" to six named items. Exit B additionally shown to be a **live shipped defect** reachable from three `pub` methods via the enable-tag path, closed by the same kind filter. Seven gate rows with red-first mutations. | P32 |
| **W4's "all 512 ids"** | **CORRECTED with new evidence**: `boyko_physics` already owns the top 128 (`scratch_ids.rs:685`), so production has 384, and P21's `[384, 512)` mod range was byte-for-byte that band — the split was unimplementable, not merely expensive. The ceiling becomes `DYN_ID_CEILING`, an atomic floor lowered by `register_layout`, which fixes a shipped hazard (the counter climbing into a pinned band panics with a misleading occupancy message) at the cost of one `.bss` word and one relaxed RMW per pin. M-A13 censuses the real headroom. | P32/32.4 |
| **K-MOD-11** | **NEW seam entry named and priced**: `add_component_raw`, because `add_tag`'s migration debug-asserts size-0 and `migrate_entity_insert` is generic. The refactor that would deduplicate it is **refused with its cost** (a runtime-length memcpy on every engine `insert`). | P32/32.5 |
| **W1 (1), (4)** | **CLOSED by deletion** — the Frame class is deleted, so there is no `Cell`-bearing field, no SEND11, and no per-slot Miri row. | P34 |
| **W1 (2)** | **CLOSED** — §5.1's two stale rows replaced and a third added; blast radius re-measured and **larger** than the critique stated: `AncestorsIter` and the two `pub` `EcsMaster` accessors move too (34 occurrences, 10 files). | P33/33.1 |
| **W1 (3)** | **CLOSED** — K-MOD-6 restated as a constraint and excluded from the inventory; G6(e)'s subject restated as an explicit **ten-item** function list with the marker's form specified (a doc line, not a proc-macro attribute). | P33/33.2 |
| **Two stale passages pass 4 did not name** | CLOSED — K-MOD-5's Shape cell (`:2029`, still describing `register_at`/`MOD_ID_BASE`) and the "Mandatory unit tests" bullet (`:438`, still requiring a `FrameArena` mark/rewind assert), plus P3's `rewind` sanitize line (`:925`). | P32, P34 |
| **W2** | **CLOSED by retirement** — `HeapString` does not ship; the finding is extended to rung 4's `HeapVec` rows, which have no legal `hv3-owner` either; every string row re-pointed to `ByteColumn`/`ScratchColumn`/`InlineStr`/`&'static str`; the two gates' heap-client set becomes **derived from §2.1** so it cannot name a retired type. | P35 |
| **pass-4 question 1** | **ANSWERED and acted on** — the inventory exists (ledger rev 4 `:126`, `:820`, `:2226`, `ledger/ui-lane.md:139-143`, `ledger/ecs-schedule.md:101-102`): zero frame-arena rows in a census of every runtime heap site. The class, its guard, its handles and FR-1 leave together; 256 MiB of VA and the G6(c) re-bless leave with them. | P34 |
| **O1, O2, O3** | CLOSED — `carve`'s 4096 alignment contract + `debug_assert`; the `.text` number read from rlib object members with the per-format acquisition and the per-leg pins (the critique's churn case shown impossible, the quantisation named); `commit` owner-parameterised by a **marker type**, correcting P31/1's unstable `adt_const_params` shape. | P36 |
| **pass-4 questions 2, 3, 4** | ANSWERED — reload cap sited in `intern_or_mint`; `ScratchColumn`'s real constraint is **two ids**, not divisibility, with the ceiling interaction; `ensure_len_zeroed` gets a debug-only `ever_truncated` tripwire. | P37 |
| rev-2.2 P25, P26 (except the five amended passages), P28, P29, P30, P31 (except /1's parameter shape) | **unchanged** | — |
| rev-2.1 P0, P3 (except its last line), P6, P7, P9, P11, P15, P16, P17, P18, P20, P21 (except K-MOD-5's cell), P22, P23 | **unchanged** | — |

## Open questions for pass 5

1. **`DYN_ID_CEILING` makes the id budget a *runtime* quantity, and M-A13 has not run.** The mechanism is fail-closed and the gate is red-first, but the number that decides whether `MAX_MOD_TYPES = 128` is even reachable (`384 − engine types − dynamic tags − pinned scratch`) does not exist yet. I judge shipping the mechanism before the number correct — it converts a misleading panic into a named error either way — but the *cap* stays provisional until M-A13.
2. **P34 deletes a class on a census produced by another workflow.** The three ledger rows are quoted and the tree and revision are pinned (`rev 4`, `d552be05`), but the ledger is being revised concurrently. If rev 5 introduces a row whose destination is a frame arena, the revival gate in 34.1 is the thing to check it against — and it should be checked before rung 1 starts, not after.
3. **`tag_by_name` becoming kind-exact is a behaviour change on a shipped `pub` method.** The sweep is specified as a rung obligation with a recorded count. I do not expect a legitimate dependent (any such caller was depending on the 32.2 defect), but that is a prediction, and the count in the test header is what turns it into a fact.

**Files read for this patch** (all read-only): `D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md`; `D:/claude/BoykoEngine/docs/memory/RUNTIME-DATA-LEDGER.md`; `D:/claude/BoykoEngine/docs/memory/ledger/ui-lane.md`; `D:/claude/BoykoEngine/docs/memory/ledger/ecs-schedule.md`; and in `D:/wt/joltab`: `crates/boyko_ecs/src/ecs/core/component/component_registry/{mod.rs, tags.rs}`, `crates/boyko_ecs/src/ecs/core/ecs_master/{tag_api.rs, enable_tag_api.rs, component_api.rs, relationship_api.rs, ecs_master.rs}`, `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs`, `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs`, `crates/boyko_ecs/src/ecs/core/iters/query/relation/traverse_iter.rs`, `crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs`, `crates/boyko_ecs/src/ecs/core/clone/materialize.rs`, `crates/boyko_physics/src/scratch_ids.rs`, plus workspace greps for `DescendantsIter`/`descendants(`, `register_layout::<`, `set_component_raw`, `is_signature_id`.

---

# Part VII - Critique log - pass 5 (2026-09-17)

**Verdict of pass 5: CHANGES REQUESTED** - two blockers (C1-C2), four important remarks (W1-W4), two optional notes (O1-O2), six preserved positives, four open questions for the architect. Both blockers are in text **Rev 2.3 introduced** (P32/32.4's `DYN_ID_CEILING`, P32/32.5's K-MOD-11) and neither re-litigates an earlier round: C1 is a sibling lane (reflect, KF-47) already owning the by-id rung §7 was about to re-implement; C2 is a per-slot pin read as a global floor. W1-W4 are the kind-classification window R2 opened, a retired type still named in the first gate P14 builds, four more superseded passages, and `ComponentId`s spent by new destinations without being counted.

Rules of this log, as in Parts II, IV, V and VI:

- Rev 2.3 above is **unchanged**. No finding is answered by silently editing Rev 2.3; every answer lands in Rev 2.4 below and names the Rev 2.3 text it removes.
- The log is reproduced **verbatim as the critic wrote it**, including its own headings, its confidence tags, its "preserve these" list and its open questions. One exception: a stray non-English word fragment in W4 is rendered as its English meaning ("counts"), because the repository forbids mixed-language files outside `docs/ru/`. The dispositions are in Rev 2.4's change log.
- Trees as the critic states them: code read in `D:/wt/joltab` (`merge/ke16-into-ecsnative`, read-only) unless a citation names `D:/wt/reflect`; documents in `D:/claude/BoykoEngine`. Ledger on disk read as **rev 4**.

---

# Architecture review: allocator design space — Rev 2.3 (pass 5)

## Verdict

```
[ ] APPROVED
[X] CHANGES REQUESTED — 2 blockers, 4 important, 2 optional
```

Rev 2.3 closes pass-4 cleanly on the axes pass 4 named, and two of its closures are better than what they replace. I traced each rather than taking the change log:

- **P32's 32.2 defect is real and I verified the whole chain.** `try_register_enable_tag_by_name` mints through `try_register_tag_by_name` and *then* reclassifies (`D:/wt/joltab/crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs:134-141`), inserting the enable tag's name into the very map `tag_by_name` reads (`:182-197`, `:201-207`) — so `tag_by_name` really does hand back a `TagId` for a `Bitset` id today, with no kind test (`.../ecs_master/tag_api.rs:76-78`). The layout-as-oracle rule R2 adopts is the rule the kernel already applies at the one site where being wrong corrupts a row (`.../commands/migration_helpers.rs:1573-1581`, quoted exactly as the patch states). Finding a live shipped defect while closing a modding blocker, and paying for the filter out of that defect rather than out of the modding budget, is the right argument.
- **P34's evidence is on disk and says what the patch says it says.** `RUNTIME-DATA-LEDGER.md:126` reads "the frame arena of the allocator design space is not built"; `:820` reads `KF-44 | Per-system state for exclusive systems | … | 21 | physics-scene-math 4, ui-lane 17`. Deleting a primitive on an inventory rather than shipping it unused is this file's own retirement criterion applied to itself for the second time.
- **P37/3 is correct and its answer is the right one**: `ScratchColumn::new` needs a registered `ComponentId` whose layout matches `T`, refuses ZSTs and refuses `needs_drop` (`.../component/scratch/scratch_column.rs:50-104`) — so "two ids, not divisibility" is exactly the constraint. `MAX_PROPAGATION_DEPTH = 1024` exists (`.../ecs/constants.rs:462`), so the `usize → u32` narrowing of the traversal frontier is sound.
- **Rev 2.3 introduces no new unsafe, no new atomic and no new aliasing argument.** The Tree-Borrows surface (Heap/`HeapRef`, `SlotChunks`, `carve`) is untouched except by P36/O1, which *adds* an alignment contract. I walked TB-1..TB-4, the Miri legs and the loom list against this rev's deltas and found nothing to raise; that topic is absent from my remarks because it has no consequence here, not because I skipped it.

Both blockers are in text **Rev 2.3 introduced** (P32/32.4's ceiling, P32/32.5's K-MOD-11) and neither re-litigates an earlier round.

All code read read-only in `D:/wt/joltab` (`merge/ke16-into-ecsnative`) unless the citation names `D:/wt/reflect`; documents in `D:/claude/BoykoEngine`. Ledger on disk read as **rev 4**.

---

## Remarks

### 🔴 Critical (blockers — implementation must not start on §7)

#### C1. K-MOD-11 and R2's kind check are a second implementation of KF-47, which another lane has already written and the owner has already approved

**Where**: P32/32.5 (`ALLOCATOR-DESIGN-SPACE.md:3031-3037`, "**K-MOD-11 (NEW seam entry)**"), P32/32.1 R2 (`:2915-2923`), P33/33.2's G6(e) ten-item list (`:3115`).

**Problem**: the patch reads `RUNTIME-DATA-LEDGER.md` — it is in Rev 2.3's own files-read list, and P34 quotes three of its rows — but not the row that already owns this rung.

`RUNTIME-DATA-LEDGER.md:823` and `:1658-1674`: **KF-47 "By-id structural seam (structural ops by ComponentId + bytes)", status active, "capability (shipped by the reflect lane)", owner-approved as B.13 #2**, described as "Attach / detach / mark-changed by ComponentId, with the bytes moved into the row, `#[require]` honoured, hooks and observers fired, and no heap in its own code". I verified the functions in the lane itself, not just the ledger's evidence lines:

- `D:/wt/reflect/crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs:200` — `pub fn add_component_by_id(`, whose doc block at `:188-199` already carries three named semantics K-MOD-11 does not have: an `AddOutcome::AlreadyPresent` refusal ("an editor's *Add Component* must refuse, not mutate"), a `#[require]` **expansion** that "pushes required ids unfiltered", and a dedicated sibling helper `migrate_entity_attach_ids_with_bytes`.
- `:500` — `pub fn remove_component_by_id(&mut self, entity: Entity, id: ComponentId) -> bool`, with a MEASURED, `g17`-gated asymmetry recorded at `:488-499` (typed remove fires 1 entity-targeted observer, by-id fires 0).
- `:616` — `pub fn mark_component_changed(...)`.
- `D:/wt/reflect/crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs:213-234` — `EnableTagId::try_from_component_id`, whose doc is R2's own argument in the lane's words: "it is a PROOF, not a cast: the kind check is what makes an `EnableTagId` mean 'this id really is an enable tag'".

**Consequence**, three, each concrete:

1. **Two erased attach paths in one kernel, with different invariants.** P32/32.5 specifies `add_component_raw` as "the migration skeleton of `migrate_entity_insert` with the bundle write replaced by one erased memcpy" — no `#[require]` expansion is mentioned anywhere in the entry. A mod attaching a component whose type declares `#[require]` targets then produces an entity the engine's own typed path could never produce, and the repair (`add_component_by_id`) exists three files away.
2. **G6(e) is red on arrival.** Its list is pinned at "**ten items**, exactly (an extra entry is red, a missing entry is red)". Whichever lane merges second adds an eleventh erased entry point, so the gate fails for the reason it was built to catch — a seam growing silently — while nothing about the seam actually grew.
3. **R1 and the lane collide in one file.** R1 renames `TAG_NAMES` → `DYN_NAMES` and changes its value type inside `component_registry/tags.rs`; the reflect lane has just appended a second inherent `impl` block to that same file with an explicit note that "every doc anchor into this file is at or below `:192`, three of them waived, and a waived anchor rots SILENTLY on a line shift".

**Confidence**: CONFIRMED (ledger `:823`, `:1658-1674`; reflect lane `seam_by_id.rs:200, :500, :616`; `tags.rs:213-234`).

**Why critical**: this is this repository's own catalogued failure — a sibling plan owning your rung — and pass 3 set the standard that the §7 repair direction "must be ruled before any seam entry is written", because it decides whether §7 stays additive. It also reaches the owner's hard requirement: a duplicate erased attach path is kernel code a no-modding build carries for the modding seam's benefit, and G6(b)'s five pinned symbols cannot see it.

**What is needed**: a ruling on whether §7 **consumes** KF-47 rather than growing its own. If it consumes it, K-MOD-11 becomes "mark the shipped `add_component_by_id`/`remove_component_by_id` with MOD-SEAM", the ten-item list is re-derived against the lane's names, and the `g17` observer asymmetry is stated as an inherited limit a mod sees. If it does not, the design owes a named reason why two by-id attaches are correct, priced against G6, and an ordering with the reflect lane's merge. Note also that the seam as written has an attach and **no per-entity detach** — `remove_component_by_id` is the answer the lane already has, so the "complete additive list" is simultaneously duplicating and incomplete.

#### C2. `DYN_ID_CEILING` turns a per-slot pin into a global floor, and its gate cannot tell the two apart

**Where**: P32/32.4 (`:3008-3029`): "The lowest id any out-of-band pin has claimed. Lowered by `register_layout`, never raised"; `register_layout::<T>(id)` adds `DYN_ID_CEILING.fetch_min(id, Relaxed)`; "`try_register_dynamic` **and `register_new`** compare against `DYN_ID_CEILING.load(Relaxed)` instead of the constant."

**Problem**: `register_layout` does not reserve a band. Its shipped contract is one slot — "Registers `T` under an explicit `component_id`", panicking only if *that* slot holds a different type (`D:/wt/joltab/.../component_registry/mod.rs:1012-1029`) — and its two sanctioned callers are (1) **tests pinning known fixed ids** and (2) synthetic scratch bands. Only (2) is a band. The tree is full of (1), at scattered mid ids:

| id | site (joltab) |
|---|---|
| 100 / 101 / 102 | `.../ecs_master/ecs_master.rs:1326-1328` (a `#[cfg(test)]` module **inside the lib**) |
| 103 | `crates/boyko_ecs/tests/dense_d1_store.rs:50` |
| 220 / 224 / 225 | `.../memory/component_pool.rs:2463-2472` |
| 323 / 324 | `.../ecs_master/enable_tag_api.rs:355-356` |
| 335 | `.../commands/migration_helpers.rs:2225` |
| 420 | `.../component/component_pool_bundle.rs:528` |
| 440 / 441 | `.../change_detection/check_ticks.rs:417-418` |
| 462 / 465 | `.../component_registry/mod.rs:1396, :1409` |
| 490-493 | `.../iters/query_state.rs:605-614` |
| 498 | `.../system/dispatcher_token.rs:471` |
| 510 | `.../resources/resource_registry.rs:365` |

`fetch_min` reads every one of those as "everything above me is reserved".

**Consequence**: in any process where one of those pins runs, the mint ceiling becomes that id. The `boyko_ecs` **lib test process** pins 100 at `ecs_master.rs:1326`, so `register_new` there asserts `raw < 100` and `try_register_dynamic` (dynamic tags **and** every asset backing — `.../asset/backing.rs:115-134` mints through it) returns `None` at `NEXT_ID ≥ 100`, with 412 ids free. Whether a given binary actually reds depends on how many types it mints, which is precisely the "true until it isn't" this file refuses to ship elsewhere. The production direction is the same shape: a physics build's ceiling lands at the band's bottom and refuses the ~38 ids the band reserves but does not use (`scratch_ids.rs:676-685`: 128 reserved, ~90 needed).

Two further defects in the same mechanism:

- **The claim "fail-closed" covers only one direction.** "A stale-high read cannot occur before the pin, because a pin that has not happened cannot be collided with" is false for the ordering that actually panics today: the ascending counter can consume a band slot *before* the pin installs, and the pin then takes `register_layout`'s different-type panic. The ceiling does nothing about that, so the misleading-panic hazard 32.4 says it fixes is only half fixed.
- **The gate cannot discriminate.** "pin 400; assert the next `try_register_dynamic_by_name` returns `Err(IdSpaceExhausted)`" passes under both the intended band semantics and the broken per-slot-as-floor semantics. And the gate requires the error to name "the pinning site's id", which an `AtomicUsize` holding one number cannot supply.

**Confidence**: CONFIRMED (the mechanism against `mod.rs:1012-1029` and the twelve pin sites above; the gate's blindness by reading its own two arms).

**Why critical**: it is a new mechanism on the kernel's own mint path (`register_new`), shipped in every build including the no-modding one, justified as a bug fix the engine wants on its own terms — and as specified it refuses ids that are free, in the test suite that is supposed to certify it.

**What is needed**: separate *reserving a band* from *pinning a slot*. A band needs its own entry point (the physics scratch module and P37/3's `TraversalScratch` are its only callers, and it can carry the floor plus a site id for the error message); `register_layout` then keeps its per-slot contract and lowers nothing. Whatever shape is chosen, the gate needs a second case that must stay **green**: a mid-id pin must NOT reduce the budget. And P37/3 owes the band its two ids come from — today the patch says "registered through `register_layout`" without naming a range, which under `fetch_min` makes the id choice a process-global effect.

---

### 🟡 Important

#### W1. R2's kind oracle is read at a point where the shipped ordering argument does not hold

**Where**: P32/R2 (`:2915-2923`) against `D:/wt/joltab/.../component_registry/tags.rs:122-141`.

The enable-tag mint classifies **after** the intern lock is released, and the shipped doc justifies that with a scope R2 invalidates: "The kind classification (`set_storage_kind`) runs AFTER that lock is released — this is sound because storage kind is write-once-idempotent and **registration completes before any archetype can read the kind**" (`:122-129`). R2 makes the kind a *lookup-path* read, not an archetype-construction read. P32 says `try_register_tag_by_name` and `try_register_enable_tag_by_name` become "two-line callers of `intern_or_mint(name, 0, 1)` plus their kind filter" and does not say where the classification lands.

**Consequence**: thread A `register_enable_tag("x")` returns from `intern_or_mint` (name interned, lock released, kind still the default) while thread B `tag_by_name("x")` reads `size == 0 && kind == Table` and gets a `TagId` for an id that is about to become `Bitset` — the 32.2 chain, through the filter that exists to stop it. The shape is the one §1 says the suite exercises (many `EcsMaster`s built concurrently) and the UI plugin re-registers its three enable tags on every build (`boyko_ui/src/interaction/plugin.rs:92-94`).

**Solution options**: state that `set_storage_kind` runs **inside** `intern_or_mint`'s lock (one sentence, and the map insert already happens there); or keep it outside and write the replacement ordering argument, since the one in the file is now about the wrong reader.

#### W2. P35's own rule ("the gate must not name a type that does not exist") is unapplied at the first gate the plan builds

**Where**: P1's C1 red-first Miri test (`:757`, `:760`), against P35's retirement of `HeapString` (`:3207-3233`) and P14 step 3 ("**C1's red-first Miri test lands in this step, before any client**", as edited by P34 site 15).

The test body still says "Repeat with `HeapBox`, `HeapDyn`, **`HeapString`**, `SortedMap`", and its anti-vacuity floor is "a counter incremented in `HeapVec::drop`'s free must read **≥ 5** (one per handle type)". P35 fixed 15.5's and 19.2's hard-coded client sets by making them *derived*; this third site was missed, and it is the one with a number attached.

**Consequence**: the first artifact of P14 step 3 does not compile as written, and the floor is re-derived at the keyboard — which is how an anti-vacuity constant gets quietly lowered. The number also does not survive derivation: with `HeapString` gone the `HeapVec::drop` count is `HeapVec(1) + SortedMap(2) = 3`, because `HeapBox` and `HeapDyn` do not free through `HeapVec::drop` at all — so "≥ 5, one per handle type" was already counting something other than what it says.

**Solution options**: derive the gate's handle list from §2.1's Heap row as P35 did for the other two, and re-derive the floor from the free sites the body actually exercises rather than from a handle-type count.

#### W3. Four more superseded passages stand as current, and one of them prescribes the receiver pass-1 C1 was raised to kill

**Where**: `:394`, `:1171`, `:405`, `:112`.

Rev 2.3 explicitly hunts this class (it found two instances pass 4 did not name) and P30/O1 made "replace, don't annotate" the rule. Four remain live:

1. **`:394` — §5.1's consumer table: `boyko_utils::SparseMap::new_in(&mut Heap)`.** This is the `&mut Heap` receiver P1 replaced with `HeapRef` to close pass-1 C1, standing in the table a developer consults for the API surface, three rows below the two rows P33 *did* replace. It is doubly stale: P31/3's closed `hv3-owner` vocabulary already ruled that a `SparseMap` user outside the four owner classes "cannot become a heap row and takes `VmColumn` instead". *Consequence*: rev 2.1 added mutation M2 precisely because "a developer picks the exit that compiles"; §5.1 currently hands them that exit in writing.
2. **`:1171` — TB-3**, one of the library-wide invariants, is still stated as "`FrameVec<'f, T>` borrows the arena (and `reset(&mut self)` makes an outliving handle a compile error, P4)". P34 deleted the class and P4 with it, so half of a numbered invariant now has no subject.
3. **`:405` — §5.2's determinism row** "Frame arena | dispatcher-only; `debug_assert!(owner == current)`" — an obligation owed for a deleted class.
4. **`:112` — §1**: "`core::fmt::Write` into a `HeapString`/`LogRing` replaces it", the only remaining place the retired primitive is named as a destination.

**Solution options**: one `**Removed**`/`**Added**` block each; for (1) the replacement is already decided (`HeapRef` receiver, and the destination is `VmColumn` per P31/3), for (2) TB-3 reduces to its scope half.

#### W4. P34 and P37 spend `ComponentId`s the budget accounting does not count, and M-A13 is specified against the pre-spend tree

**Where**: P34's destinations table (`:3137-3143`), P37/3 (`:3274-3276`), M-A13 (`:3029`).

Every `ScratchColumn` costs one registered `ComponentId` out of the same 512 (`scratch_column.rs:50-104`). P37/3 counts this honestly for traversal ("**two `ComponentId`s**") but P34 routes far more onto the same primitive without counting: 21 KF-44 rows (`RUNTIME-DATA-LEDGER.md:820`), the UI lanes as CSR **pairs** (offsets + flat), and rung 2's CSR two-pass. Meanwhile C2's ceiling makes the usable range a runtime quantity and `boyko_modding`'s `MAX_MOD_TYPES = 128` is to be checked against M-A13 — which is specified as "the distinct-id census at steady state for `boyko_demo` and the playground scene", i.e. on a tree from *before* these destinations land.

**Consequence**: the number the mod cap is validated against will be measured against a tree that has not yet spent the ids this design commits it to spending, so a green M-A13 does not license `MAX_MOD_TYPES = 128`.

**Solution options**: state the id cost of the new destinations as a projected delta beside M-A13's measured base (the ledger's per-row destinations make it countable today), or give the scratch ids their own reserved band so they are not drawn from the mint range at all — which is also C2's fix.

---

### 🟢 Optional

#### O1. K-MOD-6's restated constraint is over-broad, and G6(e) pins the row

P33/33.2 rewrites K-MOD-6 as "`drop_fn` … is **never written for a dynamic id**". The seam-scoped half is right; the general claim is false in the tree: `register_asset_layout` mints through `try_register_dynamic` with a real `TypeId`, a non-zero size and `drop_fn: Some(...)` (`.../asset/backing.rs:115-134`; `boyko_render/src/texture.rs:583-585` supplies `TextureGpu::drop_glue`). A developer implementing the sentence as an assert breaks the asset system. The gate row in 32.6 already scopes it correctly ("refused **at the seam**"); only the inventory prose needs the same scope.

#### O2. "Binary size" is the one dimension of the owner's requirement with no check

G6 pins five `.text` symbols, three `size_of`s, startup numbers, symbol absence, the seam inventory and feature unification. The requirement enumerates "no exported symbol, **binary size** or startup work", and the design's answer for the seam's own code is "no caller ⇒ dropped at link" — an assertion nothing measures, and P36/O2 deliberately moves the `.text` measurement to the *rlib's object members*, where an unused function is still present. One absolute number (linked `boyko_demo` `.text`, or section size, pinned at the same parent commit and re-blessed under the same discipline) closes it at the cost of one row.

---

## Status of earlier findings

| Round | Row | Status |
|---|---|---|
| pass 4 | C1 (intern the seam does not expose; `TagId` cannot hold a sized component) | ⛔ **partially closed** — R1/R2/R3/R4 are the right shape and the 32.2 by-product is a genuine find, but the mechanism duplicates the reflect lane's shipped kind check and the seam duplicates its by-id ops (pass-5 C1); the classification-outside-the-lock window is pass-5 W1 |
| pass 4 | W1 items 1, 4 (stale SEND11 rationale, per-slot Miri row) | ✅ closed by deletion (P34) — verified against the ledger rows it rests on |
| pass 4 | W1 item 2 (§5.1 rows) | ✅ closed for the two rows named; a third row at `:394` was not in the critique and is now pass-5 W3 |
| pass 4 | W1 item 3 (K-MOD-6) | ✅ closed, modulo O1's scope |
| pass 4 | W2 (`HeapString` / rung 4) | ✅ closed by retirement, with the finding correctly extended to rung 4's `HeapVec` rows; the gate body at `:757` was missed (pass-5 W2) |
| pass 4 | O1 (`carve` alignment) | ✅ closed — contract + `debug_assert`, with the right reason ("the premise changes when `std::alloc` is replaced by a bump") |
| pass 4 | O2 (`.text` on PE/COFF) | ✅ closed — and the critique's churn case correctly refuted ("a function inserted between two symbols is itself a symbol"); per-leg pins named |
| pass 4 | O3 (owner attribution) | ✅ closed, with a self-correction that matters: `const OWNER: CommitOwner` needs `adt_const_params`, which is unstable, so the marker-type form is right |
| pass 4 | questions 1-4 | ✅ answered; Q3's answer verified against `scratch_column.rs:50-104` and `constants.rs:462` |
| pass 3 | C1, C2, W1-W4, O1, O2 | ✅ stand closed; P34 deletes C1's subject without weakening anything that depended on it |
| pass 2 | C1-C4, W1-W6, O1-O3 | ✅ stand closed |
| pass 1 | C1, C2, W1-W5, O1-O9, Q1-Q6 | ✅ stand closed, except that C1's rejected `&mut Heap` receiver still appears at `:394` (W3) |

---

## Positive — preserve these

1. **32.2 is the best thing in Rev 2.3.** Tracing a modding blocker into a live, three-`pub`-method corruption chain in the shipped kernel, and then paying for the repair out of *that* budget instead of the modding one, is the correct way to answer "is this filter a modding cost?". I verified all four steps; they are as written.
2. **P34 deletes a primitive rather than shipping it unused, on an inventory rather than on a benchmark**, and names what is lost (FR-1) and what would bring it back. `RUNTIME-DATA-LEDGER.md:126` and `:820` say exactly what the patch quotes.
3. **The W4 self-correction in 32.3** — "the stronger claim rev 2.2 made ('keeps all 512') was wrong … P21's mod range `[384, 512)` was byte-for-byte the physics band" — is a correction that makes the architect's own earlier case *harder*, verified at `boyko_physics/src/scratch_ids.rs:685` and its census at `:671-684`. Keep the habit and keep this paragraph.
4. **P36/O3's refusal of an unstable feature for a gate's benefit**, reusing P29's rule. The marker-type-with-associated-index form is right and costs nothing.
5. **P32/32.7's cost table is the right artifact for the owner's requirement**, and the one line I checked hardest holds: `register_new` really is reached once per type behind the derive's per-monomorphisation `OnceLock` (`.../component_registry/mod.rs:908-914`), so the added load is cold.
6. **Everything the earlier rounds asked to preserve is untouched** — P0's withdrawn throughput justification, P3's protector-kind argument, P6's syntax-over-text ledger, P11's TB-1 rationale correction, P17's release-sequence argument, P20's push-side liveness proof, P24's FR-1 (now as a revival rule), P25's priced refusal, P29's "I am not paying that".

---

## Open questions for the architect

1. **Does §7 survive C1 as a seam at all, or does it become a consumer of KF-47?** If the second, the patch shrinks: K-MOD-11 disappears, the ten-item list becomes a marking exercise over `seam_by_id.rs`, and the `g17` observer asymmetry becomes an inherited limit to document. I would take that outcome, and I note it is cheaper than the one Rev 2.3 specifies.
2. **Which band do P37/3's two traversal ids come from?** Under C2's mechanism the answer is a process-global effect, and under W4 it is a budget line. Name the range and the site that reserves it.
3. **Pass-5 question 2 of your own list (P34 resting on a concurrently-revised ledger) is the right worry and the revival gate is the right instrument** — but the check should also run the other way: rev 5 may *move* rows onto `ScratchColumn` and change W4's arithmetic. One line in rung 1's entry criteria ("re-read the ledger revision and re-derive the id delta") settles both directions.
4. **`DYN_ID_CEILING`'s error is required to name "the pinning site's id"** (32.4's gate). A single `AtomicUsize` cannot carry it. If the site matters for the diagnosis — and the whole point of 32.4 is replacing a misleading panic with a named error — the reserving entry point needs to record it; if it does not matter, drop the clause so the gate does not assert what the mechanism cannot produce.

---

# Rev 2.4 (2026-09-17)

# Allocator design — Rev 2.4 (patch against Rev 2.3)

**Scope.** Pass-5 C1, C2 (blockers), W1-W4, O1-O2, and pass-5 open questions 1-4. Eight patches: **P38** (C1), **P39** (C2 + questions 2, 4), **P40** (W1), **P41** (W2), **P42** (W3), **P43** (W4 + question 3), **P44** (O1, O2), **P45** (question 1 + what stays provisional).

**Trees.** Documents in `D:/claude/BoykoEngine` (`feat/multi-paradigm-render`). Code read read-only in `D:/wt/joltab` (`merge/ke16-into-ecsnative`) unless a citation names `D:/wt/reflect` (`feat/reflection` @ `0e0b4c68`, per the ledger's own tree table `RUNTIME-DATA-LEDGER.md:11`). **Ledger on disk read as rev 4** (`RUNTIME-DATA-LEDGER.md:1`, `:13` — "Date: 2026-09-16. Revision: 4").

**Both blockers are conceded in full.** C1's duplicate is real and the patch deletes the duplicate rather than justifying it; C2's mechanism is deleted outright, not repaired — and the replacement is *cheaper* than the thing it replaces (no static, no atomic, no per-mint load), which is the argument that decided it.

---

## P38 (closes pass-5 C1) — §7 CONSUMES KF-47; the seam adds no attach path of its own

**Depends on / re-read before judging:** P12 (K-MOD table, G6), P26/26.1 constraint 1 (`drop_fn: None`), P32/32.1 R2 (the kind oracle), P32/32.6 (the gate rows), P33/33.2 (G6(e)), P29 (the ~~five~~ four `.text` pins ⚠ *Rev 2.5 (P49)*).

### 38.1 The ruling

**§7 consumes the reflect lane's shipped by-id seam. It does not grow a second one.** I verified the functions in the lane rather than taking the ledger's word:

| capability §7 needs | what already ships | site (`D:/wt/reflect`) |
|---|---|---|
| attach a sized component by id, from bytes | `EcsMaster::add_component_by_id(entity, id, bytes) -> AddOutcome` | `crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs:200` |
| detach by id | `EcsMaster::remove_component_by_id(entity, id) -> bool` | `seam_by_id.rs:500` |
| stamp a changed tick after a raw write | `EcsMaster::mark_component_changed(entity, id) -> bool` | `seam_by_id.rs:616` |
| turn a `ComponentId` into a *proven* enable-tag handle | `EnableTagId::try_from_component_id(id) -> Option<Self>` | `component_registry/tags.rs:225` (block at `:213-234`) |

Ledger rows: `RUNTIME-DATA-LEDGER.md:823`, `:1658-1674` (KF-47, "capability (shipped by the reflect lane)", owner-approved as B.13 #2).

**Four properties the shipped pair has and `add_component_raw` did not**, each read at the site, each a reason the duplicate would have been the *worse* of the two:

1. **`#[require]` is honoured** (`seam_by_id.rs:166-181`): the transitive closure is expanded, missing members are constructed through their capture-free `RequiredCtor`, and the dense phase order is reproduced, MEASURED against the typed path by the lane's `g13b`. For a *dynamic* id the closure is empty by construction — `REQUIRES_DIRECT` is populated only from the derive-generated path, gated on `Component::HAS_REQUIRES` (`component_registry/required.rs:129-134`) — so a mod pays one cold table probe; but a mod that attaches an **engine** component by id gets the engine's own semantics instead of an entity the typed path could never produce.
2. **A refusal channel instead of a `bool`**: `AddOutcome::{Attached, AlreadyPresent, Rejected(RejectReason)}` (`:94-131`), with `AlreadyPresent` not clobbering data — the semantic an editor and a mod both need and `add_tag` deliberately does not have.
3. **Five ordered arms, all release-active**: liveness → storage kind (`Table | Dense`, `:222-228`) → registered layout (`:233-235`) → residency (`Gpu` refused *before* the archetype resolver, `:240-242`) → byte length as a plain `if`, never a `debug_assert` (`:244-250`). `add_component_raw` specified one of these five.
4. **A dense arm at all.** `add_component_raw` was specified as "the migration skeleton of `migrate_entity_insert`" and would have answered `false` presence for every dense component (the signature probe cannot see dense membership, `:252-269`).

A sized dynamic id reaches those arms unchanged: `STORAGE_KIND`'s default reads back `Table` for every id never explicitly classified (`D:/wt/joltab .../component_registry/mod.rs:372-376, :387-398`), and the seam's mint registers a real layout. **No kernel change is needed to make a mod component attachable.**

**Removed** (P32/32.5, `:3031-3037` — the *heading and the blockquote only*; the paragraph beneath the heading is KEPT because it is the argument for why a by-id attach must exist at all, i.e. the argument KF-47 already acted on):

> ### 32.5 K-MOD-11 — the sized attach, priced and refused a refactor

> > **K-MOD-11 (NEW seam entry): `EcsMaster::add_component_raw(entity: Entity, id: ComponentId, bytes: &[u8]) -> bool`** — `#[cold] #[inline(never)]`, non-generic, MOD-SEAM-marked. Migrates `source → source ∪ {id}`, memcpys `bytes` into the new row (length checked against `get_layout(id).size`, refused on mismatch), stamps the added tick, and only then fires `on_add`/`on_insert`. It duplicates the migration skeleton of `migrate_entity_insert` with the bundle write replaced by one erased memcpy.
> >
> > **The refactor that would remove the duplication is priced and refused.** Sharing one erased core would replace a monomorphised, constant-size memcpy on the engine's own structural-insert path with a runtime-length one, paid by every game on every `insert`, to save duplicated code inside a `#[cold]` function. Cost when modding is absent: **zero** — no caller, so the function is dropped at link, and none of P29's five pinned symbols contains it.

**Added:**

> ### 32.5 K-MOD-11 — the sized attach is CONSUMED from KF-47, not written
>
> …(the `add_tag`-is-not-the-path paragraph stands unchanged above)…
>
> > **K-MOD-11 (no new kernel item): the by-id structural ops are the reflect lane's shipped KF-47**, marked `MOD-SEAM` and otherwise untouched: `EcsMaster::{add_component_by_id, remove_component_by_id, mark_component_changed}` (`seam_by_id.rs:200, :500, :616`) and `EnableTagId::try_from_component_id` (`tags.rs:225`). §7's attach half was simultaneously **duplicating** (a second erased attach with weaker invariants) and **incomplete** (an attach with no per-entity detach); consuming closes both with one ruling.
> >
> > **The duplication question was already priced by the lane, and the same way.** It did not make the typed insert erased either; it added a sibling helper `migrate_entity_attach_ids_with_bytes` whose only caller is the by-id attach (`seam_by_id.rs:497-499`). The refusal Rev 2.3 wrote — never put a runtime-length memcpy on the engine's own `insert` — is therefore preserved as a *fact about the tree*, not as a fresh argument.
> >
> > **Cost when modding is absent: zero, and now zero for a stronger reason.** KF-47 is shipped kernel capability with engine clients of its own (scene load, prefab KF-13, serialize KF-08/09, undo, network apply — `RUNTIME-DATA-LEDGER.md:1662`). §7 adds **no function** to the kernel for attach/detach/mark, so there is nothing for a no-modding build to drop at link and nothing for G6(b)'s ~~five~~ four `.text` pins ⚠ *Rev 2.5 (P49)* to fail to see. Rev 2.3's answer relied on link-time dead-code elimination of a function §7 had created; this one does not create it.
> >
> > **Two inherited limits a mod sees, stated because a mod author hits them and the seam did not author them:**
> > - **entity-targeted observers do not fire on a by-id detach.** MEASURED by the lane: typed remove fires 1, by-id remove fires 0, and the call still returns `true` (`seam_by_id.rs:486-499`). The cause is a *pre-existing shared* helper (`migrate_entity_detach_ids`, also `remove_tag`'s), it is filed in `docs/OPEN-QUESTIONS.md` and gated RED-by-design by `g17`. §7 inherits it and must not "fix" it inside the modding rung — that is a kernel change with `remove_tag` in its blast radius.
> > - **the bytes are MOVED, not borrowed** (`seam_by_id.rs:156-164`): a caller that keeps its own live value and passes a byte view of it double-drops, through a `pub fn` carrying no `unsafe`. **Ruling, and it costs the kernel nothing:** `boyko_modding`'s wrapper refuses an id whose `get_layout(id).drop_fn` is `Some` before forwarding. Mod component types are POD by K-MOD-6, so the refusal is vacuous for them; what it actually blocks is a mod attaching an **engine** droppable component from a byte view. One cold branch, in the crate that is not compiled when modding is absent.

### 38.2 R2's enable-tag row is deleted — the handle it wanted already exists

**Removed** (P32/32.1 R2's table, `:2920`, verbatim):

> | (enable-tag accessor, **not added until a client exists**) | `size == 0 && kind == Bitset` | `EnableTagId` |

**Added:**

> | *(none — the by-id half ships as `EnableTagId::try_from_component_id` (`D:/wt/reflect .../tags.rs:225`), whose doc states R2's own rule in the lane's words: "it is a PROOF, not a cast: the kind check is what makes an `EnableTagId` mean 'this id really is an enable tag'". A mod composes `dynamic_by_name(name) -> ComponentId` with it; the kernel grows no name-keyed enable-tag accessor)* |

### 38.3 G6(e)'s inventory, re-derived against the lane's names and given a membership RULE

**Removed** (P33/33.2, the G6 (e) row it added, `:3115`, verbatim):

> | (e) **the seam cannot grow silently** (NEW) | G1's `syn` scanner (P6 — the parse already happens) asserts that the set of kernel **function items** carrying the `MOD-SEAM` marker equals **this list, exactly** (an extra entry is red, a missing entry is red): `HeapRef::{alloc, free, grow}`, `Heap::new`, `ComponentPool::new`, `ComponentLayout::new_dynamic`, `component_registry::{try_register_dynamic_by_name, dynamic_by_name}`, `EcsMaster::{remove_component_type, add_component_raw}` — **ten items**. The marker is a doc line `/// MOD-SEAM: K-MOD-n` (syn parses doc attributes; an inert attribute would need a proc macro and would cost the kernel a dependency for a gate's benefit — P29's refusal applied again). The five that exist *only* for the seam additionally carry `#[doc(hidden)]`; `HeapRef::*`, `Heap::new` and `ComponentPool::new` are ordinary `pub` because the engine itself calls them, and pinning them by doc-visibility would have made the rule unstatable. K-MOD-2/3/7/8/9 are **not** function items and are pinned by (a), (c) and O6's allowlist instead | add an eleventh erased entry point without a ledger row → red |

**Added:**

> | (e) **the seam cannot grow silently** (NEW) ⚠ *Rev 2.5 (P47): = UG-15 leg (5); the list is 05 §3's, by this rule — the four `HeapRef`/`Heap::new` items leave with U-1, `remove_component_type` with U-10, and class A is MS-02b's (option A, Stage 3)* | G1's `syn` scanner (P6 — the parse already happens) asserts that the set of kernel **function items** carrying the doc-line marker `/// MOD-SEAM: K-MOD-n` equals the list below, **exactly** (an extra entry is red, a missing entry is red). **Membership is a rule, not a taste** (P29's form): an item is in the inventory iff a mod can reach it **and** its operation is *erased* — it takes a runtime `Layout`, a `ComponentId`, or raw bytes. It is **class A** (`#[doc(hidden)] pub` + marker) iff it has **no engine caller**, and **class B** (ordinary `pub` + marker) otherwise; pinning by doc-visibility alone would have made the rule unstatable. **Class A — 4:** `ComponentLayout::new_dynamic`, `component_registry::{try_register_dynamic_by_name, dynamic_by_name}`, `EcsMaster::remove_component_type`. **Class B — 9:** `HeapRef::{alloc, free, grow}`, `Heap::new`, `ComponentPool::new`, `EcsMaster::{add_component_by_id, remove_component_by_id, mark_component_changed}`, `EnableTagId::try_from_component_id`. **Thirteen items.** K-MOD-2/3/7/8/9 are not function items and are pinned by (a), (c) and O6's allowlist instead | add a fourteenth erased entry point without a ledger row → red; delete a marker line → red |

**Why the count grew from ten to thirteen while the kernel grew by nothing:** three of the four additions are code that ships either way and is now *named* by the gate; the tenth item (`add_component_raw`) was deleted. The inventory got bigger and the seam got smaller, which is the direction (e) exists to detect — and it is now derivable from the rule rather than from a list a merge can invalidate.

### 38.4 Ordering against the reflect lane, and the anchor obligation R1 owes

**Entry criterion for §7's rung, stated so the two lanes cannot both write it:**

- §7's modding rung **does not start** until KF-47 is merged into the branch this campaign builds on. It is not re-implemented in the interim. If the reflect lane is abandoned, §7 adopts `seam_by_id.rs` *as the lane wrote it* (the design points at the file, not at a specification of it) and the rung's diff becomes the four marker doc lines.
- **R1 (`TAG_NAMES` → `DYN_NAMES`, the rewrite of `tags.rs:155-207`) lands AFTER that merge**, because the lane has appended a second inherent `impl` block to the same file (`D:/wt/reflect .../tags.rs:209-235`) with an explicit warning that "every doc anchor into this file is at or below `:192`, three of them waived, and a waived anchor rots SILENTLY on a line shift".
- **Obligation, not a hope:** the rung runs the repository's anchors gate after R1 and **re-blesses the three waived anchors in that file explicitly**, recording the old and new lines in the commit. A waived anchor is the one class the gate cannot catch; the repair is a human re-read, scheduled here so it is not discovered later.
- Adding the four marker doc lines shifts lines inside `seam_by_id.rs`, whose own header counts **54 citations across seven documents at 23 distinct anchors** (`seam_by_id.rs:63-70`). Same obligation, same commit.

### 38.5 The cost table, corrected

**Removed** (P32/32.7, the row at `:3068`, verbatim):

> | `new_dynamic`, `try_register_dynamic_by_name`, `dynamic_by_name`, `add_component_raw`, `remove_component_type` | no caller ⇒ dropped at link; G6(a) still asserts no exported symbol |

**Added:**

> | `new_dynamic`, `try_register_dynamic_by_name`, `dynamic_by_name`, `remove_component_type` (class A — four, not five) | no caller ⇒ dropped at link, now **measured** by ~~G6(f)'s~~ G6(g)'s `.text` delta rather than asserted ⚠ *Rev 2.5 (P47): that row cannot see a kernel item; the measurement is UG-15 legs (7) and (7b), and under rule S-1 these items have no object code without an implementor*; G6(a) still asserts no exported symbol |
> | `add_component_by_id` / `remove_component_by_id` / `mark_component_changed` / `try_from_component_id` (class B) | **0** — shipped kernel capability with engine clients (KF-47); the seam adds a doc line to each |

---

## P39 (closes pass-5 C2, and pass-5 questions 2 and 4) — `DYN_ID_CEILING` is DELETED; the mint skips an occupied slot instead of dying on it

**Depends on / re-read before judging:** P26/26.1 (no id release, no partition), P32/32.3 (the seam's mint body), P32/32.6 row 6 (`DynMintError`), P37/3 (`ScratchColumn`'s id constraint), P21 (K-MOD-5's Shape cell as amended).

### 39.1 The concession, measured

The critique is right and the number is worse than it states. `register_layout` is a **per-slot** contract (`D:/wt/joltab .../component_registry/mod.rs:1012-1029`: "Registers `T` under an explicit `component_id`", two sanctioned callers, only one of which is a band), and the tree pins mid ids through it everywhere: **390 occurrences of `register_layout::<` across 82 files** in `D:/wt/joltab/crates` (ripgrep count), of which **at least 20 files are inside `boyko_ecs/src` itself** — `#[cfg(test)]` modules in the lib, which all run in **one** process. `ecs_master.rs:1326-1328` pins 100/101/102 there. Under `fetch_min` that single module sets the process mint ceiling to 100 and refuses 412 free ids to `register_new`, to `try_register_tag_by_name`, and to `register_asset_layout` — which mints every asset backing through `try_register_dynamic` and `expect`s the `Some` (`.../asset/backing.rs:115-134`).

**The gate could not have caught it** (pin 400 → expect `Err` passes under both semantics), and one `AtomicUsize` cannot name a pinning site, so 32.4's own error contract was unsatisfiable. Both points conceded.

### 39.2 The replacement

**Removed** (P32/32.4, `:3012-3029`, verbatim — the ruling, the static, the three bullets, the cost paragraph, the gate and the measurement entry; the two paragraphs of `:3008-3010` that *state the defect* are KEPT):

> **Ruling: the ceiling is derived from the pins that exist, not declared as a constant.**
>
> ```rust
> // component_registry/mod.rs
> /// The lowest id any out-of-band pin has claimed. Lowered by `register_layout`,
> /// never raised. A build that pins nothing keeps the full MAX_COMPONENTS.
> static DYN_ID_CEILING: AtomicUsize = AtomicUsize::new(MAX_COMPONENTS);
> ```
>
> - `register_layout::<T>(id)` adds `DYN_ID_CEILING.fetch_min(id, Relaxed)` — one relaxed RMW on a setup-only path that already does a `OnceLock::set`.
> - `try_register_dynamic` and `register_new` compare against `DYN_ID_CEILING.load(Relaxed)` instead of the constant. `register_new` is reached once per type behind the derive's per-monomorphisation `OnceLock`; the load is not on any per-frame path.
> - `Relaxed` is sufficient and the argument is the same one the module already makes for `NEXT_ID` (`mod.rs:34-39`): the atomic provides a *bound*, not a publication; the payload is published by `OnceLock::set`. A stale-low read refuses a mint that would have succeeded (fail-closed); a stale-high read cannot occur before the pin, because a pin that has not happened cannot be collided with.
>
> **Cost when modding is absent:** one `AtomicUsize` in `.bss`, one relaxed RMW per `register_layout` call (≈ 90 calls at boot in a physics build, per the `scratch_ids.rs` census), one relaxed load per *first* touch of a component type. Zero per-frame instructions, no `size_of` change, no new symbol in the G6(b) set, no startup allocation. **It is a bug fix the engine wants on its own terms** — the owner's standing rule is bugs before features — and the modding seam is its second beneficiary, not its justification.
>
> **Gate (new row in `tests/registry_id_ceiling.rs`):** pin 400; assert the next `try_register_dynamic_by_name` returns `Err(IdSpaceExhausted)` rather than panicking, and that the error names the ceiling and the pinning site's id. Red-first: delete the `fetch_min` → the mint walks into the pinned slot and the test observes the occupancy panic instead of the error.
>
> **Measurement entry (specified, not run): M-A13** — the distinct-id census at steady state for `boyko_demo` and the playground scene: `NEXT_ID` high-water, the count of pinned ids, and `DYN_ID_CEILING`'s final value. This is the number `boyko_modding`'s `MAX_MOD_TYPES` must be checked against; until it exists, 128 is a self-restraint with no headroom proof.

**Added:**

> **Ruling: no ceiling, no band mechanism, no new static. `register_layout` keeps its per-slot contract and lowers nothing; the MINT stops dying on a slot it does not own.**
>
> - **`try_register_dynamic`: an occupied slot is SKIPPED, not fatal.** The CAS loop already exists (`mod.rs:967-992`); on `LAYOUTS[current].set(..) == Err` the mint takes the next index and retries, and returns `None` only at `MAX_COMPONENTS`. `dynamic_slot_occupied_panic` (`:996-1010`) loses its only trigger and is **deleted** with its misleading "test-only escape hatch" message. The plan-O2 rule it protected is untouched and is the reason skipping is correct rather than convenient: the sentinel-`TypeId` idempotent arm stays refused, and advancing preserves the uniqueness it exists to protect (two names never alias one id).
> - **`register_new`: same shape, existing arms first.** Same-`TypeId` occupancy keeps its idempotent return (`:935-936`); *different*-`TypeId` occupancy retries with a fresh index instead of panicking; the release exhaustion assert stays exactly where it is, at `MAX_COMPONENTS`.
> - **Cost, and it is negative against Rev 2.3:** **zero added instructions on the success path** (the retry lives inside the branch that today panics), **zero bytes of `.bss`** (Rev 2.3 added 8 B and one relaxed RMW per pin plus one relaxed load per type's first touch — all deleted), **zero ids lost** by any build. `get_layout_unchecked`, `ComponentPool::new` and P29's ~~five~~ four pinned symbols are untouched. ⚠ *Rev 2.6 (P54/O2): four after P49. A count marker only: P39's mechanism and gate stand as pass 6 reviewed them*
> - **The two directions, stated separately, because Rev 2.3's "fail-closed" claim covered one of them.** *Mint walks into a pin* — **fixed**: the mint takes another slot; this is the direction modding makes worse, because mod names draw from the same upward counter. *Pin lands on a slot the counter already took* — **diagnosed, not fixed**: `register_layout`'s different-type panic stands and its message gains the id-space census below. Fixing that direction requires reserving a band before any mint, i.e. a compile-time floor, which charges the ids to every build including ones that never link the band owner — the exact cost W4 refused. It is refused again, with the overturn gate in 39.4.
> - **The diagnosis that replaces the atomic, at zero steady cost:** `#[cold] pub fn id_space_census() -> IdSpaceCensus` scans the 512 `LAYOUTS` slots once, on a path that has already failed, and reports `{occupied, static, dynamic, pinned_above_counter, next_id}`. `DynMintError::IdSpaceExhausted` carries it, and `register_layout`'s panic prints it. **This is strictly more than Rev 2.3's gate demanded** (it names every out-of-band occupant, not one id) and it costs one cold function with no caller in a steady frame.

### 39.3 The gate, which must now DISCRIMINATE

**Added** (replacing 32.4's single gate row; new file `tests/registry_id_mint.rs`, plus one solo binary):

> | # | Case | Verdict | Red-first mutation |
> |---|---|---|---|
> | G-MINT-1 ⚠ *Rev 2.6 (P54/O5): built in D-S1(i)'s isolated form (02 `:300-307`; 03 UG-19): its own binary, a relative pin, and it expects `Ok(j)` with `j != k + 1`* | mint one dynamic name → id `k`; `register_layout::<B>(k+1)`; mint a second name | **GREEN, and this is the discriminator**: the second name gets `k+2`, not an error and not a panic. The rejected `fetch_min` semantics fail this case, so the gate tells the two apart — which 32.4's could not | restore the occupancy panic → the test observes an abort instead of `k+2` |
> | G-MINT-2 ⚠ *Rev 2.6 (P54/O5): D-S1(i)'s form: its own binary; pin `P = next_id + 32`, not id 100, after asserting `free ≥ 201`* | pin id 100, then mint 200 distinct dynamic names | **GREEN**: all 200 succeed, none is 100 — *a mid-id pin does not reduce the budget*, the green the critique asked for | apply `fetch_min` on `register_layout` → the first mint past 100 fails |
> | G-MINT-3 ⚠ *Rev 2.6 (P54/O5): D-S1(i)'s form: `registry_mint_exhaustion.rs`, not ignored; its mutation panics out of bounds on `LAYOUTS[512]` rather than spinning* | own binary `tests/registry_id_exhaustion.rs`, **one** `#[test]` (it consumes the process id space; ignore class `solo` per the repository's reason-prefix vocabulary) — fill the space, then `try_register_dynamic_by_name` | `Err(IdSpaceExhausted)`, **no panic**, and the message carries the census counts | delete the `None` return at the ceiling → the mint spins |
> | G-MINT-4 ⚠ *Rev 2.6 (P54/O5): D-S1(i)'s form: its own binary, with the relative pin `P`, not id 100* | `register_layout::<A>(100)` twice (idempotent), then `register_layout::<B>(100)` | second is a no-op; third panics, naming both types **and** the census | none needed — this pins the shipped per-slot contract so a future "band" edit cannot widen it silently |

### 39.4 Overturn gate for the refused band (question 2, answered)

**P37/3's two traversal ids come from no band. They are ordinary mints.** `ScratchColumn::new` needs "a registered `ComponentId` whose layout matches `T`" (`scratch_column.rs:50-104`) — and `register_new::<T>()` registers `ComponentLayout::new_static::<T>()`, which *is* that (`mod.rs:920-947`). `register_layout` was never required; it was required only if the ids had to be **compile-time constants**, which is why `boyko_physics` uses it (its cohorts are computed, e.g. `broadphase_column_id(k) = SCRATCH_ID_BROADPHASE_TOP - k`, `scratch_ids.rs:697-702`) and why the traversal scratch does not. So: two `register_new` calls behind two file-local `OnceLock`s inside the non-generic `TraversalScratch::new`, **no pin, no band, no ceiling interaction, and no process-global effect beyond consuming two ids** — which P43 counts. ⚠ *Rev 2.5 (P50, U-2): superseded — no id at all; `TraversalScratch` uses `ScratchColumn::for_type` (KC-10). P39's mechanism (39.2) and gate (39.3) are unchanged.*

**The physics band is untouched.** `SCRATCH_REGION_MIN_ID = MAX_COMPONENTS - 128` with its compile-time floor assert (`scratch_ids.rs:685, :690-695`) and its 142-type census (`:671-684`) remain the shipped protection: a margin plus a loud panic, exactly as its own doc says. ⚠ *Rev 2.5 (P50, U-2): the band is deleted at D-S2 (KC-10); its 128 ids go to 0.*

**Overturn gate (the one measurement that would buy the band back):** if M-A13 shows the production counter's steady high-water above **320** (i.e. within 64 of the physics floor of 384) in any shipping scene, the band becomes a kernel-declared `const` with the mint ceiling derived from it, and every build pays the 128 ids. Until then that cost is charged to nobody. ⚠ *Rev 2.5 (P50, U-2): void with the band; U-2's own overturn is D-S2 finding an untracked-pool path that needs a registered id.*

---

## P40 (closes pass-5 W1) — the kind classification moves INSIDE the intern lock

**Depends on / re-read:** P32/32.1 R2 (kind is a lookup-path read now), P32/32.3 (the shared `intern_or_mint` body), `tags.rs:122-141, :169-171` (the shipped ordering argument).

The critique is correct and the invalidation is precise: the shipped soundness argument is scoped to archetype construction — "storage kind is write-once-idempotent and **registration completes before any archetype can read the kind**" (`tags.rs:122-129`) — and R2 makes the kind a *lookup-path* read, which happens while another thread is still inside registration. The tree exercises exactly that shape (many `EcsMaster`s built concurrently; the UI plugin re-registers three enable tags on every build).

**Removed** (P32/32.3, the "Added" block's second sentence at `:2972`, verbatim):

> `ComponentLayout::new_dynamic(name, size, align)` is still added, still with the `DynamicTagMarker` sentinel and **`drop_fn: None` asserted at the seam**; `try_register_tag_by_name` and `try_register_enable_tag_by_name` are rewritten as two-line callers of `intern_or_mint(name, 0, 1)` plus their kind filter, so there is **one** mint body, not three.

**Added:**

> `ComponentLayout::new_dynamic(name, size, align)` is still added, still with the `DynamicTagMarker` sentinel and **`drop_fn: None` asserted at the seam**. **The kind is a parameter of the mint, applied under the intern lock:** `intern_or_mint(name, size, align, kind: StorageKind)` does lookup → layout compare → capacity check → mint → **`set_storage_kind`** → insert-into-`DYN_NAMES`, all inside the one `Mutex` the shipped path already holds (`tags.rs:169-171`). `try_register_tag_by_name` = `intern_or_mint(name, 0, 1, Table)`, `try_register_enable_tag_by_name` = `intern_or_mint(name, 0, 1, Bitset)`, the seam = `intern_or_mint(name, size, align, Table)` — **one** mint body, not three, and the shipped mint-then-reclassify at `tags.rs:134-141` is deleted rather than kept.
>
> **The ordering argument is rewritten, because the one in the file is now about the wrong reader.** Shipped text: the classification may run after the lock "because registration completes before any archetype can read the kind". Under R2 the reader is `tag_by_name`/`dynamic_by_name`, which runs *during* other threads' registrations. Replacement, and it is a proof rather than a scope: **the name insert is the publication point, and it is the last thing under the lock.** A reader reaches an id only through `DYN_NAMES`, behind the same `Mutex`; the writer's `STORAGE_KIND` store therefore happens-before the reader's map read through the mutex's release/acquire, so a `Relaxed` store and a `Relaxed` load are sufficient and the kind a reader sees is final. The id returned to the *minting* thread is ordered by program order on that thread, which covers `EnableTagId::try_from_component_id` on a freshly returned id.
>
> This deletes the reclassification window as a *mechanism*, not as a discipline: there is no interval in which an interned name maps to an unclassified id.

**Gate (added to 32.6's list as row 8):** two threads, one name, one `register_enable_tag` and one `tag_by_name` in a loop; `tag_by_name` must return `None` on every iteration (never a `TagId` for a bitset id). RED: move `set_storage_kind` back outside the lock — the race is then observable with a loop bound, and the test records the observed count in its header per the tree's convention.

---

## P41 (closes pass-5 W2) — C1's Miri gate derives its handle list, and its floor is re-derived from free sites

**Depends on / re-read:** P35 (the derived heap-client set), §2.1's Heap row as amended by P35, §2.3's `SortedMap` declaration (`:239`). ⚠ *Rev 2.5 (P46, P50): P41's gate is not built (U-1; the plan's 03 §3); pass-6 W3 is carried with the revival form (P46.4).*

The critique is right twice: the body names a retired type, and the floor counted the wrong thing. `SortedMap { keys: HeapVec<K>, vals: HeapVec<V> }` (`:239`), so `HeapVec::drop` fires 1 (HeapVec) + 2 (SortedMap) = **3**, while `HeapBox` and `HeapDyn` free through `HeapRef::free` directly and never touch `HeapVec::drop`. "≥ 5, one per handle type" was counting allocations and calling them types.

**Removed** (P1, C1's red-first Miri test, the Body cell, `:757`, verbatim):

> | Body | Build an `EcsMaster`-shaped owner struct `{ heap: Heap, other: u64 }`. `let v = HeapVec::<u32>::with_capacity_in(owner.heap.as_ref());` push 8. Then take `&mut owner` afresh, call `owner.heap.as_ref().alloc(..)` **and** `free(..)` through it (so the sibling tag performs both a read and a write of the class head), then drop `v` — whose `Drop` writes the same class head through the handle minted before the sibling retag. Repeat with `HeapBox`, `HeapDyn`, `HeapString`, `SortedMap`. |

**Added:**

> | Body | Build an `EcsMaster`-shaped owner struct `{ heap: Heap, other: u64 }`. `let v = HeapVec::<u32>::with_capacity_in(owner.heap.as_ref());` push 8. Then take `&mut owner` afresh, call `owner.heap.as_ref().alloc(..)` **and** `free(..)` through it (so the sibling tag performs both a read and a write of the class head), then drop `v` — whose `Drop` writes the same class head through the handle minted before the sibling retag. **Repeat for every handle in §2.1's Heap row, derived, not listed** — the same derivation P35 gave 15.5 and 19.2, for the same reason (a gate that names a retired type is a check that cannot fail). On today's row that is `HeapVec`, `HeapBox`, `HeapDyn`, `SortedMap`. |

**Removed** (P1, the Anti-vacuity cell, `:760`, verbatim):

> | Anti-vacuity | the green run asserts the mutation site is reachable: a counter incremented in `HeapVec::drop`'s free must read ≥ 5 (one per handle type). A run that reports 0 drops is a vacuous pass. |

**Added:**

> | Anti-vacuity | the green run asserts the mutation site is reachable: a counter incremented in **`HeapRef::free`** — the one funnel every handle's `Drop` reaches, unlike `HeapVec::drop`, which `HeapBox` and `HeapDyn` never call — must equal **the number of allocations the body's drops release**, derived at write time as one per handle plus one extra per handle owning more than one allocation. On today's Heap row that is `HeapVec` 1 + `HeapBox` 1 + `HeapDyn` 1 + `SortedMap` **2** (`keys` and `vals`, §2.3 `:239`) = **5**. The old cell reached the same 5 by counting handle types, which is a different quantity that happened to agree; if the Heap row changes, the derivation moves and the constant follows it. A run reporting 0 is a vacuous pass. |

---

## P42 (closes pass-5 W3) — four superseded passages replaced

**Depends on / re-read:** P1 (the `HeapRef` receiver, pass-1 C1), P31/3 (the closed `hv3-owner` vocabulary), P34 (the Frame class deleted), P35 (`HeapString` retired), P30/O1 ("replace, don't annotate").

**(1) Removed** (§5.1's consumer table, `:394`, verbatim):

> | `boyko_utils::SparseMap::new_in(&mut Heap)` | archetype registry, any external user |

**Added:**

> | ⚠ *Rev 2.5 (P46.3): `SparseMap` moves into `boyko_ecs` on `VmColumn`s (KC-18, rung ~~D-R2d~~ D-M1); no `HeapRef`.* ⚠ *Rev 2.6 (P54/O3): D-M1 moves `SparseMap` (01 KC-16; 02 `:127`); D-R2d only retires its KF-02 row.* `boyko_utils::SparseMap::new_in(HeapRef)` — **`&mut Heap` is not a receiver anywhere in this design** (pass-1 C1; the bookkeeping lives inside the reservation, P1) | the archetype registry, which is an `hv3-owner` (`registry`). **Any other `SparseMap` user cannot become a heap row at all and takes `VmColumn` instead** (P31/3) |

*Why this row mattered more than the other three: rev 2.1 added mutation M2 because "a developer picks the exit that compiles", and this table is where a developer reads the API surface.*

**(2) Removed** (P11's TB-3 bullet, `:1171`, verbatim):

> >   - **TB-3 — no `Deref` from an arena handle that outlives the frame/scope.** `FrameVec<'f, T>` borrows the arena (and `reset(&mut self)` makes an outliving handle a compile error, P4); `Scope` cells are consumed at execution.

**Added:**

> >   - **TB-3 — no `Deref` from an arena handle that outlives the scope.** `Scope` cells are consumed at execution. *(The `FrameVec`/P4 half is deleted with the Frame class, P34; the rule keeps its remaining subject rather than being retired, because `ScopeBlock` is still an arena.)*

**(3) Removed** (§5.2's determinism table, `:405`, verbatim):

> | 2 | Frame arena | dispatcher-only; `debug_assert!(owner == current)` under `cfg(debug_assertions)` |

**Added:** *(row deleted — the obligation was owed by a class P34 deleted)*

**(4) Removed** (§1, "What (i) cannot do", `:112`, verbatim):

> - **`String` ergonomics** — `format!` is gone from engine crates; `core::fmt::Write` into a `HeapString`/`LogRing` replaces it.

**Added:**

> - **`String` ergonomics** — `format!` is gone from engine crates; `core::fmt::Write` into a **per-lane `ByteColumn`** (the log ring's own column) replaces it, per P35's rung-4 table. `HeapString` does not ship: no rung-4 owner is one of P31/3's four `hv3-owner` values.

---

## P43 (closes pass-5 W4, and pass-5 question 3) — the id cost of the new destinations, and the rule that bounds it

**Depends on / re-read:** P34's destinations table (`:3137-3143`), P37/3, P39/39.4 (no band), P26's `MAX_MOD_TYPES` self-restraint.

### 43.1 A measurement that shrinks the bill: the id is a LAYOUT TOKEN, not an identity

I read `ScratchColumn` rather than assuming the per-instance cost. `ScratchColumn::new(component_id, reserve_rows)` uses the id for exactly two things: `ComponentPool::new_untracked` resolves the **layout** from it (`scratch_column.rs:88-104`; `component_pool.rs:279-288`), and `pool_base_stagger(component_id)` derives the pool's leading in-reservation offset (`component_pool.rs:322-335`). The column is a **private field**, never handed out (`scratch_column.rs:43-48, :59-68`), and a scratch column is in no archetype signature. Therefore:

> **Rule (new, stated in P34's destinations table): one registered `ComponentId` per distinct scratch ELEMENT TYPE, not per `ScratchColumn` instance.** Twenty systems each holding a `ScratchColumn<Entity>` cost **one** id between them. ⚠ *Rev 2.5 (P50, pass-6 O2): the rule is superseded by U-2 — scratch is registry-free, zero ids; the layout-token finding above stands as the fact U-2 rests on (plan 01 R-B).*
>
> **The one exception is a cache fact, not a correctness fact:** two columns of the same element type that are iterated **in the same loop** should take distinct ids, because `pool_base_stagger` is a function of the id, so sharing it gives them the same leading offset and defeats the P2-CACHE-FIX that exists to keep co-touched columns off the same L1 sets. Necessary, not sufficient — the stagger is periodic, so two distinct ids can still collide; the rule buys the easy half. **Overturn gate:** if a rebuild pass shows L1d conflict misses between two same-typed co-iterated columns, split the id (or re-derive the stagger period), and record the measurement. ⚠ *Rev 2.5 (P50, U-2): the stagger is now a constructor argument taken from one process-global round-robin seed (KC-10); MQ-16 measures co-iterated same-layout columns.*

### 43.2 The projected delta, beside M-A13's measured base

**Removed** (implicitly carried by 32.4's deleted M-A13; restated here as the surviving entry):

**Added:**

> **Measurement entry (specified, not run): M-A13** — the id-space census at steady state for `boyko_demo` and the playground scene, taken through `id_space_census()` (P39): `NEXT_ID` high-water, occupied slots, and their partition into static / dynamic-tag / dynamic-sized(asset) / pinned-out-of-band. ⚠ *Rev 2.5 (P50): M-A13 is AL:M-A13 / MD:M-K3 (plan 03 §5, structural); under U-2 the scratch rows of the table below are 0 ids and the physics band is deleted, and `MAX_MOD_TYPES` is replaced by MS-03's D2 bound (05 §4).*
>
> **M-A13 measures a tree that has not yet spent what this design commits it to spending, so the cap is checked against base + projected delta, never against the base alone.** The projected delta, by the rule in 43.1:
>
> | destination | distinct element types | ids |
> |---|---|---|
> | `TraversalScratch` (P37/3) | `u64`, `(Entity, u32)` | ~~2~~ 0 ⚠ *Rev 2.5 (U-2)* |
> | rung 2's CSR two-pass (hierarchy rebuild) | `u32` offsets, `Entity` flat | 2 (shared with any other CSR of the same two types) |
> | UI frame lanes as CSR pairs (P34, 5 sites) | reuses `u32` / `Entity` unless a lane's element is a struct | 0-2 beyond the above |
> | KF-44's 21 exclusive-system `Local` rows (`RUNTIME-DATA-LEDGER.md:820`) | **derived by the rung from the ledger's per-row element types**, not guessed here | ≤ the number of distinct element types among them |
>
> **Headroom arithmetic, labelled as a projection:** 512 total − 128 physics band (physics builds only, `scratch_ids.rs:685`) − ~142 derive sites (the physics census's own over-count, `:671-684`) − assets and dynamic tags (M-A13's job) − the delta above. `MAX_MOD_TYPES = 128` stays a **self-restraint inside `boyko_modding`** until M-A13 exists; 39.2 removed the mechanism that would have made the budget a runtime quantity, so the arithmetic is now over constants and one census. ⚠ *Rev 2.5 (P50): superseded arithmetic — the band is deleted (U-2) and the bound is MS-03's D2 (05 §4).*

### 43.3 Rung-1 entry criterion, both directions (question 3)

**Added** (rung 1's entry criteria):

> **Before rung 1 starts: re-read the ledger's revision line and re-derive two things against it** — (i) whether any row's destination is a frame arena, which is P34's revival gate; (ii) the id delta of 43.2, because a later revision may *move* rows onto `ScratchColumn` and change the arithmetic in the other direction. One line, both directions, checked once.

---

## P44 (closes pass-5 O1, O2)

### O1 — K-MOD-6's claim is scoped to the seam, because the general form is false in the tree

Accepted, and the counterexample verified: `register_asset_layout::<T>(drop_fn)` mints through `try_register_dynamic` with a **real** `TypeId`, a non-zero size and a caller-supplied `drop_fn` (`.../asset/backing.rs:114-135`). A developer implementing "never for a dynamic id" as an assert breaks the asset system.

**Removed** (P33/33.2's K-MOD-6 row, `:3107`, the clause only):

> `drop_fn` (`component_registry/mod.rs:83, :117`) is **never written for a dynamic id**: the seam asserts `drop_fn: None` (P26 constraint 1), so no code pointer into a mod image is ever stored in `LAYOUTS` and there is nothing to dangle after unload.

**Added:**

> `drop_fn` (`component_registry/mod.rs:83, :117`) is **never written for an id minted THROUGH THE SEAM**: `try_register_dynamic_by_name` asserts `drop_fn: None` (P26 constraint 1), so no code pointer into a mod image is ever stored in `LAYOUTS` and there is nothing to dangle after unload. The scope is load-bearing — the kernel's *own* dynamic mints do carry drop glue (`asset/backing.rs:114-135`, whose `drop_fn` comes from e.g. `TextureGpu::drop_glue`), so the seam's rule is a property of the seam, never of the dynamic id space.

### O2 — binary size gets a check, and it is a DELTA, not an absolute

Accepted. The gap is real: G6 pins symbols, `size_of`s, startup numbers and the inventory, while "no binary size" rests on "no caller ⇒ dropped at link", which nothing measures — and P36/O2 deliberately reads `.text` from the **rlib's object members**, where an unused function is still present.

**Added** (a new row in the G6 table, after (e)): ⚠ *Rev 2.5 (P47, pass-6 W1): the row below is relabelled (g) and is answered by UG-15 leg (7).*

> | ~~(f)~~ (g) **no binary-size delta** (NEW) ⚠ *Rev 2.5 (P47, pass-6 W1): relabelled; both arms link the same kernel, so this row cannot see a kernel-side survivor — UG-15 leg (7) does* | `llvm-size` on the **linked** `boyko_demo`, both arms of (b): arm A declares `boyko_modding` as a dependency and never calls it, arm B removes the dependency. `.text`, `.rodata` and total section bytes must be **equal**, delta pinned at **0** | make the binary call one class-A seam function → nonzero; add `#[no_mangle]` to a kernel item → nonzero |

**Why a delta and not the absolute number the critique suggested** — and this is a refusal with a reason, not a dodge: an absolute `.text` pin needs a re-bless on every unrelated kernel edit and on every toolchain bump, which is the churn P18 already manages for ~~five~~ four symbols (⚠ *Rev 2.6, P54/O2: four after P49*) and would now manage for a whole section; a **difference between two arms of the same commit** needs no re-bless ever, is immune to toolchain drift, and tests the exact claim ("dropped at link") rather than a proxy for it. It is also the only form that can be red-first without touching the kernel. The cost is that it cannot detect a size regression shared by both arms — which is (b)'s job, per symbol, and is where that question belongs.

---

## P45 — pass-5 open questions, answered

1. **"Does §7 survive C1 as a seam, or become a consumer of KF-47?"** — **Consumer.** P38. The critic's preferred outcome and the cheaper one; K-MOD-11 is deleted as an entry, the inventory is re-derived against the lane's names with a membership rule, and the `g17` observer asymmetry is recorded as an inherited limit with its file and its gate.
2. **"Which band do P37/3's two ids come from?"** — **None.** P39/39.4: they are ordinary `register_new` mints behind two `OnceLock`s; `register_layout` was only ever needed for *compile-time-constant* ids, which is physics's requirement, not the traversal scratch's. No process-global effect, and the two ids are a line in 43.2.
3. **"Check the ledger both ways before rung 1."** — Accepted verbatim; P43/43.3 writes it into rung 1's entry criteria.
4. **"The error cannot name the pinning site."** — Conceded; the clause is deleted with the mechanism. What replaces it is **more** than it asked for: `id_space_census()` enumerates every out-of-band occupant on a path that has already failed, at zero steady cost (P39/39.2).

**Still provisional after this rev, named so it is not mistaken for settled:** `MAX_MOD_TYPES = 128` remains a self-restraint until M-A13 runs with 43.2's projected delta applied. Rev 2.3 called this out and it is not closed by this patch — it is closed by a measurement, and the machine belongs to the owner.

---

## Change log (rev 2.3 → rev 2.4)

| Row | Disposition | Where |
|---|---|---|
| **pass-5 C1** | **CLOSED by consuming KF-47 and deleting the duplicate.** `add_component_raw` is removed from §7; attach/detach/mark are the reflect lane's shipped `add_component_by_id` / `remove_component_by_id` / `mark_component_changed` plus `EnableTagId::try_from_component_id`, marked `MOD-SEAM`. Four properties of the shipped pair that the duplicate lacked are cited (`#[require]`, `AlreadyPresent`, five release-active arms, the dense arm). G6(e) is re-derived with a **membership rule** and becomes **13 items** while the kernel grows by **zero functions**. R2's enable-tag row deleted (the handle exists). Merge ordering + the anchor re-bless obligation stated for R1's edits in `tags.rs` and the four marker lines in `seam_by_id.rs`. The bytes-are-MOVED hazard is answered in `boyko_modding`, not in the kernel. | P38 |
| **pass-5 C2** | **CLOSED by deleting `DYN_ID_CEILING` entirely.** `register_layout` keeps its per-slot contract and lowers nothing (measured: **390 `register_layout::<` sites across 82 files**, ≥20 of them inside `boyko_ecs/src`'s own `#[cfg(test)]` modules, one process). The mint **skips an occupied slot** instead of panicking: zero added instructions on the success path, zero `.bss`, zero ids lost, `dynamic_slot_occupied_panic` deleted. The pin-side direction is explicitly **diagnosed, not fixed**, correcting Rev 2.3's one-directional "fail-closed" claim. Four gate rows, two of which must stay **GREEN** and fail under the rejected semantics. `id_space_census()` replaces the unsatisfiable "name the pinning site" clause. | P39 |
| **pass-5 W1** | **CLOSED** — `set_storage_kind` moves inside `intern_or_mint`'s lock, kind becomes a mint parameter, the shipped mint-then-reclassify at `tags.rs:134-141` is deleted, and the ordering argument is **rewritten** (name-insert is publication; the mutex supplies the happens-before) because the shipped one is about the wrong reader. New two-thread gate row 8. | P40 |
| **pass-5 W2** | **CLOSED** — C1's Miri gate derives its handle list from §2.1's Heap row, and the anti-vacuity counter moves to `HeapRef::free` with the floor **re-derived from free sites** (1+1+1+2 = 5); the old "one per handle type" was counting a different quantity that happened to agree. | P41 |
| **pass-5 W3** | **CLOSED** — four replacements: §5.1's `&mut Heap` `SparseMap` row (the receiver pass-1 C1 killed) → `HeapRef` + P31/3's destination rule; TB-3 loses its deleted half; §5.2's Frame determinism row deleted; §1's `HeapString` destination → `ByteColumn`. | P42 |
| **pass-5 W4** | **CLOSED with a finding that shrinks the bill** — the scratch id is a **layout token**, not an identity (`scratch_column.rs:88-104`, `component_pool.rs:322-335`), so the cost is one id per distinct element **type**, with a stagger-based exception for same-typed co-iterated columns and its overturn gate. M-A13 restated as base + **projected delta**, with the KF-44 half derived by the rung from the ledger. | P43 |
| **pass-5 O1** | CLOSED — K-MOD-6's claim scoped to the seam; `asset/backing.rs:114-135` cited as the live counterexample that would have broken an implementer. | P44 |
| **pass-5 O2** | CLOSED — G6 gains row **(f)**: linked-binary section sizes, pinned as a **delta of 0** between the two (b) arms rather than as an absolute number, with the reason the delta form is better (no re-bless, toolchain-immune, tests the actual claim) and what it deliberately does not cover. | P44 |
| **pass-5 questions 1-4** | ANSWERED — consumer; no band; ledger checked both ways; the site clause dropped with its mechanism. | P45 |
| rev-2.3 P33/33.1, P34, P35, P36, P37/2, P37/4 | **unchanged** | — |
| rev-2.3 P32 except 32.1's R2 row, 32.3's one sentence, 32.4, 32.5 and two 32.7 rows | **unchanged** — 32.2's shipped-defect finding, R1, R3, R4, R5 and the gate rows 1-7 stand | — |
| rev-2.2 P25, P26 (as already amended), P28, P29, P30, P31 | **unchanged** | — |
| rev-2.1 P0, P3, P6, P7, P9, P11 (except TB-3's half), P15, P16, P17, P18, P20, P21, P22, P23 | **unchanged** | — |

## Open questions for pass 6

1. **§7's rung is now blocked on another lane's merge** (P38/38.4). That is the correct dependency — the alternative is the duplicate this round deleted — but it is a schedule fact the owner may want to rule on, because it makes the modding rung's start date the reflect lane's merge date.
2. **G6(f)'s delta pin assumes arm A's unused dependency is actually dropped by the linker.** If it is not — e.g. a future `boyko_modding` gains a `#[used]` static or a `ctor` — the gate goes red for a *correct* reason and the design owes a ruling on whether such a construct is allowed in the modding crate at all. I judge it should be forbidden, but the gate will find out first. ⚠ *Rev 2.7 (P56.1, AP8 N-W1): the ruling asked for here is scoped to runtime-invoked code, meaning an entry in a runtime-walked table or the `DllMain` entry name. A `#[used]` static outside those tables, such as MS-15's `.boykom$m` registration static, is not forbidden.* ⚠ *Rev 2.7 closure (P58.1, AP9 W2): R2 is not one name. It is the entry mechanism, meaning the image entry symbols and the runtime's user hooks in table E.*
3. **The `g17` observer asymmetry is now a documented mod-visible limit** (P38/38.1). It is filed and RED-by-design in the lane. If the lane repairs it before §7's rung, the inherited-limit paragraph must be deleted rather than left standing — the same stale-passage class W3 keeps finding, flagged in advance.

**Files read for this patch** (all read-only). Documents in `D:/claude/BoykoEngine`: `docs/memory/ALLOCATOR-DESIGN-SPACE.md`, `docs/memory/RUNTIME-DATA-LEDGER.md`. Code in `D:/wt/joltab`: `crates/boyko_ecs/src/ecs/core/component/component_registry/{mod.rs, tags.rs}`, `crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs`, `crates/boyko_ecs/src/ecs/core/asset/backing.rs`, `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`, `crates/boyko_ecs/src/ecs/memory/component_pool.rs`, `crates/boyko_physics/src/scratch_ids.rs`, plus workspace-wide counts of `register_layout::<` and `try_register_dynamic(`. Code in `D:/wt/reflect`: `crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs`, `crates/boyko_ecs/src/ecs/core/component/component_registry/{tags.rs, required.rs}`.

## Critique pass 6 log (final)

Verdict: CHANGES_REQUESTED, no Critical remark. Closed by orchestrator ruling; the remarks below are OPEN. ⚠ *Rev 2.5 (P50): every remark now has a disposition, recorded at its own line and in P50.*

- [IMPORTANT W1 - no critical findings this pass; important findings also withhold APPROVED] The new G6 binary-size row (P44, ALLOCATOR-DESIGN-SPACE.md:3823) compares two builds that link the same kernel. P18's own row (f) (:1902) says that two-arm build cannot see anything the kernel carries in both arms. So it cannot observe what P38.5 (:3611) says it now measures: whether the four class-A kernel functions are dropped at link. Its `#[no_mangle]` canary cannot fire, and the table now has two rows labelled (f). → *Rev 2.5 (P50): answered by UG-15 leg (7); the duplicate (f) is relabelled (g); leg (6) keeps only feature unification (P47).*
- [IMPORTANT W2] The registry gate rows added by P39/P40 are written as if the registry were per-test. It is process-global, write-once and capped at 512 ids. (a) G-MINT-2 and G-MINT-4 pin the same id 100 in one binary. (b) G-MINT-1 asserts an exact successor while G-MINT-2's 200 mints run on another thread, and it can hit the pin-side panic. (c) G-MINT-3 is ignored under class `solo`, so it runs in no gate. (d) P40 row 8 races only on its first iteration, so its red-first check is unlikely to fire. → *Rev 2.5 (P50): D-S1(i)'s isolated tests (the plan's 02 §2 red-first list; UG-19).*
- [IMPORTANT W3] P41's anti-vacuity counter lives in `HeapRef::free` and must equal 5. The test body itself calls `HeapRef::free` once per handle repetition, so a correct implementation reads at least 9 and the gate fails on it. → *Rev 2.5 (P50): void under U-1 — P41's gate is not built; carried with the revival form (P46.4).*
- [IMPORTANT W4] P38.3's membership rule (reachable by a mod AND erased) is not what G6(e) checks; G6(e) only compares kernel marker lines against a list. The shipped `pub register_hooks_by_id` (joltab component_registry/mod.rs:864-906) fits the rule, is not in the 13-item list, and is not on K-MOD-7's not-exposed list. It writes process-global hook function pointers. The only thing forbidding that for mod ids is P26/3's prose, and a forwarded call leaves stale pointers that fire after a reload. → *Rev 2.5 (P50): the plan's 05 §5, row AP6-W4.*
- [IMPORTANT W5] The runtime data ledger (rev 4 on disk) still specifies rungs 1a, 1b and 1d with rev-1 mechanisms: KF-33 is a per-thread arena with a new TLS pointer (RUNTIME-DATA-LEDGER.md:1446-1457), KF-34 replaces the per-lane rings (:1459-1468), and the order of work puts these rows at rung 5 (:1783). P2, P10 and P16/P17 replaced all three and P14 schedules them at rung 1. P43.3's entry check does not cover this, so pass-1 C2's closure has not reached the document that assigns the work. → *Rev 2.5 (P50): B1 (ledger rev 5; the plan's 02 §2).*

# Architecture review: allocator design space, Rev 2.4 (pass 6)

## Verdict

```
[ ] APPROVED
[X] CHANGES REQUESTED - 0 critical, 5 important, 2 optional
```

There are no critical findings. Rev 2.4 closes both pass-5 blockers the right way, by deletion and by consuming a shipped capability, and I found no new soundness defect in the kernel mechanisms. The five important findings are:
- three gate specifications that cannot behave as written (W1, W2, W3);
- one rule/gate mismatch on the modding seam, with a plausible dangling-code-pointer consequence (W4);
- one document divergence that leaves pass-1 C2's closure unpropagated to the work order (W5).

Under this role's rules, important findings also withhold APPROVED.

**Where things were read:**
- **Design:** `D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md`, all 3865 lines (Parts I-VII and Rev 2.4).
- **Code:** read-only in `D:/wt/joltab` (merge/ke16-into-ecsnative) unless a citation says `D:/wt/reflect`.
- **Ledger:** `D:/claude/BoykoEngine/docs/memory/RUNTIME-DATA-LEDGER.md` is **rev 4** on disk (`:1`, `:13`, dated 2026-09-16).
  - Its totals line (`:30`) now reads **691** rows (29.3 %) in an ECS form, not the 690 the Rev 2 tree note quotes (`ALLOCATOR-DESIGN-SPACE.md:533`). That is a recheck repair, and no design claim depends on it.

**Checked against the code, and it holds:**
- **P38's four properties of the reflect lane's by-id seam:**
  - the `AddOutcome` refusal channel: `D:/wt/reflect/crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs:88-131`
  - `#[require]` honoured: `:166-181`
  - five release-active arms: `:206-250`
  - a dense arm: `:252-273`
  - the bytes-are-moved warning: `:156-164`
  - the by-id detach observer gap: `:486-500`
  - `EnableTagId::try_from_component_id`: reflect `tags.rs:225-234`
  - the waived-anchor warning: reflect `tags.rs:209-212`
- **`REQUIRES_DIRECT` is filled only by the derive path:** reflect `required.rs:128-134`.
- **The `STORAGE_KIND` default is `Table`:** joltab `component_registry/mod.rs:373-376, 396-410`.
- **P39's replaced code:**
  - `register_new`: `mod.rs:920-947`
  - `try_register_dynamic`: `:967-992`
  - `dynamic_slot_occupied_panic`: `:996-1010`
  - `register_layout`'s per-slot contract: `:1012-1049`
- **P40's shipped mint-then-reclassify and its archetype-scoped ordering argument:** joltab `tags.rs:122-141`.
- **P43's layout-token finding:** `scratch_column.rs:43-48, 88-104`; `component_pool.rs:284-288, 322-329`.
- **P44/O1's counterexample:** `asset/backing.rs:115-134`.

---

## Remarks

### Critical

None.

### Important

#### W1. G6's new binary-size row compares two builds that link the same kernel, so it cannot see what P38.5 says it now measures

**Where** ⚠ **Disposition (rev 2.5, P50):** answered by UG-15 leg (7); the row is relabelled (g) (P47.4); leg (6) keeps feature unification only.
- P44/O2's new row "(f) no binary-size delta" (`:3823`).
- P38.5's cost row (`:3611`): class A is "no caller => dropped at link, now **measured** by G6(f)'s `.text` delta rather than asserted".
- P18's existing row (f) (`:1902`).

**Problem**
- **The arms are the same build P18 already has.** Arm A "declares `boyko_modding` as a dependency and never calls it"; arm B removes the dependency. P18 kept exactly this build as its own row (f), and says of it: "It **cannot** see a seam the kernel carries in both arms".
- **The class-A items are kernel items.** `ComponentLayout::new_dynamic`, `try_register_dynamic_by_name`, `dynamic_by_name` and `EcsMaster::remove_component_type` all live in `boyko_ecs`, so both arms contain them. Both arms drop them or both keep them; the delta is 0 either way.
- **The canaries cannot fire as written.**
  - "Add `#[no_mangle]` to a kernel item" changes both arms identically, so the delta stays 0.
  - "Make the binary call one class-A seam function" fires only if the call exists in arm A alone. The row names no mechanism that would make the call arm-specific.
- **Arm A links nothing from the unused crate.** rustc loads an `--extern` crate only when code names it; that is the premise of the `unused_crate_dependencies` lint. So a `boyko_demo` that never calls `boyko_modding` gets nothing from it. The only thing arm A can differ in is cargo feature unification, which is P18(f)'s subject. The `#[used]`/ctor case in your question 2 is invisible for the same reason.
- **The G6 table now has two rows labelled (f).**

**Consequence**
The "binary size" dimension of the owner's hard requirement is recorded as measured, and it is not. Suppose a later change keeps a class-A function alive in a no-modding binary, for example through a `static` function-pointer table or `#[used]`:
- The linked no-modding `boyko_demo` grows, and (f) stays green because both arms grow.
- G6(a) does not catch it: it matches only `boyko_mod_*` / `^mod_api_`.
- G6(b) does not catch it: it reads rlib object members (P36/O2), where the function exists either way.

This is pass-2 C4's defect class ("a proof that compares a build to itself"), back for one dimension. It is also worse than having no check: a check believed to cover more than it does is the P11 "a flag believed to protect you is worse than no flag" lesson.

**Confidence:** CONFIRMED (plan text against plan text: `:1902` vs `:3611` and `:3823`). The rustc lazy-load point is supporting, not load-bearing.

**What is needed**
- A check whose two sides differ in exactly the property claimed. Two directions, both without re-bless churn:
  - assert that the class-A mangled names are **absent** from the linked no-modding binary, using the `llvm-tools` component that (a) already requires, with "tool absent" counted as red; or
  - compare against the pre-seam parent build, as (b)-(d) do.
- Relabel one of the two (f) rows.
- Give each canary a mechanism that lets it fire.

#### W2. The registry gate rows are written as if the registry were per-test

**Where:** P39/39.3 G-MINT-1..4 (`:3663-3668`); P40 gate row 8 (`:3698`). ⚠ **Disposition (rev 2.5, P50):** D-S1(i)'s isolated tests (plan 02 §2).

**Facts the rows run into**
- `LAYOUTS` is a process-global, write-once table.
- `register_layout` panics when an occupied slot holds a different type (`mod.rs:1039-1049`).
- `NEXT_ID` is process-global (`mod.rs:214`).
- Interned names are never removed (`tags.rs:155, 182-197`).
- libtest runs a binary's tests on parallel threads by default.

**Problem**
- **(a) G-MINT-2 and G-MINT-4 pin the same id.** Both live in `tests/registry_id_mint.rs` and both pin id 100. If the types differ, whichever test runs second hits the different-type panic. Then either G-MINT-4's expected third-call panic is satisfied by its *first* call (green for the wrong reason), or G-MINT-2 fails.
- **(b) G-MINT-1 races.** It pins `k+1` and asserts the exact successor `k+2`, while G-MINT-2's 200 mints run on another thread.
  - `NEXT_ID` moves under it, so the exact assertion is flaky.
  - A concurrent mint can take `k+1` before the pin. The pin then panics — the "pin lands on a slot the counter already took" direction that P39 says is diagnosed, not fixed.
- **(c) G-MINT-3 would run nowhere.**
  - It is the only test in its own binary, so it needs no `--test-threads=1`. Marking it `#[ignore = "solo: ..."]` only removes it from `cargo test --workspace`.
  - The device-free ignored leg in CLAUDE.md is a fixed one-test invocation (`-p boyko-log --test l14_sink_policy`), so it would not pick this test up either.
  - That leaves the one row proving "no panic, no endless loop at the ceiling" in no gate.
  - Its red-first note also mis-predicts: with the `None` return deleted, the mint indexes `LAYOUTS[512]` and panics out of bounds rather than spinning.
- **(d) P40 row 8 races only once.** It loops over one name, but that name is interned once for the whole process, so only iteration 1 can race. Using a fresh name per iteration spends one of at most 512 ids. The mutation it must catch (store after unlock) opens a window of a few instructions, once per name. The red is therefore unlikely ever to be observed — a canary that does not fire, on the gate that closes pass-5 W1.

**Consequence:** the rows that close pass-5 C2 and W1 are either flaky, red on a correct implementation, or never run. The pressure that follows is to loosen assertions at the keyboard.

**Confidence:** CONFIRMED.

**What is needed**
- A distinct pinned id per case.
- Relative assertions ("`Ok`, and not `k+1`") instead of an exact successor.
- Stated isolation: one binary per stateful case, or an explicit `--test-threads=1` in the gate command.
- G-MINT-3 not ignored.
- For row 8, a deterministic interleaving: a `cfg(test)` rendezvous between unlock and store, or a loom model of the `intern_or_mint` body.

#### W3. P41's anti-vacuity equality fails on a correct implementation

**Where:** P41's Anti-vacuity cell (`:3722`). ⚠ **Disposition (rev 2.5, P50):** void under U-1; carried with the Heap's revival form (P46.4).

**Problem**
- The counter moved to `HeapRef::free` and must **equal** 5.
- The body's own sibling operation, `owner.heap.as_ref().free(..)` (Body cell, `:3714`), is `HeapRef::free` too (P1 API, `:736`). The body repeats once per handle (four handles).
- So a correct implementation reads 5 + 4 = **9**. It reads more if `push 8` or a `SortedMap` insert grows through `HeapRef::grow`, whose documented slow path is alloc + memcpy + free (`:737-738`).
- The derivation itself is right ("count allocations released, not handle types"); only the counter's location is wrong for it.

**Consequence:** the first artifact of P14 step 3 is red on the correct shape, and its constant gets re-derived at the keyboard. That is exactly the consequence pass-5 W2 was raised for, now in its replacement.

**Confidence:** CONFIRMED (plan arithmetic).

**What is needed:** one of:
- count at the handles' `Drop` sites, which is the quantity the derivation describes;
- derive the constant including the body's own frees and any grow frees, and say which;
- use a lower bound instead of an equality.

#### W4. P38.3's membership rule is not what G6(e) checks, and a shipped hook-registration path makes the gap dangerous

**Where** ⚠ **Disposition (rev 2.5, P50):** plan 05 §5, row AP6-W4.
- P38.3's rule and 13-item list (`:3590`).
- P26 constraint 3 (`:2448`): "HOOKS[id] is never written for a mod id".
- K-MOD-7's not-exposed list (`:1207`, as amended by P34).

**Problem.** The rule is "reachable by a mod AND erased (takes a runtime `Layout`, a `ComponentId`, or raw bytes)". The gate is "the set of kernel function items carrying the `MOD-SEAM` marker equals the list". These are different sets.

1. **Reachability is decided outside the kernel.** Under K-MOD-9, every mod-facing thunk lives in `boyko_modding`, so "a mod can reach it" is a property of what `boyko_modding` forwards. A kernel-side marker scan cannot evaluate that. An erased kernel function forwarded *without* a marker is invisible to the gate — the seam grows silently, which is the case (e) exists to catch. P38.3's claim that the list is now "derivable from the rule" therefore does not hold.
2. **The rule admits shipped `pub` items the 13 omit:**
   - `EcsMaster::get_component_raw`, `get_component_changed_tick` and `set_component_raw` (`component_api.rs:176, 337, 461`)
   - `get_layout` (`mod.rs:1125`)
   - `storage_kind` (`mod.rs:390`)
   - `register_hooks_by_id` (`mod.rs:887`)
3. **`register_hooks_by_id` is the dangerous one.**
   - It is `pub` in a `pub` module (`component/mod.rs:5`).
   - It is documented as "the single entry point into the write-once `HOOKS` table for ids that have no Rust type to name" (`mod.rs:864-868`), and as what a dynamic tag "needs downstream" (`tags.rs:37, 54`).
   - It writes process-global function pointers.
   - P26/3's only guard is prose: "The seam entry does not touch the hook table".

**Consequence.** If `boyko_modding` forwards it — the natural API for a mod that wants `on_add`:
1. After unload, `HOOKS[id]` still holds pointers into the unmapped mod image.
2. On reload, the same name resolves to the same id (P32 R1/R3).
3. `register_hooks_by_id` refuses the new hooks as `AlreadyArchetyped` / `AlreadyRegistered` (`mod.rs:897-904`).
4. The stale `on_add` fires on the reloaded mod's first attach: a call into unmapped memory.

P32/32.6 row 4 inspects `HOOKS[id]` only for whatever its own test mod does, so it would not catch this.

**Confidence:** CONFIRMED on the rule/gate mismatch and on the function's shape and visibility. PLAUSIBLE on forwarding, since `boyko_modding` does not exist yet.

**What is needed**
- Evaluate reachability where it lives. For example, (e) asserts that the set of kernel paths `boyko_modding` references equals the marked inventory — the same shape as O6's `raw` allowlist.
- Put `register_hooks_by_id` (and the typed hooks builder that delegates to it, `hooks/builder.rs:149`) on K-MOD-7's not-exposed list, enforced by that check.
- Or, alternatively, state the rule as a curation rule for the listed items only, and drop the "derivable" claim.

#### W5. The work order still specifies rung 1 with the mechanisms pass 1 blocked and Rev 2 deleted

**Where:** rungs 1a/1b/1d (P2, P10, P16, P17) and P43.3's entry criterion (`:3799`), against ledger rev 4. ⚠ **Disposition (rev 2.5, P50):** B1.

**Problem.** The ledger, which assigns the work, still describes the rows these rungs touch with rev-1 mechanisms, and its evidence cites rev-1 lines:
- **KF-33** (`RUNTIME-DATA-LEDGER.md:1446-1457`):
  - Mechanism: "`ScopeBlock` backed by one VmReservation per THREAD identity, with a mark at scope entry and a rewind at the join ... Thread identity comes from a const-initialised, Drop-free TLS pointer".
  - Plan line: "ALLOCATOR-DESIGN-SPACE ScopeArena, with the C2 identity fix".
  - Evidence: `ALLOCATOR-DESIGN-SPACE.md:132, :345`.
  - Rows: `block.rs:482` and `thread_pool.rs:278,328`.
- **KF-34** (`:1459-1468`):
  - It calls in-house per-lane Chase-Lev rings "the only route" and cites `:347`.
  - `ledger/pool-utils-log.md:18` repeats that.
  - P10 withdrew the per-lane half, and rung 1f pre-registers the epoch `Local` at boot instead.
- **Order of work** (`:1783`): threadpool and scope-arena rows go to rung 5. The design schedules them at rung 1 (P14).

On the design side:
- P2 deleted that arena and refused a new `thread_local!` because of its measured windows-gnu cost (checkpoint defect 7).
- The P3 protector gate is keyed on `free_all`'s poison write, not on a rewind.
- P43.3's entry check re-reads the ledger only for frame-arena rows and the id delta.

**Consequence.** The first rung P14 schedules has two contradictory specifications, and pass-1 C2's closure (the arena deleted) has not reached the document that hands out the work. A developer following the ledger would:
- build the arena that pass 1 blocked and P2 deleted;
- add the TLS static P2 refused;
- land it four rungs later than the design says.

**Confidence:** CONFIRMED.

**What is needed**
- Extend P43.3's entry criterion to KF-33, KF-34 and the rung number.
- State which document is authoritative for *mechanism* (the design) and which for *row inventory* (the ledger).
- Hand the three corrections to the ledger's next revision. The architect cannot write the ledger, but the design can make the mismatch a checked entry criterion rather than a silent divergence.

### Optional

#### O1. Stale passages the replace-don't-annotate rule still misses

- **P17's current loom bullet (`:1870`)** still says "`Heap` and `FrameArena` genuinely have no cross-thread protocol". P34 site 10 edited the text P17 had *already removed* (`:1866`), so the live sentence still names a deleted class. → *fixed in place, rev 2.5 P51 (the sentence is at `:1877` on this tree)*
- **P18(b)'s canary** says "any of the four"; P29 pins five symbols. → *fixed in place, rev 2.5 P51*
- **P37/4** says "`VmColumn<T: Copy>` carries ... `ever_truncated`", but P5 made the struct bound `ZeroInit`. → *fixed in place, rev 2.5 P51*
- **K-MOD-7 (`:1207`)** still says `ChunkCache`. P16's blanket rename covers it, but a reader of that row alone will not know. → *fixed in place, rev 2.5 P51 (the row is at `:1214` on this tree)*
- **The G6 table has two rows labelled (f)** (see W1). → *relabelled (g), rev 2.5 P47.4*

#### O2. The one-id-per-element-type rule has a shipped mechanism that the design's own client does not use

- **The rule has a shipped mechanism.** P43.1's rule is already practised in the tree: `register_asset_layout::<T>(None)` memoizes one id per element type, and `mesh_draw.rs:418-434` uses it that way. ⚠ **Disposition (rev 2.5, P50):** void under U-2 — scratch is registry-free, so `TraversalScratch` uses `for_type` and no id.
- **The design's own client bypasses it.** P39.4 prescribes `register_new` behind two file-local `OnceLock`s for `TraversalScratch`. Those cannot share an id with that memo. So the design's first named client breaks its own rule, and 43.2's "shared with any other CSR of the same two types" cannot happen. Name the memo as the mechanism.
- **The stagger exception may already be violated in shipped code.** Render shares one `u32` id across `counts`, `offsets` and `cursors` (`mesh_draw.rs:446-448`), which are likely iterated together. That makes them the natural first subject of 43.1's overturn gate.

---

## Status of earlier findings

| Round | Row | Status |
|---|---|---|
| pass 5 | C1 (duplicate by-id attach vs KF-47) | Closed (P38). Consume, don't duplicate; the four properties verified at the reflect sites. The inventory *rule* it introduced is W4. |
| pass 5 | C2 (`DYN_ID_CEILING`) | Mechanism closed by deletion (P39). The skip sits in the branch that panics today (`mod.rs:931-944, 988-990`), so the success path costs nothing. Physics pins run in owner constructors (`scratch_ids.rs:31-38`), so no new collision direction appears. Its gate rows are W2. |
| pass 5 | W1 (kind classified after the lock) | Mechanism closed (P40): the name insert is the publication point, inside the lock. Its gate row 8 is W2(d). |
| pass 5 | W2 (C1 Miri gate names a retired type) | Handle list now derived; the counter is W3. |
| pass 5 | W3 (four stale passages) | Closed; a fifth remains (O1). |
| pass 5 | W4 (uncounted scratch ids) | Closed (P43), with O2's mechanism note. |
| pass 5 | O1 (K-MOD-6 scope) | Closed (P44). |
| pass 5 | O2 (binary size unchecked) | Replaced by a check that cannot see its subject (W1). |
| pass 5 | Questions 1-4 | Answered (P45). |
| passes 2-4 | all rows | Stand closed; Rev 2.4 reopens none. |
| pass 1 | C1 (heap back-pointer aliasing) | Shape (P1/P15) still holds; only its gate's counter is W3. |
| pass 1 | C2 (`wid`-keyed scope slots collide) | Closed in the design (P2/P16/P17); **not propagated to the ledger** (W5). |
| pass 1 | W1-W5, O1-O9, Q1-Q6 | Stand closed. W1's poison-write gate (P3) is unchanged, and P16's `give` link write after the poison stays consistent with TB-1's free-list exemption. |

## Topics walked with no finding

- **Tree Borrows and Miri.**
  - Rev 2.4 adds no `unsafe`, no aliasing argument and no new atomic; P39 *removes* one.
  - The `HeapRef` reservation-base provenance (P1/P15), the `SlotChunks` claim protocol and its Acquire/Release edge (P17), and the poison-write protector gate (P3) are unchanged.
  - The only Miri-relevant delta is P41's counter (W3).
- **Cost.**
  - P38 adds no kernel function.
  - P39 adds nothing to the success path and removes 8 B of `.bss` plus the per-pin RMW.
  - P40 moves one relaxed store under a lock the path already holds (`tags.rs:183-196`).
  - No hot-path instruction changes anywhere in this revision.
- **loom.** The claim/handoff, exclusivity, carve and injector models (P17.5) are unchanged and still cover the only cross-thread protocol.
- **Modding zero-cost.** G6(a)-(e) are unchanged apart from W1 and W4. Consuming KF-47 strictly shrinks what a no-modding build carries.

## Positive: preserve these

1. **P38 deletes the duplicate instead of justifying it,** and records the reflect lane's own "sibling helper, not an erased typed insert" as a fact about the tree. The two inherited limits a mod sees (the `g17` detach-observer gap, and moved bytes) are named where a mod author will hit them.
2. **P39 deletes `DYN_ID_CEILING` rather than repairing it,** and is honest about the two directions ("mint walks into a pin: fixed; pin lands on a taken slot: diagnosed, not fixed"). G-MINT-1/2 are the right *kind* of gate: they must stay green and fail under the rejected semantics. Only their isolation is wrong (W2).
3. **P40's rewritten ordering argument.** The shipped comment (`tags.rs:122-129`) justified the late classification by archetype construction; R2 made the reader a lookup; P40 makes the name insert the publication point. That is correct for every reader that reaches an id through the name table.
4. **P43.1's layout-token finding** (one id per element type, with a cache-based exception and an overturn gate). It shrinks the bill with a measurement of the code rather than an estimate.
5. **P41's principle:** derive the handle list from §2.1, and count allocations released, not handle types. Keep the principle; fix the counter's location (W3).
6. **Everything earlier rounds asked to preserve still stands:** P0, P3, P6, P11's TB-1 rationale, P17's release-sequence argument, P20's push-side liveness proof, P24's FR-1 as a revival rule, P25's priced refusal, P29's "I am not paying that", and P32/32.2's shipped-defect find.

## Open questions, and answers to the architect's pass-6 questions

1. **(Architect 1, the reflect-merge dependency.)** The dependency is correct. When the rung starts is the owner's schedule call.
2. **(Architect 2, G6(f) and `#[used]`/ctor.)** As written, the unused dependency is never loaded, so a `#[used]` static or ctor in `boyko_modding` cannot reach arm A either; the gate will not "find out first" (W1). The ruling you propose — forbid both in `boyko_modding` — is still worth writing, and a `syn` check over that crate enforces it cheaply.
3. **(Architect 3, `g17`.)** Agreed. Have the inherited-limit paragraph cite `g17` by name and its RED-by-design status, so the paragraph's deletion is tied to the lane flipping that test.
4. **(Mine, PLAUSIBLE, no consequence found.)** P40 says "a reader reaches an id only through `DYN_NAMES`". Readers that scan `LAYOUTS` — `is_type_registered_as_component` (`mod.rs:1111`), P39's `id_space_census`, any reflect-side enumeration — see a published layout before the kind store. I found no such reader that acts on the kind. Either confirm there is none, or scope the sentence to readers that obtain the id through the name table. ⚠ **Disposition (rev 2.5, P50):** D-S1(i)'s cut lists every `LAYOUTS` scanner and confirms none acts on the kind, or else scopes P40's sentence (plan 02 §2).
5. **(Mine.)** Class A lists `EcsMaster::remove_component_type` as having no engine caller. P21 says unload steps 1-2 are "kernel operations the engine already needs for world teardown". If world teardown calls it, the membership rule makes it class B. Which is it? ⚠ **Disposition (rev 2.5, P50):** void — `remove_component_type` is deleted (U-10).

Status: closed at rev 2.4 (critique pass 6) by orchestrator ruling; open remarks W1-W5 above. ⚠ *Rev 2.5: every remark above has a disposition (P50); rev 2.5 follows.*

---

# Rev 2.5 (2026-09-23)

# Allocator design — Rev 2.5 (patch against Rev 2.4)

**Scope.** Step DOC-1 of the unified system plan (`docs/unification/UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md` §5, first row; `UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md` §2, "Document steps"). Unlike rev 2 to 2.4, this revision answers no critique of its own. It writes the plan's rulings into the file the plan builds on, and it gives every remark of critique pass 6 (the plan calls it AP6) a disposition. Six patches:

- **P46:** revival forms for the Heap class (with `HeapDyn`), `TableSet` and `DropColumn` (plan rulings U-1, U-7, U-8, U-9).
- **P47:** §7 is re-pointed to the plan's file 05; G6 = UG-15; pass-6 W1's duplicate (f) is relabelled.
- **P48:** §2.0. The engine thread context lives in `boyko_threadpool`, and `boyko_memory` keeps G5's `#![no_std]` (U-19).
- **P49:** P29. `HeapRef::alloc_cold` is struck, and four symbols remain.
- **P50:** a disposition for every pass-6 remark.
- **P51:** pass-6 O1's stale passages, fixed in place.

**Review.** Critique pass 7 (AP7) reviews **only this delta and the pass-6 dispositions**, because pass 6 already reviewed rev 2.4 (plan 02 §2). C1 waits for AP7: P48 rewrites §2.0, and §2.0 is what C1 builds.

**What does not change.** The following content stands:
- `ByteColumn` and `ZeroInit` (P5, P37/4);
- `ChunkArena`/`SlotChunks` (P2, P16, P17, P28, P36/O1);
- the injector ring and its spin (P10, P20);
- rung 1f;
- the gate ladder G1–G5 with G2b (P6, P7, P9, P23, P31/1 as corrected by P36/O3).

P39 and P40 stay as pass 6 reviewed them. P39.4 carries a marker that records U-2's effect on its client paragraph, not on the mechanism. Where a surviving item names a primitive that P46 moves, only that name is re-pointed (P46.3).

**Trees.**
- **Documents:** read in `D:/wt/docs`, branch `u/doc-1-2` @ `49f2fcfb`, cut from `feat/multi-paradigm-render`.
- **This file is byte-identical at `b716a5dc` and `49f2fcfb`:** `git diff --stat b716a5dc 49f2fcfb -- docs/memory/ALLOCATOR-DESIGN-SPACE.md` prints nothing. So every plan citation into this file "at `b716a5dc`" holds here. Each one was still re-located by content before use (P50).
- **Plan files** are cited as `00`…`05`, meaning `docs/unification/UNIFIED-SYSTEM-PLAN-0N-*.md` at `49f2fcfb`. The ledger is `docs/memory/RUNTIME-DATA-LEDGER.md` at `49f2fcfb` (rev 4).
- **Code:** one file was read, read-only, for P48's open question: `crates/boyko_ecs/src/ecs/memory/vm.rs` at `49f2fcfb`.
- No cargo command, build or test was run.

**Convention, and one addition to it.**
- **The file's form is kept.** Each change quotes the text **removed** verbatim and then gives the text **added**, so the history reads in one place.
- **New in rev 2.5: superseded passages are also marked where they stand.** Since rev 2, the file header has warned that a removed sentence "still reads as current where it stands". Pass 6's O1 found passages that the append-only form had missed. So every passage this revision supersedes now carries a marker on its own line: a suffix that begins `⚠ Rev 2.5`. Where the words themselves are corrected (P49, P51), the stale words are struck (`~~…~~`) beside their replacement.
- **Two rules bind the markers.**
  - **No sentence is deleted.**
  - **No line is inserted before this Part.** Every marker is appended to an existing line, because 24 other documents cite this file by line number (ripgrep for `ALLOCATOR-DESIGN-SPACE.md:[0-9]` under `docs/`, excluding the frozen `docs/ru/`). Many of those citations carry no tree tag.
- **So rev 2.5 moves no line of lines 1–4158.** The marker index at the end of this Part lists every line it touched.

---

## P46 — the Heap class (with `HeapDyn`), `TableSet` and `DropColumn` become revival forms (U-1, U-7, U-8, U-9)

**Depends on / re-read:**
- plan 00 §3, rulings U-1, U-7, U-8, U-9 (`00:108`, `:114-116`);
- plan 01:
  - §2 KC-01, KC-03, KC-06, KC-07, KC-10, KC-15..KC-18 (`01:78-105`);
  - §3's "Allocator rev 2.4" mapping (`01:456-490`);
  - §4 R-A, R-F, R-G (`01:499-521`, `:570-572`);
  - §8, the revival-form shape (`01:984-989`);
- plan 03 §3 ("C1: Heap test not applicable (U-1)", `03:55`) and §5 MQ-08 (`03:135`);
- this file: P1, P8, P15, P19/19.2, P22, P25, P30, P31, P35, P41.

### 46.1 The ruling, and what a revival form is

**The four primitive families are not built.**
- Plan 00 §3 rules each one by performance and names the gate that would overturn it.
- The kernel contract takes their clients into ledger forms (01 §3, the "Allocator rev 2.4" rows; 01 §4 R-A, R-F, R-G).

**A revival form** is the plan's device for a reviewed design that is not built; 01 §8 applies it to the windows word arm.
- **The text is kept,** so that an overturn builds a reviewed design rather than a new one.
- **The revival form states** the design (by location), its gates and its trigger, with the reason it is not built now.
- **Nothing is deleted.** No text of P1, P15, P22, P25, P41 or the rest of the Heap design is removed. This patch states which text belongs to which revival form, and which rulings route the former clients.

**The Heap class:** `Heap`, `HeapHeader`, `HeapRef`, `HeapVec`, `HeapBox`, `SortedMap`, plus `HeapString`, already retired by P35.
- **Ruling:** U-1, deferred and not built.
- **Why not now:**
  - The per-frame cost is equal either way, because every client is build-time.
  - The Heap would cost an eager Miri reserve of 1.26 MiB per `EcsMaster` and 2 GiB of VA per master (`:3171-3172`), with a worst resident of 204 KiB (`:2718`).
  - It would add a new unsafe surface that is sensitive to Tree Borrows: `HeapHeader`, `HeapRef`, the class free lists and a 448-entry run table (01 R-A).
  - The ledger forms cost a 4 KiB floor per column after D-M1 and add no new `unsafe`. They run on `ComponentPool` code that is already hot in L1i.
- **Clients go to** (01 KCs):
  - K7 spans (KC-15, `SegmentedColumn`): pow2 classes with LIFO class free lists, i.e. the `HeapVec` algorithm inside ECS storage (01 R-A);
  - the erased record column (KC-17);
  - fixed `VmColumn` tables (KC-18);
  - sorted `ScratchColumn` pairs (KC-10).
  - Row-level routing is the ledger's. B1 names this design authoritative for mechanism and the ledger for row inventory (02 §2).
- **Revival trigger** (the ruling's overturn gate; 00 U-1, 05 §4), either of:
  - ledger rev 5 finds a row that needs storage that is individually freed, variable-size and non-`Copy`, owned by `schedule`, `registry` or `master-table`, and not expressible as KC-15, KC-17 or KC-18;
  - modding Stage 3 needs mod-private freed memory. The heap then lives in `boyko_mod_host`, not in the kernel.

**`HeapDyn`**
- **Ruling:** U-1 (its storage is the Heap) and U-9 (its role).
- **Why not now:**
  - U-9 stores erased objects (`Box<dyn System>`, `Box<dyn FnOnce>`, resource values) in KF-07 records. They sit behind this design's own `ErasedSystem { data, vtable }` handle, are dropped all at once after being appended, and cost one indirection, as today.
  - `HeapDyn` is that handle plus a `HeapRef` word (24 B, P8/Q5), with one individually freed heap object per value.
  - KC-17's handle is 16 B, and its records are freed together.
- **Clients go to:** KC-17's `RecordColumn` plus `ErasedSystem` (16 B).
- **What survives:** the `DynVTable` shape becomes KC-17's record vtable. That shape is a `&'static` per-trait vtable built by a generic `const fn`, with `LAYOUT` as an associated const and `drop_in_place` (P1, P8/Q5). 01 KC-17 reads: "`&'static` vtable (`drop_in_place`, `LAYOUT` const)".
- **Revival trigger:**
  - U-9's overturn (AL:M-A1's dispatch floor regresses beyond its band) reopens the question of the erased-object form. `HeapDyn` is then the reviewed candidate.
  - `HeapDyn` also needs the Heap's own trigger, because its storage is the Heap.

**`TableSet`:** `TableSet`, `TableLayout`, `TableSlot<T>`, `Table<T>`.
- **Ruling:** U-7, deferred.
- **Why not now:**
  - Fixed kernel tables become fixed-capacity `VmColumn`s, or inline arrays inside one existing reservation (KF-31).
  - After D-M1 each such column has a 4 KiB floor. So one reservation per table set shares granules only for tables smaller than a page.
  - 01 R-F: "KF-31's inline arrays and the deferred TableSet produce the same memory".
- **Clients go to:**
  - KC-18 (`VmColumn<T, TableOwner>`, the `Table` producers of UG-04);
  - KC-07 (`PoolInner` in one reservation, with inline `MAX_WORKERS` arrays);
  - KC-06 (the injector ring, in the pool's reservation).
- **Revival trigger:** UG-20 records more than 64 KiB of sub-page tables per `EcsMaster` (00 U-7).

**`DropColumn<T>`**
- **Ruling:** U-8, not built.
- **Why not now:**
  - The owning column (KF-02 / EK12) is a typed view over an untracked `ComponentPool` with drop glue.
  - The pool already stores `drop_fn`, and KC-10's registry-free constructor takes it by value from `T`. The column therefore uses **no `ComponentId`** (U-2).
  - That leaves no second droppable column type to cover under Miri.
- **Clients go to:** KC-16 `OwnedColumn<T>`, built on KC-10's `ComponentPool::new_untracked_raw(.., drop_fn, ..)`.
- **Revival trigger:** **none named.** U-8's overturn column reads "none expected". A revival would need a new ruling, and it would start from this text (open question 3).

### 46.2 What each revival form keeps

The kept text stays where it is, and each location carries a `⚠ Rev 2.5` marker (see the marker index).
- **The gates below are not built while their form is not built.** 03 §3 already records "C1: Heap test not applicable (U-1)", and 03 §5 MQ-08 strikes AL:M-A3 and AL:M-A4.
- They return with their form, unchanged unless the reviving rung re-derives them.

**Heap class, with `HeapDyn`: design.**
- §2.1's Heap row, as replaced by P1 and P35.
- §2.3/§2.4's Heap blocks, as replaced by P1, P15/15.2 and P30/O2.
- §2.5's `Heap`/`HeapVec` rows.
- P1: HEAP-1, `HeapRef` = reservation base, HV-1..HV-3, and the rejected 16-B `HeapVec` sub-variant with its overturn gate.
- P8: O1 (the ZST class); O3 as ruled by P15/15.1; O8; Q5 (`HeapDyn` is 24 B).
- P15:
  - eager reserve with lazy commit;
  - the self-sufficient header;
  - `raw::commit_at` as the header's commit route;
  - the cold path as amended by P25;
  - 15.4's Miri constants as amended by P25/25.3.
- P19/19.2: HV-4.
- P22: `MAX_RUNS = 448`, and the sanitize map's own reservation.
- P25: the class-scaled page quantum (worst case 51 pages = 204 KiB).
- P30: O1's Heap rows, and O2 (`LINE0_PAD`).
- P31/3: the closed `hv3-owner` vocabulary.
- P35: the retirement of `HeapString`, its revival gate, and the derived heap-client set.

**Heap class: gates.**
- C1's red-first Miri test `miri_heap_handle_aliasing.rs` on both legs: P1, P15/15.6's M1 and M2, and P41's derived body. **Pass-6 W3 is carried with it** (46.4).
- G1's HV-1 declaration-order assertion, and the debug `live_allocs == 0` backstop (P15/15.5).
- G1's HV-4 assertion, and the ledger's `hv3-owner` column (P19/19.2, P31/3).
- The ledger's `max-bytes` column, and `large_allocs` pinned at 0 (P25/25.2).
- The full-run-table counter pinned at 0 (P22/O1).
- The sanitize live-bit map (P3 (a), P22/O2).
- `assert!(offset_of!(HeapHeader, free) == 64)` (P30/O2).
- G2b's `Heap` owner pinned at 0 (P31/1).
- §5.2's reuse-order row, and P8/O8's capacity re-baseline row.
- The Heap halves of the unit-test and `debug_assert!` bullets (P34 sites 16 and 17):
  - "`Heap` class boundaries (16, 256, 257, 512, 64 KiB, 64 KiB+1); free-list LIFO order";
  - "owner thread on `Heap` mutation; class ↔ layout on `free`; HV-1 at `Heap::drop`".
- Measurements M-A3, M-A4, and M-A7's Heap constants (P14). 03 MQ-08 strikes AL:M-A3 and AL:M-A4.

**Heap class: modding half.** K-MOD-1..K-MOD-3 and the "> 64 mods share one `Heap`" overturn (P12, P21). P47 withdraws them from §7. Only U-1's second trigger revives them, and then in `boyko_mod_host`.

**Heap class: trigger and reason.** See 46.1.

**`TableSet`: design.**
- §2.1's Table row, §2.3's Table block and §2.4's Table API.
- P8/Q3's build point: `MasterTables` is built at `App::finish()` and holds only sites whose count is final there. Q3's other half stands and is not part of the revival form: `EventBuffer` lanes and `EnableStore.pages` left the class for growable columns (01 KC-03).
- P30/O1's `MasterTables` and `ScheduleTables` rows.
- P36/O3's statement that `TableSet` names its own commit owner.

**`TableSet`: gates.** The unit test "`TableSet` alignment of every table" and the debug assert "`TableSet` bounds" (P34 sites 16 and 17).

**`TableSet`: what survives in the kernel.** The `Table` commit owner.
- It is the owner of KC-18's tables, declared `VmColumn<T, TableOwner>`, and UG-04 pins it at 0 in the steady window (01 KC-01; 03 UG-04).
- UG-04's red control: a KC-18 table built with the default owner reads 0 at setup.

**`DropColumn`: design.**
- The `DropColumn<T>` entry in §2.1's Column row.
- §2.3's `DropColumn` struct (32 B) and §2.4's `DropColumn` API.
- The rung-2 and rung-3 rows that name it (46.3).

**`DropColumn`: gates.** The unit test "`DropColumn` destructor count on truncate/swap_remove/drop" (P34 site 16).
- Its property is also the property KC-16 must hold: each destructor runs exactly once on truncate, `swap_remove` and drop.
- Which test proves it for KC-16 is the plan's rung to decide, not this file's.

### 46.3 Consequences for content that stays

These items stay. Only the name of a primitive that P46 moves is re-pointed.

- **The commit owner** (P31/1 as corrected by P36/O3): `raw::commit_at<O: CommitOwner>`, counted in `COMMITTED_BYTES[O::INDEX]`. Source: 01 KC-01; 03 UG-04.
  - **There are three owners: `Column`, `Chunk` and `Table`.** `Heap` leaves with U-1, and `Frame` left with P34 (P36/O3 already counted four).
  - G2b (UG-04) pins `Chunk` and `Table` at 0 over the steady window and reports `Column`.
  - The anti-vacuity direction applies per owner whose producer has landed: `Chunk` from D-M2 (`ChunkArena`), `Table` from D-M1 (KC-18).
- **G1 (UG-02).** The `syn` ledger gate stays (P6). Its three Heap-only assertions have no subject and are not built (46.2): HV-1 declaration order (P15/15.5), HV-4 on component values (P19/19.2), and the `hv3-owner` and `max-bytes` columns (P31/3, P25/25.2). Source: 03 UG-02.
- **Rung 1c** is unchanged: `ByteColumn` (`spare_ptr` + `set_len`) holds `CommandQueue.bytes`, `panic_recovery` (P8/O7) and `ErasedKindBuffer.data`. It maps to KC-03 + KC-17 (01 §3: "1c → KC-17 + KC-03").
- **Rung 1d** (P10) keeps the injector ring unchanged. Its home is **the pool's own reservation** (KC-06, "pool reservation"), not "the pool's `TableSet`". Source: 01 KC-06, KC-07; U-7.
- **Rung 1e** (P34 site 8): the per-epoch `TermList` boxes go to KC-17 records or KC-10, not to `HeapDyn`/`HeapVec`. `Local<TraversalScratch>` is unchanged. Source: 01 §3 ("1e `TermList` → KC-17 / KC-10").
- **Rung 2** (Part I §4, as amended). Source: 01 KC-15, KC-16, KC-18; 00 §5 (the ledger row).
  - The observer arenas and `TriggerLists.by_trigger` move from `DropColumn<..>` and `HeapVec<HeapVec<..>>` to KC-16 or KC-15, per ledger row.
  - `boyko_utils::SparseMap` (Part I, and P42 (1)'s `new_in(HeapRef)` row) moves into `boyko_ecs` on `VmColumn`s (KC-18; rung ~~D-R2d~~ D-M1). ⚠ *Rev 2.6 (P54/O3): D-M1 moves it (01 KC-16, `01:103`; 02 `:127`); D-R2d only retires its KF-02 row (`sparse_map.rs:10`).*
- **Rung 3** (Part I §4). Source: 01 §3, R-A.
  - `ScheduleTables`, `MasterTables` and `PoolTables` become KC-18's fixed `VmColumn` tables and KC-07's inline arrays (U-7).
  - `Table<ErasedSystem>` becomes KC-17.
  - The builder's `SortedMap` becomes sorted `ScratchColumn` pairs (KC-10).
  - `HeapDyn<FnOnceVTable>` becomes KC-17.
  - `DropColumn<ComponentPool>` and `DropColumn<Option<DenseStore>>` become KC-16 or KC-18, per ledger row.
  - Unchanged: the `Arc<ThreadPool>` removal (KC-07); `EventBuffer` lanes and `EnableStore.pages` as growable `VmColumn`s (P8/Q3).
- **§5.1** (as replaced by P33/33.1 and P42 (1)): `EcsMaster` gains **neither** `heap: Heap` **nor** `tables: TableSet`. The `SparseMap` row follows rung 2 above. Source: U-1, U-7.
- **P30/O1's reservation table.** Source: U-1, U-7.
  - These rows belong to the revival forms: `EcsMaster`'s Heap rows (and P34 site 13's replacement), the sanitize-map row, `MasterTables`, `ScheduleTables`, and the per-mod row.
  - These rows stand: the `ThreadPool` rows (`ChunkArena` at 1 GiB, 2 MiB under Miri; `SlotChunks` at 448 B × (W+D)) and the per-`ComponentPool` row.
- **P14 step 3** (as replaced by P34 site 15). Source: 01 KC-01, KC-03; 03 §3.
  - `boyko_memory`'s primitives are `ZeroInit` + the relaxed `VmColumn` bound (P5) and `ByteColumn`. There is no `DropColumn`, no Heap class and no `TableSet`.
  - C1's red-first Miri test is not written.
- **§1**, "What (i) cannot do" and "Rejected alternatives". Source: 01 KC-15, KC-16.
  - `HeapVec`'s relocate-on-grow clients take KC-15 spans, which also relocate on grow.
  - The rejection of `allocator_api` stands. Its sites are now served by KC-16 and the ledger forms instead of `DropColumn`/`HeapVec`.

### 46.4 Pass-6 W3, carried with the Heap's revival form

**Removed:** nothing. P41's Anti-vacuity cell (`:3729`) stays as the revival form's text.

**Added** (a note bound to P41's cell; the rung that revives the Heap applies it):

> **Pass-6 W3 (`:4003-4020`): the counter's location contradicts its derivation.**
> - The counter sits in `HeapRef::free` and must equal 5.
> - The test body calls `owner.heap.as_ref().free(..)` once per handle repetition, and that call is `HeapRef::free` too. So a correct implementation reads 5 + 4 = 9.
> - It reads more if `push 8` or a `SortedMap` insert grows through `HeapRef::grow`, whose documented slow path is alloc + memcpy + free (`:744-745`).
>
> **The derivation stands; the constant does not.** Before the test is written, the reviving rung chooses one of pass 6's three repairs and records the choice in the test header:
> 1. count at the handles' `Drop` sites, which is the quantity the derivation describes;
> 2. derive the constant including the body's own frees and any grow frees, and say which;
> 3. assert a lower bound instead of an equality.
>
> Until then the gate is not built (U-1; 03 §3), so the remark has no current subject. It is **void under U-1 and carried here** (plan 02 §2, row W3).

---

## P47 — §7 is re-pointed to the plan's file 05; G6 = UG-15; pass-6 W1's duplicate (f) is relabelled

**Depends on / re-read:**
- this file: P12 (§7 and G6), P18, P21, P26, P29, P32, P33/33.2, P38, P44/O2;
- plan 00 §3, U-10 and U-11 (`00:117-118`);
- plan 01 §3's §7 rows (`01:474-485`), and 01 KC-19a/KC-19b;
- plan 03 §6, UG-15 legs (1)–(7b) (`03:165-184`);
- plan 05:
  - §1: H-1, rule S-1 and the crate names (`05:25-96`);
  - §3 (`05:115-214`);
  - §4, the reconciliation table (`05:216-231`);
  - §5, row AP6-W4 (`05:252`);
  - §6 (`05:265-306`).

### 47.1 The ruling

**§7 is no longer this file's to specify.**
- Plan file 05 carries modding as testable properties under two rules:
  - the owner's requirement H-1;
  - rule S-1: kernel modding code is generic over `ModSeam`, so it is not compiled when it is unused.
- 05 §4 is the reconciliation of this section with the closed modding design.
- **From rev 2.5, file 05 is authoritative for §7's content.** §7's text stays as the history of how its items were derived, and each K-MOD row carries a marker pointing to its disposition in 05.

**The crate is renamed.** P12's single crate `boyko_modding` is retired in favour of three (05 §1 (a)), and no kernel crate depends on any of them:
- `boyko_mod_host` (the host image);
- `boyko_mod_api` (the mod image);
- `boyko_mod_registry` (the modding game binary only).

### 47.2 The K-MOD rows, re-pointed

**K-MOD-3 and K-MOD-10 are removed** (00 §5, the DOC-1 row). The other rows follow 05 §4.

- **K-MOD-1** (`HeapRef::{alloc, free, grow}`, taking a runtime `Layout`): **withdrawn with U-1**, because mods use engine storage forms. Now: the Heap's revival form (P46); 05 §4, first row.
- **K-MOD-2** (`HeapRef` identity): **withdrawn with U-1**. Now: the same.
- **K-MOD-3** (one `Heap` per mod; P12, with the unload steps of P21 and P26): **removed.** Now: 05 §4, first row.
  - Its reason (iii), O(1) unload, is void under load-only (U-10).
  - Its reasons (i) and (ii) apply to any engine structure a mod writes, and a heap does not solve them (05 §4).
  - It is revived only by U-1's second trigger, and then in `boyko_mod_host`.
- **K-MOD-4** (the erased pool constructor): **kept.** Now: 05 §2, row 3 ("Pool construction").
- **K-MOD-5, with P26 and P39:** **kept.** Now: 05 §3.2 MS-02b and MS-03; 01 KC-19a/b.
  - The skip mint is KC-19a, a bug fix in rung D-S1(i). The occupancy reads are MS-03 (Stage 1, rung D-S1(ii)).
  - The sized by-name mint is MS-02b: option A only, at modding Stage 3. It covers `intern_or_mint_sized`, `new_dynamic`, `try_register_dynamic_by_name` and `dynamic_by_name`.
- **K-MOD-6** (`drop_fn` scoped to the seam, P44/O1): **kept.** Now: 05 MS-09 (mod components are POD in v1).
- **K-MOD-7** (the not-exposed list): **kept.** Now: 05 §4, row K-MOD-7.
  - `FrameArena` (P34) and the Heap (U-1) are removed from the list.
  - `ChunkCache` reads `SlotChunks` (P16); P51 fixes the row.
  - Its gate is UG-15 leg (1) plus a `raw` import census.
  - Under option A, `register_hooks_by_id` and the typed hooks builder join the list (pass-6 W4; 05 §5, row AP6-W4).
- **K-MOD-8** (the registry is a `Resource` in the modding crate): **kept** (05 §4).
- **K-MOD-9** (no C symbol in the kernel): **kept = UG-15 leg (1).** Now: 05 §4; 03 §6 leg (1). ⚠ *Rev 2.7 (P56.2, AP8 N-W1): these thunks are called by a mod or by the host after `main`, never by the loader or the C runtime, so the modding-crate ban on runtime-invoked code does not touch them.*
  - Leg (1) is a `syn` census of export and retention attributes, non-Rust-ABI function definitions and global assembly.
  - It is never re-blessed, and its owner-held allowlist is empty.
- **K-MOD-10** (`EcsMaster::remove_component_type`; P21, P26): **removed.** It is deleted by U-10: mods are load-only, and kernel registries are write-once. Now: 05 §4, row K-MOD-10; 00 U-10.
  - It was class A of P38.3's inventory, so class A shrinks to three items.
  - Those three are MS-02b's (option A, Stage 3).
- **K-MOD-11** (P32/32.5, as consumed by P38): **already consumed.** KF-47's by-id ops carry `MOD-SEAM` doc markers. Now: 05 MS-08; 01 KC-21.
- **`MAX_MOD_TYPES = 128`** (P26, P43.2): **replaced** by MS-03's D2 bound. That bound is `ENGINE_COMPONENT_CEILING`, held by a census, and it is computed against MD:M-K3 after D-S2 frees the physics band. Now: 05 §4 and MS-03.
- **P32 R1 (`TAG_NAMES` → `DYN_NAMES`) and P40's sized body:** option A, Stage 3 (MS-02b). Now: 01 §3; 01 KC-19a; 05 RM-3.
  - P40's **tag path** lands in D-S1(i) as KC-19a's `intern_or_mint_tag(name, kind)`.
  - P40's mechanism is unchanged. Only the map's name changes: it stays `TAG_NAMES` until MS-02b, because D-S1(i)'s rename list is empty.

### 47.3 G6 = UG-15

U-11 merges allocator G6 and the modding design's M-P1 into one gate, UG-15 (03 §6). Its legs succeed G6's rows:

| G6 row (P12 → P18 → P29/P33/P38/P44) | UG-15 leg (03 §6) |
|---|---|
| (a) no exported symbol (P18/18.2) | leg (1), the source census (K-MOD-9's rule), and leg (7)(c), the linked image's export directory |
| (b) no kernel code growth: the pinned `.text` set (P18, P29, P31/4, P36/O2) | leg (2): asm pins read from the post-LTO object on the msvc gate host (U-22). The set is P29's as struck by P49 (four symbols), plus the bodies leg (2) adds |
| (c) no kernel growth: `size_of` pins | leg (3) |
| (d) no startup work | leg (4), startup equality |
| (e) the seam cannot grow silently (P33/33.2 → P38.3) | leg (5): the `MOD-SEAM` markers equal the list in **05 §3**, derived by P38.3's membership rule, plus rule S-1's shape check. P38.3's thirteen-item list is not restated here, so it cannot drift from 05. The four `HeapRef`/`Heap::new` items leave with U-1, `remove_component_type` leaves with U-10, and the three remaining class-A items are MS-02b's |
| **(f)** cargo feature unification and graph leakage (P18, the two-arm build) | **leg (6), and that is its only role.** Cargo builds a dependency with the union of every feature that any package in the graph enables on it `[D Cargo Book, "Feature unification"]`. So a modding crate can switch on a feature of a shared dependency for a game that never asked for it; `cargo tree -e features` shows it |
| **(g)**, relabelled from P44/O2's second "(f)": no binary-size delta | **leg (7)**, the linked seam census, run on every seam commit against its parent (47.4) |

### 47.4 Pass-6 W1: the duplicate (f)

**Removed** (P44/O2, the row's label, `:3830`):

> (f) **no binary-size delta** (NEW)

**Added:**

> (g) **no binary-size delta** (NEW)

UG-15 leg (7) answers the row; its own two-arm mechanism does not. The row stays as P44 wrote it, with the struck label beside the new one, so its history reads.

**Why the row cannot see its subject** (pass 6, W1, `:3936-3967`; checked against the toolchain's documentation rather than taken on trust):
- **The arms.** Arm A declares the modding crate as a dependency and never calls it; arm B removes the dependency.
- **An unreferenced crate is not linked.** rustc links a crate passed with `--extern` only when code references it. The rustc lint documentation gives `extern crate foo as _` as the way to link a crate "for the side effect of ensuring the given crate is linked, even though it is not otherwise directly referenced" `[D rustc lint unused_extern_crates]`. So arm A's binary carries nothing of the unused crate that arm B lacks, except what feature unification changes, and that is row (f)'s subject.
- **Every class-A item (P38.3) is a kernel item,** present in both arms. A kernel-side survivor grows both arms equally, and the delta stays 0.
- **Neither canary can fire.** `#[no_mangle]` on a kernel item changes both arms. "The binary calls a class-A function" names no mechanism that makes the call arm-specific.

**What does see it: UG-15 leg (7).**
- **Where it runs:** on every seam commit, in strict mode, against the commit's **parent**.
- **What it compares** (03 §6), under the commit's rename list; all three must be identical:
  - the linked `boyko_demo`'s section sizes;
  - its defined-symbol multiset, over all bindings including local ones, because a fat-LTO survivor is local;
  - its export directory.
- **Why that works.** The parent does not carry the seam item, and the commit does. That is the longitudinal baseline P18/18.1 already said the quantity needs ("the seam the kernel carries in both arms … is a *longitudinal* quantity, so the baseline must be longitudinal"), applied to the linked binary.
- **Its red controls** (vii) and (viii) (03 §6) are the canaries P44's row lacked:
  - (vii): `boyko_demo` calls a kernel `pub fn` that no engine path calls. Leg (7) goes red.
  - (viii): a kernel `static` holds a fn pointer, with no attribute and no caller. Leg (7) goes red while leg (1) stays green.
- **Leg (7b)** adds the census of rlib object members, for items the linker would have dropped (05 §6).

**Leg (6) keeps only the feature-unification role** (plan 02 §2, row W1; 00 §5).
- P38.5's cost row says class A is "dropped at link, now **measured** by G6(f)'s `.text` delta" (`:3618`). That row now reads (g), and the measurement is leg (7)'s.
- Under S-1 (05 §1 (b)), the class-A items are generic over `ModSeam` and have **no object code** without an implementor. So the claim no longer rests on the linker at all, and legs (7) and (7b) are its proof.

**Pass 6's answer to the architect's question 2** (`:4153`): forbid `#[used]` statics and ctors in the modding crate, enforced by a `syn` check. ~~Leg (6)'s own control in 03 §6 carries it ("a `#[used]` static in a modding crate", run from the first modding crate on).~~ ⚠ *Rev 2.6 (P53, AP7 W2): withdrawn. Under this Part's own arm A (declared, never referenced; `:4483`) that static never reaches the binary, so the control cannot go red, and no `syn` check reads modding-crate source. The ban is a rule without a gate; P53.3 hands a candidate mechanism to the plan (03 §6).* ⚠ *Rev 2.7 (P56.1, AP8 N-W1): the ruling is scoped to its stated harm, runtime-invoked code. A `#[used]` static outside a runtime-walked table, such as MS-15's `.boykom$m` static, is not in it. The candidate mechanism is restated in P56.3–P56.5.*

---

## P48 — §2.0: the engine thread context lives in `boyko_threadpool`, and `boyko_memory` stays `#![no_std]` (U-19)

**Depends on / re-read:**
- this file: §2.0 (`:131`) and §3 G5 (`:337`);
- plan 00 §3, U-18 and U-19 (`00:125-126`), and 00 §5's ledger row (`00:157`);
- plan 01 §2: KC-01 (Layer 0, `01:78`) and KC-04 (Layer 1, `01:91`); plan 01 §6, items 1–11 (`01:624-915`);
- the ledger, `RUNTIME-DATA-LEDGER.md:1626`, `:1629` and `:1718`.

**The placement this reverses.**
- The design itself never placed the thread context, but the ledger it is read with does:
  - KF-45 is "Kind: capability (memory library, below the pool)" (`RUNTIME-DATA-LEDGER.md:1626`).
  - Its provider is the "KF-32 memory layer" (`:1629`).
  - The ledger's net result says: "KF-32, a layering move of the memory library below the pool; KF-45's thread-context column lives there" (`:1718`).
- §2.0 is where KF-32's crate split is specified (01 §3: "§2.0 crate split → KC-01"). So this is where the placement is overruled, before C1 builds the crate.
- Ledger rev 5 restates the ledger side (00 §5, ledger row; rung B1).

**Removed** (§2.0, `:131`, verbatim, without its rev-2.5 marker):

> `boyko_memory` — new crate containing `vm.rs`, `vm_column.rs`, `utils.rs`, the granule/page constants, and the four classes below. `boyko_ecs` keeps `component_pool.rs`, `device_column.rs`, `inland_store.rs`, `scratch_column.rs` (they know ticks, ids, and the stagger). Reason: `boyko_threadpool` and `boyko_utils` need reservations and cannot depend on `boyko_ecs` (cycle: `boyko_ecs` → `boyko_threadpool`). `boyko_memory` has no dependencies beyond the OS crates already used by `vm.rs`.

**Added:**

> `boyko_memory` is a new crate. It contains `vm.rs`, `vm_column.rs`, `utils.rs` and the granule/page constants; the `#[doc(hidden)] pub mod raw` (`reserve`, `commit_at<O: CommitOwner>`), with `COMMITTED_BYTES[O::INDEX]` over three owners (P15/15.2, P31/1, P36/O3, P46.3); and the Column-class contract of §2.2 as amended by P5 and P37/4 (`ZeroInit`, `ByteColumn`, `ensure_len_zeroed`, the stack helpers). These are kernel contract items KC-01 and KC-03. ⚠ *Rev 2.6 (P54/O4): two rungs land them. C1 lands KC-01 only (the move, `raw::{reserve, commit_at}`, the UG-04 counter; 02 `:119`), and D-M1 lands KC-03 (`ZeroInit`, `ByteColumn`, `ensure_len_zeroed`, the stack helpers; 02 `:127`).*
>
> **It holds no lifetime class of its own.**
> - The Heap and Table classes are revival forms (P46).
> - The Frame class is deleted (P34).
> - The Scope class's `ChunkArena`/`SlotChunks` are `pub(crate)` in `boyko_threadpool` (P2, P16; KC-05).
>
> `boyko_ecs` keeps `component_pool.rs`, `device_column.rs`, `inland_store.rs` and `scratch_column.rs` (they know ticks, ids and the stagger), and KC-16's owning column.
>
> **Why the split:** `boyko_threadpool` and `boyko_utils` need reservations and cannot depend on `boyko_ecs` (cycle: `boyko_ecs` → `boyko_threadpool`). `boyko_memory` has no dependencies beyond the OS crates `vm.rs` already uses.
>
> **The engine thread context is not here** (U-19; 01 KC-04, §6). It lives in **`boyko_threadpool::thread_ctx`**, the one crate every client already depends on:
> - the per-thread record table: `THREAD_RECORDS` (8192 × 64 B) and the `THREAD_BUSY` bitmap, both all-zero `.bss` statics;
> - the per-thread word that holds the record's index + 1;
> - the claim/adopt/release protocol;
> - the `EXIT_GUARD` exit hook.
>
> `boyko_ecs` reaches its 24-B region of a record only through `ThreadRecord::ext_ptr()`, called from `boyko_ecs`'s `ecs_fields()`.
>
> **The reason is G5.**
> - The word is a const-initialised, `Drop`-free `thread_local!` `Cell<usize>`, and the exit hook is std's TLS destructor on a guard.
> - Both are std. The macro is `std::thread_local` `[D Rust std: macro thread_local]`. The destructor behaviour is `std::thread::LocalKey`'s: "values that implement `Drop` get destructed when a thread exits" `[D Rust std: LocalKey]`.
> - The only non-std route is the `#[thread_local]` attribute on a `static`, which is feature-gated `[D Unstable Book: thread_local; rust-lang/rust#29594]`.
> - So a thread context in `boyko_memory` would cost this crate G5's `#![no_std]` (`:337`). That attribute is the structural guarantee that no `Vec`, `Box`, `String` or `format!` can be named in the memory library.
> - In `boyko_threadpool` it costs nothing, because that crate is std already: it spawns the workers.
>
> **Not built here either:** the windows word arm (a `TlsAlloc` slot read at `gs:[0x1480 + 8·index]` after a canary). It is plan 01 §8's revival form D-M6w, and if U-19 (a) or (b) ever builds it, it lives in `boyko_threadpool` and `boyko_diag`.

**Cost.** P48 changes no mechanism of KC-04. It changes only which crate is recorded as KC-04's home. KC-04's costs are the plan's (00 U-19).

---

## P49 — P29: `HeapRef::alloc_cold` is struck; four symbols remain

**Depends on:**
- this file: P18/18.1 (b), P29, P31/4, P36/O2;
- plan 00 §3 U-1;
- the plan's own critic pass 5, W1 (a) (`00:881-906`);
- plan 03 §6 leg (2) (`03:170`).

**Why.**
- **The finding.** The plan's critic pass 5 (W1 (a)) found that the P29 set has five symbols, including `HeapRef::alloc_cold`.
  - U-1 defers the whole Heap class, and `HeapRef` has 0 hits in the kernel tree.
  - P29's own rule reads "A missing symbol is RED, never a skip" (`:2625`).
  - So a gate built on the five is red by construction at its first capture (B3).
- **Where the strike lives.** The critic asked that DOC-1 carry the strike (`00:906`). UG-15 leg (2) already pins "the P29 set without `HeapRef::alloc_cold` … (four symbols)" (03 §6).

**Removed** (P29, the fifth row of the symbol table, `:2623`, verbatim):

> | `HeapRef::alloc_cold` | `#[cold]` by construction, P15/15.3 | the heap's page-assign path, written by this campaign |

**Added:** *(the row is struck in place. It returns with the Heap's revival form (P46.2), where it meets P29's three conditions by construction.)*

**The four pinned symbols.** Each meets P29's three conditions through its own attribute:

| symbol | attribute, as P29 cited it at `d552be05` |
|---|---|
| `ComponentPool::grow_rows` | `#[inline(never)]`, `component_pool.rs:600` |
| `ComponentPool::commit_subregion` | `#[inline(never)]`, `component_pool.rs:559` |
| `run_check_ticks_scan` | `#[inline(never)]`, `check_ticks.rs:107` |
| `ScopeBlock::grow` | `#[inline(never)]`, `block.rs:425` |

The rung that captures the pins re-reads each site at its cut (B3, on the trunk). A symbol absent from the post-LTO object is recorded with its reason, per 03 §6 ("Missing symbols").

**The count elsewhere.** The number "five" appears in ~~eight~~ nine places. From rev 2.5 each one reads four: ⚠ *Rev 2.6 (P54/O2): the ninth, `:3832` (P44/O2), was missed; rev 2.6 marks it, and `:3662` below, so every live site now reads four.*
- `:2615`, P29, "The five:": marked in place.
- `:2709`, P31/4, "beside each of the five pinned numbers (P29 makes it five)": struck in place. It is an instruction to the pin file's header.
- `:3076`, P32/32.7, "G6(b)'s five pinned symbols" (and the list naming `HeapRef::alloc_cold`): struck in place.
- `:3260`, P36/O2, "two of the five pinned symbols carry local linkage": struck in place. It is still two (`commit_subregion`, `grow_rows`).
- `:3531`, P38's Depends line, "P29 (the five `.text` pins)": struck in place.
- `:3573`, P38.1, "G6(b)'s five `.text` pins": struck in place.
- `:3662`, P39.2, "P29's five pinned symbols are untouched": ~~**not marked.**~~ P39 stays as pass 6 reviewed it (00 §5), and the sentence stays true of the four. ⚠ *Rev 2.6 (P54/O2): marked after all; a count marker leaves P39's mechanism and gate as reviewed.*
- `:1904`, P18(b)'s canary, "any of the four": pass-6 O1. P51 re-words it by rule, so that it cannot go stale again.

---

## P50 — critique pass 6: every remark's disposition

**Depends on:**
- pass 6's review (`:3873-4158`);
- plan 02 §2:
  - "AP6's open remarks, mapped to rungs" (`02:100-112`);
  - the red-first list "D-S1(i), isolated registry rows (AP6 W2)" (`02:300-315`);
  - B1 (`02:71`);
  - D-S2 (`02:140`);
- plan 05 §5, row AP6-W4 (`05:252`);
- plan 00 §6, RK-1 (`00:170`).

Each remark carries a marker at its own line, and the summary lines `:3877-3881` carry one too.

**W1: G6's binary-size row compares two builds that link the same kernel** (`:3936-3967`).
- **Disposition:** answered by UG-15 leg (7).
  - Leg (7) runs on seam commits against the parent, and it sees kernel survivors that the two-arm row cannot.
  - The duplicate (f) is relabelled (g).
  - Leg (6) keeps only feature unification.
- **Carried in:** P47.3, P47.4; 03 §6 legs (6) and (7); 02 §2 row W1.

**W2: the registry gate rows are written as if the registry were per-test** (`:3969-4001`).
- **Disposition:** D-S1(i)'s isolated tests.
  - Each stateful case is the only `#[test]` in its own binary. Cargo runs test binaries serially, and the leg checks `running 1 test`.
  - Pins are relative: `P = id_space_census().next_id + 32`.
  - The dynamic mint is the engine's tag path.
  - G-MINT-3 is not ignored.
  - Row 8 is a lib unit test with a `cfg(test)` rendezvous after the intern lock, so its red is deterministic on the first pass.
  - Pass 6's corrected G-MINT-3 prediction (an out-of-bounds panic on `LAYOUTS[512]`, not a spin) is the red-first text.
- **Carried in:** 02 §2's red-first list for D-S1(i); 03 UG-19. P39.3's rows stay as pass 6 reviewed them, and the rung writes D-S1(i)'s form.

**W3: P41's anti-vacuity equality fails on a correct implementation** (`:4003-4020`).
- **Disposition:** **void under U-1**, because P41's gate is not built. The remark is carried with the revival form.
- **Carried in:** P46.4; 03 §3.

**W4: P38.3's membership rule is not what G6(e) checks; `register_hooks_by_id`** (`:4022-4056`).
- **Disposition:** plan 05 §5, row AP6-W4.
  - The dangling-hook consequence is void under U-10: mods are load-only, so an image is never unmapped.
  - The mismatch between rule and gate is scoped to option A. Option A's Stage-3 rung does two things:
    - it adds `register_hooks_by_id` and the typed hooks builder to K-MOD-7's not-exposed list;
    - it replaces leg (5)'s marker comparison with a reference census: the kernel paths that `boyko_mod_api` and `boyko_mod_host` name must equal the marked inventory.
  - Cost to a non-modding build: 0.
- **Carried in:** 05 §5; P47.2 (K-MOD-7).

**W5: the ledger still specifies rung 1 with the mechanisms pass 1 blocked** (`:4058-4089`).
- **Disposition:** rung B1, ledger rev 5.
  - It restates KF-33 (`RUNTIME-DATA-LEDGER.md:1446-1457`), KF-34 (`:1459-1470`), `ledger/pool-utils-log.md:18` and the order-of-work row (`:1783`) to P2/P10/P16/P17 and to rungs D-M2/D-M3.
  - It names **this design authoritative for mechanism and the ledger for row inventory**. That adopts pass 6's second "What is needed" item.
- **Carried in:** 02 §2, B1.

**O1: stale passages that the replace-don't-annotate rule still misses** (`:4093-4099`).
- **Disposition:** **fixed in place.**
- **Carried in:** P51.

**O2: the one-id-per-element-type rule has a shipped mechanism that the design's own client does not use** (`:4101-4105`).
- **Disposition:** **void under U-2.**
  - Scratch cohorts are registry-free and use zero `ComponentId`s. So `TraversalScratch` builds its columns with `ScratchColumn::for_type`, and it needs no `register_new`, no memo from `register_asset_layout` and no id at all.
- **What stays:** P43.1's layout-token **finding** (`:3777`). It is the fact U-2 rests on (01 R-B).
- **What is superseded:**
  - P43.1's one-id-per-type **rule** (`:3779`);
  - 43.2's two `TraversalScratch` ids (`:3795`);
  - P39.4's two `register_new` calls (`:3679`).
- **The stagger exception** (`:3781`) becomes KC-10's stagger policy: one process-global `NEXT_STAGGER`, consecutive cache lines within a cohort, measured by MQ-16. Its render example is now a D-S2 client of `for_type`: one `u32` id shared across `counts`, `offsets` and `cursors` (`mesh_draw.rs:446-448`, as pass 6 read it).
- **Carried in:** 00 U-2; 01 KC-10, R-B; 02 D-S2.

**Question 4: P40 says a reader reaches an id only through `DYN_NAMES`, but `LAYOUTS` scanners see a layout before its kind** (`:4155`).
- **Disposition:** D-S1(i)'s cut.
  - The cut lists every `LAYOUTS` scanner (`is_type_registered_as_component`, `id_space_census`, the reflect enumerations).
  - It confirms that none of them acts on the kind. If one does, it scopes P40's sentence to readers that obtain the id through the name table.
  - P40 is unchanged until then.
- **Carried in:** 02 §2, row Q4.

**Question 5: is `remove_component_type` class A or class B?** (`:4156`).
- **Disposition:** **void.** The function is deleted (U-10), and P47.2 removes K-MOD-10.
- **Carried in:** 00 U-10; 05 §4.

**Pass 6's answers to the architect's pass-6 questions** (`:4152-4154`) are also recorded:
1. The dependency on the reflect merge is correct, and its date is the owner's call.
2. See P47.4.
3. The inherited-limit paragraph of P38.1 cites `g17` and its RED-by-design status, so the paragraph is deleted when the lane flips that test. This is a standing obligation, and rev 2.5 does not change it.

---

## P51 — pass-6 O1: the stale passages, fixed in place

**Depends on:** P5, P16 (the rename `ChunkCache` → `SlotChunks`), P17/17.5, P18/18.1 (b), P29, P34, P37/4, P44/O2; pass 6, O1 (`:4093-4099`).

**Where the passages are.** Pass 6 cited two of them at `:1870` and `:1207`. On this tree, as at `b716a5dc`, those sentences are at `:1877` and `:1214`: the review read a pre-commit working copy that ran seven lines low (plan 00 §9 V-61). Each fix strikes the stale words in place and writes the correction beside them.

**1. P17/17.5's live loom bullet** (`:1877`). P34 site 10 had edited the text that P17 had already removed (`:1873`), so this sentence was never corrected.
- **Removed:** `` `Heap` and `FrameArena` genuinely have no cross-thread protocol (single owner, P15/P19) and stay unmodelled, with that stated as a property of their ownership rather than of their field types. ``
- **Added (in place):** No class outside the claim protocol, the carve and the injector is left with a cross-thread protocol to model. The Frame class is deleted (P34), and the Heap is not built (U-1). The Heap's revival form keeps its single-owner argument (HV-3, P15/P19), stated as a property of its ownership rather than of its field types.

**2. P18/18.1's (b) canary** (`:1904`).
- **Removed:** `any of the four`
- **Added (in place):** "any symbol of P29's pinned set": four after P49. The set is named by P29's rule, not by a count, so a later strike or addition cannot make this canary stale again.

**3. P37/4** (`:3287`).
- **Removed:** `` `VmColumn<T: Copy>` ``
- **Added (in place):** `` `VmColumn<T: ZeroInit>` ``. P5 made `ZeroInit` the struct bound; `Copy` remains only on the by-value methods.

**4. K-MOD-7** (`:1214`), and its live replacement, P34 site 14's Added cell (`:3173`).
- **Removed:** `` `ChunkCache` `` in both places. At `:1214`, also `` `FrameArena` `` and `` `Heap`/`HeapVec`/ ``.
- **Added (in place):**
  - At `:1214`: `` `SlotChunks` `` (P16's rename, now named where the row is read). `FrameArena` is struck (P34). The sentence becomes "a mod gets … columns" (U-1: mods use engine storage forms, 05 §4).
  - At `:3173`: the rename is given in the site-number cell, because the Added cell is a code span.

**5. The two rows labelled (f)** (`:1909`, `:3830`).
- **Removed:** the second label, `(f)`.
- **Added (in place):** `(g)`. See P47.4.

---

## Change log (rev 2.4 → rev 2.5)

| Row | Disposition | Where |
|---|---|---|
| Plan U-1, U-9 | The Heap class, with `HeapDyn`, becomes a revival form. Its clients go to KC-15/17/18/10, and `DynVTable` survives as KC-17's vtable. Its gates are not built (C1's Miri test, HV-1, HV-4, `hv3-owner`, `max-bytes`, the run-table counter, the sanitize map, `LINE0_PAD`, M-A3/M-A4). Its trigger is U-1's | P46 |
| Plan U-7 | `TableSet` becomes a revival form; its clients go to KC-18, KC-07 and KC-06. The `Table` commit owner survives | P46 |
| Plan U-8 | `DropColumn` becomes a revival form; KC-16 over KC-10's registry-free constructor replaces it. No trigger is named | P46 |
| Consequences | `CommitOwner` has three owners (Column, Chunk, Table). Rungs 1d, 1e, 2 and 3, §5.1, P30/O1 and P14 step 3 are re-pointed | P46.3 |
| Pass-6 W3 | Void under U-1; carried with the revival form, together with pass 6's three repairs | P46.4 |
| §7 | Re-pointed to the plan's file 05. K-MOD-3 and K-MOD-10 are removed; K-MOD-1 and K-MOD-2 are withdrawn with U-1; the rest map to 05's rows. `boyko_modding` becomes `boyko_mod_host` / `_api` / `_registry` | P47 |
| G6 (U-11) | G6 = UG-15, mapped row by row. The duplicate (f) is relabelled (g) and answered by leg (7); leg (6) keeps feature unification only (pass-6 W1) | P47 |
| Plan U-19 | §2.0: the thread context belongs to `boyko_threadpool`, and `boyko_memory` keeps G5's `#![no_std]`. §2.0 is restated to what the crate now holds | P48 |
| Plan critic pass 5, W1 (a) | P29: `HeapRef::alloc_cold` is struck, leaving four symbols. Eight count sites are re-pointed: six struck or marked, `:3662` recorded and left as reviewed, `:1904` re-worded by P51 | P49 |
| Pass-6 W2, W4, W5, O2, Q4, Q5 | Respectively: D-S1(i)'s isolated tests; 05 §5 row AP6-W4; B1; void under U-2; D-S1(i)'s cut; void under U-10 | P50 |
| Pass-6 O1 | Five stale passages fixed in place | P51 |
| Convention | Superseded passages are marked in place on their own line, and no line before this Part moved | header (`:5`, `:52`); marker index |
| Unchanged | `ByteColumn`, `ZeroInit`, `ChunkArena`/`SlotChunks`, the injector and its spin, 1f, the content of G1–G5 and G2b, P39 and P40 (mechanisms and gates), P2, P3, P16, P17, P20, P28, P36/O1 | — |

## Open questions for pass 7 (AP7) ⚠ *Rev 2.6: answered by AP7; P55 records the answers*

1. **G5 against `vm.rs`'s fallback arm.** P48 keeps G5's "`#![no_std]` without `extern crate alloc`" for `boyko_memory`, as the plan asks. But the first file of that crate already names `alloc`.
   - **The evidence.** `vm.rs`'s Miri / non-(windows, unix) arm imports `std::alloc::{Layout, alloc_zeroed, dealloc}` under `#[cfg(any(miri, not(any(windows, unix))))]` (`crates/boyko_ecs/src/ecs/memory/vm.rs:39-40`, used at `:176` and `:295`, at `49f2fcfb`). P8/O3's and P15/15.4's Miri constants depend on that arm.
   - **The consequence.** G5 as written can hold only in configurations where that arm is compiled out.
   - **Not ruled here.** The question is outside DOC-1's row, and G5 lands at F2 (03 UG-07).
   - **A candidate** that keeps the guarantee where it matters: `#![no_std]` unconditionally; `extern crate alloc` only under that same `cfg`; and G5's compile check run on the windows and unix targets. AP7 may judge that this belongs to C1's cut instead.
2. **Leg (6)'s text in plan 03 §6** still reads "has equal linked section sizes (P44), and `cargo tree -e features` shows no modding feature" (`03:179`). P47 records leg (6) as feature unification only, per plan 02 §2 (row W1) and 00 §5. Whether 03 §6 should drop the size clause is an edit to a plan file, not to this file.
3. **`DropColumn` has no revival trigger.** U-8 names none, and P46 records that rather than inventing one. AP7 may prefer to close this revival form as "deleted" (the form P34 and P35 used) instead of "not built".
4. **K-MOD-1 and K-MOD-2.** The DOC-1 row names only K-MOD-3 and K-MOD-10 as removed, while 05 §4 withdraws K-MOD-1, K-MOD-2 and K-MOD-3 together under U-1. P47 follows 05 §4, the section §7 is re-pointed to, and records K-MOD-1 and K-MOD-2 as withdrawn with the Heap's revival form, not as deleted.

## Marker index (every in-place change of rev 2.5; no line number moved)

| Patch | Lines (this file) |
|---|---|
| Header | `:1` (title), `:5-7` and `:10` (status: the new status, with the superseded one struck), `:52` (the structure note) |
| P46 | `:117`, `:123`, `:137`, `:140`, `:141`, `:143`, `:154`, `:205`, `:248`, `:259`, `:293`, `:310`, `:327`, `:358`, `:373`, `:405`, `:415`, `:439`, `:573`, `:1069`, `:1159`, `:1284`, `:1285`, `:1288`, `:1473`, `:1957`, `:2059`, `:2372`, `:2652`, `:2653`, `:2654`, `:2657`, `:2665`, `:2703`, `:2707`, `:3101`, `:3167`, `:3172`, `:3174`, `:3175`, `:3176`, `:3187`, `:3271`, `:3711`, `:3743` |
| P47 | `:1198`, `:1202`, `:1208`–`:1216`, `:1218`, `:1220`, `:1907`, `:1909`, `:2048`, `:2049`, `:2466`, `:2659`, `:2920`, `:3597`, `:3618`, `:3828`, `:3830` |
| P48 | `:131`, `:337` |
| P49 | `:2615`, `:2623` (row struck), `:2709`, `:3076`, `:3260`, `:3531`, `:3573` |
| P50 | `:3283`, `:3679`, `:3681`, `:3683`, `:3779`, `:3781`, `:3789`, `:3795`, `:3800`, `:3875`, `:3877`–`:3881`, `:3938`, `:3971`, `:4005`, `:4024`, `:4060`, `:4095`–`:4099`, `:4103`, `:4155`, `:4156`, `:4158` |
| P51 | `:1214` (struck and corrected), `:1877`, `:1904`, `:3173`, `:3287`, `:3830` (relabelled) |

**Files read for this patch** (read-only, except this file):
- In `D:/wt/docs` @ `49f2fcfb`:
  - `docs/memory/ALLOCATOR-DESIGN-SPACE.md`;
  - `docs/unification/UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md`, `-01-KERNEL-CONTRACT.md`, `-02-ORDER-OF-WORK.md`, `-03-GATES.md`, `-05-MODDING-READINESS.md`;
  - `docs/memory/RUNTIME-DATA-LEDGER.md` (`:1-14`, `:1620-1645`, `:1710-1725`);
  - `crates/boyko_ecs/src/ecs/memory/vm.rs` (import and fallback lines only).
- A workspace ripgrep for `ALLOCATOR-DESIGN-SPACE.md:[0-9]` under `docs/`, and for heading-anchor links into this file (none found).

**External sources (read 2026-09-23):**
- `[D Rust std: macro thread_local]` <https://doc.rust-lang.org/std/macro.thread_local.html>: the macro is `std::thread_local`.
- `[D Rust std: LocalKey]` <https://doc.rust-lang.org/std/thread/struct.LocalKey.html>: "values that implement `Drop` get destructed when a thread exits", with the platform caveats, including Windows process exit and fibers. These are the caveats KC-04's restrictions already name (01 §6 item 8).
- `[D Unstable Book: thread_local]` <https://doc.rust-lang.org/beta/unstable-book/language-features/thread-local.html>: `#[thread_local]` on `static` items is feature-gated; tracking issue rust-lang/rust#29594.
- `[D Cargo Book, "Feature unification"]` <https://doc.rust-lang.org/cargo/reference/features.html#feature-unification>: "When a dependency is used by multiple packages, Cargo will use the union of all features enabled on that dependency when building it."
- `[D rustc lint unused_extern_crates]` <https://doc.rust-lang.org/stable/nightly-rustc/rustc_lint/builtin/static.UNUSED_EXTERN_CRATES.html>: `extern crate foo as _` is the way to link a crate "even though it is not otherwise directly referenced". The companion lint `unused_crate_dependencies` <https://doc.rust-lang.org/stable/nightly-rustc/rustc_lint/builtin/static.UNUSED_CRATE_DEPENDENCIES.html> reports an `--extern` dependency "never referenced via `use`, `extern crate`, or in any path".

Status: rev 2.5 written (DOC-1, 2026-09-23); critique pass 7 (AP7) ~~reviews~~ reviewed the rev-2.5 delta and the pass-6 dispositions. ⚠ *Rev 2.6: the pass-7 log and rev 2.6 follow.*

## Critique pass 7 log (AP7, 2026-09-23)

Verdict: CHANGES_REQUESTED, with 0 Critical, 2 Important and 5 Optional remarks. The scope was the rev-2.5 delta and the pass-6 dispositions only (plan 02 §2). The critic read this file uncommitted on `49f2fcfb` and checked it against the main checkout at the same commit (it had no shell, so no diff). Each remark's disposition is listed first; the review follows verbatim. Rev 2.6, the next Part, is the architect's response. ⚠ *Every disposition below points into rev 2.6.*

- [IMPORTANT W1] The O1 fix at `:1904` left P18(b)'s superseded symbol list (`Schedule::run`, `ComponentPool::new`, `ScopeBlock::grow`, `HeapRef::alloc_cold`) standing beside the new "four after P49" canary, so the cell reads as if that list were current. A leg-(2) capture taken from `:1904` would record three of P29's four subjects as absent, and nothing would go red. → *Rev 2.6 (P52): the list is struck in place, and the cell now names P29's set. Resolved.*
- [IMPORTANT W2] P47.4 (`:4503`) says leg (6)'s red control carries pass 6's `#[used]`/ctor ban. P47.4's own argument (`:4483`) shows that an unreferenced modding crate never reaches arm A, so that control cannot go red, and no `syn` check reads modding-crate source. Plan 03 also calls arm A "linked" (`03:177`), where P44 only declares it. → *Rev 2.6 (P53): the claim is withdrawn in place. Arm A is stated as declared-only, and 03's word is corrected in place. The ban is recorded as having no mechanism today, and a candidate (a source census plus an object-section census of the modding crates) is handed to the plan (03 §6) as an open item. Resolved in this file; the mechanism is the plan's to adopt.*
- [OPTIONAL O1–O5] → *Rev 2.6 (P54): all five adopted, as markers:*
  - *O1: P34 site 10 (`:3169`);*
  - *O2: the "five" sites `:3832` and `:3662`, and P49's own census (`:4593`, `:4600`);*
  - *O3: `SparseMap` moves in D-M1 (`:3743`, `:4355`);*
  - *O4: C1 lands KC-01 only, and D-M1 lands KC-03 (`:4529`); K-MOD-7's `raw` names (`:1214`);*
  - *O5: the G-MINT rows point to D-S1(i)'s form (`:3672-3675`).*
- [ANSWERS] The critic's answers to rev 2.5's four open questions are recorded in P55. The answer to question 1 adds a per-target statement to G5 (`:337`).

VERDICT: CHANGES_REQUESTED; CRITICAL=0; IMPORTANT=2

# Architecture review: allocator design rev 2.5 (critique pass 7, AP7)

## Verdict
[ ] APPROVED — the plan is ready for implementation
[X] CHANGES REQUESTED — 0 critical, 2 important, 5 optional. Both important remarks are one-paragraph text fixes, and neither touches a mechanism.

**What I reviewed.** Only the rev-2.5 delta and the pass-6 (AP6) dispositions, per 02 §2. File: `D:/wt/docs/docs/memory/ALLOCATOR-DESIGN-SPACE.md`, uncommitted, on `49f2fcfb`. There is no shell here, so I did not run a diff. Instead I checked the edits against the main checkout `D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md`, which is at the same commit:
- The status line is `:4158` in both files.
- The marker index adds up to exactly 117 lines, as the report says.
- The spot-checked lines (`:131`, `:1159`, `:1877`, `:1904`) keep their positions.

## Remarks

### Critical
None.

### Important

#### W1. The O1 fix at `:1904` leaves the wrong four symbols in the cell it edits, and the new marker makes them read as current
**Where:** P51 item 2 and the marker at `:1904`, P18(b)'s mechanism cell.
**Problem:**
- The canary now reads "any symbol of P29's pinned set … four after P49".
- About 30 words earlier, the same cell still says "The four are chosen …: `Schedule::run`, `ComponentPool::new`, `ScopeBlock::grow`, `HeapRef::alloc_cold`". That list was superseded by P29 (Removed at `:2611`).
- Before rev 2.5 the cell was stale but at least consistent with itself. Now the count "four" sits beside a four-item list, and three of those four items are wrong.
- The report (item 8) left the list alone on purpose. But this is the class AP6 O1 names: a replaced passage that still reads as current. And rev 2.5's own rule (`:4195`) promises a marker wherever this revision supersedes text.

**Consequence:** Suppose someone captures the UG-15 leg-(2) pins at B3 from `:1904` instead of from P29.
- They pin `Schedule::run` and `ComponentPool::new`, which have no inline attribute, so P29 refused them. They also pin `alloc_cold`, which is never built.
- Under 03 §6's rule that the pin list is frozen at B3 with absent candidates recorded, those three would be recorded as absent.
- `grow_rows`, `commit_subregion` and `run_check_ticks_scan` would never be pinned. Leg (2) would lose three of its four P29 subjects without anything going red.

**Confidence:** CONFIRMED for the text (`:1904` against `:2615-2623`). PLAUSIBLE for the path to harm: 03 `:170` and P47.3 row (b) both point readers to P29.
**What is needed:** Mark the cell's symbol list as superseded by P29/P49 where it stands, so that the only set the cell names is P29's.

#### W2. P47.4 says leg (6)'s red control carries AP6's `#[used]`/ctor ban, but P47.4's own argument shows that control cannot go red
**Where:** P47.4, `:4503`, against `:4483`. Also 03 `:184` and `:169`.
**Problem:**
- `:4483` argues, citing the rustc lint documentation (I re-checked the quote), that arm A never loads the unreferenced modding crate. So arm A "carries nothing of the unused crate … except what feature unification changes".
- `:4503` then says AP6's recommended ruling is carried by leg (6)'s red control in 03 §6. That ruling is to forbid `#[used]` statics and ctors in modding crates, "enforced by a `syn` check". The control is "a `#[used]` static in a modding crate".
- These two claims contradict each other. By `:4483`, that static never reaches arm A, so leg (6)'s size comparison stays equal and the red control cannot fire. This is the same point AP6 made at `:4153`: "the gate will not 'find out first'".
- No `syn` check covers modding crates either. Leg (1)'s census scope is the six kernel crates plus `boyko_memory` (03 `:169`).
- 03 `:177` also describes arm A as "linked", while P44 (`:3830`) and P47.4 describe it as declared and never referenced. Those are two different builds.

**Consequence:**
- At the first modding crate, the tester runs a red control that stays green. That is this repo's "canary that cannot fire" failure, and the usual outcome is that the control gets dropped or re-blessed.
- Meanwhile the ban is recorded as "carried" when nothing enforces it. That is the "a check believed to cover more than it does" failure AP6 cited in W1.
- No non-modding game is harmed: an unreferenced crate is not linked.

**Confidence:** CONFIRMED. The contradiction is plain in the text (`:4483` against `:4503`), and the scope is fixed by 03 `:169`.
**What is needed:**
- P47.4 should stop claiming the control carries the ban. Record the ban and the control's fireability as open for the plan (03 §6).
- State which arm-A form is meant: declared-only as in P44, or force-linked as 03 says. Leg (6)'s size clause means different things under each.
- Give the ban a mechanism that reads modding-crate source, or state that it has none.

### Optional

**O1.** Pass-6 O1 item 1 is fixed at `:1877` but not at its cause.
- P34 site 10 (`:3169`) still reads as the latest replacement of §5.3's loom bullet, and it restates the rev-2 "`Heap` and `ChunkCache` have no atomics" sentence that P17 had called the defect.
- The consequence is low: 03 §3 carries the D-M2 claim/release loom model on its own.
- A marker on site 10 saying it edited text P17 had already removed would close the chain.

**O2.** P49's census of "five" misses one live site, and contradicts itself on another.
- `:3832` (P44/O2) says "the churn P18 already manages for five symbols".
- P49 says "From rev 2.5 each one reads four", yet it deliberately leaves `:3662` reading "five".

**O3.** The `SparseMap` move is cited to the wrong rung.
- P46.3 and the `:3743` marker say "rung D-R2d".
- 01 KC-16 (`01:103`) says D-M1 moves `SparseMap`, and D-R2d only retires the `sparse_map.rs:10` KF-02 row. 02's D-M1 row (`02:127`) lists "`SparseMap` ×3".

**O4.** P48's new §2.0 folds KC-01 and KC-03 into one crate description. But C1 is KC-01 only (`02:119`), and KC-03 (`ZeroInit`, `ByteColumn`, `ensure_len_zeroed`, the stack helpers) is D-M1 (`02:127`).
- One clause naming that split would stop C1 from being read as including KC-03.
- In the same area: K-MOD-7 (`:1214`, which P51 edited) still names `raw::{reserve, commit, base}`, while §2.0 and KC-01 name `raw::{reserve, commit_at}`.

**O5.** P39.3's G-MINT rows (`:3672-3675`) carry no pointer to their D-S1(i) form, which is 02 `:300-307` and UG-19.
- Read alone, they still show absolute id 100, the `solo` ignore class, and "the mint spins".
- A pointer marker would not change P39's mechanism or gate, so P39 would still stand "as AP6 reviewed" it. The low consequence is because 02 and UG-19 are what the rung actually reads.

## Answers to rev 2.5's open questions

1. **G5 against `vm.rs`.**
   - The conflict is real, and it has been there since rev 1; P48 did not introduce it. P48's claim holds for the thread context.
   - The fallback arm is not Miri-only. `vm.rs:25` documents it as "Fallback (Miri / wasm32 / exotic)", and `vm.rs:39-40` confirms the `std::alloc` import. So G5's scope has to be stated per target.
   - The candidate fix is sound: `#![no_std]`, plus `extern crate alloc` under the same `cfg`, plus the compile check on windows and unix.
   - Its home is UG-07 (F2, 03 `:16`), not C1. C1 does not add `#![no_std]`. This does not hold C1.
2. **Leg (6)'s size clause at 03 `:179`.** Keep it.
   - By P47.4's own argument, the two arms differ only by feature unification, so the size comparison is how leg (6) observes it.
   - UG-16 (03 `:25`) also names "the modding-arm delta is leg (6)".
   - "Its only role" limits what leg (6) is claimed to see, not how it sees it. See W2 for the arm-A question.
3. **`DropColumn`'s trigger.** Keep it as a revival form with no trigger named. 00 §5 lists `DropColumn` among the revival forms, so relabelling it "deleted" would contradict the binding spec.
4. **K-MOD-1 and K-MOD-2.** Following 05 §4 is correct, because P47.1 makes file 05 authoritative for §7. The DOC-1 row's list of removed K-MOD rows was not meant to be complete.

## Positive (preserve)

- **The no-line-move convention, with a complete marker index.** It protects 24 citing documents. The index count and the line alignment both check out.
- **P46's revival forms.** Each family carries its ruling, its reason (with U-1's numbers re-located to `:3171-3172` and `:2718`), its clients by KC, its trigger, and its design and gates by location. `DropColumn` honestly says "none named". Everything the DOC-1 row says stays does stay: `ByteColumn`, `ZeroInit`, `ChunkArena`/`SlotChunks`, the injector (`:1159` is re-homed only), 1f, the G-ladder, P39/P40's mechanisms, and the P39.4 markers that record only U-2.
- **P46.4 carries W3 with AP6's three repairs and picks none.** The choice correctly belongs to the rung that revives the Heap. The re-located citations (`:3729`, `:744-745`) are right.
- **P48 is consistent with 01 §6.**
  - `ext_ptr`/`ecs_fields`, the 8192 × 64 B `.bss` table and the bitmap all match.
  - The two external citations check out:
    - the `LocalKey` destructor quote;
    - `#[thread_local]` being feature-gated under rust-lang/rust#29594.
  - The ledger lines `:1626`, `:1629` and `:1718` match.
  - The design never placed the thread context before; `grep` finds 0 hits.
- **P49.** Exactly four symbols remain, each with its reason. It matches 03 leg (2), and the canary is now worded by rule rather than by count.
- **P47.2 and P47.3.** The K-MOD rows match 05 §4 row by row, and G6 rows (a)–(e) map to the right UG-15 legs.
- **P50.** Every disposition is where 02 §2 puts it: W2 at `02:300-307`, W4 at `05:252`, W5 at `02:71`, and O2, Q4 and Q5 as ruled.

**Topics walked with no finding:** the three commit owners against KC-01 and UG-04, the HeapDyn → KC-17 sizes, the TableSet → KC-18/07/06 routing, and every external source.

## Open questions for the architect

None beyond W2's arm-A definition: declared-only or force-linked. That question belongs to the plan (03 §6 and P44), not to this file.

Status: rev 2.5 reviewed by critique pass 7: CHANGES_REQUESTED, 0 Critical, 2 Important. Rev 2.6 follows and resolves both Important remarks.

---

# Rev 2.6 (2026-09-23)

# Allocator design — Rev 2.6 (patch against Rev 2.5): closes critique pass 7

**Scope.** Rev 2.6 answers critique pass 7 (AP7; the log above). That covers its two Important remarks (W1, W2), its five Optional ones (O1–O5), and the four answers AP7 gave to rev 2.5's open questions. It adds no allocator mechanism. Four patches:
- **P52:** AP7 W1. P18(b)'s superseded symbol list is struck where it stands.
- **P53:** AP7 W2. Leg (6) does not carry the modding-crate `#[used]`/ctor ban, and arm A is declared-only. The ban has no mechanism yet; a candidate is handed to the plan.
- **P54:** AP7 O1–O5, adopted as markers.
- **P55:** AP7's answers to rev 2.5's open questions, recorded. G5 is stated per target.

**Trees.**
- **Documents:** this file and the plan, on `u/doc-1-2` @ `49f2fcfb`. Rev 2.5 is written but not yet committed beneath this Part; both land in one commit.
- **Code:** `crates/boyko_ecs/src/ecs/memory/vm.rs:20-40`, read-only on `integ/unified` @ `c33d786d`, for P55.
- No cargo command, build or test was run.

**Convention.** Rev 2.5's convention is unchanged:
- each superseded passage is marked where it stands, with a suffix that begins `⚠ Rev 2.6`;
- stale words are struck beside their correction;
- no sentence is deleted;
- no line before this Part moved.

The marker index at the end of this Part lists every line it touched.

---

## P52 — AP7 W1: P18(b)'s superseded symbol list is struck where it stands

**Where.** P18/18.1 (b), the mechanism cell of G6's row (b), at `:1904`.

**The defect** (AP7 W1).
- P51 re-worded the cell's canary by rule ("any symbol of P29's pinned set … four after P49").
- But about thirty words earlier, the same cell still named rev 2.1's four: `Schedule::run`, `ComponentPool::new`, `ScopeBlock::grow` and `HeapRef::alloc_cold`.
- P29 had already replaced that list (Removed at `:2609-2611`, Added at `:2613-2625`), and P49 struck `HeapRef::alloc_cold`.
- So the count "four" sat beside a four-item list, and three of those four items were wrong.

**Removed** (struck in place, `:1904`, verbatim):

> The four are chosen so the pin has a guaranteed subject — **every pinned symbol is `#[inline(never)]` or `#[cold]` at its definition**, because an `#[inline]` function may leave no symbol at all and an absent symbol would read as a skip: `Schedule::run`, `ComponentPool::new` (`component_pool.rs:279`, non-generic), `ScopeBlock::grow` (`block.rs:425`, `#[inline(never)]` — this replaces rev 2's `ScopeBlock::bump`, which is `#[inline]` at `block.rs:368` and may not exist), `HeapRef::alloc_cold` (P15's `#[cold]` page-assign path).

**Added** (the marker beside it):

> The pinned set is P29's, with P49's strike: `ComponentPool::grow_rows`, `ComponentPool::commit_subregion`, `run_check_ticks_scan`, `ScopeBlock::grow`. `Schedule::run` and `ComponentPool::new` carry no inline attribute and are not pinned (P29); `HeapRef::alloc_cold` is never built (U-1).

**Why it matters.**
- 03 §6 freezes the leg-(2) pin list at B3 and records every absent candidate with its reason ("Missing symbols").
- A capture taken from `:1904` instead of from P29 would have pinned two symbols P29 refused and one that is never built. It would have recorded all three as absent, and it would never have pinned `grow_rows`, `commit_subregion` or `run_check_ticks_scan`.
- Leg (2) would then have lost three of its four P29 subjects, and nothing would have gone red.
- After P52, P29's is the only set the cell names. 03 `:170` and P47.3 row (b) already point readers to P29.

The cell's "**A missing symbol is RED, never a skip.**" is not struck, because P29 keeps the rule.

**Cost.** None; this is a text fix.

## P53 — AP7 W2: leg (6) does not carry the `#[used]`/ctor ban; arm A is declared-only; the ban has no mechanism yet

**Where.**
- P47.4's closing paragraph (`:4503`), against P47.4's own argument (`:4483`);
- P44/O2's row (`:3830`);
- plan 03 §6: leg (4) (`03:177`), leg (6) (`03:179`) and the controls row (`03:184`).

### 53.1 The "carried" claim is withdrawn

`:4503` said that leg (6)'s control, "a `#[used]` static in a modding crate", carries pass 6's recommended ruling. That ruling forbids `#[used]` statics and constructors in the modding crate, "enforced by a `syn` check". The sentence is struck in place, and two facts refute it.
- **The control cannot fire.**
  - P47.4 argues, from the rustc lint documentation, that arm A never loads the unreferenced modding crate. So arm A "carries nothing of the unused crate … except what feature unification changes" (`:4483`).
  - A `#[used]` static in that crate therefore never reaches arm A's binary. Leg (6)'s size comparison stays equal, and the control stays green.
  - Pass 6 made the same point at `:4153`: "the gate will not 'find out first'".
- **No check reads modding-crate source.** UG-15 leg (1)'s scope is the six kernel crates, plus `boyko_memory` from C1 on (`03:169`).

A red control that cannot go red is this repository's "canary that cannot fire". A ban recorded as carried by such a control is "a check believed to cover more than it does" (AP7 W2).

### 53.2 Arm A is declared-only

P44's row defines both arms (`:3830`): "arm A declares `boyko_modding` as a dependency and never calls it, arm B removes the dependency". That is the form P47.4 argues from. It is also the only form under which leg (6) measures what 00 §5 assigns it: feature unification alone.
- **Declared-only.**
  - Rustc loads a crate passed with `--extern` only when code references it. `extern crate foo as _` is the documented way to link a crate that is "not otherwise directly referenced" `[D rustc lint unused_extern_crates]`.
  - So the two arms differ only in what Cargo's feature unification changes in the crates both arms link `[D Cargo Book, "Feature unification"]`.
  - Leg (6)'s size clause (`03:179`) is how that difference is observed, and `cargo tree -e features` names it (P55, answer 2).
- **Force-linked** (`extern crate … as _` in the game).
  - Arm A would then also carry the modding crates' own link survivors.
  - That is a different configuration: a game that links modding. Its cost is P2's budget, measured by MD:M-A3 (`03:177`), not a zero-delta leg.
- **03's wording.**
  - Plan 03 §6 leg (4) calls arm A "the modding crates linked and never called" (`03:177`).
  - It cites P44 for the definition, so "linked" is a transcription error, not a second design. It is corrected in place in 03, dated 2026-09-23.
  - With arm A declared-only, leg (4)'s startup equality and leg (6)'s size equality both hold by construction, except for feature unification, which is exactly their subject.

### 53.3 The ban has no mechanism today; a candidate is handed to the plan

Pass 6's recommended ruling stands as a **rule without a gate**. ⚠ *Rev 2.7 (P56.1, AP8 N-W1): restated by its harm as a ban on runtime-invoked code, and reconciled with MS-15 and K-MOD-9 (P56.2). It stays without a gate until the plan adopts P56's candidate (P56.6).*
- It matters only in the configuration of 53.2's force-linked bullet: a game that links a modding crate runs that crate's constructors before `main`, even with no mod loaded. ⚠ *Rev 2.7 (P56.1): too narrow. It matters in every binary that links a modding crate: the modding game under C (`boyko_mod_registry`) and under A and A′ (`boyko_mod_host`), and every mod image (`boyko_mod_api`, at load).*
- Nothing in 03 §6 observes that today.
- Leg (6)'s control in `03:184` is marked in place as unable to fire, so that no tester runs it as a gate. ⚠ *Rev 2.7 (P57, AP8 O2): a control that can fire is proposed for leg (6): (xiii), row (f)'s feature canary (`:1909`).*

**Candidate mechanism.** It is open for the plan (03 §6). This file does not adopt it, because the ban is a modding-crate rule and its gate belongs to UG-15.
- **(a) Source.**
  - Extend UG-15 leg (1)'s `syn` walk to the three modding crates (`boyko_mod_host`, `boyko_mod_api`, `boyko_mod_registry`; 05 §4), with an empty allowlist. ⚠ *Rev 2.7 (P56.3, AP8 N-W1): an empty allowlist is red by construction against 05's sanctioned forms (MS-15, MS-01, option A's probe, K-MOD-9's thunks). Superseded by P56.3: a modding allowlist keyed by the forms F-1 to F-4, with N-1 to N-3 non-admissible, and a walk that also reads macro bodies and `cfg_attr` lists.*
  - Leg (1) already counts `used` and `link_section` attributes (`03:169`), so a `#[used]` static written in a modding crate is red.
  - **Red control:** a `#[used]` static in a modding crate → leg (1) red. ⚠ *Rev 2.7 (P56.5): it cannot discriminate, because MS-15's own static is a `#[used]` static. It is replaced by the pair (a-r)/(a-g), which differ only in the section literal.*
- **(b) Objects.**
  - Extend leg (7b)'s rlib object census (`03:181`) to the modding crates' object members.
  - It is red on any section whose name begins `.init_array`, `.ctors`, `.CRT$XC` or `.CRT$XI`. Those are the sections that hold a load-time constructor on ELF and on Windows. ⚠ *Rev 2.7 (P56.4, AP8 O1): incomplete. It omits the `.CRT$XL*` TLS callbacks and ELF `.preinit_array`. Superseded by P56.4's family, which takes the whole `.CRT$` prefix and every ELF initialization and termination table.*
  - (a) alone cannot see a constructor written through a third-party macro, because leg (1) walks unexpanded source. The `ctor` crate's `#[ctor]` expands to `#[used]` plus `#[link_section = ".init_array"]` on Linux and `".CRT$XCU"` on Windows `[D ctor crate docs]`.
  - **Red control:** `#[ctor]` on a function in a modding crate, in the control branch only → (b) red while (a) stays green. This proves (b) is not redundant with (a).
- **When.** Both run from the first modding crate on, which is when leg (6) stops being N/A (UG-15's status cell, `03:24`).

**Cost.** None at runtime:
- (a) adds three crates to an existing walk;
- (b) adds three crates' object members to an existing census.

### 53.4 What does not change

P47.4's argument, and its answer to pass-6 W1, stand: legs (7) and (7b) see kernel survivors, and leg (6) keeps only feature unification. P44's row keeps its relabel to (g) (P47).

## P54 — AP7 O1–O5, adopted

Each fix is a marker on the line it corrects. None changes a mechanism or a gate.

| AP7 | Line(s) | Marker |
|---|---|---|
| O1 | `:3169` (P34 site 10) | The site edited P11's loom bullet (`:1182`), which P17 had already removed (17.5, `:1873`). The live §5.3 loom text is P17's Added block (`:1877`, with P51's fix). The rev-2 sentence "`Heap` and `ChunkCache` have no atomics", which this row restates, is the sentence P17 called the defect. 03 §3 carries the D-M2 claim/release loom model on its own. |
| O2 | `:3832` (P44/O2); `:3662` (P39.2); `:4593` and `:4600` (P49's census) | "Five" becomes four at `:3832`, the ninth site, which P49's census missed. `:3662` is now marked too. A count marker changes neither P39's mechanism nor its gate, so P39 still stands as pass 6 reviewed it (the same reasoning as O5). P49's census line and its `:3662` item carry the correction. |
| O3 | `:3743` (P42's row, rev-2.5 marker); `:4355` (P46.3) | `SparseMap` moves in **D-M1** (01 KC-16, `01:103`; 02's D-M1 row, `02:127`, "`SparseMap` ×3"). D-R2d only retires its KF-02 row (`sparse_map.rs:10`). |
| O4 | `:4529` (P48's §2.0); `:1214` (K-MOD-7) | §2.0's crate holds KC-01 and KC-03, but two rungs land them. **C1 lands KC-01 only**: the move, `raw::{reserve, commit_at}` and the UG-04 counter (`02:119`). **D-M1 lands KC-03**: `ZeroInit`, `ByteColumn`, `ensure_len_zeroed` and the stack helpers (`02:127`). K-MOD-7's `raw::{reserve, commit, base}` reads as KC-01's `raw::{reserve, commit_at}`. |
| O5 | `:3672`–`:3675` (P39.3, G-MINT-1..4) | These rows are built in D-S1(i)'s isolated form (02 `:300-307`; 03 UG-19), not as written here. Each is the only test in its own binary, with the relative pin `P = next_id + 32` instead of absolute id 100. G-MINT-1 expects `Ok(j)` with `j != k + 1`. G-MINT-3 is not ignored, and its mutation panics out of bounds on `LAYOUTS[512]` instead of spinning. The pointer changes neither P39's mechanism nor its gate. |

## P55 — AP7's answers to rev 2.5's open questions, recorded

| Rev 2.5 question | AP7's answer | Recorded as |
|---|---|---|
| 1. G5 against `vm.rs`'s fallback arm | The conflict is real and has existed since rev 1. P48 did not introduce it, and P48's claim holds for the thread context. The fallback arm is not Miri-only: `vm.rs:25` documents it as "Fallback (Miri / wasm32 / exotic)", and `:39-40` import `std::alloc` under `cfg(any(miri, not(any(windows, unix))))`, both re-read at `c33d786d`. The candidate fix is sound. Its home is UG-07 (F2, `03:16`), not C1, which adds no `#![no_std]`. C1 is not held. | G5 (`:337`) is stated per target, with a marker in place. `#![no_std]` applies on every target, and `extern crate alloc` only under the fallback arm's `cfg`. G5's compile check runs on the windows and unix targets, where "no `Vec`/`Box`/`String`/`format!` can be named" holds in full. It lands with UG-07 at F2. |
| 2. Leg (6)'s size clause (`03:179`) | Keep it. By P47.4's own argument, the two arms differ only by feature unification, so the size comparison is how leg (6) observes it. UG-16 (`03:25`) also states that "the modding-arm delta is leg (6)". "Its only role" limits what leg (6) is claimed to see, not how it sees it. | Closed, with no plan edit. The arm-A definition it depends on is P53.2. |
| 3. `DropColumn`'s trigger | Keep it as a revival form with no trigger named. 00 §5 lists `DropColumn` among the revival forms, so calling it "deleted" would contradict the binding spec. | Closed; P46 is unchanged. |
| 4. K-MOD-1 and K-MOD-2 | Following 05 §4 is correct, because P47.1 makes file 05 authoritative for §7. The DOC-1 row's list was not meant to be complete. | Closed; P47 is unchanged. |

## Change log (rev 2.5 → rev 2.6)

| Row | Disposition | Where |
|---|---|---|
| AP7 W1 | P18(b)'s superseded symbol list is struck in place; the cell names P29's set | P52; `:1904` |
| AP7 W2 | The "carried by leg (6)" claim is withdrawn, and arm A is stated as declared-only (P44). 03's "linked" is corrected in place, and 03's leg-(6) control is marked as unable to fire. The ban is a rule without a gate, and its candidate mechanism, (a) source plus (b) objects, is open for the plan | P53; `:4503`; `03:177`, `03:184` |
| AP7 O1 | P34 site 10 edited already-removed text; marker | P54; `:3169` |
| AP7 O2 | "Five" corrected at `:3832` and `:3662`; P49's census corrected | P54; `:3832`, `:3662`, `:4593`, `:4600` |
| AP7 O3 | `SparseMap` moves in D-M1, not D-R2d | P54; `:3743`, `:4355` |
| AP7 O4 | C1 = KC-01 only, and D-M1 = KC-03; K-MOD-7's `raw` names | P54; `:4529`, `:1214` |
| AP7 O5 | G-MINT rows point to D-S1(i)'s isolated form | P54; `:3672`–`:3675` |
| AP7 answers 1–4 | G5 is stated per target (UG-07, F2); leg (6)'s size clause is kept; `DropColumn` stays a revival form; K-MOD-1/2 follow 05 §4 | P55; `:337` |
| Unchanged | Every mechanism and gate of rev 2.5, including P39 and P40 as pass 6 reviewed them | — |

## Marker index (every in-place change of rev 2.6; no line number moved)

| Patch | Lines (this file) |
|---|---|
| Header | `:1` (title), `:5` (status: the rev-2.6 status, with rev 2.5's struck) |
| P52 | `:1904` |
| P53 | `:4503` |
| P54 | `:1214`, `:3169`, `:3662`, `:3672`, `:3673`, `:3674`, `:3675`, `:3743`, `:3832`, `:4355`, `:4529`, `:4593`, `:4600` |
| P55 | `:337`, `:4736` (rev 2.5's open questions, marked answered) |
| Rev 2.5's closing status | `:4774` |

**Also edited in place, outside this file** (plan 03, same-line edits, dated 2026-09-23): `03:177` (arm A is declared-only) and `03:184` (leg (6)'s control cannot fire; the ban's mechanism is open).

**Files read for this patch** (read-only, except this file and the two 03 lines):
- in `D:/wt/docs` @ `49f2fcfb`: this file, and `docs/unification/UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md`, `-01-KERNEL-CONTRACT.md` (`:103`), `-02-ORDER-OF-WORK.md` (`:91-98`, `:119`, `:127`, `:300-312`) and `-03-GATES.md` (`:16`, `:24-25`, `:169-184`);
- in `D:/wt/joltab`, read-only through `git show c33d786d:`: `crates/boyko_ecs/src/ecs/memory/vm.rs` (`:20-40`).

**External sources (read 2026-09-23):**
- `[D rustc lint unused_extern_crates]` and `[D Cargo Book, "Feature unification"]`: as in rev 2.5.
- `[D ctor crate docs]` <https://docs.rs/ctor/latest/ctor/>: `#[ctor]` is implemented as a `#[used]` static with `link_section = ".init_array"` on Linux, `".CRT$XCU"` on Windows (GNU and MSVC) and `"__DATA,__mod_init_func"` on Apple targets. The crate states that it "explicitly subverts" Rust's rule that nothing happens before `main`.

Status: rev 2.6 (2026-09-23). AP7's W1 and W2 are resolved and O1–O5 are adopted; no owner ruling is needed. One item is open for the plan, not for this file: the mechanism of the modding-crate `#[used]`/ctor ban (P53.3). C1's prerequisite, "AP7 has closed rev 2.5" (02 §2), is the orchestrator's to declare, either by ruling (as for pass 6) or after a re-review scoped to this Part. ⚠ *Rev 2.7: critique pass 8 (AP8) reviewed this Part; the pass-8 log and rev 2.7 follow. The open item named here is restated by P56.*

## Critique pass 8 log (AP8, 2026-09-24)

Verdict: CHANGES_REQUESTED, with 0 Critical, 1 Important and 3 Optional remarks. AP8 was a closure pass. The critic checked AP7's seven remarks and four answers against rev 2.6, and read rev 2.6's own new text for new defects. Every AP7 remark and answer is RESOLVED, and AP8 finds C1's prerequisite ("AP7 has closed rev 2.5", 02 §2) met. Its one Important remark concerns text that rev 2.6 added in P53.3. Under 02 §1 rule 8 (`02:23-25`, "A reopened item holds only the rungs that rest on it") it holds only the first modding-crate rung (Stage 3), not C1. The critic read this file on `c1e9f1db`, where it is byte-identical to `4db26681`. Each remark's disposition is listed first; the review follows verbatim. Rev 2.7, the next Part, is the architect's response. ⚠ *Every disposition below points into rev 2.7.*

- [IMPORTANT N-W1] P53.3's ban (`:4997`) and its candidate (a) (`:5003-5006`, leg (1) over the three modding crates with an empty allowlist) contradict the plan's own modding exports:
  - MS-15's `#[used] #[unsafe(no_mangle)] #[unsafe(link_section = ".boykom$m")]` statics, the `.boykom$a`/`$z` sentinels and the `extern "C"` wrappers;
  - option A's `no_mangle` probe;
  - MS-01's `extern "C"` entry;
  - K-MOD-9's C thunks.

  Adopted as written, (a) is red by construction at the first modding crate, and its red control cannot discriminate. → *Rev 2.7 (P56): the ban is restated by its harm: runtime-invoked code, meaning an entry in a table that the OS loader or the C runtime walks, or the `DllMain` entry name. It is reconciled with MS-15, K-MOD-9 and every sanctioned export (P56.2). Candidate (a) is restated with a form-keyed allowlist and non-admissible forms (P56.3). Its controls are the pair (a-r)/(a-g), which differ only in the section literal (P56.5). Resolved in this file; adopting the mechanism stays the plan's, at the first modding-crate rung (P56.6).*
- [OPTIONAL O1] (b)'s section list (`:5009`) omits the `.CRT$XL*` TLS callbacks and ELF `.preinit_array`. → *Rev 2.7 (P56.4): the list becomes one family that (a) and (b) share. Both omissions are added, the `.CRT$` prefix is taken whole, and each member is cited. Marker at `:5009`.*
- [OPTIONAL O2] Leg (6) has no red control since `03:184` struck its only one. → *Rev 2.7 (P57): row (f)'s canary (`:1909`) is proposed to the plan as leg (6)'s red control (xiii), in the form that the feature resolver guarantees red. Marker at `:1909`.*
- [OPTIONAL O3] G5's per-target form (`:337`) has no pointer at its gate row, `03:16` (UG-07). → *Rev 2.7 (P57): proposed to the plan as a same-line pointer at `03:16`. `:337` is unchanged.*
- [RELEASE] AP8 finds that C1 may be cut, and holds only the first modding-crate rung on N-W1. → *Recorded. Declaring C1's prerequisite met is the orchestrator's (02 §2).*

VERDICT: CHANGES_REQUESTED; CRITICAL=0; IMPORTANT=1

**C1 may be cut.** All 7 AP7 remarks and the 4 answers are resolved, so C1's prerequisite ("AP7 has closed rev 2.5", 02:119) is met. C1's own design item is also confirmed closed:
- §2.0 / KC-01 (`:4529`) matches 01:78 and 02:119.
- G5 lands at F2, not C1 (`:337`).

The one new Important remark (N-W1 below) is about text rev 2.6 added in P53.3. It does not concern C1. Under 02:23-25 ("A reopened item holds only the rungs that rest on it") it holds only the first modding-crate rung (Stage 3).

## AP7 remarks

Lines are in `D:/wt/docs/docs/memory/ALLOCATOR-DESIGN-SPACE.md` unless prefixed with a plan file number.

| AP7 remark | Status | Resolving lines |
|---|---|---|
| W1 | RESOLVED | `:1904`: rev 2.1's four-symbol list is struck. The marker names P29's set as struck by P49 (`grow_rows`, `commit_subregion`, `run_check_ticks_scan`, `ScopeBlock::grow`) and gives the reasons for `Schedule::run`, `ComponentPool::new` and `alloc_cold`. This matches `:2615-2625` and `:4582-4589`. The rule "A missing symbol is RED" is kept. Rationale is in P52 (`:4934-4960`). |
| W2 | RESOLVED | All three parts of "What is needed" are done:<br>• "Carried" claim withdrawn: `:4503` is struck, with the P53.1 argument at `:4969-4978`.<br>• Arm A defined: declared-only, per P53.2 (`:4980-4993`) and P44 (`:3830`). The same line is corrected in 03:177.<br>• Ban has no mechanism, and this is stated: P53.3 (`:4995-5000`); the control is struck and marked non-gating at 03:184; recorded as OPEN in 00:154. |
| O1 | RESOLVED | `:3169` (P34 site 10 marker, pointing to `:1182`, `:1873` and `:1877`) |
| O2 | RESOLVED | `:3832`, `:3662`, `:4593`, `:4600`. The remaining unmarked "five" hits are logs, change logs or removed passages (`:3044` is P32.5, which P38 removed at `:3559-3561`). |
| O3 | RESOLVED | `:3743`, `:4355`. Consistent with 01:103 (KC-16), 01:105 (KC-18) and 02:127. |
| O4 | RESOLVED | `:4529` (C1 = KC-01, D-M1 = KC-03) and `:1214` (`raw::{reserve, commit_at}`). Consistent with 01:78, 01:80, 02:119 and 02:127. |
| O5 | RESOLVED | `:3672-3675`. Each marker matches 02:300-307: own binary, pin `P = next_id+32`, `Ok(j)` with `j != k+1`, `registry_mint_exhaustion.rs` not ignored, out-of-bounds panic on `LAYOUTS[512]`. |
| Answers 1–4 | RESOLVED (recorded) | P55 (`:5034-5041`); G5 stated per target at `:337` (the cfg matches `vm.rs:39-40`); `:4736` marked as answered. |
| Plan rows | Consistent | 02:91, 02:96, 02:119 (prerequisite unchanged); 00:154; 02:139 (leg (6) is N/A until a modding crate exists). The count "nine red, three green" at 03:24 is unaffected, because the struck control was never numbered. |

## New remarks (introduced by rev 2.6)

### 🟡 N-W1. P53.3's ban, and its candidate (a), contradict the plan's own modding exports

**Where:**
- `:4997`: the ban "stands as a rule without a gate".
- `:5003-5006`: candidate (a), "leg (1)'s `syn` walk over the three modding crates, with an empty allowlist".
- 03:184 and 00:154 record the ban as standing.

**Problem:**
- Leg (1) counts `no_mangle`, `link_section`, `used` and non-Rust-ABI fn definitions (03:169).
- The plan puts exactly these attributes in the modding crates:
  - **Option C, a live option** (05:119-125): 05 MS-15 (05:171) is built from `#[used] #[unsafe(no_mangle)] #[unsafe(link_section = ".boykom$m")]` statics. It also uses `.boykom$a`/`$z` sentinels and `extern "C"` wrappers in `boyko_mod_registry` (`MODDING-DESIGN-SPACE.md:1116`, `:1127-1128`, `:1137-1138`, `:1831`).
  - **Option A:** a `#[unsafe(no_mangle)] extern "C"` probe in `boyko_mod_api` (`MODDING:1828`).
  - **MS-01:** `extern "C"` → `boyko_mod_host` (05:156).
  - **K-MOD-9, kept:** all C thunks live in the modding crate (`:1216`, `:4443`, 05:226).
- P53.3's own statement of harm (`:4998`) is code that runs before `main`. A `.boykom$m` data static is not that kind of code.

**Consequence:**
- If option C is chosen at Stage 3, the recorded ban forbids MS-15's registry mechanism.
- If the plan adopts candidate (a) as written, leg (1) is red at the first modding crate under A and under C. Its red control ("a `#[used]` static → leg (1) red") then cannot discriminate.
- This is the P29/P49 "red by construction" class. The usual repair for that class is an allowlist entry, or the gate gets dropped.

**Confidence:** CONFIRMED from the text.

**Holds:** only the first modding-crate rung, not C1.

**What is needed:**
- Scope the ban to its stated harm (pre-`main` code).
- Reconcile it explicitly with MS-15 and K-MOD-9.
- Restate candidate (a) so it is not red on 05's sanctioned exports.

### 🟢 O1. Candidate (b)'s section list is incomplete

**Where:** `:5009`, which says "Those are the sections that hold a load-time constructor on ELF and on Windows".

**Problem:** The list omits:
- `.CRT$XL*` TLS callbacks, which the Windows loader calls with `DLL_PROCESS_ATTACH` before the exe's entry point;
- ELF `.preinit_array`.

**Consequence:** If (b) is adopted as written, a macro-emitted `.CRT$XLB` callback in a modding crate runs before `main`, and both (a) and (b) stay green.

**Confidence:** CONFIRMED, from external sources.

### 🟢 O2. Leg (6) now has no red control

**Where:** 03:184 strikes leg (6)'s only control, while 03:24 still has leg (6) gating from the first modding crate.

**Problem:** The allocator's own row (f) (`:1909`) has a canary that can fire: "enable a non-default feature of a shared dep from the modding crate → red". Feature unification applies to a crate that is declared but never referenced. 03 §6 never adopted this canary.

**Consequence:** Leg (6) would gate with no red-first proof.

**Confidence:** CONFIRMED for the text; PLAUSIBLE for the harm.

### 🟢 O3. G5's per-target form has no pointer at its gate row

**Where:** 03:16 (UG-07, "`boyko_memory`, `boyko_utils` | compile"), which is the row F2 reads.

**Problem:** The per-target form of G5 (P55, `:337`) is not referenced there.

**Consequence:** Low, and the failure is loud: an unconditional `#![no_std]` without the cfg-gated `extern crate alloc` breaks the Miri compile. Nothing fails silently.

## Positive
- The no-line-move convention holds. The 7 new rev-2.6 lines inside `:1-4158` (`:3169`, `:3662`, `:3672`-`:3675`, `:3832`) are exactly the ones the marker index adds.
- P53.2 picks P44's own definition of arm A and fixes 03 to match it, instead of inventing a second build.
- P52 names the pinned set by pointer to P29 and P49 rather than by a count.

Sources:
- [cocomelonc: TLS callbacks before main](https://cocomelonc.github.io/malware/2026/09/05/malware-tricks-66.html)
- [lallous' lab: C/C++ TLS callbacks in Visual Studio](https://lallouslab.net/2017/05/30/using-cc-tls-callbacks-in-visual-studio-with-your-32-or-64bits-programs/)
- [SANS ISC: DLLs & TLS Callbacks](https://isc.sans.edu/diary/32580)

Status: rev 2.6 reviewed by critique pass 8 (AP8): CHANGES_REQUESTED, 0 Critical, 1 Important. Rev 2.7 follows and resolves it.

---

# Rev 2.7 (2026-09-24)

# Allocator design — Rev 2.7 (patch against Rev 2.6): closes critique pass 8

**Scope.** Rev 2.7 answers critique pass 8 (AP8; the log above): its Important remark N-W1 and its three Optional remarks O1–O3. It adds no allocator mechanism and changes no gate that this file owns. Two patches:
- **P56:** AP8 N-W1, and O1's section list. The modding-crate ban is restated by its harm and reconciled with the plan's sanctioned modding exports. Its candidate mechanism is restated so that it is not red by construction and its controls discriminate.
- **P57:** AP8 O1–O3. O2 and O3 go to the plan as proposed same-line patches.

**Trees.**
- **Documents:** this file, on `u/doc-3-4` @ `4db26681` (the `integ/unified` trunk). This file and the plan files are byte-identical to `c1e9f1db`, where AP8 read them, so every line number AP8 cites holds here.
- **Code:** read-only, on the same tree:
  - `crates/boyko_ecs/src/ecs/memory/vm.rs:39-40`;
  - the root `Cargo.toml:168-171`;
  - a search of every `.rs` file under `crates/` for `link_section`, `#[used]`, `.CRT$` and `init_array`, which finds 0 hits.
- No cargo command, build or test was run.

**Convention.** Unchanged from rev 2.5 and rev 2.6:
- each superseded passage is marked where it stands, with a suffix that begins `⚠ Rev 2.7`;
- no sentence is deleted;
- no line before this Part moved.

The marker index at the end of this Part lists every line it touched. **This Part does not edit the plan.** The plan-side changes are proposed in P57, as same-line patches for the plan's own step to apply.

---

## P56 — AP8 N-W1: the modding-crate ban, scoped to its harm and reconciled with the plan's modding exports

**Where.**
- P53.3 (`:4995-5016`): the rule (`:4997-4998`), candidate (a) (`:5003-5006`) and candidate (b)'s section list (`:5009`);
- P47.4's record of pass 6's ruling (`:4503`), and where that ruling came from, rev 2.4's open question 2 (`:3868`);
- K-MOD-9 (`:1216`, `:4443`);
- plan 03 §6's controls row (`03:184`) and 00 §5 (`00:154`), which record the ban as standing.

**The defect** (AP8 N-W1).
- P53.3 carried pass 6's ruling as written: forbid `#[used]` statics and constructors in the modding crates.
- The plan's own modding design puts exactly the attributes that leg (1) counts into those crates:
  - **Option C's MS-15 registry** (`05:171`), all in `boyko_mod_registry` (`MODDING-DESIGN-SPACE.md:1116`, `:1127-1128`, `:1137-1138`, `:1831`):
    - one `#[used] #[unsafe(no_mangle)] #[unsafe(link_section = ".boykom$m")]` static per mod;
    - the `.boykom$a` and `.boykom$z` sentinels;
    - the `extern "C"` wrappers.
  - **Option A's** `#[unsafe(no_mangle)] extern "C"` counter probe, in `boyko_mod_api` (`MODDING:1828`).
  - **MS-01's** `extern "C"` entry, in `boyko_mod_host` (`05:156`; `MODDING:1827`).
  - **K-MOD-9's C thunks**, which that rule itself places in the modding crate (`:1216`, `:4443`; `05:226`).
- Read literally, the rule therefore forbids MS-15's mechanism.
- Candidate (a), with an empty allowlist, is red at the first modding crate under A and under C.
- Its red control ("a `#[used]` static → red") goes red on MS-15's own static as well, so it cannot discriminate. That is the P29/P49 "red by construction" class.
- The rule's own statement of harm (`:4998`) is code that runs before `main`. A `.boykom$m` static is data.

### 56.1 The rule, restated by its harm

**Ruled: runtime-invoked code is forbidden in the three modding crates** (`boyko_mod_host`, `boyko_mod_api` and `boyko_mod_registry`; `05:60`). Runtime-invoked code is code that the OS loader or the C runtime calls on an image's behalf, without a call from the program's own code. There are two routes, and the rule names both:
- **(R1) A runtime-walked table.** An entry in a section that the loader or the C runtime walks and calls. These are the constructor and initializer tables, the TLS-callback table and the matching termination tables; 56.4 lists them.
- **(R2) The DLL entry name.** A function exported as `DllMain`. Microsoft documents it as the DLL entry point: when a process or thread starts or terminates, "it calls the entry-point function for each loaded DLL" `[D Win32: DllMain]`. The system also calls it on `LoadLibrary` and `FreeLibrary`. ⚠ *Rev 2.7 closure (P58.1, AP9 W2): superseded. R2 is the image entry and the runtime's user hooks, table E; `DllMain` is its second hop.*

**The moment.** The rule's load-bearing case is the one that pass 6 and P53.3 named: code that runs before `main` (`:4998`). R1's TLS callbacks also run at every thread attach and detach, and the termination tables run at exit. The rule covers all of them, for three reasons:
- They share the mechanism: the program never calls them.
- They share the harm: they are work that a modding build does with no mod loaded.
- No sanctioned modding form uses any of them (56.2), so the wider statement costs no allowlist entry.

**What the rule does not cover.**
- **A `#[used]` static outside the runtime-walked tables.** `used` "forces the compiler to keep a static item in the output object file" `[D Rust Reference: attributes]`, and that runs nothing. Retained data is priced by P2 and measured (MD:M-C0b, for MS-15), not banned.
- **An exported function that a mod or the host calls explicitly, after `main`.** K-MOD-9's thunks are this case (56.2).
- **A mod's own code.** The mod crate belongs to the mod author. Its initializers are the loader's concern (`MODDING-DESIGN-SPACE.md:566-572`, where the canary is read without executing mod code), not this rule's.

**Where the harm lands.** P53.3 said that the rule matters "only in the configuration of 53.2's force-linked bullet" (`:4998`). That is too narrow, and it is marked in place. The rule applies in three places:
- **Under C:** `boyko_mod_registry` is a real dependency of the modding game binary, which links it whether or not any mod is installed (`05:125`, `:171`). Its boot walk is MS-05's C slot (`05:161`).
- **Under A and A′:** the loader lives in `boyko_mod_host` (`05:124`, `:126`; MS-05, `05:161`), which is linked into the modding executable.
- **In every mod image:** `boyko_mod_api` is linked there, so any code it registers through R1 or R2 runs at each mod's load, before the loader's canary check. `MODDING-DESIGN-SPACE.md:566-567` states the failure: "Loading a `cdylib` runs `ctor` code before any call".

In each case the code runs in P2's configuration, which is either modding built in with no mod loaded, or a mod loaded but not yet validated. Nothing in the program called that code. The non-modding game links none of the three crates, so it is unaffected either way (05 §3.1; P53.2).

### 56.2 Reconciliation with MS-15, K-MOD-9 and the other sanctioned exports

Every form the plan sanctions in a modding crate either runs nothing or is called explicitly after `main`. None of them is in R1 or R2.

| Form | Crate | What it is | Sanctioned by | Who calls it, and when | Under the rule |
|---|---|---|---|---|---|
| **F-1** `.boykom` statics | `boyko_mod_registry`; the `$m` template also in `boyko_mod_api` if MS-02a's registration macro lands there (`05:157`) | the `.boykom$a` and `.boykom$z` sentinels, and the per-mod `.boykom$m` registration static (`#[used] #[unsafe(no_mangle)] #[unsafe(link_section = ".boykom$m")]`) that the SDK's registration macro emits | MS-15 (`05:171`); `MODDING-DESIGN-SPACE.md:1116`, `:1127-1128`, `:1831` | nobody, because they are data. The host's boot walk (MS-05, `05:161`) reads the table between the sentinels and asserts the collected count (`MODDING:1129-1130`, `:1247-1249`). The program calls that walk after `main` | outside the rule. `.boykom` is not a runtime-walked table (56.4). The walk costs 16 B and a zero-iteration loop, which is P2's cost, measured by MD:M-C0b (`05:125`) |
| **F-2** export set | `boyko_mod_registry` | `#[unsafe(no_mangle)] pub extern "C"` wrappers | route 3b (`MODDING:1136-1139`, `:1831`) | mod code, after `main` | outside |
| **F-3** mod-image entries | `boyko_mod_api` | option A's `#[unsafe(no_mangle)] extern "C"` counter probe; the trampoline; the mod entry and the build canary | `MODDING:1828`, `:1832-1833`; `05:124`, `:126` | the host's loader, after it loads the image (MS-05). The canary is read as data, without executing mod code (`MODDING:568-571`) | outside, **except** under the name `DllMain` (R2) ⚠ *Rev 2.7 closure (P58.1): outside; its names begin with `boyko_mod_`, and N-2 refuses every name in E* |
| **F-4** host entries | `boyko_mod_host` | MS-01's `extern "C"` descriptor entry; item 6's system-registration wrapper | MS-01 (`05:156`; `MODDING:1827`); `MODDING:1833` | a mod image, after `main` | outside |
| **K-MOD-9** | the modding crates | "every `#[no_mangle] extern "C"` thunk lives in `boyko_modding`; the kernel exports no C symbol" (`:1216`; `05:226`) | K-MOD-9, kept = UG-15 leg (1) | F-2 to F-4 are its thunks | outside. The rule and K-MOD-9 agree: the thunks sit where K-MOD-9 puts them, and the kernel's leg-(1) allowlist stays empty (`03:169`) |

**MS-15 is the case AP8 names.**
- Its static carries all three attributes that the old reading banned.
- It is still not runtime-invoked code, because no loader or C runtime walks `.boykom$m`.
- The linker merges the `$` groups of a section in alphabetical order. The MSVC documentation describes this for `.CRT` `[D MSVC: CRT initialization]`, and `MODDING:1116-1118` measured it for `.boykom` on windows-gnu. The only code that reads the merged `.boykom` table is the host's own boot walk.
- MS-15 was admitted for C on other grounds: the installer controls the relink, and the boot-time count assertion turns a stripped table into a loud failure (`MODDING:1111-1130`). The rule adds nothing against it.

### 56.3 Candidate (a), restated: the source census over the modding crates

This supersedes P53.3's (a) (`:5003-5006`), which is marked in place. It is still a candidate for the plan (UG-15, 03 §6); 56.6 says who decides.

**Scope.** The three modding crates, each from the rung that creates it. ⚠ *Rev 2.7 closure (P58.4, AP9 W4): plus the SDK's proc-macro crate, if any SDK macro is procedural.*

**What is read.** Leg (1)'s `syn` walk (`03:169`): every item, including nested items and impl items. The modding crates need two additions.
- **Macro bodies.** The SDK's registration macro emits MS-15's per-mod static, so in the modding crate that static exists only as tokens inside a macro body. The walk therefore also runs leg (1)'s (M-a) token matcher (`03:169`) over every `macro_rules!` body and every `quote!`/`quote_spanned!` body in the three crates. Without it, two things would be invisible: a macro template that emits `link_section = ".CRT$XCU"` into every mod, and F-1's own `.boykom$m` template. ⚠ *Rev 2.7 closure (P58.4, AP9 W4): the `quote!` clause cannot apply in these three crates, because a proc-macro crate exports only procedural macros. A procedural SDK macro's crate is scanned with (M-a) and (M-b), and the expansion fixture (b-x) reads what every template emits.*
- **`cfg_attr` lists.** A counted attribute can be written inside `cfg_attr(predicate, …)`. The Reference's grammar makes each listed item an `Attr`, and `Attr` includes the `unsafe ( SimplePath AttrInput? )` form. `cfg_attr` may also nest `[D Rust Reference: conditional compilation, cfg_attr; attributes]`. So the constructor form `#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]` is valid. The walk reads every attribute in a `cfg_attr` list, at any depth, as if its predicate held.

**Counted.** Leg (1)'s set (`03:169`):
- the attributes `no_mangle`, `export_name`, `link_section`, `used` and `linkage`;
- fn definitions with a non-`"Rust"` ABI;
- `global_asm!`, `naked_asm!` and `#[naked]`.

**Pass condition.** The counted set equals the modding allowlist, and no allowlist entry is non-admissible. ⚠ *Rev 2.7 closure (P58.2, AP9 W1): read as the reason model of 58.2 (U, S, and N-1 to N-4 per item and per entry).*

**The modding allowlist.**
- It is a file of its own in the gate crate, separate from the kernel's allowlist, which stays empty (`03:169`).
- Each entry names the crate, the file, the item or macro, and **the form** that sanctions it (F-1 to F-4, 56.2). So the allowlist is keyed by the sanctioned forms, and each entry justifies itself by pointing at one.
- An entry whose item does not have its form's shape is red. The shape is the crate, the construct, the attribute set, and the section literal or ABI.
- It is edited only in an owner-signed commit, like leg (1)'s.

**Non-admissible.** The validator is red on any entry that admits one of these, whatever its stated reason: ⚠ *Rev 2.7 closure (P58.2, AP9 W1): and each is also reported on the counted item itself, whether or not an entry admits it.*
- **(N-1)** a `link_section` whose value is in the runtime-invoked family (56.4), or whose value is not a string literal. `AttrInput` allows `= Expression` `[D Rust Reference: attributes]`, so a computed value is refused rather than evaluated;
- **(N-2)** a `no_mangle` fn, or an `export_name` value, named `DllMain` (R2); ⚠ *Rev 2.7 closure (P58.1, AP9 W2): superseded. N-2 covers every name in table E, and any exported name that cannot be decided from source. (N-4, build scripts and native links, is added in P58.6.)*
- **(N-3)** `global_asm!`, `naked_asm!` or `#[naked]`. Assembly text can place code in any section, and N-1's literal check cannot read it there. No sanctioned form uses them, so admitting one needs a new form and its own ruling.

**What (a) cannot see.** An attribute emitted by a third-party macro. The walk reads the modding crates' own source, not the expansions of their dependencies' macros. That is (b)'s job (56.4). ⚠ *Rev 2.7 closure (P58.3, P58.7, AP9 W2): (b) now also runs a defined-symbol census against table E, and 58.7 restates what is left.*

**Anti-vacuity.** The walk reports how many items and macro bodies it read in each crate. Zero for a crate that exists is RED. ⚠ *Rev 2.7 closure (P58.4): counted per modding crate, including the SDK's proc-macro crate if it exists.*

**Why this is not red by construction.**
- Each sanctioned form is one entry shape, admitted once per item by an owner-signed entry.
- N-1 to N-3 refuse the forms the rule forbids, no matter who signs. ⚠ *Rev 2.7 closure (P58): N-1 to N-4, with N-2 over table E; the admitted names carry the `boyko_mod_` prefix.*
- So the discrimination rests on the section literal and the entry name. It does not rest on the attribute set, which MS-15 shares with a constructor.

### 56.4 Candidate (b), restated: the object-section census, and the family (O1)

This supersedes P53.3's (b) section list (`:5009`), which is marked in place. The mechanism is unchanged: leg (7b)'s census of rlib object members (`03:181`), extended to the modding crates and to section names.

**The runtime-invoked family.** One `const` table in the gate crate. (a)'s N-1 and (b) both read it, so the two cannot drift apart. A section name is in the family if it begins with one of these prefixes or equals one of these exact names:

| Name | Match | Format | Who walks or calls it, and when | Source |
|---|---|---|---|---|
| `.CRT$` | prefix | COFF (msvc and gnu) | The C runtime's tables:<br>• **Initializers.** The CRT's startup calls the pointers between `__xc_a` (in `.CRT$XCA`) and `__xc_z` (in `.CRT$XCZ`), where the compiler puts user initializers in `.CRT$XCU`. It then calls `main`.<br>• **TLS callbacks.** The loader calls the callbacks that follow `__xl_a` (in `.CRT$XLA`), for example in `.CRT$XLB`. It calls them with `DLL_PROCESS_ATTACH` before the entry point, and again at thread attach and detach | `[D MSVC: CRT initialization]`; `[S lallouslab: TLS callbacks]`; `[S SANS ISC: DLLs & TLS callbacks]` |
| `.preinit_array` | prefix | ELF | The dynamic linker, in an executable only. It runs these after relocation and before any shared-object initializer; a shared object's copy is ignored | `[D gABI: dynamic linking]`; `[S MaskRay: .init, .ctors, .init_array]` |
| `.init_array`, `.fini_array` | prefix | ELF | The dynamic linker, through `DT_INIT_ARRAY` and `DT_FINI_ARRAY`, at load and at exit. The prefix also matches priority forms such as `.init_array.00099` | `[D gABI]`; `[S MaskRay]` |
| `.ctors`, `.dtors` | prefix | ELF (legacy GNU) | crtend's `.init` calls `__do_global_ctors_aux`, which calls the constructors in `.ctors`. crtbegin's `.fini` does the same for `.dtors` | `[S MaskRay]` |
| `.init`, `.fini` | exact | ELF | the `DT_INIT` and `DT_FINI` code | `[S MaskRay]` |

**What O1 found, and why the COFF row is a prefix.**
- P53.3 listed `.CRT$XC` and `.CRT$XI` but missed `.CRT$XL*`, the TLS callbacks.
- It listed `.init_array` and `.ctors` but missed `.preinit_array`.
- Both are enumeration misses. So the COFF row now takes the whole `.CRT$` prefix:
  - the `.CRT` section holds the C runtime's ordered tables `[D MSVC: CRT initialization]`;
  - no sanctioned modding form places anything in it (56.2), so matching the whole prefix costs nothing and needs no list of groups.
- The ELF rows cover every initialization and termination mechanism that gABI defines (`DT_INIT`, `DT_FINI`, `DT_INIT_ARRAY`, `DT_FINI_ARRAY` and `DT_PREINIT_ARRAY`), plus GNU's legacy `.ctors` and `.dtors`.
- Apple's `__DATA,__mod_init_func` `[D ctor crate docs]` is out of scope, because the target platforms are Windows and Linux (CLAUDE.md, "Target platform").

**(b) itself.**
- **What is read.** The section headers of every object member of each modding crate's rlib. They are read with `llvm-objdump --section-headers`, which will "display summaries of the headers for each section" `[D LLVM: llvm-objdump]`; it is from the same LLVM tool family that leg (7b) uses. Metadata members (`lib.rmeta`) are skipped, as in leg (7b).
- **Which build.** `[profile.seam-census]` (`03:180`):
  - the host-side crates (`boyko_mod_host` and `boyko_mod_registry`) are read as built for leg (6)'s arm A, with the features the game sees;
  - `boyko_mod_api`, which the host never links (`MODDING:1828`), is built on its own with `-p`.
- **Red:** any section whose name is in the family.
- **Anti-vacuity.** The number of members read in each crate is reported, and zero is RED. A member that lists no section is RED.
- **Objects, not the linked image.** Every linked image already carries entries in the family that the runtime owns:
  - the MSVC CRT defines `__xc_a` in `.CRT$XCA` and `__xc_z` in `.CRT$XCZ` `[D MSVC: CRT initialization]`;
  - on `linux-gnu`, std puts its argument-capture hook in `.init_array.00099` (`ARGV_INIT_ARRAY`, `#[used] #[unsafe(link_section = ".init_array.00099")]`) `[D Rust std source: sys/args/unix.rs]`.

  A census of the linked image would therefore be red on every build, which is the P29/P49 class again. The object census sees only what the modding crates themselves emit.
- **Conservative.** (b) reads objects, so it is red whether or not the linker would have kept the entry.
- **Out of scope: instrumented builds** (sanitizers, coverage, `-C profile-generate`). Instrumentation may add module constructors that the source did not write. So (b) runs only on `[profile.seam-census]`, which has no instrumentation.

### 56.5 The controls

Each red control must be red in its own branch, and each names what must go red. The gate reports every reason for a red, and a red control passes only if its own reason is among them. ⚠ *Rev 2.7 closure (P58.2, AP9 W1): superseded. A red control passes only if its reported reason set equals its expected set.*

AP8 asked for a control "that goes red on a real pre-main constructor and green on MS-15's static". That control is the pair (a-r)/(a-g), which differ only in the section literal. ⚠ *Rev 2.7 closure (P58.2, AP9 W1): as written they differed in three things (the literal, `no_mangle` and an entry). The rebuilt pair differs in the literal alone.*

| Control | Branch content | Expected | What it proves |
|---|---|---|---|
| **(a-r)** red | In `boyko_mod_registry`: a `#[used] #[unsafe(link_section = ".CRT$XCU")]` static holding a fn pointer. This is a real pre-main constructor: the CRT calls `.CRT$XCU` entries before `main` `[D MSVC: CRT initialization]` | (a) red, with N-1 among the reasons | (a) sees a constructor written in the crate ⚠ *Rev 2.7 closure (P58.2): superseded; N-1 was unreachable without an entry* |
| **(a-g)** green | In the same crate: the same static shape and the same attributes, plus `#[unsafe(no_mangle)]`, with the literal `.boykom$m`, allowlisted as F-1 | (a) green | The verdict keys on the section, not on the attribute set that MS-15 shares with a constructor. Together with (a-r), this is AP8's discriminating control ⚠ *Rev 2.7 closure (P58.2): superseded; a `$m` item is red on F-1's shape* |
| **(a-c)** red | a `#[used]` static carrying `#[cfg_attr(all(), unsafe(link_section = ".init_array"))]` | (a) red, by N-1 | `cfg_attr` lists are read (56.3) ⚠ *Rev 2.7 closure (P58.2): superseded; `#[used]` was counted on its own, so the control could not see its mutation* |
| **(a-v)** red | one allowlist entry that admits `.CRT$XLB`, and one that admits a `no_mangle` fn named `DllMain` | the validator red, on each | N-1 and N-2 cannot be signed away ⚠ *Rev 2.7 closure (P58.2): extended to table E, the name pattern and N-4* |
| **(b-r)** red | the `ctor` crate's constructor attribute on a fn in the modding crate. It expands to a `#[used]` static in `.init_array` on Linux and `.CRT$XCU` on Windows `[D ctor crate docs]` | (b) red, while (a) stays green | (b) sees what (a) cannot (unchanged from P53.3) |
| **(b-g)** green | the F-1 `.boykom$m` static, compiled | (b) green | `.boykom` is not in the family |
| **(u)** unit test | the family matcher over a table of names.<br>• **Red:** `.CRT$XCU`, `.CRT$XIC`, `.CRT$XLB`, `.CRT$XLC`, `.preinit_array`, `.init_array`, `.init_array.00099`, `.fini_array`, `.ctors`, `.ctors.65535`, `.dtors`, `.init`, `.fini`.<br>• **Green:** `.boykom$a`, `.boykom$m`, `.boykom$z`, `.text`, `.rdata`, `.data`, `.bss`, `.tls$`, `.tbss` | as listed | the family is this table, and every member O1 named is in it ⚠ *Rev 2.7 closure (P58.2): extended with table E's matcher* |

**Under option A** there is no `boyko_mod_registry` and no `.boykom` static to act as the green twin. (a-r) runs in `boyko_mod_host` instead. Its green twin is MS-01's `extern "C"` entry (F-4), which shows that the allowlist admits a sanctioned export while N-1 refuses the constructor. ⚠ *Rev 2.7 closure (P58.2): superseded. MS-01's entry is not a twin that differs in one variable; under option A the pair (a-n)/(a-n′) carries the discrimination.*

**When.**
- From the rung that creates the first modding crate, which is also when leg (6) stops being N/A (`03:24`).
- From then on, on every commit that edits a modding crate or the gate.
- The family table and (u) land with the gate.

### 56.6 What stays open, and who decides

- **Adoption is the plan's.** (a) and (b) remain a candidate for UG-15 (03 §6), as in P53.3. This file does not adopt a gate for a modding-crate rule.
- **Which rung decides.** AP8 holds only the first modding-crate rung (Stage 3) on N-W1 (`02:23-25`). Before it cuts, that rung either adopts (a) and (b) with 56.5's controls, or records a refusal with its reason. C1 is not held.
- **Until then** the rule stands without a gate, as P53.3 said. The difference is that it is now a rule that MS-15 and K-MOD-9 satisfy.
- **Plan text.** P57 proposes new text for `03:184` and `00:154`, so that the plan records the scoped rule instead of "the `#[used]`/ctor ban".
- **Found in passing, not an AP8 item.** Leg (1)'s kernel scope has the same `cfg_attr` gap that 56.3 closes for the modding crates. `03:169` counts an attribute "written plainly or inside Rust 2024's `unsafe(...)`", so `#[cfg_attr(all(), unsafe(no_mangle))]` on a kernel fn is not counted, although the Reference makes it valid (56.3). P57 hands it to the plan as a separate, optional patch group with its own red control (xiv). It touches B3's leg-(1) walker, so whether it lands in B3 or in a later gate edit is the orchestrator's call.

**Cost.** None at runtime, as P53.3 said.
- (a) adds three crates, and their macro bodies, to an existing walk.
- (b) adds three crates' object members to an existing census, and reads one more column (section names) with an LLVM tool that leg (7b) already uses.
- The shared family table and its unit test are new, and small.

## P57 — AP8 O1–O3

| AP8 | Where | Disposition |
|---|---|---|
| O1 | `:5009` | Adopted in P56.4. `.CRT$XL*` (the TLS callbacks) and `.preinit_array` join the family, the COFF row becomes the whole `.CRT$` prefix, and control (u) pins every member. Marker at `:5009`. |
| O2 | `:1909`; plan `03:184`, `03:24` | Adopted. Row (f)'s canary is proposed to the plan as UG-15 leg (6)'s red control **(xiii)**, in the form below. Marker at `:1909`, and at P53.3's `:5000`. |
| O3 | `:337`; plan `03:16` | Adopted, as a pointer on the plan's gate row. `:337` is unchanged. |

**O2: why the canary is guaranteed to fire, and the form that guarantees it.**
- **The resolver.** The root manifest is a package with `edition = "2024"` and no `resolver` key (`Cargo.toml:168-171` @ `4db26681`). Edition 2024 defaults to resolver "3", whose one change from "2" is the default for `resolver.incompatible-rust-versions` `[D Cargo Book: resolver]`.
- **What that implies.** Resolver 2's feature rules apply `[D Cargo Book: features]`:
  - build-dependencies and proc-macros do not share features with normal dependencies;
  - dev-dependencies activate features only for targets that need them;
  - features on platform-specific dependencies for targets not being built are ignored.
- **The form.** The modding crate enables a non-default feature of a **normal, host-matching** dependency that `boyko_demo` also depends on. This is done in the control branch only.
- **Why it fires.** Arm A only declares the modding crate, but that still puts it in the graph. Cargo "will use the union of all features enabled on that dependency" `[D Cargo Book, "Feature unification"]`, and `cargo tree -e features` shows each feature "showing which package enabled it" `[D Cargo Book: features]`. So the tree names the modding crate, and leg (6)'s "shows no modding feature" clause (`03:179`) goes red. ⚠ *Rev 2.7 closure (P58.5, AP9 W3): superseded. Every `feature "default"` edge also names the modding crate, so this criterion is met by the unmodified tree. (xiii) goes red on the per-package difference between the arms.*
- **Which clause the control names.** Leg (6)'s size clause goes red only if the feature changes code that the game links. So the control names the `cargo tree` clause as the one that must go red. ⚠ *Rev 2.7 closure (P58.5): the control names the package and the feature whose line differs between the arms.*
- **When.** From the rung that creates the first modding crate (`03:24`).
- **Which crate.** Row (f) names `boyko_modding`, the single crate that existed before 05 split it into three (`05:60`). The control applies to each of them.

**O3.** `03:16` (UG-07) reads "`boyko_memory`, `boyko_utils` | compile". P57 adds G5's per-target form (`:337`, P55) there as a pointer:
- `#![no_std]` on every target;
- `extern crate alloc` only under the fallback `cfg(any(miri, not(any(windows, unix))))` of `vm.rs` (`crates/boyko_ecs/src/ecs/memory/vm.rs:39-40` @ `4db26681`). C1 moves that file to `boyko_memory` (`02:119`);
- the compile check runs on the windows and unix targets.

AP8 rates the consequence as low and loud: an unconditional `#![no_std]` without the gated `extern crate alloc` breaks the Miri compile.

**Proposed plan patches** (same-line and dated; this Part does not apply them):
- `03:184`:
  - control (xiii) is inserted after (xi);
  - a dated marker scopes the ban and points to P56;
  - the "when the controls run" sentence gains (xiii).
- `03:24`: the red-control count, "nine" → "ten", with (xiii) added.
- `03:16`: UG-07's per-target pointer.
- `00:154`: the open item is restated as the ban on runtime-invoked code, and AP8's outcome is recorded.
- `02:91` and `02:96`: AP8's outcome is recorded in the document-step status and in the AP7 row.
- **Optional group, beyond AP8** (56.6): `03:169` reads `cfg_attr` lists in leg (1)'s kernel scope, and `03:184` gains red control (xiv), `#[cfg_attr(all(), unsafe(no_mangle))]` on a private fn in `boyko_threadpool` → leg (1). With it, `03:24`'s count becomes eleven.

## Change log (rev 2.6 → rev 2.7)

| Row | Disposition | Where |
|---|---|---|
| AP8 N-W1 | The ban is restated by its harm: runtime-invoked code, through a runtime-walked table (R1) or the `DllMain` name (R2). It is reconciled with MS-15 (F-1), the export set (F-2), the mod-image entries (F-3), the host entries (F-4) and K-MOD-9. Candidate (a) is restated with a form-keyed modding allowlist, N-1 to N-3, macro bodies and `cfg_attr` lists. Candidate (b) is restated with the shared family. The controls are (a-r)/(a-g), (a-c), (a-v), (b-r)/(b-g) and (u). Adoption stays the plan's, at the first modding-crate rung | P56; `:1216`, `:3868`, `:4443`, `:4503`, `:4997`, `:4998`, `:5004`, `:5006` |
| AP8 O1 | The family is completed: the whole `.CRT$` prefix, `.preinit_array`, and the termination tables | P56.4, P57; `:5009` |
| AP8 O2 | Row (f)'s canary is proposed as leg (6)'s control (xiii), through a normal, host-matching shared dependency | P57; `:1909`, `:5000`; plan `03:184`, `03:24` (proposed) |
| AP8 O3 | UG-07 gets a pointer to G5's per-target form | P57; plan `03:16` (proposed) |
| AP8 release | C1 may be cut. N-W1 holds only the first modding-crate rung | log; status |
| Unchanged | Every mechanism and gate of rev 2.6, including P52, P53.1, P53.2, P53.4, P54 and P55 | — |

## Marker index (every in-place change of rev 2.7; no line number moved)

| Patch | Lines (this file) |
|---|---|
| Header | `:1` (title), `:5` (status: the rev-2.7 status, with rev 2.6's struck) |
| P56 | `:1216`, `:3868`, `:4443`, `:4503`, `:4997`, `:4998`, `:5004`, `:5006`, `:5009` |
| P57 | `:1909`, `:5000` |
| Rev 2.6's closing status | `:5078` |

**Outside this file:** nothing is edited. P57 proposes same-line plan patches for `03:16`, `03:24`, `03:184`, `00:154`, `02:91` and `02:96`.

**Files read for this patch** (read-only, except this file). All are in `D:/wt/docs` @ `4db26681`:
- this file;
- `docs/unification/UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md` (`:154`);
- `-01-KERNEL-CONTRACT.md` (`:78`);
- `-02-ORDER-OF-WORK.md` (`:23-25`, `:91`, `:96`, `:119`);
- `-03-GATES.md` (`:16`, `:24-25`, `:165-184`);
- `-05-MODDING-READINESS.md` (`:49`, `:60`, `:119-127`, `:156-158`, `:161`, `:171`, `:220-228`);
- `docs/modding/MODDING-DESIGN-SPACE.md` (`:562-572`, `:1100-1145`, `:1242-1250`, `:1827-1833`);
- `crates/boyko_ecs/src/ecs/memory/vm.rs` (`:20-41`) and `Cargo.toml` (`:1-24`, `:160-175`).

**External sources (read 2026-09-24):**
- `[D Win32: DllMain]` <https://learn.microsoft.com/en-us/windows/win32/dlls/dllmain>. `DllMain` is the optional DLL entry point, and "DllMain is a placeholder for the library-defined function name". The system calls it at process and thread start and termination, and on `LoadLibrary` and `FreeLibrary`, with the reasons `DLL_PROCESS_ATTACH`, `DLL_THREAD_ATTACH`, `DLL_THREAD_DETACH` and `DLL_PROCESS_DETACH`.
- `[D MSVC: CRT initialization]` <https://learn.microsoft.com/en-us/cpp/c-runtime-library/crt-initialization>:
  - the CRT's startup code "calls global initializers, and then calls the user-provided `main` function";
  - the compiler puts dynamic initializers in `.CRT$XCU`;
  - the CRT defines `__xc_a` in `.CRT$XCA` and `__xc_z` in `.CRT$XCZ`;
  - the linker combines the `.CRT` subsections into one section and orders them alphabetically.
- `[S lallouslab: TLS callbacks]` <https://lallouslab.net/2017/05/30/using-cc-tls-callbacks-in-visual-studio-with-your-32-or-64bits-programs/>:
  - TLS callbacks execute before `main`;
  - the CRT defines `__xl_a` in `.CRT$XLA`, and user callbacks go in a later group such as `.CRT$XLB`;
  - it lists the four reason values.
- `[S SANS ISC: DLLs & TLS callbacks]` <https://isc.sans.edu/diary/32580>. TLS callbacks run "before the program's normal entry point is reached", and in its DLL example the callback runs before `DllMain`. This source and lallouslab are the two that AP8 cited.
- `[D gABI: dynamic linking]` <https://gabi.xinuos.com/elf/08-dynamic.html>:
  - "The `DT_PREINIT_ARRAY` table is processed only in an executable file";
  - a shared object's copy is ignored;
  - pre-initialization functions run after relocation and before any shared-object initialization functions;
  - it also defines `DT_INIT`, `DT_FINI`, `DT_INIT_ARRAY` and `DT_FINI_ARRAY`.
- `[S MaskRay: .init, .ctors, .init_array]` <https://maskray.me/blog/2021-11-07-init-ctors-init-array>:
  - `DT_INIT` and `DT_FINI` come from `_init` and `_fini`;
  - crtend's `.init` calls `__do_global_ctors_aux`, which calls the constructors in `.ctors`, and crtbegin's `.fini` does the same for `.dtors`;
  - `.preinit_array` is the only mechanism that runs before all DSO initializers.
- `[D Rust std source: sys/args/unix.rs]` <https://doc.rust-lang.org/src/std/sys/args/unix.rs.html>. `ARGV_INIT_ARRAY` sits under `cfg(all(target_os = "linux", target_env = "gnu"))`, with `#[used]` and `#[unsafe(link_section = ".init_array.00099")]`. The source comment says glibc passes `argc`, `argv` and `envp` to `.init_array` functions. *Checked and not cited:* std's Windows thread-local guard. Its current source (`library/std/src/sys/thread_local/guard/windows.rs`) no longer shows a `.CRT$XLB` static, so this Part makes no claim about std on Windows.
- `[D Rust Reference: attributes]` <https://doc.rust-lang.org/reference/attributes.html>:
  - `Attr → SimplePath AttrInput? | unsafe ( SimplePath AttrInput? )`;
  - `AttrInput → DelimTokenTree | = Expression`;
  - the unsafe attributes are `export_name`, `link_section`, `naked` and `no_mangle`;
  - `used` "forces the compiler to keep a static item in the output object file".
- `[D Rust Reference: conditional compilation, cfg_attr]` <https://doc.rust-lang.org/reference/conditional-compilation.html>:
  - `CfgAttrs → Attr ( , Attr )* ,?`;
  - when the predicate holds, `cfg_attr` "expands out to the attributes listed after the predicate";
  - a `cfg_attr` may expand to another `cfg_attr`.
- `[D LLVM: llvm-objdump]` <https://llvm.org/docs/CommandGuide/llvm-objdump.html>. `-h, --headers, --section-headers`: "Display summaries of the headers for each section."
- `[D Cargo Book: features]` <https://doc.rust-lang.org/cargo/reference/features.html>:
  - resolver 2 does not unify features in three cases: build-dependencies and proc-macros, dev-dependencies, and platform-specific dependencies for targets not being built;
  - `cargo tree -e features` shows each feature "showing which package enabled it".
- `[D Cargo Book: resolver]` <https://doc.rust-lang.org/cargo/reference/resolver.html>:
  - resolver "3" is the `edition = "2024"` default, and it changes the `resolver.incompatible-rust-versions` default from `allow` to `fallback`;
  - "only the value in the top-level package will be used".
- `[D Cargo Book, "Feature unification"]` and `[D ctor crate docs]`: as in rev 2.5 and rev 2.6.

Status: rev 2.7 (2026-09-24). AP8's N-W1 is resolved and O1–O3 are adopted; no owner ruling is needed. Two items are open for the plan, not for this file: adopting P56's candidate mechanism for the modding-crate ban on runtime-invoked code, at the first modding-crate rung (AP8 holds that rung on it), and applying P57's proposed same-line patches. AP8 found C1's prerequisite met; declaring it is the orchestrator's. ⚠ *Rev 2.7 closure: superseded by the status at the end of P58.*

---

## Critique pass 9 log (AP9, 2026-09-24)

Verdict: CHANGES_REQUESTED, with 0 Critical, 4 Important and no Optional remarks, plus two open questions. AP9 was a closure pass scoped to rev 2.7's delta.
- It found O1 and O3 resolved.
- It found N-W1 partly resolved and O2's criterion defective.
- It holds the same rung as AP8, and only that one: the first modding-crate rung (Stage 3). C1 is not held, and AP8's release of C1 stands.

Every remark was checked against the text before acting. All four stand, and none could be refuted.

- **[IMPORTANT W1]** (a-r) and (a-c) cannot produce their stated red reason, because N-1 was defined only on allowlist entries (`:5308`) and neither branch adds an entry. The pair (a-r)/(a-g) differs in three things, not one (`:5364` against `:5369`). (a-c)'s `#[used]` is counted on its own, so it cannot see the mutation it exists to catch.
  → *Rev 2.7 closure (P58.2):*
  - N-1 to N-4 are reported per item as well as per entry, so each control's reason is reachable.
  - A red control passes only when its reported reasons **equal** its expected set.
  - F-1's shape is stated (it answers question 2).
  - The pair is rebuilt so that it differs in the section literal alone.
  - A second pair, (a-n)/(a-n′), shows that N-1 keys on the literal under either option.
  - (a-c) now has no counted attribute outside `cfg_attr`.
- **[IMPORTANT W2]** R2 named only `DllMain`, but the loader enters an image through the CRT's entry symbol (`_DllMainCRTStartup` by default). F-3's shape did not constrain the name, and (b) read section names only.
  → *Rev 2.7 closure (P58.1, P58.3):*
  - R2 is restated by mechanism: the image entry and the runtime's user hooks, listed in a closed table E.
  - Every name an F-1 to F-4 entry admits must begin with `boyko_mod_`, so no name in E can be admitted.
  - N-2 refuses every name in E and every exported name that cannot be decided from source.
  - (b) gains a defined-symbol census against E.
- **[IMPORTANT W3]** Leg (6)'s "`cargo tree -e features` shows no modding feature" is met by arm A's unmodified tree (`feature "default"` nodes), or else cannot see (xiii).
  → *Rev 2.7 closure (P58.5):*
  - The clause is restated as a per-package difference: every package in both arms has the same enabled-feature set, read with `cargo tree --format "{p} {f}"`.
  - (xiii) is stated against that clause, and the baseline stays green.
  - Plan patch A3 is applied in that form, and `03:179` gets the clause.
- **[IMPORTANT W4]** A procedural SDK macro cannot live in any of the three crates, because a proc-macro crate "must only export procedural macros". So the `quote!` scan at `:5292` can never apply, and a fourth crate would escape both legs.
  → *Rev 2.7 closure (P58.4):*
  - A procedural SDK macro's crate joins the modding crates of the rule. It is never `boyko_macros`.
  - (a) runs leg (1)'s (M-a) and (M-b) over it.
  - A new leg, (b-x), runs (b)'s section and symbol censuses over a fixture mod crate that invokes every exported SDK macro, with an anti-vacuity check against the exported set.
  - Plan `05:60` gets a pointer.
- **[QUESTION 1]** Build scripts and native libraries.
  → *Rev 2.7 closure (P58.6):* in scope. They are closed by N-4: no build script, no `links` key and no `#[link]` attribute in a modding crate. Control (a-b) covers it.
- **[QUESTION 2]** Which F-1 shape (a-g)'s item matches.
  → *Rev 2.7 closure (P58.2):* none. A `.boykom$m` static written as an item fits neither F-1 shape, so rev 2.7's (a-g) would have been red on shape. The rebuilt pair lives in the registration template, which F-1b admits.
- **[HOLD]** Only the first modding-crate rung. → *Recorded. That rung adopts P56 with P58, or records a refusal (P56.6).*

AP9's review follows verbatim.

VERDICT: CHANGES_REQUESTED; CRITICAL=0; IMPORTANT=4

# Architecture review: allocator design rev 2.7 (closure pass AP9, scope = rev 2.7's delta)

Line numbers without a prefix are in `D:/wt/docs/docs/memory/ALLOCATOR-DESIGN-SPACE.md` on `u/doc-3-4` @ `4db26681`. Plan patches are cited from `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/doc34/doc4.md`. I could not run `git diff` because I have no shell. Instead I read the whole appended Part (`:5192-5506`) and spot-checked the same-line markers at `:1216`, `:1909`, `:3868`, `:4443`, `:4503` and `:4997-5009`. Each is its old text with a suffix added.

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED

**What is held.** The same rung as under AP8: only the first modding-crate rung (Stage 3). C1 is not held, and AP8's release of C1 still stands.

## Status of AP8's remarks

| AP8 | Status | Why |
|---|---|---|
| N-W1 | **Partly resolved** | Done:<br>• The ban is scoped to its harm (56.1).<br>• It is reconciled with MS-15 and K-MOD-9 (56.2, `:1216`, `:4443`).<br>• Candidate (a) is no longer red by construction (56.3).<br>Still open:<br>• The discriminating control AP8 asked for does not discriminate as written (W1).<br>• R2 misses the DLL entry symbol itself (W2).<br>• The macro-template coverage misses procedural macros (W4). |
| O1 | ✅ Resolved | The whole `.CRT$` prefix is covered, `.preinit_array` is added, the family table is shared by (a) and (b), and (u) pins every member (`:5326-5343`, `:5374`). |
| O2 | **Adopted, but the criterion is defective** | See W3. |
| O3 | ✅ Resolved | `vm.rs:39-40` re-read on this tree and it matches. Patch A1 is correct. |

## Remarks

### 🟡 Important

#### W1. The (a-r)/(a-g) pair and (a-c) cannot produce their stated red reason, and (a-c) cannot detect the mutation it exists to catch
**Where:** `:5300`, `:5308-5309`, `:5362`, `:5364`, `:5368-5370`.

**Problem:**
- N-1 is defined as a check on allowlist **entries**: "The validator is red on any entry that admits one of these" (`:5308`).
- (a-r)'s branch adds the `.CRT$XCU` static but no allowlist entry. Yet its expected result is "(a) red, **with N-1 among the reasons**", and `:5362` says a control passes only if its own reason is among the reported ones.
  - With no entry in the branch, N-1 is never reported. The only red reason is "counted item not in the allowlist" (`:5300`), which a `.boykom$m` static without an entry would also trigger.
- `:5364` says the pair "differ only in the section literal". `:5369` contradicts it: (a-g) also adds `#[unsafe(no_mangle)]` and an F-1 entry. So the pair differs in three things, and the claim in its "what it proves" column (the verdict keys on the section) is not established.
- (a-c) is `#[used]` plus `cfg_attr(…, link_section = ".init_array")`. `#[used]` is counted on its own (`:5296`).
  - **Mutant:** delete the walker's `cfg_attr` reading. The static is still red as "unlisted", so a red-only check stays green on the mutant.
  - **Reason-checked:** N-1 is unreachable without an entry, so the control fails on both the correct walker and the mutant.

**Consequence:** At the first modding-crate rung, (a-r) and (a-c) either fail on a correct gate, or pass while proving nothing. That is AP8's own requirement ("red on a real constructor, green on MS-15's static") left unmet, and the repository's recorded "gate that could not see its mutation" class. Proposed control (xiv) (`doc4.md:230`) does not have this defect: `cfg_attr` carries its only counted attribute.

**Confidence:** CONFIRMED from the text: `:5308` against `:5368`, and `:5364` against `:5369`.

**What is needed:**
- Specify each red control's branch so that its named reason can be reached and it differs from its green twin in exactly one variable. For example, (a-r) carries the same F-1 entry and the same attributes as (a-g), and changes only the literal.
- Give (a-c) a subject whose only counted attribute sits inside `cfg_attr`.
- State the expected set of reasons, not just one member of it.

#### W2. R2 names `DllMain`, but the default DLL entry symbol is `_DllMainCRTStartup`, and no leg sees R2 routes emitted by a third-party macro
**Where:**
- `:5248`: R2 is "the DLL entry name".
- `:5310`: N-2 covers only `DllMain`.
- `:5319` ("no matter who signs") and `:5371` ("cannot be signed away").
- `:5275`: F-3's shape does not include the symbol name.
- `:5313`: "That is (b)'s job".
- The `:3868` marker and plan patch A4 (`doc4.md:144`) carry the same list into the plan.

**Problem:**
- Microsoft's `/ENTRY` page says the linker's default DLL entry is `_DllMainCRTStartup`, which "calls `DllMain` if it exists". The executable defaults are `mainCRTStartup`, `wmainCRTStartup`, `WinMainCRTStartup` and `wWinMainCRTStartup`. So the entry the loader actually calls is the CRT symbol, and `DllMain` is a second hop.
- Under 56.3, a `#[unsafe(no_mangle)] extern "system" fn _DllMainCRTStartup` in `boyko_mod_api` has F-3's shape (crate, fn, `no_mangle`, non-Rust ABI). N-1 to N-3 do not refuse it, so a signature admits it.
- (b) reads section names only (`:5346-5350`), so it cannot see any R2 route. A third-party attribute macro that emits `DllMain` is visible to neither leg.

**Consequence:** Under option A on MSVC, an owner-signed F-3 entry for `_DllMainCRTStartup` passes the validator, contrary to `:5319`. If the linker resolves the entry from the rlib before `msvcrt.lib` (rlibs come earlier on the link line), that code runs at every mod's `LoadLibrary`, before the canary check. That is exactly the harm quoted from `MODDING:566-567`.

**Confidence:**
- CONFIRMED: the rule text omits the entry symbol (MS Learn `/ENTRY`).
- PLAUSIBLE: the link silently resolves to the rlib's symbol. A duplicate-symbol LNK2005 is also possible, and that failure would be loud. On windows-gnu, `dllcrt2.o` is a plain object, so a clash there is a loud duplicate.

**What is needed:**
- Define R2 by mechanism (the image entry the loader calls, plus the CRT's user hook), not by one name.
- Make the name refusal unsignable for that whole set, or key F-2 to F-4 entries to a closed name pattern, so that a name-routed hook cannot pass by matching a shape.
- Give R2 an object-level check, as R1 has in (b). Leg (7b) already runs `llvm-nm --defined-only` over rlib members.

#### W3. Leg (6)'s red criterion as written in (xiii) is also met by arm A's baseline
**Where:**
- `:5411`: "So the tree names the modding crate, and leg (6)'s … clause goes red."
- Plan patch A3 (`doc4.md:130`): "…through its `cargo tree -e features` clause, which names the modding crate as the enabler".
- `03:179`: "shows no modding feature".

**Problem:**
- `cargo tree -e features` prints a `feature "default"` node under every package that depends on a crate with default features. Cargo's documentation shows `cfg-if feature "default"` under `log`.
- `boyko_ecs` declares `default = []` (`crates/boyko_ecs/Cargo.toml:53-54`). Every modding crate depends on `boyko_ecs`, because `ModSeam`'s implementors live there (`05:59-60`).
- So arm A's unmodified tree already shows a modding crate "as the enabler" of `boyko_ecs feature "default"`.
- Under the other natural reading ("no feature *of* a modding crate"), (xiii) does not fire, because the feature it enables belongs to the shared dependency.
- Only a per-package **difference** between arms A and B (the resolved feature set of each package both arms contain) keeps the baseline green and turns (xiii) red. Neither `:5411` nor A3 states that difference.

**Consequence:** At the first modding-crate rung, where leg (6) stops being N/A (`03:24`), an implementation written to A3's wording is red on the unmodified tree. That is the red-by-construction class AP8 graded 🟡 for N-W1. The failure is loud, but it blocks the rung until the clause is redesigned.

**Confidence:**
- CONFIRMED: the criterion is met by the baseline (Cargo docs; `Cargo.toml:54`).
- PLAUSIBLE: the modding manifests keep default features. That is Cargo's default, and they are not written yet.

**What is needed:** State leg (6)'s feature clause as the per-package difference between the two arms. Then state (xiii) against that clause, with the baseline required to stay green, and patch A3 to match.

#### W4. The `quote!` part of 56.3's macro-body scan can never apply inside the three crates, so a procedural SDK macro escapes both legs
**Where:**
- `:5292`: M-a runs "over every `macro_rules!` body and every `quote!`/`quote_spanned!` body **in the three crates**". The same line claims this is what makes a template that emits `.CRT$XCU` into every mod visible.
- The scope at `:5289` and the anti-vacuity check at `:5315`.
- 05 names the SDK's registration macro (MS-02a, `05:157`; MS-04, `05:160`) and the trampoline macro (`MODDING:395`, `:1792`) as living in `boyko_mod_api` or `boyko_mod_registry`.

**Problem:**
- The Rust Reference (linkage) says `proc-macro` crates "must only export procedural macros". Each of the three crates exports non-macro items: the sentinels, wrappers, the probe and entries (56.2). None of them can be a proc-macro crate.
- So a procedural registration or trampoline macro can live only in one of two places:
  - **A fourth crate.** Neither (a) nor (b) reads it. (b) reads only the three crates' objects, and the template's output lands in the mod's crate. (a)'s anti-vacuity stays green because it counts per crate of the three.
  - **`boyko_macros`.** Leg (1)'s kernel scope reads it with an empty allowlist (`03:169`), so F-1's template is red by construction. That is the N-W1 class again.

**Consequence:** If the SDK's macros are procedural (the natural form for an attribute that wraps a mod system, or a derive that carries `stable_name`), a template that emits `.CRT$XCU` into every mod image goes through both legs green (silent). The alternative placement is red by construction (loud).

**Confidence:**
- CONFIRMED: the `quote!` clause cannot apply in the three crates (Rust Reference).
- PLAUSIBLE: the SDK macros will be procedural. Their form is undecided until Stage 3.

**What is needed:** Either scope (a) and the form-keyed allowlist to whatever crate holds the SDK's macros, and name that crate beside `05:60`'s three, or state that the SDK's templates must be `macro_rules!`.

## Positive
- The rule is stated by **mechanism**: a table the runtime walks, or an entry name. It is not stated by attribute. 56.2's table gives a caller and a call time for every sanctioned form. That is the right repair for N-W1.
- The `.CRT$` prefix is matched whole instead of enumerated, and one family table is shared by (a)'s N-1 and (b). Together these remove the drift that O1 caught.
- The argument for reading objects rather than the linked image holds: the MSVC CRT's `__xc_a`/`__xc_z` and std's `.init_array.00099` are in every image. This avoids the P29/P49 class. I also confirmed the "checked, not cited" claim: std's current Windows TLS guard has no `.CRT$XL*` static.
- N-1 refuses a `link_section` value that is not a literal. That closes the `concat!` route and the forwarded-metavariable route in `macro_rules!` templates.
- "A member that lists no section is RED" catches rlib members that hold only bitcode.
- Group B's (xiv) is correctly built as a discriminating control, which is the model for fixing W1.
- O2's resolver facts are verified: `Cargo.toml:168-171` has edition 2024 and there is no `resolver` key anywhere.

## Open questions for the architect
1. **Build scripts and native libraries.** A `build.rs` that links a C object with a constructor through `cargo:rustc-link-lib=static:-bundle=…` lands in neither (a), which reads Rust source, nor (b), which reads bundled rlib members. Is that route in scope or out of scope? `:5313` lists only third-party macros as what (a) cannot see.
2. **Which F-1 shape (a-g)'s item matches.** Under C, `boyko_mod_registry` really holds the `$a`/`$z` sentinel items and the `$m` **template**. (a-g) instead puts a `$m` static item in that crate. Does F-1's shape check (`:5305`) admit that item, or does (a-g) go red on shape?

Sources:
- [MS Learn: /ENTRY (Entry-Point Symbol)](https://learn.microsoft.com/en-us/cpp/build/reference/entry-entry-point-symbol)
- [Cargo Book: cargo tree](https://doc.rust-lang.org/cargo/commands/cargo-tree.html)
- [Rust Reference: Linkage (proc-macro crate type)](https://doc.rust-lang.org/reference/linkage.html)
- [Rust Reference: Procedural macros](https://doc.rust-lang.org/reference/procedural-macros.html)
- [rust-lang/rust: library/std/src/sys/thread_local/guard/windows.rs](https://raw.githubusercontent.com/rust-lang/rust/master/library/std/src/sys/thread_local/guard/windows.rs)

Status: rev 2.7 reviewed by critique pass 9 (AP9): CHANGES_REQUESTED, 0 Critical, 4 Important. The rev-2.7 closure (P58) follows and resolves them.

---

# Rev 2.7 closure (2026-09-24): P58, which answers critique pass 9

**Scope.** P58 answers AP9: its four Important remarks, W1–W4, and its two questions. It adds no allocator mechanism, and it changes no gate this file owns. It restates P56's candidate mechanism for the modding-crate ban and P57's control (xiii).

**Trees.** As in rev 2.7: this file on `u/doc-3-4` @ `4db26681`, with code read read-only on that tree. `crates/boyko_ecs/Cargo.toml:53-54` declares `default = []`. No cargo command, build or test was run.

**Convention.** Unchanged:
- every superseded passage is marked where it stands, with a suffix that begins `⚠ Rev 2.7 closure`;
- no sentence is deleted;
- no line before this Part moved.

The marker index at the end lists every line touched. **The plan side is applied in the same Close step** as same-line plan patches, listed at the end. Rev 2.7 had only proposed its patches.

---

## P58 — AP9 W1–W4, and its two questions

### 58.1 R2 by mechanism: the image entry and the runtime's user hooks (AP9 W2)

This supersedes 56.1's R2 (`:5248`), N-2 (`:5310`), and F-3's "except under the name `DllMain`" (`:5275`). Each is marked in place.

**The defect.**
- The loader does not enter a DLL at `DllMain`. It enters at the image's entry point, which the linker takes from `/ENTRY` or from its default.
- Microsoft's table of defaults `[D MSVC: /ENTRY]`:
  - `mainCRTStartup` (or `wmainCRTStartup`) for a console application; it "calls `main` (or `wmain`)";
  - `WinMainCRTStartup` (or `wWinMainCRTStartup`) for a Windows application; it calls `WinMain` (or `wWinMain`);
  - `_DllMainCRTStartup` for a DLL; it "calls `DllMain` if it exists".
- So `DllMain` is the second hop. A `no_mangle` definition of `_DllMainCRTStartup` in `boyko_mod_api` had F-3's shape, and no rule refused it.

**R2, restated.** R2 is code that the OS loader or the C runtime enters **by symbol name** on an image's behalf, rather than through a table walk (R1): an image's entry-point symbol, and the user hooks that the runtime's entry calls. The names form one closed `const` table, **E**, in the gate crate, beside the family table (56.4). (a)'s N-2 and (b)'s symbol census (58.3) both read it. Names are matched exactly.

| Name | Toolchain and role | Who enters it | Source |
|---|---|---|---|
| `_DllMainCRTStartup` | MSVC: the default DLL entry | the loader | `[D MSVC: /ENTRY]` |
| `mainCRTStartup`, `wmainCRTStartup`, `WinMainCRTStartup`, `wWinMainCRTStartup` | MSVC: the default executable entries | the loader | `[D MSVC: /ENTRY]` |
| `DllMain` | the DLL user hook, on MSVC and on MinGW-w64 | `_DllMainCRTStartup` | `[D MSVC: /ENTRY]`; `[S mingw-w64: crtdll.c]` |
| `main`, `wmain`, `WinMain`, `wWinMain` | MSVC: the executable user hooks | the executable entries above | `[D MSVC: /ENTRY]` |
| `DllMainCRTStartup`, `DllEntryPoint` | MinGW-w64 (windows-gnu): the DLL entry, and its second user hook | the loader enters `DllMainCRTStartup`. It calls `__DllMainCRTStartup`, which calls `DllEntryPoint` and `DllMain` | `[S mingw-w64: crtdll.c]` |
| `_init`, `_fini` | ELF: the `DT_INIT`/`DT_FINI` functions. GNU ld sets `DT_INIT` to "the address of the function", and "By default, the linker uses `_init`" (`_fini` likewise) | the dynamic linker, at load and at unload | `[D GNU ld: Options, -init/-fini]`; `[S MaskRay]` |

- **Why the ELF executable entry is not in the table.** A mod image is a shared object, and the dynamic linker enters a shared object only through `DT_INIT` and `DT_INIT_ARRAY` `[D gABI: dynamic linking]`: R2's `_init` above, and R1's family. The host executable's entry comes from the C runtime's startup object, not from a modding crate. ld takes it from `-e`, from the linker script, or from "a target-specific symbol" `[D GNU ld: Entry Point]`. The table lists names that a modding crate could define to be entered; `_start` is left out rather than cited without a source.
- **The closed name pattern for sanctioned exports.** Every exported name that an F-1 to F-4 entry admits must begin with `boyko_mod_`, compared ASCII-case-insensitively. The exported name is decided from source:
  - for a `no_mangle` item, its identifier;
  - for an `export_name`, its value: a string literal, or a `concat!` whose first argument is a string literal. Since Rust 1.54, attributes may invoke function-like macros `[D Rust 1.54 release notes]`, so a template can spell a per-mod name as `concat!("boyko_mod_…", …)`.

  An entry whose exported name does not match the pattern is red on shape (S). No name in E begins with `boyko_mod_`, so no entry can admit one, whoever signs. The only export the modding design names today, `boyko_mod_probe_counters` (`MODDING-DESIGN-SPACE.md:1828`), already matches.
- **N-2, restated** (supersedes `:5310`). An item is non-admissible when its exported name can be decided and is in E. It is also non-admissible when its exported name **cannot** be decided from source, which covers:
  - a `no_mangle` item whose identifier is a template metavariable;
  - an `export_name` value that is neither a string literal nor such a `concat!`.

  The second clause matches N-1's refusal of non-literal section values.
- **F-3, restated** (supersedes `:5275`'s last cell). F-3 is outside the rule. Its names begin with `boyko_mod_`, and N-2 refuses every name in E.

### 58.2 The reason model, F-1's shape, and controls that discriminate (AP9 W1, question 2)

This supersedes `:5300`'s pass condition (it becomes the reason model below), `:5308` ("The validator is red on any entry …"), `:5362`, `:5364`, the rows (a-r), (a-g) and (a-c) at `:5368-5370`, and the option-A paragraph at `:5376`. Each is marked in place.

**Reasons.** The walk reports every reason that applies. A red is the set of them.
- **U:** a counted item has no allowlist entry.
- **S(F-k):** an entry's item does not have its form's shape.
- **N-1 to N-4 (item):** a counted item has a non-admissible property, **whether or not any entry admits it**.
- **N-1 to N-4 (entry):** an entry admits such an item. This is the validator's check, as in 56.3.

The item-level report is the change. It makes N-1 reachable in a branch that adds no entry, which is AP9's first point.

**Pass rule for red controls** (supersedes `:5362`). A red control passes only if the reason set the gate reports **equals** its expected set. "Contains its own reason" is not enough: a mutant that drops or adds a check changes the set, so the control catches it.

**F-1's shape, stated** (answers question 2; refines 56.2's F-1 row):
- **F-1a, the sentinels.** A `static` **item** in `boyko_mod_registry` with `#[used]`, whose `link_section` value is the literal `.boykom$a` or `.boykom$z`, and whose exported name, if any, matches 58.1's pattern.
- **F-1b, the registration template.** A `static` written as **tokens inside a `macro_rules!` body**, in the crate that holds the SDK's registration macro (MS-02a, `05:157`). It carries `#[used]`, `#[unsafe(link_section = ".boykom$m")]`, and an exported name that matches 58.1's pattern.

A `.boykom$m` static written as an **item** fits neither shape. So rev 2.7's (a-g), which put such an item in `boyko_mod_registry`, would have been red on S(F-1). The rebuilt pair below is in the template.

**The controls** (supersede `:5368-5370` and `:5376`; (b-r), (b-g) and (a-v) stand, and (a-v) and (u) are extended below):

| Control | Branch content | Expected reason set (exact) | What it proves |
|---|---|---|---|
| **(a-r)** red, option C | In the SDK's registration template (F-1b), change only the section literal `.boykom$m` to `.CRT$XCU`. The F-1 entry and every other token stay as they are | {N-1 (item), N-1 (entry), S(F-1b)} | The only difference from its green twin is the literal, and the verdict turns on it. The template is the path that would put a constructor into every mod image |
| **(a-g)** green, option C | the unmodified tree, with the template and its F-1 entry | green | (a-r)'s twin. The pair is AP8's discriminating control, now differing in exactly one variable |
| **(a-n)** red, both options | a new `#[used] #[unsafe(link_section = ".CRT$XCU")] static CTOR: fn() = ctor;`, with `fn ctor() {}` beside it and no entry, in `boyko_mod_registry` (option C) or `boyko_mod_host` (option A). `ctor` has the Rust ABI, which leg (1) does not count (`03:169`), so the static is the branch's only counted item | {U, N-1 (item)} | N-1 fires with no entry at all |
| **(a-n′)** red, both options | the same item with the literal `.boykox` | {U} | Together with (a-n), N-1 keys on the literal, not on the attribute set that MS-15 shares with a constructor. Under option A, which has no `.boykom` template, this pair carries the discrimination |
| **(a-c)** red | `#[cfg_attr(all(), used)] #[cfg_attr(all(), cfg_attr(all(), unsafe(link_section = ".init_array")))] static X: fn() = ctor;`, with a Rust-ABI `ctor` beside it and no counted attribute outside `cfg_attr` | {U, N-1 (item)} | `cfg_attr` lists are read, at depth. A walker that skips `cfg_attr` counts nothing, so the control fails. A walker that reads depth 1 only reports {U}, so the control fails too |
| **(a-v)** red, extended | allowlist entries that admit: `.CRT$XLB`; `DllMain`; `_DllMainCRTStartup`; `DllMainCRTStartup`; a name without the `boyko_mod_` prefix; an `export_name` built by a macro that is not `concat!`; a `build.rs` | the validator is red on each, with N-1; N-2; N-2; N-2; S; N-2; N-4 respectively | no non-admissible form can be signed in |
| **(a-b)** red | a `build.rs` in `boyko_mod_api` that prints `cargo::rustc-link-lib=static:-bundle=stub` | {N-4 (item)} | 58.6's route is refused at source |
| **(b-s)** red | a `#[unsafe(no_mangle)] extern "system" fn _DllMainCRTStartup` in `boyko_mod_api`, with no entry | (a): {U, N-2 (item)}; (b): red on the symbol census | R2 is seen at source and in the object (58.3) |

**When.** Unchanged (56.5): from the rung that creates the first modding crate, then on every commit that edits a modding crate or the gate. E's table and its unit test land with the gate.

**(u), extended.** The unit test also runs the E matcher:
- **Red:** every name in E.
- **Green:** `boyko_mod_probe_counters`, `main_loop` and `DllMainHelper`. These show the match is exact, not by prefix.

### 58.3 R2 in the objects: the defined-symbol census (AP9 W2)

This supersedes `:5313`'s "That is (b)'s job" for R2, which (b) could not do. It is marked in place.
- **What (b) reads now.** The same rlib object members, from the same build and profile (56.4), read a second time with `llvm-nm --defined-only --extern-only`. The two flags print "only symbols defined in this file" and "only symbols whose definitions are external; that is, accessible from other files" `[D LLVM: llvm-nm]`.
- **Red:** any such symbol whose name is in E.
- **Anti-vacuity:** as 56.4. The member count is reported, and zero is RED.
- **What it sees that (a) cannot.** An R2 name that a third-party macro emits into a modding crate. Leg (7b) already runs `llvm-nm --defined-only` over rlib members (`03:181`), so the tool is already in the gate.

### 58.4 SDK macros that are procedural: the fourth crate, and the expansion fixture (AP9 W4)

This supersedes `:5289` (the scope), `:5292` (the claim that the `quote!` scan in the three crates makes a `.CRT$XCU` template visible) and `:5315` (anti-vacuity per crate of the three). Each is marked in place.

**The fact.** The Reference says a `proc-macro` crate "must only export procedural macros" `[D Rust Reference: linkage]`. Each of `05:60`'s three crates exports other items (56.2), so none of them can hold a procedural macro.

**The rule.**
- The modding crates of the rule are `05:60`'s three, **plus** the SDK's proc-macro crate if Stage 3 makes any SDK macro procedural. That could be the registration macro (MS-02a), the trampoline, or a derive carrying `stable_name`.
- That crate is named at the rung that creates it; this file uses `boyko_mod_macros` as a placeholder.
- It is never `boyko_macros`. That is the kernel's proc-macro crate, whose leg-(1) allowlist stays empty (`03:169`). Putting F-1's template there would make it red by construction.

**What reads it.**
- **(a)**, over the SDK's proc-macro crate, with leg (1)'s two macro mechanisms exactly as `03:169` runs them for `boyko_macros`:
  - (M-a), the token scan of `quote!`/`quote_spanned!` bodies;
  - (M-b), the expanded corpus: each entry point's `proc_macro2` twin is fed a fixture corpus, and its output is walked.

  (M-b) is what sees computed emission, such as an attribute built with `format_ident!`. The modding allowlist and N-1 to N-4 apply to what both mechanisms report.
- **(b-x), the expansion fixture.**
  - An in-repo fixture mod crate in the gate's test tree invokes every macro the SDK exports, with the fixture inputs. That means every `#[macro_export] macro_rules!` and every `#[proc_macro]`, `#[proc_macro_attribute]` and `#[proc_macro_derive]`.
  - (b)'s section census (56.4) and symbol census (58.3) read the fixture crate's object members.
  - **Anti-vacuity:** the set of exported SDK macros, as (a)'s walk collects it, must be a subset of the macros the fixture invokes. An exported macro with no invocation is RED.
  - This reads, by mechanism, what the templates emit, whatever form they take. That is what `:5292` claimed and could not deliver for a procedural macro.
- **Anti-vacuity of (a)** (supersedes `:5315`): items and macro bodies are counted per modding crate, the fourth one included. Zero for a crate that exists is RED.

**Why not "templates must be `macro_rules!`"** (AP9's alternative).
- It would decide an SDK design question that is open until Stage 3 (MS-02a, MS-04; `05:157`, `:160`).
- Even a declarative template's output is only fully known once expanded with its inputs.
- (b-x) covers both forms by mechanism, at the cost of building one fixture crate in the gate. There is no runtime cost.

### 58.5 Leg (6)'s feature clause, as a per-package difference (AP9 W3)

This supersedes P57's "Why it fires" (`:5411`) and "Which clause the control names" (`:5412`), and the canary marker's "the `cargo tree -e features` clause is the one that must go red" (`:1909`). Each is marked in place. It restates the plan's clause at `03:179`.

**The defect, confirmed.**
- `cargo tree -e features` prints a `feature "default"` node wherever a dependency is used with its default features. The Cargo Book's own example shows `cfg-if feature "default"` under `log` `[D Cargo Book: cargo tree]`.
- `boyko_ecs` declares `default = []` (`crates/boyko_ecs/Cargo.toml:53-54` @ `4db26681`), and every modding crate depends on it (`05:59-60`).
- So arm A's unmodified tree already shows a modding crate enabling `boyko_ecs feature "default"`.
- The two readings of "shows no modding feature" both fail:
  - read as "no feature that a modding crate enables", the clause is red on the unmodified tree;
  - read as "no feature *of* a modding crate", it cannot see (xiii).

**The clause, restated.**
- In each arm, run `cargo tree -p boyko_demo -e normal,build --prefix none --no-dedupe --format "{p} {f}"` for the host target. `{f}` prints the "Comma-separated list of package features that are enabled" `[D Cargo Book: cargo tree]`.
- **Pass:** for every package that appears in both arms' output, its set of lines is identical across the arms. A package may appear more than once under resolver 2, once for the target and once for a build context.
- **Excluded:** packages that appear only in arm A, meaning the modding crates and dependencies only they pull. They are not linked (P47.4, `:4483`), and the size clause covers them.
- **`boyko_demo`'s own line is compared too.** The arms differ by a manifest edge, not by a feature (P44, `:3830`), so it must be equal.
- **Why the baseline stays green.** A modding crate that uses a shared dependency with the same features the game already enables adds nothing to that package's set.
- **The cost of the rule, stated.** A modding crate that asks for a shared dependency's default features where the game turns them off changes that package's set, even when `default` expands to nothing. That is a real difference in the game's resolved graph, so the red is correct and loud. The modding manifests match the game's `default-features` choice on shared dependencies.
- **Unchanged:** the leg is reported as N/A until a modding crate exists (`03:179`).

**(xiii), stated against the clause.**
- In the control branch, the modding crate enables a non-default feature of a normal, host-matching dependency that `boyko_demo` also depends on.
- Expected: leg (6) red, **because that package's line in arm A differs from arm B's by exactly that feature**. The control names the package and the feature, and passes only on that difference.
- The size clause also goes red only if the feature changes code that the game links. That is unchanged from P57.

### 58.6 Build scripts and native libraries (AP9 question 1)

**In scope.** The route is real:
- `cargo::rustc-link-lib` passes `-l` to the package's library target `[D Cargo Book: build scripts]`;
- with `-bundle`, "object files from it are included only during linking of the final binary" `[D rustc book: linking modifiers]`.

So a native object with a constructor reaches the image with nothing in the crate's Rust source for (a) and nothing in its rlib for (b). With `+bundle`, the default, the native objects are rlib members, and (b)'s census of every member would see them.

**Closed by N-4 (non-admissible, new).** No modding crate has any of:
- a build script (a `build.rs` at the crate root, or a `package.build` key);
- a `package.links` key;
- a `#[link(…)]` attribute.

(a) reads each modding crate's manifest and root for them. No sanctioned form (56.2) needs native code or a build script, so N-4 costs no allowlist entry. Admitting one needs a new form and its own ruling, as for N-3. Control: (a-b).

### 58.7 What (a) and (b) cannot see, restated

This supersedes `:5313`. Nothing is left that the rule's harm can reach through a modding crate:
- A third-party macro's R1 emission is seen by (b)'s section census; its R2 emission by (b)'s symbol census (58.3).
- An SDK template's emission, in either form, is seen by (a)'s mechanisms and by (b-x) (58.4).
- A native object is refused by N-4 (58.6).
- Instrumented builds stay out of scope (56.4).
- A mod's own code stays outside the rule (56.1).

### 58.8 What stays open, and who decides

Unchanged from 56.6, restated with P58:
- (a) and (b), now with (b-x), N-4 and table E, remain a candidate for UG-15 (03 §6).
- The first modding-crate rung adopts them with P58's controls, or records a refusal with its reason, before it cuts. AP9 holds only that rung, and C1 is not held.
- Group B, the kernel-scope `cfg_attr` gap (56.6), is applied to the plan in this step. It lands at the first commit that edits leg (1)'s walker after the patch reaches the trunk. It adds no requirement to B3's merge.

**Cost.** None at runtime.
- (a) gains one crate if the SDK has a procedural macro, and a manifest read.
- (b) reads one more column (defined external symbols) from the members it already reads, plus one fixture crate's members.
- Table E and its unit test are small.

## Change log (rev 2.7 → rev 2.7 closure)

| Row | Disposition | Where |
|---|---|---|
| AP9 W1 | Reasons are reported per item and per entry. A red control passes on an **exact** reason set. F-1 is split into F-1a (sentinel items) and F-1b (the registration template). New controls: (a-r)/(a-g) differ only in the literal; (a-n)/(a-n′) work under both options; (a-c) has no counted attribute outside `cfg_attr` | P58.2; `:5300`, `:5308`, `:5362`, `:5364`, `:5368`, `:5369`, `:5370`, `:5376` |
| AP9 W2 | R2 by mechanism, through table E (the entry symbols and the user hooks). Admitted names carry the `boyko_mod_` prefix. N-2 covers E and any name that cannot be decided. (b) gains the defined-symbol census. New control (b-s); (a-v) and (u) are extended | P58.1, P58.3; `:3868`, `:5248`, `:5275`, `:5310`, `:5313`, `:5319`, `:5371`, `:5374` |
| AP9 W3 | Leg (6)'s feature clause is a per-package difference between the arms (`cargo tree --format "{p} {f}"`). (xiii) is stated against it, and the baseline stays green | P58.5; `:1909`, `:5411`, `:5412`; plan `03:179`, `03:184` |
| AP9 W4 | A procedural SDK macro's crate joins the modding crates, and is never `boyko_macros`. (a) runs (M-a) and (M-b) over it. The expansion fixture (b-x) has its own anti-vacuity | P58.4; `:5289`, `:5292`, `:5315`; plan `05:60` |
| AP9 question 1 | N-4: no build script, `links` key or `#[link]` in a modding crate. Control (a-b) | P58.6 |
| AP9 question 2 | F-1's shape is stated. A `$m` item is red on shape, and the pair is rebuilt in the template | P58.2 |
| Unchanged | R1 and the family table (56.4); (b-r), (b-g); every allocator mechanism and gate | — |

## Marker index (the closure's in-place changes; no line number moved)

| Patch | Lines (this file) |
|---|---|
| Header | `:1` (title), `:5` (status: the closure's status first, with rev 2.7's struck) |
| P58 | `:1909`, `:3868`, `:5248`, `:5275`, `:5289`, `:5292`, `:5300`, `:5308`, `:5310`, `:5313`, `:5315`, `:5319`, `:5362`, `:5364`, `:5368`, `:5369`, `:5370`, `:5371`, `:5374`, `:5376`, `:5411`, `:5412` |
| Rev 2.7's closing status | `:5506` |

**Outside this file** (same-line plan patches, applied in this Close step): `03:24`, `03:169`, `03:179`, `03:184`, `00:154`, `02:91`, `02:96` and `05:60`.

**External sources (read 2026-09-24):**
- `[D MSVC: /ENTRY]` <https://learn.microsoft.com/en-us/cpp/build/reference/entry-entry-point-symbol>. Its table of defaults: `mainCRTStartup` (or `wmainCRTStartup`), which "calls `main` (or `wmain`)"; `WinMainCRTStartup` (or `wWinMainCRTStartup`), which calls `WinMain` (or `wWinMain`); and `_DllMainCRTStartup`, which "calls `DllMain` if it exists". Also: "The functions `main`, `WinMain`, and `DllMain` are the three forms of the user-defined entry point."
- `[S mingw-w64: crtdll.c]` <https://raw.githubusercontent.com/mirror/mingw-w64/master/mingw-w64-crt/crt/crtdll.c>. It declares `DllMain` and `DllEntryPoint`, and defines `WINBOOL WINAPI DllMainCRTStartup (HANDLE hDllHandle, DWORD dwReason, LPVOID lpreserved)`, which returns `__DllMainCRTStartup (…)`. That function calls `DllEntryPoint (…)` and `DllMain(…)`.
- `[D GNU ld: Options, -init/-fini]` <https://sourceware.org/binutils/docs/ld/Options.html>. `-init=name`: "call NAME when the executable or shared object is loaded, by setting DT_INIT to the address of the function. By default, the linker uses `_init` as the function to call." `-fini` is the same, with `DT_FINI` and `_fini`.
- `[D GNU ld: Entry Point]` <https://sourceware.org/binutils/docs/ld/Entry-Point.html>. The entry comes from `-e`, from `ENTRY(symbol)`, or from "the value of a target-specific symbol, if it is defined; For many targets this is `start`, but PE- and BeOS-based systems for example check a list of possible entry symbols".
- `[D LLVM: llvm-nm]` <https://llvm.org/docs/CommandGuide/llvm-nm.html>. `--defined-only`: "Print only symbols defined in this file." `--extern-only`: "Print only symbols whose definitions are external; that is, accessible from other files."
- `[D Rust 1.54 release notes]` <https://blog.rust-lang.org/2021/07/29/Rust-1.54.0/>. "Rust 1.54 supports invoking function-like macros inside attributes", for example `#![doc = include_str!("README.md")]`.
- `[D Rust Reference: linkage]` <https://doc.rust-lang.org/reference/linkage.html>. "Crates compiled with this crate type must only export procedural macros."
- `[D Cargo Book: cargo tree]` <https://doc.rust-lang.org/cargo/commands/cargo-tree.html>:
  - the `-e features` example shows `cfg-if feature "default"` under `log`;
  - `--format`'s `{f}` is the "Comma-separated list of package features that are enabled";
  - `--prefix none` shows "a flat list";
  - `--no-dedupe` repeats duplicates.
- `[D Cargo Book: build scripts]` <https://doc.rust-lang.org/cargo/reference/build-scripts.html>:
  - "The `-l` flag is only passed to the library target of the package";
  - the `links` key declares "that the package links with the given native library".
- `[D rustc book: linking modifiers]` <https://doc.rust-lang.org/rustc/command-line-arguments.html>. "When building a rlib `-bundle` means that the native static library is registered as a dependency of that rlib 'by name', and object files from it are included only during linking of the final binary". "The default for this modifier is `+bundle`."
- `[D gABI: dynamic linking]`, `[S MaskRay]` and the rest: as in rev 2.7.

Status: rev 2.7 with its closure (P58), 2026-09-24. AP9's W1–W4 are resolved and both questions are answered; no owner ruling is needed. One item is open for the plan, not for this file: adopting P56 with P58's changes as the modding-crate ban's mechanism, at the first modding-crate rung, which AP9 holds on it and which adopts it or records a refusal. P58's plan side is applied in the same step. C1 is not held (AP8, AP9).
