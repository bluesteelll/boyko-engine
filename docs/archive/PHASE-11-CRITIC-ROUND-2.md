> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Architecture review: Phase 11 EntityCommands chaining + despawn (Round 2)

## Verdict
[X] CHANGES REQUESTED — two new criticals introduced by Round 2 revisions, plus three warnings.

## Round 1 finding status

- **C1 (two-lifetime EntityCommands)** ✅ resolved. The §5.7 sketch correctly uses `EntityCommands<'a, 's>` with `&'a mut Commands<'s>`. The implicit `'s: 'a` outlives bound comes from reference well-formedness, and `reborrow` returns `EntityCommands<'_, 's>` which correctly shortens `'a` while preserving `'s`. Use cases 1-3 will compile.
- **C2 (Commands raw pointer)** ✅ resolved structurally — the raw-pointer + PhantomData scheme is sound under the documented SystemParam contract. But see new C-N1 below for an aliasing concern in the contract wording.
- **C3 (apply_replace_in_place)** ✅ resolved structurally — single linear sequence with proper SAFETY block. But see C-N2 below for a missing API.
- **C4 (four-case proof)** ⚠️ mostly resolved. The proof is solid for cases 1-3. Case 4 (stale fabrication) is also correctly argued — `is_entity_valid` catches the gen mismatch, and `reserve_entity` cannot collide with the stale ID because the counter is monotonic and ≥ the stale ID. One residual gap — see W-N3.
- **C5 (forget_entity → move_out_entity)** ⚠️ mostly resolved. The contract covers byte storage AND both tick rows. But the code sketch in §7.2 uses a non-existent API — see C-N2.

## Remarks

### 🔴 Critical

#### C-N1. `Commands::entity_master_ptr` SAFETY contract permits coexisting `&EntityMaster` from multiple threads — needs explicit aliasing rule

