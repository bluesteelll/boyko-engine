> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Architecture review: Phase 10 — Change Detection

## Verdict
[ ] APPROVED — the plan is ready for implementation
[X] CHANGES REQUESTED — needs revision (see remarks)

The plan is well-researched and mirrors Bevy's post-PR #6547 design faithfully. The decision matrix (§3 Q1-Q12) covers the right alternatives. However, **four critical issues** in the Phase 9 integration boundary and **several important issues** in storage / wraparound / soundness need resolution before implementation.

---

## Remarks

### Critical (blockers)

#### C1. `SystemMeta::this_run_tick` writes — dispatcher does NOT have direct access
**Where**: §2.6 SCT4, §2.8 PHASE9.2, §4.5, §8.2 step 3, §14 Step 11, §20 cross-reference table.

**Problem**: The plan repeatedly asserts `self.systems[i].meta.this_run_tick = this_run` is a direct dispatcher field write. This is **structurally impossible** as the codebase stands:
- Phase 9 stores systems as `SystemBox { system: Box<dyn System<Out=()>>, .. }` (Phase 9 §5.2, `system_box.rs:1155`).
- `trait System` exposes `name()`, `access()`, `initialize()`, `run_unsafe()`, `apply()` — **no `meta_mut()` accessor**. `meta` is owned by the *concrete* implementor (`FunctionSystem.meta` at `function_system.rs:159`).
- The dispatcher only sees `Box<dyn System>` — it cannot reach through the vtable to a struct field.

The plan never adds an accessor like `fn set_this_run_tick(&mut self, tick: Tick)` to the `System` trait, yet relies on the write happening from the dispatcher per the happens-before chain in §2.8 PHASE9.2 / §8.2.

**Why critical**: Without resolving this, neither `this_run_tick` nor `last_run_tick` can actually flow from dispatcher to the system body — the entire change-detection mechanism does not work. Worse, the SAFETY claim in §2.8 PHASE9.6 ("written by the dispatcher only; read by workers through `&SystemMeta`") is uncheckable.

**What is needed**: Either (a) add a trait method `fn meta_mut(&mut self) -> &mut SystemMeta` (changes the System trait API, affects every implementor), or (b) move the tick state out of `SystemMeta` into a dispatcher-owned `Box<[(Tick, Tick)]>` parallel array indexed by `SystemIndex`, with workers reading via a separate channel (e.g., parameter to `run_unsafe`). Decide and document; show how `get_param`'s `&meta` argument exposes the value.

---

#### C2. `QueryIter` / `QueryIterMut` do NOT currently hold `&'s SystemMeta`
**Where**: §5.3 ("Update QueryIter / QueryIterMut next methods to pass meta (already held as field)"), §14 Step 8, §20.

**Problem**: The plan asserts the iterator "already holds `meta` as field". Verified in `iter.rs:82-92` (`QueryIter`) and `iter.rs:258-268` (`QueryIterMut`): both structs only hold `archetype_ids`, `data_state`, `filter_state`, `world`, `data_fetch`, `filter_fetch`, `current_row`, `current_len`, `_marker`. **No `meta` field exists**. The `meta` lives on `Query` (`query.rs:62`) but is not forwarded to the iterator in `QueryIter::new(state, world)` (`iter.rs:128-143`).

**Why critical**: `set_table_*` is called inside the iterator's per-archetype-boundary path; with no `meta` on the iterator, the plan's signature change (`set_table_*(meta: &SystemMeta)`) cannot be wired up. Step 8 mis-scopes the work — it claims "trivial" but actually requires extending iterator constructors, callers (`Query::iter`/`iter_mut`), and storing one more reference. Also: the new field tightens lifetime bounds (`'s: 'q` already exists; adding `meta: &'s SystemMeta` is fine, but the constructor signature changes and every test that builds an iterator manually breaks).

**What is needed**: Step 8 must explicitly extend `QueryIter`/`QueryIterMut` to carry `meta: &'s SystemMeta` (not `meta: &'q SystemMeta` — keep state and meta on the same lifetime). Update `QueryIter::new`/`QueryIterMut::new` signatures and `Query::iter`/`iter_mut` callers. List the breakage to existing tests (`tests/query_dsl_smoke.rs`, etc.).

