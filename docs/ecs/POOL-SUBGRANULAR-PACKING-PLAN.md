# Sub-granular packing — lowering the per-column resident floor (PLAN, rev 2)

> Status: DESIGN, revised after critique pass 1 (2026-09-10; log at the end of this file — one
> blocking finding fixed, six non-blocking fixed, none refuted; every fix re-verified against the
> tree in a second reviser pass the same day, which corrected the plan's own "iff" for the
> data/added overlap and priced the full-capacity G3 fixture). No code written; no cargo run, no bench, no timing
> (owner's box busy). Every number below is arithmetic over `const` values in
> `crates/boyko_ecs/src/ecs/constants.rs`, `memory/component_pool.rs`, `memory/vm.rs`,
> `memory/vm_column.rs` on `feat/ecs-native-storage` @ `46c8e489`, re-derived here rather than
> quoted — three of the figures the tree currently states are wrong, and the corrections are part of
> the deliverable (S3/S4).

## Problem & goal

A non-empty `ComponentPool` column costs **384 KiB of committed memory for one row** (63 of 64
component ids; 192 KiB for the 64th). Not 192 KiB — the tree's own prose (`constants.rs:101`,
`component_pool.rs:469`, `scratch_column.rs:17`) states 192 KiB, which is the `stagger == 0` case
only; the `align_down`/`align_up` pair in `commit_subregion` (`component_pool.rs:567-568`) puts the
OS frontier one whole granule beyond each of the three sub-region frontiers whenever
`pool_base_stagger(id) != 0`. Nothing in the tree measures committed bytes (zero tests, one bench),
so the doubling has been invisible.

The floor is per **column**, independent of row count, so it is the dominant cost of every sparse
archetype: a `boyko_ui` `ButtonBundle` archetype costs ≈ 2.94 MiB at one widget; a Gaia data table
with four columns costs ≈ 1.6 MiB for 2 KB of payload; 1000 sparse archetypes × 4 columns charge
**1.5 GiB** of Windows commit against a limit that, when exhausted, fails as a loud
`VirtualAlloc(MEM_COMMIT)` NULL (`vm.rs:227-231`).

**Goal:** take the floor to **12 KiB per tracked column / 4 KiB per untracked column** — 32× — with
**zero hot-path delta** (byte-identical codegen for `row_ptr`, `add`, query inner loops), **no loss
of address stability**, **no change to `pool_base_stagger` or its per-cohort contract**, and a
**memory oracle** that fails when the floor regresses. Non-goals: change-detection semantics; any
scheme that relocates a column; decommit; `InlandStore`'s per-world 256 KiB slab.

## Target metrics

| quantity | today (σ≠0) | after | ratio |
|---|---|---|---|
| tracked column, stride 4…64 B, 1 row | 384 KiB | **12 KiB** | 32× |
| tracked column, stride 2 B | 512 KiB | 20 KiB | 25.6× |
| tracked column, stride 1 B | 768 KiB | 20–36 KiB (phase) | 21–38× |
| tracked ZST / tag column | 256 KiB | **8 KiB** | 32× |
| untracked column (`ScratchColumn`) | 128 KiB | **4 KiB** | 32× |
| `VmColumn<EntityId>` (`entity_ids`, `s2e`) | 64 KiB | **4 KiB** | 16× |
| Gaia table = 4×40 B columns + `entity_ids` | 1600 KiB | **52 KiB** | 30.8× |
| twenty such tables (OPEN-QUESTIONS F2) | 31.25 MiB | **1.02 MiB** | 30.8× |
| `ButtonBundle` archetype (7 sized + 1 ZST + ids) | 2.94 MiB | **96 KiB** | 31.3× |
| 33 live solver `ScratchColumn`s | 4.13 MiB | **132 KiB** | 32× |
| 1000 sparse archetypes × 4 columns | 1.5 GiB | **48 MiB** | 32× |
| **steady state, ANY column size** | +180 KiB/column of straddle overshoot | 0 | — |

The owner's stated target for Gaia was 256 KiB per table (~128× on small tables). This plan delivers
**52 KiB per table**, 4.9× past the target, and additionally removes 180 KiB per column at *every*
size (3 × (64 KiB − 4 KiB) of `align_up`-to-granule overshoot that today is permanent, not just a
floor effect) — ≈ 540 MiB at 3000 columns.

A composition worth stating: `0cb082bb` (untracked backing) took a scratch column 384 → 128 KiB;
this plan takes it 128 → 4 KiB. End to end a solver scratch column goes **96×**.

Hot-path budget: **0 instructions changed**. Every edit lands in `#[cold] #[inline(never)]`
functions (`grow_rows`, `grow_rows_zst`, `commit_subregion`, `VmColumn::grow_to`) or in `const fn`
layout math evaluated at construction. Cold-path budget: **+12 `VirtualAlloc`/`mprotect` calls,
once, per column that grows past 4 KiB of data** (4 extra doublings × 3 sub-regions), ≈ 24 µs per
column at a ~2 µs syscall, paid only by columns with ≥ ~100 rows; ≈ 72 ms one-time across a
3000-column process, against 1.45 GiB of commit charge. Page-fault counts are unchanged (faults are
per touched page, independent of the commit quantum).

## The one structural insight the options are ranked by

At the floor, a column's cost is **(number of sub-regions) × (commit quantum)**, not
(bytes/row × rows). Two corollaries decide the whole design:

1. **Bytes-per-row levers do nothing for the floor.** Chunk-granular ticks (the Unity DOTS model,
   128× on tick bytes/row) leave the floor at exactly 3 quanta, because each of the three
   sub-regions still needs its first quantum. That lever is a *slope* lever and is out of scope
   here (owner fork 2).
2. **The quantum is an engine invariant, not an OS one.** `VmReservation::commit` debug-asserts
   64 KiB alignment on both ends (`vm.rs:201-204`) and `constants.rs:1-7` calls 64 KiB "the Windows
   reservation granularity". Reservation granularity (`dwAllocationGranularity`, 64 KiB) constrains
   `MEM_RESERVE` base and size only; `MEM_COMMIT` inside an existing reservation is **page-granular
   (4 KiB)**, and `mprotect` is page-granular on Linux. The engine has been paying a 16× quantum for
   a constraint that binds a different operation.

So the floor is 16× too high for a reason that costs nothing to remove, and the packing schemes that
try to share a quantum between columns are attacking the second factor at the price of the
invariants the kernel's parallel-safety argument rests on.

## Decisions (final)

### D1 — the commit quantum becomes `COMMIT_PAGE` (4 KiB); the reservation granularity stays 64 KiB

`COMMIT_PAGE = 4096` on `target_arch = "x86_64"` (the stated target platform, both OSes);
`COMMIT_PAGE = COMMIT_GRANULE` on every other arm, so a 16 KiB/64 KiB-page aarch64 kernel keeps
today's behaviour instead of handing `mprotect` a mis-aligned base. `const _: () =
assert!(COMMIT_GRANULE.is_multiple_of(COMMIT_PAGE))`. Reservations (`os_len`, `pool_byte_layout`,
`VmReservation::reserve`) stay granule-rounded — untouched.

`VmReservation::commit`'s alignment debug-assert relaxes from `COMMIT_GRANULE` to `COMMIT_PAGE`. The
assert's own stated purpose — "a frontier commit can never overrun the kernel's page-rounded
mapping" — is discharged by page alignment plus the unchanged `new <= os_len`, since `os_len` is
granule-rounded and a granule is a whole number of pages.

**Why not query the OS page size at runtime**: `pool_byte_layout`, `pool_commit_step` and the
`const _` proofs are `const fn`; a runtime page size makes the layout math non-const and moves four
compile-time proofs to runtime. A `#[cfg(debug_assertions)]` one-time belt at the first `reserve`
(`GetSystemInfo().dwPageSize` / `sysconf(_SC_PAGESIZE)` divides `COMMIT_PAGE`) covers the case where
the compile-time assumption is wrong, loudly, at boot.