**Where**: §5.6 (Commands<'s> get_param + entity_master accessor)
**Problem**: `entity_master()` returns `&EntityMaster` (shared reference) from a raw pointer that lives on N parallel worker threads simultaneously. Multiple workers can each hold a `&EntityMaster` at the same time during enqueue (each calling `reserve_entity(&self)`). This is fine ONLY because `reserve_entity` touches only the atomic counter — but the SAFETY block (§5.6 lines 588-593) hand-waves with "atomic-counter access is data-race-free" without formally stating: **no other field of EntityMaster may be read or written via this shared reference path on workers.** A future caller adding `Commands::reserve_n` that reads `entities_inland.len()` would silently violate this.
**Why critical**: the invariant ("only `next_entity_id` field is touchable via this shared-borrow path") is encoded only as prose. The borrow checker cannot enforce it. Without an explicit invariant ID + a `#[doc(hidden)]` wrapper that exposes only the atomic counter, the next phase will break this.
**What is needed**: introduce a dedicated invariant (e.g. `EM6`) restricting the field set accessible through `Commands::entity_master()`. Either (a) expose only an atomic-counter projection (a wrapper type that has only `reserve_entity`), not the full `&EntityMaster`, or (b) document an explicit list of permitted methods and add a debug-build runtime guard.

#### C-N2. §7.2 `swap_remove_index_no_drop` uses fictitious `byte_ptr().add(idx * stride)` API — incompatible with the chunked `ComponentPool` storage

**Where**: §7.2 (lines 1035-1058) and §7.4 (lines 1108-1113, retained-byte slice construction)
**Problem**: the planned code reads `self.byte_ptr().add(last * stride)` as if the pool were a flat `Vec<u8>`. The real `ComponentPool` (verified at `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs:48, 51, 229`) is **chunked**: `chunks: Vec<Chunk>`, `components_per_chunk: usize`, plus `units: Vec<Unit>` where each Unit holds an absolute pointer (possibly into different chunks). The existing `swap_remove` (line 339) iterates via `self.units[index].ptr()` and `self.units[last_index].ptr()` — there is no contiguous stride addressing. Similarly, `added_ticks_ptr_mut()` does not exist (only `added_ticks_ptr() -> *const UnsafeCell<Tick>`), and `UnsafeCell<Tick>` has no `.set()` method.

The same problem affects §7.4 — `pool.byte_ptr().add(source_row * stride)` to extract a `&[u8]` slice is invalid when the source row may live mid-chunk.
**Why critical**: this is not a refactoring concern; the entire migration algorithm in §7.2, §7.3, §7.4 is written against an API that does not exist. A developer following the plan literally would have to redesign the per-pool primitive on the fly, blocking implementation.
**What is needed**: rewrite §7.2 / §7.4 / §12.6 against the actual `units[idx].ptr()` + `chunks` storage model. Either (a) keep the per-Unit `ptr()` model — `swap_remove_index_no_drop` becomes a copy_nonoverlapping between two `Unit` pointers + `units.pop()`, mirroring the existing `swap_remove`; (b) explicitly introduce a separate plan section "ComponentPool storage rework to flat-bytes" before §7 with its own complexity budget. The retained-bytes extraction in §7.4 needs to use `units[row].ptr()` + a `from_raw_parts(ptr, layout.size())` slice, not stride arithmetic.

### 🟡 Important

#### W-N1. §7.4 `apply_replace_in_place` re-asserts target == source but never verifies the source archetype's component_ids() is a superset of `B`

**Where**: §7.4 (line 1338, `expect("invariant: target == source ⇒ all bundle components present")`)
**Problem**: the entry point to `apply_replace_in_place` from §6.3 only checks `target_archetype_id == source_archetype_id`. But that equality came from `merged_archetype_id`, which uses `source ∪ bundle`. Equality between merged and source archetype IDs implies bundle ⊆ source **only** if `merged_archetype_id` actually returns the source ID when bundle is empty/subset — which is a non-trivial dependence on the dedup logic. If the lookup ever produces a hash-collision-equivalent ID for a different signature, this `expect` panics on a path that should be guaranteed-by-construction.
**Solution options**: either prove the equivalence "merged_id == source_id ⇒ bundle ⊆ source" by referring to `get_or_create_archetype`'s canonicalization invariant (and quote it in the SAFETY block), or call `archetype.component_pools().has_pool(component_id)` defensively in debug builds.

#### W-N2. §7.3 remove-migration "drop then move_out_entity" ordering is subtler than the comment claims

**Where**: §7.3 (lines 1247-1264)
**Problem**: The plan drops the removed component at `source_row` via `drop_at`, then calls `move_out_entity(source_row)`. `move_out_entity` then swap-removes ALL pools at `source_row`, including the just-dropped one. The comment says "drop_at zeroed the logical state... the swap from last_row brings live bytes back into source_row. Net effect: no double-drop, no leak." But this requires `move_out_entity` to NOT call any drop on `source_row` for the C pool (it must use `swap_remove_index_no_drop`, which the plan confirms in §7.2). What is **not** explicitly stated: the moved-from `last_row` of the C pool is now a duplicate of the bytes at `source_row` of the C pool, but `source.move_out_entity` decrements `count` — so the byte at `last_row` is "logically out of range". If the pool's allocator later reuses that slot via `add`, it will overwrite the duplicate — fine. But this is fragile; a future change to `swap_remove_index_no_drop` that does drop the "from" slot would silently double-drop.
**Solution options**: add an invariant statement in §7.2: "`swap_remove_index_no_drop` MUST NOT invoke drop_fn on any slot (source or last) — caller assumes byte ownership tracking." Currently the contract only says "no drop on source-row bytes"; tighten to "no drop, period."

#### W-N3. C4 four-case proof has a residual gap on `Commands::entity_master_ptr` use across frames

**Where**: §4.6 case 4 + §5.6 SAFETY block
**Problem**: the proof argues against ID-collisions but does not address what happens if a stale handle is passed to `Commands::entity(stale)` in a system whose `Commands::get_param` was set up in a **prior** frame. The `entity_master_ptr` was minted at frame F via `UnsafeEcsCell<'w>` with `'w` = frame F's apply window. If the SystemParam state is reused across frames (which §5.6 init_state suggests — `CommandQueue::new()` only on init, not per frame), then on frame F+1 the `entity_master_ptr` should be **refreshed** by `get_param` per the per-call contract — but the plan does not explicitly state when `get_param` runs vs when the pointer becomes stale.
**Solution options**: explicitly state in §5.6 that `get_param` runs **per system invocation** (i.e. each frame), re-minting `entity_master_ptr` fresh — and that `Commands<'s>::Item<'w, 's>` is dropped at the end of the system body, so the pointer never outlives `'w`. Reference the existing Phase 8c IntoSystem contract that guarantees this.

### 🟢 Optional

#### O-N1. §10.5 contention numbers are reasonable but lack a measured baseline

The 10/20/30/60 ns cost model is the textbook x86 `lock xadd` curve, but boyko has no actual benchmark for it yet. Consider noting "if `bench_reserve_entity_parallel_8_threads` shows >100 ns/op" as a Phase 12 trigger — this is already done in §11.8, but §10.5 could reference it more tightly.

## Positive

- **C1 sketch is genuinely useful**. The §5.7 standalone sketch is exactly the right way to de-risk a lifetime change — running it through `cargo check` before integration eliminates an entire class of "doesn't compile" surprises.
- **C4 case 4 (stale fabrication)** is the right kind of proof — it walks through the actual code path (`is_entity_valid` at apply) rather than just hand-waving "the generation guard catches it".
- **W4 migration_scratch** is the right call. Per-frame reusable Vec on the dispatcher-only path, no contention, zero alloc after warmup. This matches principle 5 (Minimum allocations).
- **OQ5 decision recorded with rationale**. "Ship documented, fix in Phase 12" with three concrete reasons (Bevy parity, workaround exists, low impact) is a model trade-off acknowledgment.
- **Wave A's `allocate_entity` privacy tightening** + the trybuild test in §13.1 is good architectural discipline — eliminates the W2 ambiguity at the type level rather than via prose.
- **Loom tests (§13.7)** with explicit budget caps (2 threads × 100 ops, OQ9 escape hatch) are correctly scoped.

## Open questions for the architect

1. **Re W-N1**: is `get_or_create_archetype`'s canonicalization documented as "same signature ⇒ same id" (in particular for the `merged_archetype_id == source_archetype_id` case)? If yes, please cite it; if no, this needs a defensive check.
2. **Re C-N2**: would you prefer to (a) defer Phase 11 until ComponentPool storage is flattened to contiguous bytes (likely a separate plan), or (b) rewrite §7.2/§7.4 to match the existing chunked + Unit-pointer storage? Option (b) is faster but the per-row copy becomes a Unit-pointer dance rather than stride arithmetic, which affects the §7.5 cost numbers.
3. **Re C-N1**: are you open to introducing an `EntityCounter` newtype exposed via `Commands::entity_master()` (carrying only `next_entity_id: &AtomicUsize`) rather than `&EntityMaster`? This makes the aliasing rule type-enforced.

---

Relevant files (absolute paths):
- `D:\claude\BoykoEngine\docs\PHASE-11-ENTITY-COMMANDS-PLAN.md` — plan v2 under review
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs` — actual ComponentPool storage model (chunked + units Vec, lines 48-51, 339-424) that contradicts §7.2 stride-based pseudocode
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs` — existing component_pools_mut + get_pool_mut (lines 510-516)