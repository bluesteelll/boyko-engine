# Unified system plan — 00 Overview (rev 5.1)

## Index

| File | Content |
|---|---|
| [UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md](UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md) | this file: goal, target metrics, rulings U-1..U-20, phase map, source-document patches, risks, owner questions, readiness, citation verification record, revision log, external sources |
| [UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md](UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md) | contract invariants, the unified kernel features KC-01..KC-36 (KC-19 split into a/b, KC-30 into a/b/c), source-id mapping, conflict resolutions, threading model (with the KC-04 thread-context protocol, its capacity rule and the plan-build set), invariants and edge cases |
| [UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md](UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md) | sequencing rules, rungs per phase (A–F) and the design-pass steps, DAG, worktrees, per-rung file locks and fixed edit orders, refactor waves and the refactor-worktree priority rule, the anchor merge step, ledger-rung mapping |
| [UNIFIED-SYSTEM-PLAN-03-GATES.md](UNIFIED-SYSTEM-PLAN-03-GATES.md) | gate inventory UG-01..UG-21, gates per rung, Miri and loom rows, device legs, measurement-queue entries MQ-01..MQ-20 and the timing-profile rule, the no-cost gate UG-15 (seven legs, strict and attributed modes, eight red and two green controls) |
| [UNIFIED-SYSTEM-PLAN-04-INTEGRATION.md](UNIFIED-SYSTEM-PLAN-04-INTEGRATION.md) | the branch fleet, merge order, the owner's steps, trunk policy, prune list |
| [UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md](UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md) | modding as testable properties under the owner's zero-overhead requirement H-1 and rule S-1 (kernel modding code is generic over `ModSeam`, so it is not compiled when unused), what stays compile-time, what each live option needs from the kernel, additive modding paths MS-01..MS-15 per option, reconciliation with allocator §7, carried remarks (RM-1..RM-4 new), the no-cost gate, modding sequence |

**Status (2026-09-17): closed at rev 5.1 by orchestrator ruling.** Critic pass 5 found no Critical
remark. Its four Important remarks are OPEN and listed in §10 under "Critic pass 5 log (final)";
each is resolved before the step it names (W1 before B3, W2 with the KC-04 loom rung, W3 before the
RF-V wave, W4 is closed by committing the allocator design, which this commit does). The owner
questions in §7 are the only open decisions of values or scope.

**Status.** Rev 5: the architect's patch answering critic pass 4, applied on top of the partly
applied rev 4. Rev 5 re-emits, as one text, every part of rev 4 that was never applied (00 U-19,
RK-12, §5, §11; 01 KC-04 and §6). The rev-4 parts P0-* and P1-1 to P1-10 are therefore superseded
and are not applied. Awaiting critic pass 5. Not approved. The architect's role cannot write files;
a writer applies each revision and re-verifies every `file:line` citation it adds. Citation
corrections are listed in §9, revision changes in §10.

**Rev 5.1** (writer, 2026-09-17) reconciles files 01 and 05 with the closed modding design and the
owner's zero-overhead requirement (§10, rev 5.1 changelog). It is not an architect patch; critic
pass 5 reviews it together with rev 5. Files 02–04 are unchanged; the edits they owe are listed in
05 §5 RM-3.

**Provenance.**
- **Trees read:**
  - `[J]` = `D:/wt/joltab`, `merge/ke16-into-ecsnative` @ `d552be05` (read-only).
  - `[M]` = `D:/claude/BoykoEngine`, `feat/multi-paradigm-render`. The architect read it @
    `f2691132`; at the writer's verification (2026-09-17) its HEAD is `8a78ef6d` (ledger rev 4
    committed). The owner has uncommitted edits there.
    - `docs/memory/ALLOCATOR-DESIGN-SPACE.md` is one of them: HEAD holds a 506-line rev 1, and
      rev 2 through 2.4 exist only in the working copy (3,864 lines). Every allocator citation in
      this plan is to that working copy.
  - `[G]` = `D:/wt/_graph/refactor-census.md`, which records the refactor tree `D:/wt/refactor`
    @ `d552be05`.
  - Branch heads were read from `[M].git/refs/heads/*`, `.git/packed-refs` and
    `.git/worktrees/*/HEAD` on 2026-09-17, and re-read by the writer with read-only git commands.
  - `[R]` (`D:/wt/reflect`) and `[U]` (`D:/wt/ui`) were **not** re-read. Claims about them are
    cited through the design that quotes them.
- **Tools.** The architect had no shell, so graphify was not run, and no cargo, git or timing
  command was run. Line numbers come from Read/Grep on the tree named in each citation. The writer
  ran only read-only git commands (§9); no cargo build and no timing.

## 1. Goal

**What the plan delivers:**
- **One engine.** Every runtime datum lives in a kernel storage form: the `ComponentPool` family,
  on `boyko_memory` reservations. Every runtime loop is a system on the one scheduler.
- **Kernel features are built once.** Each capability a subsystem needs lands as one kernel
  feature, with one name, one contract and one owner.
- **Modding stays optional.** A modding layer can be added later as additive crates. A game that
  does not link them gets code generation identical to the engine without it.

**How performance is argued:**
- **Allocation removal is not a speed argument.** It is justified by the owner's ruling, by tail
  latency, and by the structural deny gate. The heap A/B was null: mi/sys = 0.998 / 1.023 / 0.992
  at W = 1 / 8 / 16 (`[M]docs/unification/CHECKPOINT-2026-09-11.md:33`). Allocator P0 withdrew
  speed as a justification (`[M]docs/memory/ALLOCATOR-DESIGN-SPACE.md:556-560`).
- **Throughput is claimed only where the ECS form removes serial work.**
  - The Jolt pyramid at W=8 is 2.47× slower than Jolt, with a serial fraction of 0.43–0.53
    (`[M]…/CHECKPOINT-2026-09-11.md:36`).
  - The broadphase runs an `AllPairs` O(n²) sweep of 769,420 pairs (`:45-46`).
  - Every such claim is gated by a quiet-window measurement (03 §5).

## 2. Target metrics

| Metric | Today (tree) | Target | Gate (03) |
|---|---|---|---|
| Ledger in-scope rows outside an ECS / kernel form | 2357 active, 691 (29.3 %) in ECS form, 1102 out of scope (`[M]docs/memory/RUNTIME-DATA-LEDGER.md:30`) | the count only decreases; end state 0 | UG-02 |
| Heap acquisitions, App frame (S0) | flat 2 per `Schedule::run` (`ALLOCATOR-DESIGN-SPACE.md:1124`) | 0 after D-M2 | UG-03 |
| Heap acquisitions, parallel pile step W=4 (S1c) | 302..339 (`:1129`) | 0 steady after D-M2 + D-M3 + P2 | UG-03, UG-05 |
| Committed floor per non-empty column | 384 KiB tracked / 128 KiB untracked (`[J]docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md:39,43`) | 12 KiB / 4 KiB | UG-04, UG-20 |
| Steady-window commits in Chunk/Table owners | not measured | 0 (Column reported) | UG-04 |
| Physics T(1)/T(8), pyramid | 1.71 (`[M]docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:94`) | ≥ 3.0 | MQ-03 |
| `pool.scope` sites in physics src | 4 (`:90`) | 0 | UG-02 physics slice |
| Host frame-loop World writes outside systems | 7 (`[M]docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:120`) | 0 | UG-14 |
| Exclusive UI systems | 13 | 1 with `UiPlugins`, 0 without (`:1839`) | UG-13 |
| Copies of one datum | 19 copies of 7 data (`:124`) | 7 | UG-02 |
| Component ids charged to physics builds by the scratch band | 128 (`[J]crates/boyko_physics/src/scratch_ids.rs:685`) | 0 (U-2) | UG-19 |
| Modding-off delta: pinned asm, `size_of`, `.text`, exports, startup census | n/a | 0 | UG-15 |
| Linked `boyko_demo` `.text` / `.rodata` per kernel rung | not recorded | recorded; +1 % budget per rung, above that a written reason | UG-16 |

## 3. Rulings this plan makes

Each ruling is decided by performance and names the gate that would overturn it. Details are in
01 §4 and 05 §4.

