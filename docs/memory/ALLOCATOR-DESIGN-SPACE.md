# Allocator design space - rev 1 and its first critique

Date: 2026-09-10. Trees: **ecsnative** = `D:/wt/ecsnative` (`feat/ecs-native-storage` @ `ad0ebea4`), **merge** = `D:/wt/merge` (`merge/ke16-into-render` @ `d2c8c646`), **main** = `D:/claude/BoykoEngine` (read-only). Companion: [ALLOCATOR-RESEARCH.md](ALLOCATOR-RESEARCH.md) (the four lens reports this design is built on).

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

Structure of this file: Part I is the architect's rev-1 design reproduced VERBATIM (including its opening orientation note); it has NOT been edited to answer the critique. Part II is the critique log of pass 1, one row per finding with its evidence and the action it takes into rev 2. Provenance tags as in the research file: [S] source read, [D] official doc / paper, [B] blog or talk (recorded, not relied on); in-tree claims carry `[tree] path:line`.

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

- **Address stability under growth is a property of `VmColumn`/`ComponentPool`/`InlandStore` only.** Every site where a pointer is held across a push keeps those primitives: pool rows (`row_ptr`, `[ecsnative] component_pool.rs:816-834`), `ScratchSolveView` copies held by workers (`scratch_column.rs:28-38`), `s2e`, `entity_ids`, the inland store. The new `HeapVec<T>` **relocates on growth** exactly like `Vec` and is therefore allowed only where no pointer into it outlives a push — a `debug_assert!`-free rule enforced by review and by the ledger's `class` column (`heap` sites must state "no pointer held").
- **Third-party runtime allocations are not reached by (i).** The one such site on the frame path is crossbeam-deque (`Injector::push` allocates a 31-slot block; `Worker` doubles; crossbeam-epoch allocates a node per sealed bag `[S deque.rs, internal.rs]`) — replaced by an in-house bounded deque in rung 1d. `std::thread::spawn` is accepted (tag `os-thread`).
- **`String` ergonomics** — `format!` is gone from engine crates; `core::fmt::Write` into a `HeapString`/`LogRing` replaces it.

### Rejected alternatives, closed

- **(ii) as a bridge for droppable/setup-once structures only.** Rejected: the same sites are served by `DropColumn<T>` and `HeapVec<T>` without nightly, and a second container vocabulary would split the codebase in two styles. No `Allocator` impl ships in v1 — it would be client-less, the exact X.J retirement criterion.
- **`allocator-api2` shim on stable.** Rejected: a third-party container as the engine's canonical vector contradicts the in-house rule for engine libraries, and it still needs the newtype for the gate.
- **A process-singleton heap (`&'static`).** Rejected: tests build many `EcsMaster`s concurrently; a single-writer heap shared across them is a data race. Heaps are per owner.

## 2. What the memory library gains

### 2.0 The crate split (architecture decision)

`boyko_memory` — new crate containing `vm.rs`, `vm_column.rs`, `utils.rs`, the granule/page constants, and the four classes below. `boyko_ecs` keeps `component_pool.rs`, `device_column.rs`, `inland_store.rs`, `scratch_column.rs` (they know ticks, ids, and the stagger). Reason: `boyko_threadpool` and `boyko_utils` need reservations and cannot depend on `boyko_ecs` (cycle: `boyko_ecs` → `boyko_threadpool`). `boyko_memory` has no dependencies beyond the OS crates already used by `vm.rs`.

### 2.1 Lifetime classes — the table

