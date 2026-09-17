# Unified system plan — 02 Order of work (rev 5)

Tree tags (`[J]`, `[M]`, `[G]`) and the citation verification record are in
[00 Overview](UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md). Bare document names below are `[M]` documents.

## 1. Sequencing rules

1. **Bugs before features.** Phase A comes first.
2. **Gates before code.** Phase B: a count that must only shrink is pinned before anything shrinks
   it.
3. **Kernel before subsystems.** A subsystem rung starts only when every KC it names has landed on
   the trunk (owner order, `RUNTIME-DATA-LEDGER.md:1779`).
4. **One trunk (U-13).** Every rung branches from `integ/unified` and merges back green. Each
   worktree has exactly one open rung, and the code worktrees are a fixed pool of three (§4.1). At
   most three code rungs are therefore open at once, whatever lane they belong to.
5. **Rungs are small and independently green.** No rung is XL. The L rungs carry their size in §2,
   and physics U2's XL scope is split into three rungs, D-S3(i)–(iii). This is the Bevy #20934
   lesson.
6. **Refactor rule (U-12).** Split a file before the first rung that adds to it. Never split a
   file a scheduled rung deletes or rewrites; split it after. The one exception is D-M0 (Phase C),
   stated with its reason there. Locks are per rung and per file (§4).
7. **Timings run only in quiet windows (MQ).** No rung waits on a timing unless the table says so.
   Rungs that change a hot loop file an MQ entry, and the entry decides whether the change is kept.
8. **Design passes before the code that rests on them.** A rung whose design item no critic pass
   has confirmed closed waits for the pass (AP6, EP3; 00 RK-1, RK-2). A reopened item holds only
   the rungs that rest on it.
9. **Gate lists live in 03 §2.** A rung runs every gate that 03 §2 assigns to it, even where a
   gate cell below omits one. The cells below exist for rung-specific arguments: named bodies,
   pins and fixtures.

## 2. Rungs

Size bands: S ≤ 600 changed lines · M 600–2000 · L 2000–5000 · XL > 5000. "Rows" counts active
ledger rows the rung retires, or unblocks (u) when the rows migrate later.

### Phase A — stabilise

| Rung | Scope | Prereq | Rows | Red-first tests | Gates | Size | Lock set |
|---|---|---|---|---|---|---|---|
| A0 | Review `claude/trusting-ramanujan-0f8927` (04 §1). `6a9871bb` (event-lane width hazard + worker-panic propagation) is checked against A2's fix and the event-lane code; `867dd734` (`docs/OPEN-QUESTIONS.md`) is read. Each commit is merged into the lane it overlaps (A2, for the panic half) or ruled obsolete with a written reason | — | 0 | the commit's own tests, if it is kept | UG-01 | S | none (a review); a merge takes the target rung's lock |
| A1 | Physics CK-A4 (support removed → sleeper hangs; pose record) + CK-A5 (apply writes another body when an entity has `RigidBody` without `Collider`) + F1 (`[J]crates/boyko_physics/src/row_identity.rs:278` `#[cfg(not(test))]`) | — | 0 (interim rows are deleted in U5–U7) | `support_loss_wakes_sleepers.rs`, `apply_row_alignment.rs` (untracked in `[J]`, [G]:362-363) | UG-01, UG-03, UG-18 | M | `crates/boyko_physics/**` |
| A2 | CK-A6 worker-panic propagation + `EntityReservoir` loom model (design rev 3 has one open blocker: `ApplyDrainGuard` off-by-one) | A0 (the panic half of `6a9871bb`) | 0 | `a6_panic_propagation.rs`, `a6_schedule_panic_propagation.rs` ([G]:365-366) | UG-01, UG-08, UG-09 | M | `boyko_threadpool/**`, `ecs/core/schedule/**`, `ecs/core/entity/entity_reservoir.rs`, `boyko_app/src/runner.rs` |
| A3 | `fix/ke13-ke14`: 1 commit, 25 `.rs` ([G]:400). Re-test first: a regression in the observer-flag seed was recorded on 2026-09-10. | — | 0 | its own | UG-01, UG-17 | S | `archetype*.rs`, `migration_helpers.rs`, `query.rs`, `component_pool.rs` |
| A4a | Inherited red: `check_hotpath_exceptions.py` at `[J]…/entity/entity_reservoir.rs:405` ([G]:835). It is keyed on (file, allow count), and A2's loom model edits the same file first | A1, A2 | 0 | the checker's own red | UG-01, UG-18 | S | `entity/entity_reservoir.rs` |
| A4b | Inherited red: the `QueryTypeId` exhaustion test race (red 1 run in 11). Read the owner's committed `query_type_registry.rs` first; it may already carry the fix | O1 | 0 | a deterministic reproduction of the race | UG-01, UG-18 | S | `iters/query/query_type_registry.rs` |
| A5 | Small lane merges (04 §2 order) | A1–A3, A4a | 0 | per lane | UG-01, UG-10, UG-12 (device steps are the owner's) | S each | per lane |
| A6 | Merge `feat/reflection` (20 commits, 82 `.rs`): KC-21 | A3 | 24 reflect-lane rows re-pointed | lane gates `g13b`, `g17` | UG-01, UG-10, UG-17 | L | `migration_helpers.rs`, `boyko_macros/src/component.rs`, `tags.rs` |
| A7 | Merge `feat/ui-advanced` (16 commits, 69 `.rs`) = engine UI0, plus the `EntityId`-recycling reap stress test (X-11: EM2′ now recycles ids) | A2 | 196 ui-lane rows re-pointed | reap never removes the wrong entity | UG-01, UG-10, UG-12 | L | 4 crates ([G]:398) |
| O1 / O2 / O3 | Owner steps (04 §3) | — | — | — | — | — | — |
| A8 | Cut the trunk `integ/unified`. **A code merge, not a docs merge.** After O1, main carries the owner's 14 modified `.rs` files (00 RK-7). They overlap A3 (`query.rs`) and A4b (`query_type_registry.rs`), and they touch `boyko_app`, `boyko_render`, `boyko_rhi` and `boyko_rhi_vulkan` — and also `boyko_physics` (`lib.rs`, `plugin.rs`: A1's crate), `boyko_log` (`codes.rs`) and D-S4's `iters/query/{par_chunk,query_view}.rs` (writer check, `git status` in `[M]`, 2026-09-17; 00 §9 V-26). Code conflicts are resolved by hand; UNION applies to diary docs only (04 §2) | A0, A1–A3, A4a, A4b, A5–A7, O1, O2 | — | the merged tree's full suite | UG-01 full, UG-10, UG-11, UG-12, UG-17, UG-18 | M | the whole tree |

### Phase B — gates on the trunk

| Rung | Scope | Prereq | Rows | Red-first tests | Gates | Size |
|---|---|---|---|---|---|---|
| B1 | Ledger rev 5 (three trees → trunk) + UG-02 | A8 | pins 2357 active (after the rev-5 recount) | P6's seven hole fixtures; an extra site → red; a missing site → red; an empty scan dir → red on the floor | UG-02 | M |
| B2 | G-LOOP, G-GRAPH (a)(b), G-RES, UG-03 re-pinned on the trunk (merges add systems), UG-16 baseline, AL:M-A13 / MD:M-K3 id census | A8 | — | add an exclusive system → G-GRAPH red; add a runner world write → G-LOOP red | UG-03, 13, 14, 16, 20 | M |
| B3 | UG-15 pin capture on the trunk. This includes leg (7)'s section sizes and symbol multiset, and the sensitivity map's file, layout and containment maps (03 §6). **Profiles:** three are added to the root `Cargo.toml`: `bench-shipped`, `seam-census` and `sensitivity-map` (03 §5, §6). **Probe build:** the `<anon-data>` probe records which anonymous-data forms the gate host's linked PE shows (03 §6, normalisation). **Leg (1)'s macro half:** a `proc_macro2` twin for every `boyko_macros` entry point, the parsers' `const` key table, and the expanded-corpus test. **Other items:** P6-1 reproducibility (two clean `profile.release` builds, disassembly diff); the UG-08 two-leg recipe; the ignore-reason prefix migration (closed vocabulary). **The touch set adds `boyko_macros/src/**`;** B3 precedes RF-K2 (§3) | A8 | — | UG-15 controls, each in its own branch: (i)–(viii) must be red and (ix)–(x) green (03 §6) | UG-08, 11, 15 | M |
| B4 | Physics R0's census (`PHYSICS-ECS-UNIFICATION-DESIGN.md:789`). Take `tests/physics_vec_side_store_census.rs` from `ad0ebea4` (`D:/wt/ecsnative`), re-derive its pin on the trunk, and record the delta from 34 row by row. U6's "Vec 34 → 30" and S0's "census 30 → 0" are re-read as −4 and → 0 from the B4 pin. R0's timing is MQ-01; its SP-1 is MQ-02 | A8 | — | an extra `Vec` side store → red; a removed one → red until the pin is lowered | UG-01, UG-18 | S |

**Phase B worktrees and lock sets (critic pass 4, C2).** B1–B4 are code rungs and run in the code
pool, which opens at A8 (§4.1). B3, B1 and B2 take its three worktrees in that order; B4 takes the
first one to go idle. Lock sets:
- **B1:** `docs/memory/{RUNTIME-DATA-LEDGER.md, runtime-data-ledger.tsv, ledger/**}` (§4.5's data
  exception), the UG-02 scanner test (new) and its fixtures.
- **B2:** `crates/boyko_physics/tests/alloc_frame_census.rs`, and the new G-LOOP, G-GRAPH, G-RES and
  id-census tests.
- **B3:** the root `Cargo.toml`, `crates/boyko_macros/src/**`, the UG-15 gate (new),
  `tests/ignore_reasons_census.rs`, and every `#[ignore]` site that the prefix migration edits.
- **B4:** `tests/physics_vec_side_store_census.rs` (new, taken from `ad0ebea4`).

