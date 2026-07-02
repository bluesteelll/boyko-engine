> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase X.I — Results: ComponentPool Row-Capacity Growth

Branch `ecs`. Implementation `f9fb5a0` (W1-W3), test matrix `868d741` (W4-W5),
benches + this document (W6). Plan: [PHASE-XI-PLAN.md](PHASE-XI-PLAN.md)
(critic-APPROVED after R2); inventory: [PHASE-XI-RESEARCH.md](PHASE-XI-RESEARCH.md).

## What landed

The engine's LAST hard ceiling is gone. Every `ComponentPool` owns one
`VmReservation` laid out `[data | added_ticks | changed_ticks]` (granule-aligned
sub-regions, three write-once base pointers, one `committed_rows` warm oracle):

- **Eager reserve, zero commit** at construction — archetype creation no longer
  allocates or zeroes anything row-sized (the 6 × 256 KiB per-element-initialized
  tick Boxes and the arena carve are deleted).
- **`#[cold] grow_rows`** — slab doubling `[64 KiB … 64 MiB]`, request-dominant,
  ticks lockstep BY ROWS, idempotent no-op arm (★R1-1), frontier fields written
  only after the commits they describe. Zero `unsafe` inside; GROW1-XI proof in
  the plan, all five proof steps pinned by debug_asserts.
- **`add`/`add_typed`**: ONE warm compare (`len >= committed_rows`), the ceiling
  check lives inside the cold path; the per-mutation chunk-dirty
  `udiv + bounds-checked get_mut + store` is DELETED (chunk.rs machinery was
  written-and-never-read; the whole file is gone).
- **`Archetype::reserve_capacity`**: two-phase — read-only ceiling check
  (`Err` = archetype unchanged), then unconditional idempotent grows. The four
  deferred-apply PANICS became growth; they fire only at the reserve ceiling
  (2^24 rows on syscall arms — 16-256× past the old class ceilings).
- **Readers untouched by construction**: bases write-once ⇒ U6/U10, the Phase-7
  column cache, the X.B `row_ptr` identity, `for_each_chunk`'s single-slice
  contract all survive verbatim (gate XI-B1 below).
- **D2 constructor mapping**: `ComponentPool::new(_, _, n, m)` ⇒
  `reserve_rows = n × m` exactly — the ENTIRE pre-existing pin-test ledger
  (drop_fn, both in-file proptests, X.B identity/dense-equivalence, the dense
  bench) passed byte-unchanged. The mapping doubles as the small-ceiling test
  knob everywhere, including under Miri.

## Gates

### XI-B1 — 0%-gate on the hot paths

- **(a) random access: PASS.** All 147 stride-shift hot regions of the
  `random_access` bench asm are mnemonic+operand-shape identical to the W0
  baseline (the X.G protocol; pool fields are not on the path — D10).
- **(b) query iter: PASS.** All 72 hot regions identical.
- **(c) spawn suites vs the W0 `xibase` criterion baseline** (population (i),
  sources untouched):

| Bench | Δ vs W0 | Verdict |
|---|---|---|
| `component_ids_cached_lookup` (444 ps) | +0.3% (p=0.07) | flat |
| `cached_archetype_id_cached_lookup` (672 ps) | −0.6% | flat |
| `commands_spawn_enqueue_x1024` | **−30.5%** | improved |
| `spawn_command_apply_arity_4` (fresh world) | **−36.9%** | improved |
| `batch_10k_spawn_apply` | **−2.8%** | improved |
| `spawn_batch_10k_1comp` | **−60.8%** | improved |
| `spawn_batch_10k_3comp` | **−69.0%** | improved |
| `spawn_batch_direct_10k_1comp` | **−61.2%** | improved |
| `component_ids_static_pin` (1.11 ns) | 0.0% (p=0.81) | flat |
| `swap_remove/{100,1000,10000}` (lib suite) | **−55% / −51% / −27%** | improved |
| `ComponentPool::get_raw row_ptr recompute` (1.11 ns) | flat | flat |

  The spawn-side wins are the deleted per-row chunk-dirty work + the deleted
  first-fill tick memsets landing exactly where the plan predicted
  ("flat-to-better").

