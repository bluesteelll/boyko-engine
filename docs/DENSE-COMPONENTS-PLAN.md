# Dense (non-fragmenting) Components — a first-class boyko_ecs storage kind (FINAL)

> Status: APPROVED-in-substance; every CHANGES-REQUESTED item folded + verified against live code on branch `ecs`. A developer can implement D0 immediately. (Verification corrected the design: C1 = 15 sites with a silent reader-fall-through bug; C2's "rides the enable slot" was wrong — dense rides `IS_ARCHETYPAL`; C3 does NOT dissolve — the colorer reads absolute slot values; W2 = 32 fire sites.)

## Problem & goal
The colored physics solver needs ONE contiguous SoA buffer per body field. Archetype-Table storage fragments bodies across N columns; today the solver mirrors them in `std::Vec<BodyState>`/`std::Vec<BodyEffective>` + a per-frame gather, and that parallel-mutated mirror is the structural root of the SP4 race (unsound whole-buffer `&mut[T]` reborrow handed to workers). Goal: a third `StorageKind::Dense` whose ONE global column holds every instance of the type across all archetypes, never fragments archetypes, first-class across Query/change-detection/scheduler/serialization/hooks. The solver reads/writes bodies natively → the Vec mirrors disappear → SP4 becomes UN-TYPEABLE (compile error). Non-goals: relationships; paged SparseMap; GPU-resident dense (dense is ALWAYS `ResidencyKind::Cpu`).

## Target metrics
- **0%-gate (load-bearing):** all-Table/all-Bitset query/spawn/iter codegen byte-identical (asm-diff hot loops). Define no dense type → pay nothing.
- Pure-dense iteration: stride ONE contiguous column, 1 predictable liveness compare/slot (const-folds to ~0 in zero-tombstone steady state).
- Mixed iteration: archetype-driven + per-row gather `entity_ids[row]→e2s.get→row_ptr` (one SparseMap indirection/row).
- Dense spawn/insert/remove: NO archetype migration (the payoff).
- Scheduler: one dense column = exactly ONE conflict node.
- Stage P: SP4 structurally impossible; 0%-regression across broadphase+narrowphase+solve.

## Decisions (final)
1. **Storage = `DenseStore`**: `column: ComponentPool` (`ComponentPool::new(id, reserve_rows)`, no arena; row_ptr provenance component_pool.rs:645-662) + `e2s: SparseMap<u32>` (EntityId→slot) + `s2e: Vec<EntityId>` (deterministic order + serde key) + `live: BitSet` (per-slot liveness, NEW — W3) + `free: Vec<u32>` (LIFO) + `arch_presence: ArchetypeBitSet` (seed). Invariant: `e2s[s2e[slot]]==slot ∀ live slot; !live(s) ⟺ s∈free`.
2. **Fully non-fragmenting**: dense excluded from the signature + no per-archetype pool (it has the global `DenseStore.column`). Membership = `arch_presence` seeds via `seed_from_candidates` + per-row `e2s.contains(entity)` on the `IS_ARCHETYPAL=false` const-fold path (C2).
3. **Deterministic order = tombstone + free-list** (EnTT in-place deletion), NOT swap-remove. Live slots never move. `compact()` cold, fixed schedule point. Claim downgraded per C3.
4. **Query**: keep `Query<D,F>`; const-gated arms. Pure-dense (`ALL_TERMS_DENSE`) strides ONE column; mixed = archetype-driven + per-row gather; all-archetypal UNCHANGED (dense arm const-folds out — 0%-gate).
5. **Change detection**: per-slot ticks already in `DenseStore.column`; `Mut` bumps changed-tick by slot; Added/Changed by slot + `Tick::is_newer_than`.
6. **Scheduler**: one dense column = one conflict node (access.rs + conflict_graph.rs are per-ComponentId-whole-world; whole-world serialization is EXACT for a global buffer). Dense is Cpu (W1) so `seed_from_candidates`' GPU assert holds.
7. **Serialization**: SerPod column-blit of the compacted-at-save live column + `s2e`, entity-remapped via existing `LoadEntityMap`/`remap_loaded_entities`.
8. **Parallel/SP4 fix = type-split**: `DenseBuildView` (!Send) = the ONLY `as_mut_slice`/push/tombstone/compact; `DenseSolveView` (Copy, Send+Sync, 32B) = `row_ptr(slot)`/`len`/`is_live` ONLY — NO `as_mut_slice`/`DerefMut<[T]>`. The SP4 whole-buffer reborrow is un-typeable. SAFETY: address-stable base, per-element row_ptr provenance, scheduler-serialized, distinct-slot writes, `debug_assert!(live.test(slot))` in row_ptr (W3).
9. **API**: `#[component(storage="dense")]`; derive emits `const STORAGE_IS_DENSE` + `set_storage_kind(id, Dense)` + `set_residency_class(id, Cpu)` (W1); `StorageKind::Dense=2`; `STORAGE_IS_DENSE=false` default (0%-gate). Spawn/insert/remove route to `DenseStore`, no migration.
10. **Physics (Stage P)**: RigidBody*/velocity→dense; contacts→dense slots (resolved at contact-build); DELETE the Vec mirrors; solver inner loop via `DenseSolveView::row_ptr`; 31-lane `ContactColumns` UNTOUCHED. AVX re-gather into a contiguous `ScratchColumn` is the DEFAULT (W4).

## C1 (RESOLVED) — 15-site fan-out + the silent reader-fall-through bug
The `storage_kind()` reader (component_registry.rs:445-451) matches only `Bitset` then `_ => Table`, so discriminant 2 (Dense) reads back as `Table` → a dense id silently re-enters the signature and fragments, NO compile error. **Fix first (D0).** Mandate predicate `#[inline] pub fn is_signature_storage(k: StorageKind) -> bool { matches!(k, StorageKind::Table) }`; every exclude/skip site becomes `if !is_signature_storage(storage_kind(id))`.

| # | File:line | Current | Dense behavior |
|---|---|---|---|
| 0 | component_registry.rs:445-451 (reader) | Bitset arm, `_ => Table` (Dense→Table BUG) | ADD explicit Dense arm |
| 1 | archetype.rs:277 filtered_signature_mask | ==Bitset→continue | EXCLUDE |
| 2 | archetype.rs:345 create_by_ids | ==Bitset→continue | EXCLUDE + skip per-archetype pool (dense has its OWN global store — not Bitset no-storage) |
| 3 | archetype.rs:373 register_component | ==Bitset→return false | refuse Table registration |
| 4 | archetype.rs:416 register_component_inplace | ==Bitset→skip pool | skip per-archetype pool |
| 5 | archetype.rs:529-530 set_enable_bit | debug_assert_eq Bitset | LEAVE (Bitset-only) |
| 6 | archetype_bundle.rs:450 | ==Bitset→continue | EXCLUDE |
| 7 | archetype_master.rs:189 create_archetype | ==Bitset→continue | EXCLUDE |
| 8 | archetype_master.rs:511 slab mint | ==Bitset→continue | EXCLUDE |
| 9 | archetype_master.rs:727-728 add_component_to_archetype | debug_assert_ne Bitset | WIDEN to is_signature_storage |
| 10 | component_registry.rs:491-499 set_storage_kind | write-once current==Table | LEAVE |
| 11 | clone/materialize.rs:235 | ==Bitset→continue | EXCLUDE from row clone; materialize dense membership separately + fire (W2) |
| 12 | enable_tag_api.rs:150-151 | debug_assert_eq Bitset | LEAVE |
| 13 | filter_enable.rs:170-171 Enabled::init_state | debug_assert_eq Bitset | LEAVE |
| 14 | filter_enable.rs:346-347 Disabled::init_state | debug_assert_eq Bitset | LEAVE |

Dense signature-EXCLUDEs like Bitset but does NOT share Bitset's "no storage" — it skips the PER-ARCHETYPE pool only; a separate D1 registration creates the ONE global `DenseStore`. A debug_assert at `DenseStore` lookup that the id is classified Dense guards a missing store.

## C2 (RESOLVED) — entity-keyed gate rides `IS_ARCHETYPAL=false`, NOT the enable seam
The enable seam (iter.rs:233, EnableTermCols::passes enable_terms.rs:182-205) is a RUNTIME hoisted branch, row-slot-keyed. Dense is EntityId-keyed. Committed seam = the change-detection const-fold twin at iter.rs:212: `if !const { F::IS_ARCHETYPAL } { filter_fetch(...) }` (IS_ARCHETYPAL filter.rs:87). Dense terms set `IS_ARCHETYPAL=false` (like Changed/Added). No dense term → stays true → const-folds out → byte-identical (0%-gate). Per-row mixed fetch: `entity = entity_ids_slice()[row]` (archetype.rs:1396) → `slot = e2s.get(entity)` (None→With fails/Without passes) → `debug_assert!(live.test(slot))` → `row_ptr(slot)`. MIXED = per-row gather (scatter in the dense column). Pure-dense (`ALL_TERMS_DENSE`, modelled on IS_SOLE_SINGLE_ENABLE filter.rs:141): dense-driven `for slot in 0..len { if !live.test(slot) {continue} row_ptr(slot); entity=s2e[slot] }` — ONLY this strides contiguously (the solver path).

## C3 (RESOLVED) — determinism finding + downgraded claim + proof + compact() boundary
(a) Apply-window order DETERMINISTIC: `CommandQueue::apply` (command_queue.rs:216-264) drains FIFO; the scheduler drains per-system queues in registration/topo order. CAVEAT: entity-id VALUES across parallel reservations are timing-dependent (`reserve_entity = fetch_add(1, Relaxed)`, entity_master.rs:181) — uniqueness only.
(b) **KEY FINDING:** coloring DEPENDS on absolute body slot values — `color_manifolds` (resources.rs ~1943-1999) first-fits over a body-INDEX occupancy bitset. So free-list churn changing slot VALUES can pick different colors → bit-different (physically valid) output. **C3 does NOT dissolve.**
(c) **Downgraded claim + proof.** Claim: for a FIXED, deterministically-ordered op sequence, `DenseStore` produces identical slot assignments run-to-run → identical coloring → BIT-IDENTICAL solver output run-to-run. Proof: (1) slot assignment is a pure function of the op-sequence (insert pops free LIFO or pushes len; tombstone pushes free; structural mutation single-threaded via `DenseBuildView` !Send). (2) op-sequence deterministic ⟸ apply-order deterministic (a) ∧ dense ops in apply-window FIFO order; the one non-determinism (entity-id VALUES across threads) never reaches slot assignment (slots assigned by op-order, not id value). (3) identical slots ⟹ identical `color_manifolds` reads ⟹ identical coloring ⟹ identical packing ⟹ bit-identical. ∎ NOT claimed: identity across different op-orderings.
(d) **compact() boundary**: an exclusive-world system at exactly one fixed point between physics FixedUpdate runs, never within a substep. Enforcement: reachable ONLY through `DenseBuildView` (!Send) → cannot run on a worker or concurrently with any `DenseSolveView` (read-only); registered exclusive with before/after(PhysicsSet::Solve). The substep loop borrows only `DenseSolveView`; compact needs `DenseBuildView` (exclusive); the conflict graph (dense id = one write node) forbids overlap → slots invariant for the full solve. ∎

## W1–W4 / O1–O2 (RESOLVED)
- **W1**: dense ALWAYS `ResidencyKind::Cpu` (component_registry.rs:518-545; set_residency_class write-once); derive emits `set_residency_class(id, Cpu)`; reject `storage="dense"`+gpu (debug-assert + derive compile error). `seed_from_candidates` GPU assert (query_state.rs:341) then holds.
- **W2**: 32 fire sites enumerated; `spawn_batch`/clone-materialize/hierarchy-cascade fire NOTHING today → dense routing adds NEW fire paths. Sites: spawn (ecs_master.rs:718/723/732/737), spawn_at (:862/867/876/881), insert (migration_helpers.rs:764-813 ×10), remove (:983-999 ×6), despawn (ecs_master.rs:1139-1182 ×7), spawn_batch (spawn_batch_*: none today), clone-materialize (materialize.rs:235: none), hierarchy cascade (rides despawn). D2 gate = per-API fire-COUNT tests + 0%-gate archetypal counts (Phase 14a/14b lesson).
- **W3**: `ComponentPool::row_ptr` (component_pool.rs:645) is BOUNDS-only (no liveness). Add `live: BitSet` to `DenseStore` (O(1) oracle + iteration skip + read-only during solve); `DenseSolveView::row_ptr` debug_asserts `live.test(slot)` via a pub(crate) liveness-checked accessor.
- **W4**: Stage P scope = BodyState/RigidBody* across broadphase (resources.rs:824/866/1130/690 + EmitPtrs *const wrapper :1488-1567) + narrowphase + solver. Tombstones poison naive as_slice → DEFAULT: compact at the fixed point → contiguous column for broadphase; the dense→`ScratchColumn` re-gather is the AVX DEFAULT (scattered row_ptr risks regression vs contiguous gather — colored.rs:641). 0%-regression covers broadphase.
- **O1/O2**: `StorageKind::Dense=2`; relationships=3 (doc at component_registry.rs:393-394 updated). Dense draws from the shared MAX_COMPONENTS=512 id budget.

## Data structures
`DenseStore { column: ComponentPool, e2s: SparseMap<u32>, s2e: Vec<EntityId>, live: BitSet, free: Vec<u32>, arch_presence: ArchetypeBitSet, id }`. `DenseBuildView<'a>{ store: &'a mut DenseStore }` !Send. `DenseSolveView<'a>{ base: *mut u8, stride, len, live: *const BitSetWords }` Copy+Send+Sync 32B — `row_ptr(slot)=base.add(slot*stride)` + live debug_assert; NO `as_mut_slice`/`DerefMut`.

## Staged build plan + gates
- **D0**: `Dense=2` + reader Dense arm (C1 #0) + `is_signature_storage` + rewrite C1 sites #1,2,4,6,7,8 + widen #9 + #3; derive dense arm (`STORAGE_IS_DENSE` + `set_storage_kind` + `set_residency_class` Cpu). Gate: asm-diff byte-identical Table/Bitset hot loops; test `storage_kind(dense)==Dense` + signature-excluded; reject dense+gpu.
- **D1**: `DenseStore` + views; insert/remove(tombstone)/slot_of/contains/compact + `live` BitSet. Gate: unit (reuse, deterministic order, address-stability, compact); Miri; property `e2s[s2e[s]]==s ∧ !live(s)⟺s∈free`; trybuild compile-fail (no `&mut[T]`) + static_assert Send/Sync vs !Send.
- **D2**: route 8 structural ops to `DenseStore` (no migration) + fire per the W2 32-site table. Gate: per-API fire-COUNT tests; 0%-gate archetypal counts; spawn_batch/clone/cascade NEW fires verified.
- **D3**: `IS_ARCHETYPAL=false` mixed gather (C2) + `ALL_TERMS_DENSE` pure-dense + With/Without/seed_from_candidates. Gate: pure-dense order; mixed exactness incl Without; 0%-gate asm-diff on IS_ARCHETYPAL=true.
- **D4**: per-slot ticks + column-blit serde of compacted snapshot + s2e + remap. Gate: change visibility; round-trip bit-identity; remap correctness.
- **Stage P**: bodies→dense; contacts→slots; delete Vec mirrors; `DenseSolveView::row_ptr`; ScratchColumn re-gather default (W4); broadphase from compacted column. Gate: run-to-run bit-identity on the fixed deterministic op-sequence (C3c) + tolerance; **Miri-TB on the colored kernel WITH SDF-sentinel-in-a-color**; 0%-regression across broadphase+narrowphase+solve_color (criterion + asm); compact()-boundary test (C3d).

D0–D4 (kernel) land first; Stage P consumes them.

## Proof obligations (must hold at each gate)
1. 0%-gate: asm-diff IS_ARCHETYPAL=true body byte-identical. 2. SP4 un-typeability: trybuild + static_assertions (DenseSolveView Send+Sync no `&mut[T]`; DenseBuildView !Send). 3. Determinism (C3c): two-run identical s2e + bit-identical output + a different-op-order counter-test. 4. compact() boundary (C3d): no overlap with DenseSolveView. 5. Liveness (W3): Miri trips the live assert on a tombstoned slot. 6. Fire counts (W2): exact per API. 7. Reader regression (C1 #0): storage_kind(dense)==Dense.

## Residual owner VALUES/SCOPE (non-blocking; do NOT block D0)
1. Positioning text dense vs EnableTag. 2. Dense fires hooks uniformly (default) vs flecs DontFragment exemption. 3. ScratchColumn re-gather kept as default; dropping it for a future ISA is a measured spike. 4. SparseMap stays FLAT (architect call); page only if a future dense type uses sparse ids.

## Kept from the design (critic-praised)
Conflict-graph one-node; `ComponentPool::new` direct (no synthetic-id/new_scratch); the BuildView/SolveView split; `seed_from_candidates`; tombstone+free-list.