The four sets are disjoint.

### Document steps (no code; after this plan is approved; parallel with Phase A)

| Step | Content | Prereq | Must close before |
|---|---|---|---|
| DOC-1 | Allocator rev 2.5 (00 §5, first row) | plan approval | AP6 |
| AP6 | Allocator critique pass 6. Scope: Rev 2.4 (P38–P45) and Rev 2.5 together | DOC-1 | C1 (Rev 2.5); D-S1(i) (P39, P40); D-S2 (P43); UG-15 leg (6) as a gate (P44) |
| DOC-2 | Engine rev 4 and physics erratum E2 (00 §5) | plan approval | EP3 |
| EP3 | Engine critique pass 3. Scope: the rev-3 patch, rev 4 and erratum E2 | DOC-2 | D-S3(iii); AS2; every engine-sourced D-E rung |

### Phase C — memory crate and refactor wave K

| Rung | Scope | Prereq | Rows | Red-first tests | Gates | Size | Lock set |
|---|---|---|---|---|---|---|---|
| D-M0 | KC-02, packing S0–S4 (`[J]docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md:458-542` and its pin ledger). Citations are re-derived once, at the cut, against the trunk (A3 moved lines in `component_pool.rs`). **Before C1 and RF-K1, by exception to U-12:** the plan's steps and pin ledger are line-cited into `component_pool.rs`, `constants.rs`, `vm.rs` and `vm_column.rs`, which C1 and RF-K1 move. Landing first means one re-derivation instead of three. Its own delta in those files is the commit quantum, `commit_page_region`, ten proof asserts and `committed_bytes()`; RF-K1 then moves that delta as a pure move | B1–B3 | with D-M1, unblocks every per-instance column form of R2 | packing oracles red before the fix: G1 (model) at 3,145,728 B for 16 columns, G3 (OS) at 393,216 B per pool (`:467-478`) | UG-01, 02, 03, 08, 15 (attributed: `ComponentPool::new`, `grow_rows`), 20 | M | `memory/{component_pool,vm,vm_column}.rs`, `ecs/constants.rs`, `core/log/ring.rs` |
| C1 | KC-01: move `vm.rs`, `vm_column.rs`, `utils.rs` and the granule constants to `boyko_memory`; re-exports; `raw::commit_at<O>`; UG-04 counter | D-M0; AP6 has closed Rev 2.5 | u: KF-32 (9) | a commit from a system body → `commit_delta` red; setup window must be non-zero | UG-01, 04, 10, 15 | M | `ecs/memory/{vm,vm_column,utils}.rs`, `constants.rs` |
| RF-K1 | Split `component_pool.rs` (4106), `block.rs` (2166), `scope.rs` (2354) | C1, A2 | — | — | UG-01, 10, 15, 17, 18 | L | those files |
| RF-K2 | Split `component_registry/mod.rs` (1789), `filter.rs` (3343), `migration_helpers.rs` (2662), `archetype.rs` (2574), `archetype_master.rs` (1617), `boyko_macros/src/component.rs` (2067) | A3, A6 | — | — | as RF-K1 | L | those files |
| RF-K3 | Split `schedule.rs` (2536), `schedule_builder.rs` (2053), `ecs_master.rs` (1918), `query.rs` (1554), `state.rs` (2243), `iter.rs` (1826), `command_queue.rs` (1589), `profiling/{store,tests}.rs` | A2, O1 | — | — | as RF-K1 | L | those files |

Line counts are [G] §1.1's. `wc -l` in `[J]` agrees except for `archetype.rs` (2573) and
`ecs_master.rs` (1917). `filter.rs` and `state.rs` are the files under `iters/query/`.

`asset/assets.rs` (2634) is **not** split. AS2–AS5 retire `Assets<T>`.

### Phase D — kernel contract

**Lane MEM**

