> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase X.F — Arena Growth: Research + Code Inventory

Two inputs for the architect: (A) external practice survey, (B) exact in-repo inventory.
Goal: replace the fixed-capacity `Arena` (64 MB, `panic!` on exhaustion) with growth that is
address-stable, zero-cost on the hot path, and measurably FASTER THAN BEVY on
growth-crossing workloads.

---

## A. External practice survey (researcher)

### TL;DR

- **Bevy grows by realloc+memcpy.** `Table::reserve` → `Vec::reserve` on the entities vector
  (std `RawVec::grow_amortized` = `max(cap*2, required)`), then `realloc_columns()` calls
  `ThinArrayPtr::realloc` → `std::alloc::realloc` per column — existing component data MOVES.
  Every growth event at N rows is an O(N) memcpy per column (plus tick arrays). Bevy can
  afford moves because nothing caches column pointers across structural changes — query
  fetches re-acquire the column pointer in `set_table` on every table visit.
- **flecs is the same shape** (per-column `ecs_os_realloc`, next-pow-2, may move).
  **EnTT and Unity DOTS are the no-move shape**: EnTT allocates fixed 1024-element pages
  (pointer-stable on add); Unity reserves a **1 GB VA range per World** for fixed 16 KiB
  chunks that never move.
- **Reserve-huge/commit-lazy is established production practice**: Unreal MallocBinned3
  reserves 1 GB VA per bin pool and commits/decommits in Blocks (verified in local UE 5.7
  source, `MallocBinned3.h:26-91`); mimalloc commits 4 MiB segments on demand inside ~1 GiB
  arenas; Our Machinery's canonical article (Frykholm, "Virtual Memory Tricks") prescribes
  exactly this design.
- **OS semantics**: `MEM_COMMIT` charges commit but allocates NO physical pages until first
  touch (demand-zero) — a commit-lazy arena pays only the soft faults Bevy ALSO pays on
  freshly realloc'd memory, and deletes the O(N) memcpy term. Re-committing an
  already-committed page is a documented no-op success (idempotent commit).

### Per-engine growth mechanics

| Aspect | Bevy (main) | flecs (v4) | EnTT | Unity DOTS 1.x | boyko target |
|---|---|---|---|---|---|
| Growth unit | whole table ×2 | column → next pow2 | +1 page (1024 elems) | +16 KiB chunk | +committed slab |
| Existing data moves? | **yes** (realloc) | **yes** (realloc) | no (paged) | no (fixed chunks) | **no** (reserve+commit) |
| Growth-event cost @ N rows | O(N) memcpy/column + faults | O(N) memcpy/column + faults | O(page) | O(chunk)+commit | syscall + faults only |
| Pointer caching across growth | impossible (re-fetch per table) | n/a | stable on add | stable | column/bundle caches stay valid |
| VM usage | global allocator | global allocator | global allocator | 1 GB VA reserve/World | multi-GB reserve, lazy commit |

### Hard OS constraints

1. **Windows**: reservation granularity 64 KiB; commit granularity 4 KiB; `MEM_COMMIT`
   charge is logical (pagefile-backed), physical pages fault in on touch; **a reservation
   cannot be grown in place** ("VirtualAlloc cannot reserve a reserved page") — the ceiling
   is fixed at creation; idempotent commit documented. User VA = 128 TB.
2. **Linux**: overcommit mode 2 **ignores `MAP_NORESERVE`** — the mode-proof pattern is
   `mmap(PROT_NONE)` reserve (not accounted: not private-writable) + `mprotect(RW)` per slab
   as the commit step (ENOMEM surfaces at mprotect). Keep the committed range a contiguous
   frontier (≤2 VMAs); scattered protections → VMA creep toward `vm.max_map_count` (65530).
   THP: 2 MiB-aligned slabs make THP help; sparse touch inside huge slabs can bloat.
