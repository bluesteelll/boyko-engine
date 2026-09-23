# Unified system plan — 03 Gates (rev 6)

Tree tags (`[J]`, `[M]`, `[G]`) and the citation verification record are in
[00 Overview](UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md). Bare document names below are `[M]` documents.

## 1. Gate inventory

| ID | Gate | Status | Mechanism | Red-first / anti-vacuity |
|---|---|---|---|---|
| UG-01 | build trio | exists | `cargo check / clippy -D warnings / test --workspace --all-targets --no-fail-fast` (CLAUDE.md) | "green" without `--no-fail-fast` is not green; a `running 0 tests` line is read against its cfg |
| UG-02 | **Ledger gate** (= allocator G1 + engine G-FORM + physics G-form) | **new, B1** | `syn` scanner over non-test fields, statics, typed locals and constructor calls in runtime crates; the scanned set must equal the TSV rows (`[M]docs/memory/runtime-data-ledger.tsv`) whose container is std-heap and whose form is not out of scope, **exactly**; a pinned `ACTIVE_STD_HEAP_ROWS` may only decrease; kernel-storage containers are exempt (M5); per-group floors (P6); physics slice pins `pool.scope` 4 → 0 | P6 seven-shape fixture suite; extra and missing site → red; empty scan dir → red on the floor |
| UG-03 | Per-class allocation census (G2) | exists (`[J]crates/boyko_physics/tests/alloc_frame_census.rs`, `d5782d43`, `harness = false`) | process-global counting allocator; per-class pins (scope / chunk / injector / OTHER / realloc) per scene S0, S0b, S1a–c, S2, S3; ⚠ *2026-09-23 (EP3 W4): plus two engine scenes, which carry the engine design's G-ALLOC (`ENGINE-RUNTIME-ECS-DESIGN.md` P4.1-§12). **E1** is UG-13's headless app (b) (`EnginePlugins` + `UiPlugins::<TestAction>`) over steady frames 10..20; OTHER is the data path, pins only fall, and its anti-vacuity is UI nodes > 0 and mesh instances > 0; it lands in B2. **E1v** is a VisibilityBuffer boot in UG-12's device leg, recording OTHER and the allocations of ≥ 24 KiB per steady frame; its anti-vacuity is the VB path with VB instances > 0; it lands in HO2 as HO2's red-first* | pins are re-derived, never widened (P9); canary "fresh Vec + push in `physics_apply`" reds on OTHER |
| UG-04 | Committed-bytes census (G2b) | new, C1 | `COMMITTED_BYTES[owner]`; steady `commit_delta` pinned 0 for Chunk and Table, Column reported (P31/1); `chunk_bytes_resident` pinned (AL:M-A10). Producers: `Chunk` from D-M2 (`ChunkArena`), `Table` from D-M1 (KC-18 tables as `VmColumn<T, TableOwner>`). Until its producer lands, an owner is reported, not pinned | a commit from a system → red; the setup window must be non-zero for every owner whose producer has landed; a KC-18 table built with the default owner → `Table` setup reads 0 → red |
| UG-05 | `DenyAfterSteady` (G3) | new, D-M3 | opt-in `#[global_allocator]` in gate binaries; abort after `enter_steady()`; `panicking()` exemption; message names the thread role; Miri arm counts instead of aborting | a `Vec::new() + push` in a system must abort |
| UG-06 | clippy type ban (G4) | new, F2 per crate | `disallowed-types` for `alloc::vec::Vec`, `Box`, `String`, `sync::Arc`, `collections::*` when a crate's ledger count is ≤ 10; test allow at the crate root | P7 seven shapes, including `vec![]`, inferred `collect()`, `format!`, and a test-module Vec that must **not** red |
| UG-07 | `no_std` (G5) | new, F2 | `boyko_memory`, `boyko_utils` | compile |
| UG-08 | Miri, two legs | exists; the recipe moves to msvc at AH and is re-established at B3 | **Legs:** SB and TB, with `-Zmiri-strict-provenance`.<br>**Invocation:** `cargo +nightly-x86_64-pc-windows-msvc miri test`, with `RUSTUP_TOOLCHAIN` unset and the full `MIRIFLAGS` copied, plus `RUSTFLAGS=-Zrandomize-layout`.<br>**The ISA baseline is dropped.** `RUSTFLAGS` replaces the configured `[target.<triple>]` flags, because Cargo takes exactly one rustflags source (00 §11). So the Miri build has no `x86-64-v3` baseline, and its `cfg(target_feature = "avx2")` arms are compiled out. The `unsafe` in those arms is covered by the no-FMA censuses and the bit-identity oracles, not by Miri. B3 tries the joined key `--config 'target."cfg(windows)".rustflags=["-Zrandomize-layout"]'` once and records whether Miri runs the AVX2 arms; if it does, UG-08 takes the joined form.<br>**Banned from recipes:** `-Zmiri-retag-fields` and `-Zmiri-unique-is-unique` (P11, `ALLOCATOR-DESIGN-SPACE.md:1181` at `b716a5dc`; `:1174` in the working copy rev 5 read, 00 §9 V-61).<br>**Comparison leg:** `nightly-x86_64-pc-windows-gnu` (miri 2026-08-20), run once at AH and on every Miri red, for diagnosis only. The msvc nightly's miri is 2026-09-09 (owner, 2026-09-17). Receipts name the nightly date | Each Miri row carries a one-line mutation and its expected diagnosis text |
| UG-09 | loom | exists (pool `cfg(loom)` shim; CI job `[Jw].github/workflows/ci.yml:310-370`) | §3 models.<br>**Gate-host recipe:** `cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' test --release -p <crate> --test <file> -- --test-threads=1 --exact <names>`, with `LOOM_MAX_PREEMPTIONS=3` (Linux: `cfg(unix)`).<br>**Why that key.** The `target."cfg(…)"` entry *joins* the configured `[target.<triple>]` flags. `--config 'build.rustflags=…'` is ignored on every tree that carries the AVX2 baseline, so its models compile to nothing and the run prints `running 0 tests` and exits 0 (measured 2026-09-17, `[M].claude/agents/tester.md:220-228`; Cargo's precedence, 00 §11). CI keeps its `RUSTFLAGS` form, which restates the baseline (`ci.yml:314`) | **Before the run,** `-- --list` must print exactly the named models. **The run** must print `running N tests` and `0 filtered out`, with N pinned (`ci.yml:322-369`). Each negative arm pins its `expected =` text (01 §6 item 9). A weakened ordering must red |
| UG-10 | Anchors | exists (`[J]tests/internal_docs_anchors.rs`; 4 gated docs, `:250`) | Anchors are re-derived in the commit that lands a line move on the trunk. For a rung, or an RF branch in F4, that is its merge commit, made in the trunk worktree (02 §4.2); a document step re-derives in its own branch (02 §4.5). On a rung or RF branch the gate runs **scoped**: every red must be an anchor into a file of the branch's touch set, and any other red is a real red | Waived anchors are re-blessed by hand when their file changes. A scoped run whose reds are all in scope is recorded with its list, and the merge must empty that list |
| UG-11 | Ignore census | exists (`[J]tests/ignore_reasons_census.rs`) | + closed reason-prefix vocabulary (B3) | a bare `#[ignore]` → red |
| UG-12 | Device legs | exists (135 tests, owner / orchestrator run) | required for any rung touching render/rhi/app code, including their refactors; goldens `goldens/PINS.toml` | the leg output must report the expected test count |
| UG-13 | G-GRAPH | new, B2 | two headless apps ((a) `EnginePlugins`; (b) + `UiPlugins`) | add an exclusive Main system → red |
| UG-14 | G-LOOP | new, B2 | runner world-write allowlist, 7 → 0 | add a write → red |
| UG-15 | **No-cost / codegen pins** (allocator G6 + modding M-P1 + refactor codegen) | new, B3. Leg (1) gates from B3 and is never re-blessed. Legs (7) and (7b) gate every seam commit from D-S1(ii) on (RP-1 and D-E22 included); leg (7) is captured at B3. Leg (6) gates once a modding crate exists; until then it is reported as N/A, never as green. AP6 ran on P44, and its W1 is answered by leg (7), which sees kernel survivors that leg (6)'s arms cannot | §6 below. Two modes, **strict** and **attributed**, which differ only in legs (2) and (3) | nine red ((i)–(viii), (xi)) and three green ((ix), (x), (xii)) controls (§6) |
| UG-16 | Binary size | new, B2 | `llvm-size` on linked `boyko_demo` (`profile.release`), recorded per rung. Up to +1 % per rung passes. Above that, the rung is held until **the owner** accepts a written reason; no agent may accept it. The delta-0 check against the parent on seam commits is UG-15 leg (7), and the modding-arm delta is leg (6). This gate is the only per-rung record of absolute linked size | add one kernel `pub fn` that `boyko_demo`'s `main` calls → `.text` delta non-zero. This works on both hosts, because a called fn survives `/OPT:REF`; an uncalled `#[no_mangle]` item is B3's probe item (v) |
| UG-17 | trybuild | exists (138 `.stderr`) + new fixtures | path-only re-bless on moves ([G] §3.8) | corpus witness |
| UG-18 | Path-keyed censuses | exist | `scripts/check_hotpath_exceptions.py`, `production_reachability_census`, `gpu_blocking_reader_census`, `code_registry`, `PINS.toml` | pre-existing red recorded (A4a) |
| UG-19 | Registry mint | new, D-S1(i) | G-MINT-1..4, each alone in its own test binary with a relative pin, plus the kind-race row as a deterministic `cfg(test)` rendezvous (D-S1(i); 02 §2); sized-name rows (MS-02b's Stage-3 rung, option A only; RM-3); zero-id assert for cohorts and for `for_type` (D-S2) | G-MINT-1 fails under the rejected `fetch_min` semantics |
| UG-20 | G-RES | new, B2, record-only | Committed kernel-column bytes after boot. For KC-04: `THREAD_CTX_CLAIMS`, `THREAD_CTX_REFUSED`, `THREAD_CTX_PEAK`, `POOL_WORKERS_CLAMPED` and `POOL_RESERVE_DIPS` (01 §6). For events (D-E20): lanes and reader-buffer bytes per event type | — |
| UG-21 | Performance | quiet window only | §5 | receipts carry the idle-machine line and ancestry |
| UG-22 | **Replay determinism** (KC-37) | new, RP-3 | §7 below | ten red controls (r1–r7, r9–r11; r8 withdrawn) and one green control (§7) |

## 2. Gates required per rung

| Gate | Required on |
|---|---|
| UG-01 | every rung |
| UG-02 | every rung from B1 on (a merge that adds rows re-derives them in the same commit) |
| UG-03 | A1, B2, D-M0..M3 (D-M2 and D-M3 add a W = 1 arm to scene S1c, pinned at 0 in the steady window: defect 5's allocation share), D-M5 (a W = 1 physics step constructs 0 scopes: defect 5's serial share), D-S7, every physics rung including D-S2, D-S3(i)–(iii) and D-S4 (physics U1–U3), the RE rungs that touch uploads, F2; ⚠ *2026-09-23 (EP3 W4): B2 lands engine scene E1, and HO2 lands E1v, run in UG-12's device leg* |
| UG-04 | from C1 on: D-M1..M6, D-S2, D-S3(i)–(iii), D-S4..S7 |
| UG-05 | from D-M3 on, in the gate binaries of scenes already at 0 |
| UG-08 | A2, D-E23 (`miri_fixed_loop`), D-M0..M6, D-S3(ii), D-S3(iii), D-S5..S7, D-E9, D-E10, D-E20, D-E21, D-R2a, RP-2, U2-class rungs, K-EK15c, AH (the recipe move) |
| UG-09 | A2, AH, D-M2, D-M3, D-M5, D-M6 |
| UG-10 | every rung that moves lines in gated files; all RF waves (F4); A6, A7 |
| UG-12 | A5 (ddgi / golden), AH, D-E23, A7, A8, RF-R, RF-V, RF-A, RF-RE (F4), D-S2, D-E18, D-E19, HO*, RE*, UI6s, UI8, AS3 |
| UG-15 | Every rung from B3 on, except RF-0, whose gate is 02 §5's snapshot equality. **Legs (1), (4), (5) and (6) hold in both modes and are never re-blessed** (§6). **Leg (7) runs on seam commits only** (05 §6), and those are always strict. **Attributed** applies only to a rung that 02 §2's "UG-15 mode per rung" table lists with named leg-(2) bodies or leg-(3) types; §6's sensitivity map decides which bodies a rung must consider. **Strict** applies to every other rung, every RF commit (F4), D-S1(ii), RP-1, D-E22 and every modding delta. The seam rule is 05 §6 |
| UG-16 | every D and E rung (record). Seam commits (D-S1(ii), RP-1, D-E22, and at Stage 3 the chosen option's kernel halves; MS-01's is option A only, RM-3) and modding deltas: delta 0, enforced as UG-15 leg (7) against the parent |
| UG-17 | A3, D-M5, D-S3(i)–(iii), D-S4, D-E*, RF waves (F4), RP-1, RP-2, U4 (U-17's fixture) |
| UG-19 | D-S1(i), D-S1(ii), D-S2, D-S3(i)–(iii), D-S6 (0 ids), D-E1 |
| UG-22 | RP-3 lands it; from then on every rung runs it, because it is a workspace test inside UG-01; CI runs it on every push. The Windows CI job also uploads the test binary for the machine arm (U-27) |

## 3. Miri rows and loom models

**Miri (both legs).**
- C1: Heap test not applicable (U-1).
- D-M2: C2 slot identity; P3 poison-write protector gate, with the `&Self` receiver mutation
  giving "write access through <TAG> is forbidden" (`ALLOCATOR-DESIGN-SPACE.md:930`).
- D-M5: gang-exit protector gate; abort-path variant.
- D-S3(ii): `GroupSlot` reads of `e2s`; views and `range_mut`. D-S3(iii): `open_chain` concurrent
  with a `GroupSlot` read of `e2s`.
- D-M6: the portable word, std's TLS destructor, the `.bss` statics (a `static` with
  `unsafe impl Sync` over `Cell` records), and the plan-build set. Miri runs all of them,
  including FLS destructors on Windows targets (00 §11).
  - Test 1: a thread claims lazily, sets every record field non-idle (pointer fields to
    `NonNull::dangling()`), and exits without an explicit release. After join, its bit is clear
    (`EXIT_GUARD`'s destructor ran), and a second thread claims the same slot and reads an idle
    record, which decodes to `DETACHED`.
    - Mutation: delete the release's zeroing (01 §6 item 7 step 3, the only site). Expected
      diagnosis: the panic `thread record not idle at claim` from the claim's debug check (Miri
      runs debug builds).
  - Test 2: a hand-written `id_fn` re-enters `get_required_plan` while the outer build has
    published `&PlanBuildSet` in `EcsThreadFields.plan_build`, and the nested build sets and
    clears bits through that shared reference. Expected under SB and TB: no diagnostic, and the
    `Cycle` panic of 02 D-M6 test 4.
    - Mutation: the set becomes `[u64; 8]` passed down the recursion by `&mut`, with its raw
      address published for re-entry. The re-entrant build writes through the published pointer
      while the outer `&mut` is protected. Expected diagnosis: the protector violation's first
      line (SB: "not granting access to tag … strongly protected"; TB: "… is forbidden … protected").
- D-S5: `SpanRef` read across a sibling relocation.
- A2: panic propagation.
- D-R2a: `WorldScratch`. A panic between `take` and `give` drops the column and leaves the stack
  consistent; a nested `take` three deep yields a cold fresh column, with no aliasing.
- D-E9: the teardown form visits each slot before its DEAD fill.

**Loom.**
- `EntityReservoir` claim / settle (A2).
- `ChunkArena` carve race; `dispatcher_claim` claim/release, with a red-first weakening (D-M2).
- Injector: push/steal, steal/steal, overflow-spin with park (D-M3).
- `LaneBoard` ticket and exit (D-M5).
- KC-04 thread context (D-M6, `boyko_threadpool/tests/loom_thread_ctx.rs`). Each iteration uses a
  fresh all-zero local table of 2 slots (the statics' state at process start). Five threads run:
  main, threads 1–3, and thread 3's child, which takes its slot through `claim_for_pool(1)` +
  `adopt`. The bound is `LOOM_MAX_PREEMPTIONS=3`. The model asserts A1 (exclusivity), A2
  (stability) and A3 (exact refusal). Negative arms M1–M4 each pin their `expected =` oracle text
  and state their thread count (01 §6 item 9; critic pass 5, W2). Run with UG-09's recipe:
  `--list` names five tests, then `running 5 tests` and `0 filtered out`.
- Replay (RP-2): none. Writer lanes are single-writer by the scheduler's invariant, which D-E20's
  debug owner assert checks (01 §7).

## 4. Device legs

- Run per binary with `--test-threads=1`, using each binary's environment protocol.
- `BOYKO_DISABLE_VALIDATION=1` where the module header says so.
- The owner runs the goldens; agents never re-bless a golden.
- The legs run on the msvc gate host. Goldens were byte-identical under msvc on 2026-09-10 (owner, 32/32), so AH re-blesses none.

## 5. Measurement queue entries (timings only on the owner's quiet word)

**Build profile of a timing (critic pass 4, W4).**
- **The mismatch.**
  - `[profile.bench]` is `codegen-units = 1`, `lto = false` (`[J]Cargo.toml:114-127`), and the
    workspace itself says such numbers do not describe shipped codegen (`:122-126`;
    `[J]docs/MEASUREMENT-QUEUE.md:44-49`).
  - UG-15's pins are taken on `profile.release`: fat LTO and default codegen units (`:111-112`).
- **The rule.** An entry times under `[profile.bench-shipped]` when its decision is any of: a pin
  move, a codegen-unit partition, an inlining choice, or keep versus re-seam for a rung that UG-15
  attributes.
  - `bench-shipped` is `inherits = "release"` with no other key; B3 adds it.
  - It is run with `cargo bench --profile bench-shipped` (Cargo's documented option, 00 §11).
  - The rule applies to MQ-12, MQ-13, MQ-18 and MQ-20.
- **`profile.bench` numbers** may be recorded beside the deciding column as a variance column, never
  as the deciding one.
- **Receipts** name the profile, the rustc commit hash and the ancestry line (RK-4). Under
  `bench-shipped`, the receipt also gives P6-1's measured run-to-run spread, which is the band.

| MQ | Entry (source) | Arms | Decides | Gates rung |
|---|---|---|---|---|
| MQ-01 | Per-stage pyramid timing (`wf_cc3f9944-c7a`, `D:/wt/stagetime`; physics R0 `PHYSICS-ECS-UNIFICATION-DESIGN.md:748-775`) | W=1 / 8, interleaved | first physics perf rung; allocator P0 overturn (> 1 % / < 0.3 % allocator share) | Phase-E physics order |
| MQ-02 | SP-1, the D2 spike (`:776-783`) | 1-line vs 2-line body record; 1241 / 10k bodies | Q5 overturn | before U4 |
| MQ-03 | G-jolt per physics rung (`:742-746`) | rung vs base | keep or revert. **For D-S2: keep, or re-seam the stagger.** D-S2 changes no loop body, only the cache line each scratch pool starts on, so a regression is answered by MQ-16's stagger re-derivation, not by reverting the rung that the STORE → ENG path is built on | U1 (D-S2: filed, **does not block the merge**) … S0 |
| MQ-04 | KE16 tournament re-take (`[J]docs/MEASUREMENT-QUEUE.md:65-79`) | candidates | pool placement | — |
| MQ-05 | KE17 `split_sim` vs `barrier_sim` (`:83-109`) | model columns only | whether KC-35 is built | F3 |
| MQ-06 | Physics default-off switches (`:113-157`) | per switch | switch defaults | P2 |
| MQ-07 | Row-identity cost R1–R3 (`:171-214`) | `d5782d43` vs fix | keep the interim until U5 | A1 follow-up |
| MQ-08 | Allocator AL:M-A1 (`Schedule::run` floor), AL:M-A2 (`par_iter` chunk source), AL:M-A5 (`EntitySlotMap` growth), AL:M-A6 (pile before/after rung 1), AL:M-A7 (Miri RSS). AL:M-A3 and AL:M-A4 are struck (U-1). The `AL:` and `MD:` prefixes separate the allocator's M-series from the modding design's; M-A1 through M-A4 exist in both | before / after | P0 overturn; U-6 overturn | D-M2, D-M3 |
| MQ-09 | Modding MD:M-K1 (512 → 1024), MD:M-A1 (mod-side monomorphisation), MD:M-A3 (startup cost of the mod-directory scan and manifest parse with 0 mods, `MODDING-DESIGN-SPACE.md:2135`), MD:M-F1 (c) and (d) (timing legs), MD:M-C0b (timing leg), MD:M-C1 (install link time); option D's legs are dropped (Q-1) | as specified (`MODDING-DESIGN-SPACE.md:2129-2146`) | ranking head; id policy; P2's startup budget (MD:M-A3) | M-Stage 2 |
| MQ-10 | Q1 handle-resolution microbench | before / after `Handle` minting | engine Q1 overturn | AS2 |
| MQ-11 | Engine G-PERF: render dispatch ≤ 2 µs, focus hit-test ≤ 5 µs at 2000 rows, gather, layout | — | ED targets | HO4, UI4, RE2, UI3 |
| MQ-12 | Hot-function timing when a split, or C1, moved UG-15 pins. Runs under `bench-shipped` (preamble): with one codegen unit, an in-crate move cannot change partitioning, so a `profile.bench` arm could never answer anything but "keep" | split vs parent | keep the split or re-seam | C1; RF waves (F4) |
| MQ-13 | **KC-04, per host, after D-M6 merges, under `bench-shipped`.** A keep / re-seam entry, as MQ-03 is for D-S2.<br>**Benches:**<br>• `worker/body_1us_tasks_64W` — the ledger's named overturn, `RUNTIME-DATA-LEDGER.md:1627`, defined at `[J]crates/boyko_threadpool/benches/ke16_nested_scope.rs:291`;<br>• `phase14a_hooks_gate` — the structural-op path, which reads the depth twice (`[J]…/hooks/scope.rs:66`, `:74-78`);<br>• `dispatcher/body_1us_tasks_W` — the per-`install` record read (`[J]crates/boyko_threadpool/src/thread_pool.rs:245`, `:252-257`, `:371-377`; 00 §9 V-53).<br>**Arms.** All are built from D-M6's merge commit or its parent.<br>• **msvc (deciding):** parent (native `thread_local!`) / D-M6 portable (committed) / *prototype direct word arm*, built on a never-committed measurement branch from 01 §8 and run only to price U-19 (b).<br>• **windows-gnu (record-only):** parent / D-M6 portable, under `stable-x86_64-pc-windows-gnu`. It records the rustc ≥ 1.98 gnu floor and decides nothing.<br>• **Linux (record-only unless the owner provides a quiet Linux box):** parent / D-M6 portable.<br>**Every receipt carries:** `git merge-base --is-ancestor <D-M6> <tree>` (RK-4), the rustc commit hash, the target triple and the profile | as listed | U-19 overturns (b) and (c). U-18's priority now follows Q-7 and is not decided here. A re-seam is D-M6r, confined to `thread_ctx.rs`, `thread_fields.rs`'s `ecs_fields()` and `tls.rs`'s `lane_deposit()`. Building the word arm is D-M6w | D-M6r / D-M6w only; D-M6 does not wait |
| MQ-14 | Schedule dispatch and build time with system entities | before / after | KF-14 / U-15 overturn | D-E11 |
| MQ-15 | Reparent churn: links vs spans | — | KF-11 overturn | D-E10 |
| MQ-16 | L1d conflict misses on co-iterated same-layout columns: cohorts and same-type `for_type` singles | — | U-2 / P43.1 stagger rule; KC-10's round-robin seed | D-S2 (the remedy for an MQ-03 regression) |
| MQ-17 | Compile time per rung (record-only, U-14) | — | — | — |
| MQ-18 | KC-36 apply-window cost: `boyko_ecs` benches `ke17_apply_window` and `phase9_scheduler`, before and after, under `bench-shipped` (D-E0 runs UG-15 attributed) | before / after | record-only unless the delta leaves the bench band | D-E0 |
| MQ-19 | Reservoir line contention: 8 non-conflicting systems × 1024 `Commands::spawn` per frame at W = 8, with HITM / conflict-miss counts on the `EntityReservoir` line (`[J]…/entity/entity_reservoir.rs:67-90`), against a per-system-lease prototype | shared RMW / lease | U-20 overturn (b): above 2 % of the frame, build leases | none (no rung waits) |
| MQ-20 | `swap_remove/10k` (`[J]crates/boyko_ecs/benches/swap_remove.rs:93-117`), under `bench-shipped`, before and after each rung that names its body: D-M1, D-S3(ii), D-S5, D-S6 (through `EcsMaster`'s layout map), D-S7, D-E2. D-M6's move of that body is priced by MQ-13's `phase14a_hooks_gate` arm | parent / rung | record-only unless the delta leaves the bench band. Outside the band: keep, or re-seam; for D-E2 the re-seam moves the redirect out of line, an inlining decision, which is why the entry runs under the shipped codegen | none (no rung waits) |
| MQ-21 | D-E20 writer lanes: `EventWriter::send` ns and `update_events` ns per type, under `bench-shipped`, at W = 1 and 8; memory per type (UG-20) | parent / D-E20 | U-21 overturn: worker lanes plus a merge at swap by (writer index, per-writer sequence) | none (does not block) |
| MQ-22 | `boyko_math::det` against std: ns per call per function over the known-answer corpus, plus every shipped hot path that calls one | std / det | U-23 overturn, per call site: a table or a reformulation, never std | none |
| MQ-23 | `boyko_replay` per-tick overhead at 10k keyed entities: recording at `hash_interval` 1 and 64; verification (hash, move digest, generation and MXCSR reads); a **spawn-churn arm** (1k derived spawns and despawns per tick, pricing the `DerivedKeyCounters` and `ReplayKeyIndex` probes); W = 1 and 8; record-only | off / record / verify / churn | the default `hash_interval`; U-24's overturn (probe cost); Q-12's alternative (a per-archetype key sort), priced when asked | none |

The MQ-02 and MQ-03 lines are in the physics design; MQ-05..MQ-07 are in `[J]docs/MEASUREMENT-QUEUE.md`.

**Structural checks, not queued** (the tester runs them; they are not load-sensitive):
- AL:M-A9 `commit_delta` by owner;
- AL:M-A10 `chunk_bytes_resident`;
- AL:M-A13 and MD:M-K3 id census;
- P6-1 release reproducibility;
- MD:M-C4 and MD:M-C5 link experiments;
- UG-15 pin captures, the sensitivity-map captures, and B3's `<anon-data>` probe;
- MD:M-F1 (a) and (b), and the size legs of MD:M-F1 (c) and MD:M-C0b (RM-3);
- the RP-0 known-answer table and the sim-math census;
- B3's probe items (i)–(iv).

## 6. UG-15, the no-cost / codegen gate (unifies G6 (a)–(f), M-P1 and the refactor codegen check)

| Leg | Content |
|---|---|
| (1) Source census | **Scope.** A `syn` walk, not a text grep, over every item, including nested items and impl items, in `boyko_ecs`, `boyko_utils`, `boyko_threadpool`, `boyko_log`, `boyko_diag` and `boyko_macros` (the six the modding design names, `MODDING-DESIGN-SPACE.md:1851-1855`), plus `boyko_memory` from C1 on. **Macro output** needs two further mechanisms. `syn` keeps a macro invocation's body as uninterpreted tokens (`syn::Macro::tokens`, 00 §11), so the walk above cannot see an attribute inside a `quote!` template. **(M-a) Template token scan.** Every `quote!` / `quote_spanned!` body in `boyko_macros` is scanned for the counted attribute paths and ABI strings. They are matched as token sequences (`#` `[` … `]`, including `unsafe` `(` … `)`, and `extern` followed by a string literal). This sees literal emission whatever the macro's inputs. **(M-b) Expanded corpus.** This is a test in `boyko_macros`' own lib tests. Every entry point has a `proc_macro2` twin, `fn <entry>_impl(TokenStream2) -> TokenStream2`; the exported `#[proc_macro_*]` fn is a one-line wrapper. B3 lands the twins. The test feeds each twin a fixture corpus with one input for every attribute key its parsers accept. The key list is the parsers' own `const` table, so a key without a fixture is red. Every output is parsed with `syn::parse2::<syn::File>` and run through the same visitor. This sees computed emission, e.g. an attribute built with `format_ident!`. **Anti-vacuity:** every fixture's output must contain its `impl Component`, `Bundle` or event item. **Counted:** (a) an attribute whose path is `no_mangle`, `export_name`, `link_section`, `used` (with any argument) or `linkage`, written plainly or inside Rust 2024's `unsafe(...)`; (b) a fn **definition** with an explicit ABI other than `"Rust"` (any ABI string, including `"system"`, `"C-unwind"`, `"sysv64"` and `"win64"`), or a bare `extern fn`; (c) `global_asm!`, `naked_asm!`, `#[naked]`. **Not counted:** `extern "…" { … }` import blocks and fn-pointer types. **Pass condition:** the count equals the allowlist. The allowlist is a file in the gate crate listing file, item and reason, editable only in an owner-signed commit; at rev 3 it is empty. KC-04 needs no entry (01 §6, item 5) |
| (2) Asm pins | byte size + disassembly, read from the post-LTO object (U-22), of: the P29 set without `HeapRef::alloc_cold`, which U-1 never builds (four symbols; critic pass 5, W1 (a)); `register_new::<Transform>`, `Transform::component_id`, the five dispensers, `try_register_dynamic`, `register_layout`, one `add_system::<F, M>`; and the hot bodies behind `query_ref_iter/10k`, `swap_remove/10k` and the `Schedule::run` dispatch |
| (2-RF) Wave pins | For each refactor commit: byte size and disassembly of every function defined in the split file that survives in the linked `boyko_demo`, plus the wave's entry bodies. RF-P: the `jolt_parity_pyramid` and `colored_solve` bench closures (`[J]crates/boyko_physics/benches/`). RF-R: each split pass's per-frame record entry. RF-A: the runner's frame function. RF-U: the layout entry. RF-RE: the `mesh_draw` and `light_system` systems. Compared after the commit's move map is applied to symbol names, with call and jump targets normalised Leg (2-RF) runs only in F4. |
| (2) Normalisation | **Symbol names.** Before comparison, call and jump targets and RIP-relative data references are replaced by their symbol names. **Anonymous data.** A data reference with no symbol, or with a compiler-generated anonymous name (`anon.*`, `.L*`, `__unnamed_*`, `alloc_*`, `str.*`, or a bare section-plus-offset), prints as `<anon-data>`. Panic `Location`s and string literals embed a file path and a line, so without the placeholder a file move, or a line shift above a release `assert!`, would change a pinned body's text without changing its code. The `swap_remove/10k` body reaches one such assert through `VmColumn::swap_remove` (`[J]crates/boyko_ecs/src/ecs/memory/vm_column.rs:278-286`). B3's probe build settles which of those forms the post-LTO object shows, and green control (ix) holds the placeholder to them. **Residual:** a body that swaps one anonymous constant for another of the same use is not seen; that has no codegen consequence. **Rename lists.** Each remaining name is mapped through the commit's declared rename list: the move map for an RF commit and for C1, and for D-S1(ii) the single entry `TAG_NAMES → DYN_NAMES`. A reference that still differs after mapping is a move |
| (2) Sensitivity map | Captured at B3 beside the pins. **(a) File map.** For each leg-(2) body: the source files of every function inlined into it. They are read with `llvm-symbolizer` over the body's address range, from a `[profile.sensitivity-map]` build (`inherits = "release"`, `debug = "line-tables-only"`, `strip = "none"`; B3 adds it). Two routes are tried in this order, and B3's probe decides between them:<br>• **(a1)** The msvc image with its PDB, read by the MSVC Build Tools copy at `…/MSVC/14.44.35207/bin/Hostx64/x64/llvm-symbolizer.exe`, whose version is printed in the receipt; the tool reads PDB (00 §11).<br>• **(a2)** If (a1) cannot resolve the known inlined frame (`VmColumn::swap_remove` inside the `swap_remove/10k` body), a `stable-x86_64-pc-windows-gnu` build of the same commit, whose image keeps local symbols and DWARF.<br>The map only attributes. A body missing from the map is still red when it moves unnamed, so a map from the other host fails loud, never silent. An absent tool is RED, never a skip (DG6's rule, `[J]docs/diagnostics/substrate/05-LADDER-GATES.md:127`). The keys are set by profile, never through `RUSTFLAGS`, which replaces the configured `-C target-cpu=x86-64-v3` (`[J].cargo/config.toml:84-91`) and would map a binary without its AVX2 arms. The map feeds attribution and is not a pin, so any codegen difference that `debug` causes only changes which frames it lists. **(b) Layout map.** For each leg-(3) type: the leg-(2) bodies that differ between two never-committed scratch builds. In both builds the type is forced to `#[repr(C)]`; the second also gets a leading `[u8; align_of::<T>()]`. Every field alignment divides that prefix, so every field displacement and the size shift by exactly `align_of::<T>()`, whatever rustc's `repr(Rust)` ordering would do. The type's layout asserts are relaxed in those branches only. The map exists because normalisation maps call targets and data symbols but not field displacements. **(c) Containment map.** For each leg-(3) type: the workspace types it holds by value, transitively. It comes from a `syn` walk (leg (1)'s walker) of the census crates and `boyko_memory`. The walk descends through arrays, tuples, `Option`, `Cell`, `UnsafeCell`, `OnceLock`, `ManuallyDrop`, `MaybeUninit`, `CachePadded`, and every generic argument of a workspace type. It stops at references, raw pointers, `NonNull`, `Box`, `Vec`, `Arc`, `Rc`, `PhantomData` and fn pointers. It over-approximates, which only makes attribution conservative. At `d552be05`, `EcsMaster`'s set contains `CommandQueue` (`[J]…/ecs_master.rs:264`) and, through `OnceLock`, `QueryStateCache` (`:296`). **Freshness.** (c) is re-derived at every cut, since it is a source walk. (a) is updated through each RF commit's move map, and fully re-captured after C1, after each RF wave (F4), and after every rung that creates a source file in a census crate (D-S3(ii)'s `binder.rs`; D-M6's `thread_ctx.rs` and `thread_fields.rs`). (b) is re-captured after every rung that changes a leg-(3) type's field list or a type in its containment set. Each re-capture is committed in that rung's merge, so the next cut reads the current map. **Use:** at a rung's cut, per 02 §2's mode-table rule |
| (2) Binaries | the post-LTO objects of the bench targets (`cargo rustc -p boyko-ecs --bench <name> --profile release -- --emit=obj`) and of `boyko_demo` (`cargo rustc -p boyko_demo --bin boyko_demo --profile <leg profile> -- --emit=obj`), built by `boyko_symcensus` with the pairing guard, in target dirs keyed by profile and host (answers P6-7; `[C]:198-200`, `:225-318`) |
| (2) Profile | `profile.release`, if P6-1 shows it reproducible; otherwise the named deterministic profile, which keeps `lto = "fat"`, with `.text` as the shipped-artifact leg |
| (3) `size_of` | `EcsMaster`, `ComponentPool` (128 / 144 at `[J]…/component_pool.rs:57,62`), `Scope` |
| (4) Startup equality | UG-03 setup and steady numbers are equal across leg (6)'s two arms: P44's arm A (the modding crates ~~linked and never called~~ declared as dependencies and never referenced ⚠ *2026-09-23, AP7 W2: P44 defines arm A as declared-only (`ALLOCATOR-DESIGN-SPACE.md:3830`, rev 2.6 P53.2); "linked" was a transcription error*) and arm B (not linked). The configuration with the loader plugin installed is not a leg-(4) arm; its startup delta is P2's budget, measured by MD:M-A3 (MQ-09). On a seam commit made before any modding crate exists, the leg compares the commit with its parent |
| (5) Seam inventory and S-1 shape | `MOD-SEAM` doc markers equal the list in 05 §3, derived by the membership rule (P38.3). **S-1 shape** (RM-3), a `syn` check over the census crates: every item that carries a `MOD-SEAM` marker and adds code is generic over a parameter bounded by `ModSeam`; MS-08's doc-only markers are exempt. No census crate implements `ModSeam` outside `#[cfg(test)]`, and `ModSeam` is re-exported at no crate root. For RP-1, the same shape check applies to `ReplayHash`: every method is generic over its sink, and no census crate instantiates one outside tests. For D-E22, it applies to the access visitor: both methods are generic over the visitor, and no census crate calls them outside tests |
| (6) Linked size, two arms | At the **same commit**, the game binary built with P44's arm A and with its arm B has equal linked section sizes (P44), and `cargo tree -e features` shows no modding feature. **Blind spot:** both arms contain every kernel seam item, so this leg cannot see a kernel-side survivor (`ALLOCATOR-DESIGN-SPACE.md:3832`); leg (7) covers that case. A kernel rung changes both arms equally, so this leg needs no re-bless in either mode. Absolute size per rung is UG-16's, and above its band the owner's. Before any modding crate exists, the two arms are one binary, and the leg is reported N/A, never green |
| (7) Linked seam census | **When it runs:** on every seam commit (05 §6: D-S1(ii), MS-01's rung, and every later kernel half of a modding item), strict, against the commit's parent. **Binary:** the linked `boyko_demo`, built at both commits under leg (2)'s profile rule, with identical flags. **What is compared:** (a) section sizes from `llvm-size -A` (`.text`, `.rdata` / `.rodata`, `.data`, `.bss`, `.pdata` / `.eh_frame`, and the total); (b) the defined-symbol multiset from `llvm-nm --defined-only --demangle`, as (name, binding class, size). It covers every binding, local (`t` / `d` / `b` / `r`) as well as global, because a fat-LTO survivor is local (modding M-10 leg 2, `MODDING-DESIGN-SPACE.md:2147`). Names have the legacy-mangling hash and any `.llvm.N` suffix stripped, and are then mapped through the commit's rename list; (c) the export directory (`llvm-readobj --coff-exports`). A windows-gnu exe has an empty export directory even with a `#[no_mangle]` item (M-10 leg 3), so (b)'s binding class is what sees that case. **Pass:** (a), (b) and (c) are identical. **What it sees that legs (1)–(3) and (6) cannot:** a seam body that survives fat LTO in the game binary with no attribute on it and no pinned caller, e.g. a KC-19b item that `id_space_census()` (reachable from engine code from D-S1(i) on) comes to call. **Host:** the msvc gate host (U-22). `link.exe` writes no COFF symbol table into the image, and its PDB carries publics only, so a fat-LTO local is invisible in both (`[C]crates/profile_fixture/tests/profile_axis_census.rs:46-78`).<br>• Part (b) therefore reads the **post-LTO object** that rustc hands the linker (leg (2)'s binaries row).<br>• Parts (a) and (c) read the linked image.<br>• `/OPT:REF` can only remove code, so an object that lacks a symbol implies an image that lacks it (`[C]:80-84`).<br>• **Anti-vacuity:** `llvm-nm` printing `no symbols`, or printing no symbol line, is RED (`[C]:90-97`), and every leg-(2) pinned symbol must be present in the object.<br>• The linker map (`/MAP`) is a cross-check only: it names local symbols in its "Static symbols" section, but GNU `ld`'s map format differs (`[C]:85-88`, `:129-133`). **Profile:** `[profile.seam-census]` (`inherits = "release"`, `strip = "none"`; B3 adds it). The explicit `strip` matters. When no debuginfo is requested, Cargo ≥ 1.77 sets `strip = "debuginfo"` by itself (cargo PR #13257; disabled on msvc since 1.77.1, 00 §11; kept explicit so that the gnu comparison leg is valid too), and the workspace sets neither key (`[J]Cargo.toml:111-112`). M-10 measured with raw `rustc` and never met that default. The recipe also checks that this profile's `.text` equals `profile.release`'s, so the profile changes symbols only. Anonymous data enters (b) as (`<anon-data>`, class, size). If `profile.release` is not reproducible (P6-1), (a)–(c) are taken on the named deterministic profile, which keeps `lto = "fat"` and `strip = "none"`. `profile.release`'s section delta is then recorded against P6-1's measured run-to-run spread, and a delta outside that spread is red. **History:** this leg restores M-P1's pins (4) and (5) (`MODDING-DESIGN-SPACE.md:2146`), which rev 3 had replaced with a two-arm comparison that cannot see a kernel-side survivor. D-M6 reuses the tool for its `.bss` structural check |
| (7b) Rlib object census | **When:** on every seam commit, together with leg (7) (05 §6; RM-3). **What:** `llvm-nm --defined-only --demangle` over the object members of every census crate's rlib, as built for `boyko_demo` under `[profile.seam-census]`. **Pass:** no defined symbol whose demangled path names a `MOD-SEAM` item; on RP-1, a `ReplayHash` method; on D-E22, `for_each_system` or `for_each_system_access`. Metadata members (`lib.rmeta`) are skipped. **Anti-vacuity:** the count of object members read is reported, and zero is RED |
| Baseline | the **parent of each gated commit** (P6-5 generalised) |
| Modes | **Strict:** every leg is identical to the parent. **Attributed:** legs (1), (4), (5) and (6) hold exactly as their rows state, and no written reason changes that. Leg (7) runs only on seam commits, which are always strict. Leg (2) may move only in the bodies, and leg (3) only in the types, that 02 §2's "UG-15 mode per rung" table names for the rung, each with a written reason in the commit. An unnamed move is red **Seam commits** are D-S1(ii), RP-1 and D-E22 (replay, not modding, but the same additive-generic shape), and each Stage-3 kernel half. |
| Controls | **Red controls.** Each must be red in its own branch, and each names the leg that must go red.<br>(i) A relaxed store in `register_new`'s failing arm → leg (2).<br>(ii) A `pub extern "C" fn` in `boyko_ecs` → leg (1).<br>(iii) `#[unsafe(no_mangle)]` on a private fn in `boyko_threadpool` → leg (1); this proves the 2024 form is matched.<br>(iv) An `extern "system" fn` definition in `boyko_memory` → leg (1); this proves the match is not on the string `"C"`.<br>(v) A `#[used]` static emitted by a `quote!` template of the `Component` derive → leg (1), through M-a and M-b.<br>(vi) The same attribute emitted from an ident built with `format_ident!` outside every template → leg (1), through M-b only; this proves M-b is not redundant with M-a.<br>(vii) `boyko_demo` calls a kernel `pub fn` that no engine path calls → leg (7), both the symbol multiset and `.text`. At B3 the subject is a test-only item added in the control branch; from D-S1(ii) on, it is an MS-03 reader instantiated from `boyko_demo` with a control-branch token (RM-3).<br>(viii) A kernel `static` holds a fn pointer to such an item, with no attribute and no caller → leg (7) red while leg (1) stays green; this proves leg (7) sees what leg (1) cannot.<br>(xi) From D-S1(ii) on: one MS-03 reader with its `ModSeam` parameter removed → leg (7b) red, while leg (7) may stay green; this proves (7b) sees what (7) cannot.<br>~~Leg (6)'s control, a `#[used]` static in a modding crate, runs from the first modding crate on.~~ ⚠ *2026-09-23 (AP7 W2): under P44's declared-only arm A that static never reaches the binary, so this control cannot go red and is not run as a gate. The modding-crate `#[used]`/ctor ban has no mechanism yet and is OPEN for this plan. A candidate (leg (1) over the three modding crates, plus a constructor-section census in leg (7b), each with its red control) is `ALLOCATOR-DESIGN-SPACE.md` rev 2.6, P53.3.*<br><br>**Green controls.** Each must stay green in its own branch.<br>(ix) A comment line inserted above the release `assert!` in `VmColumn::swap_remove` (`vm_column.rs:278-286`), which moves only a panic `Location`'s line → legs (2) and (3) identical.<br>(x) `drain_runaway_panic` (`[J]…/ecs_master.rs:1194`, a `#[cold] #[inline(never)]` call target of the drain at `:719`) moved to a new module of `boyko_ecs`, with the move declared in the rename list → legs (2) and (2-RF) identical. **Leg (7) is not part of (x)** (critic pass 5, W1 (b)): the move adds a path literal to `.rdata`, and leg (7) runs only on seam commits, which never move files. A seam commit that moves a file first defines leg (7)(a)'s normalisation of `Location` path literals.<br>(xii) From D-S1(ii) on: the committed generic readers, with no implementor anywhere in the build → leg (7b) green; this proves the census does not fire on names carried only in metadata.<br><br>**Missing symbols** (critic pass 5, W1). The pin list is frozen at B3 to the symbols present in the object; each absent candidate is recorded with its reason. After capture, a pinned symbol that disappears is RED (P29).<br><br>**When the controls run:** (i)–(x) at B3 (the capture); all twelve on every seam commit from D-S1(ii) on, and on every commit that edits the gate itself. They do not run on other rungs, whose gate code is unchanged |

## 7. UG-22, the replay determinism gate (KC-37)

**Lands** in RP-3.
- **Home:** `crates/boyko_replay/tests/replay_determinism.rs`, so UG-01 runs it on every rung afterwards. It also runs in two CI jobs.
- **Goldens:** the test embeds them with `include_bytes!` and reads no file by path, so the copied binary of the machine arm needs no source tree.

**Harness.**
- Every arm runs in a fresh child process: the test re-executes its own binary per arm with `--ignored --exact <arm>`.
- Arm tests carry `#[ignore = "solo: child process of replay_determinism; run only by the parent"]`.
- **Why:** component ids and the registry are process-global, and the Main-churn arm must mint the r3 pair in its own order.
- The parent asserts that every child printed `running 1 test` and passed.

**Scenes.** Each is a small `App` with a Fixed schedule, run for T = 300 ticks at 64 Hz, with `ReplayPlugin` added first.

- **S-R1, physics pile.**
  - **Spawning.** Two unordered Fixed systems, `spawn_a` and `spawn_b`, spawn bodies through `ReplaySpawner` in the same ticks.
    - Each busy-waits a delay of 0–2 ms drawn from a per-run seed. The seed belongs to the process, is printed in the receipt, and never enters simulation state.
    - Their completion order therefore varies between runs at W ≥ 2 (r1).
  - **Culling.** A system `cull` despawns resting bodies by key parity.
  - **Recycle ticks** (every 50th tick):
    - `cull_one`, ordered before `spawn_a`, despawns one resting body X;
    - `spawn_a` spawns body Y;
    - `marker`, unordered with `spawn_a` and with its own seeded delay, spawns one unkeyed, unhashed entity in the same window;
    - claim order decides whether Y or that entity reclaims X's id (r4);
    - physics runs after both, so the spawn applies before the gather.
  - Sleeping and waking are included.
  - **Hashed:** body pose, velocity, sleep state.
- **S-R2, gameplay.**
  - **Projectiles** use per-entity `rng` streams keyed by `ReplayKey`.
  - **Events.**
    - Two unordered Fixed senders, each with a seeded delay, send `every_tick` events.
    - The reader folds them in arrival order into a hashed resource: `acc = acc.rotate_left(5) ^ payload`.
    - The event type's lane capacity is 64. In ticks where `tick % 16 == 0`, each sender sends 40, and the refused count is folded into the same resource. This is H-02's refusal half.
  - **Hooks.**
    - Two derive-hooked, unhashed types, `HookA` and `HookB`. Their `on_add`, `on_despawn` and `on_remove` hooks append their tag to a hashed `HookLog` component on the entity's parent.
    - No startup code names either type.
    - A Fixed system inserts the bundle `(HookB, HookA)` at tick 10 and despawns the entity at tick 20.
    - The same hooks each call `spawn_derived(parent)` once, so two sites derive keys under one parent in one op (U-24).
  - A hierarchy is included.
- **S-R3, input.**
  - A recorded `TickActions` stream (one registered `ActionState<A>`) and a `SimInputs<Push>` stream whose payload names entities by key (click-to-push).
  - Recorded at W = 8 with random pacing that includes 250 ms bursts, so some presses are seen by several ticks.

**Arms.** Every arm's tier-1 hash sequence must equal the committed golden sequence of its scene.

| Arm | Values | Where |
|---|---|---|
| W | 1, 2 and 8 (`App::with_threads`); each value run 8 times with distinct delay seeds | gate host |
| id perturbation | 0, 1 and 97 unkeyed entities spawned and kept before tick 0; one run despawns 64 pre-spawned entities in descending rather than ascending order, which reorders the recycled stack. No kernel seam is needed | gate host |
| pacing | fixed 1/60 s; seeded random 0–40 ms; 250 ms bursts, i.e. 16 substeps per frame (`update_with_delta`) | gate host |
| Main churn | off, or on. **On:** a Main system that, every frame, spawns and despawns unkeyed entities with new component sets (new archetypes), interns names, and first-touches component types. **At frame 0, before any Fixed tick, it touches `HookB` and then `HookA`**, the reverse of the scene's own mint order | gate host |
| UCRT FMA3 | `_set_FMA3_enable(0)` against the default, through an import block in the test crate only | gate host (msvc) |
| profile | release: the full matrix. dev: the subset W 1/8 × id 0/97 × pacing fixed/burst | gate host; CI |
| machine | two machines, one file (below) | CI job `replay-determinism (windows)` and the gate host |
| OS | CI Linux runner. It gates the engine's scenes against the same golden; the product contract stays per binary unless Q-10 is answered yes | CI job `replay-determinism (linux)` |
| round trip | S-R3 recorded at W = 8 with random pacing, then played at W = 1 with fixed pacing | gate host |

**Two machines, one file (U-27).**
- **CI side.** The Windows CI job builds the release test binary once and runs it.
  - Build command: `cargo test --release -p boyko-replay --test replay_determinism --no-run --message-format=json`, whose output names the executable.
  - It uploads the executable and its sha256 as an artifact.
  - The job pins rustc 1.98.1 (the gate host's version) through its toolchain input. A golden red then names code rather than a toolchain bump. The tree keeps `channel = "stable"` (`[Jw]rust-toolchain.toml:38-39`).
- **Gate-host side.** The gate host downloads that artifact (`gh run download <run-id> -n <name>`, 00 §11) and runs the same file. The tester runs it, since it needs no device.
- **Pass:** both machines print the same sha256, and every hash sequence equals the golden.
- **Anti-vacuity.**
  - Each run prints the CPU brand string (CPUID leaves `0x80000002`–`0x80000004`) and the versions of the loaded `ucrtbase.dll` and `vcruntime140.dll`.
  - Equal brand strings make the leg report "not two machines", never a pass.
  - The DLL versions are recorded, not compared: a different UCRT is part of what "any machine" means.

**Pass.** Every arm equals the golden, and so equals every other arm.

**Profile or OS red for a build-dependent float reason** (H-09, H-10; critic pass 6, O9).
- The product contract is per binary, so such a red is not a Q-9 breach.
- If the fix is free, the rung that owns the site makes the tie explicit: a select that the backend emits identically in both profiles, confirmed by UG-15's pins.
- Otherwise the owner decides whether to demote that arm to record-only, as Q-10 says for the OS leg.

**Anti-vacuity.**
- **S-R1:**
  - at least 200 bodies alive at T and at least 50 despawned;
  - on recycle ticks, the scene counts ticks where Y's id equals X's id, and across the W arm's runs that count takes at least two values. This is r4's precondition, and it holds with or without A1b.
- **S-R2:**
  - at least 100 events read;
  - at least one refused send per run;
  - at least 2 derived keys under one parent in one op.
- **S-R3:** at least 20 inputs applied, and at least one press seen by more than one tick.
- **Every scene:** a non-zero count of hashed components, and 0 NaNs.
- The parent prints `running N tests`, with N pinned.

**Red controls.** Each is red in its own branch, on the arm named.
- **(r1)** KC-36 reverted to pop order → the W and pacing arms (S-R1, S-R2).
- **(r2)** Event lanes keyed by worker id again → the W arm (S-R2), through both the fold order and the refused count.
- **(r3)** Hooks fired in `ComponentId` order, on insert and on despawn → the Main-churn arm (S-R2).
- **(r4)** A1b reverted → the W arm (S-R1).
- **(r5)** `wait_for_fixed` in place of `every_tick` for S-R2's event → the pacing arm.
- **(r6)** A Main system writes a hashed component → the hash check.
- **(r7)** A Fixed system reads `Res<Time>` → the boundary report.
- **(r8)** Withdrawn.
  - Rev 6's r8 made `ReplaySpawner` take its ordinal from iteration order.
  - Under the move clause, iteration order inside Fixed is a function of the simulation, so that ordinal is exactly as deterministic as the simulation.
  - The control could therefore not go red.
- **(r9)** A std `sin` in S-R2's Fixed code → the RP-0 census.
  - Its UCRT half is red only if UCRT's FMA3 and non-FMA3 paths differ on the scene's inputs.
  - RP-3 searches 10⁶ inputs for such a pair. If it finds none, the UCRT half is recorded as N/A, with that number.
- **(r10)** S-R2 gains a catch-up throttle that reads `Time::fixed_steps()` → the boundary report (a Fixed read of `Time`) and the pacing arm (H-20).
- **(r11)** The Main-churn system also inserts an unhashed table marker on one keyed entity per frame → the move digest (U-28, H-16).

**Green control.** (g1) A Main system that churns unkeyed state and ticks a UI animation → every arm green.

**Goldens.**
- One tier-1 sequence per scene, committed.
- A golden changes only in a commit that names the cause, e.g. a solver change (Box2D re-pins its golden in the same way, 00 §11).
- As with render goldens (§4), no agent re-blesses a replay golden without the owner.

**Cost.** Recorded in MQ-23. Wall time is bounded by T, the entity floors and the reduced dev matrix. The child processes add one process start per arm.