---

#### C3. `Mut<T>::deref_mut` SAFETY contract under `par_iter` row-boundary cache line sharing
**Where**: §2.5 MUT3, §8.3, §8.4, §11.5.

**Problem**: §11.5 acknowledges: "If chunks are 256 rows and the cache line holds 16 ticks (64 B / 4 B), adjacent chunks may share a cache line at the boundary. Cost: trivial because the chunks process the boundary rows sequentially within each chunk." This dismisses the issue but the soundness argument is incomplete.

Two adjacent `par_iter` workers writing `changed_ticks[N-1]` (worker A) and `changed_ticks[N]` (worker B) on the same cache line through plain non-atomic `*UnsafeCell::get() = tick` stores **is a data race per Rust's abstract machine**, even though the rows are disjoint. The C++/Rust memory model treats two non-atomic writes to the same cache line from different threads as a race when there's no happens-before, regardless of whether the bytes overlap. Phase 9 SCH3 establishes happens-before across systems but not across `par_iter` chunks of the *same* system.

Re-reading §5 Q5.4: "The implicit memory order is supplied by the scope's `pending` counter (the calling thread waits for all chunks via `scope.Drop`'s work-stealing wait, which performs a Release/Acquire pair via the scope's `pending`)". The scope's join provides happens-before for the **post-join** observer, not for inter-worker writes during execution. Two concurrent workers writing adjacent u32s in the same cache line race per the abstract machine even though no byte aliases.

**Why critical**: Miri will flag this. The plan's `miri_phase10_par_iter_disjoint_tick_writes` test (§13.3) WILL fail — Miri's TreeBorrows model treats `&UnsafeCell<u32>` writes from different threads as racy unless mediated by atomics or scope barriers.

**What is needed**: Either (a) justify formally that disjoint-index writes to disjoint `UnsafeCell` objects are sound under Rust's data-race model (cite [P0250R3 / "no concurrent unsynchronized access to the same memory location"]); the abstract machine treats each `UnsafeCell<u32>` as a separate memory location, and two writes to different `UnsafeCell` instances at the same cache line ARE safe per the language spec — the issue is purely a MESI cache-coherence cost, not UB. If that's the position, state it explicitly with a citation, because the plan's reader will assume "same cache line = race". Or (b) require `AtomicU32` for the tick fields with `Relaxed` writes — but this is the very thing §3 Q5 rejected. Pick a path and document it crisply, with a Miri test that the chosen reasoning passes.

---

#### C4. `Or<(With<A>, Changed<B>)>` archetype gate misses the unmatched-archetype case
**Where**: §3 Q7.3, §5.4 ("Conclusion: the debug_assert is necessary as defense in depth"), §5.4 final `filter_fetch` with null-base early return.

**Problem**: The plan adds `if fetch.tick_base.is_null() { return false; }` to handle archetypes where the `Or` accepts via the *other* element. But:
1. The semantic is wrong. Consider `Or<(Changed<A>, Changed<B>)>` on an archetype that contains both A and B. If A's tick is changed but B's isn't: filter_fetch returns `true || false = true`. Correct. If neither: `false || false = false`. Correct. Now consider `Or<(With<A>, Changed<B>)>` on an archetype that contains A only (no B). `matches_component_set` returns `true || false = true` (OR accepts). Iterator enters the archetype. `With<A>::set_table_readonly` is a no-op (archetypal). `Changed<B>::set_table_readonly` sets `tick_base = null`. Per row: `With<A>::filter_fetch = true` (archetypal stub) || `Changed<B>::filter_fetch = false` (null base) = `true`. **Correct.**
2. But: the `Or<F>::IS_ARCHETYPAL` const is `true && false = false`, so the per-row branch *is* taken — every row pays the cost of both predicates. Fine.
3. The real issue: §3 Q7.4 / §2.3 FLT4 says `Added<C>::aggregate_include` sets the bit for C. **But Or's `aggregate_include` is a no-op** (Phase 8b filter.rs:604, verified). So `Or<(Added<A>, Changed<B>)>` does NOT add A or B to the include mask. The state's `update_archetypes` will then match every archetype — including archetypes lacking BOTH A and B. The `post_filter_matched` filters those out via `Or::matches_component_set`, which OR-folds the component-set check — accepting archetypes containing A OR B.

