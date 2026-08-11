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
| **L7** | Migrate `boyko_rhi_vulkan` **except the messenger, which is not touched at all**; `E2101`; `W2102` ungated in release; census wiring | as tabled | `[vk-validation]` line, byte for byte |
| **L7-gate** *(⚠️ **F2's PREMISE IS REFUTED BY THE TREE — measured 2026-08-11, before any L7 code was written.** See the block below the table; the polarity of the first clause is wrong as written and the row is left verbatim so the correction is legible)* | **G7, re-cut two-sided** (F2): `E2101` fires on a validation-**on** run and is absent on a validation-**off** run (`BOYKO_DISABLE_VALIDATION=1`). Channel liveness is proved separately by an **ordinary validation error from a deliberately invalid call** — the historical `mip_levels: 12` on a 512×512 image — with the **baseline of 19 messages accounted for**. A forced *hazard* is explicitly **not** the control: this machine has been measured unable to produce `SYNC-HAZARD` (M25) | `crates/boyko_rhi_vulkan/tests/` | — |
| **L8a** | Migrate `boyko_render`, `boyko_image`, `boyko_serialize`, `boyko_physics`. **Edit `boyko_image/Cargo.toml:5`'s description in the same commit** — it stops being true here | ledger | goldens |
| **L8b** | Migrate `boyko_app`; **zero measurement rows** (S1 — profiling rung 7 removed the producers already). Delete `boyko_demo`'s third-party `log = "0.4"` and migrate `main.rs:113`; add the tidy check banning third-party `log`/`tracing` in any workspace manifest. **MUST LAND AFTER profiling rung 7 and 7b** | ledger, `crates/boyko_demo/` | — |
| **L8c** | Check 3c armed: `Pending` == 0 (`Historical` excluded). Walker's unclassified-site count == 0 over the **≤ 78** denominator; enable `print_census.rs`; run the clippy `disallowed-macros` canary and record the result | `tests/`, `clippy.toml` | — |
| **L9** | `boyko_ui` console widget over `LogRing`. **Deferred to the UI plan** — L16 fixes the whole contract it consumes, so nothing logging-shaped remains in it (open question 12) | `crates/boyko_ui/` | — |
| **L10** | **Dynamic targets.** `DYN_NAMES` interning, `register_dynamic_target`, `find_target`, `targets()`, the five `dyn_*!` macros, `E0106` | `src/target.rs`, `src/macros.rs` | static-target expansion byte-for-byte; G1/G4 must still pass unchanged |
| **L10-gate** | **G8** (a-d). Bench `log_dyn_disabled` vs `log_disabled_runtime` | `tests/`, `benches/` | — |
| **L11a** | **Downstream code tables.** `codes!` exported with `prefix`; `CodeIdx::Dynamic` + lazy minting; `codes_tidy!`; `CODE_OCCUPANCY` + `W0114`; **exhaustion behaviour + `CODE_IDX_EXHAUSTED` + `E0115`** (M3) | `src/codes.rs` | engine `code_idx` remains a compile-time constant; **no mint may ever return an aliased index** |
| **L11b** | **`LogPod`** + `#[derive(LogPod)]` generating **field-by-field `encode_pod`** (B10) + the `*_kv!` field-name forms | `boyko_macros`, `src/site.rs` | Decision 13's structural property (asserted by test 24); **no `copy_nonoverlapping` of `size_of::<Self>()` anywhere in the derive** |
| **L11-gate** | **G9** (incl. the **exhaustion leg**, M3), **G9b** (subject changed to the padded-encode red, B10) | `tests/` | — |
| **L12** | **Sampling.** `SAMPLE_CTR`, the first-touch seed, step 4 of Algorithms A, `sampled_out` plumbing, `W0113`, census `UnprovenSampled` | `src/sample.rs`, `src/lane.rs` | the ≤ 15 ns enabled target — **G10d decides whether this rung ships default-on** |
| **L12-gate** | **G10** (a-e), including **G10e**, the leg relocated from L0 with its observable split (B4), and the perturbation control that can flip `log-sampling` to default-off | `tests/`, `benches/` | — |
| **L13a** | **Volume, part 1.** `Rotation`, `W0112`, `u64` loss accounting end-to-end via `boyko_diag::loss` (S8), `LogStats` u64 accumulation, `LogRing` cursor-wrap hardening **incl. `seq_lo`'s reconstruction rule** (M2) | `src/sink/file.rs`, `src/lane.rs`, ECS seam | `Rotation::NONE` remains the engine default |
| **L13a-gate** | **G11**, subject replaced by S8's fold-exactness red | `tests/` | — |
| **L13b** | **Volume, part 2.** `BinarySink` with the widths pinned in Decision 21 (M2), the **anchor cadence** (1 s or `u32` overflow), `SITE_DICT` + full-table `W0116` + inline site records, `SINK_OUT`, dictionary records, `logdec`, `docs/LOG-BINARY-FORMAT.md` | `src/sink/binary.rs`, `src/bin/logdec.rs` | text-sink output byte-for-byte; **the audited widths** |
| **L13b-gate** | **G12** (a-c) — **including the revert clause** | `tests/`, `benches/` | — |
| **L14** | **Runtime sink control.** `SinkSlot` state/filter/floor, `SINK_REQ`, `request_open_file`/`request_close`, `E0107`, `ControlSource::File` + `apply_control_spec`, census `UNPROVEN(unsunk)` + `W0111` | `src/sink/request.rs`, `src/control.rs` | no I/O on a caller thread |
| **L14-gate** | **G13** (a-c) | `tests/` | — |
| **L15** | **Crash path.** `CrashSink` opened **on the enable path** (S13 — it was "at boot"; the file is opened when diagnostics are turned on, which is still before the first frame and still not inside the panic hook), `SINK_STATE::Exiting`, the panic-hook protocol **with step 1.5 (`PRE_FLUSH`)**, the `DRAIN_OWNER` claim (B5), `E0109`, `E0118`. **`SinkMode::Scheduled`** and its `DRAIN_OWNER` participation (B8) | `src/sink/crash.rs`, `src/sink/mod.rs` | Decision 12's flush semantics; no new unbounded wait; **`SINK_STATE` must NOT regain an exclusivity role**; **the open must NOT move into the hook** |
| **L15-gate** | **G14**, **three-sided** — the third leg panics while a **manual `drain()` is in flight** (B5) | `tests/` | — |
| **L16** | **Game consumption.** ~~`TARGET_STATS`~~ *(landed at L4 — see that row)*, `LogCensus`, `DiagCensus`, `LogRing::since` + `RingFilter` + `skipped`, the per-frame **`frame_epoch`** record (S11 rename), `boyko_diag::SessionId` in every header, the `ONCE_SITES` census walk (M1) | `src/target.rs`, `crates/boyko_ecs/.../log/` | the drain stays off the frame thread **except under `Scheduled`, where it is on it by design** |
| **L16-gate** | **G15**, two-sided, plus the `ECS_HANDOFF` overflow leg (`W0117` fires, `lossy` set, no silent loss) | `crates/boyko_app/tests/` | — |
| **L17 → J1** *(**AXIS HALF SHIPPED** at profiling rung 14; the rest is OWED)* | **Merged with profiling rung 14 into ONE joint rung** (S9): the single `BOYKO_PROFILE` axis, `LogRuntimePreset`, the three header facts, and **5 CI legs** (`dev` existing + 4 net new). One axis cannot be split across two rungs. **What landed:** `crates/boyko_diag/build.rs`, `LOG_CEILING` on the axis at all five rows (`5/4/3/2/0`), the 4 net-new CI legs, the `compile_error!`, and `G16` (a)(b)(c). **What did NOT, and why it could not:** `LogRuntimePreset` and the three header facts (`build_profile` / `runtime_preset` / `ceiling` printed as three independent values, with a fixture proving the first two can differ in one binary) are `G16(d)`, and they need a sink header to print into — **L13b–L16 have not landed**. Measured at rung 14: `boyko_log` has `census`, `codes`, `drain_owner`, `lane`, `level`, `lifecycle`, `macros`, `rate`, `record`, `site`, `sync_out`, `target` and `sink/{ecs,file,mod}`; it has **no** `sample.rs`, `sink/binary.rs`, `sink/request.rs`, `sink/crash.rs` or `bin/logdec.rs`. **The axis was landed alone precisely because S9 forbids splitting it** — the *axis* is indivisible and it is now whole; `LogRuntimePreset` is not part of the axis, it is the runtime counterpart the axis is deliberately kept separate from | `crates/boyko_diag/build.rs`, `crates/profile_fixture_log/`, `.github/workflows/ci.yml` | G2's `off` leg must still pass unchanged |
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

| Bench | Target | Control |
|---|---|---|
| `log_disabled_runtime` | ≤ 3 ns | the same site enabled; **and the v2-shaped unpacked gate, which must be NOT RESOLVED** (G10d) |
| `log_disabled_warn` | ≤ 4 ns | `log_disabled_runtime` (an `info!`, untouched by `sink_can_accept`) in the same sitting — the delta **is** S5's added load + branch |
| `log_enabled_0args` / `_2u32` / `_str32` | ≤ 15 / 20 / 30 ns | runtime-disabled |
| `log_enabled_rate_once_fired` | ≤ 5 ns, **no store, no shared line** | `Every` policy |
| `sink_sustained_rate` | finds the drop knee; reports records·s⁻¹ | zero-record idle sink |
| `lane_padding_ablation` | padded+cached vs padded-only vs neither | — |
| `sched_cpu_logger_on_off` (gate **P1**, re-specified) | not resolvable above the floor, **at each of the two profiler states** | interleaved zero control, ABBA, **2×2 with {profiler absent, armed}** (S10) |
| `log_dyn_disabled` | ≤ 4 ns, **and the delta vs `log_disabled_runtime` must RESOLVE** | `log_disabled_runtime`, same sitting |
| `log_enabled_sampled_out` | ≤ 6 ns | the same site with shift = 0 |
| `log_enabled_0args_sampling` | NOT RESOLVED vs the pre-L12 baseline | pre-L12 baseline, same sitting |
| `log_pod_12b` | ≤ 20 ns | `dsp!` of the same value, which must be ≥ 5× slower |
| `sink_sustained_rate_binary` | ≥ 3 M rec·s⁻¹ **and ≥ 5× the text sink** | the text sink, same sitting |
| `downstream_code_warn` | ≤ 18 ns | the engine-code `warn!`, same sitting; the delta is the `idx_cell` load |
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