| Rung | Scope | Prereq | Rows | Red-first tests | Gates | Size |
|---|---|---|---|---|---|---|
| D-M1 | KC-03 + KC-18 stacks and encodings; KC-18 tables are `VmColumn<T, TableOwner>` | RF-K1, RF-K3 (`QueryStateCache` lives in `ecs_master.rs`, `[J]…/ecs_master.rs:339`) | retires ≈10 R2 rows (`DenseStore.free`, `EntitySlotMap.slots`, `LiveBitmap.words`, `ArchetypeBundle.{free_slots, id_to_slot}`, `SparseMap` ×3, KF-35, KF-42 ×4); with D-M0, unblocks all R2 (`RUNTIME-DATA-LEDGER.md:1828`) | LIFO and `slot+1` equivalence properties (allocator §5.2); a KC-18 table built with the default owner → UG-04's `Table` setup count reads 0 → red | UG-01, 02, 03, 04, 08, 15 (attributed), 20 | L |
| D-M2 | KC-05 + KC-06a (allocator 1a + 1b as one commit, then 1f) | D-M1, A2 | KF-33 (4) + "ScopeShared placement" (10) = 14 (R5) | C2 Miri `miri_scope_slot_identity.rs` (`ALLOCATOR-DESIGN-SPACE.md:892-896`); P3 poison-write protector gate re-blessed (zero `KE16-PROTECTOR-GATE-ARMED` lines); mem-shake proven to fire; exhaustion degrades (`D = 1`) | UG-01; UG-03 (App scenes scope/chunk/OTHER → 0; AL:M-A10 `chunk_bytes_resident` pinned); UG-04; UG-08; UG-09 | M |
| D-M3 | KC-06b injector ring | D-M2 | KF-34 (4) | loom: push/steal, steal/steal, overflow-spin with park (`LANE_CAP = 2`) | UG-03 (injector → 0, spin counter 0); **UG-05 lands** (first scene at 0); UG-09 | M |
| D-M4 | KC-07 pool ownership | D-M3, RF-K3 | KF-31 (19) | `Schedule` holds no `Arc` (compile); teardown-order test | UG-01, 02, 15 (`size_of` re-bless recorded) | M |
| D-M5 | KC-08 gang + `par_range` (kernel halves of physics P1 and P2) | D-M4 | 0 (KF-21) | board loom model; Miri gang-exit protector gate; abort paths (a)–(f); phase-overlap detector with its mutation (`PHYSICS-ECS-UNIFICATION-DESIGN.md:3591-3602`); NB3 release assert; NB4 trybuild | UG-01, 08, 09, 15 (attributed: mode table), 17 | L |
| D-M6 | KC-04 thread context, per 01 §6. **Touch set:** `boyko_threadpool/src/{thread_ctx.rs (new), tls.rs, worker.rs, thread_pool.rs, lib.rs}`; `boyko_threadpool/tests/{thread_ctx_lifecycle.rs (new), loom_thread_ctx.rs (new), tls_lane_merge.rs}` (D7's source-shape row now counts `current()` calls); `boyko_diag/src/lane.rs`; `boyko_log/src/{drain_owner, sync_out, codes}.rs` (`codes.rs` gains the two refusal codes); `boyko_ecs/src/ecs/core/{thread_fields.rs (new), mod.rs, component/hooks/scope.rs, component/observers/propagate.rs, hierarchy/commands.rs, relationship/mod.rs, component/component_registry/required.rs}`; `boyko_ecs/tests/required_plan_reentry.rs` (new); root `tests/thread_ctx_census.rs` (new). **Split rule:** if the cut estimates more than 2000 changed lines, the rung splits into D-M6a (threadpool, diag, log: the statics, arms, pool rows, `LANE`, tokens, lifecycle and loom tests) and D-M6b (the ECS rows and the plan-build set), in that order, each green on its own | D-M4, RF-L. MQ-13 runs after the merge and does not hold it (03 §5) | KF-45: 12 rows → 4 key/guard cells | the D-M6 block below | UG-01, 04, 08 (03 §3), 09, 15 attributed (the `Schedule::run` dispatch, including `install`'s record read, and the `swap_remove/10k` body; mode table; leg (1) stays 0 with no allowlist entry), 16 (records `.bss`'s +525,312 B virtual size beside the `.text` / `.rodata` budget of 00 §2), 18 (the new census), 20 | M |

**Lane STORE**

| Rung | Scope | Prereq | Rows | Red-first tests | Gates | Size |
|---|---|---|---|---|---|---|
| D-S1(i) | KC-19a, the registry bug fixes (01 §2) | RF-K2; AP6 has closed P39 and P40 | 0; fixes defect 32.2 | G-MINT-1..4 + the row-8 kind race (`ALLOCATOR-DESIGN-SPACE.md:3663-3698`); `install_dense_storage_kind::<D>(another type's id)` panics in release | UG-01; UG-10 (`tags.rs` waived anchors re-blessed explicitly, P38.4); UG-19; **UG-15 attributed:** pins may move only in `try_register_dynamic`, `register_new::<Transform>` and `register_layout`, each with its P-number in the commit message; any other move is red | S |
| D-S1(ii) | KC-19b, the modding Stage-1 seam (01 §2; MS-02, MS-03) | D-S1(i) | 0 | G-MINT rows for sized names; a sized name re-minted with a different layout → `Err` | UG-01, UG-10 (`tags.rs` again), UG-19; **UG-15 strict, parent = its cut commit**, which contains D-S1(i) and may also contain D-S3(i) or D-E1 (first cut wins on the registry files, §4.3). Legs (1)–(5) and (7) must be identical; leg (6) is N/A until a modding crate exists. Red controls (i)–(viii) must each be red in their own branch (03 §6). Leg (7) is the check that no KC-19b item survives in the linked `boyko_demo`: `id_space_census()` is reachable from engine code from D-S1(i) on, so a KC-19b item that it comes to call would show up there. If a leg moves, the moving item leaves the kernel for `boyko_mod_host` (modding R1, `MODDING-DESIGN-SPACE.md:1818-1823`) and D-S1(ii) re-lands without it | S |
| D-S2 | KC-10's columns, including `new_untracked_raw`'s `drop_fn` parameter and the lazy state (01 KC-10), = physics U1 (the `WorldScratch` half lands in D-R2a). **Touch set:** `memory/component_pool.rs`; `boyko_memory/src/vm.rs` (`VmReservation::UNRESERVED`); `component/scratch/**`; and every user of the band or of `ScratchColumn::new`: `boyko_physics/src/{scratch_ids,row_identity,resources}.rs`, `solver/{warm_start,soft_step,colored,colored_tests}.rs`, `narrowphase/axis_cache.rs`, `soft/{coupling,colored}.rs`, and `boyko_render/src/{mesh_draw,particle_system,particle,light_system}.rs`. Ripgrep for `ScratchColumn::new\|scratch_ids::\|SCRATCH_ID_\|broadphase_column_id\|SCRATCH_REGION` in `[J]/crates` finds 281 hits (matching lines) in 17 files, three of them in `boyko_ecs`: `memory/component_pool.rs` and the tests `crates/boyko_ecs/tests/scratch_column{,_miri}.rs`, which this touch set does not name (writer check, 00 §9 V-30) | D-S1(i), D-M0, RF-K1; AP6 has closed P43 | u: KF-01 (324), KF-05 (15); deletes the band | staggers pairwise distinct mod 64 within a cohort and consecutive for same-type singles; `NEXT_ID` unchanged across 1000 cohort and 1000 `for_type` constructions; `size_of::<ComponentPool>()` still 128 / 144; render lane microbench recorded; a `Default` column makes no reservation syscall before its first push, and its first push reserves exactly once | UG-01, 02, 03, 04, 12, 15 (attributed: `ComponentPool::new`, `grow_rows`; leg (3) `ComponentPool` must not move), 19; MQ-03 filed, **not blocking** (03 §5) | L |
| D-S3(i) | KC-12's kind: `StorageKind::Group`, the exhaustive matches, the decoder, and `create_archetype` refusing `Group`. **Touch set:** the files holding an exhaustive `match` on `StorageKind`, as the compiler reports them when the variant is added (24 non-test files reference the enum at `d552be05`: ripgrep `StorageKind::` outside `tests/`), plus `boyko_macros/src/component.rs`. *Writer check (00 §9 V-38): 24 holds for `[J]crates` with every `tests/` directory excluded. The 24 are 21 `boyko_ecs` files plus `boyko_macros/src/component.rs` (already one of them), `boyko_render/src/occlusion_marker.rs` and `boyko_serialize/src/load.rs`, so the touch set crosses into render and serialize* | D-S2, RF-K3 | 0 | `ALL_STORAGE_KINDS` covers `Group`; a `Group` id in `create_archetype` panics; the decoder round-trips every kind | UG-01, 02, 03, 04, 15 (attributed: the five dispensers, `register_new::<Transform>`), 17, 19 | M |
| D-S3(ii) | KC-12's store: the group store (with the `release` byte), DEAD, binder, `BindToken`, `GroupSlot` / `GroupRef`, `GroupHead` / `GroupTail`, and the debug reconciliation at the end of `apply_window_drain`. **Touch set:** `ecs_master/{entity_api, ecs_master, binder (new)}.rs`, `archetype/archetype.rs` (`anchor_mask`), `iters/query/**` (new terms), `schedule/schedule.rs` (the end of `apply_window_drain`: physics cites `:841` from `d11962a9`, `PHYSICS-ECS-UNIFICATION-DESIGN.md:2043`; it is `[J]…/schedule.rs:859` at `d552be05`, 00 §9 V-25), `system/params/**` (`:3049`), `dense/dense_registry.rs`, `entity/{entity_master, inland_store}.rs`, `boyko_macros` (`#[dense_group]`) | D-S3(i), D-E0 (fixed order #2) | u: KF-19 (14) | the U2 items for the store, binder and slots (`PHYSICS-ECS-UNIFICATION-DESIGN.md:2981` + `:3583-3587`), with Erratum E1; `clear()` on an `Immediate` group DEAD-fills every slot; NB5 test tokens; **D-E0's test gains its group leg:** each spawn inserts the test group's anchor, and the `(system index, spawn ordinal) → group slot` sequence is identical across runs and equal to W = 1 | UG-01, 02, 03, 04, 08 (TB), 15 (attributed: the `swap_remove/10k` body; leg (3) `EcsMaster`; rev 2's "despawn body" is not a leg-(2) pin, so it is not named), 17, 19 | L |
| D-S3(iii) | KC-12's chain and KC-13(a): chain, ChainKey, `Release::Stamped`, `died`, `release_dying_before`, and the `clear()` stamping rule. **Touch set:** the `dense/**` group-store files, `ecs_master/ecs_master.rs` (`clear`), `boyko_macros` (ChainKey emission), `system/params/**` (the `GroupHead` methods) | D-S3(ii); EP3 closed | u: KF-49 (4) | the U2 items for the chain; X-17 `clear()` red-first; the Stamped proptest (never released before the horizon, and never by removal, the first-op flush or `clear()`); `release_dying_before` on a `Chained` group fails to compile (UG-17, E0271) | UG-01, 02, 03, 04, 08 (TB), 15 (strict), 17, 19 | M |
| D-S4 | KC-14 = physics U3 (touch set: `iters/query/{par_chunk,query_view}.rs` and the par drivers) | D-S3(iii), RF-K3 | u: KF-20 (6) | mixed `par_iter` ≡ `iter` multiset; `IsEnabled` + `GroupSlot` in one query | UG-01, 17 | M |
| D-S5 | KC-15 + span-typed group columns | D-S3(iii) | u: KF-03 (40) | proptest against a Vec model (frontier bounded by live spans); copies ≤ 2n; Miri `SpanRef` read across a sibling relocation; spans freed before DEAD at all 5 release points | UG-01, 08, 15 (attributed: mode table) | L |
| D-S6 | KC-16, plus the 7 KF-02 rows in `boyko_ecs` and `boyko_render`. **Touch set:** `component/scratch/**` (the typed wrapper), `component/component_pool_bundle.rs`, `component/dense/dense_registry.rs`, `component/enable/enable_store.rs`, `ecs_master/ecs_master.rs`, `asset/staging.rs`, `boyko_render/src/retired_gpu_buffers.rs` | D-S2 (the `drop_fn` constructor), RF-K3 (`ecs_master.rs`) | KF-02 (7); the rhi row goes to D-E15, the `sparse_map` row to D-R2d | drop counts on truncate / `take_at` / clear / drop; `NEXT_ID` unchanged across 1000 `OwnedColumn` constructions (0 ids) | UG-01, 02, 04, 08, 12 (the render row), 15 (attributed: mode table) | M |
| D-S7 | KC-17 + byte users (allocator 1c, 1e) | D-M1, RF-K3 | KF-07 (6), KF-06 (12), `TermList` (7) | panic-recovery re-absorb without realloc (O7); resource record reuse; drop count | UG-01, 03, 08 | M |

**D-S3's three rungs hold their locks separately.**
- Each has its own branch (`u/D-S3i`, `u/D-S3ii`, `u/D-S3iii`) and its own touch set.
- None is cut before its predecessor has merged, and (iii) is not cut before EP3 closes. No D-S3
  lock is therefore held while a design pass is pending.
- Every U2 item lands in exactly one of (ii) and (iii); each commit message carries the checklist.

**Lane ENG** — engine rev 4 order, remapped. Every rung sourced from the engine design starts only
after EP3 has closed (RK-2). D-E0 (physics P-§14), D-E18 and D-E19 (ledger KF-10, KF-09) have other
sources and start as soon as their prerequisites land.

| Rung | Scope | Prereq | Rows | Size |
|---|---|---|---|---|
| D-E0 | KC-36 deterministic in-window apply order (EM2′-K test mandatory, RK-10) | RF-K3 | 0 (defect X-14) | S |
| D-E1 | KC-20 (K-EK22 gate) | RF-K2, EP3 | u: generic users | S |
| D-E2 | KC-29a/b (EK15a, EK15b + the despawn redirect, placed per §4.4) | D-E1, D-S3(iii) | KF-48 (1) | M |
| D-E3 | KC-22 | RF-K3, EP3 | u: KF-44 (21) | S |
| D-E4 | KC-24 | D-E3 | — | S |
| D-E5 | KC-23 structural log | D-E4 | u: KF-25 (3) | M |
| D-E6 | KC-25 | D-E5 | u: KF-17 (8) | S |
| D-E7 | KC-23 EK6 + EK6g (K-EK6g gate) | D-E6, D-S3(iii) | — | M |
| D-E8 | KC-26 | D-E7 | u: KF-23 (5) | M |
| D-E9 | KC-27 + KC-13(b): the teardown driver, `TeardownToken`, `release_dense_group_at_teardown` (EM2′-K test mandatory) | D-E8, D-M4, D-S3(iii) | KF-29 (2), KF-30 (1) | M |
| D-E10 | KC-29c (K-EK15c gate) | D-S5, D-E2 | u: KF-11 (4) | L |
| D-E11 | KC-28 (MQ-14 filed) | D-E9, D-S7 | KF-13 (8), KF-14 (26), KF-15 (5), KF-12 (18) as edge entities | L |
| D-E12 | KC-30a (EK19, KF-08, KF-04) | D-E11 | KF-08 (9), KF-04 (10) | M |
| D-E13 | KC-31 | D-E12 | KF-26 (4), KF-43 (5) | M |
| D-E14 | KC-33 | D-E9 | KF-27 (12), KF-28 (6), KF-39 (1) | M |
| D-E15 | KC-34 KF-36 edge; then KF-02's `boyko_rhi_vulkan/src/memory.rs:728` row onto KC-16 | D-E14, D-S6 | u: KF-36 (44); KF-02 (1) | M |
| D-E16 | KC-34 EK16 | D-E15 | KF-40 (2) | S |
| D-E17 | KC-32 | D-E13 | — | M |
| D-E18 | KC-30b structured asset error; every construction site migrates in the same commit | B1 (its 77 rows retire, so UG-02 must be pinned first; rule 2); RF-L (D-E18 adds codes to `boyko_log/src/codes.rs`, U-12) | KF-10 (77) | M |
| D-E19 | KC-30c loader decode context (kernel half + loader signatures) | D-E18, D-S2 | KF-09: kernel rows and the loader rows the signature forces; the rest unblocked for F1 | M |

