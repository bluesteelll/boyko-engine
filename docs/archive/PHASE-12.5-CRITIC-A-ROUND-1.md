> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 12.5 Track A — Critic Round 1

Critique of `docs/PHASE-12.5-SPAWN-OPTIMIZATIONS-PLAN.md` (Round 1).
Verdict: **NEEDS-FIX**.

## CRITICAL (block APPROVED)

### C1. `SpawnBatchCommand::apply` math collides with `Vec::with_capacity` upper bound — silent overflow on batches > `MAX_ENTITIES_HINT`

Plan §5.4 Step 3 resizes `entities_inland` / `sparse_to_active` with
`world.entity_master.entities_inland.resize(end_id, EntityInland::NULL)`.
Pre-Phase 9 these vectors are pre-allocated to `MAX_ENTITIES_HINT = 64_000`
(`ecs_master.rs:38` + `SEND5` invariant). The Phase 9 send/sync invariant
SEND5 (entity_master.rs:432-449) is **load-bearing on these vectors NOT
reallocating during steady state**: worker `&self` paths (`get_component_raw`,
`is_entity_valid`) do unsynchronised reads from `entities_inland` while in
the same frame batched apply may run on the dispatcher. SCH7 (apply window)
protects this for normal commands. But the plan never says spawn_batch must
execute inside the apply window — it does (per §5.9), but the §5.4 `resize()`
past `MAX_ENTITIES_HINT` reallocates `entities_inland`'s heap buffer
mid-apply. Any future change that lets workers read concurrently with the
apply window encounters a dangling base pointer.

A single `spawn_batch(70_000)` violates the SEND5 working assumption.
At 4 × `fetch_add(20_000)` the counter has crossed 64 000 and
`entities_inland.resize` performs a real allocation that breaks the
"no realloc in steady state" guarantee.

**Required fix**: the plan must either
(a) document an explicit per-batch upper bound `n ≤ MAX_ENTITIES_HINT - current_count`
    with a debug_assert and a graceful error path for the runtime case,
OR
(b) acknowledge SEND5 violation under large batches and design a mitigation
    (defer the resize to a pre-frame pass; switch `entities_inland` to a slab;
    pre-extend in `EcsMaster::new` based on `MAX_BATCH_HINT`).

Silently re-using `Vec::resize` in a path that explicitly violates a prior
Send+Sync invariant is not acceptable.

### C2. `SpawnBatchCommand<B, I>` Send claim is unsound — `B: Bundle` only requires `Send + Sync + 'static`, but the bundles are *held alive inside `I`* (the iterator state), and `I: Send` is the documented bound only when the iterator's items are `Send`. The plan's explicit `unsafe impl Send` (§5.2 line 595) elides the `I: Send` constraint.

Plan §5.2 line 595:
```text
unsafe impl<B: Bundle, I: ExactSizeIterator<Item = B> + Send + 'static> Send for SpawnBatchCommand<B, I> {}
```

The plan inconsistently states the Send bounds — sometimes `I: Send + 'static`,
sometimes just `Send`, sometimes via `where I::IntoIter: ExactSizeIterator + Send + 'static`.
There is no proof that this hand-written `unsafe impl Send` is consistent
with the user-facing trait bounds (e.g. what happens if a user adapter
implements `Send` for the wrapper but not for the inner iterator state —
a common pattern with `Map<I, F>` where `F: !Send`?). The auto-derived
`Send` would be the correct mechanism; the explicit `unsafe impl` adds
nothing but the risk of bound drift on future refactors.

Worse, §10.4 row "`SpawnBatchCommand<B, I>` | YES (where I: Send) | NO"
claims `Sync = NO` — but `SpawnAtCommand<B>` (the prior art) is
`Send + Sync` for any `B: Bundle` (verified at `spawn_at_command.rs:60-63`).
The plan does not justify why batch commands cannot be Sync when single-spawn
commands are.

**Required fix**: remove the hand-written `unsafe impl Send`; rely on
auto-derivation. Justify `Sync = NO` against the SpawnAtCommand precedent,
or document why `Sync` would be unsafe for the inner iterator.

