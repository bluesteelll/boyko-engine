# Phase X.G — Address-Stable Growth for `entities_inland`: Implementation Plan

Companion to `docs/PHASE-XG-RESEARCH.md` (cited as R-1..R-8; facts are NOT repeated here),
`docs/PHASE-XF-PLAN.md` (the proven reserve/commit pattern) and `docs/PHASE-XF-RESULTS.md`
(§B6 attribution = the motivating measurement). Branch `ecs`.

## Goal

Delete the engine's LAST realloc-doubling: the `EntityMaster::entities_inland:
Vec<EntityInland>` growth memcpy — the measured g7b worst-event (deterministic sub-batch
#580, 2.4–2.5 ms at ~580 k entities, twin #285 at 1.4–1.7 ms; X.F-RESULTS §B6).

- **Functionality**: entity-metadata growth becomes O(1) in live entities — one commit
  syscall, **zero bytes copied, zero bytes written** (demand-zero pages ARE
  `EntityInland::NULL` — R-1).
- **Performance**: the Phase-7 hot lookup (`get_component_raw` line 1 = one len compare +
  one 16-B indexed load, R-3) stays **codegen-identical**; the g7b doubling chain anchored
  at 9192 (R-5) disappears from the argmax attribution; g7 total improves by the deleted
  ~3–5 ms of memcpy + resize-fill + fault clustering.
- **Soundness**: growth never moves the base ⇒ SEND5's "no mid-flight realloc" clause
  becomes structural, not merely scheduling-enforced (R-4).

In scope: the `entities_inland` container only. Out of scope: `free_entity_ids` (R-6 —
plain LIFO Vec, no positional indexing), pool/arena growth (done, X.F), decommit/shrink,
refactoring `arena.rs` (D1).

## Context and constraints

- Affected: `entity_master.rs` (core), NEW `memory/vm.rs`, NEW `entity/inland_store.rs`,
  `constants.rs`, `ecs_master.rs` + `spawn_at_command.rs` (growth-site one-liners),
  comment sweeps, benches/tests per §Metrics.
- Invariants preserved: Phase-7 lookup shape (R-3); `len` as the only load-bearing scalar
  (R-2); EM2/EM6/SBO16/SCH7 write discipline (R-4); SEND5 (text updated, contract kept);
  M-001 matching deallocator; X.F gates NOT re-run (arena.rs untouched — D1).
- Binding targets: §Metrics XG-B1…XG-B6.

## Key decisions

### D1: Extract a shared `memory/vm.rs` reserve/commit primitive; arena does NOT migrate in X.G
**What**: NEW `crates/boyko_ecs/src/ecs/memory/vm.rs` with one type, `VmReservation` —
reserve (per-arm), `commit(old, new)` (per-arm), `Drop` (per-arm release), `base()`,
`os_len()`. The cfg matrix, SAFETY patterns (W-RES/W-CMT/U-RES/U-CMT/F-RES), the
`isize::MAX` guard, and `checked_align_up` mirror `arena.rs` exactly (R-8). All
*policy* (watermark, slab sizing, len) stays in the owner — `VmReservation` is a dumb
stateless wrapper over `(base, os_len[, fallback Layout])`, exactly the `Backing` role.
`arena.rs` is **not edited** — it keeps its inline arms; migration onto `vm.rs` is filed
as a follow-up (Phase X.H candidate, mechanical swap).
**Why**: zero blast radius on X.F — the X.F gates (asm identity of `allocate_layout`,
B1–B7, Miri growth suites) bind to `arena.rs` bytes; touching that file forces a full
re-run of an already-landed phase for zero functional gain. Extracting vm.rs now (rather
than duplicating ~150 lines of subtle per-arm `unsafe` inline into the store) costs the
same review effort once and makes the X.H arena migration a deletion, not a 3-way merge.
**Alternatives rejected**: (a) refactor arena.rs onto vm.rs in X.G — re-runs every X.F
gate, couples two phases' risk; (b) self-contained syscall arms inside `inland_store.rs`
— two divergent copies of the most audit-sensitive code in the engine, forever.
**Trade-off**: vm.rs is new unsafe surface that must independently pass review + the
fallback arm under Miri — but that is true of (b) too; (a) is the only option that
re-opens X.F, and it is the one rejected.
**vm.rs zero-fill contract**: "freshly committed memory reads zero on first access on ALL
arms." Syscall arms get this from the OS (`VirtualAlloc(MEM_COMMIT)` zero-fills;
anonymous `mmap` pages are zero-fill — same guarantees X.F already documents); the
fallback arm acquires with **`alloc_zeroed`** (NOT `alloc` — the global allocator returns
uninitialized bytes, and unlike the arena, this consumer READS never-written memory by
design). Miri models `alloc_zeroed` natively (no interpreted memset). Noted for X.H: the
arena does not need zeroing; add a flag then, not now.

### D2: Defaults — reserve 1 GiB (64-bit syscall arms) / 16 MiB (fallback); slabs 256 KiB → ×2 → 16 MiB
**What** (`constants.rs`):
- `DEFAULT_INLAND_RESERVE = 1 GiB` under `cfg(all(not(miri), any(windows, unix),
  target_pointer_width = "64"))`; **16 MiB otherwise** (fallback eager-allocs the full
  reserve — a large default is fatal there; same gating shape as `DEFAULT_ARENA_RESERVE`).
- `INLAND_MIN_SLAB = 256 KiB`, `INLAND_MAX_SLAB = 16 MiB`; granule =
  `ARENA_COMMIT_GRANULE` (64 KiB) reused as-is (no rename in X.G — renaming touches
  arena.rs; alias note left for X.H).
- DELETE the dead `INITIAL_ENTITY_CAPACITY` (`constants.rs:105`, zero uses — R-5).
**Arithmetic — reserve**: 1 GiB / 16 B = **67,108,864 entity slots**. VA is free (X.F:
`PROT_NONE` is unaccounted under overcommit mode 2; Windows user VA 128 TB; combined
world VA becomes 4 GiB arena + 1 GiB inland = 5 GiB — 50 concurrent test worlds = 250 GiB
VA, trivial). vs 256 MiB (16.7 M slots): zero cost difference (commit charge is identical
— it tracks committed, not reserved), only a lower ceiling; an id-leak workload through
`reserve_entity` (monotonic counter, no recycling) could plausibly pass 16.7 M, and the
arena exhausts at 4 GiB of payload long before 67 M real entities — 1 GiB aligns the two
ceilings. Fallback 16 MiB = **1,048,576 slots**: every Miri world now eagerly holds
16 MiB zeroed on top of X.F's 64 MiB arena fallback (+25%, `alloc_zeroed` is O(1) under
Miri); no Miri/wasm workload approaches 1 M entities.
**Arithmetic — slabs and the g7 workload** (R-5: first `ensure` request = 1000 + 8192 =
9192 slots = 147,072 B):
- Event 1: needed = align_up(147,072, 64 KiB) = 192 KiB; step = max(clamp(0, 256 KiB,
  16 MiB), 192 KiB) = **256 KiB** (16,384 slots).
- Then pure doubling: 256 KiB → 512 KiB → 1 MiB → 2 → 4 → 8 → **16 MiB** = 1,048,576
  slots ≥ the workload's 968,192 (960 k + 8192 hint). **7 commit events total**, largest
  single event = 8 MiB ≈ 9 µs at the X.F-measured ~1.1 µs/MiB; **all 7 events sum to
  ≈ 18 µs** (vs today's chain: caps 9192→…→588,288, with #580 alone touching 9.0 MiB of
  memcpy + fill — R-5).
- MIN = 256 KiB: covers the 9192-anchored first request in one event; a one-entity world
  commits 256 KiB resident (≈ 0.3 µs commit) — accepted floor. MAX = 16 MiB: one
  max-step covers the whole 1 M-entity bench; overshoot is bounded by one slab; past 1 M
  entities each further event is ≤ 18 µs — irrelevant.
**Post-X.G g7b prediction (honest)**: the #285/#580 twins DISAPPEAR (binding gate XG-B5:
absent from argmax). The new worst batch lands in the **batches 0–15 pool-creation
class** — X.F's attribution measured that class at 0.9–2.2 ms worst-single. Predicted
composite spike ratio (max − median, R3-2 aggregation) ≈ **0.06–0.17×** vs Bevy's
12.7 ms. The arithmetic does NOT support promising ≤ 0.1× composite: the residual upper
bound 2.2 ms / 12.7 ms = 0.17×, and that residual is X.F-attributed pool-creation work
(already ≈ 0.005× on the arena-event metric) — out of X.G's scope. Reported honestly,
gated by attribution, not by the composite letter.
**g7 total prediction**: boyko loses the doubling-chain events (#580 ≈ 2.45 ms + #285 ≈
1.55 ms + the k = 65/138 crossings ≈ 1 ms ⇒ ~4–5 ms) ⇒ ≈ 242 ms → ≈ 237–239 ms ⇒ ratio
≈ **1.78×** (from 1.75×). Binding floor stays ≥ 1.5×.

### D3: `InlandStore` with `Deref<Target = [EntityInland]>` — read/indexed-write sites need ZERO edits
**What**: NEW `entity/inland_store.rs`:
- Fields: `vm: VmReservation`, `len: usize` (slot count — THE load-bearing scalar, R-2),
  `committed_slots: usize` (commit frontier in SLOTS, not bytes — the warm-path
  comparator needs no arithmetic and the `n*16` overflow class vanishes from it).
- `impl Deref for InlandStore { type Target = [EntityInland]; }` via
  `slice::from_raw_parts(self.vm.base().cast(), self.len)` + the `DerefMut` twin.
  Every existing `.get(i)`, `.get_mut(i)`, `[i]`, `&mut ..[start..end]`, `.iter()`,
  `.len()` site across the crate (the full grep set: ecs_master.rs ×~15,
  migration_helpers.rs ×6, insert/remove_command.rs ×3, hierarchy/commands.rs ×1,
  query_view.rs ×2, spawn_at/spawn_batch reads, all in-file tests) compiles **unchanged**
  — `Vec` itself routes these through the identical slice ops, so the codegen shape is
  the same by construction (D6 proves it on the two hot symbols).
- Growth: `ensure(n)` (D4). NO `push`, NO `resize` — the API cannot express a copy.
**Why**: minimizes the diff to exactly the growth/clear/construction sites; the hot path
is untouched source-wise, leaving the asm gate to catch only field-offset displacement.
**Alternatives rejected**: explicit narrow methods without Deref — ~40 call-site edits,
same codegen, more churn to review; chunked-page table — adds a load + dependent address
computation to the Phase-7 hot lookup, fails R-3 by construction (the research's
headline disqualifier).
**Trade-off**: `Deref` exposes the whole immutable+mutable slice API `pub(crate)`-wide.
Accepted: the field was already `pub(crate)` with full `Vec` API exposed; no regression
in encapsulation.

### D4: `ensure(n)` = commit-to-cover + `len = max(len, n)` — ZERO writes for the tail
**What**:
```rust
#[inline]
pub(crate) fn ensure(&mut self, n: usize) {
    if n <= self.len { return; }                       // idempotent warm exit (1 cmp)
    if n > self.committed_slots { self.grow_to(n); }   // #[cold] #[inline(never)]
    self.len = n;                                      // the ONLY state change on the warm-grow path
}
```
`grow_to(n)` (cold): ceiling check `n ≤ os_len/16` (loud exhaustion panic naming the
entity ceiling) → `needed = checked_align_up(n * 16, GRANULE)` → `step =
clamp(old_bytes, INLAND_MIN_SLAB, INLAND_MAX_SLAB).max(needed - old_bytes)` →
`new_bytes = (old_bytes + step).min(os_len)` → `vm.commit(old_bytes, new_bytes)` →
`committed_slots = new_bytes / 16`. Post-condition `new_bytes ≥ needed` is a **proof,
not a check**: `old + step ≥ old + (needed − old) = needed`, and the `min(os_len)` clamp
cannot bite because the ceiling check guarantees `needed ≤ os_len` (os_len is
granule-aligned). The X.F GROW1/C1 sufficiency-check machinery is **entirely absent by
construction** — there is no free list, no fragmentation, no tail coalescing; growth is
a pure frontier bump. Granule induction mirrors X.F R2-W1: old_bytes starts 0 or
precommit-aligned; `clamp`/`max`/`min` of granule multiples stay granule multiples
(`needed` aligned, `needed − old` = aligned − aligned).
**Why zero writes**: R-1 — `EntityInland::NULL` is all-zero 16 B with no padding
(`repr(C)` 8+4+4, const-asserted), and freshly committed pages read zero (D1 contract).
Today's `resize(.., NULL)` writes 16 B × every new slot AND memcpys the old prefix; at
crossing #580 that is 4.5 MiB copied + 4.5 MiB written in one batch. X.G writes
**nothing** — the tail's NULL-ness is supplied by the kernel (or `alloc_zeroed`), and
demand-zero faults are deferred to each slot's first real registration write (better
distributed than today's resize-time fault burst).
**Explicit writes stay as today**: W4/W6/W11/W12 indexed stores into `[0, len)` —
unchanged through `DerefMut`.

### D5: `clear()` = memset `[0, len)` to zero, then `len = 0` — the stale-bytes hazard closed at the cheapest-correct point
**The hazard** (new in X.G, did not exist with `Vec`): today `Vec::clear` keeps capacity,
and re-growth re-NULLs via `resize`'s fill. X.G's no-write `ensure` would re-expose
whatever bytes the committed range held before `clear` — stale `archetype_ptr`s (dangling
into a possibly-rebuilt archetype slab) and stale generations: an entity-aliasing /
use-after-free class bug.
**What**: `clear()` performs `ptr::write_bytes(base, 0, len * 16)` BEFORE `len = 0`
(cold path; `#[cold]` on the memset branch). Correctness: every explicit write ever made
lands at an index `< len` at write time and `len` is monotonic between clears ⇒ zeroing
`[0, len)` restores invariant I-Z(b) (§Soundness) for the entire ever-written range;
`[len, committed)` was never written and is still kernel-zero. Generations reset to 0 —
**identical to today's semantics** (`Vec::clear` + re-grow-with-NULL also resets
generations; `clear` also empties `free_entity_ids` and zeroes the atomic counter, so
the post-clear world is uniformly fresh — the R-1 recycled-slot caveat applies only to
live ranges, which `clear` is destroying by definition).
**Cost**: O(high-water) memset, worst realistic case 16 MiB ≈ 1–2 ms — `clear` is a cold
world-reset API with zero hot callers.
**Alternatives rejected**: (b) `high_water` field + zero-on-regrow — moves the work and
an extra branch onto `grow_to` and distributes the invariant across two sites; (c)
decommit + recommit — per-arm divergence, more syscalls, fallback still needs the memset.
**Pinned test**: `clear_resets_live_count` (`entity_master.rs:866-881`) asserts
`capacity() == 0` post-clear — `capacity()` stays `len`-derived (D7) ⇒ green unchanged.
NEW regression test U-S4 (§Tests) is mandatory.

