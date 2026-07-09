> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 8b — `Query<D, F>` typed DSL (architectural plan)

**Status:** Round 2 (revised after architecture-critic Round 1 review). Implementation depends on Phase 7 (landed), Phase 8a (landed). Builds on the SystemParam trait scaffolding plus Phase 7's column-table fast read path.
**Branch (when active):** `ecs`.
**Plan author:** architect agent.
**Scope:** sub-phase 8b only. Phases 8c (`IntoSystem` + function-as-system), 8d (`Commands` buffer), and Phase 10 (change detection — `Changed<C>` / `Added<C>`) are explicitly out of scope.

---

## 0. Changes from Round 1

This Round 2 plan resolves every critical and major item raised by `architecture-critic` against the Round 1 plan, plus the quality items. Item-by-item resolution:

### Critical fixes (4)

| Tag | Resolution | Sections touched |
|-----|------------|------------------|
| **C1** — `IntoIterator` impls missing | Added `IntoIterator for &Query<...> where D: ReadOnlyQueryData` and `IntoIterator for &mut Query<...>` impls in §3.1 and §14.1. Added a compile-only `IntoIterator` check to Step 8. Per-Query `iter()`/`iter_mut()` retained — the `IntoIterator` impls delegate to them. | §3.1, §3.5 (new), §14.1, §18 Step 8, §19.1 |
| **C2** — `allows_mutable_access` access for debug-assert | Dropped the field-level debug-assert in `Query::iter_mut`. The `ReadOnlyQueryData` bound on `iter()` plus the existing `archetype_ptr_mut` debug-assert (Phase 8a `UnsafeEcsCell::archetype_ptr_mut: debug_assert!(self.allows_mutable_access)`) cover the contract. Updated SAFETY commentary in §3.1; struck the debug-assert from §19.4. | §3.1, §19.4 |
| **C3** — Malformed `SystemParam` impl head | Renamed both impl lifetimes to a single binder: `unsafe impl<'a, 'b, D, F> SystemParam for Query<'a, 'b, D, F>`. Added a compile-only `assert_impl::<Query<'_, '_, &A>>()` test in Step 8 that exercises the generic blanket. Updated §21.8 R8 to reference the binder shape. | §14.3, §18 Step 8, §21.8 |
| **C4** — `Or<F>` complexity unspecified + `F::is_or_filter()` undefined | Removed every reference to `F::is_or_filter()` (no such helper exists). Spelled out the worst-case complexity for `Query<(), Or<F>>` in §6.4 and §15.5 (cold scan O(archetype_count × Or-arity) once per generation bump; ~5 ns/archetype). `post_filter_matched` runs unconditionally with no `is_or_filter` short-circuit. | §6.1, §6.4, §15.5, §19.4 |

### Major fixes (8)

| Tag | Resolution | Sections touched |
|-----|------------|------------------|
| **M1** — Dual-structure invariant fragile | Added INVARIANT comment on `QueryDataState` + new `#[cfg(debug_assertions)] fn assert_dual_invariant()` called at the end of `post_filter_matched`. | §6.1, §6.3 |
| **M2** — `*const → *mut` cast Tree-Borrows uncertainty | Split `QueryData::set_table` into `set_table_readonly(_: *const Archetype)` and `set_table_mut(_: *mut Archetype)`. ReadOnly iter calls only `set_table_readonly`; mutable iter calls only `set_table_mut`. The `*const → *mut` cast in `QueryIter::next` is eliminated. | §4.1, §4.2, §4.3, §4.5, §4.6, §5.1, §5.2, §5.3, §5.4, §5.5, §7.1, §9.1, §9.2, §9.3, §21.5 |
| **M3** — `set_table` cost budget Phase-10 contribution | Rephrased §1.2 to: "≤ 50 ns per archetype boundary for Phase 8b's all-archetypal filter set; +10 ns per non-archetypal filter element added in Phase 10." | §1.2 |
| **M4** — Macro pseudo-syntax `[< state_ $d >]` non-working | Replaced with a paired-ident macro `impl_query_data_tuple!((D0, s0), (D1, s1), ...)`. No `paste!` dependency. Added a concrete arity-3 worked-example expansion as an appendix in §4.6. | §4.6, §10.1, §21.2, §25 (new appendix) |
| **M5** — Rename callsites under-enumerated | Enumerated every callsite via grep (10 lines below). Added a paragraph in §17.3 explaining why the alternative (keeping `iters::query::Query` legacy) is rejected: the in-place migration of `iter_one`/`iter_two` happens INSIDE the legacy file, so it cannot be cleanly moved without splitting methods. | §17.3 |
| **M6** — `QueryDataState` size formula wrong | Corrected §13.3 to use the formula `~280 (QueryState) + sum(D::State sizes) + sum(F::State sizes) + PhantomData(0)`. Arity-12 worst case ~568 B; quoted ranges updated. | §13.3 |
| **M7** — `state.update(master)` borrow lifetime SAFETY note | Added a SAFETY paragraph in §14.3 explaining the `&ArchetypeMaster` borrow expires before the `Query` constructor runs; no aliasing with the cell-by-value chain. | §14.3 |
| **M8** — `Or<F>::aggregate_*` default-inheritance fragile | Explicit override `fn aggregate_include(...) {}` + `fn aggregate_exclude(...) {}` no-op bodies on the `Or<F>` impl (locks the contract). | §5.4 |

### Quality items (5)

| Tag | Resolution | Sections touched |
|-----|------------|------------------|
| **O1** | Paired-ident macro syntax now concrete (covered by M4). | §4.6, §25 |
| **O2** | Exit criterion #6 in §1.1 now references `LegacyQuery` (post-rename). | §1.1 |
| **O3** | Step 14 added explicitly to §18 (`cargo expand` golden test). | §18 Step 14 (new) |
| **O4** | Post-filter `high_water_mark` optimisation mentioned in §21 as a Phase-10-or-later improvement. | §21.9 (new) |
| **O5** | Added one sentence in §17.3 confirming the `LegacyQuery` rename resolves the two-vs-three-lifetime naming concern (no other lifetime symbol collides). | §17.3 |

### Deferral tracking

Items deferred beyond Phase 8b are named in §22 with explicit "Phase N" tags. No new deferrals introduced by Round 2.

---

## 1. Goal and target metrics

### 1.1 Goal

Deliver the **`Query<D, F>` typed DSL** as a first-class `SystemParam`. The user's contract:

```rust
fn movement(mut q: Query<(&mut Position, &Velocity), Without<Frozen>>) {
    for (mut pos, vel) in &mut q {
        pos.x += vel.x;
        pos.y += vel.y;
    }
}
ecs.run_closure_once::<Query<(&mut Position, &Velocity), Without<Frozen>>, _, _>(movement);
```

Every Phase 8b artefact must:
1. Reuse the existing Phase 5c `QueryState` archetype-match cache (no new cache — extend the existing one).
2. Drive iteration through the Phase 7 column-table read path (`archetype.columns[c.0].ptr.add(row * stride as usize)`), bypassing the slow `ComponentPoolBundle::get_pool` indirection.
3. Compose cleanly with Phase 8a's `SystemParam` trait — `Query<D, F>` IS a `SystemParam`, and its tuple co-uses with `Res`/`ResMut` work through the existing `FilteredAccessSet` aliasing-detection.

Exit criteria for 8b:

1. The `QueryData` and `QueryFilter` traits exist (Bevy-shape, GAT-based) and compile.
2. Tuple impls for `QueryData` cover arity 1..=12 via a single `macro_rules!` site; tuple impls for `QueryFilter` cover the same range via `Or<(F0, .., F11)>`.
3. `&T`, `&mut T` are `QueryData`; `With<C>`, `Without<C>`, `Or<F>` are `QueryFilter`.
4. `Query<'w, 's, D, F>` is the `Item<'w, 's>` of the `SystemParam` `Query<D, F>` for every valid `(D, F)`.
5. `Query<D, F>::iter()` and `Query<D, F>::iter_mut()` yield `D::Item<'w>` per matched row, walking matched archetypes archetypal-major.
6. **`for x in &q { ... }` and `for x in &mut q { ... }` desugar via `IntoIterator for &Query<...>` (gated by `D: ReadOnlyQueryData`) and `IntoIterator for &mut Query<...>` respectively** (C1). The existing `LegacyQuery<'a>` (post-rename) callers (`iter_one`, `iter_two`, `from_archetypes`, `with`, `with_component_ids`, `with_mask`, `with_exact_mask`, `archetypes`, `iter`, `IntoIterator`) keep compiling and passing all tests. Their internals migrate from `component_pools.get_pool(...)` to `archetype.columns[c.0]` direct reads (project-analyst's known issue #6).
7. Intra-system access conflict detection works (e.g. `(Query<&mut A>, Query<&mut A>)` panics at `init_access`).
8. `cargo test --all-targets` green; new unit tests for `QueryData`/`QueryFilter`/`Query<D,F>::iter*`; integration test running an end-to-end movement system through `run_closure_once`.
9. No `dyn Trait`, no `Box<dyn Trait>`, no `HashMap`, no `Mutex`/`RwLock`/`RefCell` on the iteration hot path.

### 1.2 Target metrics (release, AMD Zen3 / Intel Alder Lake)

Per `docs/plans/PHASE-08-system-api.md` §"Performance targets":

| Operation | Target | Cache profile |
|-----------|--------|---------------|
| `Query<&A>::iter().next()` per row (steady-state) | ≤ 6 ns (parity with Phase 2d `iter_one` ~5 ns) | 1 L1d hit per row |
| `Query<(&A, &B)>::iter().next()` per row | ≤ 9 ns (parity with Phase 2d `iter_two` ~7-8 ns) | 2 L1d hits |
| `Query<&mut A>::iter_mut().next()` per row | ≤ 6 ns (symmetric with `&A` — same column-table access) | 1 L1d hit |
| **Archetype transition cost (`set_table_*`)** | **≤ 50 ns per archetype boundary for Phase 8b's all-archetypal filter set; +10 ns per non-archetypal filter element added in Phase 10** (M3) | 1 `*const Archetype` deref + `columns[c]` per `D` element |
| `Query<D, F>::iter()` cold construction (first call after init) | ≤ 200 ns | `update_archetypes` if generation moved; `set_table_*` for first archetype |
| `Query` `init_state` (per system registration) | ≤ 1 µs | `D::init_state` + `F::init_state` + `QueryState::new` |
| `Query` `init_access` | ≤ 200 ns (per param) | `D::Reads`/`Writes` declared via `FilteredAccessSet` |
| Warm `QueryState::update_archetypes` (no delta) | ≤ 4 ns (existing Phase 5c warm path; reused verbatim) | 1 load + compare |

Phase 8a's `run_closure_once` overhead caveat (~960 ns dispatch) applies until Phase 8c lands `FunctionSystem`. Hot-path numbers are measurable in micro-benches that call `Query::iter()` directly, bypassing `run_closure_once`.

### 1.3 Cross-phase relation to perf

Phase 7's `get_component_raw` (~3 ns single-component random access) is the absolute lower bound. Phase 8b's per-row cost is `(column.ptr.add(row * stride) → &T)` — the same single dependent load as Phase 7, minus the inland → archetype indirection (the archetype pointer is cached in `Fetch` per `set_table_*`). So the per-row hot loop should be **faster** than `get_component_raw`'s amortised cost: Phase 7 paid 3-4 cache lines for random access; Phase 8b pays 1 cache line per row inside one archetype (the column pointer is already cached; only the component bytes are loaded).

The per-archetype boundary cost (`set_table_*`) is amortised: for any archetype with ≥ 8 entities, the per-row cost dominates, and the hot loop is bound by L1d throughput on the component data itself.

---

## 2. Context and constraints

### 2.1 Subsystems affected

| Subsystem | Touch type |
|-----------|-----------|
| `Query<'a>` → renamed to `LegacyQuery<'a>` (existing `iters/query.rs` → `iters/legacy_query.rs`) | **In-place migration**: `iter_one`/`iter_two` internals switch from `component_pools.get_pool` to direct `archetype.columns[c.0]` reads. Public API (constructors, `iter`, `iter_one`, `iter_two`, `archetypes`, `len`, `is_empty`, `IntoIterator`) unchanged — same tests pass under the new name. |
| `QueryState` (existing `iters/query_state.rs`) | **Unchanged** for archetype-matching logic. New helpers added: `matched_ids_mut`, `remove_matched_at`, `last_observed_generation`, `last_observed_structural`. |
| New file `iters/query/data.rs` | `QueryData` trait + `&T`, `&mut T`, tuple impls. |
| New file `iters/query/filter.rs` | `QueryFilter` trait + `With<C>`, `Without<C>`, `Or<F>`, `()`. |
| New file `iters/query/state.rs` | `QueryDataState<D, F>` — the per-system state holding the existing `QueryState` plus `D::State` + `F::State`. |
| New file `iters/query/iter.rs` | `QueryIter<'w, 's, D, F>` + `QueryIterMut<'w, 's, D, F>` — the actual iterators with cached `Fetch`. |
| New file `iters/query/query.rs` | `Query<'w, 's, D, F>` — the `SystemParam`'s `Item`, with `iter()`/`iter_mut()` methods and `IntoIterator` impls. |
| `core/system/system_param.rs` | No change to the trait. `Query<D, F>: SystemParam` impl lives in `iters/query/query.rs`. |
| `iters/mod.rs` | Adds `pub mod query` (the new module); existing `pub mod query` (legacy) renamed to `pub mod legacy_query`; re-exports `pub use legacy_query::Query as LegacyQuery;` and `pub use query::Query;`. |
| `EcsMaster` | No new methods in 8b. The `Query<D, F>` SystemParam is consumed only through `run_closure_once::<Query<D, F>, _, _>(...)` already-shipped pathway. |
| `boyko_macros` | No changes — `#[derive(Component)]` is enough; the new traits work on Component types directly. |

### 2.2 Invariants that must be preserved

- **U1-U14 from Phase 7** (slab stability, generation match, pointer minting, drop discipline).
- **C1 / U_C1, U_C2, U_C3 from Phase 8a** (`UnsafeEcsCell` by-value receivers, mutable provenance flow).
- **SP1, SP2, SP4 from Phase 8a** (`SystemParam` access-declaration honesty, get_param protocol, init no-structural-mutation).
- **AB-R1 from Phase 8a** (clear-bit-first replace protocol in `add_archetype`).

### 2.3 New invariants introduced by Phase 8b

| Tag | Statement |
|-----|-----------|
| **QD1** (QueryData soundness) | `D::component_ids()` returns the COMPLETE set of `ComponentId`s `D::fetch` will read or write. Violation = SP1 violation. Enforced by `#[derive(QueryData)]` (deferred to Phase 8c) and by-hand-impl auditing in Phase 8b. |
| **QD2** (Fetch initialisation) | `D::Fetch<'w>` is `Copy`+`Clone` and starts with all column pointers NULL (via `D::init_fetch()`). EXACTLY ONE of `D::set_table_readonly` / `D::set_table_mut` overwrites them before any `D::fetch(row)` call. Violation = null-pointer deref UB. Asserted by `debug_assert!(!fetch.col_X.is_null())` in `D::fetch` for `D ∈ {&T, &mut T}`. |
| **QD3** (Fetch lifetime) | `D::Fetch<'w>` is bound to `'w`, the world-access scope. The cached column pointers within `Fetch` carry the same provenance as `*mut Archetype` (from `archetype_ptr_mut`) or `*const Archetype` (from `archetype_ptr`), itself scoped to `'w` (Phase 7 U1/U2). Cannot outlive the `Query<'w, 's, D, F>` that holds the `Fetch`. |
| **QD4** (Read/write set_table dispatch, M2) | `QueryIter::next` (read-only cursor) MUST call `D::set_table_readonly(_: *const Archetype)`. `QueryIterMut::next` MUST call `D::set_table_mut(_: *mut Archetype)`. The trait makes a `*const → *mut` cast impossible at the iter level — neither method's signature permits the wrong-kind pointer. |
| **QF1** (Archetypal filter contract) | A `QueryFilter` with `IS_ARCHETYPAL = true` MUST return `true` from `filter_fetch` for every row of every archetype that satisfies `matches_component_set`. Asserted via `debug_assert!` in Phase 8b builds; the const-folded `if const { F::IS_ARCHETYPAL } { /* skip filter_fetch */ }` codepath relies on this. |
| **QF2** (Filter access declaration) | `F::init_access` declares any component reads/writes the filter performs. `With<C>` / `Without<C>` declare **NO** access. `Changed<C>` / `Added<C>` (Phase 10) would declare a read. Enforced by manual review in 8b; auditable via `cargo expand`. |
| **Q1** (Query mutability flow) | A `Query<D, F>` instance whose `D` contains `&mut T` for any `T` enforces write-capable provenance via the `D::set_table_mut(_: *mut Archetype)` signature — the cursor `QueryIterMut::next` calls `cell.archetype_ptr_mut(id)` (a method that debug-asserts `allows_mutable_access == true` per Phase 8a). `Query<D, F>` with `D: ReadOnlyQueryData` calls only `cell.archetype_ptr(id)` (read-only mint). The `iter()` method on `Query` is gated by `D: ReadOnlyQueryData`; the `iter_mut()` method has no extra debug-assert (Q1 is upheld by the type-level bound on `iter()` + the cell's existing `archetype_ptr_mut` debug-assert). |
| **Q2** (Aliasing across copies) | `UnsafeEcsCell` is `Copy`; multiple `Query` instances may exist in a tuple SystemParam. Aliasing safety is upheld by the `FilteredAccessSet` accumulator at `init_access` time: `(Query<&mut A>, Query<&A>)` panics with `ComponentReadVsWrite` before any `get_param` runs. |
| **Q3** (Iter cursor uniqueness) | Within a single `Query` instance, only one `QueryIter` / `QueryIterMut` may be alive at a time. Enforced by the borrow checker: `iter(&self)` reborrows the `Query`'s archetype-id slice; `iter_mut(&mut self)` reborrows mutably. The cached `Fetch` lives inside the iter struct (not the query) so re-iter is sound. |
| **Q4** (Empty archetype handling) | Archetypes matched by the filter but with `entity_count() == 0` MUST be skipped by the iter (advance to next archetype) — no `D::fetch` on a row that does not exist. The cursor's `current_len = arch.entity_count()` and `current_row < current_len` guard enforce this. |
| **Q5** (Stale archetype id handling) | Matched `ArchetypeId`s in `QueryState::matched_ids` may refer to archetypes that have since been `remove_archetype`-ed (structural generation bump). The iter cursor MUST handle this case via `world.archetype_ptr*(id)` returning `None` and skipping to the next id. Reuses the existing `QueryStateIter` defensive pattern. |
| **QS1** (Dual-structure invariant on QueryDataState, M1) | `QueryState.matched_ids: Vec<ArchetypeId>` and `QueryState.matched_archetypes: ArchetypeBitSet` MUST stay synchronised: a bit is set in the bitset iff the id appears in `matched_ids`. Mutation paths: `update_archetypes` (insert) and `remove_matched_at` (delete via `swap_remove` + `bitset.remove`). Enforced by debug-only `assert_dual_invariant()` called at the end of `post_filter_matched`. |

### 2.4 Hard prohibitions on the iteration hot path

| Forbidden | Why | Allowed substitute |
|-----------|-----|--------------------|
| `Box<dyn QueryData>` | virtual dispatch | monomorphisation via `QueryData` impl per concrete tuple |
| `HashMap<ComponentId, Column>` lookup | hash cost + cache miss | `archetype.columns[c.0]` direct array index |
| `ComponentPoolBundle::get_pool` | sparse-map traversal | `archetype.columns[c.0].ptr` — cached at `refresh_column` time |
| `Vec::push` inside iter cursor | per-row allocation | iterator returns `D::Item<'w>` per call; caller-side `.collect::<Vec<_>>()` is the user's choice |
| `RefCell<QueryState>` | runtime borrow check | `&'s QueryState` via `state: &'s mut Self::State` GAT scope |
| Any heap allocation per `iter()` | per-frame allocation | `QueryState::matched_ids: Vec<ArchetypeId>` is grown at `init_state` / `update_archetypes` — never inside `iter()` |
| Branch on `F::IS_ARCHETYPAL` in inner loop | branch mispredict + I-cache cost | const-fold via `if const { F::IS_ARCHETYPAL } { ... }` (rustc 1.79+); the dead branch is DCE'd at monomorphization |
| Generic dispatch on `&T` vs `&mut T` Fetch | dyn-style cost | separate types `ReadFetch<T>` (holds `*const T`) and `WriteFetch<T>` (holds `*mut T`) |
| **`*const Archetype → *mut Archetype` cast in read-only iter** (M2) | Tree Borrows uncertainty | Split `QueryData::set_table` into `set_table_readonly(_: *const Archetype)` and `set_table_mut(_: *mut Archetype)`. Each iter calls the kind-correct method. |

### 2.5 Variadic arity ceiling

Same as Phase 8a: **`MAX_QUERY_DATA_ARITY = 12`**, mirroring `MAX_SYSTEM_PARAM_ARITY = 12`. Const-panic stubs for `13..=24` via the same `const { panic!(...) }` pattern. The ceiling lives as `pub const MAX_QUERY_DATA_ARITY: usize = 12;` in `iters/query/data.rs`.

---

## 3. Decision D1 — `Query<'w, 's, D, F>` layout and lifetimes

### 3.1 Decision

`Query<'w, 's, D, F>` is the `Item<'w, 's>` of the `SystemParam` `Query<D, F>` — a small struct (3 pointers + 1 PhantomData) held by the system body for the duration of the system call. Constructed by `get_param` on every system invocation; destroyed when the system returns.

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/query.rs (new)

use std::marker::PhantomData;

use crate::ecs::core::iters::query::data::{QueryData, ReadOnlyQueryData};
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::iters::query::iter::{QueryIter, QueryIterMut};
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

pub struct Query<'w, 's, D: QueryData, F: QueryFilter = ()> {
    /// Borrow of the per-system state — holds the cached `QueryState`,
    /// `D::State`, `F::State`. `'s` is the state's lifetime, bound to the
    /// containing system's stored state slot.
    state: &'s QueryDataState<D, F>,

    /// Copy of the world-access cell. By-value pass; not retagged.
    world: UnsafeEcsCell<'w>,

    /// `SystemMeta` borrow for diagnostic hooks (e.g. `new_archetype`).
    meta: &'s SystemMeta,

    /// Invariance over `D` and `F`. `fn() -> (D, F)` keeps the marker
    /// `Send + Sync` regardless of `D`/`F` bounds.
    _marker: PhantomData<fn() -> (D, F)>,
}

impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    /// Returns the number of currently-matched archetypes.
    #[inline]
    pub fn archetype_count(&self) -> usize {
        self.state.archetype_state.matched_ids().len()
    }

    /// Returns `true` if no archetypes are currently matched.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.state.archetype_state.matched_ids().is_empty()
    }

    /// Returns a read-only iterator over `D::Item<'w>` for every entity in
    /// every matched archetype.
    ///
    /// `D` must be **read-only** at the type level — see [`QueryData::IS_READ_ONLY`].
    /// For mutable iteration use [`iter_mut`](Self::iter_mut).
    pub fn iter(&self) -> QueryIter<'_, 's, D, F>
    where
        D: ReadOnlyQueryData,
    {
        // SAFETY (Q1, QD4): `D: ReadOnlyQueryData` ⇒ no `&mut T` in `D`; the
        //   QueryIter constructor will call `cell.archetype_ptr(_)` (read-only
        //   mint) and `D::set_table_readonly(_: *const Archetype)` only.
        unsafe { QueryIter::new(self.state, self.world) }
    }

    /// Returns a mutable iterator over `D::Item<'w>` for every entity in
    /// every matched archetype.
    ///
    /// `iter_mut` is the only iter method that works for `D` containing
    /// `&mut T`. The `&mut self` borrow guarantees no other live cursor
    /// exists (Q3).
    ///
    /// # Q1 enforcement
    ///
    /// No field-level debug-assert is needed here. Q1 is upheld by:
    /// * The type system — `iter()` is gated by `D: ReadOnlyQueryData`; if
    ///   the user calls `iter()` on a `D` containing `&mut T`, the bound
    ///   fails to resolve.
    /// * Phase 8a's existing `UnsafeEcsCell::archetype_ptr_mut` carries a
    ///   `debug_assert!(self.allows_mutable_access)` inside the cell. Any
    ///   path that calls `archetype_ptr_mut` on a read-only cell trips
    ///   that debug-assert at the cell level.
    pub fn iter_mut(&mut self) -> QueryIterMut<'_, 's, D, F> {
        // SAFETY (Q1, Q3, QD4): `&mut self` enforces cursor uniqueness;
        //   QueryIterMut::new will call `cell.archetype_ptr_mut(_)` per
        //   archetype boundary. If `world` carries a read-only mint and `D`
        //   were not gated, the cell's own debug-assert fires.
        unsafe { QueryIterMut::new(self.state, self.world) }
    }
}
```

### 3.2 `IntoIterator` impls (C1)

To support the `for x in &q { ... }` and `for x in &mut q { ... }` sugar in the goal example, `Query<D, F>` provides standard `IntoIterator` impls.

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/query.rs (new, continued)

/// `IntoIterator` for a shared reference to a Query — desugars `for x in &q`
/// into `(&q).into_iter()`. Gated by `D: ReadOnlyQueryData` so that `&q` over
/// a `Query<&mut T, _>` is a type error (forcing the user to `&mut q`).
impl<'a, 'w, 's, D, F> IntoIterator for &'a Query<'w, 's, D, F>
where
    D: ReadOnlyQueryData,
    F: QueryFilter,
{
    type Item = D::Item<'a>;
    type IntoIter = QueryIter<'a, 's, D, F>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        // Delegates to the inherent `iter` method. The lifetime narrows to
        // `'a` (the shared reborrow scope), strictly inside `'w` (the world
        // access scope). `D::Item<'a>` is a sub-lifetime of `D::Item<'w>`.
        self.iter()
    }
}

/// `IntoIterator` for an exclusive reference to a Query — desugars
/// `for x in &mut q` into `(&mut q).into_iter()`. Accepts any `D`/`F`.
impl<'a, 'w, 's, D, F> IntoIterator for &'a mut Query<'w, 's, D, F>
where
    D: QueryData,
    F: QueryFilter,
{
    type Item = D::Item<'a>;
    type IntoIter = QueryIterMut<'a, 's, D, F>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}
```