### C3. Q-A1.1 panic-semantics analysis is incomplete — it ignores what happens when a panicker enqueues new commands before panicking.

Plan §3.1 / Q-A1.1 declares behaviour preserved "bit-for-bit" between
current per-command `catch_unwind` and the hoisted version. The
before/after table claims "User-observable apply order | strict FIFO
with skipped panicker | Same."

But the existing `RawCommandQueue::apply_or_drop_queued` body
(`command_queue.rs:275-281`) executes `*self.cursor.as_mut() = stop`
*before the walk* explicitly to handle the case where a command-during-apply
pushes a NEW command into the same queue — the new bytes are placed past
`stop` and **deliberately deferred to the next apply**. The Err branch
(line 354) `let current_stop = bytes.len()` then captures the EXTENDED
bytes including any commands the panicker pushed before it died.

In the proposed hoisted layout (§4.2), the success path `set_len(start)`
may wipe commands that command-during-apply pushed. The current code has
the same shape — but the plan's Q-A1.1 "bit-for-bit preserved" claim does
not enumerate the 4 cases:
1. Panicker pushes & panics
2. Panicker pushes & survives
3. Non-panicker pushes & survives
4. Non-panicker pushes & panicker is later

**Required fix**: §3.1 must explicitly address each case with before/after
behaviour. The plan currently says "bit-for-bit preserved" which is unverified.

### C4. Opt-A3 `BundleColumnRecord::pool_ids` defensive `archetype_id` check is structurally insufficient for the documented SBO12 invariant.

Plan §6.2 defines `BundleColumnRecord { archetype_id, pool_ids: &'static [InlandPoolId] }`
and §6.7 documents SBO12 "cache slot is valid for the world's lifetime;
never invalidated."

