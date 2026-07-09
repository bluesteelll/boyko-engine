> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

All round-3 claims verified against source: `add_observer(kind, cid: ComponentId, ...)` at `ecs_master.rs:2135` (public, `ComponentId`-typed); `query_state.rs` bypasses at `:99` (`iter`, pub), `:247` (`iter_cached`, pub(crate)), `:215`/`:221` (`len`/`is_empty`, pub), `:307` (`matched_ids_mut`, pub(crate)), accessor `matched_ids` at `:227`; `legacy_query.rs` five sites `:152/:166/:184/:204/:214`; `POOL_MAX_ROWS = 16_777_216` (2^24) at `constants.rs:72` with a cfg-gated `262_144` variant at `:77`. The revised plan follows.

---

# Architecture: Phase 22 â€” Tags (static ZST tags, dynamic runtime tags, empty archetype, ZST-pool hazard fix) â€” REVISED (round 4)

*Process note: this session has no Bash tool, so `graphify` could not be executed directly; orientation came from the three research reports plus targeted line-reads (`component_pool.rs:116-172`, `component_registry.rs:196-222,320-364`, `bundle.rs:68-187`, `component.rs:33-56`, `query_view.rs:340-480`, `query_state.rs:99-307`, `legacy_query.rs:47-214`, `ecs_master.rs:1020,2125-2149`, `constants.rs:35-308`, and a full-crate sweep of `matched_ids()` / `matched_archetypes_bitset()` / `iter_cached` consumers). Every developer-agent prompt in the wave breakdown MUST begin with the graphify-first rule (`graphify query/explain/path` before raw file reads) â€” repeated verbatim in each wave brief.*

## Changelog vs round 3

| Critique | Disposition | Where |
|---|---|---|
| **MAJOR W3**: `TagId`'s inner `ComponentId` is `pub(crate)` â€” out-of-crate users cannot reach the id-keyed surfaces (`register_hooks_by_id`, the existing public `EcsMaster::add_observer(kind, cid: ComponentId, ...)` at ecs_master.rs:2135); in-crate tests would mask the gap (Phase-14b "silently unusable" class); id type was inconsistent (`usize` vs `ComponentId`) | **Fixed.** `TagId` gains `#[inline] pub const fn component_id(self) -> ComponentId` + `impl From<TagId> for ComponentId`. `register_hooks_by_id` signature unified to take `ComponentId` (matching `add_observer` â€” verified at :2135-2140). All tag end-to-end tests move to `tests/phase22_tags.rs` (integration tests compile as an EXTERNAL crate â€” `pub(crate)` access is impossible there by construction), including a dedicated reachability test: `register_tag â†’ tag.component_id() â†’ register_hooks_by_id + add_observer â†’ attach â†’ both fire`. The H1 three-case test (W2) lives there too, so it proves reachability, not just semantics. | D3, D8, Public API, Wave 1B/2A, Metrics |
| **MAJOR W4**: the `matched_ids()` rename funnel has three compiler-invisible bypasses â€” `ArchetypeQueryState::iter()`/`iter_cached()` (query_state.rs:99/:247) iterate the field directly, as do `len()`/`is_empty()` (:215/:221) and `matched_ids_mut()` (:307); the "future driver cannot silently bypass" guarantee was false for these paths | **Fixed.** The sweep is extended to EVERY accessor that exposes the matched list: `iter` â†’ `iter_pre_terms` (demoted pub â†’ pub(crate); only consumer is legacy_query + internal :119), `iter_cached` â†’ `iter_cached_pre_terms`, `len`/`is_empty` â†’ `len_pre_terms`/`is_empty_pre_terms` (demoted pub â†’ pub(crate); zero non-test consumers, verified), `matched_ids_mut` â†’ `matched_ids_pre_terms_mut`. Four new rows in the D4 disposition table. The guarantee is restated honestly: outside `query_state.rs`, every read of the matched list now passes through a `_pre_terms`-named symbol; inside `query_state.rs` the private field is owned by the QS1 cache-maintenance code, which is pre-terms by definition. | D4 (table + guarantee wording), Wave 2B |
| MINOR: legacy_query.rs line refs wrong (":142/:165") | **Fixed.** Verified: `matched_ids()` at :152/:184/:204, `iter_cached` at :166/:214 â€” five sites, all mechanical rename, term-free by design (no `with_tag` surface). Table corrected. | D4 table |
| MINOR: query.rs:583 can be pre-classified | **Fixed.** Pre-classified as a test-module sanity assert ("matched_ids contains exactly the two CompA archetypes") â€” mechanical rename row; Wave 2B no longer carries a "MUST classify" unknown. | D4 table |
| MINOR: D7 compile-fail â€” the named const-assert does NOT suppress the impl-level E0277 from `impl Bundle for T`; both diagnostics appear, order not guaranteed across rustc versions | **Fixed.** Acknowledged as unavoidable (supertrait obligations on a concrete impl cannot be silenced). The trybuild `.stderr` snapshot expects BOTH diagnostics; the test's value anchor is the presence of the named symbol `_boyko_component_as_bundle_requires_send_sync_unpin` in the output; snapshot regeneration on toolchain bumps via `TRYBUILD=overwrite` is documented in the test header. | D7, Wave 1C |
| MINOR: VA-budget doc note â€” ZST pool reserves 2Ã—tick_len at POOL_MAX_ROWS | **Fixed.** Quantified: 2^24 rows Ã— 4 B Ã— 2 regions = **128 MiB address space per tag pool per hosting archetype** (verified constants.rs:72; cfg-fallback variant 262_144 â†’ 2 MiB at :77), zero resident until rows commit. Added to D6 trade-off and as a per-archetype VA profile on the book's fragmentation-ceiling page (tags multiply hosting archetypes by design). | D6, Wave 3 docs |

## Rejected remarks

None rejected.

---

## Goal

- **Functional**: (a) `#[derive(Component)] struct Player;` works end-to-end (spawn/insert/remove/query/bundles/hooks/observers); (b) runtime-minted, name-keyed dynamic tags attachable/detachable/queryable without a Rust type; (c) entities may hold zero components (tag-only and empty entities are first-class); (d) the size-0 pool hazard is eliminated structurally, not by assertion.
- **Performance**: zero per-row cost for tag presence checks in queries (archetype-level filtering only); tag storage cost exactly 8 B/row (two ticks, Bevy parity); 0% regression on every existing hot path (named bench gates below); `has_tag` â‰¤ 5 ns (two dependent loads + bit test).

## Context and constraints

- Affected: `ComponentPool` + layout math (`constants.rs`), `ComponentRegistry`, `Archetype::create_entity`, `ComponentPoolBundle`, migration helpers, commands, the entire query driver family (`iter.rs`, `par_iter.rs`, `chunk_iter.rs`, `par_chunk.rs`, `query.rs`, `query_view.rs`, accessors in `query_state.rs`/`state.rs`), both derive macros, hooks registration, `hierarchy/bundles.rs`.
- Invariants preserved: `Archetype` stays 8480 B with `columns[512]` at offset 0 (const-asserted); `ComponentLayout` stays 56 B; `Column` stays 16 B; `ptr.is_null()` remains THE absent-column oracle; Phase-7 single-dependent-load read path untouched; SIMD-A1 (`buffer` multiple of `SIMD_BUFFER_ALIGN=32`) preserved for ZST pools; GROW1-XI proof discipline extended, not weakened; Miri-TB reborrow-confinement (Phase 14a/14b/19) replicated in all new migration code; QS1 dual-structure invariant (matched_ids â†” bitset bijection) untouched â€” tag terms NEVER mutate the shared cache.
- Hard ceilings, stated and accepted: dynamic tags share `MAX_COMPONENTS=512`; tag combinations fragment archetypes within `MAX_ARCHETYPES=1024` (loud failure on overflow; mitigation deferred to enable-bits â€” D10).

## Key decisions

### D1: Tag representation â€” tick-only ZST pool (Bevy model), NOT signature-only

**What**: Components with `size == 0` get a real `ComponentPool` whose reservation contains only the two tick sub-regions (`data_len == 0`); the pool's `buffer` is a provenance-free dangling pointer; the archetype `Column` for a tag is `{ ptr: dangling-aligned-non-null, stride: 0 }`. `Added<Tag>`/`Changed<Tag>` work identically to data components.

