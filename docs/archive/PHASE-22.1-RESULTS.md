> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 22.1 — Results

Zero-term-state query cursors (lock-free term prefilter) + ZST spawn-column
short-circuit. Commit `0f859f7` on branch `ecs`. Plan: `docs/PHASE-22.1-PLAN.md`.

## Why this phase existed

The Phase 22 (Tags) Wave-3 perf pass surfaced three residuals that violated the
phase's own 0%-gate / design promises:

1. **F-NEW-2** — `query_mut_iter_10k` **+27.8%** and `phase10_mut_deref_guard_1024_rows`
   **+8.8%** vs pre-Phase-22. A probe matrix proved *any* term state in
   `QueryIterMut::next` has a non-zero floor (+3.6% for a bare len-read) — a true
   0%-gate violation on a pre-existing path.
2. **F-NEW-3** — a single `with_tag` term on the `iter` driver cost **+49%** on
   10k single-archetype rows (**+0.23 ns/row**) — per-row, while plan D4 promised
   archetype-level.
3. **F-NEW-1** — `spawn_batch` of `2 data + 2 tags` ran **+42..52%** over
   `2 data` only; plan target was ≤ +10%.

## What landed

### Area A — zero-term-state cursors (the 0%-gate restoration)

`QueryIter` / `QueryIterMut` carry **no term state**; both `next()` bodies revert
to the byte-identical pre-Phase-22 shape (the measured floor *was* the term code
in the cursor, so only its absence reaches 0%). Dynamic `with_tag` / `without_tag`
terms now resolve **once per driver entry** into a per-state term-prefiltered
`&[ArchetypeId]` list; cursors walk a plain slice. The no-terms path never loads
the scratch (byte-for-byte pre-Phase-22 walk).

`term_list.rs` (new): immutable epoch-stamped `TermList`, lock-free **CAS
publication** + **mint-point reclamation** (`TermScratch` = two `AtomicPtr`),
protocol P1–P4. The cold/inline term-test asymmetry from Phase 22 F1
(`archetype_passes_tag_terms_inline_scan` / `term_scan_cold`) is deleted.

### Area B — ZST spawn-column short-circuit

`derive(Component)` emits `for_each_data_component_bytes` with a const
`size_of::<FieldTy>() != 0` filter applied **before** the macro's runtime sort, so
ZST tag columns fold out of the per-row byte-copy at monomorphisation. The spawn
loop consumes a per-batch compacted `data_pool_ids`. Tick stamping (every column,
incl. ZST — the `Added<Tag>` contract), hooks/observers, and the two-phase commit
are unchanged. The `2 data`-only path stays instruction-identical.

## Verification

### Soundness — multi-threaded Miri Tree-Borrows (authoritative oracle)

`tests/miri_phase22_1.rs` drives the **real** `resolve_term_filtered` /
`reclaim_retired` through `#[doc(hidden)] test_exports` shims (Phase-9.1 C1
discipline). **5/5 pass, no UB, no data race, with NO `-Zmiri-ignore-leaks`**
(the protocol's retire/reclaim is exact). Covers gate 11a (concurrent
first-resolve, single-publish CAS, loser frees own candidate), gate 11b
constrained (resolve → join → reclaim under the Phase-9 apply-window ordering —
the load-bearing case), steady-state concurrent fast-path readers, the
generation-change rebuild+reclaim arm, and the empty-filtered publish. The P2
reclaim-vs-read trust boundary (carried by Phase-9 invariants (a) a system is
never dispatched concurrently with itself + (b) epoch changes deferred to the
apply window) is documented executably, not asserted blindly.

`miri_schedule_parallel` (the parallel-executor path the reclaim funnel interacts
with) is TB + data-race clean with canonical `-Zmiri-ignore-leaks` (project
deliberate bounded leaks only) — no regression.

`loom_term_list.rs` is authored against the Phase-9.1 precedent (gate 11a + 11b
constrained/unconstrained) but **cannot be built on this Windows-GNU host**:
loom 0.7 pulls `tracing-subscriber -> windows-sys -> windows-result`, whose build
needs `dlltool.exe` (absent here — same wall the Phase-9.1 `boyko_threadpool`
loom build hits). Miri-TB carries the soundness claim in the interim (the plan's
documented fallback).

### Correctness

Full workspace suite green in **debug and release**; 76 Phase-22 integration
tests; `clippy --all-targets -- -D warnings` clean (debug). Code review (Opus)
found **zero static soundness bugs** and confirmed the implementation matches the
plan (cursor revert complete, QS1 invariant preserved, `Box::into_raw`-not-`Box`
per the 9.3c lesson).

