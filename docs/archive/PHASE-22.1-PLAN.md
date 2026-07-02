All verification reads done — `get_param(state: &'s mut Self::State, ...)` confirmed (query.rs:451-455), `Bundle::for_each_component_bytes` trait method confirmed (bundle/bundle.rs:274), hand-written `Bundle` impls confirmed (self_bundle.rs:85,155; spawn_batch_command.rs:210 stub; Phase-19 hierarchy newtypes), spawn_batch closure at spawn_batch_command.rs:355.

---

# Architecture: Phase 22.1 — Tag-term prefilter + ZST spawn-column short-circuit (0%-gate restoration) — REVISED (round 2)

## Changelog vs round 1

| Remark | Disposition | Change |
|---|---|---|
| **C1 (CRITICAL)** | **Accepted** | The `UnsafeCell` + probe-then-escalate memo is **withdrawn**. Replaced by **immutable epoch lists with lock-free CAS publication + mint-point reclamation** (new D-B). No `&mut`-from-`UnsafeCell` exists anywhere in the design; the concurrent-first-resolve race (shared `&q`/`&view` across `scope.spawn`, both hitting rebuild) is handled by single-publish CAS; losers adopt the winner's list without waiting. `QueryDataState: Send + Sync` preserved (the new scratch is two `AtomicPtr`s — auto-`Send+Sync`; soundness carried by a written protocol P1–P4 with loom + multi-threaded Miri-TB proof). TS1 SAFETY argument fully rewritten. |
| **W1 (MAJOR)** | **Accepted** | The `bytes.is_empty()` const-fold premise is withdrawn — the derive codegen's runtime sort + single runtime-len dispatch loop (boyko_macros/lib.rs:1218-1262) launders the per-field const size. New D-E: macro-emitted `for_each_data_component_bytes` (const `size_of::<FieldTy>()` filter at array-build time, BEFORE the sort) + per-batch compacted `data_pool_ids`. Decision made in-plan, not gated. Chosen over the critic's suggested per-batch mask alone because the mask alone still leaves a per-column-per-row runtime branch (`len` is runtime post-sort), violating the 2d-only asm gate. |
| **W2 (MAJOR)** | **Accepted** | (a) Moot by construction under immutable publication — lists are written once while private, never mutated after publish; no `&`/`&mut` escalation exists. Stated explicitly. (b) Miri-TB gate extended to BOTH rebuild arms (terms-change, generation-change) plus a concurrent-first-resolve test. (c) Panic-mid-build cannot publish: the CAS site only ever sees a complete `Box<TermList>` returned by the constructor (type-structural; CAS is the last step). Pinned by a loser-path drop-count test + SAFETY comment; honest note that panic injection inside the build loop is not testable directly. |
| **O1 (MINOR)** | Accepted | `debug_assert!(state.generations_synced(master))` in the rebuild arm (accessors exist: state.rs:192-196). |
| **O2 (MINOR)** | Accepted | Thrash cost model documented in module doc + phase doc (alternating term sets on one slot = one build + one alloc/free per alternation; steering note: one term set per view per frame). |
| **O3 (MINOR)** | Kept | All verified positives preserved unchanged: cursor revert shape, D-C single funnel with per-driver re-gates, D-B's verified premises, D-D cold-path scoping. |

## Rejected remarks

None rejected. One nuance on W1: the critic offered "asm-probe the real expansion OR promote the per-batch mask to primary"; this plan promotes a **stronger** third option (macro-emitted filtered walk + compacted pool-id list) because the mask alone cannot meet the plan's own "no new branch in the data-column write loop" asm gate — justification inline in D-E.

---

## Goal

Restore the Phase 22 0%-gate and design promises that Wave 3 perf work proved violated:

1. **Zero term code in both row cursors.** `query_mut_iter_10k` +27.8%, `phase10_mut_deref_guard_1024_rows` +8.8% vs pre-22 (existing bench — genuine gate violation). The probe matrix proves any term state in `QueryIterMut::next` has a nonzero floor (+3.6% for a bare len-read with unreachable scan). Fix: cursors carry no term state; both `next()` bodies return to byte-identical pre-Phase-22 form.
2. **Archetype-level terms as plan D4 promised.** One `with_tag` on the iter driver = +49% on 10k single-archetype rows (+0.23 ns/row, per-ROW). Fix: terms applied once per view at driver entry (archetype-granular prefilter); term cost becomes row-count-independent.
3. **Tag columns ≤ +10% on `spawn_batch`** (currently +42..52% for 2 tags over 2 data; ~7-8 ns/e per ZST column). Fix: ZST columns excluded from the per-row byte-copy walk at compile time; tick stamping (`Added<Tag>`), hooks/observers behavior, and two-phase commit untouched.

## Context and constraints

