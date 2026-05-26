# Phase 12.5 Track A — Spawn-Path Optimisations — Architectural Plan (Round 4)

## §0 Round 4 Changelog

Round 3 verdict was **NEEDS-FIX** with 5 NEW IMPORTANT-level issues (W1, W2, W3, W4, W5). No new criticals. Round 1 → Round 3 resolutions (C1..C4, I1..I6, C-N1..C-N3, I-N1..I-N5) remain unchanged per the critic's "✅ ADDRESSED" verifications. This round addresses each W finding with focused in-place edits.

### W1 — `__private_pin_test::PinTestBundle` body handwaved as `/* trivial */`

**Resolution**: option (b) — `#[derive(Bundle)]` over a stub `#[derive(Component)] struct PinTestComp(u8);`. The user-suggested fallback `()` is **not** viable because the existing `Bundle` trait (`crates/boyko_ecs/src/ecs/core/bundle/bundle.rs:177`) is sealed via `BundleSealed` and has **no `impl Bundle for ()`** in the codebase; adding one is a substantive new design decision outside the scope of W1 fixups.

The chosen route is zero-risk: `derive(Bundle)` is the production code path; the macro emits a `Send + Sync + Unpin + 'static` trait impl by construction (every field is a `Component`-derived type with no `PhantomPinned`). The `assert_impl_all!` is a `const _:` item evaluated at compile time — it does **not** invoke `static_info()`, `cached_archetype_id()`, or any other runtime path, so the lazy `OnceLock<BundleStaticInfo>` registration in the derive's `static_info` body never runs unless the pin-test bundle is actually spawned. §5.2 now contains the concrete code for both stub types.

### W2 — Direct-path §5.5 `Err` leaks `n` IDs from counter

**Resolution**: pre-check capacity via a `Relaxed` load on `next_entity_id` **before** the `fetch_add`. The check returns `Err(WorldEntityCapacityExceeded)` without advancing the counter. `EntityMaster::reserve_batch` keeps the cap-check for `MAX_BATCH_HINT` (SBO17). The combined sequence in `EcsMaster::spawn_batch` is:

1. Validate `n ≤ MAX_BATCH_HINT` (via `EntityMaster::reserve_batch`'s own gate or local check).
2. **Relaxed `load` of `next_entity_id`**; compute `prospective_end = cur + n`; if `prospective_end > entities_inland.len()`, return `Err(WorldEntityCapacityExceeded)` (counter untouched).
3. Only then call `self.entity_master.reserve_batch(n)?` (which performs the actual `fetch_add`).

**TOCTOU non-issue**: between the `load` and the `fetch_add` another thread could in principle advance the counter past `entities_inland.len()`, making the subsequent `fetch_add` overshoot. **But** `EcsMaster::spawn_batch` runs single-threaded under `&mut self` (dispatcher-only — §1.4 + §10.2). No worker can race against it because (a) workers do not hold `&mut EcsMaster`, and (b) the apply window (§10.2) and the direct-path call site are mutually exclusive by Rust's borrow checker. Documented explicitly in §5.5 below.

SBO17 wording is restored to its strong form: "the counter is not advanced when the call returns `Err`" — both for the pre-cap check (`SpawnBatchExceedsCapacity`) and for the new pre-load check (`WorldEntityCapacityExceeded`).

### W3 — `EcsMaster::spawn_batch` returns `Vec<Entity>` — asymmetric heap alloc

**Resolution**: option (a) — keep `Vec<Entity>` and document explicitly. Per §1.4 the direct path is dispatcher-only outside `Schedule` (setup-time use, fixture builds, integration tests). Returning a `Vec` is the ergonomic match for callers who hold all entity IDs in scope (typical setup pattern: `let players = ecs.spawn_batch(...)?;`). The alternative `impl Iterator + ExactSizeIterator + '_` complicates the type signature and forces caller-side `.collect()` for almost every realistic call site, undoing the apparent saving.

§5.5 rustdoc gains an explicit note: "direct path returns `Vec<Entity>` for caller ergonomics; this is a setup-time heap allocation, not a hot-path allocation. The queued path (`Commands::spawn_batch`) does NOT allocate." §1.4 already lists the direct path as out-of-scope for hot-path optimisation.

### W4 — Per-row `debug_assert × 65 536` in 8K-batch tests

**Resolution**: hoist the O(N) `pool_ids.is_sorted_by_key(|p| p.0)` check **outside** the per-row `for i in 0..n` loop. Run it exactly once before the loop. The per-row O(1) check `B::component_ids()[canonical_idx] == component_id` stays inside `for_each_component_bytes` (cheap, well-bounded by `MAX_BUNDLE_ARITY = 8`). §5.4 rewritten: the `debug_assert!` block lives in "Step 2.5" (between Step 2 cache resolution and Step 3 SBO17b guard).

This drops 8192 × O(8) = ~65 k cmp-and-compare ops to 1 × O(8) = 8 ops per batch in debug builds. Release builds are unaffected (debug_assert disappears).

### W5 — `SpawnBatchIter<'a, 's, B, I>` has dead `I` parameter in PhantomData

**Resolution**: drop the `I` type parameter. The iter only walks `range: Range<usize>` yielding `Entity`; the bundle iterator state lives entirely inside the enqueued `SpawnBatchCommand<B, I>` (queue payload), never touched by `SpawnBatchIter`. New signature: `SpawnBatchIter<'a, 's, B>` with `PhantomData<(&'a mut Commands<'s>, B)>`. `Commands::spawn_batch` return type becomes `EcsResult<SpawnBatchIter<'_, 's, B>>` (no `I::IntoIter` leak into the user-visible type). Updated in §5.2 signatures and §10.4 Send/Sync/Unpin table.

---

## §1 Summary, Target Metrics, Scope

### 1.1 Goal

Close the 1.97× spawn-throughput gap to `bevy_ecs` 0.18.1 on the canonical 10 000-entity workload (boyko 248 ns/entity vs bevy 119 ns/entity, ratio 0.51×) and ship the new `Commands::spawn_batch` API with measured ≥1.10× win on a `spawn_batch_10k` benchmark.

Three optimisations, ordered by per-entity attribution and shipped as independent waves:

| Opt | Headline | Per-entity saving (target) | Surface area |
|-----|----------|----------------------------|---------------|
| **A1** | `catch_unwind` hoisting | 5-10 ns | `CommandQueue::apply` loop body |
| **A2** | `Commands::spawn_batch<B, I>` | 80-120 ns (batch path only) | new API + bulk archetype / pool reserve |
| **A3** | `BundleStaticInfo` per-world `InlandPoolId` cache | 10-20 ns (all spawn paths) | `EcsMaster` cache + `Archetype::create_entity` rewrite |

### 1.2 Target metrics (release, AMD Zen3 / Intel Alder Lake, single thread)

| Bench | Today | Target after Track A | Bevy reference | Gate |
|-------|------:|---------------------:|---------------:|------|
| `comparison.rs` g4 `boyko_commands_spawn_10k` (1-comp bundle) | 2.48 ms | ≤ 1.15 ms (≤ 115 ns/entity) | 1.19 ms | parity-or-better single-path |
| **NEW** `spawn_batch_10k_1comp` (chunked 2×5K) | n/a | ≤ 800 µs (≤ 80 ns/entity) | Bevy ~600 µs | boyko ≥ 1.10× bevy on this path |
| **NEW** `spawn_batch_10k_3comp` (chunked 2×5K) | n/a | ≤ 1.4 ms | Bevy ~1.5 ms | boyko ≥ 1.10× bevy |
| `bench_commands_spawn_enqueue` (Phase 11 §13.4) | ≤ 30 ns | ≤ 30 ns (no regression) | — | single-path regression guard |
| `bench_spawn_at_command_apply_warm` (Phase 11) | ≤ 500 ns | ≤ 320 ns (Opt-A3 lifts here) | — | apply-path improvement |
| **NEW** `bench_commands_apply_50_noops` | ~10 µs | ≤ 5 µs (Opt-A1 catch_unwind hoist) | Bevy ~6 µs | I-cache fix |

**Stretch goal**: boyko `spawn_batch_10k_1comp` ≤ 600 µs (parity with Bevy).

**`spawn_batch_10k_1comp` workload**: 10 000 entities spread across **2 batches of 5 000** to stay under `MAX_BATCH_HINT = 8 192`. The bench is representative of real game workflows (spawn waves, level loading); a single 10 K batch would exceed the cap by design.

### 1.3 In-scope deliverables

- **A1**: `RawCommandQueue::apply_or_drop_queued` outer-loop `catch_unwind` refactor with Bevy-parity panic-survivor semantics; **success path uses `set_len(stop_snapshot)` (with command-during-apply compaction) to preserve case-1/3 deferral AND fix case-4 silent-discard bug**.
- **A2**:
  - `pub fn Commands::spawn_batch<B, I: IntoIterator<Item = B>>(iter)` where `I::IntoIter: ExactSizeIterator + Send + Sync + Unpin + 'static`, `B: Bundle + Send + Sync` (Bundle: Unpin via supertrait — SBO-UNPIN).
  - `pub fn EcsMaster::spawn_batch<B, I>(iter) -> EcsResult<Vec<Entity>>` — **dispatcher-only direct path** (`&mut self` receiver). Pre-checks capacity via Relaxed load (W2), then calls `self.entity_master.reserve_batch(n)?` (C-N2 fix).
  - `SpawnBatchCommand<B, I>` queue type with explicit iterator state; auto-derived `Send + Sync + Unpin`; pinned by `assert_impl_all!` **outside `#[cfg(test)]`** (I-N5) using a `#[derive(Bundle)]` pin-test stub (W1).
  - `EntityCounter::reserve_batch(n: usize) -> EcsResult<Range<usize>>` and `EntityMaster::reserve_batch(n) -> EcsResult<Range<usize>>` + `EntityMaster::register_batch(...)`. Both enforce `n ≤ MAX_BATCH_HINT`.
  - `Archetype::reserve_capacity(n) -> EcsResult<()>` (per-pool grow check elision; `EcsError::ArchetypePoolCapacityExceeded` on overflow).
  - `ComponentPool::write_at_unchecked_initialized(row, bytes)` (no grow, no `is_full`).
  - `ComponentPool::commit_units(start_row, count)` + `fill_ticks(start_row, count, tick)`.
  - `ComponentPool::can_reserve(n) -> bool` + `len_for_reserve() -> (usize, usize)` (C-N1).
  - `ComponentPoolBundle::pools_iter` / `pools_iter_mut` / `pool_at_unchecked_mut` / `pools_len` + batch forwarders (C-N1).
  - `slice::fill`-based tick init for the batch path.
  - Runtime aggregate-overshoot guard in `SpawnBatchCommand::apply` (I-N1).
- **A3**:
  - `BundleColumnCache` per-world structure (**eager allocation at `EcsMaster::new`**): `Box<[OnceLock<BundleColumnRecord>; MAX_BUNDLE_TYPES]>` mirror of the existing `bundle_archetype_cache`.
  - `BundleColumnRecord { archetype_id, pool_ids: &'static [InlandPoolId], pools_len_at_install: u32 }` — SBO-N debug check on warm path.
  - `Archetype::create_entity_with_pool_ids(...)`: variant that bypasses 4× SparseMap lookup.
  - Cache invalidation contract: cache slot is keyed by `(BundleTypeId, ArchetypeId)`; populated on first apply per `(B, world)`; lives for the world's lifetime (SBO12 + SBO-N **detection-only** per I-N3).

### 1.4 Out-of-scope (explicit non-goals)

- Bundle batch inserts — Phase 13.
- Sparse-set storage — Phase 13+.
- EntityCommands chaining on the batch path — Phase 13.
- Spawn observer / hook integration — Phase 13+.
- ComponentPool flattening to single-contiguous `Vec<u8>` per pool — Phase 13+.
- Lifting per-system catch_unwind to a global per-frame catch — Track A1 hoists within `CommandQueue::apply` only.
- `spawn_batch_unbounded<I: Iterator>` for non-`ExactSizeIterator` — Phase 13.
- PGO build profile — out of scope.
- Batches larger than `MAX_BATCH_HINT = 8 192` — explicit cap; larger batches must be chunked by the caller.
- **Archetype destruction** — Phase 13 (SBO-N + SBO12 v1 binding).
- **Per-thread reservation pools** — Phase 13 (mitigates aggregate-worker overshoot beyond v1's hard-panic guard).
- **`impl Bundle for ()`** — out of scope (W1 considered and rejected as a pin-test fallback; would constitute a new substantive Bundle design decision).
- **`EcsMaster::spawn_batch` returning an iterator instead of `Vec<Entity>`** — out of scope (W3 decision: direct path is setup-only; `Vec` is the ergonomic match).

### 1.5 Constants and capacity contract

```text
// crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs

/// Maximum entities pre-allocated at `EcsMaster::new` (existing).
const MAX_ENTITIES_HINT: usize = 64_000;

/// Maximum entities a single `spawn_batch` call may reserve.
///
/// `EcsMaster::new` pre-extends entity-fast-store vectors to
/// `MAX_ENTITIES_HINT + MAX_BATCH_HINT` slots so that any spawn_batch call
/// within the hint never reallocates. Workers calling `reserve_batch(n)` with
/// `n > MAX_BATCH_HINT` receive `Err(EcsError::SpawnBatchExceedsCapacity)`.
///
/// Aggregate-worker overshoot (8 workers × MAX_BATCH_HINT near steady-state)
/// is caught by a runtime guard in `SpawnBatchCommand::apply` (I-N1).
///
/// Why 8 192:
///   - 8 192 × ~32 B EntityInland = 256 KB headroom — negligible memory.
///   - Larger batches are real bugs (typical spawn waves are < 1024 entities;
///     >8K screams "iterate twice" or "use chunking").
///   - Bevy has no explicit cap; the `Vec::reserve` it relies on does
///     reallocate, which we cannot afford under SEND5.
const MAX_BATCH_HINT: usize = 8_192;
```

**Capacity guarantee (SBO16)**: after `EcsMaster::new`, `entities_inland.len() == sparse_to_active.len() == MAX_ENTITIES_HINT + MAX_BATCH_HINT`. The batch apply path may write into any slot `[0, MAX_ENTITIES_HINT + MAX_BATCH_HINT)` without reallocating.

**Counter-advance guarantee (SBO17 — restored to strong form per W2)**: `EntityCounter::reserve_batch(n)` and `EntityMaster::reserve_batch(n)` both enforce `n ≤ MAX_BATCH_HINT` **before** any atomic operation. The atomic counter cannot advance by more than `MAX_BATCH_HINT` per call, and **does not advance at all when `Err` is returned**. In addition, `EcsMaster::spawn_batch` (direct path) performs a Relaxed pre-load of `next_entity_id` to detect aggregate-overshoot **before** the fetch_add (W2): on `Err(WorldEntityCapacityExceeded)`, the counter remains untouched.

**Aggregate-overshoot guard (SBO17b — I-N1)**: `SpawnBatchCommand::apply` checks `end_id ≤ entities_inland.len()` at the start of apply; panics with `WorldEntityCapacityExceeded` diagnostic if exceeded. `EcsMaster::spawn_batch` (direct path) detects the same condition **eagerly** via the Relaxed pre-load (W2 augmentation) and returns `Err(WorldEntityCapacityExceeded)` without advancing the counter.

**Beyond the cap**: a user wanting to spawn 100 000 entities must chunk:

```text
for chunk in iter.chunks(MAX_BATCH_HINT - 1):
    commands.spawn_batch(chunk).expect("...").for_each(drop);
```

**`EcsMaster::spawn_batch(70_000)` behaviour**:

```text
let result = ecs.spawn_batch((0..70_000).map(|i| MyBundle::new(i)));
// result: Err(EcsError::SpawnBatchExceedsCapacity { requested: 70_000, max: 8_192 })
// next_entity_id was NOT advanced.
```

---

## §2 Invariants (SBO1..SBO17b + SBO-N + SBO-B2 + SBO-SEND1 + SBO-UNPIN + SBO8b)

### 2.1 New Phase 12.5 invariants

| ID | Statement |
|---|---|
| **SBO1** | `catch_unwind` is invoked at most once per `CommandQueue::apply` walk (twice in `Drop` — one per `bytes`/`recovery` walk, see I4). |
| **SBO2** | On panic mid-apply, the W3' advance discipline (cursor advanced past the panicker before `cmd.apply`) is preserved. Survivors after the panicker, plus any commands the panicker pushed before dying, are captured to `panic_recovery` and redrive on the next apply. **Q-A1.1 cases 1-4 (§3.1)** preserve current behaviour bit-for-bit (case 4 strictly improves correctness). |
| **SBO3** | `SpawnBatchCommand<B, I>` performs ONE atomic `fetch_add(n, Relaxed)` on `EntityMaster::next_entity_id` at enqueue time (worker side, via `EntityCounter::reserve_batch`). Apply consumes the pre-reserved range; no further atomic on the apply path. |
| **SBO4** | `Archetype::reserve_capacity(n)` returns `Ok(())` if every owned pool has `count + n ≤ max_components`. Returns `Err(EcsError::ArchetypePoolCapacityExceeded { archetype_id, n, ... })` on overflow. **Never panics.** |
| **SBO5** | `BundleColumnCache` slots are populated under `&mut EcsMaster` exclusively (apply path). Readers via `&self` observe either `None` (cold) or the fully-published `BundleColumnRecord` (warm) — `OnceLock::get` Acquire load provides publication ordering. |
| **SBO6** | `&'static [InlandPoolId]` slice inside `BundleColumnRecord` is leaked exactly once per `(BundleTypeId, ArchetypeId)` pair per world. Bounded by `MAX_BUNDLE_TYPES × MAX_BUNDLE_ARITY × 4 B = 1024 × 8 × 4 = 32 KB` worst case per world. |
| **SBO7** | `SpawnBatchCommand<B, I>` payload stored in `CommandQueue` carries the iterator state inline (not boxed). Drop-glue path runs `for bundle in iter { drop(bundle) }` on un-flushed queues. Bitwise relocation via `write_unaligned`/`read_unaligned` is sound only for `I: Unpin` (SBO-UNPIN). |
| **SBO8** | `Commands::spawn_batch` returns `EcsResult<SpawnBatchIter<'_, 's, B>>` (W5: no `I` type parameter). On Ok, the iter yields the `n` reserved Entity IDs (range `[start, start+n)`) without consuming the bundle iterator. Apply on flush does the actual spawn. On Err (`SpawnBatchExceedsCapacity`), no reservation occurred. |
| **SBO8b** (I-N2) | Dropping a `SpawnBatchIter` without consuming the entity-ID range has **no semantic effect on the spawn**: the underlying `SpawnBatchCommand` is already enqueued and runs in full on the next apply. Entity IDs remain reserved (counter advanced); not consuming just discards the user-visible ID list. Documented in `Commands::spawn_batch` rustdoc. |
| **SBO9** | Mid-batch bundle-iterator panic: entities `[0..i)` committed; reserved IDs `[i..n)` leak; bundle Drop for un-iterated bundles suppressed per B4. Mid-row panic in `for_each_component_bytes`: archetype `current_index` not bumped; partial bytes in pool buffer unreachable (units.len not extended); reserved ID for row `i` leaks. **No double-drop, no addressable inconsistency.** |
| **SBO10** | `Commands::spawn_batch` is safe to call from any system body whose param set includes `Commands<'s>`. No new component / resource access; no new scheduler conflicts. |
| **SBO11** | `Archetype::push_batch_with_writer(start_row, count, writer)` requires `start_row + count ≤ every pool's max_components` (asserted by a preceding `reserve_capacity`). |
| **SBO12** | The per-world `BundleColumnCache` lives for the world's lifetime. Cache slots are never invalidated in v1: cache slot for `(B, A)` is valid as long as archetype `A` exists, and archetypes are never destroyed (Phase 13 archetype-GC is out of scope). |
| **SBO13** | `ComponentPool::write_at_unchecked_initialized(row, bytes)` writes into an uninitialized slot at `row` (`row ≥ units.len()`, but `row < max_components`). Caller is responsible for `commit_units` and `mark_dirty`. |
| **SBO14** | `EntityMaster::reserve_batch(n)` performs `fetch_add(n, Relaxed)` exactly once after validating `n ≤ MAX_BATCH_HINT`. Returns `Err(SpawnBatchExceedsCapacity)` otherwise. Range is `[old..old+n)`. Workers skip the free list (EM2). |
| **SBO15** | `EntityMaster::register_batch(...)` is dispatcher-only (`&mut self`); writes `entities_inland`, `sparse_to_active`, pushes `active_ids` for all `n` entities in one tight loop. No per-entity bounds re-check (the range was reserved against pre-sized vectors). |
| **SBO16** | After `EcsMaster::new`, `entities_inland.len() == sparse_to_active.len() == MAX_ENTITIES_HINT + MAX_BATCH_HINT == 72_192`. No batch-apply path may write past this; doing so violates SEND5 (caught by SBO17b). |
| **SBO17** (RESTORED W2) | `EntityCounter::reserve_batch(n)` / `EntityMaster::reserve_batch(n)` validate `n ≤ MAX_BATCH_HINT` before any atomic operation. Overrun returns `Err(EcsError::SpawnBatchExceedsCapacity { requested: n, max: MAX_BATCH_HINT })`. **The counter is not advanced when the call returns Err.** Additionally (W2), `EcsMaster::spawn_batch` performs a Relaxed pre-load of `next_entity_id` and returns `Err(WorldEntityCapacityExceeded)` if `cur + n > entities_inland.len()` — again, the counter is not advanced. |
| **SBO17b** (I-N1) | `SpawnBatchCommand::apply` performs a runtime check `end_id ≤ entities_inland.len()` at the start of apply. If exceeded (aggregate-worker overshoot per §10.5), panic with `EcsError::WorldEntityCapacityExceeded { end_id, capacity }` diagnostic. The panic propagates through the queue's outer `catch_unwind` (Opt-A1) → `resume_unwind` → dispatcher boundary. `EcsMaster::spawn_batch` (direct path) avoids this panic by pre-checking via SBO17's Relaxed load. |
| **SBO-N** (I-N3) | Once an archetype is registered for any `BundleTypeId` in the `BundleColumnCache`, that archetype's `ComponentPoolBundle::pools` Vec MUST NOT have entries removed or reordered. Pushes are permitted (they preserve all existing indices). **v1 binding**: archetype destruction is NOT supported in v1. The `pools_len_at_install` debug_assert is **detection-only**, not prevention. Phase 13 archetype-destruction work MUST devise an invalidation mechanism before enabling any pool-removal API. |
| **SBO-B2** | The `pool_ids` array in `BundleColumnRecord` is filled in canonical-sorted `ComponentId.0` order — matching B1/B2 (`bundle.rs:155-159`) order of `B::component_ids()` and `Bundle::for_each_component_bytes`. `debug_assert!(pool_ids.is_sorted_by_key(|p| p.0))` is checked **once per batch at the top of `SpawnBatchCommand::apply`** (W4: hoisted out of the per-row loop), and once at install time inside `BundleColumnCache::resolve_and_cache`. Warm-path indexing is by `canonical_idx` (the iteration counter in `for_each_component_bytes`). |
| **SBO-SEND1** | `SpawnBatchCommand<B, I>: Send + Sync` is **auto-derived** (no hand-written `unsafe impl`). The trait bounds `B: Bundle (Send + Sync + Unpin + 'static)` and `I: ExactSizeIterator<Item = B> + Send + Sync + Unpin + 'static` ensure soundness. Pinned by `static_assertions::assert_impl_all!` over a concrete `#[derive(Bundle)]` pin-test stub (W1) and `std::ops::Range<u32>`, **outside `#[cfg(test)]`** (I-N5). |
| **SBO-UNPIN** (C-N3) | `Bundle: Unpin` is a **trait supertrait**. All bundle types produced by `derive(Bundle)` over named-field structs are `Unpin` by default; manual impls are sealed (`BundleSealed`) so they cannot bypass this. `SpawnBatchCommand<B, I>` requires `I: Unpin`. `SpawnAtCommand<B>`, `InsertCommand<B>`, and other commands that store bundle / iterator state inline in the `CommandQueue` rely on this supertrait so that `write_unaligned`/`read_unaligned` byte-copy through the queue is sound (the queue's bitwise relocation never invalidates self-pointers). |

### 2.2 Reused invariants

- **CQ1..CQ7, CQ-PACK1, CQ-SEND1, CQ-SEND2** (Phase 8d).
- **B1..B4, SBC1..SBC9** (Phase 8.5) — B3 now reads "Bundle: Send + Sync + Unpin + 'static" per SBO-UNPIN.
- **EC1..EC15, EM1..EM6** (Phase 11) — EM6 reinforced by §5.5 routing through `reserve_batch` (C-N2).
- **APP1..APP4** (Phase 8d / 9).
- **ALLOC1..ALLOC6, SEND5, SEND6, SCH7** (Phase 9).
- **CD1..CD5, STORE1..STORE10** (Phase 10).
- **U1, U2, U11, U14** (Phase 7).

---

## §3 Decision Matrix Q-A1.1 .. Q-A3.4

### 3.1 Opt-A1: `catch_unwind` hoisting — Q-A1.1 4-case enumeration (preserved from Round 2)

The Round 2 4×3 case table is preserved unchanged. Summary:

| Case | Today | After Opt-A1 | Diff |
|------|-------|--------------|------|
| (1) Panicker pushes & panics | Pushed cmd in recovery, redriven next apply, Drop NOT run on unwind | Identical | **Identical** |
| (3) Non-panicker pushes; later cmd panics | Pushed cmd in recovery, redriven; Drop NOT run on unwind | Identical | **Identical** |
| (4) Non-panicker pushes; no later panic | **Pushed cmd SILENTLY DISCARDED** (`set_len(start)` truncates) | Pushed cmd survives in bytes, runs on next apply | **DIFF: latent bug FIX** |

Decision: hoist `catch_unwind` AND fix case 4 via `ptr::copy` compaction (preserve the new bytes at `[stop_snapshot..bytes.len())` by shifting down to `[start..]`).

### 3.2 Opt-A2: `SpawnBatchCommand` — open questions (updated for C-N3, W5)

#### Q-A2.1: Iterator bound

**Decision**: `I::IntoIter: ExactSizeIterator + Send + Sync + Unpin + 'static`. The `Unpin` is the C-N3 resolution. Standard iterators (Range, Map, Take) are all trivially `Unpin`; user closures with custom self-references must `.collect()` first.

Bevy parity citation: `bevy_ecs-0.18.1/src/world/spawn_batch.rs`.

#### Q-A2.2: Panic-safety mid-batch (unchanged from Round 2)

Leak-not-double-drop semantics. Bevy parity.

#### Q-A2.3: `CommandQueue` layout for `SpawnBatchCommand<B, I>`

**Decision** (unchanged): store iterator state inline. **C-N3**: relies on `I: Unpin` so bitwise relocation through `write_unaligned`/`read_unaligned` cannot invalidate self-pointers.

Layout (for `I = Range<u32>`):
```text
+0  : start_entity: Entity     (8 B)
+8  : count: u32               (4 B)
+12 : _pad: u32                (4 B)
+16 : iter: I                  (sizeof::<I>())
```

For `I = Range<u32>` (8 B): total 24 B. For ZST `I`: 16 B.

#### Q-A2.4: EntityCounter interaction (unchanged from Round 2)

Atomic interaction: single `fetch_add(n, Relaxed)`; disjoint ranges guaranteed.

#### Q-A2.5: System-body callability (unchanged)

No new scheduler conflicts.

#### Q-A2.6 (NEW W5): `SpawnBatchIter` type parameters

**Decision**: drop `I` from `SpawnBatchIter`. The iter walks `range: Range<usize>` and yields `Entity`; the bundle iterator type is irrelevant to the user-facing iter. New signature `SpawnBatchIter<'a, 's, B>` with phantom `PhantomData<(&'a mut Commands<'s>, B)>`. `B` is retained for ergonomic discoverability (`SpawnBatchIter<'_, '_, EnemyBundle>` reads as "iter of entity IDs spawned from EnemyBundle") and to lock in the spawn-type at the type-system level.

### 3.3 Opt-A3: BundleColumnCache (unchanged from Round 2)

Q-A3.1..Q-A3.4 unchanged.

---

## §4 Opt-A1 — `catch_unwind` Hoisting Design

(Unchanged from Round 2 except cross-references.)

### 4.1 Current implementation footprint

File: `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:264-400`. Per-command `catch_unwind`. Success path `set_len(start)` discards command-during-apply pushes (Q-A1.1 case 4 latent bug).

### 4.2 Proposed implementation

Rename `apply_or_drop_queued` → `apply_or_drop_queued_no_catch`. Move `catch_unwind` to outer caller. Fix success-path with `ptr::copy` compaction:

```text
unsafe fn apply_or_drop_queued_no_catch(&mut self, world):
    let start = *self.cursor.as_ref()
    let stop_snapshot = self.bytes.as_ref().len()
    debug_assert!(start <= stop_snapshot)
    let mut local_cursor = start
    *self.cursor.as_mut() = stop_snapshot  // Bevy: freeze upper bound

    while local_cursor < stop_snapshot:
        let meta = read_unaligned::<CommandMeta>(bytes + local_cursor)
        local_cursor += sizeof::<CommandMeta>()
        let cmd_ptr = bytes + local_cursor
        (meta.consume_and_drop)(cmd_ptr, world, &mut local_cursor)
        // ^ may panic; outer catch_unwind handles it

    // SUCCESS PATH (Q-A1.1 case 4 fix):
    let new_stop = self.bytes.as_mut().len()
    if new_stop > stop_snapshot:
        // Command-during-apply pushes happened. Compact down to start.
        // SAFETY:
        //   - Source [stop_snapshot..new_stop) holds new commands' bytes.
        //   - Destination [start..start + (new_stop - stop_snapshot)) is
        //     reachable Vec capacity (set_len handled below).
        //   - Source and dest may overlap if start + (n - stop_snapshot) > stop_snapshot;
        //     use ptr::copy (handles overlap) NOT copy_nonoverlapping.
        unsafe {
            std::ptr::copy(
                bytes.as_ptr().add(stop_snapshot),
                bytes.as_mut_ptr().add(start),
                new_stop - stop_snapshot,
            )
        }
        bytes.set_len(start + (new_stop - stop_snapshot))
    else:
        bytes.set_len(start)
    *self.cursor.as_mut() = start
```

The outer caller (new `CommandQueue::apply`) wraps in single `catch_unwind`:

```text
pub(crate) fn apply(&mut self, world):
    debug_assert!(self.panic_recovery.is_empty() || !self.bytes.is_empty())
    if self.bytes.is_empty(): return
    let mut raw = self.raw()
    let world_ptr = NonNull::from(&mut *world)
    let walk = AssertUnwindSafe(|| unsafe {
        raw.apply_or_drop_queued_no_catch(Some(world_ptr))
    })
    if let Err(payload) = std::panic::catch_unwind(walk):
        unsafe { raw.handle_panic_recovery(0) }
        std::panic::resume_unwind(payload)
```

### 4.3 Drop path (I4 — two separate walks each catch-wrapped)

```text
impl Drop for CommandQueue:
    fn drop(&mut self):
        if !self.bytes.is_empty():
            let mut raw = self.raw()
            let walk = AssertUnwindSafe(|| unsafe {
                raw.apply_or_drop_queued_no_catch(None)
            })
            let _ = std::panic::catch_unwind(walk)  // swallow

        if !self.panic_recovery.is_empty():
            let mut recovery = mem::take(&mut self.panic_recovery)
            self.bytes.append(&mut recovery)
            let mut raw = self.raw()
            let walk = AssertUnwindSafe(|| unsafe {
                raw.apply_or_drop_queued_no_catch(None)
            })
            let _ = std::panic::catch_unwind(walk)  // swallow
```

### 4.4 `handle_panic_recovery` helper

```text
#[cold]
unsafe fn handle_panic_recovery(&mut self, start: usize):
    let bytes = self.bytes.as_mut()
    let recovery = self.panic_recovery.as_mut()
    let local_cursor = *self.cursor.as_ref()
    let current_stop = bytes.len()
    recovery.extend_from_slice(&bytes[local_cursor..current_stop])
    bytes.set_len(start)
    *self.cursor.as_mut() = start
    if start == 0:
        bytes.append(recovery)
```

### 4.5-4.8: Data structures, cache analysis, concurrency, test plan (unchanged from Round 2)

Test plan adds (preserved from Round 2):
- `apply_pushes_extra_command_on_success_runs_next_apply` (C3 case 4 fix)
- `apply_panicker_pushes_extra_then_panics_extras_go_to_recovery` (C3 case 1)
- `apply_non_panicker_pushes_extra_followed_by_panicker` (C3 case 3)
- `drop_with_panic_in_first_walk_continues_to_recovery_walk` (I4)

---

## §5 Opt-A2 — `SpawnBatchCommand<B, I>` Design

### 5.1 High-level flow

(Unchanged from Round 2; flow is identical.)

### 5.2 Public API (UPDATED W1, W5)

```text
// crates/boyko_ecs/src/ecs/core/commands/spawn_batch_command.rs (NEW FILE)

#[repr(C)]
pub(crate) struct SpawnBatchCommand<B, I>
where
    B: Bundle + Send + Sync,                                      // Bundle: Unpin via supertrait (SBO-UNPIN)
    I: ExactSizeIterator<Item = B> + Send + Sync + Unpin + 'static,  // C-N3: Unpin bound
{
    pub(crate) start_entity: Entity,
    pub(crate) count: u32,
    pub(crate) _pad: u32,
    pub(crate) iter: I,
}

// SBO-SEND1 + SBO-UNPIN: pinned at production build time (I-N5 — NOT in #[cfg(test)]).
use static_assertions::assert_impl_all;

// W1 RESOLUTION: a real, derive(Bundle)-emitted pin-test type.
// The derive macro produces correct Send + Sync + Unpin + 'static impls
// over a single-field Component struct. assert_impl_all! is a compile-time
// `const _:` item; it does NOT trigger any runtime path (no static_info()
// call, no OnceLock init), so the lazy ComponentRegistry / BundleTypeRegistry
// registration is never executed unless the pin-test types are spawned —
// which they are not (the module is doc-hidden and unused outside the assert).
#[doc(hidden)]
pub mod __private_pin_test {
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::bundle::bundle::Bundle;

    /// Pin-test component. Single u8 field — minimal stack footprint.
    /// Never registered or spawned at runtime; exists solely to satisfy
    /// the Bundle field-type requirement.
    #[derive(Component)]
    pub struct PinTestComp(pub u8);

    /// Pin-test bundle. Single PinTestComp field — minimal for derive(Bundle).
    /// Send + Sync + Unpin + 'static emitted by derive macro by construction.
    #[derive(Bundle)]
    pub struct PinTestBundle {
        pub c: PinTestComp,
    }
}

// Pin the auto-derived Send + Sync + Unpin at build time. Fires unconditionally.
assert_impl_all!(
    SpawnBatchCommand<__private_pin_test::PinTestBundle, std::ops::Range<u32>>:
    Send, Sync, Unpin
);

impl<B, I> Command for SpawnBatchCommand<B, I>
where
    B: Bundle + Send + Sync,
    I: ExactSizeIterator<Item = B> + Send + Sync + Unpin + 'static,
{
    fn apply(self, world: &mut EcsMaster);
}
```

```text
// crates/boyko_ecs/src/ecs/core/system/params/commands.rs (EDIT)

impl<'s> Commands<'s> {
    /// Spawns a batch of entities. `iter.len()` must be ≤ MAX_BATCH_HINT (8 192).
    ///
    /// Returns an iterator over reserved `Entity` IDs. Entities are not yet
    /// alive — they become observable after the next `CommandQueue::apply`.
    ///
    /// # Errors
    /// Returns `Err(EcsError::SpawnBatchExceedsCapacity)` if `iter.len() > MAX_BATCH_HINT`.
    /// Chunk larger requests by the caller.
    ///
    /// # Drop semantics (SBO8b — I-N2)
    /// Dropping the returned `SpawnBatchIter` without iterating does NOT
    /// cancel the spawn. The `SpawnBatchCommand` is already enqueued; the
    /// entities are spawned at the next apply regardless. Drop simply
    /// discards the unread Entity IDs (counter has already advanced).
    ///
    /// # Panic safety
    /// If the bundle iterator panics on row i, rows `[0..i)` survive; rows
    /// `[i..n)` are not spawned and their reserved IDs leak. ManuallyDrop
    /// (B4) suppresses double-drop.
    ///
    /// # Aggregate-worker overshoot (SBO17b — I-N1)
    /// If multiple workers near steady-state simultaneously call
    /// `spawn_batch(MAX_BATCH_HINT)`, the per-world counter may advance past
    /// the pre-sized fast-store. Apply will hard-panic with
    /// `WorldEntityCapacityExceeded` — observable failure, not silent UB.
    pub fn spawn_batch<B, I>(
        &mut self,
        iter: I,
    ) -> EcsResult<SpawnBatchIter<'_, 's, B>>                         // W5: no I param
    where
        B: Bundle + Send + Sync,
        I: IntoIterator<Item = B>,
        I::IntoIter: ExactSizeIterator + Send + Sync + Unpin + 'static,
    {
        let iter = iter.into_iter();
        let n = iter.len();
        if n > MAX_BATCH_HINT {
            return Err(EcsError::SpawnBatchExceedsCapacity {
                requested: n,
                max: MAX_BATCH_HINT,
            });
        }
        let start_range = self.entity_counter.reserve_batch(n)?;
        let start_entity = Entity::new(EntityId(start_range.start), 0);
        self.queue.push(SpawnBatchCommand::<B, I::IntoIter> {
            start_entity,
            count: n as u32,
            _pad: 0,
            iter,
        });
        Ok(SpawnBatchIter::new(start_range))
    }
}

/// Iterator over reserved entity IDs returned by `Commands::spawn_batch`.
/// !Send + !Sync (carries lifetime to Commands).
///
/// W5: dropped the dead `I` type parameter. The iter walks `range: Range<usize>`
/// and yields `Entity` — the bundle iterator type is irrelevant to the user
/// surface. Retaining `B` for ergonomic discoverability and type-system locking
/// of the spawn-type.
pub struct SpawnBatchIter<'a, 's, B> {
    range: std::ops::Range<usize>,
    _phantom: PhantomData<(&'a mut Commands<'s>, B)>,
}

impl<'a, 's, B> SpawnBatchIter<'a, 's, B> {
    #[inline]
    pub(crate) fn new(range: std::ops::Range<usize>) -> Self {
        Self { range, _phantom: PhantomData }
    }
}

impl<'a, 's, B> Iterator for SpawnBatchIter<'a, 's, B> {
    type Item = Entity;
    #[inline]
    fn next(&mut self) -> Option<Entity> {
        self.range.next().map(|id| Entity::new(EntityId(id), 0))
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'a, 's, B> ExactSizeIterator for SpawnBatchIter<'a, 's, B> {
    #[inline]
    fn len(&self) -> usize {
        self.range.len()
    }
}
```

```text
// crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs (EDIT)

impl EcsMaster {
    /// Dispatcher-only direct spawn. The `&mut self` borrow precludes
    /// worker access by Rust's borrow checker.
    ///
    /// # Returns
    /// `Vec<Entity>` of length `n` on success. **W3**: the direct path returns
    /// `Vec<Entity>` for caller ergonomics; this is a setup-time heap allocation,
    /// not a hot-path allocation. The queued path (`Commands::spawn_batch`) does
    /// NOT allocate. Typical use: `let players = ecs.spawn_batch(...)?;` at world
    /// setup or fixture construction.
    ///
    /// # Errors
    /// - `SpawnBatchExceedsCapacity` if `iter.len() > MAX_BATCH_HINT`.
    /// - `WorldEntityCapacityExceeded` if the per-world counter has been
    ///   advanced near `entities_inland.len()` and the requested batch would
    ///   overshoot the pre-sized fast-store. **W2**: detected via a Relaxed
    ///   pre-load on `next_entity_id`; on Err the counter is NOT advanced.
    ///
    /// # TOCTOU note (W2)
    /// Between the Relaxed pre-load and the eventual `fetch_add` inside
    /// `EntityMaster::reserve_batch`, another thread could in principle
    /// advance the counter and invalidate the pre-check. **This cannot happen
    /// here**: `EcsMaster::spawn_batch` runs single-threaded under `&mut self`
    /// (dispatcher-only, §10.2). Workers do not hold `&mut EcsMaster`, and the
    /// apply window and direct-path call site are mutually exclusive by Rust's
    /// borrow checker. The race window is closed at the type-system level.
    pub fn spawn_batch<B, I>(&mut self, iter: I) -> EcsResult<Vec<Entity>>
    where
        B: Bundle + Send + Sync,
        I: IntoIterator<Item = B>,
        I::IntoIter: ExactSizeIterator + Send + Sync + Unpin + 'static,
    {
        // ... §5.5 body ...
    }
}
```

### 5.3 `EntityCounter::reserve_batch` (unchanged from Round 2)

```text
impl<'s> EntityCounter<'s> {
    /// Reserves `n` fresh entity IDs atomically. Validates n ≤ MAX_BATCH_HINT
    /// before the atomic — overrun returns Err without advancing the counter.
    #[inline]
    pub fn reserve_batch(&self, n: usize) -> EcsResult<Range<usize>> {
        if n > MAX_BATCH_HINT {
            return Err(EcsError::SpawnBatchExceedsCapacity {
                requested: n,
                max: MAX_BATCH_HINT,
            });
        }
        // SAFETY (EM6): `next_id_ptr` projected from
        // `EntityMaster::next_id_atomic()`; the atomic is 'static for the
        // world's lifetime; Relaxed sufficient for uniqueness.
        let start = unsafe { (*self.next_id_ptr).fetch_add(n, Ordering::Relaxed) };
        debug_assert!(start.checked_add(n).is_some(), "Entity counter overflow");
        Ok(start..(start + n))
    }
}
```

### 5.4 `SpawnBatchCommand::apply` body — detailed (UPDATED W4)

```text
fn apply(self, world: &mut EcsMaster) {
    let n = self.count as usize;
    if n == 0 { return; }

    // ── Step 1: resolve archetype once ──────────────────────────────────
    let archetype_id = B::cached_archetype_id(world);
    let archetype_ptr = world.archetype_master.archetype_ptr_for(archetype_id)
        .expect("invariant: cached_archetype_id returns a registered id");
    let current_tick = world.current_tick();

    // ── Step 2: resolve column ids (Opt-A3 cache) ──────────────────────
    let cache_record = world.bundle_column_cache
        .get_resolved::<B>()
        .unwrap_or_else(|| world.bundle_column_cache.resolve_and_cache::<B>(
            archetype_id,
            unsafe { &*archetype_ptr },
        ));
    let pool_ids = cache_record.pool_ids;

    // ── Step 2.5: once-per-batch SBO-B2 + SBO-N debug invariants (W4) ──
    // These are O(N) over pool_ids.len() (≤ MAX_BUNDLE_ARITY = 8). Running
    // them ONCE per batch instead of per-row drops 8192 × O(8) = ~65k cmp ops
    // to 1 × O(8) = 8 ops in debug builds. Release builds: zero cost.
    //
    // SAFETY (SBO-N + SBO-B2 + B2):
    //   - SBO-N: pools Vec is push-only. Warm-path debug_assert verifies non-decrease.
    //   - SBO-B2: pool_ids is in canonical-sorted ComponentId.0 order,
    //     verified at install time (resolve_and_cache) AND once here.
    //   - B2 (bundle.rs:155-159): for_each_component_bytes emits in
    //     same canonical order ⇒ pool_ids[canonical_idx] indexes correctly.
    //   - B::component_ids().len() == pool_ids.len() (cache invariant).
    let pools_len = unsafe { &*archetype_ptr }.component_pools().pools_len();  // C-N1
    debug_assert!(
        cache_record.pools_len_at_install as usize <= pools_len,
        "SBO-N violation: pools Vec shrunk after cache install"
    );
    debug_assert!(
        pool_ids.is_sorted_by_key(|p| p.0),
        "SBO-B2 violation: pool_ids must be in canonical-sorted order"
    );
    debug_assert!(pool_ids.len() == B::component_ids().len());

    // ── Step 3: SBO17b runtime guard (I-N1) ─────────────────────────────
    let start_id = self.start_entity.id().0;
    let end_id = start_id + n;
    let capacity = world.entity_master.entities_inland.len();
    if end_id > capacity {
        // Aggregate-worker overshoot. Hard panic — observable failure,
        // NOT silent SEND5 violation. This catches the edge case that
        // SBO16/SBO17 alone cannot prevent (multiple workers near
        // steady-state each calling reserve_batch(MAX_BATCH_HINT)).
        panic!(
            "{}",
            EcsError::WorldEntityCapacityExceeded { end_id, capacity }
        );
    }
    world.entity_master.active_ids.reserve(n);  // amortized

    // SAFETY (U1, U2, U14): archetype_ptr is write-capable + stable slab.
    // We hold &mut EcsMaster (apply window) so no aliasing.
    let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };

    // ── Step 4: reserve pool capacity (I-N4: expect, not ?) ─────────────
    // SBO17 cap-check at enqueue is authoritative; failure here = logic bug.
    archetype.reserve_capacity(n).expect(
        "apply contract: SpawnBatchCommand reservation should have been validated \
         at enqueue (SBO17 cap-check); overrun here indicates an aggregate-worker \
         overshoot or a logic bug in enqueue."
    );
    let start_row = archetype.current_index;

    // ── Step 5: write rows (bundle iteration) ──────────────────────────
    // W4: per-row debug_asserts are O(1) only — no is_sorted_by_key, no
    // pools_len_at_install check. Those run once at Step 2.5.
    let mut iter = self.iter;
    for i in 0..n {
        let row = start_row + i;
        let bundle = iter.next()
            .expect("ExactSizeIterator contract: len() reported n, iter yielded < n");
        let mut canonical_idx = 0;
        bundle.for_each_component_bytes(|component_id, bytes| {
            debug_assert!(canonical_idx < pool_ids.len());
            // O(1) per-component check — bounded by MAX_BUNDLE_ARITY = 8.
            debug_assert!(
                B::component_ids()[canonical_idx] == component_id,
                "B2/SBO-B2 violation: bundle emit order mismatch"
            );
            let pool_idx = pool_ids[canonical_idx];
            // SAFETY (SBO13):
            //   - row < start_row + n ≤ max_components after reserve_capacity.
            //   - Pool is exclusively accessed via &mut archetype.
            //   - bytes.len() matches component_layout.size() (B + macro contract).
            unsafe {
                archetype.component_pools_mut()
                    .pool_at_unchecked_mut(pool_idx)
                    .write_at_unchecked_initialized(row, bytes);
            }
            canonical_idx += 1;
        });
        debug_assert!(canonical_idx == pool_ids.len());
    }

    // ── Step 6: bulk-commit units + tick init via slice::fill ──────────
    archetype.component_pools_mut().commit_units_batch(start_row, n);
    archetype.component_pools_mut().fill_ticks_batch(start_row, n, current_tick);

    // ── Step 7: archetype-level bookkeeping ────────────────────────────
    for i in 0..n {
        archetype.entity_ids.push(EntityId(start_id + i));
    }
    archetype.current_index = start_row + n;

    // ── Step 8: bulk-register entities ────────────────────────────────
    world.entity_master.register_batch(
        EntityId(start_id),
        archetype_ptr,
        start_row as u32,
        n,
    );
}
```

### 5.5 `EcsMaster::spawn_batch` direct path — UPDATED W2 (pre-check before fetch_add)

```text
impl EcsMaster {
    /// Dispatcher-only direct spawn. `&mut self` precludes concurrent worker
    /// access. Returns Err on `n > MAX_BATCH_HINT` (caller must chunk) or on
    /// aggregate overshoot detected via the Relaxed pre-load (W2).
    pub fn spawn_batch<B, I>(&mut self, iter: I) -> EcsResult<Vec<Entity>>
    where
        B: Bundle + Send + Sync,
        I: IntoIterator<Item = B>,
        I::IntoIter: ExactSizeIterator + Send + Sync + Unpin + 'static,
    {
        let iter = iter.into_iter();
        let n = iter.len();
        if n == 0 { return Ok(Vec::new()); }

        // ── W2 PRE-CHECK 1: MAX_BATCH_HINT cap (mirrors reserve_batch's gate
        //                   but checked here so we can short-circuit before
        //                   touching the counter at all).
        if n > MAX_BATCH_HINT {
            return Err(EcsError::SpawnBatchExceedsCapacity {
                requested: n,
                max: MAX_BATCH_HINT,
            });
        }

        // ── W2 PRE-CHECK 2: Relaxed load → would this overshoot the
        //                   pre-sized fast-store? If yes, return Err
        //                   WITHOUT advancing the counter (SBO17 strong form).
        //
        // TOCTOU non-issue: the &mut self receiver guarantees this code path
        // runs single-threaded vs any other worker on this world. Workers
        // never hold &mut EcsMaster; the apply window and direct-path are
        // mutually exclusive by Rust's borrow checker. No race window exists
        // between this load and the subsequent fetch_add inside reserve_batch.
        let cur = self.entity_master.next_id_atomic().load(Ordering::Relaxed);
        let capacity = self.entity_master.entities_inland.len();
        let prospective_end = cur.checked_add(n)
            .ok_or(EcsError::WorldEntityCapacityExceeded { end_id: usize::MAX, capacity })?;
        if prospective_end > capacity {
            return Err(EcsError::WorldEntityCapacityExceeded {
                end_id: prospective_end,
                capacity,
            });
        }

        // ── Now safe to advance the counter. Route through EntityMaster
        //    (C-N2: never poke `next_entity_id` directly — EM6 preserved).
        let start_range = self.entity_master.reserve_batch(n)?;
        let start_entity = Entity::new(EntityId(start_range.start), 0);

        // Build an equivalent SpawnBatchCommand and apply inline.
        // The pre-checks above guarantee SBO17b's runtime guard inside apply
        // will NOT fire — the apply runs the same code path as the queued
        // command but is panic-free for the direct caller.
        let cmd = SpawnBatchCommand::<B, I::IntoIter> {
            start_entity,
            count: n as u32,
            _pad: 0,
            iter,
        };
        cmd.apply(self);

        let mut result = Vec::with_capacity(n);
        for i in 0..n {
            result.push(Entity::new(EntityId(start_range.start + i), 0));
        }
        Ok(result)
    }
}
```

**Note on direct-path vs queued-command behavior on overshoot**:
- **Queued** (`Commands::spawn_batch` → `SpawnBatchCommand::apply`): overshoot panics (apply-contract); the worker thread cannot pre-check because workers race against each other.
- **Direct** (`EcsMaster::spawn_batch`): overshoot returns `Err(WorldEntityCapacityExceeded)` (Result-contract); the pre-check is sound because `&mut self` closes the race window (W2).

### 5.6 New methods on `Archetype` / `ComponentPool` / `ComponentPoolBundle` (unchanged from Round 3)

(Body unchanged — see Round 3 §5.6. All accessors stay: `can_reserve`, `len_for_reserve`, `write_at_unchecked_initialized`, `commit_units`, `fill_ticks`, `pools_iter`, `pools_iter_mut`, `pools_len`, `pool_id_for`, `pool_at_unchecked_mut`, `commit_units_batch`, `fill_ticks_batch`. Per C-N1.)

### 5.7 `EntityMaster` API: `register_batch` + `reserve_batch` (unchanged from Round 2)

(Body unchanged. `register_batch` writes `entities_inland` / `sparse_to_active` / `active_ids` in tight loop; `reserve_batch` enforces SBO17 cap + Relaxed `fetch_add`.)

Additional accessor required by W2 (already exists per Phase 11 `entity_master.rs:147`):
```text
impl EntityMaster {
    /// Returns a reference to the next-entity-id atomic counter.
    /// Used by EntityCounter (worker side) AND by EcsMaster::spawn_batch
    /// for the W2 Relaxed pre-load — both legitimate dispatcher-controlled
    /// uses; no EM6 violation (EM6 forbids `&mut` direct mutation of the
    /// counter, not Relaxed reads).
    pub(crate) fn next_id_atomic(&self) -> &AtomicUsize { &self.next_entity_id }
}
```

### 5.8 Data layout and memory analysis (unchanged from Round 2)

`SpawnBatchCommand<B, I>` for `I = Range<u32>` (8 B): 24 B + 8 B CommandMeta = 32 B per slot.

### 5.9 Concurrency analysis (unchanged from Round 2 + W2 augmentation in §5.5)

### 5.10 Worker spawn_batch from system body (unchanged from Round 2)

### 5.11 Test plan (UPDATED W1, W2, W4, W5)

| Test | File | Scenario |
|------|------|----------|
| `spawn_batch_one_thousand_one_component` | tests/spawn_batch_smoke.rs (new) | 1000 entities, all alive after apply |
| `spawn_batch_three_component_bundle` | same | 1000 × 3-comp bundle; verify pool counts |
| `spawn_batch_empty_iter_is_noop` | same | `std::iter::empty()` → no entities |
| `spawn_batch_returns_entity_ids_before_apply` | same | `.collect::<Vec<_>>()` returns 1000 IDs |
| `spawn_batch_ids_invalid_until_apply` | same | `is_entity_valid(id) == false` between spawn and apply |
| `spawn_batch_ids_valid_after_apply` | same | after apply, all 1000 valid |
| `spawn_batch_panic_in_iterator_leaks_remaining_ids` | same | `(0..1000).map(panic_at_500)` → 500 entities live |
| `spawn_batch_panic_in_bundle_for_each_leaves_consistent_archetype` | same | row-mid panic; archetype `current_index` not bumped |
| `direct_spawn_batch_eager_path` | same | `ecs.spawn_batch(...)` returns synchronously with Vec |
| `spawn_batch_exceeds_max_batch_hint_returns_err` | same | `spawn_batch(8193)` returns `SpawnBatchExceedsCapacity` |
| `spawn_batch_at_max_batch_hint_succeeds` | same | `spawn_batch(8192)` succeeds |
| `spawn_batch_chunked_to_70k_via_loop` | same | 9 chunks of 8K spawn 70K entities; SEND5 preserved |
| `reserve_batch_at_cap_does_not_advance_counter_on_err` | tests/entity_counter_smoke.rs | overflow check happens BEFORE atomic |
| `entity_counter_reserve_batch_distinct_ranges` | same | 4 threads × reserve_batch(1000) → 4 disjoint ranges |
| `archetype_reserve_capacity_succeeds_within_pool_limit` | archetype.rs tests | reserve_capacity(100) freshly-created |
| `archetype_reserve_capacity_returns_err_above_pool_limit` | same | reserve_capacity(MAX+1) → Err (NOT panic) |
| `component_pool_write_at_unchecked_initialized_roundtrip` | component_pool.rs tests | write then read |
| `entity_master_register_batch_round_trip` | entity_master.rs tests | register_batch(1000) → 1000 valid |
| `miri_spawn_batch_one_thousand` | tests/miri_phase12_5.rs (new) | no SB/TB UB |
| `miri_spawn_batch_panic_in_iter` | same | mid-batch panic; no leak of bundle bytes with Drop |
| `miri_entity_counter_reserve_batch_parallel` | same | atomic counter race-free |
| `loom_reserve_batch_no_collision` | tests/loom_phase12_5.rs (new, cfg-gated) | 2 threads × reserve_batch(10); disjoint ranges |
| `ecs_master_spawn_batch_uses_reserve_batch_no_direct_atomic` (C-N2) | tests/spawn_batch_smoke.rs | grep-test verifying `EcsMaster::spawn_batch` body does not contain `next_entity_id.fetch_add` |
| `spawn_batch_aggregate_overshoot_panics_at_apply` (I-N1) | same | construct world where pre-sized capacity is exhausted by 2 concurrent spawn_batch commands; queued apply panics |
| **`spawn_batch_direct_aggregate_overshoot_returns_err_no_counter_advance`** (NEW W2) | same | drive counter near `entities_inland.len()`; call `ecs.spawn_batch(n)` with n that would overshoot; verify (a) Err returned, (b) `next_id_atomic().load()` unchanged from before the call |
| `spawn_batch_iter_drop_without_consume_still_spawns` (I-N2) | same | drop `SpawnBatchIter` immediately; apply runs; entities exist |
| `assert_unpin_pin_on_spawn_batch_command` (C-N3 + I-N5) | tests/spawn_batch_smoke.rs | compile-time pin via `assert_impl_all!` (production build, NOT cfg(test)) |
| `bundle_unpin_supertrait_enforced` (C-N3) | trybuild test | a struct with `PhantomPinned` field fails `derive(Bundle)` |
| **`spawn_batch_iter_type_signature_no_bundle_iter_leak`** (NEW W5) | tests/spawn_batch_smoke.rs | compile-only assertion that `Commands::spawn_batch::<MyBundle, _>(...)` return type names only `SpawnBatchIter<'_, '_, MyBundle>`, not `SpawnBatchIter<'_, '_, MyBundle, std::iter::Map<...>>`. Concretely: `let _: SpawnBatchIter<'_, '_, MyBundle> = commands.spawn_batch(...).unwrap();` compiles. |
| **`pin_test_bundle_assert_impl_all_compiles`** (NEW W1) | tests/spawn_batch_smoke.rs | trivial test verifying the `assert_impl_all!` line in spawn_batch_command.rs compiles in the production build; failure mode is build-break, not test-fail |
| `bench_spawn_batch_5k_1comp_vs_bevy` | comparison.rs | boyko ≥ 1.10× bevy |
| `bench_spawn_batch_5k_3comp_vs_bevy` | same | head-to-head |
| `bench_spawn_batch_apply_warm_per_entity` | new bench | per-entity ≤ 60 ns (1-comp) / ≤ 100 ns (3-comp) |
| **`bench_spawn_batch_8k_debug_assert_overhead`** (NEW W4 perf sanity) | benches/phase12_5_debug_overhead.rs (new, debug build only) | measure 8k-batch debug-build wall time before vs after W4 hoist; expect ≥10× faster debug runs (assertion math: 65 k → 8 cmp ops) |

### 5.12 Step plan (Wave A2 — UPDATED W1, W2, W4, W5)

#### Sub-wave A2a: foundations (independent files; parallel)

1. **A2-Step 1**: `EntityCounter::reserve_batch` with cap (~12 lines).
2. **A2-Step 2**: `EntityMaster::reserve_batch` + `register_batch` (~50 lines). Add (or confirm) `pub(crate) fn next_id_atomic(&self) -> &AtomicUsize` accessor for W2.
3. **A2-Step 3** (C-N1): `Archetype::reserve_capacity` (EcsResult) + `ComponentPool::can_reserve` + `len_for_reserve` + `write_at_unchecked_initialized` + `commit_units` + `fill_ticks` (~90 lines).
4. **A2-Step 4** (C-N1): `ComponentPoolBundle::pools_iter` + `pools_iter_mut` + `pools_len` + `pool_at_unchecked_mut` + batch forwarders (~50 lines).

#### Sub-wave A2b: dispatch (depends on Opt-A3)

5. **A2-Step 5** (UPDATED W1, W4, W5): `SpawnBatchCommand<B, I>` struct + `Command` impl + auto-derived Send/Sync/Unpin + `assert_impl_all!` outside `#[cfg(test)]` (I-N5) using a `#[derive(Component)] struct PinTestComp(u8); #[derive(Bundle)] struct PinTestBundle { c: PinTestComp }` stub (W1 — concrete code, not `/* trivial */`). Apply body uses Step 2.5 hoisted SBO-B2 debug_assert (W4). File: `commands/spawn_batch_command.rs` (new, ~150 lines).
6. **A2-Step 6** (C-N2 + W2): `EcsMaster::spawn_batch::<B, I>` direct path with W2 Relaxed pre-load → `Err(WorldEntityCapacityExceeded)` without advancing counter, then routes through `self.entity_master.reserve_batch(n)?`.

#### Sub-wave A2c: Commands API + benches

7. **A2-Step 7** (W5): `SpawnBatchIter<'a, 's, B>` (W5: dropped `I` parameter) + `Commands::spawn_batch` API returning `EcsResult<SpawnBatchIter<'_, 's, B>>` + rustdoc with SBO8b drop semantics.
8. **A2-Step 8** (W1, W2, W4, W5): Integration tests in `tests/spawn_batch_smoke.rs` including pin-bundle-compiles (W1), direct-path no-counter-advance on Err (W2), W4 debug-overhead bench (release-build excluded — assertion sanity only), W5 type-signature compile test.
9. **A2-Step 9**: Criterion benches.

Total: ~800 lines new code across 6 files. ~3 PRs.

---

## §6 Opt-A3 — `BundleColumnCache` Design

### 6.1 Goal recap

Eliminate the 3× SparseMap lookup per component in `Archetype::create_entity` by pre-resolving `ComponentId → InlandPoolId` once per `(B, world)` pair.

### 6.2 Public API (unchanged from Round 3)

(Body unchanged from Round 3 §6.2 — `BundleColumnRecord` + `BundleColumnCache` with `new` / `get_resolved` / `resolve_and_cache` using `ComponentPoolBundle::pool_id_for` accessor per C-N1. `pool_ids` field stays `&'static [InlandPoolId]`. `resolve_and_cache` runs its own `is_sorted_by_key` debug_assert at install time — independent from the W4 hoisted per-batch check.)

### 6.3 Integration with `SpawnBatchCommand::apply` (§5.4 — unchanged structurally; W4 hoisted debug_assert lives in Step 2.5)

### 6.4 Integration with `SpawnAtCommand::apply` (UPDATED C-N1; W4 doesn't apply — single-row path)

(Body unchanged from Round 3 §6.4. `SpawnAtCommand` is single-row; per-call debug_asserts are O(1), not O(N), so W4 hoisting is irrelevant here.)

### 6.5 Cache analysis (unchanged from Round 2)

### 6.6 Concurrency analysis (unchanged from Round 2)

### 6.7 Cache validation: SBO5 + SBO12 + SBO-N + SBO-B2 (UPDATED C-N1, I-N3, W4)

- **SBO5**: cache writes under `&mut EcsMaster`. Reads observe NULL or fully-published. ✅
- **SBO12 (v1 binding)**: cache slot valid for world's lifetime. ✅ (no archetype destruction in v1)
- **SBO-N (I-N3)**: pools Vec is push-only — `InlandPoolId` indices stay valid. Warm-path debug_assert verifies monotonic non-decrease at Step 2.5 (W4: hoisted out of per-row loop). **Detection-only**, not prevention. Phase 13 archetype-destruction work MUST add invalidation (Open Question OQ12). ✅
- **SBO-B2**: pool_ids is canonical-sorted at install. Warm-path debug_assert verifies **once per batch** at Step 2.5 (W4). ✅

Warm-path check uses `archetype.component_pools().pools_len()` (C-N1) rather than the private field.

### 6.8 Test plan (unchanged from Round 2)

### 6.9 Step plan (Wave A3) — unchanged from Round 3

(7-step plan unchanged. W4 hoisting is contained inside `SpawnBatchCommand::apply` and does not change the A3 step plan.)

---

## §7 Public API Surface

### 7.1 New types (UPDATED W1, W5)

```text
// commands/spawn_batch_command.rs (NEW)
pub(crate) struct SpawnBatchCommand<B, I> { /* §5.2 */ }
// Send + Sync + Unpin auto-derived.
// Pinned by assert_impl_all! over a concrete derive(Bundle) stub (W1)
//   OUTSIDE #[cfg(test)] (I-N5).

// commands/spawn_batch_iter.rs (NEW)
pub struct SpawnBatchIter<'a, 's, B> { /* §5.2 — W5: no I param */ }   // !Send + !Sync

// bundle/bundle_column_cache.rs (NEW)
pub struct BundleColumnCache { /* §6.2 */ }              // Send + Sync (pinned outside cfg(test))
pub struct BundleColumnRecord { /* §6.2 */ }             // Send + Sync, 32 B
```

### 7.2 Modified APIs (UPDATED W2, W3, W5)

```text
// Bundle trait: Unpin supertrait (SBO-UNPIN / C-N3)
pub trait Bundle: sealed::BundleSealed + Send + Sync + Unpin + 'static {
    // ... existing methods unchanged ...
}

impl<'s> Commands<'s> {
    // W5: return type drops the I param.
    pub fn spawn_batch<B, I>(
        &mut self,
        iter: I,
    ) -> EcsResult<SpawnBatchIter<'_, 's, B>>
    where
        B: Bundle + Send + Sync,
        I: IntoIterator<Item = B>,
        I::IntoIter: ExactSizeIterator + Send + Sync + Unpin + 'static;
}

impl<'s> EntityCounter<'s> {
    pub fn reserve_batch(&self, n: usize) -> EcsResult<Range<usize>>;
}

impl EntityMaster {
    pub(crate) fn reserve_batch(&self, n: usize) -> EcsResult<Range<usize>>;
    pub(crate) fn register_batch(
        &mut self,
        start_entity: EntityId,
        archetype_ptr: *mut Archetype,
        start_row: u32,
        n: usize,
    );
    // W2: needed for EcsMaster::spawn_batch's Relaxed pre-load.
    // Returns a reference, not the value — EM6 unchanged (no &mut mutation surface).
    pub(crate) fn next_id_atomic(&self) -> &AtomicUsize;
}

impl EcsMaster {
    // W3 documented: Vec<Entity> ergonomic return; setup-time alloc, not hot path.
    // W2 documented: Relaxed pre-load → Err without counter advance; TOCTOU
    //                closed by &mut self.
    pub fn spawn_batch<B, I>(&mut self, iter: I) -> EcsResult<Vec<Entity>>
    where
        B: Bundle + Send + Sync,
        I: IntoIterator<Item = B>,
        I::IntoIter: ExactSizeIterator + Send + Sync + Unpin + 'static;
}

impl Archetype {
    pub(crate) fn reserve_capacity(&mut self, n: usize) -> EcsResult<()>;
}

impl ComponentPool {
    // C-N1 new accessors:
    pub(crate) fn can_reserve(&self, n: usize) -> bool;
    pub(crate) fn len_for_reserve(&self) -> (usize, usize);

    // Existing-style new methods:
    pub(crate) unsafe fn write_at_unchecked_initialized(&mut self, idx: usize, bytes: &[u8]);
    pub(crate) fn commit_units(&mut self, start_row: usize, count: usize);
    pub(crate) fn fill_ticks(&mut self, start_row: usize, count: usize, tick: Tick);
}

impl ComponentPoolBundle {
    // C-N1 new accessors:
    pub(crate) fn pools_iter(&self) -> impl Iterator<Item = &ComponentPool>;
    pub(crate) fn pools_iter_mut(&mut self) -> impl Iterator<Item = &mut ComponentPool>;
    pub(crate) fn pools_len(&self) -> usize;
    pub(crate) fn pool_id_for(&self, component_id: ComponentId) -> Option<InlandPoolId>;
    pub(crate) unsafe fn pool_at_unchecked_mut(&mut self, pool_idx: InlandPoolId) -> &mut ComponentPool;
    pub(crate) fn commit_units_batch(&mut self, start_row: usize, n: usize);
    pub(crate) fn fill_ticks_batch(&mut self, start_row: usize, n: usize, tick: Tick);
}
```

### 7.3 Modified `EcsMaster` structure (unchanged from Round 2)

```text
pub struct EcsMaster {
    pub(crate) resources: Resources,
    events: EventDispatcher,
    pub(crate) entity_master: EntityMaster,
    archetype_master: ArchetypeMaster,
    bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>,
    bundle_column_cache: BundleColumnCache,   // NEW (Opt-A3, eager — I3)
    pub(crate) change_tick: AtomicU32,
    pub(crate) last_check_tick: Tick,
    arena: Box<Arena>,
}
```

### 7.4 New error variants (UPDATED I-N1)

```text
pub enum EcsError {
    // ... existing variants ...
    SpawnBatchExceedsCapacity { requested: usize, max: usize },          // (C1, I5, I6)
    ArchetypePoolCapacityExceeded {                                       // (I5)
        archetype_id: ArchetypeId,
        pool_capacity: usize,
        requested: usize,
    },
    WorldEntityCapacityExceeded { end_id: usize, capacity: usize },      // (I-N1, W2)
}
```

`WorldEntityCapacityExceeded` Display impl:
```text
EcsError::WorldEntityCapacityExceeded { end_id, capacity } => {
    write!(f, "spawn_batch aggregate overshoot: end_id {} exceeds pre-sized capacity {} \
               (SBO16+SBO17b); reduce concurrent workers or chunk further", end_id, capacity)
}
```

### 7.5 Usage examples (UPDATED W5)

```text
// Batch spawn 1000 enemies. W5: return type is SpawnBatchIter<'_, '_, EnemyBundle>,
// not SpawnBatchIter<'_, '_, EnemyBundle, std::iter::Map<...>>.
fn spawn_enemy_wave(mut commands: Commands) {
    let entities: Vec<Entity> = commands
        .spawn_batch((0..1000).map(|i| EnemyBundle { ... }))
        .expect("1000 ≤ MAX_BATCH_HINT")
        .collect();
}

// Fire-and-forget batch spawn — IDs not needed (SBO8b — I-N2).
fn spawn_decorations(mut commands: Commands) {
    let _ = commands
        .spawn_batch((0..500).map(|i| DecorationBundle { ... }))
        .expect("500 ≤ MAX_BATCH_HINT");
    // SpawnBatchIter dropped here; spawn still happens at next apply.
}

// Larger batch must be chunked.
fn spawn_horde(mut commands: Commands) {
    for chunk_start in (0..70_000).step_by(MAX_BATCH_HINT - 1) {
        let end = (chunk_start + MAX_BATCH_HINT - 1).min(70_000);
        let _ = commands
            .spawn_batch((chunk_start..end).map(|i| EnemyBundle { ... }))
            .expect("chunk size ≤ MAX_BATCH_HINT")
            .for_each(drop);
    }
}

// Dispatcher direct path (W3: returns Vec<Entity>, setup-time alloc OK).
fn setup_world(ecs: &mut EcsMaster) {
    let players: Vec<Entity> = ecs.spawn_batch(
        (0..4).map(|i| PlayerBundle { ... })
    ).expect("4 ≤ MAX_BATCH_HINT");
}
```

---

## §8 Memory Layouts + Sizes

(Unchanged from Round 2.)

### 8.3 `BundleColumnRecord`: 32 B (with `pools_len_at_install` SBO-N field).

### 8.4 `SpawnBatchIter<'a, 's, B>` (UPDATED W5)

**W5 effect**: `SpawnBatchIter` was `<'a, 's, B, I>` with `PhantomData<(&'a mut Commands<'s>, B, I)>`. Dropping `I` reduces the type's nominal generic count from 4 to 3 (lifetimes + 1 type param). PhantomData is ZST in both versions; layout is unchanged (still `{ range: Range<usize>, _phantom: ZST }` = 16 B). **The fix is a type-signature simplification, not a layout change.** Callers no longer pay the "unnameable iterator type" tax in their function signatures or type ascriptions.

### 8.6 Heap memory totals per world: ~82.4 MB (same as Round 2).

---

## §9 Implementation Steps and New Accessors (UPDATED W2)

### 9.1 New accessors required for plan to compile

Per C-N1, the plan consumes these new methods. Each is `pub(crate)` (consistent with `pub(crate)` on `added_ticks`/`changed_ticks` and `entities_inland`/`sparse_to_active`).

**On `ComponentPool`**: `can_reserve`, `len_for_reserve`, `write_at_unchecked_initialized`, `commit_units`, `fill_ticks`. (Unchanged.)

**On `ComponentPoolBundle`**: `pools_iter`, `pools_iter_mut`, `pools_len`, `pool_id_for`, `pool_at_unchecked_mut`, `commit_units_batch`, `fill_ticks_batch`. (Unchanged.)

**On `EntityMaster`**:
```text
impl EntityMaster {
    pub(crate) fn reserve_batch(&self, n: usize) -> EcsResult<Range<usize>>;
    pub(crate) fn register_batch(
        &mut self,
        start_entity: EntityId,
        archetype_ptr: *mut Archetype,
        start_row: u32,
        n: usize,
    );
    // W2: NEW — exposes the atomic for Relaxed pre-load only (no &mut access).
    pub(crate) fn next_id_atomic(&self) -> &AtomicUsize;
}
```

**On `Archetype`**: `reserve_capacity`. (Unchanged.)

No `pub` accessor change to existing public methods.

### 9.2 Implementation order summary

(See §5.12 + §6.9 for per-step breakdown.)

---

## §10 Multithreading Model

### 10.1 New worker-side surface

`EntityCounter::reserve_batch(n)`:
- Atomic `fetch_add(n, Relaxed)` (validated `n ≤ MAX_BATCH_HINT`).
- Returns Err without atomic if cap exceeded.
- Data-race-free across N threads.

`Commands::spawn_batch(iter)`:
- Calls `EntityCounter::reserve_batch(n)` once.
- Pushes into per-system `CommandQueue` (single-writer per CQ5).
- Returns `EcsResult<SpawnBatchIter<'_, 's, B>>` — `!Send + !Sync` (W5: no I param).

### 10.2 Apply-time invariants + dispatcher-only direct path (UPDATED W2)

`SpawnBatchCommand::apply`, `SpawnAtCommand::apply`, `BundleColumnCache::resolve_and_cache` all run on the dispatcher under `&mut EcsMaster`.

`SpawnBatchCommand::apply` performs the **SBO17b runtime guard** at start: panic with `WorldEntityCapacityExceeded` if `end_id > entities_inland.len()`. Aggregate-worker overshoot becomes observable, not silent SEND5 violation.

**`EcsMaster::spawn_batch` (direct path)** — dispatcher-only; the `&mut self` receiver enforces this at compile-time. **W2 augmentation**: a Relaxed pre-load on `next_entity_id` returns `Err(WorldEntityCapacityExceeded)` **without** advancing the counter. The pre-check is race-free because `&mut self` precludes worker access (TOCTOU closed by Rust's borrow checker).

### 10.3 Memory ordering summary (UPDATED W2)

| Operation | Order |
|---|---|
| `EntityCounter::reserve_batch::fetch_add(n, Relaxed)` | Relaxed |
| `EntityMaster::reserve_batch::fetch_add(n, Relaxed)` (dispatcher, C-N2) | Relaxed |
| **`EcsMaster::spawn_batch::next_id_atomic().load(Relaxed)`** (W2 pre-check) | Relaxed |
| `BundleColumnCache::resolve_and_cache::set` | Release (OnceLock) |
| `BundleColumnCache::get_resolved::get` | Acquire (OnceLock) |
| `next_entity_id.fetch_add(1, Relaxed)` (single spawn worker) | Relaxed |

### 10.4 Send / Sync / Unpin derivations (UPDATED W5)

| Type | Send | Sync | Unpin | Reason |
|------|------|------|-------|--------|
| `SpawnBatchCommand<B, I>` | YES (auto) | YES (auto) | YES (auto) | bounds require `B: Bundle (Send + Sync + Unpin)` + `I: ... + Send + Sync + Unpin + 'static`. Pinned by `assert_impl_all!` outside `#[cfg(test)]` (I-N5) using `derive(Bundle)` stub (W1). |
| **`SpawnBatchIter<'a, 's, B>`** (W5: dropped `I` param) | NO | NO | YES | `PhantomData<(&'a mut Commands<'s>, B)>`. Type signature no longer leaks the bundle iterator's unnameable type into user API. |
| `BundleColumnCache` | YES | YES | YES | `Box<[OnceLock<T>]>` |
| `BundleColumnRecord` | YES | YES | YES | POD + `&'static [InlandPoolId]` |

**Pin lock-down** (production build, NOT `#[cfg(test)]` — I-N5; W1 stubs):

```text
use static_assertions::assert_impl_all;

assert_impl_all!(
    SpawnBatchCommand<__private_pin_test::PinTestBundle, std::ops::Range<u32>>:
    Send, Sync, Unpin
);
assert_impl_all!(BundleColumnCache: Send, Sync);
assert_impl_all!(BundleColumnRecord: Send, Sync);
```

### 10.5 Parallel contention sensitivity (UPDATED I-N1, W2)

Worst case: 8 workers × `reserve_batch(MAX_BATCH_HINT)` near steady-state ⇒ counter advances to `MAX_ENTITIES_HINT - 1 + 8 × MAX_BATCH_HINT = 63 999 + 65 536 = 129 535`, exceeding pre-sized 72 192.

**v1 mitigation**:
- **Worker side (queued path)** — SBO17b runtime guard in `SpawnBatchCommand::apply` panics with `WorldEntityCapacityExceeded`. Workers cannot pre-check because they race against each other.
- **Dispatcher side (direct path)** — `EcsMaster::spawn_batch` performs a W2 Relaxed pre-load and returns `Err(WorldEntityCapacityExceeded)` without advancing the counter. Sound because `&mut self` closes the worker race window.

**Phase 13**: per-thread reservation pools amortize cap-checking across many small batches and avoid the aggregate-overshoot case entirely.

---

## §11 Integration

### 11.1 Affected modules (UPDATED W1, W2, W5)

| Module | Change |
|--------|--------|
| `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs` | Opt-A1: hoist catch_unwind + case-4 fix |
| `crates/boyko_ecs/src/ecs/core/commands/spawn_batch_command.rs` | NEW; SBO-SEND1; `assert_impl_all!` outside `#[cfg(test)]` (I-N5) over `derive(Bundle)` stub (W1); apply body uses W4 hoisted debug_assert |
| `crates/boyko_ecs/src/ecs/core/commands/spawn_batch_iter.rs` | NEW; `<'a, 's, B>` (W5: no I param) |
| `crates/boyko_ecs/src/ecs/core/commands/spawn_at_command.rs` | Opt-A3: cache; uses new accessors (C-N1) |
| `crates/boyko_ecs/src/ecs/core/commands/mod.rs` | re-exports |
| `crates/boyko_ecs/src/ecs/core/system/params/commands.rs` | Opt-A2: `spawn_batch` returns `EcsResult<SpawnBatchIter<'_, 's, B>>` (W5); rustdoc for SBO8b (I-N2) |
| `crates/boyko_ecs/src/ecs/core/system/params/entity_counter.rs` | Opt-A2: `reserve_batch` |
| `crates/boyko_ecs/src/ecs/core/entity/entity_master.rs` | Opt-A2: `reserve_batch` + `register_batch`; **W2 NEW: `next_id_atomic()` accessor** (pub(crate), `&` not `&mut` — no EM6 surface change) |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` | Opt-A2: `reserve_capacity` → `EcsResult` |
| `crates/boyko_ecs/src/ecs/memory/component_pool.rs` | Opt-A2: `can_reserve`, `len_for_reserve`, `write_at_unchecked_initialized`, `commit_units`, `fill_ticks` (C-N1) |
| `crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs` | Opt-A2: `pools_iter` / `pools_iter_mut` / `pools_len` / `pool_id_for` / `pool_at_unchecked_mut` + batch forwarders (C-N1) |
| `crates/boyko_ecs/src/ecs/core/bundle/bundle.rs` | `Bundle: Unpin` supertrait (C-N3) |
| `crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs` | NEW |
| `crates/boyko_ecs/src/ecs/core/bundle/mod.rs` | re-exports |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | Opt-A2+A3: `bundle_column_cache` (eager), `MAX_BATCH_HINT`, pre-sized vectors, `spawn_batch` with W2 pre-load + W3 Vec return |
| `crates/boyko_ecs/src/ecs/error.rs` | NEW variants: `SpawnBatchExceedsCapacity`, `ArchetypePoolCapacityExceeded`, `WorldEntityCapacityExceeded` (I-N1) |
| `crates/boyko_macros/src/lib.rs` | `derive(Bundle)` codegen: verify `Unpin` (compile error if user has `PhantomPinned`) |
| `crates/bench_bevy_vs_boyko/benches/comparison.rs` | new `spawn_batch_*` benches (2 × 5K chunks) |
| `crates/boyko_ecs/benches/phase12_5_*.rs` | NEW |
| `crates/boyko_ecs/tests/spawn_batch_smoke.rs` | NEW (+ W1 pin-bundle, W2 no-counter-advance, W5 type-signature tests) |
| `crates/boyko_ecs/tests/phase12_5_opt_a3.rs` | NEW |
| `crates/boyko_ecs/tests/miri_phase12_5.rs` | NEW |
| `crates/boyko_ecs/tests/loom_phase12_5.rs` | NEW (cfg-gated) |
| `crates/boyko_ecs/tests/bundle_compile_fail/non_unpin.rs` | NEW (trybuild test for SBO-UNPIN) |
| `docs/PHASE-12.5-SPAWN-OPTIMIZATIONS-PLAN.md` | this file |

### 11.2 Compatibility with prior phases

| Phase | Compat |
|-------|--------|
| Phase 7 | unchanged |
| Phase 8d | extends — Q-A1.1 case 4 correctness fix |
| Phase 8.5 | extends — Bundle now requires Unpin supertrait (C-N3); pin-test bundle uses derive(Bundle) (W1). All existing `derive(Bundle)` outputs are already `Unpin` (no breakage); manual `impl Bundle` blocks are sealed (BundleSealed) so no out-of-tree impls exist |
| Phase 9 | extends — SEND5 preserved via SBO16+SBO17 + SBO17b runtime guard; W2 closes the direct-path counter-leak window |
| Phase 10 | extends — `fill_ticks_batch` bulk variant |
| Phase 11 | extends — `EntityCounter::reserve_batch` reuses atomic; `SpawnAtCommand` benefits from Opt-A3; **W2 adds `EntityMaster::next_id_atomic()` accessor** (`&` only, EM6 surface unchanged) |
| Phase 12 | unaffected |

### 11.3 ABI / API breaking changes (UPDATED W2, W3, W5)

- **Public**:
  - `Bundle: Unpin` supertrait added (C-N3). Backward-compatible — all existing derive outputs are `Unpin`.
  - `Commands::spawn_batch` and `EcsMaster::spawn_batch` accept additional `Unpin` bound on `I::IntoIter`. Standard iterators are all `Unpin`.
  - **W5**: `SpawnBatchIter<'a, 's, B>` (3 type-system parameters: 2 lifetimes + 1 type) instead of `SpawnBatchIter<'a, 's, B, I>` (4). Public type signature simplification. No call site breakage because the bundle iterator type was never user-named.
  - **W3**: `EcsMaster::spawn_batch` returns `Vec<Entity>`. Documented as setup-time alloc; queued path is alloc-free.
- **`SpawnBatchCommand::apply` panic semantics** (I-N1, I-N4): apply may hard-panic on:
  - (a) Aggregate-worker overshoot (`WorldEntityCapacityExceeded`) — SBO17b runtime guard.
  - (b) `Archetype::reserve_capacity` returning Err — logic-bug indicator.
  - Both panics propagate through the `catch_unwind` outer wrap (Opt-A1) → `resume_unwind` → dispatcher boundary.
- **`EcsMaster::spawn_batch` error semantics** (W2): may return `Err(WorldEntityCapacityExceeded)` in addition to `Err(SpawnBatchExceedsCapacity)`. **The counter is not advanced on either Err** (SBO17 strong form, restored).
- **`CommandQueue::apply` behaviour** preserved at user level (per Q-A1.1 cases 1/3). Case 4 is a bug fix.
- **`SpawnAtCommand::apply`** internal refactor (Opt-A3) — no public API change.
- `EcsMaster` size grows by 8 B (one `Box` pointer for `BundleColumnCache`).

### 11.4 Wave-based implementation sequence

- **Wave 1** (Opt-A1): A1-Step 1-7 incl. 4 new tests.
- **Wave 2** (Opt-A3 foundations): A3-Step 1-3 + Step 7 incl. SBO-N test.
- **Wave 3** (Opt-A2 sub-wave a): A2-Step 1-4 incl. C-N1 accessors + `next_id_atomic` (W2 dependency).
- **Wave 4** (Opt-A2 sub-wave b): A2-Step 5-6 + A3-Step 4 incl. `assert_impl_all!` outside `cfg(test)` over `derive(Bundle)` stub (I-N5 + W1) + W2 Relaxed pre-load.
- **Wave 5** (Opt-A2 sub-wave c): A2-Step 7-9 incl. W5 type-signature compile test + W4 debug-overhead bench + I-N1 aggregate-overshoot tests + W2 no-counter-advance test.
- **Wave 6** (Docs + memory): update internal docs.

---

## §12 Test Plan

### 12.1-12.5: as Round 3, plus W1-W5 additions

(See §4.7, §5.11, §6.8 for explicit additions. Round 4 adds: W1 pin-bundle-compiles, W2 direct-path-no-counter-advance, W4 debug-assertion-overhead bench, W5 type-signature-no-iter-leak.)

### 12.6 Debug assertions (Round 4 additions in **bold**)

| Site | Assertion |
|------|-----------|
| `EntityCounter::reserve_batch` | `n ≤ MAX_BATCH_HINT` (SBO17) — Err return, NOT debug_assert |
| `EntityCounter::reserve_batch` | `start + n < usize::MAX / 2` (debug) |
| `EntityMaster::register_batch` | each slot is currently NULL |
| `EntityMaster::register_batch` | `base + n ≤ entities_inland.len()` (SBO16) |
| `Archetype::reserve_capacity` | pool `can_reserve(n)` (EcsResult, NOT debug_assert) |
| `ComponentPool::write_at_unchecked_initialized` | `index < max_components` + bytes.len match |
| `ComponentPool::commit_units` | `start_row == units.len()` |
| `ComponentPoolBundle::pool_at_unchecked_mut` | `pool_idx.0 < pools.len()` |
| `BundleColumnCache::get_resolved` | `B::bundle_type_id().0 < MAX_BUNDLE_TYPES` |
| `BundleColumnCache::resolve_and_cache` | each `B::component_ids()[i]` exists in archetype's pool map |
| `BundleColumnCache::resolve_and_cache` | `pool_ids_owned.is_sorted_by_key(|p| p.0)` (SBO-B2 install-time) |
| **`SpawnBatchCommand::apply` (Step 2.5, W4 hoisted)** | `cache_record.pools_len_at_install ≤ pools_len()` (SBO-N) — **ONCE per batch**, not per row |
| **`SpawnBatchCommand::apply` (Step 2.5, W4 hoisted)** | `pool_ids.is_sorted_by_key(|p| p.0)` (SBO-B2) — **ONCE per batch**, not per row |
| **`SpawnBatchCommand::apply` (Step 2.5)** | `pool_ids.len() == B::component_ids().len()` — once per batch |
| `SpawnBatchCommand::apply` (I-N1) | `end_id ≤ entities_inland.len()` — hard panic with `WorldEntityCapacityExceeded` if exceeded (SBO17b); NOT debug_assert |
| `SpawnBatchCommand::apply` | `iter.len() == self.count as usize` on entry (ExactSizeIterator contract) |
| `SpawnBatchCommand::apply` (per-row, O(1)) | `canonical_idx < pool_ids.len()` inside for_each |
| `SpawnBatchCommand::apply` (per-component, O(1) per call) | `B::component_ids()[canonical_idx] == component_id` — bounded by MAX_BUNDLE_ARITY |
| `SpawnBatchCommand::apply` (per-row) | `canonical_idx == pool_ids.len()` after for_each completes |
| **`EcsMaster::spawn_batch` (W2 pre-check)** | `cur.checked_add(n).is_some()` — overflow guard before subtraction comparison |
| Post-apply | `archetype.current_index == start_row + n`; `entity_ids.len() == current_index` |

---

## §13 Risk Register

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Opt-A1 case 4 fix breaks user-visible behaviour | LOW | M | test documents the new (correct) semantic; latent bug |
| Opt-A1 hoisted catch breaks panic-recovery for some case | LOW | M | C3 4-case enumeration; tests added |
| Opt-A2 `SpawnBatchCommand<B, I>` payload exceeds 256 B (large closure capture) | LOW | L | document size cap; recommend Box closure |
| `assert_impl_all!` rejects a user iterator pattern with `!Sync` or `!Unpin` capture | M | L | Bevy parity; user `.collect()`s first |
| Opt-A3 cache pollution at >1024 distinct bundle types | LOW | M | `BundleTypeRegistry` already panics at MAX_BUNDLE_TYPES |
| Opt-A3 SBO-N detection-only, no prevention (I-N3) | LOW | H | invariant documented + monotonic debug_assert + Phase 13 hook (OQ12) |
| `spawn_batch_10k` bench does not hit ≥1.10× bevy | M | H | perf model predicts ~3× win |
| `reserve_batch` aggregate exceeds cap (8 workers × MAX_BATCH_HINT) | LOW | M | SBO17b runtime guard + W2 direct-path pre-check; Phase 13 per-thread pools mitigate |
| Mid-batch panic safety breaks Miri | M | M | leak-not-double-drop; ManuallyDrop B4; miri_spawn_batch_panic_in_iter test |
| ExactSizeIterator + Send + Sync + Unpin + 'static bound too restrictive | LOW | L | matches Bevy + standard iterators |
| `MAX_ENTITIES_HINT + MAX_BATCH_HINT (72 192)` underestimates real workload | LOW | M | document; raise constants if measured |
| Bundle: Unpin supertrait breaks a downstream consumer (C-N3) | LOW | L | all derived bundles are Unpin by default; manual impls are sealed |
| **W1 pin-test stubs `PinTestComp` + `PinTestBundle` accidentally trigger runtime registration** | LOW | L | `assert_impl_all!` is `const _:` — pure compile-time; runtime registration (`static_info()`/`OnceLock` init) only fires on first spawn, which the doc-hidden stubs never do |
| **W2 Relaxed pre-load races with worker counter advance** | NEGLIGIBLE | L | TOCTOU closed by `&mut self`; documented in §5.5; race-free by Rust borrow checker, not by atomic ordering |
| **W4 hoisted SBO-N debug_assert at Step 2.5 misses an SBO-N violation that develops mid-batch** | NEGLIGIBLE | L | impossible — pools Vec is only mutated under `&mut EcsMaster`, and the apply runs under exclusive `&mut`; no concurrent mutation can occur between Step 2.5 and Step 5 |
| **W5 dropping `I` from `SpawnBatchIter` confuses users debugging type errors** | LOW | L | new signature is simpler (3 params vs 4); rustdoc explicitly explains the bundle iter type lives inside `SpawnBatchCommand`, not `SpawnBatchIter` |

---

## §14 Rejected Alternatives

(Same as Round 2 §14.1-14.7; no Round 4 additions.)

### 14.8 (NEW W1 rejected fallback) — Using `()` as the pin-test `Bundle` placeholder

The user's W1 guidance suggested `assert_impl_all!(SpawnBatchCommand<(), core::iter::Empty<()>>: Send, Sync, Unpin)` as a "spell it out" alternative. **Rejected**: the existing `Bundle` trait (`crates/boyko_ecs/src/ecs/core/bundle/bundle.rs:177`) is sealed via `BundleSealed` and **does not implement `Bundle` for `()`** in the current codebase. Adding `impl Bundle for ()` is a substantive new design decision (does it spawn an entity with no components? how does `for_each_component_bytes` behave? what `BundleTypeId` does it claim?) — well outside W1's "≤30 line touch-up" scope. The chosen `derive(Bundle)` stub is mechanical, zero-risk, and pinned by the macro's own correctness.

### 14.9 (NEW W3 rejected) — Returning `impl Iterator<Item = Entity> + ExactSizeIterator + '_` from `EcsMaster::spawn_batch`

Considered per W3 option (b). **Rejected**: the direct path is dispatcher-only (§1.4) and used predominantly at world setup / fixture construction, where the caller almost always holds entity IDs in scope for later use (`let players = ecs.spawn_batch(...)?;`). Returning an iterator forces caller-side `.collect()` in nearly every realistic call site, undoing the apparent saving while complicating the type signature (`impl Trait + 'a` lifetime variance trips up beginners). `Vec` is the ergonomic match; the alloc is non-hot-path and documented as such.

### 14.10 (NEW W4 rejected) — Removing the per-row `B::component_ids()[canonical_idx] == component_id` check

Considered. **Rejected**: this check is O(1) per component (bounded by `MAX_BUNDLE_ARITY = 8`) and catches real bugs in the `Bundle::for_each_component_bytes` codegen (B2 invariant). Keeping it inside `for_each` is correct; only the O(N) `is_sorted_by_key` was hoisted (W4).

---

## §15 Open Questions

(Round 2 OQ9-OQ11 + Round 3 OQ12; no Round 4 additions.)

### OQ9: Should the chunking pattern be exposed as `Commands::spawn_batch_chunked<I: Iterator>`?

Decision (proposed): no in v1.

### OQ10: Should `MAX_BATCH_HINT` be configurable per-world via a builder?

Decision (proposed): no in v1.

### OQ11: Should the Q-A1.1 case-4 success-path fix be feature-gated for back-compat?

Decision (proposed): no.

### OQ12 (I-N3): How will Phase 13 archetype destruction invalidate the `BundleColumnCache`?

Decision (proposed): defer to Phase 13 design phase. v1 ships with detection-only `pools_len_at_install` guard.

---

## §16 Plan-Readiness Checklist (Round 4 update)

### Plan structure
- [x] Goal stated in terms of perf + functionality (§1.1)
- [x] Target metrics concrete (§1.2)
- [x] Every decision justified via perf/cache/parallelism
- [x] Alternatives rejected with reasoning (§14, including W1 `()` fallback, W3 iterator return, W4 per-row check removal)
- [x] Trade-offs honestly listed
- [x] Round 3 critic findings explicitly resolved (§0 changelog)

### Data structures
- [x] Field types + access role comments
- [x] `#[repr(C)]` on `SpawnBatchCommand` + `BundleColumnRecord`
- [x] Hot/cold split: cache lookup hot, resolve_and_cache cold
- [x] Sizes known + justified (§8)
- [x] False sharing analysis
- [x] **W5: SpawnBatchIter layout simplified to `<'a, 's, B>`** — type signature no longer leaks bundle iterator type

### API
- [x] Public API minimal
- [x] No leaked internal types (W5: no `I::IntoIter` in `SpawnBatchIter` user-visible signature)
- [x] Lifetimes explicit
- [x] No `dyn Trait` in hot path
- [x] Generics where needed
- [x] No hand-written `unsafe impl Send/Sync` (C2)
- [x] `EcsResult` propagation for capacity errors (C1, I5)
- [x] `Bundle: Unpin` supertrait + `I: Unpin` bounds (C-N3)
- [x] All new accessors named and signed (C-N1)
- [x] `EcsMaster::spawn_batch` routes through `reserve_batch` (C-N2)
- [x] **W2: `EcsMaster::spawn_batch` Relaxed pre-load before fetch_add** — counter not advanced on Err
- [x] **W3: `EcsMaster::spawn_batch` returns `Vec<Entity>` with explicit setup-only rustdoc**
- [x] **W5: `SpawnBatchIter<'a, 's, B>` — dropped dead `I` parameter**

### Multithreading
- [x] Model explicit
- [x] Atomic ordering specified (W2: Relaxed pre-load added to §10.3 table)
- [x] Sync points justified
- [x] Partitioning described
- [x] Send/Sync consistent
- [x] Contention model
- [x] `MAX_BATCH_HINT` cap + worst-case interleaving
- [x] Aggregate-worker runtime guard (SBO17b — I-N1) + W2 direct-path eager pre-check
- [x] **W2 TOCTOU non-issue documented** — `&mut self` closes the race window at the type system

### Correctness
- [x] Edge cases enumerated
- [x] Generation check described
- [x] Drop order discussed (incl. SBO8b — I-N2)
- [x] Unsafe invariants stated
- [x] Canonicalization preserved
- [x] SBO-N pool stability invariant (C4) — detection-only per I-N3
- [x] SBO-B2 canonical-order pin (I1)
- [x] Q-A1.1 case-4 success-path fix (C3)
- [x] `assert_impl_all!` Send+Sync pin (C2)
- [x] SBO-UNPIN supertrait (C-N3)
- [x] `assert_impl_all!` outside `#[cfg(test)]` (I-N5)
- [x] SBO8b `SpawnBatchIter` drop semantics (I-N2)
- [x] SBO17b aggregate-overshoot runtime guard (I-N1)
- [x] `SpawnBatchCommand::apply` uses `.expect`, not `?` (I-N4)
- [x] **W1: `__private_pin_test::PinTestBundle` is `derive(Bundle)` over `derive(Component) struct PinTestComp(u8)` — concrete code, not `/* trivial */`**
- [x] **W2: SBO17 restored to strong form** — "counter is not advanced on Err"
- [x] **W4: SBO-B2 / SBO-N hoisted debug_assert at Step 2.5** — once per batch, not per row

### Integration
- [x] Affected modules listed
- [x] Existing API changes noted
- [x] Phase 8.5 / 9 / 10 / 11 / 12 compatibility verified
- [x] Implementation plan stepwise
- [x] **W2: `EntityMaster::next_id_atomic()` accessor added** — pub(crate), `&` only, EM6 surface unchanged

### Validation
- [x] Unit tests specified
- [x] Integration tests specified
- [x] Property tests (existing Phase 11 cover)
- [x] Benchmarks specified with targets
- [x] debug_assert! sites listed (W4: hoisted SBO-B2 site moved to Step 2.5)
- [x] Miri tests specified
- [x] Loom tests specified
- [x] All Round 3 critic findings regression-tested (§5.11)
- [x] **W1/W2/W4/W5 regression tests enumerated (§5.11)**: pin-bundle-compiles, direct-path-no-counter-advance, debug-overhead bench, type-signature-no-iter-leak

---

**End of Phase 12.5 Track A Plan, Round 4.**

Three optimisations:
- **Opt-A1**: hoist `catch_unwind`; ~7 ns/entity saving; Q-A1.1 case 4 correctness fix.
- **Opt-A2**: `Commands::spawn_batch<B, I>(iter) -> EcsResult<SpawnBatchIter<'_, 's, B>>` with cap (`n ≤ MAX_BATCH_HINT`); ~80-120 ns/entity batch saving; Round 4 adds concrete `derive(Bundle)` pin stub (W1), Relaxed pre-load before fetch_add (W2), documented Vec return (W3), hoisted O(N) debug_asserts (W4), dropped dead `I` param from `SpawnBatchIter` (W5).
- **Opt-A3**: per-world `BundleColumnCache` with `pools_len_at_install` (SBO-N), canonical-sorted `pool_ids` (SBO-B2); ~10-20 ns/entity all paths; SBO-N tightened to detection-only with Phase 13 hook (I-N3).

Headline targets:
- `comparison.rs` g4 single-spawn: 2.48 ms → ≤ 1.15 ms.
- `spawn_batch_5k_1comp` (× 2 for 10K workload): ≤ 400 µs combined.

---

## Brief Summary (for the orchestrator)

### Round 4 NEW IMPORTANT issues resolved

- **W1**: replaced `/* trivial */` placeholder with a concrete `#[derive(Component)] struct PinTestComp(u8);` + `#[derive(Bundle)] struct PinTestBundle { c: PinTestComp }` inside the `__private_pin_test` doc-hidden module. The user-suggested `()` fallback was rejected and documented in §14.8 — `impl Bundle for ()` does not exist in the codebase and adding it is a new substantive design, out of scope. The derive macro emits correct `Send + Sync + Unpin + 'static` trait impls by construction; `assert_impl_all!` is a `const _:` item that fires at compile time without invoking any runtime registration path (no `static_info()` call, no `OnceLock` init).
- **W2**: §5.5 augmented with a Relaxed `load` of `next_entity_id` BEFORE the `fetch_add` inside `EntityMaster::reserve_batch`. On overshoot, returns `Err(WorldEntityCapacityExceeded)` without advancing the counter. SBO17 restored to its strong form: "the counter is not advanced when the call returns Err." TOCTOU non-issue documented: `&mut self` precludes worker access (Rust borrow checker closes the window). New `EntityMaster::next_id_atomic()` accessor (pub(crate), `&` only, EM6 surface unchanged). New test `spawn_batch_direct_aggregate_overshoot_returns_err_no_counter_advance` verifies counter is unchanged after Err.
- **W3**: chose option (a) — `EcsMaster::spawn_batch` keeps `Vec<Entity>` return type. §5.5 rustdoc explicitly documents this as "setup-time heap alloc, NOT hot-path; queued path is alloc-free." Rejected alternative (option b — iterator return) documented in §14.9. §1.4 already lists direct path as out-of-scope for hot-path optimisation; W3 just clarifies the rationale.
- **W4**: hoisted `pool_ids.is_sorted_by_key` (O(N), N ≤ MAX_BUNDLE_ARITY = 8) and `cache_record.pools_len_at_install ≤ pools_len()` (O(1)) OUT of the per-row `for i in 0..n` loop. Both now run once at Step 2.5, between cache resolution (Step 2) and SBO17b guard (Step 3). For an 8K-batch × 8-component workload this drops 65 k debug-build cmp ops to 8. The per-row `B::component_ids()[canonical_idx] == component_id` check stays inside `for_each_component_bytes` (O(1) per component, well-bounded). New debug-build bench `bench_spawn_batch_8k_debug_assert_overhead` validates the saving.
- **W5**: dropped `I` from `SpawnBatchIter`. New signature `SpawnBatchIter<'a, 's, B>` with phantom `PhantomData<(&'a mut Commands<'s>, B)>`. `Commands::spawn_batch` return type is `EcsResult<SpawnBatchIter<'_, 's, B>>` — no unnameable `I::IntoIter` in user-visible signature. Test `spawn_batch_iter_type_signature_no_bundle_iter_leak` is a compile-only assertion. §10.4 Send/Sync/Unpin table updated. §8.4 added (layout unchanged — phantom is ZST; only the type-system surface simplifies).

### Decisions made in Round 4

- **W1 option (b) — `derive(Bundle)` stub** chosen over user's `()` suggestion: the latter requires a new `impl Bundle for ()` not in the codebase, outside the "≤30 line touch-up" scope. The chosen route uses production code paths (the derive macro) with zero risk.
- **W3 option (a) — keep `Vec<Entity>`**: simpler API, ergonomic match for setup callers, alloc is non-hot-path.
- **W2 pre-load before fetch_add**: race window closed by `&mut self`, not by atomic ordering. Documented as a TOCTOU non-issue per the brief's guidance.
- **W4 per-row debug_asserts**: O(1) per-component check stays (catches B2 codegen bugs); O(N) `is_sorted_by_key` hoisted once per batch.
- **W5 retained `B` parameter**: ergonomic discoverability (`SpawnBatchIter<'_, '_, EnemyBundle>` reads naturally) and type-system locking of the spawn-type; only `I` (which had no semantic role in the iter) was dropped.

### File paths (absolute, for orchestrator)

- Save plan to: `D:\claude\BoykoEngine\docs\PHASE-12.5-SPAWN-OPTIMIZATIONS-PLAN.md` (replace Round 3)
- Reference: `D:\claude\BoykoEngine\docs\PHASE-12.5-SURPASS-BEVY-PLAN.md`, `D:\claude\BoykoEngine\docs\PHASE-12.5-PROFILE-SPAWN.md`, `D:\claude\BoykoEngine\docs\PHASE-12.5-CRITIC-A-ROUND-1.md`, `D:\claude\BoykoEngine\docs\PHASE-12.5-CRITIC-A-ROUND-2.md`
- Round 3 reference code paths unchanged: `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\command_queue.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\spawn_at_command.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\entity\entity_master.rs` (line 147 confirms `next_id_atomic()` exists as `pub(crate)`), `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_pool_bundle.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\bundle.rs` (line 177 = supertrait line), `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\error.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\tests\derive_bundle_smoke.rs` (W1 confirms `derive(Bundle)` stub pattern is the canonical way to construct a Bundle for assertions)

---

## Summary of W1-W5 Resolutions

- **W1** (private pin-test `/* trivial */`): use `#[derive(Component)] struct PinTestComp(u8);` + `#[derive(Bundle)] struct PinTestBundle { c: PinTestComp }` inside `__private_pin_test` doc-hidden module. User-suggested `()` rejected because `impl Bundle for ()` doesn't exist in codebase. `assert_impl_all!` runs at compile time only, never triggers registration.
- **W2** (direct-path counter leak on Err): add Relaxed `load` of `next_entity_id` BEFORE `fetch_add`; return `Err(WorldEntityCapacityExceeded)` without advancing counter. TOCTOU closed by `&mut self`. SBO17 wording restored to strong form. New `EntityMaster::next_id_atomic()` accessor exposes the atomic read-only.
- **W3** (`Vec<Entity>` heap alloc asymmetric): keep `Vec<Entity>` and document in rustdoc as setup-time alloc; the queued path remains alloc-free. Documented in §14.9 why an iterator return was rejected.
- **W4** (per-row O(N) debug_assert × 65k in 8K batch): hoist `pool_ids.is_sorted_by_key` and `pools_len_at_install ≤ pools_len()` to Step 2.5 (once per batch). The per-row O(1) check `B::component_ids()[canonical_idx] == component_id` stays inside `for_each_component_bytes`.
- **W5** (`SpawnBatchIter` dead `I` param): drop `I` from `SpawnBatchIter<'a, 's, B>`. PhantomData becomes `(&'a mut Commands<'s>, B)`. The bundle iterator type no longer leaks into user-visible signatures.

Sources:
- [Bevy SpawnBatchIter source (0.18.1)](https://docs.rs/bevy_ecs/0.18.1/src/bevy_ecs/world/spawn_batch.rs.html)
- [Bevy spawn_batch docs](https://docs.rs/bevy_ecs/latest/bevy_ecs/system/struct.Commands.html#method.spawn_batch)
- [static_assertions::assert_impl_all macro docs](https://docs.rs/static_assertions/latest/static_assertions/macro.assert_impl_all.html)
- [Rust Unpin trait docs](https://doc.rust-lang.org/std/marker/trait.Unpin.html)
