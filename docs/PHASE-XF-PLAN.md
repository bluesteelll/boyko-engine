# Phase X.F — Arena Growth: Implementation Plan

Companion to `docs/PHASE-XF-RESEARCH.md` (cited as R-§A / R-§B; background is NOT repeated
here) and `docs/PHASE-XC-RESULTS.md`. Branch `ecs`.

## Goal

Replace the fixed-capacity component-data `Arena` (64 MB, `panic!` at `arena.rs:277`) with
**one contiguous multi-GB virtual reservation + lazy slab commit at the frontier**, so that:

- **Functionality**: archetype/pool creation never hits the 64 MB ceiling (the panic moves to
  reserve exhaustion, multi-GB away); `profile_query` case F and `random_access` 1000-archetype
  scenarios become runnable.
- **Performance**: the alloc fast path stays byte-identical (0%-gate); `EcsMaster::new` gets
  *cheaper* (reserve-only, zero commit charge); a growth event costs one syscall + demand-zero
  faults — **no O(N) memcpy ever** (vs Bevy's realloc+memcpy per table doubling, R-§A).
- **Binding user targets**: everything bench-confirmed; growth-crossing workloads ≥1.5× faster
  than Bevy total, worst-single-event orders cheaper.

Explicitly **in scope**: arena-level growth only. **Out of scope**: per-`ComponentPool`
capacity growth (pools stay fixed at `num_chunks × components_per_chunk` rows — X.F is the
substrate that makes pool growth possible later), and decommit/shrink (no competitor shrinks;
the door stays open — `free_blocks` already tracks free ranges, a future phase can
`MEM_RESET`/`MADV_FREE` them).

## Context and constraints

- Affected: `arena.rs` (core), `constants.rs`, `ecs_master.rs` (plumbing),
  `component_pool.rs` + `archetype.rs` (comments only), benches, tests.
- Invariants preserved: ALLOC1 (no allocation inside `System::run_unsafe`), M-001 (matching
  deallocator per cfg arm), M-003 (single-thread `&self` interior mutability), C-001
  (`Box<Arena>` stable address), offset-based free list, `!Send`/`!Sync` Arena,
  pointer stability of the two dependents (R-§B): `ComponentPool::buffer`,
  `Archetype::columns[].ptr`. `refresh_all_columns` MUST remain dead code.
- Target metrics: §"Metrics and validation" (B1–B7).

## Key decisions

### D1: One contiguous reservation; commit = frontier slabs (fixed direction, confirmed)
**What**: per arm — Windows `VirtualAlloc(NULL, os_reserve, MEM_RESERVE, PAGE_NOACCESS)` then
`VirtualAlloc(base+old, step, MEM_COMMIT, PAGE_READWRITE)`; Linux `mmap(NULL, os_reserve,
PROT_NONE, MAP_PRIVATE|MAP_ANONYMOUS)` then `mprotect(base+old, step, PROT_READ|PROT_WRITE)`;
fallback (miri/wasm/exotic) eager `std::alloc::alloc` of the full reserve, commit = no-op
watermark bump (X.C lesson: keep Miri on a modelable path).
**Why**: address stability for free (both dependents stay valid, the offset free list stays
valid, the coalescer's adjacency assumption holds — single contiguous VA, R-§B HAZARD avoided).
`PROT_NONE` reserve is overcommit-mode-2-proof (R-§A.2: not accounted because not
private-writable; `ENOMEM` surfaces at `mprotect`). Frontier-only `mprotect` keeps the mapping
at ≤2 VMAs (no `vm.max_map_count` creep). Windows reservation ceiling is fixed at creation
("cannot reserve a reserved page", R-§A.1) — hence one big reservation up front.
**Alternatives rejected**: realloc+memcpy (Bevy/flecs shape) — O(N) move invalidates both
pointer dependents and the entire Phase-7 column-cache design; chained multi-reservation
arenas — breaks the offset coalescer (would merge across VA gaps → UB) and the
single-`buffer.add(i*stride)` row addressing.
**Trade-off**: the ceiling is fixed at construction (mitigated by a multi-GB default, D2);
+2 syscalls of acquisition shape (reserve, then first commit) — measured in B2/B7.

### D2: Defaults — reserve 4 GiB (64-bit syscall arms), initial commit ZERO
**What**:
- `DEFAULT_ARENA_RESERVE = 4 GiB` under `cfg(all(not(miri), any(windows, unix),
  target_pointer_width = "64"))`; **64 MiB otherwise** (fallback arm incl. wasm32 — eager
  alloc of the reserve makes a multi-GB default fatal there; 32-bit guard included).
- `Arena::new()` = `with_reserve(DEFAULT_ARENA_RESERVE, 0)` — **initial commit zero**: no
  commit syscall, no commit charge, free list seeded EMPTY; the first pool allocation takes
  the cold grow path and commits `max(ARENA_MIN_SLAB, request)`.
**Why 4 GiB**: VA is free (Windows user VA 128 TB; R-§A: Unity 1 GB/World, Unreal 1 GB/pool,
Our Machinery "8 GB unremarkable"); 4 GiB ≈ 1300 default 3 MB pools ≈ covers the 1000-archetype
bench with headroom; 64× the old ceiling.
**Why commit zero**: `EcsMaster::new` drops below the X.C 1.10 µs arena cost (reserve-only);
test suites that create thousands of worlds stop charging 64 MiB commit each; the deferred
cost is ONE slab commit (~µs) at first archetype creation — already a µs-scale, apply-window
event (measured in B7). The X.C `arena_new` bench's `PerIteration` commit-charge constraint
disappears (premise documented in the bench header gets updated).
**Alternatives**: 1 GiB default — no cost difference (commit charge identical, VA free), just a
lower ceiling; rejected for headroom. Eager 64 MiB initial commit — keeps X.C behavior but
pays commit charge per world and makes `new()` strictly slower; rejected.
**Trade-off**: first-spawn latency +one slab commit (bounded, measured); a request larger
than 4 GiB total live data panics (loud, same surface as today, just 64× further out).

### D3: API surface and back-compat
**What**:
- New: `Arena::with_reserve(reserve: usize, initial_commit: usize) -> Self`.
- **`Arena::with_capacity(c)` ≡ `with_reserve(c, c)`** — reserve = commit = c, growth
  impossible past `c`. All ~30 existing fixture/bench call sites and 12 of the 13 arena unit
  tests stay green with ZERO edits (eager commit reproduces X.C behavior bit-for-bit,
  including the OOM-panic surface).
- `capacity()` returns the **reserve** (the logical allocation ceiling — what the OOM panic
  is measured against). New accessor `committed()` returns the frontier. Caller audit:
  `Arena::capacity()` is referenced only inside `arena.rs` tests — the semantic shift is
  contained; exactly ONE test updates (`arena_default_size_matches_constant`).
- `EcsMaster::new()` / `EcsMaster::with_capacity(..)` both construct `Arena::new()` (the
  `DEFAULT_ARENA_SIZE` hardcode at `ecs_master.rs:466` is deleted). New knob:
  `EcsMaster::with_arena_reserve(arena_reserve: usize) -> Self` — for tests, benches, and
  memory-constrained embedders. No builder; one function is the minimal surface.
**Why**: zero churn on 30+ call sites; `with_capacity` keeps its exact historical meaning
("this much usable memory, then panic") which tests rely on; the reserve knob is needed by
the small-reserve growth tests and Miri.
**Trade-off**: two semantics (`with_capacity` eager / `new`+`with_reserve` lazy) — documented
in both doc-comments; the eager path is just `with_reserve` + one `commit_frontier` call, not
a separate code path.

### D4: Commit slab policy — geometric doubling, clamped, request-dominant
**What** (pure function, unit-testable):
```text
needed = align_up(size + align - 1, ARENA_COMMIT_GRANULE)        // checked add, cold path
step   = max(clamp(committed, ARENA_MIN_SLAB, ARENA_MAX_SLAB), needed)
step   = min(step, os_reserve - committed)                        // 0 ⇒ reserve exhausted
```
Constants (replace the dead `GROWTH_FACTOR`/`MAX_EXPANSION_FACTOR` at `constants.rs:71-77` —
**deleted**, f32 factors are unsuitable for exact offset math):
- `ARENA_COMMIT_GRANULE = 64 KiB` — Windows reservation granularity; multiple of the 4 KiB
  commit/mprotect page everywhere; the reservation length itself is rounded up to this
  (`os_reserve = align_up(reserve, GRANULE)`) so a frontier commit can never overrun the
  kernel's page-rounded mapping (mprotect past the mapping ⇒ ENOMEM — a real hazard for
  sub-page reserves like the 64 B test arenas).
- `ARENA_MIN_SLAB = 2 MiB` — one slab covers a default ~3 MB pool in ≤2 events; 2 MiB is the
  Linux THP size (alignment of the *base* is page-only, so THP benefit is opportunistic —
  documented non-goal, khugepaged can still collapse).
- `ARENA_MAX_SLAB = 64 MiB` — commit-charge overshoot never exceeds today's ENTIRE arena;
  filling 4 GiB takes ~70 events lifetime (6 doublings + 63 max-steps), each one syscall —
  syscall count is irrelevant at this magnitude, so the bound optimizes for overshoot, not
  event count (mimalloc's 4 MiB segments serve many small allocs; our single consumer
  allocates ~3 MB pools — R-§A).
**GROW1 invariant (retry-once is provably sufficient)**: grow runs only when best-fit failed,
i.e. no free block ≥ `size + align - 1` (exactly `allocate_aligned`'s `required_size`).
The frontier insert coalesces with any free tail (left-neighbor merge at `old_frontier`), so
the post-grow tail block ≥ `step` ≥ `needed` ≥ `size + align - 1` ⇒ the retry's best-fit
cannot fail. If `step == 0` (frontier at ceiling) ⇒ loud panic before any state change.
**Trade-off**: up to 64 MiB committed-but-unused at the high end (≤ one old arena); ≤60 KiB
granule waste at the logical-reserve tail (committed but never offered to the free list).

### D5: `Backing` and `Drop` per arm
**What**:
- **Windows**: `Backing {}` unchanged. `Drop` unchanged: `VirtualFree(ptr, 0, MEM_RELEASE)`
  releases the entire reservation **regardless of commit state** (R-§B note; SAFETY comment
  gains this sentence).
- **Unix**: `Backing { map_len }` now stores `os_reserve` (the FULL reservation length passed
  to `mmap`), NOT the committed length. `munmap` unmaps irrespective of page protection;
  length must equal the original mapping — verified by construction (single assignment site).
- **Fallback**: `Backing { layout }` = full-reserve layout (size `align_up(reserve,
  CACHE_LINE_SIZE)`, align `CACHE_LINE_SIZE`); `dealloc` with the same layout (GlobalAlloc
  contract). Commit is a watermark bump only.
- The cfg matrix stays total + disjoint exactly as X.C built it (M1).
**Why**: M-001 preserved per arm; cross-dealloc statically impossible (each arm's `Backing`
only carries its own descriptor).

### D6: Hot path byte-identical; `committed` is the ONLY interior-mutable scalar
**What**: `allocate_from_free_blocks` — **untouched**. `allocate_layout` — the only change is
the `None` arm: `panic!(...)` → `self.grow_then_retry(layout)` where `grow_then_retry` is
`#[cold] #[inline(never)] fn(&self, Layout) -> NonNull<u8>`. The `Some` branch, the ALLOC1
guards, and the `force_alloc_panic` escalation are untouched (growth inherits ALLOC1
restriction by running *inside* `allocate_layout`, after the guards).
**Refinement of the fixed direction** ("`capacity` reads move to `Cell<usize>`"): only the
**`committed` watermark** needs interior mutability (`Cell<usize>`, mutated by the cold grow
path through `&self`). The **reserve stays a plain immutable `usize`** — it never changes
after construction, so `capacity()` keeps a zero-overhead plain load and immutability becomes
a free invariant. The M-003 single-thread argument covers the `Cell` (see Soundness).
The `arena.rs:268-273` debug_assert becomes `layout.size() <= self.reserve` (message:
"...exceeds arena reserve ..."): it remains a fast sanity check — a request that can never
fit even an empty arena; sub-reserve requests that still can't be satisfied (alignment slack,
fragmentation at the ceiling) are handled by the cold panic with full diagnostics
(request/align/committed/reserve). Release builds compile the assert out — the hot path reads
**nothing new**.
**Validation**: B1 0%-gate (`arena_allocate_layout` group + spawn suites) under the X.B
git-stash A/B multi-run methodology; optionally asm diff of `allocate_layout`.

### D7: `allocate_from_free_blocks` does NOT grow
**What**: the public `Option`-returning probe keeps its exact semantics (`None` on no-fit) —
growth lives exclusively in `allocate_layout`'s cold arm.
**Why**: the `None` path is load-bearing for tests (`..._returns_none_when_oom`) and for any
future caller that wants a non-committing probe; a growing probe would also double the cold
logic. Single growth funnel = single audit point.

### D8: Documentation/comment obligations (code-adjacent, same waves)
- `component_pool.rs:225-233` SAFETY: "(fixed arena capacity)" → "the base is write-once and
  never moves: Phase X.F growth only commits fresh pages at the frontier of the SAME
  reservation; previously returned blocks are never remapped or relocated".
- `archetype.rs:372-389` `refresh_all_columns` doc: add "Phase X.F confirmed address-stable
  growth — this MUST remain dead; it exists only for a hypothetical future relocating arena."
  Keep `#[cold] #[allow(dead_code)]`.
- `component_pool.rs:1287-1293` SEND10 bullet 3 already specifies the growth contract
  (apply-window only) — add "(realized by Phase X.F)" pointer.
- `ecs_master.rs:398-406` X.C doc paragraph updated (reserve-lazy, zero initial commit).

## Data structures

```rust
// arena.rs — no #[repr] needed: Arena is not frame-hot (alloc path runs at
// setup/apply-window only); no false-sharing concern (!Send/!Sync). N/A by measurement site.
pub struct Arena {
    ptr: NonNull<u8>,                       // write-once base of the SINGLE reservation
    reserve: usize,                         // logical ceiling, cache-line rounded; capacity()
    committed: Cell<usize>,                 // frontier, GRANULE-aligned, monotonic, <= os_reserve;
                                            // mutated only by grow_then_retry (cold, owner thread)
    backing: Backing,                       // per-arm Drop descriptor (D5)
    free_blocks: UnsafeCell<MemFreeBlockMaster>, // unchanged; offsets always < reserve
}
// os_reserve is recomputed where needed: align_up(reserve, ARENA_COMMIT_GRANULE)
// (1 ALU op on the cold path; not worth a field). Unix Backing.map_len == os_reserve.
```

Free-list state machine: `initial_commit == 0` ⇒ `MemFreeBlockMaster::new()` (EMPTY — first
alloc faults into the grow path; empty-tree best-fit returns `None` by construction);
`initial_commit > 0` ⇒ `new_init(min(frontier, reserve))` exactly as today.

## Public API

```rust
impl Arena {
    pub fn with_reserve(reserve: usize, initial_commit: usize) -> Self; // clamps commit <= reserve
    pub fn with_capacity(capacity: usize) -> Self;  // == with_reserve(c, c)  (back-compat)
    pub fn new() -> Self;                           // == with_reserve(DEFAULT_ARENA_RESERVE, 0)
    pub fn capacity(&self) -> usize;                // the reserve (OOM ceiling)
    pub fn committed(&self) -> usize;               // the frontier (diagnostics/tests)
    // allocate_layout / allocate_from_free_blocks / allocate<T>: signatures unchanged
}
impl EcsMaster {
    pub fn with_arena_reserve(arena_reserve: usize) -> Self; // the only new knob
}
```

## Algorithm — the cold grow path

```rust
#[cold]
#[inline(never)]
fn grow_then_retry(&self, layout: Layout) -> NonNull<u8> {
    // 0. size==0 guard: a zero-size request can never be satisfied by growth
    //    (allocate_aligned returns None for size 0) — panic immediately, don't commit.
    // 1. needed = align_up(size.checked_add(align - 1).expect(..), GRANULE)
    //    (checked: Layout's own invariant makes overflow near-impossible; the check is
    //    free on a cold path and makes it airtight).
    // 2. step per D4; if step == 0 -> #[cold] panic
    //    "Arena reserve exhausted: request {size}B align {align}, committed {c}/{r}".
    // 3. commit_frontier(old, old + step)   // per-arm primitive, see below
    // 4. committed.set(old + step);
    //    free_blocks.insert([min(old, reserve), min(old+step, reserve)))
    //    — left-coalesces with a free tail at the old frontier automatically.
    // 5. retry allocate_from_free_blocks(layout) — GROW1 proves Some;
    //    None here = logic bug -> panic with full diagnostics (debug and release).
}

#[cold]
fn commit_frontier(&self, old: usize, new: usize) {
    // windows: VirtualAlloc(base+old, new-old, MEM_COMMIT, PAGE_READWRITE) — idempotent
    //          re-commit documented (R-§A); NULL -> panic (commit-charge exhausted).
    // unix:    mprotect(base+old, new-old, PROT_READ|PROT_WRITE) — != 0 -> panic
    //          (ENOMEM = the overcommit-mode-2 failure surface, R-§A.2).
    // fallback: no-op (whole reserve eagerly allocated RW).
}
```
- Complexity: O(log F) free-list insert (F = free-block count, single digits in practice) +
  one syscall. No iteration over existing data — **O(1) in live entities** (the headline
  advantage over Bevy's O(N) memcpy).
- Cache behavior: cold path; touches the free-list BTreeMaps + one syscall. Demand-zero
  faults are deferred to first touch by the pool — identical to Bevy's freshly-realloc'd
  pages (R-§A), so the *differential* vs Bevy is pure memcpy deletion.
- Branching/I-cache: `#[cold] #[inline(never)]` keeps `allocate_layout`'s body compact; all
  panics live inside the cold fn.
- Frequency: ~70 events over an entire 4 GiB lifetime (D4) — amortization is irrelevant;
  worst-event latency is what we bench (B4/B6).

## Soundness

1. **Growth never moves anything**: `ptr` is write-once; commit changes protection/charge on
   `[base+old, base+new)` only — the OS does not touch committed pages' contents or addresses
   (idempotent-commit documented; mprotect affects the new range only). Therefore every
   previously returned `NonNull` — including the two cross-frame dependents
   `ComponentPool::buffer` (`component_pool.rs:38`) and `Archetype::columns[c].ptr`
   (`archetype.rs:28-38`) — stays valid. `refresh_all_columns` stays dead (D8). The free
   list's offset model is untouched: one contiguous reservation ⇒ offset adjacency ==
   VA adjacency ⇒ the coalescer (`free_mem_block.rs:110-129`) is correct at the frontier.
2. **Interior mutability is race-free**: `Arena` is `!Send`/`!Sync` (auto-suppressed via
   `NonNull`/`UnsafeCell`/`Cell`); every grow runs inside `allocate_layout`, which ALLOC1
   confines to the owner thread at setup or the apply window (debug_assert + opt-in
   `force_alloc_panic`; threadpool TLS). No concurrent reader of `committed` or
   `free_blocks` can exist while grow mutates them — same M-003 argument as today, now also
   covering the `Cell` (a `Cell` read/write is a plain load/store; with no concurrent access
   by construction there is no tear and no race). `ComponentPool`'s SEND10 bullet 3 already
   pre-states this contract for workers.
3. **New/changed unsafe blocks** (each lands with the listed SAFETY argument):
   - W-RES `VirtualAlloc(MEM_RESERVE, PAGE_NOACCESS)` — NULL-checked; size > 0.
   - W-CMT `VirtualAlloc(MEM_COMMIT)` in `commit_frontier` — range inside own reservation
     (`new <= os_reserve`), granule-aligned, NULL-checked, idempotent by OS contract.
   - U-RES `mmap(PROT_NONE, MAP_PRIVATE|MAP_ANONYMOUS)` — `MAP_FAILED` checked BEFORE
     `NonNull::new` (X.C trap, preserved).
   - U-CMT `mprotect(PROT_READ|PROT_WRITE)` — page-aligned base+len inside own mapping
     (granule ⊇ page), return checked.
   - F-RES `alloc(layout)` — unchanged from X.C (non-zero size, power-of-two align).
   - Drop arms — unchanged shape; Windows SAFETY gains the "releases whole reservation
     regardless of commit state" sentence; Unix SAFETY states `map_len == os_reserve`.
   - The existing offset→pointer SAFETY at `arena.rs:320` gains: "block offsets are < reserve
     and below the committed frontier (free-list ranges are only ever seeded from committed
     space), so the pointer is into committed, RW memory."
4. **M-001**: matching deallocator per arm, statically enforced by the cfg-gated `Backing`
   (unchanged). Drop order in `EcsMaster` unchanged (arena declared last).
5. **Edge cases**: size 0 (panic, no commit — D7 guard); request > reserve (debug_assert,
   then cold exhaustion panic in release); request ≤ reserve but frontier at ceiling (step==0
   panic); alignment slack pushing `needed` past remaining reserve (panic — honest: the
   ceiling is `reserve`, slack can cost up to `align-1` extra; documented); `committed`
   monotonic and ≤ `os_reserve` (debug_assert); overflow in `size + align - 1`
   (`checked_add`); 64 B test arenas (os_reserve rounds to one granule; logical capacity
   stays 64 B — O4 preserved).
6. **Miri coverage**: the fallback arm exercises ALL growth bookkeeping (step math, frontier
   inserts, coalescing, offsets, retry) with small reserves under Tree Borrows — what it
   CANNOT prove is the real syscall semantics (reserve/commit/round-trip, PAGE_NOACCESS
   faulting, ENOMEM surfaces). Those are covered natively: the Windows arm by the unit tests
   on the dev/CI host (real VirtualAlloc round trips, incl. partially-committed Drop loops);
   the Unix arm minimally by `cargo check --target x86_64-unknown-linux-gnu` (X.C precedent)
   — native Linux `cargo test` listed as an open question.

## Integration

| File | Change |
|---|---|
| `constants.rs` | + `DEFAULT_ARENA_RESERVE` (cfg-gated 4 GiB / 64 MiB), `ARENA_COMMIT_GRANULE`, `ARENA_MIN_SLAB`, `ARENA_MAX_SLAB`; − `GROWTH_FACTOR`, `MAX_EXPANSION_FACTOR`, `DEFAULT_ARENA_SIZE` |
| `arena.rs` | fields, `with_reserve`, reserve-only acquisition, `commit_frontier`, `grow_then_retry`, `committed()`, Drop/SAFETY updates, debug_assert reword, tests |
| `ecs_master.rs` | `new`/`with_capacity` → `Arena::new()`; + `with_arena_reserve`; doc updates |
| `component_pool.rs`, `archetype.rs` | comments only (D8) — zero code change |
| benches/tests | per §Metrics |

No other module touches the arena's allocation surface (single production alloc site:
`ComponentPool::new`, R-§B).

## Implementation plan (waves — each compiles + tests green)

1. **W1 — math**: `constants.rs` (ADD the four consts only; deletions deferred to W4),
   `arena.rs` private `grow_step(committed, needed, os_reserve) -> usize` pure fn +
   table-driven unit tests (min-slab first step, doubling, max clamp, request-dominant,
   ceiling clamp, step==0 at ceiling).
2. **W2 — acquisition refactor**: `arena.rs` — fields (`reserve`, `committed: Cell`),
   `with_reserve` (reserve-only arms + optional eager `commit_frontier`), `with_capacity` →
   `with_reserve(c, c)`, `new()` → defaults, `capacity()`/`committed()`, `Backing.map_len =
   os_reserve`, Drop SAFETY updates. Update the ONE test (`arena_default_size_matches_constant`).
3. **W3 — growth**: `grow_then_retry` + `commit_frontier` (3 arms), `allocate_layout` None
   arm, debug_assert reword, size-0 guard; new unit tests (§Test matrix U1–U8).
4. **W4 — plumbing + comments**: `ecs_master.rs` (3 constructors + doc), `component_pool.rs`
   / `archetype.rs` comment updates, `constants.rs` deletions + fix remaining references.
5. **W5 — tests**: `tests/arena_growth.rs` (integration I1–I2), `tests/miri_arena_growth.rs`
   (M1), full suite + Miri gate.
6. **W6 — benches**: `arena_new.rs` header/premise update + `arena_first_pool_alloc` +
   `commit_slab/{2,16,64}MiB`; NEW `bench_bevy_vs_boyko/benches/growth_crossing.rs` (g7/g7b);
   resurrect `profile_query.rs` case F; restore `random_access.rs` to 1000 archetypes;
   orchestrator/tester runs the A/B gates.

## Metrics and validation

### Benchmarks (binding)
- **B1 — 0%-gate**: `allocator.rs` groups (`arena_allocate_layout`, `alloc_cold`,
  `alloc_free_roundtrip*`, `insert_*`), `bundle_static_cache.rs`,
  `phase12_5_spawn_batch.rs`, `comparison.rs` g1–g4: within ±2% vs baseline, git-stash A/B,
  ≥3 runs (X.B methodology; `--features bench-alloc` for low variance).
- **B2**: `Arena::new()` ≤ 1.10 µs (X.C baseline; expect ~0.2–0.5 µs reserve-only).
- **B3**: `EcsMaster::new` ≤ 7.5 µs (no regression; expect ≤ 7.23 µs).
- **B4 — growth event**: `commit_slab/2MiB ≤ 10 µs`, `/64MiB ≤ 50 µs` (syscall-only;
  fresh arena per iter via `iter_batched` — cheap now, no commit charge).
- **B5/B6 — vs Bevy (the headline)**: `growth_crossing.rs`, cold worlds both sides, Bevy NOT
  pre-reserved. Workload: M = 8 archetypes × 200 k entities each (3-component bundle), spawned
  in 16 sub-batches of 12.5 k per archetype. Rationale: arena growth fires at the single
  production alloc site — pool construction — so the crossing workload is multi-archetype
  creation + population (~72 MB pools ⇒ ~6 boyko slab commits past the old ceiling); the
  sub-batching forces Bevy's per-table doublings (its `spawn_batch` reserves only each
  batch's `size_hint` — incremental load is the realistic, fair pattern). Measured via
  `iter_custom`: `g7_growth_total` (whole-workload wall time) and `g7b_worst_batch` (max
  single sub-batch duration per run — criterion means hide spikes, Phase 12.6 lesson).
  **Targets: boyko total ≥ 1.5× faster; boyko worst batch ≤ 0.1× Bevy's** (expect ~100×:
  one slab syscall + faults vs largest-table memcpy + realloc + faults).
