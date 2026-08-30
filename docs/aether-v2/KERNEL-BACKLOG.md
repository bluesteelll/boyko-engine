# Kernel backlog — engine-side work, grouped by crate

Every engine addition the campaign needs, with the zero-cost-when-unused status and the oracle that
gates it. Aether-side work lives in [`CONSTRUCTS.md`](CONSTRUCTS.md) / [`MACHINES.md`](MACHINES.md).

**Id namespace (renamed 2026-08-29).** The bare `E#` / `M#` series collided with the `M#` machine
rulings in [`DECISIONS.md`](DECISIONS.md) and read as unqualified everywhere they were cited. This
file's ids are now `KE#` (`boyko_ecs`) and `KM#` (`boyko_macros`); the numbers are unchanged, so
`E11` → `KE11`. **The rename is COMPLETE across the corpus — and the corpus reaches OUTSIDE this
directory, which is what the previous version of this line got wrong.** The citing files,
**recomputed 2026-08-29** over `docs/` with `rg -n "\b(KE|KM)[0-9]+\b" docs/` (run as the PowerShell
`Select-String` equivalent on this checkout): in this directory, [`OPEN.md`](OPEN.md),
[`CAMPAIGN.md`](CAMPAIGN.md), [`DECISIONS.md`](DECISIONS.md), [`CONSTRUCTS.md`](CONSTRUCTS.md),
[`EVENTS.md`](EVENTS.md) and [`SPATIAL.md`](SPATIAL.md); outside it,
[`docs/OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) with its `docs/ru/` twin and
[`docs/gaia/DECISIONS.md`](../gaia/DECISIONS.md). **`SPATIAL.md` and `docs/gaia/DECISIONS.md` were
both absent from the previous enumeration**, and the second is why "across the directory" was the
wrong scope — `docs/gaia/` is a different corpus citing this backlog's ids.

**The list above is a dated snapshot, not the claim** — an enumeration of citers rots the moment a
sibling adds one, which is the defect it just carried. What is checkable without it, and what a
reader should run instead of trusting the list: every `\b(KE|KM)[0-9]+\b` hit over `docs/` resolves
to a row in this file, and the retired bare `E#`/`M#` form appears nowhere as a citation of it.
(Both halves are recipes to run, not results asserted here; the first was run to produce the list,
the second was not.) No waiver is carried for the retired bare form: a waiver that licenses known
rot is a measured failure class here, and since **GB-8's ruling (2026-08-30) it is a rule, not a
habit**: no per-site waivers, at any census, ever. ⚠ This sentence used to cite "the anchor census
abdicated 188 of 302 sites under one". **No in-tree gate produces those numbers** —
`tests/internal_docs_anchors.rs` states neither. Run live it prints **735 anchors, 116 waived**, and
the precedent survives the correction *stronger*: the waiver concentrated entirely in the one
document admitted under it, `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`, at **89 of 177 (50.3%)**.
Note that a bare
`E1`/`E2`/`E3` inside [`DECISIONS.md`](DECISIONS.md) is that file's **own** event ruling and is not
a citation of this file.

## `boyko_ecs`

| # | Item | Why | Zero when unused | Oracle |
|---|---|---|---|---|
| KE1 | ✅ **LANDED (R0, 2026-08-29)** — see the "What R0 landed" block below the table for the two forwarded items, the two deliberately NOT forwarded, and the shape matrix. The row below is the pre-fix statement, kept because the recount and the GB-9 back-pointer it carries are still live. **Fix `Or` dense blindness** — `impl_or_filter_tuple` (iters/query/filter.rs) forwards none of the dense plumbing the AND tuple has; a dense arm inside `Or<...>` silently matches nothing. ⚠ The "matches nothing" half was **half the class**: a dense `Without<D>` arm answers from the same NULL store with the OPPOSITE polarity and **excludes nothing** — measured, not read. Blast radius **re-measured 2026-08-29: 112 textual `Or<(` matches**, not the "~117 production sites" this row carried — 4 production type positions (`boyko_ui` layout.rs:88/:118, text/measure.rs:61; `boyko_render` light_system.rs:989), 6 doc-comment mentions, 48 kernel-internal lines across four files (filter.rs 34, par_chunk.rs 7, filter_enable.rs 4, state.rs 3), 54 test/bench/fixture sites. Producing command and the ledger row: [`OPEN.md`](OPEN.md) | R0. The priority rests on the **silent-wrong-answer mechanism** — the query compiles, runs, and returns a plausible wrong set — **not** on blast radius, which the recount shrinks to 4 production positions. Precondition for any `or(...)` grammar. ⚠ **This fix FALSIFIED a ground recorded on the Gaia side** — the generated-code ban on `Or<(Changed<A>, Changed<B>)>` over dense ([`../gaia/DECISIONS.md`](../gaia/DECISIONS.md) §UI bindings, item 8) stood on exactly this defect. That was ballot **GB-9**, ✅ **RULED 2026-08-30: option (b) — the ban is DELETED with a record**, and replaced by item 8's third row (a bind source or change-gate over a dense/bitset component), whose GK-2 ground is still live. ⚠ **The ruling establishes something this row's reader needs: the ban's ground could NOT be re-based onto KE13.** `Or` folds `NEEDS_CHANGE_DETECTION` over its members and `EcsMaster::query<D, F>()` const-refuses any `F` with it, so `Or<(Changed<A>, Changed<B>)>` cannot reach a `QueryView` at all — KE13 is a point-lookup defect over a dense `With`/`Without`, a different shape. The back-pointer this row owed that line is discharged | n/a (bug fix) | **red-first** test: `Or<(Changed<TableC>, Changed<DenseC>)>` vs a hand oracle. ⚠ GB-9's own warning survived its ruling and now rides item 8's third row instead: a fixture that asserts the KERNEL's behaviour goes green on this commit and silently stops guarding anything — the Gaia fixture must assert that the **generator does not emit the shape** |
| KE2 | **`Entities<'w>` read-only SystemParam** with `get(EntityId) -> Option<Entity>`; carrier pointer is `*const InlandStore`, NOT `*const EntityMaster` (type-enforced narrowness, the `EntityCounter` precedent) | machines need `me`; iterators yield `EntityId`, events carry `Entity` with generation; today the only resolver sits behind `&EcsMaster` (exclusive) | yes — pure type addition | unit tests + a debug assert mirroring the writer guard |
| KE3 | **`Query::{get, get_mut, single, single_mut, contains, first}`** — provenance is **SPLIT, not "port from `QueryView`, which has them all"**: `get`/`get_mut`/`single`/`single_mut` do port from `QueryView` (`crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs`:641/:767/:578/:601), but **`contains` and `first` do NOT exist on `QueryView` — they are NEW**. `get_mut` stamps the changed tick via the `SystemMeta` the `Query` param uniquely holds | "attack reads the TARGET's Health" today needs an exclusive system or an O(N) id scan. **First consumer: the R5 machine event router** | yes | **SPLIT oracle.** Ported half: port `QueryView`'s tests for the four ported methods. New half (no test to port): `contains` — including the alive-but-unmatched entity case; `first` — pinning a **STATED** order, which `QueryView` does not define, so the order is a decision this row makes rather than inherits |
| KE4 | **`Option<Res<R>>` / `Option<ResMut<R>>` params** — the null branch returns `None` instead of the `#[cold]` missing-resource panic; access declaration unchanged | unlocks the deferred `resource_exists` / `on_event` conditions; the one place the engine answers optionality with a panic | yes | unit test |
| KE5 | **Run-condition combinators** `CombinedSystem<A, B, OP>` + `Not` — **EAGER fold** (ruling D5: a skipped RHS freezes its change-tick window), access union, tick maintenance forwarded to BOTH children | `when (A or B)` / `unless C` | yes | the five D5 pin tests **cited by number**, per DECISIONS §D5: (1) both children evaluate every reached frame; (2) `run_once` on the RHS of an `or` whose LHS is true still consumes its run; (3) no bogus `Changed` burst from the would-be-skipped side on the following frame; (4) combined access = union; (5) tick maintenance forwarded to both |
| KE6 | **`ArchAdded` structural stamp** (ruling D2) — non-atomic, written only at cold sites already holding `&mut Archetype`; `Added<C>` consumers skip clean archetypes wholesale. The per-row value half is REJECTED; the amortized variant sits behind the specified bench | spawn-burst reactivity | yes (NCD const-fold) | stamp unit tests + the check-ticks clamp |
| KE7 | **`CommandQueue::{mark, rewind}`** — cursor save + drop-glue replay over the byte arena | all-or-nothing for the structural half; must DOCUMENT that `reserve_entity` ids are not reclaimed | yes | unit test incl. the reserved-id caveat |
| KE8 | **Event lanes.** ✅ **PARTLY LANDED at `01a4436e` — measured 2026-08-30, and the split matters:** `MAX_EVENT_THREADS` → 65 **and** the `MAX_WORKERS + 1 <= MAX_EVENT_THREADS` const-assert are **in the tree** (`constants.rs:400`). **Still unlanded:** `send`/`send_default` → `&self` (`event_writer.rs:110` is still `&mut self`), `send_slice` (**no occurrence anywhere in `crates/`**), the debug-assert re-aim (`event_writer.rs:112` still tests `is_in_system_run()`), the TLS depth counter, and `ordered` chunk-keyed lanes + the boot-time sender-exclusivity refusal. **New work added by the 2026-08-30 rulings:** `MAX_EVENT_THREADS` 65 → **66** with the const-assert strengthened to `MAX_WORKERS + 2 <= MAX_EVENT_THREADS`, plus the claimed-host-lane CAS with one-claimer refusal (**E5**); a **dispatcher lane-count setter** so the floor `max(N, worker_count + 1)` can be applied once the pool is known — `EcsMaster::new()` hard-wires `EventDispatcher::new(1)` and `default_thread_count` has no setter (**E4**); and `register_ordered_emitter` + the `EventWriter::init_state` refusal of a hand-written writer for an `ordered` type (**E6**). ⚠ **`send_one`'s `// SAFETY (U4 …)` clause 2 is measurably false today** ("only the worker pinned to `thread_index` accesses this UnsafeCell") and owes an edit in whichever commit builds this row | [`EVENTS.md`](EVENTS.md) | yes (default path unchanged for serial users) | loom/stress on the lane path, **re-scoped**: the stress set must include a `WORKER_ID_UNATTACHED` thread sending concurrently with worker 0 — as scoped today the debug assert excludes that class *by construction*, so the run cannot fail on it. Determinism gate for `ordered`, **re-axed at [`CAMPAIGN.md`](CAMPAIGN.md) R4 the same way R6 was**: the property is *the parallel path emits the stream the serial path emits* — a fixed scenario's event stream from the parallel pass compared **against the serial-emission reference stream** for W ∈ {1, 2, N}. `build(1) == build(W)` is demoted to a **smoke** check (it compares two runs of the same code against each other and so cannot see a lane-assignment defect that is stable across runs — exactly the defect class `ordered` exists to exclude), and the run must **report per-worker send counts** (AIR-12's counts-not-exit-code rule) so a silently-serial run is red, not green. **Plus a two-emitter red fixture** for the sender-exclusivity refusal — its predicate is now fixed by **E6**: an `ordered` event id whose `register_ordered_emitter` count exceeds 1 fails boot naming both systems. ✅ **The contract is no longer open**: ballots AB-3/AB-4 were ruled 2026-08-30 as **E5**/**E6** ([`DECISIONS.md`](DECISIONS.md)). ⚠ **The re-scoped stress fixture needs a rendezvous barrier, not just an added thread** — measured: with an unattached thread and worker 0 each sending 4000 events, 6/6 release runs lost 183–2295 of 8000 with a barrier and **6/6 lost nothing without one**, because the unattached thread drains before the pool task is enqueued. A stress test that only adds the thread reports green over this hazard |
| KE9 | **`par_for_each_chunk_entities`** — the parallel twin does not exist (the serial one does; `par_for_each_chunk` has no entity slice). Signature pinned: `par_for_each_chunk_entities<Func>(&mut self, f: Func, batching: BatchingStrategy) where Func: for<'c> Fn(&'c [EntityId], D::ChunkItem<'c>) + Send + Sync` (entity slice from :390; batching parameter and the bound from :652; `BatchingStrategy::default()` exists) | parallel machine passes; spatial Phase 3 | yes | new-unsafe review gate (entity-slice aliasing) + the serial-vs-parallel byte oracle its consumers use. Whether the batching value is author-visible was ballot **AB-8**, **RULED 2026-08-30**: it is **not** — the grammar always passes `BatchingStrategy::default()`, and the knob stays reachable only through the verbatim escape (DECISIONS **C5a**). The signature above is pinned either way. ⚠ Note the consumer named here ("parallel machine passes") now reaches `par_for_each_chunk` **only** via `each soa par`; plain `each par` lowers to `par_iter_mut`, which needs no entity-slice twin |
| KE10 | **`FLAGS_DIRECT`** — the enable-bit twin of `REQUIRES_DIRECT`: per-component initial flag states applied on attach (no transitive closure — a bit pulls no other bits; no ctor — a bit, not bytes) | the component `flags (…)` group | yes (`HAS_FLAGS` gate, same discipline as `HAS_REQUIRES`) | attach-path unit tests |
| KE11 | **Refuse (or filter) `#[require]` of an id whose storage kind owns no per-archetype pool — BOTH poolless kinds, Bitset AND Dense** (the row previously said "a bitset tag", which is half the class). Two sites reach an unfiltered pool lookup for such an id, and they are **two DIFFERENT calls** — (1) `BundleColumnCache::resolve_required_missing` (`crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs`) via `component_pools().pool_id_for(entry.component_id).expect("invariant: the archetype was expanded with every required id …")`, and (2) the `has_requires` required-ctor pass inside `migrate_entity_insert` (`crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs`) via `component_pools_mut().get_pool_mut(req_id).expect("invariant: target hosts every required id (expanded archetype)")`. Neither screens the storage kind first, unlike the sibling loop in `BundleColumnCache::resolve_and_cache`, which diverts a `StorageKind::Dense` id to `DENSE_POOL_SENTINEL` before resolving. ⚠ **Status updated by R0's census, 2026-08-29 — site (1) is MEASURED, site (2) is not.** A throwaway probe spawning a bundle whose component `#[require]`s a dense component, and one that `#[require]`s a bitset flag, panicked for **both** kinds at `bundle_column_cache.rs:410` with the predicted misleading message. Site (2) was **never reached**, because the spawn path dies at site (1) first — see the census's blind-site table for why that matters to this row's oracle | `requires` of a `flag` — and, on the same mechanism, `requires` of a dense-storage component — is a boot-time crash carrying a misleading invariant message | n/a | write the red tests FIRST, **parameterised over both poolless kinds** — and ⚠ **over both SITES**: a suite that only spawns pins site (1) twice and silently reports site (2) as covered. Reaching site (2) needs an **insert into an existing entity** (the `migrate_entity_insert` path). The tests demonstrate the panic regardless of which disposition wins. Disposition (refuse / filter / dense construct-and-commit route) is ballot **AB-6** below |
| KE12 | *(deferred, measure first)* `VmColumn` promotion to pub; the g6-style A/B of `iter` vs `for_each_chunk` on the machine-pass shape; the D2 value-half bench | — | — | criterion |
| KE13 | 🆕 **UNOWNED — raised by R0's census, 2026-08-29.** `QueryView::get` / `get_mut` apply the matched-archetype bitset, the dynamic tag terms and the enable terms, but **never call `F::filter_fetch`, and resolve `resolve_dense` only for `D`**. A **dense** `With<C>` / `Without<C>` is therefore silently ignored on the point-lookup path: a dense `With` has `IS_ARCHETYPAL = false` and a `matches_component_set` that admits every archetype, so its ONLY gate is the `filter_fetch` `get` does not run. `get` returns `Some` for a non-member and **disagrees with `iter()` on the same view** | Same shape as KE1 one path over — *a caller that does not make the dense call* — but strictly worse: KE1 made an arm never true, this applies **no gate at all**, in both polarities. ⚠ **`get`'s own doc comment asserts the opposite in writing** ("So `get` only ever sees archetypal and enable terms — there is no silent-ignore path"), on the premise that `With`/`Without` are archetypal, which the Dense plan falsified. A fix owes that paragraph an edit **in the same commit**. ⚠ In-tree precedent that this path was known under-served: `state.rs`'s `dense_get_iter_agree` carries a "PRE-EXISTING BUG … flagged for the reviewer" note that `get` never calls `resolve_dense`, calling the repair "a one-line `resolve_dense` mirror" — **the `D` half was since fixed, the `F` half was not**, and it is not a one-liner (there is no `filter_fetch` call to feed) | n/a (bug fix) | ✅ already written and **observed red**: `crates/boyko_ecs/tests/ke13_query_view_get_ignores_dense_filter.rs` — two `#[ignore = "deferred: KE13 …"]` tests (both polarities) plus a non-ignored control pinning that `get` DOES apply a table filter and that `iter()` answers the dense one correctly. Un-`#[ignore]` them when a rung takes this |
| KE14 | 🆕 **UNOWNED — opened 2026-08-30 by AB-6's landing.** The retained-dense path in `migrate_entity_insert`: five confirmed defects, four of them pre-existing, none of them memory-safety (Miri clean). **D1** is a reachable panic — `invariant: retained component must exist in source` at `migration_helpers.rs:596`, with a sibling face on the remove path at `:1234` — and **D2** makes a correctly constructed required dense component **vanish from every dense query on the next insert**. Full statement, repro and oracle: **§KE14** below the tables | `#[require]` over dense works on the **spawn path only**. KE11 made the declaration accepted and advertised, so it is now a documented route into defects that were previously unreachable | n/a (bug fix) | red-first per defect, written from the three-step repro **before** any fix; the `ke11_*` table-storage controls stay green throughout, since they are what distinguishes a real fix from a broken fixture |
| KE15 | 🆕 **UNOWNED — opened 2026-08-30 by AB-7's ruling.** Give the parallel chunk runner a world cell. `par_iter.rs:305` carries `const { assert!(!D::HAS_DENSE && !F::HAS_DENSE) }`, and the comment above it names the cause in its own words — *"the chunk runner has no world cell"* — i.e. unwired plumbing, not a design limit, the same shape as KE1. The **next** `const` block refuses `Related<R, D>` joins for the SAME missing cell, so one fix retires two refusals. Full statement and oracle: **§KE15** below the tables | the owner ruled R-DENSE lifted; lifting alone gives **sequential** machines over dense, and the `parallel` half waits on this row | yes — const-folds away for a query with no dense and no relation term | red-first: a `par_iter` over a dense term seen failing to compile, then compiling and matching the sequential `Query::iter` result set for W ∈ {1, 2, N}, with **per-worker touch counts reported** so a silently-sequential run is red rather than green |
| KE16 | 🆕 **UNOWNED, RESEARCH-AND-BUILD, deliberately not launched (owner, 2026-08-30).** Pool occupancy: work spawned by a worker is unreachable by siblings (defect A, `worker.rs:370-371`), and the joining thread parks half a wave in a stealer-less private scratch (defect B, `scope.rs:440-531`). The owner adds a third mechanism neither defect covers — an **idle-worker queue**, since the first-level system scheduler distributes unevenly and lanes drain at different times. Full decomposition, the three mechanisms with their separate prices, the cache-locality axis, and the research scope: **§KE16** below the tables | measured: `par_iter` in a system body is **1.01×** where the same driver called from outside is **7.69×**; all four parallel physics sites are on that path | n/a (bug fix + design) | ⚠ **the acceptance criterion is THROUGHPUT, not occupancy** — owner: *"if it needs synchronisation heavier than the gain from maximum core loading, it should not be done."* A red-first occupancy gate proves the mechanism; the decision between variants is wall-clock on a real consumer, and "keep the current behaviour" is a legitimate outcome for B |

### What R0 landed for KE1 (2026-08-29)

`impl_query_filter_tuple_and` declares **four** dense items. Enumerated by name, that enumeration
being the specification R0 worked from:

| # | item | forwarded by `Or` after R0? |
|---|---|---|
| 1 | `const HAS_DENSE` (OR-fold over members) | **YES** — this is the const `QueryIter::new` / `QueryIterMut::new` gate `if const { F::HAS_DENSE }` on; at `false` the store pointer was never resolved |
| 2 | `unsafe fn resolve_dense` (per-member, `world` by value) | **YES** — forwarded to each arm's `$f.0` (`$f.1` is BUG-ENABLE-PRE-1's per-archetype `matches` flag, which `set_table_*` owns) |
| 3 | `const HAS_DENSE_INCLUDE` (OR-fold) | **NO, deliberately** |
| 4 | `fn dense_include_candidates` (per-member) | **NO, deliberately** |

**Why 3 and 4 are not forwarded, and why forwarding them would have been a new defect.** A dense
INCLUDE term *bounds* the candidate archetype set — `QueryDataState::dense_seed` seeds `matched_ids`
from that term's `arch_presence` when the include mask is empty. Under a **disjunction** it bounds
nothing: an archetype admitted through a SIBLING arm would be dropped by a seed taken from one arm.
Same ground as `Or::aggregate_include`'s explicit no-op override, and the same disposition `AnyOf`
already carries (`HAS_DENSE = true`, `HAS_DENSE_INCLUDE = false`).

**Two silent wrong answers, not one.** With the store pointer NULL, the per-row predicate reads the
NULL as an answer: `Changed`/`Added`/`With` short-circuit to `false` (arm never true), while
`Without` returns `!false` = `true` (**arm excludes nothing**). Both measured pre-fix.

**Oracle** — `crates/boyko_ecs/tests/ke1_or_dense_blindness.rs`, 8 tests, each pinning a **hand
oracle** (a literal payload-id set) rather than a count. Red-first: 7 of 8 failed before the fix,
all 8 pass after; the 8th (`or_all_table_arms_are_unchanged`) is the over-correction guard and was
green on both sides by design. Shapes: dense arm on the RHS · on the LHS · sole arm (`Or<(F,)>`) ·
dense `With` · dense `Without` (opposite polarity) · nested `Or` in `Or` · `Or` inside an AND tuple ·
all-table guard.

**Shapes NOT covered**, stated so the next reader does not assume them: `Query<Entity, Or<..dense..>>`
(no table positive bound, so the empty-include branch of `QueryDataState::new` is reached — untested);
`Added<Dense>` inside `Or` (same plumbing as `Changed<Dense>`, argued not measured); arity > 2 `Or`
with more than one dense arm; the `par_iter` / `for_each_chunk` paths, which now **compile-refuse** a
dense-armed `Or` via the pre-existing `const { assert!(!D::HAS_DENSE && !F::HAS_DENSE) }`
(`chunk_iter.rs`, `par_chunk.rs`, `par_iter.rs`) — previously they compiled and answered wrongly.
No production site is affected today: the only production `#[component(storage = "dense")]` type is
`boyko_render`'s `GpuTransform3D`, and it appears in none of the four production `Or<(` positions.

### Census — "resolves a per-archetype pool by `ComponentId` without screening the storage kind"

The class R0 was asked to enumerate, because KE1 is one instance of it and two more were found by
the Gaia campaign in shipped code. **Producing command** (run from the repo root; the definitions in
`component_pool_bundle.rs` are excluded because they ARE the resolver):

```
grep -rn --include=*.rs -E "\.(get_pool|get_pool_mut|pool_id_for)\(" crates/*/src \
  | grep -v component_pool_bundle.rs
```

**52 hits, of which 10 are inside `#[cfg(test)]` modules** (`archetype.rs` ×8, `archetype_master.rs`
×1, `archetype_bundle.rs` ×1) — **42 production sites**. Classification, by enclosing function:

| class | sites | the argument |
|---|---|---|
| **(a) screens the kind first** — 13 | `BundleColumnCache::resolve_and_cache` · `SpawnBatchCommand::apply` · `InsertCommand::apply_replace_in_place` · `migrate_entity_insert` (bundle pass) · `Archetype::create_entity` · `Archetype::create_entity_with_ticks` · `MaterializeClone::drop` · `materialize_clone_into` · `write_clone_column` ×4 · `Prefab::capture` | `resolve_and_cache` is the **known-good pattern**: `matches!(storage_kind(cid.0), Dense)` → `DENSE_POOL_SENTINEL` before resolving, and it builds the `dense_mask` that `SpawnBatchCommand::apply` then skips on. `InsertCommand` screens at its own head; `migrate_entity_insert`'s bundle pass screens one loop earlier; `create_entity*` never sees a dense entry because `entity_api::partition_dense_components` splits them out upstream; the six `materialize.rs` rows AND `Prefab::capture` share ONE screen — `select_clone_ids`'s `if !is_signature_storage(storage_kind(id.0)) { continue; }`, the shared selector both call (verified by reading, not inferred from the file each site sits in) |
| **(b) unreachable for a poolless id** — 23 | `run_check_ticks_scan` · `migrate_entity_insert` (target/retained) ×2 · `migrate_entity_remove` ×2 · `migrate_entity_attach_ids` ×3 · `migrate_entity_detach_ids` ×2 · `retag_in_place` · `Archetype::refresh_column` · `Archetype::tick_column_base` · `save_world` (`boyko_serialize`) · `load_writer` ×8 (`rollback_committed`, `load_archetype` ×5, `remap_loaded_entities` ×2) | The ids come from an archetype's own `component_ids()` / signature, and `register_component` + `register_component_inplace` both refuse a non-signature id via `is_signature_storage`, so a signature list contains only `Table` ids. `refresh_column` is only called after `add_pool`. `tick_column_base`'s only callers are `Added`/`Changed`'s `set_table_*`, which `return` from the `const { C::STORAGE_IS_DENSE }` branch first. ⚠ **The 8 `load_writer` rows carry a weaker argument**: their ids come from the FILE, and the screening happens upstream in `boyko_serialize/src/load.rs` (Bitset at :484, Dense at :651). **R0 did not verify that those two screens cover every path into `load_archetype`** — the load path is Gaia ballot **F4**'s subject, and this row is a pointer for it, not a clearance |
| **(c) BLIND** — 4 | `BundleColumnCache::resolve_required_missing` · `migrate_entity_insert` (required-ctor pass) · `EcsMaster::any_changed_since` · `EcsMaster::get_component_changed_tick` | see below |
| **not settled** — 2 | `Archetype::make_component_device_backed` · `Archetype::set_component_device_handle` | Both `.expect()` on the pool. Both are guarded by `assert_eq!(residency_class(cid) == ResidencyKind::Gpu)`, and `StorageKind::Dense` is documented always-`Cpu` (Dense plan W1), so a dense id cannot reach them. **Whether a `Bitset` id can be `Gpu`-classed was not established** — if it can, these panic with an invariant message about pools |

**The four blind sites, and who owns each:**

| site | what "no pool" is consumed as | owner | red test |
|---|---|---|---|
| `BundleColumnCache::resolve_required_missing` | `.expect(…)` panic, message blames the archetype-expansion contract | **KE11** / ballot **AB-6** (rung R1) | R1's designed test is **not** written here — KE11's oracle prescribes one "parameterised over both poolless kinds" that "lands under all three dispositions", and a test asserting *correct* behaviour cannot be written before AB-6 picks what correct is. But ⚠ **KE11's premise is no longer a code reading: R0 MEASURED it.** A throwaway probe (`#[require(Dense)]` and `#[require(Bitset)]`, spawned through `Commands`) panicked for **both** kinds at `bundle_column_cache.rs:410` with the exact misleading message the row predicts. KE11's "Read as a guaranteed panic, **unconfirmed by a test**" should now read *confirmed for site 1, both kinds; the probe was not kept* |
| `migrate_entity_insert` required-ctor pass | same shape, different call (`get_pool_mut(req_id)`) | same | ⚠ **STILL unconfirmed, and R1 must not assume the site-1 measurement covers it.** The spawn path panics at site 1 first, so the probe above never reached site 2 — reaching it needs an **insert into an existing entity** (the migration path), not a spawn. A test that only spawns will pin site 1 twice and report site 2 as covered |
| `EcsMaster::any_changed_since` | `else { continue }` ⇒ the gate is **never true**; a `boyko_ui` binding over a dense source never updates | Gaia **GK-2** | ✅ `crates/boyko_ecs/tests/gk2_change_tick_api_dense_blindness.rs::any_changed_since_sees_a_dense_component` — `#[ignore = "deferred: …"]`, **observed red** |
| `EcsMaster::get_component_changed_tick` | `?` ⇒ `None` forever; every caller (`boyko_ui` `bind_system.rs` :138/:194, `boyko_scene` `propagation.rs` :427/:432) reads `None` as "source despawned, skip" | Gaia **GK-2**, but ⚠ **GK-2's row names only `any_changed_since`** | ✅ same file, `get_component_changed_tick_resolves_a_dense_member` — `#[ignore]`, **observed red** |

**A fifth site, found by the census but NOT of the pool-resolution shape — new item KE13.** The
census's grep is keyed on `get_pool` / `pool_id_for`, so it cannot see the *sibling* failure mode:
a caller that never makes the dense **resolve** call at all. That is KE1's own shape, and R0 found
one more instance of it — `QueryView::get` / `get_mut` resolve `resolve_dense` for `D` only and
never call `F::filter_fetch`, so a dense `With`/`Without` filter is applied on `iter()` and ignored
on `get()`. Filed as **KE13** in the table above, measured red, unowned. ⚠ **A future census of this
class must search BOTH shapes** — "resolves a pool without screening" *and* "consumes a dense fetch
without resolving it" — because one grep does not find the other.

⚠ **The sharpest thing the census found.** Follow-up #14 already remediated this exact class in this
exact file: `get_component` / `get_component_mut` / `has_component` / `get_component_raw` /
`set_component_raw` each grew a `storage_kind(…) == Dense` screen routing to the `DenseStore`
(`component_api.rs`; gated by `tests/dense_direct_component_access.rs`). **The two tick readers sit
between those screened functions and did not get one.** A remediation of the class, in the file, is
not evidence the file is clear — which is the argument for keeping this census as a table rather
than a claim.

**Adjacent, NOT this class, and not fixed here — ballot AB-11.** `With<F>` / `Without<F>` over a
`StorageKind::Bitset` flag are two more silent wrong answers, but the defect is in the **leaves**,
not in `Or`: `With::matches_component_set` / `Without::matches_component_set` branch on
`STORAGE_IS_DENSE` and then fall through to `mask.contains(state.id)`, and a bitset id is in no
archetype signature — so `With<F>` **matches nothing** and `Without<F>` **excludes nothing**.
`Added`/`Changed` already const-refuse a bitset `C`; `With`/`Without` carry no such guard.
**Measured, with R0's fix already in the tree**, in
`crates/boyko_ecs/tests/ab11_flag_filter_polarity.rs` (three `#[ignore = "deferred: …AB-11…"]`
tests, all observed red, plus a non-ignored `Enabled<T>` control so they cannot pass vacuously) —
including one that puts the flag arm inside `Or` and shows R0's forwarding does not reach it, which
is the evidence that AB-11 is a separate mechanism rather than a KE1 leftover.

## `boyko_macros`

| # | Item | Why | Oracle |
|---|---|---|---|
| KM1 ✅ **LANDED 2026-08-30** | **`state_chart!`** — the machine flattening moved here (`boyko_macros/src/state_chart/`: `ast` parse, `model::Chart::build` flatten + validate + `check_reachability`, `emit::leaf_fn` the **per-leaf route merge**). The merge fixed the both-chains-run defect and landed first-declared-wins arbitration | Aether stops being a codegen authority — `expand::machine_items` now emits ONE `::boyko_macros::state_chart!` invocation and Aether keeps only its sugar table; hand-written Rust gains hierarchical charts | red-first: `aether_tests/tests/r2_chart_arbitration.rs`, watched failing at `exit == 2` before the fix, green after. 22 unit tests in `state_chart::tests`; dead-state golden `ui/machine_unreachable_state.rs`. **Reachability is a hard ERROR** — stable proc-macros cannot warn, so "warn" would mean emitting nothing. One known gap: a state driven only by an external `NextState` write is a legal program this refuses, and the opt-out keyword belongs to R3's grammar |
| KM2 | **Unlock `on_despawn`** in `#[derive(Component)]` — the derive still refuses the key ("deferred to Phase 14b") while the kernel hook field exists and fires | the one push-mandatory case (despawn has no pull carrier) is unreachable from the derive | derive test + hook-fire test |
| KM3 | `#[event]`: the `ordered` registration surface; the flat-constructor emission if it lands macro-side rather than Aether-side | [`EVENTS.md`](EVENTS.md), [`CONSTRUCTS.md`](CONSTRUCTS.md) | token pins. **No oracle here may be premised on "registration forgotten → silence"**: that class cannot be made red — both generated ends are a loud init-time panic (`event_not_preregistered_panic`; DECISIONS C3), so a test written against the silence would pass vacuously. ✅ What registers the `ordered` fact was ballot **AB-4**, ruled 2026-08-30 as **E6**: **not** this macro. The registrant is the generated path's `register_ordered_emitter` at plugin build. The macro side was rejected as structurally impossible — exclusivity is a property of the emitter **set**, and the macro sees one type and zero systems; it does not read its attribute arguments at all today (`boyko_macros/src/lib.rs:261` takes `_args: TokenStream`), so `ordered` cannot even be spelled here yet. What KM3 still owes is the token pins for the `ordered` **spelling**, not the exclusivity fact |

## `aether_lang` / `aether`

The v2 construct rewrite (R3), the per-entity `machine` front-end (R5), `each`, `resource`, all
refusals, the domain-mismatch diagnostic, the cost-model note, and the two authored-scene emission
fixes (extras-bundle collapse; `spawn_batch` grouping) — specified in
[`CONSTRUCTS.md`](CONSTRUCTS.md) and [`MACHINES.md`](MACHINES.md). Gate discipline is v1's three
lanes: token pins in `aether_lang`, trybuild goldens, and the behaviour lane in `aether_tests`
against the real engine.

## New crate `boyko_spatial`

Phases 1–4 in [`SPATIAL.md`](SPATIAL.md). Phase 1 needs **zero kernel changes**; Phase 3 needs KE9.
The cell-hash + CSR + key-range-scatter internals are written as reusable building blocks (three
intended consumers — gameplay, physics-broadphase convergence, streaming cells).

## Substrate outside the workspace

- **`boyko_reflect`** — consumer: **AIR-06(b)** (the project-schema dump), rung R8. Status:
  **external — it is NOT a workspace member**; it lives on the unmerged branch `feat/reflection`
  (**15 commits ahead of and 20 behind** `feat/aether-v2` as of 2026-08-30; the "18 commits ahead"
  this bullet used to carry was measured against a different base and never carried the
  behind-count). Recording this dependency was never gated on any ballot; the *sequencing* was
  ballot **AB-9**, ✅ **RULED 2026-08-30 — the merge is pulled forward as its own rung before R8,
  and AIR-06(b) is NOT descoped.** ⚠ **The merge is necessary but not sufficient**: reflection is
  opt-in per component (`registry::type_info_of` returns `None` without `#[component(reflect)]`),
  and **not one of the 50 opt-in sites on that branch is in an engine crate** — so engine-crate
  opt-in (sweep vs derive default, **R8's design pass, not AB-9's call**) is a precondition on
  AIR-06(b), and until it exists the dump covers the empty set. Everywhere else the crate is
  cited — e.g. AI-ORIENTATION N1 — it is a **methodological precedent only**: the technique is
  portable, the dependency is not taken.

## Open ballots raised against this backlog

Nothing below is decided here; each row states the question, the alternatives on the table, and what
it blocks. **Struck rows are no longer open** — AB-3, AB-4, AB-8 and AB-9 were ruled 2026-08-30 under
the owner's delegation, and each points at the ruling that replaced it; the rows are kept so the
question a ruling answered stays readable beside it. Ballot bodies live in
[`docs/OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md).

| Ballot | Question | Alternatives | Blocks |
|---|---|---|---|
| ~~**AB-3**~~ | ✅ **RULED 2026-08-30 [delegated] → [`DECISIONS.md`](DECISIONS.md) E5** — claimed host lane, **one claimer enforced** (CAS on an owner slot; a second unattached sender is refused loudly). `MAX_EVENT_THREADS` 65 → **66**, const-assert → `MAX_WORKERS + 2 <= MAX_EVENT_THREADS` | ⚠ **This row's own premise was false and is corrected:** the const in the tree is **65** (`constants.rs:400`), KE8's lane half having landed at `01a4436e` — so the raise buys **one** lane, not two. *Rejected:* (b) `Err` on unattached — breaks the main-thread route the engine's own doc calls safe; (c) accepted hazard — **unavailable**: the loss is silent (6/6 release runs, every send `Ok`) and is UB, and the tree already carries that option's output as a **false** `// SAFETY:` clause in `send_one` | ~~R4~~ **unblocked** |
| ~~**AB-4**~~ | ✅ **RULED 2026-08-30 [delegated] → E6** — registrant is the **generated path** (`register_ordered_emitter(event_id, system_id)` at plugin build); **predicate**: an `ordered` event id with emitter count **> 1** fails boot naming both systems; **verbatim escape refused** at `EventWriter::init_state` | *Rejected:* `SystemMeta` emit-access — `init_access` is an **empty body** and events are deliberately outside the conflict graph, so there is no axis to read; adding one as a write would serialise every same-type emitter pair and surrender E1's parallel emission. The `#[event]` macro side — structurally impossible (one type, zero systems; `_args` discarded) | ~~R4~~ **unblocked** |
| ~~**AB-6**~~ | ✅ **RULED 2026-08-30 BY THE OWNER — option (b), and the class split in two.** Owner: *"then we should just make plain Rust able to do it"*, on the ground that Aether is sugar over hand-written Rust and may not refuse what the derive accepts. **The ballot's premise was refuted first**: hand-written Rust could not do it either — `ke11_require_poolless_storage_kind.rs` is plain Rust and panics, and `parse_requires` never screened storage kind — so there was no ratified surface to *narrow*, only a promise the kernel did not keep. ⇒ **DENSE: build the construct-and-commit route** (the ctor writes straight into the dense column; no scratch copy, so no double-drop site exists). ⇒ **FLAG/bitset: refuse, in the DERIVE, not in Aether** — a flag has no bytes and `RequiredCtor` writes bytes; the capability is `FLAGS_DIRECT` (KE10), and the did-you-mean points at `flags (X = true)`. Refusing in the derive keeps both surfaces identical, which a parse-only refusal would have broken | *Rejected:* (a) parse refusal — would make Aether reject what `#[derive(Component)]` accepts, violating Aether's prime directive; (c) known-open — the workaround is real (an `on_add` hook on a table component inserts the dense one, measured 2026-08-21) but leaves a misleading invariant message on the sharp edge | KE11 disposition — **settled** |
| ~~**AB-8**~~ **RULED 2026-08-30** | the `each par` / machine-parallel surface behind KE9 | **`par_iter_mut`** (measured 2048/2048 tracked vs 0/2048 chunked; 1.17–1.47× cost ≈ 1–2 % of the pass). `soa par` **exists**, and is the only route to `par_for_each_chunk`. Batching key **not** author-visible — always `BatchingStrategy::default()`. Machines and `each` share the **ladder**, not the default. → [`DECISIONS.md`](DECISIONS.md) **C5a** | — |
| ~~**AB-9**~~ | ✅ **RULED 2026-08-30 [delegated] → [`DECISIONS.md`](DECISIONS.md) §Sequencing rulings, entry AB-9** — `boyko_reflect` sequencing for AIR-06(b). **The merge is pulled forward** as its own rung before R8; **AIR-06(b) is NOT descoped** (the "reflection-free halves still satisfy its oracle" claim measured false); **plus a fourth item no option named** — engine-crate reflection opt-in as a precondition on R8's Lands | *Rejected:* R8 waits — the gate greens over a dump covering **0 of 138** engine-crate `#[derive(…Component…)]` sites (measured over `crates/*/src`), while the divergence grows in `crates/boyko_macros/src/component.rs`. *Rejected:* descope (b) — the same zero coverage plus the loss of the oracle's only workspace-facing clause. Full grounds: [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29 | ~~R8~~ **unblocked** — the merge rung and the opt-in route are *work*, not ballots |

## KE14 — the retained-dense path in `migrate_entity_insert` (NEW, opened 2026-08-30 by AB-6's landing)

**AB-6's route landed and works at SPAWN. It does not close the promise, and this row says how.** Five
defects were confirmed by two independent reviewers against the landing; **four predate KE11** and
reverting the route would not fix any of them; **none is a memory-safety defect** (Miri clean,
including a path the landing never exercised). What KE11 changed is *reachability*: before it, a
`#[require(<dense>)]` declaration died at `bundle_column_cache.rs` site (1), so the paths below were
unreachable. Now the declaration is accepted and **advertised**, so it is a documented route into
them.

⇒ **Do not describe `#[require]` over dense storage as working until D1 and D2 are closed.** It works
on the spawn path only.

| # | class | what happens |
|---|---|---|
| **D1** | **panic** | `migration_helpers.rs:596` `invariant: retained component must exist in source`. Repro, three steps, no fixture tricks: spawn a WIDE bundle `(A, B, Req)` where `Req` is `#[require(Dense)]`; spawn a NARROW bundle `(A, Req)`; insert `(B,)` into the narrow entity. A sibling face panics on the **remove** path at `:1234`, `invariant: target ⊂ source` |
| **D2** | **silent wrong answer** | a correctly constructed required dense component **vanishes from every dense query on the next insert** — the entity keeps it in the store but stops being enumerated. This is the one that decides usability |
| **D3** | silent miss | `apply_replace_in_place` never constructs the requirement (reachable via an all-table bundle; it never panicked before, so this is a miss rather than a regression) |
| **D4** | **silent wrong answer** | `flags (…)` declared by the constructed required dense component are dropped on insert. The landing's *"buys the flags for free"* claim is **struck** — it is false |
| **D5** | unsafe-contract divergence | the route's `U1` cites a precondition the callee does not document. Not UB today; the contract text has to be reconciled or one side changed |

**Why D1's "unreachable" argument failed, because the shape recurs.** The landing argued the phantom
needs to sit in *both* the source's and the target's `component_ids`, and that screening
`merged_archetype_id` controls the target. It does not: `ArchetypeMaster::get_or_create_archetype`
keys on `filtered_signature_mask` and returns `find_exact_match(&mask).first()` — **an archetype
minted earlier from a different id list** — and `cold_register_bundle_archetype` is exactly the site
the landing left unscreened. The landing **identified this race one paragraph later** and applied it
only to attach-flags, not to the sufficiency argument it had just made. A hazard named in one
paragraph and forgotten in the next is this corpus's most repeated shape.

**A second wrong oracle, found during design.** Site (2)'s present⇒skip test asks
`src!().component_ids()` — the **table signature** — about a dense id. The dense membership oracle is
`dense_registry.store(cid).contains(entity_id)`. Asking the signature about a non-signature storage
kind is the same class as KE1/KE13.

**Oracle for this row:** D1 and D2 each get a red-first test written from the repro above **before**
any fix; D1's must cover the remove face at `:1234` as well as the insert face at `:596`. The
existing `ke11_require_poolless_storage_kind.rs` controls stay green throughout — they are what
distinguishes a real fix from a broken fixture.

**Owner:** unassigned. This is *work*, not a ballot — AB-6 is settled; these are its unfinished half.

## KE15 — give the parallel chunk runner a world cell (NEW, opened 2026-08-30 by AB-7's ruling)

**Ballot AB-7 was resolved by the owner, and he rejected the ballot's framing rather than picking
one of its two options.** Owner, 2026-08-30: *"this needs to be fixed — that there is no
parallelism is just wrong."*

⇒ **R-DENSE is LIFTED**, and the reason the ballot offered for keeping it unconditionally is gone:
the candidate driver-independent ground was measured on 2026-08-30 and **refuted on both
conjuncts** — the router's deposit path does not assume a table row (both `get_component_mut` and
`Query::get_mut` carry working dense arms, pinned in `ke3_query_random_access.rs`), and there is no
layout const-assert (every dense `const assert` in the kernel belongs to a *driver*). Dense is fully
iterable and tracked under `iter_mut`, 64/64 rows.

⇒ **And the driver limitation itself is now work, not a constraint to design around.**

**What actually refuses, measured.** `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs:305`:

```rust
const {
    assert!(
        !D::HAS_DENSE && !F::HAS_DENSE,
        "a dense (storage = \"dense\") term is not supported on `par_iter` in D3 — …"
    )
};
```

and the comment above it states the cause in its own words: *"the parallel path does not resolve the
dense store into each worker chunk's `Fetch` (the chunk runner has no world cell)"*.

**That is unwired plumbing, not a design limit** — the same shape as KE1, where `Or` refused dense
because it forwarded none of the dense plumbing the AND tuple already declared. A refusal whose
stated ground is "the driver does not do it yet" must not be ratified as permanent, which is exactly
what the owner's reading caught.

**One fix, two refusals.** The immediately following `const` block rejects `Related<R, D>` joins for
the *same* missing world cell (`par_iter.rs:317`, *"the parallel chunk runner has no world cell to
resolve the FK target's archetype per row"*). Whatever gives the chunk runner access to the world
unblocks both; a design that fixes only the dense half leaves the second refusal standing on a
ground that no longer exists, which is how this corpus generates stale refusals.

**Scope note, so nobody over-promises from AB-7's lifting alone:** lifting R-DENSE gives
**sequential** machines over dense today. `parallel` machines over dense need this rung. Until it
lands, an Aether `machine … on entity parallel` over a dense-storage component is still a
compile-time `E0080`, and the refusal message should say *"not yet"* rather than *"not supported"*.

**Oracle:** red-first — a `par_iter` over a dense term must be seen failing to compile, then compile
and produce the same result set as the sequential `Query::iter` over the same fixture, for
W ∈ {1, 2, N}, with a **per-worker touch counter reported** so a silently-sequential run is red
rather than green (the campaign's standing counts-not-exit-code rule). The `Related` half gets the
same treatment or an explicit statement of why it is deferred.

**Owner:** unassigned. Depends on nothing that is still balloted.

## KE16 — pool occupancy: the design space, RECORDED AND NOT YET RESEARCHED (2026-08-30)

**Status: deliberately NOT launched.** Owner, 2026-08-30: *"we need to consider exactly all possible
variants and study various articles and information on the topic. For now don't launch the research,
just record it."* And, on what the eventual work is: *"the agents' task will be to study all possible
variants and make the most performant one."*

⇒ **The commissioned work is research THEN implementation, one pass, not a study that hands back a
recommendation.** The deliverable is the fastest variant, landed and measured; the survey is the
means. This row exists so the direction is not lost, and so whoever picks it up starts from the
decomposition below rather than from the idea.

### ⚠⚠ "All variants" means the whole space, and the decomposition below is NOT it

Owner, clarifying 2026-08-30: *"By all variants I mean not only the ones we have just discussed, but
generally all that could exist in theory. Again — study the solutions on the internet and the papers
on the topic."*

**So the three mechanisms enumerated below are a STARTING POINT, not the design space**, and the
research must not treat them as the menu. They were derived from one defect that happened to be
found, which is precisely how a survey inherits its own blind spot — the same failure this campaign
measured when a storage-kind census was built from eight known instances and could not see the class
that sat above it.

The survey therefore owes:

* **the literature**, not only the implementations — work-stealing has a research record (Blumofe &
  Leiserson's Cilk scheduler and its bounds; the work-first/help-first split; lifeline-based global
  load balancing; receiver- vs sender-initiated balancing) and the papers state *why* each design
  chose what it did, which the source code does not;
* **the whole axis set**, enumerated before candidates are scored: where work is placed on spawn,
  who initiates transfer, granularity of transfer, victim selection, what a blocked thread does,
  parking/backoff policy, affinity and NUMA, and whether the pool is even the right level for the
  fix — a scheduler-level answer (partitioning systems by measured cost) may beat a pool-level one;
* **designs deliberately unlike ours** — a fixed partition with no stealing at all, a central task
  queue, hierarchical/per-socket queues, delegation instead of stealing — including ones this engine
  would reject, since knowing *why* they lose is what makes the winner defensible;
* **what was tried and abandoned**, which is the half a survey usually skips and the half that
  prevents rediscovering a dead end.

⚠ And the standing evidence rule applies with force here: **separate what a design documents from
what a blog claims.** This repository's own token-economy record notes that the percentage claims it
once surveyed were blog-sourced and unverified. A number without a paper, a benchmark harness, or a
source read is recorded as unverified or not at all.

### What is already established, and is not in question

* **Defect A** — a task spawned by a worker goes to `injector_local[wid]`; sibling stealing iterates
  the worker **deques** only, so no thread ever polls another thread's local injector. Work spawned
  inside a system body is reachable by its own worker alone. Measured: `par_iter` in a system body
  **1.01×** against **7.69×** for the same driver called from outside. `worker.rs:370-371`.
* **Defect B** — `Scope::drop` steals ~half the wave into a private `scratch` with no registered
  `Stealer` and runs it inline, serially. Even on the healthy path only **4–5 of 16** tasks are ever
  simultaneously live. `scope.rs:440-531`.
* **Inter-system parallelism works** — four conflict-free systems reach four lanes at 25.1 % top
  lane, at W=4 and W=16.

### The owner's proposal, and why it is a third thing rather than a restatement

> *"Perhaps it makes sense to have some queue of free threads. Obviously the first-level system
> scheduler (the one with system ordering) will distribute tasks unevenly between threads, and it
> can happen that some cores go free before others. Then they could be given other work. But again,
> here the question of cache locality arises."*

The observation is correct and is **not** covered by defects A and B: systems have unequal cost, so
lanes drain at different times regardless of how intra-system work is spread. The proposal
decomposes into three mechanisms with different prices, and they must not be conflated:

| # | mechanism | what it needs | the cost to weigh |
|---|---|---|---|
| **1** | an idle worker takes **another system** | the executor must hand out systems **dynamically**; if assignment is static, an idle lane cannot help even when a runnable system exists. **Establish first whether assignment is static or dynamic** — this is unmeasured | conflict-graph re-check per hand-out; a system may be runnable-but-blocked, so idleness is not always fixable |
| **2** | an idle worker takes **intra-system work** from a busy peer | exactly defect A. Already established, already the fix under design | expected cheap: pushing to one's own deque is thread-local, cheaper than the shared-injector push it replaces |
| **3** | a **registry of idle workers** a spawner pushes into directly | push-to-idle instead of poll-to-steal — a genuinely different discipline, not an optimisation of stealing | the registry is shared mutable state on the spawn path; it can cost more than the idleness it removes, which is the owner's own caveat |

### The owner's caveat is the acceptance criterion, and it binds mechanism 3 hardest

> *"If some too-heavy synchronisation is needed that would cost more in performance than the gain
> from maximum core loading, then it should not be done. In short — simply the most performant
> variant."*

⇒ **Occupancy is a diagnostic, not the goal.** A variant that occupies more cores and finishes slower
loses. Mechanism 3 is where this bites: a shared idle-registry touched on every spawn is exactly the
"too-heavy synchronisation" the caveat rules out unless measured otherwise.

The same caveat already applies to **defect B**: `Injector::steal_batch_and_pop` takes half the queue
**to amortise the atomic traffic**. Stealing one task at a time so nothing is ever parked privately
pays a synchronised operation per task, and on short bodies that can cost more than the idle lanes it
recovers. **Keeping B as it is may be the correct answer**, and that outcome must be reportable
rather than treated as a failure to fix.

### Cache locality — the owner named it, and it is the axis that decides mechanism 1

Stealing moves a task's working set across L1/L2. This is why mature pools split the discipline —
LIFO for the owner (hot data first) and FIFO for thieves (oldest, coldest, least likely to be in the
victim's cache) — and why several keep a "last victim" hint. **For mechanism 1 the locality question
is sharper than for 2 or 3**: handing a whole *system* to a different lane moves that system's entire
component working set, and this engine's principles put D-cache locality at the same level as
parallelism. A win in lane occupancy that costs cache residency across a frame may be a net loss, and
nothing here measures that today.

### What the research must cover, when it is launched

* how mature work-stealing pools (Rayon, Tokio's multi-threaded scheduler, Intel TBB, Taskflow, Go's
  runtime) handle **nested spawn from a worker** and **the joining thread**, and which of the three
  mechanisms above each one actually implements;
* **steal granularity** — one task vs a batch vs half the queue — and what the measured trade-off is
  against contention;
* **push-to-idle vs poll-to-steal**: which designs maintain an idle registry, what it costs them, and
  whether any abandoned it;
* the **locality** heuristics: LIFO/FIFO split, last-victim hints, NUMA and core-affinity policies,
  and what evidence exists that they pay;
* ⚠ separate what a design **documents** from what a blog **claims**. This repository has been burned
  by vendor-page numbers, and its own token-economy record says the percentage claims it surveyed
  were blog-sourced and unverified.

### Two false doc comments to correct whenever this is touched

`thread_pool.rs:128-129` and `worker.rs:356` assert siblings see local-injector tasks *"via the
local-injector poll in stage 1.5 of `worker_main`"* — **there is no stage 1.5**. And
`colored.rs:2629-2631` assumes a lane pool of `num_threads + 1` where it is **1**.

**Owner:** unassigned; research pending on the owner's own instruction. Downstream of this row:
**KE15** (dense × `par_iter`) cannot pay off before defect A is fixed, and the O-series colored-solve
numbers need re-taking rather than re-reading once it is.