3. **Miri/wasm**: Miri has NO `VirtualAlloc` shim (rust-lang/miri#4187) and no reliable
   mprotect; wasm has `memory.grow` only. The fallback arm must eagerly allocate the FULL
   logical reserve from the global allocator (X.C lesson: keep Miri on a modelable path) —
   bookkeeping identical, commit becomes a no-op; tests use small reserves.
4. **Commit-charge**: never commit the whole multi-GB reserve eagerly (kills pagefile-limited
   machines) — the precise reason reserve≠commit.

### Production reservation sizes

Unreal: 1 GB per bin pool (dozens of pools; "separate VM per pool" on Windows by default).
Unity: 1 GB per World. mimalloc: ~1 GiB arenas / 4 MiB segments. Our Machinery: 8 GB per
array "unremarkable". Single-digit-GB to tens-of-GB for one component arena is comfortably
inside both OS budgets.

### Benchmarking growth (no established prior art isolates it)

Methodology: (a) steady-state ns/entity inside capacity; (b) worst-single-batch wall time
for batches straddling a doubling boundary (Bevy: realloc+memcpy+faults) vs a slab-commit
boundary (boyko: syscall+faults) — `iter_custom` with min/max/p99 capture (criterion means
hide single-event spikes — in-repo Phase 12.6 lesson); (c) optionally a prefaulted variant
to subtract demand-zero fault cost symmetrically. The Bevy side must NOT pre-reserve.

---

## B. In-repo inventory (project-analyst) — file:line precise

### Arena internals (`crates/boyko_ecs/src/ecs/memory/arena.rs`)

- Three cfg arms, total+disjoint (`:163-218`): Windows `VirtualAlloc(NULL, cap, MEM_RESERVE|MEM_COMMIT, PAGE_READWRITE)`
  (`:171-178`, hand-declared kernel32 externs `:26-46`); Unix `mmap(NULL, cap, PROT_READ|PROT_WRITE,
  MAP_PRIVATE|MAP_ANONYMOUS)` with MAP_FAILED checked before NonNull (`:184-206`); fallback
  `cfg(any(miri, not(any(windows, unix))))` `std::alloc::alloc` (`:208-218`). Drop (`:332-381`):
  `VirtualFree(ptr, 0, MEM_RELEASE)` / `munmap(ptr, map_len)` / `dealloc(ptr, layout)` (M-001).
  NOTE: `MEM_RELEASE` with dwSize=0 already releases an entire reservation regardless of commit state.
- `Backing` per-arm shapes (`:117-137`): fallback `{layout}`, windows `{}`, unix `{map_len}` —
  growth adds `reserved_len`/`committed` watermarks here.
- Fields (`:86-112`): `ptr: NonNull<u8>` (write-once), `capacity: usize` (logical, = mapping length
  today — equality breaks under growth), `free_blocks: UnsafeCell<MemFreeBlockMaster>`.
  `capacity` is read through `&self` in `allocate_layout` (`:268-273`) → growth from the alloc
  path needs interior mutability (`Cell<usize>`), same M-003 single-thread argument (`:67-75`).
- Alloc fast path (`:243-279`): ALLOC1 debug_assert + `force_alloc_panic` escalation
  (`:249-267`); capacity debug_assert (`:268-273`); `allocate_from_free_blocks` (`:283-322`)
  takes `&mut MemFreeBlockMaster` from the UnsafeCell, best-fit, converts offset→pointer at
  `:320` (`self.ptr.as_ptr().add(block.start)`). **THE exhaustion panic is one line — `:277`**
  (`"Arena out of memory"`) — the natural grow-and-retry hook.
- **`MemFreeBlockMaster` is OFFSET-based** (`free_mem_block.rs:3-7`; `new_init(size)` seeds
  `[0, size)` `:52-56`) — with one contiguous reserve, growth = `insert([old_cap, new_cap))`
  and all existing offsets stay valid. HAZARD: the coalescer merges on offset adjacency
  (`:110-129`) — disjoint slabs would merge across VA gaps → UB. Keep VA contiguous.
- O4: free tracker seeded with LOGICAL size only (`:224-227`). Base align: pages (syscall
  arms) / 64 B (fallback); max requested align today = 32 (SIMD_BUFFER_ALIGN).

### Allocation sites / discipline

- **Exactly ONE production alloc site**: `ComponentPool::new` (`component_pool.rs:141`),
  ≈3 MB per pool per archetype at default sizing. Zero dealloc sites (blocks are permanent;
  `EcsMaster::clear` skips the arena, `ecs_master.rs:2798`).
- All call chains run on the owner thread: setup (`&mut EcsMaster`, ScheduleBuilder::build)
  or the apply window (deferred spawn/insert migration) — ALLOC1 TLS guard asserts never-in-
  `System::run_unsafe` (`arena.rs:249-254`, threadpool TLS `lib.rs:32-34`). `ComponentPool`'s
  SEND10 comment already specifies the growth concurrency contract (`component_pool.rs:1287-1293`).
- Arena is `!Send/!Sync` (auto-suppressed), lives in `Box<Arena>` (C-001 stable object address).

### Pointer-stability dependents (the proof obligation)

Exactly TWO cross-frame caches of arena-derived pointers:
1. `ComponentPool::buffer: NonNull<u8>` (`component_pool.rs:38`) — all row addressing
   `row_ptr(i)=buffer.add(i*stride)` (`:225-233`, SAFETY cites "fixed arena capacity" — comment
   needs updating, not code).
2. `Archetype::columns[c].ptr` (`archetype.rs:28-38`) — persistent snapshot of
   `pool.buffer_ptr()`, the Phase-7 hot read path; NOT refreshed on data ops. A dead
   `#[cold] refresh_all_columns` exists "for future arena-grow events" (`archetype.rs:372-389`)
   — under address-stable growth it MUST stay dead (cite as proof obligation).

Verified NOT arena-backed (growth-safe): BundleColumnCache (IDs), QueryState (IDs), tick
columns (`Box<[UnsafeCell<Tick>]>` heap, STORE2), EventBuffer (heap), entities_inland,
Resources, CommandQueue.

### Capacity plumbing

- `DEFAULT_ARENA_SIZE = 64 MiB` (`constants.rs:3`); `EcsMaster::new` → `Arena::new()`
  (`ecs_master.rs:409-410`); `EcsMaster::with_capacity` hardcodes DEFAULT (`:466`) — NO public
  arena-size knob today. Dead consts free to repurpose: `GROWTH_FACTOR=1.5`,
  `MAX_EXPANSION_FACTOR=8` (`constants.rs:71-77`).
- Tests assuming 64 MB / the panic: `arena.rs:412-422, 469-480, 516-553, 558-567`; fixtures
  4-64 MB in archetype/bundle tests; benches sized AROUND the ceiling:
  `random_access.rs:593-612` (1000→200 archetypes because of 64 MB), `query_dsl.rs:412-449`,
  `bench_bevy_vs_boyko/benches/profile_query.rs:283-299` (case F DEFERRED because the 3rd
  pool panics — ready-made growth-crossing scenario).

### Miri arms

Under Miri only the fallback arm compiles. Fallback alloc/dealloc exercised by 13 in-crate
arena tests (64 B–64 MB) + every `miri_phase*` world. KB-scale reserves + several growth
steps are affordable. Fallback has NO reserve/commit primitive → eager-full-reserve is the
design (multi-slab would corrupt the offset coalescer).

### Bench harness map

- `boyko_ecs/benches/allocator.rs` (alloc fast-path 0%-gate: `arena_allocate_layout` etc.),
  `arena_new.rs` (Arena::new + EcsMaster::new; PerIteration batch because commit-whole
  exhausts commit limit — reserve-lazy growth IMPROVES this bench's premise),
  spawn suites `bundle_static_cache.rs`, `phase12_5_spawn_batch.rs`.
- Cross-engine: **`crates/bench_bevy_vs_boyko/`** (bevy_ecs 0.18.1): `comparison.rs` g1-g4,
  `comparison_v2.rs` g2b/g5/g5d, `g6_for_each_chunk.rs` g6/g6b, `profile_*.rs`.
- Ready growth-crossing scenarios: resurrect profile_query case F; restore random_access to
  1000 archetypes; spawn_batch crossing the initial commit boundary; NEW head-to-head
  "cold world → spawn N crossing growth" vs Bevy with tail-latency capture.

### Existing growth TODOs in code

`archetype.rs:372-389` (refresh_all_columns, must stay dead), `component_pool.rs:33,41-43`
(reserved fields), `component_pool.rs:1291-1293` (SEND10 growth contract),
`arena.rs:108` (free_blocks future dealloc), `constants.rs:71-77` (dead growth consts),
`profile_query.rs:293`.