### Performance — same-session git-stash A/B (drift-cancelled)

`pre22` and idle-machine vs-baseline runs were invalidated by machine drift
(controls moved +13..50%; the largest contaminant was an active Dota 2 process at
+30..50%). The trustworthy signal is a **back-to-back A/B**: `base221` (stash =
pre-22.1 code) vs `HEAD` (22.1), both measured in the same machine state seconds
apart so the drift cancels.

| Bench (HEAD vs pre-22.1) | Δ | Outcome |
|---|---|---|
| `query_mut_iter_10k` | **−18.9%** | F-NEW-2 fixed (restored to pre-Phase-22) |
| `phase10_mut_deref_guard_1024_rows` | **−10.4%** | F-NEW-2 fixed |
| `p22_query_iter_10k` one-term | **−33.1%** | F-NEW-3 fixed (per-row → archetype-level) |
| `p22_for_each_chunk_10k` one-term | −5.8% | improved |
| `p22_spawn_batch_10k_2data_2tags` | −7.3% | improved (Area B) |
| `query_ref_iter_10k` | +3.7% | within box noise (was −0.4% in F1) |
| `query_init_state_50_archetypes` | +24.8% | cold query-construction cost — see below |

Within-run (drift-robust) on HEAD: `p22_query_iter_10k` no-term vs one-term
**+1.2%** (was +49%) — the term cost is now archetype-level, row-count-independent.

## Known characteristics (documented, not bugs)

### Spawn `2d+2t` marginal cost (~+26% over `2d`) is feature-inherent, NOT tick-stamping

The user asked to push the spawn floor toward ≤10%. Investigation (this session):

- `commit_units` is **O(1) per pool** (`self.len += count`) — not a per-row cost.
- `fill_ticks` was rewritten to a vectorized two-region `slice::fill` (vs the
  per-row interleaved loop). Miri-TB clean (11/11), all tests green — but it moved
  the `2d → 2d+2t` within-run delta by **~0%** (+26.4% vs +27.8%). Tick-stamping
  is ~160 KB of vectorizable writes (~16 µs) against a ~144 µs delta, so it is not
  the bottleneck. The change was **reverted** (per the project's measurement-driven
  rule — no measurable benefit ⇒ no hot-path churn / no new unsafe pattern).
- The marginal cost is the VM-commit + growth of the **2 extra tick-only pools**
  per spawn — a direct consequence of decision **D1** (tags get real tick pools so
  `Added<Tag>` / `Changed<Tag>` work). It is the cost of the feature, not a
  regression of any pre-existing path (the `2d`-only spawn is unchanged), and the
  ≤10% target was aspirational. Reducing it further would require either breaking
  the `Added<Tag>` contract or a risky rewrite of the 0%-protected spawn/VM-commit
  path — deferred as optional future work (e.g. opt-out tick storage for tags that
  are never change-detected).

### `query_init_state_50_archetypes` +24.8% (cold)

Query-state construction now initializes a `TermScratch` (two `AtomicPtr`). This
is a **cold** path (once per query construction, ~550 ns absolute), not per-frame
or per-row. Acceptable; candidate for a future lazy-init if it ever matters.

## Process notes

- Both Area-A/B developer agents and the soundness tester **died mid-run on the
  weekly model cap** (Fable disabled this session; work continued on Opus). They
  secured their deliverables on disk (Write precedes the final report); the
  orchestrator finished forward — re-verifying every claim with the compiler /
  Miri / criterion directly, the only soundness oracles (Phase-9.1 lesson).
- **Two release-only bugs the dead Area-B dev left were caught and fixed** by the
  orchestrator: `E0425` (a `data_component_ids` reference outside its
  `#[cfg(debug_assertions)]` gate — invisible to debug `cargo check`, broke the
  release build) and an `unused component_id` in release (→ `_component_id`).
  Lesson reinforced: **run the release suite, not just debug** — `debug_assert!`
  bodies are still name-resolved in release.
- `W1`: registered `cfg(loom)` (`check-cfg` + `[target.'cfg(loom)'.dependencies]
  loom`) in `boyko_ecs/Cargo.toml`, mirroring the Phase-9.1 `boyko_threadpool`
  setup.

## Out of scope / follow-ups

- Spawn VM-commit/pool-growth optimization for ZST pools (the spawn floor) — only
  if it ever becomes load-bearing; high risk on the 0%-protected path.
- Pre-existing `ParamB` release-profile clippy dead-code in
  `events/parameters/parameters_buffer.rs` (unrelated to 22.1; debug clippy
  passes) — spawned as a separate task.
- loom execution once `dlltool.exe` is on the host PATH (or in CI on a
  full-MinGW image).