Gates for every D-E rung: UG-01, UG-02, UG-10, UG-17, and UG-15 in the mode the table below gives,
plus the rung-specific gates of engine §17 / P-§17. UG-12 on D-E18 and D-E19.

**UG-15 mode per rung (critic pass 2, O2; critic pass 3, W1; critic pass 4, W3).**
- **Default.** A rung runs **attributed** only if it names, before its cut, the leg-(2) bodies and
  the leg-(3) types it may move. Every other rung runs **strict**.
- **What a rung must consider is decided mechanically,** by 03 §6's sensitivity map (file map,
  layout map, containment map). A rung must deal with:
  - every body whose file map meets its touch set;
  - every leg-(3) type whose containment map holds a type whose definition the rung's diff changes,
    together with every body in that type's layout map;
  - every body in the layout map of a leg-(3) type the rung names.

  For each, the rung either names it below, or argues in its entry, at its cut, why its diff leaves
  the bytes unchanged.
- **Naming.** A body is named here when the plan already knows it will move. A rung may add a name
  at its cut, with the reason, in its entry.
- **Red.** A move of a body or type that is neither named nor argued is red.
- **Rename lists.** A rung that moves files declares one (03 §6, normalisation):
  - every RF commit (its move map);
  - C1 (the map `boyko_ecs::ecs::memory::{vm, vm_column, utils}` → `boyko_memory::…`).

  D-S1(ii) declares `TAG_NAMES → DYN_NAMES`.
- **R2 sweeps and the `swap_remove/10k` body** (00 §9 V-49; critic pass 4, O5).
  - No ledger row lives in `entity_api.rs` (a ripgrep of `[M]docs/memory/runtime-data-ledger.tsv`
    returns 0 hits), so no R2 sweep edits `delete_entity_core` itself.
  - That body also calls into `ecs_master.rs` (5 TSV rows) and `command_queue.rs` (3 rows, one of
    them `CommandQueue::bytes`).
  - A sweep can therefore reach it through the file map, as well as through `EcsMaster`'s layout and
    containment maps. The rules above cover all three routes.

| Rung | Leg (2) bodies it may move | Leg (3) types it may move |
|---|---|---|
| D-M0 | `ComponentPool::new`, `grow_rows`. The `swap_remove/10k` body is argued at the cut: D-M0's delta on that path is `debug_assert!`-only (01 §7, KC-02) | — |
| C1 | `ComponentPool::new`, `grow_rows` (UG-04's relaxed counter RMW per commit, KC-01), under C1's rename list. The `swap_remove/10k` body is argued at the cut: on that path C1 only moves files, which the rename list and the `<anon-data>` placeholder normalise (03 §6) | — |
| D-M1 | the `swap_remove/10k` body (archetype `VmColumn` pushes and swaps); `add_system::<F, M>` (schedule tables); every body in `EcsMaster`'s layout map. `QueryStateCache.slots` becomes a `VmColumn` (KC-18), and `QueryStateCache` sits by value inside `EcsMaster` (`[J]…/ecs_master.rs:296`, `:339-341`) | `EcsMaster` |
| D-M2 | `Schedule::run` dispatch (the scope chunk source), and `Scope`'s layout map | `Scope` |
| D-M3 | `Schedule::run` dispatch (the injector push) | — |
| D-M4 | `Schedule::run` dispatch (no `Arc`), and `Scope`'s layout map | `Scope` |
| D-M5 | `Schedule::run` dispatch: `LaneBoard` enters `PoolInner`, whose field displacements the `install` path reads | — |
| D-M6 | `Schedule::run` dispatch (`DeferredScopeGuard` inside `apply_window_drain`; the pool part and the record read inside `install`). The `swap_remove/10k` body: `DeferredScopeGuard` in `delete_entity_core` (`[J]…/ecs_master/entity_api.rs:984`, `:1110`), and the depth read and guard in `drain_deferred_hook_queue` (`[J]…/ecs_master/ecs_master.rs:668`, `:690`), which `delete_entity` calls (`entity_api.rs:815-820`) | — |
| D-S1(i) | `try_register_dynamic`, `register_new::<Transform>`, `register_layout` | — |
| D-S2 | `ComponentPool::new`, `grow_rows` | — (`ComponentPool` must not move) |
| D-S3(i) | the five dispensers, `register_new::<Transform>` | — |
| D-S3(ii) | the `swap_remove/10k` body, and `EcsMaster`'s layout map | `EcsMaster` |
| D-S5 | the `swap_remove/10k` body: span frees inside `unbind` (`binder.rs`), which D-S3(ii) puts on `delete_entity_core`'s path (§4.4 step 6) | — |
| D-S6 | every body in `EcsMaster`'s layout map: the KF-02 row at `[J]…/ecs_master.rs:953` moves per-query state into an owning column that the world holds | `EcsMaster` |
| D-S7 | `Schedule::run` dispatch (the `ErasedSystem` handle); `add_system::<F, M>`; the `swap_remove/10k` body, where the drain calls the `#[inline]` `CommandQueue::is_empty` at `[J]…/ecs_master.rs:714` (the file map says whether it is inlined); every body in `EcsMaster`'s layout map, because `CommandQueue.bytes` becomes a `ByteColumn` and `CommandQueue` sits by value in `EcsMaster` (`:264`) | `EcsMaster` |
| D-E0 | `Schedule::run` dispatch | — |
| D-E2 | the `swap_remove/10k` body: the redirect inside the `!flags.is_empty()` branch (`[J]…/entity_api.rs:1050`) and `delete_entity_core`'s return type (§4.4). The table-only path executes no added instruction; MQ-20 records the timing | — |
| D-E11 | `add_system::<F, M>` (system entities); `query_ref_iter/10k` must **not** move (U-16) | — |
| D-R2a | `EcsMaster`'s layout map | `EcsMaster` (`WorldScratch`) |
| D-R2b..d, D-E3..E10 (except D-E2), D-E12..E19 | named at the cut by the rule above, otherwise strict | named at the cut, otherwise strict |
| D-S1(ii), D-S3(iii), D-S4, D-E1, every RF commit, every modding delta | strict | strict |

**Red-first tests for the rungs this revision adds or changes.**
- **D-E0.**
  - **Setup.** Eight mutually non-conflicting systems `s0..s7`, each with a random per-run delay.
    Each spawns 4 entities carrying `Tag { sys, ord }` and one table component. A test `on_add`
    hook for `Tag` appends `(sys, ord)` to a preallocated log resource.
  - **Frames.** Frame 0 spawns. Frame 1 despawns every entity with an even `ord`, each system
    selecting its own by payload. Frame 2 spawns again.
  - **Runs.** The three frames 200 times at W = 8, and once at W = 1.
  - **Asserted identical across all runs and equal to the W = 1 run:**
    - after each frame, the `Tag` sequence in table-row order;
    - the hook log, which must also be ascending `(sys, ord)` within each spawn frame;
    - after each frame, the **set** of live entity ids. That set is fixed: frame 2 recycles
      exactly the 16 freed ids, because every counter claims from the stack until it is empty
      (`[J]…/params/entity_counter.rs:212-218`), and it mints the next 16 fresh ids.
  - **Not asserted:** which entity id carries which `Tag` (U-20).
  - **Red-first:** today's pop order makes the row sequence and the hook log differ between runs.
    The EM2′-K test also runs. D-S3(ii) adds the group leg (§2).
- **D-E2, `counted_target_despawn_keeps_group_slot`.**
  - Setup: a test group `TG` (`#[dense_group]`, `Stamped`, anchor `TA`), and a `CountOnly`
    relation that gives entity E count 1.
  - Every despawn path (`delete_entity`, `despawn_without_children`, the hierarchy cascade,
    `Commands::despawn`) reaches `delete_entity_core`, which returns `DespawnOutcome::Deferred`.
    - The new `pub fn try_despawn` returns that outcome.
    - `delete_entity` and `despawn_without_children` keep `-> bool` and return `true`, meaning
      "the handle was live". 94 call sites in 50 files use that `bool` (ripgrep
      `\.delete_entity\(|\.despawn_without_children\(` in `[J]crates`), and changing the type would
      put all of them in D-E2's touch set. *Writer check (00 §9 V-37): the count holds as 94
      matching lines in 50 files. One hit is a doc comment
      (`boyko_demo/tests/state_exclusive_smoke.rs`). About a quarter are statement calls that
      discard the value (an approximate regex finds 23). 44 of the 50 files are under `tests/` or
      `benches/`; the other 6 hold 8 sites. Roughly 70 sites use the `bool`, so the conclusion
      stands.*
    - Afterwards `group_get` returns E's bytes, `live_count` is unchanged, the debug reconciliation
      is green, and E is alive.
  - Unlinking to 0 despawns E in the next window; its slot enters `dying` stamped with that tick
    and is not released before the horizon. An `Immediate` variant is DEAD-filled only after the
    count reaches 0.
  - **A `Pinned` target.**
    - `delete_entity` returns `true`, and `try_despawn` returns `Deferred`.
    - Inside `delete_entity_core` the row is untouched. The `Pinned` removal applies at depth 0 in
      the outermost drain, and its `on_remove` test hook records depth 0.
    - Afterwards E has no `Pinned`, and its `TG` slot is live and unchanged.
  - **Mutation:** moving the redirect below `archetype.remove_entity` (`[J]…/entity_api.rs:1087`)
    makes `group_get` return `None`, so the test goes red.
- **D-E9, trybuild fixtures (UG-17):**
  - `release_dense_group_at_teardown` without a token → E0061;
  - `TeardownToken` built outside `boyko_ecs` → private-field error;
  - a rank callback that stores its `&TeardownToken` in a resource → borrow error;
  - the teardown form on a `Chained` group → E0271, from the `Release = Stamped` bound. This is a
    type error, so the fixture holds whether trybuild checks or builds.

  With a token, every dying entry is released and the visitor runs once per slot, before the DEAD
  fill (engine P-§17, `ENGINE-RUNTIME-ECS-DESIGN.md:2824`).
- **D-E18.** A decode failure formats only at emission: a counting allocator records 0 heap calls on
  the error path.
- **D-E19.** A second decode of the same asset reuses the warm lanes (0 heap calls), and
  `Asset::Cpu: Copy`.
- **D-M6.**
  1. **Census** (`tests/thread_ctx_census.rs`).
     - Production `thread_local!` statics in the four KF-45 crates go from 12 to 4, each with its
       reason (01 §6 item 10).
     - `ext_ptr(` has exactly one call site in the workspace, in `thread_fields.rs`.
     - Red-first: add a `thread_local!` to `boyko_ecs`, or a second `ext_ptr` caller.
  2. **Loom** (UG-09): 01 §6 item 9; `running 5 tests`.
  3. **Lifecycle** (`boyko_threadpool/tests/thread_ctx_lifecycle.rs`). The binary holds exactly one
     `#[test]`, so nothing in it runs beside its phases, and cargo runs test binaries one at a time.
     The phases run in order:
     - **(a) Prepare.**
       - The first pool build publishes both indices before its first spawn (the `debug_assert!` of
         01 §6 item 4).
       - On the word arm, after booting W = 64, `CTX_INDEX_ALLOCS == 1` and
         `LANE_INDEX_ALLOCS == 1`; on a portable host both read 0, and the phase asserts that
         instead.
       - Red-first: delete `prepare()` from `build` → the debug assert trips.
     - **(b) Claims.**
       - Building and dropping a pool of W = 8 raises `THREAD_CTX_CLAIMS` by exactly 8 during
         `build` and not afterwards. After the join, the busy count equals its value before the
         build.
       - Red-first: declare `worker_main`'s `_ctx` after `_deque_deposit` → the deposit's drop runs
         after the release and claims lazily → 16.
     - **(c) Exit by destructor.** A `std::thread` resolves `current()`, writes a field, and returns
       without an explicit release. After its join, the busy count is back.
     - **(d) Zero at release (W2).**
       - A W = 1 pool is built and dropped. Its worker exits with `active_pool` and `wid` still set
         (`worker.rs:46, 59`).
       - A fresh `std::thread` then claims the lowest clear slot, which is the worker's former slot,
         since only the test thread holds a lower one. It reads
         `current_worker_id() == WORKER_ID_UNATTACHED` and `ThreadPool::current_pool().is_none()`.
       - Red-first: delete the zeroing (01 §6 item 7 step 3, the only site) → a debug build trips
         the claim's `thread record not idle at claim`; a release build reads `wid == 0`.
       - A unit test in `thread_ctx.rs`, `release_zeroes_every_field`, repeats this on a local table
         with every field set, the `ext` bytes included, against the same mutation.
     - **(e) Install keeps the record.** The test thread's `current()` returns the same reference
       before, inside and after `pool.install`, while `current_worker_id()` changes.
     - **(f) Capacity (W1).** `#[doc(hidden)] pub fn __hold_free_slots(n) -> SlotHold` claims free
       bits for no thread and releases them on drop; fat LTO drops it from the game.
       - With `FOREIGN_RESERVE + 3` slots left free, `build(8)` yields `worker_count() == 3`, and
         `POOL_WORKERS_CLAMPED` rises by 1.
       - With `FOREIGN_RESERVE` left free, `build(8)` yields 1, and `POOL_RESERVE_DIPS` rises by 1.
       - With 0 left free, `build` panics on the test thread with the pool-slot code
         (`catch_unwind`).
       - In that full state, a fresh `std::thread` reads `current() == None`,
         `current_worker_id() == WORKER_ID_UNATTACHED` and `is_in_system_run() == false`. Its
         `install` on an existing pool panics with the refusal code before `active_scopes` moves,
         and that pool afterwards drops cleanly. `THREAD_CTX_REFUSED` has risen.
       - Red-first: remove the clamp → a worker takes the lazy path and is refused.
     - **(g) DG12 leg (00 §5, erratum D0-1).** With diagnostics off and after (a):
       - `boyko_diag::lane::lanes_leaked() == 0`;
       - no spare is claimed;
       - `LANE_IDX` and `LANE_TLS_OFF` are the only `boyko_diag` statics that hold a non-zero value.
  4. **Plan build** (`boyko_ecs/tests/required_plan_reentry.rs`).
     - **The cycle case.** A hand-written `Component` A registers an `id_fn`. On its first call, the
       `id_fn` builds an `EcsMaster` and inserts an A (re-entering `get_required_plan(A)`); on later
       calls it returns B's id. The result must be a `Cycle` panic naming A.
     - **Red-first:** every outermost build uses a fresh set (the publication removed) → the nested
       build of A completes and nothing panics.
     - **The acyclic case.** A second `id_fn` re-enters with an unrelated component C; the build
       completes, and A's plan is memoized once.
  5. **Miri** (UG-08): 03 §3's D-M6 rows.
  6. **Unchanged gates:** `boyko_threadpool/tests/diag_lane.rs` (DG2, DG3) stays green without
     edits, and so does the logging zero-allocation leg (`[J]crates/boyko_diag/src/lane.rs:38-40`).
  7. **Structural.** `THREAD_BUSY` and `THREAD_RECORDS` are in `.bss` of the linked `boyko_demo`
     (`llvm-nm` class `b`/`B`; `llvm-size -A`), checked with leg (7)'s tool under `seam-census`.

**R2 sweeps** — remaining kernel rows after their features land.

| Rung | Group | Rows |
|---|---|---|
| D-R2a | ecs-storage | 103 |
| D-R2b | ecs-schedule | 146 |
| D-R2c | ecs-services | 65 |
| D-R2d | pool-utils-log 6 + macros-aether 5 + render 1 + ui-lane 5 + reflect-lane 2, plus KF-02's `sparse_map.rs:10` row onto KC-16 (needs D-S6) | 19 + 1 |

Each sweep lowers UG-02 and needs D-M1 plus the features its rows name.
- **D-R2a** needs D-S2 as well. Its first commit lands KC-10's `WorldScratch` (01 §2) with its Miri
  row, then migrates the 15 KF-05 rows.
- **Counts.** The counts above are **gross** group counts. Each sweep's net count is re-derived at
  its cut, net of the rows D-M0, D-M1, D-S6, D-S7 and the D-E rungs have already retired (the
  recount rule, §6).
- **Size.** M each; a sweep whose net diff exceeds 2000 changed lines is split by file group.

**Exit from Phase D.** Every non-conditional KC is on the trunk (the conditional ones are KC-11 and
KC-35); UG-01..UG-05 and UG-15 are green; R2 is at 0.

### Phase E — subsystems

**Physics lane** (one worktree). Rungs as physics rev 5
(`PHYSICS-ECS-UNIFICATION-DESIGN.md:785-802` + patches).

| Rung | Prereq |
|---|---|
| U4 | D-S3(iii) |
| U5a | U4 |
| U5 | U5a, D-S4 |
| U6 | U5 |
| U7 | U6 |
| P1 | D-M5 |
| P2 | D-M5 |
| E1 | D-E8 |
| S0 | D-S5 |

- **Order.** P1/P2 come before U5 only if MQ-01 credits them more (`:765-774`).
- **Retires R3** (80 rows), the interim `row_identity.rs` (7 rows,
  `RUNTIME-DATA-LEDGER.md:1880`), and defects A2 and warm-start.
- **Size.** U5 and P2 are L; the rest are M.
- **Census numbers.** U6's and S0's census numbers are read from B4's pin.

**Engine lanes** (two worktrees). Rungs as engine §12 + P-§12, prerequisites remapped:

| Rung | Prereq |
|---|---|
| IN1 | D-E8 |
| IN2 | D-S2, D-E1 |
| IN3 | A7, IN2, HO3 |
| IN4 | D-E8 |
| HO1 | D-E9 |
| HO2 | D-E9, AS1 |
| HO3 | IN1, D-E9 |
| HO4 | D-E9 |
| HO5 | HO4 |
| HO6 | D-E9 |
| RE1 | — (Q4 decided) |
| AS1 | — |
| RE2 | AS1 |
| RE4 | D-E4, HO4 |
| RE5 | D-E5, D-E6 |
| RE6 | AS3, D-E7 |
| RE7 | D-S6 |
| RE8 | D-S2 |
| RE9 | U5, D-E5 |
| AS2 | D-S3(iii), D-E7, D-E9, EP3 |
| AS3 | AS2 |
| AS4 | D-E2, D-E1 |
| AS5 | AS4 |
| UI2 | A7, HO3 |
| UI3 | D-E2, D-S2 |
| UI4 | D-E8 |
| UI5 | D-E3, D-E7 |
| UI6 | D-E5 |
| UI6s | A7 |
| UI7 | AS5, UI6s |
| UI8 | D-E5, HO4, RE4 |
| UI10 | D-E17, D-S2 |
| SC1 | D-E5, D-S2 |
| SC2 | — |
| SC3 | — |
| SC4 | D-E13 |
| SC5 | D-E12 |
| SC6 | D-E11 |
| LG1 | — |
| LG2 | — (Q4 decided) |

These rungs retire R4 (156 rows).

### Phase F — tail

| Rung | Scope |
|---|---|
| F1 | R6 (648 rows): diagnostics form, KF-09 / KF-10, B and X rows, by group |
| F2 | UG-06 per crate when its ledger is ≤ 10 sites; UG-05 in every gate binary; UG-07 |
| F3 | KC-35 only if MQ-05 says build |

(Phase F's rung F1 is not the physics step F1 of rung A1.)

## 3. DAG (edges between rungs; "," means parallel)

```
Docs:  plan approved → {DOC-1 → AP6 ; DOC-2 → EP3}                 (parallel with Phase A)
A:     A0 → A2 ; {A1, A2} → A4a → A5 ; A2 → A7 ; A3 → A6 ; O1 → A4b
       {A0, A1, A2, A3, A4a, A4b, A5, A6, A7, O1, O2} → A8
B:     A8 → {B1, B2, B3, B4}                (pool order at A8: B3, B1, B2; B4 next; §4.1)
C:     {B1, B2, B3} → D-M0 → C1 (needs AP6) → RF-K1 (needs A2)
       {A3, A6, B3} → RF-K2 ; {A2, B3} → RF-K3            (O1 is inside A8)
MEM:   {RF-K1, RF-K3} → D-M1 → D-M2 → D-M3 → D-M4 → {D-M5, D-M6 (needs RF-L)}
STORE: {RF-K2, AP6} → D-S1(i) → D-S1(ii)
       {D-S1(i), D-M0, RF-K1, AP6} → D-S2
       {D-S2, RF-K3} → D-S3(i) → D-S3(ii) (needs D-E0) → D-S3(iii) (needs EP3) → {D-S4, D-S5}
       {D-S2, RF-K3} → D-S6 ; {D-M1, RF-K3} → D-S7
       (D-S3(i), (ii) and (iii) are separate rungs, branches and lock holders; §2)
ENG:   RF-K3 → D-E0
       {RF-K2, EP3} → D-E1 → D-E2 (needs D-S3(iii))
       {RF-K3, EP3} → D-E3 → D-E4 → D-E5 → D-E6 → D-E7 (needs D-S3(iii)) → D-E8
         → D-E9 (needs D-M4, D-S3(iii)) → {D-E11 (needs D-S7), D-E14}
       {D-S5, D-E2} → D-E10 ; D-E11 → D-E12 → D-E13 → D-E17 ; D-E14 → D-E15 (needs D-S6) → D-E16
       {B1, RF-L} → D-E18 → D-E19 (needs D-S2)
R2:    {D-S2, D-M1} → D-R2a ; D-R2b..d after D-M1 and the features their rows name (D-R2d also needs D-S6)
Exit:  Phase-D exit → Phase E (per-rung prereqs in §2) → F
RF:    RF-0 now. After {A8, B3}: RF-R, RF-L, RF-V, and RF-T (RF-T also after RF-R).
       RF-L → {D-E18, D-M6}. RF-K1..K3 per Phase C. RF-E after D-E16. RF-P after U7, P2, S0.
       Priority inside D:/wt/refactor: §5
       RF-A after HO4 + RE4. RF-RE after RE2 / RE5. RF-U after UI3 (§5)
MQ:    D-M6 → MQ-13 → D-M6r (only if a U-19 overturn fires) ; D-S2 → MQ-03 (does not block)
M:     Stage 1 = D-S1(ii) (+ MS-08 with A6); Stage 3 after Phase-D exit (05 §7)
```

## 4. Worktrees, file locks and the order of shared edits

### 4.1 Worktrees

| Worktree | Carries | Opens | Closes |
|---|---|---|---|
| `D:/wt/joltab` (existing) | before A8: `merge/ke16-into-ecsnative` (A1 runs here) and the Phase-A merges; after A8: the **trunk** `integ/unified` — merges (with their anchor re-derivations, §4.2) and full gate runs only, no rung code | now | end of campaign |
| `D:/wt/refactor` (existing) | RF waves, one commit open at a time, under §5's priority rule | now (RF-0) | end of campaign |
| `D:/wt/k-1`, `D:/wt/k-2`, `D:/wt/k-3` (new) | the **code pool** for every rung from Phase B on (B1–B4, then Phases C–E), in any lane: exactly one open rung each; a rung switches a free pool worktree to `u/<rung>`, cut from the trunk | **all three at A8**, right after A8's merge commit. B3, B1 and B2 take them first; B4 takes the first one to go idle | end of Phase E |
| `D:/wt/k-docs` (new, no target dir) | design-document steps after A8 (§4.5) | A8 | end of campaign |

**Why the pool opens at A8 (critic pass 4, C2).**
- B1–B4 are code rungs, and nothing earlier can host them: the trunk worktree takes no rung code,
  `D:/wt/refactor` takes RF waves only, and `D:/wt/k-docs` builds nothing.
- Opening the pool after B3 made B3 wait for itself, and the only workaround was to develop Phase B
  in the trunk worktree while A8's full gates ran there. That is the two-agents-in-one-checkout
  hazard named below.
- The worktree count and RK-11's disk cap are unchanged; the three worktrees simply open one phase
  earlier.

- **Lanes order rungs; they are not worktrees.** MEM, STORE and ENG give the order (§3); the pool
  decides how many rungs run at once.
- **The pool's cost.** At most three code rungs are open at once. Where the DAG allows more (D-S6 and
  D-S7 beside D-S2 → D-S5; D-E14..16 beside D-E10..13; D-E18 and D-E19 beside D-E0), the extra
  rungs queue. That is the price of RK-11's disk cap and of one tester per worktree: two agents in
  one checkout produce a green that belongs to neither.
- **Per worktree.**
  - `CARGO_TARGET_DIR` is `D:/wt/_targets/<worktree>`, so a pool worktree reuses its incremental
    cache from rung to rung. The trunk reuses joltab's directory.
  - The docs worktree builds nothing; its gates run in the trunk worktree at merge (§4.5).
  - `linkedProjects` always lists the main checkout and the trunk worktree.
    - A pool worktree is listed while it holds an open rung.
    - `D:/wt/refactor` is listed while an RF commit is open.
    - Each is removed when it goes idle.
    - CLAUDE.md requires the main checkout to stay listed, and each listed project costs
      several GB of RSS (RK-11).

### 4.2 Lock rule

- A lock is held by one open **rung** (its branch `u/<rung>`), never by a lane.
- A lock covers a **file**. A rung's touch set is the list of files it edits. It is declared at the
  cut from the source design's integration section, and re-derived against the post-RF-K file map,
  where a split file is locked together with its child directory.
- A rung is cut only when no open rung holds a file in its touch set, and all of its fixed-order
  predecessors (§4.4) have merged.
- A rung that meets an undeclared file mid-work stops and escalates. It never takes the lock
  silently.
- Where the ORDER of two rungs changes behaviour, the order is fixed in §4.4 and is part of the
  DAG. Everywhere else, the first rung cut wins and the other waits.
- **Anchor lines are not locked; re-deriving them is a merge step** (critic pass 4, W5).
  - The four UG-10 documents are `docs/FEATURE_MAP.md`, `docs/SYSTEMS.md`, `docs/ARCHITECTURE.md`
    and `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md` (`[J]tests/internal_docs_anchors.rs:250`). They
    enter no rung's touch set on account of anchors.
  - A rung or RF branch that moves lines in an anchored file leaves those documents alone.
  - Its merge, in the trunk worktree, runs `merge --no-commit`, re-derives every moved anchor, runs
    UG-10 on the merged tree, and commits the merge and the re-derivation as one commit.
  - Merges are serial in the one trunk worktree, so two re-derivations never race, and no rung stops
    mid-work over an anchor.
  - **Why not a file lock.** `SYSTEMS.md` anchors into most kernel files (e.g.
    `[J]docs/SYSTEMS.md:601-661, 740-759, 935-947`), and `[G]:581` counts 195 gated anchors into
    census files. A file lock would let only one anchor-moving rung run at a time, and git offers no
    line lock.
  - On the branch, UG-10 runs scoped (03 §1).
  - Prose edits to these documents remain document steps (§4.5).

### 4.3 Shared files and the rungs that edit them (Phases A–D)

"→" is a fixed order (§4.4); "·" means first cut wins.

| File (`[J]` @ `d552be05` unless new) | Rungs |
|---|---|
| `ecs/core/ecs_master/entity_api.rs` (`delete_entity_core` `:980`) | D-S3(ii) → D-E2 |
| `ecs/core/ecs_master/ecs_master.rs` | RF-K3 → { D-M1 (`QueryStateCache` `:339`) · D-S3(ii) (`entity_master_mut`, `PHYSICS-ECS-UNIFICATION-DESIGN.md:3025`) · D-S3(iii) (`clear`, `:3308`) · D-S6 (KF-02 row `:953`) · D-R2a (`WorldScratch` field) · D-E9 (teardown driver) } |
| `ecs/core/ecs_master/binder.rs` (new) | D-S3(ii) → D-S5 (span frees inside the release points) |
| `ecs/core/ecs_master/tag_api.rs` | D-S1(i) |
| `ecs/core/component/hooks/archetype_flags.rs` | D-E2 (bit 13) · D-E11 (bit 14); the bit numbers are fixed in 01 §7 |
| `ecs/core/archetype/archetype.rs` | A3 → RF-K2 → D-S3(i) (`create_archetype` refuses `Group`) → D-S3(ii) (`anchor_mask`) → { D-E2 · D-E11 } (flag computation) |
| `ecs/core/archetype/archetype_bundle.rs` | D-M1 |
| `ecs/core/iters/query/{query,state,iter}.rs` | A3 (`query.rs`) → A8 (owner edits) → RF-K3 → { D-M1 · D-S3(ii) (new terms) } → D-S4 → { D-E7 · D-E11 · D-E14 } |
| `ecs/core/iters/query/filter.rs` | RF-K2 → D-E7 |
| `ecs/core/iters/query/{par_chunk,query_view}.rs` | A8 (owner edits) → D-S4 |
| `ecs/core/iters/query/query_type_registry.rs` | O1 → A4b |
| `ecs/core/schedule/schedule.rs` (`apply_window_drain` `:783`, `Arc` `:116`) | A2 → RF-K3 → D-E0 → D-S3(ii) · { D-M4 · D-S7 · D-E5 · D-E9 · D-E11 } |
| `ecs/core/system/params/mod.rs` | D-S3(ii) · D-E3 · D-E4 · D-E7 |
| `ecs/core/component/component_registry/{mod.rs, tags.rs}` | A6 → RF-K2 → D-S1(i) → { D-S1(ii) · D-S3(i) · D-E1 }. No order among the three changes behaviour, so the first cut wins, and the optional modding seam stays off the STORE critical path |
| `ecs/memory/component_pool.rs` | A3 → D-M0 → RF-K1 → { D-S2 · D-M1 } |
| `ecs/memory/{vm, vm_column}.rs`, `ecs/constants.rs` (in `boyko_memory` after C1) | D-M0 → C1 → { D-M1 · D-S2 (`VmReservation::UNRESERVED`) } |
| `ecs/core/component/scratch/**` | D-S2 → D-R2a |
| `ecs/core/component/dense/{dense_store, entity_slot_map, live_bitmap}.rs` | D-M1 |
| `ecs/core/component/dense/dense_registry.rs` | D-S3(ii) → D-S3(iii) → D-S5; D-S6 (KF-02 row `:78`) is first-cut-wins against all three |
| `ecs/core/entity/{entity_master, inland_store}.rs` | D-S3(ii) |
| `ecs/core/entity/entity_reservoir.rs` | A2 → A4a |
| `ecs/core/asset/{loader, error, server, staging}.rs` | D-E18 → D-E19 → (Phase E) AS2..AS5 |
| `boyko_physics/src/` — the D-S2 touch set (02 §2) | A1 → D-S2 → (Phase E) U4 … |
| `boyko_render/src/{mesh_draw, particle_system, particle, light_system}.rs` | D-S2 → (Phase E) RE2 / RE5 → RF-RE |
| `boyko_render/src/loaders/**` | O1 (`glb.rs`) → D-E18 → D-E19 |
| `boyko_app/src/runner.rs` | A2 → A5 (ddgi) → (Phase E) HO* → RF-A |
| `ecs/core/component/{component_pool_bundle.rs, enable/enable_store.rs}`, `ecs/core/asset/staging.rs` | D-S6 (KF-02 rows) · D-M1 (`component_pool_bundle.rs`, the `ArchetypeBundle` tables) · D-E18 → D-E19 (`staging.rs`) |
| `ecs/core/component/{hooks/scope.rs, observers/propagate.rs, component_registry/required.rs}` | D-M6 |
| `ecs/core/relationship/mod.rs` | D-M6 · D-E2 (EK15a/b) · D-E10 (EK15c) |
| `ecs/core/hierarchy/commands.rs` | D-M6 · D-E10 (`Children` on K7) |
| `boyko_threadpool/src/{block, scope}.rs` | A2 → RF-K1 → D-M2 → D-M3 |
| `boyko_threadpool/src/{thread_pool, worker, tls}.rs` | A2 → D-M2 → D-M3 → D-M4 → { D-M5 · D-M6 }. D-M5 adds board probes to the idle loop and D-M6 moves the `tls.rs` cells; no order between them changes behaviour |
| `boyko_threadpool/src/thread_ctx.rs` (new), `boyko_ecs/src/ecs/core/thread_fields.rs` (new), `boyko_threadpool/tests/tls_lane_merge.rs`, `tests/thread_ctx_census.rs` (new) | D-M6 → D-M6r (only if an MQ-13 overturn fires) |
| `boyko_diag/src/lane.rs`, `boyko_log/src/{drain_owner, sync_out}.rs` | D-M6 |
| `boyko_log/src/codes.rs` | A8 (owner edits) → RF-L → { D-E18 · D-M6 }. Any other rung that adds a code or flips a `CodeStatus` declares the file at its cut and runs the full `--workspace` sweep: the file's own lib-test pin (`[J]crates/boyko_log/src/codes.rs:1393-1528`) is not built under a `--test` filter (`:1400-1415`) |
| `boyko_render/src/retired_gpu_buffers.rs` | D-S6 → (Phase E) RE* |
| `boyko_rhi_vulkan/src/memory.rs` | D-E15 (the KF-36 edge, then the KF-02 row) |
| `docs/{FEATURE_MAP,SYSTEMS,ARCHITECTURE}.md`, `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md` | anchor lines: no rung (the merge step, §4.2). Prose: document steps in `D:/wt/k-docs` (§4.5), after O1 for the owner-dirty `FEATURE_MAP.md`, `SYSTEMS.md` and VG plan (00 §9 V-56) |
| root `Cargo.toml` | A8 → B3 (the three profiles) → C1 (the `boyko_memory` member) · any later rung that adds a crate or a profile |
| `docs/memory/{RUNTIME-DATA-LEDGER.md, runtime-data-ledger.tsv, ledger/**}` | B1 (in its pool worktree, §4.5) → every later ledger revision that re-pins UG-02 |

### 4.4 Fixed orders, and the step order inside `delete_entity_core`

| # | Order | Where | Why |
|---|---|---|---|
| 1 | D-S3(ii) before D-E2 | `delete_entity_core` | the redirect must return before `unbind` can drop a slot; D-E2's combined test needs both |
| 2 | D-E0 before D-S3(ii) | `apply_window_drain` | D-S3(ii)'s reconciliation call follows D-E0's rewritten loop |
| 3 | A2 before A4a | `entity_reservoir.rs` | A4a's fix is keyed on (file, allow count) |
| 4 | O1 before A4b | `query_type_registry.rs` | the owner's edit may be the same fix |
| 5 | D-S1(i) before D-S1(ii) | registry | UG-15 attribution (C2) |
| 6 | D-M0 before C1 and RF-K1 | packing files | the packing citations are re-derived once |
| 7 | D-S3(iii) before D-E9 | KC-13 | the teardown form needs `Stamped` and `died` |

**Step order inside `delete_entity_core`** (`[J]…/ecs_master/entity_api.rs:980-1112`), fixed:
1. inland validity checks (`:988-996`);
2. archetype re-mint and flags read (`:1026-1049`);
3. **D-E2's redirect.** Inside the existing `if !flags.is_empty()` branch (`:1050`), before
   `fire_despawn_hooks` (`:1051`): if `flags.contains(COUNTED_TARGET)`, sum the counted targets on
   the cold path. If the sum is > 0:
   - if the entity has `Pinned`, **enqueue** `RemoveCommand::<Pinned>` on `deferred_hook_queue`,
     the route that `enqueue_child_of_removal` already uses (`[J]…/hierarchy/commands.rs:108-109`);
   - drop the scope guard;
   - return `DespawnOutcome::Deferred`.

   Nothing below has run: no hook fired, no dense tombstone, no observer retired, no row moved or
   removed, no slot unbound.

   The `Pinned` removal is an ordinary migration, applied by the outermost drain at depth 0, with
   its hooks at that depth. Performing it here would have been a row move at depth ≥ 1 inside the
   despawn (critic pass 2, O1).

   The table-only, hook-free path already skips this branch, so no instruction is added to it;
4. despawn hooks (`:1051`), dense tombstones (`:1063`), observer retire (`:1070`);
5. `remove_entity` (`:1087`);
6. **D-S3(ii)'s `unbind`,** where `deallocate_entity` runs today (`:1091`, `:1102`).

D-E2's review checks this order against the merged D-S3(ii) text, and D-E2's red-first mutation
(§2) is its gate.

### 4.5 Documents after A8

- **One tree.** From A8 on, the trunk holds the only committed copy of `docs/**` that plan steps
  edit.
  - Design steps (DOC-1, DOC-2, and the AP6 and EP3 dispositions) and every later design revision
    are edited in `D:/wt/k-docs`, on a `u/doc-<step>` branch cut from the trunk.
  - **Exception: documents a gate reads as data.** The ledger (`RUNTIME-DATA-LEDGER.md`,
    `runtime-data-ledger.tsv`, `ledger/**`) is UG-02's input. B1 edits it in its pool worktree and
    pins UG-02 against it in the same commit; `D:/wt/k-docs` takes no lock on those files while B1
    is open. Every later ledger revision that re-pins UG-02 follows the same rule.
  - They are never edited in the main checkout.
- **Merge direction.** A doc branch merges into the trunk in the trunk worktree, with
  `merge --no-commit`. UG-10 (anchors), UG-02 (ledger) and UG-11 run on the merged tree. The merge
  is committed only if all three are green, and aborted otherwise.
- **The main checkout** receives documents only through the phase-boundary fast-forward (04 §2,
  step 11).
- **A document step that closes before A8** is committed on `feat/multi-paradigm-render`, as today,
  and reaches the trunk through A8's merge.
- **An owner commit on main after A8** is merged main → trunk at the next phase boundary, with the
  same three gates, before main fast-forwards. Nothing else commits on main after A8, so the
  fast-forward cannot fail the way O2's does today.

## 5. Where the refactor campaign fits

| Wave | Files (by the letter in the table below) | When | Why |
|---|---|---|---|
| RF-0 | pilot: `aether_lang/src/{expand, parse}.rs` ([G] §5.3) | now | Compile-time crate; conflicts with nothing. **Its gate is not UG-15:** `aether_lang` is never linked into the shipped binary (ledger TSV), so a strict UG-15 run would be vacuous. The pilot's first commit adds a snapshot test over the crate's own fixture corpus, whose red-first control is a one-character change to one expansion rule. The move commit must leave every snapshot byte-identical |
| RF-R | R | **after B3**, so leg (2-RF)'s pins exist before the first split; then in parallel with C–D | No kernel rung adds to these files. Its anchor re-derivations edit `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`, which is owner-dirty until O1; `render_path_config.rs` alone holds 6 waived gated anchors ([G]:621). Its `particle.rs` is `boyko_app/src/gpu_scene/particle.rs` ([G]:89), not `boyko_render/src/particle.rs`, which D-S2 edits |
| RF-T | T | after B3 and RF-R | test-only; UG-12 per device file |
| RF-K1..K3 | K1 / K2 / K3 | Phase C, before first use | kernel rungs add to them |
| RF-L | L (`boyko_log/src/codes.rs`) | after A8 and B3, and before D-E18 and D-M6 | The file is owner-dirty until owner step O1 and reaches the trunk only through A8. B3 captures the UG-15 pins the move is gated by. D-E18 and D-M6 add rows to the file (U-12). The move also updates UG-18's path-keyed `code_registry` census |
| RF-V | V (`boyko_rhi_vulkan/src/present/targets.rs`, `boyko_rhi_vulkan/src/device.rs`) | after A8 and B3 | Owner-dirty until owner step O1. A8 also contains `chore/msvc-host`, which edits `device.rs`. No kernel rung adds to these files. UG-12 applies per device file |
| RF-P | P | after U7, P2, S0 | U4–U7 rewrite them |
| RF-A | A | after HO4 / RE4 | the runner shrinks to the pump |
| RF-RE | RE | after RE2 / RE5 | those rungs rewrite them |
| RF-E | E | after D-E16 | D-E16 folds KF-40's two rows in it (`RUNTIME-DATA-LEDGER.md:1552`) |
| RF-U | U | after UI3 | UI3 rewrites it |
| never | N | — | deleted by a scheduled rung, or holding pinned baselines |

**Every census file and its wave.** [G] §1.1 and §1.2 list 58 source files and 4 test/bench files.
This table **supersedes [G] §2.4's "free" verdicts** (`[G]:420-487`). [G] classifies blockers at
`d552be05`; this table schedules against the rungs, and where the two differ, this table rules.

| [G] # | File | Wave |
|---|---|---|
| 1 | `boyko_rhi_vulkan/src/present/targets.rs` | V |
| 2 | `boyko_app/src/gpu_scene/mod.rs` | A |
| 3 | `boyko_rhi_vulkan/src/present/graph_bridge.rs` | R |
| 4 | `boyko_rhi_vulkan/src/present/passes/vb.rs` | R |
| 5 | `boyko_rhi_vulkan/src/compute.rs` | R |
| 6 | `boyko_rhi_vulkan/src/goldens.rs` | R (its blocking lane `feat/golden-edsl-p0` merges in A5, before A8) |
| 7 | `boyko_rhi_vulkan/src/present/scene_types.rs` | R (after A7, which A8 contains) |
| 8 | `boyko_rhi_vulkan/src/device.rs` | V |
| 9 | `boyko_ecs/src/ecs/memory/component_pool.rs` | K1 |
| 10 | `boyko_rhi_vulkan/src/ffi.rs` | R |
| 11 | `boyko_physics/src/resources.rs` | P |
| 12 | `boyko_app/src/runner.rs` | A |
| 13 | `aether_lang/src/expand.rs` | RF-0 |
| 14 | `boyko_physics/src/solver/colored.rs` | P |
| 15 | `boyko_render/src/render_path_config.rs` | R |
| 16 | `boyko_rhi_vulkan/src/present/passes/gbuffer.rs` | R |
| 17 | `boyko_ecs/…/iters/query/filter.rs` | K2 |
| 18 | `boyko_physics/src/solver/colored_tests.rs` | P |
| 19 | `boyko_rhi_vulkan/src/compute/tests.rs` | R (with #5) |
| 20 | `boyko_shaderdsl/src/emit/shaders.rs` | R (after A7) |
| 21 | `boyko_shaderdsl/src/emit/mod.rs` | R (after A7) |
| 22 | `boyko_ecs/…/commands/migration_helpers.rs` | K2 |
| 23 | `boyko_ecs/…/asset/assets.rs` | N (AS2–AS5 delete it) |
| 24 | `boyko_rhi_vulkan/src/rhi_impl/device.rs` | R |
| 25 | `boyko_render/src/light.rs` | R |
| 26 | `boyko_ecs/…/archetype/archetype.rs` | K2 |
| 27 | `boyko_ecs/…/schedule/schedule.rs` | K3 |
| 28 | `boyko_render/src/csm_config.rs` | R |
| 29 | `boyko_threadpool/src/scope.rs` | K1 |
| 30 | `boyko_ui/src/layout.rs` | U |
| 31 | `boyko_ecs/…/iters/query/state.rs` | K3 |
| 32 | `boyko_render/src/hzb.rs` | R |
| 33 | `boyko_threadpool/src/block.rs` | K1 |
| 34 | `boyko_ecs/…/profiling/tests.rs` | K3 |
| 35 | `boyko_render/src/mesh_draw.rs` | RE |
| 36 | `boyko_macros/src/component.rs` | K2 |
| 37 | `boyko_shaderdsl/src/cf.rs` | R (after A7) |
| 38 | `boyko_ecs/…/schedule/schedule_builder.rs` | K3 |
| 39 | `boyko_app/src/gpu_scene/particle.rs` | R |
| 40 | `boyko_shaderdsl/src/bin/emit_particles.rs` | R |
| 41 | `boyko_ecs/…/ecs_master/ecs_master.rs` | K3 |
| 42 | `boyko_sdf_math/src/brick/tests.rs` | R |
| 43 | `boyko_sdf_math/src/brick.rs` | R |
| 44 | `boyko_ecs/…/iters/query/iter.rs` | K3 |
| 45 | `boyko_ecs/…/component_registry/mod.rs` | K2 |
| 46 | `boyko_render/src/shadow_atlas.rs` | R |
| 47 | `aether_lang/src/parse.rs` | RF-0 |
| 48 | `boyko_physics/src/solver/simd.rs` | P |
| 49 | `boyko_physics/src/systems.rs` | P |
| 50 | `boyko_log/src/codes.rs` | L |
| 51 | `boyko_ecs/…/archetype/archetype_master.rs` | K2 |
| 52 | `boyko_render/src/gpu_column.rs` | E |
| 53 | `boyko_ecs/…/profiling/store.rs` | K3 |
| 54 | `boyko_render/src/light_system.rs` | RE |
| 55 | `boyko_ecs/…/commands/command_queue.rs` | K3 |
| 56 | `boyko_ecs/…/iters/query/query.rs` | K3 (after A3 and O1) |
| 57 | `boyko_rhi_vulkan/src/present/gpu_zone.rs` | R |
| 58 | `boyko_render/src/upload.rs` | A |
| T1 | `boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | T |
| T2 | `boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs` | T |
| T3 | `boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs` | N. It holds the measured VB barrier baselines its generators emit (CLAUDE.md warns that re-running those generators casually re-measures the baselines), and no rung edits it. The owner may override |
| T4 | `boyko_ecs/benches/cull_diagnostic.rs` | T. UG-15 leg (2) is unaffected: this bench is not one of its pinned bodies |

**Every refactor commit** is a pure move. It carries:
- anchors re-derived in its merge commit, in the trunk worktree (§4.2; 195 gated anchors point into
  census files, [G]:581); on the branch, UG-10 runs scoped;
- trybuild `.stderr` re-blessed path-only ([G] §3.8);
- path-keyed censuses updated ([G]:584-590);
- UG-15 strict: the kernel pins and the wave's own leg (2-RF) pins (03 §6) are identical once the
  commit's move map is applied to symbol names. Otherwise the commit is held and an MQ-12 entry is
  filed.

RF-0's gate is the snapshot equality above, not UG-15 (critic pass 2, O6).

**Refactor-worktree priority (critic pass 4, W5).** `D:/wt/refactor` holds one open commit. Its
waves are of two kinds:
- **Kernel-critical.**
  - Priority order: RF-L (holds D-E18 and D-M6); RF-K3 (holds D-M1, D-M4, D-S3(i), D-S6, D-S7, D-E0,
    D-E3); RF-K2 (holds D-S1(i), D-E1); RF-K1 (holds D-M1, D-S2).
  - A kernel-critical wave is taken as soon as its prerequisites have merged, ahead of every other
    wave.
- **Device-gated:** RF-R, RF-V, RF-T, RF-A, RF-RE, RF-E and RF-U.
  - Each such commit needs the owner's UG-12 run. It is finished up to UG-12, committed on its own
    branch `rf/<wave>-<n>`, and **parked**; the worktree then moves on to the next commit.
  - The wave table assigns every census file to exactly one wave, so a parked branch touches no
    kernel-critical file.
  - If a kernel-critical merge lands meanwhile, the parked commit is rebased and re-runs its CPU
    gates, UG-15 strict included, against the new parent before it merges.
  - The owner's UG-12 receipt stays valid across that rebase: a green strict UG-15 shows that the
    kernel codegen the receipt ran on is unchanged.
- **RF-P** waits for the physics rungs and is neither kind.

As a result, the kernel critical path never waits on device availability, and the worktree never
holds two open commits.

The 1,018 ungated anchors are recorded as drift, not repaired ([G]:591-593).

## 6. Ledger rungs mapped to plan rungs

| Ledger rung | Count (`RUNTIME-DATA-LEDGER.md:1809-1840`) | Plan rungs |
|---|---|---|
| R1 (kernel features physics needs, 0 own rows) | — | D-M5, D-S2..S5, D-E8, D-M6 |
| R2 | 333 | retired by D-M1, D-M2 (part), D-S6, D-S7, D-E*, D-R2a–d |
| R3 | 80 | physics U4..S0 |
| R4 | 156 | engine subsystem rungs + 5 boyko_ecs eliminations (in D-R2a) |
| R5 | 38 | D-M2, D-M3, D-M4, D-M6 |
| R6 | 648 | F1 |
| out of scope | 1102 | none |

**Recount rule.** The rev-4 counts are re-derived at B1 (ledger rev 5), and each rung re-reads the
ledger revision before starting. This is P43.3, applied both ways
(`ALLOCATOR-DESIGN-SPACE.md:3799`).