### D6: Hot-path proof obligations — asm identity of `get_component_raw` + `has_entity`
**What** (binding, X.F W4 methodology including the field-offset ratification lesson):
1. **Baseline capture pre-impl**: release asm of `EcsMaster::get_component_raw`
   (`ecs_master.rs:1290`) and `EcsMaster::has_entity` (`ecs_master.rs:1530`).
   ORCHESTRATOR NOTE: both are `#[inline]` with no standalone symbol in the lib crate —
   the baseline is the `random_access` bench binary asm (codegen-units=1), captured at
   HEAD as `D:\tmp\xg_baseline_random_access.s`; the diff scopes to the bench fns
   containing the inlined lookup sequence.
2. **Post-impl diff**: the instruction **multiset and control flow must be identical**;
   the ONLY permitted delta class is **displacement constants**, which is plan-entailed:
   - `Vec<EntityInland>` (24 B: ptr/cap/len at compiler-chosen offsets) is replaced by
     `InlandStore` (32 B on syscall arms: vm{base, os_len} + len + committed_slots) —
     the two hot loads move from Vec's ptr/len offsets to the store's base/len offsets;
   - `EntityMaster` grows 24→32 B in that field ⇒ `live_count` and every `EcsMaster`
     field laid out after `entity_master` may shift ⇒ further displacement deltas in any
     symbol touching them.
   Any instruction-count, instruction-class, or branch-shape delta is a FAIL.
