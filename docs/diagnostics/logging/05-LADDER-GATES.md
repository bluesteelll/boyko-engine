# Logging — the ladder and the gates

<!-- CONTRACT
provides: logging/ladder
exports:  logging/gates
assumes:  substrate/loss-fold
assumes:  substrate/section-report
assumes:  substrate/ladder-d0-d1
assumes:  substrate/gates-dg
assumes:  seam/build-axis
assumes:  seam/free-when-off
assumes:  seam/landing-order
assumes:  profiling/ladder
assumes:  logging/budgets-and-invariants
assumes:  logging/emission-path
assumes:  logging/ring-and-statics
assumes:  logging/sink-lifecycle
assumes:  logging/registry-and-walker
assumes:  logging/game-facing-surface
-->

> Carved from `docs/LOGGING-SYSTEM-PLAN.md` (v4) — §Implementation plan and §Metrics and validation, in full. Diff against that document until the monolith is retired. The §Integration inventory (the crate's module list, the migration ledger, the behaviour-change table, Enforcement, Compatibility) is `logging/registry-and-walker`'s; the §Edge cases table is `logging/dispositions`'. Neither is restated here.

---

## Implementation plan

Each rung is independently green (`cargo clippy --workspace --all-targets -- -D warnings` +
`cargo test --workspace` — **`--workspace` is mandatory**: a root `cargo check --all-targets` is
vacuously green here because of the virtual-manifest quirk) and commits alone. L0-L9 are v2's
ladder with the fixes folded; L10-L17 are the scope extension, each purely additive to the rung
before it.

**Cross-plan preconditions and joins** *(seam record §Landing order)*. Three rungs of this ladder
are no longer free-standing:

| This rung | Waits for | Because |
|---|---|---|
| **L0** | `boyko_diag` **D0** (clock, lane, loss, storage policy, `section_report`) and **D1** (`boyko_threadpool → boyko_diag`; `set_lane` at `worker_main` / `install` / `InstallGuard::drop`) | every lane index this crate uses is minted there (S3); `LANE_COUNT` is a plain `boyko_diag` const with **no profile axis** (Q1), and `GLOBAL_CEILING` is re-exported from `boyko_diag`, where L0 adds it as a **hand-written const at the `dev` value** — `boyko_diag/build.rs` is J1's and D0 explicitly does not create it (S9) |
| **L8b** | **profiling rung 7** (the six stdout consumers migrated to the artifact) and **7b** (floor re-measurement) | S1: L8b's 20 measurement rows do not exist because rung 7 already removed their producers. Running L8b first would leave `report!`-shaped work with no macro to do it |
| **L17** | merges with **profiling rung 14** into one joint rung **J1** | S9: one compile axis cannot be split across two rungs, and the 5 CI legs are built once |

Two joint rungs close the ladder: **J1** (= L17 + profiling 14) and **J2**, the joint baseline
sitting. Until J2 lands, neither the profiler's +25 % gate nor this plan's `G10d`/`G12c` revert
clauses may **fail** a rung — they record `UNPROVEN`, because a baseline taken without the other
subsystem present is not a baseline for the both-present configuration (S10).

| # | What | Where | Must NOT move |
|---|---|---|---|
| **L0** | Skeleton; `Level`, `LogTarget`, `TargetId` (private field), `TargetControl` (**packed byte**), `targets!` table, `CONTROL`, `CONTROL_EPOCH_CTR`, the five macros with the 3-gate expansion, `GLOBAL_CEILING` **re-exported from `boyko_diag`** (S9 — no `boyko_log/build.rs`). **No sink, no `emit_impl`.** **Requires `boyko_diag` D0/D1.** **Nothing in this rung runs at process start**: `CONTROL` is `.bss`-zero, no initialiser touches it, and no `boyko_diag` clock call is made — the substrate's no-boot-work obligation (S13) is inherited, not re-argued | `src/{level,target,macros}.rs` — **there is no `control.rs`**, per `logging/ring-and-statics`' Decision 14: `CONTROL` is declared beside the `TargetId` whose closed constructor set is the only reason its `get_unchecked` is sound. This row said `{level,control,target,macros}` and the argued side won; splitting them puts an unchecked index in one file and its justification in another | nothing exists yet |
| **L0-gate** | **G4** three-way side-effect probe: (a) compile-ceiling-below with runtime armed to Trace ⇒ 0; (b) both armed ⇒ **1000**; (c) runtime ceiling `Off` ⇒ 0. Debug *and* release. **v3's leg (d) is GONE from this rung** *(B4)*: it asserted 500 for a quantity that is 1000 (arguments evaluate at step 1, before the sample decision at step 4) **and** it needed `SAMPLE_CTR`, the seed and lanes, none of which exist before L1/L12. It reappears as **G10e** at L12-gate, over the right observable. **G2** separate build leg `BOYKO_PROFILE=off`, all three legs with their mechanisms (Decision 3), **legs (b) and (c) additionally run against a flag-off RUN of a normally-built binary** (S13). **G1 is NOT here** — it moves to L1-gate (F19) | `tests/gates_disabled.rs`, CI leg | — |
| **L1** | `LogLane` (3 partitions + `sampled_out`; **NO inline loss cells** — the substrate's `CELLS[row][class]` is indexed by lane and `record_here` writes the caller's row, so an inline copy is duplication A3 reintroduced one layer up), layout asserts, lane resolution via `boyko_diag::lane()`, retire/reclaim through the substrate, wrap protocol, **the corrected admission arithmetic (F6)**, `LogValue`/`LogArgs`, `dsp!`, `u64` loss accounting (S8), `MAX_RECORD_BYTES` runtime check, `emit_impl` | `src/{lane,record,site}.rs` | L0's gate expansion |
| **L1-gate** | **G1** symbol gate (now that `emit_impl` exists). **G17** Error-reserve arithmetic, **three legs at TWO fill levels** (B3). Per-thread zero-alloc gate, leg (b) now asserting **`== 0`** on a fresh thread (S3); loom model of the cursor pair + `boyko_diag` reclaim; wrap-boundary proptest; **cursor-wrap-at-2³² test (E17)**; overflow test asserting `dropped > 0`; **G3** `.bss` section gate **via `boyko_diag::section_report`** (S12); ~~**G5** distinct-`decode`-symbol upper bound (N31)~~ **G5 is STRUCK — see the L6 decisions below.** The per-site `decode` field it counted does not exist: it could never be filled with a per-tuple monomorphisation (no generic statics) and it could never decode a `.blog` in another process, so L6 replaced it with a tagged payload and **one** walker. A census over a symbol count of one, forever, is a gate that cannot fail | `tests/` | — |
| **L2** | `codes!` (with `prefix`, `status ∈ {Live, Pending, Historical}`), `DiagInfo`, dense `code_idx` + `CODE_IDX_EXHAUSTED`, the three code newtypes, power-of-two `EveryN` assert, per-site `Once` latch + `ONCE_SITES` push, `explain()`. **Registry seeding, itemised**: the **9 measured** grandfathered codes (`B0002`, `B1802`, `B9001`, `B9101`, `B9005`, `B9004`, `B9002`, `B1801`, `W1501`) as `Pending`; the `B9003` gap note; **`docs/diagnostics/B9004.md` and `B9005.md`, which exist in source and in NO document today** (B6); and **18 `92xx` rows as `Pending(<profiling rung>)`** (`W9201`..`W9218`, consecutive; the count is adjudicated in `seam/diagnostic-code-space` and this row previously said 17) so check 4 does not red on the already-committed `docs/PROFILING-SYSTEM-PLAN.md` literals (S6) | `src/codes.rs`, `docs/diagnostics/` | code numbers |
| **L2-gate** | The **eight** registry checks (integration test) over the specified three-stream walker, with **TEXT's explicit directory list excluding `docs/archive/**`** (B6) and the **cross-file `#[cfg(test)]` rule** (B7). Check 2 is `Live`-only (S6); check 3c stays disarmed until L8c. Each check **shown red once** against a deliberately broken registry; the observed failure text recorded in the gate log. **This rung commits alone** because the grandfathered codes are `Pending`, check 3b requires them to have no emitters yet (F20), and the corpus no longer contains codes nothing will ever emit | `tests/code_registry.rs` | — |
| **L3** | `sync_out.rs` (`OUT_LOCK` per Decision 9c, `write_oracle_line` **with the durable fan-out** — B9; **no `report!`** — S1), sink thread with adaptive park, **`DRAIN_OWNER`** (B5), staged drain, console sink → `stderr()`'s own handle, `flush`/`shutdown` with `SINK_STATE`, **`PRE_FLUSH` + `sink_can_accept()`** (S5), panic-hook chaining. Timestamps come from `boyko_diag::clock`; **`RecordHeader.clock_epoch_lo` is chosen here** (S4 left the 4-byte-vs-4-bit choice to this rung; Decision 11 takes the `_pad` byte). **S13's split lands with the mechanisms it moves**: `boot(cfg)` becomes a pure struct-fill that **spawns no thread, installs no hook and calls no `calibrate()`**; `enable(spec)` does all three, and is what a launch flag calls before the game loop | `src/{sync_out,sink/*}.rs` | the 20 B header assert; **`boot()` must stay side-effect-free** |
| **L3-gate** | **G18** `OUT_LOCK` **three-sided** (unwind release; re-entrant completion; **durable fan-out with no console sink**). Flush-without-consumer ⇒ `NoConsumer` immediately; flush-timeout ⇒ within 2 s; shutdown detaches; **`sink_sustained_rate`** finds the drop knee (M19); **S5's four reds** (pre-boot `warn!`, post-shutdown `warn!`, `PRE_FLUSH` ordering, deferred `DiagFlag`); **S7's stderr line-integrity red** — 200 `warn!` while the validation callback fires under `cmd /c … > f 2>&1`, every `[vk-validation] ` occurrence must start a line; give `write_oracle_line` a raw fd ⇒ splices ⇒ red. **S13's boot red**: move the sink-thread spawn or the panic-hook install back into `boot()` ⇒ G2 leg (b)'s OS-thread-count probe and leg (c)'s behavioural hook probe both red on the flag-off run. *(v3's test 16, the `report!` concurrency test, is deleted with `report!` — S1.)* | `tests/`, `benches/` | — |
| **L4** | File sink + cap (`W0103`), rate limiter, `LOG-CENSUS` incl. `UNPROVEN(lossy)`, `SinkMode::Manual`. **`TARGET_STATS` lands here rather than at L16** — the census's rows are per target and cannot be built from process-wide counters (decision 1 below). `W0103` is the registry's **first `Live` row**, which arms checks 2 and 3a | `src/{sink/file,rate,census}.rs`, `src/target.rs` | — |
| **L4-gate** | `Once` steady state performs **no store** (assembly/`perf` check) **and touches no shared line** (per-site latch, F11); census `UNPROVEN` at 0 records **and** at `dropped > 0` | `tests/`, `benches/` | — |
| **L5** | ECS seam: `LogPlugin`, `LogRing` (16 B `LogLine`) on `VmReservation`, `LogStats`, `log_drain_system`, **`ECS_HANDOFF`** (B2), and the **manual `Send`/`Sync` impls with their `const _` pin** (B1). **`LogCensus` is NOT here** — it needs `TARGET_STATS`, which is L16's, so the pin names `LogRing` and `LogStats` at this rung and gains `LogCensus` at that one. The drain has **one** duty here; its other two arrive with L16 | `crates/boyko_ecs/.../log/`, `src/sink/{mod,ecs}.rs` | the `COMMIT_GRANULE` divisibility asserts **and** the `assert_send_sync` pin |
| **L5-gate** | **P1, re-specified twice** (F3 → instrument; S10 → leg matrix): a **headless schedule bench**, not windowed frame time, run as a **2×2 of {logger off, on} × {profiler absent, armed}**, ABBA-counterbalanced, interleaved zero control, **one sitting**. The claim it may make is "logger-on vs off **at a fixed profiler state**", reported at both states. Baselines carry `config_tag = {profiler, logger}`; a sitting whose tag differs returns `NotResolved{ConfigMismatch}` rather than a number | `crates/bench_bevy_vs_boyko/benches/` | — |
| **L6** *(**SHIPPED**, in two commits — see the two decision blocks below)* | Migrate `boyko_ecs` + `boyko_threadpool`; flip those rows `Pending`→`Live`; `W1501`, `B0002` normalisation, `W0701`, `W0501`/`B0502`, `E0201`. **Landed beyond the row**: `E0801` (the asset server's `eprintln!`, which the ledger's file list covers and the row's code list did not name), `PanicCode`'s `Display` — the mechanism that lets a `B` code reach source as an *identifier* — and checks **5** and **6**, which the check table always said arm here. **Landed first, alone**: the record decoder L3 owed, without which every migrated site would have rendered its format literal and discarded its arguments | as tabled | `#[should_panic]` substrings |
| **L7a** *(**SHIPPED**)* | `E2101` and **G7**, the rung's named gate, landed alone because the measurement that re-cut them is the rung's whole design work. Two emission sites, one code, `Once` per site: the escape hatch withheld what the caller asked for, and the extension that carries the sync-validation node is absent. The host-reachability fix landed with it — until then every emitter L5 and L6 wired up wrote into a ring with no consumer in any shipped run | `device.rs`, `tests/g7_validation_reporting.rs`, `boyko_app` | `[vk-validation]` line, byte for byte |
| **L7b** *(**SHIPPED**)* | The remaining **11** production sites: `device.rs` ×3 (`W2102`, ungated in release — the `#[cfg(debug_assertions)]` trio), `present/passes/gbuffer.rs` ×1 (`W2104`, deleting its hand-rolled `AtomicBool` latch), `present/swapchain.rs` ×1 (`W2105`), `present/targets.rs` ×7 — **not one code but two**, `E2103` ×4 and `W2106` ×3, split by what `record_vb` does with each `None` (see the block below). Five doc pages, five observing tests, five REDs. **The messenger is not touched at all** | `device.rs`, `gbuffer.rs`, `swapchain.rs`, `targets.rs`, `log_probe.rs` | same |
| **L7-gate** *(⚠️ **F2's PREMISE IS REFUTED BY THE TREE — measured 2026-08-11, before any L7 code was written.** See the block below the table; the polarity of the first clause is wrong as written and the row is left verbatim so the correction is legible)* | **G7, re-cut two-sided** (F2): `E2101` fires on a validation-**on** run and is absent on a validation-**off** run (`BOYKO_DISABLE_VALIDATION=1`). Channel liveness is proved separately by an **ordinary validation error from a deliberately invalid call** — the historical `mip_levels: 12` on a 512×512 image — with the **baseline of 19 messages accounted for**. A forced *hazard* is explicitly **not** the control: this machine has been measured unable to produce `SYNC-HAZARD` (M25) | `crates/boyko_rhi_vulkan/tests/` | — |
| **L8a** *(**SHIPPED**)* | Migrate `boyko_render` (10 sites / 6 files), `boyko_image` (2), `boyko_serialize` (1), `boyko_physics` (3) — **16 production sites, twelve codes**: `W0901` · `W1301`/`W1302`/`W1303` · `W2201`/`W2202`/`E2203`/`W2204`/`W2205`/`W2206` · `W2601`/`W2602`. Twelve doc pages, twelve observing tests, four REDs. **`boyko_image/Cargo.toml:5`'s description edited in the same commit** — and four MORE stale claims the plan did not name (see the block below) | ledger, `probe.rs` | goldens |
| **L8b** *(**SHIPPED** 2026-08-13)* | Migrate `boyko_app` — **45 code sites over 10 files**, not the 22 the ledger names (its four entries omit `runner.rs`'s 25 entirely and give `gpu_scene/mod.rs` 3 where the walker's own rule measures 1). **Ten new codes**, `E3001`–`E3010`: three terminal exits (`E3002` boot stage / `E3003` device error / `E3004` platform), five degrades (`W3005` SSAA ×2 sites · `W3006` render path · `W3007` VB geometry table · `W3008` unserviceable profiling knob · `W3009` unrecognised env value ×2 sites), one dump-write failure (`E3010`, five sites, `kind` as the argument) and `boyko_demo`'s `E3001`. Thirty sites become `info!` with no code (Decision 7). New engine target `(26, Demo)` — an engine-band row for a downstream crate, which MOVES to `96..=223` when L11a lands. `B1801`/`B1802` flipped `Pending("L8b")` → `Live` (they are **`boyko_ecs`'s**, `app.rs:887`/`:899`, not the host's). `boyko_demo`'s `log`/`env_logger`/`console_log` deleted; `manifest_no_third_party_log.rs` refuses any DIRECT redeclaration, with its RED shown against a real manifest and its scope stated in its own failure text.<br><br>**THREE MEASURED CORRECTIONS, each recorded where it was found.**<br>**(1) The ledger's "nothing measurement-shaped left in `runner.rs`" is FALSE at HEAD.** Rung 7 retired the channel that had *parse contracts*; `VB-CENSUS`/`VB-ZONE` never had one, so nothing carried them and **seven survived with zero readers** (measured across `crates/*/tests` and `scripts/`). They are migrated, not deleted — the artifact carries none of their figures.<br>**(2) A default run cannot see any of this, and that is specified rather than broken.** With `BOYKO_LOG` unset every target is `Off`, so a migrated `warn!`/`error!` emits **nothing** — the macro's third gate folds. L6/L7b/L8a converted **31 unconditional production prints** into records behind that gate and no document says so. In `docs/OPEN-QUESTIONS.md` as a VALUES fork; L8b did not wait on it, because the three terminal codes fall back to `eprintln!` on `flush() == NoConsumer` (`boyko_threadpool::worker`'s blessed precedent), so the host cannot exit silently under any answer.<br>**(3) `boyko_app` never calls `flush()`/`shutdown()`** — SEAM S5's teardown half was never built, and `enable()` DROPS the sink thread's `JoinHandle`, so a record emitted before `return AppExit(true)` races the exit. The three terminal reporters call `flush()` themselves; everything else late in a run is still exposed. Owed to the rung that owns the lifecycle | `crates/boyko_app/src/{diag,runner,plugins,host,host_dump,hzb_dump,vg_census_dump,vb_probe_dump,vb_cull_probe}.rs`, `gpu_scene/mod.rs`, `crates/boyko_demo/`, `crates/boyko_log/src/{codes,target}.rs`, `crates/boyko_log/tests/manifest_no_third_party_log.rs`, 12 new `docs/diagnostics/*.md` | 8 observers in `diag.rs`; `E3001`/`E3004` are `untested_codes.txt` rows — both are compiled OUT on the platform the gates run on (`wasm32` / `not(windows)`), which is a platform statement, not an untested shape |
| **L8c** *(**SHIPPED** 2026-08-13)* | `print_census.rs` walks `crates/*/src/**.rs` and refuses any print outside `print_allowlist.txt`, **checked in both directions** so a stale row reds too, with a third clause requiring every row to carry a reason. It shares ONE walker module with `code_registry.rs` (`tests/walker/`), as this document specifies. Six allowlist rows, each naming the condition that earns it.<br><br>**IT FOUND FIVE PRINTING SOURCES ON ITS FIRST RUN**, from three different causes, after five rungs had each reported a complete migration:<br>**(1) A genuine unmigrated site.** `boyko_rhi/src/handle.rs` — `ResourceRegistry::drop`'s leak tripwire, now `boyko-E2001`. **`boyko_rhi` was named by no migration rung's crate list**, and no reading of this ledger could have shown that; only a walk of the tree could.<br>**(2) The `#[cfg(test)]` region rule was SPECIFIED and never implemented.** This document defines CODE as excluding in-`src` test regions; `split_streams` stripped comments and literals only. Two of the five prints were inside `#[cfg(test)] mod tests`. Implemented at L8c as a separately-named step, `walker::production_code`, on the CODE stream — where brace matching is **sounder** than the `check_hotpath_exceptions.py` precedent it follows, because strings and comments are already gone.<br>**(3) `src/bin/` did not cover `src/main.rs`.** Two `[[bin]]` roots print by design. A walker rule was tried and REVERTED — it swallowed `boyko_demo/src/main.rs`, a bin root that is also its crate's only source file and holds `E3001`'s emitter, reddening checks 3a and 5 together. They take ledger rows instead; the attempt is recorded in `walker/mod.rs` so it is not retried.<br><br>**AND CHECK 5 WAS SATISFIABLE BY PROSE.** It read the RAW TEXT of test files, so a code named in a *comment* counted as a test naming it — found when L8c wrote a comment in `tests/walker/` and check 5 reported `E3001` as *"now named by a test"* when nothing tested it. The corpus now contributes `CODE ∪ LIT` per file (comments out; the frozen-literal route kept). Stripping comments exposed **six** profiling codes, and every one had a REAL test that named a **bare magic number** — `report_count(9204)` — with the doc comment beside it carrying the gate. All converted to constants. Two `untested_codes.txt` rows (`W9203`, `W9216`) were then **refuted by their own tests**: both said the condition could not be reached while `boyko_ecs`'s tests were driving it and asserting on it. `flag_code`'s table test also asserted **four of nine** rows; the five unasserted ones included `W9212` and `W9214`.<br><br>**THE CLIPPY CANARY RAN, AND THE ENTRY IS REJECTED ON A SECOND MEASUREMENT.** This document anticipated one failure mode — the key might be inert, since clippy silently ignores an unresolvable config path. It is **not** inert: on clippy 0.1.97 / rustc 1.97.1 a deliberate `println!` produced *"use of a disallowed macro `std::println`"*. It is rejected because the lint has **no notion of test code** and `--all-targets` compiles all of it — **~1051** sites (697 `tests/`, 66 `benches/`, 60 `src/bin/`, 31 `tools/`, ~197 in-`src` test regions) would each need an `#[allow(..)]`, against **six** production exceptions, destroying the "one grep enumerates every exception" property the discipline exists for. The fallback this document names is therefore right by the **wrong route**, and the route is recorded in `clippy.toml` so nobody re-adds the key blind.<br><br>**CHECK 3c IS ARMED, AND `Pending` IS NOW ZERO.** The four rows it was waiting on — `W9202`, `W9205`, `W9206`, `W9217` — all named profiling rungs 5 and 8, both SHIPPED, and all four conditions were measured present in the tree and **silent**: `alloc_pair` returning `None` past 128 pairs; a teardown that reaches `destroy` without `flush_vb_zone`; a window folding with lost pairs; a contrast refusing. The rows were not waiting for work — the work had shipped without them.<br><br>**HOW is split, and the split is a correctness requirement.** `fold.rs` is the SOLE consumer of `boyko_diag`'s flag word. `W9202` raises a flag, because its site is in `boyko_rhi_vulkan`, which neither depends on the `92xx` emitter's crate nor is depended on by it — the word is the only route — and because it is the only one raised **under load**. The other three CALL THE REPORTER DIRECTLY: a contrast resolved after the run, or a teardown once the frame loop has stopped, would raise a bit **no fold ever takes**. The module stays the sole emitter either way; the rule is about which module emits, not about the word being the only door.<br><br>**AND TWO REPORTERS COULD NOT EMIT AT ALL.** Profiling rung 10 gave `W9210` and `W9212` `flag_code` arms, `Live` rows and doc pages — and did not add either number to `LIVE_CODES`. `claim` resolves a code through that array and returns `false` when there is none, after firing a `debug_assert`: **both were complete-looking emitters that panicked in debug and emitted nothing in release, for three rungs.** Nothing could see it — the orphan check found their identifiers, the page check found their pages, the flag table asserted four of nine rows, and no test drove either condition. It surfaced only because L8c changed the array's length and a pin moved. A new gate, `every_code_the_flag_table_can_produce_has_a_census_slot`, makes the link mechanical, with its RED shown by restoring rung 10's exact defect | `crates/boyko_log/tests/{print_census.rs,walker/,print_allowlist.txt,code_registry.rs,untested_codes.txt}`, `crates/boyko_rhi/`, `crates/boyko_ecs/.../profiling/{diag,plugin,tests}.rs`, `docs/diagnostics/E2001.md` | both census directions RED-demonstrated against a real file and restored; `code_registry` 14/14 |
| **L9** | `boyko_ui` console widget over `LogRing`. **Deferred to the UI plan** — L16 fixes the whole contract it consumes, so nothing logging-shaped remains in it (open question 12) | `crates/boyko_ui/` | — |
| **L10** *(**SHIPPED** 2026-08-17 — A, B and C)* | **Dynamic targets.** **What landed:** `DYN_NAMES` interning (32 × 64 B `.bss`, `MAX_DYN_NAME = 47`), `TargetId::new_dynamic`, `register_dynamic_target`, `find_target`, `targets()`, `dyn_registered()`, and `E0106` **Live with an observer**. **L10-B added:** the five `dyn_*!` macros, `LogSite.target: Option<TargetId>`, the dynamic record's two-byte target prefix, `TargetId::from_dynamic_raw`, and the census listing registered dynamic targets. **What did not:** G8 a–d including the `log_dyn_disabled` bench (L10-C).<br><br>**THE CORPUS SPECIFIES THE SITE, THE HEADER AND THE MACROS, AND NEVER SAYS HOW A RUNTIME TARGET REACHES THE SINK.** `LogSite.target` is compile-time and `RecordHeader` is 20 bytes with a `const` assert pinning it, while a `dyn_*!` site takes its target as an argument and the SAME site may be reached with a different id on every call. Three routes existed; the one taken is `Option<TargetId>` on the site plus a two-byte little-endian prefix on the payload, because a placeholder id is a lie a reader prints (there is no honest value — `TargetId::INVALID` is deleted for exactly this reason) and a header flag bit spends the header's budget on a fact the SITE already knows. Recorded as `01-EMISSION-RING.md`'s FOURTH divergence. Static records grow by nothing.<br><br>**TWO REDs.** Producer stops writing the prefix while the drain still strips two bytes ⇒ the line arrives and its VALUES are eaten (`acme fired <corrupt tag> times`); the record carries a target fixed per SITE instead of per CALL — exactly what a site-carried id gives — caught by the attribution assertion. The second is the property the design exists for, so the test drives ONE call site with TWO ids.<br><br>**AND THE SAFETY COMMENT L10-A WROTE COLLECTED ONE RUNG LATER.** It demanded that any third `TargetId` constructor re-establish the bound and be named there; `from_dynamic_raw` — the only route from a `u16` back to a `TargetId`, validating the band and returning `None` rather than clamping — is that third constructor.<br><br>**A DEFECT FOUND BY READING A RED'S OUTPUT, AND MEASURED BEFORE BEING NAMED.** RED 1 printed a `W`-class line carrying `0106`. The emission macros take `$code:expr` into a `u16` field while the CLASS byte comes from the macro NAME, so nothing pairs them: `warn!(T, codes::E2103.number(), ..)` compiles and prints a `W`-class line carrying `2103`, which `explain(b'W', 2103)` cannot resolve — and every registry check stays green, because all of them key on the IDENTIFIER in source. `codes.rs`'s own test claims this *"is what makes `warn!(…, B1802)` a type error rather than a wrong line in a log"*; the type error exists but is not that — `warn!(…, W1501)` is EQUALLY an error (no code newtype is accepted at all) and `.number()` defeats it in four characters. **Measured: 62 of 62 production invocations pair correctly, so the hole is LATENT, not live.** `dyn_warn!`/`dyn_error!` take the typed newtype so this rung does not widen it; the static macros and their call sites are the next commit, with the mechanical gate.<br><br>**THE SPECIFIED PUBLICATION ORDER CONTAINS A CONTRADICTION**, and it had to be resolved rather than transcribed: the corpus asserts both *"`bytes`/`len` written **before** `hash.store(h, Release)`"* and *"the hash transitions `0 -> h` exactly once **by CAS**"*, which cannot both hold — a writer CAS-ing `hash` has not claimed the slot when it writes the bytes. **The claim moved to `len`**, bytes are written under it, `hash.store(Release)` publishes; the divergence is written at the struct rather than silently reinterpreted. Its consequence is load-bearing: the insert probe walks **`len`**, so a claimed-but-unpublished slot is *occupied* and is **waited on** — a prober that skipped it would mint a SECOND id for a name being written.<br><br>**`None` HAD ONE DOCUMENTED MEANING AND THREE REAL ONES.** `04-GAME-FACING.md`'s signature comment read *"`None` => band exhausted"*; the function refuses a full band, an empty name and a name past 47 bytes. A caller believing the comment would read a rejected 60-byte name as a lost band and stop registering. `E0106` covers all three with the reason as an **argument**, and the test asserts the reason, not just the code. `RatePolicy::Every` for `W0901`'s reason: past exhaustion every later registration fails and each is a different mod, so a latch would name one victim and hide the rest.<br><br>**AND THE FORWARD-DECLARED LEDGER HAD RESERVED TWO CODES FOR ONE CONDITION.** `code_registry.rs`'s ledger carried *`E0105` — "L10 — dynamic target name arena exhausted"* beside `E0106`, but the corpus assigns `E0105` to `flush()`'s 2 s timeout in five places and never to the band. Left alone it was **L8c-C's defect in the ledger next door**: a row reserved for a rung that has shipped, describing work that rung never owed — and *nothing could have caught it*, because that ledger's shrink-only check fires when a listed code is REGISTERED, which `E0105` never would be by the rung it named. `E0104` (also "L10") is likewise L11a's — the ladder scopes L10 to the DYNAMIC band and `E0104` guards the DOWNSTREAM one. `E0108`'s summary was the control spec; the corpus says shutdown-detach. Three rows repaired | `crates/boyko_log/src/{target,codes,site,macros,lane,record,lifecycle,census,lib}.rs`, `crates/boyko_log/tests/{l10_dynamic_targets.rs,code_registry.rs}`, `docs/diagnostics/E0106.md` | static-target expansion byte-for-byte; G1/G4 must still pass unchanged |
| **L10-gate** *(**MET**, with (d) returning NOT MEASURABLE)* | **G8** (a-d). **(a)(b)(c) are met by `l10_dynamic_targets.rs` and are NOT duplicated into a second file**: that test already drives a registered target's records to a real manual file sink and reads the census row under its interned name (a), drives exhaustion and reads `E0106` with its reason off disk (b), and asserts idempotency by name including on a FULL band (c). Writing a second gate over the same conditions would add a file, not a claim. **(d) is `benches/log_gate_cost.rs`**, built this rung — the ladder's FIRST bench — and it returns `NOT MEASURABLE (instrument)`: at sub-nanosecond scale the legs' loop shapes dominate the gates, which the bench proves by measuring a leg FASTER than an empty loop. Two earlier runs reported `RESOLVED` with deltas of OPPOSITE SIGN before the impossibility check existed | `tests/l10_dynamic_targets.rs`, `benches/log_gate_cost.rs` | Decision 2's gate-(a) claim is **UNPROVEN on this box, not struck** — the corpus strikes it when the comparison is MADE and resolves apart finding nothing, and it has not been made |
| **L11a** | **Downstream code tables.** `codes!` exported with `prefix`; `CodeIdx::Dynamic` + lazy minting; `codes_tidy!`; `CODE_OCCUPANCY` + `W0114`; **exhaustion behaviour + `CODE_IDX_EXHAUSTED` + `E0115`** (M3) | `src/codes.rs` | engine `code_idx` remains a compile-time constant; **no mint may ever return an aliased index** |
| **L11b** | **`LogPod`** + `#[derive(LogPod)]` generating **field-by-field `encode_pod`** (B10) + the `*_kv!` field-name forms | `boyko_macros`, `src/site.rs` | Decision 13's structural property (asserted by test 24); **no `copy_nonoverlapping` of `size_of::<Self>()` anywhere in the derive** |
| **L11-gate** | **G9** (incl. the **exhaustion leg**, M3), **G9b** (subject changed to the padded-encode red, B10) | `tests/` | — |
| **L12** | **Sampling.** `SAMPLE_CTR`, the first-touch seed, step 4 of Algorithms A, `sampled_out` plumbing, `W0113`, census `UnprovenSampled` | `src/sample.rs`, `src/lane.rs` | the ≤ 15 ns enabled target — **G10d decides whether this rung ships default-on** |
| **L12-gate** | **G10** (a-e), including **G10e**, the leg relocated from L0 with its observable split (B4), and the perturbation control that can flip `log-sampling` to default-off | `tests/`, `benches/` | — |
| **L13a** | **Volume, part 1.** `Rotation`, `W0112`, `u64` loss accounting end-to-end via `boyko_diag::loss` (S8), `LogStats` u64 accumulation, `LogRing` cursor-wrap hardening **incl. `seq_lo`'s reconstruction rule** (M2) | `src/sink/file.rs`, `src/lane.rs`, ECS seam | `Rotation::NONE` remains the engine default |
| **L13a-gate** | **G11**, subject replaced by S8's fold-exactness red | `tests/` | — |
| **L13b** *(**READER + CADENCE SHIPPED**: `logdec`, `binary::frames`, `docs/LOG-BINARY-FORMAT.md`, and the re-anchor. The cadence bound is `u32::MAX / 2` **ticks** — in ticks so the write path needs no clock scale, and exact against the wire width on every machine. It ALSO re-anchors on a BACKWARDS delta, which the corpus does not mention and the drain makes ordinary: lanes are per thread and walked in index order, so `tsc` is not monotone within a pass. Remaining: rotation and `logdec --merge`.)* | **Volume, part 2.** `BinarySink` with the widths pinned in Decision 21 (M2), the **anchor cadence** (1 s or `u32` overflow), `SITE_DICT` + full-table `W0116` + inline site records, `SINK_OUT`, dictionary records, `logdec`, `docs/LOG-BINARY-FORMAT.md` | `src/sink/binary.rs`, `src/bin/logdec.rs` | text-sink output byte-for-byte; **the audited widths** |
| **L13b-gate** | **G12** (a-c) — **including the revert clause** | `tests/`, `benches/` | — |
| **L14** | **Runtime sink control.** `SinkSlot` state/filter/floor, `SINK_REQ`, `request_open_file`/`request_close`, `E0107`, `ControlSource::File` + `apply_control_spec`, census `UNPROVEN(unsunk)` + `W0111` | `src/sink/request.rs`, `src/control.rs` | no I/O on a caller thread |
| **L14-gate** | **G13** (a-c) | `tests/` | — |
| **L15** | **Crash path.** `CrashSink` opened **on the enable path** (S13 — it was "at boot"; the file is opened when diagnostics are turned on, which is still before the first frame and still not inside the panic hook), `SINK_STATE::Exiting`, the panic-hook protocol **with step 1.5 (`PRE_FLUSH`)**, the `DRAIN_OWNER` claim (B5), `E0109`, `E0118`. **`SinkMode::Scheduled`** and its `DRAIN_OWNER` participation (B8) | `src/sink/crash.rs`, `src/sink/mod.rs` | Decision 12's flush semantics; no new unbounded wait; **`SINK_STATE` must NOT regain an exclusivity role**; **the open must NOT move into the hook** |
| **L15-gate** | **G14**, **three-sided** — the third leg panics while a **manual `drain()` is in flight** (B5) | `tests/` | — |
| **L16** | **Game consumption.** ~~`TARGET_STATS`~~ *(landed at L4 — see that row)*, `LogCensus`, `DiagCensus`, `LogRing::since` + `RingFilter` + `skipped`, the per-frame **`frame_epoch`** record (S11 rename), `boyko_diag::SessionId` in every header, the `ONCE_SITES` census walk (M1) | `src/target.rs`, `crates/boyko_ecs/.../log/` | the drain stays off the frame thread **except under `Scheduled`, where it is on it by design** |
| **L16-gate** | **G15**, two-sided, plus the `ECS_HANDOFF` overflow leg (`W0117` fires, `lossy` set, no silent loss) | `crates/boyko_app/tests/` | — |
| **L17 → J1** *(**COMPLETE.** The axis landed at profiling rung 14; `LogRuntimePreset`, the three header facts and G16(d) land here. `boot_preset(preset, text, binary)` applies the WHOLE row — paths, rotation, sinks, and the target arming without which a preset opens a file that nothing reaches — and `enable` prints the header. **Three things had no caller at all**: `enable` never opened the binary sink (`LogConfig` had no field for it, so no shipped configuration could produce a `.blog`), `header` was called only by its own test, and `rotates` by nothing — which its own doc had warned about in advance. **And `enable` opened its sinks BEFORE `clock::calibrate`**, so the `.blog` anchor stamped the uncalibrated `1.0` and `logdec` read a record 0.2 ms after open as `+85.215ms`; reverting that fix left every test green, because the only other test writing a `.blog` opens the sink by hand after `enable` has already calibrated. The ordering now has its own assertion, against the LIVE `ticks_per_ns` rather than against `1.0` — that literal is a fact about x86-64, not a correctness claim.)* | **Merged with profiling rung 14 into ONE joint rung** (S9): the single `BOYKO_PROFILE` axis, `LogRuntimePreset`, the three header facts, and **5 CI legs** (`dev` existing + 4 net new). One axis cannot be split across two rungs. **What landed:** `crates/boyko_diag/build.rs`, `LOG_CEILING` on the axis at all five rows (`5/4/3/2/0`), the 4 net-new CI legs, the `compile_error!`, and `G16` (a)(b)(c). **What did NOT, and why it could not:** `LogRuntimePreset` and the three header facts (`build_profile` / `runtime_preset` / `ceiling` printed as three independent values, with a fixture proving the first two can differ in one binary) are `G16(d)`, and they need a sink header to print into — **L13b–L16 have not landed**. Measured at rung 14: `boyko_log` has `census`, `codes`, `drain_owner`, `lane`, `level`, `lifecycle`, `macros`, `rate`, `record`, `site`, `sync_out`, `target` and `sink/{ecs,file,mod}`; it has **no** `sample.rs`, `sink/binary.rs`, `sink/request.rs`, `sink/crash.rs` or `bin/logdec.rs`. **The axis was landed alone precisely because S9 forbids splitting it** — the *axis* is indivisible and it is now whole; `LogRuntimePreset` is not part of the axis, it is the runtime counterpart the axis is deliberately kept separate from | `crates/boyko_diag/build.rs`, `crates/profile_fixture_log/`, `.github/workflows/ci.yml` | G2's `off` leg must still pass unchanged |
| **J1-gate** | **G16** two-sided symbol gate + the `compile_error!` red + the three-header-fields red (Decision 25) + **P2** soak. G14/G16 cross-profile symbol censuses are CI **steps** over two legs' artifacts, not extra legs | CI legs, `tests/` | — |
| **J2** | **The joint baseline sitting** (S10). Re-take `P1`, `P2`, `log_*` and the profiler's `zone_cost` **in the both-present configuration, in one sitting**, and stamp every baseline file with `config_tag = {profiler, logger}`. **`GJ1`, the measured off-cost with its control leg (S13), runs in this sitting** — a flag-off number taken without the other subsystem present is not a number about the both-present configuration. Until this rung lands, no revert clause may fail a rung — they record `UNPROVEN` | `benches/`, baselines | — |

### Three L0 decisions taken at implementation, recorded rather than absorbed

1. **Every engine target's `STATIC_CEILING` is `Level::Trace`.** The illustrative rows in
   `logging/emission-path` show `Ecs = Info`, `Schedule = Info`, `Threadpool = Warn`.
   `STATIC_CEILING` is the **compile** ceiling, so `Ecs = Info` means no `debug!(Ecs, …)` can exist
   in **any** build, forever, and un-guessing it is a source edit rather than a config change.
   Before a single call site exists there is no evidence for lowering any target, and the two axes
   that should be doing this work already are: `GLOBAL_CEILING` deletes `debug!`/`trace!` from
   every shipping build, and the runtime byte turns targets off in a dev one. A per-target compile
   ceiling is for a target **measured** noisy even in dev — a fact this rung cannot have.

2. **The table has 26 rows, one per domain in `logging/code-registry`'s block map**, ids dense and
   in block order. Deriving them from the block map rather than inventing a list means a reader
   who knows a code's block knows its target without a second table to consult. Ids are never
   renumbered; a domain that later splits takes the next free id.

3. **`boyko_diag` hands over a `u8`, not a `Level`.** `substrate/dedup-rationale` §4 explicitly
   does **not** own "`LogTarget` / `CONTROL` / the level model", so the bottom crate must not name
   this crate's enum. `boyko_diag::profile::LOG_CEILING` is therefore a plain `u8` and this crate
   maps it: `pub const GLOBAL_CEILING: Level = Level::from_raw(boyko_diag::profile::LOG_CEILING)`.
   That is a `const fn` over a `const`, so gate (b) still folds and N27's property is untouched.
   It also puts the range check where it belongs: `from_raw` `panic!`s outside `0..=5`, and a
   `panic!` in a `const` initialiser is a **compile error**, so a bad generated ceiling fails the
   build rather than shipping a wrong one.

   The module is landed in `boyko_diag` **now**, hand-written at the `dev` row, rather than in
   `boyko_log` to be relocated at J1 — J1 then swaps a module *body* instead of moving a public
   const between crates. It declares **only** `LOG_CEILING`: `GLOBAL_TIER`, `REGION_CAPACITY`,
   `ENGINE_ZONE_SLOTS`, `MAX_USER_BUDGET` and `DYN_NAME_BYTES` arrive with the rungs that read
   them, because a constant nothing reads is a value nothing can prove wrong.

### L2 lands in TWO commits, and three findings from its first half

**The split.** L2's row is "the registry **and** the eight mechanical checks". The registry is a
table plus a macro; the checks are a source walker with a cross-file `#[cfg(test)]` pre-pass, three
disjoint streams and eight red states to demonstrate. Landing them together means one commit in
which the walker's own defects and the registry's are indistinguishable. **L2a** is the registry,
green and alone; **L2b** is the walker and its checks. The rung's "commits alone" property is about
not needing a *migration* rung beside it, and it is unaffected.

1. **All nine grandfathered codes are `Pending`, including `B9004` and `B9005`.**
   `logging/registry-and-walker` says of those two that "both are `Live` from L2 (they have
   emitters today), so both pages land **at L2**". That contradicts its own check 3: a `Live` row
   requires the **identifier** `codes::B9004` to appear in CODE, and today's occurrences are string
   literals and doc comments — the identifiers arrive at L6. A `Live` row here would red a correct
   registry. The check semantics win; the two pages are written anyway, since a page for a
   `Pending` row is permitted and the debt was already identified.

2. **Three registry summaries were invented and had to be corrected against the tree.** The first
   draft described `B0002` as "a system parameter the world cannot supply" (it is an *intra-system
   access conflict on one resource*), `B9002` as "conflicting access to one resource" (it is a cycle
   in the **set** hierarchy) and `B9004` as "a set that was never registered" (it is *two ordered
   sets sharing a member*). Every summary is now read out of the message the engine actually
   prints. **Nothing downstream would have caught this** — a summary that disagrees with the panic
   text is wrong in the one place a reader looks after seeing the code.

3. **The contract gate had to learn that `docs/diagnostics/` holds two kinds of document.** Adding
   `B9004.md` reddened it: code pages are user-facing reference and participate in no capability
   graph, but the walker took every `*.md` under the directory as design corpus. Excluded by
   **exact filename shape** (`^[BEW][0-9]{4}\.md$`, top level only), never by a glob — the same rule
   the registry's own page check uses, and for the same reason: a shape test cannot be widened by
   accident. The corpus anticipated this collision for the registry check; it arrived at the
   contract gate first, because that gate was armed earlier.

### Four L1 decisions taken at implementation

1. **`LogArgs::args_flags()` is added.** The spec requires `STR_TRUNCATED` on the record header and
   gives no mechanism: the header is written **before** `encode` runs, so a flag discovered during
   encoding arrives too late. `args_flags()` is a second, pure pass over the arguments, run beside
   `encoded_len()` and constant-folded to `0` for every all-POD tuple. Without it the flag could
   only have been set by rewriting the header after the payload — a second write into a record the
   consumer may already be walking.

2. **`&str` truncation cuts on a CHARACTER boundary.** The spec says "capped at
   `MAX_STR_BYTES = 256`". A byte-wise cut lands inside a multi-byte codepoint one time in ~three
   and hands the sink invalid UTF-8 — corruption introduced by the logger, on the path that exists
   to report corruption. `str::floor_char_boundary` is unstable, so the walk is in
   `record.rs::str_fit` and is tested against a `€` straddling the cap.

3. **`code` lives in the per-site `static`, not in the argument tuple.** It is a constant by
   construction (the registry mints constants), so carrying it as an argument would evaluate and
   encode it on every call for a value the sink can already read through the site pointer.

4. **`LogSite::decode` is a single non-generic `decode_opaque` at this rung, and gate G5 moves with
   the real decoder.** Two reasons, and the second is a language constraint rather than a choice:
   rendering means interleaving values with the format literal's placeholders, which is the
   **sink's** policy and the sink does not exist; and the per-argument-tuple instantiation cannot
   be named at the `static`, because the site is `&'static` and **Rust has no generic statics**.
   G5 counts distinct `decode` symbols — at this rung there is one by construction, so the census
   could not go red and would be a gate that cannot fail. It lands with the sink, which is also
   the first rung at which it can mean anything.

   > ⚠️ **SUPERSEDED AT L6, and the second reason above is exactly why.** "It lands with the sink"
   > was wrong, because the language constraint this decision names **does not expire**: the tuple
   > type cannot be named at a `static` at L3 either. L3 landed the sink; the decoder did not
   > arrive with it, and nothing noticed for three rungs — **no drain path ever called `decode`**,
   > and every sink printed `site.fmt` with its placeholders intact. L6 replaced the field with a
   > tagged payload and one walker; **G5 is struck**, not relocated. Full record below.

### The L6 decoder, which was L3's and was measured missing at L6

**MEASURED, at the opening of L6.** `site.decode` was called from **nowhere** in the workspace
(`grep` over `crates/` and `src/`: `LogFormatter` was constructed only inside `site.rs`, and every
drain path rendered `LEVEL file:line <fmt> (N B)`). So a `warn!(Schedule, W1501, "…set '{}'…",
name)` would have rendered the literal `{}` and thrown `name` away — after transporting it across
the ring. **L6's whole content is call sites whose value IS their arguments**, so this was not a
gap the rung could route around, and it is the reason L6 lands in two commits.

**The replacement is a tagged payload, and two facts decided it rather than a preference:**

1. A per-site `fn` pointer **cannot be installed**. Decision 4 above says why and the reason is
   permanent. The only mechanism that would work is publishing it at run time into a mutable site
   — atomics plus a transmuted `fn` pointer on the emission path, to carry data the emitting
   thread is specified never to touch.
2. A per-site `fn` pointer **cannot decode a file**. `logdec` (L13b) reads a `.blog` written by
   another process, where a pointer into the producing binary means nothing. That rung would have
   had to introduce tags or a shape table anyway, so the fn-pointer design was never going to
   survive it.

One tag byte per value (`record::ValueTag`, discriminants **pinned as a wire format**) costs a
store adjacent to the value it describes, serves both consumers, and leaves `LogSite` a pure
immutable `'static`. `record::render_payload` is the one walker: `{}` consumes the next value,
`{{`/`}}` are literal braces, and **any other `{…}` group also consumes the next value with its
format spec ignored** — a real limitation, written down rather than discovered at a call site.
Neither direction of disagreement is silent: a placeholder with no value renders `{missing}`,
leftover values are appended as `[+ …]`, a corrupt tag or a truncated value **ends the walk**
rather than resuming mid-value and rendering ring bytes as data.

**Two things this rung's own RED found, in the order it found them.** Reverting `render_payload`
to "write `fmt`, return" reddened four `record` tests — and left `lane`'s green. That test asserted
`s.contains("probe")` against the site literal `"probe {}"`, which survives the broken renderer:
**a vacuous assertion in the very test written to prove the fix**. It now asserts the whole line,
`"lane.rs:0 probe 7"`, and the RED reddens five. The second is that the **un-laned** `Warn`/`Error`
fallback had the same defect independently — it rendered site metadata and the literal — so a
severe record on a driver or OS callback thread lost precisely the values it existed to report,
on the path where the engine has the least other information. Both paths now render arguments.

### L7 opened with a measurement that refutes F2, and G7's polarity does not survive it

**Measured at HEAD, before a line of L7 was written**, because L7's row cites line numbers that have
drifted (`device.rs:2110` / `3100,3158,3189`) and a rung that migrates a crate has to re-derive its
own site list.

**1. The site census, re-derived.** 30 print matches under `crates/boyko_rhi_vulkan/src`:
**16** in `compute/tests.rs` (test-only, keep — the corpus's own count, reproduced exactly), **1**
in `debug.rs:114` (the messenger, **NOT TOUCHED AT ALL**), **1** at `device.rs:3788` inside a
*within-file* `#[cfg(test)] mod tests` opening at `:3732`, and **13 production sites**:
`device.rs` ×3 (the `#[cfg(debug_assertions)]` degradation trio — `W2102`'s subjects, still the
same three by content: DDGI storage, shadow-denoise RG16, SSAO R16), `present/passes/gbuffer.rs` ×1
(a hand-rolled `AtomicBool` latch, the shape the ledger deletes elsewhere), `present/swapchain.rs`
×1 (present-mode fallback), `present/targets.rs` ×7 (**the ledger's count, exact**) — all seven the
same degrade-to-`None` build failure.

> **The ledger's denominator misses within-file `#[cfg(test)] mod tests` blocks.** Its test-only
> class is "16 within-file + 7 cross-file = 23", which counts whole FILES; `device.rs:3788` is a
> test's SKIP line inside a production file and is in neither class. The walker's CODE stream does
> not strip within-file `#[cfg(test)]` regions either, so **L8c's "zero unclassified sites" cannot
> be reached without a rule for them.** Recorded here rather than fixed at L7: it is L8c's gate.

**2. `E2101`'s condition, and the premise that failed.** F2 says "a *chained* validation-features
node is unbuildable here". **It is built and it works.** `create_instance` enables
`VK_EXT_validation_features` when present and chains `VkValidationFeaturesEXT`
(`VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT`) as the head of the instance
`p_next`, with the create-time messenger behind it. Measured by running it: with
`BOYKO_DISABLE_VALIDATION` **unset** and `enable_validation: true`,
`cargo test -p boyko_rhi_vulkan --test compute` boots and passes **4 of 4** (`test result: ok`).
The standing environment note that the validation layer crashes this MinGW process is therefore
**not true of the headless compute path** — whatever it describes, it is narrower than "validation
cannot run here".

**3. So G7's first clause is backwards.** "`E2101` fires on a validation-**on** run" holds only if
the node can never be chained. It can, so on a correct box a validation-on run must be **silent**,
and a gate asserting the opposite would be red against a working engine.

**The decision, taken here with the measurement rather than deferred**: `E2101` means *validation
was requested and this process is not getting it* — `enable_validation` was true in the config the
caller wrote, **and** either the `BOYKO_DISABLE_VALIDATION` escape hatch took it away or
`VK_EXT_validation_features` is absent. Both arms say one thing to a reader: **this run's validation
is weaker than the caller asked for, so a clean run is not a proof.** That is the condition this
repository has been burned by twice and which stands in `docs/OPEN-QUESTIONS.md` from 2026-08-06 —
every golden runs under `BOYKO_DISABLE_VALIDATION=1`, and until now nothing said so.

G7 becomes two-sided **and runnable on this box**, which the specified polarity was not:
**positive** — `enable_validation: true` with the escape hatch set ⇒ `E2101` fires; **negative** —
escape hatch unset ⇒ absent, and the context reports sync-validation live. What it still **cannot**
claim is that a live layer *catches* anything: `compute.rs`'s own `negative_chained_barrier_hazard`
documents, in the tree, that sync-validation is enabled and does **not** flag a compute→compute RAW
hazard on this path. `M25` stands; the instrument's presence and its sensitivity are two questions
and only the first is gateable.

---

### L8a: a count-based observer cannot see a payload, and four other things the rung measured

**Sixteen production sites, twelve codes, four crates.** The census that drove it is the walker's
rule, not the raw grep: `boyko_render` shows 15 grep hits across six files and **10** of them are
calls (five are prose in doc comments, which the ledger's own "prose mentions" row excludes);
`boyko_serialize`'s "(2)" is one call and one doc mention; `boyko_image`'s 2 and `boyko_physics`'s 3
are exact. One of `boyko_render`'s ten is the workspace's only non-`bin` `println!`, and it became
an `info!` with no code, by Decision 7.

**1. The observer this campaign has been using could not gate its own rung's claim.**

The migration ledger's row for `light_system.rs` promised "the dropped count is now reported, which
the one-shot latch never did". The implementation delivers it — the fold tallies drops and reports
once at the end instead of calling a `#[cold]` reporter per dropped light. But when the fix was
**reverted to prove the gate**, every assertion stayed green: the observer counted *records*, and
"one record saying three" and "one record saying one" are both one record. The claim is about the
**payload**, and nothing in the probe could see a payload.

`boyko_log::probe` now renders the record's arguments at the emission site and keeps the text
per-thread, so an observer can assert on the message. Re-running the same revert then produced

```text
the record must carry the tally, not merely exist: dropped 1 point/spot light(s) ...
```

Every value-carrying record this campaign has migrated since L6-A was, until this rung, gated only
by its own existence. **This is the second time L6-A's payload work turned out to be ungated at the
call site** — the first was L6 itself, where the renderer printed the format literal and threw every
argument away.

**2. A record observer needs three independent things, and each fixes a different failure.** Written
into `probe.rs`'s header because it cost three separate reddenings:

| Failure | Fix | What it looked like |
|---|---|---|
| another emitter's records inflate the count | count **per thread, per code** at emission | `left: 7, right: 1`, nondeterministic |
| an EARLIER test already spent the `Once` latch | `OnceSite::reset` before the emission | `left: 0, right: 1`, every run |
| a CONCURRENT test spends it mid-window | `observe_lock`, taken by every test that *drives* the site | `left: 0, right: 1`, one run in ten |

L7b's mechanism — a process-global delivery counter under a lock — was sound **in
`boyko_rhi_vulkan`**, whose only emitters are that rung's own tests. It does not survive contact
with `boyko_render`: 468 lib tests, many of which legitimately fold a NaN light or read a diverged
frozen config. Locking them one at a time would have worked until the next test anyone wrote. So
the *observable* changed rather than the world.

**3. `RatePolicy` is declared and never applied, and now something says so.** Measured by reading
the expansion: `warn!`/`error!` gate on the three ceilings and call `emit_impl`; **neither reaches
`rate::admit`**, which still has no production caller. Every `Once` in this registry is honoured by
a hand-placed `OnceSite`; every `Every` is honest because nothing damps it. A row declaring
`EveryN` or `MinIntervalMs` would be a promise with no machinery — so `E2203`, whose site genuinely
wants per-second damping, declares `Every` and says in its own comment that the ring is what bounds
the flood. `codes.rs` gained
`no_live_row_declares_a_policy_the_emission_path_cannot_honour`, shown red by declaring
`MinIntervalMs(1000)` on `E2203`. It proves what it can and its failure text says what it cannot:
**it does not prove that a declared `Once` has an `OnceSite` behind it.**

> **SUPERSEDED — `rate::admit` IS WIRED.** The gate above is **deleted**, because its assertion
> would now forbid the capability that exists. `warn!`/`error!`, their `_kv!` forms and
> `dyn_warn!`/`dyn_error!` gained a **fourth gate**, `__log_rate_admits!`, placed LAST in the `&&`
> chain: a rate RMW spent ahead of the ceilings would let a silenced logger keep advancing a
> code's phase. The policy is bound into a **`const`** at the call site — the three code newtypes
> now CARRY their row's `rate`, written from the same token as the registry column — so the four
> arms a site does not declare are deleted rather than branched over and an `Every` row still
> costs exactly nothing. Only `EveryN` and `MinIntervalMs` reach `rate::admit`, and only they pay
> for `code_idx`.
>
> **`Once`/`OnceCounted` deliberately still do NOT get a macro-placed latch**, and that is the one
> thing the wiring refuses. A `static` inside a macro expansion **cannot be named**, and
> `OnceSite::reset` exists precisely so an observer can reset the latch it is about to test —
> auto-latching would buy redundancy at the price of making all 45 `Once` rows untestable in
> isolation. So the human link the deleted gate disclaimed is unchanged, and it is now the ONLY
> one: see the `Once`-without-a-latch finding recorded in `docs/OPEN-QUESTIONS.md`.
>
> `tests/l8a_rate_policy_wired.rs` replaces the gate with the mechanism: a **downstream** table
> (the path that mints its `code_idx` rather than reading a compile-time row) driven through
> `dyn_warn!` with the `Log` target switched fully OFF, so every drained record is the test's own.
> Four legs, in one `#[test]` because `RATE` and the drain role are process-global: `Every` 8/8
> **first, as the positive control**, `EveryN(4)` 4 of 16 with 12 counted, `MinIntervalMs(60_000)`
> 1 of 8 with 7 counted — a window nothing can cross, so no leg sleeps — and `Once` 8 of 8, which
> pins the refusal above rather than leaving it implicit.
>
> **AND THE WIRING EXPOSED A DEAD DATUM IT WOULD HAVE CREATED.** `rate::suppressed` and
> `rate::unindexed` were `pub`, written by `admit`, and read by **nothing** — so the first policy
> that actually suppressed would have made a log quieter with nothing accounting for it, which is
> the defect the limiter is supposed to prevent, arriving through the limiter. `03-CODES-REGISTRY`
> already required both to be printed by the census and they were not. `census::print` now emits
> `LOG-CENSUS limiter suppressed=N unindexed=M`, **unconditionally**: a line that appeared only
> when a counter was non-zero would make "refused nothing" and "does not report the limiter" the
> same output.
>
> **`ONCE_SITES` DOES NOT EXIST, AND TWO DOC COMMENTS SAID IT DID.** The corpus assigns the
> per-SITE breakdown to an `ONCE_SITES` walk printing `LOG-ONCE` rows;
> `crates/boyko_log/src/rate.rs` named it as the answer to its own aggregate, and
> `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` went further and claimed a site "enrols
> itself" in it. Neither is true and no site does. Both corrected in place. This is the same
> finding as the 39 latch-less `Once` sites, from the other end: **nothing enumerates `Once` sites,
> so nothing could notice that 39 of them do not latch.**
>
> **"AN `Every` ROW COSTS NOTHING" IS NOW MEASURED, NOT ASSERTED — AND THE FIRST ATTEMPT TO CHECK
> IT WAS THE WRONG COMPARISON.** The corpus recorded `downstream_code_warn` at **10.16 ns** before
> the wiring; the first sitting after it read **11.72 ns**, four quanta above resolution, which
> reads like the gate costing 1.5 ns. It is not: across four sittings on this box the SAME leg read
> 10.16, 10.94, 11.72 and 12.11 ns, so a cross-sitting absolute cannot separate the gate from a
> rebuild's code layout, a busier box or a different minute.
>
> `log_enabled_cost` therefore gained an **in-sitting** leg, `log_enabled_rate_gate_every`:
> `warn!` with an `Every` code (FOUR gates) against `info!` with the identical format literal and
> the identical single `u32` (THREE gates). Everything else that differs — level byte, class byte,
> code number — lives in the per-site `static` and is never touched on the emitting thread.
> **Two sittings: 11.33 vs 11.33 (floor 0.39) and 12.11 vs 12.11 (floor 0.73). Delta +0.00 ns
> both times.** The `const` match folds; the fourth gate is not resolvable against a three-gate
> site.
>
> **A COMPILE-TIME GATE ON THE TWO COPIES OF THE POLICY.** The value the site folds now comes from
> the newtype and the value the registry prints from the row; `codes!` and `declare_codes!` write
> both from one token, and a `const _: () = assert!(rate_eq(..))` per row makes that structural.
> Without it, a `code_class_new!` that dropped or transposed its rate argument would leave **every
> test in this crate green** — the registry printing one policy while every call site folded
> another, visible only to a behavioural test of a damped code.
>
> **THE `Once` REGISTER IS BUILT, AND IT FOUND ITS FIRST DEFECT IMMEDIATELY.** `ONCE_SITES` and the
> `LOG-ONCE` rows — specified in `00-GOAL-TARGETS.md:37`, `01-EMISSION-RING.md:273` and this
> document — now exist as `crates/boyko_log/src/once_sites.rs`. `LogSite` gained a `rate` field
> (cold `'static` data, never read on the emission path) so the **DRAIN** can do the accounting:
> the hash, the probe and the counter all run on the consumer, and the emitting thread pays
> nothing. `census::print` emits
> `LOG-ONCE code=W2102 site=<file>:<line> fired=N suppressed=UNCOUNTED(by policy)`, with the full
> path rather than the corpus's `device.rs` basename because twenty directories here hold a
> `mod.rs`. Counted at EMISSION, before the per-sink filters — a register that counted delivered
> records would read `fired=0` for every site in a process whose sinks are off, which is the
> silence `W0111` exists to refuse, reproduced inside the mechanism meant to expose it.
>
> **`fired > 1` IS the defect**, and the row says so in words: `<-- DECLARES Once AND HAS NO LATCH`.
>
> **`W0111` was emitting from inside a public ITERATOR.** `report_unsunk` was called from
> `census::rows()`, which a host may walk every frame — so a row declaring `Once` produced one
> record per unsunk target per frame. Reverting the fix and walking `rows()` ten times makes the
> register read `fired: 10`. The report moved to `census::print()` behind a named
> `UNSUNK_REPORTED` latch. The general form: **a query must not have a diagnostic as a side
> effect.** `E0109` is the same class one file over — `report_unopenable` had no latch, and its
> `Once` was honoured by the call structure rather than by anything at the site.
>
> **AND THE FIRST DRAFT OF THAT LEG WAS A CONTROL THAT COULD NOT FIRE.** It left the `Log` target
> Off from the test's own setup, so gate (c) refused `W0111` and BOTH legs read an empty register —
> leg D passing not because the query had stopped emitting but because nothing could emit at all.
>
> **FIVE REDs.** (1) Unwire `dyn_warn!`'s fourth gate ⇒ `left: 16, right: 4`, the pre-commit state
> exactly. (2) Flip the `Every` arm to `false` ⇒ `left: 0, right: 8` on leg 1 — the control catches
> a gate closed too hard, which is what makes legs 2 and 3 mean "the declared policy" rather than
> "the gate is shut". (3) Print a literal `0` instead of the live counter ⇒ `0 -> 0`. (4) Drop the
> limiter line from `print` ⇒ `left: 27, right: 28`, which is the check that `render_limiter` is
> not itself a function nothing reaches. (5) Hard-code a policy in `code_class_new!` ⇒ `E0080` at
> `W0103`, `W0111`, `W0112`, … , one per row that disagrees.
>
> **FOUR MORE for the register.** (6) Drop the drain's `note` call ⇒ the latched site is absent,
> `left: 0, right: 1`. (7) Let the register record every policy ⇒ leg C reds, and `fired` is
> revealed as a per-site record count wearing a policy's name. (8) Pin `fired` at 1 instead of
> counting ⇒ `left: 1, right: 5`, the unlatched site made to look disciplined. (9) Put
> `report_unsunk` back inside `rows()` ⇒ `fired: 10` for `census.rs`, which is the defect the
> register was built to name.

**4. The `#[cfg(debug_assertions)]` question is per SITE, and L7b's rule is not "delete every
gate".** `boyko_physics/src/soft/self_collision.rs` had three debug-only warnings. Two lost the
gate: `W1301`'s condition (`radius <= 0.0`) **is** the release guard one line above the call, and
`W1303`'s is two compares once per call. The third kept it: deciding `W1302` needs
`min(body.c_rest)` — a scan of every distance constraint, per `resolve_self_collision` call, per
body — which a release build does not otherwise perform. L7b's rule is about conditions release
*already computes*; it is not a licence to add an O(constraints) pass for a diagnostic. The fork is
visible in the test list itself:

```text
cargo test --release -p boyko-physics --lib l8a_self_collision   ->  2 passed
cargo test           -p boyko-physics --lib l8a_self_collision   ->  3 passed
```

`boyko_serialize`'s `warn_dense_viafn_skipped` lost its gate on the same test: `report
.dense_stores_skipped += 1` runs on the line above the call in every profile.

**5. The doc-rot blast radius was thirteen sites, not two — and five of those were the ones a
reader could find.** The plan named
`crates/boyko_image/Cargo.toml:5` and called it "doc-rot with a two-line blast radius". Measured,
the claim "`boyko_image` is a workspace leaf" is also written in `crates/boyko_render/Cargo.toml`'s
dependency comment, `docs/ARCHITECTURE.md:183` and `docs/SYSTEMS.md` twice — and
`crates/boyko_serialize/Cargo.toml`'s "depends on `boyko_ecs` + `std` ONLY" becomes false in the
same commit. All five are repaired here. **A sixth was already stale and is repaired with them**:
`ARCHITECTURE.md`'s `boyko_rhi_vulkan ──→ boyko_rhi, boyko_sdf_math` row lost `boyko_diag` and
`boyko_log` at profiling rung 5 and L7 respectively, and nobody updated it — which is the same
defect this rung was warned about, one rung earlier, in the same file.

**The other eight were LINE ANCHORS, and only the sweep could have found them.** One added `use`
line in `mesh_geometry_table.rs` moved every definition below it by one, and
`docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md` cites eight of them —
`internal_docs_line_anchors_land_on_definitions` named all eight with the wrong line's text beside
each. Re-derived; every one moved by exactly `+1`. This is the half of doc-rot that reading cannot
catch and the reason that gate exists: the five prose claims above were found by grepping for a
sentence, and no grep would have found these.

**6. `boyko_image`'s own test fixtures were built wrong on purpose, and it mattered.** `wrap_zlib`
wrote a dummy Adler-32 trailer, documented as "tests need not duplicate the checksum algorithm" —
so roughly twenty decode tests silently exercised the corrupt-stream path. Invisible while the
warning was an `eprintln!` nobody counted; measurable interference the moment it became a record.
The fixtures now carry real checksums and exactly one test builds a mismatching stream on purpose.

**7. A process note, because it cost work.** `git checkout -- <file>` was used to undo a
deliberately-broken RED edit. That file also held the rung's unstaged work, all of which it
discarded; the recovery was a copy taken before the edit. **A RED is reverted from a backup copy,
never from the index.**

---

### L7b: the seven-site code became two, and the reachability gate turned out not to be a test

**1. `E2103` ×7 is `E2103` ×4 + `W2106` ×3, and the split is a measurement.** The ledger assigns one
code to all seven degrade-to-`None` builders in `present/targets.rs`. Read against `record_vb`, the
seven are not one thing: **four** are consumed with `.expect(..)`
(`present/passes/vb.rs:3706`, `:3776`, `:4291`, and `thin_normal` through the two match arms that
require it) and **three** with `if let Some(..)` (`:3988-3990`, `:4088`). The first group kills the
frame; the second loses an opt-in effect and renders. The file's own comment already says so —
*"opt-in, no dependents: `record_vb` GRACEFULLY skips … UNLIKE `vb_geo_aux_set`/`vb_ssao_set`/
`vb_split_set1`, which are the split's own mandatory core and `.expect()`-panic if missing"* — so a
single code would have erased a distinction the code beneath it states explicitly. **In this
registry the class letter IS the level**, so one row could not have carried both. `W2106` is the new
number; `E2103` keeps the four fatal ones.

**2. `RatePolicy::Every` for both, and the reason is frequency, not severity.** They run at target
build and at each resize, never per frame. A `Once` would report the first resize that ran out of
device memory and stay silent through every one after it — precisely when a reader needs the repeat.
The `W2102` trio keeps `Once` **per site** (F11), and the test that pins it trips all three and
counts: with one shared latch it reads `1 != 2` on a `not(hwrt)` build and `1 != 3` with `hwrt`.

**3. The wiring is gated by the COMPILER, not by any of these tests — found by a RED that refused to
redden.** Deleting the `report_present_mode_fallback` **call** from `Swapchain::new_with_present_mode`
left `w2105_announces_the_fallback_once_not_once_per_resize` **green**: the test calls the reporter
directly, so it can only ever prove the reporter behaves, never that production still reaches it.
What actually reddened was `cargo clippy --all-targets -- -D warnings`, with
`error: function report_present_mode_fallback is never used` and exit 101.

> That gate exists **because the reporters are private**. Had they been made `pub` so an integration
> test in `tests/` could call them — the obvious first design, and the one this rung nearly took —
> `dead_code` would not fire and the wiring would have had no gate at all, while every test stayed
> green. The compiler proves reachability; the tests prove behaviour; neither substitutes for the
> other. Written into `log_probe.rs`'s header so the next person to widen a reporter's visibility
> reads why not.

**4. The emission order was checked against the host, because L7a's own fix made it a live question.**
Every `21xx` code except `W2104` fires during boot — `E2101`/`W2102` inside `VulkanContext::boot`,
`W2105`/`E2103`/`W2106` in the window→surface→swapchain→targets chain — so if the device booted
before the logger was enabled they would all be written into a ring with no consumer, which is
precisely the defect `db197537` closed one rung earlier. **Verified at HEAD**: the logger is booted
and enabled in `EnginePlugins::build` (`plugins.rs:298`), which runs when the app is *constructed*;
`VulkanContext::boot_singleton` is `runner.rs:160`, reached from `app.run()`. The device boots
after. No test pins that ordering, and one is not owed here — `boyko_app/tests/log_host_*.rs` prove
the host enables the logger at all, and the ordering claim is a two-line read of the same file —
but it is written down so a reordering has somewhere to be checked against.

> **The `hwrt` clippy leg has exactly one correct incantation, and it is not the obvious one.**
> `cargo clippy --workspace --all-targets --features boyko_rhi_vulkan/hwrt` **fails to compile**
> `boyko_app` — `error[E0063]: missing fields ... in initializer of GBufferScene` — because it turns
> on the backend's `#[cfg]`'d fields without turning on the `#[cfg(feature = "hwrt")]` initializers
> in the crate that constructs the struct. `boyko_rhi_vulkan/Cargo.toml` states the mechanism in its
> own `[features]` comment: *features unify per PACKAGE*, so a flag no source in `boyko_app` names
> still changes what `boyko_app` must write. The leg that works is the forwarding chain,
> `--features boyko-app/hwrt` (`boyko-app` → `boyko-render` → `boyko_rhi_vulkan`). Recorded because
> "clippy clean in both configurations" is a per-rung claim and the wrong spelling makes it a claim
> about a tree that does not build.

**5. The new tests were each other's interference, and the argument that they were not was wrong.**
`log_probe`'s first header reasoned that an exact `assert_eq!` on a per-target counter is sound here
because the only `VulkanContext::boot*` in the `--lib` binary passes `enable_validation: false`, so
no `E2101` arm can fire. **That argument counted the emitters that existed before the rung and
missed the ones the rung was adding.** A filtered `--lib w2106` run showed `left: 2, right: 1` — the
extra record was the sibling `e2103` test's, landing inside the window while the two ran on
different harness threads. The full run had passed on scheduling luck. The four observers now
serialize on one `OBSERVE_LOCK`, and the exact run that was red is green.

### Six L6 decisions taken at implementation, and the three things arming its checks found

1. **A `B` code reaches source through `PanicCode`'s `Display`, positionally.** A `Live` row needs
   its *identifier* in the CODE stream, and a `B` code has no macro — its site is a `panic!`, where
   the code has only ever been a string literal. `impl Display for PanicCode` prints
   `boyko-Bnnnn`, so `panic!("{}: …", B9001)` renders **byte-identically** to the literal it
   replaced and every `#[should_panic(expected = "boyko-B…")]` in the engine keeps matching.
   **`{B9001}` would not work and the difference is invisible**: an inline format argument lives
   inside the string literal, so the walker's LIT stream sees it and its CODE stream does not — the
   row would read as an orphan while looking migrated. Written into the impl's own doc and into
   every site's comment. `WarnCode`/`ErrorCode` get **no** `Display`: they reach their sites through
   the macros as `.number()`, so one would be surface with no caller.

2. **Check 6 is re-specified, because the corpus's form is false of a correct tree.** "Panic-class
   `B` codes appear only inside a `#[cold] fn … -> !` or a `panic!`" cannot hold here:
   `ScheduleBuildError` is deliberately dual-purpose — `build` panics with `e.formatted()` while
   `try_build` returns the same value as an `Err` — so `B9001`/`B9002`/`B9004`/`B9005` necessarily
   live in a `String`-returning method that also feeds `Display`. Enforcing the literal rule would
   have required deleting the recoverable API. What the rule is **for** survives whole, and it is
   the corpus's own red state: *a `B` code is never an argument to an emission macro*. That has a
   demonstrable failure and it was demonstrated. What it cannot claim — that every `B` code reaches
   a panic — is in the check's own doc.

3. **Check 5's corpus is all test code, and "named" is a stated proxy for "observed".** The corpus
   names `crates/**/tests/**` plus `#[should_panic(expected=`; measured against the tree that
   would have called thirteen genuinely-tested profiling codes untested, because their tests are
   `#[cfg(test)] mod tests` blocks inside `src/`. The corpus is therefore the walker's own
   test-only set **plus** each production file's tail from its first `#[cfg(test)]` — an
   approximation whose error direction is one-way and written down: too large a corpus can only
   make the check permissive, never falsely red. `codes.rs` and `code_registry.rs` are excluded,
   because the first defines every identifier and the second names them as *data* — check 0's
   sentinel is `boyko-W1501`, which would have silently satisfied that row's claim. A code counts
   as named by its identifier **or** by its prefixed literal, and the second is the stronger form:
   a test asserting the emitted text beats one naming the constant.

4. **`E0201` keeps a last-resort `eprintln!`, and only in the configuration where nothing else
   will.** The ledger prescribes `error!` + `flush()` before `abort()`. Measured: with diagnostics
   never enabled, the record sits in a ring nothing will ever read, `flush()` answers `NoConsumer`,
   `abort()` runs no destructor — and the abort decision becomes **invisible**, strictly worse than
   the unconditional `eprintln!` it replaced, on the one path where the message matters most. The
   site therefore prints for itself **iff** `flush()` returned `NoConsumer`. Exactly one line in
   either configuration. **L8c owes it a `print_allowlist.txt` row** naming that reason.

5. **`W0701` latches per CALL SITE, through a `static` in a generic body.** `send_one` and
   `send_many` pass their own `OnceSite` to one shared `#[cold]` reporter, so a storm through one
   cannot silence the other (F11). A `static` inside a generic function is shared across every
   instantiation, which is exactly the granularity wanted: one report per *source site*, not one
   per event type. **The rate limiter is still not the mechanism** — `rate::admit` has zero
   production callers in the workspace, because every engine row declares `Every` or `Once` and
   both are answered by a site-local latch. Recorded rather than "fixed": reaching for
   `MinIntervalMs` here would have dragged a clock read onto a cold ECS path and put the rate
   decision *ahead* of the macro's own runtime gate.

   > **PARTLY SUPERSEDED.** `rate::admit` now has production callers (note 3's supersession), and
   > the second half of the objection is answered by construction: the fourth gate is **last** in
   > the `&&` chain, so the rate decision can no longer sit ahead of the runtime gate. The first
   > half stands unchanged — `W0701` still wants a per-CALL-SITE latch, which `MinIntervalMs`
   > (per CODE) is not, and its clock read would still land on a cold ECS path. `W0701` keeps
   > `Once` and its two hand-passed `OnceSite`s **for the granularity, not for want of wiring**.

6. **`E0801` is L6's, though the row's code list does not name it.** The ledger's file table covers
   `ecs/asset/server.rs (1)`, and its disposition class is "everything else → `error!`/`warn!` with
   codes". Block `08xx` is assets by the block map's own six-domain split of `04xx`–`09xx`, so the
   code is `E0801`. A migration rung that left one `eprintln!` in the crate it migrated because a
   summary table did not enumerate it would be the "green up to the first thing already known"
   failure, one level up.

**Three things arming the checks found, none of them predicted.** (a) Check 2 shipped at L4 as
`is_file()` alone — an empty page satisfied it — and tightening it to the specified three sections
found `W9212.md` with no `## Why` at all; its argument lived under `## Refused, not clamped`, a
fine subtitle and not a section a reader can look for. (b) Check 5's first run: **eight of twenty**
`Live` `W`/`E` rows were named by no test. Five were L6's own and got observing tests; three
profiling rows and `E0201` are in `tests/untested_codes.txt`, each naming why, asserted in both
directions so the list can only shrink. (c) `W0103` was the interesting one: its condition **is**
tested — `file_sink_and_census.rs` asserts the cap behaviourally — but the test never names the
code, so the check would have called a tested row untested. The fix is one assertion naming
`boyko-W0103` beside the ones that observe it, not a ledger row. That is the clearest possible
statement of what this check does and does not see.

### Five L4 decisions taken at implementation, and the L1 defect its gate found

1. **`TARGET_STATS` moves here from L16.** The L4 row owes a `LOG-CENSUS` with `UNPROVEN(lossy)`,
   and the L4-gate owes "`UNPROVEN` at 0 records **and** at `dropped > 0`" — neither is expressible
   over process-wide counters, because the census's whole claim is *per target*. Shipping a census
   whose rows could not distinguish two targets would be the vacuous artifact this vocabulary was
   invented against. L16 keeps `LogCensus`/`DiagCensus` (the ECS `Resource`s) and the ring reader;
   the `.bss` array and its two writers land with the report that reads them.

2. **`dropped` is charged to the target as well as to the substrate's lane row.** The two answer
   different questions — "which thread lost records" and "which category is incomplete" — and a
   census reading only the first would call a target clean while every one of its records went
   missing on a driver-callback thread. The RMW is on the cold path only.

3. **The census is printed by `shutdown` and `disable`, through one `close_out`.** Two copies would
   be how the two came to disagree about whether a session's last lines reached the disk.

4. **`LogConfig.file` is a `bool` and the path is recorded separately.** A configuration that owned
   a path could not be recorded without allocating, and `boot` is a pure struct-fill. A path longer
   than the buffer is **refused, not truncated** — a truncated path names a different file.

5. **Checks 2 and 3a are ARMED**, by the mechanism L2b left for exactly this moment: a test asserted
   that no row was `Live` so that the first flip would red and force the real checks to be written.
   `W0103` flipped and it did. The vacuity test is deleted rather than kept — a precondition
   assertion and the check it stood in for cannot both be true. The codes test's claim moved with
   it: "every row is `Pending`" became "every `Pending` row names its rung **and** the `Live` set is
   pinned", so a row that goes `Live` without its emitter, or an emitter that lands without
   flipping its row, still reds.

**The defect this rung's gate found is L1's, not L4's, and it was a use-after-free in release.** The
producer's wrap rule has two arms: a tail long enough for a header but too short for the record
carries an explicit PAD, and **a tail shorter than a header carries nothing at all** — there is no
room for a PAD header, so the producer simply advances `write` past those bytes. *The consumer had
only the first arm.* It read a "header" out of the 1..`HEADER_BYTES`−1 bytes the producer had
skipped and never written, took `len` from uninitialised memory, and walked off into the ring —
`debug_assert!(len >= HEADER_BYTES)` in a test build, a torn read through a corrupted
`&'static LogSite` in release. Same class as the F6 admission arithmetic, entered through the wrap
instead. It needs the cursor to land in a 19-byte window out of 16 384, so L1's fixed-size gate
either hits it every lap or never; the regression test cycles the payload length so the cursor
sweeps every residue class. **It surfaced by accident**, while a `RED` probe for the file-sink cap
left the drain loop running far longer than the gate normally does — which is an argument for
running a probe's consequences out rather than reverting at the first red line.

### Six L5 decisions taken at implementation

1. **There is no `Last` schedule in this engine, so the drain runs in a SET.** This row and
   Decision 26 both say "`log_drain_system` in `Last`". `crates/boyko_ecs/src/ecs/core/app/app.rs`
   declares `CoreSchedule` as a **closed set of two** — `Main` and `Fixed` — and its own doc gives
   the intended answer: *"finer-grained structure WITHIN a schedule is what Phase-15 sets are
   for."* So the drain is registered in `Main`, `in_set(LogSet)`, and `LogPlugin::build` interns
   the set with `configure_set` so a host's `.before(LogSet)` resolves regardless of plugin
   add-order (the `CameraPlugin` idiom). **What that costs, stated:** with no edge the scheduler
   may place the drain anywhere in the frame, so a record emitted after it lands in the NEXT
   frame's ring. Decision 26's "one frame under `Scheduled`" bound therefore holds only for hosts
   that add the edge; without one it is two. That is a real weakening of a specified bound and it
   is recorded rather than absorbed.

2. **`LogStats` ships ONE field, not eleven.** `logging/game-facing-surface` pins the full
   eleven-field struct, and ten of them are folds of state this rung does not own — the lane-side
   loss fold is L13a, `suppressed` is L4, `sampled_out` is L12, `codes_unindexed` is L11a.
   Declaring them now would put ten fields in a `Resource` that read `0` forever, and **a value
   that is structurally always zero is indistinguishable from a measurement of zero**: a HUD
   showing `emitted: 0` while the log streams is worse than one that does not offer the number
   yet. Each field arrives with the rung that can fill it. Same reasoning excludes `LogCensus`
   from this rung entirely (it is L16's, with `TARGET_STATS`), so the `assert_send_sync` pin
   names `LogRing` and `LogStats` here and gains `LogCensus` at L16.

3. **`log_drain_system`'s flag check is present, argued, and NOT verifiable at this rung.**
   MEASURED: deleting `if !ecs_ring_enabled() { return; }` leaves the L5 gate GREEN, because the
   system's only duty here is consuming the handoff and an empty ring is a no-op either way. The
   check is still correct — at L16 the `frame_epoch` record and the `TARGET_STATS` snapshot are
   written on the system's **own account** and would materialize the columns on frame 1 in a
   process that never enabled logging. It is written now so the hole is not left for a later rung,
   and the test says in words that it does not discriminate it. **L16 obligation: delete the check
   and confirm the flag-off assertion reds.**

4. **`VmColumn` gains `as_mut_slice`.** The drain copies a formatted line as a run; `set` in a
   loop pays a release bounds check per byte, which is the right price for a structural-change
   path and the wrong one for a `copy_from_slice`. It exposes exactly `as_slice`'s span with
   `&mut self` exclusivity, neither grows nor commits, and so cannot move the base.

5. **`ECS_HANDOFF` is zero bytes when the compile ceiling is `Off`**, mirroring `LANE_ARRAY_LEN`.
   In that build no site survives the const gates, so nothing can be emitted, drained or pushed —
   reserving 256 KiB of `.bss` for a ring with no reachable producer is a cost with no
   corresponding capability. `push`/`drain_into` const-fold to a `return` there.

6. **The consumer role's pass is `lifecycle::drain_once`, not the sink loop's body.** Three
   callers need exactly this pass — the resident sink thread, a host draining by hand, and L15's
   `SinkMode::Scheduled` — and a pass that differed between them would make "was the ECS ring fed"
   depend on which of the three ran.

**One defect this rung's gate caught, recorded because the shape recurs.** `LogRing`'s arena wraps
by abandoning the tail remainder rather than writing it. A line lying wholly inside that abandoned
tail is therefore never overwritten, so it is never evicted, so it becomes the oldest live line
**forever** — and the eviction walk stops at the first non-intersecting tail, so from that moment
on it evicts *nothing*. Observed as `len` climbing past the arena's capacity (1169 → 1650 → 2150 →
…) while the ring silently handed out slices of other lines' text. The premise "the cursor's next
span is always the oldest live line's" was true everywhere except at the wrap, which is the one
place it was not checked. **The first version of the test did not catch it**: 512 B lines from a
two-symbol alphabet, at an alignment the 512 KiB arena divides exactly, made a corrupted read
byte-identical to a correct one — it failed only on a tail assertion, i.e. on the wrong claim. The
repair was to make each line's content and length a function of its own sequence number.

Ordering constraints: **D0/D1 before L0**; L10 before L11a (a dynamic target is the first consumer
of a downstream code); L12 after L1; L13b after L13a (rotation is shared); L15 after L13a (the
crash sink shares the file machinery); L16 after L12 and L13a (`TargetStat` carries
`sampled_out`); **L8b after profiling rungs 7 and 7b**; **J1 (= L17 + profiling 14)
second-to-last; J2 last**.

---

## Metrics and validation

### Benchmarks (`crates/boyko_log/benches/emit.rs`, criterion, `harness = false`)

Every row runs against a control **in the same sitting** — because this repository has measured its
own wall-clock floor at 6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol, a number
without an in-sitting control is not a measurement. No benchmark binary may contain `time` /
`update` / `setup` / `install` / `patch` in its name (Windows os-error-740). Never two bench jobs
concurrently (`target/` once reached 74 GB and took the disk to zero, masquerading as mingw
errors).

**Every bench below carries a `config_tag = {profiler, logger}` in its baseline file** *(S10)*. A
sitting whose tag differs from the baseline's returns `NotResolved{ConfigMismatch}` and does **not**
fail the rung — a number measured without the other subsystem present is not a number about the
both-present configuration. The tags become uniform at **J2**, the joint baseline sitting.

> ⚠️ **MEASURED AT L10-C: NOT ONE OF THESE TWELVE EXISTED.** `boyko_log` had no `benches/`
> directory and no bench by any of these names existed anywhere under `crates/`. Eleven rungs
> were reported as gated while the performance half of their gate table was empty — and two
> decisions rest on numbers from it: Decision 14 reverts the packed control byte *if*
> `log_disabled_runtime` resolves against the v2 shape, and G8(d) strikes Decision 2's gate-(a)
> claim *if* it does not resolve. Both are decision procedures over measurements nobody had
> taken. **L10-C built the two G8(d) needs plus its control; the other ten remain UNBUILT**,
> and this table does not become satisfied by that one bench running.

| Bench | Target | Control |
|---|---|---|
| `log_disabled_runtime` *(**BUILT** L10-C)* | ≤ 3 ns | the same site enabled; **and the v2-shaped unpacked gate, which must be NOT RESOLVED** (G10d) |
| `log_disabled_warn` *(**BUILT**; **0.50–0.55 ns**, PASS)* | ≤ 4 ns | `log_disabled_runtime` (an `info!`, untouched by `sink_can_accept`) in the same sitting. **The delta is an UPPER BOUND on S5, not S5 itself**: the `info!` control measures within its own noise of an empty loop — a runtime-disabled `info!` folds to nothing on this tree — so the delta carries the warn gate's own cost as well. Measured S5 ≤ **0.20–0.25 ns**. Timed in **1 000 000-call blocks**, because a disabled site publishes nothing and the 256-call lane cap that forces 0.391 ns/call resolution on every other row does not apply here; that is why this row is measurable while `log_gate_cost`'s disabled leg is not |
| `log_enabled_0args` / `_2u32` / `_str32` *(**BUILT**; 9.38 / 10.94 / 12.50 ns, all PASS)* | ≤ 15 / 20 / 30 ns | runtime-disabled — **which measures AT the instrument floor on this box** (2 quanta of 0.391 ns), so the *delta* column is bounded by resolution and only the absolutes carry a verdict |
| `log_enabled_rate_once_fired` *(**BUILT**; returns **NO SUBJECT**, and the reason CHANGED)* | ≤ 5 ns, **no store, no shared line** | `Every` policy — measured delta **0.00 ns**. Originally because the emission macros never called `rate::admit` at all; since the fourth gate landed they do, and this row **still** has no subject — by design rather than by omission. `Once` folds to `true` inside `__log_rate_admits!` because the latch is the SITE's own named `OnceSite`, never one the macro places (a `static` in a macro expansion cannot be named, and `OnceSite::reset` exists so an observer can reset the latch it tests). **The subject the row wants now exists somewhere measurable**: `OnceSite::claim` on a fired latch, which is one `Relaxed` load and no store, timed against an empty control at a site that does not publish — so it is not lane-capped and can use million-call blocks |
| `log_enabled_rate_gate_every` *(**BUILT**, and it is this rung's own row)* | the fourth gate must be free where it declares no damping | `info!` with the same argument — THREE gates against the `warn!` leg's FOUR, in one sitting. **Measured +0.00 ns twice** (11.33 vs 11.33, floor 0.39; 12.11 vs 12.11, floor 0.73): the `const` match folds. Built because the alternative was a cross-sitting absolute, and the same leg read 10.16 / 10.94 / 11.72 / 12.11 ns across four sittings — a spread wider than anything the gate could have cost |
| `sink_sustained_rate` | finds the drop knee; reports records·s⁻¹ | zero-record idle sink |
| `lane_padding_ablation` *(**BUILT**; **padding RESOLVED at 0.75–0.96 ns/item, 43–57 %**; **cursor cache NOT RESOLVED**)* | padded+cached vs padded-only vs neither | — . **Two threads, because false sharing is a two-core phenomenon** and a single-threaded ablation would show three identical layouts and print a verdict about nothing. Read as PAIRS, not a league table: `A vs B` isolates the cursor cache, `A vs C` the padding. The **padding pays for itself decisively** — every accepted sitting, always positive, ~half the per-item cost. The **cursor cache's effect flips sign between sittings** (−0.13 … +0.28 ns) and is inside the combined floor in most of them: this instrument cannot see it, which is not the same as it being worthless, and it was left there rather than chased with a longer sitting. ⚠️ **41 rounds is WORSE than 15**: a longer sitting drifts further and the A-vs-A twin rejected 3 of 4 against 1 of 3, with the same answer either way |
| `sched_cpu_logger_on_off` (gate **P1**, re-specified) | not resolvable above the floor, **at each of the two profiler states** | interleaved zero control, ABBA, **2×2 with {profiler absent, armed}** (S10) |
| `log_dyn_disabled` *(**BUILT** L10-C; returns **NOT MEASURABLE** on this box — see Decision 18)* | ≤ 4 ns, **and the delta vs `log_disabled_runtime` must RESOLVE** | `log_disabled_runtime`, same sitting, plus an A-vs-A' twin and an empty-loop control |
| `log_enabled_sampled_out` *(**BUILT**; bound RE-CUT as a regression guard, owner ruling 2026-08-17 — same ruling as L13b's `5×`)* | **≤ 8 ns AND ≥ 4 quanta cheaper** than shift 0 | the same site with shift = 0 |
| `log_enabled_0args_sampling` | NOT RESOLVED vs the pre-L12 baseline | pre-L12 baseline, same sitting |
| `log_pod_12b` *(**BUILT**; `encode_pod` **1.04–1.06 ns**, PASS; ratio **74–79×**, estimate MET)* | ≤ 20 ns | `dsp!` of the same value, which must be ≥ 5× slower. **The subject is `encode_pod` alone** — `fmt_pod` runs on the SINK, later, and a first draft that timed encode+fmt against `dsp!` alone compared a round trip with a one-way trip and made the POD path look 1.5× *slower*. **And the path moves cost rather than removing it**: encode 1.06 + sink-side fmt ~127 ns is MORE total work than `dsp!`'s ~79 ns; the purchase is that the emitting thread pays 1 ns instead of 79 |
| `sink_sustained_rate_binary` | ≥ 3 M rec·s⁻¹ **and ≥ 5× the text sink** | the text sink, same sitting |
| `downstream_code_warn` *(**BUILT**; absolute **10.16 ns**, PASS; **the delta has NO SUBJECT**)* | ≤ 18 ns | the engine-code `warn!`, same sitting; the delta is the `idx_cell` load. **Measured 0.00 ns, structurally**: `resolve_idx` is called from exactly one place in the crate — `CodeNewtype::code_idx`, which addresses the RATE array — and no emission macro calls it. A downstream code and an engine code reach the ring by the same instructions. Same root cause as `log_enabled_rate_once_fired`. **What the load costs where it is actually performed is `code_idx_cost`: 1.33–1.36 ns** (dynamic 2.02 vs static 0.67), which is the number this row needs on the day `rate::admit` is wired into emission <br><br>**UPDATE (fourth gate wired):** `code_idx` now HAS an emission-path caller — but only from the `EveryN`/`MinIntervalMs` arms, and this bench's downstream code declares neither, so the delta is still 0.00 ns and the row still has no subject **as written**. Re-cutting it so the downstream leg declares `EveryN(2)` would give the mint load a subject on the emission path and should land the 1.33–1.36 ns `code_idx_cost` already measures in isolation — recorded as the next form of this row, not claimed as measured. |
| `sched_cpu_flag_on_off` (gate **GJ1**, S13) | (A) flag-on vs (B) flag-off vs **(C) ceiling removed, flags off** — three pairwise verdicts | interleaved zero control, ABBA, one sitting, **leg C is the control that decides whether the instrument measured anything** |

**`log_disabled_compile` is deleted** (B7). A compile-disabled site optimises to nothing and the
loop body is empty, so the bench measured the control against the control; it could not go red, and
adding `black_box` around the arguments would falsify the very "0 argument evaluations" property
under test. The property is proved by **G1** (symbol gate) and **G4** (side-effect probe), both of
which have named red states.

### P1 is re-specified: the old instrument could not respond to the quantity *(fixes F3)*

v2's P1 measured **windowed frame time** with the logger on vs off. `VK_PRESENT_MODE_FIFO_KHR` is
unconditional (`crates/boyko_rhi_vulkan/src/present/swapchain.rs:199`, with only a support check at
`:164`), so wall-clock frame time is clamped at the refresh interval; the wall floor here is
6.893 ms. A logger idling — or even emitting 1000 records/frame at 15 ns — perturbs CPU work by
≲ 15 µs, ~0.2 % of a clamped frame. **The channel is structurally incapable of responding**, so
P1's outcome was pre-determined `NOT RESOLVED`. That is the P4-6 lesson repeated: an instrument
whose band is structurally zero reports `RESOLVED`/`NOT RESOLVED` without measuring anything.

Worse, open question 4 then proposed to read that silence as a positive result — "if P1 comes back
`NOT RESOLVED` the thread is free" — the exact inference this corpus forbids three pages earlier
("absence is `UNPROVEN`, never `clean`"). Both are fixed:

- **P1 becomes a headless CPU-work bench.** `crates/bench_bevy_vs_boyko` already runs schedules
  with no swapchain and no FIFO clamp. P1 runs the same schedule logger-on vs logger-off,
  ABBA-counterbalanced, with an interleaved zero control (A0 vs A1) in the same sitting, and
  reports `RESOLVED(x %)` or `NOT RESOLVED` against the sitting's own floor. **What P1 cannot
  claim**: anything about a *windowed* frame, where present pacing dominates. That question is
  P2's, and P2 can only claim drift and leak, not perturbation.
- **v4 adds the second axis, because one was not enough** *(S10)*. P1 is a **2×2**: {logger off,
  on} × {profiler absent, armed}, all four legs ABBA-counterbalanced in **one sitting**, with the
  zero control interleaved. Its claim is "logger-on vs off **at a fixed profiler state**", reported
  at both states. The reason is not symmetry: the joint working set is 7-8 cache lines against this
  crate's isolated 4, so a logger-on/off delta measured with the profiler absent is a delta in a
  configuration a shipped frame never runs. A 1×2 P1 would have been a correct measurement of the
  wrong thing — which is the same class of defect as v2's FIFO-clamped channel, one level up.
- **The inference is struck.** `NOT RESOLVED` from P1 means **UNPROVEN**, and open question 4 no
  longer offers it as a licence. The sink thread's disposition in a shipping title is decided
  **structurally** — `shipping-min` exists precisely so the owner has an off switch that does not
  depend on a measurement this hardware may not be able to make.

**GJ1 is P1's shape applied to the flag, and it inherits P1's instrument for the same reason.** It
runs headless, never on a windowed frame, and its third leg exists because a two-leg A/B cannot
tell "the flag is off" from "the sites were never compiled in". Its specification, including what
it may not claim, is `SEAM.md`'s (S13); the row above records where it runs and the row in the gate
table below records its RED.

### Gates — every one has a RED that can be SHOWN, and a stated limit

v2's G1-G5 carry forward unchanged in substance (G1 relocated to L1-gate). G2, G7 and P1 are
re-specified above and in Decision 3. New and re-cut gates:

| # | Gate | RED variant that must be demonstrated once | **What this gate CANNOT claim** |
|---|---|---|---|
| **G17** | **Error-reserve arithmetic, three legs at TWO NAMED FILL LEVELS** *(the F6 regression gate, re-cut by B3)*. **Fill A — the reserve boundary**, `used` such that `need ≤ avail < need + ERROR_RESERVE` (e.g. `used = 15000`, `need = 40`, `CAPACITY = 16383`): (a) a further `Trace` is **refused and counted**, not written; (b) an `Error` still lands. **Fill B — genuinely full**, `used > CAPACITY − need` (e.g. `used = 16360`): (c) the lane is pre-seeded with known **undrained** records carrying a distinct byte pattern, and after the refused emit **every one of their bytes is unmodified and still decodes identically** | **replace `avail.saturating_sub(ERROR_RESERVE)` with `limit - used`** ⇒ at Fill A, `14336 − 15000` underflows to ~4.29e9, the `Trace` is admitted and **(a) fails**; at Fill B the same underflow admits a write of `need` bytes at `off = w & MASK` that runs past `read` into the seeded records and **(c) fails**. This is v2's exact code, so the gate reds on the shipped defect | **The two fill levels are not decoration.** At Fill A the broken arithmetic writes into genuinely free space — `used' = 15040 ≤ CAPACITY` — so no corruption is observable there, and (c) would be **vacuously green** if it ran at Fill A. That is precisely v3's error: leg (c) asserted an untouched *neighbouring lane* canary, which the F6 overrun can never reach, because the write offset is `off = w & MASK` and the pad/wrap rule keeps every byte inside this lane's `buf`. A cross-lane off-by-one belongs to test 6's wrap proptest, and that is where the neighbouring-canary assertion now lives — **it is deleted from G17**. G17 also cannot claim the ring is correct under *concurrent* drain; that is test 7's job. G17 drives a quiesced consumer on purpose, so the arithmetic is the only variable |
| **G18** | **`OUT_LOCK`, three-sided** *(the F8 gate, extended by B9)*. (a) A thread that acquires the lock and then panics releases it — a second thread's `write_oracle_line` completes within the deadline; (b) a re-entrant `write_oracle_line` from inside a sink panic handler **completes** and increments `OUT_REENTRANT`; **(c) the durable fan-out**: with **no console sink** and a crash sink configured, a `Warn` from a laneless thread appears in the **crash file** | replace the RAII guard with a bare `store(false)` after the write ⇒ (a) hangs and the test's own deadline reds it; restore v3's unconditional-stderr `write_oracle_line` ⇒ (c)'s crash file is empty ⇒ red | It cannot claim output is never interleaved. Under a **steal** it is, deliberately — and after B9 that includes **sync-routed records**, whose reason to exist is integrity. `OUT_STEALS > 0` in the census is the honest report; a nonzero value in a golden run is itself a defect signal. It also cannot claim durability past a `write_all`: `fsync` is `sync_durable`, opt-in |
| **G8** | **Dynamic targets, four-sided.** (a) A registered dynamic target's records arrive and appear in the census under its interned name; (b) registration past the 32-slot band returns `None` + `E0106`; (c) re-registering a name returns the same id; (d) the **bench** leg: `log_dyn_disabled` − `log_disabled_runtime` must **RESOLVE** above the sitting's floor | make `register_dynamic_target` grow past the band ⇒ (b) fails; share one slot for two names ⇒ (c) fails | It cannot claim the dynamic path is cheap enough for a hot loop — it bounds it at ≤ 4 ns disabled. **If (d) does not resolve, Decision 2's claim that gate (a) buys anything is STRUCK from this corpus** rather than restated |
| **G9** | **Downstream code minting, now with exhaustion** *(M3)*. (a) 16 threads mint one downstream code concurrently ⇒ exactly one dense index, no leaked counter value, `CODE_OCCUPANCY` advanced by exactly 1. **(b) exhaustion**: fill `CODE_OCCUPANCY` to `MAX_CODES`, mint once more ⇒ the mint returns `CODE_IDX_EXHAUSTED`, `boyko-E0115` fires **exactly once**, the record is **still delivered** with `Every` semantics, `codes_unindexed` advances, and **no two codes resolve to one `RateSlot`** | swap the reserve/`fetch_add` order ⇒ (a)'s density assertion fails; make the mint `fetch_add(1) % MAX_CODES` ⇒ two codes share a slot and (b)'s distinct-rate-state assertion fails | It proves nothing about a game's *registry completeness* — the eight checks are engine-scope and `codes_tidy!` must be invoked by the game. Written into the gate's own assertion message. It also cannot claim 512 is enough for a modded title; it claims only that running out is **defined, counted and never aliasing** |
| **G9b** | **`LogPod` encodes no padding** *(subject changed by B10)*. A `#[derive(LogPod)]` struct **with interior padding** (`{ a: u8, b: u32 }`) round-trips byte-identically under **Miri (Tree Borrows)** with **no uninitialised read**; `POD_LEN` equals the sum of field lengths, not `size_of::<Self>()`; a `&str` field is a **compile error** from the derive | **replace the derived field-by-field encoder with a `copy_nonoverlapping` of `size_of::<Self>()`** ⇒ Miri reports an uninitialised read on the padded struct. *(v3's red state — "drop the `POD_LEN == size_of` assert" — could only catch a **lying** `POD_LEN`; the correct-by-v3's-own-rules implementation was itself UB, and no leg covered it.)* | It cannot make `unsafe impl LogPod` safe for an arbitrary hand impl — the contract "writes exactly `POD_LEN` initialised bytes" is the hand-writer's burden. The derive is the documented route |
| **G10** | **Sampling: exactness, independence, non-perturbation — and what sampling does NOT suppress.** (a) shift = k over N records on one lane ⇒ delivered == `N >> k` **exactly** and `sampled_out == N − (N >> k)` exactly; (b) control leg shift = 0 delivers all N; (c) 8 threads × 8 targets: every (lane,target) pair independent; (d) **perturbation**: `log_enabled_0args` NOT RESOLVED vs the pre-L12 baseline; **(e) — relocated from L0-gate (B4)** — with a side-effect probe in argument position and shift = 1 over 1000 emits: **argument evaluations == 1000 AND delivered == 500**, asserted together | delete the `& mask` ⇒ (a) fails; share one counter across lanes ⇒ (c) fails; move argument evaluation behind the sample decision ⇒ (e)'s first number becomes 500 and reds | **(e) is the leg v3 got wrong twice.** It lived at L0-gate, where `SAMPLE_CTR`, the seed and the lanes do not exist (they are L1/L12) — unimplementable at its stated rung, the F19 failure. And it asserted **500 evaluations**, when the gate short-circuit means arguments are evaluated at step 1, *before* the sample decision at step 4 — the true count is 1000. Asserting both numbers in one leg is what makes the distinction legible: sampling suppresses **delivery**, never argument evaluation, and a user with a side-effecting argument needs to know that. It still cannot claim a sampled capture is *representative*: `1/2^k` is strided, not random. **If (d) resolves, `log-sampling` becomes default-off and the ≤ 15 ns row is annotated with the measured cost** |
| **G11** | **Loss-fold exactness, two-sided** *(subject replaced by S8; one gate now serves both diagnostics plans — it is `substrate/loss-fold`'s DG5 and the profiler's G4b under one specification)*. Preset a lane's `LossCell` to a known value, drop N more, fold ⇒ the global `LossTotal` advanced by **exactly N** and the cell was cleared by `fetch_sub(observed)`, not zeroed | **replace `fetch_sub(observed)` with `store(0)`** and run a **live** producer ⇒ an increment landing between the consumer's load and its clear is lost ⇒ the global lags the injected count ⇒ red | It cannot claim `LogStats.dropped` is exact *between* the last fold and process death — in that window the per-lane cell is the only record, and the census says `since_last_drain` explicitly. It also cannot close the **producer-side** window: `substrate/loss-fold`'s open Q2 is the owner-increment race, and `fold_into` does not ship until that call is made. *(v3's G11 tested `u32` saturation; with `u64` accumulation there is no ceiling to test, and the interesting property was never saturation but **fold exactness under a live producer** — which v3 did not gate at all.)* |
| **G12** | **Binary sink, three-sided.** (a) round-trip byte-identical to the text sink for the same records; (b) a record straddling a rotation appears exactly once and every rotated file decodes standalone; (c) throughput ≥ 5× the text sink in the same sitting | omit a dictionary record ⇒ (a)'s decoder cannot resolve a site; skip the dictionary re-emit on rotation ⇒ (b) fails on file `.1` | It cannot claim cross-version compatibility — the decoder **refuses** a `schema_version` mismatch. **If (c) does not separate, L13b is REVERTED** |
| **G13** | **Runtime sink control, three-sided.** (a) enabling a file sink mid-run from a non-sink thread: records before are absent, records after are present; (b) the requesting thread's **per-thread** allocation counter reads zero and no `open` occurs on it; (c) a deliberately blocking `CallbackSink` causes lane drops that are **counted** | perform the `open` on the requesting thread ⇒ (b) fails; silence the callback-stall drops ⇒ (c) fails | It cannot claim a filter change is instantaneous: a sink acts on the filter it read at the top of its current drain, so the boundary is fuzzy by up to one drain. Pinned as a property |
| **G14** | **Crash drain, THREE-sided** *(the third leg added by B5)*. (a) no consumer running, panic after an `error!` ⇒ the record reaches the crash file + `E0109`; (b) sink thread `Running` ⇒ the crash path does **not** take the role (`E0109` absent) and a per-record uniqueness check over both files shows **no record delivered twice**; **(c) `SinkMode::Manual` with a thread parked INSIDE `drain()`** (held open by a test barrier), panic on another thread ⇒ the crash path finds `DRAIN_OWNER` taken, does **not** drain, `E0109` is absent, and the uniqueness check over both files shows no record twice | replace `DRAIN_OWNER` with v3's `SINK_STATE.compare_exchange(Manual, CrashDraining)` ⇒ **(c) fails**: two consumers walk the same lanes, `STAGE`, `SITE_DICT` and `SINK_OUT`, and the uniqueness check reports duplicates (or Miri reports a data race). **v3 had no leg (c) at all** — it tested only `Running`, so the one hole it shipped was untested by construction, and it was reachable in production because `shipping-min` was the `Manual`-plus-crash-sink profile | It cannot claim survival of `abort()`, `SIGSEGV` or a guard-page stack overflow — the hook does not run (E22). It also cannot claim the crash file is complete when another consumer holds the role: that consumer writes what it staged, the rest is lost and counted. Partial mitigations named with their **real** limits (B9: a `write_all` is not an `fsync`). **And it cannot claim anything about a flag-off run**: with diagnostics disabled there is no hook installed at all (S13), which is G2 leg (c)'s subject, not this gate's |
| **G15** | **In-frame consumption, three-sided.** (a) a record emitted in frame N is visible to a system reading `LogRing::since` by frame N+2; (b) it is **not** visible before the drain that consumed it; **(c) handoff overflow is loud** — flood `ECS_HANDOFF` past `HANDOFF_BYTES` in one drain ⇒ the refused lines are counted as `LossClass::Sink`, exactly one `boyko-W0117` names the count, `LogCensus.lossy` is set, and the lines that **did** fit are intact | feed `LogRing` from the emit path ⇒ (b) fails (and would have coupled the hot path to ECS storage, and falsified B1's `Send`/`Sync` argument at the same time); silence the handoff refusal ⇒ (c) fails | It cannot claim a bound tighter than "sink park interval + one frame" (one frame under `Scheduled`). `LogRingIter::skipped` reports ring-wrap loss so a console cannot silently miss lines; `W0117` reports handoff loss, which is a **different** loss and is not folded into `skipped` |
| **G16** | **Build-profile symbol gate, four-sided** *(S9)*. (a) in the `shipping` CI leg no `emit_impl` monomorphisation reachable from a `debug!`/`trace!` fixture appears; (b) in `dev` it **must** appear; (c) `BOYKO_PROFILE=shipping BOYKO_LOG_MAX_LEVEL=trace cargo build` **fails** with a named message; (d) the sink header carries `build_profile`, `runtime_preset` **and** `ceiling` as three fields, and a fixture proves `runtime_preset` and `build_profile` **can differ in one binary** | drop the `GLOBAL_CEILING` gate ⇒ the symbol appears in `shipping`; delete the `compile_error!` ⇒ (c) builds and the header prints a ceiling the profile does not name; print one profile name instead of three fields ⇒ (d) fails. **AS BUILT (profiling rung 14): (a), (b) and (c) run; (d) does not exist.** (a)/(b) are one harness clause over `crates/profile_fixture_log`, a single-`debug!`-site bin built from one source under `dev` and `shipping`: `emit_impl` = 1 and 0 respectively. (c) is two-sided — the stray knob is refused *and* `BOYKO_PROFILE=custom` with the same knob must BUILD, or the refusal is indistinguishable from the knob simply being broken — plus a third leg refusing an unknown profile name by name rather than falling back to `dev`, which would ship a typo as a full-fat development build. **(d) needs `LogRuntimePreset` and a sink header, which L13b–L16 have not landed.** **Two limits of (a)/(b) that are real rather than rhetorical.** The fixture has **no dynamic site**: `dyn_debug!` is L10's and does not exist, so the clause covers the static path only and L10 owes the second site in that file. And the fixture must **open the runtime gate** (`set_target_level(Log::ID, Trace)`) or it measures nothing: MEASURED, its first draft did not, `CONTROL` is `.bss`-zero, LTO proved the third gate false for the whole program, and `emit_impl` read **0 in BOTH legs** — a census with no subject, which reads exactly like a pass | It cannot claim a *dynamic* site is compiled out per-target — dynamic sites have no gate (a); they are deleted only by `GLOBAL_CEILING`, which is why the fixture includes a `dyn_debug!` site. The cross-profile census is a CI **step** over two legs' artifacts, so it cannot claim anything about a profile CI does not build (`custom`). **It cannot claim anything about the RUNTIME flag either**: a symbol present in `dev` says nothing about whether it executes, which is GJ1's question |
| **GJ1** | **The off-cost is MEASURED, not asserted** *(S13; specified in `SEAM.md`, run here)*. Three legs, one sitting, ABBA-counterbalanced, interleaved zero control, on the headless schedule bench: **(A)** flags on at the shipping ceiling; **(B)** the **same binary**, flags absent; **(C)** the ceiling forced permissive (`BOYKO_PROFILE=dev`) with the flags **off**, so every site the shipping ceiling had deleted now pays one `.bss` load and one predicted branch. Reported as three pairwise verdicts (A vs B), (B vs C), (A vs C) | **the control RED, which is the entire reason leg C exists: if C does not resolve apart from B, the instrument measured nothing** ⇒ the gate reports `NOT RESOLVED (control inert)` and the free-when-off claim is recorded **UNPROVEN on this box**, not restated. **Second red**: delete the runtime gate from the emission macros so B becomes the same code as A ⇒ (A vs B) stops resolving. **Third**: move the sink-thread spawn back into `boot()` ⇒ G2 leg (b) reds on the flag-off run while GJ1 itself may not move at all — which is why the memory and boot-work claims are gated by G2 and G3, not by GJ1 | It cannot claim a *frame* got faster (this box's decidability floor is 6.3 / 14.3 / 4.7 / 13.5 %, so a frame-time claim below ~15 % is undecidable here); it bounds CPU schedule work at a stated flag state and nothing else. It cannot claim the MEMORY row — `.bss` absence from the image is `substrate/section-report`'s, and whether the OS commits an untouched page is UNPROVEN and not asserted. And it **may not fail a rung before J2** |
| **P2** | **30-minute soak, `shipping` profile, 5 K rec·s⁻¹.** (a) `dropped == 0`; (b) resident bytes flat between minute 5 and minute 30; (c) presented frame time vs logger-off, ABBA + interleaved zero control | leak one buffer per rotation ⇒ (b) fails; shrink the ring ⇒ (a) fails, which is the intended positive control for the drop counter at session scale | (c) **cannot resolve a CPU perturbation** — FIFO clamps it (F3). P2's (c) leg is retained only as a **drift/leak** check and is labelled as such in the artifact; the perturbation question belongs to P1's headless 2×2. P2 also cannot claim anything about a different game's emission profile — the load is synthetic and the artifact records its shape. *("windowed frame time" is renamed "**presented** frame time" throughout — S11: `window` is reserved for the statistics horizon, and `os_window` for the OS object.)* |

**Where no control is possible, it is written down rather than worked around**: a forced
`SYNC-HAZARD` is unavailable on this box (M25; G7 uses an ordinary validation error instead); a
*chained* validation-features node is unbuildable here (F2; G7's negative leg is validation-off
instead); and a hard-crash (non-unwinding) log tail is unobtainable by construction (G14's stated
limit). *(v3 listed code W0101 here. S4 **deletes** it rather than allowlisting it: an uncontrolled
code in one plan and a controlled `boyko-W9207` in the other, for one condition, is two answers to
one question. `tests/untested_codes.txt` loses the row.)*

### Mandatory tests

1. **G4 — three-way gate separability** (§L0-gate). Each gate has its own red state; the enabled
   leg must reach **1000**, not merely `0` when disabled. *(v3's fourth leg — "shift = 1 ⇒ exactly
   500" — is **deleted from this test** and reappears as **G10e** at L12-gate, over a split
   observable: 1000 argument evaluations **and** 500 delivered. B4.)*
2. **G1 — symbol gate** (at **L1**-gate, not L0 — F19). Disabled fixture: no `emit_impl` symbol.
   Armed fixture: symbol present. Red state: delete a gate.
3. **Allocations on the producer path — steady state 0, first emit 0** *(F26, tightened by S3)*.
   Via a **per-thread** counting allocator (`thread_local! { static N: Cell<u64> = const {
   Cell::new(0) } }` — const-init, no `Drop`, no TLS registration, no allocation of its own). Three
   legs:
   - **(a) steady state**: arm *after* a warm-up emit on the same thread; assert exactly **0**. Red
     state: make `encode` allocate.
   - **(b) first emit**: arm on a **fresh** thread *before* its first emit; assert **`== 0`**. v3
     asserted `≤ 1` and recorded the number, because its lane guard was a `thread_local!` with a
     `Drop` and destructor registration allocates on some platforms. **S3 deletes that guard** —
     `boyko_diag::lane` is a `Cell<u16>` with no `Drop` — so the allowance has no source left and
     the leg becomes exact. **Red state**: reinstate a `Drop`-carrying TLS guard ⇒ the count becomes
     1 ⇒ red. An assertion of `≤ 1` would have been green either way, which is why it is now `== 0`.
   - **(c) monotonicity**: 1000 emits on that fresh thread must not raise the count above leg (b)'s
     value. A per-emit allocation would show here even if leg (a)'s warm-up hid it.

   This covers `SinkMode::Thread`, which the process-global counter structurally cannot:
   `crates/boyko_ui/tests/zero_alloc.rs:44-60` had to add `ARM_LOCK` after observing an impossible
   **negative** delta from a sibling thread, and a permanently resident sink thread cannot be
   serialised by a test-local lock (M18). The process-global variant is retained as a second,
   `Manual`-mode gate with its limitation stated.
4. **Overflow drops and counts** — fill a lane, assert `dropped > 0`, exactly one `W0102` per drain
   with matching counts.
5. **Error reserve** — flood with `Trace`, assert a subsequent `Error` still lands.
6. **Wrap protocol** — proptest over record sizes crossing every tail offset in `LANE_BYTES-32 ..=
   LANE_BYTES`; assert no byte is written outside the lane (**poison the neighbouring lane's guard
   bytes and check them** — this is where the cross-lane canary belongs, because an off-by-one in
   the wrap rule *can* cross a lane boundary, whereas the F6 admission defect structurally cannot;
   B3), and that producer and consumer agree on every PAD.
7. **Staged drain under a LIVE producer** (B1's red state) — a producer running at full rate while
   the sink drains; assert every decoded record is byte-identical to what was offered. **v1's design
   fails this test; v1's tests never ran it, because both drove a quiesced producer.**
8. **Lane claim/retire, and the join** — 200 short-lived threads against `LANE_COUNT` lanes; assert
   every spare eventually returns to `FREE`, **and** that `Warn`/`Error` from unlaned threads
   reached the synchronous fallback **at its durable destination** (M26 + B9). Reclaim is
   asynchronous, so the assertion is "eventually, within a bounded flush", not "immediately".
   **Plus S3's three reds**: (a) a zone emitted on worker *k* lands in lane *k* and nowhere else —
   delete the `set_lane` call in `worker_main` ⇒ every worker reads `LANE_UNCLAIMED` ⇒ red; **(b)
   the JOIN red** — one fixture emits one `warn!` and opens one profiler zone on the same worker;
   the log record's lane field and the sample's lane index must be **the same integer**; give the
   logger its own registry back ⇒ they differ ⇒ red; (c) per-thread allocation count on first emit
   is **0** — reinstate the `Drop` guard ⇒ 1 ⇒ red.
9. **Flush without a consumer** returns `NoConsumer` immediately; **flush timeout** returns within
   2 s with `E0105`; **shutdown** detaches on timeout with `E0108`.
10. **Panic hook flushes** — `catch_unwind` around a panic after an `error!`.
11. **Registry: the eight checks**, each shown red once during development, over the three-stream
    walker.
12. **Rate policies** — `Once` (incl. the no-store property), `EveryN`, `MinInterval`,
    `suppressed_since_last`.
13. **Census** — `UNPROVEN` at 0 records **and** `UNPROVEN(lossy)` at `dropped > 0`.
14. **Miri (Tree Borrows)** — ring, claim CAS, typed header round-trip incl. `*const LogSite`
    provenance, staged copy.
15. **loom** — claim/retire and the cursor pair. (Loom *release* binaries crash at startup on this
    box, pre-existing; run loom in debug.)
16. **DELETED** *(S1)*. v3's test 16 pinned `report!`'s output byte-identical to the `VB-P1d`/`VB-P4`
    lines under concurrency. `report!` no longer exists and this plan writes no stdout, so the
    property has no subject here; the equivalent obligation moves to the profiler's artifact channel
    and to the S1 grep gate (`rg 'VB-P1d |VB-P4 pass=|VB-SV0-S1\.5 ' crates/*/src` ⇒ zero after
    profiling rung 7). **Replaced, not merely removed**, by S7's **stderr line-integrity** test at
    L3-gate: emit 200 `warn!` while the validation callback fires, under `cmd /c … > f 2>&1`; assert
    **every** `[vk-validation] ` occurrence starts a line. Red state: give `write_oracle_line` a
    raw-fd `write` ⇒ it splices into a messenger line ⇒ red. The number is deliberately not reused
    for another test, so a reader of an older review can still find what happened to it.
17. **`[vk-validation]` liveness and byte-exactness — with a POSITIVE control, because zero messages
    is this machine's measured normal** *(fixes F1)*. v2's test 17 said "`golden.ps1`'s grep matches,
    and the message is on the wire before the frame returns" — but `golden.ps1:226` matches nothing
    and `:232` prints "clean (0 messages)" in green at **zero**, which is the steady state here (a
    genuine missed barrier produced zero messages, twice). It could not distinguish "the prefix
    survived" from "no message existed", and its second clause named no observation mechanism at
    all. v3:
    - **(a) positive control**: a fixture makes a deliberately invalid call that produces ≥ 1
      ordinary validation message. Assert `count ≥ 1` and that the line begins with the byte-exact
      `"[vk-validation] "` **including the trailing space**, pinned against
      `crates/boyko_app/tests/vb_bench_query_validation.rs:116-118`'s constant.
    - **(b) ordering, with a mechanism**: immediately after the offending call returns, the fixture
      writes a synchronous marker line. Assert the `[vk-validation]` line **precedes** the marker in
      the merged stream. Red state: buffer the messenger ⇒ the marker comes first.
    - **(c) negative**: a run with no invalid call produces zero `[vk-validation]` lines. *(v3's (c)
      also asserted a census row reading `status=UNPROVEN` for a `vk-validation` target. **That
      clause is struck** — M4: no record can ever reach such a target, so the row was inert by
      construction and is deleted from the census. What (c) asserts is the absence of the lines,
      which is an observation about the messenger's own channel.)*
    - **What test 17 cannot claim**: that a run with zero messages is a run with no defects. It never
      could; the sync-validation confrontation in `logging/goal-and-audiences` is the standing statement of
      that.
18. **Dynamic target interning** — 32 registrations succeed with distinct ids; the 33rd returns
    `None` with `E0106`; re-registering an existing name returns the same id; concurrent registration
    of one name from 16 threads yields one id.
19. **Cursor wrap at 2³²** (E17) — preset `write`/`read` to `u32::MAX − 64`, push records across the
    boundary, assert every record decodes and `write.wrapping_sub(read) <= LANE_BYTES` throughout.
20. **`LogRing` cursor wrap and `seq_lo` reconstruction** *(extended by M2)* — same treatment for
    `head` / `arena_cursor`, with `skipped` reported; **and** a leg that drives `seq` past `2³²` and
    asserts that `since(cursor)` returns the correct records for cursors on **both** sides of the
    `seq_lo` wrap, using the Decision 21 reconstruction rule. Red state: reconstruct with a plain
    `as u64` widening instead of the wrapping-difference rule ⇒ every record before the wrap is
    reported as newer than every record after it.
21. **Loss-fold exactness under a live producer** (E18) — see G11. *(v3's "saturating drop counters"
    is gone with the saturation, S8.)*

    21b. **`ECS_HANDOFF` overflow** — see G15 (c): overflow is counted as `LossClass::Sink`, `W0117`
    fires once per drain, `lossy` is set, and the lines that fit are intact.
22. **Sampling exactness and independence** — see G10 (a)(b)(c).
23. **Downstream code minting under contention** — see G9.
24. **`LogPod` under Miri (Tree Borrows)** — a padded `#[repr(C)]` struct round-trips **with no
    uninitialised read** (B10: the derive encodes field-by-field, so padding never enters the
    record); a `fmt_pod` reading beyond `POD_LEN` is caught; **and an assertion that no user code
    executes between lane acquisition and the `Release` store** (a `LogPod` whose `fmt_pod` sets a
    TLS flag; the flag must be unset at the `Release` store and set only during drain).
25. **Sink filter and `UNPROVEN(unsunk)`** — a target enabled at `Info` with no sink accepting it
    produces `status=UNPROVEN(unsunk)` + `W0111`, not silence.
26. **Runtime sink open/close** — see G13.
27. **Crash drain, both sides** — see G14.
28. **Binary round-trip and rotation** — see G12 (a)(b).
29. **`frame_epoch` correlation** — every record's `tsc` falls between the epochs of exactly one
    frame; a record emitted during the drain itself is attributed to the next frame, and that is
    **asserted, not assumed**.
30. **Control spec parsing** — `apply_control_spec("net=debug/6!, ecs=off")` sets level, shift and
    sync bit for the named targets, bumps `control_epoch()` by exactly 1, leaves unnamed targets
    **bit-identical**, and rejects an unknown name with a coded error rather than silently ignoring
    it.
31. **Per-site `Once`, and its census row** *(F11, extended by M1)* — three sites sharing one code,
    all three fire exactly once; the same site called 10⁶ times fires once and performs **no store**
    after the first; **and the census prints three `LOG-ONCE` rows, one per fired site, each with a
    real `fired=1`** — with a fourth, never-called site **absent** from the list. Red state: restore
    v3's per-code `fired=1` literal ⇒ the three-site case prints one row and the count is fiction.
32. **Clock epoch straddle** *(S4)* — inject a synthetic forward jump into `boyko_diag::clock`
    mid-run; assert the log records emitted after the jump carry the **incremented**
    `clock_epoch_lo` and that the sink renders it. Red state: give this crate its own `ticks_per_ns`
    back ⇒ its rendered wall times drift by the injected amount while the profiler's window is
    quarantined ⇒ the cross-check reds.
33. **Crate-graph tidy** *(S2, S12)* — `crates/boyko_utils/Cargo.toml` has an **empty**
    `[dependencies]`; `crates/boyko_log/Cargo.toml` names exactly `boyko_diag` and `boyko_macros`;
    **no** workspace manifest names a third-party `log` or `tracing`. Red state: re-add
    `log = "0.4"` to `boyko_demo` ⇒ red.
34. **Registry corpus composition** *(B6)* — assert the walker's TEXT corpus **excludes**
    `docs/archive/**` and that `files_scanned ≥ 500` with the `boyko-W1501` sentinel present; assert
    `docs/diagnostics/B9004.md` and `B9005.md` exist with all three sections. Red state: add
    `docs/archive/**` to the corpus ⇒ check 4 reds on `B9000`/`B9003`/`W9003`, which have no rows and
    never will.

### Property-based

- Random `(level, target, arg-tuple)` sequences round-trip byte-identically through
  `encode`/`decode`, **including `LogPod` members**.
- Random fill/drain interleavings: `emitted == drained + dropped + sampled_out`, exactly, always.
  *(`sampled_out` is a separate term precisely so this identity stays exact — folding it into
  `dropped` would have made the drop count a liar in the other direction. With `u64` accumulation
  the identity holds at session scale too, which the saturating `u32` could not promise.)*
- For any `seq` and any retained-line set, the Decision 21 `seq_lo` reconstruction returns the true
  `u64` sequence number, including across the `2³²` boundary (M2).
- For any rotation schedule and any record stream, `logdec` over all retained files yields a
  subsequence of the emitted stream with no duplicates and no reordering within a file.
- For any control-spec string, `apply_control_spec` is idempotent: applying it twice yields
  bit-identical `CONTROL`.
- **For any `(used, need, level)` with `used <= CAPACITY`, the admission arithmetic never admits a
  write that would pass `read`** — the property F6 violated, stated as a proptest over the raw
  integers rather than only as a scenario test.

### `debug_assert!` invariants

`len <= MAX_RECORD_BYTES`; `len == HEADER_BYTES + args.encoded_len()`; `write.wrapping_sub(read) <=
CAPACITY`; `boyko_diag::lane() < LANE_COUNT || == LANE_UNCLAIMED`; `!IN_EMIT.replace(true)`
(re-entrancy); **`DRAIN_OWNER == my_token` for the whole of any drain body**; `code_idx < MAX_CODES
|| == CODE_IDX_EXHAUSTED`; `boot()` at most once; `codes!` strictly increasing (also a compile-time
`const _`); `EveryN(n).is_power_of_two()` (compile-time); `sample_shift <= 15`; **`LogPod::POD_LEN
== Σ field lengths` (compile-time, generated by the derive — B10)**; `SAMPLE_CTR` row index ==
`boyko_diag::lane()`; `SINK_STATE` transition is a permitted edge; **`ECS_HANDOFF` is written only
by the `DRAIN_OWNER` holder**. *(`TargetId < MAX_TARGETS` is **not** in this list: it is a type
invariant upheld by a closed constructor set, so there is nothing to assert at use — F15.
`LogPod::POD_LEN == size_of::<Self>()` is **gone**: it was the assertion that made the padding UB
look checked.)*

**Release-live** (these run in every profile, not only debug): the `MAX_RECORD_BYTES` check, the
admission-control `saturating_sub`, the sampling arithmetic, the census status computation, **the
`DRAIN_OWNER` CAS on every consumer entry** (B5 — an exclusivity proof that only holds in debug is
not a proof), `OUT_LOCK`'s acquire deadline, the `SINK_REQ`-full refusal, the `ECS_HANDOFF`
admission check, and the code-index exhaustion branch. *(v3 listed "the drop-counter saturation
guard"; there is no saturation any more — S8.)*
