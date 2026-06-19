# Architecture Audit — ECS-Native Data Remediation (kill parallel data systems)

> Whole-codebase audit (owner-mandated: boyko_ecs is THE SDK for logic AND data; no parallel std::Vec/HashMap data system; use the real ComponentPool, not a bespoke primitive). Classification → ComponentPool answer → hard-tension resolution → staged remediation → adversarial critique → the REVISED plan this repo will execute.

## The principle (precise)
NOT "no std::Vec anywhere." It is: **durable per-entity/per-body/per-element subsystem WORLD DATA must live in the ECS's own storage — `ComponentPool` (per-entity columns) or `Resource`-owned `ComponentPool` host columns (non-entity bulk) — not a parallel `std::Vec`/`HashMap` keyed by a subsystem-minted id.** One data system.

## Audit verdict — most of the codebase is CLEAN
LEGIT: boyko_ecs core (it IS the storage impl); boyko_utils (SparseMap/BitSet — the array-by-id primitives the principle mandates); boyko_threadpool (Chase-Lev deques + cold-only Mutexes); boyko_serialize (function-local I/O scratch); boyko_input (singleton state in `Resource`s as `Box<[T]>` by action-id); boyko_sdf_math (stateless); boyko_rhi_vulkan/boyko_render (FFI/GPU/OS-contiguity buffers — Vulkan needs `*const T+count`; can't be components). "no HashMap" rule UPHELD outside the ECS core; the one exception (`boyko_input/resource_id.rs:56`, `OnceLock<Mutex<HashMap<TypeId,ResourceId>>>`) is cold + rust#22991-forced → accepted.

## The violations — ALL in boyko_physics
- **S1 (worst, the SP4 class)** — body MIRRORS duplicating component columns: `SolverScratch.bodies: Vec<BodyState>` (resources.rs:2632, gather copy of RigidBody+RigidBodyMass), `SoftStepSolver.bodies` (soft_step.rs:187), `ColoredSoftStepSolver.bodies` (colored.rs:589 — the buffer the SP4 race re-indexed via raw `*mut` across workers).
- **S2 (cross-frame per-body/per-pair)** — `IslandSleep.asleep/below_count` (per-body sleep, side Vec), `WarmStartTable` ×4 (warm-start impulses, open-addressed), `BoxAxisCache` (SAT hysteresis).
- **S3 (per-step SoA/CSR scratch, ~20 resources / ~90 Vec fields)** — ContactPairs, Manifolds, BroadphaseGrid CSR ×10, ConstraintGraph ×8, ColoredSoftStepSolver `ContactColumns` (31 scalar SIMD lanes), SoftColorScratch ×22, etc. Pattern (CSR/SoA, cleared-and-refilled, 0-alloc) is already correct; only the backing medium (std::Vec → ComponentPool) changes.
- **S4 (cold smell)** — `GpuColumnManager.meta` (render-side `(archetype,component)`→handle map; fold into per-column archetype metadata; lowest urgency).
- **N (not a violation, flagged)** — `SoftBody` IS a Component but holds ~30 inner heap `Vec`s (variable-length-payload component-model gap; accepted L2 exception for now).

## ComponentPool, NOT a bespoke primitive (the owner's question, answered)
`ComponentPool` already IS the low-level primitive: type-erased (any Layout by id), **VmReservation-backed with address-stable in-place growth** (write-once bases, zero-copy grow — `std::Vec` reallocates and MOVES, invalidating every cached raw ptr — the SP4 footgun), change-detection-native, drop_fn-aware, SIMD-aligned, `row_ptr`-provenance-clean for lock-free parallel writes (component_pool.rs:184-189 proof). A bespoke column re-fragments the engine into two data systems and re-opens the SP4 gap. For non-entity bulk: a `Resource` that OWNS a `ComponentPool` host column.

## The hard tension + DECISION (critique-corrected)
The gather-into-dense-`BodyEffective` exists because `ComponentPool`s are partitioned BY ARCHETYPE (bodies in different archetypes = different pools = non-contiguous) AND — the critique's key finding — **body access in the colored inner loop is SCATTERED** (`body_a[i]`/`body_b[i]` random-index into the body buffer; the SIMD kernels stride the 31-lane `ContactColumns`, NOT a body array). The gather turns scattered body reads into a dense AVX-friendly mirror. **So the gather is a CACHE OPTIMIZATION, not pure debt — naively deleting it would REGRESS the solver.**

DECISION (mine, perf-driven): **keep the gather, but move its buffer + all physics bulk onto `ComponentPool`** (option (c)). This kills the parallel `std::Vec` data system AND fixes SP4 byte-identically (address-stable + per-row `row_ptr` provenance, no whole-buffer `&mut[..]` reborrow), WITHOUT touching the SIMD inner loop. The "pure operate-in-place / single body archetype / delete the gather" ideal is DEFERRED to a measured, spike-gated follow-up (it must beat the gather's cache win, which is not assumed).

## REVISED staged plan (what we execute)
- **Stage 0 (enabler):** `ComponentPool::new_scratch(layout, reserve_rows)` (synthetic-id, registry-free, tick sub-regions reserved-uncommitted) + a `ScratchColumn<T: Copy>` newtype. **C3 type-split (mandatory):** a `BuildView` (gives `&mut[T]` for single-threaded clear+refill) and a `SolveView` (`row_ptr(i)` ONLY — NO `as_mut_slice`) so the type system FORBIDS the whole-buffer reborrow in parallel paths. `!needs_drop::<T>()` asserted; synthetic id in a reserved non-colliding range (W3); 128B header pin preserved. Gate: ECS-core 0%-gate (byte-identical codegen on existing paths); ScratchColumn unit tests (zero-copy grow, address-stable, clear-no-free); Miri.
- **Stage 1' (the SP4 fix — KEEP the gather):** swap `ColoredSoftStepSolver.bodies` / `SolverScratch.bodies` / `SoftStepSolver.bodies` from `std::Vec` to `ScratchColumn`; gather unchanged; parallel access via `SolveView::row_ptr` (NO `as_mut_slice` in any worker). **Sentinel invariant (C3):** `body_b == SDF_SENTINEL` rows are NEVER written (not even a no-op store); the disjointness proof = "distinct DYNAMIC rows AND sentinel-never-written". Gate (C1-corrected): run-to-run bit-identity on the new path + the existing tolerance acceptance gates + proof the per-color group + intra-group visitation order is unchanged; **Miri-TB on the colored kernel WITH an SDF-sentinel-in-a-color test**; 0%-regression (solver criterion, asm-verify the hot loop strides identically). This is the standalone SP4 fix.
- **Stage 4 (the bulk):** mechanically swap the S3 per-step `Vec` fields → `ScratchColumn`, layout byte-identical. Gate: full physics determinism bit-identity (byte-identical — pure backing swap, no op change); Miri-TB on parallel consumers; 0%-regression on every physics bench.
- **Stage 3 (S2):** sleep → `Sleeping` component + `EnableColumn` paged-bitset tag (O(1) flip, no migration churn). Warm-start/axis-cache → **Resource-owned `ComponentPool`** keyed by the canonical pair key (NOT pair-entities — W2: pair-entity archetype-row order depends on spawn/despawn/recycle order → breaks the warm-store deterministic probe-order contract, colored.rs:67-78). Gate: warm-start/sleep determinism bit-identity; Miri-TB; 0%-regression.
- **Stage 5 (S4):** fold `GpuColumnManager.meta` into per-column archetype metadata (`PoolBacking::Device` arm). Cold; lowest urgency.
- **Stage 6:** document accepted exceptions (input HashMap, ECS-core cold registries/scheduler build-maps).

## Critique's critical corrections folded in
- **C1 determinism gate split:** pure-backing-swap stages (4, soft-mirror of 1') → byte-identical pre/post mandatory. Stage 1' (touches traversal) → run-to-run bit-identity + tolerance gates + visitation-order-unchanged proof (the colored solve has NO bit-baseline vs reference by design — colored.rs:30).
- **C2 keep the gather** (cache opt; scattered body access) — DECIDED above. Pure-in-place deferred.
- **C3 type-split ScratchColumn** (BuildView slice vs SolveView row_ptr-only) + sentinel-never-written invariant + Miri-TB sentinel test.
- **C4 honest framing:** the real win is ADDRESS-STABILITY-on-grow (no realloc-move invalidating cached raw ptrs), not "ComponentPool is faster on the hot loop." Solver scratch columns do NOT use change-detection (raw stride, no tick bump); RigidBody change-detection (if any consumer relies on `Changed<RigidBody>`) is handled at `physics_apply` write-back (once/body/step), not the inner loop.
- **W2** warm-start/axis-cache stay Resource-owned ComponentPool (not entities) for determinism. **W3** synthetic-id spec. **W4** metric restated: 0 heap allocs + 0 copies after warmup; page commits amortize to 0 at the contact high-water mark.

## Residual owner VALUES/SCOPE calls (NON-blocking — only affect deferred work)
1. **Contacts as gameplay-visible ECS data** (queryable/observable collision events as entities) vs pure solver scratch? Only affects the deferred pure-in-place / contacts-as-entities path; default = pure scratch (Resource-owned ComponentPool).
2. **`SoftBody` variable-length payload** (~30 inner Vecs) — a variable-length component model (particle-as-entity, or component-owned sub-columns)? Default = accepted L2 exception for now.
3. The pure operate-in-place / single-body-archetype follow-up — pursue only if a measured spike shows it beats the gather's cache win.

## Legit-keep (honesty, not zealotry)
boyko_ecs core storage; boyko_utils primitives; threadpool infra + cold Mutexes; serialize I/O scratch; FFI/GPU/OS-contiguity buffers; input flat-buffer Resources; SoftBody inner Vecs (L2 exception); the one input HashMap (rust#22991-forced).