### D2 — the commit ladder is expressed on **absolute page floors**, not on sub-region-relative offsets

This is what makes the floor exactly one page per sub-region instead of two.

Every sub-region offset is `≡ σ (mod COMMIT_PAGE)` where `σ = pool_base_stagger(id) < 4096`
(`data_off = σ`; `added_off = σ + data_len`; `changed_off = σ + data_len + tick_len`; `data_len` and
`tick_len` are granule multiples, hence page multiples). So each sub-region's page floor is exactly
`base_off − σ` — one subtraction on a cold path, no new field.

The frontier fields change meaning (not size — `ComponentPool` stays size-pinned at 128 B / 144 B
miri, and no field is added):

```
data_committed  := committed bytes of the data sub-region measured FROM ITS PAGE FLOOR
                   (i.e. including the σ prefix); always a COMMIT_PAGE multiple
ticks_committed := same, for EACH tick sub-region (UNTRACKED_TICKS = usize::MAX sentinel unchanged)

needed  = align_up_page(σ + n * stride)
step    = pool_commit_step(data_committed, needed)              // unchanged function
new_d   = (data_committed + step).min(align_up_page(σ + data_len))
commit_page_region(base_off - σ, data_committed, new_d)         // both ends page-aligned by construction
rows    = ((new_d - σ) / stride).min(reserve_rows)
t_new   = align_up_page(σ + rows * 4)
```

Consequences, all of them wanted:

* **The first commit is exactly one page.** `[0, 4096)` covers the σ pad and `(4096 − σ)/stride`
  rows. For σ = 4032 and stride 40 that is 1 row — it is a *floor*, and the ladder doubles from
  there.
* **No `align_up` slack survives.** Today `commit_subregion` rounds the frontier up to a granule,
  permanently overshooting by one granule per sub-region; with page floors the committed span is
  exactly `data_committed` bytes. This is the 180 KiB/column steady-state win.
* **The bookkeeping fields become an exact model of what the OS was told**, provided the model is
  read as the *union* of the three committed intervals rather than their sum (obligation 10 — the
  sum over-counts at up to two region boundaries, geometry-dependently). That is what lets the
  oracle be a cheap in-process check instead of a process-global counter.

`pool_align_up_granule` stays (it sizes reservations); `pool_align_up_page` is its twin for the
ladder. `POOL_MIN_SLAB` is retargeted to `COMMIT_PAGE` with `const _: () = assert!(POOL_MIN_SLAB ==
COMMIT_PAGE)` — one knob, not two. `POOL_MAX_SLAB` (64 MiB) unchanged.

### D3 — the ladder keeps ×2 doubling below the granule; it is NOT widened to ×4

×4 below 64 KiB would save 2 of the 4 extra grow events but raise the *effective* floor for a column
sitting at ~5 KiB of data from 8 KiB to 16 KiB. The floor is the point of the item and the events
are 12 cold syscalls once per populated column (≈ 24 µs). Rejected with those numbers. Batch spawn is
unaffected either way: `Archetype::reserve_capacity(n)` is request-dominant, so a burst is **one**
event regardless of the quantum. `ScratchColumn::clear` keeps commit, so the physics refill loop
pays zero events after warm-up.

### D4 — `pool_base_stagger` is untouched, and the plan makes its span constraint a compile error

Same function, same values, same per-cohort obligation; `scratch_ids.rs`'s seven
`const _: () = assert!(width <= POOL_STAGGER_LINES)` proofs and
`scratch_band_stagger_slots_are_distinct` remain valid verbatim. The plan *adds* a dependency the
tree has so far held by coincidence: D2's `page_floor = base_off − σ` requires `σ < COMMIT_PAGE`, so

```rust
/// The L1 set-index span the stagger cycles through: 64 lines × 64 B = 4 KiB (the VIPT index
/// span; `constants.rs:160-166`). A cache property, NOT the commit quantum.
pub const POOL_STAGGER_SPAN: usize = POOL_STAGGER_LINES * CACHE_LINE_SIZE;   // == 4096
const _: () = assert!(POOL_STAGGER_SPAN <= COMMIT_PAGE);                       // D2's page-floor need
```

and `pool_byte_layout`'s existing `stagger < 4096` belt (`constants.rs:287-290`) is kept at the
cache-set bound — written as `stagger < POOL_STAGGER_SPAN`, **not** loosened to `< COMMIT_PAGE`.
The two bounds are different facts and only coincide on x86_64: on the non-x86_64 arm
`COMMIT_PAGE == COMMIT_GRANULE == 65536`, so `< COMMIT_PAGE` would admit a 60 KiB stagger that the
L1 set index wraps to a no-op (64 lines wrap the 64-set index), and the `<= COMMIT_PAGE` assert is
merely true there (4096 ≤ 65536) rather than tight. The commit quantum must not define the
cache-set bound; it only has to be *at least* the cache-set bound, which is the second assert.

A future edit raising `POOL_STAGGER_LINES` to 128 (an 8 KiB span) would silently break the page-floor
derivation today; after this it is a compile error on x86_64 (8192 > 4096) and still a valid
layout on a 64 KiB-quantum arm — which is the correct answer on both.

`pool_base_stagger`'s doc claim "costs at most one extra lazily-committed page per pool"
(`constants.rs:194-195`) is **false today** (it costs a full 64 KiB granule per sub-region of
Windows commit charge) and becomes **true** under D2, in the strong form: zero extra pages, because
the page floor absorbs the pad.

### D5 — one reservation per column stays; no column shares a reservation, no column relocates

The three packing options that share address space are rejected, each on a measured-invariant
ground rather than on taste:

**(A) Sub-slab bump allocation — several small columns inside one 64 KiB reservation.**
Floor: below one page (multiple columns per page) — the only option that beats D1+D2. Rejected on
two independent grounds:
1. *It defeats `pool_base_stagger` by construction.* Two columns in the same page have element-0
   rows within 4 KiB of each other, i.e. the same L1/L2 set family — the exact conflict storm the
   stagger exists to prevent and that cost a measured ~40 % rigid-solver regression at P2.
2. *False sharing across parallel writers.* The scheduler's conflict nodes are per-`ComponentId`
   whole-world (`access.rs`/`conflict_graph.rs`), so two systems writing two different components
   of the same archetype **do** run concurrently. Packing their rows into one cache line converts a
   legal parallel schedule into a line-ping-pong storm. This is not recoverable by padding: padding
   to a cache line per column reinstates a 64 B floor per column, and to a page reinstates D1's.
3. Cost we would additionally pay: growth past the sub-slab requires relocation (see B).

**(B) Per-archetype arena of column slices.**
Floor: ~1 page per non-empty *archetype* — the theoretical best. Rejected: with per-column capacity
inside one span, growing column `k` relocates columns `k+1..n`. Relocation is exactly the property
`std::Vec` has and `ComponentPool` does not, and *not* having it is the structural fix for the
SP4 colored-solve data race (`scratch_column.rs:6-10`): `ScratchSolveView`/`DenseSolveView` copies
held by workers, `Archetype::columns[c].ptr`, and every `row_ptr` SAFETY comment
(`component_pool.rs:819-833`) are written against a write-once base. Re-introducing relocation to
save memory would re-open the campaign's most expensive defect. Additionally it inherits (A)'s
false-sharing objection. A Unity-DOTS-style fixed 16 KiB chunk avoids relocation but makes `row_ptr`
a chunk lookup plus an offset instead of one add — a hot-path regression on every access, rejected
against principle 1.

**(D) A size-class ladder on `reserve_rows`.**
Floor impact: **zero**. `reserve_rows` sets `data_len`/`tick_len`/`os_len` — *address space* — and
the floor is commit-driven. A column with `reserve_rows = 64` still commits the same three first
quanta as one with `reserve_rows = 26_843_545`. Rejected as a floor lever; it remains a valid VA/VMA
lever (`≤ 6` VMAs per pool, 3.4 TiB budget at 3000 pools), and nothing here needs it. Stated
explicitly because conflating the VA ceiling with the resident floor is how
`OPEN-QUESTIONS.md` F2 arrived at a wrong number (S4).