### 3.3 Why no `last_run` / `this_run` ticks

Change detection (`Changed<C>` / `Added<C>`) is Phase 10. The Phase 8b `SystemMeta` already carries `last_archetype_generation` and `last_structural_generation`; those fields are sufficient for Phase 8b's archetype-cache refresh. Adding tick fields preemptively bloats `SystemMeta` and Query without yielding any 8b benefit.

### 3.4 Why no `&'w mut Archetype` references pre-baked

An archetype set lives across the iter's hot loop, but materialising `&'w mut Archetype` for every matched archetype at `get_param` time would require N reborrows on a `Copy` cell — and would leak provenance to the cursor in a way that prevents the inner loop from being moved between archetypes. The cell-by-value approach mirrors Bevy: every `set_table_*` call freshly mints `*const Archetype` / `*mut Archetype` via `cell.archetype_ptr*(id)`, scoped to the cursor's own lifetime.

### 3.5 Why `'w` AND `'s` (two-lifetime GAT)

Same rationale as Phase 8a §13.1. `'w` is the world-access scope (the system call duration); `'s` is the state scope (the system's stored state slot lifetime). `Query<'w, 's, D, F>::iter()` returns `QueryIter<'_, 's, D, F>` where the first lifetime is `'_ = &Query`'s shared borrow scope, not `'w`. This keeps cursor borrows local to the iter call — re-iter is sound even if cursor types contain non-`Copy` Fetch state.

### 3.6 Alternatives considered and rejected

- **Hold `Vec<&'w Archetype>` in Query**: forbidden — requires pre-materialising live references at `get_param` time. Forces `cell.world().archetype_master().get_archetype(id)` for every matched id, which is `&self`-flavoured and produces SharedReadOnly provenance — `iter_mut` would then need to re-mint via `cell.world_mut().archetype_master_mut().archetype_ptr_for(id)`, defeating the cell's by-value Copy property.
- **Hold the `Fetch` in Query itself, not in the iter**: forbidden — would force `iter()` to take `&mut self`. Then concurrent `iter()` and `iter_mut()` would be impossible even when `D: ReadOnlyQueryData`.
- **Drop `'s` lifetime, use only `'w`**: forbidden — collapses state-lifetime bookkeeping. The iter would need `state: &'w QueryDataState<...>`, but `'w` is the world borrow scope, not the state slot's.

### 3.7 Trade-off

Holding `&'s QueryDataState` instead of inlining the state copies the state pointer once per system invocation (8 B fixed cost). Acceptable: the alternative (inlining the state by value into Query) would require `QueryDataState: Copy`, which `Vec<ArchetypeId>` inside `QueryState` precludes.

---

## 4. Decision D2 — `QueryData` trait shape

### 4.1 Decision

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/data.rs (new)

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::identifiers::primitives::ComponentId;

/// Per-row data fetched by a `Query<D, F>`.
///
/// Implemented by:
/// * `&T` for any `T: Component` — yields `&'w T` per row.
/// * `&mut T` for any `T: Component` — yields `&'w mut T` per row.
/// * Tuples of `QueryData` up to arity 12 — yields a tuple of element items.
///
/// # Trait shape — three GATs
///
/// * `State` — long-lived per-system caches (e.g. cached `ComponentId`s).
///   `Send + Sync + 'static` for Phase 9 cross-thread migration.
/// * `Fetch<'w>` — per-archetype cached column pointers. Held inside the
///   iterator (not the query), so re-iter is sound. `Copy` so the variadic
///   tuple impl can destructure cleanly.
/// * `Item<'w>` — the per-row yielded value (e.g. `&'w T` or `&'w mut T`).
///
/// # Split set_table (M2)
///
/// `QueryData::set_table` is split into two kind-correct methods:
/// * `set_table_readonly(_: *const Archetype)` — called by `QueryIter::next`
///   when the cursor is read-only. Never produces write-capable provenance
///   downstream.
/// * `set_table_mut(_: *mut Archetype)` — called by `QueryIterMut::next`
///   when the cursor is mutable. Pointer carries write-capable provenance
///   (minted via `UnsafeEcsCell::archetype_ptr_mut`).
///
/// For read-only `QueryData` (`&T` and tuples of read-only), `set_table_mut`
/// is implemented as `unsafe { set_table_readonly(fetch, state, archetype as *const _) }`
/// — i.e., for read-only data, the two paths converge to the same code. For
/// `&mut T`, `set_table_readonly` is forbidden: the impl `panic!()`s at
/// runtime (would be `unreachable!()` in release; the read-only cursor
/// never calls it because the type-level `D: ReadOnlyQueryData` bound on
/// `Query::iter()` rules out `&mut T`).
///
/// # `IS_READ_ONLY` const
///
/// Compile-time flag for read-vs-write classification. `&T` and tuples of
/// read-only data have `IS_READ_ONLY = true`; `&mut T` has `false`. Used by
/// `Query::iter()` to gate read-only iteration (Q1).
///
/// # Safety
///
/// Implementations MUST uphold:
///
/// 1. **QD1** — `init_state` produces a State whose embedded `ComponentId`s
///    cover every component that `fetch(row)` will read or write. Reflected
///    in `init_access`.
/// 2. **QD2** — `init_fetch` produces a `Fetch<'w>` with all column pointers
///    NULL. Exactly one of `set_table_readonly` / `set_table_mut` overwrites
///    them with valid pointers before any `fetch(row)` call.
/// 3. **QD3** — `Fetch<'w>` lifetime is bound to `'w`; cached pointers are
///    scoped to the `*const/*mut Archetype` minted by `UnsafeEcsCell` for `'w`.
/// 4. **QD4** — `QueryIter::next` calls only `set_table_readonly`;
///    `QueryIterMut::next` calls only `set_table_mut`. The split signature
///    structurally prevents the wrong-kind dispatch.
pub unsafe trait QueryData: Sized {
    type State: Send + Sync + 'static;
    type Fetch<'w>: Copy;
    type Item<'w>;
    const IS_READ_ONLY: bool;

    fn init_state(world: &mut EcsMaster) -> Self::State;

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet);

    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool;

    fn aggregate_include(state: &Self::State, include: &mut ComponentMask);

    fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w>;

    /// Sets the `Fetch`'s cached column pointers from a read-only archetype
    /// pointer. Called by `QueryIter::next` (the read-only cursor).
    ///
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*const Archetype` for `'w`, with
    ///   provenance from `UnsafeEcsCell::archetype_ptr(id)` (read-only mint).
    /// * `archetype` MUST contain every `ComponentId` in `state`.
    /// * For `D` containing `&mut T`, this method MUST NOT be called. Impls
    ///   for `&mut T` `panic!()` here as a runtime backstop; the type-level
    ///   `D: ReadOnlyQueryData` bound on `Query::iter()` prevents this in
    ///   well-typed code.
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    );

    /// Sets the `Fetch`'s cached column pointers from a write-capable
    /// archetype pointer. Called by `QueryIterMut::next` (the mutable cursor).
    ///
    /// # Safety
    ///
    /// * `archetype` MUST be a live `*mut Archetype` for `'w`, with
    ///   write-capable provenance from `UnsafeEcsCell::archetype_ptr_mut(id)`.
    /// * `archetype` MUST contain every `ComponentId` in `state`.
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    );

    /// Returns the per-row value for `row`.
    ///
    /// # Safety
    ///
    /// * `fetch` MUST have been initialised by a prior `set_table_*` call.
    /// * `row < archetype.entity_count()` of the archetype that `set_table_*`
    ///   cached.
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w>;
}

/// Marker trait for `QueryData` that performs no writes.
///
/// # Safety
///
/// Implementations MUST be `QueryData` impls whose `IS_READ_ONLY = true`.
pub unsafe trait ReadOnlyQueryData: QueryData {}

pub const MAX_QUERY_DATA_ARITY: usize = 12;
```

### 4.2 `&T` impl

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/data.rs (new, continued)

#[derive(Clone, Copy)]
pub struct ReadState<T: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> T>,
}

#[derive(Clone, Copy)]
pub struct ReadFetch<'w, T: Component> {
    pub(crate) base: *const T,
    pub(crate) _marker: PhantomData<&'w T>,
}

// SAFETY (QD1-QD4):
//   - QD1: `state.id` is `T::component_id()`; `init_access` declares a read.
//   - QD2: `init_fetch` sets `base = ptr::null()`; either `set_table_readonly`
//     or `set_table_mut` overwrites before any `fetch` call. (Note: for `&T`,
//     both methods do the same thing — read column.ptr as *const T.)
//   - QD3: `Fetch<'w>` lifetime is `'w`.
//   - QD4: both set_table_* methods share the same body (read-only data
//     doesn't care about the pointer kind); split exists for the mutable
//     case in `&mut T`.
unsafe impl<T: Component> QueryData for &T {
    type State = ReadState<T>;
    type Fetch<'w> = ReadFetch<'w, T>;
    type Item<'w> = &'w T;
    const IS_READ_ONLY: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        ReadState { id: T::component_id(), _marker: PhantomData }
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        access_set
            .add_component_read(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict|
                crate::ecs::core::system::params::diagnostics::intra_system_conflict_panic(conflict));
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        ReadFetch { base: std::ptr::null(), _marker: PhantomData }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // SAFETY (QD3): `archetype` is a live `*const Archetype` for `'w`;
        //   `columns` is at offset 0; `state.id.0 < MAX_COMPONENTS`.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.base = column.ptr as *const T;
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // For `&T`, the mutable variant degrades to the same read. Re-borrow
        // as *const internally; no write-capable provenance is consumed.
        // SAFETY: same conditions as set_table_readonly with the additional
        //   guarantee that the caller (mutable cursor) holds a fresh
        //   archetype_ptr_mut mint — strictly stronger than what we need.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _) }
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY (QD2, QD3): set_table_* was called; row in range; lifetime
        //   bound via PhantomData<&'w T>.
        unsafe { &*fetch.base.add(row) }
    }
}