- **B7**: `arena_first_pool_alloc` (cold default arena + one 3 MB pool-sized request — the
  deferred-commit cost made visible): ≤ 10 µs.
- **Regression-fixed demos**: `profile_query` case F runs (delete the DEFERRED block,
  `profile_query.rs:283-299`); `random_access.rs` restored to the plan's original 1000
  archetypes (~256 MB commit — bench-only weight).

### Test matrix
- **Unit (arena.rs)**: U1 `grow_step` table; U2 first alloc commits MIN_SLAB & succeeds;
  U3 frontier insert coalesces with free tail (assert `free_blocks` len == 1 post-grow);
  U4 oversized request (10 MiB on fresh arena) → single covering step; U5 reserve-exhausted
  panic (`with_reserve(4 MiB, 0)`, 8 MiB request, catch_unwind); U6 `with_capacity`
  back-compat (`capacity == committed == rounded c`; the 12 untouched tests + reworked
  default-size test); U7 alignment at a grown frontier (64 B-align request straddling
  growth); U8 Drop with partially-committed reserve ×50 loop (native syscall round trip);
  plus `committed()` monotonic/granule-aligned/≤ os_reserve. The existing OOM tests keep
  their meaning: `with_capacity(64)` + 128 B request still panics (reserve exhausted ==
  the old surface); `allocate_from_free_blocks` still returns `None` (D7).
