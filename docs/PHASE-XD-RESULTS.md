# Phase X.D — Results: EntityMaster slot reduction (`active_ids` + `sparse_to_active` elimination)

Branch `ecs`. PERF + simplification refactor of `EntityMaster`. Full pipeline:
architect (audit + design) → architecture-critic (REVISE → amendments folded) → developer →
code-review (APPROVED) → tester (suite + Miri + A/B bench). Contained change that **net-removes
`unsafe`** and deletes a maintenance invariant.

## Status: COMPLETE — despawn measurably faster, hot read 0%, Miri-clean, one documented cold-API cost

### The change
Deleted `EntityMaster.active_ids: Vec<EntityId>` (dense live list) and
`EntityMaster.sparse_to_active: Vec<u32>` (sparse→dense map). Replaced with a single
`live_count: usize`. The mandatory `entities_inland: Vec<EntityInland>` (the Phase-7 fast store read
by the hot `get_component_raw` path; 16 B/slot; `is_null()` = the single liveness + generation
source of truth) is **retained unchanged**.

- `register_entity_with_ptr` / `register_batch`: write only the `entities_inland` slot(s); bump
  `live_count`. (Dropped the `active_ids.push`/`.extend` + the `sparse_to_active` writes.)
- `deallocate_entity`: dropped the **entire swap-remove dance** (read dense idx, `swap_remove`,
  moved-entity sparse fix-up, sparse null) — keeps only the gen-bump + null + free-list push;
  `live_count -= 1` **on the success path only**.
- `iter_entities`: now an O(capacity) scan of `entities_inland` filtering `!is_null()`, yielding
  ascending `EntityId`. `entity_count` / `is_empty`: `live_count`-backed.

### The roadmap premise was inverted by investigation
The roadmap framed X.D as *"speculative — needs careful design / despawn invariant rework / 1-2
weeks"*, assuming we'd keep `active_ids` and make `sparse_to_active` lazy. Investigation showed
**`iter_entities()` has ZERO hot-path callers** (workspace-wide: 2 benches + 8 `.next()` test
helpers; every system/query/scheduler/command iterates via `Query` over archetypes). So
`active_ids` + `sparse_to_active` were a sparse-set acceleration structure whose only consumer is a
cold API, and the despawn swap-remove "invariant" existed *only* to maintain `active_ids`.
Deleting both **deletes the invariant** rather than reworking it. This is the Phase X.B lesson again:
a parallel structure whose state is derivable (or whose consumer is cold) is pure debt.

### Net-removes `unsafe`
This refactor DELETES unsafe-adjacent complexity: the `register_batch` tandem `&mut [..]`-slice
write + its `dense_base`/`u32`-overflow bookkeeping, and the `deallocate_entity` swap-remove
fix-up. No `unsafe` was added. The `Send`/`Sync` shared surface shrank from "3 vectors + counter"
to "1 vector + counter + a dispatcher-only `usize`" (the only worker-reachable field remains the
`next_entity_id` atomic via `EntityCounter`). The stale SEND5 comment (the obsolete
"pre-allocate to MAX_ENTITIES_HINT 64000" claim, removed in Phase 12.6) was corrected to the actual
lazy-growth-under-`&mut self`-during-apply-window (SCH7) discipline.

## Verification gate (tester-run)