3. Shape argument (why identity is achievable): `store.get(i)` = `Deref` →
   `from_raw_parts(base, len)` → `slice::get` = load base, load len, cmp, branch, lea
   `base + i*16` — exactly `Vec::get`'s sequence (Vec routes through the same slice op);
   `from_raw_parts` is two field loads, same as Vec's ptr/len loads.
**Bench gates** (alongside asm): `random_access.rs` lookup groups
(`get_component_raw_hot` ≤ 16 ns — measured ~3 ns — `has_entity` ≤ 5 ns, typed /
set_component_raw / stale_generation / missing_component) within ±2% multi-run, asm as
the controlling oracle at ns scale (X.B/X.F lesson); `create_entity_10k` ≤ 5% (expect
flat-or-better: `ensure` replaces `Vec::resize`'s fill loop with one cmp + occasional
cold commit); `delete_entity_10k` A/B; `iter_entities_*` baselines (slice iter ==
Vec iter shape).

### D7: `EntityMaster` integration surface
| Item | Decision |
|---|---|
| Field | `pub(crate) entities_inland: InlandStore` (same name/visibility) |
| `new()` | `InlandStore::new()` = reserve `DEFAULT_INLAND_RESERVE`, commit 0, len 0. Pays one reserve syscall per world (~0.3–0.8 µs, cf. X.F B2) — bound by XG-B4 (`EcsMaster::new` ≤ 7.5 µs, X.F B3 envelope). Lazy reservation rejected: a `OnceCell` indirection would put a check on the Phase-7 hot path. `phase12_6_lazy_alloc.rs` cap-0 contract holds: `capacity()` is `len`-derived = 0 on a fresh world |
| `with_capacity(c)` | reserve default + **precommit** `align_up(c*16, GRANULE)` (clamped by ceiling check), `len = 0`. Preserves `Vec::with_capacity`'s purpose — no growth events for the first `c` entities; cost = one commit syscall. `free_entity_ids: Vec::with_capacity(c/4)` unchanged (R-6) |
| `capacity()` | stays `self.entities_inland.len()` (max-ever id semantics) — zero churn on the two pinning tests. NEW `committed_slots()` diagnostic accessor (mirror of `Arena::committed()`) |
| `memory_usage()` | `free_entity_ids.capacity()*8 + committed_slots*16` — resident truth, not reservation. Zero callers (verified) ⇒ semantics change contained; doc updated |
| `compact()` | unchanged; comment gains "no decommit of `[0, len)` ever — I-Z forbids it (recycled-dead slots are non-zero); decommitting `[len, committed)` is a possible future non-goal" |
| Send/Sync | the explicit `unsafe impl Send/Sync` (SEND5, `entity_master.rs:530-555`) **stays** (it already had to exist: `EntityInland` holds `*mut Archetype`, so the old `Vec` was `!Send` too). SAFETY text updated: the "no worker can observe a mid-flight realloc" clause becomes **structural** (base is write-once; growth commits fresh pages + bumps `len` — no pointer is ever invalidated). IMPORTANT wording constraint: this is defense-in-depth, NOT a relaxation — `len` is a plain non-atomic `usize`, so a concurrent dispatcher-grow vs worker-read is still a data race; SCH7 (no workers in flight during `&mut self` windows) remains the normative argument |

