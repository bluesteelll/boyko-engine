# Unified system plan — 00 Overview (rev 6.2)

## Index

| File | Content |
|---|---|
| [UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md](UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md) | this file: goal, target metrics, rulings U-1..U-28, phase map, source-document patches, risks, owner questions, readiness, citation verification record, revision log, external sources |
| [UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md](UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md) | contract invariants, the unified kernel features KC-01..KC-37 (KC-19 split into a/b, KC-30 into a/b/c; KC-37 is the replay determinism contract of §2.1 with its hazard register H-01..H-21), source-id mapping, conflict resolutions, threading model (with the KC-04 thread-context protocol, its capacity rule and the plan-build set), invariants and edge cases |
| [UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md](UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md) | sequencing rules, rungs per phase (A–F) and the design-pass steps, DAG, worktrees, per-rung file locks and fixed edit orders, refactor waves and the refactor-worktree priority rule, the anchor merge step, ledger-rung mapping |
| [UNIFIED-SYSTEM-PLAN-03-GATES.md](UNIFIED-SYSTEM-PLAN-03-GATES.md) | gate inventory UG-01..UG-22, gates per rung, Miri and loom rows, device legs, measurement-queue entries MQ-01..MQ-23 and the timing-profile rule, the no-cost gate UG-15 (legs (1)–(7b), strict and attributed modes, nine red and three green controls), the replay determinism gate UG-22 (ten red controls and one green control) |
| [UNIFIED-SYSTEM-PLAN-04-INTEGRATION.md](UNIFIED-SYSTEM-PLAN-04-INTEGRATION.md) | the branch fleet, merge order, the owner's steps, trunk policy, prune list |
| [UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md](UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md) | modding as testable properties under the owner's zero-overhead requirement H-1 and rule S-1 (kernel modding code is generic over `ModSeam`, so it is not compiled when unused), what stays compile-time, what each live option needs from the kernel, additive modding paths MS-01..MS-15 per option, reconciliation with allocator §7, carried remarks (RM-1..RM-4 new), the no-cost gate, modding sequence |

**Status (2026-09-17): rev 6 closed by orchestrator ruling after critic pass 2, which found no Critical remark; its remaining remarks are OPEN and listed in section 10.**
Rev 6 applies the owner's answers of 2026-09-17 to Q-1 (native mods), Q-2 (load-only), Q-4 (refactoring last), Q-5 (commit the open work), Q-7 (the msvc host) and Q-9 (replays on any machine without matching entity ids) (§7).
- **Rev 6** applied the owner's answers of 2026-09-17 (§7) to files 01–05.
  Its file-00 part (P6-00-1..13) was not applied (§9, rev 6 re-verification).
- **Rev 6.1** is the architect's patch answering critic pass 6 (pass 1).
  It brings this file to rev 6 (C1) and resolves W1–W8 and O1–O11 (§10, rev 6.1 changelog).
- The architect's role cannot write files. A writer applies each revision and
  re-verifies every `file:line` citation it adds (§9).

**History.**
- **Rev 5.1** was closed by orchestrator ruling after critic pass 5, which found no
  Critical remark. Its W1–W4 are resolved in rev 6 (§10).
- **Rev 5** re-emitted the unapplied parts of rev 4.
- **Rev 5.1** was also a writer's reconciliation of files 01 and 05 with the closed
  modding design.

**Provenance.**
- **Trees read:**
  - `[J]` = `D:/wt/joltab`, `merge/ke16-into-ecsnative` @ `d552be05` (read-only).
  - `[M]` = `D:/claude/BoykoEngine`, `feat/multi-paradigm-render` @ `b716a5dc`.
    - The owner's work is committed (Q-5). Only `.claude/settings.local.json` and two
      git-ignored model archives are outside git.
    - `docs/memory/ALLOCATOR-DESIGN-SPACE.md` is committed (`ac86fc38`, 4,158 lines).
      A citation read in its pre-commit working copy is 7 lines low (§9 V-61); rev 6.1
      re-points those.
  - `[Jw]` = the `D:/wt/joltab` working copy. It is read only outside `boyko_physics`,
    because a workflow edits physics there. The writer verifies each `[Jw]` citation at
    `d552be05`.
  - `[C]` = `D:/wt/census` @ `27ac8904`.
  - `[W]` = `D:/wt/msvc` @ `a36ceaa4`.
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
- **Replays reproduce (owner Q-9).**
  - The same binary reproduces the same simulation on any machine and at any worker
    count, from recorded tick inputs.
  - Replays name entities by stable keys, never by `Entity` ids (01 §2.1, KC-37).
  - A game that does not link `boyko_replay` pays nothing: no object code and no added
    per-frame lookup (UG-15 legs (7) and (7b); U-25).

**How performance is argued:**
- **Allocation removal is not a speed argument.** It is justified by the owner's ruling, by tail
  latency, and by the structural deny gate. The heap A/B was null: mi/sys = 0.998 / 1.023 / 0.992
  at W = 1 / 8 / 16 (`[M]docs/unification/CHECKPOINT-2026-09-11.md:33`). Allocator P0 withdrew
  speed as a justification (`[M]docs/memory/ALLOCATOR-DESIGN-SPACE.md:563-567`).
- **Throughput is claimed only where the ECS form removes serial work.**
  - The Jolt pyramid at W=8 is 2.47× slower than Jolt, with a serial fraction of 0.43–0.53
    (`[M]…/CHECKPOINT-2026-09-11.md:36`).
  - The broadphase runs an `AllPairs` O(n²) sweep of 769,420 pairs (`:45-46`).
  - Every such claim is gated by a quiet-window measurement (03 §5).

## 2. Target metrics

| Metric | Today (tree) | Target | Gate (03) |
|---|---|---|---|
| Ledger in-scope rows outside an ECS / kernel form | 2357 active, 691 (29.3 %) in ECS form, 1102 out of scope (`[M]docs/memory/RUNTIME-DATA-LEDGER.md:30`) | the count only decreases; end state 0 | UG-02 |
| Heap acquisitions, App frame (S0) | flat 2 per `Schedule::run` (`ALLOCATOR-DESIGN-SPACE.md:1131`) | 0 after D-M2 | UG-03 |
| Heap acquisitions, parallel pile step W=4 (S1c) | 302..339 (`:1136`) | 0 steady after D-M2 + D-M3 + P2 | UG-03, UG-05 |
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
| Replay: tier-1 hash sequence of each UG-22 scene under W 1/2/8, pacing, id perturbation, Main churn, UCRT FMA3, and two machines running one file | no gate | equal to the scene's golden in every arm | UG-22 |
| Kernel cost of replay support to a game that does not link `boyko_replay` | n/a | 0 object code (KC-37 (g), D-E22); 0 added lookups per frame (D-E23) | UG-15 legs (7), (7b); code review of D-E23 |

## 3. Rulings this plan makes

Each ruling is decided by performance and names the gate that would overturn it. Details are in
01 §2.1, 01 §4 and 05 §4.

