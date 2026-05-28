# Architecture: Phase X.A — `Query::for_each_chunk` and `Query::par_for_each_chunk`

---

## Changes from Rounds 1+2

### Round 1 patches

| Critic ID | Severity | Change |
|-----------|----------|--------|
| W1 | Important | §10.7 — collapsed two contradictory rows on `for_each_chunk` inline policy into the Phase 9 precedent shape: `#[inline]` on the public `Query::for_each_chunk` / `Query::par_for_each_chunk` methods (cross-crate visibility for closure inlining via LTO), no annotation on the internal `chunk_iter::for_each_chunk_impl` / `par_chunk::par_for_each_chunk_impl` drivers (LLVM decides). |
| W2 | Important | §2.4 + §1.2 — added an explicit callout that `par_for_each_chunk` invokes the closure once per archetype sub-range, not once per archetype, and that reductions need a thread-safe accumulator or `par_fold_chunks` (Phase 13.X). New §1.2 row quantifies the per-call frequency target. |
| W3 | Important | §8.1 — removed the workspace-root `rust-toolchain.toml` plan. Replaced with per-package `rust-toolchain.toml` at `crates/bench_bevy_vs_boyko/rust-toolchain.toml` (verified supported by rustup per-directory override semantics). Engine workspace stays stable-Rust clean. |
| W4 | Important | §1.2 — rewrote the "Allocations per frame" row to acknowledge the cold-path `Box<UnsafeCell<QueryDataState<D, F>>>` allocation on first use of a new `(D, F)` pair (same shape as Phase 12.6 direct API). Steady state still 0. |
| W5 | Important | §12 Step 1A — reordered the bullet list so `buffer_ptr_is_simd_aligned` is the **first** test the developer writes; this gates the entire wave. |
| N1 | Nitpick | §11.6 — added a one-line `cargo asm` size check for the `for_each_chunk_impl` dispatch body; the §1.2 ≤ 256 B L1i target is kept and now has a falsification step. |
| N2 | Nitpick | §5.2 — augmented the `Or<F>` blanket impl with a `// SAFETY:` comment explaining why `F: ArchetypalQueryFilter` is sufficient (concrete `Or<F>` impl is `Or<(F0, F1, …)>` and the tuple impl forces every element archetypal). |
| N3 | Nitpick | §6.2 — cited `component_registry.rs:47` (`MAX_COMPONENTS = 512`) and `archetype_bit_set.rs:7` (`MAX_ARCHETYPES = 1024`) so the worst-case arithmetic is reproducible. Numbers unchanged. |
| N4 | Nitpick | §11.2 — clarified the `aliasing_query_mut_t_mut_t_rejected.rs` shape: the test must declare a system fn taking `Query<(&mut T, &mut T), ()>` and register it in a `Schedule`; the direct `EcsMaster::query` API bypasses `FilteredAccessSet::init_access` (verified at `ecs_master.rs:1886-1939` per critic Round 1). |

### Round 2 patches

| Critic ID | Severity | Change |
|-----------|----------|--------|
| W2.1 | Important | §2.4 + §1.2 + §9.1 — fixed closure-frequency arithmetic; real formula is `worker_count × batches_per_thread` for medium-large archetypes (the per-worker shape dominates), and the `MIN_ARCHETYPE_FOR_PARALLEL = 1024` floor binds only for small archetypes where `entity_count / worker_count < 1024`. The Round 1 "100 invocations on 100k rows" example was off by ~10× — real answer is ~8 for that input on 8 workers. Accumulator-sizing guidance changed to `worker_count`, not invocation count. |
| W2.2 | Important | §12 Step 8A + §12-trailing "New files" list + §14 Q4 — propagated the §8.1 per-package toolchain decision to every downstream mention. Removed three workspace-root references. Step 8A now creates `crates/bench_bevy_vs_boyko/rust-toolchain.toml`, not the workspace-root file; the new-files list and §14 Q4 wording match. |
| N2.1 | Nitpick | §10.7 — extended the public-methods inline policy row to also cover the `QueryView::for_each_chunk` / `QueryView::par_for_each_chunk` direct-API mirrors. Single row, no duplication. |
| N2.2 | Nitpick | §13 Risk 4 — renamed the alignment-lift gating test from `simd_buffer_align_lift_holds` to `buffer_ptr_is_simd_aligned` to match the name the §12 Step 1A developer is writing right now in parallel. |
| N2.3 | Nitpick | §1.2 + §11.6 — dropped the spurious `wc -c ≤ 256 B` L1i budget. Textual `cargo asm` character count is not a sound proxy for encoded x86-64 instruction bytes (insn lengths vary 1-15 B). The check is kept but reframed qualitatively: inspect the dispatch body for the expected tight outer-loop shape. |

---

## §1 Goals

### 1.1 Functional goal

Add two new methods to the `Query<'w, 's, D, F>` SystemParam (and to the direct-API `QueryView<'w, D, F>`):

- `for_each_chunk<Func>(&mut self, f: Func)` — sequential, per-archetype slice closure.
- `par_for_each_chunk<Func>(&mut self, f: Func)` — parallel variant fanning archetype subranges across `boyko_threadpool` workers.

The closure receives **one contiguous columnar slice per matched archetype** (a per-element slice for tuple `D`), giving the user direct control of the inner loop. This is the flecs `ecs_query_next + ecs_field` shape — the only shape that production-ECS evidence (flecs + Unity DOTS) shows can sustain LLVM auto-vectorization of multi-row reductions in stock-language code.

### 1.2 Performance goals (target metrics)