So archetypes containing neither A nor B are excluded. Archetypes containing only A: `Changed<B>::set_table_readonly` is called with an archetype lacking B → null tick_base. The null-check in `Changed<B>::filter_fetch` works for the OR composition, BUT…

4. The deeper problem: `Changed<B>` declares a **read** of B in `init_access` (§2.3 FLT2). The system's access surface includes B. But in an `Or<(With<A>, Changed<B>)>` query, the user might *expect* that running on an A-only archetype doesn't require world-side access to B. The access declaration is conservative; Phase 9's conflict graph will serialize this system against any concurrent writer of B even when it's "ineffective" on A-only archetypes. This may be intended (mirrors Bevy) but should be called out as documented behavior, not glossed over.

**Why critical**: The null-base branch is presented as an optional defense-in-depth (`debug_assert`), then escalated to a release-build runtime check ("Decision: (a) — add the null-base check"). This is a real runtime cost on every iteration of `Or` filters even in the *common* case where the archetype matches the primary side. The plan should explicitly cost this in §10's per-row table — adding one predicted-not-taken branch on a hot path.

**What is needed**: 
- Clarify whether `Or<(_, Changed<C>)>` requires the system to declare read-access to C even when C is absent from the matched archetype (yes per the conservative model, but state it).
- Show the null-base branch cost in §10's per-row breakdown (it's not free under the `Or` composition path).
- Add a test that `Or<(With<A>, Changed<B>)>` on an A-only archetype does NOT crash and DOES iterate (this is the load-bearing test for the null-base path).

---

### Important (must fix before APPROVE)

#### W1. Memory-overhead claim of 40 MB ignores per-archetype duplication
**Where**: §1.2 metrics table, §2.2 STORE2, §7.2, §10.7, §11.6.

**Problem**: "100 k × 50 → 40 MB" assumes each entity contributes 8 B per component. But ticks live in `ComponentPool`, and a component lives in **every archetype** that contains it. A `Position` component used in 100 archetypes means 100 separate `added_ticks`/`changed_ticks` buffers, each sized to `max_components` (the pool's capacity, NOT the live entity count). Phase 7's `ComponentPool::with_default_sizes` allocates `num_chunks * components_per_chunk` slots up front (per `component_pool.rs`). The plan's number assumes only live rows are charged — wrong.

§11.3 says "Heap-side tick buffer overhead per archetype: depends on chunk sizes (§7.2). Maximum: 8 KB per chunk × N chunks × N components." This contradicts the §1.2 / §10.7 "40 MB total" claim. The actual upper bound is `archetypes × components_per_archetype × chunks × 2 × 4 B × components_per_chunk`. For 1024 archetypes × 10 components × 1 chunk × 2 ticks × 4 B × 1024 rows/chunk = 80 MB at modest scale; worst case much higher.

**Why important**: This is a CLAUDE.md "minimum allocations" principle issue. The plan over-promises. The real number can be 2-5x larger than 40 MB at the AAA design point, and the architecture must consciously accept this.

**Solution options**:
- Recompute upper bound using the actual `max_components` per pool × archetype count.
- Lazily allocate tick buffers (grow with `units.len()`) — but this contradicts the parallel-array invariant.
- Accept a higher bound (revise §1.2 and §10.7).
Decide and revise the numbers.

---

#### W2. `MAX_CHANGE_AGE` derivation math is hand-waved
**Where**: §9.3 (the "Let me re-check" paragraph) — verbatim: *"Sum: 3_258_166_895 + 518_400_000 = 3_776_566_895. u32::MAX / 2 = 2_147_483_647. Sum exceeds half of u32::MAX. But Bevy ships this. Re-reading the PR more carefully: ... Empirically: Bevy's formula has been in production since 2022. We adopt it verbatim."*

**Problem**: The architect literally documents that they don't understand the proof. "It works in Bevy" is not engineering — it's cargo-culting. The plan adopts the constant by faith rather than derivation. A reader has zero confidence the wraparound math is sound under boyko's specific tick-bump policy (per-frame), which differs from Bevy's per-system policy. The actual constraint involves the relative-age comparison's signed-comparison semantics; the formula's correctness depends on the bump policy.

**Why important**: If the per-frame bump policy invalidates the formula's assumptions (e.g., because between-scan tick growth differs), correctness silently breaks. The property test in §13.4 (`prop_is_newer_than_wraparound_invariant`) may catch it — but the architect should prove the invariant before the implementer trusts it.

**Solution options**:
- Add a §9.x "Wraparound proof" subsection that derives `MAX_CHANGE_AGE` from first principles using boyko's per-frame bump (NOT Bevy's per-system). Show the precise inequality the formula satisfies. Or weaken `MAX_CHANGE_AGE` to a known-safe `u32::MAX / 2 - CHECK_TICK_THRESHOLD` and accept the more frequent scan.
- Cite a specific Bevy commit / RFC with the proof and verify the bump-policy assumption matches.