| # | Ruling | Numbers | Overturned by |
|---|---|---|---|
| U-1 | **The allocator's Heap class** (`Heap`, `HeapRef`, `HeapVec`, `HeapBox`, `HeapDyn`, `SortedMap`) **is deferred, not built.** Its clients take ledger forms: K7 spans, the erased record column, VmColumn tables, and sorted ScratchColumn pairs. | Per-frame cost equal (build-time clients). Heap: eager Miri 1.26 MiB per `EcsMaster`, VA 2 GiB per master, worst resident 204 KiB (`ALLOCATOR-DESIGN-SPACE.md:3164-3165, 2711`), plus new TB-sensitive unsafe surface. Forms: 4 KiB floor per column after D-M1, no new unsafe. | Ledger rev 5 finds a row that needs individually-freed, variable-size, non-`Copy` storage owned by `schedule`/`registry`/`master-table` and not expressible as KC-15/17/18; or modding Stage 3 needs mod-private freed memory (then the heap lives in `boyko_mod_host`) |
| U-2 | **Scratch cohorts are registry-free.** They use zero `ComponentId`s, and the physics scratch band is deleted. | +128 ids for physics builds; the ~90 physics scratch ids (`scratch_ids.rs:681`) and the 17–19 FrameGraph ids (`RUNTIME-DATA-LEDGER.md:874`) go to 0. Allocator P43's "1 id per element type" is superseded. | D-S2 finds an untracked-pool path that needs a registered id (then the P43 rule applies) |
| U-3 | **K6′ is re-filed against physics rev 5.** A group's release policy is `Immediate` \| `Chained` \| `Stamped`. `Stamped` never releases at removal and never takes the out-of-chain flush; it releases only through `GroupHead::release_dying_before(&ChainKey, horizon, visit)`. Span-typed group columns free their spans inside every kernel release point. | +4 B per dying entry for `Stamped` groups only; 0 bytes for `PhysicsBody` | correctness only: AS2's FIF-slot-reuse proptest |
| U-4 | **KF-33's form is the allocator's per-claimed-slot chunk lists** (`ScopeShared` in the block, `ChunkArena`, `SlotChunks`), not a per-thread mark/rewind arena | ≤ 390 KiB/pool at W16 D8 (derived, `:1787`); 0 heap calls per App frame | AL:M-A10 measures > 4 MiB per pool |
| U-5 | **KF-34 is narrowed to the injector ring.** The per-lane Chase-Lev deques stay crossbeam in v1. | injector = one 1520 B block per 64 outside pushes; lanes double once (`:1152`) | UG-03 shows lane growth, or the epoch `Local`, in a steady window after 1f |
| U-6 | **`ScopeShared` becomes the first cell of its `ScopeBlock`** (allocator P2). The ledger's frame-local placement is not used. | both are 0 allocations after warm-up; the block placement is covered by the Miri poison-write gate (P3) | AL:M-A2 shows a first-chunk layout cost |
| U-7 | **`TableSet` is deferred.** Fixed kernel tables become fixed-capacity `VmColumn`s, or inline arrays inside one existing reservation (KF-31). | 4 KiB floor per table after D-M1 | UG-20 records > 64 KiB of sub-page tables per `EcsMaster` |
| U-8 | **The owning column (KF-02/EK12) is a typed view over an untracked `ComponentPool` with drop glue.** The allocator's `DropColumn` primitive is not built. The drop glue is passed by value from `T` through KC-10's registry-free constructor, so the column uses **no `ComponentId`** (U-2). | the pool already stores `drop_fn` (`RUNTIME-DATA-LEDGER.md:909-910`; `[J]…/memory/component_pool.rs:235-237`), today read from the registry by id (`:284-288`). Passing it by value costs 0 ids, against today's route `register_asset_layout::<T>` (`[J]crates/boyko_ecs/src/ecs/core/asset/backing.rs:115`), which takes one id per type. No second droppable column type to cover under Miri | none expected. A KF-02 consumer that needs the column inside an archetype bundle takes a registered id, and the MS-03 D2 census counts it |
| U-9 | **Erased objects** (`Box<dyn System>`, `Box<dyn FnOnce>`, resource values) are stored in KF-07 records behind the allocator's `ErasedSystem {data, vtable}` handle. | append-then-drop-all; one indirection, as today. Bevy's proposed dedicated resource storage measured −47 % `get` ([#24058](https://github.com/bevyengine/bevy/pull/24058)) but was closed unmerged; the merged fix moved resources to sparse-set storage, `get` −10 % and `get_mut` −39 % ([#24077](https://github.com/bevyengine/bevy/pull/24077)) (§9 V-22) | AL:M-A1 dispatch floor regresses beyond its band |
| U-10 | **Modding:** load-only (no K-MOD-10 `remove_component_type`); v1 mod components are POD; crates are named per modding rev 6; the id bound is D2 plus allocator P39's skip-on-occupied mint; the refusal signal (item 2b) lives at the boundary. | removes one class-A kernel function; 0 capacity charge | owner rules that mods must unload (05 §8) |
| U-11 | **Allocator G6 and modding M-P1 become one gate, UG-15.** Pins are captured on each modding delta's parent. | — | none |
| U-12 | **Refactor rule:** split a file **before** the rungs that add to it; **do not** split a file a scheduled rung deletes or rewrites. | [G] 58 files, 171,250 lines (`[G]:31-35`) | owner scope override (§7 Q-4) |
| U-13 | **Kernel base = trunk `integ/unified`,** cut from `[J]` after Phase A plus the owner's steps | one tree for every gate and for ledger citations | — |
| U-14 | **Binary size is a per-rung gate; compile time is record-only.** | Bevy's revert PR for #20934 (#22915, approved, then closed unmerged) cited compile time +8–12 % and binary +5–7 % (RK-9; §9 V-21) | — |
| U-15 | **KF-12 (multi-target relation) is not built.** System-set edges are edge entities at schedule build. | build-time only | MQ-14 schedule-build time beyond its band |
| U-16 | **Default-excluded entities are an archetype flag,** tested at archetype match | 0 per row | UG-15 `query_ref_iter` pin moves |
| U-17 | **Physics NB2:** S1 and S6 become `pub(crate)` and are registered only through `PhysicsPlugin`; every non-panicking S6 exit reaches `close_chain` (`PHYSICS-ECS-UNIFICATION-DESIGN.md:3799`) | 0 runtime instructions: this is a visibility change. The rejected alternative, a runtime "chain systems registered together" check, costs one build-time scan per schedule and still cannot see a user system that calls neither. Gates: a UG-17 fixture (registering S1 from outside `boyko_physics` → E0603) and a U4 test that `chain_open` is clear after every S6 exit path | an owner scope ruling that games may assemble the physics pipeline from individual systems; S1/S6 then become `pub` behind a typed builder that registers both or neither |
| U-18 | **KF-45 stays** at the end of lane MEM (D-M6). Its speed case is windows-gnu only; its form is U-19. | 2 locked RMWs on one process-global line plus `FlsSetValue` per `thread_local!` read under rustc ≥ 1.98 gnu (`[J]docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:40-56, 82`). `target_thread_local` is off on gnu only (`:87-95`) | host moves to msvc (the owner's decision) → unification priority only; MQ-13 decides each host's arm (U-19) |
| U-19 | **KC-04 is one OS word per thread, holding the index + 1 of that thread's kernel record. There is no hashed key table and no materialisation step.** Records are 64-B rows of `THREAD_RECORDS`, a fixed 8192-row `.bss` static, claimed through `THREAD_BUSY`, a 128-word busy bitmap also in `.bss` (01 §6 items 1–2). A pool build claims its workers' slots in one batch, clamped so that 2048 slots stay free for every other thread, and each worker adopts its slot, so a worker is never refused. A record is zeroed at release, its only zeroing site, and is never inherited. The primitive lives in `boyko_threadpool`. `boyko_diag` and `boyko_log` keep private OS accessors, because their dependency rules forbid an edge to it. The thread-exit hook is std's TLS destructor on a guard (`EXIT_GUARD`) that only non-worker threads touch, so no census crate defines an `extern` fn. `ThreadPoolBuilder::build` first calls `thread_ctx::prepare()`, which allocates both TLS indices once per process (01 §6 item 4). **Committed arms:** windows-gnu takes the word arm with direct reads; msvc, Linux, Miri and loom take the portable arm (01 §6 item 3) | **windows-gnu hot read:** 1 RIP-relative load, 1 `gs`-relative load, 2 predictable branches, and one `shl` plus one `lea`; no lock, no call. This replaces 2 locked RMWs on one process-global line plus `FlsSetValue` per read under rustc ≥ 1.98 (`[J]docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:40-56, 82`). The call fallback costs what a 1.97.1 `thread_local!` read cost, one `TlsGetValue` (`:34-35`). **Portable arm:** 1 native TLS load, 1 null test, `shl` + `lea`, i.e. one load and one branch more than a native `thread_local!` field. No thread ever probes; rev 2's hashed lookup displaced a thread in ≈ 42 % of processes with 17 threads. **Per thread lifetime:** one `TlsSetValue` on the word arm. A non-worker thread also pays one cold claim and, on gnu, one `os-thread` System allocation plus one std `enable()` for `EXIT_GUARD`. **Per process:** 525,312 B of `.bss` virtual size; resident memory is ⌈peak claimed / 64⌉ × 4 KiB (3 pages at the gate host's 170-thread worst case, 01 §6 item 2). On gnu, the first pool build adds two `TlsAlloc` calls and two canaries | MQ-13, per host; each overturn is a re-seam in D-M6r. **(a)** windows-gnu's direct reads are not faster than the call arm beyond the band → `DIRECT_ARM = false` and `LANE_DIRECT_ARM = false`. **(b)** A host's record route is slower than native `thread_local!` beyond the band (the ledger's own overturn, `RUNTIME-DATA-LEDGER.md:1627`) → that host keeps `thread_local!` for these rows, recorded as a ledger exception with the number. **(c)** On msvc, the word arm with direct reads is faster than the portable arm beyond the band → `WORD_ARM = true` and `LANE_WORD_ARM = true` on msvc. **(d)** The owner rules that a fixed `.bss` static does not satisfy the one-allocator ruling for KF-45 → the records become a `VmColumn<ThreadRecord, TableOwner>`, reserved once and published by CAS, at +1 load per lookup for the base; loom M4, the check-then-write form, stays the negative model. If the TLS-index mapping does not hold, the per-process canary selects the call arm by itself (RK-14) |
| U-20 | **Entity ids are not made deterministic across runs or worker counts.** KC-36 makes apply, command and hook order, table rows and dense/group slots deterministic. Ids stay dependent on claim order, because `Commands::spawn` claims them on the worker during the phase (`[J]crates/boyko_ecs/src/ecs/core/system/params/commands.rs:169-173`) | 0 cost. Rejected: (a) placeholder ids resolved at apply, Unity's form (its command buffer returns temporary negative-index ids, §11). It changes `.id()`'s contract and costs one remap per placeholder reference at apply. (b) Per-system id leases carved by the dispatcher in index order. They replace the locked RMW per claim with a plain bump, but are deterministic only while no lease overflows, and a fresh-id remainder either leaks or needs inland-slot growth to keep the reservoir's F3 invariant (`[J]…/entity/entity_reservoir.rs:39-41`). No consumer needs id determinism: physics issues no command into any window, and its determinism statement is "for a fixed applied op sequence" (`PHYSICS-ECS-UNIFICATION-DESIGN.md:1279`) | Q-9 answered "ids must be identical"; or MQ-19 shows the reservoir line costing > 2 % of a spawn-heavy W = 8 frame. Then (b) is built for throughput and delivers determinism as a by-product, with its overflow counter pinned at 0 |

## 4. Phase map

```
A stabilise (bugs first, lane merges) ──> A8 trunk cut ──> B gates ──> C memory crate + refactor wave K
   ──> D kernel contract: lane MEM | lane STORE | lane ENG | R2 sweeps
   ──> E subsystems: physics (U4..S0) | engine (IN/HO/RE/AS/UI/SC/LG) | RF-P/RF-A/RF-U after their rewrites
   ──> F tail: R6 rows, G3/G4/G5 end-state gates, KE17 if MQ-05 says build
M modding: Stage 0 (owner) any time; Stage 1 kernel items inside D; Stage 3 only after D exit
RF-0 pilot and RF-R (free render/rhi/shaderdsl/sdf files) run in parallel from Phase A on
```

## 5. Patches the source documents owe once this plan is approved

| Document | Patch |
|---|---|
| `docs/memory/ALLOCATOR-DESIGN-SPACE.md` (rev 2.4 unreviewed; pass 6 pending, `:3858-3862`) | **Step DOC-1 (02 §2), reviewed by AP6 together with rev 2.4.** Rev 2.5: U-1, U-7, U-8, U-9 move Heap/TableSet/DropColumn/HeapDyn to revival form. The ByteColumn, ZeroInit, ChunkArena/SlotChunks, injector, 1f and G-ladder content stays. P39 and P40 stay as written; AP6 reviews them for the first time. §7 is re-pointed to file 05: K-MOD-3 and K-MOD-10 removed, G6 = UG-15, G6(f) = UG-15 leg (6) as redefined in 03 §6. §2.0's thread-context placement moves from `boyko_memory` to `boyko_threadpool` (U-19), which keeps G5's `#![no_std]` for `boyko_memory` (`:330`) intact |
| `docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md` (closed rev 5) | **Step DOC-2, reviewed by EP3.** Erratum E2: `Release` is an associated type with markers `Immediate` / `Chained` / `Stamped` (U-3, 01 KC-12); U1 deletes the scratch band (U-2); NB2 as U-17; N5 span freeing inside release points; **X-14 corrected** (`:1267`): "Command order, hook order and dense slots handed out in one window depend on completion-pop order (fixed by KC-36). Entity ids do not depend on the window: `Commands::spawn` claims them on the worker during the phase (`commands.rs:169-173`), so they depend on claim interleaving whatever the apply order (U-20)." |
| `docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md` (rev 3, pass 3 never ran) | **Step DOC-2, reviewed by EP3** (scope: the rev-3 patch, rev 4 and physics erratum E2). Rev 4: K6′ against physics rev 5 (U-3); registry-free EK1 (U-2); EK15b's redirect enqueues the `Pinned` removal instead of performing it (02 §4.4); prerequisites remapped to plan rung ids (02 §6). **EP3 must close before D-S3(iii), AS2 and every engine-sourced D-E rung.** D-E0 (physics P-§14), D-E18 and D-E19 (ledger KF-10, KF-09) are not engine-sourced and do not wait for EP3 (02 §2) |
| `docs/memory/RUNTIME-DATA-LEDGER.md` | rev 5 (B1): re-point to the trunk; KF-33/34 status per U-4/U-5; PC1 closed per U-1; ScopeShared per U-6; PC5 closed. **KF-45 (`:1624-1640`):** `:1628` currently reads "One VmReservation-backed column of 64-B records … Reached by slot through the platform thread-control-block pointer (an id -> slot table)". Rev 5 restates it as "one fixed `.bss` static table of 8192 64-B records and a 128-word busy bitmap, reached through a per-thread OS word that holds the record's index + 1 (U-19; 01 §6)". The precedent is KF-43's static-table form (`:1593-1605`); U-19 (d) is the owner's overturn. Per-row routes follow 01 §6 item 10: 9 record, 1 diag word, 2 OS thread id. The row's home moves from the memory library (`:1718`) to `boyko_threadpool`. The four remaining key and guard `thread_local!` cells are rowed as out of scope, each with its reason. KF-02 (`:898`): the `boyko_rhi_vulkan/src/memory.rs:728` row moves to D-E15, and the `boyko_utils/src/sparse_map/sparse_map.rs:10` row to D-R2d (02 §2) |
| `[J]docs/diagnostics/substrate/05-LADDER-GATES.md` (the D0 line item `:56-61`; DG12 `:133`), and the DG12 comment at `[J]crates/boyko_diag/src/lane.rs:105-107` | **Erratum D0-1.** D-M6 writes the code comment; DOC-2 writes the document. On a host where `LANE` lives in a word-arm OS slot (windows-gnu by default, 01 §6 item 3), the first `ThreadPoolBuilder::build` calls `boyko_diag::lane::prepare_lane()` once per process, even with diagnostics off. That call makes one `TlsAlloc`, runs one canary, and writes once to each of two shared statics, `LANE_IDX` and `LANE_TLS_OFF`. D0's "every one-time cost runs on the enable path" and DG12 (b)'s "no `boyko_diag` shared static" each gain one named exception. The reason is the one DG12 already gives for the TLS `LANE` cell: the two values identify the per-thread lane slot rather than holding diagnostic state, and they are written at pool build, not at process start. Nothing else in D0 changes: no calibration, no spare claim, no lane-buffer write, no session id. The DG12 leg that D-M6 adds (02 §2, D-M6 phase (g)) makes a document–code drift go red (critic pass 4, O1). *Writer check (§9 V-55): DG12's own reason for excluding `LANE` is that D1 mandates that write and that it costs 2 B of per-thread TLS and no `.bss` (`05-LADDER-GATES.md:133`). `LANE_IDX` and `LANE_TLS_OFF` are neither D1-mandated nor TLS; they are shared statics in `.bss`. The exception therefore rests on 01 §6 item 4's performance argument, not on DG12's existing reason.* |
| `docs/modding/MODDING-DESIGN-SPACE.md` (closed) | erratum for P6-6 citations; dispositions of the carried remarks are in 05 §5 |
| `[J]docs/MEASUREMENT-QUEUE.md` | add the MQ entries of 03 §5, including MQ-13's three host arms and MQ-19 |

## 6. Risks

| # | Risk | Evidence | Mitigation |
|---|---|---|---|
| RK-1 | The allocator design is unapproved, and U-1 reverses its Heap class. Rev 2.4 (P38–P45) has had no critic pass: P39 and P40 are its fixes for pass-5 C2 and W1 (`ALLOCATOR-DESIGN-SPACE.md:3845-3846`), and pass 6 is pending (`:3858-3862`). The table of findings that stand closed (`:3473-3486`) was written by critic pass 5 and covers passes 1–4 only | `:3473-3486`, `:3845-3846`, `:3858-3862` | DOC-1 writes rev 2.5, and AP6 (critic pass 6) reviews rev 2.4 and rev 2.5 together. **Until AP6 closes, C1, D-S1(i), D-S2 and UG-15 leg (6) do not start** (02 §2, document steps). No part of this plan treats P39 or P40 as closed. Items closed by passes 1–4, e.g. P15–P17, are not held by AP6, but every rung that uses them is also held by C1 |
| RK-2 | The engine design's rev 3 was never re-critiqued, and it was filed against physics rev 3 | `[M]CHECKPOINT-2026-09-11.md:55`; physics rev 4/5 changed K6 (`PHYSICS-ECS-UNIFICATION-DESIGN.md:2449-2463, 3330-3343`) | DOC-2 writes engine rev 4 and physics erratum E2, and EP3 (engine critique pass 3) reviews them. **EP3 must close before D-S3(iii), AS2 and every engine-sourced D-E rung.** D-E0, D-E18 and D-E19 have non-engine sources and start when their prerequisites land (02 §2) |
| RK-3 | Page-cache bit flips on a workstation without ECC | `CHECKPOINT-2026-09-11.md:236-238` | Owner memory test (Q-6); every red is re-run once before triage; receipts record the target dir |
| RK-4 | A number measured on a tree without the fix | `:198-200` | Every MQ receipt carries `git merge-base --is-ancestor <fix> <tree>` |
| RK-5 | Refactor and kernel rungs edit the same files | [G] §2.4, §5.4 | U-12 plus the file-lock table (02 §4) |
| RK-6 | A file split changes CGU partition and moves hot-loop codegen | cgu=1 under fat LTO costs `query_ref_iter` 17.4 % (modding `:2101-2103`); P6-1 (`MODDING-DESIGN-SPACE.md:2425-2438`) | UG-15 strict on every refactor commit: the kernel pins plus the wave's own leg (2-RF) pins (03 §6); a moved pin holds the commit for an MQ-12 entry |
| RK-7 | Owner-dirty files overlap trunk work (44 paths, 14 `.rs`, as [G] recorded them at `d1e77f4f`) | [G]:331-348. **Writer check, 2026-09-17:** `git status --short` in `[M]` now lists 39 entries (59 paths with `-uall`), 16 of them `.rs`: the 14 modified sources plus the untracked `crates/boyko_app/examples/{_hud_probe,playground}.rs` | Q-5 |
| RK-8 | Inherited reds at the base | `check_hotpath_exceptions.py` red at `[J]crates/boyko_ecs/src/ecs/core/entity/entity_reservoir.rs:405` ([G]:835); `production_reachability_census` red at `[J]crates/boyko_physics/src/row_identity.rs:278` (source: session record of 2026-09-17, which records it red on the pushed `b74f7ee8`; [G]:835 records only the hot-path red) | A1 and A4a (A4b fixes the flaky `QueryTypeId` race) |
| RK-9 | A long-lived mega-branch gets reverted | Bevy [#20934](https://github.com/bevyengine/bevy/pull/20934) (145 commits, merged 2026-02-10) drew an approved revert, [#22915](https://github.com/bevyengine/bevy/pull/22915), citing compile time +8–12 % and binary size +5–7 %. The revert was closed **unmerged** on 2026-02-18, and no revert commit reached Bevy `main` by 2026-03-15; the regression was worked off in follow-ups instead (e.g. [#22919](https://github.com/bevyengine/bevy/pull/22919), which hoisted generic code out of `register_component`). GitHub API, read 2026-09-17 (§9 V-21) | No rung is XL: D-S3 is three rungs, D-S3(i)–(iii), each green on its own (02 §1 rule 5) |
| RK-10 | Apply-window changes break the EM2′-K rule | `[J]docs/MEASUREMENT-QUEUE.md:93-98` | Any rung touching `apply_window_drain` (`[J]crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:783`) or `may_defer` (`:167`) carries the EM2′-K test |
| RK-11 | Disk space | D: ~36 GB, C: ~41 GB (session record) | At most 3 code worktrees (opened at A8, 02 §4.1) plus refactor; target dirs only under `D:/wt/_targets`, reused per worktree; the `tls-*` probes are pruned after review (04 §5) |
| RK-12 | Id budget after the UI and reflect merges plus the new group columns | 512 slots (`[J]…/component_registry/mod.rs:63`) | AL:M-A13 / MD:M-K3 census (structural) at B2, after D-S3(ii) and after D-S3(iii) |
| RK-13 | Parallel rungs edit shared kernel files where the order of edits changes behaviour | `delete_entity_core` (`[J]…/ecs_master/entity_api.rs:980`) is edited by D-S3(ii) (`unbind` at the two `deallocate_entity` sites, `:1091` and `:1102` at `d552be05`; physics cites `:1088`, `:1099` from `d11962a9`) and by D-E2 (the despawn redirect). `GroupRef` membership is only a `debug_assert!` (`PHYSICS-ECS-UNIFICATION-DESIGN.md:2729`), so a wrong order is silent in release | Per-rung, per-file locks; fixed orders; the fixed step order inside `delete_entity_core` (02 §4); D-E2's combined red-first test |
| RK-14 | **Hazard 1.** On windows-gnu, the direct reads of KC-04 and `LANE` assume that a `TlsAlloc` index below 64 addresses `TEB.TlsSlots[index]`. winternl places `TlsSlots` after 12·8 + 8 + 399·8 + 1952 bytes (`+0x1480` on x64), but Microsoft calls the index "an opaque value" and says to call `TlsGetValue` rather than read the TEB (§11). **Hazard 2.** On windows-gnu, std's OS-key backend reads every `thread_local!` through its own `TlsGetValue` index (`RUSTC-198-WINDOWS-GNU-TLS.md:34-41, 87-92`). By the first pool build, the 64 fast indices may already be taken, and the canary then fails | Microsoft `TlsAlloc` and `TEB` pages; the TLS document | A per-process canary (01 §6 item 3) enables direct access only when `index < 64` and the mapping holds in both directions. Otherwise, the documented `TlsGetValue` call arm serves every read, and `TLS_OUT_OF_INDEXES` falls back to the portable word. `prepare()` runs at the first pool build, before workers can race for indices (01 §6 item 4). UG-20 reports the arm in use and both indices; MQ-13 prices the call arm |
| RK-15 | Thread-table capacity (critic pass 4, W1): a fixed table can be exhausted, and at rev 4 a refused worker panicked | Every worker writes pool thread state first (`[J]crates/boyko_threadpool/src/worker.rs:46, 54, 59, 69`). `App::new()` builds `available_parallelism()` workers (`[J]crates/boyko_ecs/src/ecs/core/app/app.rs:201-203`). `boyko_render`'s lib tests build a default `App` in 10 tests, through 8 `App::new()` call sites (`[J]crates/boyko_render/src/light.rs:2214-2227` is a helper three tests share); 10 × 32 + 10 = 330 threads on a 32-logical host (critic pass 4 counted 8 tests and 264 threads; §9 V-54) | 8192 slots. Workers are batch-claimed at build and clamped, keeping a 2048-slot foreign reserve, so no worker is ever refused. A build with no free slot panics on the building thread. Only a foreign thread can be refused, and only when the table is full; it reads `DETACHED` and panics with a code on its first write. The sizing basis is in 01 §6 item 2; the test at the bound is D-M6 phase (f); UG-20 reports clamps, dips, refusals and the peak |

## 7. Owner questions (values and scope only)

| # | Question | Default if unanswered |
|---|---|---|
| Q-1 | Modding Stage 0: (i) trust model — curated native mods or untrusted/sandboxed; (ii) must mods survive engine patch releases; (iii) who builds a mod; (iv) scope — may mods add component types, or only data prototypes | no modding code beyond Stage-1 kernel items (05 §7) |
| Q-2 | Is mod unload or hot reload a product requirement? The plan rules load-only (U-10). | load-only |
| Q-3 | Quiet-machine windows. MQ-01 (per-stage pyramid) and MQ-02 (SP-1) decide the physics rung order. | physics runs the U track first; P1/P2 follow |
| Q-4 | Refactor wave K (~18 kernel files, ~40k lines) lands **before** kernel code rungs (U-12), with one stated exception: D-M0, the packing rung, lands before C1 and RF-K1 so its line-cited steps do not rot (02 §2). Accept the delay, or interleave kernel rungs with file locks and accept rebase churn? | wave K first |
| Q-5 | Commit or drop the uncommitted paths in the main checkout (44 paths / 14 `.rs` per [G]; 59 paths / 16 `.rs` on 2026-09-17, RK-7). They block the trunk cut, 4 census files and K4. | trunk cut waits |
| Q-6 | Memory test of the workstation (two bit flips this week) | gate results stay provisional |
| Q-7 | Timing of the msvc switch. It moves KF-45's priority and the Miri toolchain. | gnu stays the gate host |
| Q-8 | `Scope::spawn_batch` surface (open scope question from 2026-09-09). D-M2 is compatible with either answer. | unchanged |
| Q-9 | Must entity ids be identical across runs and across worker counts, e.g. for lockstep networking or id-keyed replays? KC-36 makes apply order, hook order, table rows and group slots deterministic, but not ids (U-20). | no |

## 8. Readiness checklist (condensed)

| Area | State |
|---|---|
| Goal | functional + performance ✔ |
| Metrics | §2 ✔ |
| Decisions | justified with numbers and overturn gates ✔ (01 §4, 05 §4) |
| Data structures | owned by the source designs; the new layout pins are named in 01 ✔ |
| Threading | 01 §6 ✔ |
| Edge cases | 01 §7 ✔ |
| Integration | 04 ✔ |
| Validation | 03 ✔ |
| Items not established | the contents of lane `fix/ke13-ke14` (1 commit, 25 `.rs`; not read). The ancestry checks of 04 §2 and the containment of `fix/inherited-red-gates` and `fix/miri-protector-arming` were established by the writer (§9, 04 §1). |

## 9. Citation verification record (writer, 2026-09-17)

Every `file:line` citation kept in files 00–05 was read in the tree it names. Citations that held
are unmarked. The ones below were corrected in place or annotated.

| # | Where | Finding | Action |
|---|---|---|---|
| V-1 | provenance | `[M]` HEAD is `8a78ef6d`, not `f2691132`; the allocator design's rev 2–2.4 are uncommitted | provenance updated |
| V-2 | 00 RK-6 | 17.4 % is at modding `:2103` (and `:2431`), not `:2104` | cited `:2101-2103` |
| V-3 | 00 RK-8 | [G]:835 records only the hot-path red; the reachability-census red is a session record | source named |
| V-4 | 00 RK-10 | `apply_window_drain` is defined at `[J]…/schedule.rs:783`; `:167` is `may_defer` | both cited |
| V-5 | 00 U-18 | the TLS doc was cited without a line | `:82` added |
| V-6 | 00 RK-7, Q-5 | the dirty-path count has moved since [G] | current count added |
| V-7 | 01 KC-12 | Erratum E1's heading is at physics `:3778`; `:3780` is its body | cited `:3778-3780` |
| V-8 | 01 KC-01 | the allocator's `COMMITTED_BYTES` at `:2696` has five owners (`Column \| Heap \| Chunk \| Frame \| Table`); this plan's three follow from U-1 and P34 | annotated |
| V-9 | 01 R-A | TSV lines 557–559 are `ArchetypeBundle` rows whose destinations are `NEW:BoundedArray`, `NEW:VmColumn::grow_filled` and `VmReservation-raw`; lines 2338–2340 are `SparseMap` rows → `NEW:VmSparseMap<U>`; lines 784–794 carry `NEW:VmBitSet`, `VmReservation-raw`, relation `NEW:VmJagged<T>` and `NEW:DynArena` | destinations quoted |
| V-10 | 01 R-D | allocator `:3484-3485` lists passes 3 and 2 only; the standing-closed table is `:3478-3486` (passes 1–4) | cited `:3478-3486` |
| V-11 | 02 D-M1 | 393,216 B is the packing plan's **G3** (OS truth) arm; 3,145,728 B is **G1** (model) | labels fixed |
| V-12 | 02 RF-K2/K3 | `wc -l` in `[J]` reads `archetype.rs` 2573 and `ecs_master.rs` 1917; [G] §1.1 records 2574 / 1918 | [J] values noted |
| V-13 | 03 UG-15 leg (1) | the six-crate census list is at modding `:1851-1855`, not `:1828`; `boyko_memory` is this plan's addition (the crate does not exist yet) | cited and annotated |
| V-14 | 04 §1 | `merge/ke16-into-ecsnative` has an upstream and is 3 commits ahead of it (`7c327121`, `fc7eb127`, `d552be05`); `b74f7ee8` is pushed | row corrected |
| V-15 | 04 §1, §5 | `feat/ecs-native-storage` is **not** contained in `d552be05`: its one commit `ad0ebea4` adds `tests/physics_vec_side_store_census.rs` (1,172 lines), the file physics R0 restores, and `[J]` has no such file | marked keep-until-R0; removed from the prune list |
| V-16 | 04 §1, §5 | `fix/inherited-red-gates` is contained in `d552be05` | "unknown" replaced |
| V-17 | 04 §1, §5 | `.claude/worktrees/trusting-ramanujan-0f8927` (branch pushed) is 3 commits / 13 `.rs` ahead of `d552be05`; `git cherry` finds 2 of them (`6a9871bb`, `867dd734`) not patch-equivalent | marked review-before-prune |
| V-18 | 04 §1 | ahead counts and merge bases re-read; `docs/ab-register-sync` is 5 / 1, `chore/doc-gates` 10 / 3, `fix/census-post-lto-object` 1 / 3 | exact numbers entered |
| V-19 | 04 §3 O2 | main is 6 commits past `0c18acfc` (`8a78ef6d` added); `f2691132` and `8a78ef6d` are not ancestors of `merge/ke16-into-render`, so `--ff-only` fails. Main's side past `0c18acfc` is docs-only; the merge side is 74 commits and carries code, so "both sides are docs-only" was wrong | text corrected |
| V-20 | 05 §4 | the write-once citation `:133-139` is in `MODDING-DESIGN-SPACE.md` | file named |

**Rev 2 re-verification (writer, 2026-09-17).** Every citation that rev 2 adds was read in the tree
it names:
- `[J]` at `d552be05`: `entity_api.rs`, `archetype_flags.rs`, `archetype_master.rs`,
  `component_pool.rs`, `component_pool_bundle.rs`, `constants.rs`, `schedule.rs`,
  `component_registry/{mod,tags}.rs`, `ecs_master.rs`, `archetype_bundle.rs`,
  `boyko_threadpool/src/worker.rs` (unchanged since `d11962a9`), the `boyko_ecs` benches
  `ke17_apply_window` and `phase9_scheduler`, the `boyko_physics` benches `jolt_parity_pyramid` and
  `colored_solve`, and the packing plan;
- `[M]` working copy: the allocator, engine, physics, ledger and modding designs;
- `[G]` `:89` and `:621`;
- Bevy #20934, #22915, #22919, #24058 and #24077, through the GitHub REST API.

The findings below were corrected in place or annotated; every other new citation held.

| # | Where | Finding | Action |
|---|---|---|---|
| V-21 | 00 RK-9, U-14 | Bevy #20934 (145 commits) was merged on 2026-02-10. Its revert, #22915, was approved but closed **unmerged** on 2026-02-18 (`merged: false`), and Bevy `main` has no revert commit between 2026-02-10 and 2026-03-15; #22919 (merged 2026-02-12) cut `register_component` codegen instead. "Reverted in #22915", and rev 1's "then reverted", were false | both rows re-worded; the risk itself stands |
| V-22 | 00 U-9 | #24058 ("Resource storage", the −47 % `get` figure) was closed unmerged on 2026-06-26. The merged change is #24077 (2026-05-04): resources on sparse-set storage, `get` −10 %, `get_mut` −39 % | row re-worded |
| V-23 | 01 §7 `ArchetypeFlags` | bits 0–12 are declared at `archetype_flags.rs:29-98` (bit 0 is `:29`), not `:40-98`. The observer recompute that clears bits is `archetype_master.rs:875-904` (the `clear` call is `:897`); `:134-143` is `ArchetypeFlags::clear` itself | both cited |
| V-24 | 01 KC-36 | the apply-window gate is `schedule.rs:678-680` (`pending > 0 && (pending == running \|\| running == 0)`); `:811-814` is the SAFETY comment that restates it | gate cited |
| V-25 | 02 D-S3 | physics's `schedule.rs:841` (`PHYSICS-ECS-UNIFICATION-DESIGN.md:2043`, the debug reconciliation site) was read at `d11962a9`, where it is `apply_window_drain`'s last statement (`pending_fetch_sub`); at `d552be05` that statement is `:859`. Physics's `entity_api.rs:1088`, `:1099` are likewise `d11962a9` lines (`:1091`, `:1102` at `d552be05`, as RK-13 says) | annotated |
| V-26 | 02 A8, 04 §2 step 10 | the owner's 14 modified `.rs` files (`git status` in `[M]`) also touch `boyko_physics` (`lib.rs`, `plugin.rs`: A1's crate), `boyko_log` (`codes.rs`) and `iters/query/{par_chunk,query_view}.rs` (D-S4's files), not only the four crates named | named |
| V-27 | 02 D-M0 | packing S0–S4 span `POOL-SUBGRANULAR-PACKING-PLAN.md:458-542`; `:458-494` ends inside S1. S2–S4 (`:498-541`) also edit files outside D-M0's lock set: the `scratch_column.rs` prose, `benches/d6_commit_vs_faults.rs`, and the documents `docs/SYSTEMS.md` (owner-dirty), `docs/MEMORY-SYSTEM-AUDIT.md`, `docs/OPEN-QUESTIONS.md` and `docs/gaia/DECISIONS.md` | range corrected; the lock set is left to the cut (02 §4.2) |
| V-28 | 04 §1, §5 | `7c327121` and `fc7eb127` are also on `origin/fix/rejected-gpu-upload-leak` (`git branch -r --contains`); only the merge commit `d552be05` is local-only | the joltab and prune rows state it |
| V-29 | 05 MS-13 | `MODDING-DESIGN-SPACE.md:1829` lists nine refusal paths; `mod.rs:938` and `:996` are the two that P39 rewrites | annotated |
| V-30 | 02 D-S2 | the ripgrep count holds: 281 matching lines (327 matches) in 17 files. The three `boyko_ecs` files are `memory/component_pool.rs` and the tests `scratch_column.rs` and `scratch_column_miri.rs`, which the touch set does not list | annotated |
| V-31 | 01 KC-10 | `component_pool_bundle.rs:431` is under `#[cfg(debug_assertions)]`, and it is the only call of a pool's `component_id()` in `[J]crates` (ripgrep) | annotated |

**Rev 3 re-verification (writer, 2026-09-17).** Every citation that the rev-3 patch adds was read
in the tree it names:
- `[J]` at `d552be05`, whose worktree was clean apart from A1's two untracked physics tests;
- the `[M]` working copy;
- `[G]`;
- the §11 web pages, fetched the same day.

The writer ran graphify once (a stale graph with off-target hits), then used ripgrep and read-only
file reads. It ran no cargo command, no git write and no timing.

| # | Where | Finding | Action |
|---|---|---|---|
| V-32 | 00 U-8; 01 KC-10 | These hold: `component_pool.rs:235-237` (the `drop_fn` field); `:284-288` (`new` reads `drop_fn` from the registry by id); `:46-54` and `:80-94` (`PoolBacking`, whose `Device` arm is a `Box`, so the enum stays at `VmReservation`'s 16 B); the 128 / 144 B pins at `:55-64`. So does `asset/backing.rs:115` (`register_asset_layout::<T>`, which mints one id per `TypeId` and caches it) | none |
| V-33 | 01 KC-10 lazy state; 02 D-S2 | `vm_column.rs:83-89` (dangling base until the first grow) and `:103-105` (`vm: Option<VmReservation>`) hold. `VmReservation::UNRESERVED` does **not** exist at `d552be05`: the struct is `vm.rs:85-97`. Its `Drop` (`:263-297`) calls `VirtualFree`, `munmap` or `dealloc` with no length test | annotated in 01: D-S2 adds the constant and the zero-length skip on all three arms |
| V-34 | 00 U-20; 01 KC-36; 02 D-E0; 03 MQ-19 | These hold: `system/params/commands.rs:169-173` (`spawn` claims on the worker); `entity_counter.rs:200-219` and `:212-218` (claims come from the stack until it is empty, then fresh ids are minted); `entity_reservoir.rs:39-41` (F3), `:67-90` (the atomics share line 0) and `:160-190` (`try_claim_recycled`, `mint_fresh`); `schedule.rs:678-680`, `:809` (`running.set(i, false)`), `:849` (`pred_remaining` decrement), and `:1079-1132` (the dispatch scan, whose loop has no worker-count term) | none |
| V-35 | 00 U-18, U-19, S-1, S-4; 01 §6 | These hold: the TLS doc `:40-56`, `:79-82` and `:87-95`; `boyko_diag/Cargo.toml:6-16`; `boyko_log/Cargo.toml:7-18`; `00-GOAL.md:220-228`. In `lane.rs`, `:27-32`, `:34-40`, `:38-40`, `:49-53`, `:139` and `:153-155` hold. In `tls.rs`, `:159` (`LaneDeposit` = 2 pointers + 8 = 24 B), `:169`, `:196` and `:205` hold. `hooks/scope.rs:4-17` and `:31` hold, as does `sync_out.rs:69-75`. The 12 KF-45 rows at ledger `:1632` are the 12 `thread_local!` statics at `[J]`, and 01 §6 item 8 splits them 8 + 1 + 2 + 1. `EcsThreadFields` has one field per ECS static. At `propagate.rs` the F2 reason is in the module header (`:8-13`), not beside the static | cites added to 01 §6 item 8 |
| V-36 | 01 §6 item 8, `BUILDING` | `required.rs:153` (`RefCell<Vec<usize>>`) and its unwind guard (`BuildingGuard`, `:166-200`) hold. The patch's condition, "the DFS calls no code outside `required.rs` between push and pop", fails at `d552be05`: `(direct.id_fn)()` at `:312`, between the push at `:301` and the pop, is `B::component_id`. `build_required_plan` is reached only through `get_required_plan` (`:259-260`) and its own recursion (`:320`) | annotated; the architect decides which test the cut applies |
| V-37 | 02 D-E2; §4.4 | "94 call sites in 50 files" holds as a count of matching lines. One hit is a doc comment. An approximate regex finds 23 statement calls that discard the value. 44 of the 50 files are tests or benches; the other 6 files hold 8 sites. `hierarchy/commands.rs:108-109` (`enqueue_child_of_removal` pushes onto `deferred_hook_queue`) holds | annotated; the conclusion stands |
| V-38 | 02 D-S3(i) | "24 non-test files" holds for `[J]crates` with every `tests/` directory excluded; 38 files including tests. The 24 cross three crates beyond `boyko_ecs`: `boyko_macros/src/component.rs` (already counted), `boyko_render/src/occlusion_marker.rs` and `boyko_serialize/src/load.rs` | annotated |
| V-39 | 00 RK-1, §5, U-17, U-20, §10; 01 KC-16 | These `[M]` citations hold. **Physics:** `:1267` (the X-14 row), `:1279` (the determinism statement), `:3799` (NB2). **Allocator:** `:330` (G5); `:3473-3486` (the table of earlier findings, inside Part VII's pass-5 log that starts at `:3312`; its rows cover passes 1–4, and V-10's `:3478-3486` is a sub-range of it); `:3845-3846` (pass-5 C2 → P39, W1 → P40); `:3858-3862`. **Ledger:** `:898` (9 rows; the 7 `boyko_ecs`/`boyko_render` rows, `boyko_rhi_vulkan/src/memory.rs:728` and `boyko_utils/src/sparse_map/sparse_map.rs:10` were each re-read in `[J]` and hold, as do 02 §4.3's `ecs_master.rs:953` and `dense_registry.rs:78`); `:912`; `:1552` (`gpu_column.rs:532,563`, which hold in `[J]`); `:1624-1640`; `:1627` (the `worker/body_1us_tasks_64W` overturn); `:1718`. **Checkpoint:** `:94` (defect 5) | none |
| V-40 | 02 §5 | `[G]` §1.1 and §1.2 list 58 + 4 files, and the wave table's 62 rows match `[G]`'s numbers and paths one for one. `[G]` §2.4 is `:420-487`. All 263 `aether_lang` rows of the ledger TSV are out of scope as compile-time proc-macro code that is never linked into the shipped binary | none |
| V-41 | 03 MQ-13 | `worker/body_1us_tasks_64W` is a criterion id built at `[J]crates/boyko_threadpool/benches/ke16_nested_scope.rs:291` (group `worker`). `phase14a_hooks_gate` is `[J]crates/boyko_ecs/benches/phase14a_hooks_gate.rs`. "Reads the depth twice" means `DeferredScopeGuard`'s enter and drop (`hooks/scope.rs:66`, `:74-78`) | cites added |
| V-42 | 00 §11 (web) | **Microsoft `TlsAlloc`:** a new index's slots start at zero, and the index is to be treated as opaque, not as an array index. **Microsoft `TEB`:** the field list gives `TlsSlots` at 12·8 + 8 + 399·8 + 1952 = 5248 = `0x1480` on x64; the page tells applications not to access the structure directly and to call `TlsGetValue` instead. **Miri's TLS shim:** runs both `FlsAlloc` destructors and `.CRT$XLB` functions. **Unity samples:** temporary ids with negative indices; playback sorted by sort key. **Unity 6.4 page:** sort keys only. **rust#99682:** open; `cargo check` skips post-monomorphisation errors. **trybuild docs:** do not say whether fixtures are checked or built. **NtDoc:** lists `TlsSlots` without an x64 offset, so the offset rests on Microsoft's definition. **cbloom:** not re-fetched | none |
| V-43 | files 00–05, text the patch did not touch | These passages still carry rev-2 wording that rev 3 changed elsewhere. **`RELEASE` as a const:** 01 R-C (`A per-group const RELEASE`) and 01 §7's KC-12/KC-13 row (`G::RELEASE`). **Bare `D-S3`, now three rungs:** prerequisite cells D-S4, D-S5, D-E2, D-E7, D-E9, U4 and AS2; the ENG lines of the 02 §3 DAG; 00 RK-12; 05 §5 R3-1. **"Two" red controls, now four:** 02 B3 ("M-P1's two red controls"), 02 D-S1(ii) ("both red controls"), 05 §6. **Parent `D-S1(i)`:** 05 §7 stage 1 ("strict UG-15 against D-S1(i)"), where 02 now says "its cut commit". The rev-2 changelog row W4 (`D:/wt/k-phys`) is history and stays | not changed (outside the brief); listed for critic pass 3 |

**Rev 4 re-verification (writer, 2026-09-17).** This covers the citations that the applied part of
the rev-4 patch adds (§10, rev 4 changelog). Each was read in the tree it names:
- `[J]` at `d552be05`, whose worktree was clean apart from A1's two untracked physics tests;
- the `[M]` working copy, with the ledger TSV as committed at `8a78ef6d`.

The writer ran graphify once (a stale graph with off-target line numbers), then used ripgrep,
read-only file reads and read-only git. It ran no cargo command, no git write, no timing and no web
fetch.

| # | Where | Finding | Action |
|---|---|---|---|
| V-44 | 02 §2 mode table (D-M6, D-S7, D-E2); 03 MQ-20 | These hold. `swap_remove/10k` is a loop of `ecs.delete_entity` at `[J]crates/boyko_ecs/benches/swap_remove.rs:104-106`, inside `bench_swap_remove` (`:93-114`); `criterion_group!` and `criterion_main!` are `:116-117`. `delete_entity` (`entity_api.rs:814-822`) calls `delete_entity_core` (`:815`) and then `drain_deferred_hook_queue` (`:820`). `delete_entity_core` enters its guard at `:984` and drops it at `:1110`. The drain (`ecs_master.rs:663`) reads the depth at `:668`, enters its guard at `:690`, and tests `deferred_hook_queue.is_empty()` at `:714`. The queue is a `CommandQueue` (`ecs_master.rs:264`), whose `is_empty` is `#[inline]` (`commands/command_queue.rs:125-128`). That it is actually inlined into the drain is a codegen fact, and was not checked (no build) | none |
| V-45 | 03 MQ-13; 02 D-M6 | These hold. `install` saves `(current_worker_id, lane())` and sets `LANE_DISPATCHER` at `thread_pool.rs:253-255`; the guard's `Drop` restores both at `:371-374`. `dispatcher/body_1us_tasks_W` is a real criterion id: `BenchmarkId::new("dispatcher", …)` at `ke16_nested_scope.rs:288`, `param = body_{body}_tasks_{mult}` at `:279`, `"1us"` in `BODIES` (`:77`) and `"W"` in `TASK_MULTIPLES` (`:85`). `dispatcher_wave` calls `pool.install` once per iteration (`:142-144`). The TLS doc's §6 (`:269`) requires a receipt to name the rustc commit hash. **No `prepare()` exists on the `ThreadPoolBuilder::build` path** at `d552be05` (`thread_pool.rs:664`); the only `fn prepare` in `boyko_threadpool` is a task constructor (`scope.rs:1194`). D-M6's `LANE` test names `prepare()` as new code, and its definition is in the part of the patch this writer did not receive | noted; listed for critic pass 4 |
| V-46 | 01 §6 item 8 (P1-8, not received); 03 §3 test 2 | These hold. `RequiredBuilder` is a `pub struct` (`component.rs:299`); `require` is a `pub fn` whose signature is `:323-327` (the closing `) {` is `:327`). `Component` is a safe trait (`pub trait Component`, `:37`). In `required.rs`: the `allow` and the `RefCell` import are `:15-16`; `BUILDING` is `:152-153`; `build_required_plan` is `:285`, with its memoized read at `:292`, `BuildingGuard::push` at `:301`, the `out` `Vec` at `:304` and `(direct.id_fn)()` at `:312` | none |
| V-47 | 02 §4.3 `codes.rs` row | Partly holds. The filter note is `[J]crates/boyko_log/src/codes.rs:1400-1415`: the rung-2 miss at `:1400-1405`, the rung-13 repeat at `:1407-1411`, the rule at `:1413-1415`. The pin test starts at `:1393` (`#[test]`; the fn is `:1394`) but ends at `:1528`, so `:1393-1430` is only its head and the first rows of `LIVE` (`:1416`) | cited `:1393-1528` |
| V-48 | 03 MQ-09, §6 legs (6)–(7); 05 §6 | These hold in `[M]`. **Modding `:2135`** is M-A3: the startup cost of the directory scan and manifest parse with 0 mods, which sets P2's budget. **`:2146`** is M-P1. Its pins (4) and (5) are the executable's exported-symbol count and its `.text` size, and the same row notes that a windows-gnu executable's export directory is empty with or without a `#[no_mangle]` item. **`:2147`** is M-10. Its leg (2) finds `NEXT_ID` and `register_new` **local** under fat LTO; its leg (3) finds a `#[no_mangle]` fn in a bin still global, with no PE export entry. **Allocator `:3825`** is the paragraph that states (f)'s blind spot: it "cannot detect a size regression shared by both arms". The (f) row itself is `:3823`; its arms are one that declares `boyko_modding` and never calls it, and one that removes the dependency | none |
| V-49 | 02 §2 mode-table bullet | Holds. `runtime-data-ledger.tsv` (2,358 lines: a header and 2,357 rows, clean at `8a78ef6d`) has 0 lines matching `entity_api`. **The claim is narrower than it may read, though.** The same TSV has 5 rows in `ecs_master/ecs_master.rs` and 3 in `commands/command_queue.rs`, one of them `CommandQueue::bytes`, the field `is_empty` reads. Both files hold code in the `swap_remove/10k` body (V-44). An R2 sweep that migrates one of those rows reaches that body through the file map, not only through `EcsMaster`'s layout map. The bullet's last sentence is therefore exact only for `delete_entity_core` itself | listed for critic pass 4 |
| V-50 | text in P0 / P1, not received | These hold. `STEAL_EMPTY_GATE` is defined at `worker.rs:39`, with its `cfg!`-constant rationale at `:36-38` (its doc comment starts at `:25`). `SPARE_OWNER` is `lane.rs:134`, and `LANE`'s `thread_local!` is `:136-140`. `tls.rs:55-80` holds D6 (`:55`), D7 (`:69`) and D8 (`:77-80`) | none |
| V-51 | 00 §11 (web; P0-8) | P0-8 was never applied, and rev 5 supersedes it: §11 is re-emitted in full (rev 5, P5-00-8) | the rev-5 writer re-fetches every §11 page that rev 5 adds or extends |
| V-52 | 01 `BUILDING` row; 02 D-M6 row | Both held before this revision: the `BUILDING` row was 01 line 508, and the D-M6 row was 02 line 95. The D-M6 row is now replaced whole, and is line 96 after A0's insertion. The `BUILDING` row belongs to P1-8, which was not received, so it is unchanged | D-M6 applied; `BUILDING` open |

**Rev 5 re-verification (writer, 2026-09-17).** The rev-5 patch adds the citations below. Each was
read in the tree it names: `[J]` at `d552be05`, whose worktree was clean apart from A1's two
untracked physics tests, and the `[M]` working copy. The writer ran no graphify, no cargo command,
no git write and no timing. It used ripgrep, read-only file reads, read-only git (`status`,
`diff --stat`, `worktree list`) and web fetches.
- **`[J]` threadpool:**
  - `crates/boyko_threadpool/src/worker.rs:36-39, 43-69`;
  - `thread_pool.rs:46-61, 238-298, 347-380, 664-796`;
  - `tls.rs:126-161, 163-206, 208-217, 318-339, 389-395`;
  - `crates/boyko_threadpool/Cargo.toml:14, 20`;
  - `crates/boyko_threadpool/tests/{tls_lane_merge, diag_lane}.rs` (existence);
  - `crates/boyko_threadpool/src/block.rs:579-580, 677`.
- **`[J]` diag and log:**
  - `crates/boyko_diag/src/lane.rs:34-53, 105-107, 134, 136-140, 142-149, 159-166, 185-231`;
  - `crates/boyko_log/src/{sync_out.rs:68-82, drain_owner.rs:40-48, probe.rs:77}`.
- **`[J]` ECS:**
  - `crates/boyko_ecs/src/ecs/core/component/component_registry/required.rs:11-16, 143-202, 259-261, 285-351`;
  - `…/component/component.rs:37, 255, 299-337`;
  - `…/bundle/bundle_column_cache.rs:419`;
  - `…/ecs_master/ecs_master.rs:264, 296, 339-341, 663-720, 953, 1194`;
  - `…/component/hooks/scope.rs:26-32, 59-91`;
  - the four ECS statics (`hierarchy/commands.rs:127`, `observers/propagate.rs:30`,
    `relationship/mod.rs:90, 152`);
  - `crates/boyko_ecs/src/ecs/memory/vm_column.rs:278-286`;
  - `crates/boyko_ecs/src/ecs/core/mod.rs` (existence).
- **`[J]` workspace and docs:**
  - `Cargo.toml:104-127`;
  - `.cargo/config.toml:84-91`;
  - `docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:34-41, 87-95`;
  - `docs/threadpool/KE16-RESULTS.md:75`;
  - `docs/diagnostics/substrate/05-LADDER-GATES.md:56-61, 133`.
- **`[M]`:** `RUNTIME-DATA-LEDGER.md:1593-1605, 1624-1640`; `docs/memory/ledger/` (existence).
- **Web:** the rev-5 pages of §11.

Findings are in the rows below. Rows V-53 to V-56 were corrected or annotated in place; every other
new citation held (V-57, V-58).

| # | Where | Finding | Action |
|---|---|---|---|
| V-53 | 01 §6 item 5; 03 MQ-13 | The patch said the record route makes an `install` frame cost "one lookup instead of today's four TLS accesses (`:252-257`, `:371-377`)". The frame's pool-part accesses are **five**: `ACTIVE_POOL` at `[J]crates/boyko_threadpool/src/thread_pool.rs:245` and `:376`, and `LANE_DEPOSIT` at `:253` (`current_worker_id`), `:254` and `:372` (`set_current_worker_id`, one `with` each). `:252-257` also holds two `LANE` accesses (`:253`, `:255`), and the restore a third (`:373`); those stay on diag's word. MQ-13's range omitted `:245` and `:376`, the two `ACTIVE_POOL` accesses that become record reads | corrected in place (01 §6 item 5; 03 MQ-13 cites `:245`, `:252-257`, `:371-377`) |
| V-54 | 00 U-19, RK-15; 01 §6 item 2 | "`boyko_render`'s lib tests run 8 `App::new()` tests" counts call sites, not tests. The 8 sites are `[J]crates/boyko_render/src/csm_plugin.rs:108` and `light.rs:2052`, `:2097`, `:2148`, `:2227`, `:2277`, `:2360`, `:2513`. `:2227` is inside the helper `run_sv0_gate` (`:2214`), which three tests call (`:2405`, `:2421`-`:2439`, `:2465`-`:2494`, in the tests at `:2395`, `:2416` and `:2455`); `:2277` is a closure inside one test. So **10** tests build a default `App`, and libtest runs up to `available_parallelism` = 16 of them at once: 10 × 16 + 10 = 170 slots on the gate host (still ⌈170 / 64⌉ = 3 record pages) and 10 × 32 + 10 = 330 on a 32-logical host. The lib has no other pool construction (ripgrep for `App::with_pool`, `ThreadPoolBuilder`, `App::default()` in `boyko_render/src`: none). Every conclusion stands: 330 is far below the 6,144-slot pool share | corrected in place; critic pass 4's own 264 is kept as the critic's figure |
| V-55 | 00 §5, erratum D0-1 | D0's line item (`[J]docs/diagnostics/substrate/05-LADDER-GATES.md:56-61`) and DG12 (`:133`) hold as quoted. The erratum's claim that "the reason is the one DG12 already gives for the TLS `LANE` cell" does not. DG12 excludes `LANE` because D1 mandates that write and because it "costs 2 B of per-thread TLS and no `.bss`". `LANE_IDX` and `LANE_TLS_OFF` are neither: D1 does not mandate them, and they are shared statics in `.bss`. The exception is real, and its reason is 01 §6 item 4's performance argument | annotated in place; open for critic pass 5 |
| V-56 | 02 §4.3, the anchored-documents row | The row named `SYSTEMS.md` and the VG plan as owner-dirty. `git status --short` in `[M]` also lists `docs/FEATURE_MAP.md` as modified (and 04 §3's O1 names it); `docs/ARCHITECTURE.md` is clean | corrected in place |
| V-57 | 00 §11, the entries rev 5 adds or extends (V-51) | These hold, fetched 2026-09-17. **`TlsAlloc`:** a new index's slots "are initialized to zero"; failure returns `TLS_OUT_OF_INDEXES`; the opaque-value sentence is quoted verbatim. **`LocalKey`:** all four statements are on the page (`try_with`'s `AccessError` "if the key has been destroyed (which may happen if this is called in a destructor)"; re-initialisation during destruction; destructors only on the exiting thread at Windows process exit; none for a thread converted to a fiber unless it is converted back). **syn `Macro`:** `tokens` is the stream within the invocation's delimiters, and `parse_body` is "equivalent to `syn::parse2::<T>(mac.tokens)`"; "uninterpreted" is the plan's gloss, not the page's word. **cargo #13257:** "Strip debuginfo when debuginfo is not requested", merged 2024-01-15, shipped in 1.77.0. **Rust 1.77.1:** the behaviour is disabled "on Windows for targets that use MSVC". **`cargo bench`:** `--profile name` benchmarks with the given profile; that the default is `bench` is stated in the page's profile section, not in the option's own text. NtDoc, cbloom, the Unity pages, rust#99682 and trybuild are unchanged since V-42 and were not re-fetched | none |
| V-58 | every other citation the rev-5 patch adds | These hold. **Threadpool:** `worker.rs:36-39` (the `STEAL_EMPTY_GATE` rationale and `cfg!` constant) and `:43-69` (`worker_main` through the deque deposit: `:46` worker id, `:54` lane, `:59` active pool, `:69` `_deque_deposit`). `worker_main` never writes the id or the pool again; `WorkerDequeDeposit::drop` clears only `pool` and `deque`, not `wid`. `thread_pool.rs:46-61` (`MAX_WORKERS = 64`, DG4), `:238-298` (`install`; `fetch_add` at `:243`), `:347-380` (`InstallGuard`), `:664-796` (`build`, which has no `prepare` call), `:721-751` (the spawn loop), `:747` (the spawn `expect`), `:449` (`current_pool`). `Cargo.toml:14` and `:20` are the `boyko-diag` and `boyko-log` edges. `tests/tls_lane_merge.rs` (D7's source-shape rows count `LANE_DEPOSIT`) and `tests/diag_lane.rs` (DG2, DG3) exist. `block.rs:579-580` is `#[cfg(test)] mod tests`, and `:677` its `thread_local!`. `tls.rs:126-161` (`LaneDeposit`, `_pad` at `:141`, `DETACHED` at `:147-152`, layout asserts at `:159-161`), `:163-206` (statics at `:169`; `:196`, whose initialiser is on `:197`; and `:205`), `:208-217` (`lane_deposit`), `:318-339` (`worker_lane_for`, predicate at `:331`), `:389-394` (`current_worker_id`), `:425` (`is_in_system_run`). `WORKER_ID_DISPATCHER` is `u32::MAX - 1` (`:106`) and `WORKER_ID_UNATTACHED` is `u32::MAX` (`:110`), so the +1 encoding maps them to `u32::MAX` and 0. **Diag and log:** `lane.rs:34-53`, `:105-107` (the DG12 comment), `:134`, `:136-140`, `:142-149`, `:159-166`, `:185-231`; `lanes_leaked` exists (`:258`); `set_lane` has its two `install` call sites at `thread_pool.rs:255` and `:373`. `sync_out.rs:68-82`, `drain_owner.rs:40-48`; `probe.rs:77` sits behind `#[cfg(feature = "test-probe")]` (`lib.rs:98`). **ECS:** `required.rs:11-16`, `:143-202` (`BUILDING` is a `RefCell<Vec<usize>>`, which has drop glue, so 01 §6 item 8 rightly leaves it out of the "eight" `Drop`-free rows), `:259-261`, `:285-351` (`:301`, `:312`, `:320`); `RequiredIdFn` is `fn() -> ComponentId` (`:59`); `get_required_plan`'s only caller outside `required.rs` is `bundle/bundle_column_cache.rs:419`. `component.rs:37`, `:255`, `:299-337`. `ecs_master.rs:264`, `:296`, `:339-341`, `:663` (the drain; `:668`, `:690`, `:714`, and `:719` calling `drain_runaway_panic`), `:953`, `:1194` (`#[cold] #[inline(never)]`). `hooks/scope.rs:26-32`, `:59-91`. The four statics are at the cited lines. `vm_column.rs:278-286` is `swap_remove` with its release `assert!`. `core/mod.rs` exists. `app/app.rs:201-203` is `App::new`, which builds the default pool. **Census:** the four KF-45 crates hold exactly 12 production `thread_local!` statics (6 ECS, 3 threadpool, 1 diag, 2 log) plus the two harness rows, so the 9 + 1 + 2 split holds. **Workspace and docs:** root `Cargo.toml:104-127` (release `:111-112`, bench `:114-127`, the warning at `:122-126`), and the manifest has no `strip` or `debug` key anywhere; `.cargo/config.toml:84-91`; the TLS document `:34-41`, `:40-56`, `:79-82`, `:87-95`; `KE16-RESULTS.md:75` (16 logical); `MEASUREMENT-QUEUE.md:44-49`; `tests/internal_docs_anchors.rs:250` (the four gated documents); `SYSTEMS.md:601-661`, `:740-759`, `:935-947` carry anchors into kernel files; `[G]:581` (195 anchors). **`[M]`:** ledger `:1624-1640`; `:1628` holds the quoted text (with the elision marked); `:1718`; `docs/memory/ledger/` exists (10 group files). KF-43 (`:1590-1606`) is a precedent for statics as an allocation-free kernel form, though its statics are per-type descriptors initialised once, not mutable per-thread records. `D:/wt/tls-fixb` has 2 modified files, 8 inserted lines. The `BUILDING` row that V-52 left open is now replaced (01 §6 item 10) | none |

## 10. Revision log

### Critic pass 5 log (final)

Verdict: CHANGES_REQUESTED, no Critical remark. Closed by orchestrator ruling; the remarks below are OPEN.

- W1: Phase B's step B3 cannot record the UG-15 gate's baselines green as written, for two reasons that hold by construction. (a) Leg (2) pins 'the P29 set', and one of its five symbols, HeapRef::alloc_cold (ALLOCATOR-DESIGN-SPACE.md:2616), belongs to the Heap class that ruling U-1 never builds. It has 0 hits in [J]/crates, and P29's own rule says a missing symbol is RED, never a skip (:2618). (b) Green control (x) moves drain_runaway_panic ([J]ecs_master.rs:1192-1200), whose body is a panic! with a Location. The Location embeds the source path, so moving the function to a new module file adds a path literal to .rdata. Leg (7)(a)'s .rdata size then changes, and so does (b)'s (<anon-data>, class, size) multiset, where the control requires both to be identical.
- W2: The negative loom arms M1-M4 for the new thread context (KC-04) are bare #[should_panic] (01 §6 item 9; 03 §3), with 'running 5 tests' as the only check that they are not vacuous. The model already uses loom 0.7.2's 5-thread limit (main + 3 threads + a spawned child; rt/mod.rs:62). Loom's own thread-count assert (scheduler.rs:99) and its branch-limit panic (path.rs:118) both satisfy a bare should_panic, so an arm can be green while proving nothing. The repo's own convention requires expected = text for exactly this reason ([J]crates/boyko_threadpool/tests/loom_pool.rs:359-369).
- W3: After the trunk cut (A8), the owner may keep committing on main, and those commits merge only at the next phase boundary (02 §4.5 :637-639; 04 §2 step 11). Meanwhile the refactor waves RF-R and RF-V split render/rhi files 'in parallel with C-D' (02 §5 :646, :650), with no lock and no owner ruling. RF-V's device.rs and present/targets.rs are files the owner has uncommitted edits in right now ([M] git status). An owner edit to either file during Phases C-D reaches the trunk only at the end of Phase D, after the split, and must then be ported by hand.
- W4: Owner question Q-5 (00:158) and owner step O1 (04 §3) offer 'commit or drop' for the 59 uncommitted paths. That set includes the only copy of allocator design rev 2-2.4 (00:27-29; ' M docs/memory/ALLOCATOR-DESIGN-SPACE.md') and this plan's own untracked files. Step DOC-1, which edits the allocator file, has no O1 prerequisite. A 'drop' answer would destroy, with no pushed copy, the design that C1, D-S1(i), D-S2 and critique pass AP6 rest on.

# Architecture review: unified system plan rev 5 (critic pass 5)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED: no Critical remarks, but 4 Important ones (W1–W4). All four are spec fixes, not redesigns.

**Scope.** This pass covers the rev-5 delta, critic pass 4's remarks, and the architect's and writer's items left open for pass 5.

**Trees read (read-only; Grep/Read only).**
- `[J]` = `D:/wt/joltab` @ `d552be05`.
- `[A2]` = `D:/wt/uploadleak`, the in-flight lane for rung A2 (worker-panic propagation plus the `EntityReservoir` loom model); uncommitted.
- `[M]` = the main checkout, working copy.
- `[G]` = `D:/wt/_graph/refactor-census.md`.
- loom 0.7.2 sources and the std sources from the local cargo/rustup installs.

graphify was not run: this role has no shell tool.

## Status of critic pass 4's remarks
All resolved:
- C1.1–C1.7: C1.1 `.bss` statics; C1.2 the `BUILDING` record route; C1.3 `prepare()`; C1.4 msvc takes the portable arm; C1.5 `running 5 tests`; C1.6 the undefined names are now defined; C1.7 the `cache_slot` + 1 encoding.
- C2: the code worktree pool opens at A8.
- W1–W5 and O1–O8 are applied.

I checked the rev-5 code claims in `[J]`, and they hold:
- `thread_pool.rs`: `:243`, `:245`, `:253-255`, `:371-377`, `:664-796`, `:721-751`, `:747`.
- `tls.rs`: `:106`, `:110`, `:141`, `:147-152`, `:159-161`, `:169`, `:196`, `:205`.
- `worker.rs`: `:39`, `:46`, `:54`, `:59`, `:69`.
- `required.rs`: `:143-202`, `:285-351`. `required_cycle_panic` takes only the id, so a bitset loses no diagnostic.
- `vm_column.rs:278-286`.
- `ecs_master.rs`: `:264`, `:296`, `:339-341`, `:1192-1201`.

Two checks against in-flight lanes:
- **A2 does not break "a worker never claims".** The A2 lane never respawns a worker: fire-and-forget task panics abort the process, and scoped task panics are caught (`[A2]worker.rs:200-234`).
- **Phase B's lock sets really are disjoint.** B2's `alloc_frame_census.rs` has 0 `#[ignore]` sites, so B3's ignore-reason migration does not touch it.

## Critical
None.

## Important

### W1. Step B3 cannot record the UG-15 baselines green as written
**Where:** 03 §6, leg (2) ("the P29 set") and green control (x); 00 ruling U-1; 02 B3.

**Problem.** There are two causes, and each holds by construction.
- **(a) A pinned symbol that will never exist.**
  - The P29 set has five symbols, including `HeapRef::alloc_cold` (`[M]ALLOCATOR-DESIGN-SPACE.md:2611-2616`).
  - U-1 defers the whole Heap class, and `HeapRef` has 0 hits in `[J]/crates`.
  - P29's own rule reads "A missing symbol is RED, never a skip" (`:2618`).
  - Neither 01 §3's U-1 row nor DOC-1's patch (00 §5) removes the symbol from leg (2).
- **(b) Green control (x) goes red on leg (7).**
  - The control moves `drain_runaway_panic` (`[J]ecs_master.rs:1192-1200`). Its whole body is a `panic!` with a format string, and the panic's `Location` embeds `file!()`.
  - Moving the function to a new module file therefore adds a new path literal to `.rdata`. The old literal stays, because the file's other panics still use it.
  - Leg (7)(a) compares `.rdata` size, and (b) compares `(<anon-data>, class, size)` entries. Both move, while (x) requires leg (7) to be identical.
  - Control (ix) is unaffected: a line shift changes a `Location`'s value, not its size.
- **(PLAUSIBLE) Other pins may have no symbol.** Several added leg-(2) pins fail the P29 conditions that P29 adopted because such bodies may leave no symbol under fat LTO:
  - `Transform::component_id` is `#[inline]` (`[J]boyko_macros/src/component.rs:370`);
  - `register_new::<T>` and `register_layout::<T>` are generic with no inline attribute (`[J]…/component_registry/mod.rs:920`, `:1031`);
  - `try_register_dynamic` (`:967`).
- **Note.** The UG-15 mode rows for D-M0 and C1 name `grow_rows` but not its P29 sibling `commit_subregion` (`[J]component_pool.rs:558-560`), which both rungs change. The sensitivity-map rule catches this at the cut.

**Consequence.** B3 needs every pin and all ten controls green before Phases C and D start. Either the kernel phase stalls at B3, or a tester closes B3 by dropping a pin or a control — the "skip" / "repaired into a wrong red" class this repo has recorded.

**Confidence:** CONFIRMED for (a) and (b); PLAUSIBLE for the symbol-less pins.

**What is needed:**
- Strike `alloc_cold` from leg (2) (four symbols remain), and carry that change in DOC-1.
- Give control (x) a subject whose move cannot change a literal pool, or define leg (7)'s normalisation for `Location` path literals.
- State the missing-symbol policy for the pins that do not meet P29's conditions.

### W2. The KC-04 loom negative arms can pass vacuously
**Where:** 01 §6 item 9; 03 §3.

**Problem.**
- M1–M4 are specified as bare `#[should_panic]`, and the only check against vacuity is `running 5 tests`.
- loom 0.7.2 (the workspace pin, `[J]Cargo.toml:80`) allows at most 5 threads (`rt/mod.rs:62`), and the model already uses all five: main + 3 threads + a spawned child.
- Loom's own thread-count assert (`scheduler.rs:99`) and its "exceeded maximum number of branches" panic (`path.rs:118`) both satisfy a bare `should_panic`.
- The repo already records this hazard and pins the message: `[J]crates/boyko_threadpool/tests/loom_pool.rs:359-369`, `expected = "M2: lost wake"`.

**Consequence.** M3 (a different protocol with its own probe loop) or M4 can report "red as required" from loom's own limit panic and prove nothing, while the count still reads 5.

**Confidence:** CONFIRMED.

**What is needed:** per arm, an `expected = "<oracle text>"` for A1, A2 and the materialised-once count; a stated per-arm thread count of at most 5; a stated preemption bound.

### W3. After A8 the owner's own lane stays open against RF-R and RF-V
**Where:** 02 §4.5 (`:637-639`); 04 §2 step 11; 02 §5 (`:646`, `:650`); the 02 §4.3 lock table.

**Problem.**
- After A8, owner commits on main are allowed and are merged into the trunk only at the next phase boundary: B, end of Phase D, or end of Phase E.
- RF-R and RF-V run "in parallel with C–D" and split about 25 render/rhi files, including `boyko_rhi_vulkan/src/device.rs` and `present/targets.rs`.
- The owner is editing exactly those two files now (`[M]` git status).
- §4.3 has no lock holder for owner work, and no owner question covers this. Q-4 covers wave K only.

**Consequence.** Suppose the owner edits `targets.rs` (the largest census file, with 11 gated anchors) on main during Phases C–D. That edit reaches the trunk only at the end of Phase D, after RF-V has split the file, so it must be ported by hand into the split files, or the split redone. The parking rule does not cover this case.

**Confidence:** CONFIRMED that the plan permits it; PLAUSIBLE that it happens (the evidence is the current uncommitted edits).

**What is needed:** an owner question plus a rule. Either the owner's render work moves onto the trunk under the lock table, with the owner as a lock holder, or RF-R and RF-V wait until the owner declares their files quiet.

### W4. Q-5 offers "drop" over files that exist nowhere else
**Where:** 00 Q-5 (`:158`), RK-7; 04 §3 O1; 02 DOC-1.

**Problem.**
- Allocator rev 2–2.4 (3,864 lines) exists only in the `[M]` working copy (00:27-29; git status ` M docs/memory/ALLOCATOR-DESIGN-SPACE.md`).
- This plan's own six files are untracked (`?? docs/unification/UNIFIED-SYSTEM-PLAN-0*.md`).
- Q-5 and O1 ask to "commit or drop" the whole set (59 paths with `-uall`).
- DOC-1, which edits that allocator file, has no O1 prerequisite.

**Consequence.** A "drop" answer permanently destroys the design that C1, D-S1(i), D-S2 and AP6 rest on, along with this plan. Neither is pushed.

**Confidence:** CONFIRMED.

**What is needed:** Q-5 and O1 name the paths that must be committed and never dropped. DOC-1 waits for, or explicitly includes, the commit of that file.

## Optional

**O1. D-M6 test (b)'s red-first prediction is wrong.**
- Declaring `_ctx` after `_deque_deposit` also places it after `set_current_worker_id` and `swap_active_pool` (`[J]worker.rs:46`, `:59`).
- Each worker therefore claims lazily before `adopt`, and `adopt` then meets a non-zero word (01 §7 stability).
- The observable result is 24, or a debug panic inside a booting worker. Under KC-06a's boot-wait that panic can hang `build` instead of failing, rather than giving "16".
- Direction: state the true observation and make the boot-wait fail fast on a worker panic, or specify a mutation that changes only the drop order.

**O2. The contract's invariants contradict KC-04.**
- I-2 names KC-17 as the only storage exception, and I-3 says every commit goes through `commit_at`. KC-04's `.bss` statics satisfy neither.
- Direction: add the exception, with U-19 (d)'s reason.
- Because the owner's ruling is literal, list U-19 (d) as an owner question in §7, not only as an overturn condition. V-58 already notes that KF-43's precedent covers immutable per-type descriptors, not mutable per-thread records.

**O3. Erratum D0-1 is too narrow (writer finding V-55).** Besides stating its own performance reason, it must also cover these:
- **The "no syscall" rule.** Diag's design lists "Explicitly NOT owned: any thread, file, socket, syscall" (`[J]docs/diagnostics/substrate/00-GOAL.md:214`), but `prepare_lane`, `set_lane` and `word_by_call` call `TlsAlloc`, `TlsSetValue` and `TlsGetValue` inside `boyko_diag`.
- **DG12's counter.** DG12 is observed through "a `#[cfg(test)]` counter on each entry point" (`05-LADDER-GATES.md:133`), so `prepare_lane` must become a counted entry point, called exactly once with diagnostics off.
- **D-M6 (g)'s list.** "The only `boyko_diag` statics that hold a non-zero value" must be an enumerated list, because a test cannot enumerate a crate's statics.

**O4. Lock-table gaps.**
- **A2 and `codes.rs`.** A2 edits `boyko_log/src/codes.rs` (E0202 and E0203 at `[A2]codes.rs:869-872`; absent in `[J]`).
  - Neither A2's lock set nor the §4.3 `codes.rs` row (02:574) lists it.
  - The owner's copy adds W2208 (`[M]codes.rs:1021`), and the two meet at A8.
- **Path-keyed census files.** "A parked branch touches no kernel-critical file" (02:748-749) is not literally true. Parked device waves and RF-K3 share these files ([G]:584-590, :614-664):
  - `production_reachability_census.rs` — RF-R's `render_path_config.rs` and RF-K3's `profiling/tests.rs`;
  - the hot-path registry — RF-V's `device.rs` and RF-K3's `schedule_builder.rs`;
  - `gpu_blocking_reader_census.rs`.
  Rebases will conflict in those files.

**O5. Answer to open question 3 (RF-V's device receipt across a rebase).**
- "A green strict UG-15 shows that the kernel codegen the receipt ran on is unchanged" does not follow. Strict UG-15 compares the rebased commit with its new parent, not with the tree the receipt ran on.
- What transfers the receipt is that the commit is still the same move. So require all of the following, or re-run UG-12:
  - `git range-diff` identity outside the census files of O4;
  - no conflict elsewhere;
  - strict UG-15 green against the new parent.

**O6. Answers to open questions 1 and 2.**
- **Scoped UG-10: accept, but make the scope mechanical.** The test should read the branch's touch set and fail on any out-of-scope red. A hand-classified red list is the "green except known X" shape CLAUDE.md warns about.
- **`__hold_free_slots`: accept.** D-M6 already runs leg (7)'s tool for the `.bss` check; have it also assert that the symbol is absent from the linked `boyko_demo`, at no extra build.

**O7. A named tool is missing on the gate host.**
- The file map uses `llvm-symbolizer`, which rustup's llvm-tools does not ship there. The stable-gnu `bin` has `llvm-nm`, `llvm-size`, `llvm-readobj` and `llvm-objdump`; the only symbolizer copies are in MSVC Build Tools.
- Direction: name the tool and its version, or use `llvm-objdump --line-numbers` over the body's range.
- Adopt DG6's rule for every UG-15 leg: an absent tool is RED, never a skip (`05-LADDER-GATES.md:127`).

**O8. `bench-shipped` entries must name `-p <crate>`.**
- Every crate, `bench_bevy_vs_boyko` included, is in `default-members` (`[J]Cargo.toml:13`).
- The manifest itself warns that fat LTO over the Bevy dependency explodes compile time (`:86-88`, `:119-120`).
- A bare `cargo bench --profile bench-shipped` would spend the owner's quiet window building Bevy.

**O9. Nits.**
- 01 §6 item 6's batch claim says "success `Acquire`, as above", but the text above it and the shared-state table say `AcqRel`.
- The "12 → 4" `thread_local!` census should say whether the loom word (`loom::thread_local!`) counts.

## Positive
- **KC-04's core design is right.** The `.bss` table, the index held in the word, and batch claims at build remove probing, materialisation and worker refusal in one move. Costs are stated per host, with an overturn for each.
- **The zeroing rule is justified.** Release is the single zeroing site, and its necessity is shown (`worker.rs:46`, `:59` are never restored); the claim only checks.
- **`prepare()` has the right justification:** the low-index argument. The canary checks both directions, and `TLS_OUT_OF_INDEXES` has a fallback.
- **`PlanBuildSet` is sound.** Its publication goes through shared `Cell`s, the SB/TB reasoning is correct, and the Miri mutation really exercises the protector.
- **The sensitivity maps are sound.** The layout-map perturbation (`repr(C)` plus an `align_of`-sized prefix) shifts every displacement whatever rustc's field order, and the containment map closes the by-value gap.
- **Adding green controls is the right instinct;** only (x)'s subject needs changing.
- **The `bench-shipped` rule is correct,** and so is the argument that a single codegen unit cannot answer MQ-12's "re-seam".
- **Modding stays optional with zero cost.** The delta adds no modding cfg or feature, no export and no callback; `asm!` is outside leg (1)'s count; B3's `proc_macro2` twins are compile-time only.

## Open questions for the architect
1. **Disk (RK-11).** The three new profiles each get their own `target/<profile>` tree holding fat-LTO artifacts. That multiplies across up to five target dirs (trunk, k-1..3, refactor), plus the layout-map scratch builds, against about 36 GB free on D:. Settle it by measuring the size of `D:/wt/_targets/joltab/release` after a `--workspace` release build.
2. **Destructor order on the portable arm.** On OS-key targets (the gnu fallback, and Miri on the gnu target), `EXIT_GUARD`'s destructor reads `WORD`. That works because std runs key destructors in reverse registration order (stable-gnu std `sys/thread_local/key/windows.rs:150-191`) and `WORD` is always registered first. Please state this dependence, so that a red Miri test 1 is diagnosed correctly.
3. **Pins without symbols (W1).** Which of the named leg-(2) bodies actually exist as symbols in the fat-LTO `boyko_demo`? B3's probe answers this cheaply.


### Rev 5.1 changelog (modding reconciliation, writer, 2026-09-17)

**Brief.** Reconcile the plan with the closed modding design
(`[M]docs/modding/MODDING-DESIGN-SPACE.md`, rev 6, approved by critique pass 6) and its survey
(`[M]docs/modding/MODDING-RESEARCH.md`), under the owner's new hard requirement: modding is
optional, and a game that does not use it pays no overhead of any kind, ideally with code generation
identical to an engine without modding support (H-1, 05 §1). The kernel contract is to provide
exactly what the recommended modding option needs, with every item additive and compiled out when
modding is unused.

**Scope.** Files 01 and 05, and in this file the title, the status paragraph, 05's index row and this
changelog. Files 02, 03 and 04 were not edited; the edits they owe are listed in 05 §5 RM-3. This is
a writer's reconciliation, not an architect patch.

| Change | Why | Where |
|---|---|---|
| **H-1** stated as the governing requirement for modding | the owner's ruling of 2026-09-16 | 05 §1 |
| **Rule S-1.** Every kernel modding item is generic over `ModSeam`, a `#[doc(hidden)] pub unsafe trait` with no methods and no implementor in any kernel crate. A game that links no modding crate instantiates none of that code, so it has no object code at any profile | Rev 5 argued "no engine caller, dropped at fat LTO". Critique pass 6 found that claim quoted beyond its measurement, which was taken at `codegen-units = 1` (P6-8, `MODDING-DESIGN-SPACE.md:2503-2507`). A generic body is compiled only where its type arguments are known (the source cited at `MODDING-RESEARCH.md:214-223`). Overturned by UG-15 leg (7b) or leg (7) moving on a seam commit | 01 I-4, KC-19b; 05 §1 (b), §2 |
| **The contract is scoped to the common set of the live options:** `ModSeam` and four occupancy readers | The live ordering is A′-nightly > C-3a ≈ A′-stable, with C route 2 unplaced and A third (`:1874-1877`). A′ has no kernel delta (`:1834`). C registers nothing dynamically (`:1083-1087`). Modding §7.9 is needed by neither A′, C nor D (`:1707-1708`). The loader's occupancy reads are needed under A′ and C (`:1653-1656`). So choosing among A′, C and D changes no Phase-D rung | 01 I-4, KC-19b, §5; 05 §3.1 |
| **KC-19b shrinks.** P32 R1's rename, P40's sized body and the class-A entries move to MS-02b (option A, Stage 3). Rev 5's five readers become four readers plus KC-19a's `id_space_census()` | Not needed by A′, C or D. `id_space_census()` is already an engine item (`ALLOCATOR-DESIGN-SPACE.md:3657`) | 01 KC-19b, §3; 05 MS-02b, MS-03 |
| **MS-01 moves to Stage 3, option A only.** Rev 5's `pub` on `try_register_dynamic`, `set_residency_class` and `install_map_entities_fn` is withdrawn in favour of one installer generic over `ModSeam` | Under S-1 a visibility change made for modding is admitted only where named. Under A′, C and D a mod component is an ordinary derived type and needs no descriptor | 05 MS-01; 01 §5 |
| **MS-02 is split:** MS-02a (every option, no kernel delta) and MS-02b (option A) | Mandatory stable names are common to every option; the dynamic sized mint is not | 05 §3.2 |
| **MS-14** (A′'s manifest overlay) and **MS-15** (the C/D plugin registry) are added | These are the head options' own items, both outside the kernel. Route (i), `["rlib", "dylib"]` in the shipped manifests, is refused because rust#51009 can silently drop fat LTO from the non-modding build (`:700-711`) | 05 §3.2 |
| **Options and Lands columns** are added to the path table | Each item is built only for the option that needs it | 05 §3.2 |
| **I-1 gains a dispatch clause** | H-1 names dispatch beside identity and storage | 01 I-1; 05 §2 |
| **Gate additions:** a leg (5) check of S-1's shape; leg (7b), an object census of the kernel rlibs; red control (xi) and green control (xii) | They prove "not compiled" rather than "not linked" | 05 §6 (03 §6 to follow, RM-3) |
| **P6-8, P6-9 and P6-10** get dispositions | Every remark of critique pass 6 is now carried | 05 §5 |
| **RM-1 to RM-4** are raised | They are listed below and in 05 §5 | 05 §5 |
| **Stage 1 builds only the common set;** Stage 3 builds only the chosen option's items | — | 05 §7 |

**Findings made while reconciling.**
- **RM-1 (option A only).** Under A, the mod image duplicates KC-04's `THREAD_RECORDS`,
  `THREAD_BUSY` and `CTX_IDX` (01 §6 items 1 and 3). The mod's first lookup therefore claims a
  record in its own table and reads detached, and giving it the host's index does not help.
  MD:M-A2 must be re-posed before A is chosen. There is no cost to a non-modding build, or to A′, C
  or D.
- **RM-2 (engine defect).** A duplicate full `stable_name` is appended, not refused
  (`[J]crates/boyko_ecs/src/ecs/core/component/component_registry/serialize.rs:394-397`), and
  resolution returns the first match (`:415-421`). The fix is proposed as an engine rung (bugs
  first).
- **Manifest census.** No `Cargo.toml` in `[J]` has a `crate-type` key, so MS-14's census starts
  at 0.
- **Reachability residual (a prediction).** MS-03's readers make `QUERY_NEXT_ID` and
  `BUNDLE_NEXT_ID` reachable from other crates, because today only non-generic dispensers name
  them. The other three counters are already named by generic bodies. Legs (7) and (7b) at
  D-S1(ii) settle it.

**Rev 5.1 re-verification (writer, 2026-09-17).** Every citation this revision adds was read in the
tree it names:
- **`[J]` at `d552be05`.** HEAD was checked with `git log -1`; the worktree was clean apart from
  A1's two untracked physics tests. Files read:
  - `crates/boyko_macros/src/component.rs:312-321` (the serialize-install arm), `:368-376` and
    `:384-394` (the closure; `#serialize_install` at `:392`);
  - `crates/boyko_ecs/src/ecs/core/`:
    - `component/component_registry/mod.rs:63`, `:133`, `:214`, `:435`, `:614`, `:920-922`, `:967`;
    - `component/component_registry/clone.rs:161`, `:181`;
    - `component/component_registry/serialize.rs:383-398`, `:410-423`;
    - `component/component.rs:77-79`, `:187-197`;
    - `iters/query/query_type_registry.rs:98`, `:125-140`;
    - `bundle/bundle_type_registry.rs:93`, `:105-122`;
    - `resources/resource_registry.rs:127`, `:164-176`;
    - `events/event_registry.rs:93`, `:100-118`;
    - `resources/resource_type_registry.rs:91-94`;
    - `asset/backing.rs:115`, `:131`;
    - `ecs_master/ecs_master.rs:317`;
    - `schedule/schedule.rs:1215`, `:1380`;
  - a grep of every `Cargo.toml` for `crate-type` / `crate_type`: 0 hits.
- **`[M]` working copy.**
  - `MODDING-DESIGN-SPACE.md`, read in full: `:3`, `:89-92`, `:104-116`, `:212-217`, `:637-641`,
    `:647-656`, `:697`, `:700-711`, `:715-719`, `:723-733`, `:741-743`, `:1083-1087`,
    `:1116-1130`, `:1242-1255`, `:1380-1407`, `:1653-1656`, `:1662-1665`, `:1707-1708`,
    `:1776-1790`, `:1795-1804`, `:1818-1823`, `:1831`, `:1834`, `:1874-1877`, `:1952-1954`,
    `:1971-1987`, `:2003-2010`, `:2134`, `:2164-2170`, `:2423-2517`, `:2503-2507`;
  - `MODDING-RESEARCH.md:150-265`, including `:214-223`;
  - `ALLOCATOR-DESIGN-SPACE.md:3657` and `:3692`.
- **Web.** The matklad page on `#[inline]` was fetched on 2026-09-17. Both quotes in 05 §1 (b)
  hold. The page does not say whether an unused `#[inline]` function is emitted in its defining
  crate, which is why S-1 relies on generics and not on `#[inline]`.
- **Tools.** The writer ran graphify once (off-target hits), then ripgrep and read-only file reads,
  read-only git (`worktree list`, `log -1`, `status --short`) and one web fetch. It ran no cargo
  command, no git write and no timing.

**Open for critic pass 5 (from rev 5.1).**
1. **S-1 is new, and the writer, not the architect, wrote it.** The critic checks it against I-4's
   never-list and UG-15's legs, and checks whether any Stage-3 item cannot take a type parameter.
2. **The reachability residual is a prediction.** D-S1(ii)'s legs (7) and (7b) settle it. If it
   moves a leg, the two readers leave the kernel for `boyko_mod_host` under S-1's overturn.
3. **RM-3 lists the edits 02 and 03 owe** (D-S1(ii)'s scope, test and rename list; UG-15 leg (5);
   leg (7b); controls (vii), (xi) and (xii); UG-16; UG-19; MQ-09). The architect's next patch makes
   them.
4. **RM-2's rung placement** is the architect's.
5. **`ModSeam` safety.** Should `ModSeam` be `unsafe` to implement, as written (a speed bump for
   implementors), or safe? The readers are safe, and the Stage-3 installers are `unsafe fn`s in
   any case.

### Rev 5 changelog (critic pass 4)

Rev 5 is the architect's patch answering critic pass 4, applied on top of the partly applied rev 4.
It is a patch, not a full re-emission, with one exception. The text rev 4 never applied (00 U-19,
RK-12, §5, §11; 01 KC-04, §6) is re-emitted here as one text, so rev 4's P0-* and P1-1 to P1-10 are
superseded. The architect read code in `[J]` @ `d552be05` and documents in the `[M]` working copy.
The architect ran no graphify, cargo, git or timing command; web pages are in §11. The writer
re-verified the citations the patch adds (§9, rev 5 re-verification).

| Remark | Resolution | Where |
|---|---|---|
| **C1.1 storage** | The records and the busy bitmap are two all-zero statics in `.bss`, `THREAD_RECORDS` (8192 × 64 B) and `THREAD_BUSY` (128 words). There is no `VmColumn` and no materialisation step. Loom arm M4 models the rejected lazy form. U-19 (d) is the owner's overturn and names its fallback | 01 §6 items 1, 9; 00 U-19, §5 |
| **C1.2 `BUILDING`** | Decided: the record route. `EcsThreadFields.plan_build` publishes a stack-local `PlanBuildSet` of `Cell<u64>` words, reached through `&`. A `&mut` set cannot see an `id_fn` re-entry, and publishing a raw pointer beside a `&mut` is UB (Miri test 2's mutation). V-36's condition is moot. Open question 3 is answered in item 11 | 01 §6 items 10–11, layout; 03 §3 |
| **C1.3 `LANE` / `prepare()`** | `thread_ctx::prepare()` is defined: it allocates both TLS indices and runs the canaries. `ThreadPoolBuilder::build` calls it first; a `debug_assert!` before the first spawn is its red-first check. The performance reason: racing workers could leave the kept index ≥ 64 and pin the process to the call arm. The lazy CAS path remains for threads that write before any build | 01 §6 item 4; 02 D-M6 (a) |
| **C1.4 msvc arm** | msvc takes the portable arm, matching MQ-13. Only windows-gnu takes the word arm; U-19 (c) is msvc's overturn | 01 §6 item 3; 00 U-19 |
| **C1.5 test count** | `running 5 tests` everywhere: one model plus M1–M4 | 01 §6 item 9; 03 §3 |
| **C1.6 undefined names** | Now defined: U-19 (a)–(d); `WORD_ARM`, `DIRECT_ARM`, `LANE_WORD_ARM`, `LANE_DIRECT_ARM` (a table with defaults and MQ-13 patches); `ecs_fields()` (`thread_fields.rs`, the single `ext_ptr` caller); D-M6r's scope; `DETACHED` (encoding); the D0 erratum row | 01 §6 items 3, 10, layout; 00 U-19, §5; 03 MQ-13 |
| **C1.7 `cache_slot`** | `wid` and `cache_slot` are both stored as value + 1, so an all-zero deposit decodes to `WORKER_ID_UNATTACHED` and `NO_CACHE_SLOT`. D-M2 lands `cache_slot` in that encoding, and D-M6 re-encodes `wid` | 01 §6 layout; 01 KC-05 |
| **C2** | The code pool opens at A8, and B3, B1 and B2 take its three worktrees, with B4 next. Phase B's lock sets are stated. B1 edits the ledger in its own worktree, because UG-02 reads the ledger as data | 02 §4.1, §4.5, Phase B; 04 §1, §4 |
| **W1** | Capacity is 8192 slots. Workers are batch-claimed at build and clamped with a 2048-slot foreign reserve, so no worker is ever refused. An exhausted build panics on the building thread. The sizing basis is stated. A single-test binary tests the bound. Counters go to UG-20; RK-15 is new | 01 §6 items 2, 6, 8; 02 D-M6 (f); 00 RK-15; 03 UG-20 |
| **W2** | One zeroing site: release. The claim only checks, with a debug-only assert. The site is necessary because `worker_main` never restores `active_pool` or `wid`. The red-first mutation deletes that site; a debug build trips the claim check and a release build reads stale fields. Open question 1 is answered here | 01 §6 items 6–7; 02 D-M6 (d); 03 §3 test 1 |
| **W3 (a)** | A new containment map (by-value types, transitively). The layout-map perturbation becomes `repr(C)` plus an `align_of`-sized prefix, which shifts every displacement whatever rustc's field ordering. D-M1, D-S6 and D-S7 now name `EcsMaster`; D-M5 and D-S5 name bodies and leave the strict row | 03 §6; 02 §2 mode table, D-M5, D-S5, D-S6 |
| **W3 (b)** | C1 declares a rename list (its move map) | 02 §2; 03 §6 |
| **W3 (c)** | Symbol-less and compiler-anonymous data references print as `<anon-data>`. B3's probe build settles the PE's forms. Open question 4 is answered here | 03 §6; 02 B3 |
| **W3 (d)** | Green controls (ix) and (x) | 03 §6, §1; 02 B3 |
| **W3 (e)** | Freshness rules: the containment map is re-derived per cut; the file map follows move maps and is re-captured after RF-K, C1 and new files; the layout map is re-captured after type changes | 03 §6 |
| **W4** | Entries that decide pins, codegen partition, inlining or an attributed rung's keep time under `bench-shipped` (`inherits = "release"`). MQ-12 can now say "re-seam" | 03 §5; 02 B3 |
| **W5 (1)** | Anchor lines are not locked. Their re-derivation is a merge step in the one trunk worktree; on branches, UG-10 runs scoped | 02 §4.2, §4.3, §5; 03 UG-10 |
| **W5 (2)** | Refactor-worktree priority: kernel-critical waves first; device-gated commits are parked on their own branches while awaiting UG-12 | 02 §5 |
| O1 | The erratum names DG12, and D-M6 adds a DG12 leg | 00 §5; 02 D-M6 (g) |
| O2 | `[profile.seam-census]` sets `strip = "none"` explicitly (Cargo ≥ 1.77's implicit `strip = "debuginfo"`). The fallback profile keeps `lto = "fat"` | 03 §6 (2), (7) |
| O3 | The sensitivity map's debuginfo is set through `[profile.sensitivity-map]`, never `RUSTFLAGS` | 03 §6 |
| O4 | RF-O1 → **RF-L**; RF-O2 → **RF-V** | 02 §2, §3, §4.3, §5; 03 §2 |
| O5 | V-49 bullet reworded | 02 §2 |
| O6 | The `tls-*` probes are not needed for MQ-13: review `tls-fixb`'s uncommitted diff, then prune | 04 §1, §5 |
| O7 | New restriction stated for engine ops inside other thread-locals' destructors | 01 §6 item 8 |
| O8 | M-series prefixes applied (00 U-4, U-6, U-9, RK-12; 02 D-M2; 03 UG-04, §5; 05); every heading reads rev 5; the index is updated | 00–05 |
| V-44 | D-S7's cell no longer asserts that `is_empty` is inlined; the file map decides | 02 §2 |
| V-45 | `prepare()` defined (C1.3) | 01 §6 item 4 |
| V-49 | O5 | 02 §2 |

**Answers to critic pass 4's open questions.**
1. **Did rev 4 change release step 5.3?** Rev 5 decides the point: the release's zeroing is the only
   zeroing site, and the claim only checks (01 §6 items 6–7).
2. **What sets the capacity?**
   - The table holds 8192 slots, and workers are clamped at build to keep a 2048-slot reserve.
   - At that size a 1024-logical host running 1024 concurrent `App` tests fits: 96 full pools + 928
     reserve dips + 1024 foreign threads = 1952 (01 §6 item 2).
   - `.bss` costs virtual size only, so a larger table would cost nothing resident.
3. **How does `id_fn` re-enter?**
   - `RequiredBuilder::require` is a `pub fn` that takes any `fn() -> ComponentId`
     (`component.rs:323-327`), and `Component` is a safe trait (`:37`).
   - A hand-written impl can therefore pass an `id_fn` that builds a world and inserts the component
     under construction.
   - That insert's bundle resolution reaches `get_required_plan`
     (`bundle/bundle_column_cache.rs:419`) while the outer build sits between `:301` and `:312`.
   - The derive never emits such an `id_fn`, but safe code can reach the path. D-M6's test uses
     exactly this shape (01 §6 item 11).
4. **What does normalisation print for a data reference with no symbol?** `<anon-data>`, which also
   covers compiler-anonymous names. B3's probe build records which forms the gate host's PE shows,
   and green control (ix) holds the rule to them (03 §6).

**Where each item of the applied rev-3/rev-4 KC-04 text went.**

| Item (01 §6 as it stood) | Disposition |
|---|---|
| 1 records: `VmColumn` 256 × 64 B, materialised and zero-filled at first claim | replaced by item 1 (`.bss` statics, 8192 rows); the lazy form survives only as loom M4 |
| 2 the word: Windows arm "msvc and gnu", `IDX`, canary, `TLS_OFF`; portable, loom; diag's `LANE` copy; `LANE` allocates only on a write | item 3. gnu only by default; the word holds index + 1; the canary checks both directions; `TLS_OUT_OF_INDEXES` falls back. `LANE`'s no-allocation-on-read property is kept; the index is now also allocated by `prepare()` (item 4) |
| 3 lookup; `Relaxed` argument; cost | item 5, same argument; `&'static ThreadRecord`, not a raw pointer |
| 4 claim: 4.1 `EXIT_GUARD`, 4.2 scan and CAS, 4.3 zero, 4.4 write word; refusal | item 6. 4.3 is now a debug check (W2); a batch claim and `adopt` are added; refusal is counted |
| 5 release: word → zero → bit; no callback; runs in std's runner | item 7. Its zeroing is now the only site, with the necessity stated; release also runs from `AdoptedRecord`'s drop |
| 6 not in contract: abnormal termination, fibers, claim during teardown | item 8, plus the O7 restriction and the refused-write rule |
| 7 loom: A1–A3, M1–M3, `running 4 tests` | item 9, plus M4 and a batch-claiming thread; `running 5 tests` |
| 8 route per row (8 + 1 + 2 + 1; `BUILDING` conditional) and end state | item 10 (9 + 1 + 2); `BUILDING` decided; census scope stated |
| `ThreadRecord` / `EcsThreadFields` layouts | layout block. The `wid` and `cache_slot` encodings are stated, `_pad` is a `Cell`, and `EcsThreadFields` gains `plan_build` |
| "Why one record" bullets | kept verbatim |
| shared-state rows `THREAD_TABLE.busy[w]`, `.records[i]`, the word, `TLS_OFF`/`IDX` | replaced by the rows in 01 §6's shared-state table, which add the counters and the free-count estimate |

**Readiness checklist (rev 5 delta).**

| Item | State |
|---|---|
| Goal and metrics | unchanged (00 §1–§2); KC-04's per-read, per-thread and per-process costs stated (00 U-19, 01 KC-04) |
| Decisions justified, with overturn gates | U-19 (a)–(d); capacity sizing basis; single zeroing site; `prepare()` (the low-index argument); the `bench-shipped` rule; the anchor merge step; RF priority |
| Data structures | `ThreadRecord` 64 B / align 64; `LaneDeposit` 24 B with its encodings; `EcsThreadFields` ≤ 24 B / align 8; `PlanBuildSet` 64 B on the stack; statics 524,288 + 1,024 B in `.bss`. False sharing: records are slot-private on their own lines; the busy words are written on cold paths only |
| API | `current`, `prepare`, `claim_for_pool`, `adopt`, `unsafe release_current`, `ext_ptr` (hidden), `refused_write_panic`; one `#[doc(hidden)]` test seam, `__hold_free_slots`, which fat LTO drops; no `dyn` |
| Threading | orderings per atomic (01 §6 shared-state table); happens-before for the release → claim and build → worker hand-offs; loom plus a single-test lifecycle binary |
| Edge cases | full table; `TLS_OUT_OF_INDEXES`; canary failure; TLS teardown; abnormal exit; fibers; refused writes; the O7 restriction; spawn failure |
| Drop order | `AdoptedRecord` declared first in `worker_main`; `PlanBuildPublish` and `PlanBit` unwind-safe |
| `unsafe` invariants | `Sync` on the record table (slot-private); the `asm!` read (`off` validated by this process's canary); unchecked indexing (`w − 1 < THREAD_SLOTS`); `release_current`'s no-live-reference contract |
| Integration | touch sets, locks and the pool timing updated; B1's ledger exception; gated-document rule |
| Validation | D-M6 tests 1–7 with red-first checks; Miri rows with diagnosis text; UG-15 red (i)–(viii) and green (ix)–(x) controls; MQ entries carry the profile |
| Modding cost | no cargo feature, no `cfg` for modding, no export, and no added always-on path; KC-04 is engine code |

**Open for critic pass 5 (architect).**
1. **Cost of scoped UG-10 on branches.** It means a rung's own green run lists expected anchor reds
   instead of passing clean. The architect judges this cheaper than a lock that serialises every
   anchor-moving rung; the critic may prefer a different trade.
2. **The `__hold_free_slots` seam.** It is a `#[doc(hidden)] pub` test seam in a shipped crate,
   dropped at fat LTO. The alternative, a `cfg` or cargo feature for tests, is also a configuration
   axis. The architect chose the seam; leg (7) does not run on D-M6, so the seam's absence from the
   game binary is argued rather than gated.
3. **RF-V's UG-12 receipt across a rebase.** Its validity relies on UG-15 strict green over
   pure-move kernel commits. If the owner wants UG-12 re-run after every rebase, the parking rule
   still holds, but device-gated merges slow down.

**Open for critic pass 5 (writer's findings).**
- **V-55:** erratum D0-1 cites DG12's reason for an exception that DG12's reason does not cover
  (`LANE_IDX` and `LANE_TLS_OFF` are `.bss` statics, not TLS). The exception needs its own sentence
  in DOC-2.
- **V-54:** the capacity examples now count 10 `boyko_render` tests, not 8. No conclusion changed.
- **How the patch was applied.**
  - The patch's closing readiness delta and its open questions for critic pass 5 had no named
    location; they are in this changelog, with "I" rendered as "the architect".
  - In 01 §6, the content of items 10 and 11 is indented four spaces so that it stays inside the
    numbered list; this is a layout change only.
  - Patch-internal names (`P5-01-5`) in the disposition and readiness tables are replaced by the
    section they name.

### Rev 4 changelog (critic pass 3)

Rev 4 is the architect's patch answering critic pass 3, applied on top of rev 3. Like rev 3, it is a
patch, not a full re-emission. The architect read code in `[J]` @ `d552be05` and documents in the
`[M]` working copy. The writer re-verified the citations of the part it applied (§9, V-44 onward).

| Remark | Resolution | Where |
|---|---|---|
| **C1** | **Gap:** UG-15 could not see a kernel seam item that survives fat LTO in the game binary without an attribute. Legs (1)–(3) look at sources and pinned bodies, and both arms of leg (6) contain the kernel (`ALLOCATOR-DESIGN-SPACE.md:3825`). **Fix: new leg (7), the linked seam census.** It runs strict on every seam commit, against the parent. The linked `boyko_demo`'s section sizes, defined-symbol multiset and export directory must be identical. The multiset covers every binding, because a fat-LTO survivor is local (`MODDING-DESIGN-SPACE.md:2147`). Leg (7) restores M-P1's pins (4) and (5) (`:2146`). **New red controls:** (vii), and (viii), which is red on leg (7) while leg (1) stays green. **Knock-on edits:** "seam commit" is now defined in 05 §6; 05 §3's "dropped at fat LTO" cells name leg (7) as the check; D-S1(ii) and B3 name it; UG-16 hands the delta-0 check to it. | 03 §1, §2, §6; 02 B3, D-S1(ii); 05 §3, §6, §7 |
| **C2** | The architect records that KC-04's table as a `.bss` static "closes C2 by construction" (open item below). **In the applied part:** KC-04's invariant row requires `THREAD_TABLE` in `.bss`, and D-M6 checks that structurally with leg (7)'s tool. Loom arm M4 keeps rev 3's lazily materialised header (check, then write) as a negative model, which fails the one-materialisation count. The loom table starts fresh and all-zero, as the static does at process start. **Not received:** the ruling (00 U-19) and 01 §6. | 01 §7; 02 D-M6; 03 §3. Pending: 00 U-19, 01 §6 |
| **W1** | **Gap:** the UG-15 mode table missed bodies that rungs move. `swap_remove/10k` runs `delete_entity`, which is `delete_entity_core` plus `drain_deferred_hook_queue`. D-M6's guard and depth read, D-S7's `CommandQueue::is_empty` and D-E2's redirect all sit in that pinned body (V-44). **Fixes:** a mechanical sensitivity map, captured at B3: a file map from line tables, and a layout map from an 8-B leading-field insertion per leg-(3) type. A rung whose touch set or named types reach a body must name the body, or argue at its cut that its bytes are unchanged. The mode table is re-derived: D-M0/C1 argue the body, D-M6, D-S7 and D-E2 name it, and D-R2a reaches it through `EcsMaster`'s layout map. MQ-20 records `swap_remove/10k` for the naming rungs. 01 §7's `ArchetypeFlags` row no longer claims that no pin moves. | 02 §2; 03 §2, §5, §6; 01 §7 |
| **W2** | D-M6 no longer waits on MQ-13. **MQ-13's new form:** it runs after D-M6 merges, as a keep / re-seam entry. Its arms are built from D-M6's merge commit or its parent, and a non-default arm is a one-line patch that is never committed. **New bench:** `dispatcher/body_1us_tasks_W` prices `install`'s `LANE` save and restore. **Re-seam:** rung D-M6r, confined to `thread_ctx.rs`, `lane.rs` and two accessors. | 02 D-M6, §3; 03 §5 |
| **W3** | **Gap:** `boyko_log/src/codes.rs` is owner-dirty, reaches the trunk only through A8, and is edited by D-E18 and D-M6 (U-12), yet the RF-O wave that splits it had no fixed place. **Fix: RF-O is split in two.** RF-O1 (`codes.rs`) runs after A8 and B3, and before D-E18 and D-M6. RF-O2 (the two `boyko_rhi_vulkan` files) runs after A8 and B3. **Knock-on edits:** D-M6's touch set gains `codes.rs`, and D-E18 waits for RF-O1. §4.3 gets a `codes.rs` row with a full-`--workspace` rule, because the file's own lib-test pin is not built under a `--test` filter (V-47). All three pool worktrees open after B3. | 02 §2, §3, §4.1, §4.3, §5; 03 §2 |
| **W4, W5, W6, O1** | The brief tags these only on D-M6's row, together with W2 and W3, so their separate statements were not available to this writer. **The row's changes beyond W2 and W3.** **Exit:** a pool performs exactly W claims (no re-claim after release). **Zero at claim:** every record field is written and every guard forgotten, then each field must read idle; the pool fields decode to `DETACHED`, including `cache_slot == NO_CACHE_SLOT`. **Refusal:** reads answer `DETACHED`, and `install` raises the coded panic. **Plan build:** `Cycle` on re-entry with the component under construction, red-first against a build without the publication. **`LANE`:** a one-time index allocation on windows-gnu. **Structural:** `THREAD_TABLE` is in `.bss`. **Matching edits:** KC-04's invariant row, UG-20 (`IDX`, and the lane's index), and 03 §3's Miri test 2 (the `PlanBuildSet` published pointer, against a `&mut` bitset under SB and TB). | 02 D-M6; 01 §7; 03 §1, §3 |
| **W7** | Prerequisites and red-control counts still carried rev-2 wording (V-43). **Prerequisites:** bare `D-S3` now reads D-S3(iii) in D-S4, D-S5, D-E2, D-E7, D-E9, U4, AS2 and the DAG. **Red controls:** 02 B3, 02 D-S1(ii) and 05 §6 now point to 03 §6's (i)–(viii), which name the leg each must turn red and when they run. | 02 §2, §3; 03 §6; 05 §6 |
| **W8** | **Gap:** leg (1)'s `syn` walk cannot see an attribute inside a `quote!` template, because `syn` keeps a macro body as uninterpreted tokens. **Fix: two mechanisms.** **M-a** scans every `quote!` / `quote_spanned!` template as token sequences. **M-b** expands a fixture corpus through `proc_macro2` twins of every `boyko_macros` entry point and runs the same visitor on the output. B3 lands the twins, and the fixture key list is the parsers' own `const` table. **New red controls:** (v), and (vi), which only M-b can see. B3's touch set gains `boyko_macros/src/**`, and B3 precedes RF-K2. | 03 §6; 02 B3 |
| O2 | New step A0 reviews the two non-equivalent commits of `claude/trusting-ramanujan-0f8927` (`6a9871bb`, `867dd734`) before A2 merges and before A8. A2, A8 and the prune wait for it. | 02 §2, §3; 04 §1, §2, §5 |
| O3 | `linkedProjects` always lists the main checkout and the trunk. A pool worktree, or `D:/wt/refactor`, is listed only while it holds open work (CLAUDE.md; RK-11). | 02 §4.1 |
| O4 | The allocator design and the modding design both have an M-A1 to M-A4. The applied cross-document cites now carry `AL:` or `MD:` (02 B2; 03 MQ-08, MQ-09; 05 §1, MS-03, MS-10, MS-12, R3-1). MQ-09 gains MD:M-A3 (startup cost with 0 mods). The unprefixed names that remain are listed below. | 02 §2; 03 §5; 05 §1, §3, §5 |
| V-43 residue (not tagged in the brief) | 01 §7's KC-12/KC-13 row now reads `<G::Release as ReleasePolicy>::KIND`. 05 R3-1 now places MD:M-K3 after D-S3(ii) and after D-S3(iii). 05 §7 now says D-S1(ii)'s parent is its cut commit. 01 R-C and 00 RK-12, also listed by V-43, belong to the part not received. | 01 §7; 05 §5, §7 |

**Answer to the critic's open question 3 (pass 3): the arms of leg (4).**
- Leg (4) compares leg (6)'s two arms: P44's arm A (the modding crates linked and never called) and
  arm B (not linked).
- The configuration with the loader plugin installed is not a leg-(4) arm. Its startup delta is
  P2's budget, measured by MD:M-A3 and queued in MQ-09.
- Before any modding crate exists, a seam commit is compared with its parent.
- The brief tags no other open question of pass 3, so any answers to the others are in the part not
  received. See 03 §6 and 05 §1.

**Open for critic pass 4 (architect).**
- **The `.bss` table against the owner's one-allocator ruling (U-19 (d)).**
  - The static wins on every performance axis and closes C2 by construction.
  - It departs from the ledger's literal form for KF-45.
  - The architect decided it under principle 0's named threadpool exception; the owner can
    overturn it.
- **D0's restatement for windows-gnu.**
  - It is justified on performance: a one-time cost at launch, which is strictly less shared-state
    work than today's gnu `thread_local!`.
  - It touches another campaign's rule, and 00 §5 records the erratum it owes.
- **Leg (7)'s delta-0 on `profile.release`** depends on P6-1 reproducibility. If P6-1 fails, the
  fallback is the deterministic profile plus a spread band, as for leg (2).

**Open for critic pass 4 (writer's findings).**
- **V-45:** D-M6's `LANE` test relies on `ThreadPoolBuilder::build` calling `prepare()` first. No
  such call exists at `d552be05`; it is new code, named only in the part not received.
- **V-49:** "no R2 sweep edits `delete_entity_core`" is exact. However, ledger rows in
  `ecs_master.rs` (5) and `command_queue.rs` (3) sit in files that hold `swap_remove/10k` code, so
  an R2 sweep can still move that body. The file-map rule catches it; the bullet alone does not.
- **"O1" and "O2" now name two or three things each:**
  - the owner's steps (04 §3);
  - the file letters of waves RF-O1 and RF-O2;
  - each critic pass's observations.

  The RF-O1 row, for example, says "owner-dirty until O1". Different wave letters would remove the
  ambiguity.
- **Unprefixed M-series names outside the brief:**
  - the allocator's: 00 U-6 (`M-A2`) and U-9 (`M-A1`);
  - `M-A13` and `M-K3`: 00 RK-12 and 03 §5's structural list;
  - the modding design's: 05 MS-04 and MS-13 (`M-A4 (a)`, `M-A4 (b)`), §4 (`M-K3`, twice), and
    §5 R3-2 and P6-6 (`M-A4`).
- **Stale labels:** 00's index still lists MQ-01..MQ-19, though MQ-20 now exists, and every file's
  heading still says rev 3. Both are left to the P0 replacements.
- **V-44:** D-S7's cell says `CommandQueue::is_empty` is "inlined into the drain". The source
  marks it `#[inline]`, but no build confirmed the inlining.

### Rev 3 changelog (critic pass 2)

Rev 3 is the architect's patch answering critic pass 2, applied on top of rev 2; it is a patch, not
a full re-emission. It resolves the pass's four blockers (C1–C4) and six warnings (W1–W6), applies
its nine observations (O1–O9), and answers its three open questions. The architect read code in
`[J]` @ `d552be05`, documents in the `[M]` working copy, and `[G]`; it ran no graphify, cargo, git
or timing, and it read the web sources listed in §11. The writer re-verified every citation the
patch adds (§9, V-32 onward).

| Remark | Resolution | Where |
|---|---|---|
| **C1** | Applied the missing file-00 edits and removed the writer note. RK-1 now says P39/P40 have had no critic pass, and AP6 holds C1, D-S1(i), D-S2 and UG-15 leg (6). RK-2 now says EP3 holds D-S3(iii), AS2 and the engine-sourced D-E rungs; D-E0, D-E18 and D-E19 are exempt. Also: the status line, U-17 (numbers and overturn gate), and the §5 rows behind DOC-1/DOC-2. | 00 §3, §5, §6, §10 |
| **C2** | Chose option (a). KC-36 is narrowed and no longer claims entity ids. D-E0's test now checks rows, hooks and the *set* of ids, keyed on (system, spawn ordinal). Its group-slot check moves to D-S3(ii), because no group store exists when D-E0 lands. The X-14 correction goes into erratum E2. Option (b) is kept as a priced revival: U-20, owner question Q-9, measurement MQ-19. | 01 KC-36; 02 D-E0, D-S3(ii); 00 U-20, Q-9, §5; 03 MQ-19 |
| **C3** | Replaced the protocol. Each thread keeps, in an OS word only it writes, the address of its own record. There is no hash table and no displaced thread. Loom checks exclusivity, stability and exact refusal; three negative models must fail, and M3 is the rev-2 protocol on the critic's schedule. | 01 KC-04, §6, §7; 03 §3; 00 U-19 |
| **C4** | Leg (1) is now a `syn` census: plain and `unsafe(...)` attributes, any non-Rust-ABI fn definition, and global asm. It is never re-blessed; only the owner can add an allowlist entry, and the list is empty. KC-04 needs no callback. Legs (1), (4), (5) and (6) are identical in both modes. Leg (6) now compares the two modding arms of one commit, and is N/A until a modding crate exists. There are four red controls. | 01 §1 I-4; 03 §1, §2, §6; 05 §4 |
| **W1** | D-S3 is now three rungs with their own branches, touch sets and cut rules. The §4.3 registry arrows are first-cut-wins. | 02 §2, §3, §4.3 |
| **W2** | One open rung per worktree, from a fixed pool of three code worktrees. Lanes only order rungs. The queueing cost is stated. D-E18 now waits for B1. | 02 §1, §4.1; 04 §1 |
| **W3** | `drop_fn` is a constructor argument, so KC-16 uses 0 ids. D-S6 needs D-S2 and RF-K3; its touch set is its 7 rows' files. The rhi row moves to D-E15 and the `sparse_map` row to D-R2d. | 01 KC-10, KC-16; 02 D-S6, D-E15, D-R2d, §4.3; 00 U-8 |
| **W4** | RF-R and the new RF-T wait for B3. Every [G] file has a wave or "never", and the table supersedes [G] §2.4. New wave RF-E splits `gpu_column.rs` after D-E16. | 02 §3, §5 |
| **W5** | Under Miri the key comes from the portable `thread_local!` arm, and std's TLS destructors run (Miri supports both FLS and `.CRT$XLB`). D-M6 is on UG-08 with a Miri row. | 01 §6; 03 §2, §3 |
| **W6** | After A8, documents are edited only on the trunk, in `D:/wt/k-docs`. They merge after UG-10, UG-02 and UG-11 pass on the merged tree. Any main-branch commit is merged into the trunk before each fast-forward. | 02 §4.5; 04 §2, §4 |
| O1 | The redirect now *enqueues* the `Pinned` removal. `delete_entity` keeps `-> bool` (94 call sites in 50 files); `try_despawn` is added. D-E2's test gains a `Pinned` case. | 02 §2, §4.4; 01 §7 |
| O2 | Added a "UG-15 mode per rung" table; 03 §2 points to it. | 02 §2; 03 §2 |
| O3 | The stagger is taken at construction. The lazy state reuses `VmColumn`'s form plus `VmReservation::UNRESERVED`, not a new enum variant. The §6 "lost race" wording is corrected. | 01 KC-10, §6 |
| O4 | Per-host costs are stated, MQ-13 has three host arms, and fibers are outside the contract. | 01 KC-04, §6; 03 MQ-13 |
| O5 | The store-drop assert is skipped while panicking. | 01 §7 |
| O6 | RF-0's gate is a snapshot-equality test, not UG-15. | 02 §5; 03 §2 |
| O7 | New rule 9 makes 03 §2 authoritative for gate lists. The D-S2 and D-S3 cells now list UG-03 and UG-04. | 02 §1, §2; 03 §2 |
| O8 | Added §4.3 rows for `boyko_threadpool`, `boyko_diag`, `boyko_log` and D-M6's ECS files. | 02 §4.3 |
| O9 | `Release` is now an associated type, and the Stamped-only APIs are bounded. Misuse is E0271 before monomorphisation, so the fixture does not depend on trybuild's build mode, which the architect could not verify. | 01 KC-12, KC-13; 02 D-E9, D-S3(iii) |

**Where each item of the rev-2 KC-04 protocol went (C3).** Rev 3 replaced the whole protocol block
in 01 §6.

| Rev-2 item | Disposition |
|---|---|
| 1 key (TCB) | deleted, replaced by item 2 |
| 2 hashed lookup | deleted; it was the C3 hazard, and M3 keeps it as a negative model |
| 3 linear-probe registration and FLS/pthread callback | replaced by item 4 and std's destructor; the callback was the C4 conflict |
| 4 release order | kept in spirit and reordered: word, then bit |
| 5 inheritance | deleted; unsound per S-3 |
| 6 loom model | replaced by item 7 |
| 7 route per row | decided now, in item 8 |

**Corrections the architect found in rev 2 itself.**
- **S-1: KC-04 cannot live in `boyko_memory`.**
  - Allocator G5 requires `#![no_std]` for that crate (`ALLOCATOR-DESIGN-SPACE.md:330`), and the
    only callback-free exit hook is std's TLS destructor.
  - `boyko_diag` may have no workspace dependency (`[J]crates/boyko_diag/Cargo.toml:6-16`).
    `boyko_log` may depend on diag only (`[J]crates/boyko_log/Cargo.toml:7-18`).
  - So two of the KF-45 crates could never reach the rev-2 location.
- **S-2: D-E0's rev-2 test used a group anchor that does not exist yet.** D-E0 lands before
  D-S3(ii) (fixed order #2), and D-S3(ii) creates group stores.
- **S-3: rev 2's inheritance rule (item 5) was unsound for frame fields.** A slot is inherited only
  when its thread did not exit normally, which is exactly when the RAII guards did not reset those
  fields.
- **S-4: `boyko_diag`'s `LANE` names a role, not a thread.** `install` relabels it
  (`[J]crates/boyko_diag/src/lane.rs:27-32`), so it cannot be the identity key. Rev 2's hazard
  would reappear.
- **S-5: KC-06 claimed all of checkpoint defect 5.** Its "no parallelism" half had no rung.
- **S-6: rule 5 said rungs are "S or M".** §2 lists more than ten L rungs.
- **S-7: two KF-02 rows cannot retire in D-S6.** The rhi row needs the KF-36 edge (D-E15); the
  `sparse_map` row needs D-M1's move.
- **S-8: D-E18 would retire 77 rows at A8,** before B1 pins UG-02, against rule 2.
- **S-9: UG-08's rung list omitted D-E9,** which has a Miri row in 03 §3.
- **S-10: Microsoft calls a TLS index "an opaque value"** and says not to access the TEB directly.
  A per-process canary now guards the direct read (new RK-14).
- **S-11: a text grep for `#[no_mangle]` misses Rust 2024's `#[unsafe(no_mangle)]`.**

**Answers to the critic's open questions (pass 2).**
1. **Does strict leg (2) normalise data-symbol names?**
   - Now it does. RIP-relative data references are compared by symbol name through a declared
     rename list (new 03 §6 row), and D-S1(ii)'s list is `TAG_NAMES → DYN_NAMES`.
   - Without it, strict D-S1(ii) would go red on relocation text alone: red for the wrong reason.
2. **Why was the choice between a thread slot and an explicit context deferred?**
   - It no longer is; 01 §6 item 8 decides all 12 rows:

     | Route | Rows |
     |---|---|
     | record (slot) | 8 |
     | diag-private word | 1 (`LANE`) |
     | OS thread id | 2 (log tokens) |
     | explicit context | 1 (`BUILDING`) |

   - The one condition: `required.rs`'s cut must show the DFS calls no code outside the file
     between push and pop. Otherwise that row takes the record route.
3. **Which rung retires checkpoint defect 5?** (`CHECKPOINT-2026-09-11.md:94`)
   - The defect has two shares, and KC-06's "Deletes" claimed both.
   - **Allocation share:** D-M2 and D-M3. The gate is UG-03's W = 1 arm of scene S1c, pinned at 0
     in the steady window.
   - **Serial share:** D-M5. `par_range` and `par_phases` run inline when the pool has one worker;
     red-first: a W = 1 physics step constructs 0 scopes. Physics P2 is the consumer.

**Left untouched by rev 3.** The positives critic pass 2 asked to keep: D-S1's split, the window
argument, the `delete_entity_core` order apart from step 3's enqueue, the `ArchetypeFlags` bit plan,
the KC-10 identity layout, the `TeardownToken` shape, the fleet re-read, U-1, and the modding-cost
findings. The kernel still keeps compile-time type identity, storage and dispatch; no cargo feature
is added, and every seam item still goes through strict UG-15. Nothing was timed: MQ-13 and MQ-19
are entries only. The plan stays unapproved until critic pass 3.

### Rev 2 changelog (critic pass 1)

C-rows are the pass's blockers, W-rows its warnings and O-rows its observations. Each row names how
the remark was resolved and where the resolution lives.

| Remark | Disposition | Where |
|---|---|---|
| C1 P39/P40 labelled reviewed | RK-1 corrected; steps DOC-1 → AP6 (scope Rev 2.4 + Rev 2.5); AP6 gates C1, D-S1(i), D-S2, UG-15 leg (6) | 00 §5, §6; 02 §2, §3 |
| C2 seam delta ungated | D-S1 split: (i) bug fixes, UG-15 attributed; (ii) seam, UG-15 strict against (i); moved items leave the kernel | 01 KC-19a/b; 02 D-S1; 03 §1, §2, §6; 05 §3, §6, §7 |
| C3 unlocked shared paths | per-rung, per-file locks; touch sets; fixed orders; step order in `delete_entity_core`; D-E2 after D-S3 with a combined test; RF-K3 before D-S3, D-S4, D-M1 | 02 §2, §4; 01 §7; 00 RK-13 |
| C4 K6′ unreviewed, token dropped | EP3 gates D-S3(iii); KC-13 (a)/(b); `TeardownToken` restored with trybuild fixtures; teardown form refused for Immediate and Chained; `clear()` stamps | 01 KC-13, KC-27, R-C; 02 D-S3, D-E9 |
| W1 contract items without rungs | KC-30b/c → D-E18/D-E19; KF-46 has no kernel delta (HO3); KF-05 = `WorldScratch` in D-R2a | 01 KC-10, KC-30, KC-34; 02 |
| W2 R0 and apply order | B4 restores the census; KC-36 built as D-E0 | 02; 01 KC-36 |
| W3 gates that cannot fail | `Table` owner producers; leg (2-RF); owner accepts binary growth above +1 % | 01 KC-01, KC-18; 03 |
| W4 worktree topology | trunk worktree = joltab; `D:/wt/k-phys`; D-S2 touch set widened | 02 §4; 04 |
| W5 A8, RF-R timing | A8 is a code merge (UG-12/17/18); RF-R after A8; A4 split into A4a/A4b | 02; 04 |
| W6 KC-04 shared state | protocol, orderings, exit callback, inheritance classes, loom | 01 §6; 02 D-M6; 03 §3 |
| O1–O11 | applied (O1 cost, O2 invariants, O3 MS-01, O4 U-17, O5 CK- prefix, O6 A4a order, O7 prune order, O8 D-M0, O9 probe sites, O10 citations, O11 flag budget) | 00, 01, 02, 04, 05 |
| Open questions 1–6 | answered | 03 MQ-03; 01 KC-10; 01 R-C; 05 MS-13; 02 R2 |

**Answers to the critic's open questions.**
1. **Does MQ-03 block D-S2's merge?** No. D-S2 changes no loop body, only each scratch pool's
   starting cache line, so a regression is repaired by re-deriving the stagger (MQ-16), not by a
   revert. MQ-03 is still filed (03 §5).
2. **Where does a registry-free pool keep its identity?** In `component_id: u32` + `stagger: u32`,
   the same 8 B as today's field, with a sentinel id; the layout is already stored by value and the
   `TypeId` comes from `T` (01 KC-10).
3. **What is `for_type`'s stagger policy?** A process-global round-robin seed: cohorts take a
   contiguous run, singles take consecutive lines, and MQ-16 measures both (01 KC-10).
4. **Which crate declares the asset groups?** `boyko_render`, whose group markers are private, so
   `render_retire::<K>` can build the ChainKey (01 R-C).
5. **Is MS-13 re-derived after D-S1?** Yes, as an obligation of its Stage-3 rung (05 MS-13).
6. **Are the D-R2a..d counts net?** No, gross. Net counts are re-derived at each cut, and a sweep
   over 2000 changed lines is split (02 R2 sweeps).

**Open after pass 1 (not blockers).**
- **R1 without a value-type change.** The rename-only R1 and the separate sized mint body depart
  from allocator P32 R1 / P40 as written. AP6 must confirm that `LAYOUTS[id]` is a sound layout
  oracle under the intern lock.
- **KC-36's determinism argument.** Critic pass 2 traced it and confirmed it for apply order
  (`[J]…/schedule/schedule.rs:678-680`, `:809`, `:849`, the scan at `:1079-1132`), and refuted it
  for entity ids (U-20). D-E0's red-first run is the proof of the narrowed claim.
- **KC-04 exit context.** Rev 3 defines no engine exit callback. The guard's `Drop` runs inside
  std's existing TLS-destructor runner, takes no lock, allocates nothing, and makes one
  `TlsSetValue` call on Windows. D-M6's exit test is the check.

## 11. External sources (read 2026-09-17)

Read by the architect for rev 3 (U-19, U-20, RK-14, 01 §6, O9) and for rev 5 (01 §6 items 3–8;
03 §5, §6).

- [Microsoft, `TlsAlloc`](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-tlsalloc):
  the slots of a new index are initialised to zero; on failure the return value is
  `TLS_OUT_OF_INDEXES`; the index "should be treated as an opaque value; do not assume that it is an
  index into a zero-based array" (rev 5 re-read).
- [Microsoft, `TEB` (winternl.h)](https://learn.microsoft.com/en-us/windows/win32/api/winternl/ns-winternl-teb):
  the struct definition (so `TlsSlots` is at `+0x1480` on x64), and the instruction not to access
  the structure directly.
- [NtDoc, TEB](https://ntdoc.m417z.com/teb)
- [cbloom, "Fast TLS on Windows"](http://cbloomrants.blogspot.com/2013/09/09-18-13-fast-tls-on-windows.html)
- [Miri TLS shim](https://doc.rust-lang.org/stable/nightly-rustc/src/miri/shims/tls.rs.html): runs
  both FLS (`FlsAlloc`) and `.CRT$XLB` destructors on Windows targets.
- [Rust std, `LocalKey`](https://doc.rust-lang.org/std/thread/struct.LocalKey.html) (rev 5):
  - `try_with` returns an `AccessError` "if the key has been destroyed (which may happen if this is
    called in a destructor)";
  - TLS may re-initialise other slots during destruction;
  - on Windows process exit, TLS destructors may run only on the exiting thread;
  - a thread converted to a fiber runs no destructors unless it is converted back.
- [syn, `Macro`](https://docs.rs/syn/latest/syn/struct.Macro.html) (rev 5): `tokens` is the token
  stream inside the invocation's delimiters, kept uninterpreted; `parse_body` parses it on request
  (03 §6 leg (1)).
- [cargo PR #13257](https://github.com/rust-lang/cargo/pull/13257) (rev 5): merged 2024-01-15,
  shipped in 1.77.0. When `strip` is unset and no debuginfo is requested, Cargo sets
  `strip = "debuginfo"` implicitly.
- [Rust 1.77.1 announcement](https://blog.rust-lang.org/2024/03/28/Rust-1.77.1/) (rev 5): that
  behaviour is disabled on Windows msvc targets (03 §6 leg (7)).
- [`cargo bench`](https://doc.rust-lang.org/cargo/commands/cargo-bench.html) (rev 5):
  `--profile <name>` benchmarks with the given profile; the default is `bench` (03 §5).
- [Unity samples, entity command buffers](https://github.com/Unity-Technologies/EntityComponentSystemSamples/blob/master/EntitiesSamples/Docs/entity-command-buffers.md):
  temporary negative-index ids, and playback sorted by sort key.
- [Unity docs, ECB playback](https://docs.unity3d.com/Packages/com.unity.entities@6.4/manual/systems-entity-command-buffer-playback.html)
- [rust-lang/rust #99682](https://github.com/rust-lang/rust/issues/99682): `cargo check` misses
  post-monomorphisation errors.
- [trybuild docs](https://docs.rs/trybuild): the docs do not say whether the harness checks or
  builds a fixture.
