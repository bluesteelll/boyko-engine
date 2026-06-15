<!-- AMENDMENT to docs/ENABLE-TAG-PLAN.md (brief-owner directive 2026-06-15: global scan Query<(),Enabled/Disabled<A>> IN v1, was out-of-scope). Delta only; supersedes the round-7 C4 narrowing. APPROVED by architecture-critic round 2 (0 CRITICAL). Bounded by the EnablePresence candidate set (popcount), provably never a full-world scan. Wave 3 implements the main plan AS AMENDED here. -->

I now have the key facts verified:

1. **C1 (CRITICAL) confirmed**: `query.rs:495` is a second non-test `QueryDataState::<D,F>::new` caller inside `SystemParam::init_state`. Both it and `ecs_master.rs:2769` route through `new` (state.rs:77-106).
2. **C2 (CRITICAL)**: tag id is in `EnabledState<T>{ id: ComponentId }` (plan line 371), resolved at runtime in `init_state` (line 79). `sole_enable_tag()` must read `self.filter_state.id` — there is no type-level id.
3. The tuple filter `filter_fetch` (filter.rs:1059) folds elements; `Or` (filter.rs:1267) — the multi-term question depends on this fold, and `Or` is M1-rejected anyway.
4. The `update` warm-path (state.rs:200-214) gates `post_filter_matched` on a generation diff — the field to reuse exists in `QueryState` (`last_observed_archetype_generation`/`last_observed_structural_generation`).

I have enough to write the revised amendment.

---

# AMENDMENT to ENABLE-TAG-PLAN.md — Global-Scan for Sole `Enabled<A>` / `Disabled<A>` (round-8 REVISED, brief-owner directive)

Delta only. Supersedes the round-7 C4 narrowing. Every changed Decision/Step listed with new text; everything not listed is unchanged. Approved-round-7 resolutions C1, C3, M1, O1, O2, D4, D8 are PRESERVED verbatim — this amendment touches only the C2/C4/D2/D7 axis (the sole-enable shape) and adds a bounded enumeration primitive.

**Every future dev-wave brief referencing this amendment MUST start verbatim with**: *"MANDATORY graphify-first: run `graphify query/explain/path` to orient BEFORE reading any source; read raw files only after graphify has scoped the subgraph, or to modify/verify specific lines. Verify every line/offset against current source before editing — offsets here are round-8 snapshots and may have drifted."*

---

## A0 — CORRECTED funnel inventory (resolves CRITICAL C1)

The previous draft asserted "the only non-test `new` caller is `ecs_master.rs:2769`." **This was FALSE and is retracted.** Source-verified this session against live `ecs` HEAD:

`QueryDataState::<D, F>::new` is defined at **`state.rs:77-106`** (the sole `(D,F)` construction body — both `D: QueryData` and `F: QueryFilter` in scope, `data_state`/`filter_state` built at lines 78-79, `archetype_state` at 89, `update_archetypes` at 91, `post_filter_matched` at 92). It has **TWO non-test, non-doc callers**:

| Caller | Site | Context |
|---|---|---|
| `EcsMaster::get_or_init_query_state` | `ecs_master.rs:2769` | direct-API query path |
| `Query::<D,F> as SystemParam>::init_state` | `query.rs:495` | scheduled-system query path |

Plus test-only direct calls (e.g. `iter.rs:624/656/694/722/762/775`, throughout the query test modules). **All callers — both non-test and every test — route through the single `new` body at state.rs:77.** No alternate `QueryDataState` constructor exists; `QueryState::new` (state.rs:89) is `(include, exclude, optional)`-generic and filter-agnostic (it cannot name `F`, so it can neither host nor bypass the shape logic).

**Consequence for boundedness**: because the `if const { IS_CANDIDATE_SEEDED }` branch (A2) and the `ASSERT_SHAPE` reference (A3) both live INSIDE the `new` body, both non-test callers AND every test inherit them by construction. There is no path that builds `filter_state` without entering `new`. **Step 7a re-mandates a grep-audit of all `QueryDataState::` and `Query::` construction sites against live source before pinning** (critic round-7 MINOR-2, re-applied) — a future driver that minted a `QueryDataState` outside `new` would silently default to `update_archetypes` and a sole-enable query would hit the empty-include full scan (the M2 hazard). The audit is the guard; the dev wave MUST re-run it, not trust this snapshot.

---

## A1 — Candidate-seeded matched-set build (the mechanism)

### A1.1 `Disabled<A>` semantics (the rule, stated explicitly) — and the two-shape coherence resolution (resolves MAJOR M2)

**Boyko rule (chosen): `Disabled<A>` over an entity in an archetype with NO A-column = the entity counts as Disabled, but the SOLE-shape candidate set still does not enumerate no-column archetypes.** This requires care — the previous draft's "absent-column = EXCLUDED" created an incoherence the critic correctly flagged (a no-A-column row is reported `Disabled=true` by the in-scope `Query<&D, Disabled<A>>` via `filter_fetch` inversion, but would be silently dropped by `Query<(), Disabled<A>>`). The revised rule eliminates the divergence:

**Per-row semantics are IDENTICAL in both shapes (no behavior change to round-7):** `Disabled<A>::filter_fetch` returns `true` for no-column / no-page / bit-clear rows (round-7, unchanged — plan line 184). An entity is "disabled for A" iff its A-bit is not set, whether or not the column exists. This is preserved verbatim.

**The candidate set for the SOLE shape is `EnablePresence[A]` (present-A archetypes only) — and this is NOT a semantic divergence, because of the boyko coexistence model (D5).** Justification:

1. A SOLE `Query<(), Disabled<A>>` has an EMPTY include mask and NO positive data term. Without the candidate restriction, its only honest answer is "every entity in the world that lacks A-bit-set" = literally every live entity in every archetype where A is bit-clear = the full world (the M2 hazard).
2. **The boyko-coherent meaning of `Disabled<A>` as a SOLE term is "A is a property of this entity AND it is off."** D5 (plan line 255) establishes that an EnableTag column's existence in an archetype is the structural record that A was ever made a property of entities in that archetype. So an archetype with NO A-column = "A was never a property here." A SOLE `Disabled<A>` therefore means "enumerate entities for which A is a property and currently off" ⇒ present-A archetypes, bit-clear rows. **This is exactly Bevy's `Disabled` ("has the disabling component and it is set") — the column IS our "has the component" predicate.**
3. **Coherence with the positive-term shape is preserved because the two shapes ask DIFFERENT questions, and the difference is now EXPLICIT and predictable, not silent:**
   - `Query<&D, Disabled<A>>` asks "of the entities with component D, which are not-A-enabled" — the `&D` term already bounds the candidate set to D-archetypes; within those, a no-A-column D-archetype contributes all its rows as `Disabled=true` (round-7, unchanged). The user explicitly named `D`, so "all D-entities without A on" is the intended answer.
   - `Query<(), Disabled<A>>` asks "which entities have A as a property and off" — there is NO positive term to bound it, so the only well-defined bounded answer is the present-A set.
   - **These are not contradictory answers to the same question; they are answers to two different questions, distinguished by whether the user supplied a positive term.** A no-A-column entity in a D-archetype: visible in `Query<&D, Disabled<A>>` (the user asked for D-entities), invisible in `Query<(), Disabled<A>>` (the user asked for A-property entities). This is documented as a deliberate, predictable rule, NOT asserted as "identical semantics."