- **(c) population (ii) — re-baselined benches** (sources rewritten this phase,
  W0-incomparable by design): `spawn_command_apply_arity_4_x10k` — the
  fresh-world-per-iteration compromise is unwound (pools grow now, so 10,000
  spawn commands amortize one world): **76.7 ns per command** end-to-end
  (enqueue + apply + create_entity memcpy ×4 components).
  `query_init_state_50_archetypes` — the 8-archetype trim is unwound back to
  the §19.3-spec 50 archetypes (a 50-archetype world's pools are reserve-only
  now): **514 ns** per cold `init_state` scan.

- **(c) residual — `component_pool_dense` microbench (X.B), MISS, decomposed
  per the ★R1-6 protocol and accepted as documented:**
  - `add fill/{100,1000,10000}`: +184% / +46% / +24%. Attribution (model fits):
    the setup-built pool is now reserve-only, so the timed body pays the first
    grow event (1-3 commit syscalls) **plus the demand-zero soft faults** that
    the pre-X.I design paid in the UNTIMED setup (committed arena pages + tick
    Boxes pre-faulted at construction). Absolute deltas: +2.4 µs at n=100
    (≈3 syscalls + 3 page faults), +13.9 µs at n=10k (≈5 syscalls + ~60 faults).
    This is a measurement-boundary relocation, not a warm-path regression —
    the warm fill path at the lib level (`batch_10k_spawn_apply`,
    `spawn_batch_*`) IMPROVED, and the per-row inner loop is asm-flat.
  - `swap_remove/{1000,10000}`: stable **+0.9 ns/op** (+18%), triple-run
    reproduced. The new closure asm is SHORTER (79 vs 128 instructions, the
    chunk `div` pair and a `memcpy` call are gone), inline attributes are
    unchanged; the engine-level `swap_remove` suite (with entity bookkeeping)
    improved 27-55%. Working hypotheses for the microbench-only delta: 4K-page
    -offset aliasing between the granule-aligned tick sub-region bases and the
    data base (all ≡ 0 mod 4096 by construction vs. arbitrary heap offsets for
    the old Boxes), and/or iteration-boundary effects of the per-iteration
    fresh reservation. `swap_remove/100` additionally carries a fixed ≈+2.1 µs
    per-drain anomaly (+411%) not explained by the per-op term —
    attribution open. **Filed as a follow-up probe** (perf-counter run:
    `ld_blocks_partial.address_alias` would confirm or kill the 4K hypothesis).
    Accepted under the X.D precedent: user-visible paths win, the isolated
    microbench shifted its measurement boundary.

### XI-B2 — archetype creation: **PASS** (gate ≤ 25 µs, predict 2-5 µs)

- `archetype_create/3x192B`: **2.91 µs** — the D3 prediction (2-5 µs) dead
  center, ~50-130× under the pre-X.I analytic estimate (150-400 µs: the six
  256 KiB per-element-initialized tick Boxes + arena carve + chunk Vec, all
  deleted). 8.6× headroom against the gate.
- `archetype_create/8x4B`: 5.53 µs (8 pools ⇒ 8 reservations; ~0.7 µs/pool).

### XI-B3 — `EcsMaster::new`: **PASS** (gate ≤ 7.5 µs)

**5.93 µs** (X.H measured 6.84 µs — the pool-free constructor got cheaper
still). `Arena::new` 777 ns (unchanged ~751 ns; the arena is now a dead
reservation pending X.J).

### XI-B4 — growth event (gates ≤ 10 / 10 / 50 µs): **MISS as written — gate
was mis-derived; measured cost == the raw OS commit floor, accepted**

Ladder run (456/228/456 events per class, median / max):

| Step class | Median | Max | Gate |
|---|---|---|---|
| 64 KiB (3 syscalls, 192 KiB charge) | 15.2 µs | 54.3 µs | ≤ 10 µs ✗ |
| 2 MiB (3 syscalls, ~2.1 MiB charge) | 17.5 µs | 37.4 µs | ≤ 10 µs ✗ |
| 64 MiB (3 syscalls, **96 MiB** charge) | 105 µs | 204 µs | ≤ 50 µs ✗ |