| Metric | Target | Rationale / source |
|---|---|---|
| f32-sum reduction, 10k entities, single archetype, nightly bench with `f32::algebraic_add` | boyko ≥ **5×** Bevy `iter().fold(_, algebraic_add)` | orlp.net 21.6× on naive vs algebraic, Bevy still pays per-row state-machine cost. Floor 5× accounts for Bevy's `Iterator::fold` override (PR #6773). |
| `for_each_chunk` outer-loop overhead per archetype, 0 rows | ≤ **5 ns** (CMOV + integer compare + indirect jump) | Same shape as Phase 9 PAR9 inline path; no per-row work. |
| `for_each_chunk` outer-loop overhead per archetype, populated | ≤ **15 ns + 1 ns/row of user closure** when `F: ArchetypalFilter` and `D::NCD = false`. The "+1 ns/row" is the user's responsibility. | Const-fold of the per-row filter + tick branches; same as current `iter()` cost minus the `Iterator::next` state machine. |
| `par_for_each_chunk` dispatch overhead per archetype-chunk | ≤ **150 ns** (Phase 9 measured ~120 ns/spawn — see PAR9) | Re-use of `pool.scope` + identical `ChunkCaptures` shape. |
| `par_for_each_chunk` user-visible closure-invocation frequency | ≈ **`min(worker_count × batches_per_thread, entity_count / MIN_ARCHETYPE_FOR_PARALLEL)`** invocations per archetype (real formula at `par_iter.rs:117-123`: `batch_size = (entity_count / (worker_count × batches_per_thread)).clamp(MIN_ARCHETYPE_FOR_PARALLEL, usize::MAX)`; floor binds only when `entity_count / worker_count < MIN_ARCHETYPE_FOR_PARALLEL`). Reductions need an interior-mutable accumulator (`AtomicF32`, sharded TLS, or `par_fold_chunks` in Phase 13.X — see §2.4). Size the accumulator to `worker_count`, not to invocation count. | Sub-range granularity from Phase 9; one sequential `for_each_chunk` call would be one closure invocation per archetype. |
| Allocations per frame on hot path | **0 in steady state**; one `Box<UnsafeCell<QueryDataState<D, F>>>` per **new** `(D, F)` pair on first use (same shape as Phase 12.6 `EcsMaster::query` direct API; cached for the world's lifetime via `OnceLock<...>` in `query_state_cache`). | Phase 12.6 direct-API cost model; the chunked path reuses that cache verbatim — no new allocator hook. |
| L1d footprint of inner loop on the canonical bench | ≤ **40 KB** working set | 10k × 4 B f32 = 40 KB; fits comfortably. |
| L1i footprint of `for_each_chunk` per-archetype dispatch body | **Qualitative**: outer loop + one indirect call body must fit in a small number of cache lines, verified by inspection of `cargo +nightly asm` output in §11.6. No byte-count target (encoded x86-64 instruction lengths vary 1-15 B; textual asm character count is not a sound proxy). | Outer loop + one indirect call; no inlined fetch logic per element. |

### 1.3 What this phase does NOT do

- No alignment generic (`::<N>`, `Align16`, `Align32`). Bevy PR #6161 sank on this.
- No engine-side pre-padding to lane width.
- No `Vec3`-style guarantee that arbitrary `T` is per-row SIMD-aligned. Column-start alignment only (§6).
- No `iter_chunks() -> impl Iterator<Item = &[T]>` — the streaming-iter lifetime puzzle (research §4) is not worth solving when `for_each_chunk(FnMut)` covers the same use cases with measured Bevy-evidence of no perf gap (PR #6773 fold override).
- No `for_each_chunk_with_mask` (Option B in research §6). Filed Phase 13.X as opt-in.
- No support for `Changed<T>` / `Added<T>` / `Ref<T>` / `Mut<T>` filter+data inside `for_each_chunk` — gated out at compile time (§3, §7).

---

## §2 API surface

### 2.1 New trait — `ArchetypalQueryFilter`

Empty marker subtrait of `QueryFilter`, with manual impls for the four archetypal filters and a `for<F0..F11> for tuples and `Or<F>`.

```rust
/// Marker for [`QueryFilter`] impls whose decision is **archetype-level only**
/// — they cannot reject individual rows. Required as a bound on
/// [`Query::for_each_chunk`] / [`Query::par_for_each_chunk`] because the
/// chunk API yields one contiguous slice per archetype with no per-row gate.
///
/// # Membership
///
/// Stable members (Phase X.A):
/// * `()`, `With<C>`, `Without<C>`.
/// * `Or<F>` iff every element of `F` is `ArchetypalQueryFilter`.
/// * Tuples `(F0, F1, ..., Fn)` for `n ≤ 12` iff every element is
///   `ArchetypalQueryFilter`.
///
/// NOT members:
/// * `Added<C>`, `Changed<C>` — per-row tick comparison.
///
/// # Safety
///
/// Implementations MUST have `IS_ARCHETYPAL = true` AND
/// `NEEDS_CHANGE_DETECTION = false` at the filter level. The chunk API
/// relies on both to elide per-row work statically.
pub unsafe trait ArchetypalQueryFilter: QueryFilter {}
```

**Manual impls** (in `filter.rs`, alongside the existing per-filter blocks):

- `unsafe impl ArchetypalQueryFilter for () {}`
- `unsafe impl<C: Component> ArchetypalQueryFilter for With<C> {}`
- `unsafe impl<C: Component> ArchetypalQueryFilter for Without<C> {}`
- `unsafe impl<F: ArchetypalQueryFilter> ArchetypalQueryFilter for Or<F> {}` *(propagation via inner; Phase X.A also needs the recursive Or membership — see §5.2)*
- Tuples 1..=12 via a `impl_archetypal_filter_tuple!` macro symmetric to the existing tuple-AND impl.

### 2.2 New sub-trait — `ChunkedQueryData`

A sibling trait to `QueryData`, gated on the existing `QueryData` plus a new GAT.

```rust
/// Per-row [`QueryData`] augmented with a per-archetype-slice fetch path.
///
/// `Query<D, F>::for_each_chunk` requires `D: ChunkedQueryData` so the
/// closure can receive `D::ChunkItem<'c>` (typically `&'c [T]`, `&'c mut [T]`,
/// or a tuple of element chunk items).
///
/// # Why a sibling trait instead of extending `QueryData`
///
/// Extending `QueryData` with `type ChunkItem<'c>` + `unsafe fn fetch_chunk`
/// would touch every existing impl (78 impls per Phase 10 expansion: 8
/// leaves × {readonly,mut,no-meta} variants + 12 arity tuples + 12 too-large
/// stubs). A sibling trait keeps every existing impl untouched and lets
/// custom `QueryData` types opt in to the chunked API by adding a single
/// extra impl block when (and only when) chunked iteration makes sense.
///
/// # Members (Phase X.A)
///
/// * `&T` for `T: Component` — `ChunkItem<'c> = &'c [T]`.
/// * `&mut T` for `T: Component` — `ChunkItem<'c> = &'c mut [T]`.
/// * `()` — `ChunkItem<'c> = ()` (no payload; useful for entity-only chunks).
/// * Tuples 1..=12 — `ChunkItem<'c> = (D0::ChunkItem<'c>, ...)`.
///
/// NON-members (deliberate):
/// * `Ref<'_, T>`, `Mut<'_, T>` — these expose per-row tick state; a slice
///   can't carry one tick per row without doubling the GAT surface. Filed
///   Phase 13.X for a `ChunkedTickedQueryData` variant if needed.
///
/// # Safety
///
/// Implementations MUST uphold:
///
/// 1. **CD1** — `set_chunk_readonly` / `set_chunk_mut` initialize a
///    `ChunkFetch<'c>` whose cached column bases are valid for the
///    full row range `[0, archetype.entity_count())`. Same contract as
///    `QueryData::set_table_*` but yielding columnar bases instead of
///    per-row scalar bases.
/// 2. **CD2** — `fetch_chunk(fetch, start, len)` returns a `ChunkItem<'c>`
///    whose constituent slices have length `len` and span rows
///    `[start, start + len)`. Caller is responsible for `start + len ≤
///    archetype.entity_count()`.
/// 3. **CD3** — for `ChunkItem<'c> = &'c mut [T]`, distinct invocations of
///    `fetch_chunk` with disjoint `[start, start + len)` ranges produce
///    non-aliasing slices. Caller enforces disjointness.
/// 4. **CD4** — split readonly/mut mirrors `QueryData::set_table_*`. For
///    types containing `&mut T`, `set_chunk_readonly` is forbidden.
pub unsafe trait ChunkedQueryData: QueryData {
    /// Per-chunk fetch scratch — typically the column bases. Distinct from
    /// `QueryData::Fetch<'w>` only in lifetime parameter (lifetime is `'c`,
    /// the closure-body scope). For most impls this is a `Copy` struct with
    /// the same fields as `QueryData::Fetch`.
    type ChunkFetch<'c>: Copy;

    /// Per-chunk yielded item. For `&T`: `&'c [T]`. For `&mut T`: `&'c mut [T]`.
    /// For tuples: a tuple of element chunk items.
    type ChunkItem<'c>;

    /// Build a `ChunkFetch` with all-NULL column bases. Paired with
    /// `set_chunk_readonly` / `set_chunk_mut`. Same shape as `QueryData::init_fetch`.
    fn init_chunk_fetch<'c>(state: &Self::State) -> Self::ChunkFetch<'c>;

    /// Refresh the chunk fetch from a read-only archetype pointer.
    ///
    /// # Safety (CD1, CD4)
    ///
    /// * `archetype` must be a live `*const Archetype` for `'c` with read-only
    ///   provenance (from `UnsafeEcsCell::archetype_ptr`).
    /// * `archetype` must contain every `ComponentId` in `state`.
    /// * For `D` containing `&mut T`, MUST NOT be called; impls panic as
    ///   per QD4 backstop.
    unsafe fn set_chunk_readonly<'c>(
        fetch: &mut Self::ChunkFetch<'c>,
        state: &Self::State,
        archetype: *const Archetype,
    );

    /// Refresh the chunk fetch from a write-capable archetype pointer.
    ///
    /// # Safety (CD1, CD4)
    ///
    /// * `archetype` must be a live `*mut Archetype` for `'c` with
    ///   write-capable provenance (from `UnsafeEcsCell::archetype_ptr_mut`).
    /// * `archetype` must contain every `ComponentId` in `state`.
    unsafe fn set_chunk_mut<'c>(
        fetch: &mut Self::ChunkFetch<'c>,
        state: &Self::State,
        archetype: *mut Archetype,
    );

    /// Materialize the chunk item for rows `[start, start + len)`.
    ///
    /// # Safety (CD2, CD3)
    ///
    /// * `set_chunk_*` must have been called for the current archetype.
    /// * `start + len ≤ archetype.entity_count()` at call time.
    /// * For `ChunkItem` containing `&'c mut [T]`, the caller must ensure no
    ///   sibling invocation references an overlapping row range.
    unsafe fn fetch_chunk<'c>(
        fetch: &Self::ChunkFetch<'c>,
        start: usize,
        len: usize,
    ) -> Self::ChunkItem<'c>;
}
```

**Note**: No `_no_meta` variants. `NEEDS_CHANGE_DETECTION` is irrelevant for `ChunkedQueryData` impls in Phase X.A because the trait excludes `Ref<T>`/`Mut<T>` by design (see §7). The dispatcher in `query.rs` simply does not need the const-fold.

### 2.3 `Query::for_each_chunk` signature

```rust
impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    /// Invoke `f` once per matched archetype, passing a slice (or tuple of
    /// slices) covering every row in that archetype.
    ///
    /// `D` must satisfy [`ChunkedQueryData`]; `F` must satisfy
    /// [`ArchetypalQueryFilter`]. Both bounds are **compile-time** —
    /// `Query<&T, Changed<U>>::for_each_chunk` is a type error, redirecting
    /// the user to the per-row `iter()` API which handles tick filtering.
    ///
    /// # Closure lifetime
    ///
    /// `for<'c> FnMut(D::ChunkItem<'c>)`: the slice lifetime `'c` is fresh
    /// for each invocation, scoped to the closure body. The fresh borrow
    /// shape lets the user re-borrow `&mut [T]` slices across calls; the
    /// `&mut self` on `for_each_chunk` plus the per-archetype disjoint
    /// memory regions (archetype invariant) make this sound.
    ///
    /// # Skipped archetypes
    ///
    /// Archetypes with 0 entities are skipped (no closure invocation).
    /// Stale archetype IDs (Q5: archetype removed mid-iter) are likewise
    /// skipped — same `archetype_ptr_mut(_)? None` continue branch as
    /// `iter_mut`.
    pub fn for_each_chunk<Func>(&mut self, f: Func)
    where
        D: ChunkedQueryData,
        F: ArchetypalQueryFilter,
        Func: for<'c> FnMut(D::ChunkItem<'c>);
}
```

### 2.4 `Query::par_for_each_chunk` signature

```rust
impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    /// Parallel variant of [`Self::for_each_chunk`]. Splits each archetype
    /// into row sub-ranges according to [`BatchingStrategy`] and dispatches
    /// each chunk to a `boyko_threadpool` worker via `ThreadPool::scope`.
    ///
    /// Same compile-time bounds as `for_each_chunk` plus `Func: Fn + Send +
    /// Sync` for cross-worker invocation. PAR7 fallback (no active pool →
    /// sequential walk on the calling thread) preserved.
    ///
    /// # Closure invocation frequency (IMPORTANT — differs from sequential)
    ///
    /// The closure is invoked **once per archetype sub-range, not once per
    /// archetype**. The exact count is derived from
    /// `BatchingStrategy::chunk_size` (`par_iter.rs:117-123`):
    /// `batch_size = (entity_count / (worker_count × batches_per_thread))
    /// .clamp(MIN_ARCHETYPE_FOR_PARALLEL, usize::MAX)`. Two regimes follow:
    ///
    /// * **Medium-large archetypes** (`entity_count / worker_count ≥
    ///   MIN_ARCHETYPE_FOR_PARALLEL`): the per-worker shape dominates.
    ///   Invocations ≈ `worker_count × batches_per_thread`. Example: a
    ///   100k-row archetype on an 8-worker pool with default
    ///   `batches_per_thread = 1` → `batch_size = 12500`, invocations
    ///   = `100000 / 12500 = 8`.
    ///
    /// * **Small archetypes** (`entity_count / worker_count <
    ///   MIN_ARCHETYPE_FOR_PARALLEL = 1024`): the floor binds.
    ///   Invocations ≈ `entity_count / 1024`. Example: a 4096-row
    ///   archetype on the same 8-worker pool → raw `4096 / 8 = 512`
    ///   clamps to 1024, invocations = `4096 / 1024 = 4`.
    ///
    /// The sequential `for_each_chunk` would yield exactly **one**
    /// invocation with a full `entity_count`-row slice in either case.
    ///
    /// For reductions, the sequential `FnMut(&mut acc, &[T])` capture pattern
    /// does NOT translate. Use a thread-safe accumulator — `[AtomicF32; N]`,
    /// a sharded thread-local, or wait for `par_fold_chunks` (Phase 13.X,
    /// see §2.6) which adds the parallel reducing variant with explicit
    /// identity + combine semantics. Size the accumulator to `worker_count`,
    /// not to invocation count.
    ///
    /// # Granularity
    ///
    /// Per-archetype-subrange (matches existing `par_iter` shape from Phase 9).
    /// Tiny archetypes (`entity_count < MIN_ARCHETYPE_FOR_PARALLEL`) process
    /// inline, paying no dispatch tax. See §9.
    pub fn par_for_each_chunk<Func>(&mut self, f: Func, batching: BatchingStrategy)
    where
        D: ChunkedQueryData,
        F: ArchetypalQueryFilter,
        Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync;
}
```

### 2.5 `QueryView::for_each_chunk` / `QueryView::par_for_each_chunk`

Mirror the inherent methods of `Query`. The cache plumbing is identical to `QueryView::iter` / `QueryView::par_iter` (already wired in `query_view.rs`). The change-detection guard at `query_change_detection_panic` (line 2027 of `ecs_master.rs`) is **subsumed** by the new bounds: `F: ArchetypalQueryFilter` excludes `Changed`/`Added`; `D: ChunkedQueryData` excludes `Ref`/`Mut`. No additional runtime check needed.

### 2.6 Decision: `fold_chunks` (reducing variant) — **DEFER to Phase 13.X**

**Decision**: Ship only `for_each_chunk` and `par_for_each_chunk` in Phase X.A.

**Rationale**:
1. `for_each_chunk(|s: &[T]| acc.add_assign(s.iter().fold(0.0, f32::algebraic_add)))` with a captured `acc: &mut f32` already covers the reduction case at the cost of a moved-into-closure mutable ref. This is the same shape as Bevy's `Iterator::fold` override.
2. A `fold_chunks<B, Func>(init: B, f: Func) -> B` form requires designing a parallel reducing variant (`par_fold_chunks` with `B: Send + identity + combine`), which is its own design (Rayon's `ParallelIterator::reduce` semantics). That work is out of scope here.
3. Risk-1 (§13): adding `fold_chunks` enlarges the surface area for the marker-trait gate without delivering measured benefit on the canonical bench.

`fold_chunks` will be filed as Phase 13.X **after** the Phase X.A bench numbers are validated. The deferral is reversible.

---

## §3 Filter gating decision

### 3.1 Decision matrix

| Option | Approach | Verdict | Reason |
|---|---|---|---|
| A — Marker subtrait (`ArchetypalQueryFilter`) | Manual `unsafe impl` blocks; tuple/`Or` propagate | **CHOSEN** | Works on **stable Rust 2024** (no `generic_const_exprs`); zero runtime overhead; clean compile-time error message; matches Phase 8.5's `derive(Bundle)` pattern (manual opt-in). |
| B — Yield `(slice, &BitSet)` | Engine pre-scans tick column, hands slice + mask to closure | DEFER | Adds per-frame bitmap-construction cost on every call (proportional to row count) even when the user doesn't consult it; locks in a `BitSet` ABI. Useful but additive — file Phase 13.X. |
| C — Scalar fallback (`if F::IS_ARCHETYPAL { fast } else { slow }`) | Silent perf cliff at the API surface | REJECT | Footgun. CLAUDE.md principle 1 (zero overhead): "no compromise in favor of convenience". |
| D — `where const { F::IS_ARCHETYPAL }` bound | Method-level const-bool bound | REJECT | Requires `generic_const_exprs` (unstable); fails CLAUDE.md target-platform constraint (stable engine library). |

### 3.2 Why not a blanket impl on `QueryFilter` gated on the const?

A blanket of the form:

```rust
unsafe impl<F: QueryFilter> ArchetypalQueryFilter for F
where /* somehow encode F::IS_ARCHETYPAL == true */ {}
```

cannot be written on stable Rust 2024:

- `where F::IS_ARCHETYPAL` is not a valid bound — associated constants can't appear in `where` clauses.
- `where { F::IS_ARCHETYPAL }: True` (typenum-style) requires `generic_const_exprs`.
- A `where F: ArchetypalQueryFilter` bound on the leaf impl is circular.

Manual `unsafe impl` per known filter is correct, terse, and unambiguous. It scales to 4 leaves + 1 macro (tuples) + 1 line (`Or<F>` propagation) = **6 impls total**. Far below the 78-impl risk surface that motivated the sibling-trait decision for `ChunkedQueryData`.

### 3.3 Compile-error UX

When a user writes:

```rust
fn sys(mut q: Query<&Position, Changed<Velocity>>) {
    q.for_each_chunk(|positions: &[Position]| { /* ... */ });
}
```

The error message is:

```
error[E0277]: the trait bound `Changed<Velocity>: ArchetypalQueryFilter` is not satisfied
   --> src/lib.rs:42:7
    |