| Class | Primitive(s) | Lifetime | Owner / thread rule | Free semantics | Zero-fill | Reset | Address-stable | Resident floor per instance (pre-plan → post-plan) |
|---|---|---|---|---|---|---|---|---|
| **Column** (exists) | `ComponentPool`, `VmColumn<T>`, `InlandStore`, `ScratchColumn` + new `DropColumn<T>`, `ByteColumn` | persistent | single writer (`&mut`), readers anywhere | `len` rollback / `swap_remove` | yes, fresh commit only | none | **yes** | 64 KiB → 4 KiB (lazy: 0 until first push) |
| **Frame** | `FrameArena` + `FrameVec<T>`, `FrameSlice<T>` | one frame between two `reset()` calls at a fixed schedule position | **dispatcher thread only** | none; `mark()/rewind(mark)` LIFO | **no** | `reset()` at frame start | within a frame | 0 until used → high-water mark, page-rounded |
| **Scope** | `ScopeArena` (one per thread slot, `W+1`), replaces `ScopeBlock`'s `std::alloc` chunks and `Box<ScopeShared>` | one `Scope` (nested LIFO) | the thread that opened the scope; indexed by `wid` from the existing single TLS read | `rewind(mark)` at join | **no** | at join | within a scope | 0 until used → HWM; typical ≤ 64 KiB |
| **Heap** | `Heap` + `HeapVec<T>`, `HeapBox<T>`, `HeapDyn<V>`, `HeapString`, `SortedMap<K,V>` | persistent, individually freed | **single writer** (structural mutation = dispatcher-only, EM2 `[ecsnative] entity_master.rs:16`); readers anywhere | size-class intrusive LIFO free lists | **no** (reused) | none | **no** (relocates on grow) | 64 KiB for the whole heap (self-packed at 4 KiB pages) — independent of the packing plan |
| **Table** | `TableSet` — one reservation, N fixed-length typed tables laid out at build | build-once | single writer at build; readers anywhere | none | yes | none | yes | total bytes granule-rounded: 64 KiB → 4 KiB |

Five rows because `TableSet` is a *shape* of the Column class (fixed length, many tables, one reservation) that kills ~20 `Box<[..]>` fields with one granule; it is listed separately because its API differs.

### 2.2 The library's own contract additions (to `VmReservation`)

- `commit` alignment relaxes to `COMMIT_PAGE` — this is packing-plan D1 (`[ecsnative] docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md:90-101`), not a new decision; the Frame/Scope/Heap classes use the page quantum from day one because they pack their own granules.
- **No decommit/reset primitive is added.** Frame and Scope floors are the high-water mark. Reason: Dawson's numbers `[B randomascii 2014]` — ~175 µs/MB first-touch faults + ~150 µs/MB kernel zeroing on release; a per-frame trim would pay that every frame; a periodic trim is a policy with no measured need. Revisit only with a measured resident number.
- `VmColumn::new` drops the `COMMIT_GRANULE % size_of::<T>() == 0` pin (`[ecsnative] vm_column.rs:144-149`). New rule: `committed_elems = committed_bytes / size_of::<T>()` (floor). The pin existed to keep `committed_elems * SIZE` exact for the commit range; computing the range in bytes and the capacity by floor division keeps every commit page-aligned for any element size. Admits 12/24/40 B elements (`traverse_iter.rs:284`, `serialize/mod.rs:288`, `bindless.rs:74`).

### 2.3 Data structures

```rust
// ---------- Column class additions (boyko_memory) ----------

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

// ---------- Heap class ----------

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

// ---------- Table class ----------

/// One reservation, N typed fixed-length tables, laid out at build.
pub struct TableSet { res: VmReservation, used: usize }
pub struct TableLayout { size: usize }                  // builder: accumulates aligned field sizes
pub struct Table<T> { ptr: NonNull<T>, len: u32 }       // 16 B handle into a TableSet; Deref<[T]>
```

### 2.4 Public API (signatures)