Attribution (decisive, from the SAME run): the raw X.F `commit_slab` bench
measures the naked `VirtualAlloc(MEM_COMMIT)` floor on this machine at
2 MiB → 4.06 µs, 16 MiB → 17.0 µs, 64 MiB → 62.4 µs (≈ 1.0-1.6 µs/MiB charge
+ ~4 µs/syscall fixed). The pool event = **three** commits (data + both tick
sub-regions in lockstep): 64 KiB event ≈ 3 × syscall-floor ≈ 12-15 µs ✓;
64 MiB event commits 96 MiB total ≈ 96 × 1.0 + 3×4 ≈ 105 µs ✓ — the measured
medians match the OS floor to within noise, i.e. **the growth path adds
nothing measurable on top of the syscalls** (bookkeeping is ns-scale; zero
unsafe, zero allocation, zero copy). The plan's gate line took the X.F
single-commit envelope (≤ 50 µs @ 64 MiB) and forgot the ×1.5 tick lockstep
and the 3-syscall fixed floor — a model error in the gate, not a regression
in the code (the critic round missed it too). Re-derived honest envelope:
`commit_floor(bytes × 1.5) + 3 × syscall ≈ 15 / 18 / 105 µs` — which is what
was measured. Frequency context: ~26 events per pool LIFETIME; the worst
event amortizes to < 0.03 ns/row. The user-facing envelope gates are XI-B5/B6
(g7/g8 spikes), which bound what a frame can actually observe.

### XI-B5 — g7/g7b re-run: **PASS** (gate total ≥ 1.5× vs Bevy)

- **Total: 2.05×** — boyko 128.4 ms vs Bevy 262.9 ms (X.G measured 1.88×; the
  deleted archetype-creation tick memsets bought another ~9 ms exactly as the
  plan's (a) prediction modeled).
- **(b) attribution (binding): PASS** — boyko's per-iteration spike argmax is
  SCATTERED (mode = sub-batch #0 at only 44/196 iterations, range 0..944): no
  archetype-creation mode rises above the payload-fault floor anymore.
- **(c) composite spike honestly reported: 0.041×** — boyko 347 µs vs Bevy
  8.41 ms median-of-iteration-spikes (prediction band was 0.06-0.11×;
  measured better). Bevy's argmax is LOCKED at sub-batch #524 in 125/125
  iterations — its table-doubling memcpy; boyko has no such mode.

### XI-B6 — g8: 1,000,000 entities into ONE archetype: **PASS** (NEW headline)

Impossible before X.I (the medium-class ceiling was 65,536 rows). 3 components
× 192 B = 576 MB payload, 100 × 10k sub-batches, cold worlds, Bevy not
pre-reserved.

- **Total: 2.24×** — boyko **130.9 ms** vs Bevy **293.0 ms** (gate ≥ 1.5×;
  model band 1.7-2.1× — exceeded). The boyko total matches the plan's model
  (~131 ms) to the millisecond; Bevy lands just above its 222-272 ms band
  (its worst-case batch alignment realized: the final doubling copies the
  full ~300 MB table).
- **Worst-batch spike: 0.022×** — boyko **1.62 ms** vs Bevy **74.8 ms**
  median-of-iteration-spikes (gate ≤ 0.1×; model 0.01-0.05× ✓). Bevy's argmax
  sits at sub-batch **#64 in 125/125 iterations** — the 524,288 → 1,048,576
  table doubling, a ~288 MB memcpy stall of ~75-81 ms in ONE batch, every
  time. Boyko's argmax mode is sub-batch #0 (first-fill class, 102/196,
  range 0..64; raw max 3.6 ms = first commits + the `entity_ids` Vec residual)
  — **growth never copies a byte, so the stall class does not exist**.

A frame budget at 60 FPS is 16.7 ms: Bevy's unreserved 1M-entity ramp eats
4.5 frames in a single batch at the doubling; boyko's worst batch fits in a
tenth of a frame.

### XI-B7 — suites

- Full workspace: **1040 passed debug / 1024 passed release, 0 failed** (the
  debug-release delta = `#[cfg(debug_assertions)]` tests).
- Clippy `-D warnings`: clean. `cargo check --all-targets`: clean.
- **Miri (Tree Borrows)**: M-XI 5/5; churn controls `miri_phase8cd` 11/11,
  `miri_phase14a` 4/4, `miri_phase14b` 10/10, `miri_phase19` 9/9.
  `miri_phase_bugfix_56` skipped (documented pre-existing windows-gnu
  livelock). Flag note: 8cd/14a additionally need `-Zmiri-ignore-leaks`
  (#53-class deliberate leaks) — verified pre-existing at `fb7cf1e`; their
  suite headers omit it (doc drift filed).

## Test matrix delivered (W4)

U-P1 sizing/layout/step tables (both cfg arms, exact clamp edges) — U-P2
address-stability witness (4 base pointers + values across 3 slab commits) —
U-P3 ceiling exhaustion, zero state change — U-P4 tick lockstep + J-XI
(never-written form) at slab boundaries — U-P5 drop-count-exact across a
boundary — U-P6 `Tick::ZERO` transmute pin — U-P7 X.B pins re-run unchanged —
U-P8 grow_rows/reserve_capacity idempotence (★R1-1) — I-1 100k-row
single-archetype spawn through Commands — I-2 migration into a target growing
mid-apply — I-3 hook-deferred spawns into the SAME archetype at a slab
boundary (nested growth, no double-apply) — I-4 `for_each_chunk` single-slice
witness — I-5 `should_panic` pinning the SpawnAtCommand ceiling wording
(release-verified) — M-XI Miri suite (5 tests).

## Unplanned wins

1. **Miri suites two orders of magnitude faster.** The per-element-initialized
   256 KiB tick Boxes were catastrophic under the interpreter:
   `miri_phase8cd` 651.8 s → 6.4 s (**~102×**), `miri_phase14a` 970.7 s →
   2.6 s (**~378×**) interpretation time. Attribution proven by re-running the
   pre-X.I parent in a worktree.
2. **`benches/archetype.rs` is runnable again.** At the pre-X.I HEAD it
   panicked ("Arena reserve exhausted"): every iteration's fresh `Archetype`
   carved ~8 MiB of pools from one shared 64 MiB arena that never frees —
   dead archetypes LEAKED their pool memory until world death, by design.
   Post-X.I a pool's memory lives in its own reservation and is released when
   the archetype drops. This is an engine property, not just a bench fix.
3. **Spawn/despawn suites improved 27-69%** (table above) — the model
   under-counted the win from deleting the chunk-dirty per-mutation work and
   the first-fill tick memsets.

## Honest residuals

1. `component_pool_dense` microbench deltas (decomposed above; probe filed —
   4K-aliasing perf-counter run; `swap_remove/100` fixed +2.1 µs anomaly
   unattributed).
2. XI-B4 gates missed as written — measured == raw OS commit floor (decisive
   `commit_slab` corroboration in the same run); gate numbers were mis-derived
   in the plan (single-commit envelope, tick lockstep forgotten). Code adds
   nothing on top of the syscalls; envelope re-derived above.
3. The W5 Linux-native residual (mprotect arm never executed on real Linux)
   now covers vm.rs + arena.rs + the pool consumer — close via WSL/CI in one
   pass (pre-existing, X.C/X.G/X.H carry the same entry).
4. `Archetype::entity_ids: Vec<EntityId>` is the engine's last realloc-doubling
   container (~1-2 ms worst memcpy at 1M rows, inside g8's spike floor) —
   X.K candidate (InlandStore pattern).
5. Fallback-arm (wasm/Miri) ceilings for ≥128 B strides SHRINK (e.g. 192 B:
   21,845 vs 32,768) — documented loud-panic trade (★R1-3); demo strides are
   all ≤16 B and keep today's 262,144 ceiling exactly.
6. X.J backlog (filed): retire arena.rs/free_mem_block.rs + the `_arena`
   parameter chain; rename the legacy constructor params; delete dead
   `ChunkId`/`InlandChunkId` + master-era compaction constants
   (`COMPACTION_THRESHOLD` class); align miri_phase8cd/14a headers with the
   `-Zmiri-ignore-leaks` reality.

## Process notes

- The architecture-critic's R1 CRITICAL (grow_rows idempotence) was the real
  catch of the phase: without the no-op arm, every satisfied
  `reserve_capacity(1)` — i.e. every single-entity spawn command — would have
  committed a fresh doubling slab (memory explosion through the busiest funnel
  in the engine) and underflowed `needed − data_committed` in release. The
  critic also independently re-derived GROW1-XI and verified the D2 pin-test
  ledger claim against the actual test files before approving.
- The R2 confirmation round found the fold faithful and strengthened the
  proof: granule-aligned `data_committed` makes "clamped ⟺ fully committed",
  so corollary (a)'s clamped case is vacuous rather than separately argued.
- The dense-microbench triple-run + worktree asm A/B discipline (X.B law:
  criterion lies at ns scale; asm is the oracle) kept a stable-but-misleading
  +18% from being either ignored or panic-fixed: the code is provably shorter;
  the boundary moved.