| Oracle | Result |
|--------|--------|
| **Correctness** | **845 pass debug / 830 release**, 0 failures. New: `live_count_tracks_register_and_deallocate`, `register_batch_sets_live_count`, `iter_entities_after_sparse_churn_yields_survivors_ascending`, `clear_resets_live_count`, `deallocate_unregistered_recycled_id_is_noop_and_preserves_live_count` (C1 regression), `live_count_equals_non_null_inland_count_after_churn` (W1 tripwire). All named pre-existing entity tests stay green. |
| **Miri** (`-Zmiri-tree-borrows`) | **CLEAN** — 19 tests across `miri_phase8cd` (11) + `miri_phase14a` (4) + `miri_phase8_5` (4), 0 UB. The only end-of-process leak is the **pre-existing by-design** `BundleColumnCache` `Box::leak` (Phase 12.5 SBO6), suppressed with the documented `-Zmiri-ignore-leaks`; execution itself is UB-free (Miri aborts on UB and these ran to completion). Surface SHRANK. |
| **Despawn (the win)** | `delete_entity_10k` **−7.65% (p=0.00)** — clean win (shed the per-despawn swap-remove + sparse fix-up + a branch). Measured via a tester-added public-API churn bench under the git-stash A/B protocol. |
| **Single spawn** | `create_entity_10k` **−1.38% (p=0.05)** — small win (one fewer `Vec::push` + one fewer `u32` write per entity). |
| **Batch spawn** | `spawn_batch_10k_{1,3}comp` — **parity** (the removed `active_ids.extend` bulk copy + per-row sparse write are lost in the dominant component byte-copy + archetype-row-push cost). |
| **Hot read 0%-gate** | `get_component_raw_hot/cold`, `get_component_typed`, `has_entity`, `set_component_raw` — **0% (within noise)**. These touch ONLY `entities_inland`, which X.D did not change → byte-identical paths. Apparent ±movements sign-flip on re-run (this machine's variance > any real effect). |
| `cargo clippy --all-targets -- -D warnings` | clean (the `iter_entities` body uses `.filter().map()` rather than `.then()` to satisfy `clippy::filter_map_bool_then`). |

## The trade-off (documented, honest)

`iter_entities` regressed — **larger than the architect anticipated**:

| Bench | pre-X.D | X.D | Δ |
|-------|---------|-----|---|
| `iter_entities_dense_10k` | 3.43 µs | 7.05 µs | **+105% (~2×)** |
| `iter_entities_sparse_post_churn` (100k cap / ~1k live) | 345 ns | 33.5 µs | **+9645% (~97×)** |

- The **sparse** ~97× is the *designed* O(active)→O(capacity) change (scan all slots, skip null
  sentinels). The bench is explicitly a *"documented baseline — no fail criterion."*
- The **dense** ~2× exceeded the "flat-or-better" expectation: the pre-X.D baseline walked a compact
  8 B-per-entry `active_ids` (80 KB working set) and indexed `entities_inland`; X.D streams the
  full 16 B-per-slot `entities_inland` (160 KB) with a per-element `is_null()` branch. Even
  all-live, the wider footprint + branch is ~2×.

**Verdict: accepted.** Both regressions are confined to `iter_entities`, which has **zero hot-path
callers** — real iteration goes through `Query`/archetype storage. The full-delete vs
keep-`active_ids` choice is binary: you cannot get the despawn/memory/simplicity wins *and* keep the
compact dense walk. Making a hot structural path (despawn) faster + simpler + smaller (−12 B/entity,
−2 allocs, net-removed `unsafe`, deleted invariant) at the cost of a cold inspection API nobody
hot-calls is the correct trade for a performance-oriented engine.

**Forward note**: if a hot dense-entity-enumeration consumer ever emerges (serialization,
inspector), walk archetype `entity_ids` rows — they are already dense AND co-located with the
components such a consumer would want — rather than reintroducing a global `active_ids`.

## Soundness (preserved + improved)
- **`live_count` accounting** (code-review-verified airtight): the ONLY null↔non-null transitions of
  an `entities_inland` slot are `register_*` (null→live, `+1`) and `deallocate_entity` (live→null,
  `-1`, success-path-gated). All migration/insert/remove repoints are live→live and leave
  `live_count` untouched. The field is private. A `debug_assert!(live_count > 0)` guards the
  decrement; a tripwire test asserts `live_count == count(!is_null)` after churn.
- **C1 (critic-critical)**: the decrement sits after the `is_entity_valid` early-return, so the real
  `EcsMaster::create_entity` rejection fallback (which calls `deallocate_entity` on an
  allocated-but-never-registered recycled id whose slot is NULL) is a no-op that does NOT decrement.
  Locked by `deallocate_unregistered_recycled_id_is_noop_and_preserves_live_count`.
- **Generation survival** + recycle semantics: unchanged (the gen-bump-then-null write is verbatim).
- **Lockstep growth (D3)**: `entities_inland` and `sparse_to_active` grew in exact lockstep, so the
  three external capacity guards (`sparse_to_active.len()` → delete the redundant block; the
  `entities_inland` block already guards) are behavior-identical.

## Pipeline notes
- Architect chose **full-delete** over the middle-ground (tombstone log) and keep-as-is, with the
  zero-hot-caller finding + archetype-redundancy as justification.
- Critic returned **REVISE** with 1 CRITICAL (C1 decrement placement + uncovered test) + 2 Important
  (W1 migration-path liveness-neutrality invariant + a `live_count == count(!is_null)` tripwire; W2
  init `live_count` in *both* `new` and `with_capacity` literals). All folded into the dev brief.
- Code-review **APPROVED** (traced all 5 mutation sites + every live→live repoint; build/clippy
  green on a forced rebuild). Three MINOR stale-comment items; the two in-crate ones fixed.

## Files
- `crates/boyko_ecs/src/ecs/core/entity/entity_master.rs` — struct (4 fields), ~10 method bodies,
  SEND5 comment, struct doc; 6 new tests.
- `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` — deleted 2 `sparse_to_active` resize
  guard blocks (+ 2 stale-comment fixes).
- `crates/boyko_ecs/src/ecs/core/commands/spawn_at_command.rs` — deleted 1 resize guard block + comment.
- `crates/boyko_ecs/src/ecs/core/commands/spawn_batch_command.rs` — comment-only.
- `crates/boyko_ecs/benches/random_access.rs` — 2 bench comments updated (bodies unchanged) +
  tester-added `bench_delete_entity_10k` (public-API, gives despawn perf coverage going forward).
- `crates/boyko_ecs/tests/phase12_6_lazy_alloc.rs` — comment-only.
- Internal-catalog sync: `docs/SYSTEMS.md` (§4.3 struct/API + the SparseMap attribution),
  `docs/FEATURE_MAP.md`, `docs/ARCHITECTURE.md`, `docs/PHASE-13-ROADMAP.md`.

## Follow-up
- O3 (pre-existing, out-of-crate): `crates/bench_bevy_vs_boyko/benches/profile_spawn_v2.rs` has a
  stale `sparse_to_active` memory-model comment (already obsolete before X.D) — fold into a future
  bench-comment sweep.
- Historical phase docs (PHASE-7/11/12.x plans, AUDIT, diagnosis) intentionally **not** edited —
  they are immutable point-in-time records.