**(C) = D1+D2, chosen.** It attacks the quantum, which is the only factor of the floor that is not
load-bearing for anything else.

### D6 — the two tick sub-regions stay separate (no interleaving), and the data-driven lockstep stays

*Interleaving* `added`/`changed` into one 8 B/row region would cut the sub-region count from 3 to 2
(floor 12 KiB → 8 KiB). Rejected: the dominant tick access is a *sequential single-region scan*
(`Changed<T>`/`Added<T>` filter fetch, `check_ticks`), and interleaving doubles that scan's working
set from 4 B/row to 8 B/row — one cache line per 8 rows instead of per 16. Paying a 2× D-cache cost
on a hot filter to save 4 KiB once per column is the wrong trade by orders of magnitude.

*Tick-driven row clamping* (`rows = min(data-derived, tick-derived)`) would make the floor a flat
12 KiB for every stride instead of 12 KiB for stride ≥ 4 and up to 36 KiB for stride 1. Rejected:
it adds a second binding constraint to the meaning of `committed_rows` and more grow events for byte
columns, to fix a case (1–2 byte components) that the engine already routes to bitset storage for
tags and that is 21–38× improved regardless.

### D7 — the oracle is an in-process byte model **plus** an OS-truth arm, not a shipped counter

D2 makes the three frontier fields an exact description of the three intervals the OS was told to
commit — `data: [0, data_committed)`, `added: [data_len, data_len + ticks_committed)`, `changed:
[data_len + tick_len, data_len + tick_len + ticks_committed)` (absolute page floors; both tick
intervals empty when `ticks_committed == UNTRACKED_TICKS`; the data interval empty for a ZST pool).
The model is therefore

```
committed_bytes() = | data ∪ added ∪ changed |        // byte length of the UNION
```

and **not** the sum `data_committed + 2 · ticks_committed`: adjacent intervals overlap by exactly
one page whenever the earlier region's ladder has reached its cap (`align_up_page(σ + len)` =
`len + P` for σ > 0 — obligations 2 and 3), which happens at the data/added boundary *and/or* the
added/changed boundary depending on the pool's geometry — and the data cap can be reached by a
doubling overshoot before full capacity, not only by the last request (obligation 10 lists five
geometries with three different answers). The union is a cold, `#[cfg(test)]`-only computation over three sorted
intervals and is exact for every geometry in both phases. A `pub(crate) fn committed_bytes(&self)`
on `ComponentPool` costs nothing shipped and is test-parallel-safe (a process-global `AtomicU64` inside `VmReservation::commit`
was considered and rejected: it is polluted by concurrently-running tests in the same binary, and
`VmReservation` cannot carry a per-reservation counter — `PoolBacking::Host(VmReservation)` must stay
≤ 16 B to keep `ComponentPool` at its pinned 128 B).

A model that only re-states our own arithmetic is a gate that cannot fail, which is this
repository's catalogued failure mode. Therefore the model is cross-checked against the OS in two
tests (G3): Windows walks the reservation with `VirtualQuery` and sums `MEM_COMMIT` regions;
Linux parses `/proc/self/maps` for the lines inside `[base, base+os_len)` and sums the `rw-p` prefix
(exact, because a monotone prefix of equal-protection mprotects merges into one VMA).

## The floor, derived

`P = COMMIT_PAGE = 4096`, `G = COMMIT_GRANULE = 65536`, `σ = (id % 64) * 64 ∈ [0, 4032]`, stride `s`.

Per sub-region, first grow to `n = 1` row:

```
data   pages = ceil((σ + s) / P)                       // = 1 for σ + s ≤ 4096
rows         = (data_pages * P - σ) / s
ticks  pages = ceil((σ + 4 * rows) / P)  each          // = 1 whenever s ≥ 4
floor        = (data_pages + 2 * tick_pages) * P       // tracked
             = data_pages * P                          // untracked
             = 2 * tick_pages * P                      // tracked ZST (vacuous data region)
```

For `4 ≤ s` and `σ + s ≤ 4096` — every component in the tree — this is **exactly 3 pages = 12 KiB,
independent of `σ`**, which is what makes the oracle an exact equality rather than a bound. `s ≥ 4`
gives `4 · rows ≤ 4 · (P − σ)/s ≤ P − σ`, so the tick pages cannot exceed the data page.

Large strides: `s = 4096, σ = 64` → data 2 pages, ticks 1 page each → 16 KiB. Tiny strides:
`s = 1, σ = 0` → data 1, ticks 4 each → 36 KiB; `σ = 4032` → 20 KiB.

Today's counterpart, for the same inputs (`abs_old = align_down_G(σ)`, `abs_new = align_up_G(σ + f)`
per sub-region): 6 × 64 KiB = 384 KiB at `s ∈ [4, 64]`, 12 × 64 KiB = 768 KiB at `s = 1`.

## Data structures

No new type, no new field, no size change.

```rust
// constants.rs — new
/// OS commit page. MEM_COMMIT inside a reservation and mprotect are page-granular;
/// only MEM_RESERVE is bound by the 64 KiB allocation granularity.
#[cfg(target_arch = "x86_64")]
pub const COMMIT_PAGE: usize = 4096;
#[cfg(not(target_arch = "x86_64"))]
pub const COMMIT_PAGE: usize = COMMIT_GRANULE;      // 16/64 KiB-page kernels keep today's behaviour

const _: () = assert!(COMMIT_GRANULE.is_multiple_of(COMMIT_PAGE));
const _: () = assert!(POOL_MIN_SLAB == COMMIT_PAGE);
pub const POOL_STAGGER_SPAN: usize = POOL_STAGGER_LINES * CACHE_LINE_SIZE;   // 4096, the L1 set span (D4)
const _: () = assert!(POOL_STAGGER_SPAN <= COMMIT_PAGE);                       // D4: page floor absorbs the pad
// pool_byte_layout keeps `stagger < POOL_STAGGER_SPAN` (the cache-set bound), NOT `< COMMIT_PAGE`.

pub const POOL_MIN_SLAB: usize = COMMIT_PAGE;       // was 64 * 1024
pub(crate) const fn pool_align_up_page(v: usize) -> usize;   // checked twin of pool_align_up_granule

// component_pool.rs — same fields, new units (D2)
//   data_committed  : bytes from the data sub-region's PAGE FLOOR   (COMMIT_PAGE multiple)
//   ticks_committed : bytes from each tick sub-region's PAGE FLOOR  (COMMIT_PAGE multiple,
//                     or UNTRACKED_TICKS = usize::MAX, unchanged sentinel)
#[cold] #[inline(never)]
fn commit_page_region(&mut self, base_off: usize, old: usize, new: usize);   // was commit_subregion

#[cfg(test)]
pub(crate) fn committed_bytes(&self) -> usize;      // D7 model: |data ∪ added ∪ changed| — exact (obligation 10)

// vm.rs — assert relaxed
debug_assert!(old.is_multiple_of(COMMIT_PAGE) && new.is_multiple_of(COMMIT_PAGE));

// vm_column.rs — checked_slab_round rounds to COMMIT_PAGE; the constructor assert tightens
assert!(COMMIT_PAGE.is_multiple_of(Self::SIZE));    // was COMMIT_GRANULE; forbids only T of 8/16/32/64 KiB
```

Unchanged and load-bearing: `pool_byte_layout` (reservation geometry), `os_len`, `buffer`,
`added_base`, `changed_base`, `row_ptr` (`buffer.add(idx * stride)` — one add), `add`'s single warm
compare, `pool_base_stagger`, `POOL_MAX_SLAB`, `INLAND_MIN_SLAB` (per-world, one instance, always
filled — lowering it only adds grow events), `EnableStore` (already sub-granular: 512 B heap pages).

## Public API

