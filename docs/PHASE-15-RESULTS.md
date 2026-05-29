# Phase 15 — Results (Explicit System Ordering & Schedule Sets)

**Status:** ✅ COMPLETE (uncommitted → committed locally on branch `ecs`).
Completes the Phase 9 dormant ordering scaffold; 0%-regression verified;
no `unsafe` added; the executor hot path is byte-identical.

## What shipped
Explicit user-specified system ordering on top of Phase 9's conflict-derived
scheduling — finishing "Wave 5 Step 14" rather than greenfield work (research
found ~70% already present + tested).
- **Set membership + ordering**: `SystemConfig::{in_set, before_set, after_set}`
  (value-based), `ScheduleBuilder::configure_set(set)` + `ConfigureSet::{before,
  after, in_set}` (set-level + set-hierarchy ordering, all value-based).
- **Build-time expansion**: set hierarchy is flattened to transitive leaf
  membership (D3, iterative WHITE/GRAY/BLACK DFS with cycle detection), then
  `InSet`/set-level edges expand to pairwise `(SystemKey, SystemKey)` edges (D1)
  that feed the EXISTING Tarjan-SCC + Kahn-topo + `ConflictGraph::build`
  pipeline. An ordering edge already forces serial execution via a "false
  conflict" bit (Bevy `before` parity) — unchanged from Phase 9.
- **`SystemSetId`** stays `#[repr(transparent)] usize`, interned from
  `(TypeId, set_discriminant())`. The config path (`configure_set(E::A)`) and the
  membership path (`in_set(E::A)`) resolve to the SAME id — the crux correctness
  property, test-verified.
- **`#[derive(SystemSet)]`** extended for fieldless enums (variant index →
  discriminant; `"Type::Variant"` name); data-carrying variants / unions /
  generics rejected with clear compile errors.
- **Diagnostics**: `try_build() -> Result<Schedule, ScheduleBuildError>` +
  panicking `build()`. Errors: `OrderingCycle` (B9001), `SetHierarchyCycle`
  (B9002), `SetsOrderedButIntersect` (B9004), `UnknownSystemKey` (B9005, debug
  AND release). A set named in an ordering edge but never joined → build warning
  `boyko-W1501` (never a silent no-op, never an error).

## Pipeline
research (found the dormant scaffold) → architect → critic R1 (CHANGES: C1/C2/C3
value-vs-type inconsistency + W1/W2; hot-path + sync-points verified sound) →
§13 Round-2 patches → critic R2 (CHANGES: C-NEW-1 `SystemSetId` spec
contradiction, W-NEW-1 empty-set warning gap, OQ-1 vestigial error) → §13.1
Round-3 corrections (APPROVED) → developer (6 files, +808/-127, no unsafe, hot
path untouched) → code-review APPROVED (1 MEDIUM: dead `OrderTarget` public API,
removed) → tester (24 tests, 0% gate).

## Findings (resolved during review)
- The architecture's hard question (conflict-vs-ordering interaction) was ALREADY
  answered correctly by Phase 9's code — no redesign. The critic rounds were
  about plan self-consistency (the value-based set API + the `SystemSetId`
  representation), not the scheduler's correctness.
- **C1 (the keying crux)**: collapsed the whole set API onto a single
  value-based `set_id_of_value` intern path keyed on `(TypeId, discriminant)`, so
  config and membership cannot resolve to different ids — verified by the
  `config_and_membership_resolve_same_set_id` test.
- Dead `OrderTarget` public enum (orphaned by the value-based redesign) removed
  before merge.

## Measured results
- **Correctness**: 19 integration tests (InSet expansion incl. a discriminating
  reorder-against-insertion-order control, system↔set, set↔set cartesian,
  hierarchy flatten, diamond dedup, config↔membership agreement, enum-variant
  hierarchy + distinctness, all 4 build errors via both `try_build` + `build`,
  empty-set warning contract) + 4 trybuild compile-fail (data-carrying variant /
  union / generic / tuple-struct rejected) — all pass.
- **Full workspace**: 767 passed, 0 failed, 2 ignored. (Two flakes —
  `into_system_exclusive_smoke` + `event_multi_type` clippy — confirmed
  PRE-EXISTING on baseline `557f509`, shared-global-static / unrelated-lint, not
  Phase 15.)
- **Miri** (`-Zmiri-tree-borrows`): 4/4 clean, zero UB (Phase 15 added no
  `unsafe`; build-time `Vec`/`HashMap` only).
- **0%-regression gate**: A/B via `git stash` vs pre-Phase-15 (`557f509`) on the
  50-empty-systems bench — within ±2% (16.26 µs baseline; Phase 15 runs 16.78 /
  15.99 µs straddle it; criterion "no change detected"). The Phase-9 "50 systems
  1.72× faster than Bevy" headroom is preserved (executor hot path byte-identical
  — `schedule.rs`/`executor_scratch.rs`/`conflict_graph.rs`/`bitset_intersects.rs`
  empty diffs).
- **Build**: `cargo build --release` + `cargo clippy -p boyko-ecs --lib -p
  boyko-macros -- -D warnings` clean.

## Residuals / follow-ups
| Item | Status |
|---|---|
| Auto sync-point insertion (coalesced command flush) | Deferred — `insert_sync_points` stays no-op; correctness-neutral (per-system apply window is already a sync point). A future parallelism optimization. |
| `before_ignore_deferred` (command-flush opt-out ordering) | Deferred — parallelism opt, not correctness. |
| Drop the redundant conflict bit for pure (non-data-conflicting) ordering edges | Deferred — benchmark-gated micro-opt; would touch the 0%-protected `conflict_graph.rs` invariant. |
| `EcsMaster::schedule_builder()` convenience | Out of scope (no ECS-core change needed). |

## Key files
- Modified: `crates/boyko_ecs/src/ecs/core/schedule/{mod,ordering,schedule_builder,system_config,system_set}.rs` (`schedule_builder.rs` is the bulk: `set_id_of_value`, `configure_set`/`ConfigureSet`, `flatten_set_membership`, `expand_set_edges`, `ScheduleBuildError`, `try_build`), `crates/boyko_macros/src/lib.rs` (enum `SystemSet` derive).
- Untouched (hot path): `schedule.rs`, `executor_scratch.rs`, `conflict_graph.rs`, `bitset_intersects.rs`.
- Tests: `tests/phase15_set_ordering.rs`, `tests/system_set_compile_fail.rs` (+4 fixtures), `tests/miri_phase15.rs`.
- Docs: `PHASE-15-RESEARCH.md`, `PHASE-15-PLAN.md` (incl. §13 Round-2 + §13.1 Round-3), this file.