**MANDATORY test of BOTH behaviors (resolves M2's required-action (a)):** a single integration test constructs one no-A-column archetype holding entity `e` with component `D`, and asserts:
- `Query<&D, Disabled<A>>` VISITS `e` (round-7 `filter_fetch` inversion — no-column ⇒ Disabled=true), AND
- `Query<(), Disabled<A>>` does NOT visit `e` (candidate set = present-A only; the no-A-column archetype is not a candidate).
The test's doc-comment states the rule: *"A positive-term `Disabled<A>` reports no-A-column rows as disabled (the user named the positive set); a sole `Disabled<A>` enumerates only entities for which A is a property (present-A archetypes). The shapes answer different questions."*

This is recorded in `storage-tradeoffs.md` as the **two-shape `Disabled` rule**, replacing the previous draft's incorrect "identical semantics" claim.

### A1.2 Candidate set (both filters)

| Sole filter | Candidate archetype set | Per-row test within candidate (UNCHANGED from round-7 `filter_fetch`) |
|---|---|---|
| `Enabled<A>` | `EnablePresence[A]` | bit set ⇒ keep row |
| `Disabled<A>` | `EnablePresence[A]` (SAME set — A1.1) | bit clear / no-page-within-present-column ⇒ keep row |

Both seed from the identical bounded candidate list. The per-row `filter_fetch` polarity is the only difference, and it is byte-identical to round-7. `Disabled` gets a candidate set but **still no per-archetype cull** (every candidate archetype may hold both enabled and disabled rows) — consistent with round-7 "Disabled does not presence-cull."

### A1.3 The build (`seed_from_candidates` — replaces the `1..gen` scan for this shape ONLY)

New `QueryState` method, additive. The inner `QueryState` is filter-agnostic, so this method takes the candidate bitset as a parameter (computed by the `(D,F)`-generic `QueryDataState::new`, which can read `self.filter_state.id`):

```rust
// query_state.rs — NEW, additive. Driven ONLY from QueryDataState::new/update
// under `if const { IS_CANDIDATE_SEEDED }` (A2). `candidates` is an owned
// ArchetypeBitSet snapshot (A1.4) the caller already computed.
pub(crate) fn seed_from_candidates(&mut self, candidates: &ArchetypeBitSet, master: &ArchetypeMaster) {
    // MIRRORS the structural branch of update_archetypes (query_state.rs:174-184):
    //   - on a structural-generation change, FULL clear (drop the dual structure)
    //     then re-push from candidates  — this is where ABA-recycled ids are purged
    //     (see A1.5 for the exact ABA argument);
    //   - otherwise delta-add only the newly-present candidate ids.
    // The ONLY difference vs update_archetypes: iterate `candidates` (a 16-word
    // bitset, popcount-bounded trailing_zeros walk) instead of `1..current_gen`,
    // and NEVER call self.matches(mask) (the include mask is empty by construction;
    // the candidate bitset IS the membership predicate).
    // For each set bit `id` in candidates:
    //   if master.get_archetype(id).is_some()      // liveness intersection
    //      && !self.matched_archetypes.contains(id) // dedup
    //   { self.push_matched(id); }                  // reuses query_state.rs:288
    // Stamp last_observed_archetype_generation + last_observed_structural_generation.
}
```

Key properties:
- Reuses `push_matched` (query_state.rs:288) ⇒ the QS1 dual-structure invariant (`matched_ids` ↔ `matched_archetypes` bitset, the `assert_dual_invariant` at state.rs:166) is preserved by construction.
- Bounded to `candidates.popcount()` set bits over a 16-word walk, NEVER `current_gen`. **No `1..gen` path is reachable for this shape.** This is the M2/C2 structural resolution (critic round-1 O1 confirmed the mechanism).
- Stale candidate ids whose archetype was removed: `master.get_archetype(id)` returns `None` ⇒ skipped — identical to `update_archetypes` (query_state.rs:197) and the iter-time `get_archetype` guard.

### A1.4 `EnablePresence` bounded snapshot accessor + the concurrency reconciliation (resolves MAJOR M1)

Round-7 deliberately omitted `for_each_present`/`present_count`. The amendment adds ONE bounded accessor returning an owned snapshot:

```rust
// enable_presence.rs — NEW. Bounded snapshot COPY (16 words = 128 B memcpy).
pub(crate) fn snapshot_present(&self, tag: ComponentId) -> ArchetypeBitSet; // empty if no presence bitset yet
```

**Concurrency model — reconciled with round-7 D1 (M1 resolution): `snapshot_present` is a PLAIN memcpy under the SAME single-threaded `&mut self` / apply-window discipline as the rest of v1. The previous draft's "lock-free epoch read / retry-on-epoch-change (Phase-22.1 shape)" framing is DROPPED — it was dead complexity that contradicted D1's `Relaxed`-soundness premise.** Justification:

- Round-7 D1 (plan line 95) and D8 (line 451) establish that in v1 the presence machinery is bumped under `&mut EcsMaster` only and read in `update()` single-threaded; `Relaxed` is sound *precisely because no concurrent writer exists*. `seed_from_candidates`/`snapshot_present` run inside `QueryDataState::new`/`update`, which A5.3 (and round-7's multithreading model) confirm are single-threaded — `new` runs at system registration / direct-API call (`&mut EcsMaster`), `update` runs at the apply-window barrier where `running==0` (no live worker). There is no concurrent writer to race; therefore there is nothing to retry against.
- `snapshot_present` is a 128 B `Box<[u64;16]>` → `ArchetypeBitSet` copy. If the tag has no presence bitset yet (never toggled), it returns an empty `ArchetypeBitSet` (zero candidates ⇒ empty result — correct: a never-toggled tag has no enabled/disabled rows).
- **No new concurrency-soundness obligation, no loom, no epoch protocol.** This matches round-7's deliberate avoidance of v1 concurrency machinery. The D7 worker-marking seam (when it lands) is where the epoch/Acquire-Release protocol would be added — out of v1, unchanged.
- `present_count` is NOT a separate method; the caller uses `snapshot_present(tag).popcount()` where needed (it is bounded by construction). `for_each_present` is still NOT provided (the snapshot + caller-side bitset walk replaces it). `EnablePresence` is still NEVER a live-entity driver — it hands back an *archetype* bitset that `seed_from_candidates` intersects with liveness.

The round-7 D1 epoch field on `EnablePresence` (plan line 335) STAYS as the D7 forward seam but is NOT read by v1 `snapshot_present` (which is a plain copy). This is documented so a future worker-marking phase knows the seam exists.

### A1.5 ArchetypeId-ABA preservation in `seed_from_candidates` (resolves part of MAJOR M3)

The critic's M3 flags that a recycled archetype id (clear()+create reusing an id) must not leak into a candidate-seeded result. The argument, made explicit:

- `seed_from_candidates`'s FULL-clear branch fires on a **structural-generation** mismatch — the same trigger as `update_archetypes`'s load-bearing ABA rebuild (query_state.rs:174-180). Archetype removal/recycle bumps `structural_generation` (round-7 D1 sub-decision, plan line 95: a structural bump force-rebuilds every cache). So a recycled id is purged on the structural-mismatch full-clear, before any re-push — identical to the existing `update_archetypes` ABA fix. The candidate path inherits the ABA correctness, it does not re-implement it.
- The candidate bitset itself is purely archetype-id-indexed; a recycled id whose new archetype has an A-column will have its bit set in `EnablePresence[A]` by `note_column_alloc` (the new archetype's first A-toggle), and `get_archetype(id)` returns the LIVE new archetype — so a recycled id is correctly re-evaluated, never stale. A recycled id whose new archetype has NO A-column will NOT be in `EnablePresence[A]` (its bit was cleared — see A4.4) ⇒ not a candidate ⇒ correctly absent.
- **The previous draft's `update_archetypes_struct_only` helper is REMOVED** (the open-question alternative is adopted — see A4.3): on a structural bump, the candidate path performs the FULL clear inside `seed_from_candidates` (re-pushing from the snapshot, popcount-bounded), so it NEVER touches the `1..gen` loop even on removal. The struct-only helper was under-specified (M3) and unnecessary: `seed_from_candidates` already does the clear+re-push, and re-pushing from a popcount-bounded snapshot is strictly bounded — there is no scenario where the candidate path runs a `1..gen` scan.

---

## A2 — Per-row scan within candidates + the 0%-gate + the resolver (resolves CRITICAL C2)

### A2.1 `IS_CANDIDATE_SEEDED` definition, evaluation discipline, and the tag-id resolver

**The resolver (C2's core gap): the tag id is read at RUNTIME from `self.filter_state`, NOT from a non-existent type-level const.** Verified: `EnabledState<T>{ id: ComponentId }` (plan line 371) is populated in `Enabled<T>::init_state` (the runtime resolve, plan line 382; mirrors With:325). There is no type-level `ComponentId` const — the id is not known until `init_state` runs against the live registry. Therefore:

```rust
// state.rs — on QueryDataState<D, F> (both generics in scope, mirroring the
// existing ASSERT_SHAPE associated-const discipline the round-7 plan mandates).
impl<D: QueryData, F: QueryFilter> QueryDataState<D, F> {
    // Compile-time SHAPE classification (the const). Force-evaluated by being
    // REFERENCED in the `new` body (A2.2) — the Phase-12.5 "const must be a
    // referenced associated const, not a free const fn body" discipline.
    const IS_CANDIDATE_SEEDED: bool =
        F::CONTAINS_ENABLE_TERM && !D::HAS_DATA_COMPONENT && !F::HAS_POSITIVE_ARCHETYPAL;

    // RUNTIME resolver — reads the already-built filter_state, NOT a type const.
    // For a sole single Enabled<A>/Disabled<A>, F::State is EnabledState<A> (or
    // DisabledState<A>), whose `.id` field holds the resolved ComponentId.
    // A new QueryFilter method `sole_enable_tag_id(state) -> ComponentId` is added
    // (default unreachable!() backstop; overridden ONLY by Enabled/Disabled leaves
    // to return state.id). It is called ONLY under `if const { IS_CANDIDATE_SEEDED }`,
    // so for any non-sole-enable F the unreachable backstop is never emitted/reached.
    #[inline] fn sole_enable_tag(&self) -> ComponentId {
        <F as QueryFilter>::sole_enable_tag_id(&self.filter_state)
    }
}
```

New `QueryFilter` trait method (additive, default backstop — no ABI break to existing leaves):
```rust
trait QueryFilter {
    // ... round-7 additive consts (CONTAINS_ENABLE_TERM etc.) ...
    /// Returns the tag id of a SOLE enable term. Default = unreachable backstop;
    /// overridden by Enabled<T>/Disabled<T> to return `state.id`. Called ONLY when
    /// IS_CANDIDATE_SEEDED (a sole single enable term) — never for tuples/others.
    #[inline] fn sole_enable_tag_id(_state: &Self::State) -> ComponentId { unreachable!() }
}
```

**Evaluation proof (C2's "pin where it is force-evaluated"):** `IS_CANDIDATE_SEEDED` is referenced in the `new` body via `if const { Self::IS_CANDIDATE_SEEDED } { ... }` (A2.2). A `const`-block condition forces evaluation of the const per `(D,F)` monomorphization (this is the *evaluation* mechanism — distinct from the round-7 `ASSERT_SHAPE` const-assert item, which is a separate referenced const for the C3 reject; both are referenced from `new`, both force-evaluate). **Mandatory behavioral test (C2 required-action):** `Query<(), Enabled<A>>` not only COMPILES but provably takes the seeded arm — a test instruments an archetype-touch counter (A6 test (a)) and asserts the walk visited == K present archetypes, NOT N total; if `IS_CANDIDATE_SEEDED` mis-folded to `false`, the query would either hit the empty-include full scan (counter == N, test fails) or fail to compile (the C2 reject would still fire). The behavioral counter is the oracle, not "it compiles."

### A2.2 The branch in `new` and `update` (the 0%-gate)

In `QueryDataState::new` (state.rs:89-97) and `update` (state.rs:200-214):

```rust
// new (replaces lines 89-97):
let mut archetype_state = QueryState::new(include, exclude, optional);
if const { Self::IS_CANDIDATE_SEEDED } {        // const-folded — zero runtime cost for non-enable queries
    let candidates = world.enable_presence().snapshot_present(  // 128 B copy, single-threaded (A1.4)
        <F as QueryFilter>::sole_enable_tag_id(&filter_state),
    );
    archetype_state.seed_from_candidates(&candidates, world.archetype_master());
    // NO update_archetypes (the 1..gen scan), NO post_filter_matched (empty include ⇒ nothing to trim).
} else {
    archetype_state.update_archetypes(world.archetype_master());   // UNCHANGED (state.rs:91)
    Self::post_filter_matched(&mut archetype_state, &data_state, &filter_state, world.archetype_master()); // UNCHANGED
}
```

`IS_CANDIDATE_SEEDED` is a `const` associated item, so for any non-enable `(D,F)` the entire `if const` collapses to the existing `else` arm at monomorphization — the candidate code, `seed_from_candidates`, `snapshot_present`, and `sole_enable_tag_id` are NEVER emitted into that monomorphization. **The no-enable path is byte-identical to today (0%-gate).** The branch resolves at compile time per `(D,F)`, never at runtime.

### A2.3 Per-row scan within each candidate archetype

UNCHANGED from round-7. Once `matched_ids` holds the candidate archetypes, iteration drives through the existing `filter_fetch` per-row path gated by `if !const { F::IS_ARCHETYPAL }` (iter.rs ~189-301; par_iter unchanged). Each candidate archetype: hoist the page word per 64-row block, test the bit; `Enabled` keeps the row if set, `Disabled` if clear. **For a sole-enable query D=() ⇒ the data tuple is empty ⇒ iteration yields entities** — exactly the entity-enumeration the brief wants. No new per-row code; candidate seeding feeds the existing per-row machinery.

### A2.4 0%-gate proof

- No enable term: `F::CONTAINS_ENABLE_TERM == false` ⇒ `IS_CANDIDATE_SEEDED == false` ⇒ const-folds to the existing `update_archetypes`+`post_filter_matched` arm. Byte-identical — verifiable by the existing `query_iter` bench (the gate) and asm diff.
- POSITIVE term + enable term (round-7 in-scope `Query<&D, Enabled<A>>`): `HAS_DATA_COMPONENT == true` (or `HAS_POSITIVE_ARCHETYPAL == true` for `With`) ⇒ `IS_CANDIDATE_SEEDED == false` ⇒ existing `update_archetypes`+`cull_enable_archetypes` path UNCHANGED from round-7. The positive-term path is unaffected.
- Only the sole-enable shape reaches `seed_from_candidates`.

---

## A3 — Const-assert changes (lift C2 for the supported shape; keep all other rejects) (resolves MAJOR M4)

### A3.1 Scope DECISION: v1 admits the SOLE-SINGLE shape only; AND-of-N enable terms is DEFERRED

The previous draft expanded scope to multi-term AND (`Query<(), (Enabled<A>, Enabled<B>)>`) and claimed "no residual unbounded shape exists, drop `_C2` entirely." **The critic (M4) correctly flagged this as an under-specified scope expansion with an unverified type-level multi-tag resolver and an unhandled mixed-polarity case. REVISED: the amendment admits ONLY the brief's literal directive — a SOLE SINGLE `Enabled<A>` or `Disabled<A>` over `D=()`.** AND-of-N (same- or mixed-polarity) is explicitly DEFERRED. Rationale:

- The brief's directive is literally `Query<(), Enabled<A>>` / `Query<(), Disabled<A>>` — single tag. That is what the candidate machinery cleanly supports: `IS_CANDIDATE_SEEDED` requires `F::CONTAINS_ENABLE_TERM && !HAS_DATA && !HAS_POSITIVE_ARCHETYPAL`, and the resolver `sole_enable_tag_id` returns ONE id from a leaf `EnabledState<A>`/`DisabledState<A>` — there is no type-level mechanism for a tuple `F` to expose N ids without new machinery (the same resolver gap the critic names). A tuple `F = (Enabled<A>, Enabled<B>)` has `F::State = (EnabledState<A>, EnabledState<B>)`; extracting both ids requires a tuple-walking accessor that does not exist and is out of the brief's literal ask.
- **Therefore `_C2` is NOT dropped — it is NARROWED**: the sole-single shape is carved out as allowed; a tuple-of-enable-terms with no positive term remains REJECTED (still potentially unbounded under our v1 machinery, since we have no resolver to bound it). This keeps the guard for genuinely-unsupported shapes, exactly as M4 demands ("do not ship a blanket no-residual-unbounded claim that removes `_C2` entirely").

### A3.2 `_C2` is NARROWED (the new const-assert at the state.rs:77 seam)

```rust
// state.rs — referenced per-(D,F) associated const ASSERT_SHAPE (Phase-12.5
// discipline; referenced from `new` so it force-evaluates).
const _C2: () = assert!(
    // ALLOWED:
    //   (a) positive-term-bounded enable query (round-7): HAS_DATA || HAS_POSITIVE_ARCHETYPAL
    //   (b) NEW: a SOLE SINGLE enable term over D=() (candidate-seeded, bounded by
    //       EnablePresence[A]) — IS_CANDIDATE_SEEDED captures exactly this shape.
    // REJECTED (still unbounded under v1 machinery — no multi-tag resolver):
    //   a tuple/composite of enable terms with NO positive term and NOT a single leaf.
    !F::CONTAINS_ENABLE_TERM
        || D::HAS_DATA_COMPONENT
        || F::HAS_POSITIVE_ARCHETYPAL
        || F::IS_SOLE_SINGLE_ENABLE,   // NEW const (A3.3) — true ONLY for a single Enabled/Disabled leaf
    "`Enabled<T>`/`Disabled<T>` with no positive term is supported ONLY as a SINGLE \
     sole term (`Query<(), Enabled<A>>`). A tuple of enable terms without a positive \
     archetypal term is not bounded in v1 — add `With<_>`/a data component, or split \
     into separate single-term queries."
);
```

`IS_CANDIDATE_SEEDED = F::CONTAINS_ENABLE_TERM && !HAS_DATA && !HAS_POSITIVE_ARCHETYPAL`. Note the carve-out term in `_C2` is `F::IS_SOLE_SINGLE_ENABLE` (A3.3), which is `true` for a single `Enabled`/`Disabled` leaf and `false` for any tuple — this is what distinguishes the allowed single-leaf shape from a rejected enable-tuple. `IS_CANDIDATE_SEEDED` for a rejected enable-tuple would be `true` by its formula, but the query never compiles (the `_C2` assert fails first), so the seeded path is never reached for an unsupported shape — and as a defense-in-depth backstop, `sole_enable_tag_id`'s default `unreachable!()` would fire if a tuple ever reached the resolver. To make the two consistent, **`IS_CANDIDATE_SEEDED` is tightened to require `F::IS_SOLE_SINGLE_ENABLE`** (so it is true iff the shape is both seedable AND a single leaf):

```rust
const IS_CANDIDATE_SEEDED: bool =
    F::IS_SOLE_SINGLE_ENABLE && !D::HAS_DATA_COMPONENT && !F::HAS_POSITIVE_ARCHETYPAL;
```

### A3.3 New const `IS_SOLE_SINGLE_ENABLE` (additive default — no ABI break)

```rust
trait QueryFilter {
    // round-7 additive consts unchanged ...
    /// true ONLY for a single Enabled<T>/Disabled<T> leaf; false for () / With /
    /// Without / Added / Changed / ALL tuples / Or. Tuple+Or macros DO NOT
    /// override it (default false) — so a tuple of enable terms is NOT a single leaf.
    const IS_SOLE_SINGLE_ENABLE: bool = false;
}
// Enabled<T> / Disabled<T>: const IS_SOLE_SINGLE_ENABLE = true;
```

Default `false`; overridden `true` ONLY on the two enable leaves. Tuple/Or macros leave it default `false` — so `(Enabled<A>, Enabled<B>)` has `IS_SOLE_SINGLE_ENABLE = false` ⇒ `IS_CANDIDATE_SEEDED = false` ⇒ falls through `_C2` to the reject (it has `CONTAINS_ENABLE_TERM=true`, no positive term, not a single leaf). Genuinely-bounded, type-level-resolvable single-leaf shapes are allowed; everything else stays rejected. No type-level multi-tag resolver is introduced (M4 resolved by deferral, not by under-specified machinery).

### A3.4 `_C3` (Enabled+Changed mix) — KEPT VERBATIM

`const { assert!(!(F::CONTAINS_ENABLE_TERM && F::CONTAINS_CHANGE_DETECTION)) }` stays. The candidate-seeded path additionally CANNOT carry a `Changed`/`Added` term (it would be `IS_SOLE_SINGLE_ENABLE && CONTAINS_CHANGE_DETECTION` — but a single enable leaf has `CONTAINS_CHANGE_DETECTION=false`, and any tuple mixing them is not a single leaf and is `_C3`-rejected anyway). C3 transitively guards the new path.

### A3.5 M1 (`Or` reject) — KEPT VERBATIM

`Enabled`/`Disabled` still do NOT impl `OrComposable`. `IS_SOLE_SINGLE_ENABLE` is a leaf-only const; an `Or` containing an enable term is a compile error before any shape classification. Unaffected.

### A3.6 The const-assert SEAM corrected to state.rs:77 (critic round-7 MINOR-1 + MINOR-2, re-applied)

The `ASSERT_SHAPE`/`_C2`/`_C3`/`IS_CANDIDATE_SEEDED` consts live on `QueryDataState<D,F>` at **state.rs:77** (the sole `(D,F)` funnel — A0), NOT `query.rs`, NOT `QueryState::new` (filter-agnostic). Both non-test callers (`ecs_master.rs:2769`, `query.rs:495`) reference them by entering `new`. Step 7a re-runs the grep-audit (A0).

---

## A4 — `enable_generation` / O2 invalidation for the candidate-seeded path (resolves MAJOR M3 reuse claim)

### A4.1 The first-toggle-into-a-new-archetype problem

Cached `Query<(), Enabled<A>>`: at build, archetype X has no A-column ⇒ not in `EnablePresence[A]` ⇒ not a candidate ⇒ not in `matched_ids`. Later `enable::<A>(e in X)` allocates X's A-column ⇒ `note_column_alloc(A, X)` sets `EnablePresence[A]` bit X + bumps world `enable_generation` (round-7 D1 step 3 / inv 5, unchanged). The next `iter()` MUST now see X.

### A4.2 The trigger — and the honest field-reuse story (M3)

**The previous draft claimed it could reuse "the round-7 O2 `enable_generation` snapshot field" with no new struct growth. The critic (M3) correctly noted that the round-7 field lives in the POSITIVE-TERM cull monomorphizations, which are a DISJOINT const-set from the candidate-seeded monomorphizations. REVISED — honest accounting:**

- Round-7 O2 added a per-`QueryDataState` field `last_observed_enable_generation: u64` (8 B), read/written under `has_enable_term` gating. The candidate-seeded path and the positive-term-cull path are both `has_enable_term` shapes (both have `CONTAINS_ENABLE_TERM=true`); they are const-DISJOINT (one has `IS_CANDIDATE_SEEDED`, the other has a positive term), but they share the SAME struct field SLOT — the field exists in every `CONTAINS_ENABLE_TERM` monomorphization. "Reuse" here means: **the same struct slot serves both roles; it is WRITTEN on whichever path the monomorphization takes.** This is sound for layout (one field, two const-disjoint write sites) and adds NO new field beyond round-7's. For a non-enable `(D,F)` the field is still present in the struct but never touched (round-7 already established this; the field is behind the same `CONTAINS_ENABLE_TERM`-gated codegen). **No new struct growth vs round-7.**

- The WRITE site on the candidate path is proven reached: it is inside the `if const { IS_CANDIDATE_SEEDED }` branch of `update` (below), which IS emitted for candidate-seeded monomorphizations.

In `QueryDataState::update` (state.rs:200), the candidate-seeded path is selected by the same const:

```rust
pub fn update(&mut self, master: &ArchetypeMaster) {
    if const { Self::IS_CANDIDATE_SEEDED } {
        let pre_enable_gen = self.last_observed_enable_generation;   // round-7 field, reused slot
        let cur_enable_gen = master.enable_generation();            // round-7 AtomicU64 (W2)
        let pre_struct     = self.archetype_state.last_observed_structural_generation();
        if pre_enable_gen != cur_enable_gen || pre_struct != master.structural_generation() {
            let candidates = master.enable_presence().snapshot_present(self.sole_enable_tag());
            self.archetype_state.seed_from_candidates(&candidates, master); // does its own struct-mismatch FULL clear (A1.5)
            self.last_observed_enable_generation = cur_enable_gen;
        }
        return;
    }
    // ── existing non-candidate path UNCHANGED (state.rs:201-214) ──
    let pre_gen = self.archetype_state.last_observed_archetype_generation();
    let pre_struct = self.archetype_state.last_observed_structural_generation();
    self.archetype_state.update_archetypes(master);
    if pre_gen != master.archetype_generation() || pre_struct != master.structural_generation() {
        Self::post_filter_matched(&mut self.archetype_state, &self.data_state, &self.filter_state, master);
    }
}
```

### A4.3 Why `enable_generation` is the load-bearing invalidator + why NO `update_archetypes_struct_only` helper (M3 + adopting the previous open question)

- **`enable_generation` bumps ONLY on column allocation** (round-7 D1 sub-decision — first toggle into an archetype, NOT every toggle, plan line 95 / inv 5). So:
  - First-toggle-into-a-NEW-archetype: bumps `enable_generation` ⇒ candidate set grew ⇒ re-seed ⇒ new archetype visited. **A4 requirement met.**
  - Steady-state toggle (column already exists): does NOT bump `enable_generation` ⇒ no re-seed ⇒ candidate set unchanged (X is already a candidate); the per-row `filter_fetch` picks up the new bit value live on the next iter. Correct and cheap.
- **Structural-generation bump (archetype removal/recycle)**: triggers the re-seed, and `seed_from_candidates` performs its own FULL clear + re-push from the popcount-bounded snapshot (A1.5) — purging recycled/removed ids exactly as `update_archetypes`'s ABA rebuild does, WITHOUT ever entering a `1..gen` loop. **The previous draft's `update_archetypes_struct_only` helper is DELETED** (it was under-specified per M3 and the open question flagged it). `seed_from_candidates` is the single, fully-specified rebuild primitive for this shape; it is strictly popcount-bounded on every path (delta-add OR full-clear-and-rebuild). **There is no path on which the candidate-seeded query touches a non-presence-bounded loop** — resolving the previous draft's lone open question by adoption, not deferral.

### A4.4 `EnablePresence` bit-clear on archetype removal (completeness for A1.5)

For the ABA argument (A1.5) to hold, a removed archetype's `EnablePresence` bit must not falsely persist. Round-7 `EnablePresence` records column existence; when an archetype is removed (its `EnableStore` dropped), its presence bit for every tag it held must be cleared. **This is an additive obligation on the archetype-removal path** (the same site that bumps `structural_generation`): `EnablePresence.clear_archetype(arch_id)` (clears the arch's bit across all its tags). Without it, a recycled id could falsely appear as a candidate for a tag the NEW archetype lacks — but `seed_from_candidates`'s `get_archetype(id)` liveness check + the structural full-clear would still drop it (the new archetype's mask wouldn't match... except this shape has no mask). To be safe, the bit-clear is REQUIRED. This is a small new method on `EnablePresence` + one call at the archetype-removal site:

```rust
// enable_presence.rs — additive
pub(crate) fn clear_archetype(&self, arch: ArchetypeId);  // clears arch's bit across all tags; called on archetype removal
```

Note: archetype removal is already a `structural_generation` bump (rare), so the clear runs only on removal, not on the hot path. Added to Step 3 + the archetype-removal wiring (Step 4).

### A4.5 O2 test extension

Round-7 O2 test (positive-term path) preserved. NEW test (A6 (c)): build+cache `Query<(), Enabled<A>>` over a world where archetype X has no A-column; iterate (X absent); `enable::<A>(e in X)`; iterate again ⇒ X's enabled row visited. Proves the candidate path's `enable_generation` invalidation. PLUS a removal test: build+cache, remove an archetype that was a candidate, iterate ⇒ removed archetype's rows absent, no panic (A1.5 / A4.4).

---

## A5 — Soundness / 0%-gate / unsafe analysis (resolves nothing new flagged; confirms A5 brief requirement)

### A5.1 No new unsafe

`seed_from_candidates` is safe Rust (bitset trailing-zeros walk + `push_matched` + `get_archetype`). `snapshot_present` is a 128 B memcpy of a `Box<[u64;16]>` under the single-threaded `&mut self`/apply-window discipline (A1.4 — plain copy, no epoch protocol, no unsafe). `clear_archetype` is safe (bit clears under the same discipline). The per-row bit test is the round-7 `filter_fetch` `AtomicU64::load(Relaxed)` — already designed, already covered by the round-7 Miri-TB suite. **No new unsafe beyond the per-row bit test already in the plan** (brief A5 requirement met).

### A5.2 Phase-9 conflict model (D8)

The global-scan query reads the bit column under `&self` iteration exactly like any other filter (`filter_fetch`), declaring the SAME no-op `init_access` (C1 — a bitset id has no `ComponentPool`, no sibling `&mut` possible). The candidate-seeded query introduces NO new access declaration — it is a pure reader, identical to `Query<&D, Enabled<A>>` minus the data read. `D=()` ⇒ strictly FEWER reads than the round-7 in-scope shape. **It fits the Phase-9 conflict model with strictly LESS access surface than an already-approved shape.** D8 access contract unchanged.

### A5.3 Multithreading

- `snapshot_present` + `seed_from_candidates` + `clear_archetype` run in `new`/`update`/archetype-removal — all single-threaded (`&mut EcsMaster` at registration/direct-API, or the apply-window barrier where `running==0`). NOT on the worker hot path. Consistent with round-7 D1/D8 and A1.4's reconciliation (M1).
- Per-row reads in `par_iter` over candidate archetypes are `AtomicU64::load(Relaxed)` — identical to round-7; no writer live during iteration (v1 toggle is `&mut EcsMaster`). TB-clean (mirrors Phase-10 `UnsafeCell<Tick>`).
- The candidate snapshot is taken in `new`/`update` before any worker spawns; workers read an immutable `matched_ids`. No data race.

### A5.4 0%-gate (re-stated)

The only new emitted code on a non-enable monomorphization is ZERO (everything new is behind `if const { IS_CANDIDATE_SEEDED }` or `CONTAINS_ENABLE_TERM` gating, both of which fold out). The round-7 positive-term enable path is unchanged. **No round-7 0%-gate or resolution regresses.** Verified by the same `query_iter`/`spawn`/`par_iter`/`query_iter_changed` benches plus the new bounded global benches (A6).

---

## A6 — Which Steps/Decisions change

### Decisions

| Decision | Change |
|---|---|
| **D2** | C2 sub-rule NARROWED (not dropped): a SOLE SINGLE `Enabled<A>`/`Disabled<A>` over `D=()` is now ALLOWED via candidate-seeding; a tuple/composite of enable terms with no positive term stays REJECTED (`_C2` narrowed via `IS_SOLE_SINGLE_ENABLE` carve-out — A3.2/A3.3). `_C3` (Enabled+Changed), M1 (Or-reject), `Disabled`-no-cull, `for_each_chunk`-reject all KEPT. Add the A1.1 two-shape `Disabled` rule (per-row semantics identical; sole-shape candidate set = present-A; the shapes answer different questions; BOTH behaviors tested). |
| **D7** | The "sole-`Enabled` / cross-archetype enumeration" seam is PROMOTED from deferred to IN v1, for the SOLE SINGLE shape only. The candidate-seeded `update_archetypes` variant is now `seed_from_candidates` (concrete, A1.3). Remove "Out of v1 because it touches QueryState core." AND-of-N enable terms remains a documented future extension (A3.1). |
| **D1** | `EnablePresence` gains `snapshot_present(tag) -> ArchetypeBitSet` (128 B plain copy, single-threaded — A1.4) + `clear_archetype(arch)` (A4.4). `for_each_present`/`present_count` still NOT provided. The round-7 epoch field STAYS as the D7 forward seam but is NOT read by v1 `snapshot_present` (plain copy). Invariant added: `snapshot_present(A).popcount()` ≤ live-archetype count; the candidate walk is popcount-bounded, never `1..gen`. |
| **D8** | No change (candidate-seeded query is a pure reader with strictly less access than the round-7 in-scope shape — A5.2). |

### Steps

| Step | Change |
|---|---|
| **Step 3 (presence)** | ADD `snapshot_present` (128 B plain copy returning `ArchetypeBitSet`, single-threaded — A1.4) + `clear_archetype` (A4.4). Test: snapshot reflects allocs; popcount bounded; empty-when-never-toggled; `clear_archetype` removes the bit. NO epoch-retry protocol (M1). |
| **Step 4 (archetype wiring)** | ADD the `EnablePresence.clear_archetype(arch_id)` call at the archetype-removal site (the `structural_generation`-bump path). Test: removing a candidate archetype clears its presence bits. |
| **Step 7 (filter family)** | ADD `const IS_SOLE_SINGLE_ENABLE: bool = false;` to `QueryFilter` (default; `true` ONLY on `Enabled`/`Disabled` leaves; tuple/Or macros do NOT override — A3.3). ADD `fn sole_enable_tag_id(state) -> ComponentId` (default `unreachable!()`; `Enabled`/`Disabled` return `state.id` — A2.1). Both additive; no ABI break to existing leaves. |
| **Step 7a (the (D,F) seam — PINNED to state.rs:77)** | (1) PIN the const seam at `state.rs:77` (`QueryDataState::new`), NOT `query.rs` (critic round-7 MINOR-1). (2) NARROW `_C2` via the `IS_SOLE_SINGLE_ENABLE` carve-out (A3.2) — do NOT drop it; KEEP `_C3`. (3) ADD `IS_CANDIDATE_SEEDED` (= `IS_SOLE_SINGLE_ENABLE && !HAS_DATA && !HAS_POSITIVE_ARCHETYPAL`, A3.2) + `sole_enable_tag()` resolver reading `self.filter_state.id` (A2.1). (4) ADD the `if const { IS_CANDIDATE_SEEDED }` branch in `new` (A2.2) selecting `seed_from_candidates` over `update_archetypes`+`post_filter_matched`. (5) Re-run the grep-audit (A0): BOTH `ecs_master.rs:2769` AND `query.rs:495` (and all tests) route through `new`; confirm no bypass exists on live source. (6) `query_state.rs`: ADD `seed_from_candidates` (the SINGLE rebuild primitive — delta-add + struct-mismatch full-clear, popcount-bounded; A1.3/A1.5). NO `update_archetypes_struct_only` (deleted — A4.3). `update_archetypes`/`post_filter_matched` UNCHANGED. |
| **Step 7a — update() path (A4)** | ADD the `enable_generation`-triggered re-seed for the candidate path inside `if const { IS_CANDIDATE_SEEDED }`, REUSING the round-7 O2 `last_observed_enable_generation` field SLOT (written on the candidate path; the slot serves both const-disjoint roles — A4.2). NO new field vs round-7. |
| **Metrics** | ADD bench `query_iter_disabled_global` (`Query<(), Disabled<A>>` over a K-present-archetype world inside an N-archetype world, K≪N) asserting iteration cost scales with K, NOT N — the boundedness proof. ADD `query_iter_enabled_global` (`Query<(), Enabled<A>>`, same bound). The 0%-gate benches (`query_iter`, `spawn`, `par_iter`, `query_iter_changed`) UNCHANGED — must stay flat (`IS_CANDIDATE_SEEDED` const-folds out). |
| **Tests** | ADD: (a) `Query<(), Enabled<A>>` enumerates exactly the enabled rows across 3+ archetypes (each with an A-column, mixed bits), visits NO no-A-column archetype — assert via an archetype-touch counter that visited == K present, NOT N total (boundedness AND the C2 behavioral proof — A2.1). (b) the two-shape `Disabled` test (A1.1): one no-A-column archetype with entity `e:D` — `Query<&D, Disabled<A>>` VISITS `e`, `Query<(), Disabled<A>>` does NOT — asserting both behaviors with the documented-rule doc-comment. (c) A4 stale-presence: cached global query + first-toggle-into-new-archetype ⇒ re-seed sees it; PLUS removal: remove a candidate archetype ⇒ absent, no panic (A1.5/A4.4). (d) trybuild: `Query<(), Enabled<A>>` and `Query<(), Disabled<A>>` now COMPILE (invert the round-7 `c2_sole_*_rejected` trybuild into `compiles` tests). (e) trybuild regression: `Query<(), (Enabled<A>, Enabled<B>)>` STILL rejected (A3.1 deferral — enable-tuple with no positive term, not a single leaf); `Query<&P, (Changed<P>, Enabled<A>)>` STILL rejected (C3); `Or<(Enabled<A>, With<B>)>` STILL rejected (M1). (f) the behavioral seeded-arm proof (A2.1): the counter in test (a) is the oracle that `IS_CANDIDATE_SEEDED` folded to `true` and took the seeded path. |
| **OUT-of-scope** | REMOVE the "sole-flag cross-archetype enumeration / entity-disabling global scan NOT supported in v1" exclusion (SOLE SINGLE shape now IN scope). **ADD: AND-of-N enable terms with no positive term (`Query<(), (Enabled<A>, Enabled<B>)>`, same- or mixed-polarity) is DEFERRED** (needs a type-level multi-tag resolver — A3.1/M4). **KEEP OUT: `DefaultQueryFilters` auto-injection (Bevy 0.16 whole-entity disabling auto-adding `Without<Disabled>` to every query)** — NOT trivially included: it requires an implicit per-query filter-injection mechanism touching the `(D,F)` aggregation for ALL queries (a 0%-gate risk) + a global "the disabling tag" registration. Separate phase; the global-scan primitive this amendment adds is its building block, but the auto-injection policy is out. KEEP OUT: worker-marking (D8), `Added`/`Changed` on tags (D4), `for_each_chunk`+enable, `Enabled`/`Disabled` in `Or` (M1), Enabled+Changed mix (C3), per-page summary block-skip (v1.1). |
| **Open questions** | RESOLVE Open-Q-1: the brief owner promoted the SOLE SINGLE entity-disabling global scan into v1 scope. RESOLVE the previous draft's lone open question (`update_archetypes_struct_only`): DELETED — `seed_from_candidates` handles structural rebuild popcount-bounded (A4.3). No open question remains on this axis. |

### Wave dependency impact

Unchanged shape. Step 3 (presence + `snapshot_present` + `clear_archetype`) stays Wave 1. Step 4 gains the removal-site `clear_archetype` call (Wave 2). Step 7 gains `IS_SOLE_SINGLE_ENABLE` + `sole_enable_tag_id` (Wave 3). Step 7a (candidate-seeded build + narrowed `_C2`) stays Wave 3, depends on Step 7 + Step 3. No new wave.

---

## Changelog (round-7 → round-8 amendment, REVISED)

| Item | Round-7 | Round-8 amendment (revised) |
|---|---|---|
| Sole SINGLE `Enabled<A>`/`Disabled<A>` (`Query<(), Enabled<A>>`) | COMPILE-REJECTED (`_C2`, unbounded) | **ALLOWED** via candidate-seeding from `EnablePresence` (bounded, popcount-walk; A1-A3) |
| AND-of-N enable terms no positive term | rejected | **STILL rejected** (`IS_SOLE_SINGLE_ENABLE` carve-out; deferred — A3.1/M4) |
| Empty-include full-world scan hazard (M2/C2) | avoided by rejecting the shape | avoided by **never calling the `1..gen` scan** — seed directly from the bounded presence candidate bitset; `seed_from_candidates` is popcount-bounded on EVERY path including structural rebuild (A1.3/A1.5/A4.3) |
| `EnablePresence` enumeration | `for_each_present`/`present_count` OMITTED | ADD bounded `snapshot_present` (128 B PLAIN copy, single-threaded — NO epoch-retry; M1) + `clear_archetype` (A4.4); still no live-entity driver |
| `Disabled<A>` over no-A-column archetype | n/a (shape rejected) | per-row `filter_fetch` returns Disabled=true (UNCHANGED); sole-shape candidate set = present-A only; **two shapes answer different questions, documented + BOTH tested** (A1.1 — M2 resolution, NOT the previous "identical semantics" claim) |
| Funnel inventory | n/a | **CORRECTED: TWO non-test callers** (`ecs_master.rs:2769` + `query.rs:495`), both route through `new` at state.rs:77 (A0 — C1 resolution) |
| `sole_enable_tag` resolver | n/a | reads `self.filter_state.id` at RUNTIME via `sole_enable_tag_id` (NO type-level const; A2.1 — C2 resolution) |
| `IS_CANDIDATE_SEEDED` evaluation | n/a | referenced via `if const {}` in `new` (force-evaluated) + behavioral counter test proves the seeded arm is taken (A2.1 — C2) |
| Const-assert seam | `query.rs` (mis-filed round-7) | `state.rs:77` (`QueryDataState::new`); grep-audit re-mandated (A0/A3.6) |
| `_C2` | rejects all sole-enable | **NARROWED** (single-leaf carve-out via `IS_SOLE_SINGLE_ENABLE`), NOT dropped (A3.2 — M4 resolution) |
| O2 invalidation field | positive-term cull slot | same slot serves both const-disjoint roles; written on the candidate path; NO new field (A4.2 — M3 honest accounting) |
| `update_archetypes_struct_only` | n/a (previous draft proposed) | **DELETED** — `seed_from_candidates` is the single popcount-bounded rebuild primitive (A4.3) |
| `_C3` (Enabled+Changed), M1 (Or), D4, D8, O1, C1 | — | **PRESERVED VERBATIM** |
| `DefaultQueryFilters` auto-injection | out of scope | **STAYS out** (0%-gate risk on all-query aggregation; A6) |

---

## Plan-readiness self-check (amendment delta)

- C1 resolved: funnel inventory CORRECTED — two non-test callers (`ecs_master.rs:2769` + `query.rs:495`), both route through `new` at state.rs:77; grep-audit re-mandated. ✓
- C2 resolved: `sole_enable_tag_id` reads `filter_state.id` at runtime (no type-level const); `IS_CANDIDATE_SEEDED` force-evaluated via `if const {}` in `new`; behavioral counter test proves the seeded arm. ✓
- M1 resolved: `snapshot_present` is a PLAIN single-threaded memcpy (epoch-retry framing dropped); consistent with D1 `Relaxed`-soundness + A5.3 single-threaded claim. ✓
- M2 resolved: two-shape `Disabled` rule — per-row semantics identical, sole-shape candidate set = present-A, the shapes answer different questions, documented + BOTH behaviors tested (not "identical semantics"). ✓
- M3 resolved: honest field-reuse (same slot, two const-disjoint write sites, no new field); `seed_from_candidates` ABA argument explicit (structural full-clear); `update_archetypes_struct_only` DELETED; `clear_archetype` added for removal completeness. ✓
- M4 resolved: scope restricted to the brief's literal SOLE SINGLE shape; AND-of-N deferred; `_C2` NARROWED not dropped via `IS_SOLE_SINGLE_ENABLE`; no under-specified multi-tag resolver. ✓
- 0%-gate: `IS_CANDIDATE_SEEDED` const-folds out for every non-sole-single-enable `(D,F)`; positive-term enable path unchanged from round-7. ✓
- Boundedness: candidate walk = `snapshot_present(tag).popcount()`, never `1..gen`, on EVERY path; bench `query_iter_disabled_global` asserts scaling with K not N. ✓
- No round-7 resolution regressed: C1/C3/M1/D4/D8/O1/O2 preserved; only C2/C4/D7 (the sole-enable axis) changed. ✓
- No new unsafe; per-row test is the already-designed `AtomicU64::load(Relaxed)`; fits Phase-9 (pure reader, less access than an approved shape). ✓

## Rejected remarks

None. All round-1 critique remarks (C1, C2 CRITICAL; M1, M2, M3, M4 MAJOR) are adopted and resolved above. The two MINOR acknowledgements (O1 boundedness-confirmed, O2 0%-gate-sound) are preserved: O1's mechanism (`seed_from_candidates` via popcount/trailing_zeros + `push_matched` + `get_archetype` liveness, NO `self.matches(empty_include)`) is kept verbatim; O2's `if const { IS_CANDIDATE_SEEDED }` fold and the `if !const { F::IS_ARCHETYPAL }` per-row reuse for `D=()` are kept, with C2's resolver gap now closed without adding any runtime branch to the no-enable path (the resolver is called only under `if const { IS_CANDIDATE_SEEDED }`).

---

Amendment is a delta alongside `D:\claude\BoykoEngine\docs\ENABLE-TAG-PLAN.md` (append as a "Round-8 Amendment" section or save as `D:\claude\BoykoEngine\docs\ENABLE-TAG-PLAN-AMENDMENT-GLOBAL-SCAN.md`). Load-bearing source seams verified this session against live `ecs` HEAD: `state.rs:77-106` (`QueryDataState::<D,F>::new`, the sole `(D,F)` funnel — both generics in scope, `filter_state` built at :79, `update_archetypes` at :91, `post_filter_matched` at :92); `state.rs:200-214` (`update`, the warm-path generation-diff gate to extend); `query.rs:495` (`Query as SystemParam>::init_state` — the SECOND non-test `new` caller, C1 correction); `ecs_master.rs:2769` (`get_or_init_query_state` — the first non-test `new` caller); `query_state.rs:174-184` (the structural-mismatch full-clear ABA rebuild that `seed_from_candidates` mirrors); `query_state.rs:195-228` (the `1..gen` empty-include full scan that the candidate path bypasses — the M2 hazard); `query_state.rs:288` (`push_matched` reuse); `filter.rs:371`/plan-line-371 (`EnabledState<T>{ id: ComponentId }` — the runtime tag-id source for the resolver); `filter.rs:1059`/`:1267` (tuple/Or `filter_fetch` fold — confirming `Or`+enable is M1-rejected and tuples-of-enable are not single leaves).

---

## Critic verdict

APPROVED (round 2). Convergence: R1 REVISE(2C/4M) -> R2 APPROVED(0C/2M/4m).

Summary: Amendment is bounded, 0%-gate-safe, and implementable as a delta. The sole-enable scan seeds from the popcount-bounded EnablePresence candidate set and structurally never reaches the 1..gen full-world scan (query_state.rs:195). C1/C2/C3/M1 verified against live source. Remaining items are precision/verification gaps, not blockers.

Remaining MAJOR/MINOR (precision/verification — fold into Wave-3 dev briefs, not blockers):

1. [MAJOR] Boundedness CONFIRMED (the #1 job). seed_from_candidates walks candidates.popcount() set bits via trailing_zeros over a 16-word ArchetypeBitSet (verified: ARCH_BITSET_WORDS=16, MAX_ARCHETYPES=1024; PRESENCE_WORDS=16, same 1024-bit capacity) and explicitly bypasses update_archetypes' `for id in 1..current_gen.get()` loop at query_state.rs:195-209 — the exact M2/C2 hazard the round-7 plan rejects (plan line 85/87). The candidate set EnablePresence[A] is structurally <= live archetypes. No genuinely-unbounded shape is admitted: IS_CANDIDATE_SEEDED is tightened to require F::IS_SOLE_SINGLE_ENABLE (single leaf only), and _C2 is NARROWED not dropped, so an enable-tuple with no positive term stays compile-rejected. The lift is safe.

2. [MAJOR] A4.2 'field-reuse / no new struct growth' rests on an UNVERIFIED stored field. Source grep confirms `last_observed_enable_generation`, `has_enable_term`, `cull_enable_archetypes`, and `CONTAINS_ENABLE_TERM` do NOT exist yet — the entire round-7 D2 query-integration layer is unbuilt (only the storage layer EnablePresence/enable_generation/EnableStore exists). The round-7 plan text (line 467) describes O2 only as a 'THIRD enable_generation CHECK gated by has_enable_term', NOT unambiguously as a STORED `last_observed_enable_generation: u64` field on QueryDataState. The amendment's preamble fact #4 conflates this with `last_observed_archetype_generation`/`last_observed_structural_generation`, which live on QueryState (the inner, filter-agnostic struct), not QueryDataState. REQUIRED: pin the field's existence/shape against the round-7 plan's struct definition (or state that the dev wave defines it); the 'no new field' accounting is only honest if round-7 actually mints a stored slot on the CONTAINS_ENABLE_TERM monomorphizations.

3. [MINOR] snapshot_present is mischaracterized as a '128 B PLAIN memcpy of a Box<[u64;16]>'. Live source: per-tag storage is `Box<PresenceWords>` = `Box<[AtomicU64;16]>` behind an `AtomicPtr<PresenceWords>` (enable_presence.rs:68,84), mutated via fetch_or. Producing an `ArchetypeBitSet{bits:[u64;16]}` requires 16 per-word atomic LOADs (Relaxed/Acquire) plus a null-slot check (never-toggled tag => empty), NOT a memcpy of atomics-to-plain. The byte count (128 B) and the no-epoch-retry single-threaded conclusion are correct, but the implementation primitive is a 16-atomic-load loop. Dev brief should say 'snapshot via per-word atomic load', not memcpy, so nobody attempts a transmute/memcpy of atomics.

4. [MINOR] clear_archetype(arch) cost is understated. The existing EnablePresence has NO removal path and NO per-archetype reverse index; tags is `[AtomicPtr; MAX_COMPONENTS]` (512 slots). Clearing 'arch's bit across all tags' is a 512-slot walk (skipping nulls), not O(1). This is acceptable (it runs ONLY on archetype removal = a rare structural-gen bump, off the hot path, and the amendment notes 'small/rare'), but the brief should call it a per-removal 512-slot scan so it isn't mistaken for O(1). Also note clear_archetype takes `&self` and mutates words via the same atomic discipline — consistent with the existing note_column_alloc `&self` model; confirm the bit-clear uses fetch_and(Release) to mirror note_column_alloc's fetch_or(Release).

5. [MINOR] Source-line citation error in the amendment footer: 'filter.rs:371 (EnabledState<T>{ id: ComponentId })'. Verified: filter.rs:371 is `set_table_readonly_no_meta` for With<C>, unrelated. EnabledState<T> exists only at PLAN line 371 (verified — docs/ENABLE-TAG-PLAN.md:371 defines `pub struct EnabledState<T>{ pub(crate) id: ComponentId, .. }` in filter_enable.rs (NEW), still unbuilt). The amendment hedges as 'filter.rs:371/plan-line-371' but the filter.rs:371 half is a false source pin. The C2 resolver mechanism itself (read self.filter_state.id at runtime) is sound and correctly grounded in the plan; only the source citation is wrong. Drop the filter.rs:371 reference.

6. [MINOR] Positive confirmations to PRESERVE: (1) C1 verified — exactly two non-test `new` callers (ecs_master.rs:2769 get_or_init_query_state, query.rs:495 Query-as-SystemParam init_state), both route through state.rs:77 `new`; no alternate QueryDataState constructor; QueryState::new is filter-agnostic and cannot host shape logic. The grep-audit re-mandate (Step 7a) is the right guard. (2) enable_generation: AtomicU64 confirmed on ArchetypeMaster (line 86) with Relaxed reader (534) + bump_enable_generation (557), bumps on column-alloc, independent of structural_generation — A4's invalidation trigger is real and load-bearing. (3) C3 (Enabled+Changed reject) and M1 (Or reject via no OrComposable impl) are intact in the plan (lines 175-178, 188) and transitively guard the new path (a single enable leaf has CONTAINS_CHANGE_DETECTION=false; any mix is not a single leaf). (4) The two-shape Disabled rule (A1.1) — per-row filter_fetch semantics identical (no-column => Disabled=true, plan line 184), sole-shape candidate set = present-A only, BOTH behaviors tested — is the correct, coherent resolution of M2 (the shapes answer different questions). Keep all of these.