- **Integration (`tests/arena_growth.rs`)**: I1 default `EcsMaster` creates ~30
  single-component archetypes (~90–100 MB — past the old 64 MB ceiling; pre-X.F this
  panics); I2 `EcsMaster::with_arena_reserve(16 MiB)` spawns across 4 archetypes crossing
  ≥2 slabs, then full query iteration validates data integrity (pointer-stability witness).
- **Miri (`tests/miri_arena_growth.rs`, fallback arm)**: M1 `with_reserve(8 MiB, 0)` + six
  1 MiB allocs ⇒ 3 growth events (2→4→8 MiB); write head+tail bytes of every block;
  asserts committed watermarks + non-overlap. Existing `miri_phase*` worlds keep running on
  the fallback default (64 MiB eager — unchanged behavior).
- **debug_assert! invariants**: granule alignment + monotonicity of `committed`; free-list
  insert ranges ⊂ `[0, reserve)`; `step > 0` before commit; post-grow retry success.

## Open questions for the critic

1. **Default reserve 4 GiB** — confident on OS budgets (R-§A), least confident about exotic
   tooling interactions (e.g. ASan shadow over large PROT_NONE reservations) for downstream
   users. Would accept 1–2 GiB if the critic weights that risk higher; nothing else changes.
2. **`with_capacity` = eager commit** (back-compat) vs uniform lazy semantics everywhere —
   I chose zero-churn bit-compat for ~30 call sites; a purist may prefer one semantic.