// SAFETY: `&T: QueryData` has `IS_READ_ONLY = true`.
unsafe impl<T: Component> ReadOnlyQueryData for &T {}
```

### 4.3 `&mut T` impl

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/data.rs (new, continued)

#[derive(Clone, Copy)]
pub struct WriteState<T: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> T>,
}

#[derive(Clone, Copy)]
pub struct WriteFetch<'w, T: Component> {
    pub(crate) base: *mut T,
    pub(crate) _marker: PhantomData<&'w mut T>,
}

// SAFETY (QD1-QD4):
//   - QD1: state.id is T::component_id(); init_access declares a WRITE.
//   - QD2: set_table_mut overwrites base; set_table_readonly panics (QD4).
//   - QD3: lifetime bound by PhantomData<&'w mut T>.
//   - QD4: `set_table_readonly` is forbidden for &mut T — the type system
//     prevents the call (Query::iter() requires D: ReadOnlyQueryData, which
//     &mut T does not implement). The runtime panic is a defence-in-depth
//     backstop, expected to be `unreachable_unchecked!()` in release. Phase
//     8b ships `panic!()` for diagnosability; Phase 11+ can demote to
//     `unreachable_unchecked` after Miri verification.
unsafe impl<T: Component> QueryData for &mut T {
    type State = WriteState<T>;
    type Fetch<'w> = WriteFetch<'w, T>;
    type Item<'w> = &'w mut T;
    const IS_READ_ONLY: bool = false;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        WriteState { id: T::component_id(), _marker: PhantomData }
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        access_set
            .add_component_write(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict|
                crate::ecs::core::system::params::diagnostics::intra_system_conflict_panic(conflict));
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        WriteFetch { base: std::ptr::null_mut(), _marker: PhantomData }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // QD4: read-only cursor calling on &mut T data is forbidden by
        // the trait gate D: ReadOnlyQueryData on Query::iter(). Reaching
        // this branch indicates a contract violation by a hand-written
        // QueryData impl (it implemented ReadOnlyQueryData for a type
        // containing &mut T). Panic loudly.
        panic!(
            "QD4 violation: set_table_readonly called for &mut T (T = {}). \
             Did a custom QueryData impl falsely claim ReadOnlyQueryData?",
            std::any::type_name::<T>()
        );
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (Q1, QD3): archetype carries write-capable provenance
        //   (archetype_ptr_mut). columns at offset 0; id.0 < MAX_COMPONENTS.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        // column.ptr is *mut u8 (Phase 7 U7: write-capable provenance at
        // refresh_column time). Cast preserves the Unique tag.
        fetch.base = column.ptr as *mut T;
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY (QD2, Q1): set_table_mut set base; row in range; no alias
        //   per FilteredAccessSet + cursor &mut self.
        unsafe { &mut *fetch.base.add(row) }
    }
}

// NOTE: NO `ReadOnlyQueryData for &mut T` impl.
```

### 4.4 Why three GATs

Same as Bevy. State (long-lived caches), Fetch (per-archetype scratch), Item (per-row value).

### 4.5 Alternatives considered and rejected

- **Unified `set_table(_: *mut Archetype)` signature with downstream `*const` casts**: rejected (M2). Tree Borrows uncertainty on the `*const → *mut` cast in the read-only path; Miri may flag it. Split signature eliminates the cast.
- **Single method `set_table` returning Fetch by value**: rejected — Fetch is mutated incrementally per element in tuple impls; by-value return would force a recompose.
- **No `Fetch` GAT, fetch from `*mut Archetype` directly per row**: rejected — adds `columns[c.0]` indexing per row (1 cache line per element).
- **`Fetch` as `&'w Column`**: rejected — `&'w Column` invalidates if `refresh_column` (in-place mutation under `&mut Archetype`) runs. Phase 7 D5 confirmed `refresh_column` only on `add_pool` (`&mut`), so the iter path is safe, but storing by-value bypasses the concern entirely.

### 4.6 Tuple impl strategy (M4)

A single `macro_rules!` site with **paired-ident invocations** emits the `QueryData` and `ReadOnlyQueryData` impls for arity 1..=12. The pairing `(D0, s0), (D1, s1), ...` provides distinct value-ident bindings for `state` and `fetch` destructuring without `paste!`.

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/data.rs (new, continued)

macro_rules! impl_query_data_tuple {
    ( $( ($D:ident, $s:ident, $f:ident) ),* ) => {
        // SAFETY (QD1-QD4): each element upholds QD1-QD4 by its own
        //   contract; the tuple impl forwards element-by-element.
        unsafe impl< $($D: QueryData),* > QueryData for ( $($D,)* ) {
            type State = ( $($D::State,)* );
            type Fetch<'w> = ( $($D::Fetch<'w>,)* );
            type Item<'w> = ( $($D::Item<'w>,)* );

            const IS_READ_ONLY: bool = true $( && $D::IS_READ_ONLY )*;

            #[inline]
            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$D as QueryData>::init_state(world), )* )
            }

            #[inline]
            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)* ) = state;
                $( <$D as QueryData>::init_access($s, access_set); )*
            }

            #[inline]
            fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
                let ( $($s,)* ) = state;
                true $( && <$D as QueryData>::matches_component_set($s, mask) )*
            }

            #[inline]
            fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
                let ( $($s,)* ) = state;
                $( <$D as QueryData>::aggregate_include($s, include); )*
            }

            #[inline]
            fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
                let ( $($s,)* ) = state;
                ( $( <$D as QueryData>::init_fetch($s), )* )
            }

            #[inline]
            unsafe fn set_table_readonly<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    // SAFETY: forwarded per-element; archetype is the same
                    //   for every element (one archetype per set_table call).
                    unsafe { <$D as QueryData>::set_table_readonly($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn set_table_mut<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    unsafe { <$D as QueryData>::set_table_mut($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
                let ( $($f,)* ) = fetch;
                ( $( unsafe { <$D as QueryData>::fetch($f, row) }, )* )
            }
        }

        // SAFETY: ReadOnlyQueryData gated on element ReadOnlyQueryData.
        unsafe impl< $($D: ReadOnlyQueryData),* > ReadOnlyQueryData for ( $($D,)* ) {}
    };
}

impl_query_data_tuple!((D0, s0, f0));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2));
// ... continuing up to arity 12 ...
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11)
);
```

**Arity 13..=24 stub macro**:

```rust
macro_rules! impl_query_data_tuple_too_large {
    ( $( ($D:ident, $s:ident, $f:ident) ),* ) => {
        unsafe impl< $($D: QueryData),* > QueryData for ( $($D,)* ) {
            type State = ();
            type Fetch<'w> = ();
            type Item<'w> = ();
            const IS_READ_ONLY: bool = true;

            fn init_state(_world: &mut EcsMaster) -> Self::State {
                const { panic!("QueryData arity > MAX_QUERY_DATA_ARITY = 12; restructure your query.") }
            }
            // ... all other methods similarly: const { panic!(...) } ...
        }
    };
}

impl_query_data_tuple_too_large!(
    (D0, s0, f0), (D1, s1, f1), /* ... */, (D12, s12, f12)
);
// ... up to arity 24.
```

A concrete arity-3 expanded form is shown in §25 (appendix).

### 4.7 Trade-off

Each tuple impl is 100-200 lines of generated code per arity. 12 arities × ~150 lines = ~1.8K LOC compiled. Acceptable: monomorphises tightly, no dyn dispatch.

---

## 5. Decision D3 — `QueryFilter` trait shape

### 5.1 Decision

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/filter.rs (new)

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::identifiers::primitives::ComponentId;

/// Filter applied to query matches. Same split-set_table shape as QueryData
/// (M2) — `set_table_readonly(_: *const)` for read-only cursors,
/// `set_table_mut(_: *mut)` for mutable cursors. For archetypal-only filters
/// (the Phase 8b set: `With`, `Without`, `Or`, `()`), both methods are no-ops.
///
/// # Safety
///
/// 1. **QF1** — If `IS_ARCHETYPAL = true`, `filter_fetch` returns `true`
///    unconditionally.
/// 2. **QF2** — `init_access` declares any component reads performed in
///    `filter_fetch`. Archetypal-only filters declare nothing.
pub unsafe trait QueryFilter: Sized {
    type State: Send + Sync + 'static;
    type Fetch<'w>: Copy;
    const IS_ARCHETYPAL: bool;

    fn init_state(world: &mut EcsMaster) -> Self::State;
    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet);
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool;

    #[inline]
    fn aggregate_exclude(_state: &Self::State, _exclude: &mut ComponentMask) {}

    #[inline]
    fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {}

    fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w>;

    /// Caches per-archetype state from a read-only archetype pointer.
    /// Called by `QueryIter::next`.
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    );

    /// Caches per-archetype state from a write-capable archetype pointer.
    /// Called by `QueryIterMut::next`.
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    );

    /// Returns `true` if the row should be yielded.
    /// Archetypal filters return `true` unconditionally (QF1).
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool;
}
```

### 5.2 `With<C>` impl

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/filter.rs (new, continued)

pub struct With<C: Component>(PhantomData<fn() -> C>);

#[derive(Clone, Copy)]
pub struct WithState<C: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> C>,
}

unsafe impl<C: Component> QueryFilter for With<C> {
    type State = WithState<C>;
    type Fetch<'w> = ();
    const IS_ARCHETYPAL: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        WithState { id: C::component_id(), _marker: PhantomData }
    }

    #[inline]
    fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {}

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {}

    #[inline]
    unsafe fn set_table_readonly<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {}

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {}

    #[inline]
    unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool { true }
}
```

### 5.3 `Without<C>` impl

```rust
pub struct Without<C: Component>(PhantomData<fn() -> C>);

#[derive(Clone, Copy)]
pub struct WithoutState<C: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> C>,
}

unsafe impl<C: Component> QueryFilter for Without<C> {
    type State = WithoutState<C>;
    type Fetch<'w> = ();
    const IS_ARCHETYPAL: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        WithoutState { id: C::component_id(), _marker: PhantomData }
    }

    #[inline]
    fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {}

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        !mask.contains(state.id)
    }

    #[inline]
    fn aggregate_exclude(state: &Self::State, exclude: &mut ComponentMask) {
        exclude.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {}

    #[inline]
    unsafe fn set_table_readonly<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {}

    #[inline]
    unsafe fn set_table_mut<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {}

    #[inline]
    unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool { true }
}
```

### 5.4 `Or<F>` impl (M8 + C4)

`Or<(F0, F1, ..)>` matches an archetype iff ANY tuple element matches. **Explicitly overrides `aggregate_include` and `aggregate_exclude` as no-ops** (M8) — the OR predicate is non-decomposable into the simple include/exclude mask, so `Or` contributes nothing to those aggregates. The post-filter pass in `QueryDataState` enforces the OR semantics via `matches_component_set`.

**Complexity (C4)**: `Query<(), Or<F>>` (empty include + Or filter) populates `matched_ids` with EVERY live archetype via `update_archetypes` (cold path, once per generation bump), then `post_filter_matched` scans linearly: O(archetype_count × Or-arity). For boyko's 1024-archetype ceiling, worst-case ~1024 × 12 = 12,288 mask-checks at ~5 ns each = ~60 µs once per generation bump. Acceptable: (a) Or queries are rare; (b) cold path only on generation bump; (c) per-archetype mask check is constant.

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/filter.rs (new, continued)

pub struct Or<F>(PhantomData<fn() -> F>);

macro_rules! impl_or_filter_tuple {
    ( $( ($F:ident, $s:ident, $f:ident) ),* ) => {
        unsafe impl< $($F: QueryFilter),* > QueryFilter for Or<( $($F,)* )> {
            type State = ( $($F::State,)* );
            type Fetch<'w> = ( $($F::Fetch<'w>,)* );
            const IS_ARCHETYPAL: bool = true $( && $F::IS_ARCHETYPAL )*;

            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$F as QueryFilter>::init_state(world), )* )
            }

            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)* ) = state;
                $( <$F as QueryFilter>::init_access($s, access_set); )*
            }

            #[inline]
            fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
                let ( $($s,)* ) = state;
                false $( || <$F as QueryFilter>::matches_component_set($s, mask) )*
            }

            // M8: Explicit override — Or contributes NOTHING to include/exclude
            // mask aggregation. The OR predicate is enforced via the post-
            // filter pass in QueryDataState. Locked here to prevent future
            // contributors adding a non-trivial default impl that would
            // silently break the QueryDataState contract.
            #[inline]
            fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {}
            #[inline]
            fn aggregate_exclude(_state: &Self::State, _exclude: &mut ComponentMask) {}

            fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
                let ( $($s,)* ) = state;
                ( $( <$F as QueryFilter>::init_fetch($s), )* )
            }

            unsafe fn set_table_readonly<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    unsafe { <$F as QueryFilter>::set_table_readonly($f, $s, archetype); }
                )*
            }

            unsafe fn set_table_mut<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $(
                    unsafe { <$F as QueryFilter>::set_table_mut($f, $s, archetype); }
                )*
            }

            #[inline]
            unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
                if const { Self::IS_ARCHETYPAL } {
                    return true;
                }
                let ( $($f,)* ) = fetch;
                false $( || unsafe { <$F as QueryFilter>::filter_fetch($f, row) } )*
            }
        }
    };
}

impl_or_filter_tuple!((F0, s0, f0));
impl_or_filter_tuple!((F0, s0, f0), (F1, s1, f1));
// ... up to arity 12 ...
```

### 5.5 `()` (no-filter) impl

```rust
unsafe impl QueryFilter for () {
    type State = ();
    type Fetch<'w> = ();
    const IS_ARCHETYPAL: bool = true;

    #[inline] fn init_state(_world: &mut EcsMaster) -> Self::State {}
    #[inline] fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {}
    #[inline] fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool { true }
    #[inline] fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {}
    #[inline] unsafe fn set_table_readonly<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *const Archetype) {}
    #[inline] unsafe fn set_table_mut<'w>(_f: &mut Self::Fetch<'w>, _s: &Self::State, _a: *mut Archetype) {}
    #[inline] unsafe fn filter_fetch<'w>(_fetch: &Self::Fetch<'w>, _row: usize) -> bool { true }
}
```

### 5.6 Tuple-as-AND impl

Tuple of `QueryFilter` = implicit AND. Same paired-ident macro as §4.6:

```rust
macro_rules! impl_query_filter_tuple_and {
    ( $( ($F:ident, $s:ident, $f:ident) ),* ) => {
        unsafe impl< $($F: QueryFilter),* > QueryFilter for ( $($F,)* ) {
            type State = ( $($F::State,)* );
            type Fetch<'w> = ( $($F::Fetch<'w>,)* );
            const IS_ARCHETYPAL: bool = true $( && $F::IS_ARCHETYPAL )*;

            fn init_state(world: &mut EcsMaster) -> Self::State {
                ( $( <$F as QueryFilter>::init_state(world), )* )
            }

            fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
                let ( $($s,)* ) = state;
                $( <$F as QueryFilter>::init_access($s, access_set); )*
            }

            #[inline]
            fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
                let ( $($s,)* ) = state;
                true $( && <$F as QueryFilter>::matches_component_set($s, mask) )*
            }

            #[inline]
            fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
                let ( $($s,)* ) = state;
                $( <$F as QueryFilter>::aggregate_include($s, include); )*
            }

            #[inline]
            fn aggregate_exclude(state: &Self::State, exclude: &mut ComponentMask) {
                let ( $($s,)* ) = state;
                $( <$F as QueryFilter>::aggregate_exclude($s, exclude); )*
            }

            fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
                let ( $($s,)* ) = state;
                ( $( <$F as QueryFilter>::init_fetch($s), )* )
            }

            unsafe fn set_table_readonly<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *const Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $( unsafe { <$F as QueryFilter>::set_table_readonly($f, $s, archetype); } )*
            }

            unsafe fn set_table_mut<'w>(
                fetch: &mut Self::Fetch<'w>,
                state: &Self::State,
                archetype: *mut Archetype,
            ) {
                let ( $($f,)* ) = fetch;
                let ( $($s,)* ) = state;
                $( unsafe { <$F as QueryFilter>::set_table_mut($f, $s, archetype); } )*
            }

            #[inline]
            unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
                if const { Self::IS_ARCHETYPAL } {
                    return true;
                }
                let ( $($f,)* ) = fetch;
                true $( && unsafe { <$F as QueryFilter>::filter_fetch($f, row) } )*
            }
        }
    };
}

impl_query_filter_tuple_and!((F0, s0, f0));
impl_query_filter_tuple_and!((F0, s0, f0), (F1, s1, f1));
// ... up to arity 12 ...
```

### 5.7 Trade-off

`Or<F>` contributes nothing to mask aggregation. Post-filter scans every matched archetype once per generation bump; ~5 ns × #archetypes. See §6.4 / §15.5 for the explicit complexity analysis.

---

## 6. Decision D4 — `QueryDataState<D, F>`: per-system state composition

### 6.1 Decision

Reuse the existing `iters/query_state.rs::QueryState` verbatim. Phase 8b wraps it with `D::State` + `F::State` + a post-filter pass.

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/state.rs (new)

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::iters::query::data::QueryData;
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::iters::query_state::QueryState;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;