---

#### W3. `check_ticks` scan walks `pool.units_len()` but allocations are `max_components`
**Where**: §4.6 (`run_check_ticks_scan`), §9.6.

**Problem**: Both pseudocode listings iterate over `pool.added_ticks.iter()` — that's `max_components` (the buffer length), not `pool.units_len()` (the live entity count). Empty slots will be scanned and clamped, wasting time. At 1024 archetypes × 50 components × max_components=1024 rows × 2 ticks = 100M slots scanned even if only 100k entities are live. The §10.6 estimate "10 M `u32::wrapping_sub + compare`" assumes live-row counting but the code as written scans buffer-len.

**Why important**: Cold path, but order-of-magnitude wrong cost estimate. The 3 ms figure becomes 30+ ms — a real frame stutter, even once per 24 h.

**Solution options**:
- Scan only the first `units_len()` slots (correctness still holds — unused slots are `Tick::ZERO` and irrelevant).
- Or document that the scan walks the full buffer and revise the cost in §10.6 + §1.2.

---

#### W4. `Commands::spawn` tick threading is hand-waved
**Where**: §15.1 last row ("Phase 8.5 `Commands::spawn` apply path — Trivial"), §2.4 INIT2, §20 cross-reference last row.

**Problem**: §2.4 INIT2 claims `Commands::spawn` apply "threads `current_tick` from the dispatcher's `world.change_tick.load(Relaxed)` into `Archetype::create_entity`. The dispatcher reads the tick once at the start of the apply window." But:
1. `EcsMaster::create_entity` (`ecs_master.rs:244-305`) does NOT currently accept `current_tick`. The signature gain affects every caller including `SpawnCommand::apply` (`spawn_command.rs:209-210`).
2. The Phase 8.5 `SpawnCommand` calls `world.create_entity(archetype_id, initialized)` — for tick threading, `EcsMaster::create_entity` must read `self.change_tick.load(Relaxed)` internally OR a new `EcsMaster::create_entity_with_tick(tick, ...)` variant is added.
3. INIT2 / INIT3 contradict each other: INIT2 says the dispatcher reads the tick and passes it down; INIT3 says `EcsMaster::create_entity` reads `self.change_tick.load` itself. Pick one.

**Why important**: Without resolution, the `Added<T>` semantic for spawned-via-Commands entities is unspecified. The "Entity added this frame is Added next frame" test (§13.2 `added_filter_basic_spawn_query`) depends on which tick the spawn sees.

**Solution options**:
- Make `EcsMaster::create_entity` read `self.change_tick.load(Relaxed)` (INIT3 path). Drop INIT2.
- Or thread the tick explicitly from `SpawnCommand::apply` via a separate `create_entity_with_tick` API. Document the convention.

---

#### W5. `init_state` in §4.2 is `fn`, not `const fn` — Tick::ZERO sentinel issue
**Where**: §4.2 Tick definition.