42  |     q.for_each_chunk(|positions: &[Position]| { /* ... */ });
    |       ^^^^^^^^^^^^^^ the trait `ArchetypalQueryFilter` is not implemented for `Changed<Velocity>`
    |
    = help: `Changed<C>` performs per-row tick comparison and cannot yield a contiguous slice.
            Use `Query::iter()` for per-row iteration with change detection,
            or `Query::for_each_chunk_with_mask` (Phase 13.X, not yet shipped).
note: required by a bound in `Query::for_each_chunk`
```

The "help" text is delivered via a `#[diagnostic::on_unimplemented]` attribute on the `ArchetypalQueryFilter` trait (stable since Rust 1.78). Mandatory in the implementation step.

---

## §4 QueryData extension decision — **Sibling trait `ChunkedQueryData`**

### 4.1 Decision

**Sibling trait `ChunkedQueryData: QueryData`**, not a new GAT on `QueryData`.

### 4.2 Trade-off analysis

| Aspect | Extend `QueryData` (add GAT) | Sibling trait `ChunkedQueryData` (CHOSEN) |
|---|---|---|
| Impls touched | **78** (Phase 10 expansion: 8 leaves × 4 set_table variants + 12 tuples + 12 too-large stubs) | **15** (3 leaves: `&T`, `&mut T`, `()` + 12 tuple macro expansions; `Ref`/`Mut`/too-large stubs do NOT implement) |
| Backward compatibility | Breaks every existing custom `QueryData` impl that downstream users may have | Zero break — opt-in |
| Trait surface size | Bloats `QueryData`'s already 11-method surface to 14 | Adds a distinct 4-method trait, neatly scoped |
| Test surface (compile-fail) | Each existing impl needs new associated type/method | Untouched impls require no test updates |
| `Ref`/`Mut` story | Must add either `cold panic` bodies (mirror of `_no_meta` backstops, 4 more cold panics) OR explicit "ChunkItem = ()" stub | Trivially `Ref`/`Mut` do not implement; type system prevents misuse |
| Compile-time on macro expansion | +78 method definitions | +15 method definitions |

### 4.3 Why `Ref` and `Mut` are not `ChunkedQueryData` members

`Ref<'_, T>` and `Mut<'_, T>` carry per-row `last_run`/`this_run` ticks inside `Self::Item<'w>`. Their per-row `Self::Item<'w> = Ref<'w, T>` (or `Mut<'w, T>`) explicitly bundles a tick snapshot with each value. There is no semantically-equivalent slice form:

- `&'c [Ref<'c, T>]` would require materializing a parallel `Box<[Ref<T>]>` per chunk, which contradicts the zero-allocation hot-path principle.
- `(&'c [T], &'c [Tick], Tick, Tick)` is possible but pushes the tick interpretation onto every user — a footgun.

Phase 13.X may add a `ChunkedTickedQueryData` with a `ChunkItem<'c> = TickedSlice<'c, T>` shape (a tiny wrapper struct holding the four pointers and exposing the value slice plus per-row tick accessors). Out of scope here.

### 4.4 Why `QueryData: ChunkedQueryData` is NOT a supertrait blanket

We do not declare `ChunkedQueryData` as a supertrait of `QueryData` (i.e., we keep them sibling traits). If `ChunkedQueryData` were a supertrait, every existing `QueryData` impl would have to provide `ChunkItem`/`fetch_chunk`. Same problem as the GAT-extension option — defeats the purpose of choosing the sibling trait.

---

## §5 Tuple support

### 5.1 `ChunkedQueryData` tuple macro

Mirrors the existing `impl_query_data_tuple!` pattern (78-impl machinery in `data.rs`). The macro destructures via paired idents `(D, s, f)`:

```rust
macro_rules! impl_chunked_query_data_tuple {
    ( $( ($D:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY (CD1-CD4): forwarded per-element; the tuple impl is
        //   sound iff each element's chunked impl is sound. The archetype
        //   pointer is identical for every element (one archetype per
        //   `set_chunk_*` call), and the row range `[start, start + len)`
        //   is identical for every per-element `fetch_chunk` call —
        //   guaranteeing same-length slices.
        #[allow(non_snake_case)]
        unsafe impl< $($D: ChunkedQueryData),* > ChunkedQueryData for ( $($D,)* ) {
            type ChunkFetch<'c> = ( $($D::ChunkFetch<'c>,)* );
            type ChunkItem<'c>  = ( $($D::ChunkItem<'c>,)* );

            #[inline]
            fn init_chunk_fetch<'c>(state: &Self::State) -> Self::ChunkFetch<'c> {
                let ( $($s,)* ) = state;
                ( $( <$D as ChunkedQueryData>::init_chunk_fetch($s), )* )
            }

            #[inline]
            unsafe fn set_chunk_readonly<'c>(
                fetch: &mut Self::ChunkFetch<'c>,
                state: &Self::State,
                archetype: *const Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (CD1, CD4): forwarded per-element; `archetype`
                    //   read-only provenance is identical for every element.
                    unsafe { <$D as ChunkedQueryData>::set_chunk_readonly($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn set_chunk_mut<'c>(
                fetch: &mut Self::ChunkFetch<'c>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY (CD1, CD4): write-capable `archetype` forwarded;
                    //   per-element CD4 enforces wrong-kind dispatch prevention.
                    unsafe { <$D as ChunkedQueryData>::set_chunk_mut($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn fetch_chunk<'c>(
                fetch: &Self::ChunkFetch<'c>,
                start: usize,
                len: usize,
            ) -> Self::ChunkItem<'c> {
                let ( $($f,)* ) = fetch;
                (
                    $(
                        // SAFETY (CD2, CD3): per-element fetch_chunk;
                        //   distinct slices into distinct columns are
                        //   non-aliasing by archetype invariant (different
                        //   components → different memory regions).
                        unsafe { <$D as ChunkedQueryData>::fetch_chunk($f, start, len) },
                    )*
                )
            }
        }
    };
}

impl_chunked_query_data_tuple!((D0, s0, f0));
impl_chunked_query_data_tuple!((D0, s0, f0), (D1, s1, f1));
// ... up to arity 12 (mirror MAX_QUERY_DATA_ARITY)
```

### 5.2 `ArchetypalQueryFilter` tuple + `Or<F>` propagation

```rust
macro_rules! impl_archetypal_filter_tuple {
    ( $( $F:ident ),* ) => {
        // SAFETY: every element is `ArchetypalQueryFilter` → IS_ARCHETYPAL
        //   = true ∧ NEEDS_CHANGE_DETECTION = false for each, and the
        //   tuple-AND propagation in QueryFilter (already in filter.rs)
        //   preserves both invariants.
        unsafe impl< $($F: ArchetypalQueryFilter),* > ArchetypalQueryFilter for ( $($F,)* ) {}
    };
}

impl_archetypal_filter_tuple!(F0);
impl_archetypal_filter_tuple!(F0, F1);
// ... arity 1..=12.

// Or<F> — element-wise propagation. Or<(With<A>, Changed<B>)> is NOT
// archetypal; Or<(With<A>, Without<B>)> IS.
//
// SAFETY: the concrete `QueryFilter for Or<F>` impl in `filter.rs:1151`
//   is monomorphised as `Or<(F0, F1, …)>` — the inner `F` is always a
//   tuple. The tuple impl above ensures `(F0, F1, …)` implements
//   `ArchetypalQueryFilter` iff every element does. Therefore the bound
//   `F: ArchetypalQueryFilter` on this blanket is sufficient: it forces
//   the inner tuple to be archetypal element-wise, which propagates the
//   `IS_ARCHETYPAL = true ∧ NEEDS_CHANGE_DETECTION = false` invariants
//   transitively to the `Or<F>` wrapper.
unsafe impl<F: ArchetypalQueryFilter> ArchetypalQueryFilter for Or<F> {}
```

The `Or<F>` impl is a single-line blanket. `F` here is the inner tuple — the tuple impl above ensures `F = (F0, F1)` is `ArchetypalQueryFilter` iff `F0, F1` both are. Propagation is transitive.

### 5.3 Empty tuple

```rust
unsafe impl ChunkedQueryData for () {
    type ChunkFetch<'c> = ();
    type ChunkItem<'c>  = ();

    #[inline] fn init_chunk_fetch<'c>(_: &()) {}
    #[inline] unsafe fn set_chunk_readonly<'c>(_: &mut (), _: &(), _: *const Archetype) {}
    #[inline] unsafe fn set_chunk_mut<'c>(_: &mut (), _: &(), _: *mut Archetype) {}
    #[inline] unsafe fn fetch_chunk<'c>(_: &(), _: usize, _: usize) {}
}
```

Useful for `Query<(), With<Player>>::for_each_chunk(|_| { /* count archetypes */ })`.

### 5.4 Arity cap & "too large" stubs

Match existing `MAX_QUERY_DATA_ARITY = 12`. No too-large stubs are needed for `ChunkedQueryData`: a user attempting an arity-13 chunked query first hits the existing `QueryData` too-large stub (monomorphisation `panic!`) before the `ChunkedQueryData` bound is even checked.

---

## §6 Alignment story

### 6.1 Current state (verified)

- `Arena::with_capacity` (arena.rs:58–75) allocates the backing buffer with `Layout::from_size_align(_, CACHE_LINE_SIZE)` → **64-byte aligned arena base**.
- `Arena::allocate_from_free_blocks` calls `free_blocks.allocate_aligned(size, align)` where `align = layout.align()` of the requested `Layout`. So column allocations honor exactly `layout.align()` of the per-`T` request, **not more**.
- `ComponentPool::buffer_ptr()` (component_pool.rs:732) returns `self.buffer.as_ptr().cast_const()` — the result of `Arena::allocate`. Alignment guarantee: **`align_of::<T>()` exactly**, no SIMD-specific lift.

For `T = f32`, `align_of::<f32>() = 4`. Column starts are 4-byte aligned, not 32-byte. AVX2 `vmovups` works fine but the first 8 elements may straddle a 64-byte cache line, doubling load cost on the first few iterations.

### 6.2 Decision — **(i) Lift to `max(align_of::<T>(), 32)` for `ComponentPool` allocations**

**What changes**: In the `ComponentPool::new` allocation path (or wherever the `Layout` is computed for the backing buffer), compute:

```rust
let element_align = component_layout.align();
let buffer_align  = element_align.max(SIMD_BUFFER_ALIGN);   // 32 = AVX2 baseline
let buffer_layout = Layout::from_size_align(buffer_size, buffer_align)
    .expect("buffer layout valid");
```

where `SIMD_BUFFER_ALIGN: usize = 32` is added to `crates/boyko_ecs/src/ecs/constants.rs`.

**Cost** (constants sourced from `crates/boyko_ecs/src/ecs/core/component/component_registry.rs:47` — `MAX_COMPONENTS = 512` — and `crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs:7` — `MAX_ARCHETYPES = 1024`):
- At most one extra alignment gap per column allocation: `32 - 4 = 28` bytes wasted for `T = f32` columns. With `MAX_COMPONENTS = 512` components × `MAX_ARCHETYPES = 1024` archetypes worst case = `512 × 1024 × 28 B ≈ 14 MB` wasted out of the 64 MB arena in pathological scenarios (every component f32, every archetype populated). Realistic workloads (most components ≥ 16 B): waste under 1 MB total. Acceptable.
- Zero runtime overhead — alignment lift happens once at `ComponentPool::new`, never on hot path.
- No effect on smaller-aligned `T` reads — `align_of::<T>() ≤ 32 → 32`-byte-aligned start still respects `T`'s alignment.

**Why not (ii) just document `align_of::<T>()`**: Bevy's `iter()` already gets `align_of::<T>()` "for free"; the whole point of `for_each_chunk` is to push past Bevy on SIMD-amenable workloads. Without column-start SIMD alignment the LLVM auto-vectorizer emits an unaligned-load prologue that drops throughput on the first cache line of every archetype. Cost-benefit favors the lift.

**Why not 64-byte (AVX-512 baseline)**: AVX-512 is opt-in (`cfg(target_feature = "avx512f")`) per CLAUDE.md target. Default 32-byte serves the stated AVX2 baseline. A future `SIMD_BUFFER_ALIGN_AVX512: usize = 64` cfg-gated constant can lift further if needed.

### 6.3 Documented invariants

In `ComponentPool::buffer_ptr` doc:

```rust
/// # Alignment guarantee (Phase X.A SIMD-A1)
///
/// The returned pointer is aligned to `max(align_of::<T>(), SIMD_BUFFER_ALIGN)`
/// where `SIMD_BUFFER_ALIGN = 32` (AVX2 register width). This guarantees that
/// the first AVX2 256-bit load from the column start lands within a single
/// 64-byte cache line, eliminating the cross-CL load penalty for archetype
/// row 0 (Intel Optimization Manual §3.6).
///
/// Per-row alignment beyond `align_of::<T>()` is **not** guaranteed: for
/// non-power-of-2-sized `T` (e.g. `struct Foo([f32; 3])`, 12 B), interior
/// rows are aligned only to `align_of::<T>()`. Users emitting explicit SIMD
/// loads must use unaligned-load intrinsics (`_mm256_loadu_ps`) or rely on
/// LLVM autovectorization which handles unaligned interior rows correctly.
///
/// See Phase X.A plan §6 for the rationale and the Bevy PR #6161 `Vec3`
/// soundness postmortem that motivated rejecting per-row alignment promises.
```

### 6.4 Verification gate

In `for_each_chunk` / `par_for_each_chunk` outer-loop body, at the point where we derive `&[T]` from `column.ptr`:

```rust
debug_assert!(
    (column.ptr as usize) % SIMD_BUFFER_ALIGN == 0,
    "Phase X.A SIMD-A1: column.ptr must be aligned to SIMD_BUFFER_ALIGN ({} B). \
     Got pointer {:p} with alignment offset {} B.",
    SIMD_BUFFER_ALIGN, column.ptr, (column.ptr as usize) % SIMD_BUFFER_ALIGN
);
```

Vanishes in release. Catches regressions in the Arena allocation path.

### 6.5 Why this isn't the Bevy `Vec3` trap

Bevy PR #6161's soundness blocker was that `_mm_loadu_ps` on a 12 B `Vec3` reads 16 B — overshooting the type by 4 B and consuming uninitialized padding. The fix would have been per-row alignment guarantees, requiring `generic_const_exprs`.

Phase X.A side-steps this entirely:

- The engine emits **no SIMD intrinsics** on behalf of the user.
- The closure receives `&[T]` typed slices; reading past `slice.len()` is a Rust-language UB (bounds check elision relies on type-checked `slice[i]`), not an engine concern.
- If the user inside the closure calls `_mm_loadu_ps((p as *const Vec3 as *const f32))`, that's the user's bug — same as today's `iter()`-based code.
- For the canonical f32 reduction bench, `T = f32` (4 B); no padding exists.

---

## §7 Change-detection integration

### 7.1 Compile-time elision

The two new bounds on `for_each_chunk` (`D: ChunkedQueryData` + `F: ArchetypalQueryFilter`) guarantee at the type level:

- `D::NEEDS_CHANGE_DETECTION = false`: enforced because the only `ChunkedQueryData` impls are `&T`, `&mut T`, `()`, and tuples thereof. `Ref`/`Mut` are NOT members. All tuples propagate via `NCD3` (existing `QueryData::NEEDS_CHANGE_DETECTION = false || $D::NCD`) — every leaf is false, so every tuple is false.
- `F::NEEDS_CHANGE_DETECTION = false`: enforced because `ArchetypalQueryFilter` members are `()`, `With<C>`, `Without<C>`, `Or<F>` (over Archetypal), and Archetypal tuples — every one has `NEEDS_CHANGE_DETECTION = false`.

Therefore the `if const { D::NCD || F::NCD }` dispatcher used in `iter.rs:245`/`par_iter.rs:582` **resolves to `false` at every `for_each_chunk` monomorphisation**. The chunked path never threads `&SystemMeta`, never reads ticks, never branches at the per-archetype boundary on NCD state.

**Practical implication**: the chunked dispatcher in `query.rs` does NOT call `set_table_readonly_no_meta` / `set_table_mut_no_meta` at all. It calls the new `ChunkedQueryData::set_chunk_readonly` / `set_chunk_mut`, which take no `meta` parameter by design (§2.2). This is one less function-pointer indirection and one less branch than the existing iter path.

### 7.2 `&mut T` writes — tick-bumping NOT performed

`for_each_chunk` over `Query<&mut T>` yields a `&'c mut [T]` slice. The user mutates rows freely. **The component's `changed_tick` column is NOT bumped** by the engine — same as today's `Query<&mut T>::iter_mut()` (which uses the plain `&mut T` data impl with `NCD = false`).

If the user wants change tracking on writes, they must use `Query<Mut<T>>::iter_mut()` with the deref guard. There is no `Query<Mut<T>>::for_each_chunk` because `Mut<T>` is not a `ChunkedQueryData` member (§4.3).

This is the same constraint Bevy enforces. It is documented as part of `ChunkedQueryData`'s trait doc:

> Writes through `for_each_chunk`'s `&'c mut [T]` slices do NOT trigger `Changed<T>` notifications. Use `Query<Mut<T>>::iter_mut` when change tracking is required.

### 7.3 Future extension path (NOT in scope)

Phase 13.X may add `Query<Mut<T>>::for_each_chunk_tracked(|values: &mut [T], ticks: &mut [Tick]|)` for the case where the user wants both batched writes and per-row tick bumps. Out of scope here.

---

## §8 Bench harness decision

### 8.1 Toolchain — **per-package `rust-toolchain.toml` at the bench crate**

The engine workspace stays on the user's default toolchain (typically stable). The bench crate — and only the bench crate — opts into nightly. This avoids forcing nightly on `cargo check --all-targets` at the workspace root, the rustdoc deploy in `.github/workflows/docs.yml`, or any downstream consumer that vendors `boyko_ecs` against stable.

**Mechanism** (verified against `https://rust-lang.github.io/rustup/overrides.html`): rustup discovers `rust-toolchain.toml` by walking up from the current working directory; the closest file wins. Placing the file inside `crates/bench_bevy_vs_boyko/` scopes the override to invocations inside that directory and its children. The workspace root and the other crates remain on the default toolchain.

Add at `D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\rust-toolchain.toml`:

```toml
[toolchain]
channel = "nightly"
components = ["rustc", "cargo", "rust-std", "rust-docs", "clippy", "rustfmt"]
profile = "minimal"
```

The exact channel pin (e.g., `nightly-2026-MM-DD`) is selected at Wave 8A impl time using the latest stable nightly available then. No date is baked in here.

**Canonical invocations** (matching CLAUDE.md build commands):

| Command | Toolchain used | Notes |
|---|---|---|
| `cargo check --all-targets` (at workspace root) | Default (stable) | Engine + all non-bench crates type-check on stable. |
| `cargo test --all-targets` (at workspace root) | Default (stable) | Tests run on stable; bench crate's tests use `#[cfg(all(test, ...))]` guards to skip nightly-only items. |
| `cargo bench --bench g6_for_each_chunk` from inside `crates/bench_bevy_vs_boyko/` | Nightly (via per-package file) | Pulls `f32::algebraic_add` etc. |
| `cargo +nightly bench -p bench_bevy_vs_boyko --bench g6_for_each_chunk` from workspace root | Nightly (explicit) | Equivalent; useful for CI scripts that don't `cd`. |

**Engine library impact**: zero — no nightly features anywhere outside `crates/bench_bevy_vs_boyko/`. The stable engine library invariant per Phase 12.5 memory ("Engine — preferably stable; benches и SIMD-критичные пути — nightly OK") is preserved.

### 8.2 Bench file path & shape

`D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\benches\g6_for_each_chunk.rs` (new file). Cargo.toml addendum:

```toml
[[bench]]
name = "g6_for_each_chunk"
harness = false
```

Bench harness (criterion 0.5, html_reports):

```rust
#![feature(float_algebraic)]

use criterion::{black_box, criterion_group, criterion_main, Criterion, BatchSize};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::Component;

// Bevy comparison side
use bevy_ecs::prelude::*;

#[derive(Component, Clone, Copy)]
#[repr(transparent)]
struct Position(f32);   // matches Bevy's Component derive on the comparison side

const N: usize = 10_000;

fn build_boyko_world() -> EcsMaster { /* spawn N entities into one archetype */ }
fn build_bevy_world() -> bevy_ecs::world::World { /* same N entities */ }

pub fn bench_g6(c: &mut Criterion) {
    let mut group = c.benchmark_group("g6_for_each_chunk_sum_10k");
    group.sample_size(60);  // ≥30 + headroom; criterion default is 100

    group.bench_function("boyko_for_each_chunk_algebraic_sum", |b| {
        let mut world = build_boyko_world();
        b.iter(|| {
            // SAFETY: bench scope; world is exclusive to this iter.
            let mut acc: f32 = 0.0;
            world.query::<&Position, ()>().for_each_chunk(|positions: &[Position]| {
                // Inner loop: typed slice → autovec via algebraic_add.
                for p in positions.iter().copied() {
                    acc = f32::algebraic_add(acc, p.0);
                }
            });
            black_box(acc)
        });
    });

    group.bench_function("bevy_iter_fold_algebraic_sum", |b| {
        let mut world = build_bevy_world();
        let mut query = world.query::<&Position>();
        b.iter(|| {
            // Bevy's fairest baseline: Iterator::fold override (PR #6773)
            // PLUS algebraic_add to eliminate the per-element reorder barrier.
            let acc = query.iter(&world).fold(0.0_f32, |a, p: &Position| {
                f32::algebraic_add(a, p.0)
            });
            black_box(acc)
        });
    });

    group.finish();
}

criterion_group!(benches, bench_g6);
criterion_main!(benches);
```

### 8.3 Why this is the fairest baseline