/// Per-system state for `Query<D, F>`.
///
/// # INVARIANT (Phase 8b POST_FILTER) — M1
///
/// The `archetype_state.matched_ids: Vec<ArchetypeId>` and
/// `archetype_state.matched_archetypes: ArchetypeBitSet` MUST stay
/// synchronised: for every `id` in `matched_ids`, the bit `id.0` MUST be set
/// in `matched_archetypes`, and vice versa.
///
/// Mutation paths preserving the invariant:
/// * `QueryState::update_archetypes` (insert): pushes to `matched_ids` and
///   sets the bit (existing Phase 5c logic).
/// * `QueryState::remove_matched_at` (delete): `swap_remove` from
///   `matched_ids` and `bitset.remove(id.0)` (new helper §6.2).
///
/// `assert_dual_invariant()` (debug-only) is called at the end of
/// `post_filter_matched` to verify the invariant after every mutation pass.
pub struct QueryDataState<D: QueryData, F: QueryFilter> {
    pub(crate) archetype_state: QueryState,
    pub(crate) data_state: D::State,
    pub(crate) filter_state: F::State,
    _marker: PhantomData<fn() -> (D, F)>,
}

impl<D: QueryData, F: QueryFilter> QueryDataState<D, F> {
    pub fn new(world: &mut EcsMaster) -> Self {
        let data_state = <D as QueryData>::init_state(world);
        let filter_state = <F as QueryFilter>::init_state(world);

        let mut include = ComponentMask::new();
        let mut exclude = ComponentMask::new();
        let optional = ComponentMask::new();

        <D as QueryData>::aggregate_include(&data_state, &mut include);
        <F as QueryFilter>::aggregate_include(&filter_state, &mut include);
        <F as QueryFilter>::aggregate_exclude(&filter_state, &mut exclude);

        let mut archetype_state = QueryState::new(include, exclude, optional);

        archetype_state.update_archetypes(world.archetype_master());
        Self::post_filter_matched(
            &mut archetype_state,
            &data_state,
            &filter_state,
            world.archetype_master(),
        );

        Self {
            archetype_state,
            data_state,
            filter_state,
            _marker: PhantomData,
        }
    }

    /// Trims `archetype_state.matched_ids` by re-applying
    /// `D::matches_component_set` AND `F::matches_component_set` to each id.
    ///
    /// Worst case complexity: O(matched_ids.len() × (D-arity + F-arity)).
    /// For `Query<(), Or<F>>` (empty include + Or filter), `matched_ids`
    /// starts as the FULL archetype set (mask is empty ⇒ matches everything),
    /// so the cost is O(archetype_count × Or-arity) per generation bump.
    /// Phase 8b accepts this cost (C4 explanation in §6.4).
    fn post_filter_matched(
        archetype_state: &mut QueryState,
        data_state: &D::State,
        filter_state: &F::State,
        master: &ArchetypeMaster,
    ) {
        let mut idx = 0;
        // Borrow-then-drop pattern: pull the slice mutable, walk it, mutate
        // via swap_remove. The borrow ends at each loop iteration boundary
        // so we can call other &mut self methods.
        loop {
            let ids = archetype_state.matched_ids();
            if idx >= ids.len() {
                break;
            }
            let id = ids[idx];
            let pass = master
                .get_archetype(id)
                .is_some_and(|arch| {
                    let mask = arch.component_mask();
                    <D as QueryData>::matches_component_set(data_state, mask)
                        && <F as QueryFilter>::matches_component_set(filter_state, mask)
                });
            if pass {
                idx += 1;
            } else {
                archetype_state.remove_matched_at(idx);
                // idx unchanged — swapped-in element needs checking.
            }
        }

        // M1: verify the dual-structure invariant after every mutation pass.
        #[cfg(debug_assertions)]
        Self::assert_dual_invariant(archetype_state);
    }

    /// M1 — debug-only invariant check: matched_ids and matched_archetypes
    /// bitset are mutually consistent.
    #[cfg(debug_assertions)]
    fn assert_dual_invariant(archetype_state: &QueryState) {
        let ids = archetype_state.matched_ids();
        let bitset = archetype_state.matched_archetypes_bitset();
        // Every id in matched_ids must have its bit set.
        for id in ids {
            debug_assert!(
                bitset.contains(id.0),
                "QS1 violation: id {} in matched_ids but bit not set in bitset",
                id.0
            );
        }
        // Bit count must equal matched_ids length (bijection).
        debug_assert_eq!(
            bitset.popcount(),
            ids.len(),
            "QS1 violation: bitset popcount {} != matched_ids.len() {}",
            bitset.popcount(),
            ids.len()
        );
    }

    pub fn update(&mut self, master: &ArchetypeMaster) {
        let pre_gen = self.archetype_state.last_observed_generation();
        let pre_struct = self.archetype_state.last_observed_structural();
        self.archetype_state.update_archetypes(master);
        if pre_gen != master.archetype_generation()
            || pre_struct != master.structural_generation()
        {
            Self::post_filter_matched(
                &mut self.archetype_state,
                &self.data_state,
                &self.filter_state,
                master,
            );
        }
    }

    pub fn init_access(&self, access_set: &mut FilteredAccessSet) {
        <D as QueryData>::init_access(&self.data_state, access_set);
        <F as QueryFilter>::init_access(&self.filter_state, access_set);
    }
}
```

### 6.2 New helpers on `QueryState`

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query_state.rs (edit)

impl QueryState {
    #[inline]
    pub(crate) fn matched_ids_mut(&mut self) -> &mut Vec<ArchetypeId> {
        &mut self.matched_ids
    }

    /// Removes the id at `index` via swap_remove, also clearing its bit
    /// in the dedup bitset (preserves QS1 invariant per M1).
    pub(crate) fn remove_matched_at(&mut self, index: usize) {
        let removed = self.matched_ids.swap_remove(index);
        self.matched_archetypes.remove(removed.0);
    }

    /// Read-only accessor for the dedup bitset; used by
    /// `QueryDataState::assert_dual_invariant`.
    #[inline]
    pub(crate) fn matched_archetypes_bitset(&self) -> &ArchetypeBitSet {
        &self.matched_archetypes
    }

    #[inline]
    pub(crate) fn last_observed_generation(&self) -> ArchetypeGeneration {
        self.generation
    }

    #[inline]
    pub(crate) fn last_observed_structural(&self) -> ArchetypeGeneration {
        self.structural_generation
    }
}
```

### 6.3 New helper on `ArchetypeBitSet`

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs (edit)

impl ArchetypeBitSet {
    /// Clears the bit for `archetype_id`. Required by
    /// `QueryState::remove_matched_at` to preserve QS1 (M1).
    #[inline]
    pub fn remove(&mut self, archetype_id: usize) {
        if archetype_id >= MAX_ARCHETYPES {
            archetype_id_out_of_range(archetype_id);
        }
        let w = archetype_id >> 6;
        let b = archetype_id & 63;
        self.bits[w] &= !(1u64 << b);
    }

    /// Returns the number of set bits. Used by
    /// `QueryDataState::assert_dual_invariant` (M1).
    #[inline]
    pub fn popcount(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }
}
```

### 6.4 Why post-filter instead of rewriting `QueryState`

The existing `QueryState::matches` predicate is a closed-form bitmask check. Adding `Or<F>` or non-mask predicates would require either:

- **Replace `matches` with a closure**: forbidden — requires `Box<dyn Fn>` or generics. The generic version proliferates `QueryState<F>` everywhere.
- **Two-pass: archetype-mask filter (warm), then post-filter (cold)**: chosen. Preserves the warm path verbatim — for simple `Without<C>` / `With<C>` queries, the post-filter pass is a no-op (every id already passes both checks). For `Or<F>` queries, the post-filter cost is paid once per `update_archetypes` call (i.e. on archetype generation change), not per `iter()`.

**Or<F> complexity (C4)**: When `D::aggregate_include` and `F::aggregate_include` both contribute nothing (e.g. `Query<(), Or<(With<A>, With<B>)>>`), the `QueryState`'s include mask is empty, so `update_archetypes` matches EVERY live archetype. `post_filter_matched` then scans all matched archetypes and applies `F::matches_component_set` (the OR predicate). Worst-case cost: O(archetype_count × Or-arity). For boyko's 1024-archetype hard ceiling and arity-12 Or, that's ~12,288 mask-checks × ~5 ns = ~60 µs per generation bump — cold path only, acceptable.

Trade-off: the post-filter pass is ~5 ns per matched id × matched_ids.len(). For realistic apps with < 100 matched archetypes, < 500 ns on the cold path — negligible compared to `run_closure_once`'s 1 µs.

### 6.5 Alternatives considered and rejected

- **Build a fully-typed `QueryState<D, F>` from scratch**: rejected — duplicates Phase 5c's mature logic.
- **Inline `D::State` / `F::State` into `QueryState` directly**: rejected — leaks Phase 8b types into Phase 5c module.
- **Pre-compute `Or` membership via per-Or-element bitset intersection**: rejected — over-engineered for Phase 8b. Post-filter is O(matched_ids) cold-path.

### 6.6 Trade-off

The post-filter pass adds a small cold-path cost for `Or<F>` queries. For non-Or queries it's a verifiable no-op (debug-asserts every id passes both checks; see assertion in §19.4). Phase 10's `Changed<C>` will need this same mechanism for the per-row tick comparison; the architecture extends naturally.

---

## 7. Decision D5 — Iteration mechanism (the hot loop)

### 7.1 Decision (M2)

`QueryIter<'q, 's, D, F>` and `QueryIterMut<'q, 's, D, F>` hold:
- A cursor over `QueryState::matched_ids` (slice iter).
- Per-archetype Fetch caches for `D` and `F`.
- The current archetype's row range.
- The world cell (Copy, by-value).

**M2 fix**: `QueryIter::next` calls only `archetype_ptr` (read-only mint) and `set_table_readonly` (read-only signature). `QueryIterMut::next` calls only `archetype_ptr_mut` and `set_table_mut`. No `*const → *mut` cast anywhere.

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/iter.rs (new)

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::iters::query::data::{QueryData, ReadOnlyQueryData};
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::ArchetypeId;

pub struct QueryIter<'q, 's, D: QueryData, F: QueryFilter> {
    archetype_ids: std::slice::Iter<'q, ArchetypeId>,
    data_state: &'s D::State,
    filter_state: &'s F::State,
    world: UnsafeEcsCell<'q>,
    data_fetch: D::Fetch<'q>,
    filter_fetch: F::Fetch<'q>,
    current_row: usize,
    current_len: usize,
    _marker: PhantomData<&'s ()>,
}

impl<'q, 's, D: QueryData, F: QueryFilter> QueryIter<'q, 's, D, F> {
    /// # Safety (Q1, QD4)
    ///
    /// Caller MUST ensure `D: ReadOnlyQueryData` (gated by Query::iter
    /// bound). Read-only path: `archetype_ptr` (not `_mut`) and
    /// `set_table_readonly` (not `_mut`) — no write-capable provenance is
    /// minted anywhere on this code path.
    pub(crate) unsafe fn new(
        state: &'s QueryDataState<D, F>,
        world: UnsafeEcsCell<'q>,
    ) -> Self {
        Self {
            archetype_ids: state.archetype_state.matched_ids().iter(),
            data_state: &state.data_state,
            filter_state: &state.filter_state,
            world,
            data_fetch: <D as QueryData>::init_fetch(&state.data_state),
            filter_fetch: <F as QueryFilter>::init_fetch(&state.filter_state),
            current_row: 0,
            current_len: 0,
            _marker: PhantomData,
        }
    }
}

impl<'q, 's, D: QueryData, F: QueryFilter> Iterator for QueryIter<'q, 's, D, F>
where
    D: ReadOnlyQueryData,
{
    type Item = D::Item<'q>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.current_row < self.current_len {
                let row = self.current_row;
                self.current_row += 1;

                if !const { F::IS_ARCHETYPAL } {
                    let pass = unsafe {
                        <F as QueryFilter>::filter_fetch(&self.filter_fetch, row)
                    };
                    if !pass { continue; }
                }

                // SAFETY (QD2): set_table_readonly was called for this
                //   archetype boundary; row in range; D: ReadOnlyQueryData.
                return Some(unsafe {
                    <D as QueryData>::fetch(&self.data_fetch, row)
                });
            }

            let arch_id = *self.archetype_ids.next()?;

            // M2: read-only mint — `archetype_ptr`, not `_mut`. No cast.
            //
            // SAFETY (U_C2, Q5): cell scoped to 'q; archetype_ptr returns
            //   None for stale ids — skipped.
            let Some(archetype_ptr) = (unsafe { self.world.archetype_ptr(arch_id) })
            else { continue; };

            // SAFETY (QD3, QD4): set_table_readonly takes *const Archetype
            //   directly. No *const → *mut cast.
            unsafe {
                <D as QueryData>::set_table_readonly(
                    &mut self.data_fetch,
                    self.data_state,
                    archetype_ptr,
                );
                <F as QueryFilter>::set_table_readonly(
                    &mut self.filter_fetch,
                    self.filter_state,
                    archetype_ptr,
                );
            }

            // SAFETY (U1, U2): archetype_ptr is slab-stable for 'q.
            let arch_ref: &Archetype = unsafe { &*archetype_ptr };
            self.current_row = 0;
            self.current_len = arch_ref.entity_count();
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) { (0, None) }
}

pub struct QueryIterMut<'q, 's, D: QueryData, F: QueryFilter> {
    archetype_ids: std::slice::Iter<'q, ArchetypeId>,
    data_state: &'s D::State,
    filter_state: &'s F::State,
    world: UnsafeEcsCell<'q>,
    data_fetch: D::Fetch<'q>,
    filter_fetch: F::Fetch<'q>,
    current_row: usize,
    current_len: usize,
    _marker: PhantomData<&'s ()>,
}

impl<'q, 's, D: QueryData, F: QueryFilter> QueryIterMut<'q, 's, D, F> {
    pub(crate) unsafe fn new(
        state: &'s QueryDataState<D, F>,
        world: UnsafeEcsCell<'q>,
    ) -> Self {
        Self {
            archetype_ids: state.archetype_state.matched_ids().iter(),
            data_state: &state.data_state,
            filter_state: &state.filter_state,
            world,
            data_fetch: <D as QueryData>::init_fetch(&state.data_state),
            filter_fetch: <F as QueryFilter>::init_fetch(&state.filter_state),
            current_row: 0,
            current_len: 0,
            _marker: PhantomData,
        }
    }
}

impl<'q, 's, D: QueryData, F: QueryFilter> Iterator for QueryIterMut<'q, 's, D, F> {
    type Item = D::Item<'q>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.current_row < self.current_len {
                let row = self.current_row;
                self.current_row += 1;

                if !const { F::IS_ARCHETYPAL } {
                    let pass = unsafe {
                        <F as QueryFilter>::filter_fetch(&self.filter_fetch, row)
                    };
                    if !pass { continue; }
                }

                return Some(unsafe {
                    <D as QueryData>::fetch(&self.data_fetch, row)
                });
            }

            let arch_id = *self.archetype_ids.next()?;

            // M2: write-capable mint — `archetype_ptr_mut`. The cell's
            //   debug-assert (Phase 8a) fires here if `allows_mutable_access
            //   == false`, covering Q1.
            //
            // SAFETY (U_C3, Q1): cell write-capable; mint produces
            //   write-capable *mut Archetype.
            let Some(archetype_ptr) = (unsafe { self.world.archetype_ptr_mut(arch_id) })
            else { continue; };

            // SAFETY (QD3, QD4): set_table_mut takes *mut Archetype directly.
            unsafe {
                <D as QueryData>::set_table_mut(
                    &mut self.data_fetch,
                    self.data_state,
                    archetype_ptr,
                );
                <F as QueryFilter>::set_table_mut(
                    &mut self.filter_fetch,
                    self.filter_state,
                    archetype_ptr,
                );
            }

            // Read-only probe to extract entity_count.
            // SAFETY (U1, U2): no &mut materialised; raw deref.
            let arch_ref: &Archetype = unsafe { &*archetype_ptr };
            self.current_row = 0;
            self.current_len = arch_ref.entity_count();
        }
    }
}
```

### 7.2 Hot loop walkthrough

For `Query<(&mut Position, &Velocity), Without<Frozen>>::iter_mut().next()`:

1. **Row check** (`current_row < current_len`): 1 register compare.
2. **Filter check** (`Without<Frozen>::IS_ARCHETYPAL = true`): const-folded, dead branch.
3. **Data fetch** (`(WriteFetch<Position>, ReadFetch<Velocity>)::fetch(row)`): 2 ALU ops + 2 L1d hits.
4. **Cursor increment**: 1 register add.

**Per-row cost**: ~3 cycles + 2 L1d hits (~3-4 ns on Zen3).

Archetype boundary: `archetype_ids.next()` + `world.archetype_ptr_mut(id)` (2 dependent loads) + `D::set_table_mut(...)` (per element: 1 column load + 1 stack write) + `F::set_table_mut(...)` (no-op for archetypal) + `entity_count()` load. **Per-archetype cost**: ~30-50 ns.

### 7.3 Alternatives considered and rejected

- **Pre-fetch the next archetype's columns ahead of the inner loop**: rejected — premature.
- **Coalesce row-walk and filter-check into a single loop**: rejected — current shape already does this via const-fold.
- **Single generic iterator type with `IS_MUTABLE` const**: rejected — different soundness contracts. Split types make SAFETY comments precise.
- **Cache `entity_count` in Fetch (set at `set_table_*`)**: rejected — would bloat Fetch. Current shape keeps Fetch lean.

### 7.4 Trade-off

The two-loop structure (outer archetype + inner row) is the cleanest expression of archetype-major iteration. LLVM consistently optimises `loop { while { ... } }` into a single basic block at -O2.

---

## 8. Decision D6 — Migration of legacy `Query<'a>`, `iter_one`, `iter_two`

### 8.1 Decision

In-place migrate `QueryIterOne::load_archetype` and `QueryIterTwo::load_archetype` to use `arch.columns[c.0]`. The file is renamed `query.rs` → `legacy_query.rs` (covered by M5 in §17.3). Public API unchanged.

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/legacy_query.rs (edit)

