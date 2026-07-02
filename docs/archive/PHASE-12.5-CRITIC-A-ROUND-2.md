> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 12.5 Track A — Critic Round 2

Critique of `docs/PHASE-12.5-SPAWN-OPTIMIZATIONS-PLAN.md` (Round 2).
Verdict: **NEEDS-FIX**.

## Round 1 follow-up verification

| Finding | Status | Notes |
|---------|--------|-------|
| C1 — SEND5 violation via resize | ✅ ADDRESSED | `MAX_BATCH_HINT = 8_192` cap + pre-extend to `MAX_ENTITIES_HINT + MAX_BATCH_HINT`. SBO16/SBO17 enumerated. Aggregate-worker edge case acknowledged in §10.5 — see I-N1. |
| C2 — Send/Sync inconsistency | ✅ ADDRESSED | Hand-written `unsafe impl Send` dropped; auto-derive with `B: Bundle + Send + Sync`, `I: ExactSizeIterator + Send + Sync + 'static`; pinned by `static_assertions::assert_impl_all!`. |
| C3 — Q-A1.1 4-case enumeration | ✅ ADDRESSED | §3.1 4-case table; case 4 correctly identified as latent bug in current code. `ptr::copy` compaction approach sound. |
| C4 — SBO-N invariant | ✅ ADDRESSED | SBO-N spelled out: pools Vec push-only; `pools_len_at_install: u32` field + warm-path debug-assert + Phase 13 hook. (See I-N3 caveat.) |
| I1 — B2 invariant | ✅ ADDRESSED | SBO-B2; `debug_assert!(pool_ids.is_sorted_by_key(...))` at install + warm path. |
| I2 — spawn_batch contract | ✅ ADDRESSED | Marked dispatcher-only in §1.5, §5.5, §10.2. |
| I3 — BundleColumnCache alloc | ✅ ADDRESSED | Eager at `EcsMaster::new`; rationale documented. |
| I4 — Drop path enumeration | ✅ ADDRESSED | §4.2 Drop body uses two separate `catch_unwind` walks. Test added. |
| I5 — reserve_capacity panic vs Result | ✅ ADDRESSED | Returns `EcsResult<()>`; SBO4 rewritten; `EcsError::ArchetypePoolCapacityExceeded` added. |
| I6 — EntityCounter consistency | ✅ ADDRESSED | Linked to C1. Aggregate-worker overshoot honestly flagged — see I-N1. |

All Round 1 findings resolved. Several NEW issues arose from Round 2 changes.

## NEW CRITICAL findings

### C-N1. §5.6 / §5.4 reference fields and methods that don't exist as pub(crate)

- `ComponentPool::units: Vec<Unit>` is **private** (`component_pool.rs:45`).
- `ComponentPool::max_components: usize` is **private** (`component_pool.rs:42`).
- `ComponentPoolBundle::pools: Vec<ComponentPool>` is **private** (`component_pool_bundle.rs:13`).
- The plan calls `archetype.component_pools.pools.len()` directly (§5.4, §6.7).
- The plan calls `self.component_pools.pools_mut()` — **method does not exist**.

**Plan cannot compile as written.** §5.6 / §6.7 must enumerate the new
accessors and pin visibility. Two natural shapes:
1. `pub(crate) fn ComponentPool::can_reserve(&self, n: usize) -> bool` +
   `pub(crate) fn ComponentPool::len_for_reserve(&self) -> (usize, usize)`,
   with `Archetype::reserve_capacity` calling through `ComponentPoolBundle::pools_iter()`.
2. Promote fields to `pub(crate)` (consistent with `added_ticks` / `changed_ticks`
   at `component_pool.rs:80, 87`), document the access contract.

### C-N2. §5.5 pokes `EntityMaster::next_entity_id` directly — EM6 violation

`EntityMaster::next_entity_id: AtomicUsize` is **private** (`entity_master.rs:38`).
Exposed only via `pub(crate) fn next_id_atomic(&self) -> &AtomicUsize` (line 147)
for `EntityCounter` projection. Adding `self.entity_master.next_entity_id.fetch_add(...)`
in `EcsMaster::spawn_batch` violates EM6 (worker-side discipline).