3. **`capacity()` now means reserve** (4 GiB on default worlds). Test-only callers today, but
   it is public API; alternative is renaming to `reserve()` and deprecating `capacity()`.
4. **`ARENA_MAX_SLAB = 64 MiB`** (overshoot-bounded) vs 256 MiB (fewer late events) — events
   are µs-scale either way; I optimized for commit-charge honesty.
5. **g7 workload fairness**: growth fires at pool creation (pool capacity is fixed — R-§B),
   so the crossing workload is multi-archetype; and Bevy's doublings require sub-batched
   spawning. Is the critic satisfied this honors "Bevy MUST NOT pre-reserve" in spirit?
6. **Native Linux runtime testing**: X.C precedent verified the unix arm by cross `cargo
   check` only. The mprotect grow path deserves one real-Linux `cargo test` run (WSL/CI) —
   who owns that gate?
7. Whether to expose `Arena::precommit(bytes)` (warm-up/prefault knob for latency-critical
   embedders) — deferred; trivial to add later on the same `commit_frontier` primitive.

---

# R2 (FINAL — folds critic round 1)

Critic verdict on R1: **CHANGES-REQUESTED**. Everything below is BINDING; the developer reads
body + R2, **R2 wins on conflict**. Changelog map: C1 → §Algorithm steps 2-5 + GROW1 + tests;
C2/C3 → B5/B6 wholesale; W1 → D3/W2/Soundness; W2 → D3's "exactly ONE test" claim; W3 →
Soundness §6 + I1; W4 → B1; W5 → Soundness §6 / OQ6; N1-N3, N6 adopted; OQ ledger at the end.