- **Workload identical**: 10k f32-component entities, one archetype, single-component f32 sum reduction. Both engines pay archetype-resolve cost once per iter; both walk the column.
- **`algebraic_add` on both sides**: removes the `black_box` per-element optimization barrier from both engines. The bench measures the **API shape** difference, not a measurement-fairness artifact.
- **Bevy's `Iterator::fold` override**: per PR #6773, Bevy's `query.iter(...).fold(...)` is already the perf-optimal scalar walk over Bevy's storage. Without this baseline, we'd be measuring against the deprecated `Query::for_each` shape, which is slower.
- **Same archetype layout assumption**: both engines store a single contiguous column for `Position` in the one archetype. No SparseSet on the Bevy side (default is Table for `Component` derive).

### 8.4 Expected gain — **5–8×, with 5× as the floor for PASS**

| Factor | Mechanism | Expected magnitude |
|---|---|---|
| Slice→typed-slice elision | `&[Position]` collapses bounds check + `Iterator::next` state machine | 1.3× |
| `f32::algebraic_add` over slice | LLVM partial-sum unroll + VADDPS over 8 lanes | 4–6× (orlp.net 21.6× under ideal; Bevy already gets some of this via fold override) |
| 32-byte aligned column start (§6) | First load avoids cross-CL split | 1.05–1.10× |
| **Combined floor** | | **5×** |
| **Combined target** | | **8×** |

If the bench falls below 5×, the criterion script should report it as a regression test failure (PASS bar: median ≥ 5× over Bevy median across 60 samples).

### 8.5 Stable-Rust fallback (NOT chosen, documented for completeness)

If the orchestrator later vetoes the nightly toolchain, the bench can be rewritten with stable `std::arch::x86_64::{_mm256_loadu_ps, _mm256_add_ps, _mm256_storeu_ps}` and a manual horizontal-reduction tail. This sacrifices portability (x86 only, would skip on Linux ARM CI runners) but works on stable. **Not the default choice** — nightly is pre-approved per the prompt and `feedback-nightly-rust-allowed.md`.

---

## §9 Parallel composition

### 9.1 Granularity decision — **per-archetype-subrange (matches Phase 9 `par_iter`)**

**Reuse** `BatchingStrategy` from `par_iter.rs` (lines 71–124) verbatim. Same `MIN_ARCHETYPE_FOR_PARALLEL = 1024` threshold. Same dispatch pattern: `pool.scope` + `scope.spawn(move || run_chunk_owned(...))`.

**Why per-subrange and not per-whole-archetype**:

1. The Phase 9 baseline already proved per-subrange works (2.93× win on `par_iter` 10k bench).
2. SIMD work per row is on the order of 0.1–1 ns (orlp.net's `algebraic_add` peaks at ~1 ns/element for f32). For a single-archetype 100k-row workload, per-whole-archetype would assign all work to one worker → 100 µs serial → idle workers.
3. Per-subrange dispatch overhead ~120 ns/spawn (Phase 9 §10.3) is amortized over the work chunk. Worked example with the real `BatchingStrategy` formula: 100k-row archetype on 8 workers with default `batches_per_thread = 1` → `batch_size = clamp(100000 / 8, 1024, _) = 12500`, 8 closure invocations, each ~12.5 µs of work at ~1 ns/row → dispatch tax `120 ns / 12500 ns ≈ 1%`. Acceptable. The floor-bound regime (small archetypes, e.g. 4096 rows / 8 workers = 512 raw, clamped to 1024) caps invocation count at `entity_count / 1024 = 4`, each still ≥ 1 µs of work → dispatch tax ≤ 12%, still acceptable.
4. The `MIN_ARCHETYPE_FOR_PARALLEL = 1024` inline threshold already prevents tiny archetypes from paying the dispatch tax.

### 9.2 Closure bounds

| Method | `Func` bound | Reason |
|---|---|---|
| `for_each_chunk` | `for<'c> FnMut(D::ChunkItem<'c>)` | Single-threaded; mutable closure state OK (e.g., accumulator). |
| `par_for_each_chunk` | `for<'c> Fn(D::ChunkItem<'c>) + Send + Sync` | Multiple workers may invoke concurrently. `Send + Sync` propagates from existing `par_iter` (line 219 / 254 of `par_iter.rs`). |

### 9.3 Aliasing safety for `&'c mut [T]` parallel workers

For `D = &mut T` (yields `&'c mut [T]`), the parallel form must guarantee no two workers receive overlapping mutable slices. This is enforced by:

1. **Cross-archetype**: each `scope.spawn` worker is bound to one `(arch_id, start, end)` triple. Distinct archetypes have disjoint storage (Archetype invariant per Phase 7).
2. **Intra-archetype (same archetype, different row ranges)**: the dispatch loop walks `start..entity_count` step `chunk_size` (par_iter.rs:325–375). Each `(start, end)` is strictly monotonic-non-overlapping. The `ChunkCaptures { start, end, ... }` already encode this.
3. **Cross-system aliasing**: Phase 9's `ConflictGraph` (SCH3) already prevents two systems with conflicting access from running concurrently. `par_for_each_chunk` inherits this — it doesn't relax the rule.

The closure body sees a `&'c mut [T]` for rows `[start, end)`. The `for<'c>` HRTB issues a fresh borrow per invocation; no borrow outlives its `scope.spawn` call. Drop happens at chunk completion; `scope.Drop` joins all chunks before `par_for_each_chunk` returns.

### 9.4 `ChunkCaptures` extension

Re-use the existing `ChunkCaptures<D, F>` from `par_iter.rs:423–437`, but adapt for the chunked path. The chunked variant does NOT need:

- `meta: *const SystemMeta` (always meta-free; NCD = false at this monomorphisation).
- `mutable: bool` (always known at monomorphisation via `D` — see below).

The chunked variant DOES need:

```rust
#[derive(Clone, Copy)]
struct ChunkChunkCaptures<D: ChunkedQueryData, F: ArchetypalQueryFilter> {
    data_state: *const D::State,
    filter_state: *const F::State,  // archetypal filters have () state — but kept for the dispatcher symmetry
    archetype: *mut Archetype,      // *mut for symmetry; the readonly path casts down
    start: usize,
    end: usize,
    mutable: bool,                  // runtime flag; same shape as par_iter
}

// SAFETY (same template as par_iter ChunkCaptures Send impl).
unsafe impl<D: ChunkedQueryData, F: ArchetypalQueryFilter> Send
    for ChunkChunkCaptures<D, F> {}
```

The naming is intentionally distinct from `ChunkCaptures` to avoid name collision in `par_iter.rs`. Lives in a new sibling module `par_chunk.rs` (§12 step 7).

### 9.5 `mutable` flag

Two options for switching between `set_chunk_readonly` and `set_chunk_mut` in the parallel driver:

| Option | Approach | Verdict |
|---|---|---|
| Runtime `mutable: bool` flag (Phase 9 pattern) | One driver fn, branch per archetype boundary | Shared with the seq path; one branch per archetype-loop iteration (cold) |
| Two monomorphic drivers gated by `IS_READ_ONLY` | `par_chunk_readonly_driver` + `par_chunk_mut_driver` | More I-cache cost, no measured benefit per Phase 9 ratio |

**Decision**: runtime flag. Same rationale as Phase 9 `for_each_impl` (par_iter.rs:236–243): branch resolves into 1–2 CMOVs amortized over the archetype walk; benchmarks have shown no measurable difference.

---

## §10 Hot-path audit (CLAUDE.md principles 1–8)

### 10.1 Principle 1 — Zero runtime overhead

| Hot-path element | Mechanism | Static dispatch? |
|---|---|---|
| `for_each_chunk<Func>` over `Func: for<'c> FnMut(D::ChunkItem<'c>)` | Generic monomorphisation | YES — `Func` is a concrete closure type per call site; no `dyn` |
| `D: ChunkedQueryData` per-element fetch | Generic monomorphisation via `ChunkedQueryData::set_chunk_*` | YES — concrete `D` per call site |
| `F: ArchetypalQueryFilter` | Empty marker trait; no runtime cost (the filter's archetype-level predicate ran at `state.update(master)` time) | YES |
| Archetype iteration | `for arch_id in state.archetype_state.matched_ids()` — slice walk | YES — no `dyn`, no `Box` |

No `Box<dyn Trait>`. No `HashMap`. No `Vec::new()`. No `format!()`. Hot path is a slice walk over cached archetype IDs with one indirect call per archetype.

### 10.2 Principle 2 — Data-Oriented Design

- Per-archetype slices are SoA — one slice per component column. User's inner loop reads contiguous bytes.
- `Column` (16 B `#[repr(C)]`, ptr + stride + reserved) lives inline at offset 0 of `Archetype` for the Phase 7 D4 fast path. The new code reuses this lookup verbatim.
- No hot/cold split needed inside the new types — `ChunkFetch<'c>` mirrors `Fetch<'w>` which is already minimal (4-pointer struct for the largest leaf variant).

### 10.3 Principle 3 — Cache (D + I)

**D-cache**:
- Column-start alignment lifted to 32 B (§6), so the first AVX2 load lands in a single cache line.
- Working set per archetype = `entity_count × sizeof(T)` of contiguous bytes. For the canonical bench: 10k × 4 B = 40 KB; fits in L1d.
- For multi-component tuples, each column is independent. The user's inner loop iterates indices `0..len` accessing slice `a[i]`, `b[i]`, ... — concurrent strided reads, well-handled by the L1d prefetcher.

**I-cache**:
- `for_each_chunk` body is small: outer loop (~6 instructions: `next archetype id`, `archetype_ptr_mut(_)?`, `entity_count > 0?`, `set_chunk_*`, `fetch_chunk(0, len)`, indirect call into user closure). Per CLAUDE.md principle 7, **no `#[inline(always)]`** on the outer body. `#[inline]` on cross-crate generic call sites (mirror existing iter.rs pattern).
- The user's closure is monomorphized into the call site; LLVM decides whether to inline. The compiler sees only one `f(slice)` call per archetype — no combinatorial inlining bloat.
- `set_chunk_readonly` / `set_chunk_mut` are `#[inline]` (cross-crate generic) — same policy as `set_table_*`.
- `#[cold]` + `#[inline(never)]` on the panic backstops for `&mut T::set_chunk_readonly` (when user violates the type gate via a custom impl).

### 10.4 Principle 4 — Lock-free

The new path uses zero locks:
- `state.update(master)` was already called by the SystemParam pipeline before the closure body; not on the inner path.
- `archetype_ptr_mut(_)` is a raw-pointer lookup against the cell's `*mut EcsMaster` (Phase 7).
- `pool.scope` (Phase 9) is the only sync primitive in `par_for_each_chunk`; same as today's `par_iter`.

### 10.5 Principle 5 — Minimum allocations

The chunked path allocates **zero** bytes per call. `ChunkFetch<'c>` is a stack-resident `Copy` struct. `ChunkChunkCaptures<D, F>` is stack-resident inside the dispatch loop body. The `scope.spawn` closure captures `ChunkChunkCaptures` by value — same shape as Phase 9.

### 10.6 Principle 6 — SIMD-friendly

This phase's **raison d'être**. The user's inner loop receives `&[T]` and is autovectorized by LLVM whenever:
- `T` is a scalar / SoA-shaped struct.
- The user's reduction respects autovec rules (use `f32::algebraic_add` for floats; integer ops always autovec).

The engine does NOT emit SIMD intrinsics itself — per research §2.5 (Intel manual), Bevy PR #6161 burned its complexity budget on intrinsics + alignment generics. Phase X.A delegates SIMD to LLVM by giving it a clean slice typed input.

### 10.7 Principle 7 — Measured inlining

Inline annotations on the new code (mirrors the Phase 9 precedent at `par_iter.rs:166-167, 216-217` for the public shim vs `par_iter.rs:244` for the internal driver):

| Site | Annotation | Justification |
|---|---|---|
| `ChunkedQueryData::init_chunk_fetch` | `#[inline]` | Cross-crate generic; LTO needs the body |
| `ChunkedQueryData::set_chunk_readonly` (leaf impls) | `#[inline]` | Same |
| `ChunkedQueryData::set_chunk_mut` (leaf impls) | `#[inline]` | Same |
| `ChunkedQueryData::fetch_chunk` (leaf impls) | `#[inline]` | Same |
| Tuple impl methods | `#[inline]` | Same |
| `Query::for_each_chunk` / `Query::par_for_each_chunk` plus their `QueryView::*` mirrors (public methods) | `#[inline]` | Cross-crate visibility for closure inlining via LTO; mirrors `par_iter.rs:166-167, 216-217` (`ParQuery::for_each` / `ParQueryMut::for_each` shims). The `QueryView::*` direct-API mirrors carry the same annotation for the same reason (cross-crate call from `EcsMaster::query` users). |
| `chunk_iter::for_each_chunk_impl` / `par_chunk::par_for_each_chunk_impl` (internal drivers) | NO annotation (LLVM decides) | Mirrors `par_iter.rs:244` (`for_each_impl`); per CLAUDE.md principle 7, no `#[inline(always)]` without profiler evidence |
| `&mut T::set_chunk_readonly` panic backstop | `#[cold]` + `#[inline(never)]` | Error path; exit the hot I-cache |
| Per-archetype-chunk worker body (inside `par_chunk`) | `#[inline]` | Mirror of `par_iter.rs:run_chunk_owned` |

**Phase 9 precedent rationale**: the public `Query::for_each_chunk` is the cross-crate-boundary method the user's system body calls. `#[inline]` exposes the body to LTO so the user's closure can be inlined into the driver call site. The internal `for_each_chunk_impl` driver is a single-translation-unit function — LLVM already has full visibility and decides whether to inline based on cost-model. Adding `#[inline]` there is redundant and bloats no fewer cases.

**Critic-deflection note**: I deliberately do not apply `#[inline(always)]` to any per-archetype-boundary function. If profiling after Phase X.A lands shows a missed inline, we add it then — measurement-driven, not doctrine. This matches the Phase 12.6 outcome ("asm byte-identical Bevy" without `#[inline(always)]`).

### 10.8 Principle 8 — Justified unsafe

Every `unsafe` block in the new code carries a `// SAFETY:` comment listing invariants. The major unsafe sites:

```rust
// In `&T as ChunkedQueryData::fetch_chunk`:
unsafe fn fetch_chunk<'c>(
    fetch: &Self::ChunkFetch<'c>,
    start: usize,
    len: usize,
) -> Self::ChunkItem<'c> {
    // SAFETY (CD1, CD2, plan §6 SIMD-A1):
    //   - `set_chunk_*` was called before `fetch_chunk` (caller contract);
    //     `fetch.base` is non-null and points at the active archetype's
    //     column for T (verified non-null by debug_assert above).
    //   - `start + len ≤ entity_count` (caller contract, asserted by the
    //     dispatcher's `len.min(entity_count - start)` clamp).
    //   - `column.ptr` is 32-B-aligned per Phase X.A SIMD-A1; `T`'s
    //     alignment is ≤ 32; therefore `column.ptr + start * size_of::<T>()`
    //     is at least `align_of::<T>()`-aligned (sufficient for `&[T]`).
    //   - The slice lifetime `'c` is the closure-body scope; `'c` ⊂ `'w`
    //     ⊂ archetype-pointer scope (Phase 7 U1/U2 slab stability).
    unsafe {
        std::slice::from_raw_parts(
            (fetch.base as *const T).add(start),
            len,
        )
    }
}
```

For the `&mut T` variant, the comment additionally cites:
- **CD3**: parallel workers receive disjoint `(start, len)` ranges by the dispatch loop's monotonic walk.
- **SCH3**: Phase 9's conflict graph prevents cross-system aliasing.

Pattern matches Phase 9's `par_iter.rs:run_chunk_raw` SAFETY block.

---

## §11 Test plan (architecture-level surface; tester writes the bodies)

### 11.1 Unit tests — correctness (per-archetype slice semantics)

`crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs` (new file, `#[cfg(test)] mod tests`):

| Test | Setup | Assertion |
|---|---|---|
| `single_archetype_yields_one_slice` | 1 archetype, 5 entities | Closure invoked exactly once; slice length = 5 |
| `multi_archetype_yields_distinct_slices` | 2 archetypes, 5 + 3 entities | Closure invoked exactly twice; total slice elements = 8 |
| `empty_archetype_skipped` | 1 archetype with 0 entities | Closure invoked 0 times |
| `stale_archetype_id_skipped` | Same setup as `iter.rs::stale_id_skipped` | Stale id continue-branch fires; closure receives only live archetypes |
| `single_component_read_sum` | 1 archetype, 100 entities, `Position(i)` for i in 0..100 | `for_each_chunk(\|s\| acc += s.iter().map(\|p\| p.0).sum::<u32>())` returns 4950 |
| `single_component_write_doubles` | 1 archetype, 100 entities | `for_each_chunk(\|s: &mut [_]\| s.iter_mut().for_each(\|p\| p.0 *= 2))`; reread via `iter()` confirms doubling |
| `tuple_3_yields_three_same_length_slices` | `Query<(&A, &mut B, &C)>` on 1 archetype, 7 entities | Closure receives `(a, b, c)`; `a.len() == b.len() == c.len() == 7` |
| `tuple_12_max_arity_compiles_and_iterates` | 12-component archetype | Compiles and yields the expected tuple shape |
| `empty_tuple_d_yields_unit_per_archetype` | `Query<(), With<Marker>>::for_each_chunk(\|()\|)` | Closure receives `()`; invocation count == matched archetype count |

### 11.2 Compile-fail tests (`trybuild`)

`crates/boyko_ecs/tests/compile_fail_chunk/`:

| Test file | Expected error | Reason |
|---|---|---|
| `changed_filter_rejected.rs` | `Changed<T>: ArchetypalQueryFilter` not satisfied | §3 gate |
| `added_filter_rejected.rs` | `Added<T>: ArchetypalQueryFilter` not satisfied | §3 gate |
| `ref_data_rejected.rs` | `Ref<'_, T>: ChunkedQueryData` not satisfied | §4.3 gate |
| `mut_data_rejected.rs` | `Mut<'_, T>: ChunkedQueryData` not satisfied | §4.3 gate |
| `or_with_changed_rejected.rs` | `Or<(With<A>, Changed<B>)>: ArchetypalQueryFilter` not satisfied | §5.2 propagation |
| `aliasing_query_mut_t_mut_t_rejected.rs` | Existing intra-system conflict B0002 from `FilteredAccessSet::init_access` | SystemParam path only: the test declares `fn sys(_: Query<(&mut T, &mut T), ()>) { … }` and registers it in a `Schedule`. The direct `EcsMaster::query::<(&mut T, &mut T), ()>()` API bypasses `FilteredAccessSet` (verified in critic Round 1 at `ecs_master.rs:1886-1939`: `query_cold_init` calls `QueryDataState::new` → `init_state`, never `init_access`); only the SystemParam dispatch path triggers the aliasing check. Verifies the chunk path doesn't silently bypass the `FilteredAccessSet` gate. |

The aliasing test is critical — it verifies that `Query<(&mut T, &mut T)>::for_each_chunk` rejection comes from `D::init_access` invoked by the SystemParam pipeline (same as `iter_mut`), not silently passes because we wrote a separate path. The test MUST be written as a system fn inside a `Schedule`; a direct `EcsMaster::query::<(&mut T, &mut T), ()>::for_each_chunk(...)` invocation would type-check (no `FilteredAccessSet` involved) and run — masking the bug.

### 11.3 Property tests (`proptest`)

`crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs::tests::proptest`:

| Property | Generator |
|---|---|
| `prop_chunked_sum_equals_iter_sum` | Random `n` ∈ [0, 1000], spawn `n` `Position(i32)` entities; assert `for_each_chunk + slice.iter().sum() == iter().map(\|p\| p.0).sum()` |
| `prop_chunked_mutation_matches_iter_mut_mutation` | Same setup; mutate via both APIs; final state identical |
| `prop_multi_archetype_total_rows_equals_entity_count` | 1..5 archetypes, 0..200 entities each; sum of slice lengths = total entities matched |

### 11.4 Parallel tests

| Test | Pool size | Assertion |
|---|---|---|
| `par_for_each_chunk_single_thread` | 1-worker pool | Identical results to `for_each_chunk` |
| `par_for_each_chunk_two_threads` | 2-worker pool | Same sum, parallel dispatch verified via thread-local counter |
| `par_for_each_chunk_eight_threads` | 8-worker pool | Same sum |
| `par_for_each_chunk_no_pool_fallback` | No `pool.install` | PAR7 fallback runs sequentially on calling thread (same path as `par_iter`) |
| `par_for_each_chunk_inline_below_min` | 1 archetype × 500 rows < MIN_ARCHETYPE_FOR_PARALLEL | Verifies PAR9 inline path invokes closure with the full slice |

### 11.5 Miri

Same suite as §11.1 + §11.3, run under `cargo +nightly miri test --lib`. **Excludes the parallel tests** (§11.4) — Phase 9.1 deferred multi-thread Miri due to Tree Borrows `protected-tag` interaction in `Scope::spawn` transmute. Document this as a known gap.

Specifically run:
- `chunk_iter::tests::*` (non-parallel correctness)
- `chunk_iter::tests::proptest::*` (small generators)
- The single-threaded `par_for_each_chunk_no_pool_fallback` (PAR7 path bypasses `scope.spawn`)

### 11.6 Bench

- `g6_for_each_chunk_sum_10k` (§8.2): PASS bar = boyko median ≥ 5× Bevy median over 60 criterion samples. Filed as a `cargo bench` gate; if it falls below 5×, treat as regression.
- Sanity bench: also run the existing `comparison_v2.rs` benches to confirm no regression on the per-row iter path (NCD elision must remain intact).
- Inner-loop autovec verification: `cargo +nightly asm --bench g6_for_each_chunk -- '<bench-monomorphisation-symbol>'` should show `vaddps`/`vmovups` (AVX2) in the user closure body. Absence = autovec broke; file before declaring Phase X.A done.
- §1.2 L1i-budget check (qualitative): `cargo +nightly asm --bench g6_for_each_chunk -- 'chunk_iter::for_each_chunk_impl'` and inspect the output by eye. The dispatch body should be a tight outer loop with one indirect call into the user closure — well under a hundred instructions, fitting comfortably in a small number of cache lines. **Do not** treat the textual asm character count (`wc -c`) as a byte budget: it counts mnemonic + operand characters and whitespace, not encoded x86-64 instruction byte length (which varies 1-15 B per insn). If the body looks bloated (visible monomorphisation cascade through tuple impls, multiple nested indirect calls, dozens of register-spill stores), file before declaring Phase X.A done.

### 11.7 Debug-assert invariants

The following `debug_assert!` calls must be added during implementation:

| File | Site | Assertion |
|---|---|---|
| `component_pool.rs::buffer_ptr` doc + caller | After `buffer_ptr` returned to dispatcher | `(ptr as usize) % SIMD_BUFFER_ALIGN == 0` |
| `chunk_iter.rs::dispatch_chunk` | After `archetype.entity_count()` read | `start + len <= entity_count` |
| `&T::set_chunk_readonly` body | After `columns.get_unchecked` | `!column.ptr.is_null()` (mirror existing QD2 assert) |
| `&mut T::set_chunk_mut` body | Same | Same |
| `&T::fetch_chunk` body | Before `from_raw_parts` | `len <= isize::MAX as usize / size_of::<T>().max(1)` (slice safety invariant) |

---

## §12 Step-by-step implementation plan

### Wave 1 — Foundations (parallelizable: 1A, 1B, 1C are independent)

**Step 1A — `SIMD_BUFFER_ALIGN` constant + Arena/ComponentPool alignment lift**

- *Test FIRST*: add `crates/boyko_ecs/src/ecs/memory/component_pool.rs::tests::buffer_ptr_is_simd_aligned`. **Rationale**: gates the entire wave; if this fails, the alignment lift is broken at the arena layer and the rest of the wave is meaningless. Write the test against the to-be-changed contract, watch it fail, then make the changes below.
- *File*: `crates/boyko_ecs/src/ecs/constants.rs` — add `pub const SIMD_BUFFER_ALIGN: usize = 32;` with doc comment (§6).
- *File*: `crates/boyko_ecs/src/ecs/memory/component_pool.rs` — in `ComponentPool::new` (find the buffer-allocation path), change the layout computation to use `align = component_layout.align().max(SIMD_BUFFER_ALIGN)`. Add doc to `buffer_ptr` describing the new guarantee (§6.3).
- *File*: same — add `debug_assert!((self.buffer.as_ptr() as usize) % SIMD_BUFFER_ALIGN == 0, "SIMD-A1 ...")` in `ComponentPool::new` right after the buffer allocation.

**Step 1B — `ArchetypalQueryFilter` marker trait + manual impls**

- *File*: `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` — at the end, before the test module:
  - Add the `ArchetypalQueryFilter` trait def (§2.1) with `#[diagnostic::on_unimplemented]` attribute.
  - Add `unsafe impl ArchetypalQueryFilter for ()` directly after the `()` `QueryFilter` impl.
  - Add `unsafe impl<C: Component> ArchetypalQueryFilter for With<C>` after `With<C>`.
  - Add `unsafe impl<C: Component> ArchetypalQueryFilter for Without<C>` after `Without<C>`.
  - Add the `impl_archetypal_filter_tuple!` macro (§5.2) + 12 invocations after the existing tuple-AND impl.
  - Add `unsafe impl<F: ArchetypalQueryFilter> ArchetypalQueryFilter for Or<F>` after `Or<F>`'s `QueryFilter` impl.
- *Test*: at the end of filter.rs, add `mod archetypal_marker_tests` with one `fn assert_archetypal<F: ArchetypalQueryFilter>()` and compile-only invocations for each member.

**Step 1C — `ChunkedQueryData` trait definition + scaffolding module**

- *File (new)*: `crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs`
  - Trait def (§2.2) — the full `unsafe trait ChunkedQueryData: QueryData` block with the four methods.
  - `#[diagnostic::on_unimplemented]` with helpful message pointing at `Ref`/`Mut` non-membership.
- *File*: `crates/boyko_ecs/src/ecs/core/iters/query/mod.rs` — add `pub mod chunked_data;` and re-export `pub use chunked_data::ChunkedQueryData;` to mirror the existing `QueryData` export pattern.

### Wave 2 — Leaf `ChunkedQueryData` impls (parallelizable: 2A, 2B, 2C independent)

**Step 2A — `&T: ChunkedQueryData` impl**

- *File*: `crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs` — add after the trait def:
  - `pub struct ReadChunkFetch<'c, T> { base: *const T, _marker: PhantomData<&'c [T]> }` with manual `Clone`/`Copy`.
  - `unsafe impl<T: Component> ChunkedQueryData for &T { ... }` with the four methods.
  - `set_chunk_readonly` body reads `(*archetype).columns.get_unchecked(state.id.0)` (mirror `QueryData::set_table_readonly` body in data.rs:367).
  - `set_chunk_mut` body delegates to `set_chunk_readonly` (mirror data.rs:373–386).
  - `fetch_chunk` body: `std::slice::from_raw_parts(fetch.base.add(start), len)` with the full SAFETY comment from §10.8.

**Step 2B — `&mut T: ChunkedQueryData` impl**

- Same file. Mirror of Step 2A:
  - `pub struct WriteChunkFetch<'c, T> { base: *mut T, _marker: PhantomData<&'c mut [T]> }`.
  - `unsafe impl<T: Component> ChunkedQueryData for &mut T { ... }`.
  - `set_chunk_readonly` body: `#[cold] #[inline(never)] panic!("CD4 violation: ...")` — mirror data.rs:540–547.
  - `set_chunk_mut` body: read column, set `fetch.base = column.ptr as *mut T`.
  - `fetch_chunk` body: `std::slice::from_raw_parts_mut(fetch.base.add(start), len)`.

**Step 2C — `(): ChunkedQueryData` impl**

- Same file. Trivial — all four methods are no-ops returning `()`. Mirror data.rs:1383–1401.

### Wave 3 — Tuple and `Or<F>`-style propagation (depends on Wave 2)

**Step 3A — `impl_chunked_query_data_tuple!` macro + 12 invocations**

- *File*: `crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs` — add the macro (§5.1) + arity 1–12 invocations (mirror data.rs:1406–1445).
- *Test*: a compile-only test inside the macro module that instantiates an arity-3 tuple `Query<(&A, &mut B, &C)>::for_each_chunk(|_| {})`.

**Step 3B — Update too-large stubs**

- *File*: `crates/boyko_ecs/src/ecs/core/iters/query/data.rs` — verify the `impl_query_data_tuple_too_large!` (arity 13–24) does NOT need a parallel `ChunkedQueryData` too-large stub. Confirmed in §5.4 — the existing `QueryData::init_state` panic fires first.

### Wave 4 — Sequential `Query::for_each_chunk` dispatcher (depends on Waves 1+2+3)

**Step 4 — Sequential dispatcher**

- *File (new)*: `crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs` — add the sequential driver, mirroring `iter.rs:165–294`'s outer-loop shape but yielding slices instead of items:

```rust
/// Phase X.A — sequential chunked-iter driver. Shared between
/// `Query::for_each_chunk` and `QueryView::for_each_chunk`.
///
/// # Safety
///
/// * `world` must satisfy the read/write contract of `D` (caller asserts).
/// * `state` must be synced against `world` (caller responsibility).
#[inline]
pub(crate) unsafe fn for_each_chunk_impl<'q, 's, D, F, Func>(
    state: &'s QueryDataState<D, F>,
    world: UnsafeEcsCell<'q>,
    mutable: bool,
    mut f: Func,
) where
    D: ChunkedQueryData,
    F: ArchetypalQueryFilter,
    Func: for<'c> FnMut(D::ChunkItem<'c>),
{
    let mut chunk_fetch = <D as ChunkedQueryData>::init_chunk_fetch(&state.data_state);

    for &arch_id in state.archetype_state.matched_ids() {
        // SAFETY (U_C2 / U_C3): mirror iter.rs:216-220 — Q5 stale-id skip.
        let arch_ptr: *mut Archetype = unsafe {
            if mutable {
                match world.archetype_ptr_mut(arch_id) {
                    Some(p) => p,
                    None => continue,
                }
            } else {
                match world.archetype_ptr(arch_id) {
                    Some(p) => p as *mut Archetype,
                    None => continue,
                }
            }
        };

        // SAFETY (U1/U2): slab-stable for the call scope.
        let entity_count = unsafe { (*arch_ptr).entity_count() };
        if entity_count == 0 { continue; }

        // SAFETY (CD1, CD4): write-capable / read-only dispatch chosen by
        //   `mutable`. The chunk-path is monomorphised per (D, F); since
        //   `D: ChunkedQueryData` excludes `Ref`/`Mut`, NCD is always
        //   false → no meta plumbing needed.
        unsafe {
            if mutable {
                <D as ChunkedQueryData>::set_chunk_mut(
                    &mut chunk_fetch, &state.data_state, arch_ptr);
            } else {
                <D as ChunkedQueryData>::set_chunk_readonly(
                    &mut chunk_fetch, &state.data_state, arch_ptr as *const _);
            }
        }

        // SAFETY (CD2): start = 0, len = entity_count, in-range.
        let item = unsafe { <D as ChunkedQueryData>::fetch_chunk(&chunk_fetch, 0, entity_count) };
        f(item);
    }
}
```

- *File*: `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` — add `pub fn for_each_chunk` to `impl<'w, 's, D, F> Query<'w, 's, D, F>`:

```rust
pub fn for_each_chunk<Func>(&mut self, f: Func)
where
    D: ChunkedQueryData,
    F: ArchetypalQueryFilter,
    Func: for<'c> FnMut(D::ChunkItem<'c>),
{
    // SAFETY (Q1, Q3, CD1-CD4): `&mut self` enforces cursor uniqueness;
    //   `D::IS_READ_ONLY` selects readonly/mut dispatch. Mirrors the
    //   `iter`/`iter_mut` split via a runtime flag (no separate driver
    //   needed since NCD = false at this monomorphisation).
    let mutable = !D::IS_READ_ONLY;
    unsafe {
        chunk_iter::for_each_chunk_impl(self.state, self.world, mutable, f);
    }
}
```

- *Test*: §11.1 tests in `chunk_iter.rs::tests`.

### Wave 5 — `QueryView::for_each_chunk` (depends on Wave 4)

**Step 5 — Direct-API wiring**

- *File*: `crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs` — add `pub fn for_each_chunk` on `impl<'w, D, F> QueryView<'w, D, F>` mirroring the `Query` method. Re-uses the same `chunk_iter::for_each_chunk_impl`.

### Wave 6 — Parallel `Query::par_for_each_chunk` (depends on Wave 4)

**Step 6 — Parallel driver**

- *File (new)*: `crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs` — implement `for_each_chunk_par_impl`, structured as the mirror of `par_iter.rs::for_each_impl` but:
  - Replaces `set_table_*` → `set_chunk_*`.
  - Replaces per-row loop with single `fetch_chunk(start, len)` + closure call.
  - Drops `meta` plumbing entirely.
  - Drops the NCD6 const-fold branch entirely.
- Define `ChunkChunkCaptures<D, F>` and its `Send` impl as in §9.4.
- Reuse `BatchingStrategy` (re-export from `par_iter`).
- *File*: `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` — add `pub fn par_for_each_chunk`:

```rust
pub fn par_for_each_chunk<Func>(&mut self, f: Func, batching: BatchingStrategy)
where
    D: ChunkedQueryData,
    F: ArchetypalQueryFilter,
    Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync,
{
    let mutable = !D::IS_READ_ONLY;
    unsafe {
        par_chunk::par_for_each_chunk_impl(self.state, self.world, mutable, batching, f);
    }
}
```

- *File*: `crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs` — mirror.

### Wave 7 — Tests + benches (depends on all previous)

**Step 7A — Unit + property + parallel tests** (§11.1, 11.3, 11.4 → into `chunk_iter.rs::tests` and `par_chunk.rs::tests`).

**Step 7B — Trybuild compile-fail tests** (§11.2)

- *File (new)*: `crates/boyko_ecs/tests/compile_fail_chunk.rs` driver:

```rust
#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_chunk/*.rs");
}
```

- *Files (new)*: 6 `.rs` files in `crates/boyko_ecs/tests/compile_fail_chunk/` per §11.2 table.

**Step 7C — Miri suite** (§11.5)

- Add `#[cfg(miri)] #[ignore]` to all parallel tests in `par_chunk.rs::tests`.
- No new files; just attribute additions on existing tests.

### Wave 8 — Bench (depends on Wave 7; needs nightly toolchain)

**Step 8A — Per-package toolchain pin**

- *File (new)*: `D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\rust-toolchain.toml` per §8.1.
- *Verification*: `cargo +nightly build --release` succeeds at workspace root; `cargo bench --bench g6_for_each_chunk` from inside `crates/bench_bevy_vs_boyko/` resolves the per-package nightly.

**Step 8B — Bench harness**

- *File (new)*: `crates/bench_bevy_vs_boyko/benches/g6_for_each_chunk.rs` per §8.2.
- *File*: `crates/bench_bevy_vs_boyko/Cargo.toml` — append `[[bench]] name = "g6_for_each_chunk" harness = false`.
- *Verification*: `cargo +nightly bench --bench g6_for_each_chunk` produces criterion HTML report with median ratio ≥ 5×.

### Wave 9 — Documentation (depends on Wave 8)

**Step 9A — Internal docs**

- *File*: `docs/SYSTEMS.md` — add `Query::for_each_chunk` and `Query::par_for_each_chunk` entries pointing at the new files.
- *File*: `docs/FEATURE_MAP.md` — add "I want batched/SIMD-friendly iteration → `Query::for_each_chunk` in `chunk_iter.rs`".

**Step 9B — Public book page**

- *Coordination*: file a Phase X.A doc-writer task. Out of scope for the developer; the developer's responsibility ends at correct rustdoc on the new public items.

### Dependency graph (developer can parallelize):

```
Wave 1A ┐
Wave 1B ┼─→ Wave 2A, 2B, 2C (parallel) ─→ Wave 3A ─┬─→ Wave 4 ─┬─→ Wave 5 ─┐
Wave 1C ┘                                  Wave 3B  │            └─→ Wave 6 ─┤
                                                    └─→ Wave 7B (no impl dep)│
                                                                              └─→ Wave 7A, 7C ─→ Wave 8 ─→ Wave 9
```

Estimated developer time: 4–6 days for impl (Waves 1–6); 1 day tests + Miri (Wave 7); 1 day bench + tuning (Wave 8); 0.5 day docs (Wave 9). Total: **~1 week**, matching the roadmap budget.

---

## §13 Risk register

### Risk 1 — Marker trait gate fails to compose with `Or<F1, F2>`

**Trigger**: A user writes `Query<&T, Or<(With<A>, Changed<B>)>>::for_each_chunk(...)`. We want this to fail to compile because `Changed<B>` is non-archetypal.

**Verification**: `Or<F>` is impl'd `ArchetypalQueryFilter` iff `F: ArchetypalQueryFilter` (§5.2). For `F = (With<A>, Changed<B>)`, the tuple's `ArchetypalQueryFilter` impl requires both elements to be archetypal. `Changed<B>` is NOT, so the tuple isn't, so `Or<...>` isn't. Compile error fires at the `for_each_chunk` call site with the `#[diagnostic::on_unimplemented]` message.

**Mitigation**: Compile-fail test `or_with_changed_rejected.rs` (§11.2) catches any regression.

**Residual risk**: very low. The trait composition is the standard tuple-folding pattern used by `IS_ARCHETYPAL` (data.rs:1244) and `NEEDS_CHANGE_DETECTION` (data.rs:1249) propagations — both verified shipping correctly through Phase 12.5.

### Risk 2 — Lifetime variance: `for<'c>` HRTB makes some closure shapes uninvokable

**Trigger**: User writes a closure that captures a `&'a Foo` and tries to return a slice from the closure scope.

```rust
let mut acc: Vec<&u32> = Vec::new();
q.for_each_chunk(|s: &[Foo]| acc.push(&s[0].x));  // FAILS: 'c does not outlive 'a
```

**Why this fails**: `'c` is a fresh per-call lifetime; the slice borrow does not outlive the closure body. The user's `acc` lives in the outer scope, so `'c` cannot satisfy `'a`.

**Mitigation**:
- Documentation note in `for_each_chunk` doc that the slice does not outlive the call (mirrors `slice::chunks` docs).
- The user can either (a) copy the value (`acc.push(s[0].x)` instead of `&s[0].x`), or (b) accept that they should use `iter()` for this pattern.
- Compile error is clear and actionable.

**Residual risk**: low. This is intentional Rust borrow-checker behaviour, identical to `slice::chunks` / `Iterator::for_each` patterns. Users encounter it once, learn the pattern, and move on.

### Risk 3 — Tuple macro expansion bloats compile time

**Trigger**: 12 tuple-arity invocations + 12 archetypal-filter-tuple invocations × per-impl method bodies → ~250–400 new generated impl methods.

**Mitigation**:
- Cap arity at 12 (matches existing `MAX_QUERY_DATA_ARITY`). No too-large stubs needed for `ChunkedQueryData` (§5.4).
- Each method body is small (3–6 lines of trivial forwarding code) — well within rustc's normal codegen budget.
- The existing `QueryData` tuple macros (data.rs:1230–1363) emit 11 methods × 12 arities = 132 functions per crate. Adding 4 more methods × 12 arities = 48 new functions. Phase 10 expansion (78 impls) already shipped without compile-time complaints.

**Residual risk**: low. Tester step records `cargo check` time before/after on the boyko_ecs crate. Acceptable if delta < 10%.

### Risk 4 — Alignment lift in Arena breaks an existing invariant

**Trigger**: `ComponentPool::new` now requests an `align ≥ 32` Layout. The Arena's `free_blocks.allocate_aligned(size, align)` must support `align = 32` without regressing other allocations.

**Verification**:
- `Arena::allocate_layout` (arena.rs:90) forwards `layout.align()` unchanged to `allocate_aligned`. Existing test `arena_allocate_typed_returns_correct_alignment` (arena.rs:301) covers `align = 32` (via `#[repr(align(32))] struct Fat`).
- `MemFreeBlockMaster::allocate_aligned` (existing impl) supports arbitrary power-of-2 alignment via best-fit + alignment-up. 32 is already in the supported range.

**Mitigation**:
- Step 1A adds a unit test `buffer_ptr_is_simd_aligned` in `component_pool.rs::tests` that creates a `ComponentPool<f32>` (or via the type-erased path with `Layout::new::<f32>()`) and asserts `(pool.buffer_ptr() as usize) % SIMD_BUFFER_ALIGN == 0`.
- If this fails, the alignment math in `ComponentPool::new` is wrong; isolate before any downstream code touches it.

**Residual risk**: medium-low. The Arena allocator is well-tested. The risk is a regression in some downstream code that assumed `align = align_of::<T>()` exactly (e.g., a manual offset calc somewhere). Mitigation: Step 1A is intentionally Wave 1's first task so any breakage surfaces before later waves depend on it.

### Risk 5 — Bench fails to demonstrate ≥5× speedup (Phase 12.6 g4 variance recurrence)

**Trigger**: Bench measurement shows 1–3× boyko-over-Bevy median, below the 5× pass bar.

**Possible causes** (and detections):

1. **`f32::algebraic_add` not actually autovectorizing** → cargo asm inspection (instructed in §8.3) shows scalar `addss` instead of `vaddps`. Fix: force `RUSTFLAGS=-Ctarget-cpu=native` in the bench harness.
2. **Bench harness variance** (Phase 12.6 g5d signal-to-noise issue) → run with `--measurement-time 30 --warm-up-time 5 --sample-size 100`; compare medians, not means.
3. **Bevy actually autovectorizes too** when the fold body is simple enough → the speedup compresses. Acceptable result; the underlying win is real even if the headline number drops. Mitigation: include a multi-component bench `g6b_for_each_chunk_3comp_sum` that exercises a triple SoA load — Bevy's per-row tuple state machine has more state, widening the gap.

**Mitigation**:
- Tester step verifies the boyko-side asm shows `vaddps` (or equivalent) using `cargo +nightly asm bench_bevy_vs_boyko::g6_for_each_chunk::<closure>`. If absent, file as a bug before declaring Phase X.A done.
- The bench PASS bar is set at the lower edge of the expected range (5×, vs 5–8× expected). Buffer for variance.
- If the 5× bar fails on the single-component bench but passes on the multi-component bench, that's still a credible win — file Phase X.A.1 (single-comp re-tune) as follow-up rather than blocking the phase.

**Residual risk**: medium. Phase 12.6 documented variance issues; the Phase X.A bench addresses them by (a) using `algebraic_add` on both sides for fairness, (b) increasing sample size, (c) including the asm verification step.

---

## §14 Open questions returned to orchestrator

**None.** All four research-open-questions are resolved in this plan:

1. **Q1 (sibling vs GAT extension)**: Resolved §4 → sibling trait `ChunkedQueryData`. Justification: 78-impl blast radius vs 15-impl additive surface; sibling preserves backward compat; `Ref`/`Mut` exclusion falls out naturally.
2. **Q2 (alignment story)**: Resolved §6 → lift `ComponentPool` allocations to `max(align_of::<T>(), 32)`. Cost bounded (<1 MB realistic waste), zero runtime cost, documented invariant verifiable via debug_assert.
3. **Q3 (`for_each_chunk_mut` explicit vs inferred)**: Resolved implicitly throughout — type-inferred from `D` (current pattern). One method `for_each_chunk` takes `&mut self`, dispatches readonly vs mut via `D::IS_READ_ONLY` runtime flag inside the impl.
4. **Q4 (bench harness toolchain)**: Resolved in the prompt + §8 → nightly with `f32::algebraic_add` + `rust-toolchain.toml` at `crates/bench_bevy_vs_boyko/` (per-package; see §8.1).

The only deferral within scope is **`fold_chunks` reducing variant** (§2.6) — explicitly filed as Phase 13.X with rationale. This is not an open question; it's a documented out-of-scope decision.

---

### Pre-return checklist

Verified against the architect-role checklist:

- **Plan structure**: goal stated in perf + functional terms (§1); target metrics concrete (§1.2 table); every decision (§3–§9) carries a perf/cache/parallelism justification with rejected alternatives listed in trade-off tables.
- **Data structures**: `ChunkedQueryData` + `ArchetypalQueryFilter` + `ChunkChunkCaptures<D, F>` all show field roles + repr where applicable (§2.2, §9.4); size/alignment is bounded by reusing existing 16-B `Column`.
- **API**: minimal surface (4 new public methods total: 2 on `Query`, 2 on `QueryView`); no internal types leak (`ChunkFetch<'c>` is `pub(crate)`); HRTBs explicit on closures (§2.3); no `dyn Trait` on any path; generics provide static dispatch.
- **Multithreading**: model described (PAR1-9 from Phase 9 reused; §9); no shared atomic state introduced; `ChunkChunkCaptures` `Send` impl justified (§9.4); `Send + Sync` propagation explicit.
- **Correctness**: edge cases (empty archetype, stale id, 0 entities) handled (§4 dispatcher pseudocode); no version bump (Phase X.A is additive); drop order n/a (no new owned heap memory); SAFETY invariants for every unsafe block enumerated (§10.8).
- **Integration**: affected modules listed (§12 file paths); existing APIs untouched (Phase 12.6 perf and Phase 10 NCD remain intact — `iter`/`iter_mut` not touched); compatible with Arena/ComponentPool/UnitId via `buffer_ptr` reuse; implementation plan broken into 9 waves with explicit parallelization graph.
- **Validation**: tests categorized (§11.1–11.7); benches specified (§8.2, §11.6); debug_assert sites enumerated (§11.7).

The Round 3 plan is ready for the developer (Wave 1 already in flight; Wave 8 has the toolchain fixes locked in).

Files relevant to this design (absolute paths, for the developer's reference):

- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\query.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\iter.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\par_iter.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\data.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\filter.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\query_view.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\mod.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\arena.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\constants.rs (target for `SIMD_BUFFER_ALIGN`)
- D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\Cargo.toml (bench registration)
- D:\claude\BoykoEngine\docs\PHASE-X.A-RESEARCH.md (research input — already saved)
- D:\claude\BoykoEngine\docs\PHASE-13-ROADMAP.md (roadmap; Phase X.A lines 80–103)
- D:\claude\BoykoEngine\docs\PHASE-12.6-RESEARCH-QUERY-BEAT.md (motivating residual)

New files to be created by the developer (per §12):

- D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\rust-toolchain.toml (Wave 8A)
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\chunked_data.rs (Wave 1C)
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\chunk_iter.rs (Wave 4)
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\par_chunk.rs (Wave 6)
- D:\claude\BoykoEngine\crates\boyko_ecs\tests\compile_fail_chunk.rs (Wave 7B)
- D:\claude\BoykoEngine\crates\boyko_ecs\tests\compile_fail_chunk\*.rs (Wave 7B, 6 files)
- D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\benches\g6_for_each_chunk.rs (Wave 8B)