| # | Ruling | Numbers | Overturned by |
|---|---|---|---|
| U-1 | **The allocator's Heap class** (`Heap`, `HeapRef`, `HeapVec`, `HeapBox`, `HeapDyn`, `SortedMap`) **is deferred, not built.** Its clients take ledger forms: K7 spans, the erased record column, VmColumn tables, and sorted ScratchColumn pairs. | Per-frame cost equal (build-time clients). Heap: eager Miri 1.26 MiB per `EcsMaster`, VA 2 GiB per master, worst resident 204 KiB (`ALLOCATOR-DESIGN-SPACE.md:3171-3172, 2718`), plus new TB-sensitive unsafe surface. Forms: 4 KiB floor per column after D-M1, no new unsafe. | Ledger rev 5 finds a row that needs individually-freed, variable-size, non-`Copy` storage owned by `schedule`/`registry`/`master-table` and not expressible as KC-15/17/18; or modding Stage 3 needs mod-private freed memory (then the heap lives in `boyko_mod_host`) |
| U-2 | **Scratch cohorts are registry-free.** They use zero `ComponentId`s, and the physics scratch band is deleted. | +128 ids for physics builds; the ~90 physics scratch ids (`scratch_ids.rs:681`) and the 17–19 FrameGraph ids (`RUNTIME-DATA-LEDGER.md:874`) go to 0. Allocator P43's "1 id per element type" is superseded. | D-S2 finds an untracked-pool path that needs a registered id (then the P43 rule applies) |
| U-3 | **K6′ is re-filed against physics rev 5.** A group's release policy is `Immediate` \| `Chained` \| `Stamped`. `Stamped` never releases at removal and never takes the out-of-chain flush; it releases only through `GroupHead::release_dying_before(&ChainKey, horizon, visit)`. Span-typed group columns free their spans inside every kernel release point. | +4 B per dying entry for `Stamped` groups only; 0 bytes for `PhysicsBody` | correctness only: AS2's FIF-slot-reuse proptest |
| U-4 | **KF-33's form is the allocator's per-claimed-slot chunk lists** (`ScopeShared` in the block, `ChunkArena`, `SlotChunks`), not a per-thread mark/rewind arena | ≤ 390 KiB/pool at W16 D8 (derived, `:1794`); 0 heap calls per App frame | AL:M-A10 measures > 4 MiB per pool |
| U-5 | **KF-34 is narrowed to the injector ring.** The per-lane Chase-Lev deques stay crossbeam in v1. | injector = one 1520 B block per 64 outside pushes; lanes double once (`:1159`) | UG-03 shows lane growth, or the epoch `Local`, in a steady window after 1f |
| U-6 | **`ScopeShared` becomes the first cell of its `ScopeBlock`** (allocator P2). The ledger's frame-local placement is not used. | both are 0 allocations after warm-up; the block placement is covered by the Miri poison-write gate (P3) | AL:M-A2 shows a first-chunk layout cost |
| U-7 | **`TableSet` is deferred.** Fixed kernel tables become fixed-capacity `VmColumn`s, or inline arrays inside one existing reservation (KF-31). | 4 KiB floor per table after D-M1 | UG-20 records > 64 KiB of sub-page tables per `EcsMaster` |
| U-8 | **The owning column (KF-02/EK12) is a typed view over an untracked `ComponentPool` with drop glue.** The allocator's `DropColumn` primitive is not built. The drop glue is passed by value from `T` through KC-10's registry-free constructor, so the column uses **no `ComponentId`** (U-2). | the pool already stores `drop_fn` (`RUNTIME-DATA-LEDGER.md:909-910`; `[J]…/memory/component_pool.rs:235-237`), today read from the registry by id (`:284-288`). Passing it by value costs 0 ids, against today's route `register_asset_layout::<T>` (`[J]crates/boyko_ecs/src/ecs/core/asset/backing.rs:115`), which takes one id per type. No second droppable column type to cover under Miri | none expected. A KF-02 consumer that needs the column inside an archetype bundle takes a registered id, and the MS-03 D2 census counts it |
| U-9 | **Erased objects** (`Box<dyn System>`, `Box<dyn FnOnce>`, resource values) are stored in KF-07 records behind the allocator's `ErasedSystem {data, vtable}` handle. | append-then-drop-all; one indirection, as today. Bevy's proposed dedicated resource storage measured −47 % `get` ([#24058](https://github.com/bevyengine/bevy/pull/24058)) but was closed unmerged; the merged fix moved resources to sparse-set storage, `get` −10 % and `get_mut` −39 % ([#24077](https://github.com/bevyengine/bevy/pull/24077)) (§9 V-22) | AL:M-A1 dispatch floor regresses beyond its band |
| U-10 | **Modding:** load-only (no K-MOD-10 `remove_component_type`); v1 mod components are POD; crates are named per modding rev 6; the id bound is D2 plus allocator P39's skip-on-occupied mint; the refusal signal (item 2b) lives at the boundary. | removes one class-A kernel function; 0 capacity charge | the owner overturns Q-2: mods must unload or hot-reload (05 §8) |
| U-11 | **Allocator G6 and modding M-P1 become one gate, UG-15.** Pins are captured on each modding delta's parent. | — | none |
| U-12 | **Refactor last (owner scope, Q-4; the rev-5 split-first rule is withdrawn).** No file is split before Phase F step F4 (02 §5). Until then, rungs edit files as they stand, under per-rung, per-file locks (02 §4). | [G] counts 58 files and 171,250 lines (`[G]:31-35`). Editing unsplit files lengthens the lock queues; RK-5 prices that. The census and the closed refactor design are kept for F4. | An owner scope ruling (Q-4 is the ruling of record). RK-5's trigger (more than three rungs queued on one file) raises an owner question. |
| U-13 | **Kernel base = trunk `integ/unified`,** cut from `[J]` after Phase A plus the owner's steps | one tree for every gate and for ledger citations | — |
| U-14 | **Binary size is a per-rung gate; compile time is record-only.** | Bevy's revert PR for #20934 (#22915, approved, then closed unmerged) cited compile time +8–12 % and binary +5–7 % (RK-9; §9 V-21) | — |
| U-15 | **KF-12 (multi-target relation) is not built.** System-set edges are edge entities at schedule build. | build-time only | MQ-14 schedule-build time beyond its band |
| U-16 | **Default-excluded entities are an archetype flag,** tested at archetype match | 0 per row | UG-15 `query_ref_iter` pin moves |
| U-17 | **Physics NB2:** S1 and S6 become `pub(crate)` and are registered only through `PhysicsPlugin`; every non-panicking S6 exit reaches `close_chain` (`PHYSICS-ECS-UNIFICATION-DESIGN.md:3799`) | 0 runtime instructions: this is a visibility change. The rejected alternative, a runtime "chain systems registered together" check, costs one build-time scan per schedule and still cannot see a user system that calls neither. Gates: a UG-17 fixture (registering S1 from outside `boyko_physics` → E0603) and a U4 test that `chain_open` is clear after every S6 exit path | an owner scope ruling that games may assemble the physics pipeline from individual systems; S1/S6 then become `pub` behind a typed builder that registers both or neither |
| U-18 | **KF-45 stays** at the end of lane MEM (D-M6) as a unification rung: one per-thread record replaces 12 `thread_local!` cells. The host moved to msvc on 2026-09-17 (Q-7), so its windows-gnu speed case no longer concerns the gate host. Its form is U-19. | The gnu floor is recorded by MQ-13's gnu arm (record-only). That floor is 2 locked RMWs on one process-global line plus `FlsSetValue` per `thread_local!` read under rustc ≥ 1.98 (`[J]docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:40-56, 82`), and `target_thread_local` is off on gnu only (`:87-95`). | MQ-13's msvc arm (U-19 (b), (c)); U-19 (a) if gnu becomes a shipped or gate host again |
| U-19 | **KC-04 is one per-thread word holding the index + 1 of that thread's kernel record: the portable arm, on every host (rev 6).**<br>• **The word.** A const-initialised, `Drop`-free `thread_local!` `Cell<usize>`. There is no hashed key table and no materialisation step.<br>• **Records and claims.** Records are 64-B rows of `THREAD_RECORDS`, a fixed 8192-row `.bss` static. They are claimed through `THREAD_BUSY`, a 128-word busy bitmap, also in `.bss` (01 §6 items 1–2).<br>• **Pool builds.** A pool build claims its workers' slots in one batch, clamped so that 2048 slots stay free for every other thread. Each worker adopts its slot, so a worker is never refused.<br>• **Zeroing.** A record is zeroed at release, its only zeroing site, and is never inherited.<br>• **Placement.** The primitive lives in `boyko_threadpool`. `boyko_diag`'s `LANE` stays its own `thread_local!`, and `boyko_log`'s tokens use the OS thread id (01 §6 item 10).<br>• **Thread exit.** The hook is std's TLS destructor on a guard (`EXIT_GUARD`) that only non-worker threads touch.<br>• **Build.** `ThreadPoolBuilder::build`'s first KC-04 statement is `claim_for_pool` (01 §6 item 4).<br>• **Not built:** the windows word arm (`TlsAlloc` slot read at `gs:[0x1480 + 8·index]` after a canary). Its reviewed text is the revival form D-M6w (01 §8). | **Hot read (msvc, Linux):** 1 native TLS load, 1 null test, then `shl` + `lea`. That is one load and one branch more than a native `thread_local!` field, with no lock and no call. No thread ever probes; rev 2's hashed lookup displaced a thread in ≈ 42 % of processes with 17 threads.<br>**windows-gnu** (comparison host): std's OS-key read. MQ-13's gnu arm records it.<br>**Per non-worker thread:** one cold claim. On gnu, add one `os-thread` System allocation and one std `enable()` for `EXIT_GUARD`.<br>**Per worker:** the adopt write only.<br>**Per process:** 525,312 B of `.bss` virtual size. Resident memory is ⌈peak claimed / 64⌉ × 4 KiB, i.e. 3 pages at the gate host's 170-thread worst case (01 §6 item 2). No `TlsAlloc`, no canary.<br>**Removed by not building the word arm:** two TLS indices, two canaries, the `asm!` read, `prepare()`, and erratum D0-1 (RK-14 is scoped to D-M6w). | MQ-13, per host. A re-seam is D-M6r; building the word arm is D-M6w.<br>**(a)** The owner makes windows-gnu a shipped target or the gate host again → D-M6w's gnu arm is built and priced by MQ-13's gnu arm.<br>**(b)** On msvc, MQ-13's measurement-only prototype of the word arm with direct reads is faster than the portable arm beyond the band → D-M6w is built on msvc.<br>**(c)** A host's record route is slower than native `thread_local!` beyond the band (the ledger's own overturn, `RUNTIME-DATA-LEDGER.md:1627`) → that host keeps `thread_local!` for these rows, recorded as a ledger exception with the number (D-M6r).<br>**(d)** The owner rules that a fixed `.bss` static does not satisfy the one-allocator ruling for KF-45 → the records become a `VmColumn<ThreadRecord, TableOwner>`, reserved once and published by CAS, at +1 load per lookup. Loom M4, the check-then-write form, stays the negative model. |
| U-20 | **Entity ids are not made deterministic across runs or worker counts.** KC-36 and KC-37 make apply, command and hook order, table rows and dense/group slots deterministic. Ids stay dependent on claim order, because `Commands::spawn` claims them on the worker during the phase (`[J]crates/boyko_ecs/src/ecs/core/system/params/commands.rs:169-173`). Replays name entities by stable keys instead (Q-9; U-24). | **Cost:** 0.<br>**Rejected (a):** placeholder ids resolved at apply, Unity's form (its command buffer returns temporary negative-index ids, §11). It changes `.id()`'s contract and costs one remap per placeholder reference at apply.<br>**Rejected (b):** per-system id leases carved by the dispatcher in index order. They replace the locked RMW per claim with a plain bump, but are deterministic only while no lease overflows. A fresh-id remainder then either leaks or needs inland-slot growth to keep the reservoir's F3 invariant (`[J]…/entity/entity_reservoir.rs:39-41`).<br>**Id-value consumers:** the one engine consumer that compared id values, physics row identity with generation-less ids (H-03), is fixed by A1b. The owner's Q-9 answer requires keys, not ids. | MQ-19 shows the reservoir line costing > 2 % of a spawn-heavy W = 8 frame. Then (b) is built for throughput, with its overflow counter pinned at 0. |
| U-21 | **Event lanes are keyed by writer** (D-E20, KC-37 (b)).<br>• `EventWriterState` holds a lane index assigned at `init_state`, and `send` reads no TLS.<br>• `EcsMaster::events().send_event` is dispatcher-only: on a worker it returns an error instead of writing. | **Per send:** one TLS read fewer (`[Jw]…/system/params/event_writer.rs:131-133`, `:162-164`).<br>**State size:** stays 24 B (`:50-63`).<br>**`send_event`:** its dispatcher check replaces today's lane routing read (`[Jw]…/events/event_dispatcher.rs:290-292`), so the TLS read count is the same.<br>**Memory:** one lane per writer instead of per worker. That is lower for types with ≤ W writers and higher above; UG-20 records it.<br>**Determinism:** order and refusals stop depending on W (H-02). | MQ-21: `send` or `update_events` slower beyond the band, or memory above UG-20's band → worker lanes plus a merge at the swap by (writer index, per-writer sequence). |
| U-22 | **The gate host is msvc** (Q-7).<br>• Every CPU gate, Miri leg and loom recipe runs on `stable`/`nightly-x86_64-pc-windows-msvc`.<br>• windows-gnu is a comparison host: the Miri comparison leg, MQ-13's record-only arm, and the sensitivity-map fallback.<br>• UG-15's symbol legs read the post-LTO object through the dev crate `boyko_symcensus`, because `link.exe` writes no COFF symbol table into the image (`[C]crates/profile_fixture/tests/profile_axis_census.rs:46-78`). | **msvc legs on `e6115223`** (owner, 2026-09-10): 5582 passed and 2 failed, both census cells, fixed by `27ac8904`. Miri agreed with gnu on 3 targets. Goldens were 32/32 byte-identical.<br>**Nightly miri:** msvc 2026-09-09; gnu 2026-08-20. | **(a)** The owner moves the gate host back → AH's re-bless is repeated in reverse.<br>**(b)** B3's probe shows that the post-LTO object cannot carry a pinned body → that leg reads the gnu comparison build of the same commit, as the sensitivity map's fallback does. |
| U-23 | **Simulation math is in-house** (`boyko_math::det`, RP-0).<br>• Transcendental functions are built from `+ − × ÷` and exact `sqrt`.<br>• No libm; no std transcendental call in simulation crates (a census enforces this).<br>• `boyko_math::rng` is a counter-based stream `(seed, key, counter) → u64`. | **Why:** the same binary diverges without it.<br>• UCRT selects FMA3 or non-FMA3 code per CPU and documents that results can differ between machines (§11).<br>• Rust documents `sin` precision as varying by platform, and even within one execution (§11).<br>**Surface today:** 2 non-test simulation calls (`cbrt`, H-12), plus 5 Main-only camera calls and 1 render `tan`.<br>**Precedents:** Jolt's whole deterministic configuration is documented at ~8 %; Box2D claims no noticeable cost (§11).<br>**Timing:** MQ-22 times `det` against std. | MQ-22: a shipped hot path slower beyond its band → per call site, a table or a reformulation, never std. |
| U-24 | **Replay keys** (`boyko_replay`, RP-2).<br>• `ReplayKey(u64)` is a component.<br>• **Root keys:** `0 \| tick(31) \| slot(12) \| ordinal(20)`, from `ReplaySpawner`, where `tick` is the replay tick index.<br>• **Derived keys:** `1 \| mix63(parent_key, tick, c)`, where `c` is a per-(parent, tick) counter read **when the key is issued inside the apply window**. The caller passes no index.<br>• `ReplayKeyIndex` is an open-addressed key → `Entity` table in a resource-owned kernel column. | **Memory:** 8 B per keyed entity.<br>**Per root spawn:** one increment.<br>**Per derived spawn, and per index insert or remove:** one table probe, O(1) expected. The rejected sorted column costs an O(n) memmove per randomly placed key (critic pass 6, O5).<br>**Why an issue-time counter:** a caller index collides across two spawn sites under one parent (critic pass 6, W4). An issue-time counter is deterministic because commands apply in KC-36 order and hooks fire in D-E21 order.<br>**Non-replay games:** 0. | **(a)** MQ-23's spawn-churn arm shows the probes above its band → per-archetype key columns with no index.<br>**(b)** A duplicate-key panic in any shipped recording → a wider key. |
| U-25 | **Frame-level fixed-loop values live in `Time`** (D-E23, H-20).<br>• `fixed_steps()`, `fixed_overstep()` and `fixed_overstep_fraction()` move out of `FixedTime`.<br>• `FixedTime` keeps only tick-level values: `timestep`, `delta`, `delta_secs`, `elapsed`.<br>• Rule B's existing refusal of a Fixed read of `Time` then covers the three. | **Per frame:** 0 added lookups. `fixed_advance` still makes one post-loop resource lookup (`[Jw]…/time/fixed_loop.rs:82-83`), now of `Time`. The last `expend` returns the remainder, so the debug assert needs no second lookup.<br>**Rejected:** a rule-only restriction, because no check can see which getter a Fixed system calls; and a separate `FixedFrame` resource, which costs one more insert at `finish` and one more rule-B entry for the same runtime. | An owner API ruling that `FixedTime` keeps the getters → a `FixedFrame` resource with the same refusal (same runtime cost). |
| U-26 | **A structural op with no declaring bundle fires hooks in canonical type order** (D-E21).<br>• Applies to despawn, clone/materialize, and archetype-driven removes.<br>• Order: `ComponentLayout::type_name` bytes, ties broken by `TypeId` (both fixed within one binary).<br>• Inserts keep declaration order (KC-37 (c)).<br>• The permutation is built lazily, once per archetype, on the first cold hook fire, and kept in two `VmColumn`s owned by `ArchetypeMaster`. | **Hook-free path:** 0, because the permutation is built on the `#[cold]` fire path (`[Jw]…/ecs_master/entity_api.rs:705-707`).<br>**Per archetype that ever fires such a hook:** one O(n log n) string sort, plus 4 B per component.<br>**Rejected:**<br>• `ComponentId` order: minted on first use, so it depends on when Main first touched a type (H-04).<br>• `TypeId` alone: differs between OS builds of one source, so the OS leg would go red on hook order.<br>• A per-type key static: 4 KB of `.bss` and one hash per mint in every game.<br>**Note:** `type_name` is documented as diagnostic and not unique (§11). That is acceptable because the contract is per binary and ties go to `TypeId`. | Two hooked components of equal `type_name` in a shipped archetype redden the OS leg → those types take explicit stable names. |
| U-27 | **UG-22's machine arm runs one artifact on two machines.** CI builds the release test binary once and uploads it. The CI Windows runner and the gate host run that same file. | **Same binary by construction:** both machines print the file's sha256.<br>**Why not rebuild on both hosts:** that needs byte-identical images, and nothing here remaps source paths. `[W].cargo/config.toml:106-113` sets only `target-cpu`, and registry crates embed `CARGO_HOME`-absolute panic paths (cargo#5505, RFC 3127; critic pass 6, W7).<br>**Cost:** one artifact upload per CI run. | A product need for users to rebuild bit-identical binaries → `trim-paths` (RFC 3127) plus a red-first check that two-host builds hash equal. |
| U-28 | **Nothing outside Fixed moves a keyed entity** (rule B's move clause; H-16 generalised). Main systems, host code and Main-triggered observers may not spawn or despawn a keyed entity, or insert or remove a table (signature) component on one. Non-fragmenting components (dense, bitset) that no Fixed system reads are exempt. | **Why it suffices:** every keyed entity carries `ReplayKey`, so a keyed entity's table holds only keyed entities. Every Fixed query iterates rows, and gameplay systems, not only physics, can depend on row order.<br>**Cost in games:** 0. Verify sessions pay one move digest per tick boundary.<br>**Rejected:**<br>• Key-sorted iteration of Fixed queries: O(n log n) per query per tick.<br>• U4–U7's group-slot order alone: covers physics only. | A game needs Main-side table inserts on sim entities (owner Q-12) → a replay-session-only per-archetype key sort, priced by MQ-23. |

## 4. Phase map

```
A stabilise (bugs first, lane merges; AH moves the gate host to msvc) ──> A8 trunk cut ──> B gates
   ──> C memory crate: D-M0, then C1 (after AP7)
   ──> D kernel contract: lane MEM | lane STORE | lane ENG | lane REPLAY (RP-0..RP-3; RP-3 lands UG-22) | R2 sweeps
   ──> E subsystems: physics (U4..S0) | engine (IN/HO/RE/AS/UI/SC/LG)
   ──> F tail: F1 R6 rows | F2 end-state gates (G3/G4/G5) | F3 KE17 if MQ-05 says build | F4 the refactor campaign (Q-4)
Docs (parallel with A): DOC-1 → AP7 (holds C1) ; DOC-2 → EP3 (holds D-S3(iii), AS2, engine-sourced D-E rungs)
M modding: Stage 0 answered (Q-1, Q-2); Stage 1 = D-S1(ii) inside D; Stage 3 only after D exit
No file is split before F4 (U-12)
```

## 5. Patches the source documents owe once this plan is approved

| Document | Patch |
|---|---|
| `docs/memory/ALLOCATOR-DESIGN-SPACE.md`: closed at rev 2.4 by orchestrator ruling. AP6 ran with 0 Critical; its W1–W5 are open (`:5-10`, `:3873-3881`). | **Step DOC-1 (02 §2) writes rev 2.5. AP7 reviews only the rev-2.5 delta and the AP6 dispositions; AP6 already reviewed rev 2.4.**<br>**Revival forms.** U-1, U-7, U-8 and U-9 move Heap, TableSet, DropColumn and HeapDyn to revival form. The ByteColumn, ZeroInit, ChunkArena/SlotChunks, injector, 1f and G-ladder content stays. P39 and P40 stay as AP6 reviewed them.<br>**§7** is re-pointed to file 05: K-MOD-3 and K-MOD-10 are removed; G6 = UG-15. The duplicate "(f)" row (AP6 W1) is relabelled and pointed to UG-15 leg (7), which sees kernel survivors that the two-arm row cannot. Leg (6) keeps only the feature-unification role.<br>**§2.0.** The thread-context placement moves from `boyko_memory` to `boyko_threadpool` (U-19). This keeps G5's `#![no_std]` for `boyko_memory` (`:337` at `b716a5dc`) intact.<br>**P29.** `HeapRef::alloc_cold` is struck from P29's pinned set (`:2619-2623` at `b716a5dc`), because U-1 never builds it (critic pass 5, W1 (a)); four symbols remain.<br>**AP6 dispositions** (02 §2, Document steps):<br>• W2 → D-S1(i)'s isolated tests.<br>• W3 → void under U-1; P41's gate is not built, and the remark is carried with the revival form.<br>• W4 → 05 §5 (row AP6-W4).<br>• W5 → B1.<br>• O1's stale passages are fixed.<br>• O2 → void under U-2.<br>• Question 4 → D-S1(i)'s cut.<br>• Question 5 → void under U-10. |
| `docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md` (closed rev 5) | **Step DOC-2, reviewed by EP3.** Erratum E2: `Release` is an associated type with markers `Immediate` / `Chained` / `Stamped` (U-3, 01 KC-12); U1 deletes the scratch band (U-2); NB2 as U-17; N5 span freeing inside release points; **X-14 corrected** (`:1267`): "Command order, hook order and dense slots handed out in one window depend on completion-pop order (fixed by KC-36). Entity ids do not depend on the window: `Commands::spawn` claims them on the worker during the phase (`commands.rs:169-173`), so they depend on claim interleaving whatever the apply order (U-20)." |
| `docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md` (rev 3, pass 3 never ran) | **Step DOC-2, reviewed by EP3** (scope: the rev-3 patch, rev 4 and physics erratum E2). Rev 4: K6′ against physics rev 5 (U-3); registry-free EK1 (U-2); EK15b's redirect enqueues the `Pinned` removal instead of performing it (02 §4.4); prerequisites remapped to plan rung ids (02 §6). **EP3 must close before D-S3(iii), AS2 and every engine-sourced D-E rung.** D-E0 (physics P-§14), D-E18 and D-E19 (ledger KF-10, KF-09) are not engine-sourced and do not wait for EP3 (02 §2) |
| `docs/memory/RUNTIME-DATA-LEDGER.md` | rev 5 (B1): re-point to the trunk; KF-33/34 status per U-4/U-5; PC1 closed per U-1; ScopeShared per U-6; PC5 closed. **KF-45 (`:1624-1640`):** `:1628` currently reads "One VmReservation-backed column of 64-B records … Reached by slot through the platform thread-control-block pointer (an id -> slot table)". Rev 5 restates it as "one fixed `.bss` static table of 8192 64-B records and a 128-word busy bitmap, reached through a per-thread OS word that holds the record's index + 1 (U-19; 01 §6)". The precedent is KF-43's static-table form (`:1593-1605`); U-19 (d) is the owner's overturn. Per-row routes follow 01 §6 item 10: 9 record, 1 unchanged `thread_local!` (`LANE`, out of scope with its reason: diag may depend on nothing, and the cell is const-initialised and `Drop`-free), 2 OS thread id. The row's home moves from the memory library (`:1718`) to `boyko_threadpool`. The four remaining key and guard `thread_local!` cells are rowed as out of scope, each with its reason. KF-02 (`:898`): the `boyko_rhi_vulkan/src/memory.rs:728` row moves to D-E15, and the `boyko_utils/src/sparse_map/sparse_map.rs:10` row to D-R2d (02 §2) |
| `[J]docs/diagnostics/substrate/05-LADDER-GATES.md` | **No patch (rev 6).** Erratum D0-1 is withdrawn with the windows word arm (U-19): D-M6 no longer touches `boyko_diag`, so D0 and DG12 stand as written. The erratum's text is kept with the revival form (01 §8.5). |
| `docs/modding/MODDING-DESIGN-SPACE.md` (closed) | erratum for P6-6 citations; dispositions of the carried remarks are in 05 §5; **Stage 0 answered (Q-1, Q-2):** options B, D, E and F are out, the exact-build family A′, C, A remains, and the exact-build contract is 05 §3.3 |
| `[J]docs/MEASUREMENT-QUEUE.md` | add the MQ entries of 03 §5, including MQ-13's host arms (msvc deciding; gnu and Linux record-only), MQ-19 and MQ-21..MQ-23 (MQ-23 with its spawn-churn arm) |
| `docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md` | DOC-2, erratum E2, gains two items. (1) The row-identity map matches the full `Entity`, so the generation is included (A1b, H-03). (2) Until U7, the solve order follows archetype-row order (H-16). Replays do not depend on U7 for this. Rule B's move clause (U-28) forbids Main-side moves of keyed entities, and the replay's move digest detects a breach, so UG-22 has no arm deferred to U7. |
| `docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md` | DOC-2 scope gains EK7's third policy, `#[event(swap = "every_tick")]` (KC-37 (d), D-E8), and D-E20's writer lanes (EK7's lanes are keyed by writer, not worker). It also gains D-E23's move of `FixedTime`'s frame-level getters into `Time` (U-25): the engine design's interpolation readers read `Time::fixed_overstep_fraction()`. |
| `[W].cargo/config.toml:15-28` (the "Miri stays on the gnu nightly" comment) | Superseded at AH: the msvc nightly's miri (2026-09-09) is now newer than the gnu nightly's (2026-08-20), so the reason given there no longer holds. AH re-spells the recipes and re-runs the receipts. |
| `D:/wt/_graph/refactor-census.md` ([G]) | Kept as is until F4, which re-derives it against the post-F3 trunk (02 §5). |

## 6. Risks

| # | Risk | Evidence | Mitigation |
|---|---|---|---|
| RK-1 | **The allocator design is closed at rev 2.4 by orchestrator ruling.** AP6 (critique pass 6) ran with 0 Critical remarks and left W1–W5 open. Rev 2.5, which U-1/U-7/U-8/U-9 require, is unwritten and unreviewed. U-1 reverses the design's Heap class. | `ALLOCATOR-DESIGN-SPACE.md:5-10`, `:3873-3881`, `:3928-4106` (at `b716a5dc`; *writer, §9 V-67: the patch cited `:3977-4101`, which starts inside W2 and ends at O2's heading; the review's Remarks section is `:3928-4106`*) | DOC-1 writes rev 2.5 with the AP6 dispositions, and AP7 reviews the rev-2.5 delta. **C1 waits for AP7**, because rev 2.5 rewrites §2.0, which C1 builds. D-S1(i) and D-S2 no longer wait: AP6 raised no remark against P39/P40's or P43's mechanisms, and its W2 (P39/P40's gate rows) is answered by D-S1(i)'s isolated tests (02 §2). *Writer check (§9 V-68): AP6 raised no Critical or Important remark against those mechanisms (`:4114-4118`). Its optional O2 (`:4101-4105`) does touch P43.1's rule and P39.4's `TraversalScratch` client; this plan voids O2 under U-2 (02 §2, Document steps).* UG-15 leg (6) gates once a modding crate exists; AP6 W1 is answered by leg (7). Every AP6 remark has a rung (02 §2, Document steps). |
| RK-2 | The engine design's rev 3 was never re-critiqued, and it was filed against physics rev 3 | `[M]CHECKPOINT-2026-09-11.md:55`; physics rev 4/5 changed K6 (`PHYSICS-ECS-UNIFICATION-DESIGN.md:2449-2463, 3330-3343`) | DOC-2 writes engine rev 4 and physics erratum E2, and EP3 (engine critique pass 3) reviews them. **EP3 must close before D-S3(iii), AS2 and every engine-sourced D-E rung.** D-E0, D-E18 and D-E19 have non-engine sources and start when their prerequisites land (02 §2) |
| RK-3 | Page-cache bit flips on a workstation without ECC | `CHECKPOINT-2026-09-11.md:236-238` | Owner memory test (Q-6); every red is re-run once before triage; receipts record the target dir |
| RK-4 | A number measured on a tree without the fix | `:198-200` | Every MQ receipt carries `git merge-base --is-ancestor <fix> <tree>` |
| RK-5 | **Kernel rungs edit unsplit large files, so more rungs queue on one lock** (Q-4 moved every split to F4) | `[J]` @ `d552be05`: `schedule.rs` 2536 lines, `ecs_master.rs` 1917, `component_pool.rs` 4106 ([G] §1.1; 02 §5). `schedule.rs` alone has 8 rungs in its 02 §4.3 row | Per-rung, per-file locks and first-cut-wins (02 §4.2). Each cut records the queue on its files. More than three rungs queued on one file raises an owner question (U-12; Q-4 is the ruling of record). |
| RK-6 | **A move changes CGU partition and cross-crate inlining, and so moves hot-loop codegen.** Before F4 this applies to C1, a move of `vm.rs`/`vm_column.rs` into another crate; in F4 it applies to every wave | cgu=1 under fat LTO costs `query_ref_iter` 17.4 % (modding `:2101-2103`); P6-1 (`MODDING-DESIGN-SPACE.md:2425-2438`) | C1 runs UG-15 attributed with its rename list (02 §2). F4's commits run UG-15 strict with leg (2-RF). A moved pin holds the commit for an MQ-12 entry. |
| RK-7 | Owner-dirty files overlap trunk work (44 paths, 14 `.rs`, as [G] recorded them at `d1e77f4f`) | [G]:331-348. **Writer check, 2026-09-17:** `git status --short` in `[M]` now lists 39 entries (59 paths with `-uall`), 16 of them `.rs`: the 14 modified sources plus the untracked `crates/boyko_app/examples/{_hud_probe,playground}.rs` | **Closed (Q-5, 2026-09-17).** The owner committed the paths (§7). Only `.claude/settings.local.json` and two model archives stay out of git; `b716a5dc` git-ignores the archives. |
| RK-8 | Inherited reds at the base | `check_hotpath_exceptions.py` red at `[J]crates/boyko_ecs/src/ecs/core/entity/entity_reservoir.rs:405` ([G]:835); `production_reachability_census` red at `[J]crates/boyko_physics/src/row_identity.rs:278` (source: session record of 2026-09-17, which records it red on the pushed `b74f7ee8`; [G]:835 records only the hot-path red) | A1 and A4a (A4b fixes the flaky `QueryTypeId` race) |
| RK-9 | A long-lived mega-branch gets reverted | Bevy [#20934](https://github.com/bevyengine/bevy/pull/20934) (145 commits, merged 2026-02-10) drew an approved revert, [#22915](https://github.com/bevyengine/bevy/pull/22915), citing compile time +8–12 % and binary size +5–7 %. The revert was closed **unmerged** on 2026-02-18, and no revert commit reached Bevy `main` by 2026-03-15; the regression was worked off in follow-ups instead (e.g. [#22919](https://github.com/bevyengine/bevy/pull/22919), which hoisted generic code out of `register_component`). GitHub API, read 2026-09-17 (§9 V-21) | No rung is XL: D-S3 is three rungs, D-S3(i)–(iii), each green on its own (02 §1 rule 5) |
| RK-10 | Apply-window changes break the EM2′-K rule | `[J]docs/MEASUREMENT-QUEUE.md:93-98` | Any rung touching `apply_window_drain` (`[J]crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:783`) or `may_defer` (`:167`) carries the EM2′-K test |
| RK-11 | Disk space | **D: about 92 GB free** (owner, 2026-09-17; the rev-5 figure of ~36 GB is stale). C: not re-measured; the session record of agent build caches found on C: stands | At most three pool worktrees plus the trunk. `D:/wt/refactor` gets no target dir until F4. Target dirs live only under `D:/wt/_targets`. AH's gnu comparison dir is deleted after its pin commit. AH moves the census's nested fat-LTO target dirs off C: (RK-18). UG-22's second-machine and OS legs use CI disk. The `tls-*` probes are pruned after review (04 §5). |
| RK-12 | Id budget after the UI and reflect merges plus the new group columns | 512 slots (`[J]…/component_registry/mod.rs:63`) | AL:M-A13 / MD:M-K3 census (structural) at B2, after D-S3(ii) and after D-S3(iii) |
| RK-13 | Parallel rungs edit shared kernel files where the order of edits changes behaviour | `delete_entity_core` (`[J]…/ecs_master/entity_api.rs:980`) is edited by D-S3(ii) (`unbind` at the two `deallocate_entity` sites, `:1091` and `:1102` at `d552be05`; physics cites `:1088`, `:1099` from `d11962a9`) and by D-E2 (the despawn redirect). `GroupRef` membership is only a `debug_assert!` (`PHYSICS-ECS-UNIFICATION-DESIGN.md:2729`), so a wrong order is silent in release | Per-rung, per-file locks; fixed orders; the fixed step order inside `delete_entity_core` (02 §4); D-E2's combined red-first test |
| RK-14 | **Hazard 1.** On windows-gnu, the direct reads of KC-04 and `LANE` assume that a `TlsAlloc` index below 64 addresses `TEB.TlsSlots[index]`. winternl places `TlsSlots` after 12·8 + 8 + 399·8 + 1952 bytes (`+0x1480` on x64), but Microsoft calls the index "an opaque value" and says to call `TlsGetValue` rather than read the TEB (§11). **Hazard 2.** On windows-gnu, std's OS-key backend reads every `thread_local!` through its own `TlsGetValue` index (`RUSTC-198-WINDOWS-GNU-TLS.md:34-41, 87-92`). By the first pool build, the 64 fast indices may already be taken, and the canary then fails | Microsoft `TlsAlloc` and `TEB` pages; the TLS document | **Scope (rev 6): applies only to the revival form D-M6w (01 §8), which is not built; for the plan as built this risk is closed.** A per-process canary (01 §6 item 3) enables direct access only when `index < 64` and the mapping holds in both directions. Otherwise, the documented `TlsGetValue` call arm serves every read, and `TLS_OUT_OF_INDEXES` falls back to the portable word. `prepare()` runs at the first pool build, before workers can race for indices (01 §6 item 4). UG-20 reports the arm in use and both indices; MQ-13 prices the call arm |
| RK-15 | Thread-table capacity (critic pass 4, W1): a fixed table can be exhausted, and at rev 4 a refused worker panicked | Every worker writes pool thread state first (`[J]crates/boyko_threadpool/src/worker.rs:46, 54, 59, 69`). `App::new()` builds `available_parallelism()` workers (`[J]crates/boyko_ecs/src/ecs/core/app/app.rs:201-203`). `boyko_render`'s lib tests build a default `App` in 10 tests, through 8 `App::new()` call sites (`[J]crates/boyko_render/src/light.rs:2214-2227` is a helper three tests share); 10 × 32 + 10 = 330 threads on a 32-logical host (critic pass 4 counted 8 tests and 264 threads; §9 V-54) | 8192 slots. Workers are batch-claimed at build and clamped, keeping a 2048-slot foreign reserve, so no worker is ever refused. A build with no free slot panics on the building thread. Only a foreign thread can be refused, and only when the table is full; it reads `DETACHED` and panics with a code on its first write. The sizing basis is in 01 §6 item 2; the test at the bound is D-M6 phase (f); UG-20 reports clamps, dips, refusals and the peak |
| RK-16 | A replay hazard class that no UG-22 arm forces | The inventory is a code read (15 items; the architect found H-16..H-19 in the same pass) | UG-22 forces each known class: W, ids, pacing, Main churn (including the move clause, r11), UCRT FMA3, profile, OS, and two machines running one file (U-27). The Main-write check and the boundary census catch crossings. Each new class becomes a register row and an arm. |
| RK-17 | A gnu-blessed pin read on msvc before AH reds for a host reason (a gate red for the wrong reason) | The owner switched hosts on 2026-09-17; pins were blessed under gnu | Until AH's pin commit, gnu-pinned tests run under `stable-x86_64-pc-windows-gnu`, and the receipt names the toolchain (`[M].claude/agents/tester.md:212-219`). |
| RK-18 | The profile-axis census builds six fat-LTO target trees under `%TEMP%` (drive C:) | `[C]crates/profile_fixture/tests/profile_axis_census.rs:143-146`, `:226` | AH's gate recipe sets `TMP`/`TEMP` to `D:/wt/_targets/tmp` for gate runs. |

## 7. Owner questions (values and scope only)

| # | Question | Default if unanswered |
|---|---|---|
| Q-1 | Modding Stage 0: (i) trust model — curated native mods or untrusted/sandboxed; (ii) must mods survive engine patch releases; (iii) who builds a mod; (iv) scope — may mods add component types, or only data prototypes | **answered 2026-09-17** (below) |
| Q-2 | Is mod unload or hot reload a product requirement? The plan rules load-only (U-10). | **answered 2026-09-17** (below) |
| Q-3 | Quiet-machine windows. MQ-01 (per-stage pyramid) and MQ-02 (SP-1) decide the physics rung order. | physics runs the U track first; P1/P2 follow |
| Q-4 | Refactor wave K (~18 kernel files, ~40k lines) lands **before** kernel code rungs (U-12), with one stated exception: D-M0, the packing rung, lands before C1 and RF-K1 so its line-cited steps do not rot (02 §2). Accept the delay, or interleave kernel rungs with file locks and accept rebase churn? | **answered 2026-09-17** (below) |
| Q-5 | Commit or drop the uncommitted paths in the main checkout (44 paths / 14 `.rs` per [G]; 59 paths / 16 `.rs` on 2026-09-17, RK-7). They block the trunk cut, 4 census files and K4. | **answered 2026-09-17** (below) |
| Q-6 | Memory test of the workstation (two bit flips this week) | gate results stay provisional |
| Q-7 | Timing of the msvc switch. It moves KF-45's priority and the Miri toolchain. | **answered 2026-09-17** (below) |
| Q-8 | `Scope::spawn_batch` surface (open scope question from 2026-09-09). D-M2 is compatible with either answer. | unchanged |
| Q-9 | Must entity ids be identical across runs and across worker counts, e.g. for lockstep networking or id-keyed replays? KC-36 makes apply order, hook order, table rows and group slots deterministic, but not ids (U-20). | **answered 2026-09-17** (below) |
| Q-10 | **Must a replay recorded by one OS build play on the other OS build of the same game** (Windows msvc ↔ Linux x86_64 — two binaries)? | **No: the replay contract is per binary** (the header's image hash, 01 §2.1). UG-22 still gates the engine's own scenes on the Linux CI leg against the same golden, because that costs one CI job and catches build-dependent float hazards (H-09, H-10) early. **What "yes" would cost:** the header keys on the source fingerprint instead of the image hash; the Linux leg becomes a product guarantee; and every future ISA or `cfg` difference in Fixed-schedule code needs both OS legs green. **If the Linux leg goes red** for a reason whose fix costs performance, the owner decides whether to demote it to record-only. |
| Q-11 | **May a replay start from a save file** rather than from app startup? | **No, in v1.** Saves do not carry solver state (physics warm-start, sleep latch, axis cache), and reload order can differ from the recording (H-14, `[J]crates/boyko_serialize/src/save.rs:170`, `…/load.rs:579`). Saves become replay-safe only after physics state lives in ECS storage (Phase E, U4–U7) and serialize covers groups and resources — a campaign this plan does not schedule. |
| Q-12 | **May a game that records replays attach Main-side table components to simulation entities** (for example a render handle inserted by a Main system onto a body spawned in Fixed)? U-28 forbids it inside a replay session, because such an insert reorders the rows every Fixed query iterates. | **No** (U-28). A game puts those components in the Fixed spawn bundle, or uses non-fragmenting (dense or bitset) storage for Main-only data. Verify sessions detect a breach (the move digest). **What "yes" would cost:** a replay-session-only sort of every Fixed query's rows by `ReplayKey`, O(n log n) per query per tick, priced by MQ-23. |

### Owner answers, 2026-09-17 (binding; the rulings and phases they touch are revised in the next plan revision)

| # | Answer | What changes |
|---|---|---|
| Q-5 | **Commit.** Done the same day: `5e86fe2d`, `f37650a6`, `f20bdafe`, `69cf3f79`, then `5aef7b39` and `b0bff31f` on `feat/multi-paradigm-render` (HEAD now `b716a5dc`). Not committed: `.claude/settings.local.json` (machine-local by rule) and the two downloaded 3D models under `assets/` (`assets/models/`, two `*_glb*.zip`): one carries a personal-use-only licence (RigModels), so it cannot go into a public repository. | The trunk cut is no longer blocked by uncommitted owner paths. The KE16 merge branch `merge/ke16-into-render` is no longer a fast-forward of `feat/multi-paradigm-render`; it needs a real merge with the overlapping files. |
| Q-4 | **The refactoring campaign runs after everything else,** so the plan is not reworked by it. | U-12 is overturned by owner scope: every refactor wave (RF-0 pilot, RF-K, RF-R, RF-V, RF-P, RF-A, RF-U) moves to the end of Phase F. Its census, design (closed after six critique passes) and partial tooling are kept for then (`D:/wt/_graph/refactor-census.md`, `D:/wt/refactor`). |
| Q-9 | **A replay must load and play on any machine; entity ids need not match.** | U-20 stands (ids are not made deterministic). New requirement: simulation replay determinism — the same binary reproduces the same simulation on any machine and any worker count, given the same recorded inputs; replay files refer to entities by stable replay keys, never by `Entity` ids. KC-36's ordering guarantees are the base; the next revision adds the gate (state hash keyed by stable keys, compared across W = 1/2/8 and across two machines). |
| Q-1 | **Native mods, no sandbox; they need not survive engine patch releases; the mod author builds the mod; mods may add their own types.** | Option E (WebAssembly) and option B (stable C ABI) are out. The exact-build family (A', C, A) remains; "the author builds" matches an exact-build contract published per engine release. Mod-defined component types are in scope (05 MS items for dynamic component registration stay). |
| Q-2 | **No hot reload** if it adds complexity. | U-10 stands: load-only. |
| Q-7 | **Answered by action (2026-09-17).** The owner moved the machine and every agent to the MSVC host: rustup default `stable-x86_64-pc-windows-msvc` 1.98.1, default host msvc. **Facts supplied:** `nightly-x86_64-pc-windows-msvc` carries miri 2026-09-09 (gnu nightly: 2026-08-20). On 2026-09-10 the msvc legs on `e6115223` gave 5582 passed / 2 failed (both profile-axis census cells, fixed on `fix/census-post-lto-object`); Miri agreed with gnu on 3 targets; goldens 32/32 byte-identical. `chore/msvc-host` (`a36ceaa4`, pushed) moves the tree's recipes and is merged into no lane. On trees carrying the AVX2 baseline, `--config 'build.rustflags=…'` passes no `--cfg loom`. Pins in the trees were blessed under windows-gnu. The gnu toolchain stays installed for comparison. | U-22 (new). U-18 and U-19 revised (the word arm is not built). MQ-13's legs are re-cut. RK-14 is scoped to D-M6w. Step AH is added before A2 (02 §2; 04 §2). UG-08, UG-09 and UG-15's symbol legs are re-specified (03). |

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
| Replay determinism | KC-37 contract; boundary rule B with the `FixedTime` split (U-25) and the move clause (U-28); register H-01..H-21; rungs D-E20..D-E23 and RP-0..RP-3; gate UG-22 with ten red and one green control ✔ (01 §2.1; 02; 03 §7) |
| Owner answers | Q-1, Q-2, Q-4, Q-5, Q-7 and Q-9 applied in files 00–05 (file 00 in rev 6.1) ✔; Q-10, Q-11 and Q-12 have stated defaults |
| Items not established | the contents of lane `fix/ke13-ke14` (1 commit, 25 `.rs`; not read). The ancestry checks of 04 §2 and the containment of `fix/inherited-red-gates` and `fix/miri-protector-arming` were established by the writer (§9, 04 §1). The `[Jw]` citations of rev 6 are working-copy reads that the writer verifies against `d552be05`. |

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

**Rev 6 re-verification (writer, 2026-09-17).** The rev-6 patch reached the writer as the second of
the architect's two output messages: its first 21 KB (P6-00-1 to P6-00-13, which edit this file's
title, index, status, provenance, §1–§4 and add U-21..U-24) was not in the writer's brief and is
**not applied** by this pass (reported to the orchestrator). Every citation the applied part adds was
read in the tree it names:
- `[J]` / `[Jw]` at `d552be05` with `git -C D:/wt/joltab show d552be05:<path>` and `git grep`
  (the working copy was not read): `app/app.rs:60-75, 198-205, 705-780`;
  `system/params/event_writer.rs:45-170, 280-295`; `events/event_buffer.rs:180-240, 336-362`;
  `events/event_dispatcher.rs:270-285, 628-635`; `events/event_config.rs:18-30`;
  `system/params/commands.rs:165-175`; `iters/query_state.rs:230-258, 580-590`;
  `iters/query/par_iter.rs:112-128`; `archetype/archetype_master.rs:30-55, 150-175, 477, 550-575,
  650-700, 950-990`; `time/fixed_loop.rs:45-92`; `schedule/schedule.rs:795-830`;
  `component_registry/mod.rs:916-923`; `commands/migration_helpers.rs:1018-1110`;
  `bundle/bundle_column_cache.rs:318-325`; `ecs_master/entity_api.rs:1049-1052`;
  `hooks/scope.rs:64-80`; `boyko_macros/src/bundle.rs:340-348`; `boyko_scene/src/{identity.rs:102-126,
  visibility_sync.rs:78-108}`; `boyko_physics/src/{components.rs:198-212, resources.rs:76-118,
  1090-1123, systems.rs:26-37, 245-269, 743-754, row_identity.rs:14-28, 515-522, 576-583}`;
  `boyko_app/src/runner.rs:1226-1240`; `boyko_input/src/action/process.rs:49-79`;
  `boyko_serialize/src/{save.rs:165-174, load.rs:574-583}`; `boyko_math/src/{lib.rs:1-22,
  mat.rs:300-310}`; `boyko_diag/src/lane.rs:34-40, 134-150`; `crates/boyko_ecs/Cargo.toml`;
  `rust-toolchain.toml:30-39`; `.github/workflows/ci.yml:305-370`; `.cargo/config.toml:80-91`;
  `docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:85-96`;
  `docs/diagnostics/substrate/05-LADDER-GATES.md:127`; `wc -l` of `schedule.rs` (2536),
  `ecs_master.rs` (1917), `component_pool.rs` (4106).
- `[C]` at `27ac8904` (the worktree is clean; `fix/census-post-lto-object` and
  `origin/fix/census-post-lto-object` are both `27ac8904` in `packed-refs`; the worktree reflog's
  line 3 is `27ac8904`; one commit on `02325b01`, the merge base with `d552be05`):
  `crates/profile_fixture/tests/profile_axis_census.rs:40-330, 490-515, 610-648`.
- `[W]` at `a36ceaa4` (clean; `.git/refs/remotes/origin/chore/msvc-host` is `a36ceaa4`; reflog line
  5; three commits on `e6115223`, the merge base with `d552be05`): `.cargo/config.toml:1-113`.
- `[M]` at `b716a5dc` (`.git/refs/heads/feat/multi-paradigm-render`): `.claude/agents/tester.md:205-232`;
  `RUNTIME-DATA-LEDGER.md:1552, 1624-1632`; `MODDING-DESIGN-SPACE.md:1971-1987`;
  `ALLOCATOR-DESIGN-SPACE.md:1-14, 1170-1182, 2607-2626, 3865-3900` and a sample of rev-5 citations
  (V-61); `git log 8a78ef6d..b716a5dc`; `git rev-list --count d552be05..b716a5dc` = 22 and
  `git diff --name-only d552be05...b716a5dc -- '*.rs'` = 16 (merge base `97c504c8`).
- Machine: `llvm-symbolizer.exe` exists at the MSVC Build Tools path of 00 §10 O7 (12,831,776 B);
  `stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin` holds `llvm-nm`,
  `llvm-objdump`, `llvm-readobj`, `llvm-size` and others, and no symbolizer; `rustup default` is
  `stable-x86_64-pc-windows-msvc`; `rustc --version` reads 1.98.1 (stable msvc), nightly 2026-09-09
  (msvc) and 2026-08-20 (gnu); loom 0.7.2 `rt/mod.rs:62` (`MAX_THREADS = 5`), `rt/scheduler.rs:99`,
  `rt/path.rs:118`; the stable-gnu std `sys/thread_local/key/windows.rs` exists (lines 150-191 were
  not re-read beyond a spot check).
- Tools: graphify once (a stale graph, off-target hits), then read-only git, `sed`/`grep` and
  Python file reads. No cargo command, no git write, no timing, no web fetch (00 §11's rev-6
  entries are the architect's and the researcher's).

| # | Where | Finding | Action |
|---|---|---|---|
| V-59 | 01 §2.1 register, H-11 | `fixed_advance` is `[J]…/time/fixed_loop.rs:51-89` (signature `:51`, closing brace `:89`), not `:50-87`; the patch's own verification list gives `:51-89` | corrected in place |
| V-60 | 03 UG-09 | The loom CI job is `[J].github/workflows/ci.yml:310-370`, and its count and `0 filtered out` pins run to `:369` (`:362-369`), so `:310-364` and `:322-364` cut the job short | corrected to `:310-370` and `:322-369` |
| V-61 | 00 §5 allocator row; 03 UG-08 | `ALLOCATOR-DESIGN-SPACE.md` was committed in `ac86fc38` as 4,158 lines, not the 3,864-line working copy every allocator citation of this plan was read in. A 7-line status block now sits at `:5-11`, so rev-5 text cited at `:N` is at `:N+7` at `b716a5dc` (sampled: G5 `:330` → `:337`, `:1124` → `:1131`, `:1129` → `:1136`, P11 `:1174` → `:1181`, `:3657` → `:3664`, `:3694` → `:3701`, `:3825` → `:3832`, `:3845` → `:3852`, `:3858` → `:3865`; P29's five pinned rows `:2611-2616` → `:2619-2623`, "A missing symbol is RED" `:2618` → `:2625`). **The status block (`:5`) also says the design is "closed at rev 2.4 by orchestrator ruling", and critique pass 6's log is appended at `:3873` onward (0 Critical, W1–W5 OPEN): AP6 has run**, which RK-1, 02's document steps and U-1's overturn text still describe as pending | the two citations rev 6 adds or re-emits are corrected (00 §5 allocator row: `:2619-2623`; 03 UG-08: `:1181`); every other allocator citation in 00–05 and the AP6 status are left for the architect (outside the brief) |
| V-62 | 01 §2.1 rule B, "Archetype ids" | `clear()` recycles every `ArchetypeId` (`[J]…/archetype_master.rs:968-971` resets `next_archetype_id` to 1; the cited `:37-52` doc says so). `remove_archetype` (`:664-690`) frees an id but does not re-mint it: `create_archetype` mints monotonically (`:158-159`). A freed id comes back only through `add_existing_archetype` (`:477`), which advances the counter past the id it is given (`:569-572`). Besides `query_state.rs`'s tests (`:585` onward), `remove_archetype` has one more test caller, `iters/query/state.rs:1272` (inside `#[cfg(test)]`); no production caller | annotated in place; the refusal rule stands |
| V-63 | 01 §2.1 register, H-08 | The non-test count holds for method-call syntax: `boyko_scene/src/camera.rs:483, 727, 728, 926, 927` (5), `boyko_math/src/mat.rs:307` (1), `boyko_physics/src/resources.rs:1094, 1121` (2 `cbrt`). The other physics hits (`math.rs:84, 120, 121, 163`, `narrowphase/box_box.rs:851, 919`, `narrowphase/sphere_box.rs:164`, `resources_tests.rs`) are all inside `#[cfg(test)]` modules | none |
| V-64 | 04 §1, main row | `[M]` is 22 commits / 16 `.rs` files ahead of `d552be05` (merge base `97c504c8`). Besides Q-5's commits, `ac86fc38` (the plan and allocator rev 2, 14:16) also landed after `8a78ef6d`; `5e86fe2d` carries 7 `.rs` files, `f37650a6` 9 and `b716a5dc` 2. `assets/models/` is ignored (`.gitignore:32`) | count entered in place |
| V-65 | 02 §2 rule 10 | The patch places rule 10 directly under rule 6. CommonMark renumbers an ordered list from its first item, so rule 10 there would render as "7" and shift rules 7–9 | placed after rule 9 instead |
| V-66 | moved text (01 §8.5) | Two moved table rows (00 §5's erratum D0-1 and 01 §6's `CTX_IDX` / `LANE_IDX` row) would not render as tables on their own | each is given its source table's header row; the rows themselves are unchanged |

Every other citation the applied part adds held.

**Rev 6.1 re-verification (writer).** Check every citation rev 6.1 adds:
- `[Jw]` citations at `d552be05`, with `git show`;
- `[M]` at `b716a5dc`;
- §11's rev-6.1 pages.

Apply the V-61 rule to every allocator citation carried from rev ≤ 5.1. Record the findings as V-67 onward.

*Done by the writer, 2026-09-17.* What was read:
- **`[J]`/`[Jw]` at `d552be05`** with `git -C D:/wt/joltab show d552be05:<path>` and `git grep` (the working copy was not read; a workflow edits it):
  - `system/params/event_writer.rs:45-170`; `events/event_dispatcher.rs:255-300`;
  - `time/{fixed_loop.rs:40-95, fixed_time.rs:95-210, time.rs}` (public surface);
  - `ecs_master/entity_api.rs:695-830, 1048-1053`; `component/component.rs:45-100`;
  - `component_registry/mod.rs:100-182, 915-1000`; `component_registry/tags.rs:125-207`; `ecs_master/tag_api.rs:40-60`;
  - `archetype/archetype_master.rs:30-56, 585-600, 664-690, 960-989`; `ecs_master/ecs_master.rs:150-160, 570-578`;
  - `iters/query/query.rs:675-690`; `schedule/schedule.rs:114-200` and its public fns; `schedule/schedule_builder.rs:1025-1080`;
  - `app/app.rs:585-640`; `system/access.rs:30-225`; `system/system_meta.rs:245-262`; `system/params/commands.rs:166-175, 340-356`;
  - `hooks/builder.rs:144-152`;
  - `boyko_input/src/action/{process.rs:1-130, state.rs:1-30, 170-200}`; `boyko_app/src/runner.rs:632-642, 2276-2286`; `boyko_demo/src/app.rs:508-517`; `boyko_demo/Cargo.toml`;
  - `boyko_ecs/tests/{app_fixed_timestep.rs, miri_fixed_loop.rs, event_send_from_worker.rs}`; `boyko_render/tests/particle_containment.rs:176-185`; `rust-toolchain.toml:36-39`;
  - `git grep` for the three `FixedTime` getters, `add_existing_archetype`, `remove_archetype(`, `structural_generation`, `dynamic_slot_occupied_panic`, and path remapping (`remap`, `trim-paths`).
- **`[W]` at `a36ceaa4`:** `.cargo/config.toml:95-113`, and `git grep` for path remapping.
- **`[M]` at `b716a5dc`:** `ALLOCATOR-DESIGN-SPACE.md` (status block, the P29 table, the pass-6 log and review, and every carried citation at N and N+7); `RUNTIME-DATA-LEDGER.md:1444-1470, 1780-1785`; `ledger/pool-utils-log.md:1-30`; `git log` of the allocator file (one commit, `ac86fc38`; the working copy is clean).
- **Web** (2026-09-17): the rev-6.1 pages of §11, plus Rust `_mm_setcsr` and Bruce Dawson's "Floating-Point Determinism".
- **Tools:** read-only git, `sed`/`awk`/`grep`, Python file reads and edits, WebFetch/WebSearch. No graphify, no cargo, no git write, no timing.

| # | Where | Finding | Action |
|---|---|---|---|
| V-67 | 00 RK-1 | The patch cited AP6's review as `ALLOCATOR-DESIGN-SPACE.md:3977-4101`. That range starts inside W2's facts (W2 is `:3969`) and ends on O2's heading (`:4101`). The review's Remarks section (Critical "None", W1–W5, O1–O2) is `:3928-4106`; `## Status of earlier findings` starts at `:4109` | corrected in place to `:3928-4106`, with a note |
| V-68 | 00 RK-1; 02 D-S2 | "AP6 raised no remark against P39/P40's or P43's mechanisms" holds for Critical and Important remarks: AP6's status table closes P39 (`:4114`), P40 (`:4115`) and P43 (`:4118`, "with O2's mechanism note"). But AP6's **optional** O2 (`:4101-4105`) concerns P43.1's one-id-per-element-type rule and P39.4's `TraversalScratch` client. This plan voids O2 under U-2 (02 Document steps), so the release of D-S1(i) and D-S2 stands | annotated in place (RK-1, D-S2) |
| V-69 | files 00–05, living text | **V-61's rule applied.** Every allocator citation carried from rev ≤ 5.1 was read at `b716a5dc` at N and at N+7. Every one misses by exactly 7 lines, and each is moved.<br>• **00:** `:556-560`→`:563-567`; `:1124`→`:1131`; `:1129`→`:1136`; `:3164-3165, 2711`→`:3171-3172, 2718`; U-4 `:1787`→`:1794`; U-5 `:1152`→`:1159`.<br>• **01:** `:61-72`→`:68-79`; `:1168-1172, 3746`→`:1175-1179, 3753`; KC-01 `:2696`→`:2703`; KC-05 `:1813-1827`→`:1820-1834` and `:868`→`:875`; KC-06 `:2007-2013`→`:2014-2020` (these three are bare allocator citations: SC-1/SC-2, the zero-extra-TLS bullet, P20's liveness proof); KC-19a `:3694`→`:3701`, `:3655`→`:3662`, `:3335`→`:3342`; KC-19b `:3530-3535`→`:3537-3542`; R-A `:2700`→`:2707`; R-B `:3770-3772`→`:3777-3779`; R-D `:3478-3486`→`:3485-3493`; R-E `:1152`→`:1159`; §5 `:3145-3147`→`:3152-3154`, `:3203`→`:3210`; §6 `:1688-1690`→`:1695-1697`, `:3694`→`:3701`.<br>• **02:** D-M2 `:892-896`→`:899-903`; §6 `:3799`→`:3806`.<br>• **03:** §3 `:923`→`:930`; leg (6) `:3825`→`:3832`.<br>• **05:** MS-01 `:3692`→`:3699`; MS-03 `:3657`→`:3664`; §6 `:3825`→`:3832`.<br>01 R-D's `:1866` is a ledger citation and is unchanged. History (the §9 records, §10 changelogs and critic logs) keeps the numbers it recorded. 03 UG-08's `:1181` was already at `b716a5dc` | all moved in place; no other miss found |
| V-70 | 01 §2.1 (c), H-04; 02 D-E21; 01 §2.1 RP-2 capture point; 02 B1 | **(a)** `fire_despawn_hooks` is `[J]…/ecs_master/entity_api.rs:705-806`, not `:705-767`. Its id copy is `:733-742`, and its on_despawn, on_replace and on_remove passes are `:756-805`. D-E21's red-first test asserts all three passes, so `:733-766` (the on_despawn pass only) is corrected to `:733-805`. **(b)** `ActionState::consume` is `[J]crates/boyko_input/src/action/state.rs:186-193`; `:186-191` stopped before `consumed.set` (`:192`), and `TickActions` records the consumed set. **(c)** KF-34's ledger block is `RUNTIME-DATA-LEDGER.md:1459-1470`; its group note at `:1470` carries the rev-1 "rung 1d" claim that B1 restates (AP6 itself cited `:1459-1468`) | corrected in place: (a) `:705-806` and `:733-805` (01 (c) also says the three passes are meant); (b) `:186-193`; (c) `:1459-1470` |
| V-71 | 02 D-E23 | The caller list is complete for code at `d552be05`: `process.rs:106`, `runner.rs:2282`, `demo/src/app.rs:513`, the three test files, and `fixed_loop.rs`'s own unit tests. Comments also name the getters in ten files outside the lock set; `fixed_loop.rs:20`'s intra-doc link is inside it. **Not addressed by the patch:** `FixedTime::discard_overstep()` (`fixed_time.rs:156`) stays `pub` and mutates the accumulator that D-E23 makes `pub(crate)`; it is a write, not a read, so rule B's read clause does not see it | the comment files are listed in 02 D-E23; `discard_overstep` is listed for critic pass 6 (pass 2) below |
| V-72 | 00 §11 (rev 6.1) | **Held:** `type_name` (all three points); `cargo test` (both sentences); `_mm_getcsr` (deprecated since 1.75.0, inline assembly recommended; "no guarantees whatsoever"); GitHub artifacts (the page title is "Store and share data with workflow artifacts"); `gh run download [<run-id>]` with `-n <name>`; Bevy `Time<Fixed>` (the fixed clock is set as the generic `Time` during `FixedUpdate`; "0, 1 or more times"); RFC 3127 (its example is a panic message carrying a `.cargo/registry/src/…` path); cargo#5505 (closed, "Reproducible builds: Automatically remap $CARGO_HOME and $PWD").<br>**Did not hold as written:** (a) the `_mm_getcsr` page does not say "only the status bits are unspecified"; that is the architect's reading. The UB statement that 01 and 02 attribute to "00 §11" is on the `_mm_setcsr` page. (b) H-21 cites "00 §11, Dawson", and §11 had no Dawson entry | (a) annotated, and `_mm_setcsr` added; (b) Dawson's 2013 "Floating-Point Determinism" added, which says rogue code that changes the rounding mode makes results subtly wrong |
| V-73 | 02 RP-2 gates | `boyko_demo` has no `boyko_input` dependency (`[J]crates/boyko_demo/Cargo.toml`: `boyko-ecs`, `boyko-macros`, `boyko-diag`, `boyko-threadpool`, `boyko-log`, and third-party crates), and `boyko_input` is not a leg-(7b) census crate. "`boyko_demo` links neither the crate nor the `boyko_input` pair" therefore holds, but no UG-15 leg can observe the pair; its zero-cost shape rests on review. The same applies to D-E23's `boyko_input` edit | annotated in place |
| V-74 | 00 §10 | The brief asks the writer to append critic pass 6 (pass 1)'s review verbatim. The review text was not in the writer's brief | heading added with a note; reported to the orchestrator |
| V-75 | every other citation rev 6.1 adds | These hold.<br>• **`[Jw]`/`[J]` at `d552be05`:** `event_writer.rs:50-63` (the 24-B state: pointer, `u32`, pad, `u64`), `:89-91`, `:126-128`, `:131-133`, `:162-164`; `event_dispatcher.rs:290-292`; `fixed_loop.rs:82-83`, `:82-87`; `fixed_time.rs:125-171`; `entity_api.rs:705-707` (`#[cold] #[inline(never)] fn fire_despawn_hooks`), `:707`, `:734`, `:1051`; `component.rs:67-78` (`STORAGE_IS_DENSE`'s doc; the const is `:79`); `component_registry/mod.rs:109-121`, `:149`, `:173-178`, `:920-921`, `:967`; `archetype_master.rs:37-52`, `:594` (both `clear()` at `:987` and `remove_archetype` at `:686` bump the counter), `:968-971`; `add_existing_archetype` (`:477`) has no caller in `crates`; `ecs_master.rs:157`, `:575`; `query.rs:682` (`par_for_each_chunk` takes a `Fn` with no return value); `schedule.rs:122`, `:286`, `:561`, `:567`, `:582`, `:1436` are the `pub(crate)` systems and the whole public fn surface; `schedule_builder.rs:1035-1075` (Kahn with a FIFO ready queue); `app.rs:597-605`, `:614-629`; `access.rs:81-99`, `:214` (`Access`'s fields are `pub(crate)`, and its public fns are the adders, `is_universal`, `universal`, `extend` and `conflicts_with`); `system_meta.rs:255`; `commands.rs:169-173`, `:344-352`, `:349-352`; `tag_api.rs:47`; `tags.rs:134`, `:201` (P40's row 8 expects `None` from `tag_by_name` for an enable-tag name, `ALLOCATOR-DESIGN-SPACE.md:3705`); `hooks/builder.rs:149`; `boyko_input/src/action/process.rs:27-28`, `:72-76`, `:102-109`, `:121`; `state.rs:10-23`; `runner.rs:638`, `:2282`; `boyko_demo/src/app.rs:513`; `app_fixed_timestep.rs:53-54`, `:105-107`, `:230`; `particle_containment.rs:181`; `rust-toolchain.toml:38-39`; `mat.rs:307` and `camera.rs:483, 727, 728, 926, 927` (V-63).<br>• **`[W]`:** `.cargo/config.toml:106-113` sets only `target-cpu` for the three x86-64 triples. No `remap-path-prefix` or `trim-paths` appears in `[W]` or `[J]` (`.cargo`, `Cargo.toml`, `.github`).<br>• **`[M]`:** allocator `:5-10` (the status block; `:11` is blank), `:337` (G5), `:2619-2623` (P29's five rows), `:3873-3881` (the pass-6 log), `:4022-4056` (AP6 W4, through "What is needed"), `:4118`; AP6's Q4 and Q5 are `:4155`, `:4156`; ledger `:1446-1457` (KF-33), `:1783` (rung 5); `ledger/pool-utils-log.md:18` ("Only replacing crossbeam-deque (KF-34) removes it")<br>• **Rev 6's `[C]`** citations were verified in rev 6 and are unchanged | none |

**Left unchanged, outside the patch (for critic pass 6, pass 2).**
- 02 §1 rule 8 still names AP6 as a pass that rungs wait for; AP6 has run, and AP7 now holds C1.
- The titles of 01–05 still read "rev 6"; the patch changes only 00's title, and 01's header gains a rev-6.1 paragraph.
- 01 H-21 names the flush-to-zero flag "FTZ"; the usual MXCSR name is FZ (bit 15).
- 01 rule B's former writer notes (V-62) were replaced with the rule; their facts are now in the rule's archetype-id bullets.
- `FixedTime::discard_overstep()` stays `pub` after D-E23 (V-71). It empties the tick accumulator (`[J]…/time/fixed_time.rs:156-158`), so a caller changes how many ticks later frames run. Rule B's read clause does not cover that write, and the patch does not say whether a replay session must refuse it.

## 10. Revision log

### Rev 6.2 (2026-09-22): one rung added by the owner's instruction

- **SI1 — a spatial index as a first-class kernel feature** (02 §2, Phase E, "Kernel-feature lane"; 02 §3
  DAG line `SI:`). Owner, 2026-09-22, after asking how the physics campaign's tree broadphase relates to the
  ECS: it is ECS-native but physics-owned (its row source, sink, predicate, scratch ids and identity are the
  physics crate's), and principle 0 says a capability one subsystem needs becomes a kernel feature every system
  uses. SI1 lifts the packed 8-wide BVH, the persistent static set and the segment-stream assembly into the
  kernel — any dense column view as the row source, a caller-owned sink, the predicate as a parameter with an
  8-wide kernel per shape, per-instance scratch ids, `Entity` identity — with the physics tree broadphase as its
  first client (bit-identity with AllPairs kept). Prerequisites U7, D-S2, D-E8 and the tree's own C4/C5; size L;
  before F4 or after it, never during. No other rung, gate or number in this plan changes.

### Rev 6 critic pass 2 log (final)

Verdict: CHANGES_REQUESTED, no Critical remark. Closed by orchestrator ruling (2026-09-17); the remarks below (W1–W5) and the report's optional remarks (O1–O9) are OPEN. No revision answers them yet.

*Writer (2026-09-17): the blockers are recorded verbatim. The report follows verbatim, except that its headings are moved down three levels so that it nests under this heading; its level-3 and level-4 headings both become level 6, because Markdown has no level 7. Its line citations (`00:…`, `01:…`, `02:…`, `03:…`) refer to the rev-6.1 text the critic read, before this log and the status line above were added.*

- W1 (Important): UG-22 scene S-R2 can never meet its own refusal floor. D-E20 applies the lane capacity per writer (02:181), and D-E20's own test asserts that 40 sends per writer against capacity 64 refuse nothing at any W (02:319-322). S-R2 sends exactly 40 per sender against capacity 64 (03:218), yet requires 'at least one refused send per run' (03:268). Once D-E20 lands, UG-22 is therefore red by construction, or the floor gets dropped and H-02's refusal half goes unwitnessed. This leaves pass-1 W6's refusal half open.
- W2 (Important): events are invisible to the boundary report and to D-E8. EventReader and EventWriter declare no access ([Jw]crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs:338-345, event_writer.rs:213-223). Access holds only component and resource masks and states that events do not participate ([Jw]…/system/access.rs:47-57, :113-123). SystemMeta records nothing about events, and D-E22 exposes only &Access. Consequences: report check 3 (01:337) has no input; D-E8's App::finish refusal of a Main reader of an every_tick type (02:169, 01:982) has no mechanism; a Main-written event read by Fixed (a pacing-dependent Main-to-Fixed crossing) is caught by no check and has no register row or red control.
- W3 (Important): boundary check 1 is red by construction whenever a universal-access (exclusive) system runs outside Fixed. Check 1 counts 'writes by exclusive systems' (01:335), and exclusive systems carry Access::universal ([Jw]access.rs:125-147), so every Fixed read intersects them. (a) UiPlugins keeps one exclusive Main system, ui_bind_apply ([M]docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:1839; 00:92), so every UI game's report is red. (b) The replay crate's Main-index-0 check reads structural_generation() and (archetype, row) per entity. An external crate can do that only through &EcsMaster: every UnsafeEcsCell accessor is pub(crate) ([Jw]…/system/unsafe_ecs_cell.rs:94-417), no SystemParam gives a world read, and KC-33's world-read param (D-E14, 02:175) is not a prerequisite of RP-2 (02:417). That system is therefore exclusive, which makes every UG-22 scene's report red (01:340). The plan states neither the access class of the replay systems nor an exemption.
- W4 (Important): H-18 is caught only for hashed state, and U-28's 'why it suffices' ignores unkeyed rows. Rule B limits Fixed reads to keyed entities (01:217), but nothing enforces that a Fixed query filters by ReplayKey. Main may spawn, despawn and insert on unkeyed entities through Commands, which declares no access ([Jw]…/params/commands.rs:404-415). The only runtime catch, 'a hashed component on an unkeyed entity is red' (01:309), and red control r6 (03:280, a declared write) cover hashed types only. Example: a Main system spawns unkeyed static colliders (an unhashed type the Fixed broadphase reads). Record and play then diverge while the boundary report, the move digest and the Main-write check all stay green. This is the architect's own rev-6 open item 1 (00:580), still unclosed.
- W5 (Important): the replay startup gate starves every event reader during loading. RP-2 inserts Time paused until the startup assets load (01:267, 01:341-343). With a Fixed schedule the default event policy is WaitForFixed ([Jw]crates/boyko_ecs/src/ecs/core/app/app.rs:76-77, :591-595), and the frame swap is gated on substeps (:713-720). A paused Time yields 0 substeps ([Jw]…/time/time.rs:201; fixed_loop.rs:41-43), which is the documented pause hazard: swaps are held indefinitely and sends are refused at lane capacity (app.rs:79-88). So every replay game's Main event readers starve for the whole loading period. If the gate waits on an event-signalled load, no tick ever runs. The plan sets no policy for the session and does not name the gate's load signal.

#### Architecture review: unified system plan rev 6.1 (critic pass 6, pass 2)

##### Verdict
[ ] APPROVED — the plan is ready for implementation
[X] CHANGES REQUESTED — no Critical remark. Five Important remarks (W1–W5) remain, all in the replay lane, and all are specification fixes rather than redesigns. Every pass-1 blocker is resolved except part of pass-1 W6, which carries forward as W1 below plus the optional O1.

**Scope.**
- This pass covers the rev-6.1 delta, the pass-1 blockers (C1, W1–W8), and the checklist in the brief.
- I read all six plan files in full, from the main-checkout working copy `[M]`.

**Reading method.**
- I had no shell, so I could not run `git show d552be05:…`.
- Code under `boyko_ecs`, `boyko_input`, `boyko_macros` and `boyko_threadpool` was read from the `D:/wt/joltab` working copy, outside `boyko_physics`, and is cited as `[Jw]`.
- Every `[Jw]` line the plan cites that I checked sits at the cited line.
- Web check: the `std::any::type_name` page.

##### Status of the pass-1 blockers

- **C1 — resolved.**
  - File 00 is at rev 6.1 (00:1).
  - U-12 is now refactor-last (00:118).
  - U-19 now describes the portable arm only (00:125).
  - U-20's rationale is corrected and its Q-9 overturn removed (00:126).
  - U-21..U-28 are defined (00:127-134).
  - The phase map puts every RF wave in F4 (00:138-147).
  - A grep of files 01–05 finds no RF wave, and no dependency on one, before F4.
- **W1 — resolved.**
  - AP6 is recorded as run (02:91).
  - Every AP6 remark is mapped (02:100-112; 05:252).
  - D-S1(i)'s tests are redesigned (02:300-312).
  - Leftovers are listed under O4 and O8.
- **W2 — resolved.**
  - (a) `TickActions` is the registered `ActionState<A>` (01:219, 01:269-272). The capture point matches the frame order: Fixed runs, then Main ([Jw]app.rs:725-744; `process.rs:72-76`, `:102-109`, `:121`).
  - (b) Detection now goes through `structural_generation()`. That getter is public ([Jw]archetype_master.rs:594-595) and is bumped by `remove_archetype` (:686) and by `clear` (:987). It is reachable through `EcsMaster::archetype_master()` ([Jw]ecs_master.rs:575).
  - (c) D-E22 supplies the missing access data (02:183). Its data source is still incomplete; see W2 and W3.
- **W3 — resolved** by D-E23 / U-25 (02:184; 00:131).
  - "0 added lookups" holds against [Jw]fixed_loop.rs:51-89.
  - The writer's open item on `discard_overstep` needs no remark: it changes only how many ticks later frames run (pacing), not tick content or `elapsed` ([Jw]fixed_time.rs:152-164). `set_timestep` is covered by the header check (01:229).
- **W4 — resolved.** Derived keys now use an issue-time counter (00:130; 01:292-294). A bound on the counter table is O6.
- **W5 — resolved for table storage** (U-26, 00:132; 02:182). I checked the `type_name` premise on the web: the name is diagnostic, not unique, and may change between compiler versions. Because the contract is per binary and ties break on `TypeId`, the premise holds. The dense-storage residual is O2.
- **W6 — partly open.**
  - r1 (seeded delays) and r4 (recycle ticks plus a precondition counter) are fixed.
  - r2's order half is fixed, but its refusal half is broken; that is W1 below.
  - r3's first-touch perturbation does nothing (O1). r3 still goes red through its despawn half.
- **W7 — resolved** by U-27 (00:133; 03:243-253).
- **W8 — resolved** (02:402-409).

##### Remarks

###### Critical
None.

###### Important

###### W1. S-R2's refusal floor cannot be met on a correct tree
- **Where:** 03 §7 S-R2 (03:215-218), the anti-vacuity list (03:268), D-E20 (02:181, 02:319-322).
- **Problem:**
  - Under D-E20, the lane capacity applies per writer.
  - S-R2's two senders send 40 each per `every_tick` window against a capacity of 64, so nothing is refused.
  - D-E20's own red-first test asserts exactly that: 0 refused at every W.
  - Yet S-R2 requires "at least one refused send per run".
- **Consequence:** after D-E20 lands, UG-22's S-R2 is red by construction (its anti-vacuity fails). If someone drops the floor to get green, the refusal fold is constantly 0, and the gate no longer shows that refusals are deterministic under the fix.
- **Confidence:** CONFIRMED (plan text).
- **What is needed:**
  - In the burst ticks, each sender must exceed its own per-writer capacity, so refusals happen per writer and identically at every W. They still differ across W under r2.
  - Restate the floor to match.

###### W2. Events are outside `Access`, so check 3, D-E8's refusal and Main→Fixed event crossings have no data
- **Where:** 01:333, 01:337; 02:169; 01:982; 02:183.
- **Problem:**
  - `EventReader` and `EventWriter` declare no access ([Jw]event_reader.rs:338-345; event_writer.rs:213-223).
  - `Access` has four component and resource masks only, and says events do not participate ([Jw]access.rs:47-57, :113-123).
  - `SystemMeta` stores nothing about events.
  - D-E22 hands the report only `&Access`.
- **What follows:**
  - Check 3 ("every Fixed-read event type is `every_tick`") cannot be computed.
  - D-E8's `App::finish` refusal of a Main reader has no mechanism.
  - A Main system that sends an event a Fixed system reads is a pacing-dependent crossing that no check sees. The report has no bits for it. The Main-write hash check sees nothing, because the send changes no hashed state before the tick. The move digest does not apply.
- **Consequence:**
  - An implementer stops at D-E8.
  - A game whose Main UI sends a gameplay event read in Fixed diverges between record and play (Main sees live input in play mode, 01:271), while its report is green.
  - A Fixed reader of a non-`every_tick` type keeps H-19 with a green report.
- **Confidence:** CONFIRMED.
- **What is needed:**
  - A build-time, per-system record of event reads and writes, exposed through D-E22 under the same "no object code until instantiated" rule.
  - The Main-writer → Fixed-reader crossing added to the report and the register (it is H-18's event half), with a red control.

###### W3. Check 1 is red whenever a universal-access system runs outside Fixed, including replay's own Main-index-0 check
- **Where:** 01:335, 01:223, 01:264, 01:318-327, 02:417.
- **Problem:**
  - Exclusive systems carry `Access::universal` ([Jw]access.rs:125-147), so every Fixed read "intersects writes by exclusive systems".
  - **(a) UI games.** `UiPlugins` keeps one exclusive Main system ([M]ENGINE-RUNTIME-ECS-DESIGN.md:1839; 00:92).
  - **(b) The replay crate itself.** Its Main-index-0 check reads `structural_generation()` and each entity's (archetype, row).
    - From an external crate, the only route is `&EcsMaster`: every `UnsafeEcsCell` accessor is `pub(crate)` ([Jw]unsafe_ecs_cell.rs:94-417).
    - No `SystemParam` impl provides a world read.
    - KC-33's world-read param lands in D-E14 (02:175), which is not an RP-2 prerequisite.
- **Consequence:**
  - (b) makes check 1 red in every replay session, including all three UG-22 scenes (01:340). UG-22 is then red at RP-3 by construction.
  - (a) makes every UI game's report permanently red, so real crossings cannot be told apart from noise.
- **Confidence:** CONFIRMED for the check arithmetic and for (a). For (b), CONFIRMED on the kernel surface in the working copy, PLAUSIBLE as to RP-2's actual build, since the plan does not state the systems' access class.
- **What is needed:**
  - Define how the report treats universal-access systems outside Fixed.
  - Give replay's checks a non-universal route, or exempt them by identity.
  - State what a `UiPlugins` game receives.

###### W4. H-18 is caught only for hashed state; unkeyed rows are unguarded
- **Where:** U-28 (00:134); 01:217, 01:224-227, 01:309, 01:366; r6 (03:280); architect's rev-6 open item 1 (00:580).
- **Problem:**
  - Rule B limits Fixed reads to keyed entities, but no check ensures that Fixed queries filter by `ReplayKey`.
  - Main may freely spawn, despawn and insert on unkeyed entities through `Commands`, which declares no access ([Jw]commands.rs:404-415). U-28 covers keyed entities only.
  - Main-side dense or bitset writes to a Fixed-read type go unchecked too: U-28's exemption is conditioned on "no Fixed system reads", and nothing checks that condition for `Commands` writes.
  - The only runtime catch (01:309) and r6 (a declared write) cover hashed types only.
- **Consequence:**
  - A Main level-streaming system spawns unkeyed static colliders (an unhashed type the Fixed broadphase reads).
  - Record and play then diverge, while the report, the move digest and the Main-write check are all green.
  - The divergence is found only as a later verify mismatch, and the tier-2 dump names the victim key, not the cause.
  - The reverse error also exists: a hashed type shared with non-sim entities makes every such entity red, whether or not Fixed reads it.
- **Confidence:** CONFIRMED (plan and code); the example scenario is PLAUSIBLE.
- **What is needed:**
  - A mechanical check of the keyed/unkeyed partition for Fixed-read component ids. For example: archetypes that contain a Fixed-read id but lack `ReplayKey`, derived from D-E22's access data on the cold generation-change path.
  - A red control for an unhashed Fixed-read component spawned from Main.
  - A restated U-28 rationale.

###### W5. The startup gate's paused `Time` holds every event swap under the default policy
- **Where:** 01:267, 01:341-343.
- **Problem:**
  - With a Fixed schedule, the policy defaults to `WaitForFixed` ([Jw]app.rs:76-77, :591-595), and the frame swap is gated on substeps (:713-720).
  - A paused `Time` gives 0 substeps ([Jw]time.rs:201; fixed_loop.rs:41-43).
  - The code documents the result as the pause hazard: swaps are held indefinitely, all readers starve, and sends are refused at capacity (app.rs:79-88).
  - The plan sets no session policy and does not name the gate's load signal.
- **Consequence:**
  - In every replay-recording game, Main event readers see nothing for the whole loading period, and sends past lane capacity are refused.
  - If the gate waits on an event-signalled load, no tick ever runs.
  - UG-22's scenes may load no assets, so they would not show it.
- **Confidence:** CONFIRMED for the held swap; PLAUSIBLE for the deadlock.
- **What is needed:**
  - Specify the session's event policy. Check 3 already forces Fixed readers onto `every_tick`, so the remaining frame-gated types have only Main readers.
  - Make the gate's load signal a state read, not an event.
  - Give one scene at least one startup asset.

###### Optional

- **O1. The churn arm mints in the same order as the scene, not the reverse.**
  - The bundle derive mints ids in field order ([Jw]crates/boyko_macros/src/bundle.rs:118-123, :341-345). The scene inserts `(HookB, HookA)` (03:222), so B is minted first; the churn also touches B first (03:236).
  - Under r3, insert order by id is [B, A] in every arm, which equals declaration order, so the churn arm cannot catch the insert half. Only D-E21's own test guards it.
  - r3 still goes red through the despawn half.
  - Fix: touch A first, and correct H-04's catch cell (01:352).
- **O2. Dense hooks fire in first-insert order.**
  - Dense despawn fires ([Jw]entity_api.rs:1124-1164) and the dense copy in materialize (materialize.rs:859-862) iterate `dense_ids`, which are in first-insert order ([Jw]dense_registry.rs:122-127).
  - D-E21's per-archetype permutation cannot serve these loops, and its audit criterion (01:248) would not flag them.
  - Exposure is narrow: a Fixed-hooked dense type first inserted by Main.
  - Fix: extend D-E21's criterion and its test to the dense loops.
- **O3. S-R3's recording needs a stated pacing seed.**
  - S-R3 is recorded "with random pacing" (03:227, 03:241), and that pacing enters the recorded `TickActions`.
  - A committed golden needs either a committed pacing seed, or a committed recording — and the player would refuse a committed recording by image hash (01:279, 01:282).
  - Fix: state which.
- **O4. G-MINT-3 does not match the chosen tag path.**
  - It expects `Err(IdSpaceExhausted)` (02:306), but `try_register_tag` returns `Option` ([Jw]tag_api.rs:47).
  - Two `None` returns guard `MAX_COMPONENTS` ([Jw]tags.rs:189-190; component_registry/mod.rs:967-970). Name which one the mutation deletes.
- **O5. Two always-on kernel costs are not reflected in the metric row.**
  - D-E8 costs every game one bool, one predicted branch per frame and a second `fixed_advance` instantiation (02:169).
  - D-E21 adds two `VmColumn`s to `EcsMaster` (02:182).
  - The metric row (00:98) says "0" only for (g) and D-E22. State these costs there.
  - A zero-branch alternative exists: register the `every_tick` swap as a Fixed system only when some type opts in.
  - I found no always-on modding cost.
- **O6. `DerivedKeyCounters` needs a stated reuse rule.** Define stale-entry reuse (01:294), so the table is bounded by the distinct parents in a tick rather than every parent ever seen. MQ-23's churn arm would show the growth.
- **O7. The OS leg's toolchain is not pinned.** Only the Windows job pins rustc (03:247), but the architect's open item 3 (00:518) assumes the OS leg pins it too.
- **O8. Stale text.**
  - 02 rule 8 still names AP6 (02:23-25).
  - Files 01–05 are still titled rev 6.
  - The 00 §7 subheading still says "revised in the next plan revision".
  - H-21 says "FTZ"; the MXCSR bit is named FZ (01:369).
- **O9. `ReplaySpawner`'s slot counter needs a home.** State that it is per App, not a process static (01:286), so record-then-verify in one process gets identical slots.

##### Positive
- C1 is applied thoroughly. Q-4 is honoured everywhere, and options B and E are gone from file 05.
- msvc is consistently the gate host across U-22, UG-08, UG-09 (the joined `target."cfg(windows)"` key, with `--list` and `running N` pins), AH, 03 §3 and MQ-13.
- KC-36's premise holds in the working copy:
  - Dispatch is wave-synchronous, and the scan has no W term ([Jw]schedule.rs:1079-1132).
  - Kahn's ready queue is FIFO (schedule_builder.rs:1056-1073), so "ReplayPlugin first" does yield index 0.
  - Queries iterate archetypes in ascending id order (query_state.rs:236-256).
- Keep these decisions:
  - detection through `structural_generation()` instead of kernel refusals;
  - U-24's issue-time counter;
  - U-25's zero-lookup move;
  - U-27's same-artifact machine arm;
  - U-28 as a permanent rule;
  - padded per-worker hash slots with no shared atomic;
  - D-S1(i)'s one-binary-per-case isolation.
- The x86-64-v3 contract claim holds. The `+fma` `compile_error!` was removed on 2026-09-02 ([Jw]crates/boyko_physics/src/sdf_simd.rs:28-34, :59-62). The float research's statement that it excludes v3 is stale, and the plan does not rely on it.

##### Open questions for the architect
1. Is replay's hashing built as per-type non-exclusive systems plus one exclusive check system, or as one exclusive system? W3's fix depends on the answer.
2. Rev-6.1 open items:
   - 1 is accepted, subject to W4.
   - 2 is accepted; FIFO is verified, and the report checks the index.
   - 3 is accepted, subject to O7.
   - 4 and 6 are accepted.
   - 5 is accepted, but see W3 (a).

**Checklist topics that showed no problem:**
- Prefetching, PGO and SIMD: replay code runs only in replay sessions.
- False sharing: the hash slots are `CachePadded`.
- New atomics: none. D-E20 removes one TLS read per send.
- Allocations: replay storage lives on kernel columns, and UG-02 is unchanged.
- Rung order: RP-2 reaches D-E20 through fixed order #10, D-E23 follows A5, and RP-0 follows B3.
- Owner answers: Q-1, Q-2, Q-4, Q-5, Q-7 and Q-9 are applied everywhere they apply.

### Rev 6.1 changelog (critic pass 6, pass 1)

Rev 6.1 is the architect's patch answering critic pass 6 (pass 1): C1, W1–W8 and O1–O11. It is a patch, not a re-emission.
- **What the architect read:** documents in `[M]` @ `b716a5dc`, and code in the `[Jw]` working copy outside `boyko_physics` (`boyko_ecs`, `boyko_input`, `boyko_app`, `boyko_demo`), plus `[W]`.
- **Tools:** Read and Grep only. No shell, no graphify, no cargo, no git, no timing. The web pages are in §11.
- **Verification:** the writer checks every `[Jw]` citation at `d552be05` (§9, rev 6.1 re-verification).

| # | Change | Reason | Where |
|---|---|---|---|
| 1 | File 00 brought to rev 6: title, index, status, provenance (`[Jw]`/`[C]`/`[W]` defined), §1 replay goal, §2 replay rows. U-12 is restated as refactor-last; U-18 and U-19 as portable arm only, with overturns (a)–(d) realigned to 01 §8 and MQ-13; U-20's rationale is corrected and its Q-9 overturn removed. U-21..U-28 are defined. The phase map moves every RF wave to F4 and adds AH, the REPLAY lane and AP7. RK-1, RK-5 and RK-16 are updated, and Q-12 is added. The §8 readiness claims are made true. | C1 | 00 |
| 2 | AP6 is recorded as run (0 Critical, W1–W5 open). DOC-1 → **AP7**, which reviews only the rev-2.5 delta and holds only C1. D-S1(i) and D-S2 are released from the AP6 hold. AP6 W1–W5, O1, O2, Q4 and Q5 are each mapped to a rung or voided with a reason. D-S1(i)'s mint rows are redesigned: one binary per stateful case, relative pins, the engine tag path, G-MINT-3 not ignored, row 8 as a deterministic `cfg(test)` rendezvous. | W1 | 00 §5, §6; 02 Document steps, B1, C1, D-S1(i), D-S2, §4.4, DAG; 03 UG-19; 05 §5 |
| 3 | `TickActions` is defined as each registered `ActionState<A>`, captured and restored at Fixed index 0. `boyko_input` gains a generic capture/restore pair inside RP-2's lock set. Rule B's `clear()`/`remove_archetype` refusal becomes detection through the public `structural_generation()`. **D-E22** adds the generic access visitor that the boundary report needs. `ReplayPlugin` must be added first, and the report checks it (the FIFO topological sort gives the replay systems index 0). The windowed host gains no call; a game that records under it runs the report in its own test. | W2 | 01 §2.1, §7; 02 D-E22, RP-2, §4.3 |
| 4 | **D-E23 / U-25:** `FixedTime`'s three frame-level getters move to `Time`, whose Fixed read is already refused, at 0 added lookups. H-20 and red control r10 are added. The startup gate keeps `Time` paused until assets load, so `elapsed()` at tick k is k × timestep. | W3 | 01 §2.1; 02 D-E23; 03 §7 |
| 5 | **U-24:** derived keys take an issue-time per-(parent, tick) counter, read inside the apply window; the caller index is removed. RP-2 test: two sites, one parent. The critic's option was taken. `ReplayKeyIndex` becomes open-addressed (O5), and MQ-23 gains a spawn-churn arm. | W4, O5 | 01 §2.1, §6; 02 RP-2; 03 MQ-23 |
| 6 | **U-26:** ops with no declaring bundle (despawn, clone, archetype-driven removes) fire in canonical type order (`type_name`, then `TypeId`). The permutation is built lazily on the cold fire path. D-E21's scope and touch set are extended, and 01 §2.1 (e)'s `TypeId` clause is narrowed. | W5 | 01 §2.1; 02 D-E21, mode table, §4.3 |
| 7 | Red-first tests are added for D-E20 (order, refusals, worker `send_event`), D-E21 (insert and despawn, through child processes), D-E22 and D-E23. UG-22's scenes now force each control red by construction: seeded delays (r1); an order-sensitive fold and refusals at capacity 64 (r2, H-02's refusal half); an r3 pair minted in reverse by the churn arm, with every arm in a fresh process; recycle ticks with a claim race (r4), plus a precondition counter. | W6 | 02 red-first list; 03 §7 |
| 8 | **U-27:** the machine arm runs one CI-built artifact on the CI runner and on the gate host. The two-host rebuild and image-hash comparison is dropped. Anti-vacuity: CPU brand strings must differ, and UCRT versions are recorded. | W7 | 00 §3; 03 §7 |
| 9 | D-M6's structural check takes the symbol class from the post-LTO object, and the `.bss` size from the image. | W8 | 02 D-M6 item 7 |
| 10 | **U-28, the move clause.** Nothing outside Fixed moves a keyed entity, and verify sessions detect a breach with a move digest. This goes beyond the interim rule the critic proposed in O8: gameplay Fixed systems depend on row order just as physics does, so H-16 needs a permanent rule, not a U7 fix. UG-22's arm deferred to U7 is withdrawn, r11 is added, and r8 is withdrawn with its reason. The Phase-D exit has no deferred arm. | O8 plus new evidence | 00 §3, §5, Q-12; 01 §2.1; 02 rule 10, Exit, Phase E; 03 §7 |
| 11 | `every_tick`'s default path is byte-identical, with one branch per frame (O1). The per-worker-slot hash merge is named (O2). The census covers `boyko_scene`, and its walker is public (O3). H-21 adds an MXCSR read-only check (O4). `send_event` is dispatcher-only (O6). RP-0 needs B3, and A2 needs AH (O7). A disposition is given for a profile or OS red (O9). `Entity` fields hash the referent's key or a sentinel (O10). 05 RM-1 and the 04 `device.rs` row are corrected (O11). | O1–O4, O6, O7, O9–O11 | 01; 02; 03; 04; 05 |
| 12 | UG-16's red control uses a called `pub fn`. B3's probe gains item (v): whether uncalled `#[no_mangle]` items and control (viii)'s static survive on msvc. | critic open question 1 | 02 B3; 03 UG-16 |
| 13 | *(writer)* Every allocator citation carried from rev ≤ 5.1 in the living text of files 00–05 is moved 7 lines down to its `b716a5dc` position (V-61's rule); each was read there first (§9 V-69). History (§9 records, §10 changelogs and critic logs) keeps the numbers it recorded. | V-61 | 00 §1, §2, §3 (U-1, U-4, U-5); 01 §1, §2, §4, §5, §6; 02 D-M2, §6; 03 §3, §6; 05 §3.2, §6 |

**Answers to the critic's open questions (pass 6).**
1. **Does an uncalled `#[no_mangle]` item survive `/OPT:REF` in an msvc executable?** This is not established; M-10 measured gnu only. The answer is no longer load-bearing:
   - UG-16's red-first control is now a kernel `pub fn` called from `boyko_demo`'s `main`, which survives on both hosts.
   - The uncalled case is B3's probe item (v), which also checks whether red control (viii)'s static can go red on msvc. A control that cannot is redesigned at B3, never dropped.
2. **Which "tick" does a root key use?** The replay tick index: Fixed ticks counted from the startup gate. It is not the kernel `Tick`, which advances per system run in every schedule. The startup gate's paused `Time` makes tick 0 well defined (01 §2.1, Keys).
3. **Which slot do startup systems holding a `ReplaySpawner` get?** They continue the one `init_state` counter after Main's and Fixed's schedules, in startup registration order, all on the one thread `finish` runs on (`[Jw]…/app/app.rs:614-629`). Their keys use tick `2³¹ − 1`, so they cannot collide with Fixed keys. A `ReplaySpawner` in Main is a boundary-report red.

### Open for critic pass 6 (pass 2) (architect)
1. **U-28 constrains games that record replays** (owner Q-12 with a default). The critic's O8 asked for an interim rule; the architect made it permanent, because order-dependent gameplay systems iterate rows too. U7 therefore no longer closes H-16 for replays.
2. **`ReplayPlugin` must be the first plugin that registers systems.** The kernel's FIFO topological sort provides the ordering and the boundary report checks it; no kernel "first" set is added. The alternative is a kernel anchor set with build-time edges.
3. **U-26 orders by `type_name`.** Rust documents it as diagnostic-only and changeable across compiler versions. That is acceptable only because the contract is per binary and the OS leg pins the same rustc. The `TypeId` tie-break is per binary.
4. **D-E23 is an API break** in `boyko_ecs`, `boyko_input`, `boyko_app` and `boyko_demo`. It also needs UG-12, because it edits the runner and a render test.
5. **The windowed host gains no boundary-report call.** A game that records under it relies on its own test to run the report.
6. **RP-2's MXCSR check** reads only the control bits and never writes them. It therefore has no live red control, only a decoder unit test (a live write is UB).

### Critic pass 6 log (pass 1)

*Writer (2026-09-17): not appended. The rev-6.1 brief asks for critic pass 6 (pass 1)'s review to be recorded here verbatim, but the review text was not in the writer's brief, and this writer does not reconstruct a critic's text. Reported to the orchestrator; the next writer appends it here.*

### Rev 6 changelog (owner answers)

Rev 6 is the architect's patch applying the owner's answers of 2026-09-17. It is a patch, not a re-emission. The architect read documents in `[M]` @ `b716a5dc`, code in `[Jw]` (outside `boyko_physics`), `[C]` and `[W]`, and the hazard inventory (`[J]` @ `d552be05`, read by the analyst). The architect ran no shell command.

| # | Change | Reason | Where |
|---|---|---|---|
| 1 | U-12 overturned. Every RF wave becomes step F4. RF prerequisites are removed from D-M1, D-M4, D-M6, D-S1(i), D-S2, D-S3(i), D-S4, D-S6, D-S7, D-E0, D-E1, D-E3 and D-E18. The refactor-worktree priority rule is withdrawn. 02 §5 is rewritten; RF-K1..K3 leave Phase C. | Q-4 | 00 §3, §4; 02 §1–§5; 03 §2, §5, §6; 04 §1 |
| 2 | D-M0's "exception to U-12" is restated as fixed order #6 on schedule grounds: D-M0 does not wait for AP6, and C1 does. | Q-4 removed the rule it was an exception to | 02 Phase C, §4.4 |
| 3 | RK-5 and RK-6 are restated: lock queues on unsplit files; C1 and F4 as the moves that change codegen. | Q-4 | 00 §6 |
| 4 | KC-37 (replay determinism contract, boundary rule B, hazard register H-01..H-19). U-21, U-23 and U-24 added. UG-22 added. MQ-21..MQ-23 added. | Q-9 | 00 §2, §3; 01 §2, §2.1, §3, §6, §7; 03 §1, §2, §5, §7 |
| 5 | Rungs added: A1b (H-03), A9 (H-06), D-E20 (H-02), D-E21 (H-04), RP-0..RP-3. D-E8 gains `every_tick` (H-19). U7 enables UG-22's deferred churn arm (H-16). | Every hazard that breaks Q-9 gets a rung before the gate that would catch it | 02 §2, §3, §4 |
| 6 | U-20 is kept, with its rationale corrected (H-03) and the Q-9 overturn removed. KC-36's "Deletes" is narrowed to hook order across systems. | Q-9; H-04 | 00 §3; 01 §2 |
| 7 | Q-10 (cross-OS replay) and Q-11 (replay from a save) added, with defaults. | Their cost is stated; the scope is the owner's | 00 §7 |
| 8 | Modding narrowed to A′, C and A. B, D, E and F are out. The exact-build contract is 05 §3.3. 05 §8 is answered. U-10's overturn row now cites Q-2. | Q-1, Q-2 | 05; 01 I-4, §5; 00 U-10 |
| 9 | RM-3's knock-ons applied: D-S1(ii)'s scope, tests and empty rename list; UG-15 legs (5) and (7b); controls (vii), (xi), (xii); UG-16; UG-19; MQ-09. RM-2 is placed in D-S1(i). `ModSeam` stays an `unsafe trait`. | Rev 5.1 assigned these to the architect's next patch | 02; 03; 05 §5 |
| 10 | U-22: msvc is the gate host. UG-15's symbol legs read the post-LTO object through the new dev crate `boyko_symcensus`. Sensitivity-map route. UG-08 moves to the msvc nightly. UG-09 recipe. B3 rewritten. | Q-7; `[C]`'s finding that the msvc image has no symbol table | 00; 02 B3; 03 |
| 11 | U-18 is a unification rung. U-19 takes the portable arm only, and the word arm becomes revival form D-M6w (01 §8). D-M6 loses `lane.rs` and tests (a) and (g). Erratum D0-1 is withdrawn. RK-14 is scoped to D-M6w. MQ-13 legs are re-cut. UG-20's fields shrink. | Q-7, and the cost of an arm no gate host would compile | 00; 01 §2, §6, §7, §8; 02 D-M6; 03 |
| 12 | New step AH: merge the census fix, then `chore/msvc-host`; re-spell Miri; redirect `TEMP`; re-bless gnu-pinned numbers with the gnu value beside each. | Q-7; RK-17; RK-18 | 02 Phase A; 04 §2 |
| 13 | RK-7 closed. O1 done. O2 is now a true merge that can meet code conflicts. Fleet rows re-read from `.git`. | Q-5 | 00 §6; 04 |
| 14 | Critic pass 5: W1, W2, W3 resolved; W4 closed (dispositions below). | — | 00 §10 |
| 15 | RK-11 updated (D: about 92 GB). RK-16..RK-18 added. | Owner fact; architect | 00 §6 |
| 16 | Status, provenance, index and all headings moved to rev 6. | — | 00–05 |

**Critic pass 5: dispositions.**

**W1 — resolved.**
- (a) `HeapRef::alloc_cold` is struck from leg (2); DOC-1 carries the strike (00 §5).
- (b) Green control (x) now asserts legs (2) and (2-RF) only. Leg (7) runs only on seam commits, and a seam commit never moves a file. If one ever does, leg (7)(a)'s `.rdata` normalisation for `Location` path literals must be defined first (03 §6).
- (Plausible) Missing-symbol policy: at B3 the pin list is frozen to the symbols present in the post-LTO object, and each absent candidate is recorded with its reason. After capture, a pinned symbol that disappears is RED (P29) (02 B3; 03 §6).

**W2 — resolved.** Each loom arm pins `expected =` oracle text, states a thread count of at most 5, and runs under `LOOM_MAX_PREEMPTIONS=3`. A `--list` check precedes the count check (01 §6 item 9; 03 UG-09, §3).

**W3 — resolved by Q-4.** No RF wave runs before F4. RF-R, RF-V and RF-T also wait for owner step O4, which declares the render/rhi files quiet (02 §5; 04 §3).

**W4 — closed by Q-5.** The owner committed the open work. There is no "drop" option left.

**Critic pass 5: optional items.**
- **O1** (D-M6 test (b)'s prediction): carried open with D-M6.
- **O2** (I-2/I-3 against KC-04's statics): carried open.
- **O3:** withdrawn together with erratum D0-1; it stays attached to the revival text (01 §8.5).
- **O4:** applied. A2 enters the `codes.rs` lock row (02 §4.3). The half about parked device waves is moot until F4.
- **O5:** folded into F4's rebase rule (02 §5).
- **O6:** carried open.
- **O7:** resolved. The symbolizer is the MSVC Build Tools copy at `C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/MSVC/14.44.35207/bin/Hostx64/x64/llvm-symbolizer.exe` (Glob, 2026-09-17). Rustup's msvc `llvm-tools` has no symbolizer; its bin holds `llvm-nm`, `llvm-objdump`, `llvm-readobj`, `llvm-size` and others (Glob of `…/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin`). An absent tool is RED (03 §6).
- **O8:** carried open.
- **O9:** the `AcqRel` nit is applied (01 §6 item 6); the loom-word counting nit is answered (the census counts production `std::thread_local!` only, so `loom::thread_local!` is not counted).

**Critic pass 5: answers to the open questions.**
1. **Disk.** At most four target dirs until F4: the trunk plus `k-1`..`k-3`. Measure `D:/wt/_targets/joltab/release` at B3 (RK-11).
2. **`EXIT_GUARD` reading `WORD`.** On native-TLS hosts (msvc, Linux), a const-initialised, `Drop`-free `WORD` has no destroyed state, so the read is always valid. On windows-gnu, the read depends on std running key destructors in reverse registration order. This is now stated in 01 §6 item 3.
3. **Pins without symbols.** B3 freezes the pin list (W1).

### Open for critic pass 6 (architect)
1. **Rule B's runtime check.** Rule B is enforced statically for declared access, and at runtime by the Main-write check, which sees only hashed state. A Main write to a Fixed-read but unhashed component through `Commands` is seen by neither, only by the boundary report when that component is declared. Is that residual acceptable, or should gate scenes hash every Fixed-read component?
2. **Derived keys.** They take a caller-supplied index (U-24). The alternative is an implicit per-parent counter, which would need a per-entity side table.
3. **Phase-D gate with a deferred arm.** UG-22 closes Phase D with its churn-on-sim-entities arm deferred to U7 (H-16).
4. **D-E20 memory.** Lanes per writer lower memory for types with few writers and raise it for types with more than W writers (MQ-21; UG-20 records it).
5. **Q-10's default.** It gates a property (cross-OS equality) that the product contract does not promise.
6. **The word arm.** Not building it leaves the rustc ≥ 1.98 gnu regression in comparison runs.
7. **Sensitivity map.** Its fallback is a gnu build of the same commit. The map only attributes, so a wrong map fails loud (an unnamed move is red). Confirm.
8. **AH's ordering.** AH runs after A1's commit and before A2's merge, while A2 is in flight in `D:/wt/uploadleak`.

**Items not established (rev 6).**
- Whether `-Cdebuginfo=line-tables-only` puts inline-site records for local functions into the msvc PDB (B3's probe).
- COFF symbol sizes in the post-LTO object (B3's probe).
- Whether Miri supports the AVX2 intrinsics that the joined target key would compile in (B3).
- Whether `/Brepro` makes two msvc builds byte-identical here; the flag is undocumented (§11).
- Whether UCRT's FMA3 and non-FMA3 `sin` differ on the gate scenes' inputs (RP-3 searches).
- The ahead / `.rs` counts of `[M]` after Q-5 (the writer re-reads them). *Writer (§9 V-64): 22 commits / 16 `.rs` files ahead of `d552be05`, merge base `97c504c8`.*
- Whether an uncalled `#[no_mangle]` kernel fn, and red control (viii)'s unreferenced static, survive `/OPT:REF` and fat LTO in the msvc `boyko_demo` image. M-10 measured gnu only. B3's probe answers this as item (v).
- Whether UCRT's `ucrtbase.dll` versions differ between the CI runner and the gate host. UG-22's machine arm records both.

### Critic pass 5 log (final)

Verdict: CHANGES_REQUESTED, no Critical remark. Closed by orchestrator ruling; the remarks below are OPEN.

- W1: Phase B's step B3 cannot record the UG-15 gate's baselines green as written, for two reasons that hold by construction. (a) Leg (2) pins 'the P29 set', and one of its five symbols, HeapRef::alloc_cold (ALLOCATOR-DESIGN-SPACE.md:2616), belongs to the Heap class that ruling U-1 never builds. It has 0 hits in [J]/crates, and P29's own rule says a missing symbol is RED, never a skip (:2618). (b) Green control (x) moves drain_runaway_panic ([J]ecs_master.rs:1192-1200), whose body is a panic! with a Location. The Location embeds the source path, so moving the function to a new module file adds a path literal to .rdata. Leg (7)(a)'s .rdata size then changes, and so does (b)'s (<anon-data>, class, size) multiset, where the control requires both to be identical. — **Resolved in rev 6** (§10, Rev 6 changelog).
- W2: The negative loom arms M1-M4 for the new thread context (KC-04) are bare #[should_panic] (01 §6 item 9; 03 §3), with 'running 5 tests' as the only check that they are not vacuous. The model already uses loom 0.7.2's 5-thread limit (main + 3 threads + a spawned child; rt/mod.rs:62). Loom's own thread-count assert (scheduler.rs:99) and its branch-limit panic (path.rs:118) both satisfy a bare should_panic, so an arm can be green while proving nothing. The repo's own convention requires expected = text for exactly this reason ([J]crates/boyko_threadpool/tests/loom_pool.rs:359-369). — **Resolved in rev 6.**
- W3: After the trunk cut (A8), the owner may keep committing on main, and those commits merge only at the next phase boundary (02 §4.5 :637-639; 04 §2 step 11). Meanwhile the refactor waves RF-R and RF-V split render/rhi files 'in parallel with C-D' (02 §5 :646, :650), with no lock and no owner ruling. RF-V's device.rs and present/targets.rs are files the owner has uncommitted edits in right now ([M] git status). An owner edit to either file during Phases C-D reaches the trunk only at the end of Phase D, after the split, and must then be ported by hand. — **Resolved by Q-4 in rev 6.**
- W4: Owner question Q-5 (00:158) and owner step O1 (04 §3) offer 'commit or drop' for the 59 uncommitted paths. That set includes the only copy of allocator design rev 2-2.4 (00:27-29; ' M docs/memory/ALLOCATOR-DESIGN-SPACE.md') and this plan's own untracked files. Step DOC-1, which edits the allocator file, has no O1 prerequisite. A 'drop' answer would destroy, with no pushed copy, the design that C1, D-S1(i), D-S2 and critique pass AP6 rest on. — **Closed by Q-5.**

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

**Read by the architect for rev 6 (2026-09-17):**
- [Cargo, configuration — `build.rustflags`, `target.<triple>.rustflags`](https://doc.rust-lang.org/cargo/reference/config.html). The four rustflags sources are mutually exclusive, checked in the order `CARGO_ENCODED_RUSTFLAGS`, `RUSTFLAGS`, joined `target.<triple>`/`target.<cfg>` entries, `build.rustflags`. "If several `<cfg>` and `<triple>` entries match the current target, the flags are joined together."
- [Nikhil, "Getting to Deterministic Builds on Windows"](https://nikhilism.com/post/2020/windows-deterministic-builds/) and [rb-general, "Reproducible Builds on Windows" (2024-12)](https://lists.reproducible-builds.org/pipermail/rb-general/2024-December/003592.html). `/Brepro` is an undocumented `link.exe` flag that puts a fixed value in the PE TimeDateStamp. `-C link-args=/Brepro` is how a Rust build passes it, and `/PDBALTPATH:%_PDB%` removes the absolute PDB path.
- [LLVM, `llvm-symbolizer`](https://llvm.org/docs/CommandGuide/llvm-symbolizer.html). It reads PDB (`--pdb`; a native reader, or DIA with `--dia`) and prints inlined frames by default.

**From the rev-6 research briefs (researcher, 2026-09-17; the architect did not re-fetch these):**
- Jolt: [Architecture.md](https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Docs/Architecture.md) (same-binary contract; ~8 % cross-platform mode; `CreateBodyWithID`) and [determinism_check.yml](https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/.github/workflows/determinism_check.yml).
- Box2D: [determinism](https://box2d.org/posts/2024/08/determinism/), [recording.md](https://raw.githubusercontent.com/erincatto/box2d/main/docs/recording.md) (deterministic across thread counts; post-snapshot ids fail validation), and [determinism.h](https://raw.githubusercontent.com/erincatto/box2d/main/shared/determinism.h) (golden re-pinned after solver changes).
- Rust: [RFC 3514](https://rust-lang.github.io/rfcs/3514-float-semantics.html) and the [f32 docs](https://doc.rust-lang.org/std/primitive.f32.html).
- Microsoft: [`_set_FMA3_enable`](https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/get-fma3-enable-set-fma3-enable).
- [CORE-MATH](https://core-math.gitlabpages.inria.fr/).
- [`libm` changelog](https://raw.githubusercontent.com/rust-lang/compiler-builtins/main/libm/CHANGELOG.md).
- Unity: [ECB playback](https://docs.unity3d.com/Packages/com.unity.entities@6.4/manual/systems-entity-command-buffer-playback.html) and [ghostId](https://docs.unity3d.com/Packages/com.unity.netcode@1.1/api/Unity.NetCode.GhostInstance.ghostId.html).
- [Factorio FFF-55](https://factorio.com/blog/post/fff-55).
- [Photon Quantum replay](https://doc-eu-test.photonengine.com/quantum/current/manual/replay).
- [Bevy multi_threaded executor](https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/schedule/executor/multi_threaded.rs) and [Bevy ParallelCommands](https://docs.rs/bevy_ecs/latest/bevy_ecs/system/struct.ParallelCommands.html).
- [boxcars (Rocket League replays)](https://docs.rs/boxcars/latest/boxcars/).
- [Rapier determinism](https://rapier.rs/docs/user_guides/rust/determinism/).

**Read by the architect for rev 6.1 (2026-09-17):**
- [Rust, `std::any::type_name`](https://doc.rust-lang.org/std/any/fn.type_name.html): "multiple types may map to the same type name"; "The output may change between versions of the compiler"; "intended for diagnostic use" (U-26). *Writer (§9 V-72): checked; the page states all three points, with slightly different capitalisation.*
- [Cargo, `cargo test`](https://doc.rust-lang.org/cargo/commands/cargo-test.html): "If the package contains multiple test targets, each target compiles to a special executable … and then is run serially"; within one binary, tests run "in multiple threads" (D-S1(i), D-E21).
- [Rust, `_mm_getcsr`](https://doc.rust-lang.org/core/arch/x86_64/fn._mm_getcsr.html): deprecated since 1.75 in favour of inline assembly, and "Rust makes no guarantees whatsoever about the contents of this register"; only the status bits are unspecified (H-21 reads only the control bits). *Writer (§9 V-72): the page does not name status bits; it says Rust's float operations may or may not update the register's exception state. "Only the status bits" is the architect's reading of that sentence together with the `_mm_setcsr` page below.*
- *(writer-added, §9 V-72)* [Rust, `_mm_setcsr`](https://doc.rust-lang.org/core/arch/x86_64/fn._mm_setcsr.html): changing the masking flags, the rounding mode or the denormals-are-zero flag is "immediate Undefined Behavior", because Rust assumes their default state. This is the source for "Rust makes writing the register UB" (01 §2.1, tick-boundary checks; 02 RP-2).
- [GitHub Actions, store and share data](https://docs.github.com/en/actions/tutorials/store-and-share-data) and [`gh run download`](https://cli.github.com/manual/gh_run_download): artifacts carry files out of a workflow run, and `gh run download <run-id> -n <name>` fetches one (U-27).
- [Bevy, `Time<Fixed>`](https://docs.rs/bevy/latest/bevy/time/struct.Fixed.html): the fixed clock replaces the generic `Time` during `FixedUpdate`, and the schedule "may run 0, 1 or more times" per update. The fixed schedule's view is therefore tick-level, the precedent for U-25.
- [RFC 3127, trim-paths](https://rust-lang.github.io/rfcs/3127-trim-paths.html) and [cargo#5505](https://github.com/rust-lang/cargo/issues/5505): panic paths embed absolute registry paths (U-27; the critic's sources). *Writer (§9 V-72): the RFC's example is a panic message carrying a `.cargo/registry/src/…` path; cargo#5505 (closed) reports `$CARGO_HOME` and `$PWD` paths found inside binaries.*
- *(writer-added, §9 V-72)* [Bruce Dawson, "Floating-Point Determinism" (2013-07-16)](https://randomascii.wordpress.com/2013/07/16/floating-point-determinism/): rogue code in a process that changes the rounding mode makes every result subtly wrong. This is the "Dawson" source H-21 names (01 §2.1).