**Fix**: §5.5 must route through `self.entity_master.reserve_batch(n)?` (the
dispatcher-side method already defined in §5.7).

### C-N3. SpawnBatchCommand<B, I>::apply byte-copies `I` via `write_unaligned`+`read_unaligned` — unsound for non-Unpin iterators

`SpawnBatchCommand<B, I>` is pushed via `CommandQueue::push` (`command_queue.rs:115-150`),
which `write_unaligned`s command bytes; `consume_and_drop_glue` later `read_unaligned`s
back. Both are bitwise memcpy.

For iterators with self-references (e.g. owned `Box<[B]>` + `*const B` pointing
into it), the relocation invalidates self-pointers — **silent memory-safety UB in release**.

The plan's bound `I: ExactSizeIterator + Send + Sync + 'static` does **NOT**
require `Unpin`. Standard iterators (Range, Map, Take) ARE Unpin trivially,
but the bound does not enforce it.

**Fix**: add `I: Unpin` to the public bound and `assert_impl_all!` it.
Also assess whether `SpawnAtCommand<B>` has been carrying this risk since
Phase 8.5 (`Bundle: 'static + Send + Sync` but no `Unpin`).

## NEW IMPORTANT findings

### I-N1. Aggregate-worker overflow of MAX_BATCH_HINT × N_workers — silent SEND5 violation under realistic workload

8 workers each calling `reserve_batch(8_192)` near counter `MAX_ENTITIES_HINT - 1` pushes counter to ~129 535, exceeding the pre-sized 72 192. The mitigation says "implicitly limited by user pattern" — but there is **no runtime guard**.

**Fix**: Add runtime check at start of `SpawnBatchCommand::apply`: if `end_id > entities_inland.len()`, return `Err(EcsError::WorldEntityCapacityExceeded)`. Propagate as hard panic at apply boundary. Failure mode becomes observable, not silent.

### I-N2. SpawnBatchIter drop semantics need explicit contract

Dropping `SpawnBatchIter` does NOT cancel the spawn (`SpawnBatchCommand` is already enqueued). Document in rustdoc + add SBO8b: "dropping `SpawnBatchIter` without consuming has no semantic effect — the underlying `SpawnBatchCommand` runs in full on the next apply."

### I-N3. SBO-N "Phase 13 archetype-destruction MUST invalidate cache before pool reordering" — aspirational, not designed

The cache is per-world, not per-archetype, and slot is keyed by `BundleTypeId.0` — no archetype→slots reverse index exists. Phase 13 invalidation strategy is undecided.

**Fix**: Tighten SBO-N wording to "Phase 13 must devise an invalidation mechanism; the current debug_assert is detection-only, not prevention." OR add reverse-index now.

### I-N4. §5.4 Step 4 uses `?` operator inside `Command::apply` body, but `apply` returns `()`

`reserve_capacity(n)?` doesn't compile inside a function that returns `()`.

**Fix**: Use `.expect("apply contract: capacity check should have been done by SpawnBatch::new")` — overrun at apply implies a logic bug because the cap-check at enqueue is authoritative.

### I-N5. `assert_impl_all!` is in `#[cfg(test)]` — does not pin bound at production build

**Fix**: Move outside `#[cfg(test)]`. `static_assertions` emits zero-cost `const _:` items.

## NEW OPTIONAL

- O-N1. §5.6 `ComponentPool::fill_ticks` could use `slice::fill` for SIMD memset.
- O-N2. §8.3 BundleColumnRecord layout already optimal at 32 B.
- O-N3. §3.2 Q-A2.3 `_pad: u32` waste for 4-aligned iterators — accept.

## VERDICT

**NEEDS-FIX** — return to architect.

## RATIONALE

All Round 1 findings resolved soundly. However three new criticals block:
(C-N1) plan references private fields without spelling new accessors → won't compile;
(C-N2) `EcsMaster::spawn_batch` violates EM6 by poking private atomic — trivial fix;
(C-N3) `I` byte-copy through queue is unsound for non-Unpin iterators.

Five new importants are mostly clarifications and edge-case hardening.
Round 3 should be tight (5-20 lines each).
