# Checkpoint 2026-09-11 - the allocation campaign becomes the unified ECS system

- **Taken:** 2026-09-11 08:38, at the owner's request ("stop at a checkpoint and record everything").
- **State at the stop:** three workflows were running. All three were stopped at a point where no
  file was half-written: the ledger rev 3 worked only on scratch copies, msvc pass 8 had written
  nothing, and the id-recycling tester runs tests and writes nothing. No build process was left
  running.
- **Supporting records:** [checkpoint-2026-09-11/](checkpoint-2026-09-11/). This directory holds
  verbatim copies of every report that existed only in workflow journals or in the session
  scratchpad on drive C:, plus the scripts of the stopped workflows.

## 1. What the owner asked for, in order

Verbatim in translation.

| When | Order |
|---|---|
| 2026-09-10 | "Ideally, first finish the kernel, then get rid of all the Vecs. We made the memory library precisely to minimise the use of standard allocations." |
| 2026-09-10 | "Forbid Vec by a gate. There is not one reason to use an allocator other than ours. If something is missing, extend the memory library. Do a deep research." |
| 2026-09-10 | "First let's deal with allocations. It is the foundation of the engine, and I am sure it affects performance a lot." |
| 2026-09-10 | "Move all runtime data structures onto our system, and all arrays and so on onto the ECS." |
| 2026-09-10 | "The point is not only to move everything onto our allocator, but to bring everything as close as possible to the ECS paradigm, in particular in physics, and to make one unified system." |
| 2026-09-11 | "Don't run benchmarks, the machine is not quiet." |
| 2026-09-11 | "Not only physics must be ECS, but all the runtime things too - for example the interface." |
| 2026-09-11 | "Decide all the questions yourself, whichever is best for performance." |
| 2026-09-11 | "Clean drive C." Answered with the commands for the owner to run (section 8); permanent deletion is not done on the owner's behalf. |
| 2026-09-11 | "Stop at a checkpoint and record everything." This file. |

## 2. What was measured

| Measurement | Result | Where it is recorded |
|---|---|---|
| System heap vs mimalloc, 1240-body Jolt pyramid, `D:/wt/joltab` | mimalloc/system = 0.998 / 1.023 / 0.992 at W = 1 / 8 / 16. The W=8 band is ±5 %, and a run with the arms in reverse order gave 1.005. **The allocator is not a speed lever.** | `docs/OPEN-QUESTIONS.md` on `merge/ke16-into-ecsnative` (`d11962a9`); [allocator-ab.md](checkpoint-2026-09-11/allocator-ab.md) |
| Frame-allocation census, **before** thread-pool Stage 3b (`D:/wt/ecsnative` @ `ad0ebea4`) | An App frame makes `n + 4` allocations with n systems. A parallel physics step makes ~2,670. | [frame-allocation-census.md](checkpoint-2026-09-11/frame-allocation-census.md) |
| The same census on the **KE16 pool** (`D:/wt/joltab`) | An App frame makes a flat **2** (one scope frame and one 4 KiB task chunk per `Schedule::run`). A parallel physics step makes **302–339** (~1.2 MB). Events, change detection, queries, system bodies and every physics buffer measured **0**. The counter sees only the Rust heap; arena growth through `VirtualAlloc` is not counted. | same file; harness uncommitted in `D:/wt/joltab` |
| Jolt head-to-head (recorded earlier at `ca582e72`) | Single-threaded the two engines are ~1.0×. At W=8 boyko is 2.47× slower. Amdahl puts the serial fraction at 0.43–0.53. | `docs/OPEN-QUESTIONS.md` on `merge/ke16-into-ecsnative` |
| Per-stage timing of the pyramid (the measurement that decides the Jolt residual) | **Not taken.** It was stopped by the owner's no-benchmark order before it ran. | script: [workflows/jolt-pyramid-per-stage-timing-wf_cc3f9944-c7a.js](checkpoint-2026-09-11/workflows/jolt-pyramid-per-stage-timing-wf_cc3f9944-c7a.js); clean worktree `D:/wt/stagetime` |

**Correction made during the campaign.** The census figure first reported to the owner, 2,671
allocations per step and "`block.rs` wired to nothing", describes the pool **before** Stage 3b
(`d51b4ced`). The shipped KE16 pool has Stage 3b. The A/B adjudicator found the mismatch by building
a probe against the timed binaries. Every brief was corrected afterwards.

**Found by the physics design, not by a timing:**
- The pyramid runs the `AllPairs` broadphase, a serial O(n²) sweep of 769,420 pairs per step. The
  4096-body parallel gate is never reached.