**VERIFIED-SOUND (critic-confirmed — preserve, do not weaken):** address-stability argument per
arm (Soundness §1); the `Cell`/`UnsafeCell` borrow structure (`allocate_from_free_blocks` drops
its `&mut` before returning; grow's insert and retry take fresh sequential borrows; no
re-entrancy — `MemFreeBlockMaster::insert` allocates from the GLOBAL allocator);
`MemFreeBlockMaster::new()` exists (`free_mem_block.rs:48-50`) and an empty tree returns `None`
cleanly; the OOM test (`arena.rs:469-480`) greps NO message — only the stale comment at `:472`
updates; D6/D7 hot-path discipline; D4 constants.

## C1 (CRITICAL) — R1's GROW1 proof was WRONG; sufficiency check added before any state change

R1's chain `tail ≥ step ≥ needed ≥ required_size` breaks twice: (1) `step = min(step,
os_reserve − committed)` can yield `0 < step < needed`; (2) the insert truncates at the LOGICAL
reserve (`hi = min(old+step, reserve)`), so offered bytes < step. Reachable trace (critic's, now
test U9): `with_reserve(3 MiB + 32 KiB, 0)` → 64 KiB alloc (commits 2 MiB) → 3 MiB request:
step = 1,114,112 < needed; usable truncated to reserve; merged tail ≈ 3,112,929 < required_size
≈ 3,145,7xx → retry `None` → release panic mislabeled "logic bug" for LEGITIMATE exhaustion.

**Fix — `grow_then_retry` steps 2-5 are replaced by:**

```text
required_size = size + align - 1            // EXACTLY allocate_aligned's criterion — NOT `needed`
old    = committed.get()
step   = per D4 (unchanged formula; `needed` is now ONLY a step-sizing input)
lo     = min(old, reserve)
hi     = min(old + step, reserve)
usable = hi - lo                            // 0 when the frontier is already past the logical reserve
tail   = length of the free block ending exactly at `lo`   // one read-only end_map lookup; 0 if none
if tail + usable < required_size  ->  #[cold] EXHAUSTION panic, NO state change
                                      (msg: size/align/required/tail/usable/committed/reserve)