None added. `COMMIT_PAGE` is `pub` for the same reason `POOL_MIN_SLAB` is (constant-table docs and
cross-crate cohort proofs); `committed_bytes()` is `#[cfg(test)] pub(crate)`. `pub` census
(`world_committed_bytes()`) is deferred to optional step S5.

## Proof obligations

The proof chains in `grow_rows` (GROW1-XI 1…5) and `grow_rows_zst` (Z1…Z6) are restated on page
floors; each must be re-established in the code's own comment style, and each has a
`debug_assert!` twin:

1. **Sufficiency.** `new_d ≥ needed = align_up_page(σ + n·s)` ⇒ `rows = (new_d − σ)/s ≥ n`. (Today's
   step-3.) `debug_assert!(rows >= n)`.
2. **Data in bounds.** `new_d ≤ align_up_page(σ + data_len) ≤ data_len + P ≤ added_off + P ≤ os_len`.
3. **Tick in bounds — the assert that made the rejected `tick_len = 0` design unsound.** Today's
   `debug_assert!(t_new <= layout.tick_len)` must become
   `debug_assert!(t_new <= pool_align_up_page(σ + layout.tick_len))`. With the old bound and σ > 0,
   `t_new` legitimately reaches `tick_len + P` at full capacity and would trip — the same class of
   error `0cb082bb` documented, so it is named here rather than discovered.
4. **No reservation overrun at full capacity.** `changed`'s maximum absolute end is
   `data_len + tick_len + align_up_page(σ + tick_len) = data_len + 2·tick_len + P` for σ > 0, and
   `os_len = align_up_granule(σ + data_len + 2·tick_len) = data_len + 2·tick_len + G ≥ that` since
   `P ≤ G`. For σ = 0 both sides equal `data_len + 2·tick_len`, so `commit`'s `new <= os_len` holds
   with equality.