**W-site map** (numbering = research R-4; every site, exhaustively):

| Site | File:line (today) | X.G change |
|---|---|---|
| W2 `allocate_entity` | `entity_master.rs:122-123` | `if id.0 >= len { resize }` → `self.entities_inland.ensure(id.0 + 1)` (ensure self-gates) |
| W3 `ensure_capacity` | `entity_master.rs:225-229` | body → `self.entities_inland.ensure(capacity)`; doc rewritten ("amortised memset" paragraph deleted — there is no memset) |
| W4 `register_batch` | `entity_master.rs:287` | `&mut self.entities_inland[start..end]` — **unchanged** (DerefMut) |
| W5 `register_entity_with_ptr` | `entity_master.rs:325-326` | defensive resize → `ensure(sparse_idx + 1)`; indexed store unchanged |
| W6 `deallocate_entity` | `entity_master.rs:356-364` | **unchanged** (indexed read+store) |
| W7 `clear` | `entity_master.rs:445-450` | `entities_inland.clear()` → store clear per D5; free list + counter resets unchanged |
| W8 `create_entity_at` | `ecs_master.rs:828-832` | resize block → `.ensure(id_raw + 1)` (one line) |
| W9 `create_entity_at_with_pool_ids` | `ecs_master.rs:949-953` | same |
| W10 `SpawnAtCommand::apply` | `spawn_at_command.rs:165-171` | same |
| W11/W12 swap-fixups + migration repoints | `ecs_master.rs:1212,1258`; `migration_helpers.rs:190,410,424,506,650,659` | **unchanged** (get/get_mut/indexed stores via Deref) |
| All read sites | `ecs_master.rs` (incl. hot :1296), `query_view.rs:349,399`, `insert_command.rs:59,113`, `remove_command.rs:66`, `hierarchy/commands.rs:119`, `spawn_*` debug_asserts, in-file tests | **unchanged** (Deref) |

## Data structures

```rust
// memory/vm.rs — cold module; no #[repr] (never frame-hot, single-owner, no sharing).
pub(crate) struct VmReservation {
    base: NonNull<u8>,   // write-once base of the single reservation; never reassigned
    os_len: usize,       // granule-rounded reservation length; isize::MAX-guarded
    #[cfg(any(miri, not(any(windows, unix))))]
    layout: Layout,      // fallback: exact Layout for dealloc (M-001)
}
// !Send/!Sync via NonNull — matches the Arena discipline; owners opt in explicitly.

// entity/inland_store.rs
pub(crate) struct InlandStore {
    vm: VmReservation,        // offset 0 ⇒ `base` is the struct's first word (hot load)
    len: usize,               // live slot count — THE load-bearing scalar (R-2);
                              // bounds oracle for get/capacity/iter/rewind_allocate
    committed_slots: usize,   // commit frontier in SLOTS (== committed_bytes/16);
                              // warm-path comparator without multiplication/overflow
}
// Hot pair (vm.base, len) = first two words — one cache line, same as Vec's (ptr, len).
// 32 B on syscall arms (Vec was 24 B) — EntityMaster grows by 8 B; displacement deltas
// downstream are plan-entailed (D6.2).
const SLOT_SIZE: usize = size_of::<EntityInland>(); // == 16, const-asserted
```

## Public API (all `pub(crate)` unless noted)