```rust
// Column
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

// Heap
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

// Table
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

SIMD: none of these are vectorisable and none need to be — all are cold-or-O(1) bookkeeping; the data they hold is what gets vectorised.

## 3. The gate ladder

| Rung | Gate | Lands | Mechanism | Red-first canary |
|---|---|---|---|---|
| **G1** | **Field ledger** (workspace) | immediately, before any migration | `tests/alloc_field_ledger.rs`, generalised from `[ecsnative] tests/physics_vec_side_store_census.rs` (`SCANNED_ROOT` → every runtime crate's `src/`, `:218`): scans struct fields typed `Vec<|Box<|String|Arc<|Rc<|VecDeque<|BTreeMap<|BTreeSet<|HashMap<|HashSet<` outside `#[cfg(test)]`; requires the set to equal `const LEDGER: &[Site { crate, file, struct, field, class, rung, tag }]` **exactly** (a missing site = red, an extra site = red — so the list can only shrink in the same commit that deletes the field); `MIN_SITES` anti-vacuity floor as in the physics test (`:277`) | (a) add a `Vec` field to production → red; (b) delete a ledgered field without its entry → red; (c) point the scanner at an empty dir → red on the floor |
| **G2** | **Frame allocation census as a gate** | with rung 1 | `crates/boyko_physics/tests/alloc_frame_census.rs` (untracked on ecsnative) becomes `tests/alloc_frame_gate.rs` at the workspace root: per scene `STEADY_MAX_S0..S3, S1a, S1b` constants; the assertion is `measured_max <= STEADY_MAX_X`; each rung lowers its constants; end state = 0 everywhere. Process-global counter (the `boyko_app/tests/zero_alloc.rs` shape, worker-visible, `:47`), not the `thread_local!` shape of `colored_solve_zero_alloc_o5.rs:281-284` | run with a deliberate `Vec::new()`+push in a system → red |
| **G3** | **`DenyAfterSteady`** | with rung 1, opt-in per gate binary | `#[global_allocator]` that delegates to `System` until `enter_steady()`; afterwards `alloc` writes a fixed 64-byte message with `WriteFile`/`write(2)` and `abort()`s. Not unwinding (required by `GlobalAlloc`); Miri arm: counts instead of aborting (the tree measured Tree-Borrows UB delegating to Windows `System` for over-aligned blocks under Miri, `[merge] block.rs:759-782` — the deny gate is `cfg(not(miri))`) | same as G2's canary, must abort |
| **G4** | **clippy `disallowed-types`** on `alloc::vec::Vec`, `alloc::boxed::Box`, `alloc::string::String`, `alloc::sync::Arc`, `alloc::collections::*` (plus the existing `HashMap` etc.) | end state, when the ledger for a crate is ≤ 10 sites (allows are then exceptions with rationale, not noise) | per crate: `#![cfg_attr(test, allow(clippy::disallowed_types))]` at the crate root covers unit tests in one line; integration tests in `tests/` get the same line each; the type ban fires on annotated locals and `collect::<Vec<_>>()` turbofish too, so production locals are covered here. The tree's L8c rejection (`[ecsnative] clippy.toml:32-63`, ~1051 sites) was of `disallowed-macros` without a crate-level test allow; this rung uses one | (a) `Vec` field in production → red; (b) `Vec` local in a `#[cfg(test)]` module → **must not** red (a canary that does not fire is a finding — verify both directions) |
| **G5** | **`#![no_std]` without `extern crate alloc`** | end state | for `boyko_memory` and `boyko_utils` only — structurally no `Vec`/`Box`/`String`/`format!` can be named. Not for `boyko_ecs` (needs `std::sync::OnceLock`, `std::thread` via the pool) | compile |

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
| Site | After | Determinism note |
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
| Site | After |
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
| Rung | Obligation | Proof |
|---|---|---|
| 1a/1b | `ScopeArena` addresses depend on which worker ran the spawner | **no computation reads a task cell's address** — cells are executed and discarded. Canary: `cfg(feature = "mem-shake")` offsets each per-worker arena base by `wid × 4096 × prime` so any address dependence changes results; the `{1,N}` physics oracle (`[ecsnative] plugin.rs:386`) must stay green with and without it |
| 1d | task execution order under the bounded deque + inline-overflow | the oracle is index-addressed and order-independent by construction (colored solve); loom model of push/pop/steal + a test that forces overflow (`LANE_CAP = 2` under test) with the oracle |
| 2 | LIFO stacks | column stack `push`/`pop` = identical sequence; existing entity-id reuse tests + a new property test: interleaved spawn/despawn sequence yields the same id sequence on `Vec` (test oracle) and `VmColumn` |
| 2 | `slot+1` encoding | property test: `EntitySlotMap` old vs new over random insert/remove sequences |
| 2, 3 | Heap reuse order | single writer, program order → same across thread counts; addresses differ run-to-run only by ASLR, as today |
| 2 | Frame arena | dispatcher-only; `debug_assert!(owner == current)` under `cfg(debug_assertions)` |

### 5.3 Miri / loom under Tree Borrows
- **Provenance granularity is the reservation.** Miri cannot see a use-after-free or overlap *inside* a `Heap`/arena `[D std::ptr; D arXiv 2206.11728]`. Compensation: under `cfg(any(miri, feature = "mem-sanitize"))` the Heap keeps a live-bit column (1 bit per 16-B unit), `free` asserts the bit is set and clears it, `alloc` asserts clear and sets it, freed chunks are filled with `0xDD` beyond the link word; `FrameArena::rewind`/`reset` and `ScopeArena::rewind` fill the released range with `0xDD`. This is `FLECS_SANITIZE` `[S flecs.h]`, and it is the only use-after-free detector the arenas will ever have.
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
8. Rung 3 (`TableSet`s for Schedule/Master/Pool; `ErasedSystem`; `Arc` removal; signature changes).
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