5. **Strict growth.** `new_d > data_committed` and `t_new > ticks_committed` on every path reaching
   `commit` (`VmReservation::commit`'s `new > old`), unchanged in form from GROW1-XI 0a/0b and Z4
   with `align_up_page` substituted.
6. **Page alignment of both ends.** `base_off − σ` is a page multiple (all sub-region offsets are
   `≡ σ mod P`); `data_committed`/`ticks_committed` are page multiples by induction over the ladder.
   `debug_assert!((base_off - σ).is_multiple_of(COMMIT_PAGE))` and the same on `old`/`new`.
7. **★R1-8 unchanged.** One reservation per pool; data + both tick regions remain one allocated
   object; no pointer derivation changes.
8. **Address stability unchanged.** No relocation, no shared reservation, `buffer` write-once;
   ★Q6 panic coherence (frontier fields written only after the commits succeed) preserved verbatim.
9. **Untracked sentinel unchanged.** `t_new > usize::MAX` is false for every `t_new`, so
   `new_untracked` still skips both tick commits with zero added branches.
10. **Model exactness (D7).** `committed_bytes()` is the byte length of the union of the three
    committed intervals (D7) and equals the OS-committed byte count **exactly, for every geometry,
    in both phases** — G3 pins a plain equality at the floor and at full capacity. The sum form
    `data_committed + 2·ticks_committed` is NOT exact and must not be used: it over-counts by `P`
    at each region boundary whose earlier region has reached its cap, and rev 1 of this plan named
    only one of the two boundaries and pinned `os == model − P`, which is red on some fixtures and
    green for the wrong reason on others. The two boundaries, with the condition for each:
    * **data/added.** Obligation 2 caps the data ladder at `align_up_page(σ + data_len) =
      data_len + P` (σ > 0) and `added`'s page floor is `added_off − σ = data_len`, so once the data
      ladder reaches its cap the page `[data_len, data_len + P)` belongs to both intervals. The cap
      is reached by **either** of two routes, and a condition that names only the first is wrong:
      (i) *by request* — `align_up_page(σ + reserve_rows·stride) > data_len`, i.e. whenever
      `reserve_rows·stride` is close enough to its granule round-up that σ pushes it over, in
      particular whenever it is *exactly* a granule multiple (this is the full-capacity route);
      (ii) *by doubling overshoot* — D2's `new_d = (data_committed + step).min(cap)` with the
      in-band step `step = data_committed` (`pool_commit_step`, `constants.rs:347-356`) lands on
      the cap whenever `2·data_committed > data_len` while `needed ≤ data_len`, i.e. whenever
      `data_len` is not itself a rung of the ladder (`P·2^k`, then 64 MiB multiples). Example:
      `data_len = 3G` (`pool_byte_layout(3072, 64, σ)`): the ladder reaches `2G`, the next request
      `needed = 2G + P` gives `new_d = min(4G, 3G + P) = 3G + P` — the data interval covers the
      `added` floor page **before** full capacity. The three original table geometries do not
      show route (ii) because `G`, `4G` and `2^30` are all rungs; the fourth row below does.
      Only the union model is indifferent to which route was taken.
    * **added/changed.** Obligation 3 caps the tick ladder at `tick_len + P`; reached iff
      `align_up_page(σ + reserve_rows·4) > tick_len`.

    Geometries from the tree, σ > 0 (constants as in `constants.rs:296, 303, 307-315, 513`;
    fixture `component_pool.rs:3303, 3338`; rows 4-5 are derived fixtures, not existing ones):

    | pool | data cap hit? | tick cap hit? | `os` vs the SUM |
    |---|---|---|---|
    | default 40 B / 64 B pool at full capacity: `pool_reserve_rows = 2^24` (`2^30/40 = 26 843 545 > POOL_MAX_ROWS`, clamped; 64 B is the exact edge), `data_len = 2^24·stride` exactly, `tick_len = 2^26` exactly | yes, route (i) (`data_len + P`; `2^30 + P` at stride 64) | yes (`2^26 + P`) | `os == sum − 2P` |
    | `make_stride64_pool(4096)` at full capacity: id 226 ⇒ σ = 2176, `data_len = 262144 = 4096·64` exactly; ticks `align_up_page(2176 + 16384) = 20480 < 65536` | yes, route (i) (`266240`) | no | `os == sum − P` (data/added, not the boundary rev 1 named) |
    | `pool_byte_layout(256, 16, σ)` at full capacity: `data_len = 65536`, data reaches `8192`; ticks reach `≤ 8192 < 65536` | no | no | `os == sum` |
    | `pool_byte_layout(3072, 64, σ = 2176)` at `n = 2015` (NOT full): `data_len = 3G = 196608`; ladder `P…G, 2G`; `needed = align_up_page(2176 + 2015·64) = 135168 > 2G` ⇒ `new_d = min(2G + 2G, 3G + P) = 200704`; ticks `align_up_page(2176 + 3072·4) = 16384 < 65536` | yes, route (ii) — before full capacity | no | `os == sum − P` from `n = 2015` on |
    | `pool_byte_layout(16384, 4, σ)` at full capacity: `data_len = tick_len = G`; data `needed = align_up_page(σ + 65536) = G + P`; ticks `align_up_page(σ + 65536) = G + P` | yes, route (i) | yes | `os == sum − 2P` at ≈ 196 KiB of commit — the cheap twin of row 1 |

    The union model returns `os` in every row; the sum model with a `− P` clause is wrong in
    three of them. **Cost note (arithmetic, not a measurement):** row 1 at full capacity commits,
    at stride 64, a union of `2^30 + 2·2^26 + P = 1 207 963 648 B ≈ 1.125 GiB` (at stride 40:
    `2^24·40 + 2^27 + P = 805 310 464 B ≈ 768 MiB`) of Windows commit charge inside a test binary
    that runs other tests concurrently; row 5 exercises the same two-boundary overlap for
    `3G + P = 200 704 B ≈ 196 KiB`. G3 therefore runs rows 2-5 unconditionally and row 1 under
    `#[ignore = "slow: commits ≈ 1.1 GiB to reach the default pool's full capacity"]` (the census
    class from CLAUDE.md), so the both-caps case is still gated on every run without a gigabyte of
    commit. `debug_assert!`-twin: none on the hot path (the model is test-only); G2(b) is the
    in-process guard against a model that double-counts.

## Windows vs Linux — where the win actually lands (honest)

* **Windows.** `MEM_COMMIT` charges the system commit limit **at commit time**, per page, touched or
  not. The 384 KiB → 12 KiB is therefore a real, immediate reduction in a resource whose exhaustion
  is a hard failure. `dwAllocationGranularity` (64 KiB) constrains only `MEM_RESERVE`, which this
  plan does not touch; `VirtualAlloc(MEM_COMMIT)` rounds its range to page boundaries. **This claim
  is the plan's load-bearing OS assumption and is gated, not asserted** — G3 walks the reservation
  with `VirtualQuery` and compares against the model.
* **Linux, default overcommit (0/1).** `mprotect(PROT_READ|PROT_WRITE)` over an already-mapped
  anonymous range allocates nothing; RSS follows first touch. Today's *resident* floor on Linux is
  therefore already ~3–6 pages, and this change moves RSS little. What it does move: the accounted
  bytes under `vm.overcommit_memory=2` (where a PROT_NONE map is unaccounted and the mprotect is
  what accounts), and the mapping's RW prefix. **State it plainly: this item is primarily a Windows
  commit-charge win and a strict-overcommit-Linux win.** It is still the right change on Linux —
  the 180 KiB/column steady-state overshoot (D2) is accounted there too — but nobody should expect a
  large `VmRSS` delta on a default Linux CI box, and a gate written against `VmRSS` there would be
  noise, which is why G3's Linux arm reads the mapping's `rw-p` extent instead. That arm is
  `target_os = "linux"`, not `unix`: the tree's syscall arm is `unix` (`vm.rs:141, 234, 279`) but
  `/proc/self/maps` exists only on Linux, so the other unix targets get a stated-reason skip (G3).
* **VMA/VAD count: unchanged.** Commits form a monotone prefix, so adjacent equal-protection
  mprotects merge; the documented "≤ 6 VMAs per pool" bound (`constants.rs:42-44`) survives. On
  Windows a VAD is per-reservation and committed pages are PTE state, so the count is unaffected by
  construction.
* **Fallback arm (Miri / wasm / 32-bit): unchanged.** `reserve` is an eager `alloc_zeroed(os_len)`
  and `commit` is a no-op, so the footprint is `os_len` regardless of the quantum. The oracle
  measures nothing there and must be `#[cfg]`-scoped to the syscall arms **with a stated reason**,
  not silently (G2 covers the vacuity risk).

## Migration steps

**S0 — the oracle, red first.** New test module (next to the existing pin ledger in
`component_pool.rs`, or `memory/commit_floor_tests.rs`): `ComponentPool::committed_bytes()`
(`#[cfg(test)]`), the floor tests of G1 written against the NEW numbers, plus the two OS-truth
tests (G3) and the anti-vacuity canaries (G2). Record the red output verbatim in the commit message.
**The two arms go red with different old numbers, and both are expected**, because they measure
different things on the old code:

* **G1 (in-process model), old code.** `committed_bytes()` is written against the NEW interval
  semantics (D7) but reads TODAY's fields, which are sub-region-relative and granule-rounded
  (`component_pool.rs:197-201`): the first grow of a stride-40 pool sets `data_committed = 65536`
  (`:654` with `step = POOL_MIN_SLAB`, `constants.rs:348-349`) and `ticks_committed = 65536`
  (`:673`), so the model reports `3 · 65536 = 196 608 B` per column, **3 145 728 B** for 16
  columns, against the asserted **196 608 B**.
* **G3 (OS truth), old code.** `commit_subregion` (`:567-568`) rounds each sub-region's range out
  to `[align_down_G(base_off), align_up_G(base_off + 65536))` = two granules for σ > 0, so the
  OS holds `6 · 65536 = 393 216 B` per column against the model's `196 608 B` — G3 walks ONE
  pool's reservation, so its red reads `393 216 ≠ 196 608` per pool, and `6 291 456 ≠ 3 145 728`
  only if the fixture sums all 16. Only this arm can see the `align_up` overshoot; the
  in-process model cannot, by construction.

A red that names a number other than these two means the instrument is measuring something else —
that is the point of recording both. **The 16-column fixture must be built from default-sized
pools** (`with_default_sizes` / `pool_reserve_rows`, `component_pool.rs:536`): the 384 KiB-per-column
old figure holds only when every sub-region is ≥ 2 granules long. For a small explicit geometry such
as `pool_byte_layout(256, 16, σ)` the three rounded-out ranges are `[0, 2G)`, `[G, 3G)`, `[2G, 4G)`,
whose union is the whole 256 KiB reservation, and the OS-truth red would read 4 194 304 B instead.

**S1 — the quantum and the ladder.** `constants.rs`: `COMMIT_PAGE`, `POOL_STAGGER_SPAN`, three
`const _` asserts, `POOL_MIN_SLAB = COMMIT_PAGE`, `pool_align_up_page`; `pool_byte_layout`'s
`stagger < 4096` literal becomes `stagger < POOL_STAGGER_SPAN` (same value, now named — it is the
cache-set bound and is **not** rewritten to `< COMMIT_PAGE`, see D4). `vm.rs`: relaxed alignment
assert + the debug-only boot page-size belt.
`component_pool.rs`: `commit_subregion` → `commit_page_region`, `grow_rows` and `grow_rows_zst`
arithmetic per D2, the ten proof `debug_assert!`s, `committed_bytes()`.
`vm_column.rs`: `checked_slab_round` → page, constructor assert → `COMMIT_PAGE.is_multiple_of(SIZE)`.
`core/log/ring.rs`: the `LogLine` divisibility `const _` follows the tightened constant (16 | 4096 ✓).
The oracle goes green here.

**S2 — the pin ledger.** Every test whose expectation encodes the 64 KiB quantum must be re-derived,
not deleted (enumerated so none is "fixed" by weakening it):
`constants.rs:675` (`pool_commit_step(0, G) == POOL_MIN_SLAB` → `== COMMIT_PAGE`);
`vm_column.rs:536` `MIN_ELEMS`; `component_pool.rs:3315-3318` `make_stride64_pool` and its
`:3346` comment (first commit = 1024 rows → `(4096 − 2176)/64 = 30` rows for `STRIDE64_ID = 226`);
`address_stability_across_three_slab_growths` (:3333); `tick_lockstep_and_jxi_zero_at_slab_boundary`
(:3472); `zst_pool_growth_two_successive_commits` (:3889);
`untracked_pool_commits_data_but_never_ticks` (:3950 — its twin-equality and anti-vacuity arms hold
unchanged, only the byte numbers move); `grow_rows_idempotent_below_frontier` (:3632, unchanged in
substance); `vm_column.rs:715` / `inland_store.rs:396` grow-policy tables (`InlandStore`'s policy is
unchanged — verified: `inland_store.rs:43` imports `COMMIT_GRANULE, DEFAULT_INLAND_RESERVE,
INLAND_MAX_SLAB, INLAND_MIN_SLAB` and not `POOL_MIN_SLAB`, so its table does not move); `benches/d6_commit_vs_faults.rs:113,162`
(its local `COMMIT_GRANULE` copy and the `committed_data_bytes` formula).

**S3 — the prose that is currently false.** `constants.rs:54-58` ("one `POOL_MIN_SLAB` (64 KiB)
commit per NON-EMPTY column" — true only for `VmColumn`, and only at σ = 0);
`constants.rs:99-103` ("3 × 64 KiB per pool"); `constants.rs:194-195` (the stagger's "at most one
extra page" claim, false today, true after D2); `component_pool.rs:458-471` and `:588`;
`scratch_column.rs:14-19` and `:70-77` (the 192 KiB / 3.0×/2.0×/9.0× ratios);
`docs/SYSTEMS.md:1898`; `docs/MEMORY-SYSTEM-AUDIT.md` per-store table.
Two more sites whose prose is true today and becomes false the moment S1 lands:
`core/log/ring.rs:11-12` ("`4096 * 16 B` is exactly one `COMMIT_GRANULE`, so the line column's
reservation is a single commit step") — under a 4 KiB `POOL_MIN_SLAB` the `LogLine` column reaches
`LINE_CAP` in **five** grow events when lines arrive one at a time (4 → 8 → 16 → 32 → 64 KiB;
`VmColumn::grow_to` is request-dominant, so a bulk request is fewer), so the sentence must say the
ceiling divides the quantum and drop "single commit step"; and `ring.rs:69-82`, whose comment
explains the divisibility pin in terms of `COMMIT_GRANULE` and must follow the assert to
`COMMIT_PAGE`. `vm_column.rs:32-45` (module doc "Supported element domain": "must divide
`COMMIT_GRANULE`"), `:134-136` (the `new` Panics list) and `:485-500`/`:506` (the "granule chain"
comment: "every term is a granule multiple") all describe the divisor as the granule; S1 changes
the assert and `checked_slab_round`, so S3 rewrites these to say page (the chain argument is
unchanged in form with `COMMIT_PAGE` substituted for `COMMIT_GRANULE`).
`book/src/memory/arena.md:87` is `doc-writer`'s, not the developer's.

**S4 — the two documents that computed from the wrong sentence.** `docs/OPEN-QUESTIONS.md` F2
(`:295-304`) derives "four non-empty columns ⇒ 256 KiB per table ⇒ ~5 MiB for twenty tables,
~128× overhead" from the `VmColumn` sentence at `constants.rs:55`. The true pre-change figure is
**1.6 MiB per table / 31.25 MiB for twenty (≈ 800×)** — F2 *understated* its own decisive number by
6.25× — and the post-change figure is **52 KiB per table / 1.02 MiB for twenty (≈ 26×)**. F2's
ruling is not reversed by this (its axis is resident memory and access cost), but its arithmetic
must be re-stated on both sides or the next reader inherits the error. Same for
`docs/gaia/DECISIONS.md` D-2/D-3, where D-3 already names "sub-granule packing" as the available
direction — this plan is its answer, and D-3's "it pays beyond Gaia: every sparse archetype in the
engine currently pays the same floor" is confirmed with the 1.5 GiB → 48 MiB figure.

**S5 (optional, non-blocking).** A `pub` memory census — `world_committed_bytes()` walking archetype
bundles and dense stores — for Gaia's load budgeting. Owner-scope; not required by this item.

## Gates

* **G1 — the floor oracle (exact equality, both phases).**
  `floor_of_sixteen_small_tracked_columns`: 16 ids with σ ≠ 0, stride 40 B, one row each ⇒
  `Σ committed_bytes == 16 * 3 * COMMIT_PAGE == 196 608`. Companions, each an equality:
  σ = 0 vs σ = 4032 give the same 12 288 B; untracked ⇒ 4 096; tracked ZST ⇒ 8 192; a four-column
  40 B table plus its `VmColumn<EntityId>` ⇒ 53 248; stride 1 and stride 2 columns pinned at their
  phase-dependent values so the tiny-stride ladder (D6) is documented by a test rather than by prose.
* **G2 — anti-vacuity (three arms, because a memory gate is exactly the kind that goes green from
  emptiness).** (a) a single grow moves `committed_bytes` by exactly one page — a model stuck at 0
  is red; (b) an idempotent `grow_rows` below the frontier moves it by exactly 0 — a model that
  double-counts is red; (c) a **recorded mutation**: reverting `POOL_MIN_SLAB` to 64 KiB must turn
  G1 red, run once by the implementer and the output pasted into the commit message. Additionally,
  the `#[cfg]` that scopes G1 to the syscall arms carries its reason at the site, per the
  `ignore_reasons_census` discipline, so "not compiled here" is never mistaken for "passed".
* **G3 — OS truth.** Windows: `VirtualQuery` walk of one pool's reservation, sum of `MEM_COMMIT`
  bytes `== committed_bytes()` — a **plain equality** at the floor AND at full capacity, for σ > 0
  and σ = 0 alike (obligation 10: the union model is exact everywhere; there is no `− COMMIT_PAGE`
  clause and no full-capacity special case). Run it on the geometries of obligation 10's table —
  rows 2-5 always, row 1 `#[ignore = "slow: …"]`-classed (its full capacity is ≈ 1.1 GiB of
  commit) — because they exercise different boundary overlaps, one of them (row 4) *before* full
  capacity, and a single fixture would pass for the wrong reason. Linux (`#[cfg(target_os =
  "linux")]`, not `unix`):
  `/proc/self/maps` lines within `[base, base+os_len)`, sum of `rw-p` extents, same equalities.
  The remaining unix targets (`vm.rs`'s arm is `unix`, `:141/:234/:279`; macOS has no
  `/proc/self/maps`) carry a stated reason at the site — `#[cfg_attr(not(target_os = "linux"),
  ignore = "…: the OS-truth arm reads /proc/self/maps, which exists only on Linux")]`, with the
  class prefix drawn from the census vocabulary once that lands, or a `#[cfg]` with its reason
  comment exactly as G2 already requires — so a non-Linux unix build is a recorded skip, never a
  vacuous green. This is the arm that makes G1 a measurement
  instead of a restatement.
* **G4 — the 0 % gate.** Objdump/asm diff of `ComponentPool::add`, `ComponentPool::row_ptr`,
  `ScratchSolveView::row_ptr` and one `Query::for_each` inner loop, before vs after: **byte
  identical**. Nothing hot is touched; a non-empty diff means something leaked out of a `#[cold]`
  function.
* **G5 — address stability AND stagger placement.** `address_stability_across_three_slab_growths`
  extended to ≥ 8 growths (the ladder now has four more rungs below 64 KiB): `buffer`,
  `added_base`, `changed_base` and a previously-taken `row_ptr(0)` are pointer-identical
  throughout. Additionally, for a σ ≠ 0 id (`STRIDE64_ID = 226`, σ = 2176), before the first grow
  and after the last: `buffer_ptr() as usize % COMMIT_PAGE == pool_base_stagger(id)`, and the same
  for `added_ticks_ptr()` and `changed_ticks_ptr()`. Nothing in the tree pins a pool's *actual*
  base offset against the stagger today — `scratch_band_stagger_slots_are_distinct` compares the
  pure function `pool_base_stagger(a)` vs `(b)` (`boyko_physics/src/scratch_ids.rs:857-866`), and
  the existing G5 pins pointer identity only (`component_pool.rs:3364-3373`). Since S1 rewrites
  `grow_rows`/`grow_rows_zst` and retargets every commit to `base_off − σ`, this is the one cheap
  pin that turns red if the stagger is ever lost from the base (a page-floor commit that also moved
  a base would show as `% COMMIT_PAGE == 0`). The identity holds on every arm: the reservation base
  is granule-aligned, `data_len`/`tick_len` are granule multiples, and `COMMIT_PAGE | COMMIT_GRANULE`.
* **G6 — Miri (fallback arm).** The pool module's existing suite under
  `cargo +nightly-x86_64-pc-windows-gnu miri test` (per the machine note: bare `+nightly` resolves
  to MSVC and dies in the linker): the layout/provenance proofs must still hold with `COMMIT_PAGE ==
  COMMIT_GRANULE` on that arm. Tree Borrows is the relevant rule for the tick-base derivations.
* **G7 — grow-event count.** For stride 40 growing to 1 M rows the number of `grow_rows` events is
  `≤ log2(1M·40 / COMMIT_PAGE) + 2 == 16`; a batch `reserve_capacity(n)` is exactly **one** event
  (request dominance). This is the gate that catches D3 being weakened.
* **G8 — property test.** `stride ∈ {1,2,4,8,16,40,64,256,1024,4096} × id ∈ 0..512 × n ∈ 1..10_000`:
  `committed_rows ≥ n`; frontier fields page-aligned; `committed_bytes()` equals the closed form of
  "The floor, derived" at `n = 1` and, for every `n`, the union of the three intervals the test
  recomputes itself from `(σ, stride, n)` via the D2 ladder; `row_ptr(n-1) + stride ≤ base + os_len`;
  no commit range exceeds `os_len`.
* **G9 — the workspace gates as written in CLAUDE.md**: `cargo check --workspace --all-targets`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --all-targets --no-fail-fast`. Both flags are load-bearing here: this
  change touches a constant seven crates' tests read transitively.

## Edge cases

* σ = 0 (1 id in 64): page floors coincide with sub-region offsets; every formula degenerates
  correctly, and G1 pins that the floor is *the same* 12 KiB, not smaller.
* `stride > COMMIT_PAGE − σ` (large components): data takes `ceil((σ+s)/P)` pages; sufficiency
  (obligation 1) still gives `rows ≥ 1` because the ladder is request-dominant.
* `stride < 4` (1–2 B components): ticks become the driver; floor 20–36 KiB (D6, accepted).
* ZST/tag pools: data region vacuous, `grow_rows_zst` unchanged in shape; floor 2 pages.
* `grow_rows(0)` / `n ≤ committed_rows`: unchanged idempotent no-op, zero syscalls (G2b).
* `reserve_rows` near `u32::MAX`, `reserve_rows == 0`, alignment > 4096: unchanged constructor
  rejects.
* Overflow: `pool_align_up_page` is the checked twin of `pool_align_up_granule`, same loud panic.
* Non-x86_64: `COMMIT_PAGE == COMMIT_GRANULE` — **today's quantum, D2's ladder**, which is *not*
  today's behaviour bit for bit. With `P = G` the ladder still commits from the absolute page
  floor: the first data commit is `[0, align_up_G(σ + stride)) = [0, G)` — one granule per
  sub-region, where today's `commit_subregion` (`component_pool.rs:567-568`) commits
  `[0, align_up_G(σ + 65536)) = [0, 2G)` for σ > 0. So that arm's floor is `3 · 64 KiB = 192 KiB`
  per tracked column, not 384 KiB, and the 180 KiB steady-state overshoot is removed there too.
  G1's `cfg`-selected expectation on that arm is therefore `3 · COMMIT_PAGE = 196 608 B` per
  column (`3 145 728 B` for the 16-column case) — the same *formula* as x86_64 with the arm's
  own `COMMIT_PAGE`, never a "keep the old number" special case.

## Residual owner VALUES/SCOPE (do NOT block S0–S2)

1. **Per-component change-detection opt-out** — `#[component(track = false)]` promoting today's
   `pub(crate) new_untracked` to a derive attribute. Worth **12 KiB → 4 KiB per column (3×)** for
   Gaia's pinned data tables and for any component never used in `Changed`/`Added`/`Mut`. Cost:
   `Changed<T>`/`Added<T>` on that type must become a compile error (a runtime debug-assert is not
   enough — the tick pages do not exist), i.e. a user-visible semantic. Owner call.
2. **Chunk-granular ticks** (Unity DOTS `ChangeVersion`): 8 B/row → ~0.06 B/row, **128× on the
   slope, 0× on the floor**. Listed only so it is not mistaken for this item; it changes
   `Changed<T>` from exact to conservative (false positives). Owner call, separate campaign.
3. **Is 12 KiB low enough?** If not, the only remaining lever is option (A)/(B) — shared pages —
   whose price is `pool_base_stagger`'s guarantee, false sharing between concurrently-scheduled
   writers, and (for B) relocation, i.e. re-opening SP4. The architect's recommendation is to stop
   at 12 KiB and take fork 1 if Gaia needs 4 KiB. Owner confirms the scope stop.

## Forks taken here (perf/architecture — not owner calls)

Quantum = `COMMIT_PAGE` with a cfg'd non-x86_64 fallback (D1); absolute page-floor ladder rather than
sub-region-relative with `align_up` (D2); ×2 not ×4 below the granule (D3); stagger untouched, span
constraint const-asserted (D4); one reservation per column, no sharing, no relocation (D5); ticks
stay two separate regions and the lockstep stays data-driven (D6); oracle = in-process model +
OS-truth arms, no shipped atomic (D7); `VmColumn`'s divisor requirement tightened to `COMMIT_PAGE`;
`INLAND_MIN_SLAB` left at 256 KiB.

## Open questions for the critic

1. `pool_commit_step`'s `POOL_MAX_SLAB` clamp now compares a frontier that includes the σ prefix
   (≤ 4032 B of skew against a 64 MiB clamp). Harmless by inspection; is it worth normalising, at
   the cost of a subtraction on the cold path?
2. G3's Linux arm parses `/proc/self/maps` (now `target_os = "linux"`-gated, with stated-reason
   skips on the other unix targets). Acceptable in the test suite, or should the Linux OS-truth
   arm be `#[ignore = "<class>: …"]`-classed and run only by the owner?
3. *(Resolved in pass 1.)* Rev 1 pinned the over-count as `os == model − P` at full capacity; the
   critique showed the sum model over-counts at up to two boundaries, geometry-dependently. The
   model is now the interval union (D7, obligation 10), exact everywhere with zero cold-path cost,
   so the alternative of clamping `added`'s ladder at `changed`'s page floor is no longer needed
   and is not taken.
## Critique log - pass 1 (2026-09-10)

Every finding was re-derived against the tree before acting on it (constants, fixture ids and
commit arithmetic re-read from `constants.rs`, `component_pool.rs`, `vm.rs`, `vm_column.rs`,
`core/log/ring.rs`, `boyko_physics/src/scratch_ids.rs`; no cargo run). Line numbers in the "finding"
column are rev 1's.

| # | finding (rev 1 line) | verdict | action taken in this revision |
|---|---|---|---|
| B1 | L351 obligation 10 / G3: the sum model `data_committed + 2·ticks_committed` over-counts at up to TWO region boundaries, geometry-dependently; the pinned `os == model − P` is red on some fixtures and green for the wrong reason on others | **CONFIRMED** by arithmetic: data cap `align_up_page(σ + data_len) = data_len + P` (D2 line 125 / obligation 2) vs `added`'s page floor `data_len`; default 40 B / 64 B pool (`pool_reserve_rows = 2^24`, `constants.rs:513`) hits both caps ⇒ `os == sum − 2P`; `make_stride64_pool(4096)` (id 226 ⇒ σ = 2176, `data_len = 262144`, `t_new = 20480 < 65536`) hits only the data cap ⇒ `os == sum − P` from the boundary rev 1 did NOT name; `pool_byte_layout(256, 16, σ)` hits neither ⇒ `os == sum` | **FIXED.** `committed_bytes()` redefined as the byte length of the UNION of the three intervals (D7, obligation 10, data-structures block); the `− P` clause and the full-capacity special case dropped; G3 is a plain equality in both phases and is required on all three geometries of the new obligation-10 table; D2's "one documented exception" sentence rewritten; open question 3 marked resolved (the `added`-clamp alternative is not taken — the union costs nothing on the cold path and changes no capacity) |
| N1 | L388 S0 names one red number (6 291 456) that the in-process model cannot produce on old code; G1's model yields 3 145 728 and only G3 sees 6 291 456; the 384 KiB figure needs ≥ 2-granule sub-regions | **CONFIRMED**: old fields are sub-region-relative granule-rounded (`component_pool.rs:197-201, 654, 673`; first step = `POOL_MIN_SLAB = 65536`, `constants.rs:348-349`) ⇒ model 3·65536/column; `commit_subregion` (`:567-568`) rounds each range to 2G ⇒ OS 6·65536/column; for `pool_byte_layout(256,16,σ)` the three rounded ranges are `[0,2G)`, `[G,3G)`, `[2G,4G)`, union 4G = `os_len` | **FIXED.** S0 now states the expected red per arm (G1: 3 145 728 vs 196 608; G3: 6 291 456 ≠ 3 145 728) and requires the 16-column fixture to be default-sized (`with_default_sizes`, `component_pool.rs:536`), naming the 4 194 304 figure a small geometry would produce instead |
| N2 | L491 "non-x86_64: today's behaviour bit for bit" is false in the plan's own favour: with `P = G` the D2 ladder commits `[0, G)` per sub-region, not today's `[0, 2G)` | **CONFIRMED**: D2 lines 123-128 with `P = G` give `needed = G`, `new_d = G`, `commit_page_region(0, 0, G)`; today's `commit_subregion(σ, 0, 65536)` gives `[0, align_up_G(σ + 65536)) = [0, 2G)` | **FIXED.** Edge-case bullet rewritten to "today's quantum, D2's ladder"; the arm's floor stated as 192 KiB and its G1 expectation as `3 · COMMIT_PAGE` per column (196 608; 3 145 728 for 16) — same formula, the arm's own constant |
| N3 | L164 D4's `POOL_STAGGER_LINES * CACHE_LINE_SIZE <= COMMIT_PAGE` is vacuous off x86_64, and S1's `stagger < 4096` → `< COMMIT_PAGE` LOOSENS the cache-set bound to < 65536 there; the sub-page constraint is an L1 set-index property, independent of the commit quantum | **CONFIRMED**: `constants.rs:160-166` (64 lines × 64 B = the 4 KiB set span), `:287-290` (the `< 4096` belt), plan 282-283 (`COMMIT_PAGE = COMMIT_GRANULE` off x86_64) | **FIXED.** New named constant `POOL_STAGGER_SPAN = POOL_STAGGER_LINES * CACHE_LINE_SIZE` (4096); `pool_byte_layout` keeps its bound at `stagger < POOL_STAGGER_SPAN`; the D4 assert becomes a SECOND, separate relation `POOL_STAGGER_SPAN <= COMMIT_PAGE`; D4 and S1 rewritten to say the quantum must be at least the cache-set bound and must never define it |
| N4 | L461 bar item 7b (a measurement that goes red if aliasing is reintroduced) is not answered by any test: `scratch_band_stagger_slots_are_distinct` compares the pure function, G5 pins pointer identity only, no `% 4096 == stagger` assertion exists in `crates/` | **CONFIRMED**: `scratch_ids.rs:857-866` (`pool_base_stagger(a) != pool_base_stagger(b)`), `component_pool.rs:3364-3373` (pointer identity only); a ripgrep for `(buffer_ptr\|added_ticks_ptr\|changed_ticks_ptr)() … (%\|&) (0xFFF\|4096\|COMMIT_GRANULE\|COMMIT_PAGE)` over `D:/wt/ecsnative/crates` returns no hit | **FIXED.** G5 extended: for `STRIDE64_ID = 226` (σ = 2176), `buffer_ptr() % COMMIT_PAGE == pool_base_stagger(id)` and the same for both tick bases, before the first grow and after the last; the identity is shown to hold on every arm |
| N5 | L415 S3 misses `ring.rs:11-12` ("single commit step" becomes false under a 4 KiB `POOL_MIN_SLAB`) and `vm_column.rs`'s module doc / Panics list / "granule chain" comment, which describe the divisor as `COMMIT_GRANULE` | **CONFIRMED**: `ring.rs:11-13, 69-82`; `vm_column.rs:32-45, 134-136, 485-500, 506`; `VmColumn::grow_to` with `POOL_MIN_SLAB = 4096` reaches `LINE_CAP · 16 = 65536` B in five doublings (4 → 8 → 16 → 32 → 64 KiB) | **FIXED.** All of them added to S3's list with the required rewording |
| N6 | L452 G3's Linux arm is named "Linux" but the tree's cfg arm is `unix` (`vm.rs:141, 234, 279`); `/proc/self/maps` does not exist on macOS ⇒ a non-Linux unix build would be a vacuous green | **CONFIRMED** against `vm.rs` (all three syscall arms are `#[cfg(all(not(miri), unix, not(windows)))]`) | **FIXED.** G3's maps walk gated `#[cfg(target_os = "linux")]`; the other unix targets carry a stated-reason skip at the site (`cfg_attr(..., ignore = "…")` or a `#[cfg]` with its reason, as G2 already requires); the "Windows vs Linux" section and open question 2 updated to match |

Reviser-noted while applying the above (not a critic finding, no new claim): G8's "`committed_bytes()`
equals the closed form of *The floor, derived*" was only meaningful at `n = 1`, since the closed form
is the floor; it now says so and, for every `n`, compares against the union the test recomputes from
`(σ, stride, n)` via the D2 ladder.

### Re-verification of pass 1 against the tree (second reviser run, 2026-09-10)

Every file:line the seven rows above cite was re-read on `46c8e489` before the row was accepted
(`constants.rs:1-7, 40-58, 99-103, 146-158, 160-166, 194-195, 274-335, 347-356, 505-520, 675`;
`component_pool.rs:197-201, 536, 560-573, 654-680, 3303-3373, 3472, 3632, 3889, 3951`;
`vm.rs:141, 201-204, 227-231, 234, 279`; `core/log/ring.rs:11-13, 69-84`; `vm_column.rs:32-45,
134-138, 485-508, 536, 715`; `boyko_physics/src/scratch_ids.rs:851-868`; `inland_store.rs:43, 396`;
`benches/d6_commit_vs_faults.rs:113, 162`; `docs/SYSTEMS.md:1898`; `docs/OPEN-QUESTIONS.md:295-304`;
`book/src/memory/arena.md:87`). The ripgrep of N4 was re-run over `D:/wt/ecsnative/crates` and
still returns no hit. All seven verdicts and actions stand. Corrections made in this run, each
arithmetic over the plan's own D2 ladder and `constants.rs`, none a measurement:

| # | what was wrong | action |
|---|---|---|
| R1 | Obligation 10 stated the data/added overlap is reached **iff** `align_up_page(σ + reserve_rows·stride) > data_len`. False as an iff: `new_d = (data_committed + step).min(cap)` with the in-band `step = data_committed` overshoots onto `cap = data_len + P` whenever `data_len` is not a ladder rung (`data_len = 3G`: `2G + 2G → min(4G, 3G + P)`), i.e. **before** full capacity and with `needed ≤ data_len`. The three table geometries hid it because `G`, `4G`, `2^30` are rungs | Condition rewritten as two routes; a fourth row (`pool_byte_layout(3072, 64, 2176)` at `n = 2015`, overlap from a doubling) added to the table and to G3's required fixtures |
| R2 | The table's default-pool row was the only both-caps fixture, and running it "at full capacity" commits a union of `2^30 + 2·2^26 + P = 1 207 963 648 B` at stride 64 (`805 310 464 B` at stride 40) inside a parallel test binary — unpriced by rev 2 | Cost stated (arithmetic); row 1 `#[ignore = "slow: …"]`-classed; a fifth row (`pool_byte_layout(16384, 4, σ)`, both caps at `3G + P = 200 704 B`) added as its cheap twin so the two-boundary case stays gated on every run |
| R3 | Row 1's "data cap" cell read `2^30 + P`, which is the stride-64 value only; the row is labelled 40 B / 64 B (`pool_reserve_rows(40) = 2^24` by clamp, `constants.rs:146-158`, `POOL_MAX_ROWS = 16 777 216`, `:85`) | Cell reads `data_len + P`, `2^30 + P` at stride 64 |
| R4 | S0's G3 red was given only as the 16-column aggregate, while G3 is specified on ONE pool's reservation | Per-pool numbers added (`393 216 ≠ 196 608`); the aggregate kept, conditioned on a summing fixture |
| R5 | `log/ring.rs` does not exist at that path; the file is `crates/boyko_ecs/src/ecs/core/log/ring.rs` | Path corrected in S1 and S3; N5's "five grow events" qualified as the one-line-at-a-time case (`grow_to` is request-dominant) |
| R6 | S2 asked the developer to "verify" `InlandStore` does not import `POOL_MIN_SLAB` | Verified here: `inland_store.rs:43` imports `INLAND_MIN_SLAB`/`INLAND_MAX_SLAB` only |
