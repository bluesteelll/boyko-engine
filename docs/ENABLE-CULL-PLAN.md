# EnableTag positive-term archetype cull (task #5) — resolved plan

Branch `ecs`, 2026-06-16. Closes the deferred `cull_enable_archetypes` NO-OP
(state.rs:402). Produced by architect → 2 independent critics (correctness +
PROFILE-FIRST) and a measured diagnostic. This doc folds every CRITICAL/MAJOR
critique finding in as a resolved decision; it is the implementation spec.

## PROFILE-FIRST verdict: GO (measured, not assumed)

Mandate: no optimization ships without a measured win (the D6 precedent — a VM
pre-commit dropped after profiling). Diagnostic bench
`crates/boyko_ecs/benches/cull_diagnostic.rs` on the **current NO-OP build**,
gnu-1.96, same-binary A/B (no cross-commit drift):

| measurement | result |
|---|---|
| `cull_full` (M=64 archetypes, R=256 rows, K=4 with an `EtFlag` column) | ~34.0 µs |
| `cull_equiv` (only the K=4 with-column archetypes exist) | ~8.9 µs |
| **`(cull_full − cull_equiv)/cull_full`** | **73.8%** (both runs, 3 sig figs) |
| R-sweep (M=64,K=4) R∈{16,256,1024} | 2.5 / 34 / 135 µs — **~linear in R** |
| control `cull_all_have_column` (K=64) | ~144 µs — cull saves nothing (as designed) |

The auditor's worst case — LLVM collapsing the perfectly-predicted
`fetch.col.is_null() => continue` no-column row loop — **did not happen** at
gnu-1.96: the no-column scan is real and R-linear, so skipping those archetypes
is a genuine ~74% iter-time win in the sparse shape. Same-binary variance <0.4%
on the large benches ⇒ the signal is far above the box's ±13% cross-commit drift.
**GO.** The control proves we are not over-claiming (no free lunch when every
archetype has a column).

## Decision 1 — Model B (separate culled list), NOT Model A (mutate `matched_ids`)

`matched_ids` (the shared, term-agnostic archetypal-match cache) stays **untouched**
by the cull. The cull result lives in a separate per-`(D,F)` `culled_ids:
Vec<ArchetypeId>` recomputed wholesale from the full `matched_ids` on each
invalidation. This dissolves the orchestrator-flagged **re-add gap by
construction**: an archetype X that gained an `EtFlag` column later was never
removed from `matched_ids`, so the next recompute's presence check re-includes it.
Model A (physical removal + force-rebuild) is rejected — it would desync the
shared cache and reintroduce the unbounded re-scan the EnablePresence module
exists to prevent.

Storage on `QueryDataState<D,F>` (cold tail; the hot cursor copies `&D::State`/
`&F::State` out and never touches `QueryDataState`, so this is free on the hot
path — verified iter.rs:99-100):

```rust
pub(crate) struct EnableCull {
    culled_ids: Vec<ArchetypeId>,     // matched_ids minus enable-rejected archetypes
    last_observed_enable_epoch: u64,  // invalidation stamp — see Decision 4
}
```
`Vec::new()` is alloc-free, so for a non-enable `(D,F)` the field is a zero-cap Vec
constructed once and never read (every access is `const { HAS_ENABLE_TERM }`-gated).

## Decision 2 — the QueryFilter hook (additive default, no ABI break)

Add ONE defaulted method (mirrors the existing `aggregate_include {}` default at
filter.rs:174 → all 78 leaf impls + `Or`/too-large macros compile untouched):

