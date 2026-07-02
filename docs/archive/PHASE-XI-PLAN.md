> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Architecture: Phase X.I — ComponentPool Row-Capacity Growth

Companion to `docs/PHASE-XI-RESEARCH.md` (cited R-§1..§8; background NOT repeated). Branch `ecs`, tree `fb7cf1e`.

## Goal

Delete the engine's LAST hard ceiling: the fixed per-pool row count (`max_components = num_chunks × components_per_chunk`; 65,536 for the medium class), whose overflow today PANICS every deferred-command apply path (R-§1: `spawn_at_command.rs:174`, `spawn_batch_command.rs:337`, `migration_helpers.rs:245`, `:577`) and errors the direct paths.

- **Functionality**: one archetype can grow to millions of rows; the panic moves to a loud reserve-ceiling exhaustion (≥16× further out for every class, up to ×128); the apply-path panics become growth.
- **Performance**: growth is **O(1) in live rows — no memcpy, ever** (vs Bevy's per-table realloc+memcpy doubling); every reader keeps **zero changes** (R-§3 tolerance matrix, in-place extension); the hot mutation path additionally LOSES the vestigial per-op chunk-dirty `udiv + branch + store` (D7); archetype creation gets ~10-100× cheaper (the 2×256 KiB-per-pool tick memset and the arena alloc are deleted — D3 arithmetic).
- **Soundness**: pool base stays write-once ⇒ U10 (`refresh_all_columns` dead), the Phase-7 column cache, and the X.B `row_ptr` identity all survive verbatim.

## Context and constraints

- Affected: `memory/component_pool.rs` (core), `memory/chunk.rs` (DELETED), `core/archetype/archetype.rs` (`reserve_capacity` only), `component_pool_bundle.rs` (`is_full` semantics), `constants.rs`, `error.rs` (docs/Display), comment sweeps at the four apply sites; benches/tests per §Metrics.
- NOT affected (proof in D10): `Archetype::columns` / `Column` / the 8480-B const assert; `refresh_column`/`refresh_all_columns`; random access (`ecs_master.rs:1320`); typed iter / par_iter / legacy bump / `for_each_chunk` single-slice; `vm.rs` (consumed as-is); `arena.rs` (D8 — zero edits); `EntityMaster`/`InlandStore`.
- Invariants preserved: U6/U10 (write-once column base), X.B `row_ptr` identity pins, SIMD-A1 (32-B base), SEND10 (growth = apply-window `&mut`), ALLOC1 discipline, M-001 per-arm deallocator (carried by `VmReservation`), Archetype size 8480.
- Binding targets: §Metrics XI-B1…XI-B7.

## Key decisions

### D1: One `VmReservation` per pool, laid out `[data | added_ticks | changed_ticks]`, fixed sub-regions, independent frontiers, ONE row oracle
**What**: each `ComponentPool` owns one `VmReservation` (X.H-unified arms, zero new syscall code). Layout, all granule(64 KiB)-aligned, computed once at construction with checked math:

```text
data_off    = 0
data_len    = align_up(reserve_rows × stride, G)
added_off   = data_len;            tick_len = align_up(reserve_rows × 4, G)
changed_off = data_len + tick_len
os_len      = data_len + 2 × tick_len
```

`buffer = vm.base()` (data base, write-once), `added_base`/`changed_base` = `base + added_off/changed_off` (write-once, cast to `*mut UnsafeCell<Tick>` — `UnsafeCell<Tick>` is `repr(transparent)` over a 4-B `u32` with every bit pattern valid). The warm-path capacity oracle is a single scalar `committed_rows`; byte frontiers `data_committed`/`ticks_committed` are cold-path bookkeeping.
**Why**: in-place extension is the only reader-transparent shape (R-§3a); shared-arena in-place fails for ~(N−1)/N pools (R-§4); per-pool reservation is the only universal address-stable option (R-§4 ⇒). One reservation per pool (not three) = one syscall at creation, one provenance object, one Drop. Replaces BOTH the arena block AND the two STORE2 heap `Box<[UnsafeCell<Tick>]>` — tick growth is lockstep by construction (R-§ bottom-line 4), and tick zero-init becomes FREE (demand-zero pages ARE `Tick::ZERO` — `Tick` is `repr(transparent)` `u32`, `ZERO = Tick(0)`, verified `tick.rs:76-90`; pinned by a new transmute test, the X.G U-S1 pattern).
**Alternatives rejected**: relocate-on-grow (revives `refresh_all_columns`, O(N) memcpy, rewrites the row_ptr/U6/U10/STORE2 lattice and the X.B pins — the exact class X.F/X.G deleted; R-§4); segmented pools (breaks random access + single-slice `for_each_chunk` — R-§3b disqualifier); three reservations per pool (3× syscalls/VMAs/Drop surface for zero benefit); `try_extend_in_place` on the arena (works for ~1/N pools only, R-§4).
**Trade-off**: per-pool VA cost (D2 budget) and up to 6 VMAs/pool (each sub-region = committed prefix + PROT_NONE tail); a pool Drop now releases its own reservation (was: arena block leaked-until-arena-Drop) — strictly better.

### D2: Sizing — byte-targeted, row-clamped; ★ the legacy constructor maps `num_chunks × per_chunk → reserve_rows` (keeps every existing pin test green)
**What** (`constants.rs`):

```text
syscall arms (cfg as DEFAULT_ARENA_RESERVE):   POOL_TARGET_DATA_BYTES = 1 GiB,  POOL_MIN_ROWS = 65_536,  POOL_MAX_ROWS = 16_777_216 (2^24)
fallback arm (miri / wasm32 / 32-bit / exotic): POOL_TARGET_DATA_BYTES = 4 MiB,  POOL_MIN_ROWS = 256,     POOL_MAX_ROWS = 262_144  (2^18)   ★R1-3

reserve_rows(stride) = clamp(POOL_TARGET_DATA_BYTES / stride, POOL_MIN_ROWS, POOL_MAX_ROWS)
```

- `ComponentPool::with_default_sizes` uses the formula. **`ComponentPool::new(arena, id, num_chunks, components_per_chunk)` keeps its signature and is re-specified as the explicit-ceiling constructor: `reserve_rows = num_chunks × components_per_chunk`.**
- New row ceilings (syscall arms) vs today: 12 B → 16.7 M (×64 over 262,144 — row cap binds for stride < 64 B); 64 B → 16.7 M (×128); 192 B → 5.59 M (×85); 256 B → 4.19 M (×64); 1 KiB → 1.05 M (×32); 4 KiB → 262,144 (×8); stride > 16 KiB → MIN floor 65,536 (never below today's medium ceiling; ≥2× large's 32,768). `unit_index` stays far below `u32::MAX` (2^24 ≪ 2^32 — the `EntityInland.unit_index: u32` constraint, stated and const-asserted `POOL_MAX_ROWS < u32::MAX as usize`).
- **VA budget arithmetic (1000 archetypes × 3 pools = 3000 pools)**: per-pool VA = data ≤ ~1 GiB + ticks ≤ 2 × 64 MiB ⇒ ≤ ~1.13 GiB worst-common (64-B class); total ≤ ~3.4 TiB = **2.7% of the 128 TB user VA** (Windows and Linux 4-level alike). VMA/VAD count ≤ 6/pool ⇒ ≤ 18,000 vs Linux default `vm.max_map_count` 65,530 (3.6× headroom; noted as the one OS knob a pathological embedder could hit). Kernel VAD cost ~hundreds of B × 18 k ≈ single-digit MB. Typical worlds (≤ 20 pools): ≤ ~23 GiB VA — invisible.
- **Fallback ceilings honesty (★R1-3 re-derived at 4 MiB)**: `vm.rs`'s fallback `reserve` is an **eager `alloc_zeroed` of the full `os_len`** (commit is a no-op) — the wasm/Miri footprint is per-POOL-eager, a different scaling class than X.G's per-world 16 MiB, so the target shrinks 16 → 4 MiB. Ceilings: ≤16 B → 262,144 (= today's tiny, row-cap-bound); 32 B → 131,072 (= today's small); 64 B → 65,536 (= today's medium); 128 B+ SHRINK (192 B → 21,845 vs 32,768; 1 KiB → 4,096 vs 32,768) — documented, loud-panic; >4 MiB of one large-component pool on wasm/Miri is out of scope. Worst eager footprint/pool = 4 MiB data + 2 × 1 MiB ticks = **6 MiB** (16 B stride; most pools 2.25-4.5 MiB); the floor 256 caps the huge-stride blowup (256 × 64 KiB = 16 MiB, pathological ≥16 KiB strides only).
- **wasm demo arithmetic (★R1-3, the one shipping fallback target)**: boyko_demo = 3 archetypes / 13 pools (Particle/Boid: Position 8 B, Velocity 8 B, GpuInstance 16 B, tag 1 B; Ball adds Radius 4 B). Per-pool: 8 B → 4 MiB, 16 B → 6 MiB, 4 B → 3 MiB, 1 B → 2.25 MiB ⇒ pools ≈ **52 MiB** total. Plus X.G inland fallback 16 MiB + the now-dead fallback arena — whose default SHRINKS 64 MiB → 1 MiB in W3 (zero clients post-W2; D8) ⇒ world ≈ **69 MiB vs ≈ 86 MiB today** (64 arena + 6.5 tick Boxes + 16 inland) — strictly better. Demo archetype ceiling on wasm stays **262,144 rows — exactly today's** (all demo strides ≤ 16 B; the 100k default and any count that works today keeps working).
**Why this shape**: pure fixed-bytes makes tick regions explode for sub-16-B strides (rows = GiB/stride); pure fixed-rows explodes data VA for KB-strides. Clamp gives both bounds: tick regions ≤ 64 MiB each ALWAYS, data ≤ ~1 GiB except the MIN-floor tail (VA-only). The 1 GiB target aligns with X.G's 67 M-entity inland ceiling (a 16.7 M-row archetype of 64-B components = 1 GiB payload).
**★ Supersedes the research assumption that the None-at-capacity pins need re-spec**: `drop_fn.rs:426` (pool `1×cap`), the two in-file proptests (`component_pool.rs:1864`, `:2067`, pools `4×64` driving to `capacity()`), the X.B dense-equivalence and drop-count tests, and the bench `component_pool_dense.rs:69` ALL construct via `ComponentPool::new(…, n, m)` — under the mapping their ceiling, `capacity()` value, and None-at-ceiling behavior are bit-identical. **Zero test re-specs; the mapping IS the test knob** (works from integration tests and benches too, where `#[cfg(test)]` lib knobs are unreachable — the X.G `with_reserve_bytes` pattern would not have been).
**Trade-off**: the parameter names `num_chunks`/`components_per_chunk` become historical (chunks are deleted, D7) — doc-comment states the mapping; rename filed for X.J. **★R1-9 (binding doc sentence)**: the legacy constructor produces `reserve_rows = n × m` EXACTLY — it deliberately BYPASSES the `POOL_MIN_ROWS`/`POOL_MAX_ROWS` clamp (the entire pin-test ledger depends on exact small ceilings: `make_dc_pool(…, 1, cap)`, proptests `4×64`, dense `1×cap`, X.B `4×4`/`1×16`); routing it through the clamp is a ledger-wide breakage, stated in the constructor doc.

### D3: Eager reserve, ZERO initial commit — lazy reservation is REJECTED (with arithmetic)
**What**: `ComponentPool::new` performs `VmReservation::reserve(os_len)` (one syscall, no commit charge) and computes the three write-once base pointers. No commit, no tick allocation, no zeroing. First `add`/`reserve_capacity` takes the cold grow path.
**Why eager reserve (vs the X.G lazy-`Option<VmReservation>` pattern)**: `Archetype::refresh_column` captures `pool.buffer_ptr()` into `columns[c].ptr` **at pool creation** and U10 forbids any later refresh (`archetype.rs:346-392`) — the base must be FINAL before `refresh_column` runs. A lazy base would dangle inside a live `Column` and require reviving `refresh_all_columns` = the disqualified relocation class. The X.G constructor-gate lesson does not bite here: that gate was `EcsMaster::new` (which creates ZERO pools — unaffected); the relevant gate is archetype creation, where the arithmetic is a NET WIN without any laziness:
- **Today** per 3-pool 192-B archetype: 3 × arena `allocate_layout` (~0.1-1 µs, plus amortized arena slab commits) + 3 × 128-`Chunk` Vec (3 KiB) + **6 × `Box<[UnsafeCell<Tick>]>` of 256 KiB, allocated AND zero-written** (≈ 64 page faults + memset each) ≈ **~150-400 µs**, dominated by the tick memsets (the X.F "N5" term, ~80% of the measured 0.9-2.2 ms first-batch spike class).
- **X.I**: 3 × `reserve` (~0.3-0.8 µs each, X.F B2-class) + 3 small Vec/SparseMap ops ≈ **~2-5 µs** — creation gets ~50-100× cheaper; the deferred cost is the first growth event (D5: ≤3 commit syscalls ≈ 2-10 µs) plus the SAME demand-zero faults both designs pay, now distributed across spawn batches instead of front-loaded (g7 batches 0-15 implication: the spike class SHRINKS — XI-B5 prediction).
**Why commit zero (vs one row-slab)**: an eager first slab adds 3-9 syscalls per archetype creation for zero measured benefit (the first spawn batch is already an apply-window µs-scale event); X.F D2 precedent. Empty archetypes (created by migrations' edge transitions, marker combinations) stay at 0 resident bytes.
**Trade-off**: first spawn into a fresh archetype carries ~3 commit syscalls (bounded, XI-B4-measured); a fresh pool's tick pages fault at first `fill_ticks` write instead of at creation (same page count, better placement).

### D4: Growth policy — data-region byte doubling [64 KiB … 64 MiB], request-dominant; ticks lockstep BY ROWS; `grow_rows` returns `bool` (GROW1-XI is a proof, not a check)
**What** (`constants.rs`: `POOL_MIN_SLAB = 64 KiB` (= granule), `POOL_MAX_SLAB = 64 MiB`):

```text
grow_rows(n) -> bool:                              #[cold] #[inline(never)]
  if n > reserve_rows           -> return false    // ceiling; ZERO state change
  if n <= committed_rows        -> return true     // ★R1-1 idempotent no-op; ZERO syscalls, ZERO state change
  needed = align_up(n × stride, G)                 // checked; ≤ data_len (granule chain below)
  step   = clamp(data_committed, MIN_SLAB, MAX_SLAB)
             .max(needed.saturating_sub(data_committed))  // sub provably > 0 (proof 0); saturating = belt
  new_d  = min(data_committed + step, data_len)    // ≥ needed (proof below); > data_committed strictly
  vm.commit(data_committed, new_d)                 // panics only on genuine OS OOM
  rows   = min(new_d / stride, reserve_rows)       // ≥ n (proof below)
  t_new  = align_up(rows × 4, G)                   // ≤ tick_len (granule chain)
  if t_new > ticks_committed:
      vm.commit(added_off + ticks_committed, added_off + t_new)
      vm.commit(changed_off + ticks_committed, changed_off + t_new)
      ticks_committed = t_new                      // ★R1/Q6: frontier fields are written only AFTER
  data_committed = new_d; committed_rows = rows;   // the commits they describe succeed (panic-coherent)
  return true
```

**GROW1-XI (sufficiency proof — no free list, no fragmentation, pure frontier; the X.F C1 retry machinery is absent BY CONSTRUCTION):**
0. **Idempotence (★R1-1)**: `n ≤ committed_rows` returns `true` with zero syscalls and zero state change — `grow_rows` is total over already-satisfied requests, so the D5 funnels may call it unconditionally. Corollaries: (a) past both guards `n > committed_rows`, and since `committed_rows = min(⌊data_committed/stride⌋, reserve_rows)` with the clamped case excluded by the ceiling check, `needed = align_up(n×stride, G) > data_committed` — the `saturating_sub` never actually saturates (debug_assert pins it); (b) `vm.commit` is never called with `new == old`: `data_committed == data_len ⇒ committed_rows = reserve_rows` (granule padding only adds rows, `⌊data_len/stride⌋ ≥ reserve_rows`) ⇒ every `n` past the ceiling check hits the no-op arm; tick commits are guarded by `t_new > ticks_committed` explicitly. The vm.rs `debug_assert!(new > old)` is therefore unreachable from this caller.
1. Ceiling first: `n ≤ reserve_rows ⇒ n×stride ≤ reserve_rows×stride ≤ data_len`, and `data_len` is a granule multiple ⇒ `align_up(n×stride, G) ≤ data_len` (the X.G granule chain verbatim) — `needed` never overruns the sub-region.
2. `new_d ≥ needed`: `step ≥ needed − data_committed` by the `max`; the `min(data_len)` clamp cannot bite below `needed` since `needed ≤ data_len`.
3. `rows ≥ n`: `new_d ≥ needed ≥ n×stride ⇒ ⌊new_d/stride⌋ ≥ n`, and `n ≤ reserve_rows` ⇒ the `min` keeps `rows ≥ n`.
4. The `min(…, reserve_rows)` on `rows` is load-bearing: granule padding can make `⌊data_len/stride⌋ > reserve_rows`, and tick regions are sized for `reserve_rows` only — uncapped rows would overrun them (debug_assert the chain).
5. Tick bound: `rows ≤ reserve_rows ⇒ align_up(rows×4, G) ≤ align_up(reserve_rows×4, G) = tick_len`.
Post-condition `committed_rows ≥ n` — callers never retry. Each event = 1-3 syscalls (tick commits saturate ~48× less often than data commits for 192-B strides: one data granule = 341 rows = 1,365 tick bytes). Lifetime events to fill 1 GiB: ~11 doublings + ~15 max-steps ≈ 26/pool.
**Why MIN = 64 KiB** (vs arena's 2 MiB / inland's 256 KiB): sparse-archetype resident cost. A 1-row archetype commits 3 × 64 KiB = 192 KiB/pool ⇒ 576 KiB/archetype; 1000 sparse archetypes ⇒ 576 MiB commit charge (vs **36 GiB** if pools were fully committed at today's medium geometry — the reason the 1000-archetype bench was impossible pre-X.F). MIN=granule is the floor; doubling reaches any real population in ≤ a dozen µs-scale events. **Why MAX = 64 MiB**: matches `ARENA_MAX_SLAB` overshoot honesty; one max-step = 64 MiB ≈ ≤50 µs (X.F B4 envelope).
**Re-entrancy (SEND10, hooks' deferred drains spawning during apply)**: growth is plain `&mut self` field mutation — **no RAII guard, no cached `NonNull` twin, no TLS** (the 14a-F2/9.3c forbidden classes are structurally absent); frontiers are monotonic; nested grows compose (inner grow advances the frontier, outer code re-reads fields). Because the base never moves, an outer migration frame's `&[u8]` slices into a SOURCE pool remain valid even if a nested drain grows ANY pool — the property realloc designs cannot offer. Pinned by I-3.

### D5: Growth funnels — `add`/`add_typed` warm branch (ONE compare, same branch count as today) + `reserve_capacity` two-phase; `grow` failure maps to `None`/`Err`, panic only for genuine OS OOM
**What**:
- `add`/`add_typed` (★R1-2 — BINDING single-compare shape, the Algorithms-table version is the spec): `if self.len >= self.committed_rows { if !self.grow_rows(self.len + 1) { return None; } }` — ONE warm compare (not-taken), identical hot shape to today's single `len >= max_components`; the ceiling check lives INSIDE the cold `grow_rows` (its first guard), so `None` still means reserve-ceiling (≥16× further out). No explicit warm ceiling compare — it would be redundant with `grow_rows`'s first guard and would break the table's −1-branch accounting.
- `Archetype::reserve_capacity(n)` (the single batch/migration guard funnel, R-§2) becomes the grow funnel, preserving its documented "on `Err` the archetype is unchanged" two-phase contract:
  - Phase A (read-only): every pool `len + n ≤ reserve_rows`, else `Err(ArchetypePoolCapacityExceeded)` — no mutation.
  - Phase B: every pool `grow_rows(len + n)` via `pools_iter_mut` — cannot return false (phase A proved the ceiling), can only panic on OS commit failure. **★R1-1**: legal to call unconditionally ONLY because of `grow_rows`'s idempotent no-op arm (GROW1-XI proof 0) — the common case (`reserve_capacity(1)` on every `SpawnAtCommand::apply`/migration with capacity already committed) is P warm compares, ZERO syscalls, ZERO state change.
- `ComponentPool::is_full()` → `len >= reserve_rows`; `can_reserve(n)` → `checked_add ≤ reserve_rows`; `can_push_entity_components` therefore becomes a ceiling pre-check and `push_entity_components → add` grows inline — `create_entity`'s C-009 two-phase commit survives with ceiling semantics.
- **Failure-surface rewrite (the 4 panics + 2 errors)**: `SpawnAtCommand::apply:174` and `SpawnBatchCommand::apply:337` `.expect`s STAY (they now fire only at the reserve ceiling; messages re-worded to name growth + ceiling); `migration_helpers.rs:245` ditto; `:577 assert!(pushed)` ditto via `can_push`; `EcsMaster` `ArchetypeRejectedEntity` (`ecs_master.rs:680-691`) now unreachable for capacity (remains for signature mismatch); direct `spawn_batch` `Err` = ceiling. **Remaining error**: `ArchetypePoolCapacityExceeded` re-documented (field `pool_capacity` keeps its name; doc + Display text re-worded to "reserve ceiling (rows)"). **Remaining panics**: `VmReservation::commit` assert (commit-charge/overcommit exhaustion — genuine OOM, world poisoned, documented like the `Component::drop` panic policy); mid-bundle OS-OOM leaves pools desynced under a panic — accepted (panic = discard world).
**Why**: research bottom-line 5 names exactly these funnels; growth must live in `add` too (not only `reserve_capacity`) or `create_entity`'s per-entity path still ceilings at `committed_rows`.

### D6: `max_components` → `reserve_rows`; accessor semantics mirror X.F's `Arena`
| Accessor | Post-X.I |
|---|---|
| `capacity()` | `reserve_rows` — the ceiling the exhaustion is measured against (X.F precedent: `Arena::capacity()` = reserve). All existing capacity-driving tests stay green via D2's constructor mapping |
| NEW `committed_rows()` | the frontier (diagnostics/tests; mirror of `Arena::committed()`) |
| `remaining_capacity()` | `reserve_rows − len` |
| `len_for_reserve()` | `(len, reserve_rows)` |
| `is_full()` / `can_reserve(n)` | ceiling-derived (D5) |
| `write_at_unchecked_initialized` / `commit_units` / `fill_ticks` debug_asserts | `< committed_rows` (callers pre-grew via `reserve_capacity`) |

### D7: DELETE the Chunk machinery outright (in scope) — it is a hot-path TAX, and lockstep growth would make it WORSE
**What**: delete `memory/chunk.rs`, the `chunks: Vec<Chunk>` field, `chunks()`/`chunks_count()` accessors, every `mark_dirty` site, and `components_per_chunk` (becomes read-by-nobody); `commit_units` collapses to the guarded `len += count`; the four class constants + `DEFAULT_CHUNKS_PER_POOL` + dead `DEFAULT_COMPONENTS_PER_CHUNK` are deleted, the three `*_THRESHOLD`s with them (R-§5: their only readers are `with_default_sizes`/`get_optimal_chunk_capacity`, both rewritten by D2; the developer wave re-greps before deletion).
**Why in scope, not a separate cleanup**: (a) keeping it FORCES growing a parallel `Vec<Chunk>` on the grow path — a heap realloc inside the growth funnel, re-introducing the exact parallel-bookkeeping class X.B deleted, plus amending the `commit_units` "chunks NEVER mutated" SAFETY (R-§1) — strictly more diff than deletion; (b) it is not dead weight but a hot-path cost: every `add`/`add_typed`/`swap_remove`/`set_component`/`get_raw_mut`/`drop_at`/`write_at` pays `idx / self.components_per_chunk` — a **runtime-divisor `udiv` (~20-40 cycles)** + bounds-checked `get_mut` + store, written-and-never-read (verified: zero callers of `is_dirty`/`clear_dirty_flag`/`chunks()`/`chunks_count()` in the entire workspace; `Chunk` is referenced only by `component_pool.rs:15,158,697`). Deleting is perf-positive on the spawn suites (XI-B1 predicts flat-to-better).
**Trade-off**: `ComponentPool::new`'s first two parameter names become historical (D2); query-side "chunking" is untouched (chunk_iter batches by row ranges, not these objects — R-§1).

### D8: The shared Arena — option (b): keep as-is, unused in production; retirement filed as X.J
**What**: `ComponentPool::new` stops calling `arena.allocate_layout` but KEEPS the `&Arena` parameter (ignored, `_arena`); the pool's dead `arena: *const Arena` and `buffer_capacity_bytes` fields are deleted (both already `#[allow(dead_code)]`, struct layout unpinned); `Archetype.arena` field and the whole parameter chain stay byte-identical — the Archetype 8480 assert never trips. `EcsMaster` still constructs the Arena (X.F lazy: reserve-only ~0.3-0.8 µs, zero commit; with NO allocations it never commits a single slab — a pure 4 GiB PROT_NONE reservation per world).
**Why (b) over (a) retire-now**: deleting `arena.rs` + `free_mem_block.rs` + the field/parameter plumbing + `with_arena_reserve` + the X.F bench/test suite mid-phase couples two risk surfaces and re-opens landed X.F gates inside the same diff that rewrites the pool — the X.G D1 blast-radius argument verbatim. **Why not (c) fallback-pools-on-arena**: it forks `ComponentPool` internals per cfg arm, so Miri would exercise DIFFERENT control flow than native on the most-audited file in the engine — exactly how the 14a-class soundness bugs hide; a unified path means every existing Miri churn suite traverses the real grow code (the X.G coverage win).
**X.J (filed)**: delete arena.rs/free_mem_block.rs, the parameter chain, `with_arena_reserve`; rename `new`'s legacy params; re-run constructor gates.
**Trade-off (★R1-3 corrected per arm)**: on syscall arms — one dead reservation syscall per world (~0.5 µs inside the existing ≤7.5 µs gate — XI-B3 controls it; pure PROT_NONE VA, zero commit). On the FALLBACK arm the dead arena is an eager 64 MiB `alloc` — therefore `DEFAULT_ARENA_RESERVE`'s fallback value SHRINKS 64 MiB → 1 MiB in W3 (the arena has zero clients post-W2; X.J deletes it outright). W3 re-runs the Miri arena suites under the shrunk default (explicit-capacity tests unaffected; any default-construction test allocating > 1 MiB would fail loudly at the X.F exhaustion path — none is expected). Dead code carried one phase.

### D9: Fallback arm (Miri/wasm) — unified code path with small reserves; the Miri strategy is the D2 constructor mapping
**What**: same code on every arm; the fallback differs only in constants (D2) and in `VmReservation`'s own arm behavior (eager `alloc_zeroed`, commit = no-op — vm.rs as-built). Per-pool eager cost ≤ 6 MiB worst at the ★R1-3 constants (D2-mapped test pools: KBs; `with_default_sizes` pools under Miri: 2.25-6 MiB each — W5 records the Miri suite RSS/wall-time delta, expected net-fine: the per-pool zeroed alloc replaces the shared-arena carve + two 256 KiB per-element-initialized tick Boxes). Growth BOOKKEEPING (frontier math, lockstep, ceiling, `committed_rows`) runs identically under Miri.
**Zero-fill**: use `VmReservation::reserve` (zeroed contract), NOT `reserve_unzeroed`: tick sub-regions rely on never-written-reads-zero (the CURRENT pre-X.I STORE10 text "slots above len stay `Tick::ZERO`" is the superseded absolute form — W2 re-words it to the ★R1-4 never-written form; `check_ticks` scans only `[0, count())` — verified `check_ticks.rs:79-84` — so zero-fill is the documented-invariant belt, not a hot requirement; the data region is write-before-read and gets zeroing free on syscall arms / cheap under Miri).
**Miri test strategy**: in-file growth tests construct small-ceiling pools via `ComponentPool::new(…, n, m)` (D2 mapping — no cfg(test) knob needed, also reachable from `tests/` and benches); M-XI drives multi-slab growth + cross-boundary writes/reads + drop-count under Tree Borrows; all existing churn suites (8cd/14a/14b/19) traverse the new grow path implicitly on every world.
**Rejected**: fallback-keeps-today's-fixed-behavior — that is D8's rejected (c) in disguise (cfg fork + Miri walks dead paths).

### D10: Reader side — NOTHING changes (the proof)
- **Random access** (`ecs_master.rs:1320`): reads `EntityInland` → `columns[c].ptr.add(unit_index × stride)` — touches the Archetype slab only, **never loads a `ComponentPool` field**. `columns[c].ptr` = data base = write-once. Gate: asm **byte-identical** (XI-B1).
- **Typed iter / par_iter / legacy bump / filter fetches**: capture column base + `entity_count` per archetype per `set_table_*` (R-§3); bases write-once; `entity_count` re-read per use; growth runs only inside `&mut` windows where no `Fetch` is live. Per-row inner loops: zero source/asm change. The per-boundary cold code reads pool fields (`buffer_ptr`, tick ptrs, layout) → **displacement-only deltas** permitted there.
- **`for_each_chunk`**: whole-archetype single slice `[base, entity_count)` — contiguity preserved by in-place extension; the demo's zero-copy GPU upload contract holds (pinned by I-4).
- **Tick-pointer escapes** (`added_ticks_ptr`/`changed_ticks_ptr` → `tick_column_base` → fetches): pointers become `base + tick_off` — write-once, valid for the pool's lifetime; the "Box never reallocates" doc promise is renegotiated to the STRONGER "vm-reservation-stable" wording.
- **Alignment**: every arm's reservation base is ≥4096-aligned (VirtualAlloc 64 KiB / mmap 4 KiB / fallback `Layout` align 4096 — vm.rs:171) ⇒ SIMD-A1 (32) holds trivially; the X.A debug_assert stays. NEW loud assert at construction: `element_align ≤ 4096` — strictly wider than today's arena bound of 64 (X.F R3-1).
- **Known residual (out of scope, honest)**: `Archetype::entity_ids: Vec<EntityId>` remains the engine's last realloc-doubling container (8 B/row; worst single memcpy at 1 M rows ≈ 8 MB ≈ ~1-2 ms). Filed as a follow-up candidate (X.K: `entity_ids` onto the InlandStore pattern); g8's spike gate is set with this term included.

## Data structures

```rust
// memory/component_pool.rs — not #[repr(C)]-pinned (no external offset contract; the
// hot READ paths never load pool fields — D10). Field ORDER groups the warm trio.
pub struct ComponentPool {
    buffer: NonNull<u8>,            // data sub-region base == vm.base(); WRITE-ONCE (U6 twin)
    len: usize,                     // live rows [0, len) initialized; THE liveness oracle
    committed_rows: usize,          // warm-path capacity comparator (single cmp in add)
    reserve_rows: usize,            // the ceiling (capacity()); immutable after new
    component_layout: Layout,       // stride/align (unchanged)
    data_committed: usize,          // bytes, granule-aligned, monotonic (cold)
    ticks_committed: usize,         // bytes per tick region, granule-aligned (cold)
    added_base: NonNull<UnsafeCell<Tick>>,   // vm.base()+data_len; WRITE-ONCE
    changed_base: NonNull<UnsafeCell<Tick>>, // vm.base()+data_len+tick_len; WRITE-ONCE
    component_id: usize,
    drop_fn: Option<DropFn>,
    component_type_id: TypeId,
    vm: VmReservation,              // declared last; Drop::drop body (drop_fn loop over
                                    // rows [0,len)) runs BEFORE field drops ⇒ release-after-use
}
// DELETED: arena (*const Arena, dead), buffer_capacity_bytes (dead),
//          chunks: Vec<Chunk>, components_per_chunk, max_components (renamed),
//          added_ticks / changed_ticks Boxes (STORE2 — replaced by sub-regions).
// DELETED FILE: memory/chunk.rs.
```

## Public API (deltas only)

```rust
impl ComponentPool {
    pub fn new(_arena: &Arena, component_id: usize,
               num_chunks: usize, components_per_chunk: usize) -> Self; // SIGNATURE KEPT;
        // re-spec: reserve_rows = num_chunks * components_per_chunk (D2 mapping)
    pub fn with_default_sizes(_arena: &Arena, component_id: usize) -> Self; // D2 formula
    pub fn capacity(&self) -> usize;          // = reserve_rows (ceiling)
    pub fn committed_rows(&self) -> usize;    // NEW (diagnostics)
    // REMOVED: chunks(), chunks_count()  (zero external callers — verified)
}
// Archetype::reserve_capacity(&mut self, n) -> EcsResult<()> — signature unchanged; now grows.
// EcsError::ArchetypePoolCapacityExceeded — variant/fields unchanged; docs + Display re-worded.
```

## Algorithms for critical paths

| Path | Steps | Big-O | Branching |
|---|---|---|---|
| `add`/`add_typed` warm | 1 cmp (`len >= committed_rows`, not-taken) + write + `len += 1` | O(1) | **−1 udiv, −1 branch, −1 store vs today** (chunk dirty deleted) |
| `grow_rows` (cold) | D4 box: ≤3 syscalls + 6 field writes | **O(1) in live rows; 0 bytes copied; 0 bytes written** | `#[cold] #[inline(never)]` |
| `reserve_capacity(n)` | phase A: P cmps; phase B: ≤P cold grows | O(P) pools | warm loop, cold grow |
| `commit_units` | guarded `len += count` (chunk loop deleted) | O(1) | −loop |
| random access / iter / for_each_chunk | **unchanged** (D10) | — | — |

Frequency: ~26 growth events per pool LIFETIME at default sizing.

## Multithreading model

Unchanged SEND10 contract: all growth/mutation reachable only through `&mut` paths (owner direct API, apply window — SCH7: zero workers in flight); worker `&self` reads race nothing because no `&mut` exists while workers run; growth syscalls are not global-allocator calls so the ALLOC1 TLS guard does not see them — the `&mut` exclusivity IS the guard (SEND10 bullet 3, "(realized by Phase X.I)"). `len`/`committed_rows` stay plain `usize` (legal via exclusivity, NOT via address stability — the X.G D7 wording discipline). No new atomics; `unsafe impl Send/Sync for ComponentPool` stays with SEND10 text updated.

## Soundness

1. **Address stability**: `buffer`/`added_base`/`changed_base` write-once; growth changes protection/charge on fresh ranges of the SAME reservation; previously returned pointers (incl. `Column.ptr` and Fetch tick bases) stay valid. `refresh_all_columns` stays dead.
2. **Provenance**: all three bases derive from `vm.base()` (one allocated object per pool on every arm); `row_ptr` = one `add` from `buffer` — SAFETY text updated to cite the pool's own reservation.
3. **Initialization (rows)**: rows `[0, len)` initialized (unchanged); `[len, committed_rows)` data is uninitialized-but-committed (never read — `len` is the oracle); `[committed, reserve)` is PROT_NONE (stray touch faults loudly = free tripwire).
4. **Tick invariant J-XI (★R1-4 — never-written form)**: every **never-written** tick slot in `[0, committed_rows)` reads `Tick::ZERO` (OS zero-fill / `alloc_zeroed`, vm contract). Slots VACATED by `pop`/`swap_remove` (old `last_index`, now `== len`) MAY hold a stale live tick — nothing zeroes on removal, and that is fine: nothing reads above `len` (`check_ticks` scans `[0, count())`; fetches index `< entity_count`) and every re-add re-stamps before any read (`create_entity`/`fill_ticks`/`write_*_tick` cover `[0, len)`). Do NOT debug_assert all-zero-above-len — it is FALSE after any churn. `Tick::ZERO` is all-zero 4 B `repr(transparent)` — pinned by transmute test. Write-before-read is the load-bearing property; J-XI (never-written form) is the belt.
5. **New/changed unsafe inventory**: row_ptr re-cite; S-TICKBASE (deriving tick bases via `byte_add` from `vm.base()` — in-bounds by D1 layout math, align 4 from granule-aligned offsets); tick asserts re-based to `committed_rows`; `commit_units` SAFETY — chunk clause DELETED; Drop loop unchanged (rows `[0,len)` ⊆ committed RW). **★R1-8**: data + both tick regions now share ONE allocated object (today: three separate allocations) — the W2 SAFETY sweep states this explicitly wherever current texts lean on "separate allocation" intuition (S-TICKBASE is the hook; the `write_at`-class "disjoint allocations" clauses stay true — caller bytes vs pool). NET unsafe count DOWN (chunk `get_unchecked_mut` loop deleted).
6. **Edge cases**: n=0 guards kept; `len == reserve_rows` → None no-state-change; ZST rejected (unchanged); `element_align > 4096` → loud construction panic (new); **★R1-5: `reserve_rows == 0` → loud pool-level construction assert naming the constructor** (else `VmReservation::reserve(0)` panics with a vm-internals message; reachable via the D2 mapping with `n × m == 0`); `reserve_rows × stride` overflow checked at construction; ceiling exhaustion → `false`/`Err` ZERO state change; `grow_rows(n ≤ committed_rows)` → idempotent no-op (★R1-1); OS commit failure mid-bundle → panic, world poisoned (documented); pool Drop with partial commit (V-DROP releases full reservation); empty-pool Drop. vm declared last ⇒ drop_fn loop runs before release.
7. **Miri**: unified path ⇒ every churn suite traverses construction+growth under TB; M-XI dedicated; syscall arms covered natively; **the W5 Linux residual widens to the pool consumer** — results doc carries it.

## Integration

| File | Change |
|---|---|
| `memory/component_pool.rs` | core rewrite per D1-D7; comment sweeps (SEND10 b.3, STORE2/3/10, row_ptr SAFETY, tick lifetime wording) |
| `memory/chunk.rs` | **DELETED** (+ mod.rs dereg) |
| `constants.rs` | + `POOL_TARGET_DATA_BYTES` (cfg pair), `POOL_MIN_ROWS`/`POOL_MAX_ROWS` (cfg pairs), `POOL_MIN_SLAB`, `POOL_MAX_SLAB`; − `DEFAULT_CHUNKS_PER_POOL`, `DEFAULT_COMPONENTS_PER_CHUNK`, 4 class consts, 3 thresholds (post-grep); `DEFAULT_ARENA_RESERVE` fallback arm 64 MiB → 1 MiB (★R1-3, dead post-W2); doc note: Linux `vm.max_map_count` is the one OS knob a pathological embedder could approach (≤6 VMAs/pool, 3.6× headroom at 3000 pools) |
| `core/archetype/archetype.rs` | `reserve_capacity` two-phase grow (D5); doc updates. Struct/columns/8480 UNTOUCHED |
| `component_pool_bundle.rs` | `can_push_entity_components` doc (ceiling semantics) |
| `error.rs` | `ArchetypePoolCapacityExceeded` doc + Display re-word |
| `spawn_at_command.rs:174`, `spawn_batch_command.rs:337`, `migration_helpers.rs:245,577`, `ecs_master.rs:680` | message/comment re-words ONLY |
| benches | `component_pool_dense.rs` unchanged-compile; ceiling workarounds un-wound (`bundle_static_cache.rs:239-254`, `query_dsl.rs:441-442`); NEW `archetype_create.rs`, `pool_grow_event`, `growth_crossing_pool.rs` (g8) |
| `arena.rs`, `vm.rs`, `inland_store.rs`, query/iter/fetch files | **ZERO code changes** |

## Implementation plan (waves)

1. **W0 — baselines (orchestrator, BEFORE any edit)**: release asm of random_access + query-iter bench fns at HEAD (`D:\tmp\xi_baseline_*.s`); spawn-suite criterion baselines (multi-run protocol).
2. **W1 — constants + pure math**: D2/D4 constants; `reserve_rows(stride)` + layout pure fns + table tests (U-P1); Tick::ZERO transmute pin; `POOL_MAX_ROWS < u32::MAX` const assert.
3. **W2 — pool core**: D1 struct swap, constructor rewrite (eager reserve, zero commit, D2 mapping), `grow_rows`, accessor re-derivations (D6), tick accessors re-based, row_ptr SAFETY re-cite, chunk machinery + chunk.rs DELETED, dead fields deleted. In-file pool tests green unchanged.
4. **W3 — funnels**: add/add_typed grow branch (single-compare shape ★R1-2); reserve_capacity two-phase; is_full/can_push ceiling semantics; error.rs + 6 message sites; SEND10/STORE sweeps; `DEFAULT_ARENA_RESERVE` fallback shrink + Miri arena-suite re-run (★R1-3). Full suite green; clippy clean.
5. **W4 — new tests**: U-P2…U-P8, I-1…I-5, M-XI; audit-greps (no stale max_components semantics, no Chunk refs, constants dead-reader check).
6. **W5 — Miri + suites**: full debug+release; Miri M-XI + churn controls; record Miri suite RSS/wall-time delta vs HEAD (★R1-3); Linux residual entry.
7. **W6 — gates + docs**: asm diffs vs W0; spawn A/B; archetype_create + pool_grow_event; g7/g7b re-run; g8 vs Bevy; PHASE-XI-RESULTS.md; internal-doc sync.

## Metrics and validation

### Binding gates
- **XI-B1 — 0%-gate**: (a) random_access lookup fns **asm byte-identical** (pool fields not on the path — any delta FAILS); (b) query-iter inner loop instruction-identical; per-boundary code displacement-only; (c) spawn suites within ±2% multi-run — predict flat-to-better (deleted udiv+branch+store per mutation). **★R1-6 (c) protocol**: two bench populations — (i) ±2%-gated UNCHANGED benches (`commands_spawn` suites, `spawn_batch`, `component_pool_dense`, g4/g5 — sources untouched); (ii) RE-BASELINED benches whose source is rewritten by this phase (the ceiling-workaround unwinds: `bundle_static_cache.rs:239-254`, `query_dsl.rs:441-442`) — W0-incomparable, reported as new numbers with the unwind noted. Known confound for (i): fresh-world-per-iteration spawn benches move first-fill commit syscalls INTO the timed region (~2-5 commits/pool, XI-B4-bounded µs-scale) — on a miss, decompose per XI-B4 (commit-event count × bounded cost) FIRST and attribute before any verdict (the XI-B6 discipline applied to (c)).
- **XI-B2 — archetype creation (NEW)**: `archetype_create/3x192B` — today est. 150-400 µs; **gate ≤ 25 µs, predict 2-5 µs**.
- **XI-B3**: `EcsMaster::new` ≤ 7.5 µs (no change expected).
- **XI-B4 — growth event**: `pool_grow_event/{64KiB, 2MiB, 64MiB}` ≤ {10, 10, 50} µs (incl. lockstep tick commits).
- **XI-B5 — g7/g7b re-run**: (a) total **≥ 1.5×** vs Bevy (predict ~1.90-1.94×: deletes 48 × 512 KiB tick memsets ≈ 2.5-4 ms + 12 arena commits; adds ~240 pool commits + 144 reserves ≈ 0.5 ms); (b) **attribution (binding)**: the batches-0-15 pool-creation spike class SHRINKS (no archetype-creation mode above the payload-fault floor); (c) composite spike honestly reported: predict 0.06-0.11× (≤0.1× NOT promised — floor = payload faults + the `entity_ids` residual).
- **XI-B6 — g8 growth-crossing (NEW headline — impossible pre-X.I)**: ONE archetype, 3×192 B, **1,000,000 entities** (15× past the old 65,536 ceiling) in 100 × 10 k sub-batches, cold worlds, Bevy NOT pre-reserved, iter_custom + per-batch durations. Model: payload 576 MB/6 GB/s ≈ 96 ms both; bookkeeping ≈ 35/26 ms; Bevy doubling memcpy ≈ 600 MB ≈ 100-150 ms; boyko growth ≈ ~80 commits ≤ 1 ms. Totals ≈ 131 vs 222-272 ms. **Targets: total ≥ 1.5× (model 1.7-2.1×); worst-batch spike (max−median, median-of-iterations) ≤ 0.1×** (Bevy's final doubling ≈ 288 MB ≈ 50-70 ms one batch; boyko worst = one 64 MiB commit ≤ 50 µs + ~1-2 ms entity_ids residual ⇒ model 0.01-0.05×). Miss ⇒ decompose per model FIRST (commit events XI-B4-bounded, CANNOT explain).
- **XI-B7 — suites**: full debug+release green; clippy; Miri M-XI + 4 churn controls.

### Test matrix
- **Unit**: U-P1 sizing/slab table (class ceilings, clamp edges 64 B/16 KiB, granule alignment, request-dominant, doubling, MAX clamp, fallback constants, **zero-ceiling loud assert ★R1-5**); U-P2 **address-stability witness** (buffer/row-0/tick bases + values across ≥3 slab growths); U-P3 ceiling exhaustion (small pool via D2 mapping → None/Err, ZERO state change); U-P4 tick lockstep + J-XI (never-written tick slots read ZERO at slab boundaries ±1); U-P5 drop-count-exact across a growth boundary; U-P6 Tick::ZERO transmute + POOL_MAX_ROWS const; U-P7 X.B identity pins re-run unchanged; **U-P8 idempotence (★R1-1)**: `grow_rows(n ≤ committed_rows)` returns true with frontier/`committed_rows` state EXACTLY unchanged, and `Archetype::reserve_capacity` called twice with satisfied capacity commits nothing the second time (state-equality witness).
- **Pinned green UNCHANGED** (D2 mapping ledger): drop_fn.rs:426, both proptests, dense-equivalence, drop-count, component_pool_dense compile, Archetype 8480, U10 dead-code, F4 witness.
- **Integration (`tests/pool_growth.rs`)**: I-1 cross-ceiling spawn (one medium archetype → 100 k rows via Commands; handles valid, iteration sums correct, Added/Changed intact); I-2 migration-into-grown-target (growth fires mid-apply; source bytes/ticks preserved); I-3 hooks-during-growth re-entrancy (on_add defers spawns into the SAME archetype at a slab boundary; nested growth; no double-apply); I-4 for_each_chunk single-slice witness after crossing; **I-5 ceiling panic on the Commands path (★R1-7, `should_panic`)**: a tiny D2-mapped archetype driven past `reserve_rows` through `Commands` pins that the re-worded apply-site `.expect` fires with the new ceiling wording (the previously-untested panic surface, R-§6).
- **Miri (`tests/miri_pool_growth.rs`)**: M-XI small-ceiling pools — multi-slab growth + boundary writes/reads + swap_remove across boundary + drop; + the 4 churn controls.
- **debug_assert invariants**: `len ≤ committed_rows ≤ reserve_rows`; frontiers granule-aligned, monotonic, ≤ region lengths; `committed_rows × stride ≤ data_committed`; post-grow `committed_rows ≥ n`; construction asserts.

## Critic Round 1 — resolutions (verdict: REVISE → folded; design core APPROVED)

The architecture-critic verified the load-bearing claims directly against the code: the D2 mapping ledger (drop_fn.rs:414-441 bit-identical, both proptests guard with `count() < capacity()`, dense bench compiles unchanged — zero re-specs), GROW1-XI sufficiency (re-derived; the step-4 `min(…, reserve_rows)` clamp confirmed load-bearing), D3 (refresh_column captures base at add_pool; U10 forbids refresh; eager reserve is the only Column-compatible design), D7 (zero chunk-accessor consumers workspace-wide), D10 (random access never loads a pool field; tick bases re-derived per set_table), the failure-surface mapping (all 4 panic sites route through reserve_capacity/can_push), the D1 layout math (disjoint, granule-aligned, SIMD-A1 + tick align hold), item-10 (no Archetype field added; pool size unpinned), and the XI-B2/B6 model arithmetic (sane, real headroom).

**Folded remarks** (★R1-n markers at the edit sites):
1. **CRITICAL — `grow_rows` idempotence**: no early-out existed; unconditional Phase-B calls would underflow `needed − data_committed` (debug panic / release wrap) and commit a doubling slab on EVERY satisfied `reserve_capacity(1)` — a per-spawn memory explosion — and could reach `vm.commit(new == old)`. Fix: the `n ≤ committed_rows → true` no-op arm + saturating-sub belt + GROW1-XI proof 0 (totality; no zero-size commit reachable) + U-P8 tests. The warm `add` path was already guarded; only `reserve_capacity` was exposed.
2. **MAJOR — D5 one-vs-two-compare contradiction**: resolved to the single-compare shape (binding); the ceiling check lives inside cold `grow_rows`.
3. **MAJOR — fallback footprint arithmetic**: vm.rs fallback `reserve` is an eager full-`os_len` `alloc_zeroed` ⇒ per-POOL eager scaling. Resolved together with old-Q5 as one decision: fallback target 16 → 4 MiB (pools ≤ 6 MiB; demo ceilings unchanged at 262,144; large strides shrink — documented loud-panic), `DEFAULT_ARENA_RESERVE` fallback 64 MiB → 1 MiB in W3, demo-world arithmetic ≈ 69 vs ≈ 86 MiB today (strictly better), D8 trade-off text corrected per arm, W5 records Miri RSS/wall-time delta.
4. MINOR — J-XI re-worded to the never-written form (vacated slots may be stale; write-before-read is load-bearing; no all-zero-above-len debug_assert).
5. MINOR — `reserve_rows == 0` loud pool-level construction assert (+U-P1).
6. MINOR — XI-B1(c) two-population protocol: ±2%-gated unchanged benches vs re-baselined workaround-unwound benches; XI-B4 decomposition on a miss.
7. MINOR — I-5 `should_panic` Commands-path ceiling test (the untested panic surface).
8. NOTE — single-allocation provenance stated in the W2 SAFETY sweep (S-TICKBASE hook).
9. NOTE — D2 constructor bypasses the MIN/MAX clamp BY DESIGN — binding doc sentence added.

**Old open questions, critic's answers**: Q1 keep 1 GiB/2^16/2^24 (VA math checks; doc note for `vm.max_map_count`, no runtime counter). Q2 keep the D2 mapping; rename in X.J. Q3 D7 in scope confirmed, no split commit. Q4 D8(b) accepted on blast radius (with the R1-3 fallback correction). Q5 folded into R1-3. Q6 desync accepted; frontier-write ordering pinned in D4 (fields written only after their commits succeed) — `can_push` pre-grow not needed. Q7 spike gate comfortable (2-10× model headroom); attribution fallback in results doc only. Q8 zeroed `reserve` accepted (X.G precedent; split-contract vm API not worth the surface).