The structural problem: `pool_ids: &'static [InlandPoolId]` is leaked from
a `Vec<InlandPoolId>` built off `archetype.component_pools.sparse_indexes.get(...)`.
The leaked slice's POINTER is `'static`. But the **semantic validity** of
each `InlandPoolId` is conditioned on the `archetype.component_pools.pools`
Vec never resizing (because `InlandPoolId` is an index into that Vec, not
a stable address). If a future code path calls `add_pool` *after* the cache
slot was populated, `pools.push(...)` may reallocate, but the indices remain
valid (`InlandPoolId.0` still points to the right slot). So the plan's
invariant holds — but only by luck.

**Required fix**: add explicit invariant to §2.1:

> **Invariant SBO-N**: once an archetype is registered for a given
> `BundleTypeId`, that archetype's `ComponentPoolBundle::pools` Vec MUST NOT
> have entries removed or reordered (pushes are tolerated because they
> preserve existing indices). This is the load-bearing invariant for
> `BundleColumnCache::pool_ids` validity across the world's lifetime.

Plus a `debug_assert` site that verifies an archetype's pool count is
monotonically non-decreasing.

## IMPORTANT (should be fixed but not blocking)

### I1. Plan §5.4 Step 4 (apply loop) violates Bundle::for_each_component_bytes invariant B2 silently in release.

Plan walks the bundle via `for_each_component_bytes`, indexes into
`pool_ids[canonical_idx]`. B2 (`bundle.rs:155`) says callbacks fire in
canonical order. In debug, the assert fires on misorder; in release, a
misordered emit silently writes the wrong component bytes to the wrong
pool — a **memory corruption** silent in release.

**Required fix**: cite B2 explicitly in the `SpawnBatchCommand::apply`
SAFETY block. Add a `debug_assert!(pool_ids.is_sorted_by_key(|i| i.0))`
at cache-install time.

### I2. `EcsMaster::spawn_batch` direct (non-Commands) path concurrency contract is unstated.

Required: state explicitly that `EcsMaster::spawn_batch` is dispatcher-only,
runs outside the apply window of any worker frame, and the `&mut self`
borrow precludes any concurrent worker access by Rust's borrow checker.

### I3. Risk register fails to enumerate the bench fixture risk.

The new `BundleColumnCache` allocation makes `iter_with_setup`-pattern
benches worse. The plan must address whether the `BundleColumnCache`
allocation is lazy (allocated on first use) or eager (allocated in
`EcsMaster::new`).

### I4. Plan §4.2 (`apply_or_drop_queued_no_catch`) loses one of the two existing Drop-path call sites.

The current `CommandQueue::Drop` has TWO `apply_or_drop_queued(None)`
calls (one for `bytes`, one for `panic_recovery`). The plan collapses
into a single `catch_unwind`-wrapped call. The plan must enumerate the
behavioural change and add a regression test
`command_queue_drop_panic_in_command_drop_skips_rest`.

### I5. `Archetype::reserve_capacity` panic semantics conflict with `EcsResult` return.

§5.6 declares `reserve_capacity(&mut self, n: usize) -> EcsResult<()>`.
But §5.4 calls it as `.expect("...")`, and §3 SBO4 says it "panics with a
clear diagnostic." Contradictory.

**Required fix**: pick one — `EcsResult` (and remove SBO4 panic language)
or `panic!` (and change the return to `()`). Recommendation: `EcsResult`,
propagated from `SpawnBatchCommand::apply` as a recoverable error path.

### I6. Memory model for `EntityCounter::reserve_batch` says nothing about per-thread state — but reserve_batch shares an atomic with reserve_entity.

Worst-case interleaving: Worker A `reserve_batch(1_000_000)` → counter
jumps to 1_000_005. Worker B `reserve_entity()` → ID 1_000_005.
Worker A's bundle iterator panics on row 500 000 → 999 500 IDs leak.
Worker B's spawn triggers `entities_inland.resize(1_000_006, NULL)` — a
16 MB heap reallocation, and a Phase 9 SEND5 invariant violation if any
worker concurrently reads `entities_inland`.

**Required fix**: link to C1's resolution. Either cap `n` in `reserve_batch`
to prevent runaway counter advance, or pre-allocate `entities_inland`
sized for the worst-case batch.

## OPTIONAL

### O1. §1.2 `bench_commands_apply_50_noops` math doesn't reconcile with Phase 8d baseline.

If existing baseline is 10 µs / 5000 cmds = 2 ns/cmd, the proposed 5 µs
target / 5000 = 1 ns/cmd is a 2× speedup — but i-cache analysis only
justifies ~7 ns/cmd savings.

### O2. §5.6 `ComponentPool::commit_units` could use `units.extend((0..count).map(...))` for tighter codegen.

### O3. Plan does not benchmark `commands.spawn(bundle)` after Opt-A3 end-to-end.

### O4. Plan §3.2 Q-A2.3 (iterator stored inline) — what if `size_of::<I>() == 0`?

## VERDICT

**NEEDS-FIX**

## RATIONALE

The plan is well-structured, deeply researched, and the three optimisations
are individually defensible. However four critical issues block approval.
None of these are fatal to the design direction, but each is a load-bearing
correctness contract that the next critic round must see addressed
explicitly. The remaining I1-I6 are important but mostly clarifications.

## Relevant file paths

- `D:\claude\BoykoEngine\docs\PHASE-12.5-SPAWN-OPTIMIZATIONS-PLAN.md` (plan under review)
- `D:\claude\BoykoEngine\docs\PHASE-12.5-PROFILE-SPAWN.md` (empirical baseline)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\command_queue.rs` (Opt-A1; lines 264-400 apply loop; 517-543 Drop)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\commands\spawn_at_command.rs` (Opt-A3; lines 79-171)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\entity\entity_master.rs` (lines 432-450 SEND5 — C1, I6)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` (lines 38, 386-452)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\bundle.rs` (lines 137-269 — C4, I1)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_pool_bundle.rs` (lines 128-185 — SparseMap lookups Opt-A3 eliminates)
- `D:\claude\BoykoEngine\crates\boyko_ecs\tests\command_queue_panic_recovery.rs` (lines 196-281 — Q-A1.1 semantics)
