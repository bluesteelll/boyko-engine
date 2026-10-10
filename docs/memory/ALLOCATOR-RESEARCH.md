# Allocator research - four lens reports

Date: 2026-09-10. Trees: **ecsnative** = `D:/wt/ecsnative` (`feat/ecs-native-storage` @ `ad0ebea4`), **merge** = `D:/wt/merge` (`merge/ke16-into-render` @ `d2c8c646`), **main** = `D:/claude/BoykoEngine` (read-only). Written by the Write phase of the allocator workflow; the four reports below are reproduced VERBATIM from the researcher agents (no edits, no reconciliation between lenses - where they disagree, the disagreement is preserved and the design/critique documents resolve it).

## Provenance-tag legend

| Tag | Meaning |
|---|---|
| **[S]** | source file read by the researcher in that session |
| **[D]** | official documentation, release note, API reference, RFC, or paper page read |
| **[B]** | blog post, talk, or secondary summary - recorded, NOT relied on |
| **[T:tree file:line]** / `[tree] path:line` | in-tree claim; the tree is named (ecsnative / merge / main) |
| *not retrieved* / "Not verified" | the researcher could not fetch or render the item; nothing was paraphrased from memory |

Lenses: A = engines (how the serious engines keep allocation inside their own memory library); B = rust (the Rust mechanisms); C = this-engine (the memory library's contract and the full allocation-shape inventory); D = gp-allocator (general-purpose allocation on top of VM reservations).

---

# LENS A - engines

# Research: Lens A — how the serious engines keep allocation inside their own memory library

Provenance tags: **[S]** source file read · **[D]** official doc / release note / API reference · **[B]** blog or talk (recorded, not relied on). In-tree claims carry `tree:path:line`. Items I could not fetch are listed at the end under "Not verified", not silently folded in.

Tooling note: no shell tool was available to this agent, so `graphify` could not be run (project memory also records it as not installed); I fell back to Grep/Read once per the rule. The Naughty Dog PDF was downloaded but the page renderer (`pdftoppm`) is absent on this machine, so its slide text is unverified.

---

## TL;DR

- **Every engine surveyed that "bans" the foreign allocator does it by replacing or fronting the process allocator, not by banning the container type.** Unreal routes global `operator new/delete` to `FMemory::Malloc` (`REPLACEMENT_OPERATOR_NEW_AND_DELETE`, ModuleBoilerplate.h) [S via donaldwuid quote]; O3DE ships `NewAndDelete.inl` routing global `new/delete` to `AZ::OperatorNew` [S]; Doom 3 replaces `malloc/free` with `Mem_Alloc/Mem_Free` on `idHeap` [S]. flecs, EnTT and Godot **sit beside** the process allocator: flecs' block/stack allocators get their chunks from a hookable `ecs_os_malloc` [S]; Godot's `Memory::alloc_static` is `malloc` plus an optional size prefix [S]; EnTT defaults to `std::allocator` [S].
- **Lifetime classes are the organising idea, and the shipped set is small.** Unity: `Temp` (one frame / one job, 64-B aligned, cannot cross into a job), `TempJob` (≤ 4 frames, 16-B), `Persistent` (manual free), plus rewindable allocators (world-update = double-buffered, 2-frame lifetime; ECB; system-group) [D]. Unreal: persistent (`GMalloc`), linear frame/scope (`FMemStack` + `FMemMark`), in-object (`TInlineAllocator`/`TFixedAllocator`) [D]. flecs: per-world/per-stage size-class block allocators (persistent), a per-stage page stack with cursors (temporary), per-stage command chunks (deferred) [S].
- **Binding a collection to an allocator is done three ways:** compile-time type parameter (Unreal `TArray<T, Alloc>`, EnTT `basic_registry<E, Alloc>` via `allocator_traits`, O3DE `AZStd::vector<T, AZStdAlloc<A>>`) [D/S]; runtime handle (Unity `AllocatorHandle {Index:u16, Version:u16}` dispatching through a global function-pointer table) [S mirror]; global/implicit (Godot `memalloc`, Doom 3 `Mem_Alloc`, flecs `ecs_os_api`) [S].
- **Measured gains that are actually published:** Unity's rewindable allocator vs `Persistent` on an i9-12900K: 3.8 µs vs 25.1 µs for a 150 × 1 KiB batch, 44× at 1 MiB, 133–135× on size-ramp patterns [D]; Doom 3's `idHeap` header claims 2.5–3.0× average over MSVC `malloc/free`, 1.65× worst, > 70× best [S]; flecs 3.1 claims "as much as 60× faster than OS allocations" [B, snippet only]. Counter-evidence: Forrest Smith's 2022 trace of Doom 3 (5.5 M allocs / 7 min) shows modern general allocators at < 10 ns median, ~25 ns p95, with a ~500 µs worst tail [B].
- **The engines' own no-foreign-allocation rule is enforced by the compiler, not by a lint, only in one case:** Unity Burst rejects managed objects, reference types, managed arrays and `string` outright [D]; every other engine enforces by convention plus a debug tracker (flecs `FLECS_SANITIZE` outstanding-map; Godot `_current_mem_usage` in `DEBUG_ENABLED`; Doom 3 `Mem_EnableLeakTest`; O3DE `aznew` compile error without `AZ_CLASS_ALLOCATOR`) [S/D].

---

## Approaches in state-of-the-art engines

### Unity DOTS / Unity.Collections

**Lifetime classes** [D, Collections 2.6 "Allocator overview"; Entities 1.3 "Allocators overview" / "World update allocator" / "System group allocator"]:

| Class | Lifetime | Thread rule | Min alignment | Freed how |
|---|---|---|---|---|
| `Allocator.Temp` | one frame on the main thread; one job on a worker ("Jobs each create one per thread, deallocating at job completion") | "You can't pass this allocator to a job"; "only safe to use in the thread and the scope where they were allocated" | 64 B | discarded as a whole; manual free "does nothing" |
| `Allocator.TempJob` | "must deallocate ... within 4 frames" | passable to jobs | 16 B | manual; Native- collections throw past 4 frames, Unsafe- collections do not check |
| `Allocator.Persistent` | indefinite; "the slowest allocator" | passable to jobs | 16 B | manual; safety checks "can't detect if a Persistent allocation has outlived its intended lifetime" |
| Rewindable (custom) | until `Rewind()` | "fast and thread safe" | 64 B | `Rewind()` keeps previously used blocks, disposes the rest; invalidates child allocators and child safety handles |
| World update allocator (Entities) | "spans two frames" — "double rewindable allocators ... created when the world is initiated", switched/rewound by `WorldUpdateAllocatorResetSystem` | "You can pass allocations from the world update allocator into a job" | (rewindable) | automatic |
| Entity command buffer allocator | "same as the entity command buffer" | — | (rewindable) | automatic |
| System group allocator | "two system group updates"; created only via `SetRateManagerCreateAllocator()` | — | (rewindable) | automatic |

**Binding**: runtime handle. Containers take `AllocatorManager.AllocatorHandle` (or the `Allocator` enum) in the constructor: `new NativeList<int>(customAllocator.Handle)` [D "Custom allocator use"]. In source (needle-mirror of `AllocatorManager.cs`) the handle is `{Index: ushort, Version: ushort}`; built-ins are `Invalid=0, None=1, Temp=2, TempJob=3, Persistent=4, AudioKernel=5`; customs start at `FirstUserIndex` and dispatch through a global table of `TryFunction` pointers; `Rewind()` bumps the version so stale handles fail [S mirror].

**Custom allocator contract** [D "Custom allocator define"]: implement `AllocatorManager.IAllocator` — `Try(ref Block)`, `Function`, `Handle`, `ToAllocator`, `IsCustomAllocator`, `IsAutoDispose`, `Dispose`; the `Try` function must carry `[BurstCompile(CompileSynchronously = true)]` and `[MonoPInvokeCallback(typeof(AllocatorManager.TryFunction))]` so it can live as a function pointer in the global table.

**RewindableAllocator internals** [S needle-mirror `RewindableAllocator.cs`; D "Rewindable allocator overview"]: array of up to 64 `MemoryBlock`s; initial block ≥ 128 KiB; block size doubles until `MaxMemoryBlockSize` = 64 MiB (2^26), then grows linearly by that amount; the allocation path is an `Interlocked.CompareExchange` on a union of `(current offset, allocation count)` — lock-free bump; a `Spinner` is taken only when a new block must be created; `Free` is a no-op unless `enableBlockFree`, in which case a per-block count decrements and the block rewinds at zero; "Make the alignment multiple of cacheline size".

**What "no managed allocation in a job" costs them** [D Burst 1.8 "C#/.NET type support"]: "Burst works on a subset of .NET that doesn't let you use any managed objects or reference types in your code (classes in C#)"; "Burst doesn't support managed arrays. Instead, use a native container such as NativeArray"; `string` is unsupported "because this is a managed type"; "Burst supports any pointer types to any Burst supported types". I.e. the ban is a compiler property of the job language, and the entire Collections library exists to give jobs containers at all.

**Measured** [D Collections 2.6 "Performance comparison of allocators"; i9-12900K, Unity 2022.2.8f1, 150 consecutive allocations per sample, 50 samples, 5 warm-ups]:

| Pattern | Rewindable (Burst, safety off) | Persistent | Ratio |
|---|---|---|---|
| 1024 B fixed, single thread | 3.8 µs | 25.1 µs | 6.6× |
| 1 MiB fixed | 4.4 µs | — | 44.1× |
| increasing sizes to 64 KiB | 4.7 µs | 625.4 µs | 133.1× |
| decreasing sizes from 64 KiB | 4.3 µs | 579.8 µs | 134.8× |

Rewindable with safety on: 4.0 µs (1 KiB), 11.9 µs (1 MiB). The page notes `Temp` reached "up to 450.8× efficiency versus Persistent" in multi-threaded tests but excludes it from the framework tests.

**Replace or beside**: beside. `Temp/TempJob/Persistent` are Unity's native (unmanaged) allocators; the .NET GC heap continues to exist for non-Burst code. The rewindable and Entities allocators are layered on top and obtain their blocks from a backing allocator (`AllocatorHelper<RewindableAllocator>(backgroundAllocator)`) [D].

### Unreal Engine

**Process allocator replaced**: "void* operator new ( size_t Size ) { return FMemory::Malloc( Size ); }" — the `REPLACEMENT_OPERATOR_NEW_AND_DELETE` macro in `ModuleBoilerplate.h` [S quoted by donaldwuid/unreal_source_explained; corroborated B ikrima]. `FMemory::Malloc/Realloc/Free` forward to `GMalloc`; if null, `GCreateMalloc()` [B ikrima]. `GMalloc` is chosen by `FPlatformMemory::BaseAllocator()` — UE4-era default `Binned2` for game, `TBB` for editor, overridable by `-ansimalloc / -tbbmalloc / -binnedmalloc2 / -binnedmalloc` [B ikrima]. Epic's coding standard: "Standard containers and strings should be avoided except in interop code" [D].

**FMallocBinned2** [B rawsourcecode.io parts 1–2, an illustrated source walkthrough]: 43 bin sizes from 16 B to 13,104 B, all multiples of 16; a request is "large" if > 13,104 B or alignment > 256; bins live in naturally aligned 64 KiB pages; large allocations are 4 KiB-granular and 64 KiB-aligned via a paged OS-allocator cache (~64 MiB); per-thread `FPerThreadFreeBlockLists` (LIFO) with two bundles per bin (partial/full), a bundle capped at 64 nodes or 64 KiB; full bundles overflow to a `GlobalRecycler` with 8 slots per bin ("8 is the sweet spot for 64 bit systems with a cache line of 64 bytes"); per-bin locks otherwise; TLS caches are opt-in per thread (`FMemory::SetupTLSCachesOnCurrentThread()`) and trimmed when a thread sleeps; average O(1) when caches hit. **FMallocBinned3**: "reserves contiguous range of virtual memory (a Pool) for each allocation size (a Bin)" [D dev.epicgames API page, via search snippet — page itself is JS-rendered].

**Container binding = compile-time type parameter** [D "TArray: Arrays in Unreal Engine"]: `TArray` is "defined by two properties: Element type, and an optional allocator", which "defines how the objects are laid out in memory and how the array should grow". Named policies [D TArray doc + B ikrima / dr-elliot]: `TInlineAllocator<N, Secondary = heap>` (N elements in-object, spills to the secondary), `TFixedAllocator<N>` (no secondary — overflow is an error), `TSizedHeapAllocator` (always indirect), `TSparseArrayAllocator`, `TSetAllocator`, `TMemStackAllocator`, `TNonRelocatableInlineAllocator`, `TLockFreeFixedSizeAllocator`.

**Frame / scope lifetime = `FMemStack` + `FMemMark`** [D docs.unrealengine.com 4.26/4.27 API text via search snippet; the pages return 403 to direct fetch]: "FMemStack is a simple linear-allocation memory stack where items are allocated via PushBytes() or specialized operator new()s, and items are freed en masse by using FMemMark to Pop() them"; "FMemMark marks a top-of-stack position ... When marker is popped, it pops all items that were added to the stack subsequent to initialization". Lineage/claims [B Frechette 2016]: same design as UE3's `FMemStack`, "many AAA games have shipped with this"; greedy retention of segments after pop to avoid kernel calls; push O(1), pop cost proportional to segments freed; ~40 B allocator footprint.

**Lifetime classes**: persistent (`GMalloc`), frame/scope (`FMemStack`/`FMemMark`, `TMemStackAllocator` for containers), in-object/fixed (`TInlineAllocator`, `TFixedAllocator`). No level/streaming class in the core allocator surface was found in the sources read.

**Measured**: none published on the fetched pages beyond the "O(1) average" statement [B]. Epic's "Optimizing TArray Usage for Performance" blog returned 403 — not cited.

### flecs

**Sits beside the OS allocator, behind a hook** [S `src/os_api.c`]: `ecs_os_api_t` carries `malloc_`, `realloc_`, `calloc_`, `free_`; `ecs_os_set_api()` replaces them before initialisation; with `FLECS_TRACK_OS_ALLOC` each allocation carries a 16-byte size prefix and `ecs_os_allocated_bytes` is maintained; counters `ecs_os_api_malloc_count / realloc_count / calloc_count / free_count`. Manual: memory returned as `T*` is freed by the caller with `ecs_os_free()` [D Manual].

**Block allocator** [S `include/flecs/datastructures/block_allocator.h`, `src/datastructures/block_allocator.c`]: `ecs_block_allocator_t {data_size, chunk_size, chunks_per_block, block_size, head, block_head, alloc_count}`; `chunk_size = ECS_ALIGN(size, 16)`; `chunks_per_block = max(4096 / chunk_size, 1)`; a block is one `ecs_os_malloc(header + block_size)`; the free list is threaded through freed chunks (`chunk->next = ECS_OFFSET(chunk, chunk_size)`), pushed at `head`; **no atomics or locks** — single-owner by construction. `FLECS_USE_OS_ALLOC` bypasses pooling ("use the OS allocator provided in the OS API directly instead of the built-in block allocator. This can decrease memory utilization as memory will be freed more often, at the cost of decreased performance" [S flecs.h]). `FLECS_SANITIZE` keeps an `outstanding` map, checks every free against its allocator, and reports leaks by type name on `fini` [S].

**Size-class front** [S `include/flecs/datastructures/allocator.h`]: `ecs_allocator_t {chunks: ecs_block_allocator_t, sizes: ecs_sparse_t}` — a sparse set from size to block allocator; `flecs_allocator_get(a, size)`; macros `flecs_alloc/calloc/realloc/free/_t/_n`, `flecs_dup`, `flecs_strdup/strfree`.

**Stack allocator** [S `include/flecs/datastructures/stack_allocator.h`]: pages of `FLECS_STACK_PAGE_SIZE` = 1024 B minus a 16-aligned offset, `int16_t sp` per page, `ecs_stack_cursor_t {prev, page, sp, is_free}`; `flecs_stack_get_cursor()` / `flecs_stack_restore_cursor()`; `flecs_stack_free()` is a no-op for small allocations; documented purpose "quick allocation of small temporary values".

**Per-stage ownership (the lock-free story)** [S `src/stage.c`]: `flecs_stage_new()` runs `flecs_allocator_init(&stage->allocator)`, `flecs_stack_init(&stage->allocators.iter_stack)` and `flecs_ballocator_init_*` for `cmd_entry_chunk`, `query_impl`, `query_cache`; `flecs_stage_free()` tears them down. Rationale quoted from the source: "each thread has its own stage which allows threads to insert mutations without having to lock the administration". So flecs' lifetime classes are: persistent per world/stage (size-class block pools), temporary (page stack with cursors, per stage), deferred (per-stage command chunk pools + stack).

**Claimed** [D v3.1.0 release notes]: "[internals] Reduced number of heap allocations with internals now mostly using custom allocators". [B Medium "Flecs 3.1 is out!", 2022-10-20 — article returned 403; text known only from a search snippet]: "Custom allocations can be as much as 60x faster than OS allocations". No benchmark table located.

### EnTT

**Binding = compile-time type parameter through `std::allocator_traits`** [S `src/entt/entity/registry.hpp`]: `basic_registry<Entity, Allocator>`; pools are a `dense_map<id_type, shared_ptr<base_type>, ..., rebind_alloc<pair<...>>>`; `assure()` creates a storage with `allocate_shared<storage_type>(get_allocator(), get_allocator())` — the allocator is passed both to the control block and to the storage; `get_allocator()` returns `entities.get_allocator()`; a constructor taking `allocator_type` exists. [S `sparse_set.hpp`]: `basic_sparse_set<Entity, Allocator>` keeps `sparse` as `vector<alloc_traits::pointer, rebind>` and `packed` as `vector<Entity, Allocator>`; sparse pages come from `alloc_traits::allocate(page_allocator, traits_type::page_size)`; deletion policies `swap_and_pop`, `in_place`, `swap_only`. [S `storage.hpp`]: `basic_storage<Type, Entity, Allocator>` is a vector of page pointers, pages allocated with `alloc_traits::allocate(allocator, comp_traits::page_size)`, `element_at(pos) = payload[pos / page_size][fast_mod(pos, page_size)]`.

**Docs** [D `docs/md/entity.md`]: "Sparse arrays are paged to avoid wasting memory. Packed arrays of components are also paged to have pointer stability upon additions"; `component_traits::page_size` defaults to `ENTT_PACKED_PAGE` for non-empty types; `in_place_delete`. [D discussion #880, v3.11 changelog]: "Partial allocator support for the `basic_registry<...>` class (registry allocators also propagate to their pools)"; allocator support added to `sigh_storage_mixin`, `basic_emitter`, `storage_type[_t]`, runtime views.

**Lifetime classes**: none distinguished — one allocator type per registry, propagated downward. **Replace or beside**: beside; default is `std::allocator`, i.e. global `new`. PMR: no `std::pmr` reference in the files read; `std::pmr::polymorphic_allocator` fits the type parameter by construction [S registry.hpp; general C++ fact].

### Godot

**Beside the process allocator** [S `core/os/memory.cpp`]: `Memory::alloc_static` calls `malloc` (or `calloc`), optionally prepadding a `DATA_OFFSET` header that stores the size (`*s = p_bytes`); `_current_mem_usage` / `_max_mem_usage` are maintained only under `DEBUG_ENABLED`; `operator new(size_t, DefaultAllocator)` and `operator new(size_t, void*(*)(size_t))` placement forms exist — the global `operator new` is not replaced. [D core_types.rst]: `memalloc/memrealloc/memfree` are "equivalent to the usual malloc(), realloc(), and free()"; `memnew/memdelete/memnew_arr/memdelete_arr` use "a little C++ magic to automatically call post-init and pre-release functions"; Godot keeps its own containers rather than `std::string`/`std::vector`.

**Containers** [D core_types; S `cowdata.h`, `local_vector.h`]: `Vector` is copy-on-write — "generally slower but can be copied around almost for free"; `CowData` stores `[refcount][capacity][size]` ahead of the data via `Memory::alloc_static/realloc_static`, atomic `SafeNumeric` refcount, growth 1.5× ("close to the ideal growth rate of the golden ratio"). `LocalVector<T, U=uint32_t, force_trivial, tight>` is "closer to std::vector in semantics", uses `memrealloc/memfree`, 1.5× growth or exact when `tight`.

**PagedAllocator** [S `core/templates/paged_allocator.h`]: `PagedAllocator<T, thread_safe=false, DEFAULT_PAGE_SIZE=4096>`; a `page_pool` of pages plus an `available_pool` free list, both grown with `memrealloc`; `SpinLock` around every operation when `thread_safe`; `reset()` validates that everything was returned unless `p_allow_unfreed`.

**Lifetime classes**: none formalised; binding is global (macros); the process allocator is not replaced. No frame/temp allocator was found in the core files read.

### id Tech 4 (Doom 3 / Doom 3 BFG)

**Process allocator replaced (Doom 3, 2004)** [S `neo/idlib/Heap.h`, `Heap.cpp`]: all allocation goes through `Mem_Alloc/Mem_Free/Mem_Alloc16/Mem_Free16` on `idHeap`; the header claims "On average 2.5-3.0 times faster than MSVC malloc()/free(). Worst case performance is 1.65 times faster and best case > 70 times". `idHeap` design: page size `65536 - sizeof(page_s)`, pages obtained with `malloc()`; three tiers — small (1–255 B, 2-byte header, 33 free lists by size class, `ALIGN = 8`), medium (256–32,768 B, doubly-linked free/used lists with coalescing), large (own page); one cached "swap page"; a pre-allocated defrag block released under pressure; marker byte `0xaa/0xbb/0xcc` before each pointer identifies the tier. Specialised pools: `idBlockAlloc` (fixed-size object pool), `idDynamicBlockAlloc` (B-tree arena over pre-allocated base blocks, lockable, defragmentable) [S Heap.h].

**BFG edition (2012)** [S `DOOM-3-BFG/neo/idlib/Heap.cpp`, `Heap.h`, `List.h`, `StaticList.h`]: `Mem_Alloc16` is `_aligned_malloc((size + 15) & ~15, 16)` — the custom heap was dropped for the CRT; `memTag_t` tags "are used to sort allocations for sys_dumpMemory and other reporting functions" (the tag is accepted but unused by `Mem_Alloc16` itself); `idList<_type_, _tag_>` carries the tag as a template parameter, allocates lazily, grows by a granularity (default 16); `idStaticList` is "a non-growing, memset-able list using no memory allocation"; `idTempArray` is "an array that is automatically free'd when it goes out of scope".

**Lifetime classes**: persistent (heap), fixed/no-alloc (`idStaticList`), scope-temporary (`idTempArray`), tag-classified (`memTag_t`, reporting only). **Measured (third-party)** [B Forrest Smith, 2022-06-03]: instrumenting Doom 3 — 5.5 M allocations and 5.5 M frees in 7 min, 2.47 GB cumulative, 330 MB peak, median 64 B, mean 480 B, 97.1 % freed on the allocating thread; jemalloc/mimalloc/rpmalloc median < 10 ns, p95 ≈ 25 ns, worst 0.1 % 1–50 µs, absolute worst ≈ 500 µs; Windows CRT p90 ≈ 60 ns, p99.99 ≈ 25 µs; tlsf best worst-case but single-threaded and pool-bound. Author's conclusion: "you can call malloc and still hit real-time framerates".

### O3DE (ex-Lumberyard)

**Process allocator replaced** [S `AzCore/Memory/NewAndDelete.inl`]: global `operator new/new[]/delete/delete[]` (throwing, `nothrow`, sized, and C++17 `std::align_val_t` overloads) route to `AZ::OperatorNew / OperatorNewArray / OperatorDelete / OperatorDeleteArray`; guarded by `AZ_GLOBAL_NEW_AND_DELETE_DEFINED` ("NewAndDelete.inl has been included multiple times in a single module").

**Allocators and schemas** [D o3de.org "Using Memory Allocators in O3DE"; D Lumberyard user guide]: "Each allocator commonly implements the IAllocator interface and uses a schema to implement the allocation algorithms and bookkeeping"; `OSAllocator` "for direct operating system allocations on the C heap"; `SystemAllocator` "the general purpose allocator for the AZ memory library"; `PoolAllocator` "performs extremely fast small object memory allocations" but "is not thread safe"; `ThreadPoolAllocator` "thread safe pool allocator". Schemas: `HphaSchema` — "Heap allocator schema, based on Dimitar Lazarov 'High Performance Heap Allocator'" [S Lumberyard HphaSchema.h], "combines a small block allocator for small allocations and a red-black tree for large allocations" [D]; `PoolSchema`; `ThreadPoolSchema` "uses thread local storage"; `ChildAllocatorSchema` "acts as a pass-through schema to another allocator" — "Each Lumberyard gem or logical subsystem create a ChildAllocator to properly tag the memory that it allocates" [D]. `IAllocator` surface [D API]: `allocate/deallocate/reallocate`, `GarbageCollect`, `NumAllocatedBytes`, `Capacity`, `GetMaxContiguousAllocationSize`, profiling/records.

**Binding**: per class via `AZ_CLASS_ALLOCATOR` (using `aznew` without it "triggers a compile error"); per container via `AZStd::vector<MyClass, AZ::AZStdAlloc<CustomAllocator>>`; per subsystem via child allocators; singleton lifecycle `AllocatorInstance<T>::Create()/Destroy()` [D].

**Lifetime classes**: none formal — classification is by subsystem/tag (child allocators) and by size (pool vs heap). **Measured**: none published on the fetched pages.

### Naughty Dog (GDC 2015, Gyrling) — recorded, unverified

The talk's abstract lists "the memory allocation patterns used in the title" [D GDC Vault listing]. A search snippet describes a "tagged heap" with 2 MB blocks, freed by tag at frame end [B, snippet only — the PDF was downloaded but could not be rendered here]. Filed as unverified.

---

## Comparative table

| Aspect | Unity DOTS | Unreal | flecs | EnTT | Godot | id Tech 4 | O3DE |
|---|---|---|---|---|---|---|---|
| Replaces process allocator? | No (native heap beside GC; Burst forbids GC heap) [D] | **Yes** — global `new/delete` → `FMemory::Malloc` → `GMalloc` [S quote] | No — chunks via hookable `ecs_os_malloc` [S] | No — `std::allocator` default [S] | No — `malloc` + size prefix [S] | **Yes** (2004: `idHeap`); BFG: `_aligned_malloc` [S] | **Yes** — `NewAndDelete.inl` [S] |
| Lifetime classes | Temp (frame/job), TempJob (4 frames), Persistent, rewindable (world 2-frame, ECB, system-group) [D] | persistent, linear frame/scope (`FMemStack`/`FMemMark`), in-object inline/fixed [D] | per-stage persistent size-class pools, per-stage temp page stack w/ cursors, deferred cmd chunks [S] | none (one allocator per registry) [S] | none formal [S] | persistent, fixed (`idStaticList`), scope (`idTempArray`), tag (reporting) [S] | by subsystem (child allocators), by size (pool/heap) [D] |
| Collection ↔ allocator binding | runtime `AllocatorHandle{Index,Version}` + fn-pointer table [S] | compile-time `TArray<T, Alloc>` [D] | implicit (`ecs_allocator_t*` passed to `ecs_vec` ops; not fully read) | compile-time `Allocator` param + `allocator_traits` rebind [S] | global macros; containers hardwired [S] | tag template param on `idList` [S] | `AZ_CLASS_ALLOCATOR` + `AZStdAlloc<A>` type param [D] |
| Temp/frame mechanism | atomic bump on 64-B aligned blocks, block doubling to 64 MiB, ≤ 64 blocks [S] | linear stack, mark/pop, greedy segment retention [D/B] | 1 KiB pages, `int16 sp`, cursor save/restore [S] | — | — | `idTempArray` scope free [S] | — |
| Thread strategy | lock-free CAS bump; spinner only on new block [S]; Temp is per-thread/per-job [D] | per-thread LIFO bundles + global recycler + per-bin locks [B] | per-stage (per-thread) allocators, no atomics [S] | user's allocator | `SpinLock` in `PagedAllocator<…, true>` [S] | not surveyed | `ThreadPoolSchema` via TLS [D] |
| Debug / enforcement | safety handles invalidated on rewind; Burst rejects managed types [D] | — (not read) | `FLECS_SANITIZE` outstanding-map + per-allocator free check [S] | — | `DEBUG_ENABLED` usage counters [S] | `Mem_EnableLeakTest`, marker bytes [S] | `aznew` compile error w/o class allocator [D] |
| Published numbers | rewindable vs Persistent 6.6×/44×/133×/135× [D] | "O(1) average" only [B] | "60× faster than OS" [B snippet] | none | none | 2.5–3.0× avg vs MSVC (header) [S]; 3rd-party: modern malloc < 10 ns median [B] | none |

---

## Key techniques that recur

1. **Size-class pools with an intrusive free list threaded through freed chunks** — flecs (`chunk->next` in the chunk, 16-B classes, 4 KiB blocks) [S]; Unreal MB2 (43 classes, 64 KiB pages) [B]; Doom 3 (33 small classes, 8-B align) [S]; O3DE HPHA small-block side [D]. Guarantee: O(1) alloc/free, no per-allocation header for pools (flecs, MB2); cost: a class per size, single-owner or locked.
2. **Linear/bump allocation with mass release** — Unity rewindable (atomic CAS bump, `Rewind()`), `Temp` (whole-allocator discard per frame/job) [S/D]; Unreal `FMemStack`/`FMemMark` [D]; flecs stack pages + cursors [S]. Guarantee: allocation is a few instructions and thread-safe without locks when the bump is atomic; free is free. Cost: nothing is reclaimed early unless block-level counting is enabled (`enableBlockFree`) [S].
3. **Per-thread / per-stage ownership instead of synchronisation** — flecs stages ("without having to lock the administration") [S]; Unreal TLS bundles [B]; Unity `Temp` per job thread [D]; O3DE `ThreadPoolSchema` TLS [D].
4. **Double-buffering the frame allocator** so that data produced in frame N stays readable while frame N+1 allocates — Unity's world-update and system-group allocators are literally two rewindable allocators swapped by a reset system, giving a 2-frame lifetime [D].
5. **Version-stamped handles** so a rewind invalidates every borrower — Unity `AllocatorHandle.Version` (15 bits) bumped on `Rewind()`; child allocators and child safety handles invalidated transitively [S/D].
6. **Virtual-address reservation per size class** — Unreal MB3 "reserves contiguous range of virtual memory (a Pool) for each allocation size (a Bin)" [D snippet]; the same reserve-then-commit primitive is what `VmReservation` is in-tree (`ecsnative:crates/boyko_ecs/src/ecs/memory/vm.rs:80-97`, `reserve` at :109, cold `commit` at :199).
7. **Compile-time in-object storage with a spill policy** — Unreal `TInlineAllocator<N, Secondary>` / `TFixedAllocator<N>` (overflow is an error) [D]; Doom 3 `idStaticList` (no allocation at all) [S].
8. **Growth ratio**: Godot chose 1.5× for both `CowData` and `LocalVector` with the explicit comment "close to the ideal growth rate of the golden ratio" [S]; Doom 3 `idList` grows by a fixed granularity (16) [S].
9. **Debug-only accounting and leak proof** — flecs `alloc_count` + outstanding map under `FLECS_SANITIZE` [S]; Godot counters under `DEBUG_ENABLED` [S]; Doom 3 leak test + tier marker bytes [S]; Unity safety handles [D].

---

## Pitfalls recorded in the sources

- **A no-op free changes semantics.** Unity: freeing a `Temp` allocation "does nothing"; rewindable `Free` is a no-op unless `enableBlockFree` [D/S]. Code that assumes memory returns on free will grow the frame allocator until `Rewind()`.
- **Lifetime windows are enforced only where safety checks exist.** Unity: exceeding `TempJob`'s 4 frames throws for Native- collections but "Unsafe collections lack automatic checks"; `Persistent` overruns are undetectable [D].
- **Per-thread caches must be opted into and trimmed.** Unreal's TLS bundles require `SetupTLSCachesOnCurrentThread()` and are trimmed when threads sleep; the global recycler size (8) is tuned to the cache line [B].
- **Pooling trades memory for speed by design.** flecs documents the reverse switch: `FLECS_USE_OS_ALLOC` "can decrease memory utilization as memory will be freed more often, at the cost of decreased performance" [S].
- **A custom heap can be dropped later.** Doom 3 (2004) shipped `idHeap`; the BFG edition (2012) allocates with `_aligned_malloc` and keeps only tags and pools [S]. Third-party measurement of the same game shows general-purpose allocators at < 10 ns median with a ~500 µs tail [B]; the tail, not the median, is the real-time argument.
- **Bitwise reallocation of non-trivial types** — Godot issue #105824: `LocalVector` bit-copies on realloc, which "is not correct if the type is not trivially copyable" [D issue]. Any typed column over a raw reservation inherits this class of bug unless `T: Copy` is pinned, which `VmColumn` does (`ecsnative:…/memory/vm_column.rs:24-30`, bound at :80).
- **Godot's `Vector` is CoW and therefore "generally slower"** — the docs steer hot code to `LocalVector` [D].
- **Partial allocator awareness is a real state.** EnTT's own changelog calls registry allocator support "Partial" as of v3.11 [D].

---

## Relevant academic / reference works

- Dimitar Lazarov, "High Performance Heap Allocator" — the basis of O3DE's `HphaSchema` (small-block + red-black tree) [S attribution in header]; the paper itself was not fetched.
- "Simulation of high-performance memory allocators", arXiv 2406.15776 — surfaced by search only; not read, not relied on.
- Forrest Smith, "Benchmarking Malloc with Doom 3" (2022) — empirical allocation-lifetime and allocator-latency data on a shipped game [B].
- Nicholas Frechette, "Greedy Stack Frame Allocator" (2016) — the `FMemStack` lineage and its cost model [B].
- Christian Gyrling, "Parallelizing the Naughty Dog Engine Using Fibers", GDC 2015 — memory-allocation section unverified (see above) [B].

---

## Applicability to boyko-engine (facts only — the design call is the architect's)

**What the tree already has (ecsnative, `feat/ecs-native-storage @ ad0ebea4`):**
- The memory library is `utils`, `component_pool`, `device_column`, `vm`, `vm_column` (`crates/boyko_ecs/src/ecs/memory/mod.rs:1-5`). `VmReservation {base, os_len}` is reserve/commit/release only — "ALL policy ... lives in the OWNER" (`vm.rs:8-10`, struct `:85-97`, `reserve` `:109`, `#[cold] commit` `:199`, `Drop` `:266`); zero-fill contract `:19-37`. `VmColumn<T: Copy>` (`vm_column.rs:80-113`) requires `size_of::<T>()` to divide `COMMIT_GRANULE` (`:32-45`) and never drops elements (`:24-30`). `InlandStore` is the same primitive for `EntityInland` records, 1 GiB reserve on 64-bit (`core/entity/inland_store.rs:1-11`). `LogRing` is already fully on `VmColumn` (`core/log/ring.rs:103-117`) — an in-tree proof that a `Resource`-owned ring needs no `Vec`.
- Grep of `memory/` for `Vec<|Box<|String|HashMap`: the only non-test, non-comment hit is `PoolBacking::Device(Box<DeviceColumn>)` (`memory/component_pool.rs:93`), documented as a FFI/GPU arm.

**Where the kernel still allocates through the global allocator (ecsnative):** `dense_store.rs:126 free: Vec<u32>` sits beside `s2e: VmColumn<EntityId>` (`:118`) in the same struct, and the doc comment at `:77-82` explicitly grants "the legitimate `std::Vec` exception" for `s2e/free/e2s/live` — which the owner's ruling withdraws; `entity_slot_map.rs:40 slots: Vec<u32>`; `live_bitmap.rs:31 words: Vec<u64>`; `entity_master.rs:73 free_entity_ids: Vec<EntityId>`; `archetype_bundle.rs:135 free_slots: Vec<u16>`; `asset/assets.rs:205 free: Vec<u32>`; `query/relation/traverse_iter.rs:51 words: Vec<u64>`; `commands/command_queue.rs:83-85` two `Vec<MaybeUninit<u8>>` (with the note at `:106-108` that `Vec::new()` defers allocation — the first push still hits the global allocator). This matches the engines' pattern where the *bookkeeping* (free lists, bitmaps, command bytes) is exactly what their size-class pools and per-stage stacks serve.

**Where the scheduler/threadpool allocate (merge, `merge/ke16-into-render @ d2c8c646`):** build-time `HashMap`/`Vec` in `schedule_builder.rs:106-144`; frame-resident `Vec`/`Box<[…]>` in `schedule.rs:122,161,167,172,187`, `conflict_graph.rs:68,73,77`, `executor_scratch.rs:219,274,282`; `Box<dyn System>` at `system_box.rs:83`; `CompletionChannel` is a `Box::into_raw` heap cell held as `NonNull` (`executor_scratch.rs:40-48`). The threadpool has no `Vec/Box/String/HashMap` struct field matching `name: Vec<` in `src/`; it allocates a `Box<ScopeShared>` on every `install` (`thread_pool.rs:278`) and every `scope` (`:328`), a `Box` in the `#[cold]` panic path (`scope.rs:627`), and setup-time `Vec`s in `thread_pool.rs:672-714`.

**Mapping to the survey:**
- The Unity/Unreal/flecs *frame-temp* class (bump, mass rewind, per-thread or atomic) has no in-tree counterpart; `VmReservation` + an atomic frontier is the same shape as Unity's rewindable block (64-B aligned CAS bump) [S], with the difference that Unity's blocks are heap-allocated and bounded at 64 MiB × 64 blocks, whereas a reservation is a single fixed VA range.
- The flecs *per-stage size-class pool* maps onto the free-list/bitmap fields above; flecs' pool is single-owner with no atomics [S] — the same ownership discipline the in-tree `CommandQueue` already documents ("single-writer access", `command_queue.rs:75-81`).
- The Unreal *in-object inline/fixed* class (`TInlineAllocator<N>`, `TFixedAllocator<N>`) and Doom 3's `idStaticList` are the zero-allocation answer for small bounded locals; nothing in-tree is the analogue yet.
- Every engine that *replaces* the process allocator does it at the language's global hook (`operator new`). Rust's equivalent is `#[global_allocator]`, which changes what `Vec` allocates from but does not remove `Vec` from the code — a ban on the type and a replacement of the allocator are different instruments, and the surveyed engines use the second (Unreal, O3DE, Doom 3) together with a container library of their own, not the first alone.
- Godot's `LocalVector` bit-copy defect (#105824) and Unity's `Temp` "free does nothing" are the two failure classes the sources name for exactly the primitives a Vec-free kernel would rely on; `VmColumn`'s `T: Copy` bound already forecloses the first.

**Constraints from the tree that the survey does not answer:** Miri's fallback arm in `vm.rs:168-179` is one eager `alloc_zeroed` per reservation — any new allocator class needs its own Miri arm; the LIFO free-list order that defines slot reuse (`dense_store.rs:124-126`) must survive whatever replaces `Vec<u32>`; determinism oracles across thread counts rule out any per-thread pool whose *ordering* leaks into results.

---

## Open questions for the architect

1. Which of the surveyed lifetime classes does the kernel actually need: persistent (already `VmReservation`-backed), frame-temp (none in tree), per-scope/job (threadpool `Box<ScopeShared>` per scope), and setup/build (schedule builder `HashMap`s)? The engines ship 3–4 classes, never one.
2. Is the ban to be on the *type* (`Vec`, `Box`, `String`, `HashMap`, `collect()`, `format!`) or on the *allocator* (`#[global_allocator]` / `Allocator` trait)? The survey shows the serious engines replace the allocator *and* own their containers; none bans by lint alone.
3. Unity's rewindable allocator invalidates borrowers by handle version; Unreal's `FMemMark` relies on scoping; flecs on cursors. Which invalidation discipline is compatible with Tree Borrows and the existing `NonNull`-not-`Box` relocation pattern (`executor_scratch.rs:40-48`)?
4. The threadpool's per-`scope` `Box<ScopeShared>` (`thread_pool.rs:278/328`) is an allocation on the dispatch path; Unity's `Temp`-per-job and flecs' per-stage stack are the two shapes the survey offers for it.
5. Determinism: flecs' and Unreal's per-thread pools change *addresses*, not orders; the physics oracle is bit-identity across thread counts — does any candidate class expose allocation order to results?

---

## Not verified (fetch failed or content unreadable)

- Epic API pages for `FMemStackBase`, `FMemMark`, `TMemStackAllocator`, `FMallocBinned3` (dev.epicgames.com renders client-side; docs.unrealengine.com 4.27 returned 403). Quotes above for `FMemStack`/`FMemMark` come from the doc text as surfaced in search snippets and are marked so.
- Epic blog "Optimizing TArray Usage for Performance" — 403.
- Sander Mertens, "Flecs 3.1 is out!" (Medium) — 403; web.archive.org is blocked for this tool. The "60×" figure is snippet-only.
- Naughty Dog GDC 2015 slides — PDF downloaded, no renderer on this machine.
- O3DE `HphaAllocator.h` header comment — the O3DE repo path 404'd; the Lumberyard ancestor gave only the one-line attribution.
- EnTT wiki page — the GitHub wiki failed to load; `docs/md/entity.md` from the repo was used instead.

---

## Sources

[1] https://docs.unity3d.com/Packages/com.unity.collections@2.6/manual/allocator-overview.html — Temp/TempJob/Persistent lifetimes, thread rules, alignments [D]
[2] https://docs.unity3d.com/Packages/com.unity.collections@2.6/manual/allocator-rewindable.html — rewindable mechanism, growth, rewind semantics [D]
[3] https://docs.unity3d.com/Packages/com.unity.collections@2.6/manual/allocator-custom-define.html — IAllocator contract, Burst function-pointer requirement [D]
[4] https://docs.unity3d.com/Packages/com.unity.collections@2.6/manual/allocator-custom-use.html — handle-based binding to containers [D]
[5] https://docs.unity3d.com/Packages/com.unity.collections@2.6/manual/performance-comparison-allocators.html — benchmark numbers and rig [D]
[6] https://docs.unity3d.com/Packages/com.unity.entities@1.3/manual/allocators-overview.html — world-update / ECB / system-group allocators [D]
[7] https://docs.unity3d.com/Packages/com.unity.entities@1.3/manual/allocators-world-update.html — double rewindable, 2-frame lifetime [D]
[8] https://docs.unity3d.com/Packages/com.unity.entities@1.3/manual/allocators-system-group.html — `SetRateManagerCreateAllocator` [D]
[9] https://docs.unity3d.com/Packages/com.unity.burst@1.8/manual/csharp-type-support.html — no managed objects / arrays / strings in Burst [D]
[10] http://docs.unity3d.com/Manual/job-system-native-container.html — NativeContainer safety system, allocator guidance [D]
[11] https://github.com/needle-mirror/com.unity.collections/blob/master/Unity.Collections/RewindableAllocator.cs — CAS bump, 64 blocks, 128 KiB / 64 MiB [S mirror]
[12] https://github.com/needle-mirror/com.unity.collections/blob/master/Unity.Collections/AllocatorManager.cs — AllocatorHandle, index table [S mirror]
[13] https://github.com/donaldwuid/unreal_source_explained/blob/master/main/memory.md — quoted `operator new` → `FMemory::Malloc` [S quote in B]
[14] https://ikrima.dev/ue4guide/engine-programming/memory/allocators-malloc/ — GMalloc selection, allocator list [B]
[15] https://rawsourcecode.io/posts/illustrated-overview-mb2-allocator-part-1 and …-part-2 — MallocBinned2 internals [B]
[16] https://dev.epicgames.com/documentation/unreal-engine/array-containers-in-unreal-engine — TArray allocator parameter, slack [D]
[17] https://dev.epicgames.com/documentation/en-us/unreal-engine/epic-cplusplus-coding-standard-for-unreal-engine — "Standard containers and strings should be avoided" [D]
[18] https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Core/HAL/FMallocBinned3 — per-bin VA pool (snippet only) [D]
[19] https://nfrechette.github.io/2016/05/09/greedy_stack_frame_allocator/ — FMemStack lineage and cost model [B]
[20] https://github.com/SanderMertens/flecs/blob/master/include/flecs/datastructures/block_allocator.h — block allocator struct/API [S]
[21] https://github.com/SanderMertens/flecs/blob/master/src/datastructures/block_allocator.c — 16-B classes, 4 KiB blocks, intrusive free list, no atomics [S]
[22] https://github.com/SanderMertens/flecs/blob/master/include/flecs/datastructures/allocator.h — size→block-allocator sparse map [S]
[23] https://github.com/SanderMertens/flecs/blob/master/include/flecs/datastructures/stack_allocator.h — 1 KiB pages, cursors [S]
[24] https://github.com/SanderMertens/flecs/blob/master/src/stage.c — per-stage allocator init, lock-free rationale [S]
[25] https://github.com/SanderMertens/flecs/blob/master/src/os_api.c — malloc hooks, tracking counters [S]
[26] https://github.com/SanderMertens/flecs/blob/master/include/flecs.h — `FLECS_USE_OS_ALLOC`, `FLECS_SANITIZE` docs [S]
[27] https://github.com/SanderMertens/flecs/releases/tag/v3.1.0 — "internals now mostly using custom allocators" [D]
[28] https://ajmmertens.medium.com/flecs-3-1-is-out-543f0aea1 — "60x" claim (403; snippet only) [B]
[29] https://github.com/skypjack/entt/blob/master/src/entt/entity/registry.hpp — allocator propagation via `allocate_shared` [S]
[30] https://github.com/skypjack/entt/blob/master/src/entt/entity/sparse_set.hpp — rebound containers, page allocation, deletion policies [S]
[31] https://github.com/skypjack/entt/blob/master/src/entt/entity/storage.hpp — paged component storage [S]
[32] https://raw.githubusercontent.com/skypjack/entt/master/docs/md/entity.md — paging / pointer stability / component_traits [D]
[33] https://github.com/skypjack/entt/discussions/880 — v3.11 "Partial allocator support" [D]
[34] https://github.com/godotengine/godot-docs/blob/master/engine_details/architecture/core_types.rst — memnew, Vector vs LocalVector [D]
[35] https://github.com/godotengine/godot/blob/master/core/os/memory.cpp and memory.h — `malloc` + prepad, debug counters [S]
[36] https://github.com/godotengine/godot/blob/master/core/templates/cowdata.h — CoW header, 1.5× growth [S]
[37] https://github.com/godotengine/godot/blob/master/core/templates/local_vector.h — memrealloc, 1.5× / tight [S]
[38] https://github.com/godotengine/godot/blob/master/core/templates/paged_allocator.h — page pool, free list, SpinLock [S]
[39] https://github.com/godotengine/godot/issues/105824 — LocalVector bitwise-realloc defect [D issue]
[40] https://github.com/id-Software/DOOM-3/blob/master/neo/idlib/Heap.h and Heap.cpp — idHeap tiers, 2.5–3× claim [S]
[41] https://github.com/id-Software/DOOM-3-BFG/blob/master/neo/idlib/Heap.h, Heap.cpp, containers/List.h, containers/StaticList.h — `_aligned_malloc`, tags, idList granularity, idStaticList [S]
[42] https://www.forrestthewoods.com/blog/benchmarking-malloc-with-doom3/ — allocation trace and allocator latencies [B]
[43] https://raw.githubusercontent.com/o3de/o3de.org/main/content/docs/user-guide/programming/memory/allocators.md — allocators, schemas, `AZ_CLASS_ALLOCATOR`, `AZStdAlloc` [D]
[44] https://raw.githubusercontent.com/awsdocs/amazon-lumberyard-user-guide/master/doc_source/memory-allocators.md — schema table, child allocators per gem [D]
[45] https://github.com/o3de/o3de/blob/development/Code/Framework/AzCore/AzCore/Memory/NewAndDelete.inl — global new/delete override [S]
[46] https://docs.o3de.org/docs/api/frameworks/azcore/class_a_z_1_1_i_allocator.html — IAllocator API [D]
[47] https://github.com/aws/lumberyard/blob/master/dev/Code/Framework/AzCore/AzCore/Memory/HphaSchema.h — Lazarov attribution [S]
[48] https://gdcvault.com/play/1022186/Parallelizing-the-Naughty-Dog-Engine and https://media.gdcvault.com/gdc2015/presentations/Gyrling_Christian_Parallelizing_The_Naughty.pdf — talk listing; slides unrendered [B]

In-tree files cited: `D:/wt/ecsnative/crates/boyko_ecs/src/ecs/memory/{mod.rs,vm.rs,vm_column.rs,component_pool.rs}`, `D:/wt/ecsnative/crates/boyko_ecs/src/ecs/core/entity/{inland_store.rs,entity_master.rs}`, `D:/wt/ecsnative/crates/boyko_ecs/src/ecs/core/component/dense/{dense_store.rs,entity_slot_map.rs,live_bitmap.rs}`, `D:/wt/ecsnative/crates/boyko_ecs/src/ecs/core/commands/command_queue.rs`, `D:/wt/ecsnative/crates/boyko_ecs/src/ecs/core/log/ring.rs`, `D:/wt/ecsnative/crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs`, `D:/wt/ecsnative/crates/boyko_ecs/src/ecs/core/asset/assets.rs`, `D:/wt/ecsnative/crates/boyko_ecs/src/ecs/core/iters/query/relation/traverse_iter.rs`, `D:/wt/merge/crates/boyko_ecs/src/ecs/core/schedule/{schedule.rs,schedule_builder.rs,conflict_graph.rs,executor_scratch.rs,system_box.rs}`, `D:/wt/merge/crates/boyko_threadpool/src/{thread_pool.rs,scope.rs}`.

---

# LENS B - rust

# Research: Lens B — the Rust mechanisms for "no allocator but the memory library"

Tool note: no Bash tool was available in this session, so `graphify` could not be run; navigation fell back to Grep/Read once per the rule. Trees read: `D:/wt/ecsnative` (feat/ecs-native-storage), `D:/wt/merge` (merge/ke16-into-render), `D:/claude/BoykoEngine` (main, read-only). Provenance tags: [S] source read, [D] official doc / paper / RFC, [B] blog or talk.

## Brief summary (TL;DR)

- **`allocator_api` is in FCP-to-stabilize as of 2026-09-09, but the MVP surface is `Allocator` + `Global`/`System` + `Box<T,A>` + `Vec<T,A>` only.** `VecDeque`/`BTreeMap`/`Rc`/`Arc`/`String`/`HashMap` collections generic over `A` stay nightly (or nonexistent — `String` has no `A` at all). Implementors are forbidden to unwind; zero-size allocations are allowed; `dyn Allocator` was separately approved. [D] PR #156882, HackMD report, issue #156906.
- **`#[global_allocator]` is one-per-crate-graph and sees only `Layout` (size+align).** It cannot express lifetime classes (frame/persistent), and it captures *everything*: std internals, `thread::spawn`, panic payloads, crossbeam, criterion. It is the only mechanism that reaches third-party code. [D] Reference "runtime", RFC 1974, `GlobalAlloc` docs.
- **Two goals must be stated separately.** (a) "No foreign allocator in OUR code" is achievable with `Vec<T, A>`/`Box<T, A>` (or in-house containers) over the memory library plus a gate; (b) "No foreign allocator in the PROCESS" is *not* achievable with the current dependency graph: crossbeam-deque's `Worker::push` doubles its buffer via `Box`, `Injector::push` allocates a 31-slot `Block` via `Global.allocate_zeroed`, crossbeam-epoch allocates a `Local` per pinning thread and a queue node per sealed bag; std's `thread::spawn` performs ≥4 heap allocations. Only a `#[global_allocator]` backed by the memory library can *route* those, not remove them. [S] crossbeam `deque.rs`, `internal.rs`, `sync/queue.rs`; std `thread/lifecycle.rs`.
- **Miri executes a user `#[global_allocator]` (since ≤2022) but sees one arena as ONE allocation:** provenance is per-allocation by definition, so use-after-free / overlap *inside* a reservation carved by the memory library is invisible to Miri, exactly as ASan is blind inside Apache's pool allocator. The tree already relies on this (the `cfg(miri)` fallback arm of `VmReservation` is one `alloc_zeroed`). [D] `std::ptr` Provenance section; [S] miri PR #1975, rust #138839; [D] arXiv 2206.11728.
- **Clippy `disallowed_types` cannot be scoped to production code and errors on unresolvable paths unless `allow-invalid`;** the tree already measured and rejected `disallowed-macros` for the same reason (~1051 test sites). A `#![no_std]`-without-`extern crate alloc` crate boundary is the only *structural* ban of `Vec`/`Box`/`String`/`format!`. [D] clippy lint_configuration; [S] `D:/wt/ecsnative/clippy.toml:32-63`; [D] `alloc` crate docs.

## What exists in the tree (measured this session)

| Fact | File:line | Tree |
|---|---|---|
| Memory library = `VmReservation` (reserve/commit/release; `cfg(miri)` fallback arm is one eager `alloc_zeroed`) | `crates/boyko_ecs/src/ecs/memory/vm.rs:85-97, 168-179` | ecsnative |
| `ComponentPool` size-pinned 128 B host / 144 B Miri; `PoolBacking::Device(Box<DeviceColumn>)` is a std `Box` inside the memory library (cold, `#[cfg(not(miri))]`) | `crates/boyko_ecs/src/ecs/memory/component_pool.rs:55-94` | ecsnative |
| `InlandStore` replaced `Vec<EntityInland>` with one reservation (the precedent for "extend memory instead of Vec") | `crates/boyko_ecs/src/ecs/core/entity/inland_store.rs:1-11, 83-108` | ecsnative |
| `DenseStore`: `s2e: VmColumn<EntityId>` migrated, but `free: Vec<u32>` sibling stayed; `e2s: EntitySlotMap { slots: Vec<u32> }`, `LiveBitmap { words: Vec<u64> }` | `dense_store.rs:118-126`, `entity_slot_map.rs:40`, `live_bitmap.rs:31` | ecsnative |
| `EntityMaster::free_entity_ids: Vec<EntityId>` | `crates/boyko_ecs/src/ecs/core/entity/entity_master.rs:73` | ecsnative |
| Count of `Vec/Box/String/VecDeque` typed struct fields in `boyko_ecs/src` (incl. test modules): **71 across 39 files**; heaviest: `schedule_builder.rs` 8, `schedule.rs` 5, `observers/entity_store.rs` 4 | Grep `^\s*(pub..)?\w+\s*:\s*(Vec\|Box\|String\|VecDeque)<` | ecsnative |
| `boyko_utils::SparseMap` / `SparseSlotMap` are three `Vec`s each | `crates/boyko_utils/src/sparse_map/sparse_map.rs:7-13`, `sparse_slot_map.rs:41-44` | ecsnative |
| ECS kernel depends on `crossbeam-queue` (only `ArrayQueue`, allocated once at construction) and `crossbeam-utils` (`CachePadded`) | `crates/boyko_ecs/Cargo.toml:22-23`; `schedule/executor_scratch.rs:34-35, 55-60` | ecsnative |
| Threadpool depends on `crossbeam-deque` + `crossbeam-utils`; criterion is a dev-dep | `crates/boyko_threadpool/Cargo.toml:21-25` | merge |
| Runtime global-allocator sites in the threadpool: `injector_global.push(task)`; `lane.deque().push(task)`; `alloc_cell::<DetachedCell<F>>()` → `std::alloc::alloc`; `Box::new(ScopeShared::new(..))` per `scope()` | `worker.rs:646, 671`; `task/detached.rs:85-97`; `task/mod.rs:310`; `thread_pool.rs:278, 328` | merge |
| `Task` is 16 B thin (`payload: *const (), execute: TaskFn`) — no `Box<dyn FnOnce>` (removed earlier) | `crates/boyko_threadpool/src/task/mod.rs:191-203` | merge |
| loom used only under `cfg(loom)` dependency table; no `loom::alloc` use | `crates/boyko_threadpool/Cargo.toml:33` | merge |
| clippy.toml bans HashMap/HashSet/Mutex/RwLock/Rc/RefCell (+ parking_lot/hashbrown forward tripwires); `disallowed-macros` MEASURED and REJECTED (~1051 test-code sites vs 6 production) | `clippy.toml:14-30, 32-63` | ecsnative |
| ≥10 counting `#[global_allocator]` gates already in tests (physics, scene, input, render, app) | e.g. `crates/boyko_physics/tests/alloc_frame_census.rs:105-148` | ecsnative |
| Toolchain pin is `channel = "stable"`; Miri job uses `cargo +nightly`; docs job pins `nightly-2026-05-20` because "newer nightlies break component_pool.rs via niche/layout changes" | `rust-toolchain.toml:21-28, 38-39` | main |
| `#![feature(...)]` appears in no production crate today (only bench_bevy_vs_boyko `float_algebraic` behind a feature) | Grep `#!\[feature\(` | ecsnative |
| `bench-alloc` feature swaps in mimalloc as `#[global_allocator]` for A/B (bench only) | `crates/boyko_ecs/Cargo.toml:36, 85-87` | ecsnative |

## Mechanism (1): `#![feature(allocator_api)]`

**The trait.** `unsafe trait Allocator { fn allocate(&self, Layout) -> Result<NonNull<[u8]>, AllocError>; unsafe fn deallocate(&self, NonNull<u8>, Layout); }` plus provided `allocate_zeroed`, `grow`, `grow_zeroed`, `shrink`, `by_ref`. Safety contract: blocks stay valid until deallocated *or the allocator is dropped*; copying/cloning/moving the allocator must not invalidate blocks; a clone must behave like the original; `p + n <= usize::MAX`; **zero-sized allocations are allowed** (unlike `GlobalAlloc`); on `Ok` from `grow`/`shrink` ownership transfers and the old pointer is dead even if resized in place. "Memory fitting" lets `deallocate` receive any `layout.size()` in `[requested, returned]` with the same align. The trait *is* dyn-compatible today (`&A`/`&mut A` blanket impls with `A: ?Sized`). [D] https://doc.rust-lang.org/std/alloc/trait.Allocator.html

**Which std types carry `A` (all nightly today):** `Vec<T, A = Global>` (`new_in`, `with_capacity_in`, `try_with_capacity_in`, `from_raw_parts_in`, `from_parts_in`, `into_raw_parts_with_alloc`, `allocator()`); `Box<T: ?Sized, A = Global>` (`new_in`, `new_uninit_in`, `try_new_in`, `into_raw_with_allocator`, `from_raw_in`; `CoerceUnsized` preserved so `Box<dyn Trait, A>` works); `VecDeque<T, A>` (`new_in`, `with_capacity_in`); `BTreeMap<K, V, A: Allocator + Clone>` (`new_in`, gate `btreemap_alloc`); `Rc<T, A>`, `Arc<T, A>` (listed as `Allocator` implementors). **`String` has NO `A` parameter and no `*_in` constructors.** [D] std docs for each type (Sources 5–10).

**Stabilization state (2026-09):** PR #156882 "alloc: stabilise `Allocator`" entered **final comment period on 2026-09-09**, `disposition-merge`, `S-waiting-on-t-lang`. The HackMD report lists the exact stable surface: `Allocator` (required `allocate`/`deallocate`; provided `allocate_zeroed`/`grow`/`grow_zeroed`/`shrink`), `Global`, `System`, `AllocError`, blanket `Allocator for &A / &mut A / Box<A,_> / Rc<A,_> / Arc<A,_>`, `Box::{new_in, from_raw_in, from_non_null_in, into_raw_with_allocator, into_non_null_with_allocator, allocator}`, `Vec::{new_in, with_capacity_in, from_raw_parts_in, from_parts_in, into_raw_parts_with_allocator, into_parts_with_allocator, allocator}`. **NOT stabilized:** `by_ref` (removed), `Clone for Box<T, A>`, `Box::into_pin` with custom `A`, `AllocatorClone`/`StaticAllocator`/`GlobalAllocator` marker traits (unstable), associated `MIN_ALIGN`, and "other collections with allocator generics beyond `Box` and `Vec`". New contract text: "Implementors are now required not unwind from any of the methods on the trait or from drop." `dyn Allocator` compatibility was split into issue #156906 (closed 2026-05-25, decision: keep it dyn-compatible; extension traits for future needs). Design decisions recorded in the PR: `Box<T, A≠Global>` carries **no `noalias`**; `Box` is `#[fundamental]` in `T` only. [D] https://github.com/rust-lang/rust/pull/156882, https://hackmd.io/nNHdKkp1TTK7jat0I-ABqA, https://github.com/rust-lang/rust/issues/156906; [B] cetra3 "State of Allocators in 2026 — 6 Months Later" (2026-09-09) corroborates and names the deferred `AllocatorClone`/`StaticAllocator` questions.

**Timing implication:** even if FCP completes, 10-day FCP → 6 weeks beta → 6 weeks stable places a stable `Vec<T, A>` no earlier than ~early 2027 [B cetra3]. On the tree's `channel = "stable"` pin this means: nightly for the ECS crates now (owner allows nightly, and `docs.yml` already pins `nightly-2026-05-20` for `core_intrinsics`), with the recorded risk that "newer nightlies break component_pool.rs via niche/layout changes" (`rust-toolchain.toml:24-25`, main).

**The `allocator-api2` shim.** Mirrors the unstable API on stable ≥1.64: `alloc::Allocator`, `Global`, `boxed::Box<T, A>`, `vec::Vec<T, A>`, `vec!`, `unsize_box!`, `collections`. Version 0.4.0 released 2026-09-04. Its own `nightly` feature was **removed** in 0.3+; interop pattern is the downstream crate `cfg`-switching imports between `allocator_api2::alloc::Allocator` and `core::alloc::Allocator`. `hashbrown::HashMap<K, V, S, A: Allocator = Global>` takes its allocator through this shim (PR #417). A parallel shim exists: orlp/stable-alloc-shim. [S] https://github.com/zakarumych/allocator-api2, docs.rs/allocator-api2 0.4.0, https://github.com/rust-lang/hashbrown/pull/417.

**The Store/Storage alternative.** RFC #3446 (matthieu-m) replaces the pointer with an associated `Handle: Copy` resolved via `resolve(&self, handle) -> *const u8`, enabling inline storage, `u32` handles, GPU/shared memory; last activity 2024-07-02, open, blocked on const-trait work; not part of the stabilization. [D] https://github.com/rust-lang/rfcs/pull/3446, https://github.com/matthieu-m/storage/blob/main/etc/rfc.md.

**Costs measured by others (opinion/measurement, not fact about this tree):** nical (WebRender, 2023) reports that adding an allocator generic to the tessellator "regresses the benchmarks by a solid 5 to 8 percents" vs `&dyn Allocator`, and lists lifetime pollution from `&'a Bump` and API noise as the practical costs. [B] https://nical.github.io/posts/rust-custom-allocators.html

## Mechanism (2): `#[global_allocator]`

- Attribute rules: applies to a `static` implementing `GlobalAlloc`; **once per crate graph**; default is `System` (malloc/free on Unix, HeapAlloc/HeapFree on Windows); `no_std` binaries must supply one. [D] https://doc.rust-lang.org/reference/runtime.html, https://rust-lang.github.io/rfcs/1974-global-allocators.html
- Contract: `alloc` with a zero-size `Layout` is UB; `dealloc` must receive the same `Layout`; **must not unwind**; re-entrancy: stick to `core`, avoid `std` (a `Mutex` allocates), but `std::thread_local`, `thread::current()`, `park/unpark` are explicitly safe to use inside it. [D] https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html
- **What it sees:** only `Layout` (size, align). No lifetime class, no owner, no "frame vs persistent". The `alloc` crate and every std collection route through it [D RFC 1974]; so does every third-party crate.
- **Optimizer caveat, load-bearing for the tree's counting gates:** the docs state the optimizer "may eliminate, move to stack, or work around allocations" and that counting allocations via side effects is *unsound* as a proof (`drop(Box::new(42)); allocator_call_count()` example). The ≥10 in-tree `CountingAlloc` gates (e.g. `alloc_frame_census.rs:120-148`, ecsnative) therefore bound from *below* only: a green count cannot prove zero allocations were *written*, only that none survived optimization on that build. [D] GlobalAlloc docs; [S] in-tree.
- **Windows-gnu TLS cost inside a global allocator:** PR #148799 (merged 2026-06-03, shipped 1.98.0) moved Windows thread-local destructor registration to FLS. The project's own KE16 finding (local memory note, not in-tree; recorded 2026-09-10) attributes 2 `lock` RMWs + `FlsSetValue` per `thread_local!` access on windows-gnu to #148799 + #157483; a per-thread cache in a global allocator would pay that on every call. [D] https://github.com/rust-lang/rust/pull/148799 (perf table there shows no instruction-count regression on Linux CI; the windows-gnu cost is the local measurement, flagged as such).
- **Integration of production allocators:** mimalloc-rs forwards `alloc→mi_malloc_aligned`, `alloc_zeroed→mi_zalloc_aligned`, `dealloc→mi_free`, `realloc→mi_realloc_aligned`, with an optional `nightly_allocator_api` module [S] purpleprotocol/mimalloc_rust `src/lib.rs`. tikv-jemallocator: `GlobalAlloc` (mallocx/rallocx/sdallocx per its docs) and optional nightly `Allocator` [D] docs.rs/tikv-jemallocator. The tree's `bench-alloc` feature already uses mimalloc this way (`boyko_ecs/Cargo.toml:36`, ecsnative).
- **Abstract-machine status is unsettled:** UCG issue #442 (opened 2023-08-08, still "Todo"): whether memory returned by a custom global allocator gets *fresh provenance*, whether allocator calls may be elided, whether only the registered global allocator is "magic". [D] https://github.com/rust-lang/unsafe-code-guidelines/issues/442

## Mechanism (3): arena crates and the frame/persistent pattern

| Crate | Backing | Allocator-parameterised? | Frame reset | Drop semantics |
|---|---|---|---|---|
| bumpalo `Bump` | chunks from the **global allocator**, doubling | Implements `Allocator` under `allocator_api` (nightly) or `allocator-api2` (stable); own `collections::{Vec,String}` and `boxed::Box` | `reset()` keeps the largest chunk | destructors NOT run (except `bumpalo::boxed::Box`); `!Sync` (`bumpalo-herd` for pools) |
| typed-arena | `Vec` chunks ("typically just a vector push") | no | no per-object free | all dropped at arena drop; `into_vec()` |
| slotmap | `Vec<(value, version)>`; 31-bit version; ABA-safe until 2³¹ reuses | no | n/a | normal |
| slab | (not fetched; not relied on) | — | — | — |

[D] docs.rs/bumpalo, docs.rs/typed-arena, docs.rs/slotmap. Conclusion that follows from the sources: **none of these removes the foreign allocator** — each is itself a client of the global allocator; only the `Allocator`-parameterised route (bumpalo's, or the std types) can be pointed at the memory library.

**How engines model lifetime classes (the thing `GlobalAlloc` cannot express):**
- **Unity DOTS**: `Allocator.Temp` (fastest; "each frame, the main thread creates a Temp allocator which it deallocates in its entirety at the end of the frame"; jobs get their own), `Allocator.TempJob` ("must deallocate ... within 4 frames", safety-checked for Native collections), `Allocator.Persistent` (slowest; "safety checks can't detect if a Persistent allocation has outlived its intended lifetime"). [D] Unity Collections 2.5 allocator-overview.
- **flecs**: per-world `ecs_allocator_t` = sparse set of size-class block allocators; `ecs_block_allocator_t` free-list with `chunk_size = ECS_ALIGN(size,16)`, `chunks_per_block = max(4096/chunk_size,1)`, blocks from `ecs_os_malloc`; `FLECS_USE_OS_ALLOC` bypass; `FLECS_SANITIZE` tracks owner per chunk; every internal vector takes the allocator explicitly: `ecs_vec_init(ecs_allocator_t*, ecs_vec_t*, size, count)`, growth `flecs_next_pow_of_2` via `flecs_realloc(allocator, ...)`. [S] flecs `src/datastructures/{block_allocator.c, allocator.c, vec.c}`.
- **EnTT** ≥3.11: `template<typename Entity, typename Allocator> class basic_registry` with `allocator_type`, ctor `basic_registry(const allocator_type&)`, `get_allocator()`; allocator propagates to pools (std allocator-traits rebind — the C++ `pmr`-style route). [D] skypjack.github.io basic_registry reference; [B] discussion #880 changelog.
- **Bevy**: `BlobArray` calls `alloc::alloc::{alloc, realloc, dealloc}` + `handle_alloc_error` directly; **no `A: Allocator`** parameter — Bevy is the "global allocator everywhere" design point. [S] bevy `storage/blob_array.rs:419-460`.
- **In Rust generally**: the "frame arena + persistent pool" split is done with `Bump::reset()` per frame + a long-lived allocator, threading either `&'a Bump` (lifetime infection) or `&dyn Allocator` [B nical]; std's design ("since this allocator is global across threads, we can't take `&mut self`" → `Allocator for &A`) is why a ZST-handle-to-static or `&'static` design avoids the lifetime parameter [D RFC 1974].

## Mechanism (4): Miri, loom, `unsafe` census

**Miri and a custom `#[global_allocator]`:** Miri *does* dispatch to the user's allocator — miri PR #1975 (merged 2022-03-20) exists precisely because "the allocator will attempt to deallocate memory allocated directly by Miri" when a `#[global_allocator]` is set; rust #138839 (2025) shows Miri reporting Stacked-Borrows UB *inside* a user `Memcheck` allocator's `dealloc` under libtest (closed not-planned; root cause was the allocator reading metadata after the buffer). [S] https://github.com/rust-lang/miri/pull/1975, https://github.com/rust-lang/rust/issues/138839. Earlier (#1207, 2020) Miri ignored them; that is stale. [S]

**Provenance granularity = the allocation.** `std::ptr` defines provenance as spatial/temporal/mutability permission inherited from an allocation's Original Pointer; "sub-ranges of one allocation are not distinguished"; in-bounds `offset` keeps provenance. Consequence (deduction from the definition, corroborated by the ASan analogy in arXiv 2206.11728 "the entire memory pool of apache is allocated from malloc as a large block"): inside one `VmReservation`, Miri cannot detect a freed slot being reused early, a row overlapping its neighbour, or a stale `row_ptr` — those are *inside* the live allocation. What Miri **does** still check on such memory: uninitialised reads (the `alloc_zeroed` fallback arm exists for this, `vm.rs:168-179`), alignment, data races, and Stacked/Tree-Borrows retag conflicts on references derived from it. [D] https://doc.rust-lang.org/std/ptr/index.html; [D] arXiv 2206.11728; [S] in-tree.

**Tree Borrows specifics relevant to sub-allocations:** bumpalo #187 — `bumpalo::boxed::Box::into_raw` then `&mut *raw` produced "trying to retag from … for Unique permission" under Stacked Borrows (fixed in #188). rust #138839 is the same shape inside a `GlobalAlloc`. Both confirm that references minted over allocator-returned bytes are retagged normally; the memory library's `row_ptr` provenance and the tree's existing "SAFETY proves the LAST ACCESS, Tree Borrows judges by PROTECTOR LIFETIME" finding (local memory note) apply unchanged to a `Vec<T, A>` whose buffer lives in a reservation. [S] https://github.com/fitzgen/bumpalo/issues/187.

**Miri flags that matter:** `-Zmiri-tree-borrows` replaces Stacked Borrows ("likely that the eventual final aliasing model … will be stricter than Tree Borrows"); `-Zmiri-strict-provenance` aborts on int→ptr casts (relevant if the memory library ever hands out integer offsets); `-Zmiri-ignore-leaks` disables the leak checker (a reservation-backed `Vec<T, A>` that is never `deallocate`d before the reservation's `Drop` is not a leak *to Miri* if the reservation is freed — but a reservation held in a `static` would be reported). FFI/syscalls unsupported → the `cfg(miri)` fallback arm stays mandatory. [D] miri README.

**loom:** models threads, atomics (C11 model, bounded), Mutex/RwLock/Condvar, `thread_local!`; anything not going through loom types is "invisible to loom". `loom::alloc` provides `alloc`/`alloc_zeroed`/`dealloc` (global allocator pass-throughs) and `Track<T>` ("Track allocations, detecting leaks"). It does **not** model an allocator's internal concurrency; a lock-free memory-library free list would need its atomics to be `loom::sync::atomic` under `cfg(loom)`, the same discipline `boyko_threadpool` already applies (`sync.rs:33-34`, merge). [D] docs.rs/loom, docs.rs/loom/alloc.

**`unsafe` census:** every `Vec::from_raw_parts_in`/`Box::from_raw_in` over reservation memory is an `unsafe` site requiring a `// SAFETY:` proving "currently allocated by this allocator + layout fits" (the Allocator contract above). The Allocator "memory fitting" clause permits passing a *larger* `layout.size()` on deallocate (up to the returned slice length), which is what lets a granule-rounded reservation hand back `NonNull<[u8]>` bigger than requested without the caller mis-sizing `deallocate`. [D] Allocator docs.

## Mechanism (5): ecosystem reality — "our code" vs "the process"

Verified runtime allocation through the **global** allocator in dependencies the shipped binary links:

| Dependency | Site | When it allocates | Source |
|---|---|---|---|
| crossbeam-deque `Worker` | `Buffer::alloc` = `Box<[MaybeUninit<T>]>`; `push` → `resize(2*cap)` when `len >= cap`; old buffer freed via `epoch::pin()` + `defer_unchecked` | every doubling of a lane deque | [S] deque.rs |
| crossbeam-deque `Injector` | `Block::new` = `Global.allocate_zeroed(LAYOUT)`; `BLOCK_CAP = LAP-1`; `push` allocates `next_block` when `offset + 1 == BLOCK_CAP` | every 31 pushes to the global injector; the tree pushes at `worker.rs:646` (merge) | [S] deque.rs |
| crossbeam-epoch | `Local::register` → `Owned::new(Local)` per pinning thread; `Global::queue: Queue<SealedBag>` is a Michael-Scott queue — **`Owned::new(Node)` on every push** (every sealed bag of 64 deferreds; 4 under Miri/sanitizers) | on buffer retirement | [S] internal.rs, sync/queue.rs |
| crossbeam-queue `ArrayQueue` | `Box<[Slot<T>]>` collected once in `new`; push/pop never allocate | construction only (`executor_scratch.rs:55-60`, ecsnative) | [S] array_queue.rs |
| std `thread::spawn` | `Arc::new(Packet{..})`, `Thread::new(id, name)`, `Box::new(rust_start)`, `Box::new(Box::new(rust_start))`, `Box::new(ThreadInit{..})` | per spawned thread (setup) | [S] std `thread/lifecycle.rs` |
| std panics | payload is `Box<dyn Any + Send>` (`Packet.result: Option<Result<T>>`) | per panic | [S] std lifecycle.rs; in-tree `scope.rs:534` (merge) stores `AtomicPtr<Box<dyn Any + Send>>` |
| criterion | dev-dependency only (`Cargo.toml` of every bench crate) — bench binaries, never the engine binary | n/a | [S] in-tree Cargo.toml lines above |

Therefore, from the mechanisms themselves:
- **"No foreign allocator in OUR code"** — deliverable by (a) `Vec<T, A>`/`Box<T, A>` (nightly now; stable MVP later) or in-house containers over the memory library, (b) rewriting the ~71 kernel fields and all locals, (c) a gate. `String`, `format!`, `BTreeMap`, `VecDeque`, `Rc/Arc` have no stable-track `A`; they must be replaced, not re-parameterised.
- **"No foreign allocator in the PROCESS"** — not deliverable by any per-type mechanism while crossbeam-deque/epoch and `std::thread` are linked. The only mechanism that *reaches* them is a `#[global_allocator]` implemented by the memory library (which then must satisfy `GlobalAlloc`: no unwind, zero-size UB, `Layout`-only, thread-safe via `&self`, `core`-only re-entrancy); that *routes* their allocations, it does not remove them. Removing them requires an in-house bounded work-stealing deque (Chase–Lev 2005 grows its circular array on overflow — the unbounded property *is* the allocation; Lê et al. 2013 give the C11-correct formulation the tree already reasons about at `worker.rs:668-671`) and no `Injector`-style linked blocks. [D] https://dl.acm.org/doi/10.1145/1073970.1073974, https://dl.acm.org/doi/10.1145/2442516.2442524.

## Gating mechanisms (what can mechanically enforce the ban)

| Mechanism | Scope it can express | Known limits (sourced) |
|---|---|---|
| clippy `disallowed-types` (`std::vec::Vec`, `alloc::vec::Vec`, `std::boxed::Box`, `std::string::String`) | fully-qualified paths; errors on unresolvable path unless `allow-invalid = true` | cannot distinguish test from production code — the tree measured ~1051 test sites for the analogous `disallowed-macros` and rejected it (`clippy.toml:32-63`, ecsnative). The WebFetch summary claiming "prelude types cannot be disallowed" is a small-model inference, not in the clippy doc; the doc says only "fully qualified paths" — **unverified either way; needs the same canary the tree ran at L8c** |
| clippy `disallowed-methods` (`core::iter::Iterator::collect`), `disallowed-macros` (`std::vec`, `alloc::format`) | same | same test-scope limit |
| `#![no_std]` without `extern crate alloc` | structural: `Vec`, `Box`, `String`, `format!`, `collect::<Vec<_>>` do not exist in the crate | must be per-crate; `std::thread`, `std::alloc::System`, TLS, I/O are also gone — only viable for kernel/library crates, not `boyko_app` [D] alloc crate docs |
| in-tree walker census (`print_census.rs`, `physics_vec_side_store_census.rs` pattern) | production-only, per-site reasons | text-based; the tree's own choice after rejecting clippy scoping [S] |
| counting `#[global_allocator]` tests | dynamic, per-frame | lower bound only — optimizer may elide allocations [D GlobalAlloc docs] |

## Comparative table

| Aspect | Bevy | flecs | EnTT | Unity DOTS | Rust std (nightly) |
|---|---|---|---|---|---|
| Allocator model | global allocator everywhere (`alloc::alloc` in `BlobArray`) | per-world size-class block allocator, explicit parameter on every vec op | allocator template param on registry, propagated to pools | lifetime classes Temp/TempJob/Persistent + custom | `A: Allocator` type param on Vec/Box(+VecDeque/BTreeMap/Rc/Arc nightly) |
| Lifetime classes | none | none (size classes) | none (std allocator traits) | yes, safety-checked (TempJob ≤4 frames) | none — encoded only by the type of `A` |
| Third-party reach | via global | via `ecs_os_api` hooks | via std allocator | n/a | only `#[global_allocator]` |
| Debug tooling | Miri (std) | `FLECS_SANITIZE` owner tag per chunk | ASan | Native collection safety checks | Miri (blind inside an arena) |

## Pitfalls and mistakes (sourced)

1. Counting-allocator gates read as proof; the docs say the optimizer may elide allocations. [D]
2. `Vec<T, &'a Arena>` infects every struct with `'a`; `&dyn Allocator` costs a vtable but was *faster* in one measured case (nical, 5–8%). [B]
3. `Allocator` blocks die when the allocator is dropped — a `Vec<T, A>` outliving its reservation is UB, so drop order in `EcsMaster` becomes a soundness invariant (the tree already orders `PoolBacking` last for this reason, `component_pool.rs:81-84`). [D][S]
4. `Box<T, A≠Global>` loses `noalias` (decided in #156882) — codegen of hot paths holding such boxes may change. [D]
5. Miri cannot see sub-allocation misuse inside a reservation; `-Zmiri-ignore-leaks` hides reservation leaks. [D]
6. A memory-library `GlobalAlloc` must never unwind and must handle every `Layout` up to page alignment (the std docs example rejects `align > 4096` with null) — a "loud OOM panic" policy like `VmReservation::reserve` (`vm.rs:109-119`) is *forbidden* inside `GlobalAlloc`. [D][S]
7. Nightly churn: the tree already documents `component_pool.rs` size pins breaking on newer nightlies. [S]

## Relevant academic / primary works

- Chase, Lev — "Dynamic Circular Work-Stealing Deque", SPAA 2005 — growable circular array; growth = allocation. [D]
- Lê, Pop, Cohen, Zappa Nardelli — "Correct and Efficient Work-Stealing for Weak Memory Models", PPoPP 2013 — C11 formulation crossbeam follows. [D]
- "There Ain't No Such Thing as a Free Custom Memory Allocator", arXiv 2206.11728 — sanitizers blind inside pool sub-allocators. [D]
- RFC 1974 (global allocators), RFC 1398 / tracking #32838 (allocator_api), RFC 3446 (Store). [D]
- Nethercote, Rust Performance Book, "Heap allocations" — `with_capacity`, reuse via `clear()`, `format!` always allocates, DHAT; "reducing allocation rates by 10 allocations per million instructions … ~1%". [D]

## Applicability to boyko-engine (facts only; design is the architect's)

- Directly usable now (nightly): `Vec<T, A>` / `Box<T, A>` over an `Allocator` implemented by the memory library; `hashbrown` with `A` if a map is ever justified; `bumpalo` is *not* a fit (allocates from the global allocator).
- Needs adaptation: `String`/`format!`/`BTreeMap`/`VecDeque`/`Rc`/`Arc` — no stable-track `A`; `boyko_utils::SparseMap` (3 `Vec`s) would need `A` or a reservation backing like `InlandStore`.
- Cannot be delivered by our-code mechanisms: crossbeam-deque/epoch and `std::thread` allocations — only a memory-library `#[global_allocator]` routes them; only replacing the deque removes them.

## Open questions for the architect

1. Which goal is the ruling read as — "our code" (per-type `A`) or "the process" (`GlobalAlloc` from the memory library, plus an in-house bounded deque)? The mechanisms differ entirely.
2. Lifetime classes: does the memory library grow Temp/Persistent classes (Unity model) or size classes (flecs model), given `GlobalAlloc` cannot carry either?
3. `A` as ZST handle to a `static` (no lifetime, no bytes) vs `&'a Reservation` (lifetime infection) vs `&dyn Allocator` (measured faster once): which, and measured how, given `Box<T, A>` loses `noalias`?
4. Gate: `#![no_std]` boundary for kernel crates vs clippy paths vs a walker census — the tree's L8c measurement already rejected clippy for test-scope reasons; does the same rejection apply here?
5. Miri coverage: what replaces Miri's use-after-free detection inside reservations (a `FLECS_SANITIZE`-style owner tag under `cfg(debug_assertions)`? Loom `Track`?).
6. Does the counting-gate family stay as a lower bound, or is a `GlobalAlloc` that *panics on any call* after boot (a "deny" allocator, which is forbidden to unwind — it would have to abort) the actual gate?

## Sources

[1] https://doc.rust-lang.org/std/alloc/trait.Allocator.html — [D] trait contract, dyn-compatible, `&A` impls
[2] https://github.com/rust-lang/rust/pull/156882 — [D] stabilise `Allocator`, FCP 2026-09-09, noalias/fundamental decisions
[3] https://hackmd.io/nNHdKkp1TTK7jat0I-ABqA — [D] stabilization report: exact stable surface / exclusions / no-unwind
[4] https://github.com/rust-lang/rust/issues/156906 — [D] `dyn Allocator` FCP, closed 2026-05-25
[5] https://doc.rust-lang.org/std/vec/struct.Vec.html — [D] `Vec<T, A>` API and allocation guarantees
[6] https://doc.rust-lang.org/std/boxed/struct.Box.html — [D] `Box<T, A>`, `CoerceUnsized`
[7] https://doc.rust-lang.org/std/collections/struct.VecDeque.html — [D] `VecDeque<T, A>`
[8] https://doc.rust-lang.org/std/collections/struct.BTreeMap.html — [D] `BTreeMap<K,V,A: Allocator + Clone>`, gate `btreemap_alloc`
[9] https://doc.rust-lang.org/std/string/struct.String.html — [D] no `A` parameter
[10] https://docs.rs/hashbrown/latest/hashbrown/struct.HashMap.html and https://github.com/rust-lang/hashbrown/pull/417 — [D][S] `A` via allocator-api2
[11] https://github.com/zakarumych/allocator-api2 and https://docs.rs/allocator-api2/latest/allocator_api2/ — [S][D] shim, 0.4.0 (2026-09-04), MSRV 1.64, nightly feature removed
[12] https://github.com/rust-lang/rfcs/pull/3446 and https://github.com/matthieu-m/storage/blob/main/etc/rfc.md — [D] Store API
[13] https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html — [D] contract, no-unwind, re-entrancy, counting-is-unsound
[14] https://doc.rust-lang.org/reference/runtime.html — [D] `#[global_allocator]` rules
[15] https://rust-lang.github.io/rfcs/1974-global-allocators.html — [D] one per program, System default, `&A` rationale
[16] https://github.com/rust-lang/unsafe-code-guidelines/issues/442 — [D] open opsem question on custom global allocators
[17] https://raw.githubusercontent.com/purpleprotocol/mimalloc_rust/master/src/lib.rs — [S] mi_* forwarding
[18] https://docs.rs/tikv-jemallocator/latest/tikv_jemallocator/ — [D] jemalloc integration
[19] https://docs.rs/bumpalo/latest/bumpalo/ — [D] chunking from global allocator, reset, features
[20] https://github.com/fitzgen/bumpalo/issues/187 — [S] Stacked Borrows retag conflict on arena Box
[21] https://docs.rs/typed-arena/latest/typed_arena/ ; https://docs.rs/slotmap/latest/slotmap/ — [D]
[22] https://nical.github.io/posts/rust-custom-allocators.html — [B] 5–8% generic regression, lifetime pollution
[23] https://github.com/rust-lang/miri/blob/master/README.md — [D] detections, limits, flags
[24] https://github.com/rust-lang/miri/pull/1975 — [S] Miri routes dealloc to user global allocator (2022-03-20)
[25] https://github.com/rust-lang/rust/issues/138839 — [S] Miri executing a custom allocator under libtest
[26] https://github.com/rust-lang/miri/issues/1207 — [S] historical (closed) "custom allocators ignored"
[27] https://doc.rust-lang.org/std/ptr/index.html — [D] provenance = per allocation
[28] https://arxiv.org/pdf/2206.11728 — [D] sanitizers blind inside sub-allocators
[29] https://docs.rs/loom/latest/loom/ and https://docs.rs/loom/latest/loom/alloc/index.html — [D] model scope, `Track`
[30] https://raw.githubusercontent.com/crossbeam-rs/crossbeam/master/crossbeam-deque/src/deque.rs — [S] Worker doubling via Box, Injector Block via `Global.allocate_zeroed`
[31] https://github.com/crossbeam-rs/crossbeam/blob/master/crossbeam-epoch/src/internal.rs and https://raw.githubusercontent.com/crossbeam-rs/crossbeam/master/crossbeam-epoch/src/sync/queue.rs — [S] Local registration, node per push
[32] https://raw.githubusercontent.com/crossbeam-rs/crossbeam/master/crossbeam-queue/src/array_queue.rs — [S] allocate-once
[33] https://raw.githubusercontent.com/rust-lang/rust/master/library/std/src/thread/lifecycle.rs — [S] spawn allocations
[34] https://doc.rust-lang.org/clippy/lint_configuration.html — [D] disallowed-* config, `allow-invalid`
[35] https://doc.rust-lang.org/alloc/index.html — [D] `no_std` + `extern crate alloc`, global allocator requirement
[36] https://github.com/rust-lang/rust/pull/148799 — [D] Windows TLS destructors via FLS, 1.98.0
[37] https://github.com/SanderMertens/flecs/blob/master/src/datastructures/{block_allocator.c,allocator.c,vec.c} — [S] flecs allocator
[38] https://skypjack.github.io/entt/classentt_1_1basic__registry.html — [D] EnTT allocator param
[39] https://docs.unity3d.com/Packages/com.unity.collections@2.5/manual/allocator-overview.html — [D] Temp/TempJob/Persistent
[40] https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/storage/blob_array.rs — [S] global allocator only
[41] https://dl.acm.org/doi/10.1145/1073970.1073974 ; https://dl.acm.org/doi/10.1145/2442516.2442524 — [D] Chase–Lev; Lê et al.
[42] https://nnethercote.github.io/perf-book/heap-allocations.html — [D]
[43] https://cetra3.github.io/blog/state-of-allocators-2026-part-2/ — [B] status narrative (recorded, not relied on)
[44] https://medium.com/@trivajay259/the-allocator-api-in-2026-rusts-almost-there-feature-that-refuses-to-land-d6f0f934d296 — [B] not relied on

---

# LENS C - this-engine

# Research report — Lens C: this engine's memory library as an allocator, and every allocation shape the engine makes

Trees read: **ecsnative** = `D:/wt/ecsnative` (`feat/ecs-native-storage` @ `ad0ebea4`), **merge** = `D:/wt/merge` (`merge/ke16-into-render` @ `d2c8c646`). Every in-tree citation is `[tree] path:line`. Nothing was run except `rg`/`git`/`sed`; no cargo, no timings. graphify was queried once (216-node subgraph, on-target) and then Grep/Read.

---

## 1. The memory library's CONTRACT as an allocator (read in full on ecsnative)

### 1.1 The primitives and their shapes

| Primitive | File | Serves | Element domain | Growth | Free / reuse |
|---|---|---|---|---|---|
| `VmReservation` | [ecsnative] `crates/boyko_ecs/src/ecs/memory/vm.rs:85-97` | one contiguous VA reservation; `reserve(len)` rounds to `COMMIT_GRANULE` = 64 KiB (`:109-111`); `commit(old,new)` monotone frontier, granule-aligned, debug-asserted (`:199-209`); `Drop` releases whole (`:266-297`) | bytes | caller-driven frontier commit only | **none** — no decommit, no free, no reset; release only at `Drop` |
| `ComponentPool` | [ecsnative] `memory/component_pool.rs:170-251` | fixed-stride type-erased rows + two 4 B/row tick sub-regions in ONE reservation `[pad|data|added|changed]` (`constants.rs:218-335`) | any `Layout` with `align ≤ 4096` (`:316-320`), stride 0 allowed (tag pools, `:614-615`, `:734-803`) | `#[cold] grow_rows` — doubling clamped `[64 KiB, 64 MiB]`, request-dominant (`:599-692`; policy `constants.rs:337-357`) | `swap_remove`/`pop` with `drop_fn` (`:1107-1224`); `clear_no_drop`/`truncate_no_drop`/`set_len_no_drop` for `Copy` only (`:995-1050`); committed pages never released before `Drop` |
| `VmColumn<T: Copy>` | [ecsnative] `memory/vm_column.rs:79-113` | typed address-stable growable column, lazy reservation | `T: Copy`, `size_of::<T>() > 0` AND `COMMIT_GRANULE % size == 0` (`:139-149`) — i.e. 1,2,4,8,16,32,64,… B only; 12 B / 24 B rejected at construction | `grow_to` doubling `[POOL_MIN_SLAB, POOL_MAX_SLAB]` (`:462-516`) | `swap_remove`/`truncate`/`clear` = `len` rollback, no destructor (`:277-306`, `:420-444`) |
| `InlandStore` | [ecsnative] `core/entity/inland_store.rs:84-108` | `EntityInland` (16 B) slots, zero-tail invariant J (`:13-31`) | one fixed type | `ensure(n)` (`:162-171`), slabs 256 KiB…16 MiB | `clear` memsets `[0,len)` (`:227-241`); no partial re-zeroing ever (`:33-38`) |
| `ScratchColumn<T: Copy>` | [ecsnative] `core/component/scratch/scratch_column.rs:43-48` | `ComponentPool::new_untracked` (ticks reserved, never committed — `component_pool.rs:455-522`) | `T: Copy`, non-ZST (`:88-109`) | via pool `grow_rows`; `resize`/`extend_fill_copy` (`:274-286`) | `clear` = `len = 0`, pages stay resident (`:237-242`) |
| `LogRing` | [ecsnative] `core/log/ring.rs:103-117` | a byte ring (`VmColumn<u8>`, 512 KiB) + a 16 B line column (`VmColumn<LogLine>`, 4096 slots) | — | materialised once to full capacity (`:341-346`) | ring wrap with eviction (`:281-335`) — **the existence proof that a variable-length byte arena runs on the library** |
| `DeviceColumn` | [ecsnative] `memory/device_column.rs:51-62` | GPU-resident forward seam, `Box<DeviceColumn>` inside `PoolBacking::Device` (`component_pool.rs:80-94`) | — | Phase-5 stub | — |

`memory/mod.rs` exports exactly these (`[ecsnative] memory/mod.rs:1-5`); `utils.rs` is one `align_up` (`:3-5`).

### 1.2 What the library does NOT serve today (each a shape the inventory in §3 needs)

1. **Droppable elements in a typed column.** `VmColumn` is `T: Copy` by design — "deliberately NOT a general `Vec` replacement for droppable `T`" (`[ecsnative] vm_column.rs:24-30`). `ComponentPool` runs `drop_fn` but is type-erased and costs a full pool reservation + tick regions.
2. **Element sizes that do not divide the granule** (12 B, 24 B, 40 B tuples): rejected at `VmColumn::new` (`vm_column.rs:144-149`). The packing plan tightens the divisor to `COMMIT_PAGE` = 4096 (`[ecsnative] docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md:336-337`), which admits 16 B and 32 B but still rejects 12/24/40.
3. **Variable-size allocation with free/reuse of arbitrary blocks.** The engine HAD this — the shared `Arena` + `MemFreeBlockMaster` best-fit free-block tracker — and deleted it in Phase X.J as *client-less*, not as broken (`[ecsnative] git 1ee2c461`; `docs/archive/PHASE-XJ-RESULTS.md:16`).
4. **Per-frame reset of a whole region** (bump + reset). `clear` on every primitive is a `len = 0` or a memset, never a decommit; `MEM_RESET`/decommit is explicitly forbidden for `InlandStore` (`inland_store.rs:37-38`) and out of scope in the packing plan ("Non-goals: … decommit", `POOL-SUBGRANULAR-PACKING-PLAN.md:31-33`).
5. **Nested / ragged collections** (`Vec<Vec<T>>`, CSR) — nothing.
6. **Small persistent structures** — see the economics below: every structure costs its own reservation.
7. **Thread-safe concurrent allocation** — every primitive is `&mut self` for mutation and `!Send/!Sync` by `NonNull`; owners add `unsafe impl` with an exclusivity argument (`vm.rs:82-84`, `vm_column.rs:70-78`, `ring.rs:119-190`).

### 1.3 Resident-floor economics (numbers from the tree, not from memory)

* Granule = 64 KiB everywhere (`[ecsnative] constants.rs:1-7`). Reservation defaults: pool data 1 GiB VA / ticks 64 MiB each (`constants.rs:32-60, 81-85`); inland 1 GiB (`:363-372`); a `VmColumn<EntityId>` per archetype = 128 MiB VA (`:47-51`).
* **The per-column resident floor is 384 KiB for 63 of 64 component ids, 192 KiB for the 64th**, because `commit_subregion` rounds each staggered sub-region out to two granules (`[ecsnative] component_pool.rs:560-574`, derivation `POOL-SUBGRANULAR-PACKING-PLAN.md:15-21`, table `:37-50`). The tree's own prose still says 192 KiB in three places (`constants.rs:99-103`, `component_pool.rs:464-471`, `scratch_column.rs:14-19`) — the plan names them as false (`POOL-SUBGRANULAR-PACKING-PLAN.md:512-517`).
* Untracked column (`ScratchColumn`): 128 KiB; `VmColumn<EntityId>`: 64 KiB; tracked ZST: 256 KiB (plan table `:39-44`).
* Worked examples in the plan: `ButtonBundle` archetype ≈ 2.94 MiB at one widget; Gaia 4-column table ≈ 1.6 MiB for 2 KB of payload; 1000 sparse archetypes × 4 columns ≈ 1.5 GiB of Windows commit charge (`:23-27, 45-49`).
* Steady-state overshoot: +180 KiB per column of `align_up` slack, permanent (`:50, 139-141`).
* The plan's remedy (DESIGN, rev 2, no code): commit quantum → `COMMIT_PAGE` = 4 KiB, absolute page-floor ladder; floor 384 → 12 KiB tracked / 4 KiB untracked / 4 KiB `VmColumn` (`:88-149`). One reservation per column stays; sharing a reservation between columns was rejected on two measured grounds — the `pool_base_stagger` cache-set guarantee (a ~40 % rigid-solver regression at P2, `constants.rs:181-189`) and false sharing between concurrently-scheduled writers (`:191-230`). **Note the ground: both objections are about hot SoA columns swept at index `i`; neither applies to cold bookkeeping (free lists, id maps).**
* Cost per reservation beyond bytes: ≤ 6 VMAs/VADs per pool, `vm.max_map_count` 65,530 (`constants.rs:40-58`); Miri/wasm fallback arm eagerly `alloc_zeroed`s the FULL `os_len` (`vm.rs:168-179`, `constants.rs:61-69`), so under Miri each small VM-backed structure is a ≥ 64 KiB heap allocation.
* OS truth for the quantum claim [D]: Microsoft's `VirtualAlloc` page — reserve address "is rounded down to the nearest multiple of the allocation granularity"; when committing inside a reservation "the address is rounded down to the next page boundary", and `dwSize` "is rounded up to the next page boundary"; committed pages read zero (https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc). This is exactly the plan's D1 premise.

### 1.4 The hot-path invariants any replacement must keep (these are why the library is shaped as it is)

* Write-once base pointers; growth never moves bytes — `row_ptr` is one `add` from `self.buffer` (`[ecsnative] component_pool.rs:816-834`); every cached `*const T` in query fetches and every `ScratchSolveView` copy held by a worker rests on it (`scratch_column.rs:28-38`). Relocation is the SP4 root cause (`docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:19`).
* Single warm compare `len >= committed_rows` in `add`/`push_copy` (`component_pool.rs:855-861, 958-961`); `ComponentPool` size-pinned at 128 B (`:55-64`).
* Zero-fill contract: never-written tick slots read `Tick::ZERO` (`vm.rs:19-37`, `component_pool.rs:423-432`).
* SIMD-A1: data base `SIMD_BUFFER_ALIGN`-aligned after the stagger (`component_pool.rs:349-366`).
* Cohort contract: columns swept together must have ids pairwise distinct mod 64 (`constants.rs:196-213`).

---

## 2. History — what was built, what was retired, and the measured reason (so the design does not re-propose it)

| Phase / commit [ecsnative] | What | Why (measured) |
|---|---|---|
| X.B `af342c68` | deleted `ComponentPool.units: Vec<Unit>` (per-row `*mut u8` cache) → `row_ptr = base + i*stride` | 8 B/row saved, one heap Vec gone, net-removes unsafe (`docs/archive/PHASE-XB-RESULTS.md:9-24`) |
| X.C `546f3c39` | Arena backing: eager 64 MB `alloc` → `VirtualAlloc`/`mmap` | `Arena::new` 23-75 µs → 1.1 µs |
| X.F `616b0896` | Arena: 4 GiB reserve + lazy 2 MiB…64 MiB slab commit; addresses never move | growth-crossing spawn 1.75× Bevy; Bevy re-copies ~581 MB across doublings, boyko copies zero |
| X.G `adbe19c7` | `EntityMaster.entities_inland: Vec<EntityInland>` → `InlandStore` on new `VmReservation` | "the engine's LAST realloc-doubling"; spawn path 15-54 % faster; the #285/#580 doubling-chain spikes deleted |
| X.H `fb7cf1e0` | Arena rebuilt on `VmReservation` | −160 lines of duplicated unsafe |
| X.I `f9fb5a03` | per-pool `VmReservation` `[data|added|changed]`, self-growing pools, chunk machinery deleted, the two per-pool 256 KiB tick `Box`es deleted | no more hard row ceiling; O(1) growth; ticks zero-init free |
| X.J `1ee2c461` | **Arena + `MemFreeBlockMaster` (best-fit free-block tracker, BTreeMap start/end maps) DELETED** (−2,999 LOC) | client-less after X.I. Retirement option chosen over "fallback pools on arena" because forking `ComponentPool` per cfg arm would make Miri exercise different control flow than native (`docs/archive/PHASE-XI-PLAN.md:125-129`). **Not retired for fragmentation or perf** — that matters for §5. |
| F1-F4 `3b3c86f6` | `VmColumn<T: Copy>` primitive; `Archetype.entity_ids` and `DenseStore.s2e` moved onto it; `e2s` → flat sentinel `Vec<u32>` (28→4 B/entry); dense ceiling 1024 → `pool_reserve_rows` | audit `docs/MEMORY-SYSTEM-AUDIT.md:18-23` |
| untracked `0cb082bb` | `new_untracked` sentinel `ticks_committed = usize::MAX` (zero bytes, zero branches) | scratch columns stop paying 8 B/row + 2×64 KiB for unread ticks; the "obvious" `tick_len = 0` design was unsound (`component_pool.rs:37-43`) |
| L5 `3372ef0c` | `LogRing` on two `VmColumn`s | Principle 0 "even inside a Resource" (`ring.rs:84-88`) |
| 2026-09-09 `0a803cfc`, `2d665c85` | `ScratchColumn` resize/truncate + cached-frontier build view | "the column now beats std::Vec" |
| 2026-09-10 (DESIGN only) | `POOL-SUBGRANULAR-PACKING-PLAN.md` rev 2 | floor 384 → 12 KiB; no code yet |

`DenseStore` is the unfinished-migration proof the brief names, confirmed: `s2e: VmColumn<EntityId>` (`[ecsnative] dense_store.rs:118`) sits beside `e2s: EntitySlotMap` (a `Vec<u32>`, `entity_slot_map.rs:40`), `live: LiveBitmap` (`Vec<u64>`, `live_bitmap.rs:31`) and `free: Vec<u32>` (`dense_store.rs:126`); the module doc calls those three "the legitimate `std::Vec` exception the Dense plan grants this module" (`:77-82`) and F4 chose a 1024-row heap floor for them precisely because "the bookkeeping arrays are real heap, so they must NOT mirror the column reserve" (`:36-45`) — i.e. the reason they stayed on `Vec` is the library's per-structure resident floor (§1.3), not a preference for `Vec`.

---

## 3. Inventory of allocation shapes

### 3.1 Per-crate counts (matching LINES in `src/`, excluding `tests/`, `benches/`, `examples/` directories; in-file `#[cfg(test)]` modules are NOT excluded — see 3.2 for the kernel with that correction)

Command (run from `D:/wt/ecsnative`):
```
for c in crates/*/; do f() { rg -n --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/examples/**' -e "$1" "$c/src" | grep -v '^\s*//' | wc -l; }; printf ... "$(f 'Vec::new\(\)')" "$(f 'Vec::with_capacity')" "$(f '\bvec!\[')" "$(f '\.collect(::<[^>]*>)?\(\)')" "$(f '\bformat!\(')" "$(f '\.to_string\(\)|String::from\(|\.to_owned\(\)')" "$(f 'Box::new\(|Box::<')" "$(f ': String\b|String::new|String::with_capacity')" "$(f '\bArc<|Arc::new')" "$(f 'VecDeque|BTreeMap|BTreeSet|LinkedList')"; done
```

| crate | `Vec::new()` | `with_capacity` | `vec!` | `collect()` | `format!` | `to_string`/`String::from` | `Box::new` | `String` | `Arc` | `VecDeque`/`BTree*` |
|---|---|---|---|---|---|---|---|---|---|---|
| boyko_ecs | 88 | 59 | 97 | 92 | 7 | 8 | 89 | 3 | 64 | 5 |
| boyko_physics | 34 | 39 | 53 | 33 | 22 | 0 | 0 | 1 | 0 | 0 |
| boyko_render | 29 | 14 | 69 | 52 | 54 | 6 | 0 | 6 | 0 | 1 |
| boyko_rhi_vulkan | 8 | 35 | 50 | 12 | 5 | 0 | 5 | 1 | 0 | 0 |
| boyko_ui | 30 | 14 | 3 | 2 | 12 | 7 | 1 | 5 | 5 | 0 |
| boyko_app | 15 | 11 | 8 | 6 | 41 | 20 | 1 | 26 | 0 | 0 |
| boyko_threadpool | 3 | 21 | 2 | 10 | 2 | 1 | 6 | 0 | 94 | 0 |
| boyko_shaderdsl | 49 | 5 | 1 | 3 | 163 | 15 | 0 | 28 | 0 | 0 |
| boyko_log | 8 | 1 | 7 | 4 | 9 | 3 | 3 | 11 | 4 | 0 |
| boyko_fontbake | 9 | 8 | 34 | 3 | 0 | 0 | 0 | 0 | 3 | 0 |
| boyko_image | 9 | 5 | 26 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| boyko_input | 2 | 5 | 6 | 2 | 25 | 3 | 0 | 6 | 0 | 0 |
| boyko_scene | 9 | 0 | 5 | 3 | 0 | 0 | 1 | 0 | 0 | 0 |
| boyko_serialize | 6 | 11 | 1 | 3 | 0 | 0 | 0 | 0 | 0 | 0 |
| boyko_utils | 6 | 6 | 0 | 7 | 0 | 0 | 0 | 0 | 0 | 0 |
| boyko_sdf_math | 5 | 4 | 2 | 1 | 0 | 0 | 0 | 0 | 0 | 2 |
| boyko_demo | 3 | 7 | 3 | 0 | 5 | 0 | 2 | 0 | 7 | 0 |
| boyko_diag | 4 | 2 | 5 | 3 | 1 | 1 | 0 | 0 | 2 | 0 |
| boyko_rhi | 3 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| boyko_macros (proc-macro) | 29 | 12 | 6 | 50 | 30 | 31 | 3 | 3 | 0 | 2 |
| aether_lang (compiler) | 30 | 5 | 6 | 24 | 90 | 62 | 4 | 4 | 0 | 0 |
| boyko_math, aether, aether_tests, bench_bevy_vs_boyko, profile_fixture* | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

`Vec`-typed field/param declaration lines per crate (same exclusions; regex `^\s*(pub(\([a-z]+\))?\s+)?[a-z_][a-z0-9_]*\s*:\s*(Option<)?Vec<`): boyko_ecs 51 · boyko_ui 38 · boyko_physics 38 · boyko_rhi_vulkan 27 · aether_lang 27 · boyko_app 19 · boyko_macros 16 · boyko_render 15 · boyko_fontbake 11 · boyko_demo 9 · boyko_utils 6 · boyko_scene 6 · boyko_serialize 4 · boyko_image 4 · boyko_input 3 · boyko_diag 3 · boyko_sdf_math 2 · boyko_shaderdsl 1 · boyko_rhi 1 · threadpool/math/log 0. **Total 281; engine-runtime crates only (minus aether_lang, boyko_macros, boyko_demo) = 229**, consistent with the brief's "~245" to within regex tolerance (the brief's regex is not stated). The regex misses tuple structs: one in the kernel, `pub struct Children(Vec<Entity>);` (`[ecsnative] core/hierarchy/mod.rs:120`), none in physics/render/ui/scene.

### 3.2 The kernel (boyko_ecs), exhaustively — fields, classified by LIFETIME from the growth sites

Non-test count (field lines before each file's first `#[cfg(test)]`, awk-filtered; command in the run log): **51 `Vec<` field lines + 1 tuple struct + 36 `Box`/`Arc`/`String`/`HashMap` field lines = 88 alloc-typed fields.** (The brief's "~62 Vec fields" and my 51+1 differ by regex; the 51 include a handful of fn-parameter lines the pattern cannot distinguish from fields.)

**A. Structural-change-only (grow on spawn/despawn/register/link; steady frames touch them read-only or LIFO):**
- `EntityMaster.free_entity_ids: Vec<EntityId>` — `Vec::new()` at `entity_master.rs:85`, `push` on despawn `:378`, pop on allocate. **LIFO = reuse order = determinism data.**
- `DenseStore.free: Vec<u32>` — `with_capacity(min(reserve,1024))` `dense_store.rs:177`, push `:450`; LIFO (`:124-126`).
- `EntitySlotMap.slots: Vec<u32>` — `resize(id+1, ABSENT)` `entity_slot_map.rs:73` — **entity-id-scaled**, 4 B/id.
- `LiveBitmap.words: Vec<u64>` — `resize(word+1)` `live_bitmap.rs:53`.
- `Archetype.component_ids` (`archetype.rs:196`, push `:513`); `ArchetypeBundle.{slots: Box<[…;1024]> new_uninit :219, id_to_slot resize :568/:689, free_slots push :724}`; `ArchetypeRegistry.active_patterns` + `SparseMap<Vec<…>>` groups (`archetype_registry.rs:14, 65-78`); `ComponentPoolBundle.pools: Vec<ComponentPool>` (`component_pool_bundle.rs:13`).
- Observers: `EntityObserverStore.{inner: Option<Box<…>>, arena: Vec<EntityObserverList>, free_list, ever_custom}` (`entity_store.rs:111-136`, lazy `Box::new` `:150`, pushes `:194-357`); `TriggerLists.by_trigger: Vec<Vec<TriggerEntry>>` (`trigger.rs:170-176`); `observers/mod.rs:164`.
- `DenseRegistry.{slots: Option<Box<[Option<DenseStore>]>>, dense_ids}` (`dense_registry.rs:78-83`).
- `Children(Vec<Entity>)` — a component whose elements spill to heap per parent (`hierarchy/mod.rs:120`; audit `MEMORY-SYSTEM-AUDIT.md:75, 87` calls it v1-deliberate with a dense backing reserved as a type swap).
- `EnableStore.{pages: Box<[Option<Box<EnablePage>>]>, summary: Box<[AtomicU64]>}` — 512 B page per 4096 rows, `Box::new` on first toggle (`enable_store.rs:75, 163-173, 198-233`); `SmallList4` spills to `Vec::with_capacity(8)` past 4 enable terms (`:797-814`) — at query construction.
- Assets: `Assets.free: Vec<u32>` + `slot_word`/`refcount` pushes (`assets.rs:205, 239, 337-393, 573, 1034`); `Staging.queue` (`staging.rs:58-71`); `path_index.debug_paths: Vec<(u64, String)>` (`path_index.rs:86`) — load path.

**B. Per-frame written, capacity retained (frame-transient content, persistent buffer):**
- `CommandQueue.{bytes, panic_recovery}: Vec<MaybeUninit<u8>>` — `Vec::new()` (`command_queue.rs:111-113`), `bytes.reserve(total)` per push (`:154`), drained at the apply window. Amortised: realloc only when a frame's command bytes exceed the retained capacity (burst). The comment cites Bevy PR #6391 for the byte-arena shape (`:60-61`).
- `ErasedKindBuffer.data: Vec<MaybeUninit<u8>>` — `resize(old_len + element_size)` per push (`erased_buffer.rs:109, 176`); owner not read here.
- `ExecutorScratch.{exclusive_to_run, to_spawn}: Vec<SystemIndex>` — `with_capacity(system_count)` at build (`executor_scratch.rs:347-348`), reused per run; `pred_remaining: Box<[u16]>` build-time (`:323-325`); `Box::new(CompletionChannel)` at build (`:330`).
- `traverse_iter.rs:51 words: Vec<u64>`, `:284 stack: Vec<(Entity, usize)>` — relation-traversal iterator scratch; **not read in full — flagged as a possible per-call allocation** (12/24 B tuple elements, which `VmColumn` cannot hold).

**C. Setup-once / build products (allocated at `App`/`Schedule` build, read per frame):**
- `ScheduleBuilder`: 7 `Vec` + `Arc<ThreadPool>` + **5 `HashMap`** (`schedule_builder.rs:103-144`; the HashMaps are the allowed build-time exceptions) + `Box<dyn FnOnce(&mut EcsMaster)>` (`:92`).
- `Schedule`: `pool: Arc<ThreadPool>`, `systems: Vec<SystemBox>` (each a `Box<dyn System<Out=()>>`, `system_box.rs:83`), `system_conditions: Vec<Vec<BoolSystem>>`, `system_gating_sets: Vec<Box<[SystemSetId]>>`, `set_conditions`, `state_entries` (`schedule.rs:116-187`). `ConflictGraph.{pred_count: Box<[u16]>, successors: Box<[Box<[SystemIndex]>]>, conflict_bits: Box<[FixedBitSet]>}` (`conflict_graph.rs:68-77`). `SystemDescriptor.{ordering_hints, sets, conditions}` (`system_descriptor.rs:50-62`). `SystemMeta.gpu_intent: Option<Box<GpuAccessIntent>>` (`system_meta.rs:140`). `FilteredAccessSet.bit_owners: Box<[&str; N]>` (`filtered_access_set.rs:127`).
- `Resources.slots: Box<[…; RESOURCE_SLOT_COUNT]>` (`resources.rs:103`); `NonSendResources.slots` (`nonsend_resources.rs:91`); `EventDispatcher.{per_lane_overflow_count: Box<[u32]>, slots: Box<[…; MAX_EVENTS]>}` (`event_dispatcher.rs:117-136`); `EventBuffer` lanes `Box<[MaybeUninit<E>]>` fixed at preregister, overflow counted never realloc (`event_buffer.rs:119, 238, 243`; audit `:78`).
- `EcsMaster.{nonsend_resources: Option<Box<…>>, bundle_archetype_cache: OnceLock<Box<[OnceLock<ArchetypeId>; 1024]>>, query cache slots: Box<[OnceLock<…>]>}` (`ecs_master.rs:127, 198, 340`); `BundleColumnCache.slots` (`bundle_column_cache.rs:193`).
- `QueryState.matched_ids: Vec<ArchetypeId>` `with_capacity(16)` (`query_state.rs:64, 119`); query `state.rs:89 culled_ids` built at `new` (`:266-269`), recomputed on structural epoch; `TermList { ids: Box<[ArchetypeId]> }` rebuilt as `Box::new` per (tag-terms, generation) epoch (`term_list.rs:141, 155-170`).
- `App.{startup: Vec<StartupSystem>, plugin_type_ids, pool: Arc<ThreadPool>}` (`app.rs:175-191`); registries `component.rs:300, 352`.

**D. Load / clone / serialize / error paths (cold):** `prefab.rs:281, 284, 364`; `serialize/mod.rs:288 Vec<(u64, Entity)>` (12 B tuples); `load_writer.rs:174`; `asset/error.rs:30, 48 String`.

**Kernel local-allocation sites** (`Vec::new/with_capacity/vec!/collect/Box::new/format!/to_string/clone/into_boxed_slice/Arc::new`, per file, incl. test modules): `schedule_builder.rs` 68, query `state.rs` 50, `schedule.rs` 34, `iter.rs` 27, `enable_store.rs` 16, `chunk_iter.rs` 14, `archetype_registry.rs` 14, `executor_scratch.rs` 13, `conflict_graph.rs` 13, `archetype.rs` 13, … (full list in the run log). Reading the boundaries: `schedule.rs`'s 34 are ALL after `#[cfg(test)]` at `:1577` except the three `empty_intent.clone()` at `:1428-1445` (build path); `state.rs`'s 50 are all test except `:269` (construction); `iter.rs` (test module from `:1129`), `chunk_iter.rs` (`:351`), `par_chunk.rs` — every hit is in tests. **The kernel's `Schedule::run` body contains no `Vec`/`Box`/`format!` site of its own on ecsnative**; the per-frame allocations it makes come from the threadpool (next).

### 3.3 The scheduler and threadpool on merge (`d2c8c646`) — the per-frame allocator users that are NOT in the kernel's own storage

- `Schedule::run` (`[merge] schedule.rs:275`) enters `pool.install(|scope| …)` at `:455`; every `par_iter` enters `pool.scope` (`[merge] par_chunk.rs:139`). Each entry does `Box::new(ScopeShared::new(…))` (`[merge] thread_pool.rs:278-281` for `install`, `:328-331` for `scope`), freed by `Box::from_raw` in `Scope::drop` (`[merge] scope.rs:1031-1038`). **One heap alloc + one free per schedule run and per `par_iter` call, by construction.**
- Task cells: `Box<dyn FnOnce>` per task was REPLACED by an in-scope bump allocator `ScopeBlock` (`[merge] block.rs:1-60`): chunks of `4096 << e` bytes, 64-aligned, obtained with `std::alloc::{alloc, dealloc}` directly (`block.rs:58`), "one `alloc` per chunk and one `dealloc` per chunk at the join" (`:26-30`). So a frame that spawns tasks pays ≥ 1 chunk alloc + free per scope. The `alloc_frame_census.rs` header states the same shape: "one `Box<ScopeShared>` plus one heap cell per chunk, per call, by construction" (`[ecsnative] crates/boyko_physics/tests/alloc_frame_census.rs:965-966`).
- **The deques are NOT in-house.** `boyko_threadpool` depends on `crossbeam-deque = "0.8"` and `crossbeam-utils = "0.8"` on BOTH trees (`[merge] crates/boyko_threadpool/Cargo.toml:21-22`; `[ecsnative] same file :21-22`; lockfile `crossbeam-deque 0.8.8`, `crossbeam-epoch 0.9.21`, `crossbeam-utils 0.8.23`, `[ecsnative] Cargo.lock:1017-1046`); `Injector`/`Stealer`/`Worker`/`Steal`/`Backoff`/`CachePadded` are imported at `[merge] thread_pool.rs:37-38, worker.rs:17-18, tls.rs:96, scope.rs:38-39`. CLAUDE.md's "`boyko_threadpool` (Chase-Lev work-stealing)" and the Cargo description "Custom Chase-Lev work-stealing thread pool" describe the policy layer; the deque buffers themselves allocate through the global allocator inside crossbeam (not read here — [S] would require reading crossbeam's source; stated as unverified). Persistent `Arc<[…]>` tables: `injector_local`, `stealers`, `workers` (`[merge] thread_pool.rs:128-135`), `Arc<PoolInner>` (`:401`), built once (`:672-792`).
- The scheduler's own fields on merge are identical to ecsnative (`[merge] schedule.rs:116-187`, `executor_scratch.rs:219-282`, `conflict_graph.rs:68-77`, `schedule_builder.rs:92-144`).

### 3.4 Other crates, by sample (fields read with their headers; lifetime from `docs/MEMORY-SYSTEM-AUDIT.md`'s cross-crate table where it already classified the site)

- **boyko_physics (38 field lines by my regex; the gate measures 34 outside tests):** `SoftBody` 30 `Vec<f32>/Vec<u32>/Vec<u8>` columns (`[ecsnative] soft/component.rs:71-177`) — durable per-particle payload inside a Component, constructor-frozen (audit `:127, 149`); `IslandSleep.{asleep, below_count, frozen_islands, energy}` (`resources.rs:3069-3085`) — per-row latch/debounce + per-island frame scratch; the 4 in `solver/colored_tests.rs` are test-only. Everything else migrated: "202 `ScratchColumn` references" (`[ecsnative] tests/physics_vec_side_store_census.rs:236-240`).
- **boyko_render (15):** GPU orphan/retire lists (`bindless.rs:73-74`, `retired_gpu_buffers.rs:53-59, 251`, `mesh_assets.rs:712`, `texture.rs:928`) — persistent, structural; `mesh_data.rs:28-30`, `texture_data.rs:28` — load-path CPU asset payload; `gpu_column.rs:532 meta` — per-(archetype,component) side table (audit `:124` OK-by-design); `ui/pack.rs:143-150` — frame-transient clear+refill, generation-gated (audit `:137`); `vg_census.rs:54-66` diagnostics.
- **boyko_ui (38):** frame scratch `Vec<Vec<Entity>>` pools and measured/child/roots lanes (`resources.rs:216-254`, `world/pick.rs:110-120`, `interaction/focus.rs:117-136`, `binding/bind_system.rs:38-50`, `widgets.rs:73-75`) — clear+refill per frame (audit `:141`); text AST/lower/report (`text/ast.rs:73-114`, `lower.rs:67-69`, `report.rs:27-30` with `String`) — load-path; `reload/*` — hot-reload path.
- **boyko_rhi_vulkan (27):** FrameGraph SoA arenas ~15 lanes preallocated once, reset retains capacity (audit `:142`); brick atlas staging (audit `:143`).
- **boyko_app (19 fields, 41 `format!`, 26 `String`):** host loop, profiling artifact writer, window titles — boot/diagnostic paths (not read individually).
- **boyko_shaderdsl (163 `format!`, 49 `Vec::new`):** the HLSL printer — an offline code generator, not a frame path.
- **boyko_threadpool (94 `Arc` lines):** 12 are the persistent pool tables above; the rest are `Arc::new(Atomic*)` in test modules (`[merge] worker.rs:912-1342`, `scope.rs:1821-2328`, `tls.rs:642-732`, `task/detached.rs:170-206`).

---

## 4. Gates that exist today, and the frame-allocation census

**4.1 Static gates.** `clippy.toml` `disallowed-types` bans `HashMap/HashSet/Mutex/RwLock/Rc/RefCell` + parking_lot/hashbrown forward tripwires — **`Vec` is not on the list** (`[ecsnative] clippy.toml:13-30`). The same file records that a lint cannot be scoped to production code: `disallowed-macros` was measured at ~1051 test/bench sites vs six production exceptions and rejected (`:32-64`) — the identical arithmetic applies to a `Vec` type ban. Clippy's `disallowed_types` checks `use` items, type annotations (`check_ty`) and trait refs [S: `clippy_lints/src/disallowed_types.rs`, "Denies the configured types in clippy.toml"; warn-by-default, fires only when configured].

**4.2 The census gate that DOES exist, physics only.** `[ecsnative] tests/physics_vec_side_store_census.rs` (root `tests/`, 1172 lines, landed in `ad0ebea4`): scans `crates/boyko_physics/src` for struct fields whose declared type mentions `Vec<` outside `#[cfg(test)]`, and requires the set to equal an EXACT roster — `IslandSleep` 4 fields, `SoftBody` 30 fields, each with the rung that removes it (`:249-269`); floors `MIN_CONTAINERS = 35` etc. (`:277`); five red-first mutations pasted (`:114-160`). Its stated blind spots: locals/params/returns invisible by design (`:41-44`), text match not type resolution (`:45-52`), `#[cfg(not(test))]` read as test (`:67-75`), hand-wrapped generics (`:76-83`). This is the decidable template a workspace-wide field census would generalise.

**4.3 The counting-allocator gates.** Files declaring `#[global_allocator]`: 65 (`rg -l global_allocator` over src/tests/benches) — 30 benches, 22 test binaries (boyko_app 1, boyko_ecs 1, boyko_input 3, boyko_physics 8 incl. the new census, boyko_render 2, boyko_scene 1, boyko_ui 6), and 4 in `src` (`boyko_app/src/profiling/alloc_shim.rs` feature-gated `profiling-alloc`, `:139-147`; `boyko_app/src/runner.rs` test module; `boyko_scene/src/camera.rs`; `boyko_threadpool/src/block.rs` `RecordingAlloc` under `cfg(test)`, `:528-610`). Shapes differ: `colored_solve_zero_alloc_o5.rs` counts in a `thread_local!` `Cell` (`:281-284`) — blind to worker threads; `boyko_app/tests/zero_alloc.rs` and `boyko_ui/tests/zero_alloc.rs` use process-global atomics (`:47`, `:77`). The `profiling-alloc` shim documents the one-per-binary conflict as measured (`alloc_shim.rs:139-144`) [D: std docs — "`#[global_allocator]` can only be used once in a crate or its recursive dependencies", https://doc.rust-lang.org/std/alloc/index.html].

**4.4 The frame-allocation census workflow.** Its test HAS landed on ecsnative as an **untracked, uncommitted** file: `crates/boyko_physics/tests/alloc_frame_census.rs` (`git status`: `?? crates/boyko_physics/tests/alloc_frame_census.rs`). It installs a process-global counting allocator (`:99-148`), proves the counter live and worker-thread-visible (`:546-632`), and reports SETUP / WARM-UP (K derived) / STEADY (mean, min, MAX over 256 frames) for six scenes: S0 executor floor for 0/1/2/4/8/16 systems, S0b with a Fixed schedule, S2 spawn/despawn churn + `par_iter`, S3 event + `Changed` loop, S1a/S1b the Jolt pyramid through the real physics schedule (`:640-1234`). **Its findings are PENDING: the brief forbids running cargo beyond `check`, so no numbers exist in this report.** What the code itself predicts structurally: S0's steady MAX cannot be below the `Box<ScopeShared>` + chunk pair per `Schedule::run` (§3.3), and S2 adds one scope per `par_iter` call plus `CommandQueue` growth on the first churn frames only.

---

## 5. What "no allocator but ours" would take — routes, costs, and what the library must become

Read at face value, ruling 3 covers every `Vec`/`Box`/`String`/`Arc`/`HashMap`/`format!`/`collect()` in engine crates, fields AND locals, plus the threadpool's `std::alloc` bump chunks and crossbeam's deque buffers. Three mechanisms can deliver it; each is priced against the inventory.

**R1 — the engine allocator becomes the process `#[global_allocator]`.** Every std container then allocates through the library with zero source edits [D: std `alloc` docs, "route all default allocation requests to a custom object … used for example by `Box<T>` and `Vec<T>`"]. Cost: the library must become a general-purpose, thread-safe, lock-free (Principle 4) malloc — variable size, arbitrary free/reuse, any `Layout` — i.e. the `MemFreeBlockMaster` class X.J deleted (§2), plus concurrency it never had. Constraints in-tree: one global allocator per crate graph, and 22 test binaries + 30 benches + the `profiling-alloc` shim each install their own (`alloc_shim.rs:139-144`) — the gate suite would have to move to a hook inside the engine allocator instead; the `VmReservation` fallback arm calls `alloc_zeroed` from the global allocator (`vm.rs:39-40, 168-179`), so under Miri/wasm the engine allocator must be `System`-backed or it recurses. It also does not change any DATA SHAPE: a `Vec` still reallocates and moves, so it satisfies the letter of ruling 3 while leaving the SP4 class (relocation) untouched.

**R2 — nightly `allocator_api`: `Vec<T, EngineAlloc>` / `Box<T, EngineAlloc>`.** Keeps std container ergonomics, routes bytes to the library, per-field opt-in. Status [D]: the `Allocator` trait page carries "🔬 This is a nightly-only experimental API. (`allocator_api` #32838)" (https://doc.rust-lang.org/nightly/std/alloc/trait.Allocator.html); nightly is owner-permitted. Cost: same allocator surface as R1 (variable-size, free/reuse) minus thread-safety if the allocator is per-owner; every one of the ~88 kernel fields and ~230 workspace fields changes type; clippy `disallowed_types` cannot tell `Vec<T>` from `Vec<T, A>` (both are the path `alloc::vec::Vec`), so enforcement needs a newtype (`EVec<T>`) and a ban on the raw path, or the census route in R4. Relocation semantics unchanged (a `Vec<T, A>` still moves on grow).

**R3 — extend the library's own container family and migrate shape by shape (the direction the tree is already on: X.G → F1/F3 → LogRing → ScratchColumn).** What the inventory says the library must gain, each with the site that proves the need:
1. **Page-granular commit** (the packing plan, S0-S2): without it every small structure costs 64-384 KiB and the F4 reasoning (`dense_store.rs:36-45`) keeps `free`/`e2s`/`live` on the heap forever. Prerequisite for everything below.
2. **A typed column for droppable `T`** (`Vec<SystemBox>`, `Vec<BoolSystem>`, `Vec<EntityObserverList>`, `Vec<ComponentPool>`): `VmColumn` is `Copy`-only (`vm_column.rs:24-30`); `ComponentPool` already carries `drop_fn` (`component_pool.rs:235-237`) — a typed wrapper over an untracked pool is the cheapest existing path but inherits the pool floor and the granule-divisible-stride constraint of `pool_byte_layout` (no — `ComponentPool` accepts any stride; only `VmColumn` pins divisibility).
3. **Non-dividing element sizes** (12 B `Vec<(u64, Entity)>` `serialize/mod.rs:288`, 24 B `Vec<(Entity, usize)>` `traverse_iter.rs:284`, 16 B `Vec<(u32, u64)>` `bindless.rs:74`): `VmColumn::new` rejects them (`:144-149`); the fix is either the pool's byte-layout route or padding to a divisor.
4. **A shared cold reservation for bookkeeping** (free lists, `id_to_slot`, `LiveBitmap.words`, `EntitySlotMap.slots`, observer arenas): the plan's D5 rejection of shared reservations rests on the stagger and on false sharing between parallel writers (`POOL-SUBGRANULAR-PACKING-PLAN.md:196-207`) — properties of hot SoA columns, not of `&mut`-only bookkeeping. Whether one reservation may host many cold sub-columns is an architect question the plan leaves open (it scopes itself to columns).
5. **A bump/reset byte arena with variable-size records** (`CommandQueue.bytes` `command_queue.rs:83`, `ErasedKindBuffer.data` `erased_buffer.rs:109`, threadpool `ScopeBlock` chunks `block.rs:58`): `LogRing` (`ring.rs:281-335`) shows a wrapping byte arena on `VmColumn<u8>` works; the command queue needs `reserve`+`set_len` semantics over `MaybeUninit<u8>` and a per-apply reset, which is `clear` (`vm_column.rs:442-444`).
6. **Ragged / nested storage** (`Vec<Vec<BoolSystem>>` `schedule.rs:161`, `Vec<Vec<TriggerEntry>>` `trigger.rs:176`, `Vec<Vec<Entity>>` `boyko_ui/resources.rs:216-226`, `Children(Vec<Entity>)`): a CSR (offsets + flat) column pair; the physics side already built CSRs on `ScratchColumn` (audit `:130`).
7. **Type-erased object storage for `Box<dyn System>` / `Box<dyn FnOnce>` / `Box<TermList>` / `Box<ScopeShared>`** (`system_box.rs:83`, `schedule_builder.rs:92`, `term_list.rs:164`, `[merge] thread_pool.rs:278`): a `Layout`-keyed variable-size arena — exactly what `Arena + MemFreeBlockMaster` was before X.J. It was deleted as client-less (§2), so re-adding it is a resurrection with clients, not a reversal of a measured verdict.
8. **The threadpool's chunks and crossbeam's deque buffers**: Principle 0 lists "lock-free threadpool internals" as a legitimate exception; ruling 3 does not. If it is in scope, `ScopeBlock` must take its chunks from the library (its own contract already requires 64-aligned power-of-two chunks ≥ 4096 B, `block.rs:60-70`), and crossbeam-deque must be replaced by an in-house deque or fed via R1 — the only way to reach a third-party crate's allocations.
9. **Strings / `format!`** (`boyko_app` 41, `boyko_log` 9, `boyko_ui/text/report.rs`): either a fixed-capacity byte column (the `LogRing` pattern) or R1 for the cold paths; `boyko_shaderdsl`'s 163 `format!` are an offline generator.

**R4 — enforcement, whichever route.** (a) Static: generalise `physics_vec_side_store_census.rs` from `SCANNED_ROOT = "crates/boyko_physics/src"` (`:218`) to the workspace with an exact per-struct roster — it is the only in-tree mechanism that can skip test code, which clippy cannot (`clippy.toml:47-64`). Its stated holes (aliases, `cfg(not(test))`, locals) stay. (b) Dynamic: `alloc_frame_census.rs` measures the frame total process-wide; a target of "zero acquisitions in steady state" is the runtime form of ruling 3 and is already written, pending a run.

**Invariants any route must preserve (from the tree):** LIFO free-list order (`entity_master.rs:378`, `dense_store.rs:124-126`) is determinism data; write-once bases and no relocation (§1.4); Miri under Tree Borrows — every new `unsafe impl Send/Sync` needs the `LogRing`-style clause list (`ring.rs:156-190`), and the fallback arm's eager `alloc_zeroed` makes each VM-backed structure ≥ 64 KiB under Miri; loom covers the threadpool only (`[merge] Cargo.toml:26-33`).

---

## 6. Corrections to claims found in the tree or the brief (each verified above)

1. "192 KiB per column" (`constants.rs:99-103`, `component_pool.rs:464-471`, `scratch_column.rs:14-19`, `MEMORY-SYSTEM-AUDIT.md:59`) — the plan shows 384 KiB for 63 of 64 ids (`POOL-SUBGRANULAR-PACKING-PLAN.md:15-21`); nothing in the tree measures committed bytes (zero tests, one bench, `:20-21`).
2. CLAUDE.md "`boyko_threadpool` (Chase-Lev work-stealing)" reads as in-house; the deque is `crossbeam-deque 0.8.8` on both trees (§3.3). Under ruling 3 that is a foreign allocator on the dispatch path.
3. `MEMORY-SYSTEM-AUDIT.md` (2026-07) still lists `Archetype.entity_ids` and `DenseStore.s2e` as `Vec` (`:38, 67`); both moved to `VmColumn` in `3b3c86f6`. Its line references predate several edits.
4. The physics census's own header corrects the brief that ordered it: "~38 … SoftBody (~26)" → measured 34 and 30 (`tests/physics_vec_side_store_census.rs:242-247`); my regex over `src` gives 38 because it counts 4 fields in `solver/colored_tests.rs`.
5. `ScratchColumn`'s claim "beats std::Vec" (`0a803cfc`) is a commit-message measurement not re-run here.

## 7. Open questions this research leaves to the architect

1. Is the threadpool (bump chunks + crossbeam) inside ruling 3's scope, or does Principle 0's exception stand? The answer decides whether R1 is mandatory (only R1 reaches crossbeam).
2. Does the packing plan land first? Every small-structure migration in R3 is uneconomic at a 64 KiB floor (the F4 argument), and the plan is DESIGN with no code.
3. May cold bookkeeping share one reservation (R3-4), given D5's objections are hot-column properties?
4. Which of the ~30 `Box<dyn …>` / build-time `HashMap` sites are in scope — the ruling says "no reason to use any allocator but ours", which reads as including setup-time, where R1 is the only zero-edit route.

Sources (external, recorded):
- [D] https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc — reserve rounds to allocation granularity; commit rounds to page boundary; committed memory zero-initialised.
- [D] https://doc.rust-lang.org/nightly/std/alloc/trait.Allocator.html — `allocator_api` nightly-only, tracking issue #32838.
- [D] https://doc.rust-lang.org/std/alloc/index.html — `#[global_allocator]` routes `Box`/`Vec`/`String`; once per crate graph.
- [S] https://raw.githubusercontent.com/rust-lang/rust-clippy/master/clippy_lints/src/disallowed_types.rs — lint checks `use` items, `check_ty`, trait refs; configured via `disallowed-types`.
- [D] https://github.com/rust-lang/rust/issues/32838 — labelled B-unstable; page defers to wg-allocators (content fetched was stale by its own statement).

---

# LENS D - gp-allocator

# Research: Lens D — general-purpose allocation on top of VM reservations

Provenance tags: **[S]** source read this session · **[D]** official doc / paper page read · **[B]** blog / talk / secondary (recorded, not relied on) · **[T:tree file:line]** in-tree, with the worktree named. Anything I could not read is marked *not retrieved* rather than paraphrased from memory.

## Brief summary (TL;DR)

- The memory library today is exactly two primitives — `VmReservation` (reserve / monotonic frontier commit / release, zero-fill contract, **no decommit, reset or discard**) and the typed growables built on it (`ComponentPool`, `VmColumn<T: Copy>`, `InlandStore`, `ScratchColumn`). It has no `Allocator` impl, no free-list, no size classes, no per-thread state [T:ecsnative `crates/boyko_ecs/src/ecs/memory/vm.rs:85-97,109,199`; `vm_column.rs:24-30,80`; grep: zero `impl Allocator` / `allocator_api` in `D:/wt/ecsnative/crates`].
- The threadpool **already contains a bump arena** — `ScopeBlock`, one per `Scope`, 4096-byte doubling chunks at 64-byte alignment, chunk table kept in the owner's frame, all chunks freed at the join, chunks taken from `std::alloc::alloc` [T:merge `crates/boyko_threadpool/src/block.rs:103-116,147-171,244-290,328-350,369-398,426-500`]. Its Tree-Borrows design rules (D1: no bookkeeping word inside a chunk; D2: allocate-and-initialise, no reference ever formed) are the constraints any VM-backed scope arena inherits.
- Every production allocator studied (mimalloc, jemalloc, snmalloc, rpmalloc) is *per-thread-heap + size classes + owner-deferred cross-thread free*; their resident unit is a 64 KiB page/slab per (thread, size class) [S mimalloc `types.h`; D jemalloc man; D snmalloc abstract; D rpmalloc README]. Berger/Zorn/McKinley (OOPSLA 2002) measured that custom allocators beat a good general one in only 2 of 8 programs — the two were **regions** (up to 44 % faster, up to 230 % more memory) [D].
- Game engines and physics libraries converge on three shapes, not a general allocator: a frame/linear arena reset by mark (Unreal `FMemStack`/`FMemMark`, Unity `Allocator.Temp` per thread per frame/job, Unity/Entities rewindable allocator rewound every 2 frames, DICE scope stacks with finalizer chains), a step-time LIFO stack allocated **on the calling thread before fan-out** (Box2D v3: one `world->stack`, workers allocate nothing, frees in reverse at step end; Jolt `TempAllocatorImpl`), and small fixed-block pools for records (flecs `ecs_block_allocator_t`: 16-byte-aligned chunks, `max(4096/chunk,1)` per block, single free list, not thread-safe) [D/S each].
- Determinism: Box2D is deterministic across thread counts *because* the arena is single-owner and every step allocation precedes the parallel stages [S `solver.c`]; per-thread heaps hand out addresses in a schedule-dependent order by construction. In-tree the `{1,N}`-worker bit-identity of the colored solve rests on address-stable `ScratchColumn` pools and LIFO free lists whose reuse order is dispatcher-defined [T:ecsnative `crates/boyko_physics/src/plugin.rs:386`; `component/scratch/scratch_column.rs:1-19`; `entity/entity_master.rs:16,125,378`].
- Residency floor of anything on `VmReservation` is one 64 KiB granule per non-empty reservation (`COMMIT_GRANULE`), 192 KiB per tracked pool; a frame arena's floor is its **high-water mark forever** unless a reset primitive (`MEM_RESET` / `DiscardVirtualMemory` / `MADV_FREE`) is added — none exists in `vm.rs` today [T:ecsnative `constants.rs:7,99-103`; `docs/MEMORY-SYSTEM-AUDIT.md:59`; D MS Learn; D man7].

## In-tree baseline (read-only; every line verified this session)

| Item | Fact | Where |
|---|---|---|
| `VmReservation` | `reserve(len)` rounds to `COMMIT_GRANULE`, `MEM_RESERVE|PAGE_NOACCESS` / `mmap PROT_NONE`; `commit(old,new)` is `#[cold]`, granule-aligned, monotonic-frontier (debug-asserted), idempotent re-commit; `Drop` releases whole; fallback arm is eager `alloc_zeroed`; `!Send/!Sync` | [T:ecsnative `ecs/memory/vm.rs:85-97, 109-180, 194-260, 263-298`] |
| Zero-fill contract | "Freshly committed memory reads zero on first access, on every arm" — consumers *read never-written memory by design* (`InlandStore` I-Z, pool J-XI tick contract) | [T:ecsnative `vm.rs:19-37`] |
| No decommit/reset | `vm.rs` exposes `reserve`, `base`, `os_len`, `commit`, `Drop` only | [T:ecsnative `vm.rs:109-261`] |
| Granule / slabs | `COMMIT_GRANULE = 64 KiB`; `POOL_MIN_SLAB = 64 KiB`; `POOL_MAX_SLAB = 64 MiB`; commit step = byte doubling clamped, request-dominant | [T:ecsnative `ecs/constants.rs:7, 99-110`; `memory/component_pool.rs:587-591, 601-675`] |
| Pool layout | `[stagger-pad | data | added | changed]` on one reservation; `commit_subregion` floors/ceils to granule | [T:ecsnative `component_pool.rs:560-574`; audit `docs/MEMORY-SYSTEM-AUDIT.md:42-43`] |
| Resident floor | 3 granules = 192 KiB per non-empty tracked pool; ZST pool 128 KiB; `InlandStore` first slab 256 KiB; "100 sparse archetypes × 3 components ≈ 56 MiB resident floor" | [T:ecsnative `docs/MEMORY-SYSTEM-AUDIT.md:59`; `constants.rs:381-385`] |
| `VmColumn<T>` | address-stable growable on one reservation; `T: Copy` only ("deliberately NOT a general `Vec` replacement for droppable `T`"); `size_of::<T>()` must divide the granule | [T:ecsnative `memory/vm_column.rs:1-45, 80, 139, 173, 464`] |
| `ScratchColumn` | `clear` = `len = 0`, no free, pages stay resident; untracked mode saves 8 B/row + 192 KiB floor | [T:ecsnative `core/component/scratch/scratch_column.rs:1-19`] |
| `DenseStore` mix | `column: ComponentPool`, `s2e: VmColumn<EntityId>` (F3 done), `e2s: EntitySlotMap` (flat `Vec<u32>`), `live: LiveBitmap` (`Vec<u64>`), `free: Vec<u32>` LIFO | [T:ecsnative `core/component/dense/dense_store.rs:83-145, 164-179`] |
| Entity id LIFO | `free_entity_ids: Vec<EntityId>`, dispatcher-only (EM2), `pop` at allocate, `push` at deallocate, `sort_unstable_by` reverse for pop | [T:ecsnative `core/entity/entity_master.rs:16, 70-73, 125, 378, 499`] |
| Kernel Vec/Box/String fields | **78 non-test fields in 40 files** of `boyko_ecs/src` (ripgrep `^\s+(pub…)?\w+:\s*(Vec<|Box<|String|HashMap<|VecDeque<|BTreeMap<)`, `!**/tests/**`) | [T:ecsnative grep count] |
| Shapes seen | `CommandQueue.bytes: Vec<MaybeUninit<u8>>` + `panic_recovery` (byte arena) [`commands/command_queue.rs:83-85`]; observers `entries: Vec<EntityObserverEntry>`, `arena: Vec<EntityObserverList>`, `free_list: Vec<u32>`, `ever_custom: Vec<bool>` [`component/observers/entity_store.rs:102,123,125,136`]; schedule `pred_count: Box<[u16]>`, `successors: Box<[Box<[SystemIndex]>]>`, `conflict_bits: Box<[FixedBitSet]>`, `systems: Vec<SystemBox>`, `system_conditions: Vec<Vec<BoolSystem>>`, `system: Box<dyn System>`, builder `Vec<SystemDescriptor>`, `Vec<String>` names | [T:merge `core/schedule/conflict_graph.rs:68-77`; `schedule.rs:122-187`; `system_box.rs:83`; `schedule_builder.rs:106-144, 906-907, 1090-1098`; `executor_scratch.rs:219-282`] |
| `ScopeBlock` | `CHUNK0 = 4096`, `CHUNK_ALIGN = 64`, `MAX_CHUNKS = 32`; hot words `cur`/`end` at offsets 0/8; `emplace` = bump-or-grow-then-write; `grow` = `alloc(Layout)` doubling `max(i, min_e)`; `free_all` after join; "No `impl Drop`"; `!Sync` via `Cell` | [T:merge `boyko_threadpool/src/block.rs:103-116, 147-171, 190-191, 244-290, 328-350, 369-398, 426-500`] |
| Miri × custom global allocator | "Miri interprets a custom global allocator instead of replacing it"; delegating to std `System` on Windows reds Tree Borrows inside `HeapFree` for over-aligned blocks — measured, so the recording allocator is `cfg(not(miri))` | [T:merge `block.rs:759-782`] |
| Workers | `MAX_WORKERS = 64`, `clamp(1, MAX_WORKERS)`; one merged `thread_local!` lane slot; every `thread_local!` read on rustc ≥ 1.98 windows-gnu = 2 lock-prefixed RMWs on a process-global line + `FlsSetValue`, 192 call sites | [T:merge `thread_pool.rs:52, 666`; `tls.rs:163-217`; `docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:1, 45-56, 82-91`] |
| Global-allocator shims in tree | Only counting `#[global_allocator]`s in tests/benches/profiling; "A crate that declares a `#[global_allocator]` forces every binary linking it to use that one" | [T:ecsnative `boyko_app/src/profiling/alloc_shim.rs:99, 139, 145`; `boyko_input/tests/zero_alloc.rs:51,78`; 9 bench files in `bench_bevy_vs_boyko/benches`] |
| Determinism oracles | colored soft step "run-to-run bit-deterministic and `{1, N}`-worker bit-identical"; SIMD kernels `f32::to_bits()`-identical to scalar | [T:ecsnative `boyko_physics/src/plugin.rs:386`; `solver/simd.rs:9`] |

## Approaches in state-of-the-art allocators

### mimalloc (Microsoft)
- **Approach**: per-thread heaps; segments → pages → blocks; every page holds one size class; free-list *sharding* per page, *multi-sharding* into three lists. [S `include/mimalloc/types.h` (v2 layout)] [D README]
- **Constants**: `MI_SMALL_PAGE_SIZE` 64 KiB; `MI_MEDIUM_PAGE_SIZE` 512 KiB; `MI_SEGMENT_SIZE` 32 MiB (64-bit); `MI_SMALL_OBJ_SIZE_MAX` = page/8 = 8 KiB; `MI_MEDIUM_OBJ_SIZE_MAX` 64 KiB; `MI_LARGE_OBJ_SIZE_MAX` = segment/2 = 16 MiB; `MI_BIN_HUGE = 73`, `MI_BIN_FULL = 74` bins; `MI_MAX_ALIGN_SIZE = 16`; fast path via a direct page table `MI_PAGES_DIRECT` for small sizes. [S types.h]
- **Free lists**: "`free` for blocks that can be allocated, `local_free` for freed blocks not yet available to `mi_malloc`, `thread_free` for freed blocks by other threads"; delayed-free states `MI_USE_DELAYED_FREE / MI_DELAYED_FREEING / MI_NO_DELAYED_FREE / MI_NEVER_DELAYED_FREE` (abandoned pages). [S types.h] Cross-thread free "can now be a single CAS". [D README]
- **Numbers**: ~0.2 % metadata overhead; secure mode ≈10 % slower; abstract: 7 % faster than tcmalloc and 14 % faster than jemalloc on Redis, "consistently out performs over a wide range of sequential and concurrent benchmarks"; purge delay default 1000 ms (v3), `MIMALLOC_PURGE_DECOMMITS` selects decommit vs reset. [D README; D APLAS abstract] Paper PDF not renderable here — per-alloc instruction counts *not retrieved*.
- **Heaps**: first-class heaps, `mi_heap_destroy` frees all at once (region semantics on top of a general allocator); v3 allows allocating into a heap from any thread. [D README]

### jemalloc
- **Approach**: multiple arenas "to reduce lock contention"; thread-specific cache (tcache) up to `opt.tcache_max` default 32 KiB (range up to 8 MiB); small objects in slabs (bitmap per slab) inside page-aligned extents, one extent per large object. [D FreeBSD man page]
- **Size classes**: small 8 B … 14 KiB with four classes per doubling; large from 16 KiB upward with spacing 2 KiB, 4 KiB, … [D man page] (The 2006 BSDCan paper PDF could not be rendered; its per-CPU scaling figures are *not retrieved*.)
- **Residency policy**: dirty → muzzy (`MADV_FREE`) → clean via decay curves; default decay 10 s. [D man page]

### snmalloc (Microsoft Research, ISMM 2019)
- **Approach**: allocator per thread; remote frees are *messages* returned to the owning allocator in batches, "1000s of remote deallocations … with only a single atomic operation"; objects collected in radix trees, sent in ~1 MB batches, multi-hop (up to 7 hops). Distinguishes large (≥16 MB), medium (≥64 KB) and small; small slabs of 64 KiB; "bump pointer-free list" with 64 bits of metadata per 64 KiB slab; two branches on the malloc fast path (Linux/Clang). [D README; D ISMM abstract via conf page; B alastairreid summary] ACM full text 403 — benchmark tables *not retrieved*.

### rpmalloc
- **Approach**: 16-byte natural alignment; four page kinds — small ≤ 4 KiB blocks in 64 KiB pages, medium-small ≤ 32 KiB in 1 MiB, medium-large ≤ 256 KiB in 4 MiB, large ≤ 2 MiB in 16 MiB; 256 MiB span alignment so a pointer finds its page by masking; smallest classes 16-byte granularity, larger classes variable interval "to limit overhead to a fixed ratio"; each page owned by the allocating thread, cross-thread frees deferred via a per-page atomic free list; first-class heaps (`rpmalloc_heap_acquire/release`) with bulk release; decommit of free pages on Linux ≥ 5.18, `disable_decommit` option; ~3300 lines of C. [D README]

### Berger, Zorn, McKinley — "Reconsidering Custom Memory Allocation" (OOPSLA 2002)
- Taxonomy: per-class, custom-pattern, region. On 8 custom-allocating programs, "for six of these applications, a state-of-the-art general-purpose allocator (the Lea allocator) performs as well as or better"; the two winners are region-based: **up to 44 % faster, up to 230 % more memory**; "reaps" (regions + individual free) proposed. [D paper via fetch summary]
- Johnstone & Wilson 1998 ("The memory fragmentation problem: solved?"): 8 real C/C++ programs, several conventional allocators show near-zero fragmentation once header/alignment overheads are accounted for. [D — abstract-level only; ACM page 403, exact per-policy percentages *not retrieved*]

## What game engines / physics libraries do instead

### Unreal Engine — `FMemStackBase` / `FMemMark`
- "Simple linear-allocation memory stack. Items are allocated via PushBytes() or the specialized operator new()s. Items are freed en masse by using FMemMark to Pop() them"; `FMemMark` saves the stack's position and pops everything added after it; `FMemStack : TThreadSingleton, FMemStackBase` (per-thread). [D — search-result rendering of the 4.26 API page; the page bodies themselves returned 403/empty this session, so chunk size and `FPageAllocator` page size are *not retrieved*]
- Nicholas Frechette records that "many AAA games have shipped with this making extensive use of it". [B]

### DICE / Frostbite — Scope Stack Allocation (Fredriksson, 2010)
- Scope stack sits on a linear allocator; a scope records a rewind point and a finalizer chain; destructors run in reverse dependency order on scope exit, then the allocator rewinds. Motivation: consoles as fixed-memory embedded systems, fragmentation, heap cost. [B slides (PDF not rendered); S gist reimplementation: `LinearAllocator{m_ptr,m_end}`, `newObject()` prepends a `Finalizer` header, `~ScopeStack` walks the chain then rewinds]

### Naughty Dog — GDC 2015 "Parallelizing the Naughty Dog Engine Using Fibers"
- The talk covers "the memory allocation patterns used in the title" (tagged heap). A search snippet states the tagged heap uses 2 MB internal buffers; **unverified** — the PDF could not be rendered and the archive page carries only the abstract. [B]

### Unity Collections / Entities
- `Allocator.Temp`: "Each frame, the main thread creates a Temp allocator which it deallocates in its entirety at the end of the frame"; "Each job also creates one Temp allocator per thread, and deallocates them in their entirety at the end of the job"; "only safe to use in the thread and the scope where they were allocated"; fastest. `TempJob`: must deallocate within 4 frames, safety checks throw past that; 16-byte min alignment. `Persistent`: "slowest allocator for indefinite lifetime". [D collections 2.5 allocator-overview]
- Rewindable allocator: "similar way to a linear allocator. It's fast and thread safe"; initial block, **doubles until a maximum block size, then grows linearly**; minimum alignment **64 bytes**; free is a no-op unless `EnableBlockFree` (then a block is rewound when its last allocation is freed); on rewind "keeps the memory blocks that it used before … and disposes the rest". [D allocator-rewindable]
- Entities `World.UpdateAllocator`: "double rewindable allocator", frees "Every 2 frames", "fast and thread safe", allocations can be passed to jobs. [D entities 1.3 allocators-overview]

### flecs
- Block allocator: `chunk_size = ECS_ALIGN(size, 16)`, `chunks_per_block = ECS_MAX(4096 / chunk_size, 1)`, block = `ecs_os_malloc(sizeof(block) + block_size)`, free list threaded through chunks (`chunk->next`), LIFO push on free; bypasses to `ecs_os_malloc` when `chunks_per_block <= FLECS_MIN_CHUNKS_PER_BLOCK (1)`; `FLECS_USE_OS_ALLOC` disables it; **no synchronisation — not thread-safe**; `FLECS_SANITIZE` tracks outstanding allocations. [S `src/datastructures/block_allocator.c`]
- Stack allocator: `FLECS_STACK_PAGE_SIZE (1024 - FLECS_STACK_PAGE_OFFSET)`; pages `{data,next,sp:int16,id}` chained, first page allocated on first use; cursor `{prev,page,sp,is_free}` with `flecs_stack_get_cursor`/`restore_cursor` (nested, asserted pairing); oversize requests bypass to the OS allocator and are the only thing `flecs_stack_free` handles; `flecs_stack_reset` asserts no leaks. [S `include/flecs/datastructures/stack_allocator.h`, `src/datastructures/stack_allocator.c`]

### Box2D v3
- One stack/arena per world (`&world->stack`, `b2StackAlloc`); allocations 32-byte aligned "to support 256-bit SIMD"; overflow "fall back to the heap (undesirable)"; free asserts strict LIFO (`B2_ASSERT( mem == entry->data )`); grows only when empty, to `maxAllocation + maxAllocation/2`. **All step allocations happen on the calling thread before `b2ExecuteMainStage`/`enqueueTask`; `b2ExecuteBlock`/`b2SolverTask` allocate nothing; frees are issued in reverse at step end** (`graphBlocks, jointBlocks, contactBlocks, bodyBlocks, stages, overflowContacts, wideContactConstraints`). [S `src/arena_allocator.c` (summariser named the type `b2ArenaAllocator`/`b2Stack` — naming across versions differs), `src/solver.c`; `world.c` 404]
- FAQ: "cross-platform determinism as of version 3.1"; "deterministic under multithreading. A simulation using two threads will give the same result as eight threads"; "For the same input, and same binary". [D]

### Jolt Physics
- `TempAllocatorImpl(size_t inSize)`: "allocates a large block through malloc upfront"; `Allocate(uint)` / `Free(void*, uint)`; results must be `JPH_RVECTOR_ALIGNMENT`-aligned; `IsEmpty/GetUsage/CanAllocate/OwnsMemory`. [D class page] The index-page fetch reported sentences "No allocations will be made by Jolt during the simulation step" and "Use a TempAllocator…", but the `Architecture.md` fetch found no such text — treat those two quotes as **unverified**.
- Determinism: "deterministic provided that: The APIs that modify the simulation are called in exactly the same order" and "The same binary code is used"; cross-platform mode via CMake option (MSVC/clang/gcc/emscripten, x86/ARM/RISC-V/PowerPC/LoongArch, 32/64-bit); broadphase queries and listener callback order are NOT deterministic. Thread-count independence is not stated in `Architecture.md`. [D]

### Bevy / EnTT (for the comparative row)
- Bevy `BlobArray` uses `alloc::alloc::{alloc, realloc, dealloc, handle_alloc_error}` directly; no `Allocator` parameter anywhere. [S `bevy_ecs/src/storage/blob_array.rs`]
- EnTT: the wiki page fetched contained no allocator material — **no reliable information retrieved** this session.

## Rust mechanics that bound the design space

- `core::alloc::Allocator` (nightly, `allocator_api`, tracking #32838): `fn allocate(&self, Layout) -> Result<NonNull<[u8]>, AllocError>`, `unsafe fn deallocate(&self, NonNull<u8>, Layout)`; provided `allocate_zeroed`, `grow`, `grow_zeroed`, `shrink`; `&self` ⇒ interior mutability; `&A`, `&mut A`, `Box<A>`, `Rc/Arc<A>` are equivalent allocators; ZST allocations allowed; a block is invalidated by `deallocate`, successful `grow/shrink`, or the allocator's drop / `&mut` mutation. [D]
- `Vec<T, A>` / `Box<T, A>` `new_in` / `with_capacity_in` exist on nightly. **`String` has no allocator parameter** (`pub struct String { /* private */ }`, `from_utf8(Vec<u8>)`); std `HashMap` has none; `hashbrown::HashMap<K, V, S = DefaultHashBuilder, A: Allocator = Global>` with `new_in`/`with_capacity_in`, trait from `allocator-api2`. [D]
- Stabilisation status (post dated 2026-09-09): PR #156882 MVP = the two-method `Allocator` + `Vec<T, A>` + `Box<T, A>` (`new_in`, `with_capacity_in`, `allocator()`), dyn-compatible; **excluded**: `String`, `HashMap`, `Box::pin_in`, `Clone` (pending `AllocatorClone`); "months off". [B cetra3]
- `bumpalo`: bump within chunk, new chunk from the global allocator when full, `reset()` keeps a chunk, "Drop implementations are not invoked" (opt-in via `bumpalo::boxed::Box`), `Vec<T, &Bump>` via `allocator_api` / `allocator-api2`, `bumpalo::collections::{Vec,String}`. [D README]
- `#[global_allocator]`: used by `Box`, `Vec`, `String` process-wide; exactly one in the crate graph; `GlobalAlloc` must not unwind, must not re-enter std allocation, `&self` is shared across threads. [D `std::alloc`]
- Whether `Vec<T, A>` for `A ≠ Global` implements `FromIterator` (i.e. whether `collect()` can target a custom allocator) was **not verified** this session.

## OS facts for residency and reset

- Windows: `MEM_RESERVE` rounds to the allocation granularity (64 K), `MEM_COMMIT` to the page; committed pages are guaranteed zero on first access, "Actual physical pages are not allocated unless/until the virtual addresses are actually accessed"; re-committing a committed page does not fail; `MEM_RESET` — "should not be decommitted … does not guarantee that the range … will contain zeros", cannot be combined with other flags; `MEM_RESET_UNDO` (Win 8+); `MEM_LARGE_PAGES` requires reserve+commit in one call and large-page-minimum multiples. [D VirtualAlloc]
- `DiscardVirtualMemory(page-aligned addr, size)`: "without decommitting the memory … contents … undefined"; may give RAM back; fails unless `PAGE_READWRITE`; Windows 8.1 Update+. [D]
- Linux: `MADV_DONTNEED` → zero-fill-on-demand on re-access (anonymous); `MADV_FREE` (4.5) → freeing "could be delayed until memory pressure", zero-fill after reclaim; `MADV_POPULATE_WRITE` (5.14) prefaults writable. [D man7]
- Costs (Bruce Dawson, Windows, 2014): ~175 µs/MB soft page faults on first touch; ~150 µs/MB kernel zeroing of freed touched pages; ~75 µs/MB to decommit dirtied memory vs 2.5 µs/MB untouched; ~7.5 µs per `VirtualAlloc`/free pair (8–32 MB, untouched); "allocating an 8 MB buffer every frame … can easily waste 3.2 ms". [B]

## Comparative table

| Aspect | mimalloc | jemalloc | snmalloc | rpmalloc | flecs | Box2D v3 | Unity Entities | In-tree today |
|---|---|---|---|---|---|---|---|---|
| Unit of residency | 64 KiB page / size class / heap | page-aligned extents, tcache ≤ 32 KiB | 64 KiB slab | 64 KiB–16 MiB pages, 256 MiB spans | 4096-byte blocks | one arena per world | rewindable blocks (double→linear) | 64 KiB granule; 192 KiB per pool |
| Thread model | per-thread heap, CAS `thread_free` | arenas + tcache | allocator per thread, batched messages | per-thread, per-page atomic free list | none (not thread-safe) | single-owner, alloc before fan-out | "thread safe" rewindable | single-writer pools; `ScopeBlock` `!Sync` |
| Free semantics | individual + heap destroy | individual | individual | individual + heap release | individual (free list) / cursor restore | LIFO only | rewind all | `ScratchColumn::clear`; `free_all` at join |
| Cross-thread free | deferred to owner | tcache flush to arena | message batches | deferred to owner | n/a | n/a (none) | n/a | n/a |
| Address determinism vs thread count | no | no | no | no | yes (single thread) | **yes** [S] | not stated | yes on pools (`{1,N}` oracle) |
| Reset to OS | purge delay, reset/decommit | decay, `MADV_FREE` | — | decommit ≥5.18 | — | grow-when-empty only | disposes extra blocks | **none** (`vm.rs` has no decommit/reset) |

## Key algorithms and techniques (as they appear across sources)

- **Pointer → owner metadata by alignment masking**: rpmalloc 256 MiB span alignment; mimalloc 32 MiB segment alignment; jemalloc page-aligned extents. Lets `free(ptr)` find its page/size class without a header. [D/S]
- **Owner-deferred cross-thread free**: mimalloc `thread_free` CAS push + delayed-free states; rpmalloc per-page atomic list; snmalloc batched messages. Keeps the fast path single-threaded. [S/D]
- **Bump + mark/rewind**: `FMemMark`, flecs cursors (nested via `prev`), Unity rewindable, DICE scope stacks (+ finalizer chain for `Drop`), in-tree `ScopeBlock` (no `Drop`, chunk table in owner frame). [D/S/T]
- **Allocate-before-fan-out**: Box2D allocates every per-step buffer on the calling thread, workers touch none, frees reverse. This is what makes the step both allocation-free in workers and thread-count-deterministic. [S]
- **Growth-when-empty**: Box2D grows the arena only when `allocation == 0`, to 1.5× peak; Unity rewindable keeps used blocks. Both avoid mid-step relocation. [S/D]
- **Fixed-block pool (single free list per class)**: flecs block allocator; the minimal (c). [S]

## Evaluation of the four shapes (facts and trade-offs only)

### (a) Frame arena on a `VmReservation`, lazy commit, reset-to-mark
- Per-allocation cost is the `ScopeBlock::bump` shape: two loads, a pad mask, two compares, one store [T:merge `block.rs:369-398`]. No free. Growth = frontier `commit` (`#[cold]`, one syscall per slab) [T:ecsnative `vm.rs:199`].
- **Zero-fill breaks on reset**: fresh commits are zero (contract, `vm.rs:19-37`); a rewound region is *not*. Consumers that rely on zero-read (`InlandStore` I-Z, tick columns) cannot live on a reset arena without an explicit zeroing or a `DiscardVirtualMemory`/`MADV_DONTNEED` path (both cost the page-fault/zeroing ~175+150 µs/MB again per Dawson [B]).
- **Residency floor = high-water mark** since nothing in `vm.rs` decommits; Unity keeps used blocks and disposes the rest, mimalloc/jemalloc purge on a delay. First frame pays first-touch faults; later frames pay none.
- **Drop is not run** by bumpalo or `ScopeBlock`; DICE and `bumpalo::boxed::Box` add a finalizer/Drop path at a per-object cost. In-tree `VmColumn` sidesteps this with `T: Copy`.
- **Concurrency**: a single cursor is either single-writer (Box2D, dispatcher-only like EM2) or an atomic `fetch_add` on one contended line (Unity claims thread-safe; mechanism not documented). Per-thread sub-arenas restore locality at the price of W partially-filled slabs.

### (b) Per-thread scope arena for job scratch (W ≤ 64 workers)
- Exists in embryo: `ScopeBlock` is per-`Scope` (not per-thread), chunks come from `std::alloc`, freed at join; nested scopes each own a block [T:merge `block.rs`, `scope.rs:1024-1055`]. Unreal's `FMemStack` is a per-thread singleton with mark/pop [D-thin]; flecs cursors chain via `prev`; Box2D and flecs both bypass to the heap for oversize requests [S].
- Reaching per-thread state costs a `thread_local!` read; on this toolchain/target that is two lock RMWs on a process-global line + `FlsSetValue` per read [T:merge `docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:45-56`]. The worker id is already obtained by the spawn path's single read (`worker_lane_for`), so a `[Arena; MAX_WORKERS]` indexed by `wid` would add no second read only if plumbed through the same call [T:merge `tls.rs:214-217, 318-339`].
- Tree-Borrows constraints that `ScopeBlock` had to satisfy (chunk-payload-only, allocate-and-initialise, no `Deref`) apply unchanged to any VM-backed replacement [T:merge `block.rs:35-81`]. Miri: the VM fallback arm is `alloc_zeroed` [T:ecsnative `vm.rs:168-179`]; a custom `#[global_allocator]` is interpreted by Miri and measured UB when delegating to Windows `System` for over-aligned blocks [T:merge `block.rs:759-782`].
- Determinism: addresses handed out by worker *k* depend on which worker ran the task → thread-count-dependent addresses by construction, exactly as in per-thread heaps. Harmless only if no computation depends on the address (no pointer hashing/sorting, no cross-frame retention).

### (c) Persistent size-class pool on reservations
- Minimal form = flecs block allocator: one intrusive LIFO per class, blocks of `max(4096/chunk,1)` chunks, 16-byte alignment, no coalescing, not thread-safe [S]. Full form = mimalloc/snmalloc: 64 KiB page per (heap, class), ~73 bins, owner-deferred remote free, purge policy [S/D]. Berger et al. measured that this class ("per-class" custom allocators) generally does *not* beat a good general allocator [D].
- Residency: each touched size class pins at least one 64 KiB granule per owning thread/heap; with 73 bins that is ≤ 4.7 MiB per heap if every class is touched (arithmetic over [S] constants). On one shared single-writer pool the multiplier is 1.
- Records in tree that have this shape are cold and small: observer lists (`entries` Vec, linear scan) [T:ecsnative `entity_store.rs:101-102`], schedule tables (`Box<[...]>` built once) [T:merge `conflict_graph.rs:68-77`], builder-time `Vec<String>` [T:merge `schedule_builder.rs:1090`]. Command records are already a byte arena (`CommandQueue.bytes`, amortised `reserve`, retained across frames) [T:ecsnative `command_queue.rs:83-85`; audit `:77`].
- Cross-thread free needs owner-deferral or a single-writer rule; the in-tree precedent is EM2 "Dispatcher-only" [T:ecsnative `entity_master.rs:16`].

### (d) Coverage of "(a)+(b)+(c) behind one `Allocator` impl"
- What `Allocator` reaches on nightly: `Vec<T, A>`, `Box<T, A>` (incl. slices via `new_uninit_slice_in`), `hashbrown::HashMap<…, A>`, bumpalo-style forks of `String`. What it does **not** reach: `std::string::String`, `format!`, std `HashMap`, and (unverified) `collect()` into non-`Global` vectors [D].
- What the trait *cannot* express: a bitmap that must be word-addressable and zero-read (`LiveBitmap.words`), a LIFO id stack whose *reuse order* is part of a determinism contract (`free_entity_ids`, `DenseStore.free`), and `T: Copy`-only address-stable columns — all of which the tree already models as bespoke primitives (`VmColumn`, `InlandStore`). Bevy's own hot storage bypasses the trait and calls `alloc::alloc` directly [S].
- The alternative route — a `#[global_allocator]` backed by the memory library — reaches *everything* (including `String`, third-party crates, crossbeam-deque buffers) with zero call-site changes, but must be thread-safe, non-reentrant, non-unwinding, one per binary, and is interpreted under Miri [D; T:merge `block.rs:759-782`; T:ecsnative `alloc_shim.rs:139`].

### Determinism summary (which shapes are thread-count-dependent)
| Shape | Address order depends on thread count? | Reason / source |
|---|---|---|
| (a) single-cursor frame arena, dispatcher-only | no | one writer, fixed program order (Box2D pattern [S]) |
| (a) with atomic cursor shared by workers | yes | interleaving decides offsets |
| (b) per-worker scope arena | yes, per address; contents scope-bounded | same as per-thread heaps |
| (c) single-writer pool | no | LIFO reuse in program order (in-tree EM2 precedent) |
| (c) per-thread pool with owner-deferred free | yes | which thread frees decides which list the block lands in (mimalloc `thread_free` [S]) |
| In-tree `ScratchColumn`/pool columns | no | address-stable base, index-addressed; oracle `{1,N}` green [T:ecsnative `plugin.rs:386`] |

## Pitfalls recorded in the sources
- Regions leak by construction: no individual free ⇒ "up to 230 %" more memory in server-like patterns [D Berger]. Unity's `EnableBlockFree` and mimalloc/rpmalloc first-class heaps are the mitigations.
- Rewinding does not zero; every zero-read contract in the tree is stated over *fresh commit* [T:ecsnative `vm.rs:19-37`].
- Frame-sized alloc/free churn costs ~400 µs/MB in faults + zeroing + decommit [B Dawson]; rpmalloc exposes `disable_decommit` for exactly this [D].
- Per-thread state on rustc ≥ 1.98 windows-gnu pays two contended RMWs per `thread_local!` read [T:merge TLS doc].
- Tree Borrows: a bookkeeping word inside an arena chunk, or any reference formed into arena memory across a task body, reopens the protector class `ScopeBlock` was designed around [T:merge `block.rs:35-81`].
- Custom global allocators are interpreted by Miri; delegation to `System` on Windows is measured UB for over-aligned blocks [T:merge `block.rs:759-782`].

## Relevant academic / primary works
- Leijen, Zorn, de Moura — "Mimalloc: Free List Sharding in Action", APLAS 2019. [D abstract; S types.h]
- Liétar et al. — "snmalloc: A Message Passing Allocator", ISMM 2019. [D abstract via conf/summary pages]
- Evans — "A Scalable Concurrent malloc(3) Implementation for FreeBSD", BSDCan 2006 (PDF not rendered; man page used). [D]
- Berger, Zorn, McKinley — "Reconsidering Custom Memory Allocation", OOPSLA 2002. [D]
- Johnstone, Wilson — "The Memory Fragmentation Problem: Solved?", ISMM 1998 (abstract level only). [D]
- Fredriksson — "Scope Stack Allocation", DICE 2010. [B]
- Gyrling — "Parallelizing the Naughty Dog Engine Using Fibers", GDC 2015. [B]

## Applicability notes for boyko-engine (facts, not recommendations)
- Directly analogous in tree: `ScopeBlock` is (b) minus the VM backing; `ScratchColumn` is (a) for `Copy` columns with `clear = len 0`; `CommandQueue.bytes` is a retained byte arena; `free_entity_ids`/`DenseStore.free` are the "dedicated free-list stack" shape.
- What the memory library lacks for (a)/(b): a reset/discard primitive (`MEM_RESET`/`DiscardVirtualMemory`/`MADV_FREE`) and a non-monotonic commit frontier (`commit` debug-asserts monotone, `vm.rs:200-209`); for (c): any free-list or size-class machinery; for all three: a `Send/Sync` story (`VmReservation` is `!Send/!Sync`, owners opt in — `vm.rs:82-84`).
- Constraint from the SDK principle: the ban target as stated by the owner is *every foreign allocator*, which `Allocator`-parameterised containers cannot reach for `String`/`format!`/std `HashMap` on any Rust today [D]; only a `#[global_allocator]` route or bespoke types cover those.

## Open questions for the architect
1. Is the ban enforced at the *type* level (gate on `Vec`/`Box`/`String` paths, requiring `Vec<T, A>`-style containers on nightly) or at the *allocator* level (`#[global_allocator]` = memory library, which also captures third-party crates and Miri interpretation)?
2. Does the zero-fill contract extend to rewound arena memory, or do zero-read consumers stay on fresh-commit-only storage?
3. Which shapes may be allocated *by workers* at all? Box2D's answer is none; Unity's is per-thread `Temp`; the in-tree `{1,N}` oracle currently holds with none.
4. Does the memory library gain decommit/reset, and if so with which OS primitive per arm (the three differ in zeroing guarantees and cost)?
5. Are `Drop`-bearing values allowed in arenas (finalizer chain à la DICE) or is `T: Copy` the rule as in `VmColumn`?
6. Per-worker arenas: indexed by `wid` from the existing single TLS read, or a second `thread_local!` (measured cost above)?

## Sources
[1] https://github.com/microsoft/mimalloc — README: pages "usually 64KiB", 0.2 % metadata, secure-mode ≈10 %, purge options, first-class heaps [D]
[2] https://raw.githubusercontent.com/microsoft/mimalloc/master/include/mimalloc/types.h — page/segment/bin constants, three free lists, delayed-free states [S]
[3] https://conf.researchr.org/details/aplas-2019/aplas-2019-papers/8/Mimalloc-Free-List-Sharding-in-Action — abstract, 7 %/14 % Redis figures [D]
[4] https://man.freebsd.org/cgi/man.cgi?query=jemalloc&sektion=3 — arenas, tcache 32 KiB, size classes, decay 10 s [D]
[5] https://github.com/microsoft/snmalloc/blob/main/README.md — message passing, "1000s of remote deallocations … single atomic", two-branch fast path [D]
[6] https://dl.acm.org/doi/10.1145/3315573.3329980 (403) / https://alastairreid.github.io/RelatedWork/papers/lietar:ismm:2019/ — thresholds 16 MB/64 kB, 64 KiB slabs, 64-bit metadata, 1 MB batches [D/B]
[7] https://github.com/mjansson/rpmalloc/blob/develop/README.md — page kinds and sizes, 256 MiB spans, per-page atomic free list, decommit policy [D]
[8] https://people.cs.umass.edu/~emery/pubs/berger-oopsla2002.pdf — 6 of 8, 44 %, 230 %, reaps [D]
[9] https://dl.acm.org/doi/10.1145/286860.286864 — Johnstone & Wilson (abstract only) [D]
[10] https://docs.unrealengine.com/4.26/en-US/API/Runtime/Core/Misc/FMemStackBase/ and …/FMemMark — description sentences via search rendering; page bodies not retrievable [D-thin]
[11] https://nfrechette.github.io/2016/05/09/greedy_stack_frame_allocator/ — "many AAA games have shipped with" FMemStack [B]
[12] http://sofiacpp.github.io/advanced-cpp/slides/scopestacks_public.pdf (not rendered) / https://gist.github.com/jhaberstro/664938 — scope stack mechanism [B/S]
[13] https://archive.org/details/GDC2015Gyrling_201508 — talk abstract only [B]
[14] https://docs.unity3d.com/Packages/com.unity.collections@2.5/manual/allocator-overview.html — Temp/TempJob/Persistent sentences [D]
[15] https://docs.unity3d.com/Packages/com.unity.collections@2.5/manual/allocator-rewindable.html — doubling→linear blocks, 64-byte alignment, EnableBlockFree [D]
[16] https://docs.unity3d.com/Packages/com.unity.entities@1.3/manual/allocators-overview.html — World.UpdateAllocator every 2 frames, thread safe [D]
[17] https://raw.githubusercontent.com/SanderMertens/flecs/master/src/datastructures/block_allocator.c — block allocator internals [S]
[18] https://raw.githubusercontent.com/SanderMertens/flecs/master/include/flecs/datastructures/stack_allocator.h and …/src/datastructures/stack_allocator.c — stack allocator, cursors [S]
[19] https://raw.githubusercontent.com/erincatto/box2d/main/src/arena_allocator.c and …/src/solver.c — 32-byte align, LIFO assert, grow-when-empty, alloc-before-fan-out, reverse frees [S]
[20] https://box2d.org/documentation/md_faq.html — cross-platform (3.1) and cross-thread-count determinism [D]
[21] https://jrouwe.github.io/JoltPhysics/class_temp_allocator_impl.html — TempAllocatorImpl API [D]; https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Docs/Architecture.md — determinism conditions [D]
[22] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/storage/blob_array.rs — `alloc::alloc` direct, no Allocator param [S]
[23] https://doc.rust-lang.org/nightly/core/alloc/trait.Allocator.html — trait contract, #32838 [D]
[24] https://doc.rust-lang.org/nightly/alloc/string/struct.String.html — no allocator parameter [D]
[25] https://docs.rs/hashbrown/latest/hashbrown/struct.HashMap.html — `A: Allocator = Global`, `new_in`, allocator-api2 [D]
[26] https://doc.rust-lang.org/std/alloc/index.html — `#[global_allocator]`, `GlobalAlloc` requirements [D]
[27] https://github.com/fitzgen/bumpalo — reset, no Drop, `Vec<T, &Bump>` [D]
[28] https://cetra3.github.io/blog/state-of-allocators-2026-part-2/ — PR #156882 MVP scope, exclusions [B]
[29] https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc — granularity, zero guarantee, MEM_RESET/UNDO [D]
[30] https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-discardvirtualmemory — discard semantics, 8.1+ [D]
[31] https://man7.org/linux/man-pages/man2/madvise.2.html — MADV_DONTNEED / MADV_FREE / MADV_POPULATE_WRITE [D]
[32] https://randomascii.wordpress.com/2014/12/10/hidden-costs-of-memory-allocation/ — µs/MB figures [B]
In-tree (read-only): `D:/wt/ecsnative/crates/boyko_ecs/src/ecs/memory/{vm.rs,vm_column.rs,component_pool.rs,mod.rs,utils.rs}`, `…/ecs/constants.rs`, `…/core/component/dense/dense_store.rs`, `…/core/component/scratch/scratch_column.rs`, `…/core/entity/entity_master.rs`, `…/core/commands/command_queue.rs`, `…/core/component/observers/entity_store.rs`, `D:/wt/ecsnative/docs/MEMORY-SYSTEM-AUDIT.md`, `D:/wt/ecsnative/crates/boyko_physics/src/plugin.rs`, `D:/wt/ecsnative/crates/boyko_app/src/profiling/alloc_shim.rs`; `D:/wt/merge/crates/boyko_threadpool/src/{block.rs,scope.rs,tls.rs,thread_pool.rs}`, `D:/wt/merge/crates/boyko_ecs/src/ecs/core/schedule/*.rs`, `D:/wt/merge/docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md`.

Not retrieved this session (do not cite from memory): Unreal `FMemStack` chunk/page sizes; Naughty Dog tagged-heap block size; EnTT allocator support; exact Johnstone-Wilson fragmentation percentages; jemalloc 2006 paper scaling figures; mimalloc paper instruction counts; snmalloc benchmark tables; whether `collect()` targets `Vec<T, A>` on nightly.