commit_frontier(old, old + step); committed.set(old + step);
free_blocks.insert([lo, hi))                // usable > 0 guaranteed below; left-coalesces with tail
retry allocate_from_free_blocks(layout)     // Some by GROW1 (restated below);
                                            // None = GENUINE logic bug -> panic, debug AND release
```

- The check uses **`required_size`, NOT granule-rounded `needed`** — a `needed`-based check
  FALSELY exhausts satisfiable cases (critic's example, now test U10: reserve = 100 KiB ⇒
  usable 102,400; a 35 KiB / 64 KiB-align request has required_size 101,375 ≤ usable but
  needed 131,072 > usable).
- **Corollaries after a passing check** (both `debug_assert!`ed): best-fit failure ⇒
  `tail < required_size` ⇒ `usable > 0` ⇒ `step > 0`. R1's standalone `step == 0` panic FOLDS
  into this single check — ONE exhaustion surface. The never-empty-insert guard is now a proven
  corollary, not a runtime branch (keep it as a `debug_assert!(usable > 0)`).
- When `old ≥ reserve` (granule-rounded frontier past the logical reserve): `usable == 0` and
  `tail < required_size` (best-fit just failed) ⇒ exhaustion panic — consistent, no underflow
  (`lo`/`hi` clamped; `old + step ≤ os_reserve` by the D4 clamp ⇒ no overflow).
- Right-coalescing at the frontier is impossible (all previously offered space ⊂ `[0, lo)`), so
  the merged block is exactly `[lo − tail, hi)`, length `tail + usable`.
- New `pub(crate)` read-only tail probe on `MemFreeBlockMaster` (length of the free block ending
  at a given offset — the end_map BTree already indexes block ends for the coalescer); the
  borrow is taken and dropped before the insert (preserves the verified-sound borrow structure).

**New unit tests:** **U9** — the critic's trace verbatim ⇒ `catch_unwind` sees the EXHAUSTION
message (not "logic bug") AND `committed()` is unchanged after unwind (no-state-change witness).
**U10** — false-exhaustion guard: `with_reserve(100 KiB, 0)` +
`Layout::from_size_align(35 KiB, 64 KiB)` must SUCCEED (passes the `required_size` check, would
fail a `needed` check — regression net against reintroducing granule-rounded sufficiency).

## C2 + C3 — B5/B6 workload reworked with in-plan arithmetic (supersedes 8×200k×3-small wholesale)

R1's workload was spawn-dominated: boyko is ~1.35× of Bevy's TIME on steady-state small-comp
spawn (Phase 12.5/12.6: ~35 vs ~26 ns/e) ⇒ critic's arithmetic: Bevy ≈ 42 ms spawn + 6-12 ms
memcpy ≈ 50 ms vs boyko ≈ 56 ms ⇒ **~0.9×, failing the ≥1.5× target by construction**. Fix:
FATTER components — Bevy's doubling-memcpy term scales with BYTES while per-entity bookkeeping
converges. Sizes pin to the pool class table (`constants.rs:44-61`): ≤16 B ⇒ 262,144 rows/pool;
17-64 ⇒ 131,072; 65-256 ⇒ 65,536; >256 ⇒ 32,768. Final spec + arithmetic in the closing section.
Shape decision: the critic's 128B-class, taken at **192 B** — in-class insurance (the ratio
improves monotonically with bytes/entity at identical pool geometry and event structure; my model
puts 3×128 B at a 1.3-1.8× spread straddling the gate, 3×192 B at 1.7-2.1×); cost is bench-only
RAM (~0.6 GB/side). N4/N5 folds are in the final spec.

## W1 — granule induction (D3/W2 amendment + Soundness)

- D3/W2: **`committed = min(align_up(initial_commit, ARENA_COMMIT_GRANULE), os_reserve)`** —
  the eager arm rounds UP to the granule. For `with_capacity(c)`: `os_reserve = align_up(c, G)`
  ⇒ `committed = os_reserve`, free list still seeds `[0, c)`, `capacity() == c` — the 64 B test
  arenas (O4) are preserved.
- Soundness gains the induction chain: **(a)** every `commit_frontier` base is granule-aligned —
  induction: initial `committed` granule-aligned by construction; `step` is granule-aligned
  because `needed` is `align_up`'d, MIN/MAX_SLAB are granule multiples, `clamp`/`max` of aligned
  values is aligned, and `min(·, os_reserve − committed)` subtracts two aligned values ⇒
  VirtualAlloc/mprotect always gets page-aligned base+len; **(b)** `step == 0 ⇔ committed ==
  os_reserve` — exact tail detection, no sub-granule residue can strand the clamp; **(c)**
  never-empty-insert: `[lo, hi)` inserted only when `usable > 0` (C1 corollary) — the degenerate
  `[reserve, reserve)` insert cannot reach the free list.

## W2 — TWO tests update, not one (supersedes D3's "exactly ONE test updates")

`arena_default_size_matches_constant` AND `arena_default_size_drop_loop_does_not_crash`
(`arena.rs:516-553` — references `DEFAULT_ARENA_SIZE` + asserts `capacity() == 64 MiB`).
Re-spec the drop-loop test: `Arena::new()` (default reserve, commit 0) + ONE small alloc
(first-alloc grow commits MIN_SLAB) + Drop, ×50 loop. DIFFERENTIATED from U8 (kept): U8 loops
small `with_reserve` arenas; the re-specced test is the ONLY one exercising the DEFAULT multi-GB
reservation round trip (reserve-only acquire → partial commit → full release).

## W3 — Miri honesty

- **(a)** SUPERSEDES the R1 sentence "Existing `miri_phase*` worlds keep running on the fallback
  default (64 MiB eager — unchanged behavior)" — that was WRONG: `new()` is lazy on ALL arms
  (the fallback eagerly allocates backing, but the free list seeds EMPTY), so **every existing
  `miri_phase*` world now traverses `grow_then_retry` on its first pool allocation**. Coverage
  BONUS (growth bookkeeping under Tree Borrows on every world-creating Miri test) — stated so a
  Miri behavior diff is not misread as a regression.
- **(b)** I1 (~90-100 MB) exceeds the 64 MiB fallback default ⇒ **`#[cfg_attr(miri, ignore)]`
  on I1** (pinned: I2 + M1 already cover growth under Miri; I1's purpose is the real-OS
  past-the-old-ceiling witness — Miri adds nothing there).