```rust
impl VmReservation {
    pub(crate) fn reserve(len: usize) -> Self;             // granule-rounds; asserts len>0, os_len<=isize::MAX; panics on OS failure
    pub(crate) fn base(&self) -> NonNull<u8>;              // #[inline]
    pub(crate) fn os_len(&self) -> usize;                  // #[inline]
    pub(crate) fn commit(&self, old: usize, new: usize);   // #[cold]; granule-aligned, new<=os_len (debug_assert'd); no-op on fallback
}
impl Drop for VmReservation;                               // per-arm release (M-001)

impl InlandStore {
    pub(crate) fn new() -> Self;                           // DEFAULT_INLAND_RESERVE, commit 0, len 0
    pub(crate) fn with_capacity(slots: usize) -> Self;     // + precommit, len 0
    pub(crate) fn with_reserve_bytes(bytes: usize) -> Self;// test knob (exhaustion/Miri small worlds)
    pub(crate) fn ensure(&mut self, n: usize);             // D4; #[inline] warm shell + #[cold] grow_to
    pub(crate) fn clear(&mut self);                        // D5
    pub(crate) fn committed_slots(&self) -> usize;
}
impl Deref for InlandStore { type Target = [EntityInland]; }   // #[inline]
impl DerefMut for InlandStore;                                  // #[inline]

impl EntityMaster {
    pub fn committed_slots(&self) -> usize;                // diagnostics (public, mirrors Arena::committed)
    // new()/with_capacity()/capacity()/memory_usage()/clear(): signatures unchanged
}
```

## Algorithms for critical paths

- **`get(idx)` (Phase-7 hot, via Deref)**: load base, load len, cmp, branch, `lea
  base + idx*16` — O(1), 2 loads + 1 cmp, one not-taken branch, sequential-free random
  access (1 line for the 16-B record). MUST be instruction-identical to today (D6).
  SIMD: N/A (single record).