**Problem**: `pub const ZERO: Tick = Tick(0)` declares ZERO at the type level. STORE10 says "Tick buffers are zero-initialized at allocation. Logical 'first ever value' is `Tick::ZERO`. But §2.1 TICK8 / §2.4 INIT1 ensure `Tick::ZERO` is never a meaningful comparand". OK — but at `EcsMaster::new`, `change_tick = AtomicU32::new(0)`. First `Schedule::run` does `fetch_add(1) → prev=0, this_run=Tick(1)`. Before the first system runs, if a user calls `EcsMaster::create_entity` (per INIT3 — direct API), `current_tick = AtomicU32::load = 0 = Tick::ZERO`. That entity gets `added=changed=Tick::ZERO`. A subsequent system with `last_run = Tick::ZERO.wrapping_sub(MAX_CHANGE_AGE)` checks `is_newer_than(last_run, this_run=1)` on tick `0`:
- `age_self = 0.wrapping_sub(0 - MAX_CHANGE_AGE) = MAX_CHANGE_AGE`.
- `age_this = 1.wrapping_sub(0 - MAX_CHANGE_AGE) = MAX_CHANGE_AGE + 1`.
- `age_this > age_self = true`. The Tick::ZERO entity reports as Added/Changed. OK.

But if the system's `last_run = Tick::ZERO` (NOT the `current - MAX_CHANGE_AGE` initialization), then `age_self = 0, age_this = 1, age_this > age_self = true`. Still works.

But: `TICK8` requires `last_run = current - MAX_CHANGE_AGE` "at first run". `SystemMeta::new` (current code, `system_meta.rs:58`) defaults `last_run_tick = Tick::ZERO` (will, post-Step 3). The §14 Step 15 sets `last_run` correctly **inside `initialize()`** — but **only for systems added via Phase 8c `IntoSystem`**. Any other code that constructs a `SystemMeta` skips Step 15. If a system is built directly (test helpers, future custom system kinds) and never goes through `FunctionSystem::initialize`, its `last_run` stays `Tick::ZERO` → first-run semantics may be wrong if `current_tick` is also small.

**Why important**: Test fixtures like `NoopSystem` (system.rs:131) and any direct `SystemMeta::new` use must also wire the `last_run = current - MAX_CHANGE_AGE`. The plan does not enumerate the impact on `ExclusiveFunctionSystem::initialize` either (mentioned in §15.1 but not detailed).

**Solution options**:
- Move the `last_run = current - MAX_CHANGE_AGE` initialization into `SystemMeta::new(name, current_tick)` (constructor takes the tick). Every system construction site now must pass it. Most code is one site (`FunctionSystem::new` / `ExclusiveFunctionSystem::new`).
- Or document explicitly that bypassing `initialize()` is unsupported and add a `debug_assert!` in `Mut::deref_mut` / `Added::filter_fetch` that `meta.last_run_tick != Tick::ZERO`.

---

#### W6. `ParQuery` / `ParQueryMut` field plumbing
**Where**: §8.4, §14 Step 8, §15.1.

**Problem**: §8.4 says "Add `meta: &'s SystemMeta` to `ParQuery` / `ParQueryMut`. Then `for_each` passes `&meta` through the spawn body into each chunk's `set_table_*` call." But the par_iter chunk worker holds `ChunkCaptures` (`par_iter.rs:320-327`), which is `Send`. `&SystemMeta` is `Send + Sync` (`Access` + `&'static str` + ticks). OK — but the closure capture must forward it as a raw pointer or a `Send` reference. The plan should specify which.

Also, in the PAR9 inline path (`par_iter.rs:300`, `entity_count < MIN_ARCHETYPE_FOR_PARALLEL`), `run_chunk_inline` is called WITHOUT going through `scope.spawn`. The meta must be forwarded there too. The plan does not mention this branch.

**Why important**: Step 8 will get partially implemented and inline-path queries lose change-detection if missed.

**Solution options**: Explicitly enumerate every `run_chunk_inline` / `run_chunk_owned` / fallback path and confirm meta forwarding. Add a test that par_iter inline path (`entity_count < 1024`) correctly reports `Changed<T>`.