**Why**:
- Audit: `Added<C>`/`Changed<C>` resolve `tick_column_base` and silently return `false` forever on a null sentinel (filter.rs:645,701). Signature-only tags make `Added<PlayerTag>` a compile-but-lie â€” the exact bug class this project has been burned by (#56, debug_assert-then-bail). Tick-only pools make tick filters work with **zero changes to filter code**, and `check_ticks` wraparound scanning covers tag pools automatically (they are ordinary pools in `ComponentPoolBundle` â€” no holes).
- `Column.ptr` non-null keeps every `is_null()` presence check (get_component_raw, QD2 guards, fetches) correct without touching the Phase-7 fast path.
- flecs' no-column model needs a column-map indirection (signature index â†’ storage index), breaking the "single dependent load at `arch + c*16`" promise.
- D-cache: tick fill on spawn is a streaming 2Ã—4 B write per row per tag; query-side cost is zero (With/Without are archetype-level; `NEEDS_CHANGE_DETECTION` elision already skips tick code when unused).

**Alternatives rejected**: signature-only presence bit (silent Added/Changed lie + null-deref UB on `&Tag` + new trait machinery to ban data leaves); full data column at stride 0 through the old path (hazard (d) itself).

**Trade-off**: 8 B/row/tag of tick memory + tick-fill on spawn/migration. flecs proves 0 B/row is possible; we consciously pay 8 B for uniform change detection and one code path. Documented in the public book.

### D2: Static tag detection â€” automatic via `size == 0`, no attribute

**What**: No derive changes for detection. Tag-ness is `ComponentLayout.size == 0`, checked where layout decisions are made. Add `#[inline] pub const fn is_zst(&self) -> bool` on `ComponentLayout`. Non-derive ZSTs (manual `Component` impls, `PhantomData` wrappers) take the same path automatically â€” size comes from registration, not the macro.

**Why**: There is no behavioral choice an attribute could express â€” a ZST has no data. flecs needs explicit tags because its ids are typeless; we have the layout. An attribute adds a divergence channel (ZST without `#[component(tag)]` = what?) for zero benefit. (The `#[component(no_bundle)]` attribute of D7 is orthogonal â€” it controls Bundle emission, not storage class.)

**Trade-off**: adding a field to a tag silently changes its storage class â€” which is exactly what should happen.

### D3: Dynamic tag identity â€” shared ComponentId space, sentinel TypeId, process-global name index, fallible mint, public id bridge

**What**:
- Dynamic tags are ordinary `ComponentId`s minted at runtime into the existing 512-slot registry: `size: 0, alignment: 1, drop_fn: None, type_id: TypeId::of::<DynamicTagMarker>(), type_name: <leaked interned name>`, where `enum DynamicTagMarker {}` is private and uninhabited.
- Public identity: `#[repr(transparent)] pub struct TagId(pub(crate) ComponentId);` â€” type-level "filter-only, no data" marker, zero cost. **Public bridge (W3)**: `#[inline] pub const fn component_id(self) -> ComponentId` + `impl From<TagId> for ComponentId`. Rationale: the id-keyed surfaces downstream users need â€” `register_hooks_by_id` (new, D8) and the EXISTING public `EcsMaster::add_observer(kind, cid: ComponentId, runner)` (verified ecs_master.rs:2135) â€” take `ComponentId`; without a public accessor the feature is reachable only from inside the crate, the Phase-14b "silently unusable" class. The bridge is one-way by design (no `ComponentId â†’ TagId` constructor: a `TagId` proves "minted as a size-0 dynamic tag"); data-fetch APIs still cannot accept a bare `TagId`, preserving the type-level filter-only guarantee.
- Name index: process-global `static TAG_NAMES: OnceLock<Mutex<HashMap<Box<str>, TagId>>>` in `component_registry.rs`. Cold path; Mutex+HashMap justified per the Phase-12.5 QueryTypeId-intern precedent; one concrete global avoids the generic-fn-static collapse trap. Names leaked once per unique tag via `Box::leak` (bounded â‰¤512, #53 bounded-leak precedent).
- **Mint protocol (slot-occupied arm specified per O2)** â€” all under the `TAG_NAMES` lock so idempotency + capacity are atomic:
  1. Name present in `TAG_NAMES` â†’ return the existing `TagId` (idempotency lives at the NAME level â€” never at the TypeId level, because every dynamic tag shares `DynamicTagMarker`'s TypeId and register_new's same-TypeId-idempotent arm (component_registry.rs:347) would alias two distinct tag names to one id).
  2. Else mint via crate-internal `try_register_dynamic(layout) -> Option<ComponentId>`: a bounded CAS loop on the shared `NEXT_ID` (`compare_exchange` while `< MAX_COMPONENTS`, `Relaxed` success ordering â€” the OnceLock slot publish remains the synchronization point for layout visibility, unchanged pattern). Returns `None` at the ceiling instead of panicking. Coexistence with typed `register_new`'s `fetch_add` (:333) is sound: a concurrent fetch_add merely makes the CAS retry on the new value; the CAS path never overshoots `MAX_COMPONENTS`; the fetch_add path keeps its release assert unchanged.
  3. `LAYOUTS[id].set(layout)`: the id came from a fresh CAS increment, so the slot MUST be empty. `Err` â‡’ `#[cold]` **panic** (invariant violation â€” reachable only via the test-only `register_layout` slot-pinning escape hatch), **never** return the id, never consult the TypeId-idempotent heuristic.
  4. Insert nameâ†’id into `TAG_NAMES`, release the lock.
- Public surface: `try_register_tag(name) -> Option<TagId>` and `register_tag(name) -> TagId` as panicking sugar with a `#[cold]` message naming the shared 512 budget. **Why fallible-first**: dynamic mints are user-data-driven (names from config/scripts); a panic on the 513th unique tag must be opt-in, not the only mode.

**Why shared id space**: a separate TagId namespace + second bitset would grow `Archetype` past 8480 B (+64 B mask), fork `ArchetypeSignature`, dual-mask `QueryState`, registry block patterns â€” the hottest matching code, for namespace aesthetics. Sharing means **mask build, find_exact_match, EVER_ARCHETYPED, observer-bit seeding, hook flags and migration work unmodified** (audit: `create_archetype` is purely id-driven). Bevy precedent: dynamic components share ComponentId space. Sentinel TypeId keeps `ComponentLayout` at 56 B (`Option<TypeId>` would break the pin); the typed-pool `debug_assert(component_type_id == TypeId::of::<T>())` then correctly REJECTS typed access to dynamic tags â€” desired, free.

**Fragmentation ceiling (explicit)**: N tags over one base archetype can mint up to 2^N archetypes; `MAX_ARCHETYPES=1024` fails loudly via the existing archetype-slab assert. **Accepted limit for Phase 22.** Mitigations: documented churn-ladder guidance (tags = persistent low-frequency state, never per-frame booleans) + future enable-bits (D10). No silent behavior at the ceiling.

**Trade-off**: dynamic tags permanently consume ComponentId slots (write-once registry, no unregistration). 512 shared slots is the documented budget; ids stay first-call-order process-unstable â€” the **name** is the stable serialization key. The public `component_id()` bridge lets users hold raw `ComponentId`s; acceptable â€” `ComponentId` is already a public, pervasive identifier (add_observer precedent), and the one-way conversion preserves every type-level guarantee.

### D4: Query surface â€” typed filters for static tags; runtime archetype-level terms for dynamic tags applied through ONE compiler-enforced funnel; `&Tag` legal

**Static tags**: `With<T>`/`Without<T>` already work (mask-only, `IS_ARCHETYPAL`, audit-verified) â€” zero changes. `&Tag`, `&mut Tag`, `Ref<Tag>`, `Mut<Tag>` are **legal QueryData** (Bevy parity). With D1 they are sound as-is: `fetch.base = column.ptr` is dangling-aligned-non-null; `&*base.add(row*0)` for a ZST is a valid reference; `Mut<Tag>` deref-mut stamps the changed tick. No code change â€” only tests.

**Dynamic tags â€” the term funnel (C1 from round 2, extended per W4)**:

- `with_tag(TagId)` / `without_tag(TagId)` exist on **both `Query<D,F>` (SystemParam) and `QueryView<D,F>` (direct API)** â€” boyko_demo dogfoods `QueryView::for_each_chunk`, so the direct API cannot be term-blind. Terms live in a per-view `TagTerms` (stack-only, no allocation, copy-threaded). The shared interned `QueryState` is NEVER mutated by terms (it is shared across all instances of the `(D,F)` type â€” QS1 stays intact).
- **Enforcement mechanism â€” the compiler enumerates the consumers.** Not just `matched_ids()`: **every accessor that exposes the matched list crosses the funnel** (W4). Renames in `query_state.rs`:
  - `matched_ids()` (:227) â†’ `matched_ids_pre_terms()`
  - `iter()` (:99, pub) â†’ `iter_pre_terms()`, demoted to `pub(crate)` (consumers: internal :119 + legacy_query only)
  - `iter_cached()` (:247, pub(crate)) â†’ `iter_cached_pre_terms()`
  - `len()`/`is_empty()` (:215/:221, pub, zero non-test consumers â€” verified) â†’ `len_pre_terms()`/`is_empty_pre_terms()`, demoted to `pub(crate)`
  - `matched_ids_mut()` (:307, pub(crate)) â†’ `matched_ids_pre_terms_mut()` (cache-maintenance writer; renamed for sweep completeness â€” its return derefs to a readable slice)
  
  Every existing call site fails to compile until Wave 2B classifies it. **Honest scope of the guarantee**: outside `query_state.rs`, every read of the matched list now passes through a `_pre_terms`-named symbol â€” a future driver cannot silently bypass terms without consciously typing `_pre_terms`. Inside `query_state.rs`, the private field is touched only by the QS1 cache-maintenance code, which is pre-terms by definition (the shared cache must stay term-agnostic); a module-header comment pins this boundary. This converts the Phase-14b fire-site-undercount failure mode (enumeration by human memory) into a compile error at the crate-visible boundary.
- **Term test**: `#[inline] fn archetype_passes_tag_terms(terms: &TagTerms, arch: &Archetype) -> bool` â€” â‰¤8 signature-bit tests against the archetype mask; `len == 0` short-circuits with one predicted not-taken branch. Placed at the archetype-TRANSITION point of each driver (outside the row loop, beside the existing stale-id/`entity_count == 0` skips).
- **Full consumer disposition table** (verified sweep; W4 rows added, legacy refs corrected, query.rs:583 pre-classified):

| Consumer | Site | Disposition |
|---|---|---|
| `QueryIter` constructors (readonly + mut) | iter.rs:151, :360 | Store `TagTerms` copy in the iterator (heap-resident cursor per Phase 12.5); apply test at archetype-advance |
| `ParQuery`/`ParQueryMut` distribution loops | par_iter.rs:272, :385 | Apply test in the per-archetype distribution loop (already per-archetype) |
| `for_each_chunk_impl` (shared by `Query::for_each_chunk` query.rs:209 AND `QueryView::for_each_chunk` query_view.rs:464) | chunk_iter.rs:101 | New `terms: &TagTerms` parameter; test at top of archetype loop |
| `par_for_each_chunk_impl` (shared by query.rs:275 and query_view.rs:524) | par_chunk.rs:120 | Same parameter threading |
| `Query::len`-analogue / `is_empty` | query.rs:79, :90 | `terms.len == 0` â†’ current fast path byte-identical; else filter the walk. Semantics preserved: archetype-level membership (today's `is_empty` does not consult `entity_count`; the term-aware path must not either) |
| `QueryView::len` / `is_empty` | query_view.rs:195, :204 | Same as above |
| `QueryView::get` / `get_mut` | query_view.rs:365, :416 (bitset membership) | After the bitset check, term test on `arch_ref` signature (the archetype ref is already in hand â€” â‰¤8 bit tests, len==0 = one predicted branch) |
| `QueryView::single` / `single_mut` | query_view.rs:296, :319 | Verify routing (expected: via iter/get â†’ inherits); behavioral test regardless |
| **`ArchetypeQueryState::iter` / `iter_cached`** (W4) | query_state.rs:99, :247 (+ internal call :119) | Rename `iter_pre_terms`/`iter_cached_pre_terms`; `iter` demoted pub â†’ pub(crate). Consumers are legacy_query (term-free by design, no `with_tag` surface) + the internal :119 delegation. Doc comment: "iterates the raw matched list term-agnostically; reserved for the legacy surface and cache maintenance" |
| **`ArchetypeQueryState::len` / `is_empty`** (W4) | query_state.rs:215, :221 | Rename `len_pre_terms`/`is_empty_pre_terms`, demote pub â†’ pub(crate); zero non-test consumers (verified) â€” tests get mechanical rename |
| **`ArchetypeQueryState::matched_ids_mut`** (W4) | query_state.rs:307 | Rename `matched_ids_pre_terms_mut`; cache-maintenance writer (delta-update), pre-terms by definition; justified comment |
| `query.rs:583` | test module | **Pre-classified**: sanity assert "matched_ids contains exactly the two CompA archetypes" â€” mechanical rename, not a runtime driver |
| Cache maintenance (post-filter, dual-invariant check, delta-update) | state.rs:122, :154; query_state.rs internals | **Pre-terms by design** â€” the shared cache must stay term-agnostic (terms are per-view); justified comment at each site |
| Tests | state.rs:286-341, chunk_iter/par_chunk/iter test modules, query_state.rs test consumers of len/is_empty | Mechanical rename |
| `legacy_query.rs` | `matched_ids()` :152, :184, :204; `iter_cached` :166, :214 â€” **five sites** (corrected) | Mechanical rename to the `_pre_terms` names; term-free by design â€” no `with_tag` surface exists on the legacy API (deliberate, documented) |

- **Alternative rejected**: a `TaggedQuery` wrapper type returned by `with_tag` (compile-time guarantee that only term-aware drivers are callable). Rejected because it duplicates 10+ driver signatures and their SAFETY proofs for the same runtime behavior; the rename-sweep achieves the same completeness with the compiler as the enumerator at zero surface duplication.
- **Cost wording**: with the candidate list a `slice::Iter<ArchetypeId>`, the `len == 0` check costs **one predicted not-taken branch per archetype transition**, outside the row loop â€” NOT zero and NOT per-construction-only. The inner row loop must remain byte-identical (asm-gated); the developer must NOT contort transition code to chase a stricter claim; if criterion is ambiguous, asm decides. >8 terms = loud release assert at term-add time (setup-time, cold).
- Direct presence check: `EcsMaster::has_tag(entity, TagId) -> bool` â€” EntityInland â†’ archetype â†’ signature word â†’ bit test.
- Typed `Added`/`Changed` for dynamic tags: **out of scope** (no type to name). Ticks ARE maintained in dynamic-tag pools (D1 uniformity), so a future `DynAdded(TagId)` term needs no storage change.

**Why archetype-time filtering**: the typed DSL is interned per `(TypeId, TypeId)` with shared cached `QueryState`; per-instance runtime bits must not contaminate shared state â€” archetype-time filtering composes with the cache instead of forking it. Bevy's full `QueryBuilder` (dynamic data access) is more machinery than tags need.

**Trade-off**: dynamic tags are filter-only; no dynamic data fetch. Accepted â€” tags have no data by definition. The extended rename touches ~40 call sites (mostly tests) â€” one-time mechanical cost for permanent structural enforcement at the module boundary.

### D5: Empty archetype â€” lazy, through the normal funnel; row-index bug fixed at the source

1. **No reserved constant, no eager creation.** The empty archetype is `get_or_create_archetype(&[])` â€” created on first demand through the single funnel, cached by `find_exact_match` on the empty mask (audit: registry handles the empty mask under block-pattern 0). Keeps `EcsMaster::new` at its Phase-12.6 lazy budget.
2. **Remove-last-component fix**: `without_component_archetype_id` (migration_helpers.rs:141-154) â€” delete the debug_assert+bail; `kept.is_empty()` â†’ `Some(get_or_create_archetype(&[]))`. Removal of the last component becomes an ordinary migration edge into the root. **The `_dyn` variant (D9) applies the identical rule**: `without_ids_archetype_id` maps `kept.is_empty()` â†’ the empty-archetype id (O3).
3. **Row-index corruption fix (the REAL silent-corruption bug)**: `Archetype::create_entity` (archetype.rs:413-483) takes the row from `self.current_index` BEFORE calling `push_entity_components`, and debug_asserts every pool's returned index equals it (the spawn_at_command.rs:180 pattern). `push_entity_components`' vacuous `unit_index = 0` for zero pools becomes harmless. Behaviorally a no-op for non-empty archetypes.
4. **Spawn**:
   - Direct: `EcsMaster::spawn_empty() -> Entity` = `get_or_create_archetype(&[])` + fixed `create_entity`.
   - Deferred: `Commands::spawn_empty() -> EntityCommands` = `self.spawn(EmptyBundle)` where **`EmptyBundle` is a crate-internal hand-written zero-component Bundle** (boyko_macros is a dev-dependency â€” no derive available in src/; and `derive(Bundle)` correctly keeps its â‰¥1-field rule for users). `EmptyBundle` is a `pub(crate)` unit struct implementing `BundleSealed + Bundle` by hand, mirroring the `hierarchy/bundles.rs` documented pattern: `component_ids() = &[]` (static empty slice, no leak), `static_info` with its own `BundleTypeId` (so the **static bundle cache** caches the empty archetype id â€” warm `spawn_empty` is sub-ns lookup like any bundle), `for_each_component_bytes` = no-op body (**zero unsafe** â€” no bytes to erase).
   - SpawnAtCommand: the guard is the **debug_assert** at spawn_at_command.rs:194-199 (`!pool_ids.is_empty() && len <= MAX_BUNDLE_ARITY`) â€” the release path is unguarded today. Relax it to `pool_ids.len() <= MAX_BUNDLE_ARITY`; the row math at :180 (`row = archetype.current_index`) is already correct; the per-component closure simply runs zero times. Wave 1A verifies `BundleColumnCache::resolve_and_cache` tolerates an empty `pool_ids` (expected: trivially yes â€” empty loop) and adds the entity-registration/`current_index`-bump steps to the test oracle.
5. **Query matching**: no special-casing. The empty archetype's signature is the empty mask; it matches only queries with zero required components (a tag-only or empty entity is invisible to every component query â€” the flecs invariant falls out of subset matching). A test pins this, including `find_matching`'s `include_mask.is_empty()` walk-all behavior.
6. **Despawn**: existing swap-remove over zero pools + `entity_ids` works once (3) lands; regression test: two empty entities, despawn the first, assert the second's identity survives (the exact audited corruption).

**Why lazy**: eager creation buys nothing (no hot path touches it before first use) and costs `EcsMaster::new` time Phase 12.6 just removed.

**Trade-off**: the empty archetype's `ArchetypeId` is not a cross-world constant. No consumer needs it; serialization keys archetypes by component sets.

### D6: ComponentPool for ZSTs â€” constructed, data-less, hazard removed structurally, growth fully specified

All in `component_pool.rs` + `constants.rs`. Verified against the real code.

- **`pool_byte_layout(reserve_rows, stride)`** (constants.rs:162-201): delete the `stride > 0` assert (:166-169). The math then degrades correctly with no further edits: `data_bytes = 0`, `data_len = align_up(0) = 0` (`pool_align_up_granule(0) == 0` â€” verified at :107-112), `added_off = 0`, `changed_off = tick_len`, `os_len = 2*tick_len`. Keep `reserve_rows > 0`. Doc note: for stride==0 the data sub-region is vacuous (`[0,0)`); the two tick regions remain disjoint (`[0, tick_len)`, `[tick_len, 2*tick_len)`) â€” the R1-8 disjointness proof becomes vacuous for data-vs-tick and unchanged for tick-vs-tick.
- **`pool_reserve_rows(stride)`** (constants.rs:120-133): replace the assert with an explicit arm: `stride == 0 â†’ POOL_MAX_ROWS` (rows bounded by ticks only; same ceiling a 1-byte component hits; reservation is address-space only, zero commit).
- **`ComponentPool::new`** (:136-254) â€” **with the O1 ordering fix**:
  - Remove the ZST debug_assert (:147-152). Keep `reserve_rows` asserts (â˜…R1-5, â˜…R1-9) and the align â‰¤ 4096 assert unchanged.
  - **Mandatory derivation order** (the current code binds `let buffer = vm.base()` at :196 and derives tick bases FROM that local at :222-227 â€” a literal per-arm edit of `buffer` would derive tick bases from the dangling pointer = UB at the first `fill_ticks`):
    1. `let base = vm.base();` â€” rename the :196 local.
    2. SIMD-A1 debug_assert (:203-208) checks `base` (the reservation base â€” its â‰¥4096 alignment guarantee is what the assert actually verifies).
    3. Tick bases derive from `base` (`base.add(added_off)`, `base.add(changed_off)`) â€” for ZSTs `added_off == 0`, so `added_base == vm.base()`: sound, the data region is vacuous.
    4. **Only then** set `buffer` per-arm: `stride > 0` â†’ `base` (unchanged behavior); `stride == 0` â†’ `NonNull::new(ptr::without_provenance_mut(SIMD_BUFFER_ALIGN.max(element_align))).unwrap()` â€” dangling, non-null, provenance-free, and a multiple of `SIMD_BUFFER_ALIGN=32` (max of two powers of two is a multiple of both; `element_align â‰¤ 4096` already asserted), so the `buffer_ptr()`/`for_each_chunk` alignment contract holds with zero changes. Add a debug_assert that the dangling buffer satisfies SIMD-A1 too (belt).
  - S-TICKBASE SAFETY comment gains the ZST arm: "for stride==0 the data sub-region is empty; `buffer` is a dangling aligned pointer valid only for zero-size access per the Rust reference; both tick bases derive from `base` (the single live reservation), tick-tick disjointness unchanged."
- **`grow_rows` â€” the CRITICAL fix from round 1, retained.** The existing body (:291-364) is **unreachable-for-ZST by an early branch**; the stride>0 path stays byte-identical (no refactor, 0% risk):

  ```rust
  // after the ceiling (:292) and idempotency (:295) guards, before :299
  if self.component_layout.size() == 0 {
      return self.grow_rows_zst(n);   // #[cold] sibling, tick-driven
  }
  ```

  `grow_rows_zst(n)` â€” tick-region-driven row capacity, **GROW1-ZST proof chain**:
  - **Z1 (driver)**: for stride==0, row capacity is bounded by the tick sub-regions alone; `data_committed` is invariantly 0 forever (debug_assert), `vm.commit` is NEVER called on the data region â€” the vm `new > old` assert and the `:335` division are structurally unreachable.
  - **Z2 (policy)**: reuse `pool_commit_step(self.ticks_committed, needed_t)` where `needed_t = pool_align_up_granule(n * 4)` â€” same request-dominant doubling, applied to the tick byte frontier.
  - **Z3 (in-bounds)**: `n <= reserve_rows â‡’ n*4 <= reserve_rows*4 â‡’ needed_t <= tick_len` (tick_len is a granule multiple). `t_new = (ticks_committed + step).min(tick_len)`, so the commit never overruns the sub-region.
  - **Z4 (strict growth)**: past the guards, `n > committed_rows`. The reserve_rows-clamp case is excluded (`committed_rows == reserve_rows` would contradict `n <= reserve_rows < n`), so `committed_rows = ticks_committed/4`, hence `n*4 > ticks_committed`, hence `needed_t >= n*4 > ticks_committed`, and with Z3, `t_new >= needed_t > ticks_committed` â€” both tick `vm.commit(added_off + old, added_off + new)` / `(changed_off + old, changed_off + new)` calls satisfy the vm `new > old` assert.
  - **Z5 (sufficiency)**: `committed_rows' = (t_new / 4).min(reserve_rows) >= n` since `t_new >= needed_t >= n*4` and `n <= reserve_rows`; debug_assert it (GROW1-XI step-3 analogue).
  - **Z6 (panic coherence)**: `ticks_committed` and `committed_rows` are written only after both commits succeed (â˜…Q6 pattern preserved).
- **`row_ptr`** (:374+): code unchanged; doc + SAFETY gain the arm: stride==0 â‡’ all rows return the dangling base â€” valid because only zero-size reads/writes/drops ever go through it.
- **`swap_remove`** (:524-591): code unchanged (a 0-byte `copy_nonoverlapping` between equal dangling pointers is allowed); SAFETY rewritten: "stride==0 â‡’ zero-byte copy, trivially non-overlapping; stride>0 â‡’ distinct rows as before." Tick swap is row-indexed, valid verbatim.
- **drop loop**: a ZST with a Drop impl is legal (`needs_drop` true): `drop_in_place` at the dangling base once per logical row reads no bytes â€” sound; dedicated test.
- **Hazard (d) disposition**: the audit corrected the prompt â€” release builds currently PANIC in `constants.rs` (release-active asserts), they do not alias; the debug-only assert at component_pool.rs:147 was the divergence. After this step size 0 is a valid, distinct layout: neither panic nor aliasing is reachable. The pinned `#[should_panic("does not support zero-sized components")]` test (`tests/drop_fn.rs:559-571`) is retired and replaced by positive ZST coverage.

**Why constructed (vs never-constructed presence-only)**: required by D1 (ticks must live where `tick_column_base` finds them); keeps `ComponentPoolBundle` lock-step row accounting uniform (no holes in `sparse_indexes`); makes `Column.ptr` non-null for free.

**Trade-off â€” quantified VA budget (round-4 fix)**: each ZST pool reserves `2 Ã— tick_len` of address space at the `POOL_MAX_ROWS` ceiling: 2^24 rows Ã— 4 B Ã— 2 regions = **128 MiB of address space per tag pool PER hosting archetype** (constants.rs:72; the cfg-fallback variant `POOL_MAX_ROWS = 262_144` at :77 yields 2 MiB under Miri/fallback) â€” zero resident cost until rows commit. Same class as a 1-byte data pool, BUT tags multiply hosting-archetype count by design (fragmentation), so the per-archetype VA profile goes on the book's fragmentation-ceiling page: at the `MAX_ARCHETYPES=1024` ceiling with every archetype hosting one tag, worst-case VA reservation is 128 GiB of a 128 TiB user VA space â€” bounded and documented, not a practical limit, but stated so nobody discovers it from a VM-commit monitor.

### D7: Bundles â€” derive(Component) emits a single-component Bundle with a readable bound diagnostic and an opt-out; in-crate components get the hand-written mirror; arity 8 â†’ 16

**The W1 bound problem, resolved**: `Bundle: BundleSealed + Send + Sync + Unpin + 'static` (verified, bundle.rs:183) vs `Component: 'static + Sized` (verified, component.rs:33). Emitting `impl Bundle for T` from derive(Component) imposes the supertrait obligations on every derived type. A conditional `impl Bundle for T where T: Send + Sync + Unpin` does NOT solve it â€” where-clauses on impls for concrete self-types are checked eagerly (E0277 at the impl site), not lazily; the derive would still hard-error. **Decision**:

- Default: derive(Component) emits (in this order) **(1)** a named const-assert block placed BEFORE the impls so its readable error leads the diagnostics:
  ```rust
  const _: () = {
      // Single-component bundle emission requires Send + Sync + Unpin.
      // Opt out with #[component(no_bundle)] for intentionally exotic types.
      const fn _boyko_component_as_bundle_requires_send_sync_unpin<T: Send + Sync + Unpin>() {}
      _boyko_component_as_bundle_requires_send_sync_unpin::<#ty>;
  };
  ```
  **(2)** `impl BundleSealed for T {}` + `impl Bundle for T` (single-component bundle): per-type concrete `static INFO: OnceLock<BundleStaticInfo>` with a 1-element id slice, `cached_archetype_id` delegating to `bundle_archetype_id_for::<Self>`, `for_each_component_bytes` erasing `self` via `ManuallyDrop` + `from_raw_parts(&raw const *md as *const u8, size_of::<T>())` (valid 0-len slice for ZSTs â€” audit-verified). Gives `commands.spawn(PlayerTag)`, `.insert(PlayerTag)`, `.insert(Velocity { .. })` for every derived component. Per-type statics in derive output are concrete â€” sidesteps the Phase-12.5 generic-fn-static collapse trap that a blanket impl would invite.
- **Diagnostic reality (round-4 fix)**: the named const-assert does NOT suppress the impl-level E0277 arising from `impl Bundle for T`'s supertrait obligations â€” for a `!Send` type, BOTH diagnostics appear, and their relative order is not guaranteed across rustc versions. This is unavoidable (supertrait obligations on a concrete impl cannot be silenced); the const-assert's job is to guarantee that AT LEAST ONE readable, named, comment-bearing diagnostic is present, not to be the sole error. The trybuild `.stderr` snapshot therefore expects **both** diagnostics; the test's load-bearing anchor is the named symbol `_boyko_component_as_bundle_requires_send_sync_unpin` appearing in the output; the test header documents `TRYBUILD=overwrite` snapshot regeneration as the standard procedure on toolchain bumps (accepted, bounded brittleness â€” snapshot-based compile-fail tests are inherently toolchain-coupled).
- **Opt-out**: `#[component(no_bundle)]` on the existing derive-attribute namespace (Phase-14a `#[component(...)]` precedent) suppresses BOTH the const-assert and the Bundle/BundleSealed emission â€” derive(Component) remains usable for `!Send`/`!Sync`/`!Unpin` types (storable via the type-erased direct API, as today); such a type simply isn't spawnable as a bare bundle (wrap it in a derive(Bundle) struct or use the direct API).
- **Contract documentation + tests**: the derive docs and the book state the tightening and the opt-out; the compile-fail test pins the diagnostics as above (an `Rc`-bearing component without the attribute); a positive test pins that `#[component(no_bundle)]` compiles and the type still works as a component. **Audit task (Wave 1C)**: grep boyko_ecs tests + boyko_demo for derive(Component) types containing `Rc`/`RefCell`/`PhantomPinned`/raw-pointer fields (expected: none â€” all conform).
- `Component` trait formally **unchanged** (`'static + Sized`) â€” formally bounding it `Send + Sync` would break the supported type-erased storage of exotic components for no Phase-22 need.

**In-crate reality** (verified at `hierarchy/bundles.rs:1-97`): `boyko_macros` is a **dev-dependency** of `boyko_ecs` â€” the derive is unavailable to library `src/` code; the Phase-19 impls live on separate newtypes `ChildOfBundle`/`ChildrenBundle`. Therefore Wave 1C: (i) add `impl_self_bundle!($ty)` to `hierarchy/bundles.rs` (or a sibling `bundle/self_bundle.rs`) â€” a mechanical mirror of the NEW derive(Component) Bundle emission where the whole `self` is the component, keeping the file's documented SAFETY-accounting note (bundles.rs:19-28: reproduced derive output, not novel unsafe); (ii) `impl_self_bundle!(ChildOf); impl_self_bundle!(Children);`; (iii) **delete** `ChildOfBundle`/`ChildrenBundle` and the `impl_single_field_bundle!` macro, migrating every call site; (iv) `EmptyBundle` (D5) lives beside them with the same hand-written pattern (no unsafe at all). First action of Wave 1C remains: read `bundle/bundle.rs` to pin the exact trait surface before writing the macro emission.

**Coherence**: a downstream type deriving BOTH `Component` and `Bundle` is now a duplicate-impl compile error unless `#[component(no_bundle)]` is applied (documented escape hatch; compile-fail test pins the conflict message). Wave 1C greps tests + demo for double-derives and fixes them (expected: none).

`derive(Bundle)` keeps the â‰¥1-field requirement; the compile_error! (boyko_macros/src/lib.rs:897-910) message now points at `spawn_empty()`.

`MAX_BUNDLE_ARITY` raised **8 â†’ 16** in lock-step: migration_helpers.rs:54, the spawn_at_command debug_assert (:194-199), and the derive arity rejection. Stack cost: 128 B + 16 B â€” irrelevant. Tags make wide bundles the norm; 8 was audit-flagged.

Static bundle cache: no structural changes â€” tag-bearing and single-component bundles resolve through the same `BundleTypeId â†’ OnceLock<ArchetypeId>` path (documented: the 1024-slot budget now also feeds single-component spawns). `spawn_batch`: no special-casing â€” a tag column's batch copy is one 0-byte memcpy; tick fill proceeds normally. `phase12_5_spawn_batch` gates regression; an explicit `stride != 0` skip is a measured follow-up only.

**Trade-off**: deriving Component reserves the type's bundle identity by default â€” a type can't be both a component and a multi-field bundle without the opt-out. The compile error plus a named escape hatch is the feature.

### D8: Hooks/observers on tags â€” fully uniform; id-keyed hook registration with the H1 staleness gate and a reachable public bridge

- All 4 hook kinds + observers fire for static tags exactly as for data components. Zero dispatch changes: `HookContext`/`ObserverContext` carry only `{ entity, component_id }` (audit-verified â€” no data pointer exists to be invalid). The flecs OnSet-downgrade problem cannot arise: our hook ABI never had a data pointer.
- Re-inserting a present tag (in-place fast path) fires on_replace + on_insert and stamps the changed tick â€” uniform with data replace semantics. Documented.
- Dynamic tags: observers are already id-keyed AND publicly reachable â€” `EcsMaster::add_observer(kind, cid: ComponentId, runner)` (ecs_master.rs:2135) accepts `tag.component_id()` directly (W3 bridge, D3). Hooks gain one entry point: `register_hooks_by_id(component_id: ComponentId, hooks) -> Result<..>` â€” **signature unified on `ComponentId`** (round-4 fix; was `usize` â€” `add_observer` is the precedent and the registry's public id vocabulary). The `HOOKS` table is already id-indexed `[OnceLock; 512]`; typed `register_hooks::<C>` becomes a thin wrapper. Write-once semantics unchanged.
- **H1 staleness gate (the W2 fix)**: `ArchetypeFlags` hook bits are OR-computed once at archetype creation; an archetype created before hook registration never re-checks. The Phase-21 H1 gate (`EVER_ARCHETYPED` staleness check) therefore applies to the id-keyed path **identically to the typed path**: `register_hooks_by_id` returns `Err` if `was_ever_archetyped(component_id)` is already set. Without this, the NATURAL dynamic-tag call order â€” `register_tag` â†’ `add_tag` (archetype created) â†’ `register_hooks_by_id` â€” would compile, succeed, and silently never fire: the compile-but-lie class. **Contract, documented on `register_tag`, `register_hooks_by_id`, and in the book**: *mint â†’ register hooks â†’ first attach*.
- **Reachability proof (W3)**: the H1 three-case test AND a dedicated bridge test live in `tests/phase22_tags.rs` â€” an integration test, compiled as an **external crate**, where `pub(crate)` access is impossible by construction. The bridge test chains the full public path: `register_tag("enemy")` â†’ `tag.component_id()` â†’ `register_hooks_by_id(...)` + `add_observer(ObserverKind::Add, tag.component_id(), ...)` â†’ `add_tag` â†’ assert hook AND observer fired. Three-case H1 test: (i) mintâ†’registerâ†’attach â‡’ hook fires; (ii) mintâ†’attachâ†’register â‡’ `Err` (message names the contract); (iii) hook bits present in the flags of an archetype created after registration.

**Why uniform**: one code path, zero new branches at the structural fire sites; the `phase14a_hooks_gate` 0%-bench stays valid byte-for-byte when no hooks are registered.

### D9: Migration â€” tag attach/detach via existing machinery; allocation-free dynamic variant

- Static tag insert/remove: **no new code** â€” `merged_archetype_id::<B>` / `without_component_archetype_id::<C>` + `migrate_entity_insert/remove` handle them; the tag's own column copy is a 0-byte memcpy; retained data columns pay the normal row-move. In-place fast path (source == target) covers re-insert (D8).
- Dynamic tags need a non-generic path (no `B: Bundle`, no `C: Component`):
  - `merged_archetype_id_dyn(world, source_id, extra: &[ComponentId]) -> ArchetypeId` and `without_ids_archetype_id(world, source_id, removed: &[ComponentId]) -> ArchetypeId` â€” union/difference + canonical sort + `get_or_create_archetype`. **`kept.is_empty()` â†’ the empty-archetype id** (the D5(2) rule, mirrored â€” O3); the absent-tag no-op is decided BEFORE calling (presence test on the source signature). **Allocation-free**: built on stack arrays `[ComponentId; MAX_MIGRATION_COLUMNS]` (4 KB, existing precedent at migration_helpers.rs:536-539) â€” the generic versions keep their `to_vec()`/`kept: Vec` (untouched, 0% risk; their cleanup is out of scope).
  - `migrate_entity_attach_ids(world, entity, source_id, target_id, added: &[ComponentId])` â€” structurally a clone of `migrate_entity_insert` minus the bundle-byte-write step: retained-column copy, tick init for newly-added ids (asserts `is_zst` for every added id â€” tag-only, minimal unsafe), EntityInland repoint hoisted into the Phase-1 block, hooks/observers fired in Phase 2 with no live archetype reborrow â€” **Phase-14a Â§3.4 confinement replicated verbatim**. **The zero-retained shape is first-class**: attach FROM the empty archetype (source has zero pools, zero retained columns â€” the retained-copy loop runs zero times; `move_out_entity` over a pool-less archetype) is an explicit test + Miri-TB case (O3). `migrate_entity_detach_ids` mirrors removal (drop_fn called uniformly when present â€” covers Drop-impl ZSTs), including detach-to-empty.
  - Public API: `EcsMaster::add_tag/remove_tag` (direct, `&mut self`); `EntityCommands::add_tag/remove_tag` via `AddTagCommand`/`RemoveTagCommand` POD commands (id payload, no type erasure). Absent-tag remove = no-op; present-tag add = in-place replace semantics; dead entity (generation mismatch) = no-op, matching existing commands.
- No flecs-style "free toggle": a tag flip still moves the whole row. Stated in docs as the churn ladder ("tags are free to carry, not free to toggle"); non-fragmenting alternative is D10 future work.

**Why no memcpy-skip branch**: a 0-byte memcpy costs a few cycles in a `#[cold]` migration; `if stride != 0` branches in the column loop are unmeasurable noise. Cleanliness wins; revisit only with profile evidence.

### D10: Forward-compatibility â€” doors explicitly left open

- **Entity-handle width**: tags never touch `Entity`/`EntityInland` layout; `TagId` wraps `ComponentId` (usize), orthogonal to a future u32+generation repack.
- **flecs-style pairs**: by REJECTING tags-as-entities unification (flecs' own O(1) path is bounded at id 256; full unification needs 64-bit encoded ids + hashmap graph edges â€” incompatible with inline `columns[512]`), we add no constraint binding tag ids to entity ids. The name-keyed registry generalizes to pairs (composite names); `TagId` is the seam where a future `PairId` lands. The one-way `TagId â†’ ComponentId` bridge (D3) does not close this door: a future `PairId` gets its own one-way bridge into whatever id space pairs land in.
- **Enable-bits / non-fragmenting storage**: tick-only pools + signature-bit presence don't preclude a per-archetype enable mask; `Column._reserved: u32` remains an untouched layout-stable seam (explicitly NOT used by this phase). Fragmentation ceiling + churn-ladder docs set expectations until then. The D4 term funnel is also the seam where an enable-mask test would slot in (same transition point).

## Data structures

```rust
// component_registry.rs â€” NEW
/// Private uninhabited sentinel: the TypeId of every dynamic tag. Can never
/// collide with a user type; typed-pool debug guards therefore correctly
/// reject typed access to dynamic-tag ids.
enum DynamicTagMarker {}

/// Process-global name->id intern. COLD: mint/lookup only (setup time).
/// Mutex+HashMap per the Phase-12.5 QueryTypeId-intern precedent; a single
/// concrete global avoids the generic-fn-static collapse trap. Capacity +
/// idempotency are atomic under this lock; idempotency is NAME-keyed, never
/// TypeId-keyed (all dynamic tags share DynamicTagMarker's TypeId â€” O2).
static TAG_NAMES: OnceLock<Mutex<HashMap<Box<str>, TagId>>> = OnceLock::new();

/// Crate-internal fallible mint: bounded CAS on NEXT_ID (< MAX_COMPONENTS),
/// None at the ceiling â€” does NOT inherit register_new's release assert.
/// Slot-occupied on a fresh CAS-minted id = invariant violation: #[cold]
/// panic, NEVER the register_new same-TypeId idempotent return (O2).
pub(crate) fn try_register_dynamic(layout: ComponentLayout) -> Option<ComponentId>;

// identifiers â€” NEW
/// Filter-only dynamic tag handle. repr(transparent) over ComponentId:
/// zero-cost, type-distinct so data-fetch APIs cannot accept it.
/// The bridge is ONE-WAY (W3): TagId -> ComponentId is public (the id-keyed
/// hook/observer surfaces take ComponentId); ComponentId -> TagId has no
/// constructor (a TagId proves "minted as a size-0 dynamic tag").
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TagId(pub(crate) ComponentId);

impl TagId {
    #[inline] pub const fn component_id(self) -> ComponentId;
}
impl From<TagId> for ComponentId { /* = component_id() */ }

// query module â€” NEW, inline, stack-only, threaded by &/copy through drivers
/// Runtime archetype-level tag terms. len==0 costs one predicted not-taken
/// branch per ARCHETYPE TRANSITION (outside the row loop); the inner row
/// loop must remain byte-identical (asm-gated). Never per-row.
/// Carried by BOTH Query<D,F> and QueryView<D,F>; never stored in the
/// shared interned QueryState (QS1 stays term-agnostic).
struct TagTerms {
    ids: [TagId; MAX_DYN_TAG_TERMS],  // MAX_DYN_TAG_TERMS = 8
    polarity: u8,                     // bit i: 1 = with, 0 = without
    len: u8,
}

/// THE single term test â€” every matched-list driver calls this at its
/// archetype-transition point. Enforced by the _pre_terms rename sweep over
/// ALL matched-list accessors: matched_ids, iter, iter_cached, len,
/// is_empty, matched_ids_mut (W4) â€” un-migrated consumers fail to compile.
#[inline]
fn archetype_passes_tag_terms(terms: &TagTerms, arch: &Archetype) -> bool;

// ComponentLayout â€” UNCHANGED 56 B pin; NEW helpers:
impl ComponentLayout {
    pub const fn is_zst(&self) -> bool;                 // size == 0
    pub fn new_dynamic_tag(name: &'static str) -> Self; // size 0, align 1,
        // drop_fn None, type_id = TypeId::of::<DynamicTagMarker>()
}

// ComponentPool â€” UNCHANGED fields; per-arm semantics (O1 ordering!):
//   new(): base = vm.base() FIRST; SIMD-A1 assert on base; tick bases from
//   base; buffer per-arm LAST (stride==0 -> dangling at
//   SIMD_BUFFER_ALIGN.max(align), non-null, provenance-free).
//   stride == 0: data_committed == 0 forever; committed_rows driven by
//   ticks_committed (GROW1-ZST, Z1-Z6).
#[cold] #[inline(never)]
fn grow_rows_zst(&mut self, n: usize) -> bool;

// bundle â€” NEW, crate-internal, hand-written (macro is dev-only):
/// Zero-component bundle backing Commands::spawn_empty. No unsafe:
/// for_each_component_bytes is a no-op; component_ids() = &[].
pub(crate) struct EmptyBundle;

// hierarchy/bundles.rs â€” REPLACED macro:
/// impl_self_bundle!(T): Bundle where the whole `self` is the component â€”
/// the exact mirror of the new derive(Component) Bundle emission.
/// Applied to ChildOf, Children; ChildOfBundle/ChildrenBundle DELETED.
```

## Public API

```rust
// TagId â€” public bridge to the id-keyed surfaces (W3)
impl TagId {
    pub const fn component_id(self) -> ComponentId;
}
impl From<TagId> for ComponentId;

// EcsMaster
pub fn spawn_empty(&mut self) -> Entity;
pub fn try_register_tag(&mut self, name: &str) -> Option<TagId>; // None = 512 budget exhausted
pub fn register_tag(&mut self, name: &str) -> TagId;             // panicking sugar, #[cold] msg
pub fn tag_by_name(&self, name: &str) -> Option<TagId>;          // cold lookup
pub fn has_tag(&self, entity: Entity, tag: TagId) -> bool;       // hot-capable O(1)
pub fn add_tag(&mut self, entity: Entity, tag: TagId);           // direct, migrating
pub fn remove_tag(&mut self, entity: Entity, tag: TagId);        // absent = no-op
// EXISTING, now tag-reachable via tag.component_id() (verified :2135):
// pub fn add_observer(&mut self, kind: ObserverKind, cid: ComponentId, runner: ObserverFn) -> ObserverId;

// Commands / EntityCommands
pub fn spawn_empty(&mut self) -> EntityCommands<'_, 's>;  // = spawn(EmptyBundle)
impl EntityCommands<'_, '_> {
    pub fn add_tag(&mut self, tag: TagId) -> &mut Self;
    pub fn remove_tag(&mut self, tag: TagId) -> &mut Self;
}

// Query (SystemParam) AND QueryView (direct API) â€” both carry TagTerms:
impl<D: QueryData, F: QueryFilter> Query<'_, D, F> {
    pub fn with_tag(self, tag: TagId) -> Self;     // archetype-level term
    pub fn without_tag(self, tag: TagId) -> Self;  // loud assert past 8 terms
}
impl<D: QueryData, F: QueryFilter> QueryView<'_, D, F> {
    pub fn with_tag(self, tag: TagId) -> Self;
    pub fn without_tag(self, tag: TagId) -> Self;
}
// Terms are honored by EVERY driver: iter/iter_mut, par_iter/par_iter_mut,
// for_each_chunk, par_for_each_chunk, len/is_empty, get/get_mut,
// single/single_mut â€” enforced by the _pre_terms rename sweep (D4 table).

// component_registry (crate-public)
/// Err if hooks for this id were already registered OR the id was ever
/// archetyped (Phase-21 H1 staleness gate â€” register hooks BEFORE the
/// tag's first attach). Typed register_hooks::<C>() becomes a thin wrapper.
/// Takes ComponentId (NOT usize) â€” unified with add_observer's vocabulary (W3).
pub fn register_hooks_by_id(component_id: ComponentId, hooks: ComponentHooks) -> Result<(), HooksError>;

// derive(Component): additionally emits the named bound const-assert +
// impl BundleSealed + Bundle for T; both suppressed by #[component(no_bundle)].
// derive(Bundle): unchanged except arity 8 -> 16 and the zero-field message
// pointing at spawn_empty().
```

## Algorithms for critical paths

| Operation | Steps | Big-O | Cache | Branches | SIMD |
|---|---|---|---|---|---|
| `has_tag` | inland load â†’ archetype ptr â†’ signature word â†’ bit test | O(1) | 2-3 dependent loads (random) | 1 generation check | n/a |
| With/Without static tag | existing mask subset test at QueryState build | O(archetypes), cached | sequential bitset | none new | existing 512-bit ops |
| Term test (all drivers) | â‰¤8 signature bit tests per candidate archetype at transition | O(matched Ã— len) | archetype header, already touched | predicted; len==0 = 1 branch/transition | no |
| `get`/`get_mut` with terms | existing bitset membership + term test on in-hand archetype ref | O(1) + O(len) | no new lines touched | len==0 = 1 predicted branch | n/a |
| Tag spawn (per row per tag) | tick pair write (8 B) | O(1) | streaming | none | fill loops vectorize |
| Tag attach (dynamic) | stack union â†’ get_or_create â†’ row move + tick init + hooks | O(retained bytes) | streaming copy | `#[cold]` path | memcpy |
| Tag attach from EMPTY | zero retained columns â†’ move_out + register + tick init | O(added ids) | trivial | `#[cold]` | n/a |
| ZST pool `add` | len++ + tick write (0-byte copy elided) | O(1) | tick region sequential | none new | n/a |
| ZST pool grow | tick-frontier doubling, 2 vm commits | O(1) amortized | cold | `#[cold]` | n/a |

## Multithreading model

- No new shared mutable state on the hot path. `TAG_NAMES` (Mutex) and registry OnceLock slots are setup-time/cold; registry reads stay a single acquire-load. `try_register_dynamic`'s CAS on `NEXT_ID` is bounded and lock-free; the OnceLock slot publish remains the synchronization point for layout visibility (unchanged pattern); CAS coexists correctly with `register_new`'s `fetch_add` (retry-on-change, never overshoots).
- All migration/attach/detach paths run under `&mut EcsMaster` (structural window or apply-window barrier) â€” same model as every structural op; the `running == 0` apply-window invariant (Phase 9/16) untouched.
- `TagTerms` is stack-/view-local; `par_iter`/`par_for_each_chunk` apply the same archetype-level term test when distributing chunks (the distribution loop is single-threaded dispatcher code; workers receive already-filtered chunks) â€” no per-row synchronization, no worker-visible term state.
- `TagId: Send + Sync + Copy`. `DynamicTagMarker` uninhabited.
- Data-race freedom: identical proof shape to the existing system â€” structural mutation only at `&mut` exclusive points; queries read archetype signatures frozen during system execution.

## Integration

- **Modified**: `constants.rs` (layout fns, ZST arms), `component_pool.rs` (ZST constructor arm with O1 ordering, `grow_rows` early branch + `grow_rows_zst`, SAFETY rewrites), `component_registry.rs` (dynamic mint, name index, `register_hooks_by_id(ComponentId, ..)` + H1 gate, `is_zst`), `archetype.rs::create_entity` (row from `current_index`), `component_pool_bundle.rs` (doc + debug_assert tightening), `migration_helpers.rs` (remove-last fix, `_dyn` pair incl. empty-target arm, arity 16), `spawn_at_command.rs` (debug_assert relaxation :194-199, arity 16), commands module (`spawn_empty`, `Add/RemoveTagCommand`, EntityCommands methods), **query driver family** â€” `query_state.rs` (FULL accessor sweep: `matched_ids`/`iter`/`iter_cached`/`len`/`is_empty`/`matched_ids_mut` â†’ `_pre_terms` names, visibility demotions, module-boundary comment), `state.rs`, `iter.rs`, `par_iter.rs`, `chunk_iter.rs`, `par_chunk.rs`, `query.rs`, `query_view.rs` (TagTerms threading per the D4 table), `legacy_query.rs` (five mechanical renames :152/:166/:184/:204/:214), `boyko_macros/src/lib.rs` (Bundle emission + const-assert + `no_bundle` attr, arity 16, zero-field message), `hierarchy/bundles.rs` (impl_self_bundle!, newtype deletion, call-site migration), `tests/drop_fn.rs` (retire should_panic).
- **Created**: `tag_id.rs` (identifiers, incl. the public bridge), `EmptyBundle` (bundle module), tag command file (or folded into existing), `tests/phase22_tags.rs` (out-of-crate integration suite â€” W3 reachability lives here), tests + benches below.
- **Unchanged, verified compatible**: `Archetype` 8480 B pin, `Column` 16 B, `ComponentLayout` 56 B, QueryState interning + QS1 dual invariant, static bundle cache structure, hooks gate (bit-test), scheduler, events, vm.rs (its `new > old` assert is satisfied by Z4, never gated), `add_observer` public signature (:2135 â€” gains tag reachability with zero changes).

## Implementation plan (numbered Steps â†’ waves)

*Every wave brief starts with: "MANDATORY: run `graphify query/explain/path` to orient before reading raw sources; read raw files only to modify/debug specific lines."*

**Wave 0 â€” foundation (single developer, serial)**
1. `constants.rs`: delete the two `stride > 0` asserts; `pool_reserve_rows` ZST arm (â†’ `POOL_MAX_ROWS`); doc the vacuous-data-region layout + the 128 MiB-VA-per-pool note; unit tests for `stride == 0` layout math (`data_len==0`, `added_off==0`, `os_len==2*tick_len`, `align_up(0)==0` pin).
2. `component_pool.rs`: remove the :147-152 debug_assert; **O1 ordering verbatim**: rename the :196 local to `base = vm.base()`, SIMD-A1 assert against `base`, tick bases (:222-227) derived from `base`, set `buffer` per-arm LAST (stride==0 â†’ dangling at `SIMD_BUFFER_ALIGN.max(align)`); `grow_rows` early ZST branch + `grow_rows_zst` with the Z1â€“Z6 proof chain as written SAFETY/debug_asserts (incl. `data_committed == 0` invariant assert); SAFETY rewrites (S-TICKBASE ZST arm, swap_remove, row_ptr, drop loop); `ComponentLayout::is_zst`.
3. Retire `tests/drop_fn.rs:559-571`; positive ZST pool unit tests: add/swap_remove/pop/tick stamping/Drop-impl-ZST teardown/**growth**: at least two successive `grow_rows_zst` invocations that each reach `vm.commit` (drive `n` past the first commit step), asserting strict `ticks_committed` frontier growth, `committed_rows >= n`, and `data_committed == 0` throughout.

**Wave 1 â€” three parallel developers (disjoint files)**
4. *(A â€” empty archetype)* `archetype.rs::create_entity` row-from-`current_index` fix + pool-agreement debug_asserts; `migration_helpers.rs:141-154` remove-last fix; `spawn_at_command.rs` debug_assert relaxation (empty `pool_ids` legal); verify `BundleColumnCache::resolve_and_cache` on zero components; `EcsMaster::spawn_empty`; two-empty-entities despawn-identity regression test; remove-lastâ†’emptyâ†’re-insert round trip.
5. *(B â€” registry)* `TagId` **with the public `component_id()` bridge + `From` impl (W3)**; `DynamicTagMarker`; `ComponentLayout::new_dynamic_tag`; `try_register_dynamic` (bounded CAS; slot-occupied â‡’ `#[cold]` panic, never TypeId-idempotent return â€” O2); `TAG_NAMES` intern (NAME-keyed idempotency, capacity-atomic, bounded leak); `try_register_tag`/`register_tag`/`tag_by_name` on EcsMaster; `register_hooks_by_id(ComponentId, ..)` **with the H1 `was_ever_archetyped` gate (W2)** + typed wrapper refactor; tests IN `tests/phase22_tags.rs` (out-of-crate): exhaustion (mint to ceiling â†’ `None`, panicking variant message pin), two-names-never-alias, **H1 three-case**, **W3 reachability chain** (register_tag â†’ component_id() â†’ register_hooks_by_id + add_observer â†’ attach â†’ both fire).
6. *(C â€” bundles/macros)* FIRST ACTION: read `bundle/bundle.rs` to pin the trait surface. Then: derive(Component) emits const-assert + single-component Bundle impl, `#[component(no_bundle)]` opt-out (W1); `impl_self_bundle!` in-crate mirror; `impl Bundle for ChildOf/Children`; DELETE `ChildOfBundle`/`ChildrenBundle` + migrate call sites; `EmptyBundle` + `Commands::spawn_empty`; arity 8â†’16 in macro + `MAX_BUNDLE_ARITY` consts; zero-field Bundle message; compile-fail tests (Component+Bundle double derive; `Rc`-bearing component without `no_bundle` â€” **snapshot expects BOTH the named const-assert E0277 and the impl-level supertrait E0277; anchor = the named symbol; `TRYBUILD=overwrite` procedure in the test header**); positive `no_bundle` test; audit grep (tests + demo) for double-derives and !Send/!Sync/!Unpin components.

**Wave 2 â€” three parallel developers (after Wave 1)**
7. *(A â€” dynamic migration)* `merged_archetype_id_dyn`/`without_ids_archetype_id` (stack arrays, allocation-free; **`kept.is_empty()` â†’ empty archetype â€” O3**); `migrate_entity_attach_ids`/`detach_ids` with Phase-14a Â§3.4 confinement, **including the zero-retained attach-FROM-empty shape**; `add_tag`/`remove_tag` direct; `AddTagCommand`/`RemoveTagCommand` + EntityCommands methods; hooks/observers fired at the new sites â€” **count against the 7-site ledger (Phase-14b lesson)**; tests (in `tests/phase22_tags.rs`): spawn_emptyâ†’add_tagâ†’query-visible, remove_tagâ†’back-to-empty, full emptyâ†”tagged round trip (direct AND deferred), dead-entity no-ops.
8. *(B â€” query term funnel)* Execute the FULL `_pre_terms` sweep per the extended D4 table: rename `matched_ids` (:227), `iter` (:99, demote pub(crate)), `iter_cached` (:247), `len`/`is_empty` (:215/:221, demote pub(crate)), `matched_ids_mut` (:307); add the query_state.rs module-boundary comment (field owned by QS1 cache-maintenance only); let the compiler surface every consumer; migrate per the table: thread `&TagTerms` through `for_each_chunk_impl` (chunk_iter.rs:101), `par_for_each_chunk_impl` (par_chunk.rs:120), both `QueryIter` constructors (iter.rs:151,:360), both par distribution loops (par_iter.rs:272,:385); term test in `len`/`is_empty` (query.rs:79,:90; query_view.rs:195,:204) and `get`/`get_mut` (query_view.rs:365,:416); verify `single`/`single_mut` routing; mechanical renames in legacy_query.rs (:152/:166/:184/:204/:214 â€” five sites) and query.rs:583 (test sanity assert); justify cache-maintenance sites (state.rs:122,:154) as pre-terms-by-design with comments; `with_tag`/`without_tag` on Query AND QueryView; `archetype_passes_tag_terms`; `has_tag`; **one behavioral test per driver** (iter, iter_mut, par_iter, par_iter_mut, Query::for_each_chunk, QueryView::for_each_chunk, both par_for_each_chunk, is_empty/len, get, get_mut, single) on a two-archetype tagged/untagged fixture; asm check: inner row loop byte-identical, transition adds one predicted branch.
9. *(C â€” static-tag end-to-end tests)* spawn/insert/remove/With/Without/`&Tag`/`Mut<Tag>`/Added/Changed/hooks/observers/bundles-with-tags/`spawn(PlayerTag)` single-tag spawn integration suite.

**Wave 3 â€” tester + doc-writer**
10. Full test plan execution, Miri-TB suite, bench gates, proptests.
11. `boyko_demo`: replace 1-byte fake tags (sim/components.rs:40-74) with real ZSTs â€” dogfooding gate (exercises QueryView::for_each_chunk with real tags).
12. Public book pages (tags, dynamic tags, churn ladder, **fragmentation ceiling incl. the per-archetype VA profile: 128 MiB reserve per tag pool per hosting archetype, 2 MiB under the cfg fallback, zero resident until commit**, hook-registration contract, `no_bundle`); internal docs sync (SYSTEMS/FEATURE_MAP/ARCHITECTURE).

## Unsafe surface delta

| New/changed unsafe | Invariants (// SAFETY content) |
|---|---|
| ZST `buffer` derivation (`new`) | dangling base = `SIMD_BUFFER_ALIGN.max(align)`: aligned for T AND SIMD-A1-aligned, non-null, provenance-free; used ONLY for zero-size access; tick bases derive from `base = vm.base()` (NOT from `buffer` â€” O1), tick-tick disjointness unchanged, data-tick disjointness vacuous |
| `grow_rows_zst` vm commits | Z1â€“Z6: data region never committed (`data_committed == 0` invariant); tick commits strictly growing (Z4) and in-bounds (Z3); frontier fields written post-commit (Z6) |
| `swap_remove` (comment-only) | stride==0 â‡’ 0-byte copy between equal dangling pointers, valid; stride>0 â‡’ original distinct-rows proof |
| drop loop over ZST rows | `drop_in_place::<ZST>` reads no bytes; per-logical-row call at the shared dangling address sound; len bounds the count |
| `migrate_entity_attach_ids`/`detach_ids` | mirrors `migrate_entity_insert` verbatim: sourceâ‰ target asserted; archetype `&mut` lifetimes confined to Phase-1; EntityInland repoint hoisted inside it; world_ptr minted only after reborrows die; UnsafeCell-rooted slab provenance; zero-retained arm = empty loop, no pointers minted |
| `impl_self_bundle!` byte-erasure | mechanical reproduction of derive output (`from_raw_parts` over `ManuallyDrop` stack local) â€” same accounting note as bundles.rs:19-28; `EmptyBundle` contributes ZERO unsafe |

Net: ~2 genuinely new unsafe blocks (the `_ids` migration pair) + the `grow_rows_zst` commit calls (same shape as existing `grow_rows`); the term funnel adds ZERO unsafe (signature bit tests on safe refs). ZST pools cannot UB by aliasing because no data byte is ever read or written through a stride-0 column.

## Metrics and validation

**0%-regression bench gates (mandatory, by name)**: `query_iter` (inner loop byte-identical â€” term-transition branch verified by asm if criterion is ambiguous), `query_dsl`, `phase9_scheduler` (50 systems), `phase12_5_spawn_batch` (warm ns/e), `phase10_change_detection`, `phase14a_hooks_gate`, `random_access` (~3 ns â€” EcsMaster path, untouched by terms), `bundle_static_cache` (sub-ns), `component_pool_dense`, `swap_remove`, `ecs_master_new` (lazy budget), `archetype_create`, **plus the for_each_chunk bench used by the demo/X.A work** (the chunk driver gains the term parameter â€” `len==0` fast path must be flat). `legacy_query` benches, if any exist, must also be flat (pure renames, zero codegen change expected).

**New benches** (`benches/phase22_tags.rs`): spawn 10k tag-only; spawn 10k (2 data + 2 tags) vs (2 data) â€” target â‰¤ +10%; `has_tag` â‰¤ 5 ns; dynamic tag attach/detach toggle 10k â€” attribution note: this bench includes `get_or_create_archetype` lookup and hook dispatch, NOT just row-moves; the report must break the number down (first-toggle archetype-creation vs warm-toggle row-move); query with 1 dynamic term vs none (iter AND for_each_chunk drivers); ZST pool grow (cold).

**Tests**: unit (Wave 0 incl. the double-grow ZST test); integration â€” **all tag end-to-end tests live in `tests/phase22_tags.rs`, an out-of-crate integration crate, so `pub(crate)` access cannot mask API gaps (W3)**: two-empty-entities despawn identity, remove-lastâ†’emptyâ†’re-insert, spawn_emptyâ†’add_tag attach-from-empty + remove_tagâ†’back-to-empty (O3), idempotent `register_tag`, two-names-never-alias (O2), exhaustion â†’ `None`, **H1 three-case (W2)**, **W3 reachability chain (register_tag â†’ component_id() â†’ register_hooks_by_id + add_observer â†’ attach â†’ both fire)**, dead-entity no-ops, >8-term loud panic, arity-16 bundle, `spawn(PlayerTag)`; **per-driver term tests** â€” one behavioral test for each of the 11 drivers in the D4 table; compile-fail (Component+Bundle double derive; !Send component without `no_bundle` â€” snapshot pins BOTH diagnostics, anchored on the named const-assert symbol); proptest (random spawn/despawn/add_tag/remove_tag interleave vs HashMap membership oracle; ZST pool len/tick/grow model); **Miri-TB** (`tests/miri_phase22.rs`): tag attach/detach migrations **including attach-FROM-empty (zero retained columns) and detach-to-empty (O3)**, empty-archetype despawn, Drop-impl ZST pool teardown, ZST grow path, in-place tag re-insert, deferred command paths â€” the F2/NEW-1 TB-UB class was historically caught ONLY by Miri; non-negotiable.

**debug_assert invariants**: every pool's `add` index == `current_index` (create_entity); `data_committed == 0` in all ZST pool paths; `is_zst` for all ids entering `attach_ids`; canonical-sortedness of `_dyn` unions; Z4/Z5 strict-growth and sufficiency asserts in `grow_rows_zst`; tag-term count â‰¤ 8 (release assert, setup-time); dangling ZST buffer satisfies SIMD-A1 (belt).

## Out of scope (explicit)

- Enable-bits / non-fragmenting tag storage (flecs DontFragment, Unity v128) â€” future phase; seams reserved (`Column._reserved`, the D4 transition funnel).
- Tags-as-entities unification, pair/relationship ids â€” rejected for Phase 22; `TagId` + name registry are the forward seam.
- Typed `Added`/`Changed` for **dynamic** tags (ticks maintained; `DynAdded(TagId)` term is a follow-up â€” the D4 funnel is where it would plug in).
- General dynamic **data** components (size > 0 descriptors) â€” only size-0 mint ships.
- Serialization itself â€” this phase only guarantees the name-keyed identity it needs.
- Raising MAX_COMPONENTS/MAX_ARCHETYPES; archetype GC of fragmented empties.
- De-allocating the generic migration helpers' `to_vec()`/`kept: Vec` (cold-path debt, separate cleanup).
- `Query<Entity>`-style enumeration over the empty archetype beyond the pinning test.
- Term support on `legacy_query.rs` (no `with_tag` surface exists there; deliberate â€” its five call sites take the `_pre_terms` rename mechanically).

## Open questions (for critic/user)

1. `register_tag` on `EcsMaster` vs free registry function: plan keeps the EcsMaster method delegating to the global registry (tags are process-global like all ComponentIds). Confirm multi-world acceptability.
2. `MAX_DYN_TAG_TERMS = 8`: cheap to raise (stack-only); pick 8 unless >8 simultaneous dynamic terms per query are anticipated.
3. `try_register_tag -> Option<TagId>` vs a dedicated error type: `Option` chosen (single failure mode â€” budget exhausted; idempotent re-mint is success). Flag if the serialization phase will want a structured error.
4. `is_empty`/`len` term semantics: kept archetype-level (filtered matched-list membership, no `entity_count` consultation) to preserve today's exact semantics under terms. Flag if row-level emptiness is preferred â€” that is a separate (pre-existing) semantic question, not a Phase 22 one.

---

## Post-approval notes (critic round 4: APPROVED, 0 CRITICAL / 0 MAJOR / 2 MINOR)

Critique convergence: R1 REVISE (1C/3M/3m) -> R2 REVISE (1C/2M/3m) -> R3 REVISE (0C/2M/4m) -> R4 APPROVED.

Minor residuals to carry into the developer waves:

1. [MINOR] D4 claims ArchetypeQueryState::len/is_empty have 'zero non-test consumers (verified)' â€” false: legacy_query.rs:136-144 delegates Query::len/is_empty to state.len()/state.is_empty(). Two extra mechanical rename sites in the term-free legacy surface; the compile-enforced _pre_terms funnel will catch them, and the pub(crate) demotion stays safe (grep confirms no consumers in boyko_demo or external tests/). Add the two rows to the D4 table; legacy_query has seven affected sites, not five.

2. [MINOR] Wave sequencing: Wave 1B's H1 three-case test and W3 reachability chain require an attach step ('â†’ attach â†’ both fire'), but add_tag ships in Wave 2A. Specify in the Wave 1B brief that attach is driven through the existing id-keyed surface available today (get_or_create_archetype(&[tag_id]) + the direct create_entity path, whose hook loop is id-driven and empty-safe), or move those two tests to Wave 2 â€” otherwise the parallel wave stalls on a missing API.

Implementation deviation: TagId lives in component_registry.rs (mint-protocol locality + constructor privacy), not the planned identifiers/tag_id.rs â€” internal-docs sync (Wave 3) must point FEATURE_MAP at the actual file.