- **`ensure(n)` warm**: 1–2 cmp + 1 store; O(1); no memory traffic beyond the store's own
  line. Cold `grow_to`: one syscall (~1.1 µs/MiB measured, X.F B4) + O(1) bookkeeping —
  **O(1) in live entities; zero copies; zero fills** (the headline deletion vs both the
  old Vec path and Bevy's `Entities` realloc).
- **`clear()`**: O(high-water) streaming memset (cold; candidate for non-temporal stores
  — NOT taken: cold path, plain `write_bytes` is simpler and the working set is dead
  afterwards anyway).
- **Branching/I-cache**: `grow_to`/`clear` memset are `#[cold]`/out-of-line; `ensure`'s
  warm shell is 3 instructions inlined at 6 call sites.

## Multithreading model

Unchanged from R-4 / SEND5, restated:
- All `InlandStore` mutation (`ensure`, writes via DerefMut, `clear`) is reachable only
  through `&mut EntityMaster` — dispatcher-side (owner direct API or apply window, SCH7:
  zero workers in flight).
- Worker `&self` reads (`get_component_raw`, `is_entity_valid`, `get_entity`, query_view)
  race nothing: no `&mut` exists while workers run.
- The ONLY worker-reachable field stays the atomic `next_entity_id` (EM6, unchanged).
- No atomics added; no ordering changes. `len`/`committed_slots` are plain `usize` —
  legal because of the exclusivity above, NOT because of address stability (the SEND5
  text update must keep this distinction — D7).
- Data-race freedom proof: identical to the pre-X.G proof (type-system-enforced `&mut`
  exclusivity + SCH7), now with the strictly weaker failure mode if the proof were ever
  violated (stale len vs dangling base+len).

## Soundness

1. **I-Z (the demand-zero-equals-NULL invariant, formal)**: at every program point, every
   slot `i < len` satisfies exactly one of: (a) it was explicitly written through a
   `&mut self` API since the most recent `clear()` (or construction), or (b) its 16 bytes
   read zero, which IS `EntityInland::NULL` (null ptr + 0 + 0; `repr(C)` 8+4+4 ⇒ **no
   padding bytes** ⇒ all 16 bytes are value bytes — unit-tested via transmute, U-S1).
   Slots in `[len, committed_slots)` are never readable (len is the bounds oracle) and
   are never written ⇒ they remain kernel-zero/alloc_zeroed. `ensure` preserves I-Z
   (touches no slot bytes); `clear` restores (b) on `[0, old_len)` by memset (D5);
   per-arm zero sources per D1. **Recycled-slot caveat (R-1)**: written-dead slots
   `{null, 0, gen+1}` are class (a) and NOT all-zero ⇒ re-zeroing any sub-range of
   `[0, len)` outside `clear` is forbidden (no decommit/`MEM_RESET` of the live range,
   ever — documented on `compact`).
2. **`Deref` slice validity**: `from_raw_parts(base, len)` — base is non-null, 8-aligned
   (VirtualAlloc 64 KiB / mmap 4 KiB / alloc_zeroed layout-align 64), provenance spans
   the whole reservation (single allocated object), `len*16 ≤ committed ≤ os_len ≤
   isize::MAX`, and all `[0, len)` bytes are initialized (I-Z: OS-zeroed pages are
   initialized memory — same position the X.F fallback/Miri suite already validates for
   read-after-commit; the fallback arm is explicitly `alloc_zeroed`). No references are
   stored — every `&[_]`/`&mut [_]` is derived per-call and dies with the call (R-2:
   zero interior pointers anywhere remains true). TB: per-call reborrows from a
   write-once raw base, no cached tags — the 9.3c/14a-F2 foreign-write class cannot
   arise (no twin caches a pointer to this buffer).
3. **New unsafe inventory** (each lands with the listed SAFETY):
   - V-RES-W / V-RES-U / V-RES-F in `vm.rs` — verbatim ports of arena W-RES / U-RES
     (incl. the MAP_FAILED-before-NonNull trap) / F-RES (with `alloc_zeroed` + non-zero
     size + power-of-two align); `isize::MAX` guard before any arm (X.F review F1).
   - V-CMT-W / V-CMT-U — ports of W-CMT (idempotent re-commit documented) / U-CMT
     (page-aligned by granule induction, return checked: the ENOMEM surface).
   - V-DROP per arm — ports of the three arena Drop SAFETY blocks (`MEM_RELEASE`
     releases regardless of commit state; `munmap(base, os_len)` full length;
     `dealloc(base, layout)` exact layout). M-001: matching deallocator statically
     enforced by the cfg-gated field set.
   - S-SLICE (Deref/DerefMut) — item 2 above.
   - S-CLEAR (`write_bytes`) — `[0, len*16) ⊆ committed RW`; restores I-Z(b).
4. **Edge cases**: `ensure(0)`/`ensure(n ≤ len)` no-ops; `ensure` past the ceiling ⇒ loud
   cold panic naming slots requested/ceiling (the entity analog of arena exhaustion);
   `n*16` overflow impossible on the warm path (slot-unit comparator) and `checked_*` on
   the cold path; `id.0 + 1` overflow excluded by the counter-exhaustion debug_asserts
   (`< usize::MAX/2`, `entity_master.rs:167,199`); `with_capacity(0)` = reserve-only;
   `clear` on empty store = no-op; `get(usize::MAX)` ⇒ None via len cmp. Generation reset
   on clear = today's semantics (D5).
5. **Drop order**: `EntityInland` is `Copy` POD ⇒ no element drops; `InlandStore` drop =
   `VmReservation` release only. The store holds raw `*mut Archetype` VALUES (never
   dereferenced in `EntityMaster`, R-1) ⇒ no ordering constraint against
   `ArchetypeMaster`/`Arena` in `EcsMaster::drop` — unchanged field order is fine.
6. **Miri**: the fallback arm runs ALL bookkeeping (`ensure` math, len, clear-memset,
   Deref reads of never-written slots) — every existing Miri churn suite (`miri_phase19`,
   14a, 14b, 8cd) traverses it implicitly on every spawn; plus the dedicated M-XG suite
   (§Tests). What Miri cannot prove (real syscall semantics) is covered natively by the
   vm.rs unit tests on the Windows host + `cargo check --target
   x86_64-unknown-linux-gnu`; the **X.F W5 residual stands**: the unix arm still has
   never executed on real Linux — X.G widens that residual to vm.rs and the results doc
   MUST carry it forward (or close it via WSL/CI for both at once).

## Integration

| File | Change |
|---|---|
| `crates/boyko_ecs/src/ecs/memory/vm.rs` | **NEW** — `VmReservation` (+ in-file unit tests) |
| `crates/boyko_ecs/src/ecs/memory/mod.rs` | register `vm` |
| `crates/boyko_ecs/src/ecs/core/entity/inland_store.rs` | **NEW** — `InlandStore` (+ in-file unit tests) |
| `crates/boyko_ecs/src/ecs/core/entity/mod.rs` | register `inland_store` |
| `crates/boyko_ecs/src/ecs/constants.rs` | + `DEFAULT_INLAND_RESERVE` (cfg-gated 1 GiB / 16 MiB), `INLAND_MIN_SLAB`, `INLAND_MAX_SLAB`; − `INITIAL_ENTITY_CAPACITY` |
| `entity_master.rs` | field swap; W2/W3/W5/W7; `new`/`with_capacity`/`memory_usage`/`compact` doc+body; SEND5 text; struct doc header; + `committed_slots()` |
| `ecs_master.rs` | W8/W9 one-liners; doc updates at :392 (lazy-alloc paragraph), :462-465 (`with_capacity` entity_capacity semantics + ceiling note), :55 (SEND comment), :783-786 |
| `spawn_at_command.rs` | W10 one-liner; Step-3 comment (:160-171) |
| `spawn_batch_command.rs` | comments only (:60, :310-319 — SBO16/SBO17b wording references `Vec` realloc) |
| `bench_bevy_vs_boyko/benches/profile_spawn_v2.rs` | stale comment :148 |
| tests/benches | per §Metrics; `growth_crossing.rs` + `random_access.rs` re-run unmodified (argmax instrumentation kept from X.F) |
| `arena.rs` | **ZERO changes** (D1) |

## Implementation plan (waves — each compiles + in-scope tests green)

1. **W1 — vm.rs + constants**: `VmReservation` (3 arms + Drop + asserts), the three new
   constants, delete `INITIAL_ENTITY_CAPACITY`. Unit tests U-V1…U-V5.
2. **W2 — inland_store.rs**: struct, Deref/DerefMut, `ensure`/`grow_to`/`clear`/
   constructors, `with_reserve_bytes`. Unit tests U-S1…U-S8 + P1 (store not yet wired).
3. **W3 — EntityMaster swap**: field type, W2/W3/W5/W7 edits, constructors,
   `capacity`/`memory_usage`/`compact`/`committed_slots`, SEND5 + doc text. All in-file
   entity_master tests green unchanged (they exercise Deref).
4. **W4 — cross-file growth sites + comment sweep**: W8/W9/W10 one-liners; all listed
   doc/comment updates. Full `cargo test --all-targets` green; clippy clean.
5. **W5 — new test files + Miri**: `tests/entity_store_growth.rs` (I1–I3),
   `tests/miri_entity_store.rs` (M1–M2); run Miri suites (new + phase19/14a/14b/8cd
   control).
6. **W6 — gates**: asm baseline-vs-post diff (D6 — baseline captured from HEAD at
   `D:\tmp\xg_baseline_random_access.s` BEFORE Wave 3), `random_access.rs` multi-run A/B,
   `create/delete_entity_10k`, XG-B4, g7/g7b re-run + argmax attribution; optional
   untargeted diagnostic group `entity_store_grow/{256KiB,8MiB}`; write
   `docs/PHASE-XG-RESULTS.md`.

## Metrics and validation

### Binding gates
- **XG-B1 — asm identity**: `get_component_raw` + `has_entity` instruction-multiset
  identical; permitted delta = displacement constants only (enumerated in D6.2).
- **XG-B2 — random_access**: all lookup groups within ±2% (multi-run, asm is the oracle
  at ns scale); `create_entity_10k` ≤ 5% (expect flat/better); `delete_entity_10k` A/B
  no regression; `iter_entities_*` baselines.
- **XG-B3 — suites**: full debug+release test suites green; clippy `-D warnings`; Miri:
  new M-XG suite + the four churn-suite controls clean.
- **XG-B4**: `EcsMaster::new` ≤ 7.5 µs (X.F B3 envelope; expect ≈ 6.8 µs = 6.32 + one
  1 GiB reserve syscall).
- **XG-B5 — g7/g7b re-run** (cold worlds, Bevy 0.18.1 not pre-reserved, X.F harness +
  R3-2 spike aggregation, instrumentation kept): (a) **g7 total ≥ 1.5× vs Bevy** (expect
  ≈ 1.78×); (b) **attribution gate: the 9192-anchored doubling chain (#285/#580 twin
  class) is ABSENT from the per-iteration argmax** — the new mode must land in the
  batches 0–15 pool-creation class; (c) composite spike ratio reported honestly with
  prediction ≈ 0.06–0.17× (improvement from 0.234×; **≤ 0.1× is NOT promised** — D2
  arithmetic; a residual above 0.1× attributes to the X.F pool-creation class via the
  argmax dump, not to X.G).
- **XG-B6 — no-memcpy growth witness**: I2 below (address stability across multi-slab
  growth — impossible with `Vec`).

### Test matrix
- **vm.rs (U-V)**: U-V1 reserve/commit/drop ×50 incl. partially-committed reservations
  (native syscall round trip); U-V2 commit range debug_asserts (granule, bounds); U-V3
  zero-on-first-access witness (read fresh-committed bytes == 0, head+tail of slab);
  U-V4 `reserve(0)`/isize-guard panics; U-V5 fallback-arm layout round trip (runs under
  Miri).
- **inland_store.rs (U-S)**: U-S1 `transmute::<EntityInland,[u8;16]>(NULL) == [0;16]`
  (the I-Z keystone); U-S2 **address-stability + no-write witness**: take `&store[0] as
  *const _` (and a written slot's value) → `ensure` across ≥ 3 slab boundaries → same
  address, value intact, `committed_slots` advanced; U-S3 never-written tail reads NULL
  after multi-slab ensure (sample slots at slab boundaries ± 1); U-S4 **stale-bytes
  clear/regrow regression** (the D5 hazard): write non-NULL records across two slabs →
  `clear()` → `ensure` past the old len → every slot in the old range reads NULL; U-S5
  grow-policy table (first-event request-dominant vs MIN_SLAB; doubling; MAX clamp;
  ceiling clamp; granule alignment of every frontier — mirror of X.F U1); U-S6
  exhaustion panic on `with_reserve_bytes(256 KiB)` + `ensure(20_000)`, message names
  the slot ceiling, `committed_slots`/`len` unchanged after unwind; U-S7
  `with_capacity` precommits (`committed_slots ≥ c`, `len == 0`, no grow event during
  first `c` ensures); U-S8 `clear` keeps `committed_slots`, `len == 0`.
- **Property (P1)**: model-based proptest — random sequence of
  `ensure/write(i<len)/clear/read` against a `Vec<EntityInland>` reference model
  (resize-with-NULL semantics); assert `get(i)` equivalence after every op.
- **Existing pins (must stay green UNCHANGED)**: all `entity_master.rs` in-file tests
  (recycle/generation/batch/live_count/clear-capacity-0/rewind);
  `phase12_6_lazy_alloc.rs` (cap-0 fresh world; spawn_batch lazy growth ≥ 1000);
  `miri_phase19/14a/14b/8cd`; F4 witness `archetype_bundle.rs:1164` (pointee-side).
- **Integration (`tests/entity_store_growth.rs`)**: I1 spawn 100 k entities via
  `Commands` batches across the 9192-anchored thresholds → all handles valid, components
  readable; I2 (XG-B6) EntityMaster-level address witness across growth; I3
  world-`clear()` + respawn → no stale liveness (old handles invalid, fresh gen-0
  handles valid).
- **Miri (`tests/miri_entity_store.rs`)**: M1 small-reserve store: ensure ×3 slabs +
  writes + clear + regrow + reads of never-written slots (validates the
  initialized-zero position under TB); M2 EntityMaster churn
  (allocate/register/deallocate/recycle) on the fallback arm.
- **debug_assert! invariants**: `len ≤ committed_slots ≤ os_len/16`; granule alignment +
  monotonicity of the commit frontier; `grow_to` post `n ≤ committed_slots`; vm commit
  range checks; (debug-only, in `ensure`) cold-path re-verification that a sampled fresh
  slot `is_null()`.

## Open questions for the critic

1. **`Deref<Target=[EntityInland]>`** (D3) — full slice API exposed `pub(crate)` to keep
   ~40 call sites untouched, vs explicit narrow methods (`get/get_mut/index/slice_mut/
   iter`) costing the churn but constraining the surface. I chose Deref (it mirrors the
   `Vec` exposure that already existed); push back if the unconstrained mutable slice
   (e.g. `swap`, `copy_from_slice`) is judged an invariant risk worth the diff.
2. **vm.rs extraction with arena migration deferred** (D1) — confirm the blast-radius
   reasoning; alternatively demand the inline-duplication variant if a NEW shared
   primitive without its second consumer is judged premature.
3. **`clear()` = memset** (D5, option a) vs high-water zero-on-regrow (b): is a 1–2 ms
   worst-case clear acceptable as the cold-path price for keeping the growth path
   write-free and the invariant single-sited?
4. **`with_capacity` precommit** (D7) vs reserve-only (uniform laziness): I chose
   precommit for `Vec::with_capacity` semantic fidelity; a purist may prefer one lazy
   semantic everywhere (then the only delta is one cold grow event later).
5. **1 GiB default reserve** vs 256 MiB (D2) — same commit cost, only ceiling/VA
   optics; would accept 256 MiB with nothing else changing.
6. **vm.rs zero-fill contract** (D1) forces `alloc_zeroed` on the fallback arm for ALL
   future vm.rs consumers until a flag is added (arena migration X.H would pay an
   unnecessary eager memset on exotic targets unless flagged) — acceptable debt?
7. **g7b composite letter** (XG-B5c): the plan deliberately does NOT bind ≤ 0.1×
   composite (arithmetic caps the guarantee at ≈ 0.17× worst case, residual is
   X.F-class). Confirm the attribution-based gate (b) is an acceptable binding substitute.
8. **Per-world reserve syscall in `EntityMaster::new`** (D7) — bound by XG-B4; the lazy
   alternative was rejected for hot-path purity. Veto if a measured `EcsMaster::new`
   regression past 7.5 µs appears.

---

# R2 (FINAL — folds critic round 1; BINDING, supersedes the body where it differs)

Critic verdict on R1: **CHANGES-REQUESTED** (1 critical proof-text fix, 3 warnings, 3
optionals — all adopted). Core design (D1/D2/D4/D6/D7) confirmed; no re-review needed for
a revision confined to this list. OQ verdicts: ALL EIGHT accepted as proposed (Deref;
vm.rs extraction with drift guard; clear=memset; precommit modulo W2; 1 GiB; alloc_zeroed
debt; attribution gate; per-world reserve).

## C1 — I-Z restated as induction J (the body's D5 lemma was false for multi-clear)

The body's clause "[len, committed) was never written" fails from the second clear cycle
onward (write 0..100 → clear → write 0..50 → clear #2 memsets only [0,50) — slots
[50,100) WERE written before clear #1). The prescribed CODE is correct; the proof is
replaced by:

> **Invariant J: at every program point, every slot in `[len, committed_slots)` reads
> all-zero.** Maintenance: `ensure` grows `len` into a region J guarantees is zero (and
> writes nothing); explicit writes land only at indices `< len` (structurally enforced —
> every write path is slice `Index`/`IndexMut`/`get_mut`; `rewind_allocate` does NOT
> truncate len — verified, it only rolls back the atomic counter; len shrinks ONLY at
> `clear`); `clear` memsets `[0, len)` then sets `len = 0`, making `[0, committed)`
> uniformly zero. I-Z(b) for newly exposed slots is a corollary of J.

This exact text goes into the S-CLEAR / I-Z SAFETY comments. **U-S4 is extended to a
two-clear shrink/regrow cycle**: write across two slabs → clear → ensure/write a SMALLER
range → clear → ensure past the ORIGINAL high-water → assert the band
`[small_len, old_highwater)` reads NULL (regression net against "optimizing" the memset
to the latest live range).

## W1 — I-Z(b) de-jure framing fixed (supersedes the "X.F already validates" clause)

X.F consumers write-before-read; X.G's U-V3/U-S3 are the FIRST never-program-written
reads — the X.F citation is dropped. The owned position: (a) de facto —
`VirtualAlloc(MEM_COMMIT)` and anonymous `mmap` zero-fill are hard OS contracts; (b) de
jure — the Rust AM does not model raw-syscall memory; the justification is
**equivalence with `alloc_zeroed` itself**: production allocators' calloc/HEAP_ZERO_MEMORY
fresh-page paths hand back untouched kernel-zero pages and the `GlobalAlloc` contract
calls them initialized — treating OS zero-fill as an external write of zeros is exactly
as official. Stated as an explicit assumption in the S-SLICE SAFETY comment; Miri
validates ONLY the fallback (`alloc_zeroed`) arm; the syscall arms are validated natively
by U-V3/U-S3. The results doc carries this as a named residual alongside the X.F W5
Linux entry.

## W2 — `with_capacity(c)` over-ceiling behavior pinned: option (b), reserve-sizing

`InlandStore::with_capacity(slots)` sizes the reservation as
`max(DEFAULT_INLAND_RESERVE, checked_align_up(slots * 16, GRANULE))` — Vec's "never
refuses a satisfiable request" semantics preserved; no silent clamp, no new panic
surface at construction (the OS reserve failure panic remains the only one). Documented
on `EcsMaster::with_capacity` (the :462-465 sweep); NEW test U-S9: `with_capacity` above
the default ceiling succeeds, `committed_slots ≥ c`, and `ensure(c)` triggers zero grow
events.

## W3 — fallback-arm cost paragraph (native wasm32 + Miri honesty)

Native wasm32/exotic worlds eagerly allocate **16 MiB zeroed per world** (a real memset
under dlmalloc) on top of X.F's 64 MiB arena fallback — accepted because the shipping
wasm demo creates exactly ONE world; embedders needing smaller worlds on exotic targets
are out of scope for X.G (a public `with_entity_reserve` knob mirroring
`with_arena_reserve` is NOT added — noted as a trivial follow-up if asked).
"`alloc_zeroed` is O(1) under Miri" is SOFTENED to: the X.F 64 MiB eager fallback
precedent showed no measurable Miri wall-time impact; W5 watches the Miri gate wall-time
and reports any regression.

## O1-O3 — adopted

- **O1**: `InlandStore` gets `#[repr(C)]` (cost-free; makes the D6.2 displacement story
  deterministic) and the field-offset comment corrected (`len` is the third word — the
  hot pair claim is "one cache line", not "first two words").
- **O2**: `rewind_allocate` (`entity_master.rs:504-514`) added to the W-map read-site row
  (Deref-covered, zero edits; its fresh-slot `is_null` debug_assert is a free I-Z witness).
- **O3**: reciprocal twin-code cross-reference comments at arena.rs arms AND vm.rs
  ("twin of …; any fix must be mirrored; unification = X.H"); `grow_to`'s comment spells
  the full granule chain verbatim: "`os_len` is a granule multiple ⇒
  `align_up(x, G) ≤ os_len ⟺ x ≤ os_len`; `64 KiB | os_len` ⇒ `os_len/16` exact;
  ceiling-check-before-`needed` ordering closes the granule-slack trap".