- Affected: `iters/query/{iter.rs, tag_terms.rs, query.rs, query_view.rs, state.rs, chunk_iter.rs, par_iter.rs, par_chunk.rs}` + NEW `iters/query/term_list.rs`; `commands/spawn_batch_command.rs`; `bundle/bundle.rs` (one defaulted trait method); `boyko_macros/src/lib.rs` (one additional emission); `ecs_master.rs` (one line at the `query()` mint funnel). NOT affected: `query_state.rs`, `legacy_query.rs`, hook/observer dispatch, `EntityMaster`, pools.
- Invariants preserved: **QS1** (shared term-agnostic matched cache never mutated by terms), **Q5** (stale-id skip in cursors), **Q1/QD4** provenance, `Added<Tag>` semantics (ticks stamped for ALL columns incl. ZST), SBO* spawn-batch invariants incl. SBO9 panic behavior, the `_pre_terms` compiler-enforced funnel (strengthened, D-C).
- Sharp edges respected: 14a-F2 (no cached pointer written across reborrows — nothing here writes through a cached pointer; published lists are immutable), 9.1/9.3c TB-protector classes (no `&mut` ever minted over shared state; raw-pointer + atomic publication mirrors the 9.2/9.3c `NonNull`-not-`Box` lesson), Phase 12.5 cursor heap-residency discipline.
- Targets: `query_mut_iter_10k` / `phase10_mut_deref_guard_1024_rows` back to ±2% of pre-22; `query_ref_iter_10k` holds; term cost row-count-independent; spawn 2d+2t ≤ +10% over 2d; 2d-only spawn 0%; zero steady-state allocations on every path; the no-terms path never touches the scratch.

## Key decisions

### D-A: Per-state term-prefiltered id list; cursors walk a plain `&[ArchetypeId]`

**What**: `QueryDataState<D, F>` gains a 16-byte cold `TermScratch` (two `AtomicPtr<TermList>`: `current`, `retired`). Every iteration-style driver entry resolves its id slice once: no-terms → the shared `matched_ids_pre_terms()` slice exactly as pre-22 (scratch never loaded); terms → `resolve_term_filtered(...)` returns a memoised archetype-granular filtered slice. `QueryIter::new` / `QueryIterMut::new` take `ids: &'q [ArchetypeId]` and lose the `terms` field/param; `for_each_chunk_impl` / `par_for_each_chunk_impl` / `for_each_impl` replace `terms: &TagTerms` with `ids: &[ArchetypeId]` and delete per-transition term tests.

**Why**: fixes residuals 2 and 3 in one move — the measured floor IS the term code in the cursor, so only its absence reaches 0%; term cost moves to one O(matched) pass per epoch (not even per view — memoised). This is the Bevy/flecs shape (dynamic terms bake into a cached matched list; iterators never re-test). I-cache: deletes the cold/inline scan asymmetry and two `&*arch_ptr` probes from four driver loops.

**Alternatives rejected** (unchanged from round 1): per-cursor term state of any shape (probe-matrix-refuted, +3.6% floor); cursor-owned buffer built in `with_tag` (per-frame allocation, k-fold rebuild per chain); a distinct `TaggedQueryIter` type (duplicates both `next()` bodies + SAFETY proofs).

**Trade-off**: a small lock-free publication protocol with a written SAFETY contract (P1–P4 below) and loom + Miri-TB proof obligations; one heap allocation per **epoch change** (not per frame, not per view — see D-B allocation discipline).

### D-B (REWRITTEN): Immutable epoch lists; lock-free CAS publication; mint-point reclamation

