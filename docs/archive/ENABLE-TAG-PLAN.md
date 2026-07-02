<!-- Enable-Bit (sparse, non-fragmenting) tag backend - 'EnableTag'. APPROVED by architecture-critic round 7 (0 CRITICAL / 0 MAJOR) after 7 rounds. The round cap was removed mid-design (user directive); a 3-round cap would have shipped 3 unresolved CRITICALs. See docs/PERF-GAP-BEAT-BEVY.md context. -->

The critique confirms two real bugs in my round-6 plan. Both CRITICAL claims are verified against source:

- **C1 confirmed.** `With<C>::init_access` (filter.rs:330-334) declares `add_component_read`, NOT a no-op. Only `Without<C>` (filter.rs:431) is a no-op. `Added<C>` (filter.rs:589-597) also declares a read. My "mirrors With" precedent was inverted. The real soundness argument is structural: an EnableTag id is filtered out of every signature (D5) and has no `ComponentPool`, so a sibling `&mut` on the same id is structurally impossible — that, not "mirrors With," is the justification.

- **C2 confirmed.** `init_state(world: &mut EcsMaster) -> Self::State` is generic only over the filter (verified With:325, Without:426, Added:582). It cannot name `D::HAS_DATA_COMPONENT`. My const-assert as placed does not compile. It must move to the `(D, F)`-generic construction boundary.

I have enough to revise.

---

# Architecture: Enable-Bit (Sparse, Non-Fragmenting) Tag Backend — "EnableTag" (Revised, Round 7)

## Changelog (round 6 → round 7)

| Critique (round-6) | Resolution |
|---|---|
| **C1 (CRITICAL): the `init_access` no-op justification cites an INVERTED precedent ("mirrors With<C>")** | **VERIFIED FALSE + RE-DERIVED.** Source check: `With<C>::init_access` (filter.rs:330-334) declares `add_component_read(state.id)` with an explicit intra-system-aliasing rationale (:294-299, :311-315); `Added<C>::init_access` (filter.rs:589-597) ALSO declares a read (FLT2); ONLY `Without<C>::init_access` (filter.rs:430-431) is a no-op, and its doc (:398-401) states why: "it inspects the absence of a bit; it performs no read of `C`'s data and cannot conflict with any sibling param." **The no-op decision STANDS, but the precedent is now `Without<C>`, and the soundness argument is STRUCTURAL, not "mirrors With."** The real argument (D8, fully rewritten): `With`/`Added`/`Changed` declare a conservative read precisely because a sibling `&mut C` in the SAME system CAN exist (the component is in the signature and has a `ComponentPool` the sibling writes), so the aliasing detector must serialize the filter's logical read of `C`'s lifecycle against that `&mut C`. **For an EnableTag this sibling is STRUCTURALLY IMPOSSIBLE**: D5 filters the bitset id OUT of every archetype signature and gives it NO `ComponentPool`, so no `&C`/`&mut C` data param can ever resolve against that id (`C::component_id()` for a bitset tag names a slot that has no column to fetch — a data param would fail at fetch). With no possible sibling data access on the id, there is nothing for the aliasing detector to serialize against; the no-op is correct, and (as before) declaring `add_component_read` would manufacture a false conflict with an unrelated system. `Without<C>`'s "absence inspection ⇒ no access" is the structurally analogous precedent. |
| **C2 (CRITICAL): the C2 const-assert is in `Enabled<T>::init_state`, which cannot name `D`** | **VERIFIED FALSE-AS-WRITTEN + RELOCATED.** Source check: `QueryFilter::init_state(world: &mut EcsMaster) -> Self::State` is generic ONLY over the filter (With:325, Without:426, Added:582) — it has no `D` parameter and `D::HAS_DATA_COMPONENT` is unnameable there; my round-6 sketch does not compile. **Fix: the combined predicate moves to the `(D, F)`-generic `QueryState` construction site.** Located: `QueryState::new` / the `Query` type-construction funnel in `query.rs` (the single place both `D: QueryData` and `F: QueryFilter` are in scope and `matched_ids`/`filter_state` are built — Step 7a pins the exact `fn` after graphify+source verification). There, a `const { assert!(!F::CONTAINS_ENABLE_TERM || D::HAS_DATA_COMPONENT || F::HAS_POSITIVE_ARCHETYPAL, "<message>") }` runs per `(D, F)` monomorphization. New consts: `QueryFilter::CONTAINS_ENABLE_TERM` (default `false`; `true` for `Enabled`/`Disabled` and AND-tuples/Or containing one), `QueryFilter::HAS_POSITIVE_ARCHETYPAL` (default `false`; `true` for `With` + AND-tuples containing one), `QueryData::HAS_DATA_COMPONENT` (`true` iff ≥1 real component; `()` = false). All additive const defaults — zero ABI change to existing filters, zero runtime cost. The headline "compile-reject sole-Enabled" is now pinned to a real `(D, F)` seam. |
| **C3 (MAJOR): get/get_mut enable-test creates a partial-filter asymmetry (honors Enabled, silently ignores Changed)** | **ADOPTED — option (b) + (c).** Adding the enable test at `get`/`get_mut` (query_view.rs:493/549) while `Changed`/`Added` stay ignored there makes `Query<&P, (Changed<P>, Enabled<A>)>::get(e)` honor `Enabled` but silently drop `Changed` — worse than uniform ignorance (users will assume per-row filters now work). **Decision: (b) COMPILE-REJECT mixing a non-archetypal change-detection filter (`Added`/`Changed`) with an Enable term in a query, AT THE SAME `(D,F)` construction seam as C2** — `const { assert!(!(F::CONTAINS_ENABLE_TERM && F::CONTAINS_CHANGE_DETECTION), "<message>") }` (new const `CONTAINS_CHANGE_DETECTION`, true for `Added`/`Changed`/tuples-containing). This removes the confusing partial-filter shape entirely. PLUS **(c)**: the `get`/`get_mut` rustdoc gains an explicit "non-archetypal change-detection filters (`Changed`/`Added`) are NOT applied by point lookups; use iteration" note (closes the documentation gap on the pre-existing BUG-ENABLE-PRE-2 without retrofitting `Changed` into point lookups — out of scope, M3). I REJECT option (a) (retrofit `Changed` into get/get_mut this phase): it widens scope into the change-detection point-lookup contract, needs its own tick-meta plumbing at the get site, and risks the 0%-gate; it is filed as BUG-ENABLE-PRE-2 and stays a separate isolated wave. |
| **C4-r6 (MAJOR): the C2 redesign silently narrows the brief's "many flags / mark-from-jobs / entity-disabling" use cases (rejects sole-flag cross-archetype enumeration)** | **FLAGGED EXPLICITLY + scoped + verified against the named v1 cases.** The Goal and OUT-of-scope sections now state in bold: **sole-flag enumeration across archetypes (`Query<(), Enabled<A>>` = "every entity with flag A regardless of components") is NOT supported in v1** — only (i) point `is_enabled` and (ii) positive-term-bounded iteration (`Query<&D, Enabled<A>>`). Audited against the brief's NAMED v1 cases: **`Selected`** (UI/picking — always iterated WITH `&Transform`/`&Renderable`, bounded ✓), **`Stunned`/`OnGround`** (gameplay — iterated WITH `&Velocity`/`&Health`, bounded ✓), **pool reservation** (iterated WITH the pooled component, bounded ✓). **Entity-disabling is the one case that genuinely wants "every disabled entity regardless of components"** (Bevy `DefaultQueryFilters` model). **Decision: entity-disabling's cross-archetype `Disabled`-scan is EXPLICITLY OUT of v1 scope and named as such** (it needs the D7 candidate-seeded `update_archetypes` variant); v1 entity-disabling is supported only as a per-row `Without`-style cull on positive-term queries (the common "skip disabled while iterating real components" path, which IS bounded). The D7 seam is documented as the single in-scope-able extension if the brief owner later promotes entity-disabling's global scan. This is surfaced as an **Open question for the brief owner** rather than silently narrowed. |
| **O1 (MINOR): swap_remove_bit wiring — confirm it fires once at move_out_entity (no-drop, all 4 helpers) vs remove_entity (drop), and add a Last/pop branch test** | **VERIFIED + PINNED + TEST ADDED.** Source: `remove_entity` (archetype.rs:541-568, the DROP path) and `move_out_entity` (:594-616, the no-drop path used by all 4 migration helpers at migration_helpers.rs:406/654/1003/1306) both compute `last = current-1` and branch `RemoveOutcome::{Last(pop), Swapped(swap)}`. **Decision: the bit op is wired at BOTH `remove_entity` AND `move_out_entity` (they are disjoint paths — drop vs no-drop — never both for one operation), and NEVER in the helper bodies (which call `move_out_entity`), so it fires exactly once per structural op (O1 double-count avoided by construction).** The `Last` (pop) branch does `clear(last)` only (no swap); the `Swapped` branch does the READ-first `swap_remove_bit(removed,last)`. **New mandatory test: RemoveOutcome::Last pop branch clears the popped row's bit** (round-6 oracle covered only `Swapped`). |
| **O2 (MINOR): five verified-good decisions — preserve** | **PRESERVED unchanged**: (1) 0%-gate `if !const { F::IS_ARCHETYPAL }` elision + `_no_meta` NCD=false routing + `has_enable_term`-gated cull; (2) `OrComposable` seal (the non-archetypal-in-AND-Or leak is real at filter.rs:1059-1068 / :282 / :388); (3) C4 migration READ-before-`move_out_entity` ordering; (4) W1 macro `LitStr` NameValue arm; (5) W2 separate `enable_generation` (structural bump clears the whole cache at query_state.rs:174-180). |

(Round-1→6 changelogs retained at bottom.)

## Goal

A SECOND tag storage backend for HIGH-CHURN boolean flags (`Stunned`, `OnGround`, `Selected`, pool reservation, per-row entity-disabling). Per-component opt-in via `#[component(storage = "bitset")]` / `register_enable_tag(name)`. Trades three Phase-22 costs:

- **Eliminates** archetype migration on toggle. Toggle = one word read-modify-write, **O(1), no migration, no structural-generation bump**.
- **Eliminates** archetype fragmentation. An EnableTag is NOT in the signature → zero new archetypes.
- **Eliminates** the per-spawn tick-pool floor (~7 ns/tag/entity): an EnableTag spawns bit-clear; no per-archetype tick-pool commit.

Price IN: `Enabled<T>`/`Disabled<T>` become a per-ROW bit test during iteration, with a per-tag **archetype-presence cull** over a BOUNDED matched set. `Added`/`Changed` are compile-rejected (D4). `for_each_chunk` is compile-rejected (D2). `Enabled`/`Disabled` inside `Or<>` are compile-rejected (M1). `Enabled<T>` REQUIRES a positive archetypal term — a sole/data-less query is compile-rejected (C2). Mixing `Enabled`/`Disabled` with `Added`/`Changed` in one query is compile-rejected (C3-r7).