```rust
/// Cull verdict for a positive-term enable query. Default: keep (no cull).
fn enable_cull_keeps_archetype(_state: &Self::State, _master: &ArchetypeMaster,
                               _arch: ArchetypeId) -> bool { true }
```
- `Enabled<T>`: override → keep iff the archetype is **present** for the tag:
  `master.enable_presence().contains(state.id, arch)`. (Use the O(1) presence
  oracle, NOT `Archetype::enable_column_ptr(id).is_null()` — the oracle avoids
  minting `&Archetype` and is the module's documented consumer.)
- `Disabled<T>`: **explicit `true` override** with a guard comment. A no-column
  archetype's rows are all "disabled" (amendment A1.1), so they MATCH `Disabled<A>`
  and MUST NOT be culled. Making it explicit (not inheriting the default) is the
  defensive choice now that the trait gains a method. Regression test
  `disabled_does_not_cull`.
- Tuple macro (filter.rs ~1090): **AND-compose** —
  `true $( && $F::enable_cull_keeps_archetype(&s.$i, master, arch) )*`. Conservative:
  drop an archetype only if some member proves it row-empty.
- `Or<…>`: leave default `true` (`Enabled`/`Disabled` are compile-rejected inside
  `Or` via the M1 OrComposable bound, so an Or never carries an enable term).
- Everything else (With/Without/Added/Changed/()): default `true`.

The cull only ever runs under `const { Self::HAS_ENABLE_TERM }` (= `F::CONTAINS_ENABLE_TERM`),
so non-enable monomorphizations never reference any of this (0%-gate).

## Decision 3 — the cull body + consumer routing (Model B)

`cull_enable_archetypes` becomes: recompute `culled_ids` from `matched_ids`.
Signature changes from `&mut QueryState` to `&QueryState` + `&mut Vec<ArchetypeId>`
(it never mutates `matched_ids`):

```rust
fn recull(matched: &[ArchetypeId], filter_state: &F::State,
          master: &ArchetypeMaster, out: &mut Vec<ArchetypeId>) {
    out.clear();
    out.extend(matched.iter().copied()
        .filter(|&a| F::enable_cull_keeps_archetype(filter_state, master, a)));
}
```
`Vec::extend` over a filtered `Copy` iterator (not push-in-loop). No new `unsafe`
(single-threaded `new`/`update`; reads the presence oracle, writes a plain Vec).

**Consumer routing — there are TWO `driver_ids` (W1, profile-auditor), not one
funnel.** Add `QueryDataState::enable_driver_ids() -> &[ArchetypeId]`:
```rust
#[inline] fn enable_driver_ids(&self) -> &[ArchetypeId] {
    if const { Self::HAS_ENABLE_TERM } {
        if const { Self::IS_CANDIDATE_SEEDED } { self.archetype_state.matched_ids_pre_terms() } // already culled by seed
        else { &self.enable_cull.culled_ids }                                                   // positive-term cull
    } else { self.archetype_state.matched_ids_pre_terms() }                                      // 0%-gate: identical load
}
```
Route BOTH `Query::driver_ids` (query.rs:211) and `QueryView::driver_ids`
(query_view.rs:302) through it. **Verify** `par_iter`/`par_chunk` source their ids
from `driver_ids` (not a direct `matched_ids_pre_terms()` that bypasses the cull);
the direct reads in par_chunk are the const-folded archetypal paths — confirm each
either routes through `enable_driver_ids` or is provably unreachable for an enable
`F`.

## Decision 4 — invalidation stamp = `EnablePresence::epoch()` (C2 fix)

The cull consults `EnablePresence::contains` (Acquire). Invalidate it off
`EnablePresence::epoch()` (Acquire, enable_presence.rs:196) — the module's
**purpose-built** invalidation stamp (documented enable_presence.rs:25-33) — NOT
`ArchetypeMaster::enable_generation()` (Relaxed). This pairs an Acquire trigger
with the Acquire oracle, correct in v1 (single-threaded apply-window) AND under the
future D7 worker-marking seam. `EnableCull.last_observed_enable_epoch` holds it.
(The candidate-seeded path keeps using `enable_generation` — it is a const-disjoint
path with its own blessed `snapshot_present` consumer; do not change it.)

## Decision 5 — the re-add invariant (C1 fix), stated precisely

The warm-only re-cull (state.rs:360 branch) is reached **iff both
`archetype_generation` and `structural_generation` are unchanged** ⇒ `matched_ids`
is identical to its value at the last full recompute ⇒ any archetype X that newly
satisfies the cull (gained an `EtFlag` column) was **already in `matched_ids`**
(the positive term `&D` is unaffected by enable toggles, and Model B never removes
from `matched_ids`). Therefore recompute-from-`matched_ids` re-adds X. Required
regression tests:
1. `cull_then_enable_readds`: new → cull (X dropped, no column) → `enable::<A>` an
   entity in X (no structural churn) → `update` → iterate → MUST see X's row.
2. `enable_into_new_archetype_interleaved`: create archetype Y AND `enable::<A>`
   into Y between two `update`s → Y appears in `culled_ids` and its row iterates
   (exercises the structural-rebuild branch, not the warm-only branch).
3. `new_populates_culled_ids`: a positive-term query's `culled_ids` is filled in
   `new` (not left empty until the first `update`).

## Decision 6 — count/is_empty consistency (W2)

Route `archetype_count`/`is_empty` (the no-terms arm at query.rs:168/189,
query_view.rs:338/358) through `enable_driver_ids()` too. Today both count and iter
read `matched_ids`; the cull would otherwise make `iter()` walk `culled_ids` (a
subset) while `archetype_count()` still returns `|matched_ids|` — a NEW divergence.
Routing them through the culled set keeps them consistent with `iter` and is the
more correct count for an enable query (excludes provably-empty archetypes).
Const-folds to `matched_ids_pre_terms()` for non-enable `F` (0%-gate intact).

## Decision 7 — dynamic `with_enabled`/`without_enabled` interaction (W4)

Invariant to document + test: the typed cull only removes archetypes where the
typed `Enabled<A>` term yields **zero rows** (no `A` column ⇒ every row fails
`Enabled<A>`). Iteration ANDs the typed term with any per-view dynamic term, so
removing a zero-typed-row archetype cannot drop a row any dynamic
`with_enabled(B)`/`without_enabled(B)` combination could have surfaced. For
`Disabled<A>` the cull is a no-op, so no interaction. Add a mixed test:
`Query<&D, Enabled<A>>.with_enabled(B)` / `.without_enabled(B)` over a culled world
matches the per-row oracle.

## Decision 8 — lifetime discipline (W3) + the `new` call site (W1)

- `culled_ids` follows the exact `matched_ids` borrow discipline: it is a plain
  `Vec` mutated only under `&mut QueryDataState` (`update`/recull); every `&'s`
  slice handed to a cursor is invalidated by that `&mut` per the borrow checker.
  **Do NOT** model it on `term_scratch` (which is `AtomicPtr`-published) — no
  interior mutability, no raw-pointer caching.
- In `new` (state.rs:216), `self` does not exist yet, so the cull cannot write
  `&mut self.enable_cull.culled_ids`. Build a `let mut culled_ids = Vec::new();`,
  recull into it, and move it into the `EnableCull` field in the struct literal
  (or construct `Self` with an empty `EnableCull` then call `self.recull(master)`).

## get/get_mut (point lookup) — unchanged

`QueryView::get`/`get_mut` keep the exact per-row bitset test (filter_enable.rs
`query_view_enable_passes`); they do NOT consult `culled_ids`. Add a test that
`get` on an entity in a no-column archetype returns `None` (per-row exact), proving
get and iter agree at the row level despite using different archetype sets.

## Soundness

No new `unsafe`. The recull is single-threaded (`new`/`update` at the apply-window
barrier), reads the Acquire presence oracle, writes a plain Vec. par_iter reads
`culled_ids` read-only after `update` completes (Phase-9 model: no concurrent
toggler during worker execution). QS1 (matched_ids ⇔ dedup bitset) is untouched
(Model B never mutates `matched_ids`).

## Gates

- **0%-gate (sacred):** `query_iter` / non-enable benches byte-identical (the cull,
  `enable_driver_ids`, count routing are all `const { HAS_ENABLE_TERM }`-gated → fold
  to today's `matched_ids_pre_terms()` for non-enable `F`). Verify with the existing
  `query_iter` criterion bench + asm spot-check.
- **Acceptance:** after implementation, `cull_full` must drop from ~34 µs toward
  ~`cull_equiv` (~9 µs); `cull_all_have_column` must NOT regress. Rename the
  diagnostic into the acceptance bench `query_iter_enabled_culled`.
- **Miri-TB:** the enable cull units (no new unsafe, but run the suite to confirm).
- **Tests:** the 3 re-add tests (Decision 5), `disabled_does_not_cull`, tuple
  `(With<P>,Enabled<A>)` culls, the dynamic-term mixed test (Decision 7), the
  get/iter-agree test, QS1 invariant after cull.