**What**: a `TermList` is an immutable heap object `{ stamp_terms: TagTerms, stamp_arch_gen, stamp_struct_gen, ids: Box<[ArchetypeId]> }`, fully built while private, published once via `compare_exchange` on `TermScratch::current`, never mutated after. A superseded list is moved to `TermScratch::retired` (atomic swap) and freed only at the next **slot-exclusive mint funnel** (`Query`'s `get_param(&mut state)` — confirmed `&'s mut Self::State`, query.rs:452 — or `EcsMaster::query(&mut self)`).

**Definitions**:
- *Epoch* of a state slot = the triple (live-prefix terms, archetype_generation, structural_generation).
- *Owner value* = the single live `Query`/`QueryView` minted for the slot (verified: `EcsMaster::query(&mut self)`, `Query` non-`Clone`/`Copy`, `ParQuery{Mut}::for_each(self)` consuming, `with_tag(mut self)`).

**Protocol invariants (the SAFETY contract; each cited by the `unsafe` blocks):**
- **P1 — at most one successful publish per epoch.** All racing resolvers within an epoch loaded the same expected pointer (null or the same stale list); the first successful CAS changes `current`, so every other CAS fails. Racers cannot span epochs: an in-flight resolve holds a borrow of the owner; epoch change requires either `with_tag(mut self)` (owner moved — blocked by any live borrow, enforced cross-thread by `scope` borrow regions) or `&mut EcsMaster` (blocked by the view's world borrow; for systems, structural ops are deferred to the apply window, which the executor's existing Acquire/Release completion machinery orders after all system borrows end). Corollary: a CAS loser's `winner` pointer is same-epoch ⇒ stamps match ⇒ adopt without waiting (**lock-free: losers never spin**).
- **P2 — at most one retired list pending per slot; freed only under slot exclusivity.** One publish per epoch (P1) ⇒ one retire per epoch; every epoch change passes through a mint funnel (a new owner value is required to observe it, and both funnels are exclusive: `&mut state` / `&mut self`), and `reclaim_retired()` runs there first ⇒ the retired slot is empty when the next retire arrives. The reclaim point cannot overlap an in-flight resolve on the same slot (same system cannot be dispatched concurrently with itself; a live `QueryView` blocks `query(&mut self)`), so nobody can be reading the old list's header when it is freed. The `retired.swap(null)` additionally makes a hypothetical double-reclaim free `null` (defense in depth: leak, never double-free — `debug_assert` pins the impossible case).
- **P3 — publication completeness.** `TermList::build(...) -> Box<TermList>` returns a complete list; the CAS site only ever sees a finished `Box` (type-structural — you cannot publish what the constructor has not returned). Release on CAS success pairs with Acquire on every load ⇒ readers see fully-initialized contents. Panic mid-build unwinds before the CAS: `current` keeps its old (null/stale) value, next resolve simply retries. No half-built list is reachable.
- **P4 — slice lifetime.** `resolve_term_filtered(&'q self, ...) -> &'q [ArchetypeId]`: valid because (i) within an epoch the published pointer never changes after the single publish (P1), and (ii) freeing requires retire (epoch change — impossible while the owner is borrowed) followed by reclaim (mint-point exclusivity — impossible while `'q` is alive). No ABA: a pointer is freed only at reclaim points where no resolve is in flight, so no racer can hold a stale `expected` across a free/realloc.

**Resolve algorithm**: fast path = `current.load(Acquire)`; if non-null and `(*p).matches(terms, master)` (live-prefix `TagTerms::same` + two generation compares) → return slice. Slow path (`#[cold] #[inline(never)]`): `debug_assert!(state.generations_synced(master))` (O1); build candidate; `compare_exchange(stale, raw, Release, Acquire)`; on success retire the old pointer (`retired.swap(old, AcqRel)` if non-null) and return; on failure free own candidate (`Box::from_raw` — never published, sole ownership) and adopt the returned winner.

**Memory orderings** (each justified):

| Atomic op | Ordering | Why |
|---|---|---|
| `current.load` (fast path) | Acquire | pairs with publish Release ⇒ list contents visible |
| `current.compare_exchange` | Release (success) / Acquire (failure) | success publishes the build; failure returns the winner pointer ready to deref |
| `retired.swap` (winner) | AcqRel | RMW publishing ownership transfer to the reclaimer |
| `retired.swap(null)` (reclaim) | Acquire | pairs with the winner's Release half ⇒ safe `Box::from_raw` |
| `retired.load` (reclaim fast path) | Relaxed | null-check hint only; the swap re-validates |

The "last reader of old has finished before free" edge is NOT carried by these atomics — it is carried by the slot-exclusivity of the reclaim point (P2), which rests on the executor's existing synchronization (Phase 9 completion channel) and the borrow checker. Stated verbatim in the module SAFETY doc.

**Allocation discipline (principle 5)**: steady state (stamps match) = zero allocations, one Acquire load + ≤8 id compares + 2 gen compares per term-bearing driver entry, construction-time, off the row loop. Allocation happens only on epoch change (structural-rare) or term-set change. **O2 thrash model (documented)**: two term sets alternating on one slot rebuild per alternation (one O(matched) build + one alloc + one deferred free) — not a regression vs the shipped per-transition test, but module doc + phase doc steer users to one term set per view per frame. No-terms path: never loads the scratch.

**Alternatives rejected**:
- *`UnsafeCell` probe-then-escalate memo* (round 1) — C1: rebuild race reachable from safe code (`Query` auto-`Send+Sync`, `QueryView` hand-`Sync` at query_view.rs:126-145, `&self` driver entries, supported PAR1 `scope.spawn` sharing) ⇒ concurrent aliasing `&mut`. Withdrawn.
- *Drop `Sync`* — impossible: the SEND1 const gate (ecs_master.rs:1737) and the leaked `Box<UnsafeCell<QueryDataState>>` world cache require `Send+Sync`.
- *CAS-claim BUILDING sentinel + losers spin* — blocking-in-disguise on a path reachable from worker threads; duplicate cold builds by ≤(racers−1) once per epoch are cheaper and simpler than a wait protocol.
- *Eager resolve inside `with_tag(mut self)`* (exclusivity for free) — a k-term chain builds k times with k−1 wasted builds, and alternating chains thrash the memo per call; resolve-at-driver-entry with CAS keeps one build per epoch.
- *Free-on-swap (no retired slot)* — UAF: a same-epoch loser may be dereferencing the stale list's stamp header when the winner swaps. The single retired slot + mint-point reclaim closes it with bounded memory (≤1 pending list per slot; `Drop` frees both).

### D-C: All four drivers migrate to the prefilter (unchanged from round 1)

`chunk_iter::for_each_chunk_impl`, `par_chunk::par_for_each_chunk_impl`, `par_iter::for_each_impl` take `ids: &[ArchetypeId]` and lose their `archetype_passes_tag_terms` blocks; entries resolve once before dispatch; PAR7 fallback forwards the slice. Parallel drivers resolve on the calling thread BEFORE `pool.scope` (workers receive chunk descriptors, never call resolve — though if user code shares `&q` inside a scope and iterates, D-B handles it). ONE funnel: `matched_ids_pre_terms()` consumers shrink to resolve, `count_term_matched`/`any_term_matched`, cache maintenance, legacy, tests — strictly stronger than the D4 rename-sweep guarantee. Mandatory per-driver A/B re-gate (loop bodies untouched; only the slice source changes). Alternative (keep transition-test shape) rejected: two permanent term mechanisms, doubled invariant surface, zero benefit.

### D-D: `tag_terms.rs` reduced, not deleted (unchanged from round 1)

DELETE `archetype_passes_tag_terms_inline_scan` + `term_scan_cold` + the F1 asymmetry narrative. KEEP one `#[inline] archetype_passes_tag_terms` for: `TermList::build`'s per-id test, `QueryView::get`/`get_mut` (per-lookup on the in-hand archetype — random access cannot use a prefilter; unflagged), `count_term_matched`/`any_term_matched` (read-only, no scratch interaction). KEEP `TagTerms` as the per-view carrier; ADD `TagTerms::same` (live-prefix equality: len, polarity, `ids[..len]` — explicit, robust against future mutation paths even though trailing slots are currently always EMPTY).

### D-E (REWRITTEN): ZST spawn columns excluded from the per-row walk at compile time

**Where the waste is** (verified): spawn_batch_command.rs:355's per-row `bundle.for_each_component_bytes(...)` closure pays, per ZST column per row, the full dispatch — `pool_ids[canonical_idx]` load, `pool_at_unchecked_mut` deref chain, `row_ptr(idx)`, and `ptr::copy_nonoverlapping` with a **runtime** size from `component_layout` (component_pool.rs:1436-1442) — a dynamic-size memcpy LLVM cannot elide. ≈7-8 ns/e per ZST column.

**Why the round-1 guard fails (W1)**: the derive emission (boyko_macros/lib.rs:1218-1262) builds a runtime `[(ComponentId, *const u8, usize); N]`, sorts it AT RUNTIME, and dispatches through ONE loop with runtime len — `bytes.is_empty()` does not const-fold; it becomes a per-column-per-row runtime branch in the data-column loop, violating the 2d-only 0% asm gate. This is the canonical path for every derived bundle.

**What instead — two cooperating pieces**:

1. **Macro** (boyko_macros): emit an additional method `for_each_data_component_bytes` in `derive(Bundle)` — identical to the existing emission EXCEPT each field's array-build push is wrapped in `if core::mem::size_of::<#field_ty>() != 0 { ... }`. This branch sits in per-field straight-line code BEFORE the sort, where `size_of::<FieldTy>()` is a monomorphisation-time constant ⇒ folds to nothing: ZST entries never enter the array; the sort and dispatch loop run over data columns only. The existing `for_each_component_bytes` is **untouched** (spawn_at/insert/migration/full-walk consumers keep exact semantics — zero blast radius).
2. **Bundle trait** (bundle/bundle.rs:274 area): add `for_each_data_component_bytes` with a **default body** forwarding to `for_each_component_bytes` and skipping empty slices at runtime — correctness fallback for hand-written impls (self_bundle.rs:85/155, Phase-19 hierarchy newtypes, test stubs), which are off the gated bench (and the hierarchy newtypes are non-ZST anyway). The derive overrides it with the filtered emission. Same B2/B4 single-pass-inside-the-closure contract (the Phase-11/14b dangling-slice class stays impossible: bytes are consumed inside the closure frame, unchanged).
3. **spawn_batch_command.rs Step 5**: precompute once per batch a compacted `data_pool_ids: ArrayVec<_, MAX_BUNDLE_COLUMNS>` = canonical pool ids filtered by `layout.size() != 0` (≤N pool-layout reads, once per batch — negligible at 10k rows; promotion into Phase-12.5's `BundleColumnCache` is a developer option if it's already on this path, per-batch is the guaranteed-cheap fallback). The row loop calls `bundle.for_each_data_component_bytes(...)`; the closure indexes `data_pool_ids[k]`, `k += 1`. Alignment proof: derive's sorted order (by `ComponentId`) filtered by `size_of::<FieldTy>() == 0` ≡ canonical `pool_ids` order (sorted by `ComponentId`) filtered by `layout.size() == 0` — same key, same predicate, same type registry. Pinned by `debug_assert_eq!(k, data_pool_ids.len())` after each row (mirrors the existing spawn_at call-count assert shape).

**Why this meets the gates by construction**: ZST columns → zero per-row instructions (never visited). Data columns → instruction-identical per-row codegen (same indexed load from a small stack/cached array, same deref chain, same memcpy) ⇒ 2d-only 0%. For 2d+2t bundles, residual tag cost = `fill_ticks_batch` only (~1-2 ns/e, vectorised, REQUIRED for `Added<Tag>`) — comfortably ≤ +10%.

**What stays intact**: `commit_units_batch` over ALL pools (tag-pool `len` invariant), `fill_ticks_batch` over ALL pools (`Added<Tag>`), Step 7/8 bookkeeping, SBO9 panic behavior (the `ExactSizeIterator` pull precedes the walk, unchanged), hooks/observers (fired from structural-op sites, not from this closure — verified premise of the phase).

**Alternatives rejected**: call-site `bytes.is_empty()` guard (W1 — does not fold); size-0 early-return inside `write_at_unchecked_initialized` (runtime branch for every data column of every spawn path in the crate); per-batch mask + per-row branch without the macro change (leaves a runtime per-column branch in the data loop — fails the asm gate literally); column-major restructure (requires materializing the whole batch of bundles — memory cost, iterator-contract churn).

**Trade-off**: +1 defaulted trait method on `Bundle` (internal engine trait), one new macro emission cloning an existing verified shape; other ZST writers (spawn_at, migrations) keep the dynamic memcpy — same waste class, not gated here, filed as follow-up.

## Data structures

```rust
// iters/query/term_list.rs (NEW)
#[cfg(not(loom))] use core::sync::atomic::{AtomicPtr, Ordering};
#[cfg(loom)]      use loom::sync::atomic::{AtomicPtr, Ordering};

/// Immutable, heap-published, epoch-stamped filtered id list.
/// Built fully while private (P3); NEVER mutated after publication.
pub(crate) struct TermList {
    stamp_terms: TagTerms,                 // epoch fingerprint (live prefix)
    stamp_arch_gen: ArchetypeGeneration,   // names per state.rs accessors
    stamp_struct_gen: ArchetypeGeneration,
    ids: Box<[ArchetypeId]>,               // archetype-granular filtered matched ids
}

/// 16 B, cold tail of QueryDataState. Auto Send+Sync (two AtomicPtr);
/// soundness carried by protocol P1-P4 (module doc).
pub(crate) struct TermScratch {
    current: AtomicPtr<TermList>,          // null = never built; ONE publish per epoch (P1)
    retired: AtomicPtr<TermList>,          // <=1 pending; freed at mint funnels (P2)
}
// impl Drop for TermScratch: under &mut self (exclusive) — plain loads,
// Box::from_raw + drop for both pointers. Frees the bounded leak on teardown.
```

```rust
// state.rs — QueryDataState<D, F>: hot fields untouched; cold 16 B appended
pub struct QueryDataState<D: QueryData, F: QueryFilter> {
    pub(crate) archetype_state: QueryState,   // unchanged (hot)
    pub(crate) data_state: D::State,          // unchanged
    pub(crate) filter_state: F::State,        // unchanged
    term_scratch: TermScratch,                // NEW — cold; untouched by no-terms paths
    _marker: PhantomData<fn() -> (D, F)>,
}
```

```rust
// iter.rs — cursors revert to the pre-Phase-22 field set (terms DELETED)
pub struct QueryIter<'q, 's, D: QueryData, F: QueryFilter> {
    archetype_ids: std::slice::Iter<'q, ArchetypeId>,  // caller-supplied slice
    data_state: &'s D::State,
    filter_state: &'s F::State,
    world: UnsafeEcsCell<'q>,
    data_fetch: D::Fetch<'q>,
    filter_fetch: F::Fetch<'q>,
    current_row: usize,
    current_len: usize,
    meta: &'s SystemMeta,
    _marker: PhantomData<&'s ()>,
}
// QueryIterMut: identical delta.
```

## Public API

No public signature changes. Internal (`pub(crate)`) deltas:

```rust
// term_list.rs
impl TermScratch {
    /// Terms MUST be non-empty (debug_assert). Lock-free memoised resolve.
    pub(crate) fn resolve_term_filtered<'q>(
        &'q self, terms: &TagTerms, master: &ArchetypeMaster, state: &QueryState,
    ) -> &'q [ArchetypeId];
    /// Frees the retired list, if any. Called ONLY at slot-exclusive mint
    /// funnels: Query::get_param (&mut state) and EcsMaster::query (&mut self).
    /// Fast path: one Relaxed null-load + predicted branch.
    pub(crate) fn reclaim_retired(&self);
}

// tag_terms.rs
impl TagTerms { #[inline] pub(crate) fn same(&self, other: &TagTerms) -> bool; }

// state.rs
impl QueryState { #[inline] pub(crate) fn generations_synced(&self, master: &ArchetypeMaster) -> bool; }

// query.rs / query_view.rs (private helpers, mirrored)
#[inline] fn driver_ids(&self) -> &[ArchetypeId];   // terms.is_empty() -> pre_terms slice; else cold resolve
#[cold] #[inline(never)] fn driver_ids_term_slow(&self) -> &[ArchetypeId];

// iter.rs — terms param DELETED, ids param ADDED (QueryIterMut::new same)
pub(crate) unsafe fn QueryIter::new(state: &'s QueryDataState<D,F>, ids: &'q [ArchetypeId],
    world: UnsafeEcsCell<'q>, meta: &'s SystemMeta) -> Self;

// chunk_iter.rs / par_chunk.rs / par_iter.rs — terms -> ids: &[ArchetypeId]
// bundle/bundle.rs — defaulted; derive(Bundle) overrides with const-filtered emission
fn for_each_data_component_bytes<F: FnMut(ComponentId, &[u8])>(self, f: F) { /* forward + skip empty */ }
```

## Algorithms for critical paths

- **`driver_ids`** — O(1). No-terms: one predicted branch → pre_terms slice; no master mint, no scratch load. Term path (cold-outlined): master mint + resolve fast path (Acquire load + ≤8 id compares + 2 gen compares, one cache line). Construction-time only; `next()` bodies contain zero term code (asm oracle).
- **`resolve_term_filtered` slow path** — O(matched): per id one `get_archetype` slab lookup + ≤8 signature bit tests + push into a private Vec → `into_boxed_slice`; one CAS. Runs once per epoch; losers ≤(racers−1) duplicate cold builds once per epoch.
- **Cursor `next()`** — byte-identical pre-Phase-22 (inner row loop: guard + const-folded filter + fetch; outer: slice-iter next → `archetype_ptr(_mut)` None-skip → `set_table_*` → `entity_count`). Sequential D-cache; Phase 12.5 register-residency restored (the spill source — reachable scan call / term state — is gone).
- **Chunk/par distribution loops** — identical to pre-22 except the slice source.
- **Spawn-batch Step 5 row loop** — per row: bundle pull + per-FIELD straight-line closure over data columns only; ZST fields compile to nothing; data fields instruction-identical to today. `fill_ticks_batch` unchanged (vectorised across ALL pools).
- **`reclaim_retired` fast path** — one Relaxed null-load + branch per query param per system run / per view mint; off the row loop; covered by the 50-systems bench gate.

## Multithreading model

- **Shared**: published `TermList`s — immutable after publish (P3); read-only from any thread holding a borrow of the owner. `current`/`retired` — atomics only, orderings per the D-B table.
- **Mutation**: none on shared memory. Builds are thread-private; ownership transfers via CAS (publish), `retired.swap` (retire), `retired.swap(null)` + `Box::from_raw` (reclaim, slot-exclusive), `Drop` (exclusive).
- **Race freedom**: P1 (single publish per epoch, epoch frozen while resolves in flight), P2 (free only where no same-slot resolve can be in flight — scheduler exclusivity / `&mut` borrows; double-free structurally impossible, hypothetical violation degrades to a leak), P3 (Release/Acquire publication), P4 (slice lifetime). Concurrent first-resolve through shared `&q` in `scope.spawn` (the C1 scenario) is the designed-for case: racers build identical-content candidates; one publishes; losers free their own and adopt. **Lock-free**: no spinning, no blocking, every racer completes in bounded steps.
- **`Send`/`Sync`**: `TermScratch` auto-`Send+Sync` (AtomicPtr); `TermList` plain data. `QueryDataState`'s existing `Send+Sync` surface (SEND1 gate, world cache) intact with zero new hand-written `unsafe impl`.
- **Proof obligations (gated)**: loom test of the extracted protocol (real `resolve`/`reclaim` methods per the 9.1 C1 lesson — loom must drive REAL code, hence the `cfg(loom)` atomic aliases and the protocol living in its own module with injectable build content); multi-threaded Miri-TB (`-Zmiri-tree-borrows` + data-race detector) on the real Query path.

## Correctness / edge cases

- Terms matching zero archetypes → published list with empty `ids` (non-null ⇒ memoised; distinct from never-built null) → cursors yield nothing; `archetype_count`/`is_empty` agree (same membership semantics via the shared scan fn).
- Stale ids: excluded at build; cursor Q5 None-skip stays as defense and carries the no-terms path exactly as pre-22.
- Generations: frozen while the owner is borrowed (P1); across owners the stamps catch staleness (`EcsMaster::query` runs `state.update` first, then the first term driver rebuilds against fresh gens). O1 `debug_assert!(generations_synced)` in the rebuild arm catches any future entry point that forgets `update()` — the memo would otherwise persist a stale list until the next gen bump (stickier than the old transient per-transition staleness).
- Panic mid-build: unwinds before CAS; `current` unchanged; next resolve retries (P3). Panic between CAS and retire-swap: impossible region contains no panicking ops (pointer swap only) — noted in code.
- `>8` terms: unchanged loud panic at term-add. Archetype signatures immutable post-creation ⇒ build-time testing ≡ transition-time testing.
- Spawn: tag-only bundle → data walk visits nothing; commit + ticks + registration run (test pinned). Hand-written bundles use the default fallback (runtime skip — parity test pinned). Canonical alignment pinned by `debug_assert_eq!(k, data_pool_ids.len())`.
- Drop order: `TermScratch::drop` frees current + retired with the state slot; every borrowing cursor is bounded by the owner's lifetime (P4).

## Integration

- `query_state.rs`: ZERO changes. `state.rs`: +1 field, +`generations_synced`. NEW `term_list.rs` (protocol + module SAFETY doc + O2 thrash note).
- `iter.rs`: cursor revert; module doc F1 narrative → pointer to 22.1 design.
- `tag_terms.rs`: −2 fns, +`same`, module doc rewritten ("terms resolve once per epoch at driver entry; cursors are term-free").
- `query.rs`: 6 driver entries route through `driver_ids`; `get_param` gains `state.term_scratch.reclaim_retired()`. `query_view.rs`: mirrored; `get`/`get_mut`/`archetype_count`/`is_empty` untouched. `ecs_master.rs`: `query()` mint calls `reclaim_retired()`.
- `par_iter.rs`/`chunk_iter.rs`/`par_chunk.rs`: signature swap + term-block deletion; PAR7 fallback forwards the slice.
- `bundle/bundle.rs`: +1 defaulted method. `boyko_macros/lib.rs`: +1 emission (clone of 1218-1262 with the const size filter). `spawn_batch_command.rs`: per-batch `data_pool_ids` + filtered walk + asserts.
- Tests touching `QueryIter*::new(.., TagTerms::EMPTY)`: mechanical update to pass `matched_ids_pre_terms()`.

## Implementation plan (numbered, for the developer)

1. **tag_terms.rs**: add `TagTerms::same`; delete `archetype_passes_tag_terms_inline_scan` + `term_scan_cold`; fold the scan into one `#[inline] archetype_passes_tag_terms`; rewrite module doc.
2. **term_list.rs (NEW)**: `TermList` (+ `matches`, `build`), `TermScratch` (`resolve_term_filtered` fast path, `#[cold]` `rebuild_publish`, `reclaim_retired`, `Drop`), `cfg(loom)` atomic aliases, module SAFETY doc P1–P4 + ordering table + O2 thrash note. Every `unsafe` block cites its P-invariant.
3. **state.rs**: embed `term_scratch`; add `QueryState::generations_synced`.
4. **iter.rs**: revert both cursors to pre-22 (delete `terms` fields/term-test plumbing; restore `current_len = arch_ref.entity_count()`); `new()` gains `ids`, drops `terms`; update in-file tests; rewrite hot-loop module doc.
5. **query.rs**: `driver_ids`/`driver_ids_term_slow`; wire `iter`/`iter_mut`/`for_each_chunk`/`par_for_each_chunk`; add `reclaim_retired()` in `get_param`.
6. **query_view.rs**: mirror step 5; **ecs_master.rs**: `reclaim_retired()` at the `query()` mint.
7. **par_iter.rs / chunk_iter.rs / par_chunk.rs**: `ids` signature swap, delete term-test blocks, par entries resolve before `pool.scope`, PAR7 forwards the slice.
8. **bundle/bundle.rs + boyko_macros**: defaulted `for_each_data_component_bytes` + derive override with the const `size_of` filter at array-build time.
9. **spawn_batch_command.rs**: per-batch `data_pool_ids` precompute (check whether `BundleColumnCache` is on this path as the storage spot; per-batch stack ArrayVec otherwise); Step-5 row loop switches to the filtered walk; `debug_assert_eq!(k, data_pool_ids.len())`.
10. **Tests** (new): nested same-terms cursors (reuse, no rebuild); rebuild on terms change and on generation change (sequential views) — **both under Miri-TB** (W2b); concurrent first-resolve (2 threads share `&q` via scope, both `iter()`) — Miri-TB + data-race detector; loser drop-count test (winner's list adopted, loser's candidate freed exactly once — pins single-publish + no-half-publish observably); empty filtered result; semantics parity on the 19 `phase22_query_terms` tests; tag-only `spawn_batch` (commit + `Added<Tag>` tick); 2d+2t value/tick integrity; hand-written-bundle parity through the default method.
11. **loom**: loom dev-dep in boyko_ecs gated `cfg(loom)` (Phase 9.1 pattern); tests drive the REAL `resolve_term_filtered`/`reclaim_retired`: (a) two threads resolve-from-null (single publish, loser frees, both read same list); (b) resolve-stale + later reclaim interleave (no UAF/leak via drop counting).
12. **Gates** (tester): full matrix below; full suite; canonical Miri set + new tests.
13. **Docs**: `docs/PHASE-22.1-RESULTS.md`; Phase 22 plan addendum (D4 cost contract superseded: "terms resolve once per epoch per state slot; cursors term-free"); thrash-model user note.

## Metrics and validation (D6 gates)

| Gate | Criterion |
|---|---|
| `query_mut_iter_10k` | ±2% of saved pre-22 baseline (from +27.8%) |
| `phase10_mut_deref_guard_1024_rows` | ±2% of pre-22 (from +8.8%), 3-run reproducibility |
| `query_ref_iter_10k` | stays ±2% (no regression from F1's −0.4%) |
| term-on-iter (`with_tag`, 1 archetype) | ≈ no-term iter (≤ ~2%); cost delta CONSTANT between 1k and 10k rows (archetype-level proof) |
| chunk/par drivers, no-terms | unchanged vs shipped (re-gate after slice-source swap) |
| chunk driver with term | ≤ shipped +4.9% (expect improvement) |
| `p22_spawn_batch_10k` 2d+2t | ≤ +10% over 2d-only (from +42..52%) |
| `spawn_batch` 2d-only | 0% p-gated + asm: per-row data-column loop instruction-identical (modulo register allocation) — no new branch |
| 50-systems schedule bench | ±2% (covers the per-run `reclaim_retired` null-check) |
| asm oracle | `QueryIter::next` + `QueryIterMut::next` no-terms monomorphisations contain ZERO term code; diff vs saved pre-22 asm |
| loom | 2 protocol tests pass (all interleavings; drop-count exact) |
| Miri-TB | canonical set + reuse-nested + BOTH rebuild arms + concurrent-first-resolve, all clean |
| tests | 19× `phase22_query_terms` green; full suite; spawn parity/integrity set |
| debug_asserts | `!terms.is_empty()`; `generations_synced` in rebuild arm (O1); retired-slot-empty on retire (P2); `k == data_pool_ids.len()` per row; existing SBO/canonical asserts retained |

**Unsafe delta (honest count)**: term_list.rs ≈ 6 blocks (fast-path deref, winner-slice deref, loser `Box::from_raw`, reclaim `Box::from_raw`, 2× `Drop` frees) — all citing one written P1–P4 contract; zero new `unsafe impl Send/Sync`; macro-emitted code adds per-derive `from_raw_parts` sites of the SAME class as the existing emission (generated, not hand-written). Deleted: 2 cursor term probes + 3 driver term probes + 2 tag_terms fns. Net hand-written surface ≈ +1, concentrated in one module with a single protocol proof.

## Out of scope (explicit)

- `SpawnAtCommand` / migration-path ZST byte-copy skip (same waste class; not gated by a named bench; g4 variance needs multi-run methodology) — follow-up filing.
- `DynAdded(TagId)` term, enable-bits seam, `legacy_query.rs` — untouched per Phase 22 plan.
- Memoising `archetype_count`/`is_empty` through the scratch (cold; keeps count/any read-only).
- Any change to `query_state.rs` / the `_pre_terms` rename surface.
- Recycling `TermList` allocations across epochs (would require a reclamation epoch scheme; epoch changes are structural-rare and the thrash case is documented as an anti-pattern).

## Open questions

1. Confirm the saved pre-22 criterion baseline artifact names/locations so the tester pins the exact artifacts the perf pass used.
2. `BundleColumnCache` as the home for `data_pool_ids` (static per (B, archetype)) vs per-batch stack ArrayVec — developer verifies whether the cache is already minted on the spawn_batch path; per-batch is the guaranteed fallback (cost: ≤N pool-layout reads per batch).
3. loom dev-dep scope in boyko_ecs: if the `cfg(loom)` alias plumbing proves invasive beyond term_list.rs, fallback = extract the two-AtomicPtr protocol into `boyko_utils` with loom there; the ECS module then re-exports. Decision at dev time; the loom-drives-REAL-code requirement (9.1 lesson C1) is non-negotiable either way.

Relevant files: `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\{iter.rs, tag_terms.rs, query.rs, query_view.rs, state.rs, chunk_iter.rs, par_iter.rs, par_chunk.rs}`, NEW `...\iters\query\term_list.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\spawn_batch_command.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\bundle.rs`, `D:\claude\BoykoEngine\crates\boyko_macros\src\lib.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\app\ecs_master.rs`.

---

## Post-approval notes (critic round 2: APPROVED, 0 CRITICAL / 1 MAJOR / 3 MINOR)

Critique convergence: R1 REVISE (1C/2M/3m) -> R2 APPROVED (0C/1M/3m).

**MAJOR (VERIFIED-SOUND, not a code blocker - a proof-obligation flag for the tester):** VERIFIED-SOUND, not a blocker, but flag the residual proof risk in P2 (reclaim exclusivity). The free-at-mint-funnel argument for cross-thread safety rests on TWO project invariants outside this plan's code: (a) a system is never dispatched concurrently with itself, and (b) structural epoch changes are deferred to the apply window ordered after all system borrows end by the Phase 9 completion channel. get_param's `&'s mut Self::State` (verified query.rs:452) covers the single-instance sequential case via the borrow checker, but the cross-thread case is carried ONLY by those external invariants, NOT by the D-B ordering table (the plan states this verbatim — correct). Requirement already in plan: the multi-threaded Miri-TB + loom gates MUST drive the REAL resolve/reclaim through a harness that actually interleaves a worker-thread resolve against a dispatcher reclaim on the same slot; a loom test that only exercises two resolvers (gate 11a) does NOT exercise the P2 reclaim-vs-read race. Confirm gate 11b ('resolve-stale + later reclaim interleave') models a reader still holding the old pointer at the reclaim point, or the central P2 claim ships unproven (9.1-class false-green risk).

**MINOR (verification confirmations + one wording tighten):**

1. Parallel-path migration is sound but tighten one wording. Current par_iter dispatcher iterates matched_ids_pre_terms() and runs archetype_passes_tag_terms INSIDE pool.scope on the dispatcher thread (verified par_iter.rs:302/330), plus a duplicate term test on the PAR7 fallback (par_iter.rs:446). The plan says parallel drivers 'resolve before pool.scope' — correct, the resolve produces the &[ArchetypeId] the existing loop then walks, and both the scoped loop and the PAR7 fallback consume the same slice. Confirm the PAR7 fallback forwards the SAME resolved slice (not a second resolve) so the published list is fetched once per for_each, matching D-B's one-load-per-driver-entry cost model.

2. D-E alignment proof is structurally correct and verified at the source. The macro array-build push (lib.rs:1218-1262, the `#(#sort_entries),*` per-field expansion) precedes sort_unstable_by_key, so wrapping each field push in `if size_of::<FieldTy>() != 0` folds ZST entries out at monomorphisation before the sort — the const truly folds, unlike the withdrawn round-1 bytes.is_empty() guard which lands after the runtime sort. The data_pool_ids alignment claim (derive sorted-by-ComponentId filtered by size==0 == canonical pool_ids sorted-by-ComponentId filtered by layout.size()==0) holds because write_at_unchecked_initialized uses component_layout.size() (verified component_pool.rs:1437-1441) from the same registry the macro's size_of reflects. The debug_assert_eq!(k, data_pool_ids.len()) per row (mirrors existing canonical_idx assert at spawn_batch_command.rs:379) is the right pin. No action needed; recorded as verification.

3. Allocation-discipline and I-cache claims check out. Steady state (stamps match) = one Acquire load + <=8 id compares + 2 gen compares, off the row loop, zero alloc; no-terms path never loads the scratch (preserves the pre-22 matched_ids_pre_terms() slice walk byte-for-byte). Cursor next() bodies lose all term state — the +3.6% bare-len-read floor measured on the mut bench is eliminated only by this absence, which the asm-oracle gate pins. The deletion of archetype_passes_tag_terms_inline_scan + term_scan_cold (verified present tag_terms.rs:147-168) removes the cold/inline I-cache asymmetry from four loops. O2 thrash model (alternating term sets = one build+alloc/free per alternation) is documented as an anti-pattern, not a regression vs the shipped per-transition shape. Acceptable.