**Supported query shapes (round-7 explicit)**:
- Point: `is_enabled<T>(e)` / `is_enabled_id(e, tag)` — any tag, any archetype, O(1). ✓
- Positive-term-bounded iteration: `Query<&D, Enabled<A>>`, `Query<&D, (With<X>, Enabled<A>)>`, `Query<&D, Disabled<A>>` (with a positive term). ✓
- **NOT supported in v1 (explicit narrowing — round-7 C4):** sole-flag enumeration across archetypes (`Query<(), Enabled<A>>` = "every entity with flag A regardless of which components it has"). The named v1 use cases (`Selected`, `Stunned`, `OnGround`, pool reservation) all pair the flag with a positive data term, so all are supported. **Entity-disabling's global "every disabled entity" scan is the one named pattern that wants the rejected shape; it is OUT of v1 scope** (D7 candidate-seeded `update_archetypes` seam) — see Open questions. v1 entity-disabling is supported as a per-row `Disabled`/`Without`-style cull on positive-term queries (the common "skip disabled while iterating real components" path).

**Non-negotiable 0%-gate**: a query naming no EnableTag term is byte-identical to today — structurally guaranteed by (a) reusing the existing `if !const { F::IS_ARCHETYPAL }` const-fold, (b) NOT modifying any existing filter's `Fetch`/`set_table_*`/`matches_component_set`/`filter_fetch`/`init_access`, and (c) gating the new `enable_generation` update check + cull pass behind a per-state `has_enable_term` bool.

## Context and constraints

**Affected**: component registry (`STORAGE_KIND` cold table); a NEW `QueryFilter` family (`Enabled`/`Disabled`) — additive, existing impls untouched, with explicit no-op `init_access` (C1 — structurally justified, NOT "mirrors With"); a NEW `cull_enable_archetypes` pass over the bounded matched set; a NEW `enable_generation` world counter (forward-seam atomic — W2) + per-state `has_enable_term` update check (O2); the C2/C3 const-asserts at the `(D, F)` `QueryState` construction seam; all 4 migration helpers + `move_out_entity` (paged bit-copy, sequenced — C4-r5); `remove_entity`/`move_out_entity` (swap_remove bit, fires once — O1-r7); the toggle API; `QueryView::get`/`get_mut` per-row enable test (C3-r5 — narrowed to these two) + a rustdoc note on the non-archetypal-change-detection gap (C3-r7-c); entity deallocate (assert only); the macro (`storage="bitset"` LitStr parse — W1); spawn (no signature membership); `Or` sealed bound (M1).

**Untouched** (0%-gate critical): `With`/`Without`/`Added`/`Changed` `Fetch`, `set_table_*`, `filter_fetch`, `matches_component_set`, `init_access`, `aggregate_include` — ALL byte-identical. `ComponentPool` byte/tick storage; the Phase-22.1 `term_list` slice machinery; the Phase-9 executor; the `Access`/`FilteredAccessSet` conflict model (no new category in v1 — Enable filters declare a no-op, C1); `driver_ids()` (NO restrict hook); `single`/`single_mut` (inherit via `iter`); `update_archetypes`/`QueryState.matched_ids` machinery (C2: Enabled bounded by a real include bit, never the empty-include path).