---

#### W7. `set_table_*` trait signature: lifetime of `meta` ref vs `Fetch<'w>`
**Where**: §5.3 (new trait signature), §5.1 / §5.4 (Fetch caching `last_run`/`this_run` by value).

**Problem**: The new signature is `fn set_table_*<'w>(fetch: &mut Self::Fetch<'w>, state: &Self::State, archetype: *const/*mut Archetype, meta: &SystemMeta)`. No lifetime on `meta`. Should be `meta: &'_ SystemMeta` — but does the compiler invent a lifetime, and what does it bind against? The `Fetch<'w>` caches `last_run: Tick` / `this_run: Tick` **by value** (Copy), so no lifetime escapes the function. Good. But for `Ref<'w, T>` and `Mut<'w, T>`, the Item carries `&'w UnsafeCell<Tick>` references — those need to be re-borrowed from the archetype, not from `meta`. So `meta`'s lifetime is purely-input-not-stored. State this.

**Why important**: Without explicit lifetime annotation, a future maintainer might attempt to store `&meta` in `Fetch<'w>` and fail compilation in mysterious ways.

**Solution options**: Annotate `meta: &'_ SystemMeta` and document "meta is read-only for the duration of set_table_*; ticks are copied into Fetch by value".

---

#### W8. `Or<F>::aggregate_include` no-op + non-archetypal element interaction
**Where**: §3 Q7.4, §16.10 OQ-8.

**Problem**: §3 Q7.4 states "For `Or<(...)>`: Phase 8b's `Or<F>::aggregate_include` is **explicitly a no-op** (filter.rs:603-606 M8 contract). The Or semantics are enforced by `QueryDataState::post_filter_matched`". So for `Or<(Added<A>, Changed<B>)>`, `update_archetypes` walks ALL archetypes (no include mask), `post_filter_matched` keeps only those matching `Added<A>::matches_component_set || Changed<B>::matches_component_set` (i.e., containing A OR B). For each kept archetype, `Or<F>::set_table_readonly` calls **both** element's `set_table_readonly`. For an A-only archetype, `Changed<B>::set_table_readonly` writes `tick_base = null` (C4). Fine.

But: `Added<A>::aggregate_include` sets bit A. When the Or contains Added/Changed elements, the Or's aggregate is no-op — meaning the per-component-id include mask **does not record A**. If the user separately writes `Query<(&A,), Or<(Added<A>, Changed<B>)>>`, the data side's `&A::aggregate_include` sets A, so it's matched. But for `Query<(&C,), Or<(Added<A>, Changed<B>)>>` (no A/B in data), no archetype mask constraint exists — `update_archetypes` walks all, post_filter filters by Or's `matches_component_set`. OK at runtime, but **archetype iteration cost balloons**: every archetype is touched, filter is checked per-row, while a smarter system could pre-filter by `A | B`.

**Why important**: Performance — `Or<(Added, Changed)>` queries scan every archetype rather than only those containing the components. The plan claims §10.7 acceptable cost but never measures this case.

**Solution options**:
- Acknowledge in §10 hot-path projections that `Or<(Added, Changed)>` queries scan every archetype (no include-mask narrowing).
- Or revise `Or::aggregate_include` to UNION the elements' includes — but this breaks Phase 8b's M8 contract. Out of scope; just document.
- Add a §13.5 bench `bench_or_added_changed_archetype_count_dominated`.

---

### Optional (improvements, not blockers)

#### O1. `Tick::is_newer_than` semantics — exclusive vs inclusive
§4.2 documents "self ∈ (last_run, this_run]". Standard Bevy semantic. But §2.5 MUT3 writes `*changed = this_run` on deref; in the same frame's subsequent read, `tick.is_newer_than(last_run, this_run)` with `tick == this_run == self.this_run` returns `this_run.wrapping_sub(last_run) > self.wrapping_sub(last_run)` = `X > X` = `false`. **A mutation made in the same system is NOT visible to the same system's later `is_changed()` call.** This contradicts §6.2 MUT6 docstring "After `Mut::deref_mut`, `Ref::is_changed` returns true" and the §13.1 test `ref_is_changed_reads_tick`. The Bevy semantic actually uses `age_this >= age_self` (or equivalent) for self-observation — verify and fix.