## W4 — asm diff promoted into the BINDING B1 gate

B1 addendum (tester gate, alongside criterion ±2%): asm diff of `Arena::allocate_layout`
(release + bench features): the **Some-path instruction sequence byte-identical** to baseline;
the only permitted delta is the `None` arm's tail (panic → call `grow_then_retry`).

## W5 — real-Linux runtime gate is BINDING (supersedes OQ6)

One real-Linux `cargo test --all-targets` of the unix commit path (mprotect grow), via WSL on
the dev host; owner = tester/orchestrator, wave W5. If WSL is unavailable, the results doc MUST
carry an explicit residual-risk entry (unix arm verified by cross-check + code review only).

## Nits — all adopted

- **N1** — B4 slab recipes pinned (fresh `with_reserve(256 MiB, 0)` per iter via `iter_batched`):
  2 MiB event = one 64 KiB alloc (step = MIN_SLAB); 16 MiB = one alloc of `16 MiB − GRANULE`
  (request-dominant: `needed` rounds to exactly 16 MiB); 64 MiB = one alloc of `64 MiB − GRANULE`
  (`needed` is NOT MAX_SLAB-clamped in D4's `max(clamp(..), needed)` — single huge requests must
  be one event; this recipe exercises exactly that).
- **N2** — every cold-path `align_up` / `size + align − 1` is CHECKED (`checked_add` +
  diagnostics) — guards `with_reserve(usize::MAX, ..)` and pathological layouts.
- **N3** — `assert!(reserve > 0)` in `with_reserve` (cold; kills the zero-length
  mmap/VirtualAlloc edge and the degenerate empty arena).
- **N6** — D8 sweep additionally covers the stale fixed-ceiling comments at
  `query_dsl.rs:412-449` and `component_pool_dense.rs:75`.

## Open-questions ledger (critic-resolved; binding)

| OQ | Verdict |
|---|---|
| 1. 4 GiB default | **KEEP**; +1 doc sentence on `DEFAULT_ARENA_RESERVE`: large `PROT_NONE` reservations show up in ASan/valgrind-class tooling as address space, not memory; constrained embedders use `EcsMaster::with_arena_reserve` |
| 2. `with_capacity` eager | **KEEP** |
| 3. `capacity()` = reserve | **ACCEPT**; +1 doc sentence: default worlds report 4 GiB (the OOM ceiling) — use `committed()` for resident-memory expectations |
| 4. `MAX_SLAB = 64 MiB` | **KEEP** |
| 5. workload fairness | superseded by the C2 rework (resolved) |
| 6. Linux runtime gate | **BINDING** (W5) |
| 7. `Arena::precommit` | **DEFER** |

## FINAL — B5/B6 workload spec + arithmetic (supersedes §Metrics B5/B6)

