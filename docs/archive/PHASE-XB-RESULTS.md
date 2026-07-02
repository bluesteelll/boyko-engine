# Phase X.B — Results: ComponentPool `Vec<Unit>` parallel-storage elimination

Branch `ecs`. PERF refactor of `crates/boyko_ecs/src/ecs/memory/component_pool.rs`. Full pipeline:
architect (audit + design) → architecture-critic (APPROVED) → developer → code-review (APPROVED) →
tester (correctness + Miri + bench). Contained, single-file change that **net-removes `unsafe`**.

## Status: COMPLETE — spawn measurably faster, iteration 0%, Miri-clean

### The change
Removed `ComponentPool.units: Vec<Unit>` (a per-row cache of `*mut u8`, each ALWAYS equal to
`buffer + i*stride`). Replaced with an explicit `len: usize` + a private
`#[inline] unsafe fn row_ptr(&self, i) -> *mut u8 { buffer.as_ptr().add(i * stride) }` computed from
the pool's **stable, write-once arena base**. The substitution `units[i].ptr() ≡ buffer.add(i*stride)`
is behavior-preserving — proven at every construction site (the architect+critic verified that even
`swap_remove`'s `units[idx] = Unit::new(removed_ptr)` was a self-assignment NO-OP, since
`removed_ptr == row_ptr(idx)`). Deleted `chunk_units` (zero callers crate-wide) and the `Unit` type /
`id_unit.rs` (Decision 3a). **All `pub`/`pub(crate)` method signatures unchanged** → every external
caller (bundle, migration_helpers, archetype, ecs_master) compiled with zero edits.

### Roadmap correction
The roadmap (and CLAUDE.md) described `Unit` as 24 B with a `buffer_index` field — **stale**. The
current `Unit` was already `#[repr(transparent)] { ptr: *mut u8 }` (8 B; `buffer_index` removed in a
prior audit). So the per-row memory saving is **8 B/row**, plus the removal of one `Vec<Unit>` heap
allocation per pool, plus the per-row `Unit`-write work. The honest gate is "spawn faster-or-equal +
iteration 0%," not the roadmap's stale "24 B / 5-10 ns" figures.

### Net-removes `unsafe`
This refactor DELETES `unsafe`: the `commit_units` raw-`ptr::write(Unit::new(..))`-into-Vec-spare-
capacity loop + `set_len` is gone (now just `self.len += count`); the `swap_remove` `Unit::new`
rewrite is gone. The only new `unsafe` is the private `row_ptr` (one `add` from the stable base) —
which consolidates arithmetic that was previously inline-`unsafe` at each `Unit`-producing site. The
Miri surface SHRANK.

## Verification gate (orchestrator/tester-run)

| Oracle | Result |
|--------|--------|
| **Correctness** | **17/17** pool tests: `dense_equivalence_after_swap_remove` (asserts `get_raw(i)` addr == `buffer_ptr()+i*stride` AND value == `Vec` oracle after every op), `proptest_pool_vs_vec_oracle` (random {add/swap_remove/pop/set_component} vs oracle), `drop_count_exactly_once` (live rows dropped once; `swap_remove_index_no_drop` drops zero) + the dev's 4 + 2 proptests |
| **Miri** (`-Zmiri-tree-borrows`) | **CLEAN** — `miri_phase8cd` 11/11 (commit_units/row_ptr path) + the pool-gate tests 9/9 (row_ptr addr-identity, swap_remove memcpy, no-drop variant, Drop 0..len). No UB in any `row_ptr` frame. |
| **Spawn (the win)** | **measurably FASTER** (rigorous git-stash A/B, p=0.00): isolated `add fill` **+88.8% @ n=100** (removed per-pool `Vec<Unit>` alloc), **+11.9% @ n=1000**, **+6.5% @ n=10000** (removed per-row `Unit` write); end-to-end `spawn_batch_10k` **+9.5%**. `row_ptr` recompute on read = 1.28 ns. |
| **Iteration 0%** | byte-identical path — `EcsMaster` random access (`column.ptr.add`) + query iter (`fetch.base.add`) never touch the pool's `units`; those files are absent from the diff. query_state_iter ~4.1 ns, random_access hot 3.18 ns — flat. |
| `cargo test -p boyko-ecs` | **503 lib + integration pass** (only the 2 known pre-existing trybuild `.stderr` drifts `bundle_compile_fail`/`compile_fail_chunk` fail — unrelated, not X.B) |
| `cargo clippy --all-targets -- -D warnings` | clean (after a needless_range_loop fix in the new multi-index test loops) |

## Soundness (preserved + improved)
- **Provenance**: every row pointer = `buffer.as_ptr().add(i*stride)`, one offset from the single
  `NonNull<u8>` arena allocation (no int→ptr). In-bounds: `i < max_components ⇒ i*stride+stride ≤
  capacity`. Canonical pattern (same as Bevy's `Column`/`BlobVec`). Miri-clean.
- **Stable base**: `buffer` is write-once in `new`, never reallocated (fixed arena capacity) — the
  same invariant the old stored `Unit.ptr` silently relied on.
- **Drop**: `for row in 0..self.len { drop_fn(row_ptr(row)) }` — each live row once, never the
  `[len, max_components)` uninit slots.
- **`len` is the single liveness authority**; `count() == Archetype::current_index` lock-step holds
  (every mutator adjusts `len` by the exact delta; `commit_units`' `debug_assert_eq!(start_row,
  self.len)` retained + retargeted). **ZST**: no new exposure (rejected at `new`; behavior identical
  to the old base-aliasing). `!Send/!Sync` unchanged (SEND10 comment text updated only).

## Pipeline notes
- Architect's audit (independently re-verified by the critic): `Unit` is pool-internal; the only
  escape `chunk_units` has ZERO callers; the substitution is behavior-preserving at every site.
- Code-review O1 (stale "Unit-pointer" prose in `migration_helpers.rs` docs) — fixed.
- Tester's 3 spec gates + the dev's tests + 2 proptests; the noisy spawn bench was resolved with a
  git-stash A/B on an isolated `add fill` bench (the conclusive p=0.00 signal).

## Files
- `crates/boyko_ecs/src/ecs/memory/component_pool.rs` — `units` field → `len` + `row_ptr`; all
  methods rewritten; `chunk_units` deleted; SEND10 + module docs updated; +3 gate tests/2 proptests.
- `crates/boyko_ecs/src/ecs/memory/id_unit.rs` — DELETED (the `Unit` type).
- `crates/boyko_ecs/src/ecs/memory/mod.rs` — `pub mod id_unit;` removed.
- `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs` — O1 doc-rot fix.
- `crates/boyko_ecs/benches/component_pool_dense.rs` (+ Cargo.toml `[[bench]]`) — isolated pool
  add/swap benches (the A/B harness).

## Follow-up
- **Internal-doc sync** (separate commit): `docs/SYSTEMS.md` (§2.5 Unit), `docs/FEATURE_MAP.md`
  (Direct-pointer-Unit section), `docs/ARCHITECTURE.md` (tree/decision/perf-table) still reference the
  deleted `id_unit.rs` / `Vec<Unit>` / `chunk_units` — synced post-landing.
- The non-Arena `EcsMaster::new` ~6 µs residual (X.C follow-up) overlaps X.D (EntityMaster slots) —
  the next phase.