#### O2. `CachePadded` on `change_tick` may be over-engineered
§4.4 wraps the atomic in `CachePadded`. The atomic is touched once per frame (dispatcher fetch_add) plus N reads per apply window. False sharing risk is essentially zero. `CachePadded` adds 60 B of waste. Either remove (trust the layout) or document the measured benefit.

#### O3. Inlining strategy not specified for hot path
The plan marks `#[inline]` on `is_newer_than`, `filter_fetch`, `deref_mut` — fine. But CLAUDE.md principle #7 says "measured inlining". The plan should note that benchmark validation (Step 16) checks the inlining decisions held (e.g., via `cargo asm` or PGO measurement on `bench_changed_filter_1024_rows`).

#### O4. `Tick` type doesn't impl `Default`
Minor: `derive(Clone, Copy, PartialEq, Eq, Hash, Debug)` — no `Default`. Users will surprise themselves trying to put `Tick` in a struct that derives `Default`. Add `Default` returning `Tick::ZERO`.

#### O5. Naming consistency — `last_run_tick` vs `last_run`
The struct uses `last_run_tick: Tick` and `this_run_tick: Tick`, while the `SystemChangeTick` SystemParam uses `last_run: Tick, this_run: Tick` (§2.6 SCT1). Either harmonize to `last_run` everywhere or document why the names diverge.

---

## Positive

- §3 decision matrix (Q1-Q12) is thorough, with rejected alternatives spelled out for each.
- §8.1 atomic discipline table is excellent — every atomic op listed with ordering, frequency, justification.
- §3 Q1 false-positive-blast-radius argument for rejecting per-archetype storage is correct and well-articulated.
- The post-PR #6547 parallel-array layout choice (separate `added_ticks` and `changed_ticks`) is right; §3 Q8 cache analysis is solid.
- §7 cache footprint analysis with concrete cache-line counts is the right level of rigor.
- The Phase 9 SCH3 reuse for tick-write happens-before is the correct architectural move (no new atomics).
- §16 rejected alternatives section is thorough — shows the architect considered the design space.
- Step partitioning (§14) is sane; the 16-step decomposition with explicit dependencies is implementable.
- §13 test plan covers unit + Miri + property + criterion — appropriate breadth.

---

## Open questions for the architect

1. **C1**: How does the dispatcher actually mutate `SystemMeta::this_run_tick` through `Box<dyn System>`? Add a trait accessor, or move tick state out?
2. **C3**: Formal data-race argument for adjacent `UnsafeCell<u32>` writes across `par_iter` chunks on the same cache line — sound or not? Miri evidence?
3. **W1**: Recompute the 40 MB number using actual `max_components × archetype_count`. Confirm or revise.
4. **W2**: Derive `MAX_CHANGE_AGE` from first principles for boyko's per-frame bump (NOT Bevy's per-system).
5. **W4**: `Commands::spawn` apply tick — does `EcsMaster::create_entity` read self.change_tick, or does the dispatcher thread it? Pick one.
6. **O1**: Is `is_changed()` supposed to return true for a mutation made earlier in the same system? Bevy semantic check.

Once C1-C4 + W1-W8 are resolved, this plan can move to Round 2. The foundation (per-row 8 B ticks, conflict-graph happens-before reuse, parallel array layout) is sound; the integration boundaries need precision.

Relevant source files for the architect to verify:
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\system.rs` (trait surface — no meta accessor)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\function_system.rs` (meta lives on concrete impl)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\system_meta.rs` (current 224 B layout)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\iter.rs` (QueryIter/QueryIterMut fields)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\filter.rs` (Or<F> aggregate_include no-op)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\par_iter.rs` (chunk capture + inline path)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs:244` (create_entity signature)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs` (max_components allocation pattern)
- `D:\claude\BoykoEngine\docs\PHASE-9-PARALLEL-SCHEDULER-PLAN.md` §5.2 (SystemBox / apply_window_drain)