- A `par_iter` inside a scheduled system has been as parallel as one from the dispatcher since KE16.

## 3. What was designed

| Document | Status | Commit |
|---|---|---|
| `docs/memory/ALLOCATOR-RESEARCH.md`, `docs/memory/ALLOCATOR-DESIGN-SPACE.md` | Four research lenses and a rev-1 design, plus one critique with two blockers: C1, a heap back-pointer that aliases under Tree Borrows, and C2, a scope arena keyed by a shared role sentinel. A dated note says rev 1 was built on the pre-Stage-3b pool. | `dc35fae8` |
| `docs/physics/PHYSICS-ECS-UNIFICATION-RESEARCH.md`, `-DESIGN.md`, `-DECISIONS.md` | **Closed at rev 5** after five critique passes, with blocking findings per pass of 3 / 5 / 3 / 2 / 0. A body is an entity with `Collider` anchoring a dense solver group. Contacts and sleep reach gameplay as kernel events. Every stage is a system, and parallelism comes from kernel drivers. | `7770aa9e` |
| `docs/unification/ENGINE-RUNTIME-ECS-RESEARCH.md`, `-DESIGN.md`, `-DECISIONS.md` | Rev 3 after two critique passes. Pass 2 found five blockers, answered by the rev-3 patch; the patch has not been re-critiqued. The design has one world (no extracted render world) and three core schedules (`Fixed → Main → Render`); the host keeps only the OS pump. The 13 exclusive UI systems become one. Its reconciliation with physics rev 5 (K3, K6, K7) is deferred to the unified plan. | this checkpoint |
| `docs/memory/RUNTIME-DATA-LEDGER.md`, `docs/memory/ledger/*.md`, `docs/memory/runtime-data-ledger.tsv` | **Rev 2.** 2,366 rows, of which 2,244 are active and 122 are `boyko_ui` rows superseded by the UI lane. 47 kernel features, 0 unclassified rows. Rungs: R2 223, R3 73, R4 202, R5 38, R6 667, out of scope 1041. The rev-2 recheck found seven consistency gaps and six minor items; they are the rev-3 work order. | this checkpoint |

## 4. Decisions taken under the owner's delegation

The owner delegated every open question to one criterion, performance. Each decision names the
measurement or gate that would overturn it.

- **Physics** ([PHYSICS-ECS-UNIFICATION-DECISIONS.md](../physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md)):
  - contacts are kernel events;
  - soft-body particles use the segmented dense column (K7);
  - sleep lives in `BodyGate` inside the solver group, plus transition events;
  - `RigidBody` stays authored, and the solver works on a derived dense group;
  - timings run only on the owner's word that the machine is quiet;
  - the broadphase default becomes `Auto`;
  - the Rapier and Jolt-FFI allowances are retired.
- **Engine runtime** ([ENGINE-RUNTIME-ECS-DECISIONS.md](ENGINE-RUNTIME-ECS-DECISIONS.md)):
  - assets are entities;
  - the UI is opt-in (`UiPlugins`), against the design's recommendation;
  - window and player are entities now, against the design's recommendation;
  - dead paths are deleted;
  - timings run only on the owner's quiet word.
- **Ledger:** the gap closures, each decided by performance, are listed in the rev-2 change log in the
  ledger index.
- **msvc citation repair:**
  - the closing scope: lane shifts are repaired by content, quotations and dated records are restored,
    and pre-lane rot is named as debt;
  - the dating rule: a dating clause counts only in the sentence, its paragraph, or a heading or
    lead-in that scopes the block, never in a document-wide header;
  - `VG-R3-P3:2924` keeps `335-341`.

## 5. Defects found

| # | Defect | Status |
|---|---|---|
| 1 | `Commands` never recycles entity ids. A population under constant spawn/despawn churn grows the free list and the slot store without bound: +131,072 entries in 2,048 frames. | Fix EM2′ (a claimable free-list stack) was designed, critiqued, implemented red-first and **reviewed APPROVED**. The tester step has **not run**. The fix is **uncommitted** in `D:/wt/joltab`. [commands-id-recycling.md](checkpoint-2026-09-11/commands-id-recycling.md) |
| 2 | Physics state is keyed by gather row, not by body. (A1) With sleeping on, a new body can inherit a sleep latch and hang in the air forever. (A2) An unrelated despawn wakes a whole pile. Warm start, which is on by default: after a despawn, a body that moves into another's row takes that row's impulse for one step. | Confirmed by reading the code; not reproduced. Plan: a partial latch fix now, the rest in physics rungs U5–U7, and three red-first tests. [latent-defects.md](checkpoint-2026-09-11/latent-defects.md) |
| 3 | A rejected GPU upload (`gpu_upload.rs:120`, `let _ = assets.fill(...)`) leaks its buffers, BLAS and bindless slot. `MeshGeometryTable::unregister` has no caller at all. | Confirmed by reading the code. Fix: route the value into the existing `Orphaned*` queues. Same file. |
| 4 | The pool allocates a scope frame and a 4 KiB chunk for every scope and frees both at the join: 2 per App frame, ~330 per parallel physics step. | For the unified plan: a pool-owned free list plus a cached chunk per joiner, taking both to 0. |
| 5 | Parallel dispatch on a one-worker pool costs 254 allocations per step and gives no parallelism. | For the physics rungs. |
| 6 | Each pool thread's first-touch objects (a crossbeam-epoch `Local`, the thread-name UTF-16 copy) land in early frames. | `ThreadPoolBuilder::build` should wait for boot and register each thread with the epoch collector. |
| 7 | On the `windows-gnu` toolchain every `thread_local!` allocates one System cell per thread, bypassing any `#[global_allocator]`. There are 31 non-test statics. | Ledger decision: an engine-owned per-thread slot. |
| 8 | The census gate had two defects: its upward headroom let an extra fan-out pass, and libtest's 60-second notice allocated inside the window. | Fixed and verified by the gate-fix agent. **Uncommitted**; it lands with defect 1 because the census carries the S2 pin that change re-derives. [census-gate-fix.md](checkpoint-2026-09-11/census-gate-fix.md) |

## 6. Commits of this campaign

| Commit | Branch | What | Pushed |
|---|---|---|---|
| `d11962a9` | `merge/ke16-into-ecsnative` | Allocator A/B record and the `bench-alloc` diagnostic | yes |
| `dc35fae8` | `feat/multi-paradigm-render` | Allocator research and rev-1 design space | yes |
| `7770aa9e` | `feat/multi-paradigm-render` | Physics as one ECS system, closed at rev 5, with the decisions | yes |
| `78759956` | `chore/msvc-host` | The msvc lane's citation repair, passes 1–7 (checkpoint; pass 8 owed) | no - local branch without an upstream |
| this checkpoint | `feat/multi-paradigm-render` | Engine-runtime docs (rev 3), ledger rev 2, this file and its records | yes |

Earlier commits of the same session (the KE16 closure, the merges, the DDGI fixes, the msvc lane
itself) are listed per branch in section 9.

## 7. Work in progress at the checkpoint

### Uncommitted in `D:/wt/joltab` (`merge/ke16-into-ecsnative` @ `d11962a9`), 31 paths

- **EM2′ id recycling:**
  - kernel files: `command_queue.rs`, `spawn_at_command.rs`, `deferred_master.rs`, `ecs_master.rs`,
    `entity_api.rs`, `entity_master.rs`, `inland_store.rs`, `entity/mod.rs`, `schedule.rs`,
    `params/commands.rs`, `params/entities.rs`, `params/entity_counter.rs`, `unsafe_ecs_cell.rs`,
    `memory/vm_column.rs`, and the new `entity/entity_reservoir.rs`;
  - tests: the new `tests/em_deferred_recycle.rs`, updated `ke2_entities_param.rs` and
    `ke3_query_random_access.rs`, and the `par_iter` compile-fail fixture with its `.stderr`;
  - docs: `ARCHITECTURE.md`, `FEATURE_MAP.md`, `SYSTEMS.md`, `MEASUREMENT-QUEUE.md`.
- **Census gate:** `crates/boyko_physics/tests/alloc_frame_census.rs` and `alloc_frame_attribution.rs`
  (both new), and `crates/boyko_physics/Cargo.toml` (`harness = false` for the two targets).
- **To land it:** run the tester step. That is `cargo check`, `clippy -D warnings` and
  `test --no-fail-fast` over the workspace, the census in both profiles, the red-first test, and Miri
  on the new lock-free structure with all five `MIRIFLAGS`. Then make two commits, the census gate
  and then the id recycling, and push.

### Other trees

- `D:/wt/ecsnative`: two untracked census files, the pre-Stage-3b copies. `D:/wt/joltab`'s copies
  supersede them.
- `D:/wt/stagetime` (`timing/per-stage-pyramid` @ `d11962a9`): a clean worktree reserved for the
  per-stage timing.
- The main checkout keeps the owner's own uncommitted files. This checkpoint does not touch them.

### Stopped workflows

`resumeFromRunId` replays completed agents from cache. The scripts are copied here as durable work
orders.

| Workflow | Run id | Done | Remaining | Script |
|---|---|---|---|---|
| census gate + Commands id leak | `wf_396905fa-b76` | gate fix, design rev 2, implementation, review | tester | [workflows/census-gate-and-commands-id-leak-wf_396905fa-b76.js](checkpoint-2026-09-11/workflows/census-gate-and-commands-id-leak-wf_396905fa-b76.js) |
| ledger rev 3 | `wf_a087d922-d51` | nothing | fix, regenerate, recheck | [workflows/runtime-data-ledger-rev3-wf_a087d922-d51.js](checkpoint-2026-09-11/workflows/runtime-data-ledger-rev3-wf_a087d922-d51.js) |
| msvc citations pass 8 | `wf_86934723-4ac` | nothing | apply, then a mechanical check | [workflows/msvc-citation-repair-pass8-last-wf_86934723-4ac.js](checkpoint-2026-09-11/workflows/msvc-citation-repair-pass8-last-wf_86934723-4ac.js) |
| per-stage pyramid timing | `wf_cc3f9944-c7a` | nothing (stopped by the no-benchmark order) | instrument, measure, adjudicate; needs the quiet-machine word | [workflows/jolt-pyramid-per-stage-timing-wf_cc3f9944-c7a.js](checkpoint-2026-09-11/workflows/jolt-pyramid-per-stage-timing-wf_cc3f9944-c7a.js) |

## 8. Next steps, in order

1. **Land EM2′ and the census gate** on `merge/ke16-into-ecsnative`: tester, two commits, push.
2. **Fix defects 2 (A1 latch) and 3 (GPU upload leak)** with red-first tests on the same tree. A2 and
   the warm-start hit wait for physics U5–U7.
3. **Ledger rev 3.** Work order: [ledger-rev3-work-order.md](checkpoint-2026-09-11/ledger-rev3-work-order.md).
4. **msvc pass 8, then push or merge the lane.** Work order:
   [msvc-citations-pass8-work-order.md](checkpoint-2026-09-11/msvc-citations-pass8-work-order.md).
5. **The unified plan**, `docs/unification/UNIFIED-SYSTEM-PLAN-*.md`, in several files:
   - the kernel contract: one name, one contract and one owner per feature across the allocator
     design, physics K1–K7, engine EK1–EK21 and K6′, and ledger KF-01 to KF-47. It includes allocator
     rev 2, which answers C1 and C2 and starts from the KE16 pool facts;
   - the order of work, kernel first;
   - the gate set: ledger non-ECS forms only shrink, the per-class allocation census, structural
     gates, and G-jolt on the quiet-machine word;
   - the branch integration order.
6. **The per-stage timing**, when the owner says the machine is quiet.
7. **Steps that are the owner's:**
   - commit the seven files that overlap the merge, then run
     `git merge --ff-only merge/ke16-into-render`;
   - run the drive-C cleanup commands. About 68 GB of it is build caches this campaign created:
     `scratchpad/msvc/target-full` (29.9 GB) and `~/.cargo/boyko-target-msvc` (37.9 GB). Both are
     safe to delete; builds recreate them.

## 9. Branch fleet at the checkpoint

| Worktree | Branch @ HEAD | Upstream | Uncommitted |
|---|---|---|---|
| `D:/claude/BoykoEngine` | `feat/multi-paradigm-render` @ `7770aa9e` + this checkpoint | origin, in sync | the owner's own files |
| `D:/wt/joltab` | `merge/ke16-into-ecsnative` @ `d11962a9` | origin, in sync | 31 paths (section 7) |
| `D:/wt/msvc` | `chore/msvc-host` @ `78759956` | none | 0 |
| `D:/wt/merge` | `merge/ke16-into-render` @ `d2c8c646` | none | 0 - waits for the owner's step |
| `D:/wt/ecsnative` | `feat/ecs-native-storage` @ `ad0ebea4` | origin, in sync | 2 untracked |
| `D:/wt/threadpool` | `feat/threadpool-ke16` @ `5550a3da` | origin, in sync | 0 |
| `D:/wt/docgates` | `chore/doc-gates` @ `add0c21b` | none | 0 |
| `D:/wt/ddgi` | `fix/ddgi-host-hook` @ `230585d0` | none | 0 |
| `D:/wt/kernel` | `fix/ke13-ke14` @ `1aaadfdc` | none | 0 |
| `D:/wt/ui` | `feat/ui-advanced` @ `615cda8f` | origin, 2 ahead | 0 |
| `D:/wt/reflect` | `feat/reflection` @ `0e0b4c68` | origin, 2 ahead | 0 |
| `D:/wt/stagetime` | `timing/per-stage-pyramid` @ `d11962a9` | none | 0 |
| `D:/wt/census` | `fix/census-post-lto-object` @ `27ac8904` | none | 0 |
| `D:/wt/golden` | `feat/golden-edsl-p0` @ `a8fc2e2a` | none | 0 |
| `D:/wt/assets` | `fix/asset-validate-prereqs` @ `419aaf2d` | none | 0 |
| `D:/wt/register` | `docs/ab-register-sync` @ `4559b6a3` | none | 0 |

## 10. Lessons recorded

- **A measurement is a claim about the tree it was taken on.** This happened twice in one day: the
  Jolt harness ran without KE16, and the census ran without Stage 3b. Before quoting a number as a
  property of the shipped code, check `git merge-base --is-ancestor <fix> <measured tree>`.
- **A map move cannot make a pre-existing wrong citation right.**
  - Separate the shifts the lane caused from rot that was there before it.
  - Never renumber a quotation or a dated record.
  - A verifier's blanket rule for a whole target file hides errors; judge each number by content.
  - End a repair loop with a mechanical check that expects a stated number, not with a new open audit.
- **Only the last message of an agent reaches the workflow journal.** An architect whose output ran
  past the limit and was split over two messages lost its first half there. Ask long outputs to be
  written as files or as numbered parts.
- **The workstation belongs to the owner.** A "quiet" word covers one window of work, not the rest of
  the session, and build caches never go on drive C:.

## 11. Update 2026-09-13 - the first batch after the checkpoint

The owner resumed the work: "continue, stop at a checkpoint and record it for compaction". Before that
the owner ran the drive cleanup (C: 61 GB free, D: 105 GB free) and took the KE16 merge and the seven
overlapping files back as the owner's own step.

| Item | Result | Commit |
|---|---|---|
| EM2' entity-id recycling | Tester: workspace 5620 passed, 1 failed (pre-existing, see below), 156 ignored; clippy clean; Miri runs the claim races clean; two mutations of the lock-free claim turn red. A test comment that claimed a separate Miri twin was false and was corrected before the commit. | `0afcbd7d` on `merge/ke16-into-ecsnative`, pushed |
| Frame-allocation census and its gate | The two gate defects fixed; four mutations turn it red; both profiles green. | `d5782d43`, same branch, pushed |
| msvc citations, pass 8 | Mechanical check PASS: the debt is 153 numbers on 73 lines. The lane's repair is closed. | `a36ceaa4` on `chore/msvc-host`, local |
| Runtime data ledger, rev 3 | 2481 rows: 2350 active, 131 superseded; 4492 non-rows. Rungs R2 314, R3 73, R4 161, R5 38, R6 662, out of scope 1102. 673 rows in ECS data forms, 0 unclassified. The recheck confirmed every total and found three small consistency gaps, recorded as the rev-4 work order. | this commit |

Of the four workflows stopped at the checkpoint, three have completed. The per-stage pyramid timing
(`wf_cc3f9944-c7a`) stays stopped until the owner says the machine is quiet.

**Corrections to the sections above.**
- Section 7 said the census gate lands before the id recycling. It had to be the reverse: the census
  header pins the churn scene after EM2' (0 reallocations). It landed as `0afcbd7d`, then `d5782d43`.
- The one red test in the workspace run, `anyof_dense_plus_enable_yields_correct_rows` (QueryTypeId
  exhaustion), is not caused by EM2'. It is a pre-existing race between the exhaustion unit tests in
  `query_type_registry.rs` and any lib test that mints a query type, red in 1 of 11 runs. It is filed as
  a separate task.

**Environment.** The tester saw a second page-cache bit flip this week: one bit in a cargo registry
source file, which read back correctly half an hour later, and a rustc access violation in the same
build. No hardware error was logged, which proves nothing without ECC memory. The owner was told.

**Open after this batch, in order.**
1. Fix defect 2 (the A1 sleep latch) and defect 3 (the rejected GPU upload leak) red-first on
   `merge/ke16-into-ecsnative`. Work order: [latent-defects.md](checkpoint-2026-09-11/latent-defects.md).
2. Ledger rev 4. Work order: [ledger-rev4-work-order.md](checkpoint-2026-09-11/ledger-rev4-work-order.md).
3. The unified plan, as in section 8, point 5.
4. A loom model of `EntityReservoir`. EM2' has Miri coverage of its claim races and no loom model.
5. The per-stage timing, on the owner's quiet word.
6. The owner's steps: the KE16 merge into `feat/multi-paradigm-render`, and a memory test of the machine.