**Workload (`growth_crossing.rs`):** 16 archetypes × 3 components × **192 B** each (65-256 B
class ⇒ 65,536 rows/pool ⇒ 12 MiB/pool arena alloc), **60,000 entities/archetype** (≤ 65,536
pool cap — pools are fixed, X.F scope), spawned in **60 sub-batches of 1,000** per archetype via
`spawn_batch`. N = 960,000 entities; payload = 553 MB. Cold worlds both sides; Bevy NOT
pre-reserved (its `spawn_batch` reserves each batch's `size_hint` only ⇒ doubling path forced).

**Harness (N4, binding):** criterion `iter_custom`, `sample_size = 10`, `measurement_time ≥
20 s`, warm-up 3 s (~130-250 ms per iteration per side); timed region = world construction +
all spawning + (boyko) command apply; **world `Drop` OUTSIDE the timed region** (stop the
`Instant` before drop). Per-sub-batch durations recorded per iteration.

**Model assumptions (pinned):** A1 payload-stripped warm spawn bookkeeping ≈ 33 ns/e boyko /
26 ns/e Bevy (Phase 12.5/12.6 records); A2 first-touch streaming write into demand-zero pages
(fault+zero+write) ≈ 6 GB/s; A3 doubling memcpy with faulting destination ≈ 4-6 GB/s (named
risk: Bevy-side allocator warm-reuse of intermediates can push this toward ~10 GB/s); A4 commit
event ≤ 50 µs (B4-bound). Bevy copied rows/archetype (RawVec path 1000→2000→4000→8000→16000→
32000→64000): 63,000 ⇒ ×16 ≈ 1.008 M rows ≈ 581 MB (+~3% tick/entity columns — omitted).

| Term | boyko | Bevy |
|---|---|---|
| bookkeeping (A1) | 0.96 M × 33 ns ≈ 32 ms | 0.96 M × 26 ns ≈ 25 ms |
| payload first-touch (A2) | 553 MB / 6 ≈ 92 ms | 553 MB / 6 ≈ 92 ms |
| doubling memcpy (A3) | — (the headline deletion) | 581 MB / 4-6 ≈ 97-145 ms |
| tick-buffer eager memset (**N5**) | 48 pools × 2 × 65,536 × 4 B = 24 MiB ≈ 2.5 ms (heap `Box<[UnsafeCell<Tick>]>`, NOT arena) | lazy per-row stamps (inside A1) |
| arena commit events (A4) | 12 × ≤ 50 µs ≤ 0.6 ms | — |
| **total** | **≈ 127 ms** | **≈ 214-262 ms** |

**Expected ratio ≈ 1.7-2.1×** (critic's envelope for the class: 1.7-2.7×). **Binding target:
g7 total ≥ 1.5× (KEPT).** If measured < 1.5×: decompose per the table FIRST — the boyko-side
suspects are the N5 memset + payload term (the arena events are B4-bounded ≤ 0.6 ms total and
CANNOT explain a miss); a Bevy-side warm-reuse compression of A3 returns to the architect with
the measured term split — it does NOT silently relax the target.

**Commit-event recount (C3):** arena demand = 48 pools × 12 MiB = **576 MiB** (tick buffers are
heap — `component_pool.rs:166-171` "STORE2 (not arena-resident)"). D4 trace: steps
{12, 12, 24, 48, 64×8} MiB (+1 granule align slack each — omitted) ⇒ committed 12→24→48→96→160→
224→288→352→416→480→544→608 ⇒ **12 events**; overshoot 32 MiB ≤ MAX_SLAB. (The critic's ~8-9
was for the 8 MiB-pool/128 B variant: steps {8, 8, 16, 32, 64×5} ⇒ 9 — the count is
shape-dependent; both shown.)

**g7b — worst EVENT (target ≤ 0.1× KEPT, metric sharpened):** per side, report the per-iteration
sub-batch **spike = max − median** (and the raw max alongside, untargeted). Rationale: both raw
maxima embed the same per-batch payload floor plus per-archetype setup (boyko's first batch per
archetype carries the 1.5 MiB N5 tick memset ≈ 0.25 ms; Bevy's carries table init); max − median
cancels the common terms and isolates the growth EVENT — which is the claim under test.
Expected: boyko spike ≈ 0.27 ms (≈ 80% N5 memset + pool setup; the arena commit itself ≤ 50 µs)
vs Bevy spike ≈ 3.0-4.5 ms (final doubling: 32,000 rows × 576 B ≈ 18.4 MB at A3) ⇒
**≈ 0.06-0.09×, inside the ≤ 0.1× gate with margin**; a near-miss attributes via N5, not X.F.

## FINAL — GROW1 (restated; supersedes D4's GROW1)

> **GROW1.** Grow runs only after best-fit failed: every free block is `< required_size =
> size + align − 1`; in particular the tail block ending at `lo` has `tail < required_size`.
> Grow commits `step` bytes and offers `[lo, hi)` (`usable = hi − lo` bytes), which
> left-coalesces with the tail into a single free block of EXACTLY `tail + usable` bytes ending
> at `hi` (right-coalescing is impossible at the frontier: all previously offered space ⊂
> `[0, lo)`). Grow proceeds only when `tail + usable ≥ required_size` — checked BEFORE any state
> change against the ACTUAL `required_size`, never against granule-rounded `needed` (false
> exhaustion) — else it panics as legitimate reserve exhaustion with zero state change.
> Therefore post-grow a free block ≥ `required_size` EXISTS, the retry's best-fit returns
> `Some`, and a retry-`None` is a GENUINE logic bug (panic, debug AND release).

---

# R3 (critic round 2 — two surgical amendments; BINDING, supersedes R2 where it differs)

Critic round-2 verdict: CHANGES-REQUESTED on exactly two items; C1 core logic, C2/C3
arithmetic, W1 induction, and both R2 deltas (g7b spike metric, 12-event recount) confirmed
correct and preserved as written.

## R3-1 (from R2-1) — supported-alignment bound + U10 re-spec

`allocate_aligned` aligns OFFSETS; pointer alignment = base alignment ∧ offset alignment.
Per-arm base guarantees: Windows reserve 64 KiB; unix mmap 4 KiB; fallback CACHE_LINE 64 B.
**The arena's documented + asserted supported alignment bound is `CACHE_LINE_SIZE` (64 B)** —
the honest cross-arm bound; production's max request is 32 (`SIMD_BUFFER_ALIGN`). Add to
`Arena` docs and a `debug_assert!(layout.align() <= CACHE_LINE_SIZE)` on `allocate_layout`
(compiled out in release; the bound is a contract, not a hot-path branch).
**U10 re-spec (critic's witness, align-independent false-exhaustion net):**
`with_reserve(100 KiB, 0)` + `Layout::from_size_align(102_300, 64)` → required_size = 102,363
≤ usable 102,400 while granule-rounded `needed` = 131,072 > usable — MUST SUCCEED, and the
returned pointer is genuinely aligned on all three arms.

## R3-2 (from R2-2) — g7b spike aggregation pinned (noise-robust)

The per-iteration spike (max − median over the 960 sub-batches) is aggregated as the
**MEDIAN across iterations**, and g7b follows the X.B multi-run methodology (≥3 bench runs;
the gate compares medians-of-medians). Rationale: a single OS preemption blip (~0.1 ms)
inside one boyko sub-batch would inflate a max-statistic from ≈0.27 ms to ≈0.37 ms →
false miss of the ≤0.1× gate; the median across iterations/runs absorbs it symmetrically.
Raw maxima are still reported untargeted.

## R3-3 (optional nits, adopted)

- Shared `required_size` helper (or coupling comment) between `allocate_aligned`
  (`free_mem_block.rs:206`) and `grow_then_retry` — drift-proofing the GROW1 comparand.
- Fallback backing sized to `os_reserve` (not `align_up(reserve, CACHE_LINE)`) for
  watermark/backing uniformity.
- Bevy tick/entity overhead figure: ~5.6% (32 B / 576 B per copied row), not ~3% —
  conservative direction, correct if cited.