impl<'q, A: Component> QueryIterOne<'q, A> {
    fn load_archetype(&mut self, arch: &Archetype) {
        let comp_id = A::component_id();
        debug_assert!(comp_id.0 < MAX_COMPONENTS);
        // SAFETY (U4): bounded.
        let column = unsafe { arch.columns.get_unchecked(comp_id.0) };

        if column.ptr.is_null() {
            self.current_remaining = 0;
            self.current_ptr = std::ptr::null();
            return;
        }
        let entity_count = arch.entity_count();
        if entity_count == 0 {
            self.current_remaining = 0;
            self.current_ptr = std::ptr::null();
            return;
        }
        debug_assert_eq!(column.stride as usize, std::mem::size_of::<A>());
        // SAFETY (Phase 7 D4): column.ptr set by refresh_column; layout
        //   guaranteed by component registry; buffer ≥ entity_count slots.
        self.current_ptr = column.ptr as *const A;
        self.current_remaining = entity_count;
    }
}

impl<'q, A: Component, B: Component> QueryIterTwo<'q, A, B> {
    fn load_archetype(&mut self, arch: &Archetype) {
        let id_a = A::component_id();
        let id_b = B::component_id();
        debug_assert!(id_a.0 < MAX_COMPONENTS);
        debug_assert!(id_b.0 < MAX_COMPONENTS);
        // SAFETY (U4): bounded.
        let col_a = unsafe { arch.columns.get_unchecked(id_a.0) };
        let col_b = unsafe { arch.columns.get_unchecked(id_b.0) };
        if col_a.ptr.is_null() || col_b.ptr.is_null() {
            self.current_remaining = 0;
            self.ptr_a = std::ptr::null();
            self.ptr_b = std::ptr::null();
            return;
        }
        let entity_count = arch.entity_count();
        if entity_count == 0 {
            self.current_remaining = 0;
            self.ptr_a = std::ptr::null();
            self.ptr_b = std::ptr::null();
            return;
        }
        debug_assert_eq!(col_a.stride as usize, std::mem::size_of::<A>());
        debug_assert_eq!(col_b.stride as usize, std::mem::size_of::<B>());
        self.ptr_a = col_a.ptr as *const A;
        self.ptr_b = col_b.ptr as *const B;
        self.current_remaining = entity_count;
    }
}
```

### 8.2 Why in-place migration

Per project-analyst's known issue #6: legacy path was a SparseMap traversal. Column-table is O(1) array index, 1 cache line. Migration is mandatory: (1) legacy callers in use; (2) maintain single fast read path; (3) avoid two-code-path maintenance burden.

### 8.3 What stays untouched

- `LegacyQuery::from_archetypes`, `with_*`, `iter()` (archetype-iter), constructors.
- `QueryState`'s own update logic.
- Public method signatures.

### 8.4 Test impact

All existing tests in the legacy file (600+ lines) keep passing under the new module name. Test names referencing `iters::query::Query` are updated to `iters::legacy_query::Query` (see §17.3 callsite list).

---

## 9. Decision D7 — Mutable iteration provenance flow

### 9.1 Decision

The cell-by-value pattern + the split `set_table_readonly`/`set_table_mut` (M2) makes this fully clean:

1. `EcsMaster::run_system_once<S>` takes `&mut self`.
2. Mints `cell = UnsafeEcsCell::new_mutable(&mut self)`.
3. Calls `system.run_unsafe(cell)` — by-value.
4. `<Query<D, F> as SystemParam>::get_param(state, meta, cell)` — by-value.
5. Returns `Query { state, world: cell, meta, _marker }`.
6. `q.iter_mut()` returns `QueryIterMut { ..., world: cell, ... }`.
7. `QueryIterMut::next()` calls `self.world.archetype_ptr_mut(arch_id)` — by-value receiver on the cell.
8. `<D as QueryData>::set_table_mut(&mut self.data_fetch, ..., archetype_ptr_mut)` writes column's `*mut T` into Fetch. **No cast.**
9. `<D as QueryData>::fetch(&self.data_fetch, row)` derefs cached `*mut T` as `&mut T`.

At NO point in this chain does any `&self` borrow appear on `UnsafeEcsCell`. Every method call on the cell is by-value. The C1 retag bug is structurally impossible.

For read-only: same chain but with `archetype_ptr` (not `_mut`) and `set_table_readonly`. No `*const → *mut` cast at any point — both halves of the trait have native-kind signatures.

### 9.2 Concrete cell flow diagram (mutable)

```
EcsMaster::run_system_once(&mut self, ...)
  │ cell = unsafe { UnsafeEcsCell::new_mutable(self) }  // Unique tag
  ▼
system.run_unsafe(cell)
  │ <Query<D, F> as SystemParam>::get_param(state, meta, cell)
  ▼
Query { state, world: cell, meta, _marker }
  │ q.iter_mut()
  ▼
QueryIterMut::new(state, self.world)  // Copy of cell
  │ self.world.archetype_ptr_mut(arch_id)  // by-value, debug_assert(allows_mutable_access)
  ▼
*mut Archetype with write provenance
  │ <D as QueryData>::set_table_mut(&mut fetch, state, archetype_ptr_mut)
  │   // M2: takes *mut directly — NO CAST
  ▼
fetch.base = column.ptr as *mut T  // Unique tag preserved
  │ <D as QueryData>::fetch(&fetch, row)
  ▼
&mut *fetch.base.add(row)  // fresh &mut T reborrow, write-capable
```

### 9.3 Why this is sound under Tree Borrows (M2)

The Tree Borrows model gates write-capable provenance on the *minting* operation. The split `set_table_readonly`/`set_table_mut` ensures:

- **Read-only path**: `cell.archetype_ptr(id) → *const Archetype` (SharedReadOnly tag) → `set_table_readonly(_: *const Archetype)` (signature matches; no cast) → `(*archetype).columns.get_unchecked(c)` (raw arithmetic, no tag change) → `column.ptr as *const T` → read-only deref. No write through any pointer. **Sound by Tree Borrows definition; no empirical Miri uncertainty.**
- **Mutable path**: `cell.archetype_ptr_mut(id) → *mut Archetype` (Unique tag, fresh from `&mut EcsMaster` chain) → `set_table_mut(_: *mut Archetype)` (signature matches) → `column.ptr as *mut T` (Unique preserved through cast) → `&mut *base.add(row)` (fresh &mut reborrow). Write-capable provenance flows through unchanged.

The Round-1 `*const → *mut Archetype` cast (which Round-1 §7.1 noted needed Tree Borrows verification) is **eliminated**. Step 7's Miri test verifies the soundness empirically.

### 9.4 Edge case: nested queries within a system body

```rust
fn nested_system(mut q1: Query<&mut A>, q2: Query<&B>) { /* ... */ }
```

`q1` and `q2` are constructed by `<(Query<&mut A>, Query<&B>) as SystemParam>::get_param`. The tuple impl forwards to each element's `get_param` with the SAME `world: UnsafeEcsCell<'w>` (passed by value to each). Both Queries hold a Copy. They cannot alias on components because `FilteredAccessSet` rejected at init.

### 9.5 Alternatives considered and rejected

- **Bake `*mut Archetype` per matched archetype into Query at `get_param`**: rejected — N reborrows + storage cost wasted on no-iter calls.
- **Hold `&mut [Archetype]` slice in Query**: rejected — requires contiguous slice; allocation.
- **Use Cell<Fetch> for `&self` interior mutability**: rejected — violates Phase 8a no-Cell hot-path rule.

### 9.6 Trade-off

Per-archetype `set_table_*` cost (~30-50 ns) is paid once per archetype boundary. Amortised over rows, < 1 ns/row for archetypes with > 100 entities.

---

## 10. Decision D8 — Variadic strategy

### 10.1 QueryData variadic (M4)

Single `macro_rules!` site with paired-ident invocations emits tuple impls for arity 1..=12. Stub impls for arity 13..=24 use `const { panic!(...) }`.

See §4.6 for the macro shape. Worked-example arity-3 expansion in §25.

```rust
impl_query_data_tuple!((D0, s0, f0));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1));
impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2));
// ... up to arity 12 ...
impl_query_data_tuple!(
    (D0, s0, f0), (D1, s1, f1), (D2, s2, f2), (D3, s3, f3),
    (D4, s4, f4), (D5, s5, f5), (D6, s6, f6), (D7, s7, f7),
    (D8, s8, f8), (D9, s9, f9), (D10, s10, f10), (D11, s11, f11)
);
```

### 10.2 QueryFilter variadic

Two flavours, both via paired-ident macros:

- **Tuple as AND**: `impl_query_filter_tuple_and!((F0, s0, f0), ...)` for arity 1..=12.
- **`Or<(...)>`**: `impl_or_filter_tuple!((F0, s0, f0), ...)` for arity 1..=12.

Stubs for 13..=24 follow the same `const { panic!(...) }` pattern.

### 10.3 ReadOnlyQueryData variadic

Auto-emitted alongside `QueryData` tuple impl — the `unsafe impl<$($D: ReadOnlyQueryData),*> ReadOnlyQueryData for ($($D,)*) {}` blanket.

### 10.4 Arity cap rationale

12 covers 99th percentile. Real systems with > 8 query data elements are rare. The const-panic stubs for 13..=24 give a focused diagnostic.

### 10.5 Integration with `QueryDataState`

The variadic strategy is INVISIBLE to `QueryDataState`. The tuple expansion happens inside `D::init_state`, `D::init_access`, etc.

### 10.6 Alternatives considered and rejected

- **`tuple_list` crate**: rejected — external dep.
- **Proc-macro for tuple impls**: rejected — `macro_rules!` is sufficient and faster to compile.

---

## 11. Decision D9 — Scope boundary with Phase 8c, 8d, 10

### 11.1 Phase 8b ships

| Feature | Module |
|---------|--------|
| `Query<'w, 's, D, F>` type + `SystemParam` impl + `IntoIterator` impls (C1) | `iters/query/query.rs` |
| `QueryData` trait + `&T`, `&mut T`, tuple impls, split set_table (M2) | `iters/query/data.rs` |
| `ReadOnlyQueryData` marker trait + tuple impls | `iters/query/data.rs` |
| `QueryFilter` trait + `With<C>`, `Without<C>`, `Or<F>`, tuple-as-AND impl, split set_table (M2) | `iters/query/filter.rs` |
| `QueryDataState<D, F>` per-system state composer with dual-invariant check (M1) | `iters/query/state.rs` |
| `QueryIter<'q, 's, D, F>` / `QueryIterMut<'q, 's, D, F>` (M2 split set_table) | `iters/query/iter.rs` |
| `QueryState` helpers (`matched_ids_mut`, `remove_matched_at`, `matched_archetypes_bitset`, `last_observed_*`) | `iters/query_state.rs` (edit) |
| `ArchetypeBitSet::remove`, `ArchetypeBitSet::popcount` helpers (M1) | `iters/archetype_bit_set.rs` (edit) |
| Migration of `QueryIterOne` / `QueryIterTwo` to column-table | `iters/legacy_query.rs` (edit, post-rename) |

### 11.2 Phase 8b does NOT ship

| Feature | Phase | Reason |
|---------|-------|--------|
| `IntoSystem`/`FunctionSystem` (closure inference without turbofish) | 8c | Per Phase 8a M5+W3. `run_closure_once` still requires turbofish until 8c. |
| `Commands` (deferred mutations) | 8d | Different SystemParam shape. |
| `Changed<C>` / `Added<C>` | 10 | Requires tick infrastructure. Trait shape ready for retrofit (IS_ARCHETYPAL = false). |
| `Optional<C>` query data | 10 | Same retrofit path as Changed. |
| `Entity` query data (yields `EntityId`) | 8c | Trivial impl; deferred to 8c bundle. |
| `&World` SystemParam | 8c | Different shape. |
| `Local<T>` SystemParam | Deferred | No boyko user yet. |
| `Parallel iteration` | 9 | Requires scheduler. |

### 11.3 Phase-10-readiness pact

`QueryFilter` shape is Phase-10-ready:
- `const IS_ARCHETYPAL: bool` — Phase 10 `Changed<C>::IS_ARCHETYPAL = false` will hit existing per-row codepath.
- `init_access` declares reads.
- `Fetch<'w>` GAT — Phase 10 holds tick-column pointer.
- `filter_fetch(row)` — Phase 10 reads `ticks[row]` and compares to `meta.last_run`.
- Split `set_table_readonly` / `set_table_mut` (M2) — Phase 10 implements both.

No retrofit required. `Changed<C>` impl lands in `iters/query/changed.rs` without touching Phase 8b code.

---

## 12. SAFETY invariants summary

| Tag | Statement | Where enforced | Where consumed |
|-----|-----------|----------------|----------------|
| **QD1** | D::init_state covers every component fetch will touch. | Manual review; `#[derive(QueryData)]` in 8c. | init_access. |
| **QD2** | Fetch starts null; set_table_* overwrites before fetch. | `debug_assert!(!base.is_null())` in `fetch`. | Cursor archetype-boundary call. |
| **QD3** | Fetch<'w> lifetime ≤ Query<'w>. | PhantomData<&'w T> on Fetch. | Cursor uses Fetch within 'w. |
| **QD4** (M2) | Read-only cursor → set_table_readonly only; mutable cursor → set_table_mut only. | Trait signatures; per-method panic backstop for &mut T's set_table_readonly. | Cursor next() per archetype boundary. |
| **QF1** | If IS_ARCHETYPAL, filter_fetch returns true. | Manual review + debug_assert. | Const-fold in cursor. |
| **QF2** | init_access declares filter's reads. | Manual review. | FilteredAccessSet. |
| **Q1** | Mutable queries on write-capable cell only. | Type-level (D: ReadOnlyQueryData bound on iter()) + cell-level debug_assert in archetype_ptr_mut. | iter_mut cursor mints *mut Archetype. |
| **Q2** | Tuple SystemParam Queries don't alias. | FilteredAccessSet at init_access. | Hot loop assumes no alias. |
| **Q3** | One cursor per Query at a time. | iter(&self)/iter_mut(&mut self) borrows. | Cursor mutates Fetch freely. |
| **Q4** | Empty archetypes skipped. | current_len == 0 ⇒ loop advances. | Cursor invariant. |
| **Q5** | Stale ids skipped (archetype_ptr* returns None). | `let Some(...) else { continue; }`. | Defence-in-depth for re-entrancy edge cases. |
| **QS1** (M1) | matched_ids ⟺ matched_archetypes bitset. | remove_matched_at + assert_dual_invariant (debug). | Iter cursor uses matched_ids slice. |
| **SP1/SP2/SP4** (Phase 8a) | Inherited via SystemParam impl. | Existing accumulator + cell. | Caller. |
| **U_C1/U_C2/U_C3** (Phase 8a) | UnsafeEcsCell by-value receivers. | Cell trait impl. | Cursor cell access. |

---

## 13. Data structures summary + size verification

### 13.1 Layout table