**Invariants preserved**:
- 0%-gate on every existing query/spawn/iteration path (sacred — bench-verified).
- `MAX_COMPONENTS = 512` shared id space (EnableTags consume one id each).
- `ComponentLayout` stays pinned 56 B / one cache line (kind in a parallel cold table).
- Tree-Borrows soundness on the per-row read (`AtomicU64` interior mutability, mirrors Phase-10 `UnsafeCell<Tick>`).
- An EnableTag id NEVER enters an archetype signature mask (else it fragments) AND has no `ComponentPool` (the C1 structural-soundness premise).
- Entity-row-bit consistency across `swap_remove` and all 4 migration paths.
- `EnablePresence[id]` ⟺ archetype has an allocated column for `id` (the cull's correctness invariant).
- A toggle-driven column-alloc is observed by every cached enable-naming query before its next iterate (O2).
- No single EnableColumn allocation exceeds one page (512 B — round-5 paging).
- The migration bit-copy READ precedes the source `swap_remove`, at the same sequence point as the component-byte swap (round-5 C4); the read snapshot is borrow-free `Copy` (round-6 W3).
- **`Enabled<T>` contributes a positive include bit via a paired archetypal term; the C2/C3 const-asserts enforce shape at the `(D, F)` construction seam (round-7).**
- **No Enable filter declares component access in v1; the no-op is sound because a sibling data access on a bitset id is structurally impossible (round-7 C1).**
- **The swap-remove bit op fires exactly once per structural op (`remove_entity` XOR `move_out_entity`, never the helper body — round-7 O1).**

**Target metrics**:
- Toggle: ≤ 5 ns warm, **0 alloc steady-state, 0 migration, 0 structural-gen bump**, 1 `enable_generation` bump + 1 ≤512 B page alloc on first-touch-of-a-page only.
- `is_enabled` point query: ≤ 5 ns.
- Per-row filter in iteration: ≤ 1.5 ns/row amortized (one hoisted word load per 64 rows + 1 bit test/row); the presence cull skips never-toggled-here archetypes within the bounded set.
- 0% regression: `query_iter`, `query_iter_changed`, `query_iter_with_filter`, `par_iter`, `for_each_chunk`, `spawn`, `spawn_batch_10k`.
- Spawn-with-EnableTag = spawn cost (no signature, no tick pool).
- `Enabled` cull bounded to O(matched_ids) — the matched set itself bounded by the required positive term (C2). No full-world scan path exists.
- Max single column alloc = 512 B.

## Key decisions

### Decision D1: Storage = per-archetype, row-indexed, PAGED bitset column; per-tag archetype-presence bitset as the cull oracle

**What**: For each EnableTag id, each Archetype that has it toggled owns a lazily-allocated **paged** `EnableColumn`: a `Box<[Option<Box<EnablePage>>]>` page directory, where `EnablePage = [AtomicU64; 64]` (= 512 B, covers 4096 rows). Page `p` covers rows `[p*4096, (p+1)*4096)`; word `w` within a page covers rows `[w*64, …)`; bit = `unit_index & 63`. A column allocates ONLY the pages a toggle touches (first toggle = one 512 B page). Columns live in a new `EnableStore` on the `Archetype` (parallel to `component_pools`), stored as an inline-4 small list of `(ComponentId, Box<EnableColumn>)`. Separately, a process-global per-tag `EnablePresence` records the set of archetype-ids that have an allocated column for that tag — a per-tag archetype **bitset** (`Box<[u64; 16]>` = 128 B/tag, lazily allocated), giving O(1) `contains` (the cull oracle). Column-alloc (the first page) bumps the per-world `enable_generation` (O2) and sets the tag's presence bit.

The bit's home is `(archetype, row)`, exactly like component data and Phase-10 tick columns. **No global EntityId bitset, no sparse-set.**

**Why**: survives migration correctly because row-indexed (travels through the existing row-copy loop — C4); cache behavior matches the data it filters (one 512 B page per 4096 rows, lockstep, hoistable, auto-vectorizable); no recycling leak BECAUSE swap_remove mirrors the swap (C2-orig); the presence bitset restores the coarse cull over the bounded matched set. Paging caps any single alloc at 512 B (round-5). `AtomicU64` = the lock-free primitive for the D3/D7 worker-marking seam at zero read cost (`load(Relaxed)` = plain `mov`).

**Why the presence bitset is the cull oracle and NOT a query driver (C2)**: query_state.rs:195-203 materializes the full `matched_ids` set unconditionally for an empty include mask (verified), so a presence-driven enumeration would have to SHRINK a full set, O(all live). Round-7 keeps the round-6 fix: `Enabled<T>` requires a positive term that contributes a real include bit, bounding `update_archetypes`; `EnablePresence` is consulted ONLY as the O(1) `contains` cull oracle over that bounded set. `for_each_present`/`present_count` are NOT provided.

**Alternatives rejected**: global EntityId bitset (recycling leak, extra write per unrelated migration); sparse-set (pointer-chase, Bevy #2144); flecs full DontFragment out-of-table sparse (slower multi-component iter); flat (non-paged) `Box<[AtomicU64]>` whole-archetype (32 KB single-alloc footgun); presence-driven sole-`Enabled` enumeration (matched set still full — C2).

**Trade-off**: a tag toggled across K archetypes has K columns + K presence bits; each column is a page directory + touched pages. The page directory adds one hoistable indirection (`pages[row>>12]`, loop-invariant per 4096-row block). `Enabled<T>` cannot be a sole/data-less query term (C2). Correct trade (bits co-located with rows; bounded alloc; bounded matched set) at one extra hoistable indirection + one compile-time term-shape constraint.

**Memory cost**: per `(archetype,tag)` a page directory `ceil(rows/4096) * 16 B` plus 512 B per TOUCHED page (lazy). 4096-row archetype with any toggle = 16 B dir + 512 B page = ~528 B. Per archetype using ANY EnableTag an inline-4 list (64 B inline, heap spill > 4). Per tag one `EnablePresence` bitset 128 B, lazy. Never allocated for never-toggled archetypes/tags/pages.

**D1 sub-decision — backing & regrow**: `EnablePage = [AtomicU64; 64]` boxed; the page directory `Box<[Option<Box<EnablePage>>]>` sized to `ceil(reserve_rows/4096)` at column creation, regrown at the `&mut` apply window when the pool's `reserve_rows` crosses a 4096 boundary (W3-r5) — directory regrow re-`Box`es the directory (16 B/entry) and moves the `Box<EnablePage>` ptrs (no page data copy). `set_table_*` re-reads the directory base + page base per archetype (no live Fetch during regrow). A page is allocated on first toggle into its 4096-row range.

**D1 sub-decision — `enable_generation` atomicity (W2)**: `enable_generation: AtomicU64` on `ArchetypeMaster`. It is `AtomicU64` PURELY as the D7 forward seam; in v1 it is bumped under `&mut self` only and read in `update()` (single-threaded). `Relaxed` is sound only because no concurrent access exists in v1. It is deliberately separate from `structural_generation` (a structural bump force-rebuilds every cache at query_state.rs:174-180; a per-toggle full invalidation would be catastrophic).

**D1 invariants** (debug_assert):
1. `swap_remove_bit` called for every allocated column on every row vacate; post-condition checked at the `swap_remove_row` site (vacated row holds former-last bit, last bit clear).
2. directory `len == ceil(reserve_rows/4096)`; a present page covers exactly 4096 rows; `row < reserve_rows` on every bit op.
3. EnableTag id absent from every archetype signature mask AND has no `ComponentPool` (C1 premise).
4. `EnablePresence[id]` contains A ⟺ A's `EnableStore` has a column for `id` (checked on alloc).
5. Column first-page-alloc bumps `enable_generation` exactly once per column (O2 consistency).
6. Migration/move_out: the source bit READ at `source_row` happens BEFORE the source `swap_remove_row`, at the same per-column sequence point as the component-byte swap; the read snapshot is borrow-free `Copy` (C4 + W3-r6).

### Decision D2: Query integration — `Enabled<T>` is a presence-CULLED non-archetypal filter REQUIRING a positive term; `Disabled<T>` is AND-only; BOTH forbidden in `Or`; mixing with `Added`/`Changed` forbidden; explicit per-row test ONLY at `get`/`get_mut`

**What**: Two `QueryFilter` impls + dynamic `with_enabled`/`without_enabled`. Both set:
```rust
const IS_ARCHETYPAL: bool = false;          // activates the per-row branch (the Changed mechanism)
const NEEDS_CHANGE_DETECTION: bool = false; // no tick meta path
const CONTAINS_ENABLE_TERM: bool = true;    // C2/C3 const-assert input (default false elsewhere)
```

**`init_access` — explicit no-op in v1; STRUCTURAL soundness, NOT "mirrors With" (round-7 C1)**:
```rust
#[inline] fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
    // ENBL-ACCESS-1: Enable filters declare NO component access in v1.
    //
    // PRECEDENT: this mirrors `Without<C>` (filter.rs:430-431, the ONLY no-op
    // leaf), NOT `With<C>`/`Added<C>`/`Changed<C>` (which DECLARE a read).
    //
    // WHY `With`/`Added`/`Changed` declare a conservative read: a sibling
    // `&mut C` data param CAN exist in the same system (C is in the signature
    // and has a ComponentPool the sibling writes), so the intra-system aliasing
    // detector must serialize the filter's logical read of C's lifecycle
    // against that &mut C.
    //
    // WHY an EnableTag does NOT: D5 filters the bitset id OUT of every archetype
    // signature and gives it NO ComponentPool. Therefore NO `&C`/`&mut C` data
    // param can ever resolve against this id (there is no column to fetch) — a
    // sibling data access on the id is STRUCTURALLY IMPOSSIBLE. With no possible
    // sibling, there is nothing for the aliasing detector to serialize against,
    // exactly as for `Without<C>`'s absence-inspection. Declaring
    // add_component_read(tag_id) would be WRONG: it would manufacture a false
    // conflict with an unrelated system and imply a change-detected read
    // contract the backend does not honor.
    //
    // D7 worker-marking is the ONLY place that adds an access declaration (a new
    // EnableWrite category — see D8).
}
```
0%-gate: existing leaves' `init_access` byte-identical.

**`matches_component_set` — UNCHANGED signature**. `Enabled<T>::matches_component_set(state, mask) = true`; `aggregate_include`/`aggregate_exclude` = NO-OPs. The per-archetype presence verdict is delivered through a NEW, separate cull pass (`cull_enable_archetypes`) gated by `has_enable_term`:

- NON-enable queries: `post_filter_matched` byte-identical (`has_enable_term=false`; new pass skipped). **0%-gate intact.**
- enable-naming queries: after `post_filter_matched`, `cull_enable_archetypes` runs over the BOUNDED matched set:
  - `Enabled<T>`: archetype KEPT iff `EnablePresence[id].contains(arch_id)`. No column = all-disabled ⇒ DROPPED. **The coarse cull.**
  - `Disabled<T>`: KEPT unconditionally. **No cull** (documented cost).

**C2/C3 enforcement at the `(D, F)` construction seam (round-7 — RELOCATED)**:

The combined-shape predicates are NOT in any filter leaf's `init_state` (which is filter-only-generic and cannot name `D` — verified). They live at the **`QueryState`/`Query` construction funnel where both `D: QueryData` and `F: QueryFilter` are in scope** (`query.rs`; Step 7a pins the exact `fn` — `QueryState::new` or the `Query` type-construction site — after graphify+source verification). Three new additive consts feed two const-asserts:

```rust
// QueryData (data.rs leaves + tuple macro): true iff ≥1 real component; () = false.
trait QueryData  { const HAS_DATA_COMPONENT: bool; }
// QueryFilter (filter.rs; additive defaults — NO ABI change to existing leaves):
trait QueryFilter {
    const HAS_POSITIVE_ARCHETYPAL: bool = false;   // With<C> + AND-tuples containing one ⇒ true
    const CONTAINS_ENABLE_TERM:    bool = false;   // Enabled/Disabled + tuples/Or containing one ⇒ true
    const CONTAINS_CHANGE_DETECTION: bool = false; // Added/Changed + tuples/Or containing one ⇒ true
}

// At the (D, F) construction site (query.rs):
const _C2: () = assert!(
    !F::CONTAINS_ENABLE_TERM || D::HAS_DATA_COMPONENT || F::HAS_POSITIVE_ARCHETYPAL,
    "`Enabled<T>`/`Disabled<T>` require a positive archetypal term (a data \
     component in the Query's data, or `With<_>`). `Query<(), Enabled<A>>` \
     would scan every live archetype. Add `With<TheArchetypeYouMean>`, or use \
     `is_enabled` for a point query."
);
const _C3: () = assert!(
    !(F::CONTAINS_ENABLE_TERM && F::CONTAINS_CHANGE_DETECTION),
    "an `Enabled<T>`/`Disabled<T>` term cannot be combined with `Added`/`Changed` \
     in one query: point lookups (`get`/`get_mut`) apply the enable bit but not \
     change-detection, which would silently mislead. Split into two queries."
);
```
These are `const _: () = assert!(...)` items evaluated at each `(D, F)` monomorphization — the same const-eval mechanism as D4, zero runtime cost, no ABI change. (Implementation note for Step 7a: the `const` items must be forced per-monomorphization — placed in a generic `const fn`/`impl` body keyed on `(D, F)` that the construction path provably instantiates, e.g. a `QueryState::<D,F>::ASSERT_SHAPE` associated const referenced from `QueryState::new`. The dev verifies the const is actually evaluated via a trybuild fail test, mirroring the D4 pattern that Phase-12.5 proved must be a referenced associated const, not a free `const fn` body.)

**Per-row test (the residual filter, gated by `IS_ARCHETYPAL=false`)**:
- `Enabled<T>::filter_fetch(f, row)`: `f.col.is_null() ? false : (*f.col).test(row)` where `test(row)` = `pages[row>>12].map_or(false, |p| (p[(row>>6)&63].load(Relaxed) >> (row&63)) & 1 == 1)`. No page = all disabled ⇒ false (but presence-culled archetypes never reach here for `Enabled`).
- `Disabled<T>::filter_fetch(f, row)`: inverted — no column/no page ⇒ true.
- `set_table_*_no_meta`: REAL body — caches the column ptr (or null) for the archetype (the `Changed` `_no_meta`-carries-real-body shape — W4-r2, O1-r5 verified). The per-page deref is hoisted per 4096-row block in the cursor.
- `set_table_*_with_meta`: NCD=false ⇒ never routed (panics if reached, mirroring `Added`/`Changed` `_no_meta` backstop at filter.rs:665-694 inverted).

**`Or` rejection (M1)**: `Enabled<T>`/`Disabled<T>` do NOT implement the sealed `OrComposable` trait → `Or<(Enabled<A>, …)>` is a compile error. Verified load-bearing: Or folds a non-archetypal per-row `filter_fetch` against an archetypal element's unconditional `true` (filter.rs:282/388, fold at :1059-1068), leaking disabled rows — the reject is the correct fix.

**Driver coverage matrix (C3-r5 — source-verified)**:

| Driver | Routes through `filter_fetch`? | Enable handling |
|---|---|---|
| `iter` / `iter_mut` | YES | INHERITED — `filter_fetch` per-row, gated by `if !const { F::IS_ARCHETYPAL }` (iter.rs ~189-301) |
| `par_iter` / `par_iter_mut` | YES | INHERITED (par_iter.rs / par_chunk.rs same gate) |
| `single` / `single_mut` | **YES** — both are `self.iter()/iter_mut() + .next()` (query_view.rs:419/442) | **INHERITED** — explicit test would double-filter; no change |
| `for_each_chunk` / `par_for_each_chunk` | requires `F: ArchetypalQueryFilter` | COMPILE ERROR for `Enabled`/`Disabled` (as `Changed`) |
| `QueryView::get` / `get_mut` | **NO** (query_view.rs:464/519: call `archetype_passes_tag_terms` at :493/:549, never `filter_fetch`) | **EXPLICIT per-row test added** at :493/:549 + rustdoc note that `Changed`/`Added` are NOT applied here (C3-r7-c). The ONLY driver pair needing it. |

`count`/`any` are NOT public `QueryView` methods (Grep-verified) — absent from the matrix/tests/benches.

**Pre-existing latent gaps filed, NOT fixed here (M3 decoupling)**:
- **BUG-ENABLE-PRE-1**: `Or<(Changed<A>, With<B>)>` leaks disabled-A rows in a B-lacking archetype. Filed; this phase does not touch `Changed`-in-Or.
- **BUG-ENABLE-PRE-2**: `QueryView::get`/`get_mut` silently ignore `Changed<C>`/`Added<C>` today. Filed; this phase adds the enable per-row test at get/get_mut, adds the rustdoc note (C3-r7-c), but does NOT retrofit `Changed` there. Mixing `Enabled`+`Changed` is now compile-rejected (C3-r7-b), so the confusing partial-filter shape cannot be constructed. `single`/`single_mut` NOT affected (route through `iter`).

**Dynamic terms**: EnableTag terms do NOT use the Phase-22.1 `term_list` slice machinery. An `EnableTerms` stack struct (≤ `MAX_ENABLE_TERMS = 8`) in the cursor, tested per-row behind a `has_dynamic_enable_terms` runtime gate (Phase-16 isolation), AND the dynamic presence cull folded into `cull_enable_archetypes` (`dyn_enable_passes(arch_id)`). A query with neither typed nor dynamic enable term is byte-identical. **A dynamic `with_enabled` on a `Query<(), ()>` view is a RUNTIME panic** (const-asserts cover typed terms only; dynamic terms cannot be const-checked, so the bounding requirement is enforced at the `with_enabled` call site: it asserts the view's matched set is bounded, i.e. the query already names a positive term). Documented.

**Trade-off** (documented in `storage-tradeoffs.md`):
- `Enabled`/`Disabled` cannot use `for_each_chunk` (compile-rejected).
- `Enabled`/`Disabled` cannot be inside `Or<>` v1 (compile-rejected).
- `Enabled<T>` REQUIRES a positive archetypal term (compile-rejected as a sole/data-less term — C2).
- `Enabled`/`Disabled` cannot be combined with `Added`/`Changed` in one query (compile-rejected — C3-r7).
- `Disabled` cannot presence-cull AND cannot be a sole/only-archetype term (compile-rejected — model the enabled state as the positive tag).
- The cull is paid per `update` (per generation/enable-generation bump), not per `iter()`. Asymmetry vs `Changed` (which sets the include bit) documented.

### Decision D3: Toggle = `&mut self` non-structural op v1; full apply-site enumeration; clean worker-marking seam

**What**: `enable<T>`/`disable<T>`/`is_enabled<T>` (typed) + `_id` (dynamic) on `EcsMaster` (`&mut self`) and deferred via `EntityCommands`. Toggle does NOT migrate, bump the STRUCTURAL gen, or fire structural hooks/observers (flecs CanToggle: no add/remove events). It DOES bump `enable_generation` on first column-page-alloc only (O2). v1 = `&mut self`, runs in the structural/apply window, NOT from a live worker.

**Toggle algorithm (`enable`)**:
1. `inland = entities_inland[e.id().0]`; null/gen mismatch → silent no-op.
2. `(archetype_ptr, row) = (inland.archetype_ptr(), inland.unit_index())` — current post-swap row, never cached.
3. `(*archetype).enable_store.get_or_alloc_column(tag_id, reserve_rows)` → on first column creation: set bit in `EnablePresence[tag]` + bump `EnablePresence` epoch + bump world `enable_generation` (O2). `column.get_or_alloc_page(row >> 12)` → `#[cold]` 512 B page alloc on first touch. Warm = ≤4 list scan + 1 directory index.
4. `page[(row>>6)&63].fetch_or(1 << (row&63), Relaxed)`.
5. No structural gen bump, no hook, no drain.

**`is_enabled`**: inland load → null/gen check (BEFORE column read) → `enable_store.column(tag_id)` (None ⇒ false) → `column.test(row)` (no page ⇒ false). Row = `inland.unit_index()`.

**Why v1 is `&mut self`-only**: Phase-9 `FilteredAccessSet` maps every `ComponentId` to read/write; worker-marking needs (a) `AtomicU64` (provided), (b) a NEW `EnableWrite` access category excluded from write-conflict but proven disjoint by entity range + a loom/Miri proof + real Acquire/Release (D8). Phase-9-touching + determinism cost → deferred. v1 delivers the PRIMARY motivation (no migration, no fragmentation, no spawn floor).

**Deferred-toggle apply-site enumeration** — verified command-apply sites + bit obligation:

| Apply site | Touches row how | Bit obligation |
|---|---|---|
| `SpawnAtCommand::apply` | append | none (defaults clear — D6) |
| `DespawnCommand` → `remove_entity` (DROP path) | swap_remove / pop | **`swap_remove_bit(removed,last)` READ-first (Swapped); `clear(last)` (Last) — O1-r7** |
| `InsertCommand` → `migrate_entity_insert` (→ `move_out_entity`) / `apply_replace_in_place` | append target + swap-out source via move_out_entity | **paged sequenced copy (C4)**; in-place replace = no row move = no bit op |
| `RemoveCommand` → `migrate_entity_remove` (→ `move_out_entity`) | append + swap-out | **paged sequenced copy (C4)** |
| `AddTagCommand`/`RemoveTagCommand` → `migrate_entity_attach/detach_ids` (→ `move_out_entity`) | append + swap-out | **paged sequenced copy (C4)** |
| `EnableTagCommand`/`DisableTagCommand` (NEW) | flip one bit at `inland.unit_index()` | the toggle |
| 4 deferred-drain sites (Phase-14b) | re-enter queue; terminal op above | covered transitively |

**O1-r7 single-fire guarantee**: the swap-remove bit op is wired at `remove_entity` (DROP) and `move_out_entity` (no-drop) ONLY — never inside the 4 helper bodies (they call `move_out_entity`). The two are disjoint paths (a structural op takes one, never both), so the bit op fires exactly once. `Last`/pop = `clear(last)`; `Swapped` = READ-first `swap_remove_bit(removed,last)`.

**Apply ORDER hazard**: commands apply in FIFO order. **Every bit op resolves the row via `inland.unit_index()` at apply time** (never a captured enqueue-time row). **T-INTERLEAVE test mandatory** (deferred enable(E) + deferred despawn(E')-causing-swap-into-E's-row → survivor bit correct, despawned slot leaks no bit).

**The clean seam (D7/D8)**: `AtomicU64` makes worker-marking purely additive. v1 `&mut self` toggle uses the same `AtomicU64` via `fetch_or`/`fetch_and(Relaxed)` — forward-identical primitive; only the access declaration + memory ordering change (D8).

### Decision D4: `Added`/`Changed` on EnableTags = COMPILE-TIME REJECTED

(Unchanged.) The derive emits `const STORAGE_IS_BITSET: bool`. `Added<C>::init_state`/`Changed<C>::init_state` gain `const { assert!(!C::STORAGE_IS_BITSET, "Added/Changed require signature storage; an enable-bit tag has no per-row tick — use #[derive(Component)] default storage for change detection") }` — a per-monomorphization hard compile error (filter-only-generic, so this CAN live in `init_state` — it names only `C`, not `D`; verified distinct from the C2/C3 case). The Phase-22 D1 "compile-but-lie" lesson. Seam for a future `changed_tick` region/side-set is the single toggle funnel.

### Decision D5: Backend choice — `#[component(storage = "bitset")]` (LitStr) + `register_enable_tag(name)`; kind in a parallel cold `STORAGE_KIND` table

(W1-r6.) Derive parses `storage = "bitset"` as a **NameValue arm with a `syn::Lit::Str` value** (the `on_*=<path>` NameValue arm extended to accept a `LitStr`, NOT the `no_bundle` bare-key shape — verified lib.rs:252-302). Any string other than `"bitset"` ⇒ `compile_error!("unknown component storage \"X\"; expected \"bitset\"")`. Emits `STORAGE_IS_BITSET`, routes registration. Runtime `register_enable_tag(name) -> EnableTagId`. Registry: NEW `static STORAGE_KIND: [AtomicU8; MAX_COMPONENTS]` (0=Table, 1=Bitset), mirroring `HOOKS`, NOT a 6th `ComponentLayout` field. **Kind checked at archetype construction**: any id with `STORAGE_KIND==Bitset` is FILTERED OUT of the signature mask + given NO `ComponentPool` (this is the C1 structural-soundness premise — verify it holds at every construction path). Migration union helpers also exclude bitset ids; `add_tag`/`remove_tag` branch on kind BEFORE migration (route to `enable`/`disable`). **Coexistence**: an entity may hold a signature tag (`Player`) AND an EnableTag (`Stunned`); `has_tag` vs `is_enabled` distinct.

### Decision D6: Spawn — EnableTags never enter the signature, never allocate a tick pool, spawn bit-clear

(Unchanged.) Bitset id filtered out of the signature at construction → no fragmentation, no new archetype, no tick-pool (the ~7 ns/tag/entity floor eliminated by construction), default-clear bit (no column/page until first toggle). EnableTag is a filter handle (no Bundle emission). "Spawn with flag on" = spawn then `enable` (direct) or `commands.spawn(b).enable::<T>()` (deferred, one apply-window toggle).

### Decision D7: Forward-compat seams

- **Entity-handle width (8 B niche)**: row-indexed by `unit_index` (u32), never by `EntityId` representation. ✓
- **Relationships/future kinds**: `STORAGE_KIND` extensible (kind=2). ✓
- **Worker-marking**: `AtomicU64` ready; only the D8 `EnableWrite` category + loom + real Acquire/Release needed. ✓
- **Change detection**: toggle method is the single funnel. ✓
- **`Or`-composition for enable filters**: deferred; the seam is the safe Fetch redesign making an archetypal element's `filter_fetch` archetype-aware, done in isolation. ✓
- **Block-skip SIMD + summary**: v1.1 drop-in — the paged layout makes the per-page summary word a natural extension (`summary[page]` precomputed). ✓
- **Sole-`Enabled` / cross-archetype flag enumeration (round-7 C4 — the entity-disabling global scan)**: deferred; the seam is an `update_archetypes` variant SEEDED from a candidate id set (`EnablePresence`'s set of archetypes) instead of the `1..gen` scan, so a sole `Enabled<A>`/`Disabled<A>` query bounds `matched_ids` to present-for-A archetypes WITHOUT the empty-include full materialization. Out of v1 because it touches `QueryState` core. **This is the single named extension the brief owner may promote into scope if entity-disabling needs the global scan (see Open questions).** ✓

### Decision D8: Access-model contract for Enable filters (round-7 C1 — structural justification)

**v1 contract**: `Enabled<T>`/`Disabled<T>::init_access` is a no-op (code + soundness in D2). The Enable bit is outside the `FilteredAccessSet`/ConflictGraph entirely in v1.

**Precedent (corrected, round-7)**: this mirrors `Without<C>` (filter.rs:430-431) — the ONLY no-op-`init_access` leaf — NOT `With<C>` (filter.rs:330-334, declares a read) or `Added`/`Changed` (filter.rs:589-597, declare a read).

**Soundness (structural, NOT "mirrors With")**: `With`/`Added`/`Changed` declare a conservative `add_component_read` because a sibling `&mut C` in the same system CAN exist — `C` is in the signature, has a `ComponentPool`, and the aliasing detector must serialize the filter's logical lifecycle-read of `C` against that `&mut C`. For an EnableTag this sibling is **structurally impossible**: D5 filters the bitset id out of every signature and gives it NO `ComponentPool`, so no `&C`/`&mut C` data param can ever resolve against the id (there is no column to fetch). With no possible sibling data access, there is nothing to serialize — exactly `Without<C>`'s "absence inspection ⇒ no access, no possible conflict" situation. Declaring a read would manufacture a false conflict with an unrelated system and imply a change-detected read contract the backend does not honor. Additionally, v1 has no concurrent toggler (`enable`/`disable` require `&mut EcsMaster`; the Phase-9.1 join edge provides happens-before), so even setting aside the structural argument, no parallel reader-vs-writer pair exists.

**The exact D7 seam**: when worker-marking lands, `Enabled<T>` keeps its no-op (pure reader), but a NEW `EnableWrite<T>` SystemParam (the worker-toggle handle) declares a NEW `FilteredAccessSet` category `enable_write: ComponentMask` (parallel to `component_writes`), excluded from the normal write-conflict rule but checked for `EnableWrite`-vs-`Enabled`-read overlap and `EnableWrite`-vs-`EnableWrite` overlap on the SAME tag. Disjointness is then proven by entity-range partitioning + a loom model (the bit word is `AtomicU64::fetch_or` with `AcqRel`). This is the ONLY place an Enable type touches the conflict graph. Pinned here so the v1 no-op is not mistaken for "Enable is forever outside the conflict model."

## Data structures

```rust
// ── core/component/enable_store.rs (NEW) ────────────────────────────────────
const WORDS_PER_PAGE: usize = 64;          // 64 * 64 bits = 4096 rows / page
const ROWS_PER_PAGE: usize  = 4096;        // = WORDS_PER_PAGE * 64
#[repr(C, align(64))]
pub(crate) struct EnablePage([AtomicU64; WORDS_PER_PAGE]);   // 512 B, one alloc unit

#[repr(C)]
pub(crate) struct EnableColumn {
    /// Page directory: one slot per 4096-row block; the page is allocated only
    /// when a row in its range is first toggled (round-5: caps any single alloc
    /// at 512 B). Directory entries are 16 B. Regrown at the &mut apply window
    /// when reserve_rows crosses a 4096 boundary (W3) — moves Box ptrs, no copy.
    pages: Box<[Option<Box<EnablePage>>]>,
    // v1.1 seam: `summary: Box<[AtomicU64]>` (one bit per page) for block-skip.
}
impl EnableColumn {
    #[inline] fn test(&self, row: usize) -> bool;             // page None ⇒ false
    #[cold]   fn get_or_alloc_page(&mut self, page: usize) -> &EnablePage;
    fn is_empty(&self) -> bool;
    fn swap_remove_bit(&mut self, removed: usize, last: usize); // C2/C4 READ-before-write
    // page index = row >> 12 ; word in page = (row >> 6) & 63 ; bit = row & 63
}

#[repr(C)]
pub(crate) struct EnableStore {
    /// Inline-4 + heap spill (W2). Dominant access = enumerate allocated
    /// columns (C2 swap, C4 migration); direct iteration, not O(1) point lookup.
    columns: SmallList4<(ComponentId, Box<EnableColumn>)>,
}
impl EnableStore {
    fn column(&self, cid: ComponentId) -> Option<&EnableColumn>;          // scan ≤4
    fn get_or_alloc_column(&mut self, cid: ComponentId, rows: usize) -> &mut EnableColumn; // #[cold]
    fn swap_remove_row(&mut self, removed: usize, last: usize);           // O1; asserts post-cond
    /// C4 phase-1 READ. SOUNDNESS (W3-r6): writes OWNED `(ComponentId, bool)`
    /// Copy values — NEVER a reference into a source column. The scratch is
    /// borrow-free, so its contents survive the phase-3 swap_remove_row that
    /// mutates the very columns just read (the NEW-1 dangling-slice class does
    /// NOT apply because nothing borrows `self` after this returns).
    fn read_row_bits(&self, row: usize, out: &mut SmallList4<(ComponentId, bool)>);
    fn write_row_bit(&mut self, cid: ComponentId, row: usize, bit: bool, rows: usize); // C4 phase-2
    fn is_empty(&self) -> bool;
}
// SmallList4: minimal inline-4 + heap-spill (local ~60 lines or boyko_utils; zero new deps).

// ── core/component/enable_presence.rs (NEW) — cull ORACLE only (C2: not a driver) ──
/// Process-global per-tag archetype bitset of "has an allocated EnableColumn".
/// Box<[u64; 16]> = 128 B/tag, lazily allocated. O(1) `contains` (the cull
/// oracle). Epoch-stamped for the lock-free snapshot read (Phase-22.1 shape).
/// NOTE (C2): `for_each_present`/`present_count` are NOT provided — EnablePresence
/// is NEVER a query driver; the matched set is bounded by the required positive
/// archetypal term. Drop-in seam for D7 (sole-Enabled / entity-disabling scan).
pub(crate) struct EnablePresence { /* per-tag Box<[u64;16]> + epoch */ }
impl EnablePresence {
    fn note_column_alloc(&self, tag: ComponentId, arch: ArchetypeId);     // set bit + bump epoch
    fn contains(&self, tag: ComponentId, arch: ArchetypeId) -> bool;      // O(1) — cull oracle
}

// ── core/component/component_registry.rs (MODIFY) ───────────────────────────
static STORAGE_KIND: [AtomicU8; MAX_COMPONENTS] = [const { AtomicU8::new(0) }; MAX_COMPONENTS];
#[repr(u8)] pub enum StorageKind { Table = 0, Bitset = 1 }
#[inline] pub fn storage_kind(cid: usize) -> StorageKind;          // Relaxed load
pub(crate) fn set_storage_kind(cid: usize, kind: StorageKind);     // write-once
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EnableTagId(pub(crate) ComponentId);
impl EnableTagId { pub const fn component_id(self) -> ComponentId; }

// ── core/iters/query/filter.rs (MODIFY — additive only; NO ABI change) ──────
pub(crate) trait OrComposable: QueryFilter {}   // sealed
// impl for: (), With<C>, Without<C>, Added<C>, Changed<C>, Or<F: OrComposable>,
//   tuple arities 1..=12 (+ >12 stub) where each element is OrComposable.
//   NOT impl'd for Enabled<T>/Disabled<T>.
// impl_or_filter_tuple! (filter.rs:1154) + >12 stub (filter.rs:1417): add
//   `$F: OrComposable` element bound. With/Without/Added/Changed UNCHANGED.

// C2/C3 enforcement consts (additive defaults; NO ABI break):
//   trait QueryData   gains `const HAS_DATA_COMPONENT: bool;`        (() = false)
//   trait QueryFilter gains `const HAS_POSITIVE_ARCHETYPAL: bool = false;` (With ⇒ true)
//   trait QueryFilter gains `const CONTAINS_ENABLE_TERM:    bool = false;` (Enabled/Disabled ⇒ true)
//   trait QueryFilter gains `const CONTAINS_CHANGE_DETECTION: bool = false;` (Added/Changed ⇒ true)
// (tuple/Or macros OR-fold each element's const.)

// ArchetypeMaster gains `enable_generation: AtomicU64` (W2: forward seam; bumped on column-alloc).

// ── core/iters/query/filter_enable.rs (NEW) ─────────────────────────────────
pub struct Enabled<T: Component> { _m: PhantomData<fn() -> T> }
pub struct Disabled<T: Component> { _m: PhantomData<fn() -> T> }
#[derive(Clone, Copy)]
pub struct EnabledState<T> { pub(crate) id: ComponentId, _m: PhantomData<fn() -> T> }
#[derive(Clone, Copy)]
pub struct EnabledFetch<'w> { col: *const EnableColumn, _m: PhantomData<&'w ()> } // NULL = no column

unsafe impl<T: Component> QueryFilter for Enabled<T> {
    type State = EnabledState<T>;
    type Fetch<'w> = EnabledFetch<'w>;
    const IS_ARCHETYPAL: bool = false;
    const NEEDS_CHANGE_DETECTION: bool = false;
    const CONTAINS_ENABLE_TERM: bool = true;     // C2/C3 input
    // HAS_POSITIVE_ARCHETYPAL stays default false (Enabled is NOT a positive term).
    // init_state: resolve id; debug_assert storage_kind==Bitset.  (NO D-naming
    //   const-assert here — it cannot see D; the C2/C3 asserts live at the
    //   (D,F) QueryState construction seam — see D2.)
    // init_access(_,_): NO-OP (D8 / C1) with the ENBL-ACCESS-1 structural comment.
    // matches_component_set(_,_): true (cull is a SEPARATE pass).
    // aggregate_include/exclude: NO-OP.
    // set_table_*_no_meta: REAL body — col = archetype.enable_column_ptr(id) or null (W4).
    // set_table_*_with_meta: panic backstop (NCD=false ⇒ never routed).
    // filter_fetch(f,row): f.col.is_null()? false : (*f.col).test(row)   // paged test
    // NOTE: Enabled does NOT impl OrComposable → compile-rejected in Or (M1).
}
// Disabled<T>: same consts + init_access NO-OP; matches_component_set = true;
//   filter_fetch inverted (NULL/no-page ⇒ true); NO OrComposable impl.
```

## Public API

```rust
impl EcsMaster {
    pub fn register_enable_tag(&mut self, name: &str) -> EnableTagId;
    pub fn try_register_enable_tag(&mut self, name: &str) -> Option<EnableTagId>;
    pub fn enable_tag_by_name(&self, name: &str) -> Option<EnableTagId>;
    pub fn enable<T: Component>(&mut self, e: Entity);          // O(1), no migration
    pub fn disable<T: Component>(&mut self, e: Entity);
    pub fn is_enabled<T: Component>(&self, e: Entity) -> bool;  // ≤5 ns
    pub fn enable_id(&mut self, e: Entity, tag: EnableTagId);
    pub fn disable_id(&mut self, e: Entity, tag: EnableTagId);
    pub fn is_enabled_id(&self, e: Entity, tag: EnableTagId) -> bool;
}
impl EntityCommands<'_, '_> {
    pub fn enable<T: Component>(self) -> Self;
    pub fn disable<T: Component>(self) -> Self;
    pub fn enable_tag(self, tag: EnableTagId) -> Self;
    pub fn disable_tag(self, tag: EnableTagId) -> Self;
}
impl Query<'_,'_, D, F> {
    pub fn with_enabled(self, tag: EnableTagId) -> Self;     // ≤ MAX_ENABLE_TERMS; runtime-bounded
    pub fn without_enabled(self, tag: EnableTagId) -> Self;
}
// + identical on QueryView. Re-exports: ...::query::{Enabled, Disabled}
// get/get_mut rustdoc gains the C3-r7-c note: "Changed/Added are NOT applied by point lookups."
```

## Algorithms for critical paths

**`is_enabled<T>`** — inland load → null/gen check (BEFORE column read) → `enable_store.column(cid)` (None ⇒ false) → `column.test(row)` = page deref (None ⇒ false) + word load + bit test, row=`inland.unit_index()`. O(1); 2 dependent loads + ≤4 scan + 1 page deref + 1 word load. Random access. ≤5 ns.

**`enable`/`disable`** — inland load → null/gen check → `get_or_alloc_column` (warm ≤4 scan; cold column-create + `note_column_alloc` + `enable_generation` bump) → `get_or_alloc_page(row>>12)` (warm directory index; cold 512 B page alloc) → `fetch_or/fetch_and(Relaxed)`. O(1) warm, no migration/structural-gen/hook/drain. ≤5 ns warm.

**Archetype cull (`cull_enable_archetypes`) — per update, NOT per iter, NOT per row, over the BOUNDED matched set (C2)** — walk `archetype_state.matched_ids` (bounded by the required positive term's include bit); for each `Enabled<T>` drop ids where `!EnablePresence.contains(id, arch)`; `Disabled<T>` no cull. O(matched_ids). NEVER the empty-include full-materialization path (compile-rejected). Runs only when `has_enable_term`; non-enable queries skip it (0%-gate).

**Iteration per-row (`Enabled<T>`)** — per row in `if !const { F::IS_ARCHETYPAL }`: `col.is_null()` (predicted-not-taken; culled archetypes don't reach here) → `pages[row>>12]` (loop-invariant per 4096-row block, hoisted) → `page[(row>>6)&63].load(Relaxed)` (loop-invariant per 64-row block, hoisted) → bit test. O(1)/row, sequential data fetch, auto-vectorizable. ≤1.5 ns/row.

**Point-lookup per-row (`get`/`get_mut` — C3-r5)** — at the `archetype_passes_tag_terms` site (:493/:549): after the existing checks, `&& enable_filter_passes::<F>(&state.filter_state, arch_ref, row)` (typed) + `&& dyn_enable_passes(&self.terms, arch_ref, row)` (dynamic). Resolves the column for the in-hand archetype + tests the row bit (mirrors `filter_fetch`; `Disabled` inverted). Returns `None` for a disabled entity. **`Changed`/`Added` are NOT applied here (C3-r7-c rustdoc note; compile-reject of `Enabled`+`Changed` means the confusing partial-filter cannot be constructed).** `single`/`single_mut` need NO such test (go through `iter`).

**`swap_remove_bit(removed,last)` (C2 + C4 ordering)** — per allocated column: **(1) READ `bit = test(last)` FIRST** (before any write); (2) `set(removed, bit)`; (3) `clear(last)`. Post-condition assert AT THE swap_remove_row site. On `Last` (pop): just `clear(last)` (O1-r7).

**Migration bit-copy (C4) — strict 3-phase sequenced, paged, per-column, borrow-free scratch (W3-r6)** — at each of the 4 helpers via `move_out_entity` (the bit op lives in `move_out_entity`, NOT the helper bodies — O1-r7):
- **Phase 1 (READ, before ANY swap):** `source.read_row_bits(source_row, &mut scratch)` — snapshot every allocated source column's bit at `source_row` into a stack `SmallList4<(ComponentId, bool)>`. **The scratch holds OWNED `(ComponentId, bool)` Copy values; it does NOT borrow `source` after the call returns** (W3-r6: `bool` is `Copy`, not `&[u8]` — structurally cannot be the NEW-1 dangling-slice class).
- **Phase 2 (WRITE target):** for each `(cid, bit)` in scratch: `target.write_row_bit(cid, new_row, bit, target_rows)` (`get_or_alloc_column`/`page` here bumps `note_column_alloc`+`enable_generation`).
- **Phase 3 (source swap-fix):** `source.swap_remove_row(source_row, source_last)` — interleaved at the SAME sequence point as the component-byte `swap_remove`, AFTER phase 1's read. Phase 1 already captured `source_row`'s bit, so the swap overwriting `source_row` with `source_last`'s bit is correct.

Ordering invariant: **phase 1 strictly precedes phase 3 and precedes the component-byte swap_remove; the phase-1 scratch is borrow-free** (D1 inv 6 + W3-r6). O(allocated_source_columns ≤4). Proptest oracle verifies bit survival under interleaved migrate/despawn/swap + cross-page; a Miri-TB test asserts the scratch outlives the source mutation.

**Archetype construction (signature filtering — cold)** — building a `ComponentMask`: `for id: if storage_kind(id)==Bitset { skip; no pool } else { mask.set(id) }`. One branch/id, cold. Bitset ids never fragment, never get a pool (the C1 premise — assert at every construction path).

## Multithreading model

- **Shared**: per-`(archetype,tag)` `EnablePage` words read concurrently by `par_iter` workers within one query — read-only (no writer live: v1 toggle is `&mut self` exclusive; Phase-9 never runs a `&mut`-world op concurrently with workers). Reads `AtomicU64::load(Relaxed)` = plain `mov`, TB-clean. `cull_enable_archetypes` + `EnablePresence.contains` run in `update` (single-threaded) — not on the worker hot path.
- **Synchronization (D8, explicit)**: NONE added on the hot path. v1 toggle is `&mut self`. The deferred path applies at the apply-window barrier where `running==0`; workers already joined via the pool's `pending.fetch_sub`/join edge (Phase-9.1) = the release/acquire edge. **`Relaxed` is sound ONLY because no other thread runs at toggle/apply time** (the `&mut`-exclusivity + join barrier), NOT because of acquire/release on the atomic. This is the precise point where worker-marking (D8) must add real Acquire/Release + a loom proof + the `EnableWrite` access category. **No Enable filter declares component access (C1): the no-op `init_access` is sound because a sibling data access on the bitset id is structurally impossible (D8).**
- **Data-race freedom (v1)**: v1 toggle requires `&mut EcsMaster`. Iteration reads via shared `&EcsMaster`/`UnsafeEcsCell` with no concurrent toggler. `AtomicU64` interior mutability is sound under TB exactly as Phase-10's `UnsafeCell<Tick>`. `enable_generation: AtomicU64` bumped only under `&mut self`; read in `update` (single-threaded) — Relaxed sufficient (W2: atomic is the forward seam, not a v1 concurrency requirement).
- **`Send`/`Sync`**: `EnablePage`/`EnableColumn`/`EnableStore` (`Box`/`AtomicU64`) → `Send + Sync` automatically; `Archetype` stays `Send + Sync`.

## Integration

**New modules**: `core/component/enable_store.rs`; `core/component/enable_presence.rs`; `core/iters/query/filter_enable.rs`; `core/ecs_master/enable_tag_api.rs`; `core/commands/enable_tag_commands.rs`.

**Modified**:
- `component_registry.rs` — `STORAGE_KIND`, `storage_kind`/`set_storage_kind`, `EnableTagId`, `register_enable_tag`, route registration.
- `archetype.rs` — `enable_store: EnableStore` field + NEW `Archetype` size const-assert tripwire, `enable_column_ptr`/`get_or_alloc_enable_column`, signature-build filters bitset ids + skips pools (assert the C1 premise), wire `swap_remove_row` into `remove_entity` (DROP) AND `move_out_entity` (no-drop) — fires once each, never the helper body (O1-r7); READ-first; post-condition assert at the swap site; `Last`/pop clears.
- `archetype_master.rs` — NEW `enable_generation: AtomicU64`, `bump_enable_generation`, `enable_generation()` accessor (W2 forward-seam note in doc-comment).
- All 4 migration helpers — call `move_out_entity` (which carries the bit op); the helper bodies do the phase-1 READ + phase-2 WRITE around the existing component-byte copy, but the source swap-fix bit op lives in `move_out_entity` (O1-r7). Paged 3-phase sequenced bit-copy with borrow-free scratch (C4 + W3-r6).
- `filter.rs` — NEW sealed `OrComposable` trait + impls; `impl_or_filter_tuple!` + >12 stub gain `$F: OrComposable` bound (M1); `Added`/`Changed::init_state` gain the D4 `const { assert!() }`; NEW `QueryFilter::{HAS_POSITIVE_ARCHETYPAL, CONTAINS_ENABLE_TERM, CONTAINS_CHANGE_DETECTION}` consts (additive defaults — C2/C3 enforcement; no ABI break). **`With`/`Without`/`Added`/`Changed` `Fetch`/`set_table_*`/`filter_fetch`/`matches_component_set`/`init_access` UNCHANGED.**
- `data.rs` — `HAS_DATA_COMPONENT` const on data leaves + tuple macro (true for any tuple with ≥1 real component; `()`=false). Additive; no ABI break.
- `query.rs` — **C2/C3 const-asserts at the `(D, F)` `QueryState`/`Query` construction funnel** (the single site both generics are in scope — Step 7a pins the exact `fn`); referenced as a per-monomorphization associated const so it is actually evaluated (the Phase-12.5 "const must be referenced" lesson).
- `state.rs` — NEW `cull_enable_archetypes` pass run after `post_filter_matched` ONLY when `has_enable_term` (over the BOUNDED matched set — C2); `update` gains the THIRD `enable_generation` check gated by `has_enable_term` (O2). `post_filter_matched` + `update_archetypes` themselves UNCHANGED (C2).
- `iter.rs`/`par_iter.rs` — NO structural change for typed `Enabled<T>` (inherited via `filter_fetch`); ADD dynamic `EnableTerms` per-row test behind `has_dynamic_enable_terms` (Phase-16 isolation).
- `query_view.rs` — EXPLICIT per-row enable test ONLY at `get` (:493) / `get_mut` (:549) + dynamic test + the C3-r7-c rustdoc note. `single`/`single_mut` UNCHANGED (inherit via `iter`). No `count`/`any` (do not exist).
- `boyko_macros/src/lib.rs` — parse `storage="bitset"` as a `LitStr` NameValue arm (W1-r6), reject unknown strings via `compile_error!`, emit `STORAGE_IS_BITSET`, route registration, suppress single-component Bundle for bitset (D6).
- `entity_commands.rs`/`commands.rs` — deferred `.enable`/`.disable` + POD commands.
- `entity_master.rs` `deallocate_entity` — NO new scan; assert ONLY `inland.is_null()` after nulling.
- `constants.rs` — `MAX_ENABLE_TERMS = 8`.

**Compatibility verified**: bitset indexes by `unit_index`; `ComponentLayout` stays 56 B; `term_list` untouched; Phase-9 executor + `FilteredAccessSet` untouched (Enable = no-op `init_access`, C1); `driver_ids` untouched; `matches_component_set` ABI unchanged; `update_archetypes`/`matched_ids` machinery unchanged (C2); `With`/`Without`/`Changed`/`single`/`single_mut` byte-identical.

## Implementation plan (for the developer)

**Every wave brief MUST start verbatim with**: *"MANDATORY graphify-first: run `graphify query/explain/path` to orient BEFORE reading any source; read raw files only after graphify has scoped the subgraph, or to modify/verify specific lines. Verify every line/offset against current source before editing — offsets in this plan are round-7 snapshots and may have drifted."*

1. **Step 1 (Wave 1, registry)** — `component_registry.rs`: `STORAGE_KIND`, `StorageKind`, `storage_kind`/`set_storage_kind` (write-once, debug-assert no-reclassify), `EnableTagId`+bridge, `register_enable_tag`. `constants.rs`: `MAX_ENABLE_TERMS`. Tests: kind round-trip, write-once, dynamic mint sets kind.
2. **Step 2 (Wave 1, storage — PAGED)** — `enable_store.rs`: `EnablePage` (512 B), `EnableColumn` (paged directory + `test`/`get_or_alloc_page`/`swap_remove_bit` READ-first), `EnableStore` (inline-4; `read_row_bits` writing borrow-free `(ComponentId,bool)` Copy — W3-r6 / `write_row_bit` / `swap_remove_row` C4 seam), `is_empty`. `SmallList4`. Tests + Miri-TB on atomic read/write + swap_remove_bit READ-first oracle + **read_row_bits scratch outlives source mutation (W3-r6)** + page-boundary toggle (rows 4095 vs 4096) + degenerate-large-archetype alloc ≤512 B/page.
3. **Step 3 (Wave 1, presence + enable_generation)** — `enable_presence.rs`: `EnablePresence` per-tag archetype bitset (`Box<[u64;16]>`, O(1) `contains`, `note_column_alloc`, lock-free epoch read — Phase-22.1 shape; **NO `for_each_present`/`present_count` — C2**). `archetype_master.rs`: `enable_generation: AtomicU64` + accessor + bump (W2 forward-seam doc). Tests: contains reflects alloc; epoch; enable_generation bumps once per column; lock-free read.
4. **Step 4 (Wave 2, archetype wiring)** — `archetype.rs`: `enable_store` field, ADD new `Archetype` size const-assert (tripwire), `enable_column_ptr`/`get_or_alloc_enable_column`, filter bitset ids out of the signature + skip pools (assert the C1 premise holds at every construction path), wire `swap_remove_row` into `remove_entity` AND `move_out_entity` (O1-r7 — once each, never the helper body; READ-first; `Last`/pop clears). Tests: bitset id never in any signature + never gets a pool (proptest); swap_remove preserves the swapped entity's bit READ-first (proptest oracle, Swapped branch); **RemoveOutcome::Last pop clears the popped bit (O1-r7 — NEW test)**; size pin holds.
5. **Step 5 (Wave 2, toggle API)** — `enable_tag_api.rs`: `enable`/`disable`/`is_enabled` (typed + `_id`), `&mut self`, no migration/hook/drain; `note_column_alloc` + `enable_generation` bump on first column; page alloc on first page touch. Tests: O(1), no archetype-count change, no STRUCTURAL gen bump, enable_generation bumps on first toggle only, no hook fire, O1 row-resolution, page-boundary toggle.
6. **Step 6 (Wave 3, migration bit-copy — C4 sequenced + W3-r6 borrow-free + O1-r7 single-fire)** — wire the 3-phase (READ-before-swap, borrow-free scratch) paged copy into all 4 helpers; the source swap-fix bit op lives in `move_out_entity` (O1-r7), the helper bodies do phase-1 READ + phase-2 WRITE. Tests: toggle then add unrelated component (insert-migration) → bit survives at target append; remove-migration → survives; attach/detach → survives; **source swap-fix preserves the swapped entity's source bit AND the migrating entity's bit was read BEFORE the swap (C4 — proptest oracle with interleaved swap)**; **bit op fires exactly once per migration (O1-r7 — assert via a counter probe, not double-counted)**; **read_row_bits scratch borrow-free across the swap (Miri-TB — W3-r6)**; alloc-on-migration bumps enable_generation; cross-page migration (source row in page 0, target append in page 1).
7. **Step 7 (Wave 3, filter family + Or seal)** — `filter_enable.rs`: `Enabled<T>` (non-archetypal Fetch; `_no_meta` real body — W4; `init_access` NO-OP with the ENBL-ACCESS-1 STRUCTURAL comment — C1-r7; `matches_component_set=true`; `CONTAINS_ENABLE_TERM=true`; NO `OrComposable` impl; paged `test`), `Disabled<T>` (inverted; same no-op `init_access`; same consts; NO `OrComposable` impl). `filter.rs`: NEW sealed `OrComposable` + impls; `$F: OrComposable` on both Or macros (M1); NEW `HAS_POSITIVE_ARCHETYPAL` (true for `With`, AND-tuples) + `CONTAINS_ENABLE_TERM` + `CONTAINS_CHANGE_DETECTION` (true for `Added`/`Changed`, tuples/Or OR-fold). `data.rs`: `HAS_DATA_COMPONENT` on leaves + tuple macro. Re-exports. Tests: golden if-const-matrix `(IS_ARCHETYPAL=false, NCD=false)`; `Query<&P, Enabled<A>>` yields only enabled rows; `Disabled<A>` AND-tuple correct; `Enabled`/`Disabled` in `Or` = COMPILE error (M1 trybuild); EXISTING `Or<(With<A>, Changed<B>)>` still compiles (M1 regression trybuild); `for_each_chunk`+`Enabled` = COMPILE error (trybuild).
   - **Step 7a (Wave 3, C2/C3 const-asserts at the `(D,F)` seam + cull pass + O2 update check)** — **PIN the exact `(D,F)` construction `fn` in `query.rs`/`state.rs`** (graphify `path "Query" "QueryState"` + source-verify both `D` and `F` are in scope and `matched_ids`/`filter_state` are built there). Add the C2 + C3 `const { assert!(...) }` as a referenced per-`(D,F)` associated const (`QueryState::<D,F>::ASSERT_SHAPE`, referenced from the construction path so it is actually evaluated — the Phase-12.5 lesson). `state.rs`: `cull_enable_archetypes` (BOUNDED walk over `matched_ids` — C2, no candidate-enumeration); `has_enable_term` per-state bool set at build; `update` THIRD `enable_generation` check gated by `has_enable_term` (O2). Tests: **`Query<(), Enabled<A>>` = COMPILE error (C2 trybuild — sole/data-less)**; **`Disabled<A>` sole = COMPILE error (C2 trybuild)**; **`Query<&P, (Changed<P>, Enabled<A>)>` = COMPILE error (C3-r7 trybuild)**; `Query<&P, Enabled<A>>` over a 200-archetype world (each containing P) culls to only present-for-A archetypes (count == archetypes-with-an-A-column); **O2 test: build+cache `Query<&P, Enabled<A>>`, iterate (empty), `enable(A)` in a not-yet-present archetype, iterate again → row visited**; no-enable query's update byte-identical (the `enable_generation` load skipped — 0%-gate bench); **trybuild-passes regression: a normal `Query<&P, With<Q>>` and `Query<&P, Changed<Q>>` still compile (the new const-asserts do not false-positive)**.
8. **Step 8 (Wave 4, change-detection rejection — D4)** — `filter.rs`: `Added`/`Changed::init_state` `const { assert!(!C::STORAGE_IS_BITSET) }` (filter-only-generic — names only `C`, compiles in `init_state`). Tests: trybuild compile-fail + doc example.
9. **Step 9 (Wave 4, deferred + dynamic terms + QueryView get/get_mut — C3-r5 + C3-r7-c)** — `enable_tag_commands.rs` POD + `entity_commands.rs` `.enable`/`.disable`; dynamic `with_enabled`/`without_enabled` per-row path + dynamic presence cull (Phase-16 `has_dynamic_enable_terms`) in `iter`/`par_iter`, with the runtime-bounded assert at `with_enabled` (C2 dynamic enforcement); **EXPLICIT per-row enable test ONLY at `query_view.rs` `get` (:493) + `get_mut` (:549) (typed and dynamic) + the C3-r7-c rustdoc note ("Changed/Added are NOT applied by point lookups"). `single`/`single_mut` UNCHANGED.** Tests: deferred toggle applies at window; **T-INTERLEAVE**; dynamic term filters per-row + presence-culls; **`Query<&P, Enabled<A>>::get(disabled)` and `::get_mut(disabled)` → `None` (C3 compile-but-lie guard); `single` over a world where the only matching entity is disabled → empty (confirms the inherited path filters it)**.
10. **Step 10 (Wave 5, macro — W1-r6)** — `boyko_macros`: parse `storage="bitset"` as a `LitStr` NameValue arm (NOT bare-key — W1), `compile_error!` on unknown storage strings, emit `STORAGE_IS_BITSET`, route registration, suppress Bundle emission. Tests: derived bitset → kind=Bitset, not in signature, not spawnable as bundle (compile-fail), spawn-then-`enable` works; **`#[component(storage="typo")]` = compile error naming the allowed value (W1)**.
11. **Step 11 (Wave 6, benches + docs)** — benches (§Metrics); book pages (`enable-tags.md`, update `storage-tradeoffs.md` incl. the C2 positive-term requirement + the C3-r7 no-mixing-with-change-detection rule + the **C4-r7 explicit "sole-flag cross-archetype enumeration / entity-disabling global scan NOT supported in v1" narrowing** + cull-cost asymmetry + the paging note + the D8 access contract); SYSTEMS/FEATURE_MAP entries. File BUG-ENABLE-PRE-1 (Changed-in-Or) and BUG-ENABLE-PRE-2 (QueryView get/get_mut + Changed) as separate issues (M3) — do NOT fix here.

**Wave dependency graph**: Wave 1 (Steps 1-3 parallel) → Wave 2 (Steps 4-5) → Wave 3 (Steps 6, 7, 7a — 7a depends on 7) → Wave 4 (Steps 8-9 parallel) → Wave 5 (Step 10) → Wave 6 (Step 11).

## Metrics and validation

**Benchmarks (0%-regression MANDATORY, named)**: `query_iter` (no EnableTag) byte-identical (±2%) — *the gate*; `query_iter_changed`, `query_iter_with_filter` (a `With`/`Without` query — flat since untouched), `par_iter`, `for_each_chunk`, `spawn`, `spawn_batch_10k` 0% regression; the warm-path `update` for a no-enable query is bench-checked (the `enable_generation` check must be skipped via `has_enable_term=false`). NEW `enable_toggle` (<5 ns warm vs `add_tag`); NEW `query_iter_enabled` (≤1.5 ns/row, `Query<&P, Enabled<A>>`); NEW `query_iter_enabled_culled` (200-archetype world all containing P, only K with an A-column → visited == K, the C2 bound); NEW `spawn_with_enable_tag` (= `spawn`); NEW `enable_toggle_large_archetype` (toggle into a >4096-row archetype — confirms the page alloc is ≤512 B).

**Unit/integration**: kind round-trip; bitset id never in any signature + never gets a pool; toggle no-migration/no-structural-gen/no-hook; `Enabled`/`Disabled` across all valid drivers (iter/iter_mut/par/get/get_mut/single/single_mut — C3 matrix); **C3 compile-but-lie guard (`get`/`get_mut` on disabled → None)**; **M1 Or-reject + existing-Or-still-compiles trybuild**; **C2 sole/data-less-Enabled reject trybuild + Disabled-sole reject trybuild + bounded-cull integration**; **C3-r7 Enabled+Changed mix reject trybuild + normal With/Changed-still-compile regression**; **O1-r7 RemoveOutcome::Last pop bit-clear + single-fire counter probe**; **O2 stale-presence-after-toggle test**; **C2-r5 swap_remove READ-first bit-correctness (proptest oracle)**; **C4 migration 3-phase sequenced + W3-r6 borrow-free bit-survival (insert+remove+attach+detach, interleaved-swap ordering + cross-page)**; T-INTERLEAVE; recycle no-leak; coexistence; `for_each_chunk`+`Enabled` compile error; `Added`/`Changed`+bitset compile error; `storage="typo"` compile error (W1); page-boundary toggle (rows 4095/4096).

**Miri-TB**: atomic read in `filter_fetch` + write in toggle + `swap_remove_bit` READ-first + the 3-phase migration copy + **`read_row_bits` scratch borrow-free across the source swap (W3-r6)** + `enable_generation` bump/read + page alloc/index (Phase-10 tick-column + Phase-11 migration suite). **proptest**: random toggle/migrate/despawn/swap (+ cross-page) vs a `HashMap<Entity, HashSet<TagId>>` oracle (C2-r5/C4 highest-risk; ordering-sensitive). **loom**: NOT needed v1; required ONLY when D8 worker-marking lands.

**debug_assert! invariants**: `storage_kind` no-reclassify; bitset id absent from signature AND has no pool at construction (C1 premise); `row < column_rows`; directory `len == ceil(rows/4096)`, present page covers 4096 rows; `swap_remove_bit` READ-first post-condition at the swap_remove_row site; bit op fires once per structural op (O1-r7); `EnablePresence.contains(id,A)` ⟺ A has a column (D1 inv 4); `enable_generation` bumped exactly once per column (D1 inv 5); migration READ precedes source swap + scratch borrow-free (D1 inv 6); `deallocate_entity` asserts ONLY `inland.is_null()`.

## Rejected remarks

- **C3-r7 option (a)** — REJECTED retrofitting `Changed`/`Added` into `get`/`get_mut` this phase. Justification: it widens scope into the change-detection point-lookup contract (needs tick-meta plumbing at the get site that today has none), risks the 0%-gate on the point-lookup path, and is an INDEPENDENT pre-existing bug (BUG-ENABLE-PRE-2) that predates this feature. Adopted (b)+(c) instead: compile-reject `Enabled`+`Changed` mixing (so the misleading partial-filter shape cannot be constructed) + a rustdoc note on the gap. This fully removes the confusion the critic identified without scope creep. The pre-existing `Changed`-ignored-by-get bug remains filed as an isolated wave.

All other round-6 CRITICAL and MAJOR remarks adopted:
- **C1 (init_access precedent inverted)** — verified `With`/`Added` declare a read, only `Without` is no-op; re-derived the no-op decision on the STRUCTURAL ground (no `ComponentPool` for a bitset id ⇒ sibling data access impossible ⇒ nothing to serialize ⇒ `Without`-analogous), with the precedent corrected to `Without<C>`.
- **C2 (const-assert cannot see D)** — verified `init_state` is filter-only-generic; relocated the combined-shape const-asserts to the `(D,F)` `QueryState`/`Query` construction seam as referenced per-monomorphization associated consts; new `CONTAINS_ENABLE_TERM`/`HAS_POSITIVE_ARCHETYPAL`/`HAS_DATA_COMPONENT` consts.
- **C4-r6 (silent narrowing)** — flagged explicitly in Goal + OUT-of-scope; audited the named v1 cases (all bounded); entity-disabling's global scan named as the one OUT-of-scope pattern + raised as an Open question for the brief owner.
- **O1-r7 (swap_remove fire site)** — pinned the bit op to `remove_entity` XOR `move_out_entity`, never the helper body (single-fire by construction); added the `RemoveOutcome::Last` pop test.
- **O2-r6 (five verified-good)** — all five preserved unchanged.

## Open questions

1. **Entity-disabling global scan (round-7 C4):** does any v1 use case need `Query<(), Disabled<A>>` / "every disabled entity regardless of components"? The named cases (`Selected`, `Stunned`, `OnGround`, pool reservation) do NOT — they all pair the flag with a positive data term. If entity-disabling's Bevy-`DefaultQueryFilters` global scan IS required in v1, the D7 candidate-seeded `update_archetypes` variant must move into scope (it is the only in-scope-able extension). **Decision needed from the brief owner before Wave 3.**

## OUT of scope (explicit)
- Worker-thread bit-marking without sync (D3/D8 deferred — `EnableWrite` category + loom/Miri-TB + real Acquire/Release; `AtomicU64` seam ready).
- `Added`/`Changed` on EnableTags (D4 compile-rejected; future `changed_tick` region/side-set).
- `for_each_chunk`/`par_for_each_chunk` with `Enabled`/`Disabled` (compile-rejected; AVX2 masked-SIMD/`vpcompressd` is a separate perf phase).
- `Enabled`/`Disabled` inside `Or<>` (v1 compile-rejected — M1).
- `Enabled`/`Disabled` combined with `Added`/`Changed` in one query (v1 compile-rejected — C3-r7).
- **Sole/data-less `Enabled<T>` (`Query<(), Enabled<A>>`) and `Disabled<T>` as a sole/only-archetype term (compile-rejected — unbounded; C2). Sole-flag cross-archetype enumeration / the entity-disabling global "every disabled entity" scan is NOT supported in v1 (round-7 C4); the D7 candidate-seeded `update_archetypes` variant is the in-scope-able extension.**
- `Disabled<T>` presence-cull (cannot — model the enabled state as the positive tag).
- **BUG-ENABLE-PRE-1** `Or<(Changed<A>, With<B>)>` row-leak (filed; isolated bugfix wave — M3).
- **BUG-ENABLE-PRE-2** `QueryView::get`/`get_mut` silently ignoring `Changed<C>`/`Added<C>` (filed; isolated bugfix wave; this phase adds only a rustdoc note + the compile-reject that prevents the misleading mix; `single` NOT affected).
- Per-page summary maintenance + inner-loop block-skip (v1.1 — paged layout makes it a drop-in).
- Entity-disabling `DefaultQueryFilters` auto-injection (Bevy 0.16) — separate phase.
- Spawning with an EnableTag bit pre-set via a Bundle (two-step spawn+enable is the v1 path).

---

### Round-5 → Round-6 changelog (retained)
C1 init_access no-op (precedent corrected in r7); C2 sole-Enabled compile-reject via positive-term requirement; W1 LitStr macro parse; W2 enable_generation AtomicU64 forward seam; W3 borrow-free read_row_bits; O1 four-decisions preserved.

### Round-4 → Round-5 changelog (retained)
C3-matrix corrected (count/any absent, single/single_mut inherit, get/get_mut explicit); EnableColumn paged (512 B page cap); C4 strict 3-phase READ-before-swap sequencing.

### Round-3 → Round-4 changelog (retained)
C1 dissolved (Or-reject); C2 With/Without rewrite deleted; C3 point-lookup explicit test (narrowed round 5); M1 Or seal; M2 sole-Enabled bound (superseded by round-6 compile-reject); M3 decouple; O1 ABI delete; O2 enable_generation.

### Round-2 → Round-3 changelog (retained)
C1-NEW presence-gate + ABI (superseded round 4); Disabled-in-Or reject; deallocate-assert relocated; size-assert tripwire.

### Round-1 → Round-2 changelog (retained)
C1 side presence-list; C2 swap_remove_bit fire-site; C3 apply-site table; C4 3-row/2-column migration copy; W1 Relaxed-single-threaded; W2 inline-4; W3 regrow-at-apply-window; W4 golden matrix; W5 const-assert rejection; O1 gen-before-row; O2 summary deferred; O3 size tripwire.

---

Key files (absolute): `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\filter.rs` (OrComposable seal + HAS_POSITIVE_ARCHETYPAL/CONTAINS_ENABLE_TERM/CONTAINS_CHANGE_DETECTION + D4 const-assert; init_access verified: With:330 reads, Without:431 no-op, Added:589 reads), `...\query\data.rs` (HAS_DATA_COMPONENT — C2), `...\query\query.rs` (C2/C3 const-asserts at the (D,F) construction seam — round-7), `...\query\state.rs` (cull_enable_archetypes over bounded matched set + O2 update check + has_enable_term build), `...\iters\query_state.rs` (verified C2: empty-include materializes full set — NOT modified), `...\query\iter.rs`, `...\query\query_view.rs` (explicit enable test at get :493 / get_mut :549 ONLY + C3-r7-c rustdoc note; single/single_mut inherit), `...\query\term_list.rs` (untouched), `...\component\component_registry.rs`, `...\component\enable_store.rs` (NEW, paged, borrow-free read_row_bits), `...\component\enable_presence.rs` (NEW, cull-oracle only), `...\iters\query\filter_enable.rs` (NEW, structural no-op init_access + CONTAINS_ENABLE_TERM), `...\ecs_master\tag_api.rs`, `...\ecs_master\enable_tag_api.rs` (NEW), `...\archetype\archetype.rs` (swap_remove bit in remove_entity XOR move_out_entity — O1-r7), `...\archetype\archetype_master.rs` (enable_generation AtomicU64 forward seam), `...\commands\migration_helpers.rs` (3-phase sequenced copy — C4 + W3-r6 + O1-r7 single-fire), `...\commands\enable_tag_commands.rs` (NEW), `...\entity\entity_master.rs`, `D:\claude\BoykoEngine\crates\boyko_macros\src\lib.rs` (LitStr storage parse — W1).

---

## Critic verdict

APPROVED (round 7). Convergence: R1(4C/5M) R2(1C/2M) R3(3C/3M) R4(1C) R5(2C/3M) R6(2C/2M) -> R7 APPROVED(0C/0M/3m).

Summary: Round-7 correctly fixes both prior CRITICALs (C1 inverted init_access precedent; C2 D-unnameable const-assert). All load-bearing file:line claims verified against source. No CRITICAL or MAJOR remains; resolutions are structurally sound and constructible. Three MINOR accuracy/coverage notes for the developer.

Three MINOR developer notes from the final round:

1. [MINOR] C2/C3 const-assert seam is mis-filed. The plan names query.rs ("QueryState::new or the Query type-construction site") for the (D,F)-generic const-asserts, but the actual sole funnel where BOTH D: QueryData and F: QueryFilter are in scope and filter_state/matched_ids are built is QueryDataState::<D,F>::new at state.rs:77-106 — NOT query.rs, and NOT QueryState::new (which is (include,exclude,optional)-generic, filter-agnostic). Step 7a's mandated graphify+source-verify-before-pin covers this, but the developer should pin state.rs:77, not query.rs.

2. [MINOR] Const-reject bypass risk (14b incomplete-enumeration class applied to const-asserts). The compile-reject of Query<(), Enabled<A>> only fires if EVERY construction path to a Query/QueryView with an Enabled F routes through QueryDataState::<D,F>::new (where the ASSERT_SHAPE associated const is referenced). Step 7a must verify new() is the SOLE funnel — if any test helper, direct QueryView mint, or alternate QueryDataState constructor exists that builds filter_state without referencing ASSERT_SHAPE, the reject is silently bypassed and the unbounded sole-Enabled scan reaches update_archetypes (verified at query_state.rs:195 to materialize all live archetypes on empty include). Add a grep-audit of all QueryDataState/Query construction sites to Step 7a, mirroring the per-monomorphization referenced-const discipline the plan already cites from Phase 12.5.

3. [MINOR] move_out_entity bit-op wiring needs the source enable_store reachable AND the READ/swap sequence-point made explicit per helper. Verified: all 4 helpers delegate the swap to move_out_entity (migration_helpers.rs:406/654/1003/1306) and the source swap is the LAST step (Step 5 at :406), so the O1-r7 single-fire claim and the C4 READ-before-swap ordering are both constructible. But the plan splits phase-1 READ (helper body) from phase-3 swap-fix (inside move_out_entity); the developer must ensure move_out_entity computes swap_remove_bit(source_row,last) for every allocated column on self at :612-613, at the SAME sequence point as swap_remove_unit_no_drop, and that remove_entity (the disjoint DROP path, archetype.rs:559-564) gets the identical wiring independently. The O1-r7 single-fire counter-probe test the plan mandates is the right guard; keep it.