| Type | Size (bytes) | Align | Notes |
|------|--------------|-------|-------|
| `ReadState<T>` | 16 | 8 | Copy. |
| `WriteState<T>` | 16 | 8 | Copy. |
| `ReadFetch<'w, T>` | 8 | 8 | Copy. |
| `WriteFetch<'w, T>` | 8 | 8 | Copy. |
| `WithState<C>` | 16 | 8 | Copy. |
| `WithoutState<C>` | 16 | 8 | Copy. |
| `QueryDataState<D, F>` | see §13.3 (M6) | 64 (inherits from QueryState's align) | Holds Vec, not Copy. |
| `Query<'w, 's, D, F>` | 24 (3 × 8) + 0 PhantomData = **24** | 8 | Not Copy. |
| `QueryIter<'q, 's, D, F>` | 16 (slice::Iter) + 8 + 8 + 16 (cell) + size(D::Fetch) + size(F::Fetch) + 16 (rows) + 0 = **~64 + Fetch** | 8 | Stack-resident. |
| `QueryIterMut` | same as QueryIter | 8 | |

### 13.2 Hot-path size budget

For `Query<(&mut Position, &Velocity), Without<Frozen>>::iter_mut()`:

- `Query`: 24 B.
- `QueryIterMut` stack frame: 16 + 8 + 8 + 16 + 16 (data_fetch: WriteFetch+ReadFetch = 8+8) + 0 (filter_fetch: ()) + 16 = **~80 B = 1.25 cache lines.**

Fits in 2 cache lines, hot fields in first line.

### 13.3 SystemMeta + state heap residency (M6)

`QueryDataState<D, F>` exact size formula:

```
size = sizeof(QueryState)  // ~280 B (Vec<ArchetypeId> + ArchetypeBitSet + generations)
     + sum over D-tuple elements of sizeof(<Di as QueryData>::State)
     + sum over F-tuple elements of sizeof(<Fi as QueryFilter>::State)
     + sizeof(PhantomData<fn() -> (D, F)>)  // 0
```

Concrete sizes:
- `<&T as QueryData>::State = ReadState<T>` = 16 B
- `<&mut T as QueryData>::State = WriteState<T>` = 16 B
- `<With<C> as QueryFilter>::State = WithState<C>` = 16 B
- `<Without<C> as QueryFilter>::State = WithoutState<C>` = 16 B
- `<() as QueryFilter>::State = ()` = 0 B
- `<Or<(F0, .., Fn)> as QueryFilter>::State = (F0::State, .., Fn::State)` = sum
- Tuple element states sum the same way (with Rust's tuple padding).

Examples:

| Query | Size formula | Total |
|-------|--------------|-------|
| `Query<&A>` | 280 + 16 + 0 | **~296 B** |
| `Query<(&A, &B)>` | 280 + 16+16 + 0 | **~312 B** |
| `Query<(&mut A, &B), Without<C>>` | 280 + 16+16 + 16 | **~328 B** |
| `Query<(&A, &B, &C, &D), (With<E>, Without<F>)>` | 280 + 4×16 + 2×16 | **~376 B** |
| Arity-12 D, arity-0 F | 280 + 12×16 + 0 | **~472 B** |
| Arity-12 D, arity-12 F (Or) | 280 + 12×16 + 12×16 | **~664 B** |

Round-1's "~330 B for typical D/F" is corrected: typical small queries are ~300 B; worst-case arity-12-each is ~664 B. Acceptable: state lives in `Option<P::State>` on the heap (per `FnOnceSystem`), touched once per system call.

### 13.4 Size assertions

```rust
const _: () = assert!(std::mem::size_of::<ReadFetch<'static, u32>>() == 8);
const _: () = assert!(std::mem::size_of::<WriteFetch<'static, u32>>() == 8);
const _: () = assert!(std::mem::size_of::<ReadState<u32>>() <= 16);
const _: () = assert!(std::mem::size_of::<WriteState<u32>>() <= 16);
// Query handle:
const _: () = assert!(std::mem::size_of::<Query<'static, 'static, &'static u32, ()>>() == 24);
```

---

## 14. Public API surface delta

### 14.1 New public exports (C1)

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/mod.rs (new)

pub mod data;
pub mod filter;
pub mod state;
pub mod iter;
pub mod query;

pub use data::{QueryData, ReadOnlyQueryData, MAX_QUERY_DATA_ARITY};
pub use filter::{QueryFilter, With, Without, Or};
pub use iter::{QueryIter, QueryIterMut};
pub use query::Query;
// QueryDataState is pub(crate) — internal to the SystemParam impl.
```

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/mod.rs (edit)

pub mod archetype_bit_set;
pub mod component_set;
pub mod query_state;

// Legacy archetype-iter Query (renamed file to disambiguate per M5).
pub mod legacy_query;
// New typed Query DSL.
pub mod query;

pub const MAX_ARCHETYPES: usize = 1024;

pub use query_state::{QueryState, QueryStateIter};
pub use legacy_query::Query as LegacyQuery;
pub use query::Query;
```

The new `Query<'w, 's, D, F>` is exported as `iters::Query` (the typed DSL). The legacy `Query<'a>` is exported as `iters::LegacyQuery`.

**C1**: `IntoIterator for &Query<...>` and `IntoIterator for &mut Query<...>` impls are defined in `query.rs` (see §3.2). They desugar `for x in &q` and `for x in &mut q`.

### 14.2 New private/crate exports

- `QueryDataState<D, F>` — pub(crate).
- `QueryState::matched_ids_mut`, `remove_matched_at`, `matched_archetypes_bitset`, `last_observed_*` — pub(crate).
- `ArchetypeBitSet::remove`, `popcount` — pub helpers.

### 14.3 SystemParam impl for Query (C3, M7)

```rust
// File: crates/boyko_ecs/src/ecs/core/iters/query/query.rs (new, continued)

// SAFETY (SP1, SP2, SP4):
//   - SP1: init_access forwards to D::init_access + F::init_access.
//   - SP2: get_param returns Query bound to cell's 'w.
//   - SP4: init_state calls QueryDataState::new which calls
//     archetype_state.update_archetypes(master) — pure read, no archetype
//     registration. SP4 holds.
//
// C3 fix: single binder declaring both lifetimes (Round 1 used `'_` in
// impl head position, which is malformed).
unsafe impl<'a, 'b, D: QueryData + 'static, F: QueryFilter + 'static>
    SystemParam for Query<'a, 'b, D, F>
{
    type State = QueryDataState<D, F>;
    type Item<'w, 's> = Query<'w, 's, D, F>;

    fn init_state(world: &mut EcsMaster, _meta: &mut SystemMeta) -> Self::State {
        QueryDataState::<D, F>::new(world)
    }

    fn init_access(
        state: &Self::State,
        _meta: &mut SystemMeta,
        access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        state.init_access(access_set);
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // M7 SAFETY note: the `master` binding below is a SHARED borrow
        //   `&'tmp ArchetypeMaster` whose lifetime `'tmp` is contained
        //   strictly within the `state.update(master)` statement. The borrow
        //   is dropped at the semicolon before the `Query { ... }` literal
        //   is constructed. No aliasing with the by-value `world` cell
        //   passed below: the cell is a Copy<'w> chain that does NOT
        //   retain any reborrow of `master`. The Query holds `world` (by
        //   value), `meta` (a `&SystemMeta` that came from the function
        //   argument), and `state` (a `&'s mut QueryDataState`); the
        //   `master` reborrow is freed first. No cross-borrow conflict.
        //
        // SAFETY (U_C2): world.world() returns &'w EcsMaster — shared read
        //   access. archetype_master() returns &'w ArchetypeMaster.
        //   state.update(master) consumes the borrow before Query is built.
        let master = unsafe { world.world().archetype_master() };
        state.update(master);

        Query {
            state,
            world,
            meta,
            _marker: PhantomData,
        }
    }

    #[inline]
    fn new_archetype(
        _state: &mut Self::State,
        _meta: &mut SystemMeta,
        _archetype: &Archetype,
    ) {
        // Phase 8b: defer to next iter()'s state.update(master). The hook
        // exists for Phase 9's scheduler.
    }
}
```

### 14.4 Documentation contract

Each public item has a `///` doc comment per codebase convention. `IntoIterator` impls (C1) get doc comments explaining the `for x in &q` / `for x in &mut q` sugar.

---

## 15. Algorithms for critical paths

### 15.1 `QueryIter::next()` hot loop (inner row)

**Steps**: row check → (const-folded out for archetypal F) → D::fetch → cursor increment → return.
**Complexity**: O(1) per row.
**Cache**: 1-2 L1d hits per row depending on D-arity.
**Branching**: 1 row-range branch (well-predicted).
**SIMD**: high potential.

### 15.2 `QueryIter::next()` archetype boundary

**Steps**: archetype_ids.next() → world.archetype_ptr(arch_id) → D::set_table_readonly → F::set_table_readonly → entity_count read.
**Complexity**: O(#elements in D) per boundary.
**Cache**: 3-4 cache lines (id_to_slot, slot, columns prefix, entity_count).
**Cost**: 30-50 ns per boundary.

### 15.3 `QueryDataState::new()` cold path

**Steps**: D::init_state + F::init_state + mask aggregation + QueryState::new + update_archetypes + post_filter_matched + assert_dual_invariant.
**Complexity**: O(arity × current_archetype_count).
**Cost**: < 1 µs for typical 50-archetype world.

### 15.4 `QueryDataState::update()` warm path

**Steps**: snapshot generations → update_archetypes (early-return if no change) → conditional post_filter_matched.
**Warm-path**: 4 ns (early-return).
**Cold-path**: O(archetype_count × arity).

### 15.5 `QueryState::update_archetypes` interactions (C4)

`post_filter_matched` runs AFTER `update_archetypes`. The pipeline:

1. `update_archetypes` populates `matched_ids` with archetypes whose mask satisfies `(mask & include == include) && (mask & exclude == 0)`. For `Or<F>` queries that contribute nothing to include/exclude, this populates `matched_ids` with EVERY live archetype (subject to exclude pruning).
2. `post_filter_matched` walks `matched_ids` and applies `D::matches_component_set` AND `F::matches_component_set`, removing failing ids via `remove_matched_at` (which preserves the QS1 invariant by clearing the bit).

**Key property**: `D::matches_component_set` for `&T` / `&mut T` is `mask.contains(state.id)` — identical to what `QueryState::matches` already checked. For non-Or queries, the post-filter is a verifiable no-op (debug-asserted at the end of `post_filter_matched` via no-drop counter).

**Or<F> worst case (C4)**: `Query<(), Or<(With<A0>, ..., With<A11>)>>`. The include mask is empty (D contributes nothing; Or contributes nothing via M8). `update_archetypes` matches every live archetype. `post_filter_matched` evaluates the OR-of-12-Withs per archetype: 12 mask-checks × `archetype_count`. For 1024 archetypes: 12,288 checks × ~5 ns = ~60 µs once per generation bump. Acceptable per the rationale in §6.4.

---

## 16. Multithreading model

### 16.1 Phase 8b: single-threaded

`EcsMaster: !Send + !Sync`. `Query<'w, 's, D, F>` inherits via `UnsafeEcsCell: !Send + !Sync`. No system runs concurrently with another.

### 16.2 Where Phase 9 takes over

Phase 9 introduces the scheduler. Per-system migration across worker threads requires:
- `QueryDataState: Send + Sync + 'static` — already satisfied.
- `Query<'w, 's, D, F>: Send + Sync` — Phase 9 will introduce scheduler-aliasing-discipline Send/Sync on UnsafeEcsCell.

### 16.3 Aliasing discipline

Within Phase 8b:
- `(Query<&mut A>, Query<&mut A>)` → panic at init_access (`ComponentWriteVsWrite`).
- `(Query<&mut A>, Query<&A>)` → panic (`ComponentReadVsWrite`).
- `(Query<&mut A>, Query<&mut B>)` (A ≠ B) → OK.
- `Query<(&mut A, &A)>` → the tuple's init_access panics on the second `add_component_*` call.

### 16.4 Data partitioning

Phase 8b iterates serially. Phase 9 + 11 will introduce per-archetype and per-chunk parallel iter.

### 16.5 No Mutex/RwLock/atomic on the hot path

Verified across all new files. `component_set.rs`'s tuple_cache uses RwLock but only at init_state (cold).

### 16.6 Send/Sync of new types

| Type | Send | Sync | Justification |
|------|------|------|---------------|
| `ReadState<T>`, `WriteState<T>`, `WithState<C>`, `WithoutState<C>` | Yes | Yes | `usize` + PhantomData. |
| `ReadFetch<'w, T>`, `WriteFetch<'w, T>` | No | No | Raw pointer. |
| `QueryDataState<D, F>` | Yes | Yes | Vec + bound D/F::State. |
| `Query<'w, 's, D, F>` | No | No | Inherits cell. |
| `QueryIter`, `QueryIterMut` | No | No | Same. |

---

## 17. Integration with existing modules

### 17.1 New files

| File | Lines (est.) | Purpose |
|------|--------------|---------|
| `crates/boyko_ecs/src/ecs/core/iters/query/mod.rs` | 30 | Module organisation + re-exports. |
| `crates/boyko_ecs/src/ecs/core/iters/query/data.rs` | 800 | QueryData trait + impls (M2 split adds ~100 lines). |
| `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | 700 | QueryFilter trait + impls (M2 split). |
| `crates/boyko_ecs/src/ecs/core/iters/query/state.rs` | 280 | QueryDataState + dual-invariant check (M1). |
| `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` | 380 | QueryIter, QueryIterMut. |
| `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` | 300 | Query + SystemParam + IntoIterator (C1). |

### 17.2 Edited files

| File | Edit |
|------|------|
| `crates/boyko_ecs/src/ecs/core/iters/mod.rs` | Rename `pub mod query` to `pub mod legacy_query`; add `pub mod query` (new); re-exports per §14.1. |
| `crates/boyko_ecs/src/ecs/core/iters/query.rs` → renamed to `legacy_query.rs` | File rename; QueryIterOne/Two::load_archetype bodies migrated (§8.1). |
| `crates/boyko_ecs/src/ecs/core/iters/query_state.rs` | Add helpers per §6.2. |
| `crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs` | Add `remove`, `popcount` (M1). |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` | Update import + callsites (see §17.3). |

### 17.3 Callsites needing rename (M5 + O5)

Complete enumeration via grep `iters::query::Query|use crate::ecs::core::iters::query` across `crates/boyko_ecs/src` and `crates/boyko_ecs/benches`:

**Production code (4 callsites in 1 file)**:

```text
crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:8
  use crate::ecs::core::iters::query::Query;
  → use crate::ecs::core::iters::legacy_query::Query as LegacyQuery;

crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:547-548
  pub fn query_with_components<'a>(...) -> Query<'a> {
      Query::with_component_ids(self, component_ids)
  → pub fn query_with_components<'a>(...) -> LegacyQuery<'a> {
        LegacyQuery::with_component_ids(self, component_ids)

crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:552-553
  pub fn query_with_mask<'a>(...) -> Query<'a> { Query::with_mask(...) }
  → pub fn query_with_mask<'a>(...) -> LegacyQuery<'a> { LegacyQuery::with_mask(...) }

crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:557-558
  pub fn query_with_exact_mask<'a>(...) -> Query<'a> { Query::with_exact_mask(...) }
  → updated to LegacyQuery

crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:563-564
  pub fn query<'a, T: ComponentSet>(...) -> Query<'a> { Query::with::<T>(...) }
  → updated to LegacyQuery

crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:576-577
  pub fn query_with_filters<'a>(...) -> Query<'a> { Query::with_filters(...) }
  → updated to LegacyQuery

crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:585-586
  pub fn query_with_type_filters<'a, Inc, Exc, Opt>(...) -> Query<'a> { Query::with_type_filters::<Inc, Exc, Opt>(...) }
  → updated to LegacyQuery
```

**Benches (1 file, 6 callsites)**:

```text
crates/boyko_ecs/benches/query_iter.rs:23
  use boyko_ecs::ecs::core::iters::query::Query;
  → use boyko_ecs::ecs::core::iters::legacy_query::Query as LegacyQuery;

crates/boyko_ecs/benches/query_iter.rs:126, 161, 245, 254
  Query::with_component_ids(...), Query::with::<...>(...)
  → LegacyQuery::with_component_ids(...), LegacyQuery::with::<...>(...)
```

**Internal tests within the renamed file** (`legacy_query.rs`): the 30+ tests inside `iters/query.rs` reference `Query::with_*` etc. The file rename keeps the type name `Query` unchanged (just the module path changes), so these tests are unaffected.

**Doc references** in `iters/query_state.rs` comments (3 mentions of `Query::iter_one` / `Query::with_exact_mask` / `Query::test_complex_filtering`): updated to refer to `LegacyQuery`.

**Why the alternative is rejected (M5)**: Keeping the legacy at `iters::query::Query` and putting the new typed DSL at `iters::query_dsl::Query` (a sibling module) was considered. It is rejected because:

1. The migration of `iter_one` / `iter_two` happens **IN PLACE** inside the legacy `query.rs` (column-table fast path swap-in per §8.1). The file cannot be cleanly moved to a sub-module without splitting `QueryIterOne` / `QueryIterTwo` away from `Query<'a>`'s constructors, and those constructors form a tightly-coupled API surface.
2. The new typed DSL is the canonical public name (`Query`) for users; the legacy is a transition shim. Burying the new DSL under `iters::query_dsl::` would invert the convention.
3. `cargo doc` would show two `Query` types under `iters::*`, confusing readers.
4. **O5**: The two-vs-three lifetime symbol concern (legacy `Query<'a>` vs new `Query<'w, 's, D, F>`) is resolved cleanly by the rename: the legacy is named `LegacyQuery<'a>` everywhere it's used, removing all symbolic overlap. No other lifetime symbol in the codebase collides with `'w` / `'s`.

The 10-callsite mechanical rename is one-shot churn; the alternative is permanent namespace clutter.

### 17.4 Build order

```
iters/query/data.rs ─┐
                      ├─→ iters/query/state.rs ──┐
iters/query/filter.rs┘                            │
                                                  ├─→ iters/query/iter.rs ──→ iters/query/query.rs
                                                  │
iters/query_state.rs (edited) ────────────────────┘
iters/archetype_bit_set.rs (edited) ───┘
iters/legacy_query.rs (rename + edit) ──┘ (independent)
```

### 17.5 Test isolation

Each new file gets its own `#[cfg(test)] mod tests` block. Component IDs reserved per existing convention:

| Range | Use |
|-------|-----|
| 100-149 | Phase 8b QueryData unit tests. |
| 150-199 | Phase 8b QueryFilter unit tests. |
| 600-699 | Phase 8b Query<D, F> integration tests. |

---

## 18. Implementation plan (step-by-step)

### Step 1 — Module skeleton

**Files**: `iters/query/mod.rs`, `iters/query/data.rs` (skeleton), `iters/query/filter.rs` (skeleton), `iters/query/state.rs` (skeleton), `iters/query/iter.rs` (skeleton), `iters/query/query.rs` (skeleton).
**Action**: create empty modules with trait signatures (no method bodies). Add to `iters/mod.rs`.
**Acceptance**: `cargo check` passes.

### Step 2 — `QueryData` trait + `&T` / `&mut T` impls (M2 split)

**File**: `iters/query/data.rs`.
**Action**:
- Define `QueryData` and `ReadOnlyQueryData` per §4.1 with `set_table_readonly` + `set_table_mut` split (M2).
- Implement `&T: QueryData + ReadOnlyQueryData` per §4.2.
- Implement `&mut T: QueryData` per §4.3 — `set_table_readonly` panics; `set_table_mut` is real.
- Add `MAX_QUERY_DATA_ARITY = 12`.
- Unit tests: IS_READ_ONLY values, init_state caches, init_access intra-conflict panic, matches_component_set predicate.
**Acceptance**: `cargo test --all-targets iter::query::data` green.

### Step 3 — `QueryFilter` trait + `With`, `Without`, `()` impls (M2 split)

**File**: `iters/query/filter.rs`.
**Action**:
- Define `QueryFilter` per §5.1 with split set_table.
- Implement `()`, `With<C>`, `Without<C>` per §5.2/5.3/5.5.
- Unit tests: IS_ARCHETYPAL == true, matches_component_set, aggregate_include/exclude bits.
**Acceptance**: `cargo test --all-targets iter::query::filter` green.

### Step 4 — Variadic tuple impls (M4 paired-ident macros + M8 Or override)

**Files**: `iters/query/data.rs`, `iters/query/filter.rs`.
**Action**:
- Add `impl_query_data_tuple!((D, s, f), ...)` per §4.6 (M4) for arity 1..=12.
- Add stubs for arity 13..=24 via `const { panic!(...) }`.
- Add `impl_query_filter_tuple_and!` and `impl_or_filter_tuple!` per §5.4 / §5.6 with explicit aggregate_include/exclude no-overrides on Or (M8).
- Auto-emit `ReadOnlyQueryData` for read-only tuples.
- Unit tests: AND-folded IS_READ_ONLY, tuple matches_component_set, Or matches_component_set OR semantics, 12-arity compiles, 13-arity stub trips const-panic.
**Acceptance**: `cargo test --all-targets iter::query::{data,filter}` green; 13-arity panics with expected diagnostic.

### Step 5 — `QueryState` helpers + `ArchetypeBitSet` helpers (M1)

**Files**: `iters/query_state.rs`, `iters/archetype_bit_set.rs`.
**Action**:
- Add `matched_ids_mut`, `remove_matched_at`, `matched_archetypes_bitset` (M1), `last_observed_*` per §6.2.
- Add `ArchetypeBitSet::remove` and `ArchetypeBitSet::popcount` (M1) per §6.3.
- Unit tests: remove_matched_at clears bit + drops; ArchetypeBitSet::remove idempotent; popcount matches set bits; last_observed_* returns snapshots.
**Acceptance**: existing query_state tests pass; new tests pass.

### Step 6 — `QueryDataState<D, F>` (M1 dual-invariant assertion)

**File**: `iters/query/state.rs`.
**Action**:
- Implement `QueryDataState::new` per §6.1.
- Implement `update`, `init_access`.
- Implement `post_filter_matched` with `assert_dual_invariant()` call at end (M1).
- Implement `assert_dual_invariant` (cfg(debug_assertions) only).
- Unit tests: new populates correctly; update short-circuits warm; post-filter drops Or misses; init_access forwards; assert_dual_invariant detects synthetic violations (test the assertion mechanism).
**Acceptance**: `cargo test --all-targets iter::query::state` green.

### Step 7 — `QueryIter` / `QueryIterMut` (M2 split set_table dispatch) + Miri verification

**File**: `iters/query/iter.rs`.
**Action**:
- Implement `QueryIter::new` and `Iterator::next` per §7.1 — calls `archetype_ptr` (not `_mut`) and `set_table_readonly` (M2).
- Implement `QueryIterMut::new` and `Iterator::next` — calls `archetype_ptr_mut` and `set_table_mut`.
- Manually verify const-fold optimisation via `cargo expand` on `Query<_, Without<C>>::iter_mut()` — confirms no `filter_fetch` call in the inner loop.
- **Miri test (M2)**: add `#[cfg_attr(miri, test)]` unit test that constructs a hand-built `QueryDataState`, runs `QueryIter::next()` over a 2-archetype layout, and verifies no UB. This empirically confirms the M2 split eliminates the Tree Borrows uncertainty (Round 1's R5).
- Unit tests: single archetype yields all, archetype transition, empty archetype skip (Q4), stale id skip (Q5), iter_mut mutations persist, Without filter excludes archetype, const-fold archetypal.
**Acceptance**: `cargo test --all-targets iter::query::iter` green; `cargo +nightly miri test iter::query::iter` green.

### Step 8 — `Query<'w, 's, D, F>` + `SystemParam` impl (C1 IntoIterator, C2 no field assert, C3 fixed lifetimes)

**File**: `iters/query/query.rs`.
**Action**:
- Implement `Query<'w, 's, D, F>` struct + `archetype_count`, `is_empty`, `iter`, `iter_mut` per §3.1.
- Implement `IntoIterator for &Query<...>` and `IntoIterator for &mut Query<...>` per §3.2 (C1).
- Implement `unsafe impl<'a, 'b, D, F> SystemParam for Query<'a, 'b, D, F>` per §14.3 (C3 fixed binder, M7 SAFETY note on state.update borrow).
- Add `iter()` `where D: ReadOnlyQueryData` bound (C2: no field-level debug-assert; trait gate is the enforcement).
- **Compile-only tests (C1, C3)**:
  - `assert_impl::<Query<'_, '_, &A>>()` exercises the generic SystemParam blanket (C3).
  - `fn _check_into_iter_ref(q: &Query<'_, '_, &A>) { for _ in q {} }` proves IntoIterator for &Query.
  - `fn _check_into_iter_mut(mut q: Query<'_, '_, &mut A>) { for _ in &mut q {} }` proves IntoIterator for &mut Query.
  - `fn _check_iter_gated() { /* Query<&mut A>.iter() must fail to compile */ }` — `compile_fail` doctest.
- Runtime tests: construct Query from hand-built state and iterate; end-to-end via `run_closure_once::<Query<&A>, _, _>(...)`.
**Acceptance**: `cargo test --all-targets iter::query::query` green; the `compile_fail` doctest fails as expected.

### Step 9 — Rename `query.rs` → `legacy_query.rs`; migrate `iter_one` / `iter_two` (M5)

**Files**: `iters/query.rs` → `iters/legacy_query.rs`; `iters/mod.rs`.
**Action**:
- `git mv crates/boyko_ecs/src/ecs/core/iters/query.rs crates/boyko_ecs/src/ecs/core/iters/legacy_query.rs`.
- Patch `iters/mod.rs` per §14.1: rename `pub mod query` to `pub mod legacy_query`; add `pub mod query` (new directory); re-export per §14.1.
- Patch `QueryIterOne::load_archetype` and `QueryIterTwo::load_archetype` per §8.1.
- Update all 10 callsites in `archetype_master.rs` and `benches/query_iter.rs` per §17.3 (M5 enumeration).
- Update doc comments in `query_state.rs` referencing `Query::*` to `LegacyQuery::*`.
- Verify existing tests in `legacy_query.rs` pass without modification (the file rename doesn't touch the test bodies; the type name `Query` is unchanged within the file).
**Acceptance**: `cargo test --all-targets iters::legacy_query` green; full crate `cargo check` green; benchmark shows ~30 % speedup vs. pre-migration.

### Step 10 — End-to-end integration test

**File**: `crates/boyko_ecs/tests/phase_8b_integration.rs` (new).
**Action**:
- Movement system: `Query<(&mut Position, &Velocity), Without<Frozen>>`.
- Spawn 3 archetypes: `(P, V)`, `(P, V, Frozen)`, `(P, V, Health)`.
- Spawn 100 entities across them.
- Run via `ecs.run_closure_once::<Query<...>, _, _>(movement)` using the `for (mut pos, vel) in &mut q { ... }` sugar (validates C1).
- Assert: archetypes 1 and 3 moved; archetype 2 (Frozen) did not.
- Test `(Query<&A>, Query<&B>)` tuple SystemParam.
- Test intra-system conflict: `(Query<&mut A>, Query<&A>)` panics at init.
**Acceptance**: test green.

### Step 11 — Criterion benchmarks

**File**: `crates/boyko_ecs/benches/phase_8b_query.rs` (new).
**Action**: bench `Query<&A>::iter`, `Query<(&A, &B)>::iter`, `Query<&mut A>::iter_mut`, archetype-transition cost, `Query<&A, Without<B>>`, `Query<&A>` over 10 archetypes × 1K entities.
**Acceptance**: hot-loop targets met per §1.2; cold-path init < 200 ns.

### Step 12 — Documentation

**Files**: doc comments in all new public items; `docs/SYSTEMS.md`; `docs/FEATURE_MAP.md`.
**Action**: each public trait, struct, method gets `///`. Module-level doc in `iters/query/mod.rs`. Update internal docs.
**Acceptance**: `cargo doc --no-deps` builds clean.

### Step 13 — Final `cargo test --all-targets` pass

**Acceptance**: all green; Miri pass on `iters::query::iter::tests` (M2 verification).

### Step 14 — `cargo expand` golden test for const-fold (O3)

**File**: `crates/boyko_ecs/tests/phase_8b_expand_golden.rs` or a CI script.
**Action**:
- Use `cargo expand --tests` (or a custom build script invoking `rustc -Zunpretty=expanded`) on a tiny test crate containing `Query<&A, Without<B>>::iter().next()`.
- Pipe expanded output to a snapshot file. Manual review confirms no `filter_fetch` symbol in the inner loop.
- Add a CI check that re-runs expand and compares against the snapshot. Diffs require human approval.

**Note**: rustc's `--pretty=expanded` may not include LLVM-level DCE info; the golden test asserts the const-fold AT THE SOURCE LEVEL (i.e., `if const { F::IS_ARCHETYPAL }` becomes `if true` after macro expansion + const evaluation, and the dead branch is visible in the expanded source). A complementary `cargo asm` snapshot can verify the LLVM DCE.

**Acceptance**: golden snapshot committed; CI passes against it.

---

## 19. Metrics and validation

### 19.1 Mandatory unit tests

| Test | File | What it verifies |
|------|------|------------------|
| `read_query_data_is_read_only` | `data.rs` | `<&T as QueryData>::IS_READ_ONLY == true`. |
| `mut_query_data_is_not_read_only` | `data.rs` | `<&mut T as QueryData>::IS_READ_ONLY == false`. |
| `tuple_read_only_propagates` | `data.rs` | `<(&A, &B)>::IS_READ_ONLY == true`. |
| `tuple_with_mut_not_read_only` | `data.rs` | `<(&mut A, &B)>::IS_READ_ONLY == false`. |
| `init_access_declares_read` | `data.rs` | After `<&T>::init_access`, set carries the read bit. |
| `init_access_declares_write` | `data.rs` | After `<&mut T>::init_access`, set carries the write bit. |
| `intra_system_write_write_panics` | `data.rs` | `<(&mut T, &mut T)>::init_access` panics. |
| `intra_system_read_write_panics` | `data.rs` | `<(&T, &mut T)>::init_access` panics. |
| **`set_table_readonly_on_mut_data_panics`** (M2/QD4) | `data.rs` | `<&mut T>::set_table_readonly(_, _, _)` panics with the expected message. |
| `with_filter_is_archetypal` | `filter.rs` | `<With<C>>::IS_ARCHETYPAL == true`. |
| `without_filter_is_archetypal` | `filter.rs` | Same for Without. |
| `or_filter_archetypal_iff_all_inputs_archetypal` | `filter.rs` | `Or<(With<A>, With<B>)>::IS_ARCHETYPAL == true`. |
| `with_filter_aggregates_include` | `filter.rs` | `With<C>::aggregate_include` sets the bit. |
| `without_filter_aggregates_exclude` | `filter.rs` | Same for exclude. |
| **`or_filter_no_aggregate_contribution`** (M8) | `filter.rs` | `Or<F>::aggregate_include` AND `aggregate_exclude` are no-ops (verified via mask remaining empty post-call). |
| `query_state_new_populates_matched_ids` | `state.rs` | After `new`, matched_ids contains expected archetypes. |
| `query_state_update_short_circuits_warm` | `state.rs` | Two `update`s with no churn run with no `post_filter` call (mock state observed via assert_dual_invariant counter). |
| `query_state_post_filter_drops_or_misses` | `state.rs` | `Or<(With<A>, With<B>)>` against archetype with only C drops it. |
| **`assert_dual_invariant_detects_violation`** (M1) | `state.rs` | Synthetic test: manually corrupt `matched_archetypes` bitset; assert_dual_invariant panics in debug. |
| `query_iter_single_archetype_yields_all` | `iter.rs` | `Query<&A>::iter()` over 5-entity archetype yields 5. |
| `query_iter_archetype_transition` | `iter.rs` | 2 archetypes (3+2) → 5 items in archetypal-major order. |
| `query_iter_skips_empty_archetype` | `iter.rs` | Empty archetype skipped. |
| `query_iter_skips_stale_id` | `iter.rs` | After `master.remove_archetype(id)`, iter skips (Q5). |
| `query_iter_mut_mutations_persist` | `iter.rs` | `iter_mut().for_each(|x| *x = 99)` then `iter()` yields 99s. |
| `query_iter_filter_with_excludes_archetype` | `iter.rs` | `Query<&A, Without<B>>` skips archetype with B. |
| `query_iter_const_fold_archetypal` | `iter.rs` | `cargo expand` golden file confirms no `filter_fetch` call (Step 14). |
| **`query_iter_readonly_calls_set_table_readonly`** (M2) | `iter.rs` | Trace test: `QueryIter::next` invokes only `set_table_readonly`, never `_mut`. Verified via a mock `QueryData` impl that counts calls. |
| **`query_iter_mut_calls_set_table_mut`** (M2) | `iter.rs` | Symmetric counter test. |
| **`query_iter_miri_clean`** (M2) | `iter.rs` (cfg miri) | Miri run over 2-archetype iter: no UB report. |
| `query_systemparam_impl` | `query.rs` | Compile-only: `assert_impl::<Query<'_, '_, &A>>()`. (C3 — verifies the corrected binder.) |
| **`query_into_iterator_ref`** (C1) | `query.rs` | Compile-only: `for x in &q {}` desugars and compiles. |
| **`query_into_iterator_mut`** (C1) | `query.rs` | Compile-only: `for x in &mut q {}` desugars and compiles. |
| `query_e2e_via_run_closure_once` | `query.rs` | End-to-end runtime test through `run_closure_once`. |
| `legacy_query_iter_one_still_works` | `legacy_query.rs` | Post-migration `iter_one` returns correct values. |
| `legacy_query_iter_two_still_works` | `legacy_query.rs` | Post-migration `iter_two` returns correct pairs. |

### 19.2 Property-based tests

| Test | What it verifies |
|------|------------------|
| `query_iter_matches_legacy_query` | For 1000 random layouts, `Query<&A>::iter().count() == LegacyQuery::with::<A>(&master).iter_one::<A>().count()`. |
| `query_post_filter_idempotent` | Calling `post_filter_matched` twice yields same matched_ids. |
| `tuple_query_data_matches_individual` | For random `(&A, &B)`, `(A,B)::matches == &A::matches AND &B::matches`. |
| **`assert_dual_invariant_holds_after_random_ops`** (M1) | After random sequences of update + post_filter + remove_matched_at, the invariant holds. |

### 19.3 Criterion benchmarks

| Benchmark | Target |
|-----------|--------|
| `query_single_component_iter_10k` | ≤ 60 µs total. |
| `query_two_component_iter_10k` | ≤ 90 µs total. |
| `query_mut_component_iter_10k` | ≤ 60 µs total. |
| `query_iter_archetype_transition_cost` | 10 archetypes × 1K entities; per-transition ≤ 50 ns. |
| `query_state_warm_update` | ≤ 5 ns. |
| `query_state_cold_init` | ≤ 1 µs for 50 archetypes. |
| `legacy_iter_one_post_migration` | ~30% faster than pre-migration. |

### 19.4 Mandatory debug_assert! invariants (C2, C4)

```rust
// In ReadFetch / WriteFetch::set_table_readonly / set_table_mut:
debug_assert!(!column.ptr.is_null(), "QD2: column null in archetype matched by query");

// In Query::iter_mut:
//   NO field-level debug_assert (C2). Q1 is enforced by:
//   * Type system: D: ReadOnlyQueryData bound on iter().
//   * Cell-level: UnsafeEcsCell::archetype_ptr_mut's existing
//     debug_assert!(self.allows_mutable_access) fires from the iter cursor.

// In post_filter_matched (cfg debug_assertions, M1):
#[cfg(debug_assertions)]
Self::assert_dual_invariant(archetype_state);
// (Method spelled out in §6.1.)

// NO `F::is_or_filter()` debug-assert (C4). The Round-1 `if !F::is_or_filter()
// { debug_assert_eq!(pre_count, post_count) }` block referred to a helper
// that does not exist. Removed entirely. Non-Or post-filter no-op behaviour
// is verified by the `query_state_update_short_circuits_warm` unit test
// (which observes that the invariant counter doesn't advance when no churn
// occurs).
```

### 19.5 Miri test coverage

- Run `cargo +nightly miri test --all-targets` scoped to `iters::query::*`.
- Verify no UB on:
  - Cell-by-value pass through `get_param → Query → QueryIter`.
  - Mutable iter on 2-archetype layout (provenance through `archetype_ptr_mut`).
  - **Read-only iter: M2 split signature eliminates the `*const → *mut` cast** (the Round-1 concern in R5).
  - Const-fold path under `Without<C>`.
  - `IntoIterator` for `&Query` and `&mut Query` (C1).

---

## 20. Cross-phase dependencies

### 20.1 Depends on (landed)

- **Phase 7** (fast random access) — column-table read pattern. Verified: `archetype.columns: [Column; MAX_COMPONENTS]` at offset 0.
- **Phase 8a** (SystemParam + Resources): `unsafe trait SystemParam`, `UnsafeEcsCell` by-value receivers, `FilteredAccessSet`, `SystemMeta`, `run_closure_once`, `intra_system_conflict_panic`.

### 20.2 Enables (consumes Phase 8b)

- **Phase 8c**: `FunctionSystem<F, M>` will infer `P = Query<D, F>` from closure signatures.
- **Phase 8d** (`Commands`): orthogonal SystemParam.
- **Phase 9** (parallel scheduler): consumes `Query`'s Access declaration for the conflict graph.
- **Phase 10** (change detection): adds `Changed<C>` / `Added<C>` as new `QueryFilter` impls with `IS_ARCHETYPAL = false`. Uses both `set_table_readonly` and `set_table_mut` (M2 split is Phase-10-ready).
- **Phase 11** (SIMD): may add `Vectorize<D>` adapter.

### 20.3 Backward compatibility

| Legacy item | Status post-Phase-8b |
|-------------|----------------------|
| `iters::Query` (legacy) | Renamed to `iters::LegacyQuery`. All 10 callsites updated (mechanical fix per §17.3). |
| `iters::query::Query` (legacy module path) | Renamed to `iters::legacy_query::Query` (and re-exported as `LegacyQuery`). |
| `Query::with_*` constructors | Unchanged (on `LegacyQuery`). |
| `Query::iter_one` / `iter_two` | Unchanged signatures; bodies migrated to column-table. |
| `Query::iter` (archetype-iter) | Unchanged. |
| `Query::from_archetypes`, `archetypes` | Unchanged. |

The new `iters::Query` is the typed DSL. **No legacy caller is broken** (the rename is mechanical).

---

## 21. Risks and mitigations

### 21.1 R1 — Macro complexity (M4 resolved)

The paired-ident macro `impl_query_data_tuple!((D0, s0, f0), ...)` is concrete (§4.6); no `paste!` dependency. Worked-example arity-3 expansion in §25.

### 21.2 R2 — Parallel state/fetch binding (M4 resolved)

The paired-ident scheme (`(D, s, f)`) provides distinct value-idents per element for `state` and `fetch` destructures. See §4.6 and the arity-3 expansion in §25.

### 21.3 R3 — Const-fold of `if const { F::IS_ARCHETYPAL }`

**Risk**: rustc may not always const-fold `if const { CONST_BOOL }` in generic contexts.
**Mitigation**:
1. `const { ... }` block (stable since 1.79; boyko targets 1.85+) forces compile-time eval. Result is a literal; LLVM DCE removes the dead branch at -O2.
2. Step 14's `cargo expand` golden test (O3) catches regressions.
3. Fallback: specialization (unstable) — deferred.

### 21.4 R4 — `post_filter_matched` cost on archetype churn (C4 acknowledged)

**Risk**: Apps creating/destroying archetypes every frame trigger post_filter per frame.
**Mitigation**:
1. Archetype set stabilises after a few frames in practice.
2. Per §15.5 (C4), worst case `Query<(), Or<arity-12>>` × 1024 archetypes = ~60 µs per generation bump. Acceptable.
3. Phase 9 may cache post-filter results indexed by `(generation, structural_generation)`.

### 21.5 R5 — Tree Borrows on `*const → *mut Archetype` cast (M2 RESOLVED)

**Round 1 risk**: Read-only `QueryIter::next` cast `*const Archetype → *mut Archetype` for the unified `set_table` signature. Tree Borrows uncertainty.
**Round 2 resolution (M2)**: `QueryData::set_table` is **split** into `set_table_readonly(_: *const Archetype)` and `set_table_mut(_: *mut Archetype)`. The cast is eliminated entirely; the read-only path's signature accepts `*const` natively. Step 7 includes a Miri test verifying no UB on the read-only iter — empirical confirmation that the Round-1 concern is structurally moot.

### 21.6 R6 — `QueryFilter::Fetch<'w>: Copy` constraint

**Risk**: Future filter types may need non-Copy Fetch.
**Mitigation**: Phase 8b's filters all have trivial Copy Fetches. Phase 10's `Changed<C>::Fetch<'w> = *const u32` is Copy. Phase 12+ may relax.

### 21.7 R7 — Architecture-critic concerns (Round 2 closeout)

All Round-1 critic findings (C1-C4, M1-M8, O1-O5) are resolved as enumerated in §0. Anticipated Round-2 follow-ups:

| Concern | Pre-empted by |
|---------|---------------|
| Tree Borrows on read-only cast | M2 split eliminates cast; Step 7 Miri test verifies. |
| GAT lifetime quantification | C3 fix: `<'a, 'b, D, F>` binder. |
| Drop discipline on `QueryDataState::Drop` | Vec<ArchetypeId> has well-defined drop; no panic-in-drop. |
| Intra-system aliasing | FilteredAccessSet returns Err → panic. |
| Const-panic stubs | `const { panic!(...) }` per Phase 8a M7+C-NEW-2. |
| Cell mutability flow for `&mut T` | §9, traced step-by-step; M2 eliminates one cast. |
| `assert_dual_invariant` cost | Debug-only (cfg(debug_assertions)); zero release cost. |
| `Or<F>` `aggregate_*` correctness | M8: explicit override prevents future contributor bugs. |

### 21.8 R8 — Borrow checker rejection of `iter_mut(&mut self)`

**Risk**: GAT lifetime quantification on `iter_mut(&mut self) -> QueryIterMut<'_, 's, ...>` may misbehave.
**Mitigation**: same blanket pattern as Phase 8a's `Res`. **C3** fixed the impl head — `<'a, 'b, D, F>` binder. Step 8's compile-only test `assert_impl::<Query<'_, '_, &A>>()` exercises the generic SystemParam blanket, confirming the binder resolves correctly.

### 21.9 R9 — `post_filter_matched` high-water-mark optimisation (O4, Phase-10+)

**Risk**: Round-1 critic note O4: post_filter walks the full `matched_ids` even when only a tail was newly added by `update_archetypes`.
**Mitigation (deferred)**: Phase 10 (or later) can add a `high_water_mark: usize` field on `QueryDataState` that records the `matched_ids.len()` value after the last `post_filter_matched` call. The next `update` only post-filters `matched_ids[high_water_mark..]`. Saves redundant work on churning workloads.

**Tracking item**: `phase-10-followup-1: post_filter_matched high_water_mark optimisation`. Phase 8b ships the simple full-scan version per §6.1; the optimisation is unmeasurable below 100 matched archetypes and only worth implementing once Phase 9 scheduler data informs the cost.

---

## 22. Out of scope

| Feature | Phase | Why deferred |
|---------|-------|--------------|
| `IntoSystem` / `FunctionSystem` (closure inference) | 8c | Orthogonal to Query DSL. |
| `Commands` (deferred mutations) | 8d | Different SystemParam shape. |
| `Changed<C>`, `Added<C>` | 10 | Requires tick infrastructure; trait shape ready. |
| `Optional<C>` | 10 | Requires per-row null check. |
| `Entity` as QueryData | 8c bundle | Trivial impl; bundled with World SystemParam. |
| `EntityCommands` | 8d | Requires Commands. |
| `&World` SystemParam | 8c | Different shape. |
| `Local<T>` SystemParam | Deferred | No user yet. |
| Parallel iteration | 9 | Requires scheduler. |
| SIMD vectorisation | 11 | Requires per-archetype chunking. |
| `#[derive(QueryData)]` for user structs | 8c bundle | Proc-macro work. |
| `Query::get(entity)` random-access | 8c | Uses Phase 7's `get_component_raw`. |
| `Query::single()` / `iter_one()` ergonomics | Deferred | Wait for use-case demand. |
| **`high_water_mark` post-filter optimisation (O4)** | **Phase-10-or-later** | Tracked as `phase-10-followup-1`. Cost-vs-complexity unjustified at Phase 8b's scale. |

---

## 23. Open questions

### Q-OPEN-1: post_filter cost amortisation for `Or<F>` (resolved via §6.4 / §15.5 / C4)

The cost is documented explicitly per C4. Acceptable at Phase 8b scale. Deferral tracked as R9 / `phase-10-followup-1` for the high-water-mark optimisation.

### Q-OPEN-2: legacy `iters::Query` rename strategy (resolved via §17.3 / M5)

Rename to `LegacyQuery`. Alternative explicitly rejected per §17.3 (in-place migration of `iter_one`/`iter_two` couples the legacy type tightly to its file).

### Q-OPEN-3: `D::matches_component_set` redundancy with QueryState's include mask (resolved)

Keep the post-filter for safety; debug-assert verifies non-Or no-op behaviour. Phase 10's Changed/Added will need the per-id pass anyway.

### Q-OPEN-4: `QueryFilter::aggregate_include` for `With<C>` (resolved)

`ComponentMask::set` is idempotent; double-add is safe.

### Q-OPEN-5: stale-id skip (resolved via Q5)

Defence-in-depth; cost is one Option::None branch per archetype boundary. Negligible.

### Q-OPEN-6: ReadOnlyQueryData blanket impls (resolved)

NO blanket via const-generic predicates (unstable). Emit `ReadOnlyQueryData` explicitly per type. `Has<C>` (Phase 11) gets manual impl.

### Q-OPEN-7: `Query::archetype_count()` semantics (resolved)

Ship only `archetype_count` + `is_empty` in 8b. Add `iter_archetypes` later if demand surfaces.

### Q-OPEN-8: `Query::get_one(EntityId)` random access (resolved)

Defer to 8c. Phase 8b ships only iter/iter_mut.

### Q-OPEN-9: const-fold IS_ARCHETYPAL verification (resolved via Step 14)

Step 14 (O3) adds a `cargo expand` golden test. Acceptance gate for Phase 8b ships with this.

---

## 24. References

### 24.1 Internal references

- **Phase 7 plan**: `docs/plans/PHASE-07-fast-random-access.md`.
- **Phase 8a plan**: `docs/PHASE-8A-SYSTEMPARAM-PLAN.md`.
- **Phase 8 master plan**: `docs/plans/PHASE-08-system-api.md`.
- **Existing QueryState**: `crates/boyko_ecs/src/ecs/core/iters/query_state.rs`.
- **Existing Query<'a>**: `crates/boyko_ecs/src/ecs/core/iters/query.rs` (to be renamed).
- **Archetype + Column**: `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs`.
- **ArchetypeBundle**: `crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs`.
- **SystemParam trait**: `crates/boyko_ecs/src/ecs/core/system/system_param.rs`.
- **UnsafeEcsCell**: `crates/boyko_ecs/src/ecs/core/system/unsafe_ecs_cell.rs`.
- **FilteredAccessSet**: `crates/boyko_ecs/src/ecs/core/system/filtered_access_set.rs`.
- **Tuple impl macro (Phase 8a)**: `crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs`.
- **Res / ResMut (Phase 8a)**: `crates/boyko_ecs/src/ecs/core/system/params/res.rs` / `resmut.rs`.

### 24.2 External references

- **Bevy QueryData**: `bevy_ecs/src/query/fetch.rs`.
- **Bevy QueryFilter**: `bevy_ecs/src/query/filter.rs`.
- **Bevy QueryState**: `bevy_ecs/src/query/state.rs`.
- **Bevy QueryIter**: `bevy_ecs/src/query/iter.rs`.
- **flecs Query**: filter-pushdown architecture (not adopted).
- **Sander Mertens ECS FAQ**: archetypal vs sparse-set tradeoffs.
- **Mike Acton "Data-Oriented Design" (GDC 2014)**.

---

## 25. Appendix — arity-3 concrete macro expansion (M4)

The paired-ident macro `impl_query_data_tuple!((D0, s0, f0), (D1, s1, f1), (D2, s2, f2));` expands to:

```rust
unsafe impl<D0: QueryData, D1: QueryData, D2: QueryData> QueryData for (D0, D1, D2) {
    type State = (D0::State, D1::State, D2::State);
    type Fetch<'w> = (D0::Fetch<'w>, D1::Fetch<'w>, D2::Fetch<'w>);
    type Item<'w> = (D0::Item<'w>, D1::Item<'w>, D2::Item<'w>);

    const IS_READ_ONLY: bool = true
        && D0::IS_READ_ONLY
        && D1::IS_READ_ONLY
        && D2::IS_READ_ONLY;

    #[inline]
    fn init_state(world: &mut EcsMaster) -> Self::State {
        (
            <D0 as QueryData>::init_state(world),
            <D1 as QueryData>::init_state(world),
            <D2 as QueryData>::init_state(world),
        )
    }

    #[inline]
    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        let (s0, s1, s2) = state;
        <D0 as QueryData>::init_access(s0, access_set);
        <D1 as QueryData>::init_access(s1, access_set);
        <D2 as QueryData>::init_access(s2, access_set);
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        let (s0, s1, s2) = state;
        true
            && <D0 as QueryData>::matches_component_set(s0, mask)
            && <D1 as QueryData>::matches_component_set(s1, mask)
            && <D2 as QueryData>::matches_component_set(s2, mask)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        let (s0, s1, s2) = state;
        <D0 as QueryData>::aggregate_include(s0, include);
        <D1 as QueryData>::aggregate_include(s1, include);
        <D2 as QueryData>::aggregate_include(s2, include);
    }

    #[inline]
    fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
        let (s0, s1, s2) = state;
        (
            <D0 as QueryData>::init_fetch(s0),
            <D1 as QueryData>::init_fetch(s1),
            <D2 as QueryData>::init_fetch(s2),
        )
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        let (f0, f1, f2) = fetch;
        let (s0, s1, s2) = state;
        unsafe { <D0 as QueryData>::set_table_readonly(f0, s0, archetype); }
        unsafe { <D1 as QueryData>::set_table_readonly(f1, s1, archetype); }
        unsafe { <D2 as QueryData>::set_table_readonly(f2, s2, archetype); }
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        let (f0, f1, f2) = fetch;
        let (s0, s1, s2) = state;
        unsafe { <D0 as QueryData>::set_table_mut(f0, s0, archetype); }
        unsafe { <D1 as QueryData>::set_table_mut(f1, s1, archetype); }
        unsafe { <D2 as QueryData>::set_table_mut(f2, s2, archetype); }
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        let (f0, f1, f2) = fetch;
        (
            unsafe { <D0 as QueryData>::fetch(f0, row) },
            unsafe { <D1 as QueryData>::fetch(f1, row) },
            unsafe { <D2 as QueryData>::fetch(f2, row) },
        )
    }
}

unsafe impl<D0: ReadOnlyQueryData, D1: ReadOnlyQueryData, D2: ReadOnlyQueryData>
    ReadOnlyQueryData for (D0, D1, D2) {}
```

Note: `s0`, `s1`, `s2`, `f0`, `f1`, `f2` are distinct value-bindings; `D0`, `D1`, `D2` are distinct type-parameters. No `paste!` is needed. The macro emits exactly this code for the matching invocation; the same pattern scales unchanged to arity 12.

---

## Orchestrator briefing — 12 most load-bearing decisions (Round 2)

1. **Reuse `QueryState` verbatim, wrap with `QueryDataState<D, F>`** (§6). Maintains QS1 dual-invariant via M1.
2. **Migrate `iter_one`/`iter_two` to column-table in place** (§8). Public API unchanged.
3. **Three-GAT QueryData shape (`State`, `Fetch<'w>`, `Item<'w>`)** (§4).
4. **`ReadOnlyQueryData` marker trait gates `Query::iter(&self)`** (§3.1, §4.1). Type-level Q1 enforcement.
5. **`QueryFilter::IS_ARCHETYPAL` const + `filter_fetch` per row** (§5.1). Const-folded in cursor.
6. **Cell-by-value flow for mutable provenance** (§9). No `&self` retag.
7. **`Or<F>` is post-filtered, not mask-aggregated** (§5.4, §6.4). Complexity bounded per C4.
8. **Variadic tuple impls via paired-ident `macro_rules!`, arity 12, const-panic stubs 13..=24** (§4.6, §25). M4 resolves Round-1 pseudo-syntax.
9. **Intra-system aliasing caught at `init_access` via `FilteredAccessSet`** (§16.3).
10. **`Query<'w, 's, D, F>` is a thin 24-byte handle; `IntoIterator` impls support `for x in &q` / `for x in &mut q`** (§3.1, §3.2, C1).
11. **Hot-loop per-row cost ≤ 6 ns for `&A`, ≤ 9 ns for `(&A, &B)`** (§1.2).
12. **Split `set_table_readonly` / `set_table_mut` eliminates the Round-1 Tree Borrows cast concern** (M2). Phase-10-ready (both methods will be used by `Changed<C>::Fetch<'w>` with non-archetypal semantics).

---

**End of Phase 

8b plan (Round 2)**. Total length: ~3,200 lines.

### Files to save the plan to

- **Plan output target**: `D:\claude\BoykoEngine\docs\PHASE-8B-QUERY-DSL-PLAN.md` (overwrite Round 1)

### Files referenced for cross-check

- `D:\claude\BoykoEngine\docs\PHASE-8A-SYSTEMPARAM-PLAN.md` (style/depth template)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query.rs` (legacy, to be renamed to `legacy_query.rs` + internals migrated)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query_state.rs` (reuse target + helpers)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\archetype_bit_set.rs` (add `remove`, `popcount` helpers)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs` (column-table contract)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype_bundle.rs` (`get_archetype_ptr*`)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype_master.rs` (callsite update per §17.3)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\system_param.rs` (Phase 8a trait to implement)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\unsafe_ecs_cell.rs` (by-value cell flow; `allows_mutable_access` field at line 79 is NOT exposed — C2 resolution)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\filtered_access_set.rs` (intra-system conflict detection)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\res.rs` (SystemParam impl style template)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\tuple_impl.rs` (macro emission template)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\fn_once_system.rs` (end-to-end runner)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` (Phase 7 fast read path + `run_closure_once`)
- `D:\claude\BoykoEngine\crates\boyko_ecs\benches\query_iter.rs` (callsite update per §17.3 — 6 references)

### Round 2 changelog summary (mirrors §0)

**Critical (4)**: C1 `IntoIterator` impls added (§3.2, §14.1, Step 8). C2 field-level debug-assert dropped, replaced by type-level gate + existing cell-level assert (§3.1, §19.4). C3 malformed `'_` impl head replaced with `<'a, 'b, D, F>` binder (§14.3, §21.8). C4 `F::is_or_filter()` removed; complexity spelled out explicitly (§6.4, §15.5, §19.4).

**Major (8)**: M1 QS1 invariant + `assert_dual_invariant` (§2.3, §6.1, §6.3, §19.1). M2 `set_table` split into `set_table_readonly`/`set_table_mut` (§4.1-§4.6, §5.1-§5.6, §7.1, §9, §21.5). M3 set_table cost budget Phase-10 contribution language (§1.2). M4 paired-ident macro + arity-3 worked example (§4.6, §25). M5 callsite enumeration + alternative rejection (§17.3). M6 size formula corrected (§13.3). M7 SAFETY note on `state.update` borrow (§14.3). M8 explicit `aggregate_*` overrides on `Or<F>` (§5.4).

**Quality (5)**: O1 paired-ident syntax (M4). O2 `LegacyQuery` reference in exit criteria (§1.1). O3 Step 14 added (§18). O4 high-water-mark deferral tracked (§21.9, §22). O5 lifetime symbol overlap resolution note (§17.3).

Ready for `architecture-critic` review